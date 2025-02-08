//! A module providing a sleep mechanism for asynchronous tasks.
//!
//! This module defines the `sleep` function that allows an async task to pause for a specified duration.
//! It uses a `TimerFd` with the `Reactor` to achieve non-blocking sleep functionality.
//! The `Sleep` struct is used internally to manage the timer and interaction with the reactor.

use std::{
    future::Future,
    os::fd::{AsFd, AsRawFd},
    panic::UnwindSafe,
    pin::Pin,
    task::Poll,
    time::Duration,
};

use mio::{unix::SourceFd, Interest, Token};
use nix::sys::timerfd::{ClockId, Expiration, TimerFd, TimerFlags, TimerSetTimeFlags};

use crate::reactor::Reactor;

/// Sleep for a `Duration`.
///
/// This function returns a future that completes after the specified duration has elapsed.
/// It uses a `TimerFd` to achieve non-blocking sleep and interacts with the reactor to register the timer.
///
/// # Parameters
///
/// * `duration` - The duration to sleep for.
///
/// # Example
///
/// ```
/// # use std::time::Duration;
/// # use hooch::time::sleep;
/// async fn example() {
///     sleep(Duration::from_secs(2)).await;
///     println!("Slept for 2 seconds");
/// }
/// ```
pub async fn sleep(duration: Duration) {
    let mut sleeper = Sleep::from_duration(duration);
    std::future::poll_fn(|cx| sleeper.as_mut().poll(cx)).await;
}

/// Represents a non-blocking sleep future using a `TimerFd`.
pub struct Sleep {
    duration: Duration,
    has_polled: bool,
    mio_token: Token,
    timer: Option<TimerFd>,
}

impl UnwindSafe for Sleep {}

impl Sleep {
    /// Creates a new `Sleep` future for the given duration.
    ///
    /// The function initializes a `TimerFd` and registers it with the reactor to be notified once the timer expires.
    fn from_duration(duration: Duration) -> Pin<Box<Self>> {
        let reactor = Reactor::get();
        let token = reactor.unique_token();
        let sleep = Sleep {
            duration,
            has_polled: false,
            mio_token: token,
            timer: None,
        };

        Box::pin(sleep)
    }
}

impl Future for Sleep {
    type Output = ();

    /// Polls the `Sleep` future to determine if the sleep duration has elapsed.
    ///
    /// The function interacts with the reactor to check if the timer has expired.
    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let reactor = Reactor::get();
        if !self.has_polled {
            reactor.status_store(self.mio_token, cx.waker().clone());
            // Register a timer
            let timer = TimerFd::new(ClockId::CLOCK_MONOTONIC, TimerFlags::TFD_NONBLOCK).unwrap();

            let spec = nix::sys::time::TimeSpec::from_duration(self.duration);
            timer
                .set(Expiration::OneShot(spec), TimerSetTimeFlags::empty())
                .unwrap();
            reactor
                .registry()
                .register(
                    &mut SourceFd(&timer.as_fd().as_raw_fd()),
                    self.mio_token,
                    Interest::READABLE | Interest::WRITABLE,
                )
                .unwrap();

            // We need to register the timer, dropping it stops it from being polled by mio
            self.timer = Some(timer);

            self.has_polled = true;
            return Poll::Pending;
        }

        if reactor.has_token_progressed(self.mio_token) {
            return Poll::Ready(());
        }

        Poll::Pending
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
        let sleep_milliseconds = 400;

        let res = handle.run_blocking(async move {
            let instant = Instant::now();
            sleep(Duration::from_millis(sleep_milliseconds)).await;
            instant.elapsed().as_millis()
        });

        assert!(res >= (sleep_milliseconds as u128));
    }
}
