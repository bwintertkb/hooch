//! This module defines a `Reactor` struct that manages asynchronous events and
//! allows tasks to register, await, and wake based on those events. The `Reactor`
//! uses a `mio::Poll` instance to handle I/O events and leverages `Waker`s to
//! signal readiness to the executor.

use std::{
    collections::{hash_map::Entry, HashMap},
    io,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex, OnceLock,
    },
    task::{Context, Poll, Waker},
};

use crate::{executor::Status, utils::ring_buffer::LockFreeBoundedRingBuffer};
use mio::{Registry, Token, Waker as MioWaker};

/// Token used for waker notifications within the reactor.
const WAKER_TOKEN: Token = Token(0);
/// Atomic counter for generating unique `ReactorTag`s.
static REACTOR_TAG_NUM: AtomicUsize = AtomicUsize::new(0);

/// Unique identifier for reactor tags, used to manage and track events.
#[derive(Debug, Hash, Eq, PartialEq, Clone, Copy)]
pub struct ReactorTag(usize);

/// Enum representing the type of event associated with a `ReactorTag`.
/// Currently, it supports only channel-based wakers.
#[derive(Debug)]
pub enum TagType {
    /// A channel-based event, which uses a `Waker` to notify.
    Channel(Waker),
}

/// The `Reactor` struct manages registration, awaiting, and polling of asynchronous events.
/// It uses `mio::Poll` for event notification and a lock-free ring buffer for managing reactor tags.
#[derive(Debug)]
pub struct Reactor {
    /// `Registry` from `mio` used for event registration.
    registry: Registry,
    /// `mio::Waker` used for signaling the reactor loop.
    mio_waker: Arc<MioWaker>,
    /// Buffer to store `ReactorTag`s for pending events.
    reactor_tag_buffer: LockFreeBoundedRingBuffer<ReactorTag>,
    /// Mapping of `ReactorTag`s to their corresponding event types.
    reactor_tags: Mutex<HashMap<ReactorTag, TagType>>,
    /// Mapping of tokens to their current status, used to track event readiness.
    statuses: Mutex<HashMap<Token, Status>>,
}

impl Reactor {
    /// Retrieves the singleton instance of the reactor.
    /// This will initialize the reactor and spawn a background thread if it has not been created.
    ///
    /// # Returns
    /// A reference to the singleton `Reactor` instance.
    pub fn get() -> &'static Self {
        static REACTOR: OnceLock<Reactor> = OnceLock::new();

        REACTOR.get_or_init(|| {
            let poll = mio::Poll::new().unwrap();
            let mio_waker = MioWaker::new(poll.registry(), Self::waker_token()).unwrap();
            let reactor = Reactor {
                registry: poll.registry().try_clone().unwrap(),
                mio_waker: Arc::new(mio_waker),
                reactor_tag_buffer: LockFreeBoundedRingBuffer::new(1024 * 1024),
                reactor_tags: Mutex::new(HashMap::new()),
                statuses: Mutex::new(HashMap::new()),
            };

            // Spawn the reactor's background event loop.
            std::thread::Builder::new()
                .name("reactor".into())
                .spawn(|| run(poll))
                .unwrap();

            reactor
        })
    }

    /// Generates a unique `ReactorTag` identifier.
    pub fn generate_reactor_tag() -> ReactorTag {
        ReactorTag(REACTOR_TAG_NUM.fetch_add(1, Ordering::Relaxed))
    }

    /// Generates a unique `Token` for registering new events with the reactor.
    pub fn unique_token(&self) -> Token {
        static CURRENT_TOKEN: AtomicUsize = AtomicUsize::new(1);
        Token(CURRENT_TOKEN.fetch_add(1, Ordering::Relaxed))
    }

    /// Returns the waker token used by the reactor.
    pub fn waker_token() -> Token {
        WAKER_TOKEN
    }

    /// Returns a reference to the reactor's `mio::Waker`.
    pub fn mio_waker(&self) -> &Arc<MioWaker> {
        &self.mio_waker
    }

    /// Stores a token with in an awaited state
    pub fn status_store(&self, token: Token, waker: Waker) {
        self.statuses
            .lock()
            .unwrap()
            .insert(token, Status::Awaited(waker));
    }

    /// Check if a token has progressed
    pub fn has_token_progressed(&self, token: Token) -> bool {
        let lock = self.statuses.lock().unwrap();
        let Some(status) = lock.get(&token) else {
            println!("TOKEN {:?} not in statuses", token);
            return false;
        };

        matches!(status, Status::Happened)
    }

    /// Stores a waker in the reactor's tag map, associating it with a `ReactorTag`.
    ///
    /// # Parameters
    /// - `reactor_tag`: The tag for the event.
    /// - `waker`: The waker to be stored, which will be used for notifying readiness.
    pub fn store_waker_channel(&self, reactor_tag: ReactorTag, waker: Waker) {
        self.reactor_tags
            .lock()
            .unwrap()
            .insert(reactor_tag, TagType::Channel(waker));
    }

    /// Registers a `ReactorTag` in the tag buffer, marking it as pending.
    ///
    /// # Parameters
    /// - `tag`: The reactor tag to register as pending.
    pub fn register_reactor_tag(&self, tag: ReactorTag) {
        self.reactor_tag_buffer.push(tag).unwrap();
    }

    /// Polls the reactor for the given `Token` and returns the readiness status.
    ///
    /// # Parameters
    /// - `token`: The event token to check for readiness.
    /// - `cx`: The task context, which provides a `Waker` if the task is pending.
    ///
    /// # Returns
    /// - `Poll::Pending` if the event is not ready.
    /// - `Poll::Ready` if the event has occurred.
    pub fn poll(&self, token: Token, cx: &mut Context) -> Poll<io::Result<()>> {
        let mut guard = self.statuses.lock().unwrap();
        match guard.entry(token) {
            Entry::Vacant(vacant) => {
                vacant.insert(Status::Awaited(cx.waker().clone()));
                Poll::Pending
            }
            Entry::Occupied(mut occupied) => match occupied.get() {
                Status::Awaited(waker) => {
                    // Skip clone if wakers are the same
                    if !waker.will_wake(cx.waker()) {
                        occupied.insert(Status::Awaited(cx.waker().clone()));
                    }
                    Poll::Pending
                }
                Status::Happened => {
                    occupied.remove();
                    Poll::Ready(Ok(()))
                }
            },
        }
    }

    /// Removes a `ReactorTag` from the tag map, effectively unregistering it.
    ///
    /// # Parameters
    /// - `reactor_tag`: The tag to be removed.
    pub fn remove_tag(&self, reactor_tag: ReactorTag) {
        self.reactor_tags.lock().unwrap().remove(&reactor_tag);
    }

    /// Returns a reference to the reactor's `Registry`.
    pub fn registry(&self) -> &Registry {
        &self.registry
    }
}

/// Main event loop for the reactor, which continuously polls for events and handles them.
/// This function should be run in a background thread.
///
/// # Parameters
/// - `poll`: The `mio::Poll` instance used for event polling.
fn run(mut poll: mio::Poll) -> ! {
    let reactor = Reactor::get();
    let mut events = mio::Events::with_capacity(1024);

    loop {
        poll.poll(&mut events, None).unwrap();

        for event in events.iter() {
            match event.token() {
                WAKER_TOKEN => {
                    // Process waker events by waking all pending tags
                    while let Some(reactor_tag) = reactor.reactor_tag_buffer.pop() {
                        if let Some(tag_type) =
                            reactor.reactor_tags.lock().unwrap().get(&reactor_tag)
                        {
                            match tag_type {
                                TagType::Channel(waker) => {
                                    waker.wake_by_ref();
                                }
                            }
                        }
                    }
                }
                _ => {
                    let mut guard = reactor.statuses.lock().unwrap();

                    // Mark the event as happened and wake the associated waker if it exists
                    let previous = guard.insert(event.token(), Status::Happened);
                    if let Some(Status::Awaited(waker)) = previous {
                        waker.wake();
                    }
                }
            }
        }
    }
}
