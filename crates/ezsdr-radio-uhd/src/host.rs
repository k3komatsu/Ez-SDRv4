//! The host clock a [`Device`](crate::Device) hands its users (UR-2, UR-33): the real host
//! clock by default, or a manual clock the smoke set uses, whose time stops while one of its
//! participants (`uhd-rx`, `uhd-tx`, `uhd-control`, `uhd-clock`) is busy (M1–M7), together
//! with the clock lock (§4.7) a shared lock held across a device call becomes.
//!
//! The rules (maintenance plan 30 §1):
//!
//! - **M1** A participant is a thread the uhd crate itself starts; [`HostClock::spawn`]
//!   registers it before the thread runs and a guard unregisters it when the thread ends,
//!   a panic included. A start that fails registers nothing.
//! - **M2** A participant is idle only while it waits on the clock — a sleep, a clock
//!   condition wait, or the clock lock — and busy otherwise, a standard mutex included.
//! - **M3** While any participant is busy the clock stops.
//! - **M4** When a waiting participant's deadline arrives the clock stops there until that
//!   participant runs; one woken by a notify or handed the clock lock is busy at that
//!   instant too.
//! - **M5** While every participant is idle the clock advances no faster than real time; it
//!   never takes back the time it lost while busy.
//! - **M6** A non-participant thread waits on the same clock but its wait stops nothing.
//! - **M7** `sleep(d)` ends only once the clock has advanced by `d`; no notify or spurious
//!   wake ends it early.
//!
//! How (plan 30 §4.1). One mutex holds a manual clock's state, and every transition reads
//! the real instant after taking it (M5). A notify and a lock hand-off make the participant
//! they wake busy inside that mutex (M4), so the clock does not run between the wake and the
//! woken thread running. Each blocked wait has a condition variable of its own and is woken
//! only when its condition can have changed: a plain wait by a notify, a lock waiter by its
//! hand-off, and a wait that found the clock stopped by the clock advancing again. While the
//! clock advances, a manual deadline is waited for as its remaining clock time in real time,
//! which the clock cannot beat (M5); while it is stopped the wait blocks with no timeout, so
//! a stopped clock costs no CPU.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

thread_local! {
    /// The participant id of the running thread, `None` for a non-participant (M6).
    static PARTICIPANT: Cell<Option<u64>> = const { Cell::new(None) };
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Whether a [`FakeDevice`](crate::FakeDevice)'s host clock is the real one or a manual one
/// (UR-33); the default is [`ClockMode::Real`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClockMode {
    /// The real host clock.
    Real,
    /// A manual clock: [`HostClock::manual`].
    Manual,
}

/// The host clock a Device hands its users (UR-2). Cheap to clone: the state is shared.
#[derive(Clone)]
pub struct HostClock {
    inner: Arc<Inner>,
}

struct Inner {
    /// The real origin: a real clock's time runs from it.
    origin: Instant,
    /// A real clock's notify generation and the condition its waits block on.
    signal: (Mutex<u64>, Condvar),
    /// A manual clock's state; `None` for the real clock.
    manual: Option<Mutex<State>>,
    /// Keys for the clock locks.
    next_lock: AtomicU64,
}

struct State {
    /// The clock time committed at `last`.
    vnow: Duration,
    /// The real instant `vnow` was committed at; the clock runs from here while it advances.
    last: Instant,
    /// The running participants (M1, M3).
    busy: BTreeSet<u64>,
    next_id: u64,
    /// Each waiting participant's manual deadline (M4), and the earliest of them.
    deadlines: BTreeMap<u64, Duration>,
    earliest: Option<Duration>,
    /// Every blocked clock wait, the lock waiters aside, by key.
    waits: BTreeMap<u64, Wait>,
    next_wait: u64,
    /// The clock locks (§4.7), by key.
    locks: BTreeMap<u64, Lock>,
    next_token: u64,
    /// Bumped by every [`HostClock::notify_all`]: a plain wait ends on it, a sleep does not
    /// (M7).
    notified: u64,
}

/// A blocked clock wait.
struct Wait {
    cond: Arc<Condvar>,
    participant: Option<u64>,
    /// A plain wait ([`HostClock::wait`]) ends on a notify; a sleep does not (M7).
    plain: bool,
    /// It found the clock stopped short of its manual deadline, and blocks with no timeout
    /// until the clock advances again.
    stalled: bool,
}

/// A clock lock: its owner's token, and its waiters in arrival order.
#[derive(Default)]
struct Lock {
    owner: Option<u64>,
    waiters: VecDeque<(u64, Option<u64>, Arc<Condvar>)>,
}

/// The clock time at `real`: frozen while a participant is busy (M3), else run from `last`
/// at real speed (M5) and capped at the earliest waiting participant's deadline (M4).
fn now_locked(st: &State, real: Instant) -> Duration {
    if !st.busy.is_empty() {
        return st.vnow;
    }
    let t = st.vnow + real.saturating_duration_since(st.last);
    match st.earliest {
        Some(d) if t > d => d,
        _ => t,
    }
}

/// Whether the clock advances at `real`: no participant busy (M3) and the clock not standing
/// at a waiting participant's deadline (M4).
fn advancing(st: &State, real: Instant) -> bool {
    st.busy.is_empty() && st.earliest.is_none_or(|d| now_locked(st, real) < d)
}

/// Commits the clock time at `real` into `vnow`.
fn commit(st: &mut State, real: Instant) {
    st.vnow = now_locked(st, real);
    st.last = real;
}

/// Makes participant `id` busy at `real`: the clock stops there (M3).
fn go_busy(st: &mut State, real: Instant, id: u64) {
    if st.busy.is_empty() {
        commit(st, real);
    }
    st.busy.insert(id);
}

/// Makes participant `id` idle at `real`. If it was the last busy one, the clock runs on from
/// where it stopped (M5), and the waits stalled on it are woken — only they.
fn go_idle(st: &mut State, real: Instant, id: u64) {
    if st.busy.remove(&id) && st.busy.is_empty() {
        st.last = real;
        if advancing(st, real) {
            for wait in st.waits.values_mut().filter(|wait| wait.stalled) {
                wait.stalled = false;
                wait.cond.notify_one();
            }
        }
    }
}

/// Records `id`'s deadline and refreshes the earliest (M4).
fn set_deadline(st: &mut State, id: u64, deadline: Duration) {
    st.deadlines.insert(id, deadline);
    st.earliest = st.deadlines.values().min().copied();
}

/// Clears `id`'s deadline and refreshes the earliest.
fn clear_deadline(st: &mut State, id: u64) {
    st.deadlines.remove(&id);
    st.earliest = st.deadlines.values().min().copied();
}

/// Whether a wait for the clock time `manual`, the real instant `real` or a notify since
/// `seen` is over at `now`.
fn ended(st: &State, now: Instant, manual: Option<Duration>, real: Option<Instant>, seen: Option<u64>) -> bool {
    seen.is_some_and(|seen| st.notified != seen) || manual.is_some_and(|d| now_locked(st, now) >= d) || real.is_some_and(|r| now >= r)
}

/// Takes a manual clock's state mutex.
fn lock_state(m: &Mutex<State>) -> MutexGuard<'_, State> {
    #[cfg(test)]
    hooks::before_lock();
    lock(m)
}

/// Takes a manual clock's state mutex and reads the real instant **after** it (M5): a thread
/// delayed before the mutex must not commit an instant read before the delay, which would
/// hand the clock back the time it was stopped meanwhile.
fn state(m: &Mutex<State>) -> (MutexGuard<'_, State>, Instant) {
    let st = lock_state(m);
    (st, Instant::now())
}

/// Blocks the wait `key` until it has [`ended`]. While the clock advances, its manual
/// deadline is waited for as the remaining clock time in real time — the clock cannot beat
/// real time (M5), so the wait never wakes after the clock reached it — and while the clock
/// is stopped the wait is stalled: it blocks with no timeout (but its real deadline's) until
/// [`go_idle`] finds the clock advancing again.
fn block<'a>(m: &'a Mutex<State>, mut st: MutexGuard<'a, State>, key: u64, cond: &Condvar, manual: Option<Duration>, real: Option<Instant>, seen: Option<u64>) -> MutexGuard<'a, State> {
    #[cfg(not(test))]
    let _ = m;
    loop {
        let now = Instant::now();
        if ended(&st, now, manual, real, seen) {
            #[cfg(test)]
            {
                st = hooks::paused(m, st);
            }
            return st;
        }
        let running = advancing(&st, now);
        st.waits.get_mut(&key).expect("a registered wait").stalled = manual.is_some() && !running;
        let remaining = manual.filter(|_| running).map(|d| d - now_locked(&st, now));
        st = match remaining.into_iter().chain(real.map(|r| r - now)).min() {
            Some(timeout) => cond.wait_timeout(st, timeout).unwrap_or_else(|e| e.into_inner()).0,
            None => cond.wait(st).unwrap_or_else(|e| e.into_inner()),
        };
        #[cfg(test)]
        hooks::pass();
    }
}

/// The wait [`HostClock::sleep`] and [`HostClock::wait`] share, entered with the state locked
/// at `now`: a participant is idle while it lasts (M2), its manual deadline capping the clock
/// (M4), and busy again when it returns.
fn park<'a>(m: &'a Mutex<State>, mut st: MutexGuard<'a, State>, now: Instant, manual: Option<Duration>, real: Option<Instant>, seen: Option<u64>) {
    #[cfg(test)]
    hooks::pass();
    if ended(&st, now, manual, real, seen) {
        return;
    }
    let me = PARTICIPANT.with(Cell::get);
    let key = st.next_wait;
    st.next_wait += 1;
    let cond = Arc::new(Condvar::new());
    st.waits.insert(key, Wait { cond: cond.clone(), participant: me, plain: seen.is_some(), stalled: false });
    if let Some(id) = me {
        if let Some(deadline) = manual {
            set_deadline(&mut st, id, deadline);
        }
        go_idle(&mut st, now, id);
    }
    st = block(m, st, key, &cond, manual, real, seen);
    st.waits.remove(&key);
    if let Some(id) = me {
        // Busy before the deadline is cleared, so the clock keeps its stop there (M4).
        go_busy(&mut st, Instant::now(), id);
        clear_deadline(&mut st, id);
    }
}

impl HostClock {
    /// The real host clock.
    pub fn real() -> HostClock {
        HostClock::new(None)
    }

    /// A manual clock (M1–M7). It advances at real speed only while every participant is
    /// idle, and never runs faster than real time.
    pub fn manual() -> HostClock {
        HostClock::new(Some(Mutex::new(State {
            vnow: Duration::ZERO,
            last: Instant::now(),
            busy: BTreeSet::new(),
            next_id: 0,
            deadlines: BTreeMap::new(),
            earliest: None,
            waits: BTreeMap::new(),
            next_wait: 0,
            locks: BTreeMap::new(),
            next_token: 0,
            notified: 0,
        })))
    }

    fn new(manual: Option<Mutex<State>>) -> HostClock {
        HostClock {
            inner: Arc::new(Inner {
                origin: Instant::now(),
                signal: (Mutex::new(0), Condvar::new()),
                manual,
                next_lock: AtomicU64::new(0),
            }),
        }
    }

    /// Whether this is a manual clock.
    pub fn is_manual(&self) -> bool {
        self.inner.manual.is_some()
    }

    /// The clock time since the clock's origin.
    pub fn now(&self) -> Duration {
        match &self.inner.manual {
            None => self.inner.origin.elapsed(),
            Some(m) => {
                let (st, now) = state(m);
                now_locked(&st, now)
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn advance_for_test(&self, by: Duration) {
        let manual = self.inner.manual.as_ref().expect("test clock is manual");
        let real = Instant::now();
        let mut st = lock_state(manual);
        assert!(st.busy.is_empty() && st.earliest.is_none(), "test clock has active participants");
        commit(&mut st, real);
        st.vnow = st.vnow.saturating_add(by);
        st.last = real;
    }

    /// Sleeps `d` of clock time (M7). A participant is idle while it waits (M2); a
    /// non-participant waits on the same clock without stopping it (M6).
    pub fn sleep(&self, d: Duration) {
        let Some(m) = &self.inner.manual else {
            std::thread::sleep(d);
            return;
        };
        let (st, now) = state(m);
        let deadline = now_locked(&st, now) + d;
        park(m, st, now, Some(deadline), None, None);
    }

    /// The notify generation, read **before** the predicate [`Self::wait`] is to wait on: a
    /// notify after this reading ends the wait at once, so none is lost between the caller's
    /// check and the wait.
    pub(crate) fn generation(&self) -> u64 {
        match &self.inner.manual {
            None => *lock(&self.inner.signal.0),
            Some(m) => lock_state(m).notified,
        }
    }

    /// Waits until the clock reaches `manual`, real time reaches `real`, or a
    /// [`Self::notify_all`] since `seen` (read by [`Self::generation`]): whichever is first.
    /// While a manual clock is stopped the wait blocks: only the real deadline or a notify can
    /// end it then. A participant's real deadline does not stop the clock; the
    /// `host.monotonic` deadlines the Authority waits for are its non-participant callers'.
    pub fn wait(&self, manual: Option<Duration>, real: Option<Instant>, seen: u64) {
        let Some(m) = &self.inner.manual else {
            return self.wait_real(manual, real, seen);
        };
        let (st, now) = state(m);
        park(m, st, now, manual, real, Some(seen));
    }

    /// [`Self::wait`] on the real clock, whose time is real time since its origin.
    fn wait_real(&self, manual: Option<Duration>, real: Option<Instant>, seen: u64) {
        let until = manual.map(|d| self.inner.origin + d).into_iter().chain(real).min();
        let (generation, cond) = &self.inner.signal;
        let mut current = lock(generation);
        while *current == seen {
            current = match until {
                None => cond.wait(current).unwrap_or_else(|e| e.into_inner()),
                Some(until) => {
                    let now = Instant::now();
                    if now >= until {
                        return;
                    }
                    cond.wait_timeout(current, until - now).unwrap_or_else(|e| e.into_inner()).0
                }
            };
        }
    }

    /// Ends every plain wait at once; a participant it wakes is busy from this instant (M4),
    /// so the clock does not run before that participant does. A sleep is bounded by its
    /// deadline alone (M7).
    pub fn notify_all(&self) {
        let Some(m) = &self.inner.manual else {
            let mut generation = lock(&self.inner.signal.0);
            *generation = generation.wrapping_add(1);
            self.inner.signal.1.notify_all();
            return;
        };
        let (mut st, now) = state(m);
        st.notified = st.notified.wrapping_add(1);
        let woken: Vec<_> = st.waits.values().filter(|wait| wait.plain).map(|wait| (wait.participant, wait.cond.clone())).collect();
        for (participant, cond) in woken {
            if let Some(id) = participant {
                go_busy(&mut st, now, id);
            }
            cond.notify_one();
        }
    }

    /// Starts a named thread, registered as a participant before it runs (M1). Until the
    /// thread owns the registration a guard rolls it back, on an `Err` and on a panic (a name
    /// with a NUL) alike.
    pub(crate) fn spawn(&self, name: &str, f: impl FnOnce() + Send + 'static) -> std::io::Result<JoinHandle<()>> {
        let mut prestart = PreStart { clock: self.clone(), id: self.register() };
        let (clock, id) = (self.clone(), prestart.id);
        let handle = std::thread::Builder::new().name(name.to_owned()).spawn(move || {
            let _guard = id.map(|id| ParticipantGuard::new(clock, id));
            f();
        })?;
        prestart.id = None;
        Ok(handle)
    }

    /// Registers a running participant (M1, M3): the clock stops while it is busy.
    fn register(&self) -> Option<u64> {
        let m = self.inner.manual.as_ref()?;
        let (mut st, now) = state(m);
        let id = st.next_id;
        st.next_id += 1;
        go_busy(&mut st, now, id);
        Some(id)
    }

    /// Unregisters `id` (M1): a thread that ends, a panic included, stops being busy.
    fn unregister(&self, id: u64) {
        let Some(m) = &self.inner.manual else { return };
        let (mut st, now) = state(m);
        clear_deadline(&mut st, id);
        go_idle(&mut st, now, id);
    }

    /// A clock lock (§4.7) over `value`.
    pub(crate) fn clock_mutex<T>(&self, value: T) -> ClockMutex<T> {
        let key = self.inner.next_lock.fetch_add(1, Ordering::Relaxed);
        ClockMutex {
            clock: self.clone(),
            key,
            data: Mutex::new(value),
            #[cfg(test)]
            waiting: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Acquires the clock lock `key`; `None` on the real clock, whose plain mutex is enough.
    fn acquire_lock(&self, key: u64) -> Option<u64> {
        let m = self.inner.manual.as_ref()?;
        let participant = PARTICIPANT.with(Cell::get);
        let (mut st, now) = state(m);
        let token = st.next_token;
        st.next_token += 1;
        let lock = st.locks.entry(key).or_default();
        if lock.owner.is_none() {
            lock.owner = Some(token);
            return Some(token);
        }
        let cond = Arc::new(Condvar::new());
        lock.waiters.push_back((token, participant, cond.clone()));
        // Idle while it waits for the hand-off (M2), which makes it busy (M4).
        if let Some(id) = participant {
            go_idle(&mut st, now, id);
        }
        while st.locks.get(&key).is_none_or(|lock| lock.owner != Some(token)) {
            st = cond.wait(st).unwrap_or_else(|e| e.into_inner());
            #[cfg(test)]
            hooks::pass();
        }
        #[cfg(test)]
        drop(hooks::paused(m, st));
        Some(token)
    }

    /// Releases the clock lock `key` held by `token`, handing it to the next waiter (§4.7).
    fn release_lock(&self, key: u64, token: u64) {
        let Some(m) = &self.inner.manual else { return };
        let (mut st, now) = state(m);
        debug_assert_eq!(st.locks.get(&key).and_then(|lock| lock.owner), Some(token));
        match st.locks.get_mut(&key).and_then(|lock| lock.waiters.pop_front()) {
            Some((next, participant, cond)) => {
                st.locks.get_mut(&key).expect("a held lock").owner = Some(next);
                // M4: busy at the hand-off, so the clock does not run before the waiter does.
                if let Some(id) = participant {
                    go_busy(&mut st, now, id);
                }
                cond.notify_one();
            }
            None => {
                st.locks.remove(&key);
            }
        }
    }

    /// How many waits block on this clock, its lock waiters included: a test's handshake that
    /// a thread has entered its wait.
    #[cfg(test)]
    pub(crate) fn blocked(&self) -> usize {
        self.inner.manual.as_ref().map_or(0, |m| {
            let st = lock(m);
            st.waits.len() + st.locks.values().map(|lock| lock.waiters.len()).sum::<usize>()
        })
    }
}

/// A [`HostClock::spawn`]'s registration until its thread owns it: rolled back on drop (M1).
struct PreStart {
    clock: HostClock,
    id: Option<u64>,
}

impl Drop for PreStart {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            self.clock.unregister(id);
        }
    }
}

/// Sets the participant id of the spawned thread and clears it when the thread ends (M1).
struct ParticipantGuard {
    clock: HostClock,
    id: u64,
}

impl ParticipantGuard {
    fn new(clock: HostClock, id: u64) -> ParticipantGuard {
        PARTICIPANT.with(|c| c.set(Some(id)));
        ParticipantGuard { clock, id }
    }
}

impl Drop for ParticipantGuard {
    fn drop(&mut self) {
        self.clock.unregister(self.id);
        PARTICIPANT.with(|c| c.set(None));
    }
}

/// A lock that is the clock lock (§4.7) on a manual clock and a plain mutex on the real one.
pub(crate) struct ClockMutex<T> {
    clock: HostClock,
    key: u64,
    data: Mutex<T>,
    /// Test only: the threads inside `lock`, waiting for it or just through it.
    #[cfg(test)]
    waiting: std::sync::atomic::AtomicUsize,
}

impl<T> ClockMutex<T> {
    /// Acquires the lock, waiting on the clock when another participant holds it (§4.7).
    pub(crate) fn lock(&self) -> ClockGuard<'_, T> {
        #[cfg(test)]
        self.waiting.fetch_add(1, Ordering::SeqCst);
        let token = self.clock.acquire_lock(self.key);
        let data = lock(&self.data);
        #[cfg(test)]
        self.waiting.fetch_sub(1, Ordering::SeqCst);
        ClockGuard { clock: self.clock.clone(), key: self.key, token, data: Some(data) }
    }

    /// Test only: whether some thread holds the lock now (a `try_lock` that fails).
    #[cfg(test)]
    pub(crate) fn is_held(&self) -> bool {
        self.data.try_lock().is_err()
    }

    /// Test only: how many threads are inside `lock` without having it yet: the handshake
    /// that a thread has reached the lock and is held there by its holder.
    #[cfg(test)]
    pub(crate) fn waiters(&self) -> usize {
        self.waiting.load(Ordering::SeqCst)
    }
}

/// The guard of a [`ClockMutex`].
pub(crate) struct ClockGuard<'a, T> {
    clock: HostClock,
    key: u64,
    token: Option<u64>,
    data: Option<MutexGuard<'a, T>>,
}

impl<T> std::ops::Deref for ClockGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.data.as_ref().expect("a held guard")
    }
}

impl<T> std::ops::DerefMut for ClockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.data.as_mut().expect("a held guard")
    }
}

impl<T> Drop for ClockGuard<'_, T> {
    fn drop(&mut self) {
        self.data.take();
        if let Some(token) = self.token.take() {
            self.clock.release_lock(self.key, token);
        }
    }
}

/// Test hooks, per thread so that a test chooses the participant they act on. The two pauses
/// are handshakes: the paused thread signals and then waits for the test to release it, so a
/// test makes its checks while the thread is held, with no fixed delay to outlast.
#[cfg(test)]
mod hooks {
    use super::*;
    use std::sync::mpsc::{Receiver, Sender};

    type Pause = Cell<Option<(Sender<()>, Receiver<()>)>>;

    thread_local! {
        static BEFORE_LOCK: Pause = const { Cell::new(None) };
        static AFTER_WAKE: Pause = const { Cell::new(None) };
        static PASSES: Cell<u64> = const { Cell::new(0) };
    }

    /// This thread's next acquisition of a manual clock's state mutex first sends on `reached`
    /// and blocks until `release` receives (or its sender is dropped): a thread delayed before
    /// the mutex for as long as the test holds it.
    pub(crate) fn pause_next_state_lock(reached: Sender<()>, release: Receiver<()>) {
        BEFORE_LOCK.set(Some((reached, release)));
    }

    /// This thread's next clock wait, once over, releases the mutex, sends on `woken` and blocks
    /// until `release` receives (or its sender is dropped) before it goes on: a waiter
    /// descheduled between its wake and its running, held until the test's checks are done.
    pub(crate) fn pause_next_wake(woken: Sender<()>, release: Receiver<()>) {
        AFTER_WAKE.set(Some((woken, release)));
    }

    /// How many times this thread has entered a clock sleep or wait, or woken in one or in a
    /// lock wait: what a wake storm or a polling wait is made of. std has no CPU clock, and
    /// process CPU time would count the other tests of the binary.
    pub(crate) fn passes() -> u64 {
        PASSES.get()
    }

    pub(super) fn before_lock() {
        if let Some((reached, release)) = BEFORE_LOCK.take() {
            let _ = reached.send(());
            let _ = release.recv();
        }
    }

    pub(super) fn pass() {
        PASSES.set(PASSES.get() + 1);
    }

    pub(super) fn paused<'a>(m: &'a Mutex<State>, st: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
        let Some((woken, release)) = AFTER_WAKE.take() else { return st };
        drop(st);
        let _ = woken.send(());
        let _ = release.recv();
        lock(m)
    }
}

#[cfg(test)]
pub(crate) use hooks::{passes, pause_next_state_lock, pause_next_wake};

#[cfg(test)]
mod tests {
    //! One test per rule (plan 30 §7), each with a wall-clock bound on everything it waits
    //! for, so a clock that freezes or deadlocks fails the test instead of hanging the suite.
    //! A rule about the instant a waiter runs holds the waiter with [`pause_next_wake`] — a
    //! handshake, released only after the test's checks — and a wall interval in a check
    //! only scales what a wrong clock does, never what a right one must.

    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
    use std::sync::mpsc::{channel, Receiver, Sender};

    const BOUND: Duration = Duration::from_secs(5);

    fn participant(clock: &HostClock, f: impl FnOnce() + Send + 'static) -> JoinHandle<()> {
        clock.spawn("test-participant", f).unwrap()
    }

    /// A value a freezing or deadlocking clock would never send.
    fn bounded<T>(rx: &Receiver<T>, what: &str) -> T {
        rx.recv_timeout(BOUND).unwrap_or_else(|_| panic!("{what} within {BOUND:?}"))
    }

    /// An inline call a freezing clock would never return from, run on its own thread.
    fn within<T: Send + 'static>(what: &str, f: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        bounded(&rx, what)
    }

    /// Joins `handle`, failing if the thread has not ended within the bound.
    fn joined(handle: JoinHandle<()>, what: &str) -> std::thread::Result<()> {
        let deadline = Instant::now() + BOUND;
        while !handle.is_finished() {
            assert!(Instant::now() < deadline, "{what} ended within {BOUND:?}");
            std::thread::sleep(Duration::from_millis(1));
        }
        handle.join()
    }

    /// Returns once `n` waits block on `clock`: the threads have entered them.
    fn until_blocked(clock: &HostClock, n: usize, what: &str) {
        let deadline = Instant::now() + BOUND;
        while clock.blocked() < n {
            assert!(Instant::now() < deadline, "{what} within {BOUND:?}");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// A pause handshake ([`pause_next_wake`], [`pause_next_state_lock`]): the paused thread's
    /// ends, then the test's — `signalled`, and `release`, whose drop on a failing test's
    /// unwind releases too.
    #[allow(clippy::type_complexity)]
    fn pause() -> ((Sender<()>, Receiver<()>), (Receiver<()>, Sender<()>)) {
        let (signal, signalled) = channel();
        let (release, released) = channel();
        ((signal, released), (signalled, release))
    }

    /// A manual clock's state, for the checks made on it directly.
    fn state_of(clock: &HostClock) -> MutexGuard<'_, State> {
        lock(clock.inner.manual.as_ref().expect("a manual clock"))
    }

    /// The clock's time, asserting that it stands still. The interval only scales a wrong
    /// clock's movement, which an exact comparison sees however small: it bounds nothing.
    fn still(clock: &HostClock, what: &str) -> Duration {
        let at = clock.now();
        std::thread::sleep(20 * MS);
        assert_eq!(clock.now(), at, "{what}");
        at
    }

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn m1_a_participant_registers_before_it_runs_and_unregisters_on_a_panic() {
        let clock = HostClock::manual();
        let (go, gate) = channel::<()>();
        let (ran, rx) = channel();
        let handle = participant(&clock, move || {
            let _ = gate.recv();
            let _ = ran.send(());
            panic!("a participant panics");
        });
        // Registered before it runs: the clock stands still from `spawn`'s return.
        let spawned = clock.now();
        std::thread::sleep(30 * MS);
        assert_eq!(clock.now(), spawned, "the clock ran before the participant registered");
        let _ = go.send(());
        bounded(&rx, "the participant ran");
        assert!(joined(handle, "the participant").is_err(), "it panicked");
        // Unregistered by its guard on the panic: the clock runs again.
        let resumed = clock.now();
        std::thread::sleep(40 * MS);
        assert!(clock.now() >= resumed + 20 * MS, "the clock stayed stopped after the panic: {:?} from {resumed:?}", clock.now());
    }

    #[test]
    fn m1_a_thread_that_never_starts_leaves_no_registration() {
        // `Builder::spawn` panics on a name with a NUL; the registration made before it is
        // rolled back, or the clock would stay stopped for ever.
        let clock = HostClock::manual();
        let refused = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| clock.spawn("a\0b", || {})));
        assert!(refused.is_err(), "Builder::spawn panics on a NUL in the name");
        let from = clock.now();
        std::thread::sleep(40 * MS);
        assert!(clock.now() >= from + 20 * MS, "the refused start left the clock stopped at {:?}", clock.now());
    }

    #[test]
    fn m2_m3_a_busy_participant_stops_the_clock() {
        let clock = HostClock::manual();
        let (ran, rx) = channel();
        let (release, released) = channel::<()>();
        let handle = participant(&clock, move || {
            let _ = ran.send(());
            // A std wait is busy (M2).
            let _ = released.recv();
        });
        bounded(&rx, "the participant ran");
        let frozen = clock.now();
        std::thread::sleep(40 * MS);
        assert_eq!(clock.now(), frozen, "the clock advanced while a participant was busy");
        let _ = release.send(());
        joined(handle, "the participant").unwrap();
    }

    #[test]
    fn m4_a_deadline_stops_the_clock_until_the_waiter_runs() {
        // The sleeper is held between its deadline's arrival and its running: the clock waits
        // for it there, however long it is held.
        let clock = Arc::new(HostClock::manual());
        let ((woken_tx, release_rx), (woken, release)) = pause();
        let (started, started_rx) = channel();
        let (done, finished) = channel();
        let c = clock.clone();
        let handle = participant(&clock, move || {
            let begun = c.now();
            let _ = started.send(begun);
            pause_next_wake(woken_tx, release_rx);
            c.sleep(30 * MS);
            let _ = done.send(c.now() - begun);
        });
        let begun = bounded(&started_rx, "the sleeper started");
        bounded(&woken, "the sleep reached its deadline");
        assert_eq!(still(&clock, "the clock ran on past the deadline before its sleeper ran"), begun + 30 * MS);
        let _ = release.send(());
        assert_eq!(bounded(&finished, "the sleeper ran"), 30 * MS);
        joined(handle, "the sleeper").unwrap();
    }

    #[test]
    fn m4_two_sleepers_stop_the_clock_at_the_earlier_deadline_and_go_on_to_the_later() {
        // More than one wait with a deadline: the clock stops at the earliest — where the
        // earlier sleeper, held after its deadline, finds it — and goes on to the later one.
        let clock = Arc::new(HostClock::manual());
        let ((woken_tx, release_rx), (woken, release)) = pause();
        let (go, gate) = channel::<()>();
        let (entered, entered_rx) = channel();
        let (done, finished) = channel();
        // A busy holder keeps the clock stopped while both record their deadlines, then goes
        // idle in a plain wait: ending would clear deadlines and recompute the earliest.
        let holder = {
            let c = clock.clone();
            participant(&clock, move || {
                let _ = gate.recv();
                let seen = c.generation();
                c.wait(None, None, seen);
            })
        };
        let sleeper = |name: &'static str, d: Duration, held: Option<(Sender<()>, Receiver<()>)>| {
            let (c, entered, done) = (clock.clone(), entered.clone(), done.clone());
            participant(&clock, move || {
                let begun = c.now();
                let _ = entered.send(begun);
                if let Some((woken, release)) = held {
                    pause_next_wake(woken, release);
                }
                c.sleep(d);
                let _ = done.send((name, c.now() - begun));
            })
        };
        let earlier = sleeper("earlier", 30 * MS, Some((woken_tx, release_rx)));
        let later = sleeper("later", 40 * MS, None);
        let begun = bounded(&entered_rx, "a sleeper started");
        assert_eq!(bounded(&entered_rx, "the other sleeper started"), begun);
        until_blocked(&clock, 2, "both sleepers entered their sleeps");
        let _ = go.send(());
        bounded(&woken, "the earlier sleep reached its deadline");
        assert_eq!(still(&clock, "the clock ran on past the earlier deadline"), begun + 30 * MS, "the clock stopped at the wrong deadline");
        let _ = release.send(());
        // In this order: the later one cannot wake before the earlier one has run.
        let got = [bounded(&finished, "a sleeper ran"), bounded(&finished, "the other sleeper ran")];
        assert_eq!(got, [("earlier", 30 * MS), ("later", 40 * MS)]);
        clock.notify_all();
        for handle in [holder, earlier, later] {
            joined(handle, "a participant").unwrap();
        }
    }

    #[test]
    fn m4_two_sleepers_with_one_deadline_stop_the_clock_there_until_both_run() {
        // Two waits with one deadline: the first to run must not lift the second's stop. The
        // first goes on into a plain wait (idle) while the second is held after the deadline.
        let clock = Arc::new(HostClock::manual());
        let ((woken_tx, release_rx), (woken, release)) = pause();
        let (go, gate) = channel::<()>();
        let (entered, entered_rx) = channel();
        let (done, finished) = channel();
        let holder = participant(&clock, move || {
            let _ = gate.recv();
        });
        // The first goes idle in a plain wait after it runs; the second just ends, since a
        // generation it read after reporting could already be the test's final notify's.
        let sleeper = |n: u8, held: Option<(Sender<()>, Receiver<()>)>| {
            let (c, entered, done) = (clock.clone(), entered.clone(), done.clone());
            participant(&clock, move || {
                let begun = c.now();
                let _ = entered.send(begun);
                let first = held.is_none();
                if let Some((woken, release)) = held {
                    pause_next_wake(woken, release);
                }
                c.sleep(40 * MS);
                let _ = done.send((n, c.now() - begun));
                if first {
                    let seen = c.generation();
                    c.wait(None, None, seen);
                }
            })
        };
        let first = sleeper(1, None);
        let second = sleeper(2, Some((woken_tx, release_rx)));
        let begun = bounded(&entered_rx, "a sleeper started");
        assert_eq!(bounded(&entered_rx, "the other sleeper started"), begun);
        until_blocked(&clock, 2, "both sleepers entered their sleeps");
        let _ = go.send(());
        // The second is held, so the first is the one that runs.
        assert_eq!(bounded(&finished, "the first sleeper ran"), (1, 40 * MS));
        bounded(&woken, "the second sleep reached its deadline");
        until_blocked(&clock, 2, "the first sleeper went idle beside the held second");
        assert_eq!(still(&clock, "the first sleeper's run lifted the second's stop"), begun + 40 * MS);
        let _ = release.send(());
        assert_eq!(bounded(&finished, "the second sleeper ran"), (2, 40 * MS));
        clock.notify_all();
        for handle in [holder, first, second] {
            joined(handle, "a participant").unwrap();
        }
    }

    #[test]
    fn m4_a_notify_racing_the_deadline_does_not_move_the_clock_past_it() {
        // A plain wait reaches its deadline while its participant is held, and a notify arrives
        // before it runs: the clock stays at the deadline through the notify.
        let clock = Arc::new(HostClock::manual());
        let ((woken_tx, release_rx), (woken, release)) = pause();
        let (entered, entered_rx) = channel();
        let (done, finished) = channel();
        let c = clock.clone();
        let handle = participant(&clock, move || {
            let seen = c.generation();
            let begun = c.now();
            let _ = entered.send(begun);
            pause_next_wake(woken_tx, release_rx);
            c.wait(Some(begun + 40 * MS), None, seen);
            let _ = done.send(c.now() - begun);
        });
        let begun = bounded(&entered_rx, "the waiter started");
        bounded(&woken, "the wait reached its deadline");
        assert_eq!(still(&clock, "the clock ran on past the deadline before its waiter ran"), begun + 40 * MS);
        clock.notify_all();
        assert_eq!(clock.now(), begun + 40 * MS, "the notify moved the clock past the deadline");
        let _ = release.send(());
        assert_eq!(bounded(&finished, "the waiter ran"), 40 * MS);
        joined(handle, "the waiter").unwrap();
    }

    #[test]
    fn m5_the_clock_runs_at_real_speed_from_its_last_commit() {
        // M5 on the state itself (row J281): with no participant busy and no deadline, the time
        // at an instant is the committed time plus exactly the real time since `last`. Fixed
        // instants, so no thread's scheduling enters the check.
        let clock = HostClock::manual();
        let st = state_of(&clock);
        assert_eq!(now_locked(&st, st.last + 10 * MS), st.vnow + 10 * MS, "the clock runs at another speed than real time");
    }

    #[test]
    fn m5_a_participant_going_idle_restarts_the_clock_from_that_instant() {
        // M5 on the state itself (row J312): a participant busy from `begun` and idle at `idle`
        // leaves the clock where it stopped and running from `idle` — from `begun` it would take
        // the busy interval back.
        let clock = HostClock::manual();
        let mut st = state_of(&clock);
        let begun = st.last + 10 * MS;
        go_busy(&mut st, begun, 7);
        let stopped = st.vnow;
        let idle = begun + 50 * MS;
        go_idle(&mut st, idle, 7);
        assert_eq!(st.last, idle, "the clock runs on from another instant than the move to idle");
        assert_eq!(now_locked(&st, idle), stopped, "the clock took the busy interval back");
    }

    #[test]
    fn m5_the_clock_is_never_faster_than_real_time() {
        // A diagnostic of the whole path (the rule itself: the test above). A bracket, not a
        // bound: the clock, created after `begun` and read before `elapsed`, can only be behind
        // that real interval, however this thread is scheduled.
        let begun = Instant::now();
        let clock = Arc::new(HostClock::manual());
        within("the clock's 60 ms sleep", {
            let c = clock.clone();
            move || c.sleep(60 * MS)
        });
        let now = clock.now();
        let elapsed = begun.elapsed();
        assert!(now >= 60 * MS, "{now:?}");
        assert!(now <= elapsed, "faster than real time: {now:?} against {elapsed:?}");
    }

    #[test]
    fn m5_the_clock_does_not_catch_up_after_a_busy_interval() {
        // A diagnostic of the whole path (the rule itself: `m5_a_participant_going_idle_…`). The
        // clock stops while a participant is busy and never takes that time back: it runs on
        // only from the busy interval's end, at least 200 ms after `went`.
        let clock = Arc::new(HostClock::manual());
        let (go, gate) = channel::<()>();
        let busy = participant(&clock, move || {
            let _ = gate.recv();
            std::thread::sleep(200 * MS);
        });
        let frozen = clock.now();
        let went = Instant::now();
        let _ = go.send(());
        joined(busy, "the busy participant").unwrap();
        let ran = clock.now() - frozen;
        let since = went.elapsed().saturating_sub(200 * MS);
        assert!(ran <= since, "the clock caught up: it ran {ran:?} in the {since:?} since the busy interval");
    }

    #[test]
    fn m5_a_transition_delayed_before_the_state_mutex_does_not_make_the_clock_jump() {
        // The participant's move to idle is held 50 ms before it takes the state mutex, busy
        // all the while: the instant it commits is read after the mutex, so the clock's `last`
        // is no earlier than the test's release — two recorded instants, compared directly —
        // and it runs on from its stop no further than the real time since then.
        let clock = Arc::new(HostClock::manual());
        let ((reached_tx, release_rx), (reached, release)) = pause();
        let (entered, entered_rx) = channel();
        let c = clock.clone();
        let handle = participant(&clock, move || {
            let seen = c.generation();
            let _ = entered.send(c.now());
            pause_next_state_lock(reached_tx, release_rx);
            c.wait(None, None, seen);
        });
        let frozen = bounded(&entered_rx, "the participant ran");
        bounded(&reached, "the participant reached the state mutex");
        std::thread::sleep(50 * MS);
        let released = Instant::now();
        let _ = release.send(());
        until_blocked(&clock, 1, "the participant went idle");
        let last = state_of(&clock).last;
        assert!(last >= released, "the move to idle committed an instant from before the state mutex");
        let ran = clock.now() - frozen;
        let since = released.elapsed();
        assert!(ran <= since, "the clock counted the delay before the state mutex: it ran {ran:?} in {since:?}");
        clock.notify_all();
        joined(handle, "the participant").unwrap();
    }

    #[test]
    fn m6_a_non_participant_wait_stops_nothing() {
        // A non-participant held past its deadline does not hold the clock there — with no
        // participant at all, the clock is real time — and nothing it did leaves it stopped.
        let clock = Arc::new(HostClock::manual());
        let ((woken_tx, release_rx), (woken, release)) = pause();
        let (done, finished) = channel();
        let c = clock.clone();
        let handle = std::thread::spawn(move || {
            pause_next_wake(woken_tx, release_rx);
            let begun = c.now();
            c.sleep(20 * MS);
            let _ = done.send(c.now() - begun);
        });
        bounded(&woken, "the non-participant's sleep reached its deadline");
        let held = clock.now();
        std::thread::sleep(20 * MS);
        assert!(clock.now() >= held + 20 * MS, "a non-participant's wait held the clock at {held:?}");
        let _ = release.send(());
        assert!(bounded(&finished, "the non-participant's sleep") >= 20 * MS);
        joined(handle, "the non-participant").unwrap();
        let after = clock.now();
        std::thread::sleep(20 * MS);
        assert!(clock.now() >= after + 20 * MS, "the clock stopped after a non-participant's wait");
    }

    #[test]
    fn m7_a_notify_does_not_end_a_sleep_early() {
        // The notifies arrive while a busy holder keeps the clock still, so before the sleep's
        // deadline can: a sleep a notify ended would return with no time slept, whenever its
        // thread got to run.
        let clock = Arc::new(HostClock::manual());
        let (go, gate) = channel::<()>();
        let holder = participant(&clock, move || {
            let _ = gate.recv();
        });
        let (done, finished) = channel();
        let c = clock.clone();
        let handle = participant(&clock, move || {
            let begun = c.now();
            c.sleep(200 * MS);
            let _ = done.send(c.now() - begun);
        });
        until_blocked(&clock, 1, "the sleeper entered its sleep");
        for _ in 0..3 {
            clock.notify_all();
        }
        let _ = go.send(());
        assert_eq!(bounded(&finished, "the sleep reached its deadline"), 200 * MS, "a notify ended the sleep early");
        joined(handle, "the sleeper").unwrap();
        joined(holder, "the holder").unwrap();
    }

    #[test]
    fn notify_a_notified_participant_is_busy_until_it_runs() {
        // M4: with no other participant busy, the notify itself makes the waiter busy, so the
        // clock stands still while the woken waiter is held before it runs.
        let clock = Arc::new(HostClock::manual());
        let ((woken_tx, release_rx), (woken, release)) = pause();
        let (resumed, resumed_rx) = channel();
        let c = clock.clone();
        let waiter = participant(&clock, move || {
            let seen = c.generation();
            pause_next_wake(woken_tx, release_rx);
            c.wait(Some(c.now() + Duration::from_secs(30)), None, seen);
            let _ = resumed.send(c.now());
        });
        until_blocked(&clock, 1, "the waiter entered its wait");
        let before = clock.now();
        clock.notify_all();
        let at = clock.now();
        assert!(at >= before, "the notify moved the clock back: {before:?} then {at:?}");
        bounded(&woken, "the notify ended the wait");
        assert_eq!(still(&clock, "the clock ran while the notified participant was held"), at);
        let _ = release.send(());
        assert_eq!(bounded(&resumed_rx, "the notified waiter resumed"), at, "the clock moved before the waiter ran");
        joined(waiter, "the waiter").unwrap();
    }

    #[test]
    fn wait_a_notify_ends_a_plain_wait_while_a_participant_is_busy() {
        // A plain wait (the Authority's nap, uhd-clock's re-anchor) ends on a notify while
        // another participant holds the clock stopped, its deadline unreachable: the Authority's
        // drop relies on it (UR-7's D-1). It ends with the clock where the notify found it.
        let clock = Arc::new(HostClock::manual());
        let (release, released) = channel::<()>();
        let busy = participant(&clock, move || {
            let _ = released.recv();
        });
        let (done, finished) = channel();
        let c = clock.clone();
        let waiter = participant(&clock, move || {
            let seen = c.generation();
            c.wait(Some(c.now() + Duration::from_secs(10)), None, seen);
            let _ = done.send(c.now());
        });
        until_blocked(&clock, 1, "the waiter entered its wait");
        let frozen = clock.now();
        clock.notify_all();
        assert_eq!(bounded(&finished, "the notified plain wait ended while the clock was stopped"), frozen);
        let _ = release.send(());
        joined(busy, "the busy participant").unwrap();
        joined(waiter, "the waiter").unwrap();
    }

    #[test]
    fn hand_off_the_clock_does_not_advance_after_the_lock_hand_off() {
        // §4.7, M4: the hand-off makes the waiting participant busy, so the clock stands still
        // while it is held after the hand-off, the holder idle meanwhile.
        let clock = Arc::new(HostClock::manual());
        let mutex = Arc::new(clock.clock_mutex(()));
        let ((woken_tx, release_rx), (woken, release)) = pause();
        let (locked, locked_rx) = channel();
        let (go, gate) = channel::<()>();
        let (handed, handed_rx) = channel();
        let (took, took_rx) = channel();
        let holder = {
            let (m, c) = (mutex.clone(), clock.clone());
            participant(&clock, move || {
                let guard = m.lock();
                let _ = locked.send(());
                let _ = gate.recv();
                let at = c.now();
                drop(guard);
                let _ = handed.send(at);
                let seen = c.generation();
                c.wait(None, None, seen);
            })
        };
        bounded(&locked_rx, "the holder took the clock lock");
        let waiter = {
            let (m, c) = (mutex.clone(), clock.clone());
            participant(&clock, move || {
                pause_next_wake(woken_tx, release_rx);
                let _held = m.lock();
                let _ = took.send(c.now());
            })
        };
        until_blocked(&clock, 1, "the waiter queued for the lock");
        let _ = go.send(());
        let at = bounded(&handed_rx, "the hand-off");
        bounded(&woken, "the waiter was handed the lock");
        until_blocked(&clock, 1, "the holder entered its wait");
        assert_eq!(still(&clock, "the clock ran after the hand-off"), at);
        let _ = release.send(());
        assert_eq!(bounded(&took_rx, "the waiter took the lock"), at);
        clock.notify_all();
        joined(holder, "the holder").unwrap();
        joined(waiter, "the waiter").unwrap();
    }

    #[test]
    fn lock_a_waiter_waits_while_the_holder_sleeps_on_the_clock() {
        // §4.7: a participant waiting for the clock lock is idle, so the clock keeps running
        // while the holder — holding the lock across a device-like wait — sleeps on it. Were
        // the waiter busy, the clock would freeze and neither would finish.
        let clock = Arc::new(HostClock::manual());
        let mutex = Arc::new(clock.clock_mutex(0u32));
        let (locked, locked_rx) = channel();
        let (acquired, acquired_rx) = channel();
        let (go, gate) = channel::<()>();
        let holder = {
            let (m, c) = (mutex.clone(), clock.clone());
            participant(&clock, move || {
                let mut guard = m.lock();
                *guard += 1;
                let _ = locked.send(());
                let _ = gate.recv_timeout(BOUND);
                c.sleep(30 * MS);
            })
        };
        bounded(&locked_rx, "the holder took the clock lock");
        let waiter = {
            let m = mutex.clone();
            participant(&clock, move || {
                let guard = m.lock();
                let _ = acquired.send(*guard);
            })
        };
        until_blocked(&clock, 1, "the waiter queued for the lock");
        let _ = go.send(());
        assert_eq!(bounded(&acquired_rx, "the lock was handed over: the waiter stopped the clock"), 1);
        joined(holder, "the holder").unwrap();
        joined(waiter, "the waiter").unwrap();
    }

    #[test]
    fn cpu_a_far_wait_sleeps_through_other_participants_cycles() {
        // An idle manual clock: two participants cycle every 1 ms, as the Provider's threads
        // poll, while a far plain wait of a participant (as uhd-clock's) and of a
        // non-participant (as the Kernel's) block. Neither condition changes, so neither wait
        // wakes, but for a stall it entered on a stopped clock; a wait woken by every cycle (a
        // wake storm) would pass hundreds of times in this test.
        let clock = Arc::new(HostClock::manual());
        let stop = Arc::new(AtomicBool::new(false));
        let pollers: Vec<_> = (0..2)
            .map(|_| {
                let (c, s) = (clock.clone(), stop.clone());
                participant(&clock, move || {
                    while !s.load(SeqCst) {
                        c.sleep(MS);
                    }
                })
            })
            .collect();
        // Each reads the generation before it reports itself entered, so the final notify,
        // made only after both reports, cannot be missed.
        let (entered, entered_rx) = channel();
        let far = |c: Arc<HostClock>| {
            let entered = entered.clone();
            move || {
                let seen = c.generation();
                let _ = entered.send(());
                c.wait(Some(c.now() + Duration::from_secs(60)), None, seen);
                passes()
            }
        };
        let (theirs, rx) = channel();
        let waiter = {
            let (f, theirs) = (far(clock.clone()), theirs.clone());
            participant(&clock, move || {
                let _ = theirs.send(f());
            })
        };
        let outsider = {
            let f = far(clock.clone());
            std::thread::spawn(move || {
                let _ = theirs.send(f());
            })
        };
        bounded(&entered_rx, "a far wait began");
        bounded(&entered_rx, "the other far wait began");
        let c = clock.clone();
        within("300 ms of the clock", move || c.sleep(300 * MS));
        clock.notify_all();
        let counts = [bounded(&rx, "a far wait, notified"), bounded(&rx, "the other far wait, notified")];
        stop.store(true, SeqCst);
        for handle in pollers {
            joined(handle, "a poller").unwrap();
        }
        joined(waiter, "the participant's wait").unwrap();
        joined(outsider, "the non-participant's wait").unwrap();
        assert!(counts.iter().all(|n| *n <= 100), "far waits passed {counts:?} times through 300 ms of cycling participants");
    }

    #[test]
    fn cpu_a_stopped_clock_costs_a_near_wait_no_wakes() {
        // A wait 1 ms short of its deadline while a participant holds the clock stopped for
        // 400 ms: it blocks, then takes one wake when the clock runs on and one at its deadline.
        // Polling the frozen remainder as real time would wake it hundreds of times.
        let clock = Arc::new(HostClock::manual());
        let (go, gate) = channel::<()>();
        let busy = participant(&clock, move || {
            let _ = gate.recv();
            std::thread::sleep(400 * MS);
        });
        let c = clock.clone();
        let waiter = std::thread::spawn(move || {
            let seen = c.generation();
            c.wait(Some(c.now() + MS), None, seen);
            passes()
        });
        until_blocked(&clock, 1, "the waiter entered its wait");
        let _ = go.send(());
        joined(busy, "the busy participant").unwrap();
        let deadline = Instant::now() + BOUND;
        while !waiter.is_finished() {
            assert!(Instant::now() < deadline, "the wait ended within {BOUND:?}");
            std::thread::sleep(MS);
        }
        let passes = waiter.join().unwrap();
        assert!(passes <= 4, "a near wait on a stopped clock passed {passes} times");
    }
}
