An easy, opinionated exponential backoff implementation.

Benefits over other implementations:
* More flexible control flow (you implement the loop yourself).
* Opinionated but sane defaults
    * Explicitly choose deadline, timeout or unlimited,
      so you know exactly what's going to happen.
* [`RetryableError`] trait allows for more reusable code.
