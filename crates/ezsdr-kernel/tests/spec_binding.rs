//! Phase 1 tests for `03-spec-and-binding.md`. Each name begins with the rule it proves (OV-19).

mod support;

use std::collections::BTreeMap;

use ezsdr_kernel::binding::{
    AdmissionCheckRegistry, BindingProfile, CheckStage, ComponentPlacement, Placements, satisfies,
};
use ezsdr_kernel::contract::{ContractRegistry, PortRef};
use ezsdr_kernel::event::ActionTemplate;
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::{ClockDomainId, DataLinkId, IslandId, MemoryDomainId};
use ezsdr_kernel::module_api::{
    AuthorityDescriptor, ExecutorDescriptor, Factories, IslandDecl, ModuleRegistry, Pacing,
    Provider, Role, UpdateClass,
};
use ezsdr_kernel::plan::{
    CompileInputs, DeclaredCost, MergedPrepare, PrepareReport, arm_order, coercion_policy,
    collect_prepare, plan, release_order, validate,
};
use ezsdr_kernel::policy::{EventKindRegistry, Reaction};
use ezsdr_kernel::spec::{
    CapabilityValue, CoercionPolicy, Constraint, ExperimentSpec, Ident, Key, KeyDecl,
    MigrationRegistry, Namespace, ResourceReq, ScheduleEntry, SpecError, SpecTime, SubResourceReq,
    Value, ValueKind,
};
use ezsdr_kernel::stream::{BackPressure, LatePolicy};
use ezsdr_kernel::time::{AbsoluteDeadline, TimePoint};
use support::{
    FailAt, TestLimitsCheck, TestProvider, TestSink, id, key, mid, ns, rid, some_hash,
    test_provider_descriptor, test_sink_descriptor, test_vocabulary,
};

fn registry() -> ModuleRegistry {
    let mut reg = ModuleRegistry::new();
    reg.register_vocabulary(test_vocabulary()).expect("fresh");
    reg.register(
        test_provider_descriptor(),
        Factories { provider: true, authority: true, ..Factories::default() },
    )
    .expect("registers");
    reg.register(test_sink_descriptor(), Factories { sink: true, ..Factories::default() })
        .expect("registers");
    reg.register(
        support::test_executor_descriptor(),
        Factories { executor: true, ..Factories::default() },
    )
    .expect("registers");
    reg
}

fn kinds() -> EventKindRegistry {
    let mut k = EventKindRegistry::with_kernel_kinds();
    for decl in test_vocabulary().event_kinds {
        k.register(Some(ns("test")), decl).expect("fresh");
    }
    k
}

/// A minimal v1 Spec requiring one `test.device` with `test.count: Eq(2)`.
fn minimal_spec() -> ExperimentSpec {
    serde_json::from_value(serde_json::json!({
        "version": 1,
        "requirements": { "vocabularies": [{ "id": "test", "major": 1 }] },
        "resources": {
            "radio": {
                "kind": "test.device",
                "requires": { "test.count": { "kind": "eq", "value": 2 } }
            }
        }
    }))
    .expect("the fixture parses")
}

/// A Spec built from parts, so tests can vary one field.
fn spec_with(resources: BTreeMap<Ident, ResourceReq>) -> ExperimentSpec {
    let mut s = ExperimentSpec { version: 1, ..ExperimentSpec::default() };
    s.requirements.vocabularies.push(ezsdr_kernel::spec::VocabularyReq { id: ns("test"), major: 1 });
    s.resources = resources;
    s
}

fn resource(kind: &str, requires: &[(&str, Constraint)]) -> ResourceReq {
    ResourceReq {
        kind: ns(kind),
        requires: requires.iter().map(|(k, c)| (key(k), c.clone())).collect(),
        needs: BTreeMap::new(),
        extensions: BTreeMap::new(),
    }
}

/// Marks the named bindings as **distinct instances** by giving each a selector of
/// its own. `profile_binding` leaves the selector empty, which under SB-3 means one
/// binding description and therefore one instance — the right default for a fixture
/// that binds one Provider object to two names, and the wrong one for a fixture that
/// binds two objects, which must say so.
fn distinct_instances(mut p: BindingProfile, names: &[&str]) -> BindingProfile {
    for n in names {
        if let Some(b) = p.bindings.get_mut(&id(n)) {
            b.selector.insert(id("instance"), Value::Str((*n).to_owned()));
        }
    }
    p
}

fn profile_binding(names: &[&str]) -> BindingProfile {
    let mut p = BindingProfile {
        version: 1,
        bindings: names
            .iter()
            .map(|n| {
                (
                    id(n),
                    ezsdr_kernel::binding::Binding {
                        module: mid("ezsdr.test.provider"),
                        feed: None,
                        selector: BTreeMap::new(),
                        profile: None,
                    },
                )
            })
            .collect(),
        authority: None,
        placements: Placements::default(),
        environment: BTreeMap::new(),
    };
    // MA-38: an Island's `executor` names a binding, so the profile binds the
    // Executor Module here rather than the runtime handing it in (finding D18).
    p.bindings.insert(
        id("exec"),
        ezsdr_kernel::binding::Binding {
            module: mid("ezsdr.test.executor"),
            feed: None,
            selector: BTreeMap::new(),
            profile: None,
        },
    );
    p
}

struct Fixture<'a> {
    registry: ModuleRegistry,
    kinds: EventKindRegistry,
    checks: AdmissionCheckRegistry,
    contracts: ContractRegistry,
    authorities: BTreeMap<Ident, AuthorityDescriptor>,
    executors: BTreeMap<Ident, ExecutorDescriptor>,
    links: Vec<ezsdr_kernel::module_api::LinkDescriptor>,
    sinks: BTreeMap<Ident, &'a dyn ezsdr_kernel::module_api::Sink>,
    is_session: bool,
}

impl<'a> Fixture<'a> {
    fn new() -> Fixture<'a> {
        Fixture {
            registry: registry(),
            kinds: kinds(),
            checks: AdmissionCheckRegistry::new(),
            contracts: ContractRegistry::with_standard_contracts(),
            authorities: [(
                id("radio"),
                AuthorityDescriptor { governs: vec![ClockDomainId::HOST_MONOTONIC], pacing: Pacing::FreeRunning },
            )]
            .into_iter()
            .collect(),
            executors: [(
                id("exec"),
                ExecutorDescriptor {
                    kind: ns("test.executor"),
                    memory_domains: vec![MemoryDomainId::local(0)],
                    impl_kinds: vec![ns("test.impl")],
                    capabilities: BTreeMap::new(),
                },
            )]
            .into_iter()
            .collect(),
            links: Vec::new(),
            sinks: BTreeMap::new(),
            is_session: false,
        }
    }

    fn inputs(&'a self, providers: &'a BTreeMap<Ident, &'a dyn Provider>) -> CompileInputs<'a> {
        CompileInputs {
            registry: &self.registry,
            checks: &self.checks,
            kinds: &self.kinds,
            providers,
            authorities: &self.authorities,
            executors: &self.executors,
            links: &self.links,
            contracts: &self.contracts,
            sinks: &self.sinks,
            is_session: self.is_session,
        }
    }
}

fn one_provider<'a>(name: &str, p: &'a dyn Provider) -> BTreeMap<Ident, &'a dyn Provider> {
    [(id(name), p)].into_iter().collect()
}

/// `collect_prepare` with the documents and inputs it now takes, for a test that
/// only cares about the reports (SB-41).
fn prepared(
    reports: Vec<Result<PrepareReport, ezsdr_kernel::module_api::ModuleError>>,
    fx: &Fixture<'_>,
    spec: &ExperimentSpec,
    profile: &ezsdr_kernel::binding::BindingProfile,
    providers: &BTreeMap<Ident, &dyn Provider>,
) -> Result<ezsdr_kernel::plan::MergedPrepare, ezsdr_kernel::plan::PrepareError> {
    // The caller's own Spec, and the admission `validate` produced for it. Hard-coding
    // `minimal_spec()` here meant every caller had MA-12 and the Spec's constraints
    // checked against a Spec it had not written, which is how a P0 on the crate's
    // headline coercion path survived two review passes and 287 green tests.
    let admission = validate(spec, profile, &fx.inputs(providers))
        .unwrap_or_else(|e| panic!("the caller's Spec validates: {e:?}"));
    collect_prepare(reports, spec, profile, &fx.inputs(providers), &admission)
}

/// `validate` then `plan`, which is the order SB-37 and SB-39 define. `plan` needs
/// `validate`'s result, so calling it alone is not a thing the pipeline does.
fn validate_then_plan(
    spec: &ExperimentSpec,
    profile: &ezsdr_kernel::binding::BindingProfile,
    inputs: &ezsdr_kernel::plan::CompileInputs<'_>,
    costs: Vec<DeclaredCost>,
) -> Result<ezsdr_kernel::plan::ExecutionPlan, SpecError> {
    let admission = validate(spec, profile, inputs)?;
    plan(spec, profile, &admission, inputs, costs)
}

// ---------------------------------------------------------------- envelope

#[test]
fn sb_10_version_one_validates() {
    let spec = minimal_spec();
    let profile = profile_binding(&["radio"]);
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2);
    let providers = one_provider("radio", &p);
    let result = validate(&spec, &profile, &fx.inputs(&providers)).expect("validates");
    assert!(result.is_admitted());
    assert_eq!(result.matched[&id("radio")], rid("radio"));
    assert_eq!(profile.version, 1);
}

#[test]
fn sb_47_version_two_refused_by_name() {
    let doc = serde_json::json!({ "version": 2, "resources": {} });
    assert_eq!(
        ExperimentSpec::from_json(&doc),
        Err(SpecError::UnsupportedVersion { found: 2, supported: vec![1] })
    );
    assert_eq!(
        BindingProfile::from_json(&serde_json::json!({ "version": 2 })),
        Err(SpecError::UnsupportedVersion { found: 2, supported: vec![1] })
    );
}

#[test]
fn sb_48_migration_point_exists() {
    let mut reg = MigrationRegistry::new();
    reg.register(0, |mut doc| {
        doc["version"] = serde_json::json!(1);
        Ok(doc)
    });
    let doc = serde_json::json!({ "version": 0, "resources": {} });
    let (migrated, original) = reg.migrate(doc).expect("migrates");
    assert_eq!(original, 0, "the Manifest records the original version (SB-49)");
    assert_eq!(migrated["version"], serde_json::json!(1));
    assert!(ExperimentSpec::from_json(&migrated).is_ok());

    // With no step registered, a version 0 document is refused, never reinterpreted.
    let empty = MigrationRegistry::new();
    assert!(matches!(
        empty.migrate(serde_json::json!({ "version": 0 })),
        Err(SpecError::UnsupportedVersion { found: 0, .. })
    ));
}

#[test]
fn sb_09_unknown_top_level_field_refused() {
    let doc = serde_json::json!({ "version": 1, "constants": { "a": 1 } });
    assert_eq!(
        ExperimentSpec::from_json(&doc),
        Err(SpecError::UnknownField { path: "constants".to_owned() })
    );
}

#[test]
fn sb_14_compute_expression_refused() {
    // v3's CONSTANTS is an unknown top-level field (SB-9).
    assert!(matches!(
        ExperimentSpec::from_json(&serde_json::json!({ "version": 1, "CONSTANTS": { "a": 1 } })),
        Err(SpecError::UnknownField { .. })
    ));
    // and a `!COMPUTE(...)` value is just a string: there is no evaluation pass.
    let spec = ExperimentSpec::from_json(&serde_json::json!({
        "version": 1,
        "requirements": { "vocabularies": [{ "id": "test", "major": 1 }] },
        "resources": {
            "radio": {
                "kind": "test.device",
                "requires": { "test.count": { "kind": "eq", "value": "!COMPUTE(A - B)" } }
            }
        }
    }))
    .expect("a string is a scalar");
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2);
    let providers = one_provider("radio", &p);
    // It is never evaluated: it fails the key's declared kind instead (SB-6).
    assert!(matches!(
        validate(&spec, &profile_binding(&["radio"]), &fx.inputs(&providers)),
        Err(SpecError::KeyShape { .. })
    ));
}

#[test]
fn sb_13_placement_in_spec_refused() {
    let doc = serde_json::json!({ "version": 1, "graph": { "placement": {} } });
    assert_eq!(
        ExperimentSpec::from_json(&doc),
        Err(SpecError::PlacementInSpec { path: "graph.placement".to_owned() })
    );
    let doc = serde_json::json!({
        "version": 1,
        "resources": { "radio": { "kind": "test.device", "memory_domain": 0 } }
    });
    assert_eq!(
        ExperimentSpec::from_json(&doc),
        Err(SpecError::PlacementInSpec { path: "resources.radio.memory_domain".to_owned() })
    );
}

#[test]
fn sb_13_environment_in_spec_refused() {
    let doc = serde_json::json!({
        "version": 1,
        "resources": { "radio": { "kind": "test.device", "environment": {} } }
    });
    assert_eq!(
        ExperimentSpec::from_json(&doc),
        Err(SpecError::EnvironmentInSpec { path: "resources.radio.environment".to_owned() })
    );
}

#[test]
fn sb_02_unknown_key_prefix_refused() {
    let mut spec = spec_with(
        [(id("radio"), resource("test.device", &[("radio.rx.channels", Constraint::Eq { value: Value::Int(2) })]))]
            .into_iter()
            .collect(),
    );
    spec.requirements.vocabularies.clear();
    spec.requirements.vocabularies.push(ezsdr_kernel::spec::VocabularyReq { id: ns("test"), major: 1 });
    assert!(matches!(spec.check_key_prefixes(), Err(SpecError::UnknownKeyPrefix { .. })));
}

// ---------------------------------------------------------------- matching

#[test]
fn sb_06_constraint_match_table() {
    let one = CapabilityValue::One { value: Value::Int(4) };
    let range = CapabilityValue::Range { min: Value::Int(2), max: Value::Int(8) };
    let any = CapabilityValue::AnyOf { values: vec![Value::Int(1), Value::Int(4), Value::Int(9)] };

    let eq4 = Constraint::Eq { value: Value::Int(4) };
    assert_eq!(satisfies(&eq4, &one), Ok(true));
    assert_eq!(satisfies(&eq4, &range), Ok(true));
    assert_eq!(satisfies(&eq4, &any), Ok(true));
    assert_eq!(satisfies(&Constraint::Eq { value: Value::Int(5) }, &one), Ok(false));
    assert_eq!(satisfies(&Constraint::Eq { value: Value::Int(5) }, &any), Ok(false));

    let r = Constraint::Range { min: Some(Value::Int(3)), max: Some(Value::Int(5)) };
    assert_eq!(satisfies(&r, &one), Ok(true));
    assert_eq!(satisfies(&r, &range), Ok(true), "overlapping ranges");
    assert_eq!(satisfies(&r, &any), Ok(true));
    let disjoint = Constraint::Range { min: Some(Value::Int(20)), max: Some(Value::Int(30)) };
    assert_eq!(satisfies(&disjoint, &range), Ok(false));

    let set = Constraint::Set { values: vec![Value::Int(4), Value::Int(100)] };
    assert_eq!(satisfies(&set, &one), Ok(true));
    assert_eq!(satisfies(&set, &range), Ok(true));
    assert_eq!(satisfies(&set, &any), Ok(true));
    assert_eq!(satisfies(&Constraint::Set { values: vec![Value::Int(100)] }, &one), Ok(false));

    assert_eq!(satisfies(&Constraint::Min { value: Value::Int(4) }, &one), Ok(true));
    assert_eq!(satisfies(&Constraint::Min { value: Value::Int(9) }, &range), Ok(false));
    assert_eq!(satisfies(&Constraint::Min { value: Value::Int(8) }, &any), Ok(true));
    assert_eq!(satisfies(&Constraint::Max { value: Value::Int(4) }, &one), Ok(true));
    assert_eq!(satisfies(&Constraint::Max { value: Value::Int(1) }, &range), Ok(false));
    assert_eq!(satisfies(&Constraint::Max { value: Value::Int(1) }, &any), Ok(true));

    assert_eq!(satisfies(&Constraint::Present, &one), Ok(true));
}

#[test]
fn sb_06_key_shape_mismatch() {
    let decl = KeyDecl {
        key: key("test.flag"),
        kind: ValueKind::Bool,
        coercible: false,
        coercion_default: CoercionPolicy::Warn,
        update_class: None,
    };
    assert!(matches!(
        ezsdr_kernel::binding::check_constraint_kind(&decl, &Constraint::Eq { value: Value::Int(1) }),
        Err(SpecError::KeyShape { .. })
    ));
    assert!(
        ezsdr_kernel::binding::check_constraint_kind(&decl, &Constraint::Eq { value: Value::Bool(true) })
            .is_ok()
    );
}

#[test]
fn sb_07_coercible_key_consults_provider() {
    let spec = spec_with(
        [(id("radio"), resource("test.device", &[("test.grid", Constraint::Eq { value: Value::Num(19.5) })]))]
            .into_iter()
            .collect(),
    );
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2).with_grid(20.0);
    let providers = one_provider("radio", &p);
    let result = validate(&spec, &profile_binding(&["radio"]), &fx.inputs(&providers))
        .expect("validates");
    assert_eq!(
        p.coerce_calls.load(std::sync::atomic::Ordering::Relaxed),
        1,
        "coerce is called once"
    );
    assert_eq!(result.coercions_preview.len(), 1);
    assert_eq!(result.coercions_preview[0].coercion.applied, Value::Num(20.0));
}

#[test]
fn sb_07_non_coercible_key_fails_directly() {
    let spec = spec_with(
        [(id("radio"), resource("test.device", &[("test.count", Constraint::Eq { value: Value::Int(9) })]))]
            .into_iter()
            .collect(),
    );
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2).with_grid(20.0);
    let providers = one_provider("radio", &p);
    let result = validate(&spec, &profile_binding(&["radio"]), &fx.inputs(&providers))
        .expect("validates");
    assert!(!result.is_admitted());
    assert_eq!(p.coerce_calls.load(std::sync::atomic::Ordering::Relaxed), 0, "coerce is not called");
    assert_eq!(result.rejected.len(), 1);
    assert_eq!(result.rejected[0].key, key("test.count"));
}

#[test]
fn sb_45_coercion_policy_chain() {
    let decl = KeyDecl {
        key: key("test.grid"),
        kind: ValueKind::Num,
        coercible: true,
        coercion_default: CoercionPolicy::Reject,
        update_class: None,
    };
    // A Session warns, whatever the Vocabulary's default says.
    assert_eq!(coercion_policy(None, true, Some(&decl)), CoercionPolicy::Warn);
    // A Spec Run takes the Vocabulary's default.
    assert_eq!(coercion_policy(None, false, Some(&decl)), CoercionPolicy::Reject);
    // A Spec override wins over both.
    assert_eq!(
        coercion_policy(Some(CoercionPolicy::Accept), false, Some(&decl)),
        CoercionPolicy::Accept
    );
    // With no declaration at all, `warn`.
    assert_eq!(coercion_policy(None, false, None), CoercionPolicy::Warn);
}

#[test]
fn sb_22_unbound_resource_fails() {
    let spec = minimal_spec();
    let profile = profile_binding(&[]);
    let fx = Fixture::new();
    let providers = BTreeMap::new();
    assert_eq!(
        validate(&spec, &profile, &fx.inputs(&providers)),
        Err(SpecError::UnboundResource { name: id("radio") })
    );
}

#[test]
fn sb_35_no_single_instance() {
    // Two instances each declaring test.count: 2, a requirement of Min(4).
    let spec = spec_with(
        [(id("radio"), resource("test.device", &[("test.count", Constraint::Min { value: Value::Int(4) })]))]
            .into_iter()
            .collect(),
    );
    let fx = Fixture::new();
    let a = TestProvider::new("radio", 2);
    let b = TestProvider::new("radio2", 2);
    let providers: BTreeMap<Ident, &dyn Provider> =
        [(id("radio"), &a as &dyn Provider), (id("radio2"), &b as &dyn Provider)]
            .into_iter()
            .collect();
    // Two devices, so the two bindings carry two descriptions (SB-3).
    let mut profile = distinct_instances(profile_binding(&["radio"]), &["radio"]);
    profile.bindings.insert(
        id("radio2"),
        ezsdr_kernel::binding::Binding {
            module: mid("ezsdr.test.provider"),
            feed: None,
            selector: [(id("instance"), Value::Str("radio2".to_owned()))].into_iter().collect(),
            profile: None,
        },
    );
    let result = validate(&spec, &profile, &fx.inputs(&providers)).expect("validates");
    assert!(matches!(
        result.into_result(),
        Err(SpecError::NoSingleInstance { name, constraint })
            if name == id("radio") && constraint.contains("Min")
    ));
}

#[test]
fn sb_34_sub_resource_binding() {
    let spec = spec_with(
        [(id("line"), resource("test.line", &[("test.count", Constraint::Eq { value: Value::Int(2) })]))]
            .into_iter()
            .collect(),
    );
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2);
    let providers = one_provider("line", &p);
    let result = validate(&spec, &profile_binding(&["line"]), &fx.inputs(&providers))
        .expect("validates");
    assert_eq!(result.matched[&id("line")], rid("radio/0"), "bound to the sub-resource's path");
}

#[test]
fn sb_34_two_resources_one_instance() {
    let spec = spec_with(
        [
            (id("a"), resource("test.device", &[])),
            (id("b"), resource("test.line", &[])),
        ]
        .into_iter()
        .collect(),
    );
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2);
    let providers: BTreeMap<Ident, &dyn Provider> =
        [(id("a"), &p as &dyn Provider), (id("b"), &p as &dyn Provider)].into_iter().collect();
    let result = validate(&spec, &profile_binding(&["a", "b"]), &fx.inputs(&providers))
        .expect("validates");
    assert_eq!(result.matched[&id("a")], rid("radio"));
    assert_eq!(result.matched[&id("b")], rid("radio/0"));
}

#[test]
fn sb_36_two_needs_of_the_same_name_do_not_collapse() {
    // SB-36's own example names a need `gpio`, which two peripherals would share.
    let need = |kind: &str| SubResourceReq {
        kind: ns(kind),
        requires: [(key("test.count"), Constraint::Eq { value: Value::Int(2) })]
            .into_iter()
            .collect(),
    };
    let mut a = resource("test.device", &[]);
    a.needs.insert(id("clk"), need("test.line"));
    let mut b = resource("test.device", &[]);
    b.needs.insert(id("clk"), need("test.line"));
    let spec = spec_with([(id("a"), a), (id("b"), b)].into_iter().collect());
    let fx = Fixture::new();
    // Two devices, because SB-34 refuses one exclusive root bound twice: the Spec
    // asked for two `test.device` resources, so the profile must bind two
    // instances. The rule this test is about — that two needs of the same name do
    // not collapse — is unaffected (finding D27).
    let p1 = TestProvider::new("radio", 2);
    let p2 = TestProvider::new("radio2", 2);
    let providers: BTreeMap<Ident, &dyn Provider> =
        [(id("a"), &p1 as &dyn Provider), (id("b"), &p2 as &dyn Provider)].into_iter().collect();
    let two = distinct_instances(profile_binding(&["a", "b"]), &["a", "b"]);
    let result = validate(&spec, &two, &fx.inputs(&providers)).expect("validates");
    assert_eq!(result.matched.len(), 4, "two bindings and two needs, not three entries");
    assert!(result.matched.contains_key(&id("a_clk")));
    assert!(result.matched.contains_key(&id("b_clk")));
    // And each need consumed its own line, rather than both resolving to one.
    assert_ne!(result.matched[&id("a_clk")], result.matched[&id("b_clk")]);
}

#[test]
fn sb_36_needs_resolves_across_instances() {
    let mut req = resource("test.device", &[]);
    req.needs.insert(
        id("line"),
        SubResourceReq {
            kind: ns("test.line"),
            requires: [(key("test.count"), Constraint::Eq { value: Value::Int(2) })].into_iter().collect(),
        },
    );
    let spec = spec_with([(id("peripheral"), req)].into_iter().collect());
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2);
    let providers = one_provider("peripheral", &p);
    let result = validate(&spec, &profile_binding(&["peripheral"]), &fx.inputs(&providers))
        .expect("validates");
    // SB-36 records the resolution in `matched`, keyed `<resource>_<need>` so that
    // two resources' needs of the same name do not collapse.
    assert_eq!(result.matched[&id("peripheral_line")], rid("radio/0"));

    // An unresolvable `needs` fails validate.
    let mut req = resource("test.device", &[]);
    req.needs.insert(
        id("line"),
        SubResourceReq {
            kind: ns("test.line"),
            requires: [(key("test.count"), Constraint::Eq { value: Value::Int(99) })].into_iter().collect(),
        },
    );
    let spec = spec_with([(id("peripheral"), req)].into_iter().collect());
    assert!(matches!(
        validate(&spec, &profile_binding(&["peripheral"]), &fx.inputs(&providers)),
        Err(SpecError::NoSingleInstance { .. })
    ));
}

#[test]
fn sb_37_matching_follows_binding() {
    // Two bound instances with different capabilities; the matcher uses the bound
    // instance's, not the union.
    let spec = spec_with(
        [(id("radio"), resource("test.device", &[("test.count", Constraint::Eq { value: Value::Int(8) })]))]
            .into_iter()
            .collect(),
    );
    let fx = Fixture::new();
    let small = TestProvider::new("small", 2);
    let big = TestProvider::new("big", 8);
    let bound_small = one_provider("radio", &small);
    let bound_big = one_provider("radio", &big);
    assert!(!validate(&spec, &profile_binding(&["radio"]), &fx.inputs(&bound_small))
        .expect("validates")
        .is_admitted());
    assert!(validate(&spec, &profile_binding(&["radio"]), &fx.inputs(&bound_big))
        .expect("validates")
        .is_admitted());
}

// ---------------------------------------------------------------- admission checks

#[test]
fn sb_30_admission_check_runs_at_three_points() {
    // SB-30's obligation is that the Kernel runs the registered checks at three
    // points, so the test has to go through those points. Calling
    // `AdmissionCheckRegistry::run` three times with three stage arguments proves only
    // that the registry dispatches on its own argument — the three call sites could
    // all be missing and it would still pass (D39).
    let ceiling = |max: f64| -> BTreeMap<Namespace, serde_json::Value> {
        [(ns("test.limits"), serde_json::json!({ "max_grid": max }))].into_iter().collect()
    };
    let asking = |v: f64| {
        let mut r = resource("test.device", &[]);
        r.requires.insert(key("test.grid"), Constraint::Eq { value: Value::Num(v) });
        let mut spec = spec_with([(id("radio"), r)].into_iter().collect());
        spec.policies.coercion.insert(key("test.grid"), CoercionPolicy::Accept);
        spec
    };
    let mut fx = Fixture::new();
    fx.checks.register(std::sync::Arc::new(TestLimitsCheck::new()));
    let p = TestProvider::new("radio", 2).with_grid(20.0);
    let providers = one_provider("radio", &p);

    // 1. `validate`, against the **requested** configuration.
    let mut profile = profile_binding(&["radio"]);
    profile.environment = ceiling(20.0);
    let over = validate(&asking(40.0), &profile, &fx.inputs(&providers)).expect("runs");
    assert!(!over.is_admitted(), "validate ran the check");
    assert_eq!(over.violations[0].check, ns("test.limits"));

    // 2. `collect_prepare`, against the **applied** one. 19.5 is inside the ceiling and
    //    the Provider snaps it to 20.0, which is not: the value only becomes illegal
    //    once it is applied, which is why SB-30 needs this second point at all.
    let mut tight = profile_binding(&["radio"]);
    tight.environment = ceiling(19.9);
    let spec = asking(19.5);
    let admission = validate(&spec, &tight, &fx.inputs(&providers)).expect("runs");
    assert!(admission.is_admitted(), "the requested value is inside the ceiling");
    let report = PrepareReport {
        fragment: id("radio"),
        effective: [(key("test.grid"), Value::Num(20.0))].into_iter().collect(),
        coercions: vec![ezsdr_kernel::spec::Coercion {
            key: key("test.grid"),
            requested: Value::Num(19.5),
            applied: Value::Num(20.0),
            reason: "snapped to a multiple of 20".to_owned(),
        }],
        warnings: Vec::new(),
    };
    let err = collect_prepare(vec![Ok(report)], &spec, &tight, &fx.inputs(&providers), &admission)
        .expect_err("prepare ran the check against the applied configuration");
    let ezsdr_kernel::plan::PrepareError::Violations(v) = err else { panic!("violations") };
    assert!(v.iter().any(|x| x.check == ns("test.limits")), "{v:?}");

    // 3. A Session Action, through the one admission path (RS-16, RS-17).
    //    OV-3: `Admitter` is constructed nowhere in `src/` — RS-17's runtime
    //    dispatcher is Phase 2's — so this point is driven from the test rather than
    //    from a Kernel call site, unlike the two above. That gap is D45.
    let env = ceiling(20.0);
    let classes: BTreeMap<Key, UpdateClass> =
        [(key("test.grid"), UpdateClass::BlockBoundary)]
            .into_iter()
            .collect();
    let spec_coercion = BTreeMap::new();
    let admitter = ezsdr_kernel::session::Admitter {
        checks: &fx.checks,
        environment: &env,
        declared_classes: &classes,
        spec_coercion: &spec_coercion,
        registry: &fx.registry,
        is_session: true,
    };
    let proposed: BTreeMap<Key, Value> =
        [(key("test.grid"), Value::Num(30.0))].into_iter().collect();
    let violations = admitter
        .admit(&BTreeMap::new(), &proposed, &[], CheckStage::Runtime)
        .expect_err("the runtime point ran the check");
    assert_eq!(violations[0].check, ns("test.limits"));
    assert_eq!(violations[0].requested, Some(Value::Num(30.0)));
}

#[test]
fn sb_31_unregistered_section_is_informational() {
    let checks = AdmissionCheckRegistry::new();
    let environment: BTreeMap<Namespace, serde_json::Value> =
        [(ns("vendor.thing"), serde_json::json!({ "anything": [1, 2, 3] }))].into_iter().collect();
    assert!(checks.run(&environment, &BTreeMap::new(), &BTreeMap::new(), CheckStage::Validate).is_empty());
    // And it is still recorded verbatim (SB-27) — in `environment`, which is what
    // the Manifest carries. `section()` is the **Kernel's** reader and serves only
    // SB-26's four names, so that a Kernel read of a Vocabulary's section is not
    // expressible (OV-21).
    let profile = BindingProfile { version: 1, environment: environment.clone(), ..BindingProfile::default() };
    assert_eq!(profile.environment.get(&ns("vendor.thing")), environment.get(&ns("vendor.thing")));
    assert!(profile.section("vendor.thing").is_none(), "not a section the Kernel reads");
    for name in ezsdr_kernel::binding::KERNEL_SECTIONS {
        assert!(profile.section(name).is_none(), "{name} is readable but absent here");
    }
}

// ---------------------------------------------------------------- the pipeline

#[test]
fn sb_39_arm_order_follows_edges() {
    let nodes = vec![id("a"), id("b"), id("c")];
    let edges = vec![(id("a"), id("b")), (id("a"), id("c"))];
    assert_eq!(arm_order(&nodes, &edges), Ok(vec![id("a"), id("b"), id("c")]));
    // Ties by name, whatever the input order.
    let nodes = vec![id("c"), id("b"), id("a")];
    assert_eq!(arm_order(&nodes, &edges), Ok(vec![id("a"), id("b"), id("c")]));
}

#[test]
fn sb_39_arm_cycle_refused() {
    let nodes = vec![id("a"), id("b")];
    let edges = vec![(id("a"), id("b")), (id("b"), id("a"))];
    assert!(matches!(arm_order(&nodes, &edges), Err(SpecError::ArmCycle { .. })));
}

#[test]
fn rs_08_reverse_dependency_order() {
    // Three fragments with `b after a`, `c after b`: released c, b, a (RS-8, SB-39).
    let nodes = vec![id("a"), id("b"), id("c")];
    let edges = vec![(id("a"), id("b")), (id("b"), id("c"))];
    let order = arm_order(&nodes, &edges).expect("acyclic");
    assert_eq!(order, vec![id("a"), id("b"), id("c")]);
    assert_eq!(release_order(&order), vec![id("c"), id("b"), id("a")]);
}

#[test]
fn sb_39_arm_after_from_the_provider_declaration() {
    // The device that sources PPS is armed first, because the other declares
    // `arm_after` on it.
    #[allow(unused_mut)]
    let spec = spec_with(
        [
            (id("pps"), resource("test.device", &[])),
            (id("slave"), resource("test.device", &[])),
        ]
        .into_iter()
        .collect(),
    );
    let fx = Fixture::new();
    let pps = TestProvider::new("pps", 2);
    let slave = TestProvider::new("slave", 2).arm_after(rid("pps"));
    let providers: BTreeMap<Ident, &dyn Provider> =
        [(id("pps"), &pps as &dyn Provider), (id("slave"), &slave as &dyn Provider)]
            .into_iter()
            .collect();
    let mut profile =
        distinct_instances(profile_binding(&["pps", "slave"]), &["pps", "slave"]);
    // SB-24: `authority` names a *binding*. The earlier `radio` here named none,
    // and `plan()` accepted it.
    profile.authority = Some(id("pps"));
    let mut fx = fx;
    fx.authorities.insert(
        id("pps"),
        AuthorityDescriptor { governs: Vec::new(), pacing: Pacing::FreeRunning },
    );
    let _ = &spec;
    let plan = validate_then_plan(&spec, &profile, &fx.inputs(&providers), Vec::new()).expect("plans");
    let names: Vec<&str> = plan.fragments.iter().map(|f| f.id.as_str()).collect();
    assert_eq!(names, vec!["pps", "slave"]);
    assert!(plan.deps.contains(&(id("pps"), id("slave"))));
}

#[test]
fn sb_24_authority_inferred_when_unique() {
    let spec = minimal_spec();
    let profile = profile_binding(&["radio"]);
    let mut fx = Fixture::new();
    let p = TestProvider::new("radio", 2);
    let providers = one_provider("radio", &p);
    let built = validate_then_plan(&spec, &profile, &fx.inputs(&providers), Vec::new()).expect("plans");
    assert_eq!(built.authority, id("radio"));

    // With two candidates and no field, refused. A candidate is a **binding** whose
    // Module declares the role (SB-24, MA-29), so the second one is bound here and not
    // merely handed in as a descriptor.
    let mut two = profile_binding(&["radio"]);
    two.bindings.insert(
        id("other"),
        ezsdr_kernel::binding::Binding {
            module: mid("ezsdr.test.provider"),
            feed: None,
            selector: BTreeMap::new(),
            profile: None,
        },
    );
    fx.authorities.insert(
        id("other"),
        AuthorityDescriptor { governs: Vec::new(), pacing: Pacing::FreeRunning },
    );
    assert!(matches!(
        validate_then_plan(&spec, &two, &fx.inputs(&providers), Vec::new()),
        Err(SpecError::Structural { .. })
    ));

    // SB-24 names "the binding whose Provider plays the Authority role (MA-29)", and
    // the role is the registry's declaration, as MA-25 reads a Sink's. Consulting the
    // runtime-supplied descriptor map alone let assembly-time input decide it — the
    // category D18 settled the other way for Providers, Sinks and Executors.
    let mut wrong_role = profile_binding(&["radio"]);
    wrong_role.authority = Some(id("exec")); // the Executor Module, which holds no Authority role
    fx.authorities.insert(
        id("exec"),
        AuthorityDescriptor { governs: Vec::new(), pacing: Pacing::FreeRunning },
    );
    assert!(
        matches!(
            validate_then_plan(&spec, &wrong_role, &fx.inputs(&providers), Vec::new()),
            Err(SpecError::WrongBindingRole { expected, .. }) if expected == "Authority"
        ),
        "a descriptor handed in at assembly time does not make a Module the Authority"
    );
}

#[test]
fn sb_39_a_provider_fragment_carries_the_matched_request() {
    // SB-44 and MA-12 require `prepare` to report the same coercions `coerce` did
    // for the same request. The fragment used to carry the binding's selector
    // alone, so no Provider could see a request at all and the MA-12 test had to
    // hand-build one (finding D32).
    let mut req = resource("test.device", &[]);
    req.requires.insert(key("test.count"), Constraint::Eq { value: Value::Int(2) });
    let spec = spec_with([(id("radio"), req)].into_iter().collect());
    let profile = profile_binding(&["radio"]);
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2);
    let providers = one_provider("radio", &p);
    let built = validate_then_plan(&spec, &profile, &fx.inputs(&providers), Vec::new())
        .expect("plans");

    let fragment = built.fragments.iter().find(|f| f.id == id("radio")).expect("a fragment");
    assert!(fragment.content.get("selector").is_some(), "the selector is still there");
    let requested: ezsdr_kernel::module_api::Requested =
        serde_json::from_value(fragment.content["requested"].clone()).expect("a request");
    // The node the matcher bound, and the constraints the Spec asked of it.
    assert_eq!(requested.resource, rid("radio"));
    assert_eq!(
        requested.constraints.get(&key("test.count")),
        Some(&Constraint::Eq { value: Value::Int(2) })
    );
}

#[test]
fn sb_40_transfer_cost_is_declared() {
    let spec = minimal_spec();
    let profile = profile_binding(&["radio"]);
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2);
    let providers = one_provider("radio", &p);
    let cost = DeclaredCost { link: DataLinkId::local(0), cost: 4_200 };
    let plan = validate_then_plan(&spec, &profile, &fx.inputs(&providers), vec![cost.clone()])
        .expect("plans");
    assert_eq!(plan.transfer_costs, vec![cost]);
    // No placement changed: the Core reports the number and never optimises.
    assert!(plan.fragments.iter().all(|f| f.role == Role::Provider));
}

#[test]
fn sb_41_prepare_report_per_fragment_and_merged() {
    let report = |name: &str, k: Option<(&str, Value)>| PrepareReport {
        fragment: id(name),
        effective: k.map(|(k, v)| (key(k), v)).into_iter().collect(),
        coercions: Vec::new(),
        warnings: Vec::new(),
    };
    let merged = MergedPrepare::from_reports(vec![
        report("a", Some(("test.count", Value::Int(2)))),
        report("b", Some(("test.flag", Value::Bool(true)))),
        report("c", None),
    ]);
    assert_eq!(merged.reports.len(), 3);
    assert_eq!(merged.effective.len(), 2);
    assert_eq!(merged.effective[&key("test.count")], Value::Int(2));
}

#[test]
fn sb_42_fragment_failure_fails_the_transaction() {
    let ok = |name: &str| PrepareReport {
        fragment: id(name),
        effective: BTreeMap::new(),
        coercions: Vec::new(),
        warnings: Vec::new(),
    };
    // The double's injected failure at `prepare`, without a full PrepareContext.
    let provider = TestProvider::new("c", 2).failing_at(FailAt::Prepare);
    assert_eq!(provider.fail_at, FailAt::Prepare);
    let err = ezsdr_kernel::module_api::ModuleError::rejected("injected failure at Prepare");
    let reports = vec![Ok(ok("a")), Ok(ok("b")), Err(err)];
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2);
    let providers = one_provider("radio", &p);
    let spec = minimal_spec();
    let failed = prepared(reports, &fx, &spec, &profile_binding(&["radio"]), &providers)
        .expect_err("the whole transaction fails");
    assert!(
        matches!(failed, ezsdr_kernel::plan::PrepareError::Fragment { index: 2, .. }),
        "the third fragment failed, and its failure is what fails the transaction: {failed:?}"
    );
    // Cleanup then releases in reverse dependency order (RS-8).
    let order = arm_order(&[id("a"), id("b"), id("c")], &[(id("a"), id("b")), (id("b"), id("c"))])
        .expect("acyclic");
    assert_eq!(release_order(&order), vec![id("c"), id("b"), id("a")]);
}

#[test]
fn sb_16_spec_time_resolves_at_arm() {
    // A schedule entry holding a TxBurst template at offset 1000 in a 20 Msps stream.
    let template = ActionTemplate::TxBurst {
        target: rid("radio/tx/0"),
        waveform: ezsdr_kernel::manifest::ArtifactRef {
            id: id("wave"),
            kind: ns("test.waveform"),
            uri: "memory://wave".to_owned(),
            hash: some_hash("wave"),
            size_bytes: 16,
            partial: false,
            marks: Vec::new(),
            continuity: Vec::new(),
        },
        repeat: false,
        late_policy: LatePolicy::SendAsapAndFlag,
        metadata: BTreeMap::new(),
    };
    let entry = ScheduleEntry {
        at: SpecTime { clock: id("radio"), offset_ticks: 1000 },
        action: template.clone(),
    };
    // The Spec validates with no time field: a Spec cannot name a ClockDomainId.
    let json = serde_json::to_value(&entry).expect("serialises");
    assert!(json["action"].get("at").is_none(), "the template carries no time (RS-49a)");
    assert!(template.is_timed());

    // SB-16: the `clock` names **a resource this Spec declares**. Nothing read
    // `spec.schedule` at all, so an entry whose clock was bound to nothing passed
    // `from_json`, `validate` and `plan` and left no trace in the plan.
    let mut spec = minimal_spec();
    spec.schedule.push(entry.clone());
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2);
    let providers = one_provider("radio", &p);
    assert!(
        validate(&spec, &profile_binding(&["radio"]), &fx.inputs(&providers)).is_ok(),
        "`radio` is a declared resource"
    );
    let mut dangling = minimal_spec();
    dangling.schedule.push(ScheduleEntry {
        at: SpecTime { clock: id("no_such_resource"), offset_ticks: 1000 },
        action: template.clone(),
    });
    let err = validate(&dangling, &profile_binding(&["radio"]), &fx.inputs(&providers))
        .expect_err("the clock names nothing");
    assert!(
        matches!(&err, SpecError::Structural { reason } if reason.contains("SB-16")),
        "{err:?}"
    );

    // RS-49a: `resolve` substitutes a deadline into the template. Computing that
    // deadline from the `SpecTime` is `arm`'s job, and there is no Kernel `arm` in
    // Phase 1 — SB-43 marks that half a forward obligation, so this asserts only the
    // substitution, with the deadline supplied.
    let sample_clock = ClockDomainId::local(7);
    let deadline = AbsoluteDeadline::new(TimePoint::new(sample_clock, 1000));
    let action = template.resolve(deadline);
    match action {
        ezsdr_kernel::event::Action::TxBurst { at, .. } => {
            assert_eq!(at, deadline);
            assert_eq!(at.time_point.domain, sample_clock);
        }
        other => panic!("expected a TxBurst, got {other:?}"),
    }
}

#[test]
fn sb_22_a_session_profile_binds_its_recorder() {
    // SB-25a is withdrawn (OV-1 keeps the number): a Sink is a Module **role**, not
    // a component an Executor loads, so a Session profile *binds* its recorder and
    // RS-12 turns that binding into the implicit Spec's output. The placed-component
    // version could not express the link that feeds it, so a Session's capture
    // recorded nothing (findings D17, N6).
    let reg = registry();
    let mut profile = profile_binding(&["radio"]);
    profile.bindings.insert(
        id("recorder"),
        ezsdr_kernel::binding::Binding {
            module: mid("ezsdr.test.sink"),
            feed: Some(ezsdr_kernel::spec::SinkFeed {
                port: PortRef { component: "radio".into(), port: "rx".into() },
                policy: BackPressure::DropOldest,
                capacity: 4,
            }),
            selector: BTreeMap::new(),
            profile: None,
        },
    );
    let sink = TestSink::new(
        ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.cf32").expect("id"),
    );
    let sinks: BTreeMap<Ident, &dyn ezsdr_kernel::module_api::Sink> =
        [(id("recorder"), &sink as &dyn ezsdr_kernel::module_api::Sink)].into_iter().collect();
    let p = TestProvider::new("radio", 2);
    let providers = one_provider("radio", &p);
    let implicit = ezsdr_kernel::session::implicit_spec(&profile, &reg, &providers, &sinks)
        .expect("builds the implicit Spec");
    assert_eq!(implicit.version, 1);
    assert_eq!(implicit.resources[&id("radio")].requires.len(), 0, "empty requires (RS-12)");

    // The recorder is an output, not a component, and it carries its own link.
    assert!(implicit.graph.components.is_empty(), "a Sink is never a graph component");
    assert_eq!(implicit.outputs.len(), 1);
    let output = &implicit.outputs[0];
    assert_eq!(output.id, id("recorder"));
    assert_eq!(output.kind, ns("test.capture"), "an artifact kind the Sink writes");
    assert_eq!(output.feed.port, PortRef { component: "radio".into(), port: "rx".into() });
    assert_eq!(output.feed.capacity, 4);

    // A Sink binding with no `feed` has no port to record, and is refused rather
    // than producing an output that records nothing.
    let mut feedless = profile.clone();
    feedless.bindings.get_mut(&id("recorder")).expect("bound").feed = None;
    assert!(matches!(
        ezsdr_kernel::session::implicit_spec(&feedless, &reg, &providers, &sinks),
        Err(SpecError::Structural { .. })
    ));
}

#[test]
fn sb_15_link_policy_is_mandatory() {
    // Both fields are mandatory in the type itself: a link with no policy or no
    // capacity does not parse.
    let no_policy = serde_json::json!({
        "from": { "component": "a", "port": "out" },
        "to": { "component": "b", "port": "in" },
        "capacity": 4
    });
    assert!(serde_json::from_value::<ezsdr_kernel::spec::LinkReq>(no_policy).is_err());
    let no_capacity = serde_json::json!({
        "from": { "component": "a", "port": "out" },
        "to": { "component": "b", "port": "in" },
        "policy": "block"
    });
    assert!(serde_json::from_value::<ezsdr_kernel::spec::LinkReq>(no_capacity).is_err());
}

#[test]
fn sb_15_link_contract_and_sink_policy() {
    let contracts = ContractRegistry::with_standard_contracts();
    let cf32 = ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.cf32").expect("id");
    let sc16 = ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.sc16").expect("id");
    assert!(contracts.check_link(&cf32, &sc16).is_err(), "mismatched contracts (SC-3)");

    let decl = ezsdr_kernel::stream::DataLinkDecl {
        id: DataLinkId::local(0),
        from: PortRef { component: "rx".into(), port: "out".into() },
        to: PortRef { component: "recorder".into(), port: "in".into() },
        contract: cf32,
        policy: BackPressure::Block,
        capacity: 4,
    };
    assert!(ezsdr_kernel::stream::check_sink_link(&decl, true).is_err(), "Block into a Sink (SC-21)");
}

#[test]
fn sb_15_a_bound_resource_port_is_a_link_endpoint() {
    // Vision §7's own correct diagram is `PHY Processor -> SampleStream -> Radio
    // Port`. Before a Resource declared its Ports, a `PortRef` naming a resource
    // resolved to no contract and `plan()` refused the link twice — once as an
    // MA-22 cycle and once as "touches an unplaced component" — so a Spec could not
    // connect a Provider's stream to its graph at all (finding D31).
    //
    // SB-15 says the port is one the **bound node** declares. The double's device
    // root and its lines both declare a port named `rx` with *different* contracts
    // (`cf32` on the root, `sc16` on a line), so this test can tell which node the
    // contract came from: a resource of kind `test.line` binds a line, and its `rx`
    // must resolve to `sc16`.
    let sc16 = ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.sc16").expect("id");
    let cf32 = ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.cf32").expect("id");
    let link_into = |consumer: &ezsdr_kernel::contract::DataContractId| {
        let mut spec = minimal_spec();
        spec.resources.insert(id("line0"), resource("test.line", &[]));
        spec.graph.components.insert(id("proc"), support::recorder_component(consumer.clone()));
        spec.graph.links.push(ezsdr_kernel::spec::LinkReq {
            from: PortRef { component: "line0".into(), port: "rx".into() },
            to: PortRef { component: "proc".into(), port: "in".into() },
            policy: BackPressure::DropOldest,
            capacity: 4,
        });
        spec
    };
    let mut profile = profile_binding(&["radio", "line0"]);
    profile.placements.components.insert(
        id("proc"),
        ComponentPlacement { island: id("io"), memory_domain: MemoryDomainId::local(0) },
    );
    profile.placements.islands.push(IslandDecl {
        id: IslandId::local(0),
        executor: id("exec"),
        components: vec![id("proc")],
        affinity: None,
        rt_policy: None,
        batch: None,
    });
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2);
    let providers: BTreeMap<Ident, &dyn Provider> =
        [(id("radio"), &p as &dyn Provider), (id("line0"), &p as &dyn Provider)]
            .into_iter()
            .collect();

    // A consumer that takes `sc16` matches the **line's** contract, so it plans.
    let planned = validate_then_plan(&link_into(&sc16), &profile, &fx.inputs(&providers), Vec::new());
    assert!(planned.is_ok(), "a resource endpoint is admissible: {planned:?}");

    // A consumer that takes `cf32` matches the **root's** contract. It must be
    // refused by SC-3: accepting it would mean the port was resolved on a node the
    // resource is not bound to — a link the Kernel calls compatible and the hardware
    // will not honour.
    let wrong_contract =
        validate(&link_into(&cf32), &profile, &fx.inputs(&providers)).expect_err("SC-3 refuses it");
    // The message names `sc16` as the producer, which is the proof: the contract came
    // from the bound line, not from the root the resource is not bound to.
    assert!(
        matches!(&wrong_contract, SpecError::Structural { reason }
            if reason.contains("ezsdr.stream.sc16") && reason.contains("not accepted")),
        "{wrong_contract:?}"
    );

    // SB-15: an endpoint that names no declared port is refused, on a resource as on
    // a component.
    let mut wrong_port = link_into(&sc16);
    // A name the node declares no port under. Not `tx`: the double declares one, for
    // the `PHY -> Radio Port` direction, and a fixture that names a real port would
    // stop testing SB-15's refusal.
    wrong_port.graph.links[0].from.port = "no_such_port".to_owned();
    assert!(matches!(
        validate(&wrong_port, &profile, &fx.inputs(&providers)),
        Err(SpecError::Structural { .. })
    ));
}

#[test]
fn sb_17_capture_without_a_sink_refused() {
    let mut spec = minimal_spec();
    spec.outputs.push(ezsdr_kernel::spec::OutputReq {
        id: id("capture0"),
        kind: ns("test.capture"),
        feed: ezsdr_kernel::spec::SinkFeed {
            port: PortRef { component: "radio".into(), port: "rx".into() },
            policy: BackPressure::DropOldest,
            capacity: 4,
        },
        params: BTreeMap::new(),
    });
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2);
    let providers = one_provider("radio", &p);
    // SB-22: the output id has no binding, so there is no Sink to serve it. This is
    // now checkable on the **Spec** path, which is what D17 was about.
    assert!(matches!(
        validate(&spec, &profile_binding(&["radio"]), &fx.inputs(&providers)),
        Err(SpecError::Structural { .. })
    ));
}

#[test]
fn sb_18_unregistered_event_kind_refused() {
    let mut spec = minimal_spec();
    spec.policies.failure.insert(
        ezsdr_kernel::event::EventKind::parse("RX_OVERFLOWS").expect("parses"),
        Reaction::Stop,
    );
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2);
    let providers = one_provider("radio", &p);
    let err = validate(&spec, &profile_binding(&["radio"]), &fx.inputs(&providers))
        .expect_err("refused");
    assert!(err.to_string().contains("RX_OVERFLOWS"), "the refusal names the kind: {err}");
}

#[test]
fn sb_27_environment_portability() {
    // One Spec, two BindingProfiles differing only in `environment`: both compile,
    // the Spec hash is equal and the binding hashes differ.
    let spec = minimal_spec();
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2);
    let providers = one_provider("radio", &p);

    let mut sim = profile_binding(&["radio"]);
    sim.environment.insert(ns("ezsdr.rf_path"), serde_json::json!({ "path": "simulated" }));
    let mut lab = profile_binding(&["radio"]);
    lab.environment.insert(ns("ezsdr.rf_path"), serde_json::json!({ "path": "simulated" }));
    lab.environment.insert(ns("sim.channel"), serde_json::json!({ "model": "awgn" }));

    assert!(validate(&spec, &sim, &fx.inputs(&providers)).expect("validates").is_admitted());
    assert!(validate(&spec, &lab, &fx.inputs(&providers)).expect("validates").is_admitted());
    assert_eq!(
        ContentHash::of(&spec).expect("hashes"),
        ContentHash::of(&spec).expect("hashes"),
        "the Spec hash does not depend on the profile"
    );
    assert_ne!(
        ContentHash::of(&sim).expect("hashes"),
        ContentHash::of(&lab).expect("hashes"),
        "the binding hashes differ"
    );
}

#[test]
fn ma_41_execution_class_table() {
    use ezsdr_kernel::module_api::{ExecutionClass, RfPath};
    assert_eq!(
        ExecutionClass::derive(Pacing::FreeRunning, RfPath::Simulated),
        Ok(ExecutionClass::Simulation)
    );
    assert_eq!(
        ExecutionClass::derive(Pacing::WallPaced, RfPath::Simulated),
        Ok(ExecutionClass::RealtimeEmulation)
    );
    assert_eq!(
        ExecutionClass::derive(Pacing::Device, RfPath::Cabled),
        Ok(ExecutionClass::HardwareInLoop)
    );
    assert_eq!(
        ExecutionClass::derive(Pacing::Device, RfPath::OverTheAir),
        Ok(ExecutionClass::Hardware)
    );
    // A simulated Authority cannot drive a real RF path.
    assert!(ExecutionClass::derive(Pacing::FreeRunning, RfPath::OverTheAir).is_err());
    assert!(ExecutionClass::derive(Pacing::WallPaced, RfPath::Cabled).is_err());
    assert!(!ExecutionClass::Hardware.may_claim_determinism());
    assert!(ExecutionClass::Simulation.may_claim_determinism());
}

#[test]
fn rs_12_a_session_compiles_through_the_whole_pipeline() {
    // RS-12: `connect()` runs the whole pipeline of SB-37, and the implicit Spec
    // must survive every stage of it — not just `validate`.
    let sink = TestSink::new(
        ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.cf32").expect("id"),
    );
    let sinks: BTreeMap<Ident, &dyn ezsdr_kernel::module_api::Sink> =
        [(id("recorder"), &sink as &dyn ezsdr_kernel::module_api::Sink)].into_iter().collect();
    let mut fx = Fixture::new();
    fx.is_session = true;
    fx.sinks = sinks.clone();
    let mut profile = profile_binding(&["radio"]);
    // The recorder is bound, not placed, so the profile owes no Island for it.
    profile.bindings.insert(
        id("recorder"),
        ezsdr_kernel::binding::Binding {
            module: mid("ezsdr.test.sink"),
            feed: Some(ezsdr_kernel::spec::SinkFeed {
                port: PortRef { component: "radio".into(), port: "rx".into() },
                policy: BackPressure::DropOldest,
                capacity: 4,
            }),
            selector: BTreeMap::new(),
            profile: None,
        },
    );
    let p = TestProvider::new("radio", 2);
    let providers = one_provider("radio", &p);
    let spec = ezsdr_kernel::session::implicit_spec(&profile, &fx.registry, &providers, &sinks)
        .expect("builds the implicit Spec");
    let result = validate(&spec, &profile, &fx.inputs(&providers)).expect("a Session validates");
    assert!(result.is_admitted());
    let built = validate_then_plan(&spec, &profile, &fx.inputs(&providers), Vec::new())
        .expect("a Session plans");
    // MA-25, MA-30: the Sink is its own fragment, prepared and stepped in its own
    // right, and never a component inside an Island.
    assert!(built.fragments.iter().any(|f| f.role == Role::Sink && f.id == id("recorder")));
    assert_eq!(built.class, ezsdr_kernel::module_api::ExecutionClass::Simulation);
}

#[test]
fn ma_39_plan_admits_a_real_graph_and_refuses_a_misplaced_component() {
    // The admission calls are wired into `plan()`; proved on a non-empty graph, not
    // only on the degenerate one every other `plan()` test uses.
    let mut fx = Fixture::new();
    let _ = &mut fx;
    let contract = ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.cf32").expect("id");
    let mut spec = minimal_spec();
    // `proc` produces and `recorder` consumes: SB-15a checks the directions, so the
    // producer needs an `out` port. Before SB-15a this fixture linked `in` to `in`.
    spec.graph.components.insert(id("proc"), support::source_component(contract.clone()));
    spec.graph.components.insert(id("recorder"), support::recorder_component(contract));
    spec.graph.links.push(ezsdr_kernel::spec::LinkReq {
        from: PortRef { component: "proc".into(), port: "out".into() },
        to: PortRef { component: "recorder".into(), port: "in".into() },
        policy: BackPressure::DropOldest,
        capacity: 4,
    });
    let mut profile = profile_binding(&["radio"]);
    for c in ["proc", "recorder"] {
        profile.placements.components.insert(
            id(c),
            ComponentPlacement {
                island: id("io"),
                memory_domain: MemoryDomainId::local(0),
            },
        );
    }
    profile.placements.islands.push(IslandDecl {
        id: IslandId::local(0),
        executor: id("exec"),
        components: vec![id("proc"), id("recorder")],
        affinity: None,
        rt_policy: None,
        batch: None,
    });
    let p = TestProvider::new("radio", 2);
    let providers = one_provider("radio", &p);
    let planned = validate_then_plan(&spec, &profile, &fx.inputs(&providers), Vec::new());
    assert!(planned.is_ok(), "{planned:?}");

    // MA-39, through `plan()`: a component the Islands do not place is refused.
    let mut unplaced = profile.clone();
    unplaced.placements.islands[0].components = vec![id("proc")];
    assert!(matches!(
        validate_then_plan(&spec, &unplaced, &fx.inputs(&providers), Vec::new()),
        Err(SpecError::Structural { reason }) if reason.contains("exactly once")
    ));

    // SC-21, through `plan()`: a `Block` feed into a Sink is refused. The rule now
    // bites on the **output's** own link, because a Sink is bound rather than placed
    // and so is never a graph link's consumer (SB-17, finding D17).
    let sink = TestSink::new(
        ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.cf32").expect("id"),
    );
    let sinks: BTreeMap<Ident, &dyn ezsdr_kernel::module_api::Sink> =
        [(id("capture0"), &sink as &dyn ezsdr_kernel::module_api::Sink)].into_iter().collect();
    let mut blocking = spec.clone();
    blocking.outputs.push(ezsdr_kernel::spec::OutputReq {
        id: id("capture0"),
        kind: ns("test.capture"),
        feed: ezsdr_kernel::spec::SinkFeed {
            port: PortRef { component: "radio".into(), port: "rx".into() },
            policy: BackPressure::Block,
            capacity: 4,
        },
        params: BTreeMap::new(),
    });
    let mut bound_sink = profile.clone();
    bound_sink.bindings.insert(
        id("capture0"),
        ezsdr_kernel::binding::Binding {
            module: mid("ezsdr.test.sink"),
            feed: None,
            selector: BTreeMap::new(),
            profile: None,
        },
    );
    let mut fx2 = Fixture::new();
    fx2.sinks = sinks;
    let refused = validate_then_plan(&blocking, &bound_sink, &fx2.inputs(&providers), Vec::new());
    assert!(
        matches!(&refused, Err(SpecError::Structural { reason }) if reason.contains("SC-21")),
        "{refused:?}"
    );
}

#[test]
fn ma_41_an_unparseable_rf_path_is_refused_not_defaulted() {
    // Defaulting turns a misspelled `over_the_air` into `Simulation`, the one class
    // that may claim determinism (RS-42).
    let spec = minimal_spec();
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2);
    let providers = one_provider("radio", &p);
    let mut profile = profile_binding(&["radio"]);
    profile.environment.insert(ns("ezsdr.rf_path"), serde_json::json!({ "path": "over_the_air " }));
    assert!(matches!(
        validate_then_plan(&spec, &profile, &fx.inputs(&providers), Vec::new()),
        Err(SpecError::Structural { reason }) if reason.contains("ezsdr.rf_path")
    ));
    // The correct spelling is refused for the right reason instead (MA-41's table).
    profile.environment.insert(ns("ezsdr.rf_path"), serde_json::json!({ "path": "over_the_air" }));
    assert!(matches!(
        validate_then_plan(&spec, &profile, &fx.inputs(&providers), Vec::new()),
        Err(SpecError::Structural { reason }) if reason.contains("MA-41")
    ));
}

#[test]
fn sb_34_two_resources_take_different_sub_resources() {
    // SB-34: "Two Spec resources may bind to **different** sub-resources of one
    // instance." Binding both to the first kind match hands one physical channel to
    // two resources with no diagnostic.
    let spec = spec_with(
        [
            (id("a"), resource("test.line", &[])),
            (id("b"), resource("test.line", &[])),
        ]
        .into_iter()
        .collect(),
    );
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2);
    let providers: BTreeMap<Ident, &dyn Provider> =
        [(id("a"), &p as &dyn Provider), (id("b"), &p as &dyn Provider)].into_iter().collect();
    let result = validate(&spec, &profile_binding(&["a", "b"]), &fx.inputs(&providers))
        .expect("validates");
    assert_ne!(result.matched[&id("a")], result.matched[&id("b")], "not double-booked");
    assert_eq!(result.matched[&id("a")], rid("radio/0"));
    assert_eq!(result.matched[&id("b")], rid("radio/1"));
}

#[test]
fn sb_34_exclusive_node_bound_twice_is_refused() {
    // SB-34: a node is bound by at most one Spec resource unless the Provider
    // declares it shareable. Three lines asked of a two-line device is not
    // satisfiable, and resolving it by sharing hands one physical channel to two
    // intents with no diagnostic — the failure D27 was raised about.
    let spec = spec_with(
        [
            (id("a"), resource("test.line", &[])),
            (id("b"), resource("test.line", &[])),
            (id("c"), resource("test.line", &[])),
        ]
        .into_iter()
        .collect(),
    );
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2);
    let providers: BTreeMap<Ident, &dyn Provider> = [
        (id("a"), &p as &dyn Provider),
        (id("b"), &p as &dyn Provider),
        (id("c"), &p as &dyn Provider),
    ]
    .into_iter()
    .collect();
    let err = validate(&spec, &profile_binding(&["a", "b", "c"]), &fx.inputs(&providers))
        .expect_err("two lines cannot serve three resources");
    let SpecError::NodeAlreadyBound { node, first, second } = err else {
        panic!("expected NodeAlreadyBound, got {err:?}")
    };
    // The diagnostic names both intents that collided, not just "no instance".
    assert_eq!(node, rid("radio/0").to_string());
    assert_eq!(first, id("a"));
    assert_eq!(second, id("c"));
}

#[test]
fn sb_34_shareable_node_may_be_bound_twice() {
    // Which kinds are shareable is the Provider's declaration, never the Kernel's
    // knowledge: Vision §8's GPIO banks "may share the timekeeper" of §39, so the
    // Kernel enforces a flag it does not interpret (MA-10, OV-21).
    let spec = spec_with(
        [
            (id("a"), resource("test.line", &[])),
            (id("b"), resource("test.line", &[])),
            (id("c"), resource("test.line", &[])),
        ]
        .into_iter()
        .collect(),
    );
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2).with_shareable_lines();
    let providers: BTreeMap<Ident, &dyn Provider> = [
        (id("a"), &p as &dyn Provider),
        (id("b"), &p as &dyn Provider),
        (id("c"), &p as &dyn Provider),
    ]
    .into_iter()
    .collect();
    let result = validate(&spec, &profile_binding(&["a", "b", "c"]), &fx.inputs(&providers))
        .expect("the Provider declared them shareable");
    assert_eq!(result.matched.len(), 3);
    // The preference for a free node still holds: sharing is what is allowed, not
    // what is chosen first.
    assert_eq!(result.matched[&id("a")], rid("radio/0"));
    assert_eq!(result.matched[&id("b")], rid("radio/1"));
    assert_eq!(result.matched[&id("c")], rid("radio/0"));
}

#[test]
fn sb_34_binds_the_sub_resource_that_can_satisfy_the_request() {
    // A constraint only the *second* channel satisfies must not be rejected because
    // the first one was picked by position.
    let spec = spec_with(
        [(id("line"), resource("test.line", &[("test.count", Constraint::Eq { value: Value::Int(8) })]))]
            .into_iter()
            .collect(),
    );
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2).with_asymmetric_lines(2, 8);
    let providers = one_provider("line", &p);
    let result = validate(&spec, &profile_binding(&["line"]), &fx.inputs(&providers))
        .expect("validates");
    assert!(result.is_admitted(), "rejected: {:?}", result.rejected);
    assert_eq!(result.matched[&id("line")], rid("radio/1"));
}

#[test]
fn sb_46_validate_applies_the_coercion_policy() {
    // SB-38 makes `validate` the dry run; a coercion its own policy will reject at
    // `prepare` must fail here, not be previewed and passed.
    let spec = spec_with(
        [(id("radio"), resource("test.device", &[("test.grid", Constraint::Eq { value: Value::Num(19.5) })]))]
            .into_iter()
            .collect(),
    );
    let fx = Fixture::new(); // `test.grid`'s KeyDecl defaults to `reject`
    let p = TestProvider::new("radio", 2).with_grid(20.0);
    let providers = one_provider("radio", &p);
    let result = validate(&spec, &profile_binding(&["radio"]), &fx.inputs(&providers))
        .expect("validates");
    assert!(!result.is_admitted(), "a `reject` coercion fails the dry run");
    assert_eq!(result.violations.len(), 1);
    assert!(result.violations[0].reason.contains("SB-46"));

    // As a Session the same coercion warns, per SB-45's chain.
    let mut session = Fixture::new();
    session.is_session = true;
    let ok = validate(&spec, &profile_binding(&["radio"]), &session.inputs(&providers))
        .expect("validates");
    assert!(ok.is_admitted());
    assert_eq!(ok.warnings.len(), 1);
}

#[test]
fn sb_30_prepare_runs_the_checks_against_the_applied_configuration() {
    // SB-30's second point: a coercion can move an applied value outside a limit the
    // requested value respected, and `collect_prepare` is where that is caught.
    let mut checks = AdmissionCheckRegistry::new();
    checks.register(std::sync::Arc::new(TestLimitsCheck::new()));
    let environment: BTreeMap<Namespace, serde_json::Value> =
        [(ns("test.limits"), serde_json::json!({ "max_grid": 35.0 }))].into_iter().collect();
    let mut fx = Fixture::new();
    fx.checks = checks;
    let mut profile = profile_binding(&["radio"]);
    profile.environment = environment;
    let p = TestProvider::new("radio", 2).with_grid(20.0);
    let providers = one_provider("radio", &p);
    // SB-30's second point is *about* a coerced value: "a coercion can move an applied
    // value outside a limit that the requested value respected". Moving the fixture
    // away from a coerced value would remove the rule from the test — which is how
    // P0-3 hid. The coercion must also be one the Provider's own `coerce` produces, or
    // SB-44's check counts as a second violation: with a grid of 20, 19.5 snaps to 20
    // (inside `max_grid: 35`) and 30 snaps to 40 (outside it).
    let spec_for = |requested: f64| {
        let mut spec =
            spec_with([(id("radio"), resource("test.device", &[]))].into_iter().collect());
        spec.resources.get_mut(&id("radio")).expect("present").requires
            .insert(key("test.grid"), Constraint::Eq { value: Value::Num(requested) });
        spec.policies.coercion.insert(key("test.grid"), CoercionPolicy::Accept);
        spec
    };
    let coerced = |requested: f64, applied: f64| PrepareReport {
        fragment: id("radio"),
        effective: [(key("test.grid"), Value::Num(applied))].into_iter().collect(),
        coercions: vec![ezsdr_kernel::spec::Coercion {
            key: key("test.grid"),
            requested: Value::Num(requested),
            applied: Value::Num(applied),
            reason: "snapped".to_owned(),
        }],
        warnings: Vec::new(),
    };
    let spec = spec_for(19.5);
    assert!(prepared(vec![Ok(coerced(19.5, 20.0))], &fx, &spec, &profile, &providers).is_ok());
    let spec = spec_for(30.0);
    let failed = prepared(vec![Ok(coerced(30.0, 40.0))], &fx, &spec, &profile, &providers)
        .expect_err("the applied value is outside the limit");
    assert!(matches!(failed, ezsdr_kernel::plan::PrepareError::Violations(v) if v.len() == 1));

    // SB-46 names "the stage", not "the validate stage": a coercion on a `reject`
    // key reported by `prepare` is refused here too.
    let coercing = PrepareReport {
        fragment: id("radio"),
        effective: BTreeMap::new(),
        coercions: vec![ezsdr_kernel::spec::Coercion {
            key: key("test.grid"),
            requested: Value::Num(19.5),
            applied: Value::Num(20.0),
            reason: "snapped".to_owned(),
        }],
        warnings: Vec::new(),
    };
    // SB-46 names "the stage", not "the validate stage". To reach the **prepare**
    // stage the Spec must pass `validate` — so it asks for a value the node satisfies
    // **directly** (20.0 is in the declared grid), which means `coerce` is never
    // called and nothing is previewed — and the Provider then declares a coercion at
    // `prepare`. The key is requested, so the report is well formed (SB-44, D42), and
    // `test.grid`'s Vocabulary default is `reject`.
    let bare = profile_binding(&["radio"]);
    let mut defaulted =
        spec_with([(id("radio"), resource("test.device", &[]))].into_iter().collect());
    defaulted.resources.get_mut(&id("radio")).expect("present").requires
        .insert(key("test.grid"), Constraint::Eq { value: Value::Num(20.0) });
    let failed = prepared(vec![Ok(coercing.clone())], &fx, &defaulted, &bare, &providers)
        .expect_err("`test.grid` defaults to `reject`");
    assert!(
        matches!(&failed, ezsdr_kernel::plan::PrepareError::Violations(v)
            if v.iter().any(|x| x.reason.contains("rejected at prepare"))),
        "{failed:?}"
    );
    // As a Session the same coercion warns (SB-45) — and SB-46 says `warn` means
    // applied, recorded **and warned**, so the warning has to land somewhere. It used
    // to be computed and dropped, so every Session coercion recorded a change and
    // warned about nothing.
    let mut as_session = Fixture::new();
    as_session.is_session = true;
    let merged = prepared(vec![Ok(coercing)], &as_session, &defaulted, &bare, &providers)
        .expect("a Session warns rather than refusing");
    assert_eq!(merged.reports[0].warnings.len(), 1, "SB-46's warning is on the report");
    assert_eq!(merged.reports[0].coercions.len(), 1, "and the coercion is still recorded");
}

#[test]
fn ma_41_a_declared_time_class_that_disagrees_is_refused() {
    // SB-26 lists `ezsdr.time` as a section the Kernel reads and MA-41's argument is
    // that "a class that was merely declared could lie" — which is only a rule if the
    // declaration is compared with the derivation. Nothing read the section at all.
    let spec = minimal_spec();
    let mut profile = profile_binding(&["radio"]);
    profile.environment.insert(ns("ezsdr.time"), serde_json::json!({ "class": "hardware" }));
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2);
    let providers = one_provider("radio", &p);
    // The Authority is free-running and `ezsdr.rf_path` is absent, so the derived
    // class is Simulation; the declaration says Hardware.
    let err = validate_then_plan(&spec, &profile, &fx.inputs(&providers), Vec::new())
        .expect_err("a declared class that disagrees is refused");
    assert!(
        matches!(&err, SpecError::Structural { reason }
            if reason.contains("MA-41") && reason.contains("ezsdr.time")),
        "{err:?}"
    );
    // The agreeing declaration plans, and so does no declaration at all.
    profile.environment.insert(ns("ezsdr.time"), serde_json::json!({ "class": "simulation" }));
    assert!(validate_then_plan(&spec, &profile, &fx.inputs(&providers), Vec::new()).is_ok());
    profile.environment.remove(&ns("ezsdr.time"));
    assert!(validate_then_plan(&spec, &profile, &fx.inputs(&providers), Vec::new()).is_ok());
}

#[test]
fn ov_16_a_non_ascii_key_is_refused_at_ingestion() {
    // OV-15's canonicaliser refuses a non-ASCII object key, and nothing refused one
    // at ingestion: a Spec carrying one in an opaque section validated, planned,
    // armed and transmitted, and `Manifest::seal()` then failed at cleanup step 8 —
    // a Run that radiated and produced no Manifest, against RS-11.
    let doc = serde_json::json!({
        "version": 1,
        "extensions": { "vendor.thing": { "\u{3c1}": 1 } }
    });
    let err = ExperimentSpec::from_json(&doc).expect_err("refused at ingestion");
    assert!(matches!(&err, SpecError::UnknownField { path } if path.contains("OV-15")), "{err:?}");

    // The same document with an ASCII key parses, and its canonical form exists —
    // which is the property the refusal protects.
    let ok = serde_json::json!({
        "version": 1,
        "extensions": { "vendor.thing": { "rho": 1 } }
    });
    let spec = ExperimentSpec::from_json(&ok).expect("parses");
    assert!(ezsdr_kernel::hash::ContentHash::of(&spec).is_ok());
}

#[test]
fn sb_30_a_validate_violation_refuses_the_plan() {
    // SB-30: "A non-empty violation list fails the stage, and nothing transmits until
    // every check at every applicable stage has passed." `validate` *reports*, because
    // SB-38 requires the rejections in the Manifest, so the refusal has to happen in
    // `plan()` — where the next thing produced is an armable plan. It did not, and
    // `AdmissionResult::into_result` had no caller anywhere in `src/`, so an
    // RF-envelope refusal raised at the validate point yielded a complete plan
    // (Vision invariant 42).
    let mut req = resource("test.device", &[]);
    req.requires.insert(key("test.grid"), Constraint::Eq { value: Value::Num(500.0) });
    let spec = spec_with([(id("radio"), req)].into_iter().collect());
    let mut profile = profile_binding(&["radio"]);
    profile.environment.insert(ns("test.limits"), serde_json::json!({ "max_grid": 100.0 }));
    let mut fx = Fixture::new();
    fx.checks.register(std::sync::Arc::new(TestLimitsCheck::new()));
    let p = TestProvider::new("radio", 2).with_grid(20.0);
    let providers = one_provider("radio", &p);

    let admission = validate(&spec, &profile, &fx.inputs(&providers)).expect("validate reports");
    assert!(!admission.is_admitted(), "the check refused it");
    assert!(!admission.violations.is_empty(), "and the violation is recorded for SB-38");

    // The same result must not yield a plan.
    let refused = plan(&spec, &profile, &admission, &fx.inputs(&providers), Vec::new())
        .expect_err("SB-30: nothing transmits after a violation");
    assert!(matches!(refused, SpecError::Violation(_)), "{refused:?}");
}

#[test]
fn sb_22_one_namespace_refuses_a_collision() {
    // SB-22: resource names, output ids and Island executor names form one namespace.
    // Unchecked, a resource and an output could share a name and `plan()` emitted a
    // single fragment for the two, leaving the Spec's declared resource never
    // prepared, armed, stopped or recorded in the Manifest — with the Run admitted.
    let sink = TestSink::new(
        ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.cf32").expect("id"),
    );
    let sinks: BTreeMap<Ident, &dyn ezsdr_kernel::module_api::Sink> =
        [(id("cap"), &sink as &dyn ezsdr_kernel::module_api::Sink)].into_iter().collect();
    let mut spec = spec_with([(id("cap"), resource("test.device", &[]))].into_iter().collect());
    spec.outputs.push(ezsdr_kernel::spec::OutputReq {
        id: id("cap"),
        kind: ns("test.capture"),
        feed: ezsdr_kernel::spec::SinkFeed {
            port: PortRef { component: "cap".into(), port: "rx".into() },
            policy: BackPressure::DropOldest,
            capacity: 4,
        },
        params: BTreeMap::new(),
    });
    let mut fx = Fixture::new();
    fx.sinks = sinks;
    let p = TestProvider::new("cap", 2);
    let providers = one_provider("cap", &p);
    let err = validate(&spec, &profile_binding(&["cap"]), &fx.inputs(&providers))
        .expect_err("the name is in two of the three sets");
    assert!(
        matches!(&err, SpecError::DuplicateBindingName { name, .. } if *name == id("cap")),
        "{err:?}"
    );
}

#[test]
fn sb_22_an_island_executor_must_hold_the_executor_role() {
    // SB-22 and MA-38: the Island's `executor` names a binding whose Module holds the
    // Executor role. Unchecked, the Island's fragment `instance` recorded a Module
    // that cannot run it — the provenance D18 was raised about — and the same name
    // also produced a phantom Provider fragment with no matched request, breaking
    // SB-39/SB-44/MA-12 for it by construction.
    let spec = minimal_spec();
    let mut profile = profile_binding(&["radio"]);
    profile.bindings.insert(
        id("exec"),
        ezsdr_kernel::binding::Binding {
            module: mid("ezsdr.test.provider"),
            feed: None,
            selector: BTreeMap::new(),
            profile: None,
        },
    );
    profile.placements.islands.push(IslandDecl {
        id: IslandId::local(0),
        executor: id("exec"),
        components: Vec::new(),
        affinity: None,
        rt_policy: None,
        batch: None,
    });
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2);
    let providers = one_provider("radio", &p);
    let err = validate(&spec, &profile, &fx.inputs(&providers))
        .expect_err("a Provider Module cannot run an Island");
    assert!(
        matches!(&err, SpecError::WrongBindingRole { name, expected, .. }
            if *name == id("exec") && expected == "Executor"),
        "{err:?}"
    );
}

#[test]
fn sb_22_feed_on_a_spec_run_binding_is_refused() {
    // SB-22: "A Spec Run's `outputs[]` already declare their feeds, so a binding that
    // carries `feed` there is refused." Unrefused, the field was a second and unread
    // source of truth: a `Block` policy on it — which SC-21 forbids — was never seen,
    // and an author editing it got no diagnostic.
    let spec = minimal_spec();
    let mut profile = profile_binding(&["radio"]);
    profile.bindings.get_mut(&id("radio")).expect("bound").feed =
        Some(ezsdr_kernel::spec::SinkFeed {
            port: PortRef { component: "radio".into(), port: "rx".into() },
            policy: BackPressure::Block,
            capacity: 1,
        });
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2);
    let providers = one_provider("radio", &p);
    let err = validate(&spec, &profile, &fx.inputs(&providers)).expect_err("not a Session");
    assert!(
        matches!(&err, SpecError::Structural { reason } if reason.contains("only a Session")),
        "{err:?}"
    );
    // The same profile is legitimate on a Session, where RS-12 reads it.
    let mut session = Fixture::new();
    session.is_session = true;
    assert!(validate(&spec, &profile, &session.inputs(&providers)).is_ok());
}

#[test]
fn sb_02_an_ext_key_reaches_the_capability_match() {
    // SB-2 admits `ext.<module-id>.<path>`, and an Extension has no `KeyDecl` by
    // construction (MA-34, OV-14). Looking one up made the documented escape hatch
    // dead: the key passed `check_key_prefixes` and `validate` then refused it as
    // `UnknownKeyPrefix`, so no Spec could ever use one.
    // `ext.` followed by a Module id and a path; a Module id is dotted (SB-1, MA-34).
    let ext = Key::parse("ext.ezsdr.test.provider.thing").expect("a valid ext key");
    assert!(ext.is_extension());
    let mut req = resource("test.device", &[]);
    req.requires.insert(ext.clone(), Constraint::Eq { value: Value::Int(1) });
    let spec = spec_with([(id("radio"), req)].into_iter().collect());
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2);
    let providers = one_provider("radio", &p);

    // It reaches the capability match, where the node declares nothing under that key,
    // so it is *rejected on the merits* rather than refused as an unknown prefix.
    let out = validate(&spec, &profile_binding(&["radio"]), &fx.inputs(&providers))
        .expect("no UnknownKeyPrefix");
    assert!(!out.is_admitted());
    assert_eq!(out.rejected.len(), 1);
    assert_eq!(out.rejected[0].key, ext);
}

#[test]
fn ma_37_a_duplicate_port_name_is_refused_by_validate() {
    // MA-37's structural check had no caller in `src/`, so a component with two ports
    // of one name validated and `source_contract` then resolved a link to whichever
    // came first: a link the author meant to carry `sc16`, checked and planned as
    // `cf32`.
    let cf32 = ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.cf32").expect("id");
    let sc16 = ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.sc16").expect("id");
    let mut spec = minimal_spec();
    let mut c = support::recorder_component(cf32);
    c.ports.push(ezsdr_kernel::contract::Port {
        name: c.ports[0].name.clone(),
        direction: ezsdr_kernel::contract::PortDirection::In,
        contract: sc16,
    });
    spec.graph.components.insert(id("proc"), c);
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2);
    let providers = one_provider("radio", &p);
    let err = validate(&spec, &profile_binding(&["radio"]), &fx.inputs(&providers))
        .expect_err("MA-37 refuses a duplicate port name");
    assert!(matches!(&err, SpecError::Structural { reason } if reason.contains("MA-37")), "{err:?}");

    // And an unregistered contract on a port nothing links to is refused too.
    let mut spec = minimal_spec();
    let ghost = ezsdr_kernel::contract::DataContractId::parse("test.ghost").expect("id");
    spec.graph.components.insert(id("proc"), support::recorder_component(ghost));
    assert!(matches!(
        validate(&spec, &profile_binding(&["radio"]), &fx.inputs(&providers)),
        Err(SpecError::Structural { .. })
    ));
}

#[test]
fn ma_12_a_widening_effective_is_refused_at_prepare() {
    // MA-12: "`effective` may narrow a declared capability and must not widen one,
    // and the coordinator re-runs constraint matching over `effective`." Both halves
    // were unenforced: the predicate existed and only a test called it, so a Provider
    // whose `prepare` disagreed with its `coerce` put a value the Spec never asked for
    // into `run.effective()` and the Manifest.
    let mut req = resource("test.device", &[]);
    req.requires.insert(key("test.count"), Constraint::Eq { value: Value::Int(2) });
    let spec = spec_with([(id("radio"), req)].into_iter().collect());
    let profile = profile_binding(&["radio"]);
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2);
    let providers = one_provider("radio", &p);
    let admission = validate(&spec, &profile, &fx.inputs(&providers)).expect("validates");

    // The node declares `test.count` as exactly 2; a report claiming 7 widens it.
    let widened = PrepareReport {
        fragment: id("radio"),
        effective: [(key("test.count"), Value::Int(7))].into_iter().collect(),
        coercions: Vec::new(),
        warnings: Vec::new(),
    };
    let failed = collect_prepare(
        vec![Ok(widened)],
        &spec,
        &profile,
        &fx.inputs(&providers),
        &admission,
    )
    .expect_err("a widening is refused");
    let ezsdr_kernel::plan::PrepareError::Violations(v) = failed else { panic!("{failed:?}") };
    // Both halves fire: the widening, and the Spec's own `Eq(2)` no longer satisfied.
    // "MA-12 (re-match)", not bare "MA-12": the narrowing half's message also
    // begins "MA-12", so the looser assertion stayed green with the constraint
    // re-match deleted.
    assert!(v.iter().any(|x| x.reason.contains("MA-12 (re-match)")), "{v:?}");

    // The declared value itself passes both.
    let exact = PrepareReport {
        fragment: id("radio"),
        effective: [(key("test.count"), Value::Int(2))].into_iter().collect(),
        coercions: Vec::new(),
        warnings: Vec::new(),
    };
    assert!(
        collect_prepare(vec![Ok(exact)], &spec, &profile, &fx.inputs(&providers), &admission)
            .is_ok()
    );
}

#[test]
fn ma_12_two_resources_naming_one_key_do_not_refuse_each_other() {
    // SB-41: the merge lets a later fragment's value win for a key two fragments both
    // name, "which the Kernel does not otherwise interpret". Reading MA-12's re-match
    // out of `merged.effective` interpreted it per resource against data that cannot
    // tell two resources apart, so two channels each asking their own line's declared
    // count refused each other — fail-closed, but the ordinary two-channel Spec was
    // unrunnable. Each resource is judged by its own report.
    let mut a = resource("test.line", &[]);
    a.requires.insert(key("test.count"), Constraint::Eq { value: Value::Int(2) });
    let mut b = resource("test.line", &[]);
    b.requires.insert(key("test.count"), Constraint::Eq { value: Value::Int(4) });
    let spec = spec_with([(id("a"), a), (id("b"), b)].into_iter().collect());
    let profile = profile_binding(&["a", "b"]);
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2).with_asymmetric_lines(2, 4);
    let providers: BTreeMap<Ident, &dyn Provider> =
        [(id("a"), &p as &dyn Provider), (id("b"), &p as &dyn Provider)].into_iter().collect();
    let admission = validate(&spec, &profile, &fx.inputs(&providers)).expect("validates");
    assert!(admission.is_admitted());

    let report = |name: &str, n: i64| PrepareReport {
        fragment: id(name),
        effective: [(key("test.count"), Value::Int(n))].into_iter().collect(),
        coercions: Vec::new(),
        warnings: Vec::new(),
    };
    // Each report states exactly what its own node declares.
    let merged = collect_prepare(
        vec![Ok(report("a", 2)), Ok(report("b", 4))],
        &spec,
        &profile,
        &fx.inputs(&providers),
        &admission,
    )
    .expect("two resources, each satisfied by its own node");
    // The merge is still lossy, by SB-41's own rule — that is why the re-match must
    // not read it.
    assert_eq!(merged.effective[&key("test.count")], Value::Int(4));
    assert_eq!(merged.reports.len(), 2);

    // A widening in one report is still refused, and named against that resource.
    let failed = collect_prepare(
        vec![Ok(report("a", 9)), Ok(report("b", 4))],
        &spec,
        &profile,
        &fx.inputs(&providers),
        &admission,
    )
    .expect_err("a's report widens its own node");
    let ezsdr_kernel::plan::PrepareError::Violations(v) = failed else { panic!("{failed:?}") };
    assert!(v.iter().any(|x| x.reason.contains("a's effective")), "{v:?}");
}

#[test]
fn sb_39_plan_refuses_an_admission_result_from_another_spec() {
    // "Admitted" means only that nothing was rejected, which
    // `AdmissionResult::default()` satisfies, so the SB-30 guard alone accepted a
    // stale or hand-built result: the Provider fragment then carried no `requested`
    // and the Provider was configured with nothing — the failure D32 was raised
    // about, silently rather than as a refusal.
    let spec = minimal_spec();
    let profile = profile_binding(&["radio"]);
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2);
    let providers = one_provider("radio", &p);
    let err = plan(
        &spec,
        &profile,
        &ezsdr_kernel::binding::AdmissionResult::default(),
        &fx.inputs(&providers),
        Vec::new(),
    )
    .expect_err("an empty result is not this Spec's");
    assert!(
        matches!(&err, SpecError::Structural { reason } if reason.contains("SB-39")),
        "{err:?}"
    );
    // The real pipeline's result plans, and its fragment carries the request.
    let built = validate_then_plan(&spec, &profile, &fx.inputs(&providers), Vec::new())
        .expect("plans");
    assert!(built.fragments[0].content.get("requested").is_some());
}

#[test]
fn sb_22_the_sink_path_segment_is_reserved() {
    // The `sink/` prefix only separates the two namespaces if a Provider may not
    // declare a node under it: `ResourceId::parse("sink/rec")` is a legal node path,
    // so a Provider named `sink` still collided with a bound Sink's address.
    let spec = spec_with([(id("radio"), resource("test.device", &[]))].into_iter().collect());
    let fx = Fixture::new();
    let p = TestProvider::new("sink", 2);
    let providers = one_provider("radio", &p);
    let err = validate(&spec, &profile_binding(&["radio"]), &fx.inputs(&providers))
        .expect_err("`sink` is reserved");
    assert!(
        matches!(&err, SpecError::Structural { reason } if reason.contains("reserved")),
        "{err:?}"
    );
    // Any other root is fine.
    let ok = TestProvider::new("radio", 2);
    let providers = one_provider("radio", &ok);
    assert!(validate(&spec, &profile_binding(&["radio"]), &fx.inputs(&providers)).is_ok());
}

#[test]
fn sb_02_an_ext_key_needs_a_registered_owner() {
    // MA-34 writes `ext.<module-id>.<path>`. Accepting any `ext.…` made the escape
    // hatch unowned: no declaration, no shape check, nobody accountable for the
    // meaning.
    let mut req = resource("test.device", &[]);
    req.requires.insert(
        Key::parse("ext.nobody.owns.this").expect("parses"),
        Constraint::Eq { value: Value::Int(1) },
    );
    let spec = spec_with([(id("radio"), req)].into_iter().collect());
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2);
    let providers = one_provider("radio", &p);
    let err = validate(&spec, &profile_binding(&["radio"]), &fx.inputs(&providers))
        .expect_err("no registered Module owns it");
    assert!(
        matches!(&err, SpecError::UnknownKeyPrefix { key } if key.contains("MA-34")),
        "{err:?}"
    );
}

#[test]
fn ov_15_a_non_ascii_key_inside_a_value_map_is_refused() {
    // `check_ascii_keys` guards the three document boundaries. A `Value::Map` reaches
    // the sealed Manifest through paths that pass none of them — a Session
    // `SetParameter`, an Action's `params` or `metadata` — so the check belongs where
    // every map is walked (RS-11, OV-15).
    let ascii = Value::Map(
        [("rho".to_owned(), Value::Int(1))].into_iter().collect::<BTreeMap<String, Value>>(),
    );
    assert!(ascii.check_nesting("x").is_ok(), "an ASCII key is fine");
    let from_json: Value =
        serde_json::from_value(serde_json::json!({ "\u{3c1}": 1 })).expect("deserialises");
    let err = from_json.check_nesting("params").expect_err("refused");
    assert!(
        matches!(&err, SpecError::KeyShape { found, .. } if found.contains("non-ASCII")),
        "{err:?}"
    );
}

#[test]
fn sb_46_an_accepted_coercion_survives_prepare() {
    // Spec 03 §2's headline evidence: a request for 19.5 Msps is applied as 20 and
    // "SB-44 and SB-46 make the coercion a document". A coercion is by definition a
    // key whose applied value does not satisfy the requested constraint, so
    // re-matching MA-12's constraint over every key refused every coercion by
    // construction and SB-46's `accept` and `warn` branches became unreachable.
    let mut req = resource("test.device", &[]);
    req.requires.insert(key("test.grid"), Constraint::Eq { value: Value::Num(19.5) });
    let mut spec = spec_with([(id("radio"), req)].into_iter().collect());
    spec.policies.coercion.insert(key("test.grid"), CoercionPolicy::Accept);
    let profile = profile_binding(&["radio"]);
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2).with_grid(20.0);
    let providers = one_provider("radio", &p);
    let admission = validate(&spec, &profile, &fx.inputs(&providers)).expect("validates");
    assert!(admission.is_admitted(), "an accepted coercion is admitted");
    assert_eq!(admission.coercions_preview.len(), 1, "and previewed");

    // The Provider replays `coerce`, which SB-44 and MA-12 oblige it to do.
    let report = PrepareReport {
        fragment: id("radio"),
        effective: [(key("test.grid"), Value::Num(20.0))].into_iter().collect(),
        coercions: vec![ezsdr_kernel::spec::Coercion {
            key: key("test.grid"),
            requested: Value::Num(19.5),
            applied: Value::Num(20.0),
            reason: "snapped to a multiple of 20".to_owned(),
        }],
        warnings: Vec::new(),
    };
    let merged = collect_prepare(
        vec![Ok(report)],
        &spec,
        &profile,
        &fx.inputs(&providers),
        &admission,
    )
    .expect("an accepted coercion is applied and recorded, not refused");
    assert_eq!(merged.effective[&key("test.grid")], Value::Num(20.0));
    assert_eq!(merged.reports[0].coercions.len(), 1, "and it is in the PrepareReport");

    // Applying something other than what `coerce` returned is SB-44's, whether or not
    // the report declares a coercion — the exclusion is keyed on the Kernel's own
    // preview, so naming a key in `coercions` is not a self-issued exemption.
    let disagreeing = PrepareReport {
        fragment: id("radio"),
        effective: [(key("test.grid"), Value::Num(40.0))].into_iter().collect(),
        coercions: vec![ezsdr_kernel::spec::Coercion {
            key: key("test.grid"),
            requested: Value::Num(19.5),
            applied: Value::Num(20.0),
            reason: "claims 20 and applies 40".to_owned(),
        }],
        warnings: Vec::new(),
    };
    let failed =
        collect_prepare(vec![Ok(disagreeing)], &spec, &profile, &fx.inputs(&providers), &admission)
            .expect_err("the applied value is not the one `coerce` returned");
    let ezsdr_kernel::plan::PrepareError::Violations(v) = failed else { panic!("{failed:?}") };
    assert!(v.iter().any(|x| x.reason.contains("SB-44")), "{v:?}");

    // A key the Kernel never coerced is still MA-12's, which is the division this
    // restores: `test.count` is satisfied directly, so it has no preview entry.
    let mut counted = resource("test.device", &[]);
    counted.requires.insert(key("test.count"), Constraint::Eq { value: Value::Int(2) });
    let count_spec = spec_with([(id("radio"), counted)].into_iter().collect());
    let count_admission =
        validate(&count_spec, &profile, &fx.inputs(&providers)).expect("validates");
    assert!(count_admission.coercions_preview.is_empty(), "satisfied directly (SB-6)");
    let undeclared = PrepareReport {
        fragment: id("radio"),
        effective: [(key("test.count"), Value::Int(9))].into_iter().collect(),
        coercions: Vec::new(),
        warnings: Vec::new(),
    };
    let failed = collect_prepare(
        vec![Ok(undeclared)],
        &count_spec,
        &profile,
        &fx.inputs(&providers),
        &count_admission,
    )
    .expect_err("an undeclared change is refused");
    let ezsdr_kernel::plan::PrepareError::Violations(v) = failed else { panic!("{failed:?}") };
    // "MA-12 (re-match)", not bare "MA-12": the narrowing half's message also
    // begins "MA-12", so the looser assertion stayed green with the constraint
    // re-match deleted.
    assert!(v.iter().any(|x| x.reason.contains("MA-12 (re-match)")), "{v:?}");
}

#[test]
fn sb_07_coerce_is_called_once_with_the_whole_request() {
    // SB-44 says the report's coercions equal what `coerce` returned **for the same
    // request**. Calling `coerce` once per key with a single-key request made that
    // unsatisfiable, because a fragment carries the resource's whole map and a
    // Provider replays `coerce` over all of it — so a Provider whose keys interact
    // answered two different questions and the Kernel's check at `prepare` could only
    // be approximate (finding D41).
    let mut req = resource("test.device", &[]);
    req.requires.insert(key("test.grid"), Constraint::Eq { value: Value::Num(19.5) });
    req.requires.insert(key("test.count"), Constraint::Eq { value: Value::Int(2) });
    let mut spec = spec_with([(id("radio"), req)].into_iter().collect());
    spec.policies.coercion.insert(key("test.grid"), CoercionPolicy::Accept);
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2).with_grid(20.0);
    let providers = one_provider("radio", &p);
    let out = validate(&spec, &profile_binding(&["radio"]), &fx.inputs(&providers))
        .expect("validates");
    assert!(out.is_admitted(), "{:?}", out.rejected);
    assert_eq!(p.coerce_calls.load(std::sync::atomic::Ordering::Relaxed), 1,
        "one call for the node, not one per key");

    // And the request it saw carried **both** keys — the same map a fragment carries,
    // which is what makes SB-44's equality literal.
    let seen = p.last_request.lock().expect("not poisoned").clone().expect("coerce was called");
    assert_eq!(seen.constraints.len(), 2);
    assert!(seen.constraints.contains_key(&key("test.grid")));
    assert!(seen.constraints.contains_key(&key("test.count")));
}

#[test]
fn sb_44_a_provider_may_not_exempt_a_key_by_declaring_a_coercion() {
    // The coercion exclusion MA-12's re-match needs must be keyed on the **Kernel's**
    // preview, not on the report's own `coercions`: keyed on the report, naming a key
    // there was a self-issued exemption, so a Provider could apply any value its node
    // declares — or any value at all for a key the node does not declare — while the
    // Manifest recorded the substitution as a legitimate coercion. A Manifest that
    // lies is worse than one that is missing (SB-44, MA-11, MA-12, spec 03 §2).
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2).with_grid(20.0);
    let providers = one_provider("radio", &p);
    let profile = profile_binding(&["radio"]);

    // Door one: the coercion documents 20.0 and the Run applies 100.0, which is in the
    // node's declared grid so the narrowing half passes.
    let mut req = resource("test.device", &[]);
    req.requires.insert(key("test.grid"), Constraint::Eq { value: Value::Num(19.5) });
    let mut spec = spec_with([(id("radio"), req)].into_iter().collect());
    spec.policies.coercion.insert(key("test.grid"), CoercionPolicy::Accept);
    let admission = validate(&spec, &profile, &fx.inputs(&providers)).expect("validates");
    assert_eq!(admission.coercions_preview.len(), 1, "the Kernel's own record");
    let lying = PrepareReport {
        fragment: id("radio"),
        effective: [(key("test.grid"), Value::Num(100.0))].into_iter().collect(),
        coercions: vec![ezsdr_kernel::spec::Coercion {
            key: key("test.grid"),
            requested: Value::Num(19.5),
            applied: Value::Num(20.0),
            reason: "documents 20 and applies 100".to_owned(),
        }],
        warnings: Vec::new(),
    };
    let failed =
        collect_prepare(vec![Ok(lying)], &spec, &profile, &fx.inputs(&providers), &admission)
            .expect_err("the applied value is not the documented one");
    let ezsdr_kernel::plan::PrepareError::Violations(v) = failed else { panic!("{failed:?}") };
    assert!(v.iter().any(|x| x.reason.contains("SB-44")), "{v:?}");

    // Door two, the worse one: the key was satisfied **directly**, so `coerce` was
    // never called and the dry run previewed nothing — and a coercion invented at
    // `prepare` claimed the exemption anyway.
    let mut direct = resource("test.device", &[]);
    direct.requires.insert(key("test.grid"), Constraint::Eq { value: Value::Num(20.0) });
    let mut spec = spec_with([(id("radio"), direct)].into_iter().collect());
    spec.policies.coercion.insert(key("test.grid"), CoercionPolicy::Accept);
    let admission = validate(&spec, &profile, &fx.inputs(&providers)).expect("validates");
    assert!(admission.coercions_preview.is_empty(), "satisfied directly, so no coercion");
    let invented = PrepareReport {
        fragment: id("radio"),
        effective: [(key("test.grid"), Value::Num(100.0))].into_iter().collect(),
        coercions: vec![ezsdr_kernel::spec::Coercion {
            key: key("test.grid"),
            requested: Value::Num(20.0),
            applied: Value::Num(100.0),
            reason: "invented at prepare".to_owned(),
        }],
        warnings: Vec::new(),
    };
    let failed =
        collect_prepare(vec![Ok(invented)], &spec, &profile, &fx.inputs(&providers), &admission)
            .expect_err("a Spec asking Eq(20.0) may not run at 100");
    let ezsdr_kernel::plan::PrepareError::Violations(v) = failed else { panic!("{failed:?}") };
    // "MA-12 (re-match)", not bare "MA-12": the narrowing half's message also
    // begins "MA-12", so the looser assertion stayed green with the constraint
    // re-match deleted.
    assert!(v.iter().any(|x| x.reason.contains("MA-12 (re-match)")), "{v:?}");
}

#[test]
fn sb_03_two_instances_may_not_declare_one_node_path() {
    // `ResourceId` is `{ node, path }` with `node == LOCAL` for all of v4.0, so it
    // carries no instance qualification and two bound instances declaring one path are
    // one address. The matcher treated them as one node and refused the second with
    // `NodeAlreadyBound`, naming two resources colliding on one node when they had
    // asked for two devices.
    let spec = spec_with(
        [(id("a"), resource("test.device", &[])), (id("b"), resource("test.device", &[]))]
            .into_iter()
            .collect(),
    );
    let fx = Fixture::new();
    let one = TestProvider::new("mock", 2);
    let two = TestProvider::new("mock", 2);
    let providers: BTreeMap<Ident, &dyn Provider> =
        [(id("a"), &one as &dyn Provider), (id("b"), &two as &dyn Provider)].into_iter().collect();
    let two = distinct_instances(profile_binding(&["a", "b"]), &["a", "b"]);
    let err = validate(&spec, &two, &fx.inputs(&providers))
        .expect_err("two instances, one node path");
    assert!(
        matches!(&err, SpecError::Structural { reason } if reason.contains("SB-3")),
        "the diagnostic names the real cause: {err:?}"
    );
    // Distinct paths bind normally, and one instance bound twice is still SB-34's case.
    let other = TestProvider::new("other", 2);
    let distinct: BTreeMap<Ident, &dyn Provider> =
        [(id("a"), &one as &dyn Provider), (id("b"), &other as &dyn Provider)]
            .into_iter()
            .collect();
    assert!(validate(&spec, &two, &fx.inputs(&distinct)).is_ok());
}

#[test]
fn sb_02_an_ext_key_parses_for_every_module_id_grammar() {
    // MA-19's `ModuleId` admits `A-Z`, `-` and a leading digit; an `Ident` does not.
    // Holding an `ext.<module-id>.<path>` key to `Ident`'s grammar refused it at
    // `Key::parse`, before any rule could run, for every Module with a dash or a
    // capital in its id — so MA-34's escape hatch was dead for most of MA-19.
    for id_text in ["ezsdr.test-provider", "ezsdr.Radio", "ezsdr.test.provider", "vendor.9lives"] {
        assert!(
            ezsdr_kernel::id::ModuleId::parse(id_text).is_ok(),
            "{id_text} is a valid ModuleId (MA-19)"
        );
        let k = format!("ext.{id_text}.thing");
        assert!(Key::parse(&k).is_ok_and(|k| k.is_extension()), "{k} must parse (SB-2, MA-34)");
    }
    // A Vocabulary-prefixed key keeps the stricter grammar.
    assert!(Key::parse("test.Grid").is_err(), "a Vocabulary key is lowercase (SB-1)");
}

#[test]
fn sb_36_a_needs_key_may_not_collide_with_a_resource_name() {
    // SB-36 records a need under `<resource>_<need>` in the one `matched` map, so a
    // key equal to another resource's name overwrote it — always the need's record,
    // since `X < X_need` puts the resource's insert second. The Manifest then
    // reported a resolution that never happened, and `matched` is load-bearing for
    // port resolution now (SB-15), not merely a record.
    let mut a = resource("test.device", &[]);
    a.needs.insert(
        id("b"),
        ezsdr_kernel::spec::SubResourceReq { kind: ns("test.line"), requires: BTreeMap::new() },
    );
    let spec = spec_with(
        [(id("a"), a), (id("a_b"), resource("test.line", &[]))].into_iter().collect(),
    );
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2);
    let providers: BTreeMap<Ident, &dyn Provider> =
        [(id("a"), &p as &dyn Provider), (id("a_b"), &p as &dyn Provider)].into_iter().collect();
    let err = validate(&spec, &profile_binding(&["a", "a_b"]), &fx.inputs(&providers))
        .expect_err("the need's key collides with a resource name");
    assert!(
        matches!(&err, SpecError::DuplicateBindingName { name, .. } if *name == id("a_b")),
        "{err:?}"
    );
}

#[test]
fn sb_39_two_islands_on_one_executor_is_not_a_cycle() {
    // An affinity split is the ordinary reason for it, and the operator must not be
    // told there is a cycle in an unnamed set of fragments.
    let nodes = vec![id("a"), id("a")];
    assert!(matches!(
        arm_order(&nodes, &[]),
        Err(SpecError::Structural { reason }) if reason.contains("share the id")
    ));
    // A dangling ordering edge is reported as itself.
    assert!(matches!(
        arm_order(&[id("a")], &[(id("ghost"), id("a"))]),
        Err(SpecError::Structural { reason }) if reason.contains("unknown fragment")
    ));
}

#[test]
fn sb_24_authority_must_name_a_binding() {
    let spec = minimal_spec();
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2);
    let providers = one_provider("radio", &p);
    let mut profile = profile_binding(&["radio"]);
    profile.authority = Some(id("nowhere"));
    assert!(matches!(
        validate_then_plan(&spec, &profile, &fx.inputs(&providers), Vec::new()),
        Err(SpecError::Structural { reason }) if reason.contains("not a binding")
    ));
}

#[test]
fn sb_06_a_capability_of_the_wrong_kind_is_key_shape_in_every_cell() {
    // SB-6: "a mismatch is `KeyShape`" — including the Range x Range cell, which is
    // the one that could report a malformed capability as merely unsatisfiable.
    let wrong = CapabilityValue::Range {
        min: Value::Str("a".into()),
        max: Value::Str("z".into()),
    };
    for c in [
        Constraint::Eq { value: Value::Int(1) },
        Constraint::Range { min: Some(Value::Int(1)), max: Some(Value::Int(2)) },
        Constraint::Min { value: Value::Int(1) },
        Constraint::Max { value: Value::Int(1) },
    ] {
        assert!(matches!(satisfies(&c, &wrong), Err(SpecError::KeyShape { .. })), "{c:?}");
    }
}

#[test]
fn sb_13_an_opaque_parameter_schema_is_not_a_placement() {
    // B3 weighed the false negative (smuggling into `extensions`) and not this false
    // positive: a Vocabulary must be able to describe a parameter whose object has
    // an `island` property.
    let doc = serde_json::json!({
        "version": 1,
        "graph": { "components": { "proc": {
            "id": "proc",
            "kind": "processor",
            "ports": [],
            "params": [{
                "key": "test.flag",
                "schema": { "type": "object", "properties": { "island": { "type": "string" } } },
                "update_class": "block_boundary",
                "default": true
            }],
            "requires": { "executor_kind": "any", "memory_bytes": null },
            "impl": { "kind": "test.impl", "id": "proc", "hash": "sha256:0000000000000000000000000000000000000000000000000000000000000000" }
        } } }
    });
    assert!(ExperimentSpec::from_json(&doc).is_ok(), "an opaque schema is not a placement");
    // A real placement field is still refused.
    let doc = serde_json::json!({ "version": 1, "resources": { "r": { "kind": "test.device", "island": 1 } } });
    assert!(matches!(ExperimentSpec::from_json(&doc), Err(SpecError::PlacementInSpec { .. })));
}

#[test]
fn sb_04_value_nesting_is_one_level() {
    let flat = Value::List(vec![Value::Int(1), Value::Str("a".into())]);
    assert!(flat.check_nesting("test.list").is_ok());
    let nested = Value::List(vec![Value::List(vec![Value::Int(1)])]);
    assert!(matches!(nested.check_nesting("test.list"), Err(SpecError::KeyShape { .. })));
    let nested_map = Value::Map([("a".to_owned(), Value::Map(BTreeMap::new()))].into_iter().collect());
    assert!(matches!(nested_map.check_nesting("test.map"), Err(SpecError::KeyShape { .. })));
}

#[test]
fn sb_05_constraint_values_are_scalars() {
    let mut spec = spec_with(
        [(id("radio"), resource("test.device", &[("test.count", Constraint::Eq { value: Value::List(vec![Value::Int(1)]) })]))]
            .into_iter()
            .collect(),
    );
    spec.version = 1;
    let doc = serde_json::to_value(&spec).expect("serialises");
    assert!(matches!(ExperimentSpec::from_json(&doc), Err(SpecError::KeyShape { .. })));
}

#[test]
fn sb_03_resource_path_addresses_a_sub_resource() {
    let root = rid("radio");
    let channel = root.child("rx").expect("valid").child("0").expect("valid");
    assert_eq!(channel.path, "radio/rx/0");
    assert!(channel.is_within(&root));
    assert_eq!(channel.parent().expect("has one").path, "radio/rx");
    assert_eq!(channel.segments().count(), 3);
    assert!(ezsdr_kernel::id::ResourceId::parse("radio//0").is_err());
    assert!(ezsdr_kernel::id::ResourceId::parse("radio/ rx").is_err());
    assert!(!rid("radio2").is_within(&root), "prefix matching is on segment boundaries");
}

// ------------------------------------------- the Fable second-opinion findings

#[test]
fn sb_44_a_coercion_is_charged_only_to_the_resource_it_was_computed_for() {
    // SB-44's obligation is that the value a fragment applies is the one `coerce`
    // returned **for that fragment's request**. `coerce` is called once per bound
    // node (SB-7), so a key alone does not name a preview: two resources may
    // constrain one key on two devices, and one of them coercing says nothing about
    // the other. Keyed on the key alone, `b` — satisfied directly, never passed to
    // `coerce` — was refused for not applying a value nobody computed for it.
    //
    // This is the ordinary two-channel Spec: two lines, each asking its own grid.
    let mut a = resource("test.device", &[]);
    a.requires.insert(key("test.grid"), Constraint::Eq { value: Value::Num(19.5) });
    let mut b = resource("test.device", &[]);
    b.requires.insert(key("test.grid"), Constraint::Eq { value: Value::Num(40.0) });
    let mut spec = spec_with([(id("a"), a), (id("b"), b)].into_iter().collect());
    spec.policies.coercion.insert(key("test.grid"), CoercionPolicy::Accept);
    let profile = distinct_instances(profile_binding(&["a", "b"]), &["a", "b"]);

    let fx = Fixture::new();
    let pa = TestProvider::new("a", 2).with_grid(20.0);
    let pb = TestProvider::new("b", 2).with_grid(20.0);
    let providers: BTreeMap<Ident, &dyn Provider> =
        [(id("a"), &pa as &dyn Provider), (id("b"), &pb as &dyn Provider)].into_iter().collect();
    let admission = validate(&spec, &profile, &fx.inputs(&providers)).expect("validates");
    assert!(admission.is_admitted());
    // Exactly one coercion, and it belongs to `a`: `b`'s 40.0 is a declared value.
    assert_eq!(admission.coercions_preview.len(), 1, "only `a` coerced");
    assert_eq!(admission.coercions_preview[0].resource, id("a"), "and the record says so");

    let reports = vec![
        Ok(PrepareReport {
            fragment: id("a"),
            effective: [(key("test.grid"), Value::Num(20.0))].into_iter().collect(),
            coercions: vec![ezsdr_kernel::spec::Coercion {
                key: key("test.grid"),
                requested: Value::Num(19.5),
                applied: Value::Num(20.0),
                reason: "snapped to a multiple of 20".to_owned(),
            }],
            warnings: Vec::new(),
        }),
        Ok(PrepareReport {
            fragment: id("b"),
            effective: [(key("test.grid"), Value::Num(40.0))].into_iter().collect(),
            coercions: Vec::new(),
            warnings: Vec::new(),
        }),
    ];
    let merged = collect_prepare(reports, &spec, &profile, &fx.inputs(&providers), &admission)
        .expect("`b` applied what it asked for and is not charged with `a`'s coercion");
    assert_eq!(merged.reports.len(), 2);

    // The check is still live for the resource the preview *is* about: `a` applying
    // something other than the 20.0 `coerce` returned is SB-44's own refusal.
    let disagreeing = vec![
        Ok(PrepareReport {
            fragment: id("a"),
            effective: [(key("test.grid"), Value::Num(100.0))].into_iter().collect(),
            coercions: vec![ezsdr_kernel::spec::Coercion {
                key: key("test.grid"),
                requested: Value::Num(19.5),
                applied: Value::Num(20.0),
                reason: "claims 20 and applies 100".to_owned(),
            }],
            warnings: Vec::new(),
        }),
        Ok(PrepareReport {
            fragment: id("b"),
            effective: [(key("test.grid"), Value::Num(40.0))].into_iter().collect(),
            coercions: Vec::new(),
            warnings: Vec::new(),
        }),
    ];
    let err = collect_prepare(disagreeing, &spec, &profile, &fx.inputs(&providers), &admission)
        .expect_err("SB-44 still refuses the resource the preview is about");
    let ezsdr_kernel::plan::PrepareError::Violations(v) = err else {
        panic!("expected violations")
    };
    assert!(
        v.iter().any(|x| x.reason.contains("SB-44") && x.reason.contains("a's effective")),
        "{v:?}"
    );
}

#[test]
fn sb_3_two_bindings_with_one_description_may_be_two_objects() {
    // SB-3's identity is the **binding description** `(module, selector, profile)`:
    // two bindings that share one are one instance, and they must report one
    // `instance().id`. Two distinct Rust objects reporting one id is that legal case
    // — the runtime is free to hand the Kernel a fresh handle per name — and the
    // pointer comparison D44 replaced refused it, because two objects have two
    // addresses whatever their descriptions say.
    // Two `test.line` resources, so SB-34 gives each its own node and the only thing
    // under test is SB-3's instance identity.
    let spec = spec_with(
        [(id("a"), resource("test.line", &[])), (id("b"), resource("test.line", &[]))]
            .into_iter()
            .collect(),
    );
    let profile = profile_binding(&["a", "b"]); // one description: empty selectors
    let fx = Fixture::new();
    let first = TestProvider::new("shared", 2);
    let second = TestProvider::new("shared", 2); // a second object, one declared id
    let providers: BTreeMap<Ident, &dyn Provider> =
        [(id("a"), &first as &dyn Provider), (id("b"), &second as &dyn Provider)]
            .into_iter()
            .collect();
    validate(&spec, &profile, &fx.inputs(&providers))
        .expect("one description and one declared id is one instance, however many objects");

    // And the refusal the rule is for: one description, two declared ids.
    let other = TestProvider::new("other", 2);
    let disagreeing: BTreeMap<Ident, &dyn Provider> =
        [(id("a"), &first as &dyn Provider), (id("b"), &other as &dyn Provider)]
            .into_iter()
            .collect();
    let err = validate(&spec, &profile, &fx.inputs(&disagreeing))
        .expect_err("one description cannot name two instances");
    assert!(
        matches!(&err, SpecError::Structural { reason }
            if reason.contains("SB-3") && reason.contains("one binding description")),
        "{err:?}"
    );
}

#[test]
fn ma_41_an_absent_or_non_string_class_is_refused() {
    // MA-41 names three refusals — "an absent, non-string or unrecognised `class` is
    // refused rather than ignored" — for the reason the clause itself gives: ignoring
    // a misspelling turns it into agreement, and the class a misspelling lands on is
    // the one that may claim determinism. Reading the section through
    // `and_then(as_str)` refused only the third.
    let spec = minimal_spec();
    let p = TestProvider::new("radio", 2);
    let providers = one_provider("radio", &p);
    let fx = Fixture::new();
    let with_time = |section: serde_json::Value| {
        let mut profile = profile_binding(&["radio"]);
        profile.environment.insert(ns("ezsdr.time"), section);
        profile
    };
    for (section, expected) in [
        (serde_json::json!({ "clas": "hardware" }), "declares no `class`"),
        (serde_json::json!({}), "declares no `class`"),
        (serde_json::json!({ "class": 3 }), "is not a string"),
    ] {
        let profile = with_time(section.clone());
        let admission =
            validate(&spec, &profile, &fx.inputs(&providers)).expect("binding itself is fine");
        let err = plan(&spec, &profile, &admission, &fx.inputs(&providers), Vec::new())
            .expect_err("MA-41 refuses it rather than deriving a class");
        assert!(
            matches!(&err, SpecError::Structural { reason }
                if reason.contains("MA-41") && reason.contains(expected)),
            "{section}: {err:?}"
        );
    }
    // The section is optional; only a section that is *present* must declare a class.
    let profile = profile_binding(&["radio"]);
    let admission = validate(&spec, &profile, &fx.inputs(&providers)).expect("validates");
    plan(&spec, &profile, &admission, &fx.inputs(&providers), Vec::new())
        .expect("no ezsdr.time section is not a declaration to disagree with");
}

#[test]
fn sb_39_a_malformed_arm_order_entry_is_refused() {
    // SB-39 exists because `v3/changelog/v3.0.20.md:56-59` records that with a shared
    // PPS the device that sources it must be started first or start-up fails, and its
    // answer is that the order is a property of the plan. An entry whose `before` or
    // `after` is missing or misspelled contributed no edge and left the fragments in
    // name order — so the PPS source armed second, which is the failure itself, with
    // no diagnostic.
    let spec = spec_with(
        [(id("zsource"), resource("test.device", &[])), (id("asink"), resource("test.device", &[]))]
            .into_iter()
            .collect(),
    );
    let pz = TestProvider::new("zsource", 2);
    let pa = TestProvider::new("asink", 2);
    let providers: BTreeMap<Ident, &dyn Provider> =
        [(id("zsource"), &pz as &dyn Provider), (id("asink"), &pa as &dyn Provider)]
            .into_iter()
            .collect();
    let with_order = |section: serde_json::Value| {
        let mut profile =
            distinct_instances(profile_binding(&["zsource", "asink"]), &["zsource", "asink"]);
        profile.authority = Some(id("zsource"));
        profile.environment.insert(ns("ezsdr.arm_order"), section);
        profile
    };
    let mut fx2 = Fixture::new();
    fx2.authorities.insert(
        id("zsource"),
        AuthorityDescriptor {
            governs: vec![ClockDomainId::HOST_MONOTONIC],
            pacing: Pacing::FreeRunning,
        },
    );

    // The well-formed entry orders the plan, which is what the refusals protect.
    let profile = with_order(serde_json::json!([{ "before": "zsource", "after": "asink" }]));
    let admission = validate(&spec, &profile, &fx2.inputs(&providers)).expect("validates");
    let p = plan(&spec, &profile, &admission, &fx2.inputs(&providers), Vec::new()).expect("plans");
    assert_eq!(p.fragments[0].id, id("zsource"), "the declared edge beats name order");

    for section in [
        serde_json::json!([{ "befor": "zsource", "after": "asink" }]),
        serde_json::json!([{ "before": "zsource" }]),
        serde_json::json!([{ "before": 1, "after": "asink" }]),
        serde_json::json!({ "before": "zsource", "after": "asink" }),
    ] {
        let profile = with_order(section.clone());
        let admission = validate(&spec, &profile, &fx2.inputs(&providers)).expect("validates");
        let err = plan(&spec, &profile, &admission, &fx2.inputs(&providers), Vec::new())
            .expect_err("a malformed entry is refused, not dropped");
        assert!(
            matches!(&err, SpecError::Structural { reason } if reason.contains("SB-39")),
            "{section}: {err:?}"
        );
    }
}

#[test]
fn sb_39_the_plan_records_the_contract_the_link_actually_carries() {
    // SB-39 says the plan carries "the DataLink declarations", and a declaration that
    // names the wrong contract is worse than a missing one: the plan reaches the
    // Manifest (RS-38) and is what a Link Module's `create(&DataLinkDecl)` receives
    // (MA-27). Resolving the consumer through `graph.components` alone and defaulting
    // to `ezsdr.control` gave that default to every link whose consumer is a resource
    // port — Vision §7's own `PHY Processor -> Radio Port` — after `validate` had
    // already checked SC-3 against the real one.
    let sc16 = ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.sc16").expect("id");
    let mut spec = minimal_spec();
    spec.resources.insert(id("line0"), resource("test.line", &[]));
    spec.graph.components.insert(id("src"), support::source_component(sc16.clone()));
    spec.graph.links.push(ezsdr_kernel::spec::LinkReq {
        from: PortRef { component: "src".into(), port: "out".into() },
        to: PortRef { component: "line0".into(), port: "tx".into() },
        policy: BackPressure::DropOldest,
        capacity: 4,
    });
    let mut profile = profile_binding(&["radio", "line0"]);
    profile.placements.components.insert(
        id("src"),
        ComponentPlacement { island: id("io"), memory_domain: MemoryDomainId::local(0) },
    );
    profile.placements.islands.push(IslandDecl {
        id: IslandId::local(0),
        executor: id("exec"),
        components: vec![id("src")],
        affinity: None,
        rt_policy: None,
        batch: None,
    });
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2);
    let providers: BTreeMap<Ident, &dyn Provider> =
        [(id("radio"), &p as &dyn Provider), (id("line0"), &p as &dyn Provider)]
            .into_iter()
            .collect();
    let admission = validate(&spec, &profile, &fx.inputs(&providers)).expect("validates");
    let plan = plan(&spec, &profile, &admission, &fx.inputs(&providers), Vec::new())
        .expect("plans");
    assert_eq!(plan.links.len(), 1);
    assert_eq!(
        plan.links[0].contract, sc16,
        "the bound line's `tx` carries sc16, which is what SC-3 was checked against"
    );
}

#[test]
fn sb_44_a_coercion_of_an_unrequested_key_is_refused_at_validate() {
    // SB-44: a report's coercions are what `coerce` returned **for the request**, so a
    // coercion of a key the request does not name is a malformed report. D42 made that
    // a refusal at `prepare`; the identical report reaching `validate` entered
    // `coercions_preview` unchecked, and the SB-45/46 policy loop then applied that
    // key's own default to a key nobody asked about — under `reject` it would fail a
    // dry run over a key absent from the Spec, and the Manifest recorded a
    // substitution of a value nobody requested.
    let spec = minimal_spec();
    let mut with_grid = spec.clone();
    with_grid
        .resources
        .get_mut(&id("radio"))
        .expect("the fixture's resource")
        .requires
        .insert(key("test.grid"), Constraint::Eq { value: Value::Num(19.5) });
    let profile = profile_binding(&["radio"]);
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2).with_grid(20.0).with_stray_coercion();
    let providers = one_provider("radio", &p);
    let admission = validate(&with_grid, &profile, &fx.inputs(&providers)).expect("runs");
    assert!(!admission.is_admitted(), "a malformed report is refused");
    assert!(
        admission.violations.iter().any(|v| v.reason.contains("SB-44")
            && v.reason.contains("test.flag")
            && v.reason.contains("does not name")),
        "{:?}",
        admission.violations
    );
    // And it is not recorded. `coercions_preview` reaches the Manifest through the
    // `AdmissionResult` (SB-38): recording the stray there kept both harms the refusal
    // exists to prevent — a substitution of a value nobody requested in the document,
    // and that key's own policy default firing for a key nobody named.
    assert!(
        admission.coercions_preview.iter().all(|p| p.coercion.key != key("test.flag")),
        "{:?}",
        admission.coercions_preview
    );
    assert!(
        admission.warnings.iter().all(|w| !format!("{w:?}").contains("test.flag")),
        "{:?}",
        admission.warnings
    );
}

#[test]
fn sb_30_prepare_refuses_an_admission_result_from_another_spec() {
    // SB-30 makes `prepare` a gate in its own right, and `collect_prepare` does not
    // require `plan`'s output — which is why it checks the admission itself. But
    // "admitted" is only "nothing was rejected", which `AdmissionResult::default()`
    // satisfies, so a stale or hand-built result passed: every `matched` lookup then
    // missed and MA-12's narrowing check, SB-44's preview comparison and the constraint
    // re-match were all skipped in silence. SB-39 guards the same hole one stage
    // earlier, which is what makes the gap reachable rather than theoretical.
    let mut req = resource("test.device", &[]);
    req.requires.insert(key("test.grid"), Constraint::Eq { value: Value::Num(40.0) });
    let spec = spec_with([(id("radio"), req)].into_iter().collect());
    let profile = profile_binding(&["radio"]);
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2);
    let providers = one_provider("radio", &p);
    let report = || PrepareReport {
        fragment: id("radio"),
        // A value the Spec never asked for and the node does not declare.
        effective: [(key("test.grid"), Value::Num(999.0))].into_iter().collect(),
        coercions: Vec::new(),
        warnings: Vec::new(),
    };
    let real = validate(&spec, &profile, &fx.inputs(&providers)).expect("validates");
    collect_prepare(vec![Ok(report())], &spec, &profile, &fx.inputs(&providers), &real)
        .expect_err("the real admission catches it");

    let foreign = ezsdr_kernel::binding::AdmissionResult::default();
    assert!(foreign.is_admitted(), "an empty result is `admitted`, which is the hole");
    let err = collect_prepare(vec![Ok(report())], &spec, &profile, &fx.inputs(&providers), &foreign)
        .expect_err("and an admission that is not this Spec's is refused, not believed");
    let ezsdr_kernel::plan::PrepareError::Violations(v) = err else { panic!("violations") };
    assert!(v.iter().any(|x| x.reason.contains("not this Spec's")), "{v:?}");
}

#[test]
fn sb_6_equality_is_exact_and_the_matcher_agrees() {
    // SB-6 knows **one** relation for "these are one value": exact numeric
    // comparison. `Eq`, `Set`, `Range`, `Min`, `Max`, `PartialEq` and SC-2's
    // "identical re-registration" all decide by it, so none of them can disagree.
    //
    // Canonical form is **document** identity, not value identity, and the two
    // coincide only while |v| <= 2^53 (OV-15a). Above that a float's canonical text
    // is the shortest decimal that *names the f64*, not the number's exact decimal —
    // so deciding equality by the form was wrong in both directions, and the second
    // direction is the dangerous one: it called two different numbers equal.
    let two60 = 1_152_921_504_606_846_976_i64;
    let shortest_naming_two60 = 1_152_921_504_606_847_000_i64;
    let cases: &[(i64, f64, bool)] = &[
        (1, 1.0, true),
        (10_000_000_000_000_000, 1e16, true),
        // One number, two canonical forms and two hashes. Still one value.
        (two60, two60 as f64, true),
        // One canonical form, two numbers. `ContractRegistry::register` took this for
        // an identical re-registration and discarded the other definition (SC-2).
        (shortest_naming_two60, two60 as f64, false),
        (i64::MAX, i64::MAX as f64, false),
        (3, 3.5, false),
    ];
    let written = |v: &Value| {
        ezsdr_kernel::hash::canonical_json(&serde_json::to_value(v).expect("serialises"))
            .expect("finite")
    };
    for &(i, f, want) in cases {
        let (a, b) = (Value::Int(i), Value::Num(f));
        assert_eq!(a == b, want, "Value {i} vs {f}");
        assert_eq!(
            satisfies(&Constraint::Eq { value: b.clone() }, &CapabilityValue::One { value: a.clone() })
                .expect("both are numbers"),
            want,
            "Eq {f} against a declared {i}"
        );
        // A degenerate `Range` is the same question asked of the ordering, so it must
        // give the same answer — the split between "equality" and "ordering" is what
        // let `Eq` and `Range{v,v}` disagree.
        assert_eq!(
            satisfies(
                &Constraint::Range { min: Some(b.clone()), max: Some(b.clone()) },
                &CapabilityValue::One { value: a.clone() }
            )
            .expect("both are numbers"),
            want,
            "Range[{f},{f}] against a declared {i}"
        );
        assert_eq!(
            ezsdr_kernel::contract::Scalar::Int(i) == ezsdr_kernel::contract::Scalar::Float(f),
            want,
            "Scalar {i} vs {f}"
        );
    }

    // OV-15a's coincidence, stated as the fact it is rather than as the criterion:
    // one value has one hash while |v| <= 2^53, and above it may not.
    assert_eq!(written(&Value::Int(1)), written(&Value::Num(1.0)), "one value, one hash");
    assert_eq!(
        written(&Value::Int(10_000_000_000_000_000)),
        written(&Value::Num(1e16)),
        "and above 2^53 the forms may still agree"
    );
    assert_ne!(
        written(&Value::Int(two60)),
        written(&Value::Num(two60 as f64)),
        "but they need not: one value, two documents"
    );
    assert_eq!(
        written(&Value::Int(shortest_naming_two60)),
        written(&Value::Num(two60 as f64)),
        "and one document may name two values, which is why the form is not the criterion"
    );
}

#[test]
fn sb_11_the_declared_vocabulary_major_is_checked() {
    // SB-11: `requirements.vocabularies` lists "the Vocabulary majors this Spec's keys
    // belong to". Nothing read `major`, so the field was a declaration with no reader
    // and a Spec written against a future major validated against the registered one —
    // the shape D40 withdrew `constraints_hit` for.
    let mut spec = minimal_spec();
    let p = TestProvider::new("radio", 2);
    let providers = one_provider("radio", &p);
    let profile = profile_binding(&["radio"]);
    let fx = Fixture::new();
    validate(&spec, &profile, &fx.inputs(&providers)).expect("major 1 is what is registered");

    spec.requirements.vocabularies[0].major = 2;
    let err = validate(&spec, &profile, &fx.inputs(&providers))
        .expect_err("a Spec's keys belong to a major the registry does not have");
    assert!(
        matches!(&err, SpecError::Structural { reason }
            if reason.contains("SB-11") && reason.contains("major 2")),
        "{err:?}"
    );

    // An unregistered id is refused before this, by SB-2's prefix check: the Spec's
    // keys then belong to no listed Vocabulary at all.
    spec.requirements.vocabularies[0].id = ns("absent");
    spec.requirements.vocabularies[0].major = 1;
    assert!(matches!(
        validate(&spec, &profile, &fx.inputs(&providers)),
        Err(SpecError::UnknownKeyPrefix { .. })
    ));
}

#[test]
fn rs_52_a_scheduled_update_states_the_declared_class() {
    // RS-52: "`UpdateParameter.class` is the parameter's declared update class", and
    // "an Action whose class was not declared is rejected at admission (RS-17)". For a
    // Spec the admission stage is `validate`. Nothing compared the two, so a schedule
    // entry could state `hardware_timed` for a key that declares no class at all and
    // the Action reached `arm` carrying a class nobody declared.
    let p = TestProvider::new("radio", 2);
    let providers = one_provider("radio", &p);
    let profile = profile_binding(&["radio"]);
    let fx = Fixture::new();
    let scheduled = |key_name: &str, class| {
        let mut spec = minimal_spec();
        spec.schedule.push(ScheduleEntry {
            at: SpecTime { clock: id("radio"), offset_ticks: 0 },
            action: ActionTemplate::UpdateParameter {
                target: rid("radio"),
                key: key(key_name),
                value: Value::Int(1),
                class,
            },
        });
        spec
    };
    // `test.gain` is the fixture's changeable Provider parameter (D33), declared
    // `HardwareTimed`.
    let declared = fx
        .registry
        .key_decl(&key("test.gain"))
        .expect("the fixture declares it")
        .update_class
        .expect("with a class");
    validate(&scheduled("test.gain", declared), &profile, &fx.inputs(&providers))
        .expect("the entry states the class the key declares");

    let other = UpdateClass::BlockBoundary;
    assert_ne!(other, declared);
    let err = validate(&scheduled("test.gain", other), &profile, &fx.inputs(&providers))
        .expect_err("a class other than the declared one is rejected");
    assert!(
        matches!(&err, SpecError::Structural { reason } if reason.contains("RS-52")),
        "{err:?}"
    );

    // SB-2: the `KeyDecl` is "the only place it can be", because a schedule entry's
    // target is a Spec resource by SB-16 and "a Provider's parameters are Vocabulary
    // keys and never a `ComponentDescriptor`'s `params`". Consulting `graph.components`
    // first made an unrelated component's parameter list decide a Provider key's class
    // — in both directions.
    let with_component = |key_name: &str, param: UpdateClass, stated: UpdateClass| {
        let mut spec = scheduled(key_name, stated);
        let mut c = support::recorder_component(
            ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.cf32").expect("id"),
        );
        c.params[0].key = key(key_name);
        c.params[0].update_class = param;
        c.params[0].default = Value::Num(0.0);
        c.params[0].schema = serde_json::json!({ "type": "number" });
        spec.graph.components.insert(id("proc"), c);
        spec
    };
    // A component cannot lend a class to a key whose Vocabulary declares none.
    let err = validate(
        &with_component("test.grid", UpdateClass::BlockBoundary, UpdateClass::BlockBoundary),
        &profile,
        &fx.inputs(&providers),
    )
    .expect_err("a component's params do not declare a Provider key's class");
    assert!(
        matches!(&err, SpecError::Structural { reason }
            if reason.contains("RS-52") && reason.contains("declares no update class")),
        "{err:?}"
    );
    // Nor can it shadow the class a key does declare.
    validate(
        &with_component("test.gain", UpdateClass::BlockBoundary, declared),
        &profile,
        &fx.inputs(&providers),
    )
    .expect("the Vocabulary's declaration is the one that counts");

    // `test.count` is a declared key with no `update_class`, which RS-52 calls "not
    // declared" — the case that must not reach a Module.
    let err = validate(&scheduled("test.count", declared), &profile, &fx.inputs(&providers))
        .expect_err("a key with no declared class cannot be updated");
    assert!(
        matches!(&err, SpecError::Structural { reason }
            if reason.contains("RS-52") && reason.contains("declares no update class")),
        "{err:?}"
    );
}

#[test]
fn sb_15a_link_direction_is_checked() {
    // SB-15a: a link's `from` names an `out` port and its `to` an `in` port, and the
    // check runs **before** SC-3, because a producer-to-consumer contract check is
    // well posed only once which end is which has been established. `Port.direction`
    // was carried through the whole compile path and never read, so `a.in → b.in`
    // validated and planned and SC-3 was evaluated on an orientation nothing had
    // checked (finding D47).
    let cf32 = ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.cf32").expect("id");
    let linked = |from: (&str, &str), to: (&str, &str)| {
        let mut spec = minimal_spec();
        spec.graph.components.insert(id("src"), support::source_component(cf32.clone()));
        spec.graph.components.insert(id("dst"), support::recorder_component(cf32.clone()));
        spec.graph.links.push(ezsdr_kernel::spec::LinkReq {
            from: PortRef { component: from.0.into(), port: from.1.into() },
            to: PortRef { component: to.0.into(), port: to.1.into() },
            policy: BackPressure::DropOldest,
            capacity: 4,
        });
        spec
    };
    let mut profile = profile_binding(&["radio"]);
    for c in ["src", "dst"] {
        profile.placements.components.insert(
            id(c),
            ComponentPlacement { island: id("io"), memory_domain: MemoryDomainId::local(0) },
        );
    }
    profile.placements.islands.push(IslandDecl {
        id: IslandId::local(0),
        executor: id("exec"),
        components: vec![id("src"), id("dst")],
        affinity: None,
        rt_policy: None,
        batch: None,
    });
    let fx = Fixture::new();
    let p = TestProvider::new("radio", 2);
    let providers = one_provider("radio", &p);

    validate(&linked(("src", "out"), ("dst", "in")), &profile, &fx.inputs(&providers))
        .expect("out -> in is the one orientation a link has");

    for (from, to, why) in [
        (("dst", "in"), ("dst", "in"), "a consumer cannot produce"),
        (("src", "out"), ("src", "out"), "a producer cannot consume"),
    ] {
        let err = validate(&linked(from, to), &profile, &fx.inputs(&providers))
            .expect_err(why);
        assert!(
            matches!(&err, SpecError::Structural { reason } if reason.contains("SB-15a")),
            "{why}: {err:?}"
        );
    }

    // And on a bound resource's port, which is the endpoint SB-15 added: the double's
    // line declares `rx` as `out` and `tx` as `in`.
    let mut spec = minimal_spec();
    spec.resources.insert(id("line0"), resource("test.line", &[]));
    spec.graph.components.insert(id("dst"), support::recorder_component(
        ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.sc16").expect("id"),
    ));
    spec.graph.links.push(ezsdr_kernel::spec::LinkReq {
        from: PortRef { component: "line0".into(), port: "tx".into() },
        to: PortRef { component: "dst".into(), port: "in".into() },
        policy: BackPressure::DropOldest,
        capacity: 4,
    });
    let mut profile = profile_binding(&["radio", "line0"]);
    profile.placements.components.insert(
        id("dst"),
        ComponentPlacement { island: id("io"), memory_domain: MemoryDomainId::local(0) },
    );
    profile.placements.islands.push(IslandDecl {
        id: IslandId::local(0),
        executor: id("exec"),
        components: vec![id("dst")],
        affinity: None,
        rt_policy: None,
        batch: None,
    });
    let providers: BTreeMap<Ident, &dyn Provider> =
        [(id("radio"), &p as &dyn Provider), (id("line0"), &p as &dyn Provider)]
            .into_iter()
            .collect();
    let err = validate(&spec, &profile, &fx.inputs(&providers))
        .expect_err("a resource's `tx` is where samples arrive, not where they leave");
    assert!(
        matches!(&err, SpecError::Structural { reason } if reason.contains("SB-15a")),
        "{err:?}"
    );
}
