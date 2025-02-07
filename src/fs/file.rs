use std::{
    fmt::Debug,
    fs::{File, OpenOptions},
    future::Future,
    io::{self, Read},
    ops::{Deref, DerefMut},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    task::Poll,
};

use crate::{
    fs::traits::OpenHooch,
    pool::thread_pool::HoochPool,
    reactor::{Reactor, ReactorTag},
};

#[derive(Debug)]
pub struct HoochFile {
    handle: File,
}

// TODO, refactor the reactor tag and async hooch file out
impl HoochFile {
    pub async fn open<P: AsRef<Path>>(path: P) -> Result<Self, std::io::Error> {
        let reactor_tag = Reactor::generate_reactor_tag();
        let reactor = Reactor::get();
        reactor.register_reactor_tag(reactor_tag);
        let file_handle: Arc<Mutex<Option<Result<HoochFile, io::Error>>>> =
            Arc::new(Mutex::default());

        let mut async_hooch_file = Box::pin(AsyncHoochFile {
            path: path.as_ref().to_path_buf(),
            file_operation: Some(FileOperation::Open),
            file_handle,
            reactor_tag,
            has_polled: false,
        });

        std::future::poll_fn(move |cx| async_hooch_file.as_mut().poll(cx)).await
    }

    pub async fn create<P: AsRef<Path>>(path: P) -> Result<Self, std::io::Error> {
        let reactor_tag = Reactor::generate_reactor_tag();
        let reactor = Reactor::get();
        reactor.register_reactor_tag(reactor_tag);
        let file_handle: Arc<Mutex<Option<Result<HoochFile, io::Error>>>> =
            Arc::new(Mutex::default());

        let mut async_hooch_file = Box::pin(AsyncHoochFile {
            path: path.as_ref().to_path_buf(),
            file_operation: Some(FileOperation::Create),
            file_handle,
            reactor_tag,
            has_polled: false,
        });

        std::future::poll_fn(move |cx| async_hooch_file.as_mut().poll(cx)).await
    }

    pub async fn read_to_string(&mut self) -> String {
        let mut async_read = Box::pin(AsyncReadToString { file: &self.handle });
        std::future::poll_fn(|ctx| async_read.as_mut().poll(ctx))
            .await
            .unwrap()
    }
}

impl OpenHooch for OpenOptions {
    fn open_hooch(self, path: &Path) -> impl Future<Output = Result<HoochFile, std::io::Error>> {
        let reactor_tag = Reactor::generate_reactor_tag();
        let reactor = Reactor::get();
        reactor.register_reactor_tag(reactor_tag);
        let file_handle: Arc<Mutex<Option<Result<HoochFile, io::Error>>>> =
            Arc::new(Mutex::default());

        let mut async_hooch_file = Box::pin(AsyncHoochFile {
            path: path.to_path_buf(),
            file_operation: Some(FileOperation::Option(self)),
            file_handle,
            reactor_tag,
            has_polled: false,
        });

        std::future::poll_fn(move |cx| async_hooch_file.as_mut().poll(cx))
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

#[derive(Debug, Clone)]
enum FileOperation {
    Create,
    Open,
    Option(OpenOptions),
}

#[derive(Debug)]
struct AsyncHoochFile {
    path: PathBuf,
    file_operation: Option<FileOperation>,
    reactor_tag: ReactorTag,
    file_handle: Arc<Mutex<Option<Result<HoochFile, io::Error>>>>,
    has_polled: bool,
}

impl Future for AsyncHoochFile {
    type Output = Result<HoochFile, io::Error>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        ctx: &mut std::task::Context<'_>,
    ) -> Poll<Self::Output> {
        if !self.has_polled {
            self.has_polled = true;
            let reactor = Reactor::get();
            reactor.register_reactor_tag(self.reactor_tag);
            reactor.store_waker_channel(self.reactor_tag, ctx.waker().clone());
            let path = self.path.clone();
            let pool = HoochPool::get();
            let file_handle_clone = Arc::clone(&self.file_handle);

            let file_operation = self.file_operation.take().unwrap();
            let block_fn = move || {
                let file_handle_result = match file_operation {
                    FileOperation::Create => File::create(path),
                    FileOperation::Open => File::open(path),
                    FileOperation::Option(options) => options.open(path),
                };

                let file_handle = file_handle_result.map(|f| HoochFile { handle: f });
                *file_handle_clone.lock().unwrap() = Some(file_handle);
                let reactor = Reactor::get();
                reactor.mio_waker().wake().unwrap();
            };
            pool.execute(Box::new(block_fn));
            return Poll::Pending;
        }

        if self.file_handle.lock().unwrap().is_none() {
            return Poll::Pending;
        }

        let file_result = self.file_handle.lock().unwrap().take().unwrap();
        Poll::Ready(file_result)
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
