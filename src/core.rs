//! Immutable core backoff API, without error management or sleeps.
//!
//! Potentially useful for when stateful backoffs are not appropriate,
//! e.g. retrying many similar operations concurrently,
//! or when a convenient error type is not available.
//!
//! # Example: Schedule Many Operations with `tokio_util::time::DelayQueue`
//!
//! (Source: `examples/tokio-concurrent.rs`)
#![cfg_attr(feature = "tokio", doc = "```rust")]
#![cfg_attr(
    not(feature = "tokio"),
    doc = "```rust,ignore\n\
           // Note: example not compiled if `tokio` feature is not enabled.\n"
)]
#![doc = include_str!("../examples/tokio-concurrent.rs")]
// If this were written using `//!`, RustRover would think this is the start of a new code block.
#![doc = "```"]

use crate::options::Options;
use rand::Rng;
use std::cmp;
use std::time::{Duration, Instant};

/// Immutable core backoff API, without error management or sleeps.
#[derive(Debug, Clone)]
pub struct EaseOffCore {
    options: Options,
}

/// Error returned by [`EaseOffCore::nth_retry_at()`].
#[derive(Debug, Clone, thiserror::Error)]
#[error("{n}th retry is {:?} after deadline", retry_at.duration_since(*deadline))]
pub struct RetryAfterDeadline {
    /// The `n` passed to `nth_retry_at()`.
    pub n: u32,
    /// The recommended time for the `n`th backoff attempt.
    pub retry_at: Instant,
    pub deadline: Instant,
}

impl EaseOffCore {
    /// Create an instance from a built [`Options`].
    ///
    /// May be more conveniently invoked as [`Options::into_core()`].
    #[inline(always)]
    pub const fn new(options: Options) -> Self {
        Self { options }
    }

    /// Returns the recommended [`Instant`] at which to schedule the `n`th backoff attempt.
    ///
    /// Returns `Ok(None)` if `n == 0` and [`Options::initial_jitter`] is not greater than zero.
    ///
    /// Returns `Err` if the calculated [`Instant`] falls after `deadline`.
    pub fn nth_retry_at(
        &self,
        n: u32,
        now: Instant,
        deadline: Option<Instant>,
        rng: &mut (impl Rng + ?Sized),
    ) -> Result<Option<Instant>, RetryAfterDeadline> {
        let Options {
            multiplier,
            jitter,
            initial_jitter,
            initial_delay,
            max_delay,
        } = self.options;

        let (delay, jitter) = if let Some(powi) = n.checked_sub(1) {
            let delay = cmp::min(
                duration_saturating_mul_f32(
                    initial_delay,
                    multiplier.powi(powi.try_into().unwrap_or(i32::MAX)),
                ),
                max_delay,
            );

            let jitter = get_jitter(delay, jitter, rng);

            (delay, jitter)
        } else {
            if !(initial_jitter > 0f32) {
                return Ok(None);
            }

            let jitter = get_jitter(initial_delay, initial_jitter, rng);
            (initial_delay, jitter)
        };

        // We only subtract jitter so that `deadline` is a hard limit
        let retry_at = now + delay - jitter;

        match deadline {
            Some(deadline) if retry_at > deadline => Err(RetryAfterDeadline {
                n,
                retry_at,
                deadline,
            }),
            _ => Ok(Some(retry_at)),
        }
    }
}

// This does not exist in `std`
#[inline(always)]
fn duration_saturating_mul_f32(duration: Duration, mul: f32) -> Duration {
    Duration::try_from_secs_f32(duration.as_secs_f32() * mul).unwrap_or(Duration::MAX)
}

fn get_jitter(
    base_duration: Duration,
    jitter_factor: f32,
    rng: &mut (impl Rng + ?Sized),
) -> Duration {
    let jitter_factor = if jitter_factor > 0f32 && jitter_factor < 1f32 {
        jitter_factor * rng.gen::<f32>()
    } else if jitter_factor >= 1f32 {
        // Act as if `jitter == 1`
        rng.gen::<f32>()
    } else {
        // `jitter` is NaN or <= 0
        0f32
    };

    duration_saturating_mul_f32(base_duration, jitter_factor)
}
