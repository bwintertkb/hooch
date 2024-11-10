use std::{future::Future, pin::Pin, task::Poll};

/// Select naive is iterating through the futures in order in which they exist in the vector
pub async fn select_naive<T, F, Fut>(futs: Vec<(Fut, Option<F>)>)
where
    T: Send,
    F: Fn(T),
    Fut: Future<Output = T> + Send,
{
    let capacity = futs.capacity();
    let select = SelectNaive {
        futs: futs
            .into_iter()
            .map(|(fut, callback)| (Box::pin(fut), callback))
            .collect(),
        incomple_futs: Vec::with_capacity(capacity),
    };
}

#[derive(Debug)]
struct SelectNaive<Fut, F, T>
where
    T: Send,
    F: Fn(T),
    Fut: Future<Output = T> + Send,
{
    futs: Vec<(Pin<Box<Fut>>, Option<F>)>,
    incomple_futs: Vec<(Pin<Box<Fut>>, Option<F>)>,
}

impl<Fut, F, T> Future for SelectNaive<Fut, F, T>
where
    T: Send,
    F: Fn(T),
    Fut: Future<Output = T> + Send,
{
    type Output = ();

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        for (ref mut fut, callback) in self.futs.iter_mut() {
            let Poll::Ready(value) = fut.as_mut().poll(cx) else {
                continue;
            };

            if let Some(callback) = callback {
                callback(value);
            }

            return Poll::Ready(());
        }
        Poll::Pending
    }
}
