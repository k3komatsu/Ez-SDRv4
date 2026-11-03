//! The brain `B` (note 45 §3–§4): one `step` per input queues its operations, and each
//! operation is one cell section or one send. A section's result decides the operations it
//! queues, so a send is never decided before the section that gates it (E1-63).

use std::collections::VecDeque;

use super::cell::{Cell, Drain, InFlight, Outcome, Outstanding, Phase, Refusal, Terminal, ValueState};
use crate::provider::core::RxEntry;

/// The drain window: the drain deadline is the cut plus this (note 45 §4.4).
pub(crate) const DRAIN_WINDOW: i64 = 8;

/// The device's capacity K (I11′; note 45 §4.5 「上限は二つ」): values counted against the device's command queue, over all chains.
pub(crate) const DEVICE_BOUND: usize = 4;

/// The resident bound D of a chain: every value in its cell, from reservation until its terminal input and its obligations are over.
pub(crate) const CHAIN_BOUND: usize = 64;

/// The first id the brain numbers its own commands with; a value's id is its entry's order id.
const FIRST_ID: u64 = 1 << 32;

/// A value the brain holds, not yet reserved in a cell.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Held {
    pub(crate) stream: usize,
    pub(crate) entry: RxEntry,
    pub(crate) generation: u64,
}

/// Why a released value is reported, not consumed or cancelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reason {
    Left,
    Loss,
    NoReservation,
    NoChannel,
}

/// The one disposition of a released value (I4, P2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Disposition {
    Consumed,
    Cancelled,
    Refused,
    Reported(Reason),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Change {
    Timed { channel: usize, freq: f64, at: i64, generation: u64 },
    Restore { channel: usize },
    Move { channel: usize, to: usize },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SetCmd {
    pub(crate) id: u64,
    pub(crate) change: Change,
}

/// A receive stream command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RxOp {
    Start {
        at: i64,
        /// The epoch the start belongs to: a start of the new epoch waits for the old one's quiesce ack (I12).
        epoch: u64,
    },
    Stop {
        at: Option<i64>,
        /// The epoch the stop ends: a Finished for an older epoch leaves the cell alone.
        epoch: u64,
    },
    Flush,
    Open,
    Close,
    /// A plan change: R stops the old epoch's stream, then acknowledges the quiesce (note 45 §4.5 procedure 3).
    /// `retiring` is the epoch the change ends; its Finished boundary carries it.
    Plan {
        epoch: u64,
        retiring: u64,
    },
    Terminate,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Command {
    Settings(SetCmd),
    Rx {
        stream: usize,
        op: RxOp,
    },
    Timer {
        stream: usize,
        at: i64,
    },
    /// The capacity timer: a device-time instant at which a counted value frees K (a kind of its own, never a drain deadline).
    CapacityTimer {
        at: i64,
    },
    /// Shutdown's terminate for `S`: `S` takes the commands queued before it, then exits.
    SettingsTerminate,
}

/// A booked timer: a stream's drain deadline, or the device's capacity timer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Timer {
    Drain(usize),
    Capacity,
}

/// A lifecycle or settings request the brain may defer until the chain is clear.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Request {
    Restore {
        channel: usize,
    },
    Move {
        channel: usize,
        to: usize,
    },
    Close {
        stream: usize,
    },
    Open {
        stream: usize,
    },
    StartNext {
        stream: usize,
        at: i64,
    },
    Flush {
        stream: usize,
    },
    /// A plan change's stop and quiesce, deferred while another lifecycle command is outstanding.
    Plan {
        stream: usize,
        epoch: u64,
        retiring: u64,
    },
    /// A late completion's restart or end-now stop, deferred while a lifecycle command is outstanding.
    Late {
        stream: usize,
        now: i64,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Boundary {
    Consumed {
        id: u64,
    },
    /// Sent by the receive executor when a publish reaches the cut.
    CutReached {
        stream: usize,
    },
    Finished {
        stream: usize,
        next_origin: Option<i64>,
        /// The epoch of the segment that finished.
        epoch: u64,
    },
    /// Sent by the receive executor at the first published sample of a stream.
    FirstSample {
        stream: usize,
    },
    Armed {
        stream: usize,
    },
    /// Sent by the E1b receive executor once the old epoch's recv has published (I6).
    Quiesced {
        stream: usize,
    },
    /// The receive executor's loss publication: its device is lost.
    Lost {
        stream: usize,
    },
    /// Sent by the receive executor when a flush's start or stop has returned, unless it was lost.
    Flushed {
        stream: usize,
    },
    /// Sent by the receive executor when a start's call failed (a failed begin keeps the phase).
    BeginFailed {
        stream: usize,
    },
    /// Sent by the receive executor when an open or a close has returned, whatever its result.
    Opened {
        stream: usize,
    },
    Closed {
        stream: usize,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Input {
    Release(Held),
    Stop {
        stream: usize,
        requested: i64,
    },
    /// A plan change: the epoch moves on, and the start at `origin` waits for the quiesce (note 45 §4.5 procedure 3).
    PlanCancel {
        stream: usize,
        origin: i64,
        epoch: u64,
        plan_cut: Option<i64>,
    },
    Request(Request),
    /// A settings command's terminal; `now` is the device time its sender read (the brain's only source of time besides the timers).
    Done {
        id: u64,
        terminal: Terminal,
        now: i64,
    },
    Boundary(Boundary),
    /// A drain deadline; `now` is the device time read at delivery (never before the deadline).
    Timer {
        stream: usize,
        now: i64,
    },
    /// A capacity timer: only re-evaluates the held values.
    CapacityTimer {
        now: i64,
    },
    LateCommand {
        stream: usize,
        now: i64,
    },
    Shutdown,
    Panic,
    /// The runtime failed to spawn the executor of `stream` (or, after a failure, did not spawn it at all): `started` is whether its `rx_start`
    /// had succeeded (note 45 §4.9). The brain sends that stream no command and waits for nothing from it.
    SpawnFailed {
        stream: usize,
        started: bool,
    },
    /// The start failed before the control thread (`S`) was spawned: it does not exist, and nothing is sent to it.
    ControlUnspawned,
}

/// The guard's events of a panic, in order: the failure flag before the loss is published (I1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Guard {
    Gate,
    FlagSet,
    Ungate,
    MarkLost,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Section {
    Reserve {
        id: u64,
    },
    /// Tries the head of the held list; each success queues the next pass (head-of-line, experiment §11 "Held order").
    ReserveAll,
    Stop {
        stream: usize,
        requested: i64,
    },
    PlanCancel {
        stream: usize,
        origin: i64,
        epoch: u64,
        plan_cut: Option<i64>,
    },
    Drain {
        stream: usize,
    },
    TimerFired {
        stream: usize,
    },
    CapacityTimer,
    Done {
        id: u64,
        terminal: Terminal,
    },
    Consumed {
        id: u64,
    },
    Finished {
        stream: usize,
        next_origin: Option<i64>,
        epoch: u64,
    },
    Armed {
        stream: usize,
    },
    FirstSample {
        stream: usize,
    },
    Quiesced {
        stream: usize,
    },
    Lost {
        stream: usize,
    },
    Flushed {
        stream: usize,
    },
    BeginFailed {
        stream: usize,
    },
    Opened {
        stream: usize,
    },
    Closed {
        stream: usize,
    },
    Decide(Request),
    Late {
        stream: usize,
        now: i64,
    },
    Shutdown,
    SpawnFailed {
        stream: usize,
        started: bool,
    },
    ControlUnspawned,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Op {
    Section(Section),
    Send(Command),
    Guard(Guard),
}

#[derive(Debug, PartialEq)]
enum Decision {
    Go(Command),
    Wait,
    Never,
}

/// The shutdown's progress. The terminates go last: after the shutdown's stops and their flushes, and
/// once no deferred request waits (the provider's stop order, provider/rx.rs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Exit {
    Running,
    ShuttingDown,
    Terminated,
    /// The brain panicked and its guard has run: it takes no further input (note 45 §4.5 procedure 4).
    Dead,
}

#[derive(Clone, Debug)]
pub(crate) struct Brain {
    pub(crate) held: Vec<Held>,
    pub(crate) dispositions: Vec<(u64, Disposition)>,
    pub(crate) results: Vec<(u64, Terminal)>,
    pub(crate) leftovers: Vec<u64>,
    pub(crate) outbox: Vec<Command>,
    pub(crate) deferred: Vec<Request>,
    pub(crate) ops: VecDeque<Op>,
    pub(crate) next_id: u64,
    /// The drain deadline is the cut plus this (the experiment's cases set it).
    pub(crate) drain_window: i64,
    pub(crate) guard: Vec<Guard>,
    pub(crate) exit: Exit,
    /// The Restore and Move commands sent and not yet done, by command id: the channel each changes (written in `decide`, cleared at `Done`).
    pub(crate) pending: std::collections::BTreeMap<u64, usize>,
    /// K and D this brain applies: [`DEVICE_BOUND`] and [`CHAIN_BOUND`] unless a case or the E2b diagnostic sets them (from `Params` in the world).
    pub(crate) device_bound: usize,
    pub(crate) chain_bound: usize,
    /// The latest device time an input carried (`i64::MIN` before any): never lowered by a stale input. Nothing else is read as time.
    pub(crate) now: i64,
    /// The instant of the capacity timer last booked and not yet delivered (timer bookkeeping, not part of K or D).
    pub(crate) capacity_timer: Option<i64>,
    /// The streams whose executor was never spawned (an input-delivered fact like `exit`): nothing is sent to them.
    pub(crate) unspawned: std::collections::BTreeSet<usize>,
    /// The control thread `S` was never spawned.
    pub(crate) control_unspawned: bool,
}

impl Brain {
    pub(crate) fn new() -> Brain {
        Brain {
            held: Vec::new(),
            dispositions: Vec::new(),
            results: Vec::new(),
            leftovers: Vec::new(),
            outbox: Vec::new(),
            deferred: Vec::new(),
            ops: VecDeque::new(),
            next_id: FIRST_ID,
            drain_window: DRAIN_WINDOW,
            guard: Vec::new(),
            exit: Exit::Running,
            pending: std::collections::BTreeMap::new(),
            device_bound: DEVICE_BOUND,
            chain_bound: CHAIN_BOUND,
            now: i64::MIN,
            capacity_timer: None,
            unspawned: std::collections::BTreeSet::new(),
            control_unspawned: false,
        }
    }

    /// Queues the operations of one input; nothing runs until [`Brain::run`] or [`Brain::op`].
    pub(crate) fn step(&mut self, input: Input) {
        assert!(!self.dead(), "a dead brain takes no input");
        match input {
            Input::Release(held) if self.unspawned.contains(&held.stream) => {
                // No executor will ever run this stream: the value is disposed at once, once.
                self.dispose(held.entry.order.1, Disposition::Reported(Reason::NoReservation));
            }
            Input::Release(held) => {
                self.held.push(held);
                // A pass over the held list starts at its head: a release never jumps ahead of a value that waits.
                let head = self.held[0].entry.order.1;
                self.ops.push_back(Op::Section(Section::Reserve { id: head }));
            }
            Input::Stop { stream, requested } => self.ops.push_back(Op::Section(Section::Stop { stream, requested })),
            Input::PlanCancel { stream, origin, epoch, plan_cut } => self.ops.push_back(Op::Section(Section::PlanCancel { stream, origin, epoch, plan_cut })),
            Input::Request(request) => self.ops.push_back(Op::Section(Section::Decide(request))),
            Input::Done { id, terminal, now } => {
                self.see(now);
                self.ops.push_back(Op::Section(Section::Done { id, terminal }));
            }
            Input::Boundary(boundary) => self.ops.push_back(Op::Section(match boundary {
                Boundary::Consumed { id } => Section::Consumed { id },
                Boundary::CutReached { stream } => Section::Drain { stream },
                Boundary::Finished { stream, next_origin, epoch } => Section::Finished { stream, next_origin, epoch },
                Boundary::FirstSample { stream } => Section::FirstSample { stream },
                Boundary::Armed { stream } => Section::Armed { stream },
                Boundary::Quiesced { stream } => Section::Quiesced { stream },
                Boundary::Lost { stream } => Section::Lost { stream },
                Boundary::Flushed { stream } => Section::Flushed { stream },
                Boundary::BeginFailed { stream } => Section::BeginFailed { stream },
                Boundary::Opened { stream } => Section::Opened { stream },
                Boundary::Closed { stream } => Section::Closed { stream },
            })),
            Input::Timer { stream, now } => {
                self.see(now);
                self.ops.push_back(Op::Section(Section::TimerFired { stream }));
            }
            Input::CapacityTimer { now } => {
                self.see(now);
                self.capacity_timer = None;
                self.ops.push_back(Op::Section(Section::CapacityTimer));
            }
            Input::LateCommand { stream, now } => {
                self.see(now);
                self.ops.push_back(Op::Section(Section::Late { stream, now }));
            }
            Input::Shutdown => self.ops.push_back(Op::Section(Section::Shutdown)),
            Input::SpawnFailed { stream, started } => self.ops.push_back(Op::Section(Section::SpawnFailed { stream, started })),
            Input::ControlUnspawned => self.ops.push_back(Op::Section(Section::ControlUnspawned)),
            Input::Panic => {
                for guard in [Guard::Gate, Guard::FlagSet, Guard::Ungate, Guard::MarkLost] {
                    self.ops.push_back(Op::Guard(guard));
                }
            }
        }
    }

    /// A time an input carried: the brain's device time only moves forward.
    fn see(&mut self, now: i64) {
        self.now = self.now.max(now);
    }

    /// The brain panicked and its guard has run.
    pub(crate) fn dead(&self) -> bool {
        self.exit == Exit::Dead
    }

    /// Runs one queued operation; false when none is queued.
    pub(crate) fn op(&mut self, cells: &mut [Cell]) -> bool {
        let Some(op) = self.ops.pop_front() else { return false };
        match op {
            Op::Send(command) => self.outbox.push(command),
            Op::Guard(guard) => {
                self.guard.push(guard);
                if guard == Guard::MarkLost {
                    self.exit = Exit::Dead;
                }
            }
            Op::Section(section) => self.section(section, cells),
        }
        true
    }

    pub(crate) fn run(&mut self, cells: &mut [Cell]) {
        while self.op(cells) {}
    }

    fn front(&mut self, ops: Vec<Op>) {
        for op in ops.into_iter().rev() {
            self.ops.push_front(op);
        }
    }

    fn fresh_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub(crate) fn disposition(&self, id: u64) -> Option<Disposition> {
        self.dispositions.iter().find(|(value, _)| *value == id).map(|(_, disposition)| *disposition)
    }

    /// One disposition per value: the first one written stands.
    fn dispose(&mut self, id: u64, disposition: Disposition) {
        if self.disposition(id).is_none() {
            self.dispositions.push((id, disposition));
        }
    }

    /// Whether the brain has taken the value's terminal input.
    fn has_result(&self, id: u64) -> bool {
        self.results.iter().any(|(done, _)| *done == id)
    }

    /// Whether the value's consumption has been seen (as debt's, or as left's).
    fn consumed(&self, id: u64) -> bool {
        self.disposition(id) == Some(Disposition::Consumed) || self.leftovers.contains(&id)
    }

    /// Runs a section; any section that can free a bound or open a reservation condition is followed by a re-evaluation of the held values
    /// (note 45 §4.5): every section but a reservation itself and the shutdown's disposal. A request's decision is included: the stop it
    /// sends cancels queued values, which frees K.
    fn section(&mut self, section: Section, cells: &mut [Cell]) {
        let reevaluates = !matches!(section, Section::Reserve { .. } | Section::ReserveAll | Section::Shutdown);
        self.section_body(section, cells);
        if reevaluates {
            self.reevaluate();
        }
    }

    /// Queues one pass over the held values after the step's other operations (once).
    fn reevaluate(&mut self) {
        if !self.held.is_empty() && !self.ops.contains(&Op::Section(Section::ReserveAll)) {
            self.ops.push_back(Op::Section(Section::ReserveAll));
        }
    }

    fn section_body(&mut self, section: Section, cells: &mut [Cell]) {
        match section {
            Section::Reserve { id } => self.reserve(id, cells),
            Section::ReserveAll => {
                if let Some(head) = self.held.first() {
                    let id = head.entry.order.1;
                    self.front(vec![Op::Section(Section::Reserve { id })]);
                }
            }
            Section::CapacityTimer => {}
            Section::ControlUnspawned => {
                self.control_unspawned = true;
                self.recheck(cells);
            }
            Section::SpawnFailed { stream, started } => {
                self.unspawned.insert(stream);
                // Its held values will never be reserved; the chain is abandoned only when its start succeeded (`abandon_unowned`: the device
                // state is uncertain, a loss already known is kept, no segment, the stop is an abort).
                for held in std::mem::take(&mut self.held) {
                    if held.stream == stream {
                        self.dispose(held.entry.order.1, Disposition::Reported(Reason::NoReservation));
                    } else {
                        self.held.push(held);
                    }
                }
                if started {
                    if cells[stream].phase != Phase::Lost {
                        cells[stream].phase = Phase::Uncertain;
                    }
                    cells[stream].outstanding = None;
                    cells[stream].drain = Drain::Ended;
                }
                self.recheck(cells);
            }
            Section::Stop { stream, requested } => {
                let cell = &mut cells[stream];
                match cell.phase {
                    // A lost stream publishes its loss instead; a stop decided in `Stopping` is in force already.
                    Phase::Stopping(_) | Phase::Lost => {}
                    Phase::Armed(_) | Phase::Streaming(_) => {
                        cell.drain = Drain::Idle;
                        cell.book_stop(requested);
                        self.front(vec![Op::Section(Section::Drain { stream })]);
                    }
                    // The cut is plan intent on the chain (provider/core.rs `request_cut`): it holds until the start is confirmed.
                    _ => {
                        cell.drain = Drain::Booked;
                        cell.book_stop(requested);
                    }
                }
            }
            // shortcut: a second PlanCancel before the quiesce ack overwrites Cell::retiring, defer it until the ack once a case scripts two
            Section::PlanCancel { stream, origin, epoch, plan_cut } => {
                let cell = &mut cells[stream];
                let retiring = cell.epoch;
                // R stops the stream itself, so the drain is over; acceptance closes (the Stopping transition).
                cell.drain = Drain::Ended;
                cell.stop_accepting();
                cell.advance_epoch(epoch, plan_cut);
                self.deferred.push(Request::StartNext { stream, at: origin });
                self.deferred.push(Request::Plan { stream, epoch, retiring });
                self.recheck(cells);
            }
            Section::Drain { stream } => self.drain_check(stream, cells),
            Section::TimerFired { stream } => {
                if cells[stream].drain == Drain::Draining {
                    self.stop_send(stream, cells);
                }
            }
            Section::Done { id, terminal } => self.done(id, terminal, cells),
            Section::Consumed { id } => self.consumed_by_sample(id, cells),
            Section::Finished { stream, next_origin, epoch } => self.finished(stream, next_origin, epoch, cells),
            Section::Armed { stream } => {
                cells[stream].settle(Outstanding::Start);
                self.drain_booked(stream, cells);
                self.stop_if_shutting_down(stream, cells);
                self.recheck(cells);
            }
            Section::FirstSample { stream } => {
                if let Phase::Armed(segment) = cells[stream].phase {
                    cells[stream].phase = Phase::Streaming(segment);
                }
                self.stop_if_shutting_down(stream, cells);
            }
            Section::Lost { stream } => self.lost(stream, cells),
            Section::Flushed { stream } => {
                cells[stream].settle(Outstanding::Flush);
                self.recheck(cells);
            }
            Section::BeginFailed { stream } => {
                cells[stream].settle(Outstanding::Start);
                self.recheck(cells);
            }
            Section::Opened { stream } => {
                cells[stream].settle(Outstanding::Open);
                self.recheck(cells);
            }
            Section::Closed { stream } => {
                cells[stream].settle(Outstanding::Close);
                self.recheck(cells);
            }
            Section::Quiesced { stream } => {
                if let Some(epoch) = cells[stream].retiring.as_ref().map(|retiring| retiring.epoch) {
                    cells[stream].ack_quiesce(epoch);
                }
                cells[stream].settle(Outstanding::Plan);
                self.recheck(cells);
            }
            Section::Decide(request) => match self.decide(request, cells) {
                Decision::Go(command) => self.front(vec![Self::sent(command, cells)]),
                Decision::Wait => self.deferred.push(request),
                Decision::Never => {}
            },
            Section::Late { stream, now } => {
                self.deferred.push(Request::Late { stream, now });
                self.recheck(cells);
            }
            Section::Shutdown => {
                for held in std::mem::take(&mut self.held) {
                    let owned = cells.iter().any(|cell| cell.channels.contains(held.entry.channel as u32));
                    let disposition = if owned { Disposition::Cancelled } else { Disposition::Reported(Reason::NoChannel) };
                    self.dispose(held.entry.order.1, disposition);
                }
                if self.exit == Exit::Running {
                    self.exit = Exit::ShuttingDown;
                }
                // Each running chain is stopped (provider stop, then its left values); shutdown does not drain.
                // Its terminate waits for the stop's result and flush (`recheck`).
                for stream in 0..cells.len() {
                    if matches!(cells[stream].phase, Phase::Armed(_) | Phase::Streaming(_) | Phase::Stopping(_)) && cells[stream].drain != Drain::Ended {
                        self.stop_send(stream, cells);
                    }
                }
                self.recheck(cells);
            }
        }
    }

    /// A chain that comes up during shutdown is stopped like the ones running when it began (provider stop).
    fn stop_if_shutting_down(&mut self, stream: usize, cells: &mut [Cell]) {
        if self.exit == Exit::ShuttingDown && matches!(cells[stream].phase, Phase::Armed(_) | Phase::Streaming(_) | Phase::Stopping(_)) && cells[stream].drain != Drain::Ended {
            self.stop_send(stream, cells);
        }
    }

    /// Why held value `id` is not reserved now (`None`: it would be, or it is not held): the in-flight bound, or the cell's refusal. Read-only: the
    /// measurement's attribution of a rejected attempt (it asks `reserve`'s own two checks, never a copy of them).
    pub(crate) fn reservation_refusal(&self, id: u64, cells: &[Cell]) -> Option<NotReserved> {
        let held = self.held.iter().find(|held| held.entry.order.1 == id)?;
        let cell = &cells[held.stream];
        if let Some(bound) = self.bound_refusal(held.stream, cells) {
            return Some(bound);
        }
        cell.clone().reserve(held.entry.clone(), held.generation).err().map(NotReserved::Refused)
    }

    /// The bound that refuses a reservation on `stream` now: K over every chain, then D of the chain (both computed from the cells).
    fn bound_refusal(&self, stream: usize, cells: &[Cell]) -> Option<NotReserved> {
        if device_load(cells, self.now) >= self.device_bound {
            Some(NotReserved::DeviceBound)
        } else if cells[stream].in_flight.len() >= self.chain_bound {
            Some(NotReserved::ChainBound)
        } else {
            None
        }
    }

    /// Reserves one held value (the reservation of U1; the bounds of I11′; a refusal leaves it held, with no send, and
    /// ends the pass). A reservation queues the next pass, so the held list drains from its head.
    fn reserve(&mut self, id: u64, cells: &mut [Cell]) {
        let Some(position) = self.held.iter().position(|held| held.entry.order.1 == id) else { return };
        let held = self.held[position].clone();
        match self.bound_refusal(held.stream, cells) {
            Some(NotReserved::DeviceBound) => return self.book_capacity(cells),
            Some(_) => return,
            None => {}
        }
        if cells[held.stream].reserve(held.entry.clone(), held.generation).is_err() {
            return;
        }
        self.held.remove(position);
        let change = Change::Timed { channel: held.entry.channel, freq: held.entry.value, at: held.entry.order.0, generation: held.generation };
        let mut ops = vec![Op::Send(Command::Settings(SetCmd { id, change }))];
        if !self.held.is_empty() {
            ops.push(Op::Section(Section::ReserveAll));
        }
        self.front(ops);
    }

    /// K is full: when a completed value will free it by time, one timer at the earliest such instant after the device time known.
    fn book_capacity(&mut self, cells: &[Cell]) {
        let next = cells
            .iter()
            .flat_map(|cell| cell.in_flight.iter().map(move |f| (cell, f)))
            .filter(|(cell, f)| counts_for_k(cell, f, self.now) && matches!(f.state, ValueState::Done(_)))
            .map(|(_, f)| f.entry.order.0)
            .min();
        if let Some(at) = next.filter(|at| self.capacity_timer.is_none_or(|booked| *at < booked)) {
            self.capacity_timer = Some(at);
            self.front(vec![Op::Send(Command::CapacityTimer { at })]);
        }
    }

    /// The drain of a stop (note 45 §4.4): the stop is sent once the cut is reached; until then the
    /// drain waits for its deadline, the cut plus [`DRAIN_WINDOW`].
    /// A stop booked before its start was confirmed starts its drain now that the stream runs.
    fn drain_booked(&mut self, stream: usize, cells: &mut [Cell]) {
        if cells[stream].drain == Drain::Booked {
            cells[stream].drain = Drain::Idle;
            self.drain_check(stream, cells);
        }
    }

    fn drain_check(&mut self, stream: usize, cells: &mut [Cell]) {
        let cell = &cells[stream];
        let reached = cell.cut.is_some_and(|cut| cell.progress >= cut);
        let cut = cell.cut;
        match cells[stream].drain {
            Drain::Ended | Drain::Booked => {}
            _ if reached && !cells[stream].pinned() => self.stop_send(stream, cells),
            Drain::Idle => {
                cells[stream].drain = Drain::Draining;
                if let Some(cut) = cut {
                    self.front(vec![Op::Send(Command::Timer { stream, at: cut + self.drain_window })]);
                }
            }
            Drain::Draining => {}
        }
    }

    /// The stop is sent, and acceptance closes with it (the Stopping transition, note 45 §4.5):
    /// no reservation follows the stop's call.
    fn stop_send(&mut self, stream: usize, cells: &mut [Cell]) {
        let epoch = cells[stream].epoch;
        self.front(vec![Self::sent(Command::Rx { stream, op: RxOp::Stop { at: None, epoch } }, cells)]);
    }

    /// The brain's send of a receive command: the cell records the command as outstanding until its
    /// completion boundary (one per stream), and a stop takes the drain's Stopping transition.
    fn sent(command: Command, cells: &mut [Cell]) -> Op {
        if let Command::Rx { stream, op } = command {
            let cell = &mut cells[stream];
            match op {
                // A start begins a new segment: a plan change left the drain `Ended` for the retired epoch (its finish is stale), and the new chain's stop must be sendable.
                RxOp::Start { .. } => {
                    cell.drain = if cell.drain == Drain::Ended { Drain::Idle } else { cell.drain };
                    cell.outstanding = Some(Outstanding::Start);
                }
                RxOp::Stop { .. } => {
                    cell.drain = Drain::Ended;
                    cell.stop_accepting();
                    cell.outstanding = Some(Outstanding::Stop);
                }
                RxOp::Flush => cell.outstanding = Some(Outstanding::Flush),
                RxOp::Plan { .. } => cell.outstanding = Some(Outstanding::Plan),
                RxOp::Open => cell.outstanding = Some(Outstanding::Open),
                RxOp::Close => cell.outstanding = Some(Outstanding::Close),
                RxOp::Terminate => {}
            }
        }
        Op::Send(command)
    }

    /// A result that clears a pin may be the last thing a reached cut waited for (I16).
    fn drain_after_result(&mut self, cells: &mut [Cell]) {
        for stream in 0..cells.len() {
            if cells[stream].drain == Drain::Draining {
                self.drain_check(stream, cells);
            }
        }
    }

    fn done(&mut self, id: u64, terminal: Terminal, cells: &mut [Cell]) {
        self.pending.remove(&id);
        if !self.results.iter().any(|(value, _)| *value == id) {
            self.results.push((id, terminal));
        }
        if !cells.iter().any(|cell| cell.state_of(id).is_some()) {
            // A settings command's result (a restore or a move) is not a value's: it writes no disposition.
            self.recheck(cells);
            return;
        }
        match terminal {
            Terminal::Call(outcome @ (Outcome::Ok | Outcome::Partial)) => {
                for cell in cells.iter_mut() {
                    cell.finish_call(id, outcome);
                }
                // A value the loss report covered has no consumption to wait for: it leaves with its terminal.
                if self.consumed(id) || self.disposition(id) == Some(Disposition::Reported(Reason::Loss)) {
                    retire(id, cells);
                }
            }
            Terminal::Call(outcome) => {
                let disposition = if outcome == Outcome::Refused { Disposition::Refused } else { Disposition::Reported(Reason::Loss) };
                self.dispose(id, disposition);
                retire(id, cells);
            }
            Terminal::Dropped => {
                self.dispose(id, Disposition::Cancelled);
                retire(id, cells);
            }
            Terminal::Abandoned => {
                self.dispose(id, Disposition::Reported(Reason::Loss));
                retire(id, cells);
            }
        }
        for stream in 0..cells.len() {
            self.owe_flush(stream, cells);
        }
        self.drain_after_result(cells);
        self.recheck(cells);
    }

    /// A sample consumed a value: the first consumption is its disposition; a consumption of a
    /// left value is a leftover fact (u11b).
    fn consumed_by_sample(&mut self, id: u64, cells: &mut [Cell]) {
        match self.disposition(id) {
            None => self.dispose(id, Disposition::Consumed),
            Some(Disposition::Reported(Reason::Left)) => self.leftovers.push(id),
            Some(_) => {}
        }
        // A consumption that comes before the value's `Done` input leaves the value resident: `done` removes it (note 45 §4.5, D).
        if self.consumed(id) && self.has_result(id) {
            retire(id, cells);
        }
        self.recheck(cells);
    }

    /// The segment ends. A held value that no later segment covers is reported (G4). The taken,
    /// maybe-applied and finished values are left (or reported as left) for their consumption.
    /// A finish of an older epoch is stale: it leaves the cell to the current epoch's segment. Its
    /// held values are judged against the origin of the start that waits for it (the deferred
    /// StartNext), as a finish of the current segment judges them against its successor.
    fn finished(&mut self, stream: usize, next_origin: Option<i64>, epoch: u64, cells: &mut [Cell]) {
        cells[stream].settle(Outstanding::Stop);
        let next_origin = if cells[stream].epoch != epoch {
            self.deferred.iter().find_map(|request| match *request {
                Request::StartNext { stream: s, at } if s == stream => Some(at),
                _ => None,
            })
        } else {
            next_origin
        };
        for held in std::mem::take(&mut self.held) {
            if held.stream == stream && !next_origin.is_some_and(|origin| held.entry.order.0 >= origin) {
                self.dispose(held.entry.order.1, Disposition::Reported(Reason::NoReservation));
            } else {
                self.held.push(held);
            }
        }
        if cells[stream].epoch == epoch {
            cells[stream].finish();
            cells[stream].drain = Drain::Idle;
        }
        let left: Vec<u64> = cells[stream]
            .in_flight
            .iter()
            .filter(|flight| matches!(flight.state, ValueState::Taken | ValueState::MaybeApplied | ValueState::Done(Terminal::Call(Outcome::Ok | Outcome::Partial))))
            .map(|flight| flight.entry.order.1)
            .collect();
        for id in left {
            if self.disposition(id).is_none() {
                self.dispose(id, Disposition::Reported(Reason::Left));
                cells[stream].mark_left(id);
            }
        }
        // The stop left the cell Off with accepted values to read: their flush follows (note 45 §4.5).
        // A stale stop's left values need their flush too: the flush check reads the cell, not the epoch.
        self.owe_flush(stream, cells);
        self.recheck(cells);
    }

    /// Queues the flush a stream owes, once. A result that arrives after the stop's Finished is covered here.
    fn owe_flush(&mut self, stream: usize, cells: &[Cell]) {
        if cells[stream].flush_owed() && !self.deferred.contains(&Request::Flush { stream }) {
            self.deferred.push(Request::Flush { stream });
        }
    }

    /// The loss of a stream's device: the cell is lost, and each of its values without a disposition
    /// is reported lost, once. A call that still holds such a value writes nothing when its result comes.
    fn lost(&mut self, stream: usize, cells: &mut [Cell]) {
        cells[stream].phase = Phase::Lost;
        cells[stream].outstanding = None;
        // The values are reported at once but stay resident until their terminal input arrives (note 45 §4.5, D): a call
        // still running, or a cancelled value S has not dropped yet, leaves the cell at its `Done`.
        let ids: Vec<u64> = cells[stream].in_flight.iter().map(|flight| flight.entry.order.1).collect();
        for id in ids {
            if self.disposition(id).is_none() {
                self.dispose(id, Disposition::Reported(Reason::Loss));
            }
            if self.has_result(id) {
                retire(id, cells);
            }
        }
        self.recheck(cells);
    }

    /// The deferred requests whose condition now holds are issued, in their order; the others
    /// stay deferred (or are dropped when they can never hold).
    fn recheck(&mut self, cells: &mut [Cell]) {
        let pending = std::mem::take(&mut self.deferred);
        let mut commands = Vec::new();
        for request in pending {
            match self.decide(request, cells) {
                Decision::Go(command) => commands.push(Self::sent(command, cells)),
                Decision::Wait => self.deferred.push(request),
                Decision::Never => {}
            }
        }
        if self.terminates_ready(cells) {
            self.exit = Exit::Terminated;
            if !self.control_unspawned {
                commands.push(Op::Send(Command::SettingsTerminate));
            }
            commands.extend((0..cells.len()).filter(|stream| !self.unspawned.contains(stream)).map(|stream| Op::Send(Command::Rx { stream, op: RxOp::Terminate })));
        }
        self.front(commands);
    }

    /// The terminates go once shutdown waits for nothing: no deferred request, and every stream Off or Uncertain (or with its stop sent and finished: a plan change's stale finish leaves the phase) with no
    /// command outstanding, no drain in progress and no flush owed (every stop sent has finished). A lost cell has no result to wait for.
    fn terminates_ready(&self, cells: &[Cell]) -> bool {
        self.exit == Exit::ShuttingDown
            && self.deferred.is_empty()
            && cells.iter().all(|cell| {
                cell.phase == Phase::Lost || ((matches!(cell.phase, Phase::Off | Phase::Uncertain) || cell.drain == Drain::Ended) && !cell.busy() && cell.outstanding.is_none() && !cell.flush_owed())
            })
    }

    /// Requests of one stream keep their order: a Close, Open or StartNext waits while an earlier one for the same stream is
    /// deferred, and a Restore or Move while an earlier one for the same channel is. During a recheck `deferred` holds the earlier requests
    /// that still wait. Production runs one stream's lifecycle commands in order on its own thread.
    fn deferred_ahead(&self, request: Request) -> bool {
        let stream = |r: &Request| match r {
            Request::Close { stream } | Request::Open { stream } | Request::StartNext { stream, .. } => Some(*stream),
            _ => None,
        };
        let channel = |r: &Request| match r {
            Request::Restore { channel } | Request::Move { channel, .. } => Some(*channel),
            _ => None,
        };
        match (stream(&request), channel(&request)) {
            (Some(s), _) => self.deferred.iter().any(|d| stream(d) == Some(s)),
            (_, Some(c)) => self.deferred.iter().any(|d| channel(d) == Some(c)),
            _ => false,
        }
    }

    /// The condition of each request (note 45 §4.3; I3, I7, I13, I15). A restore waits for every
    /// pin and every outstanding consumption on its channel.
    fn decide(&mut self, request: Request, cells: &[Cell]) -> Decision {
        // Shutdown starts no segment and no lifecycle call; a restore or move still goes before the terminates.
        if self.exit == Exit::Terminated || (self.exit == Exit::ShuttingDown && matches!(request, Request::StartNext { .. } | Request::Open { .. } | Request::Close { .. })) {
            return Decision::Never;
        }
        // An executor that was never spawned runs no command: a request for its stream ends here.
        let stream_of = match request {
            Request::Close { stream } | Request::Open { stream } | Request::StartNext { stream, .. } | Request::Flush { stream } | Request::Plan { stream, .. } | Request::Late { stream, .. } => {
                Some(stream)
            }
            Request::Restore { .. } | Request::Move { .. } => None,
        };
        if stream_of.is_some_and(|stream| self.unspawned.contains(&stream)) {
            return Decision::Never;
        }
        if self.deferred_ahead(request) {
            return Decision::Wait;
        }
        // A settings command sent and not yet done holds every lifecycle change and settings change on its channel.
        let on_channel = |channel: usize| self.pending.values().any(|c| *c == channel);
        let blocked = match request {
            Request::Restore { channel } | Request::Move { channel, .. } => on_channel(channel),
            Request::Close { stream } | Request::Open { stream } | Request::StartNext { stream, .. } => cells[stream].channels.iter().any(|channel| on_channel(channel as usize)),
            _ => false,
        };
        if blocked {
            return Decision::Wait;
        }
        match request {
            Request::Restore { channel } => {
                if cells.iter().any(|cell| cell.channels.contains(channel as u32) && matches!(cell.phase, Phase::Uncertain | Phase::Lost)) {
                    return Decision::Never;
                }
                // The flush was attempted and a left value stays: production skips this restore.
                if cells.iter().any(|cell| cell.left_after_flush(channel)) {
                    return Decision::Never;
                }
                // Production restores after stop and flush: every chain of the channel is Off (E1-179).
                if cells.iter().any(|cell| cell.committed(Some(channel)) || cell.owed_on(channel) || (cell.channels.contains(channel as u32) && (cell.busy() || cell.phase != Phase::Off))) {
                    return Decision::Wait;
                }
                let id = self.fresh_id();
                self.pending.insert(id, channel);
                Decision::Go(Command::Settings(SetCmd { id, change: Change::Restore { channel } }))
            }
            Request::Move { channel, to } => {
                if cells.iter().any(|cell| cell.committed(Some(channel)) || (cell.channels.contains(channel as u32) && cell.busy())) {
                    return Decision::Wait;
                }
                let id = self.fresh_id();
                self.pending.insert(id, channel);
                Decision::Go(Command::Settings(SetCmd { id, change: Change::Move { channel, to } }))
            }
            Request::Close { stream } | Request::Open { stream } => {
                let cell = &cells[stream];
                // Acceptance is closed by the phase (`reserve` refuses all but Armed and Streaming): a handle changes only on a chain that is not accepting.
                if cell.phase == Phase::Lost {
                    return Decision::Never;
                }
                if !matches!(cell.phase, Phase::Off | Phase::Uncertain) || cell.committed(None) || cell.retiring.is_some() || cell.busy() {
                    return Decision::Wait;
                }
                let op = if matches!(request, Request::Close { .. }) { RxOp::Close } else { RxOp::Open };
                Decision::Go(Command::Rx { stream, op })
            }
            Request::StartNext { stream, at } => {
                let cell = &cells[stream];
                if cell.phase == Phase::Lost {
                    return Decision::Never;
                }
                if cell.retiring.is_some() {
                    return Decision::Wait;
                }
                // A flush owed on the stream goes first: its start is not decided in the same batch (the phase is still Off).
                if cell.pinned() || cell.flush_owed() || cell.busy() || matches!(cell.phase, Phase::Armed(_) | Phase::Streaming(_)) {
                    return Decision::Wait;
                }
                Decision::Go(Command::Rx { stream, op: RxOp::Start { at, epoch: cell.epoch } })
            }
            Request::Plan { stream, epoch, retiring } => {
                // Nothing remains to change on a lost stream: the loss path disposes its values.
                if cells[stream].phase == Phase::Lost {
                    return Decision::Never;
                }
                // Not `busy`: the Plan is R's own stop of the old epoch, decided while the PlanCancel's `Stopping` is in force.
                if cells[stream].outstanding.is_some() {
                    return Decision::Wait;
                }
                Decision::Go(Command::Rx { stream, op: RxOp::Plan { epoch, retiring } })
            }
            Request::Late { stream, now } => {
                let cell = &cells[stream];
                if !matches!(cell.phase, Phase::Armed(_) | Phase::Streaming(_)) {
                    return Decision::Never;
                }
                if cell.outstanding.is_some() {
                    return Decision::Wait;
                }
                let op = if cell.cut.is_some() { RxOp::Stop { at: Some(now), epoch: cell.epoch } } else { RxOp::Start { at: now, epoch: cell.epoch } };
                Decision::Go(Command::Rx { stream, op })
            }
            Request::Flush { stream } => {
                let cell = &cells[stream];
                if cell.phase == Phase::Lost {
                    return Decision::Never;
                }
                // A flush is requested for a debt: once the debt is gone, the request is dropped, not deferred.
                if !cell.flush_owed() {
                    return Decision::Never;
                }
                if cell.phase == Phase::Off && !cell.committed(None) && !cell.busy() {
                    return Decision::Go(Command::Rx { stream, op: RxOp::Flush });
                }
                Decision::Wait
            }
        }
    }
}

/// Why a held value is not reserved: the device is at K, its chain is at D, or the cell refuses it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NotReserved {
    DeviceBound,
    ChainBound,
    Refused(Refusal),
}

/// Whether a value counts against the device's capacity K at device time `now` (note 45 §4.5): a value whose call is queued, taken or
/// may be running counts whatever its instant; one that finished ok or partly counts until the device time reaches its instant;
/// a cancelled, refused, dropped or abandoned one does not. Once its cell is lost a finished call no longer counts.
pub(crate) fn counts_for_k(cell: &Cell, flight: &InFlight, now: i64) -> bool {
    match flight.state {
        ValueState::Queued | ValueState::Taken | ValueState::MaybeApplied => true,
        ValueState::Done(Terminal::Call(Outcome::Ok | Outcome::Partial)) => cell.phase != Phase::Lost && now < flight.entry.order.0,
        ValueState::Cancelled | ValueState::Done(_) => false,
    }
}

/// K's load: the values counted over every chain (the only place K is computed; nothing stores it).
pub(crate) fn device_load(cells: &[Cell], now: i64) -> usize {
    cells.iter().map(|cell| cell.in_flight.iter().filter(|flight| counts_for_k(cell, flight, now)).count()).sum()
}

/// Removes the value from whichever cell holds it.
fn retire(id: u64, cells: &mut [Cell]) {
    for cell in cells.iter_mut() {
        cell.retire(id);
    }
}

#[cfg(test)]
mod tests {
    use ezsdr_radio::timeline::Channels;

    use super::super::cell::{Call, Take};
    use super::super::fixtures::{cell, feed, held, push, recv, seg, value};
    use super::*;
    use crate::device::DeviceError;

    fn timed(id: u64, channel: usize, freq: f64, at: i64, generation: u64) -> Command {
        Command::Settings(SetCmd { id, change: Change::Timed { channel, freq, at, generation } })
    }

    /// One Streaming stream, with value 1 (instant 8) taken, so the chain is pinned.
    /// The settings commands the brain sent (the outbox also holds timers).
    fn settings_sent(brain: &Brain) -> Vec<&Command> {
        brain.outbox.iter().filter(|command| matches!(command, Command::Settings(_))).collect()
    }

    fn pinned() -> (Vec<Cell>, Brain) {
        let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, None, 4)];
        push(&mut cells[0], value(0, 8, 1), ValueState::Taken);
        (cells, Brain::new())
    }

    #[test]
    fn u03_cancel_before_take_drops_without_call() {
        let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, Some(16), 4)];
        cells[0].reserve(value(0, 8, 1), 1).expect("reserved");
        cells[0].book_stop(8);
        assert_eq!(cells[0].take(1), Take::Dropped);
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Done { id: 1, terminal: Terminal::Dropped, now: 0 });
        feed(&mut brain, &mut cells, Input::Done { id: 1, terminal: Terminal::Dropped, now: 0 });
        assert_eq!(brain.dispositions, vec![(1, Disposition::Cancelled)]);
    }

    #[test]
    fn u04_taken_then_stop_is_pinned_not_cancelled() {
        let (mut cells, mut brain) = pinned();
        feed(&mut brain, &mut cells, Input::Stop { stream: 0, requested: 8 });
        assert_eq!(cells[0].state_of(1), Some(&ValueState::Taken));
        assert!(cells[0].pinned());
        assert_eq!(brain.outbox, vec![Command::Timer { stream: 0, at: 16 }]);
    }

    #[test]
    fn u04d_drain_deadline_sends_stop_while_pinned() {
        let (mut cells, mut brain) = pinned();
        feed(&mut brain, &mut cells, Input::Stop { stream: 0, requested: 8 });
        feed(&mut brain, &mut cells, Input::Timer { stream: 0, now: 0 });
        assert_eq!(brain.outbox.last(), Some(&Command::Rx { stream: 0, op: RxOp::Stop { at: None, epoch: 1 } }));
        assert_eq!(cells[0].state_of(1), Some(&ValueState::Taken));
    }

    #[test]
    fn u05b_reservation_after_stopping_refused() {
        let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, Some(16), 4)];
        cells[0].stop_accepting();
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Release(held(0, value(0, 8, 2), 1)));
        assert_eq!(brain.held.len(), 1);
        assert!(brain.outbox.is_empty());
    }

    #[test]
    fn u05c_stop_send_closes_acceptance_before_call() {
        let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, None, 4)];
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Stop { stream: 0, requested: 8 });
        feed(&mut brain, &mut cells, Input::Timer { stream: 0, now: 0 });
        assert_eq!(brain.outbox.last(), Some(&Command::Rx { stream: 0, op: RxOp::Stop { at: None, epoch: 1 } }));
        assert!(matches!(cells[0].phase, Phase::Stopping(_)));
        brain.outbox.clear();
        feed(&mut brain, &mut cells, Input::Release(held(0, value(0, 5, 2), 1)));
        assert_eq!(brain.held.len(), 1);
        assert!(brain.outbox.is_empty());
    }

    #[test]
    fn u09a_restore_deferred_while_pinned() {
        let (mut cells, mut brain) = pinned();
        cells[0].phase = Phase::Off;
        feed(&mut brain, &mut cells, Input::Request(Request::Restore { channel: 0 }));
        assert!(brain.outbox.is_empty());
        feed(&mut brain, &mut cells, Input::Done { id: 1, terminal: Terminal::Call(Outcome::Ok), now: 0 });
        assert!(brain.outbox.is_empty());
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Consumed { id: 1 }));
        assert_eq!(brain.outbox.len(), 1);
        assert!(matches!(brain.outbox[0], Command::Settings(SetCmd { change: Change::Restore { channel: 0 }, .. })));
    }

    #[test]
    fn u09b_move_deferred_while_pinned() {
        let (mut cells, mut brain) = pinned();
        feed(&mut brain, &mut cells, Input::Request(Request::Move { channel: 0, to: 1 }));
        assert!(brain.outbox.is_empty());
        feed(&mut brain, &mut cells, Input::Done { id: 1, terminal: Terminal::Call(Outcome::Ok), now: 0 });
        assert_eq!(brain.outbox.len(), 1);
        assert!(matches!(brain.outbox[0], Command::Settings(SetCmd { change: Change::Move { channel: 0, to: 1 }, .. })));
    }

    #[test]
    fn u09c_close_and_open_deferred_while_pinned() {
        let mut cells = vec![cell(Phase::Off, 0, None, 4)];
        push(&mut cells[0], value(0, 8, 1), ValueState::Taken);
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Request(Request::Close { stream: 0 }));
        feed(&mut brain, &mut cells, Input::Request(Request::Open { stream: 0 }));
        assert!(brain.outbox.is_empty());
        feed(&mut brain, &mut cells, Input::Done { id: 1, terminal: Terminal::Call(Outcome::Ok), now: 0 });
        assert_eq!(brain.outbox, vec![Command::Rx { stream: 0, op: RxOp::Close }]);
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Closed { stream: 0 }));
        assert_eq!(brain.outbox, vec![Command::Rx { stream: 0, op: RxOp::Close }, Command::Rx { stream: 0, op: RxOp::Open }]);
    }

    /// Case 8 micro (I9): a Close waits for the booked stop. The drain timer's Stop goes first, and the
    /// Close follows the stop's Finished, never behind it in the receive FIFO.
    #[test]
    fn u09w_close_waits_for_booked_stop() {
        let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, None, 4)];
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Stop { stream: 0, requested: 8 });
        feed(&mut brain, &mut cells, Input::Request(Request::Close { stream: 0 }));
        assert!(!brain.outbox.contains(&Command::Rx { stream: 0, op: RxOp::Close }), "the Close waits for the drain: {:?}", brain.outbox);
        feed(&mut brain, &mut cells, Input::Timer { stream: 0, now: 0 });
        let stop = Command::Rx { stream: 0, op: RxOp::Stop { at: None, epoch: 1 } };
        assert_eq!(brain.outbox.last(), Some(&stop));
        cells[0].phase = Phase::Off;
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Finished { stream: 0, next_origin: None, epoch: 1 }));
        let at = |command: &Command| brain.outbox.iter().position(|c| c == command);
        assert!(at(&stop) < at(&Command::Rx { stream: 0, op: RxOp::Close }), "the Close follows the stop: {:?}", brain.outbox);
    }

    /// Case 7b micro (I13): a Restore or Move is not decided while a stop is in flight on its channel's
    /// stream, so a failed stop cannot find it already on its way to S.
    #[test]
    fn u09x_restore_and_move_wait_for_stop_in_flight() {
        let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, None, 4)];
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Stop { stream: 0, requested: 8 });
        feed(&mut brain, &mut cells, Input::Request(Request::Restore { channel: 0 }));
        feed(&mut brain, &mut cells, Input::Request(Request::Move { channel: 0, to: 1 }));
        feed(&mut brain, &mut cells, Input::Timer { stream: 0, now: 0 });
        assert!(matches!(cells[0].phase, Phase::Stopping(_)));
        assert!(!brain.outbox.iter().any(|c| matches!(c, Command::Settings(_))), "nothing goes to S during the stop: {:?}", brain.outbox);
        cells[0].phase = Phase::Off;
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Finished { stream: 0, next_origin: None, epoch: 1 }));
        assert!(brain.outbox.iter().any(|c| matches!(c, Command::Settings(SetCmd { change: Change::Restore { channel: 0 }, .. }))), "{:?}", brain.outbox);
    }

    /// Case 8 micro (E1b part 3d): requests for one stream keep their order.
    /// A Close waits for a committed value. The value's result is written (the cell no longer holds it as committed) before the
    /// brain receives its `Done`, so an Open that arrives in between is decided at once and overtakes the deferred Close:
    /// the executor opens, then closes, and the StartNext behind them calls Begin on a closed handle (I9).
    #[test]
    fn u09z_open_does_not_overtake_deferred_close() {
        let mut cells = vec![cell(Phase::Off, 0, None, 0)];
        let mut brain = Brain::new();
        brain.deferred.push(Request::Close { stream: 0 });
        feed(&mut brain, &mut cells, Input::Request(Request::Open { stream: 0 }));
        assert!(!brain.outbox.contains(&Command::Rx { stream: 0, op: RxOp::Open }), "Open overtook the deferred Close: {:?}", brain.outbox);
    }

    /// Seeded configuration, horizon 96, seeds 3, 64, 67 (E1b part 3d). A stop sent to a stream
    /// the device has lost is never finished, so its `outstanding` stays set and a `Plan` for the stream waits for ever; shutdown
    /// then waits on the deferred request (Incomplete, B waiting). A Plan for a lost stream is dropped.
    #[test]
    fn u09aa_plan_on_lost_stream_does_not_wait_for_ever() {
        let mut c = cell(Phase::Lost, 0, None, 0);
        c.outstanding = Some(Outstanding::Stop);
        let mut cells = vec![c];
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Request(Request::Plan { stream: 0, epoch: 2, retiring: 1 }));
        assert!(brain.deferred.is_empty(), "the Plan waits for a result a lost stream never gives: {:?}", brain.deferred);
    }

    /// The drain alone makes a stream busy (a stop decided, its timer pending), and a lost stream is never busy.
    #[test]
    fn u09y_busy_reads_drain_and_lost() {
        let mut c = cell(Phase::Streaming(seg(0)), 0, None, 4);
        assert!(!c.busy());
        c.drain = Drain::Draining;
        assert!(c.busy());
        c.drain = Drain::Ended;
        assert!(!c.busy());
        c.drain = Drain::Draining;
        c.phase = Phase::Lost;
        assert!(!c.busy(), "a lost stream has no result to wait for");
    }

    #[test]
    fn u09d_start_next_deferred_while_pinned() {
        let mut cells = vec![cell(Phase::Off, 0, None, 0)];
        push(&mut cells[0], value(0, 8, 1), ValueState::Taken);
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Request(Request::StartNext { stream: 0, at: 20 }));
        assert!(brain.outbox.is_empty());
        feed(&mut brain, &mut cells, Input::Done { id: 1, terminal: Terminal::Call(Outcome::Ok), now: 0 });
        assert_eq!(brain.outbox, vec![Command::Rx { stream: 0, op: RxOp::Start { at: 20, epoch: 1 } }]);
    }

    /// Seed 27's shape: a Move decided while value 1 is queued in S's FIFO. S takes and pins it
    /// first, so the Move waits until the call finishes.
    #[test]
    fn u09i_move_waits_for_value_queued_in_s_fifo() {
        let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, None, 4)];
        push(&mut cells[0], value(0, 8, 1), ValueState::Queued);
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Request(Request::Move { channel: 0, to: 1 }));
        assert!(brain.outbox.is_empty(), "queued value 1 is ahead of the Move in S's FIFO");
        assert_eq!(cells[0].take(1), Take::Call);
        assert!(brain.outbox.is_empty(), "taken value 1 pins the chain");
        feed(&mut brain, &mut cells, Input::Done { id: 1, terminal: Terminal::Call(Outcome::Ok), now: 0 });
        assert!(matches!(brain.outbox.as_slice(), [Command::Settings(SetCmd { change: Change::Move { channel: 0, to: 1 }, .. })]), "a Move has no debt wait: the pin is gone");
    }

    /// Seed 18's shape: a Restore decided while value 1 is queued. Taken, applied, and reported with
    /// its Done still unprocessed, the value keeps the Restore waiting until its consumption.
    #[test]
    fn u09j_restore_waits_for_queued_value_through_its_done() {
        let mut cells = vec![cell(Phase::Off, 0, None, 4)];
        push(&mut cells[0], value(0, 8, 1), ValueState::Queued);
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Request(Request::Restore { channel: 0 }));
        assert!(brain.outbox.is_empty(), "queued value 1 is ahead of the Restore in S's FIFO");
        assert_eq!(cells[0].take(1), Take::Call);
        assert_eq!(cells[0].state_of(1), Some(&ValueState::Taken));
        feed(&mut brain, &mut cells, Input::Done { id: 1, terminal: Terminal::Call(Outcome::Partial), now: 0 });
        assert!(brain.outbox.is_empty(), "partially applied value 1 owes debt");
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Consumed { id: 1 }));
        assert!(matches!(brain.outbox.as_slice(), [Command::Settings(SetCmd { change: Change::Restore { channel: 0 }, .. })]));
    }

    /// A cancelled value is dropped at take and blocks nothing; a value on another channel does not
    /// block a Move or a Restore of channel 0.
    #[test]
    fn u09k_cancelled_or_other_channel_values_do_not_block() {
        let mut cells = vec![cell(Phase::Off, 0, None, 4)];
        push(&mut cells[0], value(0, 8, 1), ValueState::Cancelled);
        push(&mut cells[0], value(1, 9, 2), ValueState::Queued);
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Request(Request::Move { channel: 0, to: 1 }));
        assert!(matches!(brain.outbox.as_slice(), [Command::Settings(SetCmd { change: Change::Move { channel: 0, to: 1 }, .. })]));
        let Some(Command::Settings(SetCmd { id, .. })) = brain.outbox.first().cloned() else { panic!("{:?}", brain.outbox) };
        feed(&mut brain, &mut cells, Input::Done { id, terminal: Terminal::Call(Outcome::Ok), now: 0 });
        feed(&mut brain, &mut cells, Input::Request(Request::Restore { channel: 0 }));
        assert!(matches!(brain.outbox.as_slice(), [_, Command::Settings(SetCmd { change: Change::Restore { channel: 0 }, .. })]));
    }

    /// A Close waits for a value queued on its chain, as Move and Restore do.
    #[test]
    fn u09l_close_waits_for_queued_value_on_chain() {
        let mut cells = vec![cell(Phase::Off, 0, None, 0)];
        push(&mut cells[0], value(0, 8, 1), ValueState::Queued);
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Request(Request::Close { stream: 0 }));
        assert!(brain.outbox.is_empty(), "queued value 1 is ahead of the Close");
        assert_eq!(cells[0].take(1), Take::Call);
        assert!(brain.outbox.is_empty(), "taken value 1 pins the chain");
        feed(&mut brain, &mut cells, Input::Done { id: 1, terminal: Terminal::Call(Outcome::Ok), now: 0 });
        assert_eq!(brain.outbox, vec![Command::Rx { stream: 0, op: RxOp::Close }]);
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Closed { stream: 0 }));
    }

    /// An Open decided alone (no earlier request deferred) waits for a value queued on its chain. A StartNext has no such test: a
    /// Queued value exists only in an Armed or Streaming cell, where StartNext waits for the phase.
    #[test]
    fn u09l2_open_waits_for_queued_value_on_chain() {
        let mut cells = vec![cell(Phase::Off, 0, None, 4)];
        push(&mut cells[0], value(0, 8, 1), ValueState::Queued);
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Request(Request::Open { stream: 0 }));
        assert!(brain.outbox.is_empty(), "queued value 1 is ahead of the Open");
        assert_eq!(cells[0].take(1), Take::Call);
        feed(&mut brain, &mut cells, Input::Done { id: 1, terminal: Terminal::Call(Outcome::Ok), now: 0 });
        assert_eq!(brain.outbox, vec![Command::Rx { stream: 0, op: RxOp::Open }]);
    }

    /// A stop that leaves an accepted value to read requests its flush, and a start waiting for the
    /// segment waits for that flush too, though the cell is Off (note 45 §4.5). The flush's end
    /// retires the value and releases the start.
    #[test]
    fn u09m_stop_debt_requests_flush_and_start_waits_for_it() {
        let mut entry = value(0, 8, 1);
        entry.accepted_at = Some(8);
        let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, None, 4)];
        push(&mut cells[0], entry, ValueState::Done(Terminal::Call(Outcome::Ok)));
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Request(Request::StartNext { stream: 0, at: 12 }));
        cells[0].phase = Phase::Off;
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Finished { stream: 0, next_origin: None, epoch: 1 }));
        assert_eq!(brain.outbox, vec![Command::Rx { stream: 0, op: RxOp::Flush }], "the flush is the only command, the start waits");
        assert_eq!(brain.deferred, vec![Request::StartNext { stream: 0, at: 12 }]);
        cells[0].retire(1);
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Flushed { stream: 0 }));
        assert_eq!(brain.outbox.last(), Some(&Command::Rx { stream: 0, op: RxOp::Start { at: 12, epoch: 1 } }));
    }

    /// A failed stop leaves the cell Uncertain, and no flush follows (I13).
    #[test]
    fn u09n_failed_stop_requests_no_flush() {
        let mut entry = value(0, 8, 1);
        entry.accepted_at = Some(8);
        let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, None, 4)];
        push(&mut cells[0], entry, ValueState::Done(Terminal::Call(Outcome::Ok)));
        let mut brain = Brain::new();
        cells[0].phase = Phase::Uncertain;
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Finished { stream: 0, next_origin: None, epoch: 1 }));
        assert!(brain.outbox.is_empty(), "no flush after an uncertain stop");
    }

    /// Close and Open wait while the cell flushes, and go when the flush's stop has returned (note 45 §4.5).
    #[test]
    fn u09o_lifecycle_waits_for_flush_in_progress() {
        let mut cells = vec![cell(Phase::Flushing, 0, None, 0)];
        cells[0].outstanding = Some(Outstanding::Flush);
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Request(Request::Close { stream: 0 }));
        feed(&mut brain, &mut cells, Input::Request(Request::Open { stream: 0 }));
        assert!(brain.outbox.is_empty(), "{:?}", brain.outbox);
        assert_eq!(brain.deferred, vec![Request::Close { stream: 0 }, Request::Open { stream: 0 }]);
        cells[0].phase = Phase::Off;
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Flushed { stream: 0 }));
        assert_eq!(brain.outbox, vec![Command::Rx { stream: 0, op: RxOp::Close }]);
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Closed { stream: 0 }));
        assert_eq!(brain.outbox, vec![Command::Rx { stream: 0, op: RxOp::Close }, Command::Rx { stream: 0, op: RxOp::Open }]);
    }

    /// Sol E1 finding 2: a Close decided while the chain accepts would let a value be reserved, taken and pin the chain before R
    /// runs the Close. The Close waits for a chain that is not accepting; the value is reserved meanwhile and the Close goes
    /// only after the stop.
    #[test]
    fn u09ab_close_and_open_wait_for_a_chain_that_is_not_accepting() {
        for request in [Request::Close { stream: 0 }, Request::Open { stream: 0 }] {
            let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, None, 4)];
            let mut brain = Brain::new();
            feed(&mut brain, &mut cells, Input::Request(request));
            assert!(brain.outbox.is_empty(), "{request:?} waits while the chain accepts: {:?}", brain.outbox);
            feed(&mut brain, &mut cells, Input::Release(held(0, value(0, 8, 1), 1)));
            assert_eq!(cells[0].take(1), Take::Call);
            feed(&mut brain, &mut cells, Input::Done { id: 1, terminal: Terminal::Call(Outcome::Ok), now: 0 });
            assert!(!brain.outbox.iter().any(|c| matches!(c, Command::Rx { .. })), "{request:?} is not issued while the chain is accepting: {:?}", brain.outbox);
            cells[0].phase = Phase::Off;
            cells[0].retire(1);
            feed(&mut brain, &mut cells, Input::Boundary(Boundary::Finished { stream: 0, next_origin: None, epoch: 1 }));
            let op = if matches!(request, Request::Close { .. }) { RxOp::Close } else { RxOp::Open };
            assert!(brain.outbox.contains(&Command::Rx { stream: 0, op }), "{request:?} goes once the chain is stopped: {:?}", brain.outbox);
        }
    }

    /// A Close or Open for a stream whose device is lost is dropped, and one for an uncertain stream goes.
    #[test]
    fn u09ac_close_dropped_when_lost_and_goes_when_uncertain() {
        let mut cells = vec![cell(Phase::Lost, 0, None, 0)];
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Request(Request::Close { stream: 0 }));
        assert!(brain.outbox.is_empty() && brain.deferred.is_empty());
        let mut cells = vec![cell(Phase::Uncertain, 0, None, 0)];
        feed(&mut brain, &mut cells, Input::Request(Request::Close { stream: 0 }));
        assert_eq!(brain.outbox, vec![Command::Rx { stream: 0, op: RxOp::Close }]);
    }

    /// Sol E1 finding 1: the flush was attempted and a left value stays on the channel (its write is still in the device FIFO):
    /// the restore is dropped, as production skips it. Before the flush it waits.
    #[test]
    fn u09e2_restore_dropped_when_left_stays_after_the_flush() {
        let mut cells = vec![cell(Phase::Off, 0, None, 0)];
        push(&mut cells[0], value(0, 8, 1), ValueState::Done(Terminal::Call(Outcome::Ok)));
        cells[0].mark_left(1);
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Request(Request::Restore { channel: 0 }));
        assert_eq!(brain.deferred, vec![Request::Restore { channel: 0 }], "before the flush the restore waits");
        cells[0].device_result(Call::FlushStart, &Ok(()));
        cells[0].device_result(Call::FlushStop, &Ok(()));
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Flushed { stream: 0 }));
        assert!(brain.deferred.is_empty() && !brain.outbox.iter().any(|c| matches!(c, Command::Settings(_))), "the restore is dropped: {:?} {:?}", brain.deferred, brain.outbox);
        assert!(cells[0].is_left(1), "the unread value stays left on the cell");
    }

    /// The flush is attempted once per left value: after it the cell owes none, so no second flush is requested and a start waiting for it goes.
    #[test]
    fn u09m2_flush_attempted_is_not_owed_again() {
        let mut cells = vec![cell(Phase::Off, 0, None, 0)];
        push(&mut cells[0], value(0, 8, 1), ValueState::Done(Terminal::Call(Outcome::Ok)));
        cells[0].mark_left(1);
        assert!(cells[0].flush_owed());
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Request(Request::StartNext { stream: 0, at: 20 }));
        assert!(brain.outbox.is_empty(), "the start waits for the flush");
        cells[0].device_result(Call::FlushStart, &Ok(()));
        cells[0].device_result(Call::FlushStop, &Ok(()));
        assert!(!cells[0].flush_owed());
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Flushed { stream: 0 }));
        assert_eq!(brain.outbox, vec![Command::Rx { stream: 0, op: RxOp::Start { at: 20, epoch: 1 } }]);
        cells[0].mark_left(1);
        assert!(cells[0].flush_owed(), "a value left again is owed its own flush");
    }

    #[test]
    fn u09e_restore_waits_for_debt_or_left_on_channel() {
        let mut cells = vec![cell(Phase::Off, 0, None, 0)];
        push(&mut cells[0], value(0, 8, 1), ValueState::Done(Terminal::Call(Outcome::Ok)));
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Request(Request::Restore { channel: 0 }));
        assert!(brain.outbox.is_empty());
        assert_eq!(brain.deferred, vec![Request::Restore { channel: 0 }]);
    }

    #[test]
    fn u09f_restore_waits_for_left_on_other_chain() {
        let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, None, 0), cell(Phase::Off, 0, None, 0)];
        cells[0].channels = Channels::first(0).with(1);
        push(&mut cells[0], value(0, 8, 1), ValueState::Done(Terminal::Call(Outcome::Ok)));
        let mut brain = Brain::new();
        // The value's `Done` was taken before (a consumption removes a value only after its terminal input).
        brain.results.push((1, Terminal::Call(Outcome::Ok)));
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Finished { stream: 0, next_origin: None, epoch: 1 }));
        feed(&mut brain, &mut cells, Input::Request(Request::Restore { channel: 0 }));
        assert!(brain.outbox.is_empty());
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Consumed { id: 1 }));
        assert_eq!(brain.outbox.len(), 1);
        assert!(matches!(brain.outbox[0], Command::Settings(SetCmd { change: Change::Restore { channel: 0 }, .. })));
    }

    /// Shutdown's terminates wait until every stream is Off or Uncertain with its stop finished (Sol's second re-review P1): an
    /// Armed or Stopping chain whose stop was not sent holds them.
    #[test]
    fn u21_terminates_wait_for_a_chain_that_is_not_stopped() {
        for phase in [Phase::Armed(seg(0)), Phase::Streaming(seg(0)), Phase::Stopping(seg(0))] {
            let cells = vec![cell(phase, 0, Some(8), 0)];
            let mut brain = Brain::new();
            brain.exit = Exit::ShuttingDown;
            assert!(!brain.terminates_ready(&cells), "{phase:?}");
        }
        let cells = vec![cell(Phase::Off, 0, None, 0)];
        let mut brain = Brain::new();
        brain.exit = Exit::ShuttingDown;
        assert!(brain.terminates_ready(&cells));
    }

    /// A Restore is decided only when every chain of the channel is Off: production restores after stop and flush (E1-179).
    #[test]
    fn u13_restore_waits_while_a_chain_of_the_channel_is_not_off() {
        for phase in [Phase::Armed(seg(0)), Phase::Streaming(seg(0)), Phase::Stopping(seg(0)), Phase::Flushing] {
            let mut cells = vec![cell(phase, 0, None, 0)];
            let mut brain = Brain::new();
            feed(&mut brain, &mut cells, Input::Request(Request::Restore { channel: 0 }));
            assert!(brain.outbox.is_empty(), "{phase:?}");
            assert_eq!(brain.deferred, vec![Request::Restore { channel: 0 }], "{phase:?}");
        }
        // An Off chain with a command outstanding (a flush, say) is busy: the Restore waits for its completion too.
        let mut cells = vec![cell(Phase::Off, 0, None, 0)];
        cells[0].outstanding = Some(Outstanding::Flush);
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Request(Request::Restore { channel: 0 }));
        assert_eq!(brain.deferred, vec![Request::Restore { channel: 0 }], "an Off chain with a flush outstanding");
    }

    /// A Restore sent and not done holds the owning stream's StartNext, Open and Close and a second Restore or Move of the channel (E1-180).
    #[test]
    fn u13_pending_restore_blocks_lifecycle_until_done() {
        let mut cells = vec![cell(Phase::Off, 0, None, 0)];
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Request(Request::Restore { channel: 0 }));
        let id = match brain.outbox.as_slice() {
            [Command::Settings(SetCmd { id, change: Change::Restore { channel: 0 } })] => *id,
            other => panic!("{other:?}"),
        };
        for request in [Request::StartNext { stream: 0, at: 20 }, Request::Open { stream: 0 }, Request::Close { stream: 0 }, Request::Restore { channel: 0 }, Request::Move { channel: 0, to: 1 }] {
            feed(&mut brain, &mut cells, Input::Request(request));
        }
        assert_eq!(brain.outbox.len(), 1, "everything waits for the Restore's Done: {:?}", brain.outbox);
        assert_eq!(brain.deferred.len(), 5);
        feed(&mut brain, &mut cells, Input::Done { id, terminal: Terminal::Call(Outcome::Ok), now: 0 });
        assert!(brain.pending.values().all(|c| *c != 0) || brain.outbox.len() > 1);
        assert!(brain.outbox.iter().any(|c| matches!(c, Command::Rx { stream: 0, op: RxOp::Start { .. } })), "the start goes after the Done: {:?}", brain.outbox);
    }

    #[test]
    fn u09g_start_next_not_blocked_by_debt() {
        let mut cells = vec![cell(Phase::Off, 0, None, 0)];
        push(&mut cells[0], value(0, 8, 1), ValueState::Taken);
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Request(Request::StartNext { stream: 0, at: 20 }));
        feed(&mut brain, &mut cells, Input::Done { id: 1, terminal: Terminal::Call(Outcome::Ok), now: 0 });
        assert_eq!(brain.outbox, vec![Command::Rx { stream: 0, op: RxOp::Start { at: 20, epoch: 1 } }]);
    }

    /// A start waits for the flush of a left value that has a result. The setup starts from a streaming
    /// chain: an Off cell with a left value is entered only through finish(), which marks its values left.
    #[test]
    fn u09p_start_next_waits_for_flush_of_left() {
        let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, None, 0)];
        cells[0].epoch = 1;
        push(&mut cells[0], value(0, 8, 1), ValueState::Taken);
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Request(Request::StartNext { stream: 0, at: 20 }));
        feed(&mut brain, &mut cells, Input::Done { id: 1, terminal: Terminal::Call(Outcome::Ok), now: 0 });
        // The stop's result leaves the cell Off; its Finished then marks the value left.
        cells[0].phase = Phase::Off;
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Finished { stream: 0, next_origin: None, epoch: 1 }));
        assert_eq!(brain.outbox, vec![Command::Rx { stream: 0, op: RxOp::Flush }]);
        cells[0].retire(1);
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Flushed { stream: 0 }));
        assert!(matches!(brain.outbox.last(), Some(Command::Rx { stream: 0, op: RxOp::Start { at: 20, .. } })), "{:?}", brain.outbox);
        assert_eq!(brain.dispositions, vec![(1, Disposition::Reported(Reason::Left))]);
    }

    #[test]
    fn u17_stop_booked_while_off_starts_drain_when_armed() {
        let mut cells = vec![cell(Phase::Off, 0, None, 0)];
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Stop { stream: 0, requested: 16 });
        assert_eq!(cells[0].cut, Some(16), "the cut holds before the start");
        assert_eq!(cells[0].drain, Drain::Booked);
        assert!(brain.outbox.is_empty(), "no drain runs before the stream runs");
        cells[0].phase = Phase::Armed(seg(0));
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Armed { stream: 0 }));
        assert_eq!(cells[0].cut, Some(16));
        assert_eq!(cells[0].drain, Drain::Draining);
        assert_eq!(brain.outbox, vec![Command::Timer { stream: 0, at: 16 + DRAIN_WINDOW }]);
        assert_eq!(cells[0].reserve(value(0, 16, 9), 1), Err(super::super::cell::Refusal::Cut));
    }

    #[test]
    fn u19a_shutdown_stops_running_chain_before_terminate() {
        let mut c = cell(Phase::Streaming(seg(0)), 0, Some(16), 0);
        push(&mut c, value(0, 8, 1), ValueState::Taken);
        let mut cells = vec![c];
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Shutdown);
        let stop = Command::Rx { stream: 0, op: RxOp::Stop { at: None, epoch: 1 } };
        let terminate = Command::Rx { stream: 0, op: RxOp::Terminate };
        assert!(brain.outbox.contains(&stop), "a running chain is stopped at shutdown, pinned or not: {:?}", brain.outbox);
        assert!(!brain.outbox.contains(&terminate), "the terminate waits for the stop's result: {:?}", brain.outbox);
        assert_eq!(cells[0].drain, Drain::Ended);
        cells[0].phase = Phase::Off;
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Finished { stream: 0, next_origin: None, epoch: 1 }));
        let at = |command: &Command| brain.outbox.iter().position(|c| c == command);
        assert!(at(&stop) < at(&terminate), "the stop runs before the terminate: {:?}", brain.outbox);
    }

    #[test]
    fn u19b_shutdown_after_stop_sends_no_second_stop() {
        let mut c = cell(Phase::Streaming(seg(0)), 0, Some(16), 0);
        c.drain = Drain::Ended;
        c.outstanding = Some(Outstanding::Stop);
        let mut cells = vec![c];
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Shutdown);
        assert!(!brain.outbox.iter().any(|c| matches!(c, Command::Rx { op: RxOp::Stop { .. }, .. })), "{:?}", brain.outbox);
        assert!(!brain.outbox.contains(&Command::Rx { stream: 0, op: RxOp::Terminate }), "the stop in progress is awaited");
        cells[0].phase = Phase::Off;
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Finished { stream: 0, next_origin: None, epoch: 1 }));
        assert!(brain.outbox.contains(&Command::Rx { stream: 0, op: RxOp::Terminate }));
    }

    /// A shutdown whose stop leaves accepted debt: the flush follows the stop, and the terminate follows the flush.
    #[test]
    fn u19c_shutdown_flushes_stop_debt_before_terminate() {
        let mut entry = value(0, 8, 1);
        entry.accepted_at = Some(8);
        let mut c = cell(Phase::Streaming(seg(0)), 0, None, 4);
        push(&mut c, entry, ValueState::Done(Terminal::Call(Outcome::Ok)));
        let mut cells = vec![c];
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Shutdown);
        let stop = Command::Rx { stream: 0, op: RxOp::Stop { at: None, epoch: 1 } };
        let flush = Command::Rx { stream: 0, op: RxOp::Flush };
        let terminate = Command::Rx { stream: 0, op: RxOp::Terminate };
        assert_eq!(brain.outbox, vec![stop.clone()]);
        cells[0].phase = Phase::Off;
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Finished { stream: 0, next_origin: None, epoch: 1 }));
        assert!(brain.outbox.contains(&flush) && !brain.outbox.contains(&terminate), "the flush follows the stop, the terminate waits: {:?}", brain.outbox);
        cells[0].retire(1);
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Flushed { stream: 0 }));
        let at = |command: &Command| brain.outbox.iter().position(|c| c == command);
        assert!(at(&flush) < at(&terminate), "the terminate follows the flush: {:?}", brain.outbox);
        assert!(brain.outbox.contains(&Command::SettingsTerminate));
    }

    /// A stop the brain sent is outstanding until its Finished, and the drain's guard keeps a shutdown
    /// from sending a second one.
    #[test]
    fn u19e_outstanding_stop_is_not_resent() {
        let mut c = cell(Phase::Stopping(seg(0)), 0, Some(16), 0);
        c.drain = Drain::Ended;
        c.outstanding = Some(Outstanding::Stop);
        let mut cells = vec![c];
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Shutdown);
        assert!(!brain.outbox.iter().any(|c| matches!(c, Command::Rx { op: RxOp::Stop { .. }, .. })), "{:?}", brain.outbox);
    }

    /// A start and a lifecycle call deferred at shutdown are dropped: no segment starts and no handle
    /// changes once shutdown begins.
    #[test]
    fn u19d_shutdown_drops_deferred_start_and_lifecycle() {
        let mut c = cell(Phase::Streaming(seg(0)), 0, None, 4);
        push(&mut c, value(0, 8, 1), ValueState::Taken);
        let mut cells = vec![c];
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Request(Request::StartNext { stream: 0, at: 12 }));
        feed(&mut brain, &mut cells, Input::Request(Request::Close { stream: 0 }));
        assert_eq!(brain.deferred.len(), 2, "both wait for the streaming chain");
        feed(&mut brain, &mut cells, Input::Shutdown);
        cells[0].phase = Phase::Off;
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Finished { stream: 0, next_origin: None, epoch: 1 }));
        assert!(!brain.outbox.iter().any(|c| matches!(c, Command::Rx { op: RxOp::Start { .. } | RxOp::Close, .. })), "{:?}", brain.outbox);
        assert!(brain.deferred.is_empty());
        assert!(brain.outbox.contains(&Command::Rx { stream: 0, op: RxOp::Terminate }), "{:?}", brain.outbox);
    }

    /// A result that arrives after the stop's Finished makes the flush owed: the done handler requests it.
    #[test]
    fn u09q_result_after_finished_requests_flush() {
        let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, None, 0)];
        cells[0].epoch = 1;
        push(&mut cells[0], value(0, 8, 1), ValueState::Taken);
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Request(Request::StartNext { stream: 0, at: 20 }));
        cells[0].phase = Phase::Off;
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Finished { stream: 0, next_origin: None, epoch: 1 }));
        assert!(brain.outbox.is_empty(), "no flush before the result: {:?}", brain.outbox);
        feed(&mut brain, &mut cells, Input::Done { id: 1, terminal: Terminal::Call(Outcome::Ok), now: 0 });
        assert_eq!(brain.outbox, vec![Command::Rx { stream: 0, op: RxOp::Flush }]);
    }

    /// Case 5, first shape: the stop the brain sent is outstanding until its Finished. The device reads
    /// Off at the stop's result, but the start waits: the value is left only once the Finished arrives.
    #[test]
    fn u09r_start_waits_for_outstanding_stop() {
        let mut cells = vec![cell(Phase::Off, 0, None, 0)];
        cells[0].drain = Drain::Ended;
        cells[0].outstanding = Some(Outstanding::Stop);
        push(&mut cells[0], value(0, 8, 1), ValueState::Taken);
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Request(Request::StartNext { stream: 0, at: 20 }));
        feed(&mut brain, &mut cells, Input::Done { id: 1, terminal: Terminal::Call(Outcome::Ok), now: 0 });
        assert!(brain.outbox.is_empty(), "the start waits for the stop's Finished: {:?}", brain.outbox);
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Finished { stream: 0, next_origin: None, epoch: 1 }));
        assert_eq!(brain.outbox, vec![Command::Rx { stream: 0, op: RxOp::Flush }], "the left value is flushed first: {:?}", brain.outbox);
    }

    /// Case 5, second shape: the start the brain sent is outstanding until its Armed or its failed begin,
    /// so a flush is not decided on the Off phase the start has not yet left. The debt the flush clears
    /// arises while the start is outstanding (a start never goes while a debt is owed).
    #[test]
    fn u09s_flush_waits_for_outstanding_start() {
        let mut cells = vec![cell(Phase::Off, 0, None, 0)];
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Request(Request::StartNext { stream: 0, at: 20 }));
        assert_eq!(brain.outbox, vec![Command::Rx { stream: 0, op: RxOp::Start { at: 20, epoch: 1 } }]);
        push(&mut cells[0], value(0, 8, 1), ValueState::Done(Terminal::Call(Outcome::Ok)));
        cells[0].mark_left(1);
        feed(&mut brain, &mut cells, Input::Request(Request::Flush { stream: 0 }));
        assert_eq!(brain.outbox.len(), 1, "the flush waits for the start: {:?}", brain.outbox);
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::BeginFailed { stream: 0 }));
        assert_eq!(brain.outbox.last(), Some(&Command::Rx { stream: 0, op: RxOp::Flush }), "{:?}", brain.outbox);
    }

    /// A lifecycle call is outstanding until its completion boundary: a close waits for the open's result.
    #[test]
    fn u09t_close_waits_for_outstanding_open() {
        let mut cells = vec![cell(Phase::Off, 0, None, 0)];
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Request(Request::Open { stream: 0 }));
        feed(&mut brain, &mut cells, Input::Request(Request::Close { stream: 0 }));
        assert_eq!(brain.outbox, vec![Command::Rx { stream: 0, op: RxOp::Open }]);
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Opened { stream: 0 }));
        assert_eq!(brain.outbox, vec![Command::Rx { stream: 0, op: RxOp::Open }, Command::Rx { stream: 0, op: RxOp::Close }]);
    }

    /// A flush deferred for a debt is dropped once the cell owes nothing: a start went first (the cell
    /// is Armed) and no left value remains.
    #[test]
    fn u09u_deferred_flush_dropped_without_debt() {
        let mut cells = vec![cell(Phase::Armed(seg(0)), 0, None, 0)];
        let mut brain = Brain::new();
        brain.deferred.push(Request::Flush { stream: 0 });
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Armed { stream: 0 }));
        assert!(brain.deferred.is_empty(), "the flush is dropped: {:?}", brain.deferred);
        assert!(!brain.outbox.contains(&Command::Rx { stream: 0, op: RxOp::Flush }), "{:?}", brain.outbox);
    }

    /// Case 7c's shape: a failed flush stop leaves the cell uncertain with no left value (the receive
    /// executor retired it), so its Flushed owes nothing: no second flush, and shutdown ends the settings executor.
    #[test]
    fn u09v_failed_flush_stop_leaves_no_second_flush() {
        let mut cells = vec![cell(Phase::Uncertain, 0, None, 0)];
        cells[0].outstanding = Some(Outstanding::Flush);
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Flushed { stream: 0 }));
        assert!(brain.outbox.is_empty(), "no second flush: {:?}", brain.outbox);
        feed(&mut brain, &mut cells, Input::Shutdown);
        assert!(brain.outbox.contains(&Command::SettingsTerminate), "shutdown ends the settings executor: {:?}", brain.outbox);
    }

    #[test]
    fn u09h_stale_finished_keeps_new_epoch() {
        let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, Some(16), 4)];
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Stop { stream: 0, requested: 8 });
        feed(&mut brain, &mut cells, Input::PlanCancel { stream: 0, origin: 20, epoch: 2, plan_cut: Some(48) });
        push(&mut cells[0], value(0, 8, 1), ValueState::Taken);
        brain.held.push(held(0, value(0, 8, 2), 1));
        brain.held.push(held(0, value(0, 24, 3), 1));
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Finished { stream: 0, next_origin: None, epoch: 1 }));
        assert_eq!((cells[0].epoch, cells[0].cut), (2, Some(48)));
        // The held value below the new origin (20) is reported; the one at it is carried.
        assert_eq!(brain.dispositions, vec![(2, Disposition::Reported(Reason::NoReservation)), (1, Disposition::Reported(Reason::Left))]);
        assert_eq!(brain.held.iter().map(|held| held.entry.order.1).collect::<Vec<_>>(), vec![3]);
    }

    #[test]
    fn u18d_finished_marks_the_left_values_on_the_cell() {
        let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, None, 0)];
        push(&mut cells[0], value(0, 8, 1), ValueState::Taken);
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Finished { stream: 0, next_origin: None, epoch: 1 }));
        assert_eq!(brain.dispositions, vec![(1, Disposition::Reported(Reason::Left))]);
        assert!(cells[0].is_left(1));
    }

    #[test]
    fn u18e_lost_disposes_each_value_once() {
        let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, None, 0)];
        push(&mut cells[0], value(0, 8, 1), ValueState::Taken);
        push(&mut cells[0], value(0, 12, 2), ValueState::Done(Terminal::Call(Outcome::Ok)));
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Lost { stream: 0 }));
        assert_eq!(cells[0].phase, Phase::Lost);
        assert_eq!(brain.dispositions, vec![(1, Disposition::Reported(Reason::Loss)), (2, Disposition::Reported(Reason::Loss))]);
        feed(&mut brain, &mut cells, Input::Done { id: 1, terminal: Terminal::Call(Outcome::Ok), now: 0 });
        assert_eq!(brain.dispositions.len(), 2, "a late result writes no second disposition");
    }

    #[test]
    fn u10_partial_result_is_maybe_applied_not_failure() {
        let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, Some(16), 4)];
        push(&mut cells[0], value(0, 8, 1), ValueState::MaybeApplied);
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Done { id: 1, terminal: Terminal::Call(Outcome::Partial), now: 0 });
        assert_eq!(cells[0].state_of(1), Some(&ValueState::Done(Terminal::Call(Outcome::Partial))));
        assert!(!cells[0].pinned());
        assert_eq!(brain.results, vec![(1, Terminal::Call(Outcome::Partial))]);
        assert!(brain.dispositions.is_empty());
    }

    #[test]
    fn u11a_late_completion_recorded_once() {
        let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, Some(16), 4)];
        push(&mut cells[0], value(0, 8, 1), ValueState::MaybeApplied);
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Finished { stream: 0, next_origin: None, epoch: 1 }));
        feed(&mut brain, &mut cells, Input::Done { id: 1, terminal: Terminal::Call(Outcome::Ok), now: 0 });
        feed(&mut brain, &mut cells, Input::Done { id: 1, terminal: Terminal::Call(Outcome::Ok), now: 0 });
        assert_eq!(brain.results, vec![(1, Terminal::Call(Outcome::Ok))]);
        assert_eq!(brain.dispositions, vec![(1, Disposition::Reported(Reason::Left))]);
    }

    #[test]
    fn u11b_later_sample_is_leftover_consumption() {
        let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, Some(16), 4)];
        push(&mut cells[0], value(0, 8, 1), ValueState::MaybeApplied);
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Finished { stream: 0, next_origin: None, epoch: 1 }));
        feed(&mut brain, &mut cells, Input::Done { id: 1, terminal: Terminal::Call(Outcome::Ok), now: 0 });
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Consumed { id: 1 }));
        assert_eq!(brain.leftovers, vec![1]);
        assert_eq!(brain.dispositions, vec![(1, Disposition::Reported(Reason::Left))]);
    }

    #[test]
    fn u11c_next_start_waits_for_late_completion() {
        let mut cells = vec![cell(Phase::Off, 0, None, 0)];
        push(&mut cells[0], value(0, 8, 1), ValueState::MaybeApplied);
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Finished { stream: 0, next_origin: None, epoch: 1 }));
        feed(&mut brain, &mut cells, Input::Request(Request::StartNext { stream: 0, at: 20 }));
        assert!(brain.outbox.is_empty());
        assert_eq!(brain.deferred, vec![Request::StartNext { stream: 0, at: 20 }]);
    }

    /// Sol E1 finding 3: once the guard has run the brain is dead: it takes no input.
    #[test]
    fn u13c_brain_is_dead_after_its_guard() {
        let mut cells = vec![cell(Phase::Off, 0, None, 0)];
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Panic);
        assert!(brain.dead());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| brain.step(Input::Shutdown)));
        assert!(result.is_err(), "a dead brain takes no input");
    }

    #[test]
    fn u13b_panic_guard_sets_flag_before_mark_lost() {
        let mut cells = vec![cell(Phase::Off, 0, None, 0)];
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Panic);
        assert_eq!(brain.guard, vec![Guard::Gate, Guard::FlagSet, Guard::Ungate, Guard::MarkLost]);
    }

    #[test]
    fn u14_in_flight_bound_holds_value() {
        let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, Some(16), 0)];
        let mut brain = Brain::new();
        for id in 1..=5 {
            feed(&mut brain, &mut cells, Input::Release(held(0, value(0, 8, id), 1)));
        }
        assert_eq!(brain.held.len(), 1);
        assert_eq!(brain.outbox.len(), 4);
    }

    /// The bound is the brain's own field (the measurement's bound diagnostic sets it): with 6 the fifth and sixth values are reserved and the
    /// seventh waits; a slot that frees reserves it.
    #[test]
    fn u14a_the_bound_is_the_brains_own_parameter() {
        let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, Some(16), 0)];
        let mut brain = Brain::new();
        brain.device_bound = 6;
        brain.chain_bound = 6;
        for id in 1..=7 {
            feed(&mut brain, &mut cells, Input::Release(held(0, value(0, 8 + id as i64, id), 1)));
        }
        assert_eq!((brain.held.len(), brain.outbox.len(), cells[0].in_flight.len()), (1, 6, 6), "the seventh waits at the bound of six");
        assert_eq!(cells[0].take(1), Take::Call);
        feed(&mut brain, &mut cells, Input::Done { id: 1, terminal: Terminal::Call(Outcome::Ok), now: 0 });
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Consumed { id: 1 }));
        assert!(brain.held.is_empty(), "the freed slot reserves the held value");
        assert_eq!((Brain::new().device_bound, Brain::new().chain_bound), (DEVICE_BOUND, CHAIN_BOUND), "the defaults are the design's bounds");
    }

    /// A held value at the in-flight bound is reserved when a slot frees (note 45 §4.5: it waits for capacity).
    #[test]
    fn u14b_held_value_reserved_when_a_slot_frees() {
        let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, Some(16), 0)];
        let mut brain = Brain::new();
        for id in 1..=5 {
            feed(&mut brain, &mut cells, Input::Release(held(0, value(0, 8 + id as i64, id), 1)));
        }
        assert_eq!(brain.held.len(), 1);
        assert_eq!(cells[0].take(1), Take::Call);
        feed(&mut brain, &mut cells, Input::Done { id: 1, terminal: Terminal::Call(Outcome::Ok), now: 0 });
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Consumed { id: 1 }));
        assert!(brain.held.is_empty());
        let settings = settings_sent(&brain);
        assert_eq!(settings.len(), 5);
        assert_eq!(settings.last(), Some(&&timed(5, 0, 5.0, 13, 1)));
    }

    /// Sol E2b-A P1 (adjudicated): the retry uses the reservation rule as it is, so a held value whose instant has passed by the time a
    /// slot frees is reserved and sent, as a late release is today (it will not be applied at its instant).
    #[test]
    fn u14c_held_value_retried_after_its_instant_is_reserved_and_sent() {
        let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, Some(32), 0)];
        let mut brain = Brain::new();
        for id in 1..=5 {
            feed(&mut brain, &mut cells, Input::Release(held(0, value(0, 8 + id as i64, id), 1)));
        }
        assert_eq!(brain.held.len(), 1, "v5 (e = 13) waits at the bound");
        cells[0].progress = 16;
        assert_eq!(cells[0].take(1), Take::Call);
        feed(&mut brain, &mut cells, Input::Done { id: 1, terminal: Terminal::Call(Outcome::Ok), now: 0 });
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Consumed { id: 1 }));
        assert!(brain.held.is_empty());
        assert_eq!(brain.outbox.last(), Some(&timed(5, 0, 5.0, 13, 1)), "sent after its instant (progress 16 > 13)");
        assert!(cells[0].state_of(5).is_some(), "reserved in the cell");
    }

    /// The measurement's attribution of a rejected reservation: the bound, a refusal of the cell (a cell that is not running, a cut), or none (the
    /// value is reservable, or not held at all).
    #[test]
    fn u14d_reservation_refusal_names_the_bound_and_the_cells_refusals() {
        let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, Some(16), 0)];
        let mut brain = Brain::new();
        for id in 1..=5 {
            feed(&mut brain, &mut cells, Input::Release(held(0, value(0, 8 + id as i64, id), 1)));
        }
        assert_eq!(brain.held.len(), 1, "v5 waits at the bound");
        assert_eq!(brain.reservation_refusal(5, &cells), Some(NotReserved::DeviceBound));
        assert_eq!(brain.reservation_refusal(1, &cells), None, "a value that is not held has no refusal");
        // A lost (not running) cell refuses for its phase: not a bound hit.
        cells[0].in_flight.truncate(2);
        cells[0].phase = Phase::Off;
        assert_eq!(brain.reservation_refusal(5, &cells), Some(NotReserved::Refused(Refusal::Phase)));
        // A running cell with room, and a cut at or before the value's instant.
        cells[0].phase = Phase::Streaming(seg(0));
        assert_eq!(brain.reservation_refusal(5, &cells), None, "reservable");
        cells[0].cut = Some(13);
        assert_eq!(brain.reservation_refusal(5, &cells), Some(NotReserved::Refused(Refusal::Cut)));
        assert_eq!(cells[0].in_flight.len(), 2, "asking changes nothing");
    }

    #[test]
    fn u16a_cut_free_late_command_restarts() {
        let mut cells = vec![cell(Phase::Armed(seg(0)), 0, None, 0)];
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::LateCommand { stream: 0, now: 5 });
        assert_eq!(brain.outbox, vec![Command::Rx { stream: 0, op: RxOp::Start { at: 5, epoch: 1 } }]);
    }

    #[test]
    fn u16b_late_command_with_cut_ends_no_start() {
        let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, Some(12), 4)];
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::LateCommand { stream: 0, now: 5 });
        assert_eq!(brain.outbox, vec![Command::Rx { stream: 0, op: RxOp::Stop { at: Some(5), epoch: 1 } }]);
    }

    #[test]
    fn u16c_lost_no_call() {
        let mut cells = vec![cell(Phase::Lost, 0, None, 0)];
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::LateCommand { stream: 0, now: 5 });
        assert!(brain.outbox.is_empty());
    }

    #[test]
    fn u17a_failed_begin_from_uncertain_stays_uncertain() {
        let mut cells = vec![cell(Phase::Uncertain, 0, None, 0)];
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Request(Request::StartNext { stream: 0, at: 20 }));
        cells[0].device_result(Call::Begin(seg(20)), &Err(DeviceError::failed("begin refused")));
        assert_eq!(cells[0].phase, Phase::Uncertain);
        assert!(!brain.outbox.iter().any(|c| matches!(c, Command::Settings(SetCmd { change: Change::Restore { .. }, .. }))));
    }

    #[test]
    fn u17b_failed_stop_is_uncertain() {
        let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, None, 0)];
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Stop { stream: 0, requested: 8 });
        cells[0].device_result(Call::Stop, &Err(DeviceError::failed("stop refused")));
        assert_eq!(cells[0].phase, Phase::Uncertain);
    }

    #[test]
    fn u17c_flush_start_failure_stays_off() {
        let mut cells = vec![cell(Phase::Off, 0, None, 0)];
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Request(Request::Flush { stream: 0 }));
        cells[0].device_result(Call::FlushStart, &Err(DeviceError::failed("flush refused")));
        assert_eq!(cells[0].phase, Phase::Off);
        assert!(!brain.outbox.iter().any(|c| matches!(c, Command::Settings(_))));
    }

    #[test]
    fn u17d_flush_final_stop_failure_uncertain_no_restore() {
        let mut cells = vec![cell(Phase::Off, 0, None, 0)];
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Request(Request::Flush { stream: 0 }));
        cells[0].device_result(Call::FlushStart, &Ok(()));
        cells[0].device_result(Call::FlushStop, &Err(DeviceError::failed("final stop refused")));
        assert_eq!(cells[0].phase, Phase::Uncertain);
        feed(&mut brain, &mut cells, Input::Request(Request::Restore { channel: 0 }));
        assert!(!brain.outbox.iter().any(|c| matches!(c, Command::Settings(_))));
        // Never, not Wait: an Uncertain chain never becomes Off, so a waiting Restore would hold shutdown (E1-36).
        assert!(brain.deferred.is_empty());
    }

    #[test]
    fn u18a_held_carried_when_later_segment_covers() {
        let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, Some(8), 0)];
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Release(held(0, value(0, 12, 1), 1)));
        assert_eq!(brain.held.len(), 1);
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Finished { stream: 0, next_origin: Some(8), epoch: 1 }));
        assert!(brain.dispositions.is_empty());
        cells[0].device_result(Call::Stop, &Ok(()));
        cells[0].device_result(Call::Begin(seg(8)), &Ok(()));
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Armed { stream: 0 }));
        assert_eq!(brain.outbox, vec![timed(1, 0, 1.0, 12, 1)]);
    }

    #[test]
    fn u18b_reported_once_when_no_segment_covers() {
        let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, Some(8), 0)];
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Release(held(0, value(0, 12, 1), 1)));
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Finished { stream: 0, next_origin: None, epoch: 1 }));
        assert_eq!(brain.dispositions, vec![(1, Disposition::Reported(Reason::NoReservation))]);
    }

    #[test]
    fn u18c_shutdown_settles_held_by_settle_rule() {
        let mut cells = vec![cell(Phase::Off, 0, None, 0)];
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Release(held(0, value(3, 12, 1), 1)));
        feed(&mut brain, &mut cells, Input::Shutdown);
        feed(&mut brain, &mut cells, Input::Shutdown);
        assert!(brain.outbox.contains(&Command::Rx { stream: 0, op: RxOp::Terminate }));
        assert_eq!(brain.outbox.iter().filter(|c| **c == Command::Rx { stream: 0, op: RxOp::Terminate }).count(), 1, "the terminates go once");
        let sent = brain.outbox.len();
        feed(&mut brain, &mut cells, Input::Request(Request::Restore { channel: 0 }));
        assert_eq!(brain.outbox.len(), sent, "nothing is sent after the terminates");
        assert_eq!(brain.dispositions, vec![(1, Disposition::Reported(Reason::NoChannel))]);
    }

    #[test]
    fn u04e_second_stop_in_new_segment_books_drain() {
        let (mut cells, mut brain) = pinned();
        feed(&mut brain, &mut cells, Input::Stop { stream: 0, requested: 8 });
        feed(&mut brain, &mut cells, Input::Timer { stream: 0, now: 0 });
        assert_eq!(brain.outbox.last(), Some(&Command::Rx { stream: 0, op: RxOp::Stop { at: None, epoch: 1 } }));
        cells[0].phase = Phase::Streaming(seg(20));
        cells[0].cut = None;
        cells[0].progress = 0;
        brain.outbox.clear();
        feed(&mut brain, &mut cells, Input::Stop { stream: 0, requested: 30 });
        assert_eq!(brain.outbox, vec![Command::Timer { stream: 0, at: 38 }]);
    }

    #[test]
    fn u09h_start_waits_for_quiesce_ack() {
        let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, Some(16), 4)];
        cells[0].advance_epoch(2, Some(48));
        cells[0].phase = Phase::Off;
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Request(Request::StartNext { stream: 0, at: 20 }));
        assert!(brain.outbox.is_empty());
        let seen = cells[0].snapshot();
        cells[0].publish(seen, &recv(1, &[]));
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::Quiesced { stream: 0 }));
        assert_eq!(brain.outbox, vec![Command::Rx { stream: 0, op: RxOp::Start { at: 20, epoch: 2 } }]);
    }

    #[test]
    fn u17e_refused_settings_command_writes_no_value_disposition() {
        let mut cells = vec![cell(Phase::Off, 0, None, 0)];
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Request(Request::Restore { channel: 0 }));
        let Command::Settings(SetCmd { id, .. }) = brain.outbox[0].clone() else { panic!("a restore is sent") };
        feed(&mut brain, &mut cells, Input::Done { id, terminal: Terminal::Call(Outcome::Refused), now: 0 });
        assert!(brain.dispositions.is_empty());
    }

    #[test]
    fn u17f_flush_not_blocked_by_debt() {
        let mut cells = vec![cell(Phase::Off, 0, None, 0)];
        push(&mut cells[0], value(0, 8, 1), ValueState::Done(Terminal::Call(Outcome::Ok)));
        cells[0].mark_left(1);
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Request(Request::Flush { stream: 0 }));
        assert_eq!(brain.outbox, vec![Command::Rx { stream: 0, op: RxOp::Flush }]);
    }

    #[test]
    fn u16d_late_stop_takes_stopping_transition_and_no_second_stop() {
        let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, Some(12), 4)];
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::LateCommand { stream: 0, now: 5 });
        assert_eq!(brain.outbox, vec![Command::Rx { stream: 0, op: RxOp::Stop { at: Some(5), epoch: 1 } }]);
        assert!(matches!(cells[0].phase, Phase::Stopping(_)));
        feed(&mut brain, &mut cells, Input::Release(held(0, value(0, 8, 2), 1)));
        assert_eq!(brain.held.len(), 1);
        feed(&mut brain, &mut cells, Input::Timer { stream: 0, now: 0 });
        feed(&mut brain, &mut cells, Input::Stop { stream: 0, requested: 20 });
        feed(&mut brain, &mut cells, Input::Timer { stream: 0, now: 0 });
        // The cut is reached after the late stop: its drain is over (Drain::Ended), so no second stop follows (E1-71).
        cells[0].progress = 12;
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::CutReached { stream: 0 }));
        assert_eq!(brain.outbox.len(), 1, "the late stop is the only stop");
    }

    #[test]
    fn u04f_stop_booking_keeps_values_before_cut_acceptable() {
        let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, None, 0)];
        let mut brain = Brain::new();
        feed(&mut brain, &mut cells, Input::Release(held(0, value(0, 50, 1), 1)));
        feed(&mut brain, &mut cells, Input::Stop { stream: 0, requested: 100 });
        assert_eq!(cells[0].state_of(1), Some(&ValueState::Queued));
        assert!(matches!(cells[0].phase, Phase::Streaming(_)));
        brain.outbox.clear();
        feed(&mut brain, &mut cells, Input::Release(held(0, value(0, 60, 2), 1)));
        assert!(matches!(brain.outbox.as_slice(), [Command::Settings(SetCmd { id: 2, .. })]));
    }

    #[test]
    fn u04g_cut_reached_while_pinned_waits_for_the_pin_to_clear() {
        let (mut cells, mut brain) = pinned();
        feed(&mut brain, &mut cells, Input::Stop { stream: 0, requested: 8 });
        cells[0].progress = 8;
        brain.outbox.clear();
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::CutReached { stream: 0 }));
        assert!(brain.outbox.is_empty());
        feed(&mut brain, &mut cells, Input::Done { id: 1, terminal: Terminal::Call(Outcome::Ok), now: 0 });
        assert_eq!(brain.outbox, vec![Command::Rx { stream: 0, op: RxOp::Stop { at: None, epoch: 1 } }]);
        feed(&mut brain, &mut cells, Input::Boundary(Boundary::CutReached { stream: 0 }));
        feed(&mut brain, &mut cells, Input::Timer { stream: 0, now: 0 });
        assert_eq!(brain.outbox.len(), 1, "one stop");
    }
}
