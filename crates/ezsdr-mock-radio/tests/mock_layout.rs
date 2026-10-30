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
fn mr_38_keys_is_an_unknown_selector_key() {
    let mut binding = Binding {
        module: module_ref(),
        selector: layout_support::ideal_selector("mock", 1, 0),
        profile: Some(ProfileRef { name: "ideal".to_owned(), version: Version::new(2, 0, 0) }),
        feed: None,
    };
    binding.selector.insert(Ident::parse("keys").unwrap(), Value::from("channel"));
    let why = MockRadio::from_binding(&binding).err().expect("the `keys` selector is refused").message;
    assert_eq!(why, "MR-2: unsupported selector key `keys`");
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
    // segment after the cut and registers a new SampleClock, though its set
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
    // RM-25's ready instant: the cut, 3 ms release window, 2 ms receive call, 3 ms allowance,
    // then the direction's 50 ms start lead.
    assert_eq!(clocks[1].origin.ticks_in(ROOT).unwrap(), T0 + 68_000_000);
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
    assert_eq!(destination[2].origin.ticks_in(ROOT).unwrap(), T0 + 188_000_000);
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
    assert_eq!(destination[1].origin.ticks_in(ROOT).unwrap(), now + 106_500_000);
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
    assert_eq!(moved[0].origin.ticks_in(ROOT).unwrap(), now + 138_000_000);
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
fn mr_34_a_layout_an_untimed_update_and_the_entry_tune_at_one_instant_draw_in_application_order() {
    // The receive entry above, with a frequency update at the re-entry's origin. The segment
    // has not run the release window by then, so it is an untimed tune and draws first; the entry's untimed
    // tune at the same instant draws after it and is what the channel carries, not coherent
    // there (MR-34, MR-30). A stream's origin is known only once it has begun, so the scenario
    // runs twice: once to read the origin, once with the update at it.
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
    let (update_draw, rx_entry) = (retune_draw(0), retune_draw(1));
    assert_ne!(update_draw, rx_entry);
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

fn cold_change_refused_at(at_ns: i64) -> serde_json::Value {
    serde_json::json!({ "at_ns": at_ns, "fault": "cold_change_refused", "target": "radio" })
}

const REFUSED_CHANGE: &str = "MR-18: the cold change was refused";
const REFUSED_BURST: &str = "MR-18: cancelled by a refused cold change";
const HALTED_STREAM: &str = "MR-25: the stream is halted by a refused change";

/// A repeated burst on `domain`, so that it is still open at the change that cuts it (RM-15).
fn loop_burst(target: &str, domain: ClockDomainId, sample: i64, samples: usize) -> Action {
    Action::TxBurst {
        target: tgt(target),
        waveform: waveform(samples, 1),
        repeat: true,
        at: AbsoluteDeadline::new(TimePoint::new(domain, sample)),
        requested_at: None,
        late_policy: LatePolicy::SendAsapAndFlag,
        metadata: BTreeMap::new(),
    }
}

#[test]
fn mr_20_a_start_rx_resumes_a_refused_stream() {
    // MR-21, MR-25: `start_rx` is the other item that starts a halted stream. The `ideal`
    // profile has no lead, so the segment begins at the start's instant, from the refused
    // value's channel set and rate (#64).
    let env = [("sim.faults", serde_json::json!([cold_change_refused_at(1_000_000)]))];
    let constraints = [
        ("radio.rx.c1.stream", eq(Scalar::from(0))),
        ("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(1_000_000.0).unwrap())),
    ];
    let mut harness = built(Layout { profile: "ideal", id: "mock", rx: count(2), tx: count(0), selector: &[], constraints: &constraints, env: &env, ports: &["rx0", "rx1"] });
    harness.arm_start(0).unwrap();
    harness.step(1_000_000).unwrap();
    let e = 3_000_000;
    harness.push(update("radio.rx.s0.sample_rate_hz", Value::num(2_000_000.0).unwrap(), UpdateClass::Cold, Some(e)));
    harness.step(e).unwrap();
    assert_eq!(harness.rejected(), [REFUSED_CHANGE.to_owned()], "the change was not refused");
    harness.push(start_rx("mock/rx/s0", Some(e + 10_000_000)));
    harness.step(e + 10_000_000).unwrap();
    harness.step(e + 20_000_000).unwrap();
    let clocks = harness.clocks_of("mock/rx/s0");
    assert_eq!(clocks.len(), 2, "{clocks:?}");
    assert_eq!(clocks[0].ended_at, Some(TimePoint::new(ROOT, e)));
    assert_eq!(clocks[1].origin, TimePoint::new(ROOT, e + 10_000_000));
    assert_eq!(clocks[1].root_ticks_per_tick, Rational::new(500, 1).unwrap());
    let segments = harness.section("segments");
    let begun = segments.last().expect("the restarted segment");
    assert_eq!(begun["channels"], serde_json::json!([0, 1]), "{begun}");
    assert_eq!(harness.rejected(), [REFUSED_CHANGE.to_owned()], "the start was refused too");
    assert_eq!(harness.applied_of("radio.rx.s0.sample_rate_hz"), [(serde_json::json!(2_000_000.0), e)]);
    assert_eq!(harness.section("faults")[0]["applied"], true);
    let first = harness.headers("rx0").into_iter().find(|header| header.first_sample_time.domain() == clocks[1].domain).expect("a block on the restarted clock");
    assert_eq!(first.first_sample_time.ticks_in(first.first_sample_time.domain()).unwrap(), 0);
    assert!(!first.flags.contains(BlockFlags::GAP_BEFORE));
}

#[test]
fn mr_20_a_hardware_timed_update_in_a_refused_halt_is_recorded() {
    // MR-18's no-channel rule on the Mock (UR-24's on the UHD Module): while the stream is
    // halted a `hardware_timed` update is neither refused nor lost: it is recorded at its own
    // instant and applies when the stream starts again. The Mock's row has no `note` (RM-27).
    let env = [("sim.faults", serde_json::json!([cold_change_refused_at(1_000_000)]))];
    let constraints = [("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(1_000_000.0).unwrap()))];
    let mut harness = built(Layout { profile: "x3xx", id: "mock", rx: two(), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &constraints, env: &env, ports: &["rx0"] });
    harness.arm_start(T0).unwrap();
    harness.step(T0 + 1_000_000).unwrap();
    harness.step(T0 + 2_000_000).unwrap();
    let e = T0 + 3_000_000;
    let h = e + 10_000_000;
    harness.push(update("radio.rx.s0.sample_rate_hz", Value::num(2_000_000.0).unwrap(), UpdateClass::Cold, Some(e)));
    harness.push(update("radio.rx.c0.gain_db", Value::num(6.0).unwrap(), UpdateClass::HardwareTimed, Some(h)));
    harness.step(e).unwrap();
    assert_eq!(harness.rejected(), [REFUSED_CHANGE.to_owned()], "the change was not refused");
    harness.step(h).unwrap();
    assert_eq!(harness.applied_of("radio.rx.c0.gain_db"), [(serde_json::json!(6.0), h)]);
    assert_eq!(harness.rejected(), [REFUSED_CHANGE.to_owned()], "the halted stream refused the timed update");
    let kinds: Vec<String> = harness.events().into_iter().map(|event| event.kind.as_str().to_owned())
        .filter(|kind| kind == ezsdr_radio::kinds::COMMAND_REJECTED || kind == ezsdr_radio::kinds::LATE_COMMAND).collect();
    assert_eq!(kinds, [ezsdr_radio::kinds::COMMAND_REJECTED], "the only event is the refusal's");
    assert_eq!(harness.clocks_of("mock/rx/s0").len(), 1, "the halt left a clock behind");
}

#[test]
fn mr_20_a_refused_transmit_change_ends_its_clock_and_cancels_its_bursts() {
    // MR-18, MR-25 (UR-25's shape): a refused transmit change ends the bursts on the old clock
    // at the cut, closes the open one with its record, ends the clock it started at its origin
    // and cancels the bursts booked on it. The halt plays on neither clock the refusal ended.
    let env = [("sim.faults", serde_json::json!([cold_change_refused_at(1_000_000)]))];
    let constraints = [
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.s0.sample_rate_hz", eq(Scalar::try_from(1_000_000.0).unwrap())),
    ];
    let mut harness = built(Layout { profile: "x3xx", id: "mock", rx: elements(&["0:A:ubx:0"]), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &constraints, env: &env, ports: &[] });
    harness.arm_start(T0).unwrap();
    let e = T0 + 100_000_000;
    harness.push(update("radio.tx.s0.sample_rate_hz", Value::num(2_000_000.0).unwrap(), UpdateClass::Cold, Some(e)));
    harness.step(0).unwrap();
    let clocks = harness.clocks_of("mock/tx/s0");
    assert_eq!(clocks.len(), 2, "the booked change registered no clock: {clocks:?}");
    let refused = clocks[1].domain;
    // A repeated burst open at the change, and one held on the clock the change starts.
    harness.push(loop_burst("mock/tx/s0", clocks[0].domain, 2_050_000, 1_000));
    harness.push(burst("mock/tx/s0", refused, 100, 100));
    harness.step(0).unwrap();
    harness.step(2_060_000_000).unwrap();
    assert!(harness.section("bursts").is_empty(), "the open burst ended before the refusal");
    harness.step(e).unwrap();
    let clocks = harness.clocks_of("mock/tx/s0");
    assert_eq!(clocks[0].ended_at, Some(TimePoint::new(ROOT, e)));
    // RM-25: the refused change's clock begins after the cut and the start lead, and ends at its
    // own origin, since no later change cut it first.
    assert_eq!(clocks[1].origin, TimePoint::new(ROOT, 2_150_000_000));
    assert_eq!(clocks[1].ended_at, Some(TimePoint::new(ROOT, 2_150_000_000)));
    assert_eq!(harness.rejected(), [REFUSED_CHANGE.to_owned(), REFUSED_BURST.to_owned()]);
    let bursts: Vec<ezsdr_kernel::stream::BurstRecord> = serde_json::from_value(serde_json::Value::Array(harness.section("bursts"))).unwrap();
    assert_eq!(bursts.len(), 1, "{bursts:?}");
    assert_eq!((bursts[0].blocks, bursts[0].samples, bursts[0].end), (50, 50_000, ezsdr_kernel::stream::BurstEnd::Stop));
    assert_eq!(bursts[0].target, TimePoint::new(clocks[0].domain, 2_050_000));
    let mut kinds: Vec<(String, serde_json::Value)> = harness.events().into_iter()
        .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED)
        .map(|event| (event.payload["kind"].as_str().unwrap_or_default().to_owned(), event.payload["action"].clone())).collect();
    kinds.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(kinds, [("tx_burst".to_owned(), serde_json::Value::Null), ("update_parameter".to_owned(), serde_json::Value::Null)]);
    // The halt: a burst at or after the old clock's cut, or one the late policy moved past it,
    // plays on neither; the clock the refused change started is cut at its origin and is no clock
    // MR-16 takes.
    harness.push(burst("mock/tx/s0", clocks[0].domain, 2_200_000, 100));
    harness.push(burst("mock/tx/s0", refused, 0, 100));
    harness.push(burst("mock/tx/s0", clocks[0].domain, 2_099_000, 100));
    harness.step(e).unwrap();
    assert_eq!(harness.rejected()[2..], [HALTED_STREAM.to_owned(), "MR-16: target, clock, waveform size, channel count or metadata is invalid".to_owned(), HALTED_STREAM.to_owned()]);
    // The next `cold` change starts the stream again, and a burst on its clock plays; the
    // refused clock's domain still takes none.
    let later = e + 100_000_000;
    harness.push(update("radio.tx.s0.sample_rate_hz", Value::num(4_000_000.0).unwrap(), UpdateClass::Cold, Some(later)));
    harness.step(e).unwrap();
    let clocks = harness.clocks_of("mock/tx/s0");
    assert_eq!(clocks.len(), 3, "{clocks:?}");
    // RM-25: the refused clock is cut where this change cuts it, `later`, so the new clock begins
    // a lead after that, 50 ms; sample 16 000 at 4 MS/s is 4 ms after that.
    assert_eq!(clocks[2].origin, TimePoint::new(ROOT, later + 50_000_000));
    harness.push(burst("mock/tx/s0", clocks[1].domain, 0, 100));
    harness.push(burst("mock/tx/s0", clocks[2].domain, 16_000, 100));
    harness.step(later).unwrap();
    harness.step(later + 60_000_000).unwrap();
    assert_eq!(harness.rejected().last(), Some(&"MR-16: target, clock, waveform size, channel count or metadata is invalid".to_owned()), "{:?}", harness.rejected());
    let bursts: Vec<ezsdr_kernel::stream::BurstRecord> = serde_json::from_value(serde_json::Value::Array(harness.section("bursts"))).unwrap();
    assert_eq!(bursts.len(), 2, "{bursts:?}");
    assert_eq!((bursts[1].target, bursts[1].samples, bursts[1].end), (TimePoint::new(clocks[2].domain, 16_000), 100, ezsdr_kernel::stream::BurstEnd::Eob));
}

/// One transmit stream at 1 MS/s whose cold changes the first `refusals` fault lines refuse.
fn transmit_refusal_harness(refusals: usize) -> Harness {
    let env = [("sim.faults", serde_json::Value::Array((0..refusals).map(|_| cold_change_refused_at(1_000_000)).collect()))];
    let constraints = [
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.s0.sample_rate_hz", eq(Scalar::try_from(1_000_000.0).unwrap())),
    ];
    let mut harness = built(Layout { profile: "x3xx", id: "mock", rx: elements(&["0:A:ubx:0"]), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &constraints, env: &env, ports: &[] });
    harness.arm_start(T0).unwrap();
    harness
}

fn tx_rate(rate: f64, e: i64) -> Action {
    update("radio.tx.s0.sample_rate_hz", Value::num(rate).unwrap(), UpdateClass::Cold, Some(e))
}

/// The origin of the transmit clocks in the harness's `segments` section.
fn tx_segment_origins(harness: &Harness) -> Vec<i64> {
    harness.section("segments").iter().filter(|row| row["direction"] == "tx").map(|row| row["origin"]["ticks"].as_i64().unwrap()).collect()
}

#[test]
fn mr_20_a_transmit_change_after_a_refusal_begins_as_if_not_refused() {
    // RM-25 (design 43): C (2 MS/s) due at T0 + 100 ms is refused, N (4 MS/s) is due at T0 + 300 ms.
    // N begins where it would had C not been refused: C is cut at 300 ms (C's clock begins at
    // 150 ms, so sample 300 000 at 2 MS/s), and N begins a lead (50 ms) after that, at 350 ms.
    // Booked before the refusal, N cuts C's clock at 300 ms and the clock's record keeps that cut;
    // booked after it, the refused clock ends at its own origin, 150 ms. A burst at sample 16 000
    // of N's clock plays 4 ms after N's origin, at 354 ms.
    let (ea, eb) = (T0 + 100_000_000, T0 + 300_000_000);
    let (origin, start) = (T0 + 350_000_000, T0 + 354_000_000);
    for before in [true, false] {
        let mut harness = transmit_refusal_harness(1);
        harness.push(tx_rate(2_000_000.0, ea));
        if before { harness.push(tx_rate(4_000_000.0, eb)); }
        harness.step(0).unwrap();
        harness.step(ea).unwrap();
        if !before {
            harness.push(tx_rate(4_000_000.0, eb));
            harness.step(ea).unwrap();
        }
        assert_eq!(harness.rejected(), [REFUSED_CHANGE.to_owned()], "booked before {before}");
        let clocks = harness.clocks_of("mock/tx/s0");
        assert_eq!(clocks.len(), 3, "booked before {before}: {clocks:?}");
        assert_eq!(clocks[1].origin, TimePoint::new(ROOT, T0 + 150_000_000), "booked before {before}");
        let end = if before { eb } else { T0 + 150_000_000 };
        assert_eq!(clocks[1].ended_at, Some(TimePoint::new(ROOT, end)), "booked before {before}: the refused clock's record");
        assert_eq!(clocks[2].origin, TimePoint::new(ROOT, origin), "booked before {before}");
        harness.push(burst("mock/tx/s0", clocks[2].domain, 16_000, 100));
        harness.step(ea).unwrap();
        harness.step(start - 1_000_000).unwrap();
        assert!(harness.section("bursts").is_empty(), "booked before {before}: the burst played before {start}");
        harness.step(start + 20_000_000).unwrap();
        let bursts: Vec<ezsdr_kernel::stream::BurstRecord> = serde_json::from_value(serde_json::Value::Array(harness.section("bursts"))).unwrap();
        assert_eq!(bursts.len(), 1, "booked before {before}: {bursts:?}");
        assert_eq!(bursts[0].target, TimePoint::new(clocks[2].domain, 16_000));
        assert_eq!(harness.clocks_of("mock/tx/s0")[2].origin, TimePoint::new(ROOT, origin), "booked before {before}: the origin did not move");
        assert_eq!(tx_segment_origins(&harness).last(), Some(&origin), "booked before {before}: the plan's origin");
    }
}

#[test]
fn mr_20_a_transmit_move_after_a_refused_source_change_begins_as_if_not_refused() {
    // RM-25, RM-34 (design 43): the source's change C (2 MS/s, due T0 + 100 ms, origin 150 ms) is
    // refused and, received at 100 ms, a move of channel 0 into the empty stream 1 is due at 300 ms.
    // C's clock ends at its own origin, 150 ms (not at the move's cut, 300 ms), but the destination
    // begins where it would had C not been refused: 350 ms, a lead after the cut C would have had.
    // A burst at sample 4 000 of the destination's clock (1 MS/s) plays at 354 ms.
    let (ea, em) = (T0 + 100_000_000, T0 + 300_000_000);
    let run = |refused: bool| {
        let faults = if refused { serde_json::json!([cold_change_refused_at(1_000_000)]) } else { serde_json::json!([]) };
        let env = [("sim.faults", faults)];
        let constraints = [
            ("radio.tx.c0.stream", eq(Scalar::from(0))),
            ("radio.tx.c1.stream", eq(Scalar::from(-1))),
            ("radio.tx.s0.sample_rate_hz", eq(Scalar::try_from(1_000_000.0).unwrap())),
            ("radio.tx.s1.sample_rate_hz", eq(Scalar::try_from(1_000_000.0).unwrap())),
        ];
        let mut harness = built(Layout { profile: "x3xx", id: "mock", rx: elements(&["0:A:ubx:0"]), tx: two(), selector: &[], constraints: &constraints, env: &env, ports: &[] });
        harness.arm_start(T0).unwrap();
        harness.push(tx_rate(2_000_000.0, ea));
        harness.step(0).unwrap();
        harness.step(ea).unwrap();
        harness.push(update("radio.tx.c0.stream", Value::from(1), UpdateClass::Cold, Some(em)));
        harness.step(ea).unwrap();
        let destination = harness.clocks_of("mock/tx/s1");
        assert_eq!(destination.len(), 1, "refused {refused}: {destination:?}");
        harness.push(burst("mock/tx/s1", destination[0].domain, 4_000, 100));
        harness.step(ea).unwrap();
        harness.step(T0 + 353_000_000).unwrap();
        assert!(harness.section("bursts").is_empty(), "refused {refused}: the burst played before 354 ms");
        harness.step(T0 + 360_000_000).unwrap();
        let bursts: Vec<ezsdr_kernel::stream::BurstRecord> = serde_json::from_value(serde_json::Value::Array(harness.section("bursts"))).unwrap();
        assert_eq!(bursts.len(), 1, "refused {refused}: {bursts:?}");
        assert_eq!(bursts[0].target, TimePoint::new(destination[0].domain, 4_000), "refused {refused}: not late");
        (destination[0].origin, harness.clocks_of("mock/tx/s0")[1].ended_at, harness.rejected())
    };
    let (plain_origin, plain_end, plain_rejected) = run(false);
    let (refused_origin, refused_end, refused_rejected) = run(true);
    assert!(plain_rejected.is_empty(), "{plain_rejected:?}");
    assert_eq!(refused_rejected, [REFUSED_CHANGE.to_owned()]);
    assert_eq!(plain_origin, TimePoint::new(ROOT, T0 + 350_000_000));
    assert_eq!(refused_origin, plain_origin, "the destination does not begin a lead early");
    assert_eq!(plain_end, Some(TimePoint::new(ROOT, em)));
    assert_eq!(refused_end, Some(TimePoint::new(ROOT, T0 + 150_000_000)), "the refused clock's record is unchanged");
}

#[test]
fn mr_20_a_transmit_change_after_repeated_refusals_begins_as_if_none_were_refused() {
    // RM-25 (design 43): C1 (2 MS/s, T0 + 100 ms) and C2 (4 MS/s, T0 + 200 ms) are refused, each
    // booked once the one before it was refused; N (1 MS/s, T0 + 500 ms) begins at 550 ms, as it
    // would with no refusal: C1 is cut at 200, C2 begins at 250 and is cut at 500. Each refused
    // clock ends at its own origin, 150 and 250 ms.
    let (e1, e2, e3) = (T0 + 100_000_000, T0 + 200_000_000, T0 + 500_000_000);
    let mut harness = transmit_refusal_harness(2);
    harness.push(tx_rate(2_000_000.0, e1));
    harness.step(0).unwrap();
    harness.step(e1).unwrap();
    harness.push(tx_rate(4_000_000.0, e2));
    harness.step(e1).unwrap();
    harness.step(e2).unwrap();
    harness.push(tx_rate(1_000_000.0, e3));
    harness.step(e2).unwrap();
    assert_eq!(harness.rejected(), [REFUSED_CHANGE.to_owned(), REFUSED_CHANGE.to_owned()]);
    let clocks = harness.clocks_of("mock/tx/s0");
    assert_eq!(clocks.len(), 4, "{clocks:?}");
    assert_eq!((clocks[1].origin, clocks[1].ended_at), (TimePoint::new(ROOT, T0 + 150_000_000), Some(TimePoint::new(ROOT, T0 + 150_000_000))));
    assert_eq!((clocks[2].origin, clocks[2].ended_at), (TimePoint::new(ROOT, T0 + 250_000_000), Some(TimePoint::new(ROOT, T0 + 250_000_000))));
    assert_eq!(clocks[3].origin, TimePoint::new(ROOT, T0 + 550_000_000));
}

#[test]
fn mr_20_a_burst_on_a_clock_booked_after_a_refusal_plays() {
    // MR-20, MR-25 (UR-21's shape, #59): the halt refuses the clock the refusal ended, not one a
    // later `cold` change in the plan starts; a burst booked on that clock before its instant is
    // held and plays when the change applies.
    let constraints = [
        ("radio.rx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
    ];
    let env = [
        ("sim.channel", serde_json::json!({"couplings":[{"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0}]})),
        ("sim.faults", serde_json::json!([cold_change_refused_at(1_000_000)])),
    ];
    let (waveform, bytes) = waveform_values(&vec![(0.5, 0.0); 100]);
    let mut harness = built_with_inputs(Layout {
        profile: "x3xx", id: "mock", rx: elements(&["0:A:ubx:0"]), tx: elements(&["0:A:ubx:0"]),
        selector: &[], constraints: &constraints, env: &env, ports: &["rx0"],
    }, BTreeMap::from([(waveform.hash.clone(), bytes)]));
    harness.arm_start(T0).unwrap();
    let (ea, eb) = (T0 + 100_000_000, T0 + 300_000_000);
    harness.push(update("radio.tx.s0.sample_rate_hz", Value::num(2_000_000.0).unwrap(), UpdateClass::Cold, Some(ea)));
    harness.step(0).unwrap();
    assert_eq!(harness.clocks_of("mock/tx/s0").len(), 2, "the refused change registered no clock");
    harness.step(ea).unwrap();
    assert_eq!(harness.rejected(), [REFUSED_CHANGE.to_owned()], "the change was not refused");
    // B is booked for a later instant: its clock is registered, and the stream is still halted.
    harness.push(update("radio.tx.s0.sample_rate_hz", Value::num(4_000_000.0).unwrap(), UpdateClass::Cold, Some(eb)));
    harness.step(eb - 10_000_000).unwrap();
    let clocks = harness.clocks_of("mock/tx/s0");
    assert_eq!(clocks.len(), 3, "{clocks:?}");
    // RM-25 (design 43): B, booked at 290 ms after the refusal, cuts the refused clock at its own
    // instant (300 ms), so its clock begins a lead after that, 350 ms, not at 340 ms.
    assert_eq!(clocks[2].origin, TimePoint::new(ROOT, eb + 50_000_000));
    assert!(harness.section("bursts").is_empty(), "a burst played in the halt");
    // A burst on B's clock, received before B's instant: the halt takes it (UR-21's `book_burst`).
    harness.push(burst_with_waveform("mock/tx/s0", clocks[2].domain, 16_000, waveform));
    harness.step(eb - 10_000_000).unwrap();
    assert_eq!(harness.rejected(), [REFUSED_CHANGE.to_owned()], "the halt refused B's burst: {:?}", harness.rejected());
    // The blocks published so far are silence; the burst's are the ones after them.
    harness.blocks("rx0");
    // B applies at `eb`; its clock starts 50 ms later and plays the held burst.
    harness.step(eb).unwrap();
    harness.step(eb + 100_000_000).unwrap();
    let bursts: Vec<ezsdr_kernel::stream::BurstRecord> = serde_json::from_value(serde_json::Value::Array(harness.section("bursts"))).unwrap();
    assert_eq!(bursts.len(), 1, "B's clock played no burst: {bursts:?}");
    assert_eq!((bursts[0].target, bursts[0].blocks, bursts[0].samples, bursts[0].end), (TimePoint::new(clocks[2].domain, 16_000), 1, 100, ezsdr_kernel::stream::BurstEnd::Eob));
    let stats = &harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.stats").unwrap()];
    assert_eq!(stats["tx_blocks"], serde_json::json!(1), "the burst transmitted no block");
    // The samples took the medium: nothing else is coupled, so the nonzero receive sample is the
    // burst's.
    let heard = harness.blocks("rx0");
    assert!(heard.iter().any(|block| (0..block.header().len as usize).any(|sample| block_sample(block, 0, sample) != (0.0, 0.0))), "the burst did not reach the channel");
}

#[test]
fn mr_20_only_the_refused_clock_s_bursts_are_cancelled() {
    // MR-18 (UR-25's shape, #59): with two changes booked, the refusal of the first cancels the
    // bursts on the clock it started alone. A clock a later change had already cut keeps that
    // cut (RM-25: only the events after the refused instant are recomputed), and the later
    // change's own clock keeps its origin and plays its burst.
    let env = [("sim.faults", serde_json::json!([cold_change_refused_at(1_000_000)]))];
    let constraints = [
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.s0.sample_rate_hz", eq(Scalar::try_from(1_000_000.0).unwrap())),
    ];
    let mut harness = built(Layout { profile: "x3xx", id: "mock", rx: elements(&["0:A:ubx:0"]), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &constraints, env: &env, ports: &[] });
    harness.arm_start(T0).unwrap();
    let (ea, eb) = (T0 + 100_000_000, T0 + 200_000_000);
    harness.push(update("radio.tx.s0.sample_rate_hz", Value::num(2_000_000.0).unwrap(), UpdateClass::Cold, Some(ea)));
    harness.push(update("radio.tx.s0.sample_rate_hz", Value::num(4_000_000.0).unwrap(), UpdateClass::Cold, Some(eb)));
    harness.step(0).unwrap();
    let clocks = harness.clocks_of("mock/tx/s0");
    assert_eq!(clocks.len(), 3, "{clocks:?}");
    assert_eq!((clocks[1].origin, clocks[1].ended_at), (TimePoint::new(ROOT, 2_150_000_000), Some(TimePoint::new(ROOT, eb))));
    assert_eq!(clocks[2].origin, TimePoint::new(ROOT, 2_250_000_000));
    harness.push(burst("mock/tx/s0", clocks[1].domain, 100, 100));
    harness.push(burst("mock/tx/s0", clocks[2].domain, 100, 100));
    harness.step(0).unwrap();
    // The scene: both bursts are booked on the clocks they name, nothing refused yet.
    assert!(harness.rejected().is_empty(), "a burst was refused before the change: {:?}", harness.rejected());
    harness.step(ea).unwrap();
    assert_eq!(harness.rejected(), [REFUSED_CHANGE.to_owned(), REFUSED_BURST.to_owned()], "a burst on another clock was cancelled");
    let clocks = harness.clocks_of("mock/tx/s0");
    assert_eq!(clocks[1].ended_at, Some(TimePoint::new(ROOT, eb)), "the later change's cut of the refused clock moved");
    assert_eq!(clocks[2].origin, TimePoint::new(ROOT, 2_250_000_000), "the later change's clock moved");
    let segments = harness.section("segments");
    let tx: Vec<&serde_json::Value> = segments.iter().filter(|row| row["direction"] == "tx").collect();
    assert!(tx.iter().any(|row| row["origin"]["ticks"] == serde_json::json!(2_150_000_000_i64) && row["ended_at"]["ticks"] == serde_json::json!(eb)), "{segments:?}");
    assert!(tx.iter().any(|row| row["origin"]["ticks"] == serde_json::json!(2_250_000_000_i64) && row["ended_at"].is_null()), "{segments:?}");
    harness.step(eb).unwrap();
    harness.step(2_250_100_000).unwrap();
    let bursts: Vec<ezsdr_kernel::stream::BurstRecord> = serde_json::from_value(serde_json::Value::Array(harness.section("bursts"))).unwrap();
    assert_eq!(bursts.len(), 1, "the later change's burst did not play: {bursts:?}");
    assert_eq!((bursts[0].target, bursts[0].samples, bursts[0].end), (TimePoint::new(clocks[2].domain, 100), 100, ezsdr_kernel::stream::BurstEnd::Eob));
    assert_eq!(harness.rejected().len(), 2);
}

#[test]
fn mr_20_a_refused_transmit_burst_leaves_the_channel_silent() {
    // MR-32 (spec 11, CH-4): the bursts of a refused change never reach the medium. The control
    // burst before the refusal does, so the silence afterwards is the cancellation's.
    let constraints = [
        ("radio.rx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
    ];
    let env = [
        ("sim.channel", serde_json::json!({"couplings":[{"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0}]})),
        ("sim.faults", serde_json::json!([cold_change_refused_at(1_000_000)])),
    ];
    let (waveform, bytes) = waveform_values(&vec![(0.5, 0.0); 100]);
    let mut harness = built_with_inputs(Layout {
        profile: "x3xx", id: "mock", rx: elements(&["0:A:ubx:0"]), tx: elements(&["0:A:ubx:0"]),
        selector: &[], constraints: &constraints, env: &env, ports: &["rx0"],
    }, BTreeMap::from([(waveform.hash.clone(), bytes)]));
    harness.arm_start(T0).unwrap();
    let clocks = harness.clocks_of("mock/tx/s0");
    assert_eq!(clocks.len(), 1, "{clocks:?}");
    harness.push(burst_with_waveform("mock/tx/s0", clocks[0].domain, 2_000_000, waveform.clone()));
    harness.step(0).unwrap();
    harness.step(T0 + 2_000_000).unwrap();
    let control = harness.blocks("rx0");
    assert!(control.iter().any(|block| (0..block.header().len as usize).any(|sample| block_sample(block, 0, sample) != (0.0, 0.0))), "the coupling delivered nothing");
    let e = T0 + 100_000_000;
    harness.push(update("radio.tx.s0.sample_rate_hz", Value::num(2_000_000.0).unwrap(), UpdateClass::Cold, Some(e)));
    harness.step(T0 + 2_000_000).unwrap();
    let clocks = harness.clocks_of("mock/tx/s0");
    assert_eq!(clocks.len(), 2, "the booked change registered no clock: {clocks:?}");
    harness.push(burst_with_waveform("mock/tx/s0", clocks[1].domain, 16_000, waveform));
    harness.step(T0 + 2_000_000).unwrap();
    harness.step(e).unwrap();
    assert_eq!(harness.rejected(), [REFUSED_CHANGE.to_owned(), REFUSED_BURST.to_owned()], "the change or its burst was not refused");
    harness.blocks("rx0");
    harness.step(e + 100_000_000).unwrap();
    let after = harness.blocks("rx0");
    assert!(!after.is_empty(), "the receive stream stopped publishing");
    assert!(
        after.iter().all(|block| (0..block.header().len as usize).all(|sample| block_sample(block, 0, sample) == (0.0, 0.0))),
        "the refused burst radiated",
    );
}

#[test]
fn mr_18_a_layout_a_move_after_a_refused_source_change_waits_the_refused_segments_end() {
    // RM-25, RM-34 (design 43): the source's rate change (due at T0 + 10 ms; its segment would
    // begin at 68 ms) is refused, and a move of its channel c1 to the other stream is due at
    // T0 + 100 ms. The refused segment runs for planning until the move cuts it, so the
    // destination begins where it would had the change not been refused: s0 and s1 begin at the
    // same instants in both runs, and the destination a lead after the source's release.
    let run = |refused: bool| {
        let faults = if refused { serde_json::json!([cold_change_refused_at(1_000_000)]) } else { serde_json::json!([]) };
        let env = [("sim.faults", faults)];
        let constraints = [("radio.rx.c1.stream", eq(Scalar::from(0)))];
        let mut harness = built(Layout { profile: "x3xx", id: "mock", rx: two(), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &constraints, env: &env, ports: &["rx0", "rx1"] });
        harness.arm_start(T0).unwrap();
        harness.step(T0 + 2_000_000).unwrap();
        let e = T0 + 10_000_000;
        harness.push(update("radio.rx.s0.sample_rate_hz", Value::num(2_000_000.0).unwrap(), UpdateClass::Cold, Some(e)));
        harness.push(update("radio.rx.c1.stream", Value::from(1), UpdateClass::Cold, Some(T0 + 100_000_000)));
        harness.step(T0 + 2_000_000).unwrap();
        harness.step(T0 + 100_000_000).unwrap();
        harness.step(T0 + 400_000_000).unwrap();
        let origins = |path: &str| harness.clocks_of(path).iter().map(|clock| clock.origin.ticks_in(ROOT).unwrap()).collect::<Vec<_>>();
        (origins("mock/rx/s0"), origins("mock/rx/s1"), harness.rejected())
    };
    let (plain_s0, plain_s1, plain_rejected) = run(false);
    let (refused_s0, refused_s1, refused_rejected) = run(true);
    assert!(plain_rejected.is_empty(), "{plain_rejected:?}");
    assert_eq!(refused_rejected, [REFUSED_CHANGE.to_owned()]);
    assert_eq!(plain_s1.len(), 1, "the destination begins: {plain_s1:?}");
    assert!(plain_s1[0] >= T0 + 100_000_000 + 50_000_000, "the destination waits the source's release: {plain_s1:?}");
    assert_eq!(refused_s1, plain_s1, "the destination's origin does not move");
    assert_eq!(plain_s0.len(), 3, "{plain_s0:?}");
    assert_eq!(plain_s0[1], T0 + 68_000_000, "the change's segment, which the refusal leaves out");
    assert_eq!(refused_s0, [plain_s0[0], plain_s0[2]], "the source's other origins do not move");
}

#[test]
fn mr_20_a_refused_move_halts_both_lines() {
    // MR-34, MR-20: a move refused at its instant halts both lines, as RM-34's `refuse_move`
    // does; the destination starts again on its own `start_rx`, from the refused move's set.
    let env = [("sim.faults", serde_json::json!([cold_change_refused_at(1_000_000)]))];
    let constraints = [("radio.rx.c1.stream", eq(Scalar::from(0)))];
    let mut harness = built(Layout { profile: "x3xx", id: "mock", rx: two(), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &constraints, env: &env, ports: &["rx0", "rx1"] });
    harness.arm_start(T0).unwrap();
    harness.step(T0 + 2_000_000).unwrap();
    assert_eq!((harness.clocks_of("mock/rx/s0").len(), harness.clocks_of("mock/rx/s1").len()), (1, 0), "the scene is not the one the move starts from");
    let e = T0 + 10_000_000;
    harness.push(update("radio.rx.c1.stream", Value::from(1), UpdateClass::Cold, Some(e)));
    harness.step(T0 + 2_000_000).unwrap();
    harness.step(e).unwrap();
    harness.step(e + 100_000_000).unwrap();
    let s0 = harness.clocks_of("mock/rx/s0");
    assert_eq!(s0.len(), 1, "{s0:?}");
    assert_eq!(s0[0].ended_at, Some(TimePoint::new(ROOT, e)));
    assert!(harness.clocks_of("mock/rx/s1").is_empty(), "the destination ran");
    assert!(harness.blocks("rx1").is_empty(), "the destination published");
    let segments = harness.section("segments");
    assert!(segments.iter().all(|row| row["stream"] == 0), "a segment of the destination: {segments:?}");
    assert_eq!(harness.rejected(), [REFUSED_CHANGE.to_owned()]);
    assert_eq!(harness.applied_of("radio.rx.c1.stream"), [(serde_json::json!(1), e)]);
    assert_eq!(harness.section("faults")[0]["applied"], true);
    // The destination begins on `start_rx`, with the channel the refused move left it.
    let started = e + 110_000_000;
    harness.push(start_rx("mock/rx/s1", Some(started)));
    harness.step(started).unwrap();
    harness.step(started + 70_000_000).unwrap();
    let s1 = harness.clocks_of("mock/rx/s1");
    assert_eq!(s1.len(), 1, "{s1:?}");
    // RM-25: the refused move's segment ran for planning until this `start_rx` cut it at `started`,
    // so the segment waits the 3 ms release window, receive call (2 ms), allowance (3 ms) and
    // lead: 58 ms.
    assert_eq!(s1[0].origin, TimePoint::new(ROOT, started + 58_000_000));
    let begun = harness.section("segments").into_iter().find(|row| row["stream"] == 1).expect("the destination's segment");
    assert_eq!(begun["channels"], serde_json::json!([1]), "{begun}");
    let first = harness.headers("rx1").into_iter().next().expect("the destination published");
    assert_eq!(first.first_sample_time.ticks_in(first.first_sample_time.domain()).unwrap(), 0);
    assert!(!first.flags.contains(BlockFlags::GAP_BEFORE));
    assert_eq!(harness.rejected(), [REFUSED_CHANGE.to_owned()], "the start was refused");
}

#[test]
fn mr_20_two_changes_at_one_instant_refuse_the_first_booked() {
    // MR-20: at one instant the fault's candidate is the first booked change (`seq`), not the
    // lowest stream number: the booking order decides.
    let env = [("sim.faults", serde_json::json!([cold_change_refused_at(1_000_000)]))];
    let constraints = [("radio.rx.c1.stream", eq(Scalar::from(1)))];
    let changes = [("radio.rx.s0.sample_rate_hz", 2_000_000.0), ("radio.rx.s1.sample_rate_hz", 4_000_000.0)];
    for first in [0, 1] {
        let mut harness = built(Layout { profile: "ideal", id: "mock", rx: count(2), tx: count(0), selector: &[], constraints: &constraints, env: &env, ports: &["rx0", "rx1"] });
        harness.arm_start(0).unwrap();
        let e = 10_000_000;
        for index in [first, 1 - first] {
            harness.push(update(changes[index].0, Value::num(changes[index].1).unwrap(), UpdateClass::Cold, Some(e)));
        }
        harness.step(2_000_000).unwrap();
        // The scene: both streams run on one clock each, and both changes are still pending.
        assert_eq!((harness.clocks_of("mock/rx/s0").len(), harness.clocks_of("mock/rx/s1").len()), (1, 1), "first booked {first}");
        assert!(harness.applied_of("radio.rx.s0.sample_rate_hz").is_empty() && harness.applied_of("radio.rx.s1.sample_rate_hz").is_empty());
        harness.step(e).unwrap();
        harness.step(e + 10_000_000).unwrap();
        for (index, (name, rate)) in changes.iter().enumerate() {
            let path = if index == 0 { "mock/rx/s0" } else { "mock/rx/s1" };
            let clocks = harness.clocks_of(path);
            if index == first {
                assert_eq!(clocks.len(), 1, "the second change was refused instead: {clocks:?}");
                assert_eq!(clocks[0].ended_at, Some(TimePoint::new(ROOT, e)));
            } else {
                assert_eq!(clocks.len(), 2, "{path} was not refused: {clocks:?}");
                assert_eq!(clocks[1].origin, TimePoint::new(ROOT, e));
                assert_eq!(clocks[1].root_ticks_per_tick, Rational::new((1e9 / rate) as u64, 1).unwrap());
            }
            assert_eq!(harness.applied_of(name), [(serde_json::json!(rate), e)], "{name}");
        }
        assert_eq!(harness.rejected(), [REFUSED_CHANGE.to_owned()], "first booked {first}");
        assert_eq!(harness.section("faults")[0]["applied"], true);
    }
}

#[test]
fn mr_20_a_cold_change_to_a_linkless_stream_is_no_candidate() {
    // MR-20, MR-39: a linkless receive stream's line is stopped at T0, so its `cold` change
    // starts no segment and the plan gives it nothing to refuse. The fault waits for a stream
    // that runs.
    let env = [("sim.faults", serde_json::json!([cold_change_refused_at(1_000_000)]))];
    let constraints = [
        ("radio.rx.c1.stream", eq(Scalar::from(1))),
        ("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(1_000_000.0).unwrap())),
        ("radio.rx.s1.sample_rate_hz", eq(Scalar::try_from(1_000_000.0).unwrap())),
    ];
    let mut harness = built(Layout { profile: "x3xx", id: "mock", rx: two(), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &constraints, env: &env, ports: &["rx0"] });
    harness.arm_start(T0).unwrap();
    let (e1, e2) = (T0 + 3_000_000, T0 + 10_000_000);
    harness.push(update("radio.rx.s1.sample_rate_hz", Value::num(2_000_000.0).unwrap(), UpdateClass::Cold, Some(e1)));
    harness.push(update("radio.rx.s0.sample_rate_hz", Value::num(2_000_000.0).unwrap(), UpdateClass::Cold, Some(e2)));
    harness.step(T0 + 500_000).unwrap();
    harness.step(e1).unwrap();
    assert!(harness.rejected().is_empty(), "the linkless stream's change was refused: {:?}", harness.rejected());
    harness.step(e2).unwrap();
    harness.step(e2 + 60_000_000).unwrap();
    let s0 = harness.clocks_of("mock/rx/s0");
    assert_eq!(s0.len(), 1, "the running stream's change was not refused: {s0:?}");
    assert_eq!(s0[0].ended_at, Some(TimePoint::new(ROOT, e2)));
    assert!(harness.clocks_of("mock/rx/s1").is_empty());
    assert_eq!(harness.rejected(), [REFUSED_CHANGE.to_owned()]);
    assert_eq!(harness.applied_of("radio.rx.s1.sample_rate_hz"), [(serde_json::json!(2_000_000.0), e1)]);
    assert_eq!(harness.applied_of("radio.rx.s0.sample_rate_hz"), [(serde_json::json!(2_000_000.0), e2)]);
    assert_eq!(harness.section("faults")[0]["applied"], true);
}

#[test]
fn mr_20_the_fault_follows_the_effective_instant_not_the_origin() {
    // MR-20: the candidate is the first change by its effective instant and arrival order. Here
    // the transmit change's segment begins earlier (its lead is 50 ms, a receive segment also
    // waits its call and allowance), yet the receive change is the one refused.
    let env = [("sim.faults", serde_json::json!([cold_change_refused_at(1_000_000)]))];
    let constraints = [
        ("radio.rx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.s0.sample_rate_hz", eq(Scalar::try_from(1_000_000.0).unwrap())),
        ("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(1_000_000.0).unwrap())),
    ];
    let mut harness = built(Layout { profile: "x3xx", id: "mock", rx: elements(&["0:A:ubx:0"]), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &constraints, env: &env, ports: &["rx0"] });
    harness.arm_start(T0).unwrap();
    let (e_rx, e_tx) = (T0 + 5_000_000, T0 + 6_000_000);
    harness.push(update("radio.rx.s0.sample_rate_hz", Value::num(2_000_000.0).unwrap(), UpdateClass::Cold, Some(e_rx)));
    harness.push(update("radio.tx.s0.sample_rate_hz", Value::num(2_000_000.0).unwrap(), UpdateClass::Cold, Some(e_tx)));
    harness.step(0).unwrap();
    // The scene: both changes are booked — the transmit one's clock is registered at booking.
    assert_eq!(harness.clocks_of("mock/tx/s0").len(), 2, "the transmit change was not booked");
    assert!(harness.applied_of("radio.rx.s0.sample_rate_hz").is_empty());
    harness.step(e_rx).unwrap();
    harness.step(e_tx).unwrap();
    harness.step(e_tx + 60_000_000).unwrap();
    let rx = harness.clocks_of("mock/rx/s0");
    assert_eq!(rx.len(), 1, "the receive change was not the one refused: {rx:?}");
    assert_eq!(rx[0].ended_at, Some(TimePoint::new(ROOT, e_rx)));
    let tx = harness.clocks_of("mock/tx/s0");
    assert_eq!(tx.len(), 2, "the transmit change was refused instead: {tx:?}");
    // The transmit segment's origin, 50 ms after its change, is earlier than the receive
    // change's would have been (60 ms after, behind the call and allowance).
    assert_eq!(tx[1].origin, TimePoint::new(ROOT, e_tx + 50_000_000));
    assert_eq!(tx[1].ended_at, None);
    assert_eq!(harness.rejected(), [REFUSED_CHANGE.to_owned()]);
    assert_eq!(harness.applied_of("radio.rx.s0.sample_rate_hz"), [(serde_json::json!(2_000_000.0), e_rx)]);
    assert_eq!(harness.applied_of("radio.tx.s0.sample_rate_hz"), [(serde_json::json!(2_000_000.0), e_tx)]);
}

#[test]
fn mr_20_a_transmit_change_with_no_sample_is_a_candidate() {
    // MR-20: a transmit clock's segment that a later booking cut at its own origin is still in
    // the plan, so the change that began it is a candidate; the later change then applies.
    let env = [("sim.faults", serde_json::json!([cold_change_refused_at(1_000_000)]))];
    let constraints = [
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.s0.sample_rate_hz", eq(Scalar::try_from(1_000_000.0).unwrap())),
    ];
    let mut harness = built(Layout { profile: "x3xx", id: "mock", rx: elements(&["0:A:ubx:0"]), tx: elements(&["0:A:ubx:0"]), selector: &[], constraints: &constraints, env: &env, ports: &[] });
    harness.arm_start(T0).unwrap();
    let e = T0 + 10_000_000;
    harness.push(update("radio.tx.s0.sample_rate_hz", Value::num(2_000_000.0).unwrap(), UpdateClass::Cold, Some(e)));
    harness.push(update("radio.tx.s0.sample_rate_hz", Value::num(4_000_000.0).unwrap(), UpdateClass::Cold, Some(e)));
    harness.step(0).unwrap();
    // The scene: both changes are booked, each with its clock.
    let booked = harness.clocks_of("mock/tx/s0");
    assert_eq!(booked.len(), 3, "{booked:?}");
    assert!(harness.applied_of("radio.tx.s0.sample_rate_hz").is_empty());
    harness.step(e).unwrap();
    let clocks = harness.clocks_of("mock/tx/s0");
    assert_eq!(clocks.len(), 3, "{clocks:?}");
    assert_eq!(clocks[0].ended_at, Some(TimePoint::new(ROOT, e)));
    assert_eq!((clocks[1].origin, clocks[1].ended_at), (TimePoint::new(ROOT, e + 50_000_000), Some(TimePoint::new(ROOT, e + 50_000_000))));
    // The later change's segment waits the refused clock's cut at its own origin plus the start
    // lead: T0 + 10 ms + 50 ms + 50 ms.
    assert_eq!((clocks[2].origin, clocks[2].ended_at), (TimePoint::new(ROOT, e + 100_000_000), None));
    assert_eq!(harness.rejected(), [REFUSED_CHANGE.to_owned()], "the later change was refused instead");
    harness.push(burst("mock/tx/s0", clocks[2].domain, 16_000, 100));
    harness.step(e).unwrap();
    harness.step(e + 110_000_000).unwrap();
    let bursts: Vec<ezsdr_kernel::stream::BurstRecord> = serde_json::from_value(serde_json::Value::Array(harness.section("bursts"))).unwrap();
    assert_eq!(bursts.len(), 1, "{bursts:?}");
    assert_eq!((bursts[0].target, bursts[0].samples), (TimePoint::new(clocks[2].domain, 16_000), 100));
}

/// MA1 (MR-34): a ubx receive layout looped back from one transmit stream at seed 7, with the
/// transmit burst queued and the Mock armed for `T0`; blocks are collected as it steps.
struct Ma1 {
    h: Harness,
    got: Vec<BlockRef>,
    /// The latest instant stepped to; `go` to an earlier one steps nowhere.
    now: i64,
}

impl Ma1 {
    fn new(rx: Value, constraints: &[(&str, Constraint)], rx_channels: &[u32], faults: serde_json::Value, ports: &[&str]) -> Ma1 {
        let couplings: Vec<_> = rx_channels.iter()
            .map(|rx| serde_json::json!({"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":rx,"gain_db":0.0,"delay_ns":0}))
            .collect();
        let env = [("sim.seed", serde_json::json!(7)), ("sim.channel", serde_json::json!({ "couplings": couplings })), ("sim.faults", faults)];
        let (waveform, bytes) = waveform_values(&vec![(0.5, 0.0); 400_000]);
        let mut h = built_with_inputs(Layout {
            profile: "x3xx", id: "mock", rx, tx: elements(&["0:A:ubx:0"]), selector: &[], constraints, env: &env, ports,
        }, BTreeMap::from([(waveform.hash.clone(), bytes)]));
        h.arm_start(T0).unwrap();
        let tx = h.clocks_of("mock/tx/s0")[0].domain;
        h.push(burst_with_waveform("mock/tx/s0", tx, 2_000_000, waveform));
        Ma1 { h, got: Vec::new(), now: 0 }
    }

    /// One ubx receive channel `c0` in stream 0, looped back.
    fn single(faults: serde_json::Value) -> Ma1 {
        Ma1::new(elements(&["0:A:ubx:0"]), &[("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))], &[0], faults, &["rx0"])
    }

    /// Steps to `tick` and keeps the blocks that came out.
    fn go(&mut self, tick: i64) {
        if tick < self.now { return; }
        self.now = tick;
        self.h.step(tick).unwrap();
        let ports: Vec<String> = self.h.links.keys().cloned().collect();
        for port in ports { self.got.extend(self.h.blocks(&port)); }
    }

    /// As `go`, in steps of at most 10 ms, so a link of 64 blocks drops none of what it holds.
    fn go_by(&mut self, tick: i64) {
        while self.now < tick { self.go((self.now + 10_000_000).min(tick)); }
    }

    fn push(&self, action: Action) { self.h.push(action); }

    /// A frequency update to the value the key already has (so the loopback stays tuned alike), due at `e`.
    fn tune(&self, name: &str, e: i64) { self.push(update(name, self.h.effective[&key(name)].clone(), UpdateClass::HardwareTimed, Some(e))); }

    /// The last SampleClock of the stream at `path` and its origin.
    fn last_clock(&self, path: &str) -> (SampleClockRecord, i64) {
        let clock = self.h.clocks_of(path).last().unwrap().clone();
        let origin = clock.origin.ticks_in(ROOT).unwrap();
        (clock, origin)
    }

    /// What channel 0 hears of the clock at `tick`.
    fn ear(&self, clock: &SampleClockRecord, tick: i64) -> (f32, f32) { heard(&self.got, clock, tick) }

    /// The `coherence` rows of the receive segment of stream `stream` that begins at `origin`.
    fn coherence(&self, stream: u64, origin: i64) -> Vec<serde_json::Value> {
        let segments = self.h.section("segments");
        let row = segments.iter().find(|row| row["direction"] == "rx" && row["stream"] == stream && row["origin"]["ticks"] == origin).expect("a segment row");
        row["coherence"].as_array().unwrap().clone()
    }

    /// Stops stream `path` at `from`, then restarts it, and returns the restarted clock and
    /// its origin: the stream's untimed entry tune is the draw that tells how many an earlier
    /// update took. The restart is booked after every earlier update has been applied.
    fn restart(&mut self, path: &str, from: i64) -> (SampleClockRecord, i64) {
        self.push(stop(path));
        self.go(from);
        self.push(start_rx(path, Some(from + 2_000_000)));
        self.go(from + 1_000_000);
        self.go(from + 2_000_001);
        self.go(from + 100_000_000);
        self.last_clock(path)
    }
}

/// The `n`th draw of the seed-7 retune stream.
fn retune_draw(n: usize) -> f64 {
    let mut rng = ezsdr_sim::SimRng::new(7, "mock/lo/retune");
    (0..=n).map(|_| phase(&mut rng)).last().unwrap()
}

/// The first four draws of the seed-7 LO stream: with one receive and one transmit channel, receive phase, transmit phase, receive timed phase, transmit timed phase (with more channels, all receive initials come first, then all transmit initials, then the timed constants).
fn lo_draws() -> [f64; 4] {
    let mut lo = ezsdr_sim::SimRng::new(7, "mock/lo");
    [phase(&mut lo), phase(&mut lo), phase(&mut lo), phase(&mut lo)]
}

const FREQ0: &str = "radio.rx.c0.frequency_hz";

#[test]
fn mr_34_a_layout_receive_update_before_t0_is_an_untimed_tune() {
    // MR-34: the update applies at T0 - 1 ms, before the first segment began, so no segment
    // covers e and it is an untimed tune.
    let mut m = Ma1::single(serde_json::json!([]));
    m.tune(FREQ0, T0 - 1_000_000);
    m.go(0);
    m.go(T0 + 10_000_000);
    let first = m.h.clocks_of("mock/rx/s0")[0].clone();
    let [_, tx_phase, ..] = lo_draws();
    assert_near(m.ear(&first, T0 + 5_000_000), rotated(0.5, tx_phase - retune_draw(0)));
    let coherence = m.coherence(0, T0);
    assert_eq!(coherence.last().unwrap()["basis"], "none");
    assert_eq!(coherence.last().unwrap()["valid"], false);
    assert_eq!(m.h.rejected(), Vec::<String>::new());
}

#[test]
fn mr_34_a_layout_deterministic_receive_update_before_t0_stays_zero() {
    // An `ideal` channel is deterministic: an update applied before its segment began takes
    // no draw, so its phase stays 0 and it stays coherent.
    let constraints = [("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let env = [
        ("sim.seed", serde_json::json!(7)),
        ("sim.channel", serde_json::json!({"couplings":[{"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0}]})),
    ];
    let (waveform, bytes) = waveform_values(&vec![(0.25, 0.0); 100_000]);
    let mut h = built_with_inputs(Layout { profile: "ideal", id: "mock", rx: count(1), tx: count(1), selector: &[], constraints: &constraints, env: &env, ports: &["rx0"] }, BTreeMap::from([(waveform.hash.clone(), bytes)]));
    h.arm_start(T0).unwrap();
    let tx = h.clocks_of("mock/tx/s0")[0].domain;
    h.push(burst_with_waveform("mock/tx/s0", tx, 2_000_000, waveform));
    let mut m = Ma1 { h, got: Vec::new(), now: 0 };
    m.tune(FREQ0, T0 - 1_000_000);
    m.go(0);
    m.go(T0 + 10_000_000);
    let first = m.h.clocks_of("mock/rx/s0")[0].clone();
    assert_eq!(m.ear(&first, T0 + 100_000), (0.25, 0.0));
    assert_eq!(m.ear(&first, T0 + 4_000_000), (0.25, 0.0));
    assert_eq!(m.coherence(0, T0).last().unwrap()["basis"], "deterministic");
    assert_eq!(m.coherence(0, T0).last().unwrap()["valid"], true);
    assert_eq!(m.h.rejected(), Vec::<String>::new());
}

#[test]
fn mr_34_a_layout_an_update_received_while_samples_pass_draws_nothing() {
    // r = T0 + 3 ms: the first segment covers the channel then and at e = T0 + 5 ms, so the
    // update is a timed tune and takes no draw; the restart's own entry tune is the first.
    let mut m = Ma1::single(serde_json::json!([]));
    m.go(0);
    m.go(T0 + 2_100_001);
    let first = m.h.clocks_of("mock/rx/s0")[0].clone();
    m.tune(FREQ0, T0 + 5_000_000);
    m.go(T0 + 3_000_000);
    m.go(T0 + 8_000_000);
    let [_, tx_phase, rx_timed, _] = lo_draws();
    assert_near(m.ear(&first, T0 + 5_500_000), rotated(0.5, tx_phase - rx_timed));
    let coherence = m.coherence(0, T0);
    assert_eq!(coherence.last().unwrap()["at"]["ticks"], T0 + 5_000_000);
    assert_eq!(coherence.last().unwrap()["basis"], "timed_tune");
    assert_eq!(coherence.last().unwrap()["valid"], true);
    let (restarted, origin) = m.restart("mock/rx/s0", T0 + 9_000_000);
    m.go(origin + 10_000_000);
    assert_near(m.ear(&restarted, origin + 100_000), rotated(0.5, tx_phase - retune_draw(0)));
    assert_eq!(m.h.rejected(), Vec::<String>::new());
}

#[test]
fn mr_34_a_layout_update_to_a_stopped_receive_stream_is_untimed_and_the_restart_adds_its_draw() {
    // Received at T0 + 6.5 ms with the stream stopped, so no segment covers e: untimed (draw 0); the restart's entry
    // tune is draw 1, which the restarted segment hears.
    let mut m = Ma1::single(serde_json::json!([]));
    m.go(0);
    m.go(T0 + 2_100_001);
    m.push(stop("mock/rx/s0"));
    m.go(T0 + 6_000_000);
    m.tune(FREQ0, T0 + 9_000_000);
    m.go(T0 + 6_500_000);
    m.go(T0 + 9_000_001);
    m.push(start_rx("mock/rx/s0", Some(T0 + 10_000_000)));
    m.go(T0 + 10_000_000);
    m.go(T0 + 100_000_000);
    let (restarted, origin) = m.last_clock("mock/rx/s0");
    m.go(origin + 10_000_000);
    let [_, tx_phase, ..] = lo_draws();
    assert_near(m.ear(&restarted, origin + 100_000), rotated(0.5, tx_phase - retune_draw(1)));
    let coherence = m.coherence(0, origin);
    assert_eq!(coherence.first().unwrap()["basis"], "none");
    assert_eq!(coherence.first().unwrap()["valid"], false);
    assert_eq!(m.h.rejected(), Vec::<String>::new());
}

#[test]
fn mr_34_a_layout_update_before_a_planned_segment_is_untimed_and_the_segment_adds_its_draw() {
    // The channel leaves its stream at T0 + 6 ms and a segment to bring it back is booked for
    // T0 + 8 ms; an update received after both bookings, to take effect before that segment
    // begins, is untimed: draw 0, then the planned segment's entry tune is draw 1.
    let mut m = Ma1::single(serde_json::json!([]));
    m.go(0);
    m.go(T0 + 2_100_001);
    m.push(update("radio.rx.c0.stream", Value::from(-1), UpdateClass::Cold, Some(T0 + 6_000_000)));
    m.push(update("radio.rx.c0.stream", Value::from(0), UpdateClass::Cold, Some(T0 + 8_000_000)));
    m.go(T0 + 3_000_000);
    m.tune(FREQ0, T0 + 20_000_000);
    m.go(T0 + 6_500_000);
    m.go(T0 + 100_000_000);
    let (entered, origin) = m.last_clock("mock/rx/s0");
    assert!(origin > T0 + 20_000_000, "{origin}");
    m.go(origin + 10_000_000);
    let [_, tx_phase, ..] = lo_draws();
    assert_near(m.ear(&entered, origin + 100_000), rotated(0.5, tx_phase - retune_draw(1)));
    let coherence = m.coherence(0, origin);
    assert_eq!(coherence.first().unwrap()["basis"], "none");
    assert_eq!(coherence.first().unwrap()["valid"], false);
    assert_eq!(m.h.rejected(), Vec::<String>::new());
}

#[test]
fn mr_34_a_layout_an_update_at_a_cut_is_untimed() {
    // The update is received at T0 + 1 ms, while the segment runs, and a stop at T0 + 3.5 ms
    // cuts the segment at c. With e = c exactly the segment no longer covers the channel when
    // the update applies: untimed. The cut is read in a first run.
    let run = |e: i64| {
        let mut m = Ma1::single(serde_json::json!([]));
        m.go(0);
        m.go(T0 + 500_000);
        m.tune(FREQ0, e);
        m.go(T0 + 1_000_000);
        m.push(stop("mock/rx/s0"));
        m.go(T0 + 3_500_000);
        let cut = m.h.clocks_of("mock/rx/s0")[0].ended_at.map(|at| at.ticks_in(ROOT).unwrap());
        (m, cut)
    };
    let cut = run(T0 + 10_000_000).1.expect("the stop cut the segment");
    assert!(cut >= T0 + 3_000_000, "{cut}");
    let (mut m, _) = run(cut);
    m.go(cut + 1);
    assert!(m.h.applied().iter().any(|(key, at)| key == FREQ0 && *at == cut));
    m.push(start_rx("mock/rx/s0", Some(cut + 4_000_000)));
    m.go(cut + 2_000_000);
    m.go(cut + 100_000_000);
    let (restarted, origin) = m.last_clock("mock/rx/s0");
    m.go(origin + 10_000_000);
    let [_, tx_phase, ..] = lo_draws();
    assert_near(m.ear(&restarted, origin + 100_000), rotated(0.5, tx_phase - retune_draw(1)));
    assert_eq!(m.h.rejected(), Vec::<String>::new());
}

#[test]
fn mr_34_a_layout_deterministic_and_cbx_receive_channels_under_the_new_rule() {
    // A `random` (CBX) channel draws one phase for every applied update, timed or not and
    // never coherent: c0's update applies before its segment began (untimed), c1's after it
    // has run the release window (timed), and they take retune draws 0 and 1.
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
    let mut h = built_with_inputs(Layout {
        profile: "x3xx", id: "mock", rx: elements(&["0:A:cbx:0", "0:B:cbx:0"]),
        tx: elements(&["0:A:cbx:0", "0:B:cbx:0"]), selector: &[], constraints: &constraints, env: &env, ports: &["rx0", "rx1"],
    }, BTreeMap::from([(waveform.hash.clone(), bytes)]));
    h.arm_start(T0).unwrap();
    let (tx0, tx1) = (h.clocks_of("mock/tx/s0")[0].domain, h.clocks_of("mock/tx/s1")[0].domain);
    h.push(burst_with_waveform("mock/tx/s0", tx0, 2_000_000, waveform.clone()));
    h.push(burst_with_waveform("mock/tx/s1", tx1, 2_000_000, waveform));
    let mut m = Ma1 { h, got: Vec::new(), now: 0 };
    m.tune(FREQ0, T0 - 1_000_000);
    m.go(0);
    m.go(T0 + 2_100_001);
    m.tune("radio.rx.c1.frequency_hz", T0 + 5_000_000);
    m.go(T0 + 3_000_000);
    m.go(T0 + 9_000_000);
    let mut lo = ezsdr_sim::SimRng::new(19, "mock/lo");
    let (_, _, tx0_phase, tx1_phase) = (phase(&mut lo), phase(&mut lo), phase(&mut lo), phase(&mut lo));
    let mut retune = ezsdr_sim::SimRng::new(19, "mock/lo/retune");
    let (c0_draw, c1_draw) = (phase(&mut retune), phase(&mut retune));
    let (s0, s1) = (m.h.clocks_of("mock/rx/s0")[0].clone(), m.h.clocks_of("mock/rx/s1")[0].clone());
    assert_near(m.ear(&s0, T0 + 7_000_000), rotated(0.5, tx0_phase - c0_draw));
    assert_near(m.ear(&s1, T0 + 7_000_000), rotated(0.5, tx1_phase - c1_draw));
    for (stream, row) in [(0, m.coherence(0, T0)), (1, m.coherence(1, T0))] {
        assert_eq!(row.last().unwrap()["basis"], "none", "stream {stream}");
        assert_eq!(row.last().unwrap()["valid"], false, "stream {stream}");
    }
    assert_eq!(m.h.rejected(), Vec::<String>::new());
}

#[test]
fn mr_30_a_layout_untimed_update_draws_are_reproducible_and_in_application_order() {
    // Two channels of one stream, each looped back from its own transmit stream, both updated
    // before the segment began (untimed) for one instant, received c1 first: c1 takes retune
    // draw 0 and c0 draw 1, in both runs.
    let constraints = [("radio.tx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c1.stream", eq(Scalar::from(1))), ("radio.rx.c1.stream", eq(Scalar::from(0)))];
    let env = [
        ("sim.seed", serde_json::json!(7)),
        ("sim.channel", serde_json::json!({"couplings":[
            {"tx":"radio","tx_channel":0,"rx":"radio","rx_channel":0,"gain_db":0.0,"delay_ns":0},
            {"tx":"radio","tx_channel":1,"rx":"radio","rx_channel":1,"gain_db":0.0,"delay_ns":0}
        ]})),
    ];
    let run = || {
        let (waveform, bytes) = waveform_values(&vec![(0.5, 0.0); 10_000]);
        let mut h = built_with_inputs(Layout {
            profile: "x3xx", id: "mock", rx: two(), tx: two(), selector: &[], constraints: &constraints, env: &env, ports: &["rx0"],
        }, BTreeMap::from([(waveform.hash.clone(), bytes)]));
        h.arm_start(T0).unwrap();
        let (tx0, tx1) = (h.clocks_of("mock/tx/s0")[0].domain, h.clocks_of("mock/tx/s1")[0].domain);
        h.push(burst_with_waveform("mock/tx/s0", tx0, 2_000_000, waveform.clone()));
        h.push(burst_with_waveform("mock/tx/s1", tx1, 2_000_000, waveform));
        let mut m = Ma1 { h, got: Vec::new(), now: 0 };
        m.tune("radio.rx.c1.frequency_hz", T0 - 1_000_000);
        m.tune(FREQ0, T0 - 1_000_000);
        m.go(0);
        m.go(T0 + 9_000_000);
        let first = m.h.clocks_of("mock/rx/s0")[0].clone();
        (heard_channel(&m.got, &first, T0 + 5_000_000, 0), heard_channel(&m.got, &first, T0 + 5_000_000, 1))
    };
    let (c0, c1) = run();
    assert_eq!((c0, c1), run());
    let mut lo = ezsdr_sim::SimRng::new(7, "mock/lo");
    let (_, _, tx0_phase, tx1_phase) = (phase(&mut lo), phase(&mut lo), phase(&mut lo), phase(&mut lo));
    assert_near(c0, rotated(0.5, tx0_phase - retune_draw(1)));
    assert_near(c1, rotated(0.5, tx1_phase - retune_draw(0)));
}

#[test]
fn mr_34_a_layout_a_transmit_update_before_t0_stays_a_timed_tune() {
    // A transmit update is a timed tune whenever it applies; it takes no draw, so the receive
    // restart's entry tune is retune draw 0.
    let mut m = Ma1::single(serde_json::json!([]));
    m.tune("radio.tx.c0.frequency_hz", T0 - 1_000_000);
    m.go(0);
    m.go(T0 + 8_000_000);
    let first = m.h.clocks_of("mock/rx/s0")[0].clone();
    let [rx_phase, _, _, tx_timed] = lo_draws();
    assert_near(m.ear(&first, T0 + 5_000_000), rotated(0.5, tx_timed - rx_phase));
    let segments = m.h.section("segments");
    let tx = segments.iter().filter(|row| row["direction"] == "tx").flat_map(|row| row["coherence"].as_array().unwrap().clone()).collect::<Vec<_>>();
    assert_eq!(tx.last().unwrap()["basis"], "timed_tune");
    assert_eq!(tx.last().unwrap()["valid"], true);
    assert_eq!(tx.last().unwrap()["at"]["ticks"], T0 - 1_000_000);
    let (restarted, origin) = m.restart("mock/rx/s0", T0 + 9_000_000);
    m.go(origin + 10_000_000);
    assert_near(m.ear(&restarted, origin + 100_000), rotated(0.5, tx_timed - retune_draw(0)));
    assert_eq!(m.h.rejected(), Vec::<String>::new());
}

#[test]
fn mr_34_a_layout_the_other_streams_segment_covers() {
    // Channel c0 is in stream 1 and c1 in stream 0. The update to c0 is due while stream 1's
    // segment has run the release window, so it is timed; stream 0 alone would not cover c0.
    let constraints = [
        ("radio.rx.c0.stream", eq(Scalar::from(1))), ("radio.rx.c1.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0))),
    ];
    let mut m = Ma1::new(two(), &constraints, &[0, 1], serde_json::json!([]), &["rx0", "rx1"]);
    m.go(0);
    m.go(T0 + 2_100_001);
    m.tune(FREQ0, T0 + 5_000_000);
    m.go(T0 + 3_000_000);
    m.go(T0 + 8_000_000);
    let s1 = m.h.clocks_of("mock/rx/s1")[0].clone();
    let [_, _, tx_phase, rx0_timed] = lo_draws();
    assert_near(m.ear(&s1, T0 + 6_000_000), rotated(0.5, tx_phase - rx0_timed));
    let (restarted, origin) = m.restart("mock/rx/s1", T0 + 9_000_000);
    m.go(origin + 10_000_000);
    assert_near(m.ear(&restarted, origin + 100_000), rotated(0.5, tx_phase - retune_draw(0)));
    assert_eq!(m.h.rejected(), Vec::<String>::new());
}

#[test]
fn mr_34_a_layout_a_linkless_stream_does_not_cover_even_after_start_rx() {
    // Stream 1 (c1) has no link. Even after a `start_rx` has begun a segment of it, an update
    // to c1 is untimed (draw 0) and stream 0's restart takes draw 1.
    let constraints = [("radio.rx.c1.stream", eq(Scalar::from(1))), ("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let mut m = Ma1::new(two(), &constraints, &[0], serde_json::json!([]), &["rx0"]);
    m.go(0);
    m.go(T0 + 2_100_001);
    m.push(start_rx("mock/rx/s1", Some(T0 + 3_000_000)));
    m.go(T0 + 3_000_000);
    m.go(T0 + 120_000_000);
    m.tune("radio.rx.c1.frequency_hz", T0 + 135_000_000);
    m.go(T0 + 130_000_000);
    m.go(T0 + 136_000_000);
    assert!(m.h.clocks_of("mock/rx/s1").is_empty());
    assert!(m.h.section("segments").iter().all(|row| !(row["direction"] == "rx" && row["stream"] == 1)));
    let (restarted, origin) = m.restart("mock/rx/s0", T0 + 140_000_000);
    m.go(origin + 10_000_000);
    let tx_phase = lo_draws()[2];
    assert_near(m.ear(&restarted, origin + 100_000), rotated(0.5, tx_phase - retune_draw(1)));
    assert_eq!(m.h.rejected(), Vec::<String>::new());
}

#[test]
fn mr_34_a_layout_a_stop_after_the_update_makes_it_untimed_at_application() {
    // Due at T0 + 8 ms, while the segment runs when it is received (T0 + 3 ms), but a stop at
    // T0 + 4 ms cuts it before the update applies: no segment covers e, so it is untimed.
    let mut m = Ma1::single(serde_json::json!([]));
    m.go(0);
    m.go(T0 + 2_100_001);
    m.tune(FREQ0, T0 + 8_000_000);
    m.go(T0 + 3_000_000);
    m.push(stop("mock/rx/s0"));
    m.go(T0 + 4_000_000);
    m.push(start_rx("mock/rx/s0", Some(T0 + 10_000_000)));
    m.go(T0 + 5_000_000);
    m.go(T0 + 8_000_001);
    m.go(T0 + 100_000_000);
    let (restarted, origin) = m.last_clock("mock/rx/s0");
    assert!(origin > T0 + 8_000_000, "{origin}");
    m.go(origin + 10_000_000);
    let tx_phase = lo_draws()[1];
    assert_near(m.ear(&restarted, origin + 100_000), rotated(0.5, tx_phase - retune_draw(1)));
    let coherence = m.coherence(0, origin);
    assert_eq!(coherence.first().unwrap()["basis"], "none");
    assert_eq!(coherence.first().unwrap()["valid"], false);
    assert_eq!(m.h.rejected(), Vec::<String>::new());
}

#[test]
fn mr_34_a_layout_an_update_received_while_stopped_is_timed_when_a_later_segment_covers_e() {
    // Received at T0 + 2 ms with the stream stopped; the restart's segment (begun about
    // T0 + 70 ms) has run the release window when the update applies at e = T0 + 120 ms, so it
    // is a timed tune whenever it was received: the restart's entry tune is draw 0 and the
    // update takes no draw.
    let mut m = Ma1::single(serde_json::json!([]));
    m.go(0);
    m.go(T0 + 2_100_001);
    m.push(stop("mock/rx/s0"));
    m.go(T0 + 1_000_000 + 1_100_001);
    m.tune(FREQ0, T0 + 120_000_000);
    m.go(T0 + 3_000_000);
    m.push(start_rx("mock/rx/s0", Some(T0 + 20_000_000)));
    m.go(T0 + 4_000_000);
    m.go(T0 + 100_000_000);
    let (restarted, origin) = m.last_clock("mock/rx/s0");
    assert!(origin + 3_000_000 <= T0 + 120_000_000, "{origin}");
    m.go(T0 + 130_000_000);
    let [_, tx_phase, rx_timed, _] = lo_draws();
    assert_near(m.ear(&restarted, origin + 100_000), rotated(0.5, tx_phase - retune_draw(0)));
    assert_near(m.ear(&restarted, T0 + 125_000_000), rotated(0.5, tx_phase - rx_timed));
    let coherence = m.coherence(0, origin);
    assert_eq!(coherence.last().unwrap()["at"]["ticks"], T0 + 120_000_000);
    assert_eq!(coherence.last().unwrap()["basis"], "timed_tune");
    assert_eq!(coherence.last().unwrap()["valid"], true);
    assert_eq!(m.h.rejected(), Vec::<String>::new());
}

#[test]
fn mr_34_a_layout_a_flip_flop_is_timed_by_the_segment_that_covers_e() {
    // Received at T0 + 3 ms while segment S runs. S is stopped and a new segment S' begins
    // well before e = T0 + 150 ms: S' has run the release window when the update applies, so
    // it is a timed tune, though S' is not the segment that ran when it was received. S''s
    // entry tune is draw 0; the update takes none.
    let mut m = Ma1::single(serde_json::json!([]));
    m.go(0);
    m.go(T0 + 2_100_001);
    m.tune(FREQ0, T0 + 150_000_000);
    m.go(T0 + 3_000_000);
    m.push(stop("mock/rx/s0"));
    m.go(T0 + 10_000_000);
    m.push(start_rx("mock/rx/s0", Some(T0 + 20_000_000)));
    m.go(T0 + 11_000_000);
    m.go(T0 + 100_000_000);
    let (restarted, origin) = m.last_clock("mock/rx/s0");
    assert!(origin + 3_000_000 <= T0 + 150_000_000 && origin > T0, "{origin}");
    m.go(T0 + 160_000_000);
    let [_, tx_phase, rx_timed, _] = lo_draws();
    assert_near(m.ear(&restarted, origin + 100_000), rotated(0.5, tx_phase - retune_draw(0)));
    assert_near(m.ear(&restarted, T0 + 155_000_000), rotated(0.5, tx_phase - rx_timed));
    let coherence = m.coherence(0, origin);
    assert_eq!(coherence.last().unwrap()["at"]["ticks"], T0 + 150_000_000);
    assert_eq!(coherence.last().unwrap()["basis"], "timed_tune");
    assert_eq!(coherence.last().unwrap()["valid"], true);
    assert_eq!(m.h.rejected(), Vec::<String>::new());
}

#[test]
fn mr_34_a_layout_an_update_received_before_t0_is_timed_when_its_segment_has_run_the_release_window() {
    // Received at 0, before T0, as a Spec Run's scheduled update is (KC-17, KC-18): the first
    // segment (origin T0) has run the release window when it applies at T0 + 5 ms, so it is a
    // timed tune and takes no draw (the restart's entry tune is draw 0).
    let mut m = Ma1::single(serde_json::json!([]));
    m.tune(FREQ0, T0 + 5_000_000);
    m.go(0);
    m.go(T0 + 10_000_000);
    let first = m.h.clocks_of("mock/rx/s0")[0].clone();
    let [rx_phase, tx_phase, rx_timed, _] = lo_draws();
    assert_near(m.ear(&first, T0 + 3_000_000), rotated(0.5, tx_phase - rx_phase));
    assert_near(m.ear(&first, T0 + 7_000_000), rotated(0.5, tx_phase - rx_timed));
    let coherence = m.coherence(0, T0);
    assert_eq!(coherence.last().unwrap()["at"]["ticks"], T0 + 5_000_000);
    assert_eq!(coherence.last().unwrap()["basis"], "timed_tune");
    assert_eq!(coherence.last().unwrap()["valid"], true);
    let (restarted, origin) = m.restart("mock/rx/s0", T0 + 11_000_000);
    m.go(origin + 10_000_000);
    assert_near(m.ear(&restarted, origin + 100_000), rotated(0.5, tx_phase - retune_draw(0)));
    assert_eq!(m.h.rejected(), Vec::<String>::new());
}

#[test]
fn mr_34_a_receive_update_is_timed_from_the_release_window_after_the_segment_origin() {
    // A receive update is a timed tune when the segment that covers its instant began at least
    // the release window (the description's, RM-6) before it. Segment origin T0: e = T0 + W is
    // timed and one root tick earlier is not, whether the update is received before T0 or by a
    // Session at T0 with the Mock's 2 ms command lead (e = T0 + 2 ms, inside the window); a
    // Session update received at T0 + 5 ms with that lead is timed.
    let window = ezsdr_radio::x3xx::timing(&ezsdr_mock_radio::Profile::terms()).release_window_ns;
    assert!(window > 2_000_000, "the Session lead case needs a window longer than the lead");
    // `received`: the instant the update is handed over; `due`: a requested instant, or none.
    let run = |received: i64, due: Option<i64>| {
        let mut m = Ma1::single(serde_json::json!([]));
        if received > 0 { m.go(0); }
        let value = m.h.effective[&key(FREQ0)].clone();
        m.push(update(FREQ0, value, UpdateClass::HardwareTimed, due));
        m.go(received);
        m.go(T0 + 20_000_000);
        let first = m.h.clocks_of("mock/rx/s0")[0].clone();
        let [_, tx_phase, rx_timed, _] = lo_draws();
        let e = m.h.applied_of(FREQ0)[0].1;
        let coherence = m.coherence(0, T0).last().unwrap().clone();
        let timed = coherence["basis"] == "timed_tune";
        let after = m.ear(&first, e + 1_000_000);
        assert_near(after, if timed { rotated(0.5, tx_phase - rx_timed) } else { rotated(0.5, tx_phase - retune_draw(0)) });
        assert_eq!(coherence["at"]["ticks"], e);
        (e, timed)
    };
    assert_eq!(run(0, Some(T0 + window)), (T0 + window, true));
    assert_eq!(run(0, Some(T0 + window - 1)), (T0 + window - 1, false));
    assert_eq!(run(T0, None), (T0 + 2_000_000, false));
    assert_eq!(run(T0 + 5_000_000, None), (T0 + 7_000_000, true));
}

/// MA1: the rate changes the refusal tests book (receive stream 0, one `cold` segment each).
const E_C1: i64 = T0 + 10_000_000;
const E_C2: i64 = T0 + 70_000_000;

/// One run of the refusal scene: C1 (due `E_C1`) and, with `second`, C2 (due `E_C2`) are booked
/// at T0 + 3 ms; the cold-change fault, when `faulted`, refuses C1. An `update` is a frequency
/// update received at `r` for `e`. Runs to T0 + 250 ms.
fn refusal_run(faulted: bool, second: bool, update: Option<(i64, i64)>) -> Ma1 {
    let mut m = Ma1::single(if faulted { serde_json::json!([cold_change_refused_at(1_000_000)]) } else { serde_json::json!([]) });
    m.go(0);
    m.go(T0 + 2_100_001);
    m.push(update_rate(2_000_000.0, E_C1));
    if second { m.push(update_rate(1_000_000.0, E_C2)); }
    m.go(T0 + 3_000_000);
    m.go(E_C1 + 1);
    if let Some((r, e)) = update {
        assert!(r > E_C1 && e >= r + 2_000_000);
        m.tune(FREQ0, e);
        m.go(r);
    }
    m.go_by(T0 + 250_000_000);
    m
}

fn update_rate(rate: f64, e: i64) -> Action { update("radio.rx.s0.sample_rate_hz", Value::num(rate).unwrap(), UpdateClass::Cold, Some(e)) }

fn origins(m: &Ma1) -> Vec<i64> { m.h.clocks_of("mock/rx/s0").iter().map(|clock| clock.origin.ticks_in(ROOT).unwrap()).collect() }

#[test]
fn mr_34_a_layout_a_refused_segment_does_not_cover_an_update_received_after_its_planned_origin() {
    // C1 would begin S2 at o2. Received at o2 + 3 ms, an update is a timed tune when C1 is
    // not refused (S2 runs); when C1 is refused S2 never exists and S1 is cut at C1, so
    // nothing covers the channel: untimed (draw 0), and the restart's entry tune is draw 1; a
    // timed update would leave the restart draw 0.
    let calibration = refusal_run(false, false, None);
    let o2 = origins(&calibration)[1];
    let (r, e) = (o2 + 3_000_000, o2 + 6_000_000);
    let timed = refusal_run(false, false, Some((r, e)));
    let coherence = timed.coherence(0, o2);
    assert_eq!(coherence.last().unwrap()["at"]["ticks"], e);
    assert_eq!(coherence.last().unwrap()["basis"], "timed_tune");
    let [_, tx_phase, rx_timed, _] = lo_draws();
    assert_near(timed.ear(&timed.last_clock("mock/rx/s0").0, e + 3_000_000), rotated(0.5, tx_phase - rx_timed));
    let mut m = refusal_run(true, false, Some((r, e)));
    assert_eq!(m.h.rejected(), [REFUSED_CHANGE.to_owned()]);
    assert_eq!(origins(&m).len(), 1, "{:?}", origins(&m));
    assert!(m.h.section("segments").iter().all(|row| row["origin"]["ticks"] != o2), "the refused segment left a row");
    let (restarted, origin) = m.restart("mock/rx/s0", T0 + 260_000_000);
    m.go(origin + 10_000_000);
    assert_near(m.ear(&restarted, origin + 100_000), rotated(0.5, tx_phase - retune_draw(1)));
}

#[test]
fn mr_34_a_layout_an_update_at_a_cold_instant_is_timed_whatever_the_refusal_order() {
    // C1 is due off the sample lattice. The update, received while S1 runs, is due at the
    // same instant; whether it arrives before or after C1 and whether C1 is refused, S1 is
    // the segment that covers it when it applies (S1 is cut after e): timed, no draw.
    let e_c = T0 + 6_000_500;
    for update_first in [true, false] {
        let mut m = Ma1::single(serde_json::json!([cold_change_refused_at(1_000_000)]));
        m.go(0);
        m.go(T0 + 2_100_001);
        if update_first { m.tune(FREQ0, e_c); }
        m.push(update_rate(2_000_000.0, e_c));
        if !update_first { m.tune(FREQ0, e_c); }
        m.go(T0 + 3_000_000);
        m.go(T0 + 20_000_000);
        assert_eq!(m.h.rejected(), [REFUSED_CHANGE.to_owned()], "update first {update_first}");
        assert!(m.h.applied().iter().any(|(key, at)| key == FREQ0 && *at == e_c), "{:?}", m.h.applied());
        let coherence = m.coherence(0, T0);
        assert_eq!(coherence.last().unwrap()["basis"], "timed_tune", "update first {update_first}");
        assert_eq!(coherence.last().unwrap()["valid"], true, "update first {update_first}");
        let (restarted, origin) = m.restart("mock/rx/s0", T0 + 30_000_000);
        m.go(origin + 10_000_000);
        let tx_phase = lo_draws()[1];
        assert_near(m.ear(&restarted, origin + 100_000), rotated(0.5, tx_phase - retune_draw(0)));
    }
}

/// The origins of the refusal scene, in nanoseconds after T0, measured on the tree before RM-25
/// kept a refusal from moving a later origin (design 43): S2 with C1 not refused, S3 after it, and
/// the earlier origin S3 had with C1 refused. A refused C1 now leaves S3 at the second.
const REFUSAL_O2: i64 = 68_000_000;
const REFUSAL_O3: i64 = 127_000_000;
const REFUSAL_EARLY: i64 = 70_000_000;

/// The origins the refusal scene's segments have without the fault and with C1 refused.
fn refusal_origins() -> (Vec<i64>, Vec<i64>) {
    (origins(&refusal_run(false, true, None)), origins(&refusal_run(true, true, None)))
}

#[test]
fn mr_34_a_layout_a_refusal_does_not_move_the_next_segment_s_origin() {
    // RM-25 (design 43): C1 is refused, so S2 never runs, but S3 begins where it would have
    // behind S2's cut: the origins match the fault-free scene's S1 and S3, and the measured ones.
    let (plain, refused) = refusal_origins();
    assert_eq!(plain, [T0, T0 + REFUSAL_O2, T0 + REFUSAL_O3]);
    assert_eq!(refused, [T0, T0 + REFUSAL_O3]);
    assert_eq!(refused[1], plain[2]);
}

/// The refusal scene with an update `r` received and due at `e`, both before S3 begins at `o3`:
/// no segment covers `e`, so the update is untimed, takes the first retune draw, and S3's own
/// origin tune takes the second.
fn assert_an_update_in_a_refused_span_is_untimed(r: i64, e: i64) {
    let o3 = T0 + REFUSAL_O3;
    assert!(r > E_C1 && e < o3 && e >= r + 2_000_000, "{r} {e}");
    let mut m = refusal_run(true, true, Some((r, e)));
    assert_eq!(m.h.rejected(), [REFUSED_CHANGE.to_owned()]);
    assert_eq!(origins(&m), [T0, o3]);
    // S1 is cut at C1, S2 is not a segment and S3 begins at o3: nothing covers e.
    let rows = m.h.section("segments");
    assert!(rows.iter().all(|row| row["direction"] != "rx" || row["origin"]["ticks"] == T0 || row["origin"]["ticks"] == o3), "a refused segment left a row: {rows:?}");
    assert!(
        rows.iter().all(|row| row["direction"] != "rx" || row["origin"]["ticks"].as_i64().unwrap() > e || row["ended_at"]["ticks"].as_i64().is_some_and(|end| end <= e)),
        "a segment covers {e}: {rows:?}",
    );
    assert!(m.h.applied().iter().any(|(key, at)| key == FREQ0 && *at == e), "{:?}", m.h.applied());
    // The update is untimed: S3's entry row says no coherent basis, and no timed row follows it.
    let coherence = m.coherence(0, o3);
    assert_eq!(coherence.len(), 1, "{coherence:?}");
    assert_eq!(coherence[0]["basis"], "none");
    assert_eq!(coherence[0]["valid"], false);
    // Draw 0 is the update, draw 1 is S3's origin tune; a timed tune would leave draw 0 to S3.
    let (s3, origin) = m.last_clock("mock/rx/s0");
    assert_eq!(origin, o3);
    let tx_phase = lo_draws()[1];
    assert_near(m.ear(&s3, origin + 100_000), rotated(0.5, tx_phase - retune_draw(1)));
    assert_near(m.ear(&s3, origin + 5_000_000), rotated(0.5, tx_phase - retune_draw(1)));
    // Restarting takes the third draw: no other tune was taken.
    let (restarted, restart_origin) = m.restart("mock/rx/s0", T0 + 260_000_000);
    m.go(restart_origin + 10_000_000);
    assert_near(m.ear(&restarted, restart_origin + 100_000), rotated(0.5, tx_phase - retune_draw(2)));
}

#[test]
fn mr_34_a_layout_an_update_in_the_span_a_refusal_once_advanced_is_untimed() {
    // C1 (refused) would begin S2; C2, booked after it, begins S3 at o3. An update received
    // before the origin S3 had when a refusal moved it (T0 + 70 ms) and due 5 ms after that origin,
    // before o3, is untimed, however the refusal and the update were ordered: nothing runs there.
    assert_an_update_in_a_refused_span_is_untimed(T0 + REFUSAL_EARLY - 5_000_000, T0 + REFUSAL_EARLY + 5_000_000);
}

#[test]
fn mr_34_a_layout_an_update_received_after_the_origin_a_refusal_once_gave_is_untimed() {
    // As above, but received after T0 + 70 ms, the origin S3 had on the old rule, and due 5 ms
    // later: S3 has not begun, so it is untimed all the same.
    assert_an_update_in_a_refused_span_is_untimed(T0 + REFUSAL_EARLY + 5_000_000, T0 + REFUSAL_EARLY + 10_000_000);
}

#[test]
fn mr_34_a_layout_a_transmit_origin_s_untimed_tune_follows_a_timed_update_at_its_instant() {
    // A timed transmit frequency update due at the origin of a transmit segment a rate change
    // begins: the origin's untimed tune is taken after it (MR-34), so the burst on that clock
    // radiates the untimed draw, not the timed constant, and the segment is `none` there. The
    // origin is known once the segment is booked, so the scenario runs twice.
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
        harness.step(T0 + 3_000_000).unwrap();
        harness.push(update_at("mock/tx/s0", "radio.tx.s0.sample_rate_hz", Value::num(1.0e6).unwrap(), UpdateClass::Cold, Some(T0 + 10_000_000)));
        harness.step(T0 + 10_000_001).unwrap();
        if let Some(origin) = at_origin {
            let same = harness.effective[&key("radio.tx.c0.frequency_hz")].clone();
            harness.push(update("radio.tx.c0.frequency_hz", same, UpdateClass::HardwareTimed, Some(origin)));
        }
        harness.step(T0 + 30_000_000).unwrap();
        let s0 = harness.clocks_of("mock/tx/s0").last().unwrap().clone();
        (s0.origin.ticks_in(ROOT).unwrap(), harness, s0, waveform)
    };
    let (origin, ..) = run(None);
    assert!(origin > T0 + 30_000_000, "{origin}");
    let (rerun, mut harness, s0, waveform) = run(Some(origin));
    assert_eq!(rerun, origin);
    harness.step(origin + 1_000_000).unwrap();
    assert!(harness.applied().iter().any(|(key, at)| key == "radio.tx.c0.frequency_hz" && *at == origin));
    harness.push(burst_with_waveform("mock/tx/s0", s0.domain, sample_of(&s0, T0 + 150_000_000), waveform));
    let blocks = collect_blocks(&mut harness, origin + 20_000_000, T0 + 300_000_000);
    assert_eq!(harness.rejected(), Vec::<String>::new());
    let [rx_phase, _, _, tx_timed] = lo_draws();
    let tx_retuned = retune_draw(0);
    assert_ne!(tx_retuned, tx_timed);
    let rx_clock = harness.clocks_of("mock/rx/s0")[0].clone();
    assert_near(heard(&blocks, &rx_clock, T0 + 160_000_000), rotated(0.5, tx_retuned - rx_phase));
    let segment = harness.section("segments").into_iter().find(|row| row["direction"] == "tx" && row["stream"] == 0 && row["origin"]["ticks"] == origin).unwrap();
    let at_origin = segment["coherence"].as_array().unwrap().iter().rfind(|entry| entry["at"]["ticks"] == origin).unwrap().clone();
    assert_eq!(at_origin["basis"], "none");
    assert_eq!(at_origin["valid"], false);
}
