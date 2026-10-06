//! Spec 19's Kernel amendments for the device-paced classes (KG-1…KG-12), through the
//! real coordinator with the §0 doubles `WallAuthority` and `ThreadedProvider`.
//! Wall-clock margins: a factor of two on durations, counts read while the Run runs.

mod support;

use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration as Wall, Instant};

use ezsdr_kernel::binding::{AdmissionCheck, AdmissionCheckRegistry, CheckStage, Violation};
use ezsdr_kernel::contract::ContractRegistry;
use ezsdr_kernel::coordinator::{Assembly, RunHandle, RunHandleError, connect, start_spec_run};
use ezsdr_kernel::event::{Action, EventKind};
use ezsdr_kernel::id::{ClockDomainId, ResourceId};
use ezsdr_kernel::manifest::Manifest;
use ezsdr_kernel::module_api::{ExecutionClass, ModuleErrorKind, Pacing, Provider, UpdateClass};
use ezsdr_kernel::policy::{EventKindDecl, EventKindRegistry, Reaction};
use ezsdr_kernel::run::{CleanupStep, Lease, RunState, Stage, StopCause, SystemHostClock, Termination};
use ezsdr_kernel::session::{Outcome, SessionAction};
use ezsdr_kernel::spec::{Ident, Key, Namespace, Value};
use ezsdr_kernel::time::{ClockRegistry, TimePoint};
use support::{
    Probe, RecordingSink, SimAuthority, SteppedProvider, TestLinkModule, TestProvider,
    ThreadedProvider, WallAuthority, mref, ns, run_checks, run_kinds, run_registry,
};

// ---------------------------------------------------------------- fixtures

struct Paced {
    root: ClockDomainId,
    relations: Vec<ezsdr_kernel::time::ClockRelation>,
    assembly: Assembly,
}

fn assembly_with(authority: Box<dyn ezsdr_kernel::module_api::Authority>, clocks: Arc<ClockRegistry>) -> Assembly {
    Assembly {
        registry: run_registry(),
        checks: run_checks(false),
        kinds: run_kinds(),
        contracts: ContractRegistry::with_standard_contracts(),
        clocks,
        host_clock: Arc::new(SystemHostClock::new()),
        providers: BTreeMap::new(),
        executors: BTreeMap::new(),
        sinks: BTreeMap::new(),
        authority,
        links: BTreeMap::new(),
        inputs: BTreeMap::new(),
    }
}

fn paced_with(adjust: impl FnOnce(WallAuthority) -> WallAuthority) -> Paced {
    let clocks = Arc::new(ClockRegistry::new());
    let (authority, root) = WallAuthority::new(&clocks, mref("ezsdr.test.provider"));
    let authority = adjust(authority);
    let relations = authority.measured_relations();
    Paced { root, relations, assembly: assembly_with(Box::new(authority), clocks) }
}

fn paced() -> Paced {
    paced_with(|a| a)
}

fn provider(mut assembly: Assembly, name: &str, provider: impl Provider + 'static) -> Assembly {
    assembly.providers.insert(Ident::parse(name).unwrap(), Box::new(provider));
    assembly
}

fn sink(mut assembly: Assembly, sink: RecordingSink, probe: &Probe) -> Assembly {
    assembly.sinks.insert(Ident::parse("rec").unwrap(), Box::new(sink));
    assembly.links.insert(mref("ezsdr.test.link"), Box::new(TestLinkModule::new(probe)));
    assembly
}

fn environment(class: &str, rf: &str) -> serde_json::Value {
    serde_json::json!({
        "ezsdr.time": { "class": class, "start_lead_ns": 10_000_000 },
        "ezsdr.rf_path": { "path": rf }
    })
}

fn spec_one() -> serde_json::Value {
    serde_json::json!({
        "version": 1,
        "requirements": { "vocabularies": [{ "id": "test", "major": 1 }] },
        "resources": {
            "radio": { "kind": "test.device", "requires": { "test.count": { "kind": "eq", "value": 2 } } }
        }
    })
}

fn binding() -> serde_json::Value {
    serde_json::json!({ "module": { "id": "ezsdr.test.provider", "version": { "major": 1, "minor": 0, "patch": 0 } } })
}

fn profile_one() -> serde_json::Value {
    serde_json::json!({
        "version": 1,
        "bindings": { "radio": binding() },
        "authority": "radio",
        "environment": environment("hardware_in_loop", "cabled")
    })
}

/// A Spec Run whose `radio` feeds the output `rec` through a drop-oldest link.
fn output_docs(capacity: u32) -> (serde_json::Value, serde_json::Value) {
    let mut spec = spec_one();
    spec["outputs"] = serde_json::json!([{
        "id": "rec", "kind": "test.capture", "params": {},
        "feed": { "port": { "component": "radio", "port": "rx" }, "policy": "drop_oldest", "capacity": capacity }
    }]);
    let mut profile = profile_one();
    profile["bindings"]["rec"] = serde_json::json!({
        "module": { "id": "ezsdr.test.sink", "version": { "major": 1, "minor": 0, "patch": 0 } }
    });
    profile["placements"] = serde_json::json!({ "links": [{
        "link": { "id": "ezsdr.test.link", "version": { "major": 1, "minor": 0, "patch": 0 } },
        "from": { "component": "radio", "port": "rx" },
        "to": { "component": "rec", "port": "in" }
    }] });
    (spec, profile)
}

/// The Session form of `output_docs`: the Sink binding carries the feed (SB-22c).
fn session_sink_profile() -> serde_json::Value {
    let (_, mut profile) = output_docs(64);
    profile["bindings"]["rec"]["feed"] = serde_json::json!({
        "port": { "component": "radio", "port": "rx" }, "policy": "drop_oldest", "capacity": 64
    });
    profile
}

fn manifest_of(run: RunHandle) -> Manifest {
    run.finish()
}

fn failure(manifest: &Manifest) -> String {
    manifest.sections[&ns("ezsdr.failure")]["reason"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn running(run: &RunHandle) {
    assert!(matches!(run.state(), RunState::Running {}), "{:?}", run.state());
}

fn after(run: &RunHandle, wall: Wall) -> TimePoint {
    let now = run.now();
    TimePoint::new(now.domain, now.ticks + wall.as_nanos() as i64)
}

fn count(probe: &Probe, prefix: &str) -> usize {
    probe.with_prefix(prefix).len()
}

fn stepped_on(probe: &Probe) -> Vec<String> {
    probe
        .with_prefix("rec:step_on:")
        .into_iter()
        .map(|line| line.trim_start_matches("rec:step_on:").to_owned())
        .collect()
}

fn kind(s: &str) -> EventKind {
    EventKind::parse(s).unwrap()
}

// ---------------------------------------------------------------- KG-1

#[test]
fn kg_01_a_device_paced_run_reaches_running() {
    let probe = Probe::new();
    let rig = paced();
    let assembly = provider(rig.assembly, "radio", ThreadedProvider::new("radio", "radio", &probe));
    let run = start_spec_run(&spec_one(), &profile_one(), assembly).unwrap();
    running(&run);
    let manifest = manifest_of(run);
    assert_eq!(manifest.run.execution_class, ExecutionClass::HardwareInLoop);
    assert!(!manifest.run.deterministic);
    let calls: Vec<_> = probe
        .with_prefix("radio:")
        .into_iter()
        .filter(|line| ["radio:prepare", "radio:arm", "radio:start", "radio:stop", "radio:cleanup"].contains(&line.as_str()))
        .collect();
    assert_eq!(calls, ["radio:prepare", "radio:arm", "radio:start", "radio:stop", "radio:cleanup"]);
}

#[test]
fn kg_01_the_hardware_class_reaches_running() {
    let probe = Probe::new();
    let rig = paced();
    let mut profile = profile_one();
    profile["environment"] = environment("hardware", "over_the_air");
    let assembly = provider(rig.assembly, "radio", ThreadedProvider::new("radio", "radio", &probe));
    let run = start_spec_run(&spec_one(), &profile, assembly).unwrap();
    running(&run);
    assert_eq!(manifest_of(run).run.execution_class, ExecutionClass::Hardware);
}

#[test]
fn kg_01_a_stepped_provider_is_refused_in_a_device_paced_class() {
    let probe = Probe::new();
    let rig = paced();
    let assembly = provider(
        rig.assembly,
        "radio",
        SteppedProvider::new("p", TestProvider::new("radio", 2), &probe),
    );
    let run = start_spec_run(&spec_one(), &profile_one(), assembly).unwrap();
    assert_eq!(
        run.state(),
        RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Plan } }
    );
    let manifest = manifest_of(run);
    assert!(failure(&manifest).starts_with("KC-2a: radio is a stepped Provider"), "{}", failure(&manifest));
    assert!(!probe.lines().iter().any(|line| line.starts_with("p:prepare")));
}

/// A Spec Run with one component `c1` on an Island of the Executor `exec`.
fn executor_docs() -> (serde_json::Value, serde_json::Value) {
    let mut spec = spec_one();
    let mut component = support::recorder_component(support::cf32());
    component.id = Ident::parse("c1").unwrap();
    component.implementation.id = "c1".to_owned();
    // MA-39: an Island with a real-time policy needs a budget on every component.
    component.timing.budget = Some(
        ezsdr_kernel::time::RelativeBudget::new(ezsdr_kernel::time::Duration::new(
            ClockDomainId::HOST_MONOTONIC,
            1_000_000,
        ))
        .unwrap(),
    );
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

fn island_docs(field: &str, value: serde_json::Value) -> (serde_json::Value, serde_json::Value) {
    let (spec, mut profile) = executor_docs();
    profile["placements"]["islands"][0][field] = value;
    (spec, profile)
}

#[test]
fn kg_01_an_island_with_an_rt_policy_is_refused() {
    for (field, value) in [
        ("rt_policy", serde_json::json!({ "sched": "fifo", "priority": 10 })),
        ("affinity", serde_json::json!([1])),
    ] {
        let probe = Probe::new();
        let (spec, profile) = island_docs(field, value);
        let mut assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe));
        assembly.executors.insert(
            Ident::parse("exec").unwrap(),
            Box::new(support::ProbeExecutor::new("x", &probe)),
        );
        let run = start_spec_run(&spec, &profile, assembly).unwrap();
        assert_eq!(
            run.state(),
            RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Plan } },
            "{field}"
        );
        let reason = failure(&manifest_of(run));
        assert_eq!(
            reason,
            format!("KC-2a: island_0 declares {field}, which a device-paced class does not apply before Phase 10")
        );
    }
}

// ---------------------------------------------------------------- KG-2

fn publishing_spec_run(probe: &Probe, sink_double: RecordingSink) -> RunHandle {
    let (spec, profile) = output_docs(64);
    let assembly = provider(
        paced().assembly,
        "radio",
        ThreadedProvider::new("radio", "radio", probe).publishing(100, Wall::from_millis(1)),
    );
    let run = start_spec_run(&spec, &profile, sink(assembly, sink_double, probe)).unwrap();
    running(&run);
    run
}

#[test]
fn kg_02_the_data_thread_drains_a_sink_while_no_call_runs() {
    let probe = Probe::new();
    let run = publishing_spec_run(&probe, RecordingSink::new("rec", &probe));
    std::thread::sleep(Wall::from_millis(100));
    let first = count(&probe, "rec:block:");
    std::thread::sleep(Wall::from_millis(100));
    let second = count(&probe, "rec:block:");
    assert!(second >= first + 50, "{first} then {second}");
    let manifest = manifest_of(run);
    assert_eq!(manifest.sections[&ns("ezsdr.links")][0]["drops"], 0);
}

#[test]
fn kg_02_every_step_before_finish_runs_on_the_data_thread() {
    let probe = Probe::new();
    let run = publishing_spec_run(&probe, RecordingSink::new("rec", &probe).recording_threads());
    std::thread::sleep(Wall::from_millis(50));
    let before = stepped_on(&probe);
    let _ = manifest_of(run);
    assert!(!before.is_empty());
    assert!(before.iter().all(|thread| *thread == before[0]), "{before:?}");
    assert!(before[0].ends_with(":ezsdr-data"), "{}", before[0]);
    let test = format!("{:?}", std::thread::current().id());
    assert!(!before[0].starts_with(&format!("{test}:")));
}

fn simulation_run(probe: &Probe) -> (RunHandle, ClockDomainId) {
    let clocks = Arc::new(ClockRegistry::new());
    let (authority, root) = SimAuthority::new(&clocks, mref("ezsdr.test.provider"), Pacing::FreeRunning);
    let (spec, mut profile) = output_docs(64);
    profile["environment"] = serde_json::json!({});
    let assembly = provider(
        assembly_with(Box::new(authority), clocks),
        "radio",
        SteppedProvider::new("p", TestProvider::new("radio", 2), probe).publishing_every(10),
    );
    let sink_double = RecordingSink::new("rec", probe).recording_threads();
    (start_spec_run(&spec, &profile, sink(assembly, sink_double, probe)).unwrap(), root)
}

#[test]
fn kg_02_the_simulation_class_steps_on_the_callers_thread() {
    let probe = Probe::new();
    let (mut run, root) = simulation_run(&probe);
    let _ = run.advance_to(TimePoint::new(root, 100));
    let before = stepped_on(&probe);
    let _ = manifest_of(run);
    let test = format!("{:?}", std::thread::current().id());
    assert!(!before.is_empty());
    assert!(before.iter().all(|thread| thread.starts_with(&format!("{test}:"))), "{before:?}");
}

#[test]
fn kg_02_the_simulation_class_starts_no_thread() {
    // GZ-4: a Simulation Run never reaches `coordinator/paced.rs`, whose threads are
    // the only ones named `ezsdr-data` and `ezsdr-bounded`.
    let probe = Probe::new();
    let (mut run, root) = simulation_run(&probe);
    let _ = run.advance_to(TimePoint::new(root, 100));
    let _ = manifest_of(run);
    let threads = stepped_on(&probe);
    assert!(!threads.is_empty());
    assert!(
        threads.iter().all(|t| !t.ends_with(":ezsdr-data") && !t.ends_with(":ezsdr-bounded")),
        "{threads:?}"
    );
}

#[test]
fn kg_02_a_sink_that_always_progresses_does_not_livelock() {
    let probe = Probe::new();
    let run = publishing_spec_run(&probe, RecordingSink::new("rec", &probe).always_progressing());
    std::thread::sleep(Wall::from_millis(200));
    running(&run);
    let livelock = kind(EventKind::STEP_LIVELOCK);
    assert!(!run.events(0).iter().any(|event| event.kind == livelock));
    assert!(count(&probe, "rec:step:") > 1_000, "{}", count(&probe, "rec:step:"));
    let _ = manifest_of(run);
}

#[test]
fn kg_02_a_failed_sink_is_not_stepped_again() {
    let probe = Probe::new();
    let rig = paced();
    let (mut spec, profile) = output_docs(64);
    spec["policies"] = serde_json::json!({ "failure": { "DEVICE_LOST": "continue" } });
    let assembly = provider(rig.assembly, "radio", ThreadedProvider::new("radio", "radio", &probe));
    let at = 20_000_000 + Instant::now().elapsed().as_nanos() as i64;
    let sink_double = RecordingSink::new("rec", &probe).failing_step_at(at, ModuleErrorKind::DeviceLost);
    let run = start_spec_run(&spec, &profile, sink(assembly, sink_double, &probe)).unwrap();
    running(&run);
    std::thread::sleep(Wall::from_millis(100));
    let first = count(&probe, "rec:step:");
    std::thread::sleep(Wall::from_millis(100));
    assert_eq!(count(&probe, "rec:step:"), first, "stepped again after its failure");
    let lost = kind(EventKind::DEVICE_LOST);
    let sink_source = ResourceId::parse("sink/rec").unwrap();
    let reports = run.events(0).into_iter().filter(|e| e.kind == lost && e.source == sink_source).count();
    assert_eq!(reports, 1);
    running(&run);
    let _ = manifest_of(run);
}

fn session(assembly: Assembly, profile: &serde_json::Value) -> RunHandle {
    let run = connect(profile, assembly, Lease::attached()).unwrap();
    running(&run);
    run
}

#[test]
fn kg_02_wait_for_returns_when_the_data_thread_delivers() {
    let probe = Probe::new();
    let assembly = provider(
        paced().assembly,
        "radio",
        ThreadedProvider::new("radio", "radio", &probe).marking("test.MARK", Wall::from_millis(30)),
    );
    let mut run = session(assembly, &profile_one());
    let horizon = after(&run, Wall::from_secs(5));
    let begun = Instant::now();
    let found = run.wait_for(&[kind("test.MARK")], 0, horizon).unwrap();
    assert!(found.is_some());
    assert!(begun.elapsed() < Wall::from_secs(1), "{:?}", begun.elapsed());
    assert!(run.now().ticks < horizon.ticks - 3_000_000_000);
    let _ = manifest_of(run);
}

fn stopping_kinds() -> EventKindRegistry {
    let mut kinds = run_kinds();
    for name in ["test.first", "test.second"] {
        kinds
            .register(
                Some(ns("test")),
                EventKindDecl { kind: kind(name), default: Reaction::Stop, severity: ezsdr_kernel::event::Severity::Info },
            )
            .unwrap();
    }
    kinds
}

#[test]
fn kg_02_the_first_delivered_stopping_event_is_the_termination_cause() {
    // Both marks are drained by whichever thread gets there first; KC-31's one lock
    // keeps `delivered` and the reactions in one order either way.
    let mut registry_vocabulary = support::test_vocabulary();
    for name in ["test.first", "test.second"] {
        registry_vocabulary.event_kinds.push(EventKindDecl {
            kind: kind(name),
            default: Reaction::Stop,
            severity: ezsdr_kernel::event::Severity::Info,
        });
    }
    for _ in 0..50 {
        let probe = Probe::new();
        let mut rig = paced();
        rig.assembly.kinds = stopping_kinds();
        rig.assembly.registry = registry_with(registry_vocabulary.clone());
        let assembly = provider(
            rig.assembly,
            "a",
            ThreadedProvider::new("a", "a", &probe).marking("test.first", Wall::from_millis(20)),
        );
        let assembly = provider(
            assembly,
            "b",
            ThreadedProvider::new("b", "b", &probe).marking("test.second", Wall::from_millis(20)),
        );
        let profile = serde_json::json!({
            "version": 1,
            "bindings": {
                "a": { "module": binding()["module"], "selector": { "slot": "a" } },
                "b": { "module": binding()["module"], "selector": { "slot": "b" } }
            },
            "authority": "a",
            "environment": environment("hardware_in_loop", "cabled")
        });
        let mut run = session(assembly, &profile);
        let never = kind("test.custom");
        let begun = Instant::now();
        while begun.elapsed() < Wall::from_secs(2) {
            let horizon = after(&run, Wall::from_millis(1));
            if matches!(run.wait_for(std::slice::from_ref(&never), 0, horizon), Err(RunHandleError::Ended { .. })) {
                break;
            }
        }
        let manifest = manifest_of(run);
        let first = manifest
            .events
            .delivered
            .iter()
            .find(|event| event.kind == kind("test.first") || event.kind == kind("test.second"))
            .expect("a stopping event was delivered")
            .kind
            .clone();
        assert_eq!(
            manifest.termination.reason,
            Termination::Stopped { cause: StopCause::Policy { kind: first } }
        );
    }
}

fn registry_with(vocabulary: ezsdr_kernel::module_api::VocabularyDescriptor) -> ezsdr_kernel::module_api::ModuleRegistry {
    let mut registry = ezsdr_kernel::module_api::ModuleRegistry::new();
    registry.register_vocabulary(vocabulary).unwrap();
    for (descriptor, factories) in [
        (
            support::test_provider_descriptor(),
            ezsdr_kernel::module_api::Factories { provider: true, authority: true, ..Default::default() },
        ),
        (
            support::test_sink_descriptor(),
            ezsdr_kernel::module_api::Factories { sink: true, ..Default::default() },
        ),
    ] {
        registry.register(descriptor, factories).unwrap();
    }
    registry
}

#[test]
fn kg_02_every_delivery_wakes_a_waiting_call() {
    let probe = Probe::new();
    let mut double = ThreadedProvider::new("radio", "radio", &probe);
    for i in 1..=50 {
        double = double.marking("test.MARK", Wall::from_millis(10 * i));
    }
    let mut run = session(provider(paced().assembly, "radio", double), &profile_one());
    let mark = kind("test.MARK");
    let mut from = 0;
    for _ in 0..50 {
        let horizon = after(&run, Wall::from_secs(5));
        let begun = Instant::now();
        let found = run.wait_for(std::slice::from_ref(&mark), from, horizon).unwrap();
        let index = found.expect("each mark is found before the horizon");
        assert!(begun.elapsed() < Wall::from_millis(200), "{:?}", begun.elapsed());
        from = index + 1;
    }
    let _ = manifest_of(run);
}

#[test]
fn kg_02_a_wake_that_fires_before_schedule_returns_still_clears() {
    // KC-46a: the pending generation is stored before `schedule`, so a callback that
    // runs before `schedule` returns clears its own wake (Review L, P1-6).
    let probe = Probe::new();
    let mut double = ThreadedProvider::new("radio", "radio", &probe);
    for i in 1..=20 {
        double = double.marking("test.MARK", Wall::from_millis(10 * i));
    }
    let rig = paced_with(WallAuthority::firing_before_schedule_returns);
    let mut run = session(provider(rig.assembly, "radio", double), &profile_one());
    let mark = kind("test.MARK");
    let mut from = 0;
    for _ in 0..20 {
        let horizon = after(&run, Wall::from_secs(2));
        let begun = Instant::now();
        let found = run.wait_for(std::slice::from_ref(&mark), from, horizon).unwrap();
        let index = found.expect("each mark is found before the horizon");
        assert!(begun.elapsed() < Wall::from_millis(200), "{:?}", begun.elapsed());
        from = index + 1;
    }
    let _ = manifest_of(run);
}

#[test]
fn kg_02_a_detached_lease_expires_while_no_call_runs() {
    let probe = Probe::new();
    let assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe));
    let lease = Lease::detached(50, false, "token", &SystemHostClock::new()).unwrap();
    let mut run = connect(&profile_one(), assembly, lease).unwrap();
    running(&run);
    run.disconnect();
    assert!(probe.wait_for("radio:stop", Wall::from_millis(500)));
    let manifest = manifest_of(run);
    assert_eq!(
        manifest.termination.reason,
        Termination::Stopped { cause: StopCause::LeaseExpiry {} }
    );
}

fn detached_session(probe: &Probe, ttl_ms: u64) -> RunHandle {
    let assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", probe));
    let lease = Lease::detached(ttl_ms, true, "token", &SystemHostClock::new()).unwrap();
    let run = connect(&profile_one(), assembly, lease).unwrap();
    running(&run);
    run
}

fn provider_stopped(probe: &Probe) -> bool {
    probe.lines().iter().any(|line| line == "radio:stop")
}

#[test]
fn kc_36_renew_moves_the_deadline_the_data_thread_checks() {
    // The exit review's KC-36 clause, for `Renew`: renewed every 50 ms for 600 ms, a
    // 200 ms Lease outlives its first deadline, and then expires with no call running.
    let probe = Probe::new();
    let mut run = detached_session(&probe, 200);
    run.disconnect();
    let begun = Instant::now();
    while begun.elapsed() < Wall::from_millis(600) {
        std::thread::sleep(Wall::from_millis(50));
        let entry = run.submit(SessionAction::Renew {}, None).unwrap();
        assert!(admitted(&entry), "{entry:?}");
    }
    assert!(!provider_stopped(&probe), "{:?}", probe.lines());
    running(&run);
    assert!(probe.wait_for("radio:stop", Wall::from_secs(1)));
    assert_eq!(
        manifest_of(run).termination.reason,
        Termination::Stopped { cause: StopCause::LeaseExpiry {} }
    );
}

#[test]
fn kc_36_adopt_clears_the_deadline_the_data_thread_checks() {
    // The exit review's KC-36 clause, for `Adopt`: adopted at once, a 100 ms Lease
    // whose TTL the disconnect started does not expire in the next 300 ms.
    let probe = Probe::new();
    let mut run = detached_session(&probe, 100);
    run.disconnect();
    let entry = run.submit(SessionAction::Adopt { token: "token".to_owned() }, None).unwrap();
    assert!(admitted(&entry), "{entry:?}");
    std::thread::sleep(Wall::from_millis(300));
    assert!(!provider_stopped(&probe), "{:?}", probe.lines());
    running(&run);
    assert_eq!(
        manifest_of(run).termination.reason,
        Termination::Stopped { cause: StopCause::Client {} }
    );
}

// ---------------------------------------------------------------- KG-3

fn lost_device_session(probe: &Probe) -> RunHandle {
    let assembly = provider(
        paced().assembly,
        "radio",
        ThreadedProvider::new("radio", "radio", probe)
            .publishing(100, Wall::from_millis(1))
            .losing_device(Wall::from_millis(20)),
    );
    let assembly = sink(assembly, RecordingSink::new("rec", probe), probe);
    session(assembly, &session_sink_profile())
}

#[test]
fn kg_03_a_fatal_event_stops_the_providers_without_a_client_call() {
    let probe = Probe::new();
    let run = lost_device_session(&probe);
    assert!(probe.wait_for("radio:stop", Wall::from_millis(500)));
    let manifest = manifest_of(run);
    assert_eq!(
        manifest.termination.reason,
        Termination::Stopped { cause: StopCause::Policy { kind: kind(EventKind::DEVICE_LOST) } }
    );
    assert_eq!(count(&probe, "radio:stop"), 1);
}

#[test]
fn kc_29_an_end_the_data_thread_requested_is_cleaned_up_at_the_next_call() {
    // The exit review's KC-29 clause (as KG-3 amends it): after an end the data thread
    // requested while no call ran, `state()` reads `Running` and `events()` already
    // shows the cause, until the next call cleans up.
    let probe = Probe::new();
    let mut run = lost_device_session(&probe);
    assert!(probe.wait_for("radio:stop", Wall::from_millis(500)));
    std::thread::sleep(Wall::from_millis(50));
    running(&run);
    let lost = kind(EventKind::DEVICE_LOST);
    let radio = ResourceId::parse("radio").unwrap();
    assert!(run.events(0).iter().any(|e| e.kind == lost && e.source == radio), "{:?}", run.events(0));
    let begun = Instant::now();
    let result = run.advance_to(after(&run, Wall::from_secs(5)));
    assert!(begun.elapsed() < Wall::from_secs(1), "{:?}", begun.elapsed());
    let termination = Termination::Stopped { cause: StopCause::Policy { kind: lost } };
    assert_eq!(result, Err(RunHandleError::Ended { termination: termination.clone() }));
    assert_eq!(run.state(), RunState::CleanedUp { termination });
}

#[test]
fn kg_03_an_abort_stops_the_data_thread() {
    let probe = Probe::new();
    let run = lost_device_session(&probe);
    assert!(probe.wait_for("radio:stop", Wall::from_millis(500)));
    std::thread::sleep(Wall::from_millis(50));
    let _ = manifest_of(run);
    let lines = probe.lines();
    let stop = lines.iter().position(|line| line == "radio:stop").unwrap();
    assert!(
        !lines[stop..].iter().any(|line| line.starts_with("rec:step:")),
        "{:?}",
        &lines[stop..]
    );
}

#[test]
fn kg_03_a_step_done_early_is_not_repeated() {
    let probe = Probe::new();
    let (mut spec, mut profile) = output_docs(64);
    spec["resources"]["other"] = spec["resources"]["radio"].clone();
    profile["bindings"]["radio"]["selector"] = serde_json::json!({ "slot": "radio" });
    profile["bindings"]["other"] = serde_json::json!({ "module": binding()["module"], "selector": { "slot": "other" } });
    let assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe));
    let assembly = provider(assembly, "other", ThreadedProvider::new("other", "other", &probe));
    let at = 20_000_000;
    let sink_double = RecordingSink::new("rec", &probe).failing_step_at(at, ModuleErrorKind::Rejected);
    let run = start_spec_run(&spec, &profile, sink(assembly, sink_double, &probe)).unwrap();
    running(&run);
    assert!(probe.wait_for("radio:stop", Wall::from_secs(1)));
    assert!(probe.wait_for("other:stop", Wall::from_secs(1)));
    let manifest = manifest_of(run);
    assert_eq!(manifest.termination.reason, Termination::Failed { stage: Stage::Run });
    for name in ["radio", "other"] {
        assert_eq!(count(&probe, &format!("{name}:stop")), 1, "{name}");
        assert_eq!(count(&probe, &format!("{name}:cleanup")), 1, "{name}");
    }
    assert!(
        !manifest.termination.cleanup_failures.iter().any(|f| f.step == CleanupStep::StopTx),
        "{:?}",
        manifest.termination.cleanup_failures
    );
}

#[test]
fn kg_03_step_3_runs_a_final_round_over_the_sinks() {
    let probe = Probe::new();
    let (spec, profile) = output_docs(64);
    let assembly = provider(
        paced().assembly,
        "radio",
        ThreadedProvider::new("radio", "radio", &probe)
            .publishing(100, Wall::from_millis(1))
            .with_stop_tail(2),
    );
    let sink_double = RecordingSink::new("rec", &probe).recording_threads();
    let run = start_spec_run(&spec, &profile, sink(assembly, sink_double, &probe)).unwrap();
    running(&run);
    std::thread::sleep(Wall::from_millis(20));
    let data = stepped_on(&probe)[0].clone();
    let _ = manifest_of(run);
    let lines = probe.lines();
    let stop = lines.iter().position(|line| line == "radio:stop").unwrap();
    let sink_stop = lines.iter().position(|line| line == "rec:stop:Orderly").unwrap();
    assert!(
        lines[stop..sink_stop]
            .iter()
            .any(|line| line.starts_with("rec:step_on:") && !line.ends_with(&data)),
        "{:?}",
        &lines[stop..sink_stop]
    );
    let last_tail = lines.iter().rposition(|line| line == "radio:tail").unwrap();
    assert!(lines[last_tail..sink_stop].iter().any(|line| line.starts_with("rec:block:")));
}

#[test]
fn kc_46b_under_orderly_the_data_thread_steps_on_after_its_stop() {
    // The exit review's KC-46b clause: under `orderly` (here an expired Lease, KC-36)
    // the data thread goes on stepping the Sinks after its own RS-6 steps 1-2, so the
    // tail the Provider delivers in step 2 reaches the Sink with no call running, and
    // the Run stays `Running` until the control thread's cleanup.
    let probe = Probe::new();
    let assembly = provider(
        paced().assembly,
        "radio",
        ThreadedProvider::new("radio", "radio", &probe)
            .publishing(100, Wall::from_millis(1))
            .with_stop_tail(2),
    );
    let assembly = sink(assembly, RecordingSink::new("rec", &probe).recording_threads(), &probe);
    let lease = Lease::detached(50, false, "token", &SystemHostClock::new()).unwrap();
    let mut run = connect(&session_sink_profile(), assembly, lease).unwrap();
    running(&run);
    run.disconnect();
    assert!(probe.wait_for_count("radio:tail", 2, Wall::from_secs(1)));
    std::thread::sleep(Wall::from_millis(100));
    running(&run);
    let lines = probe.lines();
    let last_tail = lines.iter().rposition(|line| line == "radio:tail").unwrap();
    let after_stop = &lines[last_tail..];
    assert!(after_stop.iter().any(|line| line.starts_with("rec:block:")), "{after_stop:?}");
    let steps: Vec<_> = after_stop.iter().filter(|line| line.starts_with("rec:step_on:")).collect();
    assert!(steps.len() >= 2, "{after_stop:?}");
    assert!(steps.iter().all(|line| line.ends_with(":ezsdr-data")), "{steps:?}");
    assert!(!lines.iter().any(|line| line.starts_with("rec:stop:")), "{lines:?}");
    let manifest = manifest_of(run);
    assert_eq!(
        manifest.termination.reason,
        Termination::Stopped { cause: StopCause::LeaseExpiry {} }
    );
    assert!(
        manifest
            .run
            .transitions
            .iter()
            .any(|t| matches!(t.state, RunState::Stopping { mode: ezsdr_kernel::run::CleanupMode::Orderly })),
        "{:?}",
        manifest.run.transitions
    );
}

#[test]
fn kg_03_a_cancelled_horizon_wakes_the_control_loop() {
    let probe = Probe::new();
    let assembly = provider(
        paced().assembly,
        "radio",
        ThreadedProvider::new("radio", "radio", &probe).losing_device(Wall::from_millis(20)),
    );
    let mut run = start_spec_run(&spec_one(), &profile_one(), assembly).unwrap();
    running(&run);
    let begun = Instant::now();
    let result = run.run_until_end(after(&run, Wall::from_secs(5)));
    assert!(matches!(result, Err(RunHandleError::Ended { .. })), "{result:?}");
    assert!(begun.elapsed() < Wall::from_secs(1), "{:?}", begun.elapsed());
    let _ = manifest_of(run);
}

#[test]
fn kg_03_dropping_a_live_device_paced_handle_cleans_up() {
    let probe = Probe::new();
    let assembly = provider(
        paced().assembly,
        "radio",
        ThreadedProvider::new("radio", "radio", &probe).publishing(100, Wall::from_millis(1)),
    );
    let assembly = sink(assembly, RecordingSink::new("rec", &probe), &probe);
    let run = session(assembly, &session_sink_profile());
    std::thread::sleep(Wall::from_millis(20));
    drop(run);
    let lines = probe.lines();
    assert!(lines.iter().any(|line| line == "radio:stop"));
    assert!(lines.iter().any(|line| line == "radio:cleanup"));
    let cleanup = lines.iter().position(|line| line == "rec:cleanup").expect("the Sink was cleaned up");
    assert!(!lines[cleanup..].iter().any(|line| line.starts_with("rec:step:")));
}

// ---------------------------------------------------------------- KG-4

fn gain(value: f64) -> SessionAction {
    SessionAction::SetParameter {
        target: ResourceId::parse("radio").unwrap(),
        key: Key::parse("test.gain").unwrap(),
        value: Value::Num(value),
    }
}

fn admitted(entry: &ezsdr_kernel::session::LogEntry) -> bool {
    matches!(entry.outcome, Outcome::Admitted { .. })
}

#[test]
fn kg_04_a_session_call_returns_after_the_provider_has_finished() {
    let probe = Probe::new();
    let assembly = provider(
        paced().assembly,
        "radio",
        ThreadedProvider::new("radio", "radio", &probe).with_action_delay(Wall::from_millis(30)),
    );
    let mut run = session(assembly, &profile_one());
    let entry = run.submit(gain(3.0), None).unwrap();
    assert!(admitted(&entry), "{entry:?}");
    assert!(probe.lines().iter().any(|line| line == "radio:finished:UpdateParameter"));
    let _ = manifest_of(run);
}

#[test]
fn kg_04_every_action_of_one_call_is_finished() {
    // KC-21a: two Actions dispatched to one threaded instance at one instant; the call
    // that dispatched them returns only when both are finished (Review L, P1-5).
    let probe = Probe::new();
    let (mut spec, profile) = output_docs(64);
    let radio = serde_json::to_value(ResourceId::parse("radio").unwrap()).unwrap();
    let entry = |value: f64| serde_json::json!({
        "at": { "clock": "radio", "offset_ticks": 0 },
        "action": { "kind": "update_parameter", "target": radio, "key": "test.gain", "value": value, "class": "hardware_timed" }
    });
    spec["schedule"] = serde_json::json!([entry(1.0), entry(2.0)]);
    let assembly = provider(
        paced().assembly,
        "radio",
        ThreadedProvider::new("radio", "radio", &probe)
            .publishing(100, Wall::from_millis(1))
            .with_action_delay(Wall::from_millis(30)),
    );
    let mut run = start_spec_run(&spec, &profile, sink(assembly, RecordingSink::new("rec", &probe), &probe)).unwrap();
    running(&run);
    let t0 = run.start_instant().unwrap();
    run.advance_to(TimePoint::new(t0.domain, t0.ticks + 1)).unwrap();
    assert_eq!(count(&probe, "radio:finished:UpdateParameter"), 2, "{:?}", probe.lines());
    let _ = manifest_of(run);
}

#[test]
fn kg_04_the_next_admission_sees_the_state_the_previous_action_made() {
    for _ in 0..20 {
        let probe = Probe::new();
        let assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe));
        let mut run = session(assembly, &profile_one());
        let clock = run
            .submit(
                SessionAction::SetParameter {
                    target: ResourceId::parse("radio").unwrap(),
                    key: Key::parse("test.tx_clock").unwrap(),
                    value: Value::Int(10),
                },
                None,
            )
            .unwrap();
        assert!(admitted(&clock), "{clock:?}");
        let burst = run
            .submit(
                SessionAction::Vocabulary {
                    ns: ns("test"),
                    verb: Ident::parse("start_repeat").unwrap(),
                    target: ResourceId::parse("radio/tx").unwrap(),
                    at: None,
                    params: BTreeMap::new(),
                },
                Some(&[0u8; 80]),
            )
            .unwrap();
        assert!(admitted(&burst), "{burst:?}");
        let _ = manifest_of(run);
    }
}

#[test]
fn kg_04_a_provider_that_never_finishes_fails_the_run() {
    let probe = Probe::new();
    let assembly = provider(
        paced().assembly,
        "radio",
        ThreadedProvider::new("radio", "radio", &probe).never_finishing(),
    );
    let mut run = session(assembly, &profile_one());
    let begun = Instant::now();
    let _ = run.submit(gain(1.0), None);
    let waited = begun.elapsed();
    assert!(waited >= Wall::from_millis(4_500) && waited < Wall::from_secs(10), "{waited:?}");
    let manifest = manifest_of(run);
    assert_eq!(manifest.termination.reason, Termination::Failed { stage: Stage::Run });
    assert!(failure(&manifest).starts_with("KC-21a: radio did not finish"), "{}", failure(&manifest));
}

#[test]
fn kc_21a_an_end_requested_during_the_wait_ends_it() {
    // The exit review's KC-21a clause, its re-check: the Provider never finishes the
    // Action and loses its device 100 ms after the start; the end the data thread
    // requests ends the wait long before the 5 s budget.
    let probe = Probe::new();
    let assembly = provider(
        paced().assembly,
        "radio",
        ThreadedProvider::new("radio", "radio", &probe)
            .never_finishing()
            .losing_device(Wall::from_millis(100)),
    );
    let mut run = session(assembly, &profile_one());
    let begun = Instant::now();
    let _ = run.submit(gain(1.0), None);
    assert!(begun.elapsed() < Wall::from_secs(1), "{:?}", begun.elapsed());
    assert!(probe.lines().iter().any(|line| line == "radio:took:UpdateParameter"));
    let manifest = manifest_of(run);
    assert_eq!(
        manifest.termination.reason,
        Termination::Stopped { cause: StopCause::Policy { kind: kind(EventKind::DEVICE_LOST) } }
    );
}

#[test]
fn kc_21a_a_module_s_own_submission_is_not_waited_for() {
    // The exit review's KC-21a clause, its scope: an Action an Executor submits from
    // its step (MA-14a) is not waited for. The Provider takes it and never finishes
    // it; the data thread goes on stepping the Executor all the same.
    let probe = Probe::new();
    let (spec, profile) = executor_docs();
    let mut assembly = provider(
        paced().assembly,
        "radio",
        ThreadedProvider::new("radio", "radio", &probe).never_finishing(),
    );
    let action = Action::UpdateParameter {
        target: ResourceId::parse("radio").unwrap(),
        key: Key::parse("test.gain").unwrap(),
        value: Value::Num(3.0),
        class: UpdateClass::HardwareTimed,
        at: None,
    };
    assembly.executors.insert(
        Ident::parse("exec").unwrap(),
        Box::new(support::ProbeExecutor::new("x", &probe).submitting(action)),
    );
    let run = start_spec_run(&spec, &profile, assembly).unwrap();
    running(&run);
    assert!(probe.wait_for("radio:took:UpdateParameter", Wall::from_millis(500)));
    std::thread::sleep(Wall::from_millis(100));
    running(&run);
    let lines = probe.lines();
    let submitted = lines
        .iter()
        .position(|line| line.starts_with("x:submit:"))
        .expect("the submission returned");
    assert!(lines[submitted].starts_with("x:submit:ok:"), "{}", lines[submitted]);
    let later = lines[submitted..].iter().filter(|line| line.starts_with("x:step:")).count();
    assert!(later >= 2, "{:?}", &lines[submitted..]);
    let manifest = manifest_of(run);
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Client {} });
}

/// The runtime check of `kg_04_no_action_is_dispatched_after_the_freeze`: on its first
/// call it opens the loss gate and waits up to 200 ms for the Provider's `stop` to
/// begin, holding the admission inside the admission lock.
struct Gate {
    section: Namespace,
    stages: Vec<CheckStage>,
    loss: Arc<(Mutex<bool>, Condvar)>,
    probe: Probe,
    fired: Mutex<bool>,
}

impl AdmissionCheck for Gate {
    fn section(&self) -> &Namespace {
        &self.section
    }
    fn stages(&self) -> &[CheckStage] {
        &self.stages
    }
    fn check(
        &self,
        _section: &serde_json::Value,
        _effective: &BTreeMap<Ident, BTreeMap<Key, Value>>,
        _proposed: &BTreeMap<Ident, BTreeMap<Key, Value>>,
        _stage: CheckStage,
    ) -> Vec<Violation> {
        let mut fired = self.fired.lock().unwrap();
        if !*fired {
            *fired = true;
            *self.loss.0.lock().unwrap() = true;
            self.loss.1.notify_all();
            let _ = self.probe.wait_for("radio:stop", Wall::from_millis(200));
        }
        Vec::new()
    }
}

#[test]
fn kg_04_no_action_is_dispatched_after_the_freeze() {
    let probe = Probe::new();
    let loss = Arc::new((Mutex::new(false), Condvar::new()));
    let mut rig = paced();
    let mut checks = AdmissionCheckRegistry::new();
    checks.register(Arc::new(Gate {
        section: ns("test.gate"),
        stages: vec![CheckStage::Runtime],
        loss: loss.clone(),
        probe: probe.clone(),
        fired: Mutex::new(false),
    }));
    rig.assembly.checks = checks;
    let assembly = provider(
        rig.assembly,
        "radio",
        ThreadedProvider::new("radio", "radio", &probe)
            .losing_device_on(loss)
            .draining_after_stop(Wall::from_millis(50)),
    );
    let mut profile = profile_one();
    profile["environment"]["test.gate"] = serde_json::json!({});
    let mut run = session(assembly, &profile);
    let _ = run.submit(gain(2.0), None);
    let _ = manifest_of(run);
    assert!(
        !probe.lines().iter().any(|line| line.starts_with("radio:after_stop:")),
        "{:?}",
        probe.lines()
    );
}

#[test]
fn kg_04_a_simulation_session_does_not_wait() {
    // A stepped Provider receives its Actions in KC-21's round, after `submit`
    // dispatched them: a KC-21a wait before that round would time out and fail the Run.
    let probe = Probe::new();
    let clocks = Arc::new(ClockRegistry::new());
    let (authority, _) = SimAuthority::new(&clocks, mref("ezsdr.test.provider"), Pacing::FreeRunning);
    let assembly = provider(
        assembly_with(Box::new(authority), clocks),
        "radio",
        SteppedProvider::new("p", TestProvider::new("radio", 2), &probe),
    );
    let mut profile = profile_one();
    profile["environment"] = serde_json::json!({});
    let mut run = session(assembly, &profile);
    let begun = Instant::now();
    let entry = run.submit(gain(4.0), None).unwrap();
    assert!(admitted(&entry), "{entry:?}");
    assert!(begun.elapsed() < Wall::from_secs(1), "{:?}", begun.elapsed());
    running(&run);
    let _ = manifest_of(run);
}

// ---------------------------------------------------------------- KG-5

#[test]
fn kg_05_a_device_paced_spec_run_runs_to_its_horizon() {
    let probe = Probe::new();
    let assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe));
    let mut run = start_spec_run(&spec_one(), &profile_one(), assembly).unwrap();
    let begun = Instant::now();
    run.run_until_end(after(&run, Wall::from_millis(50))).unwrap();
    assert!(begun.elapsed() >= Wall::from_millis(50), "{:?}", begun.elapsed());
    running(&run);
    let _ = manifest_of(run);
}

#[test]
fn kg_05_a_device_paced_spec_run_ends_at_its_scheduled_stop() {
    let probe = Probe::new();
    let mut spec = spec_one();
    // 30 ms after T0 on the Provider's 1 Msps receive clock.
    spec["schedule"] = serde_json::json!([{
        "at": { "clock": "radio", "offset_ticks": 30_000 },
        "action": { "kind": "stop", "target": null }
    }]);
    let assembly = provider(
        paced().assembly,
        "radio",
        ThreadedProvider::new("radio", "radio", &probe).publishing(100, Wall::from_millis(1)),
    );
    let mut run = start_spec_run(&spec, &profile_one(), assembly).unwrap();
    running(&run);
    let begun = Instant::now();
    let result = run.run_until_end(after(&run, Wall::from_secs(5)));
    assert!(
        matches!(result, Err(RunHandleError::Ended { termination: Termination::Completed {} })),
        "{result:?}"
    );
    assert!(begun.elapsed() < Wall::from_secs(1), "{:?}", begun.elapsed());
}

// ---------------------------------------------------------------- KG-6

#[test]
fn kg_06_a_prepare_that_hangs_fails_the_run_within_its_budget() {
    let probe = Probe::new();
    let assembly = provider(
        paced().assembly,
        "radio",
        ThreadedProvider::new("radio", "radio", &probe).with_prepare_delay(Wall::from_secs(30)),
    );
    let begun = Instant::now();
    let run = start_spec_run(&spec_one(), &profile_one(), assembly).unwrap();
    assert!(begun.elapsed() < Wall::from_secs(12), "{:?}", begun.elapsed());
    assert_eq!(
        run.state(),
        RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Prepare } }
    );
    let manifest = manifest_of(run);
    assert!(failure(&manifest).starts_with("KC-12a: prepare of radio"), "{}", failure(&manifest));
    assert!(
        manifest
            .termination
            .cleanup_failures
            .iter()
            .any(|f| f.fragment.as_ref().is_some_and(|id| id.as_str() == "radio")),
        "{:?}",
        manifest.termination.cleanup_failures
    );
}

#[test]
fn kg_06_an_arm_that_hangs_fails_the_run_within_its_budget() {
    let probe = Probe::new();
    let assembly = provider(
        paced().assembly,
        "radio",
        ThreadedProvider::new("radio", "radio", &probe).with_arm_delay(Wall::from_secs(30)),
    );
    let begun = Instant::now();
    let run = start_spec_run(&spec_one(), &profile_one(), assembly).unwrap();
    assert!(begun.elapsed() < Wall::from_secs(12), "{:?}", begun.elapsed());
    assert_eq!(
        run.state(),
        RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Arm } }
    );
    let manifest = manifest_of(run);
    assert!(failure(&manifest).starts_with("KC-12a: arm of radio"), "{}", failure(&manifest));
}

#[test]
fn kg_06_the_simulation_class_prepares_on_the_callers_thread() {
    let probe = Probe::new();
    let clocks = Arc::new(ClockRegistry::new());
    let (authority, _) = SimAuthority::new(&clocks, mref("ezsdr.test.provider"), Pacing::FreeRunning);
    let assembly = provider(
        assembly_with(Box::new(authority), clocks),
        "radio",
        ThreadedProvider::new("radio", "radio", &probe),
    );
    let mut profile = profile_one();
    profile["environment"] = serde_json::json!({});
    let run = start_spec_run(&spec_one(), &profile, assembly).unwrap();
    let _ = manifest_of(run);
    let test = format!("radio:prepare_on:{:?}", std::thread::current().id());
    assert_eq!(probe.with_prefix("radio:prepare_on:"), [test]);
}

// ---------------------------------------------------------------- KG-7

/// A Simulation Spec Run armed at `now` with these declared clocks and lead.
fn armed_at(declare: &[(&str, u64)], now: i64, lead: u64) -> RunHandle {
    let clocks = Arc::new(ClockRegistry::new());
    let (authority, root) = SimAuthority::new(&clocks, mref("ezsdr.test.provider"), Pacing::FreeRunning);
    authority.manual().advance_to(TimePoint::new(root, now)).unwrap();
    let mut double = SteppedProvider::new("p", TestProvider::new("radio", 2), &Probe::new());
    for (stream, ratio) in declare {
        double = double.declaring(stream, *ratio, 1);
    }
    let assembly = provider(assembly_with(Box::new(authority), clocks), "radio", double);
    let mut profile = profile_one();
    profile["environment"] = serde_json::json!({ "ezsdr.time": { "class": "simulation", "start_lead_ns": lead } });
    start_spec_run(&spec_one(), &profile, assembly).unwrap()
}

fn t0_after(declare: &[(&str, u64)], lead: u64) -> i64 {
    let run = armed_at(declare, 7, lead);
    running(&run);
    let t0 = run.start_instant().unwrap().ticks;
    let _ = manifest_of(run);
    t0
}

#[test]
fn kg_07_t0_lies_on_every_declared_grid() {
    // L = lcm(6, 10) = 30; the first multiple of 30 at or after 7 + 1 000.
    assert_eq!(t0_after(&[("radio/a", 6), ("radio/b", 10)], 1_000), 1_020);
}

#[test]
fn kg_07_with_no_declared_clock_t0_is_not_rounded() {
    assert_eq!(t0_after(&[], 1_007), 7 + 1_007);
}

#[test]
fn kc_15_an_overflow_of_l_or_of_t0_fails_arm() {
    // The exit review's KC-15 clause: an overflow of L or of T0 is `Failed { arm }`
    // with the reason "KC-15: overflow". Ratio terms stop at 2^31 (TM-3).
    let cap = 1_u64 << 31;
    // L = lcm(2^31, 2^31 - 1, 2^31 - 3), pairwise coprime, about 2^93: past u64::MAX.
    let l = armed_at(&[("radio/a", cap), ("radio/b", cap - 1), ("radio/c", cap - 3)], 7, 1_000);
    // L = 2^31; the first multiple of it at or after 2^63 - 2^31 + 1 007 is 2^63, past
    // i64::MAX.
    let t0 = armed_at(&[("radio/a", cap)], i64::MAX - cap as i64 + 8, 1_000);
    for (name, run) in [("L", l), ("T0", t0)] {
        assert_eq!(
            run.state(),
            RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Arm } },
            "{name}"
        );
        assert_eq!(failure(&manifest_of(run)), "KC-15: overflow", "{name}");
    }
}

// ---------------------------------------------------------------- KG-9 (TM-16c)

#[test]
fn tm_16c_a_paced_callback_sees_now_at_or_after_its_instant() {
    let clocks = Arc::new(ClockRegistry::new());
    let (authority, root) = WallAuthority::new(&clocks, mref("ezsdr.test.provider"));
    use ezsdr_kernel::module_api::Authority;
    let time = authority.time();
    let read = Arc::new(Mutex::new(None));
    let at = TimePoint::new(root, time.now(root).unwrap().ticks + 5_000_000);
    let (time_in, read_in) = (time.clone(), read.clone());
    time.schedule(
        at,
        Box::new(move |_| {
            std::thread::sleep(Wall::from_millis(2));
            *read_in.lock().unwrap() = Some(time_in.now(root).unwrap());
        }),
    )
    .unwrap();
    assert_eq!(authority.next_wakeup(), Some(at));
    let seen = read.lock().unwrap().expect("the callback ran");
    assert!(seen.ticks >= at.ticks + 2_000_000, "{seen:?} {at:?}");
}

#[test]
fn tm_16c_a_paced_authority_accepts_an_instant_already_passed() {
    let clocks = Arc::new(ClockRegistry::new());
    let (authority, root) = WallAuthority::new(&clocks, mref("ezsdr.test.provider"));
    use ezsdr_kernel::module_api::Authority;
    let time = authority.time();
    let fired = time.now(root).unwrap();
    time.schedule(fired, Box::new(|_| {})).unwrap();
    assert_eq!(authority.next_wakeup(), Some(fired));
    std::thread::sleep(Wall::from_millis(6));
    let passed = TimePoint::new(root, time.now(root).unwrap().ticks - 1_000_000);
    time.schedule(passed, Box::new(|_| {})).expect("an instant already passed is accepted");
    let begun = Instant::now();
    assert_eq!(authority.next_wakeup(), Some(passed));
    assert!(begun.elapsed() < Wall::from_millis(5));
    let before = TimePoint::new(root, fired.ticks - 1);
    assert!(matches!(
        time.schedule(before, Box::new(|_| {})),
        Err(ezsdr_kernel::time::TimeError::InPast { .. })
    ));
}

// ---------------------------------------------------------------- KG-10

#[test]
fn kg_10_a_device_lost_from_a_provider_thread_aborts_the_run() {
    let probe = Probe::new();
    let assembly = provider(
        paced().assembly,
        "radio",
        ThreadedProvider::new("radio", "radio", &probe).losing_device(Wall::from_millis(20)),
    );
    let mut run = start_spec_run(&spec_one(), &profile_one(), assembly).unwrap();
    let begun = Instant::now();
    let result = run.run_until_end(after(&run, Wall::from_secs(5)));
    assert!(begun.elapsed() < Wall::from_secs(1), "{:?}", begun.elapsed());
    let lost = kind(EventKind::DEVICE_LOST);
    assert_eq!(
        result,
        Err(RunHandleError::Ended {
            termination: Termination::Stopped { cause: StopCause::Policy { kind: lost.clone() } }
        })
    );
    let manifest = manifest_of(run);
    assert!(
        manifest
            .run
            .transitions
            .iter()
            .any(|t| matches!(t.state, RunState::Stopping { mode: ezsdr_kernel::run::CleanupMode::Abort })),
        "{:?}",
        manifest.run.transitions
    );
    let radio = ResourceId::parse("radio").unwrap();
    assert!(manifest.events.delivered.iter().any(|e| e.kind == lost && e.source == radio));
}

// ---------------------------------------------------------------- KG-11

#[test]
fn kg_11_a_device_paced_run_records_its_root_s_relations() {
    let probe = Probe::new();
    let rig = paced();
    let relations = rig.relations.clone();
    let assembly = provider(rig.assembly, "radio", ThreadedProvider::new("radio", "radio", &probe));
    let manifest = manifest_of(start_spec_run(&spec_one(), &profile_one(), assembly).unwrap());
    assert_eq!(manifest.clocks.relations, relations);
    assert_eq!(
        relations.iter().map(|r| r.target).collect::<Vec<_>>(),
        [ClockDomainId::HOST_MONOTONIC, ClockDomainId::UTC]
    );
    assert!(relations.iter().all(|r| r.source == rig.root));
}

/// A Simulation Authority that would publish relations (KA-16's negative case).
struct RelatingSim(SimAuthority, Vec<ezsdr_kernel::time::ClockRelation>);

impl ezsdr_kernel::module_api::Authority for RelatingSim {
    fn descriptor(&self) -> &ezsdr_kernel::module_api::AuthorityDescriptor {
        self.0.descriptor()
    }
    fn time(&self) -> Arc<dyn ezsdr_kernel::time::TimeAuthority> {
        self.0.time()
    }
    fn next_wakeup(&self) -> Option<TimePoint> {
        self.0.next_wakeup()
    }
    fn relations(&self) -> Vec<ezsdr_kernel::time::ClockRelation> {
        self.1.clone()
    }
}

#[test]
fn kg_11_a_simulation_run_records_none() {
    let clocks = Arc::new(ClockRegistry::new());
    let (authority, _) = SimAuthority::new(&clocks, mref("ezsdr.test.provider"), Pacing::FreeRunning);
    let relations = paced().relations;
    let assembly = provider(
        assembly_with(Box::new(RelatingSim(authority, relations)), clocks),
        "radio",
        TestProvider::new("radio", 2),
    );
    let mut profile = profile_one();
    profile["environment"] = serde_json::json!({});
    let manifest = manifest_of(start_spec_run(&spec_one(), &profile, assembly).unwrap());
    assert!(manifest.clocks.relations.is_empty());
    assert!(!manifest.termination.cleanup_failures.iter().any(|f| f.reason.starts_with("TM-18")));
}

fn tm_18_failures(manifest: &Manifest) -> Vec<String> {
    manifest
        .termination
        .cleanup_failures
        .iter()
        .filter(|f| f.reason.starts_with("TM-18"))
        .map(|f| {
            assert_eq!(f.step, CleanupStep::ReleaseAndWriteManifest);
            assert_eq!(f.fragment, None);
            f.reason.clone()
        })
        .collect()
}

#[test]
fn kg_11_a_device_paced_run_without_a_utc_relation_says_so() {
    let probe = Probe::new();
    let rig = paced_with(WallAuthority::without_relations);
    let assembly = provider(rig.assembly, "radio", ThreadedProvider::new("radio", "radio", &probe));
    let manifest = manifest_of(start_spec_run(&spec_one(), &profile_one(), assembly).unwrap());
    assert!(manifest.clocks.relations.is_empty());
    assert_eq!(tm_18_failures(&manifest), ["TM-18: the Authority published no relation of its root to utc"]);

    let mut foreign = paced().relations[1].clone();
    foreign.source = ClockDomainId::HOST_MONOTONIC;
    let rig = paced_with(|a| a.with_relations(vec![foreign]));
    let assembly = provider(rig.assembly, "radio", ThreadedProvider::new("radio", "radio", &probe));
    let manifest = manifest_of(start_spec_run(&spec_one(), &profile_one(), assembly).unwrap());
    assert!(manifest.clocks.relations.is_empty());
    assert_eq!(
        tm_18_failures(&manifest),
        [
            "TM-18: a relation whose source is not the primary root",
            "TM-18: the Authority published no relation of its root to utc"
        ]
    );
}

/// A device-paced Authority whose `relations()` panics (KC-45).
struct PanickingRelations(WallAuthority);

impl ezsdr_kernel::module_api::Authority for PanickingRelations {
    fn descriptor(&self) -> &ezsdr_kernel::module_api::AuthorityDescriptor {
        self.0.descriptor()
    }
    fn time(&self) -> Arc<dyn ezsdr_kernel::time::TimeAuthority> {
        self.0.time()
    }
    fn next_wakeup(&self) -> Option<TimePoint> {
        self.0.next_wakeup()
    }
    fn relations(&self) -> Vec<ezsdr_kernel::time::ClockRelation> {
        panic!("test: relations() panics")
    }
}

#[test]
fn kc_45_an_authority_whose_relations_panic_records_none_and_says_so() {
    // The exit review's KC-45 clause: an Authority whose `relations()` panics records no
    // relation and the missing-`utc` failure.
    let probe = Probe::new();
    let clocks = Arc::new(ClockRegistry::new());
    let (authority, _) = WallAuthority::new(&clocks, mref("ezsdr.test.provider"));
    let assembly = provider(
        assembly_with(Box::new(PanickingRelations(authority)), clocks),
        "radio",
        ThreadedProvider::new("radio", "radio", &probe),
    );
    let manifest = manifest_of(start_spec_run(&spec_one(), &profile_one(), assembly).unwrap());
    assert_eq!(manifest.run.execution_class, ExecutionClass::HardwareInLoop);
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Client {} });
    assert!(manifest.clocks.relations.is_empty());
    assert_eq!(tm_18_failures(&manifest), ["TM-18: the Authority published no relation of its root to utc"]);
}

// ---------------------------------------------------------------- KG-12

#[test]
fn kg_12_run_child_is_refused_in_a_device_paced_session() {
    let probe = Probe::new();
    let assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe));
    let mut run = session(assembly, &profile_one());
    let child_probe = Probe::new();
    let child = provider(paced().assembly, "radio", ThreadedProvider::new("child", "radio", &child_probe));
    let mut driven = false;
    let (entry, manifest) = run
        .run_child(&spec_one(), &profile_one(), child, &mut |_| driven = true)
        .unwrap();
    assert!(manifest.is_none());
    assert!(!driven);
    match &entry.outcome {
        Outcome::Rejected { violations } => {
            assert_eq!(violations.len(), 1);
            assert_eq!(violations[0].check, ns("ezsdr.run_child"));
            assert!(violations[0].reason.contains("KG-12"), "{}", violations[0].reason);
        }
        other => panic!("{other:?}"),
    }
    assert!(child_probe.lines().is_empty(), "{:?}", child_probe.lines());
    let _ = manifest_of(run);
}
