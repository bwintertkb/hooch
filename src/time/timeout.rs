use std::{
    error::Error,
    fmt::Display,
    future::{poll_fn, Future},
    pin::Pin,
    task::Poll,
    time::Duration,
};

use crate::time::sleep;

/// TODO add timeout comment
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
