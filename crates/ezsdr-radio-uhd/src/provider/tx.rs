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
use ezsdr_kernel::time::{Duration, Rescaled, TimePoint};
use ezsdr_radio::kinds;
use ezsdr_radio::payloads::{TimeErrorCause, TimeErrorOutcome, TimeErrorPayload, TxUnderflowCause, TxUnderflowPayload};
use serde_json::json;

use super::control::Planned;
use super::core::{stream_key, Clock, Core, Holder, Streams, lock};
use crate::device::{Dir, Iq, TxCode, TxReport};
use crate::host::HostClock;
use crate::profile::DEVICE_LEAD_NS;
use ezsdr_radio::timeline::{Channels, StreamKey};

/// A burst uhd-control booked (UR-21).
pub(crate) struct Held {
    pub k: i64,
    pub domain: ClockDomainId,
    pub samples: Arc<Vec<Vec<Iq>>>,
    pub waveform_len: usize,
    pub repeat: bool,
    pub open: BurstOpen,
    pub policy: LatePolicy,
    pub target: TimePoint,
}

impl Held {
    pub(super) fn tile_repeat(&mut self) {
        let len = self.waveform_len;
        let cap = TX_SEND_CAP as usize;
        if !self.repeat || len == 0 || len >= cap { return; }
        let tiled_len = ((cap + 2 * len - 2) / len) * len;
        let mut channels = Vec::with_capacity(self.samples.len());
        for channel in self.samples.iter() {
            let mut tiled = Vec::with_capacity(tiled_len);
            while tiled.len() < tiled_len { tiled.extend_from_slice(&channel[..len]); }
            channels.push(tiled);
        }
        self.samples = Arc::new(channels);
    }
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

#[derive(Clone, Copy)]
struct Tail {
    samples: [Iq; TX_CHANNELS_MAX],
    channels: usize,
}

impl Tail {
    fn buffers(&self) -> [&[Iq]; TX_CHANNELS_MAX] {
        std::array::from_fn(|channel| std::slice::from_ref(&self.samples[channel]))
    }
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
    tail: Option<Tail>,
    /// First safe sample after a device burst closed on this SampleClock.
    front: Option<(ClockDomainId, i64)>,
    contract: Option<DataContractId>,
}

const IDLE: Wall = Wall::from_micros(200);
const SEND_TIMEOUT: Wall = Wall::from_secs(1);
const TX_SEND_CAP: i64 = 20_000;
const TX_CHANNELS_MAX: usize = 64;

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
            front: None,
            contract: Some(DataContractId::parse("ezsdr.stream.cf32").expect("cf32")),
        }
    }

    pub fn run(mut self) {
        // UR-23: real-time once, on the owner's own thread; a refusal is recorded and survived, not a device loss.
        match self.core.device.raise_thread_priority() {
            Ok(()) => self.core.stat("tx_realtime", 1),
            Err(error) => self.core.reject_note(json!({ "call": "set_thread_priority", "stream": self.k, "error": error.message })),
        }
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
        if !streams.begin_read(key) {
            return false;
        }
        if !self.plan_section(&mut streams, key, None) {
            streams.end_read(key);
            return false;
        }
        drop(streams);
        self.reports(Wall::ZERO);
        lock(&core.streams).end_read(key);
        let worked = self.step();
        if !worked { core.release_unneeded(Dir::Tx, self.k); }
        worked
    }

    fn command(&mut self, cmd: TxCmd) {
        match cmd {
            TxCmd::Burst(mut held) => {
                held.tile_repeat();
                if self.clock.is_some_and(|c| c.domain == held.domain) {
                    self.held.insert(held.k, held);
                } else {
                    // Booked on the clock a pending change starts (UR-25).
                    self.later.push(held);
                }
            }
            TxCmd::Stop => self.stop_now("UR-26: cancelled by Stop", ezsdr_radio::CancelReason::Stop),
            TxCmd::Shutdown => unreachable!("handled by run"),
        }
    }

    /// Reads the transmit plan it shares with uhd-control again, when it has changed (UR-25),
    /// pruning first the items before the command that began the segment `passed` (#63).
    /// False when uhd-control's enable from no stream owns the streamer this turn: the plan is
    /// read again at the next, and the version, the switch and the phase stand.
    pub(super) fn read(&mut self, passed: Option<Option<u64>>) -> bool {
        let core = self.core.clone();
        let key = stream_key(Dir::Tx, self.k);
        let mut streams = lock(&core.streams);
        if streams.holder(key) != Holder::Free {
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
        let segment = self.clock.and_then(|_| self.plan.get(self.at)).map(|planned| {
            (planned.segment.origin, planned.segment.config, planned.segment.by)
        });
        streams.plan_read(key, self.clock.is_some(), self.switch.is_some(), segment);
        true
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
                let reason = "UR-25: its transmit clock was never started";
                self.report_cancelled_burst(reason, ezsdr_radio::CancelReason::ColdChange);
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

    fn remember_front(&mut self, domain: ClockDomainId, sample: i64) {
        let sample = self.front.filter(|(old_domain, _)| *old_domain == domain).map_or(sample, |(_, old)| old.max(sample));
        self.front = Some((domain, sample));
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
        let window = self.core.ticks(self.core.description.timing.tx_in_flight_ns);
        let window_cut = clock.at_or_after(now.saturating_add(window));
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
                self.abandon(true, 0);
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
            let held_cut = self.held.keys().next().copied();
            return self.send(clock, held_cut, cut_k, window_cut);
        }
        if let Some(c) = self.continues_at {
            if clock.instant(c) < now {
                self.core.reject_note(json!({ "kind": "tx_burst", "reason": "UR-22: the transmit timeline expired; burst not resumed" }));
                self.abandon(true, 0);
                return true;
            }
            match self.held.keys().next() {
                // Not across the cut: there the device burst ends (Review O, N-4).
                Some(&k) if k == c && cut_k.is_none_or(|cut| c < cut) => {}
                // A late reservation before a held tail can still continue the open device
                // burst at its first unhanded sample.
                Some(&k) if k < c && cut_k.is_none_or(|cut| k < cut) => {
                    self.preempt(clock, k, c);
                    return true;
                }
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
            if k < window_cut {
                let held = self.held.remove(&k).expect("the first key");
                // Continuations have no timed SOB; a new device burst must still
                // have its device lead after waiting in the owner's queue.
                let held = if self.continues_at == Some(k) {
                    held
                } else {
                    let Some(held) = self.dispatch_start(clock, held) else { return true };
                    held
                };
                if held.k >= window_cut {
                    self.held.insert(held.k, held);
                    return true;
                }
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
                return self.send(clock, held_cut, cut_k, window_cut);
            }
        }
        false
    }

    /// Hands the next buffer of the open burst to the device (UR-22, UR-23). False, with nothing handed over, while the window
    /// cannot yet take all that the other limits allow, except for the burst's first timed buffer, which takes what fits.
    fn send(&mut self, clock: Clock, held_cut: Option<i64>, switch_cut: Option<i64>, window_cut: i64) -> bool {
        let chunk = (self.core.ticks(self.core.description.timing.tx_in_flight_ns) / clock.n / 4).clamp(1, TX_SEND_CAP);
        let open = self.open.as_mut().expect("a burst is open");
        let len = open.held.waveform_len as i64;
        let start = open.held.k;
        let offset = if open.held.repeat { (open.next - start).rem_euclid(len) } else { open.next - start };
        let tiled = open.held.repeat && open.held.samples[0].len() > len as usize;
        let mut wanted = if tiled { chunk } else { chunk.min(len - offset) };
        let cut = [held_cut, switch_cut].into_iter().flatten().min();
        if let Some(cut) = cut {
            wanted = wanted.min(cut - open.next);
        }
        let avail = window_cut - open.next;
        let device_sob = open.first && self.continues_at != Some(start);
        if avail < wanted && !device_sob {
            return false;
        }
        let count = wanted.min(avail);
        debug_assert!(count > 0, "the send is inside the in-flight window");
        let ends_waveform = !open.held.repeat && offset + count == len;
        let ends_at_held = held_cut.is_some_and(|h| open.next + count == h) && switch_cut.is_none_or(|cut| h_lt(held_cut, cut));
        let eob = ends_waveform || ends_at_held;
        let mut slices = [&[][..]; TX_CHANNELS_MAX];
        for (slice, channel) in slices.iter_mut().zip(open.held.samples.iter()) {
            *slice = &channel[offset as usize..(offset + count) as usize];
        }
        let channels = open.held.samples.len();
        let first = open.first;
        // The Kernel's burst boundaries (`first`, `eob`) and the device's differ where one
        // burst continues the device burst of the one before (§11 F3).
        let continuing = first && !device_sob;
        // A Kernel burst's end is the device burst's only once no burst can still be booked
        // at its next sample (it is a device lead away): otherwise its last sample is held
        // back, and `step` sends it with end-of-burst at that deadline, or without it ahead
        // of a burst that continues the device burst there (Review N, B3; Review O, O-B1).
        // A one-sample burst's first buffer is not held: its start goes with it.
        let open_ended = clock.instant(open.next + count) - self.core.now() > self.core.ticks(DEVICE_LEAD_NS);
        let hold = eob && !ends_at_held && open_ended && !(device_sob && count == 1);
        let device_eob = eob && !ends_at_held && !hold;
        let sending = (if hold { count - 1 } else { count }) as usize;
        let at = device_sob.then(|| clock.instant(start));
        // The held tail is at `start - 1`; the continuation cursor already follows it.
        if continuing {
            if let Some(tail) = self.tail.take() {
                // The burst before's held sample, ahead of this one, no end-of-burst between;
                // without it this one would play a sample early, so its failure abandons this
                // burst, the device burst ended (Review Q, NB-Q2).
                let buffers = tail.buffers();
                match self.core.device.tx_send(self.k, &buffers[..tail.channels], None, false, false, SEND_TIMEOUT) {
                    Ok(1) => {}
                    Ok(_) => {
                        self.core.reject_note(json!({ "kind": "tx_burst", "reason": "UR-22: a send did not complete within 1 s" }));
                        return self.abandon(true, 0);
                    }
                    Err(error) => {
                        self.core.device_failed(None, "tx_burst", &error);
                        return self.abandon(true, 0);
                    }
                }
            }
        }
        let mut sent = [&[][..]; TX_CHANNELS_MAX];
        for (slice, channel) in sent[..channels].iter_mut().zip(slices[..channels].iter()) {
            *slice = &channel[..sending];
        }
        let result = if sending > 0 { self.core.device.tx_send(self.k, &sent[..channels], at, device_sob, device_eob, SEND_TIMEOUT) } else { Ok(0) };
        // Held only once the samples before it went out: after a send that failed or came
        // up short it would play straight after what the device took (Review P, NB-1).
        let tail = hold.then(|| {
            let mut samples = [[0.0; 2]; TX_CHANNELS_MAX];
            for (sample, channel) in samples[..channels].iter_mut().zip(slices[..channels].iter()) {
                *sample = channel[sending];
            }
            Tail { samples, channels }
        });
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
                return self.abandon(was_open || sent > 0, sent);
            }
            Err(error) => {
                // Whether UHD sent packets before the error is unknown: a start that may have
                // gone is taken as a device burst begun, ended and counted for its report (an
                // end-of-burst outside a burst costs one zero sample; Review Q, NB-Q1).
                self.core.device_failed(None, "tx_burst", &error);
                if device_sob {
                    self.remember_burst(TimePoint::new(clock.domain, start));
                }
                return self.abandon(true, sending);
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
            contract: self.contract.take().expect("contract is available"),
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
        self.contract = Some(header.contract);
        if eob {
            let open = self.open.take().expect("open");
            if !device_eob {
                self.continues_at = Some(open.next);
            } else {
                self.remember_front(clock.domain, open.next.saturating_add(1));
            }
            self.forget(&open.held);
        }
        true
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
            let cursor = self.open.as_ref().map_or(continuing, |open| Some(open.next));
            let safe_gap = if tail.is_some() { 1 } else { 2 };
            self.close_device_burst(tail);
            if !self.core.is_lost() {
                if let (Some(clock), Some(cursor)) = (self.clock, cursor) {
                    self.remember_front(clock.domain, cursor.saturating_add(safe_gap));
                }
            }
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
    fn close_device_burst(&self, tail: Option<Tail>) {
        if self.core.is_lost() { return; }
        let empty: [Iq; 0] = [];
        let empty_buffers = [&empty[..]; TX_CHANNELS_MAX];
        let tail_buffers = tail.as_ref().map(Tail::buffers);
        let count = tail.as_ref().map_or(self.channels.count().max(1) as usize, |tail| tail.channels);
        let buffers = tail_buffers.as_ref().map_or(&empty_buffers[..count], |buffers| &buffers[..count]);
        if let Err(error) = self.core.device.tx_send(self.k, buffers, None, false, true, SEND_TIMEOUT) {
            self.core.device_failed(None, "tx_burst", &error);
        }
    }

    /// Always true: abandoning a burst is work, which `send`'s callers return.
    fn abandon(&mut self, device_open: bool, handed: usize) -> bool {
        // A send that failed or came up short: end the device burst if one is open, so
        // that none is left without end-of-burst (Review N, N7), with an empty buffer —
        // the burst's samples after what the device took are not sent (Review P, NB-1).
        let handed = i64::try_from(handed).unwrap_or(i64::MAX);
        let cursor = self.open.as_ref().map(|open| open.next.saturating_add(handed))
            .or_else(|| self.continues_at.map(|cursor| cursor.saturating_add(handed)));
        self.continues_at = None;
        self.tail = None;
        if device_open && !self.core.is_lost() {
            self.close_device_burst(None);
            if !self.core.is_lost() {
                if let (Some(clock), Some(cursor)) = (self.clock, cursor) {
                    self.remember_front(clock.domain, cursor.saturating_add(2));
                }
            }
        }
        if let Some(record) = self.tracker.as_mut().and_then(BurstTracker::stop) {
            lock(&self.core.rec).bursts.push(serde_json::to_value(record).expect("a record"));
        }
        if let Some(open) = self.open.take() {
            self.forget(&open.held);
        }
        true
    }

    fn time_error(&self, outcome: TimeErrorOutcome, late_by: Duration, target: TimePoint) {
        // SC-27's late_by is a host.monotonic duration, so its ticks are nanoseconds.
        let late_by_ns = late_by.ticks_in(ClockDomainId::HOST_MONOTONIC).unwrap_or(i64::MAX);
        let payload = serde_json::to_value(TimeErrorPayload { cause: TimeErrorCause::Late, outcome, late_by_ns, target })
            .expect("a payload");
        self.core.emit(&self.core.stream_node(Dir::Tx, self.k), kinds::TIME_ERROR, Severity::Error, payload);
    }

    fn report_send_asap(&self, held: &Held, late_by: Duration) {
        if held.open.late.is_none() {
            self.time_error(TimeErrorOutcome::SendAsap, late_by, held.target);
        }
    }

    fn front_late_by(&self, clock: Clock, ticks: i64) -> Result<Duration, ezsdr_kernel::time::TimeError> {
        match self.core.clocks.rescale(Duration::new(clock.domain, ticks), ClockDomainId::HOST_MONOTONIC)? {
            Rescaled::Exact { duration } => Ok(duration),
            Rescaled::Inexact { floor, .. } => floor.checked_add(Duration::new(ClockDomainId::HOST_MONOTONIC, 1)),
        }
    }

    /// UR-21: booking's verdict can expire before the first device hand-over.
    fn dispatch_start(&self, clock: Clock, mut held: Held) -> Option<Held> {
        let lead = self.core.ticks(DEVICE_LEAD_NS);
        let now = clock.at_or_after(ezsdr_radio::timeline::decided_at(self.core.now(), clock.origin, lead));
        let current = clock.at_or_after(clock.instant(now) + lead);
        let front = self.front.filter(|(domain, _)| *domain == clock.domain).map(|(_, sample)| sample);
        let bound = ezsdr_radio::timeline::later_bound(current, front);
        let outcome = held.policy.decide(&self.core.clocks,
            TimePoint::new(clock.domain, held.k), TimePoint::new(clock.domain, now),
            Duration::new(ClockDomainId::HOST_MONOTONIC, DEVICE_LEAD_NS));
        let outcome = outcome.and_then(|outcome| ezsdr_radio::timeline::late_policy_front_outcome(
            held.policy, held.k, current, front, |ticks| self.front_late_by(clock, ticks),
        ).map(|front| front.unwrap_or(outcome)));
        match outcome {
            Ok(LateOutcome::OnTime {}) => Some(held),
            Ok(late @ LateOutcome::SendAsap { late_by }) => {
                let start = bound;
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
                self.report_send_asap(&held, late_by);
                held.open.late = Some(late);
                held.open.requested_target = held.open.requested_target.or(Some(held.target));
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
        let lead = self.core.ticks(DEVICE_LEAD_NS);
        let lead_k = (lead + clock.n - 1) / clock.n;
        let decision = ezsdr_radio::timeline::decided_at(self.core.now(), clock.origin, lead);
        let decision_sample = clock.at_or_after(decision);
        let current = clock.at_or_after(clock.instant(decision_sample) + lead);
        let bound = ezsdr_radio::timeline::later_bound(current, Some(next));
        if h < bound {
            let outcome = held.policy.decide(
                &self.core.clocks,
                TimePoint::new(clock.domain, h),
                TimePoint::new(clock.domain, bound - lead_k),
                Duration::new(ClockDomainId::HOST_MONOTONIC, DEVICE_LEAD_NS),
            );
            match outcome {
                Ok(LateOutcome::OnTime {}) => held.k = bound,
                Ok(late @ LateOutcome::SendAsap { late_by }) => {
                    let mut reservations = lock(&self.core.held);
                    if reservations.contains(&(held.domain, bound)) {
                        reservations.remove(&(held.domain, held.k));
                        drop(reservations);
                        self.time_error(TimeErrorOutcome::Refused, late_by, held.target);
                        return self.core.command_rejected(None, "tx_burst", "UR-21: the moved start is a held burst's");
                    }
                    reservations.remove(&(held.domain, held.k));
                    reservations.insert((held.domain, bound));
                    drop(reservations);
                    self.report_send_asap(&held, late_by);
                    held.k = bound;
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
        // UR-23: only an adjacent burst continues the device burst; a later start closes it.
        // If the device burst has begun, it continues into the new one without
        // end-of-burst (the X300 drops a timed start at the tick a burst ended, §11 F3);
        // otherwise nothing was sent and the new one starts on its own (Review L, P1-4).
        if held.k == next && (self.continues_at.is_some() || self.open.as_ref().is_some_and(|open| !open.first)) {
            self.end_open(false);
            self.continues_at = Some(held.k);
        } else {
            self.end_open(true);
        }
        self.held.insert(held.k, held);
    }

    fn report_cancelled_burst(&self, reason: &str, cancel_reason: ezsdr_radio::CancelReason) {
        if ezsdr_radio::cancellation_reports(cancel_reason) {
            self.core.command_rejected(None, "tx_burst", reason);
        } else {
            self.core.reject_note(json!({ "kind": "tx_burst", "reason": reason }));
        }
    }

    /// Ends the open burst and cancels every held one (UR-23, UR-26).
    fn stop_now(&mut self, reason: &str, cancel_reason: ezsdr_radio::CancelReason) {
        self.end_open(true);
        let held: Vec<Held> = std::mem::take(&mut self.held).into_values().chain(std::mem::take(&mut self.later)).collect();
        for held in held {
            self.forget(&held);
            self.report_cancelled_burst(reason, cancel_reason);
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
        self.read_reports_before_close();
        for (_, held) in std::mem::take(&mut self.held) {
            self.forget(&held);
            let reason = "cancelled by a cold change";
            self.report_cancelled_burst(reason, ezsdr_radio::CancelReason::ColdChange);
        }
        let passed = self.clock.take().map(|_| self.plan.get(self.at).and_then(|planned| planned.segment.by));
        if passed.is_some() {
            self.at += 1;
        }
        {
            let mut streams = lock(&self.core.streams);
            let stream = stream_key(Dir::Tx, self.k);
            if let Some(active) = streams.finished(stream) { streams.complete(stream, active); }
        }
        self.tracker = None;
        let mut interrupted = false;
        let mut switched = false;
        if let Some(to) = switch.to.filter(|_| !self.core.is_lost()) {
            if to.segment.cut.is_some_and(|cut| to.segment.instant(cut).is_ok_and(|end| end <= to.segment.origin)) {
                self.at += 1;
                self.read(Some(passed.flatten()));
                self.adopt_later();
                return;
            }
            // UR-37: a refused switch makes no device call, and starts no clock; the record
            // decides what an accepted one opens and configures (UR-13, UR-36).
            let configured = if to.refused { Ok(true) } else { self.core.owner_configure(Dir::Tx, self.k, &to) };
            self.sync_streamer();
            match configured {
                // UR-29: another thread found the device lost while the configuration ran;
                // the clock is not adopted and no segment begins.
                Ok(true) if self.core.is_lost() => self.at += 1,
                // UR-25: a refused enable from no stream starts no clock, as the refusal below
                // does; uhd-control reported it.
                Ok(true) if to.refused => self.at += 1,
                Ok(true) => {
                    self.channels = to.set();
                    self.clock = to.clock;
                    self.tracker = to.clock.map(|clock| BurstTracker::new(clock.domain));
                    switched = true;
                }
                Ok(false) => interrupted = true,
                Err((error, claim)) => {
                    // RM-25: the refusal halts the stream, booked by uhd-tx itself, while the
                    // claim still holds the stream in this one section (UR-25).
                    self.at += 1;
                    let mut streams = lock(&self.core.streams);
                    self.core.probe("refused_booking", Some(&streams));
                    if let Some(by) = to.segment.by { streams.refuse(&self.core, stream_key(Dir::Tx, self.k), by); }
                    streams.give_back(claim);
                    drop(streams);
                    self.core.device_failed(None, "update_parameter", &error);
                }
            }
            if switched { self.core.timing(json!({ "what": "tx_switch", "cut": switch.cut, "origin": to.segment.origin, "at": self.core.now() })); }
        }
        if !interrupted { self.adopt_later(); }
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

    /// Reads any report made available by the old burst's end before its owner releases the
    /// streamer (UR-28, A10).
    fn read_reports_before_close(&mut self) {
        if self.core.is_lost() { return; }
        let stream = stream_key(Dir::Tx, self.k);
        // UR-25: a holder other than Free (a from-none enable, say) keeps the stream; the
        // close then makes its reports unread, rather than reading them behind the holder.
        if !lock(&self.core.streams).begin_read(stream) { return; }
        self.reports(Wall::ZERO);
        lock(&self.core.streams).end_read(stream);
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
            "code": format!("{:?}", report.code), "tick": report.tick, "channel": report.channel, "stream": self.k,
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
                    self.abandon(true, 0);
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

    fn shutdown(&mut self) {
        self.stop_now("UR-26: cancelled by Provider::stop", ezsdr_radio::CancelReason::ProviderStop);
        // 100 ms of the device's host clock for the last BURST_ACK (UR-26; plan 30 §4.3: it
        // waits for the device's acknowledgement, so it is measured in clock time). A loss —
        // found by a read here or recorded by another thread — ends the window: `reports` no
        // longer reads, so it no longer waits on the clock either, and the manual clock this
        // busy thread holds still would never reach `until` (UR-33).
        let until = self.host.now() + Wall::from_millis(100);
        while !self.core.is_lost() && self.host.now() < until {
            // UR-25: the shutdown's read is a report read all the same, but it takes no holder
            // check: a uhd-control the join bound left detached (UR-16) may never give its
            // claim back, and must not hold this window open.
            let stream = stream_key(Dir::Tx, self.k);
            lock(&self.core.streams).begin_shutdown_read(stream);
            self.reports(Wall::from_millis(10));
            lock(&self.core.streams).end_read(stream);
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
    use std::time::Duration as Wall;

    use ezsdr_kernel::event::{Action, ActionId, EventCollector, EventKind};
    use ezsdr_kernel::hash::ContentHash;
    use ezsdr_kernel::module_api::{ActionReceiver, Dispatched, StopMode, UpdateClass};
    use ezsdr_kernel::spec::{Scalar, Value};
    use ezsdr_kernel::stream::{BurstOpen, LatePolicy};
    use ezsdr_kernel::time::{AbsoluteDeadline, TimePoint};

    use super::{Held, Tail, Tx, TxCmd, TX_CHANNELS_MAX};
    use crate::device::{Device, Dir, FakeConfig, FakeDevice, FakeFault, Iq, Settings, TxCode, TxReport};
    use super::super::test_support::{HoldApply, ReportFeed};
    use crate::provider::core::{stream_key, Clock, Core, Holder, Phase, lock};
    use crate::provider::test_support::device_at;
    use ezsdr_radio::timeline::Channels;
    use super::super::mode::DescriptionExt;
    use super::super::control::Control;

    fn held(clock: &super::Clock, k: i64, len: usize, policy: LatePolicy) -> Held {
        Held {
            k,
            domain: clock.domain,
            samples: Arc::new(vec![vec![[0.25f32, 0.0]; len]]),
            waveform_len: len,
            repeat: false,
            open: BurstOpen { waveform_len: None, late: None, requested_target: None },
            policy,
            target: TimePoint::new(clock.domain, k),
        }
    }

    #[test]
    fn ur_34_a_transmit_stop_during_release_wait_cancels_later_bursts() {
        // A4/A6: a Stop booked after the owner enters its wait interrupts that turn, lets the
        // owner process the Stop, and cancels a burst held for the move's new clock.
        let spy = Arc::new(LockSpy::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() }));
        let selector = BTreeMap::from([
            (ezsdr_kernel::spec::Ident::parse("rx").unwrap(), Value::List(Vec::new())),
            (ezsdr_kernel::spec::Ident::parse("tx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:B:ubx:0")])),
        ]);
        let layout = ezsdr_radio::channels::from_selector(&selector).unwrap();
        let description = ezsdr_radio::channels::Description::new(&layout, 2_000, &crate::profile::terms());
        let (core, _time, events) = crate::provider::test_support::rig_on_description_links(spy.clone(), vec![Vec::new()], BTreeMap::new(), description);
        spy.watch(&core);
        let (from, to) = (stream_key(Dir::Tx, 0), stream_key(Dir::Tx, 1));
        let ratio = ezsdr_kernel::time::Rational::new(200, 1).unwrap();
        let line = |channels| ezsdr_radio::timeline::Line::new(ezsdr_radio::timeline::Stream {
            direction: ezsdr_kernel::stream::Direction::Tx,
            origin: 0,
            config: ezsdr_radio::timeline::Config { channels, ratio },
            lead: 0,
            call: 0,
            allowance: 0,
        });
        spy.tx_open(0, &[0]).unwrap();
        spy.tx_open(1, &[1]).unwrap();
        let move_clock = {
            let mut streams = lock(&core.streams);
            streams.lines.insert(from, line(Channels::first(1)));
            streams.lines.insert(to, line(Channels::first(2).without(0)));
            streams.stream_mut(from).set = crate::provider::core::DeviceSet::Open(Channels::first(1));
            streams.stream_mut(to).set = crate::provider::core::DeviceSet::Open(Channels::first(2).without(0));
            streams.seq = 1;
            streams.lines.book_move(from, to, 0, Some(10_000), 0, 1, 0).unwrap();
            streams.changed(&core, from);
            streams.changed(&core, to);
            streams.stream(to).clocks[1].0
        };
        let (to_tx, cmds) = std::sync::mpsc::channel();
        let old_clock = lock(&core.streams).stream(to).clocks[0].0;
        let mut tx = Tx::new(core.clone(), 1, cmds, Some(old_clock), Channels::first(2).without(0));
        assert!(tx.read(None), "the owner reads the plan");
        tx.later.push(held(&move_clock, 0, 1, LatePolicy::SendAsapAndFlag));
        lock(&core.held).insert((move_clock.domain, 0));
        let mark = spy.order().len();
        let (entered, reached) = std::sync::mpsc::channel();
        super::super::test_support::probe(&core, "wait_release", move |_| { let _ = entered.send(()); });
        let (done, finished) = std::sync::mpsc::channel();
        let (stages, stage_rx) = std::sync::mpsc::channel();
        let owner = core.device.host().spawn("tx-wait-stop", move || {
            tx.do_switch();
            let _ = stages.send("switch returned");
            tx.command(tx.cmds.recv().unwrap());
            let _ = stages.send("Stop processed");
            let _ = done.send(tx);
        }).unwrap();
        assert!(reached.recv_timeout(std::time::Duration::from_secs(10)).is_ok(), "the transmitter reached its release wait");
        {
            let mut streams = lock(&core.streams);
            let seq = streams.next_seq();
            assert!(streams.book(&core, to, ezsdr_radio::timeline::Item {
                e: core.now(), seq, ready: core.now(), release: ezsdr_radio::timeline::NO_RELEASE,
                delivered: None, refused: false, kind: ezsdr_radio::timeline::Kind::Stop,
            }));
        }
        to_tx.send(TxCmd::Stop).unwrap();
        let switch_stage = stage_rx.recv_timeout(std::time::Duration::from_secs(10));
        if switch_stage.is_err() { core.device_lost("test watchdog: release the transmit owner"); }
        let stop_stage = stage_rx.recv_timeout(std::time::Duration::from_secs(10));
        if stop_stage.is_err() { core.device_lost("test watchdog: release the transmit Stop"); }
        owner.join().unwrap();
        let tx = finished.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
        assert_eq!(switch_stage.unwrap(), "switch returned");
        assert_eq!(stop_stage.unwrap(), "Stop processed");
        assert_eq!(tx.at, 1, "the interrupted wait did not advance past the destination");
        assert!(tx.clock.is_none(), "the waiting clock was not adopted");
        assert!(tx.later.is_empty(), "Stop left a future burst held");
        assert!(!lock(&core.held).contains(&(move_clock.domain, 0)), "Stop retained its burst reservation");
        let calls = spy.log();
        let order = spy.order();
        let report = order[mark..].iter().position(|call| *call == "tx_async").expect("the old report queue was read: {order:?}");
        let close = order[mark..].iter().position(|call| *call == "tx_close").expect("the old streamer is released: {order:?}");
        assert!(report < close, "reports were read before close: {order:?}");
        assert!(!calls.iter().any(|call| call == "tx_open 1 [0, 1]"), "the interrupted destination did not open: {calls:?}");
        assert!(lock(&core.rec).rejected.iter().any(|row| row["reason"] == "UR-26: cancelled by Stop"), "{:?}", lock(&core.rec).rejected);
        assert_eq!(lock(&core.streams).holder(to), Holder::Free);
        assert!(!events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter()
            .any(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED));
    }

    /// A switch at `cut` to the segment of `clock`, `channels` channels configured with
    /// `settings`, or to no stream (UR-25).
    fn switch(core: &Core, cut: i64, clock: Option<super::Clock>, channels: usize, settings: Settings) -> super::Switch {
        switch_on(core, 0, cut, clock, channels, settings)
    }

    fn layout_settings(core: &Core, k: usize, channels: usize, settings: &Settings) -> BTreeMap<ezsdr_kernel::spec::Key, Value> {
        let mut base = core.description.defaults();
        if let Some(rate) = settings.rate {
            base.insert(crate::provider::core::key(&format!("radio.tx.s{k}.sample_rate_hz")), Value::num(rate).unwrap());
        }
        for chan in 0..channels {
            if let Some(freq) = settings.freq {
                base.insert(crate::provider::core::key(&format!("radio.tx.c{chan}.frequency_hz")), Value::num(freq).unwrap());
            }
            if let Some(gain) = settings.gain {
                base.insert(crate::provider::core::key(&format!("radio.tx.c{chan}.gain_db")), Value::num(gain).unwrap());
            }
            if let Some(antenna) = &settings.antenna {
                base.insert(crate::provider::core::key(&format!("radio.tx.c{chan}.antenna")), Value::from(antenna.as_str()));
            }
        }
        base
    }

    fn switch_on(core: &Core, k: usize, cut: i64, clock: Option<super::Clock>, channels: usize, settings: Settings) -> super::Switch {
        let to = clock.map(|clock| super::Planned {
            segment: ezsdr_radio::timeline::Segment {
                origin: clock.origin,
                cut: None,
                config: ezsdr_radio::timeline::Config { channels: ezsdr_radio::timeline::Channels::first(channels as u32), ratio: ezsdr_kernel::time::Rational::new(clock.n as u64, 1).unwrap() },
                by: Some(1),
            },
            settings: Some(Arc::new(super::super::control::ColdConfig::new(layout_settings(core, k, channels, &settings)))),
            k,
            refused: false,
            clock: Some(clock),
        });
        if let Some(to) = &to {
            let stream = stream_key(Dir::Tx, to.k);
            let mut streams = lock(&core.streams);
            if !streams.planned(stream).iter().any(|planned| planned.segment == to.segment) {
                let mut line = ezsdr_radio::timeline::Line::new(ezsdr_radio::timeline::Stream {
                    direction: ezsdr_kernel::stream::Direction::Tx,
                    origin: to.segment.origin,
                    config: to.segment.config,
                    lead: 0,
                    call: 0,
                    allowance: 0,
                });
                line.plan = vec![to.segment];
                streams.lines.insert(stream, line);
            }
            if let (Some(by), Some(settings)) = (to.segment.by, to.settings.clone()) {
                streams.stream_mut(stream).configs.insert(by, settings);
            }
        }
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
        super::super::control::Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], core.description.defaults(), false,
            super::super::control::Start { t0: 0, rx: vec![None; core.description.stream_count(Dir::Rx)], tx: vec![None; core.description.stream_count(Dir::Tx)] })
    }

    /// uhd-control books a transmit-channel membership change at `ms` from no stream
    /// (UR-13): the owner is idle, as uhd-control finds it between two segments.
    fn tx_channels_from_none(control: &mut super::super::control::Control, core: &Core, id: u64, ms: i64, channels: &[u32]) {
        let at = core.ticks(ms * 1_000_000);
        for channel in 0..2 {
            let key = crate::provider::core::key(&format!("radio.tx.c{channel}.stream"));
            let value = Value::from(if channels.contains(&channel) { 0 } else { -1 });
            if control.config_at(at).get(&key) == Some(&value) { continue; }
            lock(&core.streams).search(stream_key(Dir::Tx, 0), false);
            control.book(Dispatched::new(ActionId(id * 2 + u64::from(channel)), Action::UpdateParameter {
                target: super::super::test_support::target_of(&core.id),
                key,
                value,
                class: UpdateClass::Cold,
                at: Some(AbsoluteDeadline::new(core.at(at))),
            }, Some(core.id.clone())));
        }
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
        tx.switch = Some(switch(&core, 0, Some(clock), 1, Settings::default()));
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
    fn ur_13_a_transmit_from_none_segment_the_owner_never_started_is_opened_again() {
        // UR-13 (S2): uhd-control opens and configures B = {0,1} from no stream, its owner never
        // having begun it, and a later enable from no stream opens C = {0}. The owner opens each
        // set itself before it adopts the segment: no record of an earlier open stands for the
        // set a segment runs.
        let (core, device, time, _) = super::super::test_support::rig();
        let mut control = control_of(&core);
        // c1 is already assigned in the layout, but the stream is not armed; c0's membership
        // change makes the from-none control open cover the complete B set.
        control.config.insert(crate::provider::core::key("radio.tx.c1.stream"), Value::from(0));
        for (id, at, channels) in [(2, 100, &[0, 1][..]), (3, 200, &[][..]), (4, 300, &[0][..])] {
            tx_channels_from_none(&mut control, &core, id, at, channels);
        }
        let control_opens: Vec<String> = device.calls().into_iter().filter(|call| call.starts_with("tx_open")).collect();
        assert_eq!(control_opens, ["tx_open 0 [0, 1]", "tx_open 0 [0]"], "{control_opens:?}");
        let planned = plan_of(&core, Dir::Tx);
        let b = planned.iter().find(|planned| planned.segment.config.channels == Channels::first(2)).expect("B's segment");
        let c = planned.iter().find(|planned| planned.segment.config.channels == Channels::first(1)).expect("C's segment");
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        tx.read(None);
        assert_eq!(lock(&core.streams).phase(stream_key(Dir::Tx, 0)), Phase::Pending, "a switch waits for the owner to make it");
        assert!(tx.step(), "B's switch");
        assert_eq!(tx.clock, b.clock, "B is adopted after the owner's own open");
        assert_eq!(lock(&core.streams).active(stream_key(Dir::Tx, 0)), Some((b.segment.origin, b.segment.config, b.segment.by)), "B is the active segment");
        let cut = b.clock.expect("B's clock").instant(b.segment.cut.expect("the empty change cuts B"));
        time.advance_to(core.at(cut)).unwrap();
        tx.read(None);
        assert!(tx.step(), "the zero-duration intermediate membership change is skipped");
        assert_eq!(lock(&core.streams).phase(stream_key(Dir::Tx, 0)), Phase::Pending, "the skipped change leaves the next switch waiting");
        assert!(tx.step(), "C's switch");
        assert_eq!(tx.clock, c.clock, "C is adopted after the owner's own open");
        {
            let streams = lock(&core.streams);
            assert_eq!(streams.active(stream_key(Dir::Tx, 0)), Some((c.segment.origin, c.segment.config, c.segment.by)), "C is the active segment");
            assert!(streams.stream(stream_key(Dir::Tx, 0)).completed.contains(&(b.segment.origin, b.segment.config, b.segment.by)), "B is recorded as finished");
        }
        let opens: Vec<String> = device.calls().into_iter().filter(|call| call.starts_with("tx_open")).collect();
        assert_eq!(opens, ["tx_open 0 [0, 1]", "tx_open 0 [0]", "tx_open 0 [0, 1]", "tx_open 0 [0]"], "{opens:?}");
    }

    #[test]
    fn ur_25_a_refused_enable_with_a_later_cold_has_no_tx_clock() {
        // UR-25: a later Cold keeps the refused transmit segment alive with positive duration,
        // so the owner must not adopt its clock before switching to the next segment.
        use ezsdr_radio::timeline::{Channels, Config, Item, Kind, Line, Stream, NO_RELEASE};
        let (core, device, _, _) = super::super::test_support::rig();
        device.tx_open(0, &[0]).unwrap();
        let c0 = core.register(Dir::Tx, 200, 0).unwrap();
        let config = |n: u64| Config { channels: Channels::first(1), ratio: ezsdr_kernel::time::Rational::new(n, 1).unwrap() };
        let key = stream_key(Dir::Tx, 0);
        {
            let mut streams = lock(&core.streams);
            streams.stream_mut(key).clocks = vec![(c0, false)];
            let lead = core.ticks(ezsdr_radio::timeline::start_lead_ns(&core.description.timing, ezsdr_kernel::stream::Direction::Tx));
            streams.lines.insert(key, Line::new(Stream { direction: ezsdr_kernel::stream::Direction::Tx, origin: 0, config: config(200), lead, call: 0, allowance: 0 }));
            streams.stream_mut(key).configs.insert(1, Arc::new(super::super::control::ColdConfig::new(BTreeMap::new())));
            assert!(streams.book(&core, key, Item { e: core.ticks(100_000_000), seq: 1, ready: 0, release: NO_RELEASE, delivered: None, refused: true, kind: Kind::Cold(config(100)) }));
        }
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(1));
        tx.at = 1;
        tx.read(None);
        {
            let mut streams = lock(&core.streams);
            streams.stream_mut(key).configs.insert(2, Arc::new(super::super::control::ColdConfig::new(BTreeMap::new())));
            assert!(streams.book(&core, key, Item { e: core.ticks(200_000_000), seq: 2, ready: 0, release: NO_RELEASE, delivered: None, refused: false, kind: Kind::Cold(config(100)) }));
        }
        tx.read(None);
        let refused = tx.plan.iter().find(|planned| planned.segment.by == Some(1)).expect("the refused segment is still planned");
        let cut = refused.segment.cut.expect("the later Cold gives it a cut");
        assert!(refused.segment.instant(cut).unwrap() > refused.segment.origin, "the refused segment has positive duration");
        assert!(tx.plan.iter().any(|planned| planned.segment.by == Some(2)), "the later Cold remains planned");
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
            tx.switch = Some(switch(&core, 0, Some(clock), 1, Settings::default()));
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
            tx.switch = Some(switch(&core, 0, Some(clock), channels, Settings::default()));
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
            tx.switch = Some(switch(&core, 0, Some(clock), 1, Settings { rate: Some(2e6), ..Settings::default() }));
            tx.do_switch();
        }
        let calls = inner.calls();
        assert_eq!(calls.iter().filter(|call| call.starts_with("tx_open")).count(), 1, "{calls:?}");
        // The first switch's configure is the refusal: only the second one reaches the device.
        assert_eq!(calls.iter().filter(|call| call.starts_with("apply")).count(), 1, "{calls:?}");
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
        tx.switch = Some(switch(&core, 0, Some(refused), 1, Settings::default()));
        tx.switch.as_mut().unwrap().to.as_mut().unwrap().refused = true;
        tx.do_switch();
        assert!(tx.clock.is_none(), "a refused clock is not adopted");
        assert!(device.calls().is_empty(), "{:?}", device.calls());
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        tx.switch = Some(switch(&core, 0, Some(clock), 1, Settings { rate: Some(2e6), ..Settings::default() }));
        tx.do_switch();
        let calls = device.calls();
        assert_eq!(calls.iter().filter(|call| call.starts_with("tx_open")).count(), 1, "{calls:?}");
        assert!(tx.clock.is_some(), "the accepted change starts its clock");
    }

    #[test]
    fn ur_25_a_refused_last_transmit_change_ends_idle() {
        // UR-25 (Sol's plan review, P6): a refused change with nothing after it completes the
        // segment it switched from, starts no clock, and leaves the owner idle.
        use ezsdr_radio::timeline::{Channels, Config, Line, Segment, Stream};
        let (core, device, _, _) = super::super::test_support::rig();
        let config = |n: u64| Config { channels: Channels::first(1), ratio: ezsdr_kernel::time::Rational::new(n, 1).unwrap() };
        let key = stream_key(Dir::Tx, 0);
        let c0 = core.register(Dir::Tx, 200, 0).unwrap();
        let c1 = core.register(Dir::Tx, 200, 0).unwrap();
        let e = core.ticks(1_000_000_000);
        {
            let mut streams = lock(&core.streams);
            streams.stream_mut(key).clocks = vec![(c0, false), (c1, false)];
            let lead = core.ticks(ezsdr_radio::timeline::start_lead_ns(&core.description.timing, ezsdr_kernel::stream::Direction::Tx));
            let mut line = Line::new(Stream { direction: ezsdr_kernel::stream::Direction::Tx, origin: 0, config: config(200), lead, call: 0, allowance: 0 });
            line.plan = vec![
                Segment { origin: 0, cut: Some(c0.at_or_after(e)), config: config(200), by: None },
                Segment { origin: e, cut: None, config: config(100), by: Some(1) },
            ];
            streams.lines.insert(key, line);
        }
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(c0), Channels::first(1));
        tx.read(None);
        let switch = tx.switch.as_mut().expect("the change's switch");
        assert_eq!(switch.cut, e, "the first segment's cut is the next segment's origin");
        switch.to.as_mut().expect("the change's target").refused = true;
        tx.do_switch();
        let streams = lock(&core.streams);
        assert!(tx.clock.is_none(), "a refused change starts no clock");
        assert!(tx.switch.is_none(), "nothing follows");
        assert_eq!(streams.phase(key), Phase::Idle, "no clock and no switch");
        assert!(streams.active(key).is_none());
        assert!(streams.stream(key).completed.contains(&(0, config(200), None)), "the segment switched from is recorded");
        assert!(device.calls().iter().all(|call| !call.starts_with("tx_open")), "a refused change makes no device call: {:?}", device.calls());
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
            tx.switch = Some(switch(&core, 0, Some(clock), 1, Settings::default()));
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
            tx.stop_now("test stop", ezsdr_radio::CancelReason::Stop);
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
        tx.stop_now("test stop", ezsdr_radio::CancelReason::Stop);
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
            let lead = core.ticks(ezsdr_radio::timeline::start_lead_ns(&core.description.timing, ezsdr_kernel::stream::Direction::Tx));
            streams.lines.insert(stream_key(Dir::Tx, 0), Line::new(Stream { direction: ezsdr_kernel::stream::Direction::Tx, origin: 0, config: config(200), lead, call: 0, allowance: 0 }));
        }
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(c0), Channels::first(1));
        tx.read(None);
        assert!(tx.switch.is_none());
        {
            let streams = lock(&core.streams);
            assert_eq!(streams.phase(stream_key(Dir::Tx, 0)), Phase::Active {
                segment: Some((0, config(200), None)),
                delivered: None,
            }, "the current clock's segment");
        }
        let e = core.ticks(1_000_000_000);
        {
            let mut streams = lock(&core.streams);
            streams.stream_mut(stream_key(Dir::Tx, 0)).configs.insert(1, Arc::new(super::super::control::ColdConfig::new(BTreeMap::new())));
            assert!(streams.book(&core, stream_key(Dir::Tx, 0), Item { e, seq: 1, ready: 0, release: NO_RELEASE, delivered: None, refused: false, kind: Kind::Cold(config(100)) }));
        }
        let c1 = lock(&core.streams).stream(stream_key(Dir::Tx, 0)).clocks[1].0;
        tx.read(None);
        let switch = tx.switch.as_ref().expect("a switch");
        assert_eq!(switch.cut, e);
        assert_eq!(switch.to.as_ref().and_then(|to| to.clock), Some(c1));
        {
            let streams = lock(&core.streams);
            assert!(!streams.is_idle(stream_key(Dir::Tx, 0)), "a pending switch keeps the current segment active");
            assert_eq!(streams.active(stream_key(Dir::Tx, 0)), Some((0, config(200), None)));
        }
    }

    #[test]
    fn ur_25_a_transmit_read_sets_idle_pending_or_active() {
        // UR-25: a read with no clock and nothing to switch to leaves the owner idle; one
        // with a clock is active at the plan's current segment (none when the plan has none);
        // one with no clock but a switch leaves it pending the switch.
        let (core, _device, _, _) = super::super::test_support::rig();
        let key = stream_key(Dir::Tx, 0);
        let config = ezsdr_radio::timeline::Config { channels: Channels::first(1), ratio: ezsdr_kernel::time::Rational::new(200, 1).unwrap() };

        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        tx.read(None);
        assert_eq!(lock(&core.streams).phase(key), Phase::Idle, "nothing to begin");

        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        tx.read(None);
        assert_eq!(lock(&core.streams).phase(key), Phase::Active { segment: None, delivered: None }, "a clock with no planned segment");

        {
            let mut streams = lock(&core.streams);
            let lead = core.ticks(ezsdr_radio::timeline::start_lead_ns(&core.description.timing, ezsdr_kernel::stream::Direction::Tx));
            streams.lines.insert(key, ezsdr_radio::timeline::Line::new(ezsdr_radio::timeline::Stream {
                direction: ezsdr_kernel::stream::Direction::Tx, origin: 0, config, lead, call: 0, allowance: 0,
            }));
        }
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        tx.read(None);
        assert!(tx.switch.is_some(), "the line's first segment is the switch");
        assert_eq!(lock(&core.streams).phase(key), Phase::Pending, "a change waits for its switch");

        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        tx.read(None);
        let streams = lock(&core.streams);
        assert_eq!(streams.phase(key), Phase::Active { segment: Some((0, config, None)), delivered: None });
        assert_eq!(streams.first_undelivered(key), None, "a transmit stream has no floor");
        assert!(!streams.is_idle(key));
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
        let lead = core.ticks(ezsdr_radio::timeline::start_lead_ns(&core.description.timing, ezsdr_kernel::stream::Direction::Tx));
        streams.lines.insert(key, Line::new(Stream { direction: ezsdr_kernel::stream::Direction::Tx, origin: 0, config: config(200), lead, call: 0, allowance: 0 }));
        streams.stream_mut(key).configs.insert(1, Arc::new(super::super::control::ColdConfig::new(BTreeMap::new())));
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
                let lead = core.ticks(ezsdr_radio::timeline::start_lead_ns(&core.description.timing, ezsdr_kernel::stream::Direction::Tx));
                streams.lines.insert(key, Line::new(Stream { direction: ezsdr_kernel::stream::Direction::Tx, origin: 0, config: config(200), lead, call: 0, allowance: 0 }));
                for (seq, e, n) in [(1, s(1), 100), (2, s(2), 200), (3, s(3), 100)] {
                    streams.stream_mut(key).configs.insert(seq, Arc::new(super::super::control::ColdConfig::new(BTreeMap::new())));
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
            {
                let streams = lock(&core.streams);
                if refused {
                    assert_eq!(streams.phase(stream_key(Dir::Tx, 0)), Phase::Pending, "{case}: the refused change waits for the next one");
                } else {
                    let adopted = streams.planned(stream_key(Dir::Tx, 0))[1].segment;
                    assert_eq!(streams.phase(stream_key(Dir::Tx, 0)), Phase::Active {
                        segment: Some((adopted.origin, adopted.config, adopted.by)),
                        delivered: None,
                    }, "{case}: the adopted change is the active segment");
                }
            }
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
            {
                let streams = lock(&core.streams);
                let planned = streams.planned(stream_key(Dir::Tx, 0));
                let last = planned[3].segment;
                let second = planned[2].segment;
                assert_eq!(streams.phase(stream_key(Dir::Tx, 0)), Phase::Active {
                    segment: Some((last.origin, last.config, last.by)),
                    delivered: None,
                }, "{case}: the third change is active, not idle");
                assert!(streams.stream(stream_key(Dir::Tx, 0)).completed.contains(&(second.origin, second.config, second.by)), "{case}: the segment switched from is recorded");
            }
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
        tx.switch = Some(switch(&core, core.ticks(20_000_000), None, 0, Settings::default()));
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

    /// uhd-control on the transmit stream it begins at 1 MS/s from tick 0: the booking site
    /// whose on-time verdict a test reads beside uhd-tx's hand-over.
    fn booking_control(core: &Arc<Core>) -> (Control, std::sync::mpsc::Receiver<TxCmd>) {
        booking_control_at(core, 0)
    }

    fn booking_control_at(core: &Arc<Core>, t0: i64) -> (Control, std::sync::mpsc::Receiver<TxCmd>) {
        let clock = core.register(Dir::Tx, 200, t0).unwrap();
        booking_control_on(core, clock)
    }

    fn booking_control_on(core: &Arc<Core>, clock: Clock) -> (Control, std::sync::mpsc::Receiver<TxCmd>) {
        let (to_tx, handed) = std::sync::mpsc::channel();
        let mut start = super::super::control::Start {
            t0: clock.origin,
            rx: vec![None; core.description.stream_count(Dir::Rx)],
            tx: vec![None; core.description.stream_count(Dir::Tx)],
        };
        start.tx[0] = Some((clock, Channels::first(1)));
        (Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], core.description.defaults(), false, start), handed)
    }

    fn waveform_input(name: &str, samples: &[[f32; 2]]) -> (ezsdr_kernel::manifest::ArtifactRef, Vec<u8>) {
        let bytes: Vec<u8> = samples.iter().flat_map(|sample| sample.iter().flat_map(|value| value.to_le_bytes())).collect();
        let waveform = ezsdr_kernel::manifest::ingest_input(
            ezsdr_kernel::spec::Ident::parse(name).unwrap(),
            ezsdr_kernel::spec::Namespace::parse("ezsdr.input").unwrap(),
            format!("mem:{}", ContentHash::of_bytes(&bytes)),
            &bytes,
        );
        (waveform, bytes)
    }

    #[allow(clippy::too_many_arguments)]
    fn book_burst(control: &mut Control, core: &Core, id: u64, clock: Clock, k: i64, waveform: &ezsdr_kernel::manifest::ArtifactRef, repeat: bool, policy: LatePolicy) {
        book_burst_on(control, core, id, 0, clock, k, waveform, repeat, policy);
    }

    #[allow(clippy::too_many_arguments)]
    fn book_burst_on(control: &mut Control, core: &Core, id: u64, stream: usize, clock: Clock, k: i64, waveform: &ezsdr_kernel::manifest::ArtifactRef, repeat: bool, policy: LatePolicy) {
        let node = core.description.stream_node(&core.id, Dir::Tx, stream);
        control.book(Dispatched::new(ActionId(id), Action::TxBurst {
            target: super::super::test_support::target_of(&node),
            waveform: waveform.clone(),
            repeat,
            at: AbsoluteDeadline::new(TimePoint::new(clock.domain, k)),
            requested_at: None,
            late_policy: policy,
            metadata: BTreeMap::new(),
        }, Some(node)));
    }

    fn drain_tx(tx: &mut Tx) {
        while let Ok(command) = tx.cmds.try_recv() { tx.command(command); }
    }

    /// A reservation is not a hand-over; only `Tx::step` puts A into the device's front.
    #[test]
    fn ur_21_booking_receipt_order_changes_preemption() {
        let a_samples = vec![[0.25, 0.0]; 2_000];
        let b_samples: Vec<[f32; 2]> = (0..2_000).map(|n| [-0.5, n as f32 / 2_000.0]).collect();
        let (a_wave, a_bytes) = waveform_input("a", &a_samples);
        let (b_wave, b_bytes) = waveform_input("b", &b_samples);
        for (case, b_policy) in [
            ("booked together", LatePolicy::SendAsapAndFlag),
            ("handed at same instant", LatePolicy::SendAsapAndFlag),
            ("handed before later receipt", LatePolicy::SendAsapAndFlag),
            ("handed at same instant drop", LatePolicy::DropAndFlag),
        ] {
            let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
            let device = Arc::new(FakeDevice::new(config));
            let inputs = BTreeMap::from([
                (a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes.clone())),
                (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes.clone())),
            ]);
            let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
            let _frozen = crate::provider::test_support::Frozen::of(&device);
            device_at(&core, &time, &device, 0);
            device.tx_open(0, &[0]).unwrap();
            device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
            let (mut control, cmds) = booking_control(&core);
            let clock = plan_of(&core, Dir::Tx)[0].clock.expect("the transmit clock");
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            if case == "booked together" {
                book_burst(&mut control, &core, 1, clock, 4_000, &a_wave, true, LatePolicy::DropAndFlag);
                book_burst(&mut control, &core, 2, clock, 3_000, &b_wave, false, LatePolicy::SendAsapAndFlag);
                drain_tx(&mut tx);
                assert!(tx.step(), "{case}: B did not dispatch");
                assert_eq!(tx.continues_at, Some(4_000), "{case}: B's on-time burst reaches the next held start");
                assert!(tx.held.contains_key(&4_000), "{case}: A remains after B's earlier target");
                assert_eq!(sends(&device).first(), Some(&(1_000, true, false)), "{case}: B's timed SOB");
                assert_eq!(device.transmitted(clock.instant(3_000), 1_000), b_samples[..1_000].iter().copied().map(Some).collect::<Vec<_>>(), "{case}: B's samples");
                assert!(events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).is_empty(), "{case}: B is on time on UHD");
            } else {
                book_burst(&mut control, &core, 1, clock, 4_000, &a_wave, true, LatePolicy::DropAndFlag);
                drain_tx(&mut tx);
                assert!(tx.step(), "{case}: A did not hand over");
                assert_eq!(tx.open.as_ref().unwrap().next, 6_500, "{case}: A's cursor");
                if case == "handed before later receipt" {
                    device_at(&core, &time, &device, 1);
                }
                let calls = device.calls().len();
                book_burst(&mut control, &core, 2, clock, 3_000, &b_wave, false, b_policy);
                drain_tx(&mut tx);
                assert!(tx.step(), "{case}: B did not preempt");
                let late: Vec<_> = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
                    .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
                assert_eq!(late.len(), 1, "{case}: {late:?}");
                let outcome = if b_policy == LatePolicy::DropAndFlag { "drop" } else { "send_asap" };
                assert_eq!(late[0].payload["outcome"], outcome, "{case}");
                assert_eq!(late[0].payload["late_by_ns"], 3_500_000, "{case}");
                assert_eq!(late[0].payload["target"]["ticks"], 3_000, "{case}");
                if b_policy == LatePolicy::DropAndFlag {
                    assert_eq!(tx.open.as_ref().unwrap().next, 6_500, "{case}: A's cursor");
                    assert!(!tx.held.contains_key(&3_000), "{case}: the dropped B stayed held");
                    assert_eq!(device.calls().len(), calls, "{case}: the dropped B reached the device");
                    continue;
                }
                assert!(tx.continues_at == Some(6_500) && tx.held.contains_key(&6_500), "{case}: A's device burst was not continued: {:?}", tx.held.keys());
                assert_eq!(tx.held.get(&6_500).unwrap().open.requested_target, Some(TimePoint::new(clock.domain, 3_000)), "{case}: requested target");
                assert!(tx.step(), "{case}: B's continuation did not hand over");
                assert_eq!(device.transmitted(clock.instant(6_500), 1_999), b_samples[..1_999].iter().copied().map(Some).collect::<Vec<_>>(), "{case}: moved B samples");
                assert_eq!(sends(&device).last(), Some(&(1_999, false, false)), "{case}: B's continuation metadata");
            }
        }
    }

    #[test]
    fn ur_21_a_send_asap_move_is_reported_once_across_handovers() {
        let (a_wave, a_bytes) = waveform_input("one_sample_front", &[[0.25, 0.0]]);
        let (b_wave, b_bytes) = waveform_input("rechecked_start", &[[0.75, -0.25]]);
        let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let inputs = BTreeMap::from([
            (a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)),
            (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes)),
        ]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = plan_of(&core, Dir::Tx)[0].clock.expect("the transmit clock");
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 9_999, &a_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step(), "the one-sample predecessor was not handed over");
        assert_eq!(tx.front, Some((clock.domain, 10_001)), "the device EOB establishes the safe restart sample");

        book_burst(&mut control, &core, 2, clock, 9_999, &b_wave, false, LatePolicy::SendAsapAndFlag);
        drain_tx(&mut tx);
        let calls = device.calls().len();
        assert!(tx.step(), "the first late dispatch did not defer B past the window");
        assert!(tx.held.contains_key(&10_001), "B was not retained at its first safe start");
        let first: Vec<_> = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
            .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert_eq!(first.len(), 1, "{first:?}");
        assert_eq!(first[0].payload["outcome"], "send_asap");
        assert_eq!(first[0].payload["target"]["ticks"], 9_999);
        assert_eq!(device.calls().len(), calls, "B is still outside the hand-over window");

        device_at(&core, &time, &device, core.ticks(9_000_000));
        assert!(tx.step(), "B did not resume when its first moved start entered the window");
        assert!(events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter()
            .all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "the same action emitted a second TIME_ERROR");
        assert_eq!(device.transmitted(clock.instant(11_000), 1), vec![Some([0.75, -0.25])], "the rechecked start was moved to the current device lead");
    }

    #[test]
    fn ur_21_a_preempted_send_asap_move_keeps_one_error_and_its_requested_target() {
        let a_samples = vec![[0.25, 0.0]; 2_000];
        let (a_wave, a_bytes) = waveform_input("preempt_repeat", &a_samples);
        let (b_wave, b_bytes) = waveform_input("preempted_send_asap", &[[0.75, -0.25], [0.5, 0.125]]);
        let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let inputs = BTreeMap::from([
            (a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)),
            (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes)),
        ]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = plan_of(&core, Dir::Tx)[0].clock.expect("the transmit clock");
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 5_500, &a_wave, true, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        device_at(&core, &time, &device, core.ticks(3_000_000));
        while tx.open.as_ref().is_none_or(|open| open.next < 13_000) { assert!(tx.step(), "A did not reach cursor 13 000"); }
        assert_eq!(tx.open.as_ref().unwrap().next, 13_000, "A's first unhanded sample");

        book_burst(&mut control, &core, 2, clock, 1_000, &b_wave, false, LatePolicy::SendAsapAndFlag);
        drain_tx(&mut tx);
        assert!(tx.held.contains_key(&5_000), "control did not move B to sample 5 000: {:?}", tx.held.keys().collect::<Vec<_>>());
        assert!(tx.step(), "B did not preempt A's hand-over cursor");
        let errors: Vec<_> = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
            .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert_eq!(errors.len(), 1, "the control's first late decision is not repeated by preemption: {errors:?}");
        assert_eq!(errors[0].payload["outcome"], "send_asap");
        assert_eq!(errors[0].payload["target"]["ticks"], 1_000, "the original target is reported");
        assert!(tx.held.contains_key(&13_000), "preemption did not move B to sample 13 000");
        assert_eq!(tx.held[&13_000].open.requested_target, Some(TimePoint::new(clock.domain, 1_000)));

        device_at(&core, &time, &device, core.ticks(4_000_000));
        assert!(tx.step(), "the moved continuation was not sent");
        assert_eq!(device.transmitted(clock.instant(13_000), 1), vec![Some([0.75, -0.25])], "B starts at A's first unhanded sample");
        assert!(events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter()
            .all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "the already reported move emitted another TIME_ERROR");
    }

    #[test]
    fn ur_21_tx_front_is_per_stream() {
        let a_samples = vec![[0.25, 0.0]; 2_000];
        let b_samples = vec![[0.75, -0.25]; 2_000];
        let (a_wave, a_bytes) = waveform_input("stream0", &a_samples);
        let (b_wave, b_bytes) = waveform_input("stream1", &b_samples);
        let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let inputs = BTreeMap::from([
            (a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)),
            (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes)),
        ]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.tx_open(1, &[1]).unwrap();
        for channel in [0, 1] { device.apply(Dir::Tx, channel, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap(); }
        let c0 = core.register_in(Dir::Tx, 0, 200, 0).unwrap();
        let c1 = core.register_in(Dir::Tx, 1, 200, 0).unwrap();
        let (to0, cmds0) = std::sync::mpsc::channel();
        let (to1, cmds1) = std::sync::mpsc::channel();
        let start = super::super::control::Start {
            t0: 0,
            rx: vec![None; core.description.stream_count(Dir::Rx)],
            tx: vec![Some((c0, Channels::first(1))), Some((c1, Channels::first(1)))],
        };
        let mut control = Control::new(core.clone(), Arc::new(NoActions), vec![to0, to1], core.description.defaults(), false, start);
        let mut tx0 = Tx::new(core.clone(), 0, cmds0, Some(c0), Channels::first(1));
        let mut tx1 = Tx::new(core.clone(), 1, cmds1, Some(c1), Channels::first(1));
        book_burst_on(&mut control, &core, 1, 0, c0, 4_000, &a_wave, true, LatePolicy::DropAndFlag);
        book_burst_on(&mut control, &core, 2, 1, c1, 3_000, &b_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx0);
        drain_tx(&mut tx1);
        assert!(tx0.step() && tx1.step());
        assert_eq!(tx1.continues_at, Some(5_000), "stream 0's hand-over front did not leak");
        assert!(device.calls().iter().any(|call| call.starts_with("tx_send 1 ")));
        assert!(device.calls().iter().any(|call| {
            call.starts_with("tx_send 1 ") && call.contains(&format!("at={} sob=true", c1.instant(3_000)))
        }), "stream 1's B did not start at its own target: {:?}", device.calls());
        assert!(events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0))
            .iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR));
    }

    #[test]
    fn ur_25_cold_change_does_not_inherit_old_tx_front() {
        let t0 = 2_000_000_000;
        let a_samples = vec![[0.25, 0.0]; 10_000];
        let b_samples = vec![[0.75, -0.25]; 2_000];
        let (a_wave, a_bytes) = waveform_input("old_clock", &a_samples);
        let (b_wave, b_bytes) = waveform_input("new_clock", &b_samples);
        let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let inputs = BTreeMap::from([
            (a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)),
            (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes)),
        ]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, core.ticks(t0));
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let (mut control, cmds) = booking_control_at(&core, core.ticks(t0));
        let old = plan_of(&core, Dir::Tx)[0].clock.unwrap();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(old), Channels::first(1));
        book_burst(&mut control, &core, 1, old, 2_000, &a_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        while tx.step() {}
        assert_eq!(tx.open.as_ref().unwrap().next, 9_500, "the old clock's first unhanded sample");
        device_at(&core, &time, &device, core.ticks(t0 + 8_000_000));
        assert!(tx.step(), "the finite old-clock burst's last buffer was not handed");
        device_at(&core, &time, &device, core.ticks(t0 + 10_000_000));
        assert!(tx.step(), "the finite old-clock burst did not end");
        assert_eq!(tx.front, Some((old.domain, 12_001)));

        control.config.insert(crate::provider::core::key("radio.tx.c0.stream"), Value::from(0));
        control.book(Dispatched::new(ActionId(3), Action::UpdateParameter {
            target: super::super::test_support::target_of(&core.id),
            key: crate::provider::core::key("radio.tx.s0.sample_rate_hz"),
            value: Value::num(2_000_000.0).unwrap(),
            class: UpdateClass::Cold,
            at: Some(AbsoluteDeadline::new(core.at(core.ticks(t0 + 15_000_000)))),
        }, Some(core.id.clone())));
        assert_eq!(control.config_at(core.ticks(t0 + 15_000_000))[&crate::provider::core::key("radio.tx.s0.sample_rate_hz")], Value::num(2_000_000.0).unwrap());
        assert_eq!(plan_of(&core, Dir::Tx).len(), 2, "the cold clock was not booked: {:?}", lock(&core.rec).rejected);
        assert!(tx.read(None));
        assert!(tx.switch.is_some(), "the owner did not read the cut");
        let switch = tx.switch.as_ref().unwrap().cut;
        device_at(&core, &time, &device, switch);
        assert!(tx.step(), "the transmit owner did not switch clocks");
        let new = tx.clock.expect("the new transmit clock");
        assert_ne!(new.domain, old.domain);
        let target = 6_000;
        let old_front = tx.front.unwrap().1;
        let target_root = core.clocks.convert(TimePoint::new(new.domain, target), core.root).unwrap().floor().ticks_in(core.root).unwrap();
        let old_front_root = core.clocks.convert(TimePoint::new(old.domain, old_front), core.root).unwrap().floor().ticks_in(core.root).unwrap();
        assert!(target_root > old_front_root, "the new target is later in root time; SampleClock ticks are not compared");
        book_burst(&mut control, &core, 4, new, target, &b_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        let now = new.instant(target) - core.ticks(2_000_000);
        assert!(now >= switch, "the new-clock dispatch must not move device time backwards");
        device_at(&core, &time, &device, now);
        assert!(tx.step(), "the new-clock burst did not hand over");
        assert!(events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0))
            .iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR));
        assert!(device.calls().iter().any(|call| call.contains(&format!("at={}", new.instant(target))) && call.contains("sob=true")), "the old clock's front was ignored: {:?}", device.calls());
    }

    #[test]
    fn ur_21_a_target_after_a_completed_burst_is_on_time() {
        let a_samples = vec![[0.25, 0.0]; 2_000];
        let b_samples = vec![[0.75, -0.25]; 1_000];
        let (a_wave, a_bytes) = waveform_input("expired_a", &a_samples);
        let (b_wave, b_bytes) = waveform_input("expired_b", &b_samples);
        let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let inputs = BTreeMap::from([
            (a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)),
            (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes)),
        ]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = plan_of(&core, Dir::Tx)[0].clock.unwrap();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 2_000, &a_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step());
        device_at(&core, &time, &device, core.ticks(2_000_000));
        assert!(tx.step(), "A's EOB was not sent");
        assert_eq!(tx.front, Some((clock.domain, 4_001)));
        device_at(&core, &time, &device, core.ticks(10_000_000));
        book_burst(&mut control, &core, 2, clock, 13_000, &b_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step(), "the burst after the front's expiry did not dispatch");
        assert!(events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0))
            .iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR));
        assert!(device.calls().iter().any(|call| call.contains(&format!("at={}", clock.instant(13_000))) && call.contains("sob=true")));
    }

    #[test]
    fn ur_21_send_does_not_cross_the_in_flight_window() {
        let a_samples = vec![[0.25, 0.0]; 30_000];
        let b_samples = vec![[0.75, -0.25], [0.5, 0.25]];
        let (a_wave, a_bytes) = waveform_input("window_repeat", &a_samples);
        let (b_wave, b_bytes) = waveform_input("window_boundary", &b_samples);
        for (rate, root_ticks_per_sample, start, expected_cut) in [
            (1e6, 200, 9_000, 10_000),
            (390_625.0, 512, 979, 3_907),
        ] {
            let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
            let device = Arc::new(FakeDevice::new(config));
            let inputs = BTreeMap::from([
                (a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes.clone())),
                (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes.clone())),
            ]);
            let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
            let _frozen = crate::provider::test_support::Frozen::of(&device);
            device_at(&core, &time, &device, 0);
            device.tx_open(0, &[0]).unwrap();
            device.apply(Dir::Tx, 0, &Settings { rate: Some(rate), ..Settings::default() }, None).unwrap();
            let clock = core.register(Dir::Tx, root_ticks_per_sample, 0).unwrap();
            let (mut control, cmds) = booking_control_on(&core, clock);
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            book_burst(&mut control, &core, 1, clock, start, &a_wave, true, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            while tx.step() {}
            let case = format!("{rate} samples/s");
            assert_eq!(tx.open.as_ref().unwrap().next, expected_cut, "{case}: exclusive first-unhanded sample");
            let handed = (expected_cut - start) as usize;
            assert_eq!(sends(&device).iter().map(|(n, _, _)| n).sum::<usize>(), handed, "{case}: total payload handed");
            assert_eq!(device.transmitted(clock.instant(start), handed), vec![Some([0.25, 0.0]); handed], "{case}: payload values");
            assert_eq!(device.transmitted(clock.instant(expected_cut), 1), vec![None], "{case}: the boundary sample is not handed");
            if rate == 390_625.0 {
                let calls = device.calls().len();
                book_burst(&mut control, &core, 2, clock, 3_906, &b_wave, false, LatePolicy::DropAndFlag);
                drain_tx(&mut tx);
                assert!(tx.step(), "the sample before the cursor was not decided");
                let late: Vec<_> = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
                    .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
                assert_eq!(late.len(), 1);
                assert_eq!(late[0].payload["outcome"], "drop");
                assert_eq!(device.calls().len(), calls, "the late B reached the device");
                book_burst(&mut control, &core, 3, clock, 3_907, &b_wave, false, LatePolicy::DropAndFlag);
                drain_tx(&mut tx);
                device_at(&core, &time, &device, 1_000);
                assert!(tx.step(), "the first unhanded sample was not admitted");
                assert!(tx.step(), "the boundary burst did not continue");
                assert!(events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter()
                    .all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "the cursor boundary itself is on time");
                assert_eq!(device.transmitted(clock.instant(3_907), 1), vec![Some([0.75, -0.25])]);
            } else {
                book_burst(&mut control, &core, 2, clock, expected_cut, &b_wave, false, LatePolicy::DropAndFlag);
                drain_tx(&mut tx);
                device_at(&core, &time, &device, 201);
                assert!(tx.step(), "the aligned boundary burst was not admitted");
                assert!(tx.step(), "the aligned boundary burst did not hand over");
                assert!(events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter()
                    .all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "the +10 ms sample boundary is on time");
                assert_eq!(device.transmitted(clock.instant(expected_cut), 1), vec![Some([0.75, -0.25])]);
            }
        }
    }

    #[test]
    fn ur_21_tx_reads_in_flight_window_from_description() {
        let samples = vec![[0.25, 0.0]; 10_000];
        let (wave, bytes) = waveform_input("custom_window", &samples);
        let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let mut description = super::super::test_support::default_description(2_000);
        description.timing.tx_in_flight_ns = 4_000_000;
        let (core, time, _) = super::super::test_support::rig_on_description(device.clone(), Vec::new(), BTreeMap::from([
            (wave.hash.clone(), Arc::<[u8]>::from(bytes.clone())),
        ]), description);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (mut control, cmds) = booking_control_on(&core, clock);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 3_000, &wave, true, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step());
        assert_eq!(tx.open.as_ref().unwrap().next, 4_000);
        assert_eq!(sends(&device).iter().map(|(n, _, _)| n).sum::<usize>(), 1_000);

        let device = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() }));
        let mut description = super::super::test_support::default_description(2_000);
        description.timing.tx_in_flight_ns = 4_000_000;
        let (core, time, _) = super::super::test_support::rig_on_description(device.clone(), Vec::new(), BTreeMap::from([
            (wave.hash.clone(), Arc::<[u8]>::from(bytes)),
        ]), description);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (mut control, cmds) = booking_control_on(&core, clock);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 5_000, &wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(!tx.step(), "a burst at +5 ms is outside the custom 4 ms window");
        assert!(tx.held.contains_key(&5_000));
        assert!(device.calls().iter().all(|call| !call.starts_with("tx_send ")), "the outside-window burst reached the device: {:?}", device.calls());
    }

    #[test]
    fn measurement_candidate_in_flight_window_sets_the_tx_chunk_size() {
        let samples = vec![[0.25, 0.0]; 10_000];
        let (wave, bytes) = waveform_input("candidate_chunk", &samples);
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() }));
        let mut description = super::super::test_support::default_description(10_000);
        description.timing.tx_in_flight_ns = 8_000_000;
        let (core, time, _) = super::super::test_support::rig_on_description(device.clone(), Vec::new(), BTreeMap::from([
            (wave.hash.clone(), Arc::<[u8]>::from(bytes)),
        ]), description);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (mut control, cmds) = booking_control_on(&core, clock);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 3_000, &wave, true, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step());
        assert_eq!(tx.open.as_ref().unwrap().next, 5_000, "the candidate 8 ms window hands over its 2 ms chunk");
        assert_eq!(sends(&device).iter().map(|(n, _, _)| n).sum::<usize>(), 2_000);
    }

    #[test]
    fn measurement_candidate_admission_lead_does_not_override_the_device_lead() {
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() }));
        let mut description = super::super::test_support::default_description(2_000);
        description.timing.min_timed_command_lead_ns = 1_000_000;
        let (core, _time, events) = super::super::test_support::rig_on_description(device.clone(), Vec::new(), BTreeMap::new(), description);
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, commands) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core, 0, commands, Some(clock), Channels::first(1));
        let target = clock.at_or_after(tx.core.now() + tx.core.ticks(1_500_000));
        tx.held.insert(target, held(&clock, target, 100, LatePolicy::DropAndFlag));
        assert!(tx.step(), "the queued burst is evaluated by the Provider");
        assert!(tx.open.is_none(), "the burst inside the device lead is dropped");
        let errors: Vec<_> = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
            .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].payload["outcome"], "drop");
        assert!(device.calls().iter().all(|call| !call.starts_with("tx_send ")), "a dropped burst reached the device: {:?}", device.calls());
    }

    #[test]
    fn ur_21_a_new_burst_without_an_open_predecessor_is_on_time() {
        let samples = vec![[0.75, -0.25]];
        let (wave, bytes) = waveform_input("alone", &samples);
        let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, BTreeMap::from([
            (wave.hash.clone(), Arc::<[u8]>::from(bytes)),
        ]));
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = plan_of(&core, Dir::Tx)[0].clock.unwrap();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 5_000, &wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step());
        assert!(events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).is_empty());
        assert_eq!(sends(&device).last(), Some(&(1, true, true)));
        assert_eq!(device.transmitted(clock.instant(5_000), 1), vec![Some([0.75, -0.25])]);
    }

    #[test]
    fn ur_21_a_preemption_front_stops_at_a_finite_end() {
        let a_samples = vec![[0.25, 0.0]; 2_000];
        let b_samples = vec![[0.75, -0.25]];
        let (a_wave, a_bytes) = waveform_input("finite_predecessor", &a_samples);
        let (b_wave, b_bytes) = waveform_input("after_finite", &b_samples);
        let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let inputs = BTreeMap::from([
            (a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)),
            (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes)),
        ]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = plan_of(&core, Dir::Tx)[0].clock.unwrap();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 3_000, &a_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step());
        assert!(tx.open.is_none() && tx.continues_at == Some(5_000));
        device_at(&core, &time, &device, core.ticks(3_000_000));
        assert!(tx.step(), "the final sample and EOB were not handed");
        assert_eq!(tx.front, Some((clock.domain, 5_001)));
        book_burst(&mut control, &core, 2, clock, 6_000, &b_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step());
        assert!(events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).is_empty());
        assert!(device.calls().iter().any(|call| call.contains(&format!("at={}", clock.instant(6_000))) && call.contains("sob=true")));
    }

    #[test]
    fn ur_21_a_finite_burst_inside_the_window_is_late() {
        let a_samples = vec![[0.25, 0.0]; 27_000];
        let b_samples = vec![[0.75, -0.25]];
        let (a_wave, a_bytes) = waveform_input("long_finite", &a_samples);
        let (b_wave, b_bytes) = waveform_input("inside_long_finite", &b_samples);
        let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let inputs = BTreeMap::from([
            (a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)),
            (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes)),
        ]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = plan_of(&core, Dir::Tx)[0].clock.unwrap();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 2_500, &a_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        while tx.open.as_ref().is_none_or(|open| open.next < 10_000) { assert!(tx.step()); }
        assert_eq!(tx.open.as_ref().unwrap().next, 10_000, "finite A is clamped by W, not its end");
        let calls = device.calls().len();
        book_burst(&mut control, &core, 2, clock, 5_000, &b_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step());
        let late: Vec<_> = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
            .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert_eq!(late.len(), 1);
        assert_eq!(late[0].payload["outcome"], "drop");
        assert_eq!(late[0].payload["late_by_ns"], 5_000_000);
        assert_eq!(device.calls().len(), calls, "B reached the device after DropAndFlag");
    }

    #[test]
    fn ur_21_a_handed_repeat_sets_the_preemption_front() {
        let a_samples = vec![[0.25, 0.0]; 2_000];
        let b_samples = vec![[0.75, -0.25]; 2_000];
        let (a_wave, a_bytes) = waveform_input("held_repeat", &a_samples);
        let (b_wave, b_bytes) = waveform_input("inside_held_repeat", &b_samples);
        let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let inputs = BTreeMap::from([
            (a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)),
            (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes)),
        ]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = plan_of(&core, Dir::Tx)[0].clock.unwrap();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 2_500, &a_wave, true, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        while tx.open.as_ref().is_none_or(|open| open.next < 10_000) { assert!(tx.step()); }
        assert_eq!(tx.open.as_ref().unwrap().next, 10_000);
        book_burst(&mut control, &core, 2, clock, 6_000, &b_wave, false, LatePolicy::SendAsapAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step(), "B did not preempt the hand-over cursor");
        let late: Vec<_> = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
            .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert_eq!(late.len(), 1);
        assert_eq!(late[0].payload["outcome"], "send_asap");
        assert_eq!(late[0].payload["late_by_ns"], 4_000_000);
        assert_eq!(late[0].payload["target"]["ticks"], 6_000);
        assert_eq!(tx.continues_at, Some(10_000));
        assert_eq!(tx.held[&10_000].open.requested_target, Some(TimePoint::new(clock.domain, 6_000)));
        // A whole buffer of B fits (UR-23: a continuation waits for one).
        device_at(&core, &time, &device, core.ticks(2_000_000));
        assert!(tx.step(), "the moved continuation did not hand over");
        assert_eq!(device.transmitted(clock.instant(10_000), 1), vec![Some([0.75, -0.25])]);
        assert_eq!(sends(&device).iter().filter(|(_, sob, _)| *sob).count(), 1);
    }

    #[test]
    fn ur_21_a_late_move_past_the_cursor_closes_before_a_held_burst() {
        let a_samples = vec![[0.25, 0.0]; 2_000];
        let (a_wave, a_bytes) = waveform_input("cursor_repeat", &a_samples);
        let (c_wave, c_bytes) = waveform_input("at_cursor", &[[0.5, 0.25]]);
        let b_samples = vec![[0.75, -0.25]; 2_000];
        let (b_wave, b_bytes) = waveform_input("moved_after_cursor", &b_samples);
        let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let inputs = BTreeMap::from([
            (a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)),
            (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes)),
            (c_wave.hash.clone(), Arc::<[u8]>::from(c_bytes)),
        ]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = plan_of(&core, Dir::Tx)[0].clock.expect("the transmit clock");
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 2_500, &a_wave, true, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        while tx.open.as_ref().is_none_or(|open| open.next < 10_000) { assert!(tx.step()); }
        assert_eq!(tx.open.as_ref().unwrap().next, 10_000);

        book_burst(&mut control, &core, 2, clock, 10_000, &c_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        book_burst(&mut control, &core, 3, clock, 5_000, &b_wave, false, LatePolicy::SendAsapAndFlag);
        drain_tx(&mut tx);
        device_at(&core, &time, &device, core.ticks(9_000_000));
        assert!(tx.step(), "the earlier B was not rechecked against the now-later ordinary bound");
        assert!(tx.open.is_none() && tx.continues_at.is_none());
        assert_eq!(tx.front, Some((clock.domain, 10_002)), "A closes before the moved B");
        assert_eq!(sends(&device).last(), Some(&(1, false, true)), "A's open device burst is ended at its cursor");
        let moved = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
            .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect::<Vec<_>>();
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0].payload["outcome"], "send_asap");
        assert_eq!(moved[0].payload["target"]["ticks"], 5_000);
        assert!(tx.held.contains_key(&10_000) && tx.held.contains_key(&11_000));

        assert!(tx.step(), "C at the cursor was not judged after A's device EOB");
        let dropped = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
            .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect::<Vec<_>>();
        assert_eq!(dropped.len(), 1);
        assert_eq!(dropped[0].payload["outcome"], "drop");
        assert_eq!(dropped[0].payload["target"]["ticks"], 10_000);

        assert!(tx.step(), "the moved B was not dispatched after C was dropped");
        assert!(events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).is_empty());
        assert!(device.calls().iter().any(|call| call.contains(&format!("at={} sob=true", clock.instant(11_000)))), "B starts as a new timed device burst");
        device_at(&core, &time, &device, clock.instant(11_001));
        assert_eq!(device.transmitted(clock.instant(11_000), 1), vec![Some([0.75, -0.25])]);
    }

    #[test]
    fn ur_21_a_preemption_front_boundary_is_on_time() {
        let a_samples = vec![[0.25, 0.0]; 2_000];
        let b_samples = vec![[0.75, -0.25], [0.5, 0.25]];
        let (a_wave, a_bytes) = waveform_input("boundary_repeat", &a_samples);
        let (b_wave, b_bytes) = waveform_input("boundary_b", &b_samples);
        for delta in 0..=1 {
            let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
            let device = Arc::new(FakeDevice::new(config));
            let inputs = BTreeMap::from([
                (a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes.clone())),
                (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes.clone())),
            ]);
            let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
            let _frozen = crate::provider::test_support::Frozen::of(&device);
            device_at(&core, &time, &device, 0);
            device.tx_open(0, &[0]).unwrap();
            device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
            let (mut control, cmds) = booking_control(&core);
            let clock = plan_of(&core, Dir::Tx)[0].clock.unwrap();
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            book_burst(&mut control, &core, 1, clock, 2_500, &a_wave, true, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            while tx.open.as_ref().is_none_or(|open| open.next < 10_000) { assert!(tx.step()); }
            assert_eq!(tx.open.as_ref().unwrap().next, 10_000);
            let target = 10_000 + delta;
            book_burst(&mut control, &core, 2, clock, target, &b_wave, false, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            device_at(&core, &time, &device, 401);
            assert!(tx.step(), "A did not reach B's boundary");
            if delta == 1 {
                assert_eq!(tx.continues_at, Some(10_001), "A's device burst stays open through the target boundary");
                assert!(tx.step(), "B at {target} was not admitted at equality");
            } else {
                assert!(tx.step(), "B at {target} did not continue");
            }
            assert!(events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0))
                .iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "B at {target} is on time");
            assert_eq!(device.transmitted(clock.instant(target), 1), vec![Some([0.75, -0.25])]);
        }
    }

    #[test]
    fn ur_23_stop_preserves_handed_cursor_after_stop() {
        let a_samples = vec![[0.25, 0.0]; 2_000];
        let b_samples = vec![[0.75, -0.25]];
        let (a_wave, a_bytes) = waveform_input("repeat", &a_samples);
        let (b_wave, b_bytes) = waveform_input("after_stop", &b_samples);
        for delta in 0..=2 {
            let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
            let device = Arc::new(FakeDevice::new(config));
            let inputs = BTreeMap::from([
                (a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes.clone())),
                (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes.clone())),
            ]);
            let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
            let _frozen = crate::provider::test_support::Frozen::of(&device);
            device_at(&core, &time, &device, 0);
            device.tx_open(0, &[0]).unwrap();
            device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
            let (mut control, cmds) = booking_control(&core);
            let clock = plan_of(&core, Dir::Tx)[0].clock.expect("the transmit clock");
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            book_burst(&mut control, &core, 1, clock, 2_500, &a_wave, true, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            while tx.open.as_ref().is_none_or(|open| open.next < 10_000) {
                assert!(tx.step(), "A stopped making progress before the 10 ms hand-over frontier");
            }
            assert_eq!(tx.open.as_ref().unwrap().next, 10_000, "the future A is handed to H before Stop");
            control.book(Dispatched::new(ActionId(2), Action::Stop {
                target: Some(super::super::test_support::target_of(&core.tx_id)),
            }, Some(core.tx_id.clone())));
            drain_tx(&mut tx);
            let record = lock(&core.rec).bursts.last().cloned().expect("the stopped repeat record");
            assert_eq!(record["end"], "stop");
            assert_eq!(record["samples"], 7_500);
            assert_eq!(sends(&device).last(), Some(&(1, false, true)), "Stop closes the empty EOB at H");

            let target = 10_000 + delta;
            let before = device.calls().len();
            device_at(&core, &time, &device, core.ticks((delta + 1) * 1_000 + 1));
            book_burst(&mut control, &core, 3 + delta as u64, clock, target, &b_wave, false, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            assert!(tx.step(), "B at {target} was not considered");
            let errors: Vec<_> = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
                .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
            if delta < 2 {
                assert_eq!(errors.len(), 1, "B at {target}: {errors:?}");
                assert_eq!(errors[0].payload["outcome"], "drop");
                assert_eq!(errors[0].payload["late_by_ns"], (2 - delta) * 1_000);
                assert_eq!(device.calls().len(), before, "a dropped B reached the device");
            } else {
                assert!(errors.is_empty(), "the first safe sample is on time: {errors:?}");
                assert_eq!(sends(&device).last(), Some(&(1, true, true)), "B starts with a timed SOB after H+2");
                assert_eq!(device.transmitted(clock.instant(10_000), 3), vec![Some([0.0, 0.0]), None, Some([0.75, -0.25])]);
                device_at(&core, &time, &device, clock.instant(10_003));
                tx.reports(Wall::ZERO);
                assert!(lock(&core.rec).device_async.iter().any(|row| row["code"] == "BurstAck" && row["tick"] == clock.instant(10_003)), "the safe burst's acknowledgement is read");
            }
        }
    }

    #[test]
    fn ur_23_stop_does_not_send_unhanded_window_edge_payload() {
        let (a_wave, a_bytes) = waveform_input("window_edge_a", &[[0.5, 0.0]]);
        let (b_wave, b_bytes) = waveform_input("window_edge_b", &[[0.0, 0.5]]);
        let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let inputs = BTreeMap::from([
            (a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)),
            (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes)),
        ]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = plan_of(&core, Dir::Tx)[0].clock.expect("the transmit clock");
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 10_000, &b_wave, false, LatePolicy::DropAndFlag);
        book_burst(&mut control, &core, 2, clock, 9_999, &a_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step(), "the one-sample A at 9 999 was not handed over");
        assert_eq!(sends(&device).last(), Some(&(1, true, false)), "A stays open for the held burst at the window edge");
        tx.stop_now("test Stop", ezsdr_radio::CancelReason::Stop);
        assert_eq!(sends(&device).last(), Some(&(1, false, true)), "Stop closes the open device burst with the empty-EOB padding sample");

        assert_eq!(lock(&core.rec).bursts.len(), 1, "only A has a burst record");
        assert_eq!(lock(&core.rec).bursts[0]["samples"], 1);
        assert_eq!(device.transmitted(clock.instant(9_999), 2), vec![Some([0.5, 0.0]), Some([0.0, 0.0])], "FakeDevice received only A and the empty-EOB padding, not B");

        let before = device.calls().len();
        device_at(&core, &time, &device, core.ticks(2_001));
        book_burst(&mut control, &core, 3, clock, 10_001, &b_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step(), "C at H+1 was not considered");
        let errors: Vec<_> = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
            .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert_eq!(errors.len(), 1, "C at H+1 is late: {errors:?}");
        assert_eq!(errors[0].payload["outcome"], "drop");
        assert_eq!(errors[0].payload["late_by_ns"], 1_000);
        assert_eq!(device.calls().len(), before, "late C reached FakeDevice");

        device_at(&core, &time, &device, core.ticks(3_001));
        book_burst(&mut control, &core, 4, clock, 10_002, &b_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step(), "C at H+2 was not considered");
        assert!(events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0))
            .iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "C at H+2 is on time");
        assert_eq!(sends(&device).last(), Some(&(1, true, true)), "C starts a new device burst at H+2");
        assert_eq!(device.transmitted(clock.instant(10_000), 3), vec![Some([0.0, 0.0]), None, Some([0.0, 0.5])]);
    }

    #[test]
    fn ur_23_stop_closes_a_running_repeat_at_its_handed_cursor() {
        let samples = vec![[0.25, 0.0]; 2_000];
        let (wave, bytes) = waveform_input("running_repeat", &samples);
        let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let (core, time, _) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, BTreeMap::from([
            (wave.hash.clone(), Arc::<[u8]>::from(bytes)),
        ]));
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = plan_of(&core, Dir::Tx)[0].clock.expect("the transmit clock");
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 2_000, &wave, true, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step());
        device_at(&core, &time, &device, core.ticks(4_000_000));
        while tx.open.as_ref().is_some_and(|open| open.next < 12_000) { assert!(tx.step()); }
        assert_eq!(tx.open.as_ref().unwrap().next, 12_000);
        control.book(Dispatched::new(ActionId(2), Action::Stop {
            target: Some(super::super::test_support::target_of(&core.tx_id)),
        }, Some(core.tx_id.clone())));
        drain_tx(&mut tx);
        let record = lock(&core.rec).bursts.last().cloned().unwrap();
        assert_eq!(record["end"], "stop");
        assert_eq!(record["samples"], 10_000);
        assert_eq!(device.transmitted(clock.instant(2_000), 10_002), vec![Some([0.25, 0.0]); 10_000]
            .into_iter().chain([Some([0.0, 0.0]), None]).collect::<Vec<_>>());
    }

    #[test]
    fn ur_21_a_finite_burst_with_a_held_tail_keeps_its_front() {
        let a_samples = vec![[0.25, 0.0]; 6_000];
        let b_samples: Vec<[f32; 2]> = (0..1_000).map(|n| [-0.5, n as f32 / 1_000.0]).collect();
        let (a_wave, a_bytes) = waveform_input("finite_a", &a_samples);
        let (b_wave, b_bytes) = waveform_input("finite_b", &b_samples);
        let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let inputs = BTreeMap::from([
            (a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)),
            (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes)),
        ]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = plan_of(&core, Dir::Tx)[0].clock.expect("the transmit clock");
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 2_000, &a_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        for _ in 0..3 { assert!(tx.step(), "A did not hand over its three chunks"); }
        assert!(tx.open.is_none() && tx.continues_at == Some(8_000) && tx.tail.as_ref().is_some_and(|tail| tail.channels == 1), "A's last sample is held: {:?}", device.calls());

        book_burst(&mut control, &core, 2, clock, 5_000, &b_wave, false, LatePolicy::SendAsapAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step(), "the late continuation was not decided");
        let late: Vec<_> = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
            .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert_eq!(late.len(), 1);
        assert_eq!(late[0].payload["outcome"], "send_asap");
        assert_eq!(late[0].payload["late_by_ns"], 3_000_000);
        assert_eq!(late[0].payload["target"]["ticks"], 5_000);
        assert!(tx.continues_at == Some(8_000) && tx.held.contains_key(&8_000), "B moves to A's exclusive end");
        assert_eq!(tx.held[&8_000].open.requested_target, Some(TimePoint::new(clock.domain, 5_000)));
        assert!(tx.step(), "B did not continue A's open device burst");
        let mut expected = vec![Some([0.25, 0.0]); 2];
        expected.extend(b_samples[..4].iter().copied().map(Some));
        assert_eq!(device.transmitted(clock.instant(7_998), expected.len()), expected, "A's held sample appears once before B");
        assert_eq!(sends(&device).iter().filter(|(_, sob, _)| *sob).count(), 1, "no timed SOB splits the continuation");
        assert_eq!(sends(&device).iter().filter(|(_, _, eob)| *eob).count(), 0, "the device burst remains open through B");
    }

    #[test]
    fn ur_23_a_one_sample_burst_has_a_one_sample_safe_restart() {
        let (a_wave, a_bytes) = waveform_input("one_sample_eob", &[[0.25, 0.0]]);
        let (b_wave, b_bytes) = waveform_input("one_sample_successor", &[[0.75, -0.25]]);
        for (target, expect_late) in [(2_001, true), (2_002, false)] {
            let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
            let device = Arc::new(FakeDevice::new(config));
            let inputs = BTreeMap::from([
                (a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes.clone())),
                (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes.clone())),
            ]);
            let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
            let _frozen = crate::provider::test_support::Frozen::of(&device);
            device_at(&core, &time, &device, 0);
            device.tx_open(0, &[0]).unwrap();
            device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
            let (mut control, cmds) = booking_control(&core);
            let clock = plan_of(&core, Dir::Tx)[0].clock.expect("the transmit clock");
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            book_burst(&mut control, &core, 1, clock, 2_000, &a_wave, false, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            assert!(tx.step(), "A's one-sample device burst was not ended");
            assert_eq!(tx.front, Some((clock.domain, 2_002)), "A's EOB leaves one safe sample");
            assert_eq!(sends(&device).first(), Some(&(1, true, true)));

            book_burst(&mut control, &core, 2, clock, target, &b_wave, false, LatePolicy::SendAsapAndFlag);
            drain_tx(&mut tx);
            assert!(tx.step(), "B was not dispatched");
            let errors: Vec<_> = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
                .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
            if expect_late {
                assert_eq!(errors.len(), 1, "{errors:?}");
                assert_eq!(errors[0].payload["outcome"], "send_asap");
                assert_eq!(errors[0].payload["late_by_ns"], 1_000);
                assert_eq!(errors[0].payload["target"]["ticks"], target);
            } else {
                assert!(errors.is_empty(), "{errors:?}");
            }
            assert_eq!(sends(&device).last(), Some(&(1, true, true)));
            assert_eq!(device.transmitted(clock.instant(2_002), 1), vec![Some([0.75, -0.25])], "B's actual start");
        }
    }

    #[test]
    fn ur_21_a_post_eob_restart_uses_the_safe_sample() {
        let a_samples = vec![[0.25, 0.0]; 6_000];
        let b_samples = vec![[0.75, -0.25]];
        let (a_wave, a_bytes) = waveform_input("eob_a", &a_samples);
        let (b_wave, b_bytes) = waveform_input("eob_b", &b_samples);
        let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let inputs = BTreeMap::from([
            (a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)),
            (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes)),
        ]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = plan_of(&core, Dir::Tx)[0].clock.expect("the transmit clock");
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 2_000, &a_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        for _ in 0..3 { assert!(tx.step()); }
        assert!(tx.open.is_none() && tx.continues_at == Some(8_000) && tx.tail.is_some());
        device_at(&core, &time, &device, core.ticks(6_000_000));
        assert!(tx.step(), "the held last sample was not closed with EOB");
        assert_eq!(tx.front, Some((clock.domain, 8_001)));
        assert_eq!(sends(&device).last(), Some(&(1, false, true)));

        book_burst(&mut control, &core, 2, clock, 8_000, &b_wave, false, LatePolicy::SendAsapAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step(), "B did not dispatch after A's EOB");
        let late: Vec<_> = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
            .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert_eq!(late.len(), 1);
        assert_eq!(late[0].payload["outcome"], "send_asap");
        assert_eq!(late[0].payload["late_by_ns"], 1_000);
        assert_eq!(late[0].payload["target"]["ticks"], 8_000);
        assert_eq!(sends(&device).last(), Some(&(1, true, true)), "B uses a timed SOB at end + 1 sample");
        assert_eq!(device.transmitted(clock.instant(7_998), 4), vec![Some([0.25, 0.0]), Some([0.25, 0.0]), None, Some([0.75, -0.25])]);
        device_at(&core, &time, &device, clock.instant(8_002));
        tx.reports(Wall::ZERO);
        assert!(lock(&core.rec).device_async.iter().all(|row| row["code"] != "Underflow" && row["code"] != "TimeError"));
        assert!(lock(&core.rec).device_async.iter().any(|row| row["code"] == "BurstAck" && row["tick"] == clock.instant(8_002)));
    }

    #[test]
    fn ur_21_a_burst_inside_the_in_flight_window_is_late() {
        // RM-15 as VE-2 amends it, at the receipt site and at uhd-tx's first hand-over: a
        // burst 5 ms ahead is beyond the 2 ms device lead, so the booking hands it over
        // unchanged; once the open burst's next unhanded sample has passed that start, the
        // hand-over decides it again — moved under `send_asap`, dropped under `drop_and_flag`.
        // On the controlled clock: a delayed test thread moves the receipt, and then the
        // booking itself produces the same two verdicts (the fake's test could not tell them
        // apart).
        let a_samples: Vec<[f32; 2]> = vec![[0.25, 0.0]; 30_000];
        let b_samples: Vec<[f32; 2]> = vec![[-0.5, 0.25]; 1_000];
        let bytes = |samples: &[[f32; 2]]| {
            let mut out = Vec::with_capacity(samples.len() * 8);
            for sample in samples {
                out.extend_from_slice(&sample[0].to_le_bytes());
                out.extend_from_slice(&sample[1].to_le_bytes());
            }
            out
        };
        let (a_bytes, b_bytes) = (bytes(&a_samples), bytes(&b_samples));
        let (a_hash, b_hash) = (ContentHash::of_bytes(&a_bytes), ContentHash::of_bytes(&b_bytes));
        let input = |name: &str, bytes: &[u8]| ezsdr_kernel::manifest::ingest_input(
            ezsdr_kernel::spec::Ident::parse(name).unwrap(),
            ezsdr_kernel::spec::Namespace::parse("ezsdr.input").unwrap(),
            format!("mem:{}", ContentHash::of_bytes(bytes)),
            bytes,
        );
        let (a_wave, b_wave) = (input("waveform", &a_bytes), input("waveform2", &b_bytes));
        for (policy, outcome) in [(LatePolicy::SendAsapAndFlag, "send_asap"), (LatePolicy::DropAndFlag, "drop")] {
            let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
            let device = Arc::new(FakeDevice::new(config));
            let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, BTreeMap::from([
                (a_hash.clone(), Arc::<[u8]>::from(a_bytes.clone())),
                (b_hash.clone(), Arc::<[u8]>::from(b_bytes.clone())),
            ]));
            let _frozen = crate::provider::test_support::Frozen::of(&device);
            device_at(&core, &time, &device, 0);
            device.tx_open(0, &[0]).unwrap();
            device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
            let (mut control, handed) = booking_control(&core);
            let clock = plan_of(&core, Dir::Tx)[0].clock.expect("the transmit clock");
            let (_to_tx, cmds) = std::sync::mpsc::channel();
            std::mem::forget(_to_tx);
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            let target = super::super::test_support::target_of(&core.tx_id);
            let mut book = |id: u64, k: i64, repeat: bool, wave: &ezsdr_kernel::manifest::ArtifactRef| {
                control.book(Dispatched::new(ActionId(id), Action::TxBurst {
                    target: target.clone(),
                    waveform: wave.clone(),
                    repeat,
                    at: AbsoluteDeadline::new(TimePoint::new(clock.domain, k)),
                    requested_at: None,
                    late_policy: policy,
                    metadata: BTreeMap::new(),
                }, Some(core.tx_id.clone())));
            };
            let none = |events: &Arc<EventCollector>| events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
            // A at 29 ms booked at 20 ms: 9 ms ahead, so the booking is on time.
            device_at(&core, &time, &device, core.ticks(20_000_000));
            book(1, 29_000, false, &a_wave);
            let TxCmd::Burst(a) = handed.try_recv().expect("A handed over") else { panic!("not a burst") };
            assert_eq!(a.k, 29_000, "{policy:?}: the booking moved A, so the receipt was late");
            assert!(none(&events).is_empty(), "{policy:?}: the booking emitted a verdict");
            tx.command(TxCmd::Burst(a));
            // The 1 000-sample SOB is followed by two 2 500-sample sends; the next full chunk
            // does not fit in the 37 ms edge at 27 ms (UR-23).
            for at in [20_i64, 23, 25] {
                device_at(&core, &time, &device, core.ticks(at * 1_000_000));
                assert!(tx.step(), "{policy:?}: no hand-over at {at} ms");
            }
            device_at(&core, &time, &device, core.ticks(27_000_000));
            assert!(!tx.step(), "{policy:?}: the 2 000 samples before the 37 ms window edge were handed in a short buffer");
            assert_eq!(tx.open.as_ref().expect("A is open").next, 35_000, "{policy:?}: A's first unhanded sample");
            // B, 7 ms ahead at 27 ms: on time at the receipt (5 ms left after the device lead).
            book(2, 34_000, true, &b_wave);
            let TxCmd::Burst(b) = handed.try_recv().expect("B handed over") else { panic!("not a burst") };
            assert_eq!(b.k, 34_000, "{policy:?}: the booking moved B, so 5 ms ahead was not on time at receipt");
            assert!(none(&events).is_empty(), "{policy:?}: the booking emitted a verdict");
            let calls = device.calls().len();
            tx.command(TxCmd::Burst(b));
            assert!(tx.step(), "{policy:?}: uhd-tx did not take B at its hand-over");
            let verdicts: Vec<_> = none(&events).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
            assert_eq!(verdicts.len(), 1, "{policy:?}: {verdicts:?}");
            assert_eq!(verdicts[0].payload["outcome"], outcome, "{policy:?}: {verdicts:?}");
            assert_eq!(verdicts[0].payload["target"]["ticks"], 34_000, "{policy:?}: {verdicts:?}");
            assert_eq!(device.calls().len(), calls, "{policy:?}: the hand-over's decision called the device: {:?}", device.calls());
            if policy == LatePolicy::SendAsapAndFlag {
                assert!(tx.open.is_none() && tx.continues_at == Some(35_000), "{policy:?}: A is not kept for the continuation");
                assert!(tx.held.contains_key(&35_000) && !tx.held.contains_key(&34_000), "{policy:?}: {:?}", tx.held.keys().collect::<Vec<_>>());
                let reserved: Vec<_> = lock(&core.held).iter().copied().collect();
                assert!(reserved.contains(&(clock.domain, 35_000)) && !reserved.contains(&(clock.domain, 34_000)), "{policy:?}: {reserved:?}");
                device_at(&core, &time, &device, core.ticks(30_000_000));
                assert!(tx.step(), "{policy:?}: B is not handed over at the sample A ended at");
                assert_eq!(device.transmitted(clock.instant(35_000), 1_000), b_samples.iter().copied().map(Some).collect::<Vec<_>>(),
                    "{policy:?}: B does not play where uhd-tx had not handed A over: {:?}", device.calls());
                assert_eq!(sends(&device).last(), Some(&(2_500, false, false)), "{policy:?}: B did not continue A's device burst: {:?}", sends(&device));
                assert!(lock(&core.rec).device_async.iter().all(|row| row["code"] != "TimeError" && row["code"] != "Underflow"),
                    "{policy:?}: {:?}", lock(&core.rec).device_async);
            } else {
                assert!(tx.open.as_ref().is_some_and(|open| open.held.k == 29_000), "{policy:?}: A is not still open");
                assert!(tx.held.is_empty(), "{policy:?}: {:?}", tx.held.keys().collect::<Vec<_>>());
                assert!(!lock(&core.held).contains(&(clock.domain, 34_000)), "{policy:?}: the dropped burst kept its reservation");
            }
        }
    }

    #[test]
    fn ur_21_a_moved_start_on_a_held_burst_s_is_refused() {
        // RM-15 as VE-2 amends it: B, late against the open burst A's next sample, moves
        // there under `send_asap`; C is already held at that sample, so B is refused
        // (`TIME_ERROR { refused }`, `COMMAND_REJECTED`) and C keeps its start.
        let config = FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
        let (core, device, time, events) = crate::provider::test_support::rig_with(config, Vec::new());
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let origin = (core.now() / 200 + 1) * 200;
        let clock = core.register(Dir::Tx, 200, origin).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        // A is 20 ms ahead; advance the manual device clock until it has handed over a few buffers.
        let a = clock.at_or_after(core.now() + core.ticks(20_000_000));
        tx.command(TxCmd::Burst(held(&clock, a, 30_000, LatePolicy::DropAndFlag)));
        let mut now = core.now();
        for _ in 0..50 {
            if tx.open.as_ref().is_some_and(|open| open.next >= a + 4_000) { break; }
            now += core.ticks(1_000_000);
            device_at(&core, &time, &device, now);
            tx.step();
        }
        let next = tx.open.as_ref().unwrap().next;
        assert!(next >= a + 4_000, "A never handed over 4 000 samples: next={next}, start={a}");
        // C at A's next sample, B before it: both queued before the step that decides B.
        let decision_time = core.now();
        tx.command(TxCmd::Burst(held(&clock, next, 1_000, LatePolicy::DropAndFlag)));
        tx.command(TxCmd::Burst(held(&clock, next - 500, 1_000, LatePolicy::SendAsapAndFlag)));
        lock(&core.held).insert((clock.domain, next));
        lock(&core.held).insert((clock.domain, next - 500));
        assert_eq!(core.now(), decision_time, "reserving B and C advanced the held Core clock");
        assert_eq!(device.time_now().unwrap(), decision_time, "reserving B and C advanced the held device clock");
        assert!(tx.step());
        assert_eq!(core.now(), decision_time, "the decision advanced the held Core clock");
        assert_eq!(device.time_now().unwrap(), decision_time, "the decision advanced the held device clock");
        assert!(tx.held.contains_key(&next), "C keeps its start");
        assert_eq!(tx.held.len(), 1, "B is not held: {:?}", tx.held.keys().collect::<Vec<_>>());
        let reasons: Vec<String> = lock(&core.rec).rejected.iter().filter_map(|r| r["reason"].as_str().map(str::to_owned)).collect();
        assert!(reasons.iter().any(|r| r == "UR-21: the moved start is a held burst's"), "{reasons:?}");
        let emitted = events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        assert!(emitted.iter().any(|e| e.kind == EventKind::parse(ezsdr_radio::kinds::TIME_ERROR).unwrap() && e.payload["outcome"] == "refused"), "no TIME_ERROR {{ refused }}: {emitted:?}");
        assert!(emitted.iter().any(|e| e.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED), "no COMMAND_REJECTED: {emitted:?}");
    }
    #[test]
    fn ur_29_eob_device_errors_are_escalated() {
        for lost in [true, false] {
            for tail in [None, Some(Tail { samples: [[0.25, 0.0]; TX_CHANNELS_MAX], channels: 1 })] {
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
        tx.tail = Some(Tail { samples: [[0.25, 0.0]; TX_CHANNELS_MAX], channels: 1 });
        tx.switch = Some(switch(&core, 20_000_000, Some(new), 2, Settings { rate: Some(2e6), ..Settings::default() }));
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
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(old), Channels::first(1));
        tx.switch = Some(switch(&core, cut, None, 0, Settings::default()));
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
        tx_channels_from_none(&mut control, &core, 2, 100, &[0]);
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        tx.read(None);
        assert!(tx.step(), "the enable's switch");
        let first = tx.clock.expect("the enable starts a clock");
        tx.remember_burst(TimePoint::new(first.domain, 100));
        // The disable ends the segment's clock; the owner is idle then, and the next enable's
        // open replaces the streamer the first burst's acknowledgement was awaited on.
        tx_channels_from_none(&mut control, &core, 3, 200, &[]);
        let cut = plan_of(&core, Dir::Tx)[0].segment.cut.expect("the empty change cuts the segment");
        time.advance_to(core.at(first.instant(cut))).unwrap();
        tx.read(None);
        assert!(tx.step(), "the disable's switch");
        assert!(tx.clock.is_none(), "the disable leaves no clock");
        tx_channels_from_none(&mut control, &core, 4, 300, &[0]);
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
        tx_channels_from_none(&mut control, &core, 2, 100, &[0]);
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        tx.read(None);
        assert!(tx.step(), "the enable's switch");
        let first = tx.clock.expect("the enable starts a clock");
        tx.remember_burst(TimePoint::new(first.domain, 100));
        tx_channels_from_none(&mut control, &core, 3, 200, &[]);
        let cut = plan_of(&core, Dir::Tx)[0].segment.cut.expect("the empty change cuts the segment");
        time.advance_to(core.at(first.instant(cut))).unwrap();
        tx.read(None);
        assert!(tx.step(), "the disable's switch");
        assert!(tx.clock.is_none(), "the disable leaves no clock");
        tx_channels_from_none(&mut control, &core, 4, 300, &[0]);
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
        let (_core, feed, events, mut tx, _) = replaced_queue();
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
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(old), Channels::first(1));
        tx.remember_burst(TimePoint::new(old.domain, 100));
        // Another set: only the owner can open it, and its reopen replaces the report queue.
        tx.switch = Some(switch(&core, 20_000_000, Some(new), 2, Settings { rate: Some(2e6), ..Settings::default() }));
        tx.do_switch();
        assert!(tx.unacked.is_empty());
        tx.remember_burst(TimePoint::new(new.domain, 100));
        tx.clock = None;
        // The same switch again: the record holds the set and the segment's configuration, so
        // the streamer is not opened again and the burst's correlation stands.
        tx.switch = Some(switch(&core, i64::MIN, Some(new), 2, Settings { rate: Some(2e6), ..Settings::default() }));
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
            tx.switch = Some(switch(&core, 20_000_000, new, if new.is_some() { 2 } else { 0 },
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
                    let lead = core.ticks(ezsdr_radio::timeline::start_lead_ns(&core.description.timing, direction));
                    streams.lines.insert(key, Line::new(Stream { direction, origin: 0, config: config(200), lead, call: 0, allowance: 0 }));
                    streams.stream_mut(key).configs.insert(1, Arc::new(super::super::control::ColdConfig::new(BTreeMap::new())));
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
        tx.tail = Some(Tail { samples: [[0.25, 0.0]; TX_CHANNELS_MAX], channels: 1 });
        tx.command(TxCmd::Burst(held(&c0, 20_020, 10, LatePolicy::DropAndFlag)));
        time.advance_to(core.at(3_600_000)).unwrap();
        assert!(tx.step());
        // A switch to two channels at another clock: ends the open device burst (with its held
        // sample), reopens the streamer and configures the direction. The reports are read
        // after it, so that the held sample's end of burst is the switch's.
        tx.command(TxCmd::Burst(held(&c0, 20_100, 10, LatePolicy::DropAndFlag)));
        tx.switch = Some(switch(&core, 20_000_000, Some(c1), 2, Settings { rate: Some(2e6), ..Settings::default() }));
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
        tx_channels_from_none(&mut control, &core, 2, 100, &[0]);
        let origin = plan_of(&core, Dir::Tx)[0].segment.origin;
        let (to_tx, cmds) = std::sync::mpsc::channel();
        std::mem::forget(to_tx);
        let mut tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        assert!(tx.read(None), "the owner reads the plan");
        let switched = device.calls().len();
        assert!(tx.step(), "the owner begins the enable's segment");
        assert!(!device.calls()[switched..].iter().any(|call| call.starts_with("apply") || call.starts_with("tx_open")), "the owner configured a segment the enable did: {:?}", device.calls());
        let gain = crate::provider::core::key("radio.tx.c0.gain_db");
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
        let k = stream_key(Dir::Tx, 0);
        (streams.holder(k) != Holder::Free, streams.reading(k), streams.stream(k).configured)
    }

    #[test]
    fn ur_25_a_transmit_owner_waiting_behind_a_from_none_open_starts_nothing() {
        // UR-25 (S9, transmit): while uhd-control's enable from no stream owns the streamer,
        // uhd-tx reads no plan, moves no cursor and begins no switch; the first turn after the
        // booking opens B's set again: the record holds C's.
        let (core, spy) = spy_rig();
        let mut control = control_of(&core);
        control.config.insert(crate::provider::core::key("radio.tx.c1.stream"), Value::from(0));
        tx_channels_from_none(&mut control, &core, 2, 100, &[0, 1]);
        tx_channels_from_none(&mut control, &core, 3, 200, &[]);
        let gate = spy.stop_before("tx_open");
        let host = core.device.host();
        let core_ = core.clone();
        let (done, finished) = std::sync::mpsc::channel();
        let control_thread = host.spawn("control", move || {
            let mut control = control;
            tx_channels_from_none(&mut control, &core_, 4, 300, &[0]);
            let _ = done.send(());
        }).expect("a participant thread");
        gate.wait();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        let mark = spy.log().len();
        let idle = || lock(&core.streams).is_idle(stream_key(Dir::Tx, 0));
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
            tx_channels_from_none(&mut control, &core_, 2, 100, &[0]);
            let _ = done.send(());
        }).expect("a participant thread");
        gate.wait();
        assert_eq!(lock(&core.streams).holder(stream_key(Dir::Tx, 0)), Holder::FromNone, "the streamer is not uhd-control's");
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        let mark = spy.log().len();
        assert!(!tx.read(None));
        assert!(lock(&core.streams).is_idle(stream_key(Dir::Tx, 0)), "the poll moved the stream's phase");
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
        let released = std::sync::Mutex::new(Some(released));
        crate::provider::test_support::probe(&core, "reading_wait", move |_| {
            if let Some(wait) = released.lock().unwrap().take() {
                let _ = entered.send(());
                let _ = wait.recv();
            }
        });
        let control = control_of(&core);
        let core_ = core.clone();
        let (done, finished) = std::sync::mpsc::channel();
        let control_thread = host.spawn("control", move || {
            let mut control = control;
            tx_channels_from_none(&mut control, &core_, 2, 100, &[0]);
            let _ = done.send(());
        }).expect("a participant thread");
        entered_rx.recv_timeout(std::time::Duration::from_secs(10)).expect("the enable entered the wait");
        assert_eq!(spy.seen("tx_open"), 0, "the enable opened the streamer while a report read is in flight: {:?}", spy.log());
        assert!(tx_record(&core).0, "the enable did not take the streamer");
        assert_eq!(lock(&core.streams).holder(stream_key(Dir::Tx, 0)), Holder::FromNone, "the enable's claim is not held");
        assert!(tx_record(&core).1, "the read is in flight at the wait");
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
            tx_channels_from_none(&mut control, &core_, 2, 100, &[0]);
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
        assert_eq!(lock(&core.streams).holder(stream_key(Dir::Tx, 0)), Holder::Free, "the stopped enable gave its claim back");
        assert!(tx_record(&core).1, "the read is still uhd-tx's after the control ends");
        gate.release();
        let _ = to_tx.send(TxCmd::Shutdown);
        tx_thread.join().expect("the transmit thread");
        assert!(!tx_record(&core).1, "the read ended with the transmit thread");
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
            tx_channels_from_none(&mut control, &core_, 2, 100, &[0]);
            let _ = done.send(());
        }).expect("a participant thread");
        entered_rx.recv_timeout(std::time::Duration::from_secs(10)).expect("the enable entered the wait");
        core.device_lost("test");
        release.send(()).unwrap();
        assert!(finished.recv_timeout(std::time::Duration::from_secs(10)).is_ok(), "the loss did not end the wait");
        control_thread.join().expect("the control thread");
        assert_eq!((spy.seen("tx_open"), spy.seen("apply")), (0, 0), "a lost enable called the device: {:?}", spy.log());
        assert_eq!(tx_colds(&core), 0, "a lost enable booked a change");
        assert_eq!(lock(&core.streams).holder(stream_key(Dir::Tx, 0)), Holder::Free, "the lost enable gave its claim back");
        assert!(tx_record(&core).1, "the read is still uhd-tx's after the control ends");
        gate.release();
        let _ = to_tx.send(TxCmd::Shutdown);
        tx_thread.join().expect("the transmit thread");
        assert!(!tx_record(&core).1, "the read ended with the transmit thread");
    }

    /// A layout with two transmit streams and no receive channel (RM-29).
    fn two_tx_description() -> ezsdr_radio::channels::Description {
        use ezsdr_kernel::spec::{Ident, Scalar, Value};
        let selector = std::collections::BTreeMap::from([
            (Ident::parse("rx").unwrap(), Value::List(Vec::new())),
            (Ident::parse("tx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:B:ubx:0")])),
        ]);
        let layout = ezsdr_radio::channels::from_selector(&selector).unwrap();
        ezsdr_radio::channels::Description::new(&layout, 2_000, &crate::profile::terms())
    }

    #[test]
    fn ur_28_a_later_stream_s_reopen_discards_only_its_correlations() {
        // UR-28, UR-36 (T3): the streamer's generation is per stream; an open of stream 1
        // discards stream 1's pending correlations alone.
        let inner = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig::default()));
        let feed = Arc::new(ReportFeed::new(inner.clone()));
        let (core, _, _) = super::super::test_support::rig_on_description(feed.clone(), Vec::new(), BTreeMap::new(), two_tx_description());
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
        assert_eq!(lock(&core.rec).device_async.last().map(|row| row["stream"].as_u64()), Some(Some(1)), "the row names the report's stream");
        assert!(s1.unacked.is_empty(), "stream 1's old correlations are dropped");
        assert_eq!(s0.unacked.len(), 1, "stream 0's correlation stands");
        // The same row from stream 0: its `stream` is the report's, not a constant 1.
        feed.give(TxReport { code: TxCode::Other(0x40), tick: Some(0), channel: 0 });
        s0.reports(std::time::Duration::ZERO);
        assert_eq!(lock(&core.rec).device_async.last().map(|row| row["stream"].as_u64()), Some(Some(0)), "stream 0's row names its stream");
        assert!(!inner.calls().iter().any(|call| call.starts_with("tx_send")), "{:?}", inner.calls());
    }

    #[test]
    fn ur_25_a_transmit_report_read_of_a_later_stream_does_not_hold_the_earlier_owner() {
        // UR-25 (T4): an enable from no stream of stream 1 waits on stream 1's report read
        // alone; stream 0's owner keeps making device calls meanwhile.
        let device = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig::default()));
        device.tx_open(0, &[0]).unwrap();
        device.tx_open(1, &[1]).unwrap();
        let (core, _, _) = super::super::test_support::rig_on_description(device.clone(), Vec::new(), BTreeMap::new(), two_tx_description());
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to0, cmds0) = std::sync::mpsc::channel();
        let mut s0 = Tx::new(core.clone(), 0, cmds0, Some(clock), Channels::first(1));
        let at = clock.at_or_after(core.now() + core.ticks(1_000_000));
        s0.command(TxCmd::Burst(held(&clock, at, 1_000, LatePolicy::SendAsapAndFlag)));
        assert!(lock(&core.streams).begin_read(stream_key(Dir::Tx, 1)));
        // Ends stream 1's read even when an assert below fails (T4).
        struct Reading<'a>(&'a Arc<Core>);
        impl Drop for Reading<'_> {
            fn drop(&mut self) {
                lock(&self.0.streams).end_read(stream_key(Dir::Tx, 1));
            }
        }
        let reading = Reading(&core);
        let (to_tx, _cmds1) = std::sync::mpsc::channel();
        let mut control = Control::new(core.clone(), Arc::new(NoActions), vec![to_tx.clone(), to_tx], core.description.defaults(), false,
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
        assert!(lock(&core.streams).reading(stream_key(Dir::Tx, 1)), "stream 1's read is still in flight");
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
        tx.switch = Some(switch(&core, 0, Some(clock), 1, Settings::default()));
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
    fn ur_29_a_lost_transmit_owner_configure_releases_stream_lock_before_reporting() {
        let inner = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig::default()));
        let device: Arc<dyn crate::device::Device> = Arc::new(super::super::test_support::ApplyFails::lost(inner.clone(), Dir::Tx, Some(0)));
        let (core, _time, _events) = super::super::test_support::rig_on(device, Vec::new(), 2_000);
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        tx.switch = Some(switch(&core, 0, Some(clock), 1, Settings { rate: Some(2e6), ..Settings::default() }));

        tx.do_switch();
        assert!(core.is_lost());
        assert_eq!(lock(&core.streams).holder(stream_key(Dir::Tx, 0)), Holder::Free);
        assert!(tx.clock.is_none(), "a lost configure did not adopt its clock");
        assert!(!inner.calls().iter().any(|call| call.starts_with("tx_send")), "the lost owner sent a burst: {:?}", inner.calls());
    }

    #[test]
    fn ur_26_a_transmit_switch_at_a_stop_keeps_its_clock_and_sends_nothing() {
        // UR-26 (S16b): on stream 1, the owner does not re-read `Provider::stop` after its
        // switch's configuration; it takes the clock, and the plan ends that segment at its
        // origin, so nothing is sent.
        let inner = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig::default()));
        let holder = Arc::new(HoldApply::new(inner.clone(), Dir::Tx, Some(0)));
        let (core, _, _) = super::super::test_support::rig_on_description(holder.clone(), Vec::new(), BTreeMap::new(), two_tx_description());
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
            streams.stream_mut(key).configs.insert(1, Arc::new(super::super::control::ColdConfig::new(BTreeMap::new())));
            assert!(streams.book(&core, key, ezsdr_radio::timeline::Item { e: 2_000, seq: 1, ready: 0, release: ezsdr_radio::timeline::NO_RELEASE, delivered: None, refused: false, kind: ezsdr_radio::timeline::Kind::Cold(config) }));
            streams.planned(key)[0].clock.expect("the booked segment's clock")
        };
        tx.switch = Some(switch_on(&core, 1, 0, Some(clock), 1, Settings::default()));
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
        let mut tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        let claim = lock(&core.streams).claim_from_none(stream_key(Dir::Tx, 0)).expect("the stream is free");
        std::mem::forget(claim);
        tx.shutdown();
        assert!(spy.seen("tx_async") >= 1, "the shutdown's read waited for a detached control");
        assert!(!tx_record(&core).1, "the shutdown's read was not taken back");
    }

    #[test]
    fn ur_25_a_refused_transmit_owner_books_and_gives_back_in_one_line_section() {
        // UR-25 (S2, transmit): uhd-tx books its refusal while the claim still holds the
        // streamer, in the section that holds the line.
        use ezsdr_radio::timeline::{Config, Item, Kind, NO_RELEASE};
        let inner = Arc::new(FakeDevice::new(FakeConfig::default()));
        let device: Arc<dyn Device> = Arc::new(super::super::test_support::ApplyFails::new(inner.clone(), Dir::Tx, None));
        let (core, _, _) = crate::provider::test_support::rig_on(device, Vec::new(), 2_000);
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        tx.switch = Some(switch(&core, 0, Some(clock), 1, Settings { rate: Some(2e6), ..Settings::default() }));
        {
            let mut streams = lock(&core.streams);
            let stream = stream_key(Dir::Tx, 0);
            let mut line = streams.line(stream).expect("the switch's line").clone();
            line.items.push(Item {
                e: 0,
                seq: 1,
                ready: 0,
                release: NO_RELEASE,
                delivered: None,
                refused: false,
                kind: Kind::Cold(Config { channels: Channels::first(1), ratio: ezsdr_kernel::time::Rational::new(200, 1).unwrap() }),
            });
            streams.lines.insert(stream, line);
        }

        let (entered, reached) = std::sync::mpsc::channel();
        let (resume, wait) = std::sync::mpsc::channel();
        let wait = std::sync::Mutex::new(Some(wait));
        let (seen, holders) = std::sync::mpsc::channel();
        super::super::test_support::probe(&core, "refused_booking", move |streams| {
            if let Some(wait) = wait.lock().unwrap().take() {
                let streams = streams.expect("the caller's section");
                let _ = seen.send(streams.holder(stream_key(Dir::Tx, 0)));
                let _ = entered.send(());
                let _ = wait.recv();
            }
        });
        let switching = std::thread::spawn(move || {
            tx.do_switch();
            tx
        });
        reached.recv_timeout(std::time::Duration::from_secs(10)).expect("the owner reached its refusal book");
        assert_eq!(holders.recv().unwrap(), Holder::Owner { seq: Some(1) }, "the claim still holds the stream");
        assert!(core.streams.try_lock().is_err(), "the refusal is booked under the line lock");
        let _ = resume.send(());
        let tx = switching.join().expect("the transmit owner");
        let streams = lock(&core.streams);
        assert_eq!(streams.holder(stream_key(Dir::Tx, 0)), Holder::Free);
        assert!(!streams.reading(stream_key(Dir::Tx, 0)));
        assert!(streams.line(stream_key(Dir::Tx, 0)).unwrap().items.iter().any(|item| item.seq == 1 && item.refused), "the refusal is booked");
        drop(streams);
        assert!(tx.clock.is_none(), "a refused switch adopts no clock");
    }

    #[test]
    fn ur_25_a_shutdown_read_and_a_returning_from_none_resolve_with_the_control_first() {
        // UR-25 (S6): the from-none enable is inside a device call when the shutdown's report
        // read starts; the read takes no holder check, so both overlap. The claim comes back
        // while the read is still in flight, and the read ends after it.
        let (core, spy) = spy_rig();
        let stream = stream_key(Dir::Tx, 0);
        let apply = spy.stop_before("apply");
        let control_core = core.clone();
        let control_thread = std::thread::spawn(move || {
            let mut control = control_of(&control_core);
            tx_channels_from_none(&mut control, &control_core, 2, 100, &[0]);
        });
        apply.wait();
        assert_eq!(lock(&core.streams).holder(stream), Holder::FromNone, "the enable holds the stream through its device call");

        let read = spy.stop_before("tx_async");
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        let shutdown_thread = std::thread::spawn(move || tx.shutdown());
        read.wait();
        assert_eq!(lock(&core.streams).holder(stream), Holder::FromNone);
        assert!(lock(&core.streams).reading(stream), "the shutdown's read is in flight");

        apply.release();
        control_thread.join().expect("the control thread");
        assert_eq!(lock(&core.streams).holder(stream), Holder::Free, "the enable gave its claim back");
        assert!(lock(&core.streams).reading(stream), "the read outlives the control's claim");
        assert_eq!(spy.seen("tx_open"), 1, "the enable opened the streamer: {:?}", spy.log());
        assert!(spy.seen("apply") >= 1, "{:?}", spy.log());

        read.release();
        shutdown_thread.join().expect("the shutdown thread");
        assert!(!lock(&core.streams).reading(stream), "the read ended");
        assert_eq!(lock(&core.streams).holder(stream), Holder::Free);
    }

    #[test]
    fn ur_25_a_shutdown_read_and_a_returning_from_none_resolve_with_the_read_first() {
        // UR-25 (S6): the read ends first; the enable's claim stays `FromNone` until its own
        // booking section, and then goes back.
        let (core, spy) = spy_rig();
        let stream = stream_key(Dir::Tx, 0);
        let apply = spy.stop_before("apply");
        let control_core = core.clone();
        let control_thread = std::thread::spawn(move || {
            let mut control = control_of(&control_core);
            tx_channels_from_none(&mut control, &control_core, 2, 100, &[0]);
        });
        apply.wait();
        assert_eq!(lock(&core.streams).holder(stream), Holder::FromNone);

        let read = spy.stop_before("tx_async");
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        let shutdown_thread = std::thread::spawn(move || tx.shutdown());
        read.wait();
        assert!(lock(&core.streams).reading(stream), "the shutdown's read is in flight");

        read.release();
        shutdown_thread.join().expect("the shutdown thread");
        assert!(!lock(&core.streams).reading(stream), "the read ended");
        assert_eq!(lock(&core.streams).holder(stream), Holder::FromNone, "the control's claim remains");

        apply.release();
        control_thread.join().expect("the control thread");
        assert_eq!(lock(&core.streams).holder(stream), Holder::Free);
        assert!(!lock(&core.streams).reading(stream));
    }

    #[test]
    fn ur_26_stop_cancels_current_and_future_bursts_record_only() {
        let (core, device, _, events) = super::super::test_support::rig();
        let current = core.register(Dir::Tx, 200, 0).unwrap();
        let future = core.register(Dir::Tx, 200, core.ticks(100_000_000)).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(current), Channels::first(1));
        tx.held.insert(20, held(&current, 20, 1, LatePolicy::DropAndFlag));
        tx.later.push(held(&future, 30, 1, LatePolicy::DropAndFlag));
        lock(&core.held).extend([(current.domain, 20), (future.domain, 30)]);

        tx.stop_now("UR-26: cancelled by Stop", ezsdr_radio::CancelReason::Stop);

        assert!(tx.held.is_empty() && tx.later.is_empty());
        assert!(!lock(&core.held).contains(&(current.domain, 20)));
        assert!(!lock(&core.held).contains(&(future.domain, 30)));
        let rejected = lock(&core.rec).rejected.clone();
        assert_eq!(rejected.len(), 2, "{rejected:?}");
        assert!(rejected.iter().all(|row| row["reason"] == "UR-26: cancelled by Stop"), "{rejected:?}");
        assert!(!device.calls().iter().any(|call| call.starts_with("tx_send ")));
        assert!(!events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter()
            .any(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED));
    }

    #[test]
    fn ur_23_a_cold_switch_cancels_a_current_burst_and_releases_its_reservation() {
        let (core, device, _, events) = super::super::test_support::rig();
        let current = core.register(Dir::Tx, 200, 0).unwrap();
        let next = core.register(Dir::Tx, 200, core.ticks(100_000_000)).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(current), Channels::first(1));
        tx.held.insert(20, held(&current, 20, 1, LatePolicy::DropAndFlag));
        lock(&core.held).insert((current.domain, 20));
        let mut switching = switch(&core, core.ticks(50_000_000), Some(next), 1, Settings::default());
        switching.to.as_mut().unwrap().refused = true;
        tx.switch = Some(switching);

        tx.do_switch();

        assert!(tx.held.is_empty());
        assert!(!lock(&core.held).contains(&(current.domain, 20)));
        let rejected = lock(&core.rec).rejected.clone();
        assert_eq!(rejected.len(), 1, "{rejected:?}");
        assert_eq!(rejected[0]["reason"], "cancelled by a cold change");
        assert!(events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter()
            .any(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED));
        assert!(!device.calls().iter().any(|call| call.starts_with("tx_send ")));
    }

    #[test]
    fn ur_25_a_skipped_future_clock_cancellation_releases_its_reservation() {
        let (core, device, _, events) = super::super::test_support::rig();
        let current = core.register(Dir::Tx, 200, 0).unwrap();
        let skipped = core.register(Dir::Tx, 200, core.ticks(100_000_000)).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(current), Channels::first(1));
        tx.later.push(held(&skipped, 30, 1, LatePolicy::DropAndFlag));
        lock(&core.held).insert((skipped.domain, 30));
        let mut switching = switch(&core, core.ticks(50_000_000), Some(skipped), 1, Settings::default());
        switching.to.as_mut().unwrap().segment.cut = Some(0);
        {
            let stream = stream_key(Dir::Tx, 0);
            let mut streams = lock(&core.streams);
            let mut line = streams.lines.line(stream).unwrap().clone();
            line.plan[0].cut = Some(0);
            streams.lines.insert(stream, line);
        }
        tx.switch = Some(switching);

        tx.do_switch();

        assert!(tx.later.is_empty());
        assert!(!lock(&core.held).contains(&(skipped.domain, 30)));
        let rejected = lock(&core.rec).rejected.clone();
        assert_eq!(rejected.len(), 1, "{rejected:?}");
        assert_eq!(rejected[0]["reason"], "UR-25: its transmit clock was never started");
        let rejected_events: Vec<_> = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
            .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).collect();
        assert_eq!(rejected_events.len(), 1, "{rejected_events:?}");
        assert!(!device.calls().iter().any(|call| call.starts_with("tx_send ")));
    }

    // ------------------------------------------ bursts continued across a burst's end (UR-23)

    /// A-0: `manual_rig` with the device's clock held still (M3), A's burst at 40 000 with
    /// 30 000 samples driven in 13 steps of up to 2 500 samples, so its last sample is held
    /// back and `continues_at` is 70 000 (design note 37, T1–T5).
    fn at_a_held_tail(faults: Vec<crate::device::FakeFault>)
        -> (Arc<Core>, Arc<FakeDevice>, Arc<ezsdr_kernel::time::ManualTimeAuthority>, Tx, crate::provider::test_support::Frozen) {
        let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, faults, ..FakeConfig::default() };
        let (core, device, time, _) = crate::provider::test_support::rig_with(config, Vec::new());
        let frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        std::mem::forget(_to_tx);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        lock(&core.held).insert((clock.domain, 40_000));
        tx.command(TxCmd::Burst(held(&clock, 40_000, 30_000, LatePolicy::DropAndFlag)));
        for step in 0..12 {
            let at = 31_000_000 + step * 2_500_000;
            device_at(&core, &time, &device, core.ticks(at));
            assert!(tx.step(), "A is not dispatched at {at} ns");
        }
        device_at(&core, &time, &device, core.ticks(61_000_000));
        assert!(tx.step(), "the final pre-window sample is not handed");
        assert!(tx.open.is_none() && tx.continues_at == Some(70_000) && tx.tail.as_ref().is_some_and(|tail| tail.channels == 1),
            "A's last sample is not the held tail: {:?}", device.calls());
        (core, device, time, tx, frozen)
    }

    /// A burst that continues A's device burst at `k`: `len` samples of a waveform unlike
    /// `held`'s uniform one, so that a continuation's order shows in the values.
    fn continuation(clock: &Clock, k: i64, len: usize) -> Held {
        let samples: Vec<[f32; 2]> = (0..len).map(|i| [-0.25 - i as f32 / 4_000.0, 0.125]).collect();
        let mut burst = held(clock, k, len, LatePolicy::DropAndFlag);
        burst.samples = Arc::new(vec![samples]);
        burst
    }

    /// Each `tx_send` the fake saw: `n`, start-of-burst, end-of-burst.
    fn sends(device: &FakeDevice) -> Vec<(usize, bool, bool)> {
        device.calls().iter().filter(|call| call.starts_with("tx_send ")).map(|call| {
            let field = |key: &str| call.split_whitespace().find_map(|word| word.strip_prefix(key));
            (field("n=").unwrap().parse().unwrap(), field("sob=") == Some("true"), field("eob=") == Some("true"))
        }).collect()
    }

    #[test]
    fn ur_34_two_moves_at_one_instant_swap_their_transmit_channels_without_deadlock() {
        // A2/A10 (design note 37): two transmit moves at one instant leave a segment holding
        // both channels whose cut is its origin. The owner skips it without a device call —
        // opening it would hold the reciprocal move's channel — and reads on to its own new
        // segment. On the controlled clock both moves are booked at `e`.
        let (core, device, time) = manual_rig();
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        let c0 = core.register_in(Dir::Tx, 0, 200, 0).unwrap();
        let c1 = core.register_in(Dir::Tx, 1, 200, 0).unwrap();
        let mut values = core.description.defaults();
        values.insert(crate::provider::core::key("radio.tx.c0.stream"), Value::from(0));
        values.insert(crate::provider::core::key("radio.tx.c1.stream"), Value::from(1));
        let (to_tx, _keep) = std::sync::mpsc::channel();
        let mut control = super::super::control::Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], values, false,
            super::super::control::Start { t0: 0, rx: vec![None, None], tx: vec![Some((c0, Channels::first(1))), Some((c1, Channels::first(2).without(0)))] });
        let ms = |n: i64| core.ticks(n * 1_000_000);
        for (id, name, value) in [(1, "radio.tx.c0.stream", 1), (2, "radio.tx.c1.stream", 0)] {
            control.book(Dispatched::new(ActionId(id), Action::UpdateParameter {
                target: super::super::test_support::target_of(&core.id),
                key: crate::provider::core::key(name),
                value: Value::from(value),
                class: UpdateClass::Cold,
                at: Some(AbsoluteDeadline::new(core.at(ms(100)))),
            }, Some(core.id.clone())));
        }
        // Both owners, each with its old streamer open and recorded, read the plan.
        device.tx_open(0, &[0]).unwrap();
        device.tx_open(1, &[1]).unwrap();
        {
            let mut streams = lock(&core.streams);
            streams.record_open(stream_key(Dir::Tx, 0), Some(Channels::first(1)));
            streams.record_open(stream_key(Dir::Tx, 1), Some(Channels::first(2).without(0)));
        }
        let (_, cmds0) = std::sync::mpsc::channel();
        let (_, cmds1) = std::sync::mpsc::channel();
        let mut tx0 = Tx::new(core.clone(), 0, cmds0, Some(c0), Channels::first(1));
        let mut tx1 = Tx::new(core.clone(), 1, cmds1, Some(c1), Channels::first(2).without(0));
        assert!(tx0.read(None) && tx1.read(None), "the owners read the plan");
        // Stream 1 is the one whose plan holds the zero-length union segment.
        let union = tx1.switch.as_ref().and_then(|switch| switch.to.as_ref()).expect("the zero-length segment");
        assert_eq!(union.segment.config.channels, Channels::first(2), "the scene's intermediate segment is not the union: {:?}", union.segment.config.channels);
        let cut = union.segment.cut.expect("the intermediate segment is cut");
        assert_eq!(union.segment.instant(cut).unwrap(), union.segment.origin, "the scene's intermediate segment holds samples");
        // The zero-length segment's owner skips it at its cut without a device call and reads
        // on; then both owners' destination switches run at once, each waiting for the other's
        // release.
        device_at(&core, &time, &device, ms(200));
        let mark = device.calls().len();
        assert!(tx1.step(), "stream 1's skip of the zero-length segment");
        assert_eq!(tx1.switch.as_ref().and_then(|switch| switch.to.as_ref()).map(|to| to.segment.config.channels),
            Some(Channels::first(2).without(1)), "stream 1's owner did not read on to its destination");
        assert!(device.calls()[mark..].iter().all(|call| !call.starts_with("tx_open") && !call.starts_with("apply")), "the zero-length segment made a device call: {:?}", device.calls());
        drop(_frozen);
        let (done0, reached0) = std::sync::mpsc::channel();
        let (done1, reached1) = std::sync::mpsc::channel();
        let owners = [std::thread::spawn(move || { let _ = tx0.step(); let _ = done0.send((tx0.clock, tx0.channels)); }),
            std::thread::spawn(move || { let _ = tx1.step(); let _ = done1.send((tx1.clock, tx1.channels)); })];
        let (clock0, channels0) = reached0.recv_timeout(Wall::from_secs(10)).expect("stream 0's owner did not reach its destination");
        let (clock1, channels1) = reached1.recv_timeout(Wall::from_secs(10)).expect("stream 1's owner did not reach its destination");
        for owner in owners { owner.join().expect("the owner's thread"); }
        let calls = device.calls();
        assert!(calls[mark..].iter().all(|call| !call.starts_with("tx_open 0 [0, 1]") && !call.starts_with("tx_open 1 [0, 1]")),
            "an owner opened the zero-length segment: {calls:?}");
        assert!(clock0.is_some() && channels0 == Channels::first(2).without(0), "stream 0's owner did not adopt its destination's clock: {clock0:?}, {channels0:?} {calls:?}");
        assert!(clock1.is_some() && channels1 == Channels::first(2).without(1), "stream 1's owner did not adopt its destination's clock: {clock1:?}, {channels1:?} {calls:?}");
        assert_eq!(calls.iter().filter(|call| call.as_str() == "tx_open 0 [1]").count(), 1, "stream 0's destination streamer: {calls:?}");
        assert_eq!(calls.iter().filter(|call| call.as_str() == "tx_open 1 [0]").count(), 1, "stream 1's destination streamer: {calls:?}");
        // The zero-length segment holds both channels but no samples, so it is no segment the
        // reciprocal destination waits for (A2).
        for (k, set) in [(0, Channels::first(2).without(0)), (1, Channels::first(2).without(1))] {
            let destination = lock(&core.streams).planned(stream_key(Dir::Tx, k)).into_iter().map(|planned| planned.segment.origin).max().expect("a destination segment");
            assert!(!lock(&core.streams).needed_before(stream_key(Dir::Tx, k), set, destination), "stream {k}'s zero-length segment holds the reciprocal destination back (A2)");
        }
        for k in 0..2 {
            let done = lock(&core.streams).stream(stream_key(Dir::Tx, k)).completed.clone();
            assert!(done.iter().any(|(origin, _, by)| *origin == 0 && by.is_none()), "stream {k}'s old segment is not completed by its owner: {done:?}");
        }
    }

    #[test]
    fn ur_25_a_same_size_membership_swap_sends_the_new_segment_on_its_own_channel() {
        // M1 (design note 37): `{0}` becomes `{1}` without changing the count, while the rate
        // also changes. The new segment must reopen on channel 1 and recompute window/4 from
        // its 2 Ms/s clock; loopback distinguishes its samples from the old channel's ramp.
        let (core, device, time) = manual_rig();
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        let c0 = core.register_in(Dir::Tx, 0, 200, 0).unwrap();
        let mut values = core.description.defaults();
        values.insert(crate::provider::core::key("radio.tx.c0.stream"), Value::from(0));
        values.insert(crate::provider::core::key("radio.tx.c1.stream"), Value::from(-1));
        let (to_tx, _keep) = std::sync::mpsc::channel();
        let mut control = super::super::control::Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], values, false,
            super::super::control::Start { t0: 0, rx: vec![None, None], tx: vec![Some((c0, Channels::first(1))), None] });
        let ms = |n: i64| core.ticks(n * 1_000_000);
        for (id, name, value) in [(1, "radio.tx.c0.stream", -1), (2, "radio.tx.c1.stream", 0)] {
            control.book(Dispatched::new(ActionId(id), Action::UpdateParameter {
                target: super::super::test_support::target_of(&core.id),
                key: crate::provider::core::key(name),
                value: Value::from(value),
                class: UpdateClass::Cold,
                at: Some(AbsoluteDeadline::new(core.at(ms(100)))),
            }, Some(core.id.clone())));
        }
        control.book(Dispatched::new(ActionId(3), Action::UpdateParameter {
            target: super::super::test_support::target_of(&core.id),
            key: crate::provider::core::key("radio.tx.s0.sample_rate_hz"),
            value: Value::num(2e6).unwrap(),
            class: UpdateClass::Cold,
            at: Some(AbsoluteDeadline::new(core.at(ms(200)))),
        }, Some(core.id.clone())));
        let plan = lock(&core.streams).planned(stream_key(Dir::Tx, 0));
        let destination = plan.last().expect("the new segment").clone();
        assert_eq!(destination.segment.config.channels, Channels::first(2).without(0), "{:?}", plan.iter().map(|planned| planned.segment.config.channels).collect::<Vec<_>>());
        let destination_clock = destination.clock.expect("the destination's clock is registered at booking");
        assert_eq!(destination_clock.n, 100, "the rate switch is on the new 2 Ms/s sample grid");
        // The streamer the arm opened and the owner: the switch at the cut opens the new set.
        device.tx_open(0, &[0]).unwrap();
        lock(&core.streams).record_open(stream_key(Dir::Tx, 0), Some(Channels::first(1)));
        let (_to_owner, cmds) = std::sync::mpsc::channel();
        std::mem::forget(_to_owner);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(c0), Channels::first(1));
        assert!(tx.read(None), "the owner reads the plan");
        device_at(&core, &time, &device, ms(100));
        assert!(tx.step(), "the owner switches the channel membership");
        assert!(tx.clock.is_some_and(|clock| clock.n == 200) && tx.channels == Channels::first(2).without(0), "the membership segment: {:?}", device.calls());
        device_at(&core, &time, &device, ms(200));
        assert!(tx.step(), "the owner switches to the new sample rate");
        assert!(tx.clock.is_some_and(|clock| clock.domain == destination_clock.domain), "the owner did not adopt the destination's clock: {:?}", device.calls());
        let calls = device.calls();
        assert_eq!(calls.iter().filter(|call| call.as_str() == "tx_open 0 [1]").count(), 1, "the streamer is not reopened for the new set: {calls:?}");
        assert!(calls.iter().any(|call| call.starts_with("apply tx 1 ") && call.ends_with("at=-")), "the new set's channel is not configured: {calls:?}");
        // The destination's burst, booked before the cut on its clock, and the loopback that
        // tells device channel 1 from channel 0.
        let wave: Vec<[f32; 2]> = (0..5_001).map(|i| [0.01 + i as f32 / 100_000.0, 0.375]).collect();
        let new_k = ms(5) / destination_clock.n;
        lock(&core.held).insert((destination_clock.domain, new_k));
        tx.command(TxCmd::Burst(one_shot(&destination_clock, new_k, 1, wave.clone())));
        let burst_at = destination_clock.instant(new_k);
        device.apply(Dir::Rx, 1, &Settings { rate: Some(2e6), ..Settings::default() }, None).unwrap();
        device.rx_open(1, &[1]).unwrap();
        device.rx_start(1, burst_at).unwrap();
        device_at(&core, &time, &device, destination_clock.origin + ms(3));
        assert!(tx.step(), "the destination's burst is dispatched: {:?}", device.calls());
        assert_eq!(*sends(&device).last().unwrap(), (5_000, true, false), "window/4 is recomputed from the new rate");
        assert_eq!(tx.contract, Some(ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.cf32").unwrap()), "TX blocks retain the cf32 contract");
        assert!(tx.step(), "the finite burst's final sample is held for EOB");
        assert!(tx.tail.is_some());
        device_at(&core, &time, &device, destination_clock.origin + ms(6));
        assert!(tx.step(), "the held sample closes on the new sample clock");
        assert_eq!(sends(&device), [(5_000, true, false), (1, false, true)]);
        assert_eq!(device.unended_bursts(), 0, "{:?}", device.calls());
        device_at(&core, &time, &device, destination_clock.origin + ms(8));
        let got = match device.rx_recv(1, wave.len(), Wall::from_millis(100)) {
            crate::device::RxRecv::Samples { samples, .. } => samples,
            other => panic!("the loopback of the new segment's burst: {other:?}"),
        };
        assert_eq!(got[0], wave, "the new segment's payload and held EOB sample are not on device channel 1: {:?}", device.calls());
        assert_eq!(device.transmitted(burst_at, wave.len()), vec![None; wave.len()], "the old channel sent the new segment's samples: {:?}", device.calls());
        tx.command(TxCmd::Stop);
    }

    /// A one-shot burst of `samples` on each of `channels` buffers, booked at sample `k` of
    /// `clock`.
    fn one_shot(clock: &super::Clock, k: i64, channels: usize, samples: Vec<[f32; 2]>) -> Held {
        let waveform_len = samples.len();
        Held {
            k,
            domain: clock.domain,
            samples: Arc::new(vec![samples; channels.max(1)]),
            waveform_len,
            repeat: false,
            open: BurstOpen { waveform_len: None, late: None, requested_target: None },
            policy: LatePolicy::DropAndFlag,
            target: TimePoint::new(clock.domain, k),
        }
    }

    #[test]
    fn ur_34_a_transmit_move_ends_the_old_burst_and_sends_the_new_burst_on_its_stream() {
        // A7/S5 (design note 37): a move cuts the source's one-shot burst at the move's
        // instant — the last sample before it is played and the end-of-burst precedes the
        // release of the old streamer — and a burst booked on the destination's clock before
        // the cut is played by the destination's own owner after its switch.
        let (core, device, time) = manual_rig();
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        let c0 = core.register_in(Dir::Tx, 0, 200, 0).unwrap();
        let c1 = core.register_in(Dir::Tx, 1, 200, 0).unwrap();
        let mut values = core.description.defaults();
        values.insert(crate::provider::core::key("radio.tx.c0.stream"), Value::from(0));
        values.insert(crate::provider::core::key("radio.tx.c1.stream"), Value::from(1));
        let (to_tx, _keep) = std::sync::mpsc::channel();
        let mut control = super::super::control::Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], values, false,
            super::super::control::Start { t0: 0, rx: vec![None, None], tx: vec![Some((c0, Channels::first(1))), Some((c1, Channels::first(2).without(0)))] });
        let ms = |n: i64| core.ticks(n * 1_000_000);
        control.book(Dispatched::new(ActionId(1), Action::UpdateParameter {
            target: super::super::test_support::target_of(&core.id),
            key: crate::provider::core::key("radio.tx.c0.stream"),
            value: Value::from(1),
            class: UpdateClass::Cold,
            at: Some(AbsoluteDeadline::new(core.at(ms(30)))),
        }, Some(core.id.clone())));
        let s0 = lock(&core.streams).planned(stream_key(Dir::Tx, 0));
        assert_eq!(s0.len(), 1, "s0 has no channel after the move: {:?}", s0.iter().map(|p| p.segment.origin).collect::<Vec<_>>());
        assert_eq!(s0[0].segment.instant(s0[0].segment.cut.expect("s0 is cut")).unwrap(), ms(30));
        let destination = lock(&core.streams).planned(stream_key(Dir::Tx, 1)).into_iter().find(|planned| planned.segment.config.channels == Channels::first(2)).expect("s1's destination segment");
        let destination_clock = destination.clock.expect("the destination's clock is registered at booking");
        device.tx_open(0, &[0]).unwrap();
        device.tx_open(1, &[1]).unwrap();
        {
            let mut streams = lock(&core.streams);
            streams.record_open(stream_key(Dir::Tx, 0), Some(Channels::first(1)));
            streams.record_open(stream_key(Dir::Tx, 1), Some(Channels::first(2).without(0)));
        }
        let (_, cmds0) = std::sync::mpsc::channel();
        let (_, cmds1) = std::sync::mpsc::channel();
        let mut tx0 = Tx::new(core.clone(), 0, cmds0, Some(c0), Channels::first(1));
        let mut tx1 = Tx::new(core.clone(), 1, cmds1, Some(c1), Channels::first(2).without(0));
        assert!(tx0.read(None) && tx1.read(None), "the owners read the plan");
        // The old one-shot: 10 000 samples from 20 ms, so its end is the move's instant.
        let old_wave: Vec<[f32; 2]> = (0..10_000).map(|i| [i as f32 / 10_000.0, -0.125]).collect();
        lock(&core.held).insert((c0.domain, 20_000));
        tx0.command(TxCmd::Burst(one_shot(&c0, 20_000, 1, old_wave.clone())));
        // The destination's burst, booked before the cut: 1 000 samples 5 ms into its clock.
        let new_wave: Vec<[f32; 2]> = (0..1_000).map(|i| [0.75 - i as f32 / 4_000.0, 0.25]).collect();
        let new_k = ms(5) / 200;
        lock(&core.held).insert((destination_clock.domain, new_k));
        tx1.command(TxCmd::Burst(one_shot(&destination_clock, new_k, 2, new_wave.clone())));
        for k in 17..=29 {
            device_at(&core, &time, &device, ms(k));
            let _ = tx0.step();
        }
        assert!(device.transmitted(c0.instant(20_100), 1)[0].is_some(), "the old burst did not play: {:?}", device.calls());
        // At the cut the source's burst ends and the destination's switch waits for its
        // streamer release, so the two owners' turns run at once — each as a participant of
        // the device's clock, registered before the test gives the clock back (M1): a delayed
        // test thread then moves neither turn, and the clock stands at its poll's deadline.
        device_at(&core, &time, &device, ms(30));
        let (done, reached) = std::sync::mpsc::channel();
        let (release_destination, destination_release) = std::sync::mpsc::channel();
        let destination = device.host().spawn("tx-destination", move || { let _ = tx1.step(); let _ = done.send(tx1); let _ = destination_release.recv(); }).expect("a participant thread");
        let (source_done, source_reached) = std::sync::mpsc::channel();
        let source = device.host().spawn("tx-source", move || { let _ = tx0.step(); let _ = tx0.pass(); let _ = source_done.send(tx0.clock.is_none()); }).expect("a participant thread");
        drop(_frozen);
        let mut tx1 = reached.recv_timeout(Wall::from_secs(10)).expect("the destination's turn");
        assert!(tx1.clock.is_some(), "the destination adopted no clock: {:?}", device.calls());
        assert_eq!(source_reached.recv_timeout(Wall::from_secs(10)), Ok(true), "the source's owner kept its clock: {:?}", device.calls());
        // The destination is the last turn to finish — it waits for the source's release — and
        // past its result it stays a participant until the test holds the clock again (M1). The
        // source must end: a participant busy on the source's result would stop the clock, and
        // the destination's poll for the release would never end.
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        let _ = release_destination.send(());
        destination.join().expect("the destination's thread");
        source.join().expect("the source's thread");
        // The destination's own segment begins 50 ms after the cut; its prebooked burst, 5 ms
        // into that clock, is due 50 ms later still. Every instant now goes forward from where
        // the switch left it.
        let mut k = device.time_now().unwrap() + 1;
        while k <= ms(92) {
            device_at(&core, &time, &device, k);
            let _ = tx1.step();
            k += ms(1);
        }
        let calls = device.calls();
        let close = calls.iter().position(|call| call == "tx_close 0").unwrap_or_else(|| panic!("the source's release: {calls:?}"));
        let end = calls.iter().rposition(|call| call.starts_with("tx_send 0 ") && call.ends_with("eob=true")).expect("the old burst's end");
        assert!(end < close, "the old burst's end precedes the source's close: {calls:?}");
        let last = device.transmitted(ms(30) - 200, 1)[0].expect("the last sample before the cut");
        assert_eq!(last, old_wave[9_999], "the cut cut the old waveform: {calls:?}");
        let new_at = destination_clock.instant(new_k + 100);
        assert_eq!(device.transmitted(new_at, 1)[0], Some(new_wave[100]), "the destination's burst plays its own samples: {calls:?}");
        assert!(calls.iter().any(|call| call == "tx_open 1 [0, 1]"), "the destination opens both channels: {calls:?}");
        assert_eq!(device.unended_bursts(), 0, "{calls:?}");
        assert!(calls.iter().all(|call| !call.contains("Attempting to reconnect")), "{calls:?}");
        let ended = core.clocks.sample_clock_records().into_iter().find(|record| record.domain == c0.domain).and_then(|record| record.ended_at)
            .expect("the source's clock ends at the cut");
        assert_eq!(ended.ticks_in(core.root).unwrap(), ms(30), "the source's clock does not end at the move: {ended:?}");
    }

    #[test]
    fn ur_34_a_transmit_move_carries_a_prebooked_burst_across_the_repeat_cut() {
        // A7 (design note 37): an old repeating burst crosses its waveform wrap and is cut at
        // the move's instant, while a burst already booked on the destination's clock waits
        // through the move and its own owner plays it after the switch.
        let (core, device, time) = manual_rig();
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        let c0 = core.register_in(Dir::Tx, 0, 200, 0).unwrap();
        let c1 = core.register_in(Dir::Tx, 1, 200, 0).unwrap();
        let mut values = core.description.defaults();
        values.insert(crate::provider::core::key("radio.tx.c0.stream"), Value::from(0));
        values.insert(crate::provider::core::key("radio.tx.c1.stream"), Value::from(1));
        let (to_tx, _keep) = std::sync::mpsc::channel();
        let mut control = super::super::control::Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], values, false,
            super::super::control::Start { t0: 0, rx: vec![None, None], tx: vec![Some((c0, Channels::first(1))), Some((c1, Channels::first(2).without(0)))] });
        let ms = |n: i64| core.ticks(n * 1_000_000);
        // The cut is two and a half waveforms past the repeat's start: the wrap has happened
        // and the cut falls inside the third.
        control.book(Dispatched::new(ActionId(1), Action::UpdateParameter {
            target: super::super::test_support::target_of(&core.id),
            key: crate::provider::core::key("radio.tx.c0.stream"),
            value: Value::from(1),
            class: UpdateClass::Cold,
            at: Some(AbsoluteDeadline::new(core.at(ms(22) + core.ticks(500_000)))),
        }, Some(core.id.clone())));
        let destination = lock(&core.streams).planned(stream_key(Dir::Tx, 1)).into_iter().find(|planned| planned.segment.config.channels == Channels::first(2)).expect("s1's destination segment");
        let destination_clock = destination.clock.expect("the destination's clock is registered at booking");
        device.tx_open(0, &[0]).unwrap();
        device.tx_open(1, &[1]).unwrap();
        {
            let mut streams = lock(&core.streams);
            streams.record_open(stream_key(Dir::Tx, 0), Some(Channels::first(1)));
            streams.record_open(stream_key(Dir::Tx, 1), Some(Channels::first(2).without(0)));
        }
        let (_, cmds0) = std::sync::mpsc::channel();
        let (_, cmds1) = std::sync::mpsc::channel();
        let mut tx0 = Tx::new(core.clone(), 0, cmds0, Some(c0), Channels::first(1));
        let mut tx1 = Tx::new(core.clone(), 1, cmds1, Some(c1), Channels::first(2).without(0));
        assert!(tx0.read(None) && tx1.read(None), "the owners read the plan");
        let old_wave: Vec<[f32; 2]> = (0..1_000).map(|i| [i as f32 / 1_000.0, -0.5]).collect();
        let mut old = one_shot(&c0, 20_000, 1, old_wave.clone());
        old.repeat = true;
        lock(&core.held).insert((c0.domain, 20_000));
        tx0.command(TxCmd::Burst(old));
        let new_wave: Vec<[f32; 2]> = (0..1_000).map(|i| [0.5 + i as f32 / 4_000.0, 0.125]).collect();
        let new_k = ms(5) / 200;
        lock(&core.held).insert((destination_clock.domain, new_k));
        tx1.command(TxCmd::Burst(one_shot(&destination_clock, new_k, 2, new_wave.clone())));
        for k in 17..=21 {
            device_at(&core, &time, &device, ms(k));
            let _ = tx0.step();
        }
        // The repeat crossed its wrap: the second waveform's 100th sample is its own.
        assert_eq!(device.transmitted(c0.instant(21_100), 1)[0], Some(old_wave[100]), "the repeat did not cross its wrap: {:?}", device.calls());
        device_at(&core, &time, &device, ms(22) + core.ticks(500_000));
        // The two owners' turns run at once as participants of the device's clock, registered
        // before the test gives the clock back (M1), as in the one-shot move above.
        let (done, reached) = std::sync::mpsc::channel();
        let (release_destination, destination_release) = std::sync::mpsc::channel();
        let destination = device.host().spawn("tx-destination", move || { let _ = tx1.step(); let _ = done.send(tx1); let _ = destination_release.recv(); }).expect("a participant thread");
        let (source_done, source_reached) = std::sync::mpsc::channel();
        let source = device.host().spawn("tx-source", move || { let _ = tx0.step(); let _ = tx0.pass(); let _ = source_done.send(tx0.clock.is_none()); }).expect("a participant thread");
        drop(_frozen);
        let mut tx1 = reached.recv_timeout(Wall::from_secs(10)).expect("the destination's turn");
        assert!(tx1.clock.is_some(), "the destination adopted no clock: {:?}", device.calls());
        assert_eq!(source_reached.recv_timeout(Wall::from_secs(10)), Ok(true), "the source's owner kept its clock: {:?}", device.calls());
        // As in the one-shot move above, the destination — the last turn to finish — stays a
        // participant past its result until the test holds the clock again (M1), so the clock
        // does not run between the turns and the test's own instants.
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        let _ = release_destination.send(());
        destination.join().expect("the destination's thread");
        source.join().expect("the source's thread");
        // Each instant goes forward from where the switch left it, to the destination's tenth
        // millisecond.
        let until = destination_clock.origin + ms(10);
        let mut k = device.time_now().unwrap() + 1;
        while k <= until {
            device_at(&core, &time, &device, k);
            let _ = tx1.step();
            k += ms(1);
        }
        let calls = device.calls();
        let close = calls.iter().position(|call| call == "tx_close 0").unwrap_or_else(|| panic!("the source's release: {calls:?}"));
        let end = calls.iter().rposition(|call| call.starts_with("tx_send 0 ") && call.ends_with("eob=true")).expect("the repeat's end");
        assert!(end < close, "the repeat's end precedes the source's close: {calls:?}");
        let last_at = ms(22) + core.ticks(500_000) - 200;
        assert_eq!(device.transmitted(last_at, 1)[0], Some(old_wave[499]), "the repeat reaches its last sample before the cut: {calls:?}");
        let new_at = destination_clock.instant(new_k + 100);
        assert_eq!(device.transmitted(new_at, 1)[0], Some(new_wave[100]), "the burst reserved on the new clock is played by the destination: {calls:?}");
        let ended = core.clocks.sample_clock_records().into_iter().find(|record| record.domain == c0.domain).and_then(|record| record.ended_at)
            .expect("the source's clock ends at the cut");
        assert_eq!(ended.ticks_in(core.root).unwrap(), ms(22) + core.ticks(500_000), "the repeat is not cut at the booked instant: {ended:?}");
        assert_eq!(device.unended_bursts(), 0, "{calls:?}");
    }

    #[test]
    fn ur_23_a_burst_booked_at_a_sent_burst_s_end_continues_it() {
        // T1: B booked at A's end after A's last buffer went out (inside the in-flight
        // window) is played, continuing A's device burst: A's held last sample ahead of B's
        // first, one device burst of 31 000 samples, no end-of-burst between.
        let (core, device, time, mut tx, _frozen) = at_a_held_tail(Vec::new());
        let clock = tx.clock.expect("a transmit clock");
        let b = continuation(&clock, 70_000, 1_000);
        let second = b.samples[0].clone();
        lock(&core.held).insert((clock.domain, 70_000));
        tx.command(TxCmd::Burst(b));
        device_at(&core, &time, &device, core.ticks(62_000_000));
        assert!(tx.step(), "A's held sample is not taken ahead of B");
        device_at(&core, &time, &device, core.ticks(69_000_000));
        assert!(tx.step(), "B's held sample is not ended");
        assert!(tx.open.is_none() && tx.continues_at.is_none() && tx.tail.is_none());
        let whole: Vec<Option<[f32; 2]>> = (0..30_000).map(|_| Some([0.25, 0.0])).chain(second.iter().copied().map(Some)).collect();
        assert_eq!(device.transmitted(clock.instant(40_000), 31_000), whole, "{:?}", device.calls());
        let sends = sends(&device);
        assert_eq!(sends.iter().filter(|(_, sob, _)| *sob).count(), 1, "{sends:?}");
        assert_eq!(sends.iter().filter(|(_, _, eob)| *eob).count(), 1, "{sends:?}");
        assert_eq!(sends.iter().map(|(n, _, _)| n).sum::<usize>(), 31_000, "{sends:?}");
        assert_eq!(device.unended_bursts(), 0, "{sends:?}");
        assert!(lock(&core.rec).rejected.iter().all(|row| !row["reason"].as_str().unwrap_or("").starts_with("UR-21")
            && !row["reason"].as_str().unwrap_or("").starts_with("UR-22")), "{:?}", lock(&core.rec).rejected);
        tx.reports(Wall::ZERO);
        assert!(lock(&core.rec).device_async.iter().all(|row| row["code"] != "TimeError" && row["code"] != "Underflow"),
            "{:?}", lock(&core.rec).device_async);
    }

    #[test]
    fn ur_23_a_stop_while_the_device_burst_waits_for_a_continuation_ends_it() {
        // T2: a `Stop` while the device burst waits for a continuation ends it with one
        // end-of-burst, A's held last sample, before it runs dry.
        let (core, device, time, mut tx, _frozen) = at_a_held_tail(Vec::new());
        let clock = tx.clock.expect("a transmit clock");
        device_at(&core, &time, &device, core.ticks(63_000_000));
        tx.stop_now("test stop", ezsdr_radio::CancelReason::Stop);
        let mut whole: Vec<Option<[f32; 2]>> = (0..30_000).map(|_| Some([0.25, 0.0])).collect();
        whole.push(None);
        assert_eq!(device.transmitted(clock.instant(40_000), 30_001), whole, "{:?}", device.calls());
        let sends = sends(&device);
        // A's held last sample is the end-of-burst's own buffer: an empty end-of-burst would
        // play its zero sample where A ended (70 000), not after it.
        assert_eq!(sends.last(), Some(&(1, false, true)), "{sends:?}");
        assert_eq!(sends.iter().filter(|(_, _, eob)| *eob).count(), 1, "{sends:?}");
        assert!(tx.open.is_none() && tx.continues_at.is_none() && tx.tail.is_none());
        assert_eq!(device.unended_bursts(), 0, "{sends:?}");
        tx.reports(Wall::ZERO);
        assert!(lock(&core.rec).device_async.iter().all(|row| row["code"] != "Underflow"), "{:?}", lock(&core.rec).device_async);
    }

    #[test]
    fn ur_23_a_stop_ends_the_burst_within_the_in_flight_window() {
        // UR-23, RM-16: payload stops at the first SampleClock sample at or after `s + W`;
        // the empty EOB's padding is separate. The fake's Session test reads the receipt instant.
        let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
        let (core, device, time, _) = crate::provider::test_support::rig_with(config, Vec::new());
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        std::mem::forget(_to_tx);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        lock(&core.held).insert((clock.domain, 20_000));
        tx.command(TxCmd::Burst(held(&clock, 20_000, 30_000, LatePolicy::DropAndFlag)));
        // First hand-over at 12 ms: A's start 9 ms ahead, one 2 000-sample buffer sent, its
        // end exactly one window after this instant.
        device_at(&core, &time, &device, core.ticks(12_000_000));
        for _ in 0..8 {
            tx.step();
        }
        let open = tx.open.as_ref().expect("A is open");
        assert_eq!(open.next, 22_000, "the hand-over went past the in-flight window: {:?}", device.calls());
        tx.stop_now("test stop", ezsdr_radio::CancelReason::Stop);
        assert!(tx.open.is_none() && tx.continues_at.is_none() && tx.tail.is_none());
        let window = core.ticks(core.description.timing.tx_in_flight_ns);
        let stop = core.now();
        let record = lock(&core.rec).bursts.last().cloned().expect("A's record");
        assert_eq!(record["end"], "stop", "{record}");
        let target = record["target"]["ticks"].as_i64().expect("the record's target");
        let end = clock.instant(target + record["samples"].as_i64().expect("the record's samples"));
        assert!(end <= stop + window, "the record ends {} ms after the stop", (end - stop) / core.ticks(1_000_000));
        // The device's own end is the record's plus the empty end-of-burst's one zero sample
        // (UR-23's bench record); its acknowledgement is due once that instant has played.
        device_at(&core, &time, &device, end + clock.n);
        tx.reports(Wall::ZERO);
        let ack = lock(&core.rec).device_async.iter().find(|row| row["code"] == "BurstAck").expect("the end-of-burst's acknowledgement")["tick"].as_i64().unwrap();
        assert_eq!(ack, end + clock.n, "{:?}", lock(&core.rec).device_async);
        assert!(lock(&core.rec).device_async.iter().all(|row| row["code"] != "TimeError" && row["code"] != "Underflow"), "{:?}", lock(&core.rec).device_async);
    }

    #[test]
    fn ur_23_a_burst_one_sample_after_a_burst_is_played() {
        // T3: A's device burst ends at c, with no padded sample at c + 1, so a burst booked
        // at c + 1 is played: the device's samples are A, a gap at c, then B.
        let (core, device, time, mut tx, _frozen) = at_a_held_tail(Vec::new());
        let clock = tx.clock.expect("a transmit clock");
        let b = continuation(&clock, 70_001, 1_000);
        let second = b.samples[0].clone();
        lock(&core.held).insert((clock.domain, 70_001));
        tx.command(TxCmd::Burst(b));
        device_at(&core, &time, &device, core.ticks(62_000_000));
        assert!(tx.step(), "A's device burst is not ended at c");
        assert!(tx.step(), "B is not dispatched at c + 1");
        device_at(&core, &time, &device, core.ticks(69_000_000) + 20_000);
        assert!(tx.step(), "B's held sample is not ended");
        assert!(tx.open.is_none() && tx.continues_at.is_none() && tx.tail.is_none());
        let mut whole: Vec<Option<[f32; 2]>> = (0..30_000).map(|_| Some([0.25, 0.0])).collect();
        whole.push(None);
        whole.extend(second.iter().copied().map(Some));
        assert_eq!(device.transmitted(clock.instant(40_000), 31_001), whole, "{:?}", device.calls());
        let sends = sends(&device);
        assert_eq!(sends.iter().filter(|(_, sob, _)| *sob).count(), 2, "{sends:?}");
        assert_eq!(sends.iter().filter(|(_, _, eob)| *eob).count(), 2, "{sends:?}");
        assert_eq!(device.unended_bursts(), 0, "{sends:?}");
        tx.reports(Wall::ZERO);
        assert!(lock(&core.rec).device_async.iter().all(|row| row["code"] != "TimeError" && row["code"] != "Underflow"),
            "{:?}", lock(&core.rec).device_async);
    }

    #[test]
    fn ur_23_a_continuation_whose_held_sample_is_not_taken_is_abandoned() {
        // T4: A's held sample's send ahead of B takes nothing, or fails: B is abandoned, and
        // the device burst ended with an empty end-of-burst after A's 29 999 samples.
        for (fault, reason) in [(crate::device::FakeFault::StalledSend(13), "UR-22: a send did not complete"),
            (crate::device::FakeFault::FailSend(13), "fake: the send failed")] {
            let what = format!("{fault:?}");
            let (core, device, time, mut tx, _frozen) = at_a_held_tail(vec![fault]);
            let clock = tx.clock.expect("a transmit clock");
            lock(&core.held).insert((clock.domain, 70_000));
            tx.command(TxCmd::Burst(continuation(&clock, 70_000, 1_000)));
            device_at(&core, &time, &device, core.ticks(62_000_000));
            assert!(tx.step(), "{what}: the held sample's send was not entered");
            let sent = device.transmitted(clock.instant(40_000), 30_001);
            let whole: Vec<Option<[f32; 2]>> = (0..30_000).map(|_| Some([0.25, 0.0])).collect();
            assert_eq!(sent[..29_999], whole[..29_999], "{what}: {:?}", device.calls());
            assert_eq!(sent[29_999], Some([0.0, 0.0]), "{what}: B played in A's last sample's place: {:?}", device.calls());
            assert_eq!(sent[30_000], None, "{what}: {:?}", device.calls());
            let sends = sends(&device);
            assert_eq!(sends.iter().filter(|(_, _, eob)| *eob).count(), 1, "{what}: {sends:?}");
            assert_eq!(device.unended_bursts(), 0, "{what}: {sends:?}");
            assert!(lock(&core.rec).rejected.iter().any(|row| row["reason"].as_str().unwrap_or("").contains(reason)),
                "{what}: {:?} {sends:?}", lock(&core.rec).rejected);
            assert!(device.calls().iter().any(|call| call.contains("stalled") || call.contains("failed")), "{what}: the case was not reached: {:?}", device.calls());
        }
    }

    #[test]
    fn ur_23_a_continuation_s_first_send_that_takes_nothing_ends_the_device_burst() {
        // T5: B continues A's device burst; B's first payload send takes nothing. The device
        // burst is still open and ends with an empty end-of-burst after A's last sample.
        let (core, device, time, mut tx, _frozen) = at_a_held_tail(vec![crate::device::FakeFault::StalledSend(14)]);
        let clock = tx.clock.expect("a transmit clock");
        lock(&core.held).insert((clock.domain, 70_000));
        tx.command(TxCmd::Burst(continuation(&clock, 70_000, 1_000)));
        device_at(&core, &time, &device, core.ticks(62_000_000));
        assert!(tx.step(), "B's first send was not entered");
        assert!(device.calls().iter().any(|call| call.contains("stalled")), "the case was not reached: {:?}", device.calls());
        let sent = device.transmitted(clock.instant(40_000), 30_002);
        let whole: Vec<Option<[f32; 2]>> = (0..30_000).map(|_| Some([0.25, 0.0])).collect();
        assert_eq!(sent[..30_000], whole[..], "{:?}", device.calls());
        assert_eq!(sent[30_000], Some([0.0, 0.0]), "{:?}", device.calls());
        assert_eq!(sent[30_001], None, "{:?}", device.calls());
        let sends = sends(&device);
        assert_eq!(sends.iter().filter(|(_, _, eob)| *eob).count(), 1, "{sends:?}");
        assert_eq!(device.unended_bursts(), 0, "{sends:?}");
        tx.reports(Wall::ZERO);
        assert!(lock(&core.rec).device_async.iter().all(|row| row["code"] != "Underflow"), "{:?}", lock(&core.rec).device_async);
        assert!(lock(&core.rec).rejected.iter().any(|row| row["reason"].as_str().unwrap_or("").contains("UR-22")), "{:?}", lock(&core.rec).rejected);
    }

    #[test]
    fn ur_23_bursts_go_out_in_target_order() {
        // RM-15: held bursts go out in target order, whatever order they were booked in. The
        // Spec sorts its entries by instant, so its two bursts arrive in target order; the
        // reverse booking is pinned here, on the controlled clock.
        let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
        let (core, device, time, _) = crate::provider::test_support::rig_with(config, Vec::new());
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        std::mem::forget(_to_tx);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        // 200 ms first, then 100 ms: the later booking has the nearer target.
        for k in [200_000, 100_000] {
            lock(&core.held).insert((clock.domain, k));
            tx.command(TxCmd::Burst(held(&clock, k, 2_000, LatePolicy::DropAndFlag)));
        }
        device_at(&core, &time, &device, core.ticks(91_000_000));
        assert!(tx.step(), "the nearer burst is not dispatched");
        let sends = sends(&device);
        assert_eq!(sends.len(), 1, "{sends:?}");
        assert!(sends[0].0 == 1_000 && sends[0].1 && !sends[0].2, "the nearer burst did not go first within the strict window: {sends:?}");
        assert!(device.calls().iter().any(|call| call.contains("at=20000000 sob=true")), "the 100 ms burst's timed start: {:?}", device.calls());
        assert!(tx.held.contains_key(&200_000), "the farther burst is still held");
    }

    #[test]
    fn ur_22_a_first_send_cut_short_keeps_the_reports_paired() {
        // Review Q, TG-Q1: A's acknowledgement stays paired ahead of B's late-at-device
        // report after a short send or an error that may have accepted samples.
        for (fault, cursor) in [
            (crate::device::FakeFault::ShortSend(0), 40_499),
            (crate::device::FakeFault::FailSend(0), 40_999),
        ] {
            let what = format!("{fault:?}");
            let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, faults: vec![fault], ..FakeConfig::default() };
            let (core, device, time, events) = crate::provider::test_support::rig_with(config, Vec::new());
            let _frozen = crate::provider::test_support::Frozen::of(&device);
            device_at(&core, &time, &device, 0);
            device.tx_open(0, &[0]).unwrap();
            device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
            let clock = core.register(Dir::Tx, 200, 0).unwrap();
            let (_to_tx, cmds) = std::sync::mpsc::channel();
            std::mem::forget(_to_tx);
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            let a: Vec<[f32; 2]> = (0..1_000).map(|i| [0.25 + i as f32 / 4_000.0, -0.125]).collect();
            let mut burst = held(&clock, 40_000, 1_000, LatePolicy::DropAndFlag);
            burst.samples = Arc::new(vec![a.clone()]);
            lock(&core.held).insert((clock.domain, 40_000));
            tx.command(TxCmd::Burst(burst));
            device_at(&core, &time, &device, core.ticks(31_000_000));
            assert!(tx.step(), "{what}: A's first send");
            assert!(tx.open.is_none(), "{what}: A was not abandoned: {:?}", device.calls());
            let sent = device.transmitted(clock.instant(40_000), 501);
            assert_eq!(sent[..499], a[..499].iter().copied().map(Some).collect::<Vec<_>>(), "{what}: {:?}", device.calls());
            assert_eq!(sent[499], Some([0.0, 0.0]), "{what}: after what A took: {:?}", device.calls());
            assert_eq!(sent[500], None, "{what}: {:?}", device.calls());
            // The accepted (or conservatively attempted) cursor plus two is the first safe start.
            let safe = cursor + 2;
            lock(&core.held).insert((clock.domain, safe));
            tx.command(TxCmd::Burst(continuation(&clock, safe, 1_000)));
            device_at(&core, &time, &device, core.ticks(31_002_000));
            // The device gets past B's safe target while the owner's manual clock stays fixed.
            device.set_time(core.ticks(70_000_000));
            assert!(tx.step(), "{what}: B's timed send");
            device_at(&core, &time, &device, core.ticks(70_000_000));
            tx.reports(Wall::ZERO);
            let late: Vec<_> = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
                .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
            assert_eq!(late.len(), 1, "{what}: {:?}", lock(&core.rec).device_async);
            assert_eq!(late[0].payload["outcome"], "late_at_device", "{what}: {late:?}");
            assert_eq!(late[0].payload["target"]["ticks"], safe, "{what}: the report names B: {late:?}");
            assert!(lock(&core.rec).rejected.iter().any(|row| row["reason"].as_str().unwrap_or("").contains("fake: the send failed")
                || row["reason"].as_str().unwrap_or("").contains("UR-22: a send did not complete")), "{what}: {:?}", lock(&core.rec).rejected);
        }
    }

    #[test]
    fn ur_22_a_send_cut_short_ends_the_device_burst_after_what_it_took() {
        // Review P, NB-1: a send that comes up short abandons the burst. The device burst is
        // closed with an empty end-of-burst (one zero sample) straight after the samples the
        // device took; the burst's held last sample is not sent there. The 6 000-sample case
        // cuts the final data buffer (the 4th device send); the last sample is held inside the
        // in-flight window (D3). A send that fails after taking half is ended the same way
        // (Review Q, NB-Q1). The steps are placed by hand on the manual clock, so the first
        // buffer's size does not depend on when a thread ran (design note 37).
        let cases = [(crate::device::FakeFault::ShortSend(0), 1_000usize), (crate::device::FakeFault::ShortSend(1), 3_000), (crate::device::FakeFault::ShortSend(3), 6_000), (crate::device::FakeFault::FailSend(0), 1_000)];
        for (fault, len) in cases {
            let nth = format!("{fault:?}");
            let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, faults: vec![fault], ..FakeConfig::default() };
            let (core, device, time, _events) = crate::provider::test_support::rig_with(config, Vec::new());
            let _frozen = crate::provider::test_support::Frozen::of(&device);
            device_at(&core, &time, &device, 0);
            device.tx_open(0, &[0]).unwrap();
            device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
            let clock = core.register(Dir::Tx, 200, 0).unwrap();
            let (_to_tx, cmds) = std::sync::mpsc::channel();
            std::mem::forget(_to_tx);
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            let wave: Vec<[f32; 2]> = (0..len).map(|i| [0.25 + i as f32 / 40_000.0, -0.125]).collect();
            let mut burst = held(&clock, 40_000, len, LatePolicy::DropAndFlag);
            burst.samples = Arc::new(vec![wave.clone()]);
            lock(&core.held).insert((clock.domain, 40_000));
            tx.command(TxCmd::Burst(burst));
            // The first buffer is 100 samples (the window reaches 100 samples past the start),
            // then a whole 2 000-sample buffer once the window takes it (UR-23), one per ten 200 us steps:
            // the 4th device send is the final data buffer of the 6 000-sample case, whose last sample is held.
            let mut at = 30_100_000;
            for _ in 0..80 {
                device_at(&core, &time, &device, core.ticks(at));
                tx.step();
                at += 200_000;
                if tx.open.is_none() { break; }
            }
            let sends = sends(&device);
            assert!(tx.open.is_none(), "{nth}: the device burst was not ended: {:?}", device.calls());
            let rejected = lock(&core.rec).rejected.clone();
            assert!(rejected.iter().any(|r| r["reason"].as_str().unwrap_or("").contains("UR-22: a send did not complete") || r["reason"].as_str().unwrap_or("").contains("fake: the send failed")), "{nth}: {rejected:?} {sends:?}");
            let sent = device.transmitted(clock.instant(40_000), len + 1);
            let took = sent.iter().zip(&wave).take_while(|(s, w)| **s == Some(**w)).count();
            assert!(took < len - 1, "nth {nth}: {took} of {len} {sends:?}");
            assert_eq!(sent[took], Some([0.0, 0.0]), "nth {nth}: after {took} {sends:?}");
            assert!(sent[took + 1..].iter().all(Option::is_none), "nth {nth}: {sends:?}");
            assert_eq!(sends.iter().filter(|s| s.2).count(), 1, "nth {nth}: {sends:?}");
            if len == 6_000 { assert!(took > 5_000, "nth {nth}: the cut was not in the final buffer: {took} {sends:?}"); }
        }

        for fault in [crate::device::FakeFault::ShortSend(1), crate::device::FakeFault::FailSend(1)] {
            let (core, device, time, _events, clock, mut tx, _frozen) = txp_high_rate_rig(1, vec![fault.clone()], BTreeMap::new());
            let start = 500_000;
            device_at(&core, &time, &device, clock.instant(start - 400_000));
            let wave: Vec<Iq> = (0..997).map(|i| [i as f32 / 1_000.0, -(i as f32) / 2_000.0]).collect();
            let mut burst = txp_burst(&clock, start, wave.len(), true, LatePolicy::DropAndFlag);
            burst.samples = Arc::new(vec![wave.clone()]);
            txp_give(&mut tx, &core, burst);
            assert!(tx.step(), "{fault:?}: first tiled send");
            assert!(tx.step(), "{fault:?}: the second tiled send is cut");
            assert!(tx.open.is_none(), "{fault:?}: a partial/error send is abandoned");
            let handed = if matches!(&fault, FakeFault::ShortSend(_)) { 10_000 } else { 20_000 };
            assert_eq!(tx.front, Some((clock.domain, start + 20_000 + handed + 2)), "{fault:?}: recovery uses the accepted or conservatively attempted cursor");
            let sent = device.transmitted(clock.instant(start), 30_002);
            let expected: Vec<_> = (0..30_000).map(|i| Some(wave[i % wave.len()])).collect();
            assert_eq!(&sent[..30_000], expected, "{fault:?}: the accepted prefix keeps the original repeat phase");
            assert_eq!(sent[30_000], Some([0.0, 0.0]), "{fault:?}: empty EOB follows what the device accepted");
            assert!(sent[30_001..].iter().all(Option::is_none), "{fault:?}: the unsent tiled suffix is never replayed");
            assert!(lock(&core.rec).rejected.iter().any(|row| row["reason"].as_str().is_some_and(|reason| reason.contains("send") || reason.contains("failed"))), "{fault:?}: {:?}", lock(&core.rec).rejected);
        }
    }

    #[test]
    fn ur_23_abandoned_send_preserves_the_accepted_cursor() {
        let a_samples = vec![[0.25, 0.0]; 2_000];
        let (a_wave, a_bytes) = waveform_input("abandoned_repeat", &a_samples);
        let (b_wave, b_bytes) = waveform_input("after_abandon", &[[0.75, -0.25]]);
        for (fault, cursor) in [
            (crate::device::FakeFault::ShortSend(0), 40_500),
            (crate::device::FakeFault::FailSend(0), 41_000),
        ] {
            let case = format!("{fault:?}");
            let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, faults: vec![fault], ..FakeConfig::default() };
            let device = Arc::new(FakeDevice::new(config));
            let inputs = BTreeMap::from([
                (a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes.clone())),
                (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes.clone())),
            ]);
            let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
            let _frozen = crate::provider::test_support::Frozen::of(&device);
            device_at(&core, &time, &device, 0);
            device.tx_open(0, &[0]).unwrap();
            device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
            let (mut control, cmds) = booking_control(&core);
            let clock = plan_of(&core, Dir::Tx)[0].clock.expect("the transmit clock");
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            book_burst(&mut control, &core, 1, clock, 40_000, &a_wave, true, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            device_at(&core, &time, &device, core.ticks(31_000_000));
            assert!(tx.step(), "{case}: A was not sent");
            let safe = cursor + 2;
            assert_eq!(tx.front, Some((clock.domain, safe)), "{case}: recovery starts after the samples accepted or conservatively counted");

            let calls = device.calls().len();
            book_burst(&mut control, &core, 2, clock, 40_499, &b_wave, false, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            assert!(tx.step(), "{case}: B inside A's attempted range was not judged");
            let late = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
                .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect::<Vec<_>>();
            assert_eq!(late.len(), 1, "{case}: {late:?}");
            assert_eq!(late[0].payload["outcome"], "drop", "{case}");
            assert_eq!(late[0].payload["target"]["ticks"], 40_499, "{case}");
            assert_eq!(device.calls().len(), calls, "{case}: the late B reached the device");

            device_at(&core, &time, &device, core.ticks(31_002_000));
            book_burst(&mut control, &core, 3, clock, cursor + 1, &b_wave, false, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            assert!(tx.step(), "{case}: B one sample before the safe restart was not judged");
            let late = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
                .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect::<Vec<_>>();
            assert_eq!(late.len(), 1, "{case}: {late:?}");
            assert_eq!(late[0].payload["outcome"], "drop", "{case}");
            assert_eq!(late[0].payload["target"]["ticks"], cursor + 1, "{case}");

            device_at(&core, &time, &device, core.ticks(31_003_000));
            book_burst(&mut control, &core, 4, clock, safe, &b_wave, false, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            assert!(tx.step(), "{case}: the safe restart was not admitted");
            assert!(events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter()
                .all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "{case}: the safe restart was late");
            assert_eq!(device.transmitted(clock.instant(safe), 1), vec![Some([0.75, -0.25])], "{case}: the safe sample was sent");
        }
    }

    #[test]
    fn ur_23_a_continuation_tail_is_counted_at_recovery() {
        let a_samples = vec![[0.25, 0.0]; 6_000];
        let b_samples = vec![[0.75, -0.25]; 1_000];
        let (a_wave, a_bytes) = waveform_input("tail_a", &a_samples);
        let (b_wave, b_bytes) = waveform_input("tail_b", &b_samples);
        for (fault, cursor, tail_taken, payload_taken) in [
            (crate::device::FakeFault::ShortSend(4), 8_499, true, 499),
            (crate::device::FakeFault::FailSend(4), 8_999, true, 499),
            (crate::device::FakeFault::FailSend(3), 8_000, false, 0),
        ] {
            let case = format!("{fault:?}");
            let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, faults: vec![fault], ..FakeConfig::default() };
            let device = Arc::new(FakeDevice::new(config));
            let inputs = BTreeMap::from([
                (a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes.clone())),
                (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes.clone())),
            ]);
            let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
            let _frozen = crate::provider::test_support::Frozen::of(&device);
            device_at(&core, &time, &device, 0);
            device.tx_open(0, &[0]).unwrap();
            device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
            let (mut control, cmds) = booking_control(&core);
            let clock = plan_of(&core, Dir::Tx)[0].clock.expect("the transmit clock");
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            book_burst(&mut control, &core, 1, clock, 2_000, &a_wave, false, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            for _ in 0..3 { assert!(tx.step(), "{case}: A's chunks"); }
            assert!(tx.open.is_none() && tx.continues_at == Some(8_000) && tx.tail.as_ref().is_some_and(|tail| tail.channels == 1), "{case}: A's held tail");
            assert_eq!(sends(&device).len(), 3, "{case}: the tail must still be held");

            book_burst(&mut control, &core, 2, clock, 5_000, &b_wave, false, LatePolicy::SendAsapAndFlag);
            drain_tx(&mut tx);
            assert!(tx.step(), "{case}: B's late continuation decision");
            assert!(tx.held.contains_key(&8_000), "{case}: B did not move to A's end");
            assert!(tx.step(), "{case}: B did not send the tail and continue");
            let moved = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
                .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect::<Vec<_>>();
            assert_eq!(moved.len(), 1, "{case}: {moved:?}");
            assert_eq!(moved[0].payload["outcome"], "send_asap", "{case}");
            let safe = cursor + 2;
            assert_eq!(tx.front, Some((clock.domain, safe)), "{case}: B's base cursor and recovery payload samples");
            assert!(tx.open.is_none(), "{case}: the incomplete continuation was resumed");
            let mut expected = vec![Some([0.25, 0.0]); 5_999];
            if tail_taken { expected.push(Some([0.25, 0.0])); }
            expected.extend(b_samples[..payload_taken].iter().copied().map(Some));
            expected.push(Some([0.0, 0.0]));
            expected.resize((safe - 2_000) as usize, None);
            assert_eq!(device.transmitted(clock.instant(2_000), expected.len()), expected,
                "{case}: FakeDevice's accepted samples and padding may precede the conservative recovery cursor");

            let calls = device.calls().len();
            book_burst(&mut control, &core, 3, clock, safe - 1, &b_wave, false, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            assert!(tx.step(), "{case}: the sample before the safe restart was not judged");
            let late = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
                .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect::<Vec<_>>();
            assert_eq!(late.len(), 1, "{case}: {late:?}");
            assert_eq!(late[0].payload["outcome"], "drop", "{case}");
            assert_eq!(device.calls().len(), calls, "{case}: the late restart reached the device");

            book_burst(&mut control, &core, 4, clock, safe, &b_wave, false, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            assert!(tx.step(), "{case}: the safe restart was not admitted");
            assert!(events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter()
                .all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "{case}: the safe restart was late");
            assert_eq!(device.transmitted(clock.instant(safe), 1), vec![Some([0.75, -0.25])], "{case}: the safe sample was sent");
        }
    }

    #[test]
    fn ur_26_shutdown_after_loss_cancels_held_bursts_record_only() {
        let (core, device, _, events) = super::super::test_support::rig();
        let current = core.register(Dir::Tx, 200, 0).unwrap();
        let future = core.register(Dir::Tx, 200, core.ticks(100_000_000)).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(current), Channels::first(1));
        tx.held.insert(20, held(&current, 20, 1, LatePolicy::DropAndFlag));
        tx.later.push(held(&future, 30, 1, LatePolicy::DropAndFlag));
        lock(&core.held).extend([(current.domain, 20), (future.domain, 30)]);
        core.device_lost("fake loss before the burst start");

        tx.shutdown();

        assert!(tx.held.is_empty());
        assert!(tx.later.is_empty());
        assert!(!lock(&core.held).contains(&(current.domain, 20)));
        assert!(!lock(&core.held).contains(&(future.domain, 30)));
        let rejected = lock(&core.rec).rejected.clone();
        assert_eq!(rejected.len(), 2, "{rejected:?}");
        assert!(rejected.iter().all(|row| row["reason"] == "UR-26: cancelled by Provider::stop"), "{rejected:?}");
        let events = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        assert!(events.iter().any(|event| event.kind.as_str() == ezsdr_kernel::event::EventKind::DEVICE_LOST));
        assert!(!events.iter().any(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED));
        assert!(!device.calls().iter().any(|call| call.starts_with("tx_send ")));
    }

    // UR-23: the window's last stretch waits for a whole buffer (stage TXP). The unit scale: 1 MS/s on a manual clock,
    // 200 root ticks a sample, window 10 000 samples, chunk 2 500 (window/4), device lead 2 000 samples.

    /// A transmit owner on a frozen manual clock: the window `window_ns` (the default 10 ms when `None`) and RX `block_len`.
    #[allow(clippy::type_complexity)]
    fn txp_rig(window_ns: Option<i64>, block_len: u32) -> (Arc<Core>, Arc<FakeDevice>, Arc<ezsdr_kernel::time::ManualTimeAuthority>, Arc<EventCollector>, Clock, Tx, crate::provider::test_support::Frozen) {
        txp_rig_rate(window_ns, block_len, 1e6, 200)
    }

    #[allow(clippy::type_complexity)]
    fn txp_rig_rate(window_ns: Option<i64>, block_len: u32, rate: f64, ticks_per_sample: i64) -> (Arc<Core>, Arc<FakeDevice>, Arc<ezsdr_kernel::time::ManualTimeAuthority>, Arc<EventCollector>, Clock, Tx, crate::provider::test_support::Frozen) {
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..FakeConfig::default() }));
        let mut description = super::super::test_support::default_description(block_len);
        if let Some(window) = window_ns { description.timing.tx_in_flight_ns = window; }
        let (core, time, events) = super::super::test_support::rig_on_description(device.clone(), Vec::new(), BTreeMap::new(), description);
        let frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(rate), ..Settings::default() }, None).unwrap();
        let clock = core.register(Dir::Tx, ticks_per_sample, 0).unwrap();
        let (to_tx, cmds) = std::sync::mpsc::channel();
        std::mem::forget(to_tx);
        let tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        (core, device, time, events, clock, tx, frozen)
    }

    #[allow(clippy::type_complexity)]
    fn txp_high_rate_rig(channels: usize, faults: Vec<FakeFault>, inputs: BTreeMap<ContentHash, Arc<[u8]>>) -> (Arc<Core>, Arc<FakeDevice>, Arc<ezsdr_kernel::time::ManualTimeAuthority>, Arc<EventCollector>, Clock, Tx, crate::provider::test_support::Frozen) {
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, faults, ..FakeConfig::default() }));
        let (core, time, events) = super::super::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        let channel_ids: Vec<_> = (0..channels).collect();
        device.tx_open(0, &channel_ids).unwrap();
        for &channel in &channel_ids {
            device.apply(Dir::Tx, channel, &Settings { rate: Some(200e6), ..Settings::default() }, None).unwrap();
        }
        let clock = core.register(Dir::Tx, 1, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        std::mem::forget(_to_tx);
        let tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(channels as u32));
        (core, device, time, events, clock, tx, frozen)
    }

    /// Sample `i` of burst `k` is `[(i + 1) / 100 000, k / 10 000]`, so a played-out sample names its place.
    fn txp_burst(clock: &Clock, k: i64, len: usize, repeat: bool, policy: LatePolicy) -> Held {
        let mut burst = held(clock, k, len, policy);
        burst.samples = Arc::new(vec![(0..len).map(|i| [(i + 1) as f32 / 100_000.0, k as f32 / 10_000.0]).collect()]);
        burst.open.waveform_len = u32::try_from(len).ok();
        burst.repeat = repeat;
        burst
    }

    fn txp_give(tx: &mut Tx, core: &Core, burst: Held) {
        lock(&core.held).insert((burst.domain, burst.k));
        tx.command(TxCmd::Burst(burst));
    }

    /// Steps until the owner has nothing to do, failing instead of looping if it never stops: a wait that reports itself as work
    /// would otherwise hang the test (design note 37).
    fn txp_drain(tx: &mut Tx) {
        for _ in 0..100_000 { if !tx.step() { return; } }
        panic!("the owner still reports work after 100 000 steps");
    }

    /// Joins `thread` once it has ended, failing if it has not within 10 s of real time.
    fn txp_join(thread: std::thread::JoinHandle<()>) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !thread.is_finished() {
            assert!(std::time::Instant::now() < deadline, "a transmit owner did not end within 10 s of its Shutdown");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        thread.join().expect("a transmit owner");
    }

    /// Root ticks of `samples` samples.
    fn txp_ticks(samples: i64) -> i64 { samples * 200 }

    #[test]
    fn ur_23_txs_tiles_a_short_repeat_across_original_wraps_at_high_rate() {
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_high_rate_rig(1, Vec::new(), BTreeMap::new());
        let wave: Vec<Iq> = (0..997).map(|i| [(i + 1) as f32 / 997.0, -(i as f32) / 1_000.0]).collect();
        let start = 2_000_000 - 996;
        let mut burst = txp_burst(&clock, start, wave.len(), true, LatePolicy::DropAndFlag);
        burst.samples = Arc::new(vec![wave.clone()]);
        txp_give(&mut tx, &core, burst);
        txp_drain(&mut tx);

        let held = tx.open.as_ref().expect("the repeat remains open").held.samples.clone();
        let tiled = &held[0];
        assert_eq!(sends(&device), [(996, true, false)], "the first timed SOB takes the available prefix");
        assert_eq!(tiled.len() % wave.len(), 0, "the tile contains whole original periods");
        assert!(tiled.len() >= 20_000 + wave.len() - 1 && tiled.len() < 60_000, "tile bound: {} samples", tiled.len());
        assert!(tiled.iter().enumerate().all(|(i, sample)| *sample == wave[i % wave.len()]));
        assert_eq!(tx.open.as_ref().unwrap().held.open.waveform_len, Some(997));

        device_at(&core, &time, &device, clock.instant(20_000));
        assert!(tx.step(), "the continuation now has a full 20,000-sample window");
        assert_eq!(*sends(&device).last().unwrap(), (20_000, false, false));
        assert!(Arc::ptr_eq(&held, &tx.open.as_ref().unwrap().held.samples), "the burst is not re-tiled between sends");
        device_at(&core, &time, &device, clock.instant(40_000));
        assert!(tx.step(), "a second continuation crosses another original-waveform wrap");
        assert_eq!(*sends(&device).last().unwrap(), (20_000, false, false));
        assert!(Arc::ptr_eq(&held, &tx.open.as_ref().unwrap().held.samples), "the burst is not re-tiled between sends");
        let expected: Vec<_> = (0..40_996).map(|i| Some(wave[i % wave.len()])).collect();
        assert_eq!(device.transmitted(clock.instant(start), expected.len()), expected, "continuations keep the original 997-sample phase");

        tx.command(TxCmd::Stop);
        let record = lock(&core.rec).bursts.last().cloned().expect("the stopped repeat record");
        assert_eq!(record["samples"], 40_996);
        assert_eq!(record["wraps"], 41, "wrap accounting uses the original 997-sample waveform");
    }

    #[test]
    fn ur_22_repeat_wrap_is_not_a_new_burst_start() {
        let (core, device, _time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_give(&mut tx, &core, txp_burst(&clock, 3_000, 500, true, LatePolicy::DropAndFlag));
        assert!(tx.step());
        assert_eq!(sends(&device), [(2_500, true, false)]);
        assert_eq!(lock(&core.rec).stats.get("tx_bursts"), Some(&1));
        assert!(tx.step(), "the next chunk starts at an original-waveform wrap");
        assert_eq!(*sends(&device).last().unwrap(), (2_500, false, false));
        assert_eq!(tx.open.as_ref().unwrap().next, 8_000);
        assert_eq!(lock(&core.rec).stats.get("tx_bursts"), Some(&1));
    }

    #[test]
    fn ur_23_txs_a_held_cut_splits_a_high_rate_repeat_at_the_target() {
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_high_rate_rig(1, Vec::new(), BTreeMap::new());
        let start = 500_000;
        device_at(&core, &time, &device, clock.instant(start - 400_000));
        let repeat = txp_burst(&clock, start, 50_000, true, LatePolicy::DropAndFlag);
        let a = repeat.samples[0].clone();
        txp_give(&mut tx, &core, repeat);
        assert!(tx.step());
        assert_eq!(sends(&device), [(20_000, true, false)]);

        let next = txp_burst(&clock, start + 30_000, 1_000, false, LatePolicy::DropAndFlag);
        let b = next.samples[0].clone();
        txp_give(&mut tx, &core, next);
        assert!(tx.step(), "the repeat sends only to the held start");
        assert_eq!(*sends(&device).last().unwrap(), (10_000, false, false));
        assert_eq!(tx.continues_at, Some(start + 30_000));
        assert!(tx.step(), "the adjacent burst continues on the device burst");
        assert_eq!(*sends(&device).last().unwrap(), (999, false, false));
        device_at(&core, &time, &device, clock.instant(start + 31_000 - 400_000));
        assert!(tx.step(), "the last held sample closes at the burst end");
        assert_eq!(*sends(&device).last().unwrap(), (1, false, true));

        let expected: Vec<_> = a.into_iter().take(30_000).chain(b).map(Some).collect();
        assert_eq!(device.transmitted(clock.instant(start), expected.len()), expected, "no old sample crosses the cut and the next burst begins at it");
        let records = lock(&core.rec).bursts.clone();
        assert_eq!(records.len(), 2);
        assert_eq!((records[0]["samples"].as_u64(), records[0]["target"]["ticks"].as_i64()), (Some(30_000), Some(start)));
        assert_eq!((records[1]["samples"].as_u64(), records[1]["target"]["ticks"].as_i64()), (Some(1_000), Some(start + 30_000)));
    }

    #[test]
    fn ur_23_txs_late_preemption_uses_the_handed_frontier() {
        for (name, target, policy, outcome) in [
            ("drop inside handed range", 510_000, LatePolicy::DropAndFlag, Some("drop")),
            ("move inside handed range", 510_000, LatePolicy::SendAsapAndFlag, Some("send_asap")),
            ("on-time frontier", 520_000, LatePolicy::DropAndFlag, None),
        ] {
            let (core, device, time, events, clock, mut tx, _frozen) = txp_high_rate_rig(1, Vec::new(), BTreeMap::new());
            let start = 500_000;
            device_at(&core, &time, &device, clock.instant(start - 400_000));
            let repeat = txp_burst(&clock, start, 30_000, true, LatePolicy::DropAndFlag);
            let a = repeat.samples[0].clone();
            txp_give(&mut tx, &core, repeat);
            assert!(tx.step(), "{name}: first send");
            assert_eq!(tx.open.as_ref().unwrap().next, 520_000, "{name}: first unhanded sample");

            let next = txp_burst(&clock, target, 4, false, policy);
            let b = next.samples[0].clone();
            txp_give(&mut tx, &core, next);
            let calls = device.calls().len();
            assert!(tx.step(), "{name}: preemption is decided");
            let errors: Vec<_> = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
                .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
            match outcome {
                Some("drop") => {
                    assert_eq!(errors.len(), 1, "{name}: {errors:?}");
                    assert_eq!(errors[0].payload["outcome"], "drop");
                    assert_eq!(errors[0].payload["target"]["ticks"], target);
                    assert_eq!(device.calls().len(), calls, "{name}: dropped payload reached the device");
                    assert!(tx.held.is_empty());
                    tx.command(TxCmd::Stop);
                    assert_eq!(device.transmitted(clock.instant(start), 20_002), a[..20_000].iter().copied().map(Some).chain([Some([0.0, 0.0]), None]).collect::<Vec<_>>());
                    let record = lock(&core.rec).bursts.last().cloned().unwrap();
                    assert_eq!(record["samples"], 20_000);
                }
                Some("send_asap") => {
                    assert_eq!(errors.len(), 1, "{name}: {errors:?}");
                    assert_eq!(errors[0].payload["outcome"], "send_asap");
                    assert_eq!(errors[0].payload["target"]["ticks"], target);
                    assert_eq!(tx.held.keys().copied().collect::<Vec<_>>(), [520_000], "the moved target is the actual frontier");
                    assert!(tx.step(), "{name}: adjacent continuation");
                    assert_eq!(*sends(&device).last().unwrap(), (3, false, false));
                    tx.command(TxCmd::Stop);
                    let expected: Vec<_> = a[..20_000].iter().copied().chain(b).map(Some).collect();
                    assert_eq!(device.transmitted(clock.instant(start), expected.len()), expected, "{name}: no gap or duplicate at the actual frontier");
                    let records = lock(&core.rec).bursts.clone();
                    assert_eq!(records.len(), 2, "{name}: both logical bursts are recorded");
                    assert_eq!(records[0]["samples"], 20_000);
                    assert_eq!(records[1]["samples"], 4);
                    assert_eq!(records[1]["target"]["ticks"], 520_000);
                    assert_eq!(records[1]["requested_target"]["ticks"], target);
                }
                None => {
                    assert!(errors.is_empty(), "{name}: {errors:?}");
                    assert_eq!(tx.held.keys().copied().collect::<Vec<_>>(), [520_000]);
                    assert!(tx.step(), "{name}: adjacent on-time continuation");
                    assert_eq!(*sends(&device).last().unwrap(), (3, false, false));
                    tx.command(TxCmd::Stop);
                    let expected: Vec<_> = a[..20_000].iter().copied().chain(b).map(Some).collect();
                    assert_eq!(device.transmitted(clock.instant(start), expected.len()), expected, "{name}: the frontier sample is neither late nor skipped");
                    let records = lock(&core.rec).bursts.clone();
                    assert_eq!(records.len(), 2, "{name}: both logical bursts are recorded");
                    assert_eq!(records[1]["target"]["ticks"], 520_000);
                    assert!(records[1]["requested_target"].is_null());
                }
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn ur_23_txs_switch_and_stop_do_not_send_past_a_high_rate_frontier() {
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_high_rate_rig(1, Vec::new(), BTreeMap::new());
        let start = 500_000;
        device_at(&core, &time, &device, clock.instant(start - 400_000));
        let repeat = txp_burst(&clock, start, 30_000, true, LatePolicy::DropAndFlag);
        let expected = repeat.samples[0].clone();
        txp_give(&mut tx, &core, repeat);
        assert!(tx.step());
        tx.switch = Some(switch(&core, clock.instant(start + 30_000), None, 0, Settings::default()));
        assert!(tx.step(), "the next send ends exactly at the cold cut");
        assert_eq!(*sends(&device).last().unwrap(), (10_000, false, false));
        assert_eq!(tx.open.as_ref().unwrap().next, start + 30_000);
        assert!(tx.step(), "the device burst is closed at the cut");
        assert_eq!(*sends(&device).last().unwrap(), (1, false, true));
        let a = device.transmitted(clock.instant(start), 30_001);
        assert_eq!(a[..30_000], expected.iter().take(30_000).copied().map(Some).collect::<Vec<_>>());
        assert_eq!(a[30_000], Some([0.0, 0.0]), "only the fake's empty-EOB closure is at the cut");

        let (core, device, time, _events, clock, mut tx, _frozen) = txp_high_rate_rig(1, Vec::new(), BTreeMap::new());
        device_at(&core, &time, &device, clock.instant(start - 400_000));
        let repeat = txp_burst(&clock, start, 30_000, true, LatePolicy::DropAndFlag);
        let expected: Vec<_> = repeat.samples[0][..20_000].iter().copied().map(Some).collect();
        txp_give(&mut tx, &core, repeat);
        assert!(tx.step());
        tx.command(TxCmd::Stop);
        assert_eq!(*sends(&device).last().unwrap(), (1, false, true), "Stop sends only the EOB closure after the handed chunk");
        assert_eq!(device.transmitted(clock.instant(start), 20_002), expected.into_iter().chain([Some([0.0, 0.0]), None]).collect::<Vec<_>>());
        assert_eq!(lock(&core.rec).bursts.last().unwrap()["samples"], 20_000);
    }

    #[test]
    fn ur_23_txs_one_shots_keep_their_payload_and_end_on_the_last_sample() {
        for len in [1_000, 25_000] {
            let (core, device, time, _events, clock, mut tx, _frozen) = txp_high_rate_rig(1, Vec::new(), BTreeMap::new());
            let start = 500_000;
            device_at(&core, &time, &device, clock.instant(start - 400_000));
            let wave: Vec<Iq> = (0..len).map(|i| [0.1 + i as f32 / 100_000.0, -(i as f32) / 200_000.0]).collect();
            let mut burst = txp_burst(&clock, start, len, false, LatePolicy::DropAndFlag);
            burst.samples = Arc::new(vec![wave.clone()]);
            txp_give(&mut tx, &core, burst);
            txp_drain(&mut tx);
            assert!(tx.tail.is_some());
            let expected_sizes = if len < 20_000 { vec![len - 1] } else { vec![20_000, len - 20_001] };
            assert_eq!(sends(&device).iter().map(|send| send.0).collect::<Vec<_>>(), expected_sizes, "payload before the held tail for {len}");
            device_at(&core, &time, &device, clock.instant(start + len as i64 - 400_000));
            assert!(tx.step(), "{len}: the held tail ends at the final payload sample");
            assert_eq!(*sends(&device).last().unwrap(), (1, false, true));
            assert_eq!(device.transmitted(clock.instant(start), len), wave.iter().copied().map(Some).collect::<Vec<_>>(), "{len}: no tiled, missing, or duplicate data");
            let record = lock(&core.rec).bursts.last().cloned().unwrap();
            assert_eq!(record["samples"], len);
            assert_eq!(record["end"], "eob");
        }
    }

    #[test]
    fn ur_23_txs_grouped_short_repeats_keep_channel_phase_aligned() {
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_high_rate_rig(2, Vec::new(), BTreeMap::new());
        let start = 500_000;
        device_at(&core, &time, &device, clock.instant(start - 400_000));
        for channel in 0..2 {
            device.apply(Dir::Rx, channel, &Settings { rate: Some(200e6), ..Settings::default() }, None).unwrap();
        }
        device.rx_open(1, &[0, 1]).unwrap();
        device.rx_start(1, clock.instant(start)).unwrap();
        let waves: Vec<Vec<Iq>> = (0..2).map(|channel| {
            (0..997).map(|i| [((channel * 997 + i + 1) as f32) / 2_000.0, -((channel * 997 + i) as f32) / 3_000.0]).collect()
        }).collect();
        let mut burst = txp_burst(&clock, start, 997, true, LatePolicy::DropAndFlag);
        burst.samples = Arc::new(waves.clone());
        txp_give(&mut tx, &core, burst);
        assert!(tx.step());
        assert!(tx.step());
        assert_eq!(sends(&device), [(20_000, true, false), (20_000, false, false)]);
        device_at(&core, &time, &device, clock.instant(start + 40_000));
        let crate::device::RxRecv::Samples { samples, first_tick } = device.rx_recv(1, 40_000, Wall::from_millis(10)) else { panic!("both channels loop back") };
        assert_eq!(first_tick, clock.instant(start));
        assert_eq!(samples.len(), 2);
        for channel in 0..2 {
            let expected: Vec<_> = (0..40_000).map(|i| waves[channel][i % 997]).collect();
            assert_eq!(samples[channel], expected, "channel {channel} keeps its own tiled phase and shared cursor");
        }
        tx.command(TxCmd::Stop);
    }

    #[test]
    fn ur_23_txs_does_not_copy_repeats_at_or_above_the_send_cap() {
        for len in [20_000, 25_000] {
            let (core, device, time, _events, clock, mut tx, _frozen) = txp_high_rate_rig(1, Vec::new(), BTreeMap::new());
            let start = 500_000;
            device_at(&core, &time, &device, clock.instant(start - 400_000));
            let mut burst = txp_burst(&clock, start, len, true, LatePolicy::DropAndFlag);
            burst.samples = Arc::new(vec![(0..len).map(|i| [i as f32 / 100_000.0, 0.25]).collect()]);
            let input = burst.samples.clone();
            txp_give(&mut tx, &core, burst);
            assert!(Arc::ptr_eq(&input, &tx.held[&start].samples), "holding length {len} does not copy");
            assert!(tx.step());
            assert!(Arc::ptr_eq(&input, &tx.open.as_ref().unwrap().held.samples), "sending length {len} does not copy");
            assert_eq!(*sends(&device).last().unwrap(), (20_000, true, false));
            let expected: Vec<_> = input[0][..20_000].iter().copied().map(Some).collect();
            assert_eq!(device.transmitted(clock.instant(start), expected.len()), expected);
        }
    }

    #[test]
    fn ur_23_m1_a_repeat_stops_short_of_a_window_edge_it_cannot_fill() {
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_give(&mut tx, &core, txp_burst(&clock, 3_000, 30_000, true, LatePolicy::DropAndFlag));
        for _ in 0..10 {
            if !tx.step() { break; }
            let next = tx.open.as_ref().unwrap().next;
            assert!(next <= 10_000 && device.transmitted(clock.instant(next), 1) == vec![None], "nothing at or past the window edge after a step: {next}");
        }
        assert_eq!(sends(&device), [(2_500, true, false), (2_500, false, false)]);
        assert_eq!(tx.open.as_ref().unwrap().next, 8_000, "2 000 samples are left before the 10 000 edge: no short buffer");
        let calls = device.calls().len();
        assert!(!tx.step(), "waiting is not work");
        assert_eq!(device.calls().len(), calls, "a wait makes no device call");
        // M1b: the buffer goes the moment the whole of it fits, and not before.
        device_at(&core, &time, &device, txp_ticks(500));
        assert!(tx.step());
        assert_eq!(sends(&device).len(), 3);
        assert_eq!(*sends(&device).last().unwrap(), (2_500, false, false));
        assert_eq!(tx.open.as_ref().unwrap().next, 10_500, "the window edge itself takes a buffer: `<`, not `<=`");
        // M1c: nothing past the edge was handed over.
        assert_eq!(device.transmitted(clock.instant(10_500), 1), vec![None]);
        assert!(!tx.step());
    }

    #[test]
    fn ur_23_m2_the_last_part_of_a_burst_is_not_split_by_the_window() {
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_give(&mut tx, &core, txp_burst(&clock, 3_500, 6_800, false, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        assert_eq!(sends(&device), [(2_500, true, false), (2_500, false, false)]);
        assert_eq!(tx.open.as_ref().unwrap().next, 8_500, "1 500 samples fit, the 1 800 left do not: wait");
        device_at(&core, &time, &device, txp_ticks(300));
        assert!(tx.step());
        // All 800 samples go, the last one held back for the end-of-burst.
        assert_eq!(*sends(&device).last().unwrap(), (1_799, false, false));
        assert!(tx.tail.is_some());
        assert_eq!(tx.continues_at, Some(10_300));
    }

    #[test]
    fn ur_23_m3_a_tiled_repeat_waits_for_a_full_chunk_across_wraps() {
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_give(&mut tx, &core, txp_burst(&clock, 9_000, 1_500, true, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        assert_eq!(sends(&device), [(1_000, true, false)], "the first timed buffer takes what fits");
        device_at(&core, &time, &device, txp_ticks(500));
        assert!(!tx.step(), "a tiled continuation waits for the full chunk");
        device_at(&core, &time, &device, txp_ticks(2_500));
        assert!(tx.step());
        assert_eq!(*sends(&device).last().unwrap(), (2_500, false, false), "the full chunk crosses original waveform wraps");
        assert_eq!(tx.open.as_ref().unwrap().next, 12_500);
        let wave: Vec<_> = (0..3_500).map(|i| Some([((i % 1_500) + 1) as f32 / 100_000.0, 0.9])).collect();
        assert_eq!(device.transmitted(clock.instant(9_000), 3_500), wave, "no gap, no phase reset at the wrap");
    }

    #[test]
    fn ur_23_m4_a_continuation_waits_without_a_device_call() {
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_give(&mut tx, &core, txp_burst(&clock, 3_000, 30_000, true, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        assert_eq!(tx.open.as_ref().unwrap().next, 8_000);
        txp_give(&mut tx, &core, txp_burst(&clock, 9_600, 4_000, false, LatePolicy::DropAndFlag));
        assert!(tx.step(), "A's last 1 600 before B, a cut-sized buffer");
        assert_eq!(*sends(&device).last().unwrap(), (1_600, false, false));
        assert_eq!(tx.continues_at, Some(9_600));
        let calls = device.calls().len();
        assert!(!tx.step(), "B's first buffer, a continuation, waits for a whole buffer of room");
        assert_eq!(device.calls().len(), calls, "the wait reached the device (an entry that skips the rule)");
        assert!(tx.open.as_ref().is_some_and(|open| open.next == 9_600 && open.first));
        assert_eq!(tx.continues_at, Some(9_600));
        assert!(tx.tail.is_none());
        device_at(&core, &time, &device, txp_ticks(2_100));
        assert!(tx.step());
        assert_eq!(*sends(&device).last().unwrap(), (2_500, false, false), "B continues A's device burst");
        assert_eq!(sends(&device).iter().filter(|s| s.1).count(), 1, "one device start");
        assert_eq!(lock(&core.rec).stats.get("tx_bursts"), Some(&2));
    }

    #[test]
    fn ur_23_m4c_a_stop_during_a_continuation_s_wait_ends_the_device_burst_once() {
        // The waiting state (an open burst still `first`, its device burst continued) meets Stop, with and without a held tail.
        let (core, device, _time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_give(&mut tx, &core, txp_burst(&clock, 3_000, 30_000, true, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        txp_give(&mut tx, &core, txp_burst(&clock, 9_600, 4_000, false, LatePolicy::DropAndFlag));
        assert!(tx.step());
        assert!(!tx.step());
        let sent = sends(&device).len();
        tx.command(TxCmd::Stop);
        assert_eq!(sends(&device)[sent..], [(1, false, true)], "one empty end-of-burst and no more payload");
        assert!(tx.open.is_none() && tx.continues_at.is_none() && tx.held.is_empty());
        assert!(!lock(&core.held).contains(&(clock.domain, 9_600)), "the waiting burst's reservation is released");
        // With a held tail (M9's state): the tail goes out as the device burst's last sample, with the end-of-burst.
        let (core, device, _time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_give(&mut tx, &core, txp_burst(&clock, 3_000, 6_000, false, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        txp_give(&mut tx, &core, txp_burst(&clock, 9_000, 4_000, false, LatePolicy::DropAndFlag));
        assert!(!tx.step());
        let sent = sends(&device).len();
        tx.command(TxCmd::Stop);
        assert_eq!(sends(&device)[sent..], [(1, false, true)], "the held sample, with the end-of-burst");
        assert_eq!(device.transmitted(clock.instant(8_999), 2), vec![Some([6_000f32 / 100_000.0, 0.3]), None]);
        assert!(tx.open.is_none() && tx.tail.is_none() && tx.held.is_empty());
    }

    #[test]
    fn ur_23_m4b_the_buffer_before_a_held_start_is_not_split() {
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_give(&mut tx, &core, txp_burst(&clock, 3_000, 30_000, true, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        txp_give(&mut tx, &core, txp_burst(&clock, 10_200, 4_000, false, LatePolicy::DropAndFlag));
        let calls = device.calls().len();
        assert!(!tx.step(), "2 200 samples to B's start, 2 000 of room");
        assert_eq!(device.calls().len(), calls);
        device_at(&core, &time, &device, txp_ticks(500));
        assert!(tx.step());
        assert_eq!(*sends(&device).last().unwrap(), (2_200, false, false), "A's last buffer ends at B's start, the device burst staying open");
        assert_eq!(tx.continues_at, Some(10_200));
    }

    #[test]
    fn ur_23_m5_the_first_timed_buffer_takes_what_fits() {
        let (core, device, _time, _events, clock, mut tx, _frozen) = txp_rig(Some(2_200_000), 2_000);
        txp_give(&mut tx, &core, txp_burst(&clock, 2_000, 30_000, true, LatePolicy::DropAndFlag));
        assert!(tx.step(), "not a wait: the burst's start is its own lead away");
        assert_eq!(sends(&device), [(200, true, false)], "chunk 550, 200 samples of window");
        assert!(device.calls().iter().any(|call| call.contains(&format!("at={}", clock.instant(2_000)))));
        assert!(!tx.step(), "from here a whole buffer is waited for");
    }

    #[test]
    fn ur_23_m7_a_preempting_burst_is_not_held_up_by_a_wait() {
        let (core, device, _time, events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_give(&mut tx, &core, txp_burst(&clock, 3_000, 30_000, true, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        txp_give(&mut tx, &core, txp_burst(&clock, 7_800, 2_000, false, LatePolicy::SendAsapAndFlag));
        assert!(tx.step(), "B is late against the cursor: moved, not left behind the wait");
        let late: Vec<_> = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
            .filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert_eq!(late.len(), 1);
        assert_eq!(tx.held.keys().copied().collect::<Vec<_>>(), [8_000]);
        assert_eq!(sends(&device).len(), 2);
    }

    #[test]
    fn ur_23_m8_a_long_repeat_sends_full_chunks_as_the_window_advances() {
        let (core, device, time, events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_give(&mut tx, &core, txp_burst(&clock, 3_000, 30_000, true, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        for ms in 1..=10 {
            device_at(&core, &time, &device, txp_ticks(ms * 1_000));
            txp_drain(&mut tx);
        }
        let sends = sends(&device);
        assert_eq!(sends.len(), 6, "two initial chunks, then one whenever a full 2 500-sample chunk fits");
        assert!(sends.iter().all(|s| s.0 == 2_500), "{sends:?}");
        assert!(lock(&core.rec).rejected.is_empty());
        assert!(events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR));
    }

    #[test]
    fn ur_23_m9_a_waiting_continuation_leaves_the_held_tail_alone() {
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_give(&mut tx, &core, txp_burst(&clock, 3_000, 6_000, false, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        assert_eq!(sends(&device), [(2_500, true, false), (2_500, false, false), (999, false, false)]);
        assert!(tx.tail.is_some() && tx.continues_at == Some(9_000));
        txp_give(&mut tx, &core, txp_burst(&clock, 9_000, 4_000, false, LatePolicy::DropAndFlag));
        let calls = device.calls().len();
        assert!(!tx.step());
        assert_eq!(device.calls().len(), calls, "the tail went out ahead of a buffer that had to wait");
        assert!(tx.tail.is_some());
        device_at(&core, &time, &device, txp_ticks(1_500));
        assert!(tx.step());
        let sends = sends(&device);
        assert_eq!(sends[3..], [(1, false, false), (2_500, false, false)], "the tail, then B, in order");
        assert_eq!(device.transmitted(clock.instant(8_999), 2), vec![Some([6_000f32 / 100_000.0, 0.3]), Some([1f32 / 100_000.0, 0.9])], "A's held last sample, then B's first");
    }

    #[test]
    fn ur_23_a_held_tails_keep_each_channel_sample() {
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_high_rate_rig(2, Vec::new(), BTreeMap::new());
        let start = 500_000;
        for channel in 0..2 {
            device.apply(Dir::Rx, channel, &Settings { rate: Some(200e6), ..Settings::default() }, None).unwrap();
        }
        device.rx_open(0, &[0, 1]).unwrap();
        device.rx_start(0, clock.instant(start)).unwrap();
        device_at(&core, &time, &device, clock.instant(start - 400_000));

        let a = [vec![[0.1, 0.01], [0.2, 0.02], [0.3, 0.03]], vec![[0.4, 0.04], [0.5, 0.05], [0.6, 0.06]]];
        let mut first = one_shot(&clock, start, 2, a[0].clone());
        first.samples = Arc::new(a.to_vec());
        txp_give(&mut tx, &core, first);
        assert!(tx.step());
        assert_eq!(sends(&device), [(2, true, false)]);

        let b = [vec![[0.7, 0.07], [0.8, 0.08]], vec![[0.9, 0.09], [1.0, 0.1]]];
        let mut next = one_shot(&clock, start + 3, 2, b[0].clone());
        next.samples = Arc::new(b.to_vec());
        txp_give(&mut tx, &core, next);
        assert!(tx.step());
        assert_eq!(sends(&device)[1..], [(1, false, false), (1, false, false)], "the first tail precedes the next burst payload");
        assert!(tx.tail.is_some(), "the next burst's distinct per-channel tail is retained");
        tx.command(TxCmd::Stop);
        assert_eq!(*sends(&device).last().unwrap(), (1, false, true), "Stop ends on the next burst's held tail");

        device_at(&core, &time, &device, clock.instant(start + 5));
        let crate::device::RxRecv::Samples { samples, .. } = device.rx_recv(0, 5, Wall::from_millis(10)) else { panic!("the loopback contains both channel tails") };
        assert_eq!(samples, [
            [a[0][0], a[0][1], a[0][2], b[0][0], b[0][1]].to_vec(),
            [a[1][0], a[1][1], a[1][2], b[1][0], b[1][1]].to_vec(),
        ], "continuation and EOB preserve each channel's held sample");
    }

    #[test]
    fn ur_23_m10_a_waiting_burst_whose_timeline_passed_is_abandoned() {
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_give(&mut tx, &core, txp_burst(&clock, 3_000, 30_000, true, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        device_at(&core, &time, &device, txp_ticks(9_001));
        assert!(tx.step());
        assert!(lock(&core.rec).rejected.iter().any(|row| row["reason"].as_str().unwrap_or("").contains("UR-22: the transmit timeline expired")));
        assert_eq!(*sends(&device).last().unwrap(), (1, false, true), "an empty end-of-burst, not a late payload");
        assert_eq!(device.transmitted(clock.instant(9_000), 2), vec![None, Some([0.0, 0.0])], "the end-of-burst's own zero sample, a sample after the cursor, and no payload");
    }

    #[test]
    fn ur_23_m11_tx_sizing_is_independent_of_receive_block_len() {
        let (core, device, _time, _events, clock, mut tx, _frozen) = txp_rig(None, 1);
        txp_give(&mut tx, &core, txp_burst(&clock, 3_000, 5, true, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        assert_eq!(tx.open.as_ref().unwrap().next, 8_000);
        assert_eq!(sends(&device), [(2_500, true, false), (2_500, false, false)], "RX block_len=1 does not cap TX");

    }

    #[test]
    fn ur_23_m12_the_count_is_samples_per_channel() {
        let (core, device, _time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        tx.channels = Channels::first(2);
        device.tx_open(0, &[0, 1]).unwrap();
        let mut burst = txp_burst(&clock, 3_000, 30_000, true, LatePolicy::DropAndFlag);
        burst.samples = Arc::new(vec![burst.samples[0].clone(), burst.samples[0].clone()]);
        txp_give(&mut tx, &core, burst);
        txp_drain(&mut tx);
        assert_eq!(sends(&device), [(2_500, true, false), (2_500, false, false)]);
        assert_eq!(tx.open.as_ref().unwrap().next, 8_000);
    }

    #[test]
    fn ur_23_m14_a_cold_cut_is_not_waited_past_or_crossed() {
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_give(&mut tx, &core, txp_burst(&clock, 3_000, 30_000, true, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        let cut = clock.instant(10_200);
        tx.switch = Some(switch(&core, cut, None, 0, Settings::default()));
        let calls = device.calls().len();
        assert!(!tx.step(), "2 200 samples to the cut, 2 000 of room: wait");
        assert_eq!(device.calls().len(), calls);
        device_at(&core, &time, &device, txp_ticks(500));
        assert!(tx.step());
        assert_eq!(*sends(&device).last().unwrap(), (2_200, false, false));
        assert_eq!(tx.open.as_ref().unwrap().next, 10_200, "the exclusive cut");
        assert!(tx.step(), "at the cut the device burst ends");
        assert_eq!(*sends(&device).last().unwrap(), (1, false, true), "an empty end-of-burst only");
        assert_eq!(device.transmitted(cut, 2), vec![Some([0.0, 0.0]), None], "nothing of the burst at or past the cut");
        device_at(&core, &time, &device, cut);
        assert!(tx.step(), "the switch");
        assert!(tx.clock.is_none() && tx.at == 1);
    }

    #[test]
    fn ur_23_m14e_a_waiting_owner_switches_to_the_new_clock_at_the_cut() {
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_give(&mut tx, &core, txp_burst(&clock, 3_000, 30_000, true, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        let cut = clock.instant(10_500);
        // The same cut, now to a second clock with two channels at 2 MS/s.
        let new = core.register(Dir::Tx, 100, cut).unwrap();
        tx.switch = Some(switch(&core, cut, Some(new), 2, Settings { rate: Some(2e6), ..Settings::default() }));
        let calls = device.calls().len();
        assert!(!tx.step(), "2 500 samples to the cut, 2 000 of room: wait");
        assert_eq!(device.calls().len(), calls);
        device_at(&core, &time, &device, txp_ticks(500));
        assert!(tx.step());
        assert_eq!(*sends(&device).last().unwrap(), (2_500, false, false));
        assert_eq!(tx.open.as_ref().unwrap().next, 10_500, "the exclusive cut");
        assert!(tx.step(), "at the cut the device burst ends");
        assert_eq!(*sends(&device).last().unwrap(), (1, false, true), "an empty end-of-burst only");
        assert_eq!(device.transmitted(clock.instant(10_500), 2), vec![Some([0.0, 0.0]), None], "nothing of the burst at or past the cut");
        device_at(&core, &time, &device, cut);
        assert!(tx.step(), "the switch");
        assert_eq!(tx.clock, Some(new), "the new clock is adopted after the wait and the cut");
        assert_eq!(tx.channels, Channels::first(2));
        assert_eq!(tx.at, 1);
        // And on the new clock a two-channel burst starts with a timed start-of-burst of the expected size.
        let mut burst = txp_burst(&new, 5_000, 3_000, false, LatePolicy::DropAndFlag);
        burst.samples = Arc::new(vec![burst.samples[0].clone(), burst.samples[0].clone()]);
        txp_give(&mut tx, &core, burst);
        let sent = sends(&device).len();
        assert!(tx.step());
        assert_eq!(sends(&device)[sent..], [(2_999, true, false)], "the burst's full remaining payload is within window/4");
        assert!(device.calls().last().unwrap().contains(&format!("at={}", new.instant(5_000))), "{:?}", device.calls().last());
    }


    // The priority of the transmit owner (UR-23) and the idle wait, on `Tx::run` threads over a `TxSpy`.

    /// `owners` transmit owners of the default layout on a `TxSpy`, each with a repeat burst (start 3 000) in its channel;
    /// `setup` sees the spy before they run. The owners are joined after each has sent its first buffer (a wait for an event,
    /// not a time bound), and the Provider's thread names are `uhd-tx-s<k>`.
    fn txp_owners(owners: usize, setup: impl FnOnce(&super::super::test_support::TxSpy)) -> (Arc<Core>, Arc<FakeDevice>, Arc<super::super::test_support::TxSpy>, Arc<EventCollector>) {
        let inner = Arc::new(FakeDevice::new(FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..FakeConfig::default() }));
        let spy = Arc::new(super::super::test_support::TxSpy::new(inner.clone()));
        setup(&spy);
        let (core, time, events) = super::super::test_support::rig_on_description(spy.clone(), Vec::new(), BTreeMap::new(), super::super::test_support::default_description(2_000));
        let frozen = crate::provider::test_support::Frozen::of(&inner);
        device_at(&core, &time, &inner, 0);
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let host = core.device.host();
        let mut senders = Vec::new();
        let mut threads = Vec::new();
        for k in 0..owners {
            inner.tx_open(k, &[k]).unwrap();
            inner.apply(Dir::Tx, k, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
            let (to_tx, cmds) = std::sync::mpsc::channel();
            let burst = txp_burst(&clock, 3_000, 30_000, true, LatePolicy::DropAndFlag);
            lock(&core.held).insert((burst.domain, burst.k));
            to_tx.send(TxCmd::Burst(burst)).unwrap();
            let tx = Tx::new(core.clone(), k, cmds, Some(clock), Channels::first(1));
            senders.push(to_tx);
            threads.push(host.spawn(&super::super::tx_name(k), move || tx.run()).expect("a participant thread"));
        }
        drop(frozen);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        for k in 0..owners {
            let name = super::super::tx_name(k);
            while !spy.recs().iter().any(|rec| rec.what == "tx_send" && rec.thread == name) {
                assert!(std::time::Instant::now() < deadline, "{name} sent nothing within 10 s: {:?}", spy.recs());
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        for to_tx in &senders { to_tx.send(TxCmd::Shutdown).unwrap(); }
        for thread in threads { txp_join(thread); }
        (core, inner, spy, events)
    }

    fn txp_names(spy: &super::super::test_support::TxSpy, what: &str) -> Vec<String> {
        let mut names: Vec<String> = spy.recs().into_iter().filter(|rec| rec.what == what).map(|rec| rec.thread).collect();
        names.sort();
        names
    }

    #[test]
    fn ur_23_p1_p2_each_owner_raises_its_own_thread_once_before_its_first_send() {
        let (_core, _device, spy, _events) = txp_owners(2, |_| {});
        assert_eq!(txp_names(&spy, "raise_thread_priority"), ["uhd-tx-s0", "uhd-tx-s1"], "once per owner thread, from no other thread");
        let recs = spy.recs();
        for name in ["uhd-tx-s0", "uhd-tx-s1"] {
            let raised = recs.iter().position(|rec| rec.what == "raise_thread_priority" && rec.thread == name).expect("raised");
            let sent = recs.iter().position(|rec| rec.what == "tx_send" && rec.thread == name).expect("sent");
            assert!(raised < sent, "{name}: the priority goes before the first send: {recs:?}");
        }
    }

    #[test]
    fn ur_23_p3_every_start_raises_a_new_thread() {
        let (core, _device, spy, _events) = txp_owners(1, |_| {});
        // The same Core and spy, a second start: a new owner, a new thread, a second call.
        let (to_tx, cmds) = std::sync::mpsc::channel();
        to_tx.send(TxCmd::Shutdown).unwrap();
        let tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        txp_join(core.device.host().spawn(&super::super::tx_name(0), move || tx.run()).unwrap());
        assert_eq!(txp_names(&spy, "raise_thread_priority"), ["uhd-tx-s0", "uhd-tx-s0"]);
    }

    #[test]
    fn ur_23_p4_a_refused_priority_is_recorded_and_the_burst_still_goes_out() {
        let (core, device, spy, events) = txp_owners(1, |spy| spy.refuse_priority(crate::DeviceError { lost: true, message: "uhd_set_thread_priority: UHD error 31: EPERM".to_owned() }, None));
        assert!(!core.is_lost(), "a refused priority is not a lost device");
        let rejected = lock(&core.rec).rejected.clone();
        let rows: Vec<_> = rejected.iter().filter(|row| row["call"] == "set_thread_priority").collect();
        assert_eq!(rows.len(), 1, "{rejected:?}");
        assert_eq!(rows[0]["stream"], 0);
        assert!(rows[0]["error"].as_str().unwrap().contains("EPERM"), "{rows:?}");
        assert!(!lock(&core.rec).stats.contains_key("tx_realtime"));
        assert!(device.calls().iter().any(|call| call.starts_with("tx_send ") && call.contains("sob=true")), "the burst's payload reached the device");
        assert!(txp_names(&spy, "tx_send").iter().all(|name| name == "uhd-tx-s0"), "the payload went out on the owner's thread");
        let events = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        assert!(events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::COMMAND_REJECTED && event.kind.as_str() != EventKind::DEVICE_LOST), "a refused priority is a row, not an event: {events:?}");
    }

    #[test]
    fn ur_23_p5_p6_the_success_is_counted_per_owner() {
        let (core, _device, _spy, _events) = txp_owners(2, |_| {});
        assert_eq!(lock(&core.rec).stats.get("tx_realtime"), Some(&2));
        assert!(lock(&core.rec).rejected.iter().all(|row| row["call"] != "set_thread_priority"));
        let (core, _device, _spy, _events) = txp_owners(2, |spy| spy.refuse_priority(crate::DeviceError::failed("only the first"), Some("uhd-tx-s0")));
        assert_eq!(lock(&core.rec).stats.get("tx_realtime"), Some(&1));
        let rejected = lock(&core.rec).rejected.clone();
        let rows: Vec<_> = rejected.iter().filter(|row| row["call"] == "set_thread_priority").collect();
        assert_eq!(rows.len(), 1, "{rejected:?}");
        assert_eq!(rows[0]["stream"], 0);
    }

    /// An owner of the 100 kS/s transmit stream on a `TxSpy`: a 50 ms window (5 000 samples, buffers of 1 250) and a repeat
    /// burst at sample 1 250 in its channel, which fills the window with three buffers: the next one is 12.5 ms away.
    #[allow(clippy::type_complexity)]
    fn txp_idle_owner() -> (Arc<Core>, Arc<FakeDevice>, Arc<super::super::test_support::TxSpy>, std::sync::mpsc::Sender<TxCmd>, Tx, crate::provider::test_support::Frozen) {
        let inner = Arc::new(FakeDevice::new(FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..FakeConfig::default() }));
        let spy = Arc::new(super::super::test_support::TxSpy::new(inner.clone()));
        let mut description = super::super::test_support::default_description(2_000);
        description.timing.tx_in_flight_ns = 50_000_000;
        let (core, time, _events) = super::super::test_support::rig_on_description(spy.clone(), Vec::new(), BTreeMap::new(), description);
        let frozen = crate::provider::test_support::Frozen::of(&inner);
        device_at(&core, &time, &inner, 0);
        inner.tx_open(0, &[0]).unwrap();
        inner.apply(Dir::Tx, 0, &Settings { rate: Some(1e5), ..Settings::default() }, None).unwrap();
        let clock = core.register(Dir::Tx, 2_000, 0).unwrap();
        let (to_tx, cmds) = std::sync::mpsc::channel();
        let burst = txp_burst(&clock, 1_250, 5_000, true, LatePolicy::DropAndFlag);
        lock(&core.held).insert((burst.domain, burst.k));
        to_tx.send(TxCmd::Burst(burst)).unwrap();
        let tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        (core, inner, spy, to_tx, tx, frozen)
    }

    const TXP_IDLE: std::time::Duration = std::time::Duration::from_micros(200);

    #[test]
    fn ur_23_m8b_a_waiting_owner_sleeps_an_idle_period_between_looks() {
        let (core, _device, spy, to_tx, tx, frozen) = txp_idle_owner();
        let (snapshots, release) = spy.trap_idle(3, 5);
        let owner = core.device.host().spawn("tx-idle", move || tx.run()).expect("a participant thread");
        drop(frozen);
        // The owner's own thread takes the snapshot at its fifth idle look and waits on a channel (the clock stands still);
        // this thread only receives it, queues the Shutdown and lets it go.
        let snapshot = snapshots.recv_timeout(std::time::Duration::from_secs(10)).expect("five idle looks");
        to_tx.send(TxCmd::Shutdown).unwrap();
        release.send(()).unwrap();
        txp_join(owner);
        let sends = snapshot.iter().filter(|rec| rec.what == "tx_send").count();
        assert_eq!(sends, 3, "no payload while it waits: {snapshot:?}");
        let idle: Vec<_> = snapshot.iter().enumerate().filter(|(i, rec)| rec.what == "tx_async" && snapshot[..*i].iter().filter(|r| r.what == "tx_send").count() >= 3).map(|(_, rec)| rec.host).collect();
        assert_eq!(idle.len(), 5, "{snapshot:?}");
        // Elapsed since the third payload send, not an absolute time: the test thread may stand still before the owner starts.
        let third = snapshot.iter().filter(|rec| rec.what == "tx_send").nth(2).expect("the third send").host;
        assert!(*idle.last().unwrap() - third < std::time::Duration::from_micros(12_500), "the snapshot is long before a buffer fits (12.5 ms): {idle:?} after {third:?}");
        for pair in idle.windows(2) {
            let gap = pair[1] - pair[0];
            assert!(gap >= TXP_IDLE && gap <= 5 * TXP_IDLE, "a sleeping wait looks again an idle period later, not at once (a spin) and not a window later: {gap:?} {idle:?}");
        }
    }

    #[test]
    fn ur_23_m6_a_waiting_owner_wakes_for_a_stop_within_an_idle_period() {
        let (core, _device, spy, to_tx, tx, frozen) = txp_idle_owner();
        let host = core.device.host();
        let (woken, woken_rx) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let owner = host.spawn("tx-stop", move || {
            crate::host::pause_next_wake(woken, released);
            tx.run();
        }).expect("a participant thread");
        drop(frozen);
        // The owner's first wake, held with its deadline: the clock reads that deadline, however late this thread runs.
        woken_rx.recv_timeout(std::time::Duration::from_secs(10)).expect("the owner's first idle wake");
        let woke = host.now();
        let recs = spy.recs();
        assert_eq!(recs.iter().filter(|rec| rec.what == "tx_send").count(), 3, "{recs:?}");
        let looked = recs.last().expect("a look");
        assert_eq!(looked.what, "tx_async", "the owner went to sleep after a look: {recs:?}");
        let slept = woke - looked.host;
        assert!(slept >= TXP_IDLE && slept < 3 * TXP_IDLE, "the wait sleeps an idle period, not until a buffer fits: {slept:?}");
        to_tx.send(TxCmd::Stop).unwrap();
        release.send(()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let sends = loop {
            let sends: Vec<_> = spy.recs().into_iter().filter(|rec| rec.what == "tx_send").collect();
            if sends.len() > 3 { break sends; }
            assert!(std::time::Instant::now() < deadline, "the Stop was not acted on within 10 s: {:?}", spy.recs());
            std::thread::sleep(std::time::Duration::from_millis(1));
        };
        assert_eq!((sends[3].n, sends[3].eob), (0, true), "the Stop ends (an empty end-of-burst) the device burst before any more payload: {sends:?}");
        to_tx.send(TxCmd::Shutdown).unwrap();
        txp_join(owner);
        assert_eq!(spy.recs().iter().filter(|rec| rec.what == "tx_send").count(), 4, "no payload after the Stop");
    }
}
