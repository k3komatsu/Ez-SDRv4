//! Trace forcing (stage E2a; test plan §3, §14): the scheduler every thread of the runtime waits through, and the
//! controller that replays a DST trace on it. Outside forcing a hook does nothing and a wait is a condvar wait.
//!
//! Every wait of the runtime goes through [`Sched::wait_until`] or [`Sched::acquire`]: the waiter registers the predicate
//! it waits for, so the controller can tell a blocked actor (predicate false) from one that is about to run (predicate true).
//! A contended acquisition is permit-then-ack: the actor waits for the resource, parks at the permit point, and the grant
//! lets it take the resource; the event is complete when the controller sees the system quiet again.
//!
//! A free run on a real clock is *targeted* ([`Sched::set_targeted`]): each actor waits on a condition of its own, and a push or a change
//! wakes only the actors whose input it is ([`Sched::wake`]); nothing is woken for a block or an acquisition. The waits and their loops are the
//! same code as the forcing run's, only the wait and the wake primitive differ.

use std::cell::Cell as StdCell;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::cell::Outcome;
use super::world::PubStep;

/// A thread of the runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Actor {
    Brain,
    Settings,
    Rx(usize),
    Clock,
    Timer,
    Main,
}

/// A point where an actor parks while forcing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Point {
    BStep,
    BOp,
    STake,
    SDiscard,
    SCheck,
    SResult,
    SSend,
    RCheck,
    RCmd,
    RSnap,
    RBegin,
    REnd,
    RPublish,
    RGate,
    RUngate,
    RResult,
    RSend,
    /// The permit to acquire the native lock `L`.
    Lock,
    /// The device call under `L` (the effect).
    Call,
    /// The permit to release `L`.
    Unlock,
    Pub(PubStep),
    ClockSend,
}

/// What the controller forces on the call it grants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Forced {
    None,
    Outcome(Outcome),
    Fault,
}

type Pred = Arc<dyn Fn() -> bool + Send + Sync>;

enum Status {
    Running,
    Blocked(Pred),
    /// Parked at a point; a permit point carries the predicate of the resource it permits taking (false: still held, so the
    /// attempt is pending, not an event).
    Parked(Point, Option<Pred>),
    Exited,
}

impl std::fmt::Debug for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Status::Running => f.write_str("Running"),
            Status::Blocked(pred) => write!(f, "Blocked(ready: {})", pred()),
            Status::Parked(point, None) => write!(f, "Parked({point:?})"),
            Status::Parked(point, Some(pred)) => write!(f, "Parked({point:?}, resource free: {})", pred()),
            Status::Exited => f.write_str("Exited"),
        }
    }
}

struct Reg {
    status: BTreeMap<Actor, Status>,
    grant: Option<(Actor, Forced)>,
    aborted: bool,
    /// The messages of the panics that were not an abort.
    panics: Vec<String>,
    /// A targeted run's condition of each actor that has waited.
    cvs: BTreeMap<Actor, Arc<Condvar>>,
    /// The actors of a targeted run that are inside their condition's wait now (set and cleared under the lock the wait releases).
    sleeping: std::collections::BTreeSet<Actor>,
    /// How many times each actor has entered that wait.
    waits: BTreeMap<Actor, u64>,
}

/// The unwind payload that ends a thread when the run is aborted.
pub(crate) struct Aborted;

thread_local! {
    static ACTOR: StdCell<Option<Actor>> = const { StdCell::new(None) };
    static PARTICIPANT_HOST: std::cell::RefCell<Option<crate::host::HostClock>> = const { std::cell::RefCell::new(None) };
}

pub(crate) fn actor() -> Actor {
    ACTOR.with(StdCell::get).unwrap_or(Actor::Main)
}

pub(crate) struct Sched {
    m: Mutex<Reg>,
    cv: Condvar,
    forcing: bool,
    /// A free run on a real clock: waits and wake-ups are per actor.
    targeted: AtomicBool,
    /// Mirrors `Reg::aborted` for a waiter that does not take the scheduler lock.
    aborted: AtomicBool,
    host: Mutex<Option<crate::host::HostClock>>,
}

const POLL: Duration = Duration::from_millis(2);
const STEP_BOUND: Duration = Duration::from_secs(10);

impl Sched {
    pub(crate) fn new(forcing: bool) -> Arc<Sched> {
        Arc::new(Sched {
            m: Mutex::new(Reg { status: BTreeMap::new(), grant: None, aborted: false, panics: Vec::new(), cvs: BTreeMap::new(), sleeping: Default::default(), waits: BTreeMap::new() }),
            cv: Condvar::new(),
            forcing,
            targeted: AtomicBool::new(false),
            aborted: AtomicBool::new(false),
            host: Mutex::new(None),
        })
    }

    pub(crate) fn forcing(&self) -> bool {
        self.forcing
    }

    fn lock(&self) -> MutexGuard<'_, Reg> {
        self.m.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Makes the run targeted (a free run on a real clock, no manual host): see the module comment.
    pub(crate) fn set_targeted(&self) {
        assert!(!self.forcing, "a forcing run is never targeted");
        self.targeted.store(true, Ordering::SeqCst);
    }

    pub(crate) fn targeted(&self) -> bool {
        self.targeted.load(Ordering::SeqCst)
    }

    /// Wakes every waiter: called after any state change a predicate may read, with no inner lock held.
    pub(crate) fn notify(&self) {
        {
            let reg = self.lock();
            self.cv.notify_all();
            reg.cvs.values().for_each(|cv| cv.notify_all());
        }
        if let Some(host) = self.host.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            host.notify_all();
        }
    }

    /// Wakes `actor`: in a targeted run only its own condition (the lock makes the wake-up follow the state change the waiter evaluates under it),
    /// otherwise everyone.
    pub(crate) fn wake(&self, actor: Actor) {
        if !self.targeted() {
            return self.notify();
        }
        let reg = self.lock();
        if let Some(cv) = reg.cvs.get(&actor) {
            cv.notify_all();
        }
    }

    /// A state change only the forcing controller or a polling waiter reads (a publication): nothing in a targeted run.
    pub(crate) fn changed(&self) {
        if !self.targeted() {
            self.notify();
        }
    }

    pub(crate) fn host(&self) -> Option<crate::host::HostClock> {
        self.host.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The manual host clock whose participants some actors are: their idle waits are waits on it (M2), and a notify wakes them there.
    pub(crate) fn set_host(&self, host: crate::host::HostClock) {
        *self.host.lock().unwrap_or_else(|e| e.into_inner()) = Some(host);
    }

    /// One idle wait of the calling actor, at most `POLL` of wall time: a participant of a manual clock waits on that clock (the
    /// clock may then pass it, rules M2-M4), any other thread on the scheduler's condition.
    ///
    /// A targeted run waits on the actor's own condition, until a wake-up or, if given, `timeout`.
    fn idle<'a>(&'a self, actor: Actor, mut reg: MutexGuard<'a, Reg>, timeout: Option<Duration>) -> MutexGuard<'a, Reg> {
        match PARTICIPANT_HOST.with(|h| h.borrow().clone()) {
            Some(host) => {
                let seen = host.generation();
                drop(reg);
                host.wait(None, Some(Instant::now() + POLL), seen);
                self.lock()
            }
            None if self.targeted() => {
                let cv = reg.cvs.entry(actor).or_insert_with(|| Arc::new(Condvar::new())).clone();
                reg.sleeping.insert(actor);
                *reg.waits.entry(actor).or_default() += 1;
                let mut reg = match timeout {
                    None => cv.wait(reg).unwrap_or_else(|e| e.into_inner()),
                    Some(timeout) => cv.wait_timeout(reg, timeout).unwrap_or_else(|e| e.into_inner()).0,
                };
                reg.sleeping.remove(&actor);
                reg
            }
            None => self.cv.wait_timeout(reg, POLL).unwrap_or_else(|e| e.into_inner()).0,
        }
    }

    /// Blocks until `f` yields a value; `Err` is "not yet", with the longest the waiter may sleep (`None`: until woken). `f` is evaluated under the
    /// scheduler lock, and a waker takes that lock after its change, so no wake-up is lost. Outside forcing only (a forcing wait registers its predicate).
    pub(crate) fn wait_for<T>(&self, actor: Actor, mut f: impl FnMut() -> Result<T, Option<Duration>>) -> T {
        let mut reg = self.lock();
        loop {
            if reg.aborted {
                Self::abort_now(reg);
            }
            match f() {
                Ok(value) => {
                    reg.status.insert(actor, Status::Running);
                    return value;
                }
                Err(timeout) => reg = self.idle(actor, reg, timeout),
            }
        }
    }

    /// The receive executor's loop-top acquisition: waits until `f` yields a value (one evaluation of the loop's decision per call, not a predicate
    /// and then a second evaluation). Forcing: the permit-then-ack acquisition of the predicate `f` is `Ok`, then `f`'s value (`None` if the state
    /// moved on). Free: `f` is tried first without the scheduler lock, so an actor that does not wait costs no scheduler work.
    pub(crate) fn acquire_with<T: 'static>(&self, actor: Actor, point: Point, f: Arc<dyn Fn() -> Result<T, Option<Duration>> + Send + Sync>) -> Option<T> {
        if self.forcing {
            let pred: Pred = {
                let f = f.clone();
                Arc::new(move || f().is_ok())
            };
            self.acquire(actor, point, pred, || ());
            return f().ok();
        }
        if self.aborted.load(Ordering::SeqCst) {
            Self::abort_now(self.lock());
        }
        if let Ok(value) = f() {
            return Some(value);
        }
        Some(self.wait_for(actor, || f()))
    }

    pub(crate) fn register(&self, actor: Actor) {
        self.lock().status.insert(actor, Status::Running);
    }

    fn abort_now(reg: MutexGuard<'_, Reg>) -> ! {
        drop(reg);
        std::panic::resume_unwind(Box::new(Aborted))
    }

    /// Blocks until `pred` holds. The predicate is evaluated under the scheduler lock, so it may read other state but
    /// no holder of that state may call into the scheduler while holding it.
    pub(crate) fn wait_until(&self, actor: Actor, pred: Pred) {
        let mut reg = self.lock();
        loop {
            if reg.aborted {
                Self::abort_now(reg);
            }
            if pred() {
                reg.status.insert(actor, Status::Running);
                return;
            }
            if !matches!(reg.status.get(&actor), Some(Status::Blocked(_))) {
                reg.status.insert(actor, Status::Blocked(pred.clone()));
                self.cv.notify_all();
            }
            reg = if self.forcing { self.cv.wait(reg).unwrap_or_else(|e| e.into_inner()) } else { self.idle(actor, reg, None) };
        }
    }

    /// The hook: parks at `point` until the controller grants it. Does nothing outside forcing.
    pub(crate) fn hook(&self, actor: Actor, point: Point) -> Forced {
        self.hook_with(actor, point, None)
    }

    fn hook_with(&self, actor: Actor, point: Point, permit: Option<Pred>) -> Forced {
        if !self.forcing {
            return Forced::None;
        }
        let mut reg = self.lock();
        reg.status.insert(actor, Status::Parked(point, permit));
        self.cv.notify_all();
        loop {
            if reg.aborted {
                Self::abort_now(reg);
            }
            if reg.grant.is_some_and(|(who, _)| who == actor) {
                let (_, forced) = reg.grant.take().expect("a grant");
                reg.status.insert(actor, Status::Running);
                self.cv.notify_all();
                return forced;
            }
            reg = self.cv.wait(reg).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// An acquisition: wait for `pred` (a pending attempt is blocked, not an event), park at the permit `point`, and
    /// take the resource once granted. Outside forcing the wait and the take are one step under the scheduler lock, so
    /// two contenders never both take it.
    pub(crate) fn acquire<T>(&self, actor: Actor, point: Point, pred: Pred, take: impl FnOnce() -> T) -> (Forced, T) {
        if self.forcing {
            self.wait_until(actor, pred.clone());
            let forced = self.hook_with(actor, point, Some(pred.clone()));
            assert!(pred(), "{actor:?} was granted {point:?} while the resource is held: the trace does not describe this run");
            let taken = take();
            self.notify();
            return (forced, taken);
        }
        let mut reg = self.lock();
        loop {
            if reg.aborted {
                Self::abort_now(reg);
            }
            if pred() {
                let taken = take();
                reg.status.insert(actor, Status::Running);
                self.changed_locked(&reg);
                return (Forced::None, taken);
            }
            if !matches!(reg.status.get(&actor), Some(Status::Blocked(_))) {
                reg.status.insert(actor, Status::Blocked(pred.clone()));
                self.changed_locked(&reg);
            }
            reg = self.idle(actor, reg, None);
        }
    }

    /// [`Sched::changed`] with the lock held.
    fn changed_locked(&self, _reg: &MutexGuard<'_, Reg>) {
        if !self.targeted() {
            self.cv.notify_all();
        }
    }

    pub(crate) fn exited(&self, actor: Actor) {
        let mut reg = self.lock();
        reg.status.insert(actor, Status::Exited);
        self.cv.notify_all();
    }

    pub(crate) fn panicked(&self, message: String) {
        self.lock().panics.push(message);
    }

    pub(crate) fn panics(&self) -> Vec<String> {
        self.lock().panics.clone()
    }

    /// Ends the run: every parked or blocked actor unwinds.
    pub(crate) fn abort(&self) {
        let mut reg = self.lock();
        reg.aborted = true;
        self.aborted.store(true, Ordering::SeqCst);
        self.cv.notify_all();
        reg.cvs.values().for_each(|cv| cv.notify_all());
    }

    fn quiet(reg: &Reg) -> bool {
        reg.status.values().all(|status| match status {
            Status::Parked(..) | Status::Exited => true,
            Status::Blocked(pred) => !pred(),
            Status::Running => false,
        })
    }

    fn describe(reg: &Reg) -> String {
        format!("{:?}", reg.status)
    }

    /// Waits until every actor is parked, blocked on a false predicate, or exited.
    pub(crate) fn settle(&self) -> Result<(), String> {
        let started = Instant::now();
        let mut reg = self.lock();
        loop {
            if Self::quiet(&reg) {
                return Ok(());
            }
            if started.elapsed() > STEP_BOUND {
                return Err(format!("the system never went quiet: {}", Self::describe(&reg)));
            }
            self.cv.notify_all();
            reg = self.cv.wait_timeout(reg, Duration::from_millis(1)).unwrap_or_else(|e| e.into_inner()).0;
        }
    }

    /// Grants `point` to `actor`, which must be parked there, and waits until the system is quiet again.
    pub(crate) fn step(&self, actor: Actor, point: Point, forced: Forced) -> Result<(), String> {
        self.settle()?;
        let mut reg = self.lock();
        match reg.status.get(&actor) {
            Some(Status::Parked(parked, permit)) if *parked == point => {
                if permit.as_ref().is_some_and(|free| !free()) {
                    return Err(format!("expected {actor:?} at {point:?}, found Blocked: the resource is held (a pending attempt is not an event); all: {}", Self::describe(&reg)));
                }
            }
            other => return Err(format!("expected {actor:?} at {point:?}, found {other:?}; all: {}", Self::describe(&reg))),
        }
        reg.grant = Some((actor, forced));
        self.cv.notify_all();
        let started = Instant::now();
        while reg.grant.is_some() {
            if started.elapsed() > STEP_BOUND {
                return Err(format!("{actor:?} did not take the grant of {point:?}"));
            }
            reg = self.cv.wait_timeout(reg, Duration::from_millis(1)).unwrap_or_else(|e| e.into_inner()).0;
        }
        drop(reg);
        self.settle()
    }

    /// The state of `actor`, for assertions: blocked (waiting on a false predicate), parked at a point, and so on.
    pub(crate) fn state_of(&self, actor: Actor) -> String {
        let reg = self.lock();
        format!("{:?}", reg.status.get(&actor))
    }

    /// The thread of `actor` has ended (its body returned).
    pub(crate) fn is_exited(&self, actor: Actor) -> bool {
        matches!(self.lock().status.get(&actor), Some(Status::Exited))
    }

    /// `Some(n)` when `actor` of a targeted run has evaluated its condition, found it false and is inside the wait for its wake-up, for the `n`-th time:
    /// a push made once this holds can only be seen through that wake-up (the flag and the wait share one lock hold), and a later `n` than before
    /// shows the actor evaluated again since.
    pub(crate) fn waiting(&self, actor: Actor) -> Option<u64> {
        let reg = self.lock();
        reg.sleeping.contains(&actor).then(|| reg.waits.get(&actor).copied().unwrap_or(0))
    }

    pub(crate) fn is_blocked(&self, actor: Actor) -> bool {
        match self.lock().status.get(&actor) {
            Some(Status::Blocked(pred)) => !pred(),
            Some(Status::Parked(_, Some(free))) => !free(),
            _ => false,
        }
    }
}

/// Runs `body` as `actor` on its own thread, which is a participant of a manual host clock when `host` is given (rules M1-M7, as production's executors are,
/// `provider/mod.rs`); an abort ends it quietly, any other panic is recorded: the clock cannot advance while the thread is runnable, and a wait on the clock is its idle state.
pub(crate) fn spawn_on(sched: &Arc<Sched>, host: Option<&crate::host::HostClock>, actor: Actor, body: impl FnOnce() + Send + 'static) -> std::thread::JoinHandle<()> {
    sched.register(actor);
    let sched = sched.clone();
    let participant_host = host.cloned();
    let run = move || {
        ACTOR.with(|a| a.set(Some(actor)));
        PARTICIPANT_HOST.with(|h| *h.borrow_mut() = participant_host);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
        if let Err(payload) = result {
            if payload.downcast_ref::<Aborted>().is_none() {
                let message = payload.downcast_ref::<String>().cloned().or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string())).unwrap_or_else(|| "a panic".to_string());
                sched.panicked(format!("{actor:?}: {message}"));
            }
        }
        sched.exited(actor);
    };
    match host {
        Some(host) => host.spawn(&format!("{actor:?}"), run).expect("a participant thread"),
        None => std::thread::Builder::new().name(format!("{actor:?}")).spawn(run).expect("a thread"),
    }
}

/// Joins every handle within `bound`; a thread still running after it is a failure.
pub(crate) fn join_all(handles: Vec<std::thread::JoinHandle<()>>, bound: Duration) -> Result<(), String> {
    let started = Instant::now();
    for handle in handles {
        while !handle.is_finished() {
            if started.elapsed() > bound {
                return Err("a thread did not finish within the bound".to_string());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let _ = handle.join();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    /// An actor waiting through `wait_for` on its own flag, counting how often it evaluates the flag.
    struct Waiter {
        flag: Arc<AtomicBool>,
        evaluations: Arc<AtomicUsize>,
        handle: std::thread::JoinHandle<()>,
    }

    fn waiter(sched: &Arc<Sched>, actor: Actor) -> Waiter {
        let (flag, evaluations) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicUsize::new(0)));
        let (s, f, e) = (sched.clone(), flag.clone(), evaluations.clone());
        let handle = std::thread::spawn(move || {
            s.wait_for(actor, || {
                e.fetch_add(1, Ordering::SeqCst);
                if f.load(Ordering::SeqCst) { Ok(()) } else { Err(None) }
            })
        });
        // The waiter has evaluated once and sleeps.
        let started = Instant::now();
        while evaluations.load(Ordering::SeqCst) == 0 {
            assert!(started.elapsed() < Duration::from_secs(10), "{actor:?} never started");
            std::thread::yield_now();
        }
        Waiter { flag, evaluations, handle }
    }

    fn joins(w: Waiter) {
        let started = Instant::now();
        while !w.handle.is_finished() {
            assert!(started.elapsed() < Duration::from_secs(10), "the waiter was never woken");
            std::thread::sleep(Duration::from_millis(1));
        }
        w.handle.join().unwrap();
    }

    fn targeted() -> Arc<Sched> {
        let sched = Sched::new(false);
        sched.set_targeted();
        sched
    }

    /// In a targeted run a wake reaches the actor it names, whose predicate is then true, and no other actor evaluates anything: no broadcast. A
    /// bounded wait: a wake that reaches no one (or the wrong actor) leaves the waiter asleep and fails here.
    #[test]
    fn u15a_a_wake_reaches_the_named_actor_alone() {
        let sched = targeted();
        let (brain, settings) = (waiter(&sched, Actor::Brain), waiter(&sched, Actor::Settings));
        brain.flag.store(true, Ordering::SeqCst);
        sched.wake(Actor::Brain);
        let seen = settings.evaluations.load(Ordering::SeqCst);
        let b = brain.evaluations.clone();
        joins(brain);
        assert!(b.load(Ordering::SeqCst) >= 2, "the brain evaluated again when woken");
        assert_eq!(settings.evaluations.load(Ordering::SeqCst), seen, "S was not woken for the brain's input");
        assert_eq!(seen, 1, "S evaluated its input once, before it slept");
        settings.flag.store(true, Ordering::SeqCst);
        sched.wake(Actor::Settings);
        joins(settings);
    }

    /// A state change only a controller or a polling waiter reads (`changed`) wakes no one in a targeted run; `notify` wakes everyone (the stop).
    #[test]
    fn u15b_changed_wakes_no_one_and_notify_wakes_everyone() {
        let sched = targeted();
        let (brain, rx) = (waiter(&sched, Actor::Brain), waiter(&sched, Actor::Rx(0)));
        sched.changed();
        sched.wake(Actor::Settings);
        sched.wake(Actor::Clock);
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!((brain.evaluations.load(Ordering::SeqCst), rx.evaluations.load(Ordering::SeqCst)), (1, 1), "nothing woke them");
        brain.flag.store(true, Ordering::SeqCst);
        rx.flag.store(true, Ordering::SeqCst);
        sched.notify();
        joins(brain);
        joins(rx);
    }

    /// A waiter with a deadline wakes by itself (a flush's end, a release timer's deadline) and a wake before it ends the wait early.
    #[test]
    fn u15d_a_timed_wait_ends_at_its_deadline_or_at_a_wake() {
        let sched = targeted();
        let started = Instant::now();
        let mut tries = 0;
        sched.wait_for(Actor::Timer, || {
            tries += 1;
            if tries < 3 { Err(Some(Duration::from_millis(5))) } else { Ok(()) }
        });
        assert!(started.elapsed() >= Duration::from_millis(10), "two timed waits of 5 ms each ran their course");
        // A long deadline is cut short by a wake.
        let flag = Arc::new(AtomicBool::new(false));
        let (s, f) = (sched.clone(), flag.clone());
        let handle = std::thread::spawn(move || s.wait_for(Actor::Timer, || if f.load(Ordering::SeqCst) { Ok(()) } else { Err(Some(Duration::from_secs(600))) }));
        std::thread::sleep(Duration::from_millis(20));
        flag.store(true, Ordering::SeqCst);
        sched.wake(Actor::Timer);
        let waited = Instant::now();
        while !handle.is_finished() {
            assert!(waited.elapsed() < Duration::from_secs(10), "a wake did not cut the 600 s wait short");
            std::thread::sleep(Duration::from_millis(1));
        }
        handle.join().unwrap();
    }

    /// A receive executor that never waits still ends when the run is aborted (`acquire_with` checks before it evaluates).
    #[test]
    fn u15f_abort_ends_an_acquisition_that_never_waits() {
        let sched = targeted();
        let s = sched.clone();
        let handle = std::thread::spawn(move || {
            let always: Arc<dyn Fn() -> Result<(), Option<Duration>> + Send + Sync> = Arc::new(|| Ok(()));
            loop {
                s.acquire_with(Actor::Rx(0), Point::RCheck, always.clone());
            }
        });
        std::thread::sleep(Duration::from_millis(20));
        sched.abort();
        let started = Instant::now();
        while !handle.is_finished() {
            assert!(started.elapsed() < Duration::from_secs(10), "the abort did not end the loop");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(handle.join().unwrap_err().downcast_ref::<Aborted>().is_some());
    }

    /// An aborted run ends a targeted wait too.
    #[test]
    fn u15e_abort_ends_a_targeted_wait() {
        let sched = targeted();
        let w = waiter(&sched, Actor::Brain);
        sched.abort();
        let started = Instant::now();
        while !w.handle.is_finished() {
            assert!(started.elapsed() < Duration::from_secs(10), "the abort did not end the wait");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(w.handle.join().unwrap_err().downcast_ref::<Aborted>().is_some(), "the wait unwinds with the abort payload");
    }
}
