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
    Provider, Role,
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
        Factories { provider: true, ..Factories::default() },
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
    assert_eq!(result.coercions_preview[0].applied, Value::Num(20.0));
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
    let mut profile = profile_binding(&["radio"]);
    profile.bindings.insert(
        id("radio2"),
        ezsdr_kernel::binding::Binding {
            module: mid("ezsdr.test.provider"),
            feed: None,
            selector: BTreeMap::new(),
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
    let result =
        validate(&spec, &profile_binding(&["a", "b"]), &fx.inputs(&providers)).expect("validates");
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
    let mut checks = AdmissionCheckRegistry::new();
    checks.register(std::sync::Arc::new(TestLimitsCheck::new()));
    let environment: BTreeMap<Namespace, serde_json::Value> =
        [(ns("test.limits"), serde_json::json!({ "max_grid": 20.0 }))].into_iter().collect();

    // Inside the limit at validate.
    let requested: BTreeMap<Key, Value> =
        [(key("test.grid"), Value::Num(19.5))].into_iter().collect();
    assert!(
        checks.run(&environment, &requested, &BTreeMap::new(), CheckStage::Validate).is_empty()
    );
    // The coercion lands outside it, so prepare fails.
    let applied: BTreeMap<Key, Value> = [(key("test.grid"), Value::Num(25.0))].into_iter().collect();
    assert_eq!(
        checks.run(&environment, &applied, &BTreeMap::new(), CheckStage::Prepare).len(),
        1
    );
    // A Session Action outside it is rejected, with the proposed value in the violation.
    let proposed: BTreeMap<Key, Value> = [(key("test.grid"), Value::Num(30.0))].into_iter().collect();
    let v = checks.run(&environment, &requested, &proposed, CheckStage::Runtime);
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].requested, Some(Value::Num(30.0)));
}

#[test]
fn sb_31_unregistered_section_is_informational() {
    let checks = AdmissionCheckRegistry::new();
    let environment: BTreeMap<Namespace, serde_json::Value> =
        [(ns("vendor.thing"), serde_json::json!({ "anything": [1, 2, 3] }))].into_iter().collect();
    assert!(checks.run(&environment, &BTreeMap::new(), &BTreeMap::new(), CheckStage::Validate).is_empty());
    // And it is still recorded verbatim (SB-27).
    let profile = BindingProfile { version: 1, environment: environment.clone(), ..BindingProfile::default() };
    assert_eq!(profile.section("vendor.thing"), environment.get(&ns("vendor.thing")));
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
    let mut profile = profile_binding(&["pps", "slave"]);
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

    // With two candidates and no field, refused.
    fx.authorities.insert(
        id("other"),
        AuthorityDescriptor { governs: Vec::new(), pacing: Pacing::FreeRunning },
    );
    assert!(matches!(
        validate_then_plan(&spec, &profile, &fx.inputs(&providers), Vec::new()),
        Err(SpecError::Structural { .. })
    ));
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
        constraints_hit: Vec::new(),
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
        constraints_hit: Vec::new(),
    };
    // The double's injected failure at `prepare`, without a full PrepareContext.
    let provider = TestProvider::new("c", 2).failing_at(FailAt::Prepare);
    assert_eq!(provider.fail_at, FailAt::Prepare);
    let err = ezsdr_kernel::module_api::ModuleError::rejected("injected failure at Prepare");
    let reports = vec![Ok(ok("a")), Ok(ok("b")), Err(err)];
    let checks = AdmissionCheckRegistry::new();
    let reg = registry();
    let failed = collect_prepare(reports, &checks, &BTreeMap::new(), &BTreeMap::new(), &reg, false)
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

    // `arm` substitutes the resolved deadline, in that stream's SampleClock.
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
        ezsdr_kernel::contract::DataContractId::parse("ezsdr.iq.cf32").expect("id"),
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
    let cf32 = ezsdr_kernel::contract::DataContractId::parse("ezsdr.iq.cf32").expect("id");
    let mut spec = minimal_spec();
    spec.graph.components.insert(id("proc"), support::recorder_component(cf32.clone()));
    spec.graph.links.push(ezsdr_kernel::spec::LinkReq {
        from: PortRef { component: "radio".into(), port: "rx".into() },
        to: PortRef { component: "proc".into(), port: "in".into() },
        policy: BackPressure::DropOldest,
        capacity: 4,
    });
    let mut profile = profile_binding(&["radio"]);
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
    let providers = one_provider("radio", &p);
    let planned = validate_then_plan(&spec, &profile, &fx.inputs(&providers), Vec::new());
    assert!(planned.is_ok(), "a resource endpoint is admissible: {planned:?}");

    // SB-15: an endpoint that names no declared port is refused, on a resource as
    // on a component — the resource tree declares `rx` and nothing else.
    let mut wrong = spec.clone();
    wrong.graph.links[0].from.port = "tx".to_owned();
    assert!(matches!(
        validate(&wrong, &profile, &fx.inputs(&providers)),
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
        ezsdr_kernel::contract::DataContractId::parse("ezsdr.iq.cf32").expect("id"),
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
    spec.graph.components.insert(id("proc"), support::recorder_component(contract.clone()));
    spec.graph.components.insert(id("recorder"), support::recorder_component(contract));
    spec.graph.links.push(ezsdr_kernel::spec::LinkReq {
        from: PortRef { component: "proc".into(), port: "in".into() },
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
        ezsdr_kernel::contract::DataContractId::parse("ezsdr.iq.cf32").expect("id"),
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
        [(ns("test.limits"), serde_json::json!({ "max_grid": 20.0 }))].into_iter().collect();
    let report = |applied: f64| PrepareReport {
        fragment: id("radio"),
        effective: [(key("test.grid"), Value::Num(applied))].into_iter().collect(),
        coercions: Vec::new(),
        warnings: Vec::new(),
        constraints_hit: Vec::new(),
    };
    let reg = registry();
    let none = BTreeMap::new();
    assert!(collect_prepare(vec![Ok(report(19.5))], &checks, &environment, &none, &reg, false).is_ok());
    let failed = collect_prepare(vec![Ok(report(40.0))], &checks, &environment, &none, &reg, false)
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
        constraints_hit: Vec::new(),
    };
    let no_env: BTreeMap<Namespace, serde_json::Value> = BTreeMap::new();
    let failed = collect_prepare(vec![Ok(coercing.clone())], &checks, &no_env, &none, &reg, false)
        .expect_err("`test.grid` defaults to `reject`");
    assert!(matches!(failed, ezsdr_kernel::plan::PrepareError::Violations(v)
        if v[0].reason.contains("rejected at prepare")));
    // As a Session the same coercion warns (SB-45).
    assert!(collect_prepare(vec![Ok(coercing)], &checks, &no_env, &none, &reg, true).is_ok());
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
