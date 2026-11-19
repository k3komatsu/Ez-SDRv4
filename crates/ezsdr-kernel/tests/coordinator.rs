mod support;

use ezsdr_kernel::spec::Scalar;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use ezsdr_kernel::binding::BindingProfile;
use ezsdr_kernel::coordinator::{Assembly, connect, start_probe, start_spec_run};
use ezsdr_kernel::event::{Action, EventSource, Target};
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::ClockDomainId;
use ezsdr_kernel::manifest::RunKind;
use ezsdr_kernel::module_api::{ActionSubmitter, Pacing, UpdateClass};
use ezsdr_kernel::run::{CleanupMode, Lease, LeaseMode, RunState, Stage, StopCause, Termination};
use ezsdr_kernel::session::{Outcome, SessionAction};
use ezsdr_kernel::spec::{ExperimentSpec, Ident, Key, SpecError, Value};
use ezsdr_kernel::time::{ClockDomain, ClockRegistry, Duration, EpochRef, ManualTimeAuthority, Rational, TimePoint};
use support::start_window::{
    Call, CallKind, ClockLog, Hooks, LogEvent, OpenOnDrop, StartTrace, WatchedAuthority, WatchedClock, far_future_stop, gate, infra, open, seen_for, start_on_worker, sweep, wait, with_seen,
};
use support::{
    FakeHostClock, FinishChecked, Probe, ProbeExecutor, RecordingSink, ScalarOnlyCheck, SimAuthority, SteppedProvider, TestLimitsCheck, TestLinkModule, TestProvider, mref, ns, run_checks, run_kinds,
    run_registry, run_registry_classed, run_registry_non_namespace_provider,
};

#[allow(dead_code)]
struct Rig {
    clocks: Arc<ClockRegistry>,
    root: ClockDomainId,
    manual: Arc<ManualTimeAuthority>,
    host: Arc<FakeHostClock>,
    assembly: Assembly,
}

fn rig(pacing: Pacing) -> Rig {
    let clocks = Arc::new(ClockRegistry::new());
    let (authority, root) = SimAuthority::new(&clocks, mref("ezsdr.test.provider"), pacing);
    let manual = authority.manual();
    let host = Arc::new(FakeHostClock::new());
    let mut assembly = Assembly::new(Box::new(authority), clocks.clone());
    assembly.registry = run_registry();
    assembly.checks = run_checks(false);
    assembly.kinds = run_kinds();
    assembly.host_clock = host.clone();
    Rig { clocks, root, manual, host, assembly }
}

fn rig_with_faulting_time(fault: impl FnOnce(SimAuthority) -> SimAuthority) -> Rig {
    let mut rig = rig(Pacing::FreeRunning);
    let (authority, root) = SimAuthority::new(&rig.clocks, mref("ezsdr.test.provider"), Pacing::FreeRunning);
    rig.root = root;
    rig.manual = authority.manual();
    rig.assembly.authority = Box::new(fault(authority));
    rig
}

fn spec_one() -> serde_json::Value {
    serde_json::json!({
        "version": 1,
        "requirements": { "vocabularies": [{ "id": "test", "major": 1 }] },
        "resources": {
            "radio": {
                "kind": "test.device",
                "requires": { "test.count": { "kind": "eq", "value": 2 } }
            }
        }
    })
}

fn profile_one() -> serde_json::Value {
    serde_json::json!({
        "version": 1,
        "bindings": {
            "radio": { "module": { "id": "ezsdr.test.provider", "version": { "major": 1, "minor": 0, "patch": 0 } } }
        },
        "authority": "radio"
    })
}

fn with_provider(mut assembly: Assembly, name: &str, path: &str) -> Assembly {
    assembly.providers.insert(Ident::parse(name).expect("fixture name"), Box::new(TestProvider::new(path, 2)));
    assembly
}

/// The reason of a Run that ended `Failed` (KC-7).
fn failure(manifest: &ezsdr_kernel::manifest::Manifest) -> &str {
    match &manifest.termination.reason {
        Termination::Failed { reason, .. } => reason,
        other => panic!("the Run did not fail: {other:?}"),
    }
}

fn distinct_resource_docs(names: &[&str]) -> (serde_json::Value, serde_json::Value) {
    let resources: serde_json::Map<String, serde_json::Value> =
        names.iter().map(|name| ((*name).to_owned(), serde_json::json!({ "kind": "test.device", "requires": { "test.count": { "kind": "eq", "value": 2 } } }))).collect();
    let bindings: serde_json::Map<String, serde_json::Value> = names
        .iter()
        .map(|name| {
            (
                (*name).to_owned(),
                serde_json::json!({
                    "module": { "id": "ezsdr.test.provider", "version": { "major": 1, "minor": 0, "patch": 0 } },
                    "selector": { "slot": name }
                }),
            )
        })
        .collect();
    (
        serde_json::json!({
            "version": 1,
            "requirements": { "vocabularies": [{ "id": "test", "major": 1 }] },
            "resources": resources
        }),
        serde_json::json!({ "version": 1, "bindings": bindings, "authority": names[0] }),
    )
}

fn output_docs() -> (serde_json::Value, serde_json::Value) {
    let spec = serde_json::json!({
        "version": 1,
        "requirements": { "vocabularies": [{ "id": "test", "major": 1 }] },
        "resources": { "radio": { "kind": "test.device", "requires": { "test.count": { "kind": "eq", "value": 2 } } } },
        "outputs": [{
            "id": "rec", "kind": "test.capture", "params": {},
            "feed": { "port": { "component": "radio", "port": "rx" }, "policy": "drop_oldest", "capacity": 4 }
        }]
    });
    let profile = serde_json::json!({
        "version": 1,
        "bindings": {
            "radio": { "module": { "id": "ezsdr.test.provider", "version": { "major": 1, "minor": 0, "patch": 0 } } },
            "rec": { "module": { "id": "ezsdr.test.sink", "version": { "major": 1, "minor": 0, "patch": 0 } } }
        },
        "authority": "radio",
        "placements": { "links": [{
            "link": { "id": "ezsdr.test.link", "version": { "major": 1, "minor": 0, "patch": 0 } },
            "from": { "component": "radio", "port": "rx" },
            "to": { "component": "rec", "port": "in" }
        }] }
    });
    (spec, profile)
}

fn output_assembly(probe: &Probe, every: Option<i64>, descriptor: Option<ezsdr_kernel::module_api::LinkDescriptor>) -> Assembly {
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    let mut provider = SteppedProvider::new("p", TestProvider::new("radio", 2), probe);
    if let Some(interval) = every {
        provider = provider.publishing_every(interval);
    }
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(provider));
    assembly.sinks.insert(Ident::parse("rec").unwrap(), Box::new(RecordingSink::new("rec", probe)));
    let link = descriptor.map_or_else(|| TestLinkModule::new(probe), |descriptor| TestLinkModule::new(probe).with_descriptor(descriptor));
    assembly.links.insert(mref("ezsdr.test.link"), Box::new(link));
    assembly
}

fn add_schedule(spec: &mut serde_json::Value, clock: &str, offset: i64, action: serde_json::Value) {
    spec["schedule"] = serde_json::json!([{
        "at": { "clock": clock, "offset_ticks": offset },
        "action": action
    }]);
}

fn tx_template(target: &str, waveform: &ezsdr_kernel::manifest::ArtifactRef, late: &str) -> serde_json::Value {
    serde_json::json!({
        "kind": "tx_burst",
        "target": serde_json::to_value(support::target(target)).unwrap(),
        "waveform": waveform,
        "repeat": false,
        "late_policy": late,
        "metadata": {}
    })
}

fn input_ref() -> (Vec<u8>, ezsdr_kernel::manifest::ArtifactRef) {
    let bytes = vec![0u8; 80];
    let hash = ezsdr_kernel::hash::ContentHash::of_bytes(&bytes);
    let reference = ezsdr_kernel::manifest::ingest_input(Ident::parse("waveform").unwrap(), ns("ezsdr.input"), format!("mem:{hash}"), &bytes);
    (bytes, reference)
}

fn executor_docs() -> (serde_json::Value, serde_json::Value) {
    let mut spec = spec_one();
    let mut component = support::recorder_component(support::cf32());
    component.id = Ident::parse("c1").unwrap();
    component.implementation.id = "c1".to_owned();
    spec["graph"] = serde_json::json!({ "components": { "c1": component } });
    let mut profile = profile_one();
    profile["bindings"]["exec"] = serde_json::json!({
        "module": { "id": "ezsdr.test.executor", "version": { "major": 1, "minor": 0, "patch": 0 } }
    });
    profile["placements"] = serde_json::json!({
        "islands": [{ "id": { "node": 0, "local": 0 }, "executor": "exec",
                      "components": [{ "component": "c1", "memory_domain": { "node": 0, "local": 0 } }] }]
    });
    (spec, profile)
}

fn session_with_provider(provider: Box<dyn ezsdr_kernel::module_api::Provider>) -> ezsdr_kernel::coordinator::RunHandle {
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), provider);
    connect(&profile_one(), &Default::default(), assembly, Lease::attached()).expect("valid Session entry")
}

/// A Session whose radio is `provider`, asking `requests` of it at `connect` (RS-12).
fn session_asking(provider: TestProvider, requests: &[(&str, Scalar)]) -> Result<ezsdr_kernel::coordinator::RunHandle, ezsdr_kernel::spec::SpecError> {
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(provider));
    let initial = [(Ident::parse("radio").unwrap(), requests.iter().map(|(name, value)| (Key::parse(name).unwrap(), value.clone())).collect())].into_iter().collect();
    connect(&profile_one(), &initial, assembly, Lease::attached())
}

#[test]
fn rs_12_initial_requests_reach_the_provider_through_connect() {
    // The implicit Spec's `requires` goes through the whole pipeline: the Provider is asked in
    // validation and again in prepare, and the configuration the Session starts with holds the value.
    let provider = TestProvider::new("radio", 2);
    let calls = provider.coerce_calls.clone();
    let run = session_asking(provider, &[("test.count", Scalar::Int(2))]).expect("a request the capability satisfies connects");
    assert!(matches!(run.state(), RunState::Running {}), "{:?}", run.state());
    assert_eq!(run.effective()[&Ident::parse("radio").unwrap()].get(&Key::parse("test.count").unwrap()), Some(&Value::from(2)));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2, "the Provider is asked once in validation and once in prepare, both with the request (SB-39, SB-41)");
    let plain = session_asking(TestProvider::new("radio", 2), &[]).expect("connects");
    assert!(!plain.effective()[&Ident::parse("radio").unwrap()].contains_key(&Key::parse("test.count").unwrap()), "no request, no value");
    let (with, without) = (run.finish_checked().spec, plain.finish_checked().spec);
    assert_ne!(with.hash, without.hash, "the Manifest's Spec is the one that carries the request");
    assert_eq!(with.body["resources"]["radio"]["requires"]["test.count"], serde_json::json!({ "kind": "eq", "value": 2 }));
    assert_eq!(with.body["requirements"]["vocabularies"], serde_json::json!([{ "id": "test", "major": 1 }]));
}

#[test]
fn rs_12_an_initial_request_is_judged_like_a_spec_requires() {
    // Validation refuses what no capability satisfies and the Provider cannot coerce (SB-6,
    // SB-7), and a coercible value follows a Session's policy, `warn` (SB-45): applied, with the
    // coercion and a warning, where a Spec Run's `reject` would refuse it.
    let refused = session_asking(TestProvider::new("radio", 2), &[("test.count", Scalar::Int(3))]).expect("a refused Session is a failed Run, not an Err");
    assert!(matches!(refused.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Validate, .. } }), "{:?}", refused.state());
    let _ = refused.finish_checked();
    let provider = TestProvider::new("radio", 2).with_grid(20.0);
    let calls = provider.coerce_calls.clone();
    let run = session_asking(provider, &[("test.grid", Scalar::Num(ezsdr_kernel::spec::Finite::new(19.0).unwrap()))]).expect("connects");
    assert!(matches!(run.state(), RunState::Running {}), "{:?}", run.state());
    assert_eq!(run.effective()[&Ident::parse("radio").unwrap()][&Key::parse("test.grid").unwrap()], Value::num(20.0).unwrap(), "the coerced value is what the Session starts with");
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2, "once in validation and once in prepare");
    let manifest = run.finish_checked();
    let report = &manifest.prepare.reports[&Ident::parse("radio").unwrap()];
    assert!(report.coercions.iter().any(|c| c.key == Key::parse("test.grid").unwrap() && c.requested == Value::num(19.0).unwrap() && c.applied == Value::num(20.0).unwrap()), "{:?}", report.coercions);
}

#[test]
fn ma_07_an_instance_with_two_fragments_is_prepared_twice_and_armed_once() {
    let (spec, profile) = pair_docs();
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("a").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("dev", 2), &probe)));
    let run = start_spec_run(&spec, &profile, assembly).expect("entry creates a Run");
    let _ = run.finish_checked();
    assert_eq!(probe.with_prefix("p:"), vec!["p:prepare:a", "p:prepare:b", "p:arm", "p:start:0", "p:step:0", "p:now:0", "p:stop:Orderly", "p:cleanup",]);
}

#[test]
fn kc_10_link_descriptor_must_equal_the_registered_one() {
    let (spec, profile) = output_docs();
    let probe = Probe::new();
    let mut descriptor = support::test_link_descriptor();
    descriptor.kind = ns("test.other");
    let run = start_spec_run(&spec, &profile, output_assembly(&probe, None, Some(descriptor))).expect("entry creates a Run");
    assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Plan, .. } }));
    assert!(failure(&run.finish_checked()).starts_with("KC-10: MA-27a"));
}

#[test]
fn kc_10_both_ends_are_attached() {
    let (spec, profile) = output_docs();
    let probe = Probe::new();
    let mut run = start_spec_run(&spec, &profile, output_assembly(&probe, Some(100), None)).expect("entry creates a Run");
    run.advance_to(ezsdr_kernel::time::TimePoint::new(run.now().domain(), 250)).expect("advances");
    let manifest = run.finish_checked();
    assert!(probe.lines().iter().any(|line| line == "p:links:radio.rx:out"));
    assert!(probe.lines().iter().any(|line| line == "rec:links:rec.in:in"));
    assert!(probe.lines().iter().any(|line| line == "rec:block:100"));
    assert!(probe.lines().iter().any(|line| line == "rec:block:200"));
    assert_eq!(manifest.links[0].drops, Some(0));
}

#[test]
fn kc_11_an_island_gets_exactly_its_components() {
    let mut components = serde_json::Map::new();
    for name in ["c1", "c2", "c3"] {
        let mut component = support::recorder_component(support::cf32());
        component.id = Ident::parse(name).unwrap();
        component.implementation.id = name.to_owned();
        components.insert(name.to_owned(), serde_json::to_value(component).unwrap());
    }
    let spec = serde_json::json!({
        "version": 1,
        "requirements": { "vocabularies": [{ "id": "test", "major": 1 }] },
        "resources": { "radio": { "kind": "test.device", "requires": { "test.count": { "kind": "eq", "value": 2 } } } },
        "graph": { "components": components }
    });
    let profile = serde_json::json!({
        "version": 1,
        "bindings": {
            "radio": { "module": { "id": "ezsdr.test.provider", "version": { "major": 1, "minor": 0, "patch": 0 } } },
            "exec": { "module": { "id": "ezsdr.test.executor", "version": { "major": 1, "minor": 0, "patch": 0 } } }
        },
        "authority": "radio",
        "placements": {
            "islands": [
                { "id": { "node": 0, "local": 0 }, "executor": "exec", "components": [
                    { "component": "c1", "memory_domain": { "node": 0, "local": 0 } },
                    { "component": "c2", "memory_domain": { "node": 0, "local": 0 } }
                ] },
                { "id": { "node": 0, "local": 1 }, "executor": "exec", "components": [
                    { "component": "c3", "memory_domain": { "node": 0, "local": 0 } }
                ] }
            ]
        }
    });
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly = with_provider(assembly, "radio", "radio");
    assembly.executors.insert(Ident::parse("exec").unwrap(), Box::new(ProbeExecutor::new("x", &probe)));
    let run = start_spec_run(&spec, &profile, assembly).expect("entry creates a Run");
    let _ = run.finish_checked();
    assert!(probe.lines().iter().any(|line| line == "x:prepare:island_0:c1,c2"));
    assert!(probe.lines().iter().any(|line| line == "x:prepare:island_1:c3"));
}

#[test]
fn kc_12_a_prepare_failure_stops_the_loop_and_cleans_up_what_was_prepared() {
    let (spec, profile) = distinct_resource_docs(&["a", "b", "c"]);
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    for name in ["a", "b", "c"] {
        let mut inner = TestProvider::new(name, 2);
        if name == "b" {
            inner = inner.failing_at(support::FailAt::Prepare);
        }
        assembly.providers.insert(Ident::parse(name).unwrap(), Box::new(SteppedProvider::new(name, inner, &probe)));
    }
    let run = start_spec_run(&spec, &profile, assembly).expect("entry creates a Run");
    assert!(
        matches!(
            run.state(),
            RunState::CleanedUp {
                termination: Termination::Failed { stage: Stage::Prepare, ref reason }
            } if reason.starts_with("KC-12: fragment b: ")
        ),
        "{:?}",
        run.state()
    );
    let _ = run.finish_checked();
    let lines = probe.lines();
    assert!(!lines.iter().any(|line| line.starts_with("c:prepare:")));
    assert!(!lines.iter().any(|line| line == "c:stop:Abort" || line == "c:cleanup"));
    assert!(lines.iter().any(|line| line == "a:stop:Abort"));
    assert!(lines.iter().any(|line| line == "b:stop:Abort"));
    assert!(lines.iter().any(|line| line == "a:cleanup"));
    assert!(lines.iter().any(|line| line == "b:cleanup"));
}

#[test]
fn sb_4_a_provider_effective_map_with_a_non_ascii_key_is_recorded_and_the_manifest_reads_back() {
    // `test.zeta` is a key the node does not declare, so MA-12's narrowing check skips
    // it and the value reaches the Manifest's prepare report as a Provider reported it.
    let spec = spec_one();
    let profile = profile_one();
    let probe = Probe::new();
    let mut assembly = rig(Pacing::FreeRunning).assembly;
    let non_ascii = Value::Map([("radio.\u{fc}".to_owned(), Scalar::Int(1))].into_iter().collect());
    assembly
        .providers
        .insert(Ident::parse("radio").expect("fixture name"), Box::new(SteppedProvider::new("radio", TestProvider::new("radio", 2).with_effective("test.zeta", non_ascii.clone()), &probe)));
    let run = start_spec_run(&spec, &profile, assembly).expect("entry creates a Run");
    assert!(matches!(run.state(), RunState::Running {}), "{:?}", run.state());
    let manifest = run.finish_checked();
    assert!(manifest.hash.is_some());
    assert!(manifest.termination.cleanup_failures.is_empty(), "{:?}", manifest.termination.cleanup_failures);
    let report = &manifest.prepare.reports[&Ident::parse("radio").expect("fragment")];
    assert_eq!(report.effective[&Key::parse("test.zeta").expect("key")], non_ascii);
}

#[test]
fn sb_6_a_provider_effective_of_the_wrong_kind_for_an_undeclared_capability_is_refused_at_prepare() {
    // P1: `test.gain` has a `KeyDecl` (`Num`) but is in neither the bound node's
    // `capabilities` nor the resource's `requires`, so MA-12's narrowing and its
    // constraint re-match both skip it. A value a Provider *reports* is still a
    // value a key carries (SB-2, SB-6).
    let spec = spec_one();
    let profile = profile_one();
    let wrong = Value::from("loud");
    let mut assembly = rig(Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").expect("fixture name"), Box::new(TestProvider::new("radio", 2).with_effective("test.gain", wrong.clone())));
    let run = start_spec_run(&spec, &profile, assembly).expect("entry creates a Run");
    assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Prepare, .. } }), "{:?}", run.state());
    assert!(run.effective().values().all(|effective| effective.get(&Key::parse("test.gain").unwrap()) != Some(&wrong)), "the wrong-kind value is not applied: {:?}", run.effective());
    let manifest = run.finish_checked();
    let reason = failure(&manifest);
    assert!(reason.contains("SB-6:") && reason.contains("test.gain") && reason.contains("Num") && reason.contains("Str"), "{reason}");
    assert!(reason.contains("loud"), "{reason}");
    assert!(manifest.prepare.reports.values().all(|report| report.effective.get(&Key::parse("test.gain").unwrap()) != Some(&wrong)), "{:?}", manifest.prepare.reports);
}

#[test]
fn sb_6_an_executor_prepare_effective_of_the_wrong_kind_is_refused() {
    // P3: the loop that judges `effective` runs only over `spec.resources` with a
    // Provider, so every Executor's and Sink's report passed unchecked. `test.flag`
    // is `Bool` and `test.item0` is `Int` (`test.item`'s numbered declaration); the
    // latter's `Warn` default keeps SB-46's policy quiet, so only SB-6 refuses them.
    let (spec, profile) = executor_docs();
    let probe = Probe::new();
    let mut assembly = rig(Pacing::FreeRunning).assembly;
    assembly = with_provider(assembly, "radio", "radio");
    assembly.executors.insert(
        Ident::parse("exec").unwrap(),
        Box::new(ProbeExecutor::new("x", &probe).with_effective("test.flag", Value::from("not a bool")).with_coercion("test.item0", Value::from(3), Value::from("three"))),
    );
    let run = start_spec_run(&spec, &profile, assembly).expect("entry creates a Run");
    assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Prepare, .. } }), "{:?}", run.state());
    let manifest = run.finish_checked();
    let reason = failure(&manifest);
    assert!(reason.contains("SB-6:") && reason.contains("test.flag") && reason.contains("test.item0"), "{reason}");
    assert!(reason.contains("Bool") && reason.contains("Int"), "{reason}");
    assert!(reason.contains("not a bool"), "{reason}");
    assert!(manifest.prepare.reports.is_empty(), "a failed prepare files no report: {:?}", manifest.prepare.reports);
}

#[test]
fn sb_6_a_wrong_kind_report_ends_the_run_with_the_sb_6_violation_not_a_check_panic() {
    // F1 at the Run level: the wrong-kind value was refused with a violation, but
    // `checks.run` still saw it and the check that reads `test.gain` as a scalar
    // panicked, so the Run ended KC-30 instead of naming SB-6.
    let spec = spec_one();
    let mut profile = profile_one();
    profile["environment"] = serde_json::json!({ "test.scalar": 1 });
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut assembly = rig(Pacing::FreeRunning).assembly;
    assembly.checks.register(Arc::new(ScalarOnlyCheck::new(calls.clone())));
    let wrong = Value::Map([("\u{e9}".to_owned(), Scalar::Int(1))].into_iter().collect());
    assembly.providers.insert(Ident::parse("radio").expect("fixture name"), Box::new(TestProvider::new("radio", 2).with_effective("test.gain", wrong)));
    let run = start_spec_run(&spec, &profile, assembly).expect("entry creates a Run");
    assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Prepare, .. } }), "{:?}", run.state());
    let manifest = run.finish_checked();
    let reason = failure(&manifest);
    assert!(reason.contains("SB-6:") && reason.contains("test.gain"), "{reason}");
    assert!(!reason.contains("panick"), "the check never read the refused report: {reason}");
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0, "the refused report never reaches the check");
}

#[test]
fn sb_41_the_reports_follow_plan_order() {
    // SB-41, KC-12 (spec 20, KH-1): the fragments are prepared in dependency order — `b`
    // before `a` here, against Ident order — which the plan records; the reports are
    // filed under the fragment invoked (spec 27 §3).
    let (spec, mut profile) = distinct_resource_docs(&["a", "b"]);
    profile["environment"] = serde_json::json!({ "ezsdr.arm_order": [{ "before": "b", "after": "a" }] });
    let mut assembly = rig(Pacing::FreeRunning).assembly;
    for name in ["a", "b"] {
        assembly = with_provider(assembly, name, name);
    }
    let manifest = start_spec_run(&spec, &profile, assembly).expect("entry creates a Run").finish_checked();
    let order: Vec<_> = manifest.plan.as_ref().expect("planned").fragments.iter().map(|fragment| fragment.id.as_str()).filter(|id| ["a", "b"].contains(id)).collect();
    assert_eq!(order, ["b", "a"]);
    let keys: Vec<_> = manifest.prepare.reports.keys().map(Ident::as_str).collect();
    assert_eq!(keys, ["a", "b"]);
}

#[test]
fn kc_13_arm_and_start_follow_instance_order_cleanup_reverses_it() {
    let (spec, profile) = distinct_resource_docs(&["a", "b", "c"]);
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    for name in ["a", "b", "c"] {
        let mut inner = TestProvider::new(name, 2);
        if name == "b" {
            inner = inner.arm_after(ezsdr_kernel::id::ResourceId::parse("a").unwrap());
        }
        assembly.providers.insert(Ident::parse(name).unwrap(), Box::new(SteppedProvider::new(name, inner, &probe)));
    }
    let run = start_spec_run(&spec, &profile, assembly).expect("entry creates a Run");
    let _ = run.finish_checked();
    let lines = probe.lines();
    let arms: Vec<_> = lines.iter().filter(|line| line.ends_with(":arm")).map(String::as_str).collect();
    let cleanups: Vec<_> = lines.iter().filter(|line| line.ends_with(":cleanup")).map(String::as_str).collect();
    assert_eq!(arms, ["a:arm", "b:arm", "c:arm"]);
    assert_eq!(cleanups, ["c:cleanup", "b:cleanup", "a:cleanup"]);
}

#[test]
fn kc_15_t0_is_arm_end_plus_the_lead() {
    let spec = spec_one();
    let mut profile = profile_one();
    profile["environment"] = serde_json::json!({
        "ezsdr.time": { "class": "simulation", "start_lead_ns": 2000000 }
    });
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe)));
    let run = start_spec_run(&spec, &profile, assembly).expect("entry creates a Run");
    assert_eq!(run.start_instant(), Some(ezsdr_kernel::time::TimePoint::new(run.now().domain(), 2_000_000)));
    assert!(probe.lines().iter().any(|line| line == "p:start:2000000"));
    let _ = run.finish_checked();
}

#[test]
fn kc_09_an_input_must_be_supplied_and_match_its_hash() {
    fn scheduled(ref_: &ezsdr_kernel::manifest::ArtifactRef) -> serde_json::Value {
        let mut spec = spec_one();
        let target = serde_json::to_value(support::target("radio/tx")).unwrap();
        spec["schedule"] = serde_json::json!([{
            "at": { "clock": "radio", "offset_ticks": 0 },
            "action": { "kind": "tx_burst", "target": target, "waveform": ref_,
                "repeat": false, "late_policy": "send_asap_and_flag", "metadata": {} }
        }]);
        spec
    }
    let bytes = vec![0u8; 80];
    let valid = ezsdr_kernel::manifest::ingest_input(Ident::parse("waveform").unwrap(), ns("ezsdr.input"), format!("mem:{}", ezsdr_kernel::hash::ContentHash::of_bytes(&bytes)), &bytes);
    let run_case = |reference: ezsdr_kernel::manifest::ArtifactRef, stored: Option<Vec<u8>>| {
        let mut assembly = with_provider(rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly, "radio", "radio");
        if let Some(stored) = stored {
            assembly.inputs.insert(reference.hash.clone(), stored);
        }
        start_spec_run(&scheduled(&reference), &profile_one(), assembly).expect("entry creates a Run")
    };
    let missing = run_case(valid.clone(), None);
    assert!(matches!(missing.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Plan, .. } }));
    assert!(failure(&missing.finish_checked()).starts_with("KC-9: "));

    let bad_bytes = run_case(valid.clone(), Some(vec![1u8; 80]));
    assert!(failure(&bad_bytes.finish_checked()).starts_with("KC-9: "));

    let mut bad_size_ref = valid.clone();
    bad_size_ref.size_bytes = 81;
    let bad_size = run_case(bad_size_ref, Some(bytes.clone()));
    assert!(failure(&bad_size.finish_checked()).starts_with("KC-9: "));

    let mut bad_uri_ref = valid.clone();
    bad_uri_ref.uri = "http://x".to_owned();
    let bad_uri = run_case(bad_uri_ref, Some(bytes.clone()));
    assert!(failure(&bad_uri.finish_checked()).starts_with("KC-9: "));

    let good = run_case(valid.clone(), Some(bytes));
    let manifest = good.finish_checked();
    assert!(!matches!(manifest.termination.reason, Termination::Failed { stage: Stage::Plan, .. }));
    assert!(manifest.inputs.contains(&valid));
}

#[test]
fn kc_14_an_arm_failure_is_failed_arm() {
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2).failing_at(support::FailAt::Arm), &probe)));
    let run = start_spec_run(&spec_one(), &profile_one(), assembly).expect("entry creates a Run");
    assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Arm, .. } }));
    let _ = run.finish_checked();
    assert!(probe.lines().iter().any(|line| line == "p:stop:Abort"));
    assert!(probe.lines().iter().any(|line| line == "p:cleanup"));
}

#[test]
fn kc_18_a_start_failure_is_failed_arm() {
    // #103: an admitted batch is not dispatched when an instance fails to start, and
    // `Running` is never recorded. The Provider's own `start` fails, so this is not an
    // earlier refusal.
    let mut spec = spec_one();
    add_schedule(
        &mut spec,
        "radio",
        0,
        serde_json::json!({
            "kind": "update_parameter",
            "target": serde_json::to_value(support::target("radio")).unwrap(),
            "key": "test.gain",
            "value": 3.0
        }),
    );
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly
        .providers
        .insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2).failing_at(support::FailAt::Start), &probe).declaring("radio/rx", 1, 1)));
    let run = start_spec_run(&spec, &profile_one(), assembly).expect("entry creates a Run");
    assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Arm, .. } }));
    assert_eq!(run.effective()[&Ident::parse("radio").unwrap()].get(&Key::parse("test.gain").unwrap()), None);
    assert!(probe.lines().iter().any(|line| line.starts_with("p:start:")), "{:?}", probe.lines());
    assert!(!probe.lines().iter().any(|line| line.starts_with("p:action:") || line.starts_with("p:update:")), "{:?}", probe.lines());
    let manifest = run.finish_checked();
    assert!(failure(&manifest).starts_with("KC-30: radio: "), "{}", failure(&manifest));
    assert!(!manifest.run.transitions.iter().any(|row| matches!(row.state, RunState::Running {})));
}

#[test]
fn ma_05a_a_module_keeps_its_handles_after_prepare() {
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(
        Ident::parse("radio").unwrap(),
        Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).emitting("test.custom", ezsdr_kernel::event::Severity::Info, 50).with_wakeups(&[50])),
    );
    let mut run = start_spec_run(&spec_one(), &profile_one(), assembly).expect("entry creates a Run");
    let root = run.now().domain();
    run.advance_to(ezsdr_kernel::time::TimePoint::new(root, 60)).expect("advances");
    let manifest = run.finish_checked();
    assert!(
        manifest.events.delivered.iter().any(|event| { event.kind == ezsdr_kernel::event::EventKind::parse("test.custom").unwrap() && event.time == ezsdr_kernel::time::TimePoint::new(root, 50) })
    );
    assert!(probe.lines().iter().any(|line| line == "p:step:50"));
    assert!(probe.lines().iter().any(|line| line == "p:now:50"));
}

#[test]
fn kc_16_a_spec_time_resolves_on_the_target_stream() {
    let mut spec = spec_one();
    let (bytes, reference) = input_ref();
    add_schedule(&mut spec, "radio", 10, tx_template("radio/tx", &reference, "send_asap_and_flag"));
    let probe = Probe::new();
    let rig = rig(ezsdr_kernel::module_api::Pacing::FreeRunning);
    let provider = SteppedProvider::new("p", TestProvider::new("dev", 2), &probe).declaring("dev/rx", 50, 1).declaring("dev/tx", 20, 1).registering_at_arm("dev/tx");
    let mut assembly = rig.assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(provider));
    assembly.inputs.insert(reference.hash.clone(), bytes);
    let mut run = start_spec_run(&spec, &profile_one(), assembly).expect("entry creates a Run");
    let _ = run.run_until_end(ezsdr_kernel::time::TimePoint::new(rig.root, 1_000));
    let manifest = run.finish_checked();
    assert!(probe.lines().iter().any(|line| line == "p:burst_at:10"));
    assert!(!matches!(manifest.termination.reason, Termination::Failed { stage: Stage::Arm, .. }));
}

#[test]
fn kc_16_an_ambiguous_spec_time_is_refused() {
    let mut spec = spec_one();
    add_schedule(&mut spec, "radio", 1, serde_json::json!({ "kind": "stop", "target": null }));
    let rig = rig(ezsdr_kernel::module_api::Pacing::FreeRunning);
    let mut assembly = rig.assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("dev", 2), &Probe::new()).declaring("dev/rx", 50, 1).declaring("dev/tx", 20, 1)));
    let run = start_spec_run(&spec, &profile_one(), assembly).expect("entry creates a Run");
    let manifest = run.finish_checked();
    assert!(matches!(manifest.termination.reason, Termination::Failed { stage: Stage::Arm, .. }));
    assert!(failure(&manifest).starts_with("KC-16: entry 0: ambiguous"));
}

#[test]
fn kc_16_off_root_negative_and_overflowing_times_are_refused() {
    let run_case = |offset: i64, ratio: u64, other_root: bool| {
        let rig = rig(ezsdr_kernel::module_api::Pacing::FreeRunning);
        let mut assembly = rig.assembly;
        let sample_root = if other_root {
            let other = rig.clocks.allocate_id().unwrap();
            rig.clocks
                .register(ezsdr_kernel::time::ClockDomain::root(
                    other,
                    ezsdr_kernel::time::Rational::new(1_000_000_000, 1).unwrap(),
                    ezsdr_kernel::time::EpochRef::Arbitrary { set_by: "test.other".to_owned() },
                ))
                .unwrap();
            other
        } else {
            rig.root
        };
        let root_ticks = if other_root { 1 } else { ratio };
        assembly.providers.insert(
            Ident::parse("radio").unwrap(),
            Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &Probe::new()).declaring_on("radio/rx", sample_root, root_ticks, 1).registering_at_arm("radio/rx")),
        );
        let mut spec = spec_one();
        add_schedule(&mut spec, "radio", offset, serde_json::json!({ "kind": "stop", "target": null }));
        let run = start_spec_run(&spec, &profile_one(), assembly).unwrap();
        let manifest = run.finish_checked();
        (manifest, rig.root)
    };
    let (off_root, _) = run_case(1, 1, true);
    assert!(failure(&off_root).contains("not on the primary root"));
    let (negative, _) = run_case(-1, 1, false);
    assert!(failure(&negative).contains("negative offset"));
    let (overflow, _) = run_case(i64::MAX, 2, false);
    assert!(failure(&overflow).contains("overflow"));
}

#[test]
fn kc_17_a_scheduled_stop_ends_the_run_at_its_instant() {
    let mut spec = spec_one();
    add_schedule(&mut spec, "radio", 100, serde_json::json!({ "kind": "stop", "target": null }));
    let rig = rig(ezsdr_kernel::module_api::Pacing::FreeRunning);
    let mut assembly = rig.assembly;
    assembly
        .providers
        .insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &Probe::new()).declaring("radio/rx", 10, 1).registering_at_arm("radio/rx")));
    let mut run = start_spec_run(&spec, &profile_one(), assembly).unwrap();
    let _ = run.run_until_end(ezsdr_kernel::time::TimePoint::new(rig.root, 2_000));
    let manifest = run.finish_checked();
    assert_eq!(manifest.termination.reason, Termination::Completed {});
    assert_eq!(manifest.termination.at, Some(ezsdr_kernel::time::TimePoint::new(rig.root, 1_000)));
}

#[test]
fn kc_19_a_reject_at_plan_burst_with_a_short_lead_is_refused() {
    let mut spec = spec_one();
    let (bytes, reference) = input_ref();
    add_schedule(&mut spec, "radio", 1_000_000, tx_template("radio/tx", &reference, "reject_at_plan"));
    let probe = Probe::new();
    let rig = rig(ezsdr_kernel::module_api::Pacing::FreeRunning);
    let mut assembly = rig.assembly;
    assembly.providers.insert(
        Ident::parse("radio").unwrap(),
        Box::new(
            SteppedProvider::new("p", TestProvider::new("radio", 2).with_min_command_lead(ezsdr_kernel::time::Duration::new(ClockDomainId::HOST_MONOTONIC, 5_000_000)), &probe)
                .declaring("radio/tx", 1, 1)
                .registering_at_arm("radio/tx"),
        ),
    );
    assembly.inputs.insert(reference.hash.clone(), bytes);
    let run = start_spec_run(&spec, &profile_one(), assembly).unwrap();
    let manifest = run.finish_checked();
    assert!(matches!(manifest.termination.reason, Termination::Failed { stage: Stage::Arm, .. }));
    assert!(failure(&manifest).starts_with("KC-19: SC-27"));
}

#[test]
fn kc_20_virtual_time_advances_only_through_next_wakeup() {
    let probe = Probe::new();
    let rig = rig(ezsdr_kernel::module_api::Pacing::FreeRunning);
    let mut assembly = rig.assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).with_wakeups(&[10, 20, 30])));
    let mut run = start_spec_run(&spec_one(), &profile_one(), assembly).unwrap();
    let _ = run.run_until_end(ezsdr_kernel::time::TimePoint::new(rig.root, 1_000));
    let _ = run.finish_checked();
    assert_eq!(probe.with_prefix("p:step:"), vec!["p:step:0", "p:step:10", "p:step:20", "p:step:30"]);
}

#[test]
fn kc_22_a_same_instant_wakeup_loop_is_step_livelock() {
    let probe = Probe::new();
    let rig = rig(ezsdr_kernel::module_api::Pacing::FreeRunning);
    let mut assembly = rig.assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).rescheduling_forever()));
    let mut run = start_spec_run(&spec_one(), &profile_one(), assembly).unwrap();
    let _ = run.run_until_end(ezsdr_kernel::time::TimePoint::new(rig.root, 10));
    let manifest = run.finish_checked();
    assert!(matches!(manifest.termination.reason, Termination::Stopped { cause: ezsdr_kernel::run::StopCause::Policy { .. } }));
    assert!(manifest.events.delivered.iter().any(|event| event.kind.as_str() == "ezsdr.STEP_LIVELOCK"
        && event.source == EventSource::Kernel {}
        && event.payload == serde_json::json!({ "rounds": ezsdr_kernel::module_api::STEP_ROUND_CAP })));
    assert!(manifest.run.transitions.iter().any(|t| matches!(t.state, RunState::Stopping { mode: ezsdr_kernel::run::CleanupMode::Abort })));
}

#[test]
fn kc_22_a_downgraded_livelock_still_ends_the_run() {
    let mut spec = spec_one();
    spec["policies"] = serde_json::json!({ "failure": { "ezsdr.STEP_LIVELOCK": "continue" } });
    let probe = Probe::new();
    let rig = rig(ezsdr_kernel::module_api::Pacing::FreeRunning);
    let mut assembly = rig.assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).rescheduling_forever()));
    let mut run = start_spec_run(&spec, &profile_one(), assembly).unwrap();
    let _ = run.run_until_end(ezsdr_kernel::time::TimePoint::new(rig.root, 10));
    let manifest = run.finish_checked();
    assert!(matches!(manifest.termination.reason, Termination::Failed { stage: Stage::Run, .. }));
    assert!(failure(&manifest).starts_with("KC-22"));
    assert!(manifest.run.transitions.iter().any(|t| matches!(t.state, RunState::Stopping { mode: ezsdr_kernel::run::CleanupMode::Abort })));
}

#[test]
fn kc_23_targets_are_rewritten_through_matched() {
    let (mut spec, _) = distinct_resource_docs(&["radio"]);
    spec["resources"]["radio"]["kind"] = serde_json::json!("test.line");
    let profile = serde_json::json!({
        "version": 1,
        "bindings": { "radio": { "module": { "id": "ezsdr.test.provider", "version": { "major": 1, "minor": 0, "patch": 0 } } } },
        "authority": "radio"
    });
    spec["schedule"] = serde_json::json!([{
        "at": { "clock": "radio", "offset_ticks": 0 },
        "action": { "kind": "update_parameter", "target": serde_json::to_value(support::target("radio/x")).unwrap(),
            "key": "test.gain", "value": 3.0, }
    }]);
    let probe = Probe::new();
    let rig = rig(ezsdr_kernel::module_api::Pacing::FreeRunning);
    let mut assembly = rig.assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("dev", 2), &probe).declaring("dev/0/rx", 1, 1)));
    let run = start_spec_run(&spec, &profile, assembly).unwrap();
    let manifest = run.finish_checked();
    assert!(
        probe.lines().iter().any(|line| line == "p:action:UpdateParameter:dev/0/x@0"),
        "probe={:?}, termination={:?}, sections={:?}",
        probe.lines(),
        manifest.termination.reason,
        manifest.sections
    );
}

#[test]
fn kc_39_orderly_cleanup_drains_the_tail() {
    let (spec, profile) = output_docs();
    let probe = Probe::new();
    let mut assembly = output_assembly(&probe, None, None);
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).with_tail(2)));
    let run = start_spec_run(&spec, &profile, assembly).unwrap();
    let _ = run.finish_checked();
    let lines = probe.lines();
    let block10 = lines.iter().position(|line| line == "rec:block:10").unwrap();
    let block20 = lines.iter().position(|line| line == "rec:block:20").unwrap();
    let stop = lines.iter().position(|line| line == "rec:stop:Orderly").unwrap();
    assert!(block10 < stop && block20 < stop);
}

#[test]
fn kc_39_abort_cleanup_does_not_drain() {
    let (spec, profile) = distinct_resource_docs(&["p", "q"]);
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("p").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("p", 2), &probe).with_wakeups(&[10])));
    assembly.providers.insert(Ident::parse("q").unwrap(), Box::new(SteppedProvider::new("q", TestProvider::new("q", 2), &probe).step_error_at(0)));
    let run = start_spec_run(&spec, &profile, assembly).unwrap();
    let _ = run.finish_checked();
    let lines = probe.lines();
    let stop = lines.iter().position(|line| line.ends_with(":stop:Abort")).unwrap();
    assert!(!lines.iter().skip(stop + 1).any(|line| line.starts_with("p:step:") || line.starts_with("q:step:")));
    assert!(lines.iter().any(|line| line == "p:stop:Abort"));
}

#[test]
fn kc_29_advance_to_refuses_an_unrelated_time() {
    let rig = rig(ezsdr_kernel::module_api::Pacing::FreeRunning);
    let mut assembly = rig.assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(TestProvider::new("radio", 2)));
    let mut run = start_spec_run(&spec_one(), &profile_one(), assembly).unwrap();
    let other = rig.clocks.allocate_id().unwrap();
    rig.clocks
        .register(ezsdr_kernel::time::ClockDomain::root(
            other,
            ezsdr_kernel::time::Rational::new(1_000_000_000, 1).unwrap(),
            ezsdr_kernel::time::EpochRef::Arbitrary { set_by: "test.other".to_owned() },
        ))
        .unwrap();
    assert!(matches!(run.advance_to(ezsdr_kernel::time::TimePoint::new(other, 5)), Err(ezsdr_kernel::coordinator::RunHandleError::NotOnPrimaryRoot { .. })));
    let _ = run.finish_checked();
}

#[test]
fn kc_01_a_parse_failure_is_no_run() {
    let mut doc = spec_one();
    doc["version"] = serde_json::json!(2);
    let result = start_spec_run(&doc, &profile_one(), rig(Pacing::FreeRunning).assembly);
    assert!(matches!(result, Err(SpecError::UnsupportedVersion { found: 2, .. })));
}

#[test]
fn kc_01_a_validate_failure_still_writes_a_manifest() {
    let mut spec = spec_one();
    spec["resources"]["other"] = serde_json::json!({ "kind": "test.device" });
    let run = start_spec_run(&spec, &profile_one(), with_provider(rig(Pacing::FreeRunning).assembly, "radio", "radio")).expect("entry creates a Run");
    assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Validate, .. } }));
    let manifest = run.finish_checked();
    assert!(manifest.hash.is_some());
    assert!(manifest.plan.is_none());
    assert_eq!(
        manifest.run.transitions.iter().map(|row| row.state.clone()).collect::<Vec<_>>(),
        vec![RunState::Created {}, RunState::Stopping { mode: ezsdr_kernel::run::CleanupMode::Abort }, RunState::CleanedUp { termination: manifest.termination.reason.clone() },]
    );
    assert!(matches!(manifest.termination.reason, Termination::Failed { stage: Stage::Validate, .. }));
}

#[test]
fn kc_01_a_validate_failure_records_its_reason() {
    let mut spec = spec_one();
    spec["resources"]["other"] = serde_json::json!({ "kind": "test.device" });
    let run = start_spec_run(&spec, &profile_one(), with_provider(rig(Pacing::FreeRunning).assembly, "radio", "radio")).expect("entry creates a Run");
    let manifest = run.finish_checked();
    assert!(!failure(&manifest).is_empty());
    // No plan was built, so the class is unknown and absent, not a made-up one (KC-45).
    assert_eq!(manifest.run.execution_class, None);
    assert!(!manifest.run.deterministic);
    assert_eq!(manifest.run.transitions[0].at, None);
    assert!(manifest.run.transitions.iter().skip(1).all(|r| r.at.is_some()));
    assert!(manifest.run.transitions.iter().all(|r| r.host_utc_nanos > 0));
}

#[test]
fn kc_02_a_wall_paced_authority_is_refused() {
    let mut profile = profile_one();
    profile["environment"] = serde_json::json!({
        "ezsdr.time": { "class": "realtime_emulation" }
    });
    let run = start_spec_run(&spec_one(), &profile, with_provider(rig(Pacing::WallPaced).assembly, "radio", "radio")).expect("entry creates a Run");
    assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Plan, .. } }));
    let manifest = run.finish_checked();
    assert!(failure(&manifest).starts_with("KC-2: RealtimeEmulation"));
    assert_eq!(manifest.run.execution_class, Some(ezsdr_kernel::module_api::ExecutionClass::RealtimeEmulation));
    assert!(manifest.plan.is_some());
}

fn pair_docs() -> (serde_json::Value, serde_json::Value) {
    let resource = serde_json::json!({
        "kind": "test.line",
        "requires": { "test.count": { "kind": "eq", "value": 2 } }
    });
    let spec = serde_json::json!({
        "version": 1,
        "requirements": { "vocabularies": [{ "id": "test", "major": 1 }] },
        "resources": { "a": resource, "b": resource }
    });
    let binding = serde_json::json!({
        "module": { "id": "ezsdr.test.provider", "version": { "major": 1, "minor": 0, "patch": 0 } }
    });
    let profile = serde_json::json!({
        "version": 1,
        "bindings": { "a": binding, "b": binding },
        "authority": "a"
    });
    (spec, profile)
}

#[test]
fn kc_04_two_objects_for_one_description_are_refused() {
    let (spec, profile) = pair_docs();
    let mut assembly = rig(Pacing::FreeRunning).assembly;
    assembly = with_provider(assembly, "a", "dev");
    assembly = with_provider(assembly, "b", "dev");
    let run = start_spec_run(&spec, &profile, assembly).expect("entry creates a Run");
    assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Validate, .. } }));
    assert!(failure(&run.finish_checked()).starts_with("KC-4: "));
}

#[test]
fn kc_04_one_object_serves_both_names() {
    let (spec, profile) = pair_docs();
    let run = start_spec_run(&spec, &profile, with_provider(rig(Pacing::FreeRunning).assembly, "a", "dev")).expect("entry creates a Run");
    assert!(matches!(run.state(), RunState::Running {}));
    assert!(matches!(run.finish_checked().termination.reason, Termination::Stopped { cause: ezsdr_kernel::run::StopCause::Client {} }));
}

#[test]
fn kc_04_an_object_under_no_resource_is_refused() {
    let mut assembly = with_provider(rig(Pacing::FreeRunning).assembly, "radio", "radio");
    assembly = with_provider(assembly, "nobody", "nobody");
    let run = start_spec_run(&spec_one(), &profile_one(), assembly).expect("entry creates a Run");
    assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Validate, .. } }));
    assert!(failure(&run.finish_checked()).contains("nobody"));
}

#[test]
fn kc_45_the_documents_are_recorded_as_parsed() {
    let spec = spec_one();
    let profile = profile_one();
    let run = start_spec_run(&spec, &profile, with_provider(rig(Pacing::FreeRunning).assembly, "radio", "radio")).expect("entry creates a Run");
    let manifest = run.finish_checked();
    let parsed_spec = ExperimentSpec::from_json(&spec).unwrap();
    let parsed_profile = BindingProfile::from_json(&profile).unwrap();
    assert_eq!(manifest.spec.body, serde_json::to_value(&parsed_spec).unwrap());
    assert_eq!(manifest.binding.body, serde_json::to_value(&parsed_profile).unwrap());
    assert_eq!(manifest.spec.hash, ContentHash::of_value(&manifest.spec.body).unwrap());
    assert_eq!(manifest.binding.hash, ContentHash::of_value(&manifest.binding.body).unwrap());
    assert_eq!(manifest.run.kind, RunKind::Spec);
}

#[test]
fn rs_45_hash_equal_for_equal_inputs() {
    // One document spelled twice — its defaults omitted, then written out in another
    // key order with whitespace — is one Spec and one profile, with one hash each.
    let spelled_spec = serde_json::from_str(
        r#"{ "extensions": {}, "graph": { "components": {}, "links": [] },
             "schedule": [], "outputs": [], "inputs": [], "version": 1,
             "policies": { "failure": {}, "coercion": {} },
             "requirements": { "vocabularies": [{ "id": "test", "major": 1 }] },
             "resources": { "radio": { "kind": "test.device",
                 "requires": { "test.count": { "kind": "eq", "value": 2 } } } } }"#,
    )
    .unwrap();
    let spelled_profile = serde_json::from_str(
        r#"{ "environment": {}, "version": 1, "authority": "radio",
             "bindings": { "radio": { "selector": {},
                 "module": { "id": "ezsdr.test.provider",
                             "version": { "major": 1, "minor": 0, "patch": 0 } } } } }"#,
    )
    .unwrap();
    let manifest = |spec: &serde_json::Value, profile: &serde_json::Value| {
        start_spec_run(spec, profile, with_provider(rig(Pacing::FreeRunning).assembly, "radio", "radio")).expect("entry creates a Run").finish_checked()
    };
    let terse = manifest(&spec_one(), &profile_one());
    let spelled = manifest(&spelled_spec, &spelled_profile);
    assert_ne!(spelled_spec, spec_one(), "the fixture spells what the other omits");
    assert_ne!(spelled_profile, profile_one(), "the fixture spells what the other omits");
    assert_eq!((&terse.spec.hash, &terse.spec.body), (&spelled.spec.hash, &spelled.spec.body));
    assert_eq!((&terse.binding.hash, &terse.binding.body), (&spelled.binding.hash, &spelled.binding.body));
    // A Session records its profile the same way (`connect`).
    for profile in [profile_one(), spelled_profile] {
        let session = connect(&profile, &Default::default(), with_provider(rig(Pacing::FreeRunning).assembly, "radio", "radio"), Lease::attached()).expect("valid Session entry").finish_checked();
        assert_eq!((&session.binding.hash, &session.binding.body), (&terse.binding.hash, &terse.binding.body));
    }
}

#[test]
fn rs_38_spec_source_is_recorded_outside_the_spec_hash() {
    // The caller's source hash is recorded as given and changes neither the Spec's
    // hash nor its body: two builders that produce one Spec give one Spec hash.
    let source = ContentHash::of_value(&serde_json::json!("builder.py")).unwrap();
    let manifest = |spec_source: Option<ContentHash>| {
        let mut assembly = with_provider(rig(Pacing::FreeRunning).assembly, "radio", "radio");
        assembly.spec_source = spec_source;
        start_spec_run(&spec_one(), &profile_one(), assembly).expect("entry creates a Run").finish_checked()
    };
    let with = manifest(Some(source.clone()));
    let without = manifest(None);
    assert_eq!(with.spec.source, Some(source.clone()));
    assert_eq!(without.spec.source, None);
    assert_eq!((&with.spec.hash, &with.spec.body), (&without.spec.hash, &without.spec.body));
    assert_eq!(with.spec.hash, ContentHash::of_value(&with.spec.body).unwrap());
    // A Session records the caller's value too, as opaque provenance.
    let mut assembly = with_provider(rig(Pacing::FreeRunning).assembly, "radio", "radio");
    assembly.spec_source = Some(source.clone());
    let mut session = connect(&profile_one(), &Default::default(), assembly, Lease::attached()).unwrap();
    session.disconnect();
    assert_eq!(session.finish_checked().spec.source, Some(source));
}

#[test]
fn kc_17_scheduled_updates_are_admitted_cumulatively() {
    let probe = Probe::new();
    let fixture = rig(ezsdr_kernel::module_api::Pacing::FreeRunning);
    let mut spec = spec_one();
    spec["resources"]["radio"]["requires"]["test.grid"] = serde_json::json!({ "kind": "eq", "value": 20.0 });
    spec["resources"]["radio"]["requires"]["test.flag"] = serde_json::json!({ "kind": "eq", "value": false });
    let radio = serde_json::to_value(support::target("radio")).unwrap();
    let first = serde_json::json!({
        "at": { "clock": "radio", "offset_ticks": 10 }, "action": {
            "kind": "update_parameter", "target": radio, "key": "test.grid", "value": 40.0,
        }
    });
    spec["schedule"] = serde_json::json!([
        first,
        { "at": { "clock": "radio", "offset_ticks": 20 }, "action": {
            "kind": "update_parameter", "target": radio, "key": "test.flag", "value": true,
        } }
    ]);
    let mut profile = profile_one();
    profile["environment"] = serde_json::json!({
        "test.limits": { "max_grid": 30, "gate": "test.flag" }
    });
    let mut assembly = fixture.assembly;
    assembly.registry = run_registry_classed();
    assembly.checks = run_checks(true);
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).declaring("radio/rx", 1, 1)));
    let run = start_spec_run(&spec, &profile, assembly).expect("entry creates a Run");
    assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Arm, .. } }));
    assert_eq!(
        run.effective()[&Ident::parse("radio").unwrap()][&Key::parse("test.grid").unwrap()],
        Value::num(20.0).unwrap(),
        "the requires value the prepare report seeds, not the scheduled 40.0 which was never published"
    );
    let reason = failure(&run.finish_checked()).to_owned();
    assert!(reason.starts_with("KC-17: entry 1"));
    assert!(reason.contains("test.limits"));

    let mut only_first = spec_one();
    only_first["resources"]["radio"]["requires"]["test.grid"] = serde_json::json!({ "kind": "eq", "value": 20.0 });
    only_first["resources"]["radio"]["requires"]["test.flag"] = serde_json::json!({ "kind": "eq", "value": false });
    only_first["schedule"] = serde_json::json!([first.clone()]);
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.registry = run_registry_classed();
    assembly.checks = run_checks(true);
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).declaring("radio/rx", 1, 1)));
    let run = start_spec_run(&only_first, &profile, assembly).expect("entry creates a Run");
    assert!(matches!(run.state(), RunState::Running {}));
    let _ = run.finish_checked();
}

#[test]
fn kc_24_a_burst_needs_a_transmit_sample_clock() {
    let probe = Probe::new();
    let mut run = session_with_provider(Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe)));
    let entry = run
        .submit(SessionAction::Vocabulary { ns: ns("test"), verb: Ident::parse("start_repeat").unwrap(), target: support::target("radio/tx"), at: None, params: BTreeMap::new() }, Some(&[0u8; 80]))
        .expect("well-formed Action is logged");
    assert!(matches!(entry.outcome, Outcome::Rejected { ref violations }
        if violations.iter().any(|v| v.reason.starts_with("SC-23: ") && v.reason.ends_with("has no running transmit SampleClock"))));
    let _ = run.finish_checked();
}

#[test]
fn kc_24_a_module_reject_at_plan_burst_is_refused() {
    let (spec, profile) = executor_docs();
    let probe = Probe::new();
    let (_, waveform) = input_ref();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe)));
    assembly.executors.insert(
        Ident::parse("exec").unwrap(),
        Box::new(ProbeExecutor::new("x", &probe).submitting(Action::TxBurst {
            target: support::target("radio/tx"),
            waveform,
            repeat: false,
            at: ezsdr_kernel::time::AbsoluteDeadline::new(TimePoint::new(rig(ezsdr_kernel::module_api::Pacing::FreeRunning).root, 1)),
            requested_at: None,
            late_policy: ezsdr_kernel::stream::LatePolicy::RejectAtPlan,
            metadata: BTreeMap::new(),
        })),
    );
    let run = start_spec_run(&spec, &profile, assembly).expect("entry creates a Run");
    let _ = run.finish_checked();
    assert!(probe.lines().iter().any(|line| line.starts_with("x:submit:err:ezsdr.late_policy:KC-19")));
}

#[test]
fn kc_24_a_module_action_during_cleanup_is_refused() {
    let (spec, profile) = executor_docs();
    let probe = Probe::new();
    let primary = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).root;
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).with_tail(1)));
    assembly.executors.insert(
        Ident::parse("exec").unwrap(),
        Box::new(
            ProbeExecutor::new("x", &probe)
                .submitting_at(10, Action::SetTimer { target: support::target("radio"), at: ezsdr_kernel::time::AbsoluteDeadline::new(TimePoint::new(primary, 20)), token: 1 }),
        ),
    );
    let run = start_spec_run(&spec, &profile, assembly).expect("entry creates a Run");
    let _ = run.finish_checked();
    assert!(probe.lines().iter().any(|line| line.starts_with("x:submit:err:ezsdr.dispatch:RS-6")));
}

#[test]
fn kc_24_a_module_stop_without_target_is_refused() {
    let (spec, profile) = executor_docs();
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe)));
    assembly.executors.insert(Ident::parse("exec").unwrap(), Box::new(ProbeExecutor::new("x", &probe).submitting(Action::Stop { target: None })));
    let run = start_spec_run(&spec, &profile, assembly).expect("entry creates a Run");
    assert!(matches!(run.state(), RunState::Running {}));
    let _ = run.finish_checked();
    assert!(probe.lines().iter().any(|line| line == "x:submit:err:ezsdr.target:KC-24: a Module ends a Run only with Abort"));
}

#[test]
fn kc_24_a_burst_to_a_non_provider_target_is_refused() {
    let (_spec, mut profile) = output_docs();
    profile["bindings"]["rec"]["feed"] = serde_json::json!({
        "port": { "component": "radio", "port": "rx" },
        "policy": "drop_oldest", "capacity": 4
    });
    let probe = Probe::new();
    let assembly = output_assembly(&probe, None, None);
    let mut run = connect(&profile, &Default::default(), assembly, Lease::attached()).expect("Session profile is valid");
    if !matches!(run.state(), RunState::Running {}) {
        let manifest = run.finish_checked();
        panic!("Session failed during connect: {:?}", failure(&manifest));
    }
    let entry = run
        .submit(
            SessionAction::Vocabulary {
                ns: ns("test"),
                verb: Ident::parse("start_repeat").unwrap(),
                target: Target::Output { output: Ident::parse("rec").unwrap() },
                at: None,
                params: BTreeMap::new(),
            },
            Some(&[0u8; 80]),
        )
        .expect("well-formed Action is logged");
    assert!(
        matches!(entry.outcome, Outcome::Rejected { ref violations }
        if violations.iter().any(|v| v.reason.starts_with("SC-23: output rec is not a Provider stream"))),
        "{entry:?}"
    );
    let _ = run.finish_checked();
}

#[test]
fn kc_24_a_burst_time_on_an_unrelated_root_is_refused() {
    let probe = Probe::new();
    let mut rig = rig(ezsdr_kernel::module_api::Pacing::FreeRunning);
    let other = rig.clocks.allocate_id().unwrap();
    rig.clocks.register(ClockDomain::root(other, Rational::new(1_000_000_000, 1).unwrap(), EpochRef::Arbitrary { set_by: "other".to_owned() })).unwrap();
    rig.assembly
        .providers
        .insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).declaring("radio/tx", 1, 1).registering_at_arm("radio/tx")));
    let mut run = connect(&profile_one(), &Default::default(), rig.assembly, Lease::attached()).unwrap();
    let entry = run
        .submit(
            SessionAction::Vocabulary { ns: ns("test"), verb: Ident::parse("start_repeat").unwrap(), target: support::target("radio/tx"), at: Some(TimePoint::new(other, 5)), params: BTreeMap::new() },
            Some(&[0u8; 80]),
        )
        .unwrap();
    assert!(matches!(entry.outcome, Outcome::Rejected { ref violations }
        if violations.iter().any(|v| v.reason.starts_with("SC-23b"))));
    let _ = run.finish_checked();
}

#[test]
fn kc_24_a_module_abort_ends_the_run() {
    let (spec, profile) = executor_docs();
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe)));
    assembly.executors.insert(
        Ident::parse("exec").unwrap(),
        Box::new(
            ProbeExecutor::new("x", &probe)
                .submitting(Action::Abort { cause: StopCause::Abort { message: "test".to_owned() } })
                // Admitted before the coordinator freezes dispatch, so it takes an id too.
                .then_submitting(Action::UpdateParameter {
                    target: support::target("radio"),
                    key: Key::parse("test.gain").unwrap(),
                    value: Value::num(3.0).unwrap(),
                    class: ezsdr_kernel::module_api::UpdateClass::HardwareTimed,
                    at: None,
                }),
        ),
    );
    let run = start_spec_run(&spec, &profile, assembly).expect("entry creates a Run");
    assert!(matches!(run.state(), RunState::CleanedUp {
        termination: Termination::Stopped { cause: StopCause::Abort { message: cause } }
    } if cause == "test"));
    let termination = run.finish_checked().termination;
    assert!(termination.cleanup_failures.is_empty());
    // KC-24: the Abort takes an id of its own from the Run's counter, which the
    // Action dispatched after it does not reuse.
    let returned: Vec<u64> = probe.lines().into_iter().filter_map(|line| line.strip_prefix("x:submit:ok:").map(|id| id.parse().unwrap())).collect();
    assert!(matches!(returned[..], [abort, update] if abort > 0 && abort != update), "{:?}", probe.lines());
}

#[test]
fn rs_16_an_executor_action_the_freeze_discards_is_undelivered() {
    // F24's window for an Executor: a Command to a component, queued for an Executor
    // that never takes it, then an Abort; RS-6 step 1 discards the Command from the
    // Executor's queue and names it in `undelivered` (RS-16).
    let (spec, profile) = executor_docs();
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe)));
    assembly.executors.insert(
        Ident::parse("exec").unwrap(),
        Box::new(
            ProbeExecutor::new("x", &probe)
                .deaf()
                .submitting(Action::Command { target: Target::Component { component: Ident::parse("c1").unwrap() }, verb: Ident::parse("probe").unwrap(), params: BTreeMap::new(), at: None })
                .then_submitting(Action::Abort { cause: StopCause::Abort { message: "test".to_owned() } }),
        ),
    );
    let run = start_spec_run(&spec, &profile, assembly).expect("entry creates a Run");
    let termination = run.finish_checked().termination;
    let returned: Vec<u64> = probe.lines().into_iter().filter_map(|line| line.strip_prefix("x:submit:ok:").map(|id| id.parse().unwrap())).collect();
    assert_eq!(returned.len(), 2, "{:?}", probe.lines());
    assert_eq!(termination.undelivered.iter().map(|id| id.0).collect::<Vec<_>>(), returned[..1]);
    assert!(!probe.lines().iter().any(|line| line.starts_with("x:action:")));
}

#[test]
fn kc_23_an_unknown_target_is_refused() {
    let mut run = session_with_provider(Box::new(TestProvider::new("radio", 2)));
    let entry = run.submit(SessionAction::SetParameter { target: support::target("nothing"), key: Key::parse("test.gain").unwrap(), value: Value::num(1.0).unwrap() }, None).unwrap();
    assert!(matches!(entry.outcome, Outcome::Rejected { ref violations }
        if violations.iter().any(|v| v.check == ns("ezsdr.target") && v.reason.starts_with("KC-23: "))));
    let _ = run.finish_checked();
}

#[test]
fn kc_24_the_target_is_resolved_before_the_update_class() {
    // KC-24 (review P1-1): a class-pending request keeps its Pending form through
    // steps 1–4 and is completed at step 5, so a step-3 refusal stands even when the
    // request also has a declaration gap. Two Session inputs:
    // (a) an undeclared `{component: nothing}` target: refused as `ezsdr.target`, and
    //     the declaration the target cannot give is not even looked for — an
    //     implementation that completed the request first would refuse
    //     `ezsdr.update_class` instead;
    // (b) the resolvable `radio` target with `test.count`, a declared key with no
    //     `update_class`: refused as step 5's `ezsdr.update_class` — completing it
    //     before the earlier steps is only observable where `declared_class` gives
    //     nothing, which is this gap.
    let mut run = session_with_provider(Box::new(TestProvider::new("radio", 2)));
    let entry = run
        .submit(SessionAction::SetParameter { target: Target::Component { component: Ident::parse("nothing").unwrap() }, key: Key::parse("test.count").unwrap(), value: Value::from(1) }, None)
        .unwrap();
    assert!(
        matches!(entry.outcome, Outcome::Rejected { ref violations }
            if violations.iter().any(|v| v.check == ns("ezsdr.target"))
                && violations.iter().all(|v| v.check != ns("ezsdr.update_class"))),
        "an undeclared target is refused by step 3: {entry:?}"
    );
    let gap = run.submit(SessionAction::SetParameter { target: support::target("radio"), key: Key::parse("test.count").unwrap(), value: Value::from(1) }, None).unwrap();
    assert!(
        matches!(gap.outcome, Outcome::Rejected { ref violations }
            if violations.iter().any(|v| v.check == ns("ezsdr.update_class")
                && v.reason.contains("declares no update class"))),
        "the declaration gap is refused at step 5: {gap:?}"
    );
    let _ = run.finish_checked();
}

#[test]
fn kc_21_an_action_is_seen_at_its_admission_instant() {
    let probe = Probe::new();
    let mut run = session_with_provider(Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe)));
    let root = run.now().domain();
    run.advance_to(TimePoint::new(root, 500)).unwrap();
    let entry = run.submit(SessionAction::SetParameter { target: support::target("radio"), key: Key::parse("test.gain").unwrap(), value: Value::num(1.0).unwrap() }, None).unwrap();
    assert!(matches!(entry.outcome, Outcome::Admitted { .. }));
    assert!(probe.lines().iter().any(|line| line == "p:action:UpdateParameter:radio@500"));
    let _ = run.finish_checked();
}

#[test]
fn kc_25_an_admitted_update_changes_the_configuration() {
    let probe = Probe::new();
    let mut run = session_with_provider(Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe)));
    let entry = run.submit(SessionAction::SetParameter { target: support::target("radio"), key: Key::parse("test.gain").unwrap(), value: Value::num(3.0).unwrap() }, None).unwrap();
    // Item 3: one `SetParameter` compiles to exactly one request, so the entry
    // dispatches once; a compile that produced two requests would make this two.
    assert!(matches!(entry.outcome, Outcome::Admitted { dispatched, .. } if dispatched.len() == 1));
    assert_eq!(run.effective()[&Ident::parse("radio").unwrap()][&Key::parse("test.gain").unwrap()], Value::num(3.0).unwrap());
    assert!(probe.lines().iter().any(|line| line == "p:update:test.gain=3.0"));
    let _ = run.finish_checked();
}

#[test]
fn rs_17_a_session_rate_change_is_coerced_by_its_provider() {
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.registry = run_registry_classed();
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2).with_grid(20.0), &probe)));
    let mut run = connect(&profile_one(), &Default::default(), assembly, Lease::attached()).unwrap();
    let entry = run.submit(SessionAction::SetParameter { target: support::target("radio"), key: Key::parse("test.grid").unwrap(), value: Value::num(19.5).unwrap() }, None).unwrap();
    assert!(matches!(entry.outcome, Outcome::Admitted { ref coercions, .. }
        if coercions.iter().any(|c| c.requested == Value::num(19.5).unwrap() && c.applied == Value::num(20.0).unwrap())));
    assert!(probe.lines().iter().any(|line| line == "p:update:test.grid=20.0"));
    let _ = run.finish_checked();
}

#[test]
fn rs_17_a_kind_only_coercion_is_recorded() {
    // KC-24 steps 10–11 (review P2-2): `Scalar`'s `PartialEq` is numeric, so
    // `Int(3) == Num(3.0)`. The double's grid makes `coerce` apply `Num(3.0)` for the
    // requested `Int(3)` — the same number in another kind — and the comparison must
    // notice it by kind. With equality alone the coercion is invisible: nothing is
    // recorded and the configuration keeps the requested `Int(3)`.
    let mut assembly = rig(Pacing::FreeRunning).assembly;
    assembly.registry = run_registry_classed();
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(TestProvider::new("radio", 2).with_grid(0.001)));
    let mut run = connect(&profile_one(), &Default::default(), assembly, Lease::attached()).unwrap();
    let entry = run.submit(SessionAction::SetParameter { target: support::target("radio"), key: Key::parse("test.grid").unwrap(), value: Value::from(3) }, None).unwrap();
    assert!(
        matches!(entry.outcome, Outcome::Admitted { ref coercions, .. }
        if coercions.iter().any(|c| c.key == Key::parse("test.grid").unwrap()
            && c.requested == Value::from(3)
            && c.applied == Value::num(3.0).unwrap())),
        "the kind-only coercion is recorded: {entry:?}"
    );
    assert_eq!(run.effective()[&Ident::parse("radio").unwrap()][&Key::parse("test.grid").unwrap()], Value::num(3.0).unwrap(), "the coerced value is what the configuration holds");
    let _ = run.finish_checked();
}

#[test]
fn rs_17_a_scheduled_rate_change_under_reject_is_refused() {
    let mut spec = spec_one();
    let radio = serde_json::to_value(support::target("radio")).unwrap();
    spec["schedule"] = serde_json::json!([{
        "at": { "clock": "radio", "offset_ticks": 0 }, "action": {
            "kind": "update_parameter", "target": radio, "key": "test.grid", "value": 19.5,
        }
    }]);
    let probe = Probe::new();
    let rig = rig(ezsdr_kernel::module_api::Pacing::FreeRunning);
    let mut assembly = rig.assembly;
    assembly.registry = run_registry_classed();
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2).with_grid(20.0), &probe).declaring("radio/rx", 1, 1)));
    let run = start_spec_run(&spec, &profile_one(), assembly).unwrap();
    assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Arm, .. } }));
    assert!(failure(&run.finish_checked()).contains("SB-46"));
}

#[test]
fn rs_17_a_session_change_beyond_a_joint_limit_is_refused() {
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.registry = run_registry_classed();
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(TestProvider::new("radio", 2).with_joint_limit(50.0).with_effective("test.count", Value::from(2))));
    let mut run = connect(&profile_one(), &Default::default(), assembly, Lease::attached()).unwrap();
    let entry = run.submit(SessionAction::SetParameter { target: support::target("radio"), key: Key::parse("test.grid").unwrap(), value: Value::num(40.0).unwrap() }, None).unwrap();
    assert!(matches!(entry.outcome, Outcome::Rejected { ref violations }
        if violations.iter().any(|v| v.check == ns("ezsdr.coercion") && v.key == Some(Key::parse("test.grid").unwrap()))));
    let _ = run.finish_checked();
}

#[test]
fn kc_26_a_provider_that_applies_nothing_is_refused() {
    let mut run = session_with_provider(Box::new(TestProvider::new("radio", 2).omitting_from_applied("test.gain")));
    let entry = run.submit(SessionAction::SetParameter { target: support::target("radio"), key: Key::parse("test.gain").unwrap(), value: Value::num(1.0).unwrap() }, None).unwrap();
    assert!(matches!(entry.outcome, Outcome::Rejected { ref violations }
        if violations.iter().any(|v| v.check == ns("ezsdr.coercion")
            && v.key == Some(Key::parse("test.gain").unwrap())
            && v.requested == Some(Value::num(1.0).unwrap())
            && v.reason.contains("applied no value for test.gain"))));
    let _ = run.finish_checked();
}

#[test]
fn sb_05_a_provider_parameter_update_is_a_scalar() {
    // A Provider parameter's change is offered to `coerce` as an `Eq` constraint,
    // whose value is a scalar (SB-5): a list is refused before `coerce`, never
    // dropped or coerced into one. The key is `test.list`, whose `KeyDecl` is a
    // `List`, so KC-24's step 6 passes it and step 8 is the refusal the test names:
    // for a key whose declaration is scalar, step 6 refuses it first.
    let mut run = session_with_provider(Box::new(TestProvider::new("radio", 2)));
    let entry = run.submit(SessionAction::SetParameter { target: support::target("radio"), key: Key::parse("test.list").unwrap(), value: Value::List(vec![Scalar::from(1)]) }, None).unwrap();
    assert!(
        matches!(entry.outcome, Outcome::Rejected { ref violations }
        if violations.iter().any(|v| v.check == ns("ezsdr.value")
            && v.key == Some(Key::parse("test.list").unwrap())
            && v.reason.contains("SB-5"))),
        "{:?}",
        entry.outcome
    );
    let _ = run.finish_checked();
}

#[test]
fn kc_28_a_session_value_with_a_non_ascii_key_is_logged_and_the_manifest_reads_back() {
    let mut run = session_with_provider(Box::new(TestProvider::new("radio", 2)));
    let bad_value = Value::Map(BTreeMap::from([("nonascii-\u{e9}".to_owned(), Scalar::from(true))]));
    let entry = run.submit(SessionAction::SetParameter { target: support::target("radio"), key: Key::parse("test.gain").unwrap(), value: bad_value.clone() }, None).unwrap();
    assert_eq!(entry.seq, 0);
    assert!(matches!(entry.outcome, Outcome::Rejected { .. }), "{:?}", entry.outcome);
    assert!(!format!("{:?}", entry.outcome).contains("ASCII"), "the value-kind rule is the refusal: {:?}", entry.outcome);
    let entry = run.submit(SessionAction::SetParameter { target: support::target("radio"), key: Key::parse("test.gain").unwrap(), value: Value::num(1.0).unwrap() }, None).unwrap();
    assert_eq!(entry.seq, 1);
    let manifest = run.finish_checked();
    assert_eq!(manifest.action_log.len(), 2);
    let SessionAction::SetParameter { value, .. } = &manifest.action_log[0].action else { panic!("the logged action is the SetParameter") };
    assert_eq!(value, &bad_value);
    assert!(manifest.hash.is_some());
    assert!(manifest.inputs.is_empty());
}

#[test]
fn kc_28_a_waveform_is_an_input_before_admission() {
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).declaring("radio/tx", 1, 1).registering_at_arm("radio/tx")));
    let mut run = connect(&profile_one(), &Default::default(), assembly, Lease::attached()).unwrap();
    let bytes = vec![0u8; 800];
    let entry = run
        .submit(SessionAction::Vocabulary { ns: ns("test"), verb: Ident::parse("start_repeat").unwrap(), target: support::target("radio/tx"), at: None, params: BTreeMap::new() }, Some(&bytes))
        .unwrap();
    assert!(matches!(entry.outcome, Outcome::Admitted { .. }));
    let manifest = run.finish_checked();
    assert_eq!(manifest.inputs.len(), 1);
    assert_eq!(manifest.inputs[0].size_bytes, 800);
    assert!(manifest.inputs[0].uri.starts_with("mem:sha256:"));
    assert!(probe.lines().iter().any(|line| line.starts_with("p:action:TxBurst:radio/tx@")));
}

#[test]
fn kc_28_an_untimed_burst_is_admitted_at_now_plus_lead() {
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(
        Ident::parse("radio").unwrap(),
        Box::new(
            SteppedProvider::new("p", TestProvider::new("radio", 2).with_min_command_lead(Duration::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 2_000_000)), &probe)
                .declaring("radio/tx", 1, 1)
                .registering_at_arm("radio/tx"),
        ),
    );
    let mut run = connect(&profile_one(), &Default::default(), assembly, Lease::attached()).unwrap();
    let entry = run
        .submit(SessionAction::Vocabulary { ns: ns("test"), verb: Ident::parse("start_repeat").unwrap(), target: support::target("radio/tx"), at: None, params: BTreeMap::new() }, Some(&[0u8; 80]))
        .unwrap();
    assert!(matches!(entry.outcome, Outcome::Admitted { ref coercions, .. }
        if coercions.iter().any(|c| c.key == Key::parse("ezsdr.action.at").unwrap()
            && c.applied == Value::Scalar(Scalar::Int(2_000_000)))));
    assert!(probe.lines().iter().any(|line| line == "p:burst_at:2000000"));
    let _ = run.finish_checked();
}

#[test]
fn rs_19_an_untimed_session_action_is_dispatched_untimed() {
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).declaring("radio/tx", 1, 1).registering_at_arm("radio/tx")));
    let mut run = connect(&profile_one(), &Default::default(), assembly, Lease::attached()).unwrap();
    let entry = run
        .submit(SessionAction::Vocabulary { ns: ns("test"), verb: Ident::parse("start_repeat").unwrap(), target: support::target("radio/tx"), at: None, params: BTreeMap::new() }, Some(&[0u8; 80]))
        .unwrap();
    assert!(matches!(entry.outcome, Outcome::Admitted { .. }));
    let _ = run.finish_checked();
    assert!(probe.lines().iter().any(|line| line == "p:burst_untimed:true"));
}

#[test]
fn rs_19_an_untimed_session_action_has_no_requested_instant() {
    // RM-15 (sequence 8, M2): the Kernel chose an untimed burst's instant, so an instant off the transmit grid is not a request.
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(
        Ident::parse("radio").unwrap(),
        Box::new(
            SteppedProvider::new("p", TestProvider::new("radio", 2).with_min_command_lead(Duration::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 2_000_001)), &probe)
                .declaring("radio/tx", 1_000, 1)
                .registering_at_arm("radio/tx"),
        ),
    );
    let mut run = connect(&profile_one(), &Default::default(), assembly, Lease::attached()).unwrap();
    let entry = run
        .submit(SessionAction::Vocabulary { ns: ns("test"), verb: Ident::parse("start_repeat").unwrap(), target: support::target("radio/tx"), at: None, params: BTreeMap::new() }, Some(&[0u8; 80]))
        .unwrap();
    assert!(matches!(entry.outcome, Outcome::Admitted { .. }));
    let _ = run.finish_checked();
    assert!(probe.lines().iter().any(|line| line == "p:burst_untimed:true"), "{:?}", probe.lines());
    assert!(probe.lines().iter().any(|line| line == "p:burst_requested:false"), "{:?}", probe.lines());
}

#[test]
fn rs_19_a_timed_session_action_is_not_untimed() {
    let probe = Probe::new();
    let rig = rig(ezsdr_kernel::module_api::Pacing::FreeRunning);
    let root = rig.root;
    let mut assembly = rig.assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).declaring("radio/tx", 1, 1).registering_at_arm("radio/tx")));
    let mut run = connect(&profile_one(), &Default::default(), assembly, Lease::attached()).unwrap();
    let entry = run
        .submit(
            SessionAction::Vocabulary {
                ns: ns("test"),
                verb: Ident::parse("start_repeat").unwrap(),
                target: support::target("radio/tx"),
                at: Some(TimePoint::new(root, 5_000_000)),
                params: BTreeMap::new(),
            },
            Some(&[0u8; 80]),
        )
        .unwrap();
    assert!(matches!(entry.outcome, Outcome::Admitted { .. }));
    let _ = run.finish_checked();
    assert!(probe.lines().iter().any(|line| line == "p:burst_untimed:false"));
}

#[test]
fn kc_28_an_untimed_update_verb_to_a_resource_waits_for_the_provider_lead() {
    // #54: a verb compiles to the target it names, so an `UpdateParameter` verb naming a
    // resource is admitted no earlier than its Provider's lead, as `SetParameter` is.
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.registry = run_registry_classed();
    assembly.providers.insert(
        Ident::parse("radio").unwrap(),
        Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2).with_min_command_lead(Duration::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 2_000_000)), &probe)),
    );
    let mut run = connect(&profile_one(), &Default::default(), assembly, Lease::attached()).unwrap();
    let entry = run
        .submit(
            SessionAction::Vocabulary {
                ns: ns("test"),
                verb: Ident::parse("capture").unwrap(),
                target: support::target("radio"),
                at: None,
                params: [(Key::parse("test.capture").unwrap(), Value::Scalar(Scalar::Bool(true)))].into_iter().collect(),
            },
            None,
        )
        .unwrap();
    assert!(
        matches!(entry.outcome, Outcome::Admitted { ref coercions, .. }
        if coercions.iter().any(|c| c.key == Key::parse("ezsdr.action.at").unwrap()
            && c.applied == Value::Scalar(Scalar::Int(2_000_000)))),
        "{:?}",
        entry.outcome
    );
    let _ = run.finish_checked();
}

#[test]
fn kc_28_an_untimed_burst_is_not_admitted_before_its_clock_s_origin() {
    // RS-19: the target stream's clock begins at T0 + 5 ms, after now + the 2 ms lead,
    // so "as soon as possible" is that origin, its tick 0, and the coercion records it.
    untimed_burst_lands_at_its_clock_s_origin(Some(2_000_000));
}

#[test]
fn kc_28_an_untimed_burst_waits_for_its_clock_s_origin_without_a_provider_lead() {
    // RS-19: the origin applies to a Provider that declares no min_command_lead too.
    untimed_burst_lands_at_its_clock_s_origin(None);
}

fn untimed_burst_lands_at_its_clock_s_origin(lead_ns: Option<i64>) {
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    let mut provider = TestProvider::new("radio", 2);
    if let Some(lead_ns) = lead_ns {
        provider = provider.with_min_command_lead(Duration::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, lead_ns));
    }
    assembly
        .providers
        .insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", provider, &probe).declaring("radio/tx", 1, 1).registering_at_arm("radio/tx").with_arm_origin_after(5_000_000)));
    let mut run = connect(&profile_one(), &Default::default(), assembly, Lease::attached()).unwrap();
    let entry = run
        .submit(SessionAction::Vocabulary { ns: ns("test"), verb: Ident::parse("start_repeat").unwrap(), target: support::target("radio/tx"), at: None, params: BTreeMap::new() }, Some(&[0u8; 80]))
        .unwrap();
    assert!(
        matches!(entry.outcome, Outcome::Admitted { ref coercions, .. }
        if coercions.iter().any(|c| c.key == Key::parse("ezsdr.action.at").unwrap()
            && c.applied == Value::Scalar(Scalar::Int(5_000_000)))),
        "{entry:?}"
    );
    assert!(probe.lines().iter().any(|line| line == "p:burst_at:0"), "{:?}", probe.lines());
    let _ = run.finish_checked();
}

#[test]
fn kc_36_detached_lease_expiry_ends_the_run() {
    let mut rig = rig(ezsdr_kernel::module_api::Pacing::FreeRunning);
    let lease = Lease::detached(5000, false, "tok", &*rig.host).unwrap();
    rig.assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(TestProvider::new("radio", 2)));
    let mut run = connect(&profile_one(), &Default::default(), rig.assembly, lease).unwrap();
    run.disconnect();
    rig.host.advance(5000);
    run.check_lease();
    assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Stopped { cause: StopCause::LeaseExpiry {} } }));
    let _ = run.finish_checked();
}

#[test]
fn kc_36_attached_disconnect_ends_the_run() {
    let mut run = session_with_provider(Box::new(TestProvider::new("radio", 2)));
    run.disconnect();
    assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Stopped { cause: StopCause::ClientDisconnect {} } }));
    let _ = run.finish_checked();
}

#[test]
fn kc_37_run_child_is_refused() {
    let mut run = session_with_provider(Box::new(TestProvider::new("radio", 2)));
    let entry = run.submit(SessionAction::RunChild { spec_hash: ContentHash::of_bytes(b"child spec"), binding_hash: ContentHash::of_bytes(b"child profile") }, None).unwrap();
    assert!(matches!(entry.outcome, Outcome::Rejected { ref violations }
        if violations.iter().any(|v| v.check == ns("ezsdr.run_child"))));
    let _ = run.finish_checked();
}

#[test]
fn kc_35_connect_refuses_an_invalid_lease() {
    let lease = Lease { mode: LeaseMode::Detached { ttl_ms: 0, renewable: false }, token: Some("t".to_owned()), holder: None, expires_at_host: None, adoptions: 0, released: false };
    let error = match connect(&profile_one(), &Default::default(), rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly, lease) {
        Ok(_) => panic!("zero TTL is refused"),
        Err(error) => error,
    };
    assert!(matches!(error, SpecError::Structural { reason } if reason.contains("RS-21")));
}

#[test]
fn kc_28_stop_run_ends_the_session() {
    let mut run = session_with_provider(Box::new(TestProvider::new("radio", 2)));
    let entry = run.submit(SessionAction::Stop { target: None }, None).unwrap();
    assert!(matches!(entry.outcome, Outcome::Admitted { .. }));
    assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Stopped { cause: StopCause::Client {} } }));
    let _ = run.finish_checked();
}

#[test]
fn kc_30_a_panicking_module_fails_the_run_not_the_process() {
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).panicking_in_step()));
    let run = start_spec_run(&spec_one(), &profile_one(), assembly).unwrap();
    assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Run, .. } }));
    let manifest = run.finish_checked();
    assert!(failure(&manifest).starts_with("KC-30: radio: a Module panicked"));
    assert!(manifest.hash.is_some());
}

#[test]
fn kc_30_a_panic_in_coerce_fails_validate() {
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(TestProvider::new("radio", 2).panicking_in_coerce()));
    let run = start_spec_run(&spec_one(), &profile_one(), assembly).unwrap();
    assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Validate, .. } }));
    let manifest = run.finish_checked();
    assert!(failure(&manifest).starts_with("KC-30: a Module panicked during validate"));
    assert!(manifest.termination.cleanup_failures.is_empty());
}

#[test]
fn kc_30_device_lost_is_the_kernel_event_and_aborts() {
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).device_lost_at(0)));
    let run = start_spec_run(&spec_one(), &profile_one(), assembly).unwrap();
    let manifest = run.finish_checked();
    let kind = ezsdr_kernel::event::EventKind::parse("ezsdr.DEVICE_LOST").unwrap();
    let source = support::node_source("radio");
    assert!(manifest.events.delivered.iter().any(|event| event.kind == kind && event.source == source && event.severity == ezsdr_kernel::event::Severity::Fatal));
    assert!(manifest.events.counters.iter().any(|row| row.source == source && row.kind == kind && row.count == 1));
    assert!(matches!(manifest.termination.reason, Termination::Stopped {
        cause: StopCause::Policy { event: event_kind }
    } if event_kind == kind));
    assert!(manifest.run.transitions.iter().any(|row| matches!(row.state, RunState::Stopping { mode: ezsdr_kernel::run::CleanupMode::Abort })));
}

#[test]
fn kc_31_mark_artifact_marks_only_artifacts_open_then() {
    let (mut spec, profile) = output_docs();
    spec["policies"] = serde_json::json!({ "failure": { "test.custom": "mark_artifact" } });
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(
        Ident::parse("radio").unwrap(),
        Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).emitting("test.custom", ezsdr_kernel::event::Severity::Warning, 150).with_wakeups(&[150])),
    );
    assembly.sinks.insert(Ident::parse("rec").unwrap(), Box::new(RecordingSink::new("rec", &probe).returning_spans(&[(100, 200), (300, 400)])));
    assembly.links.insert(mref("ezsdr.test.link"), Box::new(TestLinkModule::new(&probe)));
    let mut run = start_spec_run(&spec, &profile, assembly).unwrap();
    run.advance_to(TimePoint::new(run.now().domain(), 160)).unwrap();
    let manifest = run.finish_checked();
    let kind = ezsdr_kernel::event::EventKind::parse("test.custom").unwrap();
    let rec_artifacts = &manifest.artifacts[&Ident::parse("rec").unwrap()];
    assert_eq!(rec_artifacts[0].id, Ident::parse("rec_0").unwrap());
    assert_eq!(rec_artifacts[0].marks.len(), 1);
    assert_eq!(rec_artifacts[0].marks[0].kind, kind);
    let t = rec_artifacts[0].marks[0].time;
    assert_eq!(t.ticks_in(t.domain()).unwrap(), 150);
    assert!(rec_artifacts[1].marks.is_empty());
}

#[test]
fn kc_30_a_panic_after_a_device_lost_in_one_round_fails_the_run() {
    // #65 (note 25): a DeviceLost first in the round's fault list does not hide a later
    // panic. The Policy lets the lost device continue, so only the panic ends the Run.
    let (mut spec, profile) = output_docs();
    spec["policies"] = serde_json::json!({ "failure": { "ezsdr.DEVICE_LOST": "continue" } });
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).device_lost_at(0)));
    assembly.sinks.insert(Ident::parse("rec").unwrap(), Box::new(RecordingSink::new("rec", &probe).panicking_step_at(0)));
    assembly.links.insert(mref("ezsdr.test.link"), Box::new(TestLinkModule::new(&probe)));
    let manifest = start_spec_run(&spec, &profile, assembly).unwrap().finish_checked();
    assert!(matches!(manifest.termination.reason, Termination::Failed { stage: Stage::Run, .. }));
    let reason = failure(&manifest);
    assert!(reason.starts_with("KC-30: rec: a Module panicked"), "{reason}");
    let lost = ezsdr_kernel::event::EventKind::parse("ezsdr.DEVICE_LOST").unwrap();
    let radio = support::node_source("radio");
    assert!(manifest.events.delivered.iter().any(|e| e.kind == lost && e.source == radio));
}

#[test]
fn kc_32_a_policy_reaction_a_failure_overrode_is_recorded_in_also() {
    // Review of #65: the Policy stops on the lost device, but a panic in the same round
    // ends the Run Failed first (KC-32 keeps the first end); the overridden reaction
    // is recorded beside it, once, not dropped.
    let (mut spec, profile) = output_docs();
    spec["policies"] = serde_json::json!({ "failure": { "ezsdr.DEVICE_LOST": "stop" } });
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).device_lost_at(0)));
    assembly.sinks.insert(Ident::parse("rec").unwrap(), Box::new(RecordingSink::new("rec", &probe).panicking_step_at(0)));
    assembly.links.insert(mref("ezsdr.test.link"), Box::new(TestLinkModule::new(&probe)));
    let manifest = start_spec_run(&spec, &profile, assembly).unwrap().finish_checked();
    assert!(matches!(manifest.termination.reason, Termination::Failed { stage: Stage::Run, .. }));
    assert_eq!(
        manifest.termination.also,
        vec![Termination::Stopped { cause: StopCause::Policy { event: ezsdr_kernel::event::EventKind::parse(ezsdr_kernel::event::EventKind::DEVICE_LOST).unwrap() } }]
    );
}

#[test]
fn kc_32_a_repeated_reaction_is_recorded_once() {
    // The Run fails first; its Provider then emits the same `stop` kind again from
    // `stop`. Each overridden reaction is recorded once, not once per event (KC-32).
    let (mut spec, profile) = output_docs();
    spec["policies"] = serde_json::json!({ "failure": { "test.custom": "stop" } });
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(
        Ident::parse("radio").unwrap(),
        Box::new(
            SteppedProvider::new("p", TestProvider::new("radio", 2), &probe)
                .emitting("test.custom", ezsdr_kernel::event::Severity::Warning, 0)
                .emitting_in_stop("test.custom", ezsdr_kernel::event::Severity::Warning),
        ),
    );
    assembly.sinks.insert(Ident::parse("rec").unwrap(), Box::new(RecordingSink::new("rec", &probe).panicking_step_at(0)));
    assembly.links.insert(mref("ezsdr.test.link"), Box::new(TestLinkModule::new(&probe)));
    let manifest = start_spec_run(&spec, &profile, assembly).unwrap().finish_checked();
    let custom = ezsdr_kernel::event::EventKind::parse("test.custom").unwrap();
    assert_eq!(manifest.events.delivered.iter().filter(|e| e.kind == custom).count(), 2);
    assert!(matches!(manifest.termination.reason, Termination::Failed { stage: Stage::Run, .. }));
    assert_eq!(manifest.termination.also, vec![Termination::Stopped { cause: StopCause::Policy { event: custom } }]);
}

#[test]
fn kc_32_a_later_failure_is_recorded_in_also() {
    // Two instances fail in one round: the first fixes the Termination with its
    // reason, the second is recorded in `also` with its own (KC-30, KC-32).
    let (spec, profile) = output_docs();
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).panicking_in_step()));
    assembly.sinks.insert(Ident::parse("rec").unwrap(), Box::new(RecordingSink::new("rec", &probe).panicking_step_at(0)));
    assembly.links.insert(mref("ezsdr.test.link"), Box::new(TestLinkModule::new(&probe)));
    let manifest = start_spec_run(&spec, &profile, assembly).unwrap().finish_checked();
    assert!(failure(&manifest).starts_with("KC-30: radio: "), "{}", failure(&manifest));
    let [Termination::Failed { stage: Stage::Run, reason }] = manifest.termination.also.as_slice() else {
        panic!("one later failure: {:?}", manifest.termination.also);
    };
    assert!(reason.starts_with("KC-30: rec: a Module panicked"), "{reason}");
}

#[test]
fn kc_30_a_next_wakeup_panic_in_the_cleanup_drain_is_recorded() {
    // F25 (a): the orderly cleanup drain records an Authority panic in `next_wakeup` as
    // `run_loop` does, rather than reading it as "no wakeup"; the Run's first end
    // stands, and the failure escalates cleanup, so the Sink stops in abort mode
    // (KC-32, RS-6 step 3, RS-10).
    let (spec, profile) = output_docs();
    let probe = Probe::new();
    let mut assembly = rig_with_faulting_time(SimAuthority::panicking_next_wakeup).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe)));
    assembly.sinks.insert(Ident::parse("rec").unwrap(), Box::new(RecordingSink::new("rec", &probe)));
    assembly.links.insert(mref("ezsdr.test.link"), Box::new(TestLinkModule::new(&probe)));
    let manifest = start_spec_run(&spec, &profile, assembly).unwrap().finish_checked();
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Client {} });
    assert_eq!(manifest.termination.also, vec![Termination::Failed { stage: Stage::Run, reason: "KC-30: authority panicked during next_wakeup".to_owned() }]);
    let lines = probe.lines();
    assert!(lines.iter().any(|line| line == "rec:stop:Abort"), "{lines:?}");
}

#[test]
fn kc_30_a_next_wakeup_panic_in_the_run_loop_fails_the_run() {
    // KC-30: a panic in the Authority's `next_wakeup` in KC-20's loop ends the Run
    // `Failed { run }` in abort mode — it neither escapes `advance_to` nor reads as
    // "no wakeup" and completes the Run. The cleanup drain's own panic is
    // `kc_30_a_next_wakeup_panic_in_the_cleanup_drain_is_recorded`.
    let (spec, profile) = output_docs();
    let probe = Probe::new();
    let mut assembly = rig_with_faulting_time(SimAuthority::panicking_next_wakeup).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe)));
    assembly.sinks.insert(Ident::parse("rec").unwrap(), Box::new(RecordingSink::new("rec", &probe)));
    assembly.links.insert(mref("ezsdr.test.link"), Box::new(TestLinkModule::new(&probe)));
    let mut run = start_spec_run(&spec, &profile, assembly).unwrap();
    let domain = run.now().domain();
    let error = run.advance_to(TimePoint::new(domain, 10)).expect_err("the loop must end the Run");
    let ezsdr_kernel::coordinator::RunHandleError::Ended { termination } = error else {
        panic!("advance_to must report Ended: {error:?}");
    };
    assert_eq!(termination, Termination::Failed { stage: Stage::Run, reason: "KC-30: authority panicked during next_wakeup".to_owned() });
    let lines = probe.lines();
    assert!(lines.iter().any(|line| line == "rec:stop:Abort"), "{lines:?}");
    let manifest = run.finish_checked();
    assert!(manifest.hash.is_some());
}

#[test]
fn kc_30_a_panic_in_cleanup_is_a_cleanup_failure() {
    // KC-30: a panic in a Module's `cleanup` (which returns nothing) is that step's
    // `CleanupFailure` with the reason "a Module panicked", and the Manifest is still
    // sealed.
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(TestProvider::new("radio", 2).panicking_cleanup()));
    let manifest = start_spec_run(&spec_one(), &profile_one(), assembly).unwrap().finish_checked();
    assert!(
        manifest.termination.cleanup_failures.iter().any(|failure| {
            failure.step == ezsdr_kernel::run::CleanupStep::RestoreBaseline
                && failure.fragment.as_ref() == Some(&Ident::parse("radio").unwrap())
                && failure.reason == "a Module panicked"
                && !failure.timed_out
        }),
        "{:?}",
        manifest.termination.cleanup_failures
    );
    assert!(manifest.hash.is_some());
}

/// #66 (note 25): a warning a Provider emits from `stop` is still queued when cleanup
/// reaches step 5 — no round drains after it — and must still mark the artifact open
/// at its instant (RS-30, KC-42).
fn mark_emitted_while_stopping(abort: bool) -> ezsdr_kernel::manifest::Manifest {
    let (mut spec, profile) = output_docs();
    spec["policies"] = serde_json::json!({ "failure": { "test.custom": "mark_artifact" } });
    let probe = Probe::new();
    let mut provider = SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).emitting_in_stop("test.custom", ezsdr_kernel::event::Severity::Warning).with_wakeups(&[150]);
    if abort {
        provider = provider.device_lost_at(150);
    }
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(provider));
    assembly.sinks.insert(Ident::parse("rec").unwrap(), Box::new(RecordingSink::new("rec", &probe).returning_spans(&[(100, 200), (300, 400)])));
    assembly.links.insert(mref("ezsdr.test.link"), Box::new(TestLinkModule::new(&probe)));
    let mut run = start_spec_run(&spec, &profile, assembly).unwrap();
    let _ = run.advance_to(TimePoint::new(run.now().domain(), 160));
    let manifest = run.finish_checked();
    let stopping = manifest.run.transitions.iter().find_map(|row| match row.state {
        RunState::Stopping { mode } => Some(mode),
        _ => None,
    });
    let mode = if abort { ezsdr_kernel::run::CleanupMode::Abort } else { ezsdr_kernel::run::CleanupMode::Orderly };
    assert_eq!(stopping, Some(mode));
    manifest
}

fn assert_marked_at(manifest: &ezsdr_kernel::manifest::Manifest, ticks: i64) {
    let kind = ezsdr_kernel::event::EventKind::parse("test.custom").unwrap();
    let rec_artifacts = &manifest.artifacts[&Ident::parse("rec").unwrap()];
    assert_eq!(rec_artifacts[0].marks.len(), 1, "{:?}", manifest.artifacts);
    assert_eq!(rec_artifacts[0].marks[0].kind, kind);
    let t = rec_artifacts[0].marks[0].time;
    assert_eq!(t.ticks_in(t.domain()).unwrap(), ticks);
    assert!(rec_artifacts[1].marks.is_empty());
}

#[test]
fn kc_42_a_mark_delivered_during_an_orderly_cleanup_reaches_the_artifact() {
    assert_marked_at(&mark_emitted_while_stopping(false), 160);
}

#[test]
fn kc_42_a_mark_delivered_during_an_abort_cleanup_reaches_the_artifact() {
    assert_marked_at(&mark_emitted_while_stopping(true), 150);
}

#[test]
fn kc_42_a_span_in_its_own_clock_bounds_the_marks() {
    // KC-42: an artifact's span is its first `ContinuityMap`'s `first` converted to
    // the primary root and floored, to its last map's `end` converted and rounded up,
    // and a mark applies in `[first, end)`; with no map, or a span or a mark whose own
    // conversion fails, every mark applies. The clock `radio/rx` declares divides the
    // root at 1 root tick per 3 sample ticks, so sample 91 floors to root 30, sample 88
    // rounds up to root 30 and sample 91 rounds up to root 31.
    let (mut spec, mut profile) = output_docs();
    spec["policies"] = serde_json::json!({ "failure": { "test.custom": "mark_artifact" } });
    // A second stepped Provider, whose mark is timed in another root: its own
    // conversion to the primary root fails (KC-42).
    spec["resources"]["aux"] = spec["resources"]["radio"].clone();
    profile["bindings"]["aux"] = profile["bindings"]["radio"].clone();
    profile["bindings"]["radio"]["selector"] = serde_json::json!({ "instance": "radio" });
    profile["bindings"]["aux"]["selector"] = serde_json::json!({ "instance": "aux" });
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(
        Ident::parse("radio").unwrap(),
        Box::new(
            SteppedProvider::new("p", TestProvider::new("radio", 2), &probe)
                .declaring("radio/rx", 1, 3)
                .declaring("radio/tx", 2, 1)
                .registering_at_arm("radio/rx")
                .emitting("test.custom", ezsdr_kernel::event::Severity::Warning, 30)
                .with_wakeups(&[30]),
        ),
    );
    assembly.providers.insert(
        Ident::parse("aux").unwrap(),
        Box::new(
            SteppedProvider::new("aux", TestProvider::new("aux", 2), &probe).emitting_in_domain("test.custom", ezsdr_kernel::event::Severity::Warning, 29, ClockDomainId::UTC).with_wakeups(&[29]),
        ),
    );
    assembly.sinks.insert(
        Ident::parse("rec").unwrap(),
        Box::new(RecordingSink::new("rec", &probe).returning_clock_spans(&[
            // `first` is sample 91, floored to root 30: the root mark at 30 is inside.
            (0, &[(91, 120)]),
            // `end` is sample 88, rounded up to root 30, which is exclusive.
            (0, &[(60, 88)]),
            // `end` is sample 91, rounded up to root 31: the root mark at 30 is inside.
            (0, &[(60, 91)]),
            // Two maps: the span runs from the first's `first` (root 15) to the last's
            // `end` (root 54).
            (0, &[(45, 50), (150, 160)]),
            // Sample clock 1, which the registry never registered: the span's
            // conversion fails, so every mark applies.
            (1, &[(0, 0)]),
            // No map at all: every mark applies.
            (0, &[]),
        ])),
    );
    assembly.links.insert(mref("ezsdr.test.link"), Box::new(TestLinkModule::new(&probe)));
    let mut run = start_spec_run(&spec, &profile, assembly).unwrap();
    let domain = run.now().domain();
    let _ = run.advance_to(TimePoint::new(domain, 30));
    let manifest = run.finish_checked();
    let kind = ezsdr_kernel::event::EventKind::parse("test.custom").unwrap();
    let artifacts = &manifest.artifacts[&Ident::parse("rec").unwrap()];
    assert_eq!(artifacts.len(), 6, "{artifacts:?}");
    let marks_of = |index: usize| {
        let mut marks: Vec<(ClockDomainId, i64)> = artifacts[index]
            .marks
            .iter()
            .map(|mark| {
                assert_eq!(mark.kind, kind);
                let domain = mark.time.domain();
                (domain, mark.time.ticks_in(domain).unwrap())
            })
            .collect();
        marks.sort();
        marks
    };
    let utc = (ClockDomainId::UTC, 29);
    let mut both = vec![utc, (domain, 30)];
    both.sort();
    for index in [0, 2, 3, 4, 5] {
        assert_eq!(marks_of(index), both, "artifact {index}");
    }
    assert_eq!(marks_of(1), vec![utc], "artifact 1: the span's end is exclusive");
}

#[test]
fn rs_36_a_dropped_stopping_body_ends_the_run() {
    // Review U, TG-U1: the ring filled by one step, then a hot-path `DEVICE_LOST` whose
    // body is dropped. Only the escalation flag can end the Run (RS-36, KC-31).
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(
        Ident::parse("radio").unwrap(),
        Box::new(
            SteppedProvider::new("p", TestProvider::new("radio", 2), &probe)
                .emitting("ezsdr.DEVICE_LOST", ezsdr_kernel::event::Severity::Fatal, 150)
                .flooding(ezsdr_kernel::coordinator::EVENT_RING_DEPTH)
                .with_wakeups(&[150]),
        ),
    );
    let mut run = start_spec_run(&spec_one(), &profile_one(), assembly).unwrap();
    let root = run.now().domain();
    let _ = run.advance_to(TimePoint::new(root, 160));
    let manifest = run.finish_checked();
    let kind = ezsdr_kernel::event::EventKind::parse("ezsdr.DEVICE_LOST").unwrap();
    let source = support::node_source("radio");
    assert!(!manifest.events.delivered.iter().any(|event| event.kind == kind));
    // RS-35: the meta-event is stamped at the drain after the round at 150, on the
    // primary root, with the Kernel as its source.
    assert!(
        manifest.events.delivered.iter().any(|event| event.kind.as_str() == "ezsdr.EVENTS_DROPPED"
            && event.payload == serde_json::json!({ "kind": "ezsdr.DEVICE_LOST", "count": 1 })
            && event.time == TimePoint::new(root, 150)
            && event.source == EventSource::Kernel {}),
        "{:?}",
        manifest.events.delivered
    );
    assert!(manifest.events.counters.iter().any(|row| row.source == source && row.kind == kind && row.count == 1));
    assert!(matches!(manifest.termination.reason, Termination::Stopped {
        cause: StopCause::Policy { event: event_kind }
    } if event_kind == kind));
    assert!(manifest.run.transitions.iter().any(|row| matches!(row.state, RunState::Stopping { mode: ezsdr_kernel::run::CleanupMode::Abort })));
}

#[test]
fn kc_32_an_abort_during_orderly_escalates_and_is_recorded() {
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).with_tail(1).device_lost_at(10)));
    let run = start_spec_run(&spec_one(), &profile_one(), assembly).unwrap();
    let manifest = run.finish_checked();
    assert!(manifest.run.transitions.iter().any(|row| matches!(row.state, RunState::Stopping { mode: ezsdr_kernel::run::CleanupMode::Orderly })));
    assert!(matches!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Client {} }));
    assert_eq!(
        manifest.termination.also,
        vec![Termination::Stopped { cause: StopCause::Policy { event: ezsdr_kernel::event::EventKind::parse(ezsdr_kernel::event::EventKind::DEVICE_LOST).unwrap() } }]
    );
    assert!(probe.lines().iter().any(|line| line == "p:cleanup"));
}

#[test]
fn kc_32_a_module_abort_during_an_orderly_cleanup_escalates_and_is_recorded() {
    // The client stops the Run; an Executor then submits `Abort` from its orderly
    // `stop`. The abort escalates the cleanup and is recorded in `also` (KC-32, RS-10).
    let (spec, profile) = executor_docs();
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe)));
    assembly.executors.insert(Ident::parse("exec").unwrap(), Box::new(ProbeExecutor::new("x", &probe).aborting_in_orderly_stop("late")));
    let manifest = start_spec_run(&spec, &profile, assembly).unwrap().finish_checked();
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Client {} });
    assert_eq!(manifest.termination.also, vec![Termination::Stopped { cause: StopCause::Abort { message: "late".to_owned() } }]);
}

#[test]
fn kc_32_a_reaction_equal_to_the_first_end_is_not_recorded() {
    // The Policy's `stop` reaction ends the Run; the Provider emits the same kind again
    // from `stop`. A request equal to the Termination says nothing new (KC-32).
    let (mut spec, profile) = output_docs();
    spec["policies"] = serde_json::json!({ "failure": { "test.custom": "stop" } });
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(
        Ident::parse("radio").unwrap(),
        Box::new(
            SteppedProvider::new("p", TestProvider::new("radio", 2), &probe)
                .emitting("test.custom", ezsdr_kernel::event::Severity::Warning, 0)
                .emitting_in_stop("test.custom", ezsdr_kernel::event::Severity::Warning),
        ),
    );
    assembly.sinks.insert(Ident::parse("rec").unwrap(), Box::new(RecordingSink::new("rec", &probe)));
    assembly.links.insert(mref("ezsdr.test.link"), Box::new(TestLinkModule::new(&probe)));
    let manifest = start_spec_run(&spec, &profile, assembly).unwrap().finish_checked();
    let custom = ezsdr_kernel::event::EventKind::parse("test.custom").unwrap();
    assert_eq!(manifest.events.delivered.iter().filter(|e| e.kind == custom).count(), 2);
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Policy { event: custom } });
    assert!(manifest.termination.also.is_empty(), "{:?}", manifest.termination.also);
}

#[test]
fn kc_39_every_instance_is_stopped_before_it_is_cleaned_up() {
    let (mut spec, mut profile) = output_docs();
    let mut component = support::recorder_component(support::cf32());
    component.id = Ident::parse("c1").unwrap();
    component.implementation.id = "c1".to_owned();
    spec["graph"] = serde_json::json!({ "components": { "c1": component } });
    profile["bindings"]["exec"] = serde_json::json!({
        "module": { "id": "ezsdr.test.executor", "version": { "major": 1, "minor": 0, "patch": 0 } }
    });
    profile["placements"]["islands"] = serde_json::json!([
        { "id": { "node": 0, "local": 0 }, "executor": "exec",
          "components": [{ "component": "c1", "memory_domain": { "node": 0, "local": 0 } }] }
    ]);
    let probe = Probe::new();
    let mut assembly = output_assembly(&probe, None, None);
    assembly.executors.insert(Ident::parse("exec").unwrap(), Box::new(ProbeExecutor::new("x", &probe)));
    let run = start_spec_run(&spec, &profile, assembly).unwrap();
    let manifest = run.finish_checked();
    let lines = probe.lines();
    for name in ["p", "x", "rec"] {
        let stop = lines.iter().position(|line| line.starts_with(&format!("{name}:stop:"))).unwrap();
        let cleanup = lines.iter().position(|line| line == &format!("{name}:cleanup")).unwrap();
        assert!(stop < cleanup, "{name}: stop index {stop}, cleanup index {cleanup}");
    }
    assert!(manifest.artifacts.values().flatten().any(|artifact| artifact.id == Ident::parse("rec_0").unwrap()));
}

#[test]
fn kc_41_a_failing_sink_stop_is_a_cleanup_failure() {
    let (spec, profile) = output_docs();
    let probe = Probe::new();
    let mut assembly = output_assembly(&probe, None, None);
    assembly.sinks.insert(Ident::parse("rec").unwrap(), Box::new(RecordingSink::new("rec", &probe).failing_stop()));
    let manifest = start_spec_run(&spec, &profile, assembly).unwrap().finish_checked();
    assert!(
        manifest
            .termination
            .cleanup_failures
            .iter()
            .any(|failure| failure.step == ezsdr_kernel::run::CleanupStep::DrainAndStopConsumers && failure.fragment.as_ref() == Some(&Ident::parse("rec").unwrap()))
    );
    assert!(manifest.artifacts.is_empty());
}

#[test]
fn kc_44_a_wedged_step_does_not_prevent_the_manifest() {
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).wedged_in_stop()));
    let run = start_spec_run(&spec_one(), &profile_one(), assembly).unwrap();
    let started = std::time::Instant::now();
    let manifest = run.finish_checked();
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
    assert!(manifest.termination.cleanup_failures.iter().any(|failure| failure.step == ezsdr_kernel::run::CleanupStep::StopProviders && failure.timed_out));
    assert!(manifest.termination.cleanup_failures.iter().any(|failure| failure.step == ezsdr_kernel::run::CleanupStep::ReleaseAndWriteManifest && failure.reason.starts_with("KC-44")));
    assert!(manifest.hash.is_some());
}

#[test]
fn kc_44_a_wedged_provider_stop_does_not_block_other_cleanup() {
    let (mut spec, mut profile) = output_docs();
    let aux_resource = spec["resources"]["radio"].clone();
    spec["resources"]["aux"] = aux_resource;
    let aux_binding = profile["bindings"]["radio"].clone();
    profile["bindings"]["aux"] = aux_binding;
    profile["bindings"]["radio"]["selector"] = serde_json::json!({ "instance": "radio" });
    profile["bindings"]["aux"]["selector"] = serde_json::json!({ "instance": "aux" });

    let probe = Probe::new();
    let mut assembly = output_assembly(&probe, None, None);
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("wedged", TestProvider::new("radio", 2), &probe).wedged_in_stop()));
    assembly.providers.insert(Ident::parse("aux").unwrap(), Box::new(SteppedProvider::new("healthy", TestProvider::new("aux", 2), &probe).with_wakeups(&[1])));

    let started = std::time::Instant::now();
    let manifest = start_spec_run(&spec, &profile, assembly).unwrap().finish_checked();
    assert!(started.elapsed() < std::time::Duration::from_millis(ezsdr_kernel::run::DEFAULT_CLEANUP_DEADLINE_MS * 2));

    let lines = probe.lines();
    assert!(
        lines.iter().any(|line| line == "rec:stop:Orderly"),
        "{lines:?}; failure: {:?}; termination: {:?}; failures: {:?}",
        failure(&manifest),
        manifest.termination.reason,
        manifest.termination.cleanup_failures,
    );
    let healthy_step = lines.iter().position(|line| line == "healthy:step:1").expect("the drain must step the healthy Provider");
    let sink_stop = lines.iter().position(|line| line == "rec:stop:Orderly").expect("the Sink must stop after the drain");
    assert!(healthy_step < sink_stop, "{lines:?}");
    assert!(lines.iter().any(|line| line == "healthy:cleanup"), "{lines:?}");
    assert!(lines.iter().any(|line| line == "rec:cleanup"), "{lines:?}");
    assert!(manifest.artifacts.values().flatten().any(|artifact| artifact.id == Ident::parse("rec_0").unwrap()));
    assert!(
        manifest
            .termination
            .cleanup_failures
            .iter()
            .any(|failure| failure.step == ezsdr_kernel::run::CleanupStep::StopProviders && failure.fragment.as_ref() == Some(&Ident::parse("radio").unwrap()) && failure.timed_out)
    );
    assert!(
        manifest.termination.cleanup_failures.iter().any(|failure| failure.step == ezsdr_kernel::run::CleanupStep::RestoreBaseline
            && failure.fragment.as_ref() == Some(&Ident::parse("radio").unwrap())
            && failure.reason.starts_with("KC-39:"))
    );
}

#[test]
fn kc_45_manifest_fields() {
    let (spec, profile) = output_docs();
    let probe = Probe::new();
    let fidelity = ezsdr_kernel::module_api::Fidelity { timing: ezsdr_kernel::module_api::EnvelopeFidelity::Envelope, ..ezsdr_kernel::module_api::Fidelity::NONE };
    let mut assembly = output_assembly(&probe, Some(100), None);
    assembly.providers.insert(
        Ident::parse("radio").unwrap(),
        Box::new(
            SteppedProvider::new("p", TestProvider::new("radio", 2).with_fidelity(fidelity).with_section("ezsdr.test.provider.details", serde_json::json!({"samples": 1})), &probe)
                .publishing_every(100),
        ),
    );
    let mut run = start_spec_run(&spec, &profile, assembly).unwrap();
    let id = run.id();
    run.advance_to(TimePoint::new(run.now().domain(), 350)).unwrap();
    let manifest = run.finish_checked();
    assert_eq!(manifest.run.kind, ezsdr_kernel::manifest::RunKind::Spec);
    assert!(manifest.run.parent.is_none());
    assert_eq!(manifest.run.id, id);
    assert_eq!(manifest.run.execution_class, Some(ezsdr_kernel::module_api::ExecutionClass::Simulation));
    assert!(manifest.run.deterministic);
    assert_eq!(manifest.run.fidelity.timing, ezsdr_kernel::module_api::EnvelopeFidelity::Envelope);
    assert!(
        manifest
            .events
            .counters
            .iter()
            .any(|row| row.source == EventSource::Output { output: Ident::parse("rec").unwrap() } && row.kind == ezsdr_kernel::event::EventKind::parse("ezsdr.DEVICE_LOST").unwrap() && row.count == 0)
    );
    assert!(manifest.events.counters.iter().any(|row| row.source == EventSource::Kernel {} && row.kind == ezsdr_kernel::event::EventKind::parse("ezsdr.STEP_LIVELOCK").unwrap() && row.count == 0));
    assert!(manifest.policy.is_some() && manifest.plan.is_some());
    assert_eq!(manifest.prepare.reports.len(), 2);
    for module in ["ezsdr.test.provider", "ezsdr.test.sink", "ezsdr.test.link"] {
        assert!(manifest.modules.iter().any(|entry| entry.module.id.as_str() == module && entry.impl_hash.is_some()));
    }
    assert_eq!(manifest.vocabularies[&ns("test")], ezsdr_kernel::module_api::Version::new(1, 0, 0));
    // One typed record per created link, naming its `plan.links` entry, which holds
    // its endpoints; the Kernel writes no `ezsdr.*` section of its own (KC-45).
    let plan = manifest.plan.as_ref().unwrap();
    assert_eq!(manifest.links, vec![ezsdr_kernel::manifest::LinkRecord { link: plan.links[0].id, drops: Some(0) }]);
    assert_eq!(serde_json::to_value(&plan.links[0].from).unwrap(), serde_json::json!({"component": "radio", "port": "rx"}));
    assert!(manifest.sections.iter().all(|section| !section.name.as_str().starts_with("ezsdr.") || section.name.as_str().starts_with("ezsdr.test.")), "{:?}", manifest.sections);
    // The Lease is recorded as its mode and release, with no token and no deadline.
    assert_eq!(serde_json::to_value(manifest.lease).unwrap(), serde_json::json!({ "mode": { "kind": "attached" }, "released": true }));
    assert_eq!(manifest.section(&support::node_source("radio"), "ezsdr.test.provider.details").unwrap()["samples"], 1);
    assert!(manifest.lease.released);
    assert!(matches!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Client {} }));
}

#[test]
fn ka_18_nonzero_link_drops_reach_the_manifest() {
    // SC-20: the Manifest's `links` is `shared.links` and step 6's `link_drops` zipped by
    // index, so a record must carry **its own** link's count. One link cannot tell that
    // apart from a record that always reads the first: the Plan here has two, and only
    // the Island's drops.
    // The feed keeps `output_docs`'s capacity of 4, so only the Island's link drops.
    let (mut spec, mut profile) = output_docs();
    let mut component = support::recorder_component(support::cf32());
    component.id = Ident::parse("c1").unwrap();
    component.implementation.id = "c1".to_owned();
    spec["graph"] = serde_json::json!({
        "components": { "c1": component },
        "links": [{
            "from": { "component": "radio", "port": "rx" },
            "to": { "component": "c1", "port": "in" },
            "policy": "drop_oldest", "capacity": 1
        }]
    });
    profile["bindings"]["exec"] = serde_json::json!({
        "module": { "id": "ezsdr.test.executor", "version": { "major": 1, "minor": 0, "patch": 0 } }
    });
    profile["placements"]["islands"] = serde_json::json!([
        { "id": { "node": 0, "local": 0 }, "executor": "exec",
          "components": [{ "component": "c1", "memory_domain": { "node": 0, "local": 0 } }] }
    ]);
    // `create_links` finds a placement by its two ends, so the graph link needs its own
    // ahead of the feed's.
    profile["placements"]["links"].as_array_mut().unwrap().insert(
        0,
        serde_json::json!({
            "link": { "id": "ezsdr.test.link", "version": { "major": 1, "minor": 0, "patch": 0 } },
            "from": { "component": "radio", "port": "rx" },
            "to": { "component": "c1", "port": "in" }
        }),
    );
    let probe = Probe::new();
    let mut assembly = output_assembly(&probe, None, None);
    assembly.executors.insert(Ident::parse("exec").unwrap(), Box::new(ProbeExecutor::new("x", &probe)));
    assembly.links.insert(mref("ezsdr.test.link"), Box::new(TestLinkModule::new(&probe).seeding_one_drop_into("c1")));
    let run = start_spec_run(&spec, &profile, assembly).expect("entry creates a Run");
    let manifest = run.finish_checked();
    let plan = manifest.plan.as_ref().expect("the plan reached the Manifest").links.clone();
    assert_eq!(plan.len(), 2);
    assert_eq!(manifest.links.len(), 2);
    for (i, record) in manifest.links.iter().enumerate() {
        assert_eq!(record.link, plan[i].id, "record {i} names its own plan link");
    }
    assert_eq!(manifest.links[0].drops, Some(1));
    assert_eq!(manifest.links[1].drops, Some(0));
}

#[test]
fn kc_45_sample_clocks_and_domains_are_recorded() {
    let rig = rig(ezsdr_kernel::module_api::Pacing::FreeRunning);
    let clocks = rig.clocks.clone();
    let mut assembly = rig.assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("dev", 2), &Probe::new()).declaring("dev/rx", 10, 1).registering_at_arm("dev/rx")));
    let manifest = start_spec_run(&spec_one(), &profile_one(), assembly).unwrap().finish_checked();
    assert_eq!(manifest.clocks.sample_clocks.len(), 1);
    assert_eq!(manifest.clocks.sample_clocks[0].stream, ezsdr_kernel::id::ResourceId::parse("dev/rx").unwrap());
    assert_eq!(manifest.clocks.domains, clocks.domains());
    assert!(manifest.clocks.relations.is_empty());
}

#[test]
fn kc_45_a_provider_section_outside_its_namespace_is_a_cleanup_failure() {
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(TestProvider::new("radio", 2).with_section("other.ns", serde_json::json!({}))));
    let manifest = start_spec_run(&spec_one(), &profile_one(), assembly).unwrap().finish_checked();
    assert!(manifest.sections.iter().all(|section| section.name.as_str() != "other.ns"));
    assert!(manifest.termination.cleanup_failures.iter().any(|failure| failure.step == ezsdr_kernel::run::CleanupStep::ReleaseAndWriteManifest && failure.reason.starts_with("KC-44")));
}

#[test]
fn kc_24_a_module_update_is_not_coerced_by_the_kernel() {
    let (spec, profile) = executor_docs();
    let probe = Probe::new();
    let provider = TestProvider::new("radio", 2);
    let coerce_calls = provider.coerce_calls.clone();
    let coerce_calls_at_submit = Arc::new(std::sync::atomic::AtomicU64::new(u64::MAX));
    let action = Action::UpdateParameter {
        target: support::target("radio"),
        key: Key::parse("test.gain").unwrap(),
        value: Value::num(3.0).unwrap(),
        class: ezsdr_kernel::module_api::UpdateClass::HardwareTimed,
        at: None,
    };
    let mut assembly = rig(Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", provider, &probe)));
    assembly
        .executors
        .insert(Ident::parse("exec").unwrap(), Box::new(ProbeExecutor::new("x", &probe).submitting(action).snapshot_coercions_before_submit(coerce_calls.clone(), coerce_calls_at_submit.clone())));
    let run = start_spec_run(&spec, &profile, assembly).unwrap();
    let manifest = run.finish_checked();
    assert!(probe.lines().iter().any(|line| line.starts_with("x:submit:ok:")));
    let coerce_calls_at_submit = coerce_calls_at_submit.load(std::sync::atomic::Ordering::SeqCst);
    assert_ne!(coerce_calls_at_submit, u64::MAX);
    assert_eq!(coerce_calls.load(std::sync::atomic::Ordering::SeqCst), coerce_calls_at_submit, "the Module-origin Action must not invoke Provider::coerce",);
    assert!(probe.lines().iter().any(|line| line.starts_with("p:action:UpdateParameter:radio@")));
    assert!(manifest.termination.cleanup_failures.is_empty());
}

#[test]
fn kc_24_a_module_update_must_state_its_providers_declared_class() {
    for component_shadow in [false, true] {
        let (mut spec, profile) = executor_docs();
        if component_shadow {
            spec["graph"]["components"]["c1"]["params"] = serde_json::json!([{
                "key": "test.gain", "schema": {"type": "number"},
                "update_class": "cold", "default": 0.0
            }]);
        }
        let probe = Probe::new();
        let action = Action::UpdateParameter {
            target: support::target("radio"),
            key: Key::parse("test.gain").unwrap(),
            value: Value::num(3.0).unwrap(),
            class: ezsdr_kernel::module_api::UpdateClass::Cold,
            at: None,
        };
        let mut assembly = rig(Pacing::FreeRunning).assembly;
        assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe)));
        assembly.executors.insert(Ident::parse("exec").unwrap(), Box::new(ProbeExecutor::new("x", &probe).submitting(action)));
        let manifest = start_spec_run(&spec, &profile, assembly).unwrap().finish_checked();
        assert!(probe.lines().iter().any(|line| line.starts_with("x:submit:err:ezsdr.update_class:RS-52: update states") && line.contains("which declares")));
        assert!(!probe.lines().iter().any(|line| line.starts_with("p:action:UpdateParameter:")));
        assert!(manifest.termination.cleanup_failures.is_empty());
    }
}

#[test]
fn kc_30_a_panicking_link_descriptor_fails_plan_without_unwinding() {
    let (spec, profile) = output_docs();
    let probe = Probe::new();
    let mut assembly = output_assembly(&probe, None, None);
    assembly.links.insert(mref("ezsdr.test.link"), Box::new(TestLinkModule::new(&probe).panicking_descriptor()));
    let run = start_spec_run(&spec, &profile, assembly).expect("entry returns a Run");
    let manifest = run.finish_checked();
    assert!(matches!(manifest.termination.reason, Termination::Failed { stage: Stage::Plan, .. }));
    assert!(failure(&manifest).starts_with("KC-30: a Module panicked during plan"));
}

#[test]
fn kc_08_an_executor_event_from_a_second_island_has_its_own_counter() {
    let (mut spec, mut profile) = executor_docs();
    let mut second = spec["graph"]["components"]["c1"].clone();
    second["id"] = serde_json::json!("c2");
    second["impl"]["id"] = serde_json::json!("c2");
    spec["graph"]["components"]["c2"] = second;
    profile["placements"]["islands"].as_array_mut().unwrap().push(serde_json::json!({
        "id": { "node": 0, "local": 1 }, "executor": "exec",
        "components": [{ "component": "c2", "memory_domain": { "node": 0, "local": 0 } }]
    }));
    let island_1 = EventSource::Island { island: ezsdr_kernel::id::IslandId { node: ezsdr_kernel::id::NodeId::LOCAL, local: 1 } };
    let probe = Probe::new();
    let mut assembly = rig(Pacing::FreeRunning).assembly;
    assembly = with_provider(assembly, "radio", "radio");
    assembly
        .executors
        .insert(Ident::parse("exec").unwrap(), Box::new(ProbeExecutor::new("x", &probe).emitting_from(island_1.clone(), "test.custom", ezsdr_kernel::event::Severity::Info).checking_event_views()));
    let manifest = start_spec_run(&spec, &profile, assembly).unwrap().finish_checked();
    assert!(
        manifest.events.counters.iter().any(|row| row.source == island_1 && row.kind.as_str() == "test.custom" && row.count == 1),
        "counters: {:?}; delivered: {:?}",
        manifest.events.counters,
        manifest.events.delivered
    );
    assert!(!manifest.events.counters.iter().any(|row| row.source == EventSource::Unforeseen {} && row.kind.as_str() == "test.custom" && row.count > 0));
    // Item 3 (Sol review P1-1): the two fragments of one Executor were prepared with
    // one view each, and those views refuse each other's handle — the previous view
    // takes its own, neither takes the other's.
    let lines = probe.lines();
    assert!(lines.iter().any(|line| line == "events_cross:x:own=true:previous_refuses_current=true:current_refuses_previous=true"), "{lines:?}");
}

#[test]
fn kc_45_a_rejected_admission_is_kept_in_the_manifest() {
    let mut assembly = rig(Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(TestProvider::new("radio", 1)));
    let manifest = start_spec_run(&spec_one(), &profile_one(), assembly).unwrap().finish_checked();
    assert!(!manifest.admission.is_admitted());
    assert!(!manifest.admission.rejected.is_empty());
}

#[test]
fn kc_24_a_prepare_abort_stops_before_arm_and_start() {
    let probe = Probe::new();
    let (spec, profile) = output_docs();
    let mut assembly = output_assembly(&probe, None, None);
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).aborting_in_prepare("prepare requested abort")));
    let run = start_spec_run(&spec, &profile, assembly).unwrap();
    assert!(matches!(run.state(), RunState::CleanedUp {
        termination: Termination::Stopped { cause: StopCause::Abort { message: ref cause } }
    } if cause == "prepare requested abort"));
    let lines = probe.lines();
    assert!(!lines.iter().any(|line| line == "rec:prepare:rec"));
    assert!(!lines.iter().any(|line| line == "p:arm" || line.starts_with("p:start")));
}

#[test]
fn ka_13_an_invalid_provider_namespace_without_sections_is_not_a_failure() {
    let mut profile = profile_one();
    profile["bindings"]["radio"]["module"]["id"] = serde_json::json!("TestProvider");
    let mut assembly = rig(Pacing::FreeRunning).assembly;
    assembly.registry = run_registry_non_namespace_provider();
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(TestProvider::new("radio", 2).with_module(mref("TestProvider"))));
    let manifest = start_spec_run(&spec_one(), &profile, assembly).unwrap().finish_checked();
    assert!(!manifest.termination.cleanup_failures.iter().any(|failure| failure.reason.contains("Module id TestProvider is not a Namespace")));
}

#[test]
fn rs_39_a_section_from_an_instance_whose_id_is_not_a_namespace_is_a_cleanup_failure() {
    // The other branch of RS-39 for such an instance: no sections is no failure
    // (`ka_13_…`), but a section it hands in is refused — the writer has no namespace to
    // be under — and recorded as a cleanup failure, so a reader of the Manifest sees
    // that the Module's section is missing rather than assuming it wrote nothing.
    let mut profile = profile_one();
    profile["bindings"]["radio"]["module"]["id"] = serde_json::json!("TestProvider");
    let mut assembly = rig(Pacing::FreeRunning).assembly;
    assembly.registry = run_registry_non_namespace_provider();
    assembly
        .providers
        .insert(Ident::parse("radio").unwrap(), Box::new(TestProvider::new("radio", 2).with_module(mref("TestProvider")).with_section("test.provider.details", serde_json::json!({ "samples": 1 }))));
    let manifest = start_spec_run(&spec_one(), &profile, assembly).unwrap().finish_checked();
    assert!(manifest.sections.is_empty(), "{:?}", manifest.sections);
    assert!(manifest.termination.cleanup_failures.iter().any(|failure| failure.reason.contains("Module id TestProvider is not a Namespace")), "{:?}", manifest.termination.cleanup_failures);
}

#[test]
fn kc_36_finish_checks_an_expired_detached_lease() {
    let mut rig = rig(Pacing::FreeRunning);
    let lease = Lease::detached(5000, false, "tok", &*rig.host).unwrap();
    rig.assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(TestProvider::new("radio", 2)));
    let mut run = connect(&profile_one(), &Default::default(), rig.assembly, lease).unwrap();
    run.disconnect();
    rig.host.advance(5000);
    let manifest = run.finish_checked();
    assert!(matches!(manifest.termination.reason, Termination::Stopped { cause: StopCause::LeaseExpiry {} }));
}

#[test]
fn rs_24_both_refused_lease_outcomes_are_logged() {
    // RS-24's two refusals, through a Session: a wrong token is `AdoptRejected` and a
    // `Renew` on a non-renewable Lease is `LeaseNotRenewable`, and **both outcomes are
    // logged**, because the client acts on the log. Swallowing the refusal recorded
    // `Admitted`, so the client read that it had reclaimed a Lease it never held and
    // the Run then ended at the TTL it believed it had pushed out.
    let mut first = rig(Pacing::FreeRunning);
    let lease = Lease::detached(5000, true, "tok", &*first.host).unwrap();
    first.assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(TestProvider::new("radio", 2)));
    let mut run = connect(&profile_one(), &Default::default(), first.assembly, lease).unwrap();
    let wrong = SessionAction::Adopt { token: "wrong".to_owned() };
    let entry = run.submit(wrong.clone(), None).unwrap();
    let Outcome::Rejected { violations } = entry.outcome else { panic!("a wrong token is refused, not admitted: {:?}", entry.outcome) };
    assert_eq!(violations[0].check, ns("ezsdr.lease"));
    assert!(violations[0].reason.contains("RS-24") && violations[0].reason.contains("adoption token does not match"), "{violations:?}");
    let manifest = run.finish_checked();
    assert!(manifest.action_log.iter().any(|e| e.action == wrong && matches!(e.outcome, Outcome::Rejected { .. })), "{:?}", manifest.action_log);

    // The other outcome: the Lease is Detached and not renewable (RS-24).
    let mut fixed = rig(Pacing::FreeRunning);
    let lease = Lease::detached(5000, false, "tok", &*fixed.host).unwrap();
    fixed.assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(TestProvider::new("radio", 2)));
    let mut run = connect(&profile_one(), &Default::default(), fixed.assembly, lease).unwrap();
    let renew = SessionAction::Renew {};
    let entry = run.submit(renew.clone(), None).unwrap();
    let Outcome::Rejected { violations } = entry.outcome else { panic!("a non-renewable Lease is refused, not admitted: {:?}", entry.outcome) };
    assert_eq!(violations[0].check, ns("ezsdr.lease"));
    assert!(violations[0].reason.contains("RS-24") && violations[0].reason.contains("not renewable"), "{violations:?}");
    let manifest = run.finish_checked();
    assert!(manifest.action_log.iter().any(|e| e.action == renew && matches!(e.outcome, Outcome::Rejected { .. })), "{:?}", manifest.action_log);
}

#[test]
fn ka_18_a_failed_link_drop_snapshot_is_unknown_not_zero() {
    let (spec, profile) = output_docs();
    let probe = Probe::new();
    let mut assembly = output_assembly(&probe, None, None);
    assembly.links.insert(mref("ezsdr.test.link"), Box::new(TestLinkModule::new(&probe).panicking_drops()));
    let manifest = start_spec_run(&spec, &profile, assembly).unwrap().finish_checked();
    assert_eq!(manifest.links[0].drops, None);
    assert!(manifest.termination.cleanup_failures.iter().any(|failure| failure.step == ezsdr_kernel::run::CleanupStep::FlushEvents));
}

#[test]
fn ka_12_an_abandoned_drain_does_not_stop_a_sink_after_cleanup() {
    let (spec, profile) = output_docs();
    let probe = Probe::new();
    let gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let mut assembly = output_assembly(&probe, None, None);
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).with_wakeups(&[1]).blocking_step_at(1, gate.clone())));
    let manifest = start_spec_run(&spec, &profile, assembly).unwrap().finish_checked();
    assert!(manifest.termination.cleanup_failures.iter().any(|failure| failure.step == ezsdr_kernel::run::CleanupStep::DrainAndStopConsumers && failure.timed_out));
    assert!(manifest.termination.cleanup_failures.iter().any(|failure| failure.step == ezsdr_kernel::run::CleanupStep::RestoreBaseline && failure.reason.starts_with("KC-39:")));
    let lines = probe.lines();
    assert!(!lines.iter().any(|line| line == "rec:stop:Orderly"));
    assert!(!lines.iter().any(|line| line == "rec:cleanup"));

    let (released, changed) = &*gate;
    *released.lock().unwrap_or_else(|e| e.into_inner()) = true;
    changed.notify_all();
    assert!(probe.wait_for("p:released_step:1", std::time::Duration::from_secs(2)));
    assert!(!probe.wait_for_count("rec:stop:Orderly", 1, std::time::Duration::from_secs(1),));
    assert!(!probe.wait_for_count("rec:cleanup", 1, std::time::Duration::from_secs(1),));
    let lines = probe.lines();
    assert!(!lines.iter().any(|line| line == "rec:stop:Orderly"), "the abandoned drain owner must not stop the Sink after cleanup: {lines:?}");
    assert!(!lines.iter().any(|line| line == "rec:cleanup"), "the abandoned drain owner must not clean up the Sink after the Manifest: {lines:?}");
}

#[test]
fn ka_12_an_abandoned_wakeup_drain_stops_the_sink_before_cleanup() {
    let (spec, profile) = output_docs();
    let probe = Probe::new();
    let gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let mut rig = rig(Pacing::FreeRunning);
    let (authority, root) = SimAuthority::new(&rig.clocks, mref("ezsdr.test.provider"), Pacing::FreeRunning);
    rig.root = root;
    rig.manual = authority.manual();
    rig.assembly.authority = Box::new(authority.blocking_next_wakeup(gate.clone(), TimePoint::new(root, 1), &probe));
    rig.assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).with_wakeups(&[1])));
    rig.assembly.sinks.insert(Ident::parse("rec").unwrap(), Box::new(RecordingSink::new("rec", &probe)));
    rig.assembly.links.insert(mref("ezsdr.test.link"), Box::new(TestLinkModule::new(&probe)));

    let manifest = start_spec_run(&spec, &profile, rig.assembly).unwrap().finish_checked();
    assert!(manifest.termination.cleanup_failures.iter().any(|failure| failure.step == ezsdr_kernel::run::CleanupStep::DrainAndStopConsumers && failure.timed_out));
    let lines = probe.lines();
    let stop = lines.iter().position(|line| line == "rec:stop:Orderly").unwrap_or_else(|| panic!("fallback drain_and_stop_consumers was skipped: {lines:?}"));
    let cleanup = lines.iter().position(|line| line == "rec:cleanup").unwrap_or_else(|| panic!("Sink cleanup was skipped: {lines:?}"));
    assert!(stop < cleanup, "fallback drain_and_stop_consumers must precede cleanup: {lines:?}");
    assert_eq!(lines.iter().filter(|line| *line == "rec:stop:Orderly").count(), 1);

    let (released, changed) = &*gate;
    *released.lock().unwrap_or_else(|e| e.into_inner()) = true;
    changed.notify_all();
    assert!(probe.wait_for("a:next_wakeup_returned:1", std::time::Duration::from_secs(2)));
    assert!(!probe.wait_for("p:step:1", std::time::Duration::from_secs(1)), "the abandoned drain owner must not step a Module after cleanup");
    assert!(!probe.wait_for_count("rec:stop:Orderly", 2, std::time::Duration::from_secs(1),), "the abandoned drain owner must not stop the Sink twice");
    let lines = probe.lines();
    assert_eq!(lines.iter().filter(|line| *line == "rec:stop:Orderly").count(), 1, "the abandoned drain owner must not stop the Sink after cleanup: {lines:?}");
    assert_eq!(lines.iter().filter(|line| *line == "rec:cleanup").count(), 1, "the abandoned drain owner must not clean up the Sink twice: {lines:?}");
    assert!(!lines.iter().any(|line| line == "p:step:1"), "the abandoned drain owner must not step a Module after cleanup: {lines:?}");
}

#[test]
fn kc_30_a_panicking_time_now_fails_validate_without_unwinding() {
    let rig = rig_with_faulting_time(SimAuthority::panicking_now);
    let assembly = with_provider(rig.assembly, "radio", "radio");
    let run = start_spec_run(&spec_one(), &profile_one(), assembly).unwrap();
    assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Validate, .. } }));
    let manifest = run.finish_checked();
    let reason = failure(&manifest);
    assert!(reason.starts_with("KC-30: a Module panicked during now()"), "{reason}");
}

#[test]
fn kc_30_a_panicking_time_now_during_arm_does_not_start_modules() {
    let mut rig = rig(Pacing::FreeRunning);
    let (authority, root) = SimAuthority::new(&rig.clocks, mref("ezsdr.test.provider"), Pacing::FreeRunning);
    rig.root = root;
    rig.manual = authority.manual();
    rig.assembly.authority = Box::new(authority.panicking_now_on_call(6));

    let probe = Probe::new();
    rig.assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe)));
    let run = start_spec_run(&spec_one(), &profile_one(), rig.assembly).unwrap();
    assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Arm, .. } }));
    let manifest = run.finish_checked();
    assert!(failure(&manifest).starts_with("KC-30: a Module panicked during now()"));
    assert!(probe.lines().iter().any(|line| line == "p:arm"));
    assert!(!probe.lines().iter().any(|line| line.starts_with("p:start:")));
}

#[test]
fn kc_30_a_panicking_time_schedule_fails_arm_without_unwinding() {
    let mut spec = spec_one();
    add_schedule(&mut spec, "radio", 100, serde_json::json!({ "kind": "stop", "target": null }));
    let rig = rig_with_faulting_time(SimAuthority::panicking_schedule);
    let mut assembly = rig.assembly;
    assembly
        .providers
        .insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &Probe::new()).declaring("radio/rx", 10, 1).registering_at_arm("radio/rx")));
    let run = start_spec_run(&spec, &profile_one(), assembly).unwrap();
    let manifest = run.finish_checked();
    assert!(matches!(manifest.termination.reason, Termination::Failed { stage: Stage::Arm, .. }));
    assert!(failure(&manifest).starts_with("KC-30: a Module panicked during arm: Authority schedule()",));
}

#[test]
fn kc_30_a_panicking_time_cancel_is_a_cleanup_failure() {
    // KC-30: a panicking cancel is a cleanup failure. F25 (b): the double's first
    // cancel succeeds and every later one panics. With two handles one panic is
    // reported; with three, the handle after a panicking one is still tried.
    let freeze_failures = |handles: i64| {
        let mut spec = spec_one();
        add_schedule(&mut spec, "radio", 100, serde_json::json!({ "kind": "stop", "target": null }));
        for i in 1..handles {
            let mut item = spec["schedule"][0].clone();
            item["at"]["offset_ticks"] = serde_json::json!(100 * (i + 1));
            spec["schedule"].as_array_mut().unwrap().push(item);
        }
        let rig = rig_with_faulting_time(SimAuthority::panicking_cancel);
        let mut assembly = rig.assembly;
        assembly
            .providers
            .insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &Probe::new()).declaring("radio/rx", 10, 1).registering_at_arm("radio/rx")));
        let manifest = start_spec_run(&spec, &profile_one(), assembly).unwrap().finish_checked();
        manifest.termination.cleanup_failures.into_iter().filter(|failure| failure.step == ezsdr_kernel::run::CleanupStep::FreezeDispatch).map(|failure| failure.reason).collect::<Vec<_>>()
    };
    for (handles, counted) in [(2, "1 of 2"), (3, "2 of 3")] {
        assert_eq!(freeze_failures(handles), [format!("KC-30: a Module panicked during cleanup: Authority cancel(), {counted} handles")]);
    }
}

// ---------------------------------------------------------------- Phase 3 amendments (KB)

#[test]
fn kb_01_a_provider_reads_a_spec_input_by_hash() {
    let mut spec = spec_one();
    let (bytes, reference) = input_ref();
    add_schedule(&mut spec, "radio", 10, tx_template("radio/tx", &reference, "send_asap_and_flag"));
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).declaring("radio/tx", 1, 1).registering_at_arm("radio/tx")));
    let size = bytes.len();
    assembly.inputs.insert(reference.hash.clone(), bytes);
    let run = start_spec_run(&spec, &profile_one(), assembly).unwrap();
    let manifest = run.finish_checked();
    assert_eq!(manifest.inputs.len(), 1);
    assert_eq!(probe.with_prefix("p:burst_input:"), vec![format!("p:burst_input:{size}")]);
}

#[test]
fn kb_01_a_provider_reads_a_session_waveform_by_hash() {
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).declaring("radio/tx", 1, 1).registering_at_arm("radio/tx")));
    let mut run = connect(&profile_one(), &Default::default(), assembly, Lease::attached()).unwrap();
    let bytes: Vec<u8> = (0..800u32).map(|i| (i % 251) as u8).collect();
    let entry = run
        .submit(SessionAction::Vocabulary { ns: ns("test"), verb: Ident::parse("start_repeat").unwrap(), target: support::target("radio/tx"), at: None, params: BTreeMap::new() }, Some(&bytes))
        .unwrap();
    assert!(matches!(entry.outcome, Outcome::Admitted { .. }));
    let _ = run.finish_checked();
    assert_eq!(probe.with_prefix("p:burst_input:"), vec!["p:burst_input:800".to_owned()]);
}

#[test]
fn kb_01_b_the_store_keeps_no_unverified_entry() {
    // an `Assembly.inputs` entry no schedule entry references is not kept (KC-9), so a Session
    // waveform whose hash it was filed under is stored as submitted, not as the stale 3 bytes
    let probe = Probe::new();
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).declaring("radio/tx", 1, 1).registering_at_arm("radio/tx")));
    let bytes: Vec<u8> = (0..800u32).map(|i| (i % 251) as u8).collect();
    assembly.inputs.insert(ezsdr_kernel::hash::ContentHash::of_bytes(&bytes), vec![1, 2, 3]);
    let mut run = connect(&profile_one(), &Default::default(), assembly, Lease::attached()).unwrap();
    let entry = run
        .submit(SessionAction::Vocabulary { ns: ns("test"), verb: Ident::parse("start_repeat").unwrap(), target: support::target("radio/tx"), at: None, params: BTreeMap::new() }, Some(&bytes))
        .unwrap();
    assert!(matches!(entry.outcome, Outcome::Admitted { .. }));
    let _ = run.finish_checked();
    assert_eq!(probe.with_prefix("p:burst_input:"), vec!["p:burst_input:800".to_owned()]);
}

#[test]
fn kb_02_the_manifest_records_the_fidelity_settled_in_prepare() {
    let probe = Probe::new();
    let settled = ezsdr_kernel::module_api::Fidelity { rf: ezsdr_kernel::module_api::RfFidelity::ImpairmentModel, ..ezsdr_kernel::module_api::Fidelity::NONE };
    let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).settling_fidelity(settled)));
    let run = start_spec_run(&spec_one(), &profile_one(), assembly).unwrap();
    let manifest = run.finish_checked();
    assert_eq!(manifest.run.fidelity, settled);
}

/// A Sink whose `step` always reports progress, so a round can never quiesce.
struct ForeverSink {
    probe: Probe,
    descriptor: ezsdr_kernel::module_api::SinkDescriptor,
    steps: Arc<std::sync::atomic::AtomicUsize>,
}

impl ezsdr_kernel::module_api::Sink for ForeverSink {
    fn descriptor(&self) -> &ezsdr_kernel::module_api::SinkDescriptor {
        &self.descriptor
    }
    fn prepare(&mut self, _f: &ezsdr_kernel::plan::Fragment, _c: ezsdr_kernel::module_api::PrepareContext) -> Result<ezsdr_kernel::plan::PrepareReport, ezsdr_kernel::module_api::ModuleError> {
        Ok(ezsdr_kernel::plan::PrepareReport { effective: BTreeMap::new(), coercions: Vec::new(), warnings: Vec::new() })
    }
    fn arm(&mut self) -> Result<(), ezsdr_kernel::module_api::ModuleError> {
        Ok(())
    }
    fn start(&mut self) -> Result<(), ezsdr_kernel::module_api::ModuleError> {
        Ok(())
    }
    fn step(&mut self, _until: TimePoint) -> Result<ezsdr_kernel::module_api::StepOutcome, ezsdr_kernel::module_api::ModuleError> {
        self.steps.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(ezsdr_kernel::module_api::StepOutcome { progressed: true })
    }
    fn stop(&mut self, _m: ezsdr_kernel::module_api::StopMode) -> Result<Vec<ezsdr_kernel::manifest::ArtifactRef>, ezsdr_kernel::module_api::ModuleError> {
        self.probe.record("forever:stop");
        Ok(Vec::new())
    }
    fn cleanup(&mut self) {}
}

/// KD-1 with the STEP_ROUND_CAP: a Provider that reports `device_lost` in the same
/// round as a Sink that can never quiesce. The Run must stop on DEVICE_LOST, not on
/// the Kernel's own STEP_LIVELOCK (Phase 4 review P1-1; found by the SpaceBunny
/// second opinion, whose probe this is).
#[test]
fn kd_01_a_device_lost_is_not_reported_as_a_step_livelock() {
    let (spec, profile) = output_docs();
    let probe = Probe::new();
    let mut assembly = output_assembly(&probe, None, None);
    let steps = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    assembly.sinks.insert(Ident::parse("rec").unwrap(), Box::new(ForeverSink { probe: probe.clone(), descriptor: RecordingSink::new("rec", &probe).descriptor.clone(), steps: steps.clone() }));
    let mut provider = SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).with_wakeups(&[1]);
    provider.device_lost_at = Some(0);
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(provider));

    let manifest = start_spec_run(&spec, &profile, assembly).unwrap().finish_checked();
    assert_eq!(
        manifest.termination.reason,
        Termination::Stopped { cause: StopCause::Policy { event: ezsdr_kernel::event::EventKind::parse(ezsdr_kernel::event::EventKind::DEVICE_LOST).expect("kind") } },
        "a device_lost must not be reported as the Kernel's own STEP_LIVELOCK"
    );
    assert!(manifest.events.delivered.iter().all(|e| e.kind.as_str() != ezsdr_kernel::event::EventKind::STEP_LIVELOCK));
}

/// KD-1 steps the instances after a failure, so a second failure in one round is now
/// found. The first decides the termination; every `DeviceLost` of the round is
/// still reported as `DEVICE_LOST` (Phase 4 Review D, P1-1).
#[test]
fn kd_01_every_lost_device_of_a_round_is_reported_and_the_first_failure_decides() {
    let run = |p_lost: bool, q_lost: bool| {
        let (spec, profile) = distinct_resource_docs(&["p", "q"]);
        let probe = Probe::new();
        let mut assembly = rig(ezsdr_kernel::module_api::Pacing::FreeRunning).assembly;
        let provider = |name: &str, lost: bool| {
            let double = SteppedProvider::new(name, TestProvider::new(name, 2), &probe).with_wakeups(&[1]);
            if lost { double.device_lost_at(0) } else { double.step_error_at(0) }
        };
        assembly.providers.insert(Ident::parse("p").unwrap(), Box::new(provider("p", p_lost)));
        assembly.providers.insert(Ident::parse("q").unwrap(), Box::new(provider("q", q_lost)));
        start_spec_run(&spec, &profile, assembly).unwrap().finish_checked()
    };
    let lost_sources = |manifest: &ezsdr_kernel::manifest::Manifest| -> Vec<String> {
        manifest.events.delivered.iter().filter(|e| e.kind.as_str() == ezsdr_kernel::event::EventKind::DEVICE_LOST).map(|e| e.source.to_string()).collect()
    };

    // `p` sorts first and returns an ordinary error: the Run fails on it, and `q`'s
    // lost device, found in the same round, is still delivered.
    let failed = run(false, true);
    assert!(matches!(failed.termination.reason, Termination::Failed { stage: Stage::Run, .. }));
    assert!(failure(&failed).starts_with("KC-30: p: "), "{}", failure(&failed));
    assert_eq!(lost_sources(&failed).len(), 1);

    // Both lose their device: the Run stops on DEVICE_LOST and both are reported.
    let both = run(true, true);
    assert_eq!(both.termination.reason, Termination::Stopped { cause: StopCause::Policy { event: ezsdr_kernel::event::EventKind::parse(ezsdr_kernel::event::EventKind::DEVICE_LOST).unwrap() } });
    let mut sources = lost_sources(&both);
    sources.dedup();
    assert_eq!(sources.len(), 2, "one DEVICE_LOST per device: {sources:?}");

    // Both fail with an ordinary error: each requests the end, and the first, `p`'s,
    // fixes it (KC-30 handles each failure; KC-32 keeps the first end; #65).
    let errors = run(false, false);
    assert!(matches!(errors.termination.reason, Termination::Failed { stage: Stage::Run, .. }));
    assert!(failure(&errors).starts_with("KC-30: p: "), "{}", failure(&errors));
}

// ---------------------------------------------------------------- Phase 5 amendments (KE)

/// A Spec Run whose test Executor submits one `TxBurst` of `burst` to `radio/tx` on its
/// first step, with `listed` as the Spec's `inputs` and `stored` in `Assembly.inputs`.
fn ke_run(listed: &[ezsdr_kernel::manifest::ArtifactRef], burst: ezsdr_kernel::manifest::ArtifactRef, stored: Option<Vec<u8>>) -> (Probe, ezsdr_kernel::manifest::Manifest) {
    let (mut spec, profile) = executor_docs();
    spec["inputs"] = serde_json::to_value(listed).unwrap();
    let probe = Probe::new();
    let root = rig(Pacing::FreeRunning).root;
    let mut assembly = rig(Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).declaring("radio/tx", 1, 1).registering_at_arm("radio/tx")));
    if let Some(bytes) = stored {
        assembly.inputs.insert(burst.hash.clone(), bytes);
    }
    assembly.executors.insert(
        Ident::parse("exec").unwrap(),
        Box::new(ProbeExecutor::new("x", &probe).submitting(Action::TxBurst {
            target: support::target("radio/tx"),
            waveform: burst,
            repeat: false,
            at: ezsdr_kernel::time::AbsoluteDeadline::new(TimePoint::new(root, 10)),
            requested_at: None,
            late_policy: ezsdr_kernel::stream::LatePolicy::SendAsapAndFlag,
            metadata: BTreeMap::new(),
        })),
    );
    let manifest = start_spec_run(&spec, &profile, assembly).expect("entry creates a Run").finish_checked();
    (probe, manifest)
}

#[test]
fn ke_01_a_declared_input_is_stored_and_recorded() {
    // SB-20a: an input no schedule entry carries is verified and stored, so a Module reads
    // its bytes and names it in a burst; before KE-1 KC-9 dropped it (Phase 5 §2, hole 1).
    let (bytes, reference) = input_ref();
    let (probe, manifest) = ke_run(std::slice::from_ref(&reference), reference.clone(), Some(bytes));
    assert!(probe.lines().iter().any(|line| line.starts_with("x:submit:ok:")), "{:?}", probe.lines());
    assert_eq!(probe.with_prefix("p:burst_input:"), vec!["p:burst_input:80".to_owned()]);
    assert_eq!(manifest.inputs, vec![reference]);
}

#[test]
fn ke_01_a_declared_input_is_verified_as_a_scheduled_one_is() {
    let (bytes, valid) = input_ref();
    let plan_failure = |listed: ezsdr_kernel::manifest::ArtifactRef, stored: Option<Vec<u8>>| {
        let (_, manifest) = ke_run(std::slice::from_ref(&listed), listed.clone(), stored);
        assert!(matches!(manifest.termination.reason, Termination::Failed { stage: Stage::Plan, .. }));
        failure(&manifest).to_owned()
    };
    let absent = plan_failure(valid.clone(), None);
    assert!(absent.starts_with("KC-9: input 0: no bytes were supplied"), "{absent}");
    let changed = plan_failure(valid.clone(), Some(vec![1u8; 80]));
    assert!(changed.starts_with("KC-9: input 0: bytes do not match"), "{changed}");
    let mut too_large = valid.clone();
    too_large.size_bytes = 81;
    let size = plan_failure(too_large, Some(bytes.clone()));
    assert!(size.starts_with("KC-9: input 0: size is 80"), "{size}");
    let mut http = valid.clone();
    http.uri = "http://x".to_owned();
    let uri = plan_failure(http, Some(bytes.clone()));
    assert!(uri.starts_with("KC-9: input 0: uri must begin"), "{uri}");
    // An input is consumed whole: a Spec cannot write the provenance the Run records on
    // what it produces (Review F, P2-3).
    let mut partial = valid.clone();
    partial.partial = true;
    let flag = plan_failure(partial, Some(bytes.clone()));
    assert!(flag.starts_with("KC-9: input 0: an input carries no partial flag"), "{flag}");
    let mut marked = valid.clone();
    marked.marks.push(ezsdr_kernel::manifest::ArtifactMark {
        kind: ezsdr_kernel::event::EventKind::parse(ezsdr_kernel::event::EventKind::DEVICE_LOST).unwrap(),
        time: TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0),
    });
    let mark = plan_failure(marked, Some(bytes.clone()));
    assert!(mark.starts_with("KC-9: input 0: an input carries no partial flag"), "{mark}");
    let mut mapped = valid.clone();
    let point = serde_json::json!({ "domain": { "node": 0, "local": 1 }, "ticks": 0 });
    mapped.continuity.push(
        serde_json::from_value(serde_json::json!({
            "channel_gaps": [], "channels": 1, "domain": { "node": 0, "local": 1 },
            "end": { "domain": { "node": 0, "local": 1 }, "ticks": 10 }, "first": point,
            "gaps": [], "valid": [[{ "len": 10, "start": point }]]
        }))
        .expect("a one-block ContinuityMap"),
    );
    let map = plan_failure(mapped, Some(bytes.clone()));
    assert!(map.starts_with("KC-9: input 0: an input carries no partial flag"), "{map}");
    // Two listed inputs under one name and two hashes: refused.
    let other_bytes = vec![1u8; 80];
    let mut namesake = valid.clone();
    namesake.hash = ContentHash::of_bytes(&other_bytes);
    namesake.uri = format!("mem:{}", namesake.hash);
    let twice = {
        let (mut spec, profile) = executor_docs();
        spec["inputs"] = serde_json::json!([valid, namesake]);
        let mut assembly = rig(Pacing::FreeRunning).assembly;
        assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(TestProvider::new("radio", 2)));
        assembly.executors.insert(Ident::parse("exec").unwrap(), Box::new(ProbeExecutor::new("x", &Probe::new())));
        assembly.inputs.insert(valid.hash.clone(), bytes.clone());
        assembly.inputs.insert(namesake.hash.clone(), other_bytes);
        start_spec_run(&spec, &profile, assembly).unwrap().finish_checked()
    };
    assert!(matches!(twice.termination.reason, Termination::Failed { stage: Stage::Plan, .. }));
    let reason = failure(&twice);
    assert!(reason.starts_with("KC-9: input 0: another input is also called waveform"), "{reason}");
    // A listed input and a scheduled waveform of one name and two hashes: the same refusal
    // (Review G, G-3: the rule compares every input, listed or scheduled).
    let listed_and_scheduled = {
        let mut spec = spec_one();
        spec["inputs"] = serde_json::json!([valid]);
        let target = serde_json::to_value(support::target("radio/tx")).unwrap();
        spec["schedule"] = serde_json::json!([{
            "at": { "clock": "radio", "offset_ticks": 0 },
            "action": { "kind": "tx_burst", "target": target, "waveform": namesake,
                "repeat": false, "late_policy": "send_asap_and_flag", "metadata": {} }
        }]);
        let mut assembly = with_provider(rig(Pacing::FreeRunning).assembly, "radio", "radio");
        assembly.inputs.insert(valid.hash.clone(), bytes.clone());
        assembly.inputs.insert(namesake.hash.clone(), vec![1u8; 80]);
        start_spec_run(&spec, &profile_one(), assembly).unwrap().finish_checked()
    };
    assert!(matches!(listed_and_scheduled.termination.reason, Termination::Failed { stage: Stage::Plan, .. }));
    let reason = failure(&listed_and_scheduled);
    assert!(reason.starts_with("KC-9: input 0: another input is also called waveform"), "{reason}");

    // Listed and scheduled: one input, recorded once, listed first.
    let mut spec = spec_one();
    spec["inputs"] = serde_json::json!([valid]);
    let target = serde_json::to_value(support::target("radio/tx")).unwrap();
    spec["schedule"] = serde_json::json!([{
        "at": { "clock": "radio", "offset_ticks": 0 },
        "action": { "kind": "tx_burst", "target": target, "waveform": valid,
            "repeat": false, "late_policy": "send_asap_and_flag", "metadata": {} }
    }]);
    let mut assembly = with_provider(rig(Pacing::FreeRunning).assembly, "radio", "radio");
    assembly.inputs.insert(valid.hash.clone(), bytes);
    let manifest = start_spec_run(&spec, &profile_one(), assembly).unwrap().finish_checked();
    assert!(!matches!(manifest.termination.reason, Termination::Failed { stage: Stage::Plan, .. }));
    assert_eq!(manifest.inputs, vec![valid]);
}

#[test]
fn ke_02_a_module_burst_must_name_a_run_input() {
    // RS-44a at admission, for a Module's burst: before KE-2 it reached the Provider
    // naming bytes nobody held (Phase 5 §2, hole 2).
    let (bytes, reference) = input_ref();
    let (probe, manifest) = ke_run(&[], reference.clone(), Some(bytes.clone()));
    let refused = format!("x:submit:err:ezsdr.input:RS-44a: {} is not an input of this Run", reference.hash);
    assert!(probe.lines().contains(&refused), "{:?}", probe.lines());
    assert!(probe.with_prefix("p:burst_input:").is_empty(), "the Provider must not receive it");
    assert!(manifest.inputs.is_empty());

    // Either way round: a burst declaring more bytes than the input holds, or fewer
    // (Review F, P2-4).
    for declared in [81, 79] {
        let mut wrong_size = reference.clone();
        wrong_size.size_bytes = declared;
        let (probe, _) = ke_run(std::slice::from_ref(&reference), wrong_size, Some(bytes.clone()));
        let refused = format!("x:submit:err:ezsdr.input:RS-44a: input {} holds 80 bytes, and the burst declares {declared}", reference.hash);
        assert!(probe.lines().contains(&refused), "{:?}", probe.lines());
        assert!(probe.with_prefix("p:burst_input:").is_empty());
    }
}

// ---------------------------------------------------------------- Phase 6: KF-1…KF-3

fn emitting_session(at: Option<i64>, wakeups: &[i64]) -> ezsdr_kernel::coordinator::RunHandle {
    let probe = Probe::new();
    let mut provider = SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).with_wakeups(wakeups);
    if let Some(at) = at {
        provider = provider.emitting("test.custom", ezsdr_kernel::event::Severity::Info, at);
    }
    session_with_provider(Box::new(provider))
}

fn custom() -> ezsdr_kernel::event::EventKind {
    ezsdr_kernel::event::EventKind::parse("test.custom").unwrap()
}

#[test]
fn kf_01_the_delivered_events_are_readable_during_the_run() {
    let mut run = emitting_session(Some(50), &[50]);
    let root = run.now().domain();
    assert!(run.events(0).is_empty());
    run.advance_to(TimePoint::new(root, 60)).unwrap();
    let events = run.events(0);
    let index = events.iter().position(|event| event.kind == custom()).expect("the event is readable before the Run ends");
    assert_eq!(events[index].time, TimePoint::new(root, 50));
    assert_eq!(run.events(index), events[index..].to_vec(), "`from` skips the earlier events");
    assert!(run.events(events.len()).is_empty());
    assert!(run.events(99).is_empty());
    let stop = run.submit(SessionAction::Stop { target: None }, None).unwrap();
    assert!(matches!(stop.outcome, Outcome::Admitted { .. }));
    assert!(matches!(run.state(), RunState::CleanedUp { .. }));
    let after = run.events(0);
    assert_eq!(after[index], events[index], "an index names one event for the whole Run");
    let manifest = run.finish_checked();
    assert_eq!(manifest.events.delivered, after, "the events a client read are the Manifest's");
}

#[test]
fn kf_02_wait_for_returns_at_the_round_that_delivered() {
    let mut run = emitting_session(Some(5_000), &[3_000, 5_000]);
    let root = run.now().domain();
    let found = run.wait_for(&[custom()], 0, TimePoint::new(root, 10_000)).unwrap();
    let index = found.expect("the event arrived before the horizon");
    assert_eq!(run.events(index)[0].kind, custom());
    assert_eq!(run.now().ticks_in(root).unwrap(), 5_000, "the Run stands at the round that delivered the event");
    let _ = run.finish_checked();

    // A kind that never comes: the emitted `test.custom` does not match.
    let mut run = emitting_session(Some(5_000), &[3_000, 5_000]);
    let lost = ezsdr_kernel::event::EventKind::parse(ezsdr_kernel::event::EventKind::DEVICE_LOST).unwrap();
    assert_eq!(run.wait_for(&[lost], 0, TimePoint::new(root, 10_000)).unwrap(), None);
    assert_eq!(run.now().ticks_in(root).unwrap(), 10_000);
    let _ = run.finish_checked();
}

#[test]
fn kf_02_wait_for_stands_at_its_horizon() {
    let mut run = emitting_session(None, &[3_000]);
    let root = run.now().domain();
    assert_eq!(run.wait_for(&[custom()], 0, TimePoint::new(root, 7_000)).unwrap(), None);
    assert_eq!(run.now().ticks_in(root).unwrap(), 7_000, "no match: the Run stands at the horizon");
    assert!(matches!(run.state(), RunState::Running {}));
    let _ = run.finish_checked();
}

#[test]
fn kf_02_wait_for_finds_an_event_already_delivered() {
    let mut run = emitting_session(Some(50), &[50]);
    let root = run.now().domain();
    run.advance_to(TimePoint::new(root, 60)).unwrap();
    let index = run.events(0).iter().position(|event| event.kind == custom()).unwrap();
    assert_eq!(run.wait_for(&[custom()], index, TimePoint::new(root, 1_000)).unwrap(), Some(index));
    assert_eq!(run.now().ticks_in(root).unwrap(), 60, "an event already delivered runs no round");
    assert_eq!(run.wait_for(&[custom()], index + 1, TimePoint::new(root, 1_000)).unwrap(), None);
    assert_eq!(run.now().ticks_in(root).unwrap(), 1_000);
    let _ = run.finish_checked();
}

/// A child's Assembly: one TestProvider under `radio` and its own Authority.
fn child_assembly() -> Assembly {
    with_provider(rig(Pacing::FreeRunning).assembly, "radio", "radio")
}

fn drive_to(ticks: i64) -> impl FnMut(&mut ezsdr_kernel::coordinator::RunHandle) {
    move |child| {
        let root = child.now().domain();
        let _ = child.run_until_end(TimePoint::new(root, ticks));
    }
}

#[test]
fn kf_03_a_child_run_is_admitted_logged_run_and_recorded() {
    let mut run = session_with_provider(Box::new(TestProvider::new("radio", 2)));
    let parent = run.id();
    let before = run.now();
    let mut drove = false;
    let mut drive = |child: &mut ezsdr_kernel::coordinator::RunHandle| {
        drove = true;
        assert_eq!(child.kind(), RunKind::Spec);
        drive_to(100)(child);
    };
    let (entry, child) = run.run_child(&spec_one(), &profile_one(), child_assembly(), &mut drive).unwrap();
    assert!(drove, "the caller drives the child");
    assert!(matches!(entry.outcome, Outcome::Admitted { .. }), "{:?}", entry.outcome);
    let child = child.expect("an admitted child returns its Manifest");
    support::assert_reads_back(&child);
    // The entry names the documents by the hashes the child's Manifest records (RS-45).
    assert_eq!(entry.action, SessionAction::RunChild { spec_hash: child.spec.hash.clone(), binding_hash: child.binding.hash.clone() });
    assert_eq!(child.run.parent, Some(parent.clone()));
    assert_eq!(child.run.kind, RunKind::Spec);
    assert!(child.hash.is_some());
    assert_eq!(run.now(), before, "the parent's time stands still while its child runs");
    let manifest = run.finish_checked();
    assert_eq!(manifest.run.parent, None);
    assert_eq!(manifest.action_log[0], entry);
    assert_eq!(serde_json::to_value(&manifest.run.children).unwrap(), serde_json::json!([{ "seq": entry.seq, "run": child.run.id, "manifest": child.hash }]));
}

#[test]
fn kf_03_rs_25a_refusals() {
    // The parent's profile carries the checked section `test.limits` and an unchecked one.
    let mut parent_profile = profile_one();
    parent_profile["environment"] = serde_json::json!({ "test.limits": { "max": 10 }, "test.note": "a", "test.plan": 1, "test.prep": 1 });
    let checks = || {
        let mut checks = run_checks(true);
        checks.register(Arc::new(PlanOnlyCheck(ns("test.plan"), ezsdr_kernel::binding::CheckStage::Validate)));
        checks.register(Arc::new(PlanOnlyCheck(ns("test.prep"), ezsdr_kernel::binding::CheckStage::Prepare)));
        checks
    };
    let session = || {
        let mut assembly = rig(Pacing::FreeRunning).assembly;
        assembly.checks = checks();
        assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(TestProvider::new("radio", 2)));
        connect(&parent_profile, &Default::default(), assembly, Lease::attached()).expect("valid Session entry")
    };
    let mut other_selector = parent_profile.clone();
    other_selector["bindings"]["radio"]["selector"] = serde_json::json!({ "slot": "elsewhere" });
    let mut other_authority = parent_profile.clone();
    other_authority["bindings"]["clock"] = serde_json::json!({
        "module": { "id": "ezsdr.test.provider", "version": { "major": 1, "minor": 0, "patch": 0 } },
        "selector": { "slot": "clock" }
    });
    other_authority["authority"] = serde_json::json!("clock");
    let mut dropped = parent_profile.clone();
    dropped["environment"].as_object_mut().unwrap().remove("test.limits");
    let mut changed = parent_profile.clone();
    changed["environment"]["test.limits"] = serde_json::json!({ "max": 11 });
    let mut unchecked = parent_profile.clone();
    unchecked["environment"]["test.note"] = serde_json::json!("b");
    // A section only a validate-stage check reads — `sim.channel`, `sim.seed`, `sim.faults`
    // in the real Vocabularies — may differ: §54's sweep over a simulated channel, and §58
    // #14's environment variation (Phase 6 Review H, P0-1).
    let mut planned = parent_profile.clone();
    planned["environment"]["test.plan"] = serde_json::json!(2);
    let mut prepared = parent_profile.clone();
    prepared["environment"]["test.prep"] = serde_json::json!(2);
    let cases = [
        (spec_one(), other_selector, Some("RS-25a: radio binds an instance its parent does not")),
        (spec_one(), other_authority, Some("RS-25a: the Authority")),
        (spec_one(), dropped, Some("RS-25a: section test.limits")),
        (spec_one(), changed, Some("RS-25a: section test.limits")),
        (serde_json::json!({ "version": 1, "resources": 3 }), parent_profile.clone(), Some("RS-25a: the child's Spec")),
        (spec_one(), serde_json::json!({ "version": 1, "bindings": 3 }), Some("RS-25a: the child's profile")),
        (spec_one(), unchecked, None),
        (spec_one(), planned, None),
        (spec_one(), prepared, None),
    ];
    for (spec, profile, refusal) in cases {
        let mut run = session();
        // The child's Assembly registers no check: the parent's judge it (Review H, P1-2).
        let (entry, child) = run.run_child(&spec, &profile, child_assembly(), &mut drive_to(100)).unwrap();
        let manifest = run.finish_checked();
        assert_eq!(manifest.action_log, vec![entry.clone()], "the entry is logged either way");
        match refusal {
            Some(reason) => {
                let Outcome::Rejected { violations } = &entry.outcome else { panic!("admitted: {reason}") };
                assert_eq!(violations.len(), 1);
                assert_eq!(violations[0].check, ns("ezsdr.run_child"));
                assert!(violations[0].reason.starts_with(reason), "{} does not start with {reason}", violations[0].reason);
                assert!(child.is_none());
                assert!(manifest.run.children.is_empty());
                // A document is named by its parsed hash, or by the hash of what was sent
                // when it does not parse (RS-45).
                let spec_hash = match ExperimentSpec::from_json(&spec) {
                    Ok(parsed) => ContentHash::of(&parsed),
                    Err(_) => ContentHash::of_value(&spec),
                };
                let binding_hash = match BindingProfile::from_json(&profile) {
                    Ok(parsed) => ContentHash::of(&parsed),
                    Err(_) => ContentHash::of_value(&profile),
                };
                assert_eq!(entry.action, SessionAction::RunChild { spec_hash: spec_hash.unwrap(), binding_hash: binding_hash.unwrap() });
            }
            None => {
                assert!(matches!(entry.outcome, Outcome::Admitted { .. }), "{:?}", entry.outcome);
                support::assert_reads_back(child.as_ref().expect("an admitted child returns its Manifest"));
            }
        }
    }
}

#[test]
fn kf_03_a_child_inherits_the_lease() {
    let mut rig = rig(Pacing::FreeRunning);
    let lease = Lease::detached(5000, true, "tok", &*rig.host).unwrap();
    rig.assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(TestProvider::new("radio", 2)));
    let mut run = connect(&profile_one(), &Default::default(), rig.assembly, lease).unwrap();
    let (_, child) = run.run_child(&spec_one(), &profile_one(), child_assembly(), &mut drive_to(100)).unwrap();
    let child = child.unwrap();
    support::assert_reads_back(&child);
    assert!(matches!(child.lease.mode, LeaseMode::Detached { .. }), "the child holds its parent's Lease, not an Attached one of its own");
    assert_eq!(child.lease.mode, LeaseMode::Detached { ttl_ms: 5000, renewable: true });
    let _ = run.finish_checked();
}

#[test]
fn kf_03_run_child_is_a_session_verb() {
    let mut spec_run = start_spec_run(&spec_one(), &profile_one(), child_assembly()).unwrap();
    assert!(matches!(spec_run.run_child(&spec_one(), &profile_one(), child_assembly(), &mut drive_to(100)), Err(ezsdr_kernel::coordinator::RunHandleError::NotSession)));
    let _ = spec_run.finish_checked();

    let mut run = session_with_provider(Box::new(TestProvider::new("radio", 2)));
    let entry = run.submit(SessionAction::RunChild { spec_hash: ContentHash::of_value(&spec_one()).unwrap(), binding_hash: ContentHash::of_value(&profile_one()).unwrap() }, None).unwrap();
    let Outcome::Rejected { violations } = entry.outcome else { panic!("submit created a child") };
    assert_eq!(violations[0].check, ns("ezsdr.run_child"));
    assert_eq!(violations[0].reason, "RS-25a: a child Run is created with `run_child`, which carries its documents and Modules");
    run.submit(SessionAction::Stop { target: None }, None).unwrap();
    assert!(matches!(run.run_child(&spec_one(), &profile_one(), child_assembly(), &mut drive_to(100)), Err(ezsdr_kernel::coordinator::RunHandleError::Ended { .. })));
    let _ = run.finish_checked();
}

#[test]
fn kc_28_submit_on_a_spec_run_is_not_session() {
    // KC-28's kind check, on the `submit` side: a Spec Run takes no Session Action.
    // Without it the Action was admitted, dispatched and logged like any other and the
    // Run was silently disturbed — the client's Session verbs are `connect`'s, and
    // `run_child` refuses a Spec Run the same way (kf_03).
    let mut spec_run = start_spec_run(&spec_one(), &profile_one(), child_assembly()).unwrap();
    assert_eq!(spec_run.kind(), RunKind::Spec);
    assert!(matches!(spec_run.submit(SessionAction::Stop { target: None }, None), Err(ezsdr_kernel::coordinator::RunHandleError::NotSession)));
    let manifest = spec_run.finish_checked();
    assert!(manifest.action_log.is_empty(), "the Action took no sequence number and reached no Module: {:?}", manifest.action_log);
}

/// A check that runs at one stage before the runtime, like the `sim` Vocabulary's.
struct PlanOnlyCheck(ezsdr_kernel::spec::Namespace, ezsdr_kernel::binding::CheckStage);

impl ezsdr_kernel::binding::AdmissionCheck for PlanOnlyCheck {
    fn section(&self) -> &ezsdr_kernel::spec::Namespace {
        &self.0
    }
    fn stages(&self) -> &[ezsdr_kernel::binding::CheckStage] {
        std::slice::from_ref(&self.1)
    }
    fn check(
        &self,
        _section: &serde_json::Value,
        _effective: &BTreeMap<Ident, BTreeMap<Key, Value>>,
        _proposed: &BTreeMap<Ident, BTreeMap<Key, Value>>,
        _stage: ezsdr_kernel::binding::CheckStage,
    ) -> Vec<ezsdr_kernel::binding::Violation> {
        Vec::new()
    }
}

/// A Session of two stepped Providers `a` and `b` emitting `test.custom` at `at_a` and `at_b`.
fn two_emitters(at_a: i64, at_b: i64, probe: &Probe) -> ezsdr_kernel::coordinator::RunHandle {
    let (_, profile) = distinct_resource_docs(&["a", "b"]);
    let mut assembly = rig(Pacing::FreeRunning).assembly;
    for (name, at) in [("a", at_a), ("b", at_b)] {
        let provider = SteppedProvider::new(name, TestProvider::new(name, 2), probe).with_wakeups(&[at_a, at_b, 9_000]).emitting("test.custom", ezsdr_kernel::event::Severity::Info, at);
        assembly.providers.insert(Ident::parse(name).unwrap(), Box::new(provider));
    }
    connect(&profile, &Default::default(), assembly, Lease::attached()).expect("valid Session entry")
}

#[test]
fn kf_02_wait_for_returns_the_first_match_and_withdraws_its_horizon() {
    let probe = Probe::new();
    let mut run = two_emitters(3_000, 5_000, &probe);
    let root = run.now().domain();
    let index = run.wait_for(&[custom()], 0, TimePoint::new(root, 8_000)).unwrap().expect("a match");
    assert_eq!(run.events(index)[0].time, TimePoint::new(root, 3_000), "the first match, not a later one");
    assert_eq!(run.now().ticks_in(root).unwrap(), 3_000);
    let second = run.wait_for(&[custom()], index + 1, TimePoint::new(root, 8_000)).unwrap().expect("the second match");
    assert_eq!(run.events(second)[0].time, TimePoint::new(root, 5_000));
    // The early returns withdrew their no-op at 8 000: no round runs there later
    // (Phase 6 Review H, P1-3).
    run.advance_to(TimePoint::new(root, 10_000)).unwrap();
    let steps = probe.lines();
    assert!(steps.iter().any(|line| line == "a:step:9000"));
    assert!(!steps.iter().any(|line| line.ends_with(":step:8000")), "{steps:?}");
    // With both delivered, a wait from index 0 answers the first of them.
    assert_eq!(run.wait_for(&[custom()], 0, TimePoint::new(root, 20_000)).unwrap(), Some(index));
    let _ = run.finish_checked();
}

#[test]
fn kf_02_wait_for_with_no_kinds_is_advance_to() {
    let mut run = emitting_session(Some(50), &[50]);
    let root = run.now().domain();
    assert_eq!(run.wait_for(&[], 0, TimePoint::new(root, 7_000)).unwrap(), None);
    assert_eq!(run.now().ticks_in(root).unwrap(), 7_000);
    let _ = run.finish_checked();
}

#[test]
fn kf_02_wait_for_answers_ended_first() {
    // KC-29's prologue comes before the already-delivered answer (Phase 6 Review H, P0-5).
    let mut run = emitting_session(Some(50), &[50]);
    let root = run.now().domain();
    run.advance_to(TimePoint::new(root, 60)).unwrap();
    run.submit(SessionAction::Stop { target: None }, None).unwrap();
    assert!(matches!(run.wait_for(&[custom()], 0, TimePoint::new(root, 1_000)), Err(ezsdr_kernel::coordinator::RunHandleError::Ended { .. })));
    let _ = run.finish_checked();

    let mut rig = rig(Pacing::FreeRunning);
    let lease = Lease::detached(1_000, false, "tok", &*rig.host).unwrap();
    let probe = Probe::new();
    let provider = SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).with_wakeups(&[50]).emitting("test.custom", ezsdr_kernel::event::Severity::Info, 50);
    rig.assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(provider));
    let mut run = connect(&profile_one(), &Default::default(), rig.assembly, lease).unwrap();
    run.advance_to(TimePoint::new(root, 60)).unwrap();
    run.disconnect();
    rig.host.advance(5_000);
    assert!(matches!(run.wait_for(&[custom()], 0, TimePoint::new(root, 1_000)), Err(ezsdr_kernel::coordinator::RunHandleError::Ended { .. })));
    assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Stopped { cause: StopCause::LeaseExpiry {} } }));
    let _ = run.finish_checked();
}

#[test]
fn kf_03_children_are_recorded_in_order() {
    let mut run = session_with_provider(Box::new(TestProvider::new("radio", 2)));
    run.submit(SessionAction::SetParameter { target: support::target("radio"), key: Key::parse("test.count").unwrap(), value: Value::from(2) }, None).unwrap();
    let (first, one) = run.run_child(&spec_one(), &profile_one(), child_assembly(), &mut drive_to(100)).unwrap();
    let (second, two) = run.run_child(&spec_one(), &profile_one(), child_assembly(), &mut drive_to(100)).unwrap();
    support::assert_reads_back(one.as_ref().expect("an admitted child returns its Manifest"));
    support::assert_reads_back(two.as_ref().expect("an admitted child returns its Manifest"));
    assert_eq!((first.seq, second.seq), (1, 2));
    let manifest = run.finish_checked();
    assert_eq!(
        serde_json::to_value(&manifest.run.children).unwrap(),
        serde_json::json!([
            { "seq": 1, "run": one.as_ref().unwrap().run.id, "manifest": one.unwrap().hash },
            { "seq": 2, "run": two.as_ref().unwrap().run.id, "manifest": two.unwrap().hash }
        ])
    );
}

#[test]
fn kf_03_a_lease_that_expires_during_a_child_ends_the_child_first() {
    // The child reads its Lease copy on the parent's host clock, whatever its Assembly
    // holds (Phase 6 Review H, P1-2).
    let mut rig = rig(Pacing::FreeRunning);
    let lease = Lease::detached(1_000, false, "tok", &*rig.host).unwrap();
    rig.assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(TestProvider::new("radio", 2)));
    let host = rig.host.clone();
    let mut run = connect(&profile_one(), &Default::default(), rig.assembly, lease).unwrap();
    run.disconnect();
    let mut child_assembly = child_assembly();
    child_assembly.host_clock = Arc::new(FakeHostClock::new());
    let mut drive = |child: &mut ezsdr_kernel::coordinator::RunHandle| {
        host.advance(5_000);
        drive_to(100)(child);
    };
    let (_, child) = run.run_child(&spec_one(), &profile_one(), child_assembly, &mut drive).unwrap();
    let child = child.unwrap();
    assert_eq!(child.termination.reason, Termination::Stopped { cause: StopCause::LeaseExpiry {} });
    support::assert_reads_back(&child);
    assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Stopped { cause: StopCause::LeaseExpiry {} } }));
    let _ = run.finish_checked();
}

#[test]
fn kf_03_the_parents_checks_judge_the_child() {
    // A child whose Assembly registers no check is still judged by the parent's, under
    // the section it had to copy (Phase 6 Review H, P1-2).
    let mut profile = profile_one();
    profile["environment"] = serde_json::json!({ "test.limits": { "max_grid": 1.0 } });
    let mut assembly = rig(Pacing::FreeRunning).assembly;
    assembly.checks = run_checks(true);
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(TestProvider::new("radio", 2)));
    let mut run = connect(&profile, &Default::default(), assembly, Lease::attached()).unwrap();
    let mut child_assembly = rig(Pacing::FreeRunning).assembly;
    child_assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(TestProvider::new("radio", 2).with_effective("test.grid", Value::num(5.0).unwrap())));
    let (entry, child) = run.run_child(&spec_one(), &profile, child_assembly, &mut drive_to(100)).unwrap();
    assert!(matches!(entry.outcome, Outcome::Admitted { .. }));
    let child = child.unwrap();
    support::assert_reads_back(&child);
    assert!(matches!(child.termination.reason, Termination::Failed { .. }), "{:?}", child.termination.reason);
    assert!(failure(&child).contains("exceeds the declared ceiling"), "{}", failure(&child));
    let _ = run.finish_checked();
}

#[test]
fn sb_29_a_registered_checks_violation_requested_with_a_non_ascii_map_key_is_recorded_and_the_manifest_reads_back() {
    // SB-9a is withdrawn: a `Value` map key is any string (SB-4), so a registered check's
    // `Violation.requested` carrying a non-ASCII key reaches the admission result and the
    // Manifest, and the refused Run's Manifest still seals and reads back.
    let mut spec = spec_one();
    spec["resources"]["radio"]["requires"]["test.grid"] = serde_json::json!({ "kind": "eq", "value": 20.0 });
    let mut profile = profile_one();
    profile["environment"] = serde_json::json!({ "test.limits": { "max_grid": 1.0 } });
    let mut assembly = with_provider(rig(Pacing::FreeRunning).assembly, "radio", "radio");
    assembly.checks.register(Arc::new(TestLimitsCheck::new().with_non_ascii_requested()));
    let run = start_spec_run(&spec, &profile, assembly).expect("entry creates a Run");
    assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Validate, .. } }), "{:?}", run.state());
    let manifest = run.finish_checked();
    assert_eq!(manifest.admission.violations.len(), 1, "the check's refusal still happens: {:?}", manifest.admission.violations);
    let violation = &manifest.admission.violations[0];
    assert_eq!(violation.check, ns("test.limits"));
    assert_eq!(violation.requested, Some(Value::Map([("radio.ü".to_owned(), Scalar::Int(1))].into_iter().collect())), "the offending value is recorded, non-ASCII key and all: {violation:?}");
    assert!(manifest.hash.is_some(), "the refused Run's Manifest seals: {:?}", manifest.termination.cleanup_failures);
}

/// A Sink whose report carries `test.count` = its number, so that two reports differ.
struct CountingPrepareSink(RecordingSink, i64);
impl ezsdr_kernel::module_api::Sink for CountingPrepareSink {
    fn descriptor(&self) -> &ezsdr_kernel::module_api::SinkDescriptor {
        self.0.descriptor()
    }
    fn prepare(&mut self, f: &ezsdr_kernel::plan::Fragment, ctx: ezsdr_kernel::module_api::PrepareContext) -> Result<ezsdr_kernel::plan::PrepareReport, ezsdr_kernel::module_api::ModuleError> {
        let mut report = self.0.prepare(f, ctx)?;
        report.effective.insert(Key::parse("test.count").unwrap(), Value::from(self.1));
        Ok(report)
    }
    fn arm(&mut self) -> Result<(), ezsdr_kernel::module_api::ModuleError> {
        self.0.arm()
    }
    fn start(&mut self) -> Result<(), ezsdr_kernel::module_api::ModuleError> {
        self.0.start()
    }
    fn step(&mut self, t: TimePoint) -> Result<ezsdr_kernel::module_api::StepOutcome, ezsdr_kernel::module_api::ModuleError> {
        self.0.step(t)
    }
    fn stop(&mut self, m: ezsdr_kernel::module_api::StopMode) -> Result<Vec<ezsdr_kernel::manifest::ArtifactRef>, ezsdr_kernel::module_api::ModuleError> {
        self.0.stop(m)
    }
    fn cleanup(&mut self) {
        self.0.cleanup();
    }
}

#[test]
fn sb_41_a_report_is_filed_under_the_fragment_invoked() {
    // A report names no fragment: the Kernel files it under the one it invoked, so two
    // Sinks' reports cannot be swapped or claimed (spec 27 §3; #17).
    let (mut spec, mut profile) = output_docs();
    let mut second_output = spec["outputs"][0].clone();
    second_output["id"] = serde_json::json!("rec2");
    spec["outputs"].as_array_mut().unwrap().push(second_output);
    profile["bindings"]["rec2"] = profile["bindings"]["rec"].clone();
    let mut second_link = profile["placements"]["links"][0].clone();
    second_link["to"]["component"] = serde_json::json!("rec2");
    profile["placements"]["links"].as_array_mut().unwrap().push(second_link);
    let probe = Probe::new();
    let mut assembly = output_assembly(&probe, None, None);
    for (name, count, spans) in [("rec", 1, &[(100, 200)][..]), ("rec2", 2, &[(100, 200), (300, 400)][..])] {
        assembly.sinks.insert(Ident::parse(name).unwrap(), Box::new(CountingPrepareSink(RecordingSink::new(name, &probe).returning_spans(spans), count)));
    }
    let manifest = start_spec_run(&spec, &profile, assembly).unwrap().finish_checked();
    let count = |fragment: &str| manifest.prepare.reports[&Ident::parse(fragment).unwrap()].effective.get(&Key::parse("test.count").unwrap()).cloned();
    assert_eq!(count("rec"), Some(Value::from(1)));
    assert_eq!(count("rec2"), Some(Value::from(2)));
    // RS-38: each artifact is listed under the output whose Sink returned it.
    let listed: Vec<(String, Vec<String>)> = manifest.artifacts.iter().map(|(output, refs)| (output.to_string(), refs.iter().map(|a| a.id.to_string()).collect())).collect();
    assert_eq!(listed, [("rec".to_owned(), vec!["rec_0".to_owned()]), ("rec2".to_owned(), vec!["rec2_0".to_owned(), "rec2_1".to_owned()]),]);
}

fn component_class_docs() -> (serde_json::Value, serde_json::Value) {
    let (mut spec, mut profile) = executor_docs();
    spec["graph"]["components"]["c1"]["params"] = serde_json::json!([{
        "key": "test.gain", "schema": {"type": "number"}, "update_class": "cold", "default": 0.0
    }]);
    let mut c2 = spec["graph"]["components"]["c1"].clone();
    c2["id"] = serde_json::json!("c2");
    c2["params"][0]["update_class"] = serde_json::json!("hardware_timed");
    spec["graph"]["components"]["c2"] = c2;
    // P1-4 (review): a component that declares `test.gain` fixed for the Run
    // (`update_class` null), and one that does not declare the key at all.
    let mut fixed = spec["graph"]["components"]["c1"].clone();
    fixed["id"] = serde_json::json!("fixed");
    fixed["params"][0]["update_class"] = serde_json::Value::Null;
    spec["graph"]["components"]["fixed"] = fixed;
    let mut undeclared = spec["graph"]["components"]["c1"].clone();
    undeclared["id"] = serde_json::json!("undeclared");
    undeclared["params"] = serde_json::json!([]);
    spec["graph"]["components"]["undeclared"] = undeclared;
    let entries: Vec<serde_json::Value> = ["c1", "c2", "fixed", "undeclared"]
        .into_iter()
        .map(|name| {
            let mut entry = profile["placements"]["islands"][0]["components"][0].clone();
            entry["component"] = serde_json::json!(name);
            entry
        })
        .collect();
    profile["placements"]["islands"][0]["components"] = serde_json::Value::Array(entries);
    (spec, profile)
}

#[test]
fn kc_24_component_update_classes_belong_to_the_target() {
    use ezsdr_kernel::module_api::UpdateClass::{Cold, HardwareTimed};
    for (target, class, accepted) in [
        ("c1", HardwareTimed, false),
        ("c1", Cold, true),
        ("c2", Cold, false),
        ("c2", HardwareTimed, true),
        // P1-4 (review): a fixed declaration and a missing one are both refused, and
        // neither falls back to `test.gain`'s `KeyDecl` (`hardware_timed`), which
        // would admit a `hardware_timed` update.
        ("fixed", HardwareTimed, false),
        ("undeclared", HardwareTimed, false),
    ] {
        let (spec, profile) = component_class_docs();
        let probe = Probe::new();
        let mut assembly = rig(Pacing::FreeRunning).assembly;
        assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe)));
        assembly.executors.insert(
            Ident::parse("exec").unwrap(),
            Box::new(ProbeExecutor::new("x", &probe).submitting(Action::UpdateParameter {
                target: Target::Component { component: Ident::parse(target).unwrap() },
                key: Key::parse("test.gain").unwrap(),
                value: Value::num(3.0).unwrap(),
                class,
                at: None,
            })),
        );
        let run = start_spec_run(&spec, &profile, assembly).unwrap();
        assert!(matches!(run.state(), RunState::Running {}));
        let lines = probe.lines();
        assert_eq!(lines.iter().any(|line| line.starts_with("x:submit:ok:")), accepted, "{lines:?}");
        if !accepted {
            assert!(lines.iter().any(|line| line.starts_with("x:submit:err:ezsdr.update_class:RS-52")));
            // The refused Action is not dispatched to anyone.
            assert!(!lines.iter().any(|line| line.starts_with("x:action:")), "{lines:?}");
        }
        assert!(run.finish_checked().termination.cleanup_failures.is_empty());
    }
}

#[test]
fn kc_17_a_scheduled_update_to_a_component_reaches_its_executor() {
    // P2-3 (review): a timed schedule update to a placed component passes validate's
    // target classification, KC-24's class check and arm's dispatch, and reaches the
    // Executor that owns the component. The entry states no class (item 3): the
    // component's own `ParamDecl` is read at admission.
    let (mut spec, profile) = component_class_docs();
    add_schedule(
        &mut spec,
        "radio",
        10,
        serde_json::json!({
            "kind": "update_parameter",
            "target": { "kind": "component", "component": "c1" },
            "key": "test.gain",
            "value": 3.0
        }),
    );
    let probe = Probe::new();
    let mut assembly = rig(Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).declaring("radio/rx", 1, 1)));
    assembly.executors.insert(Ident::parse("exec").unwrap(), Box::new(ProbeExecutor::new("x", &probe)));
    let run = start_spec_run(&spec, &profile, assembly).unwrap();
    assert!(matches!(run.state(), RunState::Running {}));
    let lines = probe.lines();
    assert!(lines.iter().any(|line| line.starts_with("x:action:component c1")), "{lines:?}");
    assert!(run.finish_checked().termination.cleanup_failures.is_empty());
}

#[test]
fn kc_24_a_component_value_takes_no_vocabulary_kind_check() {
    // KC-24 step 6 (review P1-2): a component parameter has an opaque schema, so a
    // value whose kind differs from the same-named Vocabulary key's is still the
    // component's to judge. `c1` declares `test.gain` (`cold`) and the Vocabulary
    // declares that key `Num`; a `Bool` update reaches the Executor through a Module
    // submission and through a scheduled entry, and is refused in neither.
    let (spec, profile) = component_class_docs();
    let probe = Probe::new();
    let mut assembly = rig(Pacing::FreeRunning).assembly;
    assembly.registry = run_registry_classed();
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe)));
    assembly.executors.insert(
        Ident::parse("exec").unwrap(),
        Box::new(ProbeExecutor::new("x", &probe).submitting(Action::UpdateParameter {
            target: Target::Component { component: Ident::parse("c1").unwrap() },
            key: Key::parse("test.gain").unwrap(),
            value: Value::from(true),
            class: ezsdr_kernel::module_api::UpdateClass::Cold,
            at: None,
        })),
    );
    let run = start_spec_run(&spec, &profile, assembly).unwrap();
    let lines = probe.lines();
    assert!(lines.iter().any(|line| line.starts_with("x:submit:ok:")), "{lines:?}");
    assert!(!lines.iter().any(|line| line.contains("ezsdr.value")), "no Vocabulary kind check applies to a component value: {lines:?}");
    let _ = run.finish_checked();

    // The scheduled origin: the same value, its class from the component's ParamDecl.
    let (mut spec, profile) = component_class_docs();
    add_schedule(
        &mut spec,
        "radio",
        10,
        serde_json::json!({
            "kind": "update_parameter",
            "target": { "kind": "component", "component": "c1" },
            "key": "test.gain",
            "value": true
        }),
    );
    let probe = Probe::new();
    let mut assembly = rig(Pacing::FreeRunning).assembly;
    assembly.registry = run_registry_classed();
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).declaring("radio/rx", 1, 1)));
    assembly.executors.insert(Ident::parse("exec").unwrap(), Box::new(ProbeExecutor::new("x", &probe)));
    let run = start_spec_run(&spec, &profile, assembly).unwrap();
    assert!(matches!(run.state(), RunState::Running {}));
    let lines = probe.lines();
    assert!(lines.iter().any(|line| line.starts_with("x:action:component c1")), "{lines:?}");
    assert!(!lines.iter().any(|line| line.contains("ezsdr.value")), "no Vocabulary kind check applies to a scheduled component value: {lines:?}");
    let _ = run.finish_checked();
}

#[test]
fn rs_36_a_dropped_stop_must_not_mask_a_dropped_abort() {
    let probe = Probe::new();
    let (mut spec, profile) = distinct_resource_docs(&["left", "right"]);
    spec["policies"] = serde_json::json!({ "failure": { "ezsdr.LINK_BACKPRESSURE": "stop" } });
    let mut assembly = rig(Pacing::FreeRunning).assembly;
    for (name, kind, severity) in [("left", "ezsdr.LINK_BACKPRESSURE", ezsdr_kernel::event::Severity::Warning), ("right", "ezsdr.STEP_LIVELOCK", ezsdr_kernel::event::Severity::Fatal)] {
        assembly.providers.insert(
            Ident::parse(name).unwrap(),
            Box::new(SteppedProvider::new(name, TestProvider::new(name, 2), &probe).emitting(kind, severity, 150).flooding(ezsdr_kernel::coordinator::EVENT_RING_DEPTH).with_wakeups(&[150])),
        );
    }
    let mut run = start_spec_run(&spec, &profile, assembly).unwrap();
    let _ = run.advance_to(TimePoint::new(run.now().domain(), 160));
    let manifest = run.finish_checked();
    eprintln!("transitions: {:?}; termination: {:?}", manifest.run.transitions, manifest.termination);
    for kind in ["ezsdr.LINK_BACKPRESSURE", "ezsdr.STEP_LIVELOCK"] {
        assert!(manifest.events.delivered.iter().any(|e| e.kind.as_str() == "ezsdr.EVENTS_DROPPED" && e.payload["kind"] == kind));
    }
    assert!(
        manifest.run.transitions.iter().any(|row| matches!(row.state, RunState::Stopping { mode: ezsdr_kernel::run::CleanupMode::Abort })),
        "a fatal STEP_LIVELOCK dropped behind a Stop must still abort"
    );
}

#[test]
fn rs_36_a_dropped_stop_body_stops_the_run_orderly() {
    // RS-36's `stop` half, on its own: `ezsdr.LINK_BACKPRESSURE` overridden to `stop`,
    // its body dropped by the full ring, and no other kind. Only the escalation flag
    // can end this Run, and it must end it **orderly** — the sibling test above keeps
    // the fatal kind, whose flag is the one that wins (KC-31, RS-26).
    let probe = Probe::new();
    let (mut spec, profile) = distinct_resource_docs(&["left"]);
    spec["policies"] = serde_json::json!({ "failure": { "ezsdr.LINK_BACKPRESSURE": "stop" } });
    let mut assembly = rig(Pacing::FreeRunning).assembly;
    assembly.providers.insert(
        Ident::parse("left").unwrap(),
        Box::new(
            SteppedProvider::new("left", TestProvider::new("left", 2), &probe)
                .emitting("ezsdr.LINK_BACKPRESSURE", ezsdr_kernel::event::Severity::Warning, 150)
                .flooding(ezsdr_kernel::coordinator::EVENT_RING_DEPTH)
                .with_wakeups(&[150]),
        ),
    );
    let mut run = start_spec_run(&spec, &profile, assembly).unwrap();
    let root = run.now().domain();
    let _ = run.advance_to(TimePoint::new(root, 160));
    let manifest = run.finish_checked();
    let kind = ezsdr_kernel::event::EventKind::parse("ezsdr.LINK_BACKPRESSURE").unwrap();
    assert!(!manifest.events.delivered.iter().any(|event| event.kind == kind), "the body was dropped, so no test can react to it: {:?}", manifest.events.delivered);
    assert!(
        manifest.events.delivered.iter().any(|event| event.kind.as_str() == "ezsdr.EVENTS_DROPPED"
            && event.payload == serde_json::json!({ "kind": "ezsdr.LINK_BACKPRESSURE", "count": 1 })
            && event.source == EventSource::Kernel {}),
        "{:?}",
        manifest.events.delivered
    );
    assert!(matches!(manifest.termination.reason, Termination::Stopped {
        cause: StopCause::Policy { event: event_kind }
    } if event_kind == kind));
    assert!(
        manifest.run.transitions.iter().any(|row| matches!(row.state, RunState::Stopping { mode: ezsdr_kernel::run::CleanupMode::Orderly })),
        "a dropped body of a stop-reacting kind ends the Run orderly: {:?}",
        manifest.run.transitions
    );
}

/// A resource `radio`, two outputs `rec` and `rec2`, and a component `c1` on `exec`.
fn role_docs() -> (serde_json::Value, serde_json::Value) {
    let (mut spec, mut profile) = output_docs();
    let (executor_spec, executor_profile) = executor_docs();
    let mut second = spec["outputs"][0].clone();
    second["id"] = serde_json::json!("rec2");
    spec["outputs"].as_array_mut().unwrap().push(second);
    spec["graph"] = executor_spec["graph"].clone();
    profile["bindings"]["rec2"] = profile["bindings"]["rec"].clone();
    profile["bindings"]["exec"] = executor_profile["bindings"]["exec"].clone();
    profile["placements"]["islands"] = executor_profile["placements"]["islands"].clone();
    let mut second_link = profile["placements"]["links"][0].clone();
    second_link["to"]["component"] = serde_json::json!("rec2");
    profile["placements"]["links"].as_array_mut().unwrap().push(second_link);
    (spec, profile)
}

/// Runs `role_docs` with an Executor that submits a `Command` to `target`.
fn command_to(target: Target) -> Vec<String> {
    let (spec, profile) = role_docs();
    let probe = Probe::new();
    let mut assembly = output_assembly(&probe, None, None);
    assembly.sinks.insert(Ident::parse("rec2").unwrap(), Box::new(RecordingSink::new("rec2", &probe)));
    assembly.executors.insert(
        Ident::parse("exec").unwrap(),
        Box::new(ProbeExecutor::new("x", &probe).submitting(Action::Command { target, verb: Ident::parse("probe").unwrap(), params: BTreeMap::new(), at: None })),
    );
    let run = start_spec_run(&spec, &profile, assembly).unwrap();
    assert!(run.finish_checked().termination.cleanup_failures.is_empty());
    probe.lines()
}

#[test]
fn kc_08_each_instance_is_given_its_source_root() {
    // A Provider's node, a Sink's output, an Executor's Island (KC-8); each Sink its
    // own output, though both are one Module.
    let lines = command_to(Target::Output { output: Ident::parse("rec").unwrap() });
    let mut sources = lines.iter().filter(|line| line.starts_with("source:")).collect::<Vec<_>>();
    sources.sort();
    assert_eq!(sources, ["source:p:node radio", "source:rec2:output rec2", "source:rec:output rec", "source:x:island 0"]);
}

#[test]
fn kc_08_each_instance_s_event_view_is_rooted_through_the_coordinator() {
    // KC-8, MA-5 (item 3; Sol review P1-1): every role's `ctx.events` is a root-bound
    // **view**, not the collector. Through a real Run each instance's view accepts its
    // own root and refuses another instance's source, `kernel` and `unforeseen`;
    // reverting `pipeline.rs`'s `collector.view(source)` to the collector turns every
    // one of these lines false, so the coordinator's own wiring is what is pinned.
    let (spec, profile) = role_docs();
    let probe = Probe::new();
    let mut assembly = output_assembly(&probe, None, None);
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).checking_event_view()));
    assembly.sinks.insert(Ident::parse("rec").unwrap(), Box::new(RecordingSink::new("rec", &probe).checking_event_view()));
    assembly.sinks.insert(Ident::parse("rec2").unwrap(), Box::new(RecordingSink::new("rec2", &probe).checking_event_view()));
    assembly.executors.insert(Ident::parse("exec").unwrap(), Box::new(ProbeExecutor::new("x", &probe).checking_event_views()));
    let run = start_spec_run(&spec, &profile, assembly).unwrap();
    assert!(run.finish_checked().termination.cleanup_failures.is_empty());
    let lines = probe.lines();
    for name in ["p", "rec", "rec2", "x"] {
        let line = format!("events:{name}:own=true:other=true:kernel=true:unforeseen=true");
        assert!(lines.iter().any(|observed| observed == &line), "{name}'s view is not rooted through the coordinator: {lines:?}");
    }
}

#[test]
fn kc_45_two_instances_writing_one_section_name_both_keep_it() {
    // Decision 2: a section is filed under its writer's source, so one name written by
    // two instances of one Module is two sections, not an overwrite.
    let (spec, profile) = distinct_resource_docs(&["a", "b"]);
    let mut assembly = rig(Pacing::FreeRunning).assembly;
    for (name, samples) in [("a", 1), ("b", 2)] {
        assembly.providers.insert(Ident::parse(name).unwrap(), Box::new(TestProvider::new(name, 2).with_section("ezsdr.test.provider.details", serde_json::json!({ "samples": samples }))));
    }
    let manifest = start_spec_run(&spec, &profile, assembly).unwrap().finish_checked();
    assert_eq!(manifest.sections.len(), 2, "{:?}", manifest.sections);
    for (name, samples) in [("a", 1), ("b", 2)] {
        let content = manifest.section(&support::node_source(name), "ezsdr.test.provider.details");
        assert_eq!(content, Some(&serde_json::json!({ "samples": samples })), "{name}");
    }
}

#[test]
fn rs_14_a_capture_reaches_the_output_it_names_with_two_sinks_bound() {
    // #54: the verb compiles to the target it names; the Kernel chooses no recorder, so
    // with two bound a capture to `{output: rec2}` reaches `rec2` and only it.
    let (_, mut profile) = output_docs();
    for name in ["rec", "rec2"] {
        profile["bindings"][name] = serde_json::json!({
            "module": { "id": "ezsdr.test.sink", "version": { "major": 1, "minor": 0, "patch": 0 } },
            "feed": { "port": { "component": "radio", "port": "rx" }, "policy": "drop_oldest", "capacity": 4 }
        });
    }
    let mut second_link = profile["placements"]["links"][0].clone();
    second_link["to"]["component"] = serde_json::json!("rec2");
    profile["placements"]["links"].as_array_mut().unwrap().push(second_link);
    let probe = Probe::new();
    let mut assembly = output_assembly(&probe, None, None);
    assembly.registry = run_registry_classed();
    assembly.sinks.insert(Ident::parse("rec2").unwrap(), Box::new(RecordingSink::new("rec2", &probe)));
    let mut run = connect(&profile, &Default::default(), assembly, Lease::attached()).expect("Session profile is valid");
    assert!(matches!(run.state(), RunState::Running {}), "{:?}", run.state());
    let entry = run
        .submit(
            SessionAction::Vocabulary {
                ns: ns("test"),
                verb: Ident::parse("capture").unwrap(),
                target: Target::Output { output: Ident::parse("rec2").unwrap() },
                at: None,
                params: BTreeMap::from([(Key::parse("test.capture").unwrap(), Value::from(true))]),
            },
            None,
        )
        .unwrap();
    assert!(matches!(entry.outcome, Outcome::Admitted { .. }), "{entry:?}");
    let _ = run.finish_checked();
    let lines = probe.lines();
    assert!(lines.iter().any(|line| line == "rec2:action:output rec2"), "{lines:?}");
    assert!(!lines.iter().any(|line| line.starts_with("rec:action:") || line.starts_with("p:action:")), "{lines:?}");
}

#[test]
fn kc_24_a_module_actions_target_is_resolved_before_its_update_class() {
    // KC-24: step 3 (the target) precedes step 5 (the update class), so one Module
    // Action that breaks both is refused as a target.
    let (spec, profile) = executor_docs();
    let probe = Probe::new();
    let mut assembly = rig(Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe)));
    assembly.executors.insert(
        Ident::parse("exec").unwrap(),
        Box::new(ProbeExecutor::new("x", &probe).submitting(Action::UpdateParameter {
            target: support::target("nothing"),
            key: Key::parse("test.gain").unwrap(),
            value: Value::from(1),
            // Not `test.gain`'s declared class, so step 5 has a refusal to raise if it
            // ever runs before step 3.
            class: UpdateClass::Cold,
            at: None,
        })),
    );
    let run = start_spec_run(&spec, &profile, assembly).unwrap();
    assert!(!run.effective()[&Ident::parse("radio").unwrap()].contains_key(&Key::parse("test.gain").unwrap()));
    let manifest = run.finish_checked();
    let lines = probe.lines();
    assert!(lines.iter().any(|line| line.starts_with("x:submit:err:ezsdr.target:KC-23:")), "{lines:?}");
    assert!(!lines.iter().any(|line| line.contains("ezsdr.update_class")), "step 5 is never reached: {lines:?}");
    assert!(!lines.iter().any(|line| line.starts_with("p:action:")), "{lines:?}");
    assert!(manifest.termination.cleanup_failures.is_empty());
}

/// An `Emit` as a Module submits one (KC-24).
fn emit_action(payload: serde_json::Value) -> Action {
    Action::Emit {
        target: support::target("radio"),
        event: ezsdr_kernel::event::Event {
            source: support::node_source("radio"),
            time: TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0),
            severity: ezsdr_kernel::event::Severity::Info,
            kind: ezsdr_kernel::event::EventKind::parse("test.custom").expect("a test kind"),
            payload,
        },
    }
}

#[test]
fn kc_24_a_module_emit_is_dispatched_to_its_target() {
    // Item 3 does not change `Emit`'s route: admission, then dispatch to the target
    // Module — never a direct collector emission, and no delivered path of its own.
    // The target here is the Provider double, which takes the Action and records it.
    let (spec, profile) = executor_docs();
    let probe = Probe::new();
    let mut assembly = rig(Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe)));
    assembly.executors.insert(Ident::parse("exec").unwrap(), Box::new(ProbeExecutor::new("x", &probe).submitting(emit_action(serde_json::json!({ "\u{540d}\u{524d}": "\u{65e5}\u{672c}\u{8a9e}" })))));
    let run = start_spec_run(&spec, &profile, assembly).unwrap();
    let lines = probe.lines();
    assert!(lines.iter().any(|line| line.starts_with("p:action:Emit:radio@")), "{lines:?}");
    assert!(run.finish_checked().termination.cleanup_failures.is_empty());
}

#[test]
fn kc_24_a_module_update_is_checked_with_its_proposed_value() {
    // KC-24 step 12 (review P0-1): every `UpdateParameter` reaches the registered
    // checks with `proposed = { fragment: { key: value } }` — a Module's update
    // included, which step 10 does not coerce. `test.limits` judges `test.grid` only
    // where `test.flag` is true, and the update below is refused exactly because the
    // check sees the proposed 25 through the gate the configuration holds. With
    // `proposed` empty the check has nothing to judge and the update is admitted,
    // which is the safety regression this test pins.
    let (spec, mut profile) = executor_docs();
    profile["environment"] = serde_json::json!({ "test.limits": { "max_grid": 20.0, "gate": "test.flag" } });
    let probe = Probe::new();
    let mut assembly = rig(Pacing::FreeRunning).assembly;
    assembly.registry = run_registry_classed();
    assembly.checks = run_checks(true);
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2).with_effective("test.flag", Value::from(true)), &probe)));
    assembly.executors.insert(
        Ident::parse("exec").unwrap(),
        Box::new(ProbeExecutor::new("x", &probe).submitting(Action::UpdateParameter {
            target: support::target("radio"),
            key: Key::parse("test.grid").unwrap(),
            value: Value::num(25.0).unwrap(),
            class: ezsdr_kernel::module_api::UpdateClass::Cold,
            at: None,
        })),
    );
    let run = start_spec_run(&spec, &profile, assembly).unwrap();
    let lines = probe.lines();
    assert!(lines.iter().any(|line| line.starts_with("x:submit:err:test.limits:")), "the check sees the proposed value: {lines:?}");
    assert!(!lines.iter().any(|line| line.starts_with("p:action:")), "{lines:?}");
    let manifest = run.finish_checked();
    assert!(manifest.termination.cleanup_failures.is_empty());
}

/// A check that refuses the proposed `test.grid = 25.0` and nothing else. It is the
/// direct witness of KC-24 step 12's `proposed` (review P0-1): a value no
/// configuration holds, so only an update that carries it can reach its refusal.
struct RefuseProposedGrid(ezsdr_kernel::spec::Namespace);

impl RefuseProposedGrid {
    fn new() -> RefuseProposedGrid {
        RefuseProposedGrid(ns("test.proposed"))
    }
}

impl ezsdr_kernel::binding::AdmissionCheck for RefuseProposedGrid {
    fn section(&self) -> &ezsdr_kernel::spec::Namespace {
        &self.0
    }
    fn stages(&self) -> &[ezsdr_kernel::binding::CheckStage] {
        &[ezsdr_kernel::binding::CheckStage::Runtime]
    }
    fn check(
        &self,
        _section: &serde_json::Value,
        _effective: &BTreeMap<Ident, BTreeMap<Key, Value>>,
        proposed: &BTreeMap<Ident, BTreeMap<Key, Value>>,
        _stage: ezsdr_kernel::binding::CheckStage,
    ) -> Vec<ezsdr_kernel::binding::Violation> {
        proposed
            .iter()
            .filter_map(|(fragment, values)| {
                let grid = values.get(&Key::parse("test.grid").unwrap())?;
                (*grid == Value::num(25.0).unwrap()).then(|| ezsdr_kernel::binding::Violation {
                    check: ns("test.proposed"),
                    key: Some(Key::parse("test.grid").unwrap()),
                    requested: Some(grid.clone()),
                    reason: format!("{fragment}: test.grid 25.0 is refused"),
                })
            })
            .collect()
    }
}

#[test]
fn kc_24_a_check_refuses_a_non_provider_update_with_its_proposed_value() {
    // The same step 12 for a non-Provider fragment (review P0-1): an update to a
    // component reaches the registered checks with its proposed value too. The value
    // is one no configuration holds, so only a check that receives `proposed` can
    // refuse it, and the component declares `test.grid` so the class check passes.
    let (mut spec, mut profile) = executor_docs();
    spec["graph"]["components"]["c1"]["params"] = serde_json::json!([
        { "key": "test.grid", "schema": {"type": "number"}, "update_class": "cold", "default": 2.0 }
    ]);
    profile["environment"] = serde_json::json!({ "test.proposed": {} });
    let probe = Probe::new();
    let mut assembly = rig(Pacing::FreeRunning).assembly;
    assembly.registry = run_registry_classed();
    assembly.checks.register(std::sync::Arc::new(RefuseProposedGrid::new()));
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe)));
    assembly.executors.insert(
        Ident::parse("exec").unwrap(),
        Box::new(ProbeExecutor::new("x", &probe).submitting(Action::UpdateParameter {
            target: Target::Component { component: Ident::parse("c1").unwrap() },
            key: Key::parse("test.grid").unwrap(),
            value: Value::num(25.0).unwrap(),
            class: ezsdr_kernel::module_api::UpdateClass::Cold,
            at: None,
        })),
    );
    let run = start_spec_run(&spec, &profile, assembly).unwrap();
    let lines = probe.lines();
    assert!(lines.iter().any(|line| line.starts_with("x:submit:err:test.proposed:")), "the check sees the proposed value on a component fragment: {lines:?}");
    assert!(!lines.iter().any(|line| line.starts_with("x:action:")), "{lines:?}");
    let manifest = run.finish_checked();
    assert!(manifest.termination.cleanup_failures.is_empty());
}

/// A check that refuses `test.grid` with a violation naming another key, `test.count`:
/// the Kernel records the Action's own value for the key the violation names, and the
/// Action asked nothing of `test.count` (KC-24 step 12).
struct RefuseOtherKey(ezsdr_kernel::spec::Namespace);

impl RefuseOtherKey {
    fn new() -> RefuseOtherKey {
        RefuseOtherKey(ns("test.other"))
    }
}

impl ezsdr_kernel::binding::AdmissionCheck for RefuseOtherKey {
    fn section(&self) -> &ezsdr_kernel::spec::Namespace {
        &self.0
    }
    fn stages(&self) -> &[ezsdr_kernel::binding::CheckStage] {
        &[ezsdr_kernel::binding::CheckStage::Runtime]
    }
    fn check(
        &self,
        _section: &serde_json::Value,
        _effective: &BTreeMap<Ident, BTreeMap<Key, Value>>,
        proposed: &BTreeMap<Ident, BTreeMap<Key, Value>>,
        _stage: ezsdr_kernel::binding::CheckStage,
    ) -> Vec<ezsdr_kernel::binding::Violation> {
        proposed
            .values()
            .any(|values| values.contains_key(&Key::parse("test.grid").unwrap()))
            .then(|| ezsdr_kernel::binding::Violation {
                check: ns("test.other"),
                key: Some(Key::parse("test.count").unwrap()),
                requested: Some(Value::num(1.0).unwrap()),
                reason: "test.other: test.count is refused".to_owned(),
            })
            .into_iter()
            .collect()
    }
}

#[test]
fn kc_24_a_registered_check_s_violation_records_what_the_action_asked() {
    // KC-24 step 12: a registered check sees `proposed` after step 10's coercion, so
    // `test.limits` judges the snapped 20.0 — but the violation's `requested` is what the
    // Action asked (19.5), not the value the check saw. A violation of another key records
    // none: the Action asked nothing of it.
    let mut profile = profile_one();
    profile["environment"] = serde_json::json!({ "test.limits": { "max_grid": 19.9 } });
    let mut assembly = rig(Pacing::FreeRunning).assembly;
    assembly.registry = run_registry_classed();
    assembly.checks = run_checks(true);
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(TestProvider::new("radio", 2).with_grid(20.0)));
    let mut run = connect(&profile, &Default::default(), assembly, Lease::attached()).expect("valid Session entry");
    let entry = run.submit(SessionAction::SetParameter { target: support::target("radio"), key: Key::parse("test.grid").unwrap(), value: Value::num(19.5).unwrap() }, None).unwrap();
    let Outcome::Rejected { violations } = &entry.outcome else { panic!("the coerced value is outside the ceiling: {:?}", entry.outcome) };
    assert_eq!(violations.len(), 1, "the ceiling is the only refusal: {violations:?}");
    let violation = &violations[0];
    assert_eq!(violation.check, ns("test.limits"));
    assert_eq!(violation.key, Some(Key::parse("test.grid").unwrap()));
    assert_eq!(violation.requested, Some(Value::num(19.5).unwrap()), "the Action's value, not the coerced 20.0");
    assert!(violation.reason.contains("test.grid 20 exceeds the declared ceiling 19.9"), "the check judged the coerced value: {violation:?}");
    let _ = run.finish_checked();

    let mut profile = profile_one();
    profile["environment"] = serde_json::json!({ "test.other": {} });
    let mut assembly = rig(Pacing::FreeRunning).assembly;
    assembly.registry = run_registry_classed();
    assembly.checks = run_checks(false);
    assembly.checks.register(Arc::new(RefuseOtherKey::new()));
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(TestProvider::new("radio", 2)));
    let mut run = connect(&profile, &Default::default(), assembly, Lease::attached()).expect("valid Session entry");
    let entry = run.submit(SessionAction::SetParameter { target: support::target("radio"), key: Key::parse("test.grid").unwrap(), value: Value::num(25.0).unwrap() }, None).unwrap();
    let Outcome::Rejected { violations } = &entry.outcome else { panic!("the check refuses the update: {:?}", entry.outcome) };
    assert_eq!(violations.len(), 1, "the check's refusal is the only one: {violations:?}");
    assert_eq!(violations[0].check, ns("test.other"));
    assert_eq!(violations[0].key, Some(Key::parse("test.count").unwrap()));
    assert_eq!(violations[0].requested, None, "the Action asked nothing of test.count");
    let _ = run.finish_checked();
}

/// A Provider double whose `coerce` applies `test.gain` as a `List` — a value outside
/// that `Num` declaration — and every other request as itself. It is how a test
/// reaches KC-24's step 11, the one refusal a conforming Provider would not produce,
/// and step 11 applies to the Session and schedule origins only: a Module's Action is
/// not coerced at all (KC-24 step 8's guard,
/// `kc_24_a_module_update_is_not_coerced_by_the_kernel`). Its instance is stepped, so
/// the coordinator holds no slot for it and no thread is involved.
struct KindWrongProvider {
    inner: TestProvider,
    /// The stream whose SampleClock this Provider declares at `prepare`, so a
    /// schedule entry can resolve its `SpecTime` against this instance (SB-16).
    declare_clock: Option<String>,
}

impl KindWrongProvider {
    fn new(inner: TestProvider) -> KindWrongProvider {
        KindWrongProvider { inner, declare_clock: None }
    }

    fn declaring(mut self, stream: &str) -> KindWrongProvider {
        self.declare_clock = Some(stream.to_owned());
        self
    }
}

impl ezsdr_kernel::module_api::Provider for KindWrongProvider {
    fn instance(&self) -> &ezsdr_kernel::module_api::ProviderInstance {
        self.inner.instance()
    }

    fn coerce(&self, request: &ezsdr_kernel::module_api::Requested) -> Result<ezsdr_kernel::module_api::CoerceReport, ezsdr_kernel::module_api::ModuleError> {
        use ezsdr_kernel::spec::{Coercion, Constraint, Scalar};
        let mut report = ezsdr_kernel::module_api::CoerceReport::default();
        for (key, constraint) in &request.constraints {
            let Constraint::Eq { value } = constraint else {
                continue;
            };
            let value = Value::Scalar(value.clone());
            if key.as_str() == "test.gain" || key.as_str() == "test.capture" {
                // A `List` where the declaration is a `Num`: step 11's refusal.
                let applied = Value::List(vec![Scalar::from(3)]);
                report.applied.insert(key.clone(), applied.clone());
                report.coercions.push(Coercion { key: key.clone(), requested: value, applied, reason: "wrapped the number in a list".to_owned() });
            } else {
                report.applied.insert(key.clone(), value);
            }
        }
        Ok(report)
    }

    fn prepare(&mut self, f: &ezsdr_kernel::plan::Fragment, ctx: ezsdr_kernel::module_api::PrepareContext) -> Result<ezsdr_kernel::plan::PrepareReport, ezsdr_kernel::module_api::ModuleError> {
        if let Some(stream) = &self.declare_clock {
            let stream_id = ezsdr_kernel::id::ResourceId::parse(stream).map_err(|e| ezsdr_kernel::module_api::ModuleError::rejected(format!("test: {e}")))?;
            let ratio = ezsdr_kernel::time::Rational::new(1_000_000_000, 1).map_err(|e| ezsdr_kernel::module_api::ModuleError::rejected(format!("test: {e}")))?;
            ctx.clocks.declare_sample_clock(stream_id, ctx.time.primary_root(), ratio).map_err(|e| ezsdr_kernel::module_api::ModuleError::rejected(format!("test: {e}")))?;
        }
        ezsdr_kernel::module_api::Provider::prepare(&mut self.inner, f, ctx)
    }

    fn arm(&mut self) -> Result<(), ezsdr_kernel::module_api::ModuleError> {
        Ok(())
    }

    fn start(&mut self, _at: Option<TimePoint>) -> Result<(), ezsdr_kernel::module_api::ModuleError> {
        Ok(())
    }

    fn stop(&mut self, _mode: ezsdr_kernel::module_api::StopMode) -> Result<(), ezsdr_kernel::module_api::ModuleError> {
        Ok(())
    }

    fn cleanup(&mut self) {}
}

#[test]
fn sb_02a_a_session_update_reads_a_numbered_declaration_s_class() {
    // SB-2a at KC-24: a Session's `SetParameter` names a concrete key, so step 5 reads
    // its update class from the numbered declaration that covers it, and step 6's
    // kind refusal names that same concrete key. The declaration's own spelling is no
    // key at all, exactly as another undeclared key is none.
    let mut run = session_with_provider(Box::new(TestProvider::new("radio", 2)));
    let set = |key_name: &str, value: Value| SessionAction::SetParameter { target: support::target("radio"), key: Key::parse(key_name).unwrap(), value };
    // `test.item` declares `hardware_timed` (SB-2a's numbered declaration), so an Int
    // value is admitted and recorded under the concrete key (KC-25).
    let admitted = run.submit(set("test.item0", Value::from(3)), None).unwrap();
    assert!(matches!(admitted.outcome, Outcome::Admitted { .. }), "{admitted:?}");
    assert_eq!(run.effective()[&Ident::parse("radio").unwrap()].get(&Key::parse("test.item0").unwrap()), Some(&Value::from(3)), "the admitted update is recorded under the concrete key");
    // The spelling `test.item` has no declaration to give a class.
    let spelled = run.submit(set("test.item", Value::from(3)), None).unwrap();
    assert!(
        matches!(spelled.outcome, Outcome::Rejected { ref violations }
            if violations.iter().any(|v| v.check == ns("ezsdr.update_class")
                && v.reason.contains("declares no update class"))),
        "the declaration's own spelling names no declaration: {spelled:?}"
    );
    // A `Bool` into the numbered `Int` declaration is step 6's refusal, and the words
    // it hands back name the key the caller wrote.
    let wrong_kind = run.submit(set("test.item1", Value::from(true)), None).unwrap();
    assert!(
        matches!(wrong_kind.outcome, Outcome::Rejected { ref violations }
            if violations.iter().any(|v| v.check == ns("ezsdr.value")
                && v.key == Some(Key::parse("test.item1").unwrap())
                && v.reason.contains("test.item1")
                && v.reason.contains("wanted Int"))),
        "step 6 names the concrete key: {wrong_kind:?}"
    );
    let _ = run.finish_checked();
}

#[test]
fn kc_24_an_admission_refusal_leaves_no_configuration_and_no_action_id() {
    // Pre-freeze item 3: a coerced value that fails step 11 is refused; the refused
    // update never reaches the configuration (KC-25), and a refused Action consumes no
    // id, so the ActionIds an admitted neighbour receives (KC-24's dispatch paragraph)
    // are the ones the refusals did not take.
    let mut assembly = rig(Pacing::FreeRunning).assembly;
    assembly.registry = run_registry_classed();
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(KindWrongProvider::new(TestProvider::new("radio", 2).stepped())));
    let mut run = connect(&profile_one(), &Default::default(), assembly, Lease::attached()).expect("valid Session entry");
    let set = |key_name: &str, value: Value| SessionAction::SetParameter { target: support::target("radio"), key: Key::parse(key_name).unwrap(), value };

    let refused = run.submit(set("test.gain", Value::num(3.0).unwrap()), None).unwrap();
    assert!(
        matches!(refused.outcome, Outcome::Rejected { ref violations }
            if violations.iter().any(|v| v.check == ns("ezsdr.value")
                && v.key == Some(Key::parse("test.gain").unwrap())
                && v.requested == Some(Value::num(3.0).unwrap())
                && v.reason.contains("SB-2"))),
        "step 11 refusals name the key and transcribe the request: {refused:?}"
    );
    assert_eq!(refused.seq, 0, "a refused Action is still logged (RS-15)");

    // A key whose declaration fixes it — `test.count`'s `KeyDecl` states no class — is
    // step 5's declaration gap, refused at admission too and consuming no ActionId.
    let fixed = run.submit(set("test.count", Value::from(1)), None).unwrap();
    assert!(
        matches!(fixed.outcome, Outcome::Rejected { ref violations }
            if violations.iter().any(|v| v.check == ns("ezsdr.update_class")
                && v.key == Some(Key::parse("test.count").unwrap())
                && v.reason.contains("declares no update class"))),
        "step 5 refuses a fixed declaration: {fixed:?}"
    );
    assert_eq!(fixed.seq, 1);

    // A value of the wrong kind for its declaration is step 6, before the Provider's
    // `coerce`: a `Bool` into `test.gain`'s `Num` is refused as `ezsdr.value` with the
    // declaration's own words, not as whatever a coercion would have made of it.
    let wrong_kind = run.submit(set("test.gain", Value::from(true)), None).unwrap();
    assert!(
        matches!(wrong_kind.outcome, Outcome::Rejected { ref violations }
            if violations.iter().any(|v| v.check == ns("ezsdr.value")
                && v.key == Some(Key::parse("test.gain").unwrap())
                && v.reason.contains("wanted Num")
                && v.reason.contains("found Bool"))),
        "step 6 checks the requested value against its KeyDecl: {wrong_kind:?}"
    );
    assert_eq!(wrong_kind.seq, 2);

    // A valid neighbour is admitted, and its ActionId is 1: the refusals before it
    // consumed none.
    let admitted = run.submit(set("test.flag", Value::from(true)), None).unwrap();
    assert!(
        matches!(admitted.outcome, Outcome::Admitted { ref dispatched, .. }
            if dispatched == &[ezsdr_kernel::event::ActionId(1)]),
        "the refusals consumed no ActionId: {admitted:?}"
    );
    assert_eq!(run.effective()[&Ident::parse("radio").unwrap()].get(&Key::parse("test.flag").unwrap()), Some(&Value::from(true)), "the admitted update is recorded (KC-25)");

    // And the refused key once more: the configuration holds no value for it, so the
    // refusal did not write one on its way out.
    let refused_again = run.submit(set("test.gain", Value::num(3.0).unwrap()), None).unwrap();
    assert!(matches!(refused_again.outcome, Outcome::Rejected { .. }), "{refused_again:?}");
    assert_eq!(run.effective()[&Ident::parse("radio").unwrap()].get(&Key::parse("test.gain").unwrap()), None, "KC-25: the refused update is not applied");
    let manifest = run.finish_checked();
    assert_eq!(manifest.action_log.len(), 5);
    assert!(manifest.termination.cleanup_failures.is_empty());
}

#[test]
fn kc_24_a_update_is_refused_by_the_same_check_at_a_session_and_at_arm() {
    // Pre-freeze item 3: the same invalid update is refused by the same KC-24 check
    // whatever its origin. KC-28's front matter and KC-17's schedule resolution stand
    // outside the order, so the Session reports a `Rejected` entry and the schedule
    // reports `Failed { arm }` — but the refusal itself, the key it names and the
    // value it transcribes are one rule's.
    let set = |key_name: &str, value: Value| SessionAction::SetParameter { target: support::target("radio"), key: Key::parse(key_name).unwrap(), value };
    let submitted = {
        let mut assembly = rig(Pacing::FreeRunning).assembly;
        assembly.registry = run_registry_classed();
        assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(KindWrongProvider::new(TestProvider::new("radio", 2).stepped())));
        let mut run = connect(&profile_one(), &Default::default(), assembly, Lease::attached()).expect("valid Session entry");
        // A valid value first, so the refusal below has a configuration to leave
        // alone rather than an absent key to agree with (KC-25).
        let good = run.submit(set("test.flag", Value::from(true)), None).unwrap();
        assert!(matches!(good.outcome, Outcome::Admitted { .. }), "{good:?}");
        let entry = run.submit(set("test.gain", Value::num(3.0).unwrap()), None).unwrap();
        assert_eq!(entry.seq, 1);
        // The same rule through a Vocabulary verb: `capture` sets `test.capture`, whose
        // declaration the classed fixture states (`Bool`, `BlockBoundary`), and the
        // Provider's coercion moves it outside that declaration's kind. The verb states
        // no class of its own (RS-14, item 3).
        let verb = run
            .submit(
                SessionAction::Vocabulary {
                    ns: ns("test"),
                    verb: Ident::parse("capture").unwrap(),
                    target: support::target("radio"),
                    at: None,
                    params: [(Key::parse("test.capture").unwrap(), Value::from(true))].into_iter().collect(),
                },
                None,
            )
            .unwrap();
        assert_eq!(verb.seq, 2);
        assert!(
            matches!(verb.outcome, Outcome::Rejected { ref violations }
                if violations.iter().any(|v| v.check == ns("ezsdr.value")
                    && v.key == Some(Key::parse("test.capture").unwrap())
                    && v.reason.contains("SB-2"))),
            "the same check refuses a Vocabulary verb's update: {verb:?}"
        );
        assert_eq!(run.effective()[&Ident::parse("radio").unwrap()].get(&Key::parse("test.gain").unwrap()), None, "the refused update did not write the configuration (KC-25)");
        let _ = run.finish_checked();
        match entry.outcome {
            Outcome::Rejected { violations } => violations,
            other => panic!("a coerced value outside its declaration is refused: {other:?}"),
        }
    };

    let mut spec = spec_one();
    spec["schedule"] = serde_json::json!([{
        "at": { "clock": "radio", "offset_ticks": 0 },
        "action": {
            "kind": "update_parameter",
            "target": serde_json::to_value(support::target("radio")).unwrap(),
            "key": "test.gain",
            "value": 3.0
        }
    }]);
    let mut assembly = rig(Pacing::FreeRunning).assembly;
    assembly.registry = run_registry_classed();
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(KindWrongProvider::new(TestProvider::new("radio", 2).stepped()).declaring("radio")));
    let run = start_spec_run(&spec, &profile_one(), assembly).expect("entry creates a Run");
    assert_eq!(run.effective()[&Ident::parse("radio").unwrap()].get(&Key::parse("test.gain").unwrap()), None, "a refused scheduled update is not recorded (KC-25)");
    let reason = failure(&run.finish_checked()).to_owned();
    assert!(reason.starts_with("KC-17: entry 0"), "{reason}");
    assert!(reason.contains("ezsdr.value"), "{reason}");
    assert!(reason.contains("SB-2"), "{reason}");

    let submitted = &submitted[0];
    assert_eq!(submitted.check, ns("ezsdr.value"), "{submitted:?}");
    assert_eq!(submitted.key, Some(Key::parse("test.gain").unwrap()));
    assert_eq!(submitted.requested, Some(Value::num(3.0).unwrap()));
    assert!(submitted.reason.contains("SB-2"), "{submitted:?}");
}

#[test]
fn kc_23_a_target_reaches_the_instance_of_its_role() {
    let resource = |name: &str| Target::Resource { resource: Ident::parse(name).unwrap(), path: String::new() };
    let output = |name: &str| Target::Output { output: Ident::parse(name).unwrap() };
    let component = |name: &str| Target::Component { component: Ident::parse(name).unwrap() };
    let received = |lines: &[String], prefix: &str| lines.iter().filter(|line| line.starts_with(prefix)).count();
    // Each role reaches its own instance, and only that one; with two Sinks bound,
    // `{output: rec2}` reaches `rec2` (#54).
    for (target, receiver) in
        [(resource("radio"), "p:action:Command:"), (output("rec2"), "rec2:action:output rec2"), (output("rec"), "rec:action:output rec"), (component("c1"), "x:action:component c1")]
    {
        let lines = command_to(target.clone());
        let all = ["p:action:", "rec:action:", "rec2:action:", "x:action:"].iter().map(|prefix| received(&lines, prefix)).sum::<usize>();
        assert_eq!((received(&lines, receiver), all), (1, 1), "{target}: {lines:?}");
    }
    // A name the Spec declares in another role, or not at all, is refused.
    for target in [output("radio"), component("radio"), resource("rec"), component("rec"), resource("c1"), output("c1"), output("exec"), resource("missing"), output("missing"), component("missing")] {
        let lines = command_to(target.clone());
        assert!(lines.iter().any(|line| line.starts_with("x:submit:err:ezsdr.target:KC-23:")), "{target}: {lines:?}");
        assert!(!lines.iter().any(|line| line.contains(":action:")), "{target}: {lines:?}");
    }
}

#[test]
fn kc_24_a_module_action_with_a_non_ascii_map_key_is_admitted_and_the_manifest_reads_back() {
    let value = Value::Map(BTreeMap::from([("日本語".to_owned(), Scalar::from(1))]));
    let target = support::target("radio");
    let key = Key::parse("test.gain").unwrap();
    let (bytes, waveform) = input_ref();
    let root = rig(Pacing::FreeRunning).root;
    let actions = [
        Action::UpdateParameter { target: target.clone(), key: key.clone(), value: value.clone(), class: UpdateClass::HardwareTimed, at: None },
        Action::Command { target: target.clone(), verb: Ident::parse("probe").unwrap(), params: BTreeMap::from([(key.clone(), value.clone())]), at: None },
        Action::TxBurst {
            target: support::target("radio/tx"),
            waveform: waveform.clone(),
            repeat: false,
            at: ezsdr_kernel::time::AbsoluteDeadline::new(TimePoint::new(root, 10)),
            requested_at: None,
            late_policy: ezsdr_kernel::stream::LatePolicy::SendAsapAndFlag,
            metadata: BTreeMap::from([(key.clone(), value.clone())]),
        },
    ];
    for (action, dispatched) in actions.into_iter().zip([None, Some("p:action:Command:"), Some("p:action:TxBurst:")]) {
        let (mut spec, profile) = executor_docs();
        spec["inputs"] = serde_json::to_value([&waveform]).unwrap();
        let probe = Probe::new();
        let mut assembly = rig(Pacing::FreeRunning).assembly;
        assembly.inputs.insert(waveform.hash.clone(), bytes.clone());
        assembly
            .providers
            .insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).declaring("radio/tx", 1, 1).registering_at_arm("radio/tx")));
        assembly.executors.insert(Ident::parse("exec").unwrap(), Box::new(ProbeExecutor::new("x", &probe).submitting(action)));
        let run = start_spec_run(&spec, &profile, assembly).unwrap();
        let lines = probe.lines();
        match dispatched {
            Some(prefix) => assert!(lines.iter().any(|line| line.starts_with(prefix)), "{lines:?}"),
            None => {
                // The map is the wrong kind for the float key `test.gain`, so the
                // value-kind rule refuses it; ASCII is not a rule any more.
                let refusal = lines.iter().find(|line| line.starts_with("x:submit:err:ezsdr.value:")).unwrap_or_else(|| panic!("no value-kind refusal: {lines:?}"));
                assert!(!refusal.to_lowercase().contains("ascii"), "{refusal}");
                assert!(!lines.iter().any(|line| line.starts_with("p:action:UpdateParameter")), "a refused update is not dispatched: {lines:?}");
            }
        }
        let manifest = run.finish_checked();
        assert!(manifest.hash.is_some());
        assert!(manifest.termination.cleanup_failures.is_empty());
    }
}

#[test]
fn kc_24a_start_a_simulation_first_step_follows_the_batch() {
    // #103: the start's batch is dispatched under the admission lock, before the
    // first round; a submission from that round is admitted after it (KC-24a).
    let (mut spec, mut profile) = executor_docs();
    add_schedule(
        &mut spec,
        "radio",
        0,
        serde_json::json!({
            "kind": "update_parameter",
            "target": serde_json::to_value(support::target("radio")).unwrap(),
            "key": "test.gain",
            "value": 1.0
        }),
    );
    let probe = Probe::new();
    let mut assembly = rig(Pacing::FreeRunning).assembly;
    let log = with_seen(&mut assembly, &mut profile);
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).declaring("radio/rx", 1, 1)));
    assembly.executors.insert(
        Ident::parse("exec").unwrap(),
        Box::new(ProbeExecutor::new("x", &probe).submitting(Action::UpdateParameter {
            target: support::target("radio"),
            key: Key::parse("test.gain").unwrap(),
            value: Value::num(2.0).unwrap(),
            class: UpdateClass::HardwareTimed,
            at: None,
        })),
    );
    let trace = StartTrace::default();
    let recorded = trace.clone();
    let started = start_on_worker(
        &probe,
        move |point, probe| {
            let reading = probe.read().expect("the Run is alive");
            recorded.push(point, reading);
            if point == start_probe::Point::Released {
                assert!(!reading.admission_held, "KC-24a: the admission lock is still held when the start releases it; the round would relock it");
            }
        },
        move || start_spec_run(&spec, &profile, assembly),
    );
    let run = started.finish();
    trace.assert_extent(&[
        start_probe::Point::Entered,
        start_probe::Point::BeforePublish,
        start_probe::Point::BeforeDispatch,
        start_probe::Point::Dispatching { next: 0 },
        start_probe::Point::Released,
    ]);
    assert!(probe.lines().iter().any(|line| line == "x:submit:ok:2"), "{:?}", probe.lines());
    assert_eq!(seen_for(&log, &Value::num(2.0).unwrap()), vec![Some(Value::num(1.0).unwrap())], "{:?}", probe.lines());
    assert_eq!(run.effective()[&Ident::parse("radio").unwrap()][&Key::parse("test.gain").unwrap()], Value::num(2.0).unwrap());
    assert!(matches!(run.state(), RunState::Running {}));
    assert!(run.finish_checked().termination.cleanup_failures.is_empty());
}

#[test]
fn kc_24a_start_reads_the_transition_clocks_outside_the_lock() {
    // KC-24a: every supplied call the start makes is made with neither the admission
    // lock nor the machine lock held, so no Module waits on a lock a callback needs.
    // The Running transition's pair is pinned: the Authority's now() first, then the
    // host clock's UTC, both before the admission lock.
    // Deschedule: only the start worker makes the recorded calls; the test thread
    // waits for it to return (10 s backstop), so a late worker delays the assertions
    // but cannot flip them.
    let probe = Probe::new();
    let mut rig = rig(Pacing::FreeRunning);
    let (authority, root) = SimAuthority::new(&rig.clocks, mref("ezsdr.test.provider"), Pacing::FreeRunning);
    rig.root = root;
    rig.manual = authority.manual();
    let log = ClockLog::default();
    let clock = WatchedClock::new(rig.host.clone(), log.clone());
    rig.assembly.authority = Box::new(WatchedAuthority::new(Box::new(authority), log.clone()));
    rig.assembly.host_clock = Arc::new(clock);
    rig.assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe)));
    let primary = rig.root;
    let trace = StartTrace::default();
    let recorded = trace.clone();
    let hook_log = log.clone();
    let started = start_on_worker(
        &probe,
        move |point, probe| {
            let reading = probe.read().expect("the Run is alive");
            recorded.push(point, reading);
            hook_log.push_point(point);
        },
        move || start_spec_run(&spec_one(), &profile_one(), rig.assembly),
    );
    let run = started.finish();
    // Everything the log holds now is the start's; cleanup's own calls (T7's subject) come later.
    let events = log.events();
    for call in log.calls() {
        assert!(
            !call.held.admission && !call.held.machine,
            "KC-24a: {:?} on {:?} ran under a coordinator lock (admission {}, machine {}); events: {events:?}",
            call.kind,
            call.thread,
            call.held.admission,
            call.held.machine
        );
    }
    let bp = events.iter().position(|event| matches!(event, LogEvent::Point(start_probe::Point::BeforePublish))).expect("the start reached BeforePublish");
    assert!(bp >= 2, "KC-24a: no clock reads were logged before BeforePublish; events: {events:?}");
    let now_sample = match &events[bp - 2] {
        LogEvent::Call(Call { kind: CallKind::Now, sample: Some(sample), .. }) => *sample,
        other => panic!("KC-24a: the event before the UTC read is {other:?}, not the Authority's now"),
    };
    let utc_sample = match &events[bp - 1] {
        LogEvent::Call(Call { kind: CallKind::Utc, sample: Some(sample), .. }) => *sample,
        other => panic!("KC-24a: the event before the publication is {other:?}, not the host clock's UTC read"),
    };
    let manifest = run.finish_checked();
    let row = manifest.run.transitions.iter().find(|row| matches!(row.state, RunState::Running {})).expect("the Run recorded Running");
    assert_eq!(row.host_utc_nanos, utc_sample, "the recorded UTC is not the sampled one; events: {events:?}");
    let at = row.at.expect("Running carries its runtime instant");
    assert_eq!(at.ticks_in(primary).expect("on the primary root"), now_sample, "the recorded instant is not the sampled one; events: {events:?}");
}

#[test]
fn kc_24a_cleanup_reads_the_stopping_and_cleaned_up_clocks_outside_the_machine_lock() {
    // KC-24a, KC-43: every transition's two reads are sampled before a lock is taken
    // and published with them, so neither the Authority's now() nor the host clock's
    // UTC runs under the machine lock. Every read of the two clocks in the run is
    // checked, and every transition after Created is pinned to its two samples.
    // Deschedule: one control thread runs the start and the cleanup in a Simulation
    // Run, and the log is read after `finish` returned; nothing can interleave.
    let probe = Probe::new();
    let mut rig = rig(Pacing::FreeRunning);
    let (authority, root) = SimAuthority::new(&rig.clocks, mref("ezsdr.test.provider"), Pacing::FreeRunning);
    rig.root = root;
    rig.manual = authority.manual();
    let log = ClockLog::default();
    let clock = WatchedClock::new(rig.host.clone(), log.clone());
    rig.assembly.authority = Box::new(WatchedAuthority::new(Box::new(authority), log.clone()));
    rig.assembly.host_clock = Arc::new(clock);
    rig.assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe)));
    let primary = rig.root;
    let run = start_spec_run(&spec_one(), &profile_one(), rig.assembly).expect("the fixture Run starts");
    let manifest = run.finish_checked();
    let calls = log.calls();
    let transitions = &manifest.run.transitions;
    assert!(matches!(transitions.first().map(|row| &row.state), Some(RunState::Created {})), "the first transition is not Created: {transitions:?}");
    assert!(transitions.iter().any(|row| matches!(row.state, RunState::Running {})), "the Run recorded no Running transition: {transitions:?}");
    assert!(transitions.iter().any(|row| matches!(row.state, RunState::Stopping { .. })), "the Run recorded no Stopping transition: {transitions:?}");
    assert!(transitions.iter().any(|row| matches!(row.state, RunState::CleanedUp { .. })), "the Run recorded no CleanedUp transition: {transitions:?}");
    let utc: Vec<&Call> = calls.iter().filter(|call| call.kind == CallKind::Utc).collect();
    assert_eq!(utc.len(), transitions.len(), "KC-43: the UTC reads are not one per transition; calls: {calls:?}");
    for (call, row) in utc.iter().zip(transitions) {
        assert_eq!(call.sample, Some(row.host_utc_nanos), "KC-43: the {:?} transition's UTC is not the sampled one; calls: {calls:?}", row.state);
    }
    // KC-24a: neither clock is read under the admission lock or the machine lock
    // anywhere in the run. This is checked before the pairs below, because a read
    // taken before its partner (N111-8's and N111-10's shape) leaves the pairs
    // intact whenever an earlier call of the same thread precedes the UTC read.
    for call in calls.iter().filter(|call| matches!(call.kind, CallKind::Now | CallKind::Utc)) {
        assert!(
            !call.held.machine && !call.held.admission,
            "KC-24a: {:?} on {:?} ran under a coordinator lock (admission {}, machine {}); calls: {calls:?}",
            call.kind,
            call.thread,
            call.held.admission,
            call.held.machine
        );
    }
    for row in transitions.iter().skip(1) {
        let position = calls.iter().position(|call| call.kind == CallKind::Utc && call.sample == Some(row.host_utc_nanos)).expect("the transition has its UTC read");
        let utc_call = &calls[position];
        assert!(
            !utc_call.held.machine && !utc_call.held.admission,
            "KC-24a: the {:?} transition's UTC read ran under a coordinator lock (admission {}, machine {}): {utc_call:?}; calls: {calls:?}",
            row.state,
            utc_call.held.admission,
            utc_call.held.machine
        );
        let now_call = calls[..position]
            .iter()
            .rev()
            .find(|call| call.thread == utc_call.thread)
            .unwrap_or_else(|| panic!("KC-43: the {:?} transition's UTC read has no earlier call on its thread; calls: {calls:?}", row.state));
        assert_eq!(now_call.kind, CallKind::Now, "KC-43: the call before the {:?} transition's UTC read is {now_call:?}, not the Authority's now; calls: {calls:?}", row.state);
        let at = row.at.expect("a transition after Created carries its runtime instant");
        assert_eq!(now_call.sample, Some(at.ticks_in(primary).expect("on the primary root")), "KC-43: the {:?} transition's instant is not the sampled one; calls: {calls:?}", row.state);
        assert!(
            !now_call.held.machine && !now_call.held.admission,
            "KC-24a: the {:?} transition's now read ran under a coordinator lock (admission {}, machine {}): {now_call:?}; calls: {calls:?}",
            row.state,
            now_call.held.admission,
            now_call.held.machine
        );
    }
    let cleaned_up = transitions.iter().find(|row| matches!(row.state, RunState::CleanedUp { .. })).expect("the Run recorded CleanedUp");
    assert_eq!(manifest.termination.at, cleaned_up.at, "KC-43: the termination's instant is not the CleanedUp transition's");
    assert_eq!(manifest.termination.host_utc_nanos, cleaned_up.host_utc_nanos, "KC-43: the termination's UTC is not the CleanedUp transition's");
}

#[test]
fn kc_24a_freeze_runs_cancels_outside_the_admission_lock() {
    // #111: RS-6 step 1 sets `frozen` and empties the queues under the admission
    // lock, drops it, and only then takes and cancels the handles -- all under the
    // freeze gate, which no Module's thread takes.
    // Deschedule: the start worker only builds the Run before `finish_checked()`; if it
    // is descheduled the test waits for it (10 s backstop) and reads the same values.
    // The freeze itself runs on the cleanup thread inside `finish_checked()` on this
    // thread, the one that waits there, so nothing the test does can interleave with
    // its points.
    let mut spec = spec_one();
    add_schedule(&mut spec, "radio", 1_000_000_000_000, serde_json::json!({ "kind": "stop", "target": null }));
    let mut second = spec["schedule"][0].clone();
    second["at"]["offset_ticks"] = serde_json::json!(2_000_000_000_000i64);
    spec["schedule"].as_array_mut().expect("the fixture schedule is an array").push(second);

    let probe = Probe::new();
    let mut rig = rig(Pacing::FreeRunning);
    let (authority, root) = SimAuthority::new(&rig.clocks, mref("ezsdr.test.provider"), Pacing::FreeRunning);
    rig.root = root;
    rig.manual = authority.manual();
    let log = ClockLog::default();
    let clock = WatchedClock::new(rig.host.clone(), log.clone());
    rig.assembly.authority = Box::new(WatchedAuthority::new(Box::new(authority), log.clone()));
    rig.assembly.host_clock = Arc::new(clock);
    rig.assembly
        .providers
        .insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).declaring("radio/rx", 10, 1).registering_at_arm("radio/rx")));
    let freezes = Arc::new(Mutex::new(Vec::<(start_probe::FreezePoint, start_probe::FreezeReading)>::new()));
    let recorded = freezes.clone();
    let started = start_on_worker(
        &probe,
        move |point, probe| {
            if point == start_probe::Point::Entered {
                let recorded = recorded.clone();
                probe.on_freeze(move |point, reading| recorded.lock().unwrap_or_else(|e| e.into_inner()).push((point, reading)));
            }
        },
        move || start_spec_run(&spec, &profile_one(), rig.assembly),
    );
    let run = started.finish();
    let manifest = run.finish_checked();

    // The two far-future stop entries are the two latest schedules; their handles are
    // what the freeze cancels.
    let calls = log.calls();
    let mut latest: Vec<Call> = calls.iter().filter(|call| call.kind == CallKind::Schedule).cloned().collect();
    latest.sort_by_key(|call| std::cmp::Reverse(call.sample));
    assert!(latest.len() >= 2, "RS-6 step 1: the two stop entries were not scheduled; calls: {calls:?}");
    let handles: Vec<_> = latest[..2].iter().map(|call| call.handle.expect("the schedule returned its handle")).collect();

    let cancels: Vec<&Call> = calls.iter().filter(|call| call.kind == CallKind::Cancel).collect();
    assert_eq!(cancels.len(), 2, "RS-6 step 1 cancels each held handle exactly once; calls: {calls:?}");
    for handle in &handles {
        let count = cancels.iter().filter(|call| call.handle == Some(*handle)).count();
        assert_eq!(count, 1, "RS-6 step 1 does not cancel {handle:?} exactly once; calls: {calls:?}");
    }
    for call in &cancels {
        assert_eq!(call.returned, Some(true), "RS-6 step 1: the cancel did not return; calls: {calls:?}");
        assert!(!call.held.admission, "KC-24a: the cancel ran under the admission lock; calls: {calls:?}");
        assert!(!call.held.machine, "KC-24a: the cancel ran under the machine lock; calls: {calls:?}");
        assert!(call.held.freeze, "KC-24a: the cancel ran outside the freeze gate; calls: {calls:?}");
    }

    let readings = freezes.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let points: Vec<start_probe::FreezePoint> = readings.iter().map(|(point, _)| *point).collect();
    assert_eq!(
        points,
        [start_probe::FreezePoint::Serialise, start_probe::FreezePoint::Serialised, start_probe::FreezePoint::BeforeDrop, start_probe::FreezePoint::BeforeWakeTake, start_probe::FreezePoint::Done,],
        "one performance of RS-6 step 1 reaches each point once, in order: {readings:?}"
    );
    let thread = readings[0].1.thread;
    assert!(readings.iter().all(|(_, reading)| reading.thread == thread), "RS-6 step 1's points are on one thread: {readings:?}");
    let at = |point| readings.iter().find(|(reached, _)| *reached == point).expect("the point was recorded").1;
    let serialise = at(start_probe::FreezePoint::Serialise);
    assert!(!serialise.held.freeze, "KC-24a: the freeze gate was already held at Serialise: {serialise:?}");
    let serialised = at(start_probe::FreezePoint::Serialised);
    assert!(serialised.held.freeze && !serialised.held.admission, "KC-24a: Serialised is not between the gate and the admission lock: {serialised:?}");
    let before_drop = at(start_probe::FreezePoint::BeforeDrop);
    assert!(before_drop.held.freeze && before_drop.held.admission && before_drop.dispatch_frozen, "KC-24a: BeforeDrop is not under both locks with dispatch frozen: {before_drop:?}");
    let before_wake = at(start_probe::FreezePoint::BeforeWakeTake);
    assert!(before_wake.held.freeze && !before_wake.held.admission && before_wake.dispatch_frozen, "KC-24a: the wake handle is taken under the admission lock: {before_wake:?}");
    let done = at(start_probe::FreezePoint::Done);
    assert!(done.held.freeze && !done.held.admission, "KC-24a: Done is not under the gate alone: {done:?}");

    assert!(manifest.termination.cleanup_failures.is_empty(), "the cleanup recorded failures: {:?}", manifest.termination.cleanup_failures);
    assert!(manifest.termination.undelivered.is_empty(), "RS-16: the freeze discarded Actions: {:?}", manifest.termination.undelivered);
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Client {} }, "RS-10: the first end request fixes the reason");
}

#[test]
fn kc_24a_an_abort_in_the_unlocked_cleanup_window_escalates_the_remaining_steps() {
    // KC-24a, RS-10, KC-39: the Stopping entry samples its clocks before the machine
    // lock, so an Abort submitted in that window reaches the end while the cleanup's
    // steps run: they escalate, and the Stopping mode stays the orderly one the
    // cleanup began with.
    // Deschedule: W_f parks in the Stopping sample until the test has submitted the
    // Abort and opened `release`, so the Abort always lands in that window; a test
    // thread descheduled after it sees the parked line leaves W_f parked and changes
    // nothing, and W_f descheduled before now() makes the 10 s wait an infra failure.
    let (spec, profile) = executor_docs();
    let probe = Probe::new();
    let rig = rig(Pacing::FreeRunning);
    let worker = Arc::new(Mutex::new(None::<std::thread::ThreadId>));
    let taken = Arc::new(AtomicBool::new(false));
    let release = gate();
    let on_now = {
        let probe = probe.clone();
        let worker = worker.clone();
        let taken = taken.clone();
        let release = release.clone();
        move |call: &Call| {
            let Some(worker) = *worker.lock().unwrap_or_else(|e| e.into_inner()) else {
                return;
            };
            if call.thread != worker || taken.swap(true, Ordering::SeqCst) {
                return;
            }
            probe.record("finish:now:parked");
            wait(&release);
        }
    };
    let mut assembly = rig.assembly;
    assembly.authority = Box::new(WatchedAuthority::new(assembly.authority, ClockLog::default()).with_hooks(Hooks { on_now: Some(Arc::new(on_now)), ..Hooks::default() }));
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe)));
    let slot: Arc<Mutex<Option<Arc<dyn ActionSubmitter>>>> = Arc::new(Mutex::new(None));
    assembly.executors.insert(Ident::parse("exec").unwrap(), Box::new(ProbeExecutor::new("x", &probe).sharing_submitter(slot.clone())));
    let run = start_spec_run(&spec, &profile, assembly).expect("the fixture Run starts");
    let submitter = slot.lock().unwrap_or_else(|e| e.into_inner()).clone().expect("the Executor was prepared");
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = {
        let worker = worker.clone();
        std::thread::spawn(move || {
            *worker.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::thread::current().id());
            let _ = tx.send(run.finish());
        })
    };
    assert!(probe.wait_for("finish:now:parked", std::time::Duration::from_secs(10)), "{}", infra("W_f did not reach the Stopping sample within 10 s"));
    let id = submitter.submit(Action::Abort { cause: StopCause::Abort { message: "test".to_owned() } });
    assert!(id.is_ok(), "KC-24: an Abort is accepted in the cleanup window: {id:?}");
    open(&release);
    let manifest = match rx.recv_timeout(std::time::Duration::from_secs(10)) {
        Ok(manifest) => manifest,
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            handle.join().expect("W_f does not panic");
            panic!("W_f returned no Manifest");
        }
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => panic!("{}", infra("W_f did not return a Manifest within 10 s")),
    };
    handle.join().expect("W_f does not panic");
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Client {} }, "RS-10: the first end request fixes the reason");
    assert_eq!(manifest.termination.also, vec![Termination::Stopped { cause: StopCause::Abort { message: "test".to_owned() } }], "RS-10: the Abort is not recorded beside the first request");
    let stopping = manifest.run.transitions.iter().find(|row| matches!(row.state, RunState::Stopping { .. })).expect("the Run recorded Stopping");
    assert!(matches!(stopping.state, RunState::Stopping { mode: CleanupMode::Orderly }), "KC-43: the Stopping transition records {stopping:?}, not the orderly mode read before the Abort");
    assert_eq!(probe.with_prefix("p:stop"), vec!["p:stop:Abort".to_owned()], "KC-39: the Provider was not stopped with the escalated mode; lines: {:?}", probe.lines());
    assert!(manifest.termination.cleanup_failures.is_empty(), "the cleanup recorded failures: {:?}", manifest.termination.cleanup_failures);
}

#[test]
fn kc_24a_freeze_a_provider_holding_what_cancel_needs_while_it_submits_does_not_deadlock() {
    // #111: RS-6 step 1 drops the admission lock before it cancels, so a Module thread
    // that holds a lock the Authority's cancel waits for and then submits meets no lock
    // the freeze holds: its submission is refused at RS-6 and the cancel completes.
    // Additional killer for N111-1, whose row names T1.
    // Deschedule: W is gated by the cancel itself (the gate opens only inside the hook)
    // and M is held until W releases it, so no descheduling flips the order. If W is
    // descheduled after the gate opens, the cancel waits for M with a bounded try_lock
    // loop and fails as infra at 10 s, never with an unbounded lock.
    let (mut spec, profile) = executor_docs();
    add_schedule(&mut spec, "radio", 1_000_000_000_000, serde_json::json!({ "kind": "stop", "target": null }));
    let probe = Probe::new();
    let mut rig = rig(Pacing::FreeRunning);
    let (authority, root) = SimAuthority::new(&rig.clocks, mref("ezsdr.test.provider"), Pacing::FreeRunning);
    rig.root = root;
    rig.manual = authority.manual();
    let log = ClockLog::default();
    let cancel_gate = gate();
    let _release = OpenOnDrop(cancel_gate.clone());
    let held_by_w = Arc::new(Mutex::new(()));
    let timed_out = Arc::new(AtomicBool::new(false));
    let on_cancel = {
        let probe = probe.clone();
        let gate = cancel_gate.clone();
        let held_by_w = held_by_w.clone();
        let timed_out = timed_out.clone();
        move |call: &Call| {
            probe.record(format!("cancel:entered:admission={}:machine={}:freeze={}", call.held.admission, call.held.machine, call.held.freeze));
            open(&gate);
            if call.held.admission {
                // The mutant's freeze still holds the lock: return without M, so the kill
                // is the held assertion below and never a deadlock.
                return;
            }
            let begun = std::time::Instant::now();
            loop {
                match held_by_w.try_lock() {
                    Ok(guard) => {
                        drop(guard);
                        probe.record("cancel:done");
                        return;
                    }
                    Err(std::sync::TryLockError::Poisoned(guard)) => {
                        drop(guard.into_inner());
                        probe.record("cancel:done");
                        return;
                    }
                    Err(std::sync::TryLockError::WouldBlock) => {
                        if begun.elapsed() >= std::time::Duration::from_secs(10) {
                            timed_out.store(true, Ordering::SeqCst);
                            probe.record("cancel:timeout");
                            return;
                        }
                        std::thread::yield_now();
                    }
                }
            }
        }
    };
    rig.assembly.authority = Box::new(WatchedAuthority::new(Box::new(authority), log.clone()).with_hooks(Hooks { on_cancel: Some(Arc::new(on_cancel)), ..Hooks::default() }));
    rig.assembly
        .providers
        .insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).declaring("radio/rx", 10, 1).registering_at_arm("radio/rx")));
    let slot: Arc<Mutex<Option<Arc<dyn ActionSubmitter>>>> = Arc::new(Mutex::new(None));
    rig.assembly.executors.insert(Ident::parse("exec").unwrap(), Box::new(ProbeExecutor::new("x", &probe).sharing_submitter(slot.clone())));
    let run = start_spec_run(&spec, &profile, rig.assembly).expect("the fixture Run starts");
    let start = run.start_instant().expect("the Run was armed").ticks_in(rig.root).expect("the start is on the primary root");
    let (handle, _instant) = far_future_stop(&log.calls(), start);
    let submitter = slot.lock().unwrap_or_else(|e| e.into_inner()).clone().expect("the Executor was prepared");
    let action = Action::UpdateParameter { target: support::target("radio"), key: Key::parse("test.gain").unwrap(), value: Value::num(2.0).unwrap(), class: UpdateClass::HardwareTimed, at: None };
    let id = submitter.submit(action.clone()).expect("KC-24: the submission is admitted while the Run is Running");
    // W takes M (what the Authority's cancel needs), submits through the Run's own
    // submitter, then records its release while it still holds M.
    let worker = {
        let probe = probe.clone();
        let gate = cancel_gate.clone();
        let held_by_w = held_by_w.clone();
        std::thread::spawn(move || {
            let guard = held_by_w.lock().unwrap_or_else(|e| e.into_inner());
            probe.record("w:holds_m");
            wait(&gate);
            let line = match submitter.submit(action) {
                Ok(id) => format!("w:submit:Ok({id:?})"),
                Err(violations) => format!("w:submit:Err({}:{})", violations[0].check.as_str(), violations[0].reason),
            };
            probe.record(line);
            probe.record("w:releasing_m");
            drop(guard);
        })
    };
    assert!(probe.wait_for("w:holds_m", std::time::Duration::from_secs(10)), "{}", infra("W did not take M within 10 s"));
    let (manifest_tx, manifest_rx) = std::sync::mpsc::channel();
    let finisher = std::thread::spawn(move || {
        let _ = manifest_tx.send(run.finish());
    });
    let manifest = match manifest_rx.recv_timeout(std::time::Duration::from_secs(10)) {
        Ok(manifest) => manifest,
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            finisher.join().expect("W_f does not panic");
            panic!("W_f returned no Manifest");
        }
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => panic!("{}", infra("W_f did not return a Manifest within 10 s")),
    };
    finisher.join().expect("W_f does not panic");
    worker.join().expect("W does not panic");
    assert!(!timed_out.load(Ordering::SeqCst), "{}", infra("the cancel waited more than 10 s for M"));
    let calls = log.calls();
    let cancels: Vec<&Call> = calls.iter().filter(|call| call.kind == CallKind::Cancel && call.handle == Some(handle)).collect();
    assert_eq!(cancels.len(), 1, "RS-6 step 1 cancels H once; calls: {calls:?}");
    assert!(!cancels[0].held.admission, "KC-24a: the freeze's cancel ran under the admission lock; calls: {calls:?}");
    assert!(cancels[0].held.freeze, "KC-24a: the freeze's cancel ran outside the freeze gate; calls: {calls:?}");
    assert_eq!(cancels[0].returned, Some(true), "RS-6 step 1: the cancel did not return; calls: {calls:?}");
    let lines = probe.lines();
    let submitted = lines.iter().find(|line| line.starts_with("w:submit:")).expect("W's submission returned");
    assert!(submitted.starts_with("w:submit:Err(ezsdr.dispatch:RS-6: dispatch is frozen)"), "KC-24a: the submission during the freeze was not refused at RS-6; lines: {lines:?}");
    assert_eq!(manifest.termination.undelivered, vec![id], "RS-16: the freeze discards exactly the Action the first submission was given; lines: {lines:?}");
    assert!(manifest.termination.cleanup_failures.is_empty(), "the cleanup recorded failures: {:?}", manifest.termination.cleanup_failures);
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Client {} }, "RS-10: the first end request fixes the reason");
    let at = |line: &str| lines.iter().position(|observed| observed == line).unwrap_or_else(|| panic!("the line {line} is missing; lines: {lines:?}"));
    let entered = lines.iter().position(|line| line.starts_with("cancel:entered")).expect("the cancel entered");
    assert!(
        at("w:holds_m") < entered
            && entered < at("w:submit:Err(ezsdr.dispatch:RS-6: dispatch is frozen)")
            && at("w:submit:Err(ezsdr.dispatch:RS-6: dispatch is frozen)") < at("w:releasing_m")
            && at("w:releasing_m") < at("cancel:done"),
        "KC-24a: the freeze's cancel and W's submission did not interleave in the observed order; lines: {lines:?}"
    );
}

#[test]
fn kc_24a_freeze_a_handle_fired_before_its_cancel_is_not_a_failure() {
    // KC-30, RS-6 step 1: a handle whose callback fired before the freeze cancels it
    // makes `cancel` return false, which is not a failure. The cancel hook fires H's
    // callback itself, so the count and the outcome are deterministic.
    // Deschedule: the start and `finish_checked` run on this thread, and the hook runs
    // on the freeze's step thread inside the latter; nothing interleaves with the count.
    let mut spec = spec_one();
    add_schedule(&mut spec, "radio", 1_000_000_000_000, serde_json::json!({ "kind": "stop", "target": null }));
    let probe = Probe::new();
    let mut rig = rig(Pacing::FreeRunning);
    let (authority, root) = SimAuthority::new(&rig.clocks, mref("ezsdr.test.provider"), Pacing::FreeRunning);
    rig.root = root;
    rig.manual = authority.manual();
    let log = ClockLog::default();
    let instant = Arc::new(std::sync::atomic::AtomicI64::new(i64::MIN));
    let fired = Arc::new(Mutex::new(None::<Result<usize, String>>));
    let first = Arc::new(AtomicBool::new(false));
    let on_cancel = {
        let manual = rig.manual.clone();
        let fired = fired.clone();
        let instant = instant.clone();
        let first = first.clone();
        move |_call: &Call| {
            if first.swap(true, Ordering::SeqCst) {
                return;
            }
            let at = instant.load(Ordering::SeqCst);
            let result = manual.advance_to(TimePoint::new(root, at)).map_err(|error| format!("{error:?}"));
            *fired.lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
        }
    };
    rig.assembly.authority = Box::new(WatchedAuthority::new(Box::new(authority), log.clone()).with_hooks(Hooks { on_cancel: Some(Arc::new(on_cancel)), ..Hooks::default() }));
    rig.assembly
        .providers
        .insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).declaring("radio/rx", 10, 1).registering_at_arm("radio/rx")));
    let run = start_spec_run(&spec, &profile_one(), rig.assembly).expect("the fixture Run starts");
    let start = run.start_instant().expect("the Run was armed").ticks_in(rig.root).expect("the start is on the primary root");
    let calls = log.calls();
    assert_eq!(calls.iter().filter(|call| call.kind == CallKind::Schedule).count(), 1, "the fixture schedules one handle; calls: {calls:?}");
    let (handle, ticks) = far_future_stop(&calls, start);
    instant.store(ticks, Ordering::SeqCst);
    let manifest = run.finish_checked();
    let fired = fired.lock().unwrap_or_else(|e| e.into_inner()).clone();
    assert_eq!(fired, Some(Ok(1)), "KC-30: the Authority did not fire the handle's one callback before the freeze's cancel; calls: {:?}", log.calls());
    let calls = log.calls();
    let cancel = calls.iter().find(|call| call.kind == CallKind::Cancel && call.handle == Some(handle)).expect("RS-6 step 1 cancels the handle");
    assert_eq!(cancel.returned, Some(false), "KC-30: cancelling a handle that already fired did not return false; calls: {calls:?}");
    assert!(manifest.termination.cleanup_failures.is_empty(), "KC-30: a handle fired before its cancel is a cleanup failure: {:?}", manifest.termination.cleanup_failures);
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Client {} }, "RS-10: the first end request fixes the reason");
    assert!(manifest.termination.also.is_empty(), "RS-10: no other request is recorded; {:?}", manifest.termination.also);
    assert!(manifest.termination.undelivered.is_empty(), "RS-16: the fixture queued no Action; {:?}", manifest.termination.undelivered);
}

/// T11/T12's fixture: `executor_docs()` with a timed `test.gain = 1.0` entry at offset 0
/// and H, the far-future untimed `stop`, and the Executor's first step submitting
/// `test.gain = 2.0` once the Run is Running (the same shared submitter the test thread
/// uses afterwards).
fn sweep_simulation_docs() -> (serde_json::Value, serde_json::Value) {
    let (mut spec, profile) = executor_docs();
    spec["schedule"] = serde_json::json!([
        {
            "at": { "clock": "radio", "offset_ticks": 0 },
            "action": { "kind": "update_parameter", "target": serde_json::to_value(support::target("radio")).unwrap(), "key": "test.gain", "value": 1.0 }
        },
        { "at": { "clock": "radio", "offset_ticks": 1_000_000_000_000i64 }, "action": { "kind": "stop", "target": null } }
    ]);
    (spec, profile)
}

/// The Module update the Executor submits in its first step (T11, T12).
fn sweep_module_update() -> Action {
    Action::UpdateParameter { target: support::target("radio"), key: Key::parse("test.gain").unwrap(), value: Value::num(2.0).unwrap(), class: UpdateClass::HardwareTimed, at: None }
}

#[test]
fn kc_24a_sweep_an_orderly_run_makes_no_supplied_call_under_a_lock() {
    // KC-24a (#111, #119): over the whole log of an orderly Simulation Run -- a timed
    // Action dispatched in the start batch, a Module Action submitted while Running, and
    // `finish` -- no supplied call (now, UTC, monotonic, schedule, cancel, wait_until,
    // next_wakeup, relations) is made under the admission or the machine lock; H is
    // scheduled in the Armed window and cancelled once in the Stopping window.
    // Deschedule: a Simulation Run's whole control path (the start's round, the test
    // thread's submission and `finish_checked`) runs on this thread, and no other thread
    // makes a watched call, so a descheduled thread only delays the log's last entry.
    let (spec, profile) = sweep_simulation_docs();
    let probe = Probe::new();
    let mut rig = rig(Pacing::FreeRunning);
    let log = ClockLog::default();
    let clock = WatchedClock::new(rig.host.clone(), log.clone());
    rig.assembly.authority = Box::new(WatchedAuthority::new(rig.assembly.authority, log.clone()));
    rig.assembly.host_clock = Arc::new(clock);
    rig.assembly
        .providers
        .insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).declaring("radio/rx", 10, 1).registering_at_arm("radio/rx")));
    let slot: Arc<Mutex<Option<Arc<dyn ActionSubmitter>>>> = Arc::new(Mutex::new(None));
    rig.assembly.executors.insert(Ident::parse("exec").unwrap(), Box::new(ProbeExecutor::new("x", &probe).sharing_submitter(slot.clone()).submitting(sweep_module_update())));
    let run = start_spec_run(&spec, &profile, rig.assembly).expect("the fixture Run starts");
    let start = run.start_instant().expect("the Run was armed").ticks_in(rig.root).expect("the start is on the primary root");
    let submitter = slot.lock().unwrap_or_else(|e| e.into_inner()).clone().expect("the Executor was prepared");
    let id = submitter.submit(sweep_module_update()).expect("KC-24: a Module update is admitted while the Run is Running");
    // The clock advances, so the Stopping and CleanedUp transitions' Now samples are not all 0.
    rig.manual.advance_to(TimePoint::new(rig.root, 500)).expect("the manual clock advances");
    let manifest = run.finish_checked();
    let calls = log.calls();
    let phases = sweep(&calls, &manifest.run.transitions, rig.root);
    let (handle, _instant) = far_future_stop(&calls, start);
    let (scheduled, _) = calls.iter().enumerate().find(|(_, call)| call.kind == CallKind::Schedule && call.handle == Some(handle)).expect("KC-20: H was scheduled");
    assert!(phases.armed.contains(&scheduled), "KC-20: H's Schedule is not in the Armed window; phases: {phases:?}; calls: {calls:?}");
    let cancels: Vec<(usize, &Call)> = calls.iter().enumerate().filter(|(_, call)| call.kind == CallKind::Cancel && call.handle == Some(handle)).collect();
    assert_eq!(cancels.len(), 1, "RS-6 step 1: H is cancelled exactly once; calls: {calls:?}");
    assert!(phases.stopping.contains(&cancels[0].0), "KC-24a: the Cancel of H is not in the Stopping window; phases: {phases:?}; calls: {calls:?}");
    let lines = probe.lines();
    assert!(lines.iter().any(|line| line == "p:update:test.gain=1.0"), "the timed Action did not reach the Provider; lines: {lines:?}");
    assert!(lines.iter().any(|line| line == "x:submit:ok:2"), "the Executor's Module update was not admitted; lines: {lines:?}");
    assert_eq!(id.0, 3, "the test thread's Module update did not take the next id; lines: {lines:?}");
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Client {} }, "RS-10: the orderly finish fixes the reason");
}

#[test]
fn kc_24a_sweep_an_abort_run_makes_no_supplied_call_under_a_lock() {
    // KC-24a (#111, #119): the same sweep for a Run ended by a Module `Action::Abort`
    // submitted while Running (no `finish` request first): the Stopping transition keeps
    // the abort mode and the Termination the Abort's cause.
    // Deschedule: as T11, every control-path step runs on this thread.
    let (spec, profile) = sweep_simulation_docs();
    let probe = Probe::new();
    let mut rig = rig(Pacing::FreeRunning);
    let log = ClockLog::default();
    let clock = WatchedClock::new(rig.host.clone(), log.clone());
    rig.assembly.authority = Box::new(WatchedAuthority::new(rig.assembly.authority, log.clone()));
    rig.assembly.host_clock = Arc::new(clock);
    rig.assembly
        .providers
        .insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).declaring("radio/rx", 10, 1).registering_at_arm("radio/rx")));
    let slot: Arc<Mutex<Option<Arc<dyn ActionSubmitter>>>> = Arc::new(Mutex::new(None));
    rig.assembly.executors.insert(Ident::parse("exec").unwrap(), Box::new(ProbeExecutor::new("x", &probe).sharing_submitter(slot.clone()).submitting(sweep_module_update())));
    let run = start_spec_run(&spec, &profile, rig.assembly).expect("the fixture Run starts");
    let start = run.start_instant().expect("the Run was armed").ticks_in(rig.root).expect("the start is on the primary root");
    let submitter = slot.lock().unwrap_or_else(|e| e.into_inner()).clone().expect("the Executor was prepared");
    submitter.submit(Action::Abort { cause: StopCause::Abort { message: "sweep".to_owned() } }).expect("KC-24: an Abort is accepted while Running");
    // The clock advances, so the Stopping and CleanedUp transitions' Now samples are not all 0.
    rig.manual.advance_to(TimePoint::new(rig.root, 500)).expect("the manual clock advances");
    let manifest = run.finish_checked();
    let calls = log.calls();
    let phases = sweep(&calls, &manifest.run.transitions, rig.root);
    let (handle, _instant) = far_future_stop(&calls, start);
    let (scheduled, _) = calls.iter().enumerate().find(|(_, call)| call.kind == CallKind::Schedule && call.handle == Some(handle)).expect("KC-20: H was scheduled");
    assert!(phases.armed.contains(&scheduled), "KC-20: H's Schedule is not in the Armed window; phases: {phases:?}; calls: {calls:?}");
    let cancels: Vec<(usize, &Call)> = calls.iter().enumerate().filter(|(_, call)| call.kind == CallKind::Cancel && call.handle == Some(handle)).collect();
    assert_eq!(cancels.len(), 1, "RS-6 step 1: H is cancelled exactly once; calls: {calls:?}");
    assert!(phases.stopping.contains(&cancels[0].0), "KC-24a: the Cancel of H is not in the Stopping window; phases: {phases:?}; calls: {calls:?}");
    let lines = probe.lines();
    assert!(lines.iter().any(|line| line == "p:update:test.gain=1.0"), "the timed Action did not reach the Provider; lines: {lines:?}");
    assert!(lines.iter().any(|line| line == "x:submit:ok:2"), "the Executor's Module update was not admitted; lines: {lines:?}");
    let stopping = manifest.run.transitions.iter().find(|row| matches!(row.state, RunState::Stopping { .. })).expect("the Run recorded Stopping");
    assert_eq!(stopping.state, RunState::Stopping { mode: CleanupMode::Abort }, "KC-43: the Stopping transition does not carry the abort mode: {stopping:?}");
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Abort { message: "sweep".to_owned() } }, "RS-10: the Abort's cause is not the reason");
    assert!(manifest.termination.also.is_empty(), "RS-10: the finish's own request is dropped; {:?}", manifest.termination.also);
}

#[test]
fn kc_24a_start_a_submission_from_start_is_refused_not_blocked() {
    // #103: `start_instances` runs before the admission lock, so a submission from
    // an instance's `start` is refused at KC-24 step 2 instead of waiting for it.
    let (spec, mut profile) = executor_docs();
    let probe = Probe::new();
    let mut assembly = rig(Pacing::FreeRunning).assembly;
    let log = with_seen(&mut assembly, &mut profile);
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe)));
    let slot: Arc<Mutex<Option<start_probe::Probe>>> = Arc::new(Mutex::new(None));
    assembly.executors.insert(
        Ident::parse("exec").unwrap(),
        Box::new(ProbeExecutor::new("x", &probe).submitting_from_start(
            Action::UpdateParameter { target: support::target("radio"), key: Key::parse("test.gain").unwrap(), value: Value::num(2.0).unwrap(), class: UpdateClass::HardwareTimed, at: None },
            slot.clone(),
        )),
    );
    let trace = StartTrace::default();
    let recorded = trace.clone();
    let hook_slot = slot.clone();
    let started = start_on_worker(
        &probe,
        move |point, probe| {
            let reading = probe.read().expect("the Run is alive");
            recorded.push(point, reading);
            if point == start_probe::Point::Entered {
                *hook_slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(probe.clone());
            }
        },
        move || start_spec_run(&spec, &profile, assembly),
    );
    let run = started.finish();
    assert!(probe.lines().iter().any(|line| line == "x:start:held=false"), "{:?}", probe.lines());
    assert!(probe.lines().iter().any(|line| line.starts_with("x:start:err:ezsdr.run_state:RS-18")), "{:?}", probe.lines());
    assert!(!probe.lines().iter().any(|line| line.starts_with("x:start:ok:")), "{:?}", probe.lines());
    assert!(matches!(run.state(), RunState::Running {}));
    assert!(seen_for(&log, &Value::num(2.0).unwrap()).is_empty());
    assert!(!run.effective()[&Ident::parse("radio").unwrap()].contains_key(&Key::parse("test.gain").unwrap()));
    trace.assert_extent(&[start_probe::Point::Entered, start_probe::Point::BeforePublish, start_probe::Point::BeforeDispatch]);
    let _ = run.finish_checked();
}

#[test]
fn kc_24a_start_an_end_before_the_check_dispatches_nothing() {
    // #103: the end check after the publication returns before the batch's dispatch.
    let (mut spec, profile) = executor_docs();
    add_schedule(
        &mut spec,
        "radio",
        0,
        serde_json::json!({
            "kind": "update_parameter",
            "target": serde_json::to_value(support::target("radio")).unwrap(),
            "key": "test.gain",
            "value": 7.0
        }),
    );
    let probe = Probe::new();
    let mut assembly = rig(Pacing::FreeRunning).assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).declaring("radio/rx", 1, 1)));
    let slot: Arc<Mutex<Option<Arc<dyn ActionSubmitter>>>> = Arc::new(Mutex::new(None));
    assembly.executors.insert(Ident::parse("exec").unwrap(), Box::new(ProbeExecutor::new("x", &probe).sharing_submitter(slot.clone())));
    let trace = StartTrace::default();
    let recorded = trace.clone();
    let hook_slot = slot.clone();
    let started = start_on_worker(
        &probe,
        move |point, probe| {
            let reading = probe.read().expect("the Run is alive");
            recorded.push(point, reading);
            if point == start_probe::Point::BeforePublish {
                let submitter = hook_slot.lock().unwrap_or_else(|e| e.into_inner()).clone().expect("the Executor was prepared");
                let id = submitter.submit(Action::Abort { cause: StopCause::Abort { message: "test".to_owned() } });
                assert!(id.is_ok(), "{id:?}");
            }
        },
        move || start_spec_run(&spec, &profile, assembly),
    );
    let run = started.finish();
    assert!(!run.effective()[&Ident::parse("radio").unwrap()].contains_key(&Key::parse("test.gain").unwrap()));
    assert_eq!(run.state(), RunState::CleanedUp { termination: Termination::Stopped { cause: StopCause::Abort { message: "test".to_owned() } } });
    assert!(run.finish_checked().termination.undelivered.is_empty());
    trace.assert_extent(&[start_probe::Point::Entered, start_probe::Point::BeforePublish]);
    assert!(!trace.points().iter().any(|(point, _)| *point == start_probe::Point::BeforeDispatch), "{:?}", trace.points());
}
