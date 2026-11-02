//! One receive stream's cell (note 45 §4.5): its device phase, the values reserved on it and
//! the procedures that change them. Each procedure takes `&mut Cell` and never the device.

use ezsdr_radio::timeline::Channels;

use super::rules;
use crate::device::DeviceError;
use crate::provider::core::{RxEntry, SegmentKey};

/// The device state of the stream (note 44 §3.2; G15), with note 45's `Stopping` on top.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Off,
    Armed(SegmentKey),
    Streaming(SegmentKey),
    Stopping(SegmentKey),
    Flushing,
    Uncertain,
    Lost,
}

/// What a device call returned, for a call that was entered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Ok,
    Partial,
    Refused,
    /// The device was lost during the call.
    Lost,
}

/// The terminal state of a settings command: a call result, or a value that never reached a call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Terminal {
    Call(Outcome),
    Dropped,
    Abandoned,
}

/// The five states of a reserved value (note 45 §4.2).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ValueState {
    Queued,
    Cancelled,
    Taken,
    MaybeApplied,
    Done(Terminal),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct InFlight {
    pub(crate) entry: RxEntry,
    pub(crate) state: ValueState,
    /// Reported left at the segment's end (the brain's Finished section); consumption offers it as a leftover.
    pub(crate) left: bool,
}

/// Why a reservation was refused, in the order the procedure checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    Phase,
    Channel,
    Generation,
    Origin,
    Cut,
    Progress,
}

/// What the settings executor finds when it takes a queued command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Take {
    Call,
    Dropped,
    Absent,
}

/// The cell as a recv saw it: the publish is refused when any of these has changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Snapshot {
    pub(crate) epoch: u64,
    pub(crate) cut: Option<i64>,
    pub(crate) revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Publish {
    Stale,
    Published { cut_reached: bool },
}

/// The device ticks of one block's samples: one sample every `step` ticks (the decimation) from `first`. The protocol reads ticks only
/// through this, so a block is never expanded into a vector of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ticks {
    pub(crate) first: i64,
    pub(crate) step: i64,
    pub(crate) count: usize,
}

impl Default for Ticks {
    fn default() -> Self {
        Ticks { first: 0, step: 1, count: 0 }
    }
}

impl Ticks {
    pub(crate) fn new(first: i64, step: i64, count: usize) -> Ticks {
        assert!(step > 0, "a sample is at least one device tick long");
        Ticks { first, step, count }
    }

    /// The ticks of `ticks`, which must be an arithmetic progression (a block's samples are).
    pub(crate) fn from_slice(ticks: &[i64]) -> Ticks {
        let step = if ticks.len() >= 2 { ticks[1] - ticks[0] } else { 1 };
        assert!(ticks.windows(2).all(|pair| pair[1] - pair[0] == step), "a block's ticks are evenly spaced: {ticks:?}");
        Ticks::new(ticks.first().copied().unwrap_or(0), step, ticks.len())
    }

    pub(crate) fn at(&self, index: usize) -> i64 {
        self.first + index as i64 * self.step
    }

    /// The index of the first sample at or after `tick` (`count` when none is).
    pub(crate) fn index_from(&self, tick: i64) -> usize {
        let behind = i128::from(tick) - i128::from(self.first);
        if behind <= 0 { 0 } else { (((behind + i128::from(self.step) - 1) / i128::from(self.step)).min(self.count as i128)) as usize }
    }
}

/// A recv begun under `epoch`, with its identity and the sample ticks it buffered, in order.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Recv {
    pub(crate) epoch: u64,
    pub(crate) handle: u64,
    pub(crate) samples: Ticks,
}

/// The epoch a plan change replaced, while its recv may still publish (note 45 §4.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Retiring {
    pub(crate) epoch: u64,
    pub(crate) cut: Option<i64>,
    /// The origin of the replaced segment: its recv publishes from there.
    pub(crate) origin: i64,
    /// R has published the old epoch's recv and returned from its call and read the new epoch: the quiesce condition.
    pub(crate) published: bool,
}

/// A device call whose result changes the phase (the mirror of `RxChain`'s result methods).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Call {
    Begin(SegmentKey),
    Stop,
    FlushStart,
    FlushStop,
}

/// The drain of one stream's stop: idle, waiting for the cut or the deadline, or ended (the stop
/// was sent).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Drain {
    Idle,
    Draining,
    Ended,
    /// A stop booked while no stream runs: its drain starts when the stream's start is confirmed.
    Booked,
}

/// A lifecycle command the brain sent to a stream's executor and whose completion boundary has not
/// arrived: at most one per stream (a decision reads the phase, which that command will change).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outstanding {
    Start,
    Stop,
    Flush,
    Plan,
    Open,
    Close,
}

#[derive(Clone, Debug)]
pub(crate) struct Cell {
    pub(crate) phase: Phase,
    /// The drain of the stop booked in this segment (note 45 §4.4).
    pub(crate) drain: Drain,
    /// The command sent and not yet completed; cleared by its completion boundary.
    pub(crate) outstanding: Option<Outstanding>,
    pub(crate) channels: Channels,
    pub(crate) generation: u64,
    pub(crate) epoch: u64,
    /// The origin of the segment in force: its recv publishes from there (note 45 §4.6).
    pub(crate) origin: i64,
    /// The cut in force: the plan's, or the shorter one a Stop booked.
    pub(crate) cut: Option<i64>,
    pub(crate) retiring: Option<Retiring>,
    pub(crate) progress: i64,
    pub(crate) in_flight: Vec<InFlight>,
    /// A flush has been attempted since the latest left value was marked (its start or stop returned): no second flush
    /// is owed, and a left value that remains is read by a later segment's samples or never (note 44 §3.4).
    pub(crate) flushed: bool,
    pub(crate) revision: u64,
    /// Diagnostic: stale publishes refused.
    pub(crate) recomputes: u32,
    /// Diagnostic: samples a publish discarded.
    pub(crate) discarded: u64,
}

impl Cell {
    /// An empty placeholder: the cell that stands in a mutex while its content is moved out (thread runtime).
    pub(crate) fn vacant() -> Cell {
        Cell {
            phase: Phase::Off,
            drain: Drain::Idle,
            outstanding: None,
            channels: Channels::first(0),
            generation: 0,
            epoch: 0,
            origin: i64::MIN,
            cut: None,
            retiring: None,
            progress: 0,
            in_flight: Vec::new(),
            flushed: false,
            revision: 0,
            recomputes: 0,
            discarded: 0,
        }
    }

    /// Reservation of a held value in the segment the phase names (I2; note 45 §4.5 procedure 1).
    pub(crate) fn reserve(&mut self, entry: RxEntry, generation: u64) -> Result<(), Refusal> {
        let segment = match self.phase {
            Phase::Armed(s) | Phase::Streaming(s) => s,
            _ => return Err(Refusal::Phase),
        };
        if !self.channels.contains(entry.channel as u32) {
            return Err(Refusal::Channel);
        }
        if generation != self.generation {
            return Err(Refusal::Generation);
        }
        if entry.order.0 < segment.origin {
            return Err(Refusal::Origin);
        }
        if let Some(cut) = self.cut {
            if entry.order.0 >= cut {
                return Err(Refusal::Cut);
            }
            if self.progress >= cut {
                return Err(Refusal::Progress);
            }
        }
        self.in_flight.push(InFlight { entry, state: ValueState::Queued, left: false });
        self.revision += 1;
        Ok(())
    }

    /// The Stop's booking (note 45 §4.5 procedure 2): the cut is the requested instant, but not
    /// below the progress already published and never beyond the cut in force. Queued values at
    /// or after it are cancelled.
    pub(crate) fn book_stop(&mut self, requested: i64) -> i64 {
        let bound = self.cut.unwrap_or(i64::MAX);
        let cut = bound.min(requested.max(self.progress));
        self.cut = Some(cut);
        self.cancel_queued(|e| e >= cut);
        self.revision += 1;
        cut
    }

    fn cancel_queued(&mut self, pred: impl Fn(i64) -> bool) {
        for flight in self.in_flight.iter_mut().filter(|f| f.state == ValueState::Queued) {
            if pred(flight.entry.order.0) {
                flight.state = ValueState::Cancelled;
            }
        }
    }

    /// The settings executor takes a queued value for its call; a cancelled one is dropped.
    pub(crate) fn take(&mut self, id: u64) -> Take {
        let taken = match self.find_mut(id) {
            Some(flight) => match flight.state {
                ValueState::Queued => {
                    flight.state = ValueState::Taken;
                    Take::Call
                }
                ValueState::Cancelled => {
                    flight.state = ValueState::Done(Terminal::Dropped);
                    Take::Dropped
                }
                _ => Take::Absent,
            },
            None => Take::Absent,
        };
        if taken != Take::Absent {
            self.revision += 1;
        }
        taken
    }

    /// The call is about to be entered.
    pub(crate) fn start_call(&mut self, id: u64) {
        if let Some(flight) = self.find_mut(id) {
            if flight.state == ValueState::Taken {
                flight.state = ValueState::MaybeApplied;
            }
        }
    }

    /// The call returned: a taken or maybe-applied value is done with that result.
    pub(crate) fn finish_call(&mut self, id: u64, outcome: Outcome) {
        self.revision += 1;
        if let Some(flight) = self.find_mut(id) {
            if matches!(flight.state, ValueState::Taken | ValueState::MaybeApplied) {
                flight.state = ValueState::Done(Terminal::Call(outcome));
            }
        }
    }

    /// The Stopping transition (note 45 §4.5): acceptance closes, queued values are cancelled,
    /// and the values whose calls may still apply are returned as the obligations.
    pub(crate) fn stop_accepting(&mut self) -> Vec<u64> {
        self.cancel_queued(|_| true);
        if let Phase::Armed(s) | Phase::Streaming(s) = self.phase {
            self.phase = Phase::Stopping(s);
        }
        self.revision += 1;
        self.in_flight.iter().filter(|f| matches!(f.state, ValueState::Taken | ValueState::MaybeApplied)).map(|f| f.entry.order.1).collect()
    }

    /// The segment ends: nothing stays queued, and the next segment has no cut and no progress.
    pub(crate) fn finish(&mut self) {
        self.stop_accepting();
        self.cut = None;
        self.progress = 0;
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot { epoch: self.epoch, cut: self.cut, revision: self.revision }
    }

    /// Publishes what a recv buffered (note 45 §4.6). Refused when the epoch, the cut or the
    /// revision changed since `seen`: the caller takes a fresh snapshot and publishes again.
    pub(crate) fn publish(&mut self, seen: Snapshot, recv: &Recv) -> Publish {
        if self.stale(seen) {
            return Publish::Stale;
        }
        let plan = rules::plan_publish(self.origin_of(recv.epoch), self.bound(recv.epoch), &recv.samples, self.progress);
        self.publish_planned(seen, recv.epoch, &plan)
    }

    fn stale(&mut self, seen: Snapshot) -> bool {
        let stale = seen.epoch != self.epoch || seen.cut != self.cut || seen.revision != self.revision;
        if stale {
            self.recomputes += 1;
        }
        stale
    }

    /// The write half of a publish: the recheck (epoch, cut, revision) and the plan's effect, nothing computed from the
    /// samples. The caller computed `plan` from a view of this cell that it has found unchanged (note 45 §4.5).
    pub(crate) fn publish_planned(&mut self, seen: Snapshot, epoch: u64, plan: &rules::Plan) -> Publish {
        if self.stale(seen) {
            return Publish::Stale;
        }
        self.progress = plan.progress;
        self.discarded += plan.discard;
        if let Some(retiring) = self.retiring.as_mut().filter(|r| r.epoch == epoch) {
            retiring.published = true;
        }
        let cut_reached = self.cut.is_some_and(|cut| self.progress >= cut);
        if cut_reached {
            self.stop_accepting();
        }
        Publish::Published { cut_reached }
    }

    /// Whether everything a publish computation reads is as in `view` (a clone taken earlier): the snapshot, the values in
    /// flight, the retiring epoch, the phase, the origin and the progress.
    pub(crate) fn unchanged_since(&self, view: &Cell) -> bool {
        self.snapshot() == view.snapshot()
            && self.in_flight == view.in_flight
            && self.retiring == view.retiring
            && self.phase == view.phase
            && self.origin == view.origin
            && self.progress == view.progress
    }

    /// The cut that bounds the samples of `epoch`: the cut in force, or the retiring cut for the
    /// epoch that was replaced. An epoch that is neither publishes nothing.
    pub(crate) fn bound(&self, epoch: u64) -> Option<i64> {
        if epoch == self.epoch {
            return self.cut;
        }
        match self.retiring {
            Some(retiring) if retiring.epoch == epoch => retiring.cut,
            _ => Some(i64::MIN),
        }
    }

    /// The origin of the segment `epoch` names, as `bound` names its cut; an epoch that is neither
    /// publishes nothing.
    pub(crate) fn origin_of(&self, epoch: u64) -> i64 {
        if epoch == self.epoch {
            return self.origin;
        }
        match self.retiring {
            Some(retiring) if retiring.epoch == epoch => retiring.origin,
            _ => i64::MIN,
        }
    }

    /// R confirmed the start at `origin`: the new segment's recv publishes from there, and the
    /// progress starts there. A cut booked below the origin becomes the origin (production ends the
    /// stream at once, provider/rx.rs `follow`).
    pub(crate) fn start_confirmed(&mut self, origin: i64) {
        self.origin = origin;
        self.cut = self.cut.map(|cut| cut.max(origin));
        self.progress = origin;
        self.revision += 1;
    }

    /// A plan change: the epoch the cell is in moves on; the one it replaced keeps the cut that
    /// was in force, for the recv that is still reading it. The phase is not written here: the
    /// start of the new epoch writes it when R confirms the start (note 45 §4.5).
    pub(crate) fn advance_epoch(&mut self, epoch: u64, plan_cut: Option<i64>) {
        self.retiring = Some(Retiring { epoch: self.epoch, cut: self.cut, origin: self.origin, published: false });
        self.epoch = epoch;
        self.cut = plan_cut;
        self.revision += 1;
    }

    /// R has returned from the old epoch's call and read the new epoch (note 45 §4.5 procedure 3).
    pub(crate) fn read_new_epoch(&mut self) {
        if let Some(retiring) = self.retiring.as_mut() {
            retiring.published = true;
        }
    }

    /// The old epoch's recv has published its samples, so its quiesce can be acknowledged (I6).
    pub(crate) fn ack_quiesce(&mut self, epoch: u64) -> bool {
        match self.retiring {
            Some(retiring) if retiring.epoch == epoch && retiring.published => {
                self.retiring = None;
                true
            }
            _ => false,
        }
    }

    /// The result of a device call that the phase depends on (the mirror of `RxChain`'s result
    /// methods: a failed begin keeps the previous state; a failed stop is `Uncertain`).
    pub(crate) fn device_result(&mut self, call: Call, result: &Result<(), DeviceError>) {
        if matches!(call, Call::FlushStart | Call::FlushStop) {
            self.flushed = true;
        }
        self.phase = match call {
            Call::Begin(segment) if matches!(self.phase, Phase::Off | Phase::Uncertain) => match result {
                Ok(()) => Phase::Armed(segment),
                Err(error) if error.lost => Phase::Lost,
                Err(_) => self.phase,
            },
            Call::Stop if matches!(self.phase, Phase::Armed(_) | Phase::Streaming(_) | Phase::Stopping(_)) => match result {
                Ok(()) => Phase::Off,
                Err(error) if error.lost => Phase::Lost,
                Err(_) => Phase::Uncertain,
            },
            Call::FlushStart if self.phase == Phase::Off => match result {
                Ok(()) => Phase::Flushing,
                Err(error) if error.lost => Phase::Lost,
                Err(_) => Phase::Off,
            },
            Call::FlushStop if self.phase == Phase::Flushing => match result {
                Ok(()) => Phase::Off,
                Err(error) if error.lost => Phase::Lost,
                Err(_) => Phase::Uncertain,
            },
            _ => self.phase,
        };
    }

    /// Removes the value from the cell (its disposition is the brain's to write).
    pub(crate) fn retire(&mut self, id: u64) {
        self.in_flight.retain(|f| f.entry.order.1 != id);
        self.revision += 1;
    }

    /// The settings executor discards a command it will not call (I1): its value is abandoned in
    /// whatever state it has, and no send follows.
    pub(crate) fn abandon(&mut self, id: u64) {
        if let Some(flight) = self.find_mut(id) {
            if !matches!(flight.state, ValueState::Done(_)) {
                flight.state = ValueState::Done(Terminal::Abandoned);
                self.revision += 1;
            }
        }
    }

    /// The device accepted the value's command, effective at `at`: its acceptance instant, which
    /// consumption waits for.
    pub(crate) fn accept(&mut self, id: u64, at: i64) {
        if let Some(flight) = self.find_mut(id) {
            flight.entry.accepted_at = Some(at);
        }
    }

    /// A value is left at its segment's end; a flush is owed again for it.
    pub(crate) fn mark_left(&mut self, id: u64) {
        if let Some(flight) = self.find_mut(id) {
            flight.left = true;
            self.flushed = false;
        }
    }

    pub(crate) fn is_left(&self, id: u64) -> bool {
        self.in_flight.iter().any(|f| f.entry.order.1 == id && f.left)
    }

    pub(crate) fn state_of(&self, id: u64) -> Option<&ValueState> {
        self.in_flight.iter().find(|f| f.entry.order.1 == id).map(|f| &f.state)
    }

    /// A pin (note 45 §4.4): a value whose call may still be running.
    pub(crate) fn pinned(&self) -> bool {
        self.in_flight.iter().any(|f| matches!(f.state, ValueState::Taken | ValueState::MaybeApplied))
    }

    pub(crate) fn pinned_on(&self, channel: usize) -> bool {
        self.in_flight.iter().any(|f| f.entry.channel == channel && matches!(f.state, ValueState::Taken | ValueState::MaybeApplied))
    }

    /// A value committed to S ahead of a lifecycle decision (note 45 §4.4): queued (S takes it
    /// first), taken, or maybe applied, on `channel` or on the whole chain. A cancelled value is
    /// dropped at take and commits nothing.
    pub(crate) fn committed(&self, channel: Option<usize>) -> bool {
        self.in_flight.iter().any(|f| channel.is_none_or(|c| f.entry.channel == c) && matches!(f.state, ValueState::Queued | ValueState::Taken | ValueState::MaybeApplied))
    }

    /// The stream is busy: an R command is outstanding, its drain is `Booked` or `Draining` (a stop is
    /// decided but not yet sent), or it is `Stopping` (its stop is not yet finished). A lifecycle
    /// decision on the stream, or a settings change that reads it, waits until the work already
    /// handed out has landed.
    pub(crate) fn busy(&self) -> bool {
        self.phase != Phase::Lost && (self.outstanding.is_some() || self.drain == Drain::Draining || matches!(self.phase, Phase::Stopping(_)))
    }

    /// The completion of `kind` clears the outstanding command when it is that command.
    pub(crate) fn settle(&mut self, kind: Outstanding) {
        if self.outstanding == Some(kind) {
            self.outstanding = None;
        }
    }

    /// A flush is owed: the cell is Off, no flush has been attempted since, and it holds a left value whose call returned a
    /// result (note 45 §4.5). A restore waits for it (I15).
    pub(crate) fn flush_owed(&self) -> bool {
        self.phase == Phase::Off && !self.flushed && self.in_flight.iter().any(|f| f.left && matches!(f.state, ValueState::Done(Terminal::Call(Outcome::Ok | Outcome::Partial))))
    }

    /// A left value on `channel` stays after the flush was attempted: only a later sample reads it, so a restore is
    /// skipped (production `restore_rx` skips a channel with left on a chain).
    pub(crate) fn left_after_flush(&self, channel: usize) -> bool {
        self.flushed && self.in_flight.iter().any(|f| f.left && f.entry.channel == channel)
    }

    /// Outstanding consumption on `channel`: a call finished, and no sample has consumed it yet
    /// (debt, or left once its segment has finished).
    pub(crate) fn owed_on(&self, channel: usize) -> bool {
        self.in_flight.iter().any(|f| f.entry.channel == channel && matches!(f.state, ValueState::Done(Terminal::Call(Outcome::Ok | Outcome::Partial))))
    }

    fn find_mut(&mut self, id: u64) -> Option<&mut InFlight> {
        self.in_flight.iter_mut().find(|f| f.entry.order.1 == id)
    }
}

#[cfg(test)]
mod tests {
    use super::super::core::{Brain, Change, Command, Input, SetCmd};
    use super::super::fixtures::{cell, feed, held, push, recv, seg, value};
    use super::*;

    /// A held value released into a one-stream brain with `cells`, then the outbox.
    fn release(cells: &mut [Cell], entry: RxEntry, generation: u64) -> (Brain, Vec<Command>) {
        let mut brain = Brain::new();
        feed(&mut brain, cells, Input::Release(held(0, entry, generation)));
        let outbox = brain.outbox.clone();
        (brain, outbox)
    }

    fn timed(id: u64, channel: usize, freq: f64, at: i64, generation: u64) -> Command {
        Command::Settings(SetCmd { id, change: Change::Timed { channel, freq, at, generation } })
    }

    #[test]
    fn u01_streaming_value_in_segment_reserves() {
        let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, Some(16), 4)];
        let (_, outbox) = release(&mut cells, value(0, 8, 1), 1);
        assert_eq!(cells[0].in_flight.len(), 1);
        assert_eq!(cells[0].in_flight[0].state, ValueState::Queued);
        assert_eq!(outbox, vec![timed(1, 0, 1.0, 8, 1)]);
    }

    #[test]
    fn u01b_armed_value_reserves() {
        let mut cells = vec![cell(Phase::Armed(seg(0)), 0, Some(16), 4)];
        let (_, outbox) = release(&mut cells, value(0, 8, 1), 1);
        assert_eq!(cells[0].in_flight.len(), 1);
        assert_eq!(outbox, vec![timed(1, 0, 1.0, 8, 1)]);
    }

    #[test]
    fn u01c_instant_equal_origin_reserves() {
        let mut cells = vec![cell(Phase::Streaming(seg(0)), 0, Some(16), 4)];
        let (_, outbox) = release(&mut cells, value(0, 0, 1), 1);
        assert_eq!(cells[0].in_flight[0].state, ValueState::Queued);
        assert_eq!(outbox, vec![timed(1, 0, 1.0, 0, 1)]);
    }

    /// The refusal of U2: the cell's procedure returns `want`, and through the brain the value
    /// stays held with an empty outbox.
    fn refused(mut cell: Cell, entry: RxEntry, generation: u64, want: Refusal) {
        assert_eq!(cell.reserve(entry.clone(), generation), Err(want));
        let mut cells = vec![cell];
        let (brain, outbox) = release(&mut cells, entry, generation);
        assert_eq!(brain.held.len(), 1);
        assert!(outbox.is_empty());
    }

    #[test]
    fn u02a_off_refuses() {
        refused(cell(Phase::Off, 0, Some(16), 4), value(0, 8, 1), 1, Refusal::Phase);
    }

    #[test]
    fn u02b_stopping_refuses() {
        refused(cell(Phase::Stopping(seg(0)), 0, Some(16), 4), value(0, 8, 1), 1, Refusal::Phase);
    }

    #[test]
    fn u02c_other_channel_refuses() {
        refused(cell(Phase::Streaming(seg(0)), 0, Some(16), 4), value(1, 8, 1), 1, Refusal::Channel);
    }

    #[test]
    fn u02d_stale_generation_refuses() {
        refused(cell(Phase::Streaming(seg(0)), 0, Some(16), 4), value(0, 8, 1), 0, Refusal::Generation);
    }

    #[test]
    fn u02e_instant_before_origin_refuses() {
        refused(cell(Phase::Streaming(seg(4)), 0, Some(16), 4), value(0, 2, 1), 1, Refusal::Origin);
    }

    #[test]
    fn u02f_instant_at_cut_refuses() {
        refused(cell(Phase::Streaming(seg(0)), 0, Some(16), 4), value(0, 16, 1), 1, Refusal::Cut);
    }

    #[test]
    fn u02g_progress_at_cut_refuses() {
        refused(cell(Phase::Streaming(seg(0)), 0, Some(16), 16), value(0, 8, 1), 1, Refusal::Progress);
    }

    #[test]
    fn u05a_stopping_cancels_queued_and_lists_obligations() {
        let mut c = cell(Phase::Streaming(seg(0)), 0, Some(16), 4);
        push(&mut c, value(0, 8, 1), ValueState::Queued);
        push(&mut c, value(0, 9, 2), ValueState::Taken);
        push(&mut c, value(0, 10, 3), ValueState::MaybeApplied);
        let obligations = c.stop_accepting();
        assert_eq!(c.state_of(1), Some(&ValueState::Cancelled));
        assert_eq!(obligations, vec![2, 3]);
    }

    #[test]
    fn u06a_stop_below_progress_clamps_to_progress() {
        let mut c = cell(Phase::Streaming(seg(0)), 0, Some(16), 8);
        assert_eq!(c.book_stop(4), 8);
    }

    /// A stop booked before its start is confirmed (progress still 0) cuts at 6; the segment starts
    /// at 8, so the cut becomes the origin and the progress starts there, with nothing published.
    #[test]
    fn u06c_booked_cut_below_origin_becomes_origin() {
        let mut c = cell(Phase::Off, 0, None, 0);
        assert_eq!(c.book_stop(6), 6);
        c.start_confirmed(8);
        assert_eq!((c.cut, c.progress), (Some(8), 8));
    }

    #[test]
    fn u06b_stop_beyond_plan_cut_keeps_plan_cut() {
        let mut c = cell(Phase::Streaming(seg(0)), 0, Some(16), 4);
        assert_eq!(c.book_stop(20), 16);
    }

    #[test]
    fn u07a_stale_snapshot_refused_and_counted() {
        let mut c = cell(Phase::Streaming(seg(0)), 0, Some(16), 4);
        let seen = c.snapshot();
        assert_eq!(c.book_stop(6), 6);
        assert_eq!(c.publish(seen, &recv(1, &[4, 5])), Publish::Stale);
        assert_eq!(c.recomputes, 1);
    }

    #[test]
    fn u07b_result_between_snapshot_and_publish_refused() {
        let mut c = cell(Phase::Streaming(seg(0)), 0, Some(16), 4);
        push(&mut c, value(0, 2, 9), ValueState::Taken);
        let seen = c.snapshot();
        c.finish_call(9, Outcome::Ok);
        let before = c.in_flight.clone();
        assert_eq!(c.publish(seen, &recv(1, &[4, 5])), Publish::Stale);
        assert_eq!(c.recomputes, 1);
        assert_eq!(c.in_flight, before);
        assert_eq!(c.state_of(9), Some(&ValueState::Done(Terminal::Call(Outcome::Ok))));
    }

    #[test]
    fn u07c_recompute_publishes_only_to_new_cut() {
        let mut c = cell(Phase::Streaming(seg(0)), 0, Some(16), 4);
        let seen = c.snapshot();
        c.book_stop(6);
        assert_eq!(c.publish(seen, &recv(1, &[4, 5, 6, 7])), Publish::Stale);
        let fresh = c.snapshot();
        assert!(matches!(c.publish(fresh, &recv(1, &[4, 5, 6, 7])), Publish::Published { .. }));
        assert_eq!(c.progress, 6);
    }

    #[test]
    fn u07d_recompute_keeps_recv_identity() {
        let mut c = cell(Phase::Streaming(seg(0)), 0, Some(16), 4);
        let seen = c.snapshot();
        let buffered = recv(1, &[4, 5, 6, 7]);
        c.book_stop(6);
        c.advance_epoch(2, Some(48));
        assert_eq!((buffered.epoch, buffered.handle), (1, 7));
        assert_eq!(c.publish(seen, &buffered), Publish::Stale);
        let fresh = c.snapshot();
        assert!(matches!(c.publish(fresh, &buffered), Publish::Published { .. }));
        assert_eq!((c.progress, c.discarded), (6, 2));
    }

    /// The old epoch of U8: a Stop booked the cut at 10 and the plan changed under a recv.
    fn old_epoch() -> Cell {
        let mut c = cell(Phase::Streaming(seg(0)), 0, Some(16), 8);
        c.book_stop(10);
        c.advance_epoch(2, Some(48));
        c
    }

    #[test]
    fn u08a_old_buffer_published_to_retiring_cut() {
        let mut c = old_epoch();
        let buffered = recv(1, &[8, 9, 10, 11]);
        assert!(matches!(c.publish(c.snapshot(), &buffered), Publish::Published { .. }));
        assert_eq!(c.progress, 10);
    }

    #[test]
    fn u08b_rest_of_old_buffer_discarded_and_counted() {
        let mut c = old_epoch();
        let buffered = recv(1, &[8, 9, 10, 11]);
        c.publish(c.snapshot(), &buffered);
        assert_eq!(c.discarded, 2);
    }

    #[test]
    fn u08c_quiesce_ack_after_publish() {
        let mut c = old_epoch();
        assert_eq!(c.phase, Phase::Streaming(seg(0)), "a plan change does not write the phase; the new start does");
        let buffered = recv(1, &[8, 9, 10, 11]);
        assert!(!c.ack_quiesce(1));
        c.publish(c.snapshot(), &buffered);
        assert!(c.ack_quiesce(1));
    }

    #[test]
    fn u08d_new_epoch_publishes_from_its_origin_only() {
        let mut c = old_epoch();
        c.start_confirmed(20);
        assert_eq!(c.progress, 20);
        let fresh = recv(2, &[19, 20, 21]);
        assert!(matches!(c.publish(c.snapshot(), &fresh), Publish::Published { .. }));
        assert_eq!(c.progress, 22);
        assert_eq!(c.discarded, 1);
    }
}
