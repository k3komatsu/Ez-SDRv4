use ezsdr_kernel::spec::Scalar;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use ezsdr_kernel::binding::{AdmissionCheckRegistry, Binding, Violation};
use ezsdr_kernel::event::{Action, ActionId, EventCollector, EventSource, Target};
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::{ClockDomainId, ModuleId, ResourceId, RunId};
use ezsdr_kernel::manifest::ArtifactRef;
use ezsdr_kernel::module_api::Requested;
use ezsdr_kernel::module_api::{
    ActionReceiver, ActionSubmitter, AttachedPort, Endpoint, ExecutionClass, Factories, KERNEL_API, ModuleError, ModuleRef, Pacing, PrepareContext, ProfileRef, Provider, Role, StopMode, UpdateClass,
    Version,
};
use ezsdr_kernel::plan::Fragment;
use ezsdr_kernel::policy::{EventKindRegistry, Policy};
use ezsdr_kernel::spec::{Constraint, Ident, Key, Namespace, Value};
use ezsdr_kernel::stream::{BackPressure, BlockFlags, BlockHeader, BlockRef, BurstEnd, BurstOpen, BurstStep, BurstTracker, DataLink, Direction, DropCarry, LatePolicy, PublishOutcome};
use ezsdr_kernel::time::{AbsoluteDeadline, ClockDomain, ClockRegistry, Duration, EpochRef, ManualTimeAuthority, Rational, RelativeBudget, TimePoint};

use ezsdr_mock_radio::{MockRadio, descriptor};
use ezsdr_radio::payloads::{RxOverflowCause, RxOverflowPayload};

#[path = "../../ezsdr-acceptance/src/experiments.rs"]
mod experiment_support;
#[path = "../../ezsdr-acceptance/src/layout.rs"]
mod layout_support;

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
        target: tgt("mock/tx/s0"),
        waveform: waveform(samples),
        repeat,
        at: AbsoluteDeadline::new(TimePoint::new(domain, target)),
        requested_at: None,
        late_policy,
        metadata: BTreeMap::new(),
    }
}

fn start_rx(target: &str, at: Option<i64>) -> Action {
    Action::Command { target: tgt(target), verb: Ident::parse("start_rx").unwrap(), params: BTreeMap::new(), at: at.map(|tick| AbsoluteDeadline::new(TimePoint::new(ROOT, tick))) }
}

fn update_action(name: &str, value: Value, class: UpdateClass, at: Option<TimePoint>) -> Action {
    Action::UpdateParameter { target: tgt("mock"), key: key(name), value, class, at: at.map(ezsdr_kernel::time::AbsoluteDeadline::new) }
}

fn tx_harness(profile: &str) -> Harness {
    Harness::new(profile, &[("radio.tx.c0.stream", eq(Scalar::from(0)))], &[], &[], None)
}

fn tx_domain(harness: &Harness) -> ClockDomainId {
    harness.clocks.sample_clock_records().iter().rev().find(|record| record.stream == rid("mock/tx/s0")).unwrap().domain
}

/// The `x3xx` selector both Mock bindings carry: a UBX in slot A and slot B, both
/// directions (design 33 D5).
fn layout(id: &str) -> BTreeMap<Ident, Value> {
    layout_support::mock_x3xx_selector(id, &["0:A:ubx:0", "0:B:ubx:0"], &["0:A:ubx:0", "0:B:ubx:0"])
}

fn binding(profile: &str) -> Binding {
    let (version, selector) = if profile == "x3xx" { (Version::new(0, 1, 0), layout("mock")) } else { (Version::new(2, 0, 0), layout_support::ideal_selector("mock", 2, 2)) };
    Binding { module: module_ref(), selector, profile: Some(ProfileRef { name: profile.to_owned(), version }), feed: None }
}

fn key(name: &str) -> Key {
    Key::parse(name).unwrap()
}
fn rid(name: &str) -> ResourceId {
    ResourceId::parse(name).unwrap()
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
        Target::Resource { resource, path } if path.is_empty() => Some(ResourceId::parse(resource.as_str()).unwrap()),
        Target::Resource { resource, path } => Some(ResourceId::parse(&format!("{resource}/{path}")).unwrap()),
        _ => None,
    }
}

fn node_src(path: &str) -> EventSource {
    EventSource::Node { node: ResourceId::parse(path).unwrap() }
}

fn request(constraints: &[(&str, Constraint)]) -> Requested {
    Requested { resource: rid("mock"), constraints: constraints.iter().map(|(name, constraint)| (key(name), constraint.clone())).collect() }
}

fn eq(value: Scalar) -> Constraint {
    Constraint::Eq { value }
}

#[derive(Default)]
struct Queue {
    actions: Mutex<VecDeque<Action>>,
    /// The ids handed out so far, as the Kernel's counter does (KC-24).
    given: std::sync::atomic::AtomicU64,
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
            let id = ActionId(self.given.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1);
            ezsdr_kernel::module_api::Dispatched::new(id, action, node)
        })
    }
}

struct RefusingSubmitter;
impl ActionSubmitter for RefusingSubmitter {
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
    fn queued(&self) -> usize {
        self.queue.lock().unwrap().len()
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
    // No test here reads a carry, so none is kept (SC-20b is HostLink's).
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

thread_local! {
    /// The event ring each harness on this thread gets: MR-37's test needs a one-slot
    /// ring to tell the hot path, which drops, from the control path, which never does.
    static RING_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(4096) };
    /// The source root each harness's context gives the Mock (KC-8).
    static SOURCE_ROOT: std::cell::Cell<&'static str> = const { std::cell::Cell::new("mock") };
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

    fn new_with_class(
        profile: &str,
        constraints: &[(&str, Constraint)],
        selector_overrides: &[(&str, Value)],
        env: &[(&str, serde_json::Value)],
        link: Option<(BackPressure, usize)>,
        class: ExecutionClass,
    ) -> Result<Self, ModuleError> {
        Self::new_with_options(profile, constraints, selector_overrides, env, link, class, Rational::new(1_000_000_000, 1).unwrap(), 0, "rx0")
    }

    #[allow(clippy::too_many_arguments)]
    fn new_with_options(
        profile: &str,
        constraints: &[(&str, Constraint)],
        selector_overrides: &[(&str, Value)],
        env: &[(&str, serde_json::Value)],
        link: Option<(BackPressure, usize)>,
        class: ExecutionClass,
        root_rate: Rational,
        extra_links: usize,
        attached_port: &str,
    ) -> Result<Self, ModuleError> {
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
        let pairs: Vec<_> = [node_src("mock"), node_src("mock/rx/s0"), node_src("mock/rx/s1"), node_src("mock/tx/s0"), node_src("mock/tx/s1")]
            .into_iter()
            .flat_map(|source| event_kinds.iter().cloned().map(move |kind| (source.clone(), kind)))
            .collect();
        let events = Arc::new(EventCollector::new(&pairs, &event_kinds, RING_DEPTH.with(|depth| depth.get()), &Policy::default()));
        let link = link.map(|(policy, capacity)| Arc::new(MemLink::new(policy, capacity)));
        let mut attached: Vec<_> =
            link.as_ref().map(|link| AttachedPort::new(Ident::parse("radio").unwrap(), Ident::parse(attached_port).unwrap(), Endpoint::StreamOut(link.clone()))).into_iter().collect();
        if let Some((policy, capacity)) = link.as_ref().map(|link| (link.policy(), link.capacity)) {
            attached.extend((0..extra_links).map(|_| AttachedPort::new(Ident::parse("radio").unwrap(), Ident::parse("rx0").unwrap(), Endpoint::StreamOut(Arc::new(MemLink::new(policy, capacity))))));
        }
        let environment = Arc::new(env.iter().map(|(name, value)| (Namespace::parse(name).unwrap(), value.clone())).collect());
        let ctx = {
            let mut ctx = PrepareContext::testing(
                EventSource::Node { node: rid(SOURCE_ROOT.with(std::cell::Cell::get)) },
                auth.clone(),
                clocks.clone(),
                events.clone(),
                actions.clone(),
                Arc::new(RefusingSubmitter),
            );
            ctx.run = RunId::from_string("test-run".to_owned());
            ctx.class = class;
            ctx.environment = environment;
            ctx.inputs = Arc::new(BTreeMap::<ContentHash, Arc<[u8]>>::new());
            ctx.links = attached;
            ctx.host_budget = RelativeBudget::new(Duration::new(ClockDomainId::HOST_MONOTONIC, 5_000_000_000)).unwrap();
            ctx
        };
        // RM-30: no channel is in a stream unless a request puts it there; these runs receive on c0.
        let mut constraints = constraints.to_vec();
        if mock.instance().tree.capabilities.contains_key(&key("radio.rx.c0.stream")) && !constraints.iter().any(|(name, _)| *name == "radio.rx.c0.stream") {
            constraints.push(("radio.rx.c0.stream", eq(Scalar::from(0))));
        }
        let fragment = Fragment {
            id: Ident::parse("radio").unwrap(),
            instance: module_ref(),
            role: Role::Provider,
            content: serde_json::json!({ "selector": {}, "requested": request(&constraints) }),
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
    let mut b = binding("x3xx");
    b.module.id = ModuleId::parse("other.mock").unwrap();
    let err = MockRadio::from_binding(&b).err().unwrap();
    assert!(err.message.starts_with("MR-2: "));
    b.profile = None;
    assert!(MockRadio::from_binding(&b).err().unwrap().message.starts_with("MR-2: "));
    b = binding("not-a-profile");
    assert!(MockRadio::from_binding(&b).err().unwrap().message.starts_with("MR-2: "));
    b = binding("ideal");
    b.selector.insert(Ident::parse("id").unwrap(), Value::from("a/b".to_owned()));
    assert!(MockRadio::from_binding(&b).err().unwrap().message.starts_with("MR-2: "));
    // MR-2: the id is one resource path segment and nothing more; it names no section
    // (MR-27), so any legal segment is accepted, and an empty one is refused.
    for id in ["Dev", "dev-rx", "dev.rx", "9dev"] {
        b = binding("ideal");
        b.selector.insert(Ident::parse("id").unwrap(), Value::from(id.to_owned()));
        assert!(MockRadio::from_binding(&b).is_ok(), "{id:?} must be accepted");
    }
    b = binding("x3xx");
    b.selector.insert(Ident::parse("id").unwrap(), Value::from(String::new()));
    assert!(MockRadio::from_binding(&b).err().unwrap().message.starts_with("MR-2: "));
    // The layout's own refusals: an element that is not four fields, a timekeeper other
    // than 0, an unknown board, a slot other than A or B, a front end other than 0, a
    // repeated `(slot, front end)` pair, and two board kinds (design 33 D5). Any slot
    // order is the channel order (RM-28): a non-prefix layout is accepted.
    for (name, element) in [("rx", "0:A:ubx"), ("rx", "1:A:ubx:0"), ("rx", "0:A:sbx:0"), ("rx", "0:C:ubx:0"), ("rx", "0:A:ubx:1")] {
        b = binding("x3xx");
        b.selector.insert(Ident::parse(name).unwrap(), Value::List(vec![Scalar::from(element)]));
        assert!(MockRadio::from_binding(&b).err().unwrap().message.starts_with("MR-2: "), "{name} {element}");
    }
    b = binding("x3xx");
    b.selector.insert(Ident::parse("rx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:A:ubx:0")]));
    assert!(MockRadio::from_binding(&b).err().unwrap().message.contains("position 1 repeats"), "the repeated pair is refused");
    // A name after `#` (RM-28): a bad one, an empty one and a repeated one are refused with the entry, and no Run is needed.
    for (rx, tx) in [(&["0:A:ubx:0#"][..], &[][..]), (&["0:A:ubx:0#7"], &[]), (&["0:A:ubx:0#a b"], &[]), (&["0:A:ubx:0#a", "0:B:ubx:0#a"], &[]), (&["0:A:ubx:0#a"], &["0:A:ubx:0#a"])] {
        b = binding("x3xx");
        b.selector.insert(Ident::parse("rx").unwrap(), Value::List(rx.iter().map(|e| Scalar::from(*e)).collect()));
        b.selector.insert(Ident::parse("tx").unwrap(), Value::List(tx.iter().map(|e| Scalar::from(*e)).collect()));
        let message = MockRadio::from_binding(&b).err().unwrap().message;
        assert!(message.starts_with("MR-2: ") && message.contains(rx.last().unwrap()), "{rx:?} {tx:?}: {message}");
    }
    b = binding("x3xx");
    b.selector.insert(Ident::parse("rx").unwrap(), Value::List(vec![Scalar::from("0:B:ubx:0")]));
    assert!(MockRadio::from_binding(&b).is_ok(), "slot B at position 0 is the channel order");
    b = binding("x3xx");
    b.selector.insert(Ident::parse("tx").unwrap(), Value::List(vec![Scalar::from("0:B:ubx:0")]));
    assert!(MockRadio::from_binding(&b).is_ok(), "slot B at position 0 is the channel order");
    b = binding("x3xx");
    b.selector.insert(Ident::parse("rx").unwrap(), Value::List(vec![Scalar::from("0:B:ubx:0"), Scalar::from("0:A:ubx:0")]));
    assert!(MockRadio::from_binding(&b).is_ok(), "a non-prefix layout is the channel order");
    b = binding("x3xx");
    b.selector.insert(Ident::parse("tx").unwrap(), Value::List(vec![Scalar::from("0:A:obx:0")]));
    assert!(MockRadio::from_binding(&b).err().unwrap().message.contains("sits in a slot that also carries `obx`; a duplex board"));
    b = binding("x3xx");
    b.selector.insert(Ident::parse("rx").unwrap(), Value::List(vec![]));
    b.selector.insert(Ident::parse("tx").unwrap(), Value::List(vec![]));
    assert!(MockRadio::from_binding(&b).err().unwrap().message.contains("names no front end"));
    b = binding("x3xx");
    b.selector.insert(Ident::parse("master_clock_hz").unwrap(), Value::List(vec![Scalar::from(184_320_000)]));
    assert!(MockRadio::from_binding(&b).err().unwrap().message.contains("master_clock_hz"));
    // X3xx-only layout selectors are unsupported by `ideal`.
    for name in ["master_clock_hz", "image", "links", "link_gbps"] {
        b = binding("ideal");
        b.selector.insert(Ident::parse(name).unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0")]));
        assert!(MockRadio::from_binding(&b).err().unwrap().message.contains("unsupported selector key"), "{name}");
    }
    // The Mock's own selector keys.
    b = binding("ideal");
    b.selector.insert(Ident::parse("instances").unwrap(), Value::from(0));
    assert!(MockRadio::from_binding(&b).err().unwrap().message.starts_with("MR-2: "));
    b = binding("ideal");
    b.selector.insert(Ident::parse("block_len_jitter").unwrap(), Value::from("yes".to_owned()));
    assert!(MockRadio::from_binding(&b).err().unwrap().message.starts_with("MR-2: "));
    b = binding("ideal");
    b.selector.insert(Ident::parse("rx_test_pattern").unwrap(), Value::from("noise".to_owned()));
    assert!(MockRadio::from_binding(&b).err().unwrap().message.starts_with("MR-2: "));
    b = binding("ideal");
    b.selector.insert(Ident::parse("unknown").unwrap(), Value::from(false));
    assert!(MockRadio::from_binding(&b).err().unwrap().message.starts_with("MR-2: "));
    // `instances` is gone (the layout is the channel count now): an `ideal` binding that
    // sets it, and an `x3xx` one, are both refused as unknown keys.
    b = binding("x3xx");
    b.selector.insert(Ident::parse("instances").unwrap(), Value::from(2));
    assert!(MockRadio::from_binding(&b).err().unwrap().message.contains("unsupported selector key `instances`"));
    b = binding("ideal");
    b.feed = Some(ezsdr_kernel::spec::SinkFeed {
        port: ezsdr_kernel::contract::PortRef { component: Ident::parse("radio").unwrap(), port: Ident::parse("rx0").unwrap() },
        policy: BackPressure::DropOldest,
        capacity: 1,
    });
    assert!(MockRadio::from_binding(&b).err().unwrap().message.starts_with("MR-2: "));
    b = binding("ideal");
    b.selector.insert(Ident::parse("arm_after").unwrap(), Value::List(vec![Scalar::from(4)]));
    assert!(MockRadio::from_binding(&b).err().unwrap().message.starts_with("MR-2: "));
    // `ideal` uses layout counts from the selector; absent counts default to one RX and no TX.
    b = binding("ideal");
    let defaults = MockRadio::from_binding(&b).unwrap();
    assert_eq!(defaults.instance().id, rid("mock"));
    assert_eq!(defaults.instance().arm_after, Vec::<ResourceId>::new());
    assert_eq!(defaults.instance().tree.capabilities[&key("radio.rx.c0.stream")], ezsdr_kernel::spec::CapabilityValue::Range { min: Scalar::from(-1), max: Scalar::from(1) });
    b.selector.remove(&Ident::parse("rx").unwrap());
    b.selector.remove(&Ident::parse("tx").unwrap());
    assert_eq!(MockRadio::from_binding(&b).unwrap().instance().tree.children.iter().map(|child| child.id.path()).collect::<Vec<_>>(), ["mock/rx/s0"]);
    // An x3xx layout requires `rx` and `tx`.
    b = binding("x3xx");
    b.selector.clear();
    assert!(MockRadio::from_binding(&b).err().unwrap().message.contains("needs `rx`"));
}

#[test]
fn mr_02_a_named_selector_builds_the_unnamed_radio() {
    // RM-28: a name after `#` is the selector's own and reaches nothing the Provider builds.
    let build = |rx: &[&str], tx: &[&str]| {
        let mut b = binding("x3xx");
        b.selector.insert(Ident::parse("rx").unwrap(), Value::List(rx.iter().map(|e| Scalar::from(*e)).collect()));
        b.selector.insert(Ident::parse("tx").unwrap(), Value::List(tx.iter().map(|e| Scalar::from(*e)).collect()));
        MockRadio::from_binding(&b).unwrap()
    };
    let plain = build(&["0:A:ubx:0", "0:B:ubx:0"], &["0:A:ubx:0", "0:B:ubx:0"]);
    let named = build(&["0:A:ubx:0#loop-rx", "0:B:ubx:0#mon"], &["0:A:ubx:0#loop-tx", "0:B:ubx:0"]);
    assert_eq!(plain.instance(), named.instance(), "the instance: id, tree, fidelity, sections");
    assert_eq!(serde_json::to_string(&plain.instance().tree).unwrap(), serde_json::to_string(&named.instance().tree).unwrap());
}

#[test]
fn mr_02_the_tree_has_the_radio_model_shape() {
    let mut b = binding("x3xx");
    // The layout's length is the channel count, per direction (design 33 D5).
    b.selector.insert(Ident::parse("rx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:B:ubx:0")]));
    b.selector.insert(Ident::parse("tx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0")]));
    let radio = MockRadio::from_binding(&b).unwrap();
    let instance = radio.instance();
    assert_eq!(instance.id, rid("mock"));
    assert_eq!(instance.tree.kind.as_str(), "radio.device");
    assert_eq!(instance.tree.children.iter().map(|child| child.id.path()).collect::<Vec<_>>(), ["mock/rx/s0", "mock/rx/s1", "mock/tx/s0"]);
    assert_eq!(instance.tree.children.iter().map(|child| child.kind.as_str()).collect::<Vec<_>>(), ["radio.rx_stream", "radio.rx_stream", "radio.tx_stream"]);
    assert!(instance.tree.children.iter().all(|child| child.capabilities.is_empty() && child.ports.is_empty() && child.children.is_empty() && !child.shareable));
    assert_eq!(instance.tree.ports.iter().map(|port| port.name.as_str()).collect::<Vec<_>>(), ["rx0", "rx1"]);
    assert_eq!(instance.tree.ports[0].direction, ezsdr_kernel::contract::PortDirection::Out);
    assert_eq!(instance.tree.ports[0].contract.as_str(), "ezsdr.stream.cf32");
    assert_eq!(instance.tree.memory_domains, [ezsdr_hostmem::HOST_MEMORY]);
    assert!(!instance.tree.shareable);
    assert_eq!(instance.tree.capabilities[&key("radio.rx.c0.stream")], ezsdr_kernel::spec::CapabilityValue::Range { min: Scalar::from(-1), max: Scalar::from(1) });
    assert_eq!(instance.tree.capabilities[&key("radio.tx.c0.stream")], ezsdr_kernel::spec::CapabilityValue::Range { min: Scalar::from(-1), max: Scalar::from(0) });
    assert_eq!(instance.tree.capabilities[&key("radio.perf.rx_bytes_per_s")], ezsdr_kernel::spec::CapabilityValue::One { value: Scalar::from(100_000_000) });
}

#[test]
fn mr_03_profile_values_reach_the_capabilities_and_the_envelope_section() {
    for (name, expected) in [("x3xx", "random_unless_timed_tune"), ("ideal", "deterministic")] {
        let radio = MockRadio::from_binding(&binding(name)).unwrap();
        let capabilities = &radio.instance().tree.capabilities;
        let one = |value| ezsdr_kernel::spec::CapabilityValue::One { value };
        let range = |min, max| ezsdr_kernel::spec::CapabilityValue::Range { min, max };
        let x3xx = name == "x3xx";
        let frequency = if x3xx { (10_000_000.0, 6_000_000_000.0, 1.0) } else { (0.0, 1.0e12, 0.0) };
        let gain = if x3xx { (0.0, 31.5, 0.5) } else { (-200.0, 200.0, 0.0) };
        for direction in ["rx", "tx"] {
            let rates = &capabilities[&key(&format!("radio.{direction}.s0.sample_rate_hz"))];
            if x3xx {
                let expected: Vec<_> = ezsdr_radio::x3xx::decimations().iter().map(|n| Scalar::try_from(200_000_000.0 / *n as f64).unwrap()).collect();
                assert_eq!(*rates, ezsdr_kernel::spec::CapabilityValue::AnyOf { values: expected });
            } else {
                assert_eq!(*rates, range(Scalar::try_from(1.0).unwrap(), Scalar::try_from(1_000_000_000.0).unwrap()));
            }
            assert_eq!(capabilities[&key(&format!("radio.{direction}.c0.frequency_hz"))], range(Scalar::try_from(frequency.0).unwrap(), Scalar::try_from(frequency.1).unwrap()));
            assert_eq!(capabilities[&key(&format!("radio.{direction}.c0.frequency_step_hz"))], one(Scalar::try_from(frequency.2).unwrap()));
            assert_eq!(capabilities[&key(&format!("radio.{direction}.c0.gain_db"))], range(Scalar::try_from(gain.0).unwrap(), Scalar::try_from(gain.1).unwrap()));
            assert_eq!(capabilities[&key(&format!("radio.{direction}.c0.gain_step_db"))], one(Scalar::try_from(gain.2).unwrap()));
        }
        assert_eq!(capabilities[&key("radio.rx.c0.antenna")], ezsdr_kernel::spec::CapabilityValue::AnyOf { values: ["RX2", "TX/RX"].into_iter().map(Scalar::from).collect() });
        assert_eq!(capabilities[&key("radio.tx.c0.antenna")], ezsdr_kernel::spec::CapabilityValue::AnyOf { values: vec![Scalar::from("TX/RX".to_owned())] });
        for name in ["radio.full_duplex", "radio.hardware_time"] {
            assert_eq!(capabilities[&key(name)], one(Scalar::from(true)));
        }
        assert_eq!(capabilities[&key("radio.rx.c0.phase_behavior_on_retune")], one(Scalar::from(expected.to_owned())));
        assert_eq!(capabilities[&key("radio.tx.repeat_max_samples")], one(Scalar::from(if x3xx { 268_435_456 } else { i64::from(u32::MAX) })));
        assert_eq!(capabilities[&key("radio.tx.repeat_align_samples")], one(Scalar::from(if x3xx { 2 } else { 1 })));
        assert_eq!(capabilities[&key("radio.rx.block_len")], one(Scalar::from(2_000)));
        let envelope = &radio.instance().sections[&Namespace::parse("ezsdr.radio.mock.envelope").unwrap()];
        let (lead, startup, depth, gap, bytes_per_s, wire, in_flight) =
            if x3xx { (2_000_000, 2_000_000_000, 16, 50_000_000, 100_000_000, 4, 10_000_000) } else { (0, 0, i64::from(u32::MAX), 0, 1_i64 << 62, 8, 0) };
        // The release window is the profile's: x3xx emulates the UHD Module's, ideal none
        // (design 38 D1).
        let window = if x3xx { 3_000_000 } else { 0 };
        // Each direction's own start lead (RM-25; design 33 D5).
        let start_lead = if x3xx { 50_000_000 } else { 0 };
        assert_eq!(capabilities[&key("radio.timing.min_timed_command_lead_ns")], one(Scalar::from(lead)));
        assert_eq!(capabilities[&key("radio.timing.startup_latency_ns")], one(Scalar::from(startup)));
        assert_eq!(capabilities[&key("radio.timing.command_queue_depth")], one(Scalar::from(depth)));
        assert_eq!(capabilities[&key("radio.timing.release_window_ns")], one(Scalar::from(window)));
        assert_eq!(capabilities[&key("radio.timing.overflow_restart_gap_ns")], one(Scalar::from(gap)));
        assert!(!capabilities.contains_key(&key("radio.timing.stop_tail_ns")) && !capabilities.contains_key(&key("radio.timing.restart_lead_ns")));
        assert_eq!(capabilities[&key("radio.timing.rx_start_lead_ns")], one(Scalar::from(start_lead)));
        assert_eq!(capabilities[&key("radio.timing.tx_start_lead_ns")], one(Scalar::from(start_lead)));
        assert_eq!(capabilities[&key("radio.timing.tx_in_flight_ns")], one(Scalar::from(in_flight)));
        assert!(!capabilities.contains_key(&key("radio.timing.start_lead_ns")));
        assert_eq!(capabilities[&key("radio.perf.rx_bytes_per_s")], one(Scalar::from(bytes_per_s)));
        assert_eq!(capabilities[&key("radio.perf.tx_bytes_per_s")], one(Scalar::from(bytes_per_s)));
        assert_eq!(capabilities[&key("radio.perf.wire_bytes_per_sample")], one(Scalar::from(wire)));
        // The common sections and layout segment table are filed under the instance source.
        let names: Vec<String> = radio.instance().sections.keys().map(|name| name.as_str().to_owned()).collect();
        assert_eq!(names, ["applied", "bursts", "envelope", "faults", "rejected", "segments", "stats"].map(|suffix| format!("ezsdr.radio.mock.{suffix}")));
        assert_eq!(envelope["profile"]["name"], name);
        let version = if name == "x3xx" { serde_json::json!({ "major": 0, "minor": 1, "patch": 0 }) } else { serde_json::json!({ "major": 2, "minor": 0, "patch": 0 }) };
        assert_eq!(envelope["profile"]["version"], version);
        let (tx_delay, rx_delay) = if name == "x3xx" { (45, 0) } else { (0, 0) };
        assert_eq!(capabilities[&key("radio.tx.c0.path_delay_samples")], one(Scalar::from(tx_delay)));
        assert_eq!(capabilities[&key("radio.rx.c0.path_delay_samples")], one(Scalar::from(rx_delay)));
        assert_eq!(envelope["timing"]["min_timed_command_lead_ns"], lead);
        assert_eq!(envelope["timing"]["startup_latency_ns"], startup);
        assert_eq!(envelope["timing"]["command_queue_depth"], depth);
        assert_eq!(envelope["timing"]["release_window_ns"], window);
        assert_eq!(envelope["timing"]["overflow_restart_gap_ns"], gap);
        assert!(envelope["timing"].get("stop_tail_ns").is_none() && envelope["timing"].get("restart_lead_ns").is_none());
        assert_eq!(envelope["timing"]["rx_start_lead_ns"], start_lead);
        // RM-6: the published receive start lead is at least the release window.
        assert!(envelope["timing"]["rx_start_lead_ns"].as_i64() >= envelope["timing"]["release_window_ns"].as_i64(), "{name}");
        assert_eq!(envelope["timing"]["tx_start_lead_ns"], start_lead);
        assert_eq!(envelope["timing"]["tx_in_flight_ns"], in_flight);
        assert_eq!(envelope["performance"]["rx_bytes_per_s"], bytes_per_s);
        assert_eq!(envelope["performance"]["tx_bytes_per_s"], bytes_per_s);
        assert_eq!(envelope["performance"]["wire_bytes_per_sample"], wire);
        assert_eq!(
            radio.instance().fidelity,
            if name == "x3xx" {
                ezsdr_kernel::module_api::Fidelity {
                    timing: ezsdr_kernel::module_api::EnvelopeFidelity::Envelope,
                    continuity: ezsdr_kernel::module_api::EnvelopeFidelity::Envelope,
                    coercion: ezsdr_kernel::module_api::CoercionFidelity::Grid,
                    rf: ezsdr_kernel::module_api::RfFidelity::None,
                    transport: ezsdr_kernel::module_api::TransportFidelity::None,
                }
            } else {
                ezsdr_kernel::module_api::Fidelity::NONE
            }
        );
    }
}

#[test]
fn mr_03_a_cbx_layer_starts_at_its_board_s_default_frequency() {
    // The Mock's configuration starts from its layout description's defaults (RM-5, RM-29, design
    // 33 D5), so a CBX layer starts on the CBX's own 2.45 GHz, not on a fixed 1 GHz that
    // its own range excludes (design 33 D12).
    let cbx = Value::List(vec![Scalar::from("0:A:cbx:0")]);
    let harness = Harness::new("x3xx", &[], &[("rx", cbx.clone()), ("tx", cbx)], &[], None);
    assert_eq!(harness.effective[&key("radio.rx.c0.frequency_hz")], Value::num(2_450_000_000.0).unwrap());
    assert_eq!(harness.effective[&key("radio.tx.c0.frequency_hz")], Value::num(2_450_000_000.0).unwrap());
}

#[test]
fn mr_04_instance() {
    let radio = MockRadio::from_binding(&binding("x3xx")).unwrap();
    assert!(radio.instance().driving.stepped);
    assert_eq!(radio.instance().min_command_lead.unwrap().ticks_in(ClockDomainId::HOST_MONOTONIC).unwrap(), 2_000_000);
    assert_eq!(radio.instance().arm_after, Vec::<ResourceId>::new());
    let ideal = MockRadio::from_binding(&binding("ideal")).unwrap();
    assert!(ideal.instance().min_command_lead.is_none());
}

#[test]
fn mr_06_coerce_cases() {
    let radio = MockRadio::from_binding(&binding("x3xx")).unwrap();
    let report = radio
        .coerce(&request(&[
            ("radio.rx.s0.sample_rate_hz", Constraint::Min { value: Scalar::try_from(1_100_000.0).unwrap() }),
            ("radio.rx.c0.gain_db", eq(Scalar::try_from(1.1).unwrap())),
            ("radio.rx.c0.antenna", Constraint::Set { values: vec![Scalar::from("TX/RX".to_owned()), Scalar::from("RX2".to_owned())] }),
        ]))
        .unwrap();
    assert_eq!(report.applied[&key("radio.rx.s0.sample_rate_hz")], Value::num(200_000_000.0 / 181.0).unwrap());
    assert_eq!(report.applied[&key("radio.rx.c0.gain_db")], Value::num(1.0).unwrap());
    assert_eq!(report.applied[&key("radio.rx.c0.antenna")], Value::from("TX/RX".to_owned()));
    assert_eq!(report.coercions.len(), 1);
    let rejected = radio
        .coerce(&request(&[("radio.tx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c1.stream", eq(Scalar::from(0))), ("radio.tx.s0.sample_rate_hz", eq(Scalar::try_from(200_000_000.0).unwrap()))]))
        .unwrap();
    assert_eq!(rejected.rejected.len(), 1);
    assert_eq!(rejected.rejected[0].key, key("radio.tx.s0.sample_rate_hz"));
    assert!(rejected.rejected[0].reason.starts_with("RM-32:"));

    let out_of_range = radio.coerce(&request(&[("radio.rx.c0.frequency_hz", eq(Scalar::try_from(7.0e9).unwrap()))])).unwrap();
    assert!(!out_of_range.applied.contains_key(&key("radio.rx.c0.frequency_hz")));
    assert_eq!(out_of_range.rejected.len(), 1);
    assert!(out_of_range.rejected[0].reason.starts_with("RM-8: 7000000000"));
    let absent_channel = radio.coerce(&request(&[("radio.rx.c2.gain_db", eq(Scalar::from(0)))])).unwrap();
    assert_eq!(absent_channel.rejected.len(), 1);
    assert_eq!(absent_channel.rejected[0].reason, "RM-4: no such channel c2");

    let tie = radio.coerce(&request(&[("radio.rx.c0.gain_db", eq(Scalar::try_from(20.25).unwrap()))])).unwrap();
    assert_eq!(tie.applied[&key("radio.rx.c0.gain_db")], Value::num(20.0).unwrap());

    let ideal = MockRadio::from_binding(&binding("ideal")).unwrap();
    let integer_rate = ideal.coerce(&request(&[("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(19_500_000.0).unwrap()))])).unwrap();
    assert_eq!(integer_rate.applied[&key("radio.rx.s0.sample_rate_hz")], Value::num(19_500_000.0).unwrap());
    let fractional_rate = ideal.coerce(&request(&[("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(1_000_000.0 / 3.0).unwrap()))])).unwrap();
    assert_eq!(fractional_rate.rejected.len(), 1);
    let bad_antenna = radio.coerce(&request(&[("radio.rx.c0.antenna", eq(Scalar::from("J1".to_owned())))])).unwrap();
    assert_eq!(bad_antenna.rejected.len(), 1);

    let rx_over_budget = radio.coerce(&request(&[("radio.rx.c1.stream", eq(Scalar::from(0))), ("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(200_000_000.0).unwrap()))])).unwrap();
    assert_eq!(rx_over_budget.rejected.len(), 1);
    assert_eq!(rx_over_budget.rejected[0].key, key("radio.rx.s0.sample_rate_hz"));
}

#[test]
fn mr_06_coerce_is_pure() {
    let radio = MockRadio::from_binding(&binding("ideal")).unwrap();
    let req = request(&[("radio.rx.c0.frequency_hz", eq(Scalar::try_from(2_400_000_000.0).unwrap()))]);
    let first = radio.coerce(&req).unwrap();
    let second = radio.coerce(&req).unwrap();
    assert_eq!(first, second);
    assert_eq!(radio.instance().sections[&Namespace::parse("ezsdr.radio.mock.stats").unwrap()]["rx_blocks"], 0);
}

#[test]
fn mr_07_prepare_cases() {
    let harness = Harness::new("x3xx", &[("radio.tx.c0.stream", eq(Scalar::from(0)))], &[], &[], None);
    let handles = harness.clocks.declared_sample_clocks();
    assert_eq!(handles.len(), 2);
    assert_eq!(handles[0].root_ticks_per_tick(), Rational::new(1000, 1).unwrap());
    assert_eq!(handles[1].root_ticks_per_tick(), Rational::new(1000, 1).unwrap());

    let mut second = Harness::new("ideal", &[], &[], &[], None);
    let context = {
        let mut ctx = PrepareContext::testing(
            EventSource::Node { node: rid(SOURCE_ROOT.with(std::cell::Cell::get)) },
            second.auth.clone(),
            second.clocks.clone(),
            second.events.clone(),
            second.actions.clone(),
            Arc::new(RefusingSubmitter),
        );
        ctx.run = RunId::from_string("second-prepare-test".to_owned());
        ctx.environment = Arc::new(BTreeMap::new());
        ctx.inputs = Arc::new(BTreeMap::<ContentHash, Arc<[u8]>>::new());
        ctx.host_budget = RelativeBudget::new(Duration::new(ClockDomainId::HOST_MONOTONIC, 5_000_000_000)).unwrap();
        ctx
    };
    let fragment = Fragment {
        id: Ident::parse("radio").unwrap(),
        instance: module_ref(),
        role: Role::Provider,
        content: serde_json::json!({ "selector": {}, "requested": request(&[("radio.rx.c0.stream", eq(Scalar::from(-1))), ("radio.tx.c0.stream", eq(Scalar::from(-1)))]) }),
        after: Vec::new(),
    };
    let result = second.mock.prepare(&fragment, context);
    let Err(error) = result else { panic!("a second prepare was accepted") };
    assert_eq!(error.kind, ezsdr_kernel::module_api::ModuleErrorKind::Rejected);
    assert!(error.message.starts_with("MR-7: "));
    second.mock.cleanup();
    let context = {
        let mut ctx = PrepareContext::testing(
            EventSource::Node { node: rid(SOURCE_ROOT.with(std::cell::Cell::get)) },
            second.auth.clone(),
            second.clocks.clone(),
            second.events.clone(),
            second.actions.clone(),
            Arc::new(RefusingSubmitter),
        );
        ctx.run = RunId::from_string("prepare-after-cleanup-test".to_owned());
        ctx.environment = Arc::new(BTreeMap::new());
        ctx.inputs = Arc::new(BTreeMap::<ContentHash, Arc<[u8]>>::new());
        ctx.host_budget = RelativeBudget::new(Duration::new(ClockDomainId::HOST_MONOTONIC, 5_000_000_000)).unwrap();
        ctx
    };
    let Err(error) = second.mock.prepare(&fragment, context) else { panic!("cleanup allowed a second prepare") };
    assert!(error.message.starts_with("MR-7: prepare was already called"));

    let Err(error) = Harness::new_with_options(
        "x3xx",
        &[("radio.rx.c1.stream", eq(Scalar::from(0))), ("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(200_000_000.0).unwrap()))],
        &[],
        &[],
        None,
        ExecutionClass::Simulation,
        Rational::new(1_000_000_000, 1).unwrap(),
        0,
        "rx0",
    ) else {
        panic!("a rejected Requested was prepared")
    };
    assert!(error.message.starts_with("MR-7: rejected requests:"));

    let Err(error) = Harness::new_with_options("ideal", &[("radio.rx.c0.stream", eq(Scalar::from(0)))], &[], &[], None, ExecutionClass::Simulation, Rational::new(3, 2).unwrap(), 0, "rx0") else {
        panic!("a non-integer root was accepted")
    };
    assert!(error.message.contains("primary root rate is not an integer"));

    let Err(error) = Harness::new_with_options(
        "ideal",
        &[("radio.rx.c0.stream", eq(Scalar::from(0)))],
        &[],
        &[],
        Some((BackPressure::DropOldest, 1)),
        ExecutionClass::Simulation,
        Rational::new(1_000_000_000, 1).unwrap(),
        0,
        "tx",
    ) else {
        panic!("a non-rx endpoint was accepted")
    };
    assert!(error.message.contains("only this fragment's rx port"));

    let Err(error) = Harness::new_with_options(
        "ideal",
        &[("radio.rx.c0.stream", eq(Scalar::from(0)))],
        &[],
        &[],
        Some((BackPressure::Block, 1)),
        ExecutionClass::Simulation,
        Rational::new(1_000_000_000, 1).unwrap(),
        1,
        "rx0",
    ) else {
        panic!("multiple Block links were accepted")
    };
    assert!(error.message.contains("multiple rx links cannot use Block"));

    let result = Harness::new_with_class("ideal", &[], &[], &[], None, ExecutionClass::RealtimeEmulation);
    let Err(error) = result else { panic!("a non-Simulation execution class was accepted") };
    assert_eq!(error.kind, ezsdr_kernel::module_api::ModuleErrorKind::Unsupported);
}

#[test]
fn mr_07_prepare_reports_the_coercions_coerce_reported() {
    let h = Harness::new("x3xx", &[("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(19_500_000.0).unwrap()))], &[], &[], None);
    let radio = MockRadio::from_binding(&binding("x3xx")).unwrap();
    let report = radio.coerce(&request(&[("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(19_500_000.0).unwrap()))])).unwrap();
    assert_eq!(report.applied[&key("radio.rx.s0.sample_rate_hz")], Value::num(20_000_000.0).unwrap());
    assert_eq!(h.prepare_coercions, report.coercions);
}

#[test]
fn mr_08_effective_holds_one_channel_and_its_stream_rate() {
    let selector = [("rx", Value::from(1)), ("tx", Value::from(0))];
    let harness = Harness::new("ideal", &[], &selector, &[], None);
    let expected: BTreeSet<Key> = ["radio.rx.c0.frequency_hz", "radio.rx.c0.gain_db", "radio.rx.c0.antenna", "radio.rx.c0.stream", "radio.rx.s0.sample_rate_hz"].into_iter().map(key).collect();
    assert_eq!(harness.effective.keys().cloned().collect::<BTreeSet<_>>(), expected);
    assert_eq!(layout_support::effective(&harness.effective, "radio.rx.c0.frequency_hz"), &Value::num(1.0e9).unwrap());
    assert_eq!(layout_support::effective(&harness.effective, "radio.rx.c0.stream"), &Value::from(0));
}

#[test]
fn mr_11_start_cases() {
    let constraints = [("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let mut harness = Harness::new("x3xx", &constraints, &[], &[], Some((BackPressure::DropOldest, 8)));
    harness.mock.arm().unwrap();
    assert!(harness.mock.start(Some(TimePoint::new(ROOT, 1_999_999_999))).unwrap_err().message.contains("ezsdr.time.start_lead_ns"));
    let early = harness.events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
    assert_eq!(early.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::LATE_COMMAND).count(), 1);
    let late = early.iter().find(|event| event.kind.as_str() == ezsdr_radio::kinds::LATE_COMMAND).unwrap();
    assert_eq!(late.source, node_src("mock"));
    assert_eq!(late.time, TimePoint::new(ROOT, 0));
    harness.mock.start(Some(TimePoint::new(ROOT, 2_000_000_000))).unwrap();
    // RM-25: the receive segment begins at T0, its SampleClock registered at its first block,
    // published once its last sample, at 2 ms, has passed.
    assert!(harness.clocks.sample_clock_records().iter().all(|record| record.stream != rid("mock/rx/s0")));
    harness.step(2_001_999_000).unwrap();
    assert!(harness.clocks.sample_clock_records().iter().all(|record| record.stream != rid("mock/rx/s0")));
    harness.step(2_002_000_001).unwrap();
    let clocks = harness.clocks.sample_clock_records();
    assert_eq!(clocks.iter().find(|record| record.stream == rid("mock/rx/s0")).unwrap().origin, TimePoint::new(ROOT, 2_000_000_000));
    assert_eq!(clocks.iter().find(|record| record.stream == rid("mock/tx/s0")).unwrap().origin, TimePoint::new(ROOT, 0));
    let mut no_link = Harness::new("x3xx", &[("radio.tx.c0.stream", eq(Scalar::from(0)))], &[], &[], None);
    no_link.mock.arm().unwrap();
    assert_eq!(no_link.clocks.sample_clock_records().iter().map(|record| record.stream.path()).collect::<Vec<_>>(), ["mock/tx/s0"]);
    no_link.mock.start(Some(TimePoint::new(ROOT, 2_000_000_000))).unwrap();
    assert_eq!(no_link.clocks.sample_clock_records().iter().map(|record| record.stream.path()).collect::<Vec<_>>(), ["mock/tx/s0"]);
}

#[test]
fn mr_05_a_layout_without_receive_channels_opens_no_receive_stream() {
    // A `tx`-only layout has no receive key, port or SampleClock (RM-29, RM-30).
    let mut harness = Harness::new("x3xx", &[("radio.tx.c0.stream", eq(Scalar::from(0)))], &[("rx", Value::List(Vec::new())), ("tx", Value::List(vec![Scalar::from("0:A:ubx:0")]))], &[], None);
    assert!(!harness.effective.contains_key(&key("radio.rx.c0.stream")));
    assert_eq!(layout_support::effective(&harness.effective, "radio.tx.c0.stream"), &Value::from(0));
    assert!(harness.mock.instance().tree.ports.is_empty());
    harness.mock.arm().unwrap();
    harness.mock.start(Some(TimePoint::new(ROOT, 2_000_000_000))).unwrap();
    harness.step(2_002_000_001).unwrap();
    assert!(harness.clocks.sample_clock_records().iter().all(|record| record.stream != rid("mock/rx/s0")));
}

#[test]
fn mr_12_block_lengths() {
    let mut harness = Harness::new("ideal", &[], &[("block_len_jitter", Value::from(true))], &[], Some((BackPressure::DropOldest, 64)));
    harness.arm_start(0).unwrap();
    harness.step(25_000_000).unwrap();
    let mut sizes = Vec::new();
    let mut starts = Vec::new();
    while let Some((block, _)) = harness.link.as_ref().unwrap().receive() {
        sizes.push(block.header().len);
        starts.push(block.header().first_sample_time.ticks_in(block.header().first_sample_time.domain()).unwrap());
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
    let plain_sizes: Vec<_> = std::iter::from_fn(|| plain.link.as_ref().unwrap().receive().map(|(b, _)| b)).map(|block| block.header().len).collect();
    assert!(!plain_sizes.is_empty());
    assert!(plain_sizes.iter().all(|size| *size == 2000));

    let mut other_seed = Harness::new("ideal", &[], &[("block_len_jitter", Value::from(true))], &[("sim.seed", serde_json::json!(8))], Some((BackPressure::DropOldest, 64)));
    other_seed.arm_start(0).unwrap();
    other_seed.step(25_000_000).unwrap();
    let other_sizes: Vec<_> = std::iter::from_fn(|| other_seed.link.as_ref().unwrap().receive().map(|(b, _)| b)).map(|block| block.header().len).collect();
    assert_ne!(sizes, other_sizes);
}

#[test]
fn mr_12_jitter_draws_belong_to_blocks_after_a_receive_cut() {
    let env = [("sim.seed", serde_json::json!(7)), ("sim.faults", serde_json::json!([{ "at_ns": 3_100_000, "fault": "rx_overflow", "target": "radio" }]))];
    let selector = [("block_len_jitter", Value::from(true))];
    let mut harness = Harness::new("x3xx", &[], &selector, &env, Some((BackPressure::DropOldest, 8)));
    harness.arm_start(2_000_000_000).unwrap();
    harness.step(2_003_100_000).unwrap();
    harness.step(2_056_000_000).unwrap();
    let blocks: Vec<_> = std::iter::from_fn(|| harness.link.as_ref().unwrap().receive().map(|(b, _)| b)).collect();
    let mut rng = ezsdr_sim::SimRng::new(7, "mock/rx/s0");
    let first_len = 1 + rng.below(4_000) as i64;
    let second_len = 1 + rng.below(4_000) as i64;
    let third_len = 1 + rng.below(4_000) as i64;
    let fault_sample = 3_100;
    assert!(first_len < fault_sample && first_len + second_len > fault_sample);
    assert_eq!(blocks.len(), 3);
    assert_eq!(i64::from(blocks[0].header().len), first_len);
    assert_eq!(i64::from(blocks[1].header().len), fault_sample - first_len);
    assert_eq!(blocks[2].header().first_sample_time.ticks_in(blocks[2].header().first_sample_time.domain()).unwrap(), 53_100);
    assert_eq!(i64::from(blocks[2].header().len), third_len);
}

#[test]
fn mr_13_ramp_values() {
    let mut harness = Harness::new("ideal", &[("radio.rx.c1.stream", eq(Scalar::from(0)))], &[("rx_test_pattern", Value::from("ramp".to_owned()))], &[], Some((BackPressure::DropOldest, 4)));
    harness.arm_start(0).unwrap();
    harness.step(1_999_001).unwrap();
    let block = harness.link.as_ref().unwrap().receive().unwrap().0;
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

    let mut zero = Harness::new("ideal", &[("radio.rx.c1.stream", eq(Scalar::from(0)))], &[], &[], Some((BackPressure::DropOldest, 4)));
    zero.arm_start(0).unwrap();
    zero.step(1_999_001).unwrap();
    let block = zero.link.as_ref().unwrap().receive().unwrap().0;
    assert_eq!(block.buffer().memory_domain, ezsdr_hostmem::HOST_MEMORY);
    assert!(block.host_bytes().unwrap().iter().all(|byte| *byte == 0));
}

#[test]
fn mr_14_blocks_appear_when_their_last_sample_has_occurred() {
    let pending_faults = [(
        "sim.faults",
        serde_json::json!([
            { "at_ns": 5_000_000, "fault": "rx_overflow", "target": "radio" }
        ]),
    )];
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
    assert_eq!(harness.link.as_ref().unwrap().receive().unwrap().0.header().len, 2000);
    assert_eq!(harness.auth.next_due(), Some(TimePoint::new(ROOT, 3_999_001)));
}

#[test]
fn mr_16_burst_refusals() {
    let mut harness = tx_harness("x3xx");
    harness.arm_start(2_000_000_000).unwrap();
    let domain = tx_domain(&harness);
    let mut wrong_target = tx_action(domain, 2_010_000, 2, false, LatePolicy::SendAsapAndFlag);
    if let Action::TxBurst { target, .. } = &mut wrong_target {
        *target = tgt("mock/rx/s0");
    }
    harness.actions.push(wrong_target);
    harness.actions.push(tx_action(ROOT, 2_010_000, 2, false, LatePolicy::SendAsapAndFlag));
    let mut bad_size = tx_action(domain, 2_010_000, 2, false, LatePolicy::SendAsapAndFlag);
    if let Action::TxBurst { waveform, .. } = &mut bad_size {
        waveform.size_bytes = 7;
    }
    harness.actions.push(bad_size);
    harness.actions.push(tx_action(domain, 2_010_000, 1, true, LatePolicy::SendAsapAndFlag));
    let mut metadata = tx_action(domain, 2_010_000, 2, false, LatePolicy::SendAsapAndFlag);
    if let Action::TxBurst { metadata, .. } = &mut metadata {
        metadata.insert(key("test.tag"), Value::from(true));
    }
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
    let rows = &harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.rejected").unwrap()];
    assert_eq!(rows.as_array().unwrap().len(), 7);
    let reasons: Vec<_> = rows.as_array().unwrap().iter().map(|row| row["reason"].as_str().unwrap()).collect();
    assert!(reasons.iter().all(|reason| reason.starts_with("MR-16:")));
    assert!(reasons[3].contains("repeat"));
    assert!(reasons[5].contains("repeat"));
    assert!(reasons[4].contains("metadata"));
    // MR-29: each refusal at receipt names the Action it answers (ids 1.. in push order).
    let events = harness.events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
    let answered: Vec<_> = events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).map(|event| event.payload["action"].clone()).collect();
    assert_eq!(answered[..3], [1, 2, 3], "{answered:?}");

    let mut no_tx = Harness::new("ideal", &[], &[], &[], None);
    no_tx.arm_start(0).unwrap();
    no_tx.actions.push(tx_action(ROOT, 0, 2, false, LatePolicy::SendAsapAndFlag));
    no_tx.step(0).unwrap();
    assert!(no_tx.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.rejected").unwrap()][0]["reason"].as_str().unwrap().starts_with("MR-16:"));

    let mut before_sync = tx_harness("x3xx");
    before_sync.arm_start(2_000_000_000).unwrap();
    let domain = tx_domain(&before_sync);
    before_sync.actions.push(tx_action(domain, 0, 2, false, LatePolicy::SendAsapAndFlag));
    before_sync.step(0).unwrap();
    assert!(before_sync.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.rejected").unwrap()][0]["reason"].as_str().unwrap().starts_with("MR-16:"));

    let mut overlap = tx_harness("ideal");
    overlap.arm_start(0).unwrap();
    let domain = tx_domain(&overlap);
    overlap.actions.push(tx_action(domain, 0, 4_000, false, LatePolicy::SendAsapAndFlag));
    overlap.step(0).unwrap();
    overlap.step(1_999_001).unwrap();
    overlap.actions.push(tx_action(domain, 1_999, 2, false, LatePolicy::SendAsapAndFlag));
    overlap.step(1_999_001).unwrap();
    let rejected = &overlap.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.rejected").unwrap()];
    assert_eq!(rejected.as_array().unwrap().len(), 1);
    assert!(rejected[0]["reason"].as_str().unwrap().contains("open burst's next sample"));

    let mut moved_duplicate = tx_harness("ideal");
    moved_duplicate.arm_start(2_000_000_000).unwrap();
    let domain = tx_domain(&moved_duplicate);
    moved_duplicate.actions.push(tx_action(domain, 2_001_000, 2, false, LatePolicy::SendAsapAndFlag));
    moved_duplicate.actions.push(tx_action(domain, 2_000_000, 2, false, LatePolicy::SendAsapAndFlag));
    moved_duplicate.step(2_001_000_000).unwrap();
    let rejected = &moved_duplicate.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.rejected").unwrap()];
    assert_eq!(rejected.as_array().unwrap().len(), 1);
    assert!(rejected[0]["reason"].as_str().unwrap().contains("held burst already has this start"));
    let events = moved_duplicate.events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
    assert_eq!(events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).count(), 1);
    let refused = events.iter().find(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).unwrap();
    assert_eq!(refused.payload["outcome"], "refused");
    assert_eq!(refused.source, node_src("mock/tx/s0"));
    assert_eq!(refused.time.domain(), domain);
    assert_eq!(events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).count(), 1);

    let mut x3xx_move = tx_harness("x3xx");
    x3xx_move.arm_start(2_000_000_000).unwrap();
    let domain = tx_domain(&x3xx_move);
    x3xx_move.actions.push(tx_action(domain, 2_003_000, 2, false, LatePolicy::SendAsapAndFlag));
    x3xx_move.actions.push(tx_action(domain, 2_000_000, 2, false, LatePolicy::SendAsapAndFlag));
    x3xx_move.step(2_001_000_000).unwrap();
    x3xx_move.step(2_004_001_000).unwrap();
    let records: Vec<ezsdr_kernel::stream::BurstRecord> =
        x3xx_move.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.bursts").unwrap()].as_array().unwrap().iter().cloned().map(serde_json::from_value).collect::<Result<_, _>>().unwrap();
    assert_eq!(records.len(), 2, "the x3xx front moves B after A instead of refusing it");
    assert_eq!(records[1].target, TimePoint::new(domain, 2_003_004));
    assert_eq!(records[1].requested_target, Some(TimePoint::new(domain, 2_000_000)));
    let events = x3xx_move.events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
    let moved = events.iter().find(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).unwrap();
    assert_eq!(moved.payload["outcome"], "send_asap");
    assert_eq!(moved.payload["late_by_ns"], 3_004_000);
    assert_eq!(moved.payload["target"], serde_json::to_value(TimePoint::new(domain, 2_000_000)).unwrap());
    assert_eq!(events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).count(), 0);
}

#[test]
fn mr_17_x3xx_moved_start_overlap_still_refuses() {
    let t0 = 2_000_000_000;
    let mut harness = tx_harness("x3xx");
    harness.arm_start(t0).unwrap();
    let domain = tx_domain(&harness);
    harness.actions.push(tx_action(domain, 2_004_000, 2_000, true, LatePolicy::DropAndFlag));
    harness.actions.push(tx_action(domain, 2_010_002, 2, false, LatePolicy::DropAndFlag));
    harness.actions.push(tx_action(domain, 2_003_000, 2, false, LatePolicy::SendAsapAndFlag));
    harness.step(t0).unwrap();

    let rejected = &harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.rejected").unwrap()];
    assert_eq!(rejected.as_array().unwrap().len(), 1, "the x3xx move still reaches held-start refusal");
    assert!(rejected[0]["reason"].as_str().unwrap().contains("held burst already has this start"));
    assert!(harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.bursts").unwrap()].as_array().unwrap().is_empty());
    let events = harness.events.drain(ezsdr_kernel::time::TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0));
    let moved = events.iter().find(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).unwrap();
    assert_eq!(moved.payload["outcome"], "refused");
    assert_eq!(moved.payload["late_by_ns"], 7_002_000);
    assert_eq!(moved.payload["target"], serde_json::to_value(TimePoint::new(domain, 2_003_000)).unwrap());
}

/// D2'' counts earlier receipt order, even at the same instant; Vision §59 permits Mock to be stricter than UHD.
#[test]
fn mr_17_x3xx_tx_preemption_front() {
    let t0 = 2_000_000_000;
    for (case, same_step, now, front) in [("same step", true, t0, 2_010_002), ("separate step, same instant", false, t0, 2_010_002), ("one root tick later", false, t0 + 1, 2_010_003)] {
        let mut harness = tx_harness("x3xx");
        harness.arm_start(t0).unwrap();
        let domain = tx_domain(&harness);
        harness.actions.push(tx_action(domain, 2_004_000, 2_000, true, LatePolicy::SendAsapAndFlag));
        if same_step {
            harness.actions.push(tx_action(domain, 2_003_000, 2_000, false, LatePolicy::SendAsapAndFlag));
            harness.step(now).unwrap();
        } else {
            harness.step(t0).unwrap();
            harness.actions.push(tx_action(domain, 2_003_000, 2_000, false, LatePolicy::SendAsapAndFlag));
            harness.step(now).unwrap();
        }
        let events = harness.events.drain(ezsdr_kernel::time::TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0));
        let late = events.iter().find(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).unwrap_or_else(|| panic!("{case}: {events:?}"));
        assert_eq!(late.payload["outcome"], "send_asap", "{case}");
        assert_eq!(late.payload["late_by_ns"], (front - 2_003_000) * 1_000, "{case}");
        assert_eq!(late.payload["target"], serde_json::to_value(TimePoint::new(domain, 2_003_000)).unwrap(), "{case}");
        assert_eq!(events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).count(), 0, "{case}");

        let end = t0 + (front + 2_000 - 2_000_000) * 1_000 + 1;
        harness.step(end).unwrap();
        let records: Vec<ezsdr_kernel::stream::BurstRecord> =
            harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.bursts").unwrap()].as_array().unwrap().iter().cloned().map(serde_json::from_value).collect::<Result<_, _>>().unwrap();
        assert_eq!(records.len(), 2, "{case}: {records:?}");
        assert_eq!(records[0].samples, (front - 2_004_000) as u64, "{case}: A's end is B's moved start");
        assert_eq!(records[1].target, TimePoint::new(domain, front), "{case}: B's target");
        assert_eq!(records[1].requested_target, Some(TimePoint::new(domain, 2_003_000)), "{case}: original B target");
    }

    for (case, same_step, now, front) in [("same step", true, t0, 2_010_002), ("separate step, same instant", false, t0, 2_010_002), ("one root tick later", false, t0 + 1, 2_010_003)] {
        let mut dropped = tx_harness("x3xx");
        dropped.arm_start(t0).unwrap();
        let domain = tx_domain(&dropped);
        dropped.actions.push(tx_action(domain, 2_004_000, 2_000, true, LatePolicy::SendAsapAndFlag));
        if same_step {
            dropped.actions.push(tx_action(domain, 2_003_000, 2_000, false, LatePolicy::DropAndFlag));
            dropped.step(now).unwrap();
        } else {
            dropped.step(t0).unwrap();
            dropped.actions.push(tx_action(domain, 2_003_000, 2_000, false, LatePolicy::DropAndFlag));
            dropped.step(now).unwrap();
        }
        let events = dropped.events.drain(ezsdr_kernel::time::TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0));
        let late = events.iter().find(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).unwrap_or_else(|| panic!("{case}: {events:?}"));
        assert_eq!(late.payload["outcome"], "drop", "{case}");
        assert_eq!(late.payload["late_by_ns"], (front - 2_003_000) * 1_000, "{case}");
        assert!(dropped.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.bursts").unwrap()].as_array().unwrap().is_empty(), "{case}");
    }
}

#[test]
fn mr_17_x3xx_tx_preemption_boundaries() {
    let t0 = 2_000_000_000;
    for (case, a, b_target, policy, expected_outcome, expected_start, expected_late) in [
        ("no predecessor", None, 2_005_000, LatePolicy::DropAndFlag, None, Some(2_005_000), 0),
        ("finite end inside window", Some((2_002_000, 1_000, false)), 2_005_000, LatePolicy::DropAndFlag, None, Some(2_005_000), 0),
        ("finite end clamped at window", Some((2_002_000, 28_000, false)), 2_005_000, LatePolicy::DropAndFlag, Some("drop"), None, 5_002_000),
        ("held repeat and target inside W", Some((2_004_000, 2_000, true)), 2_006_000, LatePolicy::DropAndFlag, Some("drop"), None, 4_002_000),
        ("handed horizon", Some((2_004_000, 2_000, true)), 2_010_000, LatePolicy::DropAndFlag, Some("drop"), None, 2_000),
        ("one sample into the margin", Some((2_004_000, 2_000, true)), 2_010_001, LatePolicy::DropAndFlag, Some("drop"), None, 1_000),
        ("front boundary", Some((2_004_000, 2_000, true)), 2_010_002, LatePolicy::SendAsapAndFlag, None, Some(2_010_002), 0),
        ("one sample after front", Some((2_004_000, 2_000, true)), 2_010_003, LatePolicy::SendAsapAndFlag, None, Some(2_010_003), 0),
    ] {
        let mut harness = tx_harness("x3xx");
        harness.arm_start(t0).unwrap();
        let domain = tx_domain(&harness);
        if let Some((start, len, repeat)) = a {
            harness.actions.push(tx_action(domain, start, len, repeat, LatePolicy::DropAndFlag));
        }
        harness.actions.push(tx_action(domain, b_target, 2_000, false, policy));
        harness.step(t0).unwrap();
        let events = harness.events.drain(ezsdr_kernel::time::TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0));
        let errors: Vec<_> = events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert_eq!(errors.first().map(|event| event.payload["outcome"].as_str().unwrap()), expected_outcome, "{case}: {events:?}");
        if let Some(start) = expected_start {
            assert!(errors.is_empty(), "{case}: {errors:?}");
            let end = t0 + (start + 2_000 - 2_000_000) * 1_000 + 1;
            harness.step(end).unwrap();
            let records: Vec<ezsdr_kernel::stream::BurstRecord> = harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.bursts").unwrap()]
                .as_array()
                .unwrap()
                .iter()
                .cloned()
                .map(serde_json::from_value)
                .collect::<Result<_, _>>()
                .unwrap();
            assert!(records.iter().any(|record| record.target == TimePoint::new(domain, start) && record.samples == 2_000), "{case}: {records:?}");
        } else {
            assert_eq!(errors[0].payload["late_by_ns"], expected_late, "{case}");
            assert!(harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.bursts").unwrap()].as_array().unwrap().is_empty(), "{case}");
        }
    }
}

#[test]
fn mr_17_stop_preserves_front_until_expiry() {
    let t0 = 2_000_000_000;
    let mut harness = tx_harness("x3xx");
    harness.arm_start(t0).unwrap();
    let domain = tx_domain(&harness);
    harness.actions.push(tx_action(domain, 2_004_000, 2_000, true, LatePolicy::DropAndFlag));
    harness.actions.push(Action::Stop { target: Some(tgt("mock/tx/s0")) });
    harness.step(t0).unwrap();
    let record: ezsdr_kernel::stream::BurstRecord = serde_json::from_value(harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.bursts").unwrap()][0].clone()).unwrap();
    assert_eq!((record.samples, record.end), (6_000, BurstEnd::Stop));

    for (target, outcome, late_by) in [(2_010_000, Some("drop"), 2_000), (2_010_001, Some("drop"), 1_000), (2_010_002, None, 0)] {
        harness.actions.push(tx_action(domain, target, 2, false, LatePolicy::DropAndFlag));
        harness.step(t0).unwrap();
        let events = harness.events.drain(ezsdr_kernel::time::TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0));
        let late = events.iter().find(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR);
        assert_eq!(late.map(|event| event.payload["outcome"].as_str().unwrap()), outcome, "target {target}");
        if let Some(late) = late {
            assert_eq!(late.payload["late_by_ns"], late_by, "target {target}");
        }
    }
    harness.step(2_012_004_001).unwrap();
    let records = harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.bursts").unwrap()].as_array().unwrap();
    assert_eq!(records.len(), 2, "the safe H+2 burst is admitted: {records:?}");
    let b: ezsdr_kernel::stream::BurstRecord = serde_json::from_value(records[1].clone()).unwrap();
    assert_eq!(b.target, TimePoint::new(domain, 2_010_002));
}

#[test]
fn mr_17_tx_front_is_per_stream() {
    let t0 = 2_000_000_000;
    let constraints = [("radio.tx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c1.stream", eq(Scalar::from(1)))];
    let mut harness = Harness::new("x3xx", &constraints, &[], &[], None);
    harness.arm_start(t0).unwrap();
    let domain0 = tx_domain(&harness);
    let domain1 = harness.clocks.sample_clock_records().iter().find(|record| record.stream == rid("mock/tx/s1")).unwrap().domain;
    harness.actions.push(tx_action(domain0, 2_004_000, 2_000, true, LatePolicy::DropAndFlag));
    harness.step(t0).unwrap();
    let mut b = tx_action(domain1, 2_003_000, 2_000, false, LatePolicy::DropAndFlag);
    if let Action::TxBurst { target, .. } = &mut b {
        *target = tgt("mock/tx/s1");
    }
    harness.actions.push(b);
    harness.step(t0).unwrap();
    assert!(
        harness.events.drain(ezsdr_kernel::time::TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR),
        "stream 0's front affected stream 1"
    );
    harness.step(t0 + 5_000_001).unwrap();
    let records: Vec<ezsdr_kernel::stream::BurstRecord> =
        harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.bursts").unwrap()].as_array().unwrap().iter().cloned().map(serde_json::from_value).collect::<Result<_, _>>().unwrap();
    assert!(records.iter().any(|record| record.target == TimePoint::new(domain1, 2_003_000) && record.samples == 2_000), "stream 1's on-time B was not sent: {records:?}");
}

#[test]
fn mr_17_a_target_after_a_stopped_burst_is_on_time() {
    let t0 = 2_000_000_000;
    let mut harness = tx_harness("x3xx");
    harness.arm_start(t0).unwrap();
    let domain = tx_domain(&harness);
    harness.actions.push(tx_action(domain, 2_004_000, 2_000, true, LatePolicy::DropAndFlag));
    harness.actions.push(Action::Stop { target: Some(tgt("mock/tx/s0")) });
    harness.step(t0).unwrap();
    let now = t0 + 20_000_000;
    harness.step(now).unwrap();
    harness.actions.push(tx_action(domain, 2_023_000, 2, false, LatePolicy::DropAndFlag));
    harness.step(now).unwrap();
    assert!(harness.events.drain(ezsdr_kernel::time::TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR));
    harness.step(t0 + 25_003_001).unwrap();
    let records: Vec<ezsdr_kernel::stream::BurstRecord> =
        harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.bursts").unwrap()].as_array().unwrap().iter().cloned().map(serde_json::from_value).collect::<Result<_, _>>().unwrap();
    assert!(records.iter().any(|record| record.target == TimePoint::new(domain, 2_023_000)));
}

#[test]
fn mr_17_ideal_closed_finite_burst_has_no_extra_gap() {
    let mut harness = tx_harness("ideal");
    harness.arm_start(0).unwrap();
    let domain = tx_domain(&harness);
    harness.actions.push(tx_action(domain, 2_000, 2_000, false, LatePolicy::SendAsapAndFlag));
    harness.step(0).unwrap();
    harness.step(4_000_000).unwrap();
    harness.actions.push(tx_action(domain, 4_000, 2, false, LatePolicy::SendAsapAndFlag));
    harness.step(4_000_000).unwrap();
    let events = harness.events.drain(ezsdr_kernel::time::TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0));
    assert!(events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "W=0 preserves the ideal profile's gap-free start");
    harness.step(4_003_001).unwrap();
    let records: Vec<ezsdr_kernel::stream::BurstRecord> =
        harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.bursts").unwrap()].as_array().unwrap().iter().cloned().map(serde_json::from_value).collect::<Result<_, _>>().unwrap();
    assert_eq!(records[0].end, BurstEnd::Eob);
    assert_eq!(records[1].target, TimePoint::new(domain, 4_000));
    assert_eq!(records[1].requested_target, None);
}

#[test]
fn mr_17_ideal_stop_has_no_in_flight_gap() {
    let mut harness = tx_harness("ideal");
    harness.arm_start(0).unwrap();
    let domain = tx_domain(&harness);
    harness.actions.push(tx_action(domain, 0, 2_000, true, LatePolicy::SendAsapAndFlag));
    harness.step(0).unwrap();
    harness.step(2_000_000).unwrap();
    harness.actions.push(Action::Stop { target: Some(tgt("mock/tx/s0")) });
    harness.step(2_000_000).unwrap();
    harness.actions.push(tx_action(domain, 2_000, 2, false, LatePolicy::SendAsapAndFlag));
    harness.step(2_000_000).unwrap();
    let events = harness.events.drain(ezsdr_kernel::time::TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0));
    assert!(events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "W=0 preserves the ideal profile's gap-free Stop");
    harness.step(2_003_001).unwrap();
    let records: Vec<ezsdr_kernel::stream::BurstRecord> =
        harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.bursts").unwrap()].as_array().unwrap().iter().cloned().map(serde_json::from_value).collect::<Result<_, _>>().unwrap();
    assert!(records.iter().any(|record| record.target == TimePoint::new(domain, 2_000) && record.requested_target.is_none()), "B did not start on time: {records:?}");
}

#[test]
fn mr_17_a_finite_burst_front_stops_at_its_end() {
    let t0 = 2_000_000_000;
    let mut harness = tx_harness("x3xx");
    harness.arm_start(t0).unwrap();
    let domain = tx_domain(&harness);
    harness.actions.push(tx_action(domain, 2_002_000, 6_000, false, LatePolicy::DropAndFlag));
    harness.actions.push(tx_action(domain, 2_005_000, 2, false, LatePolicy::SendAsapAndFlag));
    harness.step(t0).unwrap();
    let events = harness.events.drain(ezsdr_kernel::time::TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0));
    let late = events.iter().find(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).unwrap();
    assert_eq!(late.payload["outcome"], "send_asap");
    assert_eq!(late.payload["late_by_ns"], 3_002_000);
    harness.step(t0 + 10_001_000).unwrap();
    let records: Vec<ezsdr_kernel::stream::BurstRecord> =
        harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.bursts").unwrap()].as_array().unwrap().iter().cloned().map(serde_json::from_value).collect::<Result<_, _>>().unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!((records[0].samples, records[0].end), (6_000, BurstEnd::Eob));
    assert_eq!(records[1].target, TimePoint::new(domain, 2_008_002));
    assert_eq!(records[1].requested_target, Some(TimePoint::new(domain, 2_005_000)));
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
    let record: ezsdr_kernel::stream::BurstRecord = serde_json::from_value(harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.bursts").unwrap()][0].clone()).unwrap();
    assert_eq!((record.blocks, record.samples, record.wraps, record.end), (5, 5_000, 5, BurstEnd::Stop));

    let mut tracker = BurstTracker::new(ROOT);
    let mut at = 0;
    for (index, len) in [300, 300, 300, 100, 300].into_iter().enumerate() {
        let header = BlockHeader {
            first_sample_time: TimePoint::new(ROOT, at),
            len,
            channels: 1,
            direction: Direction::Tx,
            valid: ezsdr_kernel::stream::ChannelMask::full(1),
            flags: if index == 0 { BlockFlags::START_OF_BURST } else { BlockFlags::NONE },
            lost: None,
            contract: ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.cf32").unwrap(),
        };
        let open = (index == 0).then_some(BurstOpen { waveform_len: Some(1_000), ..BurstOpen::default() });
        let result = tracker.on_block(&header, open).unwrap();
        assert_eq!(result, if index == 0 { BurstStep::Started } else { BurstStep::Continued });
        at += i64::from(len);
    }
}

#[test]
fn mr_17_late_policy_outcomes() {
    for (policy, outcome, transmitted) in [(LatePolicy::SendAsapAndFlag, "send_asap", true), (LatePolicy::DropAndFlag, "drop", false), (LatePolicy::RejectAtPlan, "plan_violation", false)] {
        let mut harness = tx_harness("x3xx");
        harness.arm_start(2_000_000_000).unwrap();
        let domain = tx_domain(&harness);
        harness.step(2_010_000_000).unwrap();
        harness.actions.push(tx_action(domain, 2_011_000, 1_000, false, policy));
        harness.step(2_010_000_000).unwrap();
        let events = harness.events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        let late = events.iter().find(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).unwrap();
        assert_eq!(late.payload["outcome"], outcome);
        assert_eq!(late.payload["late_by_ns"], 1_000_000);
        assert_eq!(late.source, node_src("mock/tx/s0"));
        assert_eq!(late.time, TimePoint::new(domain, 2_010_000));
        assert_eq!(events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).count(), 1);
        assert_eq!(events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).count(), 0);
        if transmitted {
            harness.step(2_012_999_001).unwrap();
            let record: ezsdr_kernel::stream::BurstRecord = serde_json::from_value(harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.bursts").unwrap()][0].clone()).unwrap();
            assert_eq!(record.late_by.unwrap().ticks_in(ClockDomainId::HOST_MONOTONIC).unwrap(), 1_000_000);
            assert_eq!(record.samples, 1_000);
        } else {
            assert!(harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.bursts").unwrap()].as_array().unwrap().is_empty());
            let rejected = &harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.rejected").unwrap()];
            assert_eq!(rejected.as_array().unwrap().len(), 1);
            assert!(rejected[0]["reason"].as_str().unwrap().starts_with("MR-17:"));
        }
    }
}

#[test]
fn rm_14_the_recorded_requested_target_is_the_requested_at() {
    // RM-14 (SC-23a): `requested_target` is what the caller asked for — SC-23a's
    // `requested_at` when admission already advanced the target — and not the `at` the burst
    // starts at: an on-time burst at its `at`, and one `SendAsap` moves onto the device lead.
    for (at, target) in [(2_030_000, 2_030_000), (2_000_000, 2_012_000)] {
        let mut harness = tx_harness("x3xx");
        harness.arm_start(2_000_000_000).unwrap();
        let domain = tx_domain(&harness);
        harness.step(2_010_000_000).unwrap();
        let requested = TimePoint::new(domain, at - 10_000);
        let mut action = tx_action(domain, at, 1_000, false, LatePolicy::SendAsapAndFlag);
        let Action::TxBurst { requested_at, .. } = &mut action else { unreachable!("tx_action's action") };
        *requested_at = Some(AbsoluteDeadline::new(requested));
        harness.actions.push(action);
        harness.step(2_010_000_000).unwrap();
        harness.step(2_031_000_000).unwrap();
        let record: ezsdr_kernel::stream::BurstRecord = serde_json::from_value(harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.bursts").unwrap()][0].clone()).unwrap();
        assert_eq!(record.target, TimePoint::new(domain, target), "at {at}: the start the burst took");
        assert_eq!(record.requested_target, Some(requested), "at {at}: the requested target is the `requested_at`");
    }
}

#[test]
fn mr_09_the_transmit_clock_starts_on_its_lattice() {
    // RM-25: 1 Msps on a 1 GHz root is a ratio of 1 000; armed at 7, the origin is 1 000.
    let mut harness = tx_harness("ideal");
    harness.auth.advance_to(TimePoint::new(ROOT, 7)).unwrap();
    harness.mock.arm().unwrap();
    let record = harness.clocks.sample_clock_records().into_iter().find(|r| r.stream == rid("mock/tx/s0")).unwrap();
    assert_eq!(record.origin, TimePoint::new(ROOT, 1_000));
}

#[test]
fn mr_19_backpressure_is_an_overrun() {
    let mut harness = Harness::new("x3xx", &[], &[], &[], Some((BackPressure::Block, 1)));
    harness.arm_start(2_000_000_000).unwrap();
    harness.step(2_001_999_001).unwrap();
    harness.step(2_003_999_001).unwrap();
    let first = harness.link.as_ref().unwrap().receive().unwrap().0;
    assert_eq!(first.header().first_sample_time.ticks_in(first.header().first_sample_time.domain()).unwrap(), 0);
    harness.step(2_053_999_001).unwrap();
    let restarted = harness.link.as_ref().unwrap().receive().unwrap().0;
    assert_eq!(restarted.header().first_sample_time.ticks_in(restarted.header().first_sample_time.domain()).unwrap(), 52_000);
    assert!(restarted.header().flags.contains(BlockFlags::GAP_BEFORE));
    assert!(restarted.header().flags.contains(ezsdr_kernel::stream::BlockFlags::RESTARTED));
    assert_eq!(restarted.header().lost, Some(50_000));
    assert_eq!(harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.stats").unwrap()]["rx_blocks"], 2);
    let events = harness.events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
    let overflows: Vec<_> = events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW).collect();
    assert_eq!(overflows.len(), 1);
    // MR-37: the back-pressure overrun travels the hot path too, as RM-24's bytes.
    assert!(overflows[0].payload.is_array(), "{}", overflows[0].payload);
    assert_eq!(RxOverflowPayload::from_payload(&overflows[0].payload).unwrap(), RxOverflowPayload { cause: RxOverflowCause::Overrun, lost: 50_000 });

    let mut fractional = Harness::new_with_options("x3xx", &[], &[], &[], Some((BackPressure::Block, 1)), ExecutionClass::Simulation, Rational::new(1_000_000_001, 1).unwrap(), 0, "rx0").unwrap();
    let start = 2_000_000_002;
    fractional.arm_start(start).unwrap();
    fractional.step(start + 4_000_000).unwrap();
    let first = fractional.link.as_ref().unwrap().receive().unwrap().0;
    assert_eq!(first.header().first_sample_time.ticks_in(first.header().first_sample_time.domain()).unwrap(), 0);
    fractional.step(start + 54_001_000).unwrap();
    let restarted = fractional.link.as_ref().unwrap().receive().unwrap().0;
    assert_eq!(restarted.header().first_sample_time.ticks_in(restarted.header().first_sample_time.domain()).unwrap(), 52_001);
    assert_eq!(restarted.header().lost, Some(50_001));
}

#[test]
fn mr_19_a_zero_gap_still_loses_the_dropped_block() {
    // MR-19: `k_g = max(b, a + gap)` — a restart gap of 0 (`ideal`) still begins at the
    // dropped block's end `b`, so the whole dropped block is lost and the next block jumps
    // its length; `a + gap` alone would begin it at the dropped block's own start.
    let mut harness = Harness::new("ideal", &[], &[], &[], Some((BackPressure::Block, 1)));
    harness.arm_start(0).unwrap();
    // Block 0 [0, 2 000) fills the one-slot link; block 1 [2 000, 4 000) finds it Full.
    harness.step(1_999_001).unwrap();
    harness.step(3_999_001).unwrap();
    let first = harness.link.as_ref().unwrap().receive().unwrap().0;
    assert_eq!(first.header().first_sample_time.ticks_in(first.header().first_sample_time.domain()).unwrap(), 0);
    // The block at `b` = 4 000, carrying the whole dropped block as its loss.
    harness.step(5_999_001).unwrap();
    let restarted = harness.link.as_ref().unwrap().receive().unwrap().0;
    assert_eq!(restarted.header().first_sample_time.ticks_in(restarted.header().first_sample_time.domain()).unwrap(), 4_000);
    assert!(restarted.header().flags.contains(BlockFlags::GAP_BEFORE));
    assert!(restarted.header().flags.contains(BlockFlags::RESTARTED));
    assert_eq!(restarted.header().lost, Some(2_000));
    let events = harness.events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
    let overflows: Vec<_> = events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW).collect();
    assert_eq!(overflows.len(), 1);
    assert_eq!(RxOverflowPayload::from_payload(&overflows[0].payload).unwrap(), RxOverflowPayload { cause: RxOverflowCause::Overrun, lost: 2_000 });
}

#[test]
fn mr_19_a_fault_overflow_and_repeated_backpressure_are_counted_once() {
    let env = [("sim.faults", serde_json::json!([{ "at_ns": 1_000_000, "fault": "rx_overflow", "target": "radio" }]))];
    let mut harness = Harness::new("x3xx", &[], &[], &env, Some((BackPressure::Block, 1)));
    harness.arm_start(2_000_000_000).unwrap();
    harness.step(2_001_999_001).unwrap();
    assert_eq!(harness.link.as_ref().unwrap().queued(), 1);
    assert!(rx_overflows(&harness).is_empty(), "the fault waits for its restarted block");

    harness.step(2_053_999_001).unwrap();
    harness.step(2_103_999_001).unwrap();
    let events: Vec<_> = rx_overflows(&harness).into_iter().map(|(time, payload)| (time.ticks_in(time.domain()).unwrap(), payload.cause, payload.lost)).collect();
    assert_eq!(events, [(1_000, RxOverflowCause::Overrun, 50_000), (51_000, RxOverflowCause::Overrun, 50_000), (101_000, RxOverflowCause::Overrun, 50_000),]);
    assert_eq!(fault_rows(&harness, "rx_overflow"), [(serde_json::json!(true), serde_json::json!(50_000))]);

    harness.mock.stop(StopMode::Orderly).unwrap();
    assert!(rx_overflows(&harness).is_empty(), "stop before the next block emits nothing");
}

#[test]
fn mr_20_faults_fire_at_their_instants() {
    // The second fault for `radio` comes after the overrun's 50 ms restart gap: inside it,
    // the two losses would add up on the block at 53 000 (MR-21).
    let env = [(
        "sim.faults",
        serde_json::json!([
            { "at_ns": 1_000_000, "fault": "rx_overflow", "target": "radio" },
            { "at_ns": 2_000_000, "fault": "rx_sequence_error", "target": "other" },
            { "at_ns": 60_000_000, "fault": "rx_sequence_error", "target": "radio" }
        ]),
    )];
    let mut harness = Harness::new("x3xx", &[], &[], &env, Some((BackPressure::DropOldest, 8)));
    harness.arm_start(2_000_000_000).unwrap();
    harness.step(2_001_000_000).unwrap();
    harness.step(2_002_000_000).unwrap();
    // RM-17: the overrun's event comes with the block at 51 000, which carries its loss, and
    // its row was written when it fired (MR-20; spec 22, VH-5).
    harness.step(2_053_000_000).unwrap();
    let events = harness.events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
    assert_eq!(events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW).count(), 1);
    let overflow = events.iter().find(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW).unwrap();
    let rx_domain = harness.clocks.sample_clock_records().iter().find(|record| record.stream == rid("mock/rx/s0")).unwrap().domain;
    assert_eq!(overflow.source, node_src("mock/rx/s0"));
    assert_eq!(overflow.time, TimePoint::new(rx_domain, 1_000));
    let faults = &harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.faults").unwrap()];
    assert_eq!(faults.as_array().unwrap().len(), 1);
    assert_eq!(faults[0]["fault"], "rx_overflow");
    assert_eq!(faults[0]["applied"], true);
    harness.mock.stop(StopMode::Orderly).unwrap();
    let faults = &harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.faults").unwrap()];
    assert_eq!(faults.as_array().unwrap().len(), 2);
    assert_eq!(faults[1]["fault"], "rx_sequence_error");
    assert_eq!(faults[1]["applied"], false);
    assert_eq!(faults[1]["lost"], 0);

    let no_stream_env = [("sim.faults", serde_json::json!([{ "at_ns": 1_000_000, "fault": "rx_overflow", "target": "radio" }]))];
    let mut no_stream = Harness::new("x3xx", &[("radio.rx.c0.stream", eq(Scalar::from(-1)))], &[], &no_stream_env, None);
    no_stream.arm_start(2_000_000_000).unwrap();
    no_stream.step(2_001_000_000).unwrap();
    let faults = &no_stream.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.faults").unwrap()];
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
    let faults = &lost.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.faults").unwrap()];
    assert_eq!(faults.as_array().unwrap().len(), 1);
    assert_eq!(faults[0]["fault"], "device_lost");
    assert_eq!(faults[0]["applied"], true);

    let pre_start_env = [("sim.faults", serde_json::json!([{ "at_ns": 5_000_000, "fault": "rx_overflow", "target": "radio" }]))];
    let mut pre_start = Harness::new_with_options("x3xx", &[], &[], &pre_start_env, None, ExecutionClass::Simulation, Rational::new(2_000_000_000, 1).unwrap(), 0, "rx0").unwrap();
    pre_start.mock.stop(StopMode::Abort).unwrap();
    let faults = &pre_start.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.faults").unwrap()];
    assert_eq!(faults[0]["at"], serde_json::to_value(TimePoint::new(ROOT, 10_000_000)).unwrap());
    assert_eq!(faults[0]["applied"], false);
    assert_eq!(faults[0]["lost"], 0);

    let mut failed_start =
        Harness::new_with_options("x3xx", &[], &[], &pre_start_env, Some((BackPressure::DropOldest, 8)), ExecutionClass::Simulation, Rational::new(2_000_000_000, 1).unwrap(), 0, "rx0").unwrap();
    let rx_handle = failed_start.clocks.declared_sample_clocks().into_iter().find(|handle| *handle.stream() == rid("mock/rx/s0")).unwrap();
    failed_start.clocks.register_sample_clock(&rx_handle, 0).unwrap();
    failed_start.mock.arm().unwrap();
    // RM-25: the receive clock is registered at its first block, so the clash is found then.
    failed_start.mock.start(Some(TimePoint::new(ROOT, 5_000_000_000))).unwrap();
    let error = failed_start.step(5_004_000_001).unwrap_err();
    assert!(error.message.contains("already registered"));
    failed_start.mock.cleanup();
    let faults = &failed_start.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.faults").unwrap()];
    assert_eq!(faults[0]["at"], serde_json::to_value(TimePoint::new(ROOT, 5_010_000_000)).unwrap());

    let stopped_rx_env = [("sim.faults", serde_json::json!([{ "at_ns": 3_000_000, "fault": "rx_overflow", "target": "radio" }]))];
    let mut stopped_rx = Harness::new("x3xx", &[], &[], &stopped_rx_env, Some((BackPressure::DropOldest, 8)));
    stopped_rx.arm_start(2_000_000_000).unwrap();
    stopped_rx.actions.push(Action::Stop { target: Some(tgt("mock/rx/s0")) });
    stopped_rx.step(2_001_000_000).unwrap();
    stopped_rx.step(2_002_000_000).unwrap();
    stopped_rx.step(2_003_000_000).unwrap();
    let faults = &stopped_rx.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.faults").unwrap()];
    assert_eq!(faults.as_array().unwrap().len(), 1);
    assert_eq!(faults[0]["applied"], false);
    assert_eq!(faults[0]["lost"], 0);
    assert_eq!(
        stopped_rx
            .events
            .drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0))
            .iter()
            .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW)
            .count(),
        0
    );

    let tail_fault_env = [("sim.faults", serde_json::json!([{ "at_ns": 1_500_000, "fault": "rx_overflow", "target": "radio" }]))];
    let mut tail_fault = Harness::new("x3xx", &[], &[], &tail_fault_env, Some((BackPressure::DropOldest, 8)));
    tail_fault.arm_start(2_000_000_000).unwrap();
    tail_fault.actions.push(Action::Stop { target: Some(tgt("mock/rx/s0")) });
    tail_fault.step(2_001_000_000).unwrap();
    tail_fault.step(2_001_500_000).unwrap();
    // RM-16: the `Stop` cuts at its instant, with no tail, so the overrun after it finds no
    // stream running and is recorded unapplied (MR-20; spec 22, VH-3, VH-5).
    let faults = &tail_fault.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.faults").unwrap()];
    assert_eq!(faults[0]["applied"], false);
    assert_eq!(faults[0]["lost"], 0);
    let stats = &tail_fault.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.stats").unwrap()];
    assert_eq!(stats["rx_samples"], 1_000);
    assert_eq!(
        tail_fault
            .events
            .drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0))
            .iter()
            .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW)
            .count(),
        0
    );
}

fn rx_overflows(harness: &Harness) -> Vec<(TimePoint, RxOverflowPayload)> {
    harness
        .events
        .drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0))
        .into_iter()
        .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW)
        .map(|event| (event.time, RxOverflowPayload::from_payload(&event.payload).unwrap()))
        .collect()
}

/// The `faults` rows of one fault kind, as `(applied, lost)`, in the order written (MR-27).
fn fault_rows(harness: &Harness, kind: &str) -> Vec<(serde_json::Value, serde_json::Value)> {
    let faults = &harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.faults").unwrap()];
    faults.as_array().unwrap().iter().filter(|row| row["fault"] == kind).map(|row| (row["applied"].clone(), row["lost"].clone())).collect()
}

fn received(harness: &Harness) -> Vec<BlockHeader> {
    let mut headers = Vec::new();
    while let Some((block, _)) = harness.link.as_ref().unwrap().receive() {
        headers.push(block.header().clone());
    }
    headers
}

#[test]
fn mr_20a_the_overflow_comes_with_the_block_that_carries_it() {
    // RM-17: an overrun at 1 ms on x3xx at 1 MS/s loses 50 000 samples (a 50 ms restart
    // gap): its RX_OVERFLOW waits for the block at 51 000, which carries the loss, as a USRP
    // reports an overflow with the next packet (UR-18, MR-37); its row is written when it
    // fires (MR-20; spec 22, VH-5).
    let env = [("sim.faults", serde_json::json!([{ "at_ns": 1_000_000, "fault": "rx_overflow", "target": "radio" }]))];
    let mut harness = Harness::new("x3xx", &[], &[], &env, Some((BackPressure::DropOldest, 16)));
    harness.arm_start(2_000_000_000).unwrap();
    harness.step(2_002_000_000).unwrap();
    let headers = received(&harness);
    assert_eq!(headers.iter().map(|header| (header.first_sample_time.ticks_in(header.first_sample_time.domain()).unwrap(), header.len)).collect::<Vec<_>>(), [(0, 1_000)]);
    assert!(rx_overflows(&harness).is_empty());
    assert_eq!(fault_rows(&harness, "rx_overflow"), [(serde_json::json!(true), serde_json::json!(50_000))]);

    harness.step(2_060_000_000).unwrap();
    let rx_domain = harness.clocks.sample_clock_records().iter().find(|record| record.stream == rid("mock/rx/s0")).unwrap().domain;
    assert_eq!(rx_overflows(&harness), [(TimePoint::new(rx_domain, 1_000), RxOverflowPayload { cause: RxOverflowCause::Overrun, lost: 50_000 })]);
    assert_eq!(fault_rows(&harness, "rx_overflow"), [(serde_json::json!(true), serde_json::json!(50_000))]);
    let headers = received(&harness);
    assert_eq!((headers[0].first_sample_time.ticks_in(headers[0].first_sample_time.domain()).unwrap(), headers[0].lost), (51_000, Some(50_000)));
    assert_eq!(headers[0].flags, BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED);
    assert!(headers[1..].iter().all(|header| header.lost.is_none()));
}

/// The `rejected` rows, as `(kind, reason, at)`, in the order written (MR-27).
fn rejected_rows(harness: &Harness) -> Vec<(String, String, i64)> {
    let rows = &harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.rejected").unwrap()];
    rows.as_array().unwrap().iter().map(|row| (row["kind"].as_str().unwrap().to_owned(), row["reason"].as_str().unwrap().to_owned(), row["at"]["ticks"].as_i64().unwrap())).collect()
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
        assert_eq!(headers.last().map(|header| header.first_sample_time.ticks_in(header.first_sample_time.domain()).unwrap() + i64::from(header.len)), Some(last), "{at}");
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
        let mut harness = Harness::new("ideal", &[("radio.tx.c0.stream", eq(Scalar::from(0)))], &[], &env, Some((BackPressure::DropOldest, 64)));
        harness.arm_start(0).unwrap();
        let tx = tx_domain(&harness);
        harness.actions.push(tx_action(tx, 500, 1_000, true, LatePolicy::SendAsapAndFlag));
        harness.step(500_000).unwrap();
        let error = harness.step(1_000_000).unwrap_err();
        assert_eq!(error.kind, ezsdr_kernel::module_api::ModuleErrorKind::DeviceLost);
        harness.actions.push(tx_action(tx, 3_000, 1_000, true, LatePolicy::SendAsapAndFlag));
        harness.actions.push(update_action("radio.rx.s0.sample_rate_hz", Value::num(2_000_000.0).unwrap(), UpdateClass::Cold, None));
        harness.actions.push(Action::Stop { target: Some(tgt("mock/rx/s0")) });
        harness.step(2_000_000).unwrap();
        harness.auth.advance_to(TimePoint::new(ROOT, 5_000_000)).unwrap();
        harness.mock.stop(mode).unwrap();
        harness.step(10_000_000).unwrap();
        let bursts = &harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.bursts").unwrap()];
        let records: Vec<ezsdr_kernel::stream::BurstRecord> = serde_json::from_value(bursts.clone()).unwrap();
        assert_eq!(records.iter().map(|record| record.samples).collect::<Vec<_>>(), [500], "{mode:?}");
        let headers = received(&harness);
        assert_eq!(headers.last().map(|header| header.first_sample_time.ticks_in(header.first_sample_time.domain()).unwrap() + i64::from(header.len)), Some(1_000), "{mode:?}");
        let actions: Vec<_> = rejected_rows(&harness).into_iter().map(|(action, reason, _)| (action, reason)).collect();
        assert_eq!(actions, [("tx_burst", LOST), ("update_parameter", LOST), ("stop", LOST)].map(|(a, r)| (a.to_owned(), r.to_owned())), "{mode:?}");
    }
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
    harness.actions.push(update_action("radio.rx.c0.gain_db", Value::num(3.0).unwrap(), UpdateClass::HardwareTimed, None));
    harness.actions.push(Action::Stop { target: Some(tgt("mock")) });
    let error = harness.step(1_999_001).unwrap_err();
    assert_eq!(error.kind, ezsdr_kernel::module_api::ModuleErrorKind::DeviceLost);
    assert!(harness.actions.actions.lock().unwrap().is_empty());
    assert_eq!(fault_rows(&harness, "device_lost"), [(serde_json::json!(true), serde_json::json!(0))]);
    let actions: Vec<_> = rejected_rows(&harness).into_iter().map(|(action, reason, _)| (action, reason)).collect();
    assert_eq!(actions, [("update_parameter", LOST), ("stop", LOST)].map(|(a, r)| (a.to_owned(), r.to_owned())));
    // No event for a refused Action, COMMAND_REJECTED included: the step's error reports the loss.
    assert!(harness.events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).is_empty());
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
    harness.actions.push(update_action("radio.rx.c0.gain_db", Value::num(3.0).unwrap(), UpdateClass::HardwareTimed, None));
    harness.step(2_000_000).unwrap();
    assert!(harness.actions.actions.lock().unwrap().is_empty());
    harness.actions.push(Action::Stop { target: Some(tgt("mock")) });
    harness.auth.advance_to(TimePoint::new(ROOT, 3_000_000)).unwrap();
    harness.mock.stop(StopMode::Orderly).unwrap();
    assert_eq!(rejected_rows(&harness), [("update_parameter".to_owned(), LOST.to_owned(), 2_000_000), ("stop".to_owned(), LOST.to_owned(), 3_000_000)]);
    assert!(harness.events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).is_empty());
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
        assert_eq!(headers.iter().map(|header| (header.first_sample_time.ticks_in(header.first_sample_time.domain()).unwrap(), header.len)).collect::<Vec<_>>(), [(0, 1_000)], "{loss_first}");
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
    let env = [(
        "sim.faults",
        serde_json::json!([
            { "at_ns": 1_000_000, "fault": "rx_sequence_error", "target": "radio" },
            device_lost_at(1_100_000)
        ]),
    )];
    let mut harness = Harness::new("ideal", &[], &[], &env, Some((BackPressure::DropOldest, 64)));
    harness.arm_start(0).unwrap();
    harness.step(1_000_000).unwrap();
    harness.step(1_100_000).unwrap_err();
    harness.auth.advance_to(TimePoint::new(ROOT, 2_000_000)).unwrap();
    harness.mock.stop(StopMode::Orderly).unwrap();
    assert_eq!(fault_rows(&harness, "rx_sequence_error"), [(serde_json::json!(true), serde_json::json!(2_000))]);
    let clocks: Vec<_> = harness.clocks.sample_clock_records().into_iter().map(|record| (record.origin.ticks_in(ROOT).unwrap(), record.ended_at.map(|end| end.ticks_in(ROOT).unwrap()))).collect();
    assert_eq!(clocks, [(0, Some(1_100_000))]);
    assert_eq!(received(&harness).iter().map(|header| (header.first_sample_time.ticks_in(header.first_sample_time.domain()).unwrap(), header.len)).collect::<Vec<_>>(), [(0, 1_000)]);
}

#[test]
fn mr_21_overrun_shape() {
    let env = [("sim.faults", serde_json::json!([{ "at_ns": 1_000_000, "fault": "rx_overflow", "target": "radio" }]))];
    let mut harness = Harness::new("x3xx", &[], &[], &env, Some((BackPressure::DropOldest, 8)));
    harness.arm_start(2_000_000_000).unwrap();
    harness.step(2_052_999_001).unwrap();
    let mut headers = Vec::new();
    while let Some((block, _)) = harness.link.as_ref().unwrap().receive() {
        headers.push(block.header().clone());
    }
    let restarted = headers.iter().find(|header| header.first_sample_time.ticks_in(header.first_sample_time.domain()).unwrap() == 51_000).unwrap();
    assert_eq!(restarted.lost, Some(50_000));
    assert!(restarted.flags.contains(BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED));
    assert!(!restarted.flags.contains(BlockFlags::SEQ_DISCONTINUITY));
    let domain = restarted.first_sample_time.domain();
    assert_eq!(rx_overflows(&harness), [(TimePoint::new(domain, 1_000), RxOverflowPayload { cause: RxOverflowCause::Overrun, lost: 50_000 })]);
    let previous = headers.iter().find(|header| header.first_sample_time.ticks_in(header.first_sample_time.domain()).unwrap() == 0).unwrap();
    assert_eq!(
        restarted.first_sample_time.ticks_in(restarted.first_sample_time.domain()).unwrap()
            - (previous.first_sample_time.ticks_in(previous.first_sample_time.domain()).unwrap() + i64::from(previous.len)),
        50_000
    );

    let env = [(
        "sim.faults",
        serde_json::json!([
            { "at_ns": 1_000_000, "fault": "rx_overflow", "target": "radio" },
            { "at_ns": 10_000_000, "fault": "rx_overflow", "target": "radio" }
        ]),
    )];
    let mut overlap = Harness::new("x3xx", &[], &[], &env, Some((BackPressure::DropOldest, 8)));
    overlap.arm_start(2_000_000_000).unwrap();
    overlap.step(2_062_999_001).unwrap();
    let mut headers = Vec::new();
    while let Some((block, _)) = overlap.link.as_ref().unwrap().receive() {
        headers.push(block.header().clone());
    }
    let restarted = headers.iter().find(|header| header.first_sample_time.ticks_in(header.first_sample_time.domain()).unwrap() == 60_000).unwrap();
    assert_eq!(restarted.lost, Some(59_000));
    assert!(restarted.flags.contains(BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED));
    // One block carries both reports and their whole time jump (MR-20a).
    let overflows: Vec<_> = overlap
        .events
        .drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0))
        .into_iter()
        .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW)
        .map(|event| {
            let payload = RxOverflowPayload::from_payload(&event.payload).unwrap();
            (event.time.ticks_in(event.time.domain()).unwrap(), payload.cause, payload.lost)
        })
        .collect();
    assert_eq!(overflows, [(1_000, RxOverflowCause::Overrun, 59_000)]);
    let faults = overlap.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.faults").unwrap()].clone();
    let rows: Vec<_> = faults.as_array().unwrap().iter().map(|row| (row["applied"].clone(), row["lost"].clone())).collect();
    assert_eq!(rows, [(serde_json::json!(true), serde_json::json!(50_000)), (serde_json::json!(true), serde_json::json!(9_000))]);

    let env = [(
        "sim.faults",
        serde_json::json!([
            { "at_ns": 1_000_000, "fault": "rx_overflow", "target": "radio" },
            { "at_ns": 60_000_000, "fault": "rx_sequence_error", "target": "radio" }
        ]),
    )];
    let mut later = Harness::new("x3xx", &[], &[], &env, Some((BackPressure::DropOldest, 8)));
    later.arm_start(2_000_000_000).unwrap();
    later.step(2_067_000_000).unwrap();
    let events = rx_overflows(&later);
    assert_eq!(
        events.iter().map(|(at, payload)| (at.ticks_in(at.domain()).unwrap(), payload.cause, payload.lost)).collect::<Vec<_>>(),
        [(1_000, RxOverflowCause::Overrun, 50_000), (60_000, RxOverflowCause::Sequence, 2_000),]
    );

    let env = [(
        "sim.faults",
        serde_json::json!([
            { "at_ns": 1_000_000, "fault": "rx_overflow", "target": "radio" },
            { "at_ns": 102_000_000, "fault": "rx_overflow", "target": "radio" }
        ]),
    )];
    let mut blocked = Harness::new("x3xx", &[], &[], &env, Some((BackPressure::Block, 1)));
    blocked.arm_start(2_000_000_000).unwrap();
    for tick in [2_001_999_001, 2_053_999_001, 2_103_999_001, 2_154_999_001] {
        blocked.step(tick).unwrap();
    }
    let events = rx_overflows(&blocked);
    assert_eq!(
        events.iter().map(|(at, payload)| (at.ticks_in(at.domain()).unwrap(), payload.cause, payload.lost)).collect::<Vec<_>>(),
        [
            (1_000, RxOverflowCause::Overrun, 50_000),
            (51_000, RxOverflowCause::Overrun, 50_000),
            (101_000, RxOverflowCause::Overrun, 50_000),
            (151_000, RxOverflowCause::Overrun, 1_000),
            (152_000, RxOverflowCause::Overrun, 50_000),
        ]
    );
    assert_eq!(fault_rows(&blocked, "rx_overflow"), [(serde_json::json!(true), serde_json::json!(50_000)), (serde_json::json!(true), serde_json::json!(1_000)),]);

    // The row is what the overrun injected, `k_g - k_f`, written when it fires and never
    // revised: a `Stop` handled at 20 ms, before `k_g` and in the step that applies the fault, leaves
    // it 50 000 (VH-5).
    let env = [("sim.faults", serde_json::json!([{ "at_ns": 1_000_000, "fault": "rx_overflow", "target": "radio" }]))];
    let mut cut = Harness::new("x3xx", &[], &[], &env, Some((BackPressure::DropOldest, 8)));
    cut.arm_start(2_000_000_000).unwrap();
    cut.step(2_000_000_000).unwrap();
    cut.actions.push(Action::Stop { target: Some(tgt("mock/rx/s0")) });
    cut.step(2_020_000_000).unwrap();
    cut.step(2_100_000_000).unwrap();
    assert_eq!(fault_rows(&cut, "rx_overflow"), [(serde_json::json!(true), serde_json::json!(50_000))]);
    assert!(rx_overflows(&cut).is_empty(), "no block carries the loss");
}

#[test]
fn mr_20_a_fault_between_receive_clocks_is_not_applied() {
    // A `Stop` at 1 ms ends the clock, `start_rx` at 1.01 ms begins the next; both faults fall in the
    // gap, where no receive stream runs, so neither touches the new clock.
    for fault in ["rx_sequence_error", "rx_overflow"] {
        let env = [("sim.faults", serde_json::json!([{ "at_ns": 1_005_000, "fault": fault, "target": "radio" }]))];
        let mut harness = Harness::new("ideal", &[("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(3_000_000.0).unwrap()))], &[], &env, Some((BackPressure::DropOldest, 64)));
        harness.arm_start(0).unwrap();
        harness.step(0).unwrap();
        harness.actions.push(Action::Stop { target: Some(tgt("mock/rx/s0")) });
        harness.step(1_000_000).unwrap();
        harness.step(1_005_000).unwrap();
        let faults = harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.faults").unwrap()].clone();
        assert_eq!(faults.as_array().unwrap().len(), 1, "{fault}");
        assert_eq!(faults[0]["applied"], false, "{fault}");
        assert_eq!(faults[0]["lost"], 0, "{fault}");
        harness.actions.push(start_rx("mock/rx/s0", None));
        harness.step(1_010_000).unwrap();
        harness.step(20_000_000).unwrap();
        let records = harness.clocks.sample_clock_records();
        assert_eq!(records.len(), 2, "{fault}: the old clock and the new one");
        let domain = records.last().unwrap().domain;
        assert_eq!(
            harness
                .events
                .drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0))
                .iter()
                .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW)
                .count(),
            0,
            "{fault}"
        );
        let mut first = None;
        while let Some((block, _)) = harness.link.as_ref().unwrap().receive() {
            if block.header().first_sample_time.domain() == domain && first.is_none() {
                first = Some(block.header().clone());
            }
        }
        let first = first.expect("a block of the new clock");
        assert_eq!(first.first_sample_time.ticks_in(domain).unwrap(), 0, "{fault}");
        assert_eq!(first.lost, None, "{fault}");
        assert_eq!(first.flags, BlockFlags::NONE, "{fault}");
    }
}

#[test]
fn mr_25_a_stop_cuts_at_its_instant() {
    // RM-16, MR-25 (spec 22, VH-3): a `Stop` of the stream or of the device, and `stop`,
    // end the receive stream at the first sample at or after the instant MockRadio handles
    // them: every sample before it is delivered, none at or after it, and the SampleClock
    // ends there — on `ideal` at 1 MS/s, and on `x3xx` at 390.625 kS/s, whose 2 000-sample
    // blocks last 5.12 ms (2 560 root ticks a sample).
    for (profile, rate, period, t0) in [("ideal", 1_000_000.0, 1_000, 0), ("x3xx", 390_625.0, 2_560, 2_000_000_000)] {
        for how in ["mock/rx/s0", "mock", "stop"] {
            let mut h = Harness::new(profile, &[("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(rate).unwrap()))], &[], &[], Some((BackPressure::DropOldest, 64)));
            h.arm_start(t0).unwrap();
            let at = t0 + 7_777_777;
            h.step(at).unwrap();
            if how == "stop" {
                h.mock.stop(StopMode::Orderly).unwrap();
            } else {
                h.actions.push(Action::Stop { target: Some(tgt(how)) });
                h.step(at).unwrap();
            }
            h.step(at + 50_000_000).unwrap();
            let cut = (7_777_777 + period - 1) / period;
            let blocks = received(&h);
            let end = blocks.last().map(|header| header.first_sample_time.ticks_in(header.first_sample_time.domain()).unwrap() + i64::from(header.len));
            assert_eq!(end, Some(cut), "{profile}, {how}");
            let clocks: Vec<_> = h
                .clocks
                .sample_clock_records()
                .into_iter()
                .filter(|record| record.stream == rid("mock/rx/s0"))
                .map(|record| (record.origin.ticks_in(ROOT).unwrap(), record.ended_at.map(|end| end.ticks_in(ROOT).unwrap())))
                .collect();
            assert_eq!(clocks, [(t0, Some(t0 + cut * period))], "{profile}, {how}");
        }
    }
}

#[test]
fn mr_29_start_rx_is_the_one_command() {
    // MR-29, RM-12: after a `Stop` of `mock/rx`, `start_rx` on `mock/tx`, on `mock` or with a
    // param is refused; on `mock/rx` it starts the stream again on a new clock, at its instant
    // on `ideal`, and a second one on a running stream changes nothing (RM-21, RM-25).
    let mut h = Harness::new("ideal", &[], &[], &[], Some((BackPressure::DropOldest, 64)));
    h.arm_start(0).unwrap();
    h.actions.push(Action::Stop { target: Some(tgt("mock/rx/s0")) });
    h.step(1_000_000).unwrap();
    let start = |target: &str, params: BTreeMap<Key, Value>| Action::Command { target: tgt(target), verb: Ident::parse("start_rx").unwrap(), params, at: None };
    h.actions.push(start("mock/tx/s0", BTreeMap::new()));
    h.actions.push(start("mock", BTreeMap::new()));
    h.actions.push(start("mock/rx/s0", BTreeMap::from([(key("radio.now"), Value::from(true))])));
    h.step(2_000_000).unwrap();
    let rejected = rejected_rows(&h);
    assert_eq!(rejected.iter().map(|row| row.0.as_str()).collect::<Vec<_>>(), ["command"; 3]);
    assert_eq!(
        h.events
            .drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0))
            .iter()
            .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED)
            .count(),
        3
    );
    h.actions.push(start("mock/rx/s0", BTreeMap::new()));
    h.step(3_000_000).unwrap();
    h.actions.push(start("mock/rx/s0", BTreeMap::new()));
    h.step(4_000_000).unwrap();
    h.step(10_000_000).unwrap();
    let clocks: Vec<_> = h.clocks.sample_clock_records().into_iter().map(|record| (record.origin.ticks_in(ROOT).unwrap(), record.ended_at.map(|end| end.ticks_in(ROOT).unwrap()))).collect();
    assert_eq!(clocks, [(0, Some(1_000_000)), (3_000_000, None)]);
    let first_new = received(&h).into_iter().find(|header| header.first_sample_time.domain() != h.clocks.sample_clock_records()[0].domain).unwrap();
    assert_eq!((first_new.first_sample_time.ticks_in(first_new.first_sample_time.domain()).unwrap(), first_new.flags), (0, BlockFlags::NONE));
}

#[test]
fn mr_29_start_rx_takes_effect_at_its_at() {
    // MR-29: a `start_rx` whose `at` is after its receipt restarts the stream there, not at
    // the receipt: the new clock's origin is `at`'s lattice instant, and it is never late.
    let mut h = Harness::new("ideal", &[], &[], &[], Some((BackPressure::DropOldest, 64)));
    h.arm_start(0).unwrap();
    h.actions.push(Action::Stop { target: Some(tgt("mock/rx/s0")) });
    h.step(1_000_000).unwrap();
    // Handled at 2 ms, asking for 12 ms.
    h.actions.push(start_rx("mock/rx/s0", Some(12_000_000)));
    h.step(2_000_000).unwrap();
    h.step(13_999_001).unwrap();
    let clocks: Vec<_> = h.clocks.sample_clock_records().into_iter().map(|record| (record.origin.ticks_in(ROOT).unwrap(), record.ended_at.map(|end| end.ticks_in(ROOT).unwrap()))).collect();
    assert_eq!(clocks, [(0, Some(1_000_000)), (12_000_000, None)]);
    let late =
        h.events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::LATE_COMMAND).count();
    assert_eq!(late, 0);
}

#[test]
fn mr_29_a_receive_link_never_begins_before_t0() {
    // MR-29: `start_rx` never begins a stream with a receive link before T0, whatever its `at`
    // or its receipt: the new segment, and so its clock, begins at T0.
    let mut h = Harness::new("ideal", &[], &[], &[], Some((BackPressure::DropOldest, 64)));
    h.arm_start(10_000_000).unwrap();
    // A `Stop` and a `start_rx` asking for T0 − 1 ms both reach the Mock before T0.
    h.actions.push(Action::Stop { target: Some(tgt("mock/rx/s0")) });
    h.step(8_000_000).unwrap();
    h.actions.push(start_rx("mock/rx/s0", Some(9_000_000)));
    h.step(8_500_000).unwrap();
    h.step(14_000_000).unwrap();
    let clocks: Vec<_> = h.clocks.sample_clock_records().into_iter().map(|record| (record.origin.ticks_in(ROOT).unwrap(), record.ended_at.map(|end| end.ticks_in(ROOT).unwrap()))).collect();
    assert_eq!(clocks, [(10_000_000, None)], "the restarted stream's clock begins at T0");
    let blocks = received(&h);
    let first = blocks.first().expect("the restarted stream published no block");
    assert_eq!(first.first_sample_time.ticks_in(first.first_sample_time.domain()).unwrap(), 0);
}

#[test]
fn mr_29_start_rx_is_never_late() {
    // MR-29: a `start_rx` whose `at` has passed is not late — no `LATE_COMMAND` — and the
    // stream restarts at the receipt, not at the past `at` nor a lead after it.
    let mut h = Harness::new("ideal", &[], &[], &[], Some((BackPressure::DropOldest, 64)));
    h.arm_start(0).unwrap();
    h.actions.push(Action::Stop { target: Some(tgt("mock/rx/s0")) });
    h.step(1_000_000).unwrap();
    h.actions.push(start_rx("mock/rx/s0", Some(1_000_000)));
    h.step(3_000_000).unwrap();
    h.step(9_000_000).unwrap();
    let clocks: Vec<_> = h.clocks.sample_clock_records().into_iter().map(|record| (record.origin.ticks_in(ROOT).unwrap(), record.ended_at.map(|end| end.ticks_in(ROOT).unwrap()))).collect();
    assert_eq!(clocks, [(0, Some(1_000_000)), (3_000_000, None)], "the restart is at the receipt, not at the past `at`");
    let late =
        h.events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::LATE_COMMAND).count();
    assert_eq!(late, 0, "a `start_rx` is never late");
}

#[test]
fn mr_22_sequence_error_shape() {
    let env = [("sim.faults", serde_json::json!([{ "at_ns": 1_000_000, "fault": "rx_sequence_error", "target": "radio" }]))];
    let mut harness = Harness::new("ideal", &[], &[], &env, Some((BackPressure::DropOldest, 8)));
    harness.arm_start(0).unwrap();
    harness.step(4_999_001).unwrap();
    let mut headers = Vec::new();
    while let Some((block, _)) = harness.link.as_ref().unwrap().receive() {
        headers.push(block.header().clone());
    }
    let restarted = headers.iter().find(|header| header.first_sample_time.ticks_in(header.first_sample_time.domain()).unwrap() == 3000).unwrap();
    assert_eq!(restarted.lost, Some(2_000));
    assert!(restarted.flags.contains(BlockFlags::GAP_BEFORE | BlockFlags::SEQ_DISCONTINUITY));
    assert!(!restarted.flags.contains(BlockFlags::RESTARTED));
    let domain = restarted.first_sample_time.domain();
    assert_eq!(rx_overflows(&harness), [(TimePoint::new(domain, 1_000), RxOverflowPayload { cause: RxOverflowCause::Sequence, lost: 2_000 })]);
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
    let mut orderly = Harness::new("x3xx", &[], &[], &[], Some((BackPressure::DropOldest, 16)));
    orderly.arm_start(2_000_000_000).unwrap();
    orderly.step(2_001_999_000).unwrap();
    let stop = 2_001_999_000;
    orderly.mock.stop(StopMode::Orderly).unwrap();
    orderly.step(stop + 999_001).unwrap();
    let mut blocks = Vec::new();
    while let Some((block, _)) = orderly.link.as_ref().unwrap().receive() {
        blocks.push(block);
    }
    let last = blocks.last().unwrap().header();
    assert_eq!(2_000_000_000 + (last.first_sample_time.ticks_in(last.first_sample_time.domain()).unwrap() + i64::from(last.len) - 1) * 1_000, stop - 1_000);
    let ended = orderly.clocks.sample_clock_records()[0].ended_at;
    assert_eq!(ended, Some(TimePoint::new(ROOT, stop)));

    let mut abort = Harness::new("x3xx", &[], &[], &[], Some((BackPressure::DropOldest, 16)));
    abort.arm_start(2_000_000_000).unwrap();
    abort.step(2_001_999_001).unwrap();
    abort.mock.stop(StopMode::Abort).unwrap();
    abort.step(2_010_000_000).unwrap();
    let mut after_abort = Vec::new();
    while let Some((block, _)) = abort.link.as_ref().unwrap().receive() {
        after_abort.push(block);
    }
    assert_eq!(after_abort.len(), 1);
    assert_eq!(after_abort[0].header().first_sample_time.ticks_in(after_abort[0].header().first_sample_time.domain()).unwrap(), 0);
}

#[test]
fn mr_25_a_stopped_stream_keeps_its_end() {
    // A receive stream's end moves only earlier (RM-16, MR-25; spec 21, VG-1; issue #46).
    // A `Stop` of `mock/rx` at T0 + 1 ms ends the stream and its clock at sample 1 000; a
    // later stop at T0 + 5 ms keeps that end (spec 22, VH-1).
    for (profile, t0, end) in [("ideal", 0, 1_000), ("x3xx", 2_000_000_000, 1_000)] {
        for later in ["stop(Orderly)", "mock/rx/s0", "mock"] {
            let mut harness = Harness::new(profile, &[], &[], &[], Some((BackPressure::DropOldest, 64)));
            harness.arm_start(t0).unwrap();
            harness.actions.push(Action::Stop { target: Some(tgt("mock/rx/s0")) });
            harness.step(t0 + 1_000_000).unwrap();
            harness.step(t0 + 5_000_000).unwrap();
            if later == "stop(Orderly)" {
                harness.mock.stop(StopMode::Orderly).unwrap();
            } else {
                harness.actions.push(Action::Stop { target: Some(tgt(later)) });
                harness.step(t0 + 5_000_000).unwrap();
            }
            harness.step(t0 + 10_000_000).unwrap();
            let last = received(&harness).pop().unwrap();
            assert_eq!(last.first_sample_time.ticks_in(last.first_sample_time.domain()).unwrap() + i64::from(last.len), end, "{profile}, then {later}");
            let clocks: Vec<_> = harness
                .clocks
                .sample_clock_records()
                .into_iter()
                .filter(|record| record.stream == rid("mock/rx/s0"))
                .map(|record| (record.origin.ticks_in(ROOT).unwrap(), record.ended_at.map(|end| end.ticks_in(ROOT).unwrap())))
                .collect();
            assert_eq!(clocks, [(t0, Some(t0 + 1_000_000))], "{profile}, then {later}");
        }
    }

    // An abort after the `Stop` keeps its end too.
    let mut harness = Harness::new("x3xx", &[], &[], &[], Some((BackPressure::DropOldest, 64)));
    harness.arm_start(2_000_000_000).unwrap();
    harness.actions.push(Action::Stop { target: Some(tgt("mock/rx/s0")) });
    harness.step(2_001_000_000).unwrap();
    harness.step(2_001_500_000).unwrap();
    harness.mock.stop(StopMode::Abort).unwrap();
    harness.step(2_010_000_000).unwrap();
    assert_eq!(received(&harness).iter().map(|header| (header.first_sample_time.ticks_in(header.first_sample_time.domain()).unwrap(), header.len)).collect::<Vec<_>>(), [(0, 1_000)]);
}

#[test]
fn mr_25_rx_stop_leaves_transmit_running_and_stop_records_an_unstarted_burst() {
    let constraints = [("radio.rx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.stream", eq(Scalar::from(0)))];
    let mut harness = Harness::new("x3xx", &constraints, &[], &[], Some((BackPressure::DropOldest, 16)));
    harness.arm_start(2_000_000_000).unwrap();
    let domain = tx_domain(&harness);
    harness.actions.push(Action::Stop { target: Some(tgt("mock/rx/s0")) });
    harness.step(2_001_000_000).unwrap();
    harness.step(2_002_000_000).unwrap();
    while harness.link.as_ref().unwrap().receive().is_some() {}

    let now = 2_002_000_000;
    let now_tx = harness.clocks.convert(TimePoint::new(ROOT, now), domain).unwrap().floor();
    harness.actions.push(tx_action(domain, now_tx.ticks_in(domain).unwrap() + 4_000, 1_000, false, LatePolicy::SendAsapAndFlag));
    harness.step(now).unwrap();
    harness.step(now + 6_000_000).unwrap();
    let stats = &harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.stats").unwrap()];
    assert!(stats["tx_blocks"].as_u64().unwrap() > 0);
    let records = &harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.bursts").unwrap()];
    let records: Vec<ezsdr_kernel::stream::BurstRecord> = serde_json::from_value(records.clone()).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].samples, 1_000);

    let mut unstarted = tx_harness("ideal");
    unstarted.arm_start(0).unwrap();
    let domain = tx_domain(&unstarted);
    unstarted.actions.push(tx_action(domain, 0, 2_000, false, LatePolicy::SendAsapAndFlag));
    unstarted.step(0).unwrap();
    unstarted.mock.stop(StopMode::Abort).unwrap();
    let rejected = &unstarted.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.rejected").unwrap()];
    assert_eq!(rejected.as_array().unwrap().len(), 1);
    assert!(rejected[0]["reason"].as_str().unwrap().contains("cancelled by stop"));
    assert_eq!(unstarted.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.bursts").unwrap()].as_array().unwrap().len(), 0);
    assert_eq!(
        unstarted
            .events
            .drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0))
            .iter()
            .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED)
            .count(),
        0
    );
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
    harness.actions.push(Action::SetTimer { target: tgt("mock"), at: ezsdr_kernel::time::AbsoluteDeadline::new(TimePoint::new(ROOT, 1)), token: 3 });
    harness.actions.push(Action::Command { target: tgt("mock"), verb: Ident::parse("start_repeat").unwrap(), params: BTreeMap::new(), at: None });
    // MR-18: a radio key has no update class, so the Kernel refuses its Run-time update before dispatch; the Provider's own
    // guard refuses one that reaches it directly, as this harness does.
    harness.actions.push(update_action("radio.rx.c0.frequency_hz", Value::num(2.0e9).unwrap(), UpdateClass::Cold, None));
    harness.step(1).unwrap();
    let rejected = &harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.rejected").unwrap()];
    assert_eq!(rejected.as_array().unwrap().len(), 3);
    assert_eq!(rejected[0]["kind"], "set_timer");
    assert_eq!(rejected[1]["kind"], "command");
    assert_eq!(rejected[2]["kind"], "update_parameter");
    assert_eq!(rejected[2]["reason"], "MR-18: target or configuration key is not updateable");
    let events = harness.events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
    let rejected_events: Vec<_> = events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).collect();
    assert_eq!(rejected_events.len(), 3);
    assert!(rejected_events.iter().all(|event| event.source == node_src("mock") && event.time == TimePoint::new(ROOT, 1)));
    // MA-14: each refusal names the Action it answers, by the id it was dispatched with.
    let answered: Vec<_> = rejected_events.iter().map(|event| event.payload["action"].clone()).collect();
    assert_eq!(answered, [serde_json::json!(1), serde_json::json!(2), serde_json::json!(3)]);
}

#[test]
fn mr_10_events_carry_the_source_the_context_gave() {
    // KC-8: the Mock emits under the root its context names, which need not be its own
    // id; it formats no source of its own.
    SOURCE_ROOT.with(|root| root.set("given"));
    let mut harness = Harness::new("ideal", &[], &[], &[], None);
    SOURCE_ROOT.with(|root| root.set("mock"));
    harness.arm_start(0).unwrap();
    harness.actions.push(Action::Command { target: tgt("mock"), verb: Ident::parse("start_repeat").unwrap(), params: BTreeMap::new(), at: None });
    harness.step(1).unwrap();
    let events = harness.events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0));
    let sources: Vec<_> = events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).map(|event| event.source.clone()).collect();
    assert_eq!(sources, [node_src("given")]);
}

#[test]
fn mr_30_two_mocks_one_seed_identical_output() {
    let selector = [("block_len_jitter", Value::from(true)), ("rx_test_pattern", Value::from("ramp".to_owned()))];
    let mut left = Harness::new("ideal", &[], &selector, &[("sim.seed", serde_json::json!(7))], Some((BackPressure::DropOldest, 32)));
    let mut right = Harness::new("ideal", &[], &selector, &[("sim.seed", serde_json::json!(7))], Some((BackPressure::DropOldest, 32)));
    left.arm_start(0).unwrap();
    right.arm_start(0).unwrap();
    left.step(25_000_000).unwrap();
    right.step(25_000_000).unwrap();
    let a: Vec<_> = std::iter::from_fn(|| left.link.as_ref().unwrap().receive().map(|(b, _)| b)).map(|b| b.header().clone()).collect();
    let b: Vec<_> = std::iter::from_fn(|| right.link.as_ref().unwrap().receive().map(|(b, _)| b)).map(|b| b.header().clone()).collect();
    assert_eq!(a, b);
    assert_eq!(left.mock.instance().sections, right.mock.instance().sections);
    assert_eq!(
        left.events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)),
        right.events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0))
    );
}

#[test]
fn mr_37_the_overflow_travels_the_hot_path() {
    // A one-slot ring: the hot path counts every overflow but queues one body and
    // reports the other in EVENTS_DROPPED; the control path never drops, so a Mock that
    // still used it would deliver both (RS-33, RS-34, RS-35).
    RING_DEPTH.with(|depth| depth.set(1));
    let env = [(
        "sim.faults",
        serde_json::json!([
            { "at_ns": 1_000_000, "fault": "rx_overflow", "target": "radio" },
            { "at_ns": 60_000_000, "fault": "rx_sequence_error", "target": "radio" }
        ]),
    )];
    let mut harness = Harness::new("x3xx", &[], &[], &env, Some((BackPressure::DropOldest, 8)));
    RING_DEPTH.with(|depth| depth.set(4096));
    harness.arm_start(2_000_000_000).unwrap();
    for ms in 1..=66 {
        harness.step(2_000_000_000 + ms * 1_000_000).unwrap();
    }
    let faults = &harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.faults").unwrap()];
    assert_eq!(faults.as_array().unwrap().iter().filter(|fault| fault["applied"] == true).count(), 2, "{faults}");
    let counted = harness.events.counters().into_iter().find(|row| row.source == node_src("mock/rx/s0") && row.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW).unwrap().count;
    assert_eq!(counted, 2);
    let events = harness.events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
    let delivered: Vec<_> = events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW).collect();
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].severity, ezsdr_kernel::event::Severity::Warning, "RM-10's severity");
    assert_eq!(delivered[0].source, node_src("mock/rx/s0"));
    assert_eq!(delivered[0].payload.as_array().map(Vec::len), Some(9), "RM-24's bytes: {}", delivered[0].payload);
    assert_eq!(RxOverflowPayload::from_payload(&delivered[0].payload).unwrap(), RxOverflowPayload { cause: RxOverflowCause::Overrun, lost: 50_000 });
    let dropped: Vec<_> = events.iter().filter(|event| event.kind.as_str() == ezsdr_kernel::event::EventKind::EVENTS_DROPPED).collect();
    assert_eq!(dropped.len(), 1);
    assert_eq!(dropped[0].payload, serde_json::json!({ "kind": ezsdr_radio::kinds::RX_OVERFLOW, "count": 1 }));
}

// ---------------------------------------------------------------- design 38: the timed floor

/// An `x3xx` Mock armed and started at `T0`, its instance-wide state alone.
const T0: i64 = 2_000_000_000;

#[test]
fn mr_20_x3xx_loss_cuts_transmit_at_its_instant() {
    let env = [("sim.faults", serde_json::json!([device_lost_at(5_000_000)]))];
    let mut harness = Harness::new("x3xx", &[("radio.tx.c0.stream", eq(Scalar::from(0)))], &[], &env, None);
    harness.arm_start(T0).unwrap();
    let domain = tx_domain(&harness);
    harness.actions.push(tx_action(domain, 2_002_000, 2_000, true, LatePolicy::DropAndFlag));
    harness.step(T0).unwrap();
    harness.step(T0 + 4_000_000).unwrap();
    let error = harness.step(T0 + 5_000_000).unwrap_err();
    assert_eq!(error.kind, ezsdr_kernel::module_api::ModuleErrorKind::DeviceLost);
    let records: Vec<ezsdr_kernel::stream::BurstRecord> = serde_json::from_value(harness.mock.instance().sections[&Namespace::parse("ezsdr.radio.mock.bursts").unwrap()].clone()).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!((records[0].samples, records[0].end), (3_000, BurstEnd::Stop));
}
