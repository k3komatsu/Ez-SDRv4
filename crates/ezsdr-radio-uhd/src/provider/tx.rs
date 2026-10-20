//! uhd-tx: owns the transmit streamer. It transmits held bursts in target order,
//! keeps at most the in-flight window handed to the device ahead of its time, decides
//! a preempting burst against the first sample it has not handed over, performs the
//! transmit side of a `cold` change, and reads the asynchronous reports (UR-21…UR-23,
//! UR-25, UR-28).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration as Wall;

use ezsdr_kernel::contract::DataContractId;
use ezsdr_kernel::event::Severity;
use ezsdr_kernel::id::ClockDomainId;
use ezsdr_kernel::stream::{
    BlockFlags, BlockHeader, BurstOpen, BurstStep, BurstTracker, ChannelMask, Direction, LateOutcome, LatePolicy,
};
use ezsdr_kernel::time::{Duration, TimePoint};
use ezsdr_radio::kinds;
use ezsdr_radio::payloads::{TimeErrorCause, TimeErrorOutcome, TimeErrorPayload, TxUnderflowCause, TxUnderflowPayload};
use serde_json::json;

use super::control::Planned;
use super::core::{stream_key, Clock, Core, Streams, lock};
use crate::device::{Dir, Iq, TxCode, TxReport};
use crate::host::HostClock;
use crate::profile::{DEVICE_LEAD_NS, IN_FLIGHT_WINDOW_NS};
use ezsdr_radio::timeline::{Channels, StreamKey};

/// A burst uhd-control booked (UR-21).
pub(crate) struct Held {
    pub k: i64,
    pub domain: ClockDomainId,
    pub samples: Arc<Vec<Vec<Iq>>>,
    pub repeat: bool,
    pub open: BurstOpen,
    pub policy: LatePolicy,
    pub target: TimePoint,
}

pub(crate) enum TxCmd {
    Burst(Held),
    /// `Stop` for `<id>/tx` or `<id>`: it ends the bursts, not the clock (RM-16, UR-26).
    Stop,
    /// `Provider::stop` (UR-26).
    Shutdown,
}

struct Open {
    held: Held,
    next: i64,
    first: bool,
}

/// The current clock's cut and the segment after it, which becomes current there (UR-25).
struct Switch {
    /// The root tick of the current clock's cut.
    cut: i64,
    to: Option<Planned>,
}

struct PendingReport {
    target: TimePoint,
    channels: BTreeSet<usize>,
}

pub(crate) struct Tx {
    core: Arc<Core>,
    /// The stream's number, `k` (RM-29): every device call and line reference names it.
    k: usize,
    cmds: Receiver<TxCmd>,
    /// The device's host clock (plan 30 §4.3): the idle wait and the shutdown's ACK window.
    host: HostClock,
    clock: Option<Clock>,
    /// The channel set the last successful switch took, for the bursts' channel count (RM-29).
    /// The streamer's set is the stream's record (UR-13), not this.
    channels: Channels,
    /// The generation of the streamer `unacked`'s correlations were taken against (UR-36,
    /// UR-28): the record's generation as this owner last read it, so that a streamer another
    /// thread opened — uhd-control's enable from no stream — is told from this one's.
    streamer: u64,
    held: BTreeMap<i64, Held>,
    later: Vec<Held>,
    open: Option<Open>,
    tracker: Option<BurstTracker>,
    switch: Option<Switch>,
    /// The transmit plan as last read, the version it was read at, and the index in it of the
    /// current segment, or of the next one to begin while there is no clock.
    plan: Vec<Planned>,
    version: Option<u64>,
    at: usize,
    unacked: VecDeque<PendingReport>,
    /// The device burst is still open, its next sample this one: a held burst starting
    /// there continues it, without end-of-burst and start-of-burst between them, since
    /// the X300 drops a timed start at the tick its previous burst ended (UR-23;
    /// design-notes §11 F3). The Kernel bursts keep their own records.
    continues_at: Option<i64>,
    /// The last sample of a Kernel burst, one per channel, held back while its device
    /// burst stays open: the end-of-burst goes with it, so that the device burst ends at
    /// the burst's own end and not a padded sample later, as an empty end-of-burst would
    /// (UHD sends one zero sample; `hw_b8_raw_empty_eob_gap`; Review O, O-B1).
    tail: Option<Vec<Vec<Iq>>>,
}

const IDLE: Wall = Wall::from_micros(200);
const SEND_TIMEOUT: Wall = Wall::from_secs(1);

impl Tx {
    pub fn new(core: Arc<Core>, k: usize, cmds: Receiver<TxCmd>, clock: Option<Clock>, channels: Channels) -> Tx {
        let host = core.device.host();
        // The streamer this owner starts on: the record's, opened by `arm` or by uhd-control
        // (UR-36).
        let streamer = lock(&core.streams).stream(stream_key(Dir::Tx, k)).generation;
        Tx {
            core,
            k,
            cmds,
            host,
            tracker: clock.map(|c| BurstTracker::new(c.domain)),
            clock,
            channels,
            streamer,
            held: BTreeMap::new(),
            later: Vec::new(),
            open: None,
            switch: None,
            plan: Vec::new(),
            version: None,
            at: 0,
            unacked: VecDeque::new(),
            continues_at: None,
            tail: None,
        }
    }

    pub fn run(mut self) {
        loop {
            loop {
                match self.cmds.try_recv() {
                    Ok(TxCmd::Shutdown) | Err(TryRecvError::Disconnected) => return self.shutdown(),
                    Ok(cmd) => self.command(cmd),
                    Err(TryRecvError::Empty) => break,
                }
            }
            if self.core.is_lost() {
                self.host.sleep(Wall::from_millis(1));
                continue;
            }
            if !self.pass() {
                self.host.sleep(IDLE);
            }
        }
    }

    /// One turn of the run loop (UR-23, UR-25): the plan, read again when uhd-control has
    /// changed it, and the report read's ownership are one section of the instance's line
    /// state — so a turn cannot pass between them, and the enable can never find the plan read
    /// without the read in flight too —; then the report read, then one step. A turn
    /// uhd-control's enable from no stream owns the streamer for makes no device call at all —
    /// `reports` and `step` included — and nothing of the owner's state moves. False when the
    /// turn had nothing to do, which the run loop's idle wait follows.
    pub(super) fn pass(&mut self) -> bool {
        let core = self.core.clone();
        let key = stream_key(Dir::Tx, self.k);
        let mut streams = lock(&core.streams);
        if streams.stream(key).busy {
            return false;
        }
        if !self.plan_section(&mut streams, key, None) {
            return false;
        }
        streams.stream_mut(key).reading = true;
        drop(streams);
        self.reports(Wall::ZERO);
        self.end_reading();
        self.step()
    }

    fn command(&mut self, cmd: TxCmd) {
        match cmd {
            TxCmd::Burst(held) => {
                if self.clock.is_some_and(|c| c.domain == held.domain) {
                    self.held.insert(held.k, held);
                } else {
                    // Booked on the clock a pending change starts (UR-25).
                    self.later.push(held);
                }
            }
            TxCmd::Stop => self.stop_now("UR-26: cancelled by Stop", true),
            TxCmd::Shutdown => unreachable!("handled by run"),
        }
    }

    /// Reads the transmit plan it shares with uhd-control again, when it has changed (UR-25),
    /// pruning first the items before the command that began the segment `passed` (#63).
    /// False when uhd-control's enable from no stream owns the streamer this turn: the plan is
    /// read again at the next, and the version, the switch and the idle mark stand.
    pub(super) fn read(&mut self, passed: Option<Option<u64>>) -> bool {
        let core = self.core.clone();
        let key = stream_key(Dir::Tx, self.k);
        let mut streams = lock(&core.streams);
        if streams.stream(key).busy {
            return false;
        }
        self.plan_section(&mut streams, key, passed)
    }

    /// The plan inside the caller's section of the instance's line state (UR-25, #63): read
    /// again when uhd-control has changed it, pruning first the items before `passed`. False
    /// only when the section may not read at all, which the caller has already checked.
    fn plan_section(&mut self, streams: &mut Streams, key: StreamKey, passed: Option<Option<u64>>) -> bool {
        match passed {
            Some(by) => streams.prune(key, by),
            None if self.version == Some(streams.version) => return true,
            None => {}
        }
        self.version = Some(streams.version);
        self.plan = streams.planned(key);
        self.retarget();
        streams.stream_mut(key).idle = self.clock.is_none() && self.switch.is_none();
        true
    }

    fn end_reading(&mut self) {
        lock(&self.core.streams).stream_mut(stream_key(Dir::Tx, self.k)).reading = false;
    }

    /// UR-25: the plan's cut of the current clock, and the segment after it; with no clock,
    /// the next segment, which begins at once.
    fn retarget(&mut self) {
        self.switch = match self.clock {
            Some(clock) => self.plan.get(self.at).and_then(|current| current.segment.cut)
                .map(|cut| Switch { cut: clock.instant(cut), to: self.plan.get(self.at + 1).cloned() }),
            None => self.plan.get(self.at).cloned().map(|to| Switch { cut: i64::MIN, to: Some(to) }),
        };
    }

    /// UR-25: at a switch, the bursts booked on the new clock are held; those on a clock still
    /// to come stay booked, and those on one the switch passed, or that ends at its origin,
    /// are cancelled.
    fn adopt_later(&mut self) {
        let domain = self.clock.map(|c| c.domain);
        let next = self.at + usize::from(self.clock.is_some());
        for held in std::mem::take(&mut self.later) {
            if Some(held.domain) == domain {
                self.held.insert(held.k, held);
            } else if self.plan.iter().skip(next).any(|p| p.clock.is_some_and(|c| c.domain == held.domain) && p.segment.cut != Some(0)) {
                self.later.push(held);
            } else {
                self.forget(&held);
                self.core.command_rejected(None, "tx_burst", "UR-25: its transmit clock was never started");
            }
        }
    }

    fn forget(&self, held: &Held) {
        lock(&self.core.held).remove(&(held.domain, held.k));
    }

    fn remember_burst(&mut self, target: TimePoint) {
        // UR-28: a pending report's channels are the positions in the streamer — the
        // numbers UHD's asynchronous reports carry (4.10's `rfnoc_tx_streamer.cpp:152-159,
        // 312-319`; `FakeDevice`, `device.rs:1515-1516,1555-1556`) —, not the device
        // channels the stream runs: a stream opened for `{1}` reports channel 0. The set is
        // never empty: a burst is booked only on a stream this Module opened, and a stream
        // with no channel is refused (UR-21).
        self.unacked.push_back(PendingReport { target, channels: (0..self.channels.count() as usize).collect() });
    }

    /// Each channel's reports are ordered independently in UHD's shared queue.
    fn reported_target(&mut self, channel: usize) -> Option<TimePoint> {
        let index = self.unacked.iter().position(|pending| pending.channels.contains(&channel))?;
        let pending = &mut self.unacked[index];
        let target = pending.target;
        pending.channels.remove(&channel);
        if pending.channels.is_empty() { self.unacked.remove(index); }
        Some(target)
    }

    /// One unit of work; false when there was nothing to do.
    pub(super) fn step(&mut self) -> bool {
        if self.core.is_lost() { return false; }
        let Some(clock) = self.clock else {
            if self.switch.is_some() {
                self.do_switch();
                return true;
            }
            return false;
        };
        let now = self.core.now();
        let window = self.core.ticks(IN_FLIGHT_WINDOW_NS);
        let cut_k = self.switch.as_ref().map(|s| clock.at_or_after(s.cut));
        if self.open.is_none() {
            if let Some(cut) = self.switch.as_ref().map(|s| s.cut) {
                if now >= cut {
                    self.do_switch();
                    return true;
                }
            }
        }
        if let Some(next) = self.open.as_ref().map(|o| o.next) {
            if clock.instant(next) < now {
                self.core.reject_note(json!({ "kind": "tx_burst", "reason": "UR-22: the transmit timeline expired; burst not resumed" }));
                self.abandon(true);
                return true;
            }
            if let Some(&h) = self.held.keys().next() {
                if h <= next && cut_k.is_none_or(|cut| h < cut) {
                    self.preempt(clock, h, next);
                    return true;
                }
            }
            if cut_k.is_some_and(|cut| next >= cut) {
                self.end_open(true);
                return true;
            }
            if clock.instant(next) - now >= window {
                return false;
            }
            let held_cut = self.held.keys().next().copied();
            self.send(clock, held_cut, cut_k);
            return true;
        }
        if let Some(c) = self.continues_at {
            if clock.instant(c) < now {
                self.core.reject_note(json!({ "kind": "tx_burst", "reason": "UR-22: the transmit timeline expired; burst not resumed" }));
                self.abandon(true);
                return true;
            }
            match self.held.keys().next() {
                // Not across the cut: there the device burst ends (Review O, N-4).
                Some(&k) if k == c && cut_k.is_none_or(|cut| c < cut) => {}
                // Another burst is next, not the continuation: end the device burst first.
                Some(_) => {
                    self.end_open(true);
                    return true;
                }
                // None booked yet: the device burst stays open for one booked at `c` until
                // `c` is a device lead away, then ends before it runs dry (Review N, B3).
                None if clock.instant(c) - now <= self.core.ticks(DEVICE_LEAD_NS) => {
                    self.end_open(true);
                    return true;
                }
                None => return false,
            }
        }
        if let Some(&k) = self.held.keys().next() {
            if cut_k.is_some_and(|cut| k >= cut) {
                return false;
            }
            if clock.instant(k) - now < window {
                let held = self.held.remove(&k).expect("the first key");
                // Continuations have no timed SOB; a new device burst must still
                // have its device lead after waiting in the owner's queue.
                let held = if self.continues_at == Some(k) {
                    held
                } else {
                    let Some(held) = self.dispatch_start(clock, held) else { return true };
                    held
                };
                if cut_k.is_some_and(|cut| held.k >= cut) {
                    // A late-policy move cannot send on the old clock past its cut.
                    // The pending switch will cancel this old-clock reservation.
                    self.held.insert(held.k, held);
                    return true;
                }
                if self.held.keys().next().is_some_and(|k| *k < held.k) {
                    self.held.insert(held.k, held);
                    return true;
                }
                let next = held.k;
                self.open = Some(Open { held, next, first: true });
                let held_cut = self.held.keys().next().copied();
                self.send(clock, held_cut, cut_k);
                return true;
            }
        }
        false
    }

    /// Hands the next buffer of the open burst to the device (UR-22, UR-23).
    fn send(&mut self, clock: Clock, held_cut: Option<i64>, switch_cut: Option<i64>) {
        let chunk = (self.core.ticks(IN_FLIGHT_WINDOW_NS) / clock.n / 4).clamp(1, self.core.block_len as i64);
        let open = self.open.as_mut().expect("a burst is open");
        let len = open.held.samples[0].len() as i64;
        let start = open.held.k;
        let offset = if open.held.repeat { (open.next - start).rem_euclid(len) } else { open.next - start };
        let mut count = chunk.min(len - offset);
        let cut = [held_cut, switch_cut].into_iter().flatten().min();
        if let Some(cut) = cut {
            count = count.min(cut - open.next);
        }
        let count = count.max(1);
        let ends_waveform = !open.held.repeat && offset + count == len;
        let ends_at_held = held_cut.is_some_and(|h| open.next + count == h) && switch_cut.is_none_or(|cut| h_lt(held_cut, cut));
        let eob = ends_waveform || ends_at_held;
        let slices: Vec<&[Iq]> = open.held.samples.iter().map(|ch| &ch[offset as usize..(offset + count) as usize]).collect();
        let first = open.first;
        // The Kernel's burst boundaries (`first`, `eob`) and the device's differ where one
        // burst continues the device burst of the one before (§11 F3).
        let continuing = first && self.continues_at == Some(start);
        let device_sob = first && !continuing;
        // A Kernel burst's end is the device burst's only once no burst can still be booked
        // at its next sample (it is a device lead away): otherwise its last sample is held
        // back, and `step` sends it with end-of-burst at that deadline, or without it ahead
        // of a burst that continues the device burst there (Review N, B3; Review O, O-B1).
        // A one-sample burst's first buffer is not held: its start goes with it.
        let open_ended = clock.instant(open.next + count) - self.core.now() > self.core.ticks(DEVICE_LEAD_NS);
        let hold = eob && !ends_at_held && open_ended && !(device_sob && count == 1);
        let device_eob = eob && !ends_at_held && !hold;
        let sending = if hold { count - 1 } else { count } as usize;
        let at = device_sob.then(|| clock.instant(start));
        if continuing {
            if let Some(tail) = self.tail.take() {
                // The burst before's held sample, ahead of this one, no end-of-burst between;
                // without it this one would play a sample early, so its failure abandons this
                // burst, the device burst ended (Review Q, NB-Q2).
                let tail: Vec<&[Iq]> = tail.iter().map(Vec::as_slice).collect();
                match self.core.device.tx_send(self.k, &tail, None, false, false, SEND_TIMEOUT) {
                    Ok(1) => {}
                    Ok(_) => {
                        self.core.reject_note(json!({ "kind": "tx_burst", "reason": "UR-22: a send did not complete within 1 s" }));
                        return self.abandon(true);
                    }
                    Err(error) => {
                        self.core.device_failed(None, "tx_burst", &error);
                        return self.abandon(true);
                    }
                }
            }
        }
        let sent: Vec<&[Iq]> = slices.iter().map(|ch| &ch[..sending]).collect();
        let result = if sending > 0 { self.core.device.tx_send(self.k, &sent, at, device_sob, device_eob, SEND_TIMEOUT) } else { Ok(0) };
        // Held only once the samples before it went out: after a send that failed or came
        // up short it would play straight after what the device took (Review P, NB-1).
        let tail: Option<Vec<Vec<Iq>>> = hold.then(|| slices.iter().map(|ch| ch[sending..].to_vec()).collect());
        let open = self.open.as_mut().expect("a burst is open");
        let next = open.next;
        // The device burst is open when it was before this send, or this one's start went.
        let was_open = !first || continuing;
        match result {
            Ok(sent) if sent == sending => {}
            Ok(sent) => {
                self.core.reject_note(json!({ "kind": "tx_burst", "reason": "UR-22: a send did not complete within 1 s" }));
                if device_sob && sent > 0 {
                    self.remember_burst(TimePoint::new(clock.domain, start));
                }
                return self.abandon(was_open || sent > 0);
            }
            Err(error) => {
                // Whether UHD sent packets before the error is unknown: a start that may have
                // gone is taken as a device burst begun, ended and counted for its report (an
                // end-of-burst outside a burst costs one zero sample; Review Q, NB-Q1).
                self.core.device_failed(None, "tx_burst", &error);
                if device_sob {
                    self.remember_burst(TimePoint::new(clock.domain, start));
                }
                return self.abandon(true);
            }
        }
        self.tail = tail;
        let mut flags = BlockFlags::NONE;
        if first {
            flags = flags | BlockFlags::START_OF_BURST;
        }
        if eob {
            flags = flags | BlockFlags::END_OF_BURST;
        }
        let header = BlockHeader {
            first_sample_time: TimePoint::new(clock.domain, next),
            len: count as u32,
            channels: self.channels.count().max(1) as u16,
            direction: Direction::Tx,
            valid: ChannelMask::full(self.channels.count().max(1) as u16),
            flags,
            lost: None,
            contract: DataContractId::parse("ezsdr.stream.cf32").expect("cf32"),
        };
        let open_record = first.then_some(open.held.open);
        if device_sob {
            self.remember_burst(TimePoint::new(clock.domain, start));
        }
        if first {
            self.continues_at = None;
            self.core.stat("tx_bursts", 1);
        }
        self.core.stat("tx_samples", count);
        let open = self.open.as_mut().expect("still open");
        open.first = false;
        open.next += count;
        if let Some(tracker) = self.tracker.as_mut() {
            let step = tracker.on_block(&header, open_record);
            self.record(step);
        }
        if eob {
            let open = self.open.take().expect("open");
            if !device_eob {
                self.continues_at = Some(open.next);
            }
            self.forget(&open.held);
        }
    }

    fn record(&self, step: Result<BurstStep, ezsdr_kernel::stream::StreamError>) {
        let mut rec = lock(&self.core.rec);
        match step {
            Ok(BurstStep::Ended { record }) => rec.bursts.push(serde_json::to_value(record).expect("a record")),
            Ok(BurstStep::Discontinuity { closed, then_ended, .. }) => {
                rec.bursts.push(serde_json::to_value(closed).expect("a record"));
                if let Some(ended) = then_ended {
                    rec.bursts.push(serde_json::to_value(ended).expect("a record"));
                }
            }
            Ok(_) => {}
            Err(error) => rec.rejected.push(json!({ "burst_tracker": error.to_string() })),
        }
    }

    /// Ends the open burst without recalling what was handed over (RM-16), and with
    /// `device_eob` the device burst too, also one a held burst was to continue (§11 F3).
    fn end_open(&mut self, device_eob: bool) {
        let continuing = self.continues_at.take();
        let tail = self.tail.take();
        let device_open = self.open.as_ref().map_or(continuing.is_some(), |open| !open.first || continuing == Some(open.held.k));
        if device_eob && device_open && !self.core.is_lost() {
            self.close_device_burst(tail);
        } else if !device_eob {
            // Still open for a continuation (`preempt`): the held sample stays.
            self.tail = tail;
        }
        let Some(open) = self.open.take() else { return };
        if let Some(record) = self.tracker.as_mut().and_then(BurstTracker::stop) {
            lock(&self.core.rec).bursts.push(serde_json::to_value(record).expect("a record"));
        }
        self.forget(&open.held);
    }

    /// Ends the open device burst: with the held last sample when there is one, else with
    /// an empty buffer, which UHD pads to one zero sample (a repeat stopped mid-waveform).
    fn close_device_burst(&self, tail: Option<Vec<Vec<Iq>>>) {
        if self.core.is_lost() { return; }
        let empty: Vec<Iq> = Vec::new();
        let buffers: Vec<&[Iq]> = match &tail {
            Some(tail) => tail.iter().map(Vec::as_slice).collect(),
            None => vec![&empty; self.channels.count().max(1) as usize],
        };
        if let Err(error) = self.core.device.tx_send(self.k, &buffers, None, false, true, SEND_TIMEOUT) {
            self.core.device_failed(None, "tx_burst", &error);
        }
    }

    fn abandon(&mut self, device_open: bool) {
        // A send that failed or came up short: end the device burst if one is open, so
        // that none is left without end-of-burst (Review N, N7), with an empty buffer —
        // the burst's samples after what the device took are not sent (Review P, NB-1).
        self.continues_at = None;
        self.tail = None;
        if device_open && !self.core.is_lost() {
            self.close_device_burst(None);
        }
        if let Some(record) = self.tracker.as_mut().and_then(BurstTracker::stop) {
            lock(&self.core.rec).bursts.push(serde_json::to_value(record).expect("a record"));
        }
        if let Some(open) = self.open.take() {
            self.forget(&open.held);
        }
    }

    fn time_error(&self, outcome: TimeErrorOutcome, late_by: Duration, target: TimePoint) {
        // SC-27's late_by is a host.monotonic duration, so its ticks are nanoseconds.
        let late_by_ns = late_by.ticks_in(ClockDomainId::HOST_MONOTONIC).unwrap_or(i64::MAX);
        let payload = serde_json::to_value(TimeErrorPayload { cause: TimeErrorCause::Late, outcome, late_by_ns, target })
            .expect("a payload");
        self.core.emit(&self.core.stream_node(Dir::Tx, self.k), kinds::TIME_ERROR, Severity::Error, payload);
    }

    /// UR-21: booking's verdict can expire before the first device hand-over.
    fn dispatch_start(&self, clock: Clock, mut held: Held) -> Option<Held> {
        let lead = self.core.ticks(DEVICE_LEAD_NS);
        let now = clock.at_or_after(ezsdr_radio::timeline::decided_at(self.core.now(), clock.origin, lead));
        let outcome = held.policy.decide(&self.core.clocks,
            TimePoint::new(clock.domain, held.k), TimePoint::new(clock.domain, now),
            Duration::new(ClockDomainId::HOST_MONOTONIC, DEVICE_LEAD_NS));
        match outcome {
            Ok(LateOutcome::OnTime {}) => Some(held),
            Ok(late @ LateOutcome::SendAsap { late_by }) => {
                let start = clock.at_or_after(clock.instant(now) + lead);
                let mut reservations = lock(&self.core.held);
                if reservations.contains(&(held.domain, start)) {
                    reservations.remove(&(held.domain, held.k));
                    drop(reservations);
                    self.time_error(TimeErrorOutcome::Refused, late_by, held.target);
                    self.core.command_rejected(None, "tx_burst", "UR-21: the moved start is a held burst's");
                    return None;
                }
                reservations.remove(&(held.domain, held.k));
                reservations.insert((held.domain, start));
                drop(reservations);
                held.k = start;
                held.open.late = Some(late);
                held.open.requested_target = held.open.requested_target.or(Some(held.target));
                self.time_error(TimeErrorOutcome::SendAsap, late_by, held.target);
                Some(held)
            }
            Ok(LateOutcome::Drop { late_by }) => {
                self.forget(&held);
                self.time_error(TimeErrorOutcome::Drop, late_by, held.target);
                self.core.reject_note(json!({ "kind": "tx_burst", "reason": "UR-21: the late policy dropped the burst" }));
                None
            }
            Ok(LateOutcome::PlanViolation { late_by }) => {
                self.forget(&held);
                self.time_error(TimeErrorOutcome::PlanViolation, late_by, held.target);
                self.core.reject_note(json!({ "kind": "tx_burst", "reason": "UR-21: the planned burst arrived late" }));
                None
            }
            Err(error) => {
                self.forget(&held);
                self.core.command_rejected(None, "tx_burst", &format!("UR-21: {error}"));
                None
            }
        }
    }

    /// RM-15 as VE-2 amends it: a burst that would start while another is open is
    /// decided again here, against the first sample not yet handed over.
    fn preempt(&mut self, clock: Clock, h: i64, next: i64) {
        let mut held = self.held.remove(&h).expect("the first key");
        if h < next {
            let lead_k = (self.core.ticks(DEVICE_LEAD_NS) + clock.n - 1) / clock.n;
            let outcome = held.policy.decide(
                &self.core.clocks,
                TimePoint::new(clock.domain, h),
                TimePoint::new(clock.domain, next - lead_k),
                Duration::new(ClockDomainId::HOST_MONOTONIC, DEVICE_LEAD_NS),
            );
            match outcome {
                Ok(LateOutcome::OnTime {}) => held.k = next,
                Ok(late @ LateOutcome::SendAsap { late_by }) => {
                    let mut reservations = lock(&self.core.held);
                    if reservations.contains(&(held.domain, next)) {
                        reservations.remove(&(held.domain, held.k));
                        drop(reservations);
                        self.time_error(TimeErrorOutcome::Refused, late_by, held.target);
                        return self.core.command_rejected(None, "tx_burst", "UR-21: the moved start is a held burst's");
                    }
                    reservations.remove(&(held.domain, held.k));
                    reservations.insert((held.domain, next));
                    drop(reservations);
                    self.time_error(TimeErrorOutcome::SendAsap, late_by, held.target);
                    held.k = next;
                    held.open.late = Some(late);
                    held.open.requested_target = held.open.requested_target.or(Some(held.target));
                }
                Ok(LateOutcome::Drop { late_by }) => {
                    self.forget(&held);
                    self.time_error(TimeErrorOutcome::Drop, late_by, held.target);
                    return self.core.reject_note(json!({ "kind": "tx_burst", "reason": "UR-21: the late policy dropped the burst" }));
                }
                Ok(LateOutcome::PlanViolation { late_by }) => {
                    self.forget(&held);
                    self.time_error(TimeErrorOutcome::PlanViolation, late_by, held.target);
                    return self.core.reject_note(json!({ "kind": "tx_burst", "reason": "UR-21: the planned burst arrived late" }));
                }
                Err(error) => {
                    self.forget(&held);
                    return self.core.command_rejected(None, "tx_burst", &format!("UR-21: {error}"));
                }
            }
        }
        // UR-23: the new burst starts at the open one's next sample (`held.k == next`).
        // If the device burst has begun, it continues into the new one without
        // end-of-burst (the X300 drops a timed start at the tick a burst ended, §11 F3);
        // otherwise nothing was sent and the new one starts on its own (Review L, P1-4).
        if self.open.as_ref().is_some_and(|open| !open.first || self.continues_at == Some(open.held.k)) {
            self.end_open(false);
            self.continues_at = Some(held.k);
        } else {
            self.end_open(true);
        }
        self.held.insert(held.k, held);
    }

    /// Ends the open burst and cancels every held one (UR-23, UR-26).
    fn stop_now(&mut self, reason: &str, emit: bool) {
        self.end_open(true);
        let held: Vec<Held> = std::mem::take(&mut self.held).into_values().chain(std::mem::take(&mut self.later)).collect();
        for held in held {
            self.forget(&held);
            if emit {
                self.core.command_rejected(None, "tx_burst", reason);
            } else {
                self.core.reject_note(json!({ "kind": "tx_burst", "reason": reason }));
            }
        }
    }

    /// The transmit side of a `cold` change, at the old clock's cut (UR-25): the open burst
    /// ends, the old clock's held bursts are cancelled, and the next segment becomes current —
    /// its streamer reopened when its channel count changed and the direction configured,
    /// unless uhd-control did both for an enable from no stream. A configuration the device
    /// refuses halts the stream (RM-25).
    fn do_switch(&mut self) {
        let Some(switch) = self.switch.take() else { return };
        self.end_open(true);
        for (_, held) in std::mem::take(&mut self.held) {
            self.forget(&held);
            self.core.command_rejected(None, "tx_burst", "cancelled by a cold change");
        }
        let passed = self.clock.take().map(|_| self.plan.get(self.at).and_then(|planned| planned.segment.by));
        if passed.is_some() {
            self.at += 1;
        }
        self.tracker = None;
        if let Some(to) = switch.to.filter(|_| !self.core.is_lost()) {
            // UR-37: a refused switch makes no device call, and starts no clock; the record
            // decides what an accepted one opens and configures (UR-13, UR-36).
            let configured = if to.refused { Ok(()) } else { self.core.owner_configure(Dir::Tx, self.k, &to) };
            self.sync_streamer();
            match configured {
                // UR-29: another thread found the device lost while the configuration ran;
                // the clock is not adopted and no segment begins.
                Ok(()) if self.core.is_lost() => self.at += 1,
                // UR-25: a refused enable from no stream starts no clock, as the refusal below
                // does; uhd-control reported it.
                Ok(()) if to.refused => self.at += 1,
                Ok(()) => {
                    self.channels = to.set();
                    self.clock = to.clock;
                    self.tracker = to.clock.map(|clock| BurstTracker::new(clock.domain));
                }
                Err(error) => {
                    // RM-25: the refusal halts the stream, booked by uhd-tx itself.
                    if let Some(by) = to.segment.by {
                        lock(&self.core.streams).refuse(&self.core, stream_key(Dir::Tx, self.k), by);
                    }
                    self.at += 1;
                    self.core.device_failed(None, "update_parameter", &error);
                }
            }
            self.core.timing(json!({ "what": "tx_switch", "cut": switch.cut, "origin": to.segment.origin, "at": self.core.now() }));
        }
        self.adopt_later();
        // #63: the segment switched from has ended.
        self.read(Some(passed.flatten()));
    }

    /// UR-28, UR-36: a report queue belongs to the streamer of its generation. The record's
    /// generation moved whenever any thread opened one — this owner's own reopen or uhd-control's
    /// enable from no stream — and with it the correlations of the bursts in flight against the
    /// replaced queue, which are dropped here before any report is correlated with them.
    fn sync_streamer(&mut self) {
        let streamer = lock(&self.core.streams).stream(stream_key(Dir::Tx, self.k)).generation;
        if streamer != self.streamer {
            self.unacked.clear();
        }
        self.streamer = streamer;
    }

    /// UR-28: every report recorded; underflows and late bursts become events.
    fn reports(&mut self, timeout: Wall) {
        while !self.core.is_lost() {
            match self.core.device.tx_async(self.k, timeout) {
                Ok(Some(report)) => {
                    self.sync_streamer();
                    self.report(report)
                }
                Ok(None) => break,
                // UR-28: a failed read is the device's loss or a recorded error, never "no report".
                Err(error) if error.lost => return self.core.device_lost(&error.message),
                Err(error) => {
                    // Counted, with the first one's text: a failing read repeats at the poll rate.
                    let first = lock(&self.core.rec).stats.get("tx_errors").is_none_or(|n| *n == 0);
                    self.core.stat("tx_errors", 1);
                    if first {
                        self.core.reject_note(json!({ "call": "tx_async", "error": error.message }));
                    }
                    // UR-33: a read that failed at once costs the timeout it would have
                    // waited. A caller measuring clock time across a loop (shutdown's ACK
                    // window) then cannot spin without the clock ever advancing, which on
                    // the manual clock would stop the very time it measures and never end.
                    self.host.sleep(timeout);
                    break;
                }
            }
        }
    }

    fn report(&mut self, report: TxReport) {
        lock(&self.core.rec).device_async.push(json!({
            "code": format!("{:?}", report.code), "tick": report.tick, "channel": report.channel,
        }));
        let time = match (self.clock, report.tick) {
            (Some(clock), Some(tick)) => TimePoint::new(clock.domain, (tick - clock.origin).div_euclid(clock.n)),
            _ => self.core.at(self.core.now()),
        };
        let underflow = |cause| {
            let payload = serde_json::to_value(TxUnderflowPayload { cause }).expect("a payload");
            self.core.emit_at(&self.core.stream_node(Dir::Tx, self.k), kinds::TX_UNDERFLOW, Severity::Warning, payload, time);
        };
        match report.code {
            TxCode::BurstAck => {
                self.reported_target(report.channel);
            }
            TxCode::Underflow | TxCode::UnderflowInPacket | TxCode::SeqError | TxCode::SeqErrorInBurst => {
                let cause = if matches!(report.code, TxCode::Underflow | TxCode::UnderflowInPacket) {
                    TxUnderflowCause::Starved
                } else { TxUnderflowCause::Lost };
                underflow(cause);
                // Its nominal next sample no longer bounds actual playback. Do not
                // resume untimed payload on the shifted device timeline (UR-22).
                if self.open.is_some() || self.continues_at.is_some() {
                    self.core.reject_note(json!({ "kind": "tx_burst", "reason": "UR-22: the device burst underflowed; burst not resumed" }));
                    self.abandon(true);
                }
            }
            TxCode::TimeError => {
                let Some(target) = self.reported_target(report.channel) else { return };
                let late_by_ns = match report.tick {
                    Some(tick) => {
                        let root = self.core.root;
                        let start = match self.core.clocks.convert(target, root).and_then(|c| c.floor().ticks_in(root)) {
                            Ok(start) => start,
                            Err(error) => return self.core.command_rejected(None, "tx_report", &format!("UR-28: {error}")),
                        };
                        match i64::try_from((i128::from(tick) - i128::from(start)) * 1_000_000_000 / i128::from(self.core.mcr)) {
                            Ok(ns) => ns,
                            Err(_) => return self.core.command_rejected(None, "tx_report", "UR-28: lateness overflow"),
                        }
                    }
                    None => 0,
                };
                self.time_error(TimeErrorOutcome::LateAtDevice, Duration::new(ClockDomainId::HOST_MONOTONIC, late_by_ns), target);
            }
            TxCode::Other(_) => {}
        }
    }

    fn shutdown(mut self) {
        self.stop_now("UR-26: cancelled by Provider::stop", false);
        // 100 ms of the device's host clock for the last BURST_ACK (UR-26; plan 30 §4.3: it
        // waits for the device's acknowledgement, so it is measured in clock time). A loss —
        // found by a read here or recorded by another thread — ends the window: `reports` no
        // longer reads, so it no longer waits on the clock either, and the manual clock this
        // busy thread holds still would never reach `until` (UR-33).
        let until = self.host.now() + Wall::from_millis(100);
        while !self.core.is_lost() && self.host.now() < until {
            // UR-25: the shutdown's read is a report read all the same, but it takes no busy
            // check: a uhd-control the join bound left detached (UR-16) may never clear its
            // mark, and must not hold this window open.
            lock(&self.core.streams).stream_mut(stream_key(Dir::Tx, self.k)).reading = true;
            self.reports(Wall::from_millis(10));
            self.end_reading();
        }
    }
}

fn h_lt(held_cut: Option<i64>, cut: i64) -> bool {
    held_cut.is_some_and(|h| h < cut)
}

#[cfg(test)]
mod tests {
    //! uhd-tx driven directly, where the order of its commands and steps matters and the
    //! Provider's threads cannot arrange it (the exit review's UR-21 clause).

    use std::collections::BTreeMap;
    use std::sync::Arc;

    use ezsdr_kernel::event::{Action, ActionId, EventCollector, EventKind};
    use ezsdr_kernel::hash::ContentHash;
    use ezsdr_kernel::id::ResourceId;
    use ezsdr_kernel::module_api::{ActionReceiver, Authority, Dispatched, StopMode, UpdateClass};
    use ezsdr_kernel::policy::{EventKindRegistry, Policy};
    use ezsdr_kernel::spec::Value;
    use ezsdr_kernel::stream::{BurstOpen, LatePolicy};
    use ezsdr_kernel::time::{AbsoluteDeadline, ClockRegistry, TimePoint};

    use super::{Held, Tx, TxCmd};
    use crate::device::{Device, Dir, FakeConfig, FakeDevice, Settings, TxCode, TxReport};
    use super::super::test_support::{HoldApply, ReportFeed};
    use crate::provider::core::{stream_key, Clock, Core, lock};
    use crate::provider::test_support::device_at;
    use ezsdr_radio::timeline::Channels;
    use crate::DeviceAuthority;
    use super::super::control::Control;

    fn held(clock: &super::Clock, k: i64, len: usize, policy: LatePolicy) -> Held {
        Held {
            k,
            domain: clock.domain,
            samples: Arc::new(vec![vec![[0.25f32, 0.0]; len]]),
            repeat: false,
            open: BurstOpen { waveform_len: None, late: None, requested_target: None },
            policy,
            target: TimePoint::new(clock.domain, k),
        }
    }

    /// A switch at `cut` to the segment of `clock`, `channels` channels configured with
    /// `settings`, or to no stream (UR-25).
    fn switch(cut: i64, clock: Option<super::Clock>, channels: usize, settings: Settings) -> super::Switch {
        let to = clock.map(|clock| super::Planned {
            segment: ezsdr_radio::timeline::Segment {
                origin: clock.origin,
                cut: None,
                config: ezsdr_radio::timeline::Config { channels: ezsdr_radio::timeline::Channels::first(channels as u32), ratio: ezsdr_kernel::time::Rational::new(clock.n as u64, 1).unwrap() },
                by: Some(1),
            },
            settings: Some(Arc::new(super::super::control::ColdConfig::new(settings))),
            k: 0,
            refused: false,
            clock: Some(clock),
        });
        super::Switch { cut, to }
    }

    struct NoActions;
    impl ActionReceiver for NoActions {
        fn recv(&self) -> Option<Dispatched> { None }
    }

    /// uhd-control with no stream of either direction: what the S series drives beside the
    /// owner for an enable from no stream (UR-13).
    fn control_of(core: &Arc<Core>) -> super::super::control::Control {
        let (to_tx, _cmds) = std::sync::mpsc::channel();
        super::super::control::Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], core.mode.defaults(), false,
            super::super::control::Start { t0: 0, rx: vec![None; core.mode.stream_count(Dir::Rx)], tx: vec![None; core.mode.stream_count(Dir::Tx)] })
    }

    /// uhd-control books a change of the transmit stream's channel count at `ms` from no stream
    /// (UR-13): the owner is idle, as uhd-control finds it between two segments.
    fn tx_channels_from_none(control: &mut super::super::control::Control, core: &Core, id: u64, ms: i64, channels: i64) {
        lock(&core.streams).stream_mut(stream_key(Dir::Tx, 0)).idle = true;
        control.book(Dispatched::new(ActionId(id), Action::UpdateParameter {
            target: super::super::test_support::target_of(&core.id),
            key: crate::provider::core::key("radio.tx.channels"),
            value: Value::from(channels),
            class: UpdateClass::Cold,
            at: Some(AbsoluteDeadline::new(core.at(core.ticks(ms * 1_000_000)))),
        }, Some(core.id.clone())));
    }

    /// The transmit plan as its owner reads it.
    fn plan_of(core: &Core, dir: Dir) -> Vec<super::super::control::Planned> {
        lock(&core.streams).planned(stream_key(dir, 0))
    }

    #[test]
    fn ur_13_a_transmit_switch_is_compared_with_the_stored_outcome() {
        // UR-13 (Sol 3): a transmit segment uhd-control did not configure is opened at its cut
        // by uhd-tx, and that open compares the rate set again, which returns the outcome the
        // direction's first comparison stored: a refusal refuses the segment and nothing is
        // configured. The state is set up here directly: uhd-tx meets it only inside its own
        // switch, which no test can hold open.
        let (core, device, _, events) = super::super::test_support::rig();
        // The first comparison refuses (a rate read that fails); every later open sees it.
        assert!(core.check_rates(Dir::Tx, 99).is_err());
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        tx.switch = Some(switch(0, Some(clock), 1, Settings::default()));
        tx.do_switch();
        assert!(device.calls().iter().all(|call| !call.starts_with("apply")), "{:?}", device.calls());
        let refused: Vec<String> = events
            .drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0))
            .into_iter()
            .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED)
            .filter_map(|event| event.payload["reason"].as_str().map(str::to_owned))
            .collect();
        assert_eq!(refused.len(), 1, "{refused:?}");
        assert!(refused[0].starts_with("UR-13:"), "{refused:?}");
    }

    #[test]
    fn ur_25_a_refused_enable_leaves_its_set_unopened_for_the_next_switch() {
        // UR-25, UR-13 (Sol re-review 1): an enable from no stream that uhd-control refused
        // (its open failed) leaves the streamer unopened, so the next change to the same set
        // opens and compares it again. The second change is booked after uhd-tx read the plan
        // and before it takes the first switch, so the order is fixed without sleeps.
        use ezsdr_radio::timeline::{Channels, Config, Item, Kind, Line, Stream, NO_RELEASE};
        let (core, device, _, _) = super::super::test_support::rig();
        device.tx_open(0, &[0]).unwrap();
        let c0 = core.register(Dir::Tx, 200, 0).unwrap();
        let config = |n: u64| Config { channels: Channels::first(1), ratio: ezsdr_kernel::time::Rational::new(n, 1).unwrap() };
        let key = stream_key(Dir::Tx, 0);
        {
            let mut streams = lock(&core.streams);
            streams.stream_mut(key).clocks = vec![(c0, false)];
            let lead = core.ticks(ezsdr_radio::timeline::start_lead_ns(&core.mode.timing(), ezsdr_kernel::stream::Direction::Tx));
            streams.lines.insert(key, Line::new(Stream { direction: ezsdr_kernel::stream::Direction::Tx, origin: 0, config: config(200), lead, call: 0, allowance: 0 }));
            streams.stream_mut(key).configs.insert(1, Arc::new(super::super::control::ColdConfig::new(Settings::default())));
            assert!(streams.book(&core, key, Item { e: core.ticks(100_000_000), seq: 1, ready: 0, release: NO_RELEASE, delivered: None, refused: true, kind: Kind::Cold(config(100)) }));
        }
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(1));
        tx.at = 1;
        tx.read(None);
        {
            let mut streams = lock(&core.streams);
            streams.stream_mut(key).configs.insert(2, Arc::new(super::super::control::ColdConfig::new(Settings::default())));
            assert!(streams.book(&core, key, Item { e: core.ticks(200_000_000), seq: 2, ready: 0, release: NO_RELEASE, delivered: None, refused: false, kind: Kind::Cold(config(100)) }));
        }
        assert!(tx.step(), "the refused enable's switch");
        assert!(tx.clock.is_none(), "a refused enable starts no clock");
        assert!(tx.step(), "the second change's switch");
        let calls = device.calls();
        assert_eq!(calls.iter().filter(|call| call.starts_with("tx_open")).count(), 2, "{calls:?}");
        assert!(tx.clock.is_some(), "the second change starts its clock");
    }

    #[test]
    fn ur_13_a_refused_comparison_is_repeated_at_the_next_switch() {
        // UR-13 (Sol re-review 1): a switch whose rate comparison is refused does not record its
        // set as open. The next switch to the same set opens the streamer again, and its
        // comparison returns the outcome stored for the direction: refused again, with no clock
        // and no configuration applied.
        let (core, device, _, _) = super::super::test_support::rig();
        assert!(core.check_rates(Dir::Tx, 99).is_err());
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        for switch_no in 0..2 {
            let clock = core.register(Dir::Tx, 200, 0).unwrap();
            tx.switch = Some(switch(0, Some(clock), 1, Settings::default()));
            tx.do_switch();
            assert!(tx.clock.is_none(), "switch {switch_no} starts no clock");
        }
        let calls = device.calls();
        assert_eq!(calls.iter().filter(|call| call.starts_with("tx_open")).count(), 2, "{calls:?}");
        assert!(calls.iter().all(|call| !call.starts_with("apply")), "{calls:?}");
    }

    #[test]
    fn ur_25_a_refused_transmit_switch_makes_no_device_call() {
        // UR-25 (S18 for transmit, the 1a rule): a refused switch makes no device call, so the
        // record of the set the streamer holds stands, and the switch back to that set opens
        // nothing.
        let (core, device, _, _) = super::super::test_support::rig();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        for (channels, refused) in [(1, false), (2, true), (1, false)] {
            let clock = core.register(Dir::Tx, 200, 0).unwrap();
            tx.switch = Some(switch(0, Some(clock), channels, Settings::default()));
            if refused {
                tx.switch.as_mut().unwrap().to.as_mut().unwrap().refused = true;
            }
            let before = device.calls().len();
            tx.do_switch();
            if refused {
                assert_eq!(device.calls().len(), before, "{:?}", device.calls());
            }
        }
        let calls = device.calls();
        assert_eq!(calls.iter().filter(|call| call.starts_with("tx_open")).count(), 1, "{calls:?}");
    }

    #[test]
    fn ur_13_a_failed_configure_by_the_owner_is_configured_again_without_reopen() {
        // UR-13 (S8): a configure of the transmit owner that fails leaves the streamer open; the
        // next switch to the same set configures it without a second open.
        let inner = Arc::new(FakeDevice::new(FakeConfig::default()));
        let device: Arc<dyn crate::device::Device> = Arc::new(super::super::test_support::ApplyFails::new(inner.clone(), Dir::Tx, None));
        let (core, _, _) = super::super::test_support::rig_on(device, vec![], 2_000);
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        for _ in 0..2 {
            let clock = core.register(Dir::Tx, 200, 0).unwrap();
            tx.switch = Some(switch(0, Some(clock), 1, Settings { rate: Some(2e6), ..Settings::default() }));
            tx.do_switch();
        }
        let calls = inner.calls();
        assert_eq!(calls.iter().filter(|call| call.starts_with("tx_open")).count(), 1, "{calls:?}");
        // The first switch's configure is the refusal: only the second one reaches the device.
        assert_eq!(calls.iter().filter(|call| call.starts_with("apply")).count(), 1, "{calls:?}");
    }

    #[test]
    fn ur_13_a_transmit_from_none_segment_the_owner_never_started_is_opened_again() {
        // UR-13 (S2): uhd-control opens and configures B = {0,1} from no stream, its owner never
        // having begun it, and a later enable from no stream opens C = {0}. The owner opens each
        // set itself before it adopts the segment: no record of an earlier open stands for the
        // set a segment runs.
        let (core, device, time, _) = super::super::test_support::rig();
        let mut control = control_of(&core);
        for (id, at, channels) in [(2, 100, 2), (3, 200, 0), (4, 300, 1)] {
            tx_channels_from_none(&mut control, &core, id, at, channels);
        }
        let control_opens: Vec<String> = device.calls().into_iter().filter(|call| call.starts_with("tx_open")).collect();
        assert_eq!(control_opens, ["tx_open 0 [0, 1]", "tx_open 0 [0]"], "{control_opens:?}");
        let planned = plan_of(&core, Dir::Tx);
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        tx.read(None);
        assert!(tx.step(), "B's switch");
        assert_eq!(tx.clock, planned[0].clock, "B is adopted after the owner's own open");
        let cut = planned[0].clock.expect("B's clock").instant(planned[0].segment.cut.expect("the empty change cuts B"));
        time.advance_to(core.at(cut)).unwrap();
        tx.read(None);
        assert!(tx.step(), "C's switch");
        assert_eq!(tx.clock, planned[1].clock, "C is adopted after the owner's own open");
        let opens: Vec<String> = device.calls().into_iter().filter(|call| call.starts_with("tx_open")).collect();
        assert_eq!(opens, ["tx_open 0 [0, 1]", "tx_open 0 [0]", "tx_open 0 [0, 1]", "tx_open 0 [0]"], "{opens:?}");
    }

    #[test]
    fn ur_25_a_refused_transmit_clock_is_not_adopted() {
        // UR-25 (S14): a refused transmit change to {0} registers its clock, but the owner does
        // not adopt it: no open, no configure, no clock. The same set, changed again without the
        // refusal, is opened and adopted.
        let (core, device, _, _) = super::super::test_support::rig();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        let refused = core.register(Dir::Tx, 200, 0).unwrap();
        tx.switch = Some(switch(0, Some(refused), 1, Settings::default()));
        tx.switch.as_mut().unwrap().to.as_mut().unwrap().refused = true;
        tx.do_switch();
        assert!(tx.clock.is_none(), "a refused clock is not adopted");
        assert!(device.calls().is_empty(), "{:?}", device.calls());
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        tx.switch = Some(switch(0, Some(clock), 1, Settings { rate: Some(2e6), ..Settings::default() }));
        tx.do_switch();
        let calls = device.calls();
        assert_eq!(calls.iter().filter(|call| call.starts_with("tx_open")).count(), 1, "{calls:?}");
        assert!(tx.clock.is_some(), "the accepted change starts its clock");
    }

    #[test]
    fn ur_13_a_switch_to_the_verified_set_opens_nothing() {
        // UR-13 (Sol re-review 3): a switch to the set the last successful switch opened and
        // compared does not open the streamer again.
        let (core, device, _, _) = super::super::test_support::rig();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        for _ in 0..2 {
            let clock = core.register(Dir::Tx, 200, 0).unwrap();
            tx.switch = Some(switch(0, Some(clock), 1, Settings::default()));
            tx.do_switch();
        }
        let calls = device.calls();
        assert_eq!(calls.iter().filter(|call| call.starts_with("tx_open")).count(), 1, "{calls:?}");
    }

    #[test]
    fn ur_22_underflow_and_loss_end_the_damaged_burst() {
        for code in [crate::device::TxCode::Underflow, crate::device::TxCode::UnderflowInPacket,
            crate::device::TxCode::SeqError, crate::device::TxCode::SeqErrorInBurst] {
            let (core, device, time, events) = super::super::test_support::rig();
            device.tx_open(0, &[0]).unwrap();
            let clock = core.register(Dir::Tx, 200, 0).unwrap();
            let (_to_tx, cmds) = std::sync::mpsc::channel();
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            let mut burst = held(&clock, 20_000, 10_000, LatePolicy::DropAndFlag);
            burst.repeat = true;
            lock(&core.held).insert((clock.domain, 20_000));
            tx.command(TxCmd::Burst(burst));
            time.advance_to(core.at(core.ticks(11_000_000))).unwrap();
            assert!(tx.step());
            lock(&core.held).insert((clock.domain, 100_000));
            tx.command(TxCmd::Burst(held(&clock, 100_000, 10_000, LatePolicy::DropAndFlag)));
            tx.report(crate::device::TxReport { code, tick: Some(clock.instant(22_000)), channel: 0 });
            assert!(tx.open.is_none() && tx.continues_at.is_none() && tx.tail.is_none());
            assert!(!lock(&core.held).contains(&(clock.domain, 20_000)));
            assert!(tx.held.contains_key(&100_000), "future bursts retain their reservations");
            let sent = device.calls();
            assert!(!tx.step());
            tx.stop_now("test stop", false);
            assert_eq!(device.calls(), sent, "no payload or second EOB after abandonment");
            assert_eq!(lock(&core.rec).bursts[0]["end"], "stop");
            assert!(events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().any(|e| e.kind == EventKind::parse(ezsdr_radio::kinds::TX_UNDERFLOW).unwrap()));
        }
    }

    #[test]
    fn sc_25_the_underflow_event_carries_its_own_instant() {
        // SC-25: the event carries the time at which the underflow occurred. The
        // Authority's now is elsewhere — 11 ms of root time, and in the root domain —
        // and the sending clock's origin is not zero, its report tick one past sample
        // 22 000's, so neither "now when the report was read" nor a tick read without
        // that origin, nor a rounding up, can pass for the tick's floor.
        let (core, device, time, events) = super::super::test_support::rig();
        device.tx_open(0, &[0]).unwrap();
        let clock = core.register(Dir::Tx, 200, 1_000_003).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        time.advance_to(core.at(core.ticks(11_000_000))).unwrap();
        let underflow = |events: &Arc<EventCollector>| {
            events
                .drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0))
                .into_iter()
                .find(|e| e.kind.as_str() == ezsdr_radio::kinds::TX_UNDERFLOW)
                .expect("TX_UNDERFLOW")
        };
        tx.report(crate::device::TxReport {
            code: crate::device::TxCode::Underflow,
            tick: Some(clock.instant(22_000) + 1),
            channel: 0,
        });
        assert_eq!(
            underflow(&events).time,
            TimePoint::new(clock.domain, 22_000),
            "the report's tick in the sending SampleClock, floored, not the Authority's now"
        );
        // A report with no tick is still timed: at the Authority's now.
        let now = core.now();
        tx.report(crate::device::TxReport {
            code: crate::device::TxCode::Underflow,
            tick: None,
            channel: 0,
        });
        assert_eq!(underflow(&events).time, core.at(now));
    }

    #[test]
    fn ur_22_an_expired_timeline_is_not_resumed_before_its_report() {
        let (core, device, time, _) = super::super::test_support::rig();
        device.tx_open(0, &[0]).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        tx.command(TxCmd::Burst(held(&clock, 20_000, 10_000, LatePolicy::DropAndFlag)));
        time.advance_to(core.at(core.ticks(11_000_000))).unwrap();
        assert!(tx.step());
        time.advance_to(core.at(core.ticks(70_000_000))).unwrap();
        assert!(tx.step());
        assert!(tx.open.is_none());
        assert_eq!(device.calls().iter().filter(|c| c.starts_with("tx_send ")).count(), 2,
            "one payload buffer and one empty EOB, no resumed payload");
    }

    #[test]
    fn ur_22_real_starvation_does_not_extend_the_stop_window() {
        // Intentionally starve the fake; the test does not require a millisecond
        // host wake-up. Synchronize the manual decision clock to the fake afterward.
        let (core, device, time, _) = super::super::test_support::rig();
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        let mut burst = held(&clock, 0, 100_000, LatePolicy::SendAsapAndFlag);
        burst.repeat = true;
        // Prime an untimed device burst to make starvation independent of a
        // host deadline for the initial timed SOB (covered by UR-21 tests).
        device.tx_send(0, &[&burst.samples[0][..2_000]], None, true, false, std::time::Duration::from_secs(1)).unwrap();
        tx.open = Some(super::Open { held: burst, next: 2_000, first: false });
        tx.remember_burst(TimePoint::new(clock.domain, 0));
        std::thread::sleep(std::time::Duration::from_millis(60));
        time.advance_to(core.at(device.time_now().unwrap().max(core.now()))).unwrap();
        tx.reports(std::time::Duration::ZERO);
        assert!(lock(&core.rec).device_async.iter().any(|r| r["code"] == "Underflow"));
        assert!(tx.open.is_none());
        for _ in 0..20 { assert!(!tx.step()); }
        let stop = device.time_now().unwrap();
        tx.stop_now("test stop", false);
        std::thread::sleep(std::time::Duration::from_millis(20));
        tx.reports(std::time::Duration::ZERO);
        let rec = lock(&core.rec);
        let end = rec.device_async.iter().find(|r| r["code"] == "BurstAck").unwrap()["tick"].as_i64().unwrap();
        assert!(end <= stop + core.ticks(10_000_000), "stop={stop}, playback end={end}");
    }

    #[test]
    fn ur_21_first_dispatch_rechecks_all_late_policies() {
        for (policy, expected) in [(LatePolicy::DropAndFlag, "drop"),
            (LatePolicy::RejectAtPlan, "plan_violation"),
            (LatePolicy::SendAsapAndFlag, "send_asap")] {
            let (core, device, time, events) = super::super::test_support::rig();
            device.tx_open(0, &[0]).unwrap();
            device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
            let clock = core.register(Dir::Tx, 200, 0).unwrap();
            let (_to_tx, cmds) = std::sync::mpsc::channel();
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            lock(&core.held).insert((clock.domain, 20_000));
            tx.command(TxCmd::Burst(held(&clock, 20_000, 10_000, policy)));
            assert!(!tx.step(), "still outside the in-flight window");
            time.advance_to(core.at(core.ticks(40_000_000))).unwrap();
            assert!(tx.step());
            let events = events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
            assert!(events.iter().any(|e| e.payload["outcome"] == expected));
            if policy == LatePolicy::SendAsapAndFlag {
                let open = tx.open.as_ref().unwrap();
                assert_eq!(open.held.k, 42_000);
                assert_eq!(open.held.open.requested_target, Some(TimePoint::new(clock.domain, 20_000)));
                assert!(matches!(open.held.open.late, Some(ezsdr_kernel::stream::LateOutcome::SendAsap { .. })));
            } else {
                assert!(tx.open.is_none());
                assert!(!device.calls().iter().any(|call| call.starts_with("tx_send ")));
            }
            assert!(!lock(&core.held).contains(&(clock.domain, 20_000)));
        }
    }

    #[test]
    fn ur_25_uhd_tx_reads_the_plan_again_when_it_changes() {
        // UR-25: a transmit change booked after uhd-tx read the plan reaches it at its next
        // read: the switch at the plan's cut, to the clock the booking registered.
        use ezsdr_radio::timeline::{Channels, Config, Item, Kind, Line, Stream, NO_RELEASE};
        let (core, device, _, _) = super::super::test_support::rig();
        device.tx_open(0, &[0]).unwrap();
        let c0 = core.register(Dir::Tx, 200, 0).unwrap();
        let config = |n: u64| Config { channels: Channels::first(1), ratio: ezsdr_kernel::time::Rational::new(n, 1).unwrap() };
        {
            let mut streams = lock(&core.streams);
            streams.stream_mut(stream_key(Dir::Tx, 0)).clocks = vec![(c0, false)];
            let lead = core.ticks(ezsdr_radio::timeline::start_lead_ns(&core.mode.timing(), ezsdr_kernel::stream::Direction::Tx));
            streams.lines.insert(stream_key(Dir::Tx, 0), Line::new(Stream { direction: ezsdr_kernel::stream::Direction::Tx, origin: 0, config: config(200), lead, call: 0, allowance: 0 }));
        }
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(c0), Channels::first(1));
        tx.read(None);
        assert!(tx.switch.is_none());
        let e = core.ticks(1_000_000_000);
        {
            let mut streams = lock(&core.streams);
            streams.stream_mut(stream_key(Dir::Tx, 0)).configs.insert(1, Arc::new(super::super::control::ColdConfig::new(Settings::default())));
            assert!(streams.book(&core, stream_key(Dir::Tx, 0), Item { e, seq: 1, ready: 0, release: NO_RELEASE, delivered: None, refused: false, kind: Kind::Cold(config(100)) }));
        }
        let c1 = lock(&core.streams).stream(stream_key(Dir::Tx, 0)).clocks[1].0;
        tx.read(None);
        let switch = tx.switch.as_ref().expect("a switch");
        assert_eq!(switch.cut, e);
        assert_eq!(switch.to.as_ref().and_then(|to| to.clock), Some(c1));
    }

    #[test]
    fn ur_23_a_refused_transmit_change_ends_its_clock_at_its_origin() {
        // UR-23, VH-2: when the owner books a transmit change as refused, the clock its booking
        // registered ends at its origin at once, with no later booking needed.
        use ezsdr_radio::timeline::{Channels, Config, Item, Kind, Line, Stream, NO_RELEASE};
        let (core, device, _, _) = super::super::test_support::rig();
        device.tx_open(0, &[0]).unwrap();
        let c0 = core.register(Dir::Tx, 200, 0).unwrap();
        let config = |n: u64| Config { channels: Channels::first(1), ratio: ezsdr_kernel::time::Rational::new(n, 1).unwrap() };
        let mut streams = lock(&core.streams);
        let key = stream_key(Dir::Tx, 0);
        streams.stream_mut(key).clocks = vec![(c0, false)];
        let lead = core.ticks(ezsdr_radio::timeline::start_lead_ns(&core.mode.timing(), ezsdr_kernel::stream::Direction::Tx));
        streams.lines.insert(key, Line::new(Stream { direction: ezsdr_kernel::stream::Direction::Tx, origin: 0, config: config(200), lead, call: 0, allowance: 0 }));
        streams.stream_mut(key).configs.insert(1, Arc::new(super::super::control::ColdConfig::new(Settings::default())));
        assert!(streams.book(&core, key, Item { e: core.ticks(1_000_000_000), seq: 1, ready: 0, release: NO_RELEASE, delivered: None, refused: false, kind: Kind::Cold(config(100)) }));
        let c1 = streams.stream(key).clocks[1].0;
        assert!(streams.refuse(&core, key, 1));
        drop(streams);
        let ended = core.clocks.sample_clock_records().into_iter().find(|r| r.domain == c1.domain).and_then(|r| r.ended_at.map(|t| t.ticks_in(r.root).unwrap()));
        assert_eq!(ended, Some(c1.origin));
    }

    #[test]
    fn ur_25_a_burst_on_a_later_clock_survives_an_earlier_switch() {
        // UR-25 (KC-21a): at a switch, a burst booked on a clock the plan starts later stays
        // booked, and is held once its clock is current; a refused switch, which uhd-tx books
        // itself, cancels only its own clock's bursts.
        use ezsdr_radio::timeline::{Channels, Config, Item, Kind, Line, Stream, NO_RELEASE};
        for refused in [false, true] {
            let faults = if refused { vec![crate::device::FakeFault::WrongRate { claimed: 2e6, applied: 2e6 - 1_000.0, nth: 0 }] } else { Vec::new() };
            let (core, device, _, _) = super::super::test_support::rig_with(FakeConfig { faults, ..FakeConfig::default() }, Vec::new());
            device.tx_open(0, &[0]).unwrap();
            let s = |n: i64| core.ticks(n * 1_000_000_000);
            let c0 = core.register(Dir::Tx, 200, 0).unwrap();
            let config = |n: u64| Config { channels: Channels::first(1), ratio: ezsdr_kernel::time::Rational::new(n, 1).unwrap() };
            {
                let mut streams = lock(&core.streams);
                let key = stream_key(Dir::Tx, 0);
                streams.stream_mut(key).clocks = vec![(c0, false)];
                let lead = core.ticks(ezsdr_radio::timeline::start_lead_ns(&core.mode.timing(), ezsdr_kernel::stream::Direction::Tx));
                streams.lines.insert(key, Line::new(Stream { direction: ezsdr_kernel::stream::Direction::Tx, origin: 0, config: config(200), lead, call: 0, allowance: 0 }));
                for (seq, e, n) in [(1, s(1), 100), (2, s(2), 200), (3, s(3), 100)] {
                    streams.stream_mut(key).configs.insert(seq, Arc::new(super::super::control::ColdConfig::new(Settings::default())));
                    assert!(streams.book(&core, key, Item { e, seq, ready: 0, release: NO_RELEASE, delivered: None, refused: false, kind: Kind::Cold(config(n)) }));
                }
            }
            let (c1, c2) = { let streams = lock(&core.streams); let key = stream_key(Dir::Tx, 0); (streams.stream(key).clocks[1].0, streams.stream(key).clocks[2].0) };
            let (_to_tx, cmds) = std::sync::mpsc::channel();
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(c0), Channels::first(1));
            tx.read(None);
            for clock in [c1, c2] {
                lock(&core.held).insert((clock.domain, 100));
                tx.command(TxCmd::Burst(held(&clock, 100, 10, LatePolicy::SendAsapAndFlag)));
            }
            tx.do_switch();
            let case = format!("refused {refused}");
            assert_eq!(tx.clock, (!refused).then_some(c1), "{case}");
            assert_eq!(tx.later.iter().map(|h| h.domain).collect::<Vec<_>>(), [c2.domain], "{case}");
            assert_eq!(tx.held.len(), usize::from(!refused), "{case}");
            assert_eq!(lock(&core.rec).rejected.iter().filter(|r| r["kind"] == "tx_burst").count(), usize::from(refused), "{case}");
            let booked = lock(&core.streams).line(stream_key(Dir::Tx, 0)).unwrap().items.iter().filter(|item| item.refused).map(|item| item.seq).collect::<Vec<_>>();
            assert_eq!(booked, if refused { vec![1] } else { Vec::new() }, "{case}");
            tx.held.clear();
            tx.do_switch();
            assert_eq!(tx.clock, Some(c2), "{case}");
            assert!(tx.later.is_empty(), "{case}");
            assert_eq!(tx.held.len(), 1, "{case}");
            // #63: switching from the second change's clock prunes the first change.
            tx.held.clear();
            tx.do_switch();
            let items: Vec<u64> = lock(&core.streams).line(stream_key(Dir::Tx, 0)).unwrap().items.iter().map(|item| item.seq).collect();
            assert_eq!(items, [2, 3], "{case}");
        }
    }

    #[test]
    fn ur_21_first_dispatch_cannot_move_past_a_cold_switch() {
        let (core, device, time, _) = super::super::test_support::rig();
        device.tx_open(0, &[0]).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        lock(&core.held).insert((clock.domain, 17_000));
        tx.command(TxCmd::Burst(held(&clock, 17_000, 10_000, LatePolicy::SendAsapAndFlag)));
        tx.switch = Some(switch(core.ticks(20_000_000), None, 0, Settings::default()));
        time.advance_to(core.at(core.ticks(19_000_000))).unwrap();
        assert!(tx.step());
        assert!(tx.open.is_none());
        assert!(!tx.step());
        time.advance_to(core.at(core.ticks(20_000_000))).unwrap();
        assert!(tx.step());
        assert!(lock(&core.held).is_empty());
        assert!(!device.calls().iter().any(|c| c.starts_with("tx_send ")));
    }

    #[test]
    fn ur_21_first_dispatch_preserves_an_on_time_start() {
        let (core, device, time, events) = super::super::test_support::rig();
        device.tx_open(0, &[0]).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        lock(&core.held).insert((clock.domain, 20_000));
        tx.command(TxCmd::Burst(held(&clock, 20_000, 10_000, LatePolicy::DropAndFlag)));
        time.advance_to(core.at(core.ticks(18_000_000))).unwrap();
        assert!(tx.step());
        assert_eq!(tx.open.as_ref().unwrap().held.k, 20_000);
        assert!(tx.open.as_ref().unwrap().held.open.late.is_none());
        assert!(events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).is_empty());
        assert!(device.calls().iter().any(|c| c.contains("at=4000000 sob=true")));
    }

    #[test]
    fn ur_21_first_dispatch_refuses_a_moved_start_collision() {
        let (core, device, time, events) = super::super::test_support::rig();
        device.tx_open(0, &[0]).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        for k in [20_000, 42_000] {
            lock(&core.held).insert((clock.domain, k));
            tx.command(TxCmd::Burst(held(&clock, k, 1_000, LatePolicy::SendAsapAndFlag)));
        }
        time.advance_to(core.at(core.ticks(40_000_000))).unwrap();
        assert!(tx.step());
        assert!(tx.open.is_none());
        assert!(tx.held.contains_key(&42_000));
        assert!(events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().any(|e| e.payload["outcome"] == "refused"));
        assert!(!device.calls().iter().any(|call| call.starts_with("tx_send ")));
    }

    #[test]
    fn ur_21_a_moved_start_on_a_held_burst_s_is_refused() {
        // RM-15 as VE-2 amends it: B, late against the open burst A's next sample, moves
        // there under `send_asap`; C is already held at that sample, so B is refused
        // (`TIME_ERROR { refused }`, `COMMAND_REJECTED`) and C keeps its start.
        let device = Arc::new(FakeDevice::new(FakeConfig::default()));
        let clocks = Arc::new(ClockRegistry::new());
        let authority = DeviceAuthority::new(device.clone(), clocks.clone(), "internal", "internal", "fake").unwrap();
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let mut kinds = EventKindRegistry::with_kernel_kinds();
        ezsdr_radio::register(&mut ezsdr_kernel::module_api::ModuleRegistry::new(), &mut ezsdr_kernel::binding::AdmissionCheckRegistry::new(), &mut kinds).unwrap();
        let usrp = ResourceId::parse("usrp").unwrap();
        let pairs: Vec<_> = ["usrp", "usrp/rx", "usrp/tx"].iter().flat_map(|s| kinds.kinds().into_iter().map(move |k| (ezsdr_kernel::event::EventSource::Node { node: ResourceId::parse(s).unwrap() }, k))).collect();
        let events = Arc::new(EventCollector::new(&pairs, &kinds.kinds(), 4096, &Policy::default()));
        let inputs: Arc<BTreeMap<ContentHash, Arc<[u8]>>> = Arc::new(BTreeMap::new());
        let core = Arc::new(Core::new(
            device.clone(),
            usrp,
            authority.root(),
            authority.time(),
            clocks,
            events.clone(),
            inputs,
            Vec::new(),
            super::super::mode::Mode::Current(Box::new(crate::profile::ubx_description(2_000))),
        )
        .expect("the rx node is inside the instance root"));
        let origin = (core.now() / 200 + 1) * 200;
        let clock = core.register(Dir::Tx, 200, origin).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        // A, 30 ms, 20 ms ahead; stepped until it has handed over a few buffers.
        let a = clock.at_or_after(core.now() + core.ticks(20_000_000));
        tx.command(TxCmd::Burst(held(&clock, a, 30_000, LatePolicy::DropAndFlag)));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while tx.open.as_ref().is_none_or(|open| open.next < a + 4_000) {
            assert!(std::time::Instant::now() < deadline, "A never got 4 000 samples out");
            if !tx.step() {
                std::thread::sleep(std::time::Duration::from_micros(200));
            }
        }
        let next = tx.open.as_ref().unwrap().next;
        // C at A's next sample, B before it: both queued before the step that decides B.
        tx.command(TxCmd::Burst(held(&clock, next, 1_000, LatePolicy::DropAndFlag)));
        tx.command(TxCmd::Burst(held(&clock, next - 500, 1_000, LatePolicy::SendAsapAndFlag)));
        lock(&core.held).insert((clock.domain, next));
        lock(&core.held).insert((clock.domain, next - 500));
        assert!(tx.step());
        assert!(tx.held.contains_key(&next), "C keeps its start");
        assert_eq!(tx.held.len(), 1, "B is not held: {:?}", tx.held.keys().collect::<Vec<_>>());
        let reasons: Vec<String> = lock(&core.rec).rejected.iter().filter_map(|r| r["reason"].as_str().map(str::to_owned)).collect();
        assert!(reasons.iter().any(|r| r == "UR-21: the moved start is a held burst's"), "{reasons:?}");
        let refused = events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().any(|e| {
            e.kind == EventKind::parse(ezsdr_radio::kinds::TIME_ERROR).unwrap() && e.payload["outcome"] == "refused"
        });
        assert!(refused, "no TIME_ERROR {{ refused }}");
    }
    #[test]
    fn ur_29_eob_device_errors_are_escalated() {
        for lost in [true, false] {
            for tail in [None, Some(vec![vec![[0.25, 0.0]]])] {
                let (core, device, _, events) = super::super::test_support::rig();
                device.tx_open(0, &[0]).unwrap();
                let clock = core.register(Dir::Tx, 200, 0).unwrap();
                let (_to_tx, cmds) = std::sync::mpsc::channel();
                let tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
                *device.eob_error.lock().unwrap() = Some(crate::device::DeviceError {
                    lost, message: "controlled EOB failure".to_owned(),
                });
                tx.close_device_burst(tail);
                assert_eq!(core.is_lost(), lost, "EOB failure must reach UR-29");
                let emitted = events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
                let expected = if lost { EventKind::DEVICE_LOST } else { ezsdr_radio::kinds::COMMAND_REJECTED };
                assert_eq!(emitted.iter().filter(|event| event.kind.as_str() == expected).count(), 1);
                if lost {
                    tx.close_device_burst(None);
                    assert!(events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).is_empty());
                    assert_eq!(device.calls().iter().filter(|call| call.as_str() == "mark_lost").count(), 1);
                }
            }
        }
    }

    #[test]
    fn ur_29_eob_loss_during_switch_stops_device_calls() {
        let (core, device, _, _) = super::super::test_support::rig();
        device.tx_open(0, &[0]).unwrap();
        let old = core.register(Dir::Tx, 200, 0).unwrap();
        core.clocks.end(old.domain, core.at(20_000_000)).unwrap();
        let new = core.register(Dir::Tx, 100, 30_000_000).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(old), Channels::first(1));
        tx.continues_at = Some(100);
        tx.tail = Some(vec![vec![[0.25, 0.0]]]);
        tx.switch = Some(switch(20_000_000, Some(new), 2, Settings { rate: Some(2e6), ..Settings::default() }));
        *device.eob_error.lock().unwrap() = Some(crate::device::DeviceError {
            lost: true, message: "controlled EOB loss during switch".to_owned(),
        });
        let before = device.calls().len();
        tx.do_switch();
        tx.reports(std::time::Duration::ZERO);
        assert!(core.is_lost());
        assert_eq!(&device.calls()[before..], &["mark_lost"]);
        assert!(tx.clock.is_none());
    }

    #[test]
    fn ur_29_eob_loss_from_report_prevents_next_burst_send() {
        let (core, device, _, _) = super::super::test_support::rig();
        device.tx_open(0, &[0]).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        tx.continues_at = Some(100);
        tx.held.insert(3000, held(&clock, 3000, 10, LatePolicy::DropAndFlag));
        *device.eob_error.lock().unwrap() = Some(crate::device::DeviceError {
            lost: true, message: "controlled EOB loss after underflow".to_owned(),
        });
        let before = device.calls().len();
        tx.report(crate::device::TxReport { code: crate::device::TxCode::Underflow, tick: None, channel: 0 });
        assert!(core.is_lost());
        assert!(!tx.step());
        assert!(tx.held.contains_key(&3000));
        assert_eq!(&device.calls()[before..], &["mark_lost"]);
    }

    #[test]
    fn ur_25_largest_aligned_cut_is_safe_in_tx_owner() {
        let (core, _, _, _) = super::super::test_support::rig();
        let old = core.register(Dir::Tx, 200, 0).unwrap();
        let cut = i64::MAX.div_euclid(old.n) * old.n;
        assert_eq!(old.at_or_after(cut), cut / old.n);
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core, 0, cmds, Some(old), Channels::first(1));
        tx.switch = Some(switch(cut, None, 0, Settings::default()));
        assert!(!tx.step());
        assert!(tx.switch.is_some());
    }

#[test]
    fn ur_28_per_channel_ack_preserves_the_next_burst() {
        let (core, _, _, events) = super::super::test_support::rig();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core, 0, cmds, Some(clock), Channels::first(2));
        let a = TimePoint::new(clock.domain, 10_000);
        let b = TimePoint::new(clock.domain, 20_000);
        tx.remember_burst(a); tx.remember_burst(b);
        for channel in [0,1] {
            tx.report(crate::device::TxReport { code: crate::device::TxCode::BurstAck, tick: Some(clock.instant(a.ticks_in(clock.domain).unwrap())), channel });
        }
        tx.report(crate::device::TxReport { code: crate::device::TxCode::TimeError, tick: Some(clock.instant(b.ticks_in(clock.domain).unwrap())+200), channel: 0 });
        let emitted = events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        eprintln!("events={emitted:?}");
        assert!(emitted.iter().any(|e|e.kind.as_str()==ezsdr_radio::kinds::TIME_ERROR), "burst B TIME_ERROR must survive both ACKs of A");
    }

    #[test]
    fn ur_28_channels_retire_in_independent_order() {
        use crate::device::{TxCode, TxReport};
        for channels in [1, 2] {
            let (core, _, _, events) = super::super::test_support::rig();
            let clock = core.register(Dir::Tx, 200, 0).unwrap();
            let (_to_tx, cmds) = std::sync::mpsc::channel();
            let mut tx = Tx::new(core, 0, cmds, Some(clock), Channels::first(channels as u32));
            let a = TimePoint::new(clock.domain, 10_000);
            let b = TimePoint::new(clock.domain, 20_000);
            tx.remember_burst(a); tx.remember_burst(b);
            tx.report(TxReport { code: TxCode::BurstAck, tick: None, channel: 9 });
            assert_eq!(tx.unacked.len(), 2, "invalid channel must not consume a burst");
            if channels == 2 {
                // Channel 0 may finish B while channel 1 has not finished A.
                for _ in 0..2 { tx.report(TxReport { code: TxCode::BurstAck, tick: None, channel: 0 }); }
                assert_eq!(tx.unacked.len(), 2);
            }
            let channel = channels - 1;
            for _ in 0..2 { tx.report(TxReport { code: TxCode::TimeError, tick: None, channel }); }
            let errors = events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
            assert_eq!(errors.len(), 2);
            assert_eq!(errors[0].payload["target"], serde_json::to_value(a).unwrap());
            assert_eq!(errors[1].payload["target"], serde_json::to_value(b).unwrap());
            assert!(errors.iter().all(|e| e.payload["late_by_ns"] == 0));
            assert!(tx.unacked.is_empty());
        }
    }

    #[test]
    fn ur_28_a_from_none_open_replaces_the_report_queue() {
        // UR-28, UR-36 (S3): uhd-control's enable from no stream opens the transmit streamer
        // again, which replaces its report queue, so the correlations of the burst still
        // awaiting its acknowledgement are dropped: the next burst's own reports name it.
        let (core, device, time, events) = super::super::test_support::rig();
        let mut control = control_of(&core);
        tx_channels_from_none(&mut control, &core, 2, 100, 1);
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        tx.read(None);
        assert!(tx.step(), "the enable's switch");
        let first = tx.clock.expect("the enable starts a clock");
        tx.remember_burst(TimePoint::new(first.domain, 100));
        // The disable ends the segment's clock; the owner is idle then, and the next enable's
        // open replaces the streamer the first burst's acknowledgement was awaited on.
        tx_channels_from_none(&mut control, &core, 3, 200, 0);
        let cut = plan_of(&core, Dir::Tx)[0].segment.cut.expect("the empty change cuts the segment");
        time.advance_to(core.at(first.instant(cut))).unwrap();
        tx.read(None);
        assert!(tx.step(), "the disable's switch");
        assert!(tx.clock.is_none(), "the disable leaves no clock");
        tx_channels_from_none(&mut control, &core, 4, 300, 1);
        tx.read(None);
        assert!(tx.step(), "the next enable's switch");
        let second = tx.clock.expect("the next enable starts its clock");
        assert_ne!(first.domain, second.domain, "each segment has its own clock");
        assert!(tx.unacked.is_empty(), "the replaced streamer's correlations stand: {:?}", device.calls());
        // The new burst's own reports name it, on its own clock.
        let target = TimePoint::new(second.domain, 100);
        tx.remember_burst(target);
        tx.report(crate::device::TxReport { code: crate::device::TxCode::TimeError, tick: Some(second.instant(100) + 200), channel: 0 });
        let errors: Vec<_> = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
            .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert_eq!(errors.len(), 1, "the report named no burst of the new streamer: {errors:?}");
        assert_eq!(errors[0].payload["target"], serde_json::to_value(target).unwrap());
        assert_eq!(errors[0].payload["late_by_ns"], 1000);
        // Its acknowledgement retires its own correlation, and a report after it names nothing.
        tx.report(crate::device::TxReport { code: crate::device::TxCode::BurstAck, tick: None, channel: 0 });
        assert!(tx.unacked.is_empty(), "the acknowledgement retired nothing: {:?}", device.calls());
        tx.report(crate::device::TxReport { code: crate::device::TxCode::TimeError, tick: None, channel: 0 });
        let errors: Vec<_> = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
            .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert!(errors.is_empty(), "a report with no pending burst named one: {errors:?}");
    }

    /// The rig on a `ReportFeed`, and its transmit owner after a burst: the enable from no stream
    /// at 100 ms has sent one burst, awaiting its acknowledgement, and the disable at 200 ms has
    /// ended the clock. uhd-control's next enable from no stream (300 ms) then opens the streamer
    /// again, before the owner has read that enable (UR-28, UR-36).
    fn replaced_queue() -> (Arc<Core>, Arc<ReportFeed>, Arc<EventCollector>, Tx, Clock) {
        let device = Arc::new(FakeDevice::new(FakeConfig::default()));
        let feed = Arc::new(ReportFeed::new(device));
        let (core, time, events) = super::super::test_support::rig_on(feed.clone(), Vec::new(), 2_000);
        let mut control = control_of(&core);
        tx_channels_from_none(&mut control, &core, 2, 100, 1);
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        tx.read(None);
        assert!(tx.step(), "the enable's switch");
        let first = tx.clock.expect("the enable starts a clock");
        tx.remember_burst(TimePoint::new(first.domain, 100));
        tx_channels_from_none(&mut control, &core, 3, 200, 0);
        let cut = plan_of(&core, Dir::Tx)[0].segment.cut.expect("the empty change cuts the segment");
        time.advance_to(core.at(first.instant(cut))).unwrap();
        tx.read(None);
        assert!(tx.step(), "the disable's switch");
        assert!(tx.clock.is_none(), "the disable leaves no clock");
        tx_channels_from_none(&mut control, &core, 4, 300, 1);
        (core, feed, events, tx, first)
    }

    /// The TIME_ERROR events the rig has emitted since the last call.
    fn time_errors(events: &EventCollector) -> Vec<ezsdr_kernel::event::Event> {
        events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
            .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect()
    }

    #[test]
    fn ur_28_a_poll_after_a_from_none_open_names_no_burst_of_the_replaced_queue() {
        // UR-28, UR-36 (S3, poll): the report of the replacing streamer, read by the owner's
        // poll before it adopts the enable, names no burst: the replaced queue's burst is
        // dropped with its correlations, not consumed.
        let (_core, feed, events, mut tx, _) = replaced_queue();
        feed.give(TxReport { code: TxCode::TimeError, tick: None, channel: 0 });
        tx.reports(super::Wall::ZERO);
        assert!(tx.unacked.is_empty(), "the replaced queue's correlation stands");
        assert!(time_errors(&events).is_empty(), "the poll consumed the replaced burst");
    }

    #[test]
    fn ur_28_a_shutdown_after_a_from_none_open_names_no_burst_of_the_replaced_queue() {
        // UR-28, UR-36 (S3, shutdown): the shutdown's read of the acknowledgement window is
        // correlated against the replacing streamer, as the owner's poll is.
        let (_core, feed, events, tx, _) = replaced_queue();
        feed.give(TxReport { code: TxCode::TimeError, tick: None, channel: 0 });
        tx.shutdown();
        assert!(time_errors(&events).is_empty(), "the shutdown consumed the replaced burst");
    }

    #[test]
    fn ur_28_a_streamer_the_record_holds_keeps_its_correlations() {
        // UR-28, UR-36: the correlations of the bursts in flight belong to the streamer of their
        // generation. The owner's own open for another set replaces the queue and drops them; a
        // switch the record holds, which opens nothing, keeps them.
        let (core, device, _, _) = super::super::test_support::rig();
        device.tx_open(0, &[0]).unwrap();
        lock(&core.streams).record_open(stream_key(Dir::Tx, 0), Some(Channels::first(1)));
        let old = core.register(Dir::Tx, 200, 0).unwrap();
        core.clocks.end(old.domain, core.at(20_000_000)).unwrap();
        let new = core.register(Dir::Tx, 100, 30_000_000).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core, 0, cmds, Some(old), Channels::first(1));
        tx.remember_burst(TimePoint::new(old.domain, 100));
        // Another set: only the owner can open it, and its reopen replaces the report queue.
        tx.switch = Some(switch(20_000_000, Some(new), 2, Settings { rate: Some(2e6), ..Settings::default() }));
        tx.do_switch();
        assert!(tx.unacked.is_empty());
        tx.remember_burst(TimePoint::new(new.domain, 100));
        tx.clock = None;
        // The same switch again: the record holds the set and the segment's configuration, so
        // the streamer is not opened again and the burst's correlation stands.
        tx.switch = Some(switch(i64::MIN, Some(new), 2, Settings { rate: Some(2e6), ..Settings::default() }));
        tx.do_switch();
        assert_eq!(tx.unacked.len(), 1, "{:?}", device.calls());
    }

    #[test]
    fn ur_28_delayed_time_error_uses_its_original_clock() {
        for next_n in [Some(100), Some(200), None] {
            let (core, device, _, events) = super::super::test_support::rig();
            device.tx_open(0, &[0, 1]).unwrap();
            lock(&core.streams).record_open(stream_key(Dir::Tx, 0), Some(Channels::first(2)));
            let old = core.register(Dir::Tx, 200, 0).unwrap();
            let target = TimePoint::new(old.domain, 10_000);
            core.clocks.end(old.domain, core.at(20_000_000)).unwrap();
            let new = next_n.map(|n| core.register(Dir::Tx, n, 30_000_000).unwrap());
            let (_to_tx, cmds) = std::sync::mpsc::channel();
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(old), Channels::first(2));
            tx.remember_burst(target);
            tx.switch = Some(switch(20_000_000, new, if new.is_some() { 2 } else { 0 },
                Settings { rate: Some(if next_n == Some(100) { 2e6 } else { 1e6 }), ..Settings::default() }));
            tx.do_switch();
            let next_target = new.map(|clock| TimePoint::new(clock.domain, 20_000));
            if let Some(target) = next_target { tx.remember_burst(target); }
            tx.report(crate::device::TxReport { code: crate::device::TxCode::TimeError,
                tick: Some(old.instant(target.ticks_in(old.domain).unwrap()) + 200), channel: 0 });
            // The other channel's old error still has the old target after channel 0 advanced.
            tx.report(crate::device::TxReport { code: crate::device::TxCode::TimeError,
                tick: None, channel: 1 });
            if let (Some(clock), Some(target)) = (new, next_target) {
                tx.report(crate::device::TxReport { code: crate::device::TxCode::TimeError,
                    tick: Some(clock.instant(target.ticks_in(clock.domain).unwrap()) + 400), channel: 0 });
            }
            let errors: Vec<_> = events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
                .filter(|e| e.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
            assert_eq!(errors[0].payload["late_by_ns"], 1000);
            assert_eq!(errors[0].payload["target"], serde_json::to_value(target).unwrap());
            assert_eq!(errors[1].payload["late_by_ns"], 0);
            assert_eq!(errors[1].payload["target"], serde_json::to_value(target).unwrap());
            if let Some(target) = next_target {
                assert_eq!(errors[2].payload["late_by_ns"], 2000);
                assert_eq!(errors[2].payload["target"], serde_json::to_value(target).unwrap());
            }
        }
    }

    #[test]
    fn ur_25_a_refusal_leaves_the_receive_stream_alone() {
        // RM-25 (design 33 D15): a transmit configuration uhd-tx's device refuses halts the
        // transmit stream alone: the receive stream's items, plan and clocks are untouched.
        use ezsdr_radio::timeline::{Channels, Config, Item, Kind, Line, Stream, NO_RELEASE};
        for refused in [false, true] {
            let faults = if refused { vec![crate::device::FakeFault::WrongRate { claimed: 2e6, applied: 2e6 - 1_000.0, nth: 0 }] } else { Vec::new() };
            let (core, device, _, _) = super::super::test_support::rig_with(FakeConfig { faults, ..FakeConfig::default() }, Vec::new());
            device.tx_open(0, &[0]).unwrap();
            let s = |n: i64| core.ticks(n * 1_000_000_000);
            let c0 = core.register(Dir::Tx, 200, 0).unwrap();
            let config = |n: u64| Config { channels: Channels::first(1), ratio: ezsdr_kernel::time::Rational::new(n, 1).unwrap() };
            let (tx_key, rx_key) = (stream_key(Dir::Tx, 0), stream_key(Dir::Rx, 0));
            {
                let mut streams = lock(&core.streams);
                streams.stream_mut(tx_key).clocks = vec![(c0, false)];
                // The transmit change, and the same shapes on the receive stream's line, so
                // that a refusal booked on the wrong one would show.
                for key in [tx_key, rx_key] {
                    let direction = key.0;
                    let lead = core.ticks(ezsdr_radio::timeline::start_lead_ns(&core.mode.timing(), direction));
                    streams.lines.insert(key, Line::new(Stream { direction, origin: 0, config: config(200), lead, call: 0, allowance: 0 }));
                    streams.stream_mut(key).configs.insert(1, Arc::new(super::super::control::ColdConfig::new(Settings::default())));
                    let item = Item { e: s(1), seq: 1, ready: 0, release: NO_RELEASE, delivered: None, refused: false, kind: Kind::Cold(config(100)) };
                    assert!(streams.book(&core, key, item), "{key:?}");
                }
            }
            let snapshot = || {
                let streams = lock(&core.streams);
                let items: Vec<(i64, u64, bool)> = streams.line(rx_key).unwrap().items.iter().map(|item| (item.e, item.seq, item.refused)).collect();
                let planned: Vec<(i64, Option<i64>)> = streams.planned(rx_key).into_iter().map(|planned| (planned.segment.origin, planned.segment.cut)).collect();
                (items, planned)
            };
            let before = snapshot();
            let (_to_tx, cmds) = std::sync::mpsc::channel();
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(c0), Channels::first(1));
            tx.read(None);
            tx.do_switch();
            let case = format!("refused {refused}");
            let booked = lock(&core.streams).line(tx_key).unwrap().items.iter().filter(|item| item.refused).map(|item| item.seq).collect::<Vec<_>>();
            assert_eq!(booked, if refused { vec![1] } else { Vec::new() }, "{case}");
            assert_eq!(tx.clock.is_some(), !refused, "{case}");
            assert_eq!(snapshot(), before, "{case}: the transmit refusal changed the receive stream");
        }
    }

    #[test]
    fn ur_25_the_line_lock_is_free_inside_every_transmit_device_call() {
        // UR-25 (design 33 D15): uhd-tx holds no line lock across its device calls — a
        // continuation's held sample, its own payload, the end of burst that closes the
        // device burst, a switched streamer's reopen, its configuration and the
        // asynchronous report read — and the spy proves the sequence passed each one.
        let spy = Arc::new(crate::provider::test_support::LockSpy::new(FakeConfig { clock: crate::ClockMode::Manual, ..FakeConfig::default() }));
        let (core, time, _) = crate::provider::test_support::rig_on(spy.clone(), Vec::new(), 2_000);
        spy.watch(&core);
        spy.tx_open(0, &[0]).unwrap();
        let c0 = core.register(Dir::Tx, 200, 0).unwrap();
        let c1 = core.register(Dir::Tx, 100, 1_000_000).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(c0), Channels::first(1));
        // A burst continuing the device burst of the one before: its held sample goes first,
        // then its own payload; its last sample is held for the end of burst.
        tx.continues_at = Some(20_020);
        tx.tail = Some(vec![vec![[0.25, 0.0]]]);
        tx.command(TxCmd::Burst(held(&c0, 20_020, 10, LatePolicy::DropAndFlag)));
        time.advance_to(core.at(3_600_000)).unwrap();
        assert!(tx.step());
        // A switch to two channels at another clock: ends the open device burst (with its held
        // sample), reopens the streamer and configures the direction. The reports are read
        // after it, so that the held sample's end of burst is the switch's.
        tx.command(TxCmd::Burst(held(&c0, 20_100, 10, LatePolicy::DropAndFlag)));
        tx.switch = Some(switch(20_000_000, Some(c1), 2, Settings { rate: Some(2e6), ..Settings::default() }));
        tx.do_switch();
        tx.reports(std::time::Duration::ZERO);
        // The test's own `tx_open` is one of the two: the switch reopens the streamer for
        // its two channels, and configures both.
        assert_eq!(spy.seen("tx_open"), 2, "the streamer's open and its reopen");
        assert_eq!(spy.seen("apply"), 2, "the switch configured two channels");
        assert!(spy.seen("tx_send") >= 3, "the tail, the payload and the end of burst: {}", spy.seen("tx_send"));
        assert!(spy.seen("tx_async") > 0, "the spy never saw tx_async");
    }

    #[test]
    fn ur_28_a_report_is_correlated_by_its_position_in_the_streamer() {
        // UR-28 (Sol 1): `TxReport.channel` is the position in the streamer, not the device
        // channel (UHD 4.10's `rfnoc_tx_streamer.cpp:152-159,312-319`; `FakeDevice`,
        // `device.rs:1515-1516,1555-1556`). Stream 0 opened for `{1}` — device channel 1 —
        // is position 0 there: its burst's ACK retires its pending report, and a later
        // TIME_ERROR names that burst rather than the first one.
        let (core, device, _, events) = super::super::test_support::rig();
        device.tx_open(0, &[1]).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(0).with(1));
        let first = TimePoint::new(clock.domain, 20_000);
        let second = TimePoint::new(clock.domain, 40_000);
        // What each burst's start-of-burst books.
        tx.remember_burst(first);
        tx.remember_burst(second);
        assert_eq!(tx.unacked.len(), 2);
        assert_eq!(tx.unacked[0].channels.iter().copied().collect::<Vec<_>>(), [0], "the position, not the device channel");
        // The first burst's ACK (position 0) retires the first burst alone; a TIME_ERROR for
        // the second then names the second, not the first.
        tx.report(crate::device::TxReport { code: crate::device::TxCode::BurstAck, tick: Some(clock.instant(30_000)), channel: 0 });
        assert_eq!(tx.unacked.len(), 1, "the ACK retired no report");
        assert_eq!(tx.unacked[0].target, second);
        tx.report(crate::device::TxReport { code: crate::device::TxCode::TimeError, tick: Some(clock.instant(41_000)), channel: 0 });
        assert!(tx.unacked.is_empty(), "the TIME_ERROR retired no report");
        let late: Vec<_> = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert_eq!(late.len(), 1, "{late:?}");
        assert_eq!(late[0].payload["target"], serde_json::to_value(second).unwrap(), "the report names the second burst");
        assert_eq!(late[0].payload["outcome"], "late_at_device");
    }

    // ------------------------------------------------------------ 1b-4d-1b: ownership, reading, stop

    use crate::provider::test_support::LockSpy;
    use crate::ClockMode;

    /// The manual-clock `LockSpy` rig the ownership series runs on (UR-25).
    fn spy_rig() -> (Arc<Core>, Arc<LockSpy>) {
        let spy = Arc::new(LockSpy::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() }));
        let (core, _time, _) = crate::provider::test_support::rig_on(spy.clone(), Vec::new(), 2_000);
        (core, spy)
    }

    /// Books an enable of the transmit stream's `channels` at `ms` from no stream, its owner
    /// idle, through the Action path.
    fn tx_enable_from_none(control: &mut super::super::control::Control, core: &Core, id: u64, ms: i64, channels: i64) {
        lock(&core.streams).stream_mut(stream_key(Dir::Tx, 0)).idle = true;
        control.book(Dispatched::new(ActionId(id), Action::UpdateParameter {
            target: super::super::test_support::target_of(&core.id),
            key: crate::provider::core::key("radio.tx.channels"),
            value: Value::from(channels),
            class: UpdateClass::Cold,
            at: Some(AbsoluteDeadline::new(core.at(core.ticks(ms * 1_000_000)))),
        }, Some(core.id.clone())));
    }

    /// How many `cold` changes the transmit line holds (RM-25): an enable that books nothing
    /// leaves none, whatever the plan shows once an end has taken effect.
    fn tx_colds(core: &Core) -> usize {
        lock(&core.streams).line(stream_key(Dir::Tx, 0)).map_or(0, |line| line.items.iter().filter(|item| matches!(item.kind, ezsdr_radio::timeline::Kind::Cold(_))).count())
    }

    /// The manual-clock rig a test that reads the device's own settings runs on.
    fn manual_rig() -> (Arc<Core>, Arc<FakeDevice>, Arc<ezsdr_kernel::time::ManualTimeAuthority>) {
        let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
        let (core, device, time, _) = crate::provider::test_support::rig_with(config, Vec::new());
        (core, device, time)
    }

    #[test]
    fn ur_24_a_transmit_release_without_a_live_channel_reaches_the_pending_segment() {
        // UR-24 (S1, transmit): a gain due before the enable's segment is given to the device as
        // its own timed command on that segment's channel, and the owner's switch configures
        // nothing; the device holds the gain from its instant.
        let (core, device, time) = manual_rig();
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        let mut control = control_of(&core);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        tx_channels_from_none(&mut control, &core, 2, 100, 1);
        let origin = plan_of(&core, Dir::Tx)[0].segment.origin;
        let (to_tx, cmds) = std::sync::mpsc::channel();
        std::mem::forget(to_tx);
        let mut tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        assert!(tx.read(None), "the owner reads the plan");
        let switched = device.calls().len();
        assert!(tx.step(), "the owner begins the enable's segment");
        assert!(!device.calls()[switched..].iter().any(|call| call.starts_with("apply") || call.starts_with("tx_open")), "the owner configured a segment the enable did: {:?}", device.calls());
        let gain = crate::provider::core::key("radio.tx.gain_db");
        let mark = device.calls().len();
        control.book(Dispatched::new(ActionId(3), Action::UpdateParameter {
            target: super::super::test_support::target_of(&core.id),
            key: gain.clone(),
            value: Value::num(3.0).unwrap(),
            class: UpdateClass::HardwareTimed,
            at: Some(AbsoluteDeadline::new(core.at(ms(50)))),
        }, Some(core.id.clone())));
        time.advance_to(core.at(ms(47))).unwrap();
        control.release();
        let timed: Vec<String> = device.calls()[mark..].iter().filter(|call| call.starts_with("apply tx 0") && !call.ends_with("at=-")).cloned().collect();
        assert_eq!(timed.len(), 1, "{:?}", device.calls());
        assert!(timed[0].contains("gain=3") && timed[0].contains(&format!("at={} effective={}", ms(50), ms(50))), "{timed:?}");
        device_at(&core, &time, &device, origin + core.ticks(1_000_000));
        assert_eq!(device.read_back(Dir::Tx, 0).gain, Some(3.0));
    }

    /// The transmit record, as the ownership tests read it.
    fn tx_record(core: &Core) -> (bool, bool, Option<u64>) {
        let streams = lock(&core.streams);
        let state = streams.stream(stream_key(Dir::Tx, 0));
        (state.busy, state.reading, state.configured)
    }

    #[test]
    fn ur_25_a_transmit_owner_waiting_behind_a_from_none_open_starts_nothing() {
        // UR-25 (S9, transmit): while uhd-control's enable from no stream owns the streamer,
        // uhd-tx reads no plan, moves no cursor and begins no switch; the first turn after the
        // booking opens B's set again: the record holds C's.
        let (core, spy) = spy_rig();
        let mut control = control_of(&core);
        tx_channels_from_none(&mut control, &core, 2, 100, 2);
        tx_channels_from_none(&mut control, &core, 3, 200, 0);
        let gate = spy.stop_before("tx_open");
        let host = core.device.host();
        let core_ = core.clone();
        let (done, finished) = std::sync::mpsc::channel();
        let control_thread = host.spawn("control", move || {
            let mut control = control;
            tx_enable_from_none(&mut control, &core_, 4, 300, 1);
            let _ = done.send(());
        }).expect("a participant thread");
        gate.wait();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        let mark = spy.log().len();
        let idle = || lock(&core.streams).stream(stream_key(Dir::Tx, 0)).idle;
        let before = idle();
        assert!(!tx.read(None), "uhd-tx read the plan while uhd-control owns the streamer");
        assert_eq!((tx.at, tx.switch.is_some(), idle()), (0, false, before), "the owner's cursor, switch or idle mark moved");
        assert!(!tx.step(), "uhd-tx began a switch while uhd-control owns the streamer");
        assert_eq!(spy.log().len(), mark, "uhd-tx called the device: {:?}", spy.log());
        gate.release();
        assert!(finished.recv_timeout(std::time::Duration::from_secs(10)).is_ok(), "the enable did not book");
        control_thread.join().expect("the control thread");
        let mark = spy.log().len();
        assert!(tx.read(None));
        assert!(tx.step(), "B's switch");
        let opens: Vec<String> = spy.log()[mark..].iter().filter(|call| call.starts_with("tx_open")).cloned().collect();
        assert_eq!(opens.first().map(String::as_str), Some("tx_open 0 [0, 1]"), "the record's set is not C's: {opens:?}");
        assert!(tx.clock.is_some(), "the owner adopted no clock");
    }

    #[test]
    fn ur_25_a_transmit_poll_during_a_control_open_starts_no_streamer_call() {
        // UR-25, UR-22 (S22): a turn of the run loop while uhd-control's enable from no
        // stream owns the streamer — the plan, the report read and the step alike — makes no
        // device call and takes no report read.
        let (core, spy) = spy_rig();
        let control = control_of(&core);
        let gate = spy.stop_before("tx_open");
        let host = core.device.host();
        let core_ = core.clone();
        let (done, finished) = std::sync::mpsc::channel();
        let control_thread = host.spawn("control", move || {
            let mut control = control;
            tx_enable_from_none(&mut control, &core_, 2, 100, 1);
            let _ = done.send(());
        }).expect("a participant thread");
        gate.wait();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        let mark = spy.log().len();
        assert!(!tx.read(None));
        tx.pass();
        assert_eq!(spy.seen("tx_async"), 0, "the poll read reports during a control open");
        assert_eq!(spy.log().len(), mark, "the poll called the device: {:?}", spy.log());
        assert!(!tx_record(&core).1, "the poll took a report read");
        gate.release();
        assert!(finished.recv_timeout(std::time::Duration::from_secs(10)).is_ok(), "the enable did not book");
        control_thread.join().expect("the control thread");
        assert_eq!(spy.seen("tx_open"), 1, "the enable's open: {:?}", spy.log());
        assert!(spy.seen("apply") >= 1, "the enable configured nothing: {:?}", spy.log());
        assert!(!tx_record(&core).0, "the stream stayed uhd-control's");
    }

    #[test]
    fn ur_25_a_from_none_enable_waits_for_a_transmit_report_read() {
        // UR-25 (S23): a report read in flight is uhd-tx's until it ends. An enable from no
        // stream that finds it set waits — without a device call — and never falls back to a
        // change the owner books.
        let (core, spy) = spy_rig();
        let (to_tx, cmds) = std::sync::mpsc::channel();
        let tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        let gate = spy.stop_before("tx_async");
        let host = core.device.host();
        let tx_thread = host.spawn("tx", move || tx.run()).expect("a participant thread");
        gate.wait();
        assert!(tx_record(&core).1, "the report read is not owned");
        let (entered, entered_rx) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let released = std::sync::Mutex::new(released);
        crate::provider::test_support::probe(&core, "wait_reading", move |_| {
            let _ = entered.send(());
            let _ = released.lock().unwrap().recv();
        });
        let control = control_of(&core);
        let core_ = core.clone();
        let (done, finished) = std::sync::mpsc::channel();
        let control_thread = host.spawn("control", move || {
            let mut control = control;
            tx_enable_from_none(&mut control, &core_, 2, 100, 1);
            let _ = done.send(());
        }).expect("a participant thread");
        entered_rx.recv_timeout(std::time::Duration::from_secs(10)).expect("the enable entered the wait");
        assert_eq!(spy.seen("tx_open"), 0, "the enable opened the streamer while a report read is in flight: {:?}", spy.log());
        assert!(tx_record(&core).0, "the enable did not take the streamer");
        release.send(()).unwrap();
        gate.release();
        // The read is over; the enable goes on while the owner keeps polling, before any
        // stop of its own, so that only the run loop's own read can end the wait (UR-25).
        assert!(finished.recv_timeout(std::time::Duration::from_secs(30)).is_ok(), "the enable did not book after the read");
        control_thread.join().expect("the control thread");
        let _ = to_tx.send(TxCmd::Shutdown);
        tx_thread.join().expect("the transmit thread");
        assert_eq!(spy.seen("tx_open"), 1, "the enable's open: {:?}", spy.log());
        assert!(spy.seen("apply") >= 1, "the enable configured nothing: {:?}", spy.log());
        let (busy, reading, configured) = tx_record(&core);
        assert!(!busy && !reading, "the ownership did not go back");
        let planned = plan_of(&core, Dir::Tx);
        assert_eq!(configured, planned[0].segment.by, "the enable fell back to a change the owner books");
    }

    #[test]
    fn ur_25_a_from_none_enable_ends_its_wait_at_the_provider_stop() {
        // UR-25, UR-26 (S27's stop): the Provider's stop, read from its mark, ends an enable's
        // wait for a report read: no device call, and the streamer goes back.
        let (core, spy) = spy_rig();
        let (to_tx, cmds) = std::sync::mpsc::channel();
        let tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        let gate = spy.stop_before("tx_async");
        let host = core.device.host();
        let tx_thread = host.spawn("tx", move || tx.run()).expect("a participant thread");
        gate.wait();
        let (entered, entered_rx) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let released = std::sync::Mutex::new(released);
        crate::provider::test_support::probe(&core, "wait_reading", move |_| {
            let _ = entered.send(());
            let _ = released.lock().unwrap().recv();
        });
        let control = control_of(&core);
        let core_ = core.clone();
        let (done, finished) = std::sync::mpsc::channel();
        let control_thread = host.spawn("control", move || {
            let mut control = control;
            tx_enable_from_none(&mut control, &core_, 2, 100, 1);
            let _ = done.send(());
        }).expect("a participant thread");
        entered_rx.recv_timeout(std::time::Duration::from_secs(10)).expect("the enable entered the wait");
        {
            // As `Provider::stop` books it before its joins.
            let mut streams = lock(&core.streams);
            let at = core.now();
            streams.stop = Some((at, ezsdr_kernel::module_api::StopMode::Orderly));
            streams.end(&core, at, false, false);
        }
        release.send(()).unwrap();
        assert!(finished.recv_timeout(std::time::Duration::from_secs(10)).is_ok(), "the stop did not end the wait");
        control_thread.join().expect("the control thread");
        assert_eq!((spy.seen("tx_open"), spy.seen("apply")), (0, 0), "a stopped enable called the device: {:?}", spy.log());
        assert_eq!(tx_colds(&core), 0, "a stopped enable booked a change");
        assert!(!tx_record(&core).0, "the stream stayed uhd-control's");
        gate.release();
        let _ = to_tx.send(TxCmd::Shutdown);
        tx_thread.join().expect("the transmit thread");
    }

    #[test]
    fn ur_29_a_lost_device_ends_a_from_none_enable_s_wait_for_a_report_read() {
        // UR-29, UR-25: a device loss recorded while an enable from no stream waits for a
        // report read ends the wait: no device call, and the streamer goes back. The read is
        // held open until after the control thread has ended, so only the loss can end it.
        let (core, spy) = spy_rig();
        let (to_tx, cmds) = std::sync::mpsc::channel();
        let tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        let gate = spy.stop_before("tx_async");
        let host = core.device.host();
        let tx_thread = host.spawn("tx", move || tx.run()).expect("a participant thread");
        gate.wait();
        let (entered, entered_rx) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let released = std::sync::Mutex::new(released);
        crate::provider::test_support::probe(&core, "wait_reading", move |_| {
            let _ = entered.send(());
            let _ = released.lock().unwrap().recv();
        });
        let control = control_of(&core);
        let core_ = core.clone();
        let (done, finished) = std::sync::mpsc::channel();
        let control_thread = host.spawn("control", move || {
            let mut control = control;
            tx_enable_from_none(&mut control, &core_, 2, 100, 1);
            let _ = done.send(());
        }).expect("a participant thread");
        entered_rx.recv_timeout(std::time::Duration::from_secs(10)).expect("the enable entered the wait");
        core.device_lost("test");
        release.send(()).unwrap();
        assert!(finished.recv_timeout(std::time::Duration::from_secs(10)).is_ok(), "the loss did not end the wait");
        control_thread.join().expect("the control thread");
        assert_eq!((spy.seen("tx_open"), spy.seen("apply")), (0, 0), "a lost enable called the device: {:?}", spy.log());
        assert_eq!(tx_colds(&core), 0, "a lost enable booked a change");
        assert!(!tx_record(&core).0, "the stream stayed uhd-control's");
        gate.release();
        let _ = to_tx.send(TxCmd::Shutdown);
        tx_thread.join().expect("the transmit thread");
    }

    /// The layout's key set with two transmit streams and no receive channel (RM-29).
    fn two_tx_mode() -> super::super::mode::Mode {
        use ezsdr_kernel::spec::{Ident, Scalar, Value};
        let selector = std::collections::BTreeMap::from([
            (Ident::parse("keys").unwrap(), Value::from("channel".to_owned())),
            (Ident::parse("rx").unwrap(), Value::List(Vec::new())),
            (Ident::parse("tx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:B:ubx:0")])),
        ]);
        let layout = ezsdr_radio::channels::from_selector(&selector).unwrap();
        super::super::mode::Mode::Layout(Box::new(ezsdr_radio::channels::Description::new(&layout, 2_000, &crate::profile::terms())))
    }

    #[test]
    fn ur_28_a_later_stream_s_reopen_discards_only_its_correlations() {
        // UR-28, UR-36 (T3): the streamer's generation is per stream; an open of stream 1
        // discards stream 1's pending correlations alone.
        let inner = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig::default()));
        let feed = Arc::new(ReportFeed::new(inner.clone()));
        let (core, _, _) = super::super::test_support::rig_on_mode(feed.clone(), Vec::new(), BTreeMap::new(), two_tx_mode());
        let (_to0, cmds0) = std::sync::mpsc::channel();
        let (_to1, cmds1) = std::sync::mpsc::channel();
        let mut s0 = Tx::new(core.clone(), 0, cmds0, None, Channels::first(1));
        let mut s1 = Tx::new(core.clone(), 1, cmds1, None, Channels::first(1));
        s0.remember_burst(TimePoint::new(core.root, 0));
        s1.remember_burst(TimePoint::new(core.root, 0));
        core.open_streamer(Dir::Tx, 1, Channels::first(1)).unwrap();
        feed.give(TxReport { code: TxCode::Other(0x40), tick: Some(0), channel: 0 });
        s1.reports(std::time::Duration::ZERO);
        assert_eq!(feed.reads(0), 0, "stream 0's streamer was not read");
        assert!(feed.reads(1) >= 1, "stream 1's streamer was read");
        assert!(s1.unacked.is_empty(), "stream 1's old correlations are dropped");
        assert_eq!(s0.unacked.len(), 1, "stream 0's correlation stands");
        assert!(!inner.calls().iter().any(|call| call.starts_with("tx_send")), "{:?}", inner.calls());
    }

    #[test]
    fn ur_25_a_transmit_report_read_of_a_later_stream_does_not_hold_the_earlier_owner() {
        // UR-25 (T4): an enable from no stream of stream 1 waits on stream 1's report read
        // alone; stream 0's owner keeps making device calls meanwhile.
        let device = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig::default()));
        device.tx_open(0, &[0]).unwrap();
        device.tx_open(1, &[1]).unwrap();
        let (core, _, _) = super::super::test_support::rig_on_mode(device.clone(), Vec::new(), BTreeMap::new(), two_tx_mode());
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to0, cmds0) = std::sync::mpsc::channel();
        let mut s0 = Tx::new(core.clone(), 0, cmds0, Some(clock), Channels::first(1));
        let at = clock.at_or_after(core.now() + core.ticks(1_000_000));
        s0.command(TxCmd::Burst(held(&clock, at, 1_000, LatePolicy::SendAsapAndFlag)));
        lock(&core.streams).stream_mut(stream_key(Dir::Tx, 1)).reading = true;
        // Clears stream 1's `reading` even when an assert below fails (T4).
        struct Reading<'a>(&'a Arc<Core>);
        impl Drop for Reading<'_> {
            fn drop(&mut self) {
                lock(&self.0.streams).stream_mut(stream_key(Dir::Tx, 1)).reading = false;
            }
        }
        let reading = Reading(&core);
        let (to_tx, _cmds1) = std::sync::mpsc::channel();
        let mut control = Control::new(core.clone(), Arc::new(NoActions), vec![to_tx.clone(), to_tx], core.mode.defaults(), false,
            super::super::control::Start { t0: 0, rx: vec![None], tx: vec![None, None] });
        let (entered, arrived) = std::sync::mpsc::channel();
        super::super::test_support::probe(&core, "wait_reading", move |_| { let _ = entered.send(()); });
        let worker = core.clone();
        let inside = core.clone();
        let control_thread = worker.device.host().spawn("test-control", move || {
            control.book(Dispatched::new(ActionId(2), Action::UpdateParameter {
                target: super::super::test_support::target_of(&inside.id),
                key: crate::provider::core::key("radio.tx.c1.stream"),
                value: Value::from(1),
                class: UpdateClass::Cold,
                at: None,
            }, Some(inside.id.clone())));
        }).expect("a participant thread");
        arrived.recv_timeout(std::time::Duration::from_secs(10)).expect("the enable waits for stream 1's read");
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = done.clone();
        super::super::test_support::probe(&core, "reading_done", move |_| { flag.store(true, std::sync::atomic::Ordering::SeqCst); });
        device.host().sleep(std::time::Duration::from_millis(20));
        assert!(!done.load(std::sync::atomic::Ordering::SeqCst), "the enable waits for stream 1: {:?}", device.calls());
        assert!(s0.pass(), "stream 0's owner works");
        assert!(device.calls().iter().any(|call| call.starts_with("tx_send 0")), "stream 0's owner made its device call: {:?}", device.calls());
        assert!(lock(&core.streams).stream(stream_key(Dir::Tx, 1)).reading, "stream 1's read is still in flight");
        drop(reading);
        control_thread.join().expect("the control thread");
        assert!(done.load(std::sync::atomic::Ordering::SeqCst), "the enable goes on once the read ends");
    }

    #[test]
    fn ur_29_a_loss_after_a_configure_adopts_no_transmit_clock() {
        // UR-29 (L2): the device is found lost while the switch's configuration is in
        // flight; the owner adopts no clock and sends nothing.
        let inner = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig::default()));
        let holder = Arc::new(HoldApply::new(inner.clone(), Dir::Tx, Some(0)));
        let (core, _time, _events) = super::super::test_support::rig_on(holder.clone(), Vec::new(), 2_000);
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        let clock = core.register(Dir::Tx, 200, 1_000).unwrap();
        tx.switch = Some(switch(0, Some(clock), 1, Settings::default()));
        let (reached, release) = holder.arm();
        let worker = core.clone();
        let switching = std::thread::spawn(move || { tx.do_switch(); tx.clock });
        reached.recv_timeout(std::time::Duration::from_secs(10)).expect("the switch's configure");
        worker.device_lost("test: lost during the configure");
        release.send(()).unwrap();
        assert!(switching.join().unwrap().is_none(), "the lost configure adopts no clock");
        assert!(!inner.calls().iter().any(|call| call.starts_with("tx_send")), "{:?}", inner.calls());
    }

    #[test]
    fn ur_26_a_transmit_switch_at_a_stop_keeps_its_clock_and_sends_nothing() {
        // UR-26 (S16b): on stream 1, the owner does not re-read `Provider::stop` after its
        // switch's configuration; it takes the clock, and the plan ends that segment at its
        // origin, so nothing is sent.
        let inner = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig::default()));
        let holder = Arc::new(HoldApply::new(inner.clone(), Dir::Tx, Some(0)));
        let (core, _, _) = super::super::test_support::rig_on_mode(holder.clone(), Vec::new(), BTreeMap::new(), two_tx_mode());
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 1, cmds, None, Channels::first(1));
        let key = stream_key(Dir::Tx, 1);
        let config = ezsdr_radio::timeline::Config { channels: Channels::first(1), ratio: ezsdr_kernel::time::Rational::new(200, 1).unwrap() };
        let clock = {
            let mut streams = lock(&core.streams);
            streams.lines.insert(key, ezsdr_radio::timeline::Line::new(ezsdr_radio::timeline::Stream {
                direction: ezsdr_kernel::stream::Direction::Tx,
                origin: 0,
                config: ezsdr_radio::timeline::Config { channels: Channels::first(0), ratio: ezsdr_kernel::time::Rational::new(200, 1).unwrap() },
                lead: 0,
                call: 2_000,
                allowance: 0,
            }));
            streams.stream_mut(key).configs.insert(1, Arc::new(super::super::control::ColdConfig::new(Settings::default())));
            assert!(streams.book(&core, key, ezsdr_radio::timeline::Item { e: 2_000, seq: 1, ready: 0, release: ezsdr_radio::timeline::NO_RELEASE, delivered: None, refused: false, kind: ezsdr_radio::timeline::Kind::Cold(config) }));
            streams.planned(key)[0].clock.expect("the booked segment's clock")
        };
        tx.switch = Some(switch(0, Some(clock), 1, Settings::default()));
        let (reached, release) = holder.arm();
        let worker = core.clone();
        let switching = std::thread::spawn(move || { tx.do_switch(); tx });
        reached.recv_timeout(std::time::Duration::from_secs(10)).expect("the switch's configure");
        {
            let mut streams = lock(&worker.streams);
            streams.stop = Some((500, StopMode::Orderly));
            streams.end(&worker, 500, false, false);
        }
        release.send(()).unwrap();
        let mut tx = switching.join().expect("the switch's thread");
        let clock = tx.clock.expect("the owner takes the clock");
        let record = worker.clocks.sample_clock_records().into_iter().find(|record| record.domain == clock.domain).expect("the clock's record");
        assert_eq!(record.ended_at.and_then(|at| at.ticks_in(record.root).ok()), Some(clock.origin), "the segment ends at its origin: {record:?}");
        // A burst on the clock whose segment ended at its origin, 2 ms of that clock past the
        // origin so that its lateness policy is not what keeps it back: only the ended
        // segment can. The owner's ordinary turn after the switch has nothing to send on it.
        let k = 2_001;
        tx.command(TxCmd::Burst(held(&clock, k, 1_000, LatePolicy::DropAndFlag)));
        assert!(!tx.pass(), "the ended segment leaves nothing to send");
        assert!(lock(&worker.rec).rejected.is_empty(), "the switch is no refusal: {:?}", lock(&worker.rec).rejected);
        assert!(!inner.calls().iter().any(|call| call.starts_with("tx_send 1")), "{:?}", inner.calls());
    }

    #[test]
    fn ur_25_a_detached_control_does_not_hold_the_shutdown_s_read() {
        // UR-25, UR-16: a uhd-control the join bound left detached may never clear its mark;
        // the shutdown's ACK window still reads the reports.
        let (core, spy) = spy_rig();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        lock(&core.streams).stream_mut(stream_key(Dir::Tx, 0)).busy = true;
        tx.shutdown();
        assert!(spy.seen("tx_async") >= 1, "the shutdown's read waited for a detached control");
        assert!(!tx_record(&core).1, "the shutdown's read was not taken back");
    }
}
