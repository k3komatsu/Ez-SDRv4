//! One Run from its documents to its sealed Manifest (KC-1…KC-45).

mod admission;
mod ending;
mod paced;
mod pipeline;
mod state;
mod stepping;

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use crate::binding::{AdmissionCheckRegistry, BindingProfile};
use crate::contract::ContractRegistry;
use crate::hash::ContentHash;
use crate::id::RunId;
use crate::manifest::{ArtifactRef, Manifest, RunKind};
use crate::module_api::{Authority, Executor, Link, ModuleRef, ModuleRegistry, Provider, Sink};
use crate::plan::PrepareReport;
use crate::policy::EventKindRegistry;
use crate::run::{Lease, RunState, Termination};
use crate::session::{LogEntry, SessionAction, SessionLog};
use crate::spec::{ExperimentSpec, Ident, Key, SpecError, Value};
use crate::time::{ClockRegistry, SampleClockRecord, TimePoint};

use state::Shared;

/// The coordinator's reserved event source (KA-14).
pub const KERNEL_SOURCE: &str = "kernel";
/// The bounded event ring allocated for each Run (KC-8, RS-34).
pub const EVENT_RING_DEPTH: usize = 4096;
/// The prepare and arm time budget in host monotonic nanoseconds (KC-11, MA-8).
pub const DEFAULT_HOST_BUDGET_NS: i64 = 5_000_000_000;
/// Maximum wakeups in the orderly drain (KA-12).
pub const DRAIN_WAKEUP_CAP: usize = 1_000_000;

/// The registries, clocks and Module instances supplied for one Run (KC-4).
pub struct Assembly {
    /// Modules, Vocabularies and Link descriptors (KC-4, KC-6).
    pub registry: ModuleRegistry,
    /// Registered admission checks (KC-4).
    pub checks: AdmissionCheckRegistry,
    /// Registered event kinds (KC-4, KC-8).
    pub kinds: EventKindRegistry,
    /// Registered DataContracts (KC-4).
    pub contracts: ContractRegistry,
    /// The registry in which the Authority registered its roots (KC-3).
    pub clocks: Arc<ClockRegistry>,
    /// The host clock used for transitions and leases (RS-5, KC-4).
    pub host_clock: Arc<dyn crate::run::HostClock>,
    /// Provider objects keyed by a Spec resource name (KC-4).
    pub providers: BTreeMap<Ident, Box<dyn Provider>>,
    /// Executor objects keyed by Island executor name (KC-4).
    pub executors: BTreeMap<Ident, Box<dyn Executor>>,
    /// Sink objects keyed by output id (or Session Sink binding name) (KC-4).
    pub sinks: BTreeMap<Ident, Box<dyn Sink>>,
    /// The Authority named by the profile (KC-3, KC-4).
    pub authority: Box<dyn Authority>,
    /// Link objects keyed by Module version (KC-4, KC-10).
    pub links: BTreeMap<ModuleRef, Box<dyn Link>>,
    /// Input bytes keyed by content hash (KC-9).
    pub inputs: BTreeMap<ContentHash, Vec<u8>>,
}

/// Refusals from the live Run control API (RS-15, RS-18, KC-28).
#[derive(Clone, PartialEq, Debug)]
pub enum RunHandleError {
    /// The Run has already been cleaned up.
    Ended {
        /// How the Run ended.
        termination: Termination,
    },
    /// `submit` was called on a Spec Run.
    NotSession,
    /// The submitted Session Action was malformed and takes no sequence number (RS-15).
    Malformed {
        /// Why the submitted Action is not valid.
        error: SpecError,
    },
    /// The requested instant cannot be placed on the Authority's primary root (KC-29).
    NotOnPrimaryRoot {
        /// The time that could not be placed.
        t: TimePoint,
    },
}

impl fmt::Display for RunHandleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ended { termination } => write!(f, "the Run has ended: {termination:?}"),
            Self::NotSession => f.write_str("submit is available only on a Session"),
            Self::Malformed { error } => write!(f, "the Session Action is malformed: {error}"),
            Self::NotOnPrimaryRoot { t } => write!(f, "{t} is not on the Authority's primary root"),
        }
    }
}

impl std::error::Error for RunHandleError {}

/// Control-path state for one live Run (KC-1, KC-5).
pub struct RunHandle {
    shared: Arc<Shared>,
    lease: Lease,
    log: SessionLog,
    inputs: Vec<ArtifactRef>,
    agenda: Vec<(i64, usize, crate::event::Action)>,
    t0: Option<TimePoint>,
    admission: crate::binding::AdmissionResult,
    reports: Option<Vec<PrepareReport>>,
    links_by_ref: BTreeMap<ModuleRef, Box<dyn Link>>,
    attached_links: BTreeMap<Ident, Vec<crate::module_api::AttachedPort>>,
    entry_failure: Option<String>,
    last_wakeup: Option<TimePoint>,
    same_count: usize,
    manifest: Option<Manifest>,
    parent: Option<RunId>,
    children: Vec<serde_json::Value>,
}

/// Parses and starts a Spec Run; parse and hash refusals create no Run (KC-1, KC-5).
pub fn start_spec_run(
    spec_doc: &serde_json::Value,
    profile_doc: &serde_json::Value,
    assembly: Assembly,
) -> Result<RunHandle, SpecError> {
    start_spec(spec_doc, profile_doc, assembly, Lease::attached(), None)
}

/// A Spec Run with its Lease and parent: `start_spec_run`'s, or a child's (KC-37a).
fn start_spec(
    spec_doc: &serde_json::Value,
    profile_doc: &serde_json::Value,
    assembly: Assembly,
    lease: Lease,
    parent: Option<RunId>,
) -> Result<RunHandle, SpecError> {
    let spec = ExperimentSpec::from_json(spec_doc)?;
    let profile = BindingProfile::from_json(profile_doc)?;
    let (spec_section, binding_section) = sections(&spec, &profile)?;
    let mut run = pipeline::assemble(
        RunKind::Spec,
        spec,
        spec_section,
        profile,
        binding_section,
        assembly,
        lease,
    );
    run.parent = parent;
    run.pipeline();
    run.settle();
    Ok(run)
}

/// Derives and starts an implicit Session Spec after validating the Lease (KC-1, KC-35).
pub fn connect(
    profile_doc: &serde_json::Value,
    assembly: Assembly,
    lease: Lease,
) -> Result<RunHandle, SpecError> {
    let profile = BindingProfile::from_json(profile_doc)?;
    lease.validate().map_err(|e| SpecError::Structural {
        reason: format!("KC-1: RS-21: {e}"),
    })?;
    let providers: BTreeMap<Ident, &dyn Provider> = assembly
        .providers
        .iter()
        .map(|(name, provider)| (name.clone(), &**provider))
        .collect();
    let sinks: BTreeMap<Ident, &dyn Sink> = assembly
        .sinks
        .iter()
        .map(|(name, sink)| (name.clone(), &**sink))
        .collect();
    let spec = state::contain_all(|| {
        crate::session::implicit_spec(&profile, &assembly.registry, &providers, &sinks)
    })
    .map_err(|_| SpecError::Structural {
        reason: "KC-30: a Module panicked during validate".to_owned(),
    })??;
    drop((providers, sinks));
    let (spec_section, binding_section) = sections(&spec, &profile)?;
    let mut run = pipeline::assemble(
        RunKind::Session,
        spec,
        spec_section,
        profile,
        binding_section,
        assembly,
        lease,
    );
    run.pipeline();
    run.settle();
    Ok(run)
}

/// The Manifest's `spec` and `binding` sections of the parsed documents (KC-45, RS-45).
fn sections(
    spec: &ExperimentSpec,
    profile: &BindingProfile,
) -> Result<(crate::manifest::SpecSection, crate::manifest::BindingSection), SpecError> {
    let structural = |e: crate::hash::HashError| SpecError::Structural {
        reason: format!("KC-1: {e}"),
    };
    Ok((
        crate::manifest::SpecSection::of(spec).map_err(structural)?,
        crate::manifest::BindingSection::of(profile).map_err(structural)?,
    ))
}

impl RunHandle {
    /// The Run's generated id (KC-5).
    pub fn id(&self) -> RunId {
        self.shared.ctx.id.clone()
    }

    /// Whether this is a Spec Run or a Session (KC-5).
    pub fn kind(&self) -> RunKind {
        self.shared.ctx.kind
    }

    /// The current lifecycle state (KC-7).
    pub fn state(&self) -> RunState {
        state::lock(&self.shared.machine).state().clone()
    }

    /// The Authority's current instant in its primary root (KC-3).
    pub fn now(&self) -> TimePoint {
        self.shared.now()
    }

    /// T0, once the Run has been armed (KC-15).
    pub fn start_instant(&self) -> Option<TimePoint> {
        self.t0
    }

    /// Admits one Session Action before appending it to the action log (KC-28).
    pub fn submit(
        &mut self,
        action: SessionAction,
        waveform: Option<&[u8]>,
    ) -> Result<LogEntry, RunHandleError> {
        self.check_lease();
        self.ensure_live()?;
        if self.kind() == RunKind::Spec {
            return Err(RunHandleError::NotSession);
        }
        pipeline::submit(self, action, waveform)
    }

    /// The effective values recorded per fragment (SB-41).
    pub fn effective(&self) -> BTreeMap<Ident, BTreeMap<Key, Value>> {
        state::lock(&self.shared.configuration).clone()
    }

    /// Sample clocks declared during prepare (TM-13d).
    pub fn sample_clocks(&self) -> Vec<SampleClockRecord> {
        self.shared.ctx.clocks.sample_clock_records()
    }

    /// Admits, logs and runs a child Spec Run to its end under this Session's Lease,
    /// and returns the entry and, when admitted, the child's Manifest (KC-37a).
    pub fn run_child(
        &mut self,
        spec_doc: &serde_json::Value,
        profile_doc: &serde_json::Value,
        assembly: Assembly,
        drive: &mut dyn FnMut(&mut RunHandle),
    ) -> Result<(LogEntry, Option<Manifest>), RunHandleError> {
        self.check_lease();
        self.ensure_live()?;
        if self.kind() == RunKind::Spec {
            return Err(RunHandleError::NotSession);
        }
        pipeline::run_child(self, spec_doc, profile_doc, assembly, drive)
    }

    /// Handles an Attached disconnect or starts a Detached Lease's expiry (KC-36).
    pub fn disconnect(&mut self) {
        let cause = self.lease.on_disconnect(&*self.shared.ctx.host_clock);
        self.sync_lease();
        if let Some(cause) = cause {
            ending::request(
                &self.shared,
                crate::run::Termination::Stopped { cause },
                crate::run::CleanupMode::Orderly,
                None,
            );
            self.settle();
        }
    }

    /// Checks the Detached Lease deadline (KC-36).
    pub fn check_lease(&mut self) {
        if self.lease.expired(&*self.shared.ctx.host_clock) {
            ending::request(
                &self.shared,
                crate::run::Termination::Stopped {
                    cause: crate::run::StopCause::LeaseExpiry {},
                },
                crate::run::CleanupMode::Orderly,
                None,
            );
            self.settle();
        }
    }

    /// Cleans up a live Run and returns its sealed Manifest (KC-34).
    pub fn finish(mut self) -> Manifest {
        self.check_lease();
        ending::finish(&mut self);
        self.manifest.take().expect("cleanup writes a Manifest")
    }

    pub(super) fn ensure_live(&self) -> Result<(), RunHandleError> {
        if let RunState::CleanedUp { termination } = self.state() {
            Err(RunHandleError::Ended { termination })
        } else {
            Ok(())
        }
    }

    /// Mirrors a Detached Lease's expiry for the data thread (KC-36 as KG-2 amends it).
    pub(super) fn sync_lease(&self) {
        let deadline = match self.lease.mode {
            crate::run::LeaseMode::Detached { .. } => self.lease.expires_at_host,
            crate::run::LeaseMode::Attached {} => None,
        };
        *state::lock(&self.shared.lease_deadline) = deadline;
    }
}

/// KC-46c: a live device-paced Run is cleaned up when its handle is dropped, as
/// `finish` does, and its Manifest discarded; in the Simulation class nothing happens.
impl Drop for RunHandle {
    fn drop(&mut self) {
        if self.shared.device_paced() && !matches!(self.state(), RunState::CleanedUp { .. }) {
            ending::finish(self);
        }
    }
}
