//! The device-paced doubles of spec 19 §0: `WallAuthority` and `ThreadedProvider`.
//! Test code (PO-11 does not scan it).
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use ezsdr_kernel::event::{Action, Event, EventKind, EventSink, Severity};
use ezsdr_kernel::id::{ClockDomainId, ResourceId};
use ezsdr_kernel::module_api::{
    ActionReceiver, Authority, AuthorityDescriptor, CoerceReport, Endpoint, ModuleError,
    ModuleRef, Pacing, PrepareContext, Provider, ProviderInstance, StopMode,
};
use ezsdr_kernel::plan::{Fragment, PrepareReport};
use ezsdr_kernel::stream::DataLink;
use ezsdr_kernel::time::{
    ClockDomain, ClockRegistry, ClockRelation, EpochRef, ManualTimeAuthority, Rational,
    SampleClockHandle, ScheduleHandle, TimeAuthority, TimeError, TimePoint,
};

use super::doubles::TestProvider;
use super::run_doubles::Probe;
use super::{block, header};

type WallCallback = Box<dyn FnOnce(TimePoint) + Send>;

#[derive(Default)]
struct WallState {
    pending: BTreeMap<(i64, u64), WallCallback>,
    next_seq: u64,
    last_fired: Option<i64>,
    /// The callbacks that have run, for `firing_before_schedule_returns`.
    ran: std::collections::BTreeSet<u64>,
}

/// The clock of [`WallAuthority`]: a 1 GHz root counting host nanoseconds since the
/// double was built (spec 19 §0).
pub struct WallTime {
    registry: Arc<ClockRegistry>,
    root: ClockDomainId,
    base: std::time::Instant,
    state: Mutex<WallState>,
    changed: Condvar,
    fire_first: AtomicBool,
}

impl WallTime {
    fn elapsed(&self) -> i64 {
        self.base.elapsed().as_nanos() as i64
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, WallState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn root_ticks(&self, t: TimePoint) -> Result<i64, TimeError> {
        if t.domain == self.root || t.domain == ClockDomainId::HOST_MONOTONIC {
            return Ok(t.ticks);
        }
        if !self.governs(t.domain) {
            return Err(TimeError::NotGoverned { id: t.domain });
        }
        Ok(self.registry.conversion(t.domain, self.root)?.try_exact(t)?.ticks)
    }
}

impl TimeAuthority for WallTime {
    fn primary_root(&self) -> ClockDomainId {
        self.root
    }
    fn pacing(&self) -> Pacing {
        Pacing::Device
    }
    fn governs(&self, domain: ClockDomainId) -> bool {
        domain == self.root
            || domain == ClockDomainId::HOST_MONOTONIC
            || self
                .registry
                .get(domain)
                .is_ok_and(|d| d.root_id() == self.root)
    }
    fn now(&self, domain: ClockDomainId) -> Result<TimePoint, TimeError> {
        let now = self.elapsed();
        if domain == self.root || domain == ClockDomainId::HOST_MONOTONIC {
            return Ok(TimePoint::new(domain, now));
        }
        if !self.governs(domain) {
            return Err(TimeError::NotGoverned { id: domain });
        }
        Ok(self
            .registry
            .convert(TimePoint::new(self.root, now), domain)?
            .floor())
    }
    fn wait_until(&self, t: TimePoint) -> Result<(), TimeError> {
        let target = self.root_ticks(t)?;
        loop {
            let now = self.elapsed();
            if now >= target {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_nanos((target - now) as u64));
        }
    }
    /// TM-16c as KG-9 amends it for a paced Authority: only an instant before the last
    /// fired one is `InPast`.
    fn schedule(&self, t: TimePoint, f: WallCallback) -> Result<ScheduleHandle, TimeError> {
        let ticks = self.root_ticks(t)?;
        let mut state = self.lock();
        if state.last_fired.is_some_and(|fired| ticks < fired) {
            return Err(TimeError::InPast {
                now: TimePoint::new(self.root, self.elapsed()),
                requested: t,
            });
        }
        let seq = state.next_seq;
        state.next_seq += 1;
        state.pending.insert((ticks, seq), f);
        self.changed.notify_all();
        if self.fire_first.load(Ordering::Acquire) && ticks <= self.elapsed() {
            // A paced Authority may return from `schedule` after the callback of an
            // instant already passed has run on a waiting `next_wakeup` (KC-46a).
            let until = std::time::Instant::now() + std::time::Duration::from_millis(20);
            while !state.ran.contains(&seq) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                if left.is_zero() {
                    break;
                }
                state = self.changed.wait_timeout(state, left).unwrap_or_else(|e| e.into_inner()).0;
            }
        }
        Ok(ScheduleHandle { root: self.root, ticks, seq })
    }
    /// MA-29 as KG-3 amends it: a cancel wakes a waiting `next_wakeup`.
    fn cancel(&self, h: ScheduleHandle) -> bool {
        let removed = self.lock().pending.remove(&(h.ticks, h.seq)).is_some();
        self.changed.notify_all();
        removed
    }
}

/// A `Pacing::Device` Authority on the host's monotonic clock (spec 19 §0).
pub struct WallAuthority {
    descriptor: AuthorityDescriptor,
    time: Arc<WallTime>,
    relations: Vec<ClockRelation>,
}

impl WallAuthority {
    /// Registers a fresh 1 GHz root and measures its two relations.
    pub fn new(clocks: &Arc<ClockRegistry>, module: ModuleRef) -> (WallAuthority, ClockDomainId) {
        let root = clocks.allocate_id().unwrap();
        clocks
            .register(ClockDomain::root(
                root,
                Rational::new(1_000_000_000, 1).expect("a valid rate"),
                EpochRef::Arbitrary {
                    set_by: "test.wall".to_owned(),
                },
            ))
            .expect("a fresh root");
        let time = Arc::new(WallTime {
            registry: clocks.clone(),
            root,
            base: std::time::Instant::now(),
            state: Mutex::new(WallState::default()),
            changed: Condvar::new(),
            fire_first: AtomicBool::new(false),
        });
        let utc = || {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("after 1970")
                .as_nanos() as i64
        };
        let before = utc();
        let measured = time.elapsed();
        let after = utc();
        let relation = |target, offset, uncertainty, drift_uncertainty, method: &str| ClockRelation {
            source: root,
            target,
            measured_at: TimePoint::new(root, measured),
            offset: TimePoint::new(target, offset),
            drift: 0.0,
            drift_uncertainty,
            uncertainty: ezsdr_kernel::time::Duration::new(target, uncertainty),
            method: method.to_owned(),
            valid: ezsdr_kernel::time::Validity {
                from: TimePoint::new(root, measured),
                to: None,
            },
        };
        let relations = vec![
            relation(ClockDomainId::HOST_MONOTONIC, measured, 0, 0.0, "test.wall"),
            relation(
                ClockDomainId::UTC,
                before + (after - before) / 2,
                after - before,
                1e-4,
                "test.wall_utc",
            ),
        ];
        let descriptor = AuthorityDescriptor {
            module,
            governs: vec![root, ClockDomainId::HOST_MONOTONIC],
            pacing: Pacing::Device,
        };
        (WallAuthority { descriptor, time, relations }, root)
    }

    /// Publishes no relation (KG-11's negative case).
    pub fn without_relations(mut self) -> WallAuthority {
        self.relations.clear();
        self
    }

    /// Makes `schedule` of an instant already passed return only once a waiting
    /// `next_wakeup` has run its callback (Review L, P1-6).
    pub fn firing_before_schedule_returns(self) -> WallAuthority {
        self.time.fire_first.store(true, Ordering::Release);
        self
    }

    /// Publishes these relations instead of the measured ones (KG-11).
    pub fn with_relations(mut self, relations: Vec<ClockRelation>) -> WallAuthority {
        self.relations = relations;
        self
    }

    /// The relations it publishes.
    pub fn measured_relations(&self) -> Vec<ClockRelation> {
        self.relations.clone()
    }
}

impl Authority for WallAuthority {
    fn descriptor(&self) -> &AuthorityDescriptor {
        &self.descriptor
    }
    fn time(&self) -> Arc<dyn TimeAuthority> {
        self.time.clone()
    }
    /// Waits until the host clock reaches the earliest instant, re-evaluating on every
    /// `schedule` and `cancel`, and runs the callbacks due there without holding the
    /// lock (MA-29 as KG-3 amends it).
    fn next_wakeup(&self) -> Option<TimePoint> {
        let time = &self.time;
        let mut state = time.lock();
        let tick = loop {
            let (&(tick, _), _) = state.pending.first_key_value()?;
            let now = time.elapsed();
            if now >= tick {
                break tick;
            }
            let wait = std::time::Duration::from_nanos((tick - now) as u64);
            state = time
                .changed
                .wait_timeout(state, wait)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        };
        state.last_fired = Some(tick);
        for _ in 0..ManualTimeAuthority::DEFAULT_CALLBACK_CAP {
            let due = state
                .pending
                .first_key_value()
                .is_some_and(|(&(t, _), _)| t == tick);
            if !due {
                break;
            }
            let ((_, seq), callback) = state.pending.pop_first().expect("checked above");
            drop(state);
            callback(TimePoint::new(time.root, tick));
            state = time.lock();
            state.ran.insert(seq);
            time.changed.notify_all();
        }
        Some(TimePoint::new(time.root, tick))
    }
    fn relations(&self) -> Vec<ClockRelation> {
        self.relations.clone()
    }
}

/// What [`ThreadedProvider`]'s thread shares with its calls.
struct ThreadedCtx {
    name: String,
    id: ResourceId,
    probe: Probe,
    time: Arc<dyn TimeAuthority>,
    clocks: Arc<ClockRegistry>,
    events: Arc<dyn EventSink>,
    actions: Arc<dyn ActionReceiver>,
    outs: Vec<Arc<dyn DataLink>>,
    stopping: AtomicBool,
    action_delay: Option<std::time::Duration>,
    never_finishing: bool,
}

impl ThreadedCtx {
    fn record(&self, line: impl AsRef<str>) {
        self.probe.record(format!("{}:{}", self.name, line.as_ref()));
    }

    fn now(&self) -> TimePoint {
        self.time
            .now(self.time.primary_root())
            .expect("the primary root is governed")
    }

    fn emit(&self, kind: &str, severity: Severity, payload: serde_json::Value) {
        let _ = self.events.emit_control(Event {
            source: self.id.clone(),
            time: self.now(),
            severity,
            kind: EventKind::parse(kind).expect("a valid test kind"),
            payload,
        });
    }

    /// Carries out one Action before the next `recv()` (MA-14b).
    fn handle(&self, action: Action) {
        let variant = match &action {
            Action::TxBurst { .. } => "TxBurst",
            Action::UpdateParameter { .. } => "UpdateParameter",
            Action::Stop { .. } => "Stop",
            _ => "Other",
        };
        if self.stopping.load(Ordering::Acquire) {
            self.record(format!("after_stop:{variant}"));
        }
        self.record(format!("took:{variant}"));
        if let Some(delay) = self.action_delay {
            std::thread::sleep(delay);
        }
        if let Action::UpdateParameter {
            key,
            value: ezsdr_kernel::spec::Value::Int(n),
            ..
        } = &action
        {
            if key.as_str() == "test.tx_clock" {
                let stream = self.id.child("tx").expect("a valid stream id");
                let ratio = Rational::new(*n as u64, 1).expect("a positive ratio");
                let handle = self
                    .clocks
                    .declare_sample_clock(stream, self.time.primary_root(), ratio)
                    .expect("a fresh clock");
                let now = self.now().ticks;
                let origin = (now + n - 1) / n * n;
                self.clocks
                    .register_sample_clock(&handle, origin)
                    .expect("a declared clock");
                self.record(format!("tx_clock:{origin}"));
            }
        }
        self.record(format!("finished:{variant}"));
    }
}

/// A Provider of the `test` Vocabulary that is not stepped and runs one thread of its
/// own (spec 19 §0).
pub struct ThreadedProvider {
    name: String,
    inner: TestProvider,
    probe: Probe,
    action_delay: Option<std::time::Duration>,
    publish: Option<(u32, std::time::Duration)>,
    marks: Vec<(String, std::time::Duration)>,
    lost_after: Option<std::time::Duration>,
    loss: Option<Arc<(Mutex<bool>, Condvar)>>,
    prepare_delay: Option<std::time::Duration>,
    arm_delay: Option<std::time::Duration>,
    stop_tail: u32,
    never_finishing: bool,
    drain_after_stop: Option<std::time::Duration>,
    ctx: Option<Arc<ThreadedCtx>>,
    rx_clock: Option<SampleClockHandle>,
    next_index: Arc<AtomicU64>,
    worker: Option<(Arc<AtomicBool>, std::thread::JoinHandle<()>)>,
}

impl ThreadedProvider {
    /// A Provider at `path` recording `<name>:<call>` into `probe`.
    pub fn new(name: &str, path: &str, probe: &Probe) -> ThreadedProvider {
        ThreadedProvider {
            name: name.to_owned(),
            inner: TestProvider::new(path, 2),
            probe: probe.clone(),
            action_delay: None,
            publish: None,
            marks: Vec::new(),
            lost_after: None,
            loss: None,
            prepare_delay: None,
            arm_delay: None,
            stop_tail: 0,
            never_finishing: false,
            drain_after_stop: None,
            ctx: None,
            rx_clock: None,
            next_index: Arc::new(AtomicU64::new(0)),
            worker: None,
        }
    }

    /// Sleeps `d` inside each Action (KC-21a).
    pub fn with_action_delay(mut self, d: std::time::Duration) -> Self {
        self.action_delay = Some(d);
        self
    }

    /// Publishes a block of `block_len` samples on every `rx` link every `period`.
    pub fn publishing(mut self, block_len: u32, period: std::time::Duration) -> Self {
        self.publish = Some((block_len, period));
        self
    }

    /// Emits `kind` once, `after` the start.
    pub fn marking(mut self, kind: &str, after: std::time::Duration) -> Self {
        self.marks.push((kind.to_owned(), after));
        self
    }

    /// Emits `DEVICE_LOST` from its thread `after` the start (MA-9a).
    pub fn losing_device(mut self, after: std::time::Duration) -> Self {
        self.lost_after = Some(after);
        self
    }

    /// Emits `DEVICE_LOST` from its thread once `gate` holds true (MA-9a).
    pub fn losing_device_on(mut self, gate: Arc<(Mutex<bool>, Condvar)>) -> Self {
        self.loss = Some(gate);
        self
    }

    /// Sleeps `d` in `prepare`.
    pub fn with_prepare_delay(mut self, d: std::time::Duration) -> Self {
        self.prepare_delay = Some(d);
        self
    }

    /// Sleeps `d` in `arm`.
    pub fn with_arm_delay(mut self, d: std::time::Duration) -> Self {
        self.arm_delay = Some(d);
        self
    }

    /// Publishes `n` more blocks inside `stop`.
    pub fn with_stop_tail(mut self, n: u32) -> Self {
        self.stop_tail = n;
        self
    }

    /// Stops calling `recv()` after it has taken one Action.
    pub fn never_finishing(mut self) -> Self {
        self.never_finishing = true;
        self
    }

    /// Keeps calling `recv()` for `d` after `stop` began, recording what it receives.
    pub fn draining_after_stop(mut self, d: std::time::Duration) -> Self {
        self.drain_after_stop = Some(d);
        self
    }

    fn record(&self, line: impl AsRef<str>) {
        self.probe.record(format!("{}:{}", self.name, line.as_ref()));
    }
}

fn publish_block(ctx: &ThreadedCtx, clock: ClockDomainId, block_len: u32, next: &AtomicU64) {
    let index = next.fetch_add(block_len as u64, Ordering::AcqRel) as i64;
    let sample = block(header(TimePoint::new(clock, index), block_len, 1));
    for link in &ctx.outs {
        let _ = link.publish(sample.clone());
    }
}

impl Provider for ThreadedProvider {
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
        self.record("prepare");
        self.record(format!("prepare_on:{:?}", std::thread::current().id()));
        if let Some(delay) = self.prepare_delay {
            std::thread::sleep(delay);
        }
        let id = self.inner.instance().id.clone();
        if self.publish.is_some() {
            let stream = id.child("rx").expect("a valid stream id");
            let ratio = Rational::new(1000, 1).expect("a valid ratio");
            let handle = ctx
                .clocks
                .declare_sample_clock(stream, ctx.time.primary_root(), ratio)
                .map_err(|e| ModuleError::rejected(format!("test: {e}")))?;
            self.rx_clock = Some(handle);
        }
        let outs = ctx
            .links
            .iter()
            .filter_map(|attached| match &attached.endpoint {
                Endpoint::StreamOut(link) => Some(link.clone()),
                _ => None,
            })
            .collect();
        self.ctx = Some(Arc::new(ThreadedCtx {
            name: self.name.clone(),
            id,
            probe: self.probe.clone(),
            time: ctx.time.clone(),
            clocks: ctx.clocks.clone(),
            events: ctx.events.clone(),
            actions: ctx.actions.clone(),
            outs,
            stopping: AtomicBool::new(false),
            action_delay: self.action_delay,
            never_finishing: self.never_finishing,
        }));
        self.inner.prepare(f, ctx)
    }

    fn arm(&mut self) -> Result<(), ModuleError> {
        self.record("arm");
        if let Some(delay) = self.arm_delay {
            std::thread::sleep(delay);
        }
        Ok(())
    }

    fn start(&mut self, at: Option<TimePoint>) -> Result<(), ModuleError> {
        self.record("start");
        let ctx = self
            .ctx
            .clone()
            .ok_or_else(|| ModuleError::rejected("test: not prepared"))?;
        if let (Some(handle), Some(t0)) = (&self.rx_clock, at) {
            ctx.clocks
                .register_sample_clock(handle, t0.ticks)
                .map_err(|e| ModuleError::rejected(format!("test: {e}")))?;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let publish = self.publish.zip(self.rx_clock.as_ref().map(|h| h.id));
        let marks = self.marks.clone();
        let lost_after = self.lost_after;
        let loss = self.loss.clone();
        let next = self.next_index.clone();
        let worker = std::thread::spawn(move || {
            let begun = std::time::Instant::now();
            let mut next_publish = publish.map(|((_, period), _)| begun + period);
            let mut marked = vec![false; marks.len()];
            let mut lost = false;
            let mut took_one = false;
            while !flag.load(Ordering::Acquire) {
                if !(ctx.never_finishing && took_one) {
                    while let Some(action) = ctx.actions.recv() {
                        ctx.handle(action);
                        took_one = true;
                        if ctx.never_finishing {
                            break;
                        }
                    }
                }
                let now = std::time::Instant::now();
                if !lost {
                    if let (Some(((block_len, period), clock)), Some(at)) = (publish, next_publish) {
                        if now >= at {
                            publish_block(&ctx, clock, block_len, &next);
                            next_publish = Some(at + period);
                        }
                    }
                    for (i, (kind, after)) in marks.iter().enumerate() {
                        if !marked[i] && now >= begun + *after {
                            ctx.emit(kind, Severity::Info, serde_json::json!({}));
                            ctx.record(format!("marked:{kind}"));
                            marked[i] = true;
                        }
                    }
                    let gate_open = loss
                        .as_ref()
                        .is_some_and(|gate| *gate.0.lock().unwrap_or_else(|e| e.into_inner()));
                    if gate_open || lost_after.is_some_and(|after| now >= begun + after) {
                        ctx.emit(
                            EventKind::DEVICE_LOST,
                            Severity::Fatal,
                            serde_json::json!({ "message": "test: device lost" }),
                        );
                        ctx.record("lost");
                        lost = true;
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        });
        self.worker = Some((stop, worker));
        Ok(())
    }

    fn stop(&mut self, _mode: StopMode) -> Result<(), ModuleError> {
        self.record("stop");
        let Some(ctx) = self.ctx.clone() else {
            return Ok(());
        };
        ctx.stopping.store(true, Ordering::Release);
        if let Some(d) = self.drain_after_stop {
            let until = std::time::Instant::now() + d;
            while std::time::Instant::now() < until {
                while let Some(action) = ctx.actions.recv() {
                    ctx.handle(action);
                }
                // A spin, not a sleep: a push after the freeze is cleared again by the
                // control thread's own RS-6 step 1 within about 100 µs.
                std::thread::yield_now();
            }
        }
        if let Some((flag, worker)) = self.worker.take() {
            flag.store(true, Ordering::Release);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
            while !worker.is_finished() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
        if let (Some((block_len, _)), Some(handle)) = (self.publish, &self.rx_clock) {
            for _ in 0..self.stop_tail {
                publish_block(&ctx, handle.id, block_len, &self.next_index);
                self.record("tail");
            }
        }
        Ok(())
    }

    fn cleanup(&mut self) {
        self.record("cleanup");
        self.ctx = None;
    }
}
