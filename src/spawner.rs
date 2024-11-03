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

type BoxedFutureResult =
    Pin<Box<dyn Future<Output = Result<Box<dyn Any + Send>, RecvError>> + Send>>;

pub struct Task {
    pub future: Mutex<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>,
    spawner: Spawner,
}

impl Task {
    const WAKER_VTABLE: RawWakerVTable = RawWakerVTable::new(clone, wake, wake_by_ref, drop);

    pub fn waker(self: Arc<Self>) -> Waker {
        let opaque_ptr = Arc::into_raw(self) as *const ();
        let vtable = &Self::WAKER_VTABLE;

        unsafe { Waker::from_raw(RawWaker::new(opaque_ptr, vtable)) }
    }
}

#[derive(Debug, Clone)]
pub struct Spawner {
    task_sender: std::sync::mpsc::SyncSender<ExecutorTask>,
}

impl Spawner {
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

    pub fn spawn_task(&self, task: ExecutorTask) {
        self.task_sender.send(task).unwrap();
    }

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

pub fn new_executor_spawner(panic_tx: SyncSender<()>) -> (Executor, Spawner) {
    const MAX_QUEUED_TASKS: usize = 10_000;

    let (task_sender, ready_queue) = mpsc::sync_channel(MAX_QUEUED_TASKS);

    (
        Executor::new(ready_queue, panic_tx),
        Spawner { task_sender },
    )
}

fn clone(ptr: *const ()) -> RawWaker {
    let original: Arc<Task> = unsafe { Arc::from_raw(ptr as _) };

    // Increment the inner counter of the arc
    let cloned = original.clone();

    // now forget the Arc<Task> so the refcount isn't decremented
    std::mem::forget(original);
    std::mem::forget(cloned);

    RawWaker::new(ptr, &Task::WAKER_VTABLE)
}

fn drop(ptr: *const ()) {
    let _: Arc<Task> = unsafe { Arc::from_raw(ptr as _) };
}

fn wake(ptr: *const ()) {
    let arc: Arc<Task> = unsafe { Arc::from_raw(ptr as _) };
    let spawner = arc.spawner.clone();
    spawner.spawn_task(ExecutorTask::Task(arc));
}

fn wake_by_ref(ptr: *const ()) {
    let arc: Arc<Task> = unsafe { Arc::from_raw(ptr as _) };

    arc.spawner.spawn_task(ExecutorTask::Task(arc.clone()));

    // we don't actually have ownership of this arc value
    // therefore we must not drop `arc`
    std::mem::forget(arc);
}

// #[derive(Debug)]
pub struct JoinHandle<T> {
    /// The receiver states that the result is safe to cast
    rx: Arc<BoundedReceiver<Box<dyn Any + Send>>>,
    has_awaited: Arc<AtomicBool>,
    recv_fut: Option<BoxedFutureResult>,
    marker: PhantomData<T>,
    // jh_fut: Pin<Box<JoinHandleFut>>,
}

impl<T> UnwindSafe for JoinHandle<T> {}
unsafe impl<T> Send for JoinHandle<T> {}
unsafe impl<T> Sync for JoinHandle<T> {}

impl<T> Future for JoinHandle<T>
where
    T: Unpin + 'static,
{
    type Output = Result<T, JoinHandleError>;

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
                    msg: "TODO need to propagate error".into(),
                }))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

#[derive(Debug)]
pub struct JoinHandleError {
    msg: String,
}

impl Display for JoinHandleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.msg)
    }
}

impl Error for JoinHandleError {}
