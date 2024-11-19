use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use hooch::{runtime::RuntimeBuilder, spawner::Spawner, time::sleep};

#[test]
fn test_join_handle() {
    let runtime_handle = RuntimeBuilder::default().build();

    runtime_handle.run_blocking(async {
        // We need to make sure we have a runtime on this thread
        RuntimeBuilder::default().build();
        let handle = hooch::spawner::Spawner::spawn(async { 3 });
        let value = handle.await.unwrap();

        assert!(value == 3)
    })
}

#[test]
fn test_abort_task() {
    let runtime_handle = RuntimeBuilder::new().num_workers(3).build();
    let ctr = Arc::new(Mutex::new(0));
    let ctr_clone = Arc::clone(&ctr);
    let sleep_ms_top = 500;
    let sleep_ms_inner = 10;
    runtime_handle.run_blocking(async move {
        let jh = Spawner::spawn(async move {
            sleep(Duration::from_millis(sleep_ms_inner)).await;
            *ctr_clone.lock().unwrap() += 1;
        });
        jh.abort();
        std::thread::sleep(std::time::Duration::from_millis(sleep_ms_top));
    });

    assert!(*ctr.lock().unwrap() == 0);
}
