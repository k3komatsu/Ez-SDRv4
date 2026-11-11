//! What the server's test files share.

use std::time::{Duration, Instant};

/// Every bounded wait of the watched-input tests (#89): its expiry is the failure, and a correct
/// run passes in well under it.
pub const BOUND: Duration = Duration::from_secs(10);

/// Polls `probe` every millisecond for up to [`BOUND`] and returns its first `Some`.
pub fn eventually<T>(probe: impl FnMut() -> Option<T>) -> Option<T> {
    within(BOUND, probe)
}

/// As [`eventually`], for up to `bound`.
pub fn within<T>(bound: Duration, mut probe: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + bound;
    loop {
        if let Some(found) = probe() {
            return Some(found);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}
