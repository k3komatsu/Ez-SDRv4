use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use ezsdr_kernel::binding::{AdmissionCheckRegistry, Binding, Violation};
use ezsdr_kernel::event::{Action, ActionId, EventCollector};
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::{ClockDomainId, ModuleId, ResourceId, RunId};
use ezsdr_kernel::manifest::ArtifactRef;
use ezsdr_kernel::module_api::{
    ActionReceiver, ActionSubmitter, AttachedPort, Endpoint, ExecutionClass, Factories,
    KERNEL_API, ModuleError, ModuleRef, Pacing, PrepareContext, ProfileRef, Provider, Role, StopMode, UpdateClass, Version,
};
use ezsdr_kernel::plan::Fragment;
use ezsdr_kernel::policy::{EventKindRegistry, Policy};
use ezsdr_kernel::spec::{Constraint, Ident, Key, Namespace, Value};
use ezsdr_kernel::stream::{BackPressure, BlockFlags, BlockHeader, BlockRef, BurstEnd, BurstOpen, BurstStep, BurstTracker, DataLink, Direction, DropCarry, LatePolicy, PublishOutcome};
use ezsdr_kernel::time::{AbsoluteDeadline, ClockDomain, ClockRegistry, Duration, EpochRef, ManualTimeAuthority, Rational, RelativeBudget, TimePoint};
use ezsdr_kernel::module_api::Requested;

use ezsdr_mock_radio::{descriptor, MockRadio};
use ezsdr_radio::payloads::{RxOverflowCause, RxOverflowPayload};

const ROOT: ClockDomainId = ClockDomainId::local(7);

fn module_ref() -> ModuleRef {
    ModuleRef { id: ModuleId::parse("ezsdr.radio.mock").unwrap(), version: Version::new(1, 3, 0) }
}

fn waveform(samples: usize) -> ArtifactRef {
    let bytes = vec![0; samples * 8];
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

fn tx_action(domain: ClockDomainId, target: i64, samples: usize, repeat: bool, late_policy: LatePolicy) -> Action {
    Action::TxBurst {
        target: rid("mock/tx"),
        waveform: waveform(samples),
        repeat,
        at: AbsoluteDeadline::new(TimePoint::new(domain, target)),
        requested_at: None,
        late_policy,
        metadata: BTreeMap::new(),
    }
}

fn update_action(name: &str, value: Value, class: UpdateClass, at: Option<TimePoint>) -> Action {
    Action::UpdateParameter {
        target: rid("mock"),
        key: key(name),
        value,
        class,
        at: at.map(ezsdr_kernel::time::AbsoluteDeadline::new),
    }
}

fn tx_harness(profile: &str) -> Harness {
    Harness::new(profile, &[("radio.tx.channels", eq(Value::Int(1)))], &[], &[], None)
}

fn tx_domain(harness: &Harness) -> ClockDomainId {
    harness.clocks.sample_clock_records().iter().rev().find(|record| record.stream == rid("mock/tx")).unwrap().domain
}

fn binding(profile: &str) -> Binding {
    Binding {
        module: module_ref(),
        selector: BTreeMap::from([(Ident::parse("id").unwrap(), Value::Str("mock".to_owned()))]),
        profile: Some(ProfileRef { name: profile.to_owned(), version: Version::new(1, 1, 0) }),
        feed: None,
    }
}

fn key(name: &str) -> Key { Key::parse(name).unwrap() }
fn rid(name: &str) -> ResourceId { ResourceId::parse(name).unwrap() }

fn request(constraints: &[(&str, Constraint)]) -> Requested {
    Requested {
        resource: rid("mock"),
        constraints: constraints.iter().map(|(name, constraint)| (key(name), constraint.clone())).collect(),
    }
}

fn eq(value: Value) -> Constraint { Constraint::Eq { value } }

#[derive(Default)]
struct Queue {
    actions: Mutex<VecDeque<Action>>,
}

impl Queue {
    fn push(&self, action: Action) { self.actions.lock().unwrap().push_back(action); }
}

impl ActionReceiver for Queue {
    fn recv(&self) -> Option<Action> { self.actions.lock().unwrap().pop_front() }
}

struct RefusingSubmitter;
impl ActionSubmitter for RefusingSubmitter {
    fn submit(&self, _action: Action) -> Result<ActionId, Vec<Violation>> { Ok(ActionId(1)) }
}

struct MemLink {
    policy: BackPressure,
    capacity: usize,
    queue: Mutex<VecDeque<BlockRef>>,
    drops: AtomicU64,
    carry: Mutex<DropCarry>,
}

impl MemLink {
    fn new(policy: BackPressure, capacity: usize) -> Self {
        Self { policy, capacity, queue: Mutex::new(VecDeque::new()), drops: AtomicU64::new(0), carry: Mutex::new(DropCarry::default()) }
    }
    fn receive(&self) -> Option<BlockRef> { self.queue.lock().unwrap().pop_front() }
    fn queued(&self) -> usize { self.queue.lock().unwrap().len() }
}

impl DataLink for MemLink {
    fn publish(&self, block: BlockRef) -> PublishOutcome {
        let mut queue = self.queue.lock().unwrap();
        if queue.len() < self.capacity { queue.push_back(block); return PublishOutcome::Accepted; }
        match self.policy {
            BackPressure::Block => PublishOutcome::Full,
            BackPressure::DropOldest => {
                let old = queue.pop_front().unwrap();
                self.drops.fetch_add(1, Ordering::Relaxed);
                self.carry.lock().unwrap().absorb(&old);
                queue.push_back(block);
                PublishOutcome::DroppedOldest
            }
            BackPressure::DropNewest => {
                self.drops.fetch_add(1, Ordering::Relaxed);
                self.carry.lock().unwrap().absorb(&block);
                PublishOutcome::DroppedNewest
            }
        }
    }
    fn receive(&self) -> Option<BlockRef> { MemLink::receive(self) }
    fn drops(&self) -> u64 { self.drops.load(Ordering::Relaxed) }
    fn take_drop_carry(&self) -> DropCarry { std::mem::take(&mut *self.carry.lock().unwrap()) }
    fn policy(&self) -> BackPressure { self.policy }
}

thread_local! {
    /// The event ring each harness on this thread gets: MR-37's test needs a one-slot
    /// ring to tell the hot path, which drops, from the control path, which never does.
    static RING_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(4096) };
}

struct Harness {
    mock: MockRadio,
    clocks: Arc<ClockRegistry>,
    auth: Arc<ManualTimeAuthority>,
    events: Arc<EventCollector>,
    actions: Arc<Queue>,
    link: Option<Arc<MemLink>>,
    effective: BTreeMap<Key, Value>,
    prepare_coercions: Vec<ezsdr_kernel::spec::Coercion>,
}

impl Harness {
    fn new(profile: &str, constraints: &[(&str, Constraint)], selector_overrides: &[(&str, Value)], env: &[(&str, serde_json::Value)], link: Option<(BackPressure, usize)>) -> Self {
        match Self::new_with_class(profile, constraints, selector_overrides, env, link, ExecutionClass::Simulation) {
            Ok(harness) => harness,
            Err(error) => panic!("prepare MockRadio: {error}"),
        }
    }

    fn new_with_class(profile: &str, constraints: &[(&str, Constraint)], selector_overrides: &[(&str, Value)], env: &[(&str, serde_json::Value)], link: Option<(BackPressure, usize)>, class: ExecutionClass) -> Result<Self, ModuleError> {
        Self::new_with_options(profile, constraints, selector_overrides, env, link, class, Rational::new(1_000_000_000, 1).unwrap(), 0, "rx")
    }

    #[allow(clippy::too_many_arguments)]
    fn new_with_options(profile: &str, constraints: &[(&str, Constraint)], selector_overrides: &[(&str, Value)], env: &[(&str, serde_json::Value)], link: Option<(BackPressure, usize)>, class: ExecutionClass, root_rate: Rational, extra_links: usize, attached_port: &str) -> Result<Self, ModuleError> {
        let mut b = binding(profile);
        for (name, value) in selector_overrides {
            b.selector.insert(Ident::parse(name).unwrap(), value.clone());
        }
        let mut mock = MockRadio::from_binding(&b).unwrap();
        let clocks = Arc::new(ClockRegistry::new());
        clocks.register(ClockDomain::root(ROOT, root_rate, EpochRef::Arbitrary { set_by: "test".to_owned() })).unwrap();
        let auth = Arc::new(ManualTimeAuthority::new(clocks.clone(), ROOT, &[], Pacing::FreeRunning).unwrap());
        let actions = Arc::new(Queue::default());
        let mut registry = ezsdr_kernel::module_api::ModuleRegistry::new();
        let mut checks = AdmissionCheckRegistry::new();
        let mut kinds = EventKindRegistry::new();
        ezsdr_radio::register(&mut registry, &mut checks, &mut kinds).unwrap();
        ezsdr_sim::register(&mut registry, &mut checks, &mut kinds).unwrap();
        let event_kinds = kinds.kinds();
        let pairs: Vec<_> = [rid("mock"), rid("mock/rx"), rid("mock/tx")].into_iter()
            .flat_map(|source| event_kinds.iter().cloned().map(move |kind| (source.clone(), kind)))
            .collect();
        let events = Arc::new(EventCollector::new(&pairs, &event_kinds, RING_DEPTH.with(|depth| depth.get()), &Policy::default()));
        let link = link.map(|(policy, capacity)| Arc::new(MemLink::new(policy, capacity)));
        let mut attached: Vec<_> = link.as_ref().map(|link| AttachedPort {
            component: Ident::parse("radio").unwrap(),
            port: Ident::parse(attached_port).unwrap(),
            endpoint: Endpoint::StreamOut(link.clone()),
        }).into_iter().collect();
        if let Some((policy, capacity)) = link.as_ref().map(|link| (link.policy(), link.capacity)) {
            attached.extend((0..extra_links).map(|_| AttachedPort {
                component: Ident::parse("radio").unwrap(),
                port: Ident::parse("rx").unwrap(),
                endpoint: Endpoint::StreamOut(Arc::new(MemLink::new(policy, capacity))),
            }));
        }
        let environment = Arc::new(env.iter().map(|(name, value)| (Namespace::parse(name).unwrap(), value.clone())).collect());
        let ctx = PrepareContext {
            run: RunId::from_string("test-run".to_owned()),
            class,
            time: auth.clone(),
            clocks: clocks.clone(),
            events: events.clone(),
            actions: actions.clone(),
            actions_out: Arc::new(RefusingSubmitter),
            environment,
            inputs: Arc::new(BTreeMap::<ContentHash, Arc<[u8]>>::new()),
            links: attached,
            components: BTreeMap::new(),
            host_budget: RelativeBudget::new(Duration::new(ClockDomainId::HOST_MONOTONIC, 5_000_000_000)).unwrap(),
        };
        let fragment = Fragment {
            id: Ident::parse("radio").unwrap(),
            instance: module_ref(),
            role: Role::Provider,
            content: serde_json::json!({ "selector": {}, "requested": request(constraints) }),
            after: Vec::new(),
        };
        let prepared = mock.prepare(&fragment, ctx)?;
        Ok(Self { mock, clocks, auth, events, actions, link, effective: prepared.effective, prepare_coercions: prepared.coercions })
    }

    fn arm_start(&mut self, start: i64) -> Result<(), ezsdr_kernel::module_api::ModuleError> {
        self.mock.arm()?;
        self.mock.start(Some(TimePoint::new(ROOT, start)))
    }

    fn step(&mut self, tick: i64) -> Result<(), ezsdr_kernel::module_api::ModuleError> {
        self.auth.advance_to(TimePoint::new(ROOT, tick)).unwrap();
        self.mock.step(TimePoint::new(ROOT, tick)).map(|_| ())
    }
}

#[test]
fn mr_01_descriptor_registers() {
    let mut registry = ezsdr_kernel::module_api::ModuleRegistry::new();
    let mut checks = AdmissionCheckRegistry::new();
    let mut kinds = EventKindRegistry::new();
    ezsdr_radio::register(&mut registry, &mut checks, &mut kinds).unwrap();
    ezsdr_sim::register(&mut registry, &mut checks, &mut kinds).unwrap();
    let mut missing = ezsdr_kernel::module_api::ModuleRegistry::new();
    assert!(missing.register(descriptor(), Factories { provider: true, ..Factories::default() }).is_err());
    registry.register(descriptor(), Factories { provider: true, ..Factories::default() }).unwrap();
    let d = descriptor();
    assert_eq!(d.id.as_str(), "ezsdr.radio.mock");
    assert_eq!(d.version, Version::new(1, 3, 0));
    assert_eq!(d.kernel_api, KERNEL_API);
    assert_eq!(d.roles, [Role::Provider]);
    let requirements: Vec<_> = d.vocabularies.iter().map(|v| (v.id.as_str().to_owned(), v.req.0)).collect();
    assert_eq!(requirements, [("radio".to_owned(), Version::new(1, 3, 0)), ("sim".to_owned(), Version::new(1, 1, 0))]);
    assert_eq!(d.impl_hash, Some(ezsdr_kernel::hash::ContentHash::of_bytes(b"ezsdr.radio.mock 1.3.0")));
}

#[test]
fn mr_02_from_binding_refusals() {
    let mut b = binding("x310-like");
    b.module.id = ModuleId::parse("other.mock").unwrap();
    let err = MockRadio::from_binding(&b).err().unwrap();
    assert!(err.message.starts_with("MR-2: "));
    b.profile = None;
    assert!(MockRadio::from_binding(&b).err().unwrap().message.starts_with("MR-2: "));
    b = binding("not-a-profile");
    assert!(MockRadio::from_binding(&b).err().unwrap().message.starts_with("MR-2: "));
    b = binding("ideal");
    b.selector.insert(Ident::parse("id").unwrap(), Value::Str("a/b".to_owned()));
    assert!(MockRadio::from_binding(&b).err().unwrap().message.starts_with("MR-2: "));
    // MR-2: the id becomes a section-name segment (MR-27), so a legal resource path
    // segment that is not a legal one is refused rather than panicked on later
    for id in ["Dev", "dev-rx", "dev.rx", "9dev", ""] {
        b = binding("ideal");
        b.selector.insert(Ident::parse("id").unwrap(), Value::Str(id.to_owned()));
        let err = MockRadio::from_binding(&b).err().unwrap_or_else(|| panic!("{id:?} must be refused"));
        assert!(err.message.starts_with("MR-2: "), "{id:?}: {err}");
    }
    b = binding("ideal");
    b.selector.insert(Ident::parse("instances").unwrap(), Value::Int(0));
    assert!(MockRadio::from_binding(&b).err().unwrap().message.starts_with("MR-2: "));
    b = binding("ideal");
    b.selector.insert(Ident::parse("block_len_jitter").unwrap(), Value::Str("yes".to_owned()));
    assert!(MockRadio::from_binding(&b).err().unwrap().message.starts_with("MR-2: "));
    b = binding("ideal");
    b.selector.insert(Ident::parse("rx_test_pattern").unwrap(), Value::Str("noise".to_owned()));
    assert!(MockRadio::from_binding(&b).err().unwrap().message.starts_with("MR-2: "));
    b = binding("ideal");
    b.selector.insert(Ident::parse("unknown").unwrap(), Value::Bool(false));
    assert!(MockRadio::from_binding(&b).err().unwrap().message.starts_with("MR-2: "));
    b = binding("ideal");
    b.feed = Some(ezsdr_kernel::spec::SinkFeed {
        port: ezsdr_kernel::contract::PortRef { component: Ident::parse("radio").unwrap(), port: Ident::parse("rx").unwrap() },
        policy: BackPressure::DropOldest,
        capacity: 1,
    });
    assert!(MockRadio::from_binding(&b).err().unwrap().message.starts_with("MR-2: "));
    b = binding("ideal");
    b.selector.insert(Ident::parse("arm_after").unwrap(), Value::List(vec![Value::Int(4)]));
    assert!(MockRadio::from_binding(&b).err().unwrap().message.starts_with("MR-2: "));

    b = binding("x310-like");
    b.selector.clear();
    let defaults = MockRadio::from_binding(&b).unwrap();
    assert_eq!(defaults.instance().id, rid("mock"));
    assert_eq!(defaults.instance().arm_after, Vec::<ResourceId>::new());
    assert_eq!(defaults.instance().tree.capabilities[&key("radio.rx.channels")], ezsdr_kernel::spec::CapabilityValue::Range { min: Value::Int(0), max: Value::Int(2) });
}

#[test]
fn mr_02_the_tree_has_the_radio_model_shape() {
    let mut b = binding("x310-like");
    b.selector.insert(Ident::parse("instances").unwrap(), Value::Int(2));
    let radio = MockRadio::from_binding(&b).unwrap();
    let instance = radio.instance();
    assert_eq!(instance.id, rid("mock"));
    assert_eq!(instance.tree.kind.as_str(), "radio.device");
    assert_eq!(instance.tree.children.iter().map(|child| child.id.path.as_str()).collect::<Vec<_>>(), ["mock/rx", "mock/tx"]);
    assert_eq!(instance.tree.children.iter().map(|child| child.kind.as_str()).collect::<Vec<_>>(), ["radio.rx_stream", "radio.tx_stream"]);
    assert!(instance.tree.children.iter().all(|child| child.capabilities.is_empty() && child.ports.is_empty() && child.children.is_empty() && !child.shareable));
    assert_eq!(instance.tree.ports.len(), 1);
    assert_eq!(instance.tree.ports[0].name.as_str(), "rx");
    assert_eq!(instance.tree.ports[0].direction, ezsdr_kernel::contract::PortDirection::Out);
    assert_eq!(instance.tree.ports[0].contract.as_str(), "ezsdr.stream.cf32");
    assert!(!instance.tree.shareable);
    assert_eq!(instance.tree.capabilities[&key("radio.rx.channels")], ezsdr_kernel::spec::CapabilityValue::Range { min: Value::Int(0), max: Value::Int(4) });
    assert_eq!(instance.tree.capabilities[&key("radio.perf.rx_bytes_per_s")], ezsdr_kernel::spec::CapabilityValue::One { value: Value::Int(2_000_000_000) });
}

#[test]
fn mr_03_profile_values_reach_the_capabilities_and_the_envelope_section() {
    for (name, expected) in [("x310-like", "random_unless_timed_tune"), ("ideal", "deterministic")] {
        let radio = MockRadio::from_binding(&binding(name)).unwrap();
        let capabilities = &radio.instance().tree.capabilities;
        let one = |value| ezsdr_kernel::spec::CapabilityValue::One { value };
        let range = |min, max| ezsdr_kernel::spec::CapabilityValue::Range { min, max };
        let x310 = name == "x310-like";
        let max_channels = if x310 { 2 } else { 64 };
        let frequency = if x310 { (10_000_000.0, 6_000_000_000.0, 1.0) } else { (0.0, 1.0e12, 0.0) };
        let gain = if x310 { (0.0, 31.5, 0.5) } else { (-200.0, 200.0, 0.0) };
        for direction in ["rx", "tx"] {
            assert_eq!(capabilities[&key(&format!("radio.{direction}.channels"))], range(Value::Int(0), Value::Int(max_channels)));
            let rates = &capabilities[&key(&format!("radio.{direction}.sample_rate_hz"))];
            if x310 {
                let expected: Vec<_> = (1..=512).map(|n| Value::Num(200_000_000.0 / f64::from(n))).collect();
                assert_eq!(*rates, ezsdr_kernel::spec::CapabilityValue::AnyOf { values: expected });
            } else {
                assert_eq!(*rates, range(Value::Num(1.0), Value::Num(1_000_000_000.0)));
            }
            assert_eq!(capabilities[&key(&format!("radio.{direction}.frequency_hz"))], range(Value::Num(frequency.0), Value::Num(frequency.1)));
            assert_eq!(capabilities[&key(&format!("radio.{direction}.frequency_step_hz"))], one(Value::Num(frequency.2)));
            assert_eq!(capabilities[&key(&format!("radio.{direction}.gain_db"))], range(Value::Num(gain.0), Value::Num(gain.1)));
            assert_eq!(capabilities[&key(&format!("radio.{direction}.gain_step_db"))], one(Value::Num(gain.2)));
        }
        assert_eq!(capabilities[&key("radio.rx.antenna")], ezsdr_kernel::spec::CapabilityValue::AnyOf { values: ["RX2", "TX/RX"].into_iter().map(|value| Value::Str(value.to_owned())).collect() });
        assert_eq!(capabilities[&key("radio.tx.antenna")], ezsdr_kernel::spec::CapabilityValue::AnyOf { values: vec![Value::Str("TX/RX".to_owned())] });
        for name in ["radio.rx.coherent", "radio.full_duplex", "radio.hardware_time"] {
            assert_eq!(capabilities[&key(name)], one(Value::Bool(true)));
        }
        assert_eq!(capabilities[&key("radio.phase_behavior_on_retune")], one(Value::Str(expected.to_owned())));
        assert_eq!(capabilities[&key("radio.tx.repeat_max_samples")], one(Value::Int(if x310 { 268_435_456 } else { i64::from(u32::MAX) })));
        assert_eq!(capabilities[&key("radio.tx.repeat_align_samples")], one(Value::Int(if x310 { 2 } else { 1 })));
        assert_eq!(capabilities[&key("radio.rx.block_len")], one(Value::Int(2_000)));
        let envelope = &radio.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.envelope").unwrap()];
        let (lead, startup, tail, depth, gap, bytes_per_s, wire) = if x310 {
            (2_000_000, 2_000_000_000, 1_000_000, 16, 50_000_000, 1_000_000_000, 4)
        } else {
            (0, 0, 0, i64::from(u32::MAX), 0, 1_i64 << 62, 8)
        };
        assert_eq!(capabilities[&key("radio.timing.min_timed_command_lead_ns")], one(Value::Int(lead)));
        assert_eq!(capabilities[&key("radio.timing.startup_latency_ns")], one(Value::Int(startup)));
        assert_eq!(capabilities[&key("radio.timing.stop_tail_ns")], one(Value::Int(tail)));
        assert_eq!(capabilities[&key("radio.timing.command_queue_depth")], one(Value::Int(depth)));
        assert_eq!(capabilities[&key("radio.timing.overflow_restart_gap_ns")], one(Value::Int(gap)));
        assert_eq!(capabilities[&key("radio.perf.rx_bytes_per_s")], one(Value::Int(bytes_per_s)));
        assert_eq!(capabilities[&key("radio.perf.tx_bytes_per_s")], one(Value::Int(bytes_per_s)));
        assert_eq!(capabilities[&key("radio.perf.wire_bytes_per_sample")], one(Value::Int(wire)));
        // MR-27: the six sections are named after this instance's own id, so two Mocks in
        // one Run cannot overwrite each other's records. The set is exactly these six, and
        // `sections` is a `BTreeMap`, so they come out in `Namespace` order.
        let names: Vec<String> = radio.instance().sections.keys().map(|name| name.as_str().to_owned()).collect();
        assert_eq!(
            names,
            ["applied", "bursts", "envelope", "faults", "rejected", "stats"]
                .map(|suffix| format!("ezsdr.radio.mock.mock.{suffix}"))
        );
        assert_eq!(envelope["profile"]["name"], name);
        assert_eq!(envelope["profile"]["version"], serde_json::json!({ "major": 1, "minor": 1, "patch": 0 }));
        let (tx_delay, rx_delay) = if name == "x310-like" { (45, 0) } else { (0, 0) };
        assert_eq!(capabilities[&key("radio.tx.path_delay_samples")], one(Value::Int(tx_delay)));
        assert_eq!(capabilities[&key("radio.rx.path_delay_samples")], one(Value::Int(rx_delay)));
        assert_eq!(envelope["timing"]["min_timed_command_lead_ns"], lead);
        assert_eq!(envelope["timing"]["startup_latency_ns"], startup);
        assert_eq!(envelope["timing"]["stop_tail_ns"], tail);
        assert_eq!(envelope["timing"]["command_queue_depth"], depth);
        assert_eq!(envelope["timing"]["overflow_restart_gap_ns"], gap);
        assert_eq!(envelope["performance"]["rx_bytes_per_s"], bytes_per_s);
        assert_eq!(envelope["performance"]["tx_bytes_per_s"], bytes_per_s);
        assert_eq!(envelope["performance"]["wire_bytes_per_sample"], wire);
        assert_eq!(radio.instance().fidelity, if name == "x310-like" { ezsdr_kernel::module_api::Fidelity { timing: ezsdr_kernel::module_api::EnvelopeFidelity::Envelope, continuity: ezsdr_kernel::module_api::EnvelopeFidelity::Envelope, coercion: ezsdr_kernel::module_api::CoercionFidelity::Grid, rf: ezsdr_kernel::module_api::RfFidelity::None, transport: ezsdr_kernel::module_api::TransportFidelity::None } } else { ezsdr_kernel::module_api::Fidelity::NONE });
    }
}

#[test]
fn mr_04_instance() {
    let radio = MockRadio::from_binding(&binding("x310-like")).unwrap();
    assert!(radio.instance().driving.stepped);
    assert_eq!(radio.instance().min_command_lead.unwrap().ticks, 2_000_000);
    assert_eq!(radio.instance().arm_after, Vec::<ResourceId>::new());
    let ideal = MockRadio::from_binding(&binding("ideal")).unwrap();
    assert!(ideal.instance().min_command_lead.is_none());
}

#[test]
fn mr_06_coerce_cases() {
    let radio = MockRadio::from_binding(&binding("x310-like")).unwrap();
    let report = radio.coerce(&request(&[
        ("radio.rx.sample_rate_hz", Constraint::Min { value: Value::Num(1_100_000.0) }),
        ("radio.rx.gain_db", eq(Value::Num(1.1))),
        ("radio.rx.antenna", Constraint::Set { values: vec![Value::Str("TX/RX".to_owned()), Value::Str("RX2".to_owned())] }),
    ])).unwrap();
    assert_eq!(report.applied[&key("radio.rx.sample_rate_hz")], Value::Num(200_000_000.0 / 181.0));
    assert_eq!(report.applied[&key("radio.rx.gain_db")], Value::Num(1.0));
    assert_eq!(report.applied[&key("radio.rx.antenna")], Value::Str("TX/RX".to_owned()));
    assert_eq!(report.coercions.len(), 1);
    let rejected = radio.coerce(&request(&[
        ("radio.tx.channels", eq(Value::Int(2))),
        ("radio.tx.sample_rate_hz", eq(Value::Num(200_000_000.0))),
    ])).unwrap();
    assert_eq!(rejected.rejected.len(), 1);
    assert_eq!(rejected.rejected[0].key, key("radio.tx.sample_rate_hz"));
    assert!(rejected.rejected[0].reason.starts_with("RM-7:"));

    let out_of_range = radio.coerce(&request(&[("radio.rx.frequency_hz", eq(Value::Num(7.0e9)))] )).unwrap();
    assert!(!out_of_range.applied.contains_key(&key("radio.rx.frequency_hz")));
    assert_eq!(out_of_range.rejected.len(), 1);
    assert!(out_of_range.rejected[0].reason.starts_with("RM-8: 7000000000"));

    let tie = radio.coerce(&request(&[("radio.rx.gain_db", eq(Value::Num(20.25)))] )).unwrap();
    assert_eq!(tie.applied[&key("radio.rx.gain_db")], Value::Num(20.0));

    let ideal = MockRadio::from_binding(&binding("ideal")).unwrap();
    let integer_rate = ideal.coerce(&request(&[("radio.rx.sample_rate_hz", eq(Value::Num(19_500_000.0)))] )).unwrap();
    assert_eq!(integer_rate.applied[&key("radio.rx.sample_rate_hz")], Value::Num(19_500_000.0));
    let fractional_rate = ideal.coerce(&request(&[("radio.rx.sample_rate_hz", eq(Value::Num(1_000_000.0 / 3.0)))] )).unwrap();
    assert_eq!(fractional_rate.rejected.len(), 1);
    let bad_antenna = radio.coerce(&request(&[("radio.rx.antenna", eq(Value::Str("J1".to_owned())))] )).unwrap();
    assert_eq!(bad_antenna.rejected.len(), 1);

    let rx_over_budget = radio.coerce(&request(&[
        ("radio.rx.channels", eq(Value::Int(2))),
        ("radio.rx.sample_rate_hz", eq(Value::Num(200_000_000.0))),
    ])).unwrap();
    assert_eq!(rx_over_budget.rejected.len(), 1);
    assert_eq!(rx_over_budget.rejected[0].key, key("radio.rx.sample_rate_hz"));
}

#[test]
fn mr_06_coerce_is_pure() {
    let radio = MockRadio::from_binding(&binding("ideal")).unwrap();
    let req = request(&[("radio.rx.frequency_hz", eq(Value::Num(2_400_000_000.0)))]);
    let first = radio.coerce(&req).unwrap();
    let second = radio.coerce(&req).unwrap();
    assert_eq!(first, second);
    assert_eq!(radio.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.stats").unwrap()]["rx_blocks"], 0);
}

#[test]
fn mr_07_prepare_cases() {
    let harness = Harness::new("x310-like", &[("radio.tx.channels", eq(Value::Int(1)))], &[], &[], None);
    let handles = harness.clocks.declared_sample_clocks();
    assert_eq!(handles.len(), 2);
    assert_eq!(handles[0].root_ticks_per_tick, Rational::new(1000, 1).unwrap());
    assert_eq!(handles[1].root_ticks_per_tick, Rational::new(1000, 1).unwrap());

    let mut second = Harness::new("ideal", &[], &[], &[], None);
    let context = PrepareContext {
        run: RunId::from_string("second-prepare-test".to_owned()),
        class: ExecutionClass::Simulation,
        time: second.auth.clone(),
        clocks: second.clocks.clone(),
        events: second.events.clone(),
        actions: second.actions.clone(),
        actions_out: Arc::new(RefusingSubmitter),
        environment: Arc::new(BTreeMap::new()),
        inputs: Arc::new(BTreeMap::<ContentHash, Arc<[u8]>>::new()),
        links: Vec::new(),
        components: BTreeMap::new(),
        host_budget: RelativeBudget::new(Duration::new(ClockDomainId::HOST_MONOTONIC, 5_000_000_000)).unwrap(),
    };
    let fragment = Fragment {
        id: Ident::parse("radio").unwrap(),
        instance: module_ref(),
        role: Role::Provider,
        content: serde_json::json!({ "selector": {}, "requested": request(&[("radio.rx.channels", eq(Value::Int(0))), ("radio.tx.channels", eq(Value::Int(0)))]) }),
        after: Vec::new(),
    };
    let result = second.mock.prepare(&fragment, context);
    let Err(error) = result else { panic!("a second prepare was accepted") };
    assert_eq!(error.kind, ezsdr_kernel::module_api::ModuleErrorKind::Rejected);
    assert!(error.message.starts_with("MR-7: "));
    second.mock.cleanup();
    let context = PrepareContext {
        run: RunId::from_string("prepare-after-cleanup-test".to_owned()),
        class: ExecutionClass::Simulation,
        time: second.auth.clone(),
        clocks: second.clocks.clone(),
        events: second.events.clone(),
        actions: second.actions.clone(),
        actions_out: Arc::new(RefusingSubmitter),
        environment: Arc::new(BTreeMap::new()),
        inputs: Arc::new(BTreeMap::<ContentHash, Arc<[u8]>>::new()),
        links: Vec::new(),
        components: BTreeMap::new(),
        host_budget: RelativeBudget::new(Duration::new(ClockDomainId::HOST_MONOTONIC, 5_000_000_000)).unwrap(),
    };
    let Err(error) = second.mock.prepare(&fragment, context) else { panic!("cleanup allowed a second prepare") };
    assert!(error.message.starts_with("MR-7: prepare was already called"));

    let Err(error) = Harness::new_with_options(
        "x310-like",
        &[("radio.rx.channels", eq(Value::Int(2))), ("radio.rx.sample_rate_hz", eq(Value::Num(200_000_000.0)))],
        &[], &[], None, ExecutionClass::Simulation, Rational::new(1_000_000_000, 1).unwrap(), 0, "rx",
    ) else { panic!("a rejected Requested was prepared") };
    assert!(error.message.starts_with("MR-7: rejected requests:"));

    let Err(error) = Harness::new_with_options(
        "ideal", &[("radio.rx.channels", eq(Value::Int(1)))], &[], &[], None,
        ExecutionClass::Simulation, Rational::new(3, 2).unwrap(), 0, "rx",
    ) else { panic!("a non-integer root was accepted") };
    assert!(error.message.contains("primary root rate is not an integer"));

    let Err(error) = Harness::new_with_options(
        "ideal", &[("radio.rx.channels", eq(Value::Int(1)))], &[], &[], Some((BackPressure::DropOldest, 1)),
        ExecutionClass::Simulation, Rational::new(1_000_000_000, 1).unwrap(), 0, "tx",
    ) else { panic!("a non-rx endpoint was accepted") };
    assert!(error.message.contains("only this fragment's rx port"));

    let Err(error) = Harness::new_with_options(
        "ideal", &[("radio.rx.channels", eq(Value::Int(1)))], &[], &[], Some((BackPressure::Block, 1)),
        ExecutionClass::Simulation, Rational::new(1_000_000_000, 1).unwrap(), 1, "rx",
    ) else { panic!("multiple Block links were accepted") };
    assert!(error.message.contains("multiple rx links cannot use Block"));

    let result = Harness::new_with_class("ideal", &[], &[], &[], None, ExecutionClass::RealtimeEmulation);
    let Err(error) = result else { panic!("a non-Simulation execution class was accepted") };
    assert_eq!(error.kind, ezsdr_kernel::module_api::ModuleErrorKind::Unsupported);
}

#[test]
fn mr_07_prepare_reports_the_coercions_coerce_reported() {
    let h = Harness::new("x310-like", &[("radio.rx.sample_rate_hz", eq(Value::Num(19_500_000.0)))], &[], &[], None);
    let radio = MockRadio::from_binding(&binding("x310-like")).unwrap();
    let report = radio.coerce(&request(&[("radio.rx.sample_rate_hz", eq(Value::Num(19_500_000.0)))] )).unwrap();
    assert_eq!(report.applied[&key("radio.rx.sample_rate_hz")], Value::Num(20_000_000.0));
    assert_eq!(h.prepare_coercions, report.coercions);
}

#[test]
fn mr_08_effective_holds_exactly_the_ten_configuration_keys() {
    let harness = Harness::new("ideal", &[], &[], &[], None);
    let effective = &harness.effective;
    let expected_keys: BTreeSet<_> = ezsdr_radio::keys::CONFIGURATION.into_iter().collect();
    let actual_keys: BTreeSet<_> = effective.keys().map(Key::as_str).collect();
    assert_eq!(expected_keys.len(), 10);
    assert_eq!(actual_keys, expected_keys);
    assert_eq!(effective, &BTreeMap::from([
        (key(ezsdr_radio::keys::RX_CHANNELS), Value::Int(1)),
        (key(ezsdr_radio::keys::TX_CHANNELS), Value::Int(0)),
        (key(ezsdr_radio::keys::RX_SAMPLE_RATE_HZ), Value::Num(1_000_000.0)),
        (key(ezsdr_radio::keys::TX_SAMPLE_RATE_HZ), Value::Num(1_000_000.0)),
        (key(ezsdr_radio::keys::RX_FREQUENCY_HZ), Value::Num(1_000_000_000.0)),
        (key(ezsdr_radio::keys::TX_FREQUENCY_HZ), Value::Num(1_000_000_000.0)),
        (key(ezsdr_radio::keys::RX_GAIN_DB), Value::Num(0.0)),
        (key(ezsdr_radio::keys::TX_GAIN_DB), Value::Num(0.0)),
        (key(ezsdr_radio::keys::RX_ANTENNA), Value::Str("RX2".to_owned())),
        (key(ezsdr_radio::keys::TX_ANTENNA), Value::Str("TX/RX".to_owned())),
    ]));
}

#[test]
fn mr_11_start_cases() {
    let constraints = [("radio.tx.channels", eq(Value::Int(1)))];
    let mut harness = Harness::new("x310-like", &constraints, &[], &[], Some((BackPressure::DropOldest, 8)));
    harness.mock.arm().unwrap();
    assert!(harness.mock.start(Some(TimePoint::new(ROOT, 1_999_999_999))).unwrap_err().message.contains("ezsdr.time.start_lead_ns"));
    let early = harness.events.drain();
    assert_eq!(early.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::LATE_COMMAND).count(), 1);
    let late = early.iter().find(|event| event.kind.as_str() == ezsdr_radio::kinds::LATE_COMMAND).unwrap();
    assert_eq!(late.source, rid("mock"));
    assert_eq!(late.time, TimePoint::new(ROOT, 0));
    harness.mock.start(Some(TimePoint::new(ROOT, 2_000_000_000))).unwrap();
    let clocks = harness.clocks.sample_clock_records();
    assert_eq!(clocks.iter().find(|record| record.stream == rid("mock/rx")).unwrap().origin, TimePoint::new(ROOT, 2_000_000_000));
    assert_eq!(clocks.iter().find(|record| record.stream == rid("mock/tx")).unwrap().origin, TimePoint::new(ROOT, 0));
    harness.step(2_001_999_000).unwrap();
    while let Some(block) = harness.link.as_ref().unwrap().receive() {
        assert!(!block.header().flags.contains(BlockFlags::LATE));
    }

    let mut no_link = Harness::new("x310-like", &[("radio.tx.channels", eq(Value::Int(1)))], &[], &[], None);
    no_link.mock.arm().unwrap();
    assert_eq!(no_link.clocks.sample_clock_records().iter().map(|record| record.stream.path.as_str()).collect::<Vec<_>>(), ["mock/tx"]);
    no_link.mock.start(Some(TimePoint::new(ROOT, 2_000_000_000))).unwrap();
    assert_eq!(no_link.clocks.sample_clock_records().iter().map(|record| record.stream.path.as_str()).collect::<Vec<_>>(), ["mock/tx"]);
}

#[test]
fn mr_12_block_lengths() {
    let mut harness = Harness::new("ideal", &[], &[("block_len_jitter", Value::Bool(true))], &[], Some((BackPressure::DropOldest, 64)));
    harness.arm_start(0).unwrap();
    harness.step(25_000_000).unwrap();
    let mut sizes = Vec::new();
    let mut starts = Vec::new();
    while let Some(block) = harness.link.as_ref().unwrap().receive() {
        sizes.push(block.header().len);
        starts.push(block.header().first_sample_time.ticks);
    }
    assert!(!sizes.is_empty());
    assert!(sizes.iter().all(|size| (1..=4000).contains(size)));
    let mut sample = 0;
    for (start, len) in starts.iter().zip(&sizes) {
        assert_eq!(*start, sample);
        sample += i64::from(*len);
    }

    let mut plain = Harness::new("ideal", &[], &[], &[], Some((BackPressure::DropOldest, 64)));
    plain.arm_start(0).unwrap();
    plain.step(25_000_000).unwrap();
    let plain_sizes: Vec<_> = std::iter::from_fn(|| plain.link.as_ref().unwrap().receive()).map(|block| block.header().len).collect();
    assert!(!plain_sizes.is_empty());
    assert!(plain_sizes.iter().all(|size| *size == 2000));

    let mut other_seed = Harness::new("ideal", &[], &[("block_len_jitter", Value::Bool(true))], &[("sim.seed", serde_json::json!(8))], Some((BackPressure::DropOldest, 64)));
    other_seed.arm_start(0).unwrap();
    other_seed.step(25_000_000).unwrap();
    let other_sizes: Vec<_> = std::iter::from_fn(|| other_seed.link.as_ref().unwrap().receive()).map(|block| block.header().len).collect();
    assert_ne!(sizes, other_sizes);
}

#[test]
fn mr_12_jitter_draws_belong_to_blocks_after_a_receive_cut() {
    let env = [
        ("sim.seed", serde_json::json!(7)),
        ("sim.faults", serde_json::json!([{ "at_ns": 3_500_000, "fault": "rx_overflow", "target": "radio" }]))
    ];
    let selector = [("block_len_jitter", Value::Bool(true))];
    let mut harness = Harness::new("x310-like", &[], &selector, &env, Some((BackPressure::DropOldest, 8)));
    harness.arm_start(2_000_000_000).unwrap();
    harness.step(2_003_500_000).unwrap();
    harness.step(2_056_000_000).unwrap();
    let blocks: Vec<_> = std::iter::from_fn(|| harness.link.as_ref().unwrap().receive()).collect();
    let mut rng = ezsdr_sim::SimRng::new(7, "mock/rx");
    let first_len = 1 + rng.below(4_000) as i64;
    let second_len = 1 + rng.below(4_000) as i64;
    let third_len = 1 + rng.below(4_000) as i64;
    assert!(first_len < 3_500 && first_len + second_len > 3_500);
    assert_eq!(blocks.len(), 3);
    assert_eq!(i64::from(blocks[0].header().len), first_len);
    assert_eq!(i64::from(blocks[1].header().len), 3_500 - first_len);
    assert_eq!(blocks[2].header().first_sample_time.ticks, 53_500);
    assert_eq!(i64::from(blocks[2].header().len), third_len);
}

#[test]
fn mr_13_ramp_values() {
    let mut harness = Harness::new("ideal", &[("radio.rx.channels", eq(Value::Int(2)))], &[("rx_test_pattern", Value::Str("ramp".to_owned()))], &[], Some((BackPressure::DropOldest, 4)));
    harness.arm_start(0).unwrap();
    harness.step(1_999_001).unwrap();
    let block = harness.link.as_ref().unwrap().receive().unwrap();
    assert_eq!(block.buffer().memory_domain, ezsdr_hostmem::HOST_MEMORY);
    let bytes = block.host_bytes().unwrap();
    let f32_at = |offset: usize| f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    assert_eq!(f32_at(0), 0.0);
    assert_eq!(f32_at(4), 0.0);
    assert_eq!(f32_at(8), 1.0 / 65536.0);
    assert_eq!(f32_at(12), 0.0);
    assert_eq!(f32_at(16), 2.0 / 65536.0);
    assert_eq!(f32_at(20), 0.0);
    assert_eq!(f32_at(16_000), 0.0);
    assert_eq!(f32_at(16_004), 1.0 / 64.0);
    assert_eq!(f32_at(16_008), 1.0 / 65536.0);
    assert_eq!(f32_at(16_012), 1.0 / 64.0);

    let mut zero = Harness::new("ideal", &[("radio.rx.channels", eq(Value::Int(2)))], &[], &[], Some((BackPressure::DropOldest, 4)));
    zero.arm_start(0).unwrap();
    zero.step(1_999_001).unwrap();
    let block = zero.link.as_ref().unwrap().receive().unwrap();
    assert_eq!(block.buffer().memory_domain, ezsdr_hostmem::HOST_MEMORY);
    assert!(block.host_bytes().unwrap().iter().all(|byte| *byte == 0));
}

#[test]
fn mr_14_blocks_appear_when_their_last_sample_has_occurred() {
    let pending_faults = [("sim.faults", serde_json::json!([
        { "at_ns": 5_000_000, "fault": "rx_overflow", "target": "radio" }
    ]))];
    let mut before_start = Harness::new("ideal", &[], &[], &pending_faults, None);
    before_start.step(0).unwrap();
    assert_eq!(before_start.auth.next_due(), None);

    let mut harness = Harness::new("ideal", &[], &[], &[], Some((BackPressure::DropOldest, 4)));
    harness.arm_start(0).unwrap();
    assert_eq!(harness.auth.next_due(), Some(TimePoint::new(ROOT, 1_999_001)));
    harness.step(1_999_000).unwrap();
    assert_eq!(harness.link.as_ref().unwrap().queued(), 0);
    assert_eq!(harness.auth.advance_to(TimePoint::new(ROOT, 1_999_001)).unwrap(), 1);
    harness.mock.step(TimePoint::new(ROOT, 1_999_001)).unwrap();
    assert_eq!(harness.link.as_ref().unwrap().receive().unwrap().header().len, 2000);
    assert_eq!(harness.auth.next_due(), Some(TimePoint::new(ROOT, 3_999_001)));
}

#[test]
fn mr_16_burst_refusals() {
    let mut harness = tx_harness("x310-like");
    harness.arm_start(2_000_000_000).unwrap();
    let domain = tx_domain(&harness);
    let mut wrong_target = tx_action(domain, 2_010_000, 2, false, LatePolicy::SendAsapAndFlag);
    if let Action::TxBurst { target, .. } = &mut wrong_target { *target = rid("mock/rx"); }
    harness.actions.push(wrong_target);
    harness.actions.push(tx_action(ROOT, 2_010_000, 2, false, LatePolicy::SendAsapAndFlag));
    let mut bad_size = tx_action(domain, 2_010_000, 2, false, LatePolicy::SendAsapAndFlag);
    if let Action::TxBurst { waveform, .. } = &mut bad_size { waveform.size_bytes = 7; }
    harness.actions.push(bad_size);
    harness.actions.push(tx_action(domain, 2_010_000, 1, true, LatePolicy::SendAsapAndFlag));
    let mut metadata = tx_action(domain, 2_010_000, 2, false, LatePolicy::SendAsapAndFlag);
    if let Action::TxBurst { metadata, .. } = &mut metadata { metadata.insert(key("test.tag"), Value::Bool(true)); }
    harness.actions.push(metadata);
    let mut too_long = tx_action(domain, 2_010_000, 1, false, LatePolicy::SendAsapAndFlag);
    if let Action::TxBurst { waveform, repeat, .. } = &mut too_long {
        waveform.size_bytes = 268_435_458 * 8;
        *repeat = true;
    }
    harness.actions.push(too_long);
    harness.actions.push(tx_action(domain, 2_010_000, 2, false, LatePolicy::SendAsapAndFlag));
    harness.actions.push(tx_action(domain, 2_010_000, 2, false, LatePolicy::SendAsapAndFlag));
    harness.step(0).unwrap();
    let rows = &harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.rejected").unwrap()];
    assert_eq!(rows.as_array().unwrap().len(), 7);
    let reasons: Vec<_> = rows.as_array().unwrap().iter().map(|row| row["reason"].as_str().unwrap()).collect();
    assert!(reasons.iter().all(|reason| reason.starts_with("MR-16:")));
    assert!(reasons[3].contains("repeat"));
    assert!(reasons[5].contains("repeat"));
    assert!(reasons[4].contains("metadata"));

    let mut no_tx = Harness::new("ideal", &[], &[], &[], None);
    no_tx.arm_start(0).unwrap();
    no_tx.actions.push(tx_action(ROOT, 0, 2, false, LatePolicy::SendAsapAndFlag));
    no_tx.step(0).unwrap();
    assert!(no_tx.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.rejected").unwrap()][0]["reason"].as_str().unwrap().starts_with("MR-16:"));

    let mut before_sync = tx_harness("x310-like");
    before_sync.arm_start(2_000_000_000).unwrap();
    let domain = tx_domain(&before_sync);
    before_sync.actions.push(tx_action(domain, 0, 2, false, LatePolicy::SendAsapAndFlag));
    before_sync.step(0).unwrap();
    assert!(before_sync.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.rejected").unwrap()][0]["reason"].as_str().unwrap().starts_with("MR-16:"));

    let mut overlap = tx_harness("ideal");
    overlap.arm_start(0).unwrap();
    let domain = tx_domain(&overlap);
    overlap.actions.push(tx_action(domain, 0, 4_000, false, LatePolicy::SendAsapAndFlag));
    overlap.step(0).unwrap();
    overlap.step(1_999_001).unwrap();
    overlap.actions.push(tx_action(domain, 1_999, 2, false, LatePolicy::SendAsapAndFlag));
    overlap.step(1_999_001).unwrap();
    let rejected = &overlap.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.rejected").unwrap()];
    assert_eq!(rejected.as_array().unwrap().len(), 1);
    assert!(rejected[0]["reason"].as_str().unwrap().contains("open burst's next sample"));

    let mut moved_duplicate = tx_harness("x310-like");
    moved_duplicate.arm_start(2_000_000_000).unwrap();
    let domain = tx_domain(&moved_duplicate);
    moved_duplicate.actions.push(tx_action(domain, 2_003_000, 2, false, LatePolicy::SendAsapAndFlag));
    moved_duplicate.actions.push(tx_action(domain, 2_000_000, 2, false, LatePolicy::SendAsapAndFlag));
    moved_duplicate.step(2_001_000_000).unwrap();
    let rejected = &moved_duplicate.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.rejected").unwrap()];
    assert_eq!(rejected.as_array().unwrap().len(), 1);
    assert!(rejected[0]["reason"].as_str().unwrap().contains("held burst already has this start"));
    let events = moved_duplicate.events.drain();
    assert_eq!(events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).count(), 1);
    let refused = events.iter().find(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).unwrap();
    assert_eq!(refused.payload["outcome"], "refused");
    assert_eq!(refused.source, rid("mock/tx"));
    assert_eq!(refused.time.domain, domain);
    assert_eq!(events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).count(), 1);
}

#[test]
fn mr_16_repeat_is_contiguous_across_wraps() {
    let mut harness = tx_harness("ideal");
    harness.arm_start(0).unwrap();
    let domain = tx_domain(&harness);
    harness.actions.push(tx_action(domain, 0, 1_000, true, LatePolicy::SendAsapAndFlag));
    harness.step(0).unwrap();
    harness.step(4_999_001).unwrap();
    harness.mock.stop(StopMode::Orderly).unwrap();
    let record: ezsdr_kernel::stream::BurstRecord = serde_json::from_value(
        harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.bursts").unwrap()][0].clone(),
    ).unwrap();
    assert_eq!((record.blocks, record.samples, record.wraps, record.end), (5, 5_000, 5, BurstEnd::Stop));

    let mut tracker = BurstTracker::new(ROOT);
    let mut at = 0;
    for (index, len) in [300, 300, 300, 100, 300].into_iter().enumerate() {
        let header = BlockHeader {
            first_sample_time: TimePoint::new(ROOT, at), len, channels: 1, direction: Direction::Tx,
            valid: ezsdr_kernel::stream::ChannelMask::full(1),
            flags: if index == 0 { BlockFlags::START_OF_BURST } else { BlockFlags::NONE },
            lost: None, contract: ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.cf32").unwrap(),
        };
        let open = (index == 0).then_some(BurstOpen { waveform_len: Some(1_000), ..BurstOpen::default() });
        let result = tracker.on_block(&header, open).unwrap();
        assert_eq!(result, if index == 0 { BurstStep::Started } else { BurstStep::Continued });
        at += i64::from(len);
    }
}

#[test]
fn mr_17_late_policy_outcomes() {
    for (policy, outcome, transmitted) in [
        (LatePolicy::SendAsapAndFlag, "send_asap", true),
        (LatePolicy::DropAndFlag, "drop", false),
        (LatePolicy::RejectAtPlan, "plan_violation", false),
    ] {
        let mut harness = tx_harness("x310-like");
        harness.arm_start(2_000_000_000).unwrap();
        let domain = tx_domain(&harness);
        harness.step(2_010_000_000).unwrap();
        harness.actions.push(tx_action(domain, 2_011_000, 1_000, false, policy));
        harness.step(2_010_000_000).unwrap();
        let events = harness.events.drain();
        let late = events.iter().find(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).unwrap();
        assert_eq!(late.payload["outcome"], outcome);
        assert_eq!(late.payload["late_by_ns"], 1_000_000);
        assert_eq!(late.source, rid("mock/tx"));
        assert_eq!(late.time, TimePoint::new(domain, 2_010_000));
        assert_eq!(events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).count(), 1);
        assert_eq!(events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).count(), 0);
        if transmitted {
            harness.step(2_012_999_001).unwrap();
            let record: ezsdr_kernel::stream::BurstRecord = serde_json::from_value(
                harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.bursts").unwrap()][0].clone(),
            ).unwrap();
            assert_eq!(record.late_by.unwrap().ticks, 1_000_000);
            assert_eq!(record.samples, 1_000);
        } else {
            assert!(harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.bursts").unwrap()].as_array().unwrap().is_empty());
            let rejected = &harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.rejected").unwrap()];
            assert_eq!(rejected.as_array().unwrap().len(), 1);
            assert!(rejected[0]["reason"].as_str().unwrap().starts_with("MR-17:"));
        }
    }
}

#[test]
fn mr_18_hardware_timed_updates() {
    let mut harness = Harness::new("x310-like", &[], &[], &[], None);
    harness.arm_start(2_000_000_000).unwrap();
    harness.step(2_001_000_000).unwrap();
    let now = 2_001_000_000;
    harness.actions.push(update_action("radio.rx.frequency_hz", Value::Num(2.4e9), UpdateClass::HardwareTimed, Some(TimePoint::new(ROOT, now + 5_000_000))));
    harness.actions.push(update_action("radio.rx.frequency_hz", Value::Num(2.5e9), UpdateClass::HardwareTimed, Some(TimePoint::new(ROOT, now + 1_000_000))));
    harness.actions.push(update_action("radio.tx.frequency_hz", Value::Num(2.6e9), UpdateClass::HardwareTimed, Some(TimePoint::new(ROOT, now + 5_000_000))));
    harness.step(now).unwrap();
    harness.step(now + 5_000_000).unwrap();
    let applied = &harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.applied").unwrap()];
    let rows = applied.as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["at"]["ticks"], now + 2_000_000);
    assert_eq!(rows[1]["at"]["ticks"], now + 5_000_000);
    assert_eq!(rows[2]["at"]["ticks"], now + 5_000_000);
    assert_eq!(rows[1]["value"], 2.4e9);
    assert_eq!(rows[2]["key"], "radio.tx.frequency_hz");
    assert_eq!(harness.events.drain().iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::LATE_COMMAND).count(), 1);

    let mut full = Harness::new("x310-like", &[], &[], &[], None);
    full.mock.arm().unwrap();
    full.step(0).unwrap();
    for i in 0..16 {
        full.actions.push(update_action("radio.rx.frequency_hz", Value::Num(2.0e9 + i as f64), UpdateClass::HardwareTimed, Some(TimePoint::new(ROOT, 10_000_000_000 + i))));
    }
    full.actions.push(update_action("radio.rx.frequency_hz", Value::Num(2.1e9), UpdateClass::HardwareTimed, Some(TimePoint::new(ROOT, 0))));
    full.step(0).unwrap();
    let events = full.events.drain();
    assert_eq!(events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_QUEUE_FULL).count(), 1);
    assert_eq!(events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::LATE_COMMAND).count(), 0);
    let full_event = events.iter().find(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_QUEUE_FULL).unwrap();
    assert_eq!(full_event.source, rid("mock"));
    assert_eq!(full_event.time, TimePoint::new(ROOT, 0));
    let rejected = &full.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.rejected").unwrap()];
    assert_eq!(rejected.as_array().unwrap().len(), 1);
    assert!(rejected[0]["reason"].as_str().unwrap().contains("queue is full"));
}

#[test]
fn mr_25_a_stream_stop_keeps_the_pending_commands() {
    // RM-16, MR-25: a `Stop` for `<device>/tx` or `<device>/rx` cancels no pending timed command
    // (one may be for the other direction); `Stop` for the device cancels them all
    let now = 2_001_000_000;
    for (target, applied) in [("mock/tx", 2), ("mock/rx", 2), ("mock", 0)] {
        let mut harness = Harness::new("x310-like", &[], &[], &[], None);
        harness.arm_start(2_000_000_000).unwrap();
        harness.step(now).unwrap();
        harness.actions.push(update_action("radio.rx.frequency_hz", Value::Num(2.4e9), UpdateClass::HardwareTimed, Some(TimePoint::new(ROOT, now + 5_000_000))));
        harness.actions.push(update_action("radio.tx.frequency_hz", Value::Num(2.6e9), UpdateClass::HardwareTimed, Some(TimePoint::new(ROOT, now + 5_000_000))));
        harness.actions.push(Action::Stop { target: Some(rid(target)) });
        harness.step(now).unwrap();
        harness.step(now + 5_000_000).unwrap();
        let rows = &harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.applied").unwrap()];
        assert_eq!(rows.as_array().unwrap().len(), applied, "Stop {target}");
    }
}

#[test]
fn mr_18_a_cold_rate_change_starts_a_new_sample_clock() {
    let mut harness = Harness::new("x310-like", &[], &[], &[], Some((BackPressure::DropOldest, 8)));
    harness.arm_start(2_000_000_000).unwrap();
    harness.step(2_000_000_000).unwrap();
    let old = harness.clocks.sample_clock_records()[0].domain;
    let effective = 2_004_000_000;
    harness.actions.push(update_action("radio.rx.sample_rate_hz", Value::Num(2_000_000.0), UpdateClass::Cold, Some(TimePoint::new(ROOT, effective))));
    harness.step(effective).unwrap();
    harness.link.as_ref().unwrap().queue.lock().unwrap().clear();
    harness.step(effective + 999_501).unwrap();
    let records = harness.clocks.sample_clock_records();
    assert_eq!(records[0].ended_at, Some(TimePoint::new(ROOT, effective)));
    let current = records.last().unwrap();
    assert_eq!(current.origin, TimePoint::new(ROOT, effective));
    let first = harness.link.as_ref().unwrap().receive().unwrap();
    assert_ne!(first.header().first_sample_time.domain, old);
    assert_eq!(first.header().first_sample_time.ticks, 0);
    assert!(!first.header().flags.contains(BlockFlags::GAP_BEFORE));
}

#[test]
fn mr_09_the_transmit_clock_starts_on_its_lattice() {
    // RM-25: 1 Msps on a 1 GHz root is a ratio of 1 000; armed at 7, the origin is 1 000.
    let mut harness = tx_harness("ideal");
    harness.auth.advance_to(TimePoint::new(ROOT, 7)).unwrap();
    harness.mock.arm().unwrap();
    let record = harness.clocks.sample_clock_records().into_iter().find(|r| r.stream == rid("mock/tx")).unwrap();
    assert_eq!(record.origin, TimePoint::new(ROOT, 1_000));
}

#[test]
fn mr_18_a_cold_change_starts_its_clock_on_the_lattice() {
    // RM-25 with MockRadio's restart lead of zero: e₁ on the old lattice, e₂ on the new.
    for (from, to, e1, e2) in [(1_000_000.0, 20_000_000.0, 1_001_000, 1_001_000), (20_000_000.0, 1_000_000.0, 1_000_050, 1_001_000)] {
        let mut harness = Harness::new("ideal", &[("radio.rx.sample_rate_hz", eq(Value::Num(from)))], &[], &[], Some((BackPressure::DropOldest, 64)));
        harness.arm_start(0).unwrap();
        let old = harness.clocks.sample_clock_records()[0].domain;
        harness.actions.push(update_action("radio.rx.sample_rate_hz", Value::Num(to), UpdateClass::Cold, Some(TimePoint::new(ROOT, 1_000_037))));
        harness.step(0).unwrap();
        harness.step(1_000_037).unwrap();
        harness.step(3_200_000).unwrap();
        let records = harness.clocks.sample_clock_records();
        assert_eq!(records[0].ended_at, Some(TimePoint::new(ROOT, e1)), "{from} -> {to}");
        let new = records.last().unwrap();
        assert_eq!(new.origin, TimePoint::new(ROOT, e2), "{from} -> {to}");
        let old_ratio = records[0].root_ticks_per_tick.num() as i64;
        let mut first_new = None;
        while let Some(block) = harness.link.as_ref().unwrap().receive() {
            let header = block.header().clone();
            if header.first_sample_time.domain == old {
                assert!((header.first_sample_time.ticks + i64::from(header.len)) * old_ratio <= e1, "an old block past e₁");
            } else if first_new.is_none() {
                first_new = Some(header);
            }
        }
        let first_new = first_new.expect("the new clock delivered");
        assert_eq!(first_new.first_sample_time.domain, new.domain);
        assert_eq!(first_new.first_sample_time.ticks, 0);
        assert!(!first_new.flags.contains(BlockFlags::GAP_BEFORE));
    }
}

#[test]
fn mr_18_a_cold_receive_change_before_t0_applies_at_t0() {
    let mut harness = Harness::new("x310-like", &[], &[], &[], Some((BackPressure::DropOldest, 8)));
    let t0 = 2_000_000_000;
    harness.arm_start(t0).unwrap();
    harness.actions.push(update_action("radio.rx.sample_rate_hz", Value::Num(2_000_000.0), UpdateClass::Cold, None));
    harness.step(0).unwrap();
    harness.step(t0 + 999_501).unwrap();
    let records = harness.clocks.sample_clock_records();
    assert!(records.iter().all(|record| record.origin.ticks >= t0), "a receive clock starts before T0: {records:?}");
    let applied = &harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.applied").unwrap()];
    assert_eq!(applied[0]["at"]["ticks"], t0);
    let first = harness.link.as_ref().unwrap().receive().unwrap();
    assert_eq!(first.header().first_sample_time.domain, records.last().unwrap().domain);
    assert_eq!(first.header().first_sample_time.ticks, 0);
}

#[test]
fn mr_18_a_cold_transmit_change_replaces_the_tracker() {
    let mut harness = tx_harness("ideal");
    harness.arm_start(0).unwrap();
    let old = tx_domain(&harness);
    harness.actions.push(tx_action(old, 0, 1_000, true, LatePolicy::SendAsapAndFlag));
    harness.step(0).unwrap();
    harness.step(1_999_000).unwrap();
    harness.actions.push(update_action("radio.tx.sample_rate_hz", Value::Num(2_000_000.0), UpdateClass::Cold, None));
    harness.step(2_000_000).unwrap();
    let records = harness.clocks.sample_clock_records();
    assert_eq!(records[0].ended_at, Some(TimePoint::new(ROOT, 2_000_000)));
    let new = tx_domain(&harness);
    assert_ne!(new, old);
    assert_eq!(records.last().unwrap().origin, TimePoint::new(ROOT, 2_000_000));
    harness.actions.push(tx_action(new, 0, 3_000, false, LatePolicy::SendAsapAndFlag));
    harness.step(2_000_000).unwrap();
    harness.step(3_499_501).unwrap();
    let bursts = &harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.bursts").unwrap()];
    let records: Vec<ezsdr_kernel::stream::BurstRecord> = serde_json::from_value(bursts.clone()).unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].end, BurstEnd::Stop);
    assert_eq!(records[1].samples, 3_000);
    assert_eq!(harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.rejected").unwrap()].as_array().unwrap().len(), 0);

}

#[test]
fn mr_18_a_scheduled_pair_is_checked_when_it_applies() {
    let constraints = [
        ("radio.rx.channels", eq(Value::Int(2))),
        ("radio.rx.sample_rate_hz", eq(Value::Num(100_000_000.0))),
    ];
    let mut harness = Harness::new("x310-like", &constraints, &[], &[], Some((BackPressure::DropOldest, 1)));
    harness.arm_start(2_000_000_000).unwrap();
    let now = 2_001_000_000;
    harness.step(now).unwrap();
    harness.actions.push(update_action("radio.rx.channels", Value::Int(1), UpdateClass::Cold, Some(TimePoint::new(ROOT, now + 10_000))));
    harness.actions.push(update_action("radio.rx.sample_rate_hz", Value::Num(200_000_000.0), UpdateClass::Cold, Some(TimePoint::new(ROOT, now + 20_000))));
    harness.step(now).unwrap();
    harness.step(now + 20_000).unwrap();
    let rows = harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.applied").unwrap()].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["key"], "radio.rx.channels");
    assert_eq!(rows[1]["key"], "radio.rx.sample_rate_hz");
    assert_eq!(rows[1]["value"], 200_000_000.0);
    assert_eq!(harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.rejected").unwrap()].as_array().unwrap().len(), 0);

    let constraints = [
        ("radio.rx.channels", eq(Value::Int(1))),
        ("radio.rx.sample_rate_hz", eq(Value::Num(100_000_000.0))),
    ];
    let mut over = Harness::new("x310-like", &constraints, &[], &[], Some((BackPressure::DropOldest, 1)));
    over.arm_start(2_000_000_000).unwrap();
    let now = 2_001_000_000;
    over.step(now).unwrap();
    over.actions.push(update_action("radio.rx.channels", Value::Int(2), UpdateClass::Cold, Some(TimePoint::new(ROOT, now + 10_000))));
    over.actions.push(update_action("radio.rx.sample_rate_hz", Value::Num(200_000_000.0), UpdateClass::Cold, Some(TimePoint::new(ROOT, now + 20_000))));
    over.step(now).unwrap();
    over.step(now + 20_000).unwrap();
    let applied = over.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.applied").unwrap()].as_array().unwrap();
    assert_eq!(applied.len(), 1);
    assert_eq!(applied[0]["key"], "radio.rx.channels");
    let rejected = over.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.rejected").unwrap()].as_array().unwrap();
    assert_eq!(rejected.len(), 1);
    assert!(rejected[0]["reason"].as_str().unwrap().starts_with("MR-18:"));
    assert_eq!(over.events.drain().iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).count(), 1);
}

#[test]
fn mr_19_backpressure_is_an_overrun() {
    let mut harness = Harness::new("x310-like", &[], &[], &[], Some((BackPressure::Block, 1)));
    harness.arm_start(2_000_000_000).unwrap();
    harness.step(2_001_999_001).unwrap();
    harness.step(2_003_999_001).unwrap();
    let first = harness.link.as_ref().unwrap().receive().unwrap();
    assert_eq!(first.header().first_sample_time.ticks, 0);
    harness.step(2_053_999_001).unwrap();
    let restarted = harness.link.as_ref().unwrap().receive().unwrap();
    assert_eq!(restarted.header().first_sample_time.ticks, 52_000);
    assert!(restarted.header().flags.contains(BlockFlags::GAP_BEFORE));
    assert!(restarted.header().flags.contains(ezsdr_kernel::stream::BlockFlags::RESTARTED));
    assert_eq!(restarted.header().lost, Some(50_000));
    assert_eq!(harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.stats").unwrap()]["rx_blocks"], 2);
    let events = harness.events.drain();
    let overflows: Vec<_> = events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW).collect();
    assert_eq!(overflows.len(), 1);
    // MR-37: the back-pressure overrun travels the hot path too, as RM-24's bytes.
    assert!(overflows[0].payload.is_array(), "{}", overflows[0].payload);
    assert_eq!(
        RxOverflowPayload::from_payload(&overflows[0].payload).unwrap(),
        RxOverflowPayload { cause: RxOverflowCause::Overrun, lost: 50_000, restart_gap_ns: 50_000_000 }
    );

    let mut fractional = Harness::new_with_options(
        "x310-like",
        &[],
        &[],
        &[],
        Some((BackPressure::Block, 1)),
        ExecutionClass::Simulation,
        Rational::new(1_000_000_001, 1).unwrap(),
        0,
        "rx",
    ).unwrap();
    let start = 2_000_000_002;
    fractional.arm_start(start).unwrap();
    fractional.step(start + 4_000_000).unwrap();
    assert_eq!(fractional.link.as_ref().unwrap().receive().unwrap().header().first_sample_time.ticks, 0);
    fractional.step(start + 54_001_000).unwrap();
    let restarted = fractional.link.as_ref().unwrap().receive().unwrap();
    assert_eq!(restarted.header().first_sample_time.ticks, 52_001);
    assert_eq!(restarted.header().lost, Some(50_001));
}

#[test]
fn mr_09_same_tick_work_keeps_insertion_order() {
    // The fault was inserted at prepare, before the two cold updates and the burst.
    // Turning RX off first would record the fault as unapplied; reversing the
    // updates would leave RX off. TX still plays the held burst at the same tick.
    let env = [(
        "sim.faults",
        serde_json::json!([
            { "at_ns": 5_000_000, "fault": "rx_sequence_error", "target": "radio" }
        ]),
    )];
    let mut harness = Harness::new(
        "x310-like",
        &[("radio.tx.channels", eq(Value::Int(1)))],
        &[],
        &env,
        Some((BackPressure::DropOldest, 16)),
    );
    harness.arm_start(2_000_000_000).unwrap();
    let at = TimePoint::new(ROOT, 2_005_000_000);
    for channels in [0, 1] {
        harness.actions.push(update_action(
            "radio.rx.channels",
            Value::Int(channels),
            UpdateClass::Cold,
            Some(at),
        ));
    }
    harness.actions.push(tx_action(
        tx_domain(&harness),
        2_005_000,
        2,
        false,
        LatePolicy::SendAsapAndFlag,
    ));
    harness.step(2_000_000_000).unwrap();
    harness.step(at.ticks + 2_000_000).unwrap();
    let instance = harness.mock.instance();
    let faults = &instance.sections[&Namespace::parse("ezsdr.radio.mock.mock.faults").unwrap()];
    assert_eq!(faults.as_array().unwrap().len(), 1);
    assert_eq!(faults[0]["applied"], true);
    assert_eq!(faults[0]["lost"], 2_000);
    let applied = &instance.sections[&Namespace::parse("ezsdr.radio.mock.mock.applied").unwrap()];
    assert_eq!(applied.as_array().unwrap().len(), 2);
    assert_eq!(applied[0]["value"], 0);
    assert_eq!(applied[1]["value"], 1);
    assert_eq!(applied[0]["at"]["ticks"], at.ticks);
    assert_eq!(applied[1]["at"]["ticks"], at.ticks);
    let bursts = &instance.sections[&Namespace::parse("ezsdr.radio.mock.mock.bursts").unwrap()];
    let records: Vec<ezsdr_kernel::stream::BurstRecord> =
        serde_json::from_value(bursts.clone()).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].target.ticks, 2_005_000);
    assert_eq!(records[0].samples, 2);
    assert!(harness.link.as_ref().unwrap().queued() > 0);
}

#[test]
fn mr_20_faults_fire_at_their_instants() {
    let env = [("sim.faults", serde_json::json!([
        { "at_ns": 1_000_000, "fault": "rx_overflow", "target": "radio" },
        { "at_ns": 2_000_000, "fault": "rx_sequence_error", "target": "other" },
        { "at_ns": 20_000_000, "fault": "rx_sequence_error", "target": "radio" }
    ]))];
    let mut harness = Harness::new("x310-like", &[], &[], &env, Some((BackPressure::DropOldest, 8)));
    harness.arm_start(2_000_000_000).unwrap();
    harness.step(2_001_000_000).unwrap();
    harness.step(2_002_000_000).unwrap();
    let events = harness.events.drain();
    assert_eq!(events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW).count(), 1);
    let overflow = events.iter().find(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW).unwrap();
    let rx_domain = harness.clocks.sample_clock_records().iter().find(|record| record.stream == rid("mock/rx")).unwrap().domain;
    assert_eq!(overflow.source, rid("mock/rx"));
    assert_eq!(overflow.time, TimePoint::new(rx_domain, 1_000));
    let faults = &harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.faults").unwrap()];
    assert_eq!(faults.as_array().unwrap().len(), 1);
    assert_eq!(faults[0]["fault"], "rx_overflow");
    assert_eq!(faults[0]["applied"], true);
    harness.mock.stop(StopMode::Orderly).unwrap();
    let faults = &harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.faults").unwrap()];
    assert_eq!(faults.as_array().unwrap().len(), 2);
    assert_eq!(faults[1]["fault"], "rx_sequence_error");
    assert_eq!(faults[1]["applied"], false);
    assert_eq!(faults[1]["lost"], 0);

    let no_stream_env = [("sim.faults", serde_json::json!([{ "at_ns": 1_000_000, "fault": "rx_overflow", "target": "radio" }]))];
    let mut no_stream = Harness::new("x310-like", &[("radio.rx.channels", eq(Value::Int(0)))], &[], &no_stream_env, None);
    no_stream.arm_start(2_000_000_000).unwrap();
    no_stream.step(2_001_000_000).unwrap();
    let faults = &no_stream.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.faults").unwrap()];
    assert_eq!(faults.as_array().unwrap().len(), 1);
    assert_eq!(faults[0]["applied"], false);
    assert_eq!(faults[0]["lost"], 0);

    let lost_env = [("sim.faults", serde_json::json!([{ "at_ns": 1_000_000, "fault": "device_lost", "target": "radio" }]))];
    let mut lost = Harness::new("ideal", &[], &[], &lost_env, None);
    lost.arm_start(0).unwrap();
    let error = lost.step(1_000_000).unwrap_err();
    assert_eq!(error.kind, ezsdr_kernel::module_api::ModuleErrorKind::DeviceLost);
    lost.auth.advance_to(TimePoint::new(ROOT, 1_000_001)).unwrap();
    assert!(!lost.mock.step(TimePoint::new(ROOT, 1_000_001)).unwrap().progressed);
    lost.mock.stop(StopMode::Abort).unwrap();
    let faults = &lost.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.faults").unwrap()];
    assert_eq!(faults.as_array().unwrap().len(), 1);
    assert_eq!(faults[0]["fault"], "device_lost");
    assert_eq!(faults[0]["applied"], true);

    let pre_start_env = [
        ("sim.faults", serde_json::json!([{ "at_ns": 5_000_000, "fault": "rx_overflow", "target": "radio" }]))
    ];
    let mut pre_start = Harness::new_with_options(
        "x310-like",
        &[],
        &[],
        &pre_start_env,
        None,
        ExecutionClass::Simulation,
        Rational::new(2_000_000_000, 1).unwrap(),
        0,
        "rx",
    ).unwrap();
    pre_start.mock.stop(StopMode::Abort).unwrap();
    let faults = &pre_start.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.faults").unwrap()];
    assert_eq!(faults[0]["at"], serde_json::to_value(TimePoint::new(ROOT, 10_000_000)).unwrap());
    assert_eq!(faults[0]["applied"], false);
    assert_eq!(faults[0]["lost"], 0);

    let mut failed_start = Harness::new_with_options(
        "x310-like",
        &[],
        &[],
        &pre_start_env,
        Some((BackPressure::DropOldest, 8)),
        ExecutionClass::Simulation,
        Rational::new(2_000_000_000, 1).unwrap(),
        0,
        "rx",
    ).unwrap();
    let rx_handle = failed_start.clocks.declared_sample_clocks().into_iter()
        .find(|handle| handle.stream == rid("mock/rx")).unwrap();
    failed_start.clocks.register_sample_clock(&rx_handle, 0).unwrap();
    failed_start.mock.arm().unwrap();
    let error = failed_start.mock.start(Some(TimePoint::new(ROOT, 5_000_000_000))).unwrap_err();
    assert!(error.message.contains("already registered"));
    failed_start.mock.stop(StopMode::Abort).unwrap();
    let faults = &failed_start.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.faults").unwrap()];
    assert_eq!(faults[0]["at"], serde_json::to_value(TimePoint::new(ROOT, 10_000_000)).unwrap());

    let stopped_rx_env = [
        ("sim.faults", serde_json::json!([{ "at_ns": 3_000_000, "fault": "rx_overflow", "target": "radio" }]))
    ];
    let mut stopped_rx = Harness::new("x310-like", &[], &[], &stopped_rx_env, Some((BackPressure::DropOldest, 8)));
    stopped_rx.arm_start(2_000_000_000).unwrap();
    stopped_rx.actions.push(Action::Stop { target: Some(rid("mock/rx")) });
    stopped_rx.step(2_001_000_000).unwrap();
    stopped_rx.step(2_002_000_000).unwrap();
    stopped_rx.step(2_003_000_000).unwrap();
    let faults = &stopped_rx.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.faults").unwrap()];
    assert_eq!(faults.as_array().unwrap().len(), 1);
    assert_eq!(faults[0]["applied"], false);
    assert_eq!(faults[0]["lost"], 0);
    assert_eq!(stopped_rx.events.drain().iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW).count(), 0);

    let tail_fault_env = [
        ("sim.faults", serde_json::json!([{ "at_ns": 1_500_000, "fault": "rx_overflow", "target": "radio" }]))
    ];
    let mut tail_fault = Harness::new("x310-like", &[], &[], &tail_fault_env, Some((BackPressure::DropOldest, 8)));
    tail_fault.arm_start(2_000_000_000).unwrap();
    tail_fault.actions.push(Action::Stop { target: Some(rid("mock/rx")) });
    tail_fault.step(2_001_000_000).unwrap();
    tail_fault.step(2_001_500_000).unwrap();
    let faults = &tail_fault.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.faults").unwrap()];
    assert_eq!(faults[0]["applied"], true);
    assert_eq!(faults[0]["lost"], 500);
    let stats = &tail_fault.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.stats").unwrap()];
    assert_eq!(stats["rx_samples"], 1_500);
    let events = tail_fault.events.drain();
    let overflow = events.iter().find(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW).unwrap();
    assert_eq!(RxOverflowPayload::from_payload(&overflow.payload).unwrap().lost, 500);
}

#[test]
fn mr_21_overrun_shape() {
    let env = [("sim.faults", serde_json::json!([{ "at_ns": 1_000_000, "fault": "rx_overflow", "target": "radio" }]))];
    let mut harness = Harness::new("x310-like", &[], &[], &env, Some((BackPressure::DropOldest, 8)));
    harness.arm_start(2_000_000_000).unwrap();
    harness.step(2_052_999_001).unwrap();
    let mut headers = Vec::new();
    while let Some(block) = harness.link.as_ref().unwrap().receive() { headers.push(block.header().clone()); }
    let restarted = headers.iter().find(|header| header.first_sample_time.ticks == 51_000).unwrap();
    assert_eq!(restarted.lost, Some(50_000));
    assert!(restarted.flags.contains(BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED));
    assert!(!restarted.flags.contains(BlockFlags::SEQ_DISCONTINUITY));
    let previous = headers.iter().find(|header| header.first_sample_time.ticks == 0).unwrap();
    assert_eq!(restarted.first_sample_time.ticks - (previous.first_sample_time.ticks + i64::from(previous.len)), 50_000);

    let env = [("sim.faults", serde_json::json!([
        { "at_ns": 1_000_000, "fault": "rx_overflow", "target": "radio" },
        { "at_ns": 10_000_000, "fault": "rx_overflow", "target": "radio" }
    ]))];
    let mut overlap = Harness::new("x310-like", &[], &[], &env, Some((BackPressure::DropOldest, 8)));
    overlap.arm_start(2_000_000_000).unwrap();
    overlap.step(2_062_999_001).unwrap();
    let mut headers = Vec::new();
    while let Some(block) = overlap.link.as_ref().unwrap().receive() { headers.push(block.header().clone()); }
    let restarted = headers.iter().find(|header| header.first_sample_time.ticks == 60_000).unwrap();
    assert_eq!(restarted.lost, Some(59_000));
    assert!(restarted.flags.contains(BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED));
}

#[test]
fn mr_22_sequence_error_shape() {
    let env = [("sim.faults", serde_json::json!([{ "at_ns": 1_000_000, "fault": "rx_sequence_error", "target": "radio" }]))];
    let mut harness = Harness::new("ideal", &[], &[], &env, Some((BackPressure::DropOldest, 8)));
    harness.arm_start(0).unwrap();
    harness.step(4_999_001).unwrap();
    let mut headers = Vec::new();
    while let Some(block) = harness.link.as_ref().unwrap().receive() { headers.push(block.header().clone()); }
    let restarted = headers.iter().find(|header| header.first_sample_time.ticks == 3000).unwrap();
    assert_eq!(restarted.lost, Some(2_000));
    assert!(restarted.flags.contains(BlockFlags::GAP_BEFORE | BlockFlags::SEQ_DISCONTINUITY));
    assert!(!restarted.flags.contains(BlockFlags::RESTARTED));
}

#[test]
fn mr_24_a_bypassing_provider_gets_time_error() {
    let mut model = ezsdr_mock_radio::DeviceModel::new();
    let header = |flags| BlockHeader {
        first_sample_time: TimePoint::new(ROOT, 0),
        len: 1,
        channels: 1,
        direction: Direction::Tx,
        valid: ezsdr_kernel::stream::ChannelMask::full(1),
        flags,
        lost: None,
        contract: ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.cf32").unwrap(),
    };
    assert!(model.on_tx_block(&header(BlockFlags::START_OF_BURST)).is_ok());
    assert!(model.on_tx_block(&header(BlockFlags::START_OF_BURST)).is_err());
    model.close();
    assert!(model.on_tx_block(&header(BlockFlags::START_OF_BURST | BlockFlags::END_OF_BURST)).is_ok());
}

#[test]
fn mr_25_orderly_stop_delivers_the_tail_abort_does_not() {
    let mut orderly = Harness::new("x310-like", &[], &[], &[], Some((BackPressure::DropOldest, 16)));
    orderly.arm_start(2_000_000_000).unwrap();
    orderly.step(2_001_999_000).unwrap();
    let stop = 2_001_999_000;
    orderly.mock.stop(StopMode::Orderly).unwrap();
    orderly.step(stop + 999_001).unwrap();
    let mut blocks = Vec::new();
    while let Some(block) = orderly.link.as_ref().unwrap().receive() { blocks.push(block); }
    let last = blocks.last().unwrap().header();
    assert_eq!(2_000_000_000 + (last.first_sample_time.ticks + i64::from(last.len) - 1) * 1_000, stop + 999_000);

    let mut abort = Harness::new("x310-like", &[], &[], &[], Some((BackPressure::DropOldest, 16)));
    abort.arm_start(2_000_000_000).unwrap();
    abort.step(2_001_999_001).unwrap();
    abort.mock.stop(StopMode::Abort).unwrap();
    abort.step(2_010_000_000).unwrap();
    let mut after_abort = Vec::new();
    while let Some(block) = abort.link.as_ref().unwrap().receive() { after_abort.push(block); }
    assert_eq!(after_abort.len(), 1);
    assert_eq!(after_abort[0].header().first_sample_time.ticks, 0);
}

#[test]
fn mr_25_rx_stop_leaves_transmit_running_and_stop_records_an_unstarted_burst() {
    let constraints = [
        ("radio.rx.channels", eq(Value::Int(1))),
        ("radio.tx.channels", eq(Value::Int(1))),
    ];
    let mut harness = Harness::new("x310-like", &constraints, &[], &[], Some((BackPressure::DropOldest, 16)));
    harness.arm_start(2_000_000_000).unwrap();
    let domain = tx_domain(&harness);
    harness.actions.push(Action::Stop { target: Some(rid("mock/rx")) });
    harness.step(2_001_000_000).unwrap();
    harness.step(2_002_000_000).unwrap();
    while harness.link.as_ref().unwrap().receive().is_some() {}

    let now = 2_002_000_000;
    let now_tx = harness.clocks.convert(TimePoint::new(ROOT, now), domain).unwrap().floor();
    harness.actions.push(tx_action(domain, now_tx.ticks + 4_000, 1_000, false, LatePolicy::SendAsapAndFlag));
    harness.step(now).unwrap();
    harness.step(now + 6_000_000).unwrap();
    let stats = &harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.stats").unwrap()];
    assert!(stats["tx_blocks"].as_u64().unwrap() > 0);
    let records = &harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.bursts").unwrap()];
    let records: Vec<ezsdr_kernel::stream::BurstRecord> = serde_json::from_value(records.clone()).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].samples, 1_000);

    let mut unstarted = tx_harness("ideal");
    unstarted.arm_start(0).unwrap();
    let domain = tx_domain(&unstarted);
    unstarted.actions.push(tx_action(domain, 0, 2_000, false, LatePolicy::SendAsapAndFlag));
    unstarted.step(0).unwrap();
    unstarted.mock.stop(StopMode::Abort).unwrap();
    let rejected = &unstarted.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.rejected").unwrap()];
    assert_eq!(rejected.as_array().unwrap().len(), 1);
    assert!(rejected[0]["reason"].as_str().unwrap().contains("cancelled by stop"));
    assert_eq!(unstarted.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.bursts").unwrap()].as_array().unwrap().len(), 0);
    assert_eq!(unstarted.events.drain().iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).count(), 0);
}

#[test]
fn mr_26_cleanup_is_idempotent() {
    let mut harness = Harness::new("ideal", &[], &[], &[], Some((BackPressure::DropOldest, 2)));
    harness.arm_start(0).unwrap();
    harness.mock.cleanup();
    harness.mock.cleanup();
    assert!(!harness.mock.step(TimePoint::new(ROOT, 100)).unwrap().progressed);
}

#[test]
fn mr_29_other_actions_are_command_rejected() {
    let mut harness = Harness::new("ideal", &[], &[], &[], None);
    harness.arm_start(0).unwrap();
    harness.actions.push(Action::SetTimer { target: rid("mock"), at: ezsdr_kernel::time::AbsoluteDeadline::new(TimePoint::new(ROOT, 1)), token: 3 });
    harness.actions.push(Action::PeripheralCommand { target: rid("mock"), verb: Ident::parse("start_repeat").unwrap(), params: BTreeMap::new(), at: None });
    harness.step(1).unwrap();
    let rejected = &harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.rejected").unwrap()];
    assert_eq!(rejected.as_array().unwrap().len(), 2);
    assert_eq!(rejected[0]["action"], "set_timer");
    assert_eq!(rejected[1]["action"], "peripheral_command");
    let events = harness.events.drain();
    let rejected_events: Vec<_> = events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).collect();
    assert_eq!(rejected_events.len(), 2);
    assert!(rejected_events.iter().all(|event| event.source == rid("mock") && event.time == TimePoint::new(ROOT, 1)));
}

#[test]
fn mr_30_two_mocks_one_seed_identical_output() {
    let selector = [("block_len_jitter", Value::Bool(true)), ("rx_test_pattern", Value::Str("ramp".to_owned()))];
    let mut left = Harness::new("ideal", &[], &selector, &[("sim.seed", serde_json::json!(7))], Some((BackPressure::DropOldest, 32)));
    let mut right = Harness::new("ideal", &[], &selector, &[("sim.seed", serde_json::json!(7))], Some((BackPressure::DropOldest, 32)));
    left.arm_start(0).unwrap(); right.arm_start(0).unwrap();
    left.step(25_000_000).unwrap(); right.step(25_000_000).unwrap();
    let a: Vec<_> = std::iter::from_fn(|| left.link.as_ref().unwrap().receive()).map(|b| b.header().clone()).collect();
    let b: Vec<_> = std::iter::from_fn(|| right.link.as_ref().unwrap().receive()).map(|b| b.header().clone()).collect();
    assert_eq!(a, b);
    assert_eq!(left.mock.instance().sections, right.mock.instance().sections);
    assert_eq!(left.events.drain(), right.events.drain());
}

#[test]
fn mr_37_the_overflow_travels_the_hot_path() {
    // A one-slot ring: the hot path counts every overflow but queues one body and
    // reports the other in EVENTS_DROPPED; the control path never drops, so a Mock that
    // still used it would deliver both (RS-33, RS-34, RS-35).
    RING_DEPTH.with(|depth| depth.set(1));
    let env = [("sim.faults", serde_json::json!([
        { "at_ns": 1_000_000, "fault": "rx_overflow", "target": "radio" },
        { "at_ns": 60_000_000, "fault": "rx_sequence_error", "target": "radio" }
    ]))];
    let mut harness = Harness::new("x310-like", &[], &[], &env, Some((BackPressure::DropOldest, 8)));
    RING_DEPTH.with(|depth| depth.set(4096));
    harness.arm_start(2_000_000_000).unwrap();
    for ms in 1..=66 {
        harness.step(2_000_000_000 + ms * 1_000_000).unwrap();
    }
    let faults = &harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.faults").unwrap()];
    assert_eq!(faults.as_array().unwrap().iter().filter(|fault| fault["applied"] == true).count(), 2, "{faults}");
    let counted = harness.events.counters().into_iter()
        .find(|row| row.source == rid("mock/rx") && row.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW)
        .unwrap().count;
    assert_eq!(counted, 2);
    let events = harness.events.drain();
    let delivered: Vec<_> = events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW).collect();
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].severity, ezsdr_kernel::event::Severity::Warning, "RM-10's severity");
    assert_eq!(delivered[0].source, rid("mock/rx"));
    assert_eq!(
        RxOverflowPayload::from_payload(&delivered[0].payload).unwrap(),
        RxOverflowPayload { cause: RxOverflowCause::Overrun, lost: 50_000, restart_gap_ns: 50_000_000 }
    );
    let dropped: Vec<_> = events.iter().filter(|event| event.kind.as_str() == ezsdr_kernel::event::EventKind::EVENTS_DROPPED).collect();
    assert_eq!(dropped.len(), 1);
    assert_eq!(dropped[0].payload, serde_json::json!({ "kind": ezsdr_radio::kinds::RX_OVERFLOW, "count": 1 }));
}

#[test]
fn mr_18_fractional_cold_keeps_old_tail_to_e1() {
    for (rate, requested, end, samples) in [
        (3_000_000.0, 1_000_001, 1_001_000, 3003),
        (7_000_000.0, 1_000_001, 1_001_000, 7007),
        (3_000_000.0, 1_001_000, 1_001_000, 3003),
    ] {
        let mut h = Harness::new("ideal", &[("radio.rx.sample_rate_hz", eq(Value::Num(rate)))],
            &[], &[], Some((BackPressure::DropOldest, 64)));
        h.arm_start(0).unwrap();
        let old = h.clocks.sample_clock_records()[0].domain;
        h.actions.push(update_action("radio.rx.sample_rate_hz", Value::Num(2_000_000.0),
            UpdateClass::Cold, Some(TimePoint::new(ROOT, requested))));
        h.step(0).unwrap(); h.step(requested).unwrap();
        if requested < end { assert_eq!(h.clocks.sample_clock_records()[0].ended_at, None); }
        h.step(3_200_000).unwrap();
        assert_eq!(h.clocks.sample_clock_records()[0].ended_at, Some(TimePoint::new(ROOT, end)));
        let mut old_end = 0;
        while let Some(block) = h.link.as_ref().unwrap().receive() {
            if block.header().first_sample_time.domain == old {
                old_end = block.header().first_sample_time.ticks + i64::from(block.header().len);
            }
        }
        assert_eq!(old_end, samples, "old samples must be delivered to e1");
    }
}

#[test]
fn mr_18_fractional_cold_keeps_transmitted_tail_to_e1() {
    let mut h = Harness::new("ideal", &[("radio.tx.channels", eq(Value::Int(1))),
        ("radio.tx.sample_rate_hz", eq(Value::Num(3_000_000.0)))], &[], &[], None);
    h.arm_start(0).unwrap();
    let old = tx_domain(&h);
    h.actions.push(tx_action(old, 0, 1000, true, LatePolicy::SendAsapAndFlag));
    h.actions.push(update_action("radio.tx.sample_rate_hz", Value::Num(2_000_000.0),
        UpdateClass::Cold, Some(TimePoint::new(ROOT, 1_000_001))));
    h.step(0).unwrap(); h.step(1_000_001).unwrap();
    assert!(h.clocks.sample_clock_records().iter().find(|c| c.domain == old).unwrap().ended_at.is_none());
    h.step(1_001_000).unwrap();
    let records: Vec<ezsdr_kernel::stream::BurstRecord> = serde_json::from_value(
        h.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.bursts").unwrap()].clone()).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].samples, 3003);
    assert_eq!(records[0].target.domain, old);
    assert_eq!(records[0].end, BurstEnd::Stop);
    assert_eq!(h.clocks.sample_clock_records().iter().find(|c| c.domain == old).unwrap().ended_at,
        Some(TimePoint::new(ROOT, 1_001_000)));
}

#[test]
fn mr_18_rounded_cold_updates_keep_effective_instant_order() {
    for (same_time, first_rate, second_rate) in [(false, 2_000_000.0, 1_000_000.0),
        (false, 1_000_000.0, 2_000_000.0), (true, 2_000_000.0, 1_000_000.0)] {
        let mut h = Harness::new("ideal", &[("radio.rx.sample_rate_hz", eq(Value::Num(3_000_000.0)))],
            &[], &[], Some((BackPressure::DropOldest, 64)));
        h.arm_start(0).unwrap();
        h.actions.push(update_action("radio.rx.sample_rate_hz", Value::Num(first_rate),
            UpdateClass::Cold, Some(TimePoint::new(ROOT, 1_000_500))));
        h.actions.push(update_action("radio.rx.sample_rate_hz", Value::Num(second_rate),
            UpdateClass::Cold, Some(TimePoint::new(ROOT, if same_time { 1_000_500 } else { 1_000_001 }))));
        h.step(0).unwrap(); h.step(1_001_000).unwrap();
        let instance = h.mock.instance();
        let applied = &instance.sections[&Namespace::parse("ezsdr.radio.mock.mock.applied").unwrap()];
        let values: Vec<_> = applied.as_array().unwrap().iter().map(|row| row["value"].clone()).collect();
        let expected = if same_time { vec![serde_json::json!(first_rate), serde_json::json!(second_rate)] }
            else { vec![serde_json::json!(second_rate), serde_json::json!(first_rate)] };
        assert_eq!(values, expected);
        let times: Vec<TimePoint> = applied.as_array().unwrap().iter().map(|row| serde_json::from_value(row["at"].clone()).unwrap()).collect();
        assert!(times.windows(2).all(|pair| pair[0].ticks <= pair[1].ticks));
        for record in h.clocks.sample_clock_records() {
            if let Some(end) = record.ended_at { assert!(end.ticks >= record.origin.ticks); }
        }
    }
}

#[test]
fn mr_18_deferred_disable_enable_keeps_the_last_cut() {
    for direction in ["rx", "tx"] {
        let rate_key = format!("radio.{direction}.sample_rate_hz");
        let channels_key = format!("radio.{direction}.channels");
        let mut h = Harness::new("ideal", &[(rate_key.as_str(), eq(Value::Num(3_000_000.0))),
            (channels_key.as_str(), eq(Value::Int(1)))], &[], &[], Some((BackPressure::DropOldest, 64)));
        h.arm_start(0).unwrap();
        let stream = rid(&format!("mock/{direction}"));
        let old = h.clocks.sample_clock_records().into_iter().find(|record| record.stream == stream).unwrap().domain;
        h.actions.push(update_action(&channels_key, Value::Int(1), UpdateClass::Cold, Some(TimePoint::new(ROOT, 1_000_500))));
        h.actions.push(update_action(&channels_key, Value::Int(0), UpdateClass::Cold, Some(TimePoint::new(ROOT, 1_000_001))));
        h.step(0).unwrap(); h.step(1_001_000).unwrap();
        let records: Vec<_> = h.clocks.sample_clock_records().into_iter().filter(|record| record.stream == stream).collect();
        assert_eq!(records.len(), 2);
        let ended = records.iter().find(|record| record.domain == old).unwrap().ended_at.unwrap();
        let started = records.iter().find(|record| record.domain != old).unwrap().origin;
        assert_eq!(ended.ticks, 1_001_000);
        assert!(started.ticks >= ended.ticks);
        let instance = h.mock.instance();
        let applied = &instance.sections[&Namespace::parse("ezsdr.radio.mock.mock.applied").unwrap()];
        assert_eq!(applied[0]["value"], 0); assert_eq!(applied[1]["value"], 1);
        let times: Vec<TimePoint> = applied.as_array().unwrap().iter().map(|row| serde_json::from_value(row["at"].clone()).unwrap()).collect();
        assert!(times.windows(2).all(|pair| pair[0].ticks <= pair[1].ticks));
    }
}
