//! A per-test wall-clock watchdog (plan 30 §7, R05), shared by the three FakeDevice test
//! files: `crates/ezsdr-radio-uhd/tests/common/mod.rs` declares and re-exports it, and
//! `crates/ezsdr-acceptance/tests/uhd.rs` and `crates/ezsdr-server/tests/protocol.rs` include
//! this file with `#[path]` (as `ezsdr-mock-radio`'s `tests/timeline.rs` includes the radio
//! generator's module). It lives here rather than inside `mod.rs` because an integration
//! test's modules are not visible to another crate's test target, and the three files need
//! the same one.
//!
//! The manual clock's time stops while a Provider thread is busy (M3); a mutant that never
//! resumes it leaves a test waiting for an instant that never comes, and neither the test
//! harness nor `mutate.py` bounds that before their own (900 s / 180 s) timeout. [`watch()`]
//! starts a wall-clock thread for the calling test that aborts the process with a message
//! once the bound passes, so the test *fails* (under a mutant, is killed) instead of hanging.
//!
//! The armed guard lives in the test thread's thread-local and is dropped — disarming the
//! watchdog — when that thread ends; libtest runs every test on a thread of its own, under
//! `--test-threads=1` too (the thread's name is the test's). A hang, which is what the bound
//! is for, never reaches the drop. `EZSDR_TEST_WATCHDOG_SECS` overrides the bound.

use std::cell::RefCell;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration as Wall, Instant};

/// How long a test may run before the watchdog aborts it. Generous: the slowest smoke test is
/// ~50 s on the implementer's Mac, and a mutation shard on the lab hosts runs several tests at
/// once. Below `mutate.py`'s 180 s, so a frozen clock is a killed row, not a timeout.
const BOUND: Wall = Wall::from_secs(120);

struct Watchdog {
    stop: Arc<(Mutex<bool>, Condvar)>,
}

impl Watchdog {
    /// Starts the watchdog for the calling thread's test.
    fn arm() -> Watchdog {
        Watchdog::start(std::thread::Builder::new())
    }

    /// [`Self::arm`] on `builder`. A watchdog that cannot start fails its test at once: the
    /// test would otherwise run unguarded and a frozen clock would hang it (R15).
    fn start(builder: std::thread::Builder) -> Watchdog {
        let bound = std::env::var("EZSDR_TEST_WATCHDOG_SECS").ok().and_then(|s| s.parse::<u64>().ok()).map_or(BOUND, Wall::from_secs);
        let what = std::thread::current().name().unwrap_or("a test").to_owned();
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let flag = stop.clone();
        let thread = builder.name("test-watchdog".to_owned()).spawn(move || {
            let (lock, cond) = &*flag;
            let deadline = Instant::now() + bound;
            let mut stopped = lock.lock().unwrap_or_else(|e| e.into_inner());
            while !*stopped {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    eprintln!(
                        "the test `{what}` did not finish within {bound:?}: a participant's clock wait \
                         never ended (plan 30 §7's watchdog; EZSDR_TEST_WATCHDOG_SECS overrides the \
                         bound). Aborting."
                    );
                    std::process::abort();
                }
                stopped = cond.wait_timeout(stopped, left).unwrap_or_else(|e| e.into_inner()).0;
            }
        });
        thread.expect("the test's watchdog thread did not start (plan 30 §7)");
        Watchdog { stop }
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        let (lock, cond) = &*self.stop;
        *lock.lock().unwrap_or_else(|e| e.into_inner()) = true;
        cond.notify_all();
    }
}

thread_local! {
    static ARMED: RefCell<Option<Watchdog>> = const { RefCell::new(None) };
}

/// Arms the watchdog for the calling test, once (a second call in the same test does nothing).
pub fn watch() {
    ARMED.with(|armed| {
        let mut armed = armed.borrow_mut();
        if armed.is_none() {
            *armed = Some(Watchdog::arm());
        }
    });
}

#[test]
#[should_panic(expected = "watchdog thread did not start")]
fn a_watchdog_that_cannot_start_fails_its_test() {
    // A stack no OS can map: `Builder::spawn` returns `Err` (EAGAIN on macOS and Linux).
    let _ = Watchdog::start(std::thread::Builder::new().stack_size(1 << 60));
}
