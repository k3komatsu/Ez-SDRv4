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
//! channel moving from one stream to another (RM-34: the UHD Module refuses it until
//! 1b-4e-2). The Manifest's `sequence` is compared as an order, not as numbers: each
//! Provider counts its own arrivals from its own origin (MockRadio's faults first, uhd-control's
//! loss as 0). Both providers share `channels::Description`, so a shared mistake inside it
//! is invisible here: this file is a net for divergences, and the specs' own tests are the
//! ground truth.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration as Wall;

use ezsdr_kernel::binding::{AdmissionCheckRegistry, Binding, Violation};
use ezsdr_kernel::event::{
    Action, ActionId, Event, EventCollector, EventSource, Target,
};
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::{ClockDomainId, ModuleId, ResourceId, RunId};
use ezsdr_kernel::module_api::{
    ActionReceiver, ActionSubmitter, AttachedPort, Authority, CoerceReport, Endpoint,
    ExecutionClass, ModuleRef, ModuleRegistry, Pacing, PrepareContext, ProfileRef, Provider,
    ProviderInstance, Requested, Version,
};
use ezsdr_kernel::plan::Fragment;
use ezsdr_kernel::policy::{EventKindRegistry, Policy};
use ezsdr_kernel::spec::{Constraint, Ident, Key, Namespace, Scalar, Value};
use ezsdr_kernel::stream::{BackPressure, BlockFlags, BlockRef, DataLink, DropCarry, PublishOutcome};
use ezsdr_kernel::time::{
    ClockDomain, ClockRegistry, Duration, EpochRef, ManualTimeAuthority, Rational, TimePoint,
};
use ezsdr_mock_radio::MockRadio;
use ezsdr_radio::payloads::{RxOverflowCause, RxOverflowPayload};
use ezsdr_radio_uhd::{Device, DeviceAuthority, FakeConfig, FakeDevice, FakeFault, UhdRadio};
use serde_json::json;

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

/// A refusal or a LATE_COMMAND as the floored-refusal test compares it: `(kind, (requested,
/// applied) for a late one, reason for a refusal)`, instants from the Provider's `t0`.
type Reported = Vec<(String, Option<(i64, i64)>, Option<String>)>;

fn eq(value: Scalar) -> Constraint {
    Constraint::Eq { value }
}

/// The selector's shared layout values (design 33 D5).
fn selector(rx: &[&str], tx: &[&str]) -> BTreeMap<Ident, Value> {
    let list = |elements: &[&str]| Value::List(elements.iter().map(|e| Scalar::from(*e)).collect());
    BTreeMap::from([
        (Ident::parse("rx").unwrap(), list(rx)),
        (Ident::parse("tx").unwrap(), list(tx)),
    ])
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
    Requested {
        resource: rid("radio"),
        constraints: constraints.iter().map(|(name, constraint)| (key(name), constraint.clone())).collect(),
    }
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
    actions: Mutex<VecDeque<Action>>,
    given: AtomicU64,
}

impl ActionReceiver for Queue {
    fn recv(&self) -> Option<ezsdr_kernel::module_api::Dispatched> {
        self.actions.lock().unwrap().pop_front().map(|action| {
            let node = action.target().and_then(|target| match target {
                Target::Resource { resource, path } if path.is_empty() => rid(resource.as_str()).into(),
                Target::Resource { resource, path } => Some(rid(&format!("{resource}/{path}"))),
                _ => None,
            });
            let id = ActionId(self.given.fetch_add(1, Ordering::SeqCst) + 1);
            ezsdr_kernel::module_api::Dispatched::new(id, action, node)
        })
    }
}

impl Queue {
    /// The Actions the Provider has not taken yet: an empty queue means the control thread
    /// has taken the last one; its booking follows while the device clock stands still.
    fn queued(&self) -> usize {
        self.actions.lock().unwrap().len()
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
        let actions = Arc::new(Queue { actions: Mutex::new(VecDeque::new()), given: AtomicU64::new(0) });
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
        let prepared = mock.prepare(&fragment(ModuleRef { id: ModuleId::parse("ezsdr.radio.mock").unwrap(), version: Version::new(2, 0, 0) }, constraints), ctx).unwrap();
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
        self.actions.actions.lock().unwrap().push_back(action);
    }

    fn stop_at(&self, node: &str) {
        self.push(Action::Stop { target: Some(Target::Resource { resource: Ident::parse("radio").unwrap(), path: node.to_owned() }) });
    }

    fn update(&self, name: &str, value: Value, class: ezsdr_kernel::module_api::UpdateClass, at: Option<i64>) {
        self.push(Action::UpdateParameter {
            target: Target::Resource { resource: Ident::parse("radio").unwrap(), path: String::new() },
            key: key(name),
            value,
            class,
            at: at.map(|tick| ezsdr_kernel::time::AbsoluteDeadline::new(TimePoint::new(self.root, tick))),
        });
    }

    /// The declared clocks as `(node, ratio)`, smallest first (TM-12).
    fn declared(&self) -> Vec<(String, (u64, u64))> {
        let mut clocks: Vec<_> = self
            .clocks
            .declared_sample_clocks()
            .into_iter()
            .map(|handle| (handle.stream().path().to_owned(), (handle.root_ticks_per_tick().num(), handle.root_ticks_per_tick().den())))
            .collect();
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
    _authority: DeviceAuthority,
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
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: ezsdr_radio_uhd::ClockMode::Manual, ..config }));
        let clocks = Arc::new(ClockRegistry::new());
        let authority = DeviceAuthority::new(device.clone(), clocks.clone(), "internal", "internal", "type=fake").unwrap();
        let (time, root) = (authority.time(), authority.root());
        let mut registry = ModuleRegistry::new();
        let (mut checks, mut kinds) = (AdmissionCheckRegistry::new(), EventKindRegistry::with_kernel_kinds());
        ezsdr_radio::register(&mut registry, &mut checks, &mut kinds).unwrap();
        let pairs: Vec<_> = ["radio", "radio/rx/s0", "radio/tx/s0", "radio/rx/s1", "radio/tx/s1"].iter().flat_map(|source| kinds.kinds().into_iter().map(move |kind| (EventSource::Node { node: rid(source) }, kind))).collect();
        let events = Arc::new(EventCollector::new(&pairs, &kinds.kinds(), 4096, &Policy::default()));
        let queue = Arc::new(Queue { actions: Mutex::new(VecDeque::new()), given: AtomicU64::new(0) });
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
        let prepared = radio.prepare(&fragment(ezsdr_radio_uhd::module_ref(), constraints), ctx)?;
        Ok(UhdRig { radio, device, clocks, time, root, actions: queue, effective: prepared.effective, events, seen_events: Mutex::new(Vec::new()), rx_links, _authority: authority })
    }

    /// An update at the instance's node (KC-23), at `tick` on the root.
    fn update(&self, name: &str, value: Value, class: ezsdr_kernel::module_api::UpdateClass, at: Option<i64>) {
        self.actions.actions.lock().unwrap().push_back(Action::UpdateParameter {
            target: Target::Resource { resource: Ident::parse("radio").unwrap(), path: String::new() },
            key: key(name),
            value,
            class,
            at: at.map(|tick| ezsdr_kernel::time::AbsoluteDeadline::new(TimePoint::new(self.root, tick))),
        });
    }

    fn push(&self, action: Action) {
        self.actions.actions.lock().unwrap().push_back(action);
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

    /// The device's host clock, which a non-participant waits on so the device's time
    /// advances while its threads are idle (plan 30 §4.5).
    fn settle(&mut self, wall: Wall) {
        self.device.host().sleep(wall);
    }

    /// Advances the device's clock to `tick` (on the manual clock, as a non-participant),
    /// letting its threads run on the way (plan 30 §4.5).
    fn wait_until(&mut self, tick: i64) {
        let _ = self.time.wait_until(TimePoint::new(self.root, tick));
    }

    fn declared(&self) -> Vec<(String, (u64, u64))> {
        let mut clocks: Vec<_> = self
            .clocks
            .declared_sample_clocks()
            .into_iter()
            .map(|handle| (handle.stream().path().to_owned(), (handle.root_ticks_per_tick().num(), handle.root_ticks_per_tick().den())))
            .collect();
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
        let deadline = std::time::Instant::now() + Wall::from_secs(10);
        loop {
            let mut seen = self.seen_events.lock().unwrap();
            seen.extend(self.events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)));
            if seen.iter().filter(|event| reached(event)).count() >= count { return; }
            drop(seen);
            assert!(std::time::Instant::now() < deadline, "timed out waiting for {what}: {:?}", self.device.calls());
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
        (
            "mixed boards, receive",
            vec!["0:A:ubx:0", "0:B:basic_rx:0"],
            vec!["0:A:ubx:0"],
            vec![],
            vec!["UBX", "BasicRX"],
        ),
        (
            "mixed boards, transmit",
            vec!["0:A:ubx:0"],
            vec!["0:A:ubx:0", "0:B:lf_tx:0"],
            vec![("radio.tx.c0.stream", eq(Scalar::from(0)))],
            vec!["UBX", "LFTX"],
        ),
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
    // `arm` on its lattice, and a channel leaving cuts it and starts the next one a
    // transmit lead later. The arm instants differ, so each provider's own lattice formula
    // is checked (`origin == lattice(arm instant, ratio)`, `origin % 200 == 0`) and the
    // cuts are compared as the shared rule's difference from the cut.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
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
    // The channel leaves: both cut the transmit clock at the first sample at or after the
    // instant, and both owe the next one a transmit lead. The instant is pushed first and
    // the clock stepped to just before it, so that the change is booked at its request.
    let at = ms(2_000) + ms(1) + ms(100);
    mock.update("radio.tx.c0.stream", Value::from(-1), Cold, Some(at));
    mock.step(at - ms(10));
    mock.step(at + ms(1));
    let by_node = |records: &[ClockRow]| -> ClockRow {
        records.iter().find(|record| record.0 == "radio/tx/s0").expect("the transmit clock").clone()
    };
    let mock_tx = by_node(&mock.records());
    assert_eq!(mock_tx.3, Some(at), "the Mock cuts the clock at the first sample at or after the instant");
    // The UHD runs too: its control thread takes the update, and uhd-tx switches at the cut.
    let uhd_t0 = uhd.time.now(uhd.root).unwrap().ticks_in(uhd.root).unwrap() + ms(2_000) + ms(1);
    uhd.start(uhd_t0).unwrap();
    uhd.wait_until(uhd_t0 + ms(1));
    let uhd_at = uhd.time.now(uhd.root).unwrap().ticks_in(uhd.root).unwrap() + ms(100);
    uhd.update("radio.tx.c0.stream", Value::from(-1), Cold, Some(uhd_at));
    uhd.wait_until(uhd_at + ms(60));
    uhd.settle(Wall::from_millis(20));
    let uhd_tx = by_node(&uhd.records());
    assert!(uhd_tx.3.is_some_and(|cut| cut >= uhd_at && cut - uhd_at < 200), "the UHD's cut: {:?} vs {uhd_at}", uhd_tx.3);
}

#[test]
fn vh_8_the_same_receive_membership_join_leaves_the_same_clocks() {
    // Design 33 §6, item 10: a channel joining stream 0 cuts the running segment and starts
    // the next one after the receive call, the allowance and the lead — the same nodes,
    // ratios and ready gap on both Providers. The two instants differ, so the gap and the
    // lattice are compared, not the absolute times.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let constraints: [(&str, Constraint); 0] = [];
    let selector = selector(&["0:A:ubx:0", "0:B:ubx:0"], &[]);
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
    let mock_first = mock.records();
    let uhd_first = uhd.records();
    assert_eq!(mock_first.len(), 1, "{mock_first:?}");
    assert_eq!(uhd_first.len(), 1, "{uhd_first:?}");
    assert_eq!(mock_first[0].0, uhd_first[0].0, "the stream node");
    assert_eq!(mock_first[0].1, uhd_first[0].1, "the ratio");
    // The join, 100 ms in on each provider's own axis: pushed first, and the clock stepped
    // to just before its instant so that the change is booked at its request.
    let mock_at = t0 + ms(100);
    mock.update("radio.rx.c1.stream", Value::from(0), Cold, Some(mock_at));
    mock.step(mock_at - ms(10));
    mock.step(mock_at + ms(200));
    let uhd_at = uhd_t0 + ms(100);
    uhd.update("radio.rx.c1.stream", Value::from(0), Cold, Some(uhd_at));
    uhd.wait_until(uhd_at + ms(200));
    uhd.settle(Wall::from_millis(20));
    let mock_records = mock.records();
    let uhd_records = uhd.records();
    assert_eq!(mock_records.len(), 2, "{mock_records:?}");
    assert_eq!(uhd_records.len(), 2, "{uhd_records:?}");
    assert_eq!((mock_records[0].0.as_str(), mock_records[1].0.as_str()), (uhd_records[0].0.as_str(), uhd_records[1].0.as_str()), "the stream nodes");
    assert_eq!((mock_records[0].1, mock_records[1].1), (uhd_records[0].1, uhd_records[1].1), "the ratios");
    let mock_gap = mock_records[1].2 - mock_records[0].3.expect("the Mock cuts the first segment");
    let uhd_gap = uhd_records[1].2 - uhd_records[0].3.expect("the UHD cuts the first segment");
    for gap in [mock_gap, uhd_gap] {
        assert!(gap >= ms(55) && gap < ms(55) + 200, "the ready gap: the receive call, the allowance and the lead (RM-25): {gap}");
    }
    assert!((mock_gap - uhd_gap).abs() < 200, "the gaps differ by more than the lattice: {mock_gap} vs {uhd_gap}");
    let rejected: Vec<_> = mock.events().iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).cloned().collect();
    assert!(rejected.is_empty(), "{rejected:?}");
}

#[test]
fn vh_8_the_same_floor_at_t0() {
    // Design 33 §6, item 11: a layout `cold` change asked before T0 is floored at T0 on
    // both Providers — the receive start of the instance, not of a line — and both report
    // it as a late command with the same requested and applied instants.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let constraints: [(&str, Constraint); 0] = [];
    let selector = selector(&["0:A:ubx:0"], &[]);
    let mut mock = MockRig::new(selector.clone(), &constraints);
    let mut uhd = UhdRig::new(selector, &constraints, FakeConfig::default()).unwrap();
    let t0 = ms(2_000) + ms(1);
    mock.arm_start(t0);
    uhd.arm().unwrap();
    let uhd_t0 = uhd.time.now(uhd.root).unwrap().ticks_in(uhd.root).unwrap() + ms(2_000) + ms(1);
    uhd.start(uhd_t0).unwrap();
    // Asked one tick before T0: pushed before the clock reaches it.
    mock.update("radio.rx.s0.sample_rate_hz", Value::num(2_000_000.0).unwrap(), Cold, Some(t0 - 1));
    mock.step(1);
    uhd.update("radio.rx.s0.sample_rate_hz", Value::num(2_000_000.0).unwrap(), Cold, Some(uhd_t0 - 1));
    uhd.wait_until(uhd_t0 + ms(150));
    uhd.settle(Wall::from_millis(10));
    let late_of = |events: &[ezsdr_kernel::event::Event]| -> Vec<(i64, i64)> {
        events
            .iter()
            .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::LATE_COMMAND)
            .map(|event| (event.payload["requested"]["ticks"].as_i64().unwrap(), event.payload["applied"]["ticks"].as_i64().unwrap()))
            .collect()
    };
    let mock_late = late_of(&mock.events());
    let uhd_late = late_of(&uhd.events());
    assert_eq!(mock_late.len(), 1, "{mock_late:?}");
    assert_eq!(uhd_late.len(), 1, "{uhd_late:?}");
    assert_eq!(mock_late, [(t0 - 1, t0)], "the Mock's floor is T0");
    assert_eq!(uhd_late, [(uhd_t0 - 1, uhd_t0)], "the UHD's floor is T0 too");
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
    uhd.actions.actions.lock().unwrap().push_back(start("rx/s1"));
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
    uhd.actions.actions.lock().unwrap().push_back(burst("tx/s1"));
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

/// The logical updates a Manifest's `applied` section holds, as `(key, value, instant)`
/// relative to `t0`: a row *without* `channel` — a hardware configure row, one per channel,
/// is not an update — and without `cancelled` (a held command `Provider::stop` dropped) or
/// `note` (a command recorded with no channel to act on). MockRadio records one as
/// `{ key, value, at }`; uhd-control as `{ key, claimed, …, at }`, its `issued` timed row
/// included, and the value-only row of a change that took its value alone. Both record the
/// instant the update took effect (`at`), so subtracting `t0` makes the two rigs' rows
/// comparable; MockRadio's per-instant apply and uhd-control's release both write it.
fn updates(applied: &serde_json::Value, t0: i64) -> Vec<(String, serde_json::Value, i64)> {
    // In instant order, then by key, so that the two sections' recording orders agree.
    let mut rows: Vec<(String, serde_json::Value, i64)> = applied
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            if row.get("channel").is_some() || row.get("cancelled").is_some() || row.get("note").is_some() {
                return None;
            }
            let key = row["key"].as_str()?.to_owned();
            let value = row.get("claimed").or_else(|| row.get("value"))?.clone();
            let at = row["at"]["ticks"].as_i64()? - t0;
            Some((key, value, at))
        })
        .collect();
    rows.sort_by(|a, b| (a.2, &a.0).cmp(&(b.2, &b.0)));
    rows
}

/// Every `LATE_COMMAND` as `(requested, applied)` relative to `t0` (RM-25, UC-2).
fn late_rows(events: &[ezsdr_kernel::event::Event], t0: i64) -> Vec<(i64, i64)> {
    events
        .iter()
        .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::LATE_COMMAND)
        .map(|event| (event.payload["requested"]["ticks"].as_i64().unwrap() - t0, event.payload["applied"]["ticks"].as_i64().unwrap() - t0))
        .collect()
}

/// Every `COMMAND_REJECTED` reason, in order (RM-22).
fn rejected_reasons(events: &[ezsdr_kernel::event::Event]) -> Vec<String> {
    events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).map(|event| event.payload["reason"].as_str().unwrap().to_owned()).collect()
}

#[test]
fn vh_8_the_same_floor_after_a_transmit_change() {
    // Design 33 §6, item 11's (b-4): a future transmit `cold` change floors a receive `cold`
    // change asked behind it — the instance's one floor, across the directions (RM-25) — and
    // both Providers raise the receive change to the transmit change's instant, report the
    // same `LATE_COMMAND` and cut both directions' clocks at the first sample at or after it.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    // A transmit channel in stream 0, so that the direction has a clock to cut.
    let constraints = [("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let selector = selector(&["0:A:ubx:0"], &["0:A:ubx:0"]);
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
    // The transmit rate change 50 ms ahead, then the receive rate change asked 10 ms ahead,
    // behind the first and raised to its instant; the clock stepped past the bookings and
    // to just before each instant, so that both are booked at their request.
    mock.update("radio.tx.s0.sample_rate_hz", Value::num(2e6).unwrap(), Cold, Some(t0 + ms(50)));
    mock.step(t0 + ms(20));
    mock.update("radio.rx.s0.sample_rate_hz", Value::num(2e6).unwrap(), Cold, Some(t0 + ms(10)));
    mock.step(t0 + ms(30));
    mock.step(t0 + ms(300));
    uhd.update("radio.tx.s0.sample_rate_hz", Value::num(2e6).unwrap(), Cold, Some(uhd_t0 + ms(50)));
    uhd.wait_until(uhd_t0 + ms(20));
    uhd.update("radio.rx.s0.sample_rate_hz", Value::num(2e6).unwrap(), Cold, Some(uhd_t0 + ms(10)));
    uhd.wait_until(uhd_t0 + ms(300));
    uhd.settle(Wall::from_millis(20));
    assert_eq!(late_rows(&mock.events(), t0), [(ms(10), ms(50))], "the Mock's floor is the pending transmit change");
    assert_eq!(late_rows(&uhd.events(), uhd_t0), [(ms(10), ms(50))], "the UHD's floor is the instance's");
    let cut_of = |records: &[ClockRow], node: &str, t0: i64, all: &[ClockRow]| -> i64 {
        records.iter().filter(|record| record.0 == node && record.3.is_some()).min_by_key(|record| record.2).map(|record| record.3.unwrap())
            .unwrap_or_else(|| panic!("the first clock of {node} is cut: {all:?}")) - t0
    };
    let mock_records = mock.records();
    let uhd_records = uhd.records();
    for (name, records, origin) in [("mock", &mock_records, t0), ("uhd", &uhd_records, uhd_t0)] {
        assert_eq!(records.iter().filter(|record| record.0 == "radio/rx/s0").count(), 2, "{name}: the receive change started a new segment: {records:?}");
        assert_eq!(records.iter().filter(|record| record.0 == "radio/tx/s0").count(), 2, "{name}: the transmit change started a new one: {records:?}");
        let rx = cut_of(records, "radio/rx/s0", origin, records);
        let tx = cut_of(records, "radio/tx/s0", origin, records);
        assert!(rx >= ms(50) && rx < ms(50) + 200, "{name}: the receive clock's cut {rx}");
        assert!(tx >= ms(50) && tx < ms(50) + 200, "{name}: the transmit clock's cut {tx}");
    }
    // The cut ends the 1 Msps clock (200 ticks a sample) at the first sample at or after the
    // instant, on each Provider's own grid. The grids need not coincide (the runs so far
    // differed by 28 and by 159 ticks), so the cuts may differ by up to one such sample, no
    // more: both lie in the window asserted above.
    for node in ["radio/rx/s0", "radio/tx/s0"] {
        let (mock_cut, uhd_cut) = (cut_of(&mock_records, node, t0, &mock_records), cut_of(&uhd_records, node, uhd_t0, &uhd_records));
        assert!(mock_cut.abs_diff(uhd_cut) < 200, "{node}: the cuts are {mock_cut} and {uhd_cut}");
    }
}

#[test]
fn vh_8_a_floored_refusal_is_reported_in_the_same_order() {
    // Design 33 §6, item 11 (Sol 4): a receive change that a future transmit change floors,
    // and that the envelope then refuses — RM-31: a receive channel of a full-duplex board
    // must use `RX2` while its transmit channel is in a stream — is reported in the same
    // order by both Providers: `LATE_COMMAND` for the raised instant, then the refusal.
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let constraints = [("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let selector = selector(&["0:A:ubx:0", "0:B:ubx:0"], &["0:A:ubx:0"]);
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
    mock.update("radio.tx.s0.sample_rate_hz", Value::num(2e6).unwrap(), Cold, Some(t0 + ms(50)));
    mock.step(t0 + ms(20));
    mock.update("radio.rx.c0.antenna", Value::from("TX/RX"), Cold, Some(t0 + ms(10)));
    mock.step(t0 + ms(30));
    mock.step(t0 + ms(100));
    uhd.update("radio.tx.s0.sample_rate_hz", Value::num(2e6).unwrap(), Cold, Some(uhd_t0 + ms(50)));
    uhd.wait_until(uhd_t0 + ms(20));
    uhd.update("radio.rx.c0.antenna", Value::from("TX/RX"), Cold, Some(uhd_t0 + ms(10)));
    uhd.wait_until(uhd_t0 + ms(100));
    uhd.settle(Wall::from_millis(20));
    let reported = |events: &[ezsdr_kernel::event::Event], t0: i64| -> Reported {
        events
            .iter()
            .filter_map(|event| match event.kind.as_str() {
                ezsdr_radio::kinds::LATE_COMMAND => Some(("late".to_owned(), Some((event.payload["requested"]["ticks"].as_i64()? - t0, event.payload["applied"]["ticks"].as_i64()? - t0)), None)),
                ezsdr_radio::kinds::COMMAND_REJECTED => {
                    // MockRadio's update refusals carry its own rule's label (MR-18); UHD names
                    // RM-31 itself. The rule and its text are the shared part.
                    let reason = event.payload["reason"].as_str()?.trim_start_matches("MR-18: ").to_owned();
                    reason.starts_with("RM-31:").then(|| ("refused".to_owned(), None, Some(reason)))
                }
                _ => None,
            })
            .collect()
    };
    let mock_reported = reported(&mock.events(), t0);
    let uhd_reported = reported(&uhd.events(), uhd_t0);
    assert_eq!(mock_reported.len(), 2, "{mock_reported:?}");
    assert_eq!(mock_reported, uhd_reported, "the same order, instants and reasons");
    assert_eq!(mock_reported[0].0, "late", "{mock_reported:?}");
    assert_eq!(mock_reported[0].1, Some((ms(10), ms(50))), "{mock_reported:?}");
    assert_eq!(mock_reported[1].0, "refused", "{mock_reported:?}");
}

#[test]
fn vh_8_the_same_logical_updates() {
    // Design 33 §6, item 15's (b-2): one timed channel update and one membership change that
    // takes its value alone leave the same logical updates — `(key, value, effective
    // instant)` — in each Manifest's `applied` section, as `updates` reads them; a
    // per-channel hardware configure row is not an update and is left out.
    use ezsdr_kernel::module_api::UpdateClass::{Cold, HardwareTimed};
    let constraints: [(&str, Constraint); 0] = [];
    let selector = selector(&["0:A:ubx:0", "0:B:ubx:0"], &["0:A:ubx:0"]);
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
    mock.update("radio.rx.c0.gain_db", Value::num(3.0).unwrap(), HardwareTimed, Some(t0 + ms(100)));
    mock.step(t0 + ms(20));
    mock.update("radio.rx.c1.stream", Value::from(-1), Cold, Some(t0 + ms(150)));
    mock.step(t0 + ms(30));
    mock.step(t0 + ms(350));
    uhd.update("radio.rx.c0.gain_db", Value::num(3.0).unwrap(), HardwareTimed, Some(uhd_t0 + ms(100)));
    uhd.update("radio.rx.c1.stream", Value::from(-1), Cold, Some(uhd_t0 + ms(150)));
    uhd.wait_until(uhd_t0 + ms(350));
    uhd.settle(Wall::from_millis(20));
    let mock_applied = &mock.mock.instance().sections[&ezsdr_kernel::spec::Namespace::parse("ezsdr.radio.mock.applied").unwrap()];
    let instance = uhd.finish();
    let uhd_applied = &instance.sections[&ezsdr_kernel::spec::Namespace::parse("ezsdr.radio.uhd.applied").unwrap()];
    let mock_updates = updates(mock_applied, t0);
    let uhd_updates = updates(uhd_applied, uhd_t0);
    assert_eq!(mock_updates.len(), 2, "{mock_updates:?}");
    assert_eq!(mock_updates, uhd_updates, "the same logical updates");
    assert_eq!(mock_updates[0].0, "radio.rx.c0.gain_db", "{mock_updates:?}");
    assert_eq!(mock_updates[0].2, ms(100), "{mock_updates:?}");
    assert_eq!(mock_updates[1].0, "radio.rx.c1.stream", "{mock_updates:?}");
    assert_eq!(mock_updates[1].1, json!(-1), "{mock_updates:?}");
}

// ---------------------------------------------------------------- the stream dimension

/// The clock rows of both rigs from each one's T0: `(node, ratio, origin, ended)` with the
/// two instants relative to that rig's own T0 (the two axes never meet).
type RelativeRow = (String, (u64, u64), i64, Option<i64>);

fn relative(records: &[ClockRow], t0: i64) -> Vec<RelativeRow> {
    records.iter().map(|(node, ratio, origin, ended)| (node.clone(), *ratio, origin - t0, ended.map(|end| end - t0))).collect()
}

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

/// Manifest `sequence` is instance-local; compare the rank of each distinct value.
fn ranked_segments(mut rows: Vec<SegmentRow>) -> Vec<SegmentRow> {
    let mut sequences: Vec<u64> = rows.iter().filter_map(|row| row.2).collect();
    sequences.sort();
    sequences.dedup();
    let ranks: BTreeMap<u64, u64> = sequences.into_iter().enumerate().map(|(rank, seq)| (seq, rank as u64)).collect();
    for row in &mut rows {
        row.2 = row.2.map(|seq| ranks[&seq]);
    }
    rows.sort();
    rows
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

    fn mock_rows(&self) -> Vec<RelativeRow> {
        relative(&self.mock.records(), self.t0)
    }

    fn uhd_rows(&self) -> Vec<RelativeRow> {
        relative(&self.uhd.records(), self.uhd_t0)
    }

    /// One `cold` update taken at relative `now` ms with its requested instant at relative
    /// `at` ms (none: the receipt), both Providers carried to relative `until` ms. The Mock
    /// takes it in the step at `now`; the UHD is brought to `now` and left there while its
    /// control thread takes the Action, so the instant it reads for the booking is `now`
    /// (within that thread's poll).
    fn cold(&mut self, name: &str, value: Value, now: i64, at: Option<i64>, until: i64) {
        use ezsdr_kernel::module_api::UpdateClass::Cold;
        self.mock.update(name, value.clone(), Cold, at.map(|at| self.t0 + ms(at)));
        self.mock.step(self.t0 + ms(now));
        self.uhd.wait_until(self.uhd_t0 + ms(now));
        self.uhd.update(name, value, Cold, at.map(|at| self.uhd_t0 + ms(at)));
        // The device clock stands at `now` while the control thread takes the Action: wait
        // for the queue to empty (the poll follows at once), then let that thread run.
        for _ in 0..1_000 {
            if self.uhd.actions.queued() == 0 { break; }
            // On the device's clock, which stops while the control thread runs (UR-33).
            self.uhd.settle(Wall::from_millis(1));
        }
        assert_eq!(self.uhd.actions.queued(), 0, "the UHD's control thread took the Action");
        self.uhd.settle(Wall::from_millis(2));
        self.mock.step(self.t0 + ms(until));
        self.uhd.wait_until(self.uhd_t0 + ms(until));
        self.uhd.settle(Wall::from_millis(20));
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
        serde_json::Value::Array(faults.iter().map(|(fault, at_ms)| json!({
            "at_ns": at_ms * 1_000_000,
            "fault": fault,
            "target": "radio",
        })).collect()),
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
    assert_eq!(rows.iter().map(|row| (row["applied"].as_bool().unwrap(), row["lost"].as_u64().unwrap())).collect::<Vec<_>>(),
        expected_lost.iter().map(|lost| (true, *lost)).collect::<Vec<_>>());
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
    compare_rx_overflow(
        &[("rx_overflow", 0)],
        vec![FakeFault::Overflow(Wall::ZERO)],
        60,
        RxOverflowCause::Overrun,
        BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED,
        &[50_000],
        true,
    );
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
        if pair.uhd.actions.queued() == 0 { break; }
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
    assert_eq!(uhd.sections[&Namespace::parse("ezsdr.radio.uhd.stats").unwrap()]["rx_overflows"], 1,
        "the UHD report was received before stop despite no block carrying it");
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
    let mut bytes = Vec::new();
    for sample in [[0.25f32, 0.0f32], [0.5f32, 0.0f32]] {
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

fn tx_burst_action(node: &str, waveform: &ezsdr_kernel::manifest::ArtifactRef, at: TimePoint) -> Action {
    Action::TxBurst {
        target: Target::Resource { resource: Ident::parse("radio").unwrap(), path: node.to_owned() },
        waveform: waveform.clone(),
        repeat: false,
        at: ezsdr_kernel::time::AbsoluteDeadline::new(at),
        requested_at: None,
        late_policy: ezsdr_kernel::stream::LatePolicy::SendAsapAndFlag,
        metadata: BTreeMap::new(),
    }
}

fn timer_barrier(root: ClockDomainId, at: i64, token: u64) -> Action {
    Action::SetTimer {
        target: Target::Resource { resource: Ident::parse("radio").unwrap(), path: String::new() },
        at: ezsdr_kernel::time::AbsoluteDeadline::new(TimePoint::new(root, at)),
        token,
    }
}

struct CancellationPair {
    mock: MockRig,
    uhd: UhdRig,
    waveform: ezsdr_kernel::manifest::ArtifactRef,
    t0: i64,
    uhd_t0: i64,
    uhd_barriers: std::cell::Cell<usize>,
}

impl CancellationPair {
    fn new(
        selector: BTreeMap<Ident, Value>,
        constraints: &[(&str, Constraint)],
        ports: &[&str],
        fake: FakeConfig,
        environment: BTreeMap<Namespace, serde_json::Value>,
    ) -> CancellationPair {
        let (waveform, inputs) = cancellation_waveform();
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
        CancellationPair { mock, uhd, waveform, t0, uhd_t0, uhd_barriers: std::cell::Cell::new(0) }
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
        self.uhd.wait_for_events("Action booking barrier", self.uhd_barriers.get(), |event| {
            event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED && event.payload["kind"] == "set_timer"
        });
    }

    fn cold_change(&self, stream: usize, rate: f64, relative_ms: i64) {
        use ezsdr_kernel::module_api::UpdateClass::Cold;
        self.mock.update(&format!("radio.tx.s{stream}.sample_rate_hz"), Value::num(rate).unwrap(), Cold, Some(self.t0 + ms(relative_ms)));
        self.uhd.update(&format!("radio.tx.s{stream}.sample_rate_hz"), Value::num(rate).unwrap(), Cold, Some(self.uhd_t0 + ms(relative_ms)));
    }
}

fn manifest_rows(instance: &ProviderInstance, namespace: &str) -> Vec<serde_json::Value> {
    instance.sections[&Namespace::parse(namespace).unwrap()].as_array().unwrap().clone()
}

fn tx_rejected_rows(instance: &ProviderInstance, namespace: &str) -> Vec<serde_json::Value> {
    manifest_rows(instance, namespace).into_iter().filter(|row| row["kind"] == "tx_burst").collect()
}

fn mock_tx_blocks(instance: &ProviderInstance) -> u64 {
    instance.sections[&Namespace::parse("ezsdr.radio.mock.stats").unwrap()]
        ["tx_blocks"].as_u64().unwrap()
}

fn tx_cancel_events(events: &[ezsdr_kernel::event::Event]) -> Vec<&ezsdr_kernel::event::Event> {
    events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED && event.payload["kind"] == "tx_burst").collect()
}

fn assert_mock_barriers(events: &[ezsdr_kernel::event::Event], expected: usize) {
    let barriers = events.iter().filter(|event| {
        event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED && event.payload["kind"] == "set_timer"
    }).count();
    assert_eq!(barriers, expected, "the Mock processed each queued-action barrier");
}

fn tx_send_calls(device: &FakeDevice, stream: usize) -> Vec<String> {
    device.calls().into_iter().filter(|call| call.starts_with(&format!("tx_send {stream} "))).collect()
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
    assert_eq!(
        mock.declared(),
        [
            ("radio/rx/s0".to_owned(), (100, 1)),
            ("radio/rx/s1".to_owned(), (200, 1)),
            ("radio/tx/s0".to_owned(), (100, 1)),
            ("radio/tx/s1".to_owned(), (200, 1)),
        ]
    );
}

#[test]
fn vh_8_a_stream_one_rate_change_cuts_that_stream_alone() {
    // Design 33 §6: stream 1's `cold` rate change cuts stream 1's clock and starts its next
    // segment a receive call, the allowance and the lead later; stream 0's clock is not cut.
    // Both Providers' rows agree, relative to their own T0 (RM-25, UR-25).
    let mut pair = Started::new(selector(&["0:A:ubx:0", "0:B:ubx:0"], &[]), &rx_two(), &["rx0", "rx1"]);
    pair.cold("radio.rx.s1.sample_rate_hz", Value::num(4e6).unwrap(), 10, Some(100), 400);
    let mock = pair.mock_rows();
    let uhd = pair.uhd_rows();
    assert_eq!(mock, uhd, "the clock rows of both streams, from each T0");
    assert_eq!(
        mock,
        [
            ("radio/rx/s0".to_owned(), (100, 1), 0, None),
            ("radio/rx/s1".to_owned(), (50, 1), ms(155), None),
            ("radio/rx/s1".to_owned(), (200, 1), 0, Some(ms(100))),
        ],
        "stream 0 is not cut; stream 1's new clock begins 55 ms after its cut"
    );
}

#[test]
fn vh_8_a_stream_one_join_leaves_the_same_ready_gap() {
    // Design 33 §6: a channel entering stream 1 from no stream cuts stream 1 and starts the
    // next segment after the receive call, the allowance and the lead (RM-25); both
    // Providers' gaps from the cut to the new origin agree.
    let constraints = [
        ("radio.rx.c0.stream", eq(Scalar::from(1))),
        ("radio.rx.c1.stream", eq(Scalar::from(-1))),
        ("radio.rx.s1.sample_rate_hz", eq(Scalar::try_from(1e6).unwrap())),
    ];
    let mut pair = Started::new(selector(&["0:A:ubx:0", "0:B:ubx:0"], &[]), &constraints, &["rx0", "rx1"]);
    pair.cold("radio.rx.c1.stream", Value::from(1), 10, Some(100), 400);
    let mock = pair.mock_rows();
    let uhd = pair.uhd_rows();
    assert_eq!(mock, uhd, "the clock rows of both streams, from each T0");
    assert_eq!(
        mock,
        [
            ("radio/rx/s1".to_owned(), (200, 1), 0, Some(ms(100))),
            ("radio/rx/s1".to_owned(), (200, 1), ms(155), None),
        ],
        "the join's gap is the 2 ms call, the 3 ms allowance and the 50 ms lead"
    );
}

#[test]
fn vh_8_a_channel_less_stream_one_starts_the_same_first_segment() {
    // Design 33 §6, D4: stream 1 has its node and port and no channel, so no clock; the
    // first channel to enter it starts its first segment at the effective instant, both
    // Providers alike (RM-21, MR-39, UR-25).
    let constraints = [
        ("radio.rx.c1.stream", eq(Scalar::from(-1))),
        ("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(2e6).unwrap())),
        ("radio.rx.s1.sample_rate_hz", eq(Scalar::try_from(1e6).unwrap())),
    ];
    let mut pair = Started::new(selector(&["0:A:ubx:0", "0:B:ubx:0"], &[]), &constraints, &["rx0", "rx1"]);
    assert!(pair.mock_rows().iter().all(|row| row.0 != "radio/rx/s1"), "no clock for a channel-less stream");
    assert!(pair.uhd_rows().iter().all(|row| row.0 != "radio/rx/s1"), "no clock on either");
    // The effective instant is well past the UHD's opening and configuring of the stream,
    // so the first segment begins at the instant itself on both.
    pair.cold("radio.rx.c1.stream", Value::from(1), 10, Some(200), 400);
    let mock = pair.mock_rows();
    let uhd = pair.uhd_rows();
    assert_eq!(mock, uhd, "the first segment's clock, from each T0");
    assert_eq!(
        mock,
        [
            ("radio/rx/s0".to_owned(), (100, 1), 0, None),
            ("radio/rx/s1".to_owned(), (200, 1), ms(200), None),
        ],
        "the first channel's segment begins at the effective instant"
    );
}

#[test]
fn vh_8_a_transmit_stream_one_has_the_same_clock() {
    // Design 33 §6, D4: a transmit channel enabled in stream 1 registers that stream's
    // clock on its own node and ratio, beginning at the change's effective instant — no
    // previous cut to wait for (RM-21, RM-25); stream 0 has no channel and no clock.
    let constraints: [(&str, Constraint); 0] = [];
    let selector = selector(&["0:A:ubx:0"], &["0:A:ubx:0", "0:B:ubx:0"]);
    let mut pair = Started::new(selector, &constraints, &["rx0"]);
    assert!(pair.mock_rows().iter().all(|row| !row.0.starts_with("radio/tx/")), "no transmit clock before a channel");
    assert!(pair.uhd_rows().iter().all(|row| !row.0.starts_with("radio/tx/")), "none on either");
    // Far enough ahead that the stream's opening and configuring ends before the instant.
    pair.cold("radio.tx.c1.stream", Value::from(1), 10, Some(200), 400);
    let mock = pair.mock_rows();
    let uhd = pair.uhd_rows();
    assert_eq!(mock, uhd, "the clocks, from each T0");
    assert_eq!(
        mock,
        [
            ("radio/rx/s0".to_owned(), (200, 1), 0, None),
            ("radio/tx/s1".to_owned(), (200, 1), ms(200), None),
        ],
        "stream 1's clock begins at the effective instant"
    );
}

#[test]
fn vh_8_the_instance_floor_crosses_streams() {
    // Design 33 §6, RM-25: stream 0's pending `cold` change floors stream 1's change into
    // it — one floor for the instance — and both report the same raised instant as
    // `LATE_COMMAND`; each stream is cut at that instant.
    let mut pair = Started::new(selector(&["0:A:ubx:0", "0:B:ubx:0"], &[]), &rx_two(), &["rx0", "rx1"]);
    pair.cold("radio.rx.s0.sample_rate_hz", Value::num(4e6).unwrap(), 10, Some(150), 20);
    pair.cold("radio.rx.s1.sample_rate_hz", Value::num(2e6).unwrap(), 20, Some(100), 500);
    assert_eq!(late_rows(&pair.mock.events(), pair.t0), [(ms(100), ms(150))], "the Mock's floor is stream 0's change");
    assert_eq!(late_rows(&pair.uhd.events(), pair.uhd_t0), [(ms(100), ms(150))], "the UHD's floor is the instance's");
    let mock = pair.mock_rows();
    let uhd = pair.uhd_rows();
    assert_eq!(mock, uhd, "both streams' rows, from each T0");
    // Stream 0 (2 Msps: 1 ms call) begins 54 ms after the floor; stream 1 (1 Msps: 2 ms)
    // 55 ms after it.
    assert_eq!(
        mock,
        [
            ("radio/rx/s0".to_owned(), (50, 1), ms(204), None),
            ("radio/rx/s0".to_owned(), (100, 1), 0, Some(ms(150))),
            ("radio/rx/s1".to_owned(), (100, 1), ms(205), None),
            ("radio/rx/s1".to_owned(), (200, 1), 0, Some(ms(150))),
        ]
    );
}

#[test]
fn vh_8_a_stream_one_refusal_leaves_stream_zero_alone() {
    // Design 33 §6, RM-31: a stream-1-only antenna change the full-duplex rule refuses —
    // receive c0 and transmit c0 are the same board, and c0 is in stream 1 — is refused by
    // both with the same key and rule, and cuts neither stream's clock.
    let constraints = [
        ("radio.rx.c0.stream", eq(Scalar::from(1))),
        ("radio.rx.c1.stream", eq(Scalar::from(0))),
        ("radio.tx.c0.stream", eq(Scalar::from(1))),
    ];
    let selector = selector(&["0:A:ubx:0", "0:B:ubx:0"], &["0:A:ubx:0", "0:B:ubx:0"]);
    let mut pair = Started::new(selector, &constraints, &["rx0", "rx1"]);
    let before = pair.mock_rows();
    let before_uhd = pair.uhd_rows();
    pair.cold("radio.rx.c0.antenna", Value::from("TX/RX"), 10, Some(100), 400);
    let mock_rejected = rejected_reasons(&pair.mock.events());
    let uhd_rejected = rejected_reasons(&pair.uhd.events());
    assert_eq!(mock_rejected.len(), 1, "{mock_rejected:?}");
    assert_eq!(uhd_rejected.len(), 1, "{uhd_rejected:?}");
    // The Mock's update refusal carries MR-18; the shared rule and its text follow it.
    assert_eq!(mock_rejected[0].trim_start_matches("MR-18: "), uhd_rejected[0], "the same rule and reason");
    assert!(uhd_rejected[0].starts_with("RM-31:"), "{uhd_rejected:?}");
    // The refused key itself: each description refuses its own instance's projected
    // configuration on the same key (the event names the Action alone, RM-22).
    assert_eq!(pair.mock.effective, pair.uhd.effective, "the same configuration");
    let request_of = |effective: &BTreeMap<Key, Value>| -> Requested {
        let mut constraints: BTreeMap<Key, Constraint> = effective
            .iter()
            .filter_map(|(key, value)| Some((key.clone(), Constraint::Eq { value: value.as_scalar()?.clone() })))
            .collect();
        constraints.insert(key("radio.rx.c0.antenna"), Constraint::Eq { value: Scalar::from("TX/RX") });
        Requested { resource: rid("radio"), constraints }
    };
    let keys_of = |report: &CoerceReport| -> Vec<(Key, String)> { report.rejected.iter().map(|rejection| (rejection.key.clone(), rejection.reason.clone())).collect() };
    assert_eq!(
        keys_of(&pair.mock.mock.coerce(&request_of(&pair.mock.effective)).unwrap()),
        keys_of(&pair.uhd.radio.coerce(&request_of(&pair.uhd.effective)).unwrap()),
        "the same key and reason"
    );
    assert_eq!(pair.mock_rows(), before, "the Mock's clocks are not cut");
    assert_eq!(pair.uhd_rows(), before_uhd, "the UHD's clocks are not cut");
}

/// The plan comparison's run: two receive streams, a stream-1 rate change, a transmit
/// channel entering stream 1 and a transmit rate change, both stopped at 700 ms. The rows
/// are the Mock's and the UHD's, from each one's T0.
fn planned() -> (Vec<SegmentRow>, Vec<SegmentRow>) {
    let constraints = rx_two();
    let selector = selector(&["0:A:ubx:0", "0:B:ubx:0"], &["0:A:ubx:0", "0:B:ubx:0"]);
    let mut pair = Started::new(selector, &constraints, &["rx0", "rx1"]);
    pair.cold("radio.rx.s1.sample_rate_hz", Value::num(4e6).unwrap(), 10, Some(100), 150);
    // Far enough ahead that the UHD's configuration of stream 1 ends before the instant.
    pair.cold("radio.tx.c1.stream", Value::from(1), 150, Some(300), 350);
    pair.cold("radio.tx.s1.sample_rate_hz", Value::num(2e6).unwrap(), 350, Some(500), 650);
    let (t0, uhd_t0) = (pair.t0, pair.uhd_t0);
    let (mock, uhd) = pair.finish(700);
    (mock_segments(&mock, t0), uhd_plan(&uhd, uhd_t0))
}

#[test]
fn vh_8_the_same_manifest_plan_rows() {
    // Design 33 §6, D3's 6: after `stop`, the UHD's `timing` plan and the Mock's `segments`
    // name each stream's segments alike — the stream, its origin from T0 and its logical
    // channels (MR-27, UR-30). The command that began each segment is `sequence`'s own
    // comparison below.
    let (mock_rows, uhd_rows) = planned();
    let without_sequence = |rows: Vec<SegmentRow>| -> Vec<(String, u64, i64, Vec<u64>)> {
        rows.into_iter().map(|(direction, stream, _, origin, channels)| (direction, stream, origin, channels)).collect()
    };
    let mock = without_sequence(mock_rows);
    let uhd = without_sequence(uhd_rows);
    assert_eq!(mock, uhd, "the segments of every stream, from each T0");
    assert_eq!(
        mock,
        [
            ("rx".to_owned(), 0, 0, vec![0]),
            ("rx".to_owned(), 1, 0, vec![1]),
            ("rx".to_owned(), 1, ms(155), vec![1]),
            ("tx".to_owned(), 1, ms(300), vec![1]),
            ("tx".to_owned(), 1, ms(550), vec![1]),
        ]
    );
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
    assert_eq!(ranks(&mock_rows), [None, None, Some(0), Some(1), Some(2)], "{mock_rows:?}");
}

#[test]
fn vh_8_a_move_between_streams_leaves_the_same_clocks_segments_and_update() {
    // S11/A10: stream 0 retains c0 while c1 moves into s1; compare plan sequence by rank and
    // compare the one logical move row in `applied` as well as clocks and plan.
    let selector = selector(&["0:A:ubx:0", "0:B:basic_rx:0", "0:B:basic_rx:1"], &[]);
    let constraints = [
        ("radio.rx.c0.stream", eq(Scalar::from(0))),
        ("radio.rx.c1.stream", eq(Scalar::from(0))),
        ("radio.rx.c2.stream", eq(Scalar::from(1))),
        ("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(2e6).unwrap())),
        ("radio.rx.s1.sample_rate_hz", eq(Scalar::try_from(1e6).unwrap())),
        ("radio.rx.s2.sample_rate_hz", eq(Scalar::try_from(1e6).unwrap())),
    ];
    let mut pair = Started::with_fake_config(selector, &constraints, &["rx0", "rx1"], FakeConfig { front_ends: vec!["UBX", "BasicRX"], ..FakeConfig::default() });
    pair.cold("radio.rx.c1.stream", Value::from(1), 10, Some(100), 500);
    let mock_clocks = pair.mock_rows();
    let uhd_clocks = pair.uhd_rows();
    let (t0, uhd_t0) = (pair.t0, pair.uhd_t0);
    let (mock, uhd) = pair.finish(600);
    assert_eq!(mock_clocks, uhd_clocks, "the clock rows from each T0");
    let mock_segments = ranked_segments(mock_segments(&mock, t0));
    let uhd_segments = ranked_segments(uhd_plan(&uhd, uhd_t0));
    assert_eq!(mock_segments, uhd_segments, "the segment rows with sequence ranks");
    let mock_applied = &mock.sections[&Namespace::parse("ezsdr.radio.mock.applied").unwrap()];
    let uhd_applied = &uhd.sections[&Namespace::parse("ezsdr.radio.uhd.applied").unwrap()];
    let mock_updates = updates(mock_applied, t0);
    let uhd_updates = updates(uhd_applied, uhd_t0);
    assert_eq!(mock_updates, uhd_updates, "the applied logical move row");
    assert_eq!(mock_updates, [("radio.rx.c1.stream".to_owned(), json!(1), ms(100))]);
}

#[test]
fn vh_8_a_move_into_an_empty_stream_leaves_the_same_first_segment_and_update() {
    // S3/S11/A10: an empty destination stays unopened by uhd-control at booking; its first
    // segment, plan sequence rank and logical move row match the Mock's.
    let selector = selector(&["0:A:ubx:0", "0:B:basic_rx:0", "0:B:basic_rx:1"], &[]);
    let constraints = [
        ("radio.rx.c0.stream", eq(Scalar::from(0))),
        ("radio.rx.c1.stream", eq(Scalar::from(1))),
        ("radio.rx.c2.stream", eq(Scalar::from(-1))),
        ("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(2e6).unwrap())),
        ("radio.rx.s1.sample_rate_hz", eq(Scalar::try_from(1e6).unwrap())),
        ("radio.rx.s2.sample_rate_hz", eq(Scalar::try_from(1e6).unwrap())),
    ];
    let mut pair = Started::with_fake_config(selector, &constraints, &["rx0", "rx1", "rx2"], FakeConfig { front_ends: vec!["UBX", "BasicRX"], ..FakeConfig::default() });
    pair.cold("radio.rx.c1.stream", Value::from(2), 10, Some(100), 500);
    let mock_clocks = pair.mock_rows();
    let uhd_clocks = pair.uhd_rows();
    let (t0, uhd_t0) = (pair.t0, pair.uhd_t0);
    let (mock, uhd) = pair.finish(600);
    assert_eq!(mock_clocks, uhd_clocks, "the clock rows from each T0");
    assert_eq!(ranked_segments(mock_segments(&mock, t0)), ranked_segments(uhd_plan(&uhd, uhd_t0)), "the segment rows with sequence ranks");
    let mock_applied = &mock.sections[&Namespace::parse("ezsdr.radio.mock.applied").unwrap()];
    let uhd_applied = &uhd.sections[&Namespace::parse("ezsdr.radio.uhd.applied").unwrap()];
    let mock_updates = updates(mock_applied, t0);
    let uhd_updates = updates(uhd_applied, uhd_t0);
    assert_eq!(mock_updates, uhd_updates, "the applied logical move row");
    assert_eq!(mock_updates, [("radio.rx.c1.stream".to_owned(), json!(2), ms(100))]);
}

#[test]
fn vh_8_tx_stream_stop_cancels_held_bursts() {
    let selector = selector(&["0:A:ubx:0"], &["0:A:ubx:0"]);
    let constraints = [
        ("radio.rx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
    ];
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
    let constraints = [
        ("radio.rx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
    ];
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
fn vh_8_cold_change_cancels_held_bursts() {
    let selector = selector(&["0:A:ubx:0"], &["0:A:ubx:0"]);
    let constraints = [
        ("radio.rx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
    ];
    let mut pair = CancellationPair::new(selector, &constraints, &["rx0"], FakeConfig::default(), BTreeMap::new());
    for relative in [110, 120] {
        pair.mock.push(pair.burst("tx/s0", 0, 0, relative));
        pair.uhd.push(pair.uhd_burst("tx/s0", 0, 0, relative));
    }
    pair.cold_change(0, 2e6, 50);
    pair.mock_barrier(104);
    pair.mock.step(pair.t0 + ms(20));
    pair.uhd_barrier(104);
    pair.mock.step(pair.t0 + ms(200));
    pair.uhd.wait_until(pair.uhd_t0 + ms(200));
    pair.uhd.wait_for_events("both cold-change cancellations", 2, |event| {
        event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED && event.payload["kind"] == "tx_burst"
    });
    let mock = pair.mock.finish_in_place();
    let mock_events = pair.mock.events();
    let uhd = pair.uhd.finish_in_place();
    let uhd_events = pair.uhd.events();
    let mock_rejected = tx_rejected_rows(&mock, "ezsdr.radio.mock.rejected");
    let uhd_rejected = tx_rejected_rows(&uhd, "ezsdr.radio.uhd.rejected");
    let mock_cancelled = tx_cancel_events(&mock_events);
    let uhd_cancelled = tx_cancel_events(&uhd_events);
    assert_eq!(mock_rejected.len(), 2, "{mock_rejected:?}");
    assert_eq!(uhd_rejected.len(), 2, "{uhd_rejected:?}");
    assert_eq!(mock_cancelled.len(), 2, "{mock_cancelled:?}");
    assert_eq!(uhd_cancelled.len(), 2, "{uhd_cancelled:?}");
    assert!(mock_rejected.iter().all(|row| row["reason"] == "MR-18: cancelled by a cold change"), "{mock_rejected:?}");
    assert!(uhd_rejected.iter().all(|row| row["reason"] == "cancelled by a cold change"), "{uhd_rejected:?}");
    assert!(mock_cancelled.iter().all(|event| event.payload["reason"] == "MR-18: cancelled by a cold change"), "{mock_cancelled:?}");
    assert!(uhd_cancelled.iter().all(|event| event.payload["reason"] == "cancelled by a cold change"), "{uhd_cancelled:?}");
    assert!(manifest_rows(&mock, "ezsdr.radio.mock.bursts").is_empty());
    assert_mock_barriers(&mock_events, 1);
    assert_eq!(mock_tx_blocks(&mock), 0);
    assert!(manifest_rows(&uhd, "ezsdr.radio.uhd.bursts").is_empty());
    assert!(tx_send_calls(&pair.uhd.device, 0).is_empty());
}

#[test]
fn vh_8_loss_cancels_held_bursts_without_rejection() {
    let selector = selector(&["0:A:ubx:0"], &["0:A:ubx:0"]);
    let constraints = [
        ("radio.rx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
    ];
    let environment = BTreeMap::from([(
        Namespace::parse("sim.faults").unwrap(),
        json!([{ "at_ns": 50_000_000, "fault": "device_lost", "target": "radio" }]),
    )]);
    let fake = FakeConfig {
        faults: vec![ezsdr_radio_uhd::FakeFault::Lost(Wall::from_millis(2_051))],
        ..FakeConfig::default()
    };
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
fn vh_8_cold_change_cancels_a_burst_on_a_skipped_future_clock() {
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let selector = selector(&["0:A:ubx:0"], &["0:A:ubx:0"]);
    let constraints = [
        ("radio.rx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
    ];
    let mut pair = CancellationPair::new(selector, &constraints, &["rx0"], FakeConfig::default(), BTreeMap::new());
    pair.mock.update("radio.tx.s0.sample_rate_hz", Value::num(2e6).unwrap(), Cold, Some(pair.t0 + ms(50)));
    pair.mock_barrier(106);
    pair.mock.step(pair.t0 + ms(20));
    pair.uhd.update("radio.tx.s0.sample_rate_hz", Value::num(2e6).unwrap(), Cold, Some(pair.uhd_t0 + ms(50)));
    pair.uhd_barrier(106);
    pair.mock.push(pair.burst("tx/s0", 0, 1, 150));
    pair.uhd.push(pair.uhd_burst("tx/s0", 0, 1, 150));
    pair.mock.update("radio.tx.s0.sample_rate_hz", Value::num(4e6).unwrap(), Cold, Some(pair.t0 + ms(80)));
    pair.mock_barrier(107);
    pair.mock.step(pair.t0 + ms(20));
    pair.uhd.update("radio.tx.s0.sample_rate_hz", Value::num(4e6).unwrap(), Cold, Some(pair.uhd_t0 + ms(80)));
    pair.uhd_barrier(107);
    pair.mock.step(pair.t0 + ms(200));
    pair.uhd.wait_until(pair.uhd_t0 + ms(200));
    pair.uhd.wait_for_event("skipped-clock cancellation", |event| {
        event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED && event.payload["reason"] == "UR-25: its transmit clock was never started"
    });
    let mock = pair.mock.finish_in_place();
    let mock_events = pair.mock.events();
    let uhd = pair.uhd.finish_in_place();
    let uhd_events = pair.uhd.events();
    let mock_rejected = tx_rejected_rows(&mock, "ezsdr.radio.mock.rejected");
    let uhd_rejected = tx_rejected_rows(&uhd, "ezsdr.radio.uhd.rejected");
    let mock_cancelled = tx_cancel_events(&mock_events);
    let uhd_cancelled = tx_cancel_events(&uhd_events);
    assert_eq!(mock_rejected.len(), 1, "{mock_rejected:?}");
    assert_eq!(uhd_rejected.len(), 1, "{uhd_rejected:?}");
    assert_eq!(mock_cancelled.len(), 1, "{mock_cancelled:?}");
    assert_eq!(uhd_cancelled.len(), 1, "{uhd_cancelled:?}");
    assert_eq!(mock_rejected[0]["reason"], "MR-18: cancelled by a cold change");
    assert_eq!(uhd_rejected[0]["reason"], "UR-25: its transmit clock was never started");
    assert_eq!(uhd_cancelled[0].payload["reason"], "UR-25: its transmit clock was never started");
    assert_mock_barriers(&mock_events, 2);
    assert!(manifest_rows(&mock, "ezsdr.radio.mock.bursts").is_empty());
    assert!(manifest_rows(&uhd, "ezsdr.radio.uhd.bursts").is_empty());
    assert_eq!(mock_tx_blocks(&mock), 0);
    assert!(tx_send_calls(&pair.uhd.device, 0).is_empty());
}

#[test]
fn vh_8_tx_stop_cancels_a_burst_on_a_future_clock() {
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    let selector = selector(&["0:A:ubx:0"], &["0:A:ubx:0"]);
    let constraints = [
        ("radio.rx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
    ];
    let mut pair = CancellationPair::new(selector, &constraints, &["rx0"], FakeConfig::default(), BTreeMap::new());
    pair.mock.update("radio.tx.s0.sample_rate_hz", Value::num(2e6).unwrap(), Cold, Some(pair.t0 + ms(50)));
    pair.mock_barrier(108);
    pair.mock.step(pair.t0 + ms(20));
    pair.uhd.update("radio.tx.s0.sample_rate_hz", Value::num(2e6).unwrap(), Cold, Some(pair.uhd_t0 + ms(50)));
    pair.uhd_barrier(108);
    pair.mock.push(pair.burst("tx/s0", 0, 1, 150));
    pair.mock.stop_at("tx/s0");
    pair.mock_barrier(109);
    pair.mock.step(pair.t0 + ms(20));
    pair.uhd.push(pair.uhd_burst("tx/s0", 0, 1, 150));
    pair.uhd.stop_at("tx/s0");
    pair.uhd_barrier(109);
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
    assert_mock_barriers(&mock_events, 2);
    assert!(tx_cancel_events(&uhd_events).is_empty());
    assert!(manifest_rows(&mock, "ezsdr.radio.mock.bursts").is_empty());
    assert!(manifest_rows(&uhd, "ezsdr.radio.uhd.bursts").is_empty());
    assert_eq!(mock_tx_blocks(&mock), 0);
    assert!(tx_send_calls(&pair.uhd.device, 0).is_empty());
}

#[test]
fn vh_8_rx_stop_does_not_cancel_held_tx() {
    let selector = selector(&["0:A:ubx:0"], &["0:A:ubx:0"]);
    let constraints = [
        ("radio.rx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
    ];
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
    let constraints = [
        ("radio.rx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
    ];
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
fn vh_8_tx_repeat_records_match_at_a_common_explicit_cut() {
    use ezsdr_kernel::module_api::UpdateClass::Cold;
    use ezsdr_kernel::stream::{BurstEnd, BurstRecord, LatePolicy};

    let samples: Vec<[f32; 2]> = (0..998).map(|i| [i as f32 / 2_000.0, -(i as f32) / 3_000.0]).collect();
    let bytes: Vec<u8> = samples.iter().flat_map(|sample| [sample[0].to_le_bytes(), sample[1].to_le_bytes()].concat()).collect();
    let hash = ContentHash::of_bytes(&bytes);
    let waveform = ezsdr_kernel::manifest::ingest_input(
        Ident::parse("waveform").unwrap(), Namespace::parse("ezsdr.input").unwrap(), format!("mem:{hash}"), &bytes,
    );
    let inputs = BTreeMap::from([(hash, Arc::<[u8]>::from(bytes))]);
    let mut selector = selector(&[], &["0:A:ubx:0"]);
    selector.insert(Ident::parse("link_gbps").unwrap(), Value::List(vec![Scalar::from(10)]));
    let constraints = [
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.s0.sample_rate_hz", eq(Scalar::try_from(200e6).unwrap())),
    ];
    let mut mock = MockRig::with_run(selector.clone(), &constraints, &[], inputs.clone(), BTreeMap::new());
    let mut uhd = UhdRig::with_run(selector, &constraints, FakeConfig::default(), &[], inputs).unwrap();
    let mock_t0 = ms(2_001);
    mock.arm_start(mock_t0);
    uhd.arm().unwrap();
    let uhd_now = uhd.time.now(uhd.root).unwrap().ticks_in(uhd.root).unwrap();
    let uhd_t0 = uhd_now + ms(2_001);
    uhd.start(uhd_t0).unwrap();

    let mock_target_root = mock_t0 + ms(100);
    let mock_cut_root = mock_target_root + 60_000;
    let uhd_target_root = uhd_t0 + ms(100);
    let uhd_cut_root = uhd_target_root + 60_000;
    let mock_target = tx_sample_time(&mock.clocks, 0, 0, mock_target_root);
    let uhd_target = tx_sample_time(&uhd.clocks, 0, 0, uhd_target_root);
    let burst = |at: TimePoint| Action::TxBurst {
        target: Target::Resource { resource: Ident::parse("radio").unwrap(), path: "tx/s0".to_owned() },
        waveform: waveform.clone(),
        repeat: true,
        at: ezsdr_kernel::time::AbsoluteDeadline::new(at),
        requested_at: None,
        late_policy: LatePolicy::DropAndFlag,
        metadata: BTreeMap::new(),
    };
    mock.push(burst(mock_target));
    mock.update("radio.tx.s0.sample_rate_hz", Value::num(100e6).unwrap(), Cold, Some(mock_cut_root));
    uhd.push(burst(uhd_target));
    uhd.update("radio.tx.s0.sample_rate_hz", Value::num(100e6).unwrap(), Cold, Some(uhd_cut_root));

    mock.step(mock_target_root - ms(5));
    uhd.wait_until(uhd_target_root - ms(5));
    uhd.settle(Wall::from_millis(10));
    mock.step(mock_cut_root + 1);
    uhd.wait_until(uhd_cut_root + 1);
    uhd.settle(Wall::from_millis(10));
    let uhd_calls = tx_send_calls(&uhd.device, 0);
    let uhd_payload_sizes: Vec<usize> = uhd_calls.iter()
        .filter(|call| call.ends_with("eob=false"))
        .filter_map(|call| call.split(" n=").nth(1)?.split_whitespace().next()?.parse().ok())
        .collect();
    let mock_clocks = mock.clocks.clone();
    let uhd_clocks = uhd.clocks.clone();
    let mock_events = mock.events();
    let mock = mock.finish();
    let uhd = uhd.finish();

    let records = |instance: &ProviderInstance, namespace: &str| -> Vec<BurstRecord> {
        serde_json::from_value(serde_json::Value::Array(manifest_rows(instance, namespace))).unwrap()
    };
    let mock_records = records(&mock, "ezsdr.radio.mock.bursts");
    let uhd_records = records(&uhd, "ezsdr.radio.uhd.bursts");
    assert!((3..=4).contains(&uhd_payload_sizes.len()), "the timed SOB may take only the available window: {uhd_calls:?}");
    assert!(uhd_payload_sizes.iter().all(|size| (1..=20_000).contains(size)), "each payload send respects the TX cap: {uhd_payload_sizes:?}");
    assert_eq!(uhd_payload_sizes.iter().sum::<usize>(), 60_000, "UHD payload ends at the common cut: {uhd_calls:?}, target {uhd_target:?}");
    assert!(uhd_calls[0].contains("sob=true eob=false"), "the first payload is the timed SOB: {uhd_calls:?}");
    assert!(uhd_payload_sizes[1..uhd_payload_sizes.len() - 1].iter().all(|size| *size == 20_000), "continuation buffers use the cap: {uhd_payload_sizes:?}");
    assert_eq!(mock_records.len(), 1, "{mock_records:?}; events: {mock_events:?}; rejected: {:?}", manifest_rows(&mock, "ezsdr.radio.mock.rejected"));
    assert_eq!(uhd_records.len(), 1, "{uhd_records:?}");
    let (mock_record, uhd_record) = (&mock_records[0], &uhd_records[0]);
    assert_eq!((mock_record.samples, mock_record.wraps, mock_record.end), (60_000, 60, BurstEnd::Stop));
    assert_eq!((uhd_record.samples, uhd_record.wraps, uhd_record.end), (60_000, 60, BurstEnd::Stop));
    assert_eq!(mock_record.requested_target, None);
    assert_eq!(uhd_record.requested_target, None);
    let root_tick = |record: &BurstRecord, clocks: &ClockRegistry| {
        let clock = clocks.sample_clock_records().into_iter().find(|clock| clock.domain == record.target.domain()).unwrap();
        let origin = clock.origin.ticks_in(clock.root).unwrap();
        let sample = record.target.ticks_in(record.target.domain()).unwrap();
        let ratio = clock.root_ticks_per_tick;
        origin + i64::try_from(i128::from(sample) * i128::from(ratio.num()) / i128::from(ratio.den())).unwrap()
    };
    assert_eq!(root_tick(mock_record, &mock_clocks) - mock_t0, ms(100));
    assert_eq!(root_tick(uhd_record, &uhd_clocks) - uhd_t0, ms(100));
    assert_eq!(mock_record.samples, uhd_record.samples, "same total handed samples at the explicit cut");
    assert_eq!(mock_tx_blocks(&mock), 61, "MR-15 keeps its provider-local simulation blocks");
    assert_eq!(mock_record.blocks, 61);
    assert_eq!(uhd_record.blocks, uhd_payload_sizes.len() as u32, "provider-local block accounting follows UHD's native sends");
}
