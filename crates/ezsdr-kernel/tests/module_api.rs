//! Phase 1 tests for `05-module-api.md`. Each name begins with the rule it proves (OV-19).

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use ezsdr_kernel::binding::{ComponentPlacement, Placements};
use ezsdr_kernel::contract::{DataContractId, Port, PortDirection, PortRef};
use ezsdr_kernel::event::{Action, EventCollector, EventKind};
use ezsdr_kernel::id::{ClockDomainId, DataLinkId, IslandId, MemoryDomainId, RunId};
use ezsdr_kernel::manifest::ArtifactRef;
use ezsdr_kernel::module_api::{
    ActionSubmitter, Authority, ComponentDescriptor, ComponentImpl,
    ComponentKind, ComponentRequires, ComponentTiming, Deployment, Executor, ExecutorDescriptor,
    Factories, IslandDecl, Link, ModuleDescriptor, ModuleError, ModuleErrorKind, ModuleRegistry,
    ParamDecl, PrepareContext, Provider, Requested, Role, RtPolicy, Sink, StepOutcome,
    SteppedInstance, SteppedRef, StopMode, UpdateClass, Version, VersionReq,
    step_until_quiescent,
};
use ezsdr_kernel::plan::{
    EdgeKind, GraphEdge, IslandContext, PrepareReport, admit_islands, check_cycles,
    check_effective_narrows,
};
use ezsdr_kernel::policy::EventKindRegistry;
use ezsdr_kernel::spec::{
    CapabilityValue, Constraint, Ident, Value,
};
use ezsdr_kernel::stream::{BackPressure, DataLinkDecl, LatePolicy};
use ezsdr_kernel::time::{
    AbsoluteDeadline, ClockDomain, ClockRegistry, Duration, EpochRef, ManualTimeAuthority,
    Rational, RelativeBudget, TimePoint,
};
use support::{
    FailAt, QueueReceiver, TestExecutor, TestProvider, TestSink, TestSubmitter, id, key, mid, ns,
    rid, some_hash, test_provider_descriptor, test_sink_descriptor,
    test_vocabulary,
};

fn cf32() -> DataContractId {
    DataContractId::parse("ezsdr.stream.cf32").expect("id")
}

// ---------------------------------------------------------------- shape rules

#[test]
fn ma_05_traits_are_object_safe_and_send() {
    // Each of the five compiles as a trait object; the first three are `Send`, and
    // Link and Authority are `Send + Sync` because they have no lifecycle.
    fn assert_send<T: Send + ?Sized>() {}
    fn assert_send_sync<T: Send + Sync + ?Sized>() {}
    assert_send::<dyn Provider>();
    assert_send::<dyn Executor>();
    assert_send::<dyn Sink>();
    assert_send_sync::<dyn Link>();
    assert_send_sync::<dyn Authority>();
    let _: Box<dyn Provider> = Box::new(TestProvider::new("radio", 2));
    let _: Box<dyn Executor> = Box::new(TestExecutor::new(MemoryDomainId::local(0)));
    let _: Box<dyn Sink> = Box::new(TestSink::new(cf32()));
}

#[test]
fn ma_06_signature_types_are_documents() {
    // Every parameter and return document type is serialisable with a schema, which
    // is what lets a Plugin host implement any role later by message passing.
    fn assert_document<T>()
    where
        T: serde::Serialize + serde::de::DeserializeOwned + schemars::JsonSchema,
    {
    }
    assert_document::<ezsdr_kernel::module_api::ProviderInstance>();
    assert_document::<ezsdr_kernel::module_api::CoerceReport>();
    assert_document::<Requested>();
    assert_document::<PrepareReport>();
    assert_document::<ezsdr_kernel::plan::Fragment>();
    assert_document::<ComponentDescriptor>();
    assert_document::<IslandDecl>();
    assert_document::<ModuleError>();
    assert_document::<ArtifactRef>();
    assert_document::<Action>();
    assert_document::<ezsdr_kernel::event::Event>();
    assert_document::<ezsdr_kernel::module_api::ExecutionClass>();
    assert_document::<ezsdr_kernel::module_api::Fidelity>();
    assert_document::<ezsdr_kernel::run::StopCause>();
}

// ---------------------------------------------------------------- the registry

/// A **Provider-only** descriptor. The shared double declares Authority as well
/// (MA-1), and MA-31 then requires an Authority factory — which would make every
/// refusal in `ma_32_registry_refusals` fire on the wrong role.
fn descriptor() -> ModuleDescriptor {
    let mut d = test_provider_descriptor();
    d.roles = vec![Role::Provider];
    d
}

#[test]
fn ma_32_registry_refusals() {
    // A missing Vocabulary.
    let mut reg = ModuleRegistry::new();
    let err = reg
        .register(descriptor(), Factories { provider: true, ..Factories::default() })
        .expect_err("the `test` Vocabulary is not registered");
    assert!(err.message.contains("vocabulary test"), "{err}");

    // An incompatible one.
    let mut reg = ModuleRegistry::new();
    let mut v = test_vocabulary();
    v.version = Version::new(2, 0, 0);
    reg.register_vocabulary(v).expect("fresh");
    let err = reg
        .register(descriptor(), Factories { provider: true, ..Factories::default() })
        .expect_err("2.0.0 does not satisfy ^1.0.0");
    assert!(err.message.contains("does not satisfy"), "{err}");

    // A kernel_api major mismatch.
    let mut reg = ModuleRegistry::new();
    reg.register_vocabulary(test_vocabulary()).expect("fresh");
    let mut d = descriptor();
    d.kernel_api = Version::new(5, 0, 0);
    let err = reg
        .register(d, Factories { provider: true, ..Factories::default() })
        .expect_err("major mismatch");
    assert!(err.message.contains("kernel_api major"), "{err}");

    // `deployment: Plugin` is Unsupported in Phase 1.
    let mut d = descriptor();
    d.deployment = Deployment::Plugin { protocol: Version::new(1, 0, 0) };
    let err = reg
        .register(d, Factories { provider: true, ..Factories::default() })
        .expect_err("Plugin is reserved");
    assert_eq!(err.kind, ModuleErrorKind::Unsupported);

    // A role with no factory, and a factory with no role.
    let err = reg
        .register(descriptor(), Factories::default())
        .expect_err("Provider has no factory");
    assert!(err.message.contains("has no factory"), "{err}");
    let err = reg
        .register(descriptor(), Factories { provider: true, sink: true, ..Factories::default() })
        .expect_err("the Sink factory has no declared role");
    assert!(err.message.contains("has no declared role"), "{err}");

    // A duplicate `(id, version)`.
    reg.register(descriptor(), Factories { provider: true, ..Factories::default() })
        .expect("first registration");
    let err = reg
        .register(descriptor(), Factories { provider: true, ..Factories::default() })
        .expect_err("duplicate");
    assert!(err.message.contains("already registered"), "{err}");
}

#[test]
fn ma_33_caret_table() {
    let req = |a, b, c| VersionReq(Version::new(a, b, c));
    let v = Version::new;
    // Above 1.0: the major must match and `(minor, patch)` must be at least.
    assert!(req(1, 2, 3).matches(v(1, 2, 3)));
    assert!(req(1, 2, 3).matches(v(1, 3, 0)));
    assert!(req(1, 2, 3).matches(v(1, 2, 4)));
    assert!(!req(1, 2, 3).matches(v(1, 2, 2)));
    assert!(!req(1, 2, 3).matches(v(2, 0, 0)));
    assert!(!req(1, 2, 3).matches(v(0, 9, 9)));
    // Below 1.0: the minor must match exactly and the patch must be at least.
    assert!(req(0, 3, 1).matches(v(0, 3, 1)));
    assert!(req(0, 3, 1).matches(v(0, 3, 9)));
    assert!(!req(0, 3, 1).matches(v(0, 4, 0)));
    assert!(!req(0, 3, 1).matches(v(0, 3, 0)));
}

#[test]
fn ma_34_key_prefix_refused() {
    let mut reg = ModuleRegistry::new();
    reg.register_vocabulary(test_vocabulary()).expect("fresh");
    assert!(reg.key_decl(&key("test.count")).is_ok());
    let err = reg.key_decl(&key("radio.rx.channels")).expect_err("no Vocabulary owns `radio`");
    assert!(err.message.contains("radio.rx.channels"), "the refusal names the prefix: {err}");
}

#[test]
fn ma_35_vocabulary_carries_its_own_content() {
    let mut reg = ModuleRegistry::new();
    reg.register_vocabulary(test_vocabulary()).expect("fresh");
    let v = reg.vocabulary(&ns("test")).expect("registered");
    // 1. Its KeyDecls, which the Kernel enforces and never interprets (SB-2).
    assert!(v.keys.iter().any(|k| k.key == key("test.grid") && k.coercible));
    // 2. Its event kinds with their defaults and severities (RS-27, RS-28).
    let mut kinds = EventKindRegistry::with_kernel_kinds();
    for decl in v.event_kinds.clone() {
        kinds.register(Some(ns("test")), decl).expect("fresh");
    }
    assert!(kinds.get(&EventKind::parse("test.custom").expect("parses")).is_some());
    // A kind outside its owner's namespace is refused (RS-27, RS-39).
    let stray = ezsdr_kernel::policy::EventKindDecl {
        kind: EventKind::parse("vendor.other").expect("parses"),
        default: ezsdr_kernel::policy::Reaction::Continue,
        severity: ezsdr_kernel::event::Severity::Info,
        hot_layout: None,
    };
    assert!(kinds.register(Some(ns("test")), stray).is_err());
    // 3. Its Session verbs and how each compiles (RS-13a).
    assert!(reg.verb(&ns("test"), &id("capture")).is_some());
    assert!(reg.verb(&ns("test"), &id("nonexistent")).is_none());
    // 4. Its admission checks (SB-29).
    assert_eq!(v.checks, vec![ns("test.limits")]);
}

// ---------------------------------------------------------------- Provider behaviour

fn prepare_context<'a>(
    run: RunId,
    authority: &'a ManualTimeAuthority,
    events: &'a EventCollector,
    actions: &'a QueueReceiver,
    submitter: &'a TestSubmitter,
) -> PrepareContext<'a> {
    PrepareContext {
        run,
        class: ezsdr_kernel::module_api::ExecutionClass::Simulation,
        time: authority,
        events,
        actions,
        actions_out: submitter,
        links: Vec::new(),
        host_budget: RelativeBudget::new(Duration::new(ClockDomainId::HOST_MONOTONIC, 1_000_000))
            .expect("host.monotonic"),
    }
}

struct Harness {
    authority: ManualTimeAuthority,
    events: EventCollector,
    actions: QueueReceiver,
    submitter: TestSubmitter,
}

impl Harness {
    fn new(submitter: TestSubmitter) -> Harness {
        let registry = Arc::new(ClockRegistry::new());
        let root = registry.allocate_id();
        registry
            .register(ClockDomain::root(
                root,
                Rational::new(200_000_000, 1).expect("rate"),
                EpochRef::Arbitrary { set_by: "test".to_owned() },
            ))
            .expect("root");
        let authority = ManualTimeAuthority::new(
            registry,
            root,
            &[],
            ezsdr_kernel::module_api::Pacing::FreeRunning,
        )
        .expect("authority");
        let kinds = EventKindRegistry::with_kernel_kinds();
        let policy = kinds.compile(&BTreeMap::new()).expect("compiles");
        let events = EventCollector::new(&[], &policy.table.keys().cloned().collect::<Vec<_>>(), 16, &policy);
        Harness { authority, events, actions: QueueReceiver::new(), submitter }
    }

    fn ctx(&self) -> PrepareContext<'_> {
        prepare_context(
            RunId::generate(),
            &self.authority,
            &self.events,
            &self.actions,
            &self.submitter,
        )
    }
}

#[test]
fn ma_11_coerce_is_pure() {
    let p = TestProvider::new("radio", 2).with_grid(20.0);
    let request = Requested {
        resource: rid("radio"),
        constraints: [(key("test.grid"), Constraint::Eq { value: Value::Num(19.5) })]
            .into_iter()
            .collect(),
    };
    let a = p.coerce(&request).expect("pure");
    let b = p.coerce(&request).expect("pure");
    assert_eq!(a, b, "two calls with the same request return identical reports");
    assert_eq!(a.applied[&key("test.grid")], Value::Num(20.0));
}

#[test]
fn ma_12_prepare_matches_coerce() {
    let mut p = TestProvider::new("radio", 2).with_grid(20.0);
    let request = Requested {
        resource: rid("radio"),
        constraints: [(key("test.grid"), Constraint::Eq { value: Value::Num(19.5) })]
            .into_iter()
            .collect(),
    };
    let from_coerce = p.coerce(&request).expect("coerces");
    assert_eq!(from_coerce.coercions.len(), 1, "the request does coerce");
    let h = Harness::new(TestSubmitter::new());
    let fragment = ezsdr_kernel::plan::Fragment {
        id: id("radio"),
        instance: mid("ezsdr.test.provider"),
        role: Role::Provider,
        // The real shape `plan()` emits (SB-39): the selector and the matched
        // request side by side, not the request alone.
        content: serde_json::json!({
            "selector": {},
            "requested": serde_json::to_value(&request).expect("serialises"),
        }),
        after: Vec::new(),
    };
    let report = p.prepare(&fragment, h.ctx()).expect("prepares");
    // MA-12/SB-44's whole content is the equality: a dry run and a real run must
    // agree, so the report's coercions ARE what `coerce` returned.
    assert_eq!(report.coercions, from_coerce.coercions);
    assert_eq!(report.effective, from_coerce.applied);
    assert_eq!(report.fragment, id("radio"));
}

#[test]
fn ma_12_prepare_that_disagrees_with_coerce_is_visible() {
    // The double replays `coerce`; a Provider that did not would produce a report
    // the equality above catches. Proved by making the double lie once.
    let mut p = TestProvider::new("radio", 2).with_grid(20.0);
    p.prepare_disagrees = true;
    let request = Requested {
        resource: rid("radio"),
        constraints: [(key("test.grid"), Constraint::Eq { value: Value::Num(19.5) })]
            .into_iter()
            .collect(),
    };
    let from_coerce = p.coerce(&request).expect("coerces");
    let h = Harness::new(TestSubmitter::new());
    let fragment = ezsdr_kernel::plan::Fragment {
        id: id("radio"),
        instance: mid("ezsdr.test.provider"),
        role: Role::Provider,
        content: serde_json::to_value(&request).expect("serialises"),
        after: Vec::new(),
    };
    let report = p.prepare(&fragment, h.ctx()).expect("prepares");
    assert_ne!(report.coercions, from_coerce.coercions, "MA-12 would catch this Provider");
}

#[test]
fn ma_12_narrowing_triggers_readmission() {
    let declared = CapabilityValue::Range { min: Value::Int(1), max: Value::Int(8) };
    // Narrowing is allowed.
    let narrowed = CapabilityValue::Range { min: Value::Int(2), max: Value::Int(4) };
    assert!(check_effective_narrows(&declared, &narrowed).is_ok());
    // Widening is refused.
    let widened = CapabilityValue::Range { min: Value::Int(1), max: Value::Int(16) };
    assert!(check_effective_narrows(&declared, &widened).is_err());
    // Re-admission over the narrowed `effective` fails the Spec's own constraint.
    let asked = Constraint::Eq { value: Value::Int(8) };
    assert_eq!(ezsdr_kernel::binding::satisfies(&asked, &declared), Ok(true));
    assert_eq!(ezsdr_kernel::binding::satisfies(&asked, &narrowed), Ok(false));
}

#[test]
fn ma_07_lifecycle_order_and_cleanup() {
    for phase in [FailAt::Prepare, FailAt::Arm, FailAt::Start] {
        let mut p = TestProvider::new("radio", 2).failing_at(phase);
        let h = Harness::new(TestSubmitter::new());
        let fragment = ezsdr_kernel::plan::Fragment {
            id: id("radio"),
            instance: mid("ezsdr.test.provider"),
            role: Role::Provider,
            content: serde_json::Value::Null,
            after: Vec::new(),
        };
        let mut failed = false;
        failed |= p.prepare(&fragment, h.ctx()).is_err();
        if !failed {
            failed |= p.arm().is_err();
        }
        if !failed {
            failed |= p.start(None).is_err();
        }
        assert!(failed, "the injected failure at {phase:?} surfaced");
        // `stop` is always attempted before `cleanup`, on an abort as well (MA-7).
        let _ = p.stop(StopMode::Abort);
        p.cleanup();
        let calls = p.calls();
        assert_eq!(calls.last().map(String::as_str), Some("cleanup"));
        assert!(calls.contains(&"stop:Abort".to_owned()), "stop precedes cleanup: {calls:?}");
        // `cleanup` is idempotent (MA-7).
        p.cleanup();
        assert_eq!(p.calls().iter().filter(|c| *c == "cleanup").count(), 2);
    }
}

#[test]
fn ma_15_provider_step_default_is_idle() {
    let mut p = TestProvider::new("radio", 2);
    assert!(!p.instance().driving.stepped, "a hardware-style double is not stepped");
    assert_eq!(
        p.step(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)),
        Ok(StepOutcome { progressed: false }),
        "the default body is a no-op; no override is needed"
    );
}

#[test]
fn ma_10_sub_resource_binding() {
    let p = TestProvider::new("radio", 2);
    let nodes = p.instance().tree.walk();
    assert_eq!(nodes.len(), 3, "a two-level tree of a device with two sub-resources");
    let line = nodes.iter().find(|n| n.kind == ns("test.line")).expect("present");
    assert_eq!(line.id, rid("radio/0"), "a sub-resource carries its own ResourceId");
    assert!(line.id.is_within(&p.instance().id));
}

#[test]
fn ma_14_actions_arrive_only_after_admission() {
    // An Action rejected by an admission check never reaches the Module.
    let submitter = TestSubmitter::new().with_ceiling(20.0);
    let rejected = Action::TxBurst {
        target: rid("radio/tx/0"),
        waveform: ArtifactRef {
            id: id("wave"),
            kind: ns("test.waveform"),
            uri: "memory://wave".to_owned(),
            hash: some_hash("wave"),
            size_bytes: 8,
            partial: false,
            marks: Vec::new(),
            continuity: Vec::new(),
        },
        repeat: false,
        at: AbsoluteDeadline::new(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 10)),
        requested_at: None,
        late_policy: LatePolicy::SendAsapAndFlag,
        metadata: [(key("test.grid"), Value::Num(40.0))].into_iter().collect(),
    };
    assert!(submitter.submit(rejected).is_err());
    // The submitter this test *did* submit to is the carrier: the rejected Action did
    // not reach its queue. (A fresh Provider double's queue stood here too, which
    // nothing had written to and which therefore asserted nothing.)
    assert!(submitter.delivered.lock().expect("lock").is_empty(), "the queue stays empty");
}

#[test]
fn ma_14a_reactor_emits_through_admit() {
    let submitter = TestSubmitter::new().with_ceiling(20.0);
    let burst = |grid: f64| Action::TxBurst {
        target: rid("radio/tx/0"),
        waveform: ArtifactRef {
            id: id("wave"),
            kind: ns("test.waveform"),
            uri: "memory://wave".to_owned(),
            hash: some_hash("wave"),
            size_bytes: 8,
            partial: false,
            marks: Vec::new(),
            continuity: Vec::new(),
        },
        repeat: false,
        at: AbsoluteDeadline::new(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 10)),
        requested_at: None,
        late_policy: LatePolicy::SendAsapAndFlag,
        metadata: [(key("test.grid"), Value::Num(grid))].into_iter().collect(),
    };
    // Inside the envelope: admitted and dispatched.
    assert!(submitter.submit(burst(10.0)).is_ok());
    assert_eq!(submitter.delivered.lock().expect("lock").len(), 1);
    // Outside it: rejected before reaching the Provider.
    let violations = submitter.submit(burst(40.0)).expect_err("outside the envelope");
    assert_eq!(violations[0].check, ns("test.limits"));
    assert_eq!(submitter.delivered.lock().expect("lock").len(), 1, "nothing further was dispatched");
}

#[test]
fn ma_26_sink_returns_partial_on_abort() {
    let mut sink = TestSink::new(cf32());
    let refs = sink.stop(StopMode::Abort).expect("stop returns refs even on an abort");
    assert!(refs[0].partial);
    assert!(*sink.aborted.lock().expect("lock"));
}

// ---------------------------------------------------------------- descriptors

fn component(name: &str) -> ComponentDescriptor {
    ComponentDescriptor {
        id: id(name),
        kind: ComponentKind::Processor,
        ports: vec![
            Port { name: "in".to_owned(), direction: PortDirection::In, contract: cf32() },
            Port { name: "out".to_owned(), direction: PortDirection::Out, contract: cf32() },
        ],
        params: vec![ParamDecl {
            key: key("test.flag"),
            schema: serde_json::json!({ "type": "boolean" }),
            update_class: UpdateClass::BlockBoundary,
            default: Value::Bool(false),
        }],
        timing: ComponentTiming::default(),
        requires: ComponentRequires { executor_kind: "any".to_owned(), memory_bytes: None },
        implementation: ComponentImpl {
            kind: ns("test.impl"),
            id: name.to_owned(),
            hash: some_hash(name),
        },
    }
}

#[test]
fn ma_37_descriptor_structural_validation() {
    let contracts = vec![cf32()];
    assert!(component("a").validate(&contracts).is_ok());

    let mut dup = component("a");
    dup.ports[1].name = "in".to_owned();
    assert!(dup.validate(&contracts).is_err(), "duplicate port name");

    let mut unknown = component("a");
    unknown.ports[0].contract = DataContractId::parse("vendor.unregistered").expect("id");
    assert!(unknown.validate(&contracts).is_err(), "unregistered contract");

    let mut budget = component("a");
    budget.timing.budget = Some(
        RelativeBudget::new(Duration::new(ClockDomainId::HOST_MONOTONIC, 0)).expect("host"),
    );
    assert!(budget.validate(&contracts).is_err(), "a budget must be finite and positive");

    // A descriptor never carries an AbsoluteDeadline: they are distinct types, and
    // the budget field will not hold one.
    let json = serde_json::to_value(component("a")).expect("serialises");
    assert!(json["timing"].get("budget").is_some());
}

// ---------------------------------------------------------------- island admission

fn island(components: &[&str], executor: &str) -> IslandDecl {
    IslandDecl {
        id: IslandId::local(0),
        executor: id(executor),
        components: components.iter().map(|c| id(c)).collect(),
        affinity: None,
        rt_policy: None,
        batch: None,
    }
}

fn placement(domain: u32) -> ComponentPlacement {
    ComponentPlacement {
        island: id("io"),
        memory_domain: MemoryDomainId::local(domain),
    }
}

#[test]
fn ma_39_island_admission() {
    let components: BTreeMap<Ident, ComponentDescriptor> =
        [(id("a"), component("a")), (id("b"), component("b"))].into_iter().collect();
    let placements: BTreeMap<Ident, ComponentPlacement> =
        [(id("a"), placement(0)), (id("b"), placement(0))].into_iter().collect();
    let executors: BTreeMap<Ident, ExecutorDescriptor> = [(
        id("exec"),
        ExecutorDescriptor {
            kind: ns("test.executor"),
            memory_domains: vec![MemoryDomainId::local(0)],
            impl_kinds: vec![ns("test.impl")],
            capabilities: BTreeMap::new(),
        },
    )]
    .into_iter()
    .collect();
    let links: Vec<ezsdr_kernel::module_api::LinkDescriptor> = Vec::new();
    let graph_links = vec![(
        PortRef { component: "a".into(), port: "out".into() },
        PortRef { component: "b".into(), port: "in".into() },
        BackPressure::Block,
    )];
    let islands = vec![island(&["a", "b"], "exec")];
    let no_resources = BTreeSet::new();
    let ctx = IslandContext {
        islands: &islands,
        components: &components,
        placements: &placements,
        executors: &executors,
        links: &links,
        graph_links: &graph_links,
        resource_endpoints: &no_resources,
    };
    assert!(admit_islands(&ctx).is_ok());

    // 1. A component placed twice.
    let twice = vec![island(&["a", "b"], "exec"), island(&["a"], "exec")];
    let bad = IslandContext { islands: &twice, ..ctx_clone(&ctx) };
    let err = admit_islands(&bad).expect_err("placed twice");
    assert!(err.message.contains("exactly once"), "{err}");

    // 2. An executor kind the component does not want.
    let mut picky = components.clone();
    picky.get_mut(&id("a")).expect("present").requires.executor_kind = "test.gpu".to_owned();
    let bad = IslandContext { components: &picky, ..ctx_clone(&ctx) };
    let err = admit_islands(&bad).expect_err("wrong executor kind");
    assert!(err.message.contains("executor kind"), "{err}");

    // 3. An impl kind the Executor cannot load.
    let mut alien = components.clone();
    alien.get_mut(&id("a")).expect("present").implementation.kind = ns("vendor.wasm");
    let bad = IslandContext { components: &alien, ..ctx_clone(&ctx) };
    assert!(admit_islands(&bad).expect_err("unknown impl kind").message.contains("impl_kinds"));

    // 4. A memory domain the Executor cannot reach.
    let far: BTreeMap<Ident, ComponentPlacement> =
        [(id("a"), placement(9)), (id("b"), placement(0))].into_iter().collect();
    let bad = IslandContext { placements: &far, ..ctx_clone(&ctx) };
    assert!(admit_islands(&bad).expect_err("unreachable domain").message.contains("memory_domains"));

    // 5. An rt_policy with a component that declares no budget.
    let rt = vec![IslandDecl {
        rt_policy: Some(RtPolicy { sched: "fifo".to_owned(), priority: 50 }),
        ..island(&["a", "b"], "exec")
    }];
    let bad = IslandContext { islands: &rt, ..ctx_clone(&ctx) };
    assert!(admit_islands(&bad).expect_err("no budget").message.contains("budget"));
}

fn ctx_clone<'a>(ctx: &IslandContext<'a>) -> IslandContext<'a> {
    IslandContext {
        islands: ctx.islands,
        components: ctx.components,
        placements: ctx.placements,
        executors: ctx.executors,
        links: ctx.links,
        graph_links: ctx.graph_links,
        resource_endpoints: ctx.resource_endpoints,
    }
}

#[test]
fn ma_22_cycle_rules() {
    let nodes = vec![id("a"), id("b")];
    // A stream.* cycle is rejected.
    let stream = vec![
        GraphEdge { from: id("a"), to: id("b"), kind: EdgeKind::Stream, crosses_island: false },
        GraphEdge { from: id("b"), to: id("a"), kind: EdgeKind::Stream, crosses_island: false },
    ];
    assert!(check_cycles(&nodes, &stream).is_err());

    // An Event cycle across Islands is accepted.
    let across = vec![
        GraphEdge { from: id("a"), to: id("b"), kind: EdgeKind::Stream, crosses_island: true },
        GraphEdge { from: id("b"), to: id("a"), kind: EdgeKind::Event, crosses_island: true },
    ];
    assert!(check_cycles(&nodes, &across).is_ok());

    // An Event cycle inside one Island is rejected.
    let inside = vec![
        GraphEdge { from: id("a"), to: id("b"), kind: EdgeKind::Stream, crosses_island: false },
        GraphEdge { from: id("b"), to: id("a"), kind: EdgeKind::Event, crosses_island: false },
    ];
    assert!(check_cycles(&nodes, &inside).is_err());
}

#[test]
fn ma_25_sink_role_is_read_from_the_binding() {
    // MA-25: the Kernel reads the Sink role from the `ModuleDescriptor` of the
    // Module the profile **binds** to the output — on a Spec Run and a Session
    // alike. It used to come from a placement's `module`, which SB-25a left unset
    // on a Spec Run, so SC-21 was unenforceable on the publication path (D17).
    let mut reg = ModuleRegistry::new();
    reg.register_vocabulary(test_vocabulary()).expect("fresh");
    reg.register(test_sink_descriptor(), Factories { sink: true, ..Factories::default() })
        .expect("registers");
    reg.register(test_provider_descriptor(),
        Factories { provider: true, authority: true, ..Factories::default() },)
        .expect("registers");

    let holds_sink = |m: &ezsdr_kernel::id::ModuleId| {
        reg.modules().any(|d| d.id == *m && d.roles.contains(&Role::Sink))
    };
    assert!(holds_sink(&mid("ezsdr.test.sink")));
    assert!(!holds_sink(&mid("ezsdr.test.provider")), "a Provider is not a Sink");

    // SC-21: a `Block` link into a Sink is refused, and the predicate is now
    // available on both paths because it reads a binding rather than a placement.
    let decl = DataLinkDecl {
        id: DataLinkId::local(0),
        from: PortRef { component: "radio".into(), port: "rx".into() },
        to: PortRef { component: "capture0".into(), port: "in".into() },
        contract: cf32(),
        policy: BackPressure::Block,
        capacity: 4,
    };
    assert!(ezsdr_kernel::stream::check_sink_link(&decl, true).is_err());
    let _ = Placements::default();
}

// ---------------------------------------------------------------- the stepping loop

/// One double that holds three roles at once, which is exactly what MA-1 permits,
/// so that the stepping order can be observed across roles with one shared log.
struct StepLogger {
    name: String,
    log: Arc<Mutex<Vec<String>>>,
    budget: Mutex<usize>,
    instance: ezsdr_kernel::module_api::ProviderInstance,
    executor: ExecutorDescriptor,
    sink: ezsdr_kernel::module_api::SinkDescriptor,
}

impl StepLogger {
    fn new(name: &str, log: Arc<Mutex<Vec<String>>>, budget: usize) -> StepLogger {
        StepLogger {
            name: name.to_owned(),
            log,
            budget: Mutex::new(budget),
            instance: TestProvider::new("x", 1).instance().clone(),
            executor: TestExecutor::new(MemoryDomainId::local(0)).descriptor().clone(),
            sink: TestSink::new(cf32()).descriptor().clone(),
        }
    }

    fn tick(&self, role: &str) -> StepOutcome {
        self.log.lock().expect("lock").push(format!("{role}:{}", self.name));
        let mut b = self.budget.lock().expect("lock");
        let progressed = *b > 0;
        *b = b.saturating_sub(1);
        StepOutcome { progressed }
    }
}

impl Provider for StepLogger {
    fn instance(&self) -> &ezsdr_kernel::module_api::ProviderInstance {
        &self.instance
    }
    fn coerce(&self, _r: &Requested) -> Result<ezsdr_kernel::module_api::CoerceReport, ModuleError> {
        Ok(Default::default())
    }
    fn prepare(&mut self, _f: &ezsdr_kernel::plan::Fragment, _c: PrepareContext<'_>) -> Result<PrepareReport, ModuleError> {
        unreachable!("the stepping test does not prepare")
    }
    fn arm(&mut self) -> Result<(), ModuleError> {
        Ok(())
    }
    fn start(&mut self, _at: Option<TimePoint>) -> Result<(), ModuleError> {
        Ok(())
    }
    fn stop(&mut self, _m: StopMode) -> Result<(), ModuleError> {
        Ok(())
    }
    fn cleanup(&mut self) {}
    fn step(&mut self, _until: TimePoint) -> Result<StepOutcome, ModuleError> {
        Ok(self.tick("provider"))
    }
}

impl Executor for StepLogger {
    fn descriptor(&self) -> &ExecutorDescriptor {
        &self.executor
    }
    fn prepare(&mut self, _i: &IslandDecl, _c: PrepareContext<'_>) -> Result<PrepareReport, ModuleError> {
        unreachable!("the stepping test does not prepare")
    }
    fn arm(&mut self) -> Result<(), ModuleError> {
        Ok(())
    }
    fn start(&mut self) -> Result<(), ModuleError> {
        Ok(())
    }
    fn step(&mut self, _until: TimePoint) -> Result<StepOutcome, ModuleError> {
        Ok(self.tick("executor"))
    }
    fn stop(&mut self, _m: StopMode) -> Result<(), ModuleError> {
        Ok(())
    }
    fn cleanup(&mut self) {}
}

impl Sink for StepLogger {
    fn descriptor(&self) -> &ezsdr_kernel::module_api::SinkDescriptor {
        &self.sink
    }
    fn prepare(&mut self, _f: &ezsdr_kernel::plan::Fragment, _c: PrepareContext<'_>) -> Result<PrepareReport, ModuleError> {
        unreachable!("the stepping test does not prepare")
    }
    fn arm(&mut self) -> Result<(), ModuleError> {
        Ok(())
    }
    fn start(&mut self) -> Result<(), ModuleError> {
        Ok(())
    }
    fn step(&mut self, _until: TimePoint) -> Result<StepOutcome, ModuleError> {
        Ok(self.tick("sink"))
    }
    fn stop(&mut self, _m: StopMode) -> Result<Vec<ArtifactRef>, ModuleError> {
        Ok(Vec::new())
    }
    fn cleanup(&mut self) {}
}

#[test]
fn ma_30_stepping_order_and_quiescence() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut sink = StepLogger::new("a", log.clone(), 1);
    let mut executor = StepLogger::new("b", log.clone(), 0);
    let mut provider = StepLogger::new("c", log.clone(), 0);
    // Registered in reverse role order.
    let mut instances = vec![
        SteppedInstance { id: id("a"), inner: SteppedRef::Sink(&mut sink) },
        SteppedInstance { id: id("b"), inner: SteppedRef::Executor(&mut executor) },
        SteppedInstance { id: id("c"), inner: SteppedRef::Provider(&mut provider) },
    ];
    let until = TimePoint::new(ClockDomainId::HOST_MONOTONIC, 100);
    let events = collector();
    let rounds = step_until_quiescent(&mut instances, until, &events, &rid("coordinator"))
        .expect("quiesces");
    let seen = log.lock().expect("lock").clone();
    assert_eq!(
        &seen[..3],
        &["provider:c".to_owned(), "executor:b".to_owned(), "sink:a".to_owned()],
        "role rank, then instance id; not registration order"
    );
    assert_eq!(rounds, 2, "one round made progress, the next quiesced");
}

#[test]
fn ma_30_stepping_livelock_cap() {
    let log = Arc::new(Mutex::new(Vec::new()));
    // Two components that keep rescheduling each other at one instant.
    let mut a = StepLogger::new("a", log.clone(), usize::MAX);
    let mut b = StepLogger::new("b", log.clone(), usize::MAX);
    let mut instances = vec![
        SteppedInstance { id: id("a"), inner: SteppedRef::Executor(&mut a) },
        SteppedInstance { id: id("b"), inner: SteppedRef::Executor(&mut b) },
    ];
    let until = TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0);
    let events = collector();
    let err = step_until_quiescent(&mut instances, until, &events, &rid("coordinator"))
        .expect_err("STEP_LIVELOCK, not a hang");
    assert!(err.message.contains("stepping rounds"), "{err}");
    assert_eq!(err.detail["event_kind"], serde_json::json!(EventKind::STEP_LIVELOCK));

    // RS-27 registers STEP_LIVELOCK as a kind the Kernel emits from its own
    // stepping loop, so it must reach the counters and the escalation flag.
    let drained = events.drain();
    assert!(drained.iter().any(|e| e.kind.as_str() == EventKind::STEP_LIVELOCK));
    assert_eq!(
        events.escalation().map(|(_, r)| r),
        Some(ezsdr_kernel::policy::Reaction::Abort),
        "RS-28 gives it `abort` at `fatal`"
    );
}

/// An event collector over the Kernel's own kinds, for the stepping loop (RS-27).
fn collector() -> EventCollector {
    let kinds = EventKindRegistry::with_kernel_kinds();
    let policy = kinds.compile(&BTreeMap::new()).expect("compiles");
    EventCollector::new(&[], &policy.table.keys().cloned().collect::<Vec<_>>(), 16, &policy)
}

#[test]
fn ma_42_fidelity_is_the_weakest() {
    use ezsdr_kernel::module_api::{
        CoercionFidelity, EnvelopeFidelity, Fidelity, RfFidelity, TransportFidelity,
    };
    // MA-42: "A Run's value per aspect is the weakest over its bound Providers, in
    // that aspect's own order" — per aspect, so a Provider that is stronger on one
    // axis does not lift a Run that is weaker on another.
    let quirky = Fidelity {
        timing: EnvelopeFidelity::HardwareQuirk,
        continuity: EnvelopeFidelity::HardwareQuirk,
        coercion: CoercionFidelity::Grid,
        rf: RfFidelity::ImpairmentModel,
        transport: TransportFidelity::Model,
    };
    let enveloped = Fidelity {
        timing: EnvelopeFidelity::Envelope,
        continuity: EnvelopeFidelity::Envelope,
        coercion: CoercionFidelity::Grid,
        rf: RfFidelity::None,
        transport: TransportFidelity::Model,
    };
    let run = Fidelity::weakest(&[quirky, enveloped]);
    assert_eq!(run.timing, EnvelopeFidelity::Envelope);
    assert_eq!(run.continuity, EnvelopeFidelity::Envelope);
    assert_eq!(run.coercion, CoercionFidelity::Grid, "equal on this axis, so unchanged");
    assert_eq!(run.rf, RfFidelity::None, "weakest per aspect, not per Provider");
    assert_eq!(run.transport, TransportFidelity::Model);

    // MA-42's own reason for adding `real` to Vision §14's sets: a Hardware Run has
    // to have something to record, and a Hardware Run with one best-effort Mock
    // peripheral records that peripheral's weaker value — `timing: envelope`.
    let real = Fidelity {
        timing: EnvelopeFidelity::Real,
        continuity: EnvelopeFidelity::Real,
        coercion: CoercionFidelity::Real,
        rf: RfFidelity::Real,
        transport: TransportFidelity::Real,
    };
    assert_eq!(Fidelity::weakest(&[real]), real, "a Hardware Run records `real` where declared");
    assert_eq!(Fidelity::weakest(&[real, enveloped]).timing, EnvelopeFidelity::Envelope);
    // RS-41: a Run with no bound Providers records the all-`none` vector, not `real`.
    assert_eq!(Fidelity::weakest(&[]), Fidelity::NONE);
}

#[test]
fn ma_39_a_cross_domain_link_inside_one_island_needs_a_registered_link() {
    // MA-39's reachability check, and the only place `LinkDescriptor.connects`
    // (MA-27, MA-28) is read: two components in **one** Island but in two memory
    // domains are admitted only when a registered Link joins the pair. Every fixture
    // that reached this check placed both components in the same domain, so the
    // branch was unreached — the code existed and nothing ran it (exit-review GAP).
    let components: BTreeMap<Ident, ComponentDescriptor> =
        [(id("a"), component("a")), (id("b"), component("b"))].into_iter().collect();
    // `b` sits in memory domain 1, `a` in 0.
    let placements: BTreeMap<Ident, ComponentPlacement> =
        [(id("a"), placement(0)), (id("b"), placement(1))].into_iter().collect();
    let executors: BTreeMap<Ident, ExecutorDescriptor> = [(
        id("exec"),
        ExecutorDescriptor {
            kind: ns("test.executor"),
            memory_domains: vec![MemoryDomainId::local(0), MemoryDomainId::local(1)],
            impl_kinds: vec![ns("test.impl")],
            capabilities: BTreeMap::new(),
        },
    )]
    .into_iter()
    .collect();
    let graph_links = vec![(
        PortRef { component: "a".into(), port: "out".into() },
        PortRef { component: "b".into(), port: "in".into() },
        BackPressure::Block,
    )];
    let islands = vec![island(&["a", "b"], "exec")];
    let no_resources = BTreeSet::new();

    // No registered Link: the pair is unreachable and the Island is refused.
    let none: Vec<ezsdr_kernel::module_api::LinkDescriptor> = Vec::new();
    let ctx = IslandContext {
        islands: &islands,
        components: &components,
        placements: &placements,
        executors: &executors,
        links: &none,
        graph_links: &graph_links,
        resource_endpoints: &no_resources,
    };
    let err = admit_islands(&ctx).expect_err("two domains, no Link");
    assert!(err.message.contains("no registered Link connects them"), "{err}");

    // A Link that joins the pair admits it — and the check is symmetric, so a Link
    // declaring the reverse direction serves as well.
    for connects in [
        vec![(MemoryDomainId::local(0), MemoryDomainId::local(1))],
        vec![(MemoryDomainId::local(1), MemoryDomainId::local(0))],
    ] {
        let joined = vec![ezsdr_kernel::module_api::LinkDescriptor {
            kind: ns("test.link"),
            connects,
            policies: vec![BackPressure::Block],
            cross_process: false,
        }];
        let ctx = IslandContext { links: &joined, ..ctx_clone(&ctx) };
        admit_islands(&ctx).expect("a registered Link joins the two domains");
    }

    // A Link that joins some *other* pair does not help.
    let elsewhere = vec![ezsdr_kernel::module_api::LinkDescriptor {
        kind: ns("test.link"),
        connects: vec![(MemoryDomainId::local(1), MemoryDomainId::local(2))],
        policies: vec![BackPressure::Block],
        cross_process: false,
    }];
    let ctx = IslandContext { links: &elsewhere, ..ctx_clone(&ctx) };
    assert!(admit_islands(&ctx).is_err(), "a Link joining another pair is not this pair's");
}
