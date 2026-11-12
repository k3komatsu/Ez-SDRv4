//! VH-8 for the layout's keys (RM-27…RM-32, design 33 §6): the same selector and the same
//! Spec through MockRadio and through `UhdRadio` on a `FakeDevice`, compared directly —
//! `PrepareReport.effective`, the declared SampleClocks (node and ratio), the same refusals,
//! and a short run's clock records and Manifest plan rows, per stream (0 and 1). PO-8: only
//! this crate may pair two Modules, so the two rigs are copies of `ezsdr-mock-radio`'s
//! `mock_layout.rs` and `ezsdr-radio-uhd`'s `fake.rs` harnesses, not shared code.
//!
//! What is not compared, by design: the device calls and read-backs (MockRadio has no
//! device), the UHD-only opening checks (the subdev spec, the serial, the rate set, the
//! front ends — UR-5, items 2–5) and the UHD-only refusals (UR-12's read-back, UR-21's
//! target kinds); block contents and physical instants beyond VH-8's own columns; and a
//! channel moving from one stream to another (RM-34, withdrawn: channel sets are fixed for
//! the Run). The Manifest's `sequence` is compared as an order, not as numbers: each
//! Provider counts its own arrivals from its own origin (MockRadio's faults first, uhd-control's
//! loss as 0). Both providers share `channels::Description`, so a shared mistake inside it
//! is invisible here: this file is a net for divergences, and the specs' own tests are the
//! ground truth.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration as Wall;

use ezsdr_kernel::binding::{AdmissionCheckRegistry, Binding, Violation};
use ezsdr_kernel::event::{Action, ActionId, Event, EventCollector, EventSource, Target};
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::{ClockDomainId, ModuleId, ResourceId, RunId};
use ezsdr_kernel::module_api::{
    ActionReceiver, ActionSubmitter, AttachedPort, Authority, CoerceReport, Endpoint, ExecutionClass, ModuleRef, ModuleRegistry, Pacing, PrepareContext, ProfileRef, Provider, ProviderInstance,
    Requested, Version,
};
use ezsdr_kernel::plan::Fragment;
use ezsdr_kernel::policy::{EventKindRegistry, Policy};
use ezsdr_kernel::spec::{Constraint, Ident, Key, Namespace, Scalar, Value};
use ezsdr_kernel::stream::{BackPressure, BlockFlags, BlockRef, DataLink, DropCarry, PublishOutcome};
use ezsdr_kernel::time::{ClockDomain, ClockRegistry, Duration, EpochRef, ManualTimeAuthority, Rational, TimePoint};
use ezsdr_mock_radio::MockRadio;
use ezsdr_radio::payloads::{RxOverflowCause, RxOverflowPayload};
use ezsdr_radio_uhd::{Device, DeviceAuthority, FakeConfig, FakeDevice, FakeFault, UhdRadio};
use serde_json::json;

/// The marker the runners read when a real-time bound expires (note 51 §6.3): shared with the UHD tests.
#[path = "../../ezsdr-radio-uhd/tests/common/infra.rs"]
mod infra;
use infra::infra;

/// What this rig and the UHD tests' `Direct` share (note 51 stage 1c): the Action queue that withholds an Action until the device clock has moved past its push.
#[path = "../../ezsdr-radio-uhd/tests/common/rig.rs"]
mod rig;
use rig::{AuthorityGuard, assert_joined, assert_stands_still};

/// The O1 spec oracle (design note 49 §5.4.11): one source shared with the UHD and Mock tests; it names no Provider type.
mod oracle;

const ROOT_HZ: i64 = 200_000_000;

fn ms(n: i64) -> i64 {
    n * ROOT_HZ / 1_000
}

fn key(name: &str) -> Key {
    Key::parse(name).unwrap()
}

fn rid(path: &str) -> ResourceId {
    ResourceId::parse(path).unwrap()
}

/// One registered SampleClock as both rigs compare it: `(node, ratio, origin, ended)`.
type ClockRow = (String, (u64, u64), i64, Option<i64>);

fn eq(value: Scalar) -> Constraint {
    Constraint::Eq { value }
}

/// The selector's shared layout values (design 33 D5).
fn selector(rx: &[&str], tx: &[&str]) -> BTreeMap<Ident, Value> {
    let list = |elements: &[&str]| Value::List(elements.iter().map(|e| Scalar::from(*e)).collect());
    BTreeMap::from([(Ident::parse("rx").unwrap(), list(rx)), (Ident::parse("tx").unwrap(), list(tx))])
}

fn mock_binding(mut selector: BTreeMap<Ident, Value>) -> Binding {
    selector.insert(Ident::parse("id").unwrap(), Value::from("radio".to_owned()));
    Binding {
        module: ModuleRef { id: ModuleId::parse("ezsdr.radio.mock").unwrap(), version: Version::new(2, 0, 0) },
        selector,
        profile: Some(ProfileRef { name: "x3xx".to_owned(), version: Version::new(0, 1, 0) }),
        feed: None,
    }
}

fn uhd_binding(mut selector: BTreeMap<Ident, Value>) -> Binding {
    selector.insert(Ident::parse("args").unwrap(), Value::from("type=fake".to_owned()));
    selector.insert(Ident::parse("id").unwrap(), Value::from("radio".to_owned()));
    Binding {
        module: ModuleRef { id: ModuleId::parse("ezsdr.radio.uhd").unwrap(), version: Version::new(0, 4, 0) },
        selector,
        profile: Some(ProfileRef { name: "x3xx".to_owned(), version: Version::new(0, 1, 0) }),
        feed: None,
    }
}

fn requested(constraints: &[(&str, Constraint)]) -> Requested {
    Requested { resource: rid("radio"), constraints: constraints.iter().map(|(name, constraint)| (key(name), constraint.clone())).collect() }
}

/// RM-30: no channel is in a stream unless a request puts it there; these comparisons receive on
/// c0, so a request that does not name it gets it, on both providers alike.
fn with_rx0<'a>(provider: &dyn ezsdr_kernel::module_api::Provider, constraints: &[(&'a str, Constraint)]) -> Vec<(&'a str, Constraint)> {
    let mut constraints = constraints.to_vec();
    if provider.instance().tree.capabilities.contains_key(&Key::parse("radio.rx.c0.stream").unwrap()) && !constraints.iter().any(|(name, _)| *name == "radio.rx.c0.stream") {
        constraints.push(("radio.rx.c0.stream", Constraint::Eq { value: Scalar::from(0) }));
    }
    constraints
}

fn fragment(module: ModuleRef, constraints: &[(&str, Constraint)]) -> Fragment {
    Fragment {
        id: Ident::parse("radio").unwrap(),
        instance: module,
        role: ezsdr_kernel::module_api::Role::Provider,
        content: json!({ "selector": {}, "requested": requested(constraints) }),
        after: Vec::new(),
    }
}

// ---------------------------------------------------------------- the Mock's rig

struct Queue {
    actions: rig::Stamped<(Action, bool)>,
    given: AtomicU64,
}

impl ActionReceiver for Queue {
    fn recv(&self) -> Option<ezsdr_kernel::module_api::Dispatched> {
        self.actions.take().map(|(action, untimed)| {
            let node = action.target().and_then(|target| match target {
                Target::Resource { resource, path } if path.is_empty() => rid(resource.as_str()).into(),
                Target::Resource { resource, path } => Some(rid(&format!("{resource}/{path}"))),
                _ => None,
            });
            let id = ActionId(self.given.fetch_add(1, Ordering::SeqCst) + 1);
            if untimed { ezsdr_kernel::module_api::Dispatched::new_untimed(id, action, node) } else { ezsdr_kernel::module_api::Dispatched::new(id, action, node) }
        })
    }
}

impl Queue {
    /// `clock` is the device clock of a rig whose thread drives it, `None` for the Mock's (see `rig::Stamped`).
    fn new(clock: Option<ezsdr_radio_uhd::HostClock>) -> Queue {
        Queue { actions: rig::Stamped::new(clock), given: AtomicU64::new(0) }
    }

    /// The Actions the Provider has not taken yet: an empty queue means the control thread
    /// has taken the last one; its booking follows while the device clock stands still.
    fn queued(&self) -> usize {
        self.actions.len()
    }
}

struct Accepting;
impl ActionSubmitter for Accepting {
    fn submit(&self, _action: Action) -> Result<ActionId, Vec<Violation>> {
        Ok(ActionId(1))
    }
}

struct MemLink {
    queue: Mutex<VecDeque<BlockRef>>,
}

impl MemLink {
    fn new() -> MemLink {
        MemLink { queue: Mutex::new(VecDeque::new()) }
    }

    fn take_blocks(&self) -> Vec<BlockRef> {
        self.queue.lock().unwrap().drain(..).collect()
    }
}

impl DataLink for MemLink {
    fn publish(&self, block: BlockRef) -> PublishOutcome {
        let mut queue = self.queue.lock().unwrap();
        if queue.len() < 64 {
            queue.push_back(block);
        } else {
            queue.pop_front();
            queue.push_back(block);
        }
        PublishOutcome::Accepted
    }
    fn receive(&self) -> Option<(BlockRef, DropCarry)> {
        self.queue.lock().unwrap().pop_front().map(|block| (block, DropCarry::default()))
    }
    fn drops(&self) -> u64 {
        0
    }
    fn take_drop_carry(&self) -> DropCarry {
        DropCarry::default()
    }
    fn policy(&self) -> BackPressure {
        BackPressure::DropOldest
    }
}

struct MockRig {
    mock: MockRadio,
    clocks: Arc<ClockRegistry>,
    time: Arc<ManualTimeAuthority>,
    root: ClockDomainId,
    actions: Arc<Queue>,
    effective: BTreeMap<Key, Value>,
    events: Arc<EventCollector>,
    rx_links: Vec<Arc<MemLink>>,
}

impl MockRig {
    fn new(selector: BTreeMap<Ident, Value>, constraints: &[(&str, Constraint)]) -> MockRig {
        MockRig::with_ports(selector, constraints, &["rx0"])
    }

    /// With one link on each named receive port (RM-29: `rx0`, `rx1`, …), so that every
    /// stream a comparison runs has its own.
    fn with_ports(selector: BTreeMap<Ident, Value>, constraints: &[(&str, Constraint)], ports: &[&str]) -> MockRig {
        Self::with_run(selector, constraints, ports, BTreeMap::new(), BTreeMap::new())
    }

    fn with_run(
        selector: BTreeMap<Ident, Value>,
        constraints: &[(&str, Constraint)],
        ports: &[&str],
        inputs: BTreeMap<ContentHash, Arc<[u8]>>,
        environment: BTreeMap<ezsdr_kernel::spec::Namespace, serde_json::Value>,
    ) -> MockRig {
        let mut mock = MockRadio::from_binding(&mock_binding(selector)).unwrap();
        mock = mock.with_medium(ezsdr_sim::channel::Medium::new());
        let clocks = Arc::new(ClockRegistry::new());
        let root = ClockDomainId::local(41);
        clocks.register(ClockDomain::root(root, Rational::new(ROOT_HZ as u64, 1).unwrap(), EpochRef::Arbitrary { set_by: "vh8".to_owned() })).unwrap();
        let time = Arc::new(ManualTimeAuthority::new(clocks.clone(), root, &[], Pacing::FreeRunning).unwrap());
        let actions = Arc::new(Queue::new(None));
        let mut kinds = EventKindRegistry::with_kernel_kinds();
        ezsdr_radio::register(&mut ModuleRegistry::new(), &mut AdmissionCheckRegistry::new(), &mut kinds).unwrap();
        ezsdr_sim::register(&mut ModuleRegistry::new(), &mut AdmissionCheckRegistry::new(), &mut kinds).unwrap();
        let stream_nodes: Vec<ResourceId> = mock.instance().tree.children.iter().map(|child| child.id.clone()).collect();
        let sources: Vec<EventSource> = std::iter::once(rid("radio")).chain(stream_nodes).map(|node| EventSource::Node { node }).collect();
        let event_kinds = kinds.kinds();
        let pairs: Vec<_> = sources.into_iter().flat_map(|source| event_kinds.iter().cloned().map(move |kind| (source.clone(), kind))).collect();
        let events = Arc::new(EventCollector::new(&pairs, &event_kinds, 4096, &Policy::default()));
        let rx_links: Vec<_> = ports.iter().map(|_| Arc::new(MemLink::new())).collect();
        let links = ports.iter().zip(&rx_links).map(|(port, link)| AttachedPort::new(Ident::parse("radio").unwrap(), Ident::parse(port).unwrap(), Endpoint::StreamOut(link.clone()))).collect();
        let ctx = {
            let mut ctx = PrepareContext::testing(EventSource::Node { node: rid("radio") }, time.clone(), clocks.clone(), events.clone(), actions.clone(), Arc::new(Accepting));
            ctx.run = RunId::from_string("vh8-mock".to_owned());
            ctx.class = ExecutionClass::Simulation;
            ctx.environment = Arc::new(environment);
            ctx.inputs = Arc::new(inputs);
            ctx.links = links;
            ctx.host_budget = RelativeBudget::new(Duration::new(ClockDomainId::HOST_MONOTONIC, 5_000_000_000)).unwrap();
            ctx
        };
        let constraints = with_rx0(&mock, constraints);
        let prepared = mock.prepare(&fragment(ModuleRef { id: ModuleId::parse("ezsdr.radio.mock").unwrap(), version: Version::new(2, 0, 0) }, &constraints), ctx).unwrap();
        MockRig { mock, clocks, time, root, actions, effective: prepared.effective, events, rx_links }
    }

    /// `arm` and `start` at `t0`, both in root ticks.
    fn arm_start(&mut self, t0: i64) {
        self.mock.arm().unwrap();
        self.mock.start(Some(TimePoint::new(self.root, t0))).unwrap();
    }

    fn step(&mut self, tick: i64) {
        self.time.advance_to(TimePoint::new(self.root, tick)).unwrap();
        self.mock.step(TimePoint::new(self.root, tick)).unwrap();
    }

    fn push(&self, action: Action) {
        self.actions.actions.push((action, false));
    }

    fn push_untimed(&self, action: Action) {
        self.actions.actions.push((action, true));
    }

    fn stop_at(&self, node: &str) {
        self.push(Action::Stop { target: Some(Target::Resource { resource: Ident::parse("radio").unwrap(), path: node.to_owned() }) });
    }

    /// The declared clocks as `(node, ratio)`, smallest first (TM-12).
    fn declared(&self) -> Vec<(String, (u64, u64))> {
        let mut clocks: Vec<_> =
            self.clocks.declared_sample_clocks().into_iter().map(|handle| (handle.stream().path().to_owned(), (handle.root_ticks_per_tick().num(), handle.root_ticks_per_tick().den()))).collect();
        clocks.sort();
        clocks
    }

    /// The registered clocks as `(node, ratio, origin, ended)`, smallest first (TM-13d).
    fn records(&self) -> Vec<ClockRow> {
        let mut records: Vec<_> = self
            .clocks
            .sample_clock_records()
            .into_iter()
            .map(|record| {
                let origin = record.origin.ticks_in(record.root).unwrap();
                (record.stream.path().to_owned(), (record.root_ticks_per_tick.num(), record.root_ticks_per_tick.den()), origin, record.ended_at.map(|end| end.ticks_in(record.root).unwrap()))
            })
            .collect();
        records.sort();
        records
    }

    fn events(&self) -> Vec<ezsdr_kernel::event::Event> {
        self.events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0))
    }

    fn finish_in_place(&mut self) -> ProviderInstance {
        self.mock.stop(ezsdr_kernel::module_api::StopMode::Orderly).unwrap();
        self.mock.cleanup();
        self.mock.instance().clone()
    }

    /// `stop` and `cleanup`, as the Manifest's sections are written (MR-27).
    fn finish(mut self) -> ProviderInstance {
        self.finish_in_place()
    }
}

use ezsdr_kernel::time::RelativeBudget;

// ---------------------------------------------------------------- the UHD rig

struct UhdRig {
    /// First, so that it drops before the Authority: `DeviceAuthority::drop` waits in real time for `uhd-clock`, and a thread still registered (busy) would hold the clock that
    /// `uhd-clock` may be asleep on (note 51 §4.2).
    _driver: ezsdr_radio_uhd::host::Driver,
    radio: UhdRadio,
    device: Arc<FakeDevice>,
    clocks: Arc<ClockRegistry>,
    time: Arc<dyn ezsdr_kernel::time::TimeAuthority>,
    root: ClockDomainId,
    actions: Arc<Queue>,
    effective: BTreeMap<Key, Value>,
    events: Arc<EventCollector>,
    seen_events: Mutex<Vec<ezsdr_kernel::event::Event>>,
    rx_links: Vec<Arc<MemLink>>,
    _authority: AuthorityGuard,
}

impl UhdRig {
    fn new(selector: BTreeMap<Ident, Value>, constraints: &[(&str, Constraint)], config: FakeConfig) -> Result<UhdRig, ezsdr_kernel::module_api::ModuleError> {
        UhdRig::with_ports(selector, constraints, config, &["rx0"])
    }

    /// With one link on each named receive port, as `MockRig::with_ports` (RM-29).
    fn with_ports(selector: BTreeMap<Ident, Value>, constraints: &[(&str, Constraint)], config: FakeConfig, ports: &[&str]) -> Result<UhdRig, ezsdr_kernel::module_api::ModuleError> {
        Self::with_run(selector, constraints, config, ports, BTreeMap::new())
    }

    fn with_run(
        selector: BTreeMap<Ident, Value>,
        constraints: &[(&str, Constraint)],
        config: FakeConfig,
        ports: &[&str],
        inputs: BTreeMap<ContentHash, Arc<[u8]>>,
    ) -> Result<UhdRig, ezsdr_kernel::module_api::ModuleError> {
        // `TxBlocks` is a permanent busy loop in `tx_send`: on the rig's manual clock it freezes the clock for good, and every wait of the rig would hang.
        assert!(!config.faults.contains(&FakeFault::TxBlocks), "the rig refuses FakeFault::TxBlocks: it stops the manual clock for good");
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: ezsdr_radio_uhd::ClockMode::Manual, ..config }));
        let clocks = Arc::new(ClockRegistry::new());
        let authority = DeviceAuthority::new(device.clone(), clocks.clone(), "internal", "internal", "type=fake").unwrap_or_else(|error| {
            // The Authority's first read has a real-time bound (UR-7, 5 s): an expiry is the host's, so it is a marker (note 51 §6.2).
            if error.contains("did not come within") {
                infra::infra(&format!("the rig's Authority: {error}"));
            }
            panic!("{error}")
        });
        let (time, root) = (authority.time(), authority.root());
        let mut registry = ModuleRegistry::new();
        let (mut checks, mut kinds) = (AdmissionCheckRegistry::new(), EventKindRegistry::with_kernel_kinds());
        ezsdr_radio::register(&mut registry, &mut checks, &mut kinds).unwrap();
        let pairs: Vec<_> = ["radio", "radio/rx/s0", "radio/tx/s0", "radio/rx/s1", "radio/tx/s1"]
            .iter()
            .flat_map(|source| kinds.kinds().into_iter().map(move |kind| (EventSource::Node { node: rid(source) }, kind)))
            .collect();
        let events = Arc::new(EventCollector::new(&pairs, &kinds.kinds(), 4096, &Policy::default()));
        let queue = Arc::new(Queue::new(Some(device.host())));
        let rx_links: Vec<_> = ports.iter().map(|_| Arc::new(MemLink::new())).collect();
        let ctx = {
            let mut ctx = PrepareContext::testing(EventSource::Node { node: rid("radio") }, time.clone(), clocks.clone(), events.clone(), queue.clone(), Arc::new(Accepting));
            ctx.run = RunId::from_string("vh8-uhd".to_owned());
            ctx.class = ExecutionClass::HardwareInLoop;
            ctx.environment = Arc::new(BTreeMap::new());
            ctx.inputs = Arc::new(inputs);
            ctx.links = ports.iter().zip(&rx_links).map(|(port, link)| AttachedPort::new(Ident::parse("radio").unwrap(), Ident::parse(port).unwrap(), Endpoint::StreamOut(link.clone()))).collect();
            ctx.host_budget = RelativeBudget::new(Duration::new(ClockDomainId::HOST_MONOTONIC, 5_000_000_000)).unwrap();
            ctx
        };
        let mut radio = UhdRadio::from_binding(&uhd_binding(selector), device.clone())?;
        let constraints = with_rx0(&radio, constraints);
        let prepared = radio.prepare(&fragment(ezsdr_radio_uhd::module_ref(), &constraints), ctx)?;
        // Last, so that no error or unwind path above holds a live registration; the Authority's first read (5 s) and `prepare` stay outside it.
        let driver = device.host().drive();
        Ok(UhdRig {
            _driver: driver,
            radio,
            device,
            clocks,
            time,
            root,
            actions: queue,
            effective: prepared.effective,
            events,
            seen_events: Mutex::new(Vec::new()),
            rx_links,
            _authority: AuthorityGuard::new(authority),
        })
    }

    fn push(&self, action: Action) {
        self.actions.actions.push((action, false));
    }

    fn push_untimed(&self, action: Action) {
        self.actions.actions.push((action, true));
    }

    fn stop_at(&self, node: &str) {
        self.push(Action::Stop { target: Some(Target::Resource { resource: Ident::parse("radio").unwrap(), path: node.to_owned() }) });
    }

    fn arm(&mut self) -> Result<(), ezsdr_kernel::module_api::ModuleError> {
        self.radio.arm()
    }

    fn start(&mut self, t0: i64) -> Result<(), ezsdr_kernel::module_api::ModuleError> {
        self.radio.start(Some(TimePoint::new(self.root, t0)))
    }

    /// A wait on the device's host clock: the rig's thread is a participant (`_driver`), so it is idle here and the device's time advances; between waits it is busy and the clock stands
    /// (note 51 §4.2).
    fn settle(&mut self, wall: Wall) {
        self.device.host().sleep(wall);
        self.end_wait();
    }

    /// Advances the device clock to `tick`, as an idle participant, letting its threads run on the way.
    fn wait_until(&mut self, tick: i64) {
        let _ = self.time.wait_until(TimePoint::new(self.root, tick));
        self.end_wait();
    }

    /// The end of every wait that ends on an instant the test chose (R1, test plan 1c "M3"; the polls of `wait_for_events` end on an observed state, which is whole once seen): one microsecond more on the clock, so every participant due at the instant the wait ended on has run (M4 holds
    /// the instant until each has), and what the test then observes is what happened up to that instant. The order of the test thread and a poll at the instant *after* it is the queue's, not the wait's:
    /// an Action is received at the first poll strictly after its push (R2, `rig::Stamped`).
    fn end_wait(&self) {
        self.device.host().sleep(Wall::from_micros(1));
    }

    fn declared(&self) -> Vec<(String, (u64, u64))> {
        let mut clocks: Vec<_> =
            self.clocks.declared_sample_clocks().into_iter().map(|handle| (handle.stream().path().to_owned(), (handle.root_ticks_per_tick().num(), handle.root_ticks_per_tick().den()))).collect();
        clocks.sort();
        clocks
    }

    /// The registered clocks as `(node, ratio, origin, ended)`, smallest first (TM-13d).
    fn records(&self) -> Vec<ClockRow> {
        let mut records: Vec<_> = self
            .clocks
            .sample_clock_records()
            .into_iter()
            .map(|record| {
                let origin = record.origin.ticks_in(record.root).unwrap();
                (record.stream.path().to_owned(), (record.root_ticks_per_tick.num(), record.root_ticks_per_tick.den()), origin, record.ended_at.map(|end| end.ticks_in(record.root).unwrap()))
            })
            .collect();
        records.sort();
        records
    }

    fn events(&self) -> Vec<ezsdr_kernel::event::Event> {
        let mut seen = self.seen_events.lock().unwrap();
        seen.extend(self.events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)));
        std::mem::take(&mut *seen)
    }

    fn wait_for_event(&self, what: &str, reached: impl Fn(&ezsdr_kernel::event::Event) -> bool) {
        self.wait_for_events(what, 1, reached);
    }

    fn wait_for_events(&self, what: &str, count: usize, reached: impl Fn(&ezsdr_kernel::event::Event) -> bool) {
        self.wait_for_events_within(what, count, reached, Wall::from_secs(5), Wall::from_secs(10));
    }

    /// Two bounds. `horizon` is device time, the one the events must come in: a plain failure, the test's own verdict (a row killed because an event does not come is `killed`, note 51 §6.5).
    /// `bound` is real time (the polls are clock waits, which a stopped clock never ends); its expiry is the host's, so it writes the marker (§6.2). The horizon is checked first, so a failure
    /// that took `horizon` of device time stays a verdict. Arguments only so that a fixture can expire either.
    fn wait_for_events_within(&self, what: &str, count: usize, reached: impl Fn(&ezsdr_kernel::event::Event) -> bool, horizon: Wall, bound: Wall) {
        let begun = self.device.host().now();
        let deadline = std::time::Instant::now() + bound;
        loop {
            let mut seen = self.seen_events.lock().unwrap();
            seen.extend(self.events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)));
            if seen.iter().filter(|event| reached(event)).count() >= count {
                return;
            }
            drop(seen);
            assert!(self.device.host().now() - begun < horizon, "{what} did not come within {horizon:?} of device time: {:?}", self.device.calls());
            if std::time::Instant::now() >= deadline {
                infra::infra(&format!("the rig's wait for {what} did not end within {bound:?}"));
                panic!("timed out waiting for {what}: {:?}", self.device.calls());
            }
            self.device.host().sleep(Wall::from_millis(1));
        }
    }

    fn finish_in_place(&mut self) -> ProviderInstance {
        self.radio.stop(ezsdr_kernel::module_api::StopMode::Orderly).unwrap();
        self.radio.cleanup();
        self.radio.instance().clone()
    }

    /// The Provider's sections as `cleanup` writes them (UR-30).
    fn finish(mut self) -> ProviderInstance {
        self.finish_in_place()
    }
}

// ---------------------------------------------------------------- the comparisons

#[test]
fn vh_8_the_two_providers_prepare_one_layout_alike() {
    // Design 33 §6: one selector and one Spec, two Providers — `effective` is RM-30's key
    // set with the same values, and the declared SampleClocks are the same nodes and ratios
    // (the root at 200 MHz makes MockRadio's ratio `root_rate × N / 200 MHz` the same `N`).
    type Case = (&'static str, Vec<&'static str>, Vec<&'static str>, Vec<(&'static str, Constraint)>, Vec<&'static str>);
    let cases: [Case; 4] = [
        ("defaults", vec!["0:A:ubx:0", "0:B:ubx:0"], vec!["0:A:ubx:0"], vec![], vec!["UBX", "UBX"]),
        ("mixed boards, receive", vec!["0:A:ubx:0", "0:B:basic_rx:0"], vec!["0:A:ubx:0"], vec![], vec!["UBX", "BasicRX"]),
        ("mixed boards, transmit", vec!["0:A:ubx:0"], vec!["0:A:ubx:0", "0:B:lf_tx:0"], vec![("radio.tx.c0.stream", eq(Scalar::from(0)))], vec!["UBX", "LFTX"]),
        (
            "explicit values",
            vec!["0:A:ubx:0", "0:B:basic_rx:0"],
            vec!["0:A:ubx:0"],
            vec![
                ("radio.rx.c0.frequency_hz", eq(Scalar::try_from(1.1e9).unwrap())),
                ("radio.rx.c1.frequency_hz", eq(Scalar::try_from(1e6).unwrap())),
                ("radio.rx.c1.antenna", eq(Scalar::from("A"))),
                ("radio.rx.c1.stream", eq(Scalar::from(-1))),
                ("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(10e6).unwrap())),
            ],
            vec!["UBX", "BasicRX"],
        ),
    ];
    for (name, rx, tx, constraints, front_ends) in cases {
        let constraints: Vec<(&str, Constraint)> = constraints;
        let mock = MockRig::new(selector(&rx, &tx), &constraints);
        let config = FakeConfig { front_ends, serial: format!("vh8-{name}"), ..FakeConfig::default() };
        let uhd = UhdRig::new(selector(&rx, &tx), &constraints, config).unwrap_or_else(|error| panic!("{name}: {error:?}"));
        assert_eq!(mock.effective, uhd.effective, "{name}: the effective configuration");
        assert_eq!(mock.declared(), uhd.declared(), "{name}: the declared SampleClocks");
        assert!(!mock.declared().is_empty(), "{name}: a clock is declared");
    }
    // The expected values themselves, so that both providers erring alike is caught too
    // (they share `channels::Description`): like `x3xx.rs`'s comparisons, one case's
    // defaults and one case's ratios.
    let mock = MockRig::new(selector(&["0:A:ubx:0", "0:B:ubx:0"], &["0:A:ubx:0"]), &[]);
    assert_eq!(mock.effective[&key("radio.rx.c0.stream")], Value::from(0));
    assert_eq!(mock.effective[&key("radio.rx.c1.stream")], Value::from(-1));
    assert_eq!(mock.effective[&key("radio.rx.c0.frequency_hz")], Value::num(1e9).unwrap());
    assert_eq!(mock.effective[&key("radio.rx.c0.antenna")], Value::from("RX2"));
    assert_eq!(mock.effective[&key("radio.rx.s0.sample_rate_hz")], Value::num(1e6).unwrap());
    assert_eq!(mock.declared(), [("radio/rx/s0".to_owned(), (200, 1))]);
}

#[test]
fn vh_8_a_named_selector_prepares_as_the_unnamed_one_on_both_providers() {
    // RM-28: a name after `#` reaches no Provider's behaviour. For each layout, written plain and named: each
    // Provider's `effective` and declared SampleClocks equal its own plain ones and the other Provider's.
    let values = [("radio.rx.c0.frequency_hz", eq(Scalar::try_from(1.1e9).unwrap())), ("radio.tx.c0.stream", eq(Scalar::from(0))), ("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(10e6).unwrap()))];
    type Case<'a> = (&'a [&'a str], &'a [&'a str], &'a [&'a str], &'a [&'a str], Vec<&'a str>);
    let cases: [Case; 2] = [
        (&["0:A:ubx:0", "0:B:ubx:0"], &["0:A:ubx:0"], &["0:A:ubx:0#loop-rx", "0:B:ubx:0#mon"], &["0:A:ubx:0#loop-tx"], vec!["UBX", "UBX"]),
        (&["0:A:ubx:0", "0:B:basic_rx:0"], &["0:A:ubx:0"], &["0:A:ubx:0", "0:B:basic_rx:0#aux"], &["0:A:ubx:0#out"], vec!["UBX", "BasicRX"]),
    ];
    for (index, (rx, tx, named_rx, named_tx, front_ends)) in cases.into_iter().enumerate() {
        let rigs = |rx: &[&str], tx: &[&str], tag: &str| {
            let config = FakeConfig { front_ends: front_ends.clone(), serial: format!("vh8-named-{index}-{tag}"), ..FakeConfig::default() };
            (MockRig::new(selector(rx, tx), &values), UhdRig::new(selector(rx, tx), &values, config).unwrap())
        };
        let (plain_mock, plain_uhd) = rigs(rx, tx, "plain");
        let (named_mock, named_uhd) = rigs(named_rx, named_tx, "named");
        assert_eq!(plain_mock.effective, named_mock.effective, "case {index}: the Mock's effective configuration");
        assert_eq!(plain_uhd.effective, named_uhd.effective, "case {index}: the UHD Module's effective configuration");
        assert_eq!(named_mock.effective, named_uhd.effective, "case {index}: the Providers agree on a named selector");
        assert_eq!((plain_mock.declared(), plain_uhd.declared()), (named_mock.declared(), named_uhd.declared()), "case {index}: the declared SampleClocks");
        assert_eq!(named_mock.declared(), named_uhd.declared(), "case {index}");
        assert!(!named_mock.declared().is_empty());
    }
}

#[test]
fn vh_8_a_refusal_is_the_same_key_and_reason() {
    // Design 33 §6: the same Spec refused by both, with the same `(key, reason)` — the
    // `RM-` rules are the description's, which both Providers share.
    for constraints in [
        vec![("radio.rx.c2.gain_db", eq(Scalar::from(0)))],
        vec![("radio.rx.c0.frequency_hz", eq(Scalar::try_from(7e9).unwrap()))],
        vec![("radio.tx.c0.stream", eq(Scalar::from(0))), ("radio.rx.c0.antenna", eq(Scalar::from("TX/RX")))],
    ] {
        let selector = selector(&["0:A:ubx:0", "0:B:ubx:0"], &["0:A:ubx:0"]);
        let mock = MockRig::new(selector.clone(), &[]);
        let uhd = UhdRig::new(selector, &[], FakeConfig::default()).unwrap();
        let request = requested(&constraints);
        let norm = |report: &CoerceReport| -> Vec<(Key, String)> { report.rejected.iter().map(|rejection| (rejection.key.clone(), rejection.reason.clone())).collect() };
        let mock_reported = norm(&mock.mock.coerce(&request).unwrap());
        let uhd_reported = norm(&uhd.radio.coerce(&request).unwrap());
        assert!(!mock_reported.is_empty(), "the Mock refuses: {constraints:?}");
        assert_eq!(mock_reported, uhd_reported, "{constraints:?}");
    }
}

#[test]
fn vh_8_the_same_transmit_stream_clock() {
    // Design 33 §6 and RM-25: both Providers register the transmit stream 0's clock at
    // `arm` on its lattice. The arm instants differ, so each provider's own lattice formula
    // is checked (`origin == lattice(arm instant, ratio)`, `origin % 200 == 0`).
    let constraints = [("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let selector = selector(&["0:A:ubx:0"], &["0:A:ubx:0", "0:B:ubx:0"]);
    let mut mock = MockRig::new(selector.clone(), &constraints);
    // The arm instant is each rig's own now; Mock's is zero, so its clock's origin is 0's
    // lattice (RM-25: the first lattice instant at or after `arm`).
    mock.arm_start(ms(2_000) + ms(1));
    let mut uhd = UhdRig::new(selector, &constraints, FakeConfig::default()).unwrap();
    uhd.arm().unwrap();
    let mock_records = mock.records();
    let uhd_records = uhd.records();
    let mock_tx = mock_records.iter().find(|record| record.0 == "radio/tx/s0").unwrap_or_else(|| panic!("{mock_records:?}"));
    let uhd_tx = uhd_records.iter().find(|record| record.0 == "radio/tx/s0").unwrap_or_else(|| panic!("{uhd_records:?}"));
    assert_eq!(mock_tx.0, uhd_tx.0, "the stream node");
    assert_eq!(mock_tx.1, uhd_tx.1, "the ratio");
    assert_eq!(mock_tx.2, ezsdr_radio::timeline::lattice(0, Rational::new(200, 1).unwrap()).unwrap(), "the Mock's origin is its arm instant's lattice");
    assert_eq!(uhd_tx.2 % 200, 0, "the UHD's origin is on the lattice");
}

#[test]
fn vh_8_a_channel_less_stream_is_the_same_on_both() {
    // Design 33 §6, item 12's send (b-5): a `start_rx` on a receive stream with no channel
    // is carried out as nothing, and a burst on a transmit stream with no channel is
    // refused naming no transmit channel — the same treatment on both Providers, and
    // neither registers a clock for such a stream.
    let constraints: [(&str, Constraint); 0] = [];
    let selector = selector(&["0:A:ubx:0", "0:B:ubx:0"], &["0:A:ubx:0", "0:B:ubx:0"]);
    let mut mock = MockRig::new(selector.clone(), &constraints);
    let mut uhd = UhdRig::new(selector, &constraints, FakeConfig::default()).unwrap();
    let t0 = ms(2_000) + ms(1);
    mock.arm_start(t0);
    uhd.arm().unwrap();
    let uhd_t0 = uhd.time.now(uhd.root).unwrap().ticks_in(uhd.root).unwrap() + ms(2_000) + ms(1);
    uhd.start(uhd_t0).unwrap();
    uhd.wait_until(uhd_t0 + ms(5));
    uhd.settle(Wall::from_millis(5));
    mock.step(t0 + ms(5));
    // `start_rx` at the channel-less receive stream: carried out as nothing.
    let start = |node: &str| Action::Command {
        target: Target::Resource { resource: Ident::parse("radio").unwrap(), path: node.to_owned() },
        verb: Ident::parse("start_rx").unwrap(),
        params: BTreeMap::new(),
        at: None,
    };
    mock.push(start("rx/s1"));
    mock.step(t0 + ms(20));
    uhd.actions.actions.push((start("rx/s1"), false));
    uhd.wait_until(uhd_t0 + ms(20));
    uhd.settle(Wall::from_millis(5));
    // A burst at the channel-less transmit stream: refused, no transmit channel named.
    let waveform = ezsdr_kernel::manifest::ArtifactRef {
        id: Ident::parse("waveform").unwrap(),
        kind: ezsdr_kernel::spec::Namespace::parse("vh8").unwrap(),
        uri: "vh8://s1".to_owned(),
        hash: ContentHash::of(b"vh8-s1").unwrap(),
        size_bytes: 16,
        partial: false,
        marks: Vec::new(),
        continuity: Vec::new(),
    };
    let burst = |node: &str| Action::TxBurst {
        target: Target::Resource { resource: Ident::parse("radio").unwrap(), path: node.to_owned() },
        waveform: waveform.clone(),
        repeat: false,
        at: ezsdr_kernel::time::AbsoluteDeadline::new(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)),
        requested_at: None,
        late_policy: ezsdr_kernel::stream::LatePolicy::SendAsapAndFlag,
        metadata: BTreeMap::new(),
    };
    mock.push(burst("tx/s1"));
    mock.step(t0 + ms(30));
    uhd.actions.actions.push((burst("tx/s1"), false));
    uhd.wait_until(uhd_t0 + ms(30));
    uhd.settle(Wall::from_millis(5));
    let mock_rejected = rejected_reasons(&mock.events());
    let uhd_rejected = rejected_reasons(&uhd.events());
    assert_eq!(mock_rejected.len(), 1, "the Mock refuses the burst alone: {mock_rejected:?}");
    assert_eq!(uhd_rejected.len(), 1, "the UHD refuses the burst alone: {uhd_rejected:?}");
    for reason in [&mock_rejected[0], &uhd_rejected[0]] {
        assert!(reason.ends_with("no transmit channel is configured"), "{reason}");
    }
    for records in [mock.records(), uhd.records()] {
        assert!(records.iter().all(|record| !record.0.ends_with("/s1")), "no clock for a channel-less stream: {records:?}");
    }
}

/// Every `COMMAND_REJECTED` reason, in order (RM-22).
fn rejected_reasons(events: &[ezsdr_kernel::event::Event]) -> Vec<String> {
    events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).map(|event| event.payload["reason"].as_str().unwrap().to_owned()).collect()
}

// ---------------------------------------------------------------- the stream dimension

/// One Manifest segment row as both Providers name it (MR-27, UR-30): direction, stream,
/// the command that began the segment, its origin from T0, and its logical channels in block
/// order (design 33 D3's 6).
type SegmentRow = (String, u64, Option<u64>, i64, Vec<u64>);

/// A row's `sequence`: `null`, or an integer — a missing or other value fails (MR-27, UR-30).
fn sequence_of(row: &serde_json::Value) -> Option<u64> {
    let sequence = row.get("sequence").expect("a sequence");
    (!sequence.is_null()).then(|| sequence.as_u64().expect("an integer sequence"))
}

/// The Mock's `ezsdr.radio.mock.segments` rows (MR-27).
fn mock_segments(instance: &ProviderInstance, t0: i64) -> Vec<SegmentRow> {
    let rows = instance.sections[&Namespace::parse("ezsdr.radio.mock.segments").unwrap()].as_array().expect("the Mock's segments");
    let mut segments: Vec<SegmentRow> = rows
        .iter()
        .map(|row| {
            (
                row["direction"].as_str().expect("a direction").to_owned(),
                row["stream"].as_u64().expect("a stream"),
                sequence_of(row),
                row["origin"]["ticks"].as_i64().expect("an origin") - t0,
                row["channels"].as_array().expect("the channels").iter().map(|channel| channel.as_u64().expect("a channel")).collect(),
            )
        })
        .collect();
    segments.sort();
    segments
}

/// The UHD's `ezsdr.radio.uhd.timing` plan row as the same rows: its `rx` and `tx` arrays
/// name the direction the Mock's column names (UR-30).
fn uhd_plan(instance: &ProviderInstance, t0: i64) -> Vec<SegmentRow> {
    let timing = instance.sections[&Namespace::parse("ezsdr.radio.uhd.timing").unwrap()].as_array().expect("the UHD's timing");
    let plan = timing.iter().find(|row| row["what"] == "plan").expect("the plan row at the stop");
    let mut segments = Vec::new();
    for direction in ["rx", "tx"] {
        for row in plan[direction].as_array().expect("a direction's rows") {
            segments.push((
                direction.to_owned(),
                row["stream"].as_u64().expect("a stream"),
                sequence_of(row),
                row["origin"].as_i64().expect("an origin") - t0,
                row["channels"].as_array().expect("the channels").iter().map(|channel| channel.as_u64().expect("a channel")).collect(),
            ));
        }
    }
    segments.sort();
    segments
}

/// Two rigs of one stream comparison, armed and started at each one's own T0 — the Mock's
/// at `t0`, the UHD's at its own now plus the same lead after `arm` — and brought to the
/// same relative instant. The UHD's T0 is on the lattice every stream of these comparisons
/// shares (RM-25), so a cut and an origin at a whole number of milliseconds fall on the same
/// sample of both grids.
struct Started {
    mock: MockRig,
    uhd: UhdRig,
    t0: i64,
    uhd_t0: i64,
}

impl Started {
    fn new(selector: BTreeMap<Ident, Value>, constraints: &[(&str, Constraint)], ports: &[&str]) -> Started {
        Started::with_fake_config(selector, constraints, ports, FakeConfig::default())
    }

    fn with_fake_config(selector: BTreeMap<Ident, Value>, constraints: &[(&str, Constraint)], ports: &[&str], fake: FakeConfig) -> Started {
        let t0 = ms(2_000) + ms(1);
        let mut mock = MockRig::with_ports(selector.clone(), constraints, ports);
        mock.arm_start(t0);
        let mut uhd = UhdRig::with_ports(selector, constraints, fake, ports).unwrap();
        uhd.arm().unwrap();
        // The UHD's own T0: the same lead after `arm` as the Mock's T0 is after its own
        // arm. The 200-tick lattice is the one these comparisons' streams share — their
        // rates are 1, 2 or 4 Msps, whose ratios divide 200 (RM-25: T0 is a sample instant
        // of every stream); a comparison with a coarser stream widens this.
        let now = uhd.time.now(uhd.root).unwrap().ticks_in(uhd.root).unwrap();
        let uhd_t0 = ezsdr_radio::timeline::lattice(now, Rational::new(200, 1).unwrap()).unwrap() + ms(2_000) + ms(1);
        uhd.start(uhd_t0).unwrap();
        uhd.wait_until(uhd_t0 + ms(5));
        uhd.settle(Wall::from_millis(5));
        mock.step(t0 + ms(5));
        Started { mock, uhd, t0, uhd_t0 }
    }

    /// Both carried to relative `at` ms, then each stopped and cleaned up for the Manifest's
    /// sections.
    fn finish(mut self, at: i64) -> (ProviderInstance, ProviderInstance) {
        self.mock.step(self.t0 + ms(at));
        self.uhd.wait_until(self.uhd_t0 + ms(at));
        self.uhd.settle(Wall::from_millis(20));
        (self.mock.finish(), self.uhd.finish())
    }
}

fn rx_fault_pair(faults: &[(&str, i64)], fake_faults: Vec<FakeFault>) -> Started {
    let selector = selector(&["0:A:ubx:0"], &["0:A:ubx:0"]);
    let constraints = [("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(1e6).unwrap()))];
    let environment = BTreeMap::from([(
        Namespace::parse("sim.faults").unwrap(),
        serde_json::Value::Array(
            faults
                .iter()
                .map(|(fault, at_ms)| {
                    json!({
                        "at_ns": at_ms * 1_000_000,
                        "fault": fault,
                        "target": "radio",
                    })
                })
                .collect(),
        ),
    )]);
    let t0 = ms(2_001);
    let mut mock = MockRig::with_run(selector.clone(), &constraints, &["rx0"], BTreeMap::new(), environment);
    mock.arm_start(t0);
    let mut uhd = UhdRig::with_ports(selector, &constraints, FakeConfig { faults: fake_faults, ..FakeConfig::default() }, &["rx0"]).unwrap();
    uhd.arm().unwrap();
    let now = uhd.time.now(uhd.root).unwrap().ticks_in(uhd.root).unwrap();
    let uhd_t0 = ezsdr_radio::timeline::lattice(now, Rational::new(200, 1).unwrap()).unwrap() + ms(2_001);
    uhd.start(uhd_t0).unwrap();
    uhd.wait_until(uhd_t0 + ms(5));
    uhd.settle(Wall::from_millis(5));
    mock.step(t0 + ms(5));
    Started { mock, uhd, t0, uhd_t0 }
}

fn assert_rx_overflow_output(events: Vec<Event>, blocks: Vec<BlockRef>, cause: RxOverflowCause, flags: BlockFlags) -> (i64, u64) {
    let overflows: Vec<_> = events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW).collect();
    assert_eq!(overflows.len(), 1, "{events:?}");
    let payload = RxOverflowPayload::from_payload(&overflows[0].payload).unwrap();
    assert_eq!(payload.cause, cause);
    assert_eq!(overflows[0].payload.as_array().map(Vec::len), Some(9), "{}", overflows[0].payload);
    let gaps: Vec<_> = blocks.iter().filter(|block| block.header().lost.is_some()).collect();
    assert_eq!(gaps.len(), 1, "one reported jump block: {:?}", blocks.iter().map(|block| block.header().lost).collect::<Vec<_>>());
    let header = gaps[0].header();
    assert_eq!(header.flags, flags);
    let lost = header.lost.unwrap();
    assert_eq!(payload.lost, lost);
    let first = header.first_sample_time.ticks_in(header.first_sample_time.domain()).unwrap();
    let event_at = overflows[0].time.ticks_in(overflows[0].time.domain()).unwrap();
    assert_eq!(event_at, first - lost as i64, "the event names the first lost sample");
    let event_lost: u64 = overflows.iter().map(|event| RxOverflowPayload::from_payload(&event.payload).unwrap().lost).sum();
    let block_lost: u64 = gaps.iter().map(|block| block.header().lost.unwrap()).sum();
    assert_eq!(event_lost, block_lost, "T12: a nonempty event set accounts for each reported block jump");
    (event_at, lost)
}

fn assert_fault_rows(pair: &Started, expected_lost: &[u64]) {
    let rows = &pair.mock.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.faults").unwrap()];
    let rows = rows.as_array().unwrap();
    assert_eq!(rows.len(), expected_lost.len(), "{rows:?}");
    assert_eq!(rows.iter().map(|row| (row["applied"].as_bool().unwrap(), row["lost"].as_u64().unwrap())).collect::<Vec<_>>(), expected_lost.iter().map(|lost| (true, *lost)).collect::<Vec<_>>());
}

fn compare_rx_overflow(faults: &[(&str, i64)], fake_faults: Vec<FakeFault>, until_ms: i64, cause: RxOverflowCause, flags: BlockFlags, row_lost: &[u64], waits_for_gap: bool) {
    let mut pair = rx_fault_pair(faults, fake_faults);
    if waits_for_gap {
        assert_eq!(pair.mock.events().iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW).count(), 0, "the Mock waits for the block");
        assert_eq!(pair.uhd.events().iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW).count(), 0, "UHD waits for the block");
    }
    pair.mock.step(pair.t0 + ms(until_ms));
    pair.uhd.wait_until(pair.uhd_t0 + ms(until_ms));
    pair.uhd.settle(Wall::from_millis(20));
    let mock = assert_rx_overflow_output(pair.mock.events(), pair.mock.rx_links[0].take_blocks(), cause, flags);
    let uhd = assert_rx_overflow_output(pair.uhd.events(), pair.uhd.rx_links[0].take_blocks(), cause, flags);
    assert_eq!(mock, uhd, "the Providers agree in sample-index time and lost samples");
    assert_fault_rows(&pair, row_lost);
    pair.mock.finish();
    pair.uhd.finish();
}

#[test]
fn vh_8_a_reported_overrun_matches_the_block_jump() {
    compare_rx_overflow(&[("rx_overflow", 0)], vec![FakeFault::Overflow(Wall::ZERO)], 60, RxOverflowCause::Overrun, BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED, &[50_000], true);
}

#[test]
fn vh_8_a_reported_sequence_error_matches_the_block_jump() {
    compare_rx_overflow(
        &[("rx_sequence_error", 0)],
        vec![FakeFault::SequenceError(Wall::ZERO)],
        10,
        RxOverflowCause::Sequence,
        BlockFlags::GAP_BEFORE | BlockFlags::SEQ_DISCONTINUITY,
        &[2_000],
        false,
    );
}

#[test]
fn vh_8_reports_before_one_block_make_one_overrun_summary() {
    compare_rx_overflow(
        &[("rx_overflow", 0), ("rx_sequence_error", 50)],
        vec![FakeFault::Overflow(Wall::ZERO), FakeFault::SequenceError(Wall::ZERO)],
        60,
        RxOverflowCause::Overrun,
        BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED | BlockFlags::SEQ_DISCONTINUITY,
        &[50_000, 2_000],
        true,
    );
}

#[test]
fn vh_8_repeated_overruns_before_one_block_make_one_summary() {
    compare_rx_overflow(
        &[("rx_overflow", 0), ("rx_overflow", 50)],
        vec![FakeFault::Overflow(Wall::ZERO), FakeFault::Overflow(Wall::ZERO)],
        110,
        RxOverflowCause::Overrun,
        BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED,
        &[50_000, 50_000],
        true,
    );
}

#[test]
fn vh_8_a_report_without_a_following_block_emits_no_overflow() {
    let mut pair = rx_fault_pair(&[("rx_overflow", 0)], vec![FakeFault::Overflow(Wall::ZERO)]);
    pair.mock.stop_at("rx/s0");
    pair.uhd.stop_at("rx/s0");
    pair.mock.step(pair.t0 + ms(10));
    pair.uhd.wait_until(pair.uhd_t0 + ms(10));
    for _ in 0..1_000 {
        if pair.uhd.actions.queued() == 0 {
            break;
        }
        pair.uhd.settle(Wall::from_millis(1));
    }
    assert_eq!(pair.uhd.actions.queued(), 0, "the stop Action was taken");
    pair.uhd.settle(Wall::from_millis(2));
    assert_eq!(pair.mock.rx_links[0].take_blocks().len(), 0);
    assert_eq!(pair.uhd.rx_links[0].take_blocks().len(), 0);
    assert_eq!(pair.mock.events().iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW).count(), 0);
    assert_eq!(pair.uhd.events().iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW).count(), 0);
    assert_fault_rows(&pair, &[50_000]);
    let mock = pair.mock.finish();
    let uhd = pair.uhd.finish();
    assert_eq!(uhd.sections[&Namespace::parse("ezsdr.radio.uhd.stats").unwrap()]["rx_overflows"], 1, "the UHD report was received before stop despite no block carrying it");
    assert_eq!(mock.sections[&Namespace::parse("ezsdr.radio.mock.faults").unwrap()][0]["applied"], true);
}

/// The constraints of two receive streams, one channel each: c0 in stream 0 at 2 Msps, c1
/// in stream 1 at 1 Msps — a 200-tick lattice.
fn rx_two() -> Vec<(&'static str, Constraint)> {
    vec![
        ("radio.rx.c0.stream", eq(Scalar::from(0))),
        ("radio.rx.c1.stream", eq(Scalar::from(1))),
        ("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(2e6).unwrap())),
        ("radio.rx.s1.sample_rate_hz", eq(Scalar::try_from(1e6).unwrap())),
    ]
}

/// The same for two transmit streams.
fn tx_two() -> Vec<(&'static str, Constraint)> {
    vec![
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.c1.stream", eq(Scalar::from(1))),
        ("radio.tx.s0.sample_rate_hz", eq(Scalar::try_from(2e6).unwrap())),
        ("radio.tx.s1.sample_rate_hz", eq(Scalar::try_from(1e6).unwrap())),
    ]
}

fn cancellation_waveform() -> (ezsdr_kernel::manifest::ArtifactRef, BTreeMap<ContentHash, Arc<[u8]>>) {
    waveform_of(&[[0.25f32, 0.0f32], [0.5f32, 0.0f32]])
}

fn waveform_of(samples: &[[f32; 2]]) -> (ezsdr_kernel::manifest::ArtifactRef, BTreeMap<ContentHash, Arc<[u8]>>) {
    let mut bytes = Vec::new();
    for sample in samples {
        bytes.extend_from_slice(&sample[0].to_le_bytes());
        bytes.extend_from_slice(&sample[1].to_le_bytes());
    }
    let hash = ContentHash::of_bytes(&bytes);
    let waveform = ezsdr_kernel::manifest::ArtifactRef {
        id: Ident::parse("waveform").unwrap(),
        kind: Namespace::parse("ezsdr.input").unwrap(),
        uri: format!("mem:{hash}"),
        hash: hash.clone(),
        size_bytes: bytes.len() as u64,
        partial: false,
        marks: Vec::new(),
        continuity: Vec::new(),
    };
    (waveform, BTreeMap::from([(hash, Arc::<[u8]>::from(bytes))]))
}

fn tx_sample_time(clocks: &ClockRegistry, stream: usize, segment: usize, root_tick: i64) -> TimePoint {
    let node = format!("radio/tx/s{stream}");
    let mut records: Vec<_> = clocks.sample_clock_records().into_iter().filter(|record| record.stream.path() == node).collect();
    records.sort_by_key(|record| record.origin.ticks_in(record.root).unwrap());
    let record = &records[segment];
    let origin = record.origin.ticks_in(record.root).unwrap();
    let ratio = record.root_ticks_per_tick;
    let delta = i128::from(root_tick - origin).max(0);
    let sample = (delta * i128::from(ratio.den()) + i128::from(ratio.num()) - 1) / i128::from(ratio.num());
    TimePoint::new(record.domain, i64::try_from(sample).unwrap())
}

/// The device tick at which transmit sample `sample` of the stream's first clock segment is sent: the segment's origin plus `sample` root
/// ticks per sample, the exact inverse of `tx_sample_time` on the device's grid.
fn tx_device_tick(clocks: &ClockRegistry, stream: usize, sample: i64) -> i64 {
    let node = format!("radio/tx/s{stream}");
    let record = clocks.sample_clock_records().into_iter().filter(|record| record.stream.path() == node).min_by_key(|record| record.origin.ticks_in(record.root).unwrap()).unwrap();
    let ratio = record.root_ticks_per_tick;
    record.origin.ticks_in(record.root).unwrap() + i64::try_from(i128::from(sample) * i128::from(ratio.num()) / i128::from(ratio.den())).unwrap()
}

fn tx_burst_action(node: &str, waveform: &ezsdr_kernel::manifest::ArtifactRef, at: TimePoint) -> Action {
    tx_burst_action_with(node, waveform, at, ezsdr_kernel::stream::LatePolicy::SendAsapAndFlag)
}

fn tx_burst_action_with(node: &str, waveform: &ezsdr_kernel::manifest::ArtifactRef, at: TimePoint, late_policy: ezsdr_kernel::stream::LatePolicy) -> Action {
    Action::TxBurst {
        target: Target::Resource { resource: Ident::parse("radio").unwrap(), path: node.to_owned() },
        waveform: waveform.clone(),
        repeat: false,
        at: ezsdr_kernel::time::AbsoluteDeadline::new(at),
        requested_at: None,
        late_policy,
        metadata: BTreeMap::new(),
    }
}

fn timer_barrier(root: ClockDomainId, at: i64, token: u64) -> Action {
    Action::SetTimer { target: Target::Resource { resource: Ident::parse("radio").unwrap(), path: String::new() }, at: ezsdr_kernel::time::AbsoluteDeadline::new(TimePoint::new(root, at)), token }
}

struct CancellationPair {
    mock: MockRig,
    uhd: UhdRig,
    waveform: ezsdr_kernel::manifest::ArtifactRef,
    single: ezsdr_kernel::manifest::ArtifactRef,
    sized: Vec<(usize, ezsdr_kernel::manifest::ArtifactRef)>,
    /// Two-channel waveforms: `stereo(n)` holds `n` samples on each of two channels (for a two-channel stream).
    stereo: Vec<(usize, ezsdr_kernel::manifest::ArtifactRef)>,
    /// A 500-sample waveform of samples other than the repeats' (for a switch whose new samples are told apart on the device).
    tone: ezsdr_kernel::manifest::ArtifactRef,
    /// The oracle's `ramp(tag, len)` waveforms `new_with` prepared.
    ramps: Vec<((u8, usize), ezsdr_kernel::manifest::ArtifactRef)>,
    t0: i64,
    uhd_t0: i64,
    uhd_barriers: std::cell::Cell<usize>,
}

impl CancellationPair {
    fn new(selector: BTreeMap<Ident, Value>, constraints: &[(&str, Constraint)], ports: &[&str], fake: FakeConfig, environment: BTreeMap<Namespace, serde_json::Value>) -> CancellationPair {
        CancellationPair::new_with(selector, constraints, ports, fake, environment, &[])
    }

    /// As `new`, and with the oracle's `ramp(tag, len)` waveforms of `ramps` prepared too (O1: a distinct value per sample).
    fn new_with(
        selector: BTreeMap<Ident, Value>,
        constraints: &[(&str, Constraint)],
        ports: &[&str],
        fake: FakeConfig,
        environment: BTreeMap<Namespace, serde_json::Value>,
        ramps: &[(u8, usize)],
    ) -> CancellationPair {
        let (waveform, mut inputs) = cancellation_waveform();
        let (single, extra) = waveform_of(&[[0.25f32, 0.0f32]]);
        inputs.extend(extra);
        let mut sized = Vec::new();
        for samples in [4usize, 10, 31, 32, 40, 100, 500, 1_000] {
            let (artifact, extra) = waveform_of(&vec![[0.25f32, 0.0f32]; samples]);
            inputs.extend(extra);
            sized.push((samples, artifact));
        }
        let mut stereo = Vec::new();
        for samples in [1usize, 2, 3, 4, 10, 31, 32, 40] {
            let (artifact, extra) = waveform_of(&vec![[0.25f32, 0.0f32]; 2 * samples]);
            inputs.extend(extra);
            stereo.push((samples, artifact));
        }
        let (tone, extra) = waveform_of(&[[0.75f32, -0.25f32]; 500]);
        inputs.extend(extra);
        let mut prepared_ramps = Vec::new();
        for (tag, len) in ramps {
            let (artifact, extra) = waveform_of(&oracle::ramp(*tag, *len));
            inputs.extend(extra);
            prepared_ramps.push(((*tag, *len), artifact));
        }
        let t0 = ms(2_001);
        let mut mock = MockRig::with_run(selector.clone(), constraints, ports, inputs.clone(), environment);
        mock.arm_start(t0);
        let mut uhd = UhdRig::with_run(selector, constraints, fake, ports, inputs).unwrap();
        uhd.arm().unwrap();
        let now = uhd.time.now(uhd.root).unwrap().ticks_in(uhd.root).unwrap();
        let uhd_t0 = now + ms(2_001);
        uhd.start(uhd_t0).unwrap();
        uhd.wait_until(uhd_t0 + ms(5));
        uhd.settle(Wall::from_millis(1));
        mock.step(t0 + ms(5));
        CancellationPair { mock, uhd, waveform, single, sized, stereo, tone, ramps: prepared_ramps, t0, uhd_t0, uhd_barriers: std::cell::Cell::new(0) }
    }

    /// The oracle's `ramp(tag, len)`, one `new_with` prepared.
    fn ramp(&self, tag: u8, len: usize) -> &ezsdr_kernel::manifest::ArtifactRef {
        &self.ramps.iter().find(|(key, _)| *key == (tag, len)).expect("a prepared ramp").1
    }

    /// A waveform of `samples` samples, one of the four lengths `new` prepares.
    fn sized(&self, samples: usize) -> &ezsdr_kernel::manifest::ArtifactRef {
        &self.sized.iter().find(|(length, _)| *length == samples).expect("a prepared length").1
    }

    /// A two-channel waveform of `samples` samples on each channel (for a stream with two channels).
    fn stereo(&self, samples: usize) -> &ezsdr_kernel::manifest::ArtifactRef {
        &self.stereo.iter().find(|(length, _)| *length == samples).expect("a prepared length").1
    }

    fn burst(&self, node: &str, stream: usize, segment: usize, relative_ms: i64) -> Action {
        tx_burst_action(node, &self.waveform, tx_sample_time(&self.mock.clocks, stream, segment, self.t0 + ms(relative_ms)))
    }

    fn uhd_burst(&self, node: &str, stream: usize, segment: usize, relative_ms: i64) -> Action {
        tx_burst_action(node, &self.waveform, tx_sample_time(&self.uhd.clocks, stream, segment, self.uhd_t0 + ms(relative_ms)))
    }

    fn mock_barrier(&self, token: u64) {
        self.mock.push(timer_barrier(self.mock.root, self.t0 + ms(10), token));
    }

    fn uhd_barrier(&self, token: u64) {
        self.uhd.push(timer_barrier(self.uhd.root, self.uhd_t0 + ms(10), token));
        // Each barrier's own refusal: an earlier barrier's event must not satisfy a later one.
        self.uhd_barriers.set(self.uhd_barriers.get() + 1);
        self.uhd.wait_for_events("Action booking barrier", self.uhd_barriers.get(), |event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED && event.payload["kind"] == "set_timer");
    }
}

/// `submit` for untimed Actions: each Provider books them as the Kernel's untimed Actions (RM-15), with the late policy not consulted.
fn submit_untimed(pair: &mut CancellationPair, token: u64, relative_ms: i64, actions: impl FnOnce(&CancellationPair) -> Vec<(Action, Action)>) {
    pair.mock.step(pair.t0 + ms(relative_ms));
    pair.uhd.wait_until(pair.uhd_t0 + ms(relative_ms));
    for (mock, uhd) in actions(pair) {
        pair.mock.push_untimed(mock);
        pair.uhd.push_untimed(uhd);
    }
    pair.mock.step(pair.t0 + ms(relative_ms));
    pair.uhd_barrier(token);
}

/// The sample `samples` after the transmit sample at or after `root_tick`, in its own domain (S1's short bursts, UR-22).
fn tx_samples_after(clocks: &ClockRegistry, stream: usize, root_tick: i64, samples: i64) -> TimePoint {
    let at = tx_sample_time(clocks, stream, 0, root_tick);
    TimePoint::new(at.domain(), at.ticks_in(at.domain()).unwrap() + samples)
}

/// A burst of `waveform` at `samples` after the transmit sample at `relative_ms` from T0, on both Providers.
fn short_booking(pair: &CancellationPair, waveform: &ezsdr_kernel::manifest::ArtifactRef, stream: usize, relative_ms: i64, samples: i64) -> (Action, Action) {
    let node = format!("tx/s{stream}");
    let late_policy = ezsdr_kernel::stream::LatePolicy::DropAndFlag;
    let mock_at = tx_samples_after(&pair.mock.clocks, stream, pair.t0 + ms(relative_ms), samples);
    let uhd_at = tx_samples_after(&pair.uhd.clocks, stream, pair.uhd_t0 + ms(relative_ms), samples);
    (tx_burst_action_with(&node, waveform, mock_at, late_policy), tx_burst_action_with(&node, waveform, uhd_at, late_policy))
}

fn stop_both(node: &str) -> (Action, Action) {
    let stop = || Action::Stop { target: Some(Target::Resource { resource: Ident::parse("radio").unwrap(), path: node.to_owned() }) };
    (stop(), stop())
}

/// Advances both Providers to `relative_ms`, queues the actions, and waits until each Provider has judged them there.
fn submit(pair: &mut CancellationPair, token: u64, relative_ms: i64, actions: impl FnOnce(&CancellationPair) -> Vec<(Action, Action)>) {
    pair.mock.step(pair.t0 + ms(relative_ms));
    pair.uhd.wait_until(pair.uhd_t0 + ms(relative_ms));
    for (mock, uhd) in actions(pair) {
        pair.mock.push(mock);
        pair.uhd.push(uhd);
    }
    pair.mock.step(pair.t0 + ms(relative_ms));
    pair.uhd_barrier(token);
}

/// How many bookings a Provider refused at receipt because they overlap an accepted booking (RM-15, MR-16, UR-21). A refused row names no booking,
/// so the tests say which booking survived through the accepted bursts' sample counts.
fn overlap_refusals(rows: &[serde_json::Value]) -> usize {
    rows.iter().filter(|row| row["reason"].as_str().is_some_and(|reason| reason.contains("overlaps an accepted booking"))).count()
}

/// How many bookings a Provider refused at receipt because they start before an accepted booking it may already have handed (RM-15): a
/// different refusal from an overlap, with its own reason on both Providers.
fn handed_refusals(rows: &[serde_json::Value]) -> usize {
    rows.iter().filter(|row| row["reason"].as_str().is_some_and(|reason| reason.contains("may already be handed"))).count()
}

/// The late-policy drops of a run: the `rejected` rows that name one, and the targets (in their own sample domain's ticks) of the TIME_ERROR drops.
fn late_drops(rejected: &[serde_json::Value], events: &[ezsdr_kernel::event::Event]) -> (usize, Vec<i64>) {
    let rows = rejected.iter().filter(|row| row["reason"].as_str().is_some_and(|reason| reason.contains("the late policy dropped"))).count();
    let targets = events
        .iter()
        .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR && event.payload["outcome"] == "drop")
        .map(|event| event.payload["target"]["ticks"].as_i64().unwrap())
        .collect();
    (rows, targets)
}

fn manifest_rows(instance: &ProviderInstance, namespace: &str) -> Vec<serde_json::Value> {
    instance.sections[&Namespace::parse(namespace).unwrap()].as_array().unwrap().clone()
}

fn tx_rejected_rows(instance: &ProviderInstance, namespace: &str) -> Vec<serde_json::Value> {
    manifest_rows(instance, namespace).into_iter().filter(|row| row["kind"] == "tx_burst").collect()
}

fn mock_tx_blocks(instance: &ProviderInstance) -> u64 {
    instance.sections[&Namespace::parse("ezsdr.radio.mock.stats").unwrap()]["tx_blocks"].as_u64().unwrap()
}

fn tx_cancel_events(events: &[ezsdr_kernel::event::Event]) -> Vec<&ezsdr_kernel::event::Event> {
    events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED && event.payload["kind"] == "tx_burst").collect()
}

fn assert_mock_barriers(events: &[ezsdr_kernel::event::Event], expected: usize) {
    let barriers = events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED && event.payload["kind"] == "set_timer").count();
    assert_eq!(barriers, expected, "the Mock processed each queued-action barrier");
}

fn tx_send_calls(device: &FakeDevice, stream: usize) -> Vec<String> {
    device.calls().into_iter().filter(|call| call.starts_with(&format!("tx_send {stream} ")) && !call.contains(" at=- sob=true eob=true")).collect()
}

#[test]
fn vh_8_two_streams_each_prepare_alike() {
    // Design 33 §6: with two receive and two transmit streams, one channel in each,
    // `effective` and every stream's declared clock — node and ratio — are the same on both
    // Providers, stream 1's included (RM-29, RM-30).
    let mut constraints = rx_two();
    constraints.extend(tx_two());
    let selector = selector(&["0:A:ubx:0", "0:B:ubx:0"], &["0:A:ubx:0", "0:B:ubx:0"]);
    let mock = MockRig::with_ports(selector.clone(), &constraints, &["rx0", "rx1"]);
    let uhd = UhdRig::with_ports(selector, &constraints, FakeConfig::default(), &["rx0", "rx1"]).unwrap();
    assert_eq!(mock.effective, uhd.effective, "the effective configuration");
    assert_eq!(mock.declared(), uhd.declared(), "the declared SampleClocks");
    // The expected values themselves: stream 1 is declared on its own node at its own rate.
    assert_eq!(mock.declared(), [("radio/rx/s0".to_owned(), (100, 1)), ("radio/rx/s1".to_owned(), (200, 1)), ("radio/tx/s0".to_owned(), (100, 1)), ("radio/tx/s1".to_owned(), (200, 1)),]);
}

/// The plan comparison's run: two receive and two transmit streams asked for at connect, a
/// Stop and a `start_rx` of receive stream 1, all stopped at 700 ms. The rows are the Mock's and
/// the UHD's, from each one's T0.
fn planned() -> (Vec<SegmentRow>, Vec<SegmentRow>) {
    let (mock, uhd, _) = planned_with_anchors();
    (mock, uhd)
}

/// `planned`, and each rig's T0 and arm instant: the Mock's T0 and arm 0, the UHD's T0 and its `arm` timing row's `at`.
fn planned_with_anchors() -> (Vec<SegmentRow>, Vec<SegmentRow>, [(i64, i64); 2]) {
    let mut constraints = rx_two();
    constraints.extend(tx_two());
    let selector = selector(&["0:A:ubx:0", "0:B:ubx:0"], &["0:A:ubx:0", "0:B:ubx:0"]);
    let mut pair = Started::new(selector, &constraints, &["rx0", "rx1"]);
    let start = |root: ClockDomainId, at: i64| Action::Command {
        target: Target::Resource { resource: Ident::parse("radio").unwrap(), path: "rx/s1".to_owned() },
        verb: Ident::parse("start_rx").unwrap(),
        params: BTreeMap::new(),
        at: Some(ezsdr_kernel::time::AbsoluteDeadline::new(TimePoint::new(root, at))),
    };
    pair.mock.stop_at("rx/s1");
    pair.mock.step(pair.t0 + ms(10));
    pair.uhd.wait_until(pair.uhd_t0 + ms(10));
    pair.uhd.stop_at("rx/s1");
    pair.uhd.settle(Wall::from_millis(20));
    pair.mock.push(start(pair.mock.root, pair.t0 + ms(200)));
    pair.mock.step(pair.t0 + ms(100));
    pair.uhd.wait_until(pair.uhd_t0 + ms(100));
    pair.uhd.push(start(pair.uhd.root, pair.uhd_t0 + ms(200)));
    pair.uhd.settle(Wall::from_millis(20));
    let (t0, uhd_t0) = (pair.t0, pair.uhd_t0);
    let (mock, uhd) = pair.finish(700);
    let uhd_timing = uhd.sections[&Namespace::parse("ezsdr.radio.uhd.timing").unwrap()].as_array().unwrap();
    let uhd_arm = uhd_timing.iter().find(|row| row["what"] == "arm").expect("the arm row")["at"].as_i64().unwrap();
    (mock_segments(&mock, t0), uhd_plan(&uhd, uhd_t0), [(t0, 0), (uhd_t0, uhd_arm)])
}

#[test]
fn vh_8_the_same_manifest_plan_rows() {
    // Design 33 §6, D3's 6: after `stop`, the UHD's `timing` plan and the Mock's `segments`
    // name each stream's segments alike — the stream, its origin from T0 and its logical
    // channels (MR-27, UR-30). The command that began each segment is `sequence`'s own
    // comparison below. A transmit stream's initial segment starts at its Provider's own
    // `arm`, which the two rigs do not share, so its origin is not compared: it lies on the
    // stream's lattice at its arm instant, which each rig's `anchors` give (RM-25).
    let (mock_rows, uhd_rows, anchors) = planned_with_anchors();
    let without_sequence =
        |rows: Vec<SegmentRow>| -> Vec<(String, u64, i64, Vec<u64>)> { rows.into_iter().map(|(direction, stream, _, origin, channels)| (direction, stream, origin, channels)).collect() };
    let (mock, uhd) = (without_sequence(mock_rows), without_sequence(uhd_rows));
    let receive = |rows: &[(String, u64, i64, Vec<u64>)]| rows.iter().filter(|row| row.0 == "rx").cloned().collect::<Vec<_>>();
    assert_eq!(receive(&mock), receive(&uhd), "the receive segments of every stream, from each T0");
    assert_eq!(receive(&mock), [("rx".to_owned(), 0, 0, vec![0]), ("rx".to_owned(), 1, 0, vec![1]), ("rx".to_owned(), 1, ms(200), vec![1])]);
    for ((name, rows), (t0, arm)) in [("mock", &mock), ("uhd", &uhd)].into_iter().zip(anchors) {
        let transmit: Vec<_> = rows.iter().filter(|row| row.0 == "tx").collect();
        assert_eq!(transmit.iter().map(|row| (row.1, &row.3)).collect::<Vec<_>>(), [(0, &vec![0]), (1, &vec![1])], "{name}: one initial segment for each transmit stream");
        for (row, ratio) in transmit.iter().zip([100, 200]) {
            assert_eq!(row.2, ezsdr_radio::timeline::lattice(arm, Rational::new(ratio, 1).unwrap()).unwrap() - t0, "{name}: a transmit origin is its arm instant's lattice: {transmit:?}");
        }
    }
}

#[test]
fn vh_8_the_same_manifest_sequence() {
    // Design 33 §6, D3's 6, MR-27 and UR-30: `sequence` is the arrival order of the command
    // that began the segment, `null` for a segment no command began. Each Provider numbers
    // from its own origin (MockRadio's first command follows its faults, uhd-control's
    // follows the loss's 0), so the comparison is each number's rank among the run's.
    let (mock_rows, uhd_rows) = planned();
    let ranks = |rows: &[SegmentRow]| -> Vec<Option<usize>> {
        let mut numbers: Vec<u64> = rows.iter().filter_map(|row| row.2).collect();
        numbers.sort();
        numbers.dedup();
        rows.iter().map(|row| row.2.map(|n| numbers.binary_search(&n).unwrap())).collect()
    };
    assert_eq!(ranks(&mock_rows), ranks(&uhd_rows), "the same arrival order: {mock_rows:?} {uhd_rows:?}");
    assert_eq!(ranks(&mock_rows), [None, None, Some(0), None, None], "{mock_rows:?}");
}

#[test]
fn vh_8_tx_stream_stop_cancels_held_bursts() {
    let selector = selector(&["0:A:ubx:0"], &["0:A:ubx:0"]);
    let constraints = [("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let mut pair = CancellationPair::new(selector, &constraints, &["rx0"], FakeConfig::default(), BTreeMap::new());
    for relative in [100, 120] {
        pair.mock.push(pair.burst("tx/s0", 0, 0, relative));
        pair.uhd.push(pair.uhd_burst("tx/s0", 0, 0, relative));
    }
    pair.mock.stop_at("tx/s0");
    pair.mock_barrier(101);
    pair.mock.step(pair.t0 + ms(20));
    pair.uhd.stop_at("tx/s0");
    pair.uhd_barrier(101);
    pair.mock.step(pair.t0 + ms(200));
    pair.uhd.wait_until(pair.uhd_t0 + ms(200));
    let mock = pair.mock.finish_in_place();
    let mock_events = pair.mock.events();
    let uhd = pair.uhd.finish_in_place();
    let uhd_events = pair.uhd.events();
    let uhd_sends = tx_send_calls(&pair.uhd.device, 0);
    let mock_rejected = tx_rejected_rows(&mock, "ezsdr.radio.mock.rejected");
    let uhd_rejected = tx_rejected_rows(&uhd, "ezsdr.radio.uhd.rejected");
    assert_eq!(mock_rejected.len(), 2, "{mock_rejected:?}");
    assert_eq!(uhd_rejected.len(), 2, "{uhd_rejected:?}");
    assert!(mock_rejected.iter().all(|row| row["reason"] == "MR-25: cancelled by stop"), "{mock_rejected:?}");
    assert!(uhd_rejected.iter().all(|row| row["reason"] == "UR-26: cancelled by Stop"), "{uhd_rejected:?}");
    assert!(tx_cancel_events(&mock_events).is_empty());
    assert_mock_barriers(&mock_events, 1);
    assert!(tx_cancel_events(&uhd_events).is_empty());
    assert!(manifest_rows(&mock, "ezsdr.radio.mock.bursts").is_empty());
    assert_eq!(mock_tx_blocks(&mock), 0);
    assert!(manifest_rows(&uhd, "ezsdr.radio.uhd.bursts").is_empty());
    assert!(uhd_sends.is_empty(), "{uhd_sends:?}");
}

#[test]
fn vh_8_device_stop_cancels_held_bursts() {
    let selector = selector(&["0:A:ubx:0", "0:B:ubx:0"], &["0:A:ubx:0", "0:B:ubx:0"]);
    let mut constraints = rx_two();
    constraints.extend(tx_two());
    let mut pair = CancellationPair::new(selector, &constraints, &["rx0", "rx1"], FakeConfig::default(), BTreeMap::new());
    for stream in 0..2 {
        let node = format!("tx/s{stream}");
        for relative in [100, 120] {
            pair.mock.push(pair.burst(&node, stream, 0, relative));
            pair.uhd.push(pair.uhd_burst(&node, stream, 0, relative));
        }
    }
    pair.mock.stop_at("");
    pair.mock_barrier(102);
    pair.mock.step(pair.t0 + ms(20));
    pair.uhd.stop_at("");
    pair.uhd_barrier(102);
    pair.mock.step(pair.t0 + ms(200));
    pair.uhd.wait_until(pair.uhd_t0 + ms(200));
    let mock = pair.mock.finish_in_place();
    let mock_events = pair.mock.events();
    let uhd = pair.uhd.finish_in_place();
    let uhd_events = pair.uhd.events();
    let mock_rejected = tx_rejected_rows(&mock, "ezsdr.radio.mock.rejected");
    let uhd_rejected = tx_rejected_rows(&uhd, "ezsdr.radio.uhd.rejected");
    assert_eq!(mock_rejected.len(), 4, "{mock_rejected:?}");
    assert_eq!(uhd_rejected.len(), 4, "{uhd_rejected:?}");
    assert!(tx_cancel_events(&mock_events).is_empty());
    assert_mock_barriers(&mock_events, 1);
    assert!(tx_cancel_events(&uhd_events).is_empty());
    assert!(manifest_rows(&mock, "ezsdr.radio.mock.bursts").is_empty());
    assert_eq!(mock_tx_blocks(&mock), 0);
    assert!(manifest_rows(&uhd, "ezsdr.radio.uhd.bursts").is_empty());
    assert!(tx_send_calls(&pair.uhd.device, 0).is_empty());
    assert!(tx_send_calls(&pair.uhd.device, 1).is_empty());
}

#[test]
fn vh_8_provider_stop_cancels_held_bursts() {
    let selector = selector(&["0:A:ubx:0"], &["0:A:ubx:0"]);
    let constraints = [("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let mut pair = CancellationPair::new(selector, &constraints, &["rx0"], FakeConfig::default(), BTreeMap::new());
    for relative in [100, 120] {
        pair.mock.push(pair.burst("tx/s0", 0, 0, relative));
        pair.uhd.push(pair.uhd_burst("tx/s0", 0, 0, relative));
    }
    pair.mock_barrier(103);
    pair.mock.step(pair.t0 + ms(20));
    pair.uhd_barrier(103);
    let mock = pair.mock.finish_in_place();
    let mock_events = pair.mock.events();
    let uhd = pair.uhd.finish_in_place();
    let uhd_events = pair.uhd.events();
    let mock_rejected = tx_rejected_rows(&mock, "ezsdr.radio.mock.rejected");
    let uhd_rejected = tx_rejected_rows(&uhd, "ezsdr.radio.uhd.rejected");
    assert_eq!(mock_rejected.len(), 2, "{mock_rejected:?}");
    assert_eq!(uhd_rejected.len(), 2, "{uhd_rejected:?}");
    assert!(mock_rejected.iter().all(|row| row["reason"] == "MR-25: cancelled by stop"), "{mock_rejected:?}");
    assert!(uhd_rejected.iter().all(|row| row["reason"] == "UR-26: cancelled by Provider::stop"), "{uhd_rejected:?}");
    assert!(tx_cancel_events(&mock_events).is_empty());
    assert_mock_barriers(&mock_events, 1);
    assert!(tx_cancel_events(&uhd_events).is_empty());
    assert!(manifest_rows(&mock, "ezsdr.radio.mock.bursts").is_empty());
    assert_eq!(mock_tx_blocks(&mock), 0);
    assert!(manifest_rows(&uhd, "ezsdr.radio.uhd.bursts").is_empty());
    assert!(tx_send_calls(&pair.uhd.device, 0).is_empty());
}

#[test]
fn vh_8_loss_cancels_held_bursts_without_rejection() {
    let selector = selector(&["0:A:ubx:0"], &["0:A:ubx:0"]);
    let constraints = [("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let environment = BTreeMap::from([(Namespace::parse("sim.faults").unwrap(), json!([{ "at_ns": 50_000_000, "fault": "device_lost", "target": "radio" }]))]);
    let fake = FakeConfig { faults: vec![ezsdr_radio_uhd::FakeFault::Lost(Wall::from_millis(2_051))], ..FakeConfig::default() };
    let mut pair = CancellationPair::new(selector, &constraints, &["rx0"], fake, environment);
    pair.mock.push(pair.burst("tx/s0", 0, 0, 100));
    pair.mock_barrier(105);
    pair.mock.step(pair.t0 + ms(20));
    pair.uhd.push(pair.uhd_burst("tx/s0", 0, 0, 100));
    pair.uhd_barrier(105);
    pair.mock.time.advance_to(TimePoint::new(pair.mock.root, pair.t0 + ms(60))).unwrap();
    let error = pair.mock.mock.step(TimePoint::new(pair.mock.root, pair.t0 + ms(60))).unwrap_err();
    assert_eq!(error.kind, ezsdr_kernel::module_api::ModuleErrorKind::DeviceLost);
    pair.uhd.wait_until(pair.uhd_t0 + ms(60));
    pair.uhd.wait_for_event("FakeDevice loss", |event| event.kind.as_str() == ezsdr_kernel::event::EventKind::DEVICE_LOST);
    let mock = pair.mock.finish_in_place();
    let mock_events = pair.mock.events();
    let uhd = pair.uhd.finish_in_place();
    let uhd_events = pair.uhd.events();
    let mock_rejected = tx_rejected_rows(&mock, "ezsdr.radio.mock.rejected");
    let uhd_rejected = tx_rejected_rows(&uhd, "ezsdr.radio.uhd.rejected");
    assert_eq!(mock_rejected.len(), 1, "{mock_rejected:?}");
    assert_eq!(uhd_rejected.len(), 1, "{uhd_rejected:?}");
    assert_eq!(mock_rejected[0]["reason"], "MR-20: device lost");
    assert_eq!(uhd_rejected[0]["reason"], "UR-26: cancelled by Provider::stop");
    assert_mock_barriers(&mock_events, 1);
    assert!(mock_events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::COMMAND_REJECTED || event.payload["kind"] != "tx_burst"));
    assert!(uhd_events.iter().any(|event| event.kind.as_str() == ezsdr_kernel::event::EventKind::DEVICE_LOST));
    assert!(tx_cancel_events(&uhd_events).is_empty());
    assert!(manifest_rows(&mock, "ezsdr.radio.mock.bursts").is_empty());
    assert!(manifest_rows(&uhd, "ezsdr.radio.uhd.bursts").is_empty());
    assert_eq!(mock_tx_blocks(&mock), 0);
    assert!(tx_send_calls(&pair.uhd.device, 0).is_empty());
}

#[test]
fn vh_8_rx_stop_does_not_cancel_held_tx() {
    let selector = selector(&["0:A:ubx:0"], &["0:A:ubx:0"]);
    let constraints = [("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let mut pair = CancellationPair::new(selector, &constraints, &["rx0"], FakeConfig::default(), BTreeMap::new());
    pair.mock.push(pair.burst("tx/s0", 0, 0, 100));
    pair.mock.stop_at("rx/s0");
    pair.mock_barrier(110);
    pair.mock.step(pair.t0 + ms(20));
    pair.uhd.push(pair.uhd_burst("tx/s0", 0, 0, 100));
    pair.uhd.stop_at("rx/s0");
    pair.uhd_barrier(110);
    pair.mock.step(pair.t0 + ms(200));
    pair.uhd.wait_until(pair.uhd_t0 + ms(200));
    let mock = pair.mock.finish_in_place();
    let mock_events = pair.mock.events();
    let uhd = pair.uhd.finish_in_place();
    let uhd_events = pair.uhd.events();
    assert!(tx_rejected_rows(&mock, "ezsdr.radio.mock.rejected").is_empty());
    assert!(tx_rejected_rows(&uhd, "ezsdr.radio.uhd.rejected").is_empty());
    assert!(tx_cancel_events(&mock_events).is_empty());
    assert_mock_barriers(&mock_events, 1);
    assert!(tx_cancel_events(&uhd_events).is_empty());
    assert!(mock_events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::COMMAND_REJECTED || event.payload["kind"] != "stop"));
    assert!(uhd_events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::COMMAND_REJECTED || event.payload["kind"] != "stop"));
    let mock_rx = pair.mock.records().into_iter().find(|row| row.0 == "radio/rx/s0").unwrap();
    assert!(mock_rx.3.is_some(), "the receive Stop ended its clock: {mock_rx:?}");
    assert!(pair.uhd.device.calls().iter().any(|call| call == "rx_stop 0 now"), "the receive Stop reached FakeDevice");
    assert_eq!(manifest_rows(&mock, "ezsdr.radio.mock.bursts").len(), 1);
    assert_eq!(manifest_rows(&uhd, "ezsdr.radio.uhd.bursts").len(), 1);
    assert!(mock_tx_blocks(&mock) > 0);
    assert!(!tx_send_calls(&pair.uhd.device, 0).is_empty());
}

#[test]
fn vh_8_tx_stop_allows_a_later_burst() {
    let selector = selector(&["0:A:ubx:0"], &["0:A:ubx:0"]);
    let constraints = [("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let mut pair = CancellationPair::new(selector, &constraints, &["rx0"], FakeConfig::default(), BTreeMap::new());
    pair.mock.push(pair.burst("tx/s0", 0, 0, 100));
    pair.mock.stop_at("tx/s0");
    pair.mock.push(pair.burst("tx/s0", 0, 0, 150));
    pair.mock_barrier(111);
    pair.mock.step(pair.t0 + ms(20));
    pair.uhd.push(pair.uhd_burst("tx/s0", 0, 0, 100));
    pair.uhd.stop_at("tx/s0");
    pair.uhd.push(pair.uhd_burst("tx/s0", 0, 0, 150));
    pair.uhd_barrier(111);
    pair.mock.step(pair.t0 + ms(200));
    pair.uhd.wait_until(pair.uhd_t0 + ms(200));
    let mock = pair.mock.finish_in_place();
    let mock_events = pair.mock.events();
    let uhd = pair.uhd.finish_in_place();
    let uhd_events = pair.uhd.events();
    let mock_rejected = tx_rejected_rows(&mock, "ezsdr.radio.mock.rejected");
    let uhd_rejected = tx_rejected_rows(&uhd, "ezsdr.radio.uhd.rejected");
    assert_eq!(mock_rejected.len(), 1, "{mock_rejected:?}");
    assert_eq!(uhd_rejected.len(), 1, "{uhd_rejected:?}");
    assert_eq!(mock_rejected[0]["reason"], "MR-25: cancelled by stop");
    assert_eq!(uhd_rejected[0]["reason"], "UR-26: cancelled by Stop");
    assert!(tx_cancel_events(&mock_events).is_empty());
    assert_mock_barriers(&mock_events, 1);
    assert!(tx_cancel_events(&uhd_events).is_empty());
    assert_eq!(manifest_rows(&mock, "ezsdr.radio.mock.bursts").len(), 1);
    assert_eq!(manifest_rows(&uhd, "ezsdr.radio.uhd.bursts").len(), 1);
    assert!(mock_tx_blocks(&mock) > 0);
    assert!(!tx_send_calls(&pair.uhd.device, 0).is_empty());
}

#[test]
fn vh_8_tx_stream_stop_leaves_the_other_stream_held() {
    let selector = selector(&["0:A:ubx:0", "0:B:ubx:0"], &["0:A:ubx:0", "0:B:ubx:0"]);
    let mut constraints = rx_two();
    constraints.extend(tx_two());
    let mut pair = CancellationPair::new(selector, &constraints, &["rx0", "rx1"], FakeConfig::default(), BTreeMap::new());
    for (stream, relative) in [(0, 100), (1, 120)] {
        let node = format!("tx/s{stream}");
        pair.mock.push(pair.burst(&node, stream, 0, relative));
        pair.uhd.push(pair.uhd_burst(&node, stream, 0, relative));
    }
    pair.mock.stop_at("tx/s1");
    pair.mock_barrier(112);
    pair.mock.step(pair.t0 + ms(20));
    pair.uhd.stop_at("tx/s1");
    pair.uhd_barrier(112);
    pair.mock.step(pair.t0 + ms(200));
    pair.uhd.wait_until(pair.uhd_t0 + ms(200));
    let mock = pair.mock.finish_in_place();
    let mock_events = pair.mock.events();
    let uhd = pair.uhd.finish_in_place();
    let uhd_events = pair.uhd.events();
    let mock_rejected = tx_rejected_rows(&mock, "ezsdr.radio.mock.rejected");
    let uhd_rejected = tx_rejected_rows(&uhd, "ezsdr.radio.uhd.rejected");
    assert_eq!(mock_rejected.len(), 1, "{mock_rejected:?}");
    assert_eq!(uhd_rejected.len(), 1, "{uhd_rejected:?}");
    assert_eq!(mock_rejected[0]["reason"], "MR-25: cancelled by stop");
    assert_eq!(uhd_rejected[0]["reason"], "UR-26: cancelled by Stop");
    assert!(tx_cancel_events(&mock_events).is_empty());
    assert_mock_barriers(&mock_events, 1);
    assert!(tx_cancel_events(&uhd_events).is_empty());
    assert_eq!(manifest_rows(&mock, "ezsdr.radio.mock.bursts").len(), 1);
    assert_eq!(manifest_rows(&uhd, "ezsdr.radio.uhd.bursts").len(), 1);
    assert!(mock_tx_blocks(&mock) > 0);
    assert!(!tx_send_calls(&pair.uhd.device, 0).is_empty(), "stream 0's future burst is sent");
    assert!(tx_send_calls(&pair.uhd.device, 1).is_empty(), "stream 1's held burst is cancelled");
}

#[test]
fn s1_parity_short_burst_front_s_s3_s5() {
    // UR-22, RM-15 (S1): a one-sample burst at `s` is sent padded to four, so its front is `s + 19` on both Providers: its padded end plus
    // the gap (15 samples at 1 MS/s). Bookings at `s + 3`, `s + 5` and `s + 9` are refused at receipt; `s + 19` is accepted and plays on
    // time, at its target and with no TIME_ERROR.
    let selector = selector(&["0:A:ubx:0"], &["0:A:ubx:0"]);
    let constraints = [("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let mut pair = CancellationPair::new(selector, &constraints, &["rx0"], FakeConfig::default(), BTreeMap::new());
    let single = pair.single.clone();
    submit(&mut pair, 201, 91, |pair| vec![short_booking(pair, &single, 0, 100, 0)]);
    submit(&mut pair, 202, 91, |pair| vec![short_booking(pair, &single, 0, 100, 3), short_booking(pair, &single, 0, 100, 5), short_booking(pair, &single, 0, 100, 9)]);
    submit(&mut pair, 203, 91, |pair| vec![short_booking(pair, &single, 0, 100, 19)]);
    pair.uhd.wait_until(pair.uhd_t0 + ms(140));
    pair.mock.step(pair.t0 + ms(140));
    let mock = pair.mock.finish_in_place();
    let mock_events = pair.mock.events();
    let uhd = pair.uhd.finish_in_place();
    let uhd_events = pair.uhd.events();
    let mock_rejected = tx_rejected_rows(&mock, "ezsdr.radio.mock.rejected");
    let uhd_rejected = tx_rejected_rows(&uhd, "ezsdr.radio.uhd.rejected");
    assert_eq!(overlap_refusals(&mock_rejected), 3, "the Mock refuses `s + 3`, `s + 5` and `s + 9`: {mock_rejected:?}");
    assert_eq!(overlap_refusals(&uhd_rejected), 3, "UHD refuses `s + 3`, `s + 5` and `s + 9`: {uhd_rejected:?}");
    assert_eq!(late_drops(&mock_rejected, &mock_events), (0, Vec::new()), "nothing is late on the Mock");
    assert_eq!(late_drops(&uhd_rejected, &uhd_events), (0, Vec::new()), "nothing is late on UHD");
    assert!(mock_events.iter().chain(&uhd_events).all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "no TIME_ERROR: `s + 19` is on time");
    for (case, rows) in [("Mock", manifest_rows(&mock, "ezsdr.radio.mock.bursts")), ("UHD", manifest_rows(&uhd, "ezsdr.radio.uhd.bursts"))] {
        let mut starts: Vec<i64> = rows.iter().map(|row| row["target"]["ticks"].as_i64().unwrap()).collect();
        starts.sort_unstable();
        assert_eq!(starts.len(), 2, "{case}: the one-sample burst and `s + 19`: {rows:?}");
        assert_eq!(starts[1] - starts[0], 19, "{case}: the admitted burst's target is `s + 19`");
        assert!(rows.iter().all(|row| row["late_by"].is_null()), "{case}: nothing is late: {rows:?}");
    }
}

#[test]
fn s1_parity_stop_during_padded_device_burst() {
    // UR-22 (S1, TBD-m2): after a one-sample burst at `s` is dispatched and then stopped, the padded front `s + 5` survives the Stop: a booking at `s + 3` is refused at receipt on both.
    let selector = selector(&["0:A:ubx:0"], &["0:A:ubx:0"]);
    let constraints = [("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let mut pair = CancellationPair::new(selector, &constraints, &["rx0"], FakeConfig::default(), BTreeMap::new());
    let single = pair.single.clone();
    submit(&mut pair, 204, 91, |pair| vec![short_booking(pair, &single, 0, 100, 0)]);
    submit(&mut pair, 205, 92, |_| vec![]);
    assert!(!tx_send_calls(&pair.uhd.device, 0).is_empty(), "UHD dispatched A's padded packet before the Stop: {:?}", pair.uhd.device.calls());
    submit(&mut pair, 206, 92, |pair| vec![stop_both("tx/s0"), short_booking(pair, &single, 0, 100, 3)]);
    pair.uhd.wait_until(pair.uhd_t0 + ms(140));
    pair.mock.step(pair.t0 + ms(140));
    let mock = pair.mock.finish_in_place();
    let uhd = pair.uhd.finish_in_place();
    let mock_rejected = tx_rejected_rows(&mock, "ezsdr.radio.mock.rejected");
    let uhd_rejected = tx_rejected_rows(&uhd, "ezsdr.radio.uhd.rejected");
    assert_eq!(overlap_refusals(&mock_rejected), 1, "the Mock refuses `s + 3`: {mock_rejected:?}");
    assert_eq!(overlap_refusals(&uhd_rejected), 1, "UHD refuses `s + 3`: {uhd_rejected:?}");
    assert_eq!(manifest_rows(&mock, "ezsdr.radio.mock.bursts").len(), 1, "A only");
    assert_eq!(manifest_rows(&uhd, "ezsdr.radio.uhd.bursts").len(), 1, "A only");
}

#[test]
fn s1_parity_padded_front_survives_normal_close() {
    // UR-22 (S1, TBD-m2): with no Stop, the padded device burst that closed at once still sets its front at `s + 19`: a booking at `s + 3` is refused at receipt on both, `s + 19` is on time.
    let selector = selector(&["0:A:ubx:0"], &["0:A:ubx:0"]);
    let constraints = [("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let mut pair = CancellationPair::new(selector, &constraints, &["rx0"], FakeConfig::default(), BTreeMap::new());
    let single = pair.single.clone();
    submit(&mut pair, 213, 91, |pair| vec![short_booking(pair, &single, 0, 100, 0)]);
    submit(&mut pair, 214, 92, |_| vec![]);
    assert!(!tx_send_calls(&pair.uhd.device, 0).is_empty(), "UHD dispatched A's padded packet before the booking: {:?}", pair.uhd.device.calls());
    submit(&mut pair, 215, 92, |pair| vec![short_booking(pair, &single, 0, 100, 3), short_booking(pair, &single, 0, 100, 19)]);
    pair.uhd.wait_until(pair.uhd_t0 + ms(140));
    pair.mock.step(pair.t0 + ms(140));
    let mock = pair.mock.finish_in_place();
    let mock_events = pair.mock.events();
    let uhd = pair.uhd.finish_in_place();
    let uhd_events = pair.uhd.events();
    let mock_rejected = tx_rejected_rows(&mock, "ezsdr.radio.mock.rejected");
    let uhd_rejected = tx_rejected_rows(&uhd, "ezsdr.radio.uhd.rejected");
    assert_eq!(overlap_refusals(&mock_rejected), 1, "the Mock refuses `s + 3`: {mock_rejected:?}");
    assert_eq!(overlap_refusals(&uhd_rejected), 1, "UHD refuses `s + 3`: {uhd_rejected:?}");
    assert_eq!(late_drops(&mock_rejected, &mock_events), (0, Vec::new()), "nothing is late on the Mock");
    assert_eq!(late_drops(&uhd_rejected, &uhd_events), (0, Vec::new()), "nothing is late on UHD");
    assert_eq!(manifest_rows(&mock, "ezsdr.radio.mock.bursts").len(), 2, "A and `s + 19`");
    assert_eq!(manifest_rows(&uhd, "ezsdr.radio.uhd.bursts").len(), 2, "A and `s + 19`");
}

#[test]
fn s1_parity_short_head_region_is_end_plus_gap() {
    // RM-15, UR-22 (S1, sequences 1 and 2): a closed short head of `n` samples at `s` fronts at `s + max(n, 4) + g` on both Providers, `g` the
    // gap of the stream's rate (15 samples at 1 MS/s, 30 at 2; written out here, not read from the record). A booking at `s + max(n, 4) + k` is
    // refused at receipt for `k < g`, and at `k = g` it is accepted and plays on time at its target. A head of 33 samples holds a tail: its
    // front is its end plus four (`e + 3` refused, `e + 4` accepted).
    for (rate, gap) in [(1e6, 15i64), (2e6, 30)] {
        for n in [1usize, 2, 3, 4, 10, 31, 32, 33] {
            let mut pair = CancellationPair::new(
                selector(&["0:A:ubx:0"], &["0:A:ubx:0", "0:B:ubx:0"]),
                &[
                    ("radio.rx.c0.stream", eq(Scalar::from(0))),
                    ("radio.tx.c0.stream", eq(Scalar::from(0))),
                    ("radio.tx.c1.stream", eq(Scalar::from(0))),
                    ("radio.tx.s0.sample_rate_hz", eq(Scalar::try_from(rate).unwrap())),
                ],
                &["rx0"],
                FakeConfig::default(),
                BTreeMap::new(),
            );
            let head = pair.stereo(if n == 33 { 40 } else { n }).clone();
            let one = pair.stereo(1).clone();
            // A 33-sample waveform is not prepared (the prepared lengths are 1 to 4, 10, 31, 32, 40): a 40-sample head is long in the same way (a held tail, front end + 4).
            let (length, front) = if n == 33 { (40i64, 4i64) } else { (n.max(4) as i64, gap) };
            submit(&mut pair, 400, 91, |pair| vec![short_booking(pair, &head, 0, 100, 0)]);
            // All the probes are judged at one receipt, as the other parity tests judge a batch.
            // For the held tail the probes start at `e + 1`: a booking at `e` itself joins the chain, which moves its front.
            let first = i64::from(n == 33);
            submit(&mut pair, 401, 91, |pair| (first..=front).map(|k| short_booking(pair, &one, 0, 100, length + k)).collect());
            pair.uhd.wait_until(pair.uhd_t0 + ms(140));
            pair.mock.step(pair.t0 + ms(140));
            let mock = pair.mock.finish_in_place();
            let mock_events = pair.mock.events();
            let uhd = pair.uhd.finish_in_place();
            let uhd_events = pair.uhd.events();
            let refusals = front - first;
            for (case, rejected, events, rows) in [
                ("Mock", tx_rejected_rows(&mock, "ezsdr.radio.mock.rejected"), &mock_events, manifest_rows(&mock, "ezsdr.radio.mock.bursts")),
                ("UHD", tx_rejected_rows(&uhd, "ezsdr.radio.uhd.rejected"), &uhd_events, manifest_rows(&uhd, "ezsdr.radio.uhd.bursts")),
            ] {
                assert_eq!(overlap_refusals(&rejected) as i64, refusals, "{case}, {rate} S/s, n = {n}: `s + length + k` is refused below the front: {rejected:?}");
                assert_eq!(late_drops(&rejected, events), (0, Vec::new()), "{case}, {rate} S/s, n = {n}: nothing is late");
                assert!(events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "{case}, {rate} S/s, n = {n}: no TIME_ERROR");
                let mut starts: Vec<i64> = rows.iter().map(|row| row["target"]["ticks"].as_i64().unwrap()).collect();
                starts.sort_unstable();
                if n == 33 {
                    assert!(starts.len() >= 2 && starts.contains(&(starts[0] + length + front)), "{case}, {rate} S/s: the booking at the held tail's front plays: {rows:?}");
                } else {
                    assert_eq!(starts.len(), 2, "{case}, {rate} S/s, n = {n}: the head and `s + max(n, 4) + g`: {rows:?}");
                    assert_eq!(starts[1] - starts[0], length + gap, "{case}, {rate} S/s, n = {n}: the admitted booking's target");
                }
                assert!(rows.iter().all(|row| row["late_by"].is_null()), "{case}, {rate} S/s, n = {n}: nothing is late: {rows:?}");
            }
        }
    }
}

#[test]
fn f_parity_two_transmit_streams_front_at_their_own_gap() {
    // RM-15: a front is per stream and the gap is its own stream's: stream 0 at 2 MS/s keeps 30 samples past a one-sample head's padded end,
    // stream 1 at 1 MS/s keeps 15. A record armed with one stream's gap for both Providers' every stream would refuse or accept the other's
    // probe wrongly. This arms through each Provider's production arm site (the Mock's `prepare`, uhd's `Control::new`).
    let selector = selector(&["0:A:ubx:0", "0:B:ubx:0"], &["0:A:ubx:0", "0:B:ubx:0"]);
    let mut constraints = rx_two();
    constraints.extend(tx_two());
    let mut pair = CancellationPair::new(selector, &constraints, &["rx0", "rx1"], FakeConfig::default(), BTreeMap::new());
    let single = pair.single.clone();
    submit(&mut pair, 501, 91, |pair| vec![short_booking(pair, &single, 0, 100, 0), short_booking(pair, &single, 1, 100, 0)]);
    // One sample before each stream's front: refused; at each front: accepted.
    submit(&mut pair, 502, 91, |pair| {
        vec![short_booking(pair, &single, 0, 100, 4 + 29), short_booking(pair, &single, 1, 100, 4 + 14), short_booking(pair, &single, 0, 100, 4 + 30), short_booking(pair, &single, 1, 100, 4 + 15)]
    });
    pair.uhd.wait_until(pair.uhd_t0 + ms(140));
    pair.mock.step(pair.t0 + ms(140));
    let mock = pair.mock.finish_in_place();
    let uhd = pair.uhd.finish_in_place();
    let mock_rejected = tx_rejected_rows(&mock, "ezsdr.radio.mock.rejected");
    let uhd_rejected = tx_rejected_rows(&uhd, "ezsdr.radio.uhd.rejected");
    assert_eq!(overlap_refusals(&mock_rejected), 2, "the Mock refuses one probe on each stream: {mock_rejected:?}");
    assert_eq!(overlap_refusals(&uhd_rejected), 2, "UHD refuses one probe on each stream: {uhd_rejected:?}");
    // Which bursts played pins which probes were refused: each stream's head and its probe at its own front (s0 + 34 and s1 + 19), not s0 + 33
    // and s1 + 18 (a Mock that armed both streams with the 15-sample gap would take s0 + 33 and refuse s0 + 34).
    for (case, instance, namespace, clocks, base) in [("Mock", &mock, "ezsdr.radio.mock.bursts", &pair.mock.clocks, pair.t0), ("UHD", &uhd, "ezsdr.radio.uhd.bursts", &pair.uhd.clocks, pair.uhd_t0)] {
        let at = |stream: usize, samples: i64| serde_json::to_value(tx_samples_after(clocks, stream, base + ms(100), samples)).unwrap();
        let key = |value: &serde_json::Value| (value["domain"]["local"].as_u64(), value["ticks"].as_i64());
        let mut expected = vec![at(0, 0), at(1, 0), at(0, 4 + 30), at(1, 4 + 15)];
        let mut played: Vec<serde_json::Value> = manifest_rows(instance, namespace).iter().map(|row| row["target"].clone()).collect();
        expected.sort_by_key(key);
        played.sort_by_key(key);
        assert_eq!(played, expected, "{case}: the heads and the probes at their own stream's front played");
    }
}

#[test]
fn s1_parity_grouped_and_split_fronts() {
    // UR-22 (S1): a front is per stream. Stream 1's one-sample burst sets its front at `s + 5`: its booking at `s + 3` is refused at receipt; stream 0 has no front, so its booking at `s + 3` is on time.
    let selector = selector(&["0:A:ubx:0", "0:B:ubx:0"], &["0:A:ubx:0", "0:B:ubx:0"]);
    let mut constraints = rx_two();
    constraints.extend(tx_two());
    let mut pair = CancellationPair::new(selector, &constraints, &["rx0", "rx1"], FakeConfig::default(), BTreeMap::new());
    let single = pair.single.clone();
    submit(&mut pair, 207, 91, |pair| vec![short_booking(pair, &single, 1, 100, 0)]);
    submit(&mut pair, 208, 91, |pair| vec![short_booking(pair, &single, 1, 100, 3), short_booking(pair, &single, 0, 100, 3)]);
    pair.uhd.wait_until(pair.uhd_t0 + ms(140));
    pair.mock.step(pair.t0 + ms(140));
    let mock = pair.mock.finish_in_place();
    let uhd = pair.uhd.finish_in_place();
    let mock_rejected = tx_rejected_rows(&mock, "ezsdr.radio.mock.rejected");
    let uhd_rejected = tx_rejected_rows(&uhd, "ezsdr.radio.uhd.rejected");
    assert_eq!(overlap_refusals(&mock_rejected), 1, "{mock_rejected:?}");
    assert_eq!(overlap_refusals(&uhd_rejected), 1, "{uhd_rejected:?}");
    assert_eq!(manifest_rows(&mock, "ezsdr.radio.mock.bursts").len(), 2, "stream 1's burst and stream 0's booking");
    assert_eq!(manifest_rows(&uhd, "ezsdr.radio.uhd.bursts").len(), 2, "stream 1's burst and stream 0's booking");
}

#[test]
fn s1_parity_payload_cut_separate_from_padding() {
    // UR-22 (S1): a two-sample burst is padded to four on the wire; its record counts the two samples of payload only. (A padded short send closes at once, so Stop cannot cut it.) The booking is SendAsapAndFlag: no instant here is the assertion, so a late host moves the burst and never drops it.
    let selector = selector(&["0:A:ubx:0"], &["0:A:ubx:0"]);
    let constraints = [("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let mut pair = CancellationPair::new(selector, &constraints, &["rx0"], FakeConfig::default(), BTreeMap::new());
    let waveform = pair.waveform.clone();
    submit(&mut pair, 209, 91, |pair| vec![o1_asap(pair, &waveform, 100, false, false)]);
    pair.uhd.wait_until(o1_uhd_now(&pair) + ms(30));
    pair.mock.step(pair.t0 + ms(110));
    let mock = pair.mock.finish_in_place();
    let uhd = pair.uhd.finish_in_place();
    let mock_rows = manifest_rows(&mock, "ezsdr.radio.mock.bursts");
    let uhd_rows = manifest_rows(&uhd, "ezsdr.radio.uhd.bursts");
    assert_eq!(mock_rows.len(), 1, "{mock_rows:?}");
    assert_eq!(uhd_rows.len(), 1, "{uhd_rows:?}");
    assert_eq!(mock_rows[0]["samples"], 2, "{mock_rows:?}");
    assert_eq!(uhd_rows[0]["samples"], 2, "{uhd_rows:?}");
    assert_eq!(mock_tx_blocks(&mock), 1);
    assert_eq!(tx_send_calls(&pair.uhd.device, 0).len(), 1, "{:?}", pair.uhd.device.calls());
}

#[test]
fn s1_parity_short_burst_then_adjacent_burst_before_and_after_dispatch() {
    // UR-22 (S1, N3): a one-sample burst at `s` is padded and closed; an adjacent burst at `s + 1` is refused at receipt on both Providers, whether it is booked before the dispatch or after it.
    let mut before = CancellationPair::new(
        selector(&["0:A:ubx:0"], &["0:A:ubx:0"]),
        &[("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))],
        &["rx0"],
        FakeConfig::default(),
        BTreeMap::new(),
    );
    let single = before.single.clone();
    submit(&mut before, 210, 91, |before| vec![short_booking(before, &single, 0, 100, 0), short_booking(before, &single, 0, 100, 1)]);
    before.uhd.wait_until(before.uhd_t0 + ms(110));
    before.mock.step(before.t0 + ms(110));
    let mut after = CancellationPair::new(
        selector(&["0:A:ubx:0"], &["0:A:ubx:0"]),
        &[("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))],
        &["rx0"],
        FakeConfig::default(),
        BTreeMap::new(),
    );
    let single = after.single.clone();
    submit(&mut after, 211, 91, |after| vec![short_booking(after, &single, 0, 100, 0)]);
    submit(&mut after, 212, 91, |after| vec![short_booking(after, &single, 0, 100, 1)]);
    after.uhd.wait_until(after.uhd_t0 + ms(110));
    after.mock.step(after.t0 + ms(110));
    for (name, rig) in [("booked before", before), ("booked after", after)] {
        let mut rig = rig;
        let mock = rig.mock.finish_in_place();
        let mock_events = rig.mock.events();
        let uhd = rig.uhd.finish_in_place();
        let uhd_events = rig.uhd.events();
        assert_eq!(overlap_refusals(&tx_rejected_rows(&mock, "ezsdr.radio.mock.rejected")), 1, "{name}: the Mock");
        assert_eq!(overlap_refusals(&tx_rejected_rows(&uhd, "ezsdr.radio.uhd.rejected")), 1, "{name}: UHD");
        assert_eq!(late_drops(&tx_rejected_rows(&mock, "ezsdr.radio.mock.rejected"), &mock_events), (0, Vec::new()), "{name}: the Mock is not late");
        assert_eq!(late_drops(&tx_rejected_rows(&uhd, "ezsdr.radio.uhd.rejected"), &uhd_events), (0, Vec::new()), "{name}: UHD is not late");
        assert_eq!(tx_send_calls(&rig.uhd.device, 0).iter().filter(|call| call.contains("sob=true")).count(), 1, "{name}: nothing sent at `s + 1`: {:?}", rig.uhd.device.calls());
    }
}

#[test]
fn s3_parity_booking_after_a_closed_short_burst_follows_d5() {
    // UR-22, RM-15 (S3(a)): a closed standalone burst of 4 to 32 samples at `s` holds `[s, e + 15)`. A booking at its end `e`, or at `e + 14`, is refused
    // at receipt on both Providers; one at `e + 15` is accepted and on time, with none, one or two Stops between.
    for samples in [4usize, 10, 31, 32] {
        for stops in 0..=2usize {
            for offset in [0usize, 14, 15] {
                let case = format!("{samples} samples, {stops} Stop(s), B at {offset} after its end");
                let mut pair = CancellationPair::new(
                    selector(&["0:A:ubx:0"], &["0:A:ubx:0"]),
                    &[("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))],
                    &["rx0"],
                    FakeConfig::default(),
                    BTreeMap::new(),
                );
                let sized = pair.sized(samples).clone();
                let single = pair.single.clone();
                let at = samples as i64 + offset as i64;
                if stops == 0 {
                    submit(&mut pair, 201, 91, |pair| vec![short_booking(pair, &sized, 0, 100, 0), short_booking(pair, &single, 0, 100, at)]);
                } else {
                    submit(&mut pair, 201, 91, |pair| vec![short_booking(pair, &sized, 0, 100, 0)]);
                    for stop in 0..stops {
                        submit(&mut pair, 202 + stop as u64, 92 + stop as i64, |_| vec![stop_both("tx/s0")]);
                    }
                    submit(&mut pair, 204, 92 + stops as i64, |pair| vec![short_booking(pair, &single, 0, 100, at)]);
                }
                pair.uhd.wait_until(pair.uhd_t0 + ms(140));
                pair.mock.step(pair.t0 + ms(140));
                let mock = pair.mock.finish_in_place();
                let mock_events = pair.mock.events();
                let uhd = pair.uhd.finish_in_place();
                let uhd_events = pair.uhd.events();
                let refused = offset < 15;
                let (mock_refusals, uhd_refusals) = (overlap_refusals(&tx_rejected_rows(&mock, "ezsdr.radio.mock.rejected")), overlap_refusals(&tx_rejected_rows(&uhd, "ezsdr.radio.uhd.rejected")));
                assert_eq!(late_drops(&tx_rejected_rows(&mock, "ezsdr.radio.mock.rejected"), &mock_events), (0, Vec::new()), "{case}: the Mock is not late");
                assert_eq!(late_drops(&tx_rejected_rows(&uhd, "ezsdr.radio.uhd.rejected"), &uhd_events), (0, Vec::new()), "{case}: UHD is not late");
                // With a Stop before B the Stop tail decides (S7), so only the agreement is pinned there.
                if stops == 0 {
                    assert_eq!((mock_refusals, uhd_refusals), (usize::from(refused), usize::from(refused)), "{case}: the refusal");
                    let bursts = if refused { 1 } else { 2 };
                    assert_eq!(manifest_rows(&mock, "ezsdr.radio.mock.bursts").len(), bursts, "{case}: the Mock's bursts");
                    assert_eq!(manifest_rows(&uhd, "ezsdr.radio.uhd.bursts").len(), bursts, "{case}: UHD's bursts");
                } else {
                    assert_eq!(mock_refusals, uhd_refusals, "{case}: the Providers refuse alike");
                    assert_eq!(manifest_rows(&mock, "ezsdr.radio.mock.bursts").len(), manifest_rows(&uhd, "ezsdr.radio.uhd.bursts").len(), "{case}: the Providers play alike");
                }
            }
        }
    }
}

#[test]
fn s3_parity_booking_after_a_device_burst_s_short_tail_follows_d5() {
    // UR-22, RM-15 (S3(b), R1): a 10-sample burst booked at a 40-sample burst's end joins it (S4), so the joined piece holds `[e, e + 2)` and its chain front is `e + 2`. A booking at `e + 1` is refused at receipt on both Providers; one at `e + 2` is on time, with none, one or two Stops between.
    for stops in 0..=2usize {
        for offset in [3i64, 4] {
            let case = format!("long first, {stops} Stop(s), B at the short burst's end + {offset}");
            let mut pair = CancellationPair::new(
                selector(&["0:A:ubx:0"], &["0:A:ubx:0"]),
                &[("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))],
                &["rx0"],
                FakeConfig::default(),
                BTreeMap::new(),
            );
            let long = pair.sized(40).clone();
            let short = pair.sized(10).clone();
            let single = pair.single.clone();
            let at = 50 + offset;
            submit(&mut pair, 201, 50, |pair| vec![short_booking(pair, &long, 0, 100, 0), short_booking(pair, &short, 0, 100, 40)]);
            if stops == 0 {
                submit(&mut pair, 202, 91, |pair| vec![short_booking(pair, &single, 0, 100, at)]);
            } else {
                for stop in 0..stops {
                    submit(&mut pair, 202 + stop as u64, 91 + stop as i64, |_| vec![stop_both("tx/s0")]);
                }
                submit(&mut pair, 205, 91 + stops as i64, |pair| vec![short_booking(pair, &single, 0, 100, at)]);
            }
            pair.uhd.wait_until(pair.uhd_t0 + ms(140));
            pair.mock.step(pair.t0 + ms(140));
            let mock = pair.mock.finish_in_place();
            let mock_events = pair.mock.events();
            let uhd = pair.uhd.finish_in_place();
            let uhd_events = pair.uhd.events();
            let refused = offset == 3;
            let (mock_refusals, uhd_refusals) = (overlap_refusals(&tx_rejected_rows(&mock, "ezsdr.radio.mock.rejected")), overlap_refusals(&tx_rejected_rows(&uhd, "ezsdr.radio.uhd.rejected")));
            assert_eq!(late_drops(&tx_rejected_rows(&mock, "ezsdr.radio.mock.rejected"), &mock_events), (0, Vec::new()), "{case}: the Mock is not late");
            assert_eq!(late_drops(&tx_rejected_rows(&uhd, "ezsdr.radio.uhd.rejected"), &uhd_events), (0, Vec::new()), "{case}: UHD is not late");
            // With a Stop before B the Stop tail decides (S7), so only the agreement is pinned there.
            if stops == 0 {
                assert_eq!((mock_refusals, uhd_refusals), (usize::from(refused), usize::from(refused)), "{case}: the refusal");
                let bursts = if refused { 2 } else { 3 };
                assert_eq!(manifest_rows(&mock, "ezsdr.radio.mock.bursts").len(), bursts, "{case}: the Mock's bursts");
                assert_eq!(manifest_rows(&uhd, "ezsdr.radio.uhd.bursts").len(), bursts, "{case}: UHD's bursts");
            } else {
                assert_eq!(mock_refusals, uhd_refusals, "{case}: the Providers refuse alike");
                assert_eq!(manifest_rows(&mock, "ezsdr.radio.mock.bursts").len(), manifest_rows(&uhd, "ezsdr.radio.uhd.bursts").len(), "{case}: the Providers play alike");
            }
        }
    }
}

#[test]
fn s5_parity_b_received_before_a_refuses_a() {
    // UR-21, RM-15 (S5): a 40-sample burst at `s` and a 10-sample burst at `s + 40` overlap. The one received second is refused: received in the order B, A, A is refused; received in the order A, B, B joins A (S4).
    for b_first in [true, false] {
        let case = if b_first { "B received first" } else { "A received first" };
        let mut pair = CancellationPair::new(
            selector(&["0:A:ubx:0"], &["0:A:ubx:0"]),
            &[("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))],
            &["rx0"],
            FakeConfig::default(),
            BTreeMap::new(),
        );
        let long = pair.sized(40).clone();
        let short = pair.sized(10).clone();
        if b_first {
            submit(&mut pair, 201, 91, |pair| vec![short_booking(pair, &short, 0, 100, 40)]);
            submit(&mut pair, 202, 91, |pair| vec![short_booking(pair, &long, 0, 100, 0)]);
        } else {
            submit(&mut pair, 201, 91, |pair| vec![short_booking(pair, &long, 0, 100, 0)]);
            submit(&mut pair, 202, 91, |pair| vec![short_booking(pair, &short, 0, 100, 40)]);
        }
        pair.uhd.wait_until(pair.uhd_t0 + ms(140));
        pair.mock.step(pair.t0 + ms(140));
        let mock = pair.mock.finish_in_place();
        let uhd = pair.uhd.finish_in_place();
        let mock_expected = usize::from(b_first);
        assert_eq!(overlap_refusals(&tx_rejected_rows(&mock, "ezsdr.radio.mock.rejected")), mock_expected, "{case}: the Mock");
        let uhd_expected = usize::from(b_first);
        assert_eq!(overlap_refusals(&tx_rejected_rows(&uhd, "ezsdr.radio.uhd.rejected")), uhd_expected, "{case}: UHD");
        // The refused booking is the 40-sample one when B came first; the 10-sample burst is kept either way.
        let kept: Vec<u64> = if b_first { vec![10] } else { vec![40, 10] };
        let sizes = |instance: &ProviderInstance, namespace: &str| manifest_rows(instance, namespace).iter().map(|row| row["samples"].as_u64().unwrap()).collect::<Vec<_>>();
        assert_eq!(sizes(&mock, "ezsdr.radio.mock.bursts"), kept, "{case}: the Mock's bursts");
        assert_eq!(sizes(&uhd, "ezsdr.radio.uhd.bursts"), kept, "{case}: UHD's bursts");
    }
}

/// One receipt-order case: its name, its bookings as `(samples, offset from s)`, the receipt order (booking indices), and whether each
/// booking is refused, by index.
type Case = (String, Vec<(usize, i64)>, Vec<usize>, Vec<bool>);

/// The receipt-order cases of RM-15 and UR-21 (S1–S5, S4c): each is booked in both receipt orders.
fn receipt_order_cases() -> Vec<Case> {
    // The lengths of a case differ, so the kept lengths name the refused bookings.
    let mut cases: Vec<Case> = Vec::new();
    // Two bookings, `[A, B]`: `ab` and `ba` say whether B refused when A came first, and whether A refused when B came first.
    let mut pair_case = |name: String, bookings: Vec<(usize, i64)>, ab: bool, ba: bool| {
        cases.push((name.clone(), bookings.clone(), vec![0, 1], vec![false, ab]));
        cases.push((name, bookings, vec![1, 0], vec![ba, false]));
    };
    // S1: a one-sample A at `s`, fronting at `s + 19` (its padded end plus the gap, 15); B of two samples at `s + k`. Inside that front (k <= 18) B is
    // refused in either order; `s + 19` is not. B received first, A (earlier) is refused at receipt: a booking before a chain inside the
    // hand-over horizon (RM-15).
    for k in [1i64, 3, 4, 5, 9, 10, 13, 14, 18, 19] {
        pair_case(format!("S1 k={k}"), vec![(1, 0), (2, k)], k <= 18, true);
    }
    // S2: A of 4, 10 or 32 samples at `s`; B of one sample at its end `s + L` or `s + L + 14` is refused; at `s + L + 15` it is accepted.
    for length in [4usize, 10, 32] {
        pair_case(format!("S2 L={length}, B at the end"), vec![(length, 0), (1, length as i64)], true, true);
        pair_case(format!("S2 L={length}, B at the end + 14"), vec![(length, 0), (1, length as i64 + 14)], true, true);
        pair_case(format!("S2 L={length}, B at the end + 15"), vec![(length, 0), (1, length as i64 + 15)], false, true);
    }
    // S3: A of 40 at `s`; B of one sample inside it is refused, whichever comes first.
    for n in [1i64, 20, 39] {
        pair_case(format!("S3 n={n}"), vec![(40, 0), (1, n)], true, true);
    }
    // S4, S5: A of 40 at `s`; B of 10 at `s + 40` joins A when A came first (S4), and is refused when it came first (S5: A's region meets B's).
    // One sample later the gap refuses it in either order, up to `s + 43`; from `s + 44` (the held tail's front, its end plus four) B is accepted.
    pair_case("S4 B joins at s + 40".to_owned(), vec![(40, 0), (10, 40)], false, true);
    pair_case("S4 B at s + 41, the gap".to_owned(), vec![(40, 0), (10, 41)], true, true);
    pair_case("S4 B at s + 43, in the gap".to_owned(), vec![(40, 0), (10, 43)], true, true);
    pair_case("S4 B at s + 44, past the gap".to_owned(), vec![(40, 0), (10, 44)], false, true);
    // S4c: A came first, B joins A (its front is B's end + 4, `s + 54`), so C at `s + 53` is refused and C at `s + 54` is accepted. B came
    // first, A is refused, and B is a closed short head fronting at its end + 15 (`s + 65`): C is refused up to `s + 64`.
    for offset in [53i64, 54, 65] {
        let bookings = vec![(40, 0), (10, 40), (1, offset)];
        cases.push((format!("S4c C at s + {offset}, A first"), bookings.clone(), vec![0, 1, 2], vec![false, false, offset == 53]));
        cases.push((format!("S4c C at s + {offset}, B first"), bookings, vec![1, 0, 2], vec![true, false, offset < 65]));
    }
    cases
}

/// B received first (#79): the earlier-starting booking is refused at receipt when B is inside the hand-over horizon (RM-15).
fn b_first(case: &Case) -> bool {
    case.2[0] == 1
}

/// Runs each case on both Providers in its receipt order and returns one line per case where they disagree with each other or with the
/// expected refusals and plays.
fn receipt_order_mismatches<'a>(cases: impl IntoIterator<Item = &'a Case>) -> Vec<String> {
    let mut mismatches = Vec::new();
    for (case, bookings, order, refused) in cases {
        let mut pair = CancellationPair::new(
            selector(&["0:A:ubx:0"], &["0:A:ubx:0", "0:B:ubx:0"]),
            &[("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c1.stream", eq(Scalar::from(0)))],
            &["rx0"],
            FakeConfig::default(),
            BTreeMap::new(),
        );
        for (position, index) in order.iter().enumerate() {
            let (samples, offset) = bookings[*index];
            let waveform = pair.stereo(samples).clone();
            submit(&mut pair, 300 + position as u64, 91, |pair| vec![short_booking(pair, &waveform, 0, 100, offset)]);
        }
        pair.uhd.wait_until(pair.uhd_t0 + ms(140));
        pair.mock.step(pair.t0 + ms(140));
        let mock = pair.mock.finish_in_place();
        let mock_events = pair.mock.events();
        let uhd = pair.uhd.finish_in_place();
        let uhd_events = pair.uhd.events();
        let mut kept: Vec<u64> = bookings.iter().zip(refused.iter()).filter(|(_, refused)| !**refused).map(|((samples, _), _)| *samples as u64).collect();
        kept.sort_unstable();
        let refusals = refused.iter().filter(|refused| **refused).count();
        let sizes = |instance: &ProviderInstance, namespace: &str| {
            let mut sizes: Vec<u64> = manifest_rows(instance, namespace).iter().map(|row| row["samples"].as_u64().unwrap()).collect();
            sizes.sort_unstable();
            sizes
        };
        let (mock_rejected, uhd_rejected) = (tx_rejected_rows(&mock, "ezsdr.radio.mock.rejected"), tx_rejected_rows(&uhd, "ezsdr.radio.uhd.rejected"));
        let mock_plays = sizes(&mock, "ezsdr.radio.mock.bursts");
        let uhd_plays = sizes(&uhd, "ezsdr.radio.uhd.bursts");
        let late = |rejected: &[serde_json::Value], events: &[ezsdr_kernel::event::Event]| {
            late_drops(rejected, events) == (0, Vec::new()) && events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR)
        };
        let ok = overlap_refusals(&mock_rejected) + handed_refusals(&mock_rejected) == refusals
            && overlap_refusals(&uhd_rejected) + handed_refusals(&uhd_rejected) == refusals
            && overlap_refusals(&mock_rejected) == overlap_refusals(&uhd_rejected)
            && handed_refusals(&mock_rejected) == handed_refusals(&uhd_rejected)
            && mock_plays == kept
            && uhd_plays == kept
            && late(&mock_rejected, &mock_events)
            && late(&uhd_rejected, &uhd_events);
        if !ok {
            mismatches.push(format!(
                "{case} (receipt order {order:?}): refused Mock {} overlap + {} handed, UHD {} overlap + {} handed; played Mock {mock_plays:?}, UHD {uhd_plays:?}; expected refused {refusals}, played {kept:?}",
                overlap_refusals(&mock_rejected),
                handed_refusals(&mock_rejected),
                overlap_refusals(&uhd_rejected),
                handed_refusals(&uhd_rejected)
            ));
        }
    }
    mismatches
}

#[test]
fn s15_parity_booking_rule_both_receipt_orders() {
    // RM-15, UR-21 (S1–S5, S4c) on one two-channel transmit stream, the earlier-starting booking received first: each Provider refuses the
    // same bookings and plays the rest, on time. A booking is `(samples, offset from s)`. The later-starting-first orders are the next test.
    let cases = receipt_order_cases();
    let mismatches = receipt_order_mismatches(cases.iter().filter(|case| !b_first(case)));
    assert!(mismatches.is_empty(), "the Providers disagree:\n{}", mismatches.join("\n"));
}

#[test]
fn s15_parity_b_first_accepted_booking_rule() {
    // #79 (RM-15): the later-starting booking received first; the earlier-starting one is refused at receipt on both Providers, whenever the
    // later one is inside the hand-over horizon.
    let cases = receipt_order_cases();
    let mismatches = receipt_order_mismatches(cases.iter().filter(|case| b_first(case)));
    assert!(mismatches.is_empty(), "the Providers disagree:\n{}", mismatches.join("\n"));
}

/// The first root tick at which transmit stream `stream`'s sample clock is `samples` samples past the sample at `root_tick` (S13's value at `s + 5`).
fn tx_root_after(clocks: &ClockRegistry, stream: usize, root_tick: i64, samples: i64) -> i64 {
    let target = tx_samples_after(clocks, stream, root_tick, samples);
    let domain = target.domain();
    let want = target.ticks_in(domain).unwrap();
    (root_tick..).find(|&root| tx_sample_time(clocks, stream, 0, root).ticks_in(domain).unwrap() >= want).unwrap()
}

#[test]
fn s13_parity_b_refused_at_receipt_and_the_value_answers() {
    // S13 (#79's value case): a one-sample A at `s` (padded front `s + 5`); B of 100 samples at `s + 1` under DropAndFlag, booked before its start (the UHD rig's clock runs ahead of a submitted tick, so `s + 1` itself cannot be pinned); a value at `s + 5` samples is taken on both, and B is the only refusal.
    let selector = selector(&["0:A:ubx:0"], &["0:A:ubx:0"]);
    let constraints = [("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.frequency_hz", eq(Scalar::try_from(2.0e9).unwrap()))];
    let mut pair = CancellationPair::new(selector, &constraints, &["rx0"], FakeConfig::default(), BTreeMap::new());
    let single = pair.single.clone();
    let hundred = pair.sized(100).clone();
    submit(&mut pair, 201, 91, |pair| vec![short_booking(pair, &single, 0, 100, 0), short_booking(pair, &hundred, 0, 100, 1)]);
    let (mock_at, uhd_at) = (tx_root_after(&pair.mock.clocks, 0, pair.t0 + ms(100), 5), tx_root_after(&pair.uhd.clocks, 0, pair.uhd_t0 + ms(100), 5));
    s14_submit(&mut pair, 202, mock_at, uhd_at, s14_value(2.5e9));
    pair.uhd.wait_until(pair.uhd_t0 + ms(140));
    pair.mock.step(pair.t0 + ms(140));
    let (mock, uhd) = (tx_decisions(&pair.mock.events()), tx_decisions(&pair.uhd.events()));
    assert_eq!(mock, uhd, "the Providers answer alike");
    assert_eq!(mock.iter().map(|(kind, _)| kind.as_str()).collect::<Vec<_>>(), ["tx_burst"], "only B is refused; the value at `s + 5` is taken: {mock:?}");
}

#[test]
fn s15_parity_s4b_three_bookings_b_continues_a_and_c_plays_on_time() {
    // S4b (R4-5): A(40) at 20 ms is handed, C(10) is booked early at 100 samples past it, then B(10) joins A at 40 samples, received
    // before the join deadline (15.04 ms). B continues A with no end-of-burst between, and C plays on time on both Providers.
    // The scenario needs the bookings received at 6, 11 and 12 ms: the rig's thread is a participant of the device clock, so the clock stands
    // at each of those instants while the thread pushes (note 51 stage 1b), and a late or refused booking is a failure. The property itself, with exact receipt instants, is
    // also pinned on the frozen rigs: `o1_uhd_t2c_a_booking_joined_at_a_heads_end_…` (UHD) and `o1_mock_t2c_…` (Mock), and the join deadline's
    // equality by `o1_uhd_t2_a_join_is_inclusive_at_its_deadline` and `o1_mock_t2_a_join_is_inclusive_at_its_deadline`.
    let mut pair = CancellationPair::new(
        selector(&["0:A:ubx:0"], &["0:A:ubx:0"]),
        &[("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))],
        &["rx0"],
        FakeConfig::default(),
        BTreeMap::new(),
    );
    let long = pair.sized(40).clone();
    let short = pair.sized(10).clone();
    submit(&mut pair, 201, 6, |pair| vec![short_booking(pair, &long, 0, 20, 0)]);
    submit(&mut pair, 202, 11, |pair| vec![short_booking(pair, &short, 0, 20, 100)]);
    submit(&mut pair, 203, 12, |pair| vec![short_booking(pair, &short, 0, 20, 40)]);
    submit(&mut pair, 204, 16, |_| Vec::new());
    pair.uhd.wait_until(pair.uhd_t0 + ms(30));
    pair.mock.step(pair.t0 + ms(30));
    let mock = pair.mock.finish_in_place();
    let mock_events = pair.mock.events();
    let uhd = pair.uhd.finish_in_place();
    let uhd_events = pair.uhd.events();
    let uhd_sends = tx_send_calls(&pair.uhd.device, 0);
    let late = |events: &[Event]| {
        events.iter().any(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR || (event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED && event.payload["kind"] == "tx_burst"))
    };
    assert!(!late(&uhd_events), "the UHD (its thread a participant of the device clock) receives and plays everything on time: {uhd_events:?}");
    assert!(!late(&mock_events), "the Mock (stepped by the test) is on time: {mock_events:?}");
    assert_eq!(overlap_refusals(&tx_rejected_rows(&mock, "ezsdr.radio.mock.rejected")), 0, "the Mock refuses nothing");
    assert_eq!(overlap_refusals(&tx_rejected_rows(&uhd, "ezsdr.radio.uhd.rejected")), 0, "UHD refuses nothing");
    let sizes = |instance: &ProviderInstance, namespace: &str| {
        let mut sizes: Vec<u64> = manifest_rows(instance, namespace).iter().map(|row| row["samples"].as_u64().unwrap()).collect();
        sizes.sort_unstable();
        sizes
    };
    assert_eq!(sizes(&mock, "ezsdr.radio.mock.bursts"), [10, 10, 40], "the Mock's bursts");
    assert_eq!(sizes(&uhd, "ezsdr.radio.uhd.bursts"), [10, 10, 40], "UHD's bursts");
    assert_eq!(uhd_sends.iter().filter(|call| call.contains("sob=true")).count(), 2, "A and B are one device burst, C another: {uhd_sends:?}");
}

/// Submits a switch to each of `waves`, in this order and at one receipt, once the UHD's repeat is observed started (`o1_wait_playing`: a
/// switch received before the owner issued the repeat's first send is refused, UR-38, KL-41). The Mock receives them at 24 ms with `at` at
/// 26 ms (it is stepped by the test); the UHD from the observed start, with `at` there too.
fn s15_switch(pair: &mut CancellationPair, token: u64, waves: &[&ezsdr_kernel::manifest::ArtifactRef]) {
    let rel = o1_wait_playing(pair, 7);
    for wave in waves {
        let mock = o1_asap(pair, wave, 26, true, true).0;
        o1_mock_at(pair, 24, mock);
    }
    let uhd: Vec<Action> = waves.iter().map(|wave| o1_asap(pair, wave, rel, true, true).1).collect();
    o1_uhd_at(pair, token, rel, uhd);
}

/// A Provider's `switches` rows as (predecessor, old period, new period, outcome). The action id is left out: UHD's barrier takes
/// action ids the Mock never sees, so the two Providers number the same switch differently.
fn switch_rows(instance: &ProviderInstance, namespace: &str) -> Vec<(u64, i64, i64, String)> {
    manifest_rows(instance, namespace)
        .iter()
        .map(|row| (row["predecessor"].as_u64().unwrap(), row["old_period"].as_i64().unwrap(), row["new_period"].as_i64().unwrap(), row["outcome"].as_str().unwrap().to_owned()))
        .collect()
}

#[test]
fn s15_parity_switch_to_a_new_length() {
    // S8, S9 (RM-37): a 1 000-sample repeat from 20 ms, switched at 26 ms to a 500-sample waveform: both Providers record one applied
    // switch from the repeat's period to the new one.
    let mut pair = CancellationPair::new(
        selector(&["0:A:ubx:0"], &["0:A:ubx:0"]),
        &[("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))],
        &["rx0"],
        FakeConfig::default(),
        BTreeMap::new(),
    );
    let long = pair.sized(1_000).clone();
    let short = pair.sized(500).clone();
    submit(&mut pair, 201, 6, |pair| vec![o1_asap(pair, &long, 20, true, false)]);
    s15_switch(&mut pair, 202, &[&short]);
    pair.uhd.wait_until(o1_uhd_now(&pair) + ms(60));
    pair.mock.step(pair.t0 + ms(60));
    let mock = pair.mock.finish_in_place();
    let uhd = pair.uhd.finish_in_place();
    let (mock_rows, uhd_rows) = (switch_rows(&mock, "ezsdr.radio.mock.switches"), switch_rows(&uhd, "ezsdr.radio.uhd.switches"));
    assert_eq!(mock_rows, uhd_rows, "the Providers record alike");
    assert_eq!(mock_rows, [(1, 1_000, 500, "applied".to_owned())], "{mock_rows:?}");
}

/// A pair on one transmit stream (as the S15 tests), for the refusals of a time that does not fit the clock.
fn s16_pair() -> CancellationPair {
    CancellationPair::new(
        selector(&["0:A:ubx:0"], &["0:A:ubx:0"]),
        &[("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))],
        &["rx0"],
        FakeConfig::default(),
        BTreeMap::new(),
    )
}

/// The Action ids the next pushes get on each Provider (mock, UHD): a queue numbers its Actions in the order its Provider takes them, so
/// the Providers number the same Action differently once one has had a barrier. Read while both queues are empty.
fn next_ids(pair: &CancellationPair) -> (u64, u64) {
    let next = |queue: &Queue| queue.given.load(Ordering::SeqCst) + queue.queued() as u64 + 1;
    (next(&pair.mock.actions), next(&pair.uhd.actions))
}

/// What each `tx_burst` refusal says, without its Provider's rule prefix, once its Action id and rule are as `expected` (id, rule): the
/// Providers answer alike when these are equal.
fn tx_burst_refusals(events: &[Event], expected: &[(u64, &str)]) -> Vec<String> {
    let refused: Vec<(u64, String, String)> = events
        .iter()
        .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED && event.payload["kind"] == "tx_burst")
        .map(|event| {
            let (rule, reason) = event.payload["reason"].as_str().unwrap().split_once(": ").unwrap();
            (event.payload["action"].as_u64().expect("a refusal at receipt names its Action"), rule.to_owned(), reason.to_owned())
        })
        .collect();
    assert_eq!(refused.iter().map(|(id, rule, _)| (*id, rule.as_str())).collect::<Vec<_>>(), expected, "the Action ids and rules of the refusals: {refused:?}");
    refused.into_iter().map(|(_, _, reason)| reason).collect()
}

#[test]
fn s16_parity_a_burst_time_that_does_not_fit_the_clock_is_refused_on_both_and_the_next_booking_is_served() {
    // RM-22, MR-16, UR-21 (#95): a burst at 2^62 samples (its instant passes i64::MAX) and a target whose lead overflows (i64::MIN) are
    // refused at receipt with their ids on both Providers, which go on; a burst received after them is booked and played on both.
    let mut pair = s16_pair();
    let hundred = pair.sized(100).clone();
    let at = |ticks: i64| {
        let at = |clocks: &ClockRegistry| TimePoint::new(tx_sample_time(clocks, 0, 0, 0).domain(), ticks);
        (
            tx_burst_action_with("tx/s0", &hundred, at(&pair.mock.clocks), ezsdr_kernel::stream::LatePolicy::DropAndFlag),
            tx_burst_action_with("tx/s0", &hundred, at(&pair.uhd.clocks), ezsdr_kernel::stream::LatePolicy::DropAndFlag),
        )
    };
    let (far, early) = (at(1 << 62), at(i64::MIN));
    let (mock_id, uhd_id) = next_ids(&pair);
    submit(&mut pair, 201, 6, |_| vec![far, early]);
    // SendAsapAndFlag: a host that is late moves the start, it never drops the burst.
    submit(&mut pair, 202, 12, |pair| vec![o1_asap(pair, &hundred, 60, false, false)]);
    pair.uhd.wait_until(o1_uhd_now(&pair) + ms(100));
    pair.mock.step(pair.t0 + ms(100));
    let (mock_events, uhd_events) = (pair.mock.events(), pair.uhd.events());
    // The lead of a target at i64::MIN overflows in the late policy: the Mock names MR-17 for it, the UHD UR-21, with one error text.
    let mock_refused = tx_burst_refusals(&mock_events, &[(mock_id, "MR-16"), (mock_id + 1, "MR-17")]);
    let uhd_refused = tx_burst_refusals(&uhd_events, &[(uhd_id, "UR-21"), (uhd_id + 1, "UR-21")]);
    assert_eq!(mock_refused, uhd_refused, "the Providers answer alike");
    assert_eq!(mock_refused[0], "the burst time overflows the transmit clock");
    let mock = pair.mock.finish_in_place();
    let uhd = pair.uhd.finish_in_place();
    let samples = |instance: &ProviderInstance, namespace: &str| manifest_rows(instance, namespace).iter().map(|row| row["samples"].as_u64().unwrap()).collect::<Vec<_>>();
    assert_eq!(samples(&mock, "ezsdr.radio.mock.bursts"), [100], "the Mock served the burst after the refusals");
    assert_eq!(samples(&uhd, "ezsdr.radio.uhd.bursts"), [100], "UHD served the burst after the refusals");
}

#[test]
fn s16_parity_a_switch_at_the_end_of_the_range_is_refused_on_both_and_the_next_one_is_taken() {
    // RM-37, MR-16, UR-21 (#94): a switch at i64::MAX has no boundary: refused at receipt with its id on both Providers, which make no
    // `switches` row for it and keep the repeat; a switch received after it is taken and applied on both.
    let mut pair = s16_pair();
    let long = pair.sized(1_000).clone();
    let short = pair.sized(500).clone();
    submit(&mut pair, 201, 6, |pair| vec![o1_asap(pair, &long, 20, true, false)]);
    let rel = o1_wait_playing(&mut pair, 7);
    let (mut mock, mut uhd) = (o1_asap(&pair, &short, 26, true, true).0, o1_asap(&pair, &short, rel, true, true).1);
    for action in [&mut mock, &mut uhd] {
        if let Action::TxBurst { at, .. } = action {
            *at = ezsdr_kernel::time::AbsoluteDeadline::new(TimePoint::new(at.time_point.domain(), i64::MAX));
        }
    }
    let (mock_id, uhd_id) = next_ids(&pair);
    o1_mock_at(&mut pair, 24, mock);
    o1_uhd_at(&mut pair, 202, rel, [uhd]);
    s15_switch(&mut pair, 203, &[&short]);
    pair.uhd.wait_until(o1_uhd_now(&pair) + ms(60));
    pair.mock.step(pair.t0 + ms(60));
    let (mock_events, uhd_events) = (pair.mock.events(), pair.uhd.events());
    let (mock_refused, uhd_refused) = (tx_burst_refusals(&mock_events, &[(mock_id, "MR-16")]), tx_burst_refusals(&uhd_events, &[(uhd_id, "UR-21")]));
    assert_eq!(mock_refused, uhd_refused, "the Providers answer alike");
    assert_eq!(mock_refused, ["the burst time overflows the transmit clock"], "only the switch at i64::MAX is refused");
    let mock = pair.mock.finish_in_place();
    let uhd = pair.uhd.finish_in_place();
    let (mock_rows, uhd_rows) = (switch_rows(&mock, "ezsdr.radio.mock.switches"), switch_rows(&uhd, "ezsdr.radio.uhd.switches"));
    assert_eq!(mock_rows, uhd_rows, "the Providers record alike");
    assert_eq!(mock_rows, [(1, 1_000, 500, "applied".to_owned())], "the second switch is the only row: {mock_rows:?}");
}

#[test]
fn s15_parity_second_pending_switch_is_refused() {
    // S10 (RM-37): a second switch while the first is pending is refused with its id on both Providers, and the first still applies.
    let mut pair = CancellationPair::new(
        selector(&["0:A:ubx:0"], &["0:A:ubx:0"]),
        &[("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))],
        &["rx0"],
        FakeConfig::default(),
        BTreeMap::new(),
    );
    let long = pair.sized(1_000).clone();
    let short = pair.sized(500).clone();
    submit(&mut pair, 201, 6, |pair| vec![o1_asap(pair, &long, 20, true, false)]);
    s15_switch(&mut pair, 202, &[&short, &long]);
    pair.uhd.wait_until(o1_uhd_now(&pair) + ms(60));
    pair.mock.step(pair.t0 + ms(60));
    let (mock, uhd) = (tx_decisions(&pair.mock.events()), tx_decisions(&pair.uhd.events()));
    let kinds = |decisions: &[(String, u64)]| decisions.iter().map(|(kind, _)| kind.clone()).collect::<Vec<_>>();
    assert_eq!(kinds(&mock), ["tx_burst"], "only the second switch is refused: {mock:?}");
    assert_eq!(kinds(&uhd), ["tx_burst"], "only the second switch is refused: {uhd:?}");
    let mock = pair.mock.finish_in_place();
    let uhd = pair.uhd.finish_in_place();
    let (mock_rows, uhd_rows) = (switch_rows(&mock, "ezsdr.radio.mock.switches"), switch_rows(&uhd, "ezsdr.radio.uhd.switches"));
    assert_eq!(mock_rows, uhd_rows, "the Providers record alike");
    assert_eq!(mock_rows, [(1, 1_000, 500, "applied".to_owned())], "the first switch still applies: {mock_rows:?}");
}

#[test]
fn s15_parity_switch_mark_under_another_late_policy_is_refused_at_receipt() {
    // RM-13, X1: the switch mark on a repeat whose late policy is not SendAsapAndFlag is refused at receipt on both Providers, with its
    // id, as a rejected action and no `TIME_ERROR`; no `switches` row is made and the repeat playing plays on.
    let mut pair = CancellationPair::new(
        selector(&["0:A:ubx:0"], &["0:A:ubx:0"]),
        &[("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))],
        &["rx0"],
        FakeConfig::default(),
        BTreeMap::new(),
    );
    let long = pair.sized(1_000).clone();
    let short = pair.sized(500).clone();
    let dropping = |(mock, uhd): (Action, Action)| {
        let policy = |mut action: Action| {
            if let Action::TxBurst { late_policy, .. } = &mut action {
                *late_policy = ezsdr_kernel::stream::LatePolicy::DropAndFlag;
            }
            action
        };
        (policy(mock), policy(uhd))
    };
    submit(&mut pair, 201, 6, |pair| vec![o1_asap(pair, &long, 20, true, false)]);
    let rel = o1_wait_playing(&mut pair, 7);
    let (mock_switch, _) = dropping(o1_asap(&pair, &short, 26, true, true));
    let (_, uhd_switch) = dropping(o1_asap(&pair, &short, rel, true, true));
    o1_mock_at(&mut pair, 24, mock_switch);
    o1_uhd_at(&mut pair, 202, rel, [uhd_switch]);
    pair.uhd.wait_until(o1_uhd_now(&pair) + ms(60));
    pair.mock.step(pair.t0 + ms(60));
    let (mock_events, uhd_events) = (pair.mock.events(), pair.uhd.events());
    let (mock, uhd) = (tx_decisions(&mock_events), tx_decisions(&uhd_events));
    assert_eq!(mock.iter().map(|(kind, _)| kind.as_str()).collect::<Vec<_>>(), ["tx_burst"], "the marked repeat is refused: {mock:?}");
    assert_eq!(uhd.iter().map(|(kind, _)| kind.as_str()).collect::<Vec<_>>(), ["tx_burst"], "the marked repeat is refused: {uhd:?}");
    // The refused switch reports no TIME_ERROR. The UHD's repeat may be late and report its own (send_asap, naming the repeat's target, which
    // is not the switch's: a non-switch burst's TIME_ERROR names no action); the stepped Mock has none.
    assert!(mock_events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "a refusal at receipt reports no TIME_ERROR: {mock_events:?}");
    let repeat_at = tx_samples_after(&pair.uhd.clocks, 0, pair.uhd_t0 + ms(20), 0);
    let repeat_ticks = repeat_at.ticks_in(repeat_at.domain()).unwrap();
    assert!(
        uhd_events
            .iter()
            .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR)
            .all(|event| event.payload["outcome"] == "send_asap" && event.payload["target"]["ticks"].as_i64() == Some(repeat_ticks)),
        "a refusal at receipt reports no TIME_ERROR for the switch: {uhd_events:?}"
    );
    let mock = pair.mock.finish_in_place();
    let uhd = pair.uhd.finish_in_place();
    assert_eq!(switch_rows(&mock, "ezsdr.radio.mock.switches"), []);
    assert_eq!(switch_rows(&uhd, "ezsdr.radio.uhd.switches"), []);
}

#[test]
fn s15_parity_switch_starts_on_the_old_repeats_end() {
    // S8 (RM-37, D57): a 1 000-sample repeat switched at 26 ms to a 500-sample waveform of other samples. Both Providers start the new
    // burst where the old one ends. On UHD the device sends the old samples up to that boundary and the new ones from it.
    let mut pair = CancellationPair::new(
        selector(&["0:A:ubx:0"], &["0:A:ubx:0"]),
        &[("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))],
        &["rx0"],
        FakeConfig::default(),
        BTreeMap::new(),
    );
    let long = pair.sized(1_000).clone();
    let tone = pair.tone.clone();
    submit(&mut pair, 201, 6, |pair| vec![o1_asap(pair, &long, 20, true, false)]);
    s15_switch(&mut pair, 202, &[&tone]);
    pair.uhd.wait_until(o1_uhd_now(&pair) + ms(60));
    pair.mock.step(pair.t0 + ms(60));
    let mock = pair.mock.finish_in_place();
    let uhd = pair.uhd.finish_in_place();
    // The old repeat's burst, then the switched one: (start, samples) on each Provider's own sample grid.
    let bursts = |instance: &ProviderInstance, namespace: &str| {
        let rows = manifest_rows(instance, namespace);
        assert_eq!(rows.len(), 2, "{namespace}: the old repeat and the switched burst: {rows:?}");
        rows.iter().map(|row| (row["target"]["ticks"].as_i64().unwrap(), row["samples"].as_i64().unwrap())).collect::<Vec<_>>()
    };
    let (mock_bursts, uhd_bursts) = (bursts(&mock, "ezsdr.radio.mock.bursts"), bursts(&uhd, "ezsdr.radio.uhd.bursts"));
    // Each Provider's boundary is its own (RM-37), so the two may end the old repeat at different samples.
    for (case, bursts) in [("Mock", &mock_bursts), ("UHD", &uhd_bursts)] {
        let ((old_start, old_samples), (new_start, _)) = (bursts[0], bursts[1]);
        assert_eq!(new_start, old_start + old_samples, "{case}: the switched burst starts where the old repeat ends: {bursts:?}");
    }
    // UHD's device samples at the boundary: the old repeat's last sample, then the first of the new waveform.
    let boundary = uhd_bursts[1].0;
    assert_eq!(pair.uhd.device.transmitted(tx_device_tick(&pair.uhd.clocks, 0, boundary - 1), 2), vec![Some([0.25, 0.0]), Some([0.75, -0.25])], "{:?}", pair.uhd.device.calls());
}

#[test]
fn s15_parity_provider_stop_cancels_a_pending_switch() {
    // S11, D58 (RM-37, UR-26, UR-38): a switch received at 22 ms is still pending when Provider::stop ends the Run at 23 ms, before its
    // boundary. Both Providers cancel it, and the new waveform is never sent: the old repeat is the only burst. The Mock is stepped, so
    // its half always runs. The UHD receives the switch from the repeat's observed start with `at` a second later, so its boundary is a
    // second past the Stop. A Stop whose bound (RM-16: the first
    // sample at or after the Stop's `at` plus the 10 ms in-flight window, plus five, which is what `Provider::stop` books and the oracle's
    // `stop_bound` computes) lies before the boundary cancels the switch; one whose bound lies after it applies the switch (correctly).
    // The rig's thread is a participant of the device clock, so the clock stands while it works (note 51 stage 1b) and the boundary is more than 1 ms past that
    // bound, the Stop's `at` read from the `stop` row of the Manifest's timing section: the test asserts it. The frozen rigs also own the case:
    // `ur_23_provider_stop_and_loss_cancel_the_pending_switch` (UHD, tx.rs) and `mr_25_s11_loss_and_provider_stop_cancel_pending_switch`
    // (Mock, mock_radio.rs).
    let mut pair = CancellationPair::new(
        selector(&["0:A:ubx:0"], &["0:A:ubx:0"]),
        &[("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))],
        &["rx0"],
        FakeConfig::default(),
        BTreeMap::new(),
    );
    let long = pair.sized(1_000).clone();
    let tone = pair.tone.clone();
    submit(&mut pair, 201, 6, |pair| vec![o1_asap(pair, &long, 20, true, false)]);
    let rel = o1_wait_playing(&mut pair, 7);
    let (mock_switch, _) = o1_asap(&pair, &tone, 26, true, true);
    let (_, uhd_switch) = o1_asap(&pair, &tone, rel + 1_000, true, true);
    o1_mock_at(&mut pair, 22, mock_switch);
    o1_uhd_at(&mut pair, 202, rel, [uhd_switch]);
    pair.mock.step(pair.t0 + ms(23));
    let mock = pair.mock.finish_in_place();
    let uhd = pair.uhd.finish_in_place();
    let check = |case: &str, instance: &ProviderInstance, namespace: &str| {
        let outcomes: Vec<String> = switch_rows(instance, &format!("{namespace}.switches")).into_iter().map(|row| row.3).collect();
        assert_eq!(outcomes, ["cancelled"], "{case}: the switch is cancelled");
        assert_eq!(manifest_rows(instance, &format!("{namespace}.bursts")).len(), 1, "{case}: the old repeat is the only burst");
    };
    check("Mock", &mock, "ezsdr.radio.mock");
    let boundary = manifest_rows(&uhd, "ezsdr.radio.uhd.switches").first().map(|row| tx_device_tick(&pair.uhd.clocks, 0, row["boundary"].as_i64().unwrap()));
    if let Some(boundary) = boundary {
        let stop_at = manifest_rows(&uhd, "ezsdr.radio.uhd.timing").into_iter().find(|row| row["what"] == "stop").expect("the stop row")["at"].as_i64().unwrap();
        let params = o1_params(&pair.uhd.clocks);
        let bound = params.clock.instant(params.stop_bound(stop_at).1);
        assert!(boundary > bound + ms(1), "the Stop's bound {bound} is more than 1 ms before the switch's boundary {boundary}");
    }
    check("UHD", &uhd, "ezsdr.radio.uhd");
    let origin = manifest_rows(&uhd, "ezsdr.radio.uhd.bursts")[0]["target"]["ticks"].as_i64().unwrap();
    assert!(!pair.uhd.device.transmitted(tx_device_tick(&pair.uhd.clocks, 0, origin), 30_000).contains(&Some([0.75, -0.25])), "{:?}", pair.uhd.device.calls());
}

// ---------------------------------------------------------------- transmit values on an empty stream (S14)

#[test]
fn s15_parity_untimed_booking_after_stop() {
    // RM-15, RM-16 (sequence 1): an untimed repeat after a Stop starts past the Stop region, on both Providers, with no TIME_ERROR. The
    // second repeat is booked inside the region, so the placement, not the lead, moves it. This test's threads set when each repeat is
    // received, so it compares only what that timing cannot move: the Stop region's length on UHD (from the record the Provider keeps)
    // and that no repeat starts before its region's end. The exact start is pinned by manual-clock tests: for a repeat received inside
    // the region, by `ur_21_an_untimed_repeat_after_a_stop_starts_at_the_bound_or_its_ordinary_start` (UHD, control.rs) and
    // `mr_16_untimed_repeat_after_stop_starts_at_the_stop_region_end` (Mock, mock_radio.rs); for one received after it, by the UHD test
    // above and the pure `place_untimed` and `prune` tests in crates/ezsdr-radio/tests/staging.rs, which the Mock's placement calls.
    let mut pair = CancellationPair::new(
        selector(&["0:A:ubx:0"], &["0:A:ubx:0"]),
        &[("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))],
        &["rx0"],
        FakeConfig::default(),
        BTreeMap::new(),
    );
    let long = pair.sized(1_000).clone();
    submit_untimed(&mut pair, 201, 6, |pair| vec![o1_asap(pair, &long, 6, true, false)]);
    submit(&mut pair, 202, 20, |_| vec![stop_both("tx/s0")]);
    submit_untimed(&mut pair, 203, 21, |pair| vec![o1_asap(pair, &long, 21, true, false)]);
    pair.uhd.wait_until(pair.uhd_t0 + ms(60));
    pair.mock.step(pair.t0 + ms(60));
    let (mock_events, uhd_events) = (pair.mock.events(), pair.uhd.events());
    assert!(mock_events.iter().chain(&uhd_events).all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "an untimed repeat has no TIME_ERROR");
    let stop = |clocks: &ClockRegistry, root: i64| {
        let at = tx_sample_time(clocks, 0, 0, root);
        at.ticks_in(at.domain()).unwrap()
    };
    let mock_stop = stop(&pair.mock.clocks, pair.t0 + ms(20));
    let mock = pair.mock.finish_in_place();
    let uhd = pair.uhd.finish_in_place();
    // The UHD's Stop as its Provider applied it (`tx_stop_applied`, #83): the sample it was applied at and the sample its bookings end at.
    let applied: Vec<_> = manifest_rows(&uhd, "ezsdr.radio.uhd.timing").into_iter().filter(|row| row["what"] == "tx_stop_applied").collect();
    assert_eq!(applied.len(), 1, "one transmit Stop is applied: {applied:?}");
    let uhd_at = stop(&pair.uhd.clocks, applied[0]["at"].as_i64().unwrap());
    let uhd_end = stop(&pair.uhd.clocks, applied[0]["bound"].as_i64().unwrap());
    // The repeat after the Stop, in samples after the Stop's sample on each Provider.
    let after = |instance: &ProviderInstance, namespace: &str, stop: i64| {
        let rows = manifest_rows(instance, namespace);
        assert_eq!(rows.len(), 2, "{namespace}: the repeat and the one after the Stop: {rows:?}");
        assert!(rows[1]["late_by"].is_null(), "{namespace}: an untimed repeat reports no lateness: {rows:?}");
        rows[1]["target"]["ticks"].as_i64().unwrap() - stop
    };
    let (mock_gap, uhd_gap) = (after(&mock, "ezsdr.radio.mock.bursts", mock_stop), after(&uhd, "ezsdr.radio.uhd.bursts", uhd_end));
    // The Stop region is 10 ms (KL-13, KL-15) and five samples (RM-16) past the Stop's sample, to one sample of rounding onto the grid.
    // The UHD's region comes from its record, so a record that is not 10 ms fails here. The Mock's region is exact on its own
    // deterministic step: its bound is 10 005 samples past the Stop's sample.
    assert!((10_005..=10_006).contains(&(uhd_end - uhd_at)), "UHD: the Stop region is {} samples past the Stop, not 10 ms", uhd_end - uhd_at);
    assert!(uhd_gap >= 0, "UHD: the repeat starts {uhd_gap} samples before the Stop region's end");
    assert!((10_005..=10_006).contains(&mock_gap), "Mock: the repeat starts {mock_gap} samples after the Stop, not at the region's end (10 005 samples)");
}

#[test]
fn s15_parity_untimed_queue_behind_a_booking() {
    // RM-15 (sequence 3): a timed burst B of 10 samples at s = 30 ms, then an untimed burst of 10 samples at s + 3, booked in that order.
    // The untimed one queues behind B's front (s + 25) on both Providers, with no refusal and no TIME_ERROR. B is SendAsapAndFlag (a late
    // host moves it, never drops it), so the comparison is made from B's observed start: the Mock's untimed burst starts exactly at B's front,
    // s + max(10, 4) + 15 (it is stepped by the test); the UHD's no earlier than that front (a late receipt can only move it later, RM-15:
    // the first free sample at or after the ordinary bound). The UHD's exact start, with receipt instants fixed, is pinned by
    // `o1_uhd_t5_a_late_send_asap_and_an_untimed_booking_are_placed_exactly` and `o1_mock_t5_…` (untimed rows (d) and (e)).
    let mut pair = CancellationPair::new(
        selector(&["0:A:ubx:0"], &["0:A:ubx:0"]),
        &[("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))],
        &["rx0"],
        FakeConfig::default(),
        BTreeMap::new(),
    );
    let ten = pair.sized(10).clone();
    submit(&mut pair, 201, 17, |pair| vec![o1_asap(pair, &ten, 30, false, false)]);
    submit_untimed(&mut pair, 202, 17, |pair| vec![short_booking(pair, &ten, 0, 30, 3)]);
    pair.uhd.wait_until(o1_uhd_now(&pair) + ms(60));
    pair.mock.step(pair.t0 + ms(60));
    let (mock_events, uhd_events) = (pair.mock.events(), pair.uhd.events());
    assert!(tx_decisions(&mock_events).is_empty() && tx_decisions(&uhd_events).is_empty(), "both bursts are accepted: {mock_events:?}");
    // A late B reports its own TIME_ERROR (send_asap), which names B by its target (a non-switch burst's TIME_ERROR names no action, RM-22);
    // the untimed burst never has one.
    assert!(mock_events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "the Mock is stepped by the test: no TIME_ERROR");
    let b_at = tx_samples_after(&pair.uhd.clocks, 0, pair.uhd_t0 + ms(30), 0);
    let b_ticks = b_at.ticks_in(b_at.domain()).unwrap();
    assert!(
        uhd_events
            .iter()
            .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR)
            .all(|event| event.payload["outcome"] == "send_asap" && event.payload["target"]["ticks"].as_i64() == Some(b_ticks)),
        "only B may be late: {uhd_events:?}"
    );
    let mock = pair.mock.finish_in_place();
    let uhd = pair.uhd.finish_in_place();
    let gap = |instance: &ProviderInstance, namespace: &str| {
        let mut starts: Vec<i64> = manifest_rows(instance, namespace).iter().map(|row| row["target"]["ticks"].as_i64().unwrap()).collect();
        starts.sort_unstable();
        assert_eq!(starts.len(), 2, "{namespace}: B and the untimed burst: {starts:?}");
        starts[1] - starts[0]
    };
    let (mock_gap, uhd_gap) = (gap(&mock, "ezsdr.radio.mock.bursts"), gap(&uhd, "ezsdr.radio.uhd.bursts"));
    assert_eq!(mock_gap, 25, "the Mock's untimed burst starts at B's front, s + max(10, 4) + 15");
    assert!(uhd_gap >= 25, "the UHD's untimed burst starts no earlier than B's front: {uhd_gap}");
}

/// What each Provider refused among transmit values and bursts: the kind and the Action id, in order (RM-22). The samples are not compared.
fn tx_decisions(events: &[Event]) -> Vec<(String, u64)> {
    events
        .iter()
        .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED && matches!(event.payload["kind"].as_str(), Some("update_parameter" | "tx_burst")))
        .map(|event| (event.payload["kind"].as_str().unwrap().to_owned(), event.payload["action"].as_u64().unwrap()))
        .collect()
}

/// A burst of 40 samples on transmit stream 0, booked at `relative` ms from T0 on each Provider; repeated when `repeat`.
fn s14_burst(pair: &CancellationPair, relative: i64, repeat: bool) -> (Action, Action) {
    let action = |at: TimePoint| Action::TxBurst {
        target: Target::Resource { resource: Ident::parse("radio").unwrap(), path: "tx/s0".to_owned() },
        waveform: pair.sized(40).clone(),
        repeat,
        at: ezsdr_kernel::time::AbsoluteDeadline::new(at),
        requested_at: None,
        late_policy: ezsdr_kernel::stream::LatePolicy::SendAsapAndFlag,
        metadata: BTreeMap::new(),
    };
    (action(tx_sample_time(&pair.mock.clocks, 0, 0, pair.t0 + ms(relative))), action(tx_sample_time(&pair.uhd.clocks, 0, 0, pair.uhd_t0 + ms(relative))))
}

/// The transmit frequency, set on the radio as a value (RM-27: `cold`).
fn s14_value(frequency: f64) -> (Action, Action) {
    let action = || Action::UpdateParameter {
        target: Target::Resource { resource: Ident::parse("radio").unwrap(), path: String::new() },
        key: key("radio.tx.c0.frequency_hz"),
        value: Value::num(frequency).unwrap(),
        class: ezsdr_kernel::module_api::UpdateClass::Cold,
        at: None,
    };
    (action(), action())
}

/// Queues `actions` at the given root ticks on each Provider, then waits until each has judged them.
fn s14_submit(pair: &mut CancellationPair, token: u64, mock_tick: i64, uhd_tick: i64, actions: (Action, Action)) {
    pair.mock.step(mock_tick);
    pair.uhd.wait_until(uhd_tick);
    pair.mock.push(actions.0);
    pair.uhd.push(actions.1);
    pair.mock_barrier(token);
    pair.mock.step(mock_tick);
    pair.uhd_barrier(token);
}

fn s14_at(pair: &mut CancellationPair, token: u64, relative: i64, actions: (Action, Action)) {
    let (mock_tick, uhd_tick) = (pair.t0 + ms(relative), pair.uhd_t0 + ms(relative));
    s14_submit(pair, token, mock_tick, uhd_tick, actions);
}

/// S14 (R2–R4): transmit values on an empty stream decide alike on both Providers, by the kind of each refusal and its Action id.
/// The bursts are 40 samples and each is booked at least the in-flight window after the one before (§5, P-6). The samples are not compared.
/// S5's sample-exact boundary is not here: a receipt on the UHD rig is the first uhd-control poll after the submitted tick (1 ms later, `POLL`), longer than a
/// 40-sample burst, so the boundary is pinned per Provider (`ur_23_a_value_at_the_exclusive_end_of_a_burst_is_accepted`, `mr_16_a_transmit_value_at_the_exclusive_end_is_accepted`).
#[test]
fn s14_transmit_values_on_an_empty_stream_decide_the_same_on_both_providers() {
    let selector = selector(&["0:A:ubx:0"], &["0:A:ubx:0"]);
    let constraints = [("radio.tx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.frequency_hz", eq(Scalar::try_from(2.0e9).unwrap()))];
    let new_pair = || CancellationPair::new(selector.clone(), &constraints, &["rx0"], FakeConfig::default(), BTreeMap::new());
    let decided = |pair: &mut CancellationPair| {
        pair.mock.step(pair.t0 + ms(200));
        pair.uhd.wait_until(pair.uhd_t0 + ms(200));
        (tx_decisions(&pair.mock.events()), tx_decisions(&pair.uhd.events()))
    };

    // S2 and S1: A is booked for 100 ms; a value before it starts is refused; one after its end is accepted, and so is B.
    let mut pair = new_pair();
    let a = s14_burst(&pair, 100, false);
    s14_at(&mut pair, 1, 20, a);
    s14_at(&mut pair, 2, 50, s14_value(2.5e9));
    s14_at(&mut pair, 3, 101, s14_value(2.6e9));
    let b = s14_burst(&pair, 120, false);
    s14_at(&mut pair, 4, 110, b);
    let (mock, uhd) = decided(&mut pair);
    assert_eq!(mock, uhd, "S1, S2: the Providers refuse the same");
    assert_eq!(mock.iter().map(|(kind, _)| kind.as_str()).collect::<Vec<_>>(), ["update_parameter"], "only the value before A's start is refused: {mock:?}");

    // S4: two values before a burst are both taken, and the burst is booked after them; a value before its start is refused.
    let mut pair = new_pair();
    s14_at(&mut pair, 1, 20, s14_value(2.5e9));
    s14_at(&mut pair, 2, 30, s14_value(2.55e9));
    let a = s14_burst(&pair, 100, false);
    s14_at(&mut pair, 3, 40, a);
    s14_at(&mut pair, 4, 60, s14_value(2.7e9));
    let (mock, uhd) = decided(&mut pair);
    assert_eq!(mock, uhd, "S4: the Providers refuse the same");
    assert_eq!(mock.iter().map(|(kind, _)| kind.as_str()).collect::<Vec<_>>(), ["update_parameter"], "the value after the booking is refused, and the two before it are taken: {mock:?}");

    // S3: a repeat plays, so a value is refused; after a Stop it is refused until the in-flight window ends, and taken after it.
    let mut pair = new_pair();
    let repeat = s14_burst(&pair, 20, true);
    s14_at(&mut pair, 1, 20, repeat);
    s14_at(&mut pair, 2, 50, s14_value(2.5e9));
    s14_at(&mut pair, 3, 60, stop_both("tx/s0"));
    s14_at(&mut pair, 4, 65, s14_value(2.6e9));
    s14_at(&mut pair, 5, 71, s14_value(2.7e9));
    let (mock, uhd) = decided(&mut pair);
    assert_eq!(mock, uhd, "S3: the Providers refuse the same");
    assert_eq!(mock.iter().map(|(kind, _)| kind.as_str()).collect::<Vec<_>>(), ["update_parameter", "update_parameter"], "the value while the repeat plays and the one inside the window: {mock:?}");
}

// ---------------------------------------------------------------- O1c: the spec oracle on both Providers, under any scheduling
//
// The paired rig's UHD owner runs on a thread against a manual clock and the rig's thread is a participant of it, so the SCENARIO is established independently of scheduling and only
// what no scheduling can move is asserted: bookings are SendAsapAndFlag (the poll phase of uhd-control, up to 1 ms, moves a start, which the records report, it never drops
// it), and a Stop or a switch is received only after the device is observed to have started the burst (`o1_wait_playing`: the wait's length
// changes when the Stop is received, never what is asserted). The Mock is stepped by the test and keeps its planned instants. The waveform's
// identity and phase against the burst's OBSERVED start (its record's `target`), the samples the device played against what the record
// counts, a Stop's cut against the Stop the Provider says it applied (`tx_stop_applied`), and a switch's new waveform against the boundary
// the row records. The Mock's side is deterministic (stepped by the test), so its records are compared with the oracle exactly. Receipt
// instants, admission, placement and lateness, and the switch row's outcome, are the frozen rigs' (`o1_uhd_*` in `tx.rs`, `o1_mock_*` in
// `mock_channel.rs`); this file asserts no cross-Provider switch row. Observation limit: `transmitted` is channel 0, and the Mock has no
// sample view in this rig.

fn o1_pair(ramps: &[(u8, usize)]) -> CancellationPair {
    CancellationPair::new_with(
        selector(&["0:A:ubx:0"], &["0:A:ubx:0"]),
        &[("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))],
        &["rx0"],
        FakeConfig::default(),
        BTreeMap::new(),
        ramps,
    )
}

/// The oracle's parameters for a Provider's transmit clock: window 10 ms, device lead 2 ms, allowance 3 ms (profile values, 1 MS/s on a 200 MHz root).
fn o1_params(clocks: &ClockRegistry) -> oracle::Params {
    let record = clocks.sample_clock_records().into_iter().filter(|record| record.stream.path() == "radio/tx/s0").min_by_key(|record| record.origin.ticks_in(record.root).unwrap()).unwrap();
    assert_eq!(record.root_ticks_per_tick.den(), 1);
    oracle::Params {
        clock: oracle::Clock { origin: record.origin.ticks_in(record.root).unwrap(), n: record.root_ticks_per_tick.num() as i64 },
        root_hz: ROOT_HZ,
        window: ms(10),
        lead: ms(2),
        allowance: ms(3),
    }
}

/// The UHD device's samples `[from, to)` on channel 0.
fn o1_uhd_samples(pair: &CancellationPair, from: i64, to: i64) -> Vec<Option<[f32; 2]>> {
    pair.uhd.device.transmitted(tx_device_tick(&pair.uhd.clocks, 0, from), (to - from) as usize)
}

/// Checks the oracle's `slots` against the UHD device over `[from, to)`; the cut found, if the slots allow one.
fn o1_check_uhd(pair: &CancellationPair, slots: &oracle::Slots, from: i64, to: i64) -> Option<i64> {
    let seen = o1_uhd_samples(pair, from, to);
    slots.check(from, to, &|k| seen[(k - from) as usize]).unwrap_or_else(|error| panic!("{error}: {:?}", pair.uhd.device.calls()))
}

fn o1_records(instance: &ProviderInstance, namespace: &str) -> Vec<(i64, i64, i64, String)> {
    manifest_rows(instance, namespace)
        .iter()
        .map(|row| (row["target"]["ticks"].as_i64().unwrap(), row["samples"].as_i64().unwrap(), row["wraps"].as_i64().unwrap(), row["end"].as_str().unwrap().to_owned()))
        .collect()
}

/// A transmit burst on stream 0 for both Providers at `relative` ms from T0 under SendAsapAndFlag (the poll phase of uhd-control moves its start, which its
/// record reports; it is never dropped), repeating when `repeat`, as a switch when `switch`.
fn o1_asap(pair: &CancellationPair, wave: &ezsdr_kernel::manifest::ArtifactRef, relative: i64, repeat: bool, switch: bool) -> (Action, Action) {
    let action = |clocks: &ClockRegistry, root: i64| Action::TxBurst {
        target: Target::Resource { resource: Ident::parse("radio").unwrap(), path: "tx/s0".to_owned() },
        waveform: wave.clone(),
        repeat,
        at: ezsdr_kernel::time::AbsoluteDeadline::new(tx_samples_after(clocks, 0, root + ms(relative), 0)),
        requested_at: None,
        late_policy: ezsdr_kernel::stream::LatePolicy::SendAsapAndFlag,
        metadata: if switch { BTreeMap::from([(key("radio.repeat_operation"), Value::Scalar(Scalar::from("switch")))]) } else { BTreeMap::new() },
    };
    (action(&pair.mock.clocks, pair.t0), action(&pair.uhd.clocks, pair.uhd_t0))
}

/// The Mock receives `action` at `relative` ms from its T0 (it is stepped by the test: its instants are its plan).
fn o1_mock_at(pair: &mut CancellationPair, relative: i64, action: Action) {
    pair.mock.step(pair.t0 + ms(relative));
    pair.mock.push(action);
    pair.mock.step(pair.t0 + ms(relative));
}

/// The UHD receives `actions` in one receipt (they enter its queue under one lock, so uhd-control takes them with no device time between,
/// whatever the test thread does) once its device clock is at `relative` ms from its T0: the clock stands there while the rig's thread, a participant, works, so a receipt is the first uhd-control poll after that tick (`UhdRig::wait_until`).
fn o1_uhd_at(pair: &mut CancellationPair, token: u64, relative: i64, actions: impl IntoIterator<Item = Action>) {
    pair.uhd.wait_until(pair.uhd_t0 + ms(relative));
    pair.uhd.actions.actions.push_all(actions.into_iter().map(|action| (action, false)));
    pair.uhd_barrier(token);
}

/// The UHD's device time now, in root ticks.
fn o1_uhd_now(pair: &CancellationPair) -> i64 {
    pair.uhd.time.now(pair.uhd.root).unwrap().ticks_in(pair.uhd.root).unwrap()
}

/// Advances the UHD's device clock a millisecond at a time from `from` ms until the owner has issued its first timed payload send (the first
/// send that names an instant: the burst's start), and returns the first ms, relative to T0, at which a Stop or a switch is received after the
/// device has passed that start. A wait is fine; the instant a Stop is received is not what the tests assert.
fn o1_wait_playing(pair: &mut CancellationPair, from: i64) -> i64 {
    for relative in from..from + 400 {
        pair.uhd.wait_until(pair.uhd_t0 + ms(relative));
        pair.uhd.settle(Wall::from_millis(1));
        let start = pair.uhd.device.calls().iter().find_map(|call| {
            let parts: Vec<&str> = call.split_whitespace().collect();
            let numbered = parts.first() == Some(&"tx_send") && parts.get(1).is_some_and(|s| s.parse::<usize>().is_ok());
            let at = parts.iter().find_map(|p| p.strip_prefix("at=")).filter(|at| *at != "-")?.parse::<i64>().ok()?;
            numbered.then_some(at)
        });
        if let Some(start) = start {
            return (start - pair.uhd_t0 + ms(1) - 1) / ms(1) + 3;
        }
    }
    panic!("the UHD never issued a timed payload send: {:?}", pair.uhd.device.calls());
}

#[test]
fn o1_parity_a_repeat_keeps_its_phase_across_wraps_from_its_observed_start() {
    // RM-13 (seq 3): a 1 000-sample repeat is `R[(k - target) mod 1000]` over every wrap the device played, from the start its record names; the
    // record counts exactly the samples the device played (a Stop ends it), `wraps = samples / 1000`; the Mock's record is its planned start
    // and its exact MR-25 cut for the Stop it received at 50 ms.
    let mut pair = o1_pair(&[(7, 1_000)]);
    let wave = pair.ramp(7, 1_000).clone();
    submit(&mut pair, 201, 6, |pair| vec![o1_asap(pair, &wave, 40, true, false)]);
    let rel = o1_wait_playing(&mut pair, 7).max(50);
    let (mock_stop, uhd_stop) = (o1_asap_stop(), o1_asap_stop());
    o1_mock_at(&mut pair, 50, mock_stop);
    o1_uhd_at(&mut pair, 202, rel, [uhd_stop]);
    pair.uhd.wait_until(pair.uhd_t0 + ms(rel + 70));
    pair.mock.step(pair.t0 + ms(120));
    let mock = pair.mock.finish_in_place();
    let uhd = pair.uhd.finish_in_place();
    let uhd_rows = o1_records(&uhd, "ezsdr.radio.uhd.bursts");
    assert_eq!(uhd_rows.len(), 1, "{uhd_rows:?}");
    let (target, samples, wraps, end) = uhd_rows[0].clone();
    assert_eq!((end.as_str(), wraps), ("stop", samples / 1_000));
    let slots = oracle::Slots { segs: vec![oracle::Seg::repeating(7, target, 1_000, i64::MAX).cuttable()], cut: Some((target + samples, target + samples + 1)) };
    let cut = o1_check_uhd(&pair, &slots, target - 10, target + samples + 100).expect("the Stop cuts the repeat");
    assert_eq!(cut - target, samples, "the device played exactly what the record counts");
    let p = o1_params(&pair.mock.clocks);
    let mock_rows = o1_records(&mock, "ezsdr.radio.mock.bursts");
    assert_eq!(mock_rows.len(), 1, "{mock_rows:?}");
    let cut = oracle::mock_cut(&p, pair.t0 + ms(50), &[oracle::MBurst { start: mock_rows[0].0, len: None, started: true, before_stop: true }], None);
    assert_eq!((mock_rows[0].1, mock_rows[0].2, mock_rows[0].3.as_str()), (cut - mock_rows[0].0, (cut - mock_rows[0].0) / 1_000, "stop"));
}

fn o1_asap_stop() -> Action {
    stop_both("tx/s0").0
}

#[test]
fn o1_parity_a_long_finite_burst_is_what_its_record_counts() {
    // RM-13, RM-15 (seq 1): a 3 000-sample burst, longer than one send, plays `F[k - target]` for every sample from the start its record names,
    // silence after; both Providers record exactly 3 000 payload samples ending at the waveform's end. The booking is SendAsapAndFlag: a host
    // that is late moves the start, which the record reports.
    let mut pair = o1_pair(&[(5, 3_000)]);
    let wave = pair.ramp(5, 3_000).clone();
    submit(&mut pair, 201, 6, |pair| vec![o1_asap(pair, &wave, 40, false, false)]);
    let rel = o1_wait_playing(&mut pair, 7);
    pair.uhd.wait_until(pair.uhd_t0 + ms(rel + 60));
    pair.mock.step(pair.t0 + ms(120));
    let mock = pair.mock.finish_in_place();
    let uhd = pair.uhd.finish_in_place();
    for (case, rows) in [("UHD", o1_records(&uhd, "ezsdr.radio.uhd.bursts")), ("Mock", o1_records(&mock, "ezsdr.radio.mock.bursts"))] {
        assert_eq!(rows.iter().map(|row| (row.1, row.3.as_str())).collect::<Vec<_>>(), [(3_000, "eob")], "{case}: {rows:?}");
    }
    let target = o1_records(&uhd, "ezsdr.radio.uhd.bursts")[0].0;
    o1_check_uhd(&pair, &oracle::Slots { segs: vec![oracle::Seg::once(5, target, 3_000)], cut: None }, target - 10, target + 3_100);
    let played = o1_uhd_samples(&pair, target - 10, target + 3_100).into_iter().flatten().filter(|sample| *sample != [0.0, 0.0]).count();
    assert_eq!(played, 3_000, "the device played exactly the 3 000 samples the record counts");
}

#[test]
fn o1_parity_a_stop_cuts_a_playing_burst_inside_the_applied_range() {
    // RM-16, MR-25 (seq 3, 6): a 24 000-sample burst (longer than the 10 ms window) from 40 ms. The Mock receives a Stop at 41 ms; the UHD one
    // once the burst is observed started. The UHD's cut lies in [t_k, B], read from the Stop the Provider says it applied (`tx_stop_applied`,
    // #83); its record counts the samples the device played and ends `stop`, or ends with the waveform when a late-applied Stop found it
    // handed whole (which the Stop's bound allows only if it ended by `B`). The Mock's cut is MR-25's point and its record the same.
    let mut pair = o1_pair(&[(5, 24_000)]);
    let wave = pair.ramp(5, 24_000).clone();
    submit(&mut pair, 201, 6, |pair| vec![o1_asap(pair, &wave, 40, false, false)]);
    let rel = o1_wait_playing(&mut pair, 7).max(41);
    o1_mock_at(&mut pair, 41, o1_asap_stop());
    o1_uhd_at(&mut pair, 202, rel, [o1_asap_stop()]);
    pair.uhd.wait_until(pair.uhd_t0 + ms(rel + 70));
    pair.mock.step(pair.t0 + ms(120));
    let mock = pair.mock.finish_in_place();
    let uhd = pair.uhd.finish_in_place();
    let applied: Vec<_> = manifest_rows(&uhd, "ezsdr.radio.uhd.timing").into_iter().filter(|row| row["what"] == "tx_stop_applied").collect();
    assert_eq!(applied.len(), 1, "{applied:?}");
    let up = o1_params(&pair.uhd.clocks);
    let (t_k, bound) = (up.clock.at_or_after(applied[0]["at"].as_i64().unwrap()), up.clock.at_or_after(applied[0]["bound"].as_i64().unwrap()));
    assert!((10_005..=10_006).contains(&(bound - t_k)), "the applied Stop bound is the window plus five: {t_k} {bound}");
    let (target, samples, _, end) = o1_records(&uhd, "ezsdr.radio.uhd.bursts")[0].clone();
    let slots = oracle::Slots { segs: vec![oracle::Seg::once(5, target, 24_000).cuttable()], cut: Some((t_k, bound)) };
    let cut = o1_check_uhd(&pair, &slots, target - 10, target + 24_100);
    match end.as_str() {
        "stop" => {
            let cut = cut.expect("the Stop cuts the burst");
            assert!((t_k..=bound).contains(&cut), "the cut {cut} is outside [{t_k}, {bound}]");
            assert_eq!(samples, cut - target, "the record counts the samples the device played");
        }
        "eob" => assert!(samples == 24_000 && target + 24_000 <= bound, "a burst that ended by itself ended by the Stop's bound: {target} {bound} {samples}"),
        other => panic!("the burst ended {other}"),
    }
    let mp = o1_params(&pair.mock.clocks);
    let mock_rows = o1_records(&mock, "ezsdr.radio.mock.bursts");
    let mock_cut = oracle::mock_cut(&mp, pair.t0 + ms(41), &[oracle::MBurst { start: mock_rows[0].0, len: Some(24_000), started: true, before_stop: true }], None);
    assert_eq!((mock_rows[0].1, mock_rows[0].3.as_str()), (mock_cut - mock_rows[0].0, "stop"));
}

#[test]
fn o1_parity_a_switched_repeat_changes_waveform_at_its_recorded_boundary_on_its_grid() {
    // RM-37 (seq 4): R (700 samples, from 40 ms) switched to a 400-sample waveform, then a Stop closes the new repeat. The UHD receives the
    // switch once R is observed started; its new waveform starts at the boundary its `switches` row records, which lies on R's grid from
    // R's observed start; the old repeat is `R[(k - target) mod 700]` to it and the new one `N[(k - b) mod
    // 400]` from it to the Stop's cut (in the applied range), the old record ends there and the successor's record is its own: target `b`, the
    // samples played, wraps, end stop. The switch row names the switch (its action id) and R (the predecessor). The Mock's boundary is the
    // oracle's for its own receipt at 43 ms. No row outcome is compared between the Providers.
    let mut pair = o1_pair(&[(8, 700), (9, 400)]);
    let (old, new) = (pair.ramp(8, 700).clone(), pair.ramp(9, 400).clone());
    submit(&mut pair, 201, 6, |pair| vec![o1_asap(pair, &old, 40, true, false)]);
    let rel = o1_wait_playing(&mut pair, 7).max(43);
    let (mock_switch, _) = o1_asap(&pair, &new, 44, true, true);
    let (_, uhd_switch) = o1_asap(&pair, &new, rel, true, true);
    o1_mock_at(&mut pair, 43, mock_switch);
    o1_uhd_at(&mut pair, 202, rel, [uhd_switch]);
    // The Stop that ends the new repeat: the Mock at 70 ms; the UHD 30 ms after the device time at which it actually received the switch (so
    // the Stop's bound can never come before the switch's boundary, however late the test thread was).
    let received = o1_uhd_now(&pair);
    let stop_rel = (rel + 30).max((received - pair.uhd_t0 + ms(1) - 1) / ms(1) + 30);
    o1_mock_at(&mut pair, 70, o1_asap_stop());
    o1_uhd_at(&mut pair, 203, stop_rel, [o1_asap_stop()]);
    pair.uhd.wait_until(pair.uhd_t0 + ms(stop_rel + 70));
    pair.mock.step(pair.t0 + ms(stop_rel + 70));
    let mock = pair.mock.finish_in_place();
    let uhd = pair.uhd.finish_in_place();
    let rows = manifest_rows(&uhd, "ezsdr.radio.uhd.switches");
    assert_eq!(rows.len(), 1, "{rows:?}");
    // The queue gives the UHD's ids in order, its barriers included: R is 1, its barrier 2, the switch 3.
    assert_eq!((rows[0]["action_id"].as_u64(), rows[0]["predecessor"].as_u64()), (Some(3), Some(1)), "{rows:?}");
    let b = rows[0]["boundary"].as_i64().unwrap();
    let uhd_recs = o1_records(&uhd, "ezsdr.radio.uhd.bursts");
    let target = uhd_recs[0].0;
    assert_eq!((b - target) % 700, 0, "the boundary {b} is on the repeat's grid from {target}");
    let applied: Vec<_> = manifest_rows(&uhd, "ezsdr.radio.uhd.timing").into_iter().filter(|row| row["what"] == "tx_stop_applied").collect();
    assert_eq!(applied.len(), 1, "{applied:?}");
    let up = o1_params(&pair.uhd.clocks);
    let (t_k, bound) = (up.clock.at_or_after(applied[0]["at"].as_i64().unwrap()), up.clock.at_or_after(applied[0]["bound"].as_i64().unwrap()));
    let slots = oracle::Slots { segs: vec![oracle::Seg::repeating(8, target, 700, b), oracle::Seg::repeating(9, b, 400, i64::MAX).cuttable()], cut: Some((t_k, bound)) };
    let cut = o1_check_uhd(&pair, &slots, target - 10, bound + 100).expect("the Stop cuts the new repeat");
    assert_eq!(uhd_recs.len(), 2, "{uhd_recs:?}");
    assert_eq!(uhd_recs[0].1, b - target, "the old repeat's record ends at the boundary");
    assert_eq!((uhd_recs[1].0, uhd_recs[1].1, uhd_recs[1].2, uhd_recs[1].3.as_str()), (b, cut - b, (cut - b) / 400, "stop"), "the successor's record");
    // The Mock: its boundary is the oracle's for its receipt at 43 ms and `at` of 44 ms; the repeat and its successor both record.
    let mp = o1_params(&pair.mock.clocks);
    let mock_rows = o1_records(&mock, "ezsdr.radio.mock.bursts");
    let at = tx_samples_after(&pair.mock.clocks, 0, pair.t0 + ms(44), 0);
    let at = at.ticks_in(at.domain()).unwrap();
    let mb = oracle::switch_boundary(&mp, mock_rows[0].0, 700, pair.t0 + ms(43), Some(at)).unwrap();
    let rows = manifest_rows(&mock, "ezsdr.radio.mock.switches");
    assert_eq!((rows.len(), rows[0]["boundary"].as_i64(), rows[0].get("moved_to")), (1, Some(mb), None), "{rows:?}");
    assert_eq!((rows[0]["action_id"].as_u64(), rows[0]["predecessor"].as_u64()), (Some(2), Some(1)), "the Mock's queue gives the repeat 1 and the switch 2: {rows:?}");
    let mock_cut = oracle::mock_cut(&mp, pair.t0 + ms(70), &[oracle::MBurst { start: mb, len: None, started: true, before_stop: true }], None);
    assert_eq!(mock_rows.len(), 2, "{mock_rows:?}");
    assert_eq!(mock_rows[0].1, mb - mock_rows[0].0, "the Mock's old repeat ends at the boundary");
    assert_eq!((mock_rows[1].0, mock_rows[1].1, mock_rows[1].2, mock_rows[1].3.as_str()), (mb, mock_cut - mb, (mock_cut - mb) / 400, "stop"), "the Mock's successor record");
}

// ---------------------------------------------------------------- the rig's driver (note 51 stage 1b)

fn one_stream_rig(config: FakeConfig) -> UhdRig {
    UhdRig::new(selector(&["0:A:ubx:0"], &["0:A:ubx:0"]), &[], config).unwrap()
}

#[test]
fn uhd_rig_the_device_clock_stands_still_while_the_test_thread_works_after_each_wait() {
    let mut rig = one_stream_rig(FakeConfig::default());
    let host = rig.device.host();
    assert_stands_still(&host, "built");
    rig.settle(Wall::from_millis(5));
    assert_stands_still(&host, "settle");
    rig.arm().unwrap();
    let now = rig.time.now(rig.root).unwrap().ticks_in(rig.root).unwrap();
    rig.start(now + ms(2_001)).unwrap();
    rig.wait_until(now + ms(5));
    assert_stands_still(&host, "wait_until");
    rig.push(timer_barrier(rig.root, now + ms(2_010), 1));
    rig.wait_for_event("the barrier's refusal", |event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED && event.payload["kind"] == "set_timer");
    assert_stands_still(&host, "wait_for_event");
    let _ = rig.finish();
    let gone = host.now();
    std::thread::sleep(Wall::from_millis(20));
    assert!(host.now() > gone, "the clock runs again once the rig is gone");
}

#[test]
fn uhd_rig_settle_advances_the_device_clock_by_its_duration_and_wait_until_reaches_its_tick() {
    let mut rig = one_stream_rig(FakeConfig::default());
    let host = rig.device.host();
    let before = host.now();
    rig.settle(Wall::from_millis(7));
    assert_eq!(host.now() - before, Wall::from_millis(7) + Wall::from_micros(1), "settle is the driver's own idle wait on the device clock, and ends one microsecond past its duration (R1)");
    let tick = rig.time.now(rig.root).unwrap().ticks_in(rig.root).unwrap() + ms(3);
    rig.wait_until(tick);
    let reached = rig.time.now(rig.root).unwrap().ticks_in(rig.root).unwrap();
    // One microsecond past the tick: the rig lets every participant due at the tick run before it returns (receipts at the next poll).
    assert!(reached >= tick + ROOT_HZ / 1_000_000 - 1 && reached < tick + ms(1), "wait_until returns just after its tick: {reached} for {tick}");
}

#[test]
fn uhd_rig_an_action_pushed_after_wait_until_is_received_at_the_first_poll_after_its_tick() {
    let mut pair = CancellationPair::new(
        selector(&["0:A:ubx:0"], &["0:A:ubx:0"]),
        &[("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))],
        &["rx0"],
        FakeConfig::default(),
        BTreeMap::new(),
    );
    // uhd-control's poll grid has the phase of `uhd_t0`, so each of these ticks is an instant at which a poll is due: the receipt is the next one, every time, never the same instant. Without the wait's last microsecond the order of the two threads decides, and a run of 60 ticks shows it.
    let ticks: Vec<i64> = (0..60).map(|k| 10 + 3 * k).collect();
    for tick in &ticks {
        pair.uhd.wait_until(pair.uhd_t0 + ms(*tick));
        pair.uhd_barrier(*tick as u64);
    }
    let root = pair.uhd.root;
    let receipts: Vec<i64> = pair
        .uhd
        .events()
        .iter()
        .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED && event.payload["kind"] == "set_timer")
        .map(|event| event.time.ticks_in(root).unwrap())
        .collect();
    // A receipt's tick is the Authority's extrapolation of the host clock: the nanosecond of the poll converted to root ticks, which rounds by one tick.
    let expected: Vec<i64> = ticks.iter().map(|tick| pair.uhd_t0 + ms(tick + 1)).collect();
    assert_eq!(receipts.len(), expected.len(), "every barrier was refused once: {receipts:?}");
    for ((receipt, expected), tick) in receipts.iter().zip(&expected).zip(&ticks) {
        assert!(receipt.abs_diff(*expected) <= 1, "the barrier at {tick} ms is received at the poll one millisecond after it: {receipt} for {expected}");
    }
}

#[test]
fn uhd_rig_a_booking_after_the_test_thread_was_away_is_received_inside_its_window() {
    let mut pair = CancellationPair::new(
        selector(&["0:A:ubx:0"], &["0:A:ubx:0"]),
        &[("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))],
        &["rx0"],
        FakeConfig::default(),
        BTreeMap::new(),
    );
    let single = pair.single.clone();
    submit(&mut pair, 201, 91, |pair| {
        std::thread::sleep(Wall::from_millis(200));
        vec![short_booking(pair, &single, 0, 100, 0)]
    });
    pair.uhd.wait_until(pair.uhd_t0 + ms(140));
    pair.mock.step(pair.t0 + ms(140));
    let mock = pair.mock.finish_in_place();
    let mock_events = pair.mock.events();
    let uhd = pair.uhd.finish_in_place();
    let uhd_events = pair.uhd.events();
    assert_eq!(late_drops(&tx_rejected_rows(&mock, "ezsdr.radio.mock.rejected"), &mock_events), (0, Vec::new()), "nothing is late on the Mock");
    assert_eq!(late_drops(&tx_rejected_rows(&uhd, "ezsdr.radio.uhd.rejected"), &uhd_events), (0, Vec::new()), "nothing is late on UHD, however long the test thread was away");
    assert_eq!(manifest_rows(&uhd, "ezsdr.radio.uhd.bursts").len(), 1, "the booking played");
}

#[test]
fn uhd_rig_two_rigs_on_one_thread_move_only_the_clock_the_thread_waits_on() {
    let mut a = one_stream_rig(FakeConfig::default());
    let mut b = one_stream_rig(FakeConfig::default());
    let (host_a, host_b) = (a.device.host(), b.device.host());
    let (a0, b0) = (host_a.now(), host_b.now());
    a.settle(Wall::from_millis(7));
    assert_eq!(host_a.now() - a0, Wall::from_millis(7) + Wall::from_micros(1), "A moved by the wait on A");
    assert_eq!(host_b.now(), b0, "B stood still while the thread waited on A");
    let a1 = host_a.now();
    b.settle(Wall::from_millis(5));
    assert_eq!(host_b.now() - b0, Wall::from_millis(5) + Wall::from_micros(1), "B moved by the wait on B");
    assert_eq!(host_a.now(), a1, "A stood still while the thread waited on B");
}

fn rig_with_uhd_clock_asleep_inside_its_read() -> UhdRig {
    let rig = one_stream_rig(FakeConfig { faults: vec![FakeFault::Unreachable(rig::FAULT_AT)], ..FakeConfig::default() });
    let host = rig.device.host();
    rig::asleep_inside_its_read(rig, &host, |rig| Arc::strong_count(&rig.time), UhdRig::settle)
}

#[test]
fn uhd_rig_dropped_joins_uhd_clock_even_asleep_inside_its_read() {
    let mut rig = rig_with_uhd_clock_asleep_inside_its_read();
    let seen = rig._authority.install_probe(&rig.time, rig.device.host());
    let before = rig.device.host().now();
    drop(rig);
    assert_joined(&seen, before, "dropped");
}

#[test]
fn uhd_rig_unwound_joins_uhd_clock_even_asleep_inside_its_read() {
    let probe = Mutex::new(None);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut rig = rig_with_uhd_clock_asleep_inside_its_read();
        let seen = rig._authority.install_probe(&rig.time, rig.device.host());
        *probe.lock().unwrap() = Some((seen, rig.device.host().now()));
        panic!("the test body fails");
    }));
    assert!(result.is_err());
    let (seen, before) = probe.into_inner().unwrap().unwrap();
    assert_joined(&seen, before, "unwound");
}

#[test]
fn uhd_rig_refuses_a_fault_that_stops_the_manual_clock_for_good() {
    let refused = std::panic::catch_unwind(|| UhdRig::new(selector(&["0:A:ubx:0"], &["0:A:ubx:0"]), &[], FakeConfig { faults: vec![FakeFault::TxBlocks], ..FakeConfig::default() }));
    let text = refused.err().and_then(|payload| payload.downcast_ref::<String>().cloned().or_else(|| payload.downcast_ref::<&str>().map(|text| (*text).to_owned()))).expect("the constructor panics");
    assert!(text.contains("TxBlocks"), "{text}");
}

#[test]
fn uhd_rig_wait_for_events_writes_the_infra_marker_when_its_bound_expires() {
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "uhd_rig_fixture_wait_for_events_expires", "--test-threads=1", "--nocapture"])
        .env("EZSDR_RIG_FIXTURE", "wait_for_events_expires")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "the fixture fails: {stderr}");
    assert!(stderr.contains("EZSDR-INFRA: uhd_rig_fixture_wait_for_events_expires: the rig's wait for an event that never comes"), "{stderr}");
}

/// Run by the test above in a child process; a no-op otherwise.
#[test]
fn uhd_rig_fixture_wait_for_events_expires() {
    if std::env::var("EZSDR_RIG_FIXTURE").is_ok_and(|name| name == "wait_for_events_expires") {
        one_stream_rig(FakeConfig::default()).wait_for_events_within("an event that never comes", 1, |_| false, Wall::from_secs(60), Wall::from_millis(50));
    }
}

#[test]
fn uhd_rig_an_action_is_not_received_at_the_instant_it_was_pushed() {
    let mut rig = one_stream_rig(FakeConfig::default());
    rig.push(timer_barrier(rig.root, 0, 1));
    assert!(rig.actions.recv().is_none(), "the driver is busy, so the clock stands at the instant of the push: a poll at it must not take the Action");
    rig.settle(Wall::from_micros(1));
    assert!(rig.actions.recv().is_some(), "the clock has moved past the push");
}

#[test]
fn uhd_rig_a_wait_that_ends_on_a_poll_instant_still_receives_at_the_next_poll() {
    let mut rig = one_stream_rig(FakeConfig::default());
    rig.arm().unwrap();
    let host = rig.device.host();
    // The clock stands while this thread works: both readings are of one instant, and uhd-control is spawned inside `start` at it, so its polls are at `g0 + k ms`.
    let (s0, g0) = (rig.time.now(rig.root).unwrap().ticks_in(rig.root).unwrap(), host.now());
    rig.start(s0 + ms(2_001)).unwrap();
    let mut pushed_on = Vec::new();
    for k in 1..=12u64 {
        // The next poll instant at least 2 ms ahead (the event waits end on a poll instant too, so where the clock is afterwards is not for the test to assume). `settle` ends one microsecond past its
        // duration (R1): this one ends exactly on the poll, where the driver's wake and the poll share the instant.
        let m = (host.now() - g0).as_millis() as u64 + 2;
        let grid = g0 + Wall::from_millis(m);
        rig.settle(grid - Wall::from_micros(1) - host.now());
        assert_eq!(host.now(), grid, "the wait ended on a poll instant");
        rig.push(timer_barrier(rig.root, s0 + ms(2_010), k));
        pushed_on.push(m as i64);
        rig.wait_for_events("the barrier's refusal", k as usize, |event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED && event.payload["kind"] == "set_timer");
    }
    let root = rig.root;
    let receipts: Vec<i64> = rig
        .events()
        .iter()
        .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED && event.payload["kind"] == "set_timer")
        .map(|event| event.time.ticks_in(root).unwrap())
        .collect();
    assert_eq!(receipts.len(), pushed_on.len(), "{receipts:?}");
    for (receipt, m) in receipts.iter().zip(&pushed_on) {
        assert!(receipt.abs_diff(s0 + ms(m + 1)) <= 1, "the Action pushed at poll {m} is received at the poll after it: {receipt} for {}", s0 + ms(m + 1));
    }
    let _ = rig.finish();
}

#[test]
fn uhd_rig_wait_for_events_fails_at_its_device_time_horizon_without_a_marker() {
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "uhd_rig_fixture_wait_for_events_horizon", "--test-threads=1", "--nocapture"])
        .env("EZSDR_RIG_FIXTURE", "wait_for_events_horizon")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    // The child's marker line is the child's: printed by this test it would be counted by the runner as a bound that expired here.
    let stderr = stderr.replace("EZSDR-INFRA", "child's marker");
    assert!(!out.status.success(), "the fixture fails: {stderr}");
    assert!(stderr.contains("an event that never comes did not come within"), "{stderr}");
    assert!(!stderr.contains("child's marker"), "the horizon is the test's verdict, not the host's: {stderr}");
}

/// Run by the test above in a child process; a no-op otherwise.
#[test]
fn uhd_rig_fixture_wait_for_events_horizon() {
    if std::env::var("EZSDR_RIG_FIXTURE").is_ok_and(|name| name == "wait_for_events_horizon") {
        one_stream_rig(FakeConfig::default()).wait_for_events_within("an event that never comes", 1, |_| false, Wall::from_millis(50), Wall::from_secs(5));
    }
}
