//! The watch on the client's input (EA-15, issue #89): the disconnect signals (KC-36) of the
//! Runs the server is running, called when its input ends.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

type Signal = Box<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct Signals {
    fired: bool,
    next: u64,
    held: Vec<(u64, Signal)>,
}

/// A sticky list of disconnect signals: `fire` calls each once, and a signal added after
/// it is called at once. Signals are called in the order they were added, under the
/// watch's lock, so a removal waits for a fire in progress (N1: a Run's last reference,
/// which a signal holds while it runs, is never dropped on the thread that fires).
#[derive(Default)]
pub struct Watch {
    signals: Mutex<Signals>,
    /// The server's tests only: the removals that found the lock held.
    #[cfg(feature = "test-support")]
    contended: std::sync::atomic::AtomicUsize,
}

/// A signal held by a watch until this is dropped, or handed to [`unwatched`].
pub struct Watched {
    watch: Arc<Watch>,
    id: u64,
}

impl Watch {
    fn lock(&self) -> MutexGuard<'_, Signals> {
        self.signals.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Calls every signal held, once: the input has ended.
    pub fn fire(&self) {
        let mut signals = self.lock();
        if signals.fired {
            return;
        }
        signals.fired = true;
        for (_, signal) in &signals.held {
            signal();
        }
    }

    /// Whether the input has ended.
    pub fn fired(&self) -> bool {
        self.lock().fired
    }

    /// Holds `signal` until [`unwatched`] removes it; calls it at once if the input has ended.
    pub fn add(self: &Arc<Self>, signal: impl Fn() + Send + Sync + 'static) -> Watched {
        let mut signals = self.lock();
        if signals.fired {
            signal();
        }
        let id = signals.next;
        signals.next += 1;
        signals.held.push((id, Box::new(signal)));
        Watched { watch: self.clone(), id }
    }

    fn remove(&self, id: u64) {
        #[cfg(feature = "test-support")]
        if let Err(std::sync::TryLockError::WouldBlock) = self.signals.try_lock() {
            self.contended.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        self.lock().held.retain(|(held, _)| *held != id);
    }

    /// The server's tests only: the number of signals held.
    #[cfg(feature = "test-support")]
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.lock().held.len()
    }

    /// The server's tests only: how many removals found the lock held, by a fire in progress.
    #[cfg(feature = "test-support")]
    pub fn contended(&self) -> usize {
        self.contended.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Drop for Watched {
    /// A signal never outlives its holder: a panic after `add` leaves none behind.
    fn drop(&mut self) {
        self.watch.remove(self.id);
    }
}

/// N1: removes `watched`'s signal, waiting for a fire in progress, and only then hands
/// back `then`, the Run (or the child) the caller finishes.
pub fn unwatched<T>(watched: Option<Watched>, then: T) -> T {
    drop(watched);
    then
}
