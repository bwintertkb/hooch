use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc,
};

pub type BlockingFn = dyn FnOnce() + Send + 'static;

static BLOCKER_TAG_ID: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone, Copy)]
pub struct BlockerTag(usize);

impl BlockerTag {
    pub fn generate_tag() -> BlockerTag {
        BlockerTag(BLOCKER_TAG_ID.fetch_add(1, Ordering::Relaxed))
    }
}

#[derive(Debug)]
pub struct Blocker {
    rx: mpsc::Receiver<Box<BlockingFn>>,
    tag: BlockerTag,
}

impl Blocker {

    pub fn new(tag: BlockerTag)

    pub fn run(&self) {
        while let Ok(f) = self.0.recv() {
            f()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::{
        sync::{Arc, Mutex},
        task::{RawWaker, RawWakerVTable, Waker},
    };

    unsafe fn stub_fn_waker(data: *const ()) -> RawWaker {
        RawWaker::new(data, &WAKER_VTABLE)
    }
    unsafe fn stub_fn(_: *const ()) {}

    const WAKER_VTABLE: RawWakerVTable =
        RawWakerVTable::new(stub_fn_waker, stub_fn, stub_fn, stub_fn);

    fn create_stub_waker() -> Waker {
        let ptr = 1 as *const ();

        unsafe { Waker::from_raw(RawWaker::new(ptr, &WAKER_VTABLE)) }
    }

    #[test]
    fn test_execute_blocker() {
        let (tx, rx) = std::sync::mpsc::sync_channel(100);
        let (tx_ready, rx_ready) = std::sync::mpsc::sync_channel(100);
        let blocker = Blocker(rx);
        let ctr = Arc::new(Mutex::new(0));
        let ctr_clone = Arc::clone(&ctr);

        let f: Box<BlockingFn> = Box::new(move || {
            tx_ready.send(()).unwrap();
        });

        tx.send(f).unwrap();
        rx_ready.recv().unwrap();

        assert!(*ctr.lock().unwrap() == 1);
    }
}
