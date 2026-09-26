use std::sync::{Mutex, MutexGuard};

use crate::event::{Action, Event, EventKind, EventSink};
use crate::manifest::ArtifactRef;
use crate::module_api::{
    CoerceReport, Executor, ExecutorDescriptor, ModuleError, ModuleErrorKind, PrepareContext,
    Provider, ProviderInstance, Requested, Sink, StepOutcome, SteppedInstance, SteppedRef,
    StopMode,
};
use crate::plan::{Fragment, PrepareReport};
use crate::policy::Reaction;
use crate::run::{CleanupMode, CleanupStep, RunState, Stage, StopCause, Termination};
use crate::time::TimePoint;

use super::state::{Inst, Shared, Slot, contain, contain_all, lock, try_slot};
use super::{DRAIN_WAKEUP_CAP, RunHandle};

struct Fault(Mutex<Option<(Inst, ModuleError)>>);

fn guard_step(
    inst: Inst,
    fault: &Fault,
    call: impl FnOnce() -> Result<StepOutcome, ModuleError>,
) -> Result<StepOutcome, ModuleError> {
    let result = contain(call);
    if let Err(error) = &result {
        let mut first = lock(&fault.0);
        if first.is_none() {
            *first = Some((inst, error.clone()));
        }
    }
    result
}

/// One role's watched instance. The three role traits differ in their signatures but
/// not in their wrapping, so one generic wrapper serves all three: only `step` is
/// contained and attributed, and every other method passes straight through.
struct Watch<'a, T: ?Sized> {
    inner: &'a mut T,
    inst: Inst,
    fault: &'a Fault,
}

impl Provider for Watch<'_, dyn Provider> {
    fn instance(&self) -> &ProviderInstance { self.inner.instance() }
    fn coerce(&self, request: &Requested) -> Result<CoerceReport, ModuleError> { self.inner.coerce(request) }
    fn prepare(&mut self, f: &Fragment, ctx: PrepareContext) -> Result<PrepareReport, ModuleError> { self.inner.prepare(f, ctx) }
    fn arm(&mut self) -> Result<(), ModuleError> { self.inner.arm() }
    fn start(&mut self, at: Option<TimePoint>) -> Result<(), ModuleError> { self.inner.start(at) }
    fn stop(&mut self, mode: StopMode) -> Result<(), ModuleError> { self.inner.stop(mode) }
    fn cleanup(&mut self) { self.inner.cleanup() }
    fn step(&mut self, until: TimePoint) -> Result<StepOutcome, ModuleError> {
        guard_step(self.inst, self.fault, || self.inner.step(until))
    }
}

impl Executor for Watch<'_, dyn Executor> {
    fn descriptor(&self) -> &ExecutorDescriptor { self.inner.descriptor() }
    fn prepare(&mut self, island: &crate::module_api::IslandDecl, ctx: PrepareContext) -> Result<PrepareReport, ModuleError> { self.inner.prepare(island, ctx) }
    fn arm(&mut self) -> Result<(), ModuleError> { self.inner.arm() }
    fn start(&mut self) -> Result<(), ModuleError> { self.inner.start() }
    fn step(&mut self, until: TimePoint) -> Result<StepOutcome, ModuleError> {
        guard_step(self.inst, self.fault, || self.inner.step(until))
    }
    fn stop(&mut self, mode: StopMode) -> Result<(), ModuleError> { self.inner.stop(mode) }
    fn cleanup(&mut self) { self.inner.cleanup() }
}

impl Sink for Watch<'_, dyn Sink> {
    fn descriptor(&self) -> &crate::module_api::SinkDescriptor { self.inner.descriptor() }
    fn prepare(&mut self, f: &Fragment, ctx: PrepareContext) -> Result<PrepareReport, ModuleError> { self.inner.prepare(f, ctx) }
    fn arm(&mut self) -> Result<(), ModuleError> { self.inner.arm() }
    fn start(&mut self) -> Result<(), ModuleError> { self.inner.start() }
    fn step(&mut self, until: TimePoint) -> Result<StepOutcome, ModuleError> {
        guard_step(self.inst, self.fault, || self.inner.step(until))
    }
    fn stop(&mut self, mode: StopMode) -> Result<Vec<ArtifactRef>, ModuleError> { self.inner.stop(mode) }
    fn cleanup(&mut self) { self.inner.cleanup() }
}

/// The `(Inst, guard)` pairs of one role that `round` may step: every instance whose
/// `done` set does not yet hold its `RestoreBaseline` step. A cleanup step holds the
/// lock while it runs, so a cleaning round skips what it cannot take.
fn steppable<'a, T: ?Sized + 'a>(
    slots: impl Iterator<Item = (Inst, &'a Slot<T>)>,
    cleaning: bool,
    is_done: &impl Fn(Inst) -> bool,
) -> Vec<(Inst, MutexGuard<'a, Box<T>>)> {
    let mut out = Vec::new();
    for (inst, slot) in slots {
        if is_done(inst) {
            continue;
        }
        let guard = if cleaning { try_slot(slot) } else { Some(lock(slot)) };
        if let Some(guard) = guard {
            // Read twice: a `RestoreBaseline` that took the lock between the two
            // reads recorded `done`, and stepping the instance again would run it
            // after its `cleanup()`.
            if !is_done(inst) {
                out.push((inst, guard));
            }
        }
    }
    out
}

/// Wraps each locked instance so its `step` is contained and attributed. `***guard`
/// is guard → `MutexGuard` → `Box<T>` → `T`.
fn watched<'g, 'f: 'g, T: ?Sized + 'f>(
    guarded: impl Iterator<Item = &'g mut (Inst, MutexGuard<'f, Box<T>>)>,
    fault: &'g Fault,
) -> Vec<Watch<'g, T>> {
    guarded
        .map(|(inst, guard)| Watch { inner: &mut ***guard, inst: *inst, fault })
        .collect()
}

/// Runs the deterministic stepping table, contains Module errors and applies events.
pub(super) fn round(shared: &Shared, at: TimePoint, cleaning: bool) {
    let Some(collector) = shared.collector.get() else {
        return;
    };
    let fault = Fault(Mutex::new(None));
    let restored = CleanupStep::RestoreBaseline as u8;
    let is_done = |inst: Inst| cleaning && lock(&shared.done).contains(&(restored, inst));
    // A scope, so the guards and the watches borrowing them are released in reverse
    // declaration order before the fault is read: `step_until_quiescent` borrows the
    // instances, and no Module may still be locked when `drain_and_react` runs.
    let result = {
        let mut providers = steppable(
            shared
                .providers
                .iter()
                .enumerate()
                .filter(|(_, slot)| slot.stepped)
                .map(|(i, slot)| (Inst::Provider(i), &slot.object)),
            cleaning,
            &is_done,
        );
        let mut executors = steppable(
            shared
                .executors
                .iter()
                .enumerate()
                .map(|(i, slot)| (Inst::Executor(i), &slot.object)),
            cleaning,
            &is_done,
        );
        let mut sinks = steppable(
            shared
                .sinks
                .iter()
                .enumerate()
                .map(|(i, slot)| (Inst::Sink(i), &slot.object)),
            cleaning,
            &is_done,
        );
        let mut watched_providers = watched(providers.iter_mut(), &fault);
        let mut watched_executors = watched(executors.iter_mut(), &fault);
        let mut watched_sinks = watched(sinks.iter_mut(), &fault);
        let mut instances = Vec::with_capacity(
            watched_providers.len() + watched_executors.len() + watched_sinks.len(),
        );
        for watch in watched_providers.iter_mut() {
            instances.push(SteppedInstance {
                id: shared.first_fragment(watch.inst),
                inner: SteppedRef::Provider(watch),
            });
        }
        for watch in watched_executors.iter_mut() {
            instances.push(SteppedInstance {
                id: shared.first_fragment(watch.inst),
                inner: SteppedRef::Executor(watch),
            });
        }
        for watch in watched_sinks.iter_mut() {
            instances.push(SteppedInstance {
                id: shared.first_fragment(watch.inst),
                inner: SteppedRef::Sink(watch),
            });
        }
        crate::module_api::step_until_quiescent(
            &mut instances,
            at,
            &**collector,
            &super::state::kernel_source(),
        )
    };
    if let Err(_error) = result {
        match lock(&fault.0).take() {
            Some((inst, failure)) if failure.kind == ModuleErrorKind::DeviceLost => {
                super::pipeline::emit_device_lost(shared, inst, &failure);
            }
            Some((inst, failure)) => fail_run(shared, inst, &failure.message),
            None => {}
        }
    }
    drain_and_react(shared);
}

fn fail_run(shared: &Shared, inst: Inst, reason: &str) {
    let full = format!("KC-30: {}: {reason}", shared.first_fragment(inst));
    super::ending::request(
        shared,
        Termination::Failed { stage: Stage::Run },
        CleanupMode::Abort,
        Some(full),
    );
}

pub(super) fn drain_and_react(shared: &Shared) {
    let Some(collector) = shared.collector.get() else {
        return;
    };
    let Some(policy) = shared.policy.get() else {
        return;
    };
    let events = collector.drain();
    for event in &events {
        let reaction = policy.reaction_for_event(&event.kind, event.severity);
        if reaction == Reaction::MarkArtifact {
            lock(&shared.marks).push((event.kind.clone(), event.time));
        }
        let mode = match reaction {
            Reaction::Continue | Reaction::MarkArtifact => continue,
            Reaction::Stop => CleanupMode::Orderly,
            Reaction::Abort => CleanupMode::Abort,
        };
        super::ending::request(
            shared,
            Termination::Stopped {
                cause: StopCause::Policy {
                    kind: event.kind.clone(),
                },
            },
            mode,
            None,
        );
    }
    lock(&shared.delivered).extend(events);
    if let Some((kind, reaction)) = collector.escalation() {
        let mode = match reaction {
            Reaction::Abort => CleanupMode::Abort,
            Reaction::Continue | Reaction::MarkArtifact | Reaction::Stop => CleanupMode::Orderly,
        };
        super::ending::request(
            shared,
            Termination::Stopped {
                cause: StopCause::Policy { kind },
            },
            mode,
            None,
        );
    }
}

/// KC-22's livelock event at `at`: source `kernel`, severity `fatal`, payload
/// `{ "wakeups": … }` through `emit_control` (RS-27, RS-28). The two loops that
/// count `next_wakeup` results at one instant report it here rather than each
/// spelling the Event; `step_until_quiescent`'s own `STEP_LIVELOCK` carries
/// `{ "rounds": … }` (MA-30) and is not this one.
fn emit_livelock(shared: &Shared, at: TimePoint) {
    let Some(collector) = shared.collector.get() else {
        return;
    };
    let _ = collector.emit_control(Event {
        source: super::state::kernel_source(),
        time: at,
        severity: crate::event::Severity::Fatal,
        kind: EventKind::parse(EventKind::STEP_LIVELOCK).expect("Kernel event kind"),
        payload: serde_json::json!({ "wakeups": crate::module_api::STEP_ROUND_CAP }),
    });
}

pub(super) fn drain_cleanup(shared: &Shared) -> bool {
    let mode = lock(&shared.end).as_ref().map(|end| end.mode);
    if mode != Some(CleanupMode::Orderly)
        || shared
            .drained
            .swap(true, std::sync::atomic::Ordering::AcqRel)
    {
        return false;
    }
    // Read on both sides of `next_wakeup()`, which is a Module call: a later abort or a
    // `closing` flag must break the drain even if it arrives while the Authority runs.
    let interrupted = || {
        lock(&shared.end)
            .as_ref()
            .is_some_and(|end| end.mode == CleanupMode::Abort)
            || shared.closing.load(std::sync::atomic::Ordering::Acquire)
    };
    let mut last = None;
    let mut count = 0usize;
    for _ in 0..DRAIN_WAKEUP_CAP {
        if interrupted() {
            break;
        }
        let wakeup = contain_all(|| shared.authority.next_wakeup())
            .ok()
            .flatten();
        if interrupted() {
            break;
        }
        let Some(at) = wakeup else {
            break;
        };
        count = if last == Some(at) {
            count.saturating_add(1)
        } else {
            1
        };
        last = Some(at);
        if count > crate::module_api::STEP_ROUND_CAP {
            emit_livelock(shared, at);
            drain_and_react(shared);
            break;
        }
        round(shared, at, true);
    }
    true
}

impl RunHandle {
    pub(super) fn run_loop(&mut self, until: Option<TimePoint>) {
        loop {
            if lock(&self.shared.end).is_some() || !matches!(self.state(), RunState::Running {}) {
                return;
            }
            let wakeup = match contain_all(|| self.shared.authority.next_wakeup()) {
                Ok(wakeup) => wakeup,
                Err(()) => {
                    super::ending::request(
                        &self.shared,
                        Termination::Failed { stage: Stage::Run },
                        CleanupMode::Abort,
                        Some("KC-30: authority panicked during next_wakeup".to_owned()),
                    );
                    return;
                }
            };
            let Some(at) = wakeup else {
                if self.kind() == crate::manifest::RunKind::Spec {
                    super::ending::request(
                        &self.shared,
                        Termination::Completed {},
                        CleanupMode::Orderly,
                        None,
                    );
                }
                return;
            };
            self.same_count = if self.last_wakeup == Some(at) {
                self.same_count.saturating_add(1)
            } else {
                1
            };
            self.last_wakeup = Some(at);
            if self.same_count > crate::module_api::STEP_ROUND_CAP {
                self.same_count = 0;
                emit_livelock(&self.shared, at);
                drain_and_react(&self.shared);
                if lock(&self.shared.end).is_none()
                    && self.shared.policy.get().is_some_and(|p| {
                        !matches!(
                            p.reaction_for_event(
                                &EventKind::parse(EventKind::STEP_LIVELOCK)
                                    .expect("Kernel event kind"),
                                crate::event::Severity::Fatal,
                            ),
                            Reaction::Stop | Reaction::Abort
                        )
                    })
                {
                    let reason = format!("KC-22: STEP_LIVELOCK at {}; time cannot advance", at);
                    super::ending::request(
                        &self.shared,
                        Termination::Failed { stage: Stage::Run },
                        CleanupMode::Abort,
                        Some(reason),
                    );
                }
                continue;
            }
            self.dispatch_agenda(at);
            round(&self.shared, at, false);
            self.check_lease();
            if let Some(horizon) = until {
                if at.domain != horizon.domain || at.ticks >= horizon.ticks {
                    return;
                }
            }
        }
    }

    fn dispatch_agenda(&mut self, at: TimePoint) {
        self.agenda.sort_by_key(|(tick, index, _)| (*tick, *index));
        let count = self
            .agenda
            .iter()
            .take_while(|(tick, _, _)| *tick <= at.ticks)
            .count();
        let due: Vec<_> = self.agenda.drain(..count).collect();
        for (_, index, action) in due {
            if matches!(action, Action::Stop { target: None }) {
                super::ending::request(
                    &self.shared,
                    Termination::Completed {},
                    CleanupMode::Orderly,
                    None,
                );
                break;
            }
            match super::admission::admit(&self.shared, action, super::state::Origin::Schedule) {
                Ok(admitted) => {
                    super::admission::dispatch(&self.shared, admitted);
                }
                Err(violations) => {
                    super::ending::request(
                        &self.shared,
                        Termination::Failed { stage: Stage::Run },
                        CleanupMode::Abort,
                        Some(format!("KC-17: agenda entry {index}: {violations:?}")),
                    );
                    break;
                }
            }
        }
    }

    /// Advances the Run to an instant on the Authority's primary root (KC-29).
    pub fn advance_to(&mut self, target: TimePoint) -> Result<(), super::RunHandleError> {
        self.check_lease();
        self.ensure_live()?;
        let t = super::state::ceil_convert(&self.shared.ctx.clocks, target, self.shared.primary)
            .map_err(|_| super::RunHandleError::NotOnPrimaryRoot { t: target })?;
        if t.ticks <= self.shared.now().ticks {
            return Ok(());
        }
        let handle = match self.shared.schedule(t, Box::new(|_| {})) {
            Ok(handle) => handle,
            Err(super::state::TimeScheduleError::Refused(_)) => {
                return Err(super::RunHandleError::NotOnPrimaryRoot { t: target });
            }
            Err(super::state::TimeScheduleError::Panicked) => {
                super::ending::request(
                    &self.shared,
                    Termination::Failed { stage: Stage::Run },
                    CleanupMode::Abort,
                    Some("KC-30: a Module panicked during run: Authority schedule()".to_owned()),
                );
                self.settle();
                return self.ended_result();
            }
        };
        lock(&self.shared.scheduled).push(handle);
        self.run_loop(Some(t));
        self.settle();
        self.ended_result()
    }

    /// Runs until the Run ends or reaches the requested horizon (KC-29).
    pub fn run_until_end(&mut self, horizon: TimePoint) -> Result<(), super::RunHandleError> {
        self.check_lease();
        self.ensure_live()?;
        let horizon =
            super::state::ceil_convert(&self.shared.ctx.clocks, horizon, self.shared.primary)
                .map_err(|_| super::RunHandleError::NotOnPrimaryRoot { t: horizon })?;
        self.run_loop(Some(horizon));
        self.settle();
        self.ended_result()
    }

    fn ended_result(&self) -> Result<(), super::RunHandleError> {
        if let RunState::CleanedUp { termination } = self.state() {
            Err(super::RunHandleError::Ended { termination })
        } else {
            Ok(())
        }
    }
}
