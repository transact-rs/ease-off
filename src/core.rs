use std::time::Duration;

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
