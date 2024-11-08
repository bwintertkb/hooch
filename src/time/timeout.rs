//! A module providing a timeout mechanism for futures.
//!
//! This module defines a function `timeout` that attempts to resolve a given future within a specified duration.
//! If the future does not complete within the timeout, it returns a `TimeoutError`.
//! The `Timeout` struct is used internally to manage the polling of both the future and the timeout.

use std::{
    error::Error,
    fmt::Display,
    future::{poll_fn, Future},
    pin::Pin,
    task::Poll,
    time::Duration,
};

use crate::time::sleep;

/// Attempts to resolve the given future within a specified timeout duration.
///
/// If the future does not complete within the given timeout, a `TimeoutError` is returned.
/// Otherwise, returns the output of the given future.
///
/// # Parameters
///
/// * `fut` - The future that needs to be resolved.
/// * `timeout` - The duration within which the future must complete.
///
/// # Returns
///
/// A `Result` containing either the output of the future (`Ok`) or a `TimeoutError` (`Err`) if the timeout expires.
///
/// # Example
///
/// ```
/// # use std::time::Duration;
/// # use hooch::time::sleep;
/// # use hooch::time::timeout;
/// async fn my_async_function() {
///     sleep(Duration::from_secs(1)).await;
/// }
///
/// async fn example() {
///     let result = timeout(my_async_function(), Duration::from_secs(2)).await;
///     match result {
///         Ok(_) => println!("Future completed within the timeout"),
///         Err(_) => println!("Future timed out"),
///     }
/// }
/// ```
pub async fn timeout<Fut, T>(fut: Fut, timeout: Duration) -> Result<T, TimeoutError>
where
    T: Send + 'static,
    Fut: Future<Output = T> + Send + 'static,
{
    let fut = Box::pin(fut);
    let sleep = Box::pin(sleep(timeout));

    let mut timeout = Box::pin(Timeout { fut, sleep });

    poll_fn(|cx| timeout.as_mut().poll(cx)).await
}

/// Represents a future that will complete with a value or an error if it times out.
#[derive(Debug)]
pub struct Timeout<T, Fut, Sleep>
where
    T: Send + 'static,
    Fut: Future<Output = T> + Send + 'static,
    Sleep: Future<Output = ()> + Send + 'static,
{
    fut: Pin<Box<Fut>>,
    sleep: Pin<Box<Sleep>>,
}

impl<T, Fut, Sleep> Future for Timeout<T, Fut, Sleep>
where
    T: Send + 'static,
    Fut: Future<Output = T> + Send + 'static,
    Sleep: Future<Output = ()> + Send + 'static,
{
    type Output = Result<T, TimeoutError>;

    /// Polls the future and the timeout.
    ///
    /// If the sleep future completes first, it returns a `TimeoutError`.
    /// If the main future completes first, it returns the result of the future.
    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        if self.sleep.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(TimeoutError));
        }

        if let Poll::Ready(value) = self.fut.as_mut().poll(cx) {
            return Poll::Ready(Ok(value));
        }

        Poll::Pending
    }
}

/// Represents an error that occurs when a future times out.
#[derive(Debug)]
pub struct TimeoutError;

impl Display for TimeoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TimeoutError")
    }
}

impl Error for TimeoutError {}

#[cfg(test)]
pub mod tests {
    use crate::runtime::RuntimeBuilder;

    use super::*;

    async fn timeout_long_function() {
        sleep(Duration::from_secs(3)).await;
    }

    async fn timeout_short_function() {
        sleep(Duration::from_micros(500)).await;
    }

    #[test]
    fn test_timeout_function_timeout_error() {
        let handle = RuntimeBuilder::default().build();
        let res = handle.run_blocking(async {
            timeout(timeout_long_function(), Duration::from_millis(200)).await
        });
        assert!(res.is_err())
    }

    #[test]
    fn test_timeout_function_ok_result() {
        let handle = RuntimeBuilder::default().build();
        let res = handle.run_blocking(async {
            timeout(timeout_short_function(), Duration::from_secs(20)).await
        });
        assert!(res.is_ok())
    }
}
