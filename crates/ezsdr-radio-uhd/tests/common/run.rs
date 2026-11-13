//! The Run a UHD test drives (note 51 §5.4, §6.2): it tells a real-time bound that expired from a failure the code under test caused, at the
//! creation of the Run and at the return of every call that can end it, so that a test cannot fail on a late assertion with the bound unnamed.
//! Shared like `infra.rs`: `crates/ezsdr-acceptance`'s tests include this file with `#[path]` and have `creation_limit` and `infra` at their root.
//!
//! Which calls check: `connect`, `start_spec`, `submit`, `advance_to`, `wait_for`, `run_until_end` and `finish`. They are the calls that settle the
//! Run: it reaches `CleanedUp` only inside one of them (`settle()`, `ending.rs:15-47`), so a read-only method (`state`, `now`, `events`, ...) cannot
//! see an end that no checked call has already judged. A bound that cuts in the background (a Policy end's `stop_early` on the data thread) shows at
//! the next checked call, at `finish`, or in `Drop`.
//!
//! `rig_delay` (`infra.rs`) is the diagnostic delay: after `now`/`start_instant`, `submit` and the three waiting calls return, the thread sleeps
//! `EZSDR_RIG_DELAY_MS` ms when `EZSDR_RIG_DELAY_AT` names the point (`now`, `submit`, `advance`). It is unset on every gate and bench run.
#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

use ezsdr_kernel::coordinator::{Assembly, RunHandle, RunHandleError, connect, start_spec_run};
use ezsdr_kernel::event::{Event, EventKind, EventSource};
use ezsdr_kernel::id::ResourceId;
use ezsdr_kernel::manifest::{Manifest, TerminationSection};
use ezsdr_kernel::run::{CleanupFailure, CleanupStep, Lease, RunState, Stage, StopCause, Termination};
use ezsdr_kernel::session::{InitialRequests, LogEntry, SessionAction};
use ezsdr_kernel::spec::{Ident, Key, SpecError, Value};
use ezsdr_kernel::time::{SampleClockRecord, TimePoint};

use super::{creation_limit, infra, rig_delay};

/// An end that is not a real-time bound and that the test expects.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Expect {
    /// Refused at `validate`, `prepare` or `arm`.
    Refused,
    /// Ended by the Policy table on this event kind.
    Policy(&'static str),
    /// Every end that is not a bound; for a named bench loop that records how its Run ended. A bound still fails it.
    Any,
}

/// What a test declares about its Run. A bound is never declared away except one: the test whose subject is the Provider's join.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Declared {
    pub end: Option<Expect>,
    /// The test is about the Provider threads' join: a thread that does not join is its subject. No other bound is exempt.
    pub join_is_subject: bool,
}

/// How a Run's end reads against a declaration.
#[derive(PartialEq, Eq, Debug)]
pub enum Judged {
    Fine,
    /// A real-time bound expired (KC-12a, KC-21a, RS-8a or join): an `EZSDR-INFRA` marker and one panic.
    Limit(&'static str),
    /// An end the test did not expect: a failure the code under test caused, a panic with no marker.
    Unexpected,
}

/// The pure decision: `rejected` is the Provider's `(thread, reason)` notes. A bound comes first, whatever the test expects, in the primary reason
/// and in every later end the Kernel kept in `also` (KC-32: a bound that cut after a Policy end is recorded there, and no declaration covers it);
/// the declaration exempts only the join, only for the subject test.
pub fn judge_end(section: &TerminationSection, rejected: &[(&str, &str)], declared: Declared) -> Judged {
    let termination = &section.reason;
    let reasons: Vec<&str> = rejected.iter().map(|(_, reason)| *reason).collect();
    let timed_out = section.cleanup_failures.iter().any(|step| step.timed_out);
    let mut end_reasons = std::iter::once(termination).chain(&section.also).map(|end| match end {
        Termination::Failed { reason, .. } => reason.as_str(),
        _ => "",
    });
    let limit = end_reasons.find_map(|reason| creation_limit(reason, false, &[])).or_else(|| creation_limit("", timed_out, &reasons)).filter(|limit| !(declared.join_is_subject && *limit == "join"));
    if let Some(limit) = limit {
        return Judged::Limit(limit);
    }
    let expected = match (termination, declared.end) {
        (_, Some(Expect::Any)) => true,
        (Termination::Completed {}, _) | (Termination::Stopped { cause: StopCause::Client {} }, _) => true,
        (Termination::Failed { stage: Stage::Validate | Stage::Prepare | Stage::Arm, .. }, Some(Expect::Refused)) => true,
        (Termination::Stopped { cause: StopCause::Policy { event } }, Some(Expect::Policy(kind))) => *event == EventKind::parse(kind).unwrap(),
        _ => false,
    };
    if expected { Judged::Fine } else { Judged::Unexpected }
}

thread_local! {
    static CAPTURED: RefCell<Option<(Vec<&'static str>, Vec<String>)>> = const { RefCell::new(None) };
}

/// Runs `body`, and returns with its result the `EZSDR-INFRA` markers `judge` would have written on this thread meanwhile, but only those of the
/// bounds in `expected` (`"join"`, `"KC-21a"`, ...): any other marker is written as usual, so a test that provokes one bound on purpose does not
/// hide another that cuts by accident. A marker in the output of a passing test is what the runners read as a bound that expired (note 51 §6.3),
/// so such a test captures its own. A test that expects no bound does not call this at all.
pub fn capture_markers<T>(expected: &[&'static str], body: impl FnOnce() -> T) -> (T, Vec<String>) {
    CAPTURED.with(|captured| *captured.borrow_mut() = Some((expected.to_vec(), Vec::new())));
    let result = catch_unwind(AssertUnwindSafe(body));
    let (_, lines) = CAPTURED.with(|captured| captured.borrow_mut().take()).unwrap_or_default();
    match result {
        Ok(result) => (result, lines),
        Err(payload) => resume_unwind(payload),
    }
}

/// Whether a capture for the bounds in `expected` keeps the marker of `limit`.
fn captures(expected: &[&str], limit: &str) -> bool {
    expected.contains(&limit)
}

fn mark(limit: &str, message: &str) {
    let kept = CAPTURED.with(|captured| match captured.borrow_mut().as_mut() {
        Some((expected, lines)) if captures(expected, limit) => {
            lines.push(message.to_owned());
            true
        }
        _ => false,
    });
    if !kept {
        infra(message);
    }
}

/// What `judge` does with a verdict: `(write the marker, panic)`. While the thread is already panicking it only writes the marker, because a
/// panic in a `Drop` during unwinding aborts the process.
fn effects(judged: &Judged, panicking: bool) -> (bool, bool) {
    (matches!(judged, Judged::Limit(_)), !panicking && *judged != Judged::Fine)
}

pub struct Run {
    core: Option<Core>,
    declared: Declared,
}

#[allow(clippy::large_enum_variant)]
enum Core {
    Live(RunHandle),
    Ended(Box<Ended>),
}

/// A Run that a checked call found ended: `RunHandle::finish` consumes the handle, so its Manifest and the values no accessor can read after it are
/// kept here, and the Run answers from them.
struct Ended {
    manifest: Manifest,
    now: TimePoint,
    t0: Option<TimePoint>,
    clocks: Vec<SampleClockRecord>,
    effective: BTreeMap<Ident, BTreeMap<Key, Value>>,
}

fn rejected_notes(manifest: &Manifest) -> Vec<(String, String)> {
    let usrp = EventSource::Node { node: ResourceId::parse("usrp").unwrap() };
    let rows = manifest.section(&usrp, "ezsdr.radio.uhd.rejected").and_then(serde_json::Value::as_array).into_iter().flatten();
    rows.filter_map(|row| Some((row["thread"].as_str().unwrap_or_default().to_owned(), row["reason"].as_str()?.to_owned()))).collect()
}

impl Run {
    pub fn connect(profile: &serde_json::Value, initial: &InitialRequests, assembly: Assembly, lease: Lease, declared: Declared) -> Result<Run, SpecError> {
        let mut run = Run { core: Some(Core::Live(connect(profile, initial, assembly, lease)?)), declared };
        run.check("the Session");
        Ok(run)
    }

    pub fn start_spec(spec: &serde_json::Value, profile: &serde_json::Value, assembly: Assembly, declared: Declared) -> Result<Run, SpecError> {
        let mut run = Run { core: Some(Core::Live(start_spec_run(spec, profile, assembly)?)), declared };
        run.check("the Spec's Run");
        Ok(run)
    }

    pub fn expecting(mut self, end: Expect) -> Run {
        self.declared.end = Some(end);
        self
    }

    pub fn join_is_subject(mut self) -> Run {
        self.declared.join_is_subject = true;
        self
    }

    fn live(&mut self) -> Result<&mut RunHandle, RunHandleError> {
        match self.core.as_mut().expect("a Run has a core until finish") {
            Core::Live(handle) => Ok(handle),
            Core::Ended(ended) => Err(RunHandleError::Ended { termination: ended.manifest.termination.reason.clone() }),
        }
    }

    /// After a call that may have ended the Run: take its Manifest and judge how it ended. The Run keeps the Manifest first, so a test that catches
    /// the panic can still use it, and `Drop` has nothing left to judge.
    fn check(&mut self, what: &str) {
        let Some(Core::Live(handle)) = self.core.as_ref() else { return };
        if !matches!(handle.state(), RunState::CleanedUp { .. }) {
            return;
        }
        let Some(Core::Live(handle)) = self.core.take() else { unreachable!() };
        let (now, t0, clocks, effective) = (handle.now(), handle.start_instant(), handle.sample_clocks(), handle.effective());
        let manifest = handle.finish();
        self.core = Some(Core::Ended(Box::new(Ended { manifest, now, t0, clocks, effective })));
        let Some(Core::Ended(ended)) = self.core.as_ref() else { unreachable!() };
        self.judge(&ended.manifest, what);
    }

    /// A bound is an `EZSDR-INFRA` marker and one panic; an end the test did not expect is a panic ([`effects`]).
    fn judge(&self, manifest: &Manifest, what: &str) {
        let notes = rejected_notes(manifest);
        let notes: Vec<(&str, &str)> = notes.iter().map(|(thread, reason)| (thread.as_str(), reason.as_str())).collect();
        let judged = judge_end(&manifest.termination, &notes, self.declared);
        let message = match &judged {
            Judged::Fine => return,
            Judged::Limit(limit) => format!(
                "{what} met {limit}: {:?} (also {:?}), cleanup failures {:?}, rejected {notes:?}",
                manifest.termination.reason, manifest.termination.also, manifest.termination.cleanup_failures
            ),
            Judged::Unexpected => format!("{what} ended unexpectedly: {:?}", manifest.termination.reason),
        };
        let (marker, panics) = effects(&judged, std::thread::panicking());
        if let (true, Judged::Limit(limit)) = (marker, &judged) {
            mark(limit, &message);
        }
        if panics {
            panic!("{message}");
        }
    }

    pub fn state(&self) -> RunState {
        match self.core.as_ref().expect("a Run has a core until finish") {
            Core::Live(handle) => handle.state(),
            Core::Ended(ended) => RunState::CleanedUp { termination: ended.manifest.termination.reason.clone() },
        }
    }

    pub fn now(&self) -> TimePoint {
        let now = match self.core.as_ref().expect("a Run has a core until finish") {
            Core::Live(handle) => handle.now(),
            Core::Ended(ended) => ended.now,
        };
        rig_delay("now");
        now
    }

    pub fn start_instant(&self) -> Option<TimePoint> {
        let t0 = match self.core.as_ref().expect("a Run has a core until finish") {
            Core::Live(handle) => handle.start_instant(),
            Core::Ended(ended) => ended.t0,
        };
        rig_delay("now");
        t0
    }

    pub fn sample_clocks(&self) -> Vec<SampleClockRecord> {
        match self.core.as_ref().expect("a Run has a core until finish") {
            Core::Live(handle) => handle.sample_clocks(),
            Core::Ended(ended) => ended.clocks.clone(),
        }
    }

    pub fn effective(&self) -> BTreeMap<Ident, BTreeMap<Key, Value>> {
        match self.core.as_ref().expect("a Run has a core until finish") {
            Core::Live(handle) => handle.effective(),
            Core::Ended(ended) => ended.effective.clone(),
        }
    }

    pub fn events(&self, from: usize) -> Vec<Event> {
        match self.core.as_ref().expect("a Run has a core until finish") {
            Core::Live(handle) => handle.events(from),
            Core::Ended(ended) => ended.manifest.events.delivered.iter().skip(from).cloned().collect(),
        }
    }

    pub fn submit(&mut self, action: SessionAction, waveform: Option<&[u8]>) -> Result<LogEntry, RunHandleError> {
        let result = self.live()?.submit(action, waveform);
        self.check("a submit");
        rig_delay("submit");
        result
    }

    pub fn advance_to(&mut self, target: TimePoint) -> Result<(), RunHandleError> {
        let result = self.live()?.advance_to(target);
        self.check("an advance");
        rig_delay("advance");
        result
    }

    pub fn wait_for(&mut self, kinds: &[EventKind], from: usize, horizon: TimePoint) -> Result<Option<usize>, RunHandleError> {
        let result = self.live()?.wait_for(kinds, from, horizon);
        self.check("a wait");
        rig_delay("advance");
        result
    }

    pub fn run_until_end(&mut self, horizon: TimePoint) -> Result<(), RunHandleError> {
        let result = self.live()?.run_until_end(horizon);
        self.check("a wait");
        rig_delay("advance");
        result
    }

    pub fn finish(mut self) -> Manifest {
        match self.core.take().expect("a Run has a core until finish") {
            Core::Live(handle) => {
                let manifest = handle.finish();
                self.judge(&manifest, "the Run");
                manifest
            }
            Core::Ended(ended) => ended.manifest,
        }
    }
}

/// A Run that was not finished is finished and judged like `finish`, so a test that forgets `finish()` is still checked. While unwinding this
/// only writes the marker of a bound (the test's own failure is the panic that counts), and a panic of `finish()` itself is caught: the
/// Kernel's own `Drop` runs the same cleanup for a device-paced Run (KC-46c), so a broken Kernel invariant could panic there too, which this
/// cannot stop; what `catch_unwind` covers is `finish`'s own `expect` after a cleanup that ran.
impl Drop for Run {
    fn drop(&mut self) {
        let Some(Core::Live(handle)) = self.core.take() else { return };
        let manifest = if std::thread::panicking() { catch_unwind(AssertUnwindSafe(|| handle.finish())).ok() } else { Some(handle.finish()) };
        if let Some(manifest) = manifest {
            self.judge(&manifest, "the dropped Run");
        }
    }
}

fn failed(stage: Stage, reason: &str) -> Termination {
    Termination::Failed { stage, reason: reason.to_owned() }
}

fn policy(kind: &str) -> Termination {
    Termination::Stopped { cause: StopCause::Policy { event: EventKind::parse(kind).unwrap() } }
}

fn section(reason: &Termination, also: &[Termination], timed_out: bool) -> TerminationSection {
    let cleanup_failures = if timed_out { vec![CleanupFailure { step: CleanupStep::StopProviders, fragment: None, reason: "late".to_owned(), timed_out }] } else { Vec::new() };
    TerminationSection { reason: reason.clone(), at: None, host_utc_nanos: 0, cleanup_failures, also: also.to_vec(), undelivered: Vec::new() }
}

const JOIN: &str = "did not join within 1 s; left detached";
const NONE: Declared = Declared { end: None, join_is_subject: false };

#[test]
fn run_judge_end_a_completed_or_client_stopped_run_needs_no_declaration() {
    assert_eq!(judge_end(&section(&Termination::Completed {}, &[], false), &[], NONE), Judged::Fine);
    assert_eq!(judge_end(&section(&Termination::Stopped { cause: StopCause::Client {} }, &[], false), &[], NONE), Judged::Fine);
    assert_eq!(judge_end(&section(&Termination::Stopped { cause: StopCause::Abort { message: "x".to_owned() } }, &[], false), &[], NONE), Judged::Unexpected);
    assert_eq!(judge_end(&section(&failed(Stage::Run, "ezsdr.DEVICE_LOST: gone"), &[], false), &[], NONE), Judged::Unexpected);
}

#[test]
fn run_judge_end_refused_is_validate_prepare_arm_only() {
    let refused = Declared { end: Some(Expect::Refused), ..NONE };
    for stage in [Stage::Validate, Stage::Prepare, Stage::Arm] {
        assert_eq!(judge_end(&section(&failed(stage, "UR-12: refused"), &[], false), &[], refused), Judged::Fine);
    }
    assert_eq!(judge_end(&section(&failed(Stage::Run, "ezsdr.DEVICE_LOST: gone"), &[], false), &[], refused), Judged::Unexpected);
    assert_eq!(judge_end(&section(&failed(Stage::Arm, "UR-12: refused"), &[], false), &[], NONE), Judged::Unexpected, "an undeclared refusal is unexpected");
}

#[test]
fn run_judge_end_policy_names_its_event() {
    let lost = Declared { end: Some(Expect::Policy("ezsdr.DEVICE_LOST")), ..NONE };
    assert_eq!(judge_end(&section(&policy("ezsdr.DEVICE_LOST"), &[], false), &[], lost), Judged::Fine);
    assert_eq!(judge_end(&section(&policy("radio.CLOCK_LOST"), &[], false), &[], lost), Judged::Unexpected);
    assert_eq!(judge_end(&section(&policy("ezsdr.DEVICE_LOST"), &[], false), &[], NONE), Judged::Unexpected);
    assert_eq!(judge_end(&section(&Termination::Completed {}, &[], false), &[], lost), Judged::Fine, "a declaration adds an end, it does not remove the ordinary ones");
}

#[test]
fn run_judge_end_a_bound_is_named_before_any_end() {
    assert_eq!(judge_end(&section(&failed(Stage::Run, "KC-21a: a did not finish its Actions within 5000 ms"), &[], false), &[], NONE), Judged::Limit("KC-21a"));
    assert_eq!(
        judge_end(&section(&failed(Stage::Prepare, "KC-12a: prepare of radio did not return within 5000 ms"), &[], false), &[], Declared { end: Some(Expect::Refused), ..NONE }),
        Judged::Limit("KC-12a")
    );
    assert_eq!(judge_end(&section(&Termination::Completed {}, &[], true), &[], NONE), Judged::Limit("RS-8a"));
    assert_eq!(judge_end(&section(&policy("ezsdr.DEVICE_LOST"), &[], false), &[("uhd-tx-s0", JOIN)], Declared { end: Some(Expect::Policy("ezsdr.DEVICE_LOST")), ..NONE }), Judged::Limit("join"));
}

#[test]
fn run_judge_end_any_is_not_a_bound_pass() {
    let any = Declared { end: Some(Expect::Any), ..NONE };
    assert_eq!(judge_end(&section(&failed(Stage::Run, "ezsdr.DEVICE_LOST: gone"), &[], false), &[], any), Judged::Fine);
    assert_eq!(judge_end(&section(&policy("ezsdr.DEVICE_LOST"), &[], false), &[], any), Judged::Fine);
    assert_eq!(judge_end(&section(&failed(Stage::Run, "KC-21a: late"), &[], false), &[], any), Judged::Limit("KC-21a"));
    assert_eq!(judge_end(&section(&Termination::Completed {}, &[], true), &[], any), Judged::Limit("RS-8a"));
    assert_eq!(judge_end(&section(&Termination::Completed {}, &[], false), &[("uhd-rx-s0", JOIN)], any), Judged::Limit("join"));
}

#[test]
fn run_judge_end_a_join_is_a_bound_unless_it_is_the_subject() {
    let notes = [("uhd-rx-s0", JOIN)];
    assert_eq!(judge_end(&section(&Termination::Stopped { cause: StopCause::Client {} }, &[], false), &notes, NONE), Judged::Limit("join"), "a join before T0 is not exempt either");
    let subject = Declared { join_is_subject: true, ..NONE };
    assert_eq!(judge_end(&section(&Termination::Stopped { cause: StopCause::Client {} }, &[], false), &notes, subject), Judged::Fine);
    assert_eq!(judge_end(&section(&Termination::Stopped { cause: StopCause::Client {} }, &[], false), &[("x", "UR-16: something else")], NONE), Judged::Fine, "only the join note is a bound");
}

#[test]
fn run_judge_end_a_subject_exempts_only_join() {
    let subject = Declared { join_is_subject: true, ..NONE };
    assert_eq!(judge_end(&section(&Termination::Completed {}, &[], true), &[("uhd-tx-s0", JOIN)], subject), Judged::Limit("RS-8a"));
    assert_eq!(judge_end(&section(&failed(Stage::Run, "KC-21a: late"), &[], false), &[("uhd-tx-s0", JOIN)], subject), Judged::Limit("KC-21a"));
    assert_eq!(judge_end(&section(&failed(Stage::Prepare, "KC-12a: late"), &[], false), &[("uhd-tx-s0", JOIN)], subject), Judged::Limit("KC-12a"));
}

#[test]
fn run_effects_a_bound_is_marked_and_panics_unless_the_thread_is_unwinding() {
    assert_eq!(effects(&Judged::Fine, false), (false, false));
    assert_eq!(effects(&Judged::Limit("join"), false), (true, true));
    assert_eq!(effects(&Judged::Unexpected, false), (false, true));
    assert_eq!(effects(&Judged::Fine, true), (false, false));
    assert_eq!(effects(&Judged::Limit("join"), true), (true, false), "unwinding: the marker, no second panic");
    assert_eq!(effects(&Judged::Unexpected, true), (false, false));
}

#[test]
fn run_judge_end_a_bound_in_also_is_named_whatever_the_primary_end_declares() {
    let lost = Declared { end: Some(Expect::Policy("ezsdr.DEVICE_LOST")), ..NONE };
    let primary = policy("ezsdr.DEVICE_LOST");
    assert_eq!(judge_end(&section(&primary, &[], false), &[], lost), Judged::Fine);
    for (reason, limit) in [("KC-21a: late", "KC-21a"), ("KC-12a: late", "KC-12a")] {
        assert_eq!(judge_end(&section(&primary, &[failed(Stage::Run, reason)], false), &[], lost), Judged::Limit(limit), "{reason}");
    }
    let later = [policy("radio.CLOCK_LOST"), failed(Stage::Run, "KC-21a: late")];
    assert_eq!(judge_end(&section(&primary, &later, false), &[], lost), Judged::Limit("KC-21a"), "not only the first entry of `also`");
    assert_eq!(judge_end(&section(&Termination::Completed {}, &[failed(Stage::Run, "KC-21a: late")], false), &[], NONE), Judged::Limit("KC-21a"));
    let any = Declared { end: Some(Expect::Any), ..NONE };
    assert_eq!(judge_end(&section(&primary, &[failed(Stage::Run, "KC-21a: late")], false), &[], any), Judged::Limit("KC-21a"), "`Any` does not cover it either");
    assert_eq!(judge_end(&section(&primary, &[failed(Stage::Run, "ezsdr.DEVICE_LOST: gone")], false), &[], lost), Judged::Fine, "a later failure that is no bound is the declared end's company");
}

#[test]
fn run_judge_end_a_subject_does_not_exempt_a_bound_in_also() {
    let primary = policy("ezsdr.DEVICE_LOST");
    let late = [failed(Stage::Run, "KC-21a: late")];
    let subject = Declared { end: Some(Expect::Policy("ezsdr.DEVICE_LOST")), join_is_subject: true };
    assert_eq!(judge_end(&section(&primary, &late, false), &[("uhd-tx-s0", JOIN)], subject), Judged::Limit("KC-21a"));
}

#[test]
fn run_judge_end_a_timed_out_cleanup_and_a_join_stay_bounds_beside_a_declared_policy_end() {
    let lost = Declared { end: Some(Expect::Policy("ezsdr.DEVICE_LOST")), ..NONE };
    let primary = policy("ezsdr.DEVICE_LOST");
    assert_eq!(judge_end(&section(&primary, &[policy("radio.CLOCK_LOST")], true), &[], lost), Judged::Limit("RS-8a"));
    assert_eq!(judge_end(&section(&primary, &[policy("radio.CLOCK_LOST")], false), &[("uhd-tx-s0", JOIN)], lost), Judged::Limit("join"));
}

#[test]
fn run_capture_keeps_only_the_markers_of_the_bounds_it_expects() {
    assert!(captures(&["join"], "join"));
    assert!(!captures(&["join"], "KC-21a"), "an unexpected bound's marker is written, not captured");
    assert!(!captures(&["join"], "RS-8a"));
    assert!(captures(&["join", "RS-8a"], "RS-8a"));
    assert!(!captures(&[], "join"), "a capture that expects nothing keeps nothing");
}
