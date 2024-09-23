//! Core backoff API, without error management or sleeps.

use crate::Options;
use std::cmp;
use std::time::{Duration, Instant};

/// Core backoff API, without error management or sleeps.
#[derive(Debug)]
pub struct EaseOffCore {
    options: Options,
    next_delay: Option<Duration>,
    deadline: Option<Instant>,
}

#[derive(Debug, thiserror::Error)]
#[error("the given deadline has elapsed")]
pub struct DeadlineElapsed(());

impl EaseOffCore {
    pub fn new(options: Options, deadline: Option<Instant>) -> Self {
        Self {
            options,
            next_delay: None,
            deadline,
        }
    }

    /// Returns the initial sleep if [`initial_jitter`] is finite and greater than zero.
    ///
    /// Returns `None` if [`Self::next_sleep_until()`] has already been called.
    ///
    /// [`initial_jitter`]: Options::initial_jitter
    pub fn initial_sleep_until(&self) -> Option<Instant> {
        if self.next_delay.is_some() {
            return None;
        }

        let Options {
            initial_jitter,
            initial_delay,
            ..
        } = self.options;

        (initial_jitter > 0f32).then(|| {
            let jitter = apply_jitter(initial_delay, initial_jitter);

            let sleep_until = Instant::now() + initial_delay - jitter;

            self.clamp_at_deadline(sleep_until, jitter)
        })
    }

    /// Returns the next [`Instant`] to sleep until, or an error if the deadline has elapsed.
    pub fn next_sleep_until(&mut self) -> Result<Instant, DeadlineElapsed> {
        let Options {
            multiplier,
            jitter,
            initial_delay,
            max_delay,
            ..
        } = self.options;

        let now = Instant::now();

        if self.deadline.is_some_and(|deadline| now > deadline) {
            return Err(DeadlineElapsed(()));
        }

        let next_delay = self.next_delay.insert(initial_delay);

        *next_delay = cmp::min(
            duration_saturating_mul_f32(*next_delay, multiplier),
            max_delay,
        );

        let jitter = apply_jitter(*next_delay, jitter);

        // We only subtract jitter so that `deadline` is a hard limit
        let sleep_until = now + *next_delay - jitter;

        Ok(self.clamp_at_deadline(sleep_until, jitter))
    }

    #[inline(always)]
    fn clamp_at_deadline(&self, sleep_until: Instant, jitter: Duration) -> Instant {
        self.deadline.map_or(sleep_until, |deadline| {
            cmp::min(sleep_until, deadline - jitter)
        })
    }
}

// This does not exist in `std`
#[inline(always)]
fn duration_saturating_mul_f32(duration: Duration, mul: f32) -> Duration {
    Duration::try_from_secs_f32(duration.as_secs_f32() * mul).unwrap_or(Duration::MAX)
}

#[inline(always)]
fn apply_jitter(base_duration: Duration, jitter_factor: f32) -> Duration {
    let jitter_factor = if jitter_factor > 0f32 && jitter_factor < 1f32 {
        1f32 - (jitter_factor * rand::random::<f32>())
    } else if jitter_factor >= 1f32 {
        // Act as if `jitter == 1`
        1f32 - rand::random::<f32>()
    } else {
        // `jitter` is NaN or <= 0
        0f32
    };

    duration_saturating_mul_f32(base_duration, jitter_factor)
}
