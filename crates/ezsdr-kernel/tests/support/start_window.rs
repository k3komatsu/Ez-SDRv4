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
use ezsdr_kernel::run::HostClock;
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

/// One clock observation the doubles recorded, with whether the admission lock was
/// held at that call.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClockEvent {
    /// A `TimeAuthority::now` call and its returned primary-root tick.
    Now {
        /// The admission lock was held at the call.
        held: bool,
        /// The returned sample.
        sample: i64,
    },
    /// A `HostClock::utc_nanos` call and its returned value.
    Utc {
        /// The admission lock was held at the call.
        held: bool,
        /// The returned sample.
        sample: i64,
    },
    /// A `start_probe` point the hook reached.
    Point(Point),
}

/// The ordered events a [`WatchedClock`] and a [`WatchedAuthority`] record.
#[derive(Clone, Default)]
pub struct ClockLog(Arc<Mutex<Vec<ClockEvent>>>);

impl ClockLog {
    /// Appends one event.
    pub fn push(&self, event: ClockEvent) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).push(event);
    }

    /// Copies every event; the mutex is never held across an assert.
    pub fn events(&self) -> Vec<ClockEvent> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// A HostClock that forwards to a `FakeHostClock` and records, once a probe is in its
/// slot, whether the admission lock was held at each `utc_nanos`, with a distinct
/// sample per call.
pub struct WatchedClock {
    inner: Arc<FakeHostClock>,
    probe: ProbeSlot,
    log: ClockLog,
    calls: AtomicI64,
}

impl WatchedClock {
    /// A forwarding clock and the cell its probe goes through.
    pub fn new(inner: Arc<FakeHostClock>, log: ClockLog) -> (WatchedClock, ProbeSlot) {
        let probe = Arc::new(Mutex::new(None));
        (WatchedClock { inner, probe: probe.clone(), log, calls: AtomicI64::new(0) }, probe)
    }
}

impl HostClock for WatchedClock {
    fn monotonic_millis(&self) -> u64 {
        self.inner.monotonic_millis()
    }

    fn utc_nanos(&self) -> i64 {
        let sample = self.inner.utc_nanos() + self.calls.fetch_add(1, Ordering::SeqCst);
        let probe = self.probe.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(probe) = probe {
            if let Some(reading) = probe.read() {
                self.log.push(ClockEvent::Utc { held: reading.admission_held, sample });
            }
        }
        sample
    }
}

/// An Authority that forwards to `inner` and hands out a [`WatchedTime`].
pub struct WatchedAuthority {
    inner: Box<dyn Authority>,
    probe: ProbeSlot,
    log: ClockLog,
}

impl WatchedAuthority {
    /// A forwarding Authority with the cell its `now` readings go through.
    pub fn new(inner: Box<dyn Authority>, probe: ProbeSlot, log: ClockLog) -> WatchedAuthority {
        WatchedAuthority { inner, probe, log }
    }
}

impl Authority for WatchedAuthority {
    fn descriptor(&self) -> &AuthorityDescriptor {
        self.inner.descriptor()
    }

    fn time(&self) -> Arc<dyn TimeAuthority> {
        Arc::new(WatchedTime { inner: self.inner.time(), probe: self.probe.clone(), log: self.log.clone() })
    }

    fn next_wakeup(&self) -> Option<TimePoint> {
        self.inner.next_wakeup()
    }

    fn relations(&self) -> Vec<ClockRelation> {
        self.inner.relations()
    }
}

/// A TimeAuthority that forwards to `inner` and records, once a probe is in its slot,
/// whether the admission lock was held at each `now`, with the returned sample.
pub struct WatchedTime {
    inner: Arc<dyn TimeAuthority>,
    probe: ProbeSlot,
    log: ClockLog,
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
        let result = self.inner.now(domain);
        if let Ok(t) = &result {
            if let Ok(ticks) = t.ticks_in(domain) {
                let probe = self.probe.lock().unwrap_or_else(|e| e.into_inner()).clone();
                if let Some(probe) = probe {
                    if let Some(reading) = probe.read() {
                        self.log.push(ClockEvent::Now { held: reading.admission_held, sample: ticks });
                    }
                }
            }
        }
        result
    }

    fn wait_until(&self, t: TimePoint) -> Result<(), TimeError> {
        self.inner.wait_until(t)
    }

    fn schedule(&self, t: TimePoint, f: Box<dyn FnOnce(TimePoint) + Send>) -> Result<ScheduleHandle, TimeError> {
        self.inner.schedule(t, f)
    }

    fn cancel(&self, h: ScheduleHandle) -> bool {
        self.inner.cancel(h)
    }
}
