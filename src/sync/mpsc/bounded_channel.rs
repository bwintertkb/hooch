//! A module providing a bounded channel with integration for async
//! operations and wake-ups via a custom reactor. The `BoundedSender`
//! and `BoundedReceiver` types allow for message passing with a
//! specified capacity, registering with the reactor to handle async
//! wake-up events.

use core::fmt;
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

/// A bounded sender that allows sending messages with a fixed capacity.
/// The sender registers with the reactor upon sending, enabling
/// asynchronous wake-ups for tasks waiting to receive messages.
pub struct BoundedSender<T> {
    /// Inner sender used for synchronized message passing.
    sender: std::sync::mpsc::SyncSender<T>,
    /// Waker to notify the reactor when a message is sent.
    poll_waker: Arc<Waker>,
    /// Atomic counter to track the number of items in the queue.
    queue_cnt: Arc<AtomicUsize>,
    /// Unique tag to identify this channel with the reactor.
    reactor_tag: ReactorTag,
    /// Reference to the global reactor for registration and wake-ups.
    reactor: &'static Reactor,
    /// Tracks the number of clones of this sender.
    clone_count: Arc<AtomicUsize>,
}

impl<T> fmt::Debug for BoundedSender<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoundedSender")
            .field("sender", &self.sender)
            .field("poll_waker", &self.poll_waker)
            .field("queue_cnt", &self.queue_cnt.load(Ordering::Relaxed))
            .field("reactor_tag", &self.reactor_tag)
            .field("clone_count", &self.clone_count.load(Ordering::Relaxed))
            .finish()
    }
}

// Safety: `BoundedSender` is marked `Send` as it only contains items that are `Send`.
unsafe impl<T> Send for BoundedSender<T> {}

impl<T> BoundedSender<T> {
    /// Sends a value through the channel.
    ///
    /// # Arguments
    ///
    /// * `val` - The value to send through the channel.
    ///
    /// # Returns
    ///
    /// `Result<(), SendError<T>>` - Ok(()) if the value was sent successfully,
    /// or a `SendError` if the channel is disconnected.
    ///
    /// After sending, the reactor tag is registered, and the reactor
    /// is woken up to handle the new message.
    pub fn send(&self, val: T) -> Result<(), SendError<T>> {
        // Send the value through the inner sender, returns an error if it fails.
        self.sender.send(val)?;
        // Increment the queue count after a successful send.
        self.queue_cnt.fetch_add(1, Ordering::Relaxed);
        // Register the tag with the reactor for wake-ups.
        self.reactor.register_reactor_tag(self.reactor_tag);
        // Wake up the reactor to notify that a message is available.
        self.poll_waker.wake().unwrap();
        Ok(())
    }
}

impl<T> Clone for BoundedSender<T> {
    /// Clones the sender, increasing the reference count.
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
    /// Drops the sender and, if this is the last clone, removes its tag from the reactor.
    fn drop(&mut self) {
        let count = self.clone_count.fetch_sub(1, Ordering::Relaxed);
        if count == 0 {
            // Remove the tag from the reactor when there are no more clones.
            Reactor::get().remove_tag(self.reactor_tag);
        }
    }
}

/// A bounded receiver that allows asynchronous receipt of messages.
/// The receiver integrates with the reactor to wake up on new messages,
/// enabling async operations.
#[derive(Debug)]
pub struct BoundedReceiver<T> {
    /// Inner receiver for synchronized message receiving.
    rx: Receiver<T>,
    /// Atomic counter to track the number of items in the queue.
    queue_cnt: Arc<AtomicUsize>,
    /// Unique tag to identify this channel in the reactor.
    reactor_tag: ReactorTag,
}

// Safety: `BoundedReceiver` is marked `Send` and `Sync` as it only contains items that are `Send` and `Sync`.
unsafe impl<T> Send for BoundedReceiver<T> {}
unsafe impl<T> Sync for BoundedReceiver<T> {}

impl<T> BoundedReceiver<T> {
    /// Asynchronously receives a value from the channel.
    ///
    /// If there are no items in the queue, the receiver registers
    /// with the reactor to wait for new messages.
    ///
    /// # Returns
    ///
    /// `Result<T, RecvError>` - Returns the received value or a `RecvError` if the channel is closed.
    pub async fn recv(&self) -> Result<T, RecvError> {
        poll_fn(|cx| self.poll_recv(cx)).await
    }

    /// Polls the channel for a message, registering the task with the reactor
    /// if no messages are available.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The context of the current task.
    ///
    /// # Returns
    ///
    /// `Poll<Result<T, RecvError>>` - `Poll::Pending` if no messages are available,
    /// or `Poll::Ready` with the received value or `RecvError`.
    fn poll_recv(&self, ctx: &mut Context) -> Poll<Result<T, RecvError>> {
        // If the queue is empty, store the waker to notify the reactor when new messages arrive.
        if self.queue_cnt.load(Ordering::Acquire) == 0 {
            let reactor = Reactor::get();
            reactor.store_waker_channel(self.reactor_tag, ctx.waker().clone());
            return Poll::Pending;
        }

        // Try receiving a message; if successful, decrement the queue count.
        let v = self.rx.try_recv().map_err(|_| RecvError);
        if v.is_ok() {
            self.queue_cnt.fetch_sub(1, Ordering::Relaxed);
        }
        Poll::Ready(v)
    }

    /// Attempts to receive a message from the channel without blocking.
    ///
    /// # Returns
    ///
    /// `Result<T, TryRecvError>` - Returns the received value or an error if
    /// the channel is empty or closed.
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        self.rx.try_recv()
    }
}

/// Creates a new bounded channel with the specified capacity.
///
/// The function returns a pair of `BoundedSender` and `BoundedReceiver`,
/// which can be used for sending and receiving messages, respectively.
/// The channel integrates with the reactor for asynchronous operations.
///
/// # Arguments
///
/// * `bound` - The maximum number of messages the channel can hold.
///
/// # Returns
///
/// `(BoundedSender<T>, BoundedReceiver<T>)` - A pair of sender and receiver
/// for the bounded channel.
pub fn bounded_channel<T>(bound: usize) -> (BoundedSender<T>, BoundedReceiver<T>) {
    // Create a synchronized sender and receiver with the specified capacity.
    let (tx, rx) = std::sync::mpsc::sync_channel(bound);
    // Get a reference to the global reactor.
    let reactor = Reactor::get();
    // Clone the reactor's Mio waker for wake-up notifications.
    let mio_waker = Arc::clone(reactor.mio_waker());
    // Initialize the queue count to track the number of messages in the channel.
    let queue_cnt = Arc::new(AtomicUsize::new(0));
    // Generate a unique reactor tag for this channel.
    let reactor_tag = Reactor::generate_reactor_tag();

    // Return the sender and receiver, each with the necessary reactor information and queue count.
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
