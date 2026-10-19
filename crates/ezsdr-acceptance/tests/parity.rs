//! VH-8 for the layout's keys (RM-27…RM-32, design 33 §6): the same selector and the same
//! Spec through MockRadio and through `UhdRadio` on a `FakeDevice`, compared directly —
//! `PrepareReport.effective`, the declared SampleClocks (node and ratio), the same refusals,
//! and a short run's clock records. PO-8: only this crate may pair two Modules, so the two
//! rigs are copies of `ezsdr-mock-radio`'s `mock_layout.rs` and `ezsdr-radio-uhd`'s
//! `fake.rs` harnesses, not shared code.
//!
//! What is not compared, by design: the device calls and read-backs (MockRadio has no
//! device), the UHD-only opening checks (the subdev spec, the serial, the rate set, the
//! front ends — UR-5, items 2–5) and the UHD-only refusals (UR-12's read-back, UR-21's
//! target kinds); block contents and physical instants beyond VH-8's own columns; and
//! anything of the stream dimension beyond stream 0 (1b-4d's and 1b-4e's work). Both
//! providers share `channels::Description`, so a shared mistake inside it is invisible
//! here: this file is a net for divergences, and the specs' own tests are the ground
//! truth.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration as Wall;

use ezsdr_kernel::binding::{AdmissionCheckRegistry, Binding, Violation};
use ezsdr_kernel::event::{
    Action, ActionId, EventCollector, EventSource, Target,
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
use ezsdr_kernel::spec::{Constraint, Ident, Key, Scalar, Value};
use ezsdr_kernel::stream::{BackPressure, BlockRef, DataLink, DropCarry, PublishOutcome};
use ezsdr_kernel::time::{
    ClockDomain, ClockRegistry, Duration, EpochRef, ManualTimeAuthority, Rational, TimePoint,
};
use ezsdr_mock_radio::MockRadio;
use ezsdr_radio_uhd::{Device, DeviceAuthority, FakeConfig, FakeDevice, UhdRadio};
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

/// The selector of a layout, with `keys: "channel"` (RM-38) and the shared values UHD's
/// `args` beside (design 33 D5).
fn selector(rx: &[&str], tx: &[&str]) -> BTreeMap<Ident, Value> {
    let list = |elements: &[&str]| Value::List(elements.iter().map(|e| Scalar::from(*e)).collect());
    BTreeMap::from([
        (Ident::parse("keys").unwrap(), Value::from("channel".to_owned())),
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
}

impl MockRig {
    fn new(selector: BTreeMap<Ident, Value>, constraints: &[(&str, Constraint)]) -> MockRig {
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
        let links = vec![AttachedPort::new(Ident::parse("radio").unwrap(), Ident::parse("rx0").unwrap(), Endpoint::StreamOut(Arc::new(MemLink::new())))];
        let ctx = {
            let mut ctx = PrepareContext::testing(EventSource::Node { node: rid("radio") }, time.clone(), clocks.clone(), events.clone(), actions.clone(), Arc::new(Accepting));
            ctx.run = RunId::from_string("vh8-mock".to_owned());
            ctx.class = ExecutionClass::Simulation;
            ctx.environment = Arc::new(BTreeMap::new());
            ctx.inputs = Arc::new(BTreeMap::<ContentHash, Arc<[u8]>>::new());
            ctx.links = links;
            ctx.host_budget = RelativeBudget::new(Duration::new(ClockDomainId::HOST_MONOTONIC, 5_000_000_000)).unwrap();
            ctx
        };
        let prepared = mock.prepare(&fragment(ModuleRef { id: ModuleId::parse("ezsdr.radio.mock").unwrap(), version: Version::new(2, 0, 0) }, constraints), ctx).unwrap();
        MockRig { mock, clocks, time, root, actions, effective: prepared.effective, events }
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
    _authority: DeviceAuthority,
}

impl UhdRig {
    fn new(selector: BTreeMap<Ident, Value>, constraints: &[(&str, Constraint)], config: FakeConfig) -> Result<UhdRig, ezsdr_kernel::module_api::ModuleError> {
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
        let ctx = {
            let mut ctx = PrepareContext::testing(EventSource::Node { node: rid("radio") }, time.clone(), clocks.clone(), events.clone(), queue.clone(), Arc::new(Accepting));
            ctx.run = RunId::from_string("vh8-uhd".to_owned());
            ctx.class = ExecutionClass::HardwareInLoop;
            ctx.environment = Arc::new(BTreeMap::new());
            ctx.inputs = Arc::new(BTreeMap::<ContentHash, Arc<[u8]>>::new());
            ctx.links = vec![AttachedPort::new(Ident::parse("radio").unwrap(), Ident::parse("rx0").unwrap(), Endpoint::StreamOut(Arc::new(MemLink::new())))];
            ctx.host_budget = RelativeBudget::new(Duration::new(ClockDomainId::HOST_MONOTONIC, 5_000_000_000)).unwrap();
            ctx
        };
        let mut radio = UhdRadio::from_binding(&uhd_binding(selector), device.clone())?;
        let prepared = radio.prepare(&fragment(ezsdr_radio_uhd::module_ref(), constraints), ctx)?;
        Ok(UhdRig { radio, device, clocks, time, root, actions: queue, effective: prepared.effective, events, _authority: authority })
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
        self.events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0))
    }

    /// The Provider's sections as `cleanup` writes them (UR-30).
    fn finish(mut self) -> ProviderInstance {
        self.radio.stop(ezsdr_kernel::module_api::StopMode::Orderly).unwrap();
        self.radio.cleanup();
        self.radio.instance().clone()
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
    let rejected = |events: &[ezsdr_kernel::event::Event]| -> Vec<String> {
        events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).map(|event| event.payload["reason"].as_str().unwrap().to_owned()).collect()
    };
    let mock_rejected = rejected(&mock.events());
    let uhd_rejected = rejected(&uhd.events());
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
    let late_of = |events: &[ezsdr_kernel::event::Event], t0: i64| -> Vec<(i64, i64)> {
        events
            .iter()
            .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::LATE_COMMAND)
            .map(|event| (event.payload["requested"]["ticks"].as_i64().unwrap() - t0, event.payload["applied"]["ticks"].as_i64().unwrap() - t0))
            .collect()
    };
    assert_eq!(late_of(&mock.events(), t0), [(ms(10), ms(50))], "the Mock's floor is the pending transmit change");
    assert_eq!(late_of(&uhd.events(), uhd_t0), [(ms(10), ms(50))], "the UHD's floor is the instance's");
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
