use std::future::Future;
use std::time::{Duration, Instant};
use rand::Rng;

pub struct EaseOff<E> {
    deadline: Instant,
    jitter: f32,
    multiplier: f32,
    next_delay: Duration,
    max_delay: Duration,
    last_error: Option<E>,
}

impl<E> EaseOff<E> {
    pub fn new_timeout(timeout: Duration) -> Self {
        Self::new_deadline(Instant::now() + timeout)
    }

    pub fn new_deadline(deadline: Instant) -> Self {
        Self {
            deadline,
            jitter: 0.25,
            multiplier: 2.0,
            next_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            last_error: None
        }
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
        if self.last_error.is_none() {
            return Ok(None);
        }

        let now = Instant::now();

        let jitter = if self.jitter > 0f32 && self.jitter <= 1f32 {
            rand::thread_rng().gen_range(1. - self.jitter .. 1. + self.jitter)
        } else {
            1f32
        };

        let delay = self.next_delay.mul_f32(jitter);

        self.next_delay = self.next_delay.mul_f32(self.multiplier);

        let sleep_until = now + delay;

        Ok((sleep_until <= self.deadline).then_some(sleep_until))
    }
}

macro_rules! try_sleep(
    (self, $instant:ident => $sleep:expr) => {
        match self.next_sleep_until() {
            Ok(Some($instant)) => {
                $sleep
            }
            Ok(None) => (),
            Err(e) => {
                return ResultWrapper {
                    result: Err(e),
                    last_error: &mut self.last_error
                }
            }
        }
    }
);

impl<E> EaseOff<E> {
    pub fn try_blocking<T>(&mut self, op: impl FnOnce() -> Result<T, E>) -> ResultWrapper<'_, T, E> {
        try_sleep!(self, time => sleep_until(time));

        ResultWrapper {
            result: op(),
            last_error: &mut self.last_error,
        }
    }

    pub async fn try_async<T>(&mut self, op: impl Future<Output = Result<T, E>>) -> ResultWrapper<'_, T, E> {
        self.try_async_with(move || op).await
    }

    pub async fn try_async_with<T, F, Fut>(&mut self, op: F) -> ResultWrapper<'_, T, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, E>>
    {
        try_sleep!(self, time => async_sleep_until(time).await);

        ResultWrapper {
            result: op().await,
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

    pub fn map_err<E2: 'a>(self, map_err: impl FnOnce(Error<E>) -> Error<E2>) -> ResultWrapper<'a, T, E2> {
        Self {
            result: self.result.map_err(map_err),
            last_error: self.last_error,
        }
    }

    pub fn or_fatal(self) -> Result<Option<T>, E> where E: RetryableError {
        self.or_fatal_if(RetryableError::is_fatal)
    }

    pub fn or_fatal_if(self, is_fatal: impl FnOnce(&Error<E>) -> bool) -> Result<Option<T>, E> {
        match self.result {
            Ok(success) => Ok(Some(success)),
            Err(e) => {
                if is_fatal(&e) {
                    Err(e)
                } else {
                    *self.last_error = Some(e);
                    Ok(None)
                }
            }
        }
    }
}

pub trait RetryableError {
    fn is_fatal(&self) -> bool;
}

pub enum Error<E> {
    MaybeRetryable(E),
    Fatal(E),
    TimedOut(TimeoutError<E>),
}

impl<E: RetryableError> RetryableError for Error<E> {
    fn is_fatal(&self) -> bool {
        match self {
            Self::MaybeRetryable(e) => e.is_fatal(),
            Self::Fatal(_) => true,
            Self::TimedOut(_) => true,
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

#[non_exhaustive]
pub struct TimeoutError<E> {
    pub last_error: E,
}

async fn async_sleep_until(instant: Instant) {
    #[cfg(feature = "tokio")]
    if let Ok(_runtime) = tokio::runtime::Handle::try_current() {
        tokio::time::sleep_until(instant.into()).await;
        return;
    }

    #[cfg(feature = "async-std")]
    {
        let now = Instant::now();

        if let Some(sleep_duration) = instant.checked_duration_since(now) {
            async_std::task::sleep(sleep_duration).await;
        }
    }

    #[cfg(not(feature = "async-std"))]
    {
        panic!("no Tokio or async-std runtime available")
    }
}

fn sleep_until(instant: Instant) {
    let now = Instant::now();

    if let Some(sleep_duration) = instant.checked_duration_since(now) {
        std::thread::sleep(sleep_duration);
    }
}
