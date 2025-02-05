use std::sync::{Arc, Mutex};

use hooch::{pool::thread_pool::HoochPool, runtime::RuntimeBuilder};

#[test]
fn test_runs_on_pool_thread() {
    let handle_executor = RuntimeBuilder::new().num_workers(1).build();

    let blocking = Arc::new(Mutex::new(0));

    let blocking_clone = Arc::clone(&blocking);

    handle_executor.run_blocking(async move {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let block_fn = move || {
            *blocking_clone.lock().unwrap() += 1;
            tx.send(()).unwrap();
        };

        let hp = HoochPool::get();

        hp.execute(Box::new(block_fn));
        rx.recv().unwrap()
    });

    assert!(*blocking.lock().unwrap() == 1)
}
