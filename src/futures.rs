//! Backoff support for `async`/`await`.

use crate::{EaseOff, Error, ResultWrapper, TimeoutError};

use pin_project::pin_project;
use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::task::{ready, Context, Poll};
use std::time::Instant;

/// Backoff support for `async`/`await`.
///
/// ### Note: Behavior at Deadline
/// Unless otherwise stated, async operations are _not_ cancelled at the [deadline][Self::deadline()]
/// once they are in-progress.
///
/// More specifically, if the deadline elapses after an async operation has begun, i.e.
/// the future returned by these methods is `.await`ed or polled,
/// it will be allowed to run to completion.
///
/// To cancel an in-progress operation when the deadline elapses,
/// use [`TryAsync::enforce_deadline_with()`].
impl<E> EaseOff<E> {
    /// Attempt an async operation.
    ///
    /// The operation is immediately cancelled without being polled
    /// if the deadline has already elapsed. Otherwise, it is run to completion.
    ///
    /// See the note on this impl block for details.
    pub fn try_async<T, Fut>(&mut self, op: Fut) -> TryAsync<'_, E, impl FnOnce() -> Fut>
    where
        Fut: Future<Output = Result<T, E>>,
    {
        self.try_async_with(move || op)
    }

    /// Attempt the async operation returned by the given closure.
    ///
    /// This allows for some lazy computation that is not executed if the deadline
    /// has already elapsed.
    ///
    /// The closure is not called if the deadline has elapsed by the time the returned `Future`
    /// is polled. If the deadline elapses after the operation has begun, it is allowed
    /// to run to completion.
    ///
    /// See the note on this impl block for details.
    pub fn try_async_with<T, F, Fut>(&mut self, op: F) -> TryAsync<'_, E, F>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        TryAsync { ease_off: self, op }
    }
}

/// `.await`able type returned by [`EaseOff::try_async()`] and [`EaseOff::try_async_with()`].
///
/// ### Panics
/// If an async runtime is not available for sleeping between retries.
///
/// ### Note: Behavior at Deadline
/// Unless otherwise stated, async operations are _not_ cancelled at the [deadline][EaseOff::deadline()]
/// once they are in-progress.
///
/// More specifically, if the deadline elapses after an async operation has begun, i.e.
/// the future returned by these methods is `.await`ed or polled,
/// it will be allowed to run to completion.
///
/// To cancel an in-progress operation when the deadline elapses,
/// use [`Self::enforce_deadline_with()`].
#[must_use = "futures do nothing unless `.await`ed or polled"]
pub struct TryAsync<'a, E, F> {
    ease_off: &'a mut EaseOff<E>,
    op: F,
}

/// [`Future`] returned by [`TryAsync::into_future()`], [`TryAsync::enforce_deadline_with()`].
///
/// If the current state of the [`EaseOff`] prescribes a sleep before the next attempt,
/// the future will not be invoked immediately.
///
/// ### Panics
/// If an async runtime is not available for sleeping between retries.
#[pin_project]
pub struct TryAsyncFuture<'a, E, F, Fut> {
    // Wrapped in `Option` so we can take and subsequently return ownership in `poll()`
    ease_off: Option<&'a mut EaseOff<E>>,
    #[pin]
    op: LazyOp<F, Fut>,
    #[pin]
    sleep: Sleep,
}

#[pin_project(project = LazyOpPinned)]
enum LazyOp<F, Fut> {
    NotStarted(Option<F>),
    Started(#[pin] Fut),
}

// Tried to make this work with `pin-project-lite`, but couldn't
#[pin_project(project = SleepPinned)]
enum Sleep {
    Unset,
    Skipped,
    #[cfg(feature = "tokio")]
    Tokio(#[pin] tokio::time::Sleep),
}

impl<'a, T, E, F, Fut> IntoFuture for TryAsync<'a, E, F>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    type Output = ResultWrapper<'a, T, E>;
    type IntoFuture = TryAsyncFuture<'a, E, F, Fut>;

    fn into_future(self) -> Self::IntoFuture {
        TryAsyncFuture {
            ease_off: Some(self.ease_off),
            sleep: Sleep::Unset,
            op: LazyOp::NotStarted(Some(self.op)),
        }
    }
}

impl<'a, T, E, F, Fut> TryAsync<'a, E, F>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    // TODO: design an API that automatically creates an `E` for convenience/reusability
    /// Cancel the operation as soon as the [deadline][EaseOff::deadline()] elapses, if set.
    ///
    /// The closure will be called to produce the error that will be returned;
    /// if the operation failed on a previous attempt, that error is included.
    ///
    /// ### Panics
    /// If an async runtime is not available for managing the timeout.
    ///
    /// ### Example
    ///
    /// ```rust
    /// # #[tokio::main(flavor = "current_thread")]
    /// # async fn main() {
    /// use std::time::Duration;
    /// use ease_off::EaseOff;
    ///
    /// let mut ease_off = EaseOff::start_timeout(Duration::from_secs(5));
    ///
    /// let result = ease_off
    ///     // An async operation that will never complete.
    ///     .try_async(std::future::pending::<Result<String, String>>())
    ///     // You may either use the last error (`_e`) or create a new one
    ///     .enforce_deadline_with(|_e: Option<String>| "deadline elapsed".to_string())
    ///     .await
    ///     .or_retry_if(|_e| false);
    ///
    /// assert_eq!(result.unwrap_err(), "deadline elapsed");
    /// # }
    pub async fn enforce_deadline_with(
        self,
        make_error: impl FnOnce(Option<E>) -> E,
    ) -> ResultWrapper<'a, T, E> {
        if let Some(deadline) = self.ease_off.deadline {
            let res = tokio::time::timeout_at(deadline.into(), (self.op)())
                .await
                .map_or_else(
                    |_| {
                        Err(Error::TimedOut(TimeoutError {
                            last_error: make_error(self.ease_off.last_error.take()),
                        }))
                    },
                    |res| res.map_err(Error::MaybeRetryable),
                );

            self.ease_off.wrap_result(res)
        } else {
            self.await
        }
    }
}

impl<'a, T, E, F, Fut> Future for TryAsyncFuture<'a, E, F, Fut>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    type Output = ResultWrapper<'a, T, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        if this.sleep.is_unset() {
            let ease_off = this
                .ease_off
                .as_deref_mut()
                .expect("BUG: this.ease_off already taken");

            match ease_off.next_retry_at() {
                Ok(Some(retry_at)) => {
                    this.sleep.set(Sleep::until(retry_at));
                }
                Ok(None) => {
                    this.sleep.set(Sleep::Skipped);
                }
                Err(e) => {
                    return Poll::Ready(
                        this.ease_off
                            .take()
                            .expect("BUG: this.ease_off already taken")
                            .wrap_result(Err(e)),
                    );
                }
            }
        }

        ready!(this.sleep.as_mut().poll(cx));

        let res = ready!(this.op.poll(cx)).map_err(Error::MaybeRetryable);

        Poll::Ready(
            this.ease_off
                .take()
                .expect("BUG: this.ease_off already taken")
                .wrap_result(res),
        )
    }
}

impl<T, E, F, Fut> Future for LazyOp<F, Fut>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    type Output = Result<T, E>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            match self.as_mut().project() {
                LazyOpPinned::NotStarted(op) => {
                    let op = op.take().expect("`op` already taken");
                    self.set(LazyOp::Started(op()));
                },
                LazyOpPinned::Started(fut) => {
                    return fut.poll(cx);
                }
            }
        }
    }
}

impl Sleep {
    fn until(instant: Instant) -> Self {
        #[cfg(feature = "tokio")]
        if tokio::runtime::Handle::try_current().is_ok() {
            return Self::Tokio(tokio::time::sleep_until(instant.into()));
        }

        panic!("no async runtime enabled")
    }

    fn is_unset(&self) -> bool {
        matches!(self, Sleep::Unset)
    }
}

impl Future for Sleep {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.project() {
            SleepPinned::Unset | SleepPinned::Skipped => Poll::Ready(()),
            SleepPinned::Tokio(sleep) => sleep.poll(cx),
        }
    }
}
