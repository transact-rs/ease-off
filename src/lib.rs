// Note: crate root documentation is in the following file:
#![doc = include_str!("lib.md")]
use core::{calculate_jitter, duration_saturating_mul_f32};
use std::cmp;
use std::time::{Duration, Instant};

pub mod core;

/// Exponential backoff controller.
///
/// The constructors of this type use [`Options::DEFAULT`].
#[derive(Debug)]
pub struct EaseOff<E> {
    options: Options,
    started_at: Instant,
    next_delay: Duration,
    deadline: Option<Instant>,
    last_error: Option<E>,
}

#[derive(Debug, Clone)]
pub struct Options {
    multiplier: f32,
    jitter: f32,
    initial_jitter: f32,
    initial_delay: Duration,
    max_delay: Duration,
}

impl Options {
    /// Default ease-off options which should be suitable for most applications.
    ///
    /// See source for current values.
    pub const DEFAULT: Options = Options {
        multiplier: 2.0,
        jitter: 0.25,
        initial_jitter: 0.0,
        initial_delay: Duration::from_millis(150),
        max_delay: Duration::from_secs(60), // one minute
    };

    /// Returns [`Self::DEFAULT`].
    #[inline(always)]
    pub const fn new() -> Self {
        Self::DEFAULT
    }

    /// Set the factor to multiply the next delay by.
    ///
    /// * If `> 1`, backoff is exponential.
    /// * If `== 1`, backoff is constant before [jitter][Self::jitter].
    /// * If `< 1`, backoff is logarithmic(?). In any case, not recommended.
    ///
    /// Any multiplication that results in an invalid value for [`Duration`] saturates
    /// to [`Duration::MAX`] or [`max_delay`][Self::max_delay], whichever is lower.
    #[inline(always)]
    pub const fn multiplier(self, multiplier: f32) -> Self {
        Self { multiplier, ..self }
    }

    /// Get the factor that the next delay will be multiplied by.
    #[inline(always)]
    pub const fn get_multiplier(&self) -> f32 {
        self.multiplier
    }

    /// Set the maximum jitter factor.
    ///
    /// The next backoff delay will be multiplied by a random factor in the range `(1 - jitter, 1]`.
    ///
    /// This helps prevent a situation where attempts line up from multiple processes
    /// following the same backoff algorithm, which would constitute a [thundering herd].
    ///
    /// This value is clamped to the interval `[0, 1]` when calculating the next delay.
    ///
    /// If `jitter` is `<= 0` or `NaN`, no random jitter is applied (not recommended for most cases).
    ///
    /// If `jitter >= 1`, the next delay can be anywhere between `[0, next_delay]`,
    /// which means the next attempt _could_ happen immediately, without waiting.
    ///
    /// [thundering herd]: https://en.wikipedia.org/wiki/Thundering_herd_problem
    #[inline(always)]
    pub const fn jitter(self, jitter: f32) -> Self {
        Self { jitter, ..self }
    }

    /// Get the maximum jitter factor.
    ///
    /// See [`Self::jitter()`] for details.
    #[inline(always)]
    pub const fn get_jitter(&self) -> f32 {
        self.jitter
    }

    /// Set the jitter factor used for the _first_ attempt.
    ///
    /// The initial wait before the first attempt will be [`initial_delay`]
    /// multiplied by a random factor in the range `(1 - initial_jitter, 1]`.
    ///
    /// This mitigates the [thundering herd problem] when multiple processes start up
    /// at the same time and all try to access the same resource.
    ///
    /// This value is clamped to the interval `[0, 1]` when calculating the initial delay.
    ///
    /// If `initial_jitter` is `<= 0` or `NaN`, the first attempt occurs immediately.
    ///
    /// The delay after the first failure will be calculated as normal;
    /// [`multiplier`][Self::multiplier] is _not_ applied until after the first retryable failure.
    ///
    /// [thundering herd problem]: https://en.wikipedia.org/wiki/Thundering_herd_problem
    #[inline(always)]
    pub const fn initial_jitter(self, initial_jitter: f32) -> Self {
        Self { initial_jitter, .. self }
    }

    /// Get the jitter factor used for the _first_ attempt.
    ///
    /// See [`Self::initial_jitter()`] for details.
    #[inline(always)]
    pub const fn get_initial_jitter(&self) -> f32 {
        self.initial_jitter
    }

    /// Set the delay for the first backoff attempt.
    #[inline(always)]
    pub const fn initial_delay(self, initial_delay: Duration) -> Self {
        Self { initial_delay, ..self }
    }

    /// Get the delay for the first backoff attempt.
    ///
    /// See [`Self::initial_delay()`] for details.
    #[inline(always)]
    pub const fn get_initial_delay(&self) -> Duration {
        self.initial_delay
    }

    /// Set the maximum delay to wait between backoff attempts.
    #[inline(always)]
    pub const fn max_delay(self, max_delay: Duration) -> Self {
        Self { max_delay, ..self }
    }

    /// Get the maximum delay to wait between backoff attempts.
    ///
    /// See [`Self::max_delay()`] for details.
    #[inline(always)]
    pub const fn get_max_delay(&self) -> Duration {
        self.max_delay
    }

    /// Begin backing off with **indefinite** retries.
    ///
    /// The operation will be retried until it succeeds, or a non-retryable error occurs.
    #[inline(always)]
    pub fn start_unlimited<E>(&self) -> EaseOff<E> {
        self.start(Instant::now(), None)
    }

    /// Begin backing off, limited by the given timeout.
    ///
    /// Always makes one attempt, even if the timeout is zero or has elapsed
    /// by the time the first attempt is made.
    ///
    /// See also:
    /// * [`Self::start_timeout_opt()`] for a conditional timeout.
    /// * [`Self::start_deadline()`] to specify an [`Instant`] as a deadline.
    #[inline(always)]
    pub fn start_timeout<E>(&self, timeout: Duration) -> EaseOff<E> {
        let started_at = Instant::now();
        self.start(started_at, started_at.checked_add(timeout))
    }

    /// Begin backing off, limited by the given optional timeout.
    ///
    /// If `timeout` is `None`, this is equivalent to [`Self::start_unlimited()`].
    ///
    /// Always makes one attempt, even if the timeout is zero or has elapsed
    /// by the time the first attempt is made.
    ///
    /// See also:
    /// * [`Self::start_timeout()`] for a non-conditional timeout.
    /// * [`Self::start_deadline_opt()`] to specify an optional [`Instant`] as a deadline.
    #[inline(always)]
    pub fn start_timeout_opt<E>(&self, timeout: Option<Duration>) -> EaseOff<E> {
        let started_at = Instant::now();
        self.start(started_at, timeout.and_then(|timeout| started_at.checked_add(timeout)))
    }

    /// Begin backing off, halting attempts at the given deadline.
    ///
    /// Always makes one attempt, even if the deadline is `<= Instant::now()` or has elapsed
    /// by the time the first attempt is made.
    ///
    /// See also:
    /// * [`Self::start_deadline_opt()`] for a conditional deadline.
    /// * [`Self::start_timeout()`] to specify a [`Duration`] as a timeout.
    #[inline(always)]
    pub fn start_deadline<E>(&self, deadline: Instant) -> EaseOff<E> {
        self.start(Instant::now(), Some(deadline))
    }

    /// Begin backing off, halting attempts at the given deadline.
    ///
    /// If `deadline` is `None`, this is equivalent to [`Self::start_unlimited()`].
    ///
    /// Always makes one attempt, even if the deadline is `<= Instant::now()` or has elapsed
    /// by the time the first attempt is made.
    ///
    /// See also:
    /// * [`Self::start_deadline()`] for a non-conditional deadline.
    /// * [`Self::start_timeout_opt()`] to specify an optional [`Duration`] as a timeout.
    #[inline(always)]
    pub fn start_deadline_opt<E>(&self, deadline: Option<Instant>) -> EaseOff<E> {
        self.start(Instant::now(), deadline)
    }

    #[inline(always)]
    fn start<E>(&self, started_at: Instant, deadline: Option<Instant>) -> EaseOff<E> {
        EaseOff {
            options: self.clone(),
            started_at,
            next_delay: self.initial_delay,
            deadline,
            last_error: None,
        }
    }
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

    fn next_sleep_until(&mut self) -> Result<Option<Instant>, Error<E>> {
        let Options { jitter, initial_jitter, multiplier, initial_delay, max_delay, .. } = self.options;

        let now = Instant::now();

        if self.last_error.is_none() {
            return Ok(
                (initial_jitter > 0f32)
                    .then(|| calculate_jitter(initial_delay, initial_jitter))
                    .and_then(|jitter| now.checked_add(jitter))
            );
        }

        // We attempt the operation at least once, even if the deadline has passed.
        if let Some(e) = self.last_error.take_if(|_| self.deadline.is_some_and(|deadline| now > deadline)) {
            return Err(Error::TimedOut(TimeoutError { last_error: e }));
        }

        let jitter = calculate_jitter(self.next_delay, jitter);

        let sleep_until = now + jitter;

        let sleep_until = self.deadline.map(|deadline| {
            // If the deadline will pass before the next sleep,
            // just sleep until the deadline minus jitter
            cmp::min(sleep_until, deadline - jitter)
        })
            .unwrap_or(sleep_until);

        self.next_delay = cmp::min(
            duration_saturating_mul_f32(self.next_delay, multiplier),
            max_delay
        );

        Ok(Some(sleep_until))
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
    pub fn try_blocking<T>(&mut self, op: impl FnOnce() -> Result<T, E>) -> ResultWrapper<'_, T, E> {
        try_sleep!(self, time => blocking_sleep_until(time));

        ResultWrapper {
            result: op().map_err(Error::MaybeRetryable),
            last_error: &mut self.last_error,
        }
    }

    #[cfg(feature = "tokio")]
    #[cfg_attr(docsrs, doc(cfg(feature = "tokio")))]
    pub async fn try_async<T>(&mut self, op: impl std::future::Future<Output = Result<T, E>>) -> ResultWrapper<'_, T, E> {
        self.try_async_with(move || op).await
    }

    #[cfg(feature = "tokio")]
    #[cfg_attr(docsrs, doc(cfg(feature = "tokio")))]
    pub async fn try_async_with<T, F, Fut>(&mut self, op: F) -> ResultWrapper<'_, T, E>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, E>>
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
    pub fn on_timeout(self, on_timeout: impl FnOnce(TimeoutError<E>) -> Error<E>) -> ResultWrapper<'a, T, E> {
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

    pub fn or_retry(self) -> Result<Option<T>, E> where E: RetryableError {
        self.or_retry_if(RetryableError::can_retry)
    }

    pub fn or_retry_if(self, can_retry: impl FnOnce(&Error<E>) -> bool) -> Result<Option<T>, E> {
        match self.result {
            Ok(success) => Ok(Some(success)),
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
            Self::TimedOut(e) => Error::TimedOut(TimeoutError { last_error: map(e.last_error) }),
            Self::MaybeRetryable(e) => Error::MaybeRetryable(map(e)),
            Self::Fatal(e) => Error::Fatal(map(e))
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
