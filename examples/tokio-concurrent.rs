//! Example showing how `EaseOffCore` can be used to manage backoffs for multiple conccurrent
//! operations, without storing an instance of `EaseOff` for each.

use std::num::Saturating;
use std::time::{Duration, Instant};

use ease_off::core::EaseOffCore;
use futures::StreamExt;
use tokio_util::time::DelayQueue;

struct Operation {
    // By using `Saturating`, we don't have to worry about overflows if the operation
    // continues too long.
    num_attempts: Saturating<u32>,
    threshold: u32,
    deadline: Instant,
}

const OPERATION_TIMEOUT: Duration = Duration::from_secs(30);

// `static` is recommended over `const`;
// otherwise, an `EaseOffCore` will be instantiated on the stack wherever this is used.
static EASE_OFF: EaseOffCore = ease_off::Options::new()
    // Delay the first attempt by up to 25% of `initial_delay`
    .initial_jitter(0.25)
    // So the example doesn't take forever
    .max_delay(Duration::from_secs(1))
    .into_core();

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut scheduled_attempts: DelayQueue<usize> = DelayQueue::new();

    let started_at = Instant::now();

    // Start 20 operations
    let mut operations = (0u32..20)
        .map(|i| Operation {
            num_attempts: Saturating(0),
            threshold: i,
            deadline: started_at + OPERATION_TIMEOUT,
        })
        .collect::<Vec<_>>();

    // Note: this will make the resulting future `!Send`.
    //
    // This is fine for `async fn main()`, which does not have a `Send` requirement,
    // but may cause problems in a spawned task.
    //
    // If you get errors about a future not being `Send`,
    // scope the `rng` binding such that its lifetime does not cross an `.await` point.
    let mut rng = rand::thread_rng();

    // Schedule the initial attempts:
    for (i, _op) in operations.iter().enumerate() {
        scheduled_attempts.insert_at(
            i,
            EASE_OFF
                .nth_retry_at(0, started_at, None, &mut rng)
                .expect("passed `None` for deadline, should not be `Err`")
                .expect("initial_jitter is set, should not be `None`")
                // `nth_sleep_until()` returns `std::time::Instant`
                // but `DelayQueue` uses `tokio::time::Instant`;
                // luckily, the conversion is trivial.
                .into(),
        );
    }

    while let Some(expired) = scheduled_attempts.next().await {
        let i = expired.into_inner();
        let now = Instant::now();

        let operation = &mut operations[i];

        match try_operation(operation).await {
            Ok(message) => {
                println!(
                    "Operation {i} succeeded after {n} attempt(s) with message: {message}",
                    n = operation.num_attempts.0,
                )
            }
            Err(_) => {
                let attempt_num = operation.num_attempts.0;

                match EASE_OFF.nth_retry_at(attempt_num, now, Some(operation.deadline), &mut rng) {
                    Ok(Some(retry_at)) => {
                        scheduled_attempts.insert_at(i, retry_at.into());
                    }
                    Ok(None) => {
                        unreachable!("cannot be `None` if `attempt_num > 0` ({attempt_num})")
                    }
                    Err(e) => {
                        println!("Operation {i} timed out: {e}");
                    }
                }
            }
        }
    }
}

async fn try_operation(operation: &mut Operation) -> Result<String, ()> {
    // For the purposes of this example, imagine this is something non-trivial,
    // like making an HTTP request.
    if operation.num_attempts.0 > operation.threshold {
        Ok("operation success!".to_string())
    } else {
        operation.num_attempts += 1;
        Err(())
    }
}
