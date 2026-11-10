//! MockRadio's layout keys (design 33 D5; spec 09's MR-38…MR-41 with spec 07's
//! RM-27…RM-32): one stream node per logical channel, the
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
use ezsdr_kernel::module_api::Requested;
use ezsdr_kernel::module_api::{ActionReceiver, ActionSubmitter, AttachedPort, Endpoint, ExecutionClass, ModuleError, ModuleRef, Pacing, PrepareContext, ProfileRef, Provider, Role, Version};
use ezsdr_kernel::plan::Fragment;
use ezsdr_kernel::policy::{EventKindRegistry, Policy};
use ezsdr_kernel::spec::{Constraint, Ident, Key, Namespace, Scalar, Value};
use ezsdr_kernel::stream::{BackPressure, BlockFlags, BlockHeader, BlockRef, DataLink, DropCarry, LatePolicy, PublishOutcome};
use ezsdr_kernel::time::{AbsoluteDeadline, ClockDomain, ClockRegistry, Duration, EpochRef, ManualTimeAuthority, Rational, RelativeBudget, SampleClockRecord, TimePoint};

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

fn rid(name: &str) -> ResourceId {
    ResourceId::parse(name).unwrap()
}
fn key(name: &str) -> Key {
    Key::parse(name).unwrap()
}
fn eq(value: Scalar) -> Constraint {
    Constraint::Eq { value }
}

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

fn node_src(path: &str) -> EventSource {
    EventSource::Node { node: rid(path) }
}

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
    (
        ArtifactRef {
            id: Ident::parse("waveform").unwrap(),
            kind: Namespace::parse("ezsdr.input").unwrap(),
            uri: format!("mem:{hash}"),
            hash,
            size_bytes: bytes.len() as u64,
            partial: false,
            marks: Vec::new(),
            continuity: Vec::new(),
        },
        bytes.into(),
    )
}

#[derive(Default)]
struct Queue {
    actions: Mutex<VecDeque<Action>>,
    given: AtomicU64,
}

impl Queue {
    fn push(&self, action: Action) {
        self.actions.lock().unwrap().push_back(action);
    }
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
    fn submit(&self, _action: Action) -> Result<ActionId, Vec<Violation>> {
        Ok(ActionId(1))
    }
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
        if queue.len() < self.capacity {
            queue.push_back(block);
            return PublishOutcome::Accepted;
        }
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
    fn receive(&self) -> Option<(BlockRef, DropCarry)> {
        self.queue.lock().unwrap().pop_front().map(|b| (b, DropCarry::default()))
    }
    fn drops(&self) -> u64 {
        self.drops.load(Ordering::Relaxed)
    }
    fn take_drop_carry(&self) -> DropCarry {
        DropCarry::default()
    }
    fn policy(&self) -> BackPressure {
        self.policy
    }
}

/// The `rx` and `tx` selector values: element strings for `x3xx`, counts for `ideal`.
fn elements(list: &[&str]) -> Value {
    Value::List(list.iter().map(|element| Scalar::from(*element)).collect())
}
fn count(n: i64) -> Value {
    Value::from(n)
}

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
    build_layout(options, extra, inputs, true)
}

/// RM-30: no channel is in a stream unless the request puts it there. `rx0` has the runs that
/// are not about the default receive on c0 (so also the ones that name no stream) put it there.
fn build_layout(options: Layout<'_>, extra: &[(&str, BackPressure, usize)], inputs: BTreeMap<ContentHash, Arc<[u8]>>, rx0: bool) -> Result<Harness, ModuleError> {
    let version = if options.profile == "x3xx" { Version::new(0, 1, 0) } else { Version::new(2, 0, 0) };
    let mut selector =
        BTreeMap::from([(Ident::parse("id").unwrap(), Value::from(options.id.to_owned())), (Ident::parse("rx").unwrap(), options.rx.clone()), (Ident::parse("tx").unwrap(), options.tx.clone())]);
    for (name, value) in options.selector {
        selector.insert(Ident::parse(name).unwrap(), value.clone());
    }
    let binding = Binding { module: module_ref(), selector, profile: Some(ProfileRef { name: options.profile.to_owned(), version }), feed: None };
    let mock = MockRadio::from_binding(&binding)?;
    let streams: Vec<ResourceId> = mock.instance().tree.children.iter().map(|child| child.id.clone()).collect();
    let mut constraints = options.constraints.to_vec();
    if rx0 && mock.instance().tree.capabilities.contains_key(&key("radio.rx.c0.stream")) && !constraints.iter().any(|(name, _)| *name == "radio.rx.c0.stream") {
        constraints.push(("radio.rx.c0.stream", eq(Scalar::from(0))));
    }
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
    let pairs: Vec<_> = sources.into_iter().flat_map(|source| event_kinds.iter().cloned().map(move |kind| (source.clone(), kind))).collect();
    let events = Arc::new(EventCollector::new(&pairs, &event_kinds, 4096, &Policy::default()));
    let links: BTreeMap<String, Arc<MemLink>> = options.ports.iter().map(|port| ((*port).to_owned(), Arc::new(MemLink::new(BackPressure::DropOldest, 64)))).collect();
    let mut attached: Vec<_> = links.iter().map(|(port, link)| AttachedPort::new(Ident::parse("radio").unwrap(), Ident::parse(port).unwrap(), Endpoint::StreamOut(link.clone()))).collect();
    attached.extend(
        extra.iter().map(|(port, policy, capacity)| AttachedPort::new(Ident::parse("radio").unwrap(), Ident::parse(port).unwrap(), Endpoint::StreamOut(Arc::new(MemLink::new(*policy, *capacity))))),
    );
    let environment = Arc::new(options.env.iter().map(|(name, value)| (Namespace::parse(name).unwrap(), value.clone())).collect());
    let ctx = {
        let mut ctx = PrepareContext::testing(node_src(options.id), auth.clone(), clocks.clone(), events.clone(), actions.clone(), Arc::new(Accepting));
        ctx.run = RunId::from_string("mock-layout".to_owned());
        ctx.class = ExecutionClass::Simulation;
        ctx.environment = environment;
        ctx.inputs = Arc::new(inputs);
        ctx.links = attached;
        ctx.host_budget = RelativeBudget::new(Duration::new(HOST, 5_000_000_000)).unwrap();
        ctx
    };
    let requested = Requested { resource: rid(options.id), constraints: constraints.iter().map(|(name, constraint)| (key(name), constraint.clone())).collect() };
    let fragment =
        Fragment { id: Ident::parse("radio").unwrap(), instance: module_ref(), role: Role::Provider, content: serde_json::json!({ "selector": {}, "requested": requested }), after: Vec::new() };
    let prepared = mock.prepare(&fragment, ctx)?;
    Ok(Harness { mock, clocks, auth, events, actions, links, effective: prepared.effective })
}

fn built(options: Layout<'_>) -> Harness {
    match harness(options) {
        Ok(harness) => harness,
        Err(error) => panic!("prepare the layout Mock: {error}"),
    }
}

/// As [`built`], with no channel put in a stream: the layout's defaults themselves (RM-30).
fn built_defaults(options: Layout<'_>) -> Harness {
    build_layout(options, &[], BTreeMap::new(), false).unwrap_or_else(|error| panic!("prepare the layout Mock: {error}"))
}

fn built_with_inputs(options: Layout<'_>, inputs: BTreeMap<ContentHash, Arc<[u8]>>) -> Harness {
    match harness_links_with_inputs(options, &[], inputs) {
        Ok(harness) => harness,
        Err(error) => panic!("prepare the layout Mock: {error}"),
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

    fn push(&self, action: Action) {
        self.actions.push(action);
    }

    fn events(&self) -> Vec<Event> {
        self.events.drain(TimePoint::new(HOST, 0))
    }

    /// Every block a port received, in arrival order.
    fn blocks(&self, port: &str) -> Vec<BlockRef> {
        self.links[port].drain()
    }

    /// Every block header a port received.
    fn headers(&self, port: &str) -> Vec<BlockHeader> {
        self.blocks(port).iter().map(|block| block.header().clone()).collect()
    }

    /// Every SampleClock registered for the stream at `path`.
    fn clocks_of(&self, path: &str) -> Vec<SampleClockRecord> {
        let stream = rid(path);
        self.clocks.sample_clock_records().into_iter().filter(|record| record.stream == stream).collect()
    }

    /// The rows of one Manifest section (MR-27).
    fn section(&self, suffix: &str) -> Vec<serde_json::Value> {
        self.mock.instance().sections[&Namespace::parse(&format!("ezsdr.radio.mock.{suffix}")).unwrap()].as_array().cloned().unwrap_or_default()
    }

    /// The reasons of every refused, dropped or cancelled Action, in order (MR-27).
    fn rejected(&self) -> Vec<String> {
        self.section("rejected").iter().map(|row| row["reason"].as_str().unwrap_or_default().to_owned()).collect()
    }
}

fn block_sample(block: &BlockRef, channel: usize, sample: usize) -> (f32, f32) {
    let bytes = block.host_bytes().unwrap();
    let index = (channel * block.header().len as usize + sample) * 8;
    (f32::from_le_bytes(bytes[index..index + 4].try_into().unwrap()), f32::from_le_bytes(bytes[index + 4..index + 8].try_into().unwrap()))
}

fn phase(rng: &mut ezsdr_sim::SimRng) -> f64 {
    2.0 * std::f64::consts::PI * ((rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64)
}

fn rotated(amplitude: f64, phase: f64) -> (f32, f32) {
    ((amplitude * phase.cos()) as f32, (amplitude * phase.sin()) as f32)
}

fn stop(target: &str) -> Action {
    Action::Stop { target: Some(tgt(target)) }
}

fn start_rx(target: &str, at: Option<i64>) -> Action {
    Action::Command { target: tgt(target), verb: Ident::parse("start_rx").unwrap(), params: BTreeMap::new(), at: at.map(|tick| AbsoluteDeadline::new(TimePoint::new(ROOT, tick))) }
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

/// The mixed layout the owner uses: a UBX in slot A and a BasicRX in slot B, the BasicRX's
/// two front ends as two channels (design 33 §1).
fn mixed() -> Value {
    elements(&["0:A:ubx:0", "0:B:basic_rx:0", "0:B:basic_rx:1"])
}

/// The one receive stream a two-channel layout holds at its default membership.
fn two() -> Value {
    elements(&["0:A:ubx:0", "0:B:ubx:0"])
}

#[test]
fn mr_38_keys_is_an_unknown_selector_key() {
    let mut binding =
        Binding { module: module_ref(), selector: layout_support::ideal_selector("mock", 1, 0), profile: Some(ProfileRef { name: "ideal".to_owned(), version: Version::new(2, 0, 0) }), feed: None };
    binding.selector.insert(Ident::parse("keys").unwrap(), Value::from("channel"));
    let why = MockRadio::from_binding(&binding).err().expect("the `keys` selector is refused").message;
    assert_eq!(why, "MR-2: unsupported selector key `keys`");
}

#[test]
fn mr_38_the_tree_and_capabilities_are_the_layout_s() {
    let harness = built_defaults(Layout { profile: "x3xx", id: "mock", rx: mixed(), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &[], env: &[], ports: &["rx0"] });
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
    for stream in ["radio.rx.s0.sample_rate_hz", "radio.rx.s1.sample_rate_hz", "radio.rx.s2.sample_rate_hz"] {
        assert_eq!(tree.capabilities[&key(stream)], ezsdr_kernel::spec::CapabilityValue::AnyOf { values: decimations.clone() }, "{stream}");
    }
    // Transmit starts at 1 MS/s (KL-40): the first 200 decimations.
    assert_eq!(tree.capabilities[&key("radio.tx.s0.sample_rate_hz")], ezsdr_kernel::spec::CapabilityValue::AnyOf { values: decimations[..200].to_vec() });
    assert!(
        !tree.capabilities.contains_key(&key("radio.rx.channels")) && !tree.capabilities.contains_key(&key("radio.rx.antenna")) && !tree.capabilities.contains_key(&key("radio.rx.sample_rate_hz"))
    );
    assert!(!tree.capabilities.contains_key(&key("radio.rx.coherent")));
    assert_eq!(tree.capabilities[&key("radio.tx.repeat_align_samples")], one(Scalar::from(2)));
    // RM-30's defaults: every channel in no stream, each
    // front end's own default frequency and antenna, and every stream at 1 Msps.
    let default = |name: &str| harness.effective[&key(name)].clone();
    assert_eq!(default("radio.rx.c0.stream"), Value::from(-1));
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
    let expected: BTreeSet<Key> = ["c0", "c1", "c2"]
        .iter()
        .flat_map(|n| ["frequency_hz", "gain_db", "antenna", "stream"].map(|leaf| key(&format!("radio.rx.{n}.{leaf}"))))
        .chain(["radio.rx.s0.sample_rate_hz", "radio.rx.s1.sample_rate_hz", "radio.rx.s2.sample_rate_hz"].map(key))
        .chain(["radio.tx.c0.frequency_hz", "radio.tx.c0.gain_db", "radio.tx.c0.antenna", "radio.tx.c0.stream"].map(key))
        .chain(["radio.tx.s0.sample_rate_hz"].map(key))
        .collect();
    assert_eq!(harness.effective.keys().cloned().collect::<BTreeSet<_>>(), expected);

    // `ideal` has no motherboard: its channels are `ideal` front ends and its counts are
    // the selector's (RM-28, RM-29).
    let ideal = built_defaults(Layout { profile: "ideal", id: "mock", rx: count(2), tx: count(0), selector: &[], constraints: &[], env: &[], ports: &["rx0"] });
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
    let clocks: Vec<(String, i64, Rational)> =
        harness.clocks.sample_clock_records().into_iter().map(|record| (record.stream.path().to_owned(), record.origin.ticks_in(ROOT).unwrap(), record.root_ticks_per_tick)).collect();
    assert_eq!(clocks, [("mock/rx/s0".to_owned(), T0, Rational::new(100, 1).unwrap()), ("mock/rx/s1".to_owned(), T0, Rational::new(1000, 1).unwrap()),]);
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
fn mr_09_each_transmit_stream_has_its_own_clock_and_line() {
    // Two transmit channels, one stream each (a UBX in slot A and a BasicTX in slot B): at
    // `arm` each stream registers its own SampleClock and books its own line (MR-9), a burst
    // is held on the stream its own node names, and a `Stop` of one stream cancels only its
    // bursts (MR-12, MR-16, MR-25).
    let constraints = [
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.c1.stream", eq(Scalar::from(1))),
        ("radio.tx.s0.sample_rate_hz", eq(Scalar::try_from(2_000_000.0).unwrap())),
        ("radio.tx.s1.sample_rate_hz", eq(Scalar::try_from(1_000_000.0).unwrap())),
    ];
    let mut harness = built(Layout {
        profile: "x3xx",
        id: "mock",
        rx: elements(&["0:A:ubx:0"]),
        tx: elements(&["0:A:ubx:0", "0:B:basic_tx:0"]),
        selector: &[],
        constraints: &constraints,
        env: &[],
        ports: &["rx0"],
    });
    harness.arm_start(T0).unwrap();
    // 2 Msps: root ticks per sample 500 (a 1 GHz root and the 100 decimation); 1 Msps: 1 000.
    let clocks: Vec<(String, i64, Rational)> =
        harness.clocks.sample_clock_records().into_iter().map(|record| (record.stream.path().to_owned(), record.origin.ticks_in(ROOT).unwrap(), record.root_ticks_per_tick)).collect();
    assert_eq!(clocks, [("mock/tx/s0".to_owned(), 0, Rational::new(500, 1).unwrap()), ("mock/tx/s1".to_owned(), 0, Rational::new(1000, 1).unwrap()),]);
    // Each stream books a line of its own channels: its `segments` row names them (MR-27).
    let rows: Vec<_> = harness.section("segments").into_iter().filter(|row| row["direction"] == "tx").map(|row| (row["stream"].as_u64().unwrap(), row["channels"].clone())).collect();
    assert_eq!(rows, [(0, serde_json::json!([0])), (1, serde_json::json!([1]))], "{rows:?}");
    let s0 = harness.clocks_of("mock/tx/s0");
    let s1 = harness.clocks_of("mock/tx/s1");
    assert_eq!((s0.len(), s1.len()), (1, 1));
    // A burst on each stream's own node is held and transmitted on that stream: two records,
    // each of the ten samples of its own waveform (MR-16, MR-23).
    harness.push(burst("mock/tx/s0", s0[0].domain, 6_000_000, 10));
    harness.push(burst("mock/tx/s1", s1[0].domain, 4_000_000, 10));
    // Received now, held for their own instants: 3 s on stream 0 and 4 s on stream 1.
    harness.step(T0 + 200_000_000).unwrap();
    assert_eq!(harness.rejected(), Vec::<String>::new());
    harness.step(T0 + 3_000_000_000).unwrap();
    assert_eq!(harness.rejected(), Vec::<String>::new());
    let records = harness.section("bursts");
    assert_eq!(records.len(), 2, "{records:?}");
    assert_eq!(records.iter().map(|row| row["samples"].as_i64().unwrap()).collect::<Vec<_>>(), [10, 10]);
    // A `Stop` of stream 1 cancels the burst held on it and nothing of stream 0's, whose
    // burst is transmitted (MR-25).
    harness.push(burst("mock/tx/s1", s1[0].domain, 5_500_000, 10));
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
fn mr_16_two_transmit_streams_each_front_at_their_own_gap() {
    // RM-15: a one-sample head is padded to four and fronts at 4 plus the gap of its own stream's rate: stream 0 at 2 MS/s keeps 30 samples,
    // stream 1 at 1 MS/s keeps 15. A record armed with one stream's gap for both would refuse or accept the other's probe wrongly.
    let constraints = [
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.c1.stream", eq(Scalar::from(1))),
        ("radio.tx.s0.sample_rate_hz", eq(Scalar::try_from(2_000_000.0).unwrap())),
        ("radio.tx.s1.sample_rate_hz", eq(Scalar::try_from(1_000_000.0).unwrap())),
    ];
    let mut harness = built(Layout {
        profile: "x3xx",
        id: "mock",
        rx: elements(&["0:A:ubx:0"]),
        tx: elements(&["0:A:ubx:0", "0:B:basic_tx:0"]),
        selector: &[],
        constraints: &constraints,
        env: &[],
        ports: &["rx0"],
    });
    harness.arm_start(T0).unwrap();
    let (s0, s1) = (harness.clocks_of("mock/tx/s0")[0].domain, harness.clocks_of("mock/tx/s1")[0].domain);
    let (a0, a1) = (6_000_000, 4_000_000);
    harness.push(burst("mock/tx/s0", s0, a0, 1));
    harness.push(burst("mock/tx/s1", s1, a1, 1));
    // One sample before each front: refused; at each front: accepted.
    harness.push(burst("mock/tx/s0", s0, a0 + 4 + 29, 1));
    harness.push(burst("mock/tx/s1", s1, a1 + 4 + 14, 1));
    harness.push(burst("mock/tx/s0", s0, a0 + 4 + 30, 1));
    harness.push(burst("mock/tx/s1", s1, a1 + 4 + 15, 1));
    harness.step(T0 + 200_000_000).unwrap();
    let reasons = harness.rejected();
    assert_eq!(reasons.len(), 2, "{reasons:?}");
    assert!(reasons.iter().all(|reason| reason.contains("overlaps an accepted booking")), "{reasons:?}");
    // Which probes played pins which were refused: each head and the probe at its own stream's front, not the other stream's.
    harness.step(T0 + 3_000_000_000).unwrap();
    let mut targets: Vec<i64> = harness.section("bursts").iter().map(|row| row["target"]["ticks"].as_i64().unwrap()).collect();
    targets.sort_unstable();
    let mut expected = vec![a0, a1, a0 + 4 + 30, a1 + 4 + 15];
    expected.sort_unstable();
    assert_eq!(targets, expected, "the refused probes are s0 + 33 and s1 + 18, not the other way round");
}

#[test]
fn mr_09_a_burst_after_a_stop_targets_the_arm_clock() {
    // RM-25 (S11): a burst booked after a `Stop` of its transmit stream is on the clock registered at `arm`; the `Stop` registers none.
    let constraints = [("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let mut harness = built(Layout { profile: "x3xx", id: "mock", rx: elements(&[]), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &constraints, env: &[], ports: &[] });
    harness.arm_start(T0).unwrap();
    let s0 = harness.clocks_of("mock/tx/s0");
    assert_eq!(s0.len(), 1);
    harness.push(stop("mock/tx/s0"));
    harness.step(T0 + 100_000_000).unwrap();
    harness.push(burst("mock/tx/s0", s0[0].domain, 6_000_000, 10));
    harness.step(T0 + 200_000_000).unwrap();
    assert_eq!(harness.rejected(), Vec::<String>::new());
    assert_eq!(harness.clocks_of("mock/tx/s0").len(), 1);
    assert_eq!(harness.clocks_of("mock/tx/s0")[0].domain, s0[0].domain);
    harness.push(stop("mock/tx/s0"));
    harness.step(T0 + 300_000_000).unwrap();
    assert_eq!(harness.clocks_of("mock/tx/s0").len(), 1, "a second Stop registers no clock either");
}

#[test]
fn mr_12_d14_the_actions_of_a_layout_instance_name_a_stream() {
    let constraints = [("radio.rx.c1.stream", eq(Scalar::from(1))), ("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let mut harness = built(Layout { profile: "ideal", id: "mock", rx: count(2), tx: count(1), selector: &[], constraints: &constraints, env: &[], ports: &["rx0", "rx1"] });
    harness.arm_start(0).unwrap();
    let tx_domain = harness.clocks_of("mock/tx/s0")[0].domain;
    // D14: a burst names its transmit stream only (note 47 stage 2: no Run-time update names a stream).
    harness.push(burst("mock/tx/s0", tx_domain, 3_000, 100));
    harness.step(1).unwrap();
    assert_eq!(harness.rejected(), Vec::<String>::new());
    // The wrong destinations: the device for a burst, a receive stream for one, a stream
    // number the instance does not have, and the device for `start_rx`.
    harness.push(burst("mock", tx_domain, 3_100, 100));
    harness.push(burst("mock/rx/s0", tx_domain, 3_200, 100));
    harness.push(stop("mock/rx/s9"));
    harness.push(start_rx("mock", None));
    harness.step(2).unwrap();
    let reasons = harness.rejected();
    assert_eq!(reasons.len(), 4, "{reasons:?}");
    assert!(reasons[0].starts_with("MR-16:") && reasons[1].starts_with("MR-16:"));
    assert!(reasons[2].starts_with("MR-25: Stop target is not this device or its stream"));
    assert!(reasons[3].starts_with("MR-29: Action `command` is not supported"));
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
fn mr_7_each_receive_port_takes_its_own_links() {
    // MR-7 step 7: the one-Block-link rule is each port's own, not the instance's. Two
    // ports with one `Block` link each are admitted; two `Block` links on one port are
    // refused (the refusal is the port's).
    let links = [("rx0", BackPressure::Block, 1), ("rx1", BackPressure::Block, 1)];
    let ok = harness_links(
        Layout { profile: "x3xx", id: "mock", rx: two(), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &[("radio.rx.c1.stream", eq(Scalar::from(1)))], env: &[], ports: &[] },
        &links,
    );
    assert!(ok.is_ok(), "{:?}", ok.err().map(|error| error.message));
    let refused = [("rx0", BackPressure::Block, 1), ("rx0", BackPressure::Block, 1)];
    let why = harness_links(
        Layout { profile: "x3xx", id: "mock", rx: two(), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &[("radio.rx.c1.stream", eq(Scalar::from(1)))], env: &[], ports: &[] },
        &refused,
    )
    .err()
    .expect("two Block links on one port")
    .message;
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
    let mut harness = built_with_inputs(
        Layout { profile: "ideal", id: "mock", rx: count(1), tx: count(1), selector: &[], constraints: &constraints, env: &env, ports: &["rx0"] },
        BTreeMap::from([(waveform.hash.clone(), bytes)]),
    );
    harness.arm_start(0).unwrap();
    assert_eq!(harness.mock.instance().fidelity.rf, ezsdr_kernel::module_api::RfFidelity::ImpairmentModel);
    let tx = harness.clocks_of("mock/tx/s0")[0].domain;
    harness.push(burst_with_waveform("mock/tx/s0", tx, 1_000, waveform));
    harness.step(0).unwrap();
    harness.step(10_000_000).unwrap();
    let heard: usize = harness.blocks("rx0").iter().map(|block| (0..block.header().len as usize).filter(|&sample| block_sample(block, 0, sample) != (0.0, 0.0)).count()).sum();
    assert_eq!(heard, 100);
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
    let overflows: Vec<_> = harness
        .events()
        .into_iter()
        .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW)
        .map(|event| (event.source.clone(), RxOverflowPayload::from_payload(&event.payload).unwrap().lost))
        .collect();
    assert_eq!(overflows, [(node_src("mock/rx/s0"), 50_000)]);
    assert!(harness.headers("rx1").iter().all(|header| !header.flags.contains(BlockFlags::GAP_BEFORE)));
    // The lowest stream is not running there: the fault cuts the one that is.
    let mut alone =
        built(Layout { profile: "x3xx", id: "mock", rx: two(), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &[("radio.rx.c1.stream", eq(Scalar::from(1)))], env: &env, ports: &["rx1"] });
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
    let overflows: Vec<_> = alone.events().into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW).map(|event| event.source.clone()).collect();
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
    let errors: Vec<EventSource> = harness.events().into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).map(|event| event.source.clone()).collect();
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
fn mr_25_a_device_stop_stops_every_layout_stream() {
    // MR-29, D14: a `Stop` of `<device>` is every stream's. Both receive streams are cut at
    // its instant and both transmit streams lose their held bursts (RM-16, MR-25).
    let constraints = [("radio.rx.c1.stream", eq(Scalar::from(1))), ("radio.tx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c1.stream", eq(Scalar::from(1)))];
    let mut harness = built(Layout {
        profile: "x3xx",
        id: "mock",
        rx: elements(&["0:A:ubx:0", "0:B:basic_rx:0"]),
        tx: elements(&["0:A:ubx:0", "0:B:basic_tx:0"]),
        selector: &[],
        constraints: &constraints,
        env: &[],
        ports: &["rx0", "rx1"],
    });
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
    let constraints = [("radio.rx.c1.stream", eq(Scalar::from(1))), ("radio.tx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c1.stream", eq(Scalar::from(1)))];
    let env = [("sim.faults", serde_json::json!([{ "at_ns": 1_000_000, "fault": "device_lost", "target": "radio" }]))];
    let mut harness = built(Layout {
        profile: "x3xx",
        id: "mock",
        rx: elements(&["0:A:ubx:0", "0:B:basic_rx:0"]),
        tx: elements(&["0:A:ubx:0", "0:B:basic_tx:0"]),
        selector: &[],
        constraints: &constraints,
        env: &env,
        ports: &["rx0", "rx1"],
    });
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
fn mr_32_a_layout_transmit_path_delay_silences_samples_before_the_antenna() {
    let constraints = [
        ("radio.rx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
        ("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(2_000_000.0).unwrap())),
        ("radio.tx.s0.sample_rate_hz", eq(Scalar::try_from(2_000_000.0).unwrap())),
    ];
    let env = [("sim.channel", serde_json::json!({"couplings":[{"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0}]}))];
    let (waveform, bytes) = waveform_values(&vec![(0.5, 0.0); 100]);
    let mut harness = built_with_inputs(
        Layout { profile: "x3xx", id: "mock", rx: elements(&["0:A:ubx:0"]), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &constraints, env: &env, ports: &["rx0"] },
        BTreeMap::from([(waveform.hash.clone(), bytes)]),
    );
    assert_eq!(harness.mock.instance().tree.capabilities[&key("radio.tx.c0.path_delay_samples")], ezsdr_kernel::spec::CapabilityValue::One { value: Scalar::from(45) });
    harness.arm_start(T0).unwrap();
    harness.step(T0 + 2_100_001).unwrap();
    let now = T0 + 10_000_000;
    // The burst starts 10 ms ahead on the transmit clock the Session asked for: sample T0 + 20 ms at 2 Msps.
    let origin = T0 + 20_000_000;
    let tx = harness.clocks_of("mock/tx/s0").last().unwrap().domain;
    harness.push(burst_with_waveform("mock/tx/s0", tx, origin / 500, waveform));
    harness.step(now).unwrap();
    harness.step(origin + 1_000_001).unwrap();
    assert_eq!(harness.section("bursts")[0]["target"]["ticks"], origin / 500);
    let rx = harness.clocks_of("mock/rx/s0").into_iter().next().unwrap();
    let first = match harness.clocks.convert(TimePoint::new(ROOT, origin), rx.domain).unwrap() {
        ezsdr_kernel::time::Converted::Exact { point } => point.ticks_in(rx.domain).unwrap(),
        ezsdr_kernel::time::Converted::Inexact { .. } => panic!("the RX clock shares the TX segment's lattice"),
    };
    let block = harness
        .blocks("rx0")
        .into_iter()
        .find(|block| block.header().first_sample_time.domain() == rx.domain && block.header().first_sample_time.ticks_in(rx.domain).unwrap() == first)
        .expect("the RX stream publishes at the aligned TX origin");
    assert_eq!(
        match harness.clocks.convert(block.header().first_sample_time, ROOT).unwrap() {
            ezsdr_kernel::time::Converted::Exact { point } => point.ticks_in(ROOT).unwrap(),
            ezsdr_kernel::time::Converted::Inexact { .. } => panic!("the RX sample is on the TX origin"),
        },
        origin
    );
    for sample in 0..45 {
        assert_eq!(block_sample(&block, 0, sample), (0.0, 0.0), "sample {sample} precedes the antenna arrival");
    }
    let (re, im) = block_sample(&block, 0, 45);
    assert!((f64::from(re).hypot(f64::from(im)) - 0.5).abs() < 1e-6, "sample 45 is TX sample 0 at the antenna: {re}+{im}i");
}

#[test]
fn mr_33_layout_gain_and_frequency_timelines_are_per_logical_channel() {
    // The gains and frequencies are asked at the start, one pair per logical channel.
    let mut constraints = vec![("radio.rx.c1.stream", eq(Scalar::from(1))), ("radio.tx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c1.stream", eq(Scalar::from(1)))];
    for (name, value) in [
        ("radio.tx.c0.gain_db", 6.0),
        ("radio.rx.c0.gain_db", -6.0),
        ("radio.tx.c0.frequency_hz", 100.0),
        ("radio.rx.c0.frequency_hz", 100.0),
        ("radio.tx.c1.gain_db", 0.0),
        ("radio.rx.c1.gain_db", 6.0),
        ("radio.tx.c1.frequency_hz", 200.0),
        ("radio.rx.c1.frequency_hz", 200.0),
    ] {
        constraints.push((name, eq(Scalar::try_from(value).unwrap())));
    }
    let env = [(
        "sim.channel",
        serde_json::json!({"couplings":[
            {"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0},
            {"tx":"radio","tx_channel":1,"rx":"radio","rx_channel":1,"gain_db":0.0,"delay_ns":0}
        ]}),
    )];
    let (waveform, bytes) = waveform_values(&vec![(0.5, 0.0); 100]);
    let mut harness = built_with_inputs(
        Layout { profile: "ideal", id: "mock", rx: count(2), tx: count(2), selector: &[], constraints: &constraints, env: &env, ports: &["rx0", "rx1"] },
        BTreeMap::from([(waveform.hash.clone(), bytes)]),
    );
    harness.arm_start(0).unwrap();
    let (tx0, tx1) = (harness.clocks_of("mock/tx/s0")[0].domain, harness.clocks_of("mock/tx/s1")[0].domain);
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
fn mr_34_ideal_layout_channels_are_deterministic_and_reproduce_with_the_seed() {
    let constraints = [("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let env = [("sim.seed", serde_json::json!(42)), ("sim.channel", serde_json::json!({"couplings":[{"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0}]}))];
    let (waveform, bytes) = waveform_values(&vec![(0.25, 0.0); 100]);
    let build = || {
        built_with_inputs(
            Layout { profile: "ideal", id: "mock", rx: count(1), tx: count(1), selector: &[], constraints: &constraints, env: &env, ports: &["rx0"] },
            BTreeMap::from([(waveform.hash.clone(), bytes.clone())]),
        )
    };
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
    let block = blocks
        .iter()
        .find(|block| {
            let header = block.header();
            header.first_sample_time.domain() == clock.domain && {
                let first = header.first_sample_time.ticks_in(clock.domain).unwrap();
                (first..first + header.len as i64).contains(&index)
            }
        })
        .expect("a block holds the instant");
    let first = block.header().first_sample_time.ticks_in(clock.domain).unwrap();
    block_sample(block, channel, (index - first) as usize)
}

#[track_caller]
fn assert_near(actual: (f32, f32), expected: (f32, f32)) {
    assert!((actual.0 - expected.0).abs() < 1e-5 && (actual.1 - expected.1).abs() < 1e-5, "{actual:?} != {expected:?}");
}

#[test]
fn mr_34_a_layout_deterministic_channel_keeps_its_phase_and_coherence_through_entry() {
    // An `ideal` channel is deterministic: an entry takes no untimed phase and stays coherent.
    let constraints = [("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let env = [("sim.seed", serde_json::json!(7)), ("sim.channel", serde_json::json!({"couplings":[{"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0}]}))];
    let (waveform, bytes) = waveform_values(&vec![(0.25, 0.0); 100_000]);
    let mut harness = built_with_inputs(
        Layout { profile: "ideal", id: "mock", rx: count(1), tx: count(1), selector: &[], constraints: &constraints, env: &env, ports: &["rx0"] },
        BTreeMap::from([(waveform.hash.clone(), bytes)]),
    );
    harness.arm_start(0).unwrap();
    let tx = harness.clocks_of("mock/tx/s0")[0].domain;
    harness.push(burst_with_waveform("mock/tx/s0", tx, 0, waveform));
    harness.step(0).unwrap();
    harness.step(2_000_001).unwrap();
    let first = harness.clocks_of("mock/rx/s0")[0].clone();
    harness.step(5_000_000).unwrap();
    harness.push(stop("mock/rx/s0"));
    harness.step(6_000_001).unwrap();
    harness.push(start_rx("mock/rx/s0", Some(8_000_000)));
    harness.step(7_000_000).unwrap();
    harness.step(8_000_001).unwrap();
    harness.step(30_000_000).unwrap();
    assert_eq!(harness.rejected(), Vec::<String>::new(), "the Stop and `start_rx` are accepted");
    assert_eq!(harness.clocks_of("mock/rx/s0").len(), 2, "the entry is a new receive clock");
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
fn mr_34_a_layout_unlinked_receive_stream_takes_no_untimed_tune() {
    // Receive stream 1 has no link and holds channel 1. Its enable takes no untimed tune, so
    // the enable of stream 0 after it, which retunes channel 0 untimed, is the first draw of
    // the retune stream.
    let constraints = [("radio.rx.c1.stream", eq(Scalar::from(1))), ("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let env = [("sim.seed", serde_json::json!(7)), ("sim.channel", serde_json::json!({"couplings":[{"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0}]}))];
    let (waveform, bytes) = waveform_values(&vec![(0.5, 0.0); 150_000]);
    let mut harness = built_with_inputs(
        Layout { profile: "x3xx", id: "mock", rx: two(), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &constraints, env: &env, ports: &["rx0"] },
        BTreeMap::from([(waveform.hash.clone(), bytes)]),
    );
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
    // The `start_rx` on the stream with no link was accepted and carries out nothing: no clock, no segment row (D14).
    assert!(harness.clocks_of("mock/rx/s1").is_empty(), "{:?}", harness.clocks_of("mock/rx/s1"));
    assert!(!harness.section("segments").iter().any(|row| row["direction"] == "rx" && row["stream"] == 1), "{:?}", harness.section("segments"));
}

#[test]
fn mr_25_a_stream_stop_leaves_another_transmit_stream_radiating() {
    // MR-25, D14: transmit streams share one channel plan; a `Stop` of `<device>/tx/s0` cuts that stream's segments and
    // withdraws only its own later ones, so stream 1's held burst, starting after the cut, is heard.
    let constraints = [("radio.tx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c1.stream", eq(Scalar::from(1)))];
    let env = [("sim.channel", serde_json::json!({"couplings":[{"tx":"radio","tx_channel":1,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0}]}))];
    let (waveform, bytes) = waveform_values(&vec![(0.5, 0.0); 10_000]);
    let mut harness = built_with_inputs(
        Layout { profile: "ideal", id: "mock", rx: count(1), tx: count(2), selector: &[], constraints: &constraints, env: &env, ports: &["rx0"] },
        BTreeMap::from([(waveform.hash.clone(), bytes)]),
    );
    harness.arm_start(0).unwrap();
    let s1 = harness.clocks_of("mock/tx/s1")[0].domain;
    harness.push(burst_with_waveform("mock/tx/s1", s1, 5_000, waveform));
    harness.step(0).unwrap();
    harness.step(1_000_000).unwrap();
    harness.push(stop("mock/tx/s0"));
    harness.step(2_000_000).unwrap();
    harness.step(20_000_000).unwrap();
    let rx = harness.clocks_of("mock/rx/s0")[0].clone();
    let blocks = harness.blocks("rx0");
    assert_eq!(heard(&blocks, &rx, 6_000_000), (0.5, 0.0));
}

#[test]
fn mr_27_a_segment_row_holds_its_channel_order_and_its_end() {
    // MR-27: a segment's `channels` are in block order and its `ended_at` is the instant a `Stop` cut it.
    let constraints = [("radio.rx.c1.stream", eq(Scalar::from(0)))];
    let mut harness = built(Layout { profile: "ideal", id: "mock", rx: count(2), tx: count(1), selector: &[], constraints: &constraints, env: &[], ports: &["rx0"] });
    harness.arm_start(0).unwrap();
    harness.step(1_000_000).unwrap();
    harness.push(stop("mock/rx/s0"));
    harness.step(5_000_000).unwrap();
    harness.step(10_000_000).unwrap();
    let rows: Vec<_> = harness.section("segments").into_iter().filter(|row| row["direction"] == "rx" && row["stream"] == 0).collect();
    assert_eq!(rows[0]["channels"], serde_json::json!([0, 1]), "{rows:?}");
    assert_eq!(rows[0]["ended_at"]["ticks"].as_i64(), Some(5_000_000), "{rows:?}");
}

#[test]
fn mr_34_a_restart_redraws_each_random_channel_in_channel_order() {
    // MR-34: the untimed tune at a `start_rx` origin redraws every channel whose phase is not deterministic (CBX is
    // random), in the segment's channel order, from the retune stream.
    let constraints = [("radio.rx.c1.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let env = [
        ("sim.seed", serde_json::json!(7)),
        (
            "sim.channel",
            serde_json::json!({"couplings":[{"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0},{"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":1,"gain_db":0.0,"delay_ns":0}]}),
        ),
    ];
    let (waveform, bytes) = waveform_values(&vec![(0.5, 0.0); 150_000]);
    let mut harness = built_with_inputs(
        Layout { profile: "x3xx", id: "mock", rx: elements(&["0:A:cbx:0", "0:B:cbx:0"]), tx: elements(&["0:A:cbx:0"]), selector: &[], constraints: &constraints, env: &env, ports: &["rx0"] },
        BTreeMap::from([(waveform.hash.clone(), bytes)]),
    );
    harness.arm_start(T0).unwrap();
    let tx = harness.clocks_of("mock/tx/s0")[0].domain;
    harness.push(burst_with_waveform("mock/tx/s0", tx, 2_000_000, waveform));
    harness.step(0).unwrap();
    harness.step(T0 + 2_100_001).unwrap();
    let mut lo = ezsdr_sim::SimRng::new(7, "mock/lo");
    let (_, _, tx_phase) = (phase(&mut lo), phase(&mut lo), phase(&mut lo));
    let mut retune = ezsdr_sim::SimRng::new(7, "mock/lo/retune");
    let (first, second) = (phase(&mut retune), phase(&mut retune));
    harness.push(stop("mock/rx/s0"));
    harness.step(T0 + 6_000_000).unwrap();
    harness.push(start_rx("mock/rx/s0", Some(T0 + 8_000_000)));
    harness.step(T0 + 7_000_000).unwrap();
    harness.step(T0 + 100_000_000).unwrap();
    let restarted = harness.clocks_of("mock/rx/s0").last().unwrap().clone();
    let origin = restarted.origin.ticks_in(ROOT).unwrap();
    harness.step(T0 + 106_000_000).unwrap();
    let blocks = harness.blocks("rx0");
    assert_near(heard_channel(&blocks, &restarted, origin + 100_000, 0), rotated(0.5, tx_phase - first));
    assert_near(heard_channel(&blocks, &restarted, origin + 100_000, 1), rotated(0.5, tx_phase - second));
}

#[test]
fn mr_34_two_streams_restarted_at_one_origin_draw_in_stream_order() {
    // MR-34: when two receive streams restart at one origin their untimed tunes are drawn in stream order.
    let constraints = [("radio.rx.c1.stream", eq(Scalar::from(1))), ("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let env = [
        ("sim.seed", serde_json::json!(7)),
        (
            "sim.channel",
            serde_json::json!({"couplings":[{"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0},{"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":1,"gain_db":0.0,"delay_ns":0}]}),
        ),
    ];
    let (waveform, bytes) = waveform_values(&vec![(0.5, 0.0); 150_000]);
    let mut harness = built_with_inputs(
        Layout { profile: "x3xx", id: "mock", rx: elements(&["0:A:cbx:0", "0:B:cbx:0"]), tx: elements(&["0:A:cbx:0"]), selector: &[], constraints: &constraints, env: &env, ports: &["rx0", "rx1"] },
        BTreeMap::from([(waveform.hash.clone(), bytes)]),
    );
    harness.arm_start(T0).unwrap();
    let tx = harness.clocks_of("mock/tx/s0")[0].domain;
    harness.push(burst_with_waveform("mock/tx/s0", tx, 2_000_000, waveform));
    harness.step(0).unwrap();
    harness.step(T0 + 2_100_001).unwrap();
    let mut lo = ezsdr_sim::SimRng::new(7, "mock/lo");
    let (_, _, tx_phase) = (phase(&mut lo), phase(&mut lo), phase(&mut lo));
    let mut retune = ezsdr_sim::SimRng::new(7, "mock/lo/retune");
    let (first, second) = (phase(&mut retune), phase(&mut retune));
    harness.push(stop("mock/rx/s0"));
    harness.push(stop("mock/rx/s1"));
    harness.step(T0 + 6_000_000).unwrap();
    harness.push(start_rx("mock/rx/s0", Some(T0 + 80_000_000)));
    harness.push(start_rx("mock/rx/s1", Some(T0 + 80_000_000)));
    harness.step(T0 + 7_000_000).unwrap();
    harness.step(T0 + 100_000_000).unwrap();
    let (r0, r1) = (harness.clocks_of("mock/rx/s0").last().unwrap().clone(), harness.clocks_of("mock/rx/s1").last().unwrap().clone());
    assert_eq!(r0.origin, r1.origin, "one origin for both restarts");
    let origin = r0.origin.ticks_in(ROOT).unwrap();
    harness.step(T0 + 106_000_000).unwrap();
    let (b0, b1) = (harness.blocks("rx0"), harness.blocks("rx1"));
    assert_near(heard(&b0, &r0, origin + 100_000), rotated(0.5, tx_phase - first));
    assert_near(heard(&b1, &r1, origin + 100_000), rotated(0.5, tx_phase - second));
}

// ---------------------------------------------------------------- note 47 stage 3: receive values on a stopped stream

fn num(value: f64) -> Constraint {
    eq(Scalar::try_from(value).unwrap())
}

fn set_value(key: &str, value: f64) -> Action {
    Action::UpdateParameter { target: tgt("mock"), key: crate::key(key), value: Value::num(value).unwrap(), class: ezsdr_kernel::module_api::UpdateClass::Cold, at: None }
}

#[test]
fn mr_18_a_value_set_on_a_stopped_receive_stream_is_heard_by_its_next_segment_only() {
    // S1 on the channel model (CH-4: a receiver hears only its own frequency): the transmitter radiates at 2 GHz from the Run's start; the
    // receive channel is tuned to 1 GHz, hears nothing, is stopped, retuned to 2 GHz by a staged value and started: the old segment's samples are
    // silent, the new one's are the radiated pattern, and the old segment's `values` row keeps 1 GHz (MR-33, MR-18).
    let constraints = [
        ("radio.rx.c0.stream", eq(Scalar::from(0))),
        ("radio.rx.c1.stream", eq(Scalar::from(0))),
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.c0.frequency_hz", num(2.0e9)),
        ("radio.rx.c0.frequency_hz", num(1.0e9)),
        ("radio.rx.c1.frequency_hz", num(1.0e9)),
    ];
    let env = [(
        "sim.channel",
        serde_json::json!({"couplings":[{"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0},{"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":1,"gain_db":0.0,"delay_ns":0}]}),
    )];
    let (waveform, bytes) = waveform_values(&vec![(0.5, 0.25); 1_000]);
    let mut harness = built_with_inputs(
        Layout { profile: "ideal", id: "mock", rx: count(2), tx: count(1), selector: &[], constraints: &constraints, env: &env, ports: &["rx0"] },
        BTreeMap::from([(waveform.hash.clone(), bytes)]),
    );
    harness.arm_start(0).unwrap();
    let tx = harness.clocks_of("mock/tx/s0")[0].domain;
    let mut repeating = burst_with_waveform("mock/tx/s0", tx, 0, waveform);
    if let Action::TxBurst { repeat, .. } = &mut repeating {
        *repeat = true;
    }
    harness.push(repeating);
    harness.step(0).unwrap();
    harness.step(3_000_000).unwrap();
    harness.push(stop("mock/rx/s0"));
    harness.push(set_value("radio.rx.c0.frequency_hz", 2.0e9));
    harness.push(start_rx("mock/rx/s0", None));
    harness.step(4_000_000).unwrap();
    harness.step(8_000_000).unwrap();
    assert_eq!(harness.rejected(), Vec::<String>::new());
    let clocks = harness.clocks_of("mock/rx/s0");
    assert_eq!(clocks.len(), 2);
    let blocks = harness.blocks("rx0");
    let origin = |clock: &SampleClockRecord| clock.origin.ticks_in(ROOT).unwrap();
    assert_eq!(heard(&blocks, &clocks[0], origin(&clocks[0]) + 1_000_000), (0.0, 0.0), "the old segment is tuned to 1 GHz");
    let second = origin(&clocks[1]);
    assert_eq!(heard_channel(&blocks, &clocks[1], second + 100_000, 0), (0.5, 0.25), "the new segment is tuned to 2 GHz from its first sample");
    assert_eq!(heard_channel(&blocks, &clocks[1], second + 100_000, 1), (0.0, 0.0), "another channel keeps its value");
    let rows: Vec<_> = harness.section("segments").into_iter().filter(|row| row["direction"] == "rx").collect();
    assert_eq!((rows[0]["values"]["c0"]["frequency_hz"].as_f64(), rows[1]["values"]["c0"]["frequency_hz"].as_f64()), (Some(1.0e9), Some(2.0e9)));
    assert_eq!(rows[1]["values"]["c1"]["frequency_hz"].as_f64(), Some(1.0e9));
}

#[test]
fn mr_18_a_refused_value_is_not_heard_by_the_next_segment() {
    // S2 on the channel model: the value sent while the stream streams is refused, so after a Stop and a start the segment still hears nothing.
    let constraints = [("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.frequency_hz", num(2.0e9)), ("radio.rx.c0.frequency_hz", num(1.0e9))];
    let env = [("sim.channel", serde_json::json!({"couplings":[{"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0}]}))];
    let (waveform, bytes) = waveform_values(&vec![(0.5, 0.25); 1_000]);
    let mut harness = built_with_inputs(
        Layout { profile: "ideal", id: "mock", rx: count(1), tx: count(1), selector: &[], constraints: &constraints, env: &env, ports: &["rx0"] },
        BTreeMap::from([(waveform.hash.clone(), bytes)]),
    );
    harness.arm_start(0).unwrap();
    let tx = harness.clocks_of("mock/tx/s0")[0].domain;
    let mut repeating = burst_with_waveform("mock/tx/s0", tx, 0, waveform);
    if let Action::TxBurst { repeat, .. } = &mut repeating {
        *repeat = true;
    }
    harness.push(repeating);
    harness.step(0).unwrap();
    harness.push(set_value("radio.rx.c0.frequency_hz", 2.0e9));
    harness.step(3_000_000).unwrap();
    assert_eq!(harness.rejected().len(), 1);
    harness.push(stop("mock/rx/s0"));
    harness.push(start_rx("mock/rx/s0", None));
    harness.step(4_000_000).unwrap();
    harness.step(8_000_000).unwrap();
    let clocks = harness.clocks_of("mock/rx/s0");
    let blocks = harness.blocks("rx0");
    let second = clocks[1].origin.ticks_in(ROOT).unwrap();
    assert_eq!(heard(&blocks, &clocks[1], second + 100_000), (0.0, 0.0), "the refused value was not kept");
}

#[test]
fn mr_34_a_start_draws_one_phase_per_random_channel_whatever_the_values_and_a_cancelled_start_draws_nothing() {
    // Q4: each segment a `start_rx` begins draws one fresh phase per `random` channel in channel order, whether the values changed or not
    // (starts A: unchanged, B: changed); a start cancelled by a `Stop` before its origin draws nothing, so the next start takes the next draws.
    let constraints = [("radio.rx.c1.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let env = [
        ("sim.seed", serde_json::json!(7)),
        (
            "sim.channel",
            serde_json::json!({"couplings":[{"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0},{"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":1,"gain_db":0.0,"delay_ns":0}]}),
        ),
    ];
    let (waveform, bytes) = waveform_values(&vec![(0.5, 0.0); 400_000]);
    let mut harness = built_with_inputs(
        Layout { profile: "x3xx", id: "mock", rx: elements(&["0:A:cbx:0", "0:B:cbx:0"]), tx: elements(&["0:A:cbx:0"]), selector: &[], constraints: &constraints, env: &env, ports: &["rx0"] },
        BTreeMap::from([(waveform.hash.clone(), bytes)]),
    );
    harness.arm_start(T0).unwrap();
    let tx = harness.clocks_of("mock/tx/s0")[0].domain;
    harness.push(burst_with_waveform("mock/tx/s0", tx, 2_000_000, waveform));
    harness.step(0).unwrap();
    harness.step(T0 + 2_100_001).unwrap();
    let mut lo = ezsdr_sim::SimRng::new(7, "mock/lo");
    let (_, _, tx_phase) = (phase(&mut lo), phase(&mut lo), phase(&mut lo));
    let mut retune = ezsdr_sim::SimRng::new(7, "mock/lo/retune");
    let draws: Vec<f64> = (0..8).map(|_| phase(&mut retune)).collect();
    // A: same values.
    harness.push(stop("mock/rx/s0"));
    harness.step(T0 + 6_000_000).unwrap();
    harness.push(start_rx("mock/rx/s0", None));
    harness.step(T0 + 7_000_000).unwrap();
    harness.step(T0 + 80_000_000).unwrap();
    // B: a changed value.
    harness.push(stop("mock/rx/s0"));
    harness.push(set_value("radio.rx.c0.gain_db", 3.0));
    harness.step(T0 + 90_000_000).unwrap();
    harness.push(start_rx("mock/rx/s0", None));
    harness.step(T0 + 91_000_000).unwrap();
    harness.step(T0 + 200_000_000).unwrap();
    // D: a start booked ahead, cut by a `Stop` that takes effect at its origin (a start booked ahead is streaming, so nothing can be sent before
    // then, RM-21), so D has no sample; then E.
    harness.push(stop("mock/rx/s0"));
    harness.step(T0 + 210_000_000).unwrap();
    harness.push(start_rx("mock/rx/s0", Some(T0 + 250_000_000)));
    harness.step(T0 + 211_000_000).unwrap();
    harness.push(stop("mock/rx/s0"));
    harness.step(T0 + 250_000_000).unwrap();
    harness.push(start_rx("mock/rx/s0", None));
    harness.step(T0 + 251_000_000).unwrap();
    harness.step(T0 + 340_000_000).unwrap();
    let clocks = harness.clocks_of("mock/rx/s0");
    assert_eq!(clocks.len(), 4, "connect, A, B and E (D never began)");
    let blocks = harness.blocks("rx0");
    for (segment, expected) in [(1, (0, 1)), (2, (2, 3)), (3, (4, 5))] {
        let origin = clocks[segment].origin.ticks_in(ROOT).unwrap();
        // The staged gain stays the configuration of record for every later start (RM-21).
        let amplitude = if segment >= 2 { 10f64.powf(3.0 / 20.0) } else { 1.0 };
        let c0 = heard_channel(&blocks, &clocks[segment], origin + 100_000, 0);
        let c1 = heard_channel(&blocks, &clocks[segment], origin + 100_000, 1);
        assert_near(c0, rotated(0.5 * amplitude, tx_phase - draws[expected.0]));
        assert_near(c1, rotated(0.5, tx_phase - draws[expected.1]));
    }
}

#[test]
fn mr_18_a_peer_stream_is_undisturbed_by_a_stopped_stream_s_value() {
    // S5: `Stop` of stream 0 makes its channel's value acceptable and leaves stream 1 streaming, whose channel's value is refused; stream 1's
    // blocks are those of a control run without the stop and the values (RM-21: another stream's state does not matter).
    let run = |disturbed: bool| {
        let constraints = [("radio.rx.c1.stream", eq(Scalar::from(1)))];
        let mut harness = built(Layout { profile: "ideal", id: "mock", rx: count(2), tx: count(1), selector: &[], constraints: &constraints, env: &[], ports: &["rx0", "rx1"] });
        harness.arm_start(0).unwrap();
        harness.step(1_000_000).unwrap();
        if disturbed {
            harness.push(stop("mock/rx/s0"));
            harness.push(set_value("radio.rx.c0.frequency_hz", 2.0e9));
            harness.push(set_value("radio.rx.c1.frequency_hz", 2.0e9));
            harness.push(start_rx("mock/rx/s0", None));
        }
        harness.step(5_000_000).unwrap();
        harness.step(9_000_000).unwrap();
        let rejected = harness.rejected();
        let headers: Vec<_> =
            harness.headers("rx1").iter().map(|header| (header.first_sample_time.ticks_in(header.first_sample_time.domain()).unwrap(), header.len, header.flags, header.lost)).collect();
        (rejected, headers, harness.clocks_of("mock/rx/s1").len(), harness.clocks_of("mock/rx/s0").len())
    };
    let (control_rejected, control_headers, control_s1, control_s0) = run(false);
    let (rejected, headers, s1, s0) = run(true);
    assert!(control_rejected.is_empty());
    assert_eq!(rejected.len(), 1, "only stream 1's channel is refused: {rejected:?}");
    assert!(rejected[0].contains("stream 1"), "{rejected:?}");
    assert_eq!((headers, s1), (control_headers, control_s1), "stream 1 is undisturbed");
    assert_eq!((control_s0, s0), (1, 2), "stream 0 restarted");
}
