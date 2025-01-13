use std::{
    fmt::Debug,
    fs::{File, OpenOptions},
    future::Future,
    io::{self, Read},
    ops::{Deref, DerefMut},
    os::unix::fs::MetadataExt,
    path::Path,
    task::Poll,
};

use crate::fs::traits::OpenHooch;

#[derive(Debug)]
pub struct HoochFile {
    handle: File,
}

impl HoochFile {
    pub async fn open<P: AsRef<Path>>(path: P) -> Result<Self, std::io::Error> {
        let mut async_file = Box::pin(AsyncHoochFile { path });
        let file = std::future::poll_fn(|ctx| async_file.as_mut().poll(ctx)).await?;
        Ok(Self { handle: file })
    }

    pub async fn read_to_string(&mut self) -> String {
        let mut async_read = Box::pin(AsyncReadToString { file: &self.handle });
        std::future::poll_fn(|ctx| async_read.as_mut().poll(ctx))
            .await
            .unwrap()
    }
}

impl<P: AsRef<Path> + Debug> OpenHooch<P> for OpenOptions {
    fn open_hooch(&self, path: &P) -> impl Future<Output = Result<HoochFile, std::io::Error>> {
        std::future::poll_fn(move |_| {
            let result = self.open(path).map(|file| HoochFile { handle: file });
            Poll::Ready(result)
        })
    }
}

impl Deref for HoochFile {
    type Target = File;

    fn deref(&self) -> &Self::Target {
        &self.handle
    }
}

impl DerefMut for HoochFile {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.handle
    }
}

#[derive(Debug)]
struct AsyncHoochFile<P: AsRef<Path>> {
    path: P,
}

impl<P: AsRef<Path>> Future for AsyncHoochFile<P> {
    type Output = Result<File, io::Error>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> Poll<Self::Output> {
        Poll::Ready(File::open(self.path.as_ref()))
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
