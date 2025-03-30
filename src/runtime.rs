//! This module defines a custom asynchronous `Runtime` with a `RuntimeBuilder` for configuration
//! and a `Handle` for managing tasks. The runtime supports multi-threaded task execution
//! and uses a round-robin dispatch mechanism to balance load across worker threads.
//!
//! The runtime also includes panic handling, allowing detection and handling of
//! panics in worker threads.

use std::{
    cell::{Cell, OnceCell},
    future::Future,
    sync::{
        atomic::AtomicBool,
        mpsc::{sync_channel, Receiver},
        Arc, Mutex,
    },
    thread::JoinHandle,
};

use crate::{
    pool::thread_pool::{HoochPool, HOOCH_POOL},
    spawner::{new_executor_spawner, JoinHandle as SpawnerJoinHandle, Spawner},
    task::{
        manager::{TaskManager, TASK_MANAGER},
        Task,
    },
};

thread_local! {
    /// Flag indicating whether a runtime is currently active in this thread.
    pub static RUNTIME_GUARD: Cell<bool> = const { Cell::new(false) };
    /// Singleton instance of the runtime.
    pub static RUNTIME: OnceCell<Arc<Runtime>> = const { OnceCell::new() };
}

/// Builder for creating and configuring a `Runtime` instance.
/// Allows customization of the number of worker threads.
pub struct RuntimeBuilder {
    num_workers: usize,
}

impl RuntimeBuilder {
    /// Creates a new `RuntimeBuilder` with default settings.
    pub fn new() -> Self {
        Self { num_workers: 1 }
    }

    /// Sets the number of worker threads.
    pub fn num_workers(mut self, num_workers: usize) -> Self {
        self.num_workers = num_workers;
        self
    }

    /// Builds and initializes the runtime, returning a handle for task management.
    pub fn build(self) -> Handle {
        set_runtime_guard();
        // Initialize the thread-local runtime.
        let (panic_tx, panic_rx) = sync_channel(1);
        let panic_rx_arc = Arc::new(Mutex::new(panic_rx));
        let mut spawner = None;
        RUNTIME.with(|cell| {
            cell.get_or_init(|| {
                // We want a minimum of two executors, we need at least a combination of blocking
                // and non-blocking executors, primarily because a blocking task on a single
                // executor will block the entire runtime, i.e. a single worker is not truly single
                // threaded
                let num_workers = self.num_workers;
                let panic_rx_clone = Arc::clone(&panic_rx_arc);
                let mut executor_handles = Vec::with_capacity(num_workers);

                // Start off with a thread pool of 1
                HoochPool::init(1);
                let hooch_pool = HoochPool::get();
                let tm = TaskManager::get();
                // Spawn worker threads based on `num_workers`.
                let mut runtime_txs = Vec::with_capacity(num_workers);
                let mut tm_txs = Vec::with_capacity(num_workers);
                let mut hp_txs = Vec::with_capacity(num_workers);
                for i in 0..num_workers {
                    let (runtime_tx, runtime_rx) = std::sync::mpsc::sync_channel(1);
                    let (tm_tx, tm_rx) = std::sync::mpsc::sync_channel(1);
                    let (hp_tx, hp_rx) = std::sync::mpsc::sync_channel(1);
                    runtime_txs.push(runtime_tx);
                    tm_txs.push(tm_tx);
                    hp_txs.push(hp_tx);

                    let (executor, spawner_inner, exec_sender) =
                        new_executor_spawner(panic_tx.clone(), i);

                    tm.register_executor(executor.id(), exec_sender);

                    spawner = Some(spawner_inner);
                    let panic_tx_clone = panic_tx.clone();
                    let handle = std::thread::Builder::new()
                        .name(format!("executor_thread_{}", i))
                        .spawn(move || {
                            let tm = tm_rx.recv().unwrap();

                            TASK_MANAGER.with(move |cell| {
                                cell.get_or_init(move || tm);
                            });

                            let hooch_pool = hp_rx.recv().unwrap();

                            HOOCH_POOL.with(move |cell| {
                                cell.get_or_init(move || hooch_pool);
                            });

                            let runtime = runtime_rx.recv().unwrap();

                            RUNTIME.with(move |cell| {
                                cell.get_or_init(move || runtime);
                            });

                            if let Err(err) = std::panic::catch_unwind(|| {
                                set_runtime_guard();
                                executor.run();
                                exit_runtime_guard();
                            }) {
                                println!("EXECUTOR HAS PANICKED. ERROR: {:?}", err);
                                let _ = panic_tx_clone.send(());
                            }
                        })
                        .unwrap();
                    executor_handles.push(Some(handle));
                }
                tm_txs
                    .into_iter()
                    .for_each(|tx| tx.send(Arc::clone(&tm)).unwrap());
                hp_txs
                    .into_iter()
                    .for_each(|tx| tx.send(Arc::clone(&hooch_pool)).unwrap());

                let handle = Handle {
                    panic_rx: panic_rx_clone,
                };
                let rt = Arc::new(Runtime {
                    handles: executor_handles,
                    runtime_handle: handle,
                });

                runtime_txs.into_iter().for_each(|tx| {
                    tx.send(Arc::clone(&rt)).unwrap();
                });

                rt
            });
        });

        Runtime::handle()
    }
}

impl Default for RuntimeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// The main `Runtime` struct, managing task execution across multiple worker threads.
/// Uses round-robin scheduling to balance tasks among worker threads.
#[derive(Debug)]
pub struct Runtime {
    handles: Vec<Option<JoinHandle<()>>>, // Handles for worker threads.
    runtime_handle: Handle,               // Handle for interacting with the runtime.
}

impl Drop for Runtime {
    /// Ensures that all worker threads are stopped and joined when the runtime is dropped.
    fn drop(&mut self) {
        // self.handles
        //     .iter_mut()
        //     .zip(&self.spawner)
        //     .for_each(|(handle, spawner)| {
        //         let handle = handle.take();
        //         spawner.spawn_task(ExecutorTask::Finished);
        //         if let Some(handle) = handle {
        //             let _ = handle.join();
        //         }
        //     });
        exit_runtime_guard();
    }
}

impl Runtime {
    /// Returns a handle to the runtime, which can be used to manage tasks.
    pub fn handle() -> Handle {
        // let mut handle_ptr: *const Handle = std::ptr::null();
        let mut handle: Option<Handle> = None;
        RUNTIME.with(|cell| {
            let runtime = cell.get().unwrap();
            let inner_handle = runtime.runtime_handle.clone();
            handle = Some(inner_handle);
        });
        handle.unwrap()
    }

    /// Dispatches a job to a worker, selecting the next worker in a round-robin fashion.
    pub fn dispatch_job<Fut, T>(&self, future: Fut) -> SpawnerJoinHandle<T>
    where
        T: Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
    {
        Spawner::spawn(future)
    }

    /// Runs a blocking future on the runtime.
    pub fn run_blocking<Fut, T>(&self, future: Fut) -> T
    where
        T: Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
    {
        let (tx, rx) = sync_channel(1);
        let tm = TaskManager::get();

        // Create a one-shot future that will complete only once
        let task = Task {
            future: Mutex::new(Some(Box::pin(async move {
                let res = future.await;
                let _ = tx.send(res); // Use let _ to ignore send result
            }))),
            task_tag: Task::generate_tag(),
            manager: Arc::downgrade(&tm),
            abort: Arc::new(AtomicBool::new(false)),
        };

        // Register task and wait for completion
        tm.register_or_execute_non_blocking_task(Arc::new(task));

        // Remove debug print
        match rx.recv() {
            Ok(result) => result,
            Err(_) => panic!("Task failed to complete"),
        }
    }
}

/// Sets the thread-local runtime guard, ensuring no nested runtimes are allowed.
fn set_runtime_guard() {
    if RUNTIME_GUARD.get() {
        panic!("Cannot run nested runtimes");
    }
    RUNTIME_GUARD.replace(true);
}

/// Exits the runtime guard, allowing another runtime to be created in this thread.
fn exit_runtime_guard() {
    RUNTIME_GUARD.replace(false);
}

/// Handle for interacting with the runtime, providing task spawning and worker count retrieval.
#[derive(Debug, Clone)]
pub struct Handle {
    /// Channel for receiving notifications if an executor panics.
    panic_rx: Arc<Mutex<Receiver<()>>>,
}

impl Handle {
    /// Runs a blocking future on the runtime, with panic detection.
    pub fn run_blocking<Fut, T>(&self, future: Fut) -> T
    where
        T: Send + Unpin + 'static,
        Fut: Future<Output = T> + Send + 'static,
    {
        let mut res = None;
        RUNTIME.with(|cell| {
            let runtime = cell.get().unwrap();
            let blocking_res = runtime.run_blocking(future);
            res = Some(blocking_res)
        });
        if self.panic_rx.lock().unwrap().try_recv().is_ok() {
            exit_runtime_guard();
            panic!("Executor panicked");
        }
        res.unwrap()
    }

    /// Spawns a non-blocking task on the runtime.
    pub fn spawn<Fut, T>(&self, future: Fut) -> SpawnerJoinHandle<T>
    where
        T: Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
    {
        let mut join_handle = None;
        RUNTIME.with(|cell| {
            let runtime = cell.get().unwrap();
            let join_handle_inner = runtime.dispatch_job(future);
            join_handle = Some(join_handle_inner);
        });

        join_handle.unwrap()
    }

    /// Returns the number of worker threads in the runtime.
    pub fn num_workers(&self) -> usize {
        let mut num_workers = 0;
        RUNTIME.with(|cell| {
            let runtime = cell.get().unwrap();
            num_workers = runtime.handles.len();
        });
        num_workers
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    };

    use super::*;

    async fn increment(ctr: Arc<AtomicU8>) {
        ctr.fetch_add(1, Ordering::Relaxed);
    }

    #[test]
    fn test_runtime_builder_default() {
        assert!(RuntimeBuilder::default().num_workers == 1);
    }

    #[test]
    fn test_runtime_builder_num_workers() {
        assert!(RuntimeBuilder::default().num_workers(2).num_workers == 2);
    }

    #[test]
    fn test_runtime_num_workers() {
        let handle = RuntimeBuilder::default().num_workers(4).build();
        assert!(handle.num_workers() == 4);
    }

    #[test]
    fn test_run_blocking() {
        let runtime = RuntimeBuilder::default().num_workers(2).build();
        let ctr = Arc::new(AtomicU8::new(0));
        runtime.run_blocking(increment(Arc::clone(&ctr)));
        assert!(ctr.swap(0, Ordering::Relaxed) == 1);
    }

    #[test]
    fn test_run_blocking_return() {
        let handle = RuntimeBuilder::default().build();
        let ctr = 1;
        let res = handle.run_blocking(async move { ctr + 1 });
        assert!(res == 2)
    }

    #[test]
    #[should_panic]
    fn test_handle_nested_panicking_task() {
        let handle = RuntimeBuilder::default().build();
        handle.run_blocking(async {
            let _ = RuntimeBuilder::default().build();
        });
    }

    #[test]
    #[should_panic]
    /// You cannot create two runtimes if one is already in scope
    fn test_handle_scoped_panicking_task() {
        RuntimeBuilder::default().build();
        RuntimeBuilder::default().build();
    }

    #[test]
    fn test_obtaining_multiple_handles_from_same_runtime() {
        let handle = RuntimeBuilder::default().build();
        let r = handle.run_blocking(async { 1 });
        assert!(r == 1);
        let handle = Runtime::handle();
        let r = handle.run_blocking(async { 2 });
        assert!(r == 2)
    }

    #[test]
    fn test_multiple_runtimes_thread_task() {
        let ct1 = Arc::new(Mutex::new(0));
        let ct1_clone = Arc::clone(&ct1);
        let t1 = std::thread::spawn(move || {
            let handle = RuntimeBuilder::default().build();
            handle.run_blocking(async move {
                *ct1_clone.lock().unwrap() += 1;
            });
        });
        let ct2 = Arc::new(Mutex::new(0));
        let ct2_clone = Arc::clone(&ct2);
        let t2 = std::thread::spawn(move || {
            let handle = RuntimeBuilder::default().build();
            handle.run_blocking(async move {
                *ct2_clone.lock().unwrap() += 1;
            });
        });

        t1.join().unwrap();
        t2.join().unwrap();

        assert!(*ct1.lock().unwrap() == 1);
        assert!(*ct2.lock().unwrap() == 1);
    }
}
