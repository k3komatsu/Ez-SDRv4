//! The simulated world of design note 45's experiment (stage E1b, part 2a; 45-experiment §3.1–§3.2,
//! test plan §A): every actor and resource as state, the §3.2 events as atomic steps (plan change and the rate read are not yet in; see the 2a report), and the
//! monitor's record of facts. It checks nothing: the oracles are part 2b and the case scripts part 3.
//! The search is [`super::dst`]; this module implements its [`Model`] for the world. The decisions
//! stay where they live: the brain in `core`, the cell procedures in `cell`, the executors' rules in
//! `rules` and the device in `model`. The world only sequences them.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use ezsdr_radio::timeline::Channels;

use super::cell::{Cell, Drain, Outcome, Phase, Recv, Snapshot, Take, Terminal, ValueState};
use super::core::{Boundary, Brain, Change, Command, Disposition, Guard, Input, Op, Request, RxOp, Section, SetCmd};
use super::dst::Model;
use super::model::{Apply, Device, Dsp};
use super::rules::{self, CallPlan, CallResult, Check, DevCall, DeviceOp, FlushEnd, Handle, Next, Received, Round, RxCall, has_debt};
use crate::provider::core::{RxEntry, SegmentKey};

/// The witness names a later check reads (W7b3 reads W7b2; `witness_issued` reads the three waited markers): they decide a
/// future witness, so the state key keeps them (pruning a state must not lose a witness the pruned path would reach).
const READ_LATER: [&str; 4] = ["W7b2", "restore waited", "start waited", "move waited"];

/// The samples a recv asks for: one packet.
pub(crate) const PACKET: usize = 4;

/// The id of a write that is already in the device's DSP FIFO when a case starts (initial device
/// state, never reserved by the brain), so the monitor does not track it.
pub(crate) const INITIAL_OCCUPANT: u64 = 0;

/// The parameters a case sets (45-experiment §4). The run ends at `horizon + epilogue`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Params {
    pub(crate) horizon: i64,
    pub(crate) epilogue: i64,
    pub(crate) recv_timeout: i64,
    pub(crate) queue_cap: usize,
    pub(crate) fifo_cap: usize,
    pub(crate) drain_window: i64,
    /// The samples of one receive packet: a recv returns whole packets only (the one place that reads it; `PACKET` is the default).
    pub(crate) packet: usize,
    /// Whether a device call may return failed (`RFault`): the case's branch of §3.2's outcome.
    pub(crate) call_faults: bool,
    /// The brain step (counted from 0) whose input is replaced by a panic (case 6); `None` for no panic.
    pub(crate) panic_step: Option<usize>,
}

/// Who holds the native lock `L` or the lifecycle gate `G`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Holder {
    Brain,
    Settings,
    Rx(usize),
    Clock,
}

/// The four events of publishing a loss (R5; note 45 §4.5 procedure 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PubStep {
    Gate,
    Set,
    Ungate,
    Mark,
}

pub(crate) const PUB_STEPS: [PubStep; 4] = [PubStep::Gate, PubStep::Set, PubStep::Ungate, PubStep::Mark];

pub(crate) fn guard_step(guard: Guard) -> PubStep {
    match guard {
        Guard::Gate => PubStep::Gate,
        Guard::FlagSet => PubStep::Set,
        Guard::Ungate => PubStep::Ungate,
        Guard::MarkLost => PubStep::Mark,
    }
}

/// The steps of the settings executor's program, one event each.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SStep {
    Check,
    Lock,
    Effect,
    Unlock,
    Result,
    Send,
    Pub(PubStep),
}

/// The steps of a receive executor's program, one event each.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RStep {
    Cmd,
    Snap,
    Begin,
    End,
    Publish,
    Lock,
    Call(RxCall),
    Unlock,
    Gate,
    Ungate,
    Result(RxCall),
    Send(Input),
    Pub(PubStep),
}

/// The settings executor `S` (45-experiment §3.1): its FIFO, the command in hand, its terminal once
/// decided, and the steps it runs.
#[derive(Clone, Debug, Default)]
pub(crate) struct SettingsExec {
    pub(crate) fifo: VecDeque<SetCmd>,
    pub(crate) hand: Option<SetCmd>,
    pub(crate) terminal: Option<Terminal>,
    pub(crate) program: VecDeque<SStep>,
    pub(crate) exited: bool,
    /// Shutdown's terminate: `S` exits once the commands queued before it are taken.
    pub(crate) terminate: bool,
}

/// A recv in progress, with the time it times out at.
#[derive(Clone, Debug)]
pub(crate) struct RecvCall {
    pub(crate) recv: Recv,
    pub(crate) deadline: i64,
}

/// A receive executor `R_k`: its FIFO, its steps, its streamer handle (the id of the open handle),
/// the recv in progress, the snapshot and call result it holds, and the values it has reported.
#[derive(Clone, Debug, Default)]
pub(crate) struct RxExec {
    pub(crate) fifo: VecDeque<RxOp>,
    pub(crate) program: VecDeque<RStep>,
    pub(crate) handle: Option<u64>,
    pub(crate) handles: u64,
    pub(crate) recv: Option<RecvCall>,
    pub(crate) snap: Option<Snapshot>,
    pub(crate) call: Option<CallResult>,
    pub(crate) consumed: BTreeSet<u64>,
    /// The time the flush's recv cycles stop reading left values, set when its start succeeds.
    pub(crate) flush_end: Option<i64>,
    pub(crate) exited: bool,
}

/// The clock reader's hold: the late input it serves, and the time it read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ClockHold {
    pub(crate) slot: usize,
    pub(crate) now: Option<i64>,
}

/// What an environment slot delivers.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum EnvInput {
    /// An input for the brain's inbox.
    Input(Input),
    /// The device is lost.
    Loss,
    /// A late command: the clock reader reads the time under `L` and delivers it.
    Late { stream: usize },
}

/// A scripted input with its window: delivered once, in `[earliest, latest]`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EnvSlot {
    pub(crate) input: EnvInput,
    pub(crate) earliest: i64,
    pub(crate) latest: i64,
    pub(crate) delivered: bool,
    /// Delivered only after this slot is (the case's order of two inputs, e.g. equal-time releases).
    pub(crate) after: Option<usize>,
}

/// The identity of a fact about a value: the value, its channel, the segment and epoch of its
/// reservation, and the sample tick the fact is about (45-experiment §3.3).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Ident {
    pub(crate) value: u64,
    pub(crate) channel: usize,
    pub(crate) segment: SegmentKey,
    pub(crate) epoch: u64,
    pub(crate) sample: i64,
}

/// An actor whose latest `F` check the monitor records (I1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Who {
    Settings,
    Rx(usize),
}

/// The oracle's own record (45-experiment §3.3). It keeps three kinds of fact apart: device
/// application, qualifying receive evidence and publication (leftover consumption is a separate
/// fact). It records and checks nothing; the brain and the executors never read it.
#[derive(Clone, Debug, Default)]
pub(crate) struct Monitor {
    /// The sample at which the device applied a timed write.
    pub(crate) applied: BTreeMap<u64, Ident>,
    /// The sample that consumed a value, published or discarded.
    pub(crate) evidence: BTreeMap<u64, Ident>,
    /// A leftover consumption: the value was left at its segment's end.
    pub(crate) leftovers: BTreeMap<u64, Ident>,
    /// The progress each epoch has published.
    pub(crate) published: BTreeMap<u64, i64>,
    /// Each reserved value's channel, segment and epoch.
    pub(crate) reserved: BTreeMap<u64, (usize, SegmentKey, u64)>,
    /// The brain's first write per value, copied.
    pub(crate) dispositions: BTreeMap<u64, Disposition>,
    /// Each settings command's terminal, as the executor wrote it.
    pub(crate) terminals: BTreeMap<u64, Terminal>,
    /// Deadlines delivered, `(stream, at, tick)`; a recv timeout is recorded the same way.
    pub(crate) observed: BTreeSet<(usize, i64, i64)>,
    /// Streams whose drain has ended.
    pub(crate) drain_ended: BTreeSet<usize>,
    /// A loss was marked lost.
    pub(crate) marked: bool,
    /// The value ids released so far (the inbox's `Release` inputs).
    pub(crate) released: BTreeSet<u64>,
    /// The settings commands the brain has routed to `S`.
    pub(crate) commanded: BTreeSet<u64>,
    /// The shutdown input has been delivered.
    pub(crate) shutdown: bool,
    /// The drain deadlines delivered, `(stream, at)`.
    pub(crate) drain_observed: BTreeSet<(usize, i64)>,
    /// Each actor's latest `F` check, and whether it saw `F` set, until its next device call (I1).
    pub(crate) checked: BTreeMap<Who, bool>,
    /// The brain has taken a panic input.
    pub(crate) panicked: bool,
    /// The values whose replaced release (a panic took the input) is not a disposition's to write (P2).
    pub(crate) panic_reported: BTreeSet<u64>,
    /// The witnesses recorded at the event where each happens (the case's declared names, section D).
    pub(crate) witnesses: BTreeSet<&'static str>,
    /// A taken value whose cut a stop shortened: the cut it was shortened to (W1c).
    pub(crate) shortened: BTreeMap<u64, i64>,
}

/// What one publish gave the user: the stream and epoch, the ticks, and how many samples it discarded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Published {
    pub(crate) stream: usize,
    pub(crate) epoch: u64,
    pub(crate) ticks: Vec<i64>,
    pub(crate) discarded: u64,
}

/// A consumption fact: the sample that consumed `value` on `stream`, over `channel`; `leftover` for a value left at its
/// segment's end (its own list, not a second disposition).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Fact {
    pub(crate) leftover: bool,
    pub(crate) value: u64,
    pub(crate) stream: usize,
    pub(crate) channel: usize,
    pub(crate) sample: i64,
}

/// A step the world can take. Each variant is one event of 45-experiment §3.2.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Event {
    Tick,
    Env(usize),
    Timer(usize),
    BStep,
    BOp,
    ClockLock(usize),
    ClockRead,
    ClockUnlock,
    ClockSend,
    STake,
    SDiscard,
    SCheck,
    SLock,
    SEffect(Outcome),
    SUnlock,
    SResult,
    SSend,
    RCheck(usize),
    RCmd(usize),
    RSnap(usize),
    RBegin(usize),
    REnd(usize),
    RPublish(usize),
    RLock(usize),
    RCall(usize),
    RFault(usize),
    RUnlock(usize),
    RGate(usize),
    RUngate(usize),
    RResult(usize),
    RSend(usize),
    Pub(Holder, PubStep),
}

/// The whole simulated system. Stream `k` is cell `k`, executor `rx[k]` and streamer `k`.
#[derive(Clone, Debug)]
pub(crate) struct World {
    pub(crate) params: Params,
    pub(crate) brain: Brain,
    pub(crate) inbox: VecDeque<Input>,
    pub(crate) cells: Vec<Cell>,
    pub(crate) settings: SettingsExec,
    pub(crate) rx: Vec<RxExec>,
    pub(crate) clock: Option<ClockHold>,
    /// The native control lock `L`.
    pub(crate) native: Option<Holder>,
    /// The lifecycle gate `G`.
    pub(crate) gate: Option<Holder>,
    /// The failure flag `F`.
    pub(crate) failure: bool,
    pub(crate) device: Device,
    pub(crate) env: Vec<EnvSlot>,
    /// Brain-booked deadlines `(stream, at)`, not yet delivered.
    pub(crate) timers: Vec<(usize, i64)>,
    /// The events taken so far (diagnostic; not in the key).
    pub(crate) trace: Vec<Event>,
    /// What each publish gave the user, `(stream, epoch, ticks)` (diagnostic, outside the key: the observation of S1).
    pub(crate) published: Vec<Published>,
    /// Every settings terminal in the order it was written (diagnostic, outside the key).
    pub(crate) terminal_log: Vec<(u64, Terminal)>,
    /// Every consumption fact in the order it was written (diagnostic, outside the key).
    pub(crate) fact_log: Vec<Fact>,
    /// Every call made on the device, in order (diagnostic, outside the key).
    pub(crate) calls: Vec<DevCall>,
    pub(crate) monitor: Monitor,
    /// The brain steps taken, capped at the panic step plus one (the panic's position is part of the state only there).
    pub(crate) steps: usize,
}

/// One abandoned settings command: each cell's section, the terminal, and the `Done` to the brain.
fn abandon_command(cells: &mut [Cell], monitor: &mut Monitor, log: &mut Vec<(u64, Terminal)>, inbox: &mut VecDeque<Input>, id: u64) {
    rules::abandon_all(cells, id);
    monitor.terminals.insert(id, Terminal::Abandoned);
    log.push((id, Terminal::Abandoned));
    inbox.push_back(Input::Done { id, terminal: Terminal::Abandoned });
}

impl World {
    /// A world of one stream per cell, with no stream open and no command queued.
    pub(crate) fn new(cells: Vec<Cell>, env: Vec<EnvSlot>, params: Params) -> World {
        let streams = cells.len();
        let mut brain = Brain::new();
        brain.drain_window = params.drain_window;
        let mut device = Device::new(params.queue_cap, params.fifo_cap);
        device.silent_from = params.horizon + params.epilogue;
        World {
            params,
            brain,
            inbox: VecDeque::new(),
            cells,
            settings: SettingsExec::default(),
            rx: vec![RxExec::default(); streams],
            clock: None,
            native: None,
            gate: None,
            failure: false,
            device,
            env,
            timers: Vec::new(),
            trace: Vec::new(),
            published: Vec::new(),
            terminal_log: Vec::new(),
            fact_log: Vec::new(),
            calls: Vec::new(),
            monitor: Monitor::default(),
            steps: 0,
        }
    }

    fn s_front(&self) -> Option<&SStep> {
        self.settings.program.front()
    }

    fn r_front(&self, k: usize) -> Option<&RStep> {
        self.rx.get(k).and_then(|r| r.program.front())
    }

    fn pub_front(&self, holder: Holder) -> Option<PubStep> {
        match holder {
            Holder::Settings => match self.s_front() {
                Some(SStep::Pub(step)) => Some(*step),
                _ => None,
            },
            Holder::Rx(k) => match self.r_front(k) {
                Some(RStep::Pub(step)) => Some(*step),
                _ => None,
            },
            Holder::Brain | Holder::Clock => None,
        }
    }

    fn in_window(&self, slot: &EnvSlot) -> bool {
        slot.earliest <= self.device.t && self.device.t <= slot.latest
    }

    /// The order of the case's inputs: a slot waits for the one it is `after`, and a shutdown waits
    /// for every other input (section D: shutdown is last).
    fn ordered(&self, i: usize) -> bool {
        let slot = &self.env[i];
        slot.after.is_none_or(|j| self.env[j].delivered) && (!matches!(slot.input, EnvInput::Input(Input::Shutdown)) || self.env.iter().enumerate().all(|(j, other)| j == i || other.delivered))
    }

    /// Whether a device loss now is at a point a case names (W2a–W2c): S holds a command before its
    /// call or after it, or R waits in its recv.
    fn loss_witnesses(&mut self) {
        let holds = self.settings.hand.is_some();
        let front = self.s_front().cloned();
        match front {
            Some(SStep::Check | SStep::Lock) if holds => self.monitor.witnesses.insert("W2a"),
            Some(SStep::Unlock | SStep::Pub(_) | SStep::Result) if holds => self.monitor.witnesses.insert("W2b"),
            _ => false,
        };
        if (0..self.rx.len()).any(|k| self.r_front(k) == Some(&RStep::End)) {
            self.monitor.witnesses.insert("W2c");
        }
    }

    fn receiving(&self, k: usize) -> bool {
        rules::receiving(&self.cells[k], self.rx[k].handle.is_some())
    }

    fn flush_over(&self, k: usize) -> bool {
        rules::flush_over(&self.cells[k], &self.rx[k].consumed, self.rx[k].flush_end, self.device.t, self.at_end())
    }

    /// A recv may end: the device is lost, a packet is buffered, a stopped stream's tail is
    /// buffered, a missed start is due, or its timeout has passed.
    fn ready(&self, k: usize, deadline: i64) -> bool {
        self.packet_ready(k) || self.device.t >= deadline
    }

    /// The end of the epilogue (45-experiment §3.2): the device produces no more samples. Time goes on
    /// (silent ticks) while a deadline lies ahead, so a timer or a recv timeout fires at its real time.
    pub(crate) fn at_end(&self) -> bool {
        self.device.t >= self.params.horizon + self.params.epilogue
    }

    /// A timer, or the timeout of a recv in progress, lies beyond the present time.
    fn awaits_time(&self) -> bool {
        let t = self.device.t;
        self.timers.iter().any(|(_, at)| *at > t) || self.rx.iter().any(|r| r.recv.as_ref().is_some_and(|call| call.deadline > t))
    }

    fn packet_ready(&self, k: usize) -> bool {
        self.device.packet_ready(k, self.params.packet)
    }

    /// The effect of a settings outcome is possible now: a lost device returns lost; a timed
    /// command waits for the queue, or is refused by a full DSP FIFO.
    fn effect_allowed(&self, outcome: Outcome) -> bool {
        if self.device.lost {
            return outcome == Outcome::Lost;
        }
        if outcome == Outcome::Lost {
            return false;
        }
        match self.settings.hand.as_ref().map(|cmd| &cmd.change) {
            Some(Change::Timed { channel, .. }) => match self.device.room(*channel) {
                None => matches!(outcome, Outcome::Ok | Outcome::Partial),
                Some(Apply::Refused) => outcome == Outcome::Refused,
                _ => false,
            },
            _ => outcome == Outcome::Ok,
        }
    }

    fn publish_allowed(&self, holder: Holder, step: PubStep) -> bool {
        match step {
            PubStep::Gate => self.gate.is_none(),
            PubStep::Set | PubStep::Ungate => self.gate == Some(holder),
            PubStep::Mark => true,
        }
    }

    fn publish(&mut self, holder: Holder, step: PubStep) {
        match step {
            PubStep::Gate => self.gate = Some(holder),
            PubStep::Set => self.failure = true,
            PubStep::Ungate => self.gate = None,
            PubStep::Mark => self.monitor.marked = true,
        }
    }

    /// Whether `event` is enabled in this state.
    fn allowed(&self, event: &Event) -> bool {
        match *event {
            Event::Tick => (!self.at_end() || self.awaits_time()) && self.env.iter().all(|slot| slot.delivered || slot.latest > self.device.t),
            Event::Env(i) => self.env.get(i).is_some_and(|slot| !slot.delivered && !matches!(slot.input, EnvInput::Late { .. }) && self.in_window(slot) && self.ordered(i)),
            Event::Timer(i) => self.timers.get(i).is_some_and(|(_, at)| self.device.t >= *at),
            Event::BStep => !self.brain.dead() && self.brain.ops.is_empty() && !self.inbox.is_empty(),
            Event::BOp => self.brain.ops.front().is_some_and(|op| match op {
                Op::Guard(guard) => self.publish_allowed(Holder::Brain, guard_step(*guard)),
                _ => true,
            }),
            Event::ClockLock(i) => {
                self.clock.is_none()
                    && self.native.is_none()
                    && self.env.get(i).is_some_and(|slot| !slot.delivered && matches!(slot.input, EnvInput::Late { .. }) && self.in_window(slot) && self.ordered(i))
            }
            Event::ClockRead => self.clock.is_some_and(|hold| hold.now.is_none() && self.native == Some(Holder::Clock)),
            Event::ClockUnlock => self.clock.is_some_and(|hold| hold.now.is_some() && self.native == Some(Holder::Clock)),
            Event::ClockSend => self.clock.is_some_and(|hold| hold.now.is_some() && self.native.is_none()),
            Event::STake => !self.settings.exited && self.settings.program.is_empty() && self.settings.hand.is_none() && (!self.settings.fifo.is_empty() || self.settings.terminate),
            Event::SDiscard => self.settings.exited && !self.settings.fifo.is_empty(),
            Event::SCheck => self.s_front() == Some(&SStep::Check),
            Event::SLock => self.s_front() == Some(&SStep::Lock) && self.native.is_none(),
            Event::SEffect(outcome) => self.s_front() == Some(&SStep::Effect) && self.effect_allowed(outcome),
            Event::SUnlock => self.s_front() == Some(&SStep::Unlock) && self.native == Some(Holder::Settings),
            Event::SResult => self.s_front() == Some(&SStep::Result),
            Event::SSend => self.s_front() == Some(&SStep::Send),
            Event::RCheck(k) => {
                let r = &self.rx[k];
                !r.exited && r.program.is_empty() && rules::next_action(self.failure, !r.fifo.is_empty(), self.flush_over(k), self.receiving(k), self.at_end()) != Next::Idle
            }
            Event::RCmd(k) => self.r_front(k) == Some(&RStep::Cmd),
            Event::RSnap(k) => self.r_front(k) == Some(&RStep::Snap),
            Event::RBegin(k) => self.r_front(k) == Some(&RStep::Begin) && self.rx[k].snap.is_some() && self.rx[k].handle.is_some(),
            Event::REnd(k) => self.r_front(k) == Some(&RStep::End) && self.rx[k].recv.as_ref().is_some_and(|call| self.device.lost || self.ready(k, call.deadline)),
            Event::RPublish(k) => self.r_front(k) == Some(&RStep::Publish) && self.rx[k].recv.is_some(),
            Event::RLock(k) => self.r_front(k) == Some(&RStep::Lock) && self.native.is_none(),
            Event::RCall(k) => matches!(self.r_front(k), Some(RStep::Call(_))) && self.native == Some(Holder::Rx(k)),
            Event::RFault(k) => self.params.call_faults && !self.device.lost && matches!(self.r_front(k), Some(RStep::Call(_))) && self.native == Some(Holder::Rx(k)),
            Event::RUnlock(k) => self.r_front(k) == Some(&RStep::Unlock) && self.native == Some(Holder::Rx(k)),
            Event::RGate(k) => self.r_front(k) == Some(&RStep::Gate) && self.gate.is_none(),
            Event::RUngate(k) => self.r_front(k) == Some(&RStep::Ungate) && self.gate == Some(Holder::Rx(k)),
            Event::RResult(k) => matches!(self.r_front(k), Some(RStep::Result(_))),
            Event::RSend(k) => matches!(self.r_front(k), Some(RStep::Send(_))),
            Event::Pub(holder, step) => self.pub_front(holder) == Some(step) && self.publish_allowed(holder, step),
        }
    }

    /// Every event the world could take, enabled or not.
    fn candidates(&self) -> Vec<Event> {
        let mut events = vec![
            Event::Tick,
            Event::BStep,
            Event::BOp,
            Event::ClockRead,
            Event::ClockUnlock,
            Event::ClockSend,
            Event::STake,
            Event::SDiscard,
            Event::SCheck,
            Event::SLock,
            Event::SUnlock,
            Event::SResult,
            Event::SSend,
        ];
        events.extend((0..self.env.len()).flat_map(|i| [Event::Env(i), Event::ClockLock(i)]));
        events.extend((0..self.timers.len()).map(Event::Timer));
        events.extend([Outcome::Ok, Outcome::Partial, Outcome::Refused, Outcome::Lost].map(Event::SEffect));
        for step in PUB_STEPS {
            events.push(Event::Pub(Holder::Settings, step));
            events.extend((0..self.rx.len()).map(|k| Event::Pub(Holder::Rx(k), step)));
        }
        for k in 0..self.rx.len() {
            events.extend([
                Event::RCheck(k),
                Event::RCmd(k),
                Event::RSnap(k),
                Event::RBegin(k),
                Event::REnd(k),
                Event::RPublish(k),
                Event::RLock(k),
                Event::RCall(k),
                Event::RFault(k),
                Event::RUnlock(k),
                Event::RGate(k),
                Event::RUngate(k),
                Event::RResult(k),
                Event::RSend(k),
            ]);
        }
        events
    }

    /// Takes one enabled event. Each arm is one atomic step.
    fn run(&mut self, event: &Event) {
        match *event {
            Event::Tick => self.device.tick(),
            Event::Env(i) => {
                let input = self.env[i].input.clone();
                self.env[i].delivered = true;
                match input {
                    EnvInput::Input(input) => {
                        self.monitor.shutdown |= input == Input::Shutdown;
                        self.inbox.push_back(input);
                    }
                    EnvInput::Loss => {
                        self.loss_witnesses();
                        self.device.lost = true;
                    }
                    EnvInput::Late { .. } => unreachable!("the clock reader delivers late inputs"),
                }
            }
            Event::Timer(i) => {
                if self.native == Some(Holder::Settings) {
                    self.monitor.witnesses.insert("W5a");
                }
                let (stream, at) = self.timers.remove(i);
                self.inbox.push_back(Input::Timer { stream });
                self.monitor.observed.insert((stream, at, self.device.t));
                self.monitor.drain_observed.insert((stream, at));
            }
            Event::BStep => {
                let input = self.inbox.pop_front().expect("an input is queued");
                let at = self.steps;
                self.steps = (self.steps + 1).min(self.params.panic_step.map_or(0, |k| k + 1));
                let input = if rules::panics_at(at, self.params.panic_step) { self.panic_at(input) } else { input };
                match &input {
                    Input::Release(held) => {
                        self.monitor.released.insert(held.entry.order.1);
                    }
                    Input::Panic => self.monitor.panicked = true,
                    Input::Request(Request::Restore { .. }) if self.monitor.witnesses.contains("W7b2") => {
                        self.monitor.witnesses.insert("W7b3");
                    }
                    _ => {}
                }
                self.brain.step(input);
            }
            Event::BOp => self.brain_op(),
            Event::ClockLock(i) => {
                self.native = Some(Holder::Clock);
                self.clock = Some(ClockHold { slot: i, now: None });
            }
            Event::ClockRead => {
                let now = self.device.t;
                if let Some(hold) = self.clock.as_mut() {
                    hold.now = Some(now);
                }
            }
            Event::ClockUnlock => self.native = None,
            Event::ClockSend => {
                if self.failure {
                    self.monitor.witnesses.insert("W7a");
                }
                let hold = self.clock.take().expect("a clock hold");
                let EnvInput::Late { stream } = self.env[hold.slot].input else { unreachable!("the clock serves late inputs") };
                self.env[hold.slot].delivered = true;
                self.inbox.push_back(Input::LateCommand { stream, now: hold.now.expect("the time was read") });
            }
            Event::STake => self.s_take(),
            Event::SDiscard => self.s_discard(),
            Event::SCheck => self.s_check(),
            Event::SLock => {
                self.settings.program.pop_front();
                self.native = Some(Holder::Settings);
                if let Some(id) = self.settings.hand.as_ref().map(|cmd| cmd.id) {
                    rules::start_calls(&mut self.cells, id);
                }
            }
            Event::SEffect(outcome) => self.s_effect(outcome),
            Event::SUnlock => {
                self.settings.program.pop_front();
                self.native = None;
            }
            Event::SResult => self.s_result(),
            Event::SSend => {
                self.settings.program.pop_front();
                let hand = self.settings.hand.take().expect("a command in hand");
                let terminal = self.settings.terminal.take().expect("a decided terminal");
                self.inbox.push_back(Input::Done { id: hand.id, terminal });
            }
            Event::RCheck(k) => self.r_check(k),
            Event::RCmd(k) => self.r_cmd(k),
            Event::RSnap(k) => {
                self.rx[k].program.pop_front();
                self.rx[k].snap = Some(self.cells[k].snapshot());
            }
            Event::RBegin(k) => self.r_begin(k),
            Event::REnd(k) => self.r_end(k),
            Event::RPublish(k) => self.r_publish(k),
            Event::RLock(k) => {
                self.rx[k].program.pop_front();
                self.native = Some(Holder::Rx(k));
            }
            Event::RCall(k) => self.r_call(k, false),
            Event::RFault(k) => self.r_call(k, true),
            Event::RUnlock(k) => {
                self.rx[k].program.pop_front();
                self.native = None;
            }
            Event::RGate(k) => self.r_gate(k),
            Event::RUngate(k) => {
                self.rx[k].program.pop_front();
                self.gate = None;
            }
            Event::RResult(k) => self.r_result(k),
            Event::RSend(k) => {
                let Some(RStep::Send(input)) = self.rx[k].program.pop_front() else { unreachable!("allowed only at a send") };
                self.inbox.push_back(input);
            }
            Event::Pub(holder, step) => {
                match holder {
                    Holder::Settings => {
                        self.settings.program.pop_front();
                    }
                    Holder::Rx(k) => {
                        self.rx[k].program.pop_front();
                    }
                    Holder::Brain | Holder::Clock => {}
                }
                self.publish(holder, step);
            }
        }
    }

    /// The brain's panic at step `k` (case 6): the input is replaced. A held release is still
    /// released, and P2 settles it (`panic_reported`). Records W6a and W6b at the moment.
    fn panic_at(&mut self, input: Input) -> Input {
        if self.settings.hand.is_some() {
            self.monitor.witnesses.insert("W6a");
        }
        if self.rx.iter().any(|r| r.recv.is_some() && r.program.front() == Some(&RStep::End)) {
            self.monitor.witnesses.insert("W6b");
        }
        if let Input::Release(held) = &input {
            let id = held.entry.order.1;
            self.monitor.released.insert(id);
            self.monitor.panic_reported.insert(id);
        }
        Input::Panic
    }

    /// One brain operation (one cell section or one send). A guard operation acts on the gate and
    /// the flag first, as its own publication step (E1-18 orders them). The witnesses that read the
    /// state before the operation, or the cut it shortens, are recorded around it.
    fn brain_op(&mut self) {
        let op = self.brain.ops.front().cloned().expect("an operation is queued");
        if let Op::Guard(guard) = op {
            self.publish(Holder::Brain, guard_step(guard));
        }
        let reserve = rules::reserving(&self.brain, &op);
        let held = reserve.map(|(_, stream, channel)| (stream, channel));
        self.witness_before(&op, held);
        let stop = match op {
            Op::Section(Section::Stop { stream, .. }) => Some((stream, self.cells[stream].cut)),
            _ => None,
        };
        let timer = match op {
            Op::Section(Section::TimerFired { stream }) => Some((stream, self.cells[stream].drain == Drain::Draining && self.cells[stream].pinned())),
            _ => None,
        };
        self.brain.op(&mut self.cells);
        if self.brain.dead() {
            // The guard has run: the brain is gone. Its bookings fire on nothing, and its senders close with it, so each executor
            // ends after what it already holds. What reaches its inbox is never served.
            self.timers.clear();
            for command in rules::dead_commands(self.rx.len()) {
                self.route(command);
            }
        }
        let commands = std::mem::take(&mut self.brain.outbox);
        self.witness_issued(&commands);
        for command in commands {
            self.route(command);
        }
        if let Some((stream, Some(before))) = stop {
            if let Some(cut) = self.cells[stream].cut.filter(|cut| *cut < before) {
                for flight in self.cells[stream].in_flight.iter().filter(|f| matches!(f.state, ValueState::Taken | ValueState::MaybeApplied)) {
                    self.monitor.shortened.insert(flight.entry.order.1, cut);
                }
            }
        }
        if let Some((stream, true)) = timer {
            if self.cells[stream].drain == Drain::Ended {
                self.monitor.witnesses.insert("W5c");
            }
        }
        if let Some((id, channel, segment, epoch)) = reserve.and_then(|r| rules::reserved(&self.brain, &self.cells, r)) {
            self.monitor.reserved.insert(id, (channel, segment, epoch));
        }
    }

    /// The witnesses an operation decides from the state it finds (before it runs).
    fn witness_before(&mut self, op: &Op, held: Option<(usize, usize)>) {
        match op {
            Op::Section(Section::Reserve { .. }) => {
                if held.is_some_and(|(stream, _)| matches!(self.cells[stream].phase, Phase::Stopping(_))) {
                    self.monitor.witnesses.insert("W4a");
                }
            }
            Op::Section(Section::PlanCancel { stream, .. }) if self.cells[*stream].pinned() => {
                self.monitor.witnesses.insert("W3c");
            }
            Op::Section(Section::Decide(Request::Restore { channel })) => {
                let owed = self.cells.iter().any(|cell| cell.owed_on(*channel) || cell.in_flight.iter().any(|f| f.left && f.entry.channel == *channel));
                if owed {
                    self.monitor.witnesses.insert("W9c");
                }
            }
            Op::Section(Section::Late { stream, .. }) => {
                let lost = self.brain.results.iter().any(|(_, terminal)| *terminal == Terminal::Call(Outcome::Lost));
                if lost && self.cells[*stream].cut.is_none() && matches!(self.cells[*stream].phase, Phase::Armed(_) | Phase::Streaming(_)) {
                    self.monitor.witnesses.insert("W7b");
                }
            }
            _ => {}
        }
    }

    /// A request that waited in the brain's deferred list (its kind is marked) and whose command is routed
    /// now was issued after its wait (W5b, W9b). The command goes out in a later operation than the one
    /// that took the request off the list, so the list itself cannot be compared across one operation.
    fn witness_issued(&mut self, commands: &[Command]) {
        for request in &self.brain.deferred {
            self.monitor.witnesses.insert(match request {
                Request::Restore { .. } => "restore waited",
                Request::StartNext { .. } => "start waited",
                Request::Move { .. } => "move waited",
                _ => continue,
            });
        }
        for command in commands {
            let issued = match command {
                Command::Settings(SetCmd { change: Change::Restore { .. }, .. }) => ("restore waited", "W5b"),
                Command::Rx { op: RxOp::Start { .. }, .. } => ("start waited", "W5b"),
                Command::Settings(SetCmd { change: Change::Move { .. }, .. }) => ("move waited", "W9b"),
                _ => continue,
            };
            if self.monitor.witnesses.contains(issued.0) {
                self.monitor.witnesses.insert(issued.1);
            }
        }
    }

    fn route(&mut self, command: Command) {
        match command {
            Command::Settings(cmd) => {
                self.monitor.commanded.insert(cmd.id);
                self.settings.fifo.push_back(cmd);
            }
            Command::Rx { stream, op } => {
                let debt = has_debt(&self.cells[stream]);
                match op {
                    RxOp::Start { .. } if debt => {
                        self.monitor.witnesses.insert("W7c1");
                    }
                    RxOp::Flush if debt && !self.cells[stream].pinned() => {
                        self.monitor.witnesses.insert("W7b1");
                    }
                    _ => {}
                }
                self.rx[stream].fifo.push_back(op);
            }
            Command::Timer { stream, at } => self.timers.push((stream, at)),
            Command::SettingsTerminate => self.settings.terminate = true,
        }
    }

    /// `S.take`: the head command is taken. A cancelled value is dropped here, and its completion
    /// is sent here (E1-42).
    fn s_take(&mut self) {
        if self.settings.fifo.is_empty() {
            // The terminate follows every queued command; the program and the hand are empty (STake).
            self.settings.exited = true;
            return;
        }
        let cmd = self.settings.fifo.pop_front().expect("a command is queued");
        let take = rules::take_command(&mut self.cells, &cmd);
        match take {
            Take::Call => {
                self.settings.program = VecDeque::from([SStep::Check, SStep::Lock, SStep::Effect]);
                self.settings.hand = Some(cmd);
            }
            // Absent: a loss disposed and retired the value before its command was taken, so the value is gone
            // from every cell. The command is dropped all the same; the brain ignores its Done (the disposition stands).
            Take::Dropped | Take::Absent => {
                if take == Take::Dropped {
                    self.monitor.witnesses.insert("W1b");
                }
                self.monitor.terminals.insert(cmd.id, Terminal::Dropped);
                self.terminal_log.push((cmd.id, Terminal::Dropped));
                self.inbox.push_back(Input::Done { id: cmd.id, terminal: Terminal::Dropped });
            }
        }
    }

    /// `S.check`: a set flag abandons the command in hand and the FIFO, one cell section each, and
    /// the executor exits without a send (I1). Otherwise the check passes.
    fn s_check(&mut self) {
        self.monitor.checked.insert(Who::Settings, self.failure);
        let hand = self.settings.hand.as_ref().map(|cmd| cmd.id);
        let mut queued: VecDeque<u64> = self.settings.fifo.iter().map(|cmd| cmd.id).collect();
        let cells = &mut self.cells;
        let monitor = &mut self.monitor;
        let inbox = &mut self.inbox;
        let log = &mut self.terminal_log;
        let check = rules::settings_check(self.failure, hand, &mut queued, |id| abandon_command(cells, monitor, log, inbox, id));
        if check == Check::Exit {
            self.settings.exited = true;
            self.settings.fifo.clear();
            self.settings.hand = None;
            self.settings.program.clear();
        } else {
            self.settings.program.pop_front();
        }
    }

    /// `S` after its exit: each command routed to its FIFO is abandoned as the loop check does (I1), so
    /// a command routed after the exit is not stranded.
    fn s_discard(&mut self) {
        let cmd = self.settings.fifo.pop_front().expect("a command is queued");
        let mut queued = VecDeque::from([cmd.id]);
        let cells = &mut self.cells;
        let monitor = &mut self.monitor;
        let inbox = &mut self.inbox;
        let log = &mut self.terminal_log;
        rules::settings_check(true, None, &mut queued, |id| abandon_command(cells, monitor, log, inbox, id));
    }

    /// `S.effect(outcome)`: the device call of a timed command, or of a restore or move (no device
    /// queue). An accepted command is recorded with its acceptance instant. The steps after it
    /// release `L`, publish a loss if the device was lost, write the result and send it.
    fn s_effect(&mut self, outcome: Outcome) {
        if outcome != Outcome::Lost {
            self.monitor.checked.remove(&Who::Settings);
        }
        let move_pending = self.brain.deferred.iter().any(|request| matches!(request, Request::Move { .. }));
        match outcome {
            Outcome::Ok if move_pending => self.monitor.witnesses.insert("W9a.ok"),
            Outcome::Partial if move_pending => self.monitor.witnesses.insert("W9a.partial"),
            Outcome::Refused if move_pending => self.monitor.witnesses.insert("W9a.refused"),
            _ => false,
        };
        self.settings.program.pop_front();
        let cmd = self.settings.hand.clone().expect("a command in hand");
        if let Some((channel, freq, at)) = rules::settings_call(&cmd.change) {
            self.calls.push(DevCall::Apply { channel, freq, at });
        }
        if let (Change::Timed { channel, at, .. }, Outcome::Ok | Outcome::Partial) = (&cmd.change, outcome) {
            if let Apply::Accepted { effective } = self.device.apply_timed(*channel, cmd.id, *at) {
                rules::accept_all(&mut self.cells, cmd.id, effective);
            }
        }
        self.settings.terminal = Some(Terminal::Call(outcome));
        let mut program = vec![SStep::Unlock];
        if outcome == Outcome::Lost {
            program.extend(PUB_STEPS.map(SStep::Pub));
        }
        program.extend([SStep::Result, SStep::Send]);
        self.settings.program = program.into();
    }

    /// `S.result`: the cell section that writes the command's terminal, before its send.
    fn s_result(&mut self) {
        self.settings.program.pop_front();
        let hand = self.settings.hand.clone().expect("a command in hand");
        let terminal = self.settings.terminal.expect("a decided terminal");
        rules::settle_call(&mut self.cells, &hand, terminal);
        self.monitor.terminals.insert(hand.id, terminal);
        self.terminal_log.push((hand.id, terminal));
    }

    /// `R.check` at a loop top: the flag discards the FIFO and exits. The exit first sends the loss
    /// boundary of the stream (the receive executor's part of the loss publication, whichever
    /// executor saw the loss); a command, or else the next recv cycle, is queued as the program.
    fn r_check(&mut self, k: usize) {
        let failure = self.failure;
        self.monitor.checked.insert(Who::Rx(k), failure);
        let receiving = self.receiving(k);
        let flush_over = self.flush_over(k);
        let at_end = self.at_end();
        let r = &mut self.rx[k];
        match rules::next_action(failure, !r.fifo.is_empty(), flush_over, receiving, at_end) {
            Next::Exit => {
                assert_eq!(rules::loop_check(failure, &mut r.fifo), Check::Exit);
                // The send is queued as the last step: `RCheck` is off once exited, `RSend` still runs.
                r.program.push_back(RStep::Send(Input::Boundary(Boundary::Lost { stream: k })));
                r.exited = true;
            }
            Next::Cmd => r.program.push_back(RStep::Cmd),
            Next::FlushStop => r.program = call_steps(RxCall::FlushStop, false).into(),
            Next::Recv => r.program = VecDeque::from([RStep::Snap, RStep::Begin, RStep::End, RStep::Publish]),
            Next::Idle => {}
        }
    }

    /// `R.cmd`: the head command becomes its steps. A lifecycle call holds `G` around its `L` (note
    /// 45 §4.5 procedure 4); a start, stop or flush holds `L` only.
    fn r_cmd(&mut self, k: usize) {
        let op = self.rx[k].fifo.pop_front().expect("a command is queued");
        if op == RxOp::Terminate {
            self.rx[k].exited = true;
            self.rx[k].fifo.clear();
            self.rx[k].program.clear();
            return;
        }
        let (call, lifecycle, quiesce) = rules::call_of(op).expect("a terminate is handled above");
        let mut steps = call_steps(call, lifecycle);
        if quiesce {
            steps.push(RStep::Send(Input::Boundary(Boundary::Quiesced { stream: k })));
        }
        self.rx[k].program = steps.into();
    }

    /// `R` begins a recv. At the epilogue's end no recv begins: the cycle's remaining steps are dropped,
    /// so the loop top takes over and a queued command (a shutdown's terminate) is still taken.
    fn r_begin(&mut self, k: usize) {
        if self.at_end() {
            self.rx[k].program.clear();
            self.rx[k].snap = None;
            return;
        }
        self.rx[k].program.pop_front();
        let snap = self.rx[k].snap.expect("a snapshot");
        let handle = self.rx[k].handle.expect("an open handle");
        let deadline = self.device.t + self.params.recv_timeout;
        self.calls.push(DevCall::Recv { stream: k });
        self.device.enter_recv(k);
        self.rx[k].recv = Some(RecvCall { recv: Recv { epoch: snap.epoch, handle, samples: Vec::new() }, deadline });
    }

    /// `R.recv_end`: the buffered samples up to a packet. A lost device ends the recv with the loss
    /// publication instead of a publish.
    fn r_end(&mut self, k: usize) {
        self.rx[k].program.pop_front();
        if self.rx[k].recv.as_ref().is_some_and(|call| call.recv.epoch != self.cells[k].epoch) {
            self.monitor.witnesses.insert("W3a");
        }
        if !self.device.lost {
            if let Some(deadline) = self.rx[k].recv.as_ref().map(|call| call.deadline) {
                if !self.packet_ready(k) && self.device.t >= deadline {
                    self.monitor.observed.insert((k, deadline, self.device.t));
                }
            }
        }
        let received = if self.device.lost {
            Received::Lost
        } else if self.device.take_late(k) {
            Received::Late
        } else {
            Received::Samples(self.device.recv(k, self.params.packet, self.rx[k].recv.as_ref().map_or(i64::MAX, |call| call.deadline)))
        };
        let ended = rules::end_of_recv(received, k, self.device.t);
        if ended.lost {
            let r = &mut self.rx[k];
            r.recv = None;
            r.snap = None;
            r.program = PUB_STEPS.iter().map(|step| RStep::Pub(*step)).collect();
            return;
        }
        if let Some(call) = self.rx[k].recv.as_mut() {
            call.recv.samples = ended.samples;
        }
        if let Some(input) = ended.late_send {
            self.rx[k].program.push_back(RStep::Send(input));
        }
    }

    /// `R.publish`: a stale snapshot recomputes from a fresh one, keeping the recv. A published recv
    /// records its consumption (published and discarded samples alike) and the boundary inputs.
    fn r_publish(&mut self, k: usize) {
        self.rx[k].program.pop_front();
        let snap = self.rx[k].snap.take().expect("a snapshot");
        let call = self.rx[k].recv.take().expect("a recv");
        let flushing = self.cells[k].phase == Phase::Flushing;
        let region = (self.cells[k].origin_of(call.recv.epoch), self.cells[k].bound(call.recv.epoch));
        let carried = self.carried(k);
        match rules::publish_round(&mut self.cells[k], k, snap, &call.recv, carried, &self.rx[k].consumed) {
            Round::Stale => {
                if snap.epoch == self.cells[k].epoch && snap.cut != self.cells[k].cut {
                    self.monitor.witnesses.insert("W3b");
                }
                let r = &mut self.rx[k];
                r.recv = Some(call);
                r.program.push_front(RStep::Publish);
                r.program.push_front(RStep::Snap);
            }
            Round::Published { sends, consumed, published, discarded } => {
                let epoch = call.recv.epoch;
                if !flushing {
                    self.monitor.published.insert(epoch, self.cells[k].progress);
                }
                self.published.push(Published { stream: k, epoch, ticks: published, discarded });
                for (id, sample) in consumed {
                    if !flushing && region.0 <= sample && region.1.is_none_or(|bound| sample < bound) {
                        self.monitor.witnesses.insert("W1a");
                    }
                    if self.monitor.shortened.get(&id).is_some_and(|cut| sample >= *cut) {
                        self.monitor.witnesses.insert("W1c");
                    }
                    // The value was consumed over the channels the receiving streamer carries (`carried`), so its channel is one the
                    // device delivered, not the cell's claim.
                    let channel = self.cells[k].in_flight.iter().find(|f| f.entry.order.1 == id).map_or(usize::MAX, |f| f.entry.channel);
                    let ident = self.ident(id, channel, sample);
                    let left = self.cells[k].is_left(id);
                    if left && self.brain.deferred.contains(&Request::Restore { channel }) {
                        self.monitor.witnesses.insert("W5d");
                    }
                    self.fact_log.push(Fact { leftover: left, value: id, stream: k, channel, sample });
                    let fact = if left { &mut self.monitor.leftovers } else { &mut self.monitor.evidence };
                    fact.entry(id).or_insert(ident);
                    self.rx[k].consumed.insert(id);
                }
                self.rx[k].program.extend(sends.into_iter().map(RStep::Send));
            }
        }
    }

    /// `R.call`; `fault` is the `RFault` branch: the device is not touched and the call fails.
    fn r_call(&mut self, k: usize, fault: bool) {
        let Some(RStep::Call(call)) = self.rx[k].program.pop_front() else { unreachable!("allowed only at a call") };
        self.monitor.checked.remove(&Who::Rx(k));
        let result = if self.device.lost {
            CallResult::Lost
        } else if fault {
            CallResult::Failed
        } else {
            self.device_call(k, call)
        };
        self.rx[k].call = Some(result);
        if result == CallResult::Lost {
            // The loss publication replaces the rest of the program. A lifecycle caller holds `G`
            // already, so it sets the flag inside it (no `Gate`).
            let lifecycle = matches!(call, RxCall::Open | RxCall::Close);
            if lifecycle {
                self.monitor.witnesses.insert("W8d");
            }
            let mut steps = vec![RStep::Unlock, RStep::Result(call)];
            steps.extend(PUB_STEPS.iter().filter(|step| !(lifecycle && **step == PubStep::Gate)).map(|step| RStep::Pub(*step)));
            self.rx[k].program = steps.into();
        }
    }

    /// The device effect of a call on stream `k`, with the device not lost. A call on a closed
    /// handle fails without touching the device.
    fn device_call(&mut self, k: usize, call: RxCall) -> CallResult {
        match rules::plan_call(call, self.rx[k].handle.is_some()) {
            CallPlan::Skip(result) => result,
            CallPlan::Do(op) => {
                match op {
                    DeviceOp::Start { at } => {
                        self.calls.push(DevCall::Start { stream: k, at });
                        self.device.start(k, at)
                    }
                    DeviceOp::StartNow => {
                        self.calls.push(DevCall::StartNow { stream: k });
                        self.device.start_now(k)
                    }
                    DeviceOp::Stop => {
                        self.calls.push(DevCall::Stop { stream: k });
                        self.device.stop(k)
                    }
                    DeviceOp::Open => {
                        let channels: Vec<usize> = self.cells[k].channels.iter().map(|channel| channel as usize).collect();
                        self.calls.push(DevCall::Open { stream: k, channels: channels.clone() });
                        self.device.open(k, channels);
                    }
                    DeviceOp::Close => {
                        self.calls.push(DevCall::Close { stream: k });
                        self.device.close(k)
                    }
                }
                CallResult::Ok
            }
        }
    }

    /// `R.result`: the cell section (or the handle change) that writes the call's effect, then the
    /// boundary input a successful start or stop sends.
    fn r_result(&mut self, k: usize) {
        let Some(RStep::Result(call)) = self.rx[k].program.pop_front() else { unreachable!("allowed only at a result") };
        let result = self.rx[k].call.take().expect("a call's result");
        match (call, result) {
            (RxCall::Begin { .. }, CallResult::Failed) => {
                self.monitor.witnesses.insert("W7c2");
            }
            (RxCall::FlushStart, CallResult::Ok) if !self.cells[k].pinned() && has_debt(&self.cells[k]) => {
                self.monitor.witnesses.insert("W7b1");
            }
            (RxCall::FlushStop, CallResult::Failed) => {
                self.monitor.witnesses.insert("W7b2");
            }
            (RxCall::Open | RxCall::Close, CallResult::Ok) => {
                self.monitor.witnesses.insert("W8c");
            }
            _ => {}
        }
        let now = self.device.t;
        let after = rules::after_call(&mut self.cells[k], k, call, result, now, self.params.drain_window);
        let r = &mut self.rx[k];
        match after.handle {
            Handle::Opened => {
                r.handles += 1;
                r.handle = Some(r.handles);
            }
            Handle::Closed => r.handle = None,
            Handle::Keep => {}
        }
        match after.flush_end {
            FlushEnd::Set(end) => r.flush_end = Some(end),
            FlushEnd::Clear => r.flush_end = None,
            FlushEnd::Keep => {}
        }
        if let Some(input) = after.send {
            r.program.push_front(RStep::Send(input));
        }
    }

    /// `R.gate`: the lifecycle gate is taken, and the flag is rechecked inside it. A set flag
    /// releases it and drops the call.
    fn r_gate(&mut self, k: usize) {
        self.rx[k].program.pop_front();
        self.gate = Some(Holder::Rx(k));
        if self.failure {
            self.monitor.witnesses.insert("W8b");
            self.gate = None;
            self.rx[k].program.clear();
        }
    }

    /// The fact identity of `value` at `sample` on `channel`, the channel the device was observed to act on; the segment and
    /// epoch are the reservation's (I4 compares the fact with the reservation).
    fn ident(&self, value: u64, channel: usize, sample: i64) -> Ident {
        let &(_, segment, epoch) = self.monitor.reserved.get(&value).expect("a value is reserved before it is applied or consumed");
        Ident { value, channel, segment, epoch, sample }
    }

    /// The channels stream `k`'s streamer carries on the device.
    fn carried(&self, k: usize) -> Channels {
        self.device.streamers.get(&k).map_or(Channels::first(0), |streamer| streamer.channels.iter().fold(Channels::first(0), |channels, &channel| channels.with(channel as u32)))
    }

    /// Copies what the brain and the device wrote into the monitor's record.
    pub(super) fn sync(&mut self) {
        for (id, disposition) in &self.brain.dispositions {
            self.monitor.dispositions.entry(*id).or_insert(*disposition);
        }
        // The application's channel is the DSP chain whose log holds the write (the device's fact).
        for (&channel, chain) in &self.device.dsp {
            for &(sample, id) in &chain.log {
                if id != INITIAL_OCCUPANT && !self.monitor.applied.contains_key(&id) {
                    let ident = self.ident(id, channel, sample);
                    self.monitor.applied.insert(id, ident);
                }
            }
        }
        for (k, cell) in self.cells.iter().enumerate() {
            if cell.drain == Drain::Ended {
                self.monitor.drain_ended.insert(k);
            }
        }
    }
}

/// The steps of one device call: the native lock around it, and the gate around a lifecycle call.
fn call_steps(call: RxCall, lifecycle: bool) -> Vec<RStep> {
    let mut steps = Vec::new();
    if lifecycle {
        steps.push(RStep::Gate);
    }
    steps.extend([RStep::Lock, RStep::Call(call), RStep::Unlock, RStep::Result(call)]);
    if lifecycle {
        steps.push(RStep::Ungate);
    }
    steps
}

impl Model for World {
    type Event = Event;
    type Key = String;

    /// The state in canonical form (45-experiment §3.3): every field a later step or verdict reads, with
    /// the history projected to what decides the future. Two states with equal keys have the same enabled
    /// events and the same verdicts. The trace and the diagnostic counters of the cells stay out. The
    /// destructurings have no `..`, so a new field fails to compile until the key decides it; a field
    /// the key drops is named `: _` with its reason.
    fn key(&self) -> String {
        let World { params, brain, inbox, cells, settings, rx, clock, native, gate, failure, device, env, timers, trace: _, published: _, terminal_log: _, fact_log: _, calls: _, monitor, steps } =
            self;
        let cells: Vec<_> = cells
            .iter()
            .map(|cell| {
                let Cell { phase, drain, outstanding, channels, generation, epoch, origin, cut, retiring, progress, in_flight, flushed, revision, recomputes: _, discarded: _ } = cell;
                ((phase, drain, outstanding, channels, generation, epoch), (origin, cut, retiring, progress, in_flight, flushed, revision))
            })
            .collect();
        // A consumed id is kept only while its value is in the stream's cell: `consume` and `flush_pending`
        // read the cell's values alone, and a value is released once, so a retired id is never offered again.
        let rx: Vec<_> = rx
            .iter()
            .enumerate()
            .map(|(k, r)| {
                let RxExec { fifo, program, handle, handles, recv, snap, call, consumed, flush_end, exited } = r;
                let consumed: BTreeSet<u64> = consumed.iter().copied().filter(|id| self.cells[k].state_of(*id).is_some()).collect();
                (fifo, program, handle, handles, recv, snap, call, consumed, flush_end, exited)
            })
            .collect();
        let Device { t, queue, queue_cap, fifo_cap, last_command, dsp, streamers, lost, silent_from } = device;
        // The applied log is history: every write in it is in the monitor's `applied`, which `sync` copies, and
        // no behaviour reads it (I8 checks it at each state). A closed flow that ended by `t` passes no sample:
        // `drain` drops it at the next settle, and until then every head skips it, its tick being past the end.
        let dsp: Vec<_> = dsp
            .iter()
            .map(|(channel, chain)| {
                let Dsp { fifo, log: _, flows, cursor } = chain;
                let flows: Vec<_> = flows.iter().filter(|(_, end, _)| end.is_none_or(|end| end > *t)).copied().collect();
                (channel, fifo, flows, cursor)
            })
            .collect();
        let device = (t, queue, queue_cap, fifo_cap, last_command, dsp, streamers, lost, silent_from);
        // A value's disposition, result and leftover fact are read by id only, and the first write per id stands,
        // so the order of writes across ids is history. `guard` records the guard steps taken (a test reads it).
        let Brain { held, dispositions, results, leftovers, outbox, deferred, ops, next_id, drain_window, guard: _, exit, pending } = brain;
        let mut dispositions = dispositions.clone();
        dispositions.sort_by_key(|(id, _)| *id);
        let mut results = results.clone();
        results.sort_by_key(|(id, _)| *id);
        let mut leftovers = leftovers.clone();
        leftovers.sort_unstable();
        leftovers.dedup();
        let brain = (held, dispositions, results, leftovers, outbox, deferred, ops, next_id, drain_window, exit, pending);
        // Each field the key drops is read by no behaviour, oracle or later witness: `published`, `drain_ended` and `marked`
        // are monotone bits whose witness names are reported at the state that has them; `dispositions` copies the brain's.
        // A witness a later check reads stays: `shortened` (W1c) and the names in `READ_LATER`. The other witness names are
        // records only. The tick of a delivery is read by no oracle (P1 reads `drain_observed`), so `observed` keeps `(stream, at)`.
        let Monitor {
            applied,
            evidence,
            leftovers: leftover_facts,
            published: _,
            reserved,
            dispositions: _,
            terminals,
            observed,
            drain_ended: _,
            drain_observed,
            marked: _,
            released,
            commanded,
            shutdown,
            checked,
            panicked,
            panic_reported,
            witnesses,
            shortened,
        } = monitor;
        let read_later: BTreeSet<&str> = witnesses.iter().copied().filter(|name| READ_LATER.contains(name)).collect();
        let observed: BTreeSet<(usize, i64)> = observed.iter().map(|&(stream, at, _)| (stream, at)).collect();
        let monitor = ((applied, evidence, leftover_facts, reserved, terminals, observed, drain_observed), (released, commanded, shutdown, checked, panicked, panic_reported), (shortened, read_later));
        [
            format!("{params:?}"),
            format!("{brain:?}"),
            format!("{inbox:?}"),
            format!("{cells:?}"),
            format!("{settings:?}"),
            format!("{rx:?}"),
            format!("{clock:?}"),
            format!("{native:?}"),
            format!("{gate:?}"),
            format!("{failure:?}"),
            format!("{device:?}"),
            format!("{env:?}"),
            format!("{timers:?}"),
            format!("{monitor:?}"),
            format!("{steps:?}"),
        ]
        .join("|")
    }

    fn enabled(&self) -> Vec<Event> {
        self.candidates().into_iter().filter(|event| self.allowed(event)).collect()
    }

    /// Time (`Tick`), environment delivery (`Env`, `Timer`) and actor work (the brain, `S`, `R`, the clock, the guard).
    fn class(&self, event: &Event) -> usize {
        match event {
            Event::Tick => 0,
            Event::Env(_) | Event::Timer(_) => 1,
            _ => 2,
        }
    }

    fn apply(&mut self, event: &Event) {
        self.trace.push(event.clone());
        self.run(event);
        self.sync();
    }

    fn safety(&self) -> Result<(), String> {
        super::oracle::safety(self)
    }

    fn transition(&self, event: &Event, after: &World) -> Result<(), String> {
        super::oracle::transition(self, event, after)
    }

    fn progress(&self) -> Result<(), String> {
        super::oracle::progress(self)
    }

    /// The paths the monitor has seen. A case declares which of them it must reach (part 3).
    fn witnesses(&self) -> Vec<&'static str> {
        let m = &self.monitor;
        let bits = [
            (!m.applied.is_empty(), "value applied"),
            (!m.evidence.is_empty(), "value consumed"),
            (!m.leftovers.is_empty(), "leftover consumed"),
            (!m.published.is_empty(), "publication"),
            (!m.dispositions.is_empty(), "disposition written"),
            (!m.terminals.is_empty(), "command terminal"),
            (!m.observed.is_empty(), "deadline observed"),
            (!m.drain_ended.is_empty(), "drain ended"),
            (m.marked, "loss marked"),
        ];
        let mut names: Vec<&'static str> = bits.into_iter().filter(|(on, _)| *on).map(|(_, name)| name).collect();
        names.extend(m.witnesses.iter().copied());
        // Facts of the state itself, which no event records.
        let sample = |id: u64| m.evidence.get(&id).map(|fact| fact.sample);
        if let (Some(first), Some(second)) = (sample(1), sample(2)) {
            if first != second {
                names.push("W4b");
            }
        }
        if self.rx.iter().enumerate().any(|(k, r)| r.program.front() == Some(&RStep::Gate) && self.gate.is_some_and(|holder| holder != Holder::Rx(k))) {
            names.push("W8a");
        }
        names
    }

    /// `Some` at a terminal state that leaves an actor waiting: the epilogue ended before it
    /// finished, which is `Incomplete` and not a violation (45-experiment §3.2).
    fn incomplete(&self) -> Option<String> {
        let mut waiting = Vec::new();
        if !self.brain.dead() && (!self.inbox.is_empty() || !self.brain.ops.is_empty() || !self.brain.deferred.is_empty()) {
            waiting.push("B".to_string());
        }
        if self.settings.hand.is_some() || !self.settings.program.is_empty() {
            waiting.push("S".to_string());
        }
        for (k, r) in self.rx.iter().enumerate() {
            let running = !self.device.lost
                && !self.failure
                && self.cells[k].phase != Phase::Uncertain
                && self.device.streamers.get(&k).is_some_and(|streamer| streamer.next.is_some() && streamer.end.is_none());
            let outstanding = !self.device.lost && !self.failure && !self.brain.dead() && self.cells[k].phase != Phase::Lost && self.cells[k].outstanding.is_some();
            if r.recv.is_some() || !r.program.is_empty() || (!r.exited && (self.receiving(k) || self.flush_over(k))) {
                waiting.push(format!("R{k}"));
            } else if r.exited && running {
                // An exited executor must have stopped its device stream (the Stop precedes the Terminate).
                waiting.push(format!("R{k} (exited with its device stream running)"));
            } else if r.exited && outstanding {
                waiting.push(format!("R{k} (exited with a command outstanding on its cell)"));
            }
        }
        if self.clock.is_some() {
            waiting.push("clock".to_string());
        }
        if !self.timers.is_empty() {
            waiting.push("timer".to_string());
        }
        if self.env.iter().any(|slot| !slot.delivered) {
            waiting.push("environment".to_string());
        }
        if waiting.is_empty() { None } else { Some(format!("waiting at the end of the epilogue: {}", waiting.join(", "))) }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::super::cases::Probe;
    use super::super::cell::Retiring;
    use super::super::core::{DRAIN_WINDOW, Reason, Request};
    use super::super::dst::{self, Config};
    use super::super::fixtures::{cell, held, push, seg, value};
    use super::*;

    fn params() -> Params {
        Params { horizon: 8, epilogue: 8, recv_timeout: 8, queue_cap: 4, fifo_cap: 4, drain_window: DRAIN_WINDOW, packet: PACKET, call_faults: false, panic_step: None }
    }

    fn world(cells: Vec<Cell>) -> World {
        World::new(cells, Vec::new(), params())
    }

    fn ident() -> Ident {
        Ident { value: 5, channel: 0, segment: seg(0), epoch: 1, sample: 0 }
    }

    /// A streaming stream 0 on channel 0 with one queued value, a running device stream, and one
    /// monitor fact of each kind the key must see.
    fn base() -> World {
        let mut c = cell(Phase::Streaming(seg(0)), 0, Some(16), 0);
        push(&mut c, value(0, 10, 5), ValueState::Queued);
        let mut w = World::new(vec![c], vec![EnvSlot { input: EnvInput::Loss, earliest: 100, latest: 100, delivered: false, after: None }], params());
        w.device.open(0, vec![0]);
        w.device.start(0, 0);
        w.rx[0].handle = Some(1);
        w.monitor.applied.insert(5, ident());
        w
    }

    /// Takes the first enabled event that is not a tick, and a tick only when nothing else is
    /// enabled, until no event is enabled.
    fn settle(w: &mut World) {
        settle_with(w, |_| {});
    }

    /// `settle`, checking `check` after every event.
    fn settle_with(w: &mut World, mut check: impl FnMut(&World)) {
        for _ in 0..10_000 {
            let events = w.enabled();
            let Some(event) = events.iter().find(|e| **e != Event::Tick).or(events.first()).cloned() else { return };
            w.apply(&event);
            check(w);
        }
        panic!("the world did not settle within the budget");
    }

    /// An Ok move result moves the channel from the cell that owns it to the destination's set (one
    /// section of S.result); a partial result moves nothing. The reserved value stays on its chain.
    /// Section C (E1-57): a lifecycle call that finds the device lost already holds `G`, so its loss publication has no `Gate` step.
    #[test]
    fn harness_lifecycle_inline_loss_publish_no_recursion_direct() {
        let mut w = world(vec![cell(Phase::Off, 0, None, 0)]);
        w.device.lost = true;
        w.gate = Some(Holder::Rx(0));
        w.rx[0].program = vec![RStep::Call(RxCall::Close)].into();
        w.r_call(0, false);
        assert!(w.rx[0].program.contains(&RStep::Pub(PubStep::Set)));
        assert!(!w.rx[0].program.contains(&RStep::Pub(PubStep::Gate)));
    }

    #[test]
    fn harness_move_result_moves_channel_sets() {
        for (outcome, moved) in [(Outcome::Ok, true), (Outcome::Partial, false)] {
            let mut source = cell(Phase::Streaming(seg(0)), 0, None, 0);
            push(&mut source, value(0, 10, 5), ValueState::Done(Terminal::Call(Outcome::Ok)));
            let mut w = world(vec![source, cell(Phase::Off, 1, None, 0)]);
            w.settings.hand = Some(SetCmd { id: 9, change: Change::Move { channel: 0, to: 1 } });
            w.settings.terminal = Some(Terminal::Call(outcome));
            w.settings.program = VecDeque::from([SStep::Result]);
            w.apply(&Event::SResult);
            assert_eq!(!w.cells[0].channels.contains(0), moved, "{outcome:?}: the source loses the channel");
            assert_eq!(w.cells[1].channels.contains(0), moved, "{outcome:?}: the destination gains it");
            assert_eq!(w.cells[0].state_of(5), Some(&ValueState::Done(Terminal::Call(Outcome::Ok))), "{outcome:?}: the value stays on its chain");
            assert_eq!(w.monitor.terminals.get(&9), Some(&Terminal::Call(outcome)));
        }
    }

    /// A plan change then a shutdown, the value's `Done` at any time in [0, 6]: the search of the world with the
    /// focused terminal check below.
    fn plan_change_world() -> World {
        let params = Params { horizon: 6, epilogue: 6, ..params() };
        let mut c = cell(Phase::Streaming(seg(0)), 0, Some(16), 0);
        push(&mut c, RxEntry { accepted_at: Some(8), ..value(0, 8, 1) }, ValueState::Taken);
        let plan = Input::PlanCancel { stream: 0, origin: 20, epoch: 2, plan_cut: Some(48) };
        let done = Input::Done { id: 1, terminal: Terminal::Call(Outcome::Ok) };
        let env = vec![
            EnvSlot { input: EnvInput::Input(plan), earliest: 1, latest: 1, delivered: false, after: None },
            EnvSlot { input: EnvInput::Input(done), earliest: 0, latest: 6, delivered: false, after: None },
            EnvSlot { input: EnvInput::Input(Input::Shutdown), earliest: 5, latest: 12, delivered: false, after: None },
        ];
        let mut w = World::new(vec![c], env, params);
        w.device.open(0, vec![0]);
        w.device.start(0, 0);
        // The value's write is in the device (its call applied it; only the result is still to come).
        w.device.dsp.get_mut(&0).expect("the chain is open").fifo.push_back((8, 1));
        w.device.queue.push_back(8);
        w.device.last_command = 8;
        w.rx[0].handle = Some(1);
        w.monitor.reserved.insert(1, (0, seg(0), w.cells[0].epoch));
        w
    }

    /// Explores `w` with the probe's extra checks within `budget` states; a complete search passes, a budget overrun reports
    /// (`dst::assert_bounded`), and a violation, cycle, incomplete run or a declared witness a complete search missed fails.
    fn explore_probe(name: &'static str, probe: Probe, budget: usize, witnesses: &'static [&'static str]) -> dst::Verdict<Event> {
        let verdict = dst::explore(&Config { name, budget, wall: Duration::MAX, witnesses }, probe);
        println!("{}", verdict.report());
        dst::assert_bounded(&verdict);
        verdict
    }

    /// A plan change ends the old epoch with its PlanStop; the Finished boundary of that epoch then
    /// accounts its values. The accepted, unconsumed value gets its one disposition, Left, in every
    /// interleaving the search reaches (the plan change and the value's call done in either order).
    #[test]
    fn harness_plan_change_finishes_retiring_epoch() {
        let mut probe = Probe::new(plan_change_world());
        probe.end = |w| {
            let dispositions: Vec<_> = w.brain.dispositions.iter().filter(|(value, _)| *value == 1).map(|(_, d)| *d).collect();
            if dispositions == vec![Disposition::Reported(Reason::Left)] { Ok(()) } else { Err(format!("value 1 at the end: {dispositions:?}, {:?}", w.trace)) }
        };
        explore_probe("plan change finishes the retiring epoch", probe, 30_000, &[]);
    }

    #[test]
    fn harness_key_separates_each_behaviour_field() {
        type Edit = fn(&mut World);
        let edits: Vec<(&str, Edit)> = vec![
            ("brain.held", |w| w.brain.held.push(held(0, value(0, 3, 9), 1))),
            ("brain.dispositions", |w| w.brain.dispositions.push((9, Disposition::Cancelled))),
            ("brain.results", |w| w.brain.results.push((9, Terminal::Dropped))),
            ("brain.leftovers", |w| w.brain.leftovers.push(9)),
            ("brain.outbox", |w| w.brain.outbox.push(Command::Timer { stream: 0, at: 3 })),
            ("brain.deferred", |w| w.brain.deferred.push(Request::Flush { stream: 0 })),
            ("brain.ops", |w| w.brain.ops.push_back(Op::Guard(Guard::Gate))),
            ("brain.next_id", |w| w.brain.next_id += 1),
            ("brain.drain_window", |w| w.brain.drain_window += 1),
            ("brain.pending", |w| {
                w.brain.pending.insert(9, 0);
            }),
            ("inbox", |w| w.inbox.push_back(Input::Shutdown)),
            ("cell.phase", |w| w.cells[0].phase = Phase::Off),
            ("cell.drain", |w| w.cells[0].drain = Drain::Draining),
            ("cell.channels", |w| w.cells[0].channels = Channels::first(0).with(1)),
            ("cell.generation", |w| w.cells[0].generation += 1),
            ("cell.epoch", |w| w.cells[0].epoch += 1),
            ("cell.cut", |w| w.cells[0].cut = None),
            ("cell.retiring", |w| w.cells[0].retiring = Some(Retiring { epoch: 0, cut: None, origin: 0, published: false })),
            ("cell.progress", |w| w.cells[0].progress += 1),
            ("cell.in_flight state", |w| w.cells[0].in_flight[0].state = ValueState::Taken),
            ("cell.in_flight entry", |w| w.cells[0].in_flight[0].entry.accepted_at = Some(3)),
            ("cell.flushed", |w| w.cells[0].flushed = true),
            ("cell.revision", |w| w.cells[0].revision += 1),
            ("settings.fifo", |w| w.settings.fifo.push_back(SetCmd { id: 9, change: Change::Restore { channel: 0 } })),
            ("settings.hand", |w| w.settings.hand = Some(SetCmd { id: 9, change: Change::Restore { channel: 0 } })),
            ("settings.terminal", |w| w.settings.terminal = Some(Terminal::Dropped)),
            ("settings.program", |w| w.settings.program.push_back(SStep::Check)),
            ("settings.exited", |w| w.settings.exited = true),
            ("settings.terminate", |w| w.settings.terminate = true),
            ("monitor.panicked", |w| w.monitor.panicked = true),
            ("rx.fifo", |w| w.rx[0].fifo.push_back(RxOp::Flush)),
            ("rx.program", |w| w.rx[0].program.push_back(RStep::Cmd)),
            ("rx.handle", |w| w.rx[0].handle = None),
            ("rx.handles", |w| w.rx[0].handles += 1),
            ("rx.recv", |w| w.rx[0].recv = Some(RecvCall { recv: Recv { epoch: 1, handle: 1, samples: Vec::new() }, deadline: 4 })),
            ("rx.snap", |w| w.rx[0].snap = Some(Snapshot { epoch: 1, cut: None, revision: 0 })),
            ("rx.call", |w| w.rx[0].call = Some(CallResult::Failed)),
            ("rx.consumed", |w| {
                w.rx[0].consumed.insert(5);
            }),
            ("rx.exited", |w| w.rx[0].exited = true),
            ("clock", |w| w.clock = Some(ClockHold { slot: 0, now: None })),
            ("native", |w| w.native = Some(Holder::Brain)),
            ("gate", |w| w.gate = Some(Holder::Brain)),
            ("failure", |w| w.failure = true),
            ("device.t", |w| w.device.t += 1),
            ("device.queue", |w| w.device.queue.push_back(3)),
            ("device.last_command", |w| w.device.last_command += 1),
            ("device.lost", |w| w.device.lost = true),
            ("device.silent_from", |w| w.device.silent_from += 1),
            ("device.dsp fifo", |w| w.device.dsp.get_mut(&0).unwrap().fifo.push_back((4, 9))),
            ("device.dsp flows", |w| w.device.dsp.get_mut(&0).unwrap().flows.push((0, None, 0))),
            ("device.dsp cursor", |w| w.device.dsp.get_mut(&0).unwrap().cursor += 1),
            ("streamer.buffered", |w| w.device.streamers.get_mut(&0).unwrap().buffered.push_back(99)),
            ("streamer.next", |w| w.device.streamers.get_mut(&0).unwrap().next = None),
            ("streamer.end", |w| w.device.streamers.get_mut(&0).unwrap().end = Some(5)),
            ("streamer.then", |w| w.device.streamers.get_mut(&0).unwrap().then = Some(5)),
            ("streamer.late", |w| w.device.streamers.get_mut(&0).unwrap().late = Some(5)),
            ("streamer.channels", |w| w.device.streamers.get_mut(&0).unwrap().channels.push(2)),
            ("timers", |w| w.timers.push((0, 24))),
            ("env.delivered", |w| w.env[0].delivered = true),
            ("env.earliest", |w| w.env[0].earliest += 1),
            ("env.input", |w| w.env[0].input = EnvInput::Late { stream: 0 }),
            ("monitor.applied value", |w| {
                let mut entry = w.monitor.applied.remove(&5).unwrap();
                entry.value = 6;
                w.monitor.applied.insert(6, entry);
            }),
            ("monitor.applied channel", |w| w.monitor.applied.get_mut(&5).unwrap().channel += 1),
            ("monitor.applied segment", |w| w.monitor.applied.get_mut(&5).unwrap().segment = seg(9)),
            ("monitor.applied epoch", |w| w.monitor.applied.get_mut(&5).unwrap().epoch += 1),
            ("monitor.applied sample", |w| w.monitor.applied.get_mut(&5).unwrap().sample += 1),
            ("monitor.evidence", |w| {
                w.monitor.evidence.insert(7, ident());
            }),
            ("monitor.leftovers", |w| {
                w.monitor.leftovers.insert(7, ident());
            }),
            ("monitor.reserved", |w| {
                w.monitor.reserved.insert(5, (0, seg(0), 1));
            }),
            ("monitor.terminals", |w| {
                w.monitor.terminals.insert(5, Terminal::Dropped);
            }),
            ("monitor.observed", |w| {
                w.monitor.observed.insert((0, 24, 24));
            }),
            ("monitor.shortened", |w| {
                w.monitor.shortened.insert(5, 3);
            }),
            ("monitor.witness read later: W7b2", |w| {
                w.monitor.witnesses.insert("W7b2");
            }),
            ("monitor.witness read later: restore waited", |w| {
                w.monitor.witnesses.insert("restore waited");
            }),
            ("monitor.witness read later: start waited", |w| {
                w.monitor.witnesses.insert("start waited");
            }),
            ("monitor.witness read later: move waited", |w| {
                w.monitor.witnesses.insert("move waited");
            }),
            ("params.horizon", |w| w.params.horizon += 1),
            ("params.call_faults", |w| w.params.call_faults = true),
        ];
        let base_key = base().key();
        for (name, edit) in edits {
            let mut w = base();
            edit(&mut w);
            assert_ne!(w.key(), base_key, "the key does not see {name}");
        }
    }

    /// Two worlds that wrote the same two dispositions in opposite orders (`Done { Dropped }` delivered in
    /// either order) have equal keys: the order of writes across ids is history.
    #[test]
    fn harness_key_merges_equivalent_histories() {
        let run = |order: [u64; 2]| {
            let mut c = cell(Phase::Off, 0, None, 0);
            push(&mut c, value(0, 10, 1), ValueState::Cancelled);
            push(&mut c, value(0, 11, 2), ValueState::Cancelled);
            let mut w = world(vec![c]);
            w.inbox.extend(order.map(|id| Input::Done { id, terminal: Terminal::Dropped }));
            settle(&mut w);
            w
        };
        let (first, second) = (run([1, 2]), run([2, 1]));
        assert_ne!(first.brain.dispositions, second.brain.dispositions, "the writes are in opposite orders");
        assert_eq!(first.key(), second.key());
    }

    /// The fields the key drops are history: writing each one leaves the key unchanged (a witness name no later check
    /// reads included), and so do a flow
    /// closed at or before the current tick, the tick of a delivery, and a consumed id whose value is retired.
    #[test]
    fn harness_key_ignores_history() {
        type Edit = fn(&mut World);
        let edits: Vec<(&str, Edit)> = vec![
            ("device.dsp log", |w| w.device.dsp.get_mut(&0).unwrap().log.push((4, 9))),
            ("monitor.published", |w| {
                w.monitor.published.insert(1, 3);
            }),
            ("monitor.drain_ended", |w| {
                w.monitor.drain_ended.insert(0);
            }),
            ("monitor.marked", |w| w.monitor.marked = true),
            ("monitor.dispositions", |w| {
                w.monitor.dispositions.insert(5, Disposition::Consumed);
            }),
            ("monitor.witnesses", |w| {
                w.monitor.witnesses.insert("W1a");
            }),
            ("brain.guard", |w| w.brain.guard.push(Guard::Gate)),
            ("rx.consumed of a retired value", |w| {
                w.rx[0].consumed.insert(99);
            }),
        ];
        let base_key = base().key();
        for (name, edit) in edits {
            let mut w = base();
            edit(&mut w);
            assert_eq!(w.key(), base_key, "the key sees {name}, which is history");
        }
        let mut past = base();
        past.device.t = 5;
        let mut closed = past.clone();
        closed.device.dsp.get_mut(&0).unwrap().flows.push((0, Some(3), 0));
        assert_eq!(closed.key(), past.key(), "a flow closed before the tick is history");
        let (mut early, mut late) = (base(), base());
        early.monitor.observed.insert((0, 24, 24));
        late.monitor.observed.insert((0, 24, 30));
        assert_eq!(early.key(), late.key(), "the tick of a delivery is history");
    }

    #[test]
    fn harness_key_ignores_trace() {
        let mut w = base();
        w.trace.push(Event::Tick);
        w.trace.push(Event::BStep);
        assert_eq!(w.key(), base().key());
    }

    #[test]
    fn harness_key_ignores_diagnostic_counters() {
        let mut w = base();
        w.cells[0].recomputes += 3;
        w.cells[0].discarded += 7;
        assert_eq!(w.key(), base().key());
    }

    #[test]
    fn harness_per_actor_fifo_order() {
        let mut w = world(vec![cell(Phase::Off, 0, None, 0)]);
        let restore = |id| SetCmd { id, change: Change::Restore { channel: 0 } };
        w.settings.fifo = VecDeque::from([restore(1), restore(2)]);
        assert!(w.enabled().contains(&Event::STake));
        w.apply(&Event::STake);
        assert_eq!(w.settings.hand.as_ref().map(|cmd| cmd.id), Some(1));
        assert_eq!(w.settings.fifo.front().map(|cmd| cmd.id), Some(2));
        assert!(!w.enabled().contains(&Event::STake), "the second command waits for the first");
    }

    #[test]
    fn harness_first_tick_produces_sample_0() {
        let mut w = world(vec![cell(Phase::Streaming(seg(0)), 0, None, 0)]);
        w.device.open(0, vec![0]);
        w.device.start(0, 0);
        assert!(w.enabled().contains(&Event::Tick));
        w.apply(&Event::Tick);
        assert_eq!(w.device.streamers[&0].buffered, VecDeque::from([0]));
        assert_eq!(w.device.t, 1);
    }

    #[test]
    fn harness_failed_reservation_leaves_no_send() {
        let mut w = world(vec![cell(Phase::Stopping(seg(0)), 0, None, 0)]);
        w.inbox.push_back(Input::Release(held(0, value(0, 5, 7), 1)));
        settle_with(&mut w, |w| {
            assert!(w.settings.fifo.is_empty() && w.settings.hand.is_none(), "a refused reservation sends no settings command");
            assert!(w.rx.iter().all(|r| r.fifo.is_empty()), "a refused reservation sends no receive command");
        });
        assert_eq!(w.brain.held.len(), 1, "the refused value stays held");
    }

    /// A loss disposed and retired value 5 while its Timed command still sits in S's FIFO: the take finds
    /// no value, the command ends Dropped without a panic, and the brain keeps the loss as the disposition.
    #[test]
    fn harness_take_after_loss_retirement_drops_command() {
        let mut w = world(vec![cell(Phase::Off, 0, None, 0)]);
        w.brain.dispositions.push((5, Disposition::Reported(Reason::Loss)));
        w.settings.fifo.push_back(SetCmd { id: 5, change: Change::Timed { channel: 0, freq: 1.0, at: 10, generation: 1 } });
        assert!(w.enabled().contains(&Event::STake));
        w.apply(&Event::STake);
        assert_eq!(w.monitor.terminals.get(&5), Some(&Terminal::Dropped));
        assert_eq!(w.inbox, VecDeque::from([Input::Done { id: 5, terminal: Terminal::Dropped }]));
        for _ in 0..100 {
            let events = w.enabled();
            let Some(event) = events.iter().find(|e| matches!(e, Event::BStep | Event::BOp)).cloned() else { break };
            w.apply(&event);
        }
        assert_eq!(w.brain.dispositions, vec![(5, Disposition::Reported(Reason::Loss))], "{:?}", w.trace);
        assert_eq!(w.safety(), Ok(()), "{:?}", w.trace);
    }

    #[test]
    fn harness_take_cancelled_sends_done_dropped() {
        let mut c = cell(Phase::Off, 0, None, 0);
        push(&mut c, value(0, 10, 5), ValueState::Cancelled);
        let mut w = world(vec![c]);
        w.settings.fifo.push_back(SetCmd { id: 5, change: Change::Timed { channel: 0, freq: 1.0, at: 10, generation: 1 } });
        assert!(w.enabled().contains(&Event::STake));
        w.apply(&Event::STake);
        assert_eq!(w.inbox, VecDeque::from([Input::Done { id: 5, terminal: Terminal::Dropped }]));
    }

    #[test]
    fn harness_recv_timeout_loop_at_end_is_incomplete() {
        let mut w = world(vec![cell(Phase::Streaming(seg(0)), 0, None, 0)]);
        w.rx[0].handle = Some(1);
        settle(&mut w);
        assert_eq!(w.device.t, 16, "the run ends at horizon + epilogue");
        assert!(w.rx[0].recv.is_none(), "the last recv ended at the epilogue's end");
        assert!(w.rx[0].program.is_empty(), "R begins no recv cycle at the end");
        assert!(w.receiving(0), "the stream is still receiving at the end");
        let verdict = w.incomplete().expect("R is still receiving at the end");
        assert!(verdict.contains("R0"), "{verdict}");
    }

    /// W9b: a Move that waited in the deferred list and whose command goes out in a later operation is a witness.
    #[test]
    fn harness_witness_issued_after_wait_in_a_later_operation() {
        let mut w = world(vec![cell(Phase::Off, 0, None, 0)]);
        let go = Command::Settings(SetCmd { id: 1, change: Change::Move { channel: 0, to: 1 } });
        w.witness_issued(std::slice::from_ref(&go));
        assert!(!w.monitor.witnesses.contains("W9b"), "a move that never waited is not W9b");
        w.brain.deferred.push(Request::Move { channel: 0, to: 1 });
        w.witness_issued(&[]);
        w.brain.deferred.clear();
        w.witness_issued(std::slice::from_ref(&go));
        assert!(w.monitor.witnesses.contains("W9b"));
    }

    #[test]
    fn harness_timer_never_early() {
        let mut w = world(vec![cell(Phase::Off, 0, None, 0)]);
        w.timers.push((0, 5));
        w.device.t = 4;
        assert!(!w.enabled().contains(&Event::Timer(0)), "a deadline is not delivered before its instant");
        w.device.t = 5;
        assert!(w.enabled().contains(&Event::Timer(0)));
    }

    /// After the epilogue's end time goes on in silent ticks while a timer lies ahead, and the timer is
    /// delivered at its real instant, never at the end (Sol E1 finding 8).
    #[test]
    fn harness_timer_beyond_end_fires_at_its_instant() {
        let mut w = world(vec![cell(Phase::Off, 0, None, 0)]);
        w.timers.push((0, 20));
        w.device.t = 16;
        assert!(w.at_end() && !w.enabled().contains(&Event::Timer(0)), "a deadline beyond the end is not delivered at the end");
        assert!(w.enabled().contains(&Event::Tick), "time goes on while a timer lies ahead");
        while w.device.t < 20 {
            assert!(!w.enabled().contains(&Event::Timer(0)));
            w.apply(&Event::Tick);
        }
        assert!(w.enabled().contains(&Event::Timer(0)));
        assert!(!w.enabled().contains(&Event::Tick), "no timer or recv deadline lies ahead: time stops");
        w.apply(&Event::Timer(0));
        assert!(w.monitor.observed.contains(&(0, 20, 20)));
        assert!(w.monitor.drain_observed.contains(&(0, 20)));
    }

    /// A silent tick produces no sample and applies no write.
    #[test]
    fn harness_silent_tick_produces_and_applies_nothing() {
        let mut w = world(vec![cell(Phase::Streaming(seg(0)), 0, None, 0)]);
        w.device.open(0, vec![0]);
        w.device.start(0, 0);
        w.device.dsp.get_mut(&0).expect("the chain").fifo.push_back((17, 5));
        w.device.queue.push_back(18);
        w.device.t = 16;
        w.timers.push((0, 20));
        for _ in 0..4 {
            w.apply(&Event::Tick);
        }
        assert_eq!(w.device.t, 20);
        assert!(w.device.streamers[&0].buffered.is_empty(), "no sample after the end");
        w.device.settle();
        assert!(w.device.dsp[&0].log.is_empty() && w.device.dsp[&0].fifo.len() == 1, "no write applied after the end");
        assert!(w.device.queue.is_empty(), "the queue is time-driven: a due command leaves after the end");
    }

    /// A recv in progress at the epilogue's end ends at its own deadline, with silent time until then.
    #[test]
    fn harness_recv_at_epilogue_end_waits_for_its_deadline() {
        let mut w = world(vec![cell(Phase::Streaming(seg(0)), 0, None, 0)]);
        w.device.open(0, vec![0]);
        w.rx[0].handle = Some(1);
        w.rx[0].recv = Some(RecvCall { recv: Recv { epoch: 1, handle: 1, samples: Vec::new() }, deadline: 18 });
        w.rx[0].program = VecDeque::from([RStep::End]);
        w.device.t = 16;
        assert!(!w.enabled().contains(&Event::REnd(0)), "a recv does not end early at the epilogue's end");
        assert!(w.enabled().contains(&Event::Tick), "time goes on until the recv's deadline");
        w.apply(&Event::Tick);
        w.apply(&Event::Tick);
        assert!(w.enabled().contains(&Event::REnd(0)));
        w.apply(&Event::REnd(0));
        assert!(w.monitor.observed.contains(&(0, 18, 18)), "the timeout is observed at its deadline");
    }

    #[test]
    fn harness_recv_cycle_dropped_at_epilogue_end() {
        let mut w = world(vec![cell(Phase::Streaming(seg(0)), 0, None, 0)]);
        w.rx[0].handle = Some(1);
        w.rx[0].snap = Some(w.cells[0].snapshot());
        w.rx[0].program = VecDeque::from([RStep::Begin, RStep::End, RStep::Publish]);
        w.device.t = 15;
        w.apply(&Event::RBegin(0));
        assert!(w.rx[0].recv.is_some(), "a recv begins before the end");
        w.rx[0].recv = None;
        w.rx[0].program = VecDeque::from([RStep::Begin, RStep::End, RStep::Publish]);
        w.rx[0].snap = Some(w.cells[0].snapshot());
        w.device.t = 16;
        w.apply(&Event::RBegin(0));
        assert!(w.rx[0].recv.is_none() && w.rx[0].snap.is_none() && w.rx[0].program.is_empty(), "the cycle is dropped at the end");
        assert!(!w.enabled().contains(&Event::RCheck(0)), "the loop top starts no cycle at the end");
    }

    /// I1 at S's loop check: with the flag set, the command in hand is abandoned and S exits (E1-23).
    #[test]
    fn harness_settings_check_abandons_hand_under_failure() {
        let mut w = world(vec![cell(Phase::Off, 0, None, 0)]);
        w.route(Command::Settings(SetCmd { id: 7, change: Change::Restore { channel: 0 } }));
        w.apply(&Event::STake);
        assert_eq!(w.settings.hand.as_ref().map(|cmd| cmd.id), Some(7));
        w.failure = true;
        w.apply(&Event::SCheck);
        assert!(w.settings.exited && w.settings.hand.is_none(), "S exits at its check under the flag");
        assert_eq!(w.monitor.terminals.get(&7), Some(&Terminal::Abandoned));
    }

    /// A lifecycle gate taken with the flag set records W8b, releases the gate and drops the call (E1-47).
    #[test]
    fn harness_failed_lifecycle_gate_releases_and_drops_call() {
        let mut w = world(vec![cell(Phase::Off, 0, None, 0)]);
        w.rx[0].program = VecDeque::from([RStep::Gate, RStep::Lock]);
        w.failure = true;
        w.apply(&Event::RGate(0));
        assert!(w.monitor.witnesses.contains("W8b"));
        assert!(w.gate.is_none() && w.rx[0].program.is_empty(), "the gate is released and the call dropped");
    }

    #[test]
    fn harness_exited_settings_discards_routed_command() {
        let mut w = world(vec![cell(Phase::Off, 0, None, 0)]);
        w.failure = true;
        w.settings.exited = true;
        w.route(Command::Settings(SetCmd { id: 7, change: Change::Restore { channel: 0 } }));
        assert!(w.enabled().contains(&Event::SDiscard));
        w.apply(&Event::SDiscard);
        assert_eq!(w.monitor.terminals.get(&7), Some(&Terminal::Abandoned));
        assert_eq!(w.inbox, VecDeque::from([Input::Done { id: 7, terminal: Terminal::Abandoned }]));
        assert!(!w.enabled().contains(&Event::SDiscard), "the FIFO is empty");
    }

    #[test]
    fn harness_terminate_follows_queued_commands() {
        let mut w = world(vec![cell(Phase::Off, 0, None, 0)]);
        w.route(Command::Settings(SetCmd { id: 7, change: Change::Restore { channel: 0 } }));
        w.route(Command::SettingsTerminate);
        assert!(!w.settings.exited);
        settle(&mut w);
        assert!(w.settings.exited, "S exits after the command queued before the terminate");
        assert_eq!(w.monitor.terminals.get(&7), Some(&Terminal::Call(Outcome::Ok)));
        assert!(w.settings.fifo.is_empty() && w.settings.hand.is_none());
    }

    #[test]
    fn harness_deadline_beyond_last_input_is_observed() {
        let params = Params { horizon: 8, epilogue: 32, ..params() };
        let stop = EnvSlot { input: EnvInput::Input(Input::Stop { stream: 0, requested: 16 }), earliest: 8, latest: 8, delivered: false, after: None };
        let mut w = World::new(vec![cell(Phase::Streaming(seg(0)), 0, None, 0)], vec![stop], params);
        w.device.open(0, vec![0]);
        w.device.start(0, 0);
        w.rx[0].handle = Some(1);
        settle(&mut w);
        assert!(w.monitor.observed.contains(&(0, 24, 24)), "the drain deadline is delivered at 24");
        assert_eq!(w.incomplete(), None);
    }

    #[test]
    fn harness_shutdown_terminates_every_receive_executor() {
        let mut w = world(vec![cell(Phase::Off, 0, None, 0), cell(Phase::Off, 1, None, 0)]);
        w.inbox.push_back(Input::Shutdown);
        settle(&mut w);
        assert!(w.rx.iter().all(|r| r.exited), "shutdown ends every receive executor");
        assert!(w.settings.exited, "shutdown ends the settings executor");
    }

    /// The shape of case 7b: a stop leaves an accepted value to read, so its flush follows. The flush
    /// reads the value (or retires it at its stop), and the deferred restore then goes.
    #[test]
    fn harness_stop_debt_is_flushed_before_restore() {
        // The flush reads at device time 12, after the value's instant 8 (samples before it consume nothing).
        let mut w = crate::brain::fixtures::stop_debt_world(params());
        let flushing = std::cell::Cell::new(false);
        settle_with(&mut w, |w| flushing.set(flushing.get() || w.cells[0].phase == Phase::Flushing));
        assert!(flushing.get(), "the stop's debt is flushed: {:?}", w.trace);
        assert_eq!(w.cells[0].phase, Phase::Off, "the flush's stop succeeds");
        assert!(w.brain.leftovers.contains(&1), "the flush reads the value it left: {:?}", w.trace);
        assert!(w.cells[0].in_flight.is_empty(), "the flush retires the value it left: {:?}", w.cells[0].in_flight);
        assert!(w.brain.deferred.is_empty(), "the restore went after the flush");
        assert_eq!(w.brain.dispositions, vec![(1, Disposition::Reported(Reason::Left))]);
    }

    /// A flush ends at its drain window while a left value stays unread (its acceptance lies past the
    /// window): FlushStop comes within the window, not at the epilogue's end. The unread value stays left
    /// on the cell (the device FIFO still holds its write), the flush is not owed again, and a restore of
    /// its channel is dropped, as production skips it (Sol E1 finding 1).
    #[test]
    fn harness_flush_ends_at_drain_window_with_unread_left() {
        let mut entry = value(0, 8, 1);
        entry.accepted_at = Some(1000);
        let mut c = cell(Phase::Streaming(seg(0)), 0, None, 0);
        push(&mut c, entry, ValueState::Done(Terminal::Call(Outcome::Ok)));
        let mut params = params();
        params.epilogue = 64;
        let mut w = World::new(vec![c], Vec::new(), params);
        w.rx[0].handle = Some(1);
        w.device.open(0, vec![0]);
        w.monitor.reserved.insert(1, (0, seg(0), 1));
        w.rx[0].fifo.push_back(RxOp::Stop { at: None, epoch: 1 });
        w.brain.deferred.push(Request::Restore { channel: 0 });
        let (start, stop) = (std::cell::Cell::new(None), std::cell::Cell::new(None));
        settle_with(&mut w, |w| {
            assert!(w.monitor.commanded.is_empty(), "no restore goes while the value is left: {:?}", w.trace);
            if w.cells[0].phase == Phase::Flushing && start.get().is_none() {
                start.set(Some(w.device.t));
            }
            if start.get().is_some() && w.cells[0].phase == Phase::Off && stop.get().is_none() {
                stop.set(Some(w.device.t));
            }
        });
        let (start, stop) = (start.get().expect("the flush starts"), stop.get().expect("the flush stops"));
        assert!(start + DRAIN_WINDOW <= stop && stop <= start + DRAIN_WINDOW + params.recv_timeout, "the flush stops at its window: start {start}, stop {stop}");
        assert!(!w.brain.leftovers.contains(&1), "the unread value is not consumed");
        assert_eq!(w.cells[0].in_flight.len(), 1, "the unread value stays left on the cell");
        assert!(w.cells[0].is_left(1) && w.cells[0].flushed && !w.cells[0].flush_owed(), "the flush was attempted and is not owed again");
        assert!(w.brain.deferred.is_empty(), "the restore is dropped: production skips a channel with left on the chain");
        assert_eq!(w.safety(), Ok(()));
    }

    /// A failed stop of the flush leaves the value it did not read (its disposition stays Left) on the cell, and
    /// no second flush is owed; the cell is uncertain, the restore is dropped (I13), and shutdown ends
    /// both executors (case 7c).
    #[test]
    fn harness_flush_stop_fault_keeps_left_value_and_drops_restore() {
        let mut params = params();
        params.call_faults = true;
        let mut entry = value(0, 8, 1);
        entry.accepted_at = Some(8);
        let mut c = cell(Phase::Streaming(seg(0)), 0, None, 0);
        push(&mut c, entry, ValueState::Done(Terminal::Call(Outcome::Ok)));
        let mut w = World::new(vec![c], Vec::new(), params);
        w.rx[0].handle = Some(1);
        w.device.open(0, vec![0]);
        w.rx[0].fifo.push_back(RxOp::Stop { at: None, epoch: 1 });
        w.brain.deferred.push(Request::Restore { channel: 0 });
        for _ in 0..10_000 {
            let events = w.enabled();
            let fault_stop = events.contains(&Event::RFault(0)) && w.rx[0].program.front() == Some(&RStep::Call(RxCall::FlushStop));
            let next = if fault_stop { Some(Event::RFault(0)) } else { events.iter().find(|e| **e != Event::Tick).or(events.first()).cloned() };
            let Some(event) = next else { break };
            w.apply(&event);
        }
        assert_eq!(w.cells[0].phase, Phase::Uncertain, "a failed flush stop is uncertain");
        assert!(w.cells[0].is_left(1) && !w.cells[0].flush_owed(), "the failed stop leaves the unread value, and no second flush is owed: {:?}", w.cells[0].in_flight);
        assert_eq!(w.brain.dispositions, vec![(1, Disposition::Reported(Reason::Left))]);
        assert!(w.brain.deferred.is_empty(), "the restore is dropped, not sent");
        w.inbox.push_back(Input::Shutdown);
        settle(&mut w);
        assert!(w.settings.exited && w.rx[0].exited, "shutdown ends both executors: {:?}", w.trace);
    }

    /// Case 7c's shape: a failed flush start skips the stop and leaves the value on the cell, so the flush
    /// owes nothing more, and shutdown ends both executors.
    #[test]
    fn harness_flush_start_fault_keeps_left_value_and_ends() {
        let mut params = params();
        params.call_faults = true;
        let mut entry = value(0, 8, 1);
        entry.accepted_at = Some(8);
        let mut c = cell(Phase::Streaming(seg(0)), 0, None, 0);
        push(&mut c, entry, ValueState::Done(Terminal::Call(Outcome::Ok)));
        let mut w = World::new(vec![c], Vec::new(), params);
        w.rx[0].handle = Some(1);
        w.device.open(0, vec![0]);
        w.rx[0].fifo.push_back(RxOp::Stop { at: None, epoch: 1 });
        for _ in 0..10_000 {
            let events = w.enabled();
            let fault_start = events.contains(&Event::RFault(0)) && w.rx[0].program.front() == Some(&RStep::Call(RxCall::FlushStart));
            let next = if fault_start { Some(Event::RFault(0)) } else { events.iter().find(|e| **e != Event::Tick && **e != Event::RFault(0)).or(events.first()).cloned() };
            let Some(event) = next else { break };
            w.apply(&event);
        }
        assert!(w.monitor.witnesses.contains("W7b1"), "the flush starts with debt: {:?}", w.trace);
        assert_eq!(w.cells[0].phase, Phase::Off, "a failed start leaves the cell Off");
        assert!(w.cells[0].is_left(1) && !w.cells[0].flush_owed(), "the failed start leaves the unread value, and no second flush is owed: {:?}", w.cells[0].in_flight);
        assert_eq!(w.brain.dispositions, vec![(1, Disposition::Reported(Reason::Left))]);
        w.inbox.push_back(Input::Shutdown);
        settle(&mut w);
        assert!(w.settings.exited && w.rx[0].exited, "shutdown ends both executors: {:?}", w.trace);
    }

    #[test]
    fn harness_flush_stop_fault_reaches_uncertain() {
        let mut params = params();
        params.call_faults = true;
        let mut w = World::new(vec![cell(Phase::Off, 0, None, 0)], Vec::new(), params);
        w.rx[0].fifo.push_back(RxOp::Flush);
        for _ in 0..10_000 {
            let events = w.enabled();
            let fault_stop = events.contains(&Event::RFault(0)) && w.rx[0].program.front() == Some(&RStep::Call(RxCall::FlushStop));
            let next = if fault_stop { Some(Event::RFault(0)) } else { events.iter().find(|e| **e != Event::Tick).or(events.first()).cloned() };
            let Some(event) = next else { break };
            w.apply(&event);
        }
        assert_eq!(w.cells[0].phase, Phase::Uncertain, "a failed final stop of a flush is uncertain");
    }

    /// A failed stop (an `RFault`) is uncertain, and the segment still ends: each value of it gets
    /// one disposition.
    #[test]
    fn harness_failed_stop_ends_segment() {
        let mut params = params();
        params.call_faults = true;
        let mut c = cell(Phase::Streaming(seg(0)), 0, None, 0);
        push(&mut c, value(0, 8, 1), ValueState::Taken);
        push(&mut c, value(0, 12, 2), ValueState::Done(Terminal::Call(Outcome::Ok)));
        let mut w = World::new(vec![c], Vec::new(), params);
        w.rx[0].handle = Some(1);
        w.rx[0].fifo.push_back(RxOp::Stop { at: None, epoch: 1 });
        for _ in 0..10_000 {
            let events = w.enabled();
            let fault_stop = events.contains(&Event::RFault(0)) && w.rx[0].program.front() == Some(&RStep::Call(RxCall::Stop { epoch: 1 }));
            let next = if fault_stop { Some(Event::RFault(0)) } else { events.iter().find(|e| **e != Event::Tick).or(events.first()).cloned() };
            let Some(event) = next else { break };
            w.apply(&event);
        }
        assert_eq!(w.cells[0].phase, Phase::Uncertain, "a failed stop is uncertain");
        for id in [1, 2] {
            assert_eq!(w.brain.dispositions.iter().filter(|(value, _)| *value == id).count(), 1, "value {id} has one disposition");
        }
    }

    /// A loss the settings executor sees reaches an idle receive executor too: its check sends the
    /// loss boundary, and the reserved value it holds gets one disposition, lost.
    #[test]
    fn harness_loss_reaches_idle_receive_executor() {
        let mut c = cell(Phase::Streaming(seg(0)), 0, None, 0);
        push(&mut c, value(0, 8, 1), ValueState::Queued);
        let mut w = World::new(vec![c], Vec::new(), params());
        w.device.lost = true;
        w.settings.fifo.push_back(SetCmd { id: 9, change: Change::Restore { channel: 0 } });
        for _ in 0..10_000 {
            let events = w.enabled();
            let see_loss = events.contains(&Event::SEffect(Outcome::Lost));
            let next = if see_loss { Some(Event::SEffect(Outcome::Lost)) } else { events.iter().find(|e| **e != Event::Tick).or(events.first()).cloned() };
            let Some(event) = next else { break };
            w.apply(&event);
        }
        assert!(w.failure && w.rx[0].exited, "the loss publication sets the flag and the receive executor exits");
        assert_eq!(w.brain.dispositions, vec![(1, Disposition::Reported(Reason::Loss))]);
    }

    /// A leftover is offered from the cell's left flag alone (the brain is not consulted).
    #[test]
    fn u19_leftover_is_offered_from_the_cell_flag() {
        let mut c = cell(Phase::Off, 0, None, 0);
        push(&mut c, value(0, 8, 1), ValueState::Taken);
        c.mark_left(1);
        assert_eq!(rules::consume_samples(&c, Channels::first(1), &BTreeSet::new(), &[8]), vec![(1, 8)]);
    }

    /// A value the device applied at sample 15 (accepted at 15) is debt for the packet that carries
    /// that sample, even while its call result is still pending, as in the production debt rule. So
    /// the consumption is at 15, not at the next packet (I4 compares the sample; E1-96).
    #[test]
    fn u20_debt_counts_an_applied_value_before_its_call_finishes() {
        let mut c = cell(Phase::Streaming(seg(0)), 0, None, 0);
        push(&mut c, RxEntry { accepted_at: Some(15), ..value(0, 2, 5) }, ValueState::Taken);
        assert!(rules::consume_samples(&c, Channels::first(1), &BTreeSet::new(), &[12, 13, 14]).is_empty());
        assert_eq!(rules::consume_samples(&c, Channels::first(1), &BTreeSet::new(), &[12, 13, 14, 15]), vec![(5, 15)]);
    }

    /// `settle`, with the oracles' transition and safety checks at every step.
    fn settle_checked(w: &mut World) {
        settle_with_step(w, |before, event, after| {
            assert_eq!(before.transition(event, after), Ok(()), "step {event:?}: {:?}", after.trace);
            assert_eq!(after.safety(), Ok(()), "after {event:?}: {:?}", after.trace);
        });
    }

    fn settle_with_step(w: &mut World, mut check: impl FnMut(&World, &Event, &World)) {
        for _ in 0..10_000 {
            let events = w.enabled();
            let Some(event) = events.iter().find(|e| **e != Event::Tick).or(events.first()).cloned() else { return };
            let before = w.clone();
            w.apply(&event);
            check(&before, &event, w);
        }
        panic!("the world did not settle within the budget");
    }

    /// Sol E1 finding 2, in the world: a Close requested while the chain streams waits for the stop, and runs on the stopped
    /// chain (the oracle's I3 at routing and at execution passes at every step).
    #[test]
    fn harness_close_runs_after_the_stop() {
        let mut w = world(vec![cell(Phase::Streaming(seg(0)), 0, None, 0)]);
        w.device.open(0, vec![0]);
        w.device.start(0, 0);
        w.rx[0].handle = Some(1);
        w.inbox.push_back(Input::Request(Request::Close { stream: 0 }));
        w.inbox.push_back(Input::Release(held(0, value(0, 5, 7), 1)));
        w.inbox.push_back(Input::Stop { stream: 0, requested: 0 });
        settle_checked(&mut w);
        assert!(w.rx[0].handle.is_none(), "the Close ran: {:?}", w.trace);
        assert_eq!(w.cells[0].phase, Phase::Off, "the chain was stopped before the Close ran");
    }

    /// Sol E1 finding 3: after its guard the brain is dead. A shutdown and the executors' completions that arrive later
    /// are never served (no step, no disposition); the executors end by the failure flag and the closed queues.
    #[test]
    fn harness_dead_brain_serves_nothing_and_executors_end() {
        let params = Params { panic_step: Some(0), ..params() };
        let env = vec![EnvSlot { input: EnvInput::Input(Input::Shutdown), earliest: 0, latest: 20, delivered: false, after: None }];
        let mut c = cell(Phase::Streaming(seg(0)), 0, None, 0);
        push(&mut c, value(0, 5, 7), ValueState::Queued);
        let mut w = World::new(vec![c], env, params);
        w.rx[0].handle = Some(1);
        w.timers.push((0, 5));
        w.inbox.push_back(Input::Release(held(0, value(0, 9, 8), 1)));
        w.apply(&Event::BStep);
        while w.brain.ops.front().is_some() {
            assert!(!w.brain.dead());
            w.apply(&Event::BOp);
        }
        assert!(w.brain.dead() && w.failure, "the guard ran and the brain is dead");
        assert!(w.timers.is_empty(), "its bookings die with it");
        w.apply(&Event::Env(0));
        assert!(w.monitor.shutdown && w.inbox.len() == 1, "the shutdown reaches a dead brain's inbox");
        assert!(!w.enabled().contains(&Event::BStep) && !w.enabled().contains(&Event::BOp), "and is never served");
        settle(&mut w);
        assert!(w.brain.dispositions.is_empty(), "a dead brain writes no disposition: {:?}", w.brain.dispositions);
        assert!(w.settings.exited && w.rx[0].exited, "the executors end: {:?}", w.trace);
        assert_eq!(w.incomplete(), None);
        assert_eq!(w.progress(), Ok(()));
    }

    /// Sol E1 finding 5: a Move changes the cell's channel set at once but reaches the device only through the stream's next
    /// Open, which opens the streamer on the cell's channels.
    #[test]
    fn harness_move_reaches_device_only_through_open() {
        let mut w = world(vec![cell(Phase::Off, 0, None, 0), cell(Phase::Off, 1, None, 0)]);
        w.device.open(1, vec![1]);
        w.rx[1].handle = Some(1);
        w.settings.hand = Some(SetCmd { id: 9, change: Change::Move { channel: 0, to: 1 } });
        w.settings.terminal = Some(Terminal::Call(Outcome::Ok));
        w.settings.program = VecDeque::from([SStep::Result]);
        w.apply(&Event::SResult);
        assert!(w.cells[1].channels.contains(0));
        assert_eq!(w.device.streamers[&1].channels, vec![1], "the streamer still carries its own channels");
        assert_eq!(w.carried(1), Channels::first(0).with(1));
        w.rx[1].handle = None;
        w.device_call(1, RxCall::Open);
        assert_eq!(w.device.streamers[&1].channels, vec![0, 1], "the next Open uses the cell's channel set");
        assert_eq!(w.carried(1), Channels::first(0).with(0).with(1));
    }

    /// Consumption follows the channels the streamer carries (the device's), not the cell's: a left value on a channel the cell
    /// owns but the streamer does not carry is not consumed by a sample, and one on a carried channel is.
    #[test]
    fn harness_consumption_follows_the_streamers_channels() {
        for (carried, consumed) in [(vec![1], false), (vec![0, 1], true)] {
            let mut c = cell(Phase::Streaming(seg(0)), 0, None, 0);
            c.channels = c.channels.with(1);
            push(&mut c, RxEntry { accepted_at: Some(8), ..value(0, 8, 5) }, ValueState::Done(Terminal::Call(Outcome::Ok)));
            c.mark_left(5);
            let mut w = world(vec![c]);
            w.device.open(0, carried.clone());
            w.rx[0].handle = Some(1);
            w.monitor.reserved.insert(5, (0, seg(0), 1));
            w.rx[0].recv = Some(RecvCall { recv: Recv { epoch: 1, handle: 1, samples: vec![8] }, deadline: 100 });
            w.rx[0].snap = Some(w.cells[0].snapshot());
            w.rx[0].program = VecDeque::from([RStep::Publish]);
            w.apply(&Event::RPublish(0));
            assert_eq!(w.rx[0].consumed.contains(&5), consumed, "streamer channels {carried:?}");
            assert_eq!(w.monitor.leftovers.contains_key(&5), consumed);
        }
    }

    /// A missed start (a start below `now`, or one promoted behind a stopped tail after its instant) reaches the brain end to
    /// end: R's recv returns `LateCommand`, R sends it, the brain handles it and the executor then carries out its answer (a
    /// restart when no cut is in force, an end-now with one). `setup` makes the device miss a start of 6 at time 10.
    fn late_runs(phase: Phase, cut: Option<i64>, setup: impl Fn(&mut Device), seen: impl Fn(&World, &Event, &World) -> bool) {
        let mut w = world(vec![cell(phase, 0, cut, 4)]);
        w.device.open(0, vec![0]);
        setup(&mut w.device);
        w.rx[0].handle = Some(1);
        w.rx[0].snap = Some(w.cells[0].snapshot());
        w.rx[0].program = VecDeque::from([RStep::Begin, RStep::End, RStep::Publish]);
        let mut sent = false;
        let mut answered = false;
        settle_with_step(&mut w, |before, event, after| {
            sent |= after.inbox.iter().any(|input| matches!(input, Input::LateCommand { stream: 0, now: 10 }));
            answered |= seen(before, event, after);
        });
        assert!(sent, "R sent the late command");
        assert!(answered, "the brain's answer to the late command was carried out");
    }

    fn tail_then_start_at_6(dev: &mut Device) {
        dev.start(0, 0);
        for _ in 0..4 {
            dev.tick();
        }
        dev.stop(0);
        dev.start(0, 6);
        dev.streamers.get_mut(&0).expect("the stream").buffered.clear();
        dev.t = 10;
    }

    /// The answer's effect on the device: the restart's call has run and the streamer's next start is now.
    fn restarted(before: &World, event: &Event, after: &World) -> bool {
        matches!(event, Event::RCall(0)) && matches!(before.rx[0].program.front(), Some(RStep::Call(RxCall::Begin { at: 10 }))) && after.device.streamers[&0].next == Some(10)
    }

    /// The end-now stop's call has run and the streamer holds no start, tail or waiting start.
    fn ended(before: &World, event: &Event, after: &World) -> bool {
        let streamer = &after.device.streamers[&0];
        matches!(event, Event::RCall(0)) && matches!(before.rx[0].program.front(), Some(RStep::Call(RxCall::Stop { .. }))) && (streamer.next, streamer.end, streamer.then) == (None, None, None)
    }

    #[test]
    fn harness_missed_start_after_the_tail_reaches_the_brain() {
        late_runs(Phase::Armed(seg(0)), None, tail_then_start_at_6, restarted);
        late_runs(Phase::Streaming(seg(0)), Some(12), tail_then_start_at_6, ended);
    }

    /// The flush's start is a start-now: never a missed start, however late the executor runs it (Sol E1 re-review P1-B).
    #[test]
    fn harness_flush_start_is_never_late() {
        let mut w = world(vec![cell(Phase::Off, 0, None, 0)]);
        w.device.open(0, vec![0]);
        w.rx[0].handle = Some(1);
        w.device.t = 10;
        assert_eq!(w.device_call(0, RxCall::FlushStart), CallResult::Ok);
        let streamer = &w.device.streamers[&0];
        assert_eq!((streamer.late, streamer.next), (None, Some(10)));
    }

    /// The walk's scheduling classes: time, environment delivery, actor work.
    #[test]
    fn harness_event_classes_are_time_environment_actor() {
        let w = world(vec![cell(Phase::Off, 0, None, 0)]);
        assert_eq!([Event::Tick, Event::Env(0), Event::Timer(0), Event::BStep, Event::REnd(0), Event::ClockSend, Event::SEffect(Outcome::Ok)].map(|event| w.class(&event)), [0, 1, 1, 2, 2, 2, 2]);
    }

    /// A timed start below `now` on a stream with no tail reaches the brain the same way (Sol E1 re-review P1-B).
    #[test]
    fn harness_direct_past_start_reaches_the_brain() {
        let past = |dev: &mut Device| {
            dev.t = 10;
            dev.start(0, 6);
        };
        late_runs(Phase::Armed(seg(0)), None, past, restarted);
        late_runs(Phase::Streaming(seg(0)), Some(12), past, ended);
    }

    /// Case 3's shape without the Stop (45-experiment §5): stream 0 streams under epoch 1 with a taken value 1 that pins it
    /// and a recv in flight; a plan change (origin 20, epoch 2) comes at tick 1; the value's `Done` may come at any time in
    /// [0, 6]. The order of the pin's clearing and the quiesce ack is covered both ways by the search.
    #[test]
    fn harness_plan_change_quiesces_before_new_start() {
        let mut probe = Probe::new(plan_change_world());
        probe.state = |w| {
            let starts = w.rx[0].fifo.iter().filter(|op| matches!(op, RxOp::Start { .. })).count() + w.rx[0].program.iter().filter(|step| matches!(step, RStep::Call(RxCall::Begin { .. }))).count();
            if starts > 1 {
                return Err(format!("two starts in flight: {:?}", w.trace));
            }
            let cell = &w.cells[0];
            if (cell.retiring.is_some() || cell.pinned()) && starts > 0 {
                return Err(format!("a start of the new epoch before the quiesce ack and the pin clearing: {:?}", w.trace));
            }
            Ok(())
        };
        probe.end = |w| {
            // A start still waiting at the end must wait on a gate that is closed at that point.
            let cell = &w.cells[0];
            let gated = cell.retiring.is_some() || cell.pinned() || matches!(cell.phase, Phase::Armed(_) | Phase::Streaming(_) | Phase::Stopping(_));
            if w.brain.deferred.is_empty() || gated { Ok(()) } else { Err(format!("a start waits with every gate open: {:?}", w.trace)) }
        };
        probe.witness = |w| match w.cells[0].phase {
            Phase::Armed(s) | Phase::Streaming(s) if s == (SegmentKey { by: Some(2), origin: 20 }) => vec!["new start issued"],
            _ => Vec::new(),
        };
        let verdict = explore_probe("plan change quiesces before the new start", probe, 30_000, &["new start issued"]);
        assert!(verdict.reached.contains(&"new start issued"), "the new start is issued in some run: {}", verdict.report());
    }
}
