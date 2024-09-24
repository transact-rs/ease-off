use crate::core::EaseOffCore;
use crate::EaseOff;
use std::num::Saturating;
use std::time::{Duration, Instant};

/// Configuration options for [`EaseOff`] and [`EaseOffCore`].
///
/// Designed to be stored in a `const` or `static`:
///
/// ```rust
/// use std::time::Duration;
///
/// const BACKOFF_OPTS: ease_off::Options = ease_off::Options::new()
///     .initial_jitter(0.25)
///     .initial_delay(Duration::from_secs(1))
///     .max_delay(Duration::from_secs(5 * 60)); // 5 minutes
/// ```
#[derive(Debug, Clone)]
pub struct Options {
    pub(crate) multiplier: f32,
    pub(crate) jitter: f32,
    pub(crate) initial_jitter: f32,
    pub(crate) initial_delay: Duration,
    pub(crate) max_delay: Duration,
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

    /// Set the jitter factor used to delay the first attempt.
    ///
    /// The initial wait before the first attempt will be [`initial_delay`][Self::initial_delay]
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
        Self {
            initial_jitter,
            ..self
        }
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
        Self {
            initial_delay,
            ..self
        }
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

    /// Convert this `Options` into an [`EaseOffCore`].
    #[inline(always)]
    pub const fn into_core(self) -> EaseOffCore {
        EaseOffCore::new(self)
    }
}

/// Methods to create an [`EaseOff`].
impl Options {
    /// Begin backing off with **indefinite** retries.
    ///
    /// The operation will be retried until it succeeds, or a non-retryable error occurs.
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
    pub fn start_timeout_opt<E>(&self, timeout: Option<Duration>) -> EaseOff<E> {
        let started_at = Instant::now();
        self.start(
            started_at,
            timeout.and_then(|timeout| started_at.checked_add(timeout)),
        )
    }

    /// Begin backing off, halting attempts at the given deadline.
    ///
    /// Always makes one attempt, even if the deadline is `<= Instant::now()` or has elapsed
    /// by the time the first attempt is made.
    ///
    /// See also:
    /// * [`Self::start_deadline_opt()`] for a conditional deadline.
    /// * [`Self::start_timeout()`] to specify a [`Duration`] as a timeout.
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
    pub fn start_deadline_opt<E>(&self, deadline: Option<Instant>) -> EaseOff<E> {
        self.start(Instant::now(), deadline)
    }

    fn start<E>(&self, started_at: Instant, deadline: Option<Instant>) -> EaseOff<E> {
        EaseOff {
            core: EaseOffCore::new(self.clone()),
            started_at,
            deadline,
            num_attempts: Saturating(0),
            last_error: None,
        }
    }
}

impl Default for Options {
    /// Returns [`Self::DEFAULT`].
    #[inline(always)]
    fn default() -> Self {
        Self::DEFAULT
    }
}
