// use std::{cell::UnsafeCell, future::Future, mem, pin::Pin, task::Poll};
//
// /// Select naive is iterating through the futures in order in which they exist in the vector
// pub async fn select_naive<T, F, Fut>(futs: Vec<(Fut, Option<F>)>)
// where
//     T: Send,
//     F: Fn(&T),
//     Fut: Future<Output = T> + Send,
// {
//     let capacity = futs.capacity();
//     let select = SelectNaive {
//         futs: futs
//             .into_iter()
//             .map(|(fut, callback)| Futs {
//                 fut: UnsafeCell::new(Box::pin(fut)),
//                 fn_: callback,
//             })
//             .collect(),
//     };
// }
//
// struct Futs<T, F, Fut>
// where
//     T: Send,
//     F: Fn(&T),
//     Fut: Future<Output = T> + Send,
// {
//     fut: UnsafeCell<Pin<Box<Fut>>>,
//     fn_: Option<F>,
// }
//
// // #[derive(Debug)]
// struct SelectNaive<Fut, F, T>
// where
//     T: Send,
//     F: Fn(&T),
//     Fut: Future<Output = T> + Send,
// {
//     futs: Vec<Futs<T, F, Fut>>,
// }
//
// impl<Fut, F, T> Future for SelectNaive<Fut, F, T>
// where
//     T: Send,
//     F: Fn(T),
//     Fut: Future<Output = T> + Send,
// {
//     type Output = ();
//
//     fn poll(
//         mut self: std::pin::Pin<&mut Self>,
//         cx: &mut std::task::Context<'_>,
//     ) -> std::task::Poll<Self::Output> {
//         for fut in self.futs.iter() {
//             let original_future = unsafe { &mut *fut.fut.get() };
//
//             let p_fut = async { std::future::pending::<T>().await };
//             let mut placeholder_fut = Box::pin(p_fut);
//             let fut_to_poll = mem::replace(original_future, placeholder_fut);
//
//             let poll_result = fut_to_poll.as_mut().poll(cx);
//
//             *original_future = fut_to_poll;
//
//             let Poll::Ready(result) = poll_result else {
//                 continue;
//             };
//
//             if let Some(callback) = &fut.fn_ {
//                 callback(result);
//             }
//
//             return Poll::Ready(());
//         }
//         Poll::Pending
//     }
// }
