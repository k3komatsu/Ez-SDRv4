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
    ModuleRef { id: ModuleId::parse("ezsdr.radio.mock").unwrap(), version: Version::new(2, 0, 0) }
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
        profile: Some(ProfileRef { name: profile.to_owned(), version: Version::new(2, 0, 0) }),
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
    assert_eq!(d.version, Version::new(2, 0, 0));
    assert_eq!(d.kernel_api, KERNEL_API);
    assert_eq!(d.roles, [Role::Provider]);
    let requirements: Vec<_> = d.vocabularies.iter().map(|v| (v.id.as_str().to_owned(), v.req.0)).collect();
    assert_eq!(requirements, [("radio".to_owned(), Version::new(2, 0, 0)), ("sim".to_owned(), Version::new(1, 1, 0))]);
    assert_eq!(d.impl_hash, Some(ezsdr_kernel::hash::ContentHash::of_bytes(b"ezsdr.radio.mock 2.0.0")));
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
        let (lead, startup, depth, gap, bytes_per_s, wire) = if x310 {
            (2_000_000, 2_000_000_000, 16, 50_000_000, 1_000_000_000, 4)
        } else {
            (0, 0, i64::from(u32::MAX), 0, 1_i64 << 62, 8)
        };
        // Profiles 2.0.0's one start lead (RM-25; spec 22, VH-4).
        let start_lead = if x310 { 50_000_000 } else { 0 };
        assert_eq!(capabilities[&key("radio.timing.min_timed_command_lead_ns")], one(Value::Int(lead)));
        assert_eq!(capabilities[&key("radio.timing.startup_latency_ns")], one(Value::Int(startup)));
        assert_eq!(capabilities[&key("radio.timing.command_queue_depth")], one(Value::Int(depth)));
        assert_eq!(capabilities[&key("radio.timing.overflow_restart_gap_ns")], one(Value::Int(gap)));
        assert!(!capabilities.contains_key(&key("radio.timing.stop_tail_ns")) && !capabilities.contains_key(&key("radio.timing.restart_lead_ns")));
        assert_eq!(capabilities[&key("radio.timing.start_lead_ns")], one(Value::Int(start_lead)));
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
        assert_eq!(envelope["profile"]["version"], serde_json::json!({ "major": 2, "minor": 0, "patch": 0 }));
        let (tx_delay, rx_delay) = if name == "x310-like" { (45, 0) } else { (0, 0) };
        assert_eq!(capabilities[&key("radio.tx.path_delay_samples")], one(Value::Int(tx_delay)));
        assert_eq!(capabilities[&key("radio.rx.path_delay_samples")], one(Value::Int(rx_delay)));
        assert_eq!(envelope["timing"]["min_timed_command_lead_ns"], lead);
        assert_eq!(envelope["timing"]["startup_latency_ns"], startup);
        assert_eq!(envelope["timing"]["command_queue_depth"], depth);
        assert_eq!(envelope["timing"]["overflow_restart_gap_ns"], gap);
        assert!(envelope["timing"].get("stop_tail_ns").is_none() && envelope["timing"].get("restart_lead_ns").is_none());
        assert_eq!(envelope["timing"]["start_lead_ns"], start_lead);
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
    // RM-25: the receive segment begins at T0, its SampleClock registered at its first block,
    // published once its last sample, at 2 ms, has passed.
    assert!(harness.clocks.sample_clock_records().iter().all(|record| record.stream != rid("mock/rx")));
    harness.step(2_001_999_000).unwrap();
    assert!(harness.clocks.sample_clock_records().iter().all(|record| record.stream != rid("mock/rx")));
    harness.step(2_002_000_001).unwrap();
    let clocks = harness.clocks.sample_clock_records();
    assert_eq!(clocks.iter().find(|record| record.stream == rid("mock/rx")).unwrap().origin, TimePoint::new(ROOT, 2_000_000_000));
    assert_eq!(clocks.iter().find(|record| record.stream == rid("mock/tx")).unwrap().origin, TimePoint::new(ROOT, 0));
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
fn mr_17_a_burst_before_its_clock_s_origin_is_late() {
    // RM-15: a burst whose target precedes its transmit clock's origin is decided a lead
    // before that origin, so it is late, and under `send_asap` it starts at the origin with a
    // TIME_ERROR — on a clock an enable from 0 channels starts, as on one a rate change starts.
    for enable in [true, false] {
        let mut h = Harness::new("x310-like", &[("radio.tx.channels", eq(Value::Int(i64::from(!enable))))], &[], &[], None);
        h.arm_start(2_000_000_000).unwrap();
        h.step(2_001_000_000).unwrap();
        h.actions.push(if enable {
            update_action("radio.tx.channels", Value::Int(1), UpdateClass::Cold, None)
        } else {
            update_action("radio.tx.sample_rate_hz", Value::Num(2_000_000.0), UpdateClass::Cold, None)
        });
        h.step(2_001_000_000).unwrap();
        let clock = h.clocks.sample_clock_records().into_iter().rev().find(|record| record.stream == rid("mock/tx")).unwrap();
        let n = clock.root_ticks_per_tick.num() as i64;
        let target = (2_002_000_000 - clock.origin.ticks) / n;
        assert!(target < 0, "{enable}: the clock begins a start lead after the change");
        h.actions.push(tx_action(clock.domain, target, 1_000, false, LatePolicy::SendAsapAndFlag));
        h.step(2_001_000_000).unwrap();
        let events = h.events.drain();
        let late: Vec<_> = events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert_eq!(late.len(), 1, "{enable}");
        assert_eq!(late[0].payload["outcome"], "send_asap", "{enable}");
        assert_eq!(late[0].payload["late_by_ns"], clock.origin.ticks - 2_002_000_000, "{enable}");
        h.step(clock.origin.ticks + 1_000 * n).unwrap();
        let record: ezsdr_kernel::stream::BurstRecord = serde_json::from_value(
            h.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.bursts").unwrap()][0].clone()).unwrap();
        assert_eq!(record.target, TimePoint::new(clock.domain, 0), "{enable}");
        assert_eq!(record.samples, 1_000, "{enable}");
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
    // RM-16, MR-25: a `Stop` cancels no pending update, whatever its target; only `stop`
    // drops what is pending (spec 22, VH-6).
    let now = 2_001_000_000;
    for (target, applied) in [("mock/tx", 2), ("mock/rx", 2), ("mock", 2)] {
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
    harness.step(2_002_000_001).unwrap();
    let old = harness.clocks.sample_clock_records().iter().find(|record| record.stream == rid("mock/rx")).unwrap().domain;
    let effective = 2_004_000_000;
    harness.actions.push(update_action("radio.rx.sample_rate_hz", Value::Num(2_000_000.0), UpdateClass::Cold, Some(TimePoint::new(ROOT, effective))));
    harness.step(effective).unwrap();
    harness.link.as_ref().unwrap().queue.lock().unwrap().clear();
    // x310-like's ready instant and start lead: the new clock starts 50 ms after the receive
    // call in progress at the cut (2 000 samples, 2 ms) and 3 ms (RM-25, MR-18; spec 22, VH-4).
    let restarted = effective + 55_000_000;
    harness.step(restarted + 999_501).unwrap();
    let records: Vec<_> = harness.clocks.sample_clock_records().into_iter().filter(|record| record.stream == rid("mock/rx")).collect();
    assert_eq!(records[0].ended_at, Some(TimePoint::new(ROOT, effective)));
    let current = records.last().unwrap();
    assert_eq!(current.origin, TimePoint::new(ROOT, restarted));
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
    // RM-25 with `ideal`'s restart lead of zero: e₁ on the old lattice, e₂ on the new.
    for (from, to, e1, e2) in [(1_000_000.0, 20_000_000.0, 1_001_000, 1_001_000), (20_000_000.0, 1_000_000.0, 1_000_050, 1_001_000)] {
        let mut harness = Harness::new("ideal", &[("radio.rx.sample_rate_hz", eq(Value::Num(from)))], &[], &[], Some((BackPressure::DropOldest, 64)));
        harness.arm_start(0).unwrap();
        harness.actions.push(update_action("radio.rx.sample_rate_hz", Value::Num(to), UpdateClass::Cold, Some(TimePoint::new(ROOT, 1_000_037))));
        harness.step(0).unwrap();
        harness.step(1_000_037).unwrap();
        harness.step(3_200_000).unwrap();
        let records = harness.clocks.sample_clock_records();
        let old = records[0].domain;
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
    // It applies at T0, where the first segment starts, so that segment has no sample and no
    // clock, and the new one waits x310-like's receive call, 3 ms and start lead (RM-25,
    // MR-18; spec 22, VH-4).
    harness.step(t0 + 55_000_000 + 999_501).unwrap();
    let records = harness.clocks.sample_clock_records();
    assert_eq!(records.len(), 1, "one receive clock: {records:?}");
    assert_eq!(records.last().unwrap().origin, TimePoint::new(ROOT, t0 + 55_000_000));
    let applied = &harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.applied").unwrap()];
    assert_eq!(applied[0]["at"]["ticks"], t0);
    let first = harness.link.as_ref().unwrap().receive().unwrap();
    assert_eq!(first.header().first_sample_time.domain, records.last().unwrap().domain);
    assert_eq!(first.header().first_sample_time.ticks, 0);
}

#[test]
fn mr_18_a_receive_enable_from_zero_waits_the_start_lead() {
    // RM-25, MR-18 (spec 20, VF-6): a receive stream enabled from 0 channels starts at the
    // first lattice instant at or after both `e` and the end of its configuration plus the
    // profile's start lead; MockRadio's configuration ends when it receives the update.
    for (profile, at, origin) in [
        ("x310-like", None, 2_051_000_000),
        ("x310-like", Some(2_101_000_000), 2_101_000_000),
        ("ideal", None, 2_001_000_000),
    ] {
        let mut harness = Harness::new(profile, &[("radio.rx.channels", eq(Value::Int(0)))], &[], &[], Some((BackPressure::DropOldest, 8)));
        harness.arm_start(2_000_000_000).unwrap();
        harness.actions.push(update_action("radio.rx.channels", Value::Int(1), UpdateClass::Cold, at.map(|at| TimePoint::new(ROOT, at))));
        harness.step(2_001_000_000).unwrap();
        harness.step(2_200_000_000).unwrap();
        let origins: Vec<_> = harness.clocks.sample_clock_records().into_iter().filter(|record| record.stream == rid("mock/rx")).map(|record| record.origin.ticks).collect();
        assert_eq!(origins, [origin], "{profile} {at:?}");
    }
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
fn mr_18_a_burst_on_a_later_clock_survives_an_earlier_switch() {
    // MR-16, MR-18 (KC-21a): a burst booked on the clock of the second of two booked changes
    // stays booked across the first one's switch, and plays on its own clock.
    let mut h = tx_harness("ideal");
    h.arm_start(0).unwrap();
    h.actions.push(update_action("radio.tx.sample_rate_hz", Value::Num(2_000_000.0), UpdateClass::Cold, Some(TimePoint::new(ROOT, 1_000_000))));
    h.actions.push(update_action("radio.tx.sample_rate_hz", Value::Num(4_000_000.0), UpdateClass::Cold, Some(TimePoint::new(ROOT, 2_000_000))));
    h.step(0).unwrap();
    let last = tx_domain(&h);
    h.actions.push(tx_action(last, 100, 1_000, false, LatePolicy::SendAsapAndFlag));
    h.step(0).unwrap();
    h.step(3_000_000).unwrap();
    let instance = h.mock.instance();
    let records: Vec<ezsdr_kernel::stream::BurstRecord> = serde_json::from_value(instance.sections[&Namespace::parse("ezsdr.radio.mock.mock.bursts").unwrap()].clone()).unwrap();
    assert_eq!(records.iter().map(|r| (r.target, r.samples)).collect::<Vec<_>>(), [(TimePoint::new(last, 100), 1_000)]);
    assert_eq!(instance.sections[&Namespace::parse("ezsdr.radio.mock.mock.rejected").unwrap()].as_array().unwrap().len(), 0);
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
    // The second change applies at the first one's new origin, x310-like's restart lead
    // after its e₁ (RM-25; spec 20, VF-6).
    harness.step(now + 100_000_000).unwrap();
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
    over.step(now + 100_000_000).unwrap();
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
    // The fault was inserted at prepare, before the two cold updates and the burst, so it
    // meets the old stream still running: its row records the block it removed, and no
    // RX_OVERFLOW comes, since the stream ends at k_f and no block carries the loss
    // (RM-17, MR-20; spec 22, VH-5). The updates apply in the order received; reversing
    // them would leave RX off. TX still plays the held burst at the same tick.
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
    assert_eq!(harness.events.drain().iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW).count(), 0);
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
    // The second fault for `radio` comes after the overrun's 50 ms restart gap: inside it,
    // the two losses would add up on the block at 53 000 (MR-21).
    let env = [("sim.faults", serde_json::json!([
        { "at_ns": 1_000_000, "fault": "rx_overflow", "target": "radio" },
        { "at_ns": 2_000_000, "fault": "rx_sequence_error", "target": "other" },
        { "at_ns": 60_000_000, "fault": "rx_sequence_error", "target": "radio" }
    ]))];
    let mut harness = Harness::new("x310-like", &[], &[], &env, Some((BackPressure::DropOldest, 8)));
    harness.arm_start(2_000_000_000).unwrap();
    harness.step(2_001_000_000).unwrap();
    harness.step(2_002_000_000).unwrap();
    // RM-17: the overrun's event comes with the block at 51 000, which carries its loss, and
    // its row was written when it fired (MR-20; spec 22, VH-5).
    harness.step(2_053_000_000).unwrap();
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
    // RM-25: the receive clock is registered at its first block, so the clash is found then.
    failed_start.mock.start(Some(TimePoint::new(ROOT, 5_000_000_000))).unwrap();
    let error = failed_start.step(5_004_000_001).unwrap_err();
    assert!(error.message.contains("already registered"));
    failed_start.mock.cleanup();
    let faults = &failed_start.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.faults").unwrap()];
    assert_eq!(faults[0]["at"], serde_json::to_value(TimePoint::new(ROOT, 5_010_000_000)).unwrap());

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
    // RM-16: the `Stop` cuts at its instant, with no tail, so the overrun after it finds no
    // stream running and is recorded unapplied (MR-20; spec 22, VH-3, VH-5).
    let faults = &tail_fault.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.faults").unwrap()];
    assert_eq!(faults[0]["applied"], false);
    assert_eq!(faults[0]["lost"], 0);
    let stats = &tail_fault.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.stats").unwrap()];
    assert_eq!(stats["rx_samples"], 1_000);
    assert_eq!(tail_fault.events.drain().iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW).count(), 0);
}

#[test]
fn mr_20_a_fault_between_cold_clocks_is_not_applied() {
    // Old clock ends at e1 = 1 001 000, the replacement starts at e2 = 1 010 000; both faults
    // fall in [e1, e2), where no receive stream runs, so neither touches the replacement clock.
    for fault in ["rx_sequence_error", "rx_overflow"] {
        let env = [("sim.faults", serde_json::json!([{ "at_ns": 1_005_000, "fault": fault, "target": "radio" }]))];
        let mut harness = Harness::new("ideal", &[("radio.rx.sample_rate_hz", eq(Value::Num(3_000_000.0)))], &[], &env, Some((BackPressure::DropOldest, 64)));
        harness.arm_start(0).unwrap();
        harness.actions.push(update_action("radio.rx.sample_rate_hz", Value::Num(300_000.0), UpdateClass::Cold, Some(TimePoint::new(ROOT, 1_000_001))));
        harness.step(0).unwrap();
        harness.step(1_005_000).unwrap();
        let faults = harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.faults").unwrap()].clone();
        assert_eq!(faults.as_array().unwrap().len(), 1, "{fault}");
        assert_eq!(faults[0]["applied"], false, "{fault}");
        assert_eq!(faults[0]["lost"], 0, "{fault}");
        let domain = harness.clocks.sample_clock_records().last().unwrap().domain;
        harness.step(20_000_000).unwrap();
        assert_eq!(harness.events.drain().iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW).count(), 0, "{fault}");
        let mut first = None;
        while let Some(block) = harness.link.as_ref().unwrap().receive() {
            if block.header().first_sample_time.domain == domain && first.is_none() { first = Some(block.header().clone()); }
        }
        let first = first.expect("a replacement block");
        assert_eq!(first.first_sample_time.ticks, 0, "{fault}");
        assert_eq!(first.lost, None, "{fault}");
        assert_eq!(first.flags, BlockFlags::NONE, "{fault}");
    }

    // A fault at e, ordered before the cold change (spec 09 §4), still meets the running old
    // stream; when that change is refused as it applies, the old stream keeps its gap.
    let env = [("sim.faults", serde_json::json!([{ "at_ns": 1_001_000, "fault": "rx_sequence_error", "target": "radio" }]))];
    let mut harness = Harness::new("ideal", &[("radio.rx.sample_rate_hz", eq(Value::Num(3_000_000.0)))], &[], &env, Some((BackPressure::DropOldest, 64)));
    harness.arm_start(0).unwrap();
    harness.actions.push(update_action("radio.rx.sample_rate_hz", Value::Num(1e15), UpdateClass::Cold, Some(TimePoint::new(ROOT, 1_001_000))));
    harness.step(0).unwrap();
    harness.step(20_000_000).unwrap();
    let faults = harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.faults").unwrap()].clone();
    assert_eq!(faults[0]["applied"], true);
    assert_eq!(faults[0]["lost"], 2_000);
    assert_eq!(harness.clocks.sample_clock_records().iter().filter(|record| record.stream == rid("mock/rx")).count(), 1);
    let mut gap = None;
    while let Some(block) = harness.link.as_ref().unwrap().receive() {
        if block.header().lost.is_some() { gap = Some(block.header().clone()); }
    }
    let gap = gap.expect("the block after the sequence error");
    assert_eq!((gap.first_sample_time.ticks, gap.lost), (5_003, Some(2_000)));
    assert_eq!(gap.flags, BlockFlags::GAP_BEFORE | BlockFlags::SEQ_DISCONTINUITY);
}

fn rx_overflows(harness: &Harness) -> Vec<(TimePoint, RxOverflowPayload)> {
    harness.events.drain().into_iter()
        .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW)
        .map(|event| (event.time, RxOverflowPayload::from_payload(&event.payload).unwrap()))
        .collect()
}

/// The `faults` rows of one fault kind, as `(applied, lost)`, in the order written (MR-27).
fn fault_rows(harness: &Harness, kind: &str) -> Vec<(serde_json::Value, serde_json::Value)> {
    let faults = &harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.faults").unwrap()];
    faults.as_array().unwrap().iter().filter(|row| row["fault"] == kind).map(|row| (row["applied"].clone(), row["lost"].clone())).collect()
}

fn received(harness: &Harness) -> Vec<BlockHeader> {
    let mut headers = Vec::new();
    while let Some(block) = harness.link.as_ref().unwrap().receive() { headers.push(block.header().clone()); }
    headers
}

#[test]
fn mr_20a_the_overflow_comes_with_the_block_that_carries_it() {
    // RM-17: an overrun at 1 ms on x310-like at 1 MS/s loses 50 000 samples (a 50 ms restart
    // gap): its RX_OVERFLOW waits for the block at 51 000, which carries the loss, as a USRP
    // reports an overflow with the next packet (UR-18, MR-37); its row is written when it
    // fires (MR-20; spec 22, VH-5).
    let env = [("sim.faults", serde_json::json!([{ "at_ns": 1_000_000, "fault": "rx_overflow", "target": "radio" }]))];
    let mut harness = Harness::new("x310-like", &[], &[], &env, Some((BackPressure::DropOldest, 16)));
    harness.arm_start(2_000_000_000).unwrap();
    harness.step(2_002_000_000).unwrap();
    let headers = received(&harness);
    assert_eq!(headers.iter().map(|header| (header.first_sample_time.ticks, header.len)).collect::<Vec<_>>(), [(0, 1_000)]);
    assert!(rx_overflows(&harness).is_empty());
    assert_eq!(fault_rows(&harness, "rx_overflow"), [(serde_json::json!(true), serde_json::json!(50_000))]);

    harness.step(2_060_000_000).unwrap();
    let rx_domain = harness.clocks.sample_clock_records().iter().find(|record| record.stream == rid("mock/rx")).unwrap().domain;
    assert_eq!(
        rx_overflows(&harness),
        [(TimePoint::new(rx_domain, 1_000), RxOverflowPayload { cause: RxOverflowCause::Overrun, lost: 50_000, restart_gap_ns: 50_000_000 })]
    );
    assert_eq!(fault_rows(&harness, "rx_overflow"), [(serde_json::json!(true), serde_json::json!(50_000))]);
    let headers = received(&harness);
    assert_eq!((headers[0].first_sample_time.ticks, headers[0].lost), (51_000, Some(50_000)));
    assert!(headers[1..].iter().all(|header| header.lost.is_none()));
}

/// The `rejected` rows, as `(action, reason, at)`, in the order written (MR-27).
fn rejected_rows(harness: &Harness) -> Vec<(String, String, i64)> {
    let rows = &harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.rejected").unwrap()];
    rows.as_array().unwrap().iter()
        .map(|row| (row["action"].as_str().unwrap().to_owned(), row["reason"].as_str().unwrap().to_owned(), row["at"]["ticks"].as_i64().unwrap()))
        .collect()
}

fn device_lost_at(at_ns: i64) -> serde_json::Value {
    serde_json::json!({ "at_ns": at_ns, "fault": "device_lost", "target": "radio" })
}

const LOST: &str = "MR-20: device lost";

#[test]
fn mr_20_a_lost_device_delivers_every_sample_before_the_loss() {
    // MR-20 (spec 20, VF-3): the loss is a receive cut applied in the step's ordered loop,
    // so the block in progress is delivered up to the first sample at or after it, and a
    // block whose last sample comes before it is delivered in the losing step.
    for (at, last) in [(1_000_000, 1_000), (1_500_000, 1_500), (1_999_001, 2_000)] {
        let env = [("sim.faults", serde_json::json!([device_lost_at(at)]))];
        let mut harness = Harness::new("ideal", &[], &[], &env, Some((BackPressure::DropOldest, 64)));
        harness.arm_start(0).unwrap();
        let error = harness.step(at).unwrap_err();
        assert_eq!(error.kind, ezsdr_kernel::module_api::ModuleErrorKind::DeviceLost);
        let headers = received(&harness);
        assert_eq!(headers.last().map(|header| header.first_sample_time.ticks + i64::from(header.len)), Some(last), "{at}");
    }
}

#[test]
fn mr_20_a_lost_device_transmits_nothing_after_the_loss() {
    // MR-20, SE-4 (spec 20, VF-3): a lost device stops at its loss. A repeated burst open
    // at the loss transmits up to it and no further, and nothing the Run does afterwards —
    // Actions a continuing Policy dispatches, a later step, either `stop` mode — makes the
    // device transmit or receive again.
    for mode in [StopMode::Orderly, StopMode::Abort] {
        let env = [("sim.faults", serde_json::json!([device_lost_at(1_000_000)]))];
        let mut harness = Harness::new("ideal", &[("radio.tx.channels", eq(Value::Int(1)))], &[], &env, Some((BackPressure::DropOldest, 64)));
        harness.arm_start(0).unwrap();
        let tx = tx_domain(&harness);
        harness.actions.push(tx_action(tx, 500, 1_000, true, LatePolicy::SendAsapAndFlag));
        harness.step(500_000).unwrap();
        let error = harness.step(1_000_000).unwrap_err();
        assert_eq!(error.kind, ezsdr_kernel::module_api::ModuleErrorKind::DeviceLost);
        harness.actions.push(tx_action(tx, 3_000, 1_000, true, LatePolicy::SendAsapAndFlag));
        harness.actions.push(update_action("radio.rx.sample_rate_hz", Value::Num(2_000_000.0), UpdateClass::Cold, None));
        harness.actions.push(Action::Stop { target: Some(rid("mock/rx")) });
        harness.step(2_000_000).unwrap();
        harness.auth.advance_to(TimePoint::new(ROOT, 5_000_000)).unwrap();
        harness.mock.stop(mode).unwrap();
        harness.step(10_000_000).unwrap();
        let bursts = &harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.bursts").unwrap()];
        let records: Vec<ezsdr_kernel::stream::BurstRecord> = serde_json::from_value(bursts.clone()).unwrap();
        assert_eq!(records.iter().map(|record| record.samples).collect::<Vec<_>>(), [500], "{mode:?}");
        let headers = received(&harness);
        assert_eq!(headers.last().map(|header| header.first_sample_time.ticks + i64::from(header.len)), Some(1_000), "{mode:?}");
        let actions: Vec<_> = rejected_rows(&harness).into_iter().map(|(action, reason, _)| (action, reason)).collect();
        assert_eq!(actions, [("tx_burst", LOST), ("update_parameter", LOST), ("stop", LOST)].map(|(a, r)| (a.to_owned(), r.to_owned())), "{mode:?}");
    }
}

#[test]
fn mr_20_a_loss_cancels_what_it_left_pending() {
    // MR-20, MR-27 (spec 20, VF-3): the loss cancels the burst and the command it left
    // pending, at the loss and before any `stop`, recording each with the loss's reason and
    // emitting no event for either.
    let env = [("sim.faults", serde_json::json!([device_lost_at(2_000_000)]))];
    let mut harness = Harness::new("ideal", &[("radio.tx.channels", eq(Value::Int(1)))], &[], &env, Some((BackPressure::DropOldest, 64)));
    harness.arm_start(0).unwrap();
    harness.actions.push(update_action("radio.rx.gain_db", Value::Num(3.0), UpdateClass::HardwareTimed, Some(TimePoint::new(ROOT, 3_000_000))));
    harness.actions.push(tx_action(tx_domain(&harness), 3_000, 1_000, false, LatePolicy::SendAsapAndFlag));
    harness.step(1_000_000).unwrap();
    assert!(rejected_rows(&harness).is_empty());
    harness.step(2_000_000).unwrap_err();
    assert_eq!(
        rejected_rows(&harness),
        [("tx_burst".to_owned(), LOST.to_owned(), 2_000_000), ("update_parameter".to_owned(), LOST.to_owned(), 2_000_000)]
    );
    assert!(harness.events.drain().is_empty());
}

#[test]
fn mr_20_a_stop_received_in_the_losing_step_does_not_cancel_the_loss() {
    // MR-20, SE-4 (spec 20, VF-3): the Actions of the step that applies a loss reach the
    // device at or after it, so none is carried out — not even a device `Stop`, which
    // would otherwise record the loss as unapplied before the loop reached it.
    let env = [("sim.faults", serde_json::json!([device_lost_at(1_999_001)]))];
    let mut harness = Harness::new("ideal", &[], &[], &env, Some((BackPressure::DropOldest, 64)));
    harness.arm_start(0).unwrap();
    harness.step(1_000_000).unwrap();
    harness.actions.push(update_action("radio.rx.gain_db", Value::Num(3.0), UpdateClass::HardwareTimed, None));
    harness.actions.push(Action::Stop { target: Some(rid("mock")) });
    let error = harness.step(1_999_001).unwrap_err();
    assert_eq!(error.kind, ezsdr_kernel::module_api::ModuleErrorKind::DeviceLost);
    assert!(harness.actions.actions.lock().unwrap().is_empty());
    assert_eq!(fault_rows(&harness, "device_lost"), [(serde_json::json!(true), serde_json::json!(0))]);
    let actions: Vec<_> = rejected_rows(&harness).into_iter().map(|(action, reason, _)| (action, reason)).collect();
    assert_eq!(actions, [("update_parameter", LOST), ("stop", LOST)].map(|(a, r)| (a.to_owned(), r.to_owned())));
    // No event for a refused Action, COMMAND_REJECTED included: the step's error reports the loss.
    assert!(harness.events.drain().is_empty());
}

#[test]
fn mr_20_an_action_after_the_losing_step_is_refused() {
    // MR-20, MR-27, MA-30 (spec 20, VF-3): after the loss every Action that reaches the
    // device is refused and recorded — one a continuing Policy dispatches in a later round,
    // and one left queued for `stop`, as after a failure in the losing round.
    let env = [("sim.faults", serde_json::json!([device_lost_at(1_000_000)]))];
    let mut harness = Harness::new("ideal", &[], &[], &env, Some((BackPressure::DropOldest, 64)));
    harness.arm_start(0).unwrap();
    harness.step(1_000_000).unwrap_err();
    harness.actions.push(update_action("radio.rx.gain_db", Value::Num(3.0), UpdateClass::HardwareTimed, None));
    harness.step(2_000_000).unwrap();
    assert!(harness.actions.actions.lock().unwrap().is_empty());
    harness.actions.push(Action::Stop { target: Some(rid("mock")) });
    harness.auth.advance_to(TimePoint::new(ROOT, 3_000_000)).unwrap();
    harness.mock.stop(StopMode::Orderly).unwrap();
    assert_eq!(
        rejected_rows(&harness),
        [("update_parameter".to_owned(), LOST.to_owned(), 2_000_000), ("stop".to_owned(), LOST.to_owned(), 3_000_000)]
    );
    assert!(harness.events.drain().is_empty());
}

#[test]
fn mr_20_a_fault_at_a_loss_s_instant_removes_nothing() {
    // MR-20, SE-4 (spec 20, VF-3): the stream ends at the loss's instant, so a sequence
    // error due there removes no delivered sample, whichever side of the loss it is listed;
    // listed first it fires and records what it injected, listed after the loss it does not
    // fire (spec 22, VH-5).
    let sequence_error = serde_json::json!({ "at_ns": 1_000_000, "fault": "rx_sequence_error", "target": "radio" });
    for loss_first in [false, true] {
        let faults = if loss_first { [device_lost_at(1_000_000), sequence_error.clone()] } else { [sequence_error.clone(), device_lost_at(1_000_000)] };
        let env = [("sim.faults", serde_json::Value::Array(faults.to_vec()))];
        let mut harness = Harness::new("ideal", &[], &[], &env, Some((BackPressure::DropOldest, 64)));
        harness.arm_start(0).unwrap();
        let error = harness.step(1_000_000).unwrap_err();
        assert_eq!(error.kind, ezsdr_kernel::module_api::ModuleErrorKind::DeviceLost);
        harness.mock.stop(StopMode::Abort).unwrap();
        let headers = received(&harness);
        assert_eq!(headers.iter().map(|header| (header.first_sample_time.ticks, header.len)).collect::<Vec<_>>(), [(0, 1_000)], "{loss_first}");
        let row = if loss_first { (serde_json::json!(false), serde_json::json!(0)) } else { (serde_json::json!(true), serde_json::json!(2_000)) };
        assert_eq!(fault_rows(&harness, "rx_sequence_error"), [row], "{loss_first}");
        assert!(rx_overflows(&harness).is_empty(), "{loss_first}");
        assert_eq!(fault_rows(&harness, "device_lost"), [(serde_json::json!(true), serde_json::json!(0))], "{loss_first}");
    }
}

#[test]
fn mr_20_a_loss_ends_its_receive_stream_at_its_instant() {
    // MR-20 (spec 20, VF-3; spec 22, VH-1): the device lost at 1.1 ms ends the receive
    // stream and its SampleClock there, after a sequence error at 1 ms whose row, written
    // when it fired, keeps the block it injected (VH-5); the `stop` at 2 ms changes neither.
    let env = [("sim.faults", serde_json::json!([
        { "at_ns": 1_000_000, "fault": "rx_sequence_error", "target": "radio" },
        device_lost_at(1_100_000)
    ]))];
    let mut harness = Harness::new("ideal", &[], &[], &env, Some((BackPressure::DropOldest, 64)));
    harness.arm_start(0).unwrap();
    harness.step(1_000_000).unwrap();
    harness.step(1_100_000).unwrap_err();
    harness.auth.advance_to(TimePoint::new(ROOT, 2_000_000)).unwrap();
    harness.mock.stop(StopMode::Orderly).unwrap();
    assert_eq!(fault_rows(&harness, "rx_sequence_error"), [(serde_json::json!(true), serde_json::json!(2_000))]);
    let clocks: Vec<_> = harness.clocks.sample_clock_records().into_iter().map(|record| (record.origin.ticks, record.ended_at.map(|end| end.ticks))).collect();
    assert_eq!(clocks, [(0, Some(1_100_000))]);
    assert_eq!(received(&harness).iter().map(|header| (header.first_sample_time.ticks, header.len)).collect::<Vec<_>>(), [(0, 1_000)]);
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
    // One block carries both losses; each fault settles its own share with it (MR-20a).
    let overflows: Vec<_> = overlap.events.drain().into_iter()
        .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW)
        .map(|event| (event.time.ticks, RxOverflowPayload::from_payload(&event.payload).unwrap().lost))
        .collect();
    assert_eq!(overflows, [(1_000, 50_000), (51_000, 9_000)]);
    let faults = overlap.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.faults").unwrap()].clone();
    let rows: Vec<_> = faults.as_array().unwrap().iter().map(|row| (row["applied"].clone(), row["lost"].clone())).collect();
    assert_eq!(rows, [(serde_json::json!(true), serde_json::json!(50_000)), (serde_json::json!(true), serde_json::json!(9_000))]);

    // The row is what the overrun injected, `k_g − k_f`, written when it fires and never
    // revised: a cut booked for 20 ms, before `k_g`, leaves it 50 000 (VH-5).
    let env = [("sim.faults", serde_json::json!([{ "at_ns": 1_000_000, "fault": "rx_overflow", "target": "radio" }]))];
    let mut cut = Harness::new("x310-like", &[], &[], &env, Some((BackPressure::DropOldest, 8)));
    cut.arm_start(2_000_000_000).unwrap();
    cut.actions.push(update_action("radio.rx.sample_rate_hz", Value::Num(2_000_000.0), UpdateClass::Cold, Some(TimePoint::new(ROOT, 2_020_000_000))));
    cut.step(2_000_000_000).unwrap();
    cut.step(2_100_000_000).unwrap();
    assert_eq!(fault_rows(&cut, "rx_overflow"), [(serde_json::json!(true), serde_json::json!(50_000))]);
    assert!(rx_overflows(&cut).is_empty(), "no block carries the loss");
}

#[test]
fn mr_25_a_stop_cuts_at_its_instant() {
    // RM-16, MR-25 (spec 22, VH-3): a `Stop` of the stream or of the device, and `stop`,
    // end the receive stream at the first sample at or after the instant MockRadio handles
    // them: every sample before it is delivered, none at or after it, and the SampleClock
    // ends there — on `ideal` at 1 MS/s, and on `x310-like` at 390.625 kS/s, whose 2 000-sample
    // blocks last 5.12 ms (2 560 root ticks a sample).
    for (profile, rate, period, t0) in [("ideal", 1_000_000.0, 1_000, 0), ("x310-like", 390_625.0, 2_560, 2_000_000_000)] {
        for how in ["mock/rx", "mock", "stop"] {
            let mut h = Harness::new(profile, &[("radio.rx.sample_rate_hz", eq(Value::Num(rate)))], &[], &[], Some((BackPressure::DropOldest, 64)));
            h.arm_start(t0).unwrap();
            let at = t0 + 7_777_777;
            h.step(at).unwrap();
            if how == "stop" {
                h.mock.stop(StopMode::Orderly).unwrap();
            } else {
                h.actions.push(Action::Stop { target: Some(rid(how)) });
                h.step(at).unwrap();
            }
            h.step(at + 50_000_000).unwrap();
            let cut = (7_777_777 + period - 1) / period;
            let blocks = received(&h);
            let end = blocks.last().map(|header| header.first_sample_time.ticks + i64::from(header.len));
            assert_eq!(end, Some(cut), "{profile}, {how}");
            let clocks: Vec<_> = h.clocks.sample_clock_records().into_iter().filter(|record| record.stream == rid("mock/rx"))
                .map(|record| (record.origin.ticks, record.ended_at.map(|end| end.ticks))).collect();
            assert_eq!(clocks, [(t0, Some(t0 + cut * period))], "{profile}, {how}");
        }
    }
}

#[test]
fn mr_29_start_rx_is_the_one_peripheral_command() {
    // MR-29, RM-12: after a `Stop` of `mock/rx`, `start_rx` on `mock/tx`, on `mock` or with a
    // param is refused; on `mock/rx` it starts the stream again on a new clock, at its instant
    // on `ideal`, and a second one on a running stream changes nothing (RM-21, RM-25).
    let mut h = Harness::new("ideal", &[], &[], &[], Some((BackPressure::DropOldest, 64)));
    h.arm_start(0).unwrap();
    h.actions.push(Action::Stop { target: Some(rid("mock/rx")) });
    h.step(1_000_000).unwrap();
    let start = |target: &str, params: BTreeMap<Key, Value>| Action::PeripheralCommand { target: rid(target), verb: Ident::parse("start_rx").unwrap(), params, at: None };
    h.actions.push(start("mock/tx", BTreeMap::new()));
    h.actions.push(start("mock", BTreeMap::new()));
    h.actions.push(start("mock/rx", BTreeMap::from([(key("radio.now"), Value::Bool(true))])));
    h.step(2_000_000).unwrap();
    let rejected = rejected_rows(&h);
    assert_eq!(rejected.iter().map(|row| row.0.as_str()).collect::<Vec<_>>(), ["peripheral_command"; 3]);
    assert_eq!(h.events.drain().iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).count(), 3);
    h.actions.push(start("mock/rx", BTreeMap::new()));
    h.step(3_000_000).unwrap();
    h.actions.push(start("mock/rx", BTreeMap::new()));
    h.step(4_000_000).unwrap();
    h.step(10_000_000).unwrap();
    let clocks: Vec<_> = h.clocks.sample_clock_records().into_iter().map(|record| (record.origin.ticks, record.ended_at.map(|end| end.ticks))).collect();
    assert_eq!(clocks, [(0, Some(1_000_000)), (3_000_000, None)]);
    let first_new = received(&h).into_iter().find(|header| header.first_sample_time.domain != h.clocks.sample_clock_records()[0].domain).unwrap();
    assert_eq!((first_new.first_sample_time.ticks, first_new.flags), (0, BlockFlags::NONE));
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
fn mr_25_orderly_stop_ends_at_its_instant_abort_at_once() {
    // RM-16, MR-25: an orderly stop delivers every sample before its instant and none at or
    // after it, with no tail (spec 22, VH-3); an abort delivers nothing more.
    let mut orderly = Harness::new("x310-like", &[], &[], &[], Some((BackPressure::DropOldest, 16)));
    orderly.arm_start(2_000_000_000).unwrap();
    orderly.step(2_001_999_000).unwrap();
    let stop = 2_001_999_000;
    orderly.mock.stop(StopMode::Orderly).unwrap();
    orderly.step(stop + 999_001).unwrap();
    let mut blocks = Vec::new();
    while let Some(block) = orderly.link.as_ref().unwrap().receive() { blocks.push(block); }
    let last = blocks.last().unwrap().header();
    assert_eq!(2_000_000_000 + (last.first_sample_time.ticks + i64::from(last.len) - 1) * 1_000, stop - 1_000);
    let ended = orderly.clocks.sample_clock_records()[0].ended_at;
    assert_eq!(ended, Some(TimePoint::new(ROOT, stop)));

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
fn mr_25_a_stopped_stream_keeps_its_end() {
    // A receive stream's end moves only earlier (RM-16, MR-25; spec 21, VG-1; issue #46).
    // A `Stop` of `mock/rx` at T0 + 1 ms ends the stream and its clock at sample 1 000; a
    // later stop at T0 + 5 ms keeps that end (spec 22, VH-1).
    for (profile, t0, end) in [("ideal", 0, 1_000), ("x310-like", 2_000_000_000, 1_000)] {
        for later in ["stop(Orderly)", "mock/rx", "mock"] {
            let mut harness = Harness::new(profile, &[], &[], &[], Some((BackPressure::DropOldest, 64)));
            harness.arm_start(t0).unwrap();
            harness.actions.push(Action::Stop { target: Some(rid("mock/rx")) });
            harness.step(t0 + 1_000_000).unwrap();
            harness.step(t0 + 5_000_000).unwrap();
            if later == "stop(Orderly)" {
                harness.mock.stop(StopMode::Orderly).unwrap();
            } else {
                harness.actions.push(Action::Stop { target: Some(rid(later)) });
                harness.step(t0 + 5_000_000).unwrap();
            }
            harness.step(t0 + 10_000_000).unwrap();
            let last = received(&harness).pop().unwrap();
            assert_eq!(last.first_sample_time.ticks + i64::from(last.len), end, "{profile}, then {later}");
            let clocks: Vec<_> = harness.clocks.sample_clock_records().into_iter().filter(|record| record.stream == rid("mock/rx"))
                .map(|record| (record.origin.ticks, record.ended_at.map(|end| end.ticks))).collect();
            assert_eq!(clocks, [(t0, Some(t0 + 1_000_000))], "{profile}, then {later}");
        }
    }

    // An abort after the `Stop` keeps its end too.
    let mut harness = Harness::new("x310-like", &[], &[], &[], Some((BackPressure::DropOldest, 64)));
    harness.arm_start(2_000_000_000).unwrap();
    harness.actions.push(Action::Stop { target: Some(rid("mock/rx")) });
    harness.step(2_001_000_000).unwrap();
    harness.step(2_001_500_000).unwrap();
    harness.mock.stop(StopMode::Abort).unwrap();
    harness.step(2_010_000_000).unwrap();
    assert_eq!(received(&harness).iter().map(|header| (header.first_sample_time.ticks, header.len)).collect::<Vec<_>>(), [(0, 1_000)]);
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
fn mr_18_a_fractional_receive_cut_is_the_first_sample_at_or_after_e() {
    // RM-16: the old stream delivers every sample before `e` and ends at the first at or
    // after it, its clock ending at that sample's instant rounded up (MR-7).
    for (rate, requested, end, samples) in [
        (3_000_000.0, 1_000_001, 1_000_334, 3001),
        (7_000_000.0, 1_000_001, 1_000_143, 7001),
        (3_000_000.0, 1_001_000, 1_001_000, 3003),
    ] {
        let mut h = Harness::new("ideal", &[("radio.rx.sample_rate_hz", eq(Value::Num(rate)))],
            &[], &[], Some((BackPressure::DropOldest, 64)));
        h.arm_start(0).unwrap();
        h.actions.push(update_action("radio.rx.sample_rate_hz", Value::Num(2_000_000.0),
            UpdateClass::Cold, Some(TimePoint::new(ROOT, requested))));
        h.step(0).unwrap(); h.step(requested).unwrap();
        let old = h.clocks.sample_clock_records()[0].domain;
        h.step(3_200_000).unwrap();
        assert_eq!(h.clocks.sample_clock_records()[0].ended_at, Some(TimePoint::new(ROOT, end)));
        let mut old_end = 0;
        while let Some(block) = h.link.as_ref().unwrap().receive() {
            if block.header().first_sample_time.domain == old {
                old_end = block.header().first_sample_time.ticks + i64::from(block.header().len);
            }
        }
        assert_eq!(old_end, samples, "old samples must be delivered up to e");
    }
}

#[test]
fn mr_18_a_fractional_transmit_cut_is_the_first_sample_at_or_after_e() {
    // RM-16, RM-25: the change, booked at 0, ends the old transmit clock then at its cut, the
    // first sample at or after `e` (3 001, at 1 000 334), and the burst plays up to it.
    let mut h = Harness::new("ideal", &[("radio.tx.channels", eq(Value::Int(1))),
        ("radio.tx.sample_rate_hz", eq(Value::Num(3_000_000.0)))], &[], &[], None);
    h.arm_start(0).unwrap();
    let old = tx_domain(&h);
    h.actions.push(tx_action(old, 0, 1000, true, LatePolicy::SendAsapAndFlag));
    h.actions.push(update_action("radio.tx.sample_rate_hz", Value::Num(2_000_000.0),
        UpdateClass::Cold, Some(TimePoint::new(ROOT, 1_000_001))));
    h.step(0).unwrap();
    assert_eq!(h.clocks.sample_clock_records().iter().find(|c| c.domain == old).unwrap().ended_at,
        Some(TimePoint::new(ROOT, 1_000_334)));
    h.step(1_000_001).unwrap();
    h.step(1_001_000).unwrap();
    let records: Vec<ezsdr_kernel::stream::BurstRecord> = serde_json::from_value(
        h.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.mock.bursts").unwrap()].clone()).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].samples, 3001);
    assert_eq!(records[0].target.domain, old);
    assert_eq!(records[0].end, BurstEnd::Stop);
}

#[test]
fn mr_18_a_cold_change_that_arrives_after_a_later_one_follows_it() {
    // VH-2, RM-25: a `cold` change never takes effect before one of its stream received
    // earlier: the second takes the first's instant, after it, and is late.
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
        let rows: Vec<_> = applied.as_array().unwrap().iter().map(|row| (row["value"].clone(), row["at"]["ticks"].clone())).collect();
        assert_eq!(rows, [(serde_json::json!(first_rate), serde_json::json!(1_000_500)), (serde_json::json!(second_rate), serde_json::json!(1_000_500))]);
        let late = h.events.drain().into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::LATE_COMMAND).count();
        assert_eq!(late, usize::from(!same_time));
        for record in h.clocks.sample_clock_records() {
            if let Some(end) = record.ended_at { assert!(end.ticks >= record.origin.ticks); }
        }
    }
}

#[test]
fn mr_18_a_disable_that_arrives_after_a_later_change_follows_it() {
    // VH-2: a change to 0 channels received after one for a later instant takes that instant,
    // after it: the stream ends at the first one's cut, and the first one's segment ends at
    // its origin — with no clock on receive, and its transmit clock, registered at booking,
    // ended there (RM-25).
    for direction in ["rx", "tx"] {
        let rate_key = format!("radio.{direction}.sample_rate_hz");
        let channels_key = format!("radio.{direction}.channels");
        let mut h = Harness::new("ideal", &[(rate_key.as_str(), eq(Value::Num(3_000_000.0))),
            (channels_key.as_str(), eq(Value::Int(1)))], &[], &[], Some((BackPressure::DropOldest, 64)));
        h.arm_start(0).unwrap();
        let stream = rid(&format!("mock/{direction}"));
        h.actions.push(update_action(&channels_key, Value::Int(1), UpdateClass::Cold, Some(TimePoint::new(ROOT, 1_000_500))));
        h.actions.push(update_action(&channels_key, Value::Int(0), UpdateClass::Cold, Some(TimePoint::new(ROOT, 1_000_001))));
        h.step(0).unwrap(); h.step(1_001_000).unwrap();
        let records: Vec<_> = h.clocks.sample_clock_records().into_iter().filter(|record| record.stream == stream)
            .map(|record| (record.origin.ticks, record.ended_at.map(|end| end.ticks))).collect();
        let expected: &[(i64, Option<i64>)] = if direction == "rx" { &[(0, Some(1_000_667))] } else { &[(0, Some(1_000_667)), (1_001_000, Some(1_001_000))] };
        assert_eq!(records, expected, "{direction}");
        let instance = h.mock.instance();
        let applied = &instance.sections[&Namespace::parse("ezsdr.radio.mock.mock.applied").unwrap()];
        let rows: Vec<_> = applied.as_array().unwrap().iter().map(|row| (row["value"].clone(), row["at"]["ticks"].clone())).collect();
        assert_eq!(rows, [(serde_json::json!(1), serde_json::json!(1_000_500)), (serde_json::json!(0), serde_json::json!(1_000_500))], "{direction}");
    }
}
