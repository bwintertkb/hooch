//! This module defines a `Task` struct for managing asynchronous task execution,
//! a `Spawner` for task spawning, and a `JoinHandle` for awaiting task completion.
//!
//! The `Spawner` is used to create new tasks and assign them to executors, while
//! the `JoinHandle` allows the caller to await the result of a spawned task.

use std::{
    any::Any,
    error::Error,
    fmt::Display,
    future::Future,
    marker::PhantomData,
    panic::UnwindSafe,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvError, SyncSender},
        Arc, Mutex,
    },
    task::Poll,
};

use crate::sync::mpsc::bounded_channel;
use crate::{
    executor::{Executor, ExecutorTask},
    runtime::Runtime,
    sync::mpsc::BoundedReceiver,
    task::Task,
};

/// Type alias for boxed, pinned future results with any `Send` type.
pub type BoxedFutureResult =
    Pin<Box<dyn Future<Output = Result<Box<dyn Any + Send>, RecvError>> + Send>>;

/// `Spawner` is responsible for spawning and managing tasks within the runtime.
/// It provides a method for directly spawning tasks and for wrapping futures into `JoinHandle`s.
#[derive(Debug, Clone)]
pub struct Spawner {
    task_sender: std::sync::mpsc::SyncSender<ExecutorTask>, // Sender for submitting tasks to the executor.
}

impl Spawner {
    /// Spawns a future that produces an output of type `T`, returning a `JoinHandle` for result retrieval.
    pub fn spawn_self<T: Send + 'static>(
        &self,
        future: impl Future<Output = T> + Send + 'static,
    ) -> JoinHandle<T> {
        let (tx_bound, rx_bound) = bounded_channel(1);
        let wrapped_future = async move {
            let res = future.await;
            let res: Box<dyn Any + Send + 'static> = Box::new(res);
            let _ = tx_bound.send(res);
        };
        let task = Arc::new(Task {
            future: Mutex::new(Box::pin(wrapped_future)),
            spawner: self.clone(),
            task_tag: Task::generate_tag(),
        });

        self.spawn_task(ExecutorTask::Task(task));
        JoinHandle {
            rx: Arc::new(rx_bound),
            has_awaited: Arc::new(AtomicBool::new(false)),
            recv_fut: None,
            marker: PhantomData,
        }
    }

    /// Spawns a new task by sending it to the executor.
    pub fn spawn_task(&self, task: ExecutorTask) {
        self.task_sender.send(task).unwrap();
    }

    /// Convenience method for spawning tasks with the runtime handle.
    pub fn spawn<T>(future: impl Future<Output = T> + Send + 'static) -> JoinHandle<T>
    where
        T: Send + 'static,
    {
        let handle = Runtime::handle();
        handle.spawn(future)
    }
}

/// Creates a new `Executor` and `Spawner` pair, with a maximum task queue size of 10,000.
pub fn new_executor_spawner(panic_tx: SyncSender<()>, executor_id: usize) -> (Executor, Spawner) {
    const MAX_QUEUED_TASKS: usize = 10_000;
    let (task_sender, ready_queue) = mpsc::sync_channel(MAX_QUEUED_TASKS);

    (
        Executor::new(ready_queue, executor_id, panic_tx),
        Spawner { task_sender },
    )
}

/// A `JoinHandle` represents the handle for a spawned task, allowing the caller to await its completion.
pub struct JoinHandle<T> {
    rx: Arc<BoundedReceiver<Box<dyn Any + Send>>>, // Receiver for the result of the future.
    has_awaited: Arc<AtomicBool>,                  // Flag indicating if the task has been awaited.
    recv_fut: Option<BoxedFutureResult>,           // Future for receiving the result.
    marker: PhantomData<T>,                        // PhantomData for the output type.
}

impl<T> UnwindSafe for JoinHandle<T> {}
unsafe impl<T> Send for JoinHandle<T> {}
unsafe impl<T> Sync for JoinHandle<T> {}

impl<T> Future for JoinHandle<T>
where
    T: Unpin + 'static,
{
    type Output = Result<T, JoinHandleError>;

    /// Polls the task result, returning `Poll::Ready` when the task completes or an error occurs.
    fn poll(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.as_mut().get_mut();
        this.has_awaited.store(true, Ordering::Relaxed);

        if this.recv_fut.is_none() {
            let rx_clone = Arc::clone(&this.rx);
            let fut = async move { rx_clone.recv().await };
            this.recv_fut = Some(Box::pin(fut));
        }

        match this.recv_fut.as_mut().unwrap().as_mut().poll(cx) {
            Poll::Ready(value) => {
                if let Ok(ok_val) = value {
                    let ret_val = *ok_val.downcast::<T>().unwrap();
                    return Poll::Ready(Ok(ret_val));
                }

                Poll::Ready(Err(JoinHandleError {
                    msg: "Error occurred while awaiting task".into(),
                }))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<T> JoinHandle<T> {
    pub fn quit(self) {}
}

/// Error type for `JoinHandle` indicating task execution issues.
#[derive(Debug)]
pub struct JoinHandleError {
    msg: String, // Error message
}

impl Display for JoinHandleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.msg)
    }
}

impl Error for JoinHandleError {}
