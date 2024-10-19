use ease_off::RetryableError;
use std::time::Duration;

struct FallibleOperation {
    tries_required: usize,
}

struct Success {
    message: String,
}

#[derive(Debug)]
pub struct Error {
    pub message: String,
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
                message: format!("failure! tries remaining: {remaining}"),
            })
        } else {
            Ok(Success {
                message: "yay! completed successfully!".to_string(),
            })
        }
    }
}

const OPTIONS: ease_off::Options = ease_off::Options::new()
    // Set a longer `initial_delay` so we can see it working
    .initial_delay(Duration::from_secs(1));

#[async_std::main]
async fn main() -> Result<(), Error> {
    let mut fallible = FallibleOperation { tries_required: 3 };

    // If you just want to use the default options:
    // let mut ease_off = EaseOff::start_timeout(Duration::from_secs(60));
    let mut ease_off = OPTIONS.start_timeout(Duration::from_secs(60));

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
