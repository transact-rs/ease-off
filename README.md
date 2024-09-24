# `ease-off`

An easy, opinionated exponential backoff implementation.

Benefits over other implementations:
* More flexible control flow (you implement the loop yourself).
* Opinionated but sane defaults
    * Explicitly choose deadline, timeout or unlimited,
      so you know exactly what's going to happen.
* `RetryableError` trait allows for more reusable code.
* Immutable, const-friendly `core` API for when stateful backoffs aren't appropriate.

## License

Licensed under either of

* Apache License, Version 2.0
  ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
* MIT license
  ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
