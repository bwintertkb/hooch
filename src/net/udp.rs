use std::{
    io::ErrorKind,
    net::{SocketAddr, ToSocketAddrs},
};

use mio::{Interest, Token};

use crate::reactor::Reactor;

pub struct UdpSocket {
    socket: mio::net::UdpSocket,
    token: Token,
}

impl UdpSocket {
    pub fn bind(addr: impl ToSocketAddrs) -> std::io::Result<Self> {
        let std_socket = std::net::UdpSocket::bind(addr)?;
        std_socket.set_nonblocking(true)?;

        let mut socket = mio::net::UdpSocket::from_std(std_socket);

        let reactor = Reactor::get();
        let token = reactor.unique_token();

        Reactor::get().registry().register(
            &mut socket,
            token,
            Interest::READABLE | Interest::WRITABLE,
        )?;

        Ok(self::UdpSocket { socket, token })
    }

    pub async fn send_to(&self, buf: &[u8], dest: SocketAddr) -> std::io::Result<usize> {
        loop {
            match self.socket.send_to(buf, dest) {
                Ok(value) => return Ok(value),
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    std::future::poll_fn(|cx| Reactor::get().poll(self.token, cx)).await?
                }
                Err(error) => return Err(error),
            }
        }
    }

    pub async fn recv_from(&self, buf: &mut [u8]) -> std::io::Result<(usize, SocketAddr)> {
        loop {
            match self.socket.recv_from(buf) {
                Ok(value) => return Ok(value),
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    std::future::poll_fn(|cx| Reactor::get().poll(self.token, cx)).await?
                }
                Err(error) => return Err(error),
            }
        }
    }
}

impl Drop for UdpSocket {
    fn drop(&mut self) {
        let _ = Reactor::get().registry().deregister(&mut self.socket);
    }
}
