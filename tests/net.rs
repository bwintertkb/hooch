use std::sync::{Arc, Mutex};

pub use hooch::net::HoochTcpListener;
use hooch::runtime::{Handle, RuntimeBuilder};

fn build_runtime() -> Handle {
    RuntimeBuilder::default().build()
}

fn get_free_port() -> std::io::Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?; // Bind to any available port
    listener.local_addr().map(|addr| addr.port())
}

#[test]
fn test_accept_tcp_listener() {
    let port = get_free_port().unwrap();
    let addr = format!("localhost:{}", port);
    let addr_clone = addr.clone();

    let result = Arc::new(Mutex::new(false));

    let result_clone = Arc::clone(&result);

    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let (tx_bind, rx_bind) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(|| {
        let runtime_handle = build_runtime();

        runtime_handle.run_blocking(async move {
            let listener = HoochTcpListener::bind(addr_clone).await.unwrap();
            let _ = tx_bind.send(());
            if listener.accept().await.is_ok() {
                *result_clone.lock().unwrap() = true;
            }
            let _ = tx.send(());
        })
    });

    rx_bind.recv().unwrap();

    std::thread::spawn(|| {
        let _ = std::net::TcpStream::connect(addr);
    });

    rx.recv().unwrap();

    assert!(*result.lock().unwrap());
}
