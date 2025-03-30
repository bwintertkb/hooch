//! This module provides asynchronous file operations for the Hooch runtime.
//! It supports opening, creating, and reading files asynchronously by offloading
//! blocking operations to a thread pool (`HoochPool`) and coordinating with a reactor (`Reactor`).
//!
//! The main type, [`HoochFile`], wraps a standard file handle and provides asynchronous
//! methods to open, create, and read files.

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
/// An asynchronous file wrapper that encapsulates a standard file handle.
///
/// `HoochFile` provides asynchronous operations for opening, creating,
/// and reading files.
pub struct HoochFile {
    handle: File,
}

impl HoochFile {
    /// Asynchronously opens a file at the specified path.
    ///
    /// # Arguments
    ///
    /// * `path` - A reference to the file path.
    ///
    /// # Returns
    ///
    /// A future that resolves to a [`HoochFile`] upon success, or an `io::Error` if the operation fails.
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

    /// Asynchronously creates a file at the specified path.
    ///
    /// # Arguments
    ///
    /// * `path` - A reference to the file path.
    ///
    /// # Returns
    ///
    /// A future that resolves to a [`HoochFile`] upon success, or an `io::Error` if the operation fails.
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

    /// Asynchronously reads the entire contents of the file into a `String`.
    ///
    /// # Returns
    ///
    /// A future that resolves to a `String` containing the file's contents.
    ///
    /// # Panics
    ///
    /// This implementation unwraps I/O errors during the read operation.
    pub async fn read_to_string(&mut self) -> String {
        let mut async_read = Box::pin(AsyncReadToString { file: &self.handle });
        std::future::poll_fn(|ctx| async_read.as_mut().poll(ctx))
            .await
            .unwrap()
    }
}

impl OpenHooch for OpenOptions {
    /// Opens a file with the given [`OpenOptions`] asynchronously.
    ///
    /// # Arguments
    ///
    /// * `path` - A reference to the file path.
    ///
    /// # Returns
    ///
    /// A future that resolves to a [`HoochFile`] upon success, or an `io::Error` if the operation fails.
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

    /// Dereferences the [`HoochFile`] to its underlying [`File`] handle.
    fn deref(&self) -> &Self::Target {
        &self.handle
    }
}

impl DerefMut for HoochFile {
    /// Mutably dereferences the [`HoochFile`] to its underlying [`File`] handle.
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.handle
    }
}

/// Enum representing the type of file operation to be performed.
#[derive(Debug, Clone)]
enum FileOperation {
    /// Create a new file.
    Create,
    /// Open an existing file.
    Open,
    /// Open a file with specific [`OpenOptions`].
    Option(OpenOptions),
}

/// Internal future used to perform asynchronous file operations.
///
/// This future offloads blocking file operations (open, create, or open with options)
/// to a thread pool, and then returns a [`HoochFile`] when the operation is complete.
#[derive(Debug)]
struct AsyncHoochFile {
    /// The file path for the operation.
    path: PathBuf,
    /// The file operation to be performed.
    file_operation: Option<FileOperation>,
    /// The reactor tag associated with this operation.
    reactor_tag: ReactorTag,
    /// Shared state that will hold the result of the file operation.
    file_handle: Arc<Mutex<Option<Result<HoochFile, io::Error>>>>,
    /// Flag indicating whether the operation has been initiated.
    has_polled: bool,
}

impl Future for AsyncHoochFile {
    type Output = Result<HoochFile, io::Error>;

    /// Polls the future to perform the asynchronous file operation.
    ///
    /// On the first poll, the blocking file operation is offloaded to a thread pool.
    /// Subsequent polls check if the result is available.
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

            // Take the file operation (open, create, or with options) and offload it.
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

/// Internal future used to asynchronously read the entire contents of a file into a string.
///
/// This future reads the file synchronously when polled. In a real-world scenario,
/// this might be further refactored to handle large files more gracefully.
#[derive(Debug)]
struct AsyncReadToString<'a> {
    /// A reference to the file to be read.
    file: &'a File,
}

impl Future for AsyncReadToString<'_> {
    type Output = Result<String, io::Error>;

    /// Polls the future to read the file's contents.
    ///
    /// This implementation reads the entire file into a string and returns it immediately.
    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let file_size = self.file.metadata().unwrap().size();
        let mut buffer = String::with_capacity(file_size as usize);
        self.file.read_to_string(&mut buffer).unwrap();
        std::task::Poll::Ready(Ok(buffer))
    }
}
