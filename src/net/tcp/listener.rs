use std::{
    future::Future,
    io::ErrorKind,
    net::{SocketAddr, ToSocketAddrs},
    ops::Deref,
    sync::{Arc, Mutex},
    task::Poll,
};

use mio::{Interest, Token};

use crate::{pool::thread_pool::HoochPool, reactor::Reactor};

pub struct HoochTcpListener {
    listener: mio::net::TcpListener,
    token: Token,
}
impl HoochTcpListener {
    pub async fn bind(addr: impl ToSocketAddrs) -> std::io::Result<Self> {
        let reactor_tag = Reactor::generate_reactor_tag();
        let reactor = Reactor::get();
        reactor.register_reactor_tag(reactor_tag);
        let listener_handle: Arc<Mutex<Option<Result<mio::net::TcpListener, std::io::Error>>>> =
            Arc::new(Mutex::default());

        let mut async_hooch_tcp_listener = Box::pin(AsyncHoochTcpListener {
            addr,
            state: Arc::clone(&listener_handle),
            has_polled: false,
        });

        let mut listener =
            std::future::poll_fn(|cx| async_hooch_tcp_listener.as_mut().poll(cx)).await?;
        let reactor = Reactor::get();
        let token = reactor.unique_token();

        Reactor::get().registry().register(
            &mut listener,
            token,
            Interest::READABLE | Interest::WRITABLE,
        )?;

        Ok(Self { listener, token })
    }

    // TODO change return to a hooch TcpStream
    pub async fn accept(&self) -> std::io::Result<(mio::net::TcpStream, SocketAddr)> {
        loop {
            match self.listener.accept() {
                Ok(stream) => return Ok(stream),
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

    fn deref(&self) -> &Self::Target {
        &self.listener
    }
}

struct AsyncHoochTcpListener<T: ToSocketAddrs> {
    addr: T,
    state: Arc<Mutex<Option<std::io::Result<mio::net::TcpListener>>>>,
    has_polled: bool,
}

impl<T: ToSocketAddrs> Future for AsyncHoochTcpListener<T> {
    type Output = Result<mio::net::TcpListener, std::io::Error>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        if !self.has_polled {
            let this = unsafe { self.as_mut().get_unchecked_mut() };
            this.has_polled = true;

            let listener_handle_clone = Arc::clone(&self.state);
            let socket_addr = self.addr.to_socket_addrs().unwrap().next().unwrap();

            let waker = cx.waker().clone();
            let connect_fn = move || {
                let result = move || {
                    let std_listener = std::net::TcpListener::bind(socket_addr)?;
                    std_listener.set_nonblocking(true)?;
                    Ok(mio::net::TcpListener::from_std(std_listener))
                };

                let listener_result = result();
                *listener_handle_clone.lock().unwrap() = Some(listener_result);
                waker.wake();
            };

            let pool = HoochPool::get();
            pool.execute(Box::new(connect_fn));
            return Poll::Pending;
        }

        if self.state.lock().unwrap().is_none() {
            return Poll::Pending;
        }

        let listener_result = self.state.lock().unwrap().take().unwrap();
        Poll::Ready(listener_result)
    }
}
