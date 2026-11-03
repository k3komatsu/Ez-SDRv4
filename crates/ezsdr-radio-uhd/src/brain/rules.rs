//! The executors' pure rules (note 45 §4.6): what a publish takes from a recv, the consumption
//! of debt and left values by a packet (through the production `RxChain::consume`), and the
//! loop checks that discard an executor's FIFO when the failure flag is set (I1).

use std::collections::{BTreeSet, VecDeque};

use ezsdr_radio::timeline::Channels;

use super::cell::{Call, Cell, Outcome, Phase, Publish, Recv, Snapshot, Take, Terminal, Ticks, ValueState};
use super::core::{Boundary, Brain, Change, Command, Input, Op, RxOp, Section, SetCmd};
use crate::device::{DeviceError, Iq, RxRecv};
use crate::provider::core::{RxChain, RxEntry, SegmentKey};

/// What a publish reaches: the progress after it and the samples it discards.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Plan {
    pub(crate) progress: i64,
    pub(crate) discard: u64,
}

/// The samples from `origin` up to `bound` are published, and the progress passes them; the rest are discarded.
/// The progress passes the published samples to the tick after the last one (the next sample's, one `step` on).
pub(crate) fn plan_publish(origin: i64, bound: Option<i64>, samples: &Ticks, progress: i64) -> Plan {
    let published = published_range(origin, bound, samples);
    let progress = if published.is_empty() { progress } else { progress.max(samples.at(published.end - 1) + samples.step) };
    Plan { progress, discard: (samples.count - published.len()) as u64 }
}

/// The indices of the samples a publish gives the user: those at or after `origin` and before `bound`.
pub(crate) fn published_range(origin: i64, bound: Option<i64>, samples: &Ticks) -> std::ops::Range<usize> {
    let from = samples.index_from(origin);
    from..bound.map_or(samples.count, |bound| samples.index_from(bound)).max(from)
}

/// The consumption evidence of one packet: `(value id, sample tick)` for each entry the production
/// rule removes from `left` or `debt`. The state is rebuilt in a scratch chain, so the rule is
/// `RxChain::consume` itself (no copy of it), and each sample is one call.
pub(crate) fn consume_packet(left: &[RxEntry], debt: &[RxEntry], sample_ticks: i64, channels: Channels, first_tick: i64, samples: &[Vec<Iq>]) -> Vec<(u64, i64)> {
    let segment = SegmentKey { by: None, origin: i64::MIN };
    let mut chain = RxChain::default();
    chain.adopt_start(segment, i64::MIN);
    for entry in left {
        hand(&mut chain, entry);
    }
    chain.stop_result(Ok(())).expect("the scratch chain stops");
    chain.finish();
    chain.adopt_start(segment, i64::MIN);
    for entry in debt {
        hand(&mut chain, entry);
    }
    let mut consumed = Vec::new();
    let count = samples.first().map_or(0, Vec::len);
    for index in 0..count {
        let tick = first_tick + index as i64 * sample_ticks;
        let before = orders(&chain);
        let one: Vec<Vec<Iq>> = samples.iter().map(|channel| vec![channel[index]]).collect();
        chain.consume(sample_ticks, channels, tick, &one);
        let after = orders(&chain);
        consumed.extend(before.into_iter().filter(|order| !after.contains(order)).map(|order| (order.1, tick)));
    }
    consumed
}

/// Puts `entry` into the scratch chain's debt, keeping its acceptance instant.
fn hand(chain: &mut RxChain, entry: &RxEntry) {
    chain.hand(entry.clone(), || Ok(()), || entry.accepted_at.unwrap_or(i64::MIN)).expect("the scratch chain applies");
}

fn orders(chain: &RxChain) -> Vec<(i64, u64)> {
    chain.left().iter().chain(chain.debt()).map(|entry| entry.order).collect()
}

/// The outcome of an executor's loop check.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Check {
    Proceed,
    Exit,
}

/// The loop check of a receive executor (I1): a set failure flag discards the FIFO and exits
/// before any call.
pub(crate) fn loop_check<T>(failure: bool, fifo: &mut VecDeque<T>) -> Check {
    if failure {
        fifo.clear();
        Check::Exit
    } else {
        Check::Proceed
    }
}

/// The loop check of the settings executor (I1): a set failure flag abandons the command in hand
/// and every queued one, each through `abandon` (one cell section per command), and exits.
pub(crate) fn settings_check(failure: bool, in_hand: Option<u64>, fifo: &mut VecDeque<u64>, mut abandon: impl FnMut(u64)) -> Check {
    if !failure {
        return Check::Proceed;
    }
    for id in in_hand.into_iter().chain(fifo.drain(..)) {
        abandon(id);
    }
    Check::Exit
}

/// A device call the receive executor makes (the cell's calls, and the lifecycle calls that have no cell state).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RxCall {
    /// `epoch` is the epoch the brain issued the start for (I17 matches the confirmation with it).
    Begin {
        at: i64,
        epoch: u64,
    },
    /// `epoch` is the segment the stop ends: its Finished boundary carries it.
    Stop {
        epoch: u64,
    },
    /// The stop a plan change makes of the old epoch's stream (under `L` only). `retiring` is that
    /// epoch: its Finished boundary carries it, as a stop's does.
    PlanStop {
        retiring: u64,
    },
    FlushStart,
    FlushStop,
    Open,
    Close,
}

/// What a device call returned: a failure that leaves the device, a failure of the device, or ok.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CallResult {
    Ok,
    Failed,
    Lost,
}

impl CallResult {
    pub(crate) fn result(self) -> Result<(), DeviceError> {
        match self {
            CallResult::Ok => Ok(()),
            CallResult::Failed => Err(DeviceError::failed("the call failed")),
            CallResult::Lost => Err(DeviceError { lost: true, message: "the device is lost".to_string() }),
        }
    }

    /// The result a device call's `Result` stands for (lost is the device's, any other error a failed call).
    pub(crate) fn of(result: &Result<(), DeviceError>) -> CallResult {
        match result {
            Ok(()) => CallResult::Ok,
            Err(error) if error.lost => CallResult::Lost,
            Err(_) => CallResult::Failed,
        }
    }
}

/// What a receive command runs: its call, whether it is a lifecycle call (held under `G`), and whether a quiesce
/// acknowledgement follows (a plan change). `Terminate` runs nothing.
pub(crate) fn call_of(op: RxOp) -> Option<(RxCall, bool, bool)> {
    match op {
        RxOp::Start { at, epoch } => Some((RxCall::Begin { at, epoch }, false, false)),
        RxOp::Stop { epoch, .. } => Some((RxCall::Stop { epoch }, false, false)),
        RxOp::Flush => Some((RxCall::FlushStart, false, false)),
        RxOp::Open => Some((RxCall::Open, true, false)),
        RxOp::Close => Some((RxCall::Close, true, false)),
        RxOp::Plan { retiring, .. } => Some((RxCall::PlanStop { retiring }, false, true)),
        RxOp::Terminate => None,
    }
}

/// The device operation a call makes on a handle that is open or closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeviceOp {
    Start { at: i64 },
    StartNow,
    Stop,
    Open,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CallPlan {
    Do(DeviceOp),
    /// The device is not touched: a call on a closed handle fails; a flush of a closed handle is ok.
    Skip(CallResult),
}

pub(crate) fn plan_call(call: RxCall, open: bool) -> CallPlan {
    match call {
        RxCall::Begin { at, .. } if open => CallPlan::Do(DeviceOp::Start { at }),
        RxCall::Stop { .. } | RxCall::PlanStop { .. } | RxCall::FlushStop if open => CallPlan::Do(DeviceOp::Stop),
        RxCall::FlushStart if open => CallPlan::Do(DeviceOp::StartNow),
        RxCall::FlushStart | RxCall::FlushStop => CallPlan::Skip(CallResult::Ok),
        RxCall::Open if !open => CallPlan::Do(DeviceOp::Open),
        RxCall::Close if open => CallPlan::Do(DeviceOp::Close),
        _ => CallPlan::Skip(CallResult::Failed),
    }
}

/// What a receive executor does at its loop top.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Next {
    Exit,
    Cmd,
    FlushStop,
    Recv,
    Idle,
}

pub(crate) fn next_action(failure: bool, fifo_nonempty: bool, flush_over: bool, receiving: bool, at_end: bool) -> Next {
    if failure {
        Next::Exit
    } else if fifo_nonempty {
        Next::Cmd
    } else if flush_over {
        Next::FlushStop
    } else if receiving && !at_end {
        Next::Recv
    } else {
        Next::Idle
    }
}

/// What a close returns: the trait's `rx_close` is infallible, so the executor reads the outcome from what is left. A streamer still
/// open after the call did not close: lost when the device is gone (an identity read tells), failed otherwise.
pub(crate) fn close_outcome(still_open: bool, device_lost: bool) -> CallResult {
    match (still_open, device_lost) {
        (false, _) => CallResult::Ok,
        (true, true) => CallResult::Lost,
        (true, false) => CallResult::Failed,
    }
}

/// A stream is receiving: its handle is open and its phase has a stream running, stopping or flushing.
pub(crate) fn receiving(cell: &Cell, handle_open: bool) -> bool {
    handle_open && matches!(cell.phase, Phase::Armed(_) | Phase::Streaming(_) | Phase::Stopping(_) | Phase::Flushing)
}

/// A flush still has left values to read: one not yet consumed by a sample.
pub(crate) fn flush_pending(cell: &Cell, seen: &BTreeSet<u64>) -> bool {
    cell.in_flight.iter().any(|f| f.left && !seen.contains(&f.entry.order.1))
}

/// A flush is over (its recv cycles end, and the final stop is next): no left value remains, the drain window has
/// passed, or the run's end has come.
pub(crate) fn flush_over(cell: &Cell, seen: &BTreeSet<u64>, flush_end: Option<i64>, now: i64, at_end: bool) -> bool {
    cell.phase == Phase::Flushing && (at_end || flush_end.is_some_and(|end| now >= end) || !flush_pending(cell, seen))
}

/// A recv's result as the executor reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Received {
    Lost,
    Late,
    Samples(Ticks),
}

/// The sample ticks `rx_recv` returned, as the executor's decision input: a lost device, a missed start, or samples
/// (a timeout or a non-lost failure carries none). Ticks are one per sample, `sample_ticks` device ticks apart (the decimation).
pub(crate) fn received_of(result: &RxRecv, sample_ticks: i64) -> Received {
    match result {
        RxRecv::Failed(error) if error.lost => Received::Lost,
        RxRecv::LateCommand => Received::Late,
        RxRecv::Samples { first_tick, samples } => Received::Samples(Ticks::new(*first_tick, sample_ticks, samples.first().map_or(0, Vec::len))),
        _ => Received::Samples(Ticks::default()),
    }
}

/// What a finished recv decides: a lost device ends it with the loss publication; a missed start reports the time to the
/// brain (provider/rx.rs `receive`) and carries no samples.
pub(crate) struct Ended {
    pub(crate) lost: bool,
    pub(crate) samples: Ticks,
    pub(crate) late_send: Option<Input>,
}

pub(crate) fn end_of_recv(received: Received, stream: usize, now: i64) -> Ended {
    match received {
        Received::Lost => Ended { lost: true, samples: Ticks::default(), late_send: None },
        Received::Late => Ended { lost: false, samples: Ticks::default(), late_send: Some(Input::LateCommand { stream, now }) },
        Received::Samples(samples) => Ended { lost: false, samples, late_send: None },
    }
}

/// The inputs a recv's end sends to the brain, in the one order both wrappers use (the experiment's choice, 45-experiment §11 B15): the
/// publication's boundaries first (acceptance has closed in the publish, `cell.rs` `publish_planned`, before `CutReached` is sent), then the
/// missed start's `LateCommand`, which the brain judges against the cell the publication left.
pub(crate) fn recv_inputs(boundaries: Vec<Input>, late: Option<Input>) -> Vec<Input> {
    boundaries.into_iter().chain(late).collect()
}

/// The handle change a call makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Handle {
    Keep,
    Opened,
    Closed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FlushEnd {
    Keep,
    Set(i64),
    Clear,
}

/// What a finished call leaves besides the cell's phase: the boundary it sends, the handle change and the flush window.
pub(crate) struct After {
    pub(crate) send: Option<Input>,
    pub(crate) handle: Handle,
    pub(crate) flush_end: FlushEnd,
}

/// The cell section after a device call (`R.result`): writes the device result into the cell, and decides the boundary
/// the executor sends. Every lifecycle call completes with a boundary, whatever its result.
pub(crate) fn after_call(cell: &mut Cell, stream: usize, call: RxCall, result: CallResult, now: i64, drain_window: i64) -> After {
    let boundary = |b: Boundary| Some(Input::Boundary(b));
    let mut after = After { send: None, handle: Handle::Keep, flush_end: FlushEnd::Keep };
    match call {
        RxCall::Begin { at, .. } => {
            let segment = SegmentKey { by: Some(cell.epoch), origin: at };
            cell.device_result(Call::Begin(segment), &result.result());
            if result == CallResult::Ok {
                cell.start_confirmed(at);
                after.send = boundary(Boundary::Armed { stream });
            } else if result == CallResult::Failed {
                after.send = boundary(Boundary::BeginFailed { stream });
            }
        }
        RxCall::Stop { epoch } => {
            cell.device_result(Call::Stop, &result.result());
            // A failed stop ends the segment too (its phase is Uncertain); a loss publishes instead.
            if result != CallResult::Lost {
                after.send = boundary(Boundary::Finished { stream, next_origin: None, epoch });
            }
        }
        RxCall::PlanStop { retiring } => {
            cell.device_result(Call::Stop, &result.result());
            cell.read_new_epoch();
            if result != CallResult::Lost {
                after.send = boundary(Boundary::Finished { stream, next_origin: None, epoch: retiring });
            }
        }
        RxCall::FlushStart => {
            cell.device_result(Call::FlushStart, &result.result());
            // A failed start skips the stop; the left values stay on the chain (production, provider/rx.rs).
            if result == CallResult::Ok {
                after.flush_end = FlushEnd::Set(now + drain_window);
            } else if result != CallResult::Lost {
                after.send = boundary(Boundary::Flushed { stream });
            }
        }
        RxCall::FlushStop => {
            cell.device_result(Call::FlushStop, &result.result());
            after.flush_end = FlushEnd::Clear;
            if result != CallResult::Lost {
                after.send = boundary(Boundary::Flushed { stream });
            }
        }
        RxCall::Open => {
            if result == CallResult::Ok {
                after.handle = Handle::Opened;
            }
            after.send = boundary(Boundary::Opened { stream });
        }
        RxCall::Close => {
            if result == CallResult::Ok {
                after.handle = Handle::Closed;
            }
            after.send = boundary(Boundary::Closed { stream });
        }
    }
    after
}

/// The consumption of `samples` by a stream's chain: `(value, sample)` for each value a sample consumes, computed by the
/// production rule ([`consume_packet`]) at the samples that can consume. `channels` are the receiving streamer's (the device's, not the
/// cell's). Left values are the cell's left values; debt is an accepted value of a finished call that no sample has consumed
/// yet. Values the executor already consumed (`seen`) are not offered again. A sample consumes the head of each channel's queue (left
/// values first) once its tick reaches the head's instant and acceptance, so the walk jumps to the earliest such tick, and ends
/// when nothing is pending: a block with nothing to consume costs no per-sample work.
pub(crate) fn consume_samples(cell: &Cell, channels: Channels, seen: &BTreeSet<u64>, samples: &Ticks) -> Vec<(u64, i64)> {
    let mut left: Vec<RxEntry> = Vec::new();
    let mut debt: Vec<RxEntry> = Vec::new();
    for flight in &cell.in_flight {
        let id = flight.entry.order.1;
        if seen.contains(&id) || !channels.contains(flight.entry.channel as u32) {
            continue;
        }
        if flight.left {
            left.push(flight.entry.clone());
        } else if flight.entry.accepted_at.is_some() {
            debt.push(flight.entry.clone());
        }
    }
    // Nothing pending: a block costs no allocation.
    if left.is_empty() && debt.is_empty() {
        return Vec::new();
    }
    let one_sample: Vec<Vec<Iq>> = channels.iter().map(|_| vec![[0.0, 0.0]]).collect();
    let mut consumed = Vec::new();
    let mut index = 0;
    while !(left.is_empty() && debt.is_empty()) {
        let reach = |entry: &RxEntry| entry.order.0.max(entry.accepted_at.unwrap_or(i64::MIN));
        let head = |entries: &[RxEntry], channel: usize| entries.iter().find(|entry| entry.channel == channel).map(reach);
        let earliest = channels.iter().filter_map(|channel| head(&left, channel as usize).or_else(|| head(&debt, channel as usize))).min();
        let Some(earliest) = earliest else { break };
        index = index.max(samples.index_from(earliest));
        if index >= samples.count {
            break;
        }
        let tick = samples.at(index);
        for (id, at) in consume_packet(&left, &debt, 1, channels, tick, &one_sample) {
            left.retain(|entry| entry.order.1 != id);
            debt.retain(|entry| entry.order.1 != id);
            consumed.push((id, at));
        }
        index += 1;
    }
    consumed
}

/// The result of a publish round.
#[derive(Debug)]
pub(crate) enum Round {
    /// The snapshot is stale: nothing was written; the caller recomputes from a fresh one.
    Stale,
    Published {
        /// The boundaries to send, in order: first sample, one `Consumed` per consumed value, cut reached.
        sends: Vec<Input>,
        /// `(value, sample tick)` consumed by this recv (published or discarded samples alike).
        consumed: Vec<(u64, i64)>,
        /// The ticks given to the user.
        published: Vec<i64>,
        /// The samples the cell discarded (past the cut of their epoch); a flush discards none.
        discarded: u64,
    },
}

/// What a publish computes from a view of the cell, outside any lock: the consumption, the ticks given to the user, the plan of
/// the write (none for a flush, whose samples are not published), and the phase and progress it read.
#[derive(Debug)]
pub(crate) struct Computed {
    pub(crate) consumed: Vec<(u64, i64)>,
    published: Vec<i64>,
    plan: Option<Plan>,
    phase: Phase,
}

/// `R.publish`, the computing half (note 45 §4.5, §4.6): reads `view`, writes nothing. A flush's samples are not published:
/// only the left values they consume are reported.
pub(crate) fn publish_compute(view: &Cell, recv: &Recv, carried: Channels, seen: &BTreeSet<u64>, want_ticks: bool) -> Computed {
    let flushing = view.phase == Phase::Flushing;
    let consumed = consume_samples(view, carried, seen, &recv.samples);
    let (published, plan) = if flushing {
        (Vec::new(), None)
    } else {
        let (origin, bound) = (view.origin_of(recv.epoch), view.bound(recv.epoch));
        // The user's ticks are built only for an observer that reads them (a measurement run counts and does not).
        (if want_ticks { published_ticks(origin, bound, &recv.samples) } else { Vec::new() }, Some(plan_publish(origin, bound, &recv.samples, view.progress)))
    };
    Computed { consumed, published, plan, phase: view.phase }
}

/// `R.publish`, the writing half: inside the cell, the recheck against the snapshot and the plan's effect, then the boundaries.
pub(crate) fn publish_write(cell: &mut Cell, stream: usize, snap: Snapshot, recv: &Recv, computed: Computed) -> Round {
    let Computed { consumed, published, plan, phase } = computed;
    let before = cell.progress;
    let cut_reached = match &plan {
        None => false,
        Some(plan) => match cell.publish_planned(snap, recv.epoch, plan) {
            Publish::Stale => return Round::Stale,
            Publish::Published { cut_reached } => cut_reached,
        },
    };
    let mut sends = Vec::new();
    if matches!(phase, Phase::Armed(_)) && cell.progress > before {
        sends.push(Input::Boundary(Boundary::FirstSample { stream }));
    }
    sends.extend(consumed.iter().map(|&(id, _)| Input::Boundary(Boundary::Consumed { id })));
    if cut_reached {
        sends.push(Input::Boundary(Boundary::CutReached { stream }));
    }
    let discarded = plan.as_ref().map_or(0, |plan| plan.discard);
    Round::Published { sends, consumed, published, discarded }
}

/// `R.publish` on the live cell (the world's one event): compute and write in one step.
pub(crate) fn publish_round(cell: &mut Cell, stream: usize, snap: Snapshot, recv: &Recv, carried: Channels, seen: &BTreeSet<u64>) -> Round {
    let computed = publish_compute(cell, recv, carried, seen, true);
    publish_write(cell, stream, snap, recv, computed)
}

/// The samples a publish gives the user.
pub(crate) fn published_ticks(origin: i64, bound: Option<i64>, samples: &Ticks) -> Vec<i64> {
    published_range(origin, bound, samples).map(|i| samples.at(i)).collect()
}

/// `S.take` over every cell: a timed value's take is the first cell that knows it; any other change is called.
pub(crate) fn take_command(cells: &mut [Cell], cmd: &SetCmd) -> Take {
    match cmd.change {
        Change::Timed { .. } => cells.iter_mut().map(|cell| cell.take(cmd.id)).find(|take| *take != Take::Absent).unwrap_or(Take::Absent),
        _ => Take::Call,
    }
}

/// `S.lock`: the call is about to be entered, in every cell.
pub(crate) fn start_calls(cells: &mut [Cell], id: u64) {
    for cell in cells.iter_mut() {
        cell.start_call(id);
    }
}

/// The device accepted the command, effective at `at`.
pub(crate) fn accept_all(cells: &mut [Cell], id: u64, at: i64) {
    for cell in cells.iter_mut() {
        cell.accept(id, at);
    }
}

/// `S.result`: writes the command's call result into every cell; an Ok move changes the channel sets (the cell that
/// owns the channel loses it; the values reserved on it stay with the chain they were applied to).
pub(crate) fn settle_call(cells: &mut [Cell], cmd: &SetCmd, terminal: Terminal) {
    if let Terminal::Call(outcome) = terminal {
        for cell in cells.iter_mut() {
            cell.finish_call(cmd.id, outcome);
        }
    }
    if let (Terminal::Call(Outcome::Ok), Change::Move { channel, to }) = (terminal, &cmd.change) {
        for cell in cells.iter_mut().filter(|cell| cell.channels.contains(*channel as u32)) {
            cell.channels = cell.channels.without(*channel as u32);
        }
        cells[*to].channels = cells[*to].channels.with(*channel as u32);
    }
}

/// An abandoned command's value, in every cell.
pub(crate) fn abandon_all(cells: &mut [Cell], id: u64) {
    for cell in cells.iter_mut() {
        cell.abandon(id);
    }
}

/// The commands a dead brain's senders issue as they close: `S` and every receive executor end after what they hold.
pub(crate) fn dead_commands(streams: usize, unspawned: &std::collections::BTreeSet<usize>, control_unspawned: bool) -> Vec<Command> {
    (!control_unspawned)
        .then_some(Command::SettingsTerminate)
        .into_iter()
        .chain((0..streams).filter(|stream| !unspawned.contains(stream)).map(|stream| Command::Rx { stream, op: RxOp::Terminate }))
        .collect()
}

/// Whether the brain's `at`-th step is the one a case replaces with a panic.
pub(crate) fn panics_at(at: usize, panic_step: Option<usize>) -> bool {
    panic_step == Some(at)
}

/// One call on the device, as the world and the thread run both log it (the observation of S1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DevCall {
    Open {
        stream: usize,
        channels: Vec<usize>,
    },
    Start {
        stream: usize,
        at: i64,
    },
    StartNow {
        stream: usize,
    },
    Stop {
        stream: usize,
    },
    Close {
        stream: usize,
    },
    Recv {
        stream: usize,
    },
    /// `Device::apply`: channel, the value (`freq` as an integer id; a restore is `-1 - channel`), the command time.
    Apply {
        channel: usize,
        freq: Option<i64>,
        at: Option<i64>,
    },
}

/// The `Device::apply` arguments a settings command makes: a timed value, a restore (untimed, a marker value); a move has no
/// settings call (it only takes the native lock, as a null call).
pub(crate) fn settings_call(change: &Change) -> Option<(usize, Option<i64>, Option<i64>)> {
    match *change {
        Change::Timed { channel, freq, at, .. } => Some((channel, Some(freq as i64), Some(at))),
        Change::Restore { channel } => Some((channel, Some(-1 - channel as i64), None)),
        Change::Move { .. } => None,
    }
}

/// The value a `Reserve` operation is about to reserve: `(id, stream, channel)` of the held value.
pub(crate) fn reserving(brain: &Brain, op: &Op) -> Option<(u64, usize, usize)> {
    let Op::Section(Section::Reserve { id }) = op else { return None };
    brain.held.iter().find(|held| held.entry.order.1 == *id).map(|held| (*id, held.stream, held.entry.channel))
}

/// After a `Reserve` operation: the reservation `(value, channel, segment, epoch)` if the value left the brain's held list.
pub(crate) fn reserved(brain: &Brain, cells: &[Cell], (id, stream, channel): (u64, usize, usize)) -> Option<(u64, usize, SegmentKey, u64)> {
    if brain.held.iter().any(|held| held.entry.order.1 == id) {
        return None;
    }
    let segment = match cells[stream].phase {
        Phase::Armed(segment) | Phase::Streaming(segment) => segment,
        _ => unreachable!("a reservation needs a running phase"),
    };
    Some((id, channel, segment, cells[stream].epoch))
}

/// Whether a cell has debt: a finished call's value that no sample has consumed yet (a left value included).
pub(crate) fn has_debt(cell: &Cell) -> bool {
    cell.in_flight.iter().any(|f| matches!(f.state, ValueState::Done(Terminal::Call(Outcome::Ok | Outcome::Partial))))
}

#[cfg(test)]
mod tests {
    use super::super::cell::Terminal;
    use super::super::fixtures::value;
    use super::*;

    fn zeros(channels: usize, samples: usize) -> Vec<Vec<Iq>> {
        vec![vec![[0.0, 0.0]; samples]; channels]
    }

    fn with_accepted(mut entry: RxEntry, accepted: i64) -> RxEntry {
        entry.accepted_at = Some(accepted);
        entry
    }

    /// The last sample of a packet is consumption evidence too (E1-49).
    #[test]
    fn u12e_last_sample_of_packet_is_evidence() {
        let consumed = consume_packet(&[], &[value(0, 8, 1)], 1, Channels::first(1), 8, &zeros(1, 1));
        assert_eq!(consumed, vec![(1, 8)]);
    }

    #[test]
    fn u12a_equal_time_values_take_distinct_samples() {
        let debt = [value(0, 8, 1), value(0, 8, 2)];
        let consumed = consume_packet(&[], &debt, 1, Channels::first(1), 8, &zeros(1, 3));
        assert_eq!(consumed, vec![(1, 8), (2, 9)]);
    }

    #[test]
    fn u12b_value_not_consumed_before_acceptance() {
        let debt = [with_accepted(value(0, 8, 1), 10)];
        let consumed = consume_packet(&[], &debt, 1, Channels::first(1), 8, &zeros(1, 3));
        assert_eq!(consumed, vec![(1, 10)]);
    }

    #[test]
    fn u12c_unconsumed_left_entry_holds_later_debt() {
        let left = [value(0, 100, 9)];
        let debt = [value(0, 20, 1)];
        let consumed = consume_packet(&left, &debt, 1, Channels::first(1), 20, &zeros(1, 4));
        assert!(consumed.is_empty());
    }

    /// The reference: the same state consumed by one `RxChain::consume` over the whole packet.
    fn reference(left: &[RxEntry], debt: &[RxEntry], channels: Channels, first_tick: i64, samples: &[Vec<Iq>]) -> Vec<u64> {
        let segment = SegmentKey { by: None, origin: i64::MIN };
        let mut chain = RxChain::default();
        chain.adopt_start(segment, i64::MIN);
        for entry in left {
            hand(&mut chain, entry);
        }
        chain.stop_result(Ok(())).expect("the reference chain stops");
        chain.finish();
        chain.adopt_start(segment, i64::MIN);
        for entry in debt {
            hand(&mut chain, entry);
        }
        let before = orders(&chain);
        chain.consume(1, channels, first_tick, samples);
        let after = orders(&chain);
        let mut removed: Vec<u64> = before.into_iter().filter(|order| !after.contains(order)).map(|order| order.1).collect();
        removed.sort_unstable();
        removed
    }

    #[test]
    fn u12d_scratch_adapter_matches_rxchain_consume() {
        let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = move |bound: u64| {
            state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) % bound
        };
        for _ in 0..200 {
            let channels = Channels::first(2);
            let mut id = 0;
            let mut entries = |count: u64, next: &mut dyn FnMut(u64) -> u64| -> Vec<RxEntry> {
                (0..count)
                    .map(|_| {
                        id += 1;
                        let accepted = (next(2) == 1).then(|| next(20) as i64);
                        let entry = value(next(2) as usize, next(20) as i64, id);
                        RxEntry { accepted_at: accepted, ..entry }
                    })
                    .collect()
            };
            let left = entries(next(3), &mut next);
            let debt = entries(next(3), &mut next);
            let first_tick = next(10) as i64;
            let count = 1 + next(6) as usize;
            let samples = zeros(2, count);
            let mut adapter: Vec<u64> = consume_packet(&left, &debt, 1, channels, first_tick, &samples).into_iter().map(|(id, _)| id).collect();
            adapter.sort_unstable();
            assert_eq!(adapter, reference(&left, &debt, channels, first_tick, &samples));
        }
    }

    /// The moved helper itself: each value is offered once; a persisted `seen` stops a second offer (E2-28).
    #[test]
    fn consume_samples_offers_each_value_once() {
        use super::super::cell::{InFlight, ValueState};
        use super::super::fixtures::cell;
        let mut c = cell(Phase::Streaming(SegmentKey { by: Some(1), origin: 0 }), 0, None, 0);
        for id in [1, 2] {
            let mut entry = value(0, 8, id);
            entry.accepted_at = Some(8);
            c.in_flight.push(InFlight { entry, state: ValueState::Done(Terminal::Call(Outcome::Ok)), left: false });
        }
        let channels = Channels::first(1);
        let mut seen = BTreeSet::new();
        assert_eq!(consume_samples(&c, channels, &seen, &Ticks::from_slice(&[6, 7, 8, 9, 10])), vec![(1, 8), (2, 9)], "first offer");
        seen.extend([1, 2]);
        assert_eq!(consume_samples(&c, channels, &seen, &Ticks::from_slice(&[10, 11])), vec![], "second offer, `seen` persisted");
        let only_first = BTreeSet::from([1]);
        assert_eq!(consume_samples(&c, channels, &only_first, &Ticks::from_slice(&[10])), vec![(2, 10)]);
        let mut left_entry = value(0, 100, 9);
        left_entry.accepted_at = Some(100);
        c.in_flight.push(InFlight { entry: left_entry, state: ValueState::Done(Terminal::Call(Outcome::Ok)), left: true });
        assert!(consume_samples(&c, channels, &seen, &Ticks::from_slice(&[99, 100])).contains(&(9, 100)), "a left value is consumed at its instant");
        let with_left = BTreeSet::from([1, 2, 9]);
        assert!(consume_samples(&c, channels, &with_left, &Ticks::from_slice(&[99, 100])).is_empty(), "and not again once seen");
    }

    /// A sample is `step` device ticks (the decimation): a value is consumed by the first sample at or after its instant, a value
    /// beyond the block's last sample is not consumed by it, and the progress passes the last published sample by one step.
    #[test]
    fn decimated_ticks_consume_at_the_first_sample_at_or_after_the_instant() {
        use super::super::cell::{InFlight, ValueState};
        use super::super::fixtures::cell;
        let mut c = cell(Phase::Streaming(SegmentKey { by: Some(1), origin: 0 }), 0, None, 0);
        for (id, e) in [(1, 9), (2, 10), (3, 31)] {
            let mut entry = value(0, e, id);
            entry.accepted_at = Some(0);
            c.in_flight.push(InFlight { entry, state: ValueState::Done(Terminal::Call(Outcome::Ok)), left: false });
        }
        let block = Ticks::new(4, 2, 10);
        assert_eq!(consume_samples(&c, Channels::first(1), &BTreeSet::new(), &block), vec![(1, 10), (2, 12)], "one value per sample; 3's instant is past the block's last tick, 22");
        assert_eq!(consume_samples(&c, Channels::first(1), &BTreeSet::new(), &Ticks::new(4, 2, 0)), vec![], "an empty block");
        let plan = plan_publish(6, Some(20), &block, 0);
        assert_eq!((plan.progress, plan.discard), (20, 3), "published ticks 6..=18, the progress is the next sample's tick");
    }

    /// Sol E2a-1 P6: the computation reads a view and writes nothing; the write half rechecks and refuses a cell that moved on.
    #[test]
    fn publish_write_rechecks_what_compute_read() {
        use super::super::fixtures::{cell, recv};
        let segment = SegmentKey { by: Some(1), origin: 0 };
        let mut c = cell(Phase::Streaming(segment), 0, Some(16), 0);
        let view = c.clone();
        let snap = c.snapshot();
        let samples = recv(1, &[0, 1, 2, 3]);
        let computed = publish_compute(&view, &samples, Channels::first(1), &BTreeSet::new(), true);
        assert_eq!((view.progress, view.in_flight.len()), (0, 0), "the computation wrote nothing");
        assert!(c.unchanged_since(&view));
        // The cell moves on between the view and the write: the recheck notices, and a stale snapshot is refused.
        c.book_stop(2);
        assert!(!c.unchanged_since(&view), "a booked cut is a change");
        let stale = publish_write(&mut c, 0, snap, &samples, publish_compute(&view, &samples, Channels::first(1), &BTreeSet::new(), true));
        assert!(matches!(stale, Round::Stale));
        assert_eq!(c.progress, 0, "nothing was written");
        // Unchanged: the write publishes what the view computed.
        let mut fresh = cell(Phase::Streaming(segment), 0, Some(16), 0);
        let fresh_snap = fresh.snapshot();
        let round = publish_write(&mut fresh, 0, fresh_snap, &samples, computed);
        let Round::Published { published, discarded, .. } = round else { panic!("a publish") };
        assert_eq!((published, discarded, fresh.progress), (vec![0, 1, 2, 3], 0, 4));
    }

    /// The loop decision: a set failure flag wins over everything, then a queued command, a flush that is over, a recv, idleness.
    #[test]
    fn next_action_decides_in_order() {
        assert_eq!(next_action(true, true, true, true, false), Next::Exit, "the failure flag wins");
        assert_eq!(next_action(true, false, false, false, false), Next::Exit, "even when nothing else is to do");
        assert_eq!(next_action(false, true, true, true, false), Next::Cmd);
        assert_eq!(next_action(false, false, true, true, false), Next::FlushStop);
        assert_eq!(next_action(false, false, false, true, false), Next::Recv);
        assert_eq!(next_action(false, false, false, true, true), Next::Idle, "no recv at the end of the run");
        assert_eq!(next_action(false, false, false, false, false), Next::Idle);
    }

    #[test]
    fn close_outcome_reads_what_is_left() {
        assert_eq!(close_outcome(false, false), CallResult::Ok);
        assert_eq!(close_outcome(false, true), CallResult::Ok);
        assert_eq!(close_outcome(true, false), CallResult::Failed);
        assert_eq!(close_outcome(true, true), CallResult::Lost);
    }

    #[test]
    fn u13a_loop_check_stops_before_call() {
        let mut fifo: VecDeque<u32> = VecDeque::from([1, 2]);
        assert_eq!(loop_check(true, &mut fifo), Check::Exit);
        assert!(fifo.is_empty());
        let mut kept: VecDeque<u32> = VecDeque::from([1]);
        assert_eq!(loop_check(false, &mut kept), Check::Proceed);
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn u13c_settings_discard_writes_abandoned_per_command() {
        let mut fifo = VecDeque::from([2, 3]);
        let mut terminals: Vec<(u64, Terminal)> = Vec::new();
        let check = settings_check(true, Some(1), &mut fifo, |id| terminals.push((id, Terminal::Abandoned)));
        assert_eq!(check, Check::Exit);
        assert_eq!(terminals, vec![(1, Terminal::Abandoned), (2, Terminal::Abandoned), (3, Terminal::Abandoned)]);
        assert!(fifo.is_empty());
    }
}
