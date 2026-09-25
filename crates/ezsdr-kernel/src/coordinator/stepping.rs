use std::sync::Mutex;

use crate::event::{Action, Event, EventKind, EventSink};
use crate::manifest::ArtifactRef;
use crate::module_api::{
    CoerceReport, Executor, ExecutorDescriptor, ModuleError, ModuleErrorKind, PrepareContext,
    Provider, ProviderInstance, Requested, Sink, StepOutcome, SteppedInstance, SteppedRef,
    StopMode,
};
use crate::plan::{Fragment, PrepareReport};
use crate::policy::Reaction;
use crate::run::{CleanupMode, RunState, Stage, StopCause, Termination};
use crate::time::TimePoint;

use super::state::{contain, contain_all, lock, try_slot, Inst, Shared};
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

struct WatchProvider<'a> {
    inner: &'a mut dyn Provider,
    inst: Inst,
    fault: &'a Fault,
}

impl Provider for WatchProvider<'_> {
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

struct WatchExecutor<'a> {
    inner: &'a mut dyn Executor,
    inst: Inst,
    fault: &'a Fault,
}

impl Executor for WatchExecutor<'_> {
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

struct WatchSink<'a> {
    inner: &'a mut dyn Sink,
    inst: Inst,
    fault: &'a Fault,
}

impl Sink for WatchSink<'_> {
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

/// Runs the deterministic stepping table, contains Module errors and applies events.
pub(super) fn round(shared: &Shared, at: TimePoint, cleaning: bool) {
    let Some(collector) = shared.collector.get() else {
        return;
    };
    let fault = Fault(Mutex::new(None));
    let is_done = |inst: Inst| cleaning && lock(&shared.done).contains(&(5, inst));
    let mut providers = Vec::new();
    for (i, slot) in shared.providers.iter().enumerate().filter(|(_, slot)| slot.stepped) {
        let inst = Inst::Provider(i);
        if is_done(inst) {
            continue;
        }
        let guard = if cleaning { try_slot(&slot.object) } else { Some(lock(&slot.object)) };
        if let Some(guard) = guard {
            if !is_done(inst) {
                providers.push((inst, guard));
            }
        }
    }
    let mut executors = Vec::new();
    for (i, slot) in shared.executors.iter().enumerate() {
        let inst = Inst::Executor(i);
        if is_done(inst) {
            continue;
        }
        let guard = if cleaning { try_slot(&slot.object) } else { Some(lock(&slot.object)) };
        if let Some(guard) = guard {
            if !is_done(inst) {
                executors.push((inst, guard));
            }
        }
    }
    let mut sinks = Vec::new();
    for (i, slot) in shared.sinks.iter().enumerate() {
        let inst = Inst::Sink(i);
        if is_done(inst) {
            continue;
        }
        let guard = if cleaning { try_slot(&slot.object) } else { Some(lock(&slot.object)) };
        if let Some(guard) = guard {
            if !is_done(inst) {
                sinks.push((inst, guard));
            }
        }
    }
    let provider_ids: Vec<_> = providers.iter().map(|(inst, _)| *inst).collect();
    let executor_ids: Vec<_> = executors.iter().map(|(inst, _)| *inst).collect();
    let sink_ids: Vec<_> = sinks.iter().map(|(inst, _)| *inst).collect();
    let mut watched_providers: Vec<_> = providers
        .iter_mut()
        .zip(&provider_ids)
        .map(|((_, guard), inst)| WatchProvider {
            inner: &mut ***guard,
            inst: *inst,
            fault: &fault,
        })
        .collect();
    let mut watched_executors: Vec<_> = executors
        .iter_mut()
        .zip(&executor_ids)
        .map(|((_, guard), inst)| WatchExecutor {
            inner: &mut ***guard,
            inst: *inst,
            fault: &fault,
        })
        .collect();
    let mut watched_sinks: Vec<_> = sinks
        .iter_mut()
        .zip(&sink_ids)
        .map(|((_, guard), inst)| WatchSink {
            inner: &mut ***guard,
            inst: *inst,
            fault: &fault,
        })
        .collect();
    let mut instances =
        Vec::with_capacity(watched_providers.len() + watched_executors.len() + watched_sinks.len());
    for (watch, inst) in watched_providers.iter_mut().zip(&provider_ids) {
        instances.push(SteppedInstance {
            id: shared.first_fragment(*inst),
            inner: SteppedRef::Provider(watch),
        });
    }
    for (watch, inst) in watched_executors.iter_mut().zip(&executor_ids) {
        instances.push(SteppedInstance {
            id: shared.first_fragment(*inst),
            inner: SteppedRef::Executor(watch),
        });
    }
    for (watch, inst) in watched_sinks.iter_mut().zip(&sink_ids) {
        instances.push(SteppedInstance {
            id: shared.first_fragment(*inst),
            inner: SteppedRef::Sink(watch),
        });
    }
    let result = crate::module_api::step_until_quiescent(
        &mut instances,
        at,
        &**collector,
        &super::state::kernel_source(),
    );
    drop(instances);
    drop(watched_sinks);
    drop(watched_executors);
    drop(watched_providers);
    drop(sinks);
    drop(executors);
    drop(providers);
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
        match reaction {
            Reaction::Continue | Reaction::MarkArtifact => {}
            Reaction::Stop => super::ending::request(
                shared,
                Termination::Stopped {
                    cause: StopCause::Policy {
                        kind: event.kind.clone(),
                    },
                },
                CleanupMode::Orderly,
                None,
            ),
            Reaction::Abort => super::ending::request(
                shared,
                Termination::Stopped {
                    cause: StopCause::Policy {
                        kind: event.kind.clone(),
                    },
                },
                CleanupMode::Abort,
                None,
            ),
        }
    }
    lock(&shared.delivered).extend(events);
    if let Some((kind, reaction)) = collector.escalation() {
        let mode = if reaction == Reaction::Abort {
            CleanupMode::Abort
        } else {
            CleanupMode::Orderly
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

pub(super) fn drain_cleanup(shared: &Shared) -> bool {
    let mode = lock(&shared.end).as_ref().map(|end| end.mode);
    if mode != Some(CleanupMode::Orderly)
        || shared
            .drained
            .swap(true, std::sync::atomic::Ordering::AcqRel)
    {
        return false;
    }
    let mut last = None;
    let mut count = 0usize;
    for _ in 0..DRAIN_WAKEUP_CAP {
        if lock(&shared.end)
            .as_ref()
            .is_some_and(|end| end.mode == CleanupMode::Abort)
            || shared.closing.load(std::sync::atomic::Ordering::Acquire)
        {
            break;
        }
        let wakeup = contain_all(|| shared.authority.next_wakeup())
            .ok()
            .flatten();
        if lock(&shared.end)
            .as_ref()
            .is_some_and(|end| end.mode == CleanupMode::Abort)
            || shared.closing.load(std::sync::atomic::Ordering::Acquire)
        {
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
            if let Some(collector) = shared.collector.get() {
                let _ = collector.emit_control(Event {
                    source: super::state::kernel_source(),
                    time: at,
                    severity: crate::event::Severity::Fatal,
                    kind: EventKind::parse(EventKind::STEP_LIVELOCK).expect("Kernel event kind"),
                    payload: serde_json::json!({ "wakeups": crate::module_api::STEP_ROUND_CAP }),
                });
            }
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
                if let Some(collector) = self.shared.collector.get() {
                    let _ = collector.emit_control(Event {
                        source: super::state::kernel_source(),
                        time: at,
                        severity: crate::event::Severity::Fatal,
                        kind: EventKind::parse(EventKind::STEP_LIVELOCK).expect("Kernel event kind"),
                        payload: serde_json::json!({ "wakeups": crate::module_api::STEP_ROUND_CAP }),
                    });
                }
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

    pub(super) fn advance(&mut self, target: TimePoint) -> Result<(), super::RunHandleError> {
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

    pub(super) fn run_until(&mut self, horizon: TimePoint) -> Result<(), super::RunHandleError> {
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

pub(super) fn advance(run: &mut RunHandle, target: TimePoint) -> Result<(), super::RunHandleError> {
    run.advance(target)
}

pub(super) fn run_until(
    run: &mut RunHandle,
    horizon: TimePoint,
) -> Result<(), super::RunHandleError> {
    run.run_until(horizon)
}
