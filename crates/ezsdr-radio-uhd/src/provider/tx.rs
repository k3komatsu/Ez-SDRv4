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

use super::core::{Clock, Core, lock};
use crate::device::{Dir, Iq, Settings, TxCode, TxReport};
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
    Switch { e1: i64, clock: Option<Clock>, channels: usize, settings: Settings },
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
    settings: Settings,
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
            match self.held.keys().next() {
                Some(&k) if k == c => {}
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
                self.open = Some(Open { held, next: k, first: true });
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
        // A Kernel burst's end is never the device burst's at once: the next burst held at
        // its next sample continues it, and a burst booked there later still may (Review N,
        // B3); `step` ends the device burst otherwise, a device lead before it runs dry.
        let device_eob = false;
        let at = device_sob.then(|| clock.instant(start));
        let result = self.core.device.tx_send(&slices, at, device_sob, device_eob, SEND_TIMEOUT);
        let next = open.next;
        match result {
            Ok(sent) if sent == count as usize => {}
            Ok(_) => {
                self.core.reject_note(json!({ "action": "tx_burst", "reason": "UR-22: a send did not complete within 1 s" }));
                return self.abandon();
            }
            Err(error) => {
                self.core.device_failed("tx_burst", &error);
                return self.abandon();
            }
        }
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
            self.continues_at = Some(open.next);
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
        let device_open = self.open.as_ref().map_or(continuing.is_some(), |open| !open.first || continuing == Some(open.held.k));
        if device_eob && device_open && !self.core.is_lost() {
            let empty: Vec<&[Iq]> = vec![&[]; self.channels.max(1)];
            let _ = self.core.device.tx_send(&empty, None, false, true, SEND_TIMEOUT);
        }
        let Some(open) = self.open.take() else { return };
        if let Some(record) = self.tracker.as_mut().and_then(BurstTracker::stop) {
            lock(&self.core.rec).bursts.push(serde_json::to_value(record).expect("a record"));
        }
        self.forget(&open.held);
    }

    fn abandon(&mut self) {
        // A send that failed or came up short: end the device burst if one is open, so
        // that none is left without end-of-burst (Review N, N7).
        let continuing = self.continues_at.take();
        let device_open = self.open.as_ref().is_some_and(|open| !open.first || continuing == Some(open.held.k));
        if device_open && !self.core.is_lost() {
            let empty: Vec<&[Iq]> = vec![&[]; self.channels.max(1)];
            let _ = self.core.device.tx_send(&empty, None, false, true, SEND_TIMEOUT);
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
                    if self.held.contains_key(&next) {
                        self.forget(&held);
                        self.time_error(TimeErrorOutcome::Refused, late_by.ticks, held.target);
                        return self.core.command_rejected("tx_burst", "UR-21: the moved start is a held burst's");
                    }
                    self.time_error(TimeErrorOutcome::SendAsap, late_by.ticks, held.target);
                    self.forget(&held);
                    held.k = next;
                    lock(&self.core.held).insert((held.domain, next));
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
        if let Some(new) = switch.clock {
            let reopened = if switch.channels != self.channels {
                self.core.device.tx_open(switch.channels)
            } else {
                Ok(())
            };
            match reopened.and_then(|()| self.core.configure(Dir::Tx, switch.channels, &switch.settings)) {
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
    }

    /// UR-28: every report recorded; underflows and late bursts become events.
    fn reports(&mut self, timeout: Wall) {
        while let Some(report) = self.core.device.tx_async(timeout) {
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
            TxCode::Underflow | TxCode::UnderflowInPacket => underflow(TxUnderflowCause::Starved),
            TxCode::SeqError | TxCode::SeqErrorInBurst => underflow(TxUnderflowCause::Lost),
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
