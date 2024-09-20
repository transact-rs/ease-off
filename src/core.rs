use std::cmp;
use std::time::{Duration, Instant};

use crate::Options;

// This does not exist in `std`
#[inline(always)]
pub fn duration_saturating_mul_f32(duration: Duration, mul: f32) -> Duration {
    Duration::try_from_secs_f32(duration.as_secs_f32() * mul).unwrap_or(Duration::MAX)
}

#[inline(always)]
pub fn calculate_jitter(base_duration: Duration, jitter_factor: f32) -> Duration {
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

pub fn next_sleep_until(
    now: Instant,
    next_delay: &mut Duration,
    deadline: Option<Instant>,
    options: &Options,
) -> Instant {
    let jitter = calculate_jitter(*next_delay, options.jitter);

    let sleep_until = now + jitter;

    let sleep_until = deadline
        .map(|deadline| {
            // If the deadline will pass before the next sleep,
            // just sleep until the deadline minus jitter
            cmp::min(sleep_until, deadline - jitter)
        })
        .unwrap_or(sleep_until);

    *next_delay = cmp::min(
        duration_saturating_mul_f32(*next_delay, options.multiplier),
        options.max_delay,
    );

    sleep_until
}
