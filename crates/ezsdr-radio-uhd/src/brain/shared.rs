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
pub(crate) struct CellGuard<'a>(MutexGuard<'a, Cell>, Option<std::time::Instant>);

impl<'a> CellGuard<'a> {
    fn new(guard: MutexGuard<'a, Cell>, timed: bool) -> Self {
        HELD.with(|held| held.set(held.get() + 1));
        CellGuard(guard, timed.then(std::time::Instant::now))
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
        if let Some(since) = self.1 {
            super::speed::CELL_HOLD.record(since.elapsed());
        }
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

/// A FIFO between threads, read by one actor: a push wakes that actor (everyone, outside a targeted run) through the scheduler.
pub(crate) struct Queue<T> {
    /// The items and the number of registered senders, behind one lock: "empty and no sender left" is one read.
    items: Mutex<(VecDeque<T>, usize)>,
    owner: Actor,
}

impl<T: Clone> Queue<T> {
    pub(crate) fn new(owner: Actor) -> Queue<T> {
        Queue { items: Mutex::new((VecDeque::new(), 0)), owner }
    }
    pub(crate) fn nonempty(&self) -> bool {
        !self.items.lock().unwrap_or_else(|e| e.into_inner()).0.is_empty()
    }
    pub(crate) fn pop(&self) -> Option<T> {
        self.items.lock().unwrap_or_else(|e| e.into_inner()).0.pop_front()
    }
    pub(crate) fn clear(&self) {
        self.items.lock().unwrap_or_else(|e| e.into_inner()).0.clear();
    }
    /// Removes every queued item in one step.
    pub(crate) fn take_all(&self) -> Vec<T> {
        self.items.lock().unwrap_or_else(|e| e.into_inner()).0.drain(..).collect()
    }
    pub(crate) fn items(&self) -> Vec<T> {
        self.items.lock().unwrap_or_else(|e| e.into_inner()).0.iter().cloned().collect()
    }
    /// A thread that can send to this queue registers before it starts.
    pub(crate) fn register(&self) {
        self.items.lock().unwrap_or_else(|e| e.into_inner()).1 += 1;
    }
    /// Nothing is queued and no registered sender is left, read together: nothing can arrive any more.
    pub(crate) fn drained(&self) -> bool {
        let g = self.items.lock().unwrap_or_else(|e| e.into_inner());
        g.0.is_empty() && g.1 == 0
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
    pub(crate) timers: Mutex<Vec<(super::core::Timer, i64)>>,
    pub(crate) sink: Mutex<Sink>,
    pub(crate) params: Params,
    /// The wall time of one device tick on a fake's manual clock.
    pub(crate) tick: std::time::Duration,
    pub(crate) now: Box<dyn Fn() -> i64 + Send + Sync>,
    pub(crate) stopping: AtomicBool,
    /// The brain has terminated (stop model, free running): the clock reader and the timer start no more work and return after the send they have begun.
    pub(crate) halt_senders: AtomicBool,
    /// The brain thread returns once it has terminated with nothing left to take (and at once after its guard), at a `BExit` event under
    /// forcing: the run models the provider's join of the decision thread (set when the world has the stop model).
    pub(crate) exit_model: bool,
    /// The brain thread is told to return (the `BExit` event under forcing).
    pub(crate) brain_exit: AtomicBool,
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
    /// A test's hold on a free run: called at each [`Gate`] a thread passes (a closure that blocks holds that thread there).
    pub(crate) gate_hook: Mutex<Option<GateHook>>,
    /// E2b's measurement hooks: the receive work and the counters (absent outside a measurement run).
    pub(crate) bench: Option<Arc<super::speed::Bench>>,
    /// Device ticks between two samples of a receive stream (the decimation): the receive executor turns a block into ticks with it.
    pub(crate) sample_ticks: i64,
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
}

/// A test hook run with the shared state.
pub(crate) type BatchHook = Arc<dyn Fn(&Shared) + Send + Sync>;

/// The places of the stop's last hand-offs a free run can be held at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Gate {
    /// A sender's last send is about to be made (a receive executor's final `Lost`, the clock's read time, the timer's removed booking).
    FinalSend(Actor),
    /// A sender has made its last send and is about to deregister from the brain's inbox.
    Deregister(Actor),
    /// The builder is about to register this sender with the brain's inbox.
    Registering(Actor),
    /// The brain's loop top, before it tells the clock and the timer to stop.
    BrainTop,
    /// The brain's step is over and it is about to test whether it may return.
    ExitTest,
}
pub(crate) type GateHook = Arc<dyn Fn(&Shared, Gate) + Send + Sync>;

pub(crate) type Pred = Arc<dyn Fn() -> bool + Send + Sync>;

impl Shared {
    pub(crate) fn lock_cell(&self, k: usize) -> CellGuard<'_> {
        let timed = super::speed::instrument_on();
        let asked = timed.then(std::time::Instant::now);
        let guard = self.cells[k].lock().unwrap_or_else(|e| e.into_inner());
        if let Some(asked) = asked {
            super::speed::CELL_WAIT.record(asked.elapsed());
        }
        CellGuard::new(guard, timed)
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
        queue.items.lock().unwrap_or_else(|e| e.into_inner()).0.push_back(item);
        self.sched.wake(queue.owner);
    }

    pub(crate) fn send(&self, input: Input) {
        self.push(&self.inbox, input);
    }

    /// Free running with the stop model: the brain has terminated and the inbox is empty with no registered sender left, read together under the
    /// inbox's lock, so its receiver reports a closed channel and its thread returns (under forcing the trace's `BExit` says so instead).
    pub(crate) fn brain_may_exit(&self) -> bool {
        self.exit_model && !self.sched.forcing() && self.brain().exit == super::core::Exit::Terminated && self.inbox.drained()
    }

    /// A sending thread's registration with the brain's inbox; it ends the registration when dropped, that is when the thread returns after its
    /// final send (also by an unwind).
    pub(crate) fn sender(self: &Arc<Self>, actor: Actor) -> SenderGuard {
        self.gate(Gate::Registering(actor));
        self.inbox.register();
        SenderGuard(self.clone(), actor)
    }

    /// Passes `gate`: a test's hook may hold the calling thread here.
    pub(crate) fn gate(&self, gate: Gate) {
        let hook = self.gate_hook.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(hook) = hook {
            hook(self, gate);
        }
    }

    /// The thread has completed: it returned, after its last send and its deregistration (S, a receive executor), or the brain's returns. This is
    /// what a join waits for; a loop that exited on a failure is not completion (a discarder may still run).
    pub(crate) fn mark_exited(&self, actor: Actor) {
        self.sink().exited.insert(actor);
    }

    /// One abandoned settings command: each cell's section, the terminal, and the `Done` to the brain (I1).
    pub(crate) fn abandon(&self, id: u64) {
        self.with_cells(|cells| rules::abandon_all(cells, id));
        self.sink().terminals.push((id, Terminal::Abandoned));
        self.send(Input::Done { id, terminal: Terminal::Abandoned, now: (self.now)() });
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
            PubStep::Record => {
                self.sched.hook(actor, point);
                self.sink().order.push("record");
            }
            PubStep::Mark => {
                self.sched.hook(actor, point);
                self.sink().order.push("mark");
                self.device.mark_lost();
            }
        }
    }
}

/// A registered sender of the brain's inbox. Dropping it deregisters (the decrement is under the inbox's lock) and wakes the brain's own wait: the
/// waiter evaluates its predicate under the scheduler's lock and this wake takes that lock after the decrement, so no wake-up is lost.
pub(crate) struct SenderGuard(Arc<Shared>, Actor);

impl Drop for SenderGuard {
    fn drop(&mut self) {
        self.0.gate(Gate::Deregister(self.1));
        self.0.inbox.items.lock().unwrap_or_else(|e| e.into_inner()).1 -= 1;
        self.0.sched.wake(self.0.inbox.owner);
    }
}
