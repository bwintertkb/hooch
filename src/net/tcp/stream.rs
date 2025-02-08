//! This module provides an asynchronous TCP stream implementation using the `mio` crate
//! for non-blocking I/O, a custom reactor (`Reactor`), and a thread pool (`HoochPool`).
//!
//! The primary type, [`HoochTcpStream`], offers asynchronous methods for connecting,
//! reading from, and writing to a TCP stream. Under the hood, it offloads blocking
//! operations (like the initial TCP connection) to a thread pool and uses a custom
//! reactor to manage I/O events.

use std::{
    future::Future,
    io::{ErrorKind, Read, Write},
    net::ToSocketAddrs,
    sync::{Arc, Mutex},
    task::Poll,
};

use mio::Interest;

use crate::{pool::thread_pool::HoochPool, reactor::Reactor};

/// An asynchronous TCP stream that wraps a non-blocking `mio::net::TcpStream`.
///
/// [`HoochTcpStream`] leverages a custom reactor to handle I/O readiness events and a thread pool
/// to offload the initial blocking connection operation. It provides asynchronous methods to
/// connect, read from, and write to the underlying stream.
pub struct HoochTcpStream {
    /// The underlying non-blocking TCP stream.
    pub stream: mio::net::TcpStream,
    /// A unique token associated with the stream for event registration with the reactor.
    pub token: mio::Token,
}

impl HoochTcpStream {
    /// Asynchronously connects to the specified socket address and returns a new [`HoochTcpStream`].
    ///
    /// # Arguments
    ///
    /// * `addr` - An address that can be converted into a socket address (e.g., a `&str` or `SocketAddr`).
    ///
    /// # Errors
    ///
    /// Returns an error if the connection fails.
    pub async fn connect(addr: impl ToSocketAddrs) -> std::io::Result<Self> {
        // Generate a unique reactor tag for this connection attempt.
        let reactor_tag = Reactor::generate_reactor_tag();
        let reactor = Reactor::get();
        reactor.register_reactor_tag(reactor_tag);

        // Shared state to store the connection result from the thread pool.
        let stream_handle: Arc<Mutex<Option<Result<mio::net::TcpStream, std::io::Error>>>> =
            Arc::new(Mutex::default());

        // Create a future that will attempt the connection asynchronously.
        let mut async_hooch_tcp_stream = Box::pin(AsyncHoochTcpStream {
            addr,
            state: Arc::clone(&stream_handle),
            has_polled: false,
        });

        // Poll the connection future until it is ready.
        let mut stream =
            std::future::poll_fn(|cx| async_hooch_tcp_stream.as_mut().poll(cx)).await?;

        // Get a unique token from the reactor to register the stream for I/O events.
        let reactor = Reactor::get();
        let token = reactor.unique_token();

        // Register the stream with the reactor for both readability and writability events.
        Reactor::get().registry().register(
            &mut stream,
            token,
            Interest::READABLE | Interest::WRITABLE,
        )?;

        Ok(Self { stream, token })
    }

    /// Asynchronously reads data from the TCP stream into the provided buffer.
    ///
    /// This method will poll the stream until it becomes readable. It then reads data into the buffer
    /// and returns the number of bytes read.
    ///
    /// # Arguments
    ///
    /// * `buf` - A mutable byte slice to store the read data.
    ///
    /// # Errors
    ///
    /// Returns an error if the read operation fails.
    pub async fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            match self.stream.read(buf) {
                Ok(num) => return Ok(num),
                // If the stream is not ready for reading, wait for the reactor to signal readiness.
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    std::future::poll_fn(|cx| Reactor::get().poll(self.token, cx)).await?
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Asynchronously writes data from the provided buffer to the TCP stream.
    ///
    /// This method will poll the stream until it becomes writable. It then writes data from the buffer
    /// to the stream and returns the number of bytes written.
    ///
    /// # Arguments
    ///
    /// * `buf` - A byte slice containing the data to write.
    ///
    /// # Errors
    ///
    /// Returns an error if the write operation fails.
    pub async fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        loop {
            match self.stream.write(buf) {
                Ok(num) => return Ok(num),
                // If the stream is not ready for writing, wait for the reactor to signal readiness.
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    std::future::poll_fn(|cx| Reactor::get().poll(self.token, cx)).await?
                }
                Err(e) => return Err(e),
            }
        }
    }
}

/// A future that resolves to a non-blocking TCP stream once the connection attempt completes.
///
/// This future offloads the blocking `std::net::TcpStream::connect` call to a thread pool,
/// then converts the connected stream to a non-blocking `mio::net::TcpStream`.
struct AsyncHoochTcpStream<T: ToSocketAddrs> {
    /// The target address to connect to.
    addr: T,
    /// Shared state that will eventually hold the connection result.
    state: Arc<Mutex<Option<std::io::Result<mio::net::TcpStream>>>>,
    /// A flag to ensure the connection attempt is only initiated once.
    has_polled: bool,
}

impl<T: ToSocketAddrs> Future for AsyncHoochTcpStream<T> {
    type Output = Result<mio::net::TcpStream, std::io::Error>;

    /// Attempts to establish a TCP connection asynchronously.
    ///
    /// The first time `poll` is called, the connection operation is offloaded to a thread pool.
    /// Subsequent polls check the shared state for the connection result.
    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        // On the first poll, initiate the connection attempt.
        if !self.has_polled {
            // SAFETY: We have exclusive access to the future because we are in poll.
            let this = unsafe { self.as_mut().get_unchecked_mut() };
            this.has_polled = true;

            // Clone the shared state to be moved into the thread pool task.
            let listener_handle_clone = Arc::clone(&self.state);
            // Convert the address to a concrete socket address.
            let socket_addr = self.addr.to_socket_addrs().unwrap().next().unwrap();

            // Clone the waker to notify when the connection attempt is complete.
            let waker = cx.waker().clone();
            let connect_fn = move || {
                // Attempt to connect and set the socket to non-blocking mode.
                let result = move || {
                    let stream = std::net::TcpStream::connect(socket_addr)?;
                    stream.set_nonblocking(true)?;
                    Ok(mio::net::TcpStream::from_std(stream))
                };

                // Store the result of the connection attempt in the shared state.
                let stream_result = result();
                *listener_handle_clone.lock().unwrap() = Some(stream_result);
                // Wake up the task that is waiting for the connection to complete.
                waker.wake();
            };

            // Execute the connection function in the thread pool.
            let pool = HoochPool::get();
            pool.execute(Box::new(connect_fn));
            return Poll::Pending;
        }

        // If the connection result is not yet available, continue waiting.
        if self.state.lock().unwrap().is_none() {
            return Poll::Pending;
        }

        // Retrieve and return the connection result.
        let listener_result = self.state.lock().unwrap().take().unwrap();
        Poll::Ready(listener_result)
    }
}
