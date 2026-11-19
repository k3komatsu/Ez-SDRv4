//! The start-window doubles shared by the coordinator and device-paced tests: a gate a
//! hook opens, the trace of `start_probe` points, a bounded worker start, a check that
//! records what the admitted configuration held, and clock doubles that read the lock.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use ezsdr_kernel::binding::{AdmissionCheck, AdmissionCheckRegistry, CheckStage, Violation};
use ezsdr_kernel::coordinator::start_probe::{self, Point, Reading};
use ezsdr_kernel::coordinator::{Assembly, RunHandle};
use ezsdr_kernel::id::ClockDomainId;
use ezsdr_kernel::module_api::{ActionSubmitter, Authority, AuthorityDescriptor, Pacing};
use ezsdr_kernel::run::{HostClock, RunState, TransitionRecord};
use ezsdr_kernel::spec::{Ident, Key, SpecError, Value};
use ezsdr_kernel::time::{ClockRelation, ScheduleHandle, TimeAuthority, TimeError, TimePoint};

use super::{FakeHostClock, Probe, ns};

/// A cell a hook fills with the start's probe.
pub type ProbeSlot = Arc<Mutex<Option<start_probe::Probe>>>;
/// A cell `ProbeExecutor::sharing_submitter` stores the Run's ActionSubmitter in.
pub type SubmitterSlot = Arc<Mutex<Option<Arc<dyn ActionSubmitter>>>>;

/// A gate a worker waits on until a hook opens it.
pub type Gate = Arc<(Mutex<bool>, Condvar)>;

/// A closed gate.
pub fn gate() -> Gate {
    Arc::new((Mutex::new(false), Condvar::new()))
}

/// Opens `gate` and wakes every waiter.
pub fn open(gate: &Gate) {
    let (closed, changed) = &**gate;
    *closed.lock().unwrap_or_else(|e| e.into_inner()) = true;
    changed.notify_all();
}

/// Held by a test so that a failing test does not leave a worker, an Executor or
/// a Provider parked on its gate.
pub struct OpenOnDrop(pub Gate);

impl Drop for OpenOnDrop {
    fn drop(&mut self) {
        open(&self.0);
    }
}

/// Blocks until `gate` is open.
pub fn wait(gate: &Gate) {
    let (closed, changed) = &**gate;
    let mut closed = closed.lock().unwrap_or_else(|e| e.into_inner());
    while !*closed {
        closed = changed.wait(closed).unwrap_or_else(|e| e.into_inner());
    }
}

/// The `(point, reading)`s a hook saw, shared with the test thread.
#[derive(Clone, Default)]
pub struct StartTrace(Arc<Mutex<Vec<(Point, Reading)>>>);

impl StartTrace {
    /// Appends one observation.
    pub fn push(&self, point: Point, reading: Reading) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).push((point, reading));
    }

    /// Copies every observation; the mutex is never held across an assert.
    pub fn points(&self) -> Vec<(Point, Reading)> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Checks the lock's extent at every recorded point and that every point in
    /// `required` was reached: `Entered` outside the lock, the held points inside it
    /// and unfrozen; `Released` is not checked (a competitor parked on the lock may
    /// take it right after the drop).
    pub fn assert_extent(&self, required: &[Point]) {
        let points = self.points();
        for (point, reading) in &points {
            match point {
                Point::Entered => assert!(!reading.admission_held, "KC-24a: the admission lock is held at {point:?}; trace: {points:?}"),
                Point::BeforePublish | Point::BeforeDispatch | Point::Dispatching { .. } => {
                    assert!(reading.admission_held, "KC-24a: the admission lock is not held at {point:?}; trace: {points:?}");
                    assert!(!reading.dispatch_frozen, "KC-24a: dispatch is frozen at {point:?}; trace: {points:?}");
                }
                Point::Released => {}
                _ => {}
            }
        }
        for wanted in required {
            assert!(points.iter().any(|(point, _)| point == wanted), "KC-24a: the start never reached {wanted:?}; trace: {points:?}");
        }
    }
}

/// A failure message that begins with the marker: a harness failure (a watchdog
/// expiry), which the mutation runner does not count as a kill.
pub fn infra(what: impl std::fmt::Display) -> String {
    format!("EZSDR-INFRA: a watchdog expired: {what}")
}

/// The result of running a start on a worker thread.
pub struct Started {
    result: Option<Result<RunHandle, SpecError>>,
    rx: std::sync::mpsc::Receiver<std::thread::Result<Result<RunHandle, SpecError>>>,
}

/// Runs `start` on a worker with `hook` installed there, bounded by the caller's waits.
pub fn start_on_worker(lines: &Probe, hook: impl FnMut(Point, &start_probe::Probe) + Send + 'static, start: impl FnOnce() -> Result<RunHandle, SpecError> + Send + 'static) -> Started {
    let (tx, rx) = std::sync::mpsc::channel();
    let lines = lines.clone();
    std::thread::Builder::new()
        .spawn(move || {
            let _installed = start_probe::install(hook);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(start));
            let panicked = result.is_err();
            let _ = tx.send(result);
            if panicked {
                lines.record("start:panicked");
            }
        })
        .expect("the start worker spawns");
    Started { result: None, rx }
}

impl Started {
    /// Resumes a panic the worker sent, or keeps a returned result.
    pub fn surface_panic(&mut self) {
        match self.rx.try_recv() {
            Ok(Err(payload)) => std::panic::resume_unwind(payload),
            Ok(Ok(result)) => self.result = Some(result),
            Err(_) => {}
        }
    }

    /// The started Run, or a panic that fired before it returned.
    pub fn finish(mut self) -> RunHandle {
        let result = match self.result.take() {
            Some(result) => Ok(result),
            None => match self.rx.recv_timeout(Duration::from_secs(10)) {
                Ok(result) => result,
                Err(_) => panic!("{}", infra("start did not return within 10 s: a lock cycle or a lock held across the wait")),
            },
        };
        match result {
            Ok(result) => result.expect("the Run starts"),
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// Whether the worker has returned, surfacing a panic that fired first; non-blocking.
    pub fn returned(&mut self) -> bool {
        self.surface_panic();
        self.result.is_some()
    }
}

/// Waits for `pred` or the worker's panic, then surfaces a panic so its assertion is the failure.
pub fn wait_or_panicked(started: &mut Started, lines: &Probe, pred: impl Fn(&[String]) -> bool) -> bool {
    let found = lines.wait_until(|observed| pred(observed) || observed.iter().any(|line| line == "start:panicked"), Duration::from_secs(10));
    started.surface_panic();
    found
}

/// Spins, for use inside a hook, until the Run has an end request; false after 10 s.
pub fn wait_end_requested(probe: &start_probe::Probe) -> bool {
    let begun = Instant::now();
    loop {
        if probe.read().is_some_and(|reading| reading.end_requested) {
            return true;
        }
        if begun.elapsed() >= Duration::from_secs(10) {
            return false;
        }
        std::thread::yield_now();
    }
}

/// The `(proposed, effective)` gains of every admission it judged.
pub type SeenLog = Arc<Mutex<Vec<(Option<Value>, Option<Value>)>>>;

/// An admission check that records what `test.gain` was proposed and what was effective, and refuses nothing.
pub struct Seen {
    section: ezsdr_kernel::spec::Namespace,
    stages: Vec<CheckStage>,
    log: SeenLog,
}

impl AdmissionCheck for Seen {
    fn section(&self) -> &ezsdr_kernel::spec::Namespace {
        &self.section
    }

    fn stages(&self) -> &[CheckStage] {
        &self.stages
    }

    fn check(&self, _section: &serde_json::Value, effective: &BTreeMap<Ident, BTreeMap<Key, Value>>, proposed: &BTreeMap<Ident, BTreeMap<Key, Value>>, _stage: CheckStage) -> Vec<Violation> {
        let key = Key::parse("test.gain").expect("the fixture key is valid");
        let proposed = proposed.values().find_map(|map| map.get(&key)).cloned();
        let effective = effective.values().find_map(|map| map.get(&key)).cloned();
        self.log.lock().unwrap_or_else(|e| e.into_inner()).push((proposed, effective));
        Vec::new()
    }
}

/// A `Seen` check and the log it writes.
pub fn seen() -> (Arc<Seen>, SeenLog) {
    let log: SeenLog = Arc::new(Mutex::new(Vec::new()));
    (Arc::new(Seen { section: ns("test.seen"), stages: vec![CheckStage::Runtime], log: log.clone() }), log)
}

/// The effective gains of every entry whose proposed gain equals `proposed`.
pub fn seen_for(log: &SeenLog, proposed: &Value) -> Vec<Option<Value>> {
    log.lock().unwrap_or_else(|e| e.into_inner()).iter().filter(|(seen, _)| seen.as_ref() == Some(proposed)).map(|(_, effective)| effective.clone()).collect()
}

/// Installs only `Seen` on the Run's checks and puts its environment section in `profile`.
pub fn with_seen(assembly: &mut Assembly, profile: &mut serde_json::Value) -> SeenLog {
    let (check, log) = seen();
    let mut checks = AdmissionCheckRegistry::new();
    checks.register(check);
    assembly.checks = checks;
    if !profile["environment"].is_object() {
        profile["environment"] = serde_json::json!({});
    }
    profile["environment"]["test.seen"] = serde_json::json!({});
    log
}

/// Which of the coordinator's serialising locks the calling thread held, and the
/// reading of it, from the Kernel's own per-thread record (KC-24a).
pub use ezsdr_kernel::coordinator::lock_record::{Held, held};

/// The supplied call a [`Call`] records.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CallKind {
    /// `TimeAuthority::now`.
    Now,
    /// `HostClock::utc_nanos`.
    Utc,
    /// `HostClock::monotonic_millis`.
    Monotonic,
    /// `TimeAuthority::schedule`.
    Schedule,
    /// `TimeAuthority::cancel`.
    Cancel,
    /// `TimeAuthority::wait_until`.
    WaitUntil,
    /// `Authority::next_wakeup`.
    NextWakeup,
    /// `Authority::relations`.
    Relations,
}

/// One supplied call, recorded at entry; [`ClockLog::fill`] sets the result slots once
/// the inner call returned.
#[derive(Clone, Debug)]
pub struct Call {
    /// Which supplied method.
    pub kind: CallKind,
    /// Which of the coordinator's locks the calling thread held at entry.
    pub held: Held,
    /// The thread that made the call.
    pub thread: std::thread::ThreadId,
    /// `Now`, `Utc`: the returned sample; `Schedule`, `WaitUntil`: the requested ticks in the primary root.
    pub sample: Option<i64>,
    /// `Schedule`: the handle returned; `Cancel`: the argument.
    pub handle: Option<ScheduleHandle>,
    /// `Cancel`: what the inner authority returned; `None` until it returned.
    pub returned: Option<bool>,
}

/// One event of the log: a supplied call, or a `start_probe` point a hook reached.
#[derive(Clone, Debug)]
pub enum LogEvent {
    /// A supplied call.
    Call(Call),
    /// A `start_probe` point.
    Point(Point),
}

/// The ordered events a [`WatchedClock`] and a [`WatchedAuthority`] record, so that a
/// call and a `start_probe` point keep their order.
#[derive(Clone, Default)]
pub struct ClockLog(Arc<Mutex<Vec<LogEvent>>>);

impl ClockLog {
    /// Appends one call at entry and returns its index, which [`fill`](Self::fill) takes.
    pub fn push_call(&self, call: Call) -> usize {
        let mut events = self.0.lock().unwrap_or_else(|e| e.into_inner());
        events.push(LogEvent::Call(call));
        events.len() - 1
    }

    /// Sets the result slots of the call at `index`, after the inner call returned.
    pub fn fill(&self, index: usize, set: impl FnOnce(&mut Call)) {
        let mut events = self.0.lock().unwrap_or_else(|e| e.into_inner());
        match &mut events[index] {
            LogEvent::Call(call) => set(call),
            LogEvent::Point(point) => panic!("KC-24a: {point:?} is not a call"),
        }
    }

    /// Appends one `start_probe` point, keeping the calls' order around it.
    pub fn push_point(&self, point: Point) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).push(LogEvent::Point(point));
    }

    /// Copies every event; the mutex is never held across an assert.
    pub fn events(&self) -> Vec<LogEvent> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Copies every call in order; the mutex is never held across an assert.
    pub fn calls(&self) -> Vec<Call> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter_map(|event| match event {
                LogEvent::Call(call) => Some(call.clone()),
                LogEvent::Point(_) => None,
            })
            .collect()
    }
}

/// `H`, the far-future untimed `stop` entry's handle (KC-20): the one `Schedule` call
/// whose instant is at least an hour after the Run's start, `start` in primary-root
/// ticks, with that instant. Fails when the fixture did not schedule exactly one.
pub fn far_future_stop(calls: &[Call], start: i64) -> (ScheduleHandle, i64) {
    const HOUR: i64 = 3_600_000_000_000;
    let far: Vec<&Call> = calls.iter().filter(|call| call.kind == CallKind::Schedule && call.sample.is_some_and(|sample| sample >= start.saturating_add(HOUR))).collect();
    assert_eq!(far.len(), 1, "RS-6 step 1: the Run scheduled one far-future handle; calls: {calls:?}");
    (far[0].handle.expect("the schedule returned its handle"), far[0].sample.expect("the schedule recorded its instant"))
}

/// The index windows of a Run's ordered [`Call`]s, delimited by its transitions' `Utc`
/// calls (KC-24a): a test requires what it needs to have seen in which window.
#[derive(Clone, Debug)]
pub struct Phases {
    /// The calls after the Armed transition's `Utc` and before Running's.
    pub armed: std::ops::Range<usize>,
    /// The calls after the Running transition's `Utc` and before Stopping's.
    pub running: std::ops::Range<usize>,
    /// The calls after the Stopping transition's `Utc` and before CleanedUp's.
    pub stopping: std::ops::Range<usize>,
}

/// KC-24a's rule over a whole Run made with the watched doubles (T11..T14): no supplied
/// call was made with the admission or the machine lock held; every transition after
/// Created has its `Now` sample immediately before its `Utc` sample on one thread; and
/// the Armed, Running and Stopping windows are returned for the test's own requirements.
/// When the Authority never advances, every `Now` sample is 0 and the sample equality is
/// vacuous -- the pair's presence and thread are still checked.
///
/// What the sweep cannot see: locks other than `admission` and `machine` (`end`, `also`,
/// `delivered`, `links`, the `wake_handle` held across `schedule`, slot locks); paths no
/// Run drives; supplied objects other than the Authority's time handle and the
/// HostClock; guards that bypass the recording helpers (T15 covers the text only); and
/// the deadlock itself -- it checks the deadlock's precondition.
pub fn sweep(calls: &[Call], transitions: &[TransitionRecord], primary: ClockDomainId) -> Phases {
    for call in calls {
        assert!(
            !call.held.admission && !call.held.machine,
            "KC-24a: {:?} on {:?} ran under a coordinator lock (admission {}, machine {}); calls: {calls:?}",
            call.kind,
            call.thread,
            call.held.admission,
            call.held.machine
        );
    }
    let utc: Vec<(usize, &Call)> = calls.iter().enumerate().filter(|(_, call)| call.kind == CallKind::Utc).collect();
    assert_eq!(utc.len(), transitions.len(), "KC-43: the UTC reads are not one per transition; calls: {calls:?}");
    let mut positions = [None; 4];
    for (row, (position, call)) in transitions.iter().zip(&utc) {
        assert_eq!(call.sample, Some(row.host_utc_nanos), "KC-43: the {:?} transition's UTC is not the sampled one; calls: {calls:?}", row.state);
        if !matches!(row.state, RunState::Created {}) {
            let at = row.at.expect("a transition after Created carries its runtime instant");
            let ticks = at.ticks_in(primary).expect("the transition's instant is on the primary root");
            let previous = calls[..*position]
                .iter()
                .rev()
                .find(|earlier| earlier.thread == call.thread)
                .unwrap_or_else(|| panic!("KC-43: the {:?} transition's UTC read has no earlier call on its thread; calls: {calls:?}", row.state));
            assert_eq!(previous.kind, CallKind::Now, "KC-43: the call before the {:?} transition's UTC read is {previous:?}, not the Authority's now; calls: {calls:?}", row.state);
            assert_eq!(previous.sample, Some(ticks), "KC-43: the {:?} transition's instant is not the sampled one; calls: {calls:?}", row.state);
        }
        match &row.state {
            RunState::Armed {} => positions[0] = Some(*position),
            RunState::Running {} => positions[1] = Some(*position),
            RunState::Stopping { .. } => positions[2] = Some(*position),
            RunState::CleanedUp { .. } => positions[3] = Some(*position),
            _ => {}
        }
    }
    let [Some(armed), Some(running), Some(stopping), Some(cleaned_up)] = positions else {
        panic!("KC-24a: the Run did not record Armed, Running, Stopping and CleanedUp: {positions:?}; calls: {calls:?}");
    };
    assert!(armed < running && running < stopping && stopping < cleaned_up, "KC-43: the transitions are not in order: {transitions:?}");
    Phases { armed: armed + 1..running, running: running + 1..stopping, stopping: stopping + 1..cleaned_up }
}

/// A hook body: it records into a probe or a log, or parks on a gate, on the calling
/// thread; assertions run on the test thread.
pub type CallHook = Arc<dyn Fn(&Call) + Send + Sync>;

/// Hooks a test installs on the watched `Authority` and the `TimeAuthority` it hands
/// out. Each runs on the calling thread, after its call was logged (so the entry and
/// its `held` are visible) and before it is forwarded, and only records or parks:
/// assertions run on the test thread.
#[derive(Clone, Default)]
pub struct Hooks {
    /// Runs at each `TimeAuthority::now` entry.
    pub on_now: Option<CallHook>,
    /// Runs at each `TimeAuthority::schedule` entry.
    pub on_schedule: Option<CallHook>,
    /// Runs at each `TimeAuthority::cancel` entry.
    pub on_cancel: Option<CallHook>,
    /// Runs at each `TimeAuthority::cancel` after the inner call returned and its result slot is filled.
    pub on_cancel_return: Option<CallHook>,
    /// Runs after `Authority::next_wakeup` returned, with its result.
    pub on_next_wakeup_return: Option<Arc<dyn Fn(Option<TimePoint>) + Send + Sync>>,
}

/// A `HostClock` that forwards to a `FakeHostClock` and records every call with the
/// locks its thread held, and a distinct sample per `utc_nanos` call.
pub struct WatchedClock {
    inner: Arc<FakeHostClock>,
    log: ClockLog,
    calls: AtomicI64,
}

impl WatchedClock {
    /// A forwarding clock that records into `log`.
    pub fn new(inner: Arc<FakeHostClock>, log: ClockLog) -> WatchedClock {
        WatchedClock { inner, log, calls: AtomicI64::new(0) }
    }
}

impl HostClock for WatchedClock {
    fn monotonic_millis(&self) -> u64 {
        self.log.push_call(Call { kind: CallKind::Monotonic, held: held(), thread: std::thread::current().id(), sample: None, handle: None, returned: None });
        self.inner.monotonic_millis()
    }

    fn utc_nanos(&self) -> i64 {
        let index = self.log.push_call(Call { kind: CallKind::Utc, held: held(), thread: std::thread::current().id(), sample: None, handle: None, returned: None });
        let sample = self.inner.utc_nanos() + self.calls.fetch_add(1, Ordering::SeqCst);
        self.log.fill(index, |call| call.sample = Some(sample));
        sample
    }
}

/// An Authority that forwards to `inner` and hands out a [`WatchedTime`]; both record
/// into the same log.
pub struct WatchedAuthority {
    inner: Box<dyn Authority>,
    log: ClockLog,
    hooks: Hooks,
}

impl WatchedAuthority {
    /// A forwarding Authority that records into `log`.
    pub fn new(inner: Box<dyn Authority>, log: ClockLog) -> WatchedAuthority {
        WatchedAuthority { inner, log, hooks: Hooks::default() }
    }

    /// Installs `hooks`; the [`WatchedTime`] handles this Authority hands out share them.
    pub fn with_hooks(mut self, hooks: Hooks) -> WatchedAuthority {
        self.hooks = hooks;
        self
    }
}

impl Authority for WatchedAuthority {
    fn descriptor(&self) -> &AuthorityDescriptor {
        self.inner.descriptor()
    }

    fn time(&self) -> Arc<dyn TimeAuthority> {
        Arc::new(WatchedTime { inner: self.inner.time(), log: self.log.clone(), hooks: self.hooks.clone() })
    }

    fn next_wakeup(&self) -> Option<TimePoint> {
        self.log.push_call(Call { kind: CallKind::NextWakeup, held: held(), thread: std::thread::current().id(), sample: None, handle: None, returned: None });
        let result = self.inner.next_wakeup();
        if let Some(hook) = &self.hooks.on_next_wakeup_return {
            hook(result);
        }
        result
    }

    fn relations(&self) -> Vec<ClockRelation> {
        self.log.push_call(Call { kind: CallKind::Relations, held: held(), thread: std::thread::current().id(), sample: None, handle: None, returned: None });
        self.inner.relations()
    }
}

/// A TimeAuthority that forwards to `inner` and records every watched call with the
/// locks its thread held at entry and the result it needed.
pub struct WatchedTime {
    inner: Arc<dyn TimeAuthority>,
    log: ClockLog,
    hooks: Hooks,
}

impl TimeAuthority for WatchedTime {
    fn primary_root(&self) -> ClockDomainId {
        self.inner.primary_root()
    }

    fn pacing(&self) -> Pacing {
        self.inner.pacing()
    }

    fn governs(&self, domain: ClockDomainId) -> bool {
        self.inner.governs(domain)
    }

    fn now(&self, domain: ClockDomainId) -> Result<TimePoint, TimeError> {
        let call = Call { kind: CallKind::Now, held: held(), thread: std::thread::current().id(), sample: None, handle: None, returned: None };
        let index = self.log.push_call(call.clone());
        if let Some(hook) = &self.hooks.on_now {
            hook(&call);
        }
        let result = self.inner.now(domain);
        if let Ok(t) = &result {
            if let Ok(ticks) = t.ticks_in(domain) {
                self.log.fill(index, |call| call.sample = Some(ticks));
            }
        }
        result
    }

    fn wait_until(&self, t: TimePoint) -> Result<(), TimeError> {
        let held = held();
        let thread = std::thread::current().id();
        let sample = t.ticks_in(self.inner.primary_root()).ok();
        self.log.push_call(Call { kind: CallKind::WaitUntil, held, thread, sample, handle: None, returned: None });
        self.inner.wait_until(t)
    }

    fn schedule(&self, t: TimePoint, f: Box<dyn FnOnce(TimePoint) + Send>) -> Result<ScheduleHandle, TimeError> {
        let held = held();
        let thread = std::thread::current().id();
        let sample = t.ticks_in(self.inner.primary_root()).ok();
        let call = Call { kind: CallKind::Schedule, held, thread, sample, handle: None, returned: None };
        let index = self.log.push_call(call.clone());
        if let Some(hook) = &self.hooks.on_schedule {
            hook(&call);
        }
        let result = self.inner.schedule(t, f);
        if let Ok(handle) = &result {
            self.log.fill(index, |call| call.handle = Some(*handle));
        }
        result
    }

    fn cancel(&self, h: ScheduleHandle) -> bool {
        let call = Call { kind: CallKind::Cancel, held: held(), thread: std::thread::current().id(), sample: None, handle: Some(h), returned: None };
        let index = self.log.push_call(call.clone());
        if let Some(hook) = &self.hooks.on_cancel {
            hook(&call);
        }
        let returned = self.inner.cancel(h);
        self.log.fill(index, |call| call.returned = Some(returned));
        if let Some(hook) = &self.hooks.on_cancel_return {
            hook(&Call { returned: Some(returned), ..call });
        }
        returned
    }
}
