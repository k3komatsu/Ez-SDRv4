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

#[path = "../../ezsdr-acceptance/src/layout.rs"]
mod layout_support;

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

fn waveform_values(samples: &[(f32, f32)]) -> (ArtifactRef, Arc<[u8]>) {
    let mut bytes = Vec::with_capacity(samples.len() * 8);
    for (re, im) in samples {
        bytes.extend_from_slice(&re.to_le_bytes());
        bytes.extend_from_slice(&im.to_le_bytes());
    }
    let hash = ContentHash::of_bytes(&bytes);
    (ArtifactRef {
        id: Ident::parse("waveform").unwrap(),
        kind: Namespace::parse("ezsdr.input").unwrap(),
        uri: format!("mem:{hash}"),
        hash,
        size_bytes: bytes.len() as u64,
        partial: false,
        marks: Vec::new(),
        continuity: Vec::new(),
    }, bytes.into())
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
    harness_links_with_inputs(options, extra, BTreeMap::new())
}

fn harness_links_with_inputs(options: Layout<'_>, extra: &[(&str, BackPressure, usize)], inputs: BTreeMap<ContentHash, Arc<[u8]>>) -> Result<Harness, ModuleError> {
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
         ctx.inputs = Arc::new(inputs);
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

fn built_with_inputs(options: Layout<'_>, inputs: BTreeMap<ContentHash, Arc<[u8]>>) -> Harness {
    match harness_links_with_inputs(options, &[], inputs) {
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

fn block_sample(block: &BlockRef, channel: usize, sample: usize) -> (f32, f32) {
    let bytes = block.host_bytes().unwrap();
    let index = (channel * block.header().len as usize + sample) * 8;
    (
        f32::from_le_bytes(bytes[index..index + 4].try_into().unwrap()),
        f32::from_le_bytes(bytes[index + 4..index + 8].try_into().unwrap()),
    )
}

fn phase(rng: &mut ezsdr_sim::SimRng) -> f64 {
    2.0 * std::f64::consts::PI * ((rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64)
}

fn rotated(amplitude: f64, phase: f64) -> (f32, f32) {
    ((amplitude * phase.cos()) as f32, (amplitude * phase.sin()) as f32)
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

fn burst_with_waveform(target: &str, domain: ClockDomainId, sample: i64, waveform: ArtifactRef) -> Action {
    Action::TxBurst {
        target: tgt(target),
        waveform,
        repeat: false,
        at: AbsoluteDeadline::new(TimePoint::new(domain, sample)),
        requested_at: None,
        late_policy: LatePolicy::SendAsapAndFlag,
        metadata: BTreeMap::new(),
    }
}

/// The waveform of a two-channel transmit burst of `frames` samples, both channels at `value`:
/// interleaved by frame, so a stream of two channels reads it two values to a frame (MR-32).
fn two_channel_waveform(frames: usize, value: (f32, f32)) -> (ArtifactRef, Arc<[u8]>) {
    waveform_values(&vec![value; 2 * frames])
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
        let mut selector = layout_support::layout_selector("mock", rx, tx);
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
    // An absent selector key still selects the layout; the removed direction-wide key is
    // not part of its description.
    let requested = |name: &str, constraint: Constraint| Requested { resource: rid("mock"), constraints: BTreeMap::from([(key(name), constraint)]) };
    let layout_instance = MockRadio::from_binding(&layout(Some("channel"))).unwrap();
    let report = layout_instance.coerce(&requested("radio.rx.channels", eq(Scalar::from(1)))).unwrap();
    assert_eq!(report.rejected.len(), 1);
    assert!(report.rejected[0].reason.contains("RM-4: not a radio key"), "{:?}", report.rejected[0].reason);
    let without_key = MockRadio::from_binding(&layout(None)).unwrap();
    assert_eq!(without_key.instance().tree.children.iter().map(|child| child.id.path()).collect::<Vec<_>>(), ["mock/rx/s0", "mock/rx/s1", "mock/tx/s0"]);
    let report = without_key.coerce(&requested("radio.rx.channels", eq(Scalar::from(1)))).unwrap();
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
    harness.push(burst("mock", tx_domain, 3_100, 100));
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
    // A channel moving between streams is one `book_move`; the instance's configuration and
    // both stream plans now change together (RM-34, MR-40).
    harness.push(update("radio.rx.c0.stream", Value::from(1), UpdateClass::Cold, None));
    harness.step(T0 + 130_000_001).unwrap();
    let reasons = harness.rejected();
    assert_eq!(reasons.len(), 2, "{reasons:?}");
    assert_eq!(harness.applied().len(), 4);
    // The rate key belongs to its own stream: only that stream's clock changes (RM-27).
    harness.push(update("radio.rx.s0.sample_rate_hz", Value::num(2_000_000.0).unwrap(), UpdateClass::Cold, None));
    harness.step(T0 + 130_000_001).unwrap();
    harness.step(T0 + 300_000_000).unwrap();
    assert_eq!(harness.applied().len(), 5);
    let source = harness.clocks_of("mock/rx/s0");
    let destination = harness.clocks_of("mock/rx/s1");
    assert_eq!(source.len(), 1);
    assert!(source[0].ended_at.is_some());
    assert_eq!(destination.len(), 3);
    assert_eq!(destination[2].origin.ticks_in(ROOT).unwrap(), T0 + 185_000_000);
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
fn mr_31_a_layout_instance_joins_simulation_channel() {
    let env = [("sim.channel", serde_json::json!({ "couplings": [{ "tx": "radio", "tx_channel": 0, "rx": "radio", "rx_channel": 0, "gain_db": 0.0 }] }))];
    let constraints = [("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let (waveform, bytes) = waveform_values(&vec![(0.5, 0.0); 100]);
    let mut harness = built_with_inputs(Layout { profile: "ideal", id: "mock", rx: count(1), tx: count(1), selector: &[], constraints: &constraints, env: &env, ports: &["rx0"] }, BTreeMap::from([(waveform.hash.clone(), bytes)]));
    harness.arm_start(0).unwrap();
    assert_eq!(harness.mock.instance().fidelity.rf, ezsdr_kernel::module_api::RfFidelity::ImpairmentModel);
    let tx = harness.clocks_of("mock/tx/s0")[0].domain;
    harness.push(burst_with_waveform("mock/tx/s0", tx, 1_000, waveform));
    harness.step(0).unwrap();
    harness.step(10_000_000).unwrap();
    let heard: usize = harness.blocks("rx0").iter()
        .map(|block| (0..block.header().len as usize).filter(|&sample| block_sample(block, 0, sample) != (0.0, 0.0)).count())
        .sum();
    assert_eq!(heard, 100);
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
    // RM-25, RM-34: an update received after a reservation cannot overtake it, so a second
    // membership update is a move at the first update's effective instant.
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
    assert!(harness.rejected().is_empty());
    assert_eq!(harness.applied_of("radio.rx.c1.stream"), [
        (serde_json::json!(1), now + 20_000_000),
        (serde_json::json!(0), now + 20_000_000),
    ]);
    harness.step(now + 200_000_000).unwrap();
    assert!(harness.clocks_of("mock/rx/s1").is_empty());
    let s0 = harness.clocks_of("mock/rx/s0");
    assert_eq!(s0.len(), 2);
    assert_eq!(s0[1].origin.ticks_in(ROOT).unwrap(), now + 100_000_000);
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

#[test]
fn mr_40_a_receive_move_waits_for_release_and_lead_and_records_channel_order() {
    let constraints = [
        ("radio.rx.c1.stream", eq(Scalar::from(1))),
        ("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(4_000_000.0).unwrap())),
        ("radio.rx.s1.sample_rate_hz", eq(Scalar::try_from(1_000_000.0).unwrap())),
    ];
    let selector = [("rx_test_pattern", Value::from("ramp".to_owned()))];
    let mut harness = built(Layout { profile: "x3xx", id: "mock", rx: two(), tx: elements(&[]), selector: &selector, constraints: &constraints, env: &[], ports: &["rx0", "rx1"] });
    harness.arm_start(T0).unwrap();
    harness.step(T0 + 2_100_001).unwrap();
    let now = T0 + 10_000_000;
    harness.push(update("radio.rx.s0.sample_rate_hz", Value::num(2_000_000.0).unwrap(), UpdateClass::Cold, Some(now)));
    harness.push(update("radio.rx.c0.stream", Value::from(1), UpdateClass::Cold, Some(now)));
    harness.step(now).unwrap();
    assert_eq!(harness.rejected(), Vec::<String>::new());
    assert_eq!(harness.applied(), [
        ("radio.rx.s0.sample_rate_hz".to_owned(), now),
        ("radio.rx.c0.stream".to_owned(), now),
    ]);
    harness.step(now + 110_000_001).unwrap();
    let source = harness.clocks_of("mock/rx/s0");
    let destination = harness.clocks_of("mock/rx/s1");
    assert_eq!(source.len(), 1);
    assert_eq!(source[0].ended_at.unwrap().ticks_in(ROOT).unwrap(), now);
    assert_eq!(destination.len(), 2);
    assert_eq!(destination[0].ended_at.unwrap().ticks_in(ROOT).unwrap(), now);
    assert_eq!(destination[1].origin.ticks_in(ROOT).unwrap(), now + 103_500_000);
    let block = harness.blocks("rx1").into_iter()
        .find(|block| block.header().first_sample_time.domain() == destination[1].domain)
        .expect("the moved receive segment publishes on its new clock");
    assert_eq!(block.header().channels, 2);
    assert_eq!(block_sample(&block, 0, 1), (1.0 / 65_536.0, 0.0));
    assert_eq!(block_sample(&block, 1, 1), (1.0 / 65_536.0, 1.0 / 64.0));
    let segments = harness.section("segments");
    let source_segment = segments.iter().find(|row| row["direction"] == "rx" && row["stream"] == 0 && row["origin"]["ticks"] == T0 && row["channels"] == serde_json::json!([0])).unwrap();
    assert_eq!(source_segment["ended_at"]["ticks"], now);
    assert!(segments.iter().any(|row| row["direction"] == "rx" && row["stream"] == 0 && row["channels"] == serde_json::json!([0])));
    assert!(segments.iter().any(|row| row["direction"] == "rx" && row["stream"] == 1 && row["channels"] == serde_json::json!([1])));
    assert!(segments.iter().any(|row| row["direction"] == "rx" && row["stream"] == 1 && row["channels"] == serde_json::json!([0, 1])));
}

#[test]
fn mr_40_a_transmit_move_cuts_both_clocks_and_cancels_both_held_bursts() {
    let constraints = [
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.c1.stream", eq(Scalar::from(1))),
    ];
    let mut harness = built(Layout { profile: "ideal", id: "mock", rx: count(0), tx: count(2), selector: &[], constraints: &constraints, env: &[], ports: &[] });
    harness.arm_start(0).unwrap();
    let (s0, s1) = (harness.clocks_of("mock/tx/s0")[0].domain, harness.clocks_of("mock/tx/s1")[0].domain);
    harness.push(burst("mock/tx/s0", s0, 5_000, 100));
    harness.push(burst("mock/tx/s1", s1, 5_000, 100));
    harness.push(update("radio.tx.c0.stream", Value::from(1), UpdateClass::Cold, Some(1_000)));
    harness.step(0).unwrap();
    harness.step(1_000).unwrap();
    let reasons = harness.rejected();
    assert_eq!(reasons.len(), 2, "{reasons:?}");
    assert!(reasons.iter().all(|reason| reason.starts_with("MR-18: cancelled by a cold change")), "{reasons:?}");
    let s0 = harness.clocks_of("mock/tx/s0");
    let s1 = harness.clocks_of("mock/tx/s1");
    assert_eq!(s0.len(), 1);
    assert_eq!(s0[0].ended_at.unwrap().ticks_in(ROOT).unwrap(), 1_000);
    assert_eq!(s1.len(), 2);
    assert_eq!(s1[0].ended_at.unwrap().ticks_in(ROOT).unwrap(), 1_000);
    assert_eq!(s1[1].origin.ticks_in(ROOT).unwrap(), 1_000);
    let segments = harness.section("segments");
    assert!(segments.iter().any(|row| row["direction"] == "tx" && row["stream"] == 0 && row["channels"] == serde_json::json!([0])));
    assert!(segments.iter().any(|row| row["direction"] == "tx" && row["stream"] == 1 && row["channels"] == serde_json::json!([1])));
    assert!(segments.iter().any(|row| row["direction"] == "tx" && row["stream"] == 1 && row["channels"] == serde_json::json!([0, 1])));
}

#[test]
fn mr_40_a_move_uses_the_instance_floor_across_streams_directions_and_missing_links() {
    let constraints = [
        ("radio.rx.c1.stream", eq(Scalar::from(0))),
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
    ];
    let mut harness = built(Layout {
        profile: "x3xx", id: "mock", rx: mixed(), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &constraints, env: &[], ports: &["rx0", "rx1"],
    });
    harness.arm_start(T0).unwrap();
    let now = T0 + 10_000_000;
    harness.push(update("radio.rx.s2.sample_rate_hz", Value::num(2_000_000.0).unwrap(), UpdateClass::Cold, Some(now + 30_000_000)));
    harness.push(update("radio.rx.c0.stream", Value::from(1), UpdateClass::Cold, Some(now + 10_000_000)));
    harness.push(update("radio.tx.s0.sample_rate_hz", Value::num(2_000_000.0).unwrap(), UpdateClass::Cold, Some(now + 40_000_000)));
    harness.push(update("radio.rx.c1.stream", Value::from(1), UpdateClass::Cold, Some(now + 35_000_000)));
    harness.step(now).unwrap();
    harness.step(now + 40_000_000).unwrap();
    assert_eq!(harness.rejected(), Vec::<String>::new());
    assert_eq!(harness.applied_of("radio.rx.c0.stream"), [(serde_json::json!(1), now + 30_000_000)]);
    assert_eq!(harness.applied_of("radio.rx.c1.stream"), [(serde_json::json!(1), now + 40_000_000)]);
    assert_eq!(harness.applied_of("radio.rx.s2.sample_rate_hz"), [(serde_json::json!(2_000_000.0), now + 30_000_000)]);
    assert_eq!(harness.applied_of("radio.tx.s0.sample_rate_hz"), [(serde_json::json!(2_000_000.0), now + 40_000_000)]);
    harness.step(now + 160_000_001).unwrap();
    assert_eq!(harness.clocks_of("mock/rx/s0")[0].ended_at.unwrap().ticks_in(ROOT).unwrap(), now + 30_000_000);
    let moved = harness.clocks_of("mock/rx/s1");
    assert_eq!(moved.len(), 1);
    assert_eq!(moved[0].origin.ticks_in(ROOT).unwrap(), now + 135_000_000);
    let late: Vec<i64> = harness.events().into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::LATE_COMMAND)
        .map(|event| event.payload["applied"]["ticks"].as_i64().unwrap()).collect();
    assert_eq!(late, [now + 30_000_000, now + 40_000_000]);
    assert!(harness.clocks_of("mock/rx/s2").is_empty());
}

#[test]
fn mr_40_a_refused_move_applies_no_update_and_changes_no_manifest_or_clock() {
    let constraints = [("radio.rx.c1.stream", eq(Scalar::from(1)))];
    let mut harness = built(Layout { profile: "x3xx", id: "mock", rx: two(), tx: elements(&[]), selector: &[], constraints: &constraints, env: &[], ports: &["rx0", "rx1"] });
    harness.arm_start(T0).unwrap();
    let now = T0 + 2_100_001;
    harness.step(now).unwrap();
    let before_segments = harness.section("segments");
    harness.push(update("radio.rx.c0.stream", Value::from(1), UpdateClass::Cold, Some(i64::MAX)));
    harness.step(now + 1).unwrap();
    let reasons = harness.rejected();
    assert_eq!(reasons.len(), 1, "{reasons:?}");
    assert!(reasons[0].starts_with("RM-34: move of channel c0 from stream s0 to stream s1 was refused (Time("), "{reasons:?}");
    assert!(harness.applied().is_empty());
    assert_eq!(harness.section("segments"), before_segments);
    let command_rejected: Vec<_> = harness.events().into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).collect();
    assert_eq!(command_rejected.len(), 1);
    let s0 = harness.clocks_of("mock/rx/s0");
    let s1 = harness.clocks_of("mock/rx/s1");
    assert_eq!(s0.len(), 1);
    assert_eq!(s1.len(), 1);
    assert!(s0[0].ended_at.is_none() && s1[0].ended_at.is_none());
    let later = now + 10_000_000;
    harness.push(update("radio.rx.c0.stream", Value::from(-1), UpdateClass::Cold, Some(later)));
    harness.step(later).unwrap();
    assert_eq!(harness.clocks_of("mock/rx/s0")[0].ended_at.unwrap().ticks_in(ROOT).unwrap(), later + 999);
    assert!(harness.clocks_of("mock/rx/s1")[0].ended_at.is_none());
}

#[test]
fn mr_40_a_move_to_and_from_a_linkless_receive_stream_never_publishes_there() {
    let mut harness = built(Layout { profile: "ideal", id: "mock", rx: count(2), tx: count(0), selector: &[], constraints: &[], env: &[], ports: &["rx0"] });
    harness.arm_start(0).unwrap();
    harness.step(2_000_001).unwrap();
    let move_out = 3_000_000;
    harness.push(update("radio.rx.c0.stream", Value::from(1), UpdateClass::Cold, Some(move_out)));
    harness.step(move_out).unwrap();
    harness.step(move_out + 2_000_001).unwrap();
    assert!(harness.clocks_of("mock/rx/s1").is_empty());
    assert!(harness.clocks_of("mock/rx/s0")[0].ended_at.is_some());
    harness.push(start_rx("mock/rx/s1", Some(move_out + 10_000_000)));
    harness.step(move_out + 10_000_000).unwrap();
    harness.step(move_out + 12_000_001).unwrap();
    assert!(harness.clocks_of("mock/rx/s1").is_empty());
    let move_back = 20_000_000;
    harness.push(update("radio.rx.c0.stream", Value::from(0), UpdateClass::Cold, Some(move_back)));
    harness.step(move_back).unwrap();
    harness.step(move_back + 2_000_001).unwrap();
    let source = harness.clocks_of("mock/rx/s0");
    assert_eq!(source.len(), 2);
    assert_eq!(source[1].origin.ticks_in(ROOT).unwrap(), move_back);
    assert!(harness.clocks_of("mock/rx/s1").is_empty());
    assert!(harness.section("segments").iter().all(|row| !(row["direction"] == "rx" && row["stream"] == 1)));
    assert!(!harness.blocks("rx0").is_empty());
    assert_eq!(harness.rejected(), Vec::<String>::new());
}

#[test]
fn mr_32_a_logical_transmit_channel_follows_its_move_to_another_stream() {
    let constraints = [
        ("radio.rx.c2.stream", eq(Scalar::from(0))),
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.c2.stream", eq(Scalar::from(1))),
        ("radio.tx.c3.stream", eq(Scalar::from(0))),
    ];
    let env = [
        ("sim.seed", serde_json::json!(7)),
        ("sim.channel", serde_json::json!({"couplings":[{"tx":"radio","tx_channel":2,"rx":"radio","rx_channel":2,"gain_db":0.0,"delay_ns":0}]})),
    ];
    let (old_waveform, old_bytes) = waveform_values(&vec![(0.5, 0.0); 100]);
    let new_values: Vec<(f32, f32)> = (0..100).flat_map(|_| [(0.0, 0.0), (0.25, 0.0), (0.0, 0.0)]).collect();
    let (new_waveform, new_bytes) = waveform_values(&new_values);
    let inputs = BTreeMap::from([(old_waveform.hash.clone(), old_bytes), (new_waveform.hash.clone(), new_bytes)]);
    let mut harness = built_with_inputs(Layout { profile: "ideal", id: "mock", rx: count(3), tx: count(4), selector: &[], constraints: &constraints, env: &env, ports: &["rx0"] }, inputs);
    harness.arm_start(0).unwrap();
    let old_domain = harness.clocks_of("mock/tx/s1")[0].domain;
    harness.push(burst_with_waveform("mock/tx/s1", old_domain, 0, old_waveform));
    harness.push(update("radio.tx.c2.stream", Value::from(0), UpdateClass::Cold, Some(1_000_000)));
    harness.step(0).unwrap();
    let new_domain = harness.clocks_of("mock/tx/s0").last().unwrap().domain;
    harness.push(burst_with_waveform("mock/tx/s0", new_domain, 0, new_waveform));
    harness.step(1).unwrap();
    harness.step(2_000_001).unwrap();
    let block = harness.blocks("rx0").into_iter().next().unwrap();
    assert_eq!(block.header().channels, 2);
    assert_eq!(block_sample(&block, 1, 0), (0.5, 0.0));
    assert_eq!(block_sample(&block, 1, 1_000), (0.25, 0.0));
    let segments = harness.section("segments");
    assert!(segments.iter().any(|row| row["direction"] == "tx" && row["stream"] == 0 && row["channels"] == serde_json::json!([0, 3])));
    assert!(segments.iter().any(|row| row["direction"] == "tx" && row["stream"] == 0 && row["channels"] == serde_json::json!([0, 2, 3])));
    assert!(harness.rejected().is_empty());
}

#[test]
fn mr_32_a_layout_transmit_path_delay_silences_samples_before_the_antenna() {
    let constraints = [
        ("radio.rx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
        ("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(2_000_000.0).unwrap())),
    ];
    let env = [
        ("sim.channel", serde_json::json!({"couplings":[{"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0}]})),
    ];
    let (waveform, bytes) = waveform_values(&vec![(0.5, 0.0); 100]);
    let mut harness = built_with_inputs(Layout {
        profile: "x3xx", id: "mock", rx: elements(&["0:A:ubx:0"]), tx: elements(&["0:A:ubx:0"]),
        selector: &[], constraints: &constraints, env: &env, ports: &["rx0"],
    }, BTreeMap::from([(waveform.hash.clone(), bytes)]));
    assert_eq!(harness.mock.instance().tree.capabilities[&key("radio.tx.c0.path_delay_samples")], ezsdr_kernel::spec::CapabilityValue::One { value: Scalar::from(45) });
    harness.arm_start(T0).unwrap();
    harness.step(T0 + 2_100_001).unwrap();
    let now = T0 + 10_000_000;
    harness.push(update("radio.tx.s0.sample_rate_hz", Value::num(2_000_000.0).unwrap(), UpdateClass::Cold, Some(now)));
    harness.step(now).unwrap();
    let tx = harness.clocks_of("mock/tx/s0").last().unwrap().domain;
    let origin = harness.clocks_of("mock/tx/s0").last().unwrap().origin.ticks_in(ROOT).unwrap();
    harness.push(burst_with_waveform("mock/tx/s0", tx, 0, waveform));
    harness.step(now).unwrap();
    harness.step(origin + 1_000_001).unwrap();
    assert_eq!(harness.section("bursts")[0]["target"]["ticks"], 0);
    let rx = harness.clocks_of("mock/rx/s0").into_iter().next().unwrap();
    let first = match harness.clocks.convert(TimePoint::new(ROOT, origin), rx.domain).unwrap() {
        ezsdr_kernel::time::Converted::Exact { point } => point.ticks_in(rx.domain).unwrap(),
        ezsdr_kernel::time::Converted::Inexact { .. } => panic!("the RX clock shares the TX segment's lattice"),
    };
    let block = harness.blocks("rx0").into_iter().find(|block| {
        block.header().first_sample_time.domain() == rx.domain
            && block.header().first_sample_time.ticks_in(rx.domain).unwrap() == first
    }).expect("the RX stream publishes at the aligned TX origin");
    assert_eq!(match harness.clocks.convert(block.header().first_sample_time, ROOT).unwrap() {
        ezsdr_kernel::time::Converted::Exact { point } => point.ticks_in(ROOT).unwrap(),
        ezsdr_kernel::time::Converted::Inexact { .. } => panic!("the RX sample is on the TX origin"),
    }, origin);
    for sample in 0..45 {
        assert_eq!(block_sample(&block, 0, sample), (0.0, 0.0), "sample {sample} precedes the antenna arrival");
    }
    let (re, im) = block_sample(&block, 0, 45);
    assert!((f64::from(re).hypot(f64::from(im)) - 0.5).abs() < 1e-6, "sample 45 is TX sample 0 at the antenna: {re}+{im}i");
}

#[test]
fn mr_33_layout_gain_and_frequency_timelines_are_per_logical_channel() {
    let constraints = [
        ("radio.rx.c1.stream", eq(Scalar::from(1))),
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.c1.stream", eq(Scalar::from(1))),
    ];
    let env = [("sim.channel", serde_json::json!({"couplings":[
        {"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0},
        {"tx":"radio","tx_channel":1,"rx":"radio","rx_channel":1,"gain_db":0.0,"delay_ns":0}
    ]}))];
    let (waveform, bytes) = waveform_values(&vec![(0.5, 0.0); 100]);
    let mut harness = built_with_inputs(Layout { profile: "ideal", id: "mock", rx: count(2), tx: count(2), selector: &[], constraints: &constraints, env: &env, ports: &["rx0", "rx1"] }, BTreeMap::from([(waveform.hash.clone(), bytes)]));
    harness.arm_start(0).unwrap();
    let (tx0, tx1) = (harness.clocks_of("mock/tx/s0")[0].domain, harness.clocks_of("mock/tx/s1")[0].domain);
    for (name, value) in [
        ("radio.tx.c0.gain_db", 6.0), ("radio.rx.c0.gain_db", -6.0),
        ("radio.tx.c0.frequency_hz", 100.0), ("radio.rx.c0.frequency_hz", 100.0),
        ("radio.tx.c1.gain_db", 0.0), ("radio.rx.c1.gain_db", 6.0),
        ("radio.tx.c1.frequency_hz", 200.0), ("radio.rx.c1.frequency_hz", 200.0),
    ] {
        harness.push(update(name, Value::num(value).unwrap(), UpdateClass::HardwareTimed, None));
    }
    harness.push(burst_with_waveform("mock/tx/s0", tx0, 0, waveform.clone()));
    harness.push(burst_with_waveform("mock/tx/s1", tx1, 0, waveform));
    harness.step(0).unwrap();
    harness.step(2_000_001).unwrap();
    let rx0 = harness.blocks("rx0").into_iter().next().unwrap();
    let rx1 = harness.blocks("rx1").into_iter().next().unwrap();
    let (r0, i0) = block_sample(&rx0, 0, 0);
    let (r1, i1) = block_sample(&rx1, 0, 0);
    assert!((f64::from(r0) - 0.5).abs() < 1e-6 && i0.abs() < 1e-6, "{r0}+{i0}i");
    assert!((f64::from(r1) - 10f64.powf(6.0 / 20.0) * 0.5).abs() < 1e-6 && i1.abs() < 1e-6, "{r1}+{i1}i");
    assert!(harness.rejected().is_empty());
}

#[test]
fn mr_34_layout_mixed_boards_have_per_channel_timed_phase_behavior() {
    let constraints = [
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.c1.stream", eq(Scalar::from(1))),
        ("radio.rx.c1.stream", eq(Scalar::from(0))),
        ("radio.rx.c0.frequency_hz", eq(Scalar::try_from(1.0e9).unwrap())),
        ("radio.tx.c0.frequency_hz", eq(Scalar::try_from(1.0e9).unwrap())),
    ];
    let env = [
        ("sim.seed", serde_json::json!(7)),
        ("sim.channel", serde_json::json!({"couplings":[
            {"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0},
            {"tx":"radio","tx_channel":1,"rx":"radio","rx_channel":1,"gain_db":0.0,"delay_ns":0}
        ]})),
    ];
    let (waveform, bytes) = waveform_values(&vec![(0.5, 0.0); 10_000]);
    let mut harness = built_with_inputs(Layout {
        profile: "x3xx", id: "mock", rx: elements(&["0:A:ubx:0", "0:B:basic_rx:0"]),
        tx: elements(&["0:A:ubx:0", "0:B:basic_tx:0"]), selector: &[], constraints: &constraints, env: &env, ports: &["rx0"],
    }, BTreeMap::from([(waveform.hash.clone(), bytes)]));
    harness.arm_start(T0).unwrap();
    let (tx0, tx1) = (harness.clocks_of("mock/tx/s0")[0].domain, harness.clocks_of("mock/tx/s1")[0].domain);
    harness.push(burst_with_waveform("mock/tx/s0", tx0, 2_000_000, waveform.clone()));
    harness.push(burst_with_waveform("mock/tx/s1", tx1, 2_000_000, waveform));
    harness.step(0).unwrap();
    harness.step(T0 + 2_100_001).unwrap();
    let mut rng = ezsdr_sim::SimRng::new(7, "mock/lo");
    let rx_ubx = phase(&mut rng);
    let rx_basic = phase(&mut rng);
    let tx_ubx = phase(&mut rng);
    let tx_basic = phase(&mut rng);
    let rx_ubx_timed = phase(&mut rng);
    let rx_basic_timed = phase(&mut rng);
    let tx_ubx_timed = phase(&mut rng);
    let tx_basic_timed = phase(&mut rng);
    let first = harness.blocks("rx0").into_iter().next().unwrap();
    for (actual, expected) in [
        (block_sample(&first, 0, 100), rotated(0.5, tx_ubx - rx_ubx)),
        (block_sample(&first, 1, 100), rotated(0.5, tx_basic - rx_basic)),
    ] {
        assert!((actual.0 - expected.0).abs() < 1e-5 && (actual.1 - expected.1).abs() < 1e-5, "{actual:?} != {expected:?}");
    }
    let now = T0 + 3_000_000;
    let timed = T0 + 5_000_000;
    for (name, frequency) in [
        ("radio.tx.c0.frequency_hz", 1.1e9), ("radio.rx.c0.frequency_hz", 1.1e9),
        ("radio.tx.c1.frequency_hz", 10.0e6), ("radio.rx.c1.frequency_hz", 10.0e6),
    ] {
        harness.push(update(name, Value::num(frequency).unwrap(), UpdateClass::HardwareTimed, Some(timed)));
    }
    harness.step(now).unwrap();
    harness.step(T0 + 8_000_001).unwrap();
    let updated = harness.blocks("rx0").into_iter().find(|block| block.header().first_sample_time.ticks_in(block.header().first_sample_time.domain()).unwrap() == 6_000).unwrap();
    for (actual, expected) in [
        (block_sample(&updated, 0, 0), rotated(0.5, tx_ubx_timed - rx_ubx_timed)),
        (block_sample(&updated, 1, 0), rotated(0.5, tx_basic_timed - rx_basic_timed)),
    ] {
        assert!((actual.0 - expected.0).abs() < 1e-5 && (actual.1 - expected.1).abs() < 1e-5, "{actual:?} != {expected:?}");
    }
    let segment = harness.section("segments").into_iter().find(|row| row["direction"] == "rx" && row["stream"] == 0 && row["channels"] == serde_json::json!([0, 1])).unwrap();
    let coherence = segment["coherence"].as_array().unwrap();
    assert_eq!(coherence.first().unwrap()["basis"], "none");
    assert_eq!(coherence.first().unwrap()["valid"], false);
    assert_eq!(coherence.last().unwrap()["at"]["ticks"], timed);
    assert_eq!(coherence.last().unwrap()["basis"], "timed_tune");
    assert_eq!(coherence.last().unwrap()["valid"], true);
    assert!(harness.rejected().is_empty());
}

#[test]
fn mr_34_a_cbx_layout_redraws_only_the_retuned_channel() {
    let constraints = [
        ("radio.rx.c1.stream", eq(Scalar::from(1))),
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.c1.stream", eq(Scalar::from(1))),
    ];
    let env = [
        ("sim.seed", serde_json::json!(19)),
        ("sim.channel", serde_json::json!({"couplings":[
            {"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0},
            {"tx":"radio","tx_channel":1,"rx":"radio","rx_channel":1,"gain_db":0.0,"delay_ns":0}
        ]})),
    ];
    let (waveform, bytes) = waveform_values(&vec![(0.5, 0.0); 10_000]);
    let build = || built_with_inputs(Layout {
        profile: "x3xx", id: "mock", rx: elements(&["0:A:cbx:0", "0:B:cbx:0"]),
        tx: elements(&["0:A:cbx:0", "0:B:cbx:0"]), selector: &[], constraints: &constraints, env: &env, ports: &["rx0", "rx1"],
    }, BTreeMap::from([(waveform.hash.clone(), bytes.clone())]));
    let run = || {
        let mut harness = build();
        harness.arm_start(T0).unwrap();
        let (tx0, tx1) = (harness.clocks_of("mock/tx/s0")[0].domain, harness.clocks_of("mock/tx/s1")[0].domain);
        harness.push(burst_with_waveform("mock/tx/s0", tx0, 2_000_000, waveform.clone()));
        harness.push(burst_with_waveform("mock/tx/s1", tx1, 2_000_000, waveform.clone()));
        harness.step(0).unwrap();
        harness.step(T0 + 2_100_001).unwrap();
        let first0 = harness.blocks("rx0").into_iter().next().unwrap();
        let first1 = harness.blocks("rx1").into_iter().next().unwrap();
        let before = [block_sample(&first0, 0, 100), block_sample(&first1, 0, 100)];
        let now = T0 + 3_000_000;
        let timed = T0 + 5_000_000;
        harness.push(update("radio.tx.c1.frequency_hz", Value::num(2.46e9).unwrap(), UpdateClass::HardwareTimed, Some(timed)));
        harness.push(update("radio.rx.c1.frequency_hz", Value::num(2.46e9).unwrap(), UpdateClass::HardwareTimed, Some(timed)));
        harness.step(now).unwrap();
        harness.step(T0 + 8_000_001).unwrap();
        let updated0 = harness.blocks("rx0").into_iter().find(|block| block.header().first_sample_time.ticks_in(block.header().first_sample_time.domain()).unwrap() == 6_000).unwrap();
        let updated1 = harness.blocks("rx1").into_iter().find(|block| block.header().first_sample_time.ticks_in(block.header().first_sample_time.domain()).unwrap() == 6_000).unwrap();
        let after = [block_sample(&updated0, 0, 0), block_sample(&updated1, 0, 0)];
        (before, after, harness.mock.instance().sections.clone(), harness.section("segments"), harness.events())
    };
    let (before, after, sections, segments, events) = run();
    assert_eq!(run(), (before, after, sections.clone(), segments.clone(), events.clone()));
    let mut lo = ezsdr_sim::SimRng::new(19, "mock/lo");
    let rx0_initial = phase(&mut lo);
    let rx1_initial = phase(&mut lo);
    let tx0_initial = phase(&mut lo);
    let tx1_initial = phase(&mut lo);
    for (actual, expected) in [
        (before[0], rotated(0.5, tx0_initial - rx0_initial)),
        (before[1], rotated(0.5, tx1_initial - rx1_initial)),
    ] {
        assert!((actual.0 - expected.0).abs() < 1e-5 && (actual.1 - expected.1).abs() < 1e-5, "{actual:?} != {expected:?}");
    }
    let mut retune = ezsdr_sim::SimRng::new(19, "mock/lo/retune");
    let tx1_tuned = phase(&mut retune);
    let rx1_tuned = phase(&mut retune);
    for (actual, expected) in [
        (after[0], rotated(0.5, tx0_initial - rx0_initial)),
        (after[1], rotated(0.5, tx1_tuned - rx1_tuned)),
    ] {
        assert!((actual.0 - expected.0).abs() < 1e-5 && (actual.1 - expected.1).abs() < 1e-5, "{actual:?} != {expected:?}");
    }
    let rx1 = segments.iter().find(|row| row["direction"] == "rx" && row["stream"] == 1 && row["channels"] == serde_json::json!([1])).unwrap();
    assert_eq!(rx1["phase_behavior"], serde_json::json!(["random"]));
    let coherence = rx1["coherence"].as_array().unwrap();
    assert_eq!(coherence.last().unwrap()["basis"], "none");
    assert_eq!(coherence.last().unwrap()["valid"], false);
}

#[test]
fn mr_34_ideal_layout_channels_are_deterministic_and_reproduce_with_the_seed() {
    let constraints = [("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let env = [
        ("sim.seed", serde_json::json!(42)),
        ("sim.channel", serde_json::json!({"couplings":[{"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0}]})),
    ];
    let (waveform, bytes) = waveform_values(&vec![(0.25, 0.0); 100]);
    let build = || built_with_inputs(Layout { profile: "ideal", id: "mock", rx: count(1), tx: count(1), selector: &[], constraints: &constraints, env: &env, ports: &["rx0"] }, BTreeMap::from([(waveform.hash.clone(), bytes.clone())]));
    let run = |mut harness: Harness| {
        harness.arm_start(0).unwrap();
        let domain = harness.clocks_of("mock/tx/s0")[0].domain;
        harness.push(burst_with_waveform("mock/tx/s0", domain, 0, waveform.clone()));
        harness.step(0).unwrap();
        harness.step(1_999_001).unwrap();
        let block = harness.blocks("rx0").into_iter().next().unwrap();
        (block.header().clone(), block.host_bytes().unwrap().to_vec(), harness.mock.instance().sections.clone(), harness.events())
    };
    let left = run(build());
    let right = run(build());
    assert_eq!(left, right);
    assert_eq!((f32::from_le_bytes(left.1[0..4].try_into().unwrap()), f32::from_le_bytes(left.1[4..8].try_into().unwrap())), (0.25, 0.0));
}

/// The sample index of `clock` at root instant `tick`.
fn sample_of(clock: &SampleClockRecord, tick: i64) -> i64 {
    let ratio = clock.root_ticks_per_tick;
    i64::try_from(i128::from(tick - clock.origin.ticks_in(ROOT).unwrap()) * i128::from(ratio.den()) / i128::from(ratio.num())).unwrap()
}

/// The channel-0 sample a block of `clock` carries at root instant `tick`.
fn heard(blocks: &[BlockRef], clock: &SampleClockRecord, tick: i64) -> (f32, f32) {
    heard_channel(blocks, clock, tick, 0)
}

/// The sample of block channel `channel` a block of `clock` carries at root instant `tick`.
fn heard_channel(blocks: &[BlockRef], clock: &SampleClockRecord, tick: i64, channel: usize) -> (f32, f32) {
    let index = sample_of(clock, tick);
    let block = blocks.iter().find(|block| {
        let header = block.header();
        header.first_sample_time.domain() == clock.domain && {
            let first = header.first_sample_time.ticks_in(clock.domain).unwrap();
            (first..first + header.len as i64).contains(&index)
        }
    }).expect("a block holds the instant");
    let first = block.header().first_sample_time.ticks_in(clock.domain).unwrap();
    block_sample(block, channel, (index - first) as usize)
}

#[track_caller]
fn assert_near(actual: (f32, f32), expected: (f32, f32)) {
    assert!((actual.0 - expected.0).abs() < 1e-5 && (actual.1 - expected.1).abs() < 1e-5, "{actual:?} != {expected:?}");
}

#[test]
fn mr_34_a_layout_receive_entry_is_an_untimed_tune() {
    // A UBX receive channel is timed-tuned, leaves its stream and comes back: its new segment
    // is an untimed tune (`none`), coherent again only after the next timed tune.
    let constraints = [("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let env = [
        ("sim.seed", serde_json::json!(7)),
        ("sim.channel", serde_json::json!({"couplings":[{"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0}]})),
    ];
    let (waveform, bytes) = waveform_values(&vec![(0.5, 0.0); 150_000]);
    let mut harness = built_with_inputs(Layout {
        profile: "x3xx", id: "mock", rx: elements(&["0:A:ubx:0"]), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &constraints, env: &env, ports: &["rx0"],
    }, BTreeMap::from([(waveform.hash.clone(), bytes)]));
    harness.arm_start(T0).unwrap();
    let tx = harness.clocks_of("mock/tx/s0")[0].domain;
    harness.push(burst_with_waveform("mock/tx/s0", tx, 2_000_000, waveform));
    harness.step(0).unwrap();
    harness.step(T0 + 2_100_001).unwrap();
    let mut lo = ezsdr_sim::SimRng::new(7, "mock/lo");
    let (rx_phase, tx_phase, rx_timed) = (phase(&mut lo), phase(&mut lo), phase(&mut lo));
    let mut retune = ezsdr_sim::SimRng::new(7, "mock/lo/retune");
    let rx_entry = phase(&mut retune);
    assert_ne!(rx_entry, rx_timed);
    let first = harness.clocks_of("mock/rx/s0")[0].clone();
    harness.push(update("radio.rx.c0.frequency_hz", Value::num(1.0e9).unwrap(), UpdateClass::HardwareTimed, Some(T0 + 5_000_000)));
    harness.step(T0 + 3_000_000).unwrap();
    harness.push(update("radio.rx.c0.stream", Value::from(-1), UpdateClass::Cold, Some(T0 + 6_000_000)));
    harness.step(T0 + 5_000_000).unwrap();
    harness.step(T0 + 6_000_001).unwrap();
    harness.push(update("radio.rx.c0.stream", Value::from(0), UpdateClass::Cold, Some(T0 + 8_000_000)));
    harness.step(T0 + 7_000_000).unwrap();
    harness.step(T0 + 8_000_001).unwrap();
    harness.push(update("radio.rx.c0.frequency_hz", Value::num(1.0e9).unwrap(), UpdateClass::HardwareTimed, Some(T0 + 104_000_000)));
    harness.step(T0 + 100_000_000).unwrap();
    let entered = harness.clocks_of("mock/rx/s0").last().unwrap().clone();
    let origin = entered.origin.ticks_in(ROOT).unwrap();
    harness.step(T0 + 104_000_001).unwrap();
    harness.step(T0 + 108_000_001).unwrap();
    let blocks = harness.blocks("rx0");
    assert_near(heard(&blocks, &first, T0 + 100_000), rotated(0.5, tx_phase - rx_phase));
    assert_near(heard(&blocks, &first, T0 + 5_500_000), rotated(0.5, tx_phase - rx_timed));
    assert_near(heard(&blocks, &entered, origin + 100_000), rotated(0.5, tx_phase - rx_entry));
    assert_near(heard(&blocks, &entered, T0 + 106_000_000), rotated(0.5, tx_phase - rx_timed));
    let segments = harness.section("segments");
    let entry_row = segments.iter().find(|row| row["direction"] == "rx" && row["stream"] == 0 && row["origin"]["ticks"] == origin).unwrap();
    let coherence = entry_row["coherence"].as_array().unwrap();
    assert_eq!(coherence.first().unwrap()["basis"], "none");
    assert_eq!(coherence.first().unwrap()["valid"], false);
    assert_eq!(coherence.last().unwrap()["at"]["ticks"], T0 + 104_000_000);
    assert_eq!(coherence.last().unwrap()["basis"], "timed_tune");
    assert_eq!(coherence.last().unwrap()["valid"], true);
    let first_row = segments.iter().find(|row| row["direction"] == "rx" && row["stream"] == 0 && row["origin"]["ticks"] == T0).unwrap();
    let first_coherence = first_row["coherence"].as_array().unwrap();
    assert_eq!(first_coherence.last().unwrap()["at"]["ticks"], T0 + 5_000_000);
    assert_eq!(first_coherence.last().unwrap()["basis"], "timed_tune");
    assert_eq!(first_coherence.last().unwrap()["valid"], true);
    assert_eq!(harness.rejected(), Vec::<String>::new());
}

#[test]
fn mr_34_a_layout_untimed_tune_follows_a_timed_tune_at_its_instant() {
    // The receive entry above, with a timed frequency update at the re-entry's origin: the
    // untimed tune is taken after that update, so the channel carries the untimed phase and is
    // not coherent there (MR-34). A stream's origin is known only once it has begun, so the
    // scenario runs twice: once to read the origin, once with the update at it.
    let constraints = [("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let env = [
        ("sim.seed", serde_json::json!(7)),
        ("sim.channel", serde_json::json!({"couplings":[{"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0}]})),
    ];
    let run = |at_origin: Option<i64>| {
        let (waveform, bytes) = waveform_values(&vec![(0.5, 0.0); 150_000]);
        let mut harness = built_with_inputs(Layout {
            profile: "x3xx", id: "mock", rx: elements(&["0:A:ubx:0"]), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &constraints, env: &env, ports: &["rx0"],
        }, BTreeMap::from([(waveform.hash.clone(), bytes)]));
        harness.arm_start(T0).unwrap();
        let tx = harness.clocks_of("mock/tx/s0")[0].domain;
        harness.push(burst_with_waveform("mock/tx/s0", tx, 2_000_000, waveform));
        harness.step(0).unwrap();
        harness.step(T0 + 2_100_001).unwrap();
        harness.push(update("radio.rx.c0.frequency_hz", Value::num(1.0e9).unwrap(), UpdateClass::HardwareTimed, Some(T0 + 5_000_000)));
        harness.step(T0 + 3_000_000).unwrap();
        harness.push(update("radio.rx.c0.stream", Value::from(-1), UpdateClass::Cold, Some(T0 + 6_000_000)));
        harness.step(T0 + 5_000_000).unwrap();
        harness.step(T0 + 6_000_001).unwrap();
        harness.push(update("radio.rx.c0.stream", Value::from(0), UpdateClass::Cold, Some(T0 + 8_000_000)));
        harness.step(T0 + 7_000_000).unwrap();
        harness.step(T0 + 8_000_001).unwrap();
        if let Some(origin) = at_origin {
            harness.push(update("radio.rx.c0.frequency_hz", Value::num(1.0e9).unwrap(), UpdateClass::HardwareTimed, Some(origin)));
        }
        harness.step(T0 + 20_000_000).unwrap();
        harness.push(update("radio.rx.c0.frequency_hz", Value::num(1.0e9).unwrap(), UpdateClass::HardwareTimed, Some(T0 + 104_000_000)));
        harness.step(T0 + 100_000_000).unwrap();
        harness.step(T0 + 104_000_001).unwrap();
        harness.step(T0 + 108_000_001).unwrap();
        let entered = harness.clocks_of("mock/rx/s0").last().unwrap().clone();
        (entered.origin.ticks_in(ROOT).unwrap(), harness, entered)
    };
    let (origin, _, _) = run(None);
    let (rerun, harness, entered) = run(Some(origin));
    assert_eq!(rerun, origin);
    assert!(harness.applied().iter().any(|(key, at)| key == "radio.rx.c0.frequency_hz" && *at == origin));
    let mut lo = ezsdr_sim::SimRng::new(7, "mock/lo");
    let _rx_phase = phase(&mut lo);
    let tx_phase = phase(&mut lo);
    let rx_timed = phase(&mut lo);
    let rx_entry = phase(&mut ezsdr_sim::SimRng::new(7, "mock/lo/retune"));
    assert_ne!(rx_entry, rx_timed);
    let blocks = harness.blocks("rx0");
    assert_near(heard(&blocks, &entered, origin + 100_000), rotated(0.5, tx_phase - rx_entry));
    assert_near(heard(&blocks, &entered, T0 + 106_000_000), rotated(0.5, tx_phase - rx_timed));
    let segments = harness.section("segments");
    let entry_row = segments.iter().find(|row| row["direction"] == "rx" && row["stream"] == 0 && row["origin"]["ticks"] == origin).unwrap();
    let coherence = entry_row["coherence"].as_array().unwrap();
    let at_origin = coherence.iter().rfind(|entry| entry["at"]["ticks"] == origin).unwrap();
    assert_eq!(at_origin["basis"], "none");
    assert_eq!(at_origin["valid"], false);
    assert_eq!(harness.rejected(), Vec::<String>::new());
}

#[test]
fn mr_34_a_layout_transmit_entry_is_an_untimed_tune() {
    // The transmit side of the entry above: the channel leaves transmit stream 0 and comes
    // back on a new clock, and its first burst there radiates the untimed phase.
    let constraints = [("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let env = [
        ("sim.seed", serde_json::json!(7)),
        ("sim.channel", serde_json::json!({"couplings":[{"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0}]})),
    ];
    let (waveform, bytes) = waveform_values(&vec![(0.5, 0.0); 150_000]);
    let mut harness = built_with_inputs(Layout {
        profile: "x3xx", id: "mock", rx: elements(&["0:A:ubx:0"]), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &constraints, env: &env, ports: &["rx0"],
    }, BTreeMap::from([(waveform.hash.clone(), bytes)]));
    harness.arm_start(T0).unwrap();
    let tx = harness.clocks_of("mock/tx/s0")[0].domain;
    harness.push(burst_with_waveform("mock/tx/s0", tx, 2_000_000, waveform.clone()));
    harness.step(0).unwrap();
    harness.step(T0 + 2_100_001).unwrap();
    let mut lo = ezsdr_sim::SimRng::new(7, "mock/lo");
    let (rx_phase, tx_phase, _, tx_timed) = (phase(&mut lo), phase(&mut lo), phase(&mut lo), phase(&mut lo));
    let mut retune = ezsdr_sim::SimRng::new(7, "mock/lo/retune");
    let tx_entry = phase(&mut retune);
    assert_ne!(tx_entry, tx_timed);
    let first = harness.clocks_of("mock/rx/s0")[0].clone();
    harness.push(update("radio.tx.c0.frequency_hz", Value::num(1.0e9).unwrap(), UpdateClass::HardwareTimed, Some(T0 + 5_000_000)));
    harness.step(T0 + 3_000_000).unwrap();
    harness.push(update("radio.tx.c0.stream", Value::from(-1), UpdateClass::Cold, Some(T0 + 6_000_000)));
    harness.step(T0 + 5_000_000).unwrap();
    harness.step(T0 + 6_000_001).unwrap();
    harness.push(update("radio.tx.c0.stream", Value::from(0), UpdateClass::Cold, Some(T0 + 8_000_000)));
    harness.step(T0 + 7_000_000).unwrap();
    let entered = harness.clocks_of("mock/tx/s0").last().unwrap().clone();
    let origin = entered.origin.ticks_in(ROOT).unwrap();
    harness.push(burst_with_waveform("mock/tx/s0", entered.domain, sample_of(&entered, T0 + 80_000_000), waveform));
    harness.step(T0 + 8_000_001).unwrap();
    harness.push(update("radio.tx.c0.frequency_hz", Value::num(1.0e9).unwrap(), UpdateClass::HardwareTimed, Some(T0 + 104_000_000)));
    harness.step(T0 + 100_000_000).unwrap();
    harness.step(T0 + 104_000_001).unwrap();
    harness.step(T0 + 108_000_001).unwrap();
    let blocks = harness.blocks("rx0");
    assert_near(heard(&blocks, &first, T0 + 100_000), rotated(0.5, tx_phase - rx_phase));
    assert_near(heard(&blocks, &first, T0 + 5_500_000), rotated(0.5, tx_timed - rx_phase));
    assert_near(heard(&blocks, &first, T0 + 80_500_000), rotated(0.5, tx_entry - rx_phase));
    assert_near(heard(&blocks, &first, T0 + 106_000_000), rotated(0.5, tx_timed - rx_phase));
    let segments = harness.section("segments");
    let entry_row = segments.iter().find(|row| row["direction"] == "tx" && row["stream"] == 0 && row["origin"]["ticks"] == origin).unwrap();
    let coherence = entry_row["coherence"].as_array().unwrap();
    assert_eq!(coherence.first().unwrap()["basis"], "none");
    assert_eq!(coherence.first().unwrap()["valid"], false);
    assert_eq!(coherence.last().unwrap()["at"]["ticks"], T0 + 104_000_000);
    assert_eq!(coherence.last().unwrap()["basis"], "timed_tune");
    assert_eq!(coherence.last().unwrap()["valid"], true);
    let first_row = segments.iter().find(|row| row["direction"] == "tx" && row["stream"] == 0 && row["origin"]["ticks"] == 0).unwrap();
    let first_coherence = first_row["coherence"].as_array().unwrap();
    assert_eq!(first_coherence.last().unwrap()["at"]["ticks"], T0 + 5_000_000);
    assert_eq!(first_coherence.last().unwrap()["basis"], "timed_tune");
    assert_eq!(first_coherence.last().unwrap()["valid"], true);
}

#[test]
fn mr_34_a_layout_receive_move_is_an_untimed_tune_of_the_moved_channel() {
    // Channel 0 moves from receive stream 0 to stream 1, which holds channel 1: the origin of
    // stream 1's new segment is an untimed tune of both its channels, channel 0 first (MR-34).
    let constraints = [("radio.rx.c1.stream", eq(Scalar::from(1))), ("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let env = [
        ("sim.seed", serde_json::json!(7)),
        ("sim.channel", serde_json::json!({"couplings":[
            {"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0},
            {"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":1,"gain_db":0.0,"delay_ns":0}
        ]})),
    ];
    let (waveform, bytes) = waveform_values(&vec![(0.5, 0.0); 150_000]);
    let mut harness = built_with_inputs(Layout {
        profile: "x3xx", id: "mock", rx: two(), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &constraints, env: &env, ports: &["rx0", "rx1"],
    }, BTreeMap::from([(waveform.hash.clone(), bytes)]));
    harness.arm_start(T0).unwrap();
    let tx = harness.clocks_of("mock/tx/s0")[0].domain;
    harness.push(burst_with_waveform("mock/tx/s0", tx, 2_000_000, waveform));
    harness.step(0).unwrap();
    harness.step(T0 + 2_100_001).unwrap();
    let mut lo = ezsdr_sim::SimRng::new(7, "mock/lo");
    let (_, _, tx_phase, rx0_timed, _, _) = (phase(&mut lo), phase(&mut lo), phase(&mut lo), phase(&mut lo), phase(&mut lo), phase(&mut lo));
    let mut retune = ezsdr_sim::SimRng::new(7, "mock/lo/retune");
    let (rx0_moved, rx1_moved) = (phase(&mut retune), phase(&mut retune));
    assert_ne!(rx0_moved, rx0_timed);
    let source = harness.clocks_of("mock/rx/s0")[0].clone();
    harness.push(update("radio.rx.c0.frequency_hz", Value::num(1.0e9).unwrap(), UpdateClass::HardwareTimed, Some(T0 + 5_000_000)));
    harness.step(T0 + 3_000_000).unwrap();
    harness.push(update("radio.rx.c0.stream", Value::from(1), UpdateClass::Cold, Some(T0 + 6_000_000)));
    harness.step(T0 + 5_000_000).unwrap();
    harness.step(T0 + 6_000_001).unwrap();
    harness.push(update("radio.rx.c0.frequency_hz", Value::num(1.0e9).unwrap(), UpdateClass::HardwareTimed, Some(T0 + 104_000_000)));
    harness.push(update("radio.rx.c1.frequency_hz", Value::num(1.0e9).unwrap(), UpdateClass::HardwareTimed, Some(T0 + 104_000_000)));
    harness.step(T0 + 100_000_000).unwrap();
    assert_eq!(harness.clocks_of("mock/rx/s1").len(), 2);
    let moved = harness.clocks_of("mock/rx/s1").last().unwrap().clone();
    let origin = moved.origin.ticks_in(ROOT).unwrap();
    harness.step(T0 + 104_000_001).unwrap();
    harness.step(T0 + 108_000_001).unwrap();
    let blocks0 = harness.blocks("rx0");
    let blocks1 = harness.blocks("rx1");
    assert_near(heard(&blocks0, &source, T0 + 5_500_000), rotated(0.5, tx_phase - rx0_timed));
    assert_near(heard(&blocks1, &moved, origin + 100_000), rotated(0.5, tx_phase - rx0_moved));
    assert_near(heard_channel(&blocks1, &moved, origin + 100_000, 1), rotated(0.5, tx_phase - rx1_moved));
    assert_near(heard(&blocks1, &moved, T0 + 106_000_000), rotated(0.5, tx_phase - rx0_timed));
    let segments = harness.section("segments");
    let source_row = segments.iter().find(|row| row["direction"] == "rx" && row["stream"] == 0 && row["origin"]["ticks"] == T0).unwrap();
    assert_eq!(source_row["ended_at"]["ticks"], T0 + 6_000_000);
    assert!(source_row["coherence"].as_array().unwrap().iter().all(|entry| entry["at"]["ticks"].as_i64().unwrap() <= T0 + 6_000_000));
    assert_eq!(source_row["coherence"].as_array().unwrap().last().unwrap()["basis"], "timed_tune");
    let moved_row = segments.iter().find(|row| row["direction"] == "rx" && row["stream"] == 1 && row["origin"]["ticks"] == origin).unwrap();
    assert_eq!(moved_row["channels"], serde_json::json!([0, 1]));
    let coherence = moved_row["coherence"].as_array().unwrap();
    assert_eq!(coherence.first().unwrap()["basis"], "none");
    assert_eq!(coherence.first().unwrap()["valid"], false);
    assert_eq!(coherence.last().unwrap()["basis"], "timed_tune");
    assert_eq!(coherence.last().unwrap()["valid"], true);
    assert_eq!(harness.rejected(), Vec::<String>::new());
}

#[test]
fn mr_34_a_layout_stream_enable_is_an_untimed_tune_of_its_channels() {
    // A receive stream stopped and started again (`start_rx`) is enabled: its channel is
    // retuned untimed, as a channel entering the stream is.
    let constraints = [("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let env = [
        ("sim.seed", serde_json::json!(7)),
        ("sim.channel", serde_json::json!({"couplings":[{"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0}]})),
    ];
    let (waveform, bytes) = waveform_values(&vec![(0.5, 0.0); 150_000]);
    let mut harness = built_with_inputs(Layout {
        profile: "x3xx", id: "mock", rx: elements(&["0:A:ubx:0"]), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &constraints, env: &env, ports: &["rx0"],
    }, BTreeMap::from([(waveform.hash.clone(), bytes)]));
    harness.arm_start(T0).unwrap();
    let tx = harness.clocks_of("mock/tx/s0")[0].domain;
    harness.push(burst_with_waveform("mock/tx/s0", tx, 2_000_000, waveform));
    harness.step(0).unwrap();
    harness.step(T0 + 2_100_001).unwrap();
    let mut lo = ezsdr_sim::SimRng::new(7, "mock/lo");
    let (_, tx_phase, rx_timed) = (phase(&mut lo), phase(&mut lo), phase(&mut lo));
    let mut retune = ezsdr_sim::SimRng::new(7, "mock/lo/retune");
    let rx_enabled = phase(&mut retune);
    let first = harness.clocks_of("mock/rx/s0")[0].clone();
    harness.push(update("radio.rx.c0.frequency_hz", Value::num(1.0e9).unwrap(), UpdateClass::HardwareTimed, Some(T0 + 5_000_000)));
    harness.step(T0 + 3_000_000).unwrap();
    harness.push(stop("mock/rx/s0"));
    harness.step(T0 + 6_000_000).unwrap();
    harness.push(start_rx("mock/rx/s0", Some(T0 + 8_000_000)));
    harness.step(T0 + 7_000_000).unwrap();
    harness.step(T0 + 8_000_001).unwrap();
    harness.push(update("radio.rx.c0.frequency_hz", Value::num(1.0e9).unwrap(), UpdateClass::HardwareTimed, Some(T0 + 104_000_000)));
    harness.step(T0 + 100_000_000).unwrap();
    assert_eq!(harness.clocks_of("mock/rx/s0").len(), 2);
    let enabled = harness.clocks_of("mock/rx/s0").last().unwrap().clone();
    let origin = enabled.origin.ticks_in(ROOT).unwrap();
    harness.step(T0 + 104_000_001).unwrap();
    harness.step(T0 + 108_000_001).unwrap();
    let blocks = harness.blocks("rx0");
    assert_near(heard(&blocks, &first, T0 + 5_500_000), rotated(0.5, tx_phase - rx_timed));
    assert_near(heard(&blocks, &enabled, origin + 100_000), rotated(0.5, tx_phase - rx_enabled));
    assert_near(heard(&blocks, &enabled, T0 + 106_000_000), rotated(0.5, tx_phase - rx_timed));
    let segments = harness.section("segments");
    let row = segments.iter().find(|row| row["direction"] == "rx" && row["stream"] == 0 && row["origin"]["ticks"] == origin).unwrap();
    let coherence = row["coherence"].as_array().unwrap();
    assert_eq!(coherence.first().unwrap()["basis"], "none");
    assert_eq!(coherence.first().unwrap()["valid"], false);
    assert_eq!(coherence.last().unwrap()["at"]["ticks"], T0 + 104_000_000);
    assert_eq!(coherence.last().unwrap()["basis"], "timed_tune");
    assert_eq!(coherence.last().unwrap()["valid"], true);
}

#[test]
fn mr_34_a_layout_deterministic_channel_keeps_its_phase_and_coherence_through_entry() {
    // An `ideal` channel is deterministic: an entry takes no untimed phase and stays coherent.
    let constraints = [("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let env = [
        ("sim.seed", serde_json::json!(7)),
        ("sim.channel", serde_json::json!({"couplings":[{"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0}]})),
    ];
    let (waveform, bytes) = waveform_values(&vec![(0.25, 0.0); 100_000]);
    let mut harness = built_with_inputs(Layout { profile: "ideal", id: "mock", rx: count(1), tx: count(1), selector: &[], constraints: &constraints, env: &env, ports: &["rx0"] }, BTreeMap::from([(waveform.hash.clone(), bytes)]));
    harness.arm_start(0).unwrap();
    let tx = harness.clocks_of("mock/tx/s0")[0].domain;
    harness.push(burst_with_waveform("mock/tx/s0", tx, 0, waveform));
    harness.step(0).unwrap();
    harness.step(2_000_001).unwrap();
    let first = harness.clocks_of("mock/rx/s0")[0].clone();
    harness.push(update("radio.rx.c0.stream", Value::from(-1), UpdateClass::Cold, Some(6_000_000)));
    harness.step(5_000_000).unwrap();
    harness.step(6_000_001).unwrap();
    harness.push(update("radio.rx.c0.stream", Value::from(0), UpdateClass::Cold, Some(8_000_000)));
    harness.step(7_000_000).unwrap();
    harness.step(8_000_001).unwrap();
    harness.step(30_000_000).unwrap();
    let entered = harness.clocks_of("mock/rx/s0").last().unwrap().clone();
    let origin = entered.origin.ticks_in(ROOT).unwrap();
    let blocks = harness.blocks("rx0");
    assert_eq!(heard(&blocks, &first, 100_000), (0.25, 0.0));
    assert_eq!(heard(&blocks, &entered, origin + 100_000), (0.25, 0.0));
    let segments = harness.section("segments");
    let row = segments.iter().find(|row| row["direction"] == "rx" && row["stream"] == 0 && row["origin"]["ticks"] == origin).unwrap();
    let coherence = row["coherence"].as_array().unwrap();
    assert_eq!(coherence.first().unwrap()["basis"], "deterministic");
    assert_eq!(coherence.first().unwrap()["valid"], true);
}

#[test]
fn mr_34_a_layout_rate_change_is_an_untimed_tune() {
    // A stream's rate change begins a new segment, and its origin is an untimed tune of the channel
    // it keeps, though that channel was in the stream all along (MR-34).
    let constraints = [("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let env = [
        ("sim.seed", serde_json::json!(7)),
        ("sim.channel", serde_json::json!({"couplings":[{"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0}]})),
    ];
    let (waveform, bytes) = waveform_values(&vec![(0.5, 0.0); 150_000]);
    let mut harness = built_with_inputs(Layout {
        profile: "x3xx", id: "mock", rx: elements(&["0:A:ubx:0"]), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &constraints, env: &env, ports: &["rx0"],
    }, BTreeMap::from([(waveform.hash.clone(), bytes)]));
    harness.arm_start(T0).unwrap();
    let tx = harness.clocks_of("mock/tx/s0")[0].domain;
    harness.push(burst_with_waveform("mock/tx/s0", tx, 2_000_000, waveform));
    harness.step(0).unwrap();
    harness.step(T0 + 2_100_001).unwrap();
    let mut lo = ezsdr_sim::SimRng::new(7, "mock/lo");
    let (rx_phase, tx_phase, rx_timed) = (phase(&mut lo), phase(&mut lo), phase(&mut lo));
    let changed_phase = phase(&mut ezsdr_sim::SimRng::new(7, "mock/lo/retune"));
    assert_ne!(changed_phase, rx_timed);
    let first = harness.clocks_of("mock/rx/s0")[0].clone();
    harness.push(update("radio.rx.c0.frequency_hz", Value::num(1.0e9).unwrap(), UpdateClass::HardwareTimed, Some(T0 + 5_000_000)));
    harness.step(T0 + 3_000_000).unwrap();
    harness.push(update("radio.rx.s0.sample_rate_hz", Value::num(2_000_000.0).unwrap(), UpdateClass::Cold, Some(T0 + 7_000_000)));
    harness.step(T0 + 5_000_000).unwrap();
    harness.step(T0 + 7_000_001).unwrap();
    harness.step(T0 + 100_000_000).unwrap();
    assert_eq!(harness.clocks_of("mock/rx/s0").len(), 2);
    let changed = harness.clocks_of("mock/rx/s0").last().unwrap().clone();
    let origin = changed.origin.ticks_in(ROOT).unwrap();
    harness.step(T0 + 106_000_000).unwrap();
    let blocks = harness.blocks("rx0");
    assert_near(heard(&blocks, &first, T0 + 100_000), rotated(0.5, tx_phase - rx_phase));
    assert_near(heard(&blocks, &first, T0 + 5_500_000), rotated(0.5, tx_phase - rx_timed));
    assert_near(heard(&blocks, &changed, origin + 100_000), rotated(0.5, tx_phase - changed_phase));
    let segments = harness.section("segments");
    let row = segments.iter().find(|row| row["direction"] == "rx" && row["stream"] == 0 && row["origin"]["ticks"] == origin).unwrap();
    let coherence = row["coherence"].as_array().unwrap();
    assert_eq!(coherence.first().unwrap()["basis"], "none");
    assert_eq!(coherence.first().unwrap()["valid"], false);
    assert_eq!(harness.rejected(), Vec::<String>::new());
}

#[test]
fn mr_34_a_layout_move_out_is_an_untimed_tune_of_the_channels_left_behind() {
    // Channel 0 leaves receive stream 0, which keeps channel 2. Both streams' new segments begin
    // at one origin: stream 0's is an untimed tune of channel 2, drawn first, and stream 1's, which
    // takes channel 0, draws after it (MR-34: streams in number order at one origin).
    let ten_mhz = eq(Scalar::try_from(10.0e6).unwrap());
    let constraints = [
        ("radio.rx.c0.frequency_hz", ten_mhz.clone()), ("radio.rx.c1.stream", eq(Scalar::from(1))), ("radio.rx.c2.frequency_hz", ten_mhz.clone()),
        ("radio.rx.c2.stream", eq(Scalar::from(0))),
        ("radio.tx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.frequency_hz", ten_mhz),
    ];
    let env = [
        ("sim.seed", serde_json::json!(7)),
        ("sim.channel", serde_json::json!({"couplings":[
            {"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0},
            {"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":2,"gain_db":0.0,"delay_ns":0}
        ]})),
    ];
    let (waveform, bytes) = waveform_values(&vec![(0.5, 0.0); 150_000]);
    let mut harness = built_with_inputs(Layout {
        profile: "x3xx", id: "mock", rx: mixed(), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &constraints, env: &env, ports: &["rx0", "rx1"],
    }, BTreeMap::from([(waveform.hash.clone(), bytes)]));
    harness.arm_start(T0).unwrap();
    let tx = harness.clocks_of("mock/tx/s0")[0].domain;
    harness.push(burst_with_waveform("mock/tx/s0", tx, 2_000_000, waveform));
    harness.step(0).unwrap();
    harness.step(T0 + 2_100_001).unwrap();
    let mut lo = ezsdr_sim::SimRng::new(7, "mock/lo");
    let initial: Vec<f64> = (0..4).map(|_| phase(&mut lo)).collect();
    let timed: Vec<f64> = (0..4).map(|_| phase(&mut lo)).collect();
    let tx_phase = initial[3];
    let mut retune = ezsdr_sim::SimRng::new(7, "mock/lo/retune");
    let (left_phase, moved_phase) = (phase(&mut retune), phase(&mut retune));
    assert_ne!(left_phase, timed[2]);
    let source = harness.clocks_of("mock/rx/s0")[0].clone();
    harness.push(update("radio.rx.c2.frequency_hz", Value::num(10.0e6).unwrap(), UpdateClass::HardwareTimed, Some(T0 + 5_000_000)));
    harness.step(T0 + 3_000_000).unwrap();
    harness.push(update("radio.rx.c0.stream", Value::from(1), UpdateClass::Cold, Some(T0 + 6_000_000)));
    harness.step(T0 + 5_000_000).unwrap();
    harness.step(T0 + 6_000_001).unwrap();
    harness.step(T0 + 100_000_000).unwrap();
    assert_eq!(harness.clocks_of("mock/rx/s0").len(), 2);
    let left = harness.clocks_of("mock/rx/s0").last().unwrap().clone();
    let origin = left.origin.ticks_in(ROOT).unwrap();
    let moved = harness.clocks_of("mock/rx/s1").last().unwrap().clone();
    assert_eq!(moved.origin.ticks_in(ROOT).unwrap(), origin);
    harness.step(T0 + 106_000_000).unwrap();
    let blocks0 = harness.blocks("rx0");
    let blocks1 = harness.blocks("rx1");
    assert_near(heard(&blocks0, &left, origin + 100_000), rotated(0.5, tx_phase - left_phase));
    assert_near(heard(&blocks1, &moved, origin + 100_000), rotated(0.5, tx_phase - moved_phase));
    assert_near(heard(&blocks0, &source, T0 + 5_500_000), rotated(0.5, tx_phase - initial[0]));
    let segments = harness.section("segments");
    let row = segments.iter().find(|row| row["direction"] == "rx" && row["stream"] == 0 && row["origin"]["ticks"] == origin).unwrap();
    assert_eq!(row["channels"], serde_json::json!([2]));
    let coherence = row["coherence"].as_array().unwrap();
    assert_eq!(coherence.first().unwrap()["basis"], "none");
    assert_eq!(coherence.first().unwrap()["valid"], false);
    assert_eq!(harness.rejected(), Vec::<String>::new());
}

#[test]
fn mr_34_a_cbx_layout_entry_draws_a_fresh_phase() {
    // A `random` channel is never coherent: its entry draws a phase of its own, not the one its
    // last frequency tune drew.
    let two_ghz = eq(Scalar::try_from(2.0e9).unwrap());
    let constraints = [
        ("radio.rx.c0.frequency_hz", two_ghz.clone()), ("radio.rx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.c0.frequency_hz", two_ghz), ("radio.tx.c0.stream", eq(Scalar::from(0))),
    ];
    let env = [
        ("sim.seed", serde_json::json!(19)),
        ("sim.channel", serde_json::json!({"couplings":[{"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0}]})),
    ];
    let (waveform, bytes) = waveform_values(&vec![(0.5, 0.0); 150_000]);
    let mut harness = built_with_inputs(Layout {
        profile: "x3xx", id: "mock", rx: elements(&["0:A:cbx:0"]), tx: elements(&["0:A:cbx:0"]), selector: &[], constraints: &constraints, env: &env, ports: &["rx0"],
    }, BTreeMap::from([(waveform.hash.clone(), bytes)]));
    harness.arm_start(T0).unwrap();
    let tx = harness.clocks_of("mock/tx/s0")[0].domain;
    harness.push(burst_with_waveform("mock/tx/s0", tx, 2_000_000, waveform));
    harness.step(0).unwrap();
    harness.step(T0 + 2_100_001).unwrap();
    let mut lo = ezsdr_sim::SimRng::new(19, "mock/lo");
    let (rx_initial, tx_phase) = (phase(&mut lo), phase(&mut lo));
    let mut retune = ezsdr_sim::SimRng::new(19, "mock/lo/retune");
    let (tuned, entered_phase) = (phase(&mut retune), phase(&mut retune));
    assert_ne!(tuned, entered_phase);
    let first = harness.clocks_of("mock/rx/s0")[0].clone();
    harness.push(update("radio.rx.c0.frequency_hz", Value::num(2.0e9).unwrap(), UpdateClass::HardwareTimed, Some(T0 + 5_000_000)));
    harness.step(T0 + 3_000_000).unwrap();
    harness.push(update("radio.rx.c0.stream", Value::from(-1), UpdateClass::Cold, Some(T0 + 6_000_000)));
    harness.step(T0 + 5_000_000).unwrap();
    harness.step(T0 + 6_000_001).unwrap();
    harness.push(update("radio.rx.c0.stream", Value::from(0), UpdateClass::Cold, Some(T0 + 8_000_000)));
    harness.step(T0 + 7_000_000).unwrap();
    harness.step(T0 + 8_000_001).unwrap();
    harness.step(T0 + 100_000_000).unwrap();
    let entered = harness.clocks_of("mock/rx/s0").last().unwrap().clone();
    let origin = entered.origin.ticks_in(ROOT).unwrap();
    harness.step(T0 + 106_000_000).unwrap();
    let blocks = harness.blocks("rx0");
    assert_near(heard(&blocks, &first, T0 + 100_000), rotated(0.5, tx_phase - rx_initial));
    assert_near(heard(&blocks, &first, T0 + 5_500_000), rotated(0.5, tx_phase - tuned));
    assert_near(heard(&blocks, &entered, origin + 100_000), rotated(0.5, tx_phase - entered_phase));
    let segments = harness.section("segments");
    let row = segments.iter().find(|row| row["direction"] == "rx" && row["stream"] == 0 && row["origin"]["ticks"] == origin).unwrap();
    let coherence = row["coherence"].as_array().unwrap();
    assert_eq!(coherence.first().unwrap()["basis"], "none");
    assert_eq!(coherence.first().unwrap()["valid"], false);
}

#[test]
fn mr_34_a_layout_unlinked_receive_stream_takes_no_untimed_tune() {
    // Receive stream 1 has no link and holds channel 1. Its enable takes no untimed tune, so
    // the enable of stream 0 after it, which retunes channel 0 untimed, is the first draw of
    // the retune stream.
    let constraints = [("radio.rx.c1.stream", eq(Scalar::from(1))), ("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let env = [
        ("sim.seed", serde_json::json!(7)),
        ("sim.channel", serde_json::json!({"couplings":[{"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0}]})),
    ];
    let (waveform, bytes) = waveform_values(&vec![(0.5, 0.0); 150_000]);
    let mut harness = built_with_inputs(Layout {
        profile: "x3xx", id: "mock", rx: two(), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &constraints, env: &env, ports: &["rx0"],
    }, BTreeMap::from([(waveform.hash.clone(), bytes)]));
    harness.arm_start(T0).unwrap();
    let tx = harness.clocks_of("mock/tx/s0")[0].domain;
    harness.push(burst_with_waveform("mock/tx/s0", tx, 2_000_000, waveform));
    harness.step(0).unwrap();
    harness.step(T0 + 2_100_001).unwrap();
    let mut lo = ezsdr_sim::SimRng::new(7, "mock/lo");
    let (_, _, tx_phase) = (phase(&mut lo), phase(&mut lo), phase(&mut lo));
    let mut retune = ezsdr_sim::SimRng::new(7, "mock/lo/retune");
    let enabled = phase(&mut retune);
    harness.push(start_rx("mock/rx/s1", Some(T0 + 3_000_000)));
    harness.step(T0 + 3_000_000).unwrap();
    harness.push(stop("mock/rx/s0"));
    harness.step(T0 + 6_000_000).unwrap();
    harness.push(start_rx("mock/rx/s0", Some(T0 + 8_000_000)));
    harness.step(T0 + 7_000_000).unwrap();
    harness.step(T0 + 8_000_001).unwrap();
    harness.step(T0 + 100_000_000).unwrap();
    assert_eq!(harness.clocks_of("mock/rx/s0").len(), 2);
    let restarted = harness.clocks_of("mock/rx/s0").last().unwrap().clone();
    let origin = restarted.origin.ticks_in(ROOT).unwrap();
    harness.step(T0 + 106_000_000).unwrap();
    let blocks = harness.blocks("rx0");
    assert_near(heard(&blocks, &restarted, origin + 100_000), rotated(0.5, tx_phase - enabled));
    assert_eq!(harness.rejected(), Vec::<String>::new());
}

#[test]
fn mr_34_a_layout_deterministic_transmit_entry_keeps_phase_zero() {
    // The transmit side of the `ideal` entry: a deterministic transmit channel that leaves its
    // stream and comes back radiates with zero phase.
    let constraints = [("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let env = [
        ("sim.seed", serde_json::json!(7)),
        ("sim.channel", serde_json::json!({"couplings":[{"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0}]})),
    ];
    let (waveform, bytes) = waveform_values(&vec![(0.25, 0.0); 100_000]);
    let mut harness = built_with_inputs(Layout { profile: "ideal", id: "mock", rx: count(1), tx: count(1), selector: &[], constraints: &constraints, env: &env, ports: &["rx0"] }, BTreeMap::from([(waveform.hash.clone(), bytes)]));
    harness.arm_start(0).unwrap();
    let tx = harness.clocks_of("mock/tx/s0")[0].domain;
    harness.push(burst_with_waveform("mock/tx/s0", tx, 0, waveform.clone()));
    harness.step(0).unwrap();
    harness.step(2_000_001).unwrap();
    let first = harness.clocks_of("mock/rx/s0")[0].clone();
    harness.push(update("radio.tx.c0.stream", Value::from(-1), UpdateClass::Cold, Some(6_000_000)));
    harness.step(5_000_000).unwrap();
    harness.step(6_000_001).unwrap();
    harness.push(update("radio.tx.c0.stream", Value::from(0), UpdateClass::Cold, Some(8_000_000)));
    harness.step(7_000_000).unwrap();
    let entered = harness.clocks_of("mock/tx/s0").last().unwrap().clone();
    let origin = entered.origin.ticks_in(ROOT).unwrap();
    harness.push(burst_with_waveform("mock/tx/s0", entered.domain, sample_of(&entered, 30_000_000), waveform));
    harness.step(8_000_001).unwrap();
    harness.step(40_000_000).unwrap();
    let blocks = harness.blocks("rx0");
    assert_eq!(heard(&blocks, &first, 100_000), (0.25, 0.0));
    assert_eq!(heard(&blocks, &first, 36_000_000), (0.25, 0.0));
    let segments = harness.section("segments");
    let row = segments.iter().find(|row| row["direction"] == "tx" && row["stream"] == 0 && row["origin"]["ticks"] == origin).unwrap();
    let coherence = row["coherence"].as_array().unwrap();
    assert_eq!(coherence.first().unwrap()["basis"], "deterministic");
    assert_eq!(coherence.first().unwrap()["valid"], true);
}

#[test]
fn mr_34_a_layout_transmit_move_is_an_untimed_tune_of_the_moved_channel() {
    // Transmit channels 0 and 1 share stream 0, both timed tuned. Channel 1 moves into stream 1,
    // which holds no channel: it radiates the untimed phase there, and its segment is not
    // coherent. Channel 0 stays on stream 0, whose new clock begins at the move with an untimed
    // tune of it too, drawn first (MR-34: streams in number order at one origin).
    let constraints = [
        ("radio.rx.c0.stream", eq(Scalar::from(0))),
        ("radio.rx.c1.stream", eq(Scalar::from(0))),
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.c1.stream", eq(Scalar::from(0))),
        ("radio.rx.c1.frequency_hz", eq(Scalar::try_from(10.0e6).unwrap())),
    ];
    let env = [
        ("sim.seed", serde_json::json!(7)),
        ("sim.channel", serde_json::json!({"couplings":[
            {"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0},
            {"tx":"radio","tx_channel":1,"rx":"radio","rx_channel":1,"gain_db":0.0,"delay_ns":0}
        ]})),
    ];
    let (waveform, bytes) = waveform_values(&vec![(0.5, 0.0); 150_000]);
    let mut harness = built_with_inputs(Layout {
        profile: "x3xx", id: "mock", rx: elements(&["0:A:ubx:0", "0:B:basic_rx:0"]), tx: elements(&["0:A:ubx:0", "0:B:basic_tx:0"]), selector: &[], constraints: &constraints, env: &env, ports: &["rx0"],
    }, BTreeMap::from([(waveform.hash.clone(), bytes)]));
    harness.arm_start(T0).unwrap();
    harness.push(update("radio.tx.c0.frequency_hz", Value::num(1.0e9).unwrap(), UpdateClass::HardwareTimed, Some(T0 + 5_000_000)));
    harness.push(update("radio.tx.c1.frequency_hz", Value::num(10.0e6).unwrap(), UpdateClass::HardwareTimed, Some(T0 + 5_000_000)));
    harness.step(T0 + 3_000_000).unwrap();
    harness.push(update("radio.tx.c1.stream", Value::from(1), UpdateClass::Cold, Some(T0 + 10_000_000)));
    harness.step(T0 + 10_000_001).unwrap();
    harness.step(T0 + 30_000_000).unwrap();
    let s0 = harness.clocks_of("mock/tx/s0").last().unwrap().clone();
    let s1 = harness.clocks_of("mock/tx/s1").last().unwrap().clone();
    let (origin0, origin1) = (s0.origin.ticks_in(ROOT).unwrap(), s1.origin.ticks_in(ROOT).unwrap());
    assert!(origin0 > T0 + 10_000_000 && origin1 > T0 + 10_000_000);
    harness.push(burst_with_waveform("mock/tx/s0", s0.domain, sample_of(&s0, T0 + 150_000_000), waveform.clone()));
    harness.push(burst_with_waveform("mock/tx/s1", s1.domain, sample_of(&s1, T0 + 150_000_000), waveform));
    let blocks = collect_blocks(&mut harness, T0 + 40_000_000, T0 + 300_000_000);
    assert_eq!(harness.rejected(), Vec::<String>::new());
    let mut lo = ezsdr_sim::SimRng::new(7, "mock/lo");
    let rx = [phase(&mut lo), phase(&mut lo)];
    let _tx_initial = [phase(&mut lo), phase(&mut lo)];
    let _rx_timed = [phase(&mut lo), phase(&mut lo)];
    let tx_timed = [phase(&mut lo), phase(&mut lo)];
    let mut retune = ezsdr_sim::SimRng::new(7, "mock/lo/retune");
    let (stay, moved) = (phase(&mut retune), phase(&mut retune));
    assert_ne!(moved, tx_timed[1]);
    assert_ne!(stay, tx_timed[0]);
    assert_eq!(origin0, origin1);
    let rx_clock = harness.clocks_of("mock/rx/s0")[0].clone();
    let at = T0 + 160_000_000;
    assert_near(heard_channel(&blocks, &rx_clock, at, 0), rotated(0.5, stay - rx[0]));
    assert_near(heard_channel(&blocks, &rx_clock, at, 1), rotated(0.5, moved - rx[1]));
    let segments = harness.section("segments");
    let moved_row = segments.iter().find(|row| row["direction"] == "tx" && row["stream"] == 1 && row["origin"]["ticks"] == origin1).unwrap();
    assert_eq!(moved_row["channels"], serde_json::json!([1]));
    let coherence = moved_row["coherence"].as_array().unwrap();
    assert_eq!(coherence.last().unwrap()["basis"], "none");
    assert_eq!(coherence.last().unwrap()["valid"], false);
    let stay_row = segments.iter().find(|row| row["direction"] == "tx" && row["stream"] == 0 && row["origin"]["ticks"] == origin0).unwrap();
    assert_eq!(stay_row["channels"], serde_json::json!([0]));
    let coherence = stay_row["coherence"].as_array().unwrap();
    assert_eq!(coherence.last().unwrap()["basis"], "none");
    assert_eq!(coherence.last().unwrap()["valid"], false);
}

/// Steps `harness` from `from` to `to` in steps of 20 ms and collects the receive blocks of
/// `rx0` as they come: the link keeps only 64 blocks at a time.
fn collect_blocks(harness: &mut Harness, from: i64, to: i64) -> Vec<BlockRef> {
    let mut blocks = Vec::new();
    for tick in (from..=to).step_by(20_000_000) {
        harness.step(tick).unwrap();
        blocks.extend(harness.blocks("rx0"));
    }
    blocks
}

#[test]
fn mr_34_a_layout_transmit_rate_change_is_an_untimed_tune() {
    // Transmit channels 0 and 1 share stream 0, both timed tuned. A `cold` rate change replaces
    // the stream's clock, and the origin of the new clock is an untimed tune of both channels,
    // channel 0 first (MR-34).
    let constraints = [
        ("radio.rx.c0.stream", eq(Scalar::from(0))),
        ("radio.rx.c1.stream", eq(Scalar::from(0))),
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.c1.stream", eq(Scalar::from(0))),
        ("radio.tx.s0.sample_rate_hz", eq(Scalar::try_from(2_000_000.0).unwrap())),
        ("radio.rx.c1.frequency_hz", eq(Scalar::try_from(10.0e6).unwrap())),
    ];
    let env = [
        ("sim.seed", serde_json::json!(7)),
        ("sim.channel", serde_json::json!({"couplings":[
            {"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0},
            {"tx":"radio","tx_channel":1,"rx":"radio","rx_channel":1,"gain_db":0.0,"delay_ns":0}
        ]})),
    ];
    let (waveform, bytes) = two_channel_waveform(150_000, (0.5, 0.0));
    let mut harness = built_with_inputs(Layout {
        profile: "x3xx", id: "mock", rx: elements(&["0:A:ubx:0", "0:B:basic_rx:0"]), tx: elements(&["0:A:ubx:0", "0:B:basic_tx:0"]), selector: &[], constraints: &constraints, env: &env, ports: &["rx0"],
    }, BTreeMap::from([(waveform.hash.clone(), bytes)]));
    harness.arm_start(T0).unwrap();
    harness.push(update("radio.tx.c0.frequency_hz", Value::num(1.0e9).unwrap(), UpdateClass::HardwareTimed, Some(T0 + 5_000_000)));
    harness.push(update("radio.tx.c1.frequency_hz", Value::num(10.0e6).unwrap(), UpdateClass::HardwareTimed, Some(T0 + 5_000_000)));
    harness.step(T0 + 3_000_000).unwrap();
    harness.push(update_at("mock/tx/s0", "radio.tx.s0.sample_rate_hz", Value::num(1.0e6).unwrap(), UpdateClass::Cold, Some(T0 + 10_000_000)));
    harness.step(T0 + 10_000_001).unwrap();
    harness.step(T0 + 30_000_000).unwrap();
    let s0 = harness.clocks_of("mock/tx/s0").last().unwrap().clone();
    let origin = s0.origin.ticks_in(ROOT).unwrap();
    assert!(origin > T0 + 10_000_000);
    assert_eq!(s0.root_ticks_per_tick, Rational::new(1000, 1).unwrap());
    harness.push(burst_with_waveform("mock/tx/s0", s0.domain, sample_of(&s0, T0 + 150_000_000), waveform));
    let blocks = collect_blocks(&mut harness, T0 + 40_000_000, T0 + 300_000_000);
    assert_eq!(harness.rejected(), Vec::<String>::new());
    let mut lo = ezsdr_sim::SimRng::new(7, "mock/lo");
    let rx = [phase(&mut lo), phase(&mut lo)];
    let _tx_initial = [phase(&mut lo), phase(&mut lo)];
    let _rx_timed = [phase(&mut lo), phase(&mut lo)];
    let tx_timed = [phase(&mut lo), phase(&mut lo)];
    let mut retune = ezsdr_sim::SimRng::new(7, "mock/lo/retune");
    let tx_retuned = [phase(&mut retune), phase(&mut retune)];
    assert_ne!(tx_retuned[0], tx_timed[0]);
    assert_ne!(tx_retuned[1], tx_timed[1]);
    let rx_clock = harness.clocks_of("mock/rx/s0")[0].clone();
    let at = T0 + 160_000_000;
    assert_near(heard_channel(&blocks, &rx_clock, at, 0), rotated(0.5, tx_retuned[0] - rx[0]));
    assert_near(heard_channel(&blocks, &rx_clock, at, 1), rotated(0.5, tx_retuned[1] - rx[1]));
    let segment = harness.section("segments").into_iter().find(|row| row["direction"] == "tx" && row["stream"] == 0 && row["origin"]["ticks"] == origin).unwrap();
    assert_eq!(segment["channels"], serde_json::json!([0, 1]));
    let coherence = segment["coherence"].as_array().unwrap();
    assert_eq!(coherence.first().unwrap()["basis"], "none");
    assert_eq!(coherence.first().unwrap()["valid"], false);
}

#[test]
fn mr_34_a_layout_reentry_cut_before_its_start_leaves_the_rate_change_untimed() {
    // Channel 0 is timed tuned, leaves receive stream 0, and at one instant re-enters it and
    // changes its rate. The re-entry's segment is cut before it starts, so it never begins; the
    // rate change's segment keeps the channel, and its origin is an untimed tune of it (MR-34).
    let constraints = [("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let env = [
        ("sim.seed", serde_json::json!(7)),
        ("sim.channel", serde_json::json!({"couplings":[{"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0}]})),
    ];
    let (waveform, bytes) = waveform_values(&vec![(0.5, 0.0); 150_000]);
    let mut harness = built_with_inputs(Layout {
        profile: "x3xx", id: "mock", rx: elements(&["0:A:ubx:0"]), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &constraints, env: &env, ports: &["rx0"],
    }, BTreeMap::from([(waveform.hash.clone(), bytes)]));
    harness.arm_start(T0).unwrap();
    let tx = harness.clocks_of("mock/tx/s0")[0].domain;
    harness.push(burst_with_waveform("mock/tx/s0", tx, 2_000_000, waveform));
    harness.step(0).unwrap();
    harness.step(T0 + 2_100_001).unwrap();
    let mut lo = ezsdr_sim::SimRng::new(7, "mock/lo");
    let (_, tx_phase, rx_timed) = (phase(&mut lo), phase(&mut lo), phase(&mut lo));
    let changed_phase = phase(&mut ezsdr_sim::SimRng::new(7, "mock/lo/retune"));
    assert_ne!(changed_phase, rx_timed);
    let first = harness.clocks_of("mock/rx/s0")[0].clone();
    harness.push(update("radio.rx.c0.frequency_hz", Value::num(1.0e9).unwrap(), UpdateClass::HardwareTimed, Some(T0 + 5_000_000)));
    harness.step(T0 + 3_000_000).unwrap();
    harness.push(update("radio.rx.c0.stream", Value::from(-1), UpdateClass::Cold, Some(T0 + 6_000_000)));
    harness.step(T0 + 5_000_000).unwrap();
    harness.step(T0 + 6_000_001).unwrap();
    harness.push(update("radio.rx.c0.stream", Value::from(0), UpdateClass::Cold, Some(T0 + 8_000_000)));
    harness.push(update("radio.rx.s0.sample_rate_hz", Value::num(2_000_000.0).unwrap(), UpdateClass::Cold, Some(T0 + 8_000_000)));
    harness.step(T0 + 7_000_000).unwrap();
    harness.step(T0 + 8_000_001).unwrap();
    harness.step(T0 + 130_000_000).unwrap();
    assert_eq!(harness.clocks_of("mock/rx/s0").len(), 2);
    let changed = harness.clocks_of("mock/rx/s0").last().unwrap().clone();
    let origin = changed.origin.ticks_in(ROOT).unwrap();
    harness.step(T0 + 140_000_000).unwrap();
    let blocks = harness.blocks("rx0");
    assert_near(heard(&blocks, &first, T0 + 5_500_000), rotated(0.5, tx_phase - rx_timed));
    assert_near(heard(&blocks, &changed, origin + 100_000), rotated(0.5, tx_phase - changed_phase));
    let segments = harness.section("segments");
    let row = segments.iter().find(|row| row["direction"] == "rx" && row["stream"] == 0 && row["origin"]["ticks"] == origin).unwrap();
    assert_eq!(row["channels"], serde_json::json!([0]));
    let coherence = row["coherence"].as_array().unwrap();
    assert_eq!(coherence.first().unwrap()["basis"], "none");
    assert_eq!(coherence.first().unwrap()["valid"], false);
    assert_eq!(harness.rejected(), Vec::<String>::new());
}

#[test]
fn mr_34_a_layout_transmit_clock_cut_at_its_origin_takes_no_untimed_tune() {
    // Two transmit rate changes at one instant: the first clock is cut at its own origin, so it
    // never runs and takes no untimed tune. The second clock's origin is an untimed tune of both
    // channels of stream 0, which draw the first two phases of the retune stream (MR-34).
    let constraints = [
        ("radio.rx.c0.stream", eq(Scalar::from(0))),
        ("radio.rx.c1.stream", eq(Scalar::from(0))),
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.c1.stream", eq(Scalar::from(0))),
        ("radio.tx.s0.sample_rate_hz", eq(Scalar::try_from(2_000_000.0).unwrap())),
        ("radio.rx.c1.frequency_hz", eq(Scalar::try_from(10.0e6).unwrap())),
    ];
    let env = [
        ("sim.seed", serde_json::json!(7)),
        ("sim.channel", serde_json::json!({"couplings":[
            {"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0},
            {"tx":"radio","tx_channel":1,"rx":"radio","rx_channel":1,"gain_db":0.0,"delay_ns":0}
        ]})),
    ];
    let (waveform, bytes) = two_channel_waveform(150_000, (0.5, 0.0));
    let mut harness = built_with_inputs(Layout {
        profile: "x3xx", id: "mock", rx: elements(&["0:A:ubx:0", "0:B:basic_rx:0"]), tx: elements(&["0:A:ubx:0", "0:B:basic_tx:0"]), selector: &[], constraints: &constraints, env: &env, ports: &["rx0"],
    }, BTreeMap::from([(waveform.hash.clone(), bytes)]));
    harness.arm_start(T0).unwrap();
    harness.push(update("radio.tx.c0.frequency_hz", Value::num(1.0e9).unwrap(), UpdateClass::HardwareTimed, Some(T0 + 5_000_000)));
    harness.push(update("radio.tx.c1.frequency_hz", Value::num(10.0e6).unwrap(), UpdateClass::HardwareTimed, Some(T0 + 5_000_000)));
    harness.step(T0 + 3_000_000).unwrap();
    harness.push(update_at("mock/tx/s0", "radio.tx.s0.sample_rate_hz", Value::num(1.0e6).unwrap(), UpdateClass::Cold, Some(T0 + 10_000_000)));
    harness.push(update_at("mock/tx/s0", "radio.tx.s0.sample_rate_hz", Value::num(1.0e6).unwrap(), UpdateClass::Cold, Some(T0 + 10_000_000)));
    harness.step(T0 + 10_000_001).unwrap();
    harness.step(T0 + 30_000_000).unwrap();
    let clocks = harness.clocks_of("mock/tx/s0");
    let s0 = clocks.last().unwrap().clone();
    let origin = s0.origin.ticks_in(ROOT).unwrap();
    assert!(origin > T0 + 10_000_000);
    assert_eq!(s0.root_ticks_per_tick, Rational::new(1000, 1).unwrap());
    harness.push(burst_with_waveform("mock/tx/s0", s0.domain, sample_of(&s0, T0 + 150_000_000), waveform));
    let blocks = collect_blocks(&mut harness, T0 + 40_000_000, T0 + 300_000_000);
    assert_eq!(harness.rejected(), Vec::<String>::new());
    let mut lo = ezsdr_sim::SimRng::new(7, "mock/lo");
    let rx = [phase(&mut lo), phase(&mut lo)];
    let _tx_initial = [phase(&mut lo), phase(&mut lo)];
    let _rx_timed = [phase(&mut lo), phase(&mut lo)];
    let _tx_timed = [phase(&mut lo), phase(&mut lo)];
    let mut retune = ezsdr_sim::SimRng::new(7, "mock/lo/retune");
    let tx_retuned = [phase(&mut retune), phase(&mut retune)];
    let rx_clock = harness.clocks_of("mock/rx/s0")[0].clone();
    let at = T0 + 160_000_000;
    assert_near(heard_channel(&blocks, &rx_clock, at, 0), rotated(0.5, tx_retuned[0] - rx[0]));
    assert_near(heard_channel(&blocks, &rx_clock, at, 1), rotated(0.5, tx_retuned[1] - rx[1]));
    let segments = harness.section("segments");
    let row = segments.iter().find(|row| row["direction"] == "tx" && row["stream"] == 0 && row["origin"]["ticks"] == origin).unwrap();
    assert_eq!(row["channels"], serde_json::json!([0, 1]));
    let coherence = row["coherence"].as_array().unwrap();
    assert_eq!(coherence.first().unwrap()["basis"], "none");
    assert_eq!(coherence.first().unwrap()["valid"], false);
}

#[test]
fn mr_18_a_layout_receive_floor_at_t0_emits_late_command() {
    let mut harness = built(Layout { profile: "ideal", id: "mock", rx: count(1), tx: count(0), selector: &[], constraints: &[], env: &[], ports: &["rx0"] });
    harness.arm_start(T0).unwrap();
    harness.push(update("radio.rx.s0.sample_rate_hz", Value::num(2_000_000.0).unwrap(), UpdateClass::Cold, Some(T0 - 1)));
    harness.step(1).unwrap();
    let late: Vec<_> = harness.events().into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::LATE_COMMAND).collect();
    assert_eq!(late.len(), 1);
    assert_eq!(late[0].payload["requested"]["ticks"], T0 - 1);
    assert_eq!(late[0].payload["applied"]["ticks"], T0);
}

#[test]
fn mr_18_a_layout_rejects_an_out_of_range_stream_before_booking_a_move() {
    let mut harness = built(Layout { profile: "ideal", id: "mock", rx: count(2), tx: count(0), selector: &[], constraints: &[], env: &[], ports: &["rx0", "rx1"] });
    harness.arm_start(0).unwrap();
    harness.push(update("radio.rx.c0.stream", Value::from(2), UpdateClass::Cold, Some(1_000_000)));
    harness.step(1).unwrap();
    let reasons = harness.rejected();
    assert_eq!(reasons.len(), 1, "{reasons:?}");
    assert!(reasons[0].starts_with("MR-18: RM-8:"), "{reasons:?}");
    assert!(harness.applied().is_empty());
    assert!(harness.clocks_of("mock/rx/s0").is_empty() && harness.clocks_of("mock/rx/s1").is_empty());
}
