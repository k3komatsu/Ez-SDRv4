//! One Run from its documents to its sealed Manifest (KC-1…KC-45).

mod admission;
mod ending;
mod paced;
mod pipeline;
mod state;
mod stepping;
#[cfg(feature = "testing")]
pub mod start_probe {
    //! Pause points on the control thread's `start` (KC-18, KC-24a), for the tests that pin the admission lock's extent. Compiled only with the `testing` feature; inert unless a hook is installed on the calling thread.

    use std::cell::RefCell;
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Weak};

    use super::state::Shared;

    /// Where the control thread is.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum Point {
        /// First line of the pipeline.
        Entered,
        /// The lock is taken, the Run is still `Armed`.
        BeforePublish,
        /// `Running` is published and the data thread (device-paced) started; nothing dispatched yet.
        BeforeDispatch,
        /// About to dispatch the batch's entry `next`.
        Dispatching {
            /// Index of the entry.
            next: usize,
        },
        /// Right after the lock was dropped, before the wait and the round.
        Released,
        /// `advance_to_with`, after the early-return decision is taken and before the instant is read.
        Withdrawal,
        /// `advance_to_with`, with the withdrawal decided and the freeze gate not yet tried; the horizon's handle is still in `scheduled`.
        BeforeWithdrawalGate,
        /// `advance_to_with`, with the freeze gate held, after the horizon's handle left `scheduled` and before it is cancelled.
        Withdrawn,
        /// `advance_to_with`, after the horizon's `schedule` returned and before the handle is pushed to `scheduled`.
        Scheduled,
        /// `dispatch_agenda`, before the admission lock is taken for the agenda item `index`.
        BeforeAgendaLock {
            /// The item's schedule index.
            index: usize,
        },
    }

    /// What a hook can read without blocking.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub struct Reading {
        /// The admission lock is held by some thread (a `try_lock` that would block); `lock_record::Held::admission` is the calling thread's own.
        pub admission_held: bool,
        /// An end has been requested.
        pub end_requested: bool,
        /// Dispatch is frozen (RS-6 step 1).
        pub dispatch_frozen: bool,
        /// How many links' `drops` step 6 has stored (0 until it completes).
        pub link_drops_len: usize,
        /// How many events the Run has delivered (KC-31).
        pub delivered_len: usize,
        /// How many wake handles the wake list holds (KC-46a).
        pub wake_handles: usize,
        /// The highest generation in the wake list; 0 for none.
        pub wake_last_generation: u64,
        /// The last wake generation issued.
        pub wake_next: u64,
        /// The cleanup's performance of RS-6 step 1 took the wake list and closed it.
        pub wake_closed: bool,
    }

    /// Where a performance of RS-6 step 1 is (KC-24a).
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum FreezePoint {
        /// Before the freeze gate is taken.
        Serialise,
        /// The freeze gate is held, the admission lock is not yet.
        Serialised,
        /// `frozen` is set and the queues are cleared; the admission lock is still held.
        BeforeDrop,
        /// Every handle is cancelled; the gate is still held.
        Done,
        /// The data thread's Provider stop of one fragment is about to start (KC-46b): after its performance of step 1, with the freeze gate held.
        BeforeStop,
    }

    /// What the freeze hook sees at a [`FreezePoint`], on the thread that performs the step.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub struct FreezeReading {
        /// The coordinator locks the performing thread holds.
        pub held: super::lock_record::Held,
        /// Dispatch is frozen.
        pub dispatch_frozen: bool,
        /// The performing thread.
        pub thread: std::thread::ThreadId,
    }

    /// A hook for [`Probe::on_freeze`].
    pub type FreezeHook = Arc<dyn Fn(FreezePoint, FreezeReading) + Send + Sync>;

    /// A weak handle on the Run's shared state; reading it never keeps the Run alive.
    #[derive(Clone)]
    pub struct Probe(Weak<Shared>);

    impl Probe {
        /// A reading, or `None` once the Run is gone. Non-blocking except for a brief lock of `end`.
        pub fn read(&self) -> Option<Reading> {
            let shared = self.0.upgrade()?;
            let admission_held = shared.admission.is_held();
            let end_requested = shared.end.lock().is_some();
            let dispatch_frozen = shared.frozen.load(Ordering::Acquire);
            let link_drops_len = shared.link_drops.lock().len();
            let (wake_handles, wake_last_generation, wake_closed) = {
                let list = shared.wake_list.lock();
                (list.handles.len(), list.handles.iter().map(|wake| wake.generation).max().unwrap_or(0), list.closed)
            };
            let wake_next = shared.wake_next.load(Ordering::Acquire);
            let delivered_len = shared.delivered.lock().len();
            Some(Reading { admission_held, end_requested, dispatch_frozen, link_drops_len, delivered_len, wake_handles, wake_last_generation, wake_next, wake_closed })
        }

        /// Installs the hook every performance of RS-6 step 1 calls at each [`FreezePoint`], on whichever thread performs it (the data thread, a cleanup step thread). It may park its thread; the Run holds no lock of the hook's while it runs.
        pub fn on_freeze(&self, hook: impl Fn(FreezePoint, FreezeReading) + Send + Sync + 'static) {
            if let Some(shared) = self.0.upgrade() {
                *super::state::lock(&shared.freeze_hook) = Some(Arc::new(hook));
            }
        }
    }

    type Hook = Box<dyn FnMut(Point, &Probe)>;

    thread_local! {
        static HOOK: RefCell<Option<Hook>> = const { RefCell::new(None) };
    }

    /// Installs `hook` on the calling thread until the guard drops: the pipeline of a Run started on this thread calls it at each [`Point`].
    ///
    /// # Panics
    ///
    /// Panics if a hook is already installed on this thread.
    pub fn install(hook: impl FnMut(Point, &Probe) + 'static) -> Installed {
        HOOK.with(|slot| {
            let mut slot = slot.borrow_mut();
            assert!(slot.is_none(), "a start_probe hook is already installed on this thread");
            *slot = Some(Box::new(hook));
        });
        Installed(std::marker::PhantomData)
    }

    /// Removes the hook on drop. Thread-bound, so it cannot clear another thread's installation.
    pub struct Installed(std::marker::PhantomData<*const ()>);

    impl Drop for Installed {
        fn drop(&mut self) {
            HOOK.with(|slot| *slot.borrow_mut() = None);
        }
    }

    pub(super) fn reach(shared: &Arc<Shared>, point: Point) {
        let Some(mut hook) = HOOK.with(|slot| slot.borrow_mut().take()) else { return };
        hook(point, &Probe(Arc::downgrade(shared)));
        HOOK.with(|slot| *slot.borrow_mut() = Some(hook));
    }

    pub(super) fn reach_freeze(shared: &Shared, point: FreezePoint) {
        // The Arc is cloned out of the slot first, so a hook that parks never holds the slot's lock.
        let hook = super::state::lock(&shared.freeze_hook).clone();
        if let Some(hook) = hook {
            hook(point, FreezeReading { held: super::lock_record::held(), dispatch_frozen: shared.frozen.load(Ordering::Acquire), thread: std::thread::current().id() });
        }
    }
}

mod record;

#[cfg(feature = "testing")]
pub mod lock_record {
    //! Which of the coordinator's locks the calling thread holds, read from the record `Locked::lock` keeps (KC-24a). A test double reads it at each supplied call. Compiled only with the `testing` feature.

    /// Which locks the calling thread holds.
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
    pub struct Held {
        /// The admission lock.
        pub admission: bool,
        /// The Run state machine's lock.
        pub machine: bool,
        /// The freeze gate (RS-6 step 1's serialisation).
        pub freeze: bool,
        /// Any other lock of the coordinator's own (`Locked::new`).
        pub state: bool,
    }

    /// The locks the calling thread holds now.
    pub fn held() -> Held {
        let [admission, machine, freeze, state] = super::record::counts();
        Held { admission: admission > 0, machine: machine > 0, freeze: freeze > 0, state: state > 0 }
    }

    #[cfg(test)]
    mod tests {
        use super::super::record::Recorded;
        use super::super::state::Which;
        use super::*;

        #[test]
        fn held_records_the_calling_threads_guards() {
            let admission_lock = std::sync::Mutex::new(());
            let machine_lock = std::sync::Mutex::new(());
            let admission = Recorded::new(admission_lock.lock().unwrap(), Which::Admission);
            assert!(held().admission);
            assert!(!held().machine);
            let machine = Recorded::new(machine_lock.lock().unwrap(), Which::Machine);
            assert!(held().admission && held().machine);
            drop(machine);
            assert!(held().admission && !held().machine);
            drop(admission);
            assert_eq!(held(), Held::default());
        }

        #[test]
        fn held_records_a_locked_state_lock_and_only_that() {
            use super::super::state::Locked;
            let lock = Locked::new(0u8);
            let raw = std::sync::Mutex::new(0u8);
            let raw_guard = raw.lock().unwrap();
            assert!(!held().state, "a raw Mutex guard is not recorded");
            drop(raw_guard);
            let guard = lock.lock();
            assert!(held().state && !held().admission && !held().machine && !held().freeze);
            drop(guard);
            assert_eq!(held(), Held::default());
        }
    }
}

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

/// The bounded event ring allocated for each Run (KC-8, RS-34).
pub const EVENT_RING_DEPTH: usize = 4096;
/// The prepare and arm time budget in host monotonic nanoseconds (KC-11, MA-8).
pub const DEFAULT_HOST_BUDGET_NS: i64 = 5_000_000_000;
/// Maximum wakeups in the orderly drain (KA-12).
pub const DRAIN_WAKEUP_CAP: usize = 1_000_000;

/// The registries, clocks and Module instances supplied for one Run (KC-4).
/// Non-exhaustive: built by [`Assembly::new`], every other field assigned.
#[non_exhaustive]
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
    /// The hash of the code that generated the Spec, when the caller has one; the
    /// Manifest records it as `spec.source` and nothing reads it (SB-14, RS-38).
    pub spec_source: Option<ContentHash>,
}

impl Assembly {
    /// An Assembly of the Authority and the registry it registered its roots in, with
    /// the standard contracts, the Kernel's event kinds, the system host clock and
    /// nothing else; the runtime assigns the rest (KC-4).
    pub fn new(authority: Box<dyn Authority>, clocks: Arc<ClockRegistry>) -> Assembly {
        Assembly {
            registry: ModuleRegistry::new(),
            checks: AdmissionCheckRegistry::new(),
            kinds: EventKindRegistry::with_kernel_kinds(),
            contracts: ContractRegistry::with_standard_contracts(),
            clocks,
            host_clock: Arc::new(crate::run::SystemHostClock::new()),
            providers: BTreeMap::new(),
            executors: BTreeMap::new(),
            sinks: BTreeMap::new(),
            authority,
            links: BTreeMap::new(),
            inputs: BTreeMap::new(),
            spec_source: None,
        }
    }
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
    reports: Option<BTreeMap<Ident, PrepareReport>>,
    links_by_ref: BTreeMap<ModuleRef, Box<dyn Link>>,
    attached_links: BTreeMap<Ident, Vec<crate::module_api::AttachedPort>>,
    entry_failure: Option<String>,
    last_wakeup: Option<TimePoint>,
    same_count: usize,
    manifest: Option<Manifest>,
    parent: Option<RunId>,
    children: Vec<crate::manifest::ChildRecord>,
}

/// Parses and starts a Spec Run; parse and hash refusals create no Run (KC-1, KC-5).
pub fn start_spec_run(spec_doc: &serde_json::Value, profile_doc: &serde_json::Value, assembly: Assembly) -> Result<RunHandle, SpecError> {
    start_spec(spec_doc, profile_doc, assembly, Lease::attached(), None)
}

/// A Spec Run with its Lease and parent: `start_spec_run`'s, or a child's (KC-37a).
fn start_spec(spec_doc: &serde_json::Value, profile_doc: &serde_json::Value, assembly: Assembly, lease: Lease, parent: Option<RunId>) -> Result<RunHandle, SpecError> {
    let spec = ExperimentSpec::from_json(spec_doc)?;
    let profile = BindingProfile::from_json(profile_doc)?;
    let (spec_section, binding_section) = sections(&spec, &profile, assembly.spec_source.clone())?;
    Ok(start_parsed(spec, profile, (spec_section, binding_section), assembly, lease, parent))
}

/// Starts a parsed Spec Run from its documents' sections (KC-1, KC-37a).
fn start_parsed(
    spec: ExperimentSpec,
    profile: BindingProfile,
    sections: (crate::manifest::SpecSection, crate::manifest::BindingSection),
    assembly: Assembly,
    lease: Lease,
    parent: Option<RunId>,
) -> RunHandle {
    let (spec_section, binding_section) = sections;
    let mut run = pipeline::assemble(RunKind::Spec, spec, spec_section, profile, binding_section, assembly, lease);
    run.parent = parent;
    run.pipeline();
    run.settle();
    run
}

/// Derives and starts an implicit Session Spec after validating the Lease (KC-1, KC-35);
/// `initial` enters the Spec's `requires` before it is hashed (RS-12).
pub fn connect(profile_doc: &serde_json::Value, initial: &crate::session::InitialRequests, assembly: Assembly, lease: Lease) -> Result<RunHandle, SpecError> {
    let profile = BindingProfile::from_json(profile_doc)?;
    lease.validate().map_err(|e| SpecError::Structural { reason: format!("KC-1: RS-21: {e}") })?;
    let providers: BTreeMap<Ident, &dyn Provider> = assembly.providers.iter().map(|(name, provider)| (name.clone(), &**provider)).collect();
    let sinks: BTreeMap<Ident, &dyn Sink> = assembly.sinks.iter().map(|(name, sink)| (name.clone(), &**sink)).collect();
    let spec = state::contain_all(|| crate::session::implicit_spec(&profile, &assembly.registry, &providers, &sinks, initial))
        .map_err(|_| SpecError::Structural { reason: "KC-30: a Module panicked during validate".to_owned() })??;
    drop((providers, sinks));
    let (spec_section, binding_section) = sections(&spec, &profile, assembly.spec_source.clone())?;
    let mut run = pipeline::assemble(RunKind::Session, spec, spec_section, profile, binding_section, assembly, lease);
    run.pipeline();
    run.settle();
    Ok(run)
}

/// The Manifest's `spec` and `binding` sections of the parsed documents (KC-45, RS-45).
pub(super) fn sections(spec: &ExperimentSpec, profile: &BindingProfile, source: Option<ContentHash>) -> Result<(crate::manifest::SpecSection, crate::manifest::BindingSection), SpecError> {
    let structural = |e: crate::hash::HashError| SpecError::Structural { reason: format!("KC-1: {e}") };
    Ok((crate::manifest::SpecSection::of(spec, source).map_err(structural)?, crate::manifest::BindingSection::of(profile).map_err(structural)?))
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
        self.shared.lock_machine().state().clone()
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
    pub fn submit(&mut self, action: SessionAction, waveform: Option<&[u8]>) -> Result<LogEntry, RunHandleError> {
        self.check_lease();
        self.ensure_live()?;
        if self.kind() == RunKind::Spec {
            return Err(RunHandleError::NotSession);
        }
        pipeline::submit(self, action, waveform)
    }

    /// The effective values recorded per fragment (SB-41).
    pub fn effective(&self) -> BTreeMap<Ident, BTreeMap<Key, Value>> {
        self.shared.configuration.lock().clone()
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
            ending::request(&self.shared, crate::run::Termination::Stopped { cause }, crate::run::CleanupMode::Orderly);
            self.settle();
        }
    }

    /// A function any thread may call, any number of times, to report that an Attached
    /// Lease's client has gone (KC-36): it requests `Stopped { client_disconnect }` in
    /// orderly mode and, when that request set the end in a class whose control loop waits
    /// in host time, wakes that loop (KC-46a). It runs no cleanup: the control thread's call
    /// in progress, or its next one that runs a round or settles, does (KC-38); a `submit`
    /// in progress or made after it still dispatches until cleanup freezes dispatch. It holds the Run weakly and does nothing
    /// for a Detached Lease or once the Run is gone. While it runs it holds the Run, so a
    /// caller that may call it as the handle is dropped serialises the two: otherwise the
    /// Run's last reference, and its devices, may be dropped on the calling thread.
    pub fn disconnect_signal(&self) -> impl Fn() + Send + Sync + 'static {
        // RS-23's one decision of what a disconnect means, on a copy: the Lease's mode never changes.
        let cause = self.lease.clone().on_disconnect(&*self.shared.ctx.host_clock);
        let shared = Arc::downgrade(&self.shared);
        move || {
            let (Some(cause), Some(shared)) = (cause.clone(), shared.upgrade()) else {
                return;
            };
            // Only HardwareInLoop and Hardware reach Running among the classes that wait in host time (KC-2).
            if ending::request(&shared, Termination::Stopped { cause }, crate::run::CleanupMode::Orderly) && shared.device_paced() {
                paced::wake(&shared, true);
            }
        }
    }

    /// Checks the Detached Lease deadline (KC-36).
    pub fn check_lease(&mut self) {
        if self.lease.expired(&*self.shared.ctx.host_clock) {
            ending::request(&self.shared, crate::run::Termination::Stopped { cause: crate::run::StopCause::LeaseExpiry {} }, crate::run::CleanupMode::Orderly);
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
        if let RunState::CleanedUp { termination } = self.state() { Err(RunHandleError::Ended { termination }) } else { Ok(()) }
    }

    /// Mirrors a Detached Lease's expiry for the data thread (KC-36 as KG-2 amends it).
    pub(super) fn sync_lease(&self) {
        let deadline = match self.lease.mode {
            crate::run::LeaseMode::Detached { .. } => self.lease.expires_at_host,
            crate::run::LeaseMode::Attached {} => None,
        };
        *self.shared.lease_deadline.lock() = deadline;
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
