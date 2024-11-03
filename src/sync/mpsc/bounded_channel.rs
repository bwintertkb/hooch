use std::{
    future::poll_fn,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{Receiver, RecvError, SendError, TryRecvError},
        Arc,
    },
    task::{Context, Poll},
};

use mio::Waker;

use crate::reactor::{Reactor, ReactorTag};

#[derive(Debug)]
pub struct BoundedSender<T> {
    sender: std::sync::mpsc::SyncSender<T>,
    poll_waker: Arc<Waker>,
    queue_cnt: Arc<AtomicUsize>,
    reactor_tag: ReactorTag,
    reactor: &'static Reactor,
    clone_count: Arc<AtomicUsize>,
}

unsafe impl<T> Send for BoundedSender<T> {}

impl<T> BoundedSender<T> {
    pub fn send(&self, val: T) -> Result<(), SendError<T>> {
        self.sender.send(val)?;
        self.queue_cnt.fetch_add(1, Ordering::Relaxed);
        self.reactor.register_reactor_tag(self.reactor_tag);
        self.poll_waker.wake().unwrap();
        Ok(())
    }
}

impl<T> Clone for BoundedSender<T> {
    fn clone(&self) -> Self {
        self.clone_count.fetch_add(1, Ordering::Relaxed);
        Self {
            sender: self.sender.clone(),
            poll_waker: Arc::clone(&self.poll_waker),
            queue_cnt: Arc::clone(&self.queue_cnt),
            reactor_tag: self.reactor_tag,
            reactor: Reactor::get(),
            clone_count: Arc::clone(&self.clone_count),
        }
    }
}

impl<T> Drop for BoundedSender<T> {
    fn drop(&mut self) {
        let count = self.clone_count.fetch_sub(1, Ordering::Relaxed);
        if count == 0 {
            Reactor::get().remove_tag(self.reactor_tag);
        }
    }
}

#[derive(Debug)]
pub struct BoundedReceiver<T> {
    rx: Receiver<T>,
    queue_cnt: Arc<AtomicUsize>,
    reactor_tag: ReactorTag,
}

unsafe impl<T> Send for BoundedReceiver<T> {}
unsafe impl<T> Sync for BoundedReceiver<T> {}

impl<T> BoundedReceiver<T> {
    pub async fn recv(&self) -> Result<T, RecvError> {
        poll_fn(|cx| self.poll_recv(cx)).await
    }

    fn poll_recv(&self, ctx: &mut Context) -> Poll<Result<T, RecvError>> {
        if self.queue_cnt.load(Ordering::Acquire) == 0 {
            let reactor = Reactor::get();
            reactor.store_waker_channel(self.reactor_tag, ctx.waker().clone());
            return Poll::Pending;
        }

        let v = self.rx.try_recv().map_err(|_| RecvError);
        if v.is_ok() {
            self.queue_cnt.fetch_sub(1, Ordering::Relaxed);
        }
        Poll::Ready(v)
    }

    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        self.rx.try_recv()
    }
}

pub fn bounded_channel<T>(bound: usize) -> (BoundedSender<T>, BoundedReceiver<T>) {
    let (tx, rx) = std::sync::mpsc::sync_channel(bound);
    let reactor = Reactor::get();
    let mio_waker = Arc::clone(reactor.mio_waker());
    let queue_cnt = Arc::new(AtomicUsize::new(0));
    let reactor_tag = Reactor::generate_reactor_tag();
    (
        BoundedSender {
            sender: tx,
            poll_waker: mio_waker,
            queue_cnt: Arc::clone(&queue_cnt),
            reactor_tag,
            reactor: Reactor::get(),
            clone_count: Arc::new(AtomicUsize::new(1)),
        },
        BoundedReceiver {
            rx,
            queue_cnt,
            reactor_tag,
        },
    )
}
