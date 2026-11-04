//! uhd-tx: owns the transmit streamer. It transmits held bursts in target order,
//! keeps at most the in-flight window handed to the device ahead of its time, decides
//! a preempting burst against the first sample it has not handed over, ends its clock at
//! its cut, and reads the asynchronous reports (UR-21…UR-23, UR-25, UR-28).

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration as Wall;

use ezsdr_kernel::contract::DataContractId;
use ezsdr_kernel::event::Severity;
use ezsdr_kernel::id::ClockDomainId;
use ezsdr_kernel::stream::{BlockFlags, BlockHeader, BurstOpen, BurstStep, BurstTracker, ChannelMask, Direction, LateOutcome, LatePolicy};
use ezsdr_kernel::time::{Duration, Rescaled, TimePoint};
use ezsdr_radio::kinds;
use ezsdr_radio::payloads::{TimeErrorCause, TimeErrorOutcome, TimeErrorPayload, TxUnderflowCause, TxUnderflowPayload};
use serde_json::json;

use super::control::Planned;
use super::core::{Clock, Core, Streams, lock, stream_key};
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
        if !self.repeat || len == 0 || len >= cap {
            return;
        }
        let tiled_len = ((cap + 2 * len - 2) / len) * len;
        let mut channels = Vec::with_capacity(self.samples.len());
        for channel in self.samples.iter() {
            let mut tiled = Vec::with_capacity(tiled_len);
            while tiled.len() < tiled_len {
                tiled.extend_from_slice(&channel[..len]);
            }
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

#[derive(Clone, Copy, Default)]
// Keep the fixed-capacity tail inline; boxing it would allocate on the TX path.
#[allow(clippy::large_enum_variant)]
enum DeviceBurst {
    #[default]
    Closed,
    Open {
        next: i64,
        tail: Option<Tail>,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Recovery {
    #[default]
    Clean,
    ReopenNeeded,
    Halted,
}

#[derive(Default)]
struct TxChain {
    burst: DeviceBurst,
    front: Option<(ClockDomainId, i64)>,
    recovery: Recovery,
}

impl TxChain {
    fn next(&self) -> Option<i64> {
        match self.burst {
            DeviceBurst::Closed => None,
            DeviceBurst::Open { next, .. } => Some(next),
        }
    }

    fn tail(&self) -> Option<Tail> {
        match self.burst {
            DeviceBurst::Closed => None,
            DeviceBurst::Open { tail, .. } => tail,
        }
    }

    fn continues_at(&self, sample: i64) -> bool {
        self.next() == Some(sample)
    }

    fn burst_is_open(&self) -> bool {
        matches!(self.burst, DeviceBurst::Open { .. })
    }

    fn front(&self) -> Option<(ClockDomainId, i64)> {
        self.front
    }

    #[cfg(test)]
    fn recovery(&self) -> Recovery {
        self.recovery
    }

    fn needs_reopen(&self) -> bool {
        self.recovery == Recovery::ReopenNeeded
    }

    fn open_device_burst(&mut self, next: i64, tail: Option<Tail>) {
        assert!(matches!(self.burst, DeviceBurst::Closed));
        self.burst = DeviceBurst::Open { next, tail };
    }

    fn continue_device_burst(&mut self, next: i64, tail: Option<Tail>) {
        assert!(self.burst_is_open());
        self.burst = DeviceBurst::Open { next, tail };
    }

    fn close_device_burst(&mut self) {
        self.burst = DeviceBurst::Closed;
    }

    fn advance_front(&mut self, domain: ClockDomainId, sample: i64) {
        let sample = self.front.filter(|(old_domain, _)| *old_domain == domain).map_or(sample, |(_, old)| old.max(sample));
        self.front = Some((domain, sample));
    }

    fn mark_reopen_needed(&mut self) {
        self.recovery = Recovery::ReopenNeeded;
    }

    fn reopen_succeeded(&mut self) {
        self.recovery = Recovery::Clean;
    }

    fn reopen_failed(&mut self) {
        self.burst = DeviceBurst::Closed;
        self.recovery = Recovery::Halted;
    }

    fn provider_stop(&mut self) {
        self.recovery = Recovery::Clean;
    }

    fn lose(&mut self) {
        self.burst = DeviceBurst::Closed;
        self.recovery = Recovery::Clean;
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
    /// The channel set of the stream's clock, for the bursts' channel count (RM-29).
    /// The streamer's set is the stream's record (UR-13), not this.
    channels: Channels,
    /// The generation of the streamer `unacked`'s correlations were taken against (UR-36,
    /// UR-28): the record's generation as this owner last read it, so that a streamer opened
    /// since is told from this one's.
    streamer: u64,
    held: BTreeMap<i64, Held>,
    open: Option<Open>,
    tracker: Option<BurstTracker>,
    /// The root tick of the current clock's cut, from the plan (UR-25).
    cut: Option<i64>,
    /// The transmit plan as last read, the version it was read at, and the index in it of the
    /// current segment.
    plan: Vec<Planned>,
    version: Option<u64>,
    at: usize,
    unacked: VecDeque<PendingReport>,
    /// One radio TIME_ERROR per late packet can name one Kernel burst more than once.
    late_reported: HashSet<TimePoint>,
    /// Per-streamer device-burst, safe-start, and recovery facts (design note 44 §4).
    chain: TxChain,
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
            open: None,
            cut: None,
            plan: Vec::new(),
            version: None,
            at: 0,
            unacked: VecDeque::new(),
            late_reported: HashSet::new(),
            chain: TxChain::default(),
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
    /// state; then the report read, then one step. A turn another holder owns the streamer for
    /// makes no device call at all — `reports` and `step` included — and nothing of the owner's
    /// state moves. False when the turn had nothing to do, which the run loop's idle wait follows.
    pub(super) fn pass(&mut self) -> bool {
        // An uncertain EOB invalidates this report queue. Replace its streamer before
        // consuming anything that could have been queued by the failed generation.
        if self.chain.needs_reopen() {
            return self.reopen_streamer();
        }
        let core = self.core.clone();
        let key = stream_key(Dir::Tx, self.k);
        let mut streams = lock(&core.streams);
        if !streams.begin_read(key) {
            return false;
        }
        if !self.plan_section(&mut streams, key) {
            streams.end_read(key);
            return false;
        }
        drop(streams);
        self.reports(Wall::ZERO);
        lock(&core.streams).end_read(key);
        self.step()
    }

    fn command(&mut self, cmd: TxCmd) {
        match cmd {
            TxCmd::Burst(mut held) => {
                held.tile_repeat();
                if self.clock.is_some_and(|c| c.domain == held.domain) {
                    self.held.insert(held.k, held);
                } else {
                    // Booked on a clock that ended at its origin before this thread took the burst (UR-25).
                    self.cancel_unstarted(&held);
                }
            }
            TxCmd::Stop => self.stop_now("UR-26: cancelled by Stop", ezsdr_radio::CancelReason::Stop),
            TxCmd::Shutdown => unreachable!("handled by run"),
        }
    }

    /// The plan, read again when uhd-control has changed it (UR-25), inside the caller's section of
    /// the instance's line state.
    fn plan_section(&mut self, streams: &mut Streams, key: StreamKey) -> bool {
        if self.version == Some(streams.version) {
            return true;
        }
        self.version = Some(streams.version);
        self.plan = streams.planned(key);
        self.retarget();
        true
    }

    /// UR-25: the plan's cut of the current clock.
    fn retarget(&mut self) {
        self.cut = self.clock.and_then(|clock| self.plan.get(self.at).and_then(|current| current.segment.cut).map(|cut| clock.instant(cut)));
    }

    /// UR-25: a burst booked on a clock that ended at its origin is cancelled.
    fn cancel_unstarted(&self, held: &Held) {
        self.forget(held);
        self.report_cancelled_burst("UR-25: its transmit clock was never started", ezsdr_radio::CancelReason::ColdChange);
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
    fn reported_targets(&mut self, channel: usize, tick: Option<i64>) -> Vec<TimePoint> {
        let matching: Vec<usize> = if let Some(tick) = tick {
            self.unacked
                .iter()
                .enumerate()
                .filter_map(|(index, pending)| {
                    let root_tick = self.core.clocks.convert(pending.target, self.core.root).and_then(|converted| converted.floor().ticks_in(self.core.root)).ok()?;
                    (pending.channels.contains(&channel) && root_tick <= tick).then_some(index)
                })
                .collect()
        } else {
            self.unacked.iter().position(|pending| pending.channels.contains(&channel)).into_iter().collect()
        };
        let mut retired = Vec::with_capacity(matching.len());
        for index in matching.into_iter().rev() {
            let pending = &mut self.unacked[index];
            retired.push(pending.target);
            pending.channels.remove(&channel);
            if pending.channels.is_empty() {
                self.unacked.remove(index);
            }
        }
        for target in &retired {
            if !self.target_pending(*target) {
                self.late_reported.remove(target);
            }
        }
        retired
    }

    fn time_error_target(&self, channel: usize, tick: Option<i64>) -> Option<TimePoint> {
        let mut targets = self.unacked.iter().filter(|pending| pending.channels.contains(&channel)).filter_map(|pending| {
            let target = pending.target;
            let root_tick = self.core.clocks.convert(target, self.core.root).and_then(|converted| converted.floor().ticks_in(self.core.root)).ok()?;
            tick.is_none_or(|tick| root_tick <= tick).then_some(target)
        });
        let target = targets.next()?;
        if tick.is_some() && targets.next().is_some() { None } else { Some(target) }
    }

    fn target_pending(&self, target: TimePoint) -> bool {
        self.unacked.iter().any(|pending| pending.target == target)
    }

    /// One unit of work; false when there was nothing to do.
    pub(super) fn step(&mut self) -> bool {
        if self.core.is_lost() {
            self.chain.lose();
            return false;
        }
        let Some(clock) = self.clock else { return false };
        let now = self.core.now();
        let window = self.core.ticks(self.core.description.timing.tx_in_flight_ns);
        let window_cut = clock.at_or_after(now.saturating_add(window));
        let cut_k = self.cut.map(|cut| clock.at_or_after(cut));
        if self.open.is_none() && self.cut.is_some_and(|cut| now >= cut) {
            self.end_clock();
            return true;
        }
        if let Some(next) = self.open.as_ref().map(|o| o.next) {
            if clock.instant(next) < now {
                self.core.reject_note(json!({ "kind": "tx_burst", "reason": "UR-22: the transmit timeline expired; burst not resumed" }));
                self.abandon(true, 0, true);
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
        if let Some(c) = self.chain.next() {
            if clock.instant(c) < now {
                self.core.reject_note(json!({ "kind": "tx_burst", "reason": "UR-22: the transmit timeline expired; burst not resumed" }));
                self.abandon(true, 0, true);
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
                let held = if self.chain.continues_at(k) {
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
        if self.chain.needs_reopen() {
            return false;
        }
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
        let device_sob = open.first && !self.chain.continues_at(start);
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
        // The held tail is at `start - 1`; the continuation cursor already follows it.
        if continuing {
            if let Some(tail) = self.chain.tail() {
                // The burst before's held sample, ahead of this one, no end-of-burst between;
                // without it this one would play a sample early, so its failure abandons this
                // burst, the device burst ended (Review Q, NB-Q2).
                let buffers = tail.buffers();
                let at = Some(clock.instant(open.next.saturating_sub(1)));
                match self.core.device.tx_send(self.k, &buffers[..tail.channels], at, false, false, SEND_TIMEOUT) {
                    Ok(1) => {}
                    Ok(_) => {
                        self.core.reject_note(json!({ "kind": "tx_burst", "reason": "UR-22: a send did not complete within 1 s" }));
                        return self.abandon(true, 0, true);
                    }
                    Err(error) => {
                        self.core.device_failed(None, "tx_burst", &error);
                        return self.abandon(true, 0, true);
                    }
                }
            }
        }
        let padded_eob = device_eob && sending == 1;
        let device_count = sending + usize::from(padded_eob);
        let mut padded = [[Iq::default(); 2]; TX_CHANNELS_MAX];
        let mut sent = [&[][..]; TX_CHANNELS_MAX];
        if padded_eob {
            for channel in 0..channels {
                padded[channel][0] = slices[channel][0];
            }
            for channel in 0..channels {
                sent[channel] = &padded[channel];
            }
        } else {
            for (slice, channel) in sent[..channels].iter_mut().zip(slices[..channels].iter()) {
                *slice = &channel[..sending];
            }
        }
        // T1: every payload-bearing packet names its first sample, even inside a device burst.
        let at = Some(clock.instant(open.next));
        let result = if device_count > 0 { self.core.device.tx_send(self.k, &sent[..channels], at, device_sob, device_eob, SEND_TIMEOUT) } else { Ok(0) };
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
            Ok(accepted) if accepted == device_count => {}
            Ok(accepted) => {
                self.core.reject_note(json!({ "kind": "tx_burst", "reason": "UR-22: a send did not complete within 1 s" }));
                if device_sob && accepted > 0 {
                    self.remember_burst(TimePoint::new(clock.domain, start));
                }
                let result = self.abandon(was_open || accepted > 0, accepted, !device_eob);
                if device_eob {
                    self.core.reject_note(json!({ "call": "tx_close", "stream": self.k,
                        "error": format!("EOB send accepted {accepted} of {device_count} samples") }));
                    if self.core.is_lost() {
                        self.chain.lose();
                    } else {
                        self.chain.mark_reopen_needed();
                    }
                }
                return result;
            }
            Err(error) => {
                // Whether UHD sent packets before the error is unknown: a start that may have
                // gone is counted for its report, then closed by the two-sample EOB below.
                self.core.device_failed(None, "tx_burst", &error);
                if device_sob {
                    self.remember_burst(TimePoint::new(clock.domain, start));
                }
                let result = self.abandon(true, device_count, !device_eob);
                if device_eob {
                    self.core.reject_note(json!({ "call": "tx_close", "stream": self.k, "error": error.message }));
                    if self.core.is_lost() {
                        self.chain.lose();
                    } else {
                        self.chain.mark_reopen_needed();
                    }
                }
                return result;
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
            self.core.stat("tx_bursts", 1);
        }
        self.core.stat("tx_samples", count);
        let open = self.open.as_mut().expect("still open");
        open.first = false;
        open.next += count;
        if device_eob {
            self.chain.close_device_burst();
        } else if device_sob {
            self.chain.open_device_burst(open.next, tail);
        } else {
            self.chain.continue_device_burst(open.next, tail);
        }
        if let Some(tracker) = self.tracker.as_mut() {
            let step = tracker.on_block(&header, open_record);
            self.record(step);
        }
        self.contract = Some(header.contract);
        if eob {
            let open = self.open.take().expect("open");
            if !device_eob {
                // The device burst remains open at the next sample.
            } else {
                self.chain.advance_front(clock.domain, open.next.saturating_add(if padded_eob { 2 } else { 1 }));
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
        let continuing = self.chain.next();
        let tail = self.chain.tail();
        let device_open = self.open.as_ref().map_or(continuing.is_some(), |open| !open.first || continuing == Some(open.held.k));
        if device_eob {
            self.chain.close_device_burst();
        }
        if device_eob && device_open && !self.core.is_lost() {
            let cursor = self.open.as_ref().map_or(continuing, |open| Some(open.next));
            if let (Some(clock), Some(cursor)) = (self.clock, cursor) {
                let close_start = cursor.saturating_sub(if tail.is_some() { 1 } else { 0 });
                if self.close_device_burst(tail, cursor, clock) {
                    self.chain.advance_front(clock.domain, close_start.saturating_add(3));
                }
            }
        } else if device_eob && self.core.is_lost() {
            self.chain.lose();
        }
        let Some(open) = self.open.take() else { return };
        if let Some(record) = self.tracker.as_mut().and_then(BurstTracker::stop) {
            lock(&self.core.rec).bursts.push(serde_json::to_value(record).expect("a record"));
        }
        self.forget(&open.held);
    }

    /// Closes with exactly two samples and their first SampleClock instant: held tail + zero,
    /// or two zeros when there is no tail.
    fn close_device_burst(&mut self, tail: Option<Tail>, cursor: i64, clock: Clock) -> bool {
        if self.core.is_lost() {
            self.chain.lose();
            return false;
        }
        let has_tail = tail.is_some();
        let count = tail.as_ref().map_or(self.channels.count().max(1) as usize, |tail| tail.channels);
        let mut close = [[Iq::default(); 2]; TX_CHANNELS_MAX];
        if let Some(tail) = tail {
            for (samples, held) in close[..count].iter_mut().zip(tail.samples[..count].iter()) {
                samples[0] = *held;
            }
        }
        let buffers: Vec<&[Iq]> = close[..count].iter().map(|samples| &samples[..]).collect();
        let first = cursor.saturating_sub(if has_tail { 1 } else { 0 });
        let at = Some(clock.instant(first));
        match self.core.device.tx_send(self.k, &buffers, at, false, true, SEND_TIMEOUT) {
            Ok(2) => true,
            Ok(n) => {
                self.core.reject_note(json!({ "call": "tx_close", "stream": self.k, "error": format!("EOB close accepted {n} of 2 samples") }));
                if self.core.is_lost() {
                    self.chain.lose();
                } else {
                    self.chain.mark_reopen_needed();
                }
                false
            }
            Err(error) => {
                self.core.device_failed(None, "tx_close", &error);
                self.core.reject_note(json!({ "call": "tx_close", "stream": self.k, "error": error.message }));
                if error.lost {
                    self.chain.lose();
                } else {
                    self.chain.mark_reopen_needed();
                }
                false
            }
        }
    }

    fn reopen_streamer(&mut self) -> bool {
        let core = self.core.clone();
        let key = stream_key(Dir::Tx, self.k);
        let (claim, configured) = {
            let mut streams = lock(&core.streams);
            if streams.stop.is_some() || core.is_lost() {
                if core.is_lost() {
                    self.chain.lose();
                } else {
                    self.chain.provider_stop();
                }
                return false;
            }
            let Some(claim) = streams.claim_owner(key) else { return false };
            (claim, streams.stream(key).configured)
        };
        core.probe("tx_reopen_claimed", None);
        let result = core.reopen_tx_streamer(self.k, self.channels);
        if let (Ok(flush), Some(clock), Some(tick)) = (&result, self.clock, result.as_ref().ok().and_then(|flush| flush.ack_tick)) {
            if flush.acknowledged {
                self.chain.advance_front(clock.domain, clock.at_or_after(tick).saturating_add(2));
            }
        }
        let mut halted = false;
        {
            let mut streams = lock(&core.streams);
            if result.is_ok() {
                streams.record_configured(key, configured);
            } else if !core.is_lost() && streams.stop.is_none() {
                self.halt_failed_reopen(&mut streams, key);
                self.chain.reopen_failed();
                halted = true;
            } else if core.is_lost() {
                self.chain.lose();
            } else {
                self.chain.provider_stop();
            }
            streams.give_back(claim);
        }
        self.sync_streamer();
        if result.is_ok() {
            self.chain.reopen_succeeded();
        }
        if halted {
            for held in std::mem::take(&mut self.held).into_values() {
                self.forget(&held);
                self.report_cancelled_burst("UR-22: cancelled because the TX streamer could not be reopened", ezsdr_radio::CancelReason::ColdChange);
            }
        }
        if let Err(error) = &result {
            if !core.is_lost() && lock(&core.streams).stop.is_none() {
                self.core.device_failed(None, "tx_reopen", error);
            }
        }
        true
    }

    /// RM-25: a non-terminal reopen refusal ends the stream's clock: the open burst is cancelled and
    /// the stream has no clock after it.
    fn halt_failed_reopen(&mut self, streams: &mut Streams, key: StreamKey) {
        if let Some(open) = self.open.take() {
            self.forget(&open.held);
            self.report_cancelled_burst("UR-22: cancelled because the TX streamer could not be reopened", ezsdr_radio::CancelReason::ColdChange);
        }
        if let Some(record) = self.tracker.as_mut().and_then(BurstTracker::stop) {
            lock(&self.core.rec).bursts.push(serde_json::to_value(record).expect("a record"));
        }
        streams.finished(key);
        self.plan = streams.planned(key);
        self.at = self.plan.len();
        self.version = Some(streams.version);
        self.clock = None;
        self.tracker = None;
        self.cut = None;
    }

    /// Always true: abandoning a burst is work, which `send`'s callers return.
    fn abandon(&mut self, device_open: bool, handed: usize, close_open_burst: bool) -> bool {
        // A send that failed or came up short: end the device burst if one is open, so
        // that none is left without end-of-burst (Review N, N7), with an empty buffer —
        // the burst's samples after what the device took are not sent (Review P, NB-1).
        let handed = i64::try_from(handed).unwrap_or(i64::MAX);
        let cursor = self.open.as_ref().map(|open| open.next.saturating_add(handed)).or_else(|| self.chain.next().map(|cursor| cursor.saturating_add(handed)));
        self.chain.close_device_burst();
        if device_open && close_open_burst && !self.core.is_lost() {
            if let (Some(clock), Some(cursor)) = (self.clock, cursor) {
                if self.close_device_burst(None, cursor, clock) {
                    self.chain.advance_front(clock.domain, cursor.saturating_add(3));
                }
            }
        } else if self.core.is_lost() {
            self.chain.lose();
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
        let payload = serde_json::to_value(TimeErrorPayload { cause: TimeErrorCause::Late, outcome, late_by_ns, target }).expect("a payload");
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
        let front = self.chain.front().filter(|(domain, _)| *domain == clock.domain).map(|(_, sample)| sample);
        let bound = ezsdr_radio::timeline::later_bound(current, front);
        let outcome = held.policy.decide(&self.core.clocks, TimePoint::new(clock.domain, held.k), TimePoint::new(clock.domain, now), Duration::new(ClockDomainId::HOST_MONOTONIC, DEVICE_LEAD_NS));
        let outcome = outcome
            .and_then(|outcome| ezsdr_radio::timeline::late_policy_front_outcome(held.policy, held.k, current, front, |ticks| self.front_late_by(clock, ticks)).map(|front| front.unwrap_or(outcome)));
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
            let outcome =
                held.policy.decide(&self.core.clocks, TimePoint::new(clock.domain, h), TimePoint::new(clock.domain, bound - lead_k), Duration::new(ClockDomainId::HOST_MONOTONIC, DEVICE_LEAD_NS));
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
        if held.k == next && (self.chain.burst_is_open() || self.open.as_ref().is_some_and(|open| !open.first)) {
            self.end_open(false);
            self.chain.continue_device_burst(held.k, self.chain.tail());
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
        for held in std::mem::take(&mut self.held).into_values() {
            self.forget(&held);
            self.report_cancelled_burst(reason, cancel_reason);
        }
    }

    /// The clock ends at its cut (UR-25): the open burst ends, the clock's held bursts are
    /// cancelled, and the stream has no clock after it.
    fn end_clock(&mut self) {
        self.cut = None;
        self.end_open(true);
        self.read_reports_before_close();
        for (_, held) in std::mem::take(&mut self.held) {
            self.cancel_unstarted(&held);
        }
        if self.clock.take().is_some() {
            self.at += 1;
        }
        lock(&self.core.streams).finished(stream_key(Dir::Tx, self.k));
        self.tracker = None;
    }

    /// UR-28, UR-36: a report queue belongs to the streamer of its generation. The record's
    /// generation moved whenever the owner reopened it, and with it the correlations of the bursts
    /// in flight against the replaced queue, which are dropped here before any report is
    /// correlated with them.
    fn sync_streamer(&mut self) {
        let streamer = lock(&self.core.streams).stream(stream_key(Dir::Tx, self.k)).generation;
        if streamer != self.streamer {
            self.unacked.clear();
            self.late_reported.clear();
        }
        self.streamer = streamer;
    }

    /// Reads any report made available by the old burst's end before its owner releases the
    /// streamer (UR-28, A10).
    fn read_reports_before_close(&mut self) {
        if self.core.is_lost() {
            return;
        }
        let stream = stream_key(Dir::Tx, self.k);
        // UR-25: a holder other than Free keeps the stream; the close then makes its reports
        // unread, rather than reading them behind the holder.
        if !lock(&self.core.streams).begin_read(stream) {
            return;
        }
        self.reports(Wall::ZERO);
        lock(&self.core.streams).end_read(stream);
    }

    /// UR-28: every report recorded; underflows and late bursts become events.
    fn reports(&mut self, timeout: Wall) {
        while !self.core.is_lost() {
            let report = {
                let _reports = self.core.lock_tx_reports(self.k);
                if let Some(report) = self.core.take_stashed_tx_report(self.k) { Ok(Some(report)) } else { self.core.device.tx_async(self.k, timeout) }
            };
            match report {
                Ok(Some(report)) => {
                    self.sync_streamer();
                    self.report(report)
                }
                Ok(None) => break,
                // UR-28: a failed read is the device's loss or a recorded error, never "no report".
                Err(error) if error.lost => {
                    self.core.device_lost(&error.message);
                    self.chain.lose();
                    return;
                }
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
        let raw = json!({
            "code": format!("{:?}", report.code), "tick": report.tick, "channel": report.channel, "stream": self.k,
        });
        lock(&self.core.rec).device_async.push(raw);
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
                self.reported_targets(report.channel, report.tick);
            }
            TxCode::Underflow | TxCode::UnderflowInPacket | TxCode::SeqError | TxCode::SeqErrorInBurst => {
                let cause = if matches!(report.code, TxCode::Underflow | TxCode::UnderflowInPacket) { TxUnderflowCause::Starved } else { TxUnderflowCause::Lost };
                underflow(cause);
                // Its nominal next sample no longer bounds actual playback. Do not
                // resume untimed payload on the shifted device timeline (UR-22).
                if self.open.is_some() || self.chain.burst_is_open() {
                    self.core.reject_note(json!({ "kind": "tx_burst", "reason": "UR-22: the device burst underflowed; burst not resumed" }));
                    self.abandon(true, 0, true);
                }
            }
            TxCode::TimeError => {
                // The report tick is radio-current-time, not the rejected packet's timestamp.
                // Only a unique pending burst whose first sample is no later than that tick fits.
                let Some(target) = self.time_error_target(report.channel, report.tick) else { return };
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
                if self.late_reported.insert(target) {
                    self.time_error(TimeErrorOutcome::LateAtDevice, Duration::new(ClockDomainId::HOST_MONOTONIC, late_by_ns), target);
                }
            }
            TxCode::Other(_) => {}
        }
    }

    fn shutdown(&mut self) {
        self.stop_now("UR-26: cancelled by Provider::stop", ezsdr_radio::CancelReason::ProviderStop);
        self.chain.provider_stop();
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
    use ezsdr_kernel::module_api::{ActionReceiver, Dispatched, StopMode};

    use ezsdr_kernel::stream::{BurstOpen, LatePolicy};
    use ezsdr_kernel::time::{AbsoluteDeadline, TimePoint};

    use super::super::control::Control;
    use super::super::mode::DescriptionExt;
    use super::super::test_support::{ReportFeed, TxSpy};
    use super::{Held, Recovery, TX_CHANNELS_MAX, Tail, Tx, TxChain, TxCmd};
    use crate::device::{Device, Dir, FakeConfig, FakeDevice, FakeFault, Iq, Settings, TxCode, TxReport};
    use crate::provider::core::{Clock, Core, Holder, lock, stream_key};
    use crate::provider::test_support::device_at;
    use ezsdr_radio::timeline::Channels;

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

    fn recovery_rig(device: Arc<dyn Device>) -> (Arc<Core>, Arc<ezsdr_kernel::time::ManualTimeAuthority>, Tx, Clock) {
        let mut description = super::super::test_support::default_description(2_000);
        description.timing.tx_in_flight_ns = 200_000_000;
        let (core, time, _) = super::super::test_support::rig_on_description(device.clone(), Vec::new(), BTreeMap::new(), description);
        device.tx_open(0, &[0]).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        (core, time, tx, clock)
    }

    #[test]
    fn tx_chain_device_burst_open_matrix() {
        let (core, _, _, _) = super::super::test_support::rig();
        let a = core.register(Dir::Tx, 200, 0).unwrap();
        let b = core.register(Dir::Tx, 100, 1_000_000).unwrap();
        let tail = Tail { samples: [[0.25, 0.0]; TX_CHANNELS_MAX], channels: 1 };
        let mut chain = TxChain::default();

        assert_eq!(chain.next(), None);
        assert_eq!(chain.front(), None);
        assert_eq!(chain.recovery(), Recovery::Clean);
        chain.open_device_burst(20, None);
        assert_eq!(chain.next(), Some(20));
        assert!(chain.tail().is_none());

        chain.continue_device_burst(40, Some(tail));
        assert_eq!(chain.next(), Some(40));
        assert!(chain.continues_at(40));
        assert_eq!(chain.tail().map(|tail| tail.channels), Some(1));
        chain.advance_front(a.domain, 50);
        assert_eq!(chain.front(), Some((a.domain, 50)), "front remains independent of the open burst");

        chain.advance_front(a.domain, 45);
        assert_eq!(chain.front(), Some((a.domain, 50)), "same-domain front never regresses");
        chain.advance_front(b.domain, 3);
        assert_eq!(chain.front(), Some((b.domain, 3)), "a new clock replaces the stored front");
        chain.close_device_burst();
        assert_eq!(chain.next(), None);
        assert!(chain.tail().is_none());
    }

    #[test]
    fn tx_chain_recovery_transitions() {
        let mut chain = TxChain::default();
        chain.mark_reopen_needed();
        assert_eq!(chain.recovery(), Recovery::ReopenNeeded);

        chain.reopen_succeeded();
        assert_eq!(chain.recovery(), Recovery::Clean);
        chain.open_device_burst(90, Some(Tail { samples: [[0.25, 0.0]; TX_CHANNELS_MAX], channels: 1 }));
        chain.mark_reopen_needed();
        chain.reopen_failed();
        assert_eq!(chain.recovery(), Recovery::Halted);
        assert!(chain.next().is_none() && chain.tail().is_none());
        chain.provider_stop();
        assert_eq!(chain.recovery(), Recovery::Clean);
        chain.open_device_burst(100, Some(Tail { samples: [[0.25, 0.0]; TX_CHANNELS_MAX], channels: 1 }));
        chain.mark_reopen_needed();
        chain.lose();
        assert_eq!(chain.recovery(), Recovery::Clean);
        assert!(chain.next().is_none() && chain.tail().is_none());
    }

    struct NoActions;
    impl ActionReceiver for NoActions {
        fn recv(&self) -> Option<Dispatched> {
            None
        }
    }

    /// Transmit stream 0's clock, registered at `arm` (RM-25).
    fn tx_clock(core: &Core) -> Clock {
        lock(&core.streams).stream(stream_key(Dir::Tx, 0)).clocks[0]
    }

    #[test]
    fn ur_22_tx_owner_reopen_waits_for_t3_flush_ack_before_timed_tx_dsp() {
        let (core, spy) = spy_rig();
        spy.watch(&core);
        spy.tx_open(0, &[0]).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        tx.chain.mark_reopen_needed();
        super::super::core::assert_tx_open_flush_barrier(core, spy, move || {
            assert!(tx.pass());
        });
    }

    #[test]
    fn ur_22_uncertain_reopen_waits_for_owner_claim_and_stops_on_stop_or_loss() {
        let (core, device, _, _) = super::super::test_support::rig();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(1));
        tx.chain.mark_reopen_needed();
        let key = stream_key(Dir::Tx, 0);
        let claim = lock(&core.streams).claim_owner(key).expect("test holds the stream");
        let before = device.calls();
        assert!(!tx.pass(), "a held stream claim defers the reopen for an idle turn");
        assert!(tx.chain.needs_reopen(), "a deferred reopen remains pending");
        assert_eq!(device.calls(), before, "the owner makes no call while another claim is held");
        lock(&core.streams).give_back(claim);
        assert!(tx.pass(), "the owner retries the reopen after the claim is returned");
        assert!(!tx.chain.needs_reopen());
        assert!(device.calls().iter().any(|call| call == "tx_send 0 n=2 at=- sob=true eob=true"), "the retry runs T3: {:?}", device.calls());

        let (stopped_core, stopped_device, _, _) = super::super::test_support::rig();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut stopped = Tx::new(stopped_core.clone(), 0, cmds, None, Channels::first(1));
        stopped.chain.mark_reopen_needed();
        lock(&stopped_core.streams).stop = Some((stopped_core.now(), StopMode::Orderly));
        let before = stopped_device.calls();
        assert!(!stopped.pass(), "Stop ends an unavailable reopen");
        assert!(!stopped.chain.needs_reopen());
        assert_eq!(stopped_device.calls(), before, "no device call follows Stop");

        let (lost_core, lost_device, _, _) = super::super::test_support::rig();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut lost = Tx::new(lost_core.clone(), 0, cmds, None, Channels::first(1));
        lost.chain.mark_reopen_needed();
        lost_core.device_lost("test loss during reopen");
        let before = lost_device.calls();
        assert!(!lost.pass(), "device loss ends an unavailable reopen");
        assert!(!lost.chain.needs_reopen());
        assert_eq!(lost_device.calls(), before, "no device call follows loss");
    }

    #[test]
    fn ur_22_stop_and_loss_during_owner_recovery_release_the_claim() {
        for (at_ack_return, stop) in [(false, true), (false, false), (true, true), (true, false)] {
            let spy = Arc::new(LockSpy::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() }));
            let (core, _, _) = super::super::test_support::rig_on(spy.clone(), Vec::new(), 2_000);
            spy.watch(&core);
            spy.tx_open(0, &[0]).unwrap();
            let clock = core.register(Dir::Tx, 200, 0).unwrap();
            let (_to_tx, cmds) = std::sync::mpsc::channel();
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            let target = clock.at_or_after(core.now() + core.ticks(60_000_000));
            tx.held.insert(target, held(&clock, target, 1, LatePolicy::DropAndFlag));
            lock(&core.held).insert((clock.domain, target));
            tx.chain.mark_reopen_needed();

            let (claim_reached, claim_release) = if at_ack_return {
                (None, None)
            } else {
                let (reached, wait) = std::sync::mpsc::channel();
                let release = Arc::new(std::sync::Barrier::new(2));
                let resume = release.clone();
                super::super::test_support::probe(&core, "tx_reopen_claimed", move |_| {
                    let _ = reached.send(());
                    resume.wait();
                });
                (Some(wait), Some(release))
            };
            let report_gate = at_ack_return.then(|| spy.stop_after("tx_async"));
            let (done, finished) = std::sync::mpsc::channel();
            let owner = core
                .device
                .host()
                .spawn("tx-recovery-race", move || {
                    let worked = tx.pass();
                    let _ = done.send((tx, worked));
                })
                .unwrap();
            if let Some(gate) = &report_gate {
                gate.wait();
                assert_eq!(spy.seen("tx_async"), 1, "the gated return is T3's final ACK read");
            } else {
                assert!(claim_reached.unwrap().recv_timeout(Wall::from_secs(10)).is_ok(), "recovery did not reach the post-claim gate");
            }
            let key = stream_key(Dir::Tx, 0);
            assert_eq!(lock(&core.streams).holder(key), Holder::Owner, "the owner holds its claim at the gate");

            if stop {
                let mut streams = lock(&core.streams);
                let at = core.now();
                streams.stop = Some((at, StopMode::Orderly));
                streams.end(&core, at, false, false);
            } else {
                core.device_lost("test loss during TX recovery");
            }
            let order_after_loss = (!stop).then(|| spy.order().len());
            if let Some(gate) = report_gate {
                gate.release();
            }
            if let Some(release) = claim_release {
                release.wait();
            }

            owner.join().unwrap();
            let (mut tx, worked) = finished.recv_timeout(Wall::from_secs(10)).unwrap();
            assert!(worked, "the owner handled the interrupted recovery");
            assert_eq!(lock(&core.streams).holder(key), Holder::Free, "the recovery claim is always returned");
            assert_eq!(tx.chain.recovery(), Recovery::Clean, "Stop/loss must not mark the chain Halted");
            assert!(tx.held.contains_key(&target), "recovery interruption does not refuse the pending burst");
            assert!(lock(&core.held).contains(&(clock.domain, target)), "the pending reservation survives until Stop cleanup");
            assert!(
                lock(&core.rec).rejected.iter().all(|row| !row["reason"].as_str().is_some_and(|reason| reason.contains("could not be reopened"))),
                "Stop/loss is not reported as a failed-reopen cancellation"
            );
            assert!(spy.log().iter().filter(|call| call.starts_with("tx_send")).all(|call| call.contains("n=2 at=- sob=true eob=true")), "only T3, never experiment payload, is sent: {:?}", spy.log());
            if let Some(start) = order_after_loss {
                assert!(
                    spy.order()[start..].iter().all(|call| !matches!(*call, "tx_close" | "tx_open" | "tx_send" | "tx_async")),
                    "no TX device operation starts after loss publication: {:?}",
                    spy.order()
                );
            }

            tx.command(TxCmd::Stop);
            assert!(!lock(&core.held).contains(&(clock.domain, target)), "the Stop path settles the held reservation");
        }
    }

    #[test]
    fn ur_22_underflow_close_failure_reopens_before_the_next_payload() {
        for (short_close, close_fails, reopen_fault, has_tail) in [
            (false, true, None, true),
            (false, true, None, false),
            (true, false, None, true),
            (false, true, Some(FakeFault::TxOpenFails(1)), true),
            (true, false, Some(FakeFault::RatesMismatch(0)), true),
        ] {
            let mut faults = Vec::new();
            if short_close {
                faults.push(FakeFault::ShortSend(1));
            }
            if let Some(fault) = reopen_fault.clone() {
                faults.push(fault);
            }
            let device = Arc::new(FakeDevice::new(FakeConfig { faults, ..FakeConfig::default() }));
            let (core, time, mut tx, clock) = recovery_rig(device.clone());
            device.set_time(0);

            let start = clock.at_or_after(device.time_now().unwrap() + core.ticks(5_000_000));
            let samples = [[0.1, 0.0]; 100];
            device.tx_send(0, &[&samples], Some(clock.instant(start)), true, false, Wall::from_secs(1)).unwrap();
            tx.chain.open_device_burst(start + samples.len() as i64, has_tail.then_some(Tail { samples: [[0.1, 0.0]; TX_CHANNELS_MAX], channels: 1 }));
            let dry = clock.instant(start + samples.len() as i64) + core.ticks(5_000_000);
            time.advance_to(core.at(dry)).unwrap();
            device.set_time(dry);

            let target = clock.at_or_after(core.now() + core.ticks(60_000_000));
            tx.held.insert(target, held(&clock, target, 1, LatePolicy::DropAndFlag));
            lock(&core.held).insert((clock.domain, target));
            if close_fails {
                *device.eob_error.lock().unwrap() = Some(crate::DeviceError::failed("controlled non-lost EOB failure"));
            }
            tx.pass();
            assert!(tx.chain.needs_reopen(), "underflow's {short_close} close did not require recovery: {:?}", device.calls());
            assert_eq!(tx.chain.next(), None, "the failed close leaves no retained device cursor");
            assert!(tx.chain.tail().is_none(), "the abandoned tail is not replayed after failed close");
            assert!(tx.open.as_ref().is_some_and(|open| open.held.k == target && open.next == target && open.first), "B's new Kernel cursor stays unchanged at the recovery barrier");
            let target_send = format!("at={} sob=true", clock.instant(target));
            assert!(device.calls().iter().all(|call| !call.contains(&target_send)), "B was sent on the uncertain streamer: {:?}", device.calls());

            *device.eob_error.lock().unwrap() = None;
            if reopen_fault.is_some() {
                use ezsdr_kernel::stream::{BlockFlags, BlockHeader, BurstTracker, ChannelMask, Direction};
                let mut tracker = BurstTracker::new(clock.domain);
                tracker
                    .on_block(
                        &BlockHeader {
                            first_sample_time: TimePoint::new(clock.domain, target),
                            len: 3,
                            channels: 1,
                            direction: Direction::Tx,
                            valid: ChannelMask::full(1),
                            flags: BlockFlags::START_OF_BURST,
                            lost: None,
                            contract: ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.cf32").unwrap(),
                        },
                        Some(BurstOpen::default()),
                    )
                    .unwrap();
                tx.tracker = Some(tracker);
                assert!(tx.pass(), "the failed reopen was not handled ({reopen_fault:?})");
                assert!(!tx.chain.needs_reopen(), "the failed reopen is no longer pending ({reopen_fault:?})");
                assert_eq!(tx.chain.recovery(), Recovery::Halted, "a failed reopen enters Halted ({reopen_fault:?})");
                assert!(tx.open.is_none(), "B was cancelled from the send barrier ({reopen_fault:?})");
                assert!(!lock(&core.held).contains(&(clock.domain, target)), "B's reservation was released ({reopen_fault:?})");
                let rejected = lock(&core.rec).rejected.clone();
                assert!(rejected.iter().any(|row| row["reason"] == "UR-22: cancelled because the TX streamer could not be reopened"), "B's cancellation was recorded ({reopen_fault:?}): {rejected:?}");
                let record = lock(&core.rec).bursts.last().cloned().expect("the in-flight burst record was finished");
                assert_eq!(record["end"], "stop", "the in-flight burst record was finished ({reopen_fault:?}): {record}");
                assert_eq!(record["samples"], 3, "the in-flight burst record was preserved ({reopen_fault:?}): {record}");
                let calls = device.calls();
                assert!(calls.iter().all(|call| !call.contains(&target_send)), "B's payload was never sent ({reopen_fault:?}): {calls:?}");
                assert_eq!(device.transmitted(clock.instant(target), 3), [None, None, None], "no B samples reached the device ({reopen_fault:?})");
                continue;
            }
            assert!(tx.pass(), "the owner did not replace the uncertain streamer");
            assert!(!tx.chain.needs_reopen());
            assert!(device.calls().iter().all(|call| !call.contains(&target_send)), "the reopen turn sent B before its T3 completed: {:?}", device.calls());
            assert!(tx.pass(), "B was not sent after recovery");
            let calls = device.calls();
            let opens: Vec<_> = calls.iter().enumerate().filter(|(_, call)| call.starts_with("tx_open 0 ")).map(|(index, _)| index).collect();
            let reopened = *opens.get(1).expect("the streamer was reopened");
            let flush = calls.iter().enumerate().skip(reopened + 1).find(|(_, call)| *call == "tx_send 0 n=2 at=- sob=true eob=true").map(|(index, _)| index).expect("T3 flush");
            let payload = calls.iter().enumerate().skip(flush + 1).find(|(_, call)| call.contains(&target_send)).map(|(index, _)| index).expect("B's first payload");
            assert!(reopened < flush && flush < payload, "reopen/T3/payload order: {calls:?}");
        }
    }

    #[test]
    fn ur_22_owner_keeps_clean_recovery_after_an_unacknowledged_t3() {
        for fault in [FakeFault::DropBurstAck, FakeFault::ShortSend(0), FakeFault::FailSend(0)] {
            let device = Arc::new(FakeDevice::new(FakeConfig { clock: crate::ClockMode::Manual, faults: vec![fault.clone()], ..FakeConfig::default() }));
            let (core, _time, mut tx, clock) = recovery_rig(device.clone());
            let old_front = (clock.domain, 123);
            tx.chain.advance_front(old_front.0, old_front.1);
            let old_target = TimePoint::new(clock.domain, 10);
            tx.remember_burst(old_target);
            let old_generation = tx.streamer;
            let target = clock.at_or_after(core.now() + core.ticks(60_000_000));
            tx.held.insert(target, held(&clock, target, 1, LatePolicy::DropAndFlag));
            lock(&core.held).insert((clock.domain, target));
            tx.chain.mark_reopen_needed();

            let host = core.device.host();
            let clock_host = host.clone();
            let clock_driver = host.spawn("tx-flush-deadline", move || clock_host.sleep(Wall::from_millis(150))).expect("a manual-clock participant");
            assert!(tx.pass(), "the owner did not complete its non-lost reopen ({fault:?})");
            clock_driver.join().unwrap();
            assert_eq!(tx.chain.recovery(), Recovery::Clean, "Clean retains its current meaning ({fault:?})");
            assert_eq!(tx.chain.front(), Some(old_front), "an unacknowledged T3 has no tick to advance the front ({fault:?})");
            assert_ne!(tx.streamer, old_generation, "the replacement generation was adopted ({fault:?})");
            assert!(tx.unacked.is_empty(), "old-generation correlations are discarded ({fault:?})");
            assert_eq!(lock(&core.rec).rejected.iter().filter(|row| row["call"] == "tx_flush").count(), 1, "the incomplete T3 is recorded ({fault:?}): {:?}", lock(&core.rec).rejected);
            assert!(lock(&core.rec).device_async.iter().all(|row| row["tx_flush"] != true), "an absent acknowledgement is not recorded as one ({fault:?})");
            assert!(!device.calls().iter().any(|call| call.contains(&format!("at={} sob=true", clock.instant(target)))), "no experiment payload crossed the reopen turn ({fault:?})");

            assert!(tx.pass(), "Clean recovery did not permit the next owner dispatch ({fault:?})");
            assert!(
                device.calls().iter().any(|call| call == &format!("tx_send 0 n=2 at={} sob=true eob=true", clock.instant(target))),
                "the next timed dispatch follows today's Clean behavior ({fault:?}): {:?}",
                device.calls()
            );
            assert!(tx.unacked.iter().any(|pending| pending.target == TimePoint::new(clock.domain, target)), "the new-generation burst has its own correlation ({fault:?})");
        }

        let config = FakeConfig { clock: crate::ClockMode::Manual, faults: vec![FakeFault::TxReport(Wall::ZERO, TxCode::BurstAck), FakeFault::DropBurstAck], ..FakeConfig::default() };
        let (core, device, _time, _) = super::super::test_support::rig_with(config, Vec::new());
        device.tx_open(0, &[0, 1]).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(2));
        let old_front = (clock.domain, -1);
        tx.chain.advance_front(old_front.0, old_front.1);
        tx.chain.mark_reopen_needed();
        let host = core.device.host();
        let clock_host = host.clone();
        let clock_driver = host.spawn("tx-partial-ack-deadline", move || clock_host.sleep(Wall::from_millis(150))).expect("a manual-clock participant");

        assert!(tx.pass(), "the owner did not finish the partial-ack reopen");
        clock_driver.join().unwrap();
        assert_eq!(tx.chain.recovery(), Recovery::Clean);
        assert_eq!(tx.chain.front(), Some(old_front), "one channel ack is not a completed T3 front");
        let rec = lock(&core.rec);
        assert_eq!(
            rec.device_async.iter().filter(|row| row["tx_flush"] == true).map(|row| row["channel"].as_u64()).collect::<Vec<_>>(),
            [Some(0)],
            "the first channel ack is recorded while the second remains missing"
        );
        assert_eq!(rec.rejected.iter().filter(|row| row["call"] == "tx_flush").count(), 1);
    }

    #[test]
    fn ur_22_nonterminal_reopen_failures_cancel_held_initial_bursts() {
        for fault in [FakeFault::TxOpenFails(1), FakeFault::RatesMismatch(0)] {
            let device = Arc::new(FakeDevice::new(FakeConfig { faults: vec![fault.clone()], ..FakeConfig::default() }));
            let (core, _time, mut tx, clock) = recovery_rig(device.clone());
            {
                let mut streams = lock(&core.streams);
                assert!(tx.plan_section(&mut streams, stream_key(Dir::Tx, 0)), "the initial segment is active before recovery ({fault:?})");
            }
            tx.chain.mark_reopen_needed();
            let target = clock.at_or_after(core.now() + core.ticks(60_000_000));
            tx.held.insert(target, held(&clock, target, 3, LatePolicy::DropAndFlag));
            lock(&core.held).insert((clock.domain, target));

            assert!(tx.pass(), "the failed reopen is handled ({fault:?})");
            assert!(!tx.chain.needs_reopen(), "recovery is no longer pending ({fault:?})");
            assert!(tx.clock.is_none(), "the initial segment was halted ({fault:?})");
            assert!(tx.held.is_empty(), "the held burst was cancelled ({fault:?})");
            assert!(!lock(&core.held).contains(&(clock.domain, target)), "the held reservation was released ({fault:?})");
            let at = clock.instant(target);
            let calls = device.calls();
            assert!(!calls.iter().any(|call| call.contains(&format!("at={at} sob=true"))), "the held payload was never sent ({fault:?}): {calls:?}");
            assert_eq!(device.transmitted(at, 3), [None, None, None], "no held-burst samples reached the device ({fault:?})");
            let rejected = lock(&core.rec).rejected.clone();
            assert!(rejected.iter().any(|row| row["reason"] == "UR-22: cancelled because the TX streamer could not be reopened"), "the cancellation was recorded ({fault:?}): {rejected:?}");
        }
    }

    #[test]
    fn ur_26_provider_stop_ends_recovery_after_stop_and_keeps_shutdown_report_reads() {
        for fail_close in [false, true] {
            let faults = if fail_close { vec![FakeFault::ShortSend(0)] } else { Vec::new() };
            let inner = Arc::new(FakeDevice::new(FakeConfig { clock: crate::ClockMode::Manual, faults, ..FakeConfig::default() }));
            let spy = Arc::new(TxSpy::new(inner));
            let (core, _, _) = super::super::test_support::rig_on(spy.clone(), Vec::new(), 2_000);
            spy.tx_open(0, &[0]).unwrap();
            let clock = core.register(Dir::Tx, 200, 0).unwrap();
            let (_to_tx, cmds) = std::sync::mpsc::channel();
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            let target = clock.at_or_after(core.now() + core.ticks(60_000_000));
            tx.held.insert(target, held(&clock, target, 1, LatePolicy::DropAndFlag));
            lock(&core.held).insert((clock.domain, target));
            if fail_close {
                tx.chain.open_device_burst(100, Some(Tail { samples: [[0.25, 0.0]; TX_CHANNELS_MAX], channels: 1 }));
            } else {
                tx.chain.mark_reopen_needed();
            }

            let host = core.device.host();
            let clock_host = host.clone();
            let clock_driver = host.spawn("tx-shutdown-deadline", move || clock_host.sleep(Wall::from_millis(150))).expect("a manual-clock participant");
            tx.shutdown();
            clock_driver.join().unwrap();

            assert_eq!(tx.chain.recovery(), Recovery::Clean, "Provider::stop follows stop_now (failed close={fail_close})");
            assert!(tx.chain.next().is_none() && tx.chain.tail().is_none());
            assert!(!lock(&core.held).contains(&(clock.domain, target)), "shutdown settles the held reservation");
            let calls = spy.recs();
            assert_eq!(calls.iter().filter(|rec| rec.what == "tx_open").count(), 1, "shutdown never reopens the streamer: {calls:?}");
            assert!(calls.iter().any(|rec| rec.what == "tx_async"), "shutdown retains its report-read window: {calls:?}");
            assert!(!lock(&core.streams).reading(stream_key(Dir::Tx, 0)));
        }
    }

    #[test]
    fn ur_22_tx_stop_preserves_recovery_for_later_bursts() {
        for (fail_stop_close, has_tail) in [(true, true), (true, false), (false, false)] {
            let device = Arc::new(FakeDevice::new(FakeConfig::default()));
            let (core, _time, mut tx, clock) = recovery_rig(device.clone());
            if fail_stop_close {
                tx.chain.open_device_burst(100, has_tail.then_some(Tail { samples: [[0.25, 0.0]; TX_CHANNELS_MAX], channels: 1 }));
                *device.eob_error.lock().unwrap() = Some(crate::DeviceError::failed("controlled Stop EOB failure"));
            } else {
                tx.chain.mark_reopen_needed();
            }

            tx.command(TxCmd::Stop);
            assert!(tx.chain.needs_reopen(), "ordinary Stop cleared the recovery requirement (failed close={fail_stop_close})");
            assert_eq!(tx.chain.next(), None, "Stop abandoned the failed-close cursor");
            assert!(tx.chain.tail().is_none(), "Stop abandoned the failed-close tail");
            *device.eob_error.lock().unwrap() = None;
            let target = clock.at_or_after(core.now() + core.ticks(60_000_000));
            tx.command(TxCmd::Burst(held(&clock, target, 1, LatePolicy::DropAndFlag)));
            let target_send = format!("at={} sob=true", clock.instant(target));

            assert!(tx.pass(), "Stop's pending recovery did not reopen the streamer");
            assert!(device.calls().iter().all(|call| !call.contains(&target_send)), "the reopen turn sent the later burst before T3: {:?}", device.calls());
            assert!(tx.pass(), "the later burst was not sent after recovery");
            let calls = device.calls();
            let reopened = calls.iter().enumerate().filter(|(_, call)| call.starts_with("tx_open 0 ")).nth(1).map(|(index, _)| index).expect("the streamer was reopened");
            let flush = calls.iter().enumerate().skip(reopened + 1).find(|(_, call)| *call == "tx_send 0 n=2 at=- sob=true eob=true").map(|(index, _)| index).expect("T3 flush");
            let payload = calls.iter().enumerate().skip(flush + 1).find(|(_, call)| call.contains(&target_send)).map(|(index, _)| index).expect("later burst payload");
            assert!(reopened < flush && flush < payload, "reopen/T3/payload order (failed close={fail_stop_close}): {calls:?}");
        }
    }

    #[test]
    fn ur_22_underflow_and_loss_end_the_damaged_burst() {
        for code in [crate::device::TxCode::Underflow, crate::device::TxCode::UnderflowInPacket, crate::device::TxCode::SeqError, crate::device::TxCode::SeqErrorInBurst] {
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
            assert!(tx.open.is_none() && tx.chain.next().is_none() && tx.chain.tail().is_none());
            assert!(!lock(&core.held).contains(&(clock.domain, 20_000)));
            assert!(tx.held.contains_key(&100_000), "future bursts retain their reservations");
            let sent = device.calls();
            assert!(!tx.step());
            tx.stop_now("test stop", ezsdr_radio::CancelReason::Stop);
            assert_eq!(device.calls(), sent, "no payload or second EOB after abandonment");
            assert_eq!(lock(&core.rec).bursts[0]["end"], "stop");
            assert!(
                events
                    .drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0))
                    .iter()
                    .any(|e| e.kind == EventKind::parse(ezsdr_radio::kinds::TX_UNDERFLOW).unwrap())
            );
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
            events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().find(|e| e.kind.as_str() == ezsdr_radio::kinds::TX_UNDERFLOW).expect("TX_UNDERFLOW")
        };
        tx.report(crate::device::TxReport { code: crate::device::TxCode::Underflow, tick: Some(clock.instant(22_000) + 1), channel: 0 });
        assert_eq!(underflow(&events).time, TimePoint::new(clock.domain, 22_000), "the report's tick in the sending SampleClock, floored, not the Authority's now");
        // A report with no tick is still timed: at the Authority's now.
        let now = core.now();
        tx.report(crate::device::TxReport { code: crate::device::TxCode::Underflow, tick: None, channel: 0 });
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
        assert_eq!(device.calls().iter().filter(|c| c.starts_with("tx_send ")).count(), 2, "one payload buffer and one empty EOB, no resumed payload");
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
        for _ in 0..20 {
            assert!(!tx.step());
        }
        let stop = device.time_now().unwrap();
        tx.stop_now("test stop", ezsdr_radio::CancelReason::Stop);
        std::thread::sleep(std::time::Duration::from_millis(20));
        tx.reports(std::time::Duration::ZERO);
        let rec = lock(&core.rec);
        let reports: Vec<_> = rec.device_async.iter().map(|report| report["code"].as_str().unwrap()).collect();
        assert_eq!(reports, ["Underflow", "BurstAck"], "the untimed DUC burst accepts its close at once: {:?}", rec.device_async);
        let end = rec.device_async.iter().find(|report| report["code"] == "BurstAck").unwrap()["tick"].as_i64().unwrap();
        assert!(end <= stop + core.ticks(10_000_000), "stop={stop}, playback end={end}");
        assert!(device.calls().iter().any(|call| call.contains("sob=false eob=true")), "the owner attempted a timed two-sample close: {:?}", device.calls());
        assert!(device.calls().iter().filter(|call| call.contains("sob=false eob=true")).all(|call| !call.contains("at=-")), "the close is passed to the DUC with a timestamp; stop={stop}");
    }

    #[test]
    fn ur_21_first_dispatch_rechecks_all_late_policies() {
        for (policy, expected) in [(LatePolicy::DropAndFlag, "drop"), (LatePolicy::RejectAtPlan, "plan_violation"), (LatePolicy::SendAsapAndFlag, "send_asap")] {
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
        // UR-25: an end booked after uhd-tx read the plan reaches it at its next read: the cut
        // of the clock that has not begun, at its origin.
        use ezsdr_radio::timeline::{Channels, Config, Line, Stream};
        let (core, device, _, _) = super::super::test_support::rig();
        device.tx_open(0, &[0]).unwrap();
        let origin = core.ticks(1_000_000_000);
        let c0 = core.register(Dir::Tx, 200, origin).unwrap();
        let config = Config { channels: Channels::first(1), ratio: ezsdr_kernel::time::Rational::new(200, 1).unwrap() };
        {
            let mut streams = lock(&core.streams);
            streams.stream_mut(stream_key(Dir::Tx, 0)).clocks = vec![c0];
            let lead = core.ticks(ezsdr_radio::timeline::start_lead_ns(&core.description.timing, ezsdr_kernel::stream::Direction::Tx));
            streams.lines.insert(stream_key(Dir::Tx, 0), Line::new(Stream { direction: ezsdr_kernel::stream::Direction::Tx, origin, config, lead, call: 0, release_window: 0, allowance: 0 }));
        }
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(c0), Channels::first(1));
        let read = |tx: &mut Tx| {
            let mut streams = lock(&core.streams);
            assert!(tx.plan_section(&mut streams, stream_key(Dir::Tx, 0)));
        };
        read(&mut tx);
        assert!(tx.cut.is_none());
        lock(&core.streams).end(&core, core.ticks(500_000_000), false, false);
        read(&mut tx);
        assert_eq!(tx.cut, Some(origin), "the clock that has not begun ends at its origin");
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
        let mut start = super::super::control::Start { t0: clock.origin, rx: vec![None; core.description.stream_count(Dir::Rx)], tx: vec![None; core.description.stream_count(Dir::Tx)] };
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
        control.book(Dispatched::new(
            ActionId(id),
            Action::TxBurst {
                target: super::super::test_support::target_of(&node),
                waveform: waveform.clone(),
                repeat,
                at: AbsoluteDeadline::new(TimePoint::new(clock.domain, k)),
                requested_at: None,
                late_policy: policy,
                metadata: BTreeMap::new(),
            },
            Some(node),
        ));
    }

    fn drain_tx(tx: &mut Tx) {
        while let Ok(command) = tx.cmds.try_recv() {
            tx.command(command);
        }
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
            let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes.clone())), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes.clone()))]);
            let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
            let _frozen = crate::provider::test_support::Frozen::of(&device);
            device_at(&core, &time, &device, 0);
            device.tx_open(0, &[0]).unwrap();
            device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
            let (mut control, cmds) = booking_control(&core);
            let clock = tx_clock(&core);
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            if case == "booked together" {
                book_burst(&mut control, &core, 1, clock, 4_000, &a_wave, true, LatePolicy::DropAndFlag);
                book_burst(&mut control, &core, 2, clock, 3_000, &b_wave, false, LatePolicy::SendAsapAndFlag);
                drain_tx(&mut tx);
                assert!(tx.step(), "{case}: B did not dispatch");
                assert_eq!(tx.chain.next(), Some(4_000), "{case}: B's on-time burst reaches the next held start");
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
                let late: Vec<_> =
                    events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
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
                assert!(tx.chain.next() == Some(6_500) && tx.held.contains_key(&6_500), "{case}: A's device burst was not continued: {:?}", tx.held.keys());
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
        let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes))]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 9_999, &a_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step(), "the one-sample predecessor was not handed over");
        assert_eq!(tx.chain.front(), Some((clock.domain, 10_002)), "the two-sample device EOB establishes the safe restart sample");

        book_burst(&mut control, &core, 2, clock, 9_999, &b_wave, false, LatePolicy::SendAsapAndFlag);
        drain_tx(&mut tx);
        let calls = device.calls().len();
        assert!(tx.step(), "the first late dispatch did not defer B past the window");
        assert!(tx.held.contains_key(&10_002), "B was not retained at its first safe start");
        let first: Vec<_> =
            events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert_eq!(first.len(), 1, "{first:?}");
        assert_eq!(first[0].payload["outcome"], "send_asap");
        assert_eq!(first[0].payload["target"]["ticks"], 9_999);
        assert_eq!(device.calls().len(), calls, "B is still outside the hand-over window");

        device_at(&core, &time, &device, core.ticks(9_000_000));
        assert!(tx.step(), "B did not resume when its first moved start entered the window");
        assert!(
            events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR),
            "the same action emitted a second TIME_ERROR"
        );
        assert_eq!(device.transmitted(clock.instant(11_000), 1), vec![Some([0.75, -0.25])], "the rechecked start was moved to the current device lead");
    }

    #[test]
    fn ur_21_a_preempted_send_asap_move_keeps_one_error_and_its_requested_target() {
        let a_samples = vec![[0.25, 0.0]; 2_000];
        let (a_wave, a_bytes) = waveform_input("preempt_repeat", &a_samples);
        let (b_wave, b_bytes) = waveform_input("preempted_send_asap", &[[0.75, -0.25], [0.5, 0.125]]);
        let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes))]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 5_500, &a_wave, true, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        device_at(&core, &time, &device, core.ticks(3_000_000));
        while tx.open.as_ref().is_none_or(|open| open.next < 13_000) {
            assert!(tx.step(), "A did not reach cursor 13 000");
        }
        assert_eq!(tx.open.as_ref().unwrap().next, 13_000, "A's first unhanded sample");

        book_burst(&mut control, &core, 2, clock, 1_000, &b_wave, false, LatePolicy::SendAsapAndFlag);
        drain_tx(&mut tx);
        assert!(tx.held.contains_key(&5_000), "control did not move B to sample 5 000: {:?}", tx.held.keys().collect::<Vec<_>>());
        assert!(tx.step(), "B did not preempt A's hand-over cursor");
        let errors: Vec<_> =
            events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert_eq!(errors.len(), 1, "the control's first late decision is not repeated by preemption: {errors:?}");
        assert_eq!(errors[0].payload["outcome"], "send_asap");
        assert_eq!(errors[0].payload["target"]["ticks"], 1_000, "the original target is reported");
        assert!(tx.held.contains_key(&13_000), "preemption did not move B to sample 13 000");
        assert_eq!(tx.held[&13_000].open.requested_target, Some(TimePoint::new(clock.domain, 1_000)));

        device_at(&core, &time, &device, core.ticks(4_000_000));
        assert!(tx.step(), "the moved continuation was not sent");
        assert_eq!(device.transmitted(clock.instant(13_000), 1), vec![Some([0.75, -0.25])], "B starts at A's first unhanded sample");
        assert!(
            events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR),
            "the already reported move emitted another TIME_ERROR"
        );
    }

    #[test]
    fn ur_21_tx_front_is_per_stream() {
        let a_samples = vec![[0.25, 0.0]; 2_000];
        let b_samples = vec![[0.75, -0.25]; 2_000];
        let (a_wave, a_bytes) = waveform_input("stream0", &a_samples);
        let (b_wave, b_bytes) = waveform_input("stream1", &b_samples);
        let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes))]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.tx_open(1, &[1]).unwrap();
        for channel in [0, 1] {
            device.apply(Dir::Tx, channel, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        }
        let c0 = core.register_in(Dir::Tx, 0, 200, 0).unwrap();
        let c1 = core.register_in(Dir::Tx, 1, 200, 0).unwrap();
        let (to0, cmds0) = std::sync::mpsc::channel();
        let (to1, cmds1) = std::sync::mpsc::channel();
        let start = super::super::control::Start { t0: 0, rx: vec![None; core.description.stream_count(Dir::Rx)], tx: vec![Some((c0, Channels::first(1))), Some((c1, Channels::first(1)))] };
        let mut control = Control::new(core.clone(), Arc::new(NoActions), vec![to0, to1], core.description.defaults(), false, start);
        let mut tx0 = Tx::new(core.clone(), 0, cmds0, Some(c0), Channels::first(1));
        let mut tx1 = Tx::new(core.clone(), 1, cmds1, Some(c1), Channels::first(1));
        book_burst_on(&mut control, &core, 1, 0, c0, 4_000, &a_wave, true, LatePolicy::DropAndFlag);
        book_burst_on(&mut control, &core, 2, 1, c1, 3_000, &b_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx0);
        drain_tx(&mut tx1);
        assert!(tx0.step() && tx1.step());
        assert_eq!(tx1.chain.next(), Some(5_000), "stream 0's hand-over front did not leak");
        assert!(device.calls().iter().any(|call| call.starts_with("tx_send 1 ")));
        assert!(
            device.calls().iter().any(|call| { call.starts_with("tx_send 1 ") && call.contains(&format!("at={} sob=true", c1.instant(3_000))) }),
            "stream 1's B did not start at its own target: {:?}",
            device.calls()
        );
        assert!(events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR));
    }

    #[test]
    fn ur_21_a_target_after_a_completed_burst_is_on_time() {
        let a_samples = vec![[0.25, 0.0]; 2_000];
        let b_samples = vec![[0.75, -0.25]; 1_000];
        let (a_wave, a_bytes) = waveform_input("expired_a", &a_samples);
        let (b_wave, b_bytes) = waveform_input("expired_b", &b_samples);
        let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes))]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 2_000, &a_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step());
        device_at(&core, &time, &device, core.ticks(2_000_000));
        assert!(tx.step(), "A's EOB was not sent");
        assert_eq!(tx.chain.front(), Some((clock.domain, 4_002)));
        device_at(&core, &time, &device, core.ticks(10_000_000));
        book_burst(&mut control, &core, 2, clock, 13_000, &b_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step(), "the burst after the front's expiry did not dispatch");
        assert!(events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR));
        assert!(device.calls().iter().any(|call| call.contains(&format!("at={}", clock.instant(13_000))) && call.contains("sob=true")));
    }

    #[test]
    fn ur_21_send_does_not_cross_the_in_flight_window() {
        let a_samples = vec![[0.25, 0.0]; 30_000];
        let b_samples = vec![[0.75, -0.25], [0.5, 0.25]];
        let (a_wave, a_bytes) = waveform_input("window_repeat", &a_samples);
        let (b_wave, b_bytes) = waveform_input("window_boundary", &b_samples);
        for (rate, root_ticks_per_sample, start, expected_cut) in [(1e6, 200, 9_000, 10_000), (390_625.0, 512, 979, 3_907)] {
            let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
            let device = Arc::new(FakeDevice::new(config));
            let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes.clone())), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes.clone()))]);
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
                let late: Vec<_> =
                    events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
                assert_eq!(late.len(), 1);
                assert_eq!(late[0].payload["outcome"], "drop");
                assert_eq!(device.calls().len(), calls, "the late B reached the device");
                book_burst(&mut control, &core, 3, clock, 3_907, &b_wave, false, LatePolicy::DropAndFlag);
                drain_tx(&mut tx);
                device_at(&core, &time, &device, 1_000);
                assert!(tx.step(), "the first unhanded sample was not admitted");
                assert!(tx.step(), "the boundary burst did not continue");
                assert!(
                    events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR),
                    "the cursor boundary itself is on time"
                );
                assert_eq!(device.transmitted(clock.instant(3_907), 1), vec![Some([0.75, -0.25])]);
            } else {
                book_burst(&mut control, &core, 2, clock, expected_cut, &b_wave, false, LatePolicy::DropAndFlag);
                drain_tx(&mut tx);
                device_at(&core, &time, &device, 201);
                assert!(tx.step(), "the aligned boundary burst was not admitted");
                assert!(tx.step(), "the aligned boundary burst did not hand over");
                assert!(
                    events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR),
                    "the +10 ms sample boundary is on time"
                );
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
        let (core, time, _) = super::super::test_support::rig_on_description(device.clone(), Vec::new(), BTreeMap::from([(wave.hash.clone(), Arc::<[u8]>::from(bytes.clone()))]), description);
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
        let (core, time, _) = super::super::test_support::rig_on_description(device.clone(), Vec::new(), BTreeMap::from([(wave.hash.clone(), Arc::<[u8]>::from(bytes))]), description);
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
        let (core, time, _) = super::super::test_support::rig_on_description(device.clone(), Vec::new(), BTreeMap::from([(wave.hash.clone(), Arc::<[u8]>::from(bytes))]), description);
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
        let errors: Vec<_> =
            events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
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
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, BTreeMap::from([(wave.hash.clone(), Arc::<[u8]>::from(bytes))]));
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 5_000, &wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step());
        assert!(events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).is_empty());
        assert_eq!(sends(&device).last(), Some(&(2, true, true)));
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
        let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes))]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 3_000, &a_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step());
        assert!(tx.open.is_none() && tx.chain.next() == Some(5_000));
        device_at(&core, &time, &device, core.ticks(3_000_000));
        assert!(tx.step(), "the final sample and EOB were not handed");
        assert_eq!(tx.chain.front(), Some((clock.domain, 5_002)));
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
        let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes))]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 2_500, &a_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        while tx.open.as_ref().is_none_or(|open| open.next < 10_000) {
            assert!(tx.step());
        }
        assert_eq!(tx.open.as_ref().unwrap().next, 10_000, "finite A is clamped by W, not its end");
        let calls = device.calls().len();
        book_burst(&mut control, &core, 2, clock, 5_000, &b_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step());
        let late: Vec<_> = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
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
        let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes))]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 2_500, &a_wave, true, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        while tx.open.as_ref().is_none_or(|open| open.next < 10_000) {
            assert!(tx.step());
        }
        assert_eq!(tx.open.as_ref().unwrap().next, 10_000);
        book_burst(&mut control, &core, 2, clock, 6_000, &b_wave, false, LatePolicy::SendAsapAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step(), "B did not preempt the hand-over cursor");
        let late: Vec<_> = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert_eq!(late.len(), 1);
        assert_eq!(late[0].payload["outcome"], "send_asap");
        assert_eq!(late[0].payload["late_by_ns"], 4_000_000);
        assert_eq!(late[0].payload["target"]["ticks"], 6_000);
        assert_eq!(tx.chain.next(), Some(10_000));
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
        let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes)), (c_wave.hash.clone(), Arc::<[u8]>::from(c_bytes))]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 2_500, &a_wave, true, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        while tx.open.as_ref().is_none_or(|open| open.next < 10_000) {
            assert!(tx.step());
        }
        assert_eq!(tx.open.as_ref().unwrap().next, 10_000);

        book_burst(&mut control, &core, 2, clock, 10_000, &c_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        book_burst(&mut control, &core, 3, clock, 5_000, &b_wave, false, LatePolicy::SendAsapAndFlag);
        drain_tx(&mut tx);
        device_at(&core, &time, &device, core.ticks(9_000_000));
        assert!(tx.step(), "the earlier B was not rechecked against the now-later ordinary bound");
        assert!(tx.open.is_none() && tx.chain.next().is_none());
        assert_eq!(tx.chain.front(), Some((clock.domain, 10_003)), "A closes before the moved B");
        assert_eq!(sends(&device).last(), Some(&(2, false, true)), "A's open device burst is ended at its cursor");
        let moved =
            events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect::<Vec<_>>();
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0].payload["outcome"], "send_asap");
        assert_eq!(moved[0].payload["target"]["ticks"], 5_000);
        assert!(tx.held.contains_key(&10_000) && tx.held.contains_key(&11_000));

        assert!(tx.step(), "C at the cursor was not judged after A's device EOB");
        let dropped =
            events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect::<Vec<_>>();
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
            let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes.clone())), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes.clone()))]);
            let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
            let _frozen = crate::provider::test_support::Frozen::of(&device);
            device_at(&core, &time, &device, 0);
            device.tx_open(0, &[0]).unwrap();
            device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
            let (mut control, cmds) = booking_control(&core);
            let clock = tx_clock(&core);
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            book_burst(&mut control, &core, 1, clock, 2_500, &a_wave, true, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            while tx.open.as_ref().is_none_or(|open| open.next < 10_000) {
                assert!(tx.step());
            }
            assert_eq!(tx.open.as_ref().unwrap().next, 10_000);
            let target = 10_000 + delta;
            book_burst(&mut control, &core, 2, clock, target, &b_wave, false, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            device_at(&core, &time, &device, 401);
            assert!(tx.step(), "A did not reach B's boundary");
            if delta == 1 {
                assert_eq!(tx.chain.next(), Some(10_001), "A's device burst stays open through the target boundary");
                assert!(tx.step(), "B at {target} was not admitted at equality");
            } else {
                assert!(tx.step(), "B at {target} did not continue");
            }
            assert!(
                events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR),
                "B at {target} is on time"
            );
            assert_eq!(device.transmitted(clock.instant(target), 1), vec![Some([0.75, -0.25])]);
        }
    }

    #[test]
    fn ur_23_stop_preserves_handed_cursor_after_stop() {
        let a_samples = vec![[0.25, 0.0]; 2_000];
        let b_samples = vec![[0.75, -0.25]];
        let (a_wave, a_bytes) = waveform_input("repeat", &a_samples);
        let (b_wave, b_bytes) = waveform_input("after_stop", &b_samples);
        for delta in 0..=3 {
            let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
            let device = Arc::new(FakeDevice::new(config));
            let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes.clone())), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes.clone()))]);
            let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
            let _frozen = crate::provider::test_support::Frozen::of(&device);
            device_at(&core, &time, &device, 0);
            device.tx_open(0, &[0]).unwrap();
            device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
            let (mut control, cmds) = booking_control(&core);
            let clock = tx_clock(&core);
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            book_burst(&mut control, &core, 1, clock, 2_500, &a_wave, true, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            while tx.open.as_ref().is_none_or(|open| open.next < 10_000) {
                assert!(tx.step(), "A stopped making progress before the 10 ms hand-over frontier");
            }
            assert_eq!(tx.open.as_ref().unwrap().next, 10_000, "the future A is handed to H before Stop");
            control.book(Dispatched::new(ActionId(2), Action::Stop { target: Some(super::super::test_support::target_of(&core.tx_id)) }, Some(core.tx_id.clone())));
            drain_tx(&mut tx);
            let record = lock(&core.rec).bursts.last().cloned().expect("the stopped repeat record");
            assert_eq!(record["end"], "stop");
            assert_eq!(record["samples"], 7_500);
            assert_eq!(sends(&device).last(), Some(&(2, false, true)), "Stop closes the device burst with two zeros at H");

            let target = 10_000 + delta;
            let before = device.calls().len();
            device_at(&core, &time, &device, core.ticks((delta + 1) * 1_000 + 1));
            book_burst(&mut control, &core, 3 + delta as u64, clock, target, &b_wave, false, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            assert!(tx.step(), "B at {target} was not considered");
            let errors: Vec<_> =
                events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
            if delta < 3 {
                assert_eq!(errors.len(), 1, "B at {target}: {errors:?}");
                assert_eq!(errors[0].payload["outcome"], "drop");
                assert_eq!(errors[0].payload["late_by_ns"], (3 - delta) * 1_000);
                assert_eq!(device.calls().len(), before, "a dropped B reached the device");
            } else {
                assert!(errors.is_empty(), "the first safe sample is on time: {errors:?}");
                assert_eq!(sends(&device).last(), Some(&(2, true, true)), "B starts with a timed SOB after H+3");
                assert_eq!(device.transmitted(clock.instant(10_000), 4), vec![Some([0.0, 0.0]), Some([0.0, 0.0]), None, Some([0.75, -0.25])]);
                device_at(&core, &time, &device, clock.instant(10_004));
                tx.reports(Wall::ZERO);
                assert!(lock(&core.rec).device_async.iter().any(|row| row["code"] == "BurstAck" && row["tick"] == clock.instant(10_004)), "the safe burst's acknowledgement is read");
            }
        }
    }

    #[test]
    fn ur_23_stop_does_not_send_unhanded_window_edge_payload() {
        let (a_wave, a_bytes) = waveform_input("window_edge_a", &[[0.5, 0.0]]);
        let (b_wave, b_bytes) = waveform_input("window_edge_b", &[[0.0, 0.5]]);
        let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes))]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 10_000, &b_wave, false, LatePolicy::DropAndFlag);
        book_burst(&mut control, &core, 2, clock, 9_999, &a_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step(), "the one-sample A at 9 999 was not handed over");
        assert_eq!(sends(&device).last(), Some(&(1, true, false)), "A stays open for the held burst at the window edge");
        tx.stop_now("test Stop", ezsdr_radio::CancelReason::Stop);
        assert_eq!(sends(&device).last(), Some(&(2, false, true)), "Stop closes the open device burst with two zeros");

        assert_eq!(lock(&core.rec).bursts.len(), 1, "only A has a burst record");
        assert_eq!(lock(&core.rec).bursts[0]["samples"], 1);
        assert_eq!(device.transmitted(clock.instant(9_999), 3), vec![Some([0.5, 0.0]), Some([0.0, 0.0]), Some([0.0, 0.0])], "FakeDevice received only A and the two-zero EOB, not B");

        let before = device.calls().len();
        device_at(&core, &time, &device, core.ticks(2_001));
        book_burst(&mut control, &core, 3, clock, 10_002, &b_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step(), "C at H+1 was not considered");
        let errors: Vec<_> =
            events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert_eq!(errors.len(), 1, "C at H+1 is late: {errors:?}");
        assert_eq!(errors[0].payload["outcome"], "drop");
        assert_eq!(errors[0].payload["late_by_ns"], 1_000);
        assert_eq!(device.calls().len(), before, "late C reached FakeDevice");

        device_at(&core, &time, &device, core.ticks(3_001));
        book_burst(&mut control, &core, 4, clock, 10_003, &b_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step(), "C at H+2 was not considered");
        assert!(events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "C at H+2 is on time");
        assert_eq!(sends(&device).last(), Some(&(2, true, true)), "C starts a new device burst at H+3");
        assert_eq!(device.transmitted(clock.instant(10_000), 4), vec![Some([0.0, 0.0]), Some([0.0, 0.0]), None, Some([0.0, 0.5])]);
    }

    #[test]
    fn ur_23_stop_closes_a_running_repeat_at_its_handed_cursor() {
        let samples = vec![[0.25, 0.0]; 2_000];
        let (wave, bytes) = waveform_input("running_repeat", &samples);
        let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let (core, time, _) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, BTreeMap::from([(wave.hash.clone(), Arc::<[u8]>::from(bytes))]));
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 2_000, &wave, true, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step());
        device_at(&core, &time, &device, core.ticks(4_000_000));
        while tx.open.as_ref().is_some_and(|open| open.next < 12_000) {
            assert!(tx.step());
        }
        assert_eq!(tx.open.as_ref().unwrap().next, 12_000);
        control.book(Dispatched::new(ActionId(2), Action::Stop { target: Some(super::super::test_support::target_of(&core.tx_id)) }, Some(core.tx_id.clone())));
        drain_tx(&mut tx);
        let record = lock(&core.rec).bursts.last().cloned().unwrap();
        assert_eq!(record["end"], "stop");
        assert_eq!(record["samples"], 10_000);
        assert_eq!(device.transmitted(clock.instant(2_000), 10_002), vec![Some([0.25, 0.0]); 10_000].into_iter().chain([Some([0.0, 0.0]), Some([0.0, 0.0])]).collect::<Vec<_>>());
    }

    #[test]
    fn ur_21_a_finite_burst_with_a_held_tail_keeps_its_front() {
        let a_samples = vec![[0.25, 0.0]; 6_000];
        let b_samples: Vec<[f32; 2]> = (0..1_000).map(|n| [-0.5, n as f32 / 1_000.0]).collect();
        let (a_wave, a_bytes) = waveform_input("finite_a", &a_samples);
        let (b_wave, b_bytes) = waveform_input("finite_b", &b_samples);
        let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes))]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 2_000, &a_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        for _ in 0..3 {
            assert!(tx.step(), "A did not hand over its three chunks");
        }
        assert!(tx.open.is_none() && tx.chain.next() == Some(8_000) && tx.chain.tail().as_ref().is_some_and(|tail| tail.channels == 1), "A's last sample is held: {:?}", device.calls());

        book_burst(&mut control, &core, 2, clock, 5_000, &b_wave, false, LatePolicy::SendAsapAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step(), "the late continuation was not decided");
        let late: Vec<_> = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert_eq!(late.len(), 1);
        assert_eq!(late[0].payload["outcome"], "send_asap");
        assert_eq!(late[0].payload["late_by_ns"], 3_000_000);
        assert_eq!(late[0].payload["target"]["ticks"], 5_000);
        assert!(tx.chain.next() == Some(8_000) && tx.held.contains_key(&8_000), "B moves to A's exclusive end");
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
        for (target, expect_late) in [(2_002, true), (2_003, false)] {
            let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
            let device = Arc::new(FakeDevice::new(config));
            let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes.clone())), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes.clone()))]);
            let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
            let _frozen = crate::provider::test_support::Frozen::of(&device);
            device_at(&core, &time, &device, 0);
            device.tx_open(0, &[0]).unwrap();
            device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
            let (mut control, cmds) = booking_control(&core);
            let clock = tx_clock(&core);
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            book_burst(&mut control, &core, 1, clock, 2_000, &a_wave, false, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            assert!(tx.step(), "A's one-sample device burst was not ended");
            assert_eq!(tx.chain.front(), Some((clock.domain, 2_003)), "A's two-sample EOB leaves the exclusive end plus one safe sample");
            assert_eq!(sends(&device).first(), Some(&(2, true, true)));

            book_burst(&mut control, &core, 2, clock, target, &b_wave, false, LatePolicy::SendAsapAndFlag);
            drain_tx(&mut tx);
            assert!(tx.step(), "B was not dispatched");
            let errors: Vec<_> =
                events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
            if expect_late {
                assert_eq!(errors.len(), 1, "{errors:?}");
                assert_eq!(errors[0].payload["outcome"], "send_asap");
                assert_eq!(errors[0].payload["late_by_ns"], 1_000);
                assert_eq!(errors[0].payload["target"]["ticks"], target);
            } else {
                assert!(errors.is_empty(), "{errors:?}");
            }
            assert_eq!(sends(&device).last(), Some(&(2, true, true)));
            assert_eq!(device.transmitted(clock.instant(2_003), 1), vec![Some([0.75, -0.25])], "B's actual start");
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
        let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes))]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 2_000, &a_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        for _ in 0..3 {
            assert!(tx.step());
        }
        assert!(tx.open.is_none() && tx.chain.next() == Some(8_000) && tx.chain.tail().is_some());
        device_at(&core, &time, &device, core.ticks(6_000_000));
        assert!(tx.step(), "the held last sample was not closed with EOB");
        assert_eq!(tx.chain.front(), Some((clock.domain, 8_002)));
        assert_eq!(sends(&device).last(), Some(&(2, false, true)));

        book_burst(&mut control, &core, 2, clock, 8_000, &b_wave, false, LatePolicy::SendAsapAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step(), "B did not dispatch after A's EOB");
        let late: Vec<_> = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert_eq!(late.len(), 1);
        assert_eq!(late[0].payload["outcome"], "send_asap");
        assert_eq!(late[0].payload["late_by_ns"], 2_000);
        assert_eq!(late[0].payload["target"]["ticks"], 8_000);
        assert_eq!(sends(&device).last(), Some(&(2, true, true)), "B uses a timed SOB at the close's safe sample");
        assert_eq!(device.transmitted(clock.instant(7_998), 5), vec![Some([0.25, 0.0]), Some([0.25, 0.0]), Some([0.0, 0.0]), None, Some([0.75, -0.25])]);
        device_at(&core, &time, &device, clock.instant(8_004));
        tx.reports(Wall::ZERO);
        assert!(lock(&core.rec).device_async.iter().all(|row| row["code"] != "Underflow" && row["code"] != "TimeError"));
        assert!(lock(&core.rec).device_async.iter().any(|row| row["code"] == "BurstAck" && row["tick"] == clock.instant(8_000)));
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
        let input = |name: &str, bytes: &[u8]| {
            ezsdr_kernel::manifest::ingest_input(
                ezsdr_kernel::spec::Ident::parse(name).unwrap(),
                ezsdr_kernel::spec::Namespace::parse("ezsdr.input").unwrap(),
                format!("mem:{}", ContentHash::of_bytes(bytes)),
                bytes,
            )
        };
        let (a_wave, b_wave) = (input("waveform", &a_bytes), input("waveform2", &b_bytes));
        for (policy, outcome) in [(LatePolicy::SendAsapAndFlag, "send_asap"), (LatePolicy::DropAndFlag, "drop")] {
            let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, ..FakeConfig::default() };
            let device = Arc::new(FakeDevice::new(config));
            let (core, time, events) = crate::provider::test_support::rig_on_inputs(
                device.clone(),
                Vec::new(),
                2_000,
                BTreeMap::from([(a_hash.clone(), Arc::<[u8]>::from(a_bytes.clone())), (b_hash.clone(), Arc::<[u8]>::from(b_bytes.clone()))]),
            );
            let _frozen = crate::provider::test_support::Frozen::of(&device);
            device_at(&core, &time, &device, 0);
            device.tx_open(0, &[0]).unwrap();
            device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
            let (mut control, handed) = booking_control(&core);
            let clock = tx_clock(&core);
            let (_to_tx, cmds) = std::sync::mpsc::channel();
            std::mem::forget(_to_tx);
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            let target = super::super::test_support::target_of(&core.tx_id);
            let mut book = |id: u64, k: i64, repeat: bool, wave: &ezsdr_kernel::manifest::ArtifactRef| {
                control.book(Dispatched::new(
                    ActionId(id),
                    Action::TxBurst {
                        target: target.clone(),
                        waveform: wave.clone(),
                        repeat,
                        at: AbsoluteDeadline::new(TimePoint::new(clock.domain, k)),
                        requested_at: None,
                        late_policy: policy,
                        metadata: BTreeMap::new(),
                    },
                    Some(core.tx_id.clone()),
                ));
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
                assert!(tx.open.is_none() && tx.chain.next() == Some(35_000), "{policy:?}: A is not kept for the continuation");
                assert!(tx.held.contains_key(&35_000) && !tx.held.contains_key(&34_000), "{policy:?}: {:?}", tx.held.keys().collect::<Vec<_>>());
                let reserved: Vec<_> = lock(&core.held).iter().copied().collect();
                assert!(reserved.contains(&(clock.domain, 35_000)) && !reserved.contains(&(clock.domain, 34_000)), "{policy:?}: {reserved:?}");
                device_at(&core, &time, &device, core.ticks(30_000_000));
                assert!(tx.step(), "{policy:?}: B is not handed over at the sample A ended at");
                assert_eq!(
                    device.transmitted(clock.instant(35_000), 1_000),
                    b_samples.iter().copied().map(Some).collect::<Vec<_>>(),
                    "{policy:?}: B does not play where uhd-tx had not handed A over: {:?}",
                    device.calls()
                );
                assert_eq!(sends(&device).last(), Some(&(2_500, false, false)), "{policy:?}: B did not continue A's device burst: {:?}", sends(&device));
                assert!(lock(&core.rec).device_async.iter().all(|row| row["code"] != "TimeError" && row["code"] != "Underflow"), "{policy:?}: {:?}", lock(&core.rec).device_async);
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
            if tx.open.as_ref().is_some_and(|open| open.next >= a + 4_000) {
                break;
            }
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
                let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
                *device.eob_error.lock().unwrap() = Some(crate::device::DeviceError { lost, message: "controlled EOB failure".to_owned() });
                tx.close_device_burst(tail, 100, clock);
                assert_eq!(core.is_lost(), lost, "EOB failure must reach UR-29");
                let emitted = events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
                let expected = if lost { EventKind::DEVICE_LOST } else { ezsdr_radio::kinds::COMMAND_REJECTED };
                assert_eq!(emitted.iter().filter(|event| event.kind.as_str() == expected).count(), 1);
                if lost {
                    tx.close_device_burst(None, 100, clock);
                    assert!(events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).is_empty());
                    assert_eq!(device.calls().iter().filter(|call| call.as_str() == "mark_lost").count(), 1);
                }
            }
        }
    }

    #[test]
    fn ur_29_non_lost_payload_eob_error_after_concurrent_loss_does_not_reopen() {
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: crate::ClockMode::Manual, ..FakeConfig::default() }));
        let (core, _, mut tx, clock) = recovery_rig(device.clone());
        tx.chain.advance_front(clock.domain, 123);
        let front = tx.chain.front();
        let target = clock.at_or_after(core.now() + core.ticks(60_000_000));
        tx.command(TxCmd::Burst(held(&clock, target, 1, LatePolicy::DropAndFlag)));
        *device.eob_error.lock().unwrap() = Some(crate::DeviceError::failed("controlled non-lost payload EOB failure"));
        let (entered, release) = device.gate_next_payload_eob_send();
        let (finished, result) = std::sync::mpsc::channel();
        let owner = core
            .device
            .host()
            .spawn("tx-eob-loss-race", move || {
                let worked = tx.step();
                let _ = finished.send((tx, worked));
            })
            .unwrap();

        assert!(entered.recv_timeout(Wall::from_secs(10)).is_ok(), "the payload-bearing EOB send did not reach its gate");
        core.device_lost("test loss while the payload EOB send is outstanding");
        let calls_after_loss = device.calls();
        release.send(()).unwrap();
        owner.join().unwrap();
        let (mut tx, worked) = result.recv_timeout(Wall::from_secs(10)).expect("the TX step returned");

        assert!(worked);
        assert!(core.is_lost());
        assert!(tx.open.is_none() && tx.chain.next().is_none() && tx.chain.tail().is_none(), "the failed burst is closed");
        assert!(!tx.chain.needs_reopen(), "published loss must not reintroduce ReopenNeeded");
        assert_eq!(tx.chain.recovery(), Recovery::Clean);
        assert_eq!(tx.chain.front(), front, "the failed EOB leaves the safe front unchanged");
        assert_eq!(device.calls(), calls_after_loss, "the outstanding send made no device call after loss publication");
        assert!(!tx.step(), "the lost owner does no further work");
        assert_eq!(device.calls(), calls_after_loss, "no later device operation follows loss");
    }

    #[test]
    fn ur_29_eob_loss_at_the_cut_stops_device_calls() {
        let (core, device, _, _) = super::super::test_support::rig();
        device.tx_open(0, &[0]).unwrap();
        let old = core.register(Dir::Tx, 200, 0).unwrap();
        core.clocks.end(old.domain, core.at(20_000_000)).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(old), Channels::first(1));
        tx.chain.open_device_burst(100, Some(Tail { samples: [[0.25, 0.0]; TX_CHANNELS_MAX], channels: 1 }));
        tx.cut = Some(20_000_000);
        *device.eob_error.lock().unwrap() = Some(crate::device::DeviceError { lost: true, message: "controlled EOB loss at the cut".to_owned() });
        let before = device.calls().len();
        tx.end_clock();
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
        tx.chain.open_device_burst(100, None);
        tx.held.insert(3000, held(&clock, 3000, 10, LatePolicy::DropAndFlag));
        *device.eob_error.lock().unwrap() = Some(crate::device::DeviceError { lost: true, message: "controlled EOB loss after underflow".to_owned() });
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
        tx.cut = Some(cut);
        assert!(!tx.step());
        assert!(tx.cut.is_some());
    }

    #[test]
    fn ur_28_per_channel_ack_preserves_the_next_burst() {
        let (core, _, _, events) = super::super::test_support::rig();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core, 0, cmds, Some(clock), Channels::first(2));
        let a = TimePoint::new(clock.domain, 10_000);
        let b = TimePoint::new(clock.domain, 20_000);
        tx.remember_burst(a);
        tx.remember_burst(b);
        for channel in [0, 1] {
            tx.report(crate::device::TxReport { code: crate::device::TxCode::BurstAck, tick: Some(clock.instant(a.ticks_in(clock.domain).unwrap())), channel });
        }
        tx.report(crate::device::TxReport { code: crate::device::TxCode::TimeError, tick: Some(clock.instant(b.ticks_in(clock.domain).unwrap()) + 200), channel: 0 });
        let emitted = events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        eprintln!("events={emitted:?}");
        assert!(emitted.iter().any(|e| e.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR), "burst B TIME_ERROR must survive both ACKs of A");
    }

    #[test]
    fn ur_28_ambiguous_time_errors_leave_each_channel_pending() {
        let (core, _, _, events) = super::super::test_support::rig();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(2));
        let a = TimePoint::new(clock.domain, 10_000);
        let b = TimePoint::new(clock.domain, 20_000);
        tx.remember_burst(a);
        tx.remember_burst(b);

        tx.report(TxReport { code: TxCode::TimeError, tick: Some(clock.instant(25_000)), channel: 0 });

        assert!(time_errors(&events).is_empty(), "a current-time tick matching both bursts is ambiguous");
        let channels = std::collections::BTreeSet::from([0, 1]);
        assert_eq!(tx.unacked.iter().map(|pending| (pending.target, pending.channels.clone())).collect::<Vec<_>>(), [(a, channels.clone()), (b, channels)]);
        assert_eq!(lock(&core.rec).device_async.iter().filter(|row| row["code"] == "TimeError").count(), 1, "the raw radio-current-time report is retained");
    }

    #[test]
    fn ur_28_late_burst_error_and_later_ack_use_report_ticks() {
        let (core, _, _, events) = super::super::test_support::rig();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core, 0, cmds, Some(clock), Channels::first(1));
        let late = TimePoint::new(clock.domain, 10_000);
        let future = TimePoint::new(clock.domain, 1_000_000);
        tx.remember_burst(late);
        tx.remember_burst(future);

        tx.report(TxReport { code: TxCode::TimeError, tick: Some(clock.instant(15_000)), channel: 0 });
        let errors = time_errors(&events);
        assert_eq!(errors.len(), 1, "the current-time report uniquely names the late burst");
        assert_eq!(errors[0].payload["target"], serde_json::to_value(late).unwrap());
        assert_eq!(tx.unacked.len(), 2, "a TimeError does not retire its burst");

        tx.report(TxReport { code: TxCode::BurstAck, tick: Some(clock.instant(1_000_100)), channel: 0 });
        assert!(tx.unacked.is_empty(), "the later burst's ack also retires the earlier unacknowledged burst");
    }

    #[test]
    fn ur_28_a_ticked_ack_does_not_retire_a_future_burst() {
        let (core, _, _, _) = super::super::test_support::rig();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core, 0, cmds, Some(clock), Channels::first(1));
        let a = TimePoint::new(clock.domain, 10_000);
        let b = TimePoint::new(clock.domain, 20_000);
        tx.remember_burst(a);
        tx.remember_burst(b);

        tx.report(TxReport { code: TxCode::BurstAck, tick: Some(clock.instant(15_000)), channel: 0 });

        assert_eq!(tx.unacked.iter().map(|pending| pending.target).collect::<Vec<_>>(), [b], "an ack retires every earlier target, but not one starting after its tick");
    }

    #[test]
    fn ur_28_a_tickless_burst_ack_retires_only_the_first_pending_target() {
        let (core, _, _, _) = super::super::test_support::rig();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core, 0, cmds, Some(clock), Channels::first(1));
        let a = TimePoint::new(clock.domain, 10_000);
        let b = TimePoint::new(clock.domain, 20_000);
        tx.remember_burst(a);
        tx.remember_burst(b);

        tx.report(TxReport { code: TxCode::BurstAck, tick: None, channel: 0 });
        assert_eq!(tx.unacked.iter().map(|pending| pending.target).collect::<Vec<_>>(), [b], "a tickless ack uses the first-pending rule");
        tx.report(TxReport { code: TxCode::BurstAck, tick: None, channel: 0 });
        assert!(tx.unacked.is_empty());
    }

    #[test]
    fn ur_28_tickless_time_errors_keep_the_first_pending_target() {
        use crate::device::{TxCode, TxReport};
        for channels in [1, 2] {
            let (core, _, _, events) = super::super::test_support::rig();
            let clock = core.register(Dir::Tx, 200, 0).unwrap();
            let (_to_tx, cmds) = std::sync::mpsc::channel();
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(channels as u32));
            let a = TimePoint::new(clock.domain, 10_000);
            let b = TimePoint::new(clock.domain, 20_000);
            tx.remember_burst(a);
            tx.remember_burst(b);
            tx.report(TxReport { code: TxCode::BurstAck, tick: None, channel: 9 });
            assert_eq!(tx.unacked.len(), 2, "invalid channel must not consume a burst");
            if channels == 2 {
                // Channel 0 may finish B while channel 1 has not finished A.
                for _ in 0..2 {
                    tx.report(TxReport { code: TxCode::BurstAck, tick: None, channel: 0 });
                }
                assert_eq!(tx.unacked.len(), 2);
            }
            let channel = channels - 1;
            for _ in 0..2 {
                tx.report(TxReport { code: TxCode::TimeError, tick: None, channel });
            }
            let errors = events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
            assert_eq!(errors.len(), 1, "repeated reports are deduplicated for the first pending target: {errors:?}");
            assert_eq!(errors[0].payload["target"], serde_json::to_value(a).unwrap());
            assert_eq!(tx.unacked.len(), 2, "TimeErrors do not retire their report correlations");
            assert!(tx.unacked.iter().all(|pending| pending.channels.contains(&channel)));
            assert_eq!(lock(&core.rec).device_async.iter().filter(|row| row["code"] == "TimeError").count(), 2, "both raw reports remain recorded");
        }
    }

    #[test]
    fn ur_28_time_error_after_abandoned_burst_does_not_claim_the_next_burst() {
        let (core, _, _, events) = super::super::test_support::rig();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        let a = TimePoint::new(clock.domain, 10_000);
        let b = TimePoint::new(clock.domain, 20_000);
        tx.remember_burst(a);
        tx.open = Some(super::Open { held: held(&clock, 10_000, 1, LatePolicy::DropAndFlag), next: 10_001, first: false });
        assert!(tx.abandon(false, 0, false), "A was not abandoned");
        assert!(tx.open.is_none());
        tx.remember_burst(b);

        tx.report(TxReport { code: TxCode::TimeError, tick: Some(clock.instant(25_000)), channel: 0 });
        assert!(time_errors(&events).is_empty(), "A's late report was assigned to B");
        assert_eq!(tx.unacked.iter().map(|pending| pending.target).collect::<Vec<_>>(), [a, b], "the ambiguous report claimed neither correlation");
        assert_eq!(lock(&core.rec).device_async.iter().filter(|row| row["code"] == "TimeError").count(), 1, "the raw radio-current-time report is retained");
    }

    #[test]
    fn ur_28_streamer_generation_clears_late_report_deduplication() {
        let (core, device, _, events) = super::super::test_support::rig();
        device.tx_open(0, &[0]).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        let target = TimePoint::new(clock.domain, 100);
        tx.remember_burst(target);
        tx.report(TxReport { code: TxCode::TimeError, tick: Some(clock.instant(101)), channel: 0 });
        assert!(tx.late_reported.contains(&target));
        assert_eq!(time_errors(&events).len(), 1, "the pending target was reported late");

        lock(&core.streams).record_open(stream_key(Dir::Tx, 0), Some(Channels::first(1)));
        tx.sync_streamer();
        assert!(tx.unacked.is_empty(), "the previous generation's correlation remains");
        assert!(!tx.late_reported.contains(&target), "the previous generation's deduplication entry remains");
    }

    /// The TIME_ERROR events the rig has emitted since the last call.
    fn time_errors(events: &EventCollector) -> Vec<ezsdr_kernel::event::Event> {
        events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect()
    }

    #[test]
    fn ur_28_a_streamer_the_record_holds_keeps_its_correlations() {
        // UR-28, UR-36: the correlations of the bursts in flight belong to the streamer of their
        // generation. An open of the stream — the owner's reopen — replaces the queue and drops
        // them; while the record holds the same streamer, they stand.
        let (core, device, _, _) = super::super::test_support::rig();
        device.tx_open(0, &[0]).unwrap();
        let key = stream_key(Dir::Tx, 0);
        lock(&core.streams).record_open(key, Some(Channels::first(1)));
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        tx.remember_burst(TimePoint::new(clock.domain, 100));
        lock(&core.streams).record_open(key, Some(Channels::first(1)));
        tx.sync_streamer();
        assert!(tx.unacked.is_empty(), "a new streamer's queue starts empty");
        tx.remember_burst(TimePoint::new(clock.domain, 100));
        tx.sync_streamer();
        assert_eq!(tx.unacked.len(), 1, "the record holds the same streamer: {:?}", device.calls());
    }

    #[test]
    fn ur_28_delayed_time_error_uses_its_original_clock() {
        let (core, device, _, events) = super::super::test_support::rig();
        device.tx_open(0, &[0, 1]).unwrap();
        lock(&core.streams).record_open(stream_key(Dir::Tx, 0), Some(Channels::first(2)));
        let old = core.register(Dir::Tx, 200, 0).unwrap();
        let target = TimePoint::new(old.domain, 10_000);
        core.clocks.end(old.domain, core.at(20_000_000)).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(old), Channels::first(2));
        tx.remember_burst(target);
        tx.cut = Some(20_000_000);
        tx.end_clock();
        tx.report(crate::device::TxReport { code: crate::device::TxCode::TimeError, tick: Some(old.instant(target.ticks_in(old.domain).unwrap()) + 200_000), channel: 0 });
        // This channel's report has no tick; it is still the same Kernel burst, so it is deduplicated.
        tx.report(crate::device::TxReport { code: crate::device::TxCode::TimeError, tick: None, channel: 1 });
        let errors: Vec<_> =
            events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|e| e.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert_eq!(errors.len(), 1, "only the uniquely attributable old burst reports: {errors:?}");
        assert_eq!(errors[0].payload["late_by_ns"], 1_000_000);
        assert_eq!(errors[0].payload["target"], serde_json::to_value(target).unwrap());
        assert_eq!(tx.unacked.len(), 1, "TimeErrors leave correlations pending");
        assert_eq!(lock(&core.rec).device_async.iter().filter(|row| row["code"] == "TimeError").count(), 2, "ambiguous reports remain visible in the raw log");
    }

    #[test]
    fn ur_25_the_line_lock_is_free_inside_every_transmit_device_call() {
        // UR-25 (design 33 D15): uhd-tx holds no line lock across its device calls — a
        // continuation's held sample, its own payload, the end of burst that closes the
        // device burst and the asynchronous report read — and the spy proves the sequence passed each one.
        let spy = Arc::new(crate::provider::test_support::LockSpy::new(FakeConfig { clock: crate::ClockMode::Manual, ..FakeConfig::default() }));
        let (core, time, _) = crate::provider::test_support::rig_on(spy.clone(), Vec::new(), 2_000);
        spy.watch(&core);
        spy.tx_open(0, &[0]).unwrap();
        let c0 = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(c0), Channels::first(1));
        // A burst continuing the device burst of the one before: its held sample goes first,
        // then its own payload; its last sample is held for the end of burst.
        tx.chain.open_device_burst(20_020, Some(Tail { samples: [[0.25, 0.0]; TX_CHANNELS_MAX], channels: 1 }));
        tx.command(TxCmd::Burst(held(&c0, 20_020, 10, LatePolicy::DropAndFlag)));
        time.advance_to(core.at(3_600_000)).unwrap();
        assert!(tx.step());
        // The clock's end at its cut: ends the open device burst (with its held sample). The
        // reports are read after it, so that the held sample's end of burst is the cut's.
        tx.command(TxCmd::Burst(held(&c0, 20_100, 10, LatePolicy::DropAndFlag)));
        tx.cut = Some(20_000_000);
        tx.end_clock();
        tx.reports(std::time::Duration::ZERO);
        assert_eq!(spy.seen("tx_open"), 1, "the test's own open");
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
        // The first burst's ACK retires that position; with only B pending, its TIME_ERROR is
        // attributable to B. The TIME_ERROR itself never consumes the BurstAck correlation.
        tx.report(crate::device::TxReport { code: crate::device::TxCode::BurstAck, tick: Some(clock.instant(30_000)), channel: 0 });
        assert_eq!(tx.unacked.len(), 1, "the ACK retired no report");
        assert_eq!(tx.unacked[0].target, second);
        tx.report(crate::device::TxReport { code: crate::device::TxCode::TimeError, tick: Some(clock.instant(41_000)), channel: 0 });
        assert_eq!(tx.unacked.len(), 1, "TIME_ERROR is not an acknowledgement");
        assert_eq!(tx.unacked[0].target, second);
        let late: Vec<_> = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert_eq!(late.len(), 1, "{late:?}");
        assert_eq!(late[0].payload["target"], serde_json::to_value(second).unwrap(), "the report names the second burst");
        assert_eq!(late[0].payload["outcome"], "late_at_device");
        tx.report(crate::device::TxReport { code: crate::device::TxCode::TimeError, tick: Some(clock.instant(41_100)), channel: 0 });
        assert!(
            events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR),
            "duplicate packet errors produce one burst event"
        );
        tx.report(crate::device::TxReport { code: crate::device::TxCode::BurstAck, tick: None, channel: 0 });
        assert!(tx.unacked.is_empty(), "the BurstAck retires B");
    }

    // ------------------------------------------------------------ 1b-4d-1b: ownership, reading, stop

    use crate::ClockMode;
    use crate::provider::test_support::LockSpy;

    /// The manual-clock `LockSpy` rig the ownership series runs on (UR-25).
    fn spy_rig() -> (Arc<Core>, Arc<LockSpy>) {
        let spy = Arc::new(LockSpy::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() }));
        let (core, _time, _) = crate::provider::test_support::rig_on(spy.clone(), Vec::new(), 2_000);
        (core, spy)
    }

    /// The transmit record, as the ownership tests read it.
    fn tx_record(core: &Core) -> (bool, bool, Option<u64>) {
        let streams = lock(&core.streams);
        let k = stream_key(Dir::Tx, 0);
        (streams.holder(k) != Holder::Free, streams.reading(k), streams.stream(k).configured)
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
        assert_eq!(
            inner.calls().iter().filter(|call| call.starts_with("tx_send")).map(String::as_str).collect::<Vec<_>>(),
            ["tx_send 1 n=2 at=- sob=true eob=true"],
            "only the open-time flush was sent: {:?}",
            inner.calls()
        );
    }

    #[test]
    fn ur_25_a_detached_control_does_not_hold_the_shutdown_s_read() {
        // UR-25, UR-16: a uhd-control the join bound left detached may never clear its mark;
        // the shutdown's ACK window still reads the reports.
        let (core, spy) = spy_rig();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        let claim = lock(&core.streams).claim_owner(stream_key(Dir::Tx, 0)).expect("the stream is free");
        std::mem::forget(claim);
        tx.shutdown();
        assert!(spy.seen("tx_async") >= 1, "the shutdown's read waited for a detached control");
        assert!(!tx_record(&core).1, "the shutdown's read was not taken back");
    }

    #[test]
    fn ur_26_stop_cancels_the_held_burst_record_only() {
        let (core, device, _, events) = super::super::test_support::rig();
        let current = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(current), Channels::first(1));
        tx.held.insert(20, held(&current, 20, 1, LatePolicy::DropAndFlag));
        lock(&core.held).extend([(current.domain, 20)]);

        tx.stop_now("UR-26: cancelled by Stop", ezsdr_radio::CancelReason::Stop);

        assert!(tx.held.is_empty());
        assert!(!lock(&core.held).contains(&(current.domain, 20)));
        let rejected = lock(&core.rec).rejected.clone();
        assert_eq!(rejected.len(), 1, "{rejected:?}");
        assert!(rejected.iter().all(|row| row["reason"] == "UR-26: cancelled by Stop"), "{rejected:?}");
        assert!(!device.calls().iter().any(|call| call.starts_with("tx_send ")));
        assert!(!events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().any(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED));
    }

    #[test]
    fn ur_23_a_clock_ending_at_its_cut_cancels_a_held_burst_and_releases_its_reservation() {
        let (core, device, _, events) = super::super::test_support::rig();
        let current = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(current), Channels::first(1));
        tx.held.insert(20, held(&current, 20, 1, LatePolicy::DropAndFlag));
        lock(&core.held).insert((current.domain, 20));
        tx.cut = Some(core.ticks(50_000_000));

        tx.end_clock();

        assert!(tx.held.is_empty());
        assert!(!lock(&core.held).contains(&(current.domain, 20)));
        let rejected = lock(&core.rec).rejected.clone();
        assert_eq!(rejected.len(), 1, "{rejected:?}");
        assert_eq!(rejected[0]["reason"], "UR-25: its transmit clock was never started");
        assert!(events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().any(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED));
        assert!(!device.calls().iter().any(|call| call.starts_with("tx_send ")));
    }

    #[test]
    fn ur_25_a_burst_on_a_clock_that_never_started_is_cancelled_and_releases_its_reservation() {
        let (core, device, _, events) = super::super::test_support::rig();
        let current = core.register(Dir::Tx, 200, 0).unwrap();
        let ended = core.register(Dir::Tx, 200, core.ticks(100_000_000)).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(current), Channels::first(1));
        lock(&core.held).insert((ended.domain, 30));

        // A burst booked on another clock than the owner's reaches the owner: the clock the booking saw ended at its origin.
        tx.command(TxCmd::Burst(held(&ended, 30, 1, LatePolicy::DropAndFlag)));

        assert!(tx.held.is_empty());
        assert!(!lock(&core.held).contains(&(ended.domain, 30)));
        let rejected = lock(&core.rec).rejected.clone();
        assert_eq!(rejected.len(), 1, "{rejected:?}");
        assert_eq!(rejected[0]["reason"], "UR-25: its transmit clock was never started");
        let rejected_events: Vec<_> =
            events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).collect();
        assert_eq!(rejected_events.len(), 1, "{rejected_events:?}");
        assert!(!device.calls().iter().any(|call| call.starts_with("tx_send ")));
    }

    // ------------------------------------------ bursts continued across a burst's end (UR-23)

    /// A-0: `manual_rig` with the device's clock held still (M3), A's burst at 40 000 with
    /// 30 000 samples driven in 13 steps of up to 2 500 samples, so its last sample is held
    /// back and `continues_at` is 70 000 (design note 37, T1–T5).
    fn at_a_held_tail(faults: Vec<crate::device::FakeFault>) -> (Arc<Core>, Arc<FakeDevice>, Arc<ezsdr_kernel::time::ManualTimeAuthority>, Tx, crate::provider::test_support::Frozen) {
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
        assert!(tx.open.is_none() && tx.chain.next() == Some(70_000) && tx.chain.tail().as_ref().is_some_and(|tail| tail.channels == 1), "A's last sample is not the held tail: {:?}", device.calls());
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
        device
            .calls()
            .iter()
            .filter(|call| call.starts_with("tx_send "))
            .map(|call| {
                let field = |key: &str| call.split_whitespace().find_map(|word| word.strip_prefix(key));
                (field("n=").unwrap().parse().unwrap(), field("sob=") == Some("true"), field("eob=") == Some("true"))
            })
            .collect()
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
        assert!(tx.open.is_none() && tx.chain.next().is_none() && tx.chain.tail().is_none());
        let mut whole: Vec<Option<[f32; 2]>> = (0..30_000).map(|_| Some([0.25, 0.0])).chain(second.iter().copied().map(Some)).collect();
        whole.push(Some([0.0, 0.0]));
        assert_eq!(device.transmitted(clock.instant(40_000), 31_001), whole, "{:?}", device.calls());
        let sends = sends(&device);
        assert_eq!(sends.iter().filter(|(_, sob, _)| *sob).count(), 1, "{sends:?}");
        assert_eq!(sends.iter().filter(|(_, _, eob)| *eob).count(), 1, "{sends:?}");
        assert_eq!(sends.iter().map(|(n, _, _)| n).sum::<usize>(), 31_001, "{sends:?}");
        assert_eq!(device.unended_bursts(), 0, "{sends:?}");
        assert!(
            lock(&core.rec).rejected.iter().all(|row| !row["reason"].as_str().unwrap_or("").starts_with("UR-21") && !row["reason"].as_str().unwrap_or("").starts_with("UR-22")),
            "{:?}",
            lock(&core.rec).rejected
        );
        tx.reports(Wall::ZERO);
        assert!(lock(&core.rec).device_async.iter().all(|row| row["code"] != "TimeError" && row["code"] != "Underflow"), "{:?}", lock(&core.rec).device_async);
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
        whole.push(Some([0.0, 0.0]));
        assert_eq!(device.transmitted(clock.instant(40_000), 30_001), whole, "{:?}", device.calls());
        let sends = sends(&device);
        // A's held last payload sample leads the two-sample close; its padding zero is at 70 000.
        assert_eq!(sends.last(), Some(&(2, false, true)), "{sends:?}");
        assert_eq!(sends.iter().filter(|(_, _, eob)| *eob).count(), 1, "{sends:?}");
        assert!(tx.open.is_none() && tx.chain.next().is_none() && tx.chain.tail().is_none());
        assert_eq!(device.unended_bursts(), 0, "{sends:?}");
        tx.reports(Wall::ZERO);
        assert!(lock(&core.rec).device_async.iter().all(|row| row["code"] != "Underflow"), "{:?}", lock(&core.rec).device_async);
    }

    #[test]
    fn ur_23_a_stop_ends_the_burst_within_the_in_flight_window() {
        // UR-23, RM-16: payload stops at the first SampleClock sample at or after `s + W`;
        // the two-zero EOB is separate. The fake's Session test reads the receipt instant.
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
        assert!(tx.open.is_none() && tx.chain.next().is_none() && tx.chain.tail().is_none());
        let window = core.ticks(core.description.timing.tx_in_flight_ns);
        let stop = core.now();
        let record = lock(&core.rec).bursts.last().cloned().expect("A's record");
        assert_eq!(record["end"], "stop", "{record}");
        let target = record["target"]["ticks"].as_i64().expect("the record's target");
        let end = clock.instant(target + record["samples"].as_i64().expect("the record's samples"));
        assert!(end <= stop + window, "the record ends {} ms after the stop", (end - stop) / core.ticks(1_000_000));
        // The device's own end is the record's plus the two zero samples in the EOB.
        device_at(&core, &time, &device, end + 2 * clock.n);
        tx.reports(Wall::ZERO);
        let ack = lock(&core.rec).device_async.iter().find(|row| row["code"] == "BurstAck").expect("the end-of-burst's acknowledgement")["tick"].as_i64().unwrap();
        assert_eq!(ack, end + clock.n, "{:?}", lock(&core.rec).device_async);
        assert!(lock(&core.rec).device_async.iter().all(|row| row["code"] != "TimeError" && row["code"] != "Underflow"), "{:?}", lock(&core.rec).device_async);
    }

    #[test]
    fn ur_23_a_burst_at_the_first_safe_sample_after_a_held_tail_close_is_played() {
        // T2: A closes with its held tail and one zero; B starts at the close's exclusive end
        // plus one sample, so the device sees A, zero, a gap, then B.
        let (core, device, time, mut tx, _frozen) = at_a_held_tail(Vec::new());
        let clock = tx.clock.expect("a transmit clock");
        let b = continuation(&clock, 70_002, 1_000);
        let second = b.samples[0].clone();
        lock(&core.held).insert((clock.domain, 70_002));
        tx.command(TxCmd::Burst(b));
        device_at(&core, &time, &device, core.ticks(62_000_000));
        assert!(tx.step(), "A's device burst is not ended at c");
        assert_eq!(tx.chain.front(), Some((clock.domain, 70_002)), "the safe sample follows the two-sample close");
        let before_unsafe = sends(&device);
        lock(&core.held).insert((clock.domain, 70_001));
        tx.command(TxCmd::Burst(continuation(&clock, 70_001, 1_000)));
        assert!(tx.step(), "the start inside A's EOB was not judged");
        assert_eq!(sends(&device), before_unsafe, "the sample before the safe front was not handed over");
        assert!(lock(&core.rec).rejected.iter().any(|row| row["reason"] == "UR-21: the late policy dropped the burst"));
        assert!(tx.step(), "B is not dispatched at the safe sample");
        device_at(&core, &time, &device, core.ticks(69_000_000) + 40_000);
        assert!(tx.step(), "B's held sample is not ended");
        assert!(tx.open.is_none() && tx.chain.next().is_none() && tx.chain.tail().is_none());
        let mut whole: Vec<Option<[f32; 2]>> = (0..30_000).map(|_| Some([0.25, 0.0])).collect();
        whole.extend([Some([0.0, 0.0]), None]);
        whole.extend(second.iter().copied().map(Some));
        assert_eq!(device.transmitted(clock.instant(40_000), 31_002), whole, "{:?}", device.calls());
        let sends = sends(&device);
        assert_eq!(sends.iter().filter(|(_, sob, _)| *sob).count(), 2, "{sends:?}");
        assert_eq!(sends.iter().filter(|(_, _, eob)| *eob).count(), 2, "{sends:?}");
        assert_eq!(device.unended_bursts(), 0, "{sends:?}");
        tx.reports(Wall::ZERO);
        assert!(lock(&core.rec).device_async.iter().all(|row| row["code"] != "TimeError" && row["code"] != "Underflow"), "{:?}", lock(&core.rec).device_async);
    }

    #[test]
    fn ur_23_a_continuation_whose_held_sample_is_not_taken_is_abandoned() {
        // T4: A's held sample's send ahead of B takes nothing, or fails: B is abandoned, and
        // the device burst ends with two zero samples after A's 29 999 accepted samples.
        for (fault, reason) in [(crate::device::FakeFault::StalledSend(13), "UR-22: a send did not complete"), (crate::device::FakeFault::FailSend(13), "fake: the send failed")] {
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
            assert_eq!(sent[30_000], Some([0.0, 0.0]), "{what}: second EOB sample {:?}", device.calls());
            let sends = sends(&device);
            assert_eq!(sends.iter().filter(|(_, _, eob)| *eob).count(), 1, "{what}: {sends:?}");
            assert_eq!(device.unended_bursts(), 0, "{what}: {sends:?}");
            assert!(lock(&core.rec).rejected.iter().any(|row| row["reason"].as_str().unwrap_or("").contains(reason)), "{what}: {:?} {sends:?}", lock(&core.rec).rejected);
            assert!(device.calls().iter().any(|call| call.contains("stalled") || call.contains("failed")), "{what}: the case was not reached: {:?}", device.calls());
        }
    }

    #[test]
    fn ur_23_a_continuation_s_first_send_that_takes_nothing_ends_the_device_burst() {
        // T5: B continues A's device burst; B's first payload send takes nothing. The device
        // burst is still open and ends with a two-zero EOB after A's last sample.
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
        assert_eq!(sent[30_001], Some([0.0, 0.0]), "the second EOB sample {:?}", device.calls());
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
        for (fault, cursor) in [(crate::device::FakeFault::ShortSend(0), 40_499), (crate::device::FakeFault::FailSend(0), 40_999)] {
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
            assert_eq!(tx.unacked.iter().map(|pending| pending.target).collect::<Vec<_>>(), [TimePoint::new(clock.domain, 40_000)], "{what}: the attempted first burst stays correlated");
            let sent = device.transmitted(clock.instant(40_000), 501);
            assert_eq!(sent[..499], a[..499].iter().copied().map(Some).collect::<Vec<_>>(), "{what}: {:?}", device.calls());
            assert_eq!(sent[499], Some([0.0, 0.0]), "{what}: after what A took: {:?}", device.calls());
            assert_eq!(sent[500], Some([0.0, 0.0]), "{what}: the second closing sample {:?}", device.calls());
            // The accepted (or conservatively attempted) cursor plus two is the first safe start.
            let safe = cursor + 3;
            lock(&core.held).insert((clock.domain, safe));
            tx.command(TxCmd::Burst(continuation(&clock, safe, 1_000)));
            device_at(&core, &time, &device, core.ticks(31_004_000));
            // The device gets past B's safe target while the owner's manual clock stays fixed.
            device.set_time(core.ticks(70_000_000));
            assert!(tx.step(), "{what}: B's timed send");
            device_at(&core, &time, &device, core.ticks(70_000_000));
            tx.reports(Wall::ZERO);
            assert_eq!(tx.unacked.iter().map(|pending| pending.target).collect::<Vec<_>>(), [TimePoint::new(clock.domain, safe)], "{what}: A's close ack retires A, not B");
            let late: Vec<_> =
                events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
            assert_eq!(late.len(), 1, "{what}: {:?}", lock(&core.rec).device_async);
            assert_eq!(late[0].payload["outcome"], "late_at_device", "{what}: {late:?}");
            assert_eq!(late[0].payload["target"]["ticks"], safe, "{what}: the report names B: {late:?}");
            assert!(
                lock(&core.rec)
                    .rejected
                    .iter()
                    .any(|row| row["reason"].as_str().unwrap_or("").contains("fake: the send failed") || row["reason"].as_str().unwrap_or("").contains("UR-22: a send did not complete")),
                "{what}: {:?}",
                lock(&core.rec).rejected
            );
        }
    }

    #[test]
    fn ur_22_a_send_cut_short_ends_the_device_burst_after_what_it_took() {
        // Review P, NB-1: a send that comes up short abandons the burst. The device burst is
        // closed with a two-zero EOB straight after the samples the
        // device took; the burst's held last sample is not sent there. The 6 000-sample case
        // cuts the final data buffer (the 4th device send); the last sample is held inside the
        // in-flight window (D3). A send that fails after taking half is ended the same way
        // (Review Q, NB-Q1). The steps are placed by hand on the manual clock, so the first
        // buffer's size does not depend on when a thread ran (design note 37).
        let cases = [
            (crate::device::FakeFault::ShortSend(0), 1_000usize),
            (crate::device::FakeFault::ShortSend(1), 3_000),
            (crate::device::FakeFault::ShortSend(3), 6_000),
            (crate::device::FakeFault::FailSend(0), 1_000),
        ];
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
                if tx.open.is_none() {
                    break;
                }
            }
            let sends = sends(&device);
            assert!(tx.open.is_none(), "{nth}: the device burst was not ended: {:?}", device.calls());
            let rejected = lock(&core.rec).rejected.clone();
            assert!(
                rejected.iter().any(|r| r["reason"].as_str().unwrap_or("").contains("UR-22: a send did not complete") || r["reason"].as_str().unwrap_or("").contains("fake: the send failed")),
                "{nth}: {rejected:?} {sends:?}"
            );
            let sent = device.transmitted(clock.instant(40_000), len + 2);
            let took = sent.iter().zip(&wave).take_while(|(s, w)| **s == Some(**w)).count();
            assert!(took < len - 1, "nth {nth}: {took} of {len} {sends:?}");
            assert_eq!(sent[took], Some([0.0, 0.0]), "nth {nth}: after {took} {sends:?}");
            assert_eq!(sent[took + 1], Some([0.0, 0.0]), "nth {nth}: the second closing zero");
            assert!(sent[took + 2..].iter().all(Option::is_none), "nth {nth}: {sends:?}");
            assert_eq!(sends.iter().filter(|s| s.2).count(), 1, "nth {nth}: {sends:?}");
            if len == 6_000 {
                assert!(took > 5_000, "nth {nth}: the cut was not in the final buffer: {took} {sends:?}");
            }
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
            assert_eq!(tx.chain.front(), Some((clock.domain, start + 20_000 + handed + 3)), "{fault:?}: recovery uses the accepted or conservatively attempted cursor");
            let sent = device.transmitted(clock.instant(start), 30_002);
            let expected: Vec<_> = (0..30_000).map(|i| Some(wave[i % wave.len()])).collect();
            assert_eq!(&sent[..30_000], expected, "{fault:?}: the accepted prefix keeps the original repeat phase");
            assert_eq!(sent[30_000], Some([0.0, 0.0]), "{fault:?}: EOB follows what the device accepted");
            assert_eq!(sent[30_001], Some([0.0, 0.0]), "{fault:?}: the second EOB zero follows the first");
            assert!(sent[30_002..].iter().all(Option::is_none), "{fault:?}: the unsent tiled suffix is never replayed");
            assert!(
                lock(&core.rec).rejected.iter().any(|row| row["reason"].as_str().is_some_and(|reason| reason.contains("send") || reason.contains("failed"))),
                "{fault:?}: {:?}",
                lock(&core.rec).rejected
            );
        }
    }

    #[test]
    fn ur_22_a_failed_eob_close_recovers_with_flush_before_the_next_timed_burst() {
        for fault in [FakeFault::ShortSend(1), FakeFault::StalledSend(1), FakeFault::FailSend(1)] {
            let faults = vec![fault.clone(), FakeFault::TxReport(Wall::ZERO, TxCode::BurstAck)];
            let config = FakeConfig { clock: ClockMode::Manual, faults, ..FakeConfig::default() };
            let (core, device, time, _events) = super::super::test_support::rig_with(config, Vec::new());
            let frozen = crate::provider::test_support::Frozen::of(&device);
            device_at(&core, &time, &device, 0);
            device.tx_open(0, &[0]).unwrap();
            device.apply(Dir::Tx, 0, &Settings { rate: Some(200e6), ..Settings::default() }, None).unwrap();
            let clock = core.register(Dir::Tx, 1, 0).unwrap();
            let sample = [[0.25, 0.0]];
            device.tx_send(0, &[&sample], Some(clock.instant(500_000)), true, false, Wall::ZERO).unwrap();
            let (_to_tx, cmds) = std::sync::mpsc::channel();
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            tx.chain.advance_front(clock.domain, 200);
            tx.remember_burst(TimePoint::new(clock.domain, 500_000));
            tx.chain.open_device_burst(500_001, Some(Tail { samples: [[0.25, 0.0]; TX_CHANNELS_MAX], channels: 1 }));

            tx.end_open(true);
            assert!(tx.chain.needs_reopen(), "{fault:?}: a partial or failed EOB did not invalidate the streamer");
            assert_eq!(tx.chain.next(), None, "{fault:?}: failed end_open does not retain the device cursor");
            assert!(tx.chain.tail().is_none(), "{fault:?}: failed end_open abandons the tail");
            assert_eq!(tx.chain.front(), Some((clock.domain, 200)), "{fault:?}: failed close advanced the safe front");
            assert!(lock(&core.rec).rejected.iter().any(|row| row["call"] == "tx_close"), "{fault:?}: {:?}", lock(&core.rec).rejected);
            assert_eq!(tx.unacked.len(), 1, "{fault:?}: the old experiment burst stays correlated");

            txp_give(&mut tx, &core, txp_burst(&clock, 1_000_000, 1, false, LatePolicy::DropAndFlag));
            // The recovery flush waits for its ack on the device's clock: it runs as a participant, and the clock is held again before it ends.
            let ((mut tx, passed), frozen) = crate::provider::test_support::released(frozen, &device.host(), move || {
                let passed = tx.pass();
                (tx, passed)
            });
            assert!(passed, "{fault:?}: the owner did not reopen the uncertain streamer");
            let sends: Vec<_> = device.calls().into_iter().filter(|call| call.starts_with("tx_send")).collect();
            assert!(sends[2].contains("n=2 at=- sob=true eob=true"), "{fault:?}: recovery flush was not the first send after the failed close: {sends:?}");
            assert_eq!(sends.len(), 3, "{fault:?}: timed experiment data crossed streamer replacement: {sends:?}");
            assert!(!tx.chain.needs_reopen());
            assert!(tx.unacked.is_empty(), "{fault:?}: replacing the streamer retained old-generation correlations");

            // Both clocks, the Core's and the device's, are put at one instant, with the device's clock held, before the burst is dispatched.
            assert!(device.time_now().unwrap() <= TX_RECOVERY_RESUME, "the held clock stands before the resume instant");
            device_at(&core, &time, &device, TX_RECOVERY_RESUME);
            assert!(tx.pass(), "{fault:?}: the following timed burst was not dispatched");
            let sends: Vec<_> = device.calls().into_iter().filter(|call| call.starts_with("tx_send")).collect();
            assert_eq!(sends.len(), 4, "{fault:?}: {sends:?}");
            assert!(sends[3].contains("at=1000000 sob=true"), "{fault:?}: the timed burst did not follow the recovery flush: {sends:?}");
            drop(frozen);
        }
    }

    #[test]
    fn ur_28_ack_ticks_separate_a_lost_flush_ack_from_later_bursts() {
        let config = FakeConfig { clock: ClockMode::Manual, faults: vec![FakeFault::DropBurstAck], ..FakeConfig::default() };
        let (core, device, _time, events) = super::super::test_support::rig_with(config, Vec::new());
        device.tx_open(0, &[0]).unwrap();
        let result = core.tx_flush(0, &[0]);
        assert!(!result.acknowledged, "the suppressed flush ack was reported as received");

        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        tx.remember_burst(TimePoint::new(clock.domain, 20_000));
        tx.report(TxReport { code: TxCode::BurstAck, tick: Some(clock.instant(20_000)), channel: 0 });
        assert!(tx.unacked.is_empty(), "the experiment ack is not swallowed as the missing flush ack");

        let next = TimePoint::new(clock.domain, 30_000);
        tx.remember_burst(next);
        core.stash_tx_report(0, TxReport { code: TxCode::BurstAck, tick: Some(clock.instant(29_999)), channel: 0 });
        tx.reports(Wall::ZERO);
        assert_eq!(tx.unacked.iter().map(|pending| pending.target).collect::<Vec<_>>(), [next], "an earlier flush ack does not retire a later experiment correlation");
        assert!(events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).is_empty());
        assert_eq!(lock(&core.rec).device_async.iter().filter(|row| row["code"] == "BurstAck").count(), 2, "the late ack is still visible in the raw async record");
    }

    /// An instant after the device's time at a recovery flush's ack (a few device ticks past the failed send) and before the next burst's start.
    const TX_RECOVERY_RESUME: i64 = 600_000;

    #[test]
    fn ur_22_a_failed_one_sample_padded_eob_is_not_safe_until_flush_recovery() {
        let (core, device, time, _events, clock, mut tx, frozen) = txp_high_rate_rig(1, vec![FakeFault::ShortSend(0)], BTreeMap::new());
        let start = 500_000;
        txp_give(&mut tx, &core, txp_burst(&clock, start, 1, false, LatePolicy::DropAndFlag));
        assert!(tx.step(), "the one-sample payload did not reach the padded EOB send");
        assert!(tx.chain.needs_reopen());
        assert_eq!(tx.chain.front(), None, "an attempted two-sample EOB is not a safe front");
        let sends: Vec<_> = device.calls().into_iter().filter(|call| call.starts_with("tx_send")).collect();
        assert_eq!(sends.len(), 1, "the uncertain EOB is not followed by a close on the same streamer: {sends:?}");
        assert!(sends[0].contains("n=1 at=500000 sob=true eob=false"), "the short EOB transferred only the payload prefix: {sends:?}");
        assert!(lock(&core.rec).rejected.iter().any(|row| row["reason"].as_str().is_some_and(|reason| reason.contains("did not complete"))));
        assert!(lock(&core.rec).rejected.iter().any(|row| row["call"] == "tx_close"), "the failed EOB is identified as an uncertain close");

        device_at(&core, &time, &device, start + 3);
        // The recovery flush waits for its ack on the device's clock: it runs as a participant, and the clock is held again before it ends.
        let ((mut tx, recovered), frozen) = crate::provider::test_support::released(frozen, &device.host(), move || {
            let recovered = tx.pass();
            (tx, recovered)
        });
        assert!(recovered, "streamer replacement and recovery flush were not attempted");
        assert!(!tx.chain.needs_reopen());
        let flush_tick =
            lock(&core.rec).device_async.iter().find(|row| row["tx_flush"] == true && row["code"] == "BurstAck").and_then(|row| row["tick"].as_i64()).expect("the recovery flush ack was recorded");
        assert_eq!(tx.chain.front(), Some((clock.domain, clock.at_or_after(flush_tick) + 2)), "the recovered flush ack establishes the safe front after both zeros");
        let after_flush: Vec<_> = device.calls().into_iter().filter(|call| call.starts_with("tx_send")).collect();
        assert!(after_flush[1].contains("n=2 at=- sob=true eob=true"), "the replacement flush precedes any next timed burst: {after_flush:?}");

        // Both clocks, the Core's and the device's, are put at one instant before the next burst is booked and dispatched, with the device's clock held.
        assert!(device.time_now().unwrap() <= TX_RECOVERY_RESUME, "the held clock stands before the resume instant");
        device_at(&core, &time, &device, TX_RECOVERY_RESUME);
        txp_give(&mut tx, &core, txp_burst(&clock, 1_000_000, 1, false, LatePolicy::DropAndFlag));
        assert!(tx.pass(), "the burst after a recovered EOB was not sent");
        let sends: Vec<_> = device.calls().into_iter().filter(|call| call.starts_with("tx_send")).collect();
        assert_eq!(sends.len(), 3, "the next burst follows the flush: {sends:?}");
        assert!(sends[2].contains("at=1000000 sob=true"), "the first later data packet remains timed: {sends:?}");
        drop(frozen);
    }

    #[test]
    fn ur_23_abandoned_send_preserves_the_accepted_cursor() {
        let a_samples = vec![[0.25, 0.0]; 2_000];
        let (a_wave, a_bytes) = waveform_input("abandoned_repeat", &a_samples);
        let (b_wave, b_bytes) = waveform_input("after_abandon", &[[0.75, -0.25]]);
        for (fault, cursor) in [(crate::device::FakeFault::ShortSend(0), 40_500), (crate::device::FakeFault::FailSend(0), 41_000)] {
            let case = format!("{fault:?}");
            let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, faults: vec![fault], ..FakeConfig::default() };
            let device = Arc::new(FakeDevice::new(config));
            let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes.clone())), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes.clone()))]);
            let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
            let _frozen = crate::provider::test_support::Frozen::of(&device);
            device_at(&core, &time, &device, 0);
            device.tx_open(0, &[0]).unwrap();
            device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
            let (mut control, cmds) = booking_control(&core);
            let clock = tx_clock(&core);
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            book_burst(&mut control, &core, 1, clock, 40_000, &a_wave, true, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            device_at(&core, &time, &device, core.ticks(31_000_000));
            assert!(tx.step(), "{case}: A was not sent");
            let safe = cursor + 3;
            assert_eq!(tx.chain.front(), Some((clock.domain, safe)), "{case}: recovery starts after the samples accepted or conservatively counted");

            let calls = device.calls().len();
            book_burst(&mut control, &core, 2, clock, 40_499, &b_wave, false, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            assert!(tx.step(), "{case}: B inside A's attempted range was not judged");
            let late =
                events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect::<Vec<_>>();
            assert_eq!(late.len(), 1, "{case}: {late:?}");
            assert_eq!(late[0].payload["outcome"], "drop", "{case}");
            assert_eq!(late[0].payload["target"]["ticks"], 40_499, "{case}");
            assert_eq!(device.calls().len(), calls, "{case}: the late B reached the device");

            device_at(&core, &time, &device, core.ticks(31_002_000));
            book_burst(&mut control, &core, 3, clock, cursor + 1, &b_wave, false, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            assert!(tx.step(), "{case}: B one sample before the safe restart was not judged");
            let late =
                events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect::<Vec<_>>();
            assert_eq!(late.len(), 1, "{case}: {late:?}");
            assert_eq!(late[0].payload["outcome"], "drop", "{case}");
            assert_eq!(late[0].payload["target"]["ticks"], cursor + 1, "{case}");

            device_at(&core, &time, &device, core.ticks(31_004_000));
            book_burst(&mut control, &core, 4, clock, safe, &b_wave, false, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            assert!(
                tx.step(),
                "{case}: the safe restart was not admitted; held={:?} clock={:?} front={:?} reopen_needed={} calls={:?} rejected={:?}",
                tx.held.keys().collect::<Vec<_>>(),
                tx.clock,
                tx.chain.front(),
                tx.chain.needs_reopen(),
                device.calls(),
                lock(&core.rec).rejected
            );
            assert!(
                events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR),
                "{case}: the safe restart was late"
            );
            assert_eq!(device.transmitted(clock.instant(safe), 1), vec![Some([0.75, -0.25])], "{case}: the safe sample was sent");
        }
    }

    #[test]
    fn ur_23_a_continuation_tail_is_counted_at_recovery() {
        let a_samples = vec![[0.25, 0.0]; 6_000];
        let b_samples = vec![[0.75, -0.25]; 1_000];
        let (a_wave, a_bytes) = waveform_input("tail_a", &a_samples);
        let (b_wave, b_bytes) = waveform_input("tail_b", &b_samples);
        for (fault, cursor, tail_taken, payload_taken) in
            [(crate::device::FakeFault::ShortSend(4), 8_499, true, 499), (crate::device::FakeFault::FailSend(4), 8_999, true, 499), (crate::device::FakeFault::FailSend(3), 8_000, false, 0)]
        {
            let case = format!("{fault:?}");
            let config = FakeConfig { clock: ClockMode::Manual, command_queue: 64, faults: vec![fault], ..FakeConfig::default() };
            let device = Arc::new(FakeDevice::new(config));
            let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes.clone())), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes.clone()))]);
            let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
            let _frozen = crate::provider::test_support::Frozen::of(&device);
            device_at(&core, &time, &device, 0);
            device.tx_open(0, &[0]).unwrap();
            device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
            let (mut control, cmds) = booking_control(&core);
            let clock = tx_clock(&core);
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            book_burst(&mut control, &core, 1, clock, 2_000, &a_wave, false, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            for _ in 0..3 {
                assert!(tx.step(), "{case}: A's chunks");
            }
            assert!(tx.open.is_none() && tx.chain.next() == Some(8_000) && tx.chain.tail().as_ref().is_some_and(|tail| tail.channels == 1), "{case}: A's held tail");
            assert_eq!(sends(&device).len(), 3, "{case}: the tail must still be held");

            book_burst(&mut control, &core, 2, clock, 5_000, &b_wave, false, LatePolicy::SendAsapAndFlag);
            drain_tx(&mut tx);
            assert!(tx.step(), "{case}: B's late continuation decision");
            assert!(tx.held.contains_key(&8_000), "{case}: B did not move to A's end");
            assert!(tx.step(), "{case}: B did not send the tail and continue");
            let moved =
                events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect::<Vec<_>>();
            assert_eq!(moved.len(), 1, "{case}: {moved:?}");
            assert_eq!(moved[0].payload["outcome"], "send_asap", "{case}");
            let safe = cursor + 3;
            assert_eq!(tx.chain.front(), Some((clock.domain, safe)), "{case}: B's base cursor and recovery payload samples");
            assert!(tx.open.is_none(), "{case}: the incomplete continuation was resumed");
            let mut expected = vec![Some([0.25, 0.0]); 5_999];
            if tail_taken {
                expected.push(Some([0.25, 0.0]));
            }
            expected.extend(b_samples[..payload_taken].iter().copied().map(Some));
            expected.extend([Some([0.0, 0.0]), Some([0.0, 0.0])]);
            expected.resize((safe - 2_000) as usize, None);
            assert_eq!(device.transmitted(clock.instant(2_000), expected.len()), expected, "{case}: FakeDevice's accepted samples and padding may precede the conservative recovery cursor");

            let calls = device.calls().len();
            book_burst(&mut control, &core, 3, clock, safe - 1, &b_wave, false, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            assert!(tx.step(), "{case}: the sample before the safe restart was not judged");
            let late =
                events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect::<Vec<_>>();
            assert_eq!(late.len(), 1, "{case}: {late:?}");
            assert_eq!(late[0].payload["outcome"], "drop", "{case}");
            assert_eq!(device.calls().len(), calls, "{case}: the late restart reached the device");

            book_burst(&mut control, &core, 4, clock, safe, &b_wave, false, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            assert!(tx.step(), "{case}: the safe restart was not admitted");
            assert!(
                events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR),
                "{case}: the safe restart was late"
            );
            assert_eq!(device.transmitted(clock.instant(safe), 1), vec![Some([0.75, -0.25])], "{case}: the safe sample was sent");
        }
    }

    #[test]
    fn ur_26_shutdown_after_loss_cancels_held_bursts_record_only() {
        let (core, device, _, events) = super::super::test_support::rig();
        let current = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(current), Channels::first(1));
        tx.held.insert(20, held(&current, 20, 1, LatePolicy::DropAndFlag));
        lock(&core.held).extend([(current.domain, 20)]);
        core.device_lost("fake loss before the burst start");

        tx.shutdown();

        assert!(tx.held.is_empty());
        assert!(!lock(&core.held).contains(&(current.domain, 20)));
        let rejected = lock(&core.rec).rejected.clone();
        assert_eq!(rejected.len(), 1, "{rejected:?}");
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
    fn txp_rig(
        window_ns: Option<i64>,
        block_len: u32,
    ) -> (Arc<Core>, Arc<FakeDevice>, Arc<ezsdr_kernel::time::ManualTimeAuthority>, Arc<EventCollector>, Clock, Tx, crate::provider::test_support::Frozen) {
        txp_rig_rate(window_ns, block_len, 1e6, 200)
    }

    #[allow(clippy::type_complexity)]
    fn txp_rig_rate(
        window_ns: Option<i64>,
        block_len: u32,
        rate: f64,
        ticks_per_sample: i64,
    ) -> (Arc<Core>, Arc<FakeDevice>, Arc<ezsdr_kernel::time::ManualTimeAuthority>, Arc<EventCollector>, Clock, Tx, crate::provider::test_support::Frozen) {
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..FakeConfig::default() }));
        let mut description = super::super::test_support::default_description(block_len);
        if let Some(window) = window_ns {
            description.timing.tx_in_flight_ns = window;
        }
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
    fn txp_high_rate_rig(
        channels: usize,
        faults: Vec<FakeFault>,
        inputs: BTreeMap<ContentHash, Arc<[u8]>>,
    ) -> (Arc<Core>, Arc<FakeDevice>, Arc<ezsdr_kernel::time::ManualTimeAuthority>, Arc<EventCollector>, Clock, Tx, crate::provider::test_support::Frozen) {
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
        for _ in 0..100_000 {
            if !tx.step() {
                return;
            }
        }
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
    fn txp_ticks(samples: i64) -> i64 {
        samples * 200
    }

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
        assert_eq!(tx.chain.next(), Some(start + 30_000));
        assert!(tx.step(), "the adjacent burst continues on the device burst");
        assert_eq!(*sends(&device).last().unwrap(), (999, false, false));
        device_at(&core, &time, &device, clock.instant(start + 31_000 - 400_000));
        assert!(tx.step(), "the last held sample closes at the burst end");
        assert_eq!(*sends(&device).last().unwrap(), (2, false, true));

        let expected: Vec<_> = a.into_iter().take(30_000).chain(b).map(Some).chain([Some([0.0, 0.0])]).collect();
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
            let errors: Vec<_> =
                events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
            match outcome {
                Some("drop") => {
                    assert_eq!(errors.len(), 1, "{name}: {errors:?}");
                    assert_eq!(errors[0].payload["outcome"], "drop");
                    assert_eq!(errors[0].payload["target"]["ticks"], target);
                    assert_eq!(device.calls().len(), calls, "{name}: dropped payload reached the device");
                    assert!(tx.held.is_empty());
                    tx.command(TxCmd::Stop);
                    assert_eq!(device.transmitted(clock.instant(start), 20_002), a[..20_000].iter().copied().map(Some).chain([Some([0.0, 0.0]), Some([0.0, 0.0])]).collect::<Vec<_>>());
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
    fn ur_23_txs_cut_and_stop_do_not_send_past_a_high_rate_frontier() {
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_high_rate_rig(1, Vec::new(), BTreeMap::new());
        let start = 500_000;
        device_at(&core, &time, &device, clock.instant(start - 400_000));
        let repeat = txp_burst(&clock, start, 30_000, true, LatePolicy::DropAndFlag);
        let expected = repeat.samples[0].clone();
        txp_give(&mut tx, &core, repeat);
        assert!(tx.step());
        tx.cut = Some(clock.instant(start + 30_000));
        assert!(tx.step(), "the next send ends exactly at the cut");
        assert_eq!(*sends(&device).last().unwrap(), (10_000, false, false));
        assert_eq!(tx.open.as_ref().unwrap().next, start + 30_000);
        assert!(tx.step(), "the device burst is closed at the cut");
        assert_eq!(*sends(&device).last().unwrap(), (2, false, true));
        let a = device.transmitted(clock.instant(start), 30_002);
        assert_eq!(a[..30_000], expected.iter().take(30_000).copied().map(Some).collect::<Vec<_>>());
        assert_eq!(a[30_000..], [Some([0.0, 0.0]), Some([0.0, 0.0])], "the two-zero EOB closure is at the cut");

        let (core, device, time, _events, clock, mut tx, _frozen) = txp_high_rate_rig(1, Vec::new(), BTreeMap::new());
        device_at(&core, &time, &device, clock.instant(start - 400_000));
        let repeat = txp_burst(&clock, start, 30_000, true, LatePolicy::DropAndFlag);
        let expected: Vec<_> = repeat.samples[0][..20_000].iter().copied().map(Some).collect();
        txp_give(&mut tx, &core, repeat);
        assert!(tx.step());
        tx.command(TxCmd::Stop);
        assert_eq!(*sends(&device).last().unwrap(), (2, false, true), "Stop sends only the EOB closure after the handed chunk");
        assert_eq!(device.transmitted(clock.instant(start), 20_002), expected.into_iter().chain([Some([0.0, 0.0]), Some([0.0, 0.0])]).collect::<Vec<_>>());
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
            assert!(tx.chain.tail().is_some());
            let expected_sizes = if len < 20_000 { vec![len - 1] } else { vec![20_000, len - 20_001] };
            assert_eq!(sends(&device).iter().map(|send| send.0).collect::<Vec<_>>(), expected_sizes, "payload before the held tail for {len}");
            device_at(&core, &time, &device, clock.instant(start + len as i64 - 400_000));
            assert!(tx.step(), "{len}: the held tail ends at the final payload sample");
            assert_eq!(*sends(&device).last().unwrap(), (2, false, true));
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
        let waves: Vec<Vec<Iq>> = (0..2).map(|channel| (0..997).map(|i| [((channel * 997 + i + 1) as f32) / 2_000.0, -((channel * 997 + i) as f32) / 3_000.0]).collect()).collect();
        let mut burst = txp_burst(&clock, start, 997, true, LatePolicy::DropAndFlag);
        burst.samples = Arc::new(waves.clone());
        txp_give(&mut tx, &core, burst);
        assert!(tx.step());
        assert!(tx.step());
        assert_eq!(sends(&device), [(20_000, true, false), (20_000, false, false)]);
        device_at(&core, &time, &device, clock.instant(start + 40_000));
        let crate::device::RxRecv::Samples { samples, first_tick } = device.rx_recv(1, 40_000, Wall::from_millis(10), false) else { panic!("both channels loop back") };
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
            if !tx.step() {
                break;
            }
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
        assert!(tx.chain.tail().is_some());
        assert_eq!(tx.chain.next(), Some(10_300));
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
        assert_eq!(tx.chain.next(), Some(9_600));
        let calls = device.calls().len();
        assert!(!tx.step(), "B's first buffer, a continuation, waits for a whole buffer of room");
        assert_eq!(device.calls().len(), calls, "the wait reached the device (an entry that skips the rule)");
        assert!(tx.open.as_ref().is_some_and(|open| open.next == 9_600 && open.first));
        assert_eq!(tx.chain.next(), Some(9_600));
        assert!(tx.chain.tail().is_none());
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
        assert_eq!(sends(&device)[sent..], [(2, false, true)], "one two-zero end-of-burst and no more payload");
        assert!(tx.open.is_none() && tx.chain.next().is_none() && tx.held.is_empty());
        assert!(!lock(&core.held).contains(&(clock.domain, 9_600)), "the waiting burst's reservation is released");
        // With a held tail (M9's state): the tail goes out as the device burst's last sample, with the end-of-burst.
        let (core, device, _time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_give(&mut tx, &core, txp_burst(&clock, 3_000, 6_000, false, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        txp_give(&mut tx, &core, txp_burst(&clock, 9_000, 4_000, false, LatePolicy::DropAndFlag));
        assert!(!tx.step());
        let sent = sends(&device).len();
        tx.command(TxCmd::Stop);
        assert_eq!(sends(&device)[sent..], [(2, false, true)], "the held sample and one zero, with the end-of-burst");
        assert_eq!(device.transmitted(clock.instant(8_999), 2), vec![Some([6_000f32 / 100_000.0, 0.3]), Some([0.0, 0.0])]);
        assert!(tx.open.is_none() && tx.chain.tail().is_none() && tx.held.is_empty());
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
        assert_eq!(tx.chain.next(), Some(10_200));
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
        let late: Vec<_> = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
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
        assert!(tx.chain.tail().is_some() && tx.chain.next() == Some(9_000));
        txp_give(&mut tx, &core, txp_burst(&clock, 9_000, 4_000, false, LatePolicy::DropAndFlag));
        let calls = device.calls().len();
        assert!(!tx.step());
        assert_eq!(device.calls().len(), calls, "the tail went out ahead of a buffer that had to wait");
        assert!(tx.chain.tail().is_some());
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
        assert!(tx.chain.tail().is_some(), "the next burst's distinct per-channel tail is retained");
        tx.command(TxCmd::Stop);
        assert_eq!(*sends(&device).last().unwrap(), (2, false, true), "Stop ends on the next burst's held tail and one zero");

        device_at(&core, &time, &device, clock.instant(start + 6));
        let crate::device::RxRecv::Samples { samples, .. } = device.rx_recv(0, 6, Wall::from_millis(10), false) else { panic!("the loopback contains both channel tails") };
        assert_eq!(
            samples,
            [[a[0][0], a[0][1], a[0][2], b[0][0], b[0][1], [0.0, 0.0]].to_vec(), [a[1][0], a[1][1], a[1][2], b[1][0], b[1][1], [0.0, 0.0]].to_vec(),],
            "continuation and EOB preserve each channel's held sample"
        );
    }

    #[test]
    fn ur_23_m10_a_waiting_burst_whose_timeline_passed_is_abandoned() {
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_give(&mut tx, &core, txp_burst(&clock, 3_000, 30_000, true, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        device_at(&core, &time, &device, txp_ticks(9_001));
        assert!(tx.step());
        assert!(lock(&core.rec).rejected.iter().any(|row| row["reason"].as_str().unwrap_or("").contains("UR-22: the transmit timeline expired")));
        assert_eq!(*sends(&device).last().unwrap(), (2, false, true), "a two-zero timed end-of-burst, not a late payload");
        assert_eq!(device.calls().last().map(String::as_str), Some(format!("tx_send 0 n=2 at={} sob=false eob=true", clock.instant(8_000)).as_str()));
        assert_eq!(device.transmitted(clock.instant(8_000), 2), vec![None, None], "a starved device does not play the late close");
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
    fn ur_23_m14_a_cut_is_not_waited_past_or_crossed() {
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_give(&mut tx, &core, txp_burst(&clock, 3_000, 30_000, true, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        let cut = clock.instant(10_200);
        tx.cut = Some(cut);
        let calls = device.calls().len();
        assert!(!tx.step(), "2 200 samples to the cut, 2 000 of room: wait");
        assert_eq!(device.calls().len(), calls);
        device_at(&core, &time, &device, txp_ticks(500));
        assert!(tx.step());
        assert_eq!(*sends(&device).last().unwrap(), (2_200, false, false));
        assert_eq!(tx.open.as_ref().unwrap().next, 10_200, "the exclusive cut");
        assert!(tx.step(), "at the cut the device burst ends");
        assert_eq!(*sends(&device).last().unwrap(), (2, false, true), "a two-zero end-of-burst only");
        assert_eq!(device.transmitted(cut, 2), vec![Some([0.0, 0.0]), Some([0.0, 0.0])], "nothing of the burst at or past the cut");
        device_at(&core, &time, &device, cut);
        assert!(tx.step(), "the clock ends");
        assert!(tx.clock.is_none() && tx.at == 1);
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
        for to_tx in &senders {
            to_tx.send(TxCmd::Shutdown).unwrap();
        }
        for thread in threads {
            txp_join(thread);
        }
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
        assert!(
            events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::COMMAND_REJECTED && event.kind.as_str() != EventKind::DEVICE_LOST),
            "a refused priority is a row, not an event: {events:?}"
        );
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
        let idle: Vec<_> =
            snapshot.iter().enumerate().filter(|(i, rec)| rec.what == "tx_async" && snapshot[..*i].iter().filter(|r| r.what == "tx_send").count() >= 3).map(|(_, rec)| rec.host).collect();
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
        let owner = host
            .spawn("tx-stop", move || {
                crate::host::pause_next_wake(woken, released);
                tx.run();
            })
            .expect("a participant thread");
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
            if sends.len() > 3 {
                break sends;
            }
            assert!(std::time::Instant::now() < deadline, "the Stop was not acted on within 10 s: {:?}", spy.recs());
            std::thread::sleep(std::time::Duration::from_millis(1));
        };
        assert_eq!((sends[3].n, sends[3].eob), (2, true), "the Stop ends the device burst with two zeros before any more payload: {sends:?}");
        to_tx.send(TxCmd::Shutdown).unwrap();
        txp_join(owner);
        assert_eq!(spy.recs().iter().filter(|rec| rec.what == "tx_send").count(), 4, "no payload after the Stop");
    }
}
