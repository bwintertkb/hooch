use std::{fmt::Debug, future::Future, path::Path};

use crate::fs::file::HoochFile;

pub trait OpenHooch<P>
where
    P: AsRef<Path> + Debug,
{
    fn open_hooch(&self, path: &P) -> impl Future<Output = Result<HoochFile, std::io::Error>>;
}
