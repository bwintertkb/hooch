use std::{future::Future, path::Path};

use crate::fs::file::HoochFile;

pub trait OpenHooch {
    fn open_hooch(self, path: &Path) -> impl Future<Output = Result<HoochFile, std::io::Error>>;
}
