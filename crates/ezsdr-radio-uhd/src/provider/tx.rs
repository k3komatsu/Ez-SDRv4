//! uhd-tx: owns the transmit streamer. It transmits held bursts in target order,
//! keeps at most the in-flight window handed to the device ahead of its time, decides
//! a preempting burst against the first sample it has not handed over, performs the
//! transmit side of a `cold` change, and reads the asynchronous reports (UR-21…UR-23,
//! UR-25, UR-28).

use std::collections::{BTreeMap, VecDeque};
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

use super::control::ColdConfig;
use super::core::{Clock, Core, lock};
use crate::device::{Dir, Iq, TxCode, TxReport};
use crate::profile::{DEVICE_LEAD_NS, IN_FLIGHT_WINDOW_NS};

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
    /// `Stop` for `<id>/tx` or `<id>` (UR-26).
    Stop,
    /// A stream enabled from 0 channels, already configured (UR-25).
    Enable { clock: Clock, channels: usize },
    /// A `cold` change: the old stream ends at `e1` (UR-25).
    Switch { e1: i64, clock: Option<Clock>, channels: usize, settings: Arc<ColdConfig> },
    /// `Provider::stop` (UR-26).
    Shutdown,
}

struct Open {
    held: Held,
    next: i64,
    first: bool,
}

struct Switch {
    e1: i64,
    clock: Option<Clock>,
    channels: usize,
    settings: Arc<ColdConfig>,
}

pub(crate) struct Tx {
    core: Arc<Core>,
    cmds: Receiver<TxCmd>,
    clock: Option<Clock>,
    channels: usize,
    held: BTreeMap<i64, Held>,
    later: Vec<Held>,
    open: Option<Open>,
    tracker: Option<BurstTracker>,
    switch: Option<Switch>,
    unacked: VecDeque<TimePoint>,
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
    pub fn new(core: Arc<Core>, cmds: Receiver<TxCmd>, clock: Option<Clock>, channels: usize) -> Tx {
        Tx {
            core,
            cmds,
            tracker: clock.map(|c| BurstTracker::new(c.domain)),
            clock,
            channels,
            held: BTreeMap::new(),
            later: Vec::new(),
            open: None,
            switch: None,
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
                std::thread::sleep(Wall::from_millis(1));
                continue;
            }
            self.reports(Wall::ZERO);
            if !self.step() {
                std::thread::sleep(IDLE);
            }
        }
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
            TxCmd::Enable { clock, channels } => {
                self.clock = Some(clock);
                self.channels = channels;
                self.tracker = Some(BurstTracker::new(clock.domain));
                self.adopt_later();
            }
            TxCmd::Switch { e1, clock, channels, settings } => {
                self.switch = Some(Switch { e1, clock, channels, settings });
            }
            TxCmd::Shutdown => unreachable!("handled by run"),
        }
    }

    fn adopt_later(&mut self) {
        let domain = self.clock.map(|c| c.domain);
        for held in std::mem::take(&mut self.later) {
            if Some(held.domain) == domain {
                self.held.insert(held.k, held);
            } else {
                self.forget(&held);
                self.core.command_rejected("tx_burst", "UR-25: its transmit clock was never started");
            }
        }
    }

    fn forget(&self, held: &Held) {
        lock(&self.core.held).remove(&(held.domain, held.k));
    }

    /// One unit of work; false when there was nothing to do.
    fn step(&mut self) -> bool {
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
        let e1_k = self.switch.as_ref().map(|s| clock.at_or_after(s.e1));
        if self.open.is_none() {
            if let Some(e1) = self.switch.as_ref().map(|s| s.e1) {
                if now >= e1 {
                    self.do_switch();
                    return true;
                }
            }
        }
        if let Some(next) = self.open.as_ref().map(|o| o.next) {
            if clock.instant(next) < now {
                self.core.reject_note(json!({ "action": "tx_burst", "reason": "UR-22: the transmit timeline expired; burst not resumed" }));
                self.abandon(true);
                return true;
            }
            if let Some(&h) = self.held.keys().next() {
                if h <= next && e1_k.is_none_or(|e1| h < e1) {
                    self.preempt(clock, h, next);
                    return true;
                }
            }
            if e1_k.is_some_and(|e1| next >= e1) {
                self.end_open(true);
                return true;
            }
            if clock.instant(next) - now >= window {
                return false;
            }
            let held_cut = self.held.keys().next().copied();
            self.send(clock, held_cut, e1_k);
            return true;
        }
        if let Some(c) = self.continues_at {
            if clock.instant(c) < now {
                self.core.reject_note(json!({ "action": "tx_burst", "reason": "UR-22: the transmit timeline expired; burst not resumed" }));
                self.abandon(true);
                return true;
            }
            match self.held.keys().next() {
                // Not across e₁: there the device burst ends (Review O, N-4).
                Some(&k) if k == c && e1_k.is_none_or(|e1| c < e1) => {}
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
            if e1_k.is_some_and(|e1| k >= e1) {
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
                if e1_k.is_some_and(|e1| held.k >= e1) {
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
                self.send(clock, held_cut, e1_k);
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
        let ends_at_held = held_cut.is_some_and(|h| open.next + count == h) && switch_cut.is_none_or(|e1| h_lt(held_cut, e1));
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
                match self.core.device.tx_send(&tail, None, false, false, SEND_TIMEOUT) {
                    Ok(1) => {}
                    Ok(_) => {
                        self.core.reject_note(json!({ "action": "tx_burst", "reason": "UR-22: a send did not complete within 1 s" }));
                        return self.abandon(true);
                    }
                    Err(error) => {
                        self.core.device_failed("tx_burst", &error);
                        return self.abandon(true);
                    }
                }
            }
        }
        let sent: Vec<&[Iq]> = slices.iter().map(|ch| &ch[..sending]).collect();
        let result = if sending > 0 { self.core.device.tx_send(&sent, at, device_sob, device_eob, SEND_TIMEOUT) } else { Ok(0) };
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
                self.core.reject_note(json!({ "action": "tx_burst", "reason": "UR-22: a send did not complete within 1 s" }));
                if device_sob && sent > 0 {
                    self.unacked.push_back(TimePoint::new(clock.domain, start));
                }
                return self.abandon(was_open || sent > 0);
            }
            Err(error) => {
                // Whether UHD sent packets before the error is unknown: a start that may have
                // gone is taken as a device burst begun, ended and counted for its report (an
                // end-of-burst outside a burst costs one zero sample; Review Q, NB-Q1).
                self.core.device_failed("tx_burst", &error);
                if device_sob {
                    self.unacked.push_back(TimePoint::new(clock.domain, start));
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
            channels: self.channels.max(1) as u16,
            direction: Direction::Tx,
            valid: ChannelMask::full(self.channels.max(1) as u16),
            flags,
            lost: None,
            contract: DataContractId::parse("ezsdr.stream.cf32").expect("cf32"),
        };
        let open_record = first.then_some(open.held.open);
        if device_sob {
            self.unacked.push_back(TimePoint::new(clock.domain, start));
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
            None => vec![&empty; self.channels.max(1)],
        };
        if let Err(error) = self.core.device.tx_send(&buffers, None, false, true, SEND_TIMEOUT) {
            self.core.device_failed("tx_burst", &error);
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

    fn time_error(&self, outcome: TimeErrorOutcome, late_by_ns: i64, target: TimePoint) {
        let payload = serde_json::to_value(TimeErrorPayload { cause: TimeErrorCause::Late, outcome, late_by_ns, target })
            .expect("a payload");
        self.core.emit(&self.core.tx_id, kinds::TIME_ERROR, Severity::Error, payload);
    }

    /// UR-21: booking's verdict can expire before the first device hand-over.
    fn dispatch_start(&self, clock: Clock, mut held: Held) -> Option<Held> {
        let lead = self.core.ticks(DEVICE_LEAD_NS);
        let now = clock.at_or_after(self.core.now().max(clock.origin - lead));
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
                    self.time_error(TimeErrorOutcome::Refused, late_by.ticks, held.target);
                    self.core.command_rejected("tx_burst", "UR-21: the moved start is a held burst's");
                    return None;
                }
                reservations.remove(&(held.domain, held.k));
                reservations.insert((held.domain, start));
                drop(reservations);
                held.k = start;
                held.open.late = Some(late);
                held.open.requested_target = held.open.requested_target.or(Some(held.target));
                self.time_error(TimeErrorOutcome::SendAsap, late_by.ticks, held.target);
                Some(held)
            }
            Ok(LateOutcome::Drop { late_by }) => {
                self.forget(&held);
                self.time_error(TimeErrorOutcome::Drop, late_by.ticks, held.target);
                self.core.reject_note(json!({ "action": "tx_burst", "reason": "UR-21: the late policy dropped the burst" }));
                None
            }
            Ok(LateOutcome::PlanViolation { late_by }) => {
                self.forget(&held);
                self.time_error(TimeErrorOutcome::PlanViolation, late_by.ticks, held.target);
                self.core.reject_note(json!({ "action": "tx_burst", "reason": "UR-21: the planned burst arrived late" }));
                None
            }
            Err(error) => {
                self.forget(&held);
                self.core.command_rejected("tx_burst", &format!("UR-21: {error}"));
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
                        self.time_error(TimeErrorOutcome::Refused, late_by.ticks, held.target);
                        return self.core.command_rejected("tx_burst", "UR-21: the moved start is a held burst's");
                    }
                    reservations.remove(&(held.domain, held.k));
                    reservations.insert((held.domain, next));
                    drop(reservations);
                    self.time_error(TimeErrorOutcome::SendAsap, late_by.ticks, held.target);
                    held.k = next;
                    held.open.late = Some(late);
                    held.open.requested_target = held.open.requested_target.or(Some(held.target));
                }
                Ok(LateOutcome::Drop { late_by }) => {
                    self.forget(&held);
                    self.time_error(TimeErrorOutcome::Drop, late_by.ticks, held.target);
                    return self.core.reject_note(json!({ "action": "tx_burst", "reason": "UR-21: the late policy dropped the burst" }));
                }
                Ok(LateOutcome::PlanViolation { late_by }) => {
                    self.forget(&held);
                    self.time_error(TimeErrorOutcome::PlanViolation, late_by.ticks, held.target);
                    return self.core.reject_note(json!({ "action": "tx_burst", "reason": "UR-21: the planned burst arrived late" }));
                }
                Err(error) => {
                    self.forget(&held);
                    return self.core.command_rejected("tx_burst", &format!("UR-21: {error}"));
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
                self.core.command_rejected("tx_burst", reason);
            } else {
                self.core.reject_note(json!({ "action": "tx_burst", "reason": reason }));
            }
        }
    }

    /// The transmit side of a `cold` change, at `e1` (UR-25).
    fn do_switch(&mut self) {
        let Some(switch) = self.switch.take() else { return };
        self.end_open(true);
        for (_, held) in std::mem::take(&mut self.held) {
            self.forget(&held);
            self.core.command_rejected("tx_burst", "cancelled by a cold change");
        }
        self.clock = None;
        self.tracker = None;
        if let Some(new) = switch.clock.filter(|_| !self.core.is_lost()) {
            let reopened = if switch.channels != self.channels {
                self.core.device.tx_open(switch.channels)
            } else {
                Ok(())
            };
            match reopened.and_then(|()| switch.settings.configure(&self.core, Dir::Tx, switch.channels)) {
                Ok(_) => {
                    self.clock = Some(new);
                    self.tracker = Some(BurstTracker::new(new.domain));
                }
                Err(error) => {
                    let _ = self.core.clocks.end(new.domain, self.core.at(new.origin));
                    lock(&self.core.streams).tx = None;
                    self.core.device_failed("update_parameter", &error);
                }
            }
        }
        self.channels = switch.channels;
        self.core.timing(json!({ "what": "tx_switch", "e1": switch.e1, "e2": switch.clock.map(|c| c.origin), "at": self.core.now() }));
        self.adopt_later();
        lock(&self.core.streams).switching[Dir::Tx as usize] = false;
    }

    /// UR-28: every report recorded; underflows and late bursts become events.
    fn reports(&mut self, timeout: Wall) {
        while !self.core.is_lost() {
            let Some(report) = self.core.device.tx_async(timeout) else { break };
            self.report(report);
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
            self.core.emit_at(&self.core.tx_id, kinds::TX_UNDERFLOW, Severity::Warning, payload, time);
        };
        match report.code {
            TxCode::BurstAck => {
                self.unacked.pop_front();
            }
            TxCode::Underflow | TxCode::UnderflowInPacket | TxCode::SeqError | TxCode::SeqErrorInBurst => {
                let cause = if matches!(report.code, TxCode::Underflow | TxCode::UnderflowInPacket) {
                    TxUnderflowCause::Starved
                } else { TxUnderflowCause::Lost };
                underflow(cause);
                // Its nominal next sample no longer bounds actual playback. Do not
                // resume untimed payload on the shifted device timeline (UR-22).
                if self.open.is_some() || self.continues_at.is_some() {
                    self.core.reject_note(json!({ "action": "tx_burst", "reason": "UR-22: the device burst underflowed; burst not resumed" }));
                    self.abandon(true);
                }
            }
            TxCode::TimeError => {
                let Some(target) = self.unacked.pop_front() else { return };
                let late_by_ns = match (report.tick, self.clock) {
                    (Some(tick), Some(clock)) => self.core.ns(tick - clock.instant(target.ticks)),
                    _ => 0,
                };
                self.time_error(TimeErrorOutcome::LateAtDevice, late_by_ns, target);
            }
            TxCode::Other(_) => {}
        }
    }

    fn shutdown(mut self) {
        self.stop_now("UR-26: cancelled by Provider::stop", false);
        if !self.core.is_lost() {
            // 100 ms for the last BURST_ACK (UR-26).
            let until = std::time::Instant::now() + Wall::from_millis(100);
            while std::time::Instant::now() < until {
                self.reports(Wall::from_millis(10));
            }
        }
    }
}

fn h_lt(held_cut: Option<i64>, e1: i64) -> bool {
    held_cut.is_some_and(|h| h < e1)
}

#[cfg(test)]
mod tests {
    //! uhd-tx driven directly, where the order of its commands and steps matters and the
    //! Provider's threads cannot arrange it (the exit review's UR-21 clause).

    use std::collections::BTreeMap;
    use std::sync::Arc;

    use ezsdr_kernel::event::{EventCollector, EventKind};
    use ezsdr_kernel::hash::ContentHash;
    use ezsdr_kernel::id::ResourceId;
    use ezsdr_kernel::module_api::Authority;
    use ezsdr_kernel::policy::{EventKindRegistry, Policy};
    use ezsdr_kernel::stream::{BurstOpen, LatePolicy};
    use ezsdr_kernel::time::{ClockRegistry, TimePoint};

    use super::{Held, Tx, TxCmd};
    use crate::device::{Device, Dir, FakeConfig, FakeDevice, Settings};
    use crate::provider::core::{Core, lock};
    use crate::DeviceAuthority;

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

    #[test]
    fn ur_22_underflow_and_loss_end_the_damaged_burst() {
        for code in [crate::device::TxCode::Underflow, crate::device::TxCode::UnderflowInPacket,
            crate::device::TxCode::SeqError, crate::device::TxCode::SeqErrorInBurst] {
            let (core, device, time, events) = super::super::test_support::rig();
            device.tx_open(1).unwrap();
            let clock = core.register(Dir::Tx, 200, 0).unwrap();
            let (_to_tx, cmds) = std::sync::mpsc::channel();
            let mut tx = Tx::new(core.clone(), cmds, Some(clock), 1);
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
            assert!(events.drain().iter().any(|e| e.kind == EventKind::parse(ezsdr_radio::kinds::TX_UNDERFLOW).unwrap()));
        }
    }

    #[test]
    fn ur_22_an_expired_timeline_is_not_resumed_before_its_report() {
        let (core, device, time, _) = super::super::test_support::rig();
        device.tx_open(1).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), cmds, Some(clock), 1);
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
        device.tx_open(1).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), cmds, Some(clock), 1);
        let mut burst = held(&clock, 0, 100_000, LatePolicy::SendAsapAndFlag);
        burst.repeat = true;
        // Prime an untimed device burst to make starvation independent of a
        // host deadline for the initial timed SOB (covered by UR-21 tests).
        device.tx_send(&[&burst.samples[0][..2_000]], None, true, false, std::time::Duration::from_secs(1)).unwrap();
        tx.open = Some(super::Open { held: burst, next: 2_000, first: false });
        tx.unacked.push_back(TimePoint::new(clock.domain, 0));
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
            device.tx_open(1).unwrap();
            device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
            let clock = core.register(Dir::Tx, 200, 0).unwrap();
            let (_to_tx, cmds) = std::sync::mpsc::channel();
            let mut tx = Tx::new(core.clone(), cmds, Some(clock), 1);
            lock(&core.held).insert((clock.domain, 20_000));
            tx.command(TxCmd::Burst(held(&clock, 20_000, 10_000, policy)));
            assert!(!tx.step(), "still outside the in-flight window");
            time.advance_to(core.at(core.ticks(40_000_000))).unwrap();
            assert!(tx.step());
            let events = events.drain();
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
    fn ur_21_first_dispatch_cannot_move_past_a_cold_switch() {
        let (core, device, time, _) = super::super::test_support::rig();
        device.tx_open(1).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), cmds, Some(clock), 1);
        lock(&core.held).insert((clock.domain, 17_000));
        tx.command(TxCmd::Burst(held(&clock, 17_000, 10_000, LatePolicy::SendAsapAndFlag)));
        tx.command(TxCmd::Switch { e1: core.ticks(20_000_000), clock: None,
            channels: 0, settings: Arc::new(super::ColdConfig::new(0, Settings::default())) });
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
        device.tx_open(1).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), cmds, Some(clock), 1);
        lock(&core.held).insert((clock.domain, 20_000));
        tx.command(TxCmd::Burst(held(&clock, 20_000, 10_000, LatePolicy::DropAndFlag)));
        time.advance_to(core.at(core.ticks(18_000_000))).unwrap();
        assert!(tx.step());
        assert_eq!(tx.open.as_ref().unwrap().held.k, 20_000);
        assert!(tx.open.as_ref().unwrap().held.open.late.is_none());
        assert!(events.drain().is_empty());
        assert!(device.calls().iter().any(|c| c.contains("at=4000000 sob=true")));
    }

    #[test]
    fn ur_21_first_dispatch_refuses_a_moved_start_collision() {
        let (core, device, time, events) = super::super::test_support::rig();
        device.tx_open(1).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), cmds, Some(clock), 1);
        for k in [20_000, 42_000] {
            lock(&core.held).insert((clock.domain, k));
            tx.command(TxCmd::Burst(held(&clock, k, 1_000, LatePolicy::SendAsapAndFlag)));
        }
        time.advance_to(core.at(core.ticks(40_000_000))).unwrap();
        assert!(tx.step());
        assert!(tx.open.is_none());
        assert!(tx.held.contains_key(&42_000));
        assert!(events.drain().iter().any(|e| e.payload["outcome"] == "refused"));
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
        device.tx_open(1).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let mut kinds = EventKindRegistry::with_kernel_kinds();
        ezsdr_radio::register(&mut ezsdr_kernel::module_api::ModuleRegistry::new(), &mut ezsdr_kernel::binding::AdmissionCheckRegistry::new(), &mut kinds).unwrap();
        let usrp = ResourceId::parse("usrp").unwrap();
        let pairs: Vec<_> = ["usrp", "usrp/rx", "usrp/tx"].iter().flat_map(|s| kinds.kinds().into_iter().map(move |k| (ResourceId::parse(s).unwrap(), k))).collect();
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
            crate::profile::Profile::X310Ubx.description(2_000),
        ));
        let origin = (core.now() / 200 + 1) * 200;
        let clock = core.register(Dir::Tx, 200, origin).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), cmds, Some(clock), 1);
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
        let refused = events.drain().into_iter().any(|e| {
            e.kind == EventKind::parse(ezsdr_radio::kinds::TIME_ERROR).unwrap() && e.payload["outcome"] == "refused"
        });
        assert!(refused, "no TIME_ERROR {{ refused }}");
    }
    #[test]
    fn ur_29_eob_device_errors_are_escalated() {
        for lost in [true, false] {
            for tail in [None, Some(vec![vec![[0.25, 0.0]]])] {
                let (core, device, _, events) = super::super::test_support::rig();
                device.tx_open(1).unwrap();
                let clock = core.register(Dir::Tx, 200, 0).unwrap();
                let (_to_tx, cmds) = std::sync::mpsc::channel();
                let tx = Tx::new(core.clone(), cmds, Some(clock), 1);
                *device.eob_error.lock().unwrap() = Some(crate::device::DeviceError {
                    lost, message: "controlled EOB failure".to_owned(),
                });
                tx.close_device_burst(tail);
                assert_eq!(core.is_lost(), lost, "EOB failure must reach UR-29");
                let emitted = events.drain();
                let expected = if lost { EventKind::DEVICE_LOST } else { ezsdr_radio::kinds::COMMAND_REJECTED };
                assert_eq!(emitted.iter().filter(|event| event.kind.as_str() == expected).count(), 1);
                if lost {
                    tx.close_device_burst(None);
                    assert!(events.drain().is_empty());
                    assert_eq!(device.calls().iter().filter(|call| call.as_str() == "mark_lost").count(), 1);
                }
            }
        }
    }

    #[test]
    fn ur_29_eob_loss_during_switch_stops_device_calls() {
        let (core, device, _, _) = super::super::test_support::rig();
        device.tx_open(1).unwrap();
        let old = core.register(Dir::Tx, 200, 0).unwrap();
        core.clocks.end(old.domain, core.at(20_000_000)).unwrap();
        let new = core.register(Dir::Tx, 100, 30_000_000).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), cmds, Some(old), 1);
        tx.continues_at = Some(100);
        tx.tail = Some(vec![vec![[0.25, 0.0]]]);
        tx.switch = Some(super::Switch { e1: 20_000_000, clock: Some(new), channels: 2,
            settings: Arc::new(super::ColdConfig::new(new.origin, Settings { rate: Some(2e6), ..Settings::default() })) });
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
        device.tx_open(1).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), cmds, Some(clock), 1);
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

}
