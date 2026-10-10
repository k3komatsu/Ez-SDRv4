//! What only the device-paced classes run: the data thread (KC-46…KC-46b), its wake
//! (KC-46a), the wait for threaded instances (KC-21a) and the bounded call (KC-12a).
//! A Simulation Run never reaches this module, which is why PO-11 exempts it by name:
//! it spawns threads and reads the host clock (GZ-4).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::module_api::{ModuleError, ModuleErrorKind, Role, StepOutcome};
use crate::run::{CleanupFailure, CleanupMode, CleanupOps, CleanupStep, StopCause, Termination};
use crate::time::TimePoint;

use super::DEFAULT_HOST_BUDGET_NS;
use super::state::{Inst, Shared, contain, lock};

/// How long the data thread parks after a pass in which nothing progressed (KC-46).
const DATA_IDLE_PARK: Duration = Duration::from_micros(200);
/// How often KC-21a's wait looks again at an end or a freeze.
const RECHECK: Duration = Duration::from_millis(10);

fn host_budget() -> Duration {
    Duration::from_nanos(DEFAULT_HOST_BUDGET_NS as u64)
}

pub(super) struct DataThread {
    stop: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

/// Starts the data thread as the Run enters `Running` (KC-46).
pub(super) fn start_data_thread(shared: &Arc<Shared>) {
    let stop = Arc::new(AtomicBool::new(false));
    let (thread_shared, thread_stop) = (shared.clone(), stop.clone());
    match std::thread::Builder::new()
        .name("ezsdr-data".to_owned())
        .spawn(move || run(thread_shared, thread_stop))
    {
        Ok(handle) => *lock(&shared.data) = Some(DataThread { stop, handle }),
        Err(error) => {
            super::ending::request(
                shared,
                Termination::Failed { stage: crate::run::Stage::Run, reason: format!("KC-46: the data thread could not start: {error}") },
                CleanupMode::Abort,
            );
        }
    }
}

/// Stops and joins the data thread; it never joins itself (KA-12 step 3, KC-46c).
pub(super) fn stop_data_thread(shared: &Shared) {
    let Some(data) = lock(&shared.data).take() else {
        return;
    };
    if data.handle.thread().id() == std::thread::current().id() {
        return;
    }
    data.stop.store(true, Ordering::Release);
    data.handle.thread().unpark();
    let _ = data.handle.join();
}

fn run(shared: Arc<Shared>, stop: Arc<AtomicBool>) {
    let mut failed = BTreeSet::new();
    loop {
        // The last holder: a handle dropped without cleanup (KC-46c's backstop).
        if stop.load(Ordering::Acquire) || Arc::strong_count(&shared) == 1 {
            return;
        }
        if shared.stopped_early.load(Ordering::Acquire) && end_mode(&shared) == Some(CleanupMode::Abort) {
            return;
        }
        let (progressed, mut requested, delivered) = pass(&shared, &mut failed);
        let deadline = *lock(&shared.lease_deadline);
        if deadline.is_some_and(|at| shared.ctx.host_clock.monotonic_millis() >= at) {
            requested |= super::ending::request(
                &shared,
                Termination::Stopped { cause: StopCause::LeaseExpiry {} },
                CleanupMode::Orderly,
            );
        }
        if requested && !shared.stopped_early.swap(true, Ordering::AcqRel) {
            stop_early(&shared);
        } else if delivered > 0 {
            wake(&shared, false);
        }
        if !progressed {
            std::thread::park_timeout(DATA_IDLE_PARK);
        }
    }
}

fn end_mode(shared: &Shared) -> Option<CleanupMode> {
    lock(&shared.end).as_ref().map(|end| end.mode)
}

/// One pass: every Executor and Sink stepped once at one instant, in MA-30's order,
/// then the drain (KC-46). Returns whether any instance progressed, whether this
/// thread requested an end, and how many events the drain delivered.
fn pass(shared: &Shared, failed: &mut BTreeSet<Inst>) -> (bool, bool, usize) {
    let t = shared.now();
    let mut order: Vec<(u8, crate::spec::Ident, Inst)> = (0..shared.executors.len())
        .map(|i| (Role::Executor.step_rank(), Inst::Executor(i)))
        .chain((0..shared.sinks.len()).map(|i| (Role::Sink.step_rank(), Inst::Sink(i))))
        .map(|(rank, inst)| (rank, shared.first_fragment(inst), inst))
        .collect();
    order.sort();
    let restored = CleanupStep::RestoreBaseline as u8;
    let mut progressed = false;
    let mut faults = Vec::new();
    for (_, _, inst) in order {
        if failed.contains(&inst) || lock(&shared.done).contains(&(restored, inst)) {
            continue;
        }
        match step(shared, inst, t) {
            Ok(outcome) => progressed |= outcome.progressed,
            Err(error) => {
                failed.insert(inst);
                faults.push((inst, error));
            }
        }
    }
    let requested = super::stepping::apply_faults(shared, &faults);
    let (delivered, reacted) = super::stepping::drain_and_react(shared);
    (progressed, requested || reacted, delivered)
}

fn step(shared: &Shared, inst: Inst, t: TimePoint) -> Result<StepOutcome, ModuleError> {
    match inst {
        Inst::Executor(i) => {
            let mut guard = lock(&shared.executors[i].object);
            contain(|| guard.step(t))
        }
        Inst::Sink(i) => {
            let mut guard = lock(&shared.sinks[i].object);
            contain(|| guard.step(t))
        }
        Inst::Provider(_) => Ok(StepOutcome { progressed: false }),
    }
}

/// RS-6 steps 1 and 2 on the data thread, once, after it requested an end (KC-46b).
fn stop_early(shared: &Arc<Shared>) {
    let ops = super::ending::Ops::new(shared.clone());
    let mode = end_mode(shared).unwrap_or(CleanupMode::Orderly);
    let record = |step: CleanupStep, fragment: Option<&crate::spec::Ident>| {
        if let Err(error) = ops.perform(step, fragment, mode) {
            lock(&shared.cleanup_failures).push(CleanupFailure {
                step,
                fragment: fragment.cloned(),
                reason: error.message,
                timed_out: false,
            });
        }
    };
    record(CleanupStep::FreezeDispatch, None);
    if let Some(routing) = shared.routing() {
        for fragment in &routing.reverse {
            record(CleanupStep::StopProviders, Some(fragment));
        }
    }
    wake(shared, true);
}

/// Wakes a control loop waiting in `next_wakeup` with a no-op at the current instant,
/// unless a wake is still pending; pending wakes are counted by generation (KC-46a).
pub(super) fn wake(shared: &Shared, unconditional: bool) {
    if !unconditional && shared.wake_pending.load(Ordering::Acquire) != 0 {
        return;
    }
    let generation = shared.wake_next.fetch_add(1, Ordering::AcqRel) + 1;
    shared.wake_pending.store(generation, Ordering::Release);
    let pending = shared.wake_pending.clone();
    let clear = move |_| {
        let _ = pending.compare_exchange(generation, 0, Ordering::AcqRel, Ordering::Acquire);
    };
    let at = shared.now();
    // Held across `schedule`, so that a concurrent FreezeDispatch cancels this wake
    // rather than miss it (RS-6 step 1; Review L, NONBLOCKING 1).
    let mut slot = lock(&shared.wake_handle);
    match shared.schedule(at, Box::new(clear)) {
        Ok(handle) => *slot = Some(handle),
        Err(_) => {
            let _ = shared.wake_pending.compare_exchange(
                generation,
                0,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        }
    }
}

/// KC-21a: waits until every instance dispatched to has finished with its Actions,
/// for at most the host budget, and fails the Run if one has not.
pub(super) fn wait_finished(shared: &Shared, dispatched: &[(Inst, u64)]) {
    let mut targets: BTreeMap<Inst, u64> = BTreeMap::new();
    for (inst, pushed) in dispatched {
        let target = targets.entry(*inst).or_default();
        *target = (*target).max(*pushed);
    }
    let deadline = Instant::now() + host_budget();
    for (inst, target) in targets {
        loop {
            if lock(&shared.end).is_some() || shared.frozen.load(Ordering::Acquire) {
                return;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                super::ending::request(
                    shared,
                    Termination::Failed { stage: crate::run::Stage::Run, reason: format!(
                        "KC-21a: {} did not finish its Actions within {} ms",
                        shared.first_fragment(inst),
                        DEFAULT_HOST_BUDGET_NS / 1_000_000
                    ) },
                    CleanupMode::Abort,
                );
                return;
            }
            if shared.queue(inst).wait_finished_for(target, remaining.min(RECHECK)) {
                break;
            }
        }
    }
}

/// MA-8 enforced (KC-12a): `f` on a worker thread, waited for at most the host
/// budget. `None` means it did not return in time and was abandoned.
pub(super) fn bounded<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, ModuleError> + Send + 'static,
) -> Option<Result<T, ModuleError>> {
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("ezsdr-bounded".to_owned())
        .spawn(move || {
            let _ = tx.send(contain(f));
        });
    if let Err(error) = spawned {
        return Some(Err(ModuleError {
            kind: ModuleErrorKind::Internal,
            message: format!("KC-12a: no worker thread: {error}"),
            detail: serde_json::Value::Null,
        }));
    }
    match rx.recv_timeout(host_budget()) {
        Ok(result) => Some(result),
        Err(mpsc::RecvTimeoutError::Timeout) => None,
        Err(mpsc::RecvTimeoutError::Disconnected) => Some(Err(ModuleError {
            kind: ModuleErrorKind::Internal,
            message: "a Module panicked".to_owned(),
            detail: serde_json::Value::Null,
        })),
    }
}
