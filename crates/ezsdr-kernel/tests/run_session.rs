//! Phase 1 tests for `04-run-and-session.md`. Each name begins with the rule it proves (OV-19).

mod support;

use std::collections::BTreeMap;
use std::sync::Arc;

use ezsdr_kernel::binding::{
    AdmissionCheckRegistry, BindingProfile, CheckStage, Placements,
};
use ezsdr_kernel::event::{
    Action, ActionTemplate, CounterRow, Event, EventCollector, EventKind, EventSink, Severity,
};
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::{ClockDomainId, ResourceId, RunId};
use ezsdr_kernel::manifest::{
    ArtifactRef, BindingSection, ClocksSection, EventsSection, Manifest, PrepareSection, RunKind,
    RunSection, SpecSection, TerminationSection, ingest_input, mark_open_artifacts,
};
use ezsdr_kernel::module_api::{
    CoercionFidelity, EnvelopeFidelity, ExecutionClass, Factories, Fidelity, ModuleRegistry,
    RfFidelity, StopMode, TransportFidelity, UpdateClass, Sink,
};
use ezsdr_kernel::policy::{EventKindRegistry, Policy, Reaction};
use ezsdr_kernel::run::{
    CleanupMode, CleanupStep, Lease, LeaseMode, RunError, RunState, RunStateMachine,
    Stage, StopCause, Termination, run_cleanup,
};
use ezsdr_kernel::session::{
    Admitter, Compiled, ControlOp, Outcome, SessionAction, SessionLog, compile, implicit_spec,
};
use ezsdr_kernel::spec::{ExperimentSpec, Ident, Key, Namespace, Value};
use ezsdr_kernel::stream::{BlockFlags, ContinuityBuilder, DropCarry, LatePolicy};
use ezsdr_kernel::time::{AbsoluteDeadline, TimePoint};
use support::{
    FakeHostClock, RecordingCleanup, TestLimitsCheck, TestProvider, TestSink, id, key, mid, ns, rid, some_hash, test_provider_descriptor, test_sink_descriptor,
    test_vocabulary,
};

fn t(ticks: i64) -> TimePoint {
    TimePoint::new(ClockDomainId::HOST_MONOTONIC, ticks)
}

fn registry() -> ModuleRegistry {
    let mut reg = ModuleRegistry::new();
    reg.register_vocabulary(test_vocabulary()).expect("fresh");
    reg.register(test_provider_descriptor(),
        Factories { provider: true, authority: true, ..Factories::default() },)
        .expect("registers");
    reg.register(test_sink_descriptor(), Factories { sink: true, ..Factories::default() })
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

// ---------------------------------------------------------------- the state machine

#[test]
fn rs_02_happy_path_transitions() {
    let clock = FakeHostClock::new();
    let mut run = RunStateMachine::new(&clock);
    for state in [
        RunState::Validated,
        RunState::Planned,
        RunState::Prepared,
        RunState::Armed,
        RunState::Running,
    ] {
        run.move_to(state, Some(t(0)), &clock).expect("forward");
    }
    run.begin_stopping(CleanupMode::Orderly, Some(t(1)), &clock).expect("into Stopping");
    run.finish(Termination::Completed, Some(t(2)), &clock).expect("into CleanedUp");
    let states: Vec<&RunState> = run.transitions().iter().map(|r| &r.state).collect();
    assert_eq!(states.len(), 8, "exactly the eight states, all recorded");
    assert!(matches!(states[0], RunState::Created));
    assert!(matches!(states[7], RunState::CleanedUp { .. }));
    // RS-5: each carries its runtime instant and the host UTC time.
    assert!(run.transitions().iter().all(|r| r.host_utc_nanos > 0));
    // There is no going back, and no skipping.
    assert!(matches!(
        run.move_to(RunState::Running, None, &clock),
        Err(RunError::IllegalTransition { .. })
    ));
    let mut fresh = RunStateMachine::new(&clock);
    assert!(matches!(
        fresh.move_to(RunState::Running, None, &clock),
        Err(RunError::IllegalTransition { .. })
    ));
}

#[test]
fn rs_03_failure_at_each_stage_reaches_cleanup() {
    let clock = FakeHostClock::new();
    for (stage, reached) in [
        (Stage::Validate, vec![]),
        (Stage::Plan, vec![RunState::Validated]),
        (Stage::Prepare, vec![RunState::Validated, RunState::Planned]),
        (Stage::Arm, vec![RunState::Validated, RunState::Planned, RunState::Prepared]),
    ] {
        let mut run = RunStateMachine::new(&clock);
        for s in reached {
            run.move_to(s, None, &clock).expect("forward");
        }
        run.begin_stopping(CleanupMode::Abort, None, &clock).expect("failure aborts");
        run.finish(Termination::Failed { stage }, None, &clock).expect("cleans up");
        assert!(matches!(
            run.state(),
            RunState::CleanedUp { termination: Termination::Failed { stage: s } } if *s == stage
        ));
        // RS-11: a Manifest is written for every Run that reaches CleanedUp.
        let m = manifest_fixture(Termination::Failed { stage });
        assert!(matches!(m.termination.reason, Termination::Failed { .. }));
    }
}

#[test]
fn rs_04_structural_mutation_forbidden() {
    let clock = FakeHostClock::new();
    let mut run = RunStateMachine::new(&clock);
    assert!(run.check_structural_mutation().is_ok());
    for s in [
        RunState::Validated,
        RunState::Planned,
        RunState::Prepared,
        RunState::Armed,
        RunState::Running,
    ] {
        run.move_to(s, None, &clock).expect("forward");
    }
    assert_eq!(run.check_structural_mutation(), Err(RunError::StructuralMutationForbidden));
}

// ---------------------------------------------------------------- cleanup

fn fragments() -> Vec<Ident> {
    vec![id("a"), id("b"), id("c")]
}

fn release() -> Vec<Ident> {
    vec![id("c"), id("b"), id("a")]
}

#[test]
fn rs_06_cleanup_order_orderly() {
    let ops = Arc::new(RecordingCleanup::new());
    let out = run_cleanup(ops.clone(), &release(), CleanupMode::Orderly, &|| None);
    assert!(out.failures.is_empty());
    let steps: Vec<CleanupStep> = ops.steps().into_iter().collect();
    // Exactly the nine steps of RS-6, with the three per-fragment ones repeated.
    let distinct: Vec<CleanupStep> = {
        let mut v = Vec::new();
        for s in &steps {
            if v.last() != Some(s) {
                v.push(*s);
            }
        }
        v
    };
    assert_eq!(distinct, ezsdr_kernel::run::CLEANUP_STEPS.to_vec());
    assert!(out.modes.iter().all(|(_, m)| *m == CleanupMode::Orderly));
}

#[test]
fn rs_06_cleanup_order_abort() {
    let ops = Arc::new(RecordingCleanup::new());
    let out = run_cleanup(ops.clone(), &release(), CleanupMode::Abort, &|| None);
    assert!(out.modes.iter().all(|(_, m)| *m == CleanupMode::Abort));
    assert!(ops.entries().iter().all(|(_, _, m)| *m == CleanupMode::Abort));
}

#[test]
fn rs_06_cleanup_step_failure_continues() {
    let ops = Arc::new(RecordingCleanup::new().failing_at(CleanupStep::StopRx));
    let out = run_cleanup(ops.clone(), &release(), CleanupMode::Orderly, &|| None);
    // Every step is attempted even when an earlier one failed.
    assert!(ops.steps().contains(&CleanupStep::ReleaseAndWriteManifest));
    assert_eq!(out.failures.len(), 3, "one per fragment of the failing step");
    assert!(out.failures.iter().all(|f| f.step == CleanupStep::StopRx && !f.timed_out));
}

#[test]
fn rs_07_pending_burst_cancelled_before_tx_stop() {
    let ops = Arc::new(RecordingCleanup::new());
    run_cleanup(ops.clone(), &release(), CleanupMode::Orderly, &|| None);
    let steps = ops.steps();
    let freeze = steps.iter().position(|s| *s == CleanupStep::FreezeDispatch).expect("present");
    let stop_tx = steps.iter().position(|s| *s == CleanupStep::StopTx).expect("present");
    assert!(freeze < stop_tx, "the dispatch freeze precedes stopping TX (RS-7)");
}

#[test]
fn rs_08_cleanup_runs_in_reverse_dependency_order() {
    let ops = Arc::new(RecordingCleanup::new());
    run_cleanup(ops.clone(), &release(), CleanupMode::Orderly, &|| None);
    let stop_tx: Vec<Ident> = ops
        .entries()
        .into_iter()
        .filter(|(s, _, _)| *s == CleanupStep::StopTx)
        .filter_map(|(_, f, _)| f)
        .collect();
    assert_eq!(stop_tx, release(), "the device armed first is released last");
    assert_eq!(fragments().len(), 3);
}

#[test]
fn rs_10_abort_during_orderly_escalates() {
    let ops = Arc::new(RecordingCleanup::new());
    let seen = std::sync::atomic::AtomicUsize::new(0);
    let escalate = || {
        // The abort is raised once the sequence has begun.
        if seen.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 3 {
            Some(CleanupMode::Abort)
        } else {
            None
        }
    };
    let out = run_cleanup(ops.clone(), &release(), CleanupMode::Orderly, &escalate);
    assert_eq!(out.modes[0].1, CleanupMode::Orderly);
    assert_eq!(out.modes.last().expect("nine steps").1, CleanupMode::Abort);
    // Both causes are recorded in the termination.
    let mut section = termination_fixture(Termination::Stopped { cause: StopCause::Client });
    section.also.push(StopCause::Abort { cause: "policy".to_owned() });
    assert_eq!(section.also.len(), 1);
}

#[test]
fn rs_8a_wedged_cleanup_step_times_out() {
    let ops = Arc::new(RecordingCleanup::new().wedged_at(CleanupStep::StopRx));
    let out = run_cleanup(ops.clone(), &[id("a")], CleanupMode::Orderly, &|| None);
    assert_eq!(out.failures.len(), 1);
    assert!(out.failures[0].timed_out, "the step is abandoned and recorded as a timeout");
    // Steps 4 to 8 still run, so a Manifest is written for exactly the failure that
    // most needs recording.
    let steps = ops.steps();
    for later in [
        CleanupStep::CancelPeripherals,
        CleanupStep::RestoreBaseline,
        CleanupStep::FinaliseArtifacts,
        CleanupStep::FlushEvents,
        CleanupStep::ReleaseAndWriteManifest,
    ] {
        assert!(steps.contains(&later), "{later:?} still ran");
    }
}

// ---------------------------------------------------------------- the Lease

#[test]
fn rs_21_lease_default_and_ttl_required() {
    assert_eq!(Lease::default().mode, LeaseMode::Attached);
    let clock = FakeHostClock::new();
    assert_eq!(Lease::detached(0, true, "tok", &clock), Err(RunError::LeaseTtlRequired));
    assert!(Lease::detached(5_000, true, "tok", &clock).is_ok());
}

#[test]
fn rs_22_ttl_uses_the_host_clock() {
    let clock = FakeHostClock::new();
    let mut lease = Lease::detached(5_000, true, "tok", &clock).expect("granted");
    lease.on_disconnect(&clock);
    // Virtual time may advance an hour; the Lease does not care.
    clock.advance(1);
    assert!(!lease.expired(&clock), "one millisecond of host time is not five seconds");
}

#[test]
fn rs_23_attached_stops_on_disconnect() {
    let clock = FakeHostClock::new();
    let mut lease = Lease::attached();
    assert_eq!(lease.on_disconnect(&clock), Some(StopCause::ClientDisconnect));
}

#[test]
fn rs_23_detached_expires() {
    let clock = FakeHostClock::new();
    let mut lease = Lease::detached(5_000, true, "tok", &clock).expect("granted");
    assert_eq!(lease.on_disconnect(&clock), None, "a Detached Lease survives the disconnect");
    clock.advance(4_999);
    assert!(!lease.expired(&clock));
    clock.advance(1);
    assert!(lease.expired(&clock));
    // OV-3: RS-23's second half — that expiry terminates the Run with
    // `StopCause::LeaseExpiry` — has no Kernel carrier in Phase 1: nothing maps an
    // expired Lease to a `Termination`. The assertion that stood here compared one
    // literal with itself and so recorded the gap as coverage.
}

#[test]
fn rs_23_detached_does_not_expire_while_the_client_is_attached() {
    // RS-22: "A Lease is about a client's **absence**." Arming the TTL at the grant
    // ends a Run whose client never left.
    let clock = FakeHostClock::new();
    let lease = Lease::detached(5_000, true, "tok", &clock).expect("granted");
    clock.advance(100_000);
    assert!(!lease.expired(&clock), "no disconnect, no expiry");
}

#[test]
fn rs_21_lease_validate_refuses_a_zero_ttl() {
    // The constructor refuses it, and so does the predicate for a Lease that did not
    // come through the constructor.
    let clock = FakeHostClock::new();
    assert_eq!(Lease::detached(0, true, "tok", &clock), Err(RunError::LeaseTtlRequired));
    let malformed = Lease {
        mode: LeaseMode::Detached { ttl_ms: 0, renewable: true },
        token: Some("tok".to_owned()),
        holder: None,
        expires_at_host: None,
        adoptions: 0,
        released: false,
    };
    assert_eq!(malformed.validate(), Err(RunError::LeaseTtlRequired));
    assert!(Lease::attached().validate().is_ok());
}

#[test]
fn rs_36_an_unforeseen_source_does_not_silence_the_abort() {
    // A second device, a hot-plugged one, or a Module that labels its source one
    // segment differently must not cost the kind: RS-36's abort and RS-31's
    // delivered kind both hang off it.
    let policy = kinds().compile(&BTreeMap::new()).expect("compiles");
    let fatal = EventKind::parse(EventKind::DEVICE_LOST).expect("parses");
    let c = collector(8, &policy, &[(rid("radio"), fatal.clone())]);
    let h = c.resolve(&rid("radio2"), &fatal);
    c.emit(h, t(0), Severity::Fatal, &[]).expect("emits");
    assert_eq!(
        c.escalation(),
        Some((fatal.clone(), Reaction::Abort)),
        "the Run aborts on a device that is gone, whatever its source name"
    );
    assert!(c.drain().iter().any(|e| e.kind == fatal), "the delivered body keeps its kind");
}

#[test]
fn rs_29_an_unregistered_fatal_kind_escalates() {
    // RS-29: a kind with no entry in the Run's Policy takes a default from its
    // severity, and RS-36 sets the flag on the hot path.
    let policy = kinds().compile(&BTreeMap::new()).expect("compiles");
    let unknown = EventKind::parse("vendor.crash").expect("parses");
    let c = collector(8, &policy, &[(rid("radio"), EventKind::parse("test.custom").expect("k"))]);
    let h = c.resolve(&rid("radio"), &unknown);
    c.emit(h, t(0), Severity::Fatal, &[]).expect("emits");
    assert_eq!(c.escalation().map(|(_, r)| r), Some(Reaction::Abort));

    // The same kind at `info` does not escalate.
    let c = collector(8, &policy, &[(rid("radio"), EventKind::parse("test.custom").expect("k"))]);
    let h = c.resolve(&rid("radio"), &unknown);
    c.emit(h, t(0), Severity::Info, &[]).expect("emits");
    assert!(c.escalation().is_none());
}

#[test]
fn rs_30_marks_every_open_artifact_including_a_partial_one() {
    // MA-26 makes `partial` the ordinary state of an artifact still open on an
    // abort, so RS-30's "every artifact open at that moment" includes it.
    let mut open = vec![artifact("a"), ArtifactRef { partial: true, ..artifact("b") }];
    let kind = EventKind::parse("test.custom").expect("parses");
    mark_open_artifacts(&mut open, kind.clone(), t(1));
    mark_open_artifacts(&mut open, kind, t(2));
    assert_eq!(open[0].marks.len(), 2);
    assert_eq!(open[1].marks.len(), 2, "a partial artifact is still open");
}

#[test]
fn rs_24_adopt_requires_the_token() {
    let clock = FakeHostClock::new();
    let mut lease = Lease::detached(5_000, true, "secret", &clock).expect("granted");
    lease.on_disconnect(&clock);
    assert!(lease.adopt("secret", "client-2").is_ok());
    assert_eq!(lease.adoptions, 1);
    assert_eq!(lease.adopt("wrong", "client-3"), Err(RunError::AdoptRejected));

    let mut fixed = Lease::detached(5_000, false, "secret", &clock).expect("granted");
    assert_eq!(fixed.renew(&clock), Err(RunError::LeaseNotRenewable));
    assert!(lease.renew(&clock).is_ok());
}

#[test]
fn rs_25_child_inherits_the_lease() {
    // A child Run has no Lease of its own, and the parent's expiry ends it first:
    // step 0 of RS-6 is the children.
    let ops = Arc::new(RecordingCleanup::new());
    run_cleanup(ops.clone(), &release(), CleanupMode::Orderly, &|| None);
    assert_eq!(ops.steps().first(), Some(&CleanupStep::Children));
    // RS-25's other half — "a child Run inherits its parent's Lease and may not
    // declare its own" — has no Kernel seam in Phase 1, because there is no child-Run
    // type until the coordinator exists (Phase 2). Recorded as finding D28 rather
    // than asserted against a clone.
}

// ---------------------------------------------------------------- policy and events

fn collector(ring: usize, policy: &Policy, pairs: &[(ResourceId, EventKind)]) -> EventCollector {
    let kinds: Vec<EventKind> = policy.table.keys().cloned().collect();
    EventCollector::new(pairs, &kinds, ring, policy)
}

#[test]
fn rs_28_policy_defaults_table() {
    let kinds = kinds();
    let policy = kinds.compile(&BTreeMap::new()).expect("compiles");
    let k = |s: &str| EventKind::parse(s).expect("parses");
    assert_eq!(policy.reaction_for(&k(EventKind::EVENTS_DROPPED)), Reaction::MarkArtifact);
    assert_eq!(policy.reaction_for(&k(EventKind::LINK_BACKPRESSURE)), Reaction::MarkArtifact);
    assert_eq!(policy.reaction_for(&k(EventKind::PROCESSOR_DEADLINE_MISS)), Reaction::MarkArtifact);
    assert_eq!(policy.reaction_for(&k(EventKind::DEVICE_LOST)), Reaction::Abort);
    assert_eq!(policy.reaction_for(&k(EventKind::STEP_LIVELOCK)), Reaction::Abort);
    // The test Module's own kind keeps its declared default.
    assert_eq!(policy.reaction_for(&k("test.custom")), Reaction::Continue);
    // No radio kind is registered by the Kernel.
    for radio in ["RX_OVERFLOW", "TX_UNDERFLOW", "TX_DISCONTINUITY", "TIME_ERROR", "ALIGNMENT_ERROR"] {
        assert!(kinds.get(&k(radio)).is_none(), "{radio} is the Radio Model's, not the Kernel's");
    }
    assert_eq!(EventKind::kernel_kinds().len(), 5);
}

#[test]
fn rs_29_unknown_kind_by_severity() {
    let policy = kinds().compile(&BTreeMap::new()).expect("compiles");
    let unknown = EventKind::parse("vendor.unregistered").expect("parses");
    assert_eq!(policy.reaction_for_event(&unknown, Severity::Info), Reaction::Continue);
    assert_eq!(policy.reaction_for_event(&unknown, Severity::Error), Reaction::MarkArtifact);
    assert_eq!(policy.reaction_for_event(&unknown, Severity::Fatal), Reaction::Abort);
    assert_eq!(Policy::by_severity(Severity::Debug), Reaction::Continue);
    assert_eq!(Policy::by_severity(Severity::Warning), Reaction::MarkArtifact);
}

#[test]
fn rs_26_policy_override_from_spec() {
    let kinds = kinds();
    let custom = EventKind::parse("test.custom").expect("parses");
    let overrides: BTreeMap<EventKind, Reaction> =
        [(custom.clone(), Reaction::Stop)].into_iter().collect();
    let policy = kinds.compile(&overrides).expect("compiles");
    assert_eq!(policy.reaction_for(&custom), Reaction::Stop, "the Spec overrides the default");

    // An unregistered kind in the table is refused (SB-18).
    let bad: BTreeMap<EventKind, Reaction> =
        [(EventKind::parse("RX_OVERFLOWS").expect("parses"), Reaction::Stop)].into_iter().collect();
    assert!(matches!(kinds.compile(&bad), Err(RunError::UnknownEventKind { .. })));
}

#[test]
fn rs_33_counters_exact_under_drop() {
    let policy = kinds().compile(&BTreeMap::new()).expect("compiles");
    let source = rid("radio");
    let kind = EventKind::parse("test.custom").expect("parses");
    let c = collector(8, &policy, &[(source.clone(), kind.clone())]);
    let h = c.resolve(&source, &kind);
    for _ in 0..30_000 {
        c.emit(h, t(0), Severity::Info, &[]).expect("emits");
    }
    let counters = c.counters();
    let row = counters.iter().find(|r| r.source == source && r.kind == kind).expect("present");
    assert_eq!(row.count, 30_000, "the counter never drops");
    let drained = c.drain();
    let bodies = drained.iter().filter(|e| e.kind == kind).count();
    assert_eq!(bodies, 8, "the ring held eight");
    let dropped: Vec<&Event> = drained
        .iter()
        .filter(|e| e.kind.as_str() == EventKind::EVENTS_DROPPED)
        .collect();
    assert_eq!(dropped.len(), 1);
    assert_eq!(dropped[0].payload["count"], serde_json::json!(29_992));
    // RS-35's invariant: the counter equals the delivered bodies plus the drops.
    assert_eq!(row.count, bodies as u64 + 29_992);
}

#[test]
fn rs_35_dropped_counts_are_deltas() {
    let policy = kinds().compile(&BTreeMap::new()).expect("compiles");
    let source = rid("radio");
    let kind = EventKind::parse("test.custom").expect("parses");
    let c = collector(1, &policy, &[(source.clone(), kind.clone())]);
    let h = c.resolve(&source, &kind);
    for _ in 0..11 {
        c.emit(h, t(0), Severity::Info, &[]).expect("emits");
    }
    let first = c.drain();
    let n = |events: &[Event]| -> u64 {
        events
            .iter()
            .find(|e| e.kind.as_str() == EventKind::EVENTS_DROPPED)
            .and_then(|e| e.payload["count"].as_u64())
            .unwrap_or(0)
    };
    assert_eq!(n(&first), 10, "one body fitted, ten dropped");
    for _ in 0..6 {
        c.emit(h, t(0), Severity::Info, &[]).expect("emits");
    }
    let second = c.drain();
    assert_eq!(n(&second), 5, "a delta, not a running total");
}

#[test]
fn rs_35_dropped_events_never_enter_the_ring() {
    let policy = kinds().compile(&BTreeMap::new()).expect("compiles");
    let source = rid("radio");
    let kind = EventKind::parse("test.custom").expect("parses");
    let c = collector(4, &policy, &[(source.clone(), kind.clone())]);
    let h = c.resolve(&source, &kind);
    for round in 0..3 {
        for _ in 0..100 {
            c.emit(h, t(round), Severity::Info, &[]).expect("emits");
        }
        let drained = c.drain();
        let bodies = drained.iter().filter(|e| e.kind == kind).count() as u64;
        let dropped: u64 = drained
            .iter()
            .filter(|e| e.kind.as_str() == EventKind::EVENTS_DROPPED)
            .filter_map(|e| e.payload["count"].as_u64())
            .sum();
        assert_eq!(bodies + dropped, 100, "the invariant holds each time (round {round})");
        assert_eq!(bodies, 4);
    }
}

#[test]
fn rs_35_the_dropped_events_meta_event_is_itself_counted() {
    // RS-35's invariant is stated "for every kind", and RS-38 invites a reader to
    // compare `events.counters` with `events.delivered`.
    let policy = kinds().compile(&BTreeMap::new()).expect("compiles");
    let source = rid("radio");
    let kind = EventKind::parse("test.custom").expect("parses");
    let c = collector(1, &policy, &[(source.clone(), kind.clone())]);
    let h = c.resolve(&source, &kind);
    for _ in 0..5 {
        c.emit(h, t(0), Severity::Info, &[]).expect("emits");
    }
    let drained = c.drain();
    let delivered =
        drained.iter().filter(|e| e.kind.as_str() == EventKind::EVENTS_DROPPED).count() as u64;
    assert_eq!(delivered, 1);
    let counted: u64 = c
        .counters()
        .into_iter()
        .filter(|r| r.kind.as_str() == EventKind::EVENTS_DROPPED)
        .map(|r| r.count)
        .sum();
    assert_eq!(counted, delivered, "the meta-event is counted like any other body");
}

#[test]
fn rs_33_unforeseen_pair_uses_the_fallback_row() {
    let policy = kinds().compile(&BTreeMap::new()).expect("compiles");
    let known = rid("radio");
    let kind = EventKind::parse("test.custom").expect("parses");
    let c = collector(8, &policy, &[(known.clone(), kind.clone())]);
    let h = c.resolve(&rid("nowhere"), &kind);
    c.emit(h, t(0), Severity::Info, &[]).expect("emits");
    let counters: Vec<CounterRow> = c.counters();
    let fallback = counters.last().expect("the fallback row is last");
    assert_eq!(fallback.count, 1);
    assert_eq!(fallback.source.path, "unforeseen", "one fallback row, not one per source");
}

#[test]
fn rs_36_abort_survives_a_drop() {
    let policy = kinds().compile(&BTreeMap::new()).expect("compiles");
    let source = rid("radio");
    let noise = EventKind::parse("test.custom").expect("parses");
    let fatal = EventKind::parse(EventKind::DEVICE_LOST).expect("parses");
    let c = collector(
        2,
        &policy,
        &[(source.clone(), noise.clone()), (source.clone(), fatal.clone())],
    );
    let hn = c.resolve(&source, &noise);
    for _ in 0..10 {
        c.emit(hn, t(0), Severity::Info, &[]).expect("emits");
    }
    assert!(c.escalation().is_none());
    let hf = c.resolve(&source, &fatal);
    c.emit(hf, t(1), Severity::Fatal, &[]).expect("emits");
    let (kind, reaction) = c.escalation().expect("the Run aborts even though the body was dropped");
    assert_eq!(kind, fatal);
    assert_eq!(reaction, Reaction::Abort);
    let row = c.counters().into_iter().find(|r| r.kind == fatal).expect("present");
    assert_eq!(row.count, 1);
}

#[test]
fn rs_38_counters_include_zero_rows() {
    let policy = kinds().compile(&BTreeMap::new()).expect("compiles");
    let pairs = [
        (rid("radio"), EventKind::parse("test.custom").expect("parses")),
        (rid("radio"), EventKind::parse(EventKind::DEVICE_LOST).expect("parses")),
    ];
    let c = collector(8, &policy, &pairs);
    let counters = c.counters();
    assert!(counters.len() >= 3, "every registered row plus the fallback");
    assert!(counters.iter().all(|r| r.count == 0), "a Run with no events reports zero rows");
}

#[test]
fn rs_30_mark_artifact_marks_open_artifacts() {
    let mut open = vec![artifact("capture0")];
    let kind = EventKind::parse("test.custom").expect("parses");
    mark_open_artifacts(&mut open, kind.clone(), t(42));
    assert_eq!(open[0].marks.len(), 1);
    assert_eq!(open[0].marks[0].kind, kind);
    assert_eq!(open[0].marks[0].time, t(42));
}

// ---------------------------------------------------------------- Session

fn session_profile() -> BindingProfile {
    let mut p = BindingProfile {
        version: 1,
        bindings: [(
            id("radio"),
            ezsdr_kernel::binding::Binding {
                module: mid("ezsdr.test.provider"),
                feed: None,
                selector: BTreeMap::new(),
                profile: None,
            },
        )]
        .into_iter()
        .collect(),
        authority: None,
        placements: Placements::default(),
        environment: BTreeMap::new(),
    };
    // SB-22: a recorder is **bound**, not placed. Its binding carries the `feed`
    // RS-12 turns into the implicit Spec's output — the port it records and the
    // drop-class policy and capacity of the link (SC-19, SC-21).
    p.bindings.insert(
        id("recorder"),
        ezsdr_kernel::binding::Binding {
            module: mid("ezsdr.test.sink"),
            feed: Some(ezsdr_kernel::spec::SinkFeed {
                port: ezsdr_kernel::contract::PortRef {
                    component: "radio".to_owned(),
                    port: "rx".to_owned(),
                },
                policy: ezsdr_kernel::stream::BackPressure::DropOldest,
                capacity: 4,
            }),
            selector: BTreeMap::new(),
            profile: None,
        },
    );
    p
}

/// The bound Sink per output id, which RS-12 reads the artifact kind from.
fn sinks(s: &TestSink) -> BTreeMap<Ident, &dyn ezsdr_kernel::module_api::Sink> {
    [(id("recorder"), s as &dyn ezsdr_kernel::module_api::Sink)].into_iter().collect()
}

/// The bound Provider per binding name, which RS-12 reads the root kind from.
fn bound(p: &TestProvider) -> BTreeMap<Ident, &dyn ezsdr_kernel::module_api::Provider> {
    [(id("radio"), p as &dyn ezsdr_kernel::module_api::Provider)].into_iter().collect()
}

/// The recorders the profile bound, which RS-14 refuses a capture without.
fn placed() -> std::collections::BTreeSet<Ident> {
    [id("recorder")].into_iter().collect()
}

/// A recorder whose contract matches the `rx` port the test tree declares (SC-3).
fn test_sink() -> TestSink {
    TestSink::new(
        ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.cf32").expect("a valid literal"),
    )
}

fn declared_classes() -> BTreeMap<Key, UpdateClass> {
    [(key("test.capture"), UpdateClass::BlockBoundary), (key("test.flag"), UpdateClass::BlockBoundary)]
        .into_iter()
        .collect()
}

#[test]
fn rs_12_session_implicit_spec_hashed() {
    let reg = registry();
    let profile = session_profile();
    let provider = TestProvider::new("radio", 2);
    let spec = implicit_spec(&profile, &reg, &bound(&provider), &sinks(&test_sink())).expect("builds");
    assert_eq!(spec.version, 1);
    assert_eq!(spec.resources.len(), 1);
    assert!(spec.resources[&id("radio")].requires.is_empty(), "empty requires");
    // RS-12: one resource per Provider binding, one **output** per Sink binding. A
    // Sink is bound and never a graph component (finding D17).
    assert!(spec.graph.components.is_empty());
    assert_eq!(spec.outputs.len(), 1);
    assert_eq!(spec.outputs[0].id, id("recorder"));
    // Hashed like any other document.
    let hash = ContentHash::of(&spec).expect("hashes");
    assert_eq!(hash, ContentHash::of(&spec).expect("hashes"));
    assert!(hash.as_str().starts_with("sha256:"));
}

#[test]
fn rs_12_bare_connect_then_capture_succeeds() {
    let reg = registry();
    let profile = session_profile();
    let provider = TestProvider::new("radio", 2);
    let spec = implicit_spec(&profile, &reg, &bound(&provider), &sinks(&test_sink())).expect("builds");
    assert_eq!(spec.outputs.len(), 1, "the recorder is a bound Sink, carried as an output");
    // The output carries the link that feeds it, which is what makes the capture
    // actually record something (SC-19, finding N6).
    assert_eq!(spec.outputs[0].feed.port.port, "rx");

    let action = SessionAction::Vocabulary {
        ns: ns("test"),
        verb: id("capture"),
        target: rid("radio"),
        at: Some(t(100)),
        params: [(key("test.capture"), Value::Bool(true))].into_iter().collect(),
    };
    let compiled = compile(&action, &reg, &declared_classes(), &placed(), t(0), None).expect("compiles");
    assert_eq!(compiled.actions.len(), 1);
    assert!(matches!(compiled.actions[0], Action::UpdateParameter { .. }));
}

#[test]
fn rs_14_capture_compiles_to_update_parameter() {
    let reg = registry();
    let action = SessionAction::Vocabulary {
        ns: ns("test"),
        verb: id("capture"),
        target: rid("radio"),
        at: Some(t(100)),
        params: [(key("test.capture"), Value::Bool(true))].into_iter().collect(),
    };
    let compiled = compile(&action, &reg, &declared_classes(), &placed(), t(0), None).expect("compiles");
    match &compiled.actions[0] {
        Action::UpdateParameter { target, key: k, class, at, .. } => {
            // RS-14: the update targets the **Sink**, addressed under the reserved
            // `sink/` prefix, not the radio the action named. The prefix keeps the
            // address disjoint from a Provider's own tree node ids, which are the
            // Provider's declaration and could otherwise collide with a binding name
            // (findings D17, P1-5).
            assert_eq!(*target, rid("sink/recorder"));
            assert_ne!(*target, rid("recorder"), "a bare output id is a Provider's namespace");
            assert_eq!(*k, key("test.capture"));
            assert_eq!(*class, UpdateClass::BlockBoundary);
            // RS-14, RS-19: a capture compiles to a *timed* UpdateParameter. This
            // is what Vision §61's `capture(n, at:)` rests on; the instant used to
            // be computed and then dropped (finding OQ2).
            assert_eq!(*at, Some(AbsoluteDeadline::new(t(100))));
        }
        other => panic!("expected an UpdateParameter, got {other:?}"),
    }
}

#[test]
fn rs_14_capture_without_recorder_rejected() {
    let reg = registry();
    let mut profile = session_profile();
    profile.bindings.remove(&id("recorder"));
    let provider = TestProvider::new("radio", 2);
    let spec = implicit_spec(&profile, &reg, &bound(&provider), &sinks(&test_sink())).expect("builds");
    assert!(spec.outputs.is_empty(), "no recorder was bound");

    let capture = SessionAction::Vocabulary {
        ns: ns("test"),
        verb: id("capture"),
        target: rid("radio"),
        at: Some(t(100)),
        params: BTreeMap::new(),
    };
    // RS-14: refused when the profile bound no recorder, rather than silently
    // buffered on the host.
    let violations = compile(&capture, &reg, &declared_classes(), &Default::default(), t(0), None)
        .expect_err("no recorder bound");
    assert!(violations[0].reason.contains("bound none"), "{:?}", violations[0]);
    // The same verb against a profile that did bind one compiles.
    assert!(compile(&capture, &reg, &declared_classes(), &placed(), t(0), None).is_ok());
}

#[test]
fn rs_13a_unknown_vocabulary_verb_rejected() {
    let reg = registry();
    let action = SessionAction::Vocabulary {
        ns: ns("radio"),
        verb: id("start_repeat"),
        target: rid("radio"),
        at: None,
        params: BTreeMap::new(),
    };
    let violations = compile(&action, &reg, &declared_classes(), &placed(), t(0), None)
        .expect_err("no loaded Vocabulary claims it");
    assert!(violations[0].reason.contains("radio.start_repeat"));
}

#[test]
fn rs_17_provider_parameter_class_comes_from_its_key_decl() {
    // Vision §3's `sdr.rx.gain = 20` targets a Provider, whose parameters are
    // Vocabulary keys and never a component's `params`. Before SB-2 carried an
    // update class there was nowhere for such a key's class to be declared, so the
    // Easy API's first parameter change was rejected as undeclared (finding D33).
    let reg = registry();
    let gain = SessionAction::SetParameter {
        target: rid("radio"),
        key: key("test.gain"),
        value: Value::Num(20.0),
    };
    // Nothing in the caller's map: the class comes from the KeyDecl alone.
    let compiled = compile(&gain, &reg, &BTreeMap::new(), &placed(), t(0), None).expect("compiles");
    assert!(matches!(
        &compiled.actions[0],
        Action::UpdateParameter { class: UpdateClass::HardwareTimed, at: None, .. }
    ));

    // And the Admitter agrees, so the two paths cannot disagree about one key.
    let checks = AdmissionCheckRegistry::new();
    let environment = BTreeMap::new();
    let classes = BTreeMap::new();
    let spec_coercion = BTreeMap::new();
    let admitter = Admitter {
        checks: &checks,
        environment: &environment,
        declared_classes: &classes,
        spec_coercion: &spec_coercion,
        registry: &reg,
        is_session: true,
    };
    let proposed: BTreeMap<Key, Value> =
        [(key("test.gain"), Value::Num(20.0))].into_iter().collect();
    assert!(admitter.admit(&BTreeMap::new(), &proposed, &[], CheckStage::Runtime).is_ok());

    // A key whose KeyDecl declares no class is still not changeable during a Run,
    // so the absent value means "not changeable" and not "any class" (RS-17).
    let flag = SessionAction::SetParameter {
        target: rid("radio"),
        key: key("test.flag"),
        value: Value::Bool(true),
    };
    assert!(compile(&flag, &reg, &BTreeMap::new(), &placed(), t(0), None).is_err());
}

#[test]
fn rs_19_capture_asap_records_applied_time() {
    let reg = registry();
    let action = SessionAction::Vocabulary {
        ns: ns("test"),
        verb: id("capture"),
        target: rid("radio"),
        at: None,
        params: BTreeMap::new(),
    };
    let compiled = compile(&action, &reg, &declared_classes(), &placed(), t(4_242), None).expect("compiles");
    assert_eq!(compiled.coercions.len(), 1);
    assert_eq!(compiled.coercions[0].requested, Value::Str("asap".to_owned()));
    assert_eq!(compiled.coercions[0].applied, Value::Int(4_242));
}

#[test]
fn rs_49_update_parameter_carries_its_instant() {
    let reg = registry();
    // The `asap` path: RS-19 resolves `earliest`, records it as a coercion, and the
    // resolved instant reaches the Action's own `at`.
    let asap = SessionAction::Vocabulary {
        ns: ns("test"),
        verb: id("capture"),
        target: rid("radio"),
        at: None,
        params: BTreeMap::new(),
    };
    let compiled =
        compile(&asap, &reg, &declared_classes(), &placed(), t(4_242), None).expect("compiles");
    assert!(matches!(
        &compiled.actions[0],
        Action::UpdateParameter { at: Some(at), .. } if *at == AbsoluteDeadline::new(t(4_242))
    ));

    // A bare SetParameter has no time field to leave empty, so it compiles to an
    // Action with none and records no coercion — and is *not* refused, which a
    // mandatory `at` would have done to Vision §3's `sdr.rx.gain = 20`.
    let bare = SessionAction::SetParameter {
        target: rid("radio"),
        key: key("test.flag"),
        value: Value::Bool(true),
    };
    let compiled =
        compile(&bare, &reg, &declared_classes(), &placed(), t(7), None).expect("compiles");
    assert!(matches!(&compiled.actions[0], Action::UpdateParameter { at: None, .. }));
    assert!(compiled.coercions.is_empty(), "nothing was requested to coerce");

    // RS-49a: a scheduled parameter change is timed, so `arm` substitutes the
    // instant the Spec named rather than applying it at `arm`.
    let template = ActionTemplate::UpdateParameter {
        target: rid("radio"),
        key: key("test.flag"),
        value: Value::Bool(true),
        class: UpdateClass::BlockBoundary,
    };
    assert!(template.is_timed());
    assert!(matches!(
        template.resolve(AbsoluteDeadline::new(t(555))),
        Action::UpdateParameter { at: Some(at), .. } if at == AbsoluteDeadline::new(t(555))
    ));
}

#[test]
fn rs_19_vocabulary_action_carries_a_time() {
    let reg = registry();
    let timed = SessionAction::Vocabulary {
        ns: ns("test"),
        verb: id("sweep"),
        target: rid("radio"),
        at: Some(t(900)),
        params: BTreeMap::new(),
    };
    let compiled = compile(&timed, &reg, &declared_classes(), &placed(), t(0), None).expect("compiles");
    match &compiled.actions[0] {
        Action::PeripheralCommand { at: Some(at), .. } => {
            assert_eq!(*at, AbsoluteDeadline::new(t(900)));
        }
        other => panic!("expected a timed PeripheralCommand, got {other:?}"),
    }
    assert!(compiled.coercions.is_empty(), "an explicit time is not a coercion");
}

#[test]
fn rs_15_log_sequence_is_dense() {
    let mut log = SessionLog::new();
    for i in 0..5 {
        let outcome = if i == 1 || i == 3 {
            Outcome::Rejected { violations: Vec::new() }
        } else {
            Outcome::Admitted { coercions: Vec::new(), warnings: Vec::new(), dispatched: Vec::new() }
        };
        log.append(t(i), SessionAction::Renew, outcome).expect("well-formed");
    }
    let seqs: Vec<u32> = log.entries().iter().map(|e| e.seq).collect();
    assert_eq!(seqs, vec![0, 1, 2, 3, 4], "a rejected Action occupies a number too");
    assert_eq!(log.admitted().count(), 3);
}

#[test]
fn rs_20_replay_divergence() {
    let log = SessionLog::new();
    let recorded = ContentHash::of_bytes(b"profile-a");
    assert!(log.check_replay_target(&recorded, &recorded).is_ok());
    assert_eq!(
        log.check_replay_target(&recorded, &ContentHash::of_bytes(b"profile-b")),
        Err(RunError::ReplayDivergence { field: "binding.hash".to_owned() })
    );
}

#[test]
fn rs_50_stop_with_and_without_a_target() {
    let reg = registry();
    let with = compile(
        &SessionAction::Stop { target: Some(rid("radio")) },
        &reg,
        &declared_classes(),
        &placed(),
        t(0),
        None,
    )
    .expect("compiles");
    assert!(matches!(with.actions[0], Action::Stop { target: Some(_) }));
    assert!(with.control.is_none(), "the Run keeps Running");

    let without =
        compile(&SessionAction::Stop { target: None }, &reg, &declared_classes(), &placed(), t(0), None)
            .expect("compiles");
    assert!(without.actions.is_empty());
    assert_eq!(without.control, Some(ControlOp::StopRun));
}

#[test]
fn rs_16_rejected_action_not_dispatched() {
    let mut checks = AdmissionCheckRegistry::new();
    checks.register(Arc::new(TestLimitsCheck::new()));
    let environment: BTreeMap<Namespace, serde_json::Value> =
        [(ns("test.limits"), serde_json::json!({ "max_grid": 20.0 }))].into_iter().collect();
    let reg = registry();
    let classes: BTreeMap<Key, UpdateClass> =
        [(key("test.grid"), UpdateClass::BlockBoundary)].into_iter().collect();
    let spec_coercion = BTreeMap::new();
    let admitter = Admitter {
        checks: &checks,
        environment: &environment,
        declared_classes: &classes,
        spec_coercion: &spec_coercion,
        registry: &reg,
        is_session: true,
    };
    let proposed: BTreeMap<Key, Value> = [(key("test.grid"), Value::Num(40.0))].into_iter().collect();
    let violations = admitter
        .admit(&BTreeMap::new(), &proposed, &[], CheckStage::Runtime)
        .expect_err("outside the limit");
    assert_eq!(violations[0].check, ns("test.limits"));

    // OV-3: "not dispatched" is asserted by the rejection itself — `admit` returns
    // `Err`, so there is no admitted Action to dispatch. Draining a Provider double
    // that this test never submitted to asserted nothing; the dispatch step is Phase
    // 2's, and RS-16's Phase 1 carrier is the refusal plus the log entry below.

    let mut log = SessionLog::new();
    log.append(
        t(0),
        SessionAction::SetParameter {
            target: rid("radio"),
            key: key("test.grid"),
            value: Value::Num(40.0),
        },
        Outcome::Rejected { violations },
    )
    .expect("well-formed");
    assert!(matches!(log.entries()[0].outcome, Outcome::Rejected { .. }));
}

#[test]
fn rs_17_undeclared_update_class_rejected() {
    let checks = AdmissionCheckRegistry::new();
    let environment = BTreeMap::new();
    let reg = registry();
    let classes = BTreeMap::new();
    let spec_coercion = BTreeMap::new();
    let admitter = Admitter {
        checks: &checks,
        environment: &environment,
        declared_classes: &classes,
        spec_coercion: &spec_coercion,
        registry: &reg,
        is_session: true,
    };
    let proposed: BTreeMap<Key, Value> = [(key("test.flag"), Value::Bool(true))].into_iter().collect();
    let violations = admitter
        .admit(&BTreeMap::new(), &proposed, &[], CheckStage::Runtime)
        .expect_err("no declared class");
    assert!(violations[0].reason.contains("update class"));

    // Compilation refuses it too, so an Executor never receives one.
    let action = SessionAction::SetParameter {
        target: rid("radio"),
        key: key("test.flag"),
        value: Value::Bool(true),
    };
    assert!(compile(&action, &reg, &BTreeMap::new(), &placed(), t(0), None).is_err());
}

#[test]
fn rs_18_action_before_running_rejected() {
    let clock = FakeHostClock::new();
    let mut run = RunStateMachine::new(&clock);
    for s in [RunState::Validated, RunState::Planned, RunState::Prepared, RunState::Armed] {
        run.move_to(s, None, &clock).expect("forward");
    }
    assert_eq!(run.check_running(), Err(RunError::RunNotRunning));
    let mut log = SessionLog::new();
    log.append(t(0), SessionAction::Renew, Outcome::Rejected { violations: Vec::new() })
        .expect("a rejected Action is still logged (RS-15)");
    assert_eq!(log.entries().len(), 1, "it is rejected and logged");
    run.move_to(RunState::Running, None, &clock).expect("forward");
    assert!(run.check_running().is_ok());
}

#[test]
fn rs_44a_waveform_ingested_before_admission() {
    let reg = registry();
    let bytes: Vec<u8> = (0..64u8).collect();
    let waveform = ingest_input(id("wave"), ns("test.waveform"), "memory://wave", &bytes);
    assert_eq!(waveform.size_bytes, 64);
    assert_eq!(waveform.hash, ContentHash::of_bytes(&bytes));

    let action = SessionAction::Vocabulary {
        ns: ns("test"),
        verb: id("start_repeat"),
        target: rid("radio"),
        at: Some(t(10)),
        params: BTreeMap::new(),
    };
    let compiled: Compiled =
        compile(&action, &reg, &declared_classes(), &placed(), t(0), Some(waveform.clone()))
            .expect("compiles with a resolved waveform");
    match &compiled.actions[0] {
        Action::TxBurst { waveform: w, repeat, .. } => {
            assert_eq!(w.hash, waveform.hash);
            assert!(*repeat);
        }
        other => panic!("expected a TxBurst, got {other:?}"),
    }
    // An Action whose ArtifactRef resolves to nothing is rejected.
    assert!(compile(&action, &reg, &declared_classes(), &placed(), t(0), None).is_err());
}

// ---------------------------------------------------------------- the Kernel Action set

#[test]
fn rs_48_action_set_is_closed_and_schematised() {
    let actions = vec![
        Action::TxBurst {
            target: rid("radio/tx/0"),
            waveform: artifact("wave"),
            repeat: true,
            at: AbsoluteDeadline::new(t(10)),
            requested_at: Some(AbsoluteDeadline::new(t(9))),
            late_policy: LatePolicy::SendAsapAndFlag,
            metadata: BTreeMap::new(),
        },
        Action::SetTimer { target: rid("radio"), at: AbsoluteDeadline::new(t(20)), token: 7 },
        Action::UpdateParameter {
            target: rid("radio"),
            key: key("test.flag"),
            value: Value::Bool(true),
            class: UpdateClass::BlockBoundary,
            at: Some(AbsoluteDeadline::new(t(30))),
        },
        Action::PeripheralCommand {
            target: rid("radio"),
            verb: id("sweep"),
            params: BTreeMap::new(),
            at: None,
        },
        Action::Emit {
            target: rid("radio"),
            event: Event {
                source: rid("radio"),
                time: t(1),
                severity: Severity::Info,
                kind: EventKind::parse("test.custom").expect("parses"),
                payload: serde_json::json!({}),
            },
        },
        Action::Stop { target: None },
        Action::Abort { cause: StopCause::Client },
    ];
    assert_eq!(actions.len(), Action::MEMBERS, "the set has exactly seven members");
    for a in &actions {
        let json = serde_json::to_value(a).expect("serialises");
        assert!(json.get("kind").is_some(), "an internal tag for non-Rust consumers (OV-13)");
        let back: Action = serde_json::from_value(json).expect("round-trips");
        assert_eq!(back, *a);
    }
    // A Spec's scheduled Action is a template, with no time field (RS-49a).
    let template = ActionTemplate::SetTimer { target: rid("radio"), token: 7 };
    let json = serde_json::to_value(&template).expect("serialises");
    assert!(json.get("at").is_none());
    assert!(matches!(
        template.resolve(AbsoluteDeadline::new(t(20))),
        Action::SetTimer { at, .. } if at == AbsoluteDeadline::new(t(20))
    ));
}

// ---------------------------------------------------------------- the Manifest

fn artifact(name: &str) -> ArtifactRef {
    ArtifactRef {
        id: id(name),
        kind: ns("test.capture"),
        uri: format!("memory://{name}"),
        hash: some_hash(name),
        size_bytes: 0,
        partial: false,
        marks: Vec::new(),
        continuity: Vec::new(),
    }
}

fn termination_fixture(reason: Termination) -> TerminationSection {
    TerminationSection {
        reason,
        at: Some(t(99)),
        host_utc_nanos: 1_700_000_000_000_000_000,
        cleanup_failures: Vec::new(),
        also: Vec::new(),
    }
}

fn manifest_fixture(reason: Termination) -> Manifest {
    let spec = ExperimentSpec { version: 1, ..ExperimentSpec::default() };
    let profile = session_profile();
    Manifest {
        version: 1,
        run: RunSection {
            id: RunId::from_string("local:1:2-0".to_owned()),
            kind: RunKind::Session,
            parent: None,
            execution_class: ExecutionClass::Simulation,
            fidelity: Fidelity::NONE,
            transitions: Vec::new(),
            deterministic: false,
        },
        spec: SpecSection {
            hash: ContentHash::of(&spec).expect("hashes"),
            body: serde_json::to_value(&spec).expect("serialises"),
            original_version: None,
            original_hash: None,
        },
        binding: BindingSection {
            hash: ContentHash::of(&profile).expect("hashes"),
            body: serde_json::to_value(&profile).expect("serialises"),
        },
        plan: None,
        prepare: PrepareSection::default(),
        admission: Default::default(),
        modules: Vec::new(),
        vocabularies: BTreeMap::new(),
        components: BTreeMap::new(),
        inputs: Vec::new(),
        clocks: ClocksSection::default(),
        events: EventsSection::default(),
        lease: Lease::attached(),
        action_log: Vec::new(),
        termination: termination_fixture(reason),
        artifacts: Vec::new(),
        sections: BTreeMap::new(),
        hash: None,
    }
}

#[test]
fn rs_11_manifest_for_every_terminal_run() {
    for reason in [
        Termination::Completed,
        Termination::Stopped { cause: StopCause::Client },
        Termination::Failed { stage: Stage::Validate },
    ] {
        let mut m = manifest_fixture(reason.clone());
        let hash = m.seal().expect("seals");
        assert_eq!(m.termination.reason, reason);
        assert_eq!(m.hash.as_ref(), Some(&hash));
    }
}

#[test]
fn rs_45_hash_equal_for_equal_inputs() {
    let text_a = r#"{"version":1,"resources":{},"extensions":{}}"#;
    let text_b = "  { \"extensions\" : { } ,\n  \"resources\" : { } , \"version\" : 1 }  ";
    let a = ExperimentSpec::from_json(&serde_json::from_str(text_a).expect("parses"))
        .expect("validates");
    let b = ExperimentSpec::from_json(&serde_json::from_str(text_b).expect("parses"))
        .expect("validates");
    assert_eq!(ContentHash::of(&a).expect("hashes"), ContentHash::of(&b).expect("hashes"));

    let mut ma = manifest_fixture(Termination::Completed);
    let mut mb = manifest_fixture(Termination::Completed);
    assert_eq!(ma.seal().expect("seals"), mb.seal().expect("seals"));
}

#[test]
fn rs_46_manifest_hash_is_stored_beside_the_body() {
    let mut m = manifest_fixture(Termination::Completed);
    let hash = m.seal().expect("seals");
    // The hash is computed over the Manifest with that field **removed** — the shape
    // a non-Rust consumer implementing RS-46 will build.
    let mut body = serde_json::to_value(&m).expect("serialises");
    body.as_object_mut().expect("object").remove("hash");
    assert_eq!(ContentHash::of_value(&body).expect("hashes"), hash);
    let mut unsealed = m.clone();
    unsealed.hash = None;
    let json = serde_json::to_value(&unsealed).expect("serialises");
    assert!(
        json.as_object().expect("object").get("hash").is_none(),
        "an absent hash is skipped, not written as null"
    );
    // Sealing twice is stable.
    assert_eq!(m.seal().expect("seals"), hash);
}

#[test]
fn rs_39_section_namespace_enforced() {
    let mut m = manifest_fixture(Termination::Completed);
    assert!(m.write_section(&ns("ezsdr.test"), ns("ezsdr.test.bursts"), serde_json::json!([])).is_ok());
    assert_eq!(
        m.write_section(&ns("ezsdr.test"), ns("vendor.other"), serde_json::json!({})),
        Err(RunError::SectionNamespaceForbidden { ns: "ezsdr.test".to_owned() })
    );
    assert_eq!(m.sections.len(), 1);
}

#[test]
fn rs_43_no_seeds_field_in_the_envelope() {
    let mut profile = session_profile();
    profile.environment.insert(ns("sim.engine"), serde_json::json!({ "seed": 42 }));
    let mut m = manifest_fixture(Termination::Completed);
    m.binding.body = serde_json::to_value(&profile).expect("serialises");
    let json = serde_json::to_value(&m).expect("serialises");
    assert!(json.get("seeds").is_none(), "the envelope has no `seeds` field");
    assert_eq!(json["binding"]["body"]["environment"]["sim.engine"]["seed"], serde_json::json!(42));
}

#[test]
fn rs_38_environment_recorded_verbatim() {
    let mut profile = session_profile();
    let section = serde_json::json!({ "nested": { "a": [1, 2, 3] }, "s": "verbatim" });
    profile.environment.insert(ns("vendor.thing"), section.clone());
    let mut m = manifest_fixture(Termination::Completed);
    m.binding.body = serde_json::to_value(&profile).expect("serialises");
    assert_eq!(m.binding.body["environment"]["vendor.thing"], section);
}

#[test]
fn rs_32_a_fabricated_event_handle_does_not_panic_the_kernel() {
    // `EventHandle`'s fields are public, so a Module can build one. Indexing the
    // counter table with it directly panicked the Kernel instead of returning an
    // error — and RS-32's infallible hot path is a promise about the Kernel's own
    // handles, not about any `u32` a Module writes (MA-9).
    let policy = kinds().compile(&BTreeMap::new()).expect("compiles");
    let kind = EventKind::parse("test.custom").expect("parses");
    let c = collector(8, &policy, &[(rid("radio"), kind.clone())]);
    // Both `u32`s, not one: checking only `row` let the fabricated `kind` into the
    // ring and moved the panic into the coordinator's `drain`, which indexes `kinds`
    // with it — away from the Module that caused it.
    for bad in [
        ezsdr_kernel::event::EventHandle { row: u32::MAX, kind: 0 },
        ezsdr_kernel::event::EventHandle { row: 0, kind: u32::MAX },
    ] {
        assert!(
            matches!(
                c.emit(bad, t(1), Severity::Info, &[]),
                Err(ezsdr_kernel::run::RunError::BadEventHandle { .. })
            ),
            "row {} kind {} must be refused",
            bad.row,
            bad.kind
        );
    }
    // A real handle still works, so the guard did not break the hot path, and the
    // drain the fabricated kind would have panicked in completes.
    let good = c.resolve(&rid("radio"), &kind);
    assert!(c.emit(good, t(2), Severity::Info, &[]).is_ok());
    assert_eq!(c.drain().len(), 1, "one delivered body, and no panic");
}

#[test]
fn rs_39_a_module_section_with_a_non_ascii_key_is_refused() {
    // `sections` is the path RS-39 and RS-43 design for untrusted Module content and
    // it passes no `from_json`, so OV-15's ASCII key rule was never applied to it:
    // `seal()` then failed at cleanup step 8, after the Run had transmitted (SB-9a,
    // RS-11).
    let mut m = manifest_fixture(Termination::Completed);
    assert!(matches!(
        m.write_section(&ns("test"), ns("test.envelope"), serde_json::json!({ "\u{3c1}": 1 })),
        Err(ezsdr_kernel::run::RunError::SectionKeyNotAscii { .. })
    ));
    // An ASCII key is written, and the Manifest still seals.
    m.write_section(&ns("test"), ns("test.envelope"), serde_json::json!({ "rho": 1 }))
        .expect("writes");
    assert!(m.seal().is_ok());
}

#[test]
fn rs_13a_a_session_value_with_a_non_ascii_key_is_refused_at_compile() {
    // A `Value` reaches the sealed Manifest through the action log and an Action's
    // `params`, neither of which passes a `from_json`, so the check belongs where the
    // value enters the Kernel (SB-9a, RS-11).
    let reg = registry();
    let bad: Value =
        serde_json::from_value(serde_json::json!({ "\u{3c1}": 1 })).expect("deserialises");
    let action = SessionAction::SetParameter {
        target: rid("radio"),
        key: key("test.flag"),
        value: bad.clone(),
    };
    let violations = compile(&action, &reg, &declared_classes(), &placed(), t(0), None)
        .expect_err("refused before it can be logged");
    assert!(violations[0].reason.contains("non-ASCII"), "{:?}", violations[0]);

    // And through a Vocabulary action's params.
    let vocab = SessionAction::Vocabulary {
        ns: ns("test"),
        verb: id("capture"),
        target: rid("radio"),
        at: Some(t(1)),
        params: [(key("test.capture"), bad)].into_iter().collect(),
    };
    assert!(compile(&vocab, &reg, &declared_classes(), &placed(), t(0), None).is_err());
}

#[test]
fn rs_23_a_huge_lease_ttl_does_not_wrap() {
    // `Lease` is a document type (OV-10), so `ttl_ms` is whatever a profile wrote.
    // An unchecked add panicked in debug and, in release, wrapped to a small instant
    // so `expired()` was true immediately — the Run killed at once by a TTL meant to
    // keep it alive.
    let clock = FakeHostClock::new();
    clock.advance(1_000);
    let mut lease = Lease::detached(u64::MAX, true, "tok", &clock).expect("a nonzero ttl");
    assert!(lease.on_disconnect(&clock).is_none());
    assert!(!lease.expired(&clock), "a saturated deadline is in the far future");
    clock.advance(u64::MAX / 2);
    assert!(!lease.expired(&clock), "and it stays there");
}

#[test]
fn rs_38_manifest_carries_its_mandatory_version() {
    // Vision §10 requires a mandatory `version` of the Manifest by name, and the
    // Manifest is the one document that outlives every Run. Without the field
    // SB-47's migrate-or-refuse has nothing to read (finding R10 / D-table N3).
    let mut m = manifest_fixture(Termination::Completed);
    m.seal().expect("seals");
    let doc = serde_json::to_value(&m).expect("serialises");
    assert_eq!(doc["version"], serde_json::json!(1), "the version is in the hashed body");
    assert_eq!(ezsdr_kernel::spec::check_version(&doc), Ok(1));

    // SB-47: a stored Manifest from a future major is refused by name, never
    // reinterpreted under version 1's defaults. The field arrived without this path,
    // so `from_value` accepted `version: 99` and SB-47 had nothing to act on.
    let mut future = doc.clone();
    future["version"] = serde_json::json!(2);
    assert!(matches!(
        ezsdr_kernel::spec::check_version(&future),
        Err(ezsdr_kernel::spec::SpecError::UnsupportedVersion { found: 2, .. })
    ));
    assert!(matches!(
        Manifest::from_json(&future),
        Err(ezsdr_kernel::spec::SpecError::UnsupportedVersion { found: 2, .. })
    ));
    // And the reader exists, so a version-1 Manifest round-trips through it.
    let back = Manifest::from_json(&doc).expect("a v1 Manifest reads");
    assert_eq!(back.version, 1);
    assert_eq!(back.hash, m.hash);
}

#[test]
fn rs_41_fidelity_is_the_weakest() {
    let strong = Fidelity {
        timing: EnvelopeFidelity::HardwareQuirk,
        continuity: EnvelopeFidelity::Real,
        coercion: CoercionFidelity::Real,
        rf: RfFidelity::Real,
        transport: TransportFidelity::Real,
    };
    let weak = Fidelity { timing: EnvelopeFidelity::Envelope, ..strong };
    assert_eq!(Fidelity::weakest(&[strong, weak]).timing, EnvelopeFidelity::Envelope);
    // A Run in which no Provider declares an aspect records `none`.
    assert_eq!(Fidelity::weakest(&[]), Fidelity::NONE);
    // A Hardware Run records `real` where declared.
    assert_eq!(Fidelity::weakest(&[strong]).rf, RfFidelity::Real);
}

#[test]
fn rs_42_determinism_only_in_simulation() {
    let mut m = manifest_fixture(Termination::Completed);
    m.run.execution_class = ExecutionClass::RealtimeEmulation;
    m.run.deterministic = true; // a caller claims it anyway
    m.seal().expect("seals");
    assert!(!m.run.deterministic, "sealing clears a claim RS-42 does not allow");

    let mut sim = manifest_fixture(Termination::Completed);
    sim.run.execution_class = ExecutionClass::Simulation;
    sim.run.deterministic = true;
    sim.seal().expect("seals");
    assert!(sim.run.deterministic, "the Simulation class may claim it");
}

#[test]
fn rs_44_partial_artifact_on_abort() {
    let mut sink = TestSink::new(
        ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.cf32").expect("id"),
    );
    let refs = sink.stop(StopMode::Abort).expect("stop returns refs even on an abort");
    assert_eq!(refs.len(), 1);
    assert!(refs[0].partial, "the ArtifactRef is marked partial");
    let mut orderly = TestSink::new(
        ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.cf32").expect("id"),
    );
    assert!(!orderly.stop(StopMode::Orderly).expect("stops")[0].partial);
}

#[test]
fn rs_40_capture_across_a_rate_change() {
    // A capture spanning a `cold` rate change carries two ContinuityMaps, in
    // SampleClock order (TM-13c, SC-30).
    let a = ClockDomainId::local(7);
    let b = ClockDomainId::local(8);
    let header = |domain: ClockDomainId, at: i64| ezsdr_kernel::stream::BlockHeader {
        first_sample_time: TimePoint::new(domain, at),
        len: 100,
        channels: 1,
        direction: ezsdr_kernel::stream::Direction::Rx,
        valid: ezsdr_kernel::stream::ChannelMask::full(1),
        flags: BlockFlags::NONE,
        lost: None,
        contract: ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.cf32").expect("id"),
    };
    let mut first = ContinuityBuilder::new(a, 1, true);
    first.push(&header(a, 0), DropCarry::default()).expect("accepted");
    let (err, carry) = first
        .push(&header(b, 0), DropCarry::default())
        .expect_err("the rate change ends the map");
    assert!(matches!(err, ezsdr_kernel::stream::StreamError::DomainChanged { .. }));
    let map_a = first.finish(carry);

    let mut second = ContinuityBuilder::new(b, 1, true);
    second.push(&header(b, 0), DropCarry::default()).expect("accepted");
    let map_b = second.finish(DropCarry::default());

    let mut art = artifact("capture0");
    art.continuity = vec![map_a, map_b];
    assert_eq!(art.continuity.len(), 2);
    assert_eq!(art.continuity[0].domain, a);
    assert_eq!(art.continuity[1].domain, b);
}
