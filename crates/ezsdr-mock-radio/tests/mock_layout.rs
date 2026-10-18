//! MockRadio's layout keys (design 33 D5; spec 09's MR-38…MR-41 with spec 07's
//! RM-27…RM-32): the `keys: "channel"` selector, one stream node per logical channel, the
//! numbered configuration keys, D14's targets, per-channel cold changes, the move's
//! refusal and the SimulationChannel's.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use ezsdr_kernel::binding::{AdmissionCheckRegistry, Binding, Violation};
use ezsdr_kernel::event::{Action, ActionId, Event, EventCollector, EventSource, Target};
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::{ClockDomainId, ModuleId, ResourceId, RunId};
use ezsdr_kernel::manifest::ArtifactRef;
use ezsdr_kernel::module_api::{
    ActionReceiver, ActionSubmitter, AttachedPort, Endpoint, ExecutionClass, ModuleError, ModuleRef,
    Pacing, PrepareContext, ProfileRef, Provider, Role, UpdateClass, Version,
};
use ezsdr_kernel::plan::Fragment;
use ezsdr_kernel::policy::{EventKindRegistry, Policy};
use ezsdr_kernel::spec::{Constraint, Ident, Key, Namespace, Scalar, Value};
use ezsdr_kernel::stream::{BackPressure, BlockFlags, BlockHeader, BlockRef, DataLink, DropCarry, LatePolicy, PublishOutcome};
use ezsdr_kernel::time::{AbsoluteDeadline, ClockDomain, ClockRegistry, Duration, EpochRef, ManualTimeAuthority, Rational, RelativeBudget, SampleClockRecord, TimePoint};
use ezsdr_kernel::module_api::Requested;

use ezsdr_mock_radio::MockRadio;
use ezsdr_radio::payloads::RxOverflowPayload;
use ezsdr_sim::channel::Medium;

const ROOT: ClockDomainId = ClockDomainId::local(7);
const HOST: ClockDomainId = ClockDomainId::HOST_MONOTONIC;
/// A `x3xx` arm at instant 0 ends its synchronisation at 2 s (MR-9), so T0 is at least
/// that.
const T0: i64 = 2_000_000_000;

fn module_ref() -> ModuleRef {
    ModuleRef { id: ModuleId::parse("ezsdr.radio.mock").unwrap(), version: Version::new(2, 0, 0) }
}

fn rid(name: &str) -> ResourceId { ResourceId::parse(name).unwrap() }
fn key(name: &str) -> Key { Key::parse(name).unwrap() }
fn eq(value: Scalar) -> Constraint { Constraint::Eq { value } }

/// A resource target written as the node path the test resolves it to: the double plays
/// KC-23 with `matched` the identity, `<resource>[/<path>]` (spec 27).
fn tgt(path: &str) -> Target {
    let (resource, rest) = path.split_once('/').unwrap_or((path, ""));
    Target::Resource { resource: Ident::parse(resource).unwrap(), path: rest.to_owned() }
}

/// The node KC-23 resolves a test target to: the same path.
fn node_of(action: &Action) -> Option<ResourceId> {
    match action.target()? {
        Target::Resource { resource, path } if path.is_empty() => Some(rid(resource.as_str())),
        Target::Resource { resource, path } => Some(rid(&format!("{resource}/{path}"))),
        _ => None,
    }
}

fn node_src(path: &str) -> EventSource { EventSource::Node { node: rid(path) } }

fn waveform(samples: usize, channels: usize) -> ArtifactRef {
    let bytes = vec![0; samples * channels * 8];
    let hash = ContentHash::of_bytes(&bytes);
    ArtifactRef {
        id: Ident::parse("waveform").unwrap(),
        kind: Namespace::parse("ezsdr.input").unwrap(),
        uri: format!("mem:{hash}"),
        hash,
        size_bytes: bytes.len() as u64,
        partial: false,
        marks: Vec::new(),
        continuity: Vec::new(),
    }
}

#[derive(Default)]
struct Queue {
    actions: Mutex<VecDeque<Action>>,
    given: AtomicU64,
}

impl Queue {
    fn push(&self, action: Action) { self.actions.lock().unwrap().push_back(action); }
}

impl ActionReceiver for Queue {
    fn recv(&self) -> Option<ezsdr_kernel::module_api::Dispatched> {
        self.actions.lock().unwrap().pop_front().map(|action| {
            let node = node_of(&action);
            let id = ActionId(self.given.fetch_add(1, Ordering::SeqCst) + 1);
            ezsdr_kernel::module_api::Dispatched::new(id, action, node)
        })
    }
}

struct Accepting;
impl ActionSubmitter for Accepting {
    fn submit(&self, _action: Action) -> Result<ActionId, Vec<Violation>> { Ok(ActionId(1)) }
}

struct MemLink {
    policy: BackPressure,
    capacity: usize,
    queue: Mutex<VecDeque<BlockRef>>,
    drops: AtomicU64,
}

impl MemLink {
    fn new(policy: BackPressure, capacity: usize) -> Self {
        Self { policy, capacity, queue: Mutex::new(VecDeque::new()), drops: AtomicU64::new(0) }
    }
    fn drain(&self) -> Vec<BlockRef> {
        let mut queue = self.queue.lock().unwrap();
        queue.drain(..).collect()
    }
}

impl DataLink for MemLink {
    fn publish(&self, block: BlockRef) -> PublishOutcome {
        let mut queue = self.queue.lock().unwrap();
        if queue.len() < self.capacity { queue.push_back(block); return PublishOutcome::Accepted; }
        match self.policy {
            BackPressure::Block => PublishOutcome::Full,
            BackPressure::DropOldest => {
                queue.pop_front();
                self.drops.fetch_add(1, Ordering::Relaxed);
                queue.push_back(block);
                PublishOutcome::DroppedOldest
            }
            BackPressure::DropNewest => {
                self.drops.fetch_add(1, Ordering::Relaxed);
                PublishOutcome::DroppedNewest
            }
        }
    }
    fn receive(&self) -> Option<(BlockRef, DropCarry)> { self.queue.lock().unwrap().pop_front().map(|b| (b, DropCarry::default())) }
    fn drops(&self) -> u64 { self.drops.load(Ordering::Relaxed) }
    fn take_drop_carry(&self) -> DropCarry { DropCarry::default() }
    fn policy(&self) -> BackPressure { self.policy }
}

/// The `rx` and `tx` selector values: element strings for `x3xx`, counts for `ideal`.
fn elements(list: &[&str]) -> Value { Value::List(list.iter().map(|element| Scalar::from(*element)).collect()) }
fn count(n: i64) -> Value { Value::from(n) }

/// One layout instance to build (MR-38): its profile, selector, request and ports.
struct Layout<'a> {
    profile: &'a str,
    id: &'a str,
    rx: Value,
    tx: Value,
    selector: &'a [(&'a str, Value)],
    constraints: &'a [(&'a str, Constraint)],
    env: &'a [(&'a str, serde_json::Value)],
    /// The receive ports to attach a link to (MR-7).
    ports: &'a [&'a str],
}

struct Harness {
    mock: MockRadio,
    clocks: Arc<ClockRegistry>,
    auth: Arc<ManualTimeAuthority>,
    events: Arc<EventCollector>,
    actions: Arc<Queue>,
    links: BTreeMap<String, Arc<MemLink>>,
    effective: BTreeMap<Key, Value>,
}

fn harness(options: Layout<'_>) -> Result<Harness, ModuleError> {
    harness_links(options, &[])
}

/// As [`harness`], with further links attached (several of them to one port when the test
/// asks for it, MR-7 step 7).
fn harness_links(options: Layout<'_>, extra: &[(&str, BackPressure, usize)]) -> Result<Harness, ModuleError> {
    let version = if options.profile == "x3xx" { Version::new(0, 1, 0) } else { Version::new(2, 0, 0) };
    let mut selector = BTreeMap::from([
        (Ident::parse("id").unwrap(), Value::from(options.id.to_owned())),
        (Ident::parse("keys").unwrap(), Value::from("channel".to_owned())),
        (Ident::parse("rx").unwrap(), options.rx.clone()),
        (Ident::parse("tx").unwrap(), options.tx.clone()),
    ]);
    for (name, value) in options.selector {
        selector.insert(Ident::parse(name).unwrap(), value.clone());
    }
    let binding = Binding { module: module_ref(), selector, profile: Some(ProfileRef { name: options.profile.to_owned(), version }), feed: None };
    let mock = MockRadio::from_binding(&binding)?;
    let streams: Vec<ResourceId> = mock.instance().tree.children.iter().map(|child| child.id.clone()).collect();
    let mut mock = mock.with_medium(Medium::new());
    let clocks = Arc::new(ClockRegistry::new());
    clocks.register(ClockDomain::root(ROOT, Rational::new(1_000_000_000, 1).unwrap(), EpochRef::Arbitrary { set_by: "test".to_owned() })).unwrap();
    let auth = Arc::new(ManualTimeAuthority::new(clocks.clone(), ROOT, &[], Pacing::FreeRunning).unwrap());
    let actions = Arc::new(Queue::default());
    let mut registry = ezsdr_kernel::module_api::ModuleRegistry::new();
    let mut checks = AdmissionCheckRegistry::new();
    let mut kinds = EventKindRegistry::new();
    ezsdr_radio::register(&mut registry, &mut checks, &mut kinds).unwrap();
    ezsdr_sim::register(&mut registry, &mut checks, &mut kinds).unwrap();
    let event_kinds = kinds.kinds();
    let sources: Vec<EventSource> = std::iter::once(rid(options.id)).chain(streams).map(|node| node_src(node.path())).collect();
    let pairs: Vec<_> = sources.into_iter()
        .flat_map(|source| event_kinds.iter().cloned().map(move |kind| (source.clone(), kind)))
        .collect();
    let events = Arc::new(EventCollector::new(&pairs, &event_kinds, 4096, &Policy::default()));
    let links: BTreeMap<String, Arc<MemLink>> = options.ports.iter()
        .map(|port| ((*port).to_owned(), Arc::new(MemLink::new(BackPressure::DropOldest, 64))))
        .collect();
    let mut attached: Vec<_> = links.iter()
        .map(|(port, link)| AttachedPort::new(Ident::parse("radio").unwrap(), Ident::parse(port).unwrap(), Endpoint::StreamOut(link.clone())))
        .collect();
    attached.extend(extra.iter().map(|(port, policy, capacity)| {
        AttachedPort::new(Ident::parse("radio").unwrap(), Ident::parse(port).unwrap(), Endpoint::StreamOut(Arc::new(MemLink::new(*policy, *capacity))))
    }));
    let environment = Arc::new(options.env.iter().map(|(name, value)| (Namespace::parse(name).unwrap(), value.clone())).collect());
    let ctx = {
        let mut ctx = PrepareContext::testing(
            node_src(options.id),
            auth.clone(),
            clocks.clone(),
            events.clone(),
            actions.clone(),
            Arc::new(Accepting),
        );
        ctx.run = RunId::from_string("mock-layout".to_owned());
        ctx.class = ExecutionClass::Simulation;
        ctx.environment = environment;
        ctx.inputs = Arc::new(BTreeMap::<ContentHash, Arc<[u8]>>::new());
        ctx.links = attached;
        ctx.host_budget = RelativeBudget::new(Duration::new(HOST, 5_000_000_000)).unwrap();
        ctx
    };
    let requested = Requested {
        resource: rid(options.id),
        constraints: options.constraints.iter().map(|(name, constraint)| (key(name), constraint.clone())).collect(),
    };
    let fragment = Fragment {
        id: Ident::parse("radio").unwrap(),
        instance: module_ref(),
        role: Role::Provider,
        content: serde_json::json!({ "selector": {}, "requested": requested }),
        after: Vec::new(),
    };
    let prepared = mock.prepare(&fragment, ctx)?;
    Ok(Harness { mock, clocks, auth, events, actions, links, effective: prepared.effective })
}

fn built(options: Layout<'_>) -> Harness {
    match harness(options) {
        Ok(harness) => harness,
        Err(error) => panic!("prepare the layout Mock: {error}"),
    }
}

fn refusal(options: Layout<'_>) -> String {
    match harness(options) {
        Ok(_) => panic!("the request was admitted"),
        Err(error) => error.message,
    }
}

impl Harness {
    fn arm_start(&mut self, start: i64) -> Result<(), ModuleError> {
        self.mock.arm()?;
        self.mock.start(Some(TimePoint::new(ROOT, start)))
    }

    fn step(&mut self, tick: i64) -> Result<(), ModuleError> {
        self.auth.advance_to(TimePoint::new(ROOT, tick)).unwrap();
        self.mock.step(TimePoint::new(ROOT, tick)).map(|_| ())
    }

    fn push(&self, action: Action) { self.actions.push(action); }

    fn events(&self) -> Vec<Event> { self.events.drain(TimePoint::new(HOST, 0)) }

    /// Every block a port received, in arrival order.
    fn blocks(&self, port: &str) -> Vec<BlockRef> { self.links[port].drain() }

    /// Every block header a port received.
    fn headers(&self, port: &str) -> Vec<BlockHeader> { self.blocks(port).iter().map(|block| block.header().clone()).collect() }

    /// Every SampleClock registered for the stream at `path`.
    fn clocks_of(&self, path: &str) -> Vec<SampleClockRecord> {
        let stream = rid(path);
        self.clocks.sample_clock_records().into_iter().filter(|record| record.stream == stream).collect()
    }

    /// The rows of one Manifest section (MR-27).
    fn section(&self, suffix: &str) -> Vec<serde_json::Value> {
        self.mock.instance().sections[&Namespace::parse(&format!("ezsdr.radio.mock.{suffix}")).unwrap()]
            .as_array().cloned().unwrap_or_default()
    }

    /// The reasons of every refused, dropped or cancelled Action, in order (MR-27).
    fn rejected(&self) -> Vec<String> {
        self.section("rejected").iter().map(|row| row["reason"].as_str().unwrap_or_default().to_owned()).collect()
    }

    /// The `(key, at)` of every applied update (MR-27).
    fn applied(&self) -> Vec<(String, i64)> {
        self.section("applied").iter()
            .map(|row| (row["key"].as_str().unwrap_or_default().to_owned(), row["at"]["ticks"].as_i64().unwrap_or_default()))
            .collect()
    }

    /// The `(value, at)` of every applied update of one key, in application order (MR-27).
    fn applied_of(&self, name: &str) -> Vec<(serde_json::Value, i64)> {
        self.section("applied").into_iter()
            .filter(|row| row["key"] == serde_json::json!(name))
            .map(|row| (row["value"].clone(), row["at"]["ticks"].as_i64().unwrap_or_default()))
            .collect()
    }
}

fn update(name: &str, value: Value, class: UpdateClass, at: Option<i64>) -> Action {
    Action::UpdateParameter {
        target: tgt("mock"),
        key: key(name),
        value,
        class,
        at: at.map(|tick| AbsoluteDeadline::new(TimePoint::new(ROOT, tick))),
    }
}

fn update_at(target: &str, name: &str, value: Value, class: UpdateClass, at: Option<i64>) -> Action {
    Action::UpdateParameter {
        target: tgt(target),
        key: key(name),
        value,
        class,
        at: at.map(|tick| AbsoluteDeadline::new(TimePoint::new(ROOT, tick))),
    }
}

fn stop(target: &str) -> Action { Action::Stop { target: Some(tgt(target)) } }

fn start_rx(target: &str, at: Option<i64>) -> Action {
    Action::Command {
        target: tgt(target),
        verb: Ident::parse("start_rx").unwrap(),
        params: BTreeMap::new(),
        at: at.map(|tick| AbsoluteDeadline::new(TimePoint::new(ROOT, tick))),
    }
}

fn burst(target: &str, domain: ClockDomainId, sample: i64, samples: usize) -> Action {
    Action::TxBurst {
        target: tgt(target),
        waveform: waveform(samples, 1),
        repeat: false,
        at: AbsoluteDeadline::new(TimePoint::new(domain, sample)),
        requested_at: None,
        late_policy: LatePolicy::SendAsapAndFlag,
        metadata: BTreeMap::new(),
    }
}

/// The mixed layout the owner uses: a UBX in slot A and a BasicRX in slot B, the BasicRX's
/// two front ends as two channels (design 33 §1).
fn mixed() -> Value { elements(&["0:A:ubx:0", "0:B:basic_rx:0", "0:B:basic_rx:1"]) }

/// The one receive stream a two-channel layout holds at its default membership.
fn two() -> Value { elements(&["0:A:ubx:0", "0:B:ubx:0"]) }

#[test]
fn mr_38_a_layout_instance_uses_one_key_set() {
    // MR-38: `keys: "channel"` picks the layout builder's keys; another value is refused.
    let selector = |keys: Option<Value>, rx: Value, tx: Value| {
        let mut selector = BTreeMap::from([
            (Ident::parse("id").unwrap(), Value::from("mock".to_owned())),
            (Ident::parse("rx").unwrap(), rx),
            (Ident::parse("tx").unwrap(), tx),
        ]);
        if let Some(keys) = keys {
            selector.insert(Ident::parse("keys").unwrap(), keys);
        }
        selector
    };
    let binding = |selector: BTreeMap<Ident, Value>| Binding { module: module_ref(), selector, profile: Some(ProfileRef { name: "x3xx".to_owned(), version: Version::new(0, 1, 0) }), feed: None };
    let layout = |keys: Option<&str>| binding(selector(keys.map(|keys| Value::from(keys.to_owned())), two(), elements(&["0:A:ubx:0"])));
    for value in [Value::from("layout".to_owned()), Value::from(1), Value::List(vec![Scalar::from("channel")])] {
        let why = MockRadio::from_binding(&binding(selector(Some(value), two(), elements(&["0:A:ubx:0"])))).err().unwrap().message;
        assert!(why.contains("selector `keys` must be the string `channel`"), "{why}");
    }
    // An `ideal` layout instance exists: its `rx` and `tx` are counts (RM-28).
    let ideal = Binding { module: module_ref(), selector: BTreeMap::from([
        (Ident::parse("id").unwrap(), Value::from("mock".to_owned())),
        (Ident::parse("keys").unwrap(), Value::from("channel".to_owned())),
        (Ident::parse("rx").unwrap(), count(2)),
    ]), profile: Some(ProfileRef { name: "ideal".to_owned(), version: Version::new(2, 0, 0) }), feed: None };
    assert!(MockRadio::from_binding(&ideal).is_ok());
    // The two key sets are disjoint: the current keys are no radio keys of a layout
    // instance, and a layout key is none of a current-key instance (RM-27).
    let requested = |name: &str, constraint: Constraint| Requested { resource: rid("mock"), constraints: BTreeMap::from([(key(name), constraint)]) };
    let layout_instance = MockRadio::from_binding(&layout(Some("channel"))).unwrap();
    let report = layout_instance.coerce(&requested("radio.rx.channels", eq(Scalar::from(1)))).unwrap();
    assert_eq!(report.rejected.len(), 1);
    assert!(report.rejected[0].reason.contains("RM-4: not a radio key"), "{:?}", report.rejected[0].reason);
    let current_instance = MockRadio::from_binding(&layout(None)).unwrap();
    let report = current_instance.coerce(&requested("radio.rx.c0.frequency_hz", eq(Scalar::try_from(1.0e9).unwrap()))).unwrap();
    assert_eq!(report.rejected.len(), 1);
    assert!(report.rejected[0].reason.contains("RM-4: not a radio key"), "{:?}", report.rejected[0].reason);
    // A layout instance's own keys are the ones its description declares (RM-27, RM-30).
    let report = layout_instance.coerce(&requested("radio.rx.c0.frequency_hz", eq(Scalar::try_from(1.0e9).unwrap()))).unwrap();
    assert!(report.rejected.is_empty());
    assert_eq!(report.applied[&key("radio.rx.c0.frequency_hz")], Value::num(1.0e9).unwrap());
    // A channel or stream number the layout does not have is refused by the Provider
    // (RM-30, D2).
    let why = layout_instance.coerce(&requested("radio.rx.c1.gain_db", eq(Scalar::from(0)))).unwrap().rejected;
    assert_eq!(why.len(), 0);
    let why = layout_instance.coerce(&requested("radio.rx.c2.gain_db", eq(Scalar::from(0)))).unwrap().rejected;
    assert!(why[0].reason.contains("RM-4: no such channel c2"), "{:?}", why[0].reason);
}

#[test]
fn mr_38_the_tree_and_capabilities_are_the_layout_s() {
    let harness = built(Layout { profile: "x3xx", id: "mock", rx: mixed(), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &[], env: &[], ports: &["rx0"] });
    let tree = &harness.mock.instance().tree;
    assert_eq!(tree.kind.as_str(), "radio.device");
    assert_eq!(tree.children.iter().map(|child| child.id.path()).collect::<Vec<_>>(), ["mock/rx/s0", "mock/rx/s1", "mock/rx/s2", "mock/tx/s0"]);
    assert_eq!(tree.children.iter().map(|child| child.kind.as_str()).collect::<Vec<_>>(), ["radio.rx_stream", "radio.rx_stream", "radio.rx_stream", "radio.tx_stream"]);
    assert_eq!(tree.ports.iter().map(|port| port.name.as_str()).collect::<Vec<_>>(), ["rx0", "rx1", "rx2"]);
    let range = |lo: f64, hi: f64| ezsdr_kernel::spec::CapabilityValue::Range { min: Scalar::try_from(lo).unwrap(), max: Scalar::try_from(hi).unwrap() };
    let one = |value: Scalar| ezsdr_kernel::spec::CapabilityValue::One { value };
    // The UBX's channel 0 and the BasicRX's channels 1 and 2, each with its own values
    // (RM-29).
    assert_eq!(tree.capabilities[&key("radio.rx.c0.frequency_hz")], range(10_000_000.0, 6_000_000_000.0));
    assert_eq!(tree.capabilities[&key("radio.rx.c1.frequency_hz")], range(0.0, 100_000_000.0));
    assert_eq!(tree.capabilities[&key("radio.rx.c0.gain_db")], range(0.0, 31.5));
    assert_eq!(tree.capabilities[&key("radio.rx.c1.gain_db")], range(0.0, 0.0));
    assert_eq!(tree.capabilities[&key("radio.rx.c1.antenna")], ezsdr_kernel::spec::CapabilityValue::AnyOf { values: ["A", "B", "AB", "BA"].into_iter().map(Scalar::from).collect() });
    assert_eq!(tree.capabilities[&key("radio.rx.c0.antenna")], ezsdr_kernel::spec::CapabilityValue::AnyOf { values: ["RX2", "TX/RX"].into_iter().map(Scalar::from).collect() });
    assert_eq!(tree.capabilities[&key("radio.rx.c1.phase_behavior_on_retune")], one(Scalar::from("random_unless_timed_tune")));
    assert_eq!(tree.capabilities[&key("radio.rx.c0.path_delay_samples")], one(Scalar::from(0)));
    assert_eq!(tree.capabilities[&key("radio.tx.c0.frequency_hz")], range(10_000_000.0, 6_000_000_000.0));
    assert_eq!(tree.capabilities[&key("radio.rx.c2.stream")], range(-1.0, 2.0));
    // Every stream's rate is the motherboard's grid, and the current keys are gone.
    let decimations: Vec<Scalar> = ezsdr_radio::x3xx::decimations().iter().map(|n| Scalar::try_from(200_000_000.0 / *n as f64).unwrap()).collect();
    for stream in ["radio.rx.s0.sample_rate_hz", "radio.rx.s1.sample_rate_hz", "radio.rx.s2.sample_rate_hz", "radio.tx.s0.sample_rate_hz"] {
        assert_eq!(tree.capabilities[&key(stream)], ezsdr_kernel::spec::CapabilityValue::AnyOf { values: decimations.clone() }, "{stream}");
    }
    assert!(!tree.capabilities.contains_key(&key("radio.rx.channels")) && !tree.capabilities.contains_key(&key("radio.rx.antenna")) && !tree.capabilities.contains_key(&key("radio.rx.sample_rate_hz")));
    assert!(!tree.capabilities.contains_key(&key("radio.rx.coherent")));
    assert_eq!(tree.capabilities[&key("radio.tx.repeat_align_samples")], one(Scalar::from(2)));
    // RM-30's defaults: receive channel 0 in stream 0, every other channel in none, each
    // front end's own default frequency and antenna, and every stream at 1 Msps.
    let default = |name: &str| harness.effective[&key(name)].clone();
    assert_eq!(default("radio.rx.c0.stream"), Value::from(0));
    assert_eq!(default("radio.rx.c1.stream"), Value::from(-1));
    assert_eq!(default("radio.rx.c2.stream"), Value::from(-1));
    assert_eq!(default("radio.tx.c0.stream"), Value::from(-1));
    assert_eq!(default("radio.rx.c0.frequency_hz"), Value::num(1_000_000_000.0).unwrap());
    assert_eq!(default("radio.rx.c1.frequency_hz"), Value::num(0.0).unwrap());
    assert_eq!(default("radio.rx.c0.antenna"), Value::from("RX2"));
    assert_eq!(default("radio.rx.c1.antenna"), Value::from("AB"));
    assert_eq!(default("radio.rx.s0.sample_rate_hz"), Value::num(1_000_000.0).unwrap());
    assert_eq!(default("radio.rx.s2.sample_rate_hz"), Value::num(1_000_000.0).unwrap());
    // The effective set is exactly RM-27's: four keys per channel and one per stream.
    let expected: BTreeSet<Key> = ["c0", "c1", "c2"].iter()
        .flat_map(|n| ["frequency_hz", "gain_db", "antenna", "stream"].map(|leaf| key(&format!("radio.rx.{n}.{leaf}"))))
        .chain(["radio.rx.s0.sample_rate_hz", "radio.rx.s1.sample_rate_hz", "radio.rx.s2.sample_rate_hz"].map(key))
        .chain(["radio.tx.c0.frequency_hz", "radio.tx.c0.gain_db", "radio.tx.c0.antenna", "radio.tx.c0.stream"].map(key))
        .chain(["radio.tx.s0.sample_rate_hz"].map(key))
        .collect();
    assert_eq!(harness.effective.keys().cloned().collect::<BTreeSet<_>>(), expected);

    // `ideal` has no motherboard: its channels are `ideal` front ends and its counts are
    // the selector's (RM-28, RM-29).
    let ideal = built(Layout { profile: "ideal", id: "mock", rx: count(2), tx: count(0), selector: &[], constraints: &[], env: &[], ports: &["rx0"] });
    let tree = &ideal.mock.instance().tree;
    assert_eq!(tree.children.iter().map(|child| child.id.path()).collect::<Vec<_>>(), ["mock/rx/s0", "mock/rx/s1"]);
    assert_eq!(tree.ports.iter().map(|port| port.name.as_str()).collect::<Vec<_>>(), ["rx0", "rx1"]);
    assert_eq!(tree.capabilities[&key("radio.rx.s1.sample_rate_hz")], range(1.0, 1_000_000_000.0));
    assert_eq!(tree.capabilities[&key("radio.rx.c0.phase_behavior_on_retune")], one(Scalar::from("deterministic")));
    assert_eq!(ideal.effective[&key("radio.rx.c1.stream")], Value::from(-1));
}

#[test]
fn mr_39_each_receive_stream_declares_and_runs_its_own_clock() {
    // Two receive channels, one stream each, at two rates: two SampleClocks with their own
    // ratios, and blocks at their own instants (RM-25, RM-29).
    let constraints = [
        ("radio.rx.c1.stream", eq(Scalar::from(1))),
        ("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(10_000_000.0).unwrap())),
        ("radio.rx.s1.sample_rate_hz", eq(Scalar::try_from(1_000_000.0).unwrap())),
    ];
    let mut harness = built(Layout { profile: "x3xx", id: "mock", rx: two(), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &constraints, env: &[], ports: &["rx0", "rx1"] });
    harness.arm_start(T0).unwrap();
    // 10 Msps: 2 000 samples take 200 000 root ticks; 1 Msps: 2 000 000.
    harness.step(T0 + 199_901).unwrap();
    let fast = harness.headers("rx0");
    assert_eq!(fast.len(), 1);
    assert_eq!(fast[0].first_sample_time.ticks_in(fast[0].first_sample_time.domain()).unwrap(), 0);
    assert_eq!(fast[0].len, 2_000);
    assert!(harness.blocks("rx1").is_empty());
    harness.step(T0 + 1_999_001).unwrap();
    let slow = harness.headers("rx1");
    assert_eq!(slow.len(), 1);
    assert_eq!(slow[0].first_sample_time.ticks_in(slow[0].first_sample_time.domain()).unwrap(), 0);
    assert_ne!(fast[0].first_sample_time.domain(), slow[0].first_sample_time.domain());
    // Each stream declared its own clock and registered it with its own first block
    // (MR-7, RM-25): one clock each, on its own ratio and origin.
    let clocks: Vec<(String, i64, Rational)> = harness.clocks.sample_clock_records().into_iter()
        .map(|record| (record.stream.path().to_owned(), record.origin.ticks_in(ROOT).unwrap(), record.root_ticks_per_tick)).collect();
    assert_eq!(clocks, [
        ("mock/rx/s0".to_owned(), T0, Rational::new(100, 1).unwrap()),
        ("mock/rx/s1".to_owned(), T0, Rational::new(1000, 1).unwrap()),
    ]);
}

#[test]
fn mr_13_a_stream_s_block_channels_follow_its_channel_set() {
    // A stream of channels {0, 2}: its block has two channels, the set's smallest first
    // (RM-3), and the ramp pattern's `im` is the block position, not the logical number
    // (MR-13).
    let constraints = [("radio.rx.c2.stream", eq(Scalar::from(0)))];
    let selector = [("rx_test_pattern", Value::from("ramp".to_owned()))];
    let mut harness = built(Layout { profile: "x3xx", id: "mock", rx: mixed(), tx: elements(&["0:A:ubx:0"]), selector: &selector, constraints: &constraints, env: &[], ports: &["rx0"] });
    harness.arm_start(T0).unwrap();
    harness.step(T0 + 1_999_001).unwrap();
    let block = harness.blocks("rx0").into_iter().next().unwrap();
    assert_eq!(block.header().channels, 2);
    let bytes = block.host_bytes().unwrap();
    let f32_at = |offset: usize| f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    assert_eq!(f32_at(0), 0.0);
    assert_eq!(f32_at(4), 0.0);
    // The second block channel is logical channel 2; its `im` is 1/64, the position in the
    // block, not 2/64.
    assert_eq!(f32_at(16_000), 0.0);
    assert_eq!(f32_at(16_004), 1.0 / 64.0);
    assert_eq!(f32_at(16_008), 1.0 / 65536.0);
}

#[test]
fn mr_12_d14_the_actions_of_a_layout_instance_name_a_stream() {
    let constraints = [
        ("radio.rx.c1.stream", eq(Scalar::from(1))),
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
    ];
    let mut harness = built(Layout { profile: "ideal", id: "mock", rx: count(2), tx: count(1), selector: &[], constraints: &constraints, env: &[], ports: &["rx0", "rx1"] });
    harness.arm_start(0).unwrap();
    let tx_domain = harness.clocks_of("mock/tx/s0")[0].domain;
    // D14: an update names the device or a stream node, and the key decides what changes.
    harness.push(update("radio.rx.c0.frequency_hz", Value::num(2.0e9).unwrap(), UpdateClass::HardwareTimed, None));
    harness.push(update_at("mock/rx/s1", "radio.rx.c1.gain_db", Value::num(6.0).unwrap(), UpdateClass::HardwareTimed, None));
    // A burst names its transmit stream only.
    harness.push(burst("mock/tx/s0", tx_domain, 3_000, 100));
    harness.step(1).unwrap();
    assert_eq!(harness.rejected(), Vec::<String>::new());
    assert_eq!(harness.applied().iter().map(|(name, _)| name.clone()).collect::<Vec<_>>(), ["radio.rx.c0.frequency_hz", "radio.rx.c1.gain_db"]);
    // The wrong destinations: the device for a burst, a receive stream for one, a stream
    // number the instance does not have, and a node that is neither.
    harness.push(burst("mock/tx", tx_domain, 3_100, 100));
    harness.push(burst("mock/rx/s0", tx_domain, 3_200, 100));
    harness.push(stop("mock/rx/s9"));
    harness.push(update_at("mock/tx/s9", "radio.tx.c0.gain_db", Value::num(1.0).unwrap(), UpdateClass::HardwareTimed, None));
    // A capability-only leaf of RM-27's table declares no class, and neither does an
    // instance-wide capability, so neither is updateable (MR-18).
    harness.push(update("radio.rx.c0.frequency_step_hz", Value::num(1.0).unwrap(), UpdateClass::Cold, None));
    harness.push(update("radio.tx.repeat_max_samples", Value::from(1), UpdateClass::Cold, None));
    harness.push(start_rx("mock", None));
    harness.step(2).unwrap();
    let reasons = harness.rejected();
    assert_eq!(reasons.len(), 7, "{reasons:?}");
    assert!(reasons[0].starts_with("MR-16:") && reasons[1].starts_with("MR-16:"));
    assert!(reasons[2].starts_with("MR-25: Stop target is not this device or its stream"));
    assert!(reasons[3].starts_with("MR-18: target or configuration key is not updateable"));
    assert!(reasons[4].starts_with("MR-18: target or configuration key is not updateable") && reasons[5].starts_with("MR-18: target or configuration key is not updateable"));
    assert!(reasons[6].starts_with("MR-29: Action `command` is not supported"));
    // A `Stop` of one stream stops it alone: stream 1's clock ends and stream 0 keeps
    // publishing; `start_rx` on stream 1's node starts it again.
    let mut harness = built(Layout { profile: "ideal", id: "mock", rx: count(2), tx: count(0), selector: &[], constraints: &constraints[..1], env: &[], ports: &["rx0", "rx1"] });
    harness.arm_start(0).unwrap();
    harness.push(stop("mock/rx/s1"));
    harness.step(2_500_000).unwrap();
    let first = harness.clocks_of("mock/rx/s1");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].ended_at.unwrap().ticks_in(ROOT).unwrap(), 2_500_000);
    assert_eq!(harness.clocks_of("mock/rx/s0").len(), 1);
    assert!(harness.clocks_of("mock/rx/s0")[0].ended_at.is_none());
    harness.blocks("rx0");
    harness.blocks("rx1");
    harness.step(9_000_000).unwrap();
    assert!(!harness.blocks("rx0").is_empty());
    assert!(harness.blocks("rx1").is_empty());
    harness.push(start_rx("mock/rx/s1", Some(10_000_000)));
    harness.step(10_000_000).unwrap();
    harness.step(19_000_000).unwrap();
    let restarted = harness.clocks_of("mock/rx/s1");
    assert_eq!(restarted.len(), 2);
    assert_eq!(restarted[1].origin.ticks_in(ROOT).unwrap(), 10_000_000);
    assert!(!harness.blocks("rx1").is_empty());
}

#[test]
fn mr_18_a_layout_instance_s_changes_belong_to_their_own_stream() {
    // A mixed layout: the UBX's channel 0 in stream 0, the BasicRX's channel 1 in stream 1,
    // both linked and running.
    let constraints = [
        ("radio.rx.c1.stream", eq(Scalar::from(1))),
        ("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(4_000_000.0).unwrap())),
        ("radio.rx.s1.sample_rate_hz", eq(Scalar::try_from(1_000_000.0).unwrap())),
    ];
    let mut harness = built(Layout { profile: "x3xx", id: "mock", rx: mixed(), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &constraints, env: &[], ports: &["rx0", "rx1"] });
    harness.arm_start(T0).unwrap();
    harness.step(T0 + 10_000_000).unwrap();
    assert_eq!(harness.clocks_of("mock/rx/s0").len(), 1);
    assert_eq!(harness.clocks_of("mock/rx/s1").len(), 1);
    // Channel 1's antenna is `cold` (D16): the stream that holds it is cut, begins a new
    // segment a start lead after the cut and registers a new SampleClock, though its set
    // and rate do not change.
    harness.push(update("radio.rx.c1.antenna", Value::from("B"), UpdateClass::Cold, None));
    harness.step(T0 + 10_000_000).unwrap();
    assert_eq!(harness.rejected(), Vec::<String>::new());
    assert_eq!(harness.applied(), [("radio.rx.c1.antenna".to_owned(), T0 + 10_000_000)]);
    let ended = harness.clocks_of("mock/rx/s1")[0].ended_at.unwrap().ticks_in(ROOT).unwrap();
    assert_eq!(ended, T0 + 10_000_000);
    harness.step(T0 + 70_000_000).unwrap();
    let clocks = harness.clocks_of("mock/rx/s1");
    assert_eq!(clocks.len(), 2);
    // RM-18's ready instant for a receive segment after a cut: the cut's instant, the
    // receive call in progress (2 000 samples at 1 Msps = 2 ms), the 3 ms allowance and the
    // direction's 50 ms start lead.
    assert_eq!(clocks[1].origin.ticks_in(ROOT).unwrap(), T0 + 65_000_000);
    assert_eq!(harness.clocks_of("mock/rx/s0").len(), 1);
    // A per-channel value whose number is not the channel's own front end is judged on
    // that channel alone: the BasicRX's channel 1 has no gain to set and tunes only its
    // DDC, while the UBX's channel 0 takes the same values (RM-8, RM-29).
    harness.push(update("radio.rx.c1.frequency_hz", Value::num(200_000_000.0).unwrap(), UpdateClass::HardwareTimed, None));
    harness.push(update("radio.rx.c0.frequency_hz", Value::num(200_000_000.0).unwrap(), UpdateClass::HardwareTimed, None));
    harness.push(update("radio.rx.c1.gain_db", Value::num(10.0).unwrap(), UpdateClass::HardwareTimed, None));
    harness.step(T0 + 70_000_001).unwrap();
    let reasons = harness.rejected();
    assert_eq!(reasons.len(), 2, "{reasons:?}");
    assert!(reasons[0].starts_with("MR-18: RM-8: 200000000 is outside 0"), "{reasons:?}");
    assert!(reasons[1].starts_with("MR-18: RM-8: 10 is outside 0..0"), "{reasons:?}");
    // Membership: a channel leaving its stream books a `cold` change on that stream alone,
    // and with no channel left the stream holds no segment (RM-21, RM-27).
    harness.push(update_at("mock/rx/s1", "radio.rx.c1.stream", Value::from(-1), UpdateClass::Cold, None));
    harness.step(T0 + 130_000_000).unwrap();
    assert_eq!(harness.rejected().len(), 2);
    let clocks = harness.clocks_of("mock/rx/s1");
    assert_eq!(clocks.len(), 2);
    assert!(clocks[1].ended_at.is_some());
    // A channel moving from one stream to another is refused, and nothing changes (RM-34).
    harness.push(update("radio.rx.c0.stream", Value::from(1), UpdateClass::Cold, None));
    harness.step(T0 + 130_000_001).unwrap();
    let reasons = harness.rejected();
    assert_eq!(reasons.len(), 3, "{reasons:?}");
    assert!(reasons[2].starts_with("RM-34: moving channel c0 from stream s0 to stream s1"), "{reasons:?}");
    assert_eq!(harness.applied().len(), 3);
    // The rate key belongs to its own stream: only that stream's clock changes (RM-27).
    harness.push(update("radio.rx.s0.sample_rate_hz", Value::num(2_000_000.0).unwrap(), UpdateClass::Cold, None));
    harness.step(T0 + 130_000_001).unwrap();
    harness.step(T0 + 300_000_000).unwrap();
    assert_eq!(harness.applied().len(), 4);
    assert_eq!(harness.clocks_of("mock/rx/s0").len(), 2);
    assert_eq!(harness.clocks_of("mock/rx/s1").len(), 2);
}


#[test]
fn mr_09_each_transmit_stream_has_its_own_clock_and_line() {
    // Two transmit channels, one stream each (a UBX in slot A and a BasicTX in slot B): at
    // `arm` each stream registers its own SampleClock and books its own line (MR-9), a burst
    // is held on the stream its own node names, a `cold` rate change moves that stream's
    // clock alone, and a `Stop` of one stream cancels only its bursts (MR-12, MR-16, MR-25).
    let constraints = [
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.c1.stream", eq(Scalar::from(1))),
        ("radio.tx.s0.sample_rate_hz", eq(Scalar::try_from(2_000_000.0).unwrap())),
        ("radio.tx.s1.sample_rate_hz", eq(Scalar::try_from(1_000_000.0).unwrap())),
    ];
    let mut harness = built(Layout { profile: "x3xx", id: "mock", rx: elements(&["0:A:ubx:0"]), tx: elements(&["0:A:ubx:0", "0:B:basic_tx:0"]), selector: &[], constraints: &constraints, env: &[], ports: &["rx0"] });
    harness.arm_start(T0).unwrap();
    // 2 Msps: root ticks per sample 500 (a 1 GHz root and the 100 decimation); 1 Msps: 1 000.
    let clocks: Vec<(String, i64, Rational)> = harness.clocks.sample_clock_records().into_iter()
        .map(|record| (record.stream.path().to_owned(), record.origin.ticks_in(ROOT).unwrap(), record.root_ticks_per_tick)).collect();
    assert_eq!(clocks, [
        ("mock/tx/s0".to_owned(), 0, Rational::new(500, 1).unwrap()),
        ("mock/tx/s1".to_owned(), 0, Rational::new(1000, 1).unwrap()),
    ]);
    // A `cold` change of stream 1's rate ends its own clock and starts its successor a
    // start lead after the cut; stream 0's clock is untouched (RM-25, MR-18).
    harness.push(update_at("mock/tx/s1", "radio.tx.s1.sample_rate_hz", Value::num(500_000.0).unwrap(), UpdateClass::Cold, Some(T0 + 100_000_000)));
    harness.step(T0 + 100_000_000).unwrap();
    assert_eq!(harness.rejected(), Vec::<String>::new());
    let moved = harness.clocks_of("mock/tx/s1");
    assert_eq!(moved.len(), 2);
    assert_eq!(moved[0].ended_at.unwrap().ticks_in(ROOT).unwrap(), T0 + 100_000_000);
    assert_eq!(moved[1].root_ticks_per_tick, Rational::new(2000, 1).unwrap());
    assert_eq!(moved[1].origin.ticks_in(ROOT).unwrap(), T0 + 150_000_000);
    let s0 = harness.clocks_of("mock/tx/s0");
    assert_eq!(s0.len(), 1);
    assert!(s0[0].ended_at.is_none());
    // A burst on each stream's own node is held and transmitted on that stream: two records,
    // each of the ten samples of its own waveform (MR-16, MR-23).
    harness.push(burst("mock/tx/s0", s0[0].domain, 6_000_000, 10));
    harness.push(burst("mock/tx/s1", moved[1].domain, 1_000_000, 10));
    // Received now, held for their own instants: 3 s on stream 0 and, on stream 1's new
    // clock (origin 2.15 s), 4.15 s.
    harness.step(T0 + 200_000_000).unwrap();
    assert_eq!(harness.rejected(), Vec::<String>::new());
    harness.step(T0 + 3_000_000_000).unwrap();
    assert_eq!(harness.rejected(), Vec::<String>::new());
    let records = harness.section("bursts");
    assert_eq!(records.len(), 2, "{records:?}");
    assert_eq!(records.iter().map(|row| row["samples"].as_i64().unwrap()).collect::<Vec<_>>(), [10, 10]);
    // A `Stop` of stream 1 cancels the burst held on it and nothing of stream 0's, whose
    // burst is transmitted (MR-25).
    harness.push(burst("mock/tx/s1", moved[1].domain, 2_500_000, 10));
    harness.push(stop("mock/tx/s1"));
    harness.push(burst("mock/tx/s0", s0[0].domain, 12_000_000, 10));
    harness.step(T0 + 3_100_000_000).unwrap();
    let reasons = harness.rejected();
    assert_eq!(reasons.len(), 1, "{reasons:?}");
    assert!(reasons[0].starts_with("MR-25: cancelled by stop"), "{reasons:?}");
    harness.step(T0 + 4_100_000_000).unwrap();
    assert_eq!(harness.section("bursts").len(), 3);
}

#[test]
fn mr_7_each_receive_port_takes_its_own_links() {
    // MR-7 step 7: the one-Block-link rule is each port's own, not the instance's. Two
    // ports with one `Block` link each are admitted; two `Block` links on one port are
    // refused (the refusal is the port's).
    let links = [("rx0", BackPressure::Block, 1), ("rx1", BackPressure::Block, 1)];
    let ok = harness_links(Layout { profile: "x3xx", id: "mock", rx: two(), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &[("radio.rx.c1.stream", eq(Scalar::from(1)))], env: &[], ports: &[] }, &links);
    assert!(ok.is_ok(), "{:?}", ok.err().map(|error| error.message));
    let refused = [("rx0", BackPressure::Block, 1), ("rx0", BackPressure::Block, 1)];
    let why = harness_links(Layout { profile: "x3xx", id: "mock", rx: two(), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &[("radio.rx.c1.stream", eq(Scalar::from(1)))], env: &[], ports: &[] }, &refused).err().expect("two Block links on one port").message;
    assert!(why.contains("multiple rx links cannot use Block back-pressure"), "{why}");
    // A link on a port whose stream holds no channel is kept, and the stream begins no
    // segment: the same as the current keys' `rx.channels = 0` with a link (RM-21, MR-7).
    let mut idle = built(Layout { profile: "ideal", id: "mock", rx: count(2), tx: count(0), selector: &[], constraints: &[], env: &[], ports: &["rx0", "rx1"] });
    idle.arm_start(0).unwrap();
    idle.step(9_000_000).unwrap();
    assert!(!idle.blocks("rx0").is_empty());
    assert!(idle.blocks("rx1").is_empty());
    assert_eq!(idle.clocks_of("mock/rx/s1").len(), 0);
}

#[test]
fn mr_41_a_layout_instance_takes_no_simulation_channel() {
    let env = [("sim.channel", serde_json::json!({ "couplings": [] }))];
    let why = refusal(Layout { profile: "x3xx", id: "mock", rx: mixed(), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &[], env: &env, ports: &["rx0"] });
    assert!(why.starts_with("MR-41: "), "{why}");
}

#[test]
fn rm_31_rm_32_a_layout_instance_s_violations_are_judged_at_runtime() {
    // RM-31: while a transmit channel of a duplex board is in a stream, that board's
    // receive channel may not leave RX2. The judgement is on the whole configuration, so an
    // update is refused even though it names only the antenna (RM-30, RM-31).
    let constraints = [("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let mut harness = built(Layout { profile: "x3xx", id: "mock", rx: elements(&["0:A:ubx:0"]), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &constraints, env: &[], ports: &["rx0"] });
    harness.arm_start(T0).unwrap();
    harness.push(update("radio.rx.c0.antenna", Value::from("TX/RX"), UpdateClass::Cold, None));
    harness.step(T0 + 100_000).unwrap();
    let reasons = harness.rejected();
    assert_eq!(reasons.len(), 1, "{reasons:?}");
    assert!(reasons[0].contains("RM-31:") && reasons[0].contains("receive channel c0"), "{reasons:?}");
    assert!(harness.applied().is_empty());
    // The receive channel's antenna may still be set while the transmit channel is in no
    // stream.
    let mut free = built(Layout { profile: "x3xx", id: "mock", rx: elements(&["0:A:ubx:0"]), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &[], env: &[], ports: &["rx0"] });
    free.arm_start(T0).unwrap();
    free.push(update("radio.rx.c0.antenna", Value::from("TX/RX"), UpdateClass::Cold, None));
    free.step(T0 + 100_000).unwrap();
    assert_eq!(free.rejected(), Vec::<String>::new());

    // `prepare` judges its own configuration on RM-31 and RM-32 too (MR-7, MR-30).
    let commits = [("radio.tx.c0.stream", eq(Scalar::from(0))), ("radio.rx.c0.antenna", eq(Scalar::from("TX/RX")))];
    let why = refusal(Layout { profile: "x3xx", id: "mock", rx: elements(&["0:A:ubx:0"]), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &commits, env: &[], ports: &["rx0"] });
    assert!(why.contains("RM-31:") && why.contains("MR-7: rejected"), "{why}");
    let over = [("radio.rx.c1.stream", eq(Scalar::from(0))), ("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(20_000_000.0).unwrap()))];
    let why = refusal(Layout { profile: "x3xx", id: "mock", rx: two(), tx: elements(&["0:A:ubx:0"]), selector: &[("link_gbps", Value::List(vec![Scalar::from(1)]))], constraints: &over, env: &[], ports: &["rx0", "rx1"] });
    assert!(why.contains("RM-32:") && why.contains("MR-7: rejected"), "{why}");

    // RM-32: one link's bytes per second, whatever `links` names. A 1 GbE link carries
    // 100 MB/s, so two channels of 20 Msps need 160 MB/s and are refused (RM-32).
    let constraints = [
        ("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(20_000_000.0).unwrap())),
        ("radio.rx.s1.sample_rate_hz", eq(Scalar::try_from(20_000_000.0).unwrap())),
    ];
    let selector = [("link_gbps", Value::List(vec![Scalar::from(1)])), ("links", Value::List(vec![Scalar::from(2)]))];
    let mut harness = built(Layout { profile: "x3xx", id: "mock", rx: two(), tx: elements(&["0:A:ubx:0"]), selector: &selector, constraints: &constraints, env: &[], ports: &["rx0", "rx1"] });
    harness.arm_start(T0).unwrap();
    // One channel at 20 Msps on the 1 GbE link is 80 MB/s and admitted by `prepare`; the
    // second channel's membership takes the direction over the limit.
    assert_eq!(harness.effective[&key("radio.rx.c1.stream")], Value::from(-1));
    harness.push(update("radio.rx.c1.stream", Value::from(0), UpdateClass::Cold, None));
    harness.step(T0 + 100_000).unwrap();
    let reasons = harness.rejected();
    assert_eq!(reasons.len(), 1, "{reasons:?}");
    assert!(reasons[0].contains("RM-32:") && reasons[0].contains("160000000"), "{reasons:?}");
    assert!(harness.applied().is_empty());
    // The same membership on the other channel is admitted when the sum fits: 20 + 5 Msps.
    let constraints = [
        ("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(10_000_000.0).unwrap())),
        ("radio.rx.s1.sample_rate_hz", eq(Scalar::try_from(2_000_000.0).unwrap())),
    ];
    let mut fits = built(Layout { profile: "x3xx", id: "mock", rx: two(), tx: elements(&["0:A:ubx:0"]), selector: &selector, constraints: &constraints, env: &[], ports: &["rx0", "rx1"] });
    fits.arm_start(T0).unwrap();
    fits.push(update("radio.rx.c1.stream", Value::from(0), UpdateClass::Cold, None));
    fits.step(T0 + 100_000).unwrap();
    assert_eq!(fits.rejected(), Vec::<String>::new());
    assert_eq!(fits.applied().len(), 1);
}

#[test]
fn mr_21_a_fault_cuts_the_lowest_running_receive_stream() {
    // Two receive streams both running: the instance's fault cuts the lowest-numbered one
    // (D5), and the other stream's blocks stay contiguous.
    let constraints = [("radio.rx.c1.stream", eq(Scalar::from(1)))];
    let env = [("sim.faults", serde_json::json!([{ "at_ns": 1_000_000, "fault": "rx_overflow", "target": "radio" }]))];
    let mut harness = built(Layout { profile: "x3xx", id: "mock", rx: two(), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &constraints, env: &env, ports: &["rx0", "rx1"] });
    harness.arm_start(T0).unwrap();
    harness.step(T0 + 52_999_001).unwrap();
    let faults = harness.section("faults");
    assert_eq!(faults.len(), 1);
    assert_eq!((faults[0]["applied"].clone(), faults[0]["lost"].clone()), (serde_json::json!(true), serde_json::json!(50_000)));
    let fast = harness.headers("rx0");
    let restarted = fast.iter().find(|header| header.first_sample_time.ticks_in(header.first_sample_time.domain()).unwrap() == 51_000).expect("the restarted block");
    assert!(restarted.flags.contains(BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED));
    let overflows: Vec<_> = harness.events().into_iter()
        .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW)
        .map(|event| (event.source.clone(), RxOverflowPayload::from_payload(&event.payload).unwrap().lost))
        .collect();
    assert_eq!(overflows, [(node_src("mock/rx/s0"), 50_000)]);
    assert!(harness.headers("rx1").iter().all(|header| !header.flags.contains(BlockFlags::GAP_BEFORE)));
    // The lowest stream is not running there: the fault cuts the one that is.
    let mut alone = built(Layout { profile: "x3xx", id: "mock", rx: two(), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &[("radio.rx.c1.stream", eq(Scalar::from(1)))], env: &env, ports: &["rx1"] });
    // A sequence error cuts the same stream: `block_len` samples lost, the next block
    // carrying `SEQ_DISCONTINUITY` and not `RESTARTED` (MR-22).
    let sequence = [("sim.faults", serde_json::json!([{ "at_ns": 1_000_000, "fault": "rx_sequence_error", "target": "radio" }]))];
    let constraints = [("radio.rx.c1.stream", eq(Scalar::from(1)))];
    let mut lost = built(Layout { profile: "x3xx", id: "mock", rx: two(), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &constraints, env: &sequence, ports: &["rx0", "rx1"] });
    lost.arm_start(T0).unwrap();
    lost.step(T0 + 5_000_000).unwrap();
    let faults = lost.section("faults");
    assert_eq!((faults[0]["applied"].clone(), faults[0]["lost"].clone()), (serde_json::json!(true), serde_json::json!(2_000)));
    let fast = lost.headers("rx0");
    // The loss is `k_f`'s sample 1 000 through 2 999: the next block begins at 3 000.
    let after = fast.iter().find(|header| header.first_sample_time.ticks_in(header.first_sample_time.domain()).unwrap() == 3_000).expect("the block after the loss");
    assert!(after.flags.contains(BlockFlags::GAP_BEFORE | BlockFlags::SEQ_DISCONTINUITY));
    assert!(!after.flags.contains(BlockFlags::RESTARTED));
    assert!(lost.headers("rx1").iter().all(|header| !header.flags.contains(BlockFlags::SEQ_DISCONTINUITY | BlockFlags::GAP_BEFORE)));
    alone.arm_start(T0).unwrap();
    alone.step(T0 + 52_999_001).unwrap();
    let faults = alone.section("faults");
    assert_eq!((faults[0]["applied"].clone(), faults[0]["lost"].clone()), (serde_json::json!(true), serde_json::json!(50_000)));
    let slow = alone.headers("rx1");
    let restarted = slow.iter().find(|header| header.first_sample_time.ticks_in(header.first_sample_time.domain()).unwrap() == 51_000).expect("the restarted block");
    assert!(restarted.flags.contains(BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED));
    // The overflow travels on the handle of the stream the fault cut (MR-37, MR-28).
    let overflows: Vec<_> = alone.events().into_iter()
        .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW)
        .map(|event| event.source.clone())
        .collect();
    assert_eq!(overflows, [node_src("mock/rx/s1")]);
}

#[test]
fn mr_28_a_layout_instance_s_events_name_their_stream_node() {
    // A late or refused burst's TIME_ERROR names the transmit stream that received it, and
    // a receive overflow names the receive stream (MR-28).
    let constraints = [("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let mut harness = built(Layout { profile: "ideal", id: "mock", rx: count(1), tx: count(1), selector: &[], constraints: &constraints, env: &[], ports: &["rx0"] });
    harness.arm_start(0).unwrap();
    let tx_domain = harness.clocks_of("mock/tx/s0")[0].domain;
    // `drop_and_flag` with no lead on a target already past: one TIME_ERROR on the transmit
    // stream's node.
    harness.push(Action::TxBurst {
        target: tgt("mock/tx/s0"),
        waveform: waveform(100, 1),
        repeat: false,
        at: AbsoluteDeadline::new(TimePoint::new(tx_domain, 0)),
        requested_at: None,
        late_policy: LatePolicy::DropAndFlag,
        metadata: BTreeMap::new(),
    });
    harness.step(10).unwrap();
    let errors: Vec<EventSource> = harness.events().into_iter()
        .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR)
        .map(|event| event.source.clone())
        .collect();
    assert_eq!(errors, [node_src("mock/tx/s0")]);
}

#[test]
fn mr_30_two_layout_mocks_one_seed_identical_output() {
    let selector = [("block_len_jitter", Value::from(true)), ("rx_test_pattern", Value::from("ramp".to_owned()))];
    let constraints = [("radio.rx.c1.stream", eq(Scalar::from(1)))];
    let env = [("sim.seed", serde_json::json!(7))];
    let build = || built(Layout { profile: "x3xx", id: "mock", rx: mixed(), tx: elements(&["0:A:ubx:0"]), selector: &selector, constraints: &constraints, env: &env, ports: &["rx0", "rx1"] });
    let (mut left, mut right) = (build(), build());
    left.arm_start(T0).unwrap();
    right.arm_start(T0).unwrap();
    left.step(T0 + 25_000_000).unwrap();
    right.step(T0 + 25_000_000).unwrap();
    let headers = |harness: &Harness, port: &str| harness.blocks(port).iter().map(|block| block.header().clone()).collect::<Vec<_>>();
    let (left_rx0, right_rx0) = (headers(&left, "rx0"), headers(&right, "rx0"));
    let (left_rx1, right_rx1) = (headers(&left, "rx1"), headers(&right, "rx1"));
    assert!(!left_rx0.is_empty() && !left_rx1.is_empty());
    assert_eq!(left_rx0, right_rx0);
    assert_eq!(left_rx1, right_rx1);
    assert_eq!(left.mock.instance().sections, right.mock.instance().sections);
    assert_eq!(left.events(), right.events());
    // A jittered layout stream draws from its own generator stream, so its lengths differ
    // from the nominal 2 000, and two streams do not draw one sequence (MR-12).
    assert!(left_rx0.iter().any(|header| header.len != 2_000), "{:?}", left_rx0.iter().map(|header| header.len).collect::<Vec<_>>());
    let prefix = |headers: &[BlockHeader]| headers.iter().take(4).map(|header| header.len).collect::<Vec<_>>();
    assert_ne!(prefix(&left_rx0), prefix(&left_rx1));
}

#[test]
fn mr_18_a_layout_cold_change_is_floored_at_every_earlier_cold_change() {
    // RM-25: a layout instance's `cold` change never takes effect before a `cold` change the
    // instance received earlier, whatever stream or line it belongs to. Channel 1 leaves
    // stream 1 at `now + 5 ms`; a change received after it that puts it back is floored at
    // that instant, so it is late and books on the stream the configuration then names.
    let constraints = [
        ("radio.rx.c1.stream", eq(Scalar::from(1))),
        ("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(4_000_000.0).unwrap())),
        ("radio.rx.s1.sample_rate_hz", eq(Scalar::try_from(1_000_000.0).unwrap())),
    ];
    let mut harness = built(Layout { profile: "x3xx", id: "mock", rx: mixed(), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &constraints, env: &[], ports: &["rx0", "rx1"] });
    harness.arm_start(T0).unwrap();
    let now = T0 + 10_000_000;
    // Both receipts arrive at `now`, before either instant.
    harness.push(update("radio.rx.c1.stream", Value::from(-1), UpdateClass::Cold, Some(now + 5_000_000)));
    harness.push(update("radio.rx.c1.stream", Value::from(1), UpdateClass::Cold, None));
    harness.step(now).unwrap();
    assert_eq!(harness.rejected(), Vec::<String>::new());
    let late: Vec<serde_json::Value> = harness.events().into_iter()
        .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::LATE_COMMAND)
        .map(|event| event.payload["key"].clone())
        .collect();
    assert_eq!(late, [serde_json::json!("radio.rx.c1.stream")]);
    harness.blocks("rx1");
    harness.step(now + 5_000_000).unwrap();
    assert_eq!(
        harness.applied_of("radio.rx.c1.stream"),
        [(serde_json::json!(-1), now + 5_000_000), (serde_json::json!(1), now + 5_000_000)],
    );
    // The line agrees with the configuration: stream 1 is cut at `now + 5 ms` and begins a
    // new segment that holds channel 1 again (RM-25, MR-18).
    harness.blocks("rx1");
    harness.step(now + 200_000_000).unwrap();
    let clocks = harness.clocks_of("mock/rx/s1");
    assert_eq!(clocks.len(), 2, "{clocks:?}");
    assert_eq!(clocks[0].ended_at.unwrap().ticks_in(ROOT).unwrap(), now + 5_000_000);
    let restarted = harness.headers("rx1");
    assert!(!restarted.is_empty());
    assert!(restarted.iter().all(|header| header.channels == 1 && header.first_sample_time.domain() == clocks[1].domain), "{restarted:?}");
}

#[test]
fn mr_18_a_layout_cold_floor_crosses_directions() {
    // RM-25: the floor is the instance's, not the key's direction: a transmit membership
    // change received after a receive antenna change takes effect no earlier than it, and
    // the judgement then refuses it by RM-31 (RM-30, MR-18).
    let mut harness = built(Layout { profile: "x3xx", id: "mock", rx: elements(&["0:A:ubx:0"]), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &[], env: &[], ports: &["rx0"] });
    harness.arm_start(T0).unwrap();
    let now = T0 + 10_000_000;
    harness.push(update("radio.rx.c0.antenna", Value::from("TX/RX"), UpdateClass::Cold, Some(now + 20_000_000)));
    harness.push(update("radio.tx.c0.stream", Value::from(0), UpdateClass::Cold, Some(now + 10_000_000)));
    harness.step(now).unwrap();
    harness.step(now + 20_000_000).unwrap();
    let reasons = harness.rejected();
    assert_eq!(reasons.len(), 1, "{reasons:?}");
    assert!(reasons[0].contains("RM-31:"), "{reasons:?}");
    assert_eq!(harness.applied(), [("radio.rx.c0.antenna".to_owned(), now + 20_000_000)]);
    // The transmit channel stayed in no stream, so its stream never held a clock (RM-21).
    assert!(harness.clocks_of("mock/tx/s0").is_empty());
}

#[test]
fn mr_18_a_layout_reservations_cannot_invalidate_an_earlier_one() {
    // RM-25, RM-40: an update received after a reservation cannot overtake it, so the stream
    // a channel is in at the later instant decides what the change does. Membership: the
    // channel is in stream 1 then, so putting it in stream 0 is a move and is refused.
    let constraints = [
        ("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(4_000_000.0).unwrap())),
        ("radio.rx.s1.sample_rate_hz", eq(Scalar::try_from(1_000_000.0).unwrap())),
    ];
    let mut harness = built(Layout { profile: "x3xx", id: "mock", rx: two(), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &constraints, env: &[], ports: &["rx0", "rx1"] });
    harness.arm_start(T0).unwrap();
    let now = T0 + 10_000_000;
    harness.push(update("radio.rx.c1.stream", Value::from(1), UpdateClass::Cold, Some(now + 20_000_000)));
    harness.push(update("radio.rx.c1.stream", Value::from(0), UpdateClass::Cold, Some(now + 10_000_000)));
    harness.step(now).unwrap();
    harness.step(now + 20_000_000).unwrap();
    let reasons = harness.rejected();
    assert_eq!(reasons.len(), 1, "{reasons:?}");
    assert!(reasons[0].starts_with("RM-34: moving channel c1 from stream s1 to stream s0"), "{reasons:?}");
    assert_eq!(harness.applied_of("radio.rx.c1.stream"), [(serde_json::json!(1), now + 20_000_000)]);
    harness.step(now + 200_000_000).unwrap();
    assert_eq!(harness.clocks_of("mock/rx/s1").len(), 1);
    assert_eq!(harness.clocks_of("mock/rx/s0").len(), 1);
    // Throughput: at the later instant the second channel is in stream 1 and takes the 1 GbE
    // link's 100 MB/s over, so the rate change is refused there though it would fit before it
    // (RM-32).
    let selector = [("link_gbps", Value::List(vec![Scalar::from(1)]))];
    let constraints = [
        ("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(5_000_000.0).unwrap())),
        ("radio.rx.s1.sample_rate_hz", eq(Scalar::try_from(10_000_000.0).unwrap())),
    ];
    let mut harness = built(Layout { profile: "x3xx", id: "mock", rx: two(), tx: elements(&["0:A:ubx:0"]), selector: &selector, constraints: &constraints, env: &[], ports: &["rx0", "rx1"] });
    harness.arm_start(T0).unwrap();
    harness.push(update("radio.rx.c1.stream", Value::from(1), UpdateClass::Cold, Some(now + 20_000_000)));
    harness.push(update("radio.rx.s0.sample_rate_hz", Value::num(20_000_000.0).unwrap(), UpdateClass::Cold, Some(now + 15_000_000)));
    harness.step(now).unwrap();
    harness.step(now + 20_000_000).unwrap();
    let reasons = harness.rejected();
    assert_eq!(reasons.len(), 1, "{reasons:?}");
    assert!(reasons[0].contains("RM-32:") && reasons[0].contains("120000000"), "{reasons:?}");
    assert_eq!(harness.applied().iter().map(|(name, _)| name.clone()).collect::<Vec<_>>(), ["radio.rx.c1.stream"]);
}

#[test]
fn mr_18_a_layout_cold_floor_holds_without_a_line() {
    // RM-25: the floor is the instance's `cold` changes, not a line's, so a receive stream
    // that runs nothing — no link, no segment — still keeps the order of its reservations
    // (MR-18).
    let constraints = [("radio.rx.c1.stream", eq(Scalar::from(1)))];
    let mut harness = built(Layout { profile: "x3xx", id: "mock", rx: two(), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &constraints, env: &[], ports: &["rx0"] });
    harness.arm_start(T0).unwrap();
    let now = T0 + 10_000_000;
    harness.push(update_at("mock/rx/s1", "radio.rx.s1.sample_rate_hz", Value::num(2_000_000.0).unwrap(), UpdateClass::Cold, Some(now + 10_000_000)));
    harness.push(update_at("mock/rx/s1", "radio.rx.s1.sample_rate_hz", Value::num(4_000_000.0).unwrap(), UpdateClass::Cold, Some(now + 5_000_000)));
    harness.step(now).unwrap();
    assert_eq!(harness.clocks_of("mock/rx/s1").len(), 0);
    harness.step(now + 10_000_000).unwrap();
    assert_eq!(harness.rejected(), Vec::<String>::new());
    assert_eq!(
        harness.applied_of("radio.rx.s1.sample_rate_hz"),
        [(serde_json::json!(2_000_000.0), now + 10_000_000), (serde_json::json!(4_000_000.0), now + 10_000_000)],
    );
    let late: Vec<serde_json::Value> = harness.events().into_iter()
        .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::LATE_COMMAND)
        .map(|event| event.payload["key"].clone())
        .collect();
    assert_eq!(late, [serde_json::json!("radio.rx.s1.sample_rate_hz")]);
}

#[test]
fn mr_18_a_layout_hardware_timed_change_keeps_its_own_instant() {
    // RM-25: the floor is a `cold` change's alone. A `hardware_timed` update received after a
    // `cold` one takes effect at its own instant, earlier than that change's, and is not late
    // (MR-18).
    let constraints = [("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(4_000_000.0).unwrap()))];
    let mut harness = built(Layout { profile: "x3xx", id: "mock", rx: two(), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &constraints, env: &[], ports: &["rx0", "rx1"] });
    harness.arm_start(T0).unwrap();
    let now = T0 + 10_000_000;
    harness.push(update("radio.rx.s1.sample_rate_hz", Value::num(2_000_000.0).unwrap(), UpdateClass::Cold, Some(now + 20_000_000)));
    harness.push(update("radio.rx.c0.frequency_hz", Value::num(2_000_000_000.0).unwrap(), UpdateClass::HardwareTimed, Some(now + 10_000_000)));
    harness.step(now).unwrap();
    harness.step(now + 20_000_000).unwrap();
    assert_eq!(harness.rejected(), Vec::<String>::new());
    assert_eq!(harness.applied(), [
        ("radio.rx.c0.frequency_hz".to_owned(), now + 10_000_000),
        ("radio.rx.s1.sample_rate_hz".to_owned(), now + 20_000_000),
    ]);
    let late = harness.events().into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::LATE_COMMAND).count();
    assert_eq!(late, 0);
}

#[test]
fn mr_25_a_device_stop_stops_every_layout_stream() {
    // MR-29, D14: a `Stop` of `<device>` is every stream's. Both receive streams are cut at
    // its instant and both transmit streams lose their held bursts (RM-16, MR-25).
    let constraints = [
        ("radio.rx.c1.stream", eq(Scalar::from(1))),
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.c1.stream", eq(Scalar::from(1))),
    ];
    let mut harness = built(Layout { profile: "x3xx", id: "mock", rx: elements(&["0:A:ubx:0", "0:B:basic_rx:0"]), tx: elements(&["0:A:ubx:0", "0:B:basic_tx:0"]), selector: &[], constraints: &constraints, env: &[], ports: &["rx0", "rx1"] });
    harness.arm_start(T0).unwrap();
    let now = T0 + 10_000_000;
    harness.push(burst("mock/tx/s0", harness.clocks_of("mock/tx/s0")[0].domain, 3_000_000, 10));
    harness.push(burst("mock/tx/s1", harness.clocks_of("mock/tx/s1")[0].domain, 3_000_000, 10));
    harness.push(stop("mock"));
    harness.step(now).unwrap();
    let reasons = harness.rejected();
    assert_eq!(reasons.len(), 2, "{reasons:?}");
    assert!(reasons.iter().all(|reason| reason.starts_with("MR-25: cancelled by stop")), "{reasons:?}");
    for path in ["mock/rx/s0", "mock/rx/s1"] {
        let clocks = harness.clocks_of(path);
        assert_eq!(clocks.len(), 1, "{path}");
        assert_eq!(clocks[0].ended_at.map(|at| at.ticks_in(ROOT).unwrap()), Some(now), "{path}");
    }
    // Nothing is published after the stop.
    harness.blocks("rx0");
    harness.blocks("rx1");
    harness.step(now + 100_000_000).unwrap();
    assert!(harness.blocks("rx0").is_empty() && harness.blocks("rx1").is_empty());
}

#[test]
fn mr_20_a_lost_device_stops_every_layout_stream() {
    // MR-20: a device loss stops every stream of the instance: both receive streams end at
    // the loss's instant and both transmit streams lose their held bursts.
    let constraints = [
        ("radio.rx.c1.stream", eq(Scalar::from(1))),
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.c1.stream", eq(Scalar::from(1))),
    ];
    let env = [("sim.faults", serde_json::json!([{ "at_ns": 1_000_000, "fault": "device_lost", "target": "radio" }]))];
    let mut harness = built(Layout { profile: "x3xx", id: "mock", rx: elements(&["0:A:ubx:0", "0:B:basic_rx:0"]), tx: elements(&["0:A:ubx:0", "0:B:basic_tx:0"]), selector: &[], constraints: &constraints, env: &env, ports: &["rx0", "rx1"] });
    harness.arm_start(T0).unwrap();
    harness.push(burst("mock/tx/s0", harness.clocks_of("mock/tx/s0")[0].domain, 3_000_000, 10));
    harness.push(burst("mock/tx/s1", harness.clocks_of("mock/tx/s1")[0].domain, 3_000_000, 10));
    harness.step(T0).unwrap();
    let lost = T0 + 1_000_000;
    let error = harness.step(lost).expect_err("the device is lost");
    assert_eq!(error.kind, ezsdr_kernel::module_api::ModuleErrorKind::DeviceLost);
    let reasons = harness.rejected();
    assert_eq!(reasons.len(), 2, "{reasons:?}");
    assert!(reasons.iter().all(|reason| reason.starts_with("MR-20: device lost")), "{reasons:?}");
    for path in ["mock/rx/s0", "mock/rx/s1"] {
        let clocks = harness.clocks_of(path);
        assert_eq!(clocks.len(), 1, "{path}");
        assert_eq!(clocks[0].ended_at.map(|at| at.ticks_in(ROOT).unwrap()), Some(lost), "{path}");
    }
}

#[test]
fn mr_16_each_transmit_stream_is_judged_by_its_own_channel_count() {
    // MR-16, MR-13, RM-3: a burst is judged against its own stream's set, so one waveform is
    // another length on another stream, and a size that is no whole sample on one is one on
    // the other.
    let constraints = [
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.c1.stream", eq(Scalar::from(0))),
        ("radio.tx.c2.stream", eq(Scalar::from(1))),
        ("radio.tx.s0.sample_rate_hz", eq(Scalar::try_from(2_000_000.0).unwrap())),
        ("radio.tx.s1.sample_rate_hz", eq(Scalar::try_from(1_000_000.0).unwrap())),
    ];
    let mut harness = built(Layout { profile: "ideal", id: "mock", rx: count(1), tx: count(3), selector: &[], constraints: &constraints, env: &[], ports: &["rx0"] });
    harness.arm_start(0).unwrap();
    let (s0, s1) = (harness.clocks_of("mock/tx/s0")[0].domain, harness.clocks_of("mock/tx/s1")[0].domain);
    // Stream 0 holds channels 0 and 1, stream 1 channel 2 (RM-30). 400 bytes is 25 samples
    // on the two-channel stream and 50 on the one-channel one; 24 bytes is 3 and no whole
    // sample.
    harness.push(burst("mock/tx/s0", s0, 3_000, 50));
    harness.push(burst("mock/tx/s1", s1, 6_000, 50));
    harness.push(burst("mock/tx/s0", s0, 9_000, 3));
    harness.push(burst("mock/tx/s1", s1, 9_000, 3));
    harness.step(0).unwrap();
    harness.step(10_000_000).unwrap();
    let reasons = harness.rejected();
    assert_eq!(reasons.len(), 1, "{reasons:?}");
    assert!(reasons[0].starts_with("MR-16: target, clock, waveform size"), "{reasons:?}");
    let samples: Vec<i64> = harness.section("bursts").iter().map(|row| row["samples"].as_i64().unwrap()).collect();
    assert_eq!(samples, [25, 50, 3]);
}

