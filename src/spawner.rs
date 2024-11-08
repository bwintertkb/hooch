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
    task::{Poll, RawWaker, RawWakerVTable, Waker},
};

use crate::sync::mpsc::bounded_channel;
use crate::{
    executor::{Executor, ExecutorTask},
    runtime::Runtime,
    sync::mpsc::BoundedReceiver,
};

/// Type alias for boxed, pinned future results with any `Send` type.
pub type BoxedFutureResult =
    Pin<Box<dyn Future<Output = Result<Box<dyn Any + Send>, RecvError>> + Send>>;

/// A `Task` represents an asynchronous operation to be executed by an executor.
/// It stores the future that represents the task, as well as a `Spawner` for task management.
pub struct Task {
    pub future: Mutex<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>, // The task's future.
    spawner: Spawner, // Spawner associated with the task.
}

impl Task {
    /// The waker virtual table used to handle task waking functionality.
    const WAKER_VTABLE: RawWakerVTable = RawWakerVTable::new(clone, wake, wake_by_ref, drop);

    /// Creates a `Waker` for this task, allowing it to be polled by an executor.
    pub fn waker(self: Arc<Self>) -> Waker {
        let opaque_ptr = Arc::into_raw(self) as *const ();
        let vtable = &Self::WAKER_VTABLE;
        unsafe { Waker::from_raw(RawWaker::new(opaque_ptr, vtable)) }
    }
}

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
    pub fn spawn<T>(
        future: impl Future<Output = T> + Send + 'static + std::panic::UnwindSafe,
    ) -> JoinHandle<T>
    where
        T: Send + 'static,
    {
        let handle = Runtime::handle();
        handle.spawn(future)
    }
}

/// Creates a new `Executor` and `Spawner` pair, with a maximum task queue size of 10,000.
pub fn new_executor_spawner(panic_tx: SyncSender<()>) -> (Executor, Spawner) {
    const MAX_QUEUED_TASKS: usize = 10_000;
    let (task_sender, ready_queue) = mpsc::sync_channel(MAX_QUEUED_TASKS);

    (
        Executor::new(ready_queue, panic_tx),
        Spawner { task_sender },
    )
}

/// Clones a `RawWaker` pointer, incrementing the reference count.
fn clone(ptr: *const ()) -> RawWaker {
    let original: Arc<Task> = unsafe { Arc::from_raw(ptr as _) };
    let cloned = original.clone();
    std::mem::forget(original);
    std::mem::forget(cloned);

    RawWaker::new(ptr, &Task::WAKER_VTABLE)
}

/// Drops a `RawWaker`, decrementing the reference count.
fn drop(ptr: *const ()) {
    let _: Arc<Task> = unsafe { Arc::from_raw(ptr as _) };
}

/// Wakes a task by scheduling it back into the executor.
fn wake(ptr: *const ()) {
    let arc: Arc<Task> = unsafe { Arc::from_raw(ptr as _) };
    let spawner = arc.spawner.clone();
    spawner.spawn_task(ExecutorTask::Task(arc));
}

/// Wakes a task by reference without consuming the `Arc`.
fn wake_by_ref(ptr: *const ()) {
    let arc: Arc<Task> = unsafe { Arc::from_raw(ptr as _) };
    arc.spawner.spawn_task(ExecutorTask::Task(arc.clone()));
    std::mem::forget(arc);
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
