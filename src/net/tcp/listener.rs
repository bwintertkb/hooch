//! This module provides an asynchronous TCP listener implementation using the `mio` crate
//! for non-blocking I/O, a custom reactor (`Reactor`), and a thread pool (`HoochPool`).
//!
//! The primary type, [`HoochTcpListener`], allows binding to a socket address asynchronously
//! and accepting incoming connections. Each accepted connection is wrapped in a [`HoochTcpStream`],
//! which provides asynchronous read and write capabilities.

use std::{
    future::Future,
    io::ErrorKind,
    net::{SocketAddr, ToSocketAddrs},
    ops::Deref,
    sync::{Arc, Mutex},
    task::Poll,
};

use mio::{Interest, Token};

use crate::{net::HoochTcpStream, pool::thread_pool::HoochPool, reactor::Reactor};

/// An asynchronous TCP listener based on `mio::net::TcpListener`.
///
/// [`HoochTcpListener`] provides an interface to bind to a socket address asynchronously,
/// and accept incoming connections. Accepted connections are wrapped in a [`HoochTcpStream`].
pub struct HoochTcpListener {
    listener: mio::net::TcpListener,
    token: Token,
}

impl HoochTcpListener {
    /// Asynchronously binds to the provided socket address.
    ///
    /// This method offloads the blocking bind operation to a thread pool, then registers
    /// the resulting listener with the reactor for non-blocking I/O events.
    ///
    /// # Arguments
    ///
    /// * `addr` - An address that can be converted into a socket address (e.g., a `&str` or `SocketAddr`).
    ///
    /// # Errors
    ///
    /// Returns an error if the bind operation fails.
    pub async fn bind(addr: impl ToSocketAddrs) -> std::io::Result<Self> {
        // Generate a reactor tag for this bind operation.
        let reactor_tag = Reactor::generate_reactor_tag();
        let reactor = Reactor::get();
        reactor.register_reactor_tag(reactor_tag);

        // Shared state to hold the result of the bind operation from the thread pool.
        let listener_handle: Arc<Mutex<Option<Result<mio::net::TcpListener, std::io::Error>>>> =
            Arc::new(Mutex::default());

        // Create a future that will attempt to bind the listener asynchronously.
        let mut async_hooch_tcp_listener = Box::pin(AsyncHoochTcpListener {
            addr,
            state: Arc::clone(&listener_handle),
            has_polled: false,
        });

        // Poll the future until the binding operation completes.
        let mut listener =
            std::future::poll_fn(|cx| async_hooch_tcp_listener.as_mut().poll(cx)).await?;

        // Obtain a unique token for the listener and register it with the reactor.
        let reactor = Reactor::get();
        let token = reactor.unique_token();

        Reactor::get().registry().register(
            &mut listener,
            token,
            Interest::READABLE | Interest::WRITABLE,
        )?;

        Ok(Self { listener, token })
    }

    /// Asynchronously accepts a new incoming TCP connection.
    ///
    /// This method continuously polls the listener until a new connection is accepted.
    /// Upon acceptance, it registers the new connection with the reactor and wraps it in a
    /// [`HoochTcpStream`].
    ///
    /// # Errors
    ///
    /// Returns an error if accepting a connection fails.
    pub async fn accept(&self) -> std::io::Result<(HoochTcpStream, SocketAddr)> {
        loop {
            match self.listener.accept() {
                Ok(mut stream) => {
                    let reactor = Reactor::get();
                    let token = reactor.unique_token();

                    // Register the accepted stream with the reactor for I/O events.
                    Reactor::get().registry().register(
                        &mut stream.0,
                        token,
                        Interest::READABLE | Interest::WRITABLE,
                    )?;

                    return Ok((
                        HoochTcpStream {
                            stream: stream.0,
                            token,
                        },
                        stream.1,
                    ));
                }
                // If no connection is ready, wait until the listener is readable.
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    std::future::poll_fn(|cx| Reactor::get().poll(self.token, cx)).await?
                }
                Err(error) => return Err(error),
            }
        }
    }
}

impl Deref for HoochTcpListener {
    type Target = mio::net::TcpListener;

    /// Dereferences to the underlying `mio::net::TcpListener`.
    fn deref(&self) -> &Self::Target {
        &self.listener
    }
}

/// A future that resolves to a non-blocking TCP listener once the bind operation completes.
///
/// This future offloads the blocking `TcpListener::bind` call to a thread pool,
/// then converts the bound listener into a non-blocking `mio::net::TcpListener`.
struct AsyncHoochTcpListener<T: ToSocketAddrs> {
    /// The address to bind to.
    addr: T,
    /// Shared state that will eventually hold the result of the bind operation.
    state: Arc<Mutex<Option<std::io::Result<mio::net::TcpListener>>>>,
    /// A flag to ensure the bind operation is initiated only once.
    has_polled: bool,
}

impl<T: ToSocketAddrs> Future for AsyncHoochTcpListener<T> {
    type Output = Result<mio::net::TcpListener, std::io::Error>;

    /// Polls the future to attempt binding the TCP listener.
    ///
    /// On the first poll, the binding operation is offloaded to a thread pool. Subsequent polls
    /// check the shared state for the result of the bind operation.
    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        // Initiate the binding operation on the first poll.
        if !self.has_polled {
            // SAFETY: We have exclusive access to the future.
            let this = unsafe { self.as_mut().get_unchecked_mut() };
            this.has_polled = true;

            // Clone the shared state for use in the thread pool task.
            let listener_handle_clone = Arc::clone(&self.state);
            // Convert the provided address into a concrete socket address.
            let socket_addr = self.addr.to_socket_addrs().unwrap().next().unwrap();

            // Clone the waker to notify the current task once the binding completes.
            let waker = cx.waker().clone();
            let connect_fn = move || {
                // Perform the blocking bind operation.
                let result = move || {
                    let std_listener = std::net::TcpListener::bind(socket_addr)?;
                    std_listener.set_nonblocking(true)?;
                    Ok(mio::net::TcpListener::from_std(std_listener))
                };

                // Store the result in the shared state.
                let listener_result = result();
                *listener_handle_clone.lock().unwrap() = Some(listener_result);
                // Wake up the task waiting on this future.
                waker.wake();
            };

            // Execute the binding operation in the thread pool.
            let pool = HoochPool::get();
            pool.execute(Box::new(connect_fn));
            return Poll::Pending;
        }

        // Continue waiting if the binding result is not yet available.
        if self.state.lock().unwrap().is_none() {
            return Poll::Pending;
        }

        // Retrieve and return the binding result.
        let listener_result = self.state.lock().unwrap().take().unwrap();
        Poll::Ready(listener_result)
    }
}
