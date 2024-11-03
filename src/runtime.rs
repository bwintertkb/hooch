use std::{
    cell::{Cell, OnceCell},
    future::Future,
    mem::MaybeUninit,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{sync_channel, Receiver},
        Arc, Mutex,
    },
    thread::JoinHandle,
};

use crate::{
    executor::ExecutorTask,
    spawner::{new_executor_spawner, JoinHandle as SpawnerJoinHandle, Spawner},
};

thread_local! {
    pub static RUNTIME_GUARD: Cell<bool> = const { Cell::new(false) };
    pub static RUNTIME: OnceCell<Runtime> = const { OnceCell::new() };
}

pub struct RuntimeBuilder {
    num_workers: usize,
}

impl RuntimeBuilder {
    pub fn new() -> Self {
        Self { num_workers: 1 }
    }

    pub fn num_workers(mut self, num_workers: usize) -> Self {
        self.num_workers = num_workers;
        self
    }

    pub fn build(self) -> Handle {
        // Initialize the thread-local runtime.
        let (panic_tx, panic_rx) = sync_channel(1);
        let panic_rx_arc = Arc::new(Mutex::new(panic_rx));
        RUNTIME.with(|cell| {
            cell.get_or_init(|| {
                let panic_rx_clone = Arc::clone(&panic_rx_arc);
                let mut executor_handles = Vec::with_capacity(self.num_workers);
                let mut spawners = Vec::with_capacity(self.num_workers);
                for i in 0..self.num_workers {
                    let (executor, spawner) = new_executor_spawner(panic_tx.clone());
                    // Sender and receiver for panic detection
                    spawners.push(spawner);
                    let panic_tx_clone = panic_tx.clone();
                    let handle = std::thread::Builder::new()
                        .name(format!("executor_thread_{}", i))
                        .spawn(move || {
                            if let Err(err) = std::panic::catch_unwind(|| {
                                set_runtime_guard();
                                executor.run();
                                exit_runtime_guard();
                            }) {
                                // Notify the handler that a panic has occured in a executor
                                println!("EXECUTOR HAS PANICKED. ERROR: {:?}", err);
                                let _ = panic_tx_clone.send(());
                            }
                        })
                        .unwrap();
                    executor_handles.push(Some(handle));
                }

                let handle = Handle {
                    panic_rx: panic_rx_clone,
                };
                Runtime {
                    dispatch_worker: AtomicUsize::new(0),
                    spawners,
                    handles: executor_handles,
                    runtime_handle: handle,
                }
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

#[derive(Debug)]
pub struct Runtime {
    /// This is a handler for a scheduler, currently it will just be a round robin
    dispatch_worker: AtomicUsize,
    spawners: Vec<Spawner>,
    handles: Vec<Option<JoinHandle<()>>>,
    runtime_handle: Handle,
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.handles
            .iter_mut()
            .zip(&self.spawners)
            .for_each(|(handle, spawner)| {
                let handle = handle.take();
                spawner.spawn_task(ExecutorTask::Finished);
                if let Some(handle) = handle {
                    let _ = handle.join();
                }
            });
    }
}

impl Runtime {
    pub fn handle() -> Handle {
        let mut handle_ptr: *const Handle = std::ptr::null();
        RUNTIME.with(|cell| {
            let runtime = cell.get().unwrap();
            let inner_handle = runtime.runtime_handle.clone();
            handle_ptr = &inner_handle;
            std::mem::forget(inner_handle);
        });

        unsafe {
            if handle_ptr.is_null() {
                panic!("HANDLE PTR IS NULL");
            }
            let handle: Handle = (*handle_ptr).clone();
            handle
        }
    }

    pub fn dispatch_job<Fut, T>(&self, future: Fut) -> SpawnerJoinHandle<T>
    where
        T: Send + 'static,
        Fut: Future<Output = T> + Send + 'static + std::panic::UnwindSafe,
    {
        let dispatch_idx = self.get_dispatch_worker_idx();
        self.spawners[dispatch_idx].spawn_self(future)
    }

    pub fn run_blocking<Fut>(&self, future: Fut)
    where
        Fut: Future<Output = ()> + Send + 'static + std::panic::UnwindSafe,
    {
        let (tx, rx) = sync_channel(1);
        let dispatch_idx = self.get_dispatch_worker_idx();
        self.spawners[dispatch_idx].spawn_self(async move {
            future.await;
            tx.send(()).unwrap();
        });

        let _ = rx.recv();
    }

    /// Currently naive round-robin
    fn get_dispatch_worker_idx(&self) -> usize {
        self.dispatch_worker.fetch_add(1, Ordering::Relaxed) % self.spawners.len()
    }
}
fn set_runtime_guard() {
    if RUNTIME_GUARD.get() {
        panic!("Cannot run nested runtimes");
    }
    RUNTIME_GUARD.replace(true);
}

fn exit_runtime_guard() {
    RUNTIME_GUARD.replace(false);
}

#[derive(Debug, Clone)]
pub struct Handle {
    /// If notified in this channel it means that and executor has panicked
    panic_rx: Arc<Mutex<Receiver<()>>>,
}

impl Handle {
    pub fn run_blocking<Fut>(&self, future: Fut)
    where
        Fut: Future<Output = ()> + Send + 'static + std::panic::UnwindSafe,
    {
        set_runtime_guard();
        RUNTIME.with(|cell| {
            let runtime = cell.get().unwrap();
            runtime.run_blocking(future);
        });
        if self.panic_rx.lock().unwrap().try_recv().is_ok() {
            exit_runtime_guard();
            panic!("Executor panicked");
        }
        exit_runtime_guard();
    }

    /// TODO add the join handle
    pub fn spawn<Fut, T>(&self, future: Fut) -> SpawnerJoinHandle<T>
    where
        T: Send + 'static,
        Fut: Future<Output = T> + Send + 'static + std::panic::UnwindSafe,
    {
        let mut join_handle: MaybeUninit<SpawnerJoinHandle<T>> = MaybeUninit::uninit();
        let join_handle_ptr: *mut SpawnerJoinHandle<T> = join_handle.as_mut_ptr();
        RUNTIME.with(|cell| {
            let runtime = cell.get().unwrap();
            let join_handle = runtime.dispatch_job(future);
            // Write the handle into the unitialized memory location
            unsafe { join_handle_ptr.write(join_handle) }
        });

        unsafe { join_handle.assume_init() }
    }

    pub fn num_workers(&self) -> usize {
        let mut num_workers = 0;
        RUNTIME.with(|cell| {
            let runtime = cell.get().unwrap();
            num_workers = runtime.spawners.len();
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
    fn test_get_dispatch_worker_idx() {
        let _ = RuntimeBuilder::default().num_workers(2).build();
        RUNTIME.with(|cell| {
            let runtime = cell.get().unwrap();
            assert!(runtime.get_dispatch_worker_idx() == 0);
            assert!(runtime.get_dispatch_worker_idx() == 1);
            assert!(runtime.get_dispatch_worker_idx() == 0);
            assert!(runtime.get_dispatch_worker_idx() == 1);
        });
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
        handle.run_blocking(async {
            let thread_name = get_thread_name().await;
            assert!(thread_name == "executor_thread_1");
        });
        handle.run_blocking(async {
            let thread_name = get_thread_name().await;
            assert!(thread_name == "executor_thread_2");
        });
    }

    #[test]
    #[should_panic]
    /// Main handle thread should panic if if blocking task panics
    fn test_handle_panicking_task() {
        let handle = RuntimeBuilder::default().build();
        handle.run_blocking(async {
            // This will panic because the thread it's being sent too did not create a Runtime yet
            let _ = Runtime::handle();
        });
    }
}
