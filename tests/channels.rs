use std::sync::{Arc, Mutex};

use myasyncruntime::{self, runtime::RuntimeBuilder, sync::mpsc::bounded_channel};

#[test]
fn test_mychannels() {
    let (mytx, myrx) = bounded_channel::<i32>(1000);
    let runtime_handle = RuntimeBuilder::default().build();

    std::thread::Builder::new()
        .name("sender".into())
        .spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            let _ = mytx.send(1);
        })
        .unwrap();

    let result = Arc::new(Mutex::new(0));
    let result_clone = Arc::clone(&result);
    runtime_handle.run_blocking(async move {
        let rx_res = myrx.recv().await.unwrap();
        *result_clone.lock().unwrap() = rx_res;
    });

    assert!(*result.lock().unwrap() == 1);
}
