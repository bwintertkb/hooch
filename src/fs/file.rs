use std::{
    fs::File,
    future::Future,
    io::{self, Read},
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::{Path, PathBuf},
    task::Poll,
    time::Duration,
};

use mio::Interest;

use crate::{reactor::Reactor, task::Task};

#[derive(Debug)]
pub struct HoochFile {
    handle: File,
}

impl HoochFile {
    pub fn try_new<P: AsRef<Path>>(path: P) -> Result<Self, std::io::Error> {
        let file = File::open(path)?;
        Ok(Self { handle: file })
    }

    pub async fn read_to_string(&mut self) -> String {
        let mut async_read = Box::pin(AsyncReadToString { file: &self.handle });
        std::future::poll_fn(|ctx| async_read.as_mut().poll(ctx))
            .await
            .unwrap()
    }
}

#[derive(Debug)]
struct AsyncReadToString<'a> {
    file: &'a File,
}

impl<'a> Future for AsyncReadToString<'a> {
    type Output = Result<String, io::Error>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let file_size = self.file.metadata().unwrap().size();
        let mut buffer = String::with_capacity(file_size as usize);
        self.file.read_to_string(&mut buffer).unwrap();
        Poll::Ready(Ok(buffer))
    }
}
