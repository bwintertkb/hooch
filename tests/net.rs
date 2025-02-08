use std::{
    io::{Read, Write},
    sync::{Arc, Mutex},
};

pub use hooch::net::HoochTcpListener;
use hooch::{
    net::HoochTcpStream,
    runtime::{Handle, RuntimeBuilder},
};

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

#[test]
fn test_tcp_stream_connect() {
    let port = get_free_port().unwrap();
    let addr = format!("localhost:{}", port);
    let addr_clone = addr.clone();

    let (tx_listen, rx_listen) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let listener = std::net::TcpListener::bind(addr_clone).unwrap();
        let _ = tx_listen.send(());
        let _ = listener.accept();
    });

    rx_listen.recv().unwrap();

    let runtime_handler = build_runtime();
    let result = runtime_handler.run_blocking(async move { HoochTcpStream::connect(addr).await });

    assert!(result.is_ok())
}

#[test]
fn test_tcp_stream_read() {
    let port = get_free_port().unwrap();
    let addr = format!("localhost:{}", port);
    let addr_clone = addr.clone();

    let (tx_listen, rx_listen) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let listener = std::net::TcpListener::bind(addr_clone).unwrap();
        let _ = tx_listen.send(());
        let (mut stream, _) = listener.accept().unwrap();
        stream.write_all(&[1]).unwrap();
    });

    rx_listen.recv().unwrap();

    let runtime_handler = build_runtime();
    let result = runtime_handler.run_blocking(async move {
        let mut buffer = [0; 1];
        let mut hooch_stream = HoochTcpStream::connect(addr).await.unwrap();
        hooch_stream.read(&mut buffer).await.unwrap();
        buffer
    });

    assert!(result == [1])
}

#[test]
fn test_tcp_stream_write() {
    let port = get_free_port().unwrap();
    let addr = format!("localhost:{}", port);
    let addr_clone = addr.clone();

    let (tx_listen, rx_listen) = std::sync::mpsc::sync_channel(1);
    let handle = std::thread::spawn(move || {
        let listener = std::net::TcpListener::bind(addr_clone).unwrap();
        let _ = tx_listen.send(());
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0; 1];
        stream.read_exact(&mut buf).unwrap();
        buf
    });

    rx_listen.recv().unwrap();

    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let runtime_handler = build_runtime();
    runtime_handler.run_blocking(async move {
        let buffer = [3; 1];
        let mut hooch_stream = HoochTcpStream::connect(addr).await.unwrap();
        hooch_stream.write(&buffer).await.unwrap();
        let _ = tx.send(());
    });

    rx.recv().unwrap();
    let result = handle.join().unwrap();
    assert!(result == [3])
}
