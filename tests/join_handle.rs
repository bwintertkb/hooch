use myasyncruntime::runtime::RuntimeBuilder;

#[test]
fn test_join_handle() {
    let runtime_handle = RuntimeBuilder::default().build();

    runtime_handle.run_blocking(async {
        // We need to make sure we have a runtime on this thread
        RuntimeBuilder::default().build();
        let handle = myasyncruntime::spawner::Spawner::spawn(async { 3 });
        let value = handle.await.unwrap();

        assert!(value == 3)
    })
}
