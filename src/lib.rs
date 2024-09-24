//! An easy, opinionated exponential backoff implementation.
//!
//! Benefits over other implementations:
//! * More flexible control flow (you implement the loop yourself).
//! * Opinionated but sane defaults
//!     * Explicitly choose deadline, timeout or unlimited,
//!       so you know exactly what's going to happen.
//! * [`RetryableError`] trait allows for more reusable code.
//! * Immutable [`core`] API for when stateful backoffs aren't appropriate.
//!
//! # Examples
//!
//! ## Blocking Operation
//!
//! (Source: `examples/blocking.rs`)
#![doc = "```rust"]
#![doc = include_str!("../examples/blocking.rs")]
// If this were written using `//!`, RustRover would think this is the start of a new code block.
#![doc = "```"]
//!
//! ## Async Operation (Tokio)
//!
//! (Source: `examples/tokio.rs`)
#![doc = "```rust"]
#![cfg_attr(feature = "tokio", doc = include_str!("../examples/tokio.rs"))]
#![cfg_attr(
    not(feature = "tokio"),
    doc = "// `tokio` feature required to compile this example\nfn main() {}"
)]
// If this were written using `//!`, RustRover would think this is the start of a new code block.
#![doc = "```"]

use crate::core::EaseOffCore;
use std::num::Saturating;
use std::time::{Duration, Instant};

pub mod core;

mod options;

pub use options::Options;

/// Exponential backoff controller.
///
/// The constructors of this type use [`Options::DEFAULT`].
#[derive(Debug)]
pub struct EaseOff<E> {
    core: EaseOffCore,
    started_at: Instant,
    deadline: Option<Instant>,
    num_attempts: Saturating<u32>,
    last_error: Option<E>,
}

impl<E> EaseOff<E> {
    /// Alias for [`Options::start_unlimited()`] using [`Options::DEFAULT`].
    #[inline(always)]
    pub fn start_unlimited() -> Self {
        Options::DEFAULT.start_unlimited()
    }

    /// Alias for [`Options::start_timeout()`] using [`Options::DEFAULT`].
    #[inline(always)]
    pub fn start_timeout(timeout: Duration) -> Self {
        Options::DEFAULT.start_timeout(timeout)
    }

    /// Alias for [`Options::start_timeout_opt()`] using [`Options::DEFAULT`].
    #[inline(always)]
    pub fn start_timeout_opt(timeout: Option<Duration>) -> Self {
        Options::DEFAULT.start_timeout_opt(timeout)
    }

    /// Alias for [`Options::start_deadline()`] using [`Options::DEFAULT`].
    #[inline(always)]
    pub fn start_deadline(deadline: Instant) -> Self {
        Options::DEFAULT.start_deadline(deadline)
    }

    /// Alias for [`Options::start_deadline_opt()`] using [`Options::DEFAULT`].
    #[inline(always)]
    pub fn start_deadline_opt(deadline: Option<Instant>) -> Self {
        Options::DEFAULT.start_deadline_opt(deadline)
    }

    /// Returns the [`Instant`] when this instance was constructed.
    #[inline(always)]
    pub fn started_at(&self) -> Instant {
        self.started_at
    }

    /// Returns the deadline, if provided.
    ///
    /// If constructed with a timeout, it is converted to a deadline on construction
    /// by adding the timeout to [`Self::started_at()`].
    #[inline(always)]
    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    /// Returns the number of attempts that have been made.
    ///
    /// Saturates at [`u32::MAX`].
    #[inline(always)]
    pub fn num_attempts(&self) -> u32 {
        self.num_attempts.0
    }

    fn next_sleep_until(&mut self) -> Result<Option<Instant>, Error<E>> {
        let now = Instant::now();

        let mut rng = rand::thread_rng();

        if self.last_error.is_none() {
            self.num_attempts = Saturating(0);
            return Ok(self
                .core
                .nth_retry_at(0, now, None, &mut rng)
                .expect("passed `None` for deadline, should not be `Err`"));
        }

        let attempt_num = self.num_attempts.0;
        // `num_attempts` is `Saturating<u32>` so we don't have to worry about overflow.
        self.num_attempts += 1;

        self.core
            .nth_retry_at(attempt_num, now, self.deadline, &mut rng)
            .map_err(|_e| {
                Error::TimedOut(TimeoutError {
                    last_error: self
                        .last_error
                        .take()
                        .expect("BUG: `last_error` should not be `None` here"),
                })
            })
    }
}

macro_rules! try_sleep(
    ($this:ident, $instant:ident => $sleep:expr) => {
        match $this.next_sleep_until() {
            Ok(Some($instant)) => {
                $sleep
            }
            Ok(None) => (),
            Err(e) => {
                return ResultWrapper {
                    result: Err(e),
                    last_error: &mut $this.last_error
                }
            }
        }
    }
);

impl<E> EaseOff<E> {
    /// Attempt a blocking operation.
    pub fn try_blocking<T>(
        &mut self,
        op: impl FnOnce() -> Result<T, E>,
    ) -> ResultWrapper<'_, T, E> {
        try_sleep!(self, time => blocking_sleep_until(time));

        ResultWrapper {
            result: op().map_err(Error::MaybeRetryable),
            last_error: &mut self.last_error,
        }
    }

    #[cfg(feature = "tokio")]
    #[cfg_attr(docsrs, doc(cfg(feature = "tokio")))]
    pub async fn try_async<T>(
        &mut self,
        op: impl std::future::Future<Output = Result<T, E>>,
    ) -> ResultWrapper<'_, T, E> {
        self.try_async_with(move || op).await
    }

    #[cfg(feature = "tokio")]
    #[cfg_attr(docsrs, doc(cfg(feature = "tokio")))]
    pub async fn try_async_with<T, F, Fut>(&mut self, op: F) -> ResultWrapper<'_, T, E>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, E>>,
    {
        try_sleep!(self, time => tokio::time::sleep_until(time.into()).await);

        ResultWrapper {
            result: op().await.map_err(Error::MaybeRetryable),
            last_error: &mut self.last_error,
        }
    }
}

pub struct ResultWrapper<'a, T, E: 'a> {
    result: Result<T, Error<E>>,
    last_error: &'a mut Option<E>,
}

impl<'a, T, E: 'a> ResultWrapper<'a, T, E> {
    pub fn on_timeout(
        self,
        on_timeout: impl FnOnce(TimeoutError<E>) -> Error<E>,
    ) -> ResultWrapper<'a, T, E> {
        Self {
            result: self.result.map_err(|e| e.on_timeout(on_timeout)),
            last_error: self.last_error,
        }
    }

    pub fn inspect_err(self, inspect_err: impl FnOnce(&Error<E>)) -> Self {
        Self {
            result: self.result.inspect_err(inspect_err),
            last_error: self.last_error,
        }
    }

    pub fn or_retry(self) -> Result<Option<T>, E>
    where
        E: RetryableError,
    {
        self.or_retry_if(RetryableError::can_retry)
    }

    pub fn or_retry_if(self, can_retry: impl FnOnce(&Error<E>) -> bool) -> Result<Option<T>, E> {
        match self.result {
            Ok(success) => {
                *self.last_error = None;
                Ok(Some(success))
            }
            Err(e) => {
                if can_retry(&e) {
                    *self.last_error = Some(e.into_inner());
                    Ok(None)
                } else {
                    Err(e.into_inner())
                }
            }
        }
    }
}

pub trait RetryableError {
    fn can_retry(&self) -> bool;
}

#[derive(Debug)]
pub enum Error<E> {
    MaybeRetryable(E),
    Fatal(E),
    TimedOut(TimeoutError<E>),
}

impl<E: RetryableError> RetryableError for Error<E> {
    fn can_retry(&self) -> bool {
        match self {
            Self::MaybeRetryable(e) => e.can_retry(),
            Self::Fatal(_) => false,
            Self::TimedOut(_) => false,
        }
    }
}

impl<E> Error<E> {
    pub fn on_timeout(self, on_timeout: impl FnOnce(TimeoutError<E>) -> Self) -> Self {
        match self {
            Self::TimedOut(e) => on_timeout(e),
            other => other,
        }
    }

    pub fn map<E2>(self, map: impl FnOnce(E) -> E2) -> Error<E2> {
        match self {
            Self::TimedOut(e) => Error::TimedOut(TimeoutError {
                last_error: map(e.last_error),
            }),
            Self::MaybeRetryable(e) => Error::MaybeRetryable(map(e)),
            Self::Fatal(e) => Error::Fatal(map(e)),
        }
    }

    pub fn into_inner(self) -> E {
        match self {
            Self::TimedOut(e) => e.last_error,
            Self::MaybeRetryable(e) => e,
            Self::Fatal(e) => e,
        }
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub struct TimeoutError<E> {
    pub last_error: E,
}

fn blocking_sleep_until(instant: Instant) {
    let now = Instant::now();

    if let Some(sleep_duration) = instant.checked_duration_since(now) {
        std::thread::sleep(sleep_duration);
    }
}
