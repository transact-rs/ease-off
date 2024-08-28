use std::time::Duration;
use ease_off::{EaseOff, RetryableError};

struct FallibleOperation {
    tries_required: usize,
}

struct Success {
    message: String,
}

#[derive(Debug)]
struct Error {
    message: String,
}

impl RetryableError for Error {
    fn can_retry(&self) -> bool {
        true
    }
}

impl FallibleOperation {
    async fn try_op(&mut self) -> Result<Success, Error> {
        if self.tries_required > 0 {
            let remaining = self.tries_required;
            self.tries_required -= 1;

            Err(Error {
                message: format!("failure! tries remaining: {remaining}")
            })
        } else {
            Ok(Success {
                message: "yay! completed successfully!".to_string(),
            })
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let mut fallible = FallibleOperation {
        tries_required: 3,
    };

    let mut ease_off = EaseOff::new_timeout(Duration::from_secs(60));

    loop {
        let Some(Success { message }) = ease_off
            .try_async(fallible.try_op())
            .await
            .inspect_err(|e| println!("error: {e:?}"))
            .or_retry()?
        else {
            continue;
        };

        println!("completed with message: {message}");
        break;
    }

    Ok(())
}
