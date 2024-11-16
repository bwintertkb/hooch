//! This module defines a custom asynchronous `Runtime` with a `RuntimeBuilder` for configuration
//! and a `Handle` for managing tasks. The runtime supports multi-threaded task execution
//! and uses a round-robin dispatch mechanism to balance load across worker threads.
//!
//! The runtime also includes panic handling, allowing detection and handling of
//! panics in worker threads.

use std::{
    cell::{Cell, OnceCell},
    future::Future,
    mem::MaybeUninit,
    sync::{
        mpsc::{sync_channel, Receiver},
        Arc, Mutex,
    },
    thread::JoinHandle,
};

use crate::{
    spawner::{new_executor_spawner, JoinHandle as SpawnerJoinHandle, Spawner},
    task::manager::{TaskManager, TASK_MANAGER},
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
        // Initialize the thread-local runtime.
        let (panic_tx, panic_rx) = sync_channel(1);
        let panic_rx_arc = Arc::new(Mutex::new(panic_rx));
        let mut spawner = None;
        RUNTIME.with(|cell| {
            cell.get_or_init(|| {
                let panic_rx_clone = Arc::clone(&panic_rx_arc);
                let mut executor_handles = Vec::with_capacity(self.num_workers);

                let tm = TaskManager::get();
                // Spawn worker threads based on `num_workers`.
                let mut runtime_txs = Vec::with_capacity(self.num_workers);
                let mut tm_txs = Vec::with_capacity(self.num_workers);
                for i in 0..self.num_workers {
                    let (runtime_tx, runtime_rx) = std::sync::mpsc::sync_channel(1);
                    let (tm_tx, tm_rx) = std::sync::mpsc::sync_channel(1);
                    runtime_txs.push(runtime_tx);
                    tm_txs.push(tm_tx);

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

                let handle = Handle {
                    panic_rx: panic_rx_clone,
                };
                let rt = Arc::new(Runtime {
                    spawner: spawner.unwrap(),
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
    spawner: Spawner,                     // Spawner for task execution.
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
        self.spawner.spawn_self(future)
    }

    /// Runs a blocking future on the runtime.
    pub fn run_blocking<Fut, T>(&self, future: Fut) -> T
    where
        T: Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
    {
        let (tx, rx) = sync_channel(1);

        self.spawner.spawn_self(async move {
            let res = future.await;
            tx.send(res).unwrap();
        });

        rx.recv().unwrap()
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
        T: Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
    {
        set_runtime_guard();
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
        exit_runtime_guard();

        res.unwrap()
    }

    /// Spawns a non-blocking task on the runtime.
    pub fn spawn<Fut, T>(&self, future: Fut) -> SpawnerJoinHandle<T>
    where
        T: Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
    {
        let mut join_handle: MaybeUninit<SpawnerJoinHandle<T>> = MaybeUninit::uninit();
        let join_handle_ptr: *mut SpawnerJoinHandle<T> = join_handle.as_mut_ptr();
        RUNTIME.with(|cell| {
            let runtime = cell.get().unwrap();
            let join_handle = runtime.dispatch_job(future);
            unsafe { join_handle_ptr.write(join_handle) }
        });

        unsafe { join_handle.assume_init() }
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

    async fn get_thread_name() -> String {
        std::thread::current().name().unwrap().to_string()
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
    fn test_multithread_round_robin_dispatch() {
        let handle = RuntimeBuilder::default().num_workers(3).build();
        handle.run_blocking(async {
            let thread_name = get_thread_name().await;
            assert!(thread_name == "executor_thread_0");
        });
        handle.run_blocking(async {
            let thread_name = get_thread_name().await;
            assert!(thread_name == "executor_thread_1");
        });
        handle.run_blocking(async {
            let thread_name = get_thread_name().await;
            assert!(thread_name == "executor_thread_2");
        });
        handle.run_blocking(async {
            let thread_name = get_thread_name().await;
            assert!(thread_name == "executor_thread_0");
        });
    }

    #[test]
    fn test_run_blocking_return() {
        let handle = RuntimeBuilder::default().build();

        let ctr = 1;
        let res = handle.run_blocking(async move { ctr + 1 });

        assert!(res == 2)
    }

    // TODO FIX THIS ASPECT OF THE RUNTIME
    #[test]
    #[should_panic]
    /// Main handle thread should panic if a blocking task panics.
    fn test_handle_panicking_task() {
        let handle = RuntimeBuilder::default().build();
        handle.run_blocking(async {
            let _ = RuntimeBuilder::default().build();
        });
    }
}
