//! Test doubles for the Phase 2 coordinator (spec 06 §12). Never compiled into `src/`.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use ezsdr_kernel::binding::AdmissionCheckRegistry;
use ezsdr_kernel::event::{Action, Event, EventKind, EventSink, Severity};
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::{ClockDomainId, MemoryDomainId, ModuleId, ResourceId};
use ezsdr_kernel::manifest::ArtifactRef;
use ezsdr_kernel::module_api::{
    ActionReceiver, ActionSubmitter, Authority, AuthorityDescriptor, CoerceReport, Endpoint,
    Executor, ExecutorDescriptor, Factories, IslandDecl, Link, LinkDescriptor, ModuleError,
    ModuleErrorKind, ModuleRef, ModuleRegistry, Pacing, PrepareContext, Provider, ProviderInstance,
    Sink, SinkDescriptor, StepOutcome, StopMode, UpdateClass, VocabularyDescriptor,
};
use ezsdr_kernel::plan::{Fragment, PrepareReport};
use ezsdr_kernel::policy::EventKindRegistry;
use ezsdr_kernel::run::StopCause;
use ezsdr_kernel::stream::{
    BackPressure, BlockRef, ContinuityMap, DataLink, DataLinkDecl, DropCarry, PublishOutcome,
};
use ezsdr_kernel::time::{
    ClockDomain, EpochRef, ManualTimeAuthority, Rational, SampleClockHandle, ScheduleHandle,
    TimeAuthority, TimeError, TimePoint,
};

use super::doubles::{
    FailAt, TestExecutor, TestLimitsCheck, TestProvider, id, mref, ns, test_executor_descriptor,
    test_link_descriptor, test_link_module_descriptor, test_provider_descriptor,
    test_sink_descriptor, test_vocabulary,
};
use super::{MemLink, block, header};

/// Shared, ordered observations from coordinator test doubles.
#[derive(Clone, Default)]
pub struct Probe(Arc<(Mutex<Vec<String>>, Condvar)>);

impl Probe {
    /// Creates an empty probe.
    pub fn new() -> Probe {
        Probe::default()
    }

    /// Appends one observation.
    pub fn record(&self, line: impl Into<String>) {
        let (lines, changed) = &*self.0;
        lines
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(line.into());
        changed.notify_all();
    }

    /// Copies every observation in order.
    pub fn lines(&self) -> Vec<String> {
        self.0.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Copies observations with the given prefix in order.
    pub fn with_prefix(&self, prefix: &str) -> Vec<String> {
        self.0
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|line| line.starts_with(prefix))
            .cloned()
            .collect()
    }

    /// Waits for an exact observation without polling.
    pub fn wait_for(&self, line: &str, timeout: std::time::Duration) -> bool {
        self.wait_for_count(line, 1, timeout)
    }

    /// Waits until an observation reaches a count, without polling.
    pub fn wait_for_count(&self, line: &str, count: usize, timeout: std::time::Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        let (lines, changed) = &*self.0;
        let mut lines = lines.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if lines
                .iter()
                .filter(|observed| observed.as_str() == line)
                .count()
                >= count
            {
                return true;
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let (next, result) = changed
                .wait_timeout(lines, remaining)
                .unwrap_or_else(|e| e.into_inner());
            lines = next;
            if result.timed_out()
                && lines
                    .iter()
                    .filter(|observed| observed.as_str() == line)
                    .count()
                    < count
            {
                return false;
            }
        }
    }
}

/// An Authority backed by the Kernel's deterministic manual clock.
pub struct SimAuthority {
    inner: Arc<ManualTimeAuthority>,
    descriptor: AuthorityDescriptor,
    panic_now: bool,
    panic_now_at: Option<usize>,
    panic_schedule: bool,
    panic_cancel: bool,
    now_calls: Arc<AtomicUsize>,
    next_wakeup_gate: Option<Arc<(Mutex<bool>, Condvar)>>,
    next_wakeup_after_gate: Option<TimePoint>,
    next_wakeup_probe: Option<Probe>,
    next_wakeup_calls: Arc<AtomicUsize>,
    next_wakeup_after_gate_returned: AtomicBool,
    panic_next_wakeup: bool,
}

impl SimAuthority {
    /// Allocates and registers a 1 GHz test root, then builds its manual clock.
    pub fn new(
        clocks: &Arc<ezsdr_kernel::time::ClockRegistry>,
        module: ModuleRef,
        pacing: Pacing,
    ) -> (SimAuthority, ClockDomainId) {
        let tick_rate = Rational::new(1_000_000_000, 1).expect("the test root rate is valid");
        let root = clocks.allocate_id().unwrap();
        clocks
            .register(ClockDomain::root(
                root,
                tick_rate,
                EpochRef::Arbitrary {
                    set_by: "test.sim".to_owned(),
                },
            ))
            .expect("the allocated test root is fresh");
        let inner = Arc::new(
            ManualTimeAuthority::new(clocks.clone(), root, &[], pacing)
                .expect("the registered test root is valid"),
        );
        let descriptor = AuthorityDescriptor {
            module,
            governs: vec![root, ClockDomainId::HOST_MONOTONIC],
            pacing,
        };
        (
            SimAuthority {
                inner,
                descriptor,
                panic_now: false,
                panic_now_at: None,
                panic_schedule: false,
                panic_cancel: false,
                now_calls: Arc::new(AtomicUsize::new(0)),
                next_wakeup_gate: None,
                next_wakeup_after_gate: None,
                next_wakeup_probe: None,
                next_wakeup_calls: Arc::new(AtomicUsize::new(0)),
                next_wakeup_after_gate_returned: AtomicBool::new(false),
                panic_next_wakeup: false,
            },
            root,
        )
    }

    /// Returns another handle to the manual clock, for inspecting its current time.
    pub fn manual(&self) -> Arc<ManualTimeAuthority> {
        self.inner.clone()
    }

    /// Makes its `TimeAuthority::now` implementation panic (KC-30).
    pub fn panicking_now(mut self) -> SimAuthority {
        self.panic_now = true;
        self
    }

    /// Panics once on the selected `now` call (KC-30).
    pub fn panicking_now_on_call(mut self, call: usize) -> SimAuthority {
        self.panic_now_at = Some(call);
        self
    }

    /// Blocks cleanup's next-wakeup query until the test releases the gate.
    pub fn blocking_next_wakeup(
        mut self,
        gate: Arc<(Mutex<bool>, Condvar)>,
        after_release: TimePoint,
        probe: &Probe,
    ) -> SimAuthority {
        self.next_wakeup_gate = Some(gate);
        self.next_wakeup_after_gate = Some(after_release);
        self.next_wakeup_probe = Some(probe.clone());
        self
    }

    /// Makes its `next_wakeup` panic (KC-30).
    pub fn panicking_next_wakeup(mut self) -> SimAuthority {
        self.panic_next_wakeup = true;
        self
    }

    /// Makes its `TimeAuthority::schedule` implementation panic (KC-30).
    pub fn panicking_schedule(mut self) -> SimAuthority {
        self.panic_schedule = true;
        self
    }

    /// Makes its `TimeAuthority::cancel` implementation panic on every call but the
    /// first (KC-30).
    pub fn panicking_cancel(mut self) -> SimAuthority {
        self.panic_cancel = true;
        self
    }
}

impl Authority for SimAuthority {
    fn descriptor(&self) -> &AuthorityDescriptor {
        &self.descriptor
    }

    fn time(&self) -> Arc<dyn TimeAuthority> {
        if self.panic_now || self.panic_now_at.is_some() || self.panic_schedule || self.panic_cancel
        {
            Arc::new(FaultingTime {
                inner: self.inner.clone(),
                panic_now: self.panic_now,
                panic_now_at: self.panic_now_at,
                panic_schedule: self.panic_schedule,
                panic_cancel: self.panic_cancel,
                now_calls: self.now_calls.clone(),
                cancel_calls: AtomicUsize::new(0),
            })
        } else {
            self.inner.clone()
        }
    }

    fn next_wakeup(&self) -> Option<TimePoint> {
        if self.panic_next_wakeup {
            panic!("test: panic in Authority::next_wakeup");
        }
        if let Some(gate) = &self.next_wakeup_gate {
            let (released, changed) = &**gate;
            let mut released = released.lock().unwrap_or_else(|e| e.into_inner());
            while !*released {
                released = changed.wait(released).unwrap_or_else(|e| e.into_inner());
            }
            if !self
                .next_wakeup_after_gate_returned
                .swap(true, Ordering::AcqRel)
            {
                if let Some(probe) = &self.next_wakeup_probe {
                    let call = self.next_wakeup_calls.fetch_add(1, Ordering::SeqCst) + 1;
                    probe.record(format!("a:next_wakeup_returned:{call}"));
                }
                return self.next_wakeup_after_gate;
            }
        }
        let wakeup = self.inner.next_due().inspect(|t| {
            let _ = self.inner.advance_to(*t);
        });
        if let Some(probe) = &self.next_wakeup_probe {
            let call = self.next_wakeup_calls.fetch_add(1, Ordering::SeqCst) + 1;
            probe.record(format!("a:next_wakeup_returned:{call}"));
        }
        wakeup
    }
}

struct FaultingTime {
    inner: Arc<ManualTimeAuthority>,
    panic_now: bool,
    panic_now_at: Option<usize>,
    panic_schedule: bool,
    panic_cancel: bool,
    now_calls: Arc<AtomicUsize>,
    cancel_calls: AtomicUsize,
}

impl TimeAuthority for FaultingTime {
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
        let call = self.now_calls.fetch_add(1, Ordering::SeqCst) + 1;
        if self.panic_now || self.panic_now_at == Some(call) {
            panic!("test: panic in TimeAuthority::now");
        }
        self.inner.now(domain)
    }
    fn wait_until(&self, t: TimePoint) -> Result<(), TimeError> {
        self.inner.wait_until(t)
    }
    fn schedule(
        &self,
        t: TimePoint,
        f: Box<dyn FnOnce(TimePoint) + Send>,
    ) -> Result<ScheduleHandle, TimeError> {
        if self.panic_schedule {
            panic!("test: panic in TimeAuthority::schedule");
        }
        self.inner.schedule(t, f)
    }
    fn cancel(&self, handle: ScheduleHandle) -> bool {
        // The first cancel succeeds, every later one panics: a freeze that stops at
        // its first handle, or reports only several panics, then reports none.
        if self.panic_cancel && self.cancel_calls.fetch_add(1, Ordering::SeqCst) > 0 {
            panic!("test: panic in TimeAuthority::cancel");
        }
        self.inner.cancel(handle)
    }
}

/// A stepped Provider with injectable time, events, links and failures.
pub struct SteppedProvider {
    pub name: String,
    pub inner: TestProvider,
    pub probe: Probe,
    pub wakeups: Vec<i64>,
    pub declare: Vec<(String, u64, u64)>,
    pub register_at_arm: Vec<String>,
    pub emit: Option<(EventKind, Severity, i64)>,
    /// One control-path event emitted from `stop`, at the stop instant (#66).
    pub stop_emit: Option<(EventKind, Severity)>,
    pub panic_in_step: bool,
    pub device_lost_at: Option<i64>,
    pub step_error_at: Option<i64>,
    pub reschedule_forever: bool,
    pub wedge_in_stop: bool,
    pub tail_blocks: u32,
    pub publish_every: Option<i64>,
    pub time: Option<Arc<dyn TimeAuthority>>,
    pub clocks: Option<Arc<ezsdr_kernel::time::ClockRegistry>>,
    pub events: Option<Arc<dyn EventSink>>,
    pub actions: Option<Arc<dyn ActionReceiver>>,
    pub outs: Vec<Arc<dyn DataLink>>,
    pub handles: Vec<SampleClockHandle>,
    pub stopped_at: Option<i64>,
    pub emitted: bool,
    /// Bodies of `test.custom` at `info` emitted on the hot path just before `emit`,
    /// which then also goes through the hot path (RS-36).
    pub flood: Option<usize>,
    pub lost_reported: bool,
    pub declare_roots: BTreeMap<String, ClockDomainId>,
    pub next_publish: Option<i64>,
    pub next_tail: Option<i64>,
    pub prepare_abort: Option<String>,
    pub block_step_at: Option<i64>,
    pub step_gate: Option<Arc<(Mutex<bool>, Condvar)>>,
    pub inputs: Option<Arc<dyn ezsdr_kernel::module_api::InputStore>>,
    pub settle_fidelity: Option<ezsdr_kernel::module_api::Fidelity>,
    /// How far after `arm`'s instant the clocks registered there begin.
    pub arm_origin_after: i64,
}

impl SteppedProvider {
    /// Wraps a Phase 1 Provider and marks its instance as stepped.
    pub fn new(name: &str, inner: TestProvider, probe: &Probe) -> SteppedProvider {
        SteppedProvider {
            name: name.to_owned(),
            inner: inner.stepped(),
            probe: probe.clone(),
            wakeups: Vec::new(),
            declare: Vec::new(),
            register_at_arm: Vec::new(),
            emit: None,
            stop_emit: None,
            panic_in_step: false,
            device_lost_at: None,
            step_error_at: None,
            reschedule_forever: false,
            wedge_in_stop: false,
            tail_blocks: 0,
            publish_every: None,
            time: None,
            clocks: None,
            events: None,
            actions: None,
            outs: Vec::new(),
            handles: Vec::new(),
            stopped_at: None,
            emitted: false,
            flood: None,
            lost_reported: false,
            declare_roots: BTreeMap::new(),
            next_publish: None,
            next_tail: None,
            prepare_abort: None,
            block_step_at: None,
            step_gate: None,
            inputs: None,
            settle_fidelity: None,
            arm_origin_after: 0,
        }
    }

    /// Sets its instance's fidelity to `fidelity` inside `prepare` (MA-10 as KB-2 amends it).
    pub fn settling_fidelity(mut self, fidelity: ezsdr_kernel::module_api::Fidelity) -> SteppedProvider {
        self.settle_fidelity = Some(fidelity);
        self
    }

    /// Schedules these primary-root instants at `start`.
    pub fn with_wakeups(mut self, ticks: &[i64]) -> SteppedProvider {
        self.wakeups = ticks.to_vec();
        self
    }

    /// Declares a SampleClock on the Authority's primary root at `prepare`.
    pub fn declaring(mut self, stream: &str, num: u64, den: u64) -> SteppedProvider {
        self.declare.push((stream.to_owned(), num, den));
        self
    }

    /// Declares a SampleClock on the supplied root at `prepare`.
    pub fn declaring_on(
        mut self,
        stream: &str,
        root: ClockDomainId,
        num: u64,
        den: u64,
    ) -> SteppedProvider {
        self.declare.push((stream.to_owned(), num, den));
        self.declare_roots.insert(stream.to_owned(), root);
        self
    }

    /// Registers the named declared clock at `arm`.
    pub fn registering_at_arm(mut self, stream: &str) -> SteppedProvider {
        self.register_at_arm.push(stream.to_owned());
        self
    }

    /// Makes the clocks registered at `arm` begin `ticks` primary-root ticks later.
    pub fn with_arm_origin_after(mut self, ticks: i64) -> SteppedProvider {
        self.arm_origin_after = ticks;
        self
    }

    /// Emits one control-path event at or after the given primary-root tick.
    pub fn emitting(mut self, kind: &str, severity: Severity, at: i64) -> SteppedProvider {
        self.emit = Some((
            EventKind::parse(kind).expect("the test event kind is valid"),
            severity,
            at,
        ));
        self
    }

    /// Emits one control-path event from `stop`, at the stop instant (#66).
    pub fn emitting_in_stop(mut self, kind: &str, severity: Severity) -> SteppedProvider {
        self.stop_emit = Some((EventKind::parse(kind).expect("the test event kind is valid"), severity));
        self
    }

    /// Emits `n` hot-path bodies of `test.custom` at `info`, then the `emitting`
    /// event on the hot path too, in the same step: with `n` the ring's depth its body
    /// is dropped (RS-36).
    pub fn flooding(mut self, n: usize) -> SteppedProvider {
        self.flood = Some(n);
        self
    }

    /// Panics on the next step.
    pub fn panicking_in_step(mut self) -> SteppedProvider {
        self.panic_in_step = true;
        self
    }

    /// Reports DeviceLost on the first step at or after `t`.
    pub fn device_lost_at(mut self, t: i64) -> SteppedProvider {
        self.device_lost_at = Some(t);
        self
    }

    /// Reports a rejected step error on the first step at or after `t`.
    pub fn step_error_at(mut self, t: i64) -> SteppedProvider {
        self.step_error_at = Some(t);
        self
    }

    /// Reschedules a no-op callback at every step instant.
    pub fn rescheduling_forever(mut self) -> SteppedProvider {
        self.reschedule_forever = true;
        self
    }

    /// Blocks forever in `stop`, for the cleanup timeout test.
    pub fn wedged_in_stop(mut self) -> SteppedProvider {
        self.wedge_in_stop = true;
        self
    }

    /// Requests an Abort through the prepare-time ActionSubmitter (KC-24).
    pub fn aborting_in_prepare(mut self, cause: &str) -> SteppedProvider {
        self.prepare_abort = Some(cause.to_owned());
        self
    }

    /// Blocks one step until the test releases its gate (KA-12).
    pub fn blocking_step_at(
        mut self,
        tick: i64,
        gate: Arc<(Mutex<bool>, Condvar)>,
    ) -> SteppedProvider {
        self.block_step_at = Some(tick);
        self.step_gate = Some(gate);
        self
    }

    /// Publishes this many tail blocks after an orderly stop.
    pub fn with_tail(mut self, n: u32) -> SteppedProvider {
        self.tail_blocks = n;
        self
    }

    /// Publishes once per interval until `stop`.
    pub fn publishing_every(mut self, n: i64) -> SteppedProvider {
        self.publish_every = Some(n);
        self
    }

    fn record(&self, event: impl AsRef<str>) {
        self.probe
            .record(format!("{}:{}", self.name, event.as_ref()));
    }

    fn schedule_noop(&self, ticks: i64) -> Result<(), ModuleError> {
        let time = self
            .time
            .as_ref()
            .ok_or_else(|| ModuleError::rejected("test: no time handle"))?;
        time.schedule(TimePoint::new(time.primary_root(), ticks), Box::new(|_| {}))
            .map(|_| ())
            .map_err(|e| ModuleError::rejected(format!("test: {e}")))
    }

    fn publish(&self, until: TimePoint) -> Result<(), ModuleError> {
        let clocks = self
            .clocks
            .as_ref()
            .ok_or_else(|| ModuleError::rejected("test: no clock registry"))?;
        let primary = self
            .time
            .as_ref()
            .ok_or_else(|| ModuleError::rejected("test: no time handle"))?
            .primary_root();
        let domain = self
            .handles
            .first()
            .filter(|handle| clocks.is_registered(handle.id))
            .map(|handle| handle.id)
            .unwrap_or(primary);
        let sample = block(header(TimePoint::new(domain, until.ticks), 10, 1));
        for link in &self.outs {
            let _ = link.publish(sample.clone());
        }
        self.record(format!("published:{}", until.ticks));
        Ok(())
    }

    fn drain_actions(&self, until: TimePoint) -> bool {
        let mut pending = Vec::new();
        if let Some(actions) = &self.actions {
            while let Some(action) = actions.recv() {
                pending.push(action);
            }
        }
        let progressed = !pending.is_empty();
        for action in pending {
            let target = action
                .target()
                .map(|target| target.path.clone())
                .unwrap_or_else(|| "-".to_owned());
            let variant = match &action {
                Action::TxBurst { .. } => "TxBurst",
                Action::SetTimer { .. } => "SetTimer",
                Action::UpdateParameter { .. } => "UpdateParameter",
                Action::PeripheralCommand { .. } => "PeripheralCommand",
                Action::Emit { .. } => "Emit",
                Action::Stop { .. } => "Stop",
                Action::Abort { .. } => "Abort",
            };
            self.record(format!("action:{variant}:{target}@{}", until.ticks));
            match action {
                Action::UpdateParameter { key, value, .. } => {
                    let value = serde_json::to_string(&value).unwrap_or_else(|_| "null".to_owned());
                    self.record(format!("update:{key}={value}"));
                }
                Action::TxBurst { at, waveform, .. } => {
                    self.record(format!("burst_at:{}", at.time_point.ticks));
                    let found = self
                        .inputs
                        .as_ref()
                        .and_then(|inputs| inputs.get(&waveform.hash))
                        .map_or_else(|| "missing".to_owned(), |bytes| bytes.len().to_string());
                    self.record(format!("burst_input:{found}"));
                }
                _ => {}
            }
        }
        progressed
    }
}

impl Provider for SteppedProvider {
    fn instance(&self) -> &ProviderInstance {
        self.inner.instance()
    }

    fn coerce(
        &self,
        request: &ezsdr_kernel::module_api::Requested,
    ) -> Result<CoerceReport, ModuleError> {
        self.inner.coerce(request)
    }

    fn prepare(&mut self, f: &Fragment, ctx: PrepareContext) -> Result<PrepareReport, ModuleError> {
        self.record(format!("prepare:{}", f.id));
        if let Some(cause) = self.prepare_abort.take() {
            ctx.actions_out
                .submit(Action::Abort {
                    cause: StopCause::Abort { message: cause },
                })
                .map_err(|violations| {
                    ModuleError::rejected(format!("test: Abort refused: {violations:?}"))
                })?;
        }
        self.time = Some(ctx.time.clone());
        self.clocks = Some(ctx.clocks.clone());
        self.events = Some(ctx.events.clone());
        self.actions = Some(ctx.actions.clone());
        self.inputs = Some(ctx.inputs.clone());
        if let Some(fidelity) = self.settle_fidelity {
            self.inner.set_fidelity(fidelity);
        }
        self.outs.clear();
        self.handles.clear();
        for attached in &ctx.links {
            let (direction, link) = match &attached.endpoint {
                Endpoint::StreamIn(_) => ("in", None),
                Endpoint::StreamOut(link) => ("out", Some(link.clone())),
                Endpoint::EventIn => ("in", None),
                Endpoint::EventOut => ("out", None),
            };
            if let Some(link) = link {
                self.outs.push(link);
            }
            self.record(format!(
                "links:{}.{}:{direction}",
                attached.component, attached.port
            ));
        }
        for (stream, num, den) in &self.declare {
            let stream_id = ResourceId::parse(stream)
                .map_err(|e| ModuleError::rejected(format!("test: {e}")))?;
            let root = self
                .declare_roots
                .get(stream)
                .copied()
                .unwrap_or_else(|| ctx.time.primary_root());
            let ratio = Rational::new(*num, *den)
                .map_err(|e| ModuleError::rejected(format!("test: {e}")))?;
            let handle = ctx
                .clocks
                .declare_sample_clock(stream_id, root, ratio)
                .map_err(|e| ModuleError::rejected(format!("test: {e}")))?;
            self.handles.push(handle);
        }
        self.inner.prepare(f, ctx)
    }

    fn arm(&mut self) -> Result<(), ModuleError> {
        self.record("arm");
        self.inner.fail_if(FailAt::Arm)?;
        let Some(time) = &self.time else {
            return Err(ModuleError::rejected("test: no time handle"));
        };
        let Some(clocks) = &self.clocks else {
            return Err(ModuleError::rejected("test: no clock registry"));
        };
        let origin = time
            .now(time.primary_root())
            .map_err(|e| ModuleError::rejected(format!("test: {e}")))?
            .ticks
            + self.arm_origin_after;
        for stream in &self.register_at_arm {
            let stream_id = ResourceId::parse(stream)
                .map_err(|e| ModuleError::rejected(format!("test: {e}")))?;
            let handles: Vec<_> = self
                .handles
                .iter()
                .filter(|h| h.stream == stream_id)
                .collect();
            if handles.is_empty() {
                return Err(ModuleError::rejected(format!(
                    "test: no declared clock for {stream}"
                )));
            }
            for handle in handles {
                clocks
                    .register_sample_clock(handle, origin)
                    .map_err(|e| ModuleError::rejected(format!("test: {e}")))?;
            }
        }
        Ok(())
    }

    fn start(&mut self, at: Option<TimePoint>) -> Result<(), ModuleError> {
        self.record(format!(
            "start:{}",
            at.map(|t| t.ticks.to_string())
                .unwrap_or_else(|| "none".to_owned())
        ));
        self.inner.fail_if(FailAt::Start)?;
        for ticks in self.wakeups.clone() {
            self.schedule_noop(ticks)?;
        }
        if let Some(interval) = self.publish_every {
            let time = self
                .time
                .as_ref()
                .ok_or_else(|| ModuleError::rejected("test: no time handle"))?;
            let now = time
                .now(time.primary_root())
                .map_err(|e| ModuleError::rejected(format!("test: {e}")))?
                .ticks;
            let next = now
                .checked_add(interval)
                .ok_or_else(|| ModuleError::rejected("test: publish time overflow"))?;
            self.next_publish = Some(next);
            self.schedule_noop(next)?;
        }
        Ok(())
    }

    fn step(&mut self, until: TimePoint) -> Result<StepOutcome, ModuleError> {
        self.record(format!("step:{}", until.ticks));
        if self.block_step_at == Some(until.ticks) {
            if let Some(gate) = &self.step_gate {
                let (released, changed) = &**gate;
                let mut released = released.lock().unwrap_or_else(|e| e.into_inner());
                while !*released {
                    released = changed.wait(released).unwrap_or_else(|e| e.into_inner());
                }
            }
            self.block_step_at = None;
            self.record(format!("released_step:{}", until.ticks));
        }
        let now = self
            .time
            .as_ref()
            .ok_or_else(|| ModuleError::rejected("test: no time handle"))?
            .now(self.time.as_ref().expect("checked above").primary_root())
            .map_err(|e| ModuleError::rejected(format!("test: {e}")))?;
        self.record(format!("now:{}", now.ticks));
        if self.lost_reported {
            return Ok(StepOutcome { progressed: false });
        }
        let mut progressed = self.drain_actions(until);
        if self.panic_in_step {
            panic!("test: panic in step");
        }
        if !self.lost_reported && self.device_lost_at.is_some_and(|t| until.ticks >= t) {
            self.lost_reported = true;
            return Err(ModuleError {
                kind: ModuleErrorKind::DeviceLost,
                message: "test: device lost".to_owned(),
                detail: serde_json::Value::Null,
            });
        }
        if self.step_error_at.is_some_and(|t| until.ticks >= t) {
            self.step_error_at = None;
            return Err(ModuleError::rejected("test: step error"));
        }
        if !self.emitted {
            if let Some((kind, severity, at)) = &self.emit {
                if until.ticks >= *at {
                    let event = Event {
                        source: self.instance().id.clone(),
                        time: until,
                        severity: *severity,
                        kind: kind.clone(),
                        payload: serde_json::json!({}),
                    };
                    let sink = self
                        .events
                        .as_ref()
                        .ok_or_else(|| ModuleError::rejected("test: no event sink"))?;
                    if let Some(n) = self.flood {
                        let noise = sink.resolve(
                            &event.source,
                            &EventKind::parse("test.custom").expect("a valid kind"),
                        );
                        for _ in 0..n {
                            sink.emit(noise, until, Severity::Info, &[])
                                .map_err(|e| ModuleError::rejected(format!("test: {e}")))?;
                        }
                        let handle = sink.resolve(&event.source, &event.kind);
                        sink.emit(handle, until, event.severity, &[])
                            .map_err(|e| ModuleError::rejected(format!("test: {e}")))?;
                    } else {
                        sink.emit_control(event)
                            .map_err(|e| ModuleError::rejected(format!("test: {e}")))?;
                    }
                    self.emitted = true;
                }
            }
        }
        if self.stopped_at.is_none() && self.next_publish == Some(until.ticks) {
            self.publish(until)?;
            progressed = true;
            if let Some(interval) = self.publish_every {
                let next = until
                    .ticks
                    .checked_add(interval)
                    .ok_or_else(|| ModuleError::rejected("test: publish time overflow"))?;
                self.next_publish = Some(next);
                self.schedule_noop(next)?;
            }
        }
        if self.stopped_at.is_some() && self.tail_blocks > 0 && self.next_tail == Some(until.ticks)
        {
            self.publish(until)?;
            progressed = true;
            self.tail_blocks -= 1;
            if self.tail_blocks > 0 {
                let next = until
                    .ticks
                    .checked_add(10)
                    .ok_or_else(|| ModuleError::rejected("test: tail time overflow"))?;
                self.next_tail = Some(next);
                self.schedule_noop(next)?;
            } else {
                self.next_tail = None;
            }
        }
        if self.reschedule_forever {
            self.schedule_noop(until.ticks)?;
        }
        Ok(StepOutcome { progressed })
    }

    fn stop(&mut self, mode: StopMode) -> Result<(), ModuleError> {
        self.record(format!("stop:{mode:?}"));
        if self.wedge_in_stop {
            loop {
                std::thread::park();
            }
        }
        self.inner.fail_if(FailAt::Stop)?;
        let time = self
            .time
            .as_ref()
            .ok_or_else(|| ModuleError::rejected("test: no time handle"))?;
        self.stopped_at = Some(
            time.now(time.primary_root())
                .map_err(|e| ModuleError::rejected(format!("test: {e}")))?
                .ticks,
        );
        self.next_publish = None;
        if let Some((kind, severity)) = self.stop_emit.take() {
            let event = Event {
                source: self.instance().id.clone(),
                time: TimePoint::new(time.primary_root(), self.stopped_at.expect("just set")),
                severity,
                kind,
                payload: serde_json::json!({}),
            };
            self.events
                .as_ref()
                .ok_or_else(|| ModuleError::rejected("test: no event sink"))?
                .emit_control(event)
                .map_err(|e| ModuleError::rejected(format!("test: {e}")))?;
        }
        if mode == StopMode::Orderly && self.tail_blocks > 0 {
            let next = self
                .stopped_at
                .expect("just set")
                .checked_add(10)
                .ok_or_else(|| ModuleError::rejected("test: tail time overflow"))?;
            self.next_tail = Some(next);
            self.schedule_noop(next)?;
        }
        Ok(())
    }

    fn cleanup(&mut self) {
        self.record("cleanup");
        self.time = None;
        self.clocks = None;
        self.events = None;
        self.actions = None;
        self.outs.clear();
        self.handles.clear();
    }
}

/// A Sink that records received blocks and returns declared artifact spans.
pub struct RecordingSink {
    pub name: String,
    pub descriptor: SinkDescriptor,
    pub probe: Probe,
    pub ins: Vec<Arc<dyn DataLink>>,
    pub received: Vec<ezsdr_kernel::stream::BlockHeader>,
    pub spans: Vec<(i64, i64)>,
    pub fail_stop: bool,
    pub primary: Option<ClockDomainId>,
    /// Records `<name>:step_on:<thread>` for every step (spec 19 §0).
    pub record_threads: bool,
    /// Every step reports `progressed` (KC-46's livelock test).
    pub always_progress: bool,
    /// The first step at or after this primary-root tick fails with this kind.
    pub fail_step: Option<(i64, ModuleErrorKind)>,
    /// The first step at or after this primary-root tick panics (#65).
    pub panic_step: Option<i64>,
}

impl RecordingSink {
    /// Creates a recorder for the standard cf32 contract.
    pub fn new(name: &str, probe: &Probe) -> RecordingSink {
        RecordingSink {
            name: name.to_owned(),
            descriptor: SinkDescriptor {
                module: mref("ezsdr.test.sink"),
                kind: ns("test.recorder"),
                memory_domains: vec![MemoryDomainId::local(0)],
                contracts: vec![
                    ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.cf32")
                        .expect("SC-4 id"),
                ],
                artifact_kinds: vec![ns("test.capture")],
            },
            probe: probe.clone(),
            ins: Vec::new(),
            received: Vec::new(),
            spans: Vec::new(),
            fail_stop: false,
            primary: None,
            record_threads: false,
            always_progress: false,
            fail_step: None,
            panic_step: None,
        }
    }

    /// Records the thread each step runs on (spec 19 §0).
    pub fn recording_threads(mut self) -> RecordingSink {
        self.record_threads = true;
        self
    }

    /// Makes every step report `progressed` (spec 19 §0).
    pub fn always_progressing(mut self) -> RecordingSink {
        self.always_progress = true;
        self
    }

    /// Fails the first step at or after `tick` with an error of `kind` (spec 19 §0).
    pub fn failing_step_at(mut self, tick: i64, kind: ModuleErrorKind) -> RecordingSink {
        self.fail_step = Some((tick, kind));
        self
    }

    /// Panics in the first step at or after `tick`, once (#65).
    pub fn panicking_step_at(mut self, tick: i64) -> RecordingSink {
        self.panic_step = Some(tick);
        self
    }

    /// Sets the primary-root continuity spans returned by `stop`.
    pub fn returning_spans(mut self, spans: &[(i64, i64)]) -> RecordingSink {
        self.spans = spans.to_vec();
        self
    }

    /// Makes `stop` fail.
    pub fn failing_stop(mut self) -> RecordingSink {
        self.fail_stop = true;
        self
    }

    fn record(&self, event: impl AsRef<str>) {
        self.probe
            .record(format!("{}:{}", self.name, event.as_ref()));
    }
}

impl Sink for RecordingSink {
    fn descriptor(&self) -> &SinkDescriptor {
        &self.descriptor
    }

    fn prepare(&mut self, f: &Fragment, ctx: PrepareContext) -> Result<PrepareReport, ModuleError> {
        self.record(format!("prepare:{}", f.id));
        self.primary = Some(ctx.time.primary_root());
        self.ins.clear();
        for attached in &ctx.links {
            let (direction, link) = match &attached.endpoint {
                Endpoint::StreamIn(link) => ("in", Some(link.clone())),
                Endpoint::StreamOut(_) => ("out", None),
                Endpoint::EventIn => ("in", None),
                Endpoint::EventOut => ("out", None),
            };
            if let Some(link) = link {
                self.ins.push(link);
            }
            self.record(format!(
                "links:{}.{}:{direction}",
                attached.component, attached.port
            ));
        }
        Ok(PrepareReport {
            fragment: f.id.clone(),
            effective: BTreeMap::new(),
            coercions: Vec::new(),
            warnings: Vec::new(),
        })
    }

    fn arm(&mut self) -> Result<(), ModuleError> {
        self.record("arm");
        Ok(())
    }

    fn start(&mut self) -> Result<(), ModuleError> {
        self.record("start");
        Ok(())
    }

    fn step(&mut self, until: TimePoint) -> Result<StepOutcome, ModuleError> {
        self.record(format!("step:{}", until.ticks));
        if self.record_threads {
            let thread = std::thread::current();
            self.record(format!("step_on:{:?}:{}", thread.id(), thread.name().unwrap_or("-")));
        }
        if let Some((tick, kind)) = self.fail_step {
            if until.ticks >= tick {
                self.fail_step = None;
                return Err(ModuleError {
                    kind,
                    message: "test: sink step failed".to_owned(),
                    detail: serde_json::Value::Null,
                });
            }
        }
        if self.panic_step.is_some_and(|tick| until.ticks >= tick) {
            self.panic_step = None;
            panic!("test: panic in sink step");
        }
        let mut progressed = self.always_progress;
        for link in &self.ins {
            while let Some((block, _)) = link.receive() {
                let header = block.header().clone();
                self.record(format!("block:{}", header.first_sample_time.ticks));
                self.received.push(header);
                progressed = true;
            }
        }
        Ok(StepOutcome { progressed })
    }

    fn stop(&mut self, mode: StopMode) -> Result<Vec<ArtifactRef>, ModuleError> {
        self.record(format!("stop:{mode:?}"));
        if self.fail_stop {
            return Err(ModuleError::rejected("test: sink stop failed"));
        }
        let primary = self
            .primary
            .ok_or_else(|| ModuleError::rejected("test: no primary root"))?;
        let spans = if self.spans.is_empty() {
            vec![None]
        } else {
            self.spans.iter().copied().map(Some).collect()
        };
        Ok(spans
            .into_iter()
            .enumerate()
            .map(|(index, span)| {
                let artifact_id = format!("{}_{}", self.name, index);
                let continuity = span
                    .map(|(first, end)| ContinuityMap {
                        domain: primary,
                        channels: 1,
                        valid: vec![vec![]],
                        gaps: vec![],
                        channel_gaps: vec![],
                        first: TimePoint::new(primary, first),
                        end: TimePoint::new(primary, end),
                    })
                    .into_iter()
                    .collect();
                ArtifactRef {
                    id: id(&artifact_id),
                    kind: ns("test.capture"),
                    uri: format!("memory://{artifact_id}"),
                    hash: ContentHash::of_bytes(artifact_id.as_bytes()),
                    size_bytes: 0,
                    partial: mode == StopMode::Abort,
                    marks: Vec::new(),
                    continuity,
                }
            })
            .collect())
    }

    fn cleanup(&mut self) {
        self.record("cleanup");
        self.ins.clear();
    }
}

/// An Executor that can submit one Action through the supplied Run interface.
pub struct ProbeExecutor {
    pub name: String,
    pub descriptor: ExecutorDescriptor,
    pub probe: Probe,
    pub submit: Option<Action>,
    pub out: Option<Arc<dyn ActionSubmitter>>,
    pub submitted: bool,
    submit_at: Option<i64>,
    abort_in_stop: Option<String>,
    pub event: Option<(ResourceId, EventKind, Severity)>,
    events: Option<Arc<dyn EventSink>>,
    event_emitted: bool,
    coercion_counter_snapshot: Option<(Arc<AtomicU64>, Arc<AtomicU64>)>,
}

impl ProbeExecutor {
    /// Creates a recorder with the same descriptor as the Phase 1 test Executor.
    pub fn new(name: &str, probe: &Probe) -> ProbeExecutor {
        ProbeExecutor {
            name: name.to_owned(),
            descriptor: TestExecutor::new(MemoryDomainId::local(0))
                .descriptor()
                .clone(),
            probe: probe.clone(),
            submit: None,
            out: None,
            submitted: false,
            submit_at: None,
            abort_in_stop: None,
            event: None,
            events: None,
            event_emitted: false,
            coercion_counter_snapshot: None,
        }
    }

    /// Submits the Action on its first step.
    pub fn submitting(mut self, action: Action) -> ProbeExecutor {
        self.submit = Some(action);
        self.submit_at = None;
        self
    }

    /// Captures a Provider's coercion count immediately before this Executor submits.
    pub fn snapshot_coercions_before_submit(
        mut self,
        counter: Arc<AtomicU64>,
        snapshot: Arc<AtomicU64>,
    ) -> ProbeExecutor {
        self.coercion_counter_snapshot = Some((counter, snapshot));
        self
    }

    /// Submits the Action on the first step at or after `tick`.
    pub fn submitting_at(mut self, tick: i64, action: Action) -> ProbeExecutor {
        self.submit = Some(action);
        self.submit_at = Some(tick);
        self
    }

    /// Submits `Abort` from an orderly `stop`, escalating the cleanup (KC-32).
    pub fn aborting_in_orderly_stop(mut self, message: &str) -> ProbeExecutor {
        self.abort_in_stop = Some(message.to_owned());
        self
    }

    /// Emits once from a caller-selected source during its first step (KC-8).
    pub fn emitting_from(mut self, source: &str, kind: &str, severity: Severity) -> ProbeExecutor {
        self.event = Some((
            ResourceId::parse(source).expect("test event source is valid"),
            EventKind::parse(kind).expect("test event kind is valid"),
            severity,
        ));
        self
    }

    fn record(&self, event: impl AsRef<str>) {
        self.probe
            .record(format!("{}:{}", self.name, event.as_ref()));
    }
}

impl Executor for ProbeExecutor {
    fn descriptor(&self) -> &ExecutorDescriptor {
        &self.descriptor
    }

    fn prepare(
        &mut self,
        island: &IslandDecl,
        ctx: PrepareContext,
    ) -> Result<PrepareReport, ModuleError> {
        let names = ctx
            .components
            .keys()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        self.record(format!("prepare:island_{}:{names}", island.id.local));
        self.out = Some(ctx.actions_out.clone());
        self.events = Some(ctx.events.clone());
        Ok(PrepareReport {
            fragment: id(&format!("island_{}", island.id.local)),
            effective: BTreeMap::new(),
            coercions: Vec::new(),
            warnings: Vec::new(),
        })
    }

    fn arm(&mut self) -> Result<(), ModuleError> {
        self.record("arm");
        Ok(())
    }

    fn start(&mut self) -> Result<(), ModuleError> {
        self.record("start");
        Ok(())
    }

    fn step(&mut self, until: TimePoint) -> Result<StepOutcome, ModuleError> {
        self.record(format!("step:{}", until.ticks));
        if !self.event_emitted {
            if let Some((source, kind, severity)) = &self.event {
                self.events
                    .as_ref()
                    .ok_or_else(|| ModuleError::rejected("test: no event sink"))?
                    .emit_control(Event {
                        source: source.clone(),
                        time: until,
                        severity: *severity,
                        kind: kind.clone(),
                        payload: serde_json::json!({}),
                    })
                    .map_err(|error| ModuleError::rejected(format!("test: {error}")))?;
                self.event_emitted = true;
            }
        }
        let due = !self.submitted
            && self.submit.is_some()
            && self.submit_at.is_none_or(|tick| until.ticks >= tick);
        if due {
            let action = self.submit.take().expect("checked above");
            let out = self
                .out
                .as_ref()
                .ok_or_else(|| ModuleError::rejected("test: no Action submitter"))?;
            if let Some((counter, snapshot)) = &self.coercion_counter_snapshot {
                snapshot.store(counter.load(Ordering::SeqCst), Ordering::SeqCst);
            }
            match out.submit(action) {
                Ok(id) => self.record(format!("submit:ok:{}", id.0)),
                Err(violations) => {
                    if let Some(first) = violations.first() {
                        self.record(format!("submit:err:{}:{}", first.check, first.reason));
                    } else {
                        self.record("submit:err:unknown:no violation detail");
                    }
                }
            }
            self.submitted = true;
        }
        Ok(StepOutcome { progressed: due })
    }

    fn stop(&mut self, mode: StopMode) -> Result<(), ModuleError> {
        self.record(format!("stop:{mode:?}"));
        if mode == StopMode::Orderly {
            if let (Some(message), Some(out)) = (self.abort_in_stop.take(), &self.out) {
                let _ = out.submit(Action::Abort {
                    cause: StopCause::Abort { message },
                });
            }
        }
        Ok(())
    }

    fn cleanup(&mut self) {
        self.record("cleanup");
        self.out = None;
    }
}

/// A Link Module backed by the Phase 1 in-memory DataLink.
pub struct TestLinkModule {
    pub descriptor: LinkDescriptor,
    pub probe: Probe,
    pub panic_descriptor: bool,
    pub panic_drops: bool,
    pub seed_drop: bool,
}

impl TestLinkModule {
    /// Uses the fixture descriptor.
    pub fn new(probe: &Probe) -> TestLinkModule {
        TestLinkModule {
            descriptor: test_link_descriptor(),
            probe: probe.clone(),
            panic_descriptor: false,
            panic_drops: false,
            seed_drop: false,
        }
    }

    /// Replaces the descriptor, for refusal tests.
    pub fn with_descriptor(mut self, descriptor: LinkDescriptor) -> TestLinkModule {
        self.descriptor = descriptor;
        self
    }

    /// Panics when the coordinator reads the descriptor (KC-30).
    pub fn panicking_descriptor(mut self) -> TestLinkModule {
        self.panic_descriptor = true;
        self
    }

    /// Returns a DataLink whose `drops()` panics during cleanup (KA-18).
    pub fn panicking_drops(mut self) -> TestLinkModule {
        self.panic_drops = true;
        self
    }

    /// Seeds one DropOldest eviction so the coordinator's Manifest path is exercised (KA-18).
    pub fn seeding_one_drop(mut self) -> TestLinkModule {
        self.seed_drop = true;
        self
    }
}

impl Link for TestLinkModule {
    fn descriptor(&self) -> &LinkDescriptor {
        if self.panic_descriptor {
            panic!("test: panic in Link descriptor");
        }
        &self.descriptor
    }

    fn create(&self, decl: &DataLinkDecl) -> Result<Arc<dyn DataLink>, ModuleError> {
        self.probe.record(format!(
            "link:create:{}.{}->{}.{}",
            decl.from.component, decl.from.port, decl.to.component, decl.to.port
        ));
        let link = MemLink::new(decl.policy, decl.capacity);
        if self.seed_drop {
            let root = ClockDomainId::local(0);
            assert_eq!(link.publish(block(header(TimePoint::new(root, 0), 10, 1))), PublishOutcome::Accepted);
            assert_eq!(link.publish(block(header(TimePoint::new(root, 10), 10, 1))), PublishOutcome::DroppedOldest);
        }
        if self.panic_drops {
            Ok(Arc::new(PanickingDropsLink(link)))
        } else {
            Ok(Arc::new(link))
        }
    }
}

struct PanickingDropsLink(MemLink);

impl DataLink for PanickingDropsLink {
    fn publish(&self, block: BlockRef) -> PublishOutcome {
        self.0.publish(block)
    }
    fn receive(&self) -> Option<(BlockRef, DropCarry)> {
        self.0.receive()
    }
    fn drops(&self) -> u64 {
        panic!("test: panic in DataLink drops")
    }
    fn take_drop_carry(&self) -> DropCarry {
        self.0.take_drop_carry()
    }
    fn policy(&self) -> BackPressure {
        self.0.policy()
    }
}

/// The Phase 1 fixture Module registry shared by coordinator tests.
pub fn run_registry() -> ezsdr_kernel::module_api::ModuleRegistry {
    fixture_registry(test_vocabulary())
}

/// Registry with a valid ModuleId that is not a valid Manifest Namespace (KA-13).
pub fn run_registry_non_namespace_provider() -> ModuleRegistry {
    let mut registry = ModuleRegistry::new();
    registry
        .register_vocabulary(test_vocabulary())
        .expect("fresh fixture Vocabulary");
    let mut descriptor = test_provider_descriptor();
    descriptor.id = ModuleId::parse("TestProvider").expect("valid ModuleId");
    registry
        .register(
            descriptor,
            Factories {
                provider: true,
                authority: true,
                ..Factories::default()
            },
        )
        .expect("register Provider and Authority");
    registry
}

/// Kernel and `test` Vocabulary event kinds for coordinator fixtures.
pub fn run_kinds() -> EventKindRegistry {
    let mut kinds = EventKindRegistry::with_kernel_kinds();
    for decl in test_vocabulary().event_kinds {
        kinds
            .register(Some(ns("test")), decl)
            .expect("fixture event kind is unique");
    }
    kinds
}

/// The fixture `test.limits` check, optionally registered.
pub fn run_checks(with_limits: bool) -> AdmissionCheckRegistry {
    let mut checks = AdmissionCheckRegistry::new();
    if with_limits {
        checks.register(Arc::new(TestLimitsCheck::new()));
    }
    checks
}

/// The fixture registry with `test.grid` and `test.flag` classed as Cold.
pub fn run_registry_classed() -> ezsdr_kernel::module_api::ModuleRegistry {
    let mut vocabulary = test_vocabulary();
    for declaration in &mut vocabulary.keys {
        if matches!(declaration.key.as_str(), "test.grid" | "test.flag") {
            declaration.update_class = Some(UpdateClass::Cold);
        }
    }
    fixture_registry(vocabulary)
}

fn fixture_registry(vocabulary: VocabularyDescriptor) -> ezsdr_kernel::module_api::ModuleRegistry {
    let mut registry = ezsdr_kernel::module_api::ModuleRegistry::new();
    registry
        .register_vocabulary(vocabulary)
        .expect("fresh fixture vocabulary");
    registry
        .register(
            test_provider_descriptor(),
            Factories {
                provider: true,
                authority: true,
                ..Factories::default()
            },
        )
        .expect("register fixture Provider and Authority");
    registry
        .register(
            test_sink_descriptor(),
            Factories {
                sink: true,
                ..Factories::default()
            },
        )
        .expect("register fixture Sink");
    registry
        .register(
            test_executor_descriptor(),
            Factories {
                executor: true,
                ..Factories::default()
            },
        )
        .expect("register fixture Executor");
    registry
        .register(
            test_link_module_descriptor(),
            Factories {
                link: true,
                ..Factories::default()
            },
        )
        .expect("register fixture Link");
    registry
        .register_link_descriptor(test_link_descriptor())
        .expect("fixture Link descriptor");
    registry
}
