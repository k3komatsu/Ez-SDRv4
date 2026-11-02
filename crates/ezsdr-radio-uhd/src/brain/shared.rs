//! The state the runtime's threads share (stage E2a; test plan §2): one `Mutex<Cell>` per stream behind a counting
//! guard (the I10 oracle), the failure flag `F`, the lifecycle gate `G`, the queues, and the sink the observation reads.

use std::cell::Cell as StdCell;
use std::collections::{BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use ezsdr_radio::timeline::Channels;

use super::cell::{Cell, Terminal};
use super::core::{Brain, Input, RxOp, SetCmd};
use super::force::{Actor, Point, Sched};
use super::rules;
use super::world::{Fact, Params, PubStep, Published};
use crate::device::Device;

thread_local! {
    static HELD: StdCell<usize> = const { StdCell::new(0) };
}

/// The number of cell guards this thread holds (I10: a device method called with a non-zero count is a bug).
pub(crate) fn held_cells() -> usize {
    HELD.with(StdCell::get)
}

/// A cell's guard; the thread-local count follows its lifetime.
pub(crate) struct CellGuard<'a>(MutexGuard<'a, Cell>);

impl<'a> CellGuard<'a> {
    fn new(guard: MutexGuard<'a, Cell>) -> Self {
        HELD.with(|held| held.set(held.get() + 1));
        CellGuard(guard)
    }
}

impl std::ops::Deref for CellGuard<'_> {
    type Target = Cell;
    fn deref(&self) -> &Cell {
        &self.0
    }
}

impl std::ops::DerefMut for CellGuard<'_> {
    fn deref_mut(&mut self) -> &mut Cell {
        &mut self.0
    }
}

impl Drop for CellGuard<'_> {
    fn drop(&mut self) {
        HELD.with(|held| held.set(held.get() - 1));
    }
}

/// The cells moved back into their guards when dropped, so a panic inside the operation restores them (unwind-safe).
struct Swap<'a, 'b> {
    guards: &'a mut Vec<CellGuard<'b>>,
    scratch: Vec<Cell>,
}

impl Drop for Swap<'_, '_> {
    fn drop(&mut self) {
        for (guard, cell) in self.guards.iter_mut().zip(self.scratch.drain(..)) {
            **guard = cell;
        }
    }
}

/// A FIFO between threads. A push wakes the waiters through the scheduler.
pub(crate) struct Queue<T>(Mutex<VecDeque<T>>);

impl<T: Clone> Queue<T> {
    pub(crate) fn new() -> Queue<T> {
        Queue(Mutex::new(VecDeque::new()))
    }
    fn q(&self) -> MutexGuard<'_, VecDeque<T>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub(crate) fn nonempty(&self) -> bool {
        !self.q().is_empty()
    }
    pub(crate) fn pop(&self) -> Option<T> {
        self.q().pop_front()
    }
    pub(crate) fn clear(&self) {
        self.q().clear();
    }
    /// Removes every queued item in one step.
    pub(crate) fn take_all(&self) -> Vec<T> {
        self.q().drain(..).collect()
    }
    pub(crate) fn items(&self) -> Vec<T> {
        self.q().iter().cloned().collect()
    }
}

/// A command for the clock reader: read the time under `L` and report a late command of `stream`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClockCmd {
    Serve { stream: usize },
}

/// A receive executor's own state, behind a mutex so a wait predicate can read it.
#[derive(Clone, Debug)]
pub(crate) struct RxState {
    pub(crate) handle: Option<u64>,
    pub(crate) handles: u64,
    pub(crate) carried: Channels,
    pub(crate) consumed: BTreeSet<u64>,
    pub(crate) flush_end: Option<i64>,
}

/// What the threads record for the observation.
#[derive(Debug, Default)]
pub(crate) struct Sink {
    pub(crate) terminals: Vec<(u64, Terminal)>,
    pub(crate) published: Vec<Published>,
    pub(crate) facts: Vec<Fact>,
    pub(crate) exited: BTreeSet<Actor>,
    /// The order of the failure flag's setting and the loss mark, for S5: `"F"` then `"mark"`.
    pub(crate) order: Vec<&'static str>,
    /// The order of the receive executor's publications and quiesce acknowledgements (S10).
    pub(crate) events: Vec<&'static str>,
    /// Every command the brain routed, in order (the brain's sends, not the device calls they cause).
    pub(crate) routed: Vec<super::core::Command>,
    pub(crate) reservations: Vec<(u64, usize, crate::provider::core::SegmentKey, u64)>,
    /// `(value, instant)` of each acceptance `S` recorded after its call returned (S7 reads when acceptance happened).
    pub(crate) acceptances: Vec<(u64, i64)>,
}

pub(crate) struct Shared {
    pub(crate) sched: Arc<Sched>,
    pub(crate) cells: Vec<Mutex<Cell>>,
    pub(crate) brain: Mutex<Brain>,
    pub(crate) failure: Arc<AtomicBool>,
    pub(crate) gate: Mutex<Option<Actor>>,
    pub(crate) device: Arc<dyn Device>,
    pub(crate) inbox: Queue<Input>,
    pub(crate) s_fifo: Queue<SetCmd>,
    pub(crate) s_terminate: AtomicBool,
    pub(crate) r_fifo: Vec<Queue<RxOp>>,
    pub(crate) rx: Vec<Mutex<RxState>>,
    pub(crate) k_fifo: Queue<ClockCmd>,
    pub(crate) timers: Mutex<Vec<(usize, i64)>>,
    pub(crate) sink: Mutex<Sink>,
    pub(crate) params: Params,
    /// The wall time of one device tick on a fake's manual clock.
    pub(crate) tick: std::time::Duration,
    pub(crate) now: Box<dyn Fn() -> i64 + Send + Sync>,
    /// The effective instant of the device's latest accepted timed command (read only by `Quirk::AcceptAtEffective`).
    pub(crate) effective: Box<dyn Fn() -> i64 + Send + Sync>,
    pub(crate) stopping: AtomicBool,
    /// The brain thread has ended: nothing routes to `S` any more (the closed sender).
    pub(crate) brain_gone: AtomicBool,
    /// The injected real panic of the brain thread (S5): the step at which it panics.
    pub(crate) real_panic: Option<usize>,
    pub(crate) quirk: Quirk,
    /// A test hook run by `S` right after it took the commands it abandons: it lets a test route a command at that instant.
    /// A test injection: the brain operations (counted from 0 over the whole run) that panic inside their cell section.
    pub(crate) panic_in_op: Mutex<Vec<usize>>,
    pub(crate) ops_run: std::sync::atomic::AtomicUsize,
    /// A test hook run by `R` between a publish's computation and its write.
    pub(crate) before_write: Mutex<Option<BatchHook>>,
    pub(crate) after_batch: Mutex<Option<BatchHook>>,
}

/// A deliberate protocol violation for the negative tests: a wrapper that reorders the protocol must fail S1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Quirk {
    None,
    /// `S` sends `Done` before it writes the result.
    SendBeforeResult,
    /// `S` calls the device before it takes the command in the cell.
    CallBeforeTake,
    /// `R` publishes from a snapshot taken at the publish itself (no recheck against the one its recv began with).
    PublishWithoutRecheck,
    /// `S` records a value's acceptance at the instant the device queued its command (the world's `effective`) instead of the time
    /// source read after the call returns: the diagnostic that classifies the Q1 difference.
    AcceptAtEffective,
    /// The acceptance is recorded inside the device's effect, at the effective instant (the world's timing and value): the
    /// diagnostic that classifies the whole Q1 difference.
    AcceptInEffect,
}

/// A test hook run with the shared state.
pub(crate) type BatchHook = Arc<dyn Fn(&Shared) + Send + Sync>;

pub(crate) type Pred = Arc<dyn Fn() -> bool + Send + Sync>;

impl Shared {
    pub(crate) fn lock_cell(&self, k: usize) -> CellGuard<'_> {
        CellGuard::new(self.cells[k].lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Runs `f` on every cell at once, locked in index order. The cells are moved into a scratch vector and back, so the
    /// shared decisions (`&mut [Cell]`) run unchanged; the guards stay held, so the section is atomic. Restoration is
    /// unwind-safe.
    pub(crate) fn with_cells<R>(&self, f: impl FnOnce(&mut [Cell]) -> R) -> R {
        let mut guards: Vec<CellGuard<'_>> = (0..self.cells.len()).map(|k| self.lock_cell(k)).collect();
        let scratch: Vec<Cell> = guards.iter_mut().map(|guard| std::mem::replace(&mut **guard, Cell::vacant())).collect();
        let mut swap = Swap { guards: &mut guards, scratch };
        f(&mut swap.scratch)
    }

    pub(crate) fn with_cell<R>(&self, k: usize, f: impl FnOnce(&mut Cell) -> R) -> R {
        f(&mut self.lock_cell(k))
    }

    pub(crate) fn failed(&self) -> bool {
        self.failure.load(Ordering::SeqCst)
    }

    pub(crate) fn at_end(&self) -> bool {
        self.sched.forcing() && (self.now)() >= self.params.horizon + self.params.epilogue
    }

    pub(crate) fn sink(&self) -> MutexGuard<'_, Sink> {
        self.sink.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn brain(&self) -> MutexGuard<'_, Brain> {
        self.brain.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn rx_state(&self, k: usize) -> MutexGuard<'_, RxState> {
        self.rx[k].lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn push<T: Clone>(&self, queue: &Queue<T>, item: T) {
        queue.q().push_back(item);
        self.sched.notify();
    }

    pub(crate) fn send(&self, input: Input) {
        self.push(&self.inbox, input);
    }

    pub(crate) fn mark_exited(&self, actor: Actor) {
        self.sink().exited.insert(actor);
    }

    /// One abandoned settings command: each cell's section, the terminal, and the `Done` to the brain (I1).
    pub(crate) fn abandon(&self, id: u64) {
        self.with_cells(|cells| rules::abandon_all(cells, id));
        self.sink().terminals.push((id, Terminal::Abandoned));
        self.send(Input::Done { id, terminal: Terminal::Abandoned });
    }

    pub(crate) fn pred(self: &Arc<Self>, f: impl Fn(&Shared) -> bool + Send + Sync + 'static) -> Pred {
        let shared = self.clone();
        Arc::new(move || f(&shared))
    }

    fn gate_free(&self) -> bool {
        self.gate.lock().unwrap_or_else(|e| e.into_inner()).is_none()
    }

    /// Takes `G` (a permit point, then the take).
    pub(crate) fn gate_acquire(self: &Arc<Self>, actor: Actor, point: Point) {
        let shared = self.clone();
        self.sched.acquire(actor, point, self.pred(|s| s.gate_free()), move || *shared.gate.lock().unwrap_or_else(|e| e.into_inner()) = Some(actor));
    }

    /// Releases `G` (after the point's permit).
    pub(crate) fn gate_release(&self, actor: Actor, point: Point) {
        self.sched.hook(actor, point);
        let mut gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(*gate, Some(actor), "{actor:?} releases a gate it does not hold");
        *gate = None;
        drop(gate);
        self.sched.notify();
    }

    /// Releases `G` inside the step that took it (no event of its own: a refused recheck, R.gate in the world).
    pub(crate) fn gate_drop(&self) {
        *self.gate.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.sched.notify();
    }

    /// One step of the loss publication (note 45 §4.5 procedure 4): take `G`, set `F`, release `G`, call `mark_lost`. `point`
    /// names the event of each step (`Pub(step)` for an executor, `BOp` for the brain's guard).
    pub(crate) fn publish_step(self: &Arc<Self>, actor: Actor, step: PubStep, point: Point) {
        match step {
            PubStep::Gate => self.gate_acquire(actor, point),
            PubStep::Set => {
                self.sched.hook(actor, point);
                self.failure.store(true, Ordering::SeqCst);
                self.sink().order.push("F");
                self.sched.notify();
            }
            PubStep::Ungate => self.gate_release(actor, point),
            PubStep::Mark => {
                self.sched.hook(actor, point);
                self.sink().order.push("mark");
                self.device.mark_lost();
            }
        }
    }
}
