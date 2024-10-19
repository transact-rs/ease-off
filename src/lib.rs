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
#![cfg_attr(feature = "tokio", doc = "```rust")]
#![cfg_attr(
    not(feature = "tokio"),
    doc = "```rust,ignore\n\
           // Note: example not compiled if `tokio` feature is not enabled.\n"
)]
#![doc = include_str!("../examples/tokio.rs")]
// If this were written using `//!`, RustRover would think this is the start of a new code block.
#![doc = "```"]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]

use crate::core::EaseOffCore;
use std::num::Saturating;
use std::time::{Duration, Instant};

#[cfg(feature = "__futures")]
#[cfg_attr(docsrs, doc(cfg(any(feature = "tokio", feature = "async-io-2"))))]
pub mod futures;

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

    fn next_retry_at(&mut self) -> Result<Option<Instant>, Error<E>> {
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

    fn wrap_result<T>(&mut self, result: Result<T, Error<E>>) -> ResultWrapper<'_, T, E> {
        ResultWrapper {
            result,
            last_error: &mut self.last_error,
        }
    }
}

impl<E> EaseOff<E> {
    /// Attempt a blocking operation.
    ///
    /// If the operation previously failed, sleeps for the prescribed backoff period
    /// using [`std::thread::sleep()`].
    ///
    /// ### Note: Behavior at Deadline
    /// Most blocking operations cannot be cancelled once begun, so the [deadline][Self::deadline],
    /// if set, is only checked *before* attempting the operation.
    ///
    /// Generally, the only kinds of blocking operations that support cancellation
    /// take an explicit timeout (such as setting a read timeout on a socket).
    ///
    /// If you want a blocking operation to be cancelled immediately once the deadline elapses,
    /// consult the documentation for the API you are calling to see if timeouts are supported,
    /// and if so, how to configure them.
    pub fn try_blocking<T>(
        &mut self,
        op: impl FnOnce() -> Result<T, E>,
    ) -> ResultWrapper<'_, T, E> {
        match self.next_retry_at() {
            Ok(Some(instant)) => {
                blocking_sleep_until(instant);
            }
            Ok(None) => (),
            Err(e) => return self.wrap_result(Err(e)),
        }

        self.wrap_result(op().map_err(Error::MaybeRetryable))
    }
}

/// Wrapper for [`Result`] returned from methods on [`EaseOff`].
///
/// Retryable errors will be stored in the `EaseOff` to be returned on the next attempt
/// if the [deadline][EaseOff::deadline] has passed.
#[must_use = "`.or_retry()` or `.or_retry_if()` must be called"]
pub struct ResultWrapper<'a, T, E: 'a> {
    result: Result<T, Error<E>>,
    last_error: &'a mut Option<E>,
}

impl<'a, T, E: 'a> ResultWrapper<'a, T, E> {
    /// Convert a [`TimeoutError`], if applicable, to another [`Error`] variant.
    ///
    /// May be used to convert a timeout error into [`Error::MaybeRetryable`].
    ///
    /// If not otherwise handled, `.or_retry()` and `.or_retry_if()` will return the error.
    pub fn on_timeout(
        self,
        on_timeout: impl FnOnce(TimeoutError<E>) -> Error<E>,
    ) -> ResultWrapper<'a, T, E> {
        Self {
            result: self.result.map_err(|e| e.on_timeout(on_timeout)),
            last_error: self.last_error,
        }
    }

    /// Inspect the error if the operation failed.
    ///
    /// This could also be [`Error::TimedOut`] containing an error from a previous iteration.
    pub fn inspect_err(self, inspect_err: impl FnOnce(&Error<E>)) -> Self {
        Self {
            result: self.result.inspect_err(inspect_err),
            last_error: self.last_error,
        }
    }

    /// Check the result, testing the error for retryability using [`RetryableError`] if applicable.
    ///
    /// If the operation was successful, `Ok(Some(_))` is returned.
    ///
    /// If the operation failed but [`RetryableError::can_retry()`] returned `true`,
    /// `Ok(None)` is returned and the error is stored in the [`EaseOff`] instance for the next
    /// iteration.
    ///
    /// If the error was determined to be fatal or the [deadline][EaseOff::deadline()] has elapsed,
    /// `Err` is returned.
    pub fn or_retry(self) -> Result<Option<T>, E>
    where
        E: RetryableError,
    {
        self.or_retry_if(RetryableError::can_retry)
    }

    /// Check the result, testing the error for retryability using the given closure if applicable.
    ///
    /// The closure will be invoked with either the error from the current attempt,
    /// or a previous error if the [deadline][EaseOff::deadline()] has elapsed
    /// (this is indicated by the [`Error::TimedOut`] variant).
    ///
    /// If the operation was successful, `Ok(Some(_))` is returned.
    ///
    /// If the operation failed but the given closure returns `true`,
    /// `Ok(None)` is returned and the error is stored in the [`EaseOff`] instance for the next
    /// iteration.
    ///
    /// If the error was determined to be fatal, `Err` is returned.
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

/// Trait which may be implemented for error types to enable code reuse with [`EaseOff`].
pub trait RetryableError {
    /// Returns `true` if the error is non-fatal, `false` otherwise.
    fn can_retry(&self) -> bool;
}

/// Error type for [`EaseOff`] which includes the fatality level of the error.
#[derive(Debug)]
pub enum Error<E> {
    /// The inner error has not been determined to be fatal yet.
    ///
    /// [`RetryableError::can_retry()`] passes through to the inner error.
    MaybeRetryable(E),
    /// The error was determined to be fatal.
    ///
    /// Always returns `false` from [`RetryableError::can_retry()`].
    Fatal(E),
    /// The [deadline][EaseOff::deadline()] has elapsed.
    ///
    /// Contained is the error from the most recent attempt.
    ///
    /// Always returns `false` from [`RetryableError::can_retry()`].
    TimedOut(TimeoutError<E>),
}

/// Error wrapper type indicating a failure due to a [deadline][EaseOff::deadline()] elapsing.
#[derive(Debug)]
#[non_exhaustive]
pub struct TimeoutError<E> {
    /// The error from the most recent failed attempt.
    pub last_error: E,
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
    /// Convert [`Error::TimedOut`], if applicable, to another variant.
    pub fn on_timeout(self, on_timeout: impl FnOnce(TimeoutError<E>) -> Self) -> Self {
        match self {
            Self::TimedOut(e) => on_timeout(e),
            other => other,
        }
    }

    /// Map the inner error type, retaining its retryability status.
    pub fn map<E2>(self, map: impl FnOnce(E) -> E2) -> Error<E2> {
        match self {
            Self::TimedOut(e) => Error::TimedOut(TimeoutError {
                last_error: map(e.last_error),
            }),
            Self::MaybeRetryable(e) => Error::MaybeRetryable(map(e)),
            Self::Fatal(e) => Error::Fatal(map(e)),
        }
    }

    /// Get the inner error.
    pub fn inner(&self) -> &E {
        match self {
            Self::TimedOut(e) => &e.last_error,
            Self::MaybeRetryable(e) => e,
            Self::Fatal(e) => e,
        }
    }

    /// Unwrap the inner error.
    pub fn into_inner(self) -> E {
        match self {
            Self::TimedOut(e) => e.last_error,
            Self::MaybeRetryable(e) => e,
            Self::Fatal(e) => e,
        }
    }
}

fn blocking_sleep_until(instant: Instant) {
    let now = Instant::now();

    if let Some(sleep_duration) = instant.checked_duration_since(now) {
        std::thread::sleep(sleep_duration);
    }
}
