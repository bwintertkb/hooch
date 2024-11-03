use std::{
    collections::{hash_map::Entry, HashMap},
    io,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex, OnceLock,
    },
    task::{Context, Poll, Waker},
};

use mio::{Registry, Token, Waker as MioWaker};

use crate::{executor::Status, utils::ring_buffer::LockFreeBoundedRingBuffer};

const WAKER_TOKEN: Token = Token(0);
static REACTOR_TAG_NUM: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Hash, Eq, PartialEq, Clone, Copy)]
pub struct ReactorTag(usize);

#[derive(Debug)]
pub enum TagType {
    Channel(Waker),
}

#[derive(Debug)]
pub struct Reactor {
    registry: Registry,
    mio_waker: Arc<MioWaker>,
    reactor_tag_buffer: LockFreeBoundedRingBuffer<ReactorTag>,
    reactor_tags: Mutex<HashMap<ReactorTag, TagType>>,
    statuses: Mutex<HashMap<Token, Status>>,
}

impl Reactor {
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

            std::thread::Builder::new()
                .name("reactor".into())
                .spawn(|| run(poll))
                .unwrap();

            reactor
        })
    }

    pub fn generate_reactor_tag() -> ReactorTag {
        ReactorTag(REACTOR_TAG_NUM.fetch_add(1, Ordering::Relaxed))
    }

    pub fn unique_token(&self) -> Token {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static CURRENT_TOKEN: AtomicUsize = AtomicUsize::new(1);
        Token(CURRENT_TOKEN.fetch_add(1, Ordering::Relaxed))
    }

    pub fn waker_token() -> Token {
        WAKER_TOKEN
    }

    pub fn mio_waker(&self) -> &Arc<MioWaker> {
        &self.mio_waker
    }

    pub fn store_waker_channel(&self, reactor_tag: ReactorTag, waker: Waker) {
        self.reactor_tags
            .lock()
            .unwrap()
            .insert(reactor_tag, TagType::Channel(waker));
    }

    pub fn register_reactor_tag(&self, tag: ReactorTag) {
        self.reactor_tag_buffer.push(tag).unwrap();
    }

    pub fn poll(&self, token: Token, cx: &mut Context) -> Poll<io::Result<()>> {
        let mut guard = self.statuses.lock().unwrap();
        match guard.entry(token) {
            Entry::Vacant(vacant) => {
                vacant.insert(Status::Awaited(cx.waker().clone()));
                Poll::Pending
            }
            Entry::Occupied(mut occupied) => {
                match occupied.get() {
                    Status::Awaited(waker) => {
                        // skip clone is wakers are the same
                        if !waker.will_wake(cx.waker()) {
                            occupied.insert(Status::Awaited(cx.waker().clone()));
                        }
                        Poll::Pending
                    }
                    Status::Happened => {
                        occupied.remove();
                        Poll::Ready(Ok(()))
                    }
                }
            }
        }
    }

    pub fn remove_tag(&self, reactor_tag: ReactorTag) {
        self.reactor_tags.lock().unwrap().remove(&reactor_tag);
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }
}

fn run(mut poll: mio::Poll) -> ! {
    let reactor = Reactor::get();
    let mut events = mio::Events::with_capacity(1024);

    loop {
        poll.poll(&mut events, None).unwrap();

        for event in events.iter() {
            match event.token() {
                WAKER_TOKEN => {
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

                    let previous = guard.insert(event.token(), Status::Happened);

                    if let Some(Status::Awaited(waker)) = previous {
                        // Send the future to the executor
                        waker.wake();
                    }
                }
            }
        }
    }
}
