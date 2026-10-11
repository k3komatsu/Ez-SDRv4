//! Phase 1 tests for `04-run-and-session.md`. Each name begins with the rule it proves (OV-19).

mod support;

use std::collections::BTreeMap;
use std::sync::Arc;

use ezsdr_kernel::binding::{AdmissionCheckRegistry, BindingProfile, CheckStage, Placements};
use ezsdr_kernel::event::{
    Action, ActionTemplate, CounterRow, Event, EventCollector, EventKind, EventSink, EventSource,
    Severity,
};
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::{ClockDomainId, RunId};
use ezsdr_kernel::manifest::{
    ArtifactRef, BindingSection, ClocksSection, EventsSection, Manifest, PrepareSection, RunKind,
    RunSection, SpecSection, TerminationSection, mark_open_artifacts,
};
use ezsdr_kernel::module_api::{
    CoercionFidelity, EnvelopeFidelity, ExecutionClass, Factories, Fidelity, ModuleRegistry,
    RfFidelity, Sink, StopMode, TransportFidelity, UpdateClass,
};
use ezsdr_kernel::policy::{EventKindRegistry, Policy, Reaction};
use ezsdr_kernel::run::{
    CleanupMode, CleanupStep, Lease, LeaseMode, RunError, RunState, RunStateMachine, Stage,
    StopCause, Termination, run_cleanup,
};
use ezsdr_kernel::session::{Admitter, Outcome, SessionAction, SessionLog, implicit_spec};
use ezsdr_kernel::spec::{ExperimentSpec, Ident, Key, Namespace, Value};
use ezsdr_kernel::stream::{BlockFlags, ContinuityBuilder, DropCarry, LatePolicy};
use ezsdr_kernel::time::{AbsoluteDeadline, TimePoint};
use support::{
    FakeHostClock, RecordingCleanup, TestLimitsCheck, TestProvider, TestSink, id, key, mref, ns,
    some_hash, test_provider_descriptor, test_sink_descriptor, test_vocabulary,
};

fn t(ticks: i64) -> TimePoint {
    TimePoint::new(ClockDomainId::HOST_MONOTONIC, ticks)
}

fn registry() -> ModuleRegistry {
    let mut reg = ModuleRegistry::new();
    reg.register_vocabulary(test_vocabulary()).expect("fresh");
    reg.register(
        test_provider_descriptor(),
        Factories {
            provider: true,
            authority: true,
            ..Factories::default()
        },
    )
    .expect("registers");
    reg.register(
        test_sink_descriptor(),
        Factories {
            sink: true,
            ..Factories::default()
        },
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

// ---------------------------------------------------------------- the state machine

#[test]
fn rs_01_run_id_is_generated_and_unique_within_the_process() {
    let first = RunId::generate();
    let second = RunId::generate();
    assert_ne!(first, second);
    assert!(first.as_str().starts_with("local:"));
    assert_eq!(first.as_str().split(':').count(), 3);
    assert!(first.as_str().rsplit_once('-').is_some());
}

#[test]
fn rs_02_happy_path_transitions() {
    let clock = FakeHostClock::new();
    let mut run = RunStateMachine::new(&clock);
    for state in [
        RunState::Validated {},
        RunState::Planned {},
        RunState::Prepared {},
        RunState::Armed {},
        RunState::Running {},
    ] {
        run.move_to(state, Some(t(0)), &clock).expect("forward");
    }
    run.begin_stopping(CleanupMode::Orderly, Some(t(1)), &clock)
        .expect("into Stopping");
    run.finish(Termination::Completed {}, Some(t(2)), &clock)
        .expect("into CleanedUp");
    let states: Vec<&RunState> = run.transitions().iter().map(|r| &r.state).collect();
    assert_eq!(states.len(), 8, "exactly the eight states, all recorded");
    assert!(matches!(states[0], RunState::Created {}));
    assert!(matches!(states[7], RunState::CleanedUp { .. }));
    // RS-5: each carries its runtime instant and the host UTC time.
    assert!(run.transitions().iter().all(|r| r.host_utc_nanos > 0));
    // There is no going back, and no skipping.
    assert!(matches!(
        run.move_to(RunState::Running {}, None, &clock),
        Err(RunError::IllegalTransition { .. })
    ));
    let mut fresh = RunStateMachine::new(&clock);
    assert!(matches!(
        fresh.move_to(RunState::Running {}, None, &clock),
        Err(RunError::IllegalTransition { .. })
    ));
}

#[test]
fn rs_03_failure_at_each_stage_reaches_cleanup() {
    let clock = FakeHostClock::new();
    for (stage, reached) in [
        (Stage::Validate, vec![]),
        (Stage::Plan, vec![RunState::Validated {}]),
        (
            Stage::Prepare,
            vec![RunState::Validated {}, RunState::Planned {}],
        ),
        (
            Stage::Arm,
            vec![
                RunState::Validated {},
                RunState::Planned {},
                RunState::Prepared {},
            ],
        ),
    ] {
        let mut run = RunStateMachine::new(&clock);
        for s in reached {
            run.move_to(s, None, &clock).expect("forward");
        }
        run.begin_stopping(CleanupMode::Abort, None, &clock)
            .expect("failure aborts");
        run.finish(Termination::Failed { stage, reason: "a test failure".to_owned() }, None, &clock)
            .expect("cleans up");
        assert!(matches!(
            run.state(),
            RunState::CleanedUp { termination: Termination::Failed { stage: s, .. } } if *s == stage
        ));
    }
    // A cancellation before `Running` stops orderly with `Stopped{client}`, from every
    // state it can arrive in.
    let forward = [
        RunState::Validated {},
        RunState::Planned {},
        RunState::Prepared {},
        RunState::Armed {},
    ];
    for reached in 0..=forward.len() {
        let mut run = RunStateMachine::new(&clock);
        for s in &forward[..reached] {
            run.move_to(s.clone(), None, &clock).expect("forward");
        }
        run.begin_stopping(CleanupMode::Orderly, None, &clock)
            .expect("a cancellation stops orderly");
        run.finish(
            Termination::Stopped {
                cause: StopCause::Client {},
            },
            None,
            &clock,
        )
        .expect("cleans up");
        assert!(matches!(
            run.state(),
            RunState::CleanedUp {
                termination: Termination::Stopped {
                    cause: StopCause::Client {}
                }
            }
        ));
    }
}

#[test]
fn rs_27_a_second_declaration_of_one_kind_is_refused() {
    // One Vocabulary cannot replace another's default reaction and severity, and the
    // refusal says what happened rather than "the kind is not registered".
    let mut k = kinds();
    let decl = test_vocabulary()
        .event_kinds
        .into_iter()
        .next()
        .expect("the test Vocabulary declares a kind");
    let kind = decl.kind.to_string();
    assert_eq!(
        k.register(Some(ns("test")), decl),
        Err(RunError::EventKindAlreadyRegistered { kind })
    );
}

#[test]
fn rs_11_cleanup_reaches_the_manifest_step_with_nothing_prepared() {
    // `run_cleanup` cannot observe a failure stage; what differs between stages is
    // how much was prepared, and a `validate` failure leaves nothing. With no
    // fragments, in either mode, step 7 is still reached and is still the last
    // step run. Failing and wedged steps are RS-6's and RS-8a's tests (D70).
    for mode in [CleanupMode::Orderly, CleanupMode::Abort] {
        let cleanup = Arc::new(RecordingCleanup::new());
        let ops: Arc<dyn ezsdr_kernel::run::CleanupOps> = cleanup.clone();
        let outcome = run_cleanup(ops, &[], mode, &|| None);
        assert!(
            outcome.failures.is_empty(),
            "nothing to fail under {mode:?}"
        );
        assert_eq!(
            cleanup.steps().last(),
            Some(&CleanupStep::ReleaseAndWriteManifest),
            "step 7 is reached with nothing prepared under {mode:?}"
        );
        assert!(
            outcome
                .modes
                .iter()
                .any(|(s, _)| *s == CleanupStep::ReleaseAndWriteManifest)
        );
    }
}

#[test]
fn rs_04_structural_mutation_forbidden() {
    let clock = FakeHostClock::new();
    let mut run = RunStateMachine::new(&clock);
    assert!(run.check_structural_mutation().is_ok());
    for s in [
        RunState::Validated {},
        RunState::Planned {},
        RunState::Prepared {},
        RunState::Armed {},
        RunState::Running {},
    ] {
        run.move_to(s, None, &clock).expect("forward");
    }
    assert_eq!(
        run.check_structural_mutation(),
        Err(RunError::StructuralMutationForbidden)
    );
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
    // Exactly the seven steps of RS-6, with the three per-fragment ones repeated.
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
    // Each step is named for what the coordinator does in it; the Manifest records
    // these names in `cleanup_failures` (RS-6).
    assert_eq!(
        serde_json::to_value(ezsdr_kernel::run::CLEANUP_STEPS).unwrap(),
        serde_json::json!([
            "freeze_dispatch",
            "stop_providers",
            "drain_and_stop_consumers",
            "restore_baseline",
            "finalise_artifacts",
            "flush_events",
            "release_and_write_manifest"
        ])
    );
}

#[test]
fn rs_06_cleanup_order_abort() {
    let ops = Arc::new(RecordingCleanup::new());
    let out = run_cleanup(ops.clone(), &release(), CleanupMode::Abort, &|| None);
    assert!(out.modes.iter().all(|(_, m)| *m == CleanupMode::Abort));
    assert!(
        ops.entries()
            .iter()
            .all(|(_, _, m)| *m == CleanupMode::Abort)
    );
}

#[test]
fn rs_06_cleanup_step_failure_continues() {
    let ops = Arc::new(RecordingCleanup::new().failing_at(CleanupStep::DrainAndStopConsumers));
    let out = run_cleanup(ops.clone(), &release(), CleanupMode::Orderly, &|| None);
    // Every step is attempted even when an earlier one failed.
    assert!(ops.steps().contains(&CleanupStep::ReleaseAndWriteManifest));
    assert_eq!(
        out.failures.len(),
        3,
        "one per fragment of the failing step"
    );
    assert!(
        out.failures
            .iter()
            .all(|f| f.step == CleanupStep::DrainAndStopConsumers && !f.timed_out)
    );
}

#[test]
fn rs_07_pending_burst_cancelled_before_tx_stop() {
    let ops = Arc::new(RecordingCleanup::new());
    run_cleanup(ops.clone(), &release(), CleanupMode::Orderly, &|| None);
    let steps = ops.steps();
    let freeze = steps
        .iter()
        .position(|s| *s == CleanupStep::FreezeDispatch)
        .expect("present");
    let stop_providers = steps
        .iter()
        .position(|s| *s == CleanupStep::StopProviders)
        .expect("present");
    assert!(
        freeze < stop_providers,
        "the dispatch freeze precedes stopping TX (RS-7)"
    );
}

#[test]
fn rs_08_cleanup_runs_in_reverse_dependency_order() {
    let ops = Arc::new(RecordingCleanup::new());
    run_cleanup(ops.clone(), &release(), CleanupMode::Orderly, &|| None);
    let stop_providers: Vec<Ident> = ops
        .entries()
        .into_iter()
        .filter(|(s, _, _)| *s == CleanupStep::StopProviders)
        .filter_map(|(_, f, _)| f)
        .collect();
    assert_eq!(
        stop_providers,
        release(),
        "the device armed first is released last"
    );
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
    assert_eq!(out.modes.last().expect("seven steps").1, CleanupMode::Abort);
    // Both causes are recorded in the termination.
    let mut section = termination_fixture(Termination::Stopped {
        cause: StopCause::Client {},
    });
    section.also.push(Termination::Stopped {
        cause: StopCause::Abort {
            message: "policy".to_owned(),
        },
    });
    assert_eq!(section.also.len(), 1);
}

#[test]
fn rs_8a_wedged_cleanup_step_times_out() {
    let ops = Arc::new(RecordingCleanup::new().wedged_at(CleanupStep::DrainAndStopConsumers));
    let out = run_cleanup(ops.clone(), &[id("a")], CleanupMode::Orderly, &|| None);
    assert_eq!(out.failures.len(), 1);
    assert!(
        out.failures[0].timed_out,
        "the step is abandoned and recorded as a timeout"
    );
    // Steps 4 to 7 still run, so a Manifest is written for exactly the failure that
    // most needs recording.
    let steps = ops.steps();
    for later in [
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
    assert_eq!(Lease::default().mode, LeaseMode::Attached {});
    let clock = FakeHostClock::new();
    assert_eq!(
        Lease::detached(0, true, "tok", &clock),
        Err(RunError::LeaseTtlRequired)
    );
    assert!(Lease::detached(5_000, true, "tok", &clock).is_ok());
}

#[test]
fn rs_22_ttl_uses_the_host_clock() {
    let clock = FakeHostClock::new();
    let mut lease = Lease::detached(5_000, true, "tok", &clock).expect("granted");
    lease.on_disconnect(&clock);
    // Virtual time may advance an hour; the Lease does not care.
    clock.advance(1);
    assert!(
        !lease.expired(&clock),
        "one millisecond of host time is not five seconds"
    );
}

#[test]
fn rs_23_attached_stops_on_disconnect() {
    let clock = FakeHostClock::new();
    let mut lease = Lease::attached();
    assert_eq!(
        lease.on_disconnect(&clock),
        Some(StopCause::ClientDisconnect {})
    );
}

#[test]
fn rs_23_detached_expires() {
    let clock = FakeHostClock::new();
    let mut lease = Lease::detached(5_000, true, "tok", &clock).expect("granted");
    assert_eq!(
        lease.on_disconnect(&clock),
        None,
        "a Detached Lease survives the disconnect"
    );
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
    assert_eq!(
        Lease::detached(0, true, "tok", &clock),
        Err(RunError::LeaseTtlRequired)
    );
    let malformed = Lease {
        mode: LeaseMode::Detached {
            ttl_ms: 0,
            renewable: true,
        },
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
    // Delivered, the body keeps its kind, and the Policy aborts on it (KC-31).
    let c = collector(8, &policy, &[(support::node_source("radio"), fatal.clone())]);
    let h = c.resolve(&support::node_source("radio2"), &fatal).expect("the Kernel's own sink resolves any source");
    c.emit(h, t(0), Severity::Fatal, &[]).expect("emits");
    let drained = c.drain(t(9));
    assert!(drained.iter().any(|e| e.kind == fatal), "the delivered body keeps its kind");
    assert_eq!(policy.reaction_for_event(&fatal, Severity::Fatal), Reaction::Abort);
    // Dropped (the ring full), the escalation flag keeps the kind (RS-36).
    let c = collector(1, &policy, &[(support::node_source("radio"), fatal.clone())]);
    let h = c.resolve(&support::node_source("radio2"), &fatal).expect("the Kernel's own sink resolves any source");
    c.emit(h, t(0), Severity::Fatal, &[]).expect("emits");
    c.emit(h, t(1), Severity::Fatal, &[]).expect("emits");
    assert_eq!(
        c.escalation(),
        Some((fatal.clone(), Reaction::Abort)),
        "the Run aborts on a device that is gone, whatever its source name"
    );
}

#[test]
fn rs_36_a_delivered_body_raises_no_escalation_flag() {
    // design-notes §21: a stopping body that is queued, on the hot path or the control
    // path, raises no flag — it is reacted to when delivered, so KC-31's cause is the
    // first stopping event in `delivered`, not the lowest-index flag raised before its
    // body arrived. Only a dropped body raises one.
    let policy = kinds().compile(&BTreeMap::new()).expect("compiles");
    let fatal = EventKind::parse(EventKind::DEVICE_LOST).expect("parses");
    let c = collector(8, &policy, &[(support::node_source("radio"), fatal.clone())]);
    let h = c.resolve(&support::node_source("radio"), &fatal).expect("the Kernel's own sink resolves any source");
    c.emit(h, t(0), Severity::Fatal, &[]).expect("emits");
    assert!(c.escalation().is_none(), "a queued hot-path body raises no flag");
    c.emit_control(ezsdr_kernel::event::Event {
        source: support::node_source("radio"),
        time: t(1),
        severity: Severity::Fatal,
        kind: fatal.clone(),
        payload: serde_json::Value::Null,
    })
    .expect("emits");
    assert!(c.escalation().is_none(), "a control-path body raises no flag");
    assert_eq!(c.drain(t(9)).iter().filter(|e| e.kind == fatal).count(), 2);
}

#[test]
fn rs_29_an_unregistered_fatal_kind_escalates() {
    // RS-29: a kind with no entry in the Run's Policy takes a default from its
    // severity, and RS-36 sets the flag on the hot path.
    let policy = kinds().compile(&BTreeMap::new()).expect("compiles");
    let unknown = EventKind::parse("vendor.crash").expect("parses");
    let c = collector(
        8,
        &policy,
        &[(support::node_source("radio"), EventKind::parse("test.custom").expect("k"))],
    );
    let h = c.resolve(&support::node_source("radio"), &unknown).expect("the Kernel's own sink resolves any source");
    assert_eq!(policy.reaction_for_event(&unknown, Severity::Fatal), Reaction::Abort);
    // A dropped body of it escalates (RS-36; a ring of 8, filled first).
    for _ in 0..9 {
        c.emit(h, t(0), Severity::Fatal, &[]).expect("emits");
    }
    assert_eq!(c.escalation().map(|(_, r)| r), Some(Reaction::Abort));

    // The same kind at `info` does not escalate.
    let c = collector(
        8,
        &policy,
        &[(support::node_source("radio"), EventKind::parse("test.custom").expect("k"))],
    );
    let h = c.resolve(&support::node_source("radio"), &unknown).expect("the Kernel's own sink resolves any source");
    c.emit(h, t(0), Severity::Info, &[]).expect("emits");
    assert!(c.escalation().is_none());
}

#[test]
fn rs_30_marks_every_open_artifact_including_a_partial_one() {
    // MA-26 makes `partial` the ordinary state of an artifact still open on an
    // abort, so RS-30's "every artifact open at that moment" includes it.
    let mut open = vec![
        artifact("a"),
        ArtifactRef {
            partial: true,
            ..artifact("b")
        },
    ];
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
    assert_eq!(
        lease.adopt("wrong", "client-3"),
        Err(RunError::AdoptRejected)
    );

    let mut fixed = Lease::detached(5_000, false, "secret", &clock).expect("granted");
    assert_eq!(fixed.renew(&clock), Err(RunError::LeaseNotRenewable));
    assert!(lease.renew(&clock).is_ok());
}

// ---------------------------------------------------------------- policy and events

fn collector(ring: usize, policy: &Policy, pairs: &[(EventSource, EventKind)]) -> EventCollector {
    let kinds: Vec<EventKind> = policy.table.keys().cloned().collect();
    EventCollector::new(pairs, &kinds, ring, policy)
}

#[test]
fn rs_28_policy_defaults_table() {
    let kinds = kinds();
    let policy = kinds.compile(&BTreeMap::new()).expect("compiles");
    let k = |s: &str| EventKind::parse(s).expect("parses");
    assert_eq!(
        policy.reaction_for(&k(EventKind::EVENTS_DROPPED)),
        Reaction::MarkArtifact
    );
    assert_eq!(
        policy.reaction_for(&k(EventKind::LINK_BACKPRESSURE)),
        Reaction::MarkArtifact
    );
    assert_eq!(
        policy.reaction_for(&k(EventKind::PROCESSOR_DEADLINE_MISS)),
        Reaction::MarkArtifact
    );
    assert_eq!(
        policy.reaction_for(&k(EventKind::DEVICE_LOST)),
        Reaction::Abort
    );
    assert_eq!(
        policy.reaction_for(&k(EventKind::STEP_LIVELOCK)),
        Reaction::Abort
    );
    // The test Module's own kind keeps its declared default.
    assert_eq!(policy.reaction_for(&k("test.custom")), Reaction::Continue);
    // No radio kind is registered by the Kernel.
    for radio in [
        "RX_OVERFLOW",
        "TX_UNDERFLOW",
        "TX_DISCONTINUITY",
        "TIME_ERROR",
        "ALIGNMENT_ERROR",
    ] {
        assert!(
            kinds.get(&k(&format!("ezsdr.{radio}"))).is_none(),
            "{radio} is the Radio Model's, not the Kernel's"
        );
    }
    assert_eq!(EventKind::kernel_kinds().len(), 5);
}

#[test]
fn rs_29_unknown_kind_by_severity() {
    let policy = kinds().compile(&BTreeMap::new()).expect("compiles");
    let unknown = EventKind::parse("vendor.unregistered").expect("parses");
    assert_eq!(
        policy.reaction_for_event(&unknown, Severity::Info),
        Reaction::Continue
    );
    assert_eq!(
        policy.reaction_for_event(&unknown, Severity::Error),
        Reaction::MarkArtifact
    );
    assert_eq!(
        policy.reaction_for_event(&unknown, Severity::Fatal),
        Reaction::Abort
    );
    assert_eq!(Policy::by_severity(Severity::Debug), Reaction::Continue);
    assert_eq!(
        Policy::by_severity(Severity::Warning),
        Reaction::MarkArtifact
    );
}

#[test]
fn rs_26_policy_override_from_spec() {
    let kinds = kinds();
    let custom = EventKind::parse("test.custom").expect("parses");
    let overrides: BTreeMap<EventKind, Reaction> =
        [(custom.clone(), Reaction::Stop)].into_iter().collect();
    let policy = kinds.compile(&overrides).expect("compiles");
    assert_eq!(
        policy.reaction_for(&custom),
        Reaction::Stop,
        "the Spec overrides the default"
    );

    // An unregistered kind in the table is refused (SB-18).
    let bad: BTreeMap<EventKind, Reaction> = [(
        EventKind::parse("test.RX_OVERFLOWS").expect("parses"),
        Reaction::Stop,
    )]
    .into_iter()
    .collect();
    assert!(matches!(
        kinds.compile(&bad),
        Err(RunError::UnknownEventKind { .. })
    ));
}

#[test]
fn rs_33_counters_exact_under_drop() {
    let policy = kinds().compile(&BTreeMap::new()).expect("compiles");
    let source = support::node_source("radio");
    let kind = EventKind::parse("test.custom").expect("parses");
    let c = collector(8, &policy, &[(source.clone(), kind.clone())]);
    let h = c.resolve(&source, &kind).expect("the Kernel's own sink resolves any source");
    for _ in 0..30_000 {
        c.emit(h, t(0), Severity::Info, &[]).expect("emits");
    }
    let counters = c.counters();
    let row = counters
        .iter()
        .find(|r| r.source == source && r.kind == kind)
        .expect("present");
    assert_eq!(row.count, 30_000, "the counter never drops");
    let drained = c.drain(t(9));
    let bodies = drained.iter().filter(|e| e.kind == kind).count();
    assert_eq!(bodies, 8, "the ring held eight");
    let dropped: Vec<&Event> = drained
        .iter()
        .filter(|e| e.kind.as_str() == EventKind::EVENTS_DROPPED)
        .collect();
    assert_eq!(dropped.len(), 1);
    assert_eq!(dropped[0].payload["count"], serde_json::json!(29_992));
    // RS-35: stamped at the drain's instant, with the Kernel as its source.
    assert_eq!(dropped[0].time, t(9));
    assert_eq!(dropped[0].source, EventSource::Kernel);
    // RS-35's invariant: the counter equals the delivered bodies plus the drops.
    assert_eq!(row.count, bodies as u64 + 29_992);
}

#[test]
fn rs_35_dropped_counts_are_deltas() {
    let policy = kinds().compile(&BTreeMap::new()).expect("compiles");
    let source = support::node_source("radio");
    let kind = EventKind::parse("test.custom").expect("parses");
    let c = collector(1, &policy, &[(source.clone(), kind.clone())]);
    let h = c.resolve(&source, &kind).expect("the Kernel's own sink resolves any source");
    for _ in 0..11 {
        c.emit(h, t(0), Severity::Info, &[]).expect("emits");
    }
    let first = c.drain(t(9));
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
    let second = c.drain(t(9));
    assert_eq!(n(&second), 5, "a delta, not a running total");
}

#[test]
fn rs_35_dropped_events_never_enter_the_ring() {
    let policy = kinds().compile(&BTreeMap::new()).expect("compiles");
    let source = support::node_source("radio");
    let kind = EventKind::parse("test.custom").expect("parses");
    let c = collector(4, &policy, &[(source.clone(), kind.clone())]);
    let h = c.resolve(&source, &kind).expect("the Kernel's own sink resolves any source");
    for round in 0..3 {
        for _ in 0..100 {
            c.emit(h, t(round), Severity::Info, &[]).expect("emits");
        }
        let drained = c.drain(t(9));
        let bodies = drained.iter().filter(|e| e.kind == kind).count() as u64;
        let dropped: u64 = drained
            .iter()
            .filter(|e| e.kind.as_str() == EventKind::EVENTS_DROPPED)
            .filter_map(|e| e.payload["count"].as_u64())
            .sum();
        assert_eq!(
            bodies + dropped,
            100,
            "the invariant holds each time (round {round})"
        );
        assert_eq!(bodies, 4);
    }
}

#[test]
fn rs_35_the_dropped_events_meta_event_is_itself_counted() {
    // RS-35's invariant is stated "for every kind", and RS-38 invites a reader to
    // compare `events.counters` with `events.delivered`.
    let policy = kinds().compile(&BTreeMap::new()).expect("compiles");
    let source = support::node_source("radio");
    let kind = EventKind::parse("test.custom").expect("parses");
    let c = collector(1, &policy, &[(source.clone(), kind.clone())]);
    let h = c.resolve(&source, &kind).expect("the Kernel's own sink resolves any source");
    for _ in 0..5 {
        c.emit(h, t(0), Severity::Info, &[]).expect("emits");
    }
    let drained = c.drain(t(9));
    let delivered = drained
        .iter()
        .filter(|e| e.kind.as_str() == EventKind::EVENTS_DROPPED)
        .count() as u64;
    assert_eq!(delivered, 1);
    let counted: u64 = c
        .counters()
        .into_iter()
        .filter(|r| r.kind.as_str() == EventKind::EVENTS_DROPPED)
        .map(|r| r.count)
        .sum();
    assert_eq!(
        counted, delivered,
        "the meta-event is counted like any other body"
    );
}

#[test]
fn rs_33_unforeseen_pair_uses_the_fallback_row() {
    let policy = kinds().compile(&BTreeMap::new()).expect("compiles");
    let known = support::node_source("radio");
    let kind = EventKind::parse("test.custom").expect("parses");
    let c = collector(8, &policy, &[(known.clone(), kind.clone())]);
    let h = c.resolve(&support::node_source("nowhere"), &kind).expect("the Kernel's own sink resolves any source");
    c.emit(h, t(0), Severity::Info, &[]).expect("emits");
    let counters: Vec<CounterRow> = c.counters();
    let fallback = counters.last().expect("the fallback row is last");
    assert_eq!(fallback.count, 1);
    assert_eq!(fallback.kind.as_str(), "ezsdr.unforeseen");
    assert_eq!(
        fallback.source, EventSource::Unforeseen,
        "one fallback row, not one per source"
    );
}

#[test]
fn rs_32_a_view_refuses_a_source_outside_its_root() {
    // MA-5, MA-6 (item 3): each instance's view admits exactly its root — a Provider's
    // node and its descendants, a Sink's output, an Executor's Island — and nothing
    // else, and the refusal is a `RunError` before RS-33's fallback can fold the pair
    // in. The node comparison is on segment boundaries (SB-33), so `radio/rx` covers
    // `radio/rx/0` and not `radio/rxx`.
    let policy = kinds().compile(&BTreeMap::new()).expect("compiles");
    let root = support::node_source("radio/rx");
    let kind = EventKind::parse("test.custom").expect("parses");
    let collector = Arc::new(collector(8, &policy, &[(root.clone(), kind.clone())]));
    let view = collector.view(root.clone());
    assert!(view.resolve(&root, &kind).is_ok(), "its own node");
    assert!(
        view.resolve(&support::node_source("radio/rx/0"), &kind).is_ok(),
        "a node below it"
    );
    for outside in ["radio", "radio2", "radio/rxx", "tx"] {
        let source = support::node_source(outside);
        assert_eq!(
            view.resolve(&source, &kind),
            Err(RunError::EventSourceOutsideRoot {
                root: root.clone(),
                source: source.clone(),
            }),
            "{outside} is not below {}",
            root
        );
    }
}

#[test]
fn rs_32_a_view_refuses_another_instance_s_source() {
    // MA-5, MA-6 (item 3): a Sink's root is exactly its output and an Executor's
    // exactly its Island — not another output's, another Island's, or another role's.
    let policy = kinds().compile(&BTreeMap::new()).expect("compiles");
    let kind = EventKind::parse("test.custom").expect("parses");
    let output = EventSource::Output {
        output: Ident::parse("rec").expect("an output"),
    };
    let island = EventSource::Island {
        island: ezsdr_kernel::id::IslandId::local(0),
    };
    let collector = Arc::new(collector(
        8,
        &policy,
        &[(output.clone(), kind.clone()), (island.clone(), kind.clone())],
    ));
    let sink = collector.view(output.clone());
    assert!(sink.resolve(&output, &kind).is_ok());
    for outside in [
        EventSource::Output {
            output: Ident::parse("rec2").expect("an output"),
        },
        EventSource::Output {
            output: Ident::parse("recording").expect("an output"),
        },
        island.clone(),
    ] {
        assert_eq!(
            sink.resolve(&outside, &kind),
            Err(RunError::EventSourceOutsideRoot {
                root: output.clone(),
                source: outside.clone(),
            })
        );
    }
    let executor = collector.view(island.clone());
    assert!(executor.resolve(&island, &kind).is_ok());
    assert_eq!(
        executor.resolve(
            &EventSource::Island {
                island: ezsdr_kernel::id::IslandId::local(1),
            },
            &kind
        ),
        Err(RunError::EventSourceOutsideRoot {
            root: island.clone(),
            source: EventSource::Island {
                island: ezsdr_kernel::id::IslandId::local(1),
            },
        })
    );
}

#[test]
fn rs_33_a_view_refuses_kernel_and_unforeseen() {
    // RS-32, RS-33 (item 3): a Module never names the Kernel's own source or RS-33's
    // fallback source. That is distinct from the collector folding an unknown
    // **in-root** source into the fallback row: the pair is refused here, before the
    // fallback can see it, and the fallback row is reported as such.
    let policy = kinds().compile(&BTreeMap::new()).expect("compiles");
    let root = support::node_source("radio");
    let kind = EventKind::parse("test.custom").expect("parses");
    let collector = Arc::new(collector(8, &policy, &[(root.clone(), kind.clone())]));
    let view = collector.view(root.clone());
    for source in [EventSource::Kernel, EventSource::Unforeseen] {
        assert_eq!(
            view.resolve(&source, &kind),
            Err(RunError::EventSourceOutsideRoot {
                root: root.clone(),
                source: source.clone(),
            }),
            "{source} is no Module's to name"
        );
    }
    // The Kernel's own sink still resolves its own source (KC-31).
    assert!(collector.resolve(&EventSource::Kernel, &kind).is_ok());
}

#[test]
fn rs_33_an_unforeseen_pair_inside_the_root_uses_the_fallback_row() {
    // RS-33 (item 3): the fallback row is the collector's answer to a pair the plan
    // did not foresee **inside the instance's own tree**; a view admits such a pair,
    // and the planned pair still reaches its own row through the same view.
    let policy = kinds().compile(&BTreeMap::new()).expect("compiles");
    let node = support::node_source("radio");
    let kind = EventKind::parse("test.custom").expect("parses");
    let collector = Arc::new(collector(8, &policy, &[(node.clone(), kind.clone())]));
    let view = collector.view(node.clone());
    // A node below the root the plan did not list, with a registered kind.
    let handle = view
        .resolve(&support::node_source("radio/rx"), &kind)
        .expect("inside the root");
    view.emit(handle, t(0), Severity::Info, &[]).expect("emits");
    // An unregistered kind on the root itself.
    let unknown_kind = EventKind::parse("vendor.other").expect("parses");
    let handle = view.resolve(&node, &unknown_kind).expect("inside the root");
    view.emit(handle, t(1), Severity::Info, &[]).expect("emits");
    let fallback = collector
        .counters()
        .pop()
        .expect("the fallback row is last");
    assert_eq!(fallback.kind.as_str(), "ezsdr.unforeseen");
    assert_eq!(fallback.source, EventSource::Unforeseen);
    assert_eq!(fallback.count, 2, "both unforeseen pairs share the one fallback row");
    let planned = view.resolve(&node, &kind).expect("inside the root");
    view.emit(planned, t(2), Severity::Info, &[]).expect("emits");
    let row = collector
        .counters()
        .into_iter()
        .find(|row| row.source == node && row.kind == kind)
        .expect("the planned row exists");
    assert_eq!(row.count, 1);
    assert_eq!(collector.drain(t(9)).len(), 3, "every body was delivered");
}

#[test]
fn rs_32_a_control_event_from_outside_the_root_is_refused() {
    // MA-5, MA-9a (item 3): `emit_control` is root-bound like `resolve` — a Provider's
    // control event carries its node or a node below it, a Sink's exactly its output —
    // and the refusal comes before the collector counts or delivers it. The ASCII
    // check stays the collector's (item 4).
    let policy = kinds().compile(&BTreeMap::new()).expect("compiles");
    let node = support::node_source("radio");
    let output = EventSource::Output {
        output: Ident::parse("rec").expect("an output"),
    };
    let kind = EventKind::parse("test.custom").expect("parses");
    let collector = Arc::new(collector(
        8,
        &policy,
        &[(node.clone(), kind.clone()), (output.clone(), kind.clone())],
    ));
    let event = |source: &EventSource| Event {
        source: source.clone(),
        time: t(0),
        severity: Severity::Info,
        kind: kind.clone(),
        payload: serde_json::json!({}),
    };
    let provider = collector.view(node.clone());
    assert!(provider.emit_control(event(&node)).is_ok(), "its own root");
    assert!(
        provider.emit_control(event(&support::node_source("radio/rx"))).is_ok(),
        "a node below it"
    );
    for outside in [
        support::node_source("radio2"),
        EventSource::Kernel,
        EventSource::Unforeseen,
    ] {
        assert_eq!(
            provider.emit_control(event(&outside)),
            Err(RunError::EventSourceOutsideRoot {
                root: node.clone(),
                source: outside.clone(),
            }),
            "{outside} is no event source of this Provider"
        );
    }
    let sink = collector.view(output.clone());
    assert!(sink.emit_control(event(&output)).is_ok());
    let other = EventSource::Output {
        output: Ident::parse("rec2").expect("an output"),
    };
    assert_eq!(
        sink.emit_control(event(&other)),
        Err(RunError::EventSourceOutsideRoot {
            root: output.clone(),
            source: other.clone(),
        })
    );
    assert_eq!(
        collector.counters().iter().map(|row| row.count).sum::<u64>(),
        3,
        "only the in-root control events were counted"
    );
    assert_eq!(collector.drain(t(9)).len(), 3, "and only they were delivered");
}

#[test]
fn rs_32_a_control_event_with_a_non_ascii_key_is_refused() {
    // SB-9a (item 3): `emit_control` checks every map key in the payload, nested keys
    // included, before the event is counted or delivered; the refusal names the
    // position only, never the offending key, which OV-15 cannot canonicalise. A
    // non-ASCII **value** is not a key and stays allowed.
    let policy = kinds().compile(&BTreeMap::new()).expect("compiles");
    let source = support::node_source("radio");
    let kind = EventKind::parse("test.custom").expect("parses");
    let collector = Arc::new(collector(8, &policy, &[(source.clone(), kind.clone())]));
    let view = collector.view(source.clone());
    let event = |payload: serde_json::Value| Event {
        source: source.clone(),
        time: t(0),
        severity: Severity::Info,
        kind: kind.clone(),
        payload,
    };
    let error = view
        .emit_control(event(
            serde_json::json!({ "fine": 1, "nested": [{ "\u{3c1}": 2 }] }),
        ))
        .expect_err("a non-ASCII key anywhere in the payload is refused");
    assert_eq!(
        error,
        RunError::PayloadKeyNotAscii {
            position: "payload".to_owned()
        }
    );
    assert!(
        !format!("{error} {error:?}").contains('\u{3c1}'),
        "the refusal never transcribes the offending key: {error}"
    );
    view.emit_control(event(serde_json::json!({ "name": "\u{65e5}\u{672c}\u{8a9e}" })))
        .expect("a non-ASCII value is not a key (SB-9a)");
    assert_eq!(
        collector.counters().iter().map(|row| row.count).sum::<u64>(),
        1,
        "the refused event was not counted"
    );
    assert_eq!(collector.drain(t(9)).len(), 1, "and it was not delivered");
}

#[test]
fn rs_33_duplicate_pairs_share_one_counter_row() {
    let policy = kinds().compile(&BTreeMap::new()).expect("compiles");
    let source = support::node_source("radio");
    let kind = EventKind::parse("test.custom").expect("parses");
    let pairs = [
        (source.clone(), kind.clone()),
        (source.clone(), kind.clone()),
    ];
    let c = collector(8, &policy, &pairs);

    let matching_rows = |rows: &[CounterRow]| {
        rows.iter()
            .filter(|row| row.source == source && row.kind == kind)
            .count()
    };
    assert_eq!(matching_rows(&c.counters()), 1);

    let handle = c.resolve(&source, &kind).expect("the Kernel's own sink resolves any source");
    c.emit(handle, t(0), Severity::Info, &[]).expect("emits");
    let row = c
        .counters()
        .into_iter()
        .find(|row| row.source == source && row.kind == kind)
        .expect("the planned row exists");
    assert_eq!(row.count, 1);
}

#[test]
fn rs_33_planned_kernel_source_shares_its_meta_counter_row() {
    let policy = kinds().compile(&BTreeMap::new()).expect("compiles");
    let source = EventSource::Kernel;
    let kind = EventKind::parse(EventKind::EVENTS_DROPPED).expect("parses");
    let c = collector(8, &policy, &[(source.clone(), kind.clone())]);

    assert_eq!(
        c.counters()
            .iter()
            .filter(|row| row.source == source && row.kind == kind)
            .count(),
        1,
        "the planned and the collector's own kernel row are the same row"
    );
}

#[test]
fn rs_33_drain_uses_the_planned_kernel_source_for_meta_events() {
    let policy = kinds().compile(&BTreeMap::new()).expect("compiles");
    let source = EventSource::Kernel;
    let kind = EventKind::parse("test.custom").expect("parses");
    let c = collector(1, &policy, &[(source.clone(), kind.clone())]);
    let handle = c.resolve(&source, &kind).expect("the Kernel's own sink resolves any source");
    for _ in 0..2 {
        c.emit(handle, t(0), Severity::Info, &[]).expect("emits");
    }

    let dropped_kind = EventKind::parse(EventKind::EVENTS_DROPPED).expect("parses");
    let drained = c.drain(t(9));
    assert_eq!(
        drained
            .iter()
            .filter(|event| event.kind == dropped_kind)
            .count(),
        1
    );
    let dropped_row = c
        .counters()
        .into_iter()
        .find(|row| row.source == source && row.kind == dropped_kind)
        .expect("the planned kernel source has a meta-event counter");
    assert_eq!(dropped_row.count, 1);
}

#[test]
fn rs_36_abort_survives_a_drop() {
    let policy = kinds().compile(&BTreeMap::new()).expect("compiles");
    let source = support::node_source("radio");
    let noise = EventKind::parse("test.custom").expect("parses");
    let fatal = EventKind::parse(EventKind::DEVICE_LOST).expect("parses");
    let c = collector(
        2,
        &policy,
        &[
            (source.clone(), noise.clone()),
            (source.clone(), fatal.clone()),
        ],
    );
    let hn = c.resolve(&source, &noise).expect("the Kernel's own sink resolves any source");
    for _ in 0..10 {
        c.emit(hn, t(0), Severity::Info, &[]).expect("emits");
    }
    assert!(c.escalation().is_none());
    let hf = c.resolve(&source, &fatal).expect("the Kernel's own sink resolves any source");
    c.emit(hf, t(1), Severity::Fatal, &[]).expect("emits");
    let (kind, reaction) = c
        .escalation()
        .expect("the Run aborts even though the body was dropped");
    assert_eq!(kind, fatal);
    assert_eq!(reaction, Reaction::Abort);
    let row = c
        .counters()
        .into_iter()
        .find(|r| r.kind == fatal)
        .expect("present");
    assert_eq!(row.count, 1);
}

#[test]
fn rs_38_counters_include_zero_rows() {
    let policy = kinds().compile(&BTreeMap::new()).expect("compiles");
    let pairs = [
        (
            support::node_source("radio"),
            EventKind::parse("test.custom").expect("parses"),
        ),
        (
            support::node_source("radio"),
            EventKind::parse(EventKind::DEVICE_LOST).expect("parses"),
        ),
    ];
    let c = collector(8, &policy, &pairs);
    let counters = c.counters();
    assert!(
        counters.len() >= 3,
        "every registered row plus the fallback"
    );
    assert!(
        counters.iter().all(|r| r.count == 0),
        "a Run with no events reports zero rows"
    );
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
                module: mref("ezsdr.test.provider"),
                feed: None,
                selector: BTreeMap::new(),
                profile: None,
            },
        )]
        .into_iter()
        .collect(),
        authority: id("radio"),
        placements: Placements::default(),
        environment: BTreeMap::new(),
    };
    // SB-22: a recorder is **bound**, not placed. Its binding carries the `feed`
    // RS-12 turns into the implicit Spec's output — the port it records and the
    // drop-class policy and capacity of the link (SC-19, SC-21).
    p.bindings.insert(
        id("recorder"),
        ezsdr_kernel::binding::Binding {
            module: mref("ezsdr.test.sink"),
            feed: Some(ezsdr_kernel::spec::SinkFeed {
                port: ezsdr_kernel::contract::PortRef {
                    component: id("radio"),
                    port: id("rx"),
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
    [(id("recorder"), s as &dyn ezsdr_kernel::module_api::Sink)]
        .into_iter()
        .collect()
}

/// The bound Provider per binding name, which RS-12 reads the root kind from.
fn bound(p: &TestProvider) -> BTreeMap<Ident, &dyn ezsdr_kernel::module_api::Provider> {
    [(id("radio"), p as &dyn ezsdr_kernel::module_api::Provider)]
        .into_iter()
        .collect()
}

/// A recorder whose contract matches the `rx` port the test tree declares (SC-3).
fn test_sink() -> TestSink {
    TestSink::new(
        ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.cf32")
            .expect("a valid literal"),
    )
}

#[test]
fn rs_12_session_implicit_spec_hashed() {
    let reg = registry();
    let profile = session_profile();
    let provider = TestProvider::new("radio", 2);
    let spec =
        implicit_spec(&profile, &reg, &bound(&provider), &sinks(&test_sink())).expect("builds");
    assert_eq!(spec.version, 1);
    assert_eq!(spec.resources.len(), 1);
    assert!(
        spec.resources[&id("radio")].requires.is_empty(),
        "empty requires"
    );
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
fn rs_15_log_sequence_is_dense() {
    let mut log = SessionLog::new();
    for i in 0..5 {
        let outcome = if i == 1 || i == 3 {
            Outcome::Rejected {
                violations: Vec::new(),
            }
        } else {
            Outcome::Admitted {
                coercions: Vec::new(),
                warnings: Vec::new(),
                dispatched: Vec::new(),
            }
        };
        log.append(t(i), SessionAction::Renew {}, outcome)
            .expect("well-formed");
    }
    let seqs: Vec<u32> = log.entries().iter().map(|e| e.seq).collect();
    assert_eq!(
        seqs,
        vec![0, 1, 2, 3, 4],
        "a rejected Action occupies a number too"
    );
    assert_eq!(log.admitted().count(), 3);
}

#[test]
fn rs_20_replay_divergence() {
    let log = SessionLog::new();
    let recorded = ContentHash::of_bytes(b"profile-a");
    assert!(log.check_replay_target(&recorded, &recorded).is_ok());
    assert_eq!(
        log.check_replay_target(&recorded, &ContentHash::of_bytes(b"profile-b")),
        Err(RunError::ReplayDivergence {
            field: "binding.hash".to_owned()
        })
    );
}

#[test]
fn rs_16_rejected_action_not_dispatched() {
    let mut checks = AdmissionCheckRegistry::new();
    checks.register(Arc::new(TestLimitsCheck::new()));
    let environment: BTreeMap<Namespace, serde_json::Value> =
        [(ns("test.limits"), serde_json::json!({ "max_grid": 20.0 }))]
            .into_iter()
            .collect();
    let reg = registry();
    let spec_coercion = BTreeMap::new();
    let admitter = Admitter {
        checks: &checks,
        environment: &environment,
        spec_coercion: &spec_coercion,
        registry: &reg,
        is_session: true,
    };
    let proposed: BTreeMap<Key, Value> =
        [(key("test.grid"), Value::num(40.0).unwrap())].into_iter().collect();
    let violations = admitter
        .admit(
            &BTreeMap::new(),
            &[(id("radio"), proposed.clone())].into_iter().collect(),
            &[],
            CheckStage::Runtime,
        )
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
            target: support::target("radio"),
            key: key("test.grid"),
            value: Value::num(40.0).unwrap(),
        },
        Outcome::Rejected { violations },
    )
    .expect("well-formed");
    assert!(matches!(log.entries()[0].outcome, Outcome::Rejected { .. }));
}


#[test]
fn rs_18_action_before_running_rejected() {
    let clock = FakeHostClock::new();
    let mut run = RunStateMachine::new(&clock);
    for s in [
        RunState::Validated {},
        RunState::Planned {},
        RunState::Prepared {},
        RunState::Armed {},
    ] {
        run.move_to(s, None, &clock).expect("forward");
    }
    assert_eq!(run.check_running(), Err(RunError::RunNotRunning));
    let mut log = SessionLog::new();
    log.append(
        t(0),
        SessionAction::Renew {},
        Outcome::Rejected {
            violations: Vec::new(),
        },
    )
    .expect("a rejected Action is still logged (RS-15)");
    assert_eq!(log.entries().len(), 1, "it is rejected and logged");
    run.move_to(RunState::Running {}, None, &clock)
        .expect("forward");
    assert!(run.check_running().is_ok());
}

// ---------------------------------------------------------------- the Kernel Action set

#[test]
fn rs_48_action_set_is_closed_and_schematised() {
    let actions = vec![
        Action::TxBurst {
            target: support::target("radio/tx/0"),
            waveform: artifact("wave"),
            repeat: true,
            at: AbsoluteDeadline::new(t(10)),
            requested_at: Some(AbsoluteDeadline::new(t(9))),
            late_policy: LatePolicy::SendAsapAndFlag,
            metadata: BTreeMap::new(),
        },
        Action::SetTimer {
            target: support::target("radio"),
            at: AbsoluteDeadline::new(t(20)),
            token: 7,
        },
        Action::UpdateParameter {
            target: support::target("radio"),
            key: key("test.flag"),
            value: Value::from(true),
            class: UpdateClass::BlockBoundary,
            at: Some(AbsoluteDeadline::new(t(30))),
        },
        Action::Command {
            target: support::target("radio"),
            verb: id("sweep"),
            params: BTreeMap::new(),
            at: None,
        },
        Action::Emit {
            target: support::target("radio"),
            event: Event {
                source: support::node_source("radio"),
                time: t(1),
                severity: Severity::Info,
                kind: EventKind::parse("test.custom").expect("parses"),
                payload: serde_json::json!({}),
            },
        },
        Action::Stop { target: None },
        Action::Abort {
            cause: StopCause::Client {},
        },
    ];
    assert_eq!(
        actions.len(),
        Action::MEMBERS,
        "the set has exactly seven members"
    );
    for a in &actions {
        let json = serde_json::to_value(a).expect("serialises");
        assert!(
            json.get("kind").is_some(),
            "an internal tag for non-Rust consumers (OV-13)"
        );
        let back: Action = serde_json::from_value(json).expect("round-trips");
        assert_eq!(back, *a);
    }
    // A Spec's scheduled Action is a template, with no time field (RS-49a); `arm`
    // resolves it and no public method does (item 3).
    let template = ActionTemplate::SetTimer {
        target: support::target("radio"),
        token: 7,
    };
    let json = serde_json::to_value(&template).expect("serialises");
    assert!(json.get("at").is_none());
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
        undelivered: Vec::new(),
    }
}

fn manifest_fixture(reason: Termination) -> Manifest {
    let spec = ExperimentSpec {
        version: 1,
        ..ExperimentSpec::default()
    };
    let profile = session_profile();
    Manifest {
        version: 1,
        run: RunSection {
            id: RunId::from_string("local:1:2-0".to_owned()),
            kind: RunKind::Session,
            parent: None,
            execution_class: Some(ExecutionClass::Simulation),
            fidelity: Fidelity::NONE,
            transitions: Vec::new(),
            deterministic: false,
            children: Vec::new(),
        },
        policy: None,
        spec: SpecSection {
            hash: ContentHash::of(&spec).expect("hashes"),
            body: serde_json::to_value(&spec).expect("serialises"),
            source: None,
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
        links: Vec::new(),
        events: EventsSection::default(),
        lease: Lease::attached().record(),
        action_log: Vec::new(),
        termination: termination_fixture(reason),
        artifacts: BTreeMap::new(),
        sections: Vec::new(),
        hash: None,
    }
}

#[test]
fn rs_01_manifest_records_the_compiled_policy() {
    let kinds = kinds();
    let custom = EventKind::parse("test.custom").expect("parses");
    let overrides = [(custom.clone(), Reaction::Stop)].into_iter().collect();
    let compiled = kinds
        .compile(&overrides)
        .expect("compiles the resolved table");
    let mut manifest = manifest_fixture(Termination::Completed {});
    manifest.policy = Some(compiled.clone());

    let json = serde_json::to_value(&manifest).expect("serialises");
    let recorded: Policy = serde_json::from_value(json["policy"].clone()).expect("policy is typed");
    assert_eq!(recorded, compiled);
    assert_eq!(recorded.reaction_for(&custom), Reaction::Stop);
}

#[test]
fn rs_11_manifest_for_every_terminal_run() {
    for reason in [
        Termination::Completed {},
        Termination::Stopped {
            cause: StopCause::Client {},
        },
        Termination::Failed {
            stage: Stage::Validate,
            reason: "SB-38: refused".to_owned(),
        },
    ] {
        let mut m = manifest_fixture(reason.clone());
        let hash = m.seal().expect("seals");
        assert_eq!(m.termination.reason, reason);
        assert_eq!(m.hash.as_ref(), Some(&hash));
    }
    // A Run refused at validate never compiled a Policy (SB-18 refuses before
    // `compile` could), so it records none rather than an empty table that would
    // read as "continue for every kind" (RS-1, §65 #39).
    let mut refused = manifest_fixture(Termination::Failed {
        stage: Stage::Validate,
        reason: "SB-18: an unknown kind".to_owned(),
    });
    refused.seal().expect("seals without a Policy");
    let json = serde_json::to_value(&refused).expect("serialises");
    assert!(json["policy"].is_null(), "{}", json["policy"]);
}

#[test]
fn rs_46_manifest_hash_is_stored_beside_the_body() {
    let mut m = manifest_fixture(Termination::Completed {});
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
    let mut m = manifest_fixture(Termination::Completed {});
    assert!(
        m.write_section(
            &ns("ezsdr.test"), support::node_source("radio"),
            ns("ezsdr.test.bursts"),
            serde_json::json!([])
        )
        .is_ok()
    );
    assert_eq!(
        m.write_section(
            &ns("ezsdr.test"), support::node_source("radio"), ns("vendor.other"), serde_json::json!({})),
        Err(RunError::SectionNamespaceForbidden {
            ns: "ezsdr.test".to_owned()
        })
    );
    assert_eq!(m.sections.len(), 1);
}

#[test]
fn rs_43_no_seeds_field_in_the_envelope() {
    let mut profile = session_profile();
    profile
        .environment
        .insert(ns("sim.engine"), serde_json::json!({ "seed": 42 }));
    let mut m = manifest_fixture(Termination::Completed {});
    m.binding.body = serde_json::to_value(&profile).expect("serialises");
    let json = serde_json::to_value(&m).expect("serialises");
    assert!(
        json.get("seeds").is_none(),
        "the envelope has no `seeds` field"
    );
    assert_eq!(
        json["binding"]["body"]["environment"]["sim.engine"]["seed"],
        serde_json::json!(42)
    );
}

#[test]
fn rs_38_environment_recorded_verbatim() {
    let mut profile = session_profile();
    let section = serde_json::json!({ "nested": { "a": [1, 2, 3] }, "s": "verbatim" });
    profile
        .environment
        .insert(ns("vendor.thing"), section.clone());
    let mut m = manifest_fixture(Termination::Completed {});
    m.binding.body = serde_json::to_value(&profile).expect("serialises");
    assert_eq!(m.binding.body["environment"]["vendor.thing"], section);
}

#[test]
fn rs_32_another_view_s_event_handle_is_refused() {
    // `EventHandle`'s fields are private and a sink's `resolve` is its only
    // constructor, so a Module can no longer fabricate one. What it can still do is
    // hold a handle **another scope** minted — another view of the same collector, or
    // the Kernel's own sink — and that is refused, in constant time, rather than
    // counted or panicked on (RS-32, MA-9; pre-freeze item 3).
    let policy = kinds().compile(&BTreeMap::new()).expect("compiles");
    let source = support::node_source("radio");
    let kind = EventKind::parse("test.custom").expect("parses");
    let collector = Arc::new(collector(8, &policy, &[(source.clone(), kind.clone())]));
    let first = collector.view(source.clone());
    let second = collector.view(source.clone());
    let handle = first
        .resolve(&source, &kind)
        .expect("the view's own root");
    // Its own view takes it; another view of the same collector and the Kernel's own
    // sink have other scopes and refuse it.
    assert!(first.emit(handle, t(0), Severity::Info, &[]).is_ok());
    assert_eq!(
        second.emit(handle, t(1), Severity::Info, &[]),
        Err(RunError::ForeignEventHandle)
    );
    assert_eq!(
        collector.emit(handle, t(2), Severity::Info, &[]),
        Err(RunError::ForeignEventHandle)
    );
    // The Kernel's sink resolves any source for itself; a view refuses its handle the
    // same way, and a planned pair still reaches the table through the view that owns
    // it.
    let kernel = collector
        .resolve(&source, &kind)
        .expect("the Kernel's own sink resolves any source");
    assert!(collector.emit(kernel, t(3), Severity::Info, &[]).is_ok());
    assert_eq!(
        first.emit(kernel, t(4), Severity::Info, &[]),
        Err(RunError::ForeignEventHandle)
    );
    let delivered = collector.drain(t(9));
    assert_eq!(delivered.len(), 2, "only the two owned bodies are delivered");
    assert_eq!(collector.counters().iter().map(|r| r.count).sum::<u64>(), 2);
}

#[test]
fn rs_32_another_run_s_event_handle_is_refused() {
    // Scopes are process-unique, not per collector, so a handle minted in another Run
    // — another collector in the same process — is refused by this Run's sink too
    // (RS-32, MA-9; pre-freeze item 3).
    let policy = kinds().compile(&BTreeMap::new()).expect("compiles");
    let source = support::node_source("radio");
    let kind = EventKind::parse("test.custom").expect("parses");
    let one = Arc::new(collector(8, &policy, &[(source.clone(), kind.clone())]));
    let two = Arc::new(collector(8, &policy, &[(source.clone(), kind.clone())]));
    let handle = one
        .resolve(&source, &kind)
        .expect("the Kernel's own sink resolves any source");
    assert_eq!(
        two.emit(handle, t(0), Severity::Info, &[]),
        Err(RunError::ForeignEventHandle)
    );
    assert_eq!(two.counters().iter().map(|r| r.count).sum::<u64>(), 0);
    assert!(one.emit(handle, t(1), Severity::Info, &[]).is_ok());
    assert_eq!(one.drain(t(9)).len(), 1);
}

#[test]
fn rs_39_a_module_section_with_a_non_ascii_key_is_refused() {
    // `sections` is the path RS-39 and RS-43 design for untrusted Module content and
    // it passes no `from_json`, so OV-15's ASCII key rule was never applied to it:
    // `seal()` then failed at cleanup step 7, after the Run had transmitted (SB-9a,
    // RS-11).
    let mut m = manifest_fixture(Termination::Completed {});
    assert!(matches!(
        m.write_section(
            &ns("test"), support::node_source("radio"),
            ns("test.envelope"),
            serde_json::json!({ "\u{3c1}": 1 })
        ),
        Err(ezsdr_kernel::run::RunError::SectionKeyNotAscii { .. })
    ));
    // An ASCII key is written, and the Manifest still seals.
    m.write_section(
            &ns("test"), support::node_source("radio"),
        ns("test.envelope"),
        serde_json::json!({ "rho": 1 }),
    )
    .expect("writes");
    assert!(m.seal().is_ok());
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
    assert!(
        !lease.expired(&clock),
        "a saturated deadline is in the far future"
    );
    clock.advance(u64::MAX / 2);
    assert!(!lease.expired(&clock), "and it stays there");
}

#[test]
fn rs_38_manifest_carries_its_mandatory_version() {
    // Vision §10 requires a mandatory `version` of the Manifest by name, and the
    // Manifest is the one document that outlives every Run. Without the field
    // SB-47's migrate-or-refuse has nothing to read (finding R10 / D-table N3).
    let mut m = manifest_fixture(Termination::Completed {});
    m.seal().expect("seals");
    let doc = serde_json::to_value(&m).expect("serialises");
    assert_eq!(
        doc["version"],
        serde_json::json!(1),
        "the version is in the hashed body"
    );
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
fn sb_04_a_manifest_refuses_a_nested_value() {
    // The Manifest carries `Value`s in its prepare reports, coercions and action log;
    // one level is the type there too (SB-4), so a stored Manifest with a nested value
    // is refused rather than read.
    let mut m = manifest_fixture(Termination::Completed {});
    m.prepare.reports.insert(Ident::parse("radio").unwrap(), ezsdr_kernel::plan::PrepareReport {
        effective: BTreeMap::from([(Key::parse("test.count").unwrap(), Value::from(1))]),
        coercions: Vec::new(),
        warnings: Vec::new(),
    });
    let doc = serde_json::to_value(&m).expect("serialises");
    Manifest::from_json(&doc).expect("a flat value reads");
    let slot = "/prepare/reports/radio/effective/test.count";
    for nested in [serde_json::json!([[1]]), serde_json::json!({ "a": { "b": 1 } })] {
        let mut bad = doc.clone();
        *bad.pointer_mut(slot).expect("the slot") = nested.clone();
        assert!(Manifest::from_json(&bad).is_err(), "{nested}");
    }
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
    let weak = Fidelity {
        timing: EnvelopeFidelity::Envelope,
        ..strong
    };
    assert_eq!(
        Fidelity::weakest(&[strong, weak]).timing,
        EnvelopeFidelity::Envelope
    );
    // A Run in which no Provider declares an aspect records `none`.
    assert_eq!(Fidelity::weakest(&[]), Fidelity::NONE);
    // A Hardware Run records `real` where declared.
    assert_eq!(Fidelity::weakest(&[strong]).rf, RfFidelity::Real);
}

#[test]
fn rs_42_determinism_only_in_simulation() {
    let mut m = manifest_fixture(Termination::Completed {});
    m.run.execution_class = Some(ExecutionClass::RealtimeEmulation);
    m.run.deterministic = true; // a caller claims it anyway
    m.seal().expect("seals");
    assert!(
        !m.run.deterministic,
        "sealing clears a claim RS-42 does not allow"
    );

    let mut sim = manifest_fixture(Termination::Completed {});
    sim.run.execution_class = Some(ExecutionClass::Simulation);
    sim.run.deterministic = true;
    sim.seal().expect("seals");
    assert!(sim.run.deterministic, "the Simulation class may claim it");
}

#[test]
fn rs_44_partial_artifact_on_abort() {
    let mut sink = TestSink::new(
        ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.cf32").expect("id"),
    );
    let refs = sink
        .stop(StopMode::Abort)
        .expect("stop returns refs even on an abort");
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
    first
        .push(&header(a, 0), DropCarry::default())
        .expect("accepted");
    let err = first
        .push(&header(b, 0), DropCarry::default())
        .expect_err("the rate change ends the map");
    assert!(matches!(
        err,
        ezsdr_kernel::stream::StreamError::DomainChanged { .. }
    ));
    let map_a = first.finish(DropCarry::default());

    let mut second = ContinuityBuilder::new(b, 1, true);
    second
        .push(&header(b, 0), DropCarry::default())
        .expect("accepted");
    let map_b = second.finish(DropCarry::default());

    let mut art = artifact("capture0");
    art.continuity = vec![map_a, map_b];
    assert_eq!(art.continuity.len(), 2);
    assert_eq!(art.continuity[0].domain, a);
    assert_eq!(art.continuity[1].domain, b);
}

#[test]
fn rs_14_a_reject_at_plan_session_verb_is_refused_at_registration() {
    // RS-14: the Kernel learns no Vocabulary word, so the late policy is the
    // Vocabulary's own declaration, not a constant in `compile`
    // (`rs_14_the_kernel_supplies_no_vocabulary_value_or_late_policy`, in-crate
    // since pre-freeze item 3).
    // And a Vocabulary declaring `RejectAtPlan` for a Session verb is refused at
    // registration: a Session burst's target is resolved at `compile`, so the verb
    // would name a stage the Action never reaches (SC-27, RS-51).
    let mut v = support::test_vocabulary();
    v.id = ns("late");
    v.prefix = ns("late");
    v.keys.clear();
    v.event_kinds.clear();
    v.verbs = vec![ezsdr_kernel::module_api::VerbDecl {
        verb: id("start_repeat"),
        compiles_to: ezsdr_kernel::module_api::CompileRule::TxBurst {
            repeat: true,
            late_policy: LatePolicy::RejectAtPlan,
        },
    }];
    let mut fresh = ModuleRegistry::new();
    let err = fresh
        .register_vocabulary(v)
        .expect_err("RejectAtPlan is unreachable here");
    assert!(err.message.contains("RS-14"), "{err}");
}

