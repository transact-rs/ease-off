use std::cmp;
use std::time::{Duration, Instant};

pub struct EaseOff<E> {
    limit: Limit,
    jitter: f32,
    multiplier: f32,
    next_delay: Duration,
    max_delay: Duration,
    last_error: Option<E>,
}

#[derive(Debug)]
enum Limit {
    Timeout(Duration),
    Deadline(Instant),
    Unlimited
}

impl<E> EaseOff<E> {
    #[inline(always)]
    pub fn new_unlimited() -> Self {
        Self {
            limit: Limit::Unlimited,
            jitter: 0.25,
            multiplier: 2.0,
            next_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            last_error: None
        }
    }

    pub fn new_timeout(timeout: Duration) -> Self {
        Self::new_unlimited().with_timeout(timeout)
    }

    pub fn new_deadline(deadline: Instant) -> Self {
        Self::new_unlimited().with_deadline(deadline)
    }

    pub fn with_timeout(self, timeout: Duration) -> Self {
        Self { limit: Limit::Timeout(timeout), ..self }
    }

    pub fn with_deadline(self, deadline: Instant) -> Self {
        Self { limit: Limit::Deadline(deadline), ..self }
    }

    pub fn without_deadline(self) -> Self {
        Self { limit: Limit::Unlimited, ..self }
    }

    pub fn with_jitter(self, jitter: f32) -> Self {
        Self { jitter, ..self }
    }

    pub fn with_multiplier(self, multiplier: f32) -> Self {
        Self { multiplier, ..self }
    }

    pub fn with_initial_delay(self, initial_delay: Duration) -> Self {
        Self { next_delay: initial_delay, ..self }
    }

    pub fn with_max_delay(self, max_delay: Duration) -> Self {
        Self { max_delay, ..self }
    }

    fn next_sleep_until(&mut self) -> Result<Option<Instant>, Error<E>> {
        let deadline = self.limit.make_deadline();

        let now = Instant::now();

        // We attempt the operation at least once, even if the deadline has passed.
        if self.last_error.is_none() {
            return Ok(None);
        }

        let jitter = if self.jitter > 0f32 && self.jitter <= 1f32 {
            self.next_delay.mul_f32(self.jitter * rand::random::<f32>())
        } else {
            Duration::ZERO
        };

        // We only subtract `jitter`, never add it, so `deadline` becomes a semi-hard cutoff
        let sleep_until = now + self.next_delay - jitter;

        let sleep_until = deadline.map(|deadline| {
            // If the deadline will pass before the next sleep,
            // just sleep until the deadline minus jitter
            cmp::min(sleep_until, deadline - jitter)
        })
            .unwrap_or(sleep_until);

        self.next_delay = cmp::min(self.next_delay.mul_f32(self.multiplier), self.max_delay);

        Ok(Some(sleep_until))
    }
}

impl Limit {
    fn make_deadline(&mut self) -> Option<Instant> {
        match *self {
            Self::Deadline(deadline) => Some(deadline),
            Self::Timeout(timeout) => {
                let deadline = Instant::now() + timeout;
                *self = Self::Deadline(deadline);
                Some(deadline)
            }
            Self::Unlimited => None,
        }
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
    pub fn try_blocking<T>(&mut self, op: impl FnOnce() -> Result<T, E>) -> ResultWrapper<'_, T, E> {
        try_sleep!(self, time => sleep_until(time));

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

fn sleep_until(instant: Instant) {
    let now = Instant::now();

    if let Some(sleep_duration) = instant.checked_duration_since(now) {
        std::thread::sleep(sleep_duration);
    }
}
