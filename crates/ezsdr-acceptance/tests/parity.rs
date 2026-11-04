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
        self.actions.actions.lock().unwrap().push_back(action);
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
        let pairs: Vec<_> = ["radio", "radio/rx/s0", "radio/tx/s0", "radio/rx/s1", "radio/tx/s1"]
            .iter()
            .flat_map(|source| kinds.kinds().into_iter().map(move |kind| (EventSource::Node { node: rid(source) }, kind)))
            .collect();
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
        let constraints = with_rx0(&radio, constraints);
        let prepared = radio.prepare(&fragment(ezsdr_radio_uhd::module_ref(), &constraints), ctx)?;
        Ok(UhdRig { radio, device, clocks, time, root, actions: queue, effective: prepared.effective, events, seen_events: Mutex::new(Vec::new()), rx_links, _authority: authority })
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
        let deadline = std::time::Instant::now() + Wall::from_secs(10);
        loop {
            let mut seen = self.seen_events.lock().unwrap();
            seen.extend(self.events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)));
            if seen.iter().filter(|event| reached(event)).count() >= count {
                return;
            }
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
    Action::SetTimer { target: Target::Resource { resource: Ident::parse("radio").unwrap(), path: String::new() }, at: ezsdr_kernel::time::AbsoluteDeadline::new(TimePoint::new(root, at)), token }
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
    fn new(selector: BTreeMap<Ident, Value>, constraints: &[(&str, Constraint)], ports: &[&str], fake: FakeConfig, environment: BTreeMap<Namespace, serde_json::Value>) -> CancellationPair {
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
        self.uhd.wait_for_events("Action booking barrier", self.uhd_barriers.get(), |event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED && event.payload["kind"] == "set_timer");
    }
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
