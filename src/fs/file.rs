use std::{
    fs::File,
    os::fd::AsRawFd,
    path::{Path, PathBuf},
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
        let file = File::create(path)?;
        Ok(Self { handle: file })
    }

    pub fn read_to_string(&mut self) {
        let registry = Reactor::get();

        let _ = registry.registry().register(
            &mut mio::unix::SourceFd(&self.handle.as_raw_fd()),
            registry.unique_token(),
            Interest::READABLE | Interest::WRITABLE,
        );

        std::thread::sleep(Duration::from_secs(1));
    }
}
