//! Which of the coordinator's locks the calling thread holds, kept by `Locked::lock` in every build (KC-24a): `any_held` lets `wake` refuse to schedule under a lock, and `lock_record::held` (the `testing` feature) reads which ones, for a test double at each supplied call.

use std::cell::Cell;
use std::ops::{Deref, DerefMut};
use std::sync::MutexGuard;

use super::state::Which;

thread_local! {
    static COUNTS: Cell<[u32; 4]> = const { Cell::new([0; 4]) };
}

/// The calling thread's count per `Which`, for `lock_record::held`.
#[cfg(feature = "testing")]
pub(super) fn counts() -> [u32; 4] {
    COUNTS.with(Cell::get)
}

/// Whether the calling thread holds any lock of the coordinator's own.
pub(super) fn any_held() -> bool {
    COUNTS.with(Cell::get).iter().any(|count| *count > 0)
}

/// A guard that counts itself in the acquiring thread's record until it drops. `MutexGuard` is not `Send`, so neither is this.
pub(super) struct Recorded<'a, T> {
    guard: MutexGuard<'a, T>,
    which: Which,
}

impl<'a, T> Recorded<'a, T> {
    pub(super) fn new(guard: MutexGuard<'a, T>, which: Which) -> Recorded<'a, T> {
        COUNTS.with(|counts| {
            let mut n = counts.get();
            n[which as usize] += 1;
            counts.set(n);
        });
        Recorded { guard, which }
    }
}

impl<T> Deref for Recorded<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.guard
    }
}

impl<T> DerefMut for Recorded<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.guard
    }
}

impl<T> Drop for Recorded<'_, T> {
    fn drop(&mut self) {
        COUNTS.with(|counts| {
            let mut n = counts.get();
            n[self.which as usize] -= 1;
            counts.set(n);
        });
    }
}
