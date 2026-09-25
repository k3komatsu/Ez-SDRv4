use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, TryLockError};

use crate::binding::{AdmissionCheckRegistry, BindingProfile};
use crate::contract::ContractRegistry;
use crate::event::{Action, Event, EventCollector, EventKind};
use crate::id::{ClockDomainId, ResourceId, RunId};
use crate::manifest::{ArtifactRef, BindingSection, RunKind, SpecSection};
use crate::module_api::{
    ActionReceiver, Authority, Executor, ExecutorDescriptor, ModuleError, ModuleErrorKind,
    ModuleRef, ModuleRegistry, Provider, Sink,
};
use crate::plan::ExecutionPlan;
use crate::policy::{EventKindRegistry, Policy};
use crate::run::{
    CleanupFailure, CleanupMode, HostClock, RunState, RunStateMachine, Stage, StopCause,
    Termination,
};
use crate::spec::{ExperimentSpec, Ident, Key, Namespace, Value};
use crate::stream::DataLinkDecl;
use crate::time::{
    ClockRegistry, Converted, Duration, Rescaled, ScheduleHandle, TimeAuthority, TimeError,
    TimePoint,
};

pub(super) type Slot<T> = Arc<Mutex<Box<T>>>;

/// One instance's inbound Action queue (MA-14, KC-25).
pub(super) struct Queue(pub(super) Mutex<VecDeque<Action>>);

impl ActionReceiver for Queue {
    fn recv(&self) -> Option<Action> {
        lock(&self.0).pop_front()
    }
}

/// Which instance: an index into Shared's slot vectors (KC-13).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(super) enum Inst {
    Provider(usize),
    Sink(usize),
    Executor(usize),
}

pub(super) struct ProviderSlot {
    pub(super) names: Vec<Ident>,
    pub(super) id: ResourceId,
    pub(super) module: ModuleRef,
    pub(super) stepped: bool,
    pub(super) lead: Option<Duration>,
    pub(super) object: Slot<dyn Provider>,
    pub(super) queue: Arc<Queue>,
}

pub(super) struct SinkSlot {
    pub(super) output: Ident,
    pub(super) object: Slot<dyn Sink>,
    pub(super) queue: Arc<Queue>,
}

pub(super) struct ExecutorSlot {
    pub(super) name: Ident,
    pub(super) descriptor: ExecutorDescriptor,
    pub(super) object: Slot<dyn Executor>,
    pub(super) queue: Arc<Queue>,
}

/// Where an Action or an event comes from (KC-24).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Origin {
    Schedule,
    Session,
    Module,
}

#[derive(Clone, PartialEq, Debug)]
pub(super) struct EndRequest {
    pub(super) termination: Termination,
    pub(super) mode: CleanupMode,
}

/// What never changes after entry (KC-6).
pub(super) struct Context {
    pub(super) kind: RunKind,
    pub(super) id: RunId,
    pub(super) spec: ExperimentSpec,
    pub(super) spec_section: SpecSection,
    pub(super) profile: BindingProfile,
    pub(super) binding_section: BindingSection,
    pub(super) environment: Arc<BTreeMap<Namespace, serde_json::Value>>,
    pub(super) registry: ModuleRegistry,
    pub(super) checks: AdmissionCheckRegistry,
    pub(super) kinds: EventKindRegistry,
    pub(super) contracts: ContractRegistry,
    pub(super) clocks: Arc<ClockRegistry>,
    pub(super) host_clock: Arc<dyn HostClock>,
    pub(super) declared_classes: BTreeMap<Key, crate::module_api::UpdateClass>,
    pub(super) outputs: BTreeSet<Ident>,
}

/// Set once, right after `plan()` succeeds (KC-13, KC-23).
pub(super) struct Routing {
    pub(super) plan: ExecutionPlan,
    pub(super) matched: BTreeMap<Ident, ResourceId>,
    pub(super) fragment_of: BTreeMap<Ident, Inst>,
    pub(super) first_fragment: BTreeMap<Inst, Ident>,
    pub(super) order: Vec<Inst>,
    pub(super) island_of: BTreeMap<Ident, Ident>,
    pub(super) reverse: Vec<Ident>,
}

/// Everything a cleanup step can touch (spec 06 §3).
pub(super) struct Shared {
    pub(super) ctx: Context,
    pub(super) providers: Vec<ProviderSlot>,
    pub(super) sinks: Vec<SinkSlot>,
    pub(super) executors: Vec<ExecutorSlot>,
    pub(super) authority: Arc<dyn Authority>,
    pub(super) time: Arc<dyn TimeAuthority>,
    pub(super) primary: ClockDomainId,
    pub(super) last_now: AtomicI64,
    pub(super) routing: OnceLock<Routing>,
    pub(super) machine: Mutex<RunStateMachine>,
    pub(super) collector: OnceLock<Arc<EventCollector>>,
    pub(super) policy: OnceLock<Policy>,
    pub(super) configuration: Mutex<BTreeMap<Ident, BTreeMap<Key, Value>>>,
    pub(super) next_action: AtomicU64,
    pub(super) frozen: AtomicBool,
    pub(super) delivered: Mutex<Vec<Event>>,
    pub(super) marks: Mutex<Vec<(EventKind, TimePoint)>>,
    pub(super) end: Mutex<Option<EndRequest>>,
    pub(super) also: Mutex<Vec<StopCause>>,
    pub(super) failure: Mutex<Option<(Stage, String)>>,
    pub(super) artifacts: Mutex<Vec<ArtifactRef>>,
    pub(super) links: Mutex<Vec<(DataLinkDecl, Arc<dyn crate::stream::DataLink>)>>,
    pub(super) link_drops: Mutex<Vec<u64>>,
    pub(super) counters: Mutex<Option<Vec<crate::event::CounterRow>>>,
    pub(super) prepared: Mutex<BTreeSet<Ident>>,
    pub(super) done: Mutex<BTreeSet<(u8, Inst)>>,
    pub(super) drained: AtomicBool,
    pub(super) closing: AtomicBool,
    pub(super) scheduled: Mutex<Vec<ScheduleHandle>>,
    pub(super) cleanup_failures: Mutex<Vec<CleanupFailure>>,
}

/// Poison-tolerant lock (§0.6 rule 7).
pub(super) fn lock<T: ?Sized>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A Module call under catch_unwind (KC-30).
pub(super) fn contain<T>(f: impl FnOnce() -> Result<T, ModuleError>) -> Result<T, ModuleError> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(_) => Err(ModuleError {
            kind: ModuleErrorKind::Internal,
            message: "a Module panicked".to_owned(),
            detail: serde_json::Value::Null,
        }),
    }
}

pub(super) fn contain_all<T>(f: impl FnOnce() -> T) -> Result<T, ()> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|_| ())
}

/// `try_lock` for cleanup and the Manifest (KC-44).
pub(super) fn try_slot<T: ?Sized>(m: &Mutex<T>) -> Option<MutexGuard<'_, T>> {
    match m.try_lock() {
        Ok(g) => Some(g),
        Err(TryLockError::Poisoned(p)) => Some(p.into_inner()),
        Err(TryLockError::WouldBlock) => None,
    }
}

/// `d` rescaled into `to`, rounded up to a whole tick (KC-15, KC-24, RS-19).
pub(super) fn ceil_rescale(
    clocks: &ClockRegistry,
    d: Duration,
    to: ClockDomainId,
) -> Result<i64, TimeError> {
    Ok(match clocks.rescale(d, to)? {
        Rescaled::Exact { duration } => duration.ticks,
        Rescaled::Inexact { floor, .. } => floor.ticks.checked_add(1).ok_or(TimeError::Overflow)?,
    })
}

/// `t` converted into `to`, rounded up (KC-29).
pub(super) fn ceil_convert(
    clocks: &ClockRegistry,
    t: TimePoint,
    to: ClockDomainId,
) -> Result<TimePoint, TimeError> {
    if t.domain == to {
        return Ok(t);
    }
    Ok(match clocks.convert(t, to)? {
        Converted::Exact { point } => point,
        Converted::Inexact { floor, .. } => {
            TimePoint::new(to, floor.ticks.checked_add(1).ok_or(TimeError::Overflow)?)
        }
    })
}

pub(super) fn kernel_source() -> ResourceId {
    ResourceId::parse(super::KERNEL_SOURCE).expect("a valid literal")
}

impl Shared {
    pub(super) fn now(&self) -> TimePoint {
        match contain_all(|| self.time.now(self.primary)) {
            Ok(Ok(now)) if now.domain == self.primary => {
                self.last_now
                    .store(now.ticks, std::sync::atomic::Ordering::Release);
                now
            }
            Ok(Ok(_)) => self.time_failure(
                "KC-30: the Authority time handle returned a time outside its primary root",
            ),
            Ok(Err(error)) => self.time_failure(&format!(
                "KC-30: the Authority time handle failed during now(): {error}"
            )),
            Err(()) => self.time_failure("KC-30: a Module panicked during now()"),
        }
    }

    fn time_failure(&self, reason: &str) -> TimePoint {
        let state = lock(&self.machine).state().clone();
        match state {
            RunState::Stopping { .. } | RunState::CleanedUp { .. } => {
                lock(&self.cleanup_failures).push(CleanupFailure {
                    step: crate::run::CleanupStep::ReleaseAndWriteManifest,
                    fragment: None,
                    reason: reason.to_owned(),
                    timed_out: false,
                });
            }
            _ => {
                let stage = match state {
                    RunState::Created {} => Stage::Validate,
                    RunState::Validated {} => Stage::Plan,
                    RunState::Planned {} => Stage::Prepare,
                    RunState::Prepared {} | RunState::Armed {} => Stage::Arm,
                    RunState::Running {} => Stage::Run,
                    RunState::Stopping { .. } | RunState::CleanedUp { .. } => unreachable!(),
                };
                super::ending::request(
                    self,
                    Termination::Failed { stage },
                    CleanupMode::Abort,
                    Some(reason.to_owned()),
                );
            }
        }
        TimePoint::new(
            self.primary,
            self.last_now.load(std::sync::atomic::Ordering::Acquire),
        )
    }

    pub(super) fn schedule(
        &self,
        at: TimePoint,
        callback: Box<dyn FnOnce(TimePoint) + Send>,
    ) -> Result<ScheduleHandle, TimeScheduleError> {
        match contain_all(|| self.time.schedule(at, callback)) {
            Ok(Ok(handle)) => Ok(handle),
            Ok(Err(error)) => Err(TimeScheduleError::Refused(error)),
            Err(()) => Err(TimeScheduleError::Panicked),
        }
    }

    pub(super) fn cancel(&self, handle: ScheduleHandle) -> Result<bool, ()> {
        contain_all(|| self.time.cancel(handle))
    }

    pub(super) fn routing(&self) -> Option<&Routing> {
        self.routing.get()
    }

    pub(super) fn queue(&self, i: Inst) -> &Arc<Queue> {
        match i {
            Inst::Provider(n) => &self.providers[n].queue,
            Inst::Sink(n) => &self.sinks[n].queue,
            Inst::Executor(n) => &self.executors[n].queue,
        }
    }

    pub(super) fn source_root(&self, i: Inst) -> ResourceId {
        match i {
            Inst::Provider(n) => self.providers[n].id.clone(),
            Inst::Sink(n) => ResourceId::parse(&format!("sink/{}", self.sinks[n].output))
                .expect("a valid Sink source"),
            Inst::Executor(_) => {
                let fragment = self.first_fragment(i);
                ResourceId::parse(fragment.as_str()).expect("a valid Island source")
            }
        }
    }

    pub(super) fn first_fragment(&self, i: Inst) -> Ident {
        if let Some(routing) = self.routing() {
            if let Some(name) = routing.first_fragment.get(&i) {
                return name.clone();
            }
        }
        match i {
            Inst::Provider(n) => self.providers[n].names[0].clone(),
            Inst::Sink(n) => self.sinks[n].output.clone(),
            Inst::Executor(n) => self.executors[n].name.clone(),
        }
    }

    pub(super) fn move_to(&self, state: RunState) {
        let at = Some(self.now());
        let result = lock(&self.machine).move_to(state, at, &*self.ctx.host_clock);
        debug_assert!(
            result.is_ok(),
            "coordinator attempted an illegal state transition: {result:?}"
        );
    }
}

pub(super) enum TimeScheduleError {
    Refused(TimeError),
    Panicked,
}

/// A failed `Authority::time()` still needs a time handle to write the failed Run's
/// Manifest. It is used only after KC-30 has made the Run fail at validate.
pub(super) struct FailedTime {
    pub(super) root: ClockDomainId,
}

impl TimeAuthority for FailedTime {
    fn primary_root(&self) -> ClockDomainId {
        self.root
    }
    fn pacing(&self) -> crate::module_api::Pacing {
        crate::module_api::Pacing::FreeRunning
    }
    fn governs(&self, domain: ClockDomainId) -> bool {
        domain == self.root || domain == ClockDomainId::HOST_MONOTONIC
    }
    fn now(&self, domain: ClockDomainId) -> Result<TimePoint, TimeError> {
        if self.governs(domain) {
            Ok(TimePoint::new(domain, 0))
        } else {
            Err(TimeError::NotGoverned { id: domain })
        }
    }
    fn wait_until(&self, time: TimePoint) -> Result<(), TimeError> {
        Err(TimeError::NotGoverned { id: time.domain })
    }
    fn schedule(
        &self,
        time: TimePoint,
        _f: Box<dyn FnOnce(TimePoint) + Send>,
    ) -> Result<ScheduleHandle, TimeError> {
        Err(TimeError::NotGoverned { id: time.domain })
    }
    fn cancel(&self, _handle: ScheduleHandle) -> bool {
        false
    }
}
