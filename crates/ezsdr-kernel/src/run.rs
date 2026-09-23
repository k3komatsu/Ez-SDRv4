//! The Run state machine, cleanup and the Lease —
//! `04-run-and-session.md` RS-1…RS-11, RS-21…RS-25 (Vision §50, §53).

use std::fmt;
use std::sync::Arc;
use std::sync::mpsc;

use serde::{Deserialize, Serialize};

use crate::event::EventKind;
use crate::module_api::ModuleError;
use crate::spec::Ident;
use crate::time::TimePoint;

/// Whether a stop delivers the declared tail (RS-9).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
pub enum CleanupMode {
    /// The Provider delivers its declared tail (RS-9).
    Orderly,
    /// It does not (RS-9).
    Abort,
}

/// Which pipeline stage a Run was in when it failed (RS-3).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Schema and semantic validation (SB-38).
    Validate,
    /// Plan construction (SB-39).
    Plan,
    /// Fragment preparation (SB-41).
    Prepare,
    /// Arming (SB-43).
    Arm,
    /// Running.
    Run,
}

/// Why a Run stopped (RS-3, RS-23, RS-26).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum StopCause {
    /// The client asked (RS-3).
    Client {},
    /// An `Attached` Lease's client went away (RS-23).
    ClientDisconnect {},
    /// A `Detached` Lease's TTL ran out (RS-23).
    LeaseExpiry {},
    /// The Policy table reacted to an event (RS-26).
    Policy {
        /// Which kind triggered it.
        kind: EventKind,
    },
    /// Something aborted the Run (RS-9).
    Abort {
        /// Why, uninterpreted.
        cause: String,
    },
}

/// How a Run ended. There is no `Failed` **state**: a failure is a termination
/// reached through `Stopping`, because a Run that failed and was not cleaned up is
/// a transmitter nobody turned off.
///
/// Rule: RS-2, RS-3, decision R1.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Termination {
    /// It ran to completion.
    Completed {},
    /// It was stopped (RS-3, RS-23).
    Stopped {
        /// Why.
        cause: StopCause,
    },
    /// A stage failed (RS-3).
    Failed {
        /// Which one.
        stage: Stage,
    },
}

/// The eight states of a Run (RS-2).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RunState {
    /// Nothing has happened yet.
    Created {},
    /// `validate()` passed (SB-38).
    Validated {},
    /// `plan()` passed (SB-39).
    Planned {},
    /// Every fragment prepared (SB-41).
    Prepared {},
    /// Everything armed (SB-43).
    Armed {},
    /// Live.
    Running {},
    /// Cleanup is under way (RS-6).
    Stopping {
        /// Whether the tail is delivered (RS-9).
        mode: CleanupMode,
    },
    /// Cleanup finished; a Manifest was written (RS-11).
    CleanedUp {
        /// How it ended (RS-3).
        termination: Termination,
    },
}

impl RunState {
    fn rank(&self) -> u8 {
        match self {
            RunState::Created {} => 0,
            RunState::Validated {} => 1,
            RunState::Planned {} => 2,
            RunState::Prepared {} => 3,
            RunState::Armed {} => 4,
            RunState::Running {} => 5,
            RunState::Stopping { .. } => 6,
            RunState::CleanedUp { .. } => 7,
        }
    }

    /// The legal transitions: forward through the first six, from any of those into
    /// `Stopping`, and from `Stopping` into `CleanedUp` (RS-2).
    pub fn can_move_to(&self, to: &RunState) -> bool {
        match (self, to) {
            (RunState::Stopping { .. }, RunState::CleanedUp { .. }) => true,
            (RunState::Stopping { .. } | RunState::CleanedUp { .. }, _) => false,
            (_, RunState::Stopping { .. }) => true,
            (_, RunState::CleanedUp { .. }) => false,
            (a, b) => b.rank() == a.rank() + 1,
        }
    }
}

/// Why a Run operation was refused (RS-4, RS-18, RS-20, RS-21, RS-24, RS-32, RS-39).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RunError {
    /// The graph's structure does not change while a Run is `Running`. A structural
    /// change is `stop`, re-plan, `start` (RS-4).
    StructuralMutationForbidden,
    /// An Action arrived while the Run was not `Running` (RS-18).
    RunNotRunning,
    /// A `Detached` Lease was requested without a TTL (RS-21).
    LeaseTtlRequired,
    /// `Renew` on a non-renewable Lease (RS-24).
    LeaseNotRenewable,
    /// `Adopt` with the wrong token. Adoption by run id alone would let any client
    /// seize a live transmitter (RS-24).
    AdoptRejected,
    /// The kind is not registered (RS-27, SB-18).
    UnknownEventKind {
        /// The kind named.
        kind: String,
    },
    /// The kind is already registered, which RS-27 refuses rather than overwriting
    /// a Vocabulary's declaration with another's (RS-27, SB-18).
    EventKindAlreadyRegistered {
        /// The kind named.
        kind: String,
    },
    /// A Module's section carries a key the canonicaliser cannot order, so the
    /// Manifest could not be written after the Run had already transmitted (RS-39,
    /// RS-11, SB-9a, OV-15).
    SectionKeyNotAscii {
        /// The offending key.
        key: String,
    },
    /// An `EventHandle` names a counter row the table does not have. Its fields are
    /// public, so a Module can fabricate one; the Kernel returns this rather than
    /// panicking (RS-32, MA-9).
    BadEventHandle {
        /// The row named.
        row: u32,
    },
    /// A Module wrote under another Module's namespace (RS-39).
    SectionNamespaceForbidden {
        /// The namespace it may write under.
        ns: String,
    },
    /// Re-application against a different BindingProfile was attempted (RS-20).
    ReplayDivergence {
        /// Which field differed.
        field: String,
    },
    /// A hot-path payload exceeded [`crate::event::HOT_PAYLOAD_BYTES`] (RS-32).
    PayloadTooLarge,
    /// A transition RS-2 does not allow. Not in `04-run-and-session.md` §4's error
    /// list, which names no error for RS-2's own rule; recorded in
    /// `00-overview.md` §11.
    IllegalTransition {
        /// Where the Run was.
        from: RunState,
        /// Where it was asked to go.
        to: RunState,
    },
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunError::StructuralMutationForbidden => {
                f.write_str("the graph's structure does not change while a Run is Running")
            }
            RunError::RunNotRunning => f.write_str("the Run is not Running"),
            RunError::LeaseTtlRequired => f.write_str("a Detached Lease needs a TTL"),
            RunError::LeaseNotRenewable => f.write_str("this Lease is not renewable"),
            RunError::AdoptRejected => f.write_str("the adoption token does not match"),
            RunError::UnknownEventKind { kind } => {
                write!(f, "event kind {kind:?} is not registered")
            }
            RunError::EventKindAlreadyRegistered { kind } => {
                write!(f, "event kind {kind:?} is already registered")
            }
            RunError::SectionKeyNotAscii { key } => {
                write!(f, "section key {key:?} is not ASCII, which OV-15 cannot canonicalise")
            }
            RunError::BadEventHandle { row } => {
                write!(f, "event handle names row {row}, which the counter table does not have")
            }
            RunError::SectionNamespaceForbidden { ns } => {
                write!(f, "a Module may write only under {ns:?}")
            }
            RunError::ReplayDivergence { field } => write!(f, "the recorded log diverges from the target profile on {field}"),
            RunError::PayloadTooLarge => f.write_str("a hot-path payload is over 32 bytes"),
            RunError::IllegalTransition { from, to } => {
                write!(f, "illegal transition from {from:?} to {to:?}")
            }
        }
    }
}

impl std::error::Error for RunError {}

// ---------------------------------------------------------------- the host clock

/// The wall clock a Lease runs on, injected so that tests can advance it.
///
/// Run time would be wrong in both directions: a paused simulation would keep a
/// detached transmitter alive forever, and a simulation running faster than wall
/// clock would expire it early. A Lease is about a client's absence, which is a
/// wall-clock fact.
///
/// Rule: RS-22, decision R4.
pub trait HostClock: Send + Sync {
    /// Milliseconds on the host monotonic clock (RS-22).
    fn monotonic_millis(&self) -> u64;
    /// The corresponding UTC time, in nanoseconds since 1970, which is recorded
    /// beside every transition (RS-5, RS-22).
    fn utc_nanos(&self) -> i64;
}

/// The real host clock (RS-22).
pub struct SystemHostClock {
    base: std::time::Instant,
}

impl Default for SystemHostClock {
    fn default() -> Self {
        SystemHostClock::new()
    }
}

impl SystemHostClock {
    /// A clock anchored at construction (RS-22).
    pub fn new() -> SystemHostClock {
        SystemHostClock { base: std::time::Instant::now(),
        }
    }
}

impl HostClock for SystemHostClock {
    fn monotonic_millis(&self) -> u64 {
        self.base.elapsed().as_millis() as u64
    }

    fn utc_nanos(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0)
    }
}

// ---------------------------------------------------------------- the Lease

/// Whether a Run survives its client's disconnection (RS-21).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LeaseMode {
    /// The Run ends when the client disconnects. The default (RS-21, RS-23).
    Attached {},
    /// The Run survives the disconnect until the TTL expires (RS-21, RS-23).
    Detached {
        /// How long, on the host monotonic clock (RS-22).
        ttl_ms: u64,
        /// Whether `Renew` is allowed (RS-24).
        renewable: bool,
    },
}

/// Who holds a Run, and for how long (RS-21…RS-25).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Lease {
    /// Attached or Detached (RS-21).
    pub mode: LeaseMode,
    /// Issued at grant for a Detached Lease; only `Adopt { token }` reclaims it
    /// (RS-24, decision R5).
    pub token: Option<String>,
    /// Who holds it now, uninterpreted by the Kernel (RS-24).
    pub holder: Option<String>,
    /// When it expires, on the host monotonic clock (RS-22).
    pub expires_at_host: Option<u64>,
    /// How many times it has been adopted (RS-24).
    pub adoptions: u32,
    /// Whether it has been released (RS-6, step 8).
    pub released: bool,
}

impl Default for Lease {
    fn default() -> Self {
        Lease::attached()
    }
}

impl Lease {
    /// The default: the Run ends when its client disconnects (RS-21).
    pub fn attached() -> Lease {
        Lease {
            mode: LeaseMode::Attached {},
            token: None,
            holder: None,
            expires_at_host: None,
            adoptions: 0,
            released: false,
        }
    }

    /// A Detached Lease with its token. A `Detached` Lease without a TTL is
    /// `LeaseTtlRequired` (RS-21, RS-24).
    pub fn detached(
        ttl_ms: u64,
        renewable: bool,
        token: impl Into<String>,
        clock: &dyn HostClock,
    ) -> Result<Lease, RunError> {
        if ttl_ms == 0 {
            return Err(RunError::LeaseTtlRequired);
        }
        let _ = clock;
        Ok(Lease {
            mode: LeaseMode::Detached { ttl_ms, renewable },
            token: Some(token.into()),
            holder: None,
            // Not armed here: RS-22 makes a Lease "about a client's **absence**",
            // and `on_disconnect` is the only thing that establishes one. Arming at
            // the grant expires a Run whose client never left.
            expires_at_host: None,
            adoptions: 0,
            released: false,
        })
    }

    /// RS-21's shape rule, for a Lease that did not come through
    /// [`Lease::detached`] — one read back from a document, or built field by
    /// field. A `Detached` Lease without a TTL is `LeaseTtlRequired`.
    ///
    /// ponytail: Phase 1 has no Run-admission seam to call this from, because the
    /// coordinator is Phase 2. It is public and tested so that the seam has
    /// something to call. Raised as finding D20.
    ///
    /// Rule: RS-21.
    pub fn validate(&self) -> Result<(), RunError> {
        match self.mode {
            LeaseMode::Detached { ttl_ms: 0, .. } => Err(RunError::LeaseTtlRequired),
            _ => Ok(()),
        }
    }

    /// Starts the TTL running, as a disconnect does (RS-23).
    pub fn on_disconnect(&mut self, clock: &dyn HostClock) -> Option<StopCause> {
        match self.mode {
            LeaseMode::Attached {} => Some(StopCause::ClientDisconnect {}),
            LeaseMode::Detached { ttl_ms, .. } => {
                self.holder = None;
                // `Lease` is a document type (OV-10), so `ttl_ms` is whatever a
                // profile wrote. An unchecked add panicked in debug and, in release,
                // wrapped to a small instant so `expired()` was true at once and the
                // Run was killed immediately — the opposite of a long TTL's intent.
                self.expires_at_host = Some(clock.monotonic_millis().saturating_add(ttl_ms));
                None
            }
        }
    }

    /// Whether the TTL has run out (RS-23).
    pub fn expired(&self, clock: &dyn HostClock) -> bool {
        matches!(self.mode, LeaseMode::Detached { .. })
            && self.expires_at_host.is_some_and(|at| clock.monotonic_millis() >= at)
    }

    /// Reclaims a Detached Lease. A wrong token is `AdoptRejected` (RS-24).
    pub fn adopt(&mut self, token: &str, holder: impl Into<String>) -> Result<(), RunError> {
        match &self.token {
            Some(t) if t == token => {
                self.holder = Some(holder.into());
                self.adoptions += 1;
                self.expires_at_host = None;
                Ok(())
            }
            _ => Err(RunError::AdoptRejected),
        }
    }

    /// Extends the TTL. A `Renew` on a non-renewable Lease is `LeaseNotRenewable`
    /// (RS-24).
    pub fn renew(&mut self, clock: &dyn HostClock) -> Result<(), RunError> {
        match self.mode {
            LeaseMode::Detached { ttl_ms, renewable: true,
            } => {
                // Only an armed Lease has a deadline to push out; renewing an
                // attached one is a no-op rather than a new expiry (RS-22, RS-24).
                if self.expires_at_host.is_some() {
                    self.expires_at_host = Some(clock.monotonic_millis().saturating_add(ttl_ms));
                }
                Ok(())
            }
            _ => Err(RunError::LeaseNotRenewable),
        }
    }
}

// ---------------------------------------------------------------- cleanup

/// The nine ordered steps of RS-6, not the unordered list of Vision §53.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
pub enum CleanupStep {
    /// 0. Clean up child Runs, each by this same algorithm (RS-6, RS-25).
    Children,
    /// 1. Freeze dispatch and cancel every pending burst and timer. This precedes
    ///    stopping TX: a burst still queued when TX stops will reopen it on a
    ///    device that honours timed commands (RS-7).
    FreezeDispatch,
    /// 2. Stop TX on every Provider, in reverse dependency order (RS-8).
    StopTx,
    /// 3. Stop RX on every Provider, in reverse dependency order (RS-8).
    StopRx,
    /// 4. Cancel outstanding Peripheral operations (RS-6).
    CancelPeripherals,
    /// 5. Restore baseline state, in reverse dependency order (RS-8).
    RestoreBaseline,
    /// 6. Finalise artifacts, marking as partial anything still open (RS-6, RS-44).
    FinaliseArtifacts,
    /// 7. Flush the event path and collect the counters (RS-6, RS-35).
    FlushEvents,
    /// 8. Release the Lease and write the Manifest (RS-6, RS-11).
    ReleaseAndWriteManifest,
}

/// The nine steps in order (RS-6).
pub const CLEANUP_STEPS: [CleanupStep; 9] = [
    CleanupStep::Children,
    CleanupStep::FreezeDispatch,
    CleanupStep::StopTx,
    CleanupStep::StopRx,
    CleanupStep::CancelPeripherals,
    CleanupStep::RestoreBaseline,
    CleanupStep::FinaliseArtifacts,
    CleanupStep::FlushEvents,
    CleanupStep::ReleaseAndWriteManifest,
];

impl CleanupStep {
    /// The three steps that run per fragment, in reverse dependency order: the
    /// device that was armed first is released last (RS-8).
    pub fn is_per_fragment(self) -> bool {
        matches!(self, CleanupStep::StopTx | CleanupStep::StopRx | CleanupStep::RestoreBaseline)
    }
}

/// A cleanup step that failed or timed out. Every step is attempted even when an
/// earlier one failed; each failure is recorded and the sequence continues (RS-6).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CleanupFailure {
    /// Which step (RS-6).
    pub step: CleanupStep,
    /// Which fragment, for a per-fragment step (RS-8).
    pub fragment: Option<Ident>,
    /// What happened (RS-6).
    pub reason: String,
    /// Whether it was abandoned on its deadline rather than returning an error
    /// (RS-8a).
    pub timed_out: bool,
}

/// What the coordinator asks of the Run's Modules during cleanup.
///
/// `&self` rather than `&mut self` because RS-8a abandons a step that exceeds its
/// deadline, and a wedged step must not hold the coordinator's only handle;
/// implementers use interior mutability.
///
/// Rule: RS-6, RS-8a.
pub trait CleanupOps: Send + Sync {
    /// Performs one step, on one fragment for the three per-fragment steps (RS-6).
    fn perform(
        &self,
        step: CleanupStep,
        fragment: Option<&Ident>,
        mode: CleanupMode,
    ) -> Result<(), ModuleError>;

    /// The deadline for a step, in milliseconds: the Provider's declared one where
    /// it has one and a Kernel default otherwise (RS-8a).
    fn deadline_millis(&self, _step: CleanupStep, _fragment: Option<&Ident>) -> u64 {
        DEFAULT_CLEANUP_DEADLINE_MS
    }
}

/// The Kernel's default per-step cleanup deadline, used where a Provider declares
/// none (RS-8a).
pub const DEFAULT_CLEANUP_DEADLINE_MS: u64 = 5_000;

/// What cleanup recorded (RS-6, RS-8a, RS-10).
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CleanupOutcome {
    /// Every step and fragment that failed or timed out (RS-6, RS-8a).
    pub failures: Vec<CleanupFailure>,
    /// The mode each step actually ran under; an abort raised mid-cleanup
    /// escalates the remaining steps (RS-10).
    pub modes: Vec<(CleanupStep, CleanupMode)>,
}

/// Runs RS-6's nine steps.
///
/// `reverse_order` is the inverse of SB-39's arm order, and is used for the three
/// per-fragment steps. Every step is attempted even when an earlier one failed.
/// Each step runs under a deadline; one that exceeds it is abandoned, recorded as a
/// timeout, and the sequence continues — without that, a single wedged peripheral
/// stops cleanup before step 8, so no Manifest is written for exactly the failure
/// that most needs recording.
///
/// ponytail: an abandoned step's thread is left running, because an in-process call
/// cannot be cancelled. MA-8's ceiling names the same limit and the upgrade is the
/// same: a worker thread with a join timeout, which is what this is.
///
/// Rule: RS-6, RS-7, RS-8, RS-8a, RS-9, RS-10.
pub fn run_cleanup(
    ops: Arc<dyn CleanupOps>,
    reverse_order: &[Ident],
    mode: CleanupMode,
    escalate: &dyn Fn() -> Option<CleanupMode>,
) -> CleanupOutcome {
    let mut out = CleanupOutcome::default();
    let mut mode = mode;
    for step in CLEANUP_STEPS {
        // RS-10: an abort raised while an orderly stop is in progress escalates the
        // remaining steps.
        if let Some(escalated) = escalate() {
            mode = escalated;
        }
        out.modes.push((step, mode));
        let targets: Vec<Option<Ident>> = if step.is_per_fragment() {
            reverse_order.iter().cloned().map(Some).collect()
        } else {
            vec![None]
        };
        for fragment in targets {
            let deadline = ops.deadline_millis(step, fragment.as_ref());
            match perform_with_deadline(&ops, step, fragment.clone(), mode, deadline) {
                Ok(()) => {}
                Err(Some(e)) => out.failures.push(CleanupFailure {
                    step,
                    fragment,
                    reason: e.message,
                    timed_out: false,
                }),
                Err(None) => out.failures.push(CleanupFailure {
                    step,
                    fragment,
                    reason: format!("RS-8a: abandoned after {deadline} ms"),
                    timed_out: true,
                }),
            }
        }
    }
    out
}

/// `Err(None)` means the step exceeded its deadline and was abandoned (RS-8a).
fn perform_with_deadline(
    ops: &Arc<dyn CleanupOps>,
    step: CleanupStep,
    fragment: Option<Ident>,
    mode: CleanupMode,
    deadline_ms: u64,
) -> Result<(), Option<ModuleError>> {
    let (tx, rx) = mpsc::channel();
    let ops = Arc::clone(ops);
    std::thread::spawn(move || {
        let r = ops.perform(step, fragment.as_ref(), mode);
        let _ = tx.send(r);
    });
    match rx.recv_timeout(std::time::Duration::from_millis(deadline_ms)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(Some(e)),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(None),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(Some(ModuleError::rejected(
            "RS-6: the cleanup step panicked".to_owned(),
        ))),
    }
}

// ---------------------------------------------------------------- the state machine

/// One recorded transition, with its runtime instant and the host UTC time (RS-5).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TransitionRecord {
    /// The state entered (RS-2).
    pub state: RunState,
    /// When, in Run time; absent before an Authority exists (RS-5).
    pub at: Option<TimePoint>,
    /// When, in UTC nanoseconds (RS-5).
    pub host_utc_nanos: i64,
}

/// The Run's state and its recorded transitions. Every execution is a Run,
/// including an interactive Session (RS-1, Vision §50).
pub struct RunStateMachine {
    state: RunState,
    transitions: Vec<TransitionRecord>,
}

impl RunStateMachine {
    /// A Run in `Created`, with that transition recorded (RS-2, RS-5).
    pub fn new(clock: &dyn HostClock) -> RunStateMachine {
        RunStateMachine {
            state: RunState::Created {},
            transitions: vec![TransitionRecord {
                state: RunState::Created {},
                at: None,
                host_utc_nanos: clock.utc_nanos(),
            }],
        }
    }

    /// Where the Run is (RS-2).
    pub fn state(&self) -> &RunState {
        &self.state
    }

    /// Every transition, in order; the sequence appears in the Manifest (RS-5).
    pub fn transitions(&self) -> &[TransitionRecord] {
        &self.transitions
    }

    /// Moves to `to`, recording it. An illegal transition is refused (RS-2, RS-5).
    pub fn move_to(
        &mut self,
        to: RunState,
        at: Option<TimePoint>,
        clock: &dyn HostClock,
    ) -> Result<(), RunError> {
        if !self.state.can_move_to(&to) {
            return Err(RunError::IllegalTransition { from: self.state.clone(), to,
            });
        }
        self.transitions.push(TransitionRecord {
            state: to.clone(),
            at,
            host_utc_nanos: clock.utc_nanos(),
        });
        self.state = to;
        Ok(())
    }

    /// A failure at any stage moves the Run to `Stopping { abort }` with
    /// `Failed { stage }`; a cancellation before `Running` moves it to
    /// `Stopping { orderly }` (RS-3).
    pub fn begin_stopping(
        &mut self,
        mode: CleanupMode,
        at: Option<TimePoint>,
        clock: &dyn HostClock,
    ) -> Result<(), RunError> {
        self.move_to(RunState::Stopping { mode }, at, clock)
    }

    /// Ends the Run. A Manifest is written for every Run that reaches `CleanedUp`,
    /// including one that failed at `validate` (RS-11).
    pub fn finish(
        &mut self,
        termination: Termination,
        at: Option<TimePoint>,
        clock: &dyn HostClock,
    ) -> Result<(), RunError> {
        self.move_to(RunState::CleanedUp { termination }, at, clock)
    }

    /// Refuses a structural change while the Run is `Running` (RS-4).
    pub fn check_structural_mutation(&self) -> Result<(), RunError> {
        if matches!(self.state, RunState::Running {}) {
            return Err(RunError::StructuralMutationForbidden);
        }
        Ok(())
    }

    /// Refuses an Action while the Run is not `Running` (RS-18).
    pub fn check_running(&self) -> Result<(), RunError> {
        if matches!(self.state, RunState::Running {}) {
            Ok(())
        } else {
            Err(RunError::RunNotRunning)
        }
    }
}
