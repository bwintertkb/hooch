use std::{
    future::Future,
    os::fd::{AsFd, AsRawFd},
    time::{Duration, Instant},
};

use mio::{event::Source, unix::SourceFd};
use nix::sys::timerfd::{ClockId, Expiration, TimerFd, TimerFlags, TimerSetTimeFlags};

use crate::reactor::{Reactor, ReactorTag};

pub async fn sleep(duration: Duration) {
    let mut sleeper = Sleep::from_duration(duration);
    // Need to add a register tag and register the sleep to mio for wakertoken 0
    let reactor = Reactor::get();
    // Reactor::get().registry().register(&mut sleeper, , interests);
}

pub struct Sleep {
    timer: TimerFd,
    reactor_tag: ReactorTag,
}

impl Sleep {
    fn from_duration(duration: Duration) -> Self {
        let timer = TimerFd::new(ClockId::CLOCK_MONOTONIC, TimerFlags::TFD_NONBLOCK).unwrap();

        let spec = nix::sys::time::TimeSpec::from_duration(duration);
        timer
            .set(Expiration::OneShot(spec), TimerSetTimeFlags::empty())
            .unwrap();
        Sleep {
            timer,
            reactor_tag: Reactor::generate_reactor_tag(),
        }
    }
}

impl AsRawFd for Sleep {
    fn as_raw_fd(&self) -> std::os::unix::prelude::RawFd {
        self.timer.as_fd().as_raw_fd()
    }
}

impl Source for Sleep {
    fn register(
        &mut self,
        registry: &mio::Registry,
        token: mio::Token,
        interests: mio::Interest,
    ) -> std::io::Result<()> {
        SourceFd(&self.as_raw_fd()).register(registry, token, interests)
    }

    fn reregister(
        &mut self,
        registry: &mio::Registry,
        token: mio::Token,
        interests: mio::Interest,
    ) -> std::io::Result<()> {
        SourceFd(&self.as_raw_fd()).reregister(registry, token, interests)
    }

    fn deregister(&mut self, registry: &mio::Registry) -> std::io::Result<()> {
        SourceFd(&self.as_raw_fd()).deregister(registry)
    }
}

impl Future for Sleep {
    type Output = ();

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn test_sleep() {
        sleep(Duration::from_millis(200));
    }
}
