//! Trace forcing (stage E2a; test plan §3, §14): the scheduler every thread of the runtime waits through, and the
//! controller that replays a DST trace on it. Outside forcing a hook does nothing and a wait is a condvar wait.
//!
//! Every wait of the runtime goes through [`Sched::wait_until`] or [`Sched::acquire`]: the waiter registers the predicate
//! it waits for, so the controller can tell a blocked actor (predicate false) from one that is about to run (predicate true).
//! A contended acquisition is permit-then-ack: the actor waits for the resource, parks at the permit point, and the grant
//! lets it take the resource; the event is complete when the controller sees the system quiet again.

use std::cell::Cell as StdCell;
use std::collections::BTreeMap;
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
    host: Mutex<Option<crate::host::HostClock>>,
}

const POLL: Duration = Duration::from_millis(2);
const STEP_BOUND: Duration = Duration::from_secs(10);

impl Sched {
    pub(crate) fn new(forcing: bool) -> Arc<Sched> {
        Arc::new(Sched { m: Mutex::new(Reg { status: BTreeMap::new(), grant: None, aborted: false, panics: Vec::new() }), cv: Condvar::new(), forcing, host: Mutex::new(None) })
    }

    pub(crate) fn forcing(&self) -> bool {
        self.forcing
    }

    fn lock(&self) -> MutexGuard<'_, Reg> {
        self.m.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Wakes every waiter: called after any state change a predicate may read, with no inner lock held.
    pub(crate) fn notify(&self) {
        {
            let _reg = self.lock();
            self.cv.notify_all();
        }
        if let Some(host) = self.host.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            host.notify_all();
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
    fn idle<'a>(&'a self, reg: MutexGuard<'a, Reg>) -> MutexGuard<'a, Reg> {
        match PARTICIPANT_HOST.with(|h| h.borrow().clone()) {
            Some(host) => {
                let seen = host.generation();
                drop(reg);
                host.wait(None, Some(Instant::now() + POLL), seen);
                self.lock()
            }
            None => self.cv.wait_timeout(reg, POLL).unwrap_or_else(|e| e.into_inner()).0,
        }
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
            reg = if self.forcing { self.cv.wait(reg).unwrap_or_else(|e| e.into_inner()) } else { self.idle(reg) };
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
                self.cv.notify_all();
                return (Forced::None, taken);
            }
            if !matches!(reg.status.get(&actor), Some(Status::Blocked(_))) {
                reg.status.insert(actor, Status::Blocked(pred.clone()));
                self.cv.notify_all();
            }
            reg = self.idle(reg);
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
        self.cv.notify_all();
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
        None => std::thread::spawn(run),
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
