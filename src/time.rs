use std::{
    future::Future,
    os::fd::{AsFd, AsRawFd},
    panic::UnwindSafe,
    pin::Pin,
    task::Poll,
    time::Duration,
};

use mio::{event::Source, unix::SourceFd, Interest, Token};
use nix::sys::timerfd::{ClockId, Expiration, TimerFd, TimerFlags, TimerSetTimeFlags};

use crate::reactor::Reactor;

/// Sleep for a `Duration`
pub async fn sleep(duration: Duration) {
    let mut sleeper = Sleep::from_duration(duration);
    std::future::poll_fn(|cx| sleeper.as_mut().poll(cx)).await;
}

pub struct Sleep {
    timer: TimerFd,
    has_polled: bool,
    mio_token: Token,
}

impl UnwindSafe for Sleep {}

impl Sleep {
    fn from_duration(duration: Duration) -> Pin<Box<Self>> {
        let timer = TimerFd::new(ClockId::CLOCK_MONOTONIC, TimerFlags::TFD_NONBLOCK).unwrap();

        let spec = nix::sys::time::TimeSpec::from_duration(duration);
        timer
            .set(Expiration::OneShot(spec), TimerSetTimeFlags::empty())
            .unwrap();

        let reactor = Reactor::get();
        let token = reactor.unique_token();
        let mut sleep = Sleep {
            timer,
            has_polled: false,
            mio_token: token,
        };

        reactor
            .registry()
            .register(&mut sleep, token, Interest::READABLE | Interest::WRITABLE)
            .unwrap();

        Box::pin(sleep)
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
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        if !self.has_polled {
            let reactor = Reactor::get();
            reactor.status_store(self.mio_token, cx.waker().clone());
            self.has_polled = true;
            return Poll::Pending;
        }

        Poll::Ready(())
    }
}

#[cfg(test)]
pub mod tests {
    use std::time::Instant;

    use crate::runtime::RuntimeBuilder;

    use super::*;

    #[test]
    fn test_sleep() {
        let handle = RuntimeBuilder::default().build();
        let sleep_milliseconds = 100;

        let res = handle.run_blocking(async move {
            let instant = Instant::now();
            sleep(Duration::from_millis(sleep_milliseconds)).await;
            instant.elapsed().as_millis()
        });

        assert!(res >= (sleep_milliseconds as u128));
    }
}
