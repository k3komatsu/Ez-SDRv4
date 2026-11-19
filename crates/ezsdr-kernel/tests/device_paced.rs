//! Spec 19's Kernel amendments for the device-paced classes (KG-1…KG-12), through the
//! real coordinator with the §0 doubles `WallAuthority` and `ThreadedProvider`.
//! No positive verdict is a wall interval: a wake test releases its event only after the waiting call is seen parked, and a count is waited for as a condition;
//! every positive expiry left is an expiry that fails a test, at least 3 s long and meant to be 100 times what the right behaviour takes (note 51, P3). The negative
//! waits (a count that must not move) are pinned, each with its reason, in `ezsdr-acceptance`'s `governance.rs` (P3a).

mod support;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration as Wall, Instant};

use ezsdr_kernel::binding::{AdmissionCheck, AdmissionCheckRegistry, CheckStage, Violation};
use ezsdr_kernel::coordinator::start_probe::{self, FreezePoint, FreezeReading, Point, Reading};
use ezsdr_kernel::coordinator::{Assembly, RunHandle, RunHandleError, connect, start_spec_run};
use ezsdr_kernel::event::{Action, EventKind, Target};
use ezsdr_kernel::id::ClockDomainId;
use ezsdr_kernel::manifest::Manifest;
use ezsdr_kernel::module_api::{ActionSubmitter, ExecutionClass, ModuleErrorKind, Pacing, Provider, UpdateClass};
use ezsdr_kernel::policy::{EventKindDecl, EventKindRegistry, Reaction};
use ezsdr_kernel::run::{CleanupMode, CleanupStep, Lease, RunState, Stage, StopCause, SystemHostClock, Termination};
use ezsdr_kernel::session::{Outcome, SessionAction};
use ezsdr_kernel::spec::{Ident, Key, Namespace, Value};
use ezsdr_kernel::time::{ClockRegistry, Duration, ScheduleHandle, TimePoint};
use support::start_window::{
    Call, CallKind, ClockLog, Gate as ParkGate, Held, Hooks, OpenOnDrop, StartTrace, WatchedAuthority, WatchedClock, far_future_stop, gate, infra, open, seen_for, start_on_worker, sweep, wait,
    wait_end_requested, wait_or_panicked, with_seen,
};
use support::{
    FakeHostClock, FinishChecked, Probe, RecordingSink, SimAuthority, SteppedProvider, TestLinkModule, TestProvider, ThreadedProvider, WallAuthority, WallPark, mref, ns, release, release_one,
    run_checks, run_kinds, run_registry,
};

// ---------------------------------------------------------------- fixtures

struct Paced {
    root: ClockDomainId,
    relations: Vec<ezsdr_kernel::time::ClockRelation>,
    assembly: Assembly,
}

fn assembly_with(authority: Box<dyn ezsdr_kernel::module_api::Authority>, clocks: Arc<ClockRegistry>) -> Assembly {
    let mut assembly = Assembly::new(authority, clocks);
    assembly.registry = run_registry();
    assembly.checks = run_checks(false);
    assembly.kinds = run_kinds();
    assembly
}

fn paced_with(adjust: impl FnOnce(WallAuthority) -> WallAuthority) -> Paced {
    let clocks = Arc::new(ClockRegistry::new());
    let (authority, root) = WallAuthority::new(&clocks, mref("ezsdr.test.provider"));
    let authority = adjust(authority);
    let relations = authority.measured_relations();
    Paced { root, relations, assembly: assembly_with(Box::new(authority), clocks) }
}

/// `paced_with`, plus the probe that sees the control loop park in `next_wakeup`.
fn paced_parked(adjust: impl FnOnce(WallAuthority) -> WallAuthority) -> (Paced, WallPark) {
    let mut park = None;
    let rig = paced_with(|authority| {
        park = Some(authority.park_probe());
        adjust(authority)
    });
    (rig, park.unwrap())
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
    run.finish_checked()
}

/// The reason of a Run that ended `Failed` (KC-7), or nothing.
fn failure(manifest: &Manifest) -> String {
    match &manifest.termination.reason {
        Termination::Failed { reason, .. } => reason.clone(),
        _ => String::new(),
    }
}

fn running(run: &RunHandle) {
    assert!(matches!(run.state(), RunState::Running {}), "{:?}", run.state());
}

fn after(run: &RunHandle, wall: Wall) -> TimePoint {
    let now = run.now();
    now.checked_add(Duration::new(now.domain(), wall.as_nanos() as i64)).unwrap()
}

fn count(probe: &Probe, prefix: &str) -> usize {
    probe.with_prefix(prefix).len()
}

fn stepped_on(probe: &Probe) -> Vec<String> {
    probe.with_prefix("rec:step_on:").into_iter().map(|line| line.trim_start_matches("rec:step_on:").to_owned()).collect()
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
    assert_eq!(manifest.run.execution_class, Some(ExecutionClass::HardwareInLoop));
    assert!(!manifest.run.deterministic);
    let calls: Vec<_> = probe.with_prefix("radio:").into_iter().filter(|line| ["radio:prepare", "radio:arm", "radio:start", "radio:stop", "radio:cleanup"].contains(&line.as_str())).collect();
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
    assert_eq!(manifest_of(run).run.execution_class, Some(ExecutionClass::Hardware));
}

#[test]
fn kg_01_a_stepped_provider_is_refused_in_a_device_paced_class() {
    let probe = Probe::new();
    let rig = paced();
    let assembly = provider(rig.assembly, "radio", SteppedProvider::new("p", TestProvider::new("radio", 2), &probe));
    let run = start_spec_run(&spec_one(), &profile_one(), assembly).unwrap();
    assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Plan, .. } }));
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
    component.timing.budget = Some(ezsdr_kernel::time::RelativeBudget::new(ezsdr_kernel::time::Duration::new(ClockDomainId::HOST_MONOTONIC, 1_000_000)).unwrap());
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
    for (field, value) in [("rt_policy", serde_json::json!({ "sched": "fifo", "priority": 10 })), ("affinity", serde_json::json!([1]))] {
        let probe = Probe::new();
        let (spec, profile) = island_docs(field, value);
        let mut assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe));
        assembly.executors.insert(Ident::parse("exec").unwrap(), Box::new(support::ProbeExecutor::new("x", &probe)));
        let run = start_spec_run(&spec, &profile, assembly).unwrap();
        assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Plan, .. } }), "{field}");
        let reason = failure(&manifest_of(run));
        assert_eq!(reason, format!("KC-2a: island_0 declares {field}, which a device-paced class does not apply before Phase 10"));
    }
}

// ---------------------------------------------------------------- KG-2

fn publishing_spec_run(probe: &Probe, sink_double: RecordingSink) -> RunHandle {
    let (spec, profile) = output_docs(64);
    let assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", probe).publishing(100, Wall::from_millis(1)));
    let run = start_spec_run(&spec, &profile, sink(assembly, sink_double, probe)).unwrap();
    running(&run);
    run
}

#[test]
fn kg_02_the_data_thread_drains_a_sink_while_no_call_runs() {
    let probe = Probe::new();
    let run = publishing_spec_run(&probe, RecordingSink::new("rec", &probe));
    // The test thread makes no call to the Run while it waits, so only the data thread can drain the Sink.
    let first = count(&probe, "rec:block:");
    assert!(probe.wait_until(|lines| lines.iter().filter(|line| line.starts_with("rec:block:")).count() >= first + 50, Wall::from_secs(10)), "{first} blocks, then {}", count(&probe, "rec:block:"));
    let manifest = manifest_of(run);
    assert_eq!(manifest.links[0].drops, Some(0));
}

#[test]
fn kg_02_every_step_before_finish_runs_on_the_data_thread() {
    let probe = Probe::new();
    let run = publishing_spec_run(&probe, RecordingSink::new("rec", &probe).recording_threads());
    assert!(probe.wait_until(|lines| lines.iter().any(|line| line.starts_with("rec:step_on:")), Wall::from_secs(10)));
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
    let assembly = provider(assembly_with(Box::new(authority), clocks), "radio", SteppedProvider::new("p", TestProvider::new("radio", 2), probe).publishing_every(10));
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
    assert!(threads.iter().all(|t| !t.ends_with(":ezsdr-data") && !t.ends_with(":ezsdr-bounded")), "{threads:?}");
}

#[test]
fn kg_02_a_sink_that_always_progresses_does_not_livelock() {
    let probe = Probe::new();
    let run = publishing_spec_run(&probe, RecordingSink::new("rec", &probe).always_progressing());
    // A livelocking Sink crosses the livelock threshold (1 000 rounds at one instant) before it can step 1 000 times.
    assert!(probe.wait_until(|lines| lines.iter().filter(|line| line.starts_with("rec:step:")).count() > 1_000, Wall::from_secs(10)));
    running(&run);
    let livelock = kind(EventKind::STEP_LIVELOCK);
    assert!(!run.events(0).iter().any(|event| event.kind == livelock));
    let _ = manifest_of(run);
}

#[test]
fn kg_02_a_failed_sink_is_not_stepped_again() {
    let probe = Probe::new();
    let rig = paced();
    let (mut spec, profile) = output_docs(64);
    spec["policies"] = serde_json::json!({ "failure": { "ezsdr.DEVICE_LOST": "continue" } });
    let assembly = provider(rig.assembly, "radio", ThreadedProvider::new("radio", "radio", &probe));
    let at = 20_000_000 + Instant::now().elapsed().as_nanos() as i64;
    let sink_double = RecordingSink::new("rec", &probe).failing_step_at(at, ModuleErrorKind::DeviceLost);
    let run = start_spec_run(&spec, &profile, sink(assembly, sink_double, &probe)).unwrap();
    running(&run);
    // The failing step is the first one at or after `at`; it records its line before it fails. Wait for it as a condition, then give the sink time to be stepped
    // again (the one negative wait).
    let failed = |lines: &[String]| lines.iter().filter_map(|line| line.strip_prefix("rec:step:")).any(|until| until.parse::<i64>().is_ok_and(|until| until >= at));
    assert!(probe.wait_until(failed, Wall::from_secs(10)), "the sink never reached its failing step: {:?}", probe.lines());
    let first = count(&probe, "rec:step:");
    std::thread::sleep(Wall::from_millis(100));
    assert_eq!(count(&probe, "rec:step:"), first, "stepped again after its failure");
    let lost = kind(EventKind::DEVICE_LOST);
    let sink_source = ezsdr_kernel::event::EventSource::Output { output: Ident::parse("rec").unwrap() };
    let reports = run.events(0).into_iter().filter(|e| e.kind == lost && e.source == sink_source).count();
    assert_eq!(reports, 1);
    running(&run);
    let _ = manifest_of(run);
}

fn session(assembly: Assembly, profile: &serde_json::Value) -> RunHandle {
    let run = connect(profile, &Default::default(), assembly, Lease::attached()).unwrap();
    running(&run);
    run
}

/// `rounds` times: a `wait_for` that only the data thread's delivery can end, with a horizon an hour away. The Provider emits its mark when the test releases it,
/// and the test does so only after it has seen the control loop wait for the horizon, so the delivery cannot come before the call parks and the call cannot
/// return without a wake: a lost wake is a hang that the 10 s expiry fails, never an early return a bound would have to measure. With `fire_first` the Authority
/// also runs each wake's callback before `schedule` returns (KC-46a), and the double counts that it did.
fn woken_rounds(rounds: u32, fire_first: bool) {
    let probe = Probe::new();
    let gate = release();
    let (rig, park) = paced_parked(|authority| if fire_first { authority.firing_before_schedule_returns() } else { authority });
    let double = ThreadedProvider::new("radio", "radio", &probe).marking_on("test.MARK", gate.clone());
    let mut run = session(provider(rig.assembly, "radio", double), &profile_one());
    let mark = kind("test.MARK");
    let (tick_tx, ticks) = mpsc::channel();
    let (found_tx, found) = mpsc::channel();
    let (run_tx, returned) = mpsc::channel();
    // Detached, as the KC-36 tests' workers are: a joined worker would turn a lost wake into a test timeout instead of a failed assertion.
    std::thread::spawn(move || {
        let mut from = 0;
        for _ in 0..rounds {
            let horizon = after(&run, Wall::from_secs(3600));
            tick_tx.send(horizon.ticks_in(horizon.domain()).unwrap()).unwrap();
            let index = run.wait_for(std::slice::from_ref(&mark), from, horizon).unwrap().expect("the delivered mark is found before the horizon");
            found_tx.send(index).unwrap();
            from = index + 1;
        }
        let _ = run_tx.send(run);
    });
    for round in 0..rounds {
        let tick = ticks.recv_timeout(Wall::from_secs(10)).expect("the worker began a wait");
        assert!(park.wait_parked_until(tick, Wall::from_secs(10)), "round {round}: the control loop never waited for the horizon");
        release_one(&gate);
        found.recv_timeout(Wall::from_secs(10)).unwrap_or_else(|_| panic!("round {round}: a delivery did not wake the waiting call"));
    }
    let run = returned.recv_timeout(Wall::from_secs(10)).expect("the worker returned the Run");
    // Read once, after the data thread is joined: it counts a wake after it has re-locked the Authority, so no earlier read is stable.
    let _ = manifest_of(run);
    if fire_first {
        assert!(park.fired_before_schedule_returned() >= u64::from(rounds), "{} of {rounds} wakes ran their callback before `schedule` returned", park.fired_before_schedule_returned());
    }
}

#[test]
fn kg_02_wait_for_returns_when_the_data_thread_delivers() {
    woken_rounds(1, false);
}

fn stopping_kinds() -> EventKindRegistry {
    let mut kinds = run_kinds();
    for name in ["test.first", "test.second"] {
        kinds.register(Some(ns("test")), EventKindDecl { kind: kind(name), default: Reaction::Stop, severity: ezsdr_kernel::event::Severity::Info }).unwrap();
    }
    kinds
}

#[test]
fn kg_02_the_first_delivered_stopping_event_is_the_termination_cause() {
    // Both marks are drained by whichever thread gets there first; KC-31's one lock
    // keeps `delivered` and the reactions in one order either way.
    let mut registry_vocabulary = support::test_vocabulary();
    for name in ["test.first", "test.second"] {
        registry_vocabulary.event_kinds.push(EventKindDecl { kind: kind(name), default: Reaction::Stop, severity: ezsdr_kernel::event::Severity::Info });
    }
    for _ in 0..50 {
        let probe = Probe::new();
        let mut rig = paced();
        rig.assembly.kinds = stopping_kinds();
        rig.assembly.registry = registry_with(registry_vocabulary.clone());
        let assembly = provider(rig.assembly, "a", ThreadedProvider::new("a", "a", &probe).marking("test.first", Wall::from_millis(20)));
        let assembly = provider(assembly, "b", ThreadedProvider::new("b", "b", &probe).marking("test.second", Wall::from_millis(20)));
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
        while begun.elapsed() < Wall::from_secs(10) {
            let horizon = after(&run, Wall::from_millis(1));
            if matches!(run.wait_for(std::slice::from_ref(&never), 0, horizon), Err(RunHandleError::Ended { .. })) {
                break;
            }
        }
        let manifest = manifest_of(run);
        let first = manifest.events.delivered.iter().find(|event| event.kind == kind("test.first") || event.kind == kind("test.second")).expect("a stopping event was delivered").kind.clone();
        assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Policy { event: first } });
    }
}

fn registry_with(vocabulary: ezsdr_kernel::module_api::VocabularyDescriptor) -> ezsdr_kernel::module_api::ModuleRegistry {
    let mut registry = ezsdr_kernel::module_api::ModuleRegistry::new();
    registry.register_vocabulary(vocabulary).unwrap();
    for (descriptor, factories) in [
        (support::test_provider_descriptor(), ezsdr_kernel::module_api::Factories { provider: true, authority: true, ..Default::default() }),
        (support::test_sink_descriptor(), ezsdr_kernel::module_api::Factories { sink: true, ..Default::default() }),
    ] {
        registry.register(descriptor, factories).unwrap();
    }
    registry
}

#[test]
fn kg_02_every_delivery_wakes_a_waiting_call() {
    woken_rounds(50, false);
}

#[test]
fn kg_02_a_wake_that_fires_before_schedule_returns_still_clears() {
    // KC-46a: the pending generation is stored before `schedule`, so a callback that
    // runs before `schedule` returns clears its own wake (Review L, P1-6). Every round
    // runs the callback first, so a wake left pending by round one wakes nothing in round two.
    woken_rounds(20, true);
}

#[test]
fn kg_02_a_detached_lease_expires_while_no_call_runs() {
    let probe = Probe::new();
    let assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe));
    let lease = Lease::detached(50, false, "token", &SystemHostClock::new()).unwrap();
    let mut run = connect(&profile_one(), &Default::default(), assembly, lease).unwrap();
    running(&run);
    run.disconnect();
    assert!(probe.wait_for("radio:stop", Wall::from_secs(10)));
    let manifest = manifest_of(run);
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::LeaseExpiry {} });
}

/// A manual host clock (RS-22) and the probe of the data thread's passes: a test moves the clock and then waits until that thread has run on, so the TTL tests judge no wall interval.
struct ManualHost {
    clock: Arc<FakeHostClock>,
    park: WallPark,
}

impl ManualHost {
    /// Moves the clock, then waits until the data thread has read the time for three more passes: the second read the new host time, and the third began after that pass had requested the end it found. The expiry is a failure, 10 s, not an oracle.
    fn advance_seen(&self, millis: u64) {
        self.clock.advance(millis);
        let target = self.park.data_thread_reads() + 3;
        let begun = Instant::now();
        while self.park.data_thread_reads() < target {
            assert!(begun.elapsed() < Wall::from_secs(10), "the data thread did not run on");
            std::thread::yield_now();
        }
    }
}

fn detached_session(probe: &Probe, ttl_ms: u64) -> (RunHandle, ManualHost) {
    let (rig, park) = paced_parked(|a| a);
    let host = ManualHost { clock: Arc::new(FakeHostClock::new()), park };
    let mut assembly = provider(rig.assembly, "radio", ThreadedProvider::new("radio", "radio", probe));
    assembly.host_clock = host.clock.clone();
    let lease = Lease::detached(ttl_ms, true, "token", &*host.clock).unwrap();
    let run = connect(&profile_one(), &Default::default(), assembly, lease).unwrap();
    running(&run);
    (run, host)
}

fn provider_stopped(probe: &Probe) -> bool {
    probe.lines().iter().any(|line| line == "radio:stop")
}

#[test]
fn kc_36_renew_moves_the_deadline_the_data_thread_checks() {
    // The exit review's KC-36 clause, for `Renew`: renewed every 50 ms of host time for 600 ms, a
    // 200 ms Lease outlives its first deadline, and then expires with no call running.
    let probe = Probe::new();
    let (mut run, host) = detached_session(&probe, 200);
    run.disconnect();
    for _ in 0..12 {
        host.advance_seen(50);
        let entry = run.submit(SessionAction::Renew {}, None).unwrap();
        assert!(admitted(&entry), "{entry:?}");
    }
    assert!(!provider_stopped(&probe), "{:?}", probe.lines());
    running(&run);
    host.advance_seen(200);
    assert!(probe.wait_for("radio:stop", Wall::from_secs(10)));
    assert_eq!(manifest_of(run).termination.reason, Termination::Stopped { cause: StopCause::LeaseExpiry {} });
}

#[test]
fn kc_36_adopt_clears_the_deadline_the_data_thread_checks() {
    // The exit review's KC-36 clause, for `Adopt`: adopted at once, a 100 ms Lease
    // whose TTL the disconnect started does not expire in the next 300 ms of host time.
    let probe = Probe::new();
    let (mut run, host) = detached_session(&probe, 100);
    run.disconnect();
    let entry = run.submit(SessionAction::Adopt { token: "token".to_owned() }, None).unwrap();
    assert!(admitted(&entry), "{entry:?}");
    host.advance_seen(300);
    assert!(!provider_stopped(&probe), "{:?}", probe.lines());
    running(&run);
    assert_eq!(manifest_of(run).termination.reason, Termination::Stopped { cause: StopCause::Client {} });
}

// ---------------------------------------------------------------- KC-36's disconnect signal (#89)

fn client_disconnect() -> Termination {
    Termination::Stopped { cause: StopCause::ClientDisconnect {} }
}

/// Whether the Run entered `Stopping { Orderly }` and no other `Stopping`.
fn stopped_orderly(manifest: &Manifest) -> bool {
    let stopping: Vec<_> = manifest.run.transitions.iter().filter(|t| matches!(t.state, RunState::Stopping { .. })).collect();
    stopping.len() == 1 && stopping[0].state == RunState::Stopping { mode: CleanupMode::Orderly }
}

#[test]
fn kc_36_a_disconnect_signal_ends_a_waiting_advance() {
    // F1: the signal comes only once the control loop waits in `next_wakeup` for the
    // horizon; no event is ever delivered, so only the signal's own wake ends that wait.
    let probe = Probe::new();
    let (rig, park) = paced_parked(|authority| authority);
    let mut run = session(provider(rig.assembly, "radio", ThreadedProvider::new("radio", "radio", &probe)), &profile_one());
    let signal = run.disconnect_signal();
    let target = after(&run, Wall::from_secs(3600));
    let (done, returned) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = run.advance_to(target);
        let _ = done.send((result, run));
    });
    let root = target.domain();
    assert!(park.wait_parked_until(target.ticks_in(root).unwrap(), Wall::from_secs(10)), "the control loop never waited for the horizon");
    signal();
    let (result, run) = returned.recv_timeout(Wall::from_secs(10)).expect("the signal ended the waiting advance");
    assert_eq!(result, Err(RunHandleError::Ended { termination: client_disconnect() }));
    assert_eq!(count(&probe, "radio:stop"), 1);
    let manifest = manifest_of(run);
    assert_eq!(manifest.termination.reason, client_disconnect());
    assert!(manifest.termination.also.is_empty(), "{:?}", manifest.termination.also);
    assert!(manifest.termination.at.unwrap().ticks_in(root).unwrap() < target.ticks_in(root).unwrap());
    assert!(stopped_orderly(&manifest), "{:?}", manifest.run.transitions);
}

#[test]
fn kc_36_a_disconnect_signal_ends_a_submit_waiting_for_its_provider() {
    // KC-21a's wait looks at the end every 10 ms: the Provider holds the Action it took,
    // and the signal ends the wait long before the 5 s budget would fail the Run.
    let probe = Probe::new();
    let hold = Arc::new((Mutex::new(false), Condvar::new()));
    let assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe).hold_after_take(hold.clone()));
    let mut run = session(assembly, &profile_one());
    let signal = run.disconnect_signal();
    let (done, returned) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let entry = run.submit(gain(2.0), None);
        let _ = done.send((entry, run));
    });
    assert!(probe.wait_for("radio:took:UpdateParameter", Wall::from_secs(10)), "{:?}", probe.lines());
    signal();
    let returned = returned.recv_timeout(Wall::from_secs(10));
    *hold.0.lock().unwrap() = true;
    hold.1.notify_all();
    let (entry, run) = returned.expect("the submit returned");
    let entry = entry.unwrap();
    assert!(admitted(&entry), "{entry:?}");
    assert!(!matches!(run.state(), RunState::Running {}), "{:?}", run.state());
    let manifest = manifest_of(run);
    assert_eq!(manifest.termination.reason, client_disconnect(), "{}", failure(&manifest));
    assert!(manifest.termination.also.is_empty(), "{:?}", manifest.termination.also);
}

#[test]
fn kc_36_a_disconnect_signal_ends_a_simulated_advance() {
    // Q2: a Simulation Run is cut too. The signal comes while the control thread is held
    // inside a step at 1000; the loop reads the end at its next round.
    let probe = Probe::new();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let clocks = Arc::new(ClockRegistry::new());
    let (authority, root) = SimAuthority::new(&clocks, mref("ezsdr.test.provider"), Pacing::FreeRunning);
    let (spec, mut profile) = output_docs(64);
    profile["environment"] = serde_json::json!({});
    let stepped = SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).publishing_every(10).with_wakeups(&[1_000]).blocking_step_at(1_000, gate.clone());
    let assembly = provider(assembly_with(Box::new(authority), clocks), "radio", stepped);
    let mut run = start_spec_run(&spec, &profile, sink(assembly, RecordingSink::new("rec", &probe), &probe)).unwrap();
    let signal = run.disconnect_signal();
    let far = TimePoint::new(root, 1_000_000_000_000);
    let (done, returned) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = run.advance_to(far);
        let _ = done.send((result, run));
    });
    assert!(probe.wait_for("p:step:1000", Wall::from_secs(10)), "{:?}", probe.lines());
    signal();
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    let (result, run) = returned.recv_timeout(Wall::from_secs(10)).expect("the signal ended the simulated advance");
    assert_eq!(result, Err(RunHandleError::Ended { termination: client_disconnect() }));
    let manifest = manifest_of(run);
    assert!(manifest.termination.at.unwrap().ticks_in(root).unwrap() < 1_000_000_000_000);
}

#[test]
fn kc_36_a_disconnect_signal_does_nothing_for_a_detached_lease_or_an_ended_run() {
    let probe = Probe::new();
    let (mut run, _host) = detached_session(&probe, 60_000);
    let signal = run.disconnect_signal();
    signal();
    signal();
    let near = after(&run, Wall::from_millis(10));
    assert_eq!(run.advance_to(near), Ok(()));
    running(&run);
    let manifest = manifest_of(run);
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Client {} });
    assert!(manifest.termination.also.is_empty(), "{:?}", manifest.termination.also);
    // The Run is gone: the signal neither panics nor keeps it, or its Provider, alive.
    signal();
    assert_eq!(probe.holders(), 1, "a signal kept the dropped Run alive");
}

#[test]
fn kc_36_the_first_end_requested_stands_against_a_disconnect_signal() {
    // (a) An idle Run: the signal runs no cleanup (KC-38); the next call does, and the
    // signal's end, requested first, stands against `finish`'s (KC-32).
    let probe = Probe::new();
    let run = session(provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe)), &profile_one());
    let signal = run.disconnect_signal();
    signal();
    running(&run);
    assert!(!provider_stopped(&probe), "{:?}", probe.lines());
    let manifest = manifest_of(run);
    assert_eq!(manifest.termination.reason, client_disconnect());
    assert!(manifest.termination.also.is_empty(), "{:?}", manifest.termination.also);

    // (b) The Policy ended the Run first: the signal says nothing new.
    let probe = Probe::new();
    let run = lost_device_session(&probe);
    let signal = run.disconnect_signal();
    assert!(probe.wait_for("radio:stop", Wall::from_secs(10)));
    signal();
    let manifest = manifest_of(run);
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Policy { event: kind(EventKind::DEVICE_LOST) } });
    assert!(manifest.termination.also.is_empty(), "{:?}", manifest.termination.also);
}

#[test]
fn kc_36_a_disconnect_signal_during_finish_s_cleanup_does_not_change_its_end() {
    // F3: `finish` asked first; a disconnect while its cleanup stops the Provider is dropped.
    let probe = Probe::new();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let run = session(provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe).gated_stop(gate.clone())), &profile_one());
    let signal = run.disconnect_signal();
    let (done, returned) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(run.finish_checked());
    });
    let stopping = probe.wait_for("radio:stop", Wall::from_secs(10));
    signal();
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    assert!(stopping, "{:?}", probe.lines());
    let manifest = returned.recv_timeout(Wall::from_secs(10)).expect("finish returned");
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Client {} });
    assert!(manifest.termination.also.is_empty(), "{:?}", manifest.termination.also);
    assert!(stopped_orderly(&manifest), "{:?}", manifest.run.transitions);
}

// ---------------------------------------------------------------- KG-3

fn lost_device_session(probe: &Probe) -> RunHandle {
    let assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", probe).publishing(100, Wall::from_millis(1)).losing_device(Wall::from_millis(20)));
    let assembly = sink(assembly, RecordingSink::new("rec", probe), probe);
    session(assembly, &session_sink_profile())
}

#[test]
fn kg_03_a_fatal_event_stops_the_providers_without_a_client_call() {
    let probe = Probe::new();
    let run = lost_device_session(&probe);
    assert!(probe.wait_for("radio:stop", Wall::from_secs(10)));
    let manifest = manifest_of(run);
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Policy { event: kind(EventKind::DEVICE_LOST) } });
    assert_eq!(count(&probe, "radio:stop"), 1);
}

#[test]
fn kc_29_an_end_the_data_thread_requested_is_cleaned_up_at_the_next_call() {
    // The exit review's KC-29 clause (as KG-3 amends it): after an end the data thread
    // requested while no call ran, `state()` reads `Running` and `events()` already
    // shows the cause, until the next call cleans up.
    let probe = Probe::new();
    let mut run = lost_device_session(&probe);
    assert!(probe.wait_for("radio:stop", Wall::from_secs(10)));
    std::thread::sleep(Wall::from_millis(50));
    if !matches!(run.state(), RunState::Running {}) {
        // A Run the data thread moved on dropped mid-unwind trips cleanup's debug_assert and aborts: leak it so the failure is the assertion.
        let state = run.state();
        std::mem::forget(run);
        panic!("{state:?}");
    }
    let lost = kind(EventKind::DEVICE_LOST);
    let radio = support::node_source("radio");
    assert!(run.events(0).iter().any(|e| e.kind == lost && e.source == radio), "{:?}", run.events(0));
    let begun = Instant::now();
    let result = run.advance_to(after(&run, Wall::from_secs(10)));
    assert!(begun.elapsed() < Wall::from_secs(5), "{:?}", begun.elapsed());
    let termination = Termination::Stopped { cause: StopCause::Policy { event: lost } };
    assert_eq!(result, Err(RunHandleError::Ended { termination: termination.clone() }));
    assert_eq!(run.state(), RunState::CleanedUp { termination });
}

#[test]
fn kc_32_a_disconnect_after_a_policy_stop_is_dropped() {
    // KC-32: a client stop that does not escalate says nothing the first end does not.
    // The data thread ends the Run through the Policy; a disconnect before the next
    // call requests a different Termination, which is dropped, not recorded in `also`.
    let probe = Probe::new();
    let mut run = lost_device_session(&probe);
    assert!(probe.wait_for("radio:stop", Wall::from_secs(10)));
    run.disconnect();
    let manifest = manifest_of(run);
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Policy { event: kind(EventKind::DEVICE_LOST) } });
    assert!(manifest.termination.also.is_empty(), "{:?}", manifest.termination.also);
}

#[test]
fn kg_03_an_abort_stops_the_data_thread() {
    let probe = Probe::new();
    let run = lost_device_session(&probe);
    assert!(probe.wait_for("radio:stop", Wall::from_secs(10)));
    std::thread::sleep(Wall::from_millis(50));
    let _ = manifest_of(run);
    let lines = probe.lines();
    let stop = lines.iter().position(|line| line == "radio:stop").unwrap();
    assert!(!lines[stop..].iter().any(|line| line.starts_with("rec:step:")), "{:?}", &lines[stop..]);
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
    assert!(probe.wait_for("radio:stop", Wall::from_secs(10)));
    assert!(probe.wait_for("other:stop", Wall::from_secs(10)));
    let manifest = manifest_of(run);
    assert!(matches!(manifest.termination.reason, Termination::Failed { stage: Stage::Run, .. }));
    for name in ["radio", "other"] {
        assert_eq!(count(&probe, &format!("{name}:stop")), 1, "{name}");
        assert_eq!(count(&probe, &format!("{name}:cleanup")), 1, "{name}");
    }
    assert!(!manifest.termination.cleanup_failures.iter().any(|f| f.step == CleanupStep::StopProviders), "{:?}", manifest.termination.cleanup_failures);
}

#[test]
fn kc_30_a_panic_after_a_device_lost_in_one_pass_fails_the_run() {
    // #65 (note 25): the data thread's pass applies KC-30 as a round does. `rec` loses
    // its device and `rec2` panics at one instant; the Policy lets the lost device
    // continue, so only the panic ends the Run.
    let probe = Probe::new();
    let (mut spec, mut profile) = output_docs(64);
    spec["policies"] = serde_json::json!({ "failure": { "ezsdr.DEVICE_LOST": "continue" } });
    let mut second = spec["outputs"][0].clone();
    second["id"] = serde_json::json!("rec2");
    spec["outputs"].as_array_mut().unwrap().push(second);
    profile["bindings"]["rec2"] = profile["bindings"]["rec"].clone();
    let mut link = profile["placements"]["links"][0].clone();
    link["to"]["component"] = serde_json::json!("rec2");
    profile["placements"]["links"].as_array_mut().unwrap().push(link);
    let at = 20_000_000;
    let mut assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe));
    assembly.sinks.insert(Ident::parse("rec2").unwrap(), Box::new(RecordingSink::new("rec2", &probe).panicking_step_at(at)));
    let sink_double = RecordingSink::new("rec", &probe).failing_step_at(at, ModuleErrorKind::DeviceLost);
    let run = start_spec_run(&spec, &profile, sink(assembly, sink_double, &probe)).unwrap();
    running(&run);
    assert!(probe.wait_for("radio:stop", Wall::from_secs(10)));
    let manifest = manifest_of(run);
    assert!(matches!(manifest.termination.reason, Termination::Failed { stage: Stage::Run, .. }));
    assert!(failure(&manifest).starts_with("KC-30: rec2: a Module panicked"), "{}", failure(&manifest));
    let lost = kind(EventKind::DEVICE_LOST);
    let rec = ezsdr_kernel::event::EventSource::Output { output: Ident::parse("rec").unwrap() };
    assert!(manifest.events.delivered.iter().any(|e| e.kind == lost && e.source == rec));
}

#[test]
fn kg_03_step_3_runs_a_final_round_over_the_sinks() {
    let probe = Probe::new();
    let (spec, profile) = output_docs(64);
    let assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe).publishing(100, Wall::from_millis(1)).with_stop_tail(2));
    let sink_double = RecordingSink::new("rec", &probe).recording_threads();
    let run = start_spec_run(&spec, &profile, sink(assembly, sink_double, &probe)).unwrap();
    running(&run);
    assert!(probe.wait_until(|lines| lines.iter().any(|line| line.starts_with("rec:step_on:")), Wall::from_secs(10)));
    let data = stepped_on(&probe)[0].clone();
    let _ = manifest_of(run);
    let lines = probe.lines();
    let stop = lines.iter().position(|line| line == "radio:stop").unwrap();
    let sink_stop = lines.iter().position(|line| line == "rec:stop:Orderly").unwrap();
    assert!(lines[stop..sink_stop].iter().any(|line| line.starts_with("rec:step_on:") && !line.ends_with(&data)), "{:?}", &lines[stop..sink_stop]);
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
    let assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe).publishing(100, Wall::from_millis(1)).with_stop_tail(2));
    let assembly = sink(assembly, RecordingSink::new("rec", &probe).recording_threads(), &probe);
    let lease = Lease::detached(50, false, "token", &SystemHostClock::new()).unwrap();
    let mut run = connect(&session_sink_profile(), &Default::default(), assembly, lease).unwrap();
    running(&run);
    run.disconnect();
    assert!(probe.wait_for_count("radio:tail", 2, Wall::from_secs(10)));
    // The tail the Provider delivered in stop reaches the Sink, and the data thread steps on after it, with no call running.
    let stepped_after_tail = |lines: &[String]| {
        let Some(last_tail) = lines.iter().rposition(|line| line == "radio:tail") else { return false };
        lines[last_tail..].iter().any(|line| line.starts_with("rec:block:")) && lines[last_tail..].iter().filter(|line| line.starts_with("rec:step_on:")).count() >= 2
    };
    assert!(probe.wait_until(stepped_after_tail, Wall::from_secs(10)), "{:?}", probe.lines());
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
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::LeaseExpiry {} });
    assert!(manifest.run.transitions.iter().any(|t| matches!(t.state, RunState::Stopping { mode: ezsdr_kernel::run::CleanupMode::Orderly })), "{:?}", manifest.run.transitions);
}

#[test]
fn kg_03_a_cancelled_horizon_wakes_the_control_loop() {
    // The loss cancels the horizon an hour away, and only that cancel can wake the loop: it is released after the loop is seen waiting for it, and the
    // wait would otherwise last the hour.
    let probe = Probe::new();
    let loss = Arc::new((Mutex::new(false), Condvar::new()));
    let (rig, park) = paced_parked(|authority| authority);
    let assembly = provider(rig.assembly, "radio", ThreadedProvider::new("radio", "radio", &probe).losing_device_on(loss.clone()));
    let mut run = start_spec_run(&spec_one(), &profile_one(), assembly).unwrap();
    running(&run);
    let horizon = after(&run, Wall::from_secs(3600));
    let (done, returned) = mpsc::channel();
    // Detached: a joined worker would turn a lost wake into a test timeout.
    std::thread::spawn(move || {
        let result = run.run_until_end(horizon);
        let _ = done.send((result, run));
    });
    assert!(park.wait_parked_until(horizon.ticks_in(horizon.domain()).unwrap(), Wall::from_secs(10)), "the control loop never waited for the horizon");
    *loss.0.lock().unwrap() = true;
    loss.1.notify_all();
    let (result, run) = returned.recv_timeout(Wall::from_secs(10)).expect("the loss woke the control loop that waited for the horizon");
    assert!(matches!(result, Err(RunHandleError::Ended { .. })), "{result:?}");
    let _ = manifest_of(run);
}

#[test]
fn kg_03_dropping_a_live_device_paced_handle_cleans_up() {
    let probe = Probe::new();
    let assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe).publishing(100, Wall::from_millis(1)));
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
    SessionAction::SetParameter { target: support::target("radio"), key: Key::parse("test.gain").unwrap(), value: Value::num(value).unwrap() }
}

fn admitted(entry: &ezsdr_kernel::session::LogEntry) -> bool {
    matches!(entry.outcome, Outcome::Admitted { .. })
}

#[test]
fn kg_04_a_session_call_returns_after_the_provider_has_finished() {
    let probe = Probe::new();
    let assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe).with_action_delay(Wall::from_millis(30)));
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
    let radio = serde_json::to_value(support::target("radio")).unwrap();
    let entry = |value: f64| {
        serde_json::json!({
            "at": { "clock": "radio", "offset_ticks": 0 },
            "action": { "kind": "update_parameter", "target": radio, "key": "test.gain", "value": value, }
        })
    };
    spec["schedule"] = serde_json::json!([entry(1.0), entry(2.0)]);
    let assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe).publishing(100, Wall::from_millis(1)).with_action_delay(Wall::from_millis(30)));
    let mut run = start_spec_run(&spec, &profile, sink(assembly, RecordingSink::new("rec", &probe), &probe)).unwrap();
    running(&run);
    let t0 = run.start_instant().unwrap();
    run.advance_to(t0.checked_add(Duration::new(t0.domain(), 1)).unwrap()).unwrap();
    assert_eq!(count(&probe, "radio:finished:UpdateParameter"), 2, "{:?}", probe.lines());
    let _ = manifest_of(run);
}

#[test]
fn kg_04_the_next_admission_sees_the_state_the_previous_action_made() {
    for _ in 0..20 {
        let probe = Probe::new();
        let assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe));
        let mut run = session(assembly, &profile_one());
        let clock = run.submit(SessionAction::SetParameter { target: support::target("radio"), key: Key::parse("test.tx_clock").unwrap(), value: Value::from(10) }, None).unwrap();
        assert!(admitted(&clock), "{clock:?}");
        let burst = run
            .submit(SessionAction::Vocabulary { ns: ns("test"), verb: Ident::parse("start_repeat").unwrap(), target: support::target("radio/tx"), at: None, params: BTreeMap::new() }, Some(&[0u8; 80]))
            .unwrap();
        assert!(admitted(&burst), "{burst:?}");
        let _ = manifest_of(run);
    }
}

#[test]
fn kg_04_a_provider_that_never_finishes_fails_the_run() {
    let probe = Probe::new();
    let assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe).never_finishing());
    let mut run = session(assembly, &profile_one());
    let begun = Instant::now();
    let _ = run.submit(gain(1.0), None);
    let waited = begun.elapsed();
    assert!(waited >= Wall::from_millis(4_500) && waited < Wall::from_secs(10), "{waited:?}");
    let manifest = manifest_of(run);
    assert!(matches!(manifest.termination.reason, Termination::Failed { stage: Stage::Run, .. }));
    assert!(failure(&manifest).starts_with("KC-21a: radio did not finish"), "{}", failure(&manifest));
}

#[test]
fn kc_21a_an_end_requested_during_the_wait_ends_it() {
    // The exit review's KC-21a clause, its re-check: the Provider never finishes the
    // Action and loses its device once it has taken it; the end the data thread
    // requests ends the wait long before the 5 s budget would.
    let probe = Probe::new();
    let loss = Arc::new((Mutex::new(false), Condvar::new()));
    let assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe).never_finishing().losing_device_on(loss.clone()));
    let mut run = session(assembly, &profile_one());
    let (done, returned) = mpsc::channel();
    // Detached: a joined worker would turn a wait that lasts its budget into a test timeout.
    std::thread::spawn(move || {
        let _ = run.submit(gain(1.0), None);
        let _ = done.send(run);
    });
    assert!(probe.wait_for("radio:took:UpdateParameter", Wall::from_secs(10)), "{:?}", probe.lines());
    *loss.0.lock().unwrap() = true;
    loss.1.notify_all();
    let run = returned.recv_timeout(Wall::from_secs(3)).expect("the end the Provider's loss requested ended the wait");
    let manifest = manifest_of(run);
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Policy { event: kind(EventKind::DEVICE_LOST) } });
}

#[test]
fn rs_16_an_action_the_freeze_discards_is_undelivered() {
    // F24's window: the Provider never takes the Action, KC-21a's wait returns as the
    // device loss requests the end, the Session logs the Action admitted, and RS-6
    // step 1 discards it from the queue. The termination section names it (RS-16).
    let probe = Probe::new();
    let assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe).deaf().losing_device(Wall::from_millis(100)));
    let mut run = session(assembly, &profile_one());
    let entry = run.submit(gain(1.0), None).unwrap();
    let Outcome::Admitted { dispatched, .. } = entry.outcome else { panic!("{entry:?}") };
    assert_eq!(dispatched.len(), 1, "{dispatched:?}");
    let manifest = manifest_of(run);
    assert_eq!(manifest.termination.undelivered, dispatched);
    assert!(!probe.lines().iter().any(|line| line.starts_with("radio:took:")));
}

#[test]
fn rs_16_a_sink_action_the_freeze_discards_is_undelivered() {
    // F24's window for a Sink: a recorder's verb queued for a recorder that never
    // takes it is discarded by RS-6 step 1 and named in `undelivered` (RS-16).
    let probe = Probe::new();
    let assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe).losing_device(Wall::from_millis(100)));
    let assembly = sink(assembly, RecordingSink::new("rec", &probe).deaf(), &probe);
    let mut run = session(assembly, &session_sink_profile());
    let entry = run
        .submit(
            SessionAction::Vocabulary { ns: ns("test"), verb: Ident::parse("sweep").unwrap(), target: Target::Output { output: Ident::parse("rec").unwrap() }, at: None, params: BTreeMap::new() },
            None,
        )
        .unwrap();
    let Outcome::Admitted { dispatched, .. } = entry.outcome else { panic!("{entry:?}") };
    assert_eq!(dispatched.len(), 1, "{dispatched:?}");
    let manifest = manifest_of(run);
    assert_eq!(manifest.termination.undelivered, dispatched);
    assert!(!probe.lines().iter().any(|line| line.starts_with("rec:action:")));
}

#[test]
fn kc_21a_a_module_s_own_submission_is_not_waited_for() {
    // The exit review's KC-21a clause, its scope: an Action an Executor submits from
    // its step (MA-14a) is not waited for. The Provider takes it and never finishes
    // it; the data thread goes on stepping the Executor all the same.
    let probe = Probe::new();
    let (spec, profile) = executor_docs();
    let mut assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe).never_finishing());
    let action = Action::UpdateParameter { target: support::target("radio"), key: Key::parse("test.gain").unwrap(), value: Value::num(3.0).unwrap(), class: UpdateClass::HardwareTimed, at: None };
    assembly.executors.insert(Ident::parse("exec").unwrap(), Box::new(support::ProbeExecutor::new("x", &probe).submitting(action)));
    let run = start_spec_run(&spec, &profile, assembly).unwrap();
    running(&run);
    assert!(probe.wait_for("radio:took:UpdateParameter", Wall::from_secs(10)));
    let stepped_after_submit =
        |lines: &[String]| lines.iter().position(|line| line.starts_with("x:submit:")).is_some_and(|at| lines[at..].iter().filter(|line| line.starts_with("x:step:")).count() >= 2);
    assert!(probe.wait_until(stepped_after_submit, Wall::from_secs(10)), "{:?}", probe.lines());
    running(&run);
    let lines = probe.lines();
    let submitted = lines.iter().position(|line| line.starts_with("x:submit:")).expect("the submission returned");
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
    fn check(&self, _section: &serde_json::Value, _effective: &BTreeMap<Ident, BTreeMap<Key, Value>>, _proposed: &BTreeMap<Ident, BTreeMap<Key, Value>>, _stage: CheckStage) -> Vec<Violation> {
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
    checks.register(Arc::new(Gate { section: ns("test.gate"), stages: vec![CheckStage::Runtime], loss: loss.clone(), probe: probe.clone(), fired: Mutex::new(false) }));
    rig.assembly.checks = checks;
    let assembly = provider(rig.assembly, "radio", ThreadedProvider::new("radio", "radio", &probe).losing_device_on(loss).draining_after_stop(Wall::from_millis(50)));
    let mut profile = profile_one();
    profile["environment"]["test.gate"] = serde_json::json!({});
    let mut run = session(assembly, &profile);
    let _ = run.submit(gain(2.0), None);
    let _ = manifest_of(run);
    assert!(!probe.lines().iter().any(|line| line.starts_with("radio:after_stop:")), "{:?}", probe.lines());
}

#[test]
fn kg_21a_submit_cannot_complete_before_the_booking_gate_opens() {
    // KC-21a, MA-14b: a submit returns once the Provider has finished with the Action it was dispatched, which the Provider acknowledges at its next
    // `recv`. With the Provider holding the Action after taking it, the submit does not return; once the gate opens it does, the Provider's decision made.
    let probe = Probe::new();
    let hold = Arc::new((Mutex::new(false), Condvar::new()));
    let assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe).hold_after_take(hold.clone()));
    let mut run = session(assembly, &profile_one());
    let (done, returned) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let entry = run.submit(gain(2.0), None).unwrap();
        done.send(()).unwrap();
        (run, entry)
    });
    assert!(probe.wait_for("radio:took:UpdateParameter", Wall::from_secs(5)), "{:?}", probe.lines());
    assert!(returned.recv_timeout(Wall::from_millis(150)).is_err(), "the submit returned while the Provider held the Action: {:?}", probe.lines());
    *hold.0.lock().unwrap() = true;
    hold.1.notify_all();
    let (run, entry) = worker.join().unwrap();
    assert!(admitted(&entry), "{entry:?}");
    assert!(probe.lines().iter().any(|line| line == "radio:finished:UpdateParameter"), "{:?}", probe.lines());
    let _ = manifest_of(run);
}

#[test]
fn kg_21a_a_run_that_ends_while_submit_waits_is_no_longer_running_when_it_returns() {
    // The wait returns early when the Run ends (paced.rs): the reply is then no evidence that the Provider accepted the Action, and a caller (the
    // server's `Ended` rule, EA-11) must find the Run no longer Running. The Provider takes the Action and holds it; the device is lost meanwhile.
    let probe = Probe::new();
    let assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe).never_finishing().losing_device(Wall::from_millis(100)));
    let mut run = session(assembly, &profile_one());
    let entry = run.submit(gain(1.0), None).unwrap();
    assert!(admitted(&entry), "the Action is logged admitted: {entry:?}");
    assert!(probe.lines().iter().any(|line| line == "radio:took:UpdateParameter"));
    assert!(!matches!(run.state(), RunState::Running {}), "{:?}", run.state());
    let _ = manifest_of(run);
}

#[test]
fn kg_04_a_simulation_session_does_not_wait() {
    // A stepped Provider receives its Actions in KC-21's round, after `submit`
    // dispatched them: a KC-21a wait before that round would time out and fail the Run.
    let probe = Probe::new();
    let clocks = Arc::new(ClockRegistry::new());
    let (authority, _) = SimAuthority::new(&clocks, mref("ezsdr.test.provider"), Pacing::FreeRunning);
    let assembly = provider(assembly_with(Box::new(authority), clocks), "radio", SteppedProvider::new("p", TestProvider::new("radio", 2), &probe));
    let mut profile = profile_one();
    profile["environment"] = serde_json::json!({});
    let mut run = session(assembly, &profile);
    let begun = Instant::now();
    let entry = run.submit(gain(4.0), None).unwrap();
    assert!(admitted(&entry), "{entry:?}");
    assert!(begun.elapsed() < Wall::from_secs(3), "{:?}", begun.elapsed());
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
    let assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe).publishing(100, Wall::from_millis(1)));
    let mut run = start_spec_run(&spec, &profile_one(), assembly).unwrap();
    running(&run);
    let begun = Instant::now();
    let result = run.run_until_end(after(&run, Wall::from_secs(10)));
    assert!(matches!(result, Err(RunHandleError::Ended { termination: Termination::Completed {} })), "{result:?}");
    assert!(begun.elapsed() < Wall::from_secs(5), "{:?}", begun.elapsed());
}

// ---------------------------------------------------------------- KG-6

#[test]
fn kg_06_a_prepare_that_hangs_fails_the_run_within_its_budget() {
    let probe = Probe::new();
    let assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe).with_prepare_delay(Wall::from_secs(30)));
    let begun = Instant::now();
    let run = start_spec_run(&spec_one(), &profile_one(), assembly).unwrap();
    assert!(begun.elapsed() < Wall::from_secs(12), "{:?}", begun.elapsed());
    assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Prepare, .. } }));
    let manifest = manifest_of(run);
    assert!(failure(&manifest).starts_with("KC-12a: prepare of radio"), "{}", failure(&manifest));
    assert!(manifest.termination.cleanup_failures.iter().any(|f| f.fragment.as_ref().is_some_and(|id| id.as_str() == "radio")), "{:?}", manifest.termination.cleanup_failures);
}

#[test]
fn kg_06_an_arm_that_hangs_fails_the_run_within_its_budget() {
    let probe = Probe::new();
    let assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe).with_arm_delay(Wall::from_secs(30)));
    let begun = Instant::now();
    let run = start_spec_run(&spec_one(), &profile_one(), assembly).unwrap();
    assert!(begun.elapsed() < Wall::from_secs(12), "{:?}", begun.elapsed());
    assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Arm, .. } }));
    let manifest = manifest_of(run);
    assert!(failure(&manifest).starts_with("KC-12a: arm of radio"), "{}", failure(&manifest));
}

#[test]
fn kg_06_the_simulation_class_prepares_on_the_callers_thread() {
    let probe = Probe::new();
    let clocks = Arc::new(ClockRegistry::new());
    let (authority, _) = SimAuthority::new(&clocks, mref("ezsdr.test.provider"), Pacing::FreeRunning);
    let assembly = provider(assembly_with(Box::new(authority), clocks), "radio", ThreadedProvider::new("radio", "radio", &probe));
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
    let t0 = run.start_instant().unwrap();
    let t0 = t0.ticks_in(run.now().domain()).unwrap();
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
        assert!(matches!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Arm, .. } }), "{name}");
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
    let at = TimePoint::new(root, time.now(root).unwrap().ticks_in(root).unwrap() + 5_000_000);
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
    assert!(seen.ticks_in(root).unwrap() >= at.ticks_in(root).unwrap() + 2_000_000, "{seen:?} {at:?}");
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
    let passed = TimePoint::new(root, time.now(root).unwrap().ticks_in(root).unwrap() - 1_000_000);
    time.schedule(passed, Box::new(|_| {})).expect("an instant already passed is accepted");
    assert_eq!(authority.next_wakeup(), Some(passed));
    let before = TimePoint::new(root, fired.ticks_in(root).unwrap() - 1);
    assert!(matches!(time.schedule(before, Box::new(|_| {})), Err(ezsdr_kernel::time::TimeError::InPast { .. })));
}

// ---------------------------------------------------------------- KG-10

#[test]
fn kg_10_a_device_lost_from_a_provider_thread_aborts_the_run() {
    let probe = Probe::new();
    let assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe).losing_device(Wall::from_millis(20)));
    let mut run = start_spec_run(&spec_one(), &profile_one(), assembly).unwrap();
    let begun = Instant::now();
    let result = run.run_until_end(after(&run, Wall::from_secs(10)));
    assert!(begun.elapsed() < Wall::from_secs(5), "{:?}", begun.elapsed());
    let lost = kind(EventKind::DEVICE_LOST);
    assert_eq!(result, Err(RunHandleError::Ended { termination: Termination::Stopped { cause: StopCause::Policy { event: lost.clone() } } }));
    let manifest = manifest_of(run);
    assert!(manifest.run.transitions.iter().any(|t| matches!(t.state, RunState::Stopping { mode: ezsdr_kernel::run::CleanupMode::Abort })), "{:?}", manifest.run.transitions);
    let radio = support::node_source("radio");
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
    assert_eq!(relations.iter().map(|r| r.target()).collect::<Vec<_>>(), [ClockDomainId::HOST_MONOTONIC, ClockDomainId::UTC]);
    assert!(relations.iter().all(|r| r.source() == rig.root));
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
    let assembly = provider(assembly_with(Box::new(RelatingSim(authority, relations)), clocks), "radio", TestProvider::new("radio", 2));
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

    let utc = paced().relations[1].clone();
    let host = ClockDomainId::HOST_MONOTONIC;
    let foreign = ezsdr_kernel::time::ClockRelation::new(
        host,
        utc.target(),
        TimePoint::new(host, 0),
        utc.offset(),
        utc.drift(),
        utc.drift_uncertainty(),
        utc.uncertainty(),
        utc.method().to_owned(),
        ezsdr_kernel::time::Validity { from: TimePoint::new(host, 0), to: None },
    )
    .unwrap();
    let rig = paced_with(|a| a.with_relations(vec![foreign]));
    let assembly = provider(rig.assembly, "radio", ThreadedProvider::new("radio", "radio", &probe));
    let manifest = manifest_of(start_spec_run(&spec_one(), &profile_one(), assembly).unwrap());
    assert!(manifest.clocks.relations.is_empty());
    assert_eq!(tm_18_failures(&manifest), ["TM-18: a relation whose source is not the primary root", "TM-18: the Authority published no relation of its root to utc"]);
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
    let assembly = provider(assembly_with(Box::new(PanickingRelations(authority)), clocks), "radio", ThreadedProvider::new("radio", "radio", &probe));
    let manifest = manifest_of(start_spec_run(&spec_one(), &profile_one(), assembly).unwrap());
    assert_eq!(manifest.run.execution_class, Some(ExecutionClass::HardwareInLoop));
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
    let (entry, manifest) = run.run_child(&spec_one(), &profile_one(), child, &mut |_| driven = true).unwrap();
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

// ---------------------------------------------------------------- KC-24a (#103)

/// One scheduled `test.gain` update at `offset_ticks` on the `radio` clock.
fn gain_entry(offset_ticks: i64, value: f64) -> serde_json::Value {
    serde_json::json!({
        "at": { "clock": "radio", "offset_ticks": offset_ticks },
        "action": {
            "kind": "update_parameter",
            "target": serde_json::to_value(support::target("radio")).unwrap(),
            "key": "test.gain",
            "value": value
        }
    })
}

/// The competing update an Executor submits: `test.gain = B = 2.0`.
fn competing_update() -> Action {
    Action::UpdateParameter { target: support::target("radio"), key: Key::parse("test.gain").unwrap(), value: Value::num(2.0).unwrap(), class: UpdateClass::HardwareTimed, at: None }
}

/// The Run's effective `test.gain`.
fn effective_gain(run: &RunHandle) -> Value {
    run.effective()[&Ident::parse("radio").unwrap()][&Key::parse("test.gain").unwrap()].clone()
}

#[test]
fn kc_24a_start_a_worker_submission_across_the_transition_is_admitted_after_the_batch() {
    // #103: the worker is released at `Point::BeforePublish`, where the admission lock
    // is held and the Run is still `Armed`; its submission is admitted after the start's
    // batch (`x:worker:ok:2`), in both classes. The test observes the release point, not
    // that the worker's call into the Kernel has reached the lock: a lock taken later
    // than `BeforePublish` lets the worker finish first inside the hook (`...RS-18`).
    for simulation in [false, true] {
        let probe = Probe::new();
        let (mut spec, mut profile) = executor_docs();
        spec["schedule"] = serde_json::json!([gain_entry(0, 1.0)]);
        let gate = gate();
        let mut assembly = if simulation {
            let clocks = Arc::new(ClockRegistry::new());
            let (authority, _) = SimAuthority::new(&clocks, mref("ezsdr.test.provider"), Pacing::FreeRunning);
            profile["environment"] = serde_json::json!({});
            provider(assembly_with(Box::new(authority), clocks), "radio", SteppedProvider::new("p", TestProvider::new("radio", 2), &probe).declaring("radio/rx", 1, 1))
        } else {
            provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe).publishing(100, Wall::from_millis(1)))
        };
        let log = with_seen(&mut assembly, &mut profile);
        assembly.executors.insert(Ident::parse("exec").unwrap(), Box::new(support::ProbeExecutor::new("x", &probe).submitting_from_worker(gate.clone(), competing_update())));
        let _release = OpenOnDrop(gate.clone());
        let trace = StartTrace::default();
        let recorded = trace.clone();
        let hook_gate = gate.clone();
        let hook_lines = probe.clone();
        let mut started = start_on_worker(
            &probe,
            move |point, p| {
                let r = p.read().expect("the Run is alive");
                recorded.push(point, r);
                if point == Point::BeforePublish {
                    let held = r.admission_held;
                    open(&hook_gate);
                    assert!(
                        hook_lines.wait_until(|l| l.iter().any(|x| x == "x:worker:begin"), Wall::from_secs(10)),
                        "{}",
                        infra(format!("{simulation}: the worker never reached its submit; {:?}", hook_lines.lines()))
                    );
                    if !held {
                        assert!(
                            hook_lines.wait_until(|l| l.iter().any(|x| x.starts_with("x:worker:ok:") || x.starts_with("x:worker:err:")), Wall::from_secs(10)),
                            "{}",
                            infra(format!("{simulation}: the ungated worker never returned from its submit; {:?}", hook_lines.lines()))
                        );
                    }
                }
            },
            move || start_spec_run(&spec, &profile, assembly),
        );
        assert!(
            wait_or_panicked(&mut started, &probe, |l| l.iter().any(|x| x.starts_with("x:worker:ok:") || x.starts_with("x:worker:err:"))),
            "{}",
            infra(format!("{simulation}: the worker's submission never returned; {:?}", probe.lines()))
        );
        let run = started.finish();
        let lines = probe.lines();
        assert!(lines.iter().any(|line| line == "x:worker:ok:2"), "{simulation}: {:?}", lines);
        assert!(!lines.iter().any(|line| line.starts_with("x:worker:err:")), "{simulation}: {:?}", lines);
        trace.assert_extent(&[Point::Entered, Point::BeforePublish, Point::BeforeDispatch, Point::Dispatching { next: 0 }]);
        assert_eq!(seen_for(&log, &Value::num(2.0).unwrap()), vec![Some(Value::num(1.0).unwrap())], "{simulation}: {:?}", lines);
        assert_eq!(effective_gain(&run), Value::num(2.0).unwrap(), "{simulation}");
        assert!(matches!(run.state(), RunState::Running {}), "{simulation}: {:?}", run.state());
        let manifest = run.finish_checked();
        assert!(manifest.termination.cleanup_failures.is_empty(), "{simulation}: {:?}", manifest.termination.cleanup_failures);
    }
}

/// T2a/T2b: two scheduled entries, the Executor's first step released at `release`.
fn data_thread_submission(release: Point) {
    let probe = Probe::new();
    let (mut spec, mut profile) = executor_docs();
    spec["schedule"] = serde_json::json!([gain_entry(0, 1.0), gain_entry(1, 3.0)]);
    let mut assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe).publishing(100, Wall::from_millis(1)));
    let log = with_seen(&mut assembly, &mut profile);
    let gate = gate();
    assembly.executors.insert(Ident::parse("exec").unwrap(), Box::new(support::ProbeExecutor::new("x", &probe).submitting_after(gate.clone(), competing_update())));
    let _release = OpenOnDrop(gate.clone());
    let trace = StartTrace::default();
    let recorded = trace.clone();
    let hook_gate = gate.clone();
    let hook_lines = probe.clone();
    let mut started = start_on_worker(
        &probe,
        move |point, p| {
            let r = p.read().expect("the Run is alive");
            recorded.push(point, r);
            if point == release {
                let held = r.admission_held;
                assert!(
                    hook_lines.wait_until(|l| l.iter().any(|x| x.starts_with("x:step:")), Wall::from_secs(10)),
                    "{}",
                    infra(format!("{release:?}: the Executor's step never began; {:?}", hook_lines.lines()))
                );
                open(&hook_gate);
                if !held {
                    assert!(
                        hook_lines.wait_until(|l| l.iter().any(|x| x.starts_with("x:submit:")), Wall::from_secs(10)),
                        "{}",
                        infra(format!("{release:?}: the ungated Executor never submitted; {:?}", hook_lines.lines()))
                    );
                }
            }
        },
        move || start_spec_run(&spec, &profile, assembly),
    );
    assert!(
        wait_or_panicked(&mut started, &probe, |l| l.iter().any(|x| x.starts_with("x:submit:ok:"))),
        "{}",
        infra(format!("{release:?}: the Executor's submission was not admitted; {:?}", probe.lines()))
    );
    let run = started.finish();
    let lines = probe.lines();
    assert!(lines.iter().any(|line| line == "x:submit:ok:3"), "{release:?}: {:?}", lines);
    assert_eq!(seen_for(&log, &Value::num(2.0).unwrap()), vec![Some(Value::num(3.0).unwrap())], "{release:?}: {:?}", lines);
    assert_eq!(effective_gain(&run), Value::num(2.0).unwrap(), "{release:?}");
    let stepped_after_submit = |l: &[String]| l.iter().position(|x| x.starts_with("x:submit:")).is_some_and(|at| l[at..].iter().any(|x| x.starts_with("x:step:")));
    assert!(probe.wait_until(stepped_after_submit, Wall::from_secs(10)), "{}", infra(format!("{release:?}: the data thread did not step after the submit; {:?}", lines)));
    trace.assert_extent(&[Point::Entered, Point::BeforePublish, Point::BeforeDispatch, Point::Dispatching { next: 0 }, Point::Dispatching { next: 1 }]);
    let manifest = run.finish_checked();
    assert!(manifest.termination.cleanup_failures.is_empty(), "{release:?}: {:?}", manifest.termination.cleanup_failures);
}

#[test]
fn kc_24a_start_the_data_threads_first_submission_follows_the_batch() {
    // #103: the Executor is released inside the window, at `BeforeDispatch`; on the
    // fixed path its submit is admitted after the whole batch whenever it runs (the old
    // order forces its completion first).
    data_thread_submission(Point::BeforeDispatch);
}

#[test]
fn kc_24a_start_a_submission_between_scheduled_entries_is_admitted_after_the_last() {
    // #103: the Executor is released inside the window, between entry 0's and entry 1's
    // dispatch; on the fixed path its submit is admitted after the whole batch whenever
    // it runs (the old order forces its completion first).
    data_thread_submission(Point::Dispatching { next: 1 });
}

#[test]
fn kc_24a_start_an_end_in_the_window_records_the_batch_as_undelivered() {
    // #103: an end requested while the start holds the lock cannot freeze the dispatch
    // before the batch's dispatch; the deaf Provider never takes it, so the freeze
    // records it as undelivered.
    let probe = Probe::new();
    let mut spec = spec_one();
    spec["schedule"] = serde_json::json!([gain_entry(0, 1.0)]);
    let loss = gate();
    let _release = OpenOnDrop(loss.clone());
    let assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe).publishing(100, Wall::from_millis(1)).deaf().losing_device_on(loss.clone()));
    let trace = StartTrace::default();
    let recorded = trace.clone();
    let readings: Arc<Mutex<Option<(Reading, Reading)>>> = Arc::new(Mutex::new(None));
    let hook_readings = readings.clone();
    let hook_loss = loss.clone();
    let hook_lines = probe.clone();
    let started = start_on_worker(
        &probe,
        move |point, p| {
            let r = p.read().expect("the Run is alive");
            recorded.push(point, r);
            if point == Point::BeforeDispatch {
                let first = r;
                let held = first.admission_held;
                open(&hook_loss);
                assert!(wait_end_requested(p), "{}", infra("the end the loss requests was never seen"));
                if !held {
                    assert!(hook_lines.wait_for("radio:stop", Wall::from_secs(10)), "{}", infra(format!("the Provider's stop never began; {:?}", hook_lines.lines())));
                }
                let after = p.read().expect("the Run is alive");
                *hook_readings.lock().unwrap() = Some((first, after));
            }
        },
        move || start_spec_run(&spec, &profile_one(), assembly),
    );
    let run = started.finish();
    let (first, after) = readings.lock().unwrap().take().expect("the hook ran");
    assert!(first.admission_held, "KC-24a: the admission lock is not held at BeforeDispatch");
    assert!(after.admission_held, "KC-24a: the admission lock was not held after the end request");
    assert!(after.end_requested, "the end the loss requested was not requested");
    assert!(!after.dispatch_frozen, "KC-24a: the dispatch froze before the batch's dispatch");
    trace.assert_extent(&[Point::Entered, Point::BeforePublish, Point::BeforeDispatch, Point::Dispatching { next: 0 }]);
    let manifest = run.finish_checked();
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Policy { event: kind(EventKind::DEVICE_LOST) } });
    assert_eq!(manifest.termination.undelivered, vec![ezsdr_kernel::event::ActionId(1)]);
    assert!(manifest.termination.cleanup_failures.is_empty(), "{:?}", manifest.termination.cleanup_failures);
    assert!(!probe.lines().iter().any(|line| line.starts_with("radio:took:") || line.starts_with("radio:after_stop:")), "{:?}", probe.lines());
}

#[test]
fn kc_24a_start_an_end_between_two_pushes_records_both_ids() {
    // #110 (S9, N110-7): an end requested after entry 0's push does not stop the dispatch
    // loop; entry 1 is pushed too (its gain is the Run's effective one), and the freeze --
    // which reached `Serialised` while the loop still held the admission lock -- then
    // records both ids as undelivered, since the deaf Provider took neither.
    let probe = Probe::new();
    let mut spec = spec_one();
    spec["schedule"] = serde_json::json!([gain_entry(0, 1.0), gain_entry(0, 2.0)]);
    let loss = gate();
    let _release = OpenOnDrop(loss.clone());
    let assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe).publishing(100, Wall::from_millis(1)).deaf().losing_device_on(loss.clone()));
    let trace = StartTrace::default();
    let recorded = trace.clone();
    let freezes: Arc<Mutex<Vec<FreezePoint>>> = Arc::new(Mutex::new(Vec::new()));
    let hook_freezes = freezes.clone();
    let first: Arc<Mutex<Option<Reading>>> = Arc::new(Mutex::new(None));
    let hook_first = first.clone();
    let hook_loss = loss.clone();
    let started = start_on_worker(
        &probe,
        move |point, p| {
            let r = p.read().expect("the Run is alive");
            recorded.push(point, r);
            if point == Point::Entered {
                let recorded = hook_freezes.clone();
                p.on_freeze(move |point, _reading| recorded.lock().unwrap_or_else(|e| e.into_inner()).push(point));
            }
            if point == (Point::Dispatching { next: 1 }) {
                *hook_first.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
                open(&hook_loss);
                let begun = Instant::now();
                loop {
                    if hook_freezes.lock().unwrap_or_else(|e| e.into_inner()).contains(&FreezePoint::Serialised) {
                        break;
                    }
                    assert!(begun.elapsed() < Wall::from_secs(10), "{}", infra("the data thread's freeze never reached Serialised within 10 s"));
                    std::thread::yield_now();
                }
            }
        },
        move || start_spec_run(&spec, &profile_one(), assembly),
    );
    let run = started.finish();
    let reading = first.lock().unwrap_or_else(|e| e.into_inner()).expect("the hook ran at Dispatching { next: 1 }");
    assert!(reading.admission_held, "KC-24a: the dispatch does not hold the admission lock between the two pushes");
    assert_eq!(effective_gain(&run), Value::num(2.0).unwrap(), "KC-24a: the second entry was not pushed after the end request; the freeze took its id from start_batch");
    let manifest = run.finish_checked();
    trace.assert_extent(&[Point::Entered, Point::BeforePublish, Point::BeforeDispatch, Point::Dispatching { next: 0 }, Point::Dispatching { next: 1 }]);
    assert_eq!(
        manifest.termination.undelivered,
        vec![ezsdr_kernel::event::ActionId(1), ezsdr_kernel::event::ActionId(2)],
        "KC-18, RS-16: the batch's second entry is in neither the queue nor the Manifest"
    );
    assert!(!probe.lines().iter().any(|line| line.starts_with("radio:took:")), "the deaf Provider took an Action: {:?}", probe.lines());
    assert!(manifest.termination.cleanup_failures.is_empty(), "{:?}", manifest.termination.cleanup_failures);
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Policy { event: kind(EventKind::DEVICE_LOST) } });
}

#[test]
fn kc_24a_start_the_lock_is_free_at_the_wait() {
    // #103: the lock is free at `Released`, so the Executor's submission is admitted
    // while the start waits for the Provider that holds the batch's Action; the start is
    // still waiting while the Provider holds, and it returns a live Run once the
    // Provider acknowledges. A guard held across the wait is the kill (asserted at
    // `Released`).
    let probe = Probe::new();
    let (mut spec, mut profile) = executor_docs();
    spec["schedule"] = serde_json::json!([gain_entry(0, 1.0)]);
    let hold = gate();
    let gate_x = gate();
    let mut assembly = provider(paced().assembly, "radio", ThreadedProvider::new("radio", "radio", &probe).publishing(100, Wall::from_millis(1)).hold_after_take(hold.clone()));
    // The check is installed as in T1/T2; T6's verdict is the `Released` read below.
    let _log = with_seen(&mut assembly, &mut profile);
    assembly.executors.insert(Ident::parse("exec").unwrap(), Box::new(support::ProbeExecutor::new("x", &probe).submitting_after(gate_x.clone(), competing_update())));
    let _release_hold = OpenOnDrop(hold.clone());
    let _release_gate_x = OpenOnDrop(gate_x.clone());
    let trace = StartTrace::default();
    let recorded = trace.clone();
    let hook_gate = gate_x.clone();
    let mut started = start_on_worker(
        &probe,
        move |point, p| {
            let r = p.read().expect("the Run is alive");
            recorded.push(point, r);
            if point == Point::Released {
                open(&hook_gate);
                assert!(!r.admission_held, "KC-24a: the admission lock is still held when the start releases it; KC-21a's wait would run under it");
            }
        },
        move || start_spec_run(&spec, &profile, assembly),
    );
    assert!(wait_or_panicked(&mut started, &probe, |l| l.iter().any(|x| x == "radio:took:UpdateParameter")), "{}", infra(format!("{:?}", probe.lines())));
    assert!(
        wait_or_panicked(&mut started, &probe, |l| l.iter().any(|x| x.starts_with("x:submit:ok:"))),
        "{}",
        infra("the Executor's submission was not admitted while the control thread waited for the Provider")
    );
    assert!(!started.returned(), "KC-21a: the start returned while the Provider still held the batch's Action: its wait was skipped");
    open(&hold);
    let run = started.finish();
    assert!(matches!(run.state(), RunState::Running {}), "{}", infra(format!("KC-21a's 5 s budget expired and failed the Run: {:?}", run.state())));
    let lines = probe.lines();
    assert!(lines.iter().any(|line| line == "x:submit:ok:2"), "{:?}", lines);
    assert_eq!(effective_gain(&run), Value::num(2.0).unwrap(), "{:?}", lines);
    trace.assert_extent(&[Point::Entered, Point::BeforePublish, Point::BeforeDispatch, Point::Dispatching { next: 0 }, Point::Released]);
    let manifest = run.finish_checked();
    assert!(manifest.termination.cleanup_failures.is_empty(), "{:?}", manifest.termination.cleanup_failures);
}

#[test]
fn kc_24a_a_wakes_schedule_holds_no_lock_and_blocks_no_one() {
    // #120, KC-24a, KC-46a: the data thread's ordinary wake is parked inside the Authority's
    // `schedule` (the handle not yet returned). It holds no lock of the coordinator's, so a
    // Module thread's `disconnect_signal` (its own wake) returns, the cleanup's freeze runs
    // to `Done` and a Module's submission during it is refused, all while that `schedule`
    // is still parked. The parked wake's handle is accepted after the cleanup's freeze took
    // the wake list and closed it, so its publisher holds no handle and cancels nothing.
    // The fixture is the Spec Run of `executor_docs()`: a Session's implicit Spec has no
    // graph, so no Executor -- and so no shared submitter -- is prepared in one. No
    // `next_wakeup` runs, so no wake callback fires and W's cancel returns true.
    // Deschedule: W (the signal's thread) and W_f start only after `schedule:parked` was
    // seen; the parked Schedule's entry in the log, with its `held`, was written before the
    // park, so a late W or W_f only delays the infra-bounded waits; the data thread stays
    // parked on G until the refusal and the freeze's `Done` were read.
    let probe = Probe::new();
    let gate_s = gate();
    let _release = OpenOnDrop(gate_s.clone());
    let log = ClockLog::default();
    let parked = Arc::new(AtomicBool::new(false));
    let on_schedule = {
        let probe = probe.clone();
        let gate = gate_s.clone();
        let parked = parked.clone();
        move |_call: &Call| {
            if std::thread::current().name() != Some("ezsdr-data") || parked.swap(true, Ordering::SeqCst) {
                return;
            }
            probe.record("schedule:parked");
            wait(&gate);
        }
    };
    let marks = release();
    let (rig, _) = paced_parked(|authority| authority);
    let mut assembly = rig.assembly;
    assembly.authority = Box::new(WatchedAuthority::new(assembly.authority, log.clone()).with_hooks(Hooks { on_schedule: Some(Arc::new(on_schedule)), ..Hooks::default() }));
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(ThreadedProvider::new("radio", "radio", &probe).marking_on("test.MARK", marks.clone())));
    let slot: Arc<Mutex<Option<Arc<dyn ActionSubmitter>>>> = Arc::new(Mutex::new(None));
    assembly.executors.insert(Ident::parse("exec").unwrap(), Box::new(support::ProbeExecutor::new("x", &probe).sharing_submitter(slot.clone())));
    let (freeze_tx, freeze_rx) = mpsc::channel();
    let (spec, profile) = executor_docs();
    let started = start_on_worker(
        &probe,
        move |point, p| {
            if point == Point::Entered {
                let freeze_tx = freeze_tx.clone();
                p.on_freeze(move |point, reading| {
                    let _ = freeze_tx.send((point, reading));
                });
            }
        },
        move || start_spec_run(&spec, &profile, assembly),
    );
    let run = started.finish();
    let signal = run.disconnect_signal();
    let submitter = slot.lock().unwrap_or_else(|e| e.into_inner()).clone().expect("the Executor was prepared");
    release_one(&marks);
    assert!(probe.wait_for("schedule:parked", Wall::from_secs(10)), "{}", infra("the data thread's wake did not reach schedule within 10 s"));
    // (1) the parked `schedule` holds nothing.
    let parked_calls: Vec<Call> = log.calls().into_iter().filter(|call| call.kind == CallKind::Schedule && call.handle.is_none()).collect();
    assert_eq!(parked_calls.len(), 1, "the data thread's wake is the one Schedule not yet returned: {parked_calls:?}");
    assert_eq!(parked_calls[0].held, Held::default(), "KC-24a: the wake called `schedule` under a lock: {:?}", parked_calls[0]);
    // (2) a Module thread's `disconnect_signal` returns while that `schedule` is parked.
    let (id_tx, id_rx) = mpsc::channel();
    let (signalled_tx, signalled_rx) = mpsc::channel();
    let w = std::thread::spawn(move || {
        let _ = id_tx.send(std::thread::current().id());
        signal();
        let _ = signalled_tx.send(());
    });
    let w_id = id_rx.recv_timeout(Wall::from_secs(10)).expect("W started");
    if signalled_rx.recv_timeout(Wall::from_secs(10)).is_err() {
        open(&gate_s);
        panic!("{}", infra("`disconnect_signal` did not return within 10 s while the wake's schedule was parked: it waits for a lock the wake holds"));
    }
    // (3) its own wake's `schedule` ran under no lock either.
    let w_calls: Vec<Call> = log.calls().into_iter().filter(|call| call.kind == CallKind::Schedule && call.thread == w_id).collect();
    assert_eq!(w_calls.len(), 1, "W's wake made one Schedule: {w_calls:?}");
    assert_eq!(w_calls[0].held, Held::default(), "KC-24a: the signal's wake called `schedule` under a lock: {:?}", w_calls[0]);
    let (manifest_tx, manifest_rx) = mpsc::channel();
    let finisher = std::thread::spawn(move || {
        let _ = manifest_tx.send(run.finish());
    });
    // (4) the freeze reaches `Done` while the data thread's `schedule` is still parked.
    let deadline = Instant::now() + Wall::from_secs(10);
    let done = loop {
        match freeze_rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok((FreezePoint::Done, reading)) => break reading,
            Ok(_) => {}
            Err(_) => {
                open(&gate_s);
                panic!("{}", infra("the cleanup's freeze did not reach Done within 10 s while the wake's schedule was parked"));
            }
        }
    };
    assert!(done.held.freeze && !done.held.admission && done.dispatch_frozen, "KC-24a: Done is not under the gate alone with dispatch frozen: {done:?}");
    // (5) a submission during the freeze is refused at RS-6, not blocked.
    let action = Action::UpdateParameter { target: support::target("radio"), key: Key::parse("test.gain").unwrap(), value: Value::num(2.0).unwrap(), class: UpdateClass::HardwareTimed, at: None };
    let (done_tx, done_rx) = mpsc::channel();
    let w_s = std::thread::spawn(move || {
        let _ = done_tx.send(submitter.submit(action));
    });
    let result = match done_rx.recv_timeout(Wall::from_secs(10)) {
        Ok(result) => result,
        Err(_) => {
            open(&gate_s);
            panic!("{}", infra("the submission during the freeze did not return within 10 s"));
        }
    };
    let violations = match result {
        Ok(id) => panic!("RS-6: a submission during the freeze was admitted with {id:?}"),
        Err(violations) => violations,
    };
    assert_eq!(violations.len(), 1, "KC-24a: the refusal is one violation: {violations:?}");
    assert_eq!(violations[0].check.as_str(), "ezsdr.dispatch", "KC-24a: the refusal is not RS-6's: {violations:?}");
    assert_eq!(violations[0].reason, "RS-6: dispatch is frozen", "KC-24a: the refusal is not RS-6's: {violations:?}");
    // (6) the parked wake's handle is not cancelled by anyone once the Authority returns it: the list is closed.
    open(&gate_s);
    let manifest = match manifest_rx.recv_timeout(Wall::from_secs(10)) {
        Ok(manifest) => manifest,
        Err(_) => panic!("{}", infra("the freeze did not return a Manifest within 10 s after the gate opened")),
    };
    finisher.join().expect("W_f does not panic");
    w.join().expect("W does not panic");
    w_s.join().expect("W_s does not panic");
    assert!(manifest.termination.cleanup_failures.is_empty(), "{:?}", manifest.termination.cleanup_failures);
    assert_eq!(manifest.termination.reason, client_disconnect(), "{:?}", manifest.termination);
    let calls = log.calls();
    let w_handle = w_calls[0].handle.expect("W's schedule returned its handle");
    let w_cancels = cancels_of(&calls, w_handle);
    assert_eq!(w_cancels.len(), 1, "RS-6 step 1: the freeze cancels W's wake once; calls: {calls:?}");
    assert!(w_cancels[0].held.freeze && w_cancels[0].returned == Some(true), "KC-24a: W's wake is cancelled under the gate and was pending: {:?}", w_cancels[0]);
    let late: Vec<&Call> = calls.iter().filter(|call| call.kind == CallKind::Schedule && call.thread != w_id).collect();
    assert_eq!(late.len(), 1, "the data thread made the one other Schedule: {calls:?}");
    let late_cancels = cancels_of(&calls, late[0].handle.expect("the parked schedule returned its handle"));
    assert!(late_cancels.is_empty(), "KC-46a: the wake accepted after the cleanup's freeze was cancelled, by the freeze or its publisher; calls: {calls:?}");
}

#[test]
fn kc_24a_overlapping_freezes_serialise_and_cancel_before_the_single_stop() {
    // KC-24a, RS-6 step 1: the data thread's freeze and the cleanup's freeze are
    // serialised by the freeze gate: the second takes the gate only after the first's
    // cancels returned, and the Provider is stopped once, after them.
    // Deschedule: the second freeze can only start after `cancel:entered` (the test
    // starts `finish` after it), and G_c opens only after the second `Serialise` was
    // seen, so no descheduling reorders the asserted lines.
    let probe = Probe::new();
    let loss = gate();
    let _release = OpenOnDrop(loss.clone());
    let cancel_gate = gate();
    let _cancel_release = OpenOnDrop(cancel_gate.clone());
    let (mut spec, profile) = executor_docs();
    spec["schedule"] = serde_json::json!([{
        "at": { "clock": "radio", "offset_ticks": 3_600_000_000i64 },
        "action": { "kind": "stop", "target": null }
    }]);
    let log = ClockLog::default();
    let far = Arc::new(Mutex::new(None::<ScheduleHandle>));
    let parked = Arc::new(AtomicBool::new(false));
    let on_cancel = {
        let probe = probe.clone();
        let gate = cancel_gate.clone();
        let far = far.clone();
        let parked = parked.clone();
        move |call: &Call| {
            if *far.lock().unwrap_or_else(|e| e.into_inner()) != call.handle || parked.swap(true, Ordering::SeqCst) {
                return;
            }
            probe.record("cancel:entered");
            wait(&gate);
        }
    };
    // The line is recorded after the inner `cancel` returned, so the asserted orders observe the return, not the entry.
    let on_cancel_return = {
        let probe = probe.clone();
        let far = far.clone();
        move |call: &Call| {
            if *far.lock().unwrap_or_else(|e| e.into_inner()) == call.handle {
                probe.record("cancel:returned");
            }
        }
    };
    let (rig, _) = paced_parked(|authority| authority);
    let mut assembly = rig.assembly;
    assembly.authority = Box::new(WatchedAuthority::new(assembly.authority, log.clone()).with_hooks(Hooks {
        on_cancel: Some(Arc::new(on_cancel)),
        on_cancel_return: Some(Arc::new(on_cancel_return)),
        ..Hooks::default()
    }));
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(ThreadedProvider::new("radio", "radio", &probe).publishing(100, Wall::from_millis(1)).deaf().losing_device_on(loss.clone())));
    let slot: Arc<Mutex<Option<Arc<dyn ActionSubmitter>>>> = Arc::new(Mutex::new(None));
    assembly.executors.insert(Ident::parse("exec").unwrap(), Box::new(support::ProbeExecutor::new("x", &probe).sharing_submitter(slot.clone())));
    let freezes = Arc::new(Mutex::new(Vec::<(FreezePoint, FreezeReading)>::new()));
    let hook_freezes = freezes.clone();
    let hook_lines = probe.clone();
    let started = start_on_worker(
        &probe,
        move |point, p| {
            if point == Point::Entered {
                let hook_freezes = hook_freezes.clone();
                let hook_lines = hook_lines.clone();
                p.on_freeze(move |point, reading| {
                    hook_lines.record(format!("freeze:{point:?}:{:?}", reading.thread));
                    hook_freezes.lock().unwrap_or_else(|e| e.into_inner()).push((point, reading));
                });
            }
        },
        move || start_spec_run(&spec, &profile, assembly),
    );
    let run = started.finish();
    let start = run.start_instant().expect("the Run was armed").ticks_in(rig.root).expect("the start is on the primary root");
    let (handle, _) = far_future_stop(&log.calls(), start);
    *far.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
    let submitter = slot.lock().unwrap_or_else(|e| e.into_inner()).clone().expect("the Executor was prepared");
    let action = Action::UpdateParameter { target: support::target("radio"), key: Key::parse("test.gain").unwrap(), value: Value::num(2.0).unwrap(), class: UpdateClass::HardwareTimed, at: None };
    let id = submitter.submit(action).expect("KC-24: the submission is admitted before the loss");
    open(&loss);
    assert!(probe.wait_for("cancel:entered", Wall::from_secs(10)), "{}", infra("the data thread's freeze did not reach the cancel within 10 s"));
    let (manifest_tx, manifest_rx) = mpsc::channel();
    let finisher = std::thread::spawn(move || {
        let _ = manifest_tx.send(run.finish());
    });
    let second_freeze = |lines: &[String]| {
        let serialise: Vec<&String> = lines.iter().filter(|line| line.starts_with("freeze:Serialise:")).collect();
        serialise.len() >= 2 && serialise.iter().skip(1).any(|thread| *thread != serialise[0])
    };
    assert!(probe.wait_until(second_freeze, Wall::from_secs(10)), "{}", infra("the second freeze did not reach Serialise within 10 s"));
    open(&cancel_gate);
    let manifest = match manifest_rx.recv_timeout(Wall::from_secs(10)) {
        Ok(manifest) => manifest,
        Err(_) => panic!("{}", infra("W_f did not return a Manifest within 10 s after the gate opened")),
    };
    finisher.join().expect("W_f does not panic");
    let lines = probe.lines();
    let serialise: Vec<String> = lines.iter().filter_map(|line| line.strip_prefix("freeze:Serialise:").map(str::to_owned)).collect();
    let second_thread = serialise.iter().find(|thread| **thread != serialise[0]).cloned().expect("the second freeze's thread differs from the first's");
    let calls = log.calls();
    let cancels: Vec<&Call> = calls.iter().filter(|call| call.kind == CallKind::Cancel && call.handle == Some(handle)).collect();
    assert_eq!(cancels.len(), 1, "RS-6 step 1: the overlapping performances cancel H once; calls: {calls:?}");
    assert!(!cancels[0].held.admission, "KC-24a: H's cancel ran under the admission lock; calls: {calls:?}");
    assert!(cancels[0].held.freeze, "KC-24a: H's cancel ran outside the freeze gate; calls: {calls:?}");
    let readings = freezes.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let second_serialised =
        readings.iter().find(|(point, reading)| *point == FreezePoint::Serialised && format!("{:?}", reading.thread) == second_thread).expect("the second freeze reached Serialised");
    assert!(second_serialised.1.held.freeze, "KC-24a: the second freeze's Serialised ran outside the freeze gate: {second_serialised:?}");
    assert_eq!(lines.iter().filter(|line| *line == "radio:stop").count(), 1, "KC-39: the Provider is stopped once; lines: {lines:?}");
    let at = |line: &str| lines.iter().position(|observed| observed == line).unwrap_or_else(|| panic!("the line {line} is missing; lines: {lines:?}"));
    let returned = at("cancel:returned");
    assert!(returned < at("radio:stop"), "KC-24a: the freeze's cancel of H did not return before the Provider stop; lines: {lines:?}");
    assert!(returned < at(&format!("freeze:Serialised:{second_thread}")), "KC-24a: the second freeze passed the gate before the first freeze's cancel returned; lines: {lines:?}");
    assert_eq!(manifest.termination.undelivered, vec![id], "RS-16: the freeze discards exactly the queued Action; lines: {lines:?}");
    assert!(manifest.termination.cleanup_failures.is_empty(), "the cleanup recorded failures: {:?}", manifest.termination.cleanup_failures);
}

// ---------------------------------------------------------------- KC-24a: the wake, the withdrawal and the agenda (#111)

/// A cell a hook fills with a thread id, for the test thread to read once the Run passed the point.
type ThreadCell = Arc<Mutex<Option<std::thread::ThreadId>>>;

/// The thread cells the hooks fill: W_c's own thread, the first performance of RS-6 step 1 (the
/// data thread in these tests) and that freeze's `Done`.
#[derive(Clone)]
struct Threads {
    control: ThreadCell,
    data: ThreadCell,
    done: ThreadCell,
    /// The Run's probe, stored at `Entered`.
    run: Arc<Mutex<Option<start_probe::Probe>>>,
}

impl Threads {
    fn new() -> Threads {
        Threads { control: Arc::new(Mutex::new(None)), data: Arc::new(Mutex::new(None)), done: Arc::new(Mutex::new(None)), run: Arc::new(Mutex::new(None)) }
    }

    /// A reading of the Run, or `None` before `Entered` and after the Run is gone.
    fn read(&self) -> Option<Reading> {
        self.run.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(start_probe::Probe::read)
    }

    /// Waits, as a condition, until a reading satisfies `pred`; fails through `infra` after 10 s.
    fn wait_reading(&self, what: &str, pred: impl Fn(&Reading) -> bool) -> Reading {
        let deadline = Instant::now() + Wall::from_secs(10);
        loop {
            if let Some(reading) = self.read().filter(&pred) {
                return reading;
            }
            assert!(Instant::now() < deadline, "{}", infra(format!("the Run's reading did not show {what} within 10 s: {:?}", self.read())));
            std::thread::yield_now();
        }
    }
}

/// W_c: a worker that installs a `start_probe` hook for its whole life and runs `body` with it.
/// `Withdrawal`, `Withdrawn`, `Scheduled` and `BeforeAgendaLock` are reached on the thread that
/// makes the control call, so the hook must outlive the start's own call; the test thread waits
/// for the worker's value through a channel with the 10 s `infra` backstop.
struct ControlWorker<T> {
    result: mpsc::Receiver<std::thread::Result<T>>,
    worker: std::thread::JoinHandle<()>,
}

impl<T> ControlWorker<T> {
    /// Waits for W_c's value, resuming a panic it caught as the test's own failure.
    fn join(self) -> T {
        let result = match self.result.recv_timeout(Wall::from_secs(10)) {
            Ok(result) => result,
            Err(_) => panic!("{}", infra("the control worker did not return within 10 s")),
        };
        self.worker.join().expect("the control worker does not panic outside its body");
        match result {
            Ok(value) => value,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }
}

/// Runs `body` on W_c with `hook` installed there for the thread's whole life.
fn on_control_worker<T: Send + 'static>(hook: impl FnMut(Point, &start_probe::Probe) + Send + 'static, body: impl FnOnce() -> T + Send + 'static) -> ControlWorker<T> {
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::Builder::new()
        .name("ezsdr-test-control".to_owned())
        .spawn(move || {
            let _installed = start_probe::install(hook);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
            let _ = tx.send(result);
        })
        .expect("the control worker spawns");
    ControlWorker { result: rx, worker }
}

/// W_c's value from `start_horizon_run`: the call's result (mapped to `()`) and the Run.
type EndedRun = (Result<(), RunHandleError>, RunHandle);

/// Starts a Spec Run on W_c and returns its hour-away horizon tick: `spec_one()`/`profile_one()` inside, the Run
/// reported `Running`, then either `wait_for` of `test.MARK` (`wait_for_mark`) or `run_until_end`, its result mapped
/// to `()`. The tick arrives within 10 s or the test fails through `infra`, the backstop every wait here has.
fn start_horizon_run(hook: impl FnMut(Point, &start_probe::Probe) + Send + 'static, assembly: Assembly, wait_for_mark: bool) -> (ControlWorker<EndedRun>, i64) {
    let (tick_tx, tick_rx) = mpsc::channel();
    let (spec, profile) = (spec_one(), profile_one());
    let worker = on_control_worker(hook, move || {
        let mut run = start_spec_run(&spec, &profile, assembly).expect("the Run starts");
        running(&run);
        let horizon = after(&run, Wall::from_secs(3600));
        let _ = tick_tx.send(horizon.ticks_in(horizon.domain()).expect("the horizon is on the primary root"));
        let result = if wait_for_mark { run.wait_for(&[kind("test.MARK")], 0, horizon).map(|_| ()) } else { run.run_until_end(horizon) };
        (result, run)
    });
    let tick = match tick_rx.recv_timeout(Wall::from_secs(10)) {
        Ok(tick) => tick,
        Err(_) => panic!("{}", infra("the control worker did not start the Run within 10 s")),
    };
    (worker, tick)
}

/// The freeze hook these tests share: it records `freeze:<point>:<thread>` at every point, stores
/// the first performing thread (the data thread's freeze #1) and that freeze's `Done`. It only
/// records; every assertion runs on the test thread.
fn freeze_trace(probe: &Probe, threads: &Threads) -> Box<dyn Fn(FreezePoint, FreezeReading) + Send + Sync> {
    let lines = probe.clone();
    let threads = threads.clone();
    Box::new(move |point, reading| {
        threads.data.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert(reading.thread);
        if point == FreezePoint::Done {
            threads.done.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert(reading.thread);
        }
        lines.record(format!("freeze:{point:?}:{:?}", reading.thread));
    })
}

/// W_c's `Entered` handling: stores W_c's own thread id and installs the `freeze_trace` (the start hands out the
/// probe at that point). Returns whether the point was `Entered`.
fn entered(point: Point, p: &start_probe::Probe, probe: &Probe, threads: &Threads) -> bool {
    if point != Point::Entered {
        return false;
    }
    *threads.control.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::thread::current().id());
    *threads.run.lock().unwrap_or_else(|e| e.into_inner()) = Some(p.clone());
    p.on_freeze(freeze_trace(probe, threads));
    true
}

/// The thread a hook recorded, read once the Run passed the point.
fn recorded_thread(cell: &ThreadCell) -> std::thread::ThreadId {
    cell.lock().unwrap_or_else(|e| e.into_inner()).expect("the hook recorded the thread")
}

/// The one `Schedule` call `thread` made whose instant in the primary root is `tick`: the handle a
/// test names. Its exactly-one match is asserted before the handle is used (KC-20, KC-29b).
fn scheduled_at(calls: &[Call], tick: i64, thread: std::thread::ThreadId) -> (ScheduleHandle, Call) {
    let matching: Vec<&Call> = calls.iter().filter(|call| call.kind == CallKind::Schedule && call.sample == Some(tick) && call.thread == thread).collect();
    assert_eq!(matching.len(), 1, "the fixture scheduled exactly one handle at {tick} on {thread:?}: {calls:?}");
    (matching[0].handle.expect("the schedule returned its handle"), matching[0].clone())
}

/// The one `Schedule` call `thread` made — the late wake in these tests.
fn scheduled_by(calls: &[Call], thread: std::thread::ThreadId) -> (ScheduleHandle, Call) {
    let matching: Vec<&Call> = calls.iter().filter(|call| call.kind == CallKind::Schedule && call.thread == thread).collect();
    assert_eq!(matching.len(), 1, "{thread:?} made exactly one schedule: {calls:?}");
    (matching[0].handle.expect("the schedule returned its handle"), matching[0].clone())
}

/// Every `Cancel` of `handle`, in call order (KC-24a's per-handle counts).
fn cancels_of(calls: &[Call], handle: ScheduleHandle) -> Vec<Call> {
    calls.iter().filter(|call| call.kind == CallKind::Cancel && call.handle == Some(handle)).cloned().collect()
}

#[test]
fn kc_24a_freeze_cancels_the_wake_handle_exactly_once() {
    // #111, RS-6 step 1, KC-46a: the data thread's freeze cancels the horizon it drained; the
    // unconditional wake it schedules afterwards is taken and cancelled once, by the control
    // cleanup's freeze, after that wake's `schedule` returned. N111-6 leaves the wake uncancelled.
    // Deschedule: the test opens G_r only after the Run's reading shows the data thread's wake
    // published (not merely scheduled), so the control freeze meets the handle in the list and
    // never meets a handle still in flight; W_c is parked in the `next_wakeup` return hook until
    // then, so no other order is reachable. A descheduled data thread only delays the reading.
    let probe = Probe::new();
    let loss = gate();
    let _release = OpenOnDrop(loss.clone());
    let gate_r = gate();
    let _release_r = OpenOnDrop(gate_r.clone());
    let armed = Arc::new(AtomicBool::new(false));
    let threads = Threads::new();
    let log = ClockLog::default();
    let on_next_wakeup_return = {
        let probe = probe.clone();
        let gate = gate_r.clone();
        let armed = armed.clone();
        let parked = Arc::new(AtomicBool::new(false));
        move |_result: Option<TimePoint>| {
            if !armed.load(Ordering::SeqCst) || parked.swap(true, Ordering::SeqCst) {
                return;
            }
            probe.record("loop:parked");
            wait(&gate);
        }
    };
    let (rig, park) = paced_parked(|authority| authority);
    let mut assembly = rig.assembly;
    assembly.authority = Box::new(WatchedAuthority::new(assembly.authority, log.clone()).with_hooks(Hooks { on_next_wakeup_return: Some(Arc::new(on_next_wakeup_return)), ..Hooks::default() }));
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(ThreadedProvider::new("radio", "radio", &probe).publishing(100, Wall::from_millis(1)).losing_device_on(loss.clone())));
    let hook = {
        let probe = probe.clone();
        let threads = threads.clone();
        move |point: Point, p: &start_probe::Probe| {
            entered(point, p, &probe, &threads);
        }
    };
    let (worker, tick) = start_horizon_run(hook, assembly, false);
    assert!(park.wait_parked_until(tick, Wall::from_secs(10)), "{}", infra("the control loop never waited for the horizon"));
    armed.store(true, Ordering::SeqCst);
    open(&loss);
    assert!(probe.wait_for("loop:parked", Wall::from_secs(10)), "{}", infra("the horizon's cancel did not wake the control loop within 10 s"));
    threads.wait_reading("the data thread's wake published", |reading| reading.wake_handles >= 1);
    open(&gate_r);
    let (result, run) = worker.join();
    assert!(matches!(result, Err(RunHandleError::Ended { .. })), "{result:?}");
    let manifest = manifest_of(run);
    let calls = log.calls();
    let (horizon, _) = scheduled_at(&calls, tick, recorded_thread(&threads.control));
    let (wake, _) = scheduled_by(&calls, recorded_thread(&threads.data));
    let horizon_cancels = cancels_of(&calls, horizon);
    assert_eq!(horizon_cancels.len(), 1, "RS-6 step 1: the freeze cancels the horizon once; calls: {calls:?}");
    let wake_cancels = cancels_of(&calls, wake);
    assert_eq!(wake_cancels.len(), 1, "RS-6 step 1: the freeze takes and cancels the wake handle once; lines: {:?}; calls: {calls:?}", probe.lines());
    for call in calls.iter().filter(|call| call.kind == CallKind::Cancel) {
        assert!(!call.held.admission && !call.held.machine && call.held.freeze, "KC-24a: a cancel runs outside the admission and machine locks and under the freeze gate: {call:?}");
    }
    assert!(manifest.termination.cleanup_failures.is_empty(), "{:?}", manifest.termination.cleanup_failures);
}

#[test]
fn kc_46a_a_wake_scheduled_across_the_cleanups_freeze_is_never_cancelled() {
    // KC-46a: a wake whose `schedule` returns after the cleanup's freeze took the wake list is
    // held by no one: its publisher cancels nothing and the wake stays a no-op. The data thread's
    // first `now` after its freeze's `Done` is its end-wake's own read, taken after the
    // closed-list check and before `schedule`; the control freeze takes and closes the list
    // while the data thread is parked there, so the wake's handle arrives after it.
    // Deschedule: W_c is parked in the `next_wakeup` return hook until the data thread recorded
    // `wake:now_parked` and the data thread is parked in `now()`, before it takes any lock, so
    // nothing either holds can block the control freeze; the test opens G_n only after freeze #2
    // recorded `Done`, so the list is closed before the wake's handle arrives whatever the order.
    // The parked data thread is joined by cleanup step 3 under RS-8a's default 5 s bound, so G_n
    // must open within it, or step 3 is abandoned and the final cleanup-failure assertion fails.
    let probe = Probe::new();
    let loss = gate();
    let _release = OpenOnDrop(loss.clone());
    let gate_n = gate();
    let _release_n = OpenOnDrop(gate_n.clone());
    let gate_wc = gate();
    let _release_wc = OpenOnDrop(gate_wc.clone());
    let armed = Arc::new(AtomicBool::new(false));
    let threads = Threads::new();
    let parked_at = Arc::new(Mutex::new(None::<usize>));
    let log = ClockLog::default();
    let on_now = {
        let probe = probe.clone();
        let threads = threads.clone();
        let log = log.clone();
        let gate = gate_n.clone();
        let parked_at = parked_at.clone();
        let parked = Arc::new(AtomicBool::new(false));
        move |call: &Call| {
            if *threads.done.lock().unwrap_or_else(|e| e.into_inner()) != Some(call.thread) || parked.swap(true, Ordering::SeqCst) {
                return;
            }
            *parked_at.lock().unwrap_or_else(|e| e.into_inner()) = Some(log.calls().iter().filter(|seen| seen.thread == call.thread).count());
            probe.record("wake:now_parked");
            wait(&gate);
        }
    };
    // The horizon's cancel wakes the control loop while the data thread is still stopping the
    // Provider; W_c waits here until that thread parked in `now`, so the control freeze takes
    // the wake list while the data thread is parked in the wake's `now`, before it publishes.
    let on_next_wakeup_return = {
        let probe = probe.clone();
        let gate = gate_wc.clone();
        let armed = armed.clone();
        let parked = Arc::new(AtomicBool::new(false));
        move |_result: Option<TimePoint>| {
            if !armed.load(Ordering::SeqCst) || parked.swap(true, Ordering::SeqCst) {
                return;
            }
            probe.record("loop:parked");
            wait(&gate);
        }
    };
    let (rig, park) = paced_parked(|authority| authority);
    let mut assembly = rig.assembly;
    assembly.authority = Box::new(WatchedAuthority::new(assembly.authority, log.clone()).with_hooks(Hooks {
        on_now: Some(Arc::new(on_now)),
        on_next_wakeup_return: Some(Arc::new(on_next_wakeup_return)),
        ..Hooks::default()
    }));
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(ThreadedProvider::new("radio", "radio", &probe).publishing(100, Wall::from_millis(1)).losing_device_on(loss.clone())));
    let hook = {
        let probe = probe.clone();
        let threads = threads.clone();
        move |point: Point, p: &start_probe::Probe| {
            entered(point, p, &probe, &threads);
        }
    };
    let (worker, tick) = start_horizon_run(hook, assembly, false);
    assert!(park.wait_parked_until(tick, Wall::from_secs(10)), "{}", infra("the control loop never waited for the horizon"));
    armed.store(true, Ordering::SeqCst);
    open(&loss);
    assert!(probe.wait_for("wake:now_parked", Wall::from_secs(10)), "{}", infra("the data thread's wake did not reach its now within 10 s"));
    open(&gate_wc);
    let second_done = {
        let threads = threads.clone();
        move |lines: &[String]| {
            let Some(data) = *threads.data.lock().unwrap_or_else(|e| e.into_inner()) else {
                return false;
            };
            let first = format!("freeze:Done:{data:?}");
            lines.iter().filter(|line| line.starts_with("freeze:Done:")).any(|line| *line != first)
        }
    };
    assert!(probe.wait_until(second_done, Wall::from_secs(10)), "{}", infra("the control freeze did not record Done within 10 s"));
    open(&gate_n);
    let (result, run) = worker.join();
    assert!(matches!(result, Err(RunHandleError::Ended { .. })), "{result:?}");
    let manifest = manifest_of(run);
    let calls = log.calls();
    let (horizon, _) = scheduled_at(&calls, tick, recorded_thread(&threads.control));
    let data = recorded_thread(&threads.data);
    let (wake, _) = scheduled_by(&calls, data);
    let wake_cancels = cancels_of(&calls, wake);
    assert!(wake_cancels.is_empty(), "KC-46a: a wake accepted after the cleanup's freeze was cancelled, by the freeze or its publisher; calls: {calls:?}");
    assert_eq!(cancels_of(&calls, horizon).len(), 1, "RS-6 step 1: the horizon is cancelled once; calls: {calls:?}");
    let by_data: Vec<&Call> = calls.iter().filter(|call| call.thread == data).collect();
    let parked = parked_at.lock().unwrap_or_else(|e| e.into_inner()).expect("the data thread's now parked");
    assert!(parked > 0 && by_data.len() > parked, "the data thread issued its wake after the parked now: {by_data:?}");
    assert_eq!(by_data[parked - 1].kind, CallKind::Now, "the parked call is the data thread's now: {by_data:?}");
    assert_eq!(by_data[parked].kind, CallKind::Schedule, "KC-46a: the parked now is the wake's own read, before `schedule`: {by_data:?}");
    assert!(manifest.termination.cleanup_failures.is_empty(), "{:?}", manifest.termination.cleanup_failures);
}

#[test]
fn kc_24a_an_agenda_item_reached_after_the_freeze_is_not_admitted_and_requests_nothing() {
    // KC-20: an agenda item whose turn comes after RS-6 step 1 froze dispatch is not admitted,
    // and that is no failure — the Run keeps the orderly Policy stop, records no `also` entry
    // and never dispatches the item's Stop Action. N111-15 deletes the check and N111-24 reads it
    // before the admission lock, ahead of `BeforeAgendaLock`: the item then passes it before the
    // freeze, the freeze completes while W_c is parked, and the admission refuses the item, which
    // escalates a Failed/Abort. Residual: a read of `frozen` between `BeforeAgendaLock` and
    // `lock_admission()` has no window a test can reach, because the hook and the lock are adjacent
    // statements: a mutant that puts the read there is equivalent for every test and has no row. The
    // rule stays written at the check in `dispatch_agenda` (the read is under the lock).
    // Deschedule: the freeze has recorded `Done` before W_c is released, and W_c's park is the
    // positive `BeforeAgendaLock` point; a descheduled W_c only lengthens the test.
    let probe = Probe::new();
    let loss = gate();
    let _release = OpenOnDrop(loss.clone());
    let gate_a = gate();
    let _release_a = OpenOnDrop(gate_a.clone());
    let threads = Threads::new();
    let (rig, _park) = paced_parked(|authority| authority);
    let mut assembly = rig.assembly;
    assembly
        .providers
        .insert(Ident::parse("radio").unwrap(), Box::new(ThreadedProvider::new("radio", "radio", &probe).publishing(100, Wall::from_millis(1)).losing_device_on(loss.clone()).recording_stop_mode()));
    let mut spec = spec_one();
    spec["policies"] = serde_json::json!({ "failure": { "ezsdr.DEVICE_LOST": "stop" } });
    spec["schedule"] = serde_json::json!([{
        "at": { "clock": "radio", "offset_ticks": 50_000 },
        "action": { "kind": "stop", "target": serde_json::to_value(support::target("radio")).unwrap() }
    }]);
    let hook = parked_hook(&probe, &threads, Point::BeforeAgendaLock { index: 0 }, "agenda:parked", &gate_a, &[]);
    let profile = profile_one();
    let worker = on_control_worker(hook, move || {
        let mut run = start_spec_run(&spec, &profile, assembly).expect("the Run starts");
        running(&run);
        let result = run.run_until_end(after(&run, Wall::from_secs(3600)));
        (result, run)
    });
    assert!(probe.wait_for("agenda:parked", Wall::from_secs(10)), "{}", infra("the agenda item never became due within 10 s"));
    open(&loss);
    assert!(probe.wait_until(|lines| lines.iter().any(|line| line.starts_with("freeze:Done:")), Wall::from_secs(10)), "{}", infra("the data thread's freeze did not record Done within 10 s"));
    open(&gate_a);
    let (result, run) = worker.join();
    assert!(matches!(result, Err(RunHandleError::Ended { .. })), "{result:?}");
    let manifest = manifest_of(run);
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Policy { event: kind(EventKind::DEVICE_LOST) } }, "{:?}", manifest.termination);
    assert!(manifest.termination.also.is_empty(), "KC-20: a skipped agenda item is no failure: {:?}", manifest.termination.also);
    let stopping: Vec<_> = manifest.run.transitions.iter().filter(|transition| matches!(transition.state, RunState::Stopping { .. })).collect();
    assert_eq!(stopping.len(), 1, "{:?}", manifest.run.transitions);
    assert_eq!(stopping[0].state, RunState::Stopping { mode: CleanupMode::Orderly }, "{:?}", manifest.run.transitions);
    let lines = probe.lines();
    assert_eq!(lines.iter().filter(|line| *line == "radio:stop_mode:Orderly").count(), 1, "{lines:?}");
    assert!(!lines.iter().any(|line| line == "radio:took:Stop"), "KC-20: the skipped item's Stop Action was never dispatched: {lines:?}");
    assert!(manifest.termination.cleanup_failures.is_empty(), "{:?}", manifest.termination.cleanup_failures);
}

/// W_c's hook for the withdrawal and agenda tests: at `Entered` it stores W_c's id and installs
/// the freeze trace (the start hands out the probe there), it records `trail` for each named
/// point, and it parks once at `halt`. It only records and parks; every assertion runs on the
/// test thread.
fn parked_hook(probe: &Probe, threads: &Threads, halt: Point, line: &'static str, gate: &ParkGate, trail: &'static [(Point, &'static str)]) -> impl FnMut(Point, &start_probe::Probe) + Send + 'static {
    let probe = probe.clone();
    let threads = threads.clone();
    let gate = gate.clone();
    let mut parked = false;
    move |point, p: &start_probe::Probe| {
        if entered(point, p, &probe, &threads) {
            return;
        }
        for (at, name) in trail {
            if point == *at {
                probe.record(*name);
            }
        }
        if point == halt && !parked {
            parked = true;
            probe.record(line);
            wait(&gate);
        }
    }
}

#[test]
fn kc_24a_an_early_withdrawal_does_not_cancel_a_handle_the_freeze_took() {
    // KC-29b, KC-24a: the horizon freeze #1 drained out of `scheduled` is not cancelled again by
    // the control loop's withdrawal — `now()` is before the horizon, the gate is free (the freeze
    // finished) and the handle is gone. N111-16 cancels it a second time. W_c is parked at
    // `BeforeWithdrawalGate`, with the horizon still in `scheduled` and the gate not tried, so
    // the freeze takes the handle and a withdrawal that removed it before the gate (N111-25)
    // leaves the freeze nothing to cancel: the one cancel would then run on W_c after the stop.
    // The order tested: the data thread has stopped the Provider and dropped the gate (its next
    // `schedule` is the unconditional wake after `drop(gate)`), the handle is gone, `now()` is
    // before the horizon; the withdrawal then finds the gate free and removes nothing, so it must
    // not cancel.
    // Deschedule: W_c is parked before it removes the handle or touches any lock; the test
    // releases it only after the data thread's wake reached the Authority, which is after its
    // gate drop, so the legal order "gate free, handle gone" is the one tested. A descheduled
    // data thread only delays that wake.
    let probe = Probe::new();
    let loss = gate();
    let _release = OpenOnDrop(loss.clone());
    let gate_w = gate();
    let _release_w = OpenOnDrop(gate_w.clone());
    let mark_release = release();
    let threads = Threads::new();
    let log = ClockLog::default();
    let on_schedule = {
        let probe = probe.clone();
        let threads = threads.clone();
        let recorded = Arc::new(AtomicBool::new(false));
        move |call: &Call| {
            if !threads.data.lock().unwrap_or_else(|e| e.into_inner()).is_some_and(|data| data == call.thread) {
                return;
            }
            if !probe.lines().iter().any(|line| line == "radio:stop") || recorded.swap(true, Ordering::SeqCst) {
                return;
            }
            probe.record("wake:after_stop");
        }
    };
    let (rig, park) = paced_parked(|authority| authority);
    let mut assembly = rig.assembly;
    assembly.authority = Box::new(WatchedAuthority::new(assembly.authority, log.clone()).with_hooks(Hooks { on_schedule: Some(Arc::new(on_schedule)), ..Hooks::default() }));
    assembly.providers.insert(
        Ident::parse("radio").unwrap(),
        Box::new(ThreadedProvider::new("radio", "radio", &probe).publishing(100, Wall::from_millis(1)).marking_on("test.MARK", mark_release.clone()).losing_device_on(loss.clone())),
    );
    let hook = parked_hook(&probe, &threads, Point::BeforeWithdrawalGate, "withdrawal:parked", &gate_w, &[]);
    let (worker, tick) = start_horizon_run(hook, assembly, true);
    assert!(park.wait_parked_until(tick, Wall::from_secs(10)), "{}", infra("the control loop never waited for the horizon"));
    release_one(&mark_release);
    assert!(probe.wait_for("withdrawal:parked", Wall::from_secs(10)), "{}", infra("the delivered mark did not end the wait within 10 s"));
    open(&loss);
    assert!(probe.wait_for("wake:after_stop", Wall::from_secs(10)), "{}", infra("the data thread's wake after its stop did not reach the Authority within 10 s"));
    open(&gate_w);
    let (result, run) = worker.join();
    // The delivery ended the wait, and the loss then ended the Run before `wait_for` returned.
    assert!(matches!(result, Err(RunHandleError::Ended { .. })), "{result:?}");
    let manifest = manifest_of(run);
    assert!(manifest.events.delivered.iter().any(|event| event.kind == kind("test.MARK")), "the delivery ended the wait before the horizon: {:?}", manifest.events.delivered);
    let calls = log.calls();
    let control = recorded_thread(&threads.control);
    let data = recorded_thread(&threads.data);
    let (horizon, _) = scheduled_at(&calls, tick, control);
    let cancels = cancels_of(&calls, horizon);
    assert_eq!(cancels.len(), 1, "KC-29b: the withdrawal does not cancel a handle the freeze took; calls: {calls:?}");
    assert_eq!(cancels[0].thread, data, "the horizon's one cancel is the data thread's freeze: {calls:?}");
    assert_ne!(cancels[0].thread, control, "the withdrawal cancelled nothing: {calls:?}");
    assert!(manifest.termination.cleanup_failures.is_empty(), "{:?}", manifest.termination.cleanup_failures);
}

#[test]
fn kc_24a_a_horizon_handle_published_after_the_early_drain_is_claimed_by_the_control_freeze() {
    // KC-29b, KC-24a: a horizon handle pushed to `scheduled` after the data thread's freeze
    // drained it is cancelled by the control cleanup's freeze, after freeze #1's `Done`.
    // N111-13 leaves it uncancelled.
    // Deschedule: the test opens G_s only after the data thread's freeze recorded `Done`, so the
    // horizon cannot be in that drain; a descheduled cleanup step thread only delays the cancel.
    let probe = Probe::new();
    let loss = gate();
    let _release = OpenOnDrop(loss.clone());
    let gate_s = gate();
    let _release_s = OpenOnDrop(gate_s.clone());
    let threads = Threads::new();
    let horizon_cell: Arc<Mutex<Option<ScheduleHandle>>> = Arc::new(Mutex::new(None));
    let log = ClockLog::default();
    let on_cancel = {
        let probe = probe.clone();
        let horizon = horizon_cell.clone();
        move |call: &Call| {
            if *horizon.lock().unwrap_or_else(|e| e.into_inner()) == call.handle {
                probe.record(format!("cancel:horizon:{:?}", call.thread));
            }
        }
    };
    let (rig, _park) = paced_parked(|authority| authority);
    let mut assembly = rig.assembly;
    assembly.authority = Box::new(WatchedAuthority::new(assembly.authority, log.clone()).with_hooks(Hooks { on_cancel: Some(Arc::new(on_cancel)), ..Hooks::default() }));
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(ThreadedProvider::new("radio", "radio", &probe).publishing(100, Wall::from_millis(1)).losing_device_on(loss.clone())));
    let hook = parked_hook(&probe, &threads, Point::Scheduled, "horizon:unpublished", &gate_s, &[]);
    let (worker, tick) = start_horizon_run(hook, assembly, false);
    assert!(probe.wait_for("horizon:unpublished", Wall::from_secs(10)), "{}", infra("the horizon's schedule did not return within 10 s"));
    let (horizon, _) = scheduled_at(&log.calls(), tick, recorded_thread(&threads.control));
    *horizon_cell.lock().unwrap_or_else(|e| e.into_inner()) = Some(horizon);
    open(&loss);
    assert!(probe.wait_until(|lines| lines.iter().any(|line| line.starts_with("freeze:Done:")), Wall::from_secs(10)), "{}", infra("the data thread's freeze did not record Done within 10 s"));
    open(&gate_s);
    let (result, run) = worker.join();
    assert!(matches!(result, Err(RunHandleError::Ended { .. })), "{result:?}");
    let manifest = manifest_of(run);
    let calls = log.calls();
    let data = recorded_thread(&threads.data);
    let (horizon, _) = scheduled_at(&calls, tick, recorded_thread(&threads.control));
    let cancels = cancels_of(&calls, horizon);
    assert_eq!(cancels.len(), 1, "KC-29b: the control freeze claims the handle published after the early drain; calls: {calls:?}");
    let lines = probe.lines();
    let done = lines.iter().position(|line| line == &format!("freeze:Done:{data:?}")).expect("freeze #1 recorded Done");
    let cancelled = lines.iter().position(|line| line.starts_with("cancel:horizon:")).expect("the horizon's cancel was recorded");
    assert!(done < cancelled, "KC-29b: the horizon's cancel comes after freeze #1's Done; lines: {lines:?}");
    assert_ne!(cancels[0].thread, data, "the control freeze, not the data thread, cancels the late handle; calls: {calls:?}");
    assert!(manifest.termination.cleanup_failures.is_empty(), "{:?}", manifest.termination.cleanup_failures);
}

#[test]
fn kc_24a_the_data_threads_provider_stop_does_not_overlap_a_late_horizon_cancel() {
    // KC-24a, RS-6 steps 1 and 2 (Sol review F1): the horizon's handle H is published after the
    // data thread's freeze drained `scheduled`; the cleanup's freeze then cancels H. The data
    // thread's Provider stop must not start while that cancel is outstanding. Fixed tree: the
    // data thread holds the gate through its stops, so the cleanup's freeze waits and cancels
    // after the stop. On this tree the gate is free at `BeforeStop`, so the cleanup passes it and
    // parks in H's cancel; the test opens G_c only after `radio:stop`, which forces
    // `cancel:entered` < `radio:stop` < `cancel:returned` and fails the first assertion.
    // Deschedule: W_c is parked at `Point::Scheduled` before it pushes H, so H cannot be in the
    // data freeze's drain whatever runs before G_s opens; afterwards a descheduled W_c only
    // delays the cleanup's freeze, which the test waits for by its `Serialise` line with a 10 s
    // backstop. The data thread is parked at `BeforeStop`; a descheduled data thread only delays
    // `radio:stop`, and the test opens G_c only after that line, so the unfixed tree's
    // `entered < stop < returned` is forced by the gates, not raced. On the fixed tree the data
    // thread holds the gate while parked, so the cleanup cannot cancel before the stop; it parks
    // at the gate instead, and its cancel then finds G_c already open (`stop < entered`).
    let probe = Probe::new();
    let loss = gate();
    let _release = OpenOnDrop(loss.clone());
    let gate_s = gate();
    let _release_s = OpenOnDrop(gate_s.clone());
    let gate_d = gate();
    let _release_d = OpenOnDrop(gate_d.clone());
    let gate_c = gate();
    let _release_c = OpenOnDrop(gate_c.clone());
    let threads = Threads::new();
    let horizon_cell: Arc<Mutex<Option<ScheduleHandle>>> = Arc::new(Mutex::new(None));
    let log = ClockLog::default();
    let on_cancel = {
        let probe = probe.clone();
        let gate = gate_c.clone();
        let horizon = horizon_cell.clone();
        let parked = Arc::new(AtomicBool::new(false));
        move |call: &Call| {
            if *horizon.lock().unwrap_or_else(|e| e.into_inner()) != call.handle || parked.swap(true, Ordering::SeqCst) {
                return;
            }
            probe.record("cancel:entered");
            wait(&gate);
        }
    };
    let on_cancel_return = {
        let probe = probe.clone();
        let horizon = horizon_cell.clone();
        move |call: &Call| {
            if *horizon.lock().unwrap_or_else(|e| e.into_inner()) == call.handle {
                probe.record("cancel:returned");
            }
        }
    };
    let (rig, _park) = paced_parked(|authority| authority);
    let mut assembly = rig.assembly;
    assembly.authority = Box::new(WatchedAuthority::new(assembly.authority, log.clone()).with_hooks(Hooks {
        on_cancel: Some(Arc::new(on_cancel)),
        on_cancel_return: Some(Arc::new(on_cancel_return)),
        ..Hooks::default()
    }));
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(ThreadedProvider::new("radio", "radio", &probe).publishing(100, Wall::from_millis(1)).losing_device_on(loss.clone())));
    let readings: Arc<Mutex<Vec<(FreezePoint, FreezeReading)>>> = Arc::new(Mutex::new(Vec::new()));
    let hook = {
        let probe = probe.clone();
        let threads = threads.clone();
        let gate_d = gate_d.clone();
        let readings = readings.clone();
        let mut inner = parked_hook(&probe, &threads, Point::Scheduled, "horizon:unpublished", &gate_s, &[]);
        move |point: Point, p: &start_probe::Probe| {
            inner(point, p);
            if point != Point::Entered {
                return;
            }
            // `on_freeze` overwrites the trace `entered` installed; this one keeps every reading
            // and parks the data thread at `BeforeStop`. It only records and parks.
            let trace = freeze_trace(&probe, &threads);
            let readings = readings.clone();
            let gate = gate_d.clone();
            let parked = Arc::new(AtomicBool::new(false));
            p.on_freeze(move |point, reading| {
                readings.lock().unwrap_or_else(|e| e.into_inner()).push((point, reading));
                trace(point, reading);
                if point == FreezePoint::BeforeStop && !parked.swap(true, Ordering::SeqCst) {
                    wait(&gate);
                }
            });
        }
    };
    let (worker, tick) = start_horizon_run(hook, assembly, false);
    assert!(probe.wait_for("horizon:unpublished", Wall::from_secs(10)), "{}", infra("the horizon's schedule did not return within 10 s"));
    let (horizon, _) = scheduled_at(&log.calls(), tick, recorded_thread(&threads.control));
    *horizon_cell.lock().unwrap_or_else(|e| e.into_inner()) = Some(horizon);
    open(&loss);
    assert!(probe.wait_until(|lines| lines.iter().any(|line| line.starts_with("freeze:BeforeStop:")), Wall::from_secs(10)), "{}", infra("the data thread did not reach BeforeStop within 10 s"));
    let (_, stop_reading) = *readings.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(point, _)| *point == FreezePoint::BeforeStop).expect("the data thread recorded its BeforeStop reading");
    let held_freeze = stop_reading.held.freeze;
    open(&gate_s);
    let data = recorded_thread(&threads.data);
    let second_freeze = {
        let first = format!("freeze:Serialise:{data:?}");
        move |lines: &[String]| lines.iter().filter(|line| line.starts_with("freeze:Serialise:")).any(|line| *line != first)
    };
    assert!(probe.wait_until(second_freeze, Wall::from_secs(10)), "{}", infra("the cleanup's freeze did not reach the gate within 10 s"));
    if !held_freeze {
        // Unfixed tree: the gate is free at `BeforeStop`, so the cleanup's freeze passes it and
        // parks in H's cancel. A fixed tree waits at the gate, so there is nothing to wait for.
        assert!(probe.wait_for("cancel:entered", Wall::from_secs(10)), "{}", infra("the cleanup's freeze did not enter the late horizon's cancel within 10 s"));
    }
    open(&gate_d);
    assert!(probe.wait_for("radio:stop", Wall::from_secs(10)), "{}", infra("the data thread's Provider stop did not start within 10 s"));
    open(&gate_c);
    let (result, run) = worker.join();
    assert!(matches!(result, Err(RunHandleError::Ended { .. })), "{result:?}");
    let manifest = manifest_of(run);
    let lines = probe.lines();
    let stop = lines.iter().position(|line| line == "radio:stop").expect("the Provider's stop was recorded");
    assert_eq!(lines.iter().filter(|line| *line == "radio:stop").count(), 1, "the Provider is stopped exactly once; lines: {lines:?}");
    if let (Some(entered), Some(returned)) = (lines.iter().position(|line| line == "cancel:entered"), lines.iter().position(|line| line == "cancel:returned")) {
        assert!(!(entered < stop && stop < returned), "RS-6: the Provider stop started while the cancel of the late horizon was outstanding; lines: {lines:?}");
    }
    assert!(stop_reading.held.freeze, "KC-24a: the data thread's Provider stops run outside the freeze gate, so another performance's cancel can overlap them: {stop_reading:?}");
    let calls = log.calls();
    let cancels = cancels_of(&calls, horizon);
    assert_eq!(cancels.len(), 1, "RS-6 step 1: the late horizon is cancelled exactly once; calls: {calls:?}");
    assert_ne!(cancels[0].thread, data, "KC-24a: the control freeze, not the data thread, cancels the late handle; calls: {calls:?}");
    assert!(cancels[0].held.freeze, "KC-24a: the late horizon's cancel runs under the freeze gate; calls: {calls:?}");
    assert!(manifest.termination.cleanup_failures.is_empty(), "{:?}", manifest.termination.cleanup_failures);
}

#[test]
fn kc_24a_an_early_stop_after_the_cleanups_freeze_performs_no_second_freeze() {
    // KC-24a, RS-6 steps 1 and 2 (KC-46b): the cleanup's freeze runs to `Done` while the data
    // thread is parked before it takes the gate, so the data thread finds dispatch frozen,
    // freezes nothing again and only stops the Provider. N111-27 makes it freeze again.
    // Deschedule: the data thread is parked before it takes the gate, so the cleanup's freeze (a
    // different thread) cannot be reordered after it; the test opens G_p only after that freeze's
    // `Done`, and the cleanup's step 3 joins the data thread only after that. A descheduled data
    // thread or finisher only delays.
    let probe = Probe::new();
    let loss = gate();
    let _release = OpenOnDrop(loss.clone());
    let gate_p = gate();
    let _release_p = OpenOnDrop(gate_p.clone());
    let threads = Threads::new();
    let readings: Arc<Mutex<Vec<(FreezePoint, FreezeReading)>>> = Arc::new(Mutex::new(Vec::new()));
    let (rig, _park) = paced_parked(|authority| authority);
    let mut assembly = rig.assembly;
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(ThreadedProvider::new("radio", "radio", &probe).publishing(100, Wall::from_millis(1)).losing_device_on(loss.clone())));
    let hook = {
        let probe = probe.clone();
        let threads = threads.clone();
        let gate = gate_p.clone();
        let readings = readings.clone();
        move |point: Point, p: &start_probe::Probe| {
            if point != Point::Entered {
                return;
            }
            // `entered` cannot install this hook: it must park the data thread at `Serialise`,
            // before that thread takes the gate, not at every point alike.
            let trace = freeze_trace(&probe, &threads);
            let readings = readings.clone();
            let gate = gate.clone();
            let parked = Arc::new(AtomicBool::new(false));
            p.on_freeze(move |point, reading| {
                readings.lock().unwrap_or_else(|e| e.into_inner()).push((point, reading));
                trace(point, reading);
                if point == FreezePoint::Serialise && std::thread::current().name() == Some("ezsdr-data") && !parked.swap(true, Ordering::SeqCst) {
                    wait(&gate);
                }
            });
        }
    };
    let (spec, profile) = (spec_one(), profile_one());
    let started = start_on_worker(&probe, hook, move || start_spec_run(&spec, &profile, assembly));
    let run = started.finish();
    open(&loss);
    assert!(probe.wait_until(|lines| lines.iter().any(|line| line.starts_with("freeze:Serialise:")), Wall::from_secs(10)), "{}", infra("the data thread did not reach the freeze gate within 10 s"));
    let data = recorded_thread(&threads.data);
    let (manifest_tx, manifest_rx) = mpsc::channel();
    let finisher = std::thread::spawn(move || {
        let _ = manifest_tx.send(run.finish());
    });
    let cleanup_done = {
        let first = format!("freeze:Done:{data:?}");
        move |lines: &[String]| lines.iter().filter(|line| line.starts_with("freeze:Done:")).any(|line| *line != first)
    };
    assert!(probe.wait_until(cleanup_done, Wall::from_secs(10)), "{}", infra("the cleanup's freeze did not record Done within 10 s"));
    open(&gate_p);
    let manifest = match manifest_rx.recv_timeout(Wall::from_secs(10)) {
        Ok(manifest) => manifest,
        Err(_) => panic!("{}", infra("the cleanup did not return a Manifest within 10 s after the gate opened")),
    };
    finisher.join().expect("W_f does not panic");
    let lines = probe.lines();
    let serialised: Vec<&str> = lines.iter().filter_map(|line| line.strip_prefix("freeze:Serialised:")).collect();
    assert_eq!(serialised.len(), 1, "KC-24a: the data thread performed a second freeze; lines: {lines:?}");
    assert_ne!(serialised[0], format!("{data:?}"), "KC-24a: the data thread performed a second freeze; lines: {lines:?}");
    let mine: Vec<String> = lines.iter().filter(|line| line.starts_with("freeze:") && line.ends_with(&format!(":{data:?}"))).cloned().collect();
    assert_eq!(mine, vec![format!("freeze:Serialise:{data:?}"), format!("freeze:BeforeStop:{data:?}")], "KC-24a: the data thread's early stop performed a second freeze; lines: {lines:?}");
    let (_, reading) = *readings
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|(point, reading)| *point == FreezePoint::BeforeStop && reading.thread == data)
        .expect("the data thread recorded its BeforeStop reading");
    assert!(reading.held.freeze, "KC-24a: step 2 did not run under the freeze gate: {reading:?}");
    assert!(reading.dispatch_frozen, "KC-24a: dispatch is not frozen when the data thread stops the Provider: {reading:?}");
    assert_eq!(lines.iter().filter(|line| *line == "radio:stop").count(), 1, "KC-39: the Provider is stopped once; lines: {lines:?}");
    assert!(manifest.termination.cleanup_failures.is_empty(), "{:?}", manifest.termination.cleanup_failures);
}

#[test]
fn kc_24a_a_withdrawal_cancel_precedes_the_provider_stop() {
    // KC-24a, RS-6 step 1: the withdrawal removes and cancels the horizon under the freeze gate,
    // so the data thread's freeze cannot reach the Provider stop before that cancel returned, and
    // the freeze records `Serialised` only after it. N111-21 cancels outside the gate.
    // Deschedule: W_c is parked while holding the gate, so the data thread cannot pass
    // `Serialise`'s gate; the test opens G_w only after `freeze:Serialise`, so the data thread is
    // already waiting when the gate is handed on.
    let probe = Probe::new();
    let loss = gate();
    let _release = OpenOnDrop(loss.clone());
    let gate_w = gate();
    let _release_w = OpenOnDrop(gate_w.clone());
    let mark_release = release();
    let threads = Threads::new();
    let horizon_cell: Arc<Mutex<Option<ScheduleHandle>>> = Arc::new(Mutex::new(None));
    let log = ClockLog::default();
    // Recorded after the inner `cancel` returned, so the asserted orders observe the return, not the entry.
    let on_cancel_return = {
        let probe = probe.clone();
        let horizon = horizon_cell.clone();
        move |call: &Call| {
            if *horizon.lock().unwrap_or_else(|e| e.into_inner()) == call.handle {
                probe.record(format!("cancel:returned:{:?}", call.thread));
            }
        }
    };
    let (rig, park) = paced_parked(|authority| authority);
    let mut assembly = rig.assembly;
    assembly.authority = Box::new(WatchedAuthority::new(assembly.authority, log.clone()).with_hooks(Hooks { on_cancel_return: Some(Arc::new(on_cancel_return)), ..Hooks::default() }));
    assembly.providers.insert(
        Ident::parse("radio").unwrap(),
        Box::new(ThreadedProvider::new("radio", "radio", &probe).publishing(100, Wall::from_millis(1)).marking_on("test.MARK", mark_release.clone()).losing_device_on(loss.clone())),
    );
    let hook = parked_hook(&probe, &threads, Point::Withdrawn, "withdrawn:parked", &gate_w, &[]);
    let (worker, tick) = start_horizon_run(hook, assembly, true);
    assert!(park.wait_parked_until(tick, Wall::from_secs(10)), "{}", infra("the control loop never waited for the horizon"));
    release_one(&mark_release);
    assert!(probe.wait_for("withdrawn:parked", Wall::from_secs(10)), "{}", infra("the delivered mark did not end the wait within 10 s"));
    let (horizon, _) = scheduled_at(&log.calls(), tick, recorded_thread(&threads.control));
    *horizon_cell.lock().unwrap_or_else(|e| e.into_inner()) = Some(horizon);
    open(&loss);
    assert!(probe.wait_until(|lines| lines.iter().any(|line| line.starts_with("freeze:Serialise:")), Wall::from_secs(10)), "{}", infra("the data thread's freeze did not reach the gate within 10 s"));
    open(&gate_w);
    let (result, run) = worker.join();
    // The delivery ended the wait, and the loss then ended the Run before `wait_for` returned.
    assert!(matches!(result, Err(RunHandleError::Ended { .. })), "{result:?}");
    let manifest = manifest_of(run);
    assert!(manifest.events.delivered.iter().any(|event| event.kind == kind("test.MARK")), "the delivery ended the wait before the horizon: {:?}", manifest.events.delivered);
    let calls = log.calls();
    let (horizon, _) = scheduled_at(&calls, tick, recorded_thread(&threads.control));
    let cancels = cancels_of(&calls, horizon);
    assert_eq!(cancels.len(), 1, "RS-6 step 1: the horizon is cancelled once, by the withdrawal; calls: {calls:?}");
    assert!(cancels[0].held.freeze, "KC-24a: the withdrawal's cancel runs under the freeze gate; calls: {calls:?}");
    let lines = probe.lines();
    let returned = lines.iter().position(|line| line.starts_with("cancel:returned:")).expect("the horizon's cancel was recorded");
    let serialised: Vec<usize> = lines.iter().enumerate().filter(|(_, line)| line.starts_with("freeze:Serialised:")).map(|(at, _)| at).collect();
    assert!(!serialised.is_empty(), "{lines:?}");
    assert!(serialised.iter().all(|at| returned < *at), "KC-24a: the withdrawal's cancel precedes every pass of the freeze gate; lines: {lines:?}");
    let stop = lines.iter().position(|line| line == "radio:stop").expect("the Provider was stopped");
    assert!(returned < stop, "KC-24a: the withdrawal's cancel precedes the Provider stop; lines: {lines:?}");
    assert!(manifest.termination.cleanup_failures.is_empty(), "{:?}", manifest.termination.cleanup_failures);
}

type Signal = Arc<dyn Fn() + Send + Sync>;

#[test]
fn kc_24a_a_cancel_that_calls_disconnect_signal_schedules_nothing_under_the_gate() {
    // KC-24a, KC-46a (#120 review): the horizon's withdrawal cancels under the freeze gate, and a supplied
    // `cancel` that synchronously delivers `disconnect_signal` makes the signal's wake run on that thread:
    // `wake` schedules nothing there (its `now()` and `schedule()` would run under the gate, and an Authority
    // that serialises `schedule` with `cancel` would wait for itself), and the end request the signal set is
    // still served: `wait_for` returns the client disconnect, the Run is cleaned up orderly.
    // Deschedule: the signal is called inside the withdrawal's cancel on W_c itself, so no other thread's
    // schedule orders anything; the mark is released only once W_c waits for the horizon.
    let probe = Probe::new();
    let mark_release = release();
    let log = ClockLog::default();
    let signal_cell: Arc<Mutex<Option<Signal>>> = Arc::new(Mutex::new(None));
    let on_cancel = {
        let signal_cell = signal_cell.clone();
        let called = Arc::new(AtomicBool::new(false));
        move |_: &Call| {
            if !called.swap(true, Ordering::SeqCst) {
                let signal = signal_cell.lock().unwrap_or_else(|e| e.into_inner()).clone().expect("W_c stored the signal before it waited");
                signal();
            }
        }
    };
    let (rig, park) = paced_parked(|authority| authority);
    let mut assembly = rig.assembly;
    assembly.authority = Box::new(WatchedAuthority::new(assembly.authority, log.clone()).with_hooks(Hooks { on_cancel: Some(Arc::new(on_cancel)), ..Hooks::default() }));
    assembly
        .providers
        .insert(Ident::parse("radio").unwrap(), Box::new(ThreadedProvider::new("radio", "radio", &probe).publishing(100, Wall::from_millis(1)).marking_on("test.MARK", mark_release.clone())));
    let (tick_tx, tick_rx) = mpsc::channel();
    let (spec, profile) = (spec_one(), profile_one());
    let cell = signal_cell.clone();
    let worker = on_control_worker(
        |_, _| {},
        move || {
            let mut run = start_spec_run(&spec, &profile, assembly).expect("the Run starts");
            running(&run);
            *cell.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(run.disconnect_signal()));
            let horizon = after(&run, Wall::from_secs(3600));
            let _ = tick_tx.send(horizon.ticks_in(horizon.domain()).expect("the horizon is on the primary root"));
            let result = run.wait_for(&[kind("test.MARK")], 0, horizon).map(|_| ());
            (result, run)
        },
    );
    let tick = tick_rx.recv_timeout(Wall::from_secs(10)).unwrap_or_else(|_| panic!("{}", infra("the control worker did not start the Run within 10 s")));
    assert!(park.wait_parked_until(tick, Wall::from_secs(10)), "{}", infra("the control loop never waited for the horizon"));
    release_one(&mark_release);
    let (result, run): EndedRun = worker.join();
    assert_eq!(result, Err(RunHandleError::Ended { termination: client_disconnect() }), "the end request the signal set was not served");
    let manifest = manifest_of(run);
    assert_eq!(manifest.termination.reason, client_disconnect());
    let calls = log.calls();
    let cancels: Vec<&Call> = calls.iter().filter(|call| call.kind == CallKind::Cancel).collect();
    assert!(!cancels.is_empty() && cancels[0].held.freeze, "the fixture never reached the withdrawal's cancel under the gate; calls: {calls:?}");
    for call in &calls {
        let exempt_freeze = call.kind == CallKind::Cancel;
        assert!(
            call.held == ezsdr_kernel::coordinator::lock_record::Held { freeze: call.held.freeze && exempt_freeze, ..Default::default() },
            "KC-24a: {:?} on {:?} ran under a coordinator lock ({:?}) -- the signal's wake ran under the gate; calls: {calls:?}",
            call.kind,
            call.thread,
            call.held
        );
    }
    assert!(manifest.termination.cleanup_failures.is_empty(), "{:?}", manifest.termination.cleanup_failures);
}

#[test]
fn kc_24a_an_early_withdrawal_skips_when_a_freeze_holds_the_gate() {
    // KC-24a: a withdrawal that meets a freeze holding the gate skips its remove-and-cancel — the
    // freeze drained and cancels the handle itself — and goes on to `settle`, whose control freeze
    // waits on the gate. N111-22 waits on the gate instead of skipping: its blocking `lock_freeze`
    // records `freeze:Serialise` on W_c before it blocks. The verdict is that line: the fixed
    // tree's cleanup freeze runs on its own step thread, after W_c skipped, so no `Serialise` line
    // names W_c, and the mutant's does deterministically.
    // Deschedule: W_c is released only while freeze #1 is inside the cancel (the test waits for
    // `cancel:entered`), so the gate is held when it tries; a W_c descheduled after the release
    // only delays the cleanup's freeze, whose `Serialise` the test waits for with a 10 s infra
    // backstop, never the verdict. G_c opens either way, so a slow host passes and only a
    // withdrawal that took the gate (recording its own `Serialise` line) fails.
    let probe = Probe::new();
    let loss = gate();
    let _release = OpenOnDrop(loss.clone());
    let gate_w = gate();
    let _release_w = OpenOnDrop(gate_w.clone());
    let gate_c = gate();
    let _release_c = OpenOnDrop(gate_c.clone());
    let mark_release = release();
    let threads = Threads::new();
    let horizon_cell: Arc<Mutex<Option<ScheduleHandle>>> = Arc::new(Mutex::new(None));
    let log = ClockLog::default();
    let on_cancel = {
        let probe = probe.clone();
        let gate = gate_c.clone();
        let horizon = horizon_cell.clone();
        let parked = Arc::new(AtomicBool::new(false));
        move |call: &Call| {
            if *horizon.lock().unwrap_or_else(|e| e.into_inner()) != call.handle || parked.swap(true, Ordering::SeqCst) {
                return;
            }
            probe.record(format!("cancel:entered:{:?}", call.thread));
            wait(&gate);
        }
    };
    let (rig, park) = paced_parked(|authority| authority);
    let mut assembly = rig.assembly;
    assembly.authority = Box::new(WatchedAuthority::new(assembly.authority, log.clone()).with_hooks(Hooks { on_cancel: Some(Arc::new(on_cancel)), ..Hooks::default() }));
    assembly.providers.insert(
        Ident::parse("radio").unwrap(),
        Box::new(ThreadedProvider::new("radio", "radio", &probe).publishing(100, Wall::from_millis(1)).marking_on("test.MARK", mark_release.clone()).losing_device_on(loss.clone())),
    );
    let hook = parked_hook(&probe, &threads, Point::Withdrawal, "withdrawal:parked", &gate_w, &[(Point::Withdrawn, "withdrawn:parked")]);
    let (worker, tick) = start_horizon_run(hook, assembly, true);
    assert!(park.wait_parked_until(tick, Wall::from_secs(10)), "{}", infra("the control loop never waited for the horizon"));
    release_one(&mark_release);
    assert!(probe.wait_for("withdrawal:parked", Wall::from_secs(10)), "{}", infra("the delivered mark did not end the wait within 10 s"));
    let (horizon, _) = scheduled_at(&log.calls(), tick, recorded_thread(&threads.control));
    *horizon_cell.lock().unwrap_or_else(|e| e.into_inner()) = Some(horizon);
    open(&loss);
    assert!(
        probe.wait_until(|lines| lines.iter().any(|line| line.starts_with("cancel:entered:")), Wall::from_secs(10)),
        "{}",
        infra("the data thread's freeze did not reach the horizon's cancel within 10 s")
    );
    open(&gate_w);
    let data = recorded_thread(&threads.data);
    let control = recorded_thread(&threads.control);
    let another_freeze = {
        let first = format!("freeze:Serialise:{data:?}");
        move |lines: &[String]| lines.iter().filter(|line| line.starts_with("freeze:Serialise:")).any(|line| *line != first)
    };
    assert!(probe.wait_until(another_freeze, Wall::from_secs(10)), "{}", infra("the cleanup's freeze did not reach the gate within 10 s"));
    let freeze_lines = probe.lines();
    assert!(
        !freeze_lines.iter().any(|line| line == &format!("freeze:Serialise:{control:?}")),
        "KC-24a: the withdrawal waited for the freeze gate (a blocking take) instead of skipping when a freeze held it; lines: {freeze_lines:?}"
    );
    open(&gate_c);
    let (result, run) = worker.join();
    // The delivery ended the wait, and the loss then ended the Run before `wait_for` returned.
    assert!(matches!(result, Err(RunHandleError::Ended { .. })), "{result:?}");
    let manifest = manifest_of(run);
    assert!(manifest.events.delivered.iter().any(|event| event.kind == kind("test.MARK")), "the delivery ended the wait before the horizon: {:?}", manifest.events.delivered);
    let calls = log.calls();
    let (horizon, _) = scheduled_at(&calls, tick, control);
    let cancels = cancels_of(&calls, horizon);
    assert_eq!(cancels.len(), 1, "RS-6 step 1: the freeze cancels the horizon once and the skipped withdrawal none; calls: {calls:?}");
    assert_eq!(cancels[0].thread, data, "the horizon's one cancel is the data thread's freeze: {calls:?}");
    assert!(!calls.iter().any(|call| call.kind == CallKind::Cancel && call.thread == control), "KC-24a: the withdrawal cancelled nothing when the freeze held the gate; calls: {calls:?}");
    let lines = probe.lines();
    assert!(!lines.iter().any(|line| line == "withdrawn:parked"), "KC-24a: the withdrawal did not reach Withdrawn when the freeze held the gate; lines: {lines:?}");
    assert!(manifest.termination.cleanup_failures.is_empty(), "{:?}", manifest.termination.cleanup_failures);
}

#[test]
fn kc_24a_sweep_a_device_loss_run_makes_no_supplied_call_under_a_lock() {
    // KC-24a (#111, #119): in a paced Run ended by DEVICE_LOST, the data thread's freeze
    // cancels H while Running and schedules its unconditional wake; the control freeze
    // cancels that wake handle while Stopping; over the whole log no supplied call runs
    // under the admission or the machine lock.
    // Deschedule: `finish` runs only after the Run's reading shows the data thread's wake
    // published, so the cleanup's freeze always takes the handle from the list (a handle
    // still in flight would be held by no one instead, a different window); a
    // data thread descheduled before its freeze's Cancel or inside `schedule` only extends
    // the test's 10 s wait for the reading.
    let probe = Probe::new();
    let loss = gate();
    let _release = OpenOnDrop(loss.clone());
    let mut spec = spec_one();
    spec["schedule"] = serde_json::json!([{
        "at": { "clock": "radio", "offset_ticks": 3_600_000_000i64 },
        "action": { "kind": "stop", "target": null }
    }]);
    let rig = paced();
    let mut assembly = rig.assembly;
    let log = ClockLog::default();
    assembly.authority = Box::new(WatchedAuthority::new(assembly.authority, log.clone()));
    assembly.host_clock = Arc::new(WatchedClock::new(Arc::new(FakeHostClock::new()), log.clone()));
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(ThreadedProvider::new("radio", "radio", &probe).publishing(100, Wall::from_millis(1)).losing_device_on(loss.clone())));
    let threads = Threads::new();
    let hook_run = threads.run.clone();
    let freezes = Arc::new(Mutex::new(Vec::<(FreezePoint, FreezeReading)>::new()));
    let hook_freezes = freezes.clone();
    let hook_lines = probe.clone();
    let started = start_on_worker(
        &probe,
        move |point, p| {
            if point == Point::Entered {
                *hook_run.lock().unwrap_or_else(|e| e.into_inner()) = Some(p.clone());
                let hook_freezes = hook_freezes.clone();
                let hook_lines = hook_lines.clone();
                p.on_freeze(move |point, reading| {
                    hook_lines.record(format!("freeze:{point:?}:{:?}", reading.thread));
                    hook_freezes.lock().unwrap_or_else(|e| e.into_inner()).push((point, reading));
                });
            }
        },
        move || start_spec_run(&spec, &profile_one(), assembly),
    );
    let run = started.finish();
    let start = run.start_instant().expect("the Run was armed").ticks_in(rig.root).expect("the start is on the primary root");
    let (handle, _instant) = far_future_stop(&log.calls(), start);
    open(&loss);
    assert!(probe.wait_until(|lines| lines.iter().any(|line| line.starts_with("freeze:Serialise:")), Wall::from_secs(10)), "{}", infra("the data thread's freeze did not start within 10 s"));
    let data = {
        let freezes = freezes.lock().unwrap_or_else(|e| e.into_inner());
        freezes.iter().find(|(point, _)| *point == FreezePoint::Serialise).expect("the data thread's freeze was recorded").1.thread
    };
    threads.wait_reading("the data thread's end-wake published", |reading| reading.wake_handles >= 1);
    let wake = log.calls().into_iter().find(|call| call.kind == CallKind::Schedule && call.thread == data && call.handle.is_some()).expect("the published wake's Schedule is in the log");
    let wake_handle = wake.handle.expect("checked above");
    let manifest = run.finish_checked();
    let calls = log.calls();
    let phases = sweep(&calls, &manifest.run.transitions, rig.root);
    let h_cancels: Vec<(usize, &Call)> = calls.iter().enumerate().filter(|(_, call)| call.kind == CallKind::Cancel && call.handle == Some(handle)).collect();
    assert_eq!(h_cancels.len(), 1, "RS-6 step 1: the data thread cancels H exactly once; calls: {calls:?}");
    assert_eq!(h_cancels[0].1.thread, data, "KC-24a: H's Cancel is not the data thread's; calls: {calls:?}");
    assert!(phases.running.contains(&h_cancels[0].0), "KC-24a: H's Cancel is not in the Running window; phases: {phases:?}; calls: {calls:?}");
    let (wake_at, _) = calls.iter().enumerate().find(|(_, call)| call.kind == CallKind::Schedule && call.handle == Some(wake_handle)).expect("the data thread's wake was scheduled");
    assert!(phases.running.contains(&wake_at), "KC-24a: the data thread's wake Schedule is not in the Running window; phases: {phases:?}; calls: {calls:?}");
    let wake_cancels: Vec<(usize, &Call)> = calls.iter().enumerate().filter(|(_, call)| call.kind == CallKind::Cancel && call.handle == Some(wake_handle)).collect();
    assert_eq!(wake_cancels.len(), 1, "RS-6 step 1: the wake handle is cancelled exactly once; calls: {calls:?}");
    assert!(phases.stopping.contains(&wake_cancels[0].0), "KC-24a: the wake handle's Cancel is not in the Stopping window; phases: {phases:?}; calls: {calls:?}");
    assert!(calls.iter().any(|call| call.thread == data && call.kind == CallKind::Now), "KC-24a: the data thread made no now() read; calls: {calls:?}");
    assert!(manifest.termination.cleanup_failures.is_empty(), "the cleanup recorded failures: {:?}", manifest.termination.cleanup_failures);
}

#[test]
fn kc_24a_sweep_a_paced_orderly_finish_makes_no_supplied_call_under_a_lock() {
    // KC-24a (#111, #119): in a paced Run finished at once, the cleanup's freeze cancels
    // H once while Stopping, and the sweep finds no supplied call under the admission or
    // the machine lock over the start, the data thread's passes and the cleanup; a Sink and
    // its link make RS-6 step 6's `drops()` (#121) one of the calls the sweep covers.
    // Deschedule: `finish` requests the end on this thread and its freeze takes H from
    // `scheduled` under the freeze gate; the data thread may be descheduled in a pass,
    // but none of its calls carries an assertion and step 3 joins it.
    let (mut spec, profile) = output_docs(64);
    spec["schedule"] = serde_json::json!([{
        "at": { "clock": "radio", "offset_ticks": 3_600_000_000i64 },
        "action": { "kind": "stop", "target": null }
    }]);
    let probe = Probe::new();
    let rig = paced();
    let mut assembly = sink(rig.assembly, RecordingSink::new("rec", &probe), &probe);
    let log = ClockLog::default();
    assembly.links.insert(mref("ezsdr.test.link"), Box::new(TestLinkModule::new(&probe).logging_drops(&log)));
    assembly.authority = Box::new(WatchedAuthority::new(assembly.authority, log.clone()));
    assembly.host_clock = Arc::new(WatchedClock::new(Arc::new(FakeHostClock::new()), log.clone()));
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(ThreadedProvider::new("radio", "radio", &probe).publishing(100, Wall::from_millis(1))));
    let run = start_spec_run(&spec, &profile, assembly).expect("the fixture Run starts");
    let start = run.start_instant().expect("the Run was armed").ticks_in(rig.root).expect("the start is on the primary root");
    let (handle, _instant) = far_future_stop(&log.calls(), start);
    let manifest = run.finish_checked();
    let calls = log.calls();
    let phases = sweep(&calls, &manifest.run.transitions, rig.root);
    let cancels: Vec<(usize, &Call)> = calls.iter().enumerate().filter(|(_, call)| call.kind == CallKind::Cancel && call.handle == Some(handle)).collect();
    assert_eq!(cancels.len(), 1, "RS-6 step 1: H is cancelled exactly once; calls: {calls:?}");
    assert!(phases.stopping.contains(&cancels[0].0), "KC-24a: the Cancel of H is not in the Stopping window; phases: {phases:?}; calls: {calls:?}");
    let drops: Vec<(usize, &Call)> = calls.iter().enumerate().filter(|(_, call)| call.kind == CallKind::Drops).collect();
    assert_eq!(drops.len(), 1, "RS-6 step 6: one link, one drops() call; calls: {calls:?}");
    assert!(phases.stopping.contains(&drops[0].0), "RS-6 step 6: the drops() call is not in the Stopping window; phases: {phases:?}; calls: {calls:?}");
    assert_eq!(drops[0].1.held, Held::default(), "KC-24a: drops() ran under a lock; calls: {calls:?}");
    assert!(manifest.termination.cleanup_failures.is_empty(), "the cleanup recorded failures: {:?}", manifest.termination.cleanup_failures);
}

#[test]
fn kc_24a_a_device_lost_stop_stamps_its_event_under_the_gate_only() {
    // KC-24a's named exception: when the data thread's early stop fails with DeviceLost,
    // the coordinator reads `now()` to stamp the DEVICE_LOST event with the freeze gate
    // held (the Provider `stop` runs under it). That one `Now` holds the gate and nothing
    // else -- no `end`, `done`, `scheduled` or other lock with it -- and no other `Now` of
    // the Run is made with the gate held. The Provider double never calls the time handle
    // it was given, so a `Now` under the gate is the coordinator's.
    // Deschedule: the data thread's freeze and stop run in one hold of the gate and are
    // joined by `finish`'s step 3, so the log is complete when `finish` returns; no wait
    // here reads a wall clock.
    let probe = Probe::new();
    let loss = gate();
    let _release = OpenOnDrop(loss.clone());
    let rig = paced();
    let mut assembly = rig.assembly;
    let log = ClockLog::default();
    assembly.authority = Box::new(WatchedAuthority::new(assembly.authority, log.clone()));
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(ThreadedProvider::new("radio", "radio", &probe).losing_device_on(loss.clone()).losing_device_in_stop()));
    let run = start_spec_run(&spec_one(), &profile_one(), assembly).expect("the fixture Run starts");
    open(&loss);
    assert!(probe.wait_for("radio:stop", Wall::from_secs(10)), "{}", infra("the data thread's early stop did not reach the Provider within 10 s"));
    let manifest = run.finish_checked();
    let calls = log.calls();
    let under_gate: Vec<&Call> = calls.iter().filter(|call| call.kind == CallKind::Now && call.held.freeze).collect();
    assert_eq!(under_gate.len(), 1, "KC-24a: exactly one now() is made under the freeze gate, the DEVICE_LOST stamp; calls: {calls:?}");
    assert_eq!(under_gate[0].held, Held { freeze: true, ..Held::default() }, "KC-24a: the DEVICE_LOST stamp holds a lock beside the gate; calls: {calls:?}");
    let stop_failure =
        manifest.termination.cleanup_failures.iter().filter(|failure| failure.step == ezsdr_kernel::run::CleanupStep::StopProviders && failure.reason.contains("device lost in stop")).count();
    assert_eq!(stop_failure, 1, "the Provider's stop did not fail with DeviceLost: {:?}", manifest.termination.cleanup_failures);
}

// ---------------------------------------------------------------- KC-46a: the wake list

/// A paced Session (Attached Lease) on the test thread whose Provider emits `test.MARK` per `marks` unit and loses its device when `loss` opens, with every supplied call logged. No control loop runs, so no wake callback fires and every wake cancel returns true.
struct WakeSession {
    run: RunHandle,
    root: ClockDomainId,
    marks: support::Release,
    loss: ParkGate,
    _loss_release: OpenOnDrop,
    _installed: start_probe::Installed,
}

fn wake_session(probe: &Probe, log: &ClockLog, threads: &Threads, hooks: Hooks) -> WakeSession {
    let marks = release();
    let loss = gate();
    let rig = paced();
    let mut assembly = rig.assembly;
    assembly.authority = Box::new(WatchedAuthority::new(assembly.authority, log.clone()).with_hooks(hooks));
    assembly.host_clock = Arc::new(WatchedClock::new(Arc::new(FakeHostClock::new()), log.clone()));
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(ThreadedProvider::new("radio", "radio", probe).marking_on("test.MARK", marks.clone()).losing_device_on(loss.clone())));
    let installed = {
        let (probe, threads) = (probe.clone(), threads.clone());
        start_probe::install(move |point, p| {
            entered(point, p, &probe, &threads);
        })
    };
    let run = session(assembly, &profile_one());
    WakeSession { run, root: rig.root, marks, loss: loss.clone(), _loss_release: OpenOnDrop(loss), _installed: installed }
}

fn on_data_thread() -> bool {
    std::thread::current().name() == Some("ezsdr-data")
}

/// An `on_schedule_return` hook: the data thread's first `schedule` return is recorded in `accepted` (`wake:accepted`) and parked on `park`.
fn park_first_data_wake(probe: &Probe, park: &ParkGate, accepted: &Arc<Mutex<Option<Call>>>) -> impl Fn(&Call) + Send + Sync + 'static {
    let (probe, park, accepted) = (probe.clone(), park.clone(), accepted.clone());
    move |call: &Call| {
        if !on_data_thread() || accepted.lock().unwrap_or_else(|e| e.into_inner()).replace(call.clone()).is_some() {
            return;
        }
        probe.record("wake:accepted");
        wait(&park);
    }
}

#[test]
fn kc_24a_sweep_a_paced_run_with_an_ordinary_and_a_signal_wake_cancels_both() {
    // #120 shape 2, KC-46a: the data thread's ordinary wake (h1) is published, then a Module
    // thread's `disconnect_signal` schedules the unconditional wake (h2): h2's publish must not
    // replace h1, so the cleanup's freeze cancels both, once each, while each is still pending.
    // No `next_wakeup` runs, so no callback fires and both cancels return true.
    // Deschedule: the signal is called only after the reading shows h1 published, and `finish`
    // only after `disconnect_signal` returned (the publish of h2 is inside that call), so the
    // freeze sees both handles whatever the data thread's schedule.
    let probe = Probe::new();
    let log = ClockLog::default();
    let threads = Threads::new();
    let s = wake_session(&probe, &log, &threads, Hooks::default());
    let signal = s.run.disconnect_signal();
    release_one(&s.marks);
    threads.wait_reading("the ordinary wake published", |reading| reading.wake_handles == 1);
    let test_thread = std::thread::current().id();
    signal();
    let manifest = s.run.finish_checked();
    let calls = log.calls();
    let phases = sweep(&calls, &manifest.run.transitions, s.root);
    let wakes: Vec<&Call> = calls.iter().filter(|call| call.kind == CallKind::Schedule && call.handle.is_some()).collect();
    assert_eq!(wakes.len(), 2, "one ordinary wake and one signal wake were scheduled; calls: {calls:?}");
    let h1 = wakes.iter().find(|call| call.thread != test_thread).expect("the data thread's ordinary wake").handle.unwrap();
    let h2 = wakes.iter().find(|call| call.thread == test_thread).expect("the signal's wake").handle.unwrap();
    for (name, handle) in [("ordinary", h1), ("signal", h2)] {
        let cancels = cancels_of(&calls, handle);
        assert_eq!(cancels.len(), 1, "RS-6 step 1: the {name} wake is cancelled exactly once; calls: {calls:?}");
        assert_eq!(cancels[0].returned, Some(true), "RS-6 step 1: the {name} wake was still pending: {:?}", cancels[0]);
        assert!(cancels[0].held.freeze, "KC-24a: the {name} wake is cancelled under the freeze gate: {:?}", cancels[0]);
        let at = calls.iter().position(|call| call.kind == CallKind::Cancel && call.handle == Some(handle)).unwrap();
        assert!(phases.stopping.contains(&at), "KC-24a: the {name} wake's Cancel is not in the Stopping window; phases: {phases:?}");
    }
    assert!(manifest.termination.cleanup_failures.is_empty(), "{:?}", manifest.termination.cleanup_failures);
}

#[test]
fn kc_24a_a_wake_published_before_the_early_freeze_is_cancelled_once_across_both_freezes() {
    // #120, KC-46a, RS-6 step 1: an ordinary wake (h1) is published; the device is then lost,
    // so the data thread's early freeze takes the list and cancels h1 and the data thread
    // publishes its end-wake (h2); the cleanup's freeze cancels h2. Each handle is cancelled
    // exactly once over the two freezes (a freeze that read the list without taking it would
    // cancel h1 twice), h1 by the data thread, h2 by the cleanup.
    // Deschedule: the loss opens only after h1 is published, and `finish` runs only after h2
    // is published (the reading), so each freeze sees the list the plan names.
    let probe = Probe::new();
    let log = ClockLog::default();
    let threads = Threads::new();
    let s = wake_session(&probe, &log, &threads, Hooks::default());
    release_one(&s.marks);
    threads.wait_reading("the ordinary wake published", |reading| reading.wake_handles == 1 && reading.wake_last_generation == 1);
    open(&s.loss);
    let published = threads.wait_reading("the end-wake published", |reading| reading.wake_last_generation == 2 && reading.wake_next == 2);
    assert_eq!(published.wake_handles, 1, "RS-6 step 1: the data thread's freeze took the list, so only the end-wake is in it: {published:?}");
    let manifest = s.run.finish_checked();
    let calls = log.calls();
    let wakes: Vec<&Call> = calls.iter().filter(|call| call.kind == CallKind::Schedule && call.handle.is_some()).collect();
    assert_eq!(wakes.len(), 2, "the ordinary wake and the end-wake were scheduled; calls: {calls:?}");
    let data = wakes[0].thread;
    assert_eq!(wakes[1].thread, data, "both wakes are the data thread's: {wakes:?}");
    let (h1, h2) = (wakes[0].handle.unwrap(), wakes[1].handle.unwrap());
    let c1 = cancels_of(&calls, h1);
    let c2 = cancels_of(&calls, h2);
    assert_eq!(c1.len(), 1, "RS-6 step 1: h1 is cancelled once over both freezes; calls: {calls:?}");
    assert_eq!(c2.len(), 1, "RS-6 step 1: h2 is cancelled once over both freezes; calls: {calls:?}");
    assert!(c1[0].thread == data && c2[0].thread != data, "KC-46a: the data thread's freeze cancels h1, the cleanup's h2: {c1:?} {c2:?}");
    assert!(c1[0].held.freeze && c2[0].held.freeze && c1[0].returned == Some(true) && c2[0].returned == Some(true), "KC-24a: both cancels run under the gate and found a pending wake: {c1:?} {c2:?}");
    assert!(manifest.termination.cleanup_failures.is_empty(), "the cleanup recorded failures: {:?}", manifest.termination.cleanup_failures);
}

#[test]
fn kc_46a_the_wake_list_is_bounded() {
    // KC-46a: the wake list holds the wakes not yet fired and at most one fired handle: after
    // ten rounds of a wake that fires, it holds one entry, not ten.
    // `woken_rounds`' pattern: each round's release comes only after the control loop is
    // seen parked for a fresh horizon, so the round's wake fires and is the only one in
    // flight; the final handle's publication is observed (`wake_last_generation ==
    // wake_next`) before the list is counted, so the count never reads a list one
    // publication short.
    // Deschedule: the control loop parks before each release, the data thread's wake of a
    // round publishes before the next round's wake (the round's `found` follows its
    // callback, which follows its own `schedule`), and the final wait is a condition.
    let probe = Probe::new();
    let marks = release();
    let threads = Threads::new();
    let (rig, park) = paced_parked(|authority| authority);
    let double = ThreadedProvider::new("radio", "radio", &probe).marking_on("test.MARK", marks.clone());
    let mut run = {
        let (probe, threads) = (probe.clone(), threads.clone());
        let _installed = start_probe::install(move |point, p| {
            entered(point, p, &probe, &threads);
        });
        session(provider(rig.assembly, "radio", double), &profile_one())
    };
    let mark = kind("test.MARK");
    let mut from = 0;
    for round in 0..10 {
        let horizon = after(&run, Wall::from_secs(3600));
        let tick = horizon.ticks_in(horizon.domain()).unwrap();
        let (found_tx, found) = mpsc::channel();
        let (run_tx, returned) = mpsc::channel();
        let mark = mark.clone();
        let worker = std::thread::spawn(move || {
            let index = run.wait_for(std::slice::from_ref(&mark), from, horizon).unwrap().expect("the delivered mark is found before the horizon");
            let _ = found_tx.send(index);
            let _ = run_tx.send(run);
        });
        assert!(park.wait_parked_until(tick, Wall::from_secs(10)), "{}", infra(format!("round {round}: the control loop never waited for the horizon")));
        release_one(&marks);
        from = found.recv_timeout(Wall::from_secs(10)).unwrap_or_else(|_| panic!("{}", infra(format!("round {round}: a delivery did not wake the waiting call")))) + 1;
        run = returned.recv_timeout(Wall::from_secs(10)).unwrap_or_else(|_| panic!("{}", infra("the worker did not return the Run within 10 s")));
        worker.join().expect("the round's worker does not panic");
    }
    let reading = threads.wait_reading("the last wake published", |reading| reading.wake_next >= 10 && reading.wake_last_generation == reading.wake_next);
    assert_eq!(reading.wake_handles, 1, "KC-46a: ten fired wakes left more than one entry in the list: {reading:?}");
    let _ = run.finish_checked();
}

/// Runs `finish` on a worker and returns its Manifest channel; the test waits on it with the infra bound.
fn finish_on_worker(run: RunHandle) -> (std::thread::JoinHandle<()>, mpsc::Receiver<Manifest>) {
    let (tx, rx) = mpsc::channel();
    (std::thread::spawn(move || drop(tx.send(run.finish()))), rx)
}

fn manifest_within(rx: &mpsc::Receiver<Manifest>, what: &str) -> Manifest {
    rx.recv_timeout(Wall::from_secs(10)).unwrap_or_else(|_| panic!("{}", infra(format!("{what}: no Manifest within 10 s"))))
}

#[test]
fn kc_46a_a_wake_accepted_before_the_cleanups_freeze_and_published_after_it_is_never_cancelled() {
    // KC-46a, RS-6 step 1: the data thread's ordinary wake is parked after the Authority
    // accepted it (handle h) and before h reaches the list. The cleanup's freeze then takes
    // the (empty) list and closes it; when the publisher resumes, it finds the list closed
    // and drops h: nothing cancels it, and the wake stays scheduled as a no-op.
    // No `next_wakeup` runs, so h never fires. Deschedule: the park opens only after the
    // freeze reached `Done` on the cleanup's thread with the list closed and empty (the
    // reading), so the publisher's decision never races the freeze; the data thread's own
    // schedule is irrelevant until then. The parked data thread is joined by cleanup step 3
    // under RS-8a's default 5 s bound: if the park stays shut for longer, step 3 is abandoned
    // and the final cleanup-failure assertion fails, so the test opens it as soon as it can.
    let probe = Probe::new();
    let log = ClockLog::default();
    let threads = Threads::new();
    let park = gate();
    let _open = OpenOnDrop(park.clone());
    let accepted = Arc::new(Mutex::new(None::<Call>));
    let on_schedule_return = park_first_data_wake(&probe, &park, &accepted);
    let s = wake_session(&probe, &log, &threads, Hooks { on_schedule_return: Some(Arc::new(on_schedule_return)), ..Hooks::default() });
    release_one(&s.marks);
    assert!(probe.wait_for("wake:accepted", Wall::from_secs(10)), "{}", infra("the data thread's wake was not accepted within 10 s"));
    // The publisher is parked, so the list is stable: open and empty, one wake issued.
    let before = threads.read().expect("the Run is live");
    assert!(!before.wake_closed && before.wake_handles == 0 && before.wake_next == 1, "KC-46a: the parked wake's list is not the open, empty one: {before:?}");
    let (finisher, manifest_rx) = finish_on_worker(s.run);
    threads.wait_reading("the list closed by the cleanup's freeze", |reading| reading.wake_closed);
    assert!(probe.wait_until(|lines| lines.iter().any(|line| line.starts_with("freeze:Done:")), Wall::from_secs(10)), "{}", infra("the cleanup's freeze did not reach Done within 10 s"));
    let h = accepted.lock().unwrap_or_else(|e| e.into_inner()).clone().expect("the accepted wake was recorded");
    assert!(cancels_of(&log.calls(), h.handle.unwrap()).is_empty(), "KC-46a: the freeze cancelled a handle it never held; calls: {:?}", log.calls());
    open(&park);
    let manifest = manifest_within(&manifest_rx, "the cleanup after the wake was published");
    finisher.join().expect("W_f does not panic");
    // The data thread is joined by cleanup step 3, so a cancel its publisher made would be in the log by now.
    let calls = log.calls();
    assert!(cancels_of(&calls, h.handle.unwrap()).is_empty(), "KC-46a: the wake published into the closed list was cancelled, by the freeze or its publisher; calls: {calls:?}");
    assert!(manifest.termination.cleanup_failures.is_empty(), "{:?}", manifest.termination.cleanup_failures);
}

#[test]
fn kc_46a_the_data_threads_end_wake_published_after_its_freeze_is_cancelled_by_the_cleanups() {
    // KC-46a, RS-6, RS-7: the data thread's end-wake (h) is accepted after its own early
    // performance of step 1 and parked before it reaches the list. The data thread's
    // performance did not close the list, so h is pushed to the open list when the park
    // opens, and the cleanup's performance cancels it: after the data thread's Provider stop,
    // under the freeze gate, on the cleanup's thread, once.
    // The Provider's loss opens the data thread's stop; no `next_wakeup` runs, so h never fires.
    // Deschedule: the park is the data thread's only schedule return; the test reads the list
    // open and empty while it is parked, and `finish` runs only after the reading shows h
    // published, so the cleanup's performance always meets it in the list.
    let probe = Probe::new();
    let log = ClockLog::default();
    let threads = Threads::new();
    let park = gate();
    let _open = OpenOnDrop(park.clone());
    let accepted = Arc::new(Mutex::new(None::<Call>));
    let on_schedule_return = park_first_data_wake(&probe, &park, &accepted);
    let on_cancel = {
        let probe = probe.clone();
        move |call: &Call| probe.record(format!("cancel:{:?}", call.handle.unwrap()))
    };
    let s = wake_session(&probe, &log, &threads, Hooks { on_schedule_return: Some(Arc::new(on_schedule_return)), on_cancel: Some(Arc::new(on_cancel)), ..Hooks::default() });
    open(&s.loss);
    assert!(probe.wait_until(|lines| lines.iter().any(|line| line.starts_with("freeze:Done:")), Wall::from_secs(10)), "{}", infra("the data thread's freeze did not reach Done within 10 s"));
    let data = *threads.data.lock().unwrap_or_else(|e| e.into_inner()).as_ref().expect("the data thread's freeze was recorded");
    // The list was taken at that performance and is stable until the cleanup's: still open.
    let after_freeze = threads.read().expect("the Run is live");
    assert!(!after_freeze.wake_closed, "KC-46a: the data thread's performance closed the wake list: {after_freeze:?}");
    assert!(probe.wait_for("wake:accepted", Wall::from_secs(10)), "{}", infra("the data thread's end-wake was not accepted within 10 s"));
    let lines = probe.lines();
    assert!(lines.iter().any(|line| *line == format!("freeze:Done:{data:?}")), "the end-wake is not after the data thread's performance of step 1: {lines:?}");
    // The publisher is parked after the data thread's performance, so the list is stable: that performance left it open and empty.
    let parked = threads.read().expect("the Run is live");
    assert!(!parked.wake_closed && parked.wake_handles == 0 && parked.wake_next == 1, "KC-46a: the data thread's performance closed the wake list or left a handle: {parked:?}");
    open(&park);
    threads.wait_reading("the end-wake published", |reading| reading.wake_handles == 1 && reading.wake_last_generation == reading.wake_next);
    let manifest = s.run.finish_checked();
    let calls = log.calls();
    let h = accepted.lock().unwrap_or_else(|e| e.into_inner()).clone().expect("the accepted wake was recorded");
    let cancels = cancels_of(&calls, h.handle.unwrap());
    assert_eq!(cancels.len(), 1, "KC-46a: the end-wake is cancelled exactly once; calls: {calls:?}");
    assert_ne!(cancels[0].thread, data, "KC-46a: the data thread's performance closed the list: it cancelled its own end-wake; {cancels:?}");
    assert!(cancels[0].held.freeze && cancels[0].returned == Some(true), "KC-24a: the cleanup cancels the end-wake under the gate while it is pending: {cancels:?}");
    let lines = probe.lines();
    let last_stop = lines.iter().rposition(|line| line.starts_with("freeze:BeforeStop:")).expect("the data thread stopped the Provider");
    let cancelled = lines.iter().position(|line| *line == format!("cancel:{:?}", h.handle.unwrap())).expect("the cancel was recorded");
    assert!(last_stop < cancelled, "RS-7: the end-wake's cancel is not after the data thread's last Provider stop: {lines:?}");
    assert!(manifest.termination.cleanup_failures.is_empty(), "{:?}", manifest.termination.cleanup_failures);
}

#[test]
fn kc_46a_a_wake_after_the_cleanups_freeze_schedules_nothing() {
    // KC-46a: once the cleanup's performance of step 1 closed the list, the data thread's
    // ordinary wakes schedule nothing (no `schedule`, so no handle to cancel): a wake that
    // scheduled and at once cancelled would sit beside every delivery until the data thread
    // is joined.
    // The freeze hook parks the cleanup thread at its `Done` (list closed, gate held); the
    // Provider then emits a mark, the data thread delivers it and calls `wake`, and the test
    // waits for three data-thread `now()` reads after the mark was emitted. The first pass whose
    // drain follows the emission is the one in progress or the next; a wake that did not stop at
    // the closed list reads `now()` itself, and its `schedule` follows that read. Of the three
    // reads, the last begins a pass after that wake call returned, whichever pass delivered and
    // whether or not the wake reads the time. The log must hold no Schedule and no Cancel after
    // the `Done`.
    // Deschedule: the data thread descheduled anywhere only delays the pass count; the mark
    // is released only after the park was reached, so the order the test names cannot flip.
    // The cleanup thread sits at `Done` under RS-8a's default 5 s bound for FreezeDispatch, so
    // the three reads must come within it, or the freeze is abandoned and the final
    // cleanup-failure assertion fails (rather than the 10 s infra bound).
    let probe = Probe::new();
    let log = ClockLog::default();
    let threads = Threads::new();
    let hold = gate();
    let _open = OpenOnDrop(hold.clone());
    let data_nows = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let on_now = {
        let data_nows = data_nows.clone();
        move |_call: &Call| {
            if on_data_thread() {
                data_nows.fetch_add(1, Ordering::SeqCst);
            }
        }
    };
    let done_at = Arc::new(Mutex::new(None::<usize>));
    let s = {
        let s = wake_session(&probe, &log, &threads, Hooks { on_now: Some(Arc::new(on_now)), ..Hooks::default() });
        let run_probe = threads.run.lock().unwrap_or_else(|e| e.into_inner()).clone().expect("Entered stored the Run's probe");
        let (hold, log, done_at, probe) = (hold.clone(), log.clone(), done_at.clone(), probe.clone());
        run_probe.on_freeze(move |point, _reading| {
            if point == FreezePoint::Done {
                *done_at.lock().unwrap_or_else(|e| e.into_inner()) = Some(log.calls().len());
                probe.record("freeze:parked");
                wait(&hold);
            }
        });
        s
    };
    let (finisher, manifest_rx) = finish_on_worker(s.run);
    assert!(probe.wait_for("freeze:parked", Wall::from_secs(10)), "{}", infra("the cleanup's freeze did not reach Done within 10 s"));
    let closed = threads.read().expect("the Run is live");
    assert!(closed.wake_closed, "KC-46a: the cleanup's performance did not close the wake list: {closed:?}");
    release_one(&s.marks);
    assert!(probe.wait_for("radio:marked:test.MARK", Wall::from_secs(10)), "{}", infra("the Provider did not emit its mark within 10 s"));
    let emitted = data_nows.load(Ordering::SeqCst);
    let deadline = Instant::now() + Wall::from_secs(10);
    while data_nows.load(Ordering::SeqCst) < emitted + 3 {
        assert!(Instant::now() < deadline, "{}", infra("the data thread did not read the time three times within 10 s"));
        std::thread::yield_now();
    }
    // The mark was delivered, so the data thread called `wake` on the closed list; the list is
    // closed before `wake` counts a generation, so none was counted. (A wake that went on to
    // `schedule` counts first, so this holds whatever the log has caught by now.)
    let after = threads.read().expect("the Run is live");
    assert!(after.delivered_len > closed.delivered_len, "{}", infra(format!("the mark was not delivered: {closed:?} -> {after:?}")));
    assert_eq!(after.wake_next, 0, "KC-46a: a wake counted a generation after the cleanup's freeze closed the list: {after:?}");
    let calls = log.calls();
    let from = done_at.lock().unwrap_or_else(|e| e.into_inner()).expect("the freeze recorded its Done");
    let after_done: Vec<&Call> = calls[from..].iter().filter(|call| matches!(call.kind, CallKind::Schedule | CallKind::Cancel)).collect();
    assert!(after_done.is_empty(), "KC-46a: a wake scheduled or cancelled after the cleanup's freeze closed the list: {after_done:?}");
    open(&hold);
    let manifest = manifest_within(&manifest_rx, "the cleanup after the freeze was released");
    finisher.join().expect("W_f does not panic");
    assert!(manifest.termination.cleanup_failures.is_empty(), "{:?}", manifest.termination.cleanup_failures);
}
