//! uhd-rx: owns the receive streamer. It carries out the receive plan it shares with
//! uhd-control — each segment configured, started at its origin, its clock registered at its
//! first block and ended at its cut — and turns `rx_recv` results into Stream Contract blocks on
//! every attached link, gaps into flags and losses into events (UR-17…UR-20, UR-25, UR-26,
//! UR-29).

use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration as Wall, Instant};

use ezsdr_hostmem::{HOST_MEMORY, HostPool, write_cf32};
use ezsdr_kernel::contract::DataContractId;
use ezsdr_kernel::event::Severity;
use ezsdr_kernel::module_api::StopMode;
use ezsdr_kernel::stream::{BlockFlags, BlockHeader, ChannelMask, Direction, PublishOutcome, SampleBlock};
use ezsdr_kernel::time::{Rational, SampleClockHandle, TimePoint};
use ezsdr_radio::kinds;
use ezsdr_radio::payloads::{AlignmentErrorPayload, LateCommandPayload, RxOverflowCause};
use ezsdr_radio::timeline::{Channels, Config, StreamKey};
use serde_json::json;

use super::control::Planned;
use super::core::{Clock, Core, Holder, RxChain, RxDevice, SegmentKey, lock, stream_key};
use super::mode::DescriptionExt;
use crate::device::{Dir, Iq, RxRecv};
use crate::host::HostClock;
use crate::profile::{DELIVERY_ALLOWANCE_NS, DEVICE_LEAD_NS};

pub(crate) enum RxCmd {
    /// `Provider::stop`, which has booked the stream's end (UR-26).
    Shutdown(StopMode),
}

/// What a turn of uhd-rx's run loop leaves it to do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Turn {
    /// Return: `Provider::stop` has ended the stream.
    Exit,
    /// Wait a millisecond: nothing to begin or receive.
    Idle,
    /// Go on at once.
    Busy,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pending {
    Overrun,
    Sequence,
    Alignment,
    MissedStart,
}

struct Stream {
    clock: Clock,
    /// Its clock's handle until the clock is registered, at its first block (RM-25).
    handle: Option<SampleClockHandle>,
    config: Config,
    channels: Channels,
    expected: i64,
    /// The distinct reports since the last block, in arrival order (UR-17…UR-19).
    pending: Vec<Pending>,
    /// The timed start's instant: the segment's origin (UR-17).
    start: i64,
    /// Blocks before this root tick are the previous stream's, still in flight after its
    /// untimed stop, whatever this stream's origin (Review N, B1).
    not_before: i64,
}

impl Stream {
    fn new(clock: Clock, handle: SampleClockHandle, channels: Channels, not_before: Option<i64>) -> Stream {
        Stream {
            clock,
            handle: Some(handle),
            config: Config { channels, ratio: Rational::new(clock.n as u64, 1).expect("a positive ratio") },
            channels,
            expected: 0,
            pending: Vec::new(),
            start: clock.origin,
            not_before: not_before.unwrap_or(i64::MIN),
        }
    }

    /// Keeps each distinct report until the next block; repeats of one kind coalesce.
    fn report(&mut self, pending: Pending) {
        if !self.pending.contains(&pending) {
            self.pending.push(pending);
        }
    }
}

pub(crate) struct Rx {
    core: Arc<Core>,
    /// The stream's number, `k` (RM-29): every device call and line reference names it.
    k: usize,
    cmds: Receiver<RxCmd>,
    stream: Option<Stream>,
    /// The earliest origin of a planned segment not yet begun.
    from: i64,
    pool: HostPool,
    /// The device's host clock (plan 30 §4.3): the UR-29 silence monitor, the idle wait and
    /// the stall run on it; `started` is real, for the `host_delay_ms` diagnostic only.
    host: HostClock,
    started: Instant,
    last_samples: Wall,
    stall: Option<(Wall, Wall)>,
    first: bool,
    /// The host-clock instant of the first block, the stall's origin (plan 30 §4.3).
    stall_at: Option<Wall>,
    /// When the last untimed stop was issued: the next stream drops what precedes it.
    last_stop: Option<i64>,
}

const RECV_TIMEOUT: Wall = Wall::from_millis(100);

impl Rx {
    /// uhd-rx for stream `k`, with the stream `start` started at T0, its clock declared
    /// (UR-15).
    pub fn new(core: Arc<Core>, k: usize, cmds: Receiver<RxCmd>, first: Option<(SampleClockHandle, Channels, i64)>, stall: Option<(Wall, Wall)>) -> Rx {
        let channels = first.as_ref().map_or(Channels::first(0), |(_, channels, _)| *channels);
        let pool = HostPool::new(core.block_len * channels.count().max(1) as usize * 8);
        let stream = first.map(|(handle, channels, t0)| {
            let clock = Clock { domain: handle.id(), origin: t0, n: handle.root_ticks_per_tick().num() as i64 };
            Stream::new(clock, handle, channels, None)
        });
        let key = stream_key(Dir::Rx, k);
        if let Some(stream) = stream.as_ref() {
            let mut chain = core.rx_chain[k].lock();
            let mut streams = lock(&core.streams);
            let active = streams
                .planned(key)
                .into_iter()
                .find(|planned| planned.segment.origin == stream.clock.origin && planned.segment.config == stream.config)
                .map(|planned| (planned.segment.origin, planned.segment.config, planned.segment.by));
            let segment = SegmentKey { by: active.and_then(|active| active.2), origin: stream.clock.origin };
            if core.is_lost() {
                chain.lose();
            } else {
                chain.adopt_start(segment, stream.clock.origin);
            }
            streams.started(key, stream.clock.origin, stream.config);
        }
        let host = core.device.host();
        let now = host.now();
        Rx {
            from: stream.as_ref().map_or(i64::MIN, |stream| stream.clock.origin + 1),
            stream,
            core,
            k,
            cmds,
            pool,
            host,
            started: Instant::now(),
            last_samples: now,
            stall,
            first: true,
            stall_at: None,
            last_stop: None,
        }
    }

    /// Carries out every command that has arrived, and follows the plan.
    fn poll(&mut self) {
        loop {
            match self.cmds.try_recv() {
                Ok(cmd) => self.command(cmd),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.command(RxCmd::Shutdown(StopMode::Abort));
                    break;
                }
            }
        }
        self.follow();
    }

    /// The chain lock orders plan intent with receive completion.
    fn follow(&mut self) {
        let Some(stream) = self.stream.as_ref() else { return };
        let key = stream_key(Dir::Rx, self.k);
        let mut chain = self.core.rx_chain[self.k].lock();
        let streams = lock(&self.core.streams);
        refresh_intent(&mut chain, &streams, key, stream);
        let done = chain.cut().is_some_and(|cut| stream.clock.at_or_after(cut) <= stream.expected) && self.core.now() >= stream.clock.origin;
        let end_now = chain.end_now();
        drop(streams);
        drop(chain);
        if done || end_now {
            self.try_end();
        }
    }

    pub fn run(mut self) {
        loop {
            match self.turn() {
                Turn::Exit => return,
                Turn::Idle => self.host.sleep(Wall::from_millis(1)),
                Turn::Busy => {}
            }
        }
    }

    /// One pass of the run loop: the commands that arrived, then a segment begun or one
    /// receive call carried out.
    fn turn(&mut self) -> Turn {
        self.poll();
        let terminate = self.core.rx_chain[self.k].lock().terminate().is_some();
        if terminate && self.stream.is_none() {
            self.finish();
            return Turn::Exit;
        }
        if self.core.is_lost() {
            self.finish();
            return if terminate { Turn::Exit } else { Turn::Idle };
        }
        self.core.probe("rx_after_early_loss_check", None);
        if self.stream.is_none() {
            return if self.begin() { Turn::Busy } else { Turn::Idle };
        }
        if let Some((after, duration)) = self.stall {
            // Counted from the first block, in the host clock's time (plan 30 §4.3), so that
            // `after` is into the stream and a busy thread does not fire the stall early.
            if self.stall_at.is_some_and(|at| self.host.now().saturating_sub(at) >= after) {
                // UR-34's B5 builder: stop reading once, to provoke a real overflow.
                self.stall = None;
                self.core.timing(json!({ "what": "rx_stall", "ms": duration.as_millis() as u64 }));
                self.host.sleep(duration);
            }
        }
        // A cut booked during the wait cuts this block too: `samples` reads the plan (RM-16).
        let len = self.recv_len();
        let timeout = self.recv_timeout();
        if len == 0 || timeout < Wall::from_millis(1) {
            self.core.rx_chain[self.k].lock().request_end_now();
            self.try_end();
            return Turn::Idle;
        }
        let (one_packet, lost) = {
            let chain = self.core.rx_chain[self.k].lock();
            (chain.cut().is_some_and(|cut| self.core.now() >= cut), self.core.is_lost() || matches!(chain.device(), RxDevice::Lost))
        };
        if lost {
            self.finish();
            return if terminate { Turn::Exit } else { Turn::Idle };
        }
        let result = self.core.device.rx_recv(self.k, len, timeout, one_packet);
        self.receive(result);
        Turn::Busy
    }

    fn command(&mut self, cmd: RxCmd) {
        let RxCmd::Shutdown(mode) = cmd;
        self.core.rx_chain[self.k].lock().request_terminate(mode);
        if mode == StopMode::Abort {
            // RM-16: an abort ends the stream uhd-rx has at the first sample not yet delivered,
            // a segment still being delivered up to an earlier cut included.
            self.follow();
            if let Some(stream) = self.stream.as_mut() {
                self.core.rx_chain[self.k].lock().request_cut(stream.clock.instant(stream.expected));
            }
            self.core.rx_chain[self.k].lock().request_end_now();
            self.try_end();
        }
    }

    /// Stops the device at the segment's cut and ends the stream (UR-17, UR-26).
    pub(super) fn try_end(&mut self) {
        let Some(stream) = self.stream.as_ref() else { return };
        let clock = stream.clock;
        let core = self.core.clone();
        let mut chain = core.rx_chain[self.k].lock();
        let streams = lock(&self.core.streams);
        refresh_intent(&mut chain, &streams, stream_key(Dir::Rx, self.k), stream);
        drop(streams);
        if self.core.is_lost() || matches!(chain.device(), RxDevice::Lost) {
            chain.lose();
            self.finish_locked(&mut chain);
            return;
        }
        let cut = chain.cut().unwrap_or_else(|| {
            assert!(chain.end_now());
            clock.instant(self.stream.as_ref().expect("a stream").expected)
        });
        let active = matches!(chain.device(), RxDevice::Armed { .. } | RxDevice::Streaming);
        if active {
            let now = self.core.now();
            self.core.timing(json!({ "what": "rx_stop_untimed", "cut": cut, "at": now }));
            let result = self.core.device.rx_stop(self.k);
            match chain.stop_result(result) {
                Ok(()) => {
                    lock(&self.core.rec).applied.push(json!({ "key": "rx_stop", "at": self.core.at(cut), "issued": self.core.at(now) }));
                    self.last_stop = Some(self.core.now() + self.core.ticks(DEVICE_LEAD_NS)).or(self.last_stop);
                }
                Err(error) => self.core.device_failed(None, "stop", &error),
            }
        }
        if self.core.is_lost() && !matches!(chain.device(), RxDevice::Lost) {
            chain.lose();
        }
        self.core.probe("rx_before_finish", None);
        self.finish_locked(&mut chain);
    }

    fn finish(&mut self) {
        let core = self.core.clone();
        let mut chain = core.rx_chain[self.k].lock();
        if let Some(stream) = self.stream.as_ref() {
            let streams = lock(&core.streams);
            refresh_intent(&mut chain, &streams, stream_key(Dir::Rx, self.k), stream);
        }
        if self.core.is_lost() {
            chain.lose();
        }
        self.finish_locked(&mut chain);
    }

    fn finish_locked(&mut self, chain: &mut RxChain) {
        let stream = self.stream.take();
        let cut = chain.cut();
        chain.finish();
        if let Some(stream) = stream {
            let key = stream_key(Dir::Rx, self.k);
            let mut streams = lock(&self.core.streams);
            if let (None, Some(cut)) = (&stream.handle, cut) {
                if let Err(error) = self.core.clocks.end(stream.clock.domain, self.core.at(cut)) {
                    self.core.reject_note(json!({ "clock_not_ended": error.to_string() }));
                }
            }
            streams.finished(key);
            let by = streams.line(key).and_then(|line| line.segment(stream.clock.origin, stream.config)).and_then(|segment| segment.by);
            streams.prune(key, by);
        }
    }

    /// RM-25, UR-17: the next planned segment begins: the streamer reopened when its
    /// channel set changed, the direction configured with the configuration in effect at its
    /// origin and a timed start
    /// issued there. A configuration the device refuses halts the stream: uhd-rx books the
    /// refusal, and the plan it then reads keeps the next segment where it was (RM-25).
    pub(super) fn begin(&mut self) -> bool {
        let key = stream_key(Dir::Rx, self.k);
        let next: Option<Planned> = {
            let streams = lock(&self.core.streams);
            if streams.holder(key) != Holder::Free {
                // UR-25: another holder owns the streamer this turn; the plan is read again at
                // the next, and the cursor stands.
                return false;
            }
            streams.planned(key).into_iter().find(|planned| planned.segment.origin >= self.from)
        };
        let Some(next) = next else { return false };
        let origin = next.segment.origin;
        // The record decides what is opened and configured (UR-13, UR-36).
        let core = self.core.clone();
        match core.owner_configure(Dir::Rx, self.k, &next) {
            Ok(true) => {}
            Ok(false) => return true,
            Err((error, claim)) => {
                self.refuse_segment(&next, Some(claim), error);
                return true;
            }
        }
        self.from = origin.saturating_add(1);
        let channels = next.set();
        let segment = SegmentKey { by: next.segment.by, origin };
        let mut chain = self.core.rx_chain[self.k].lock();
        if self.core.is_lost() || chain.device() == RxDevice::Lost {
            drop(chain);
            self.finish();
            return true;
        }
        // RM-25, UR-25: the start's configure has run inside the start lead. When it left less than the declared
        // command lead before the planned origin the start still goes out at the origin, and that is reported.
        let completion = self.core.now();
        let min_lead = self.core.ticks(self.core.description.timing.min_timed_command_lead_ns);
        if let Some(left) = ezsdr_radio::timeline::lead_left(origin, completion, min_lead) {
            let payload = serde_json::to_value(ezsdr_radio::payloads::StartLeadShortPayload {
                requested: self.core.at(origin),
                configured: self.core.at(completion),
                lead_left_ns: self.core.ns(left),
                min_lead_ns: self.core.description.timing.min_timed_command_lead_ns,
            })
            .expect("a payload");
            self.core.emit(&self.core.stream_node(Dir::Rx, self.k), kinds::START_LEAD_SHORT, Severity::Warning, payload);
        }
        if let Err(error) = chain.begin_result(segment, origin, self.core.device.rx_start(self.k, origin)) {
            drop(chain);
            self.refuse_segment(&next, None, error);
            return true;
        }
        let stream = self.core.description.stream_node(&self.core.id, Dir::Rx, self.k);
        let declared = self.core.clocks.declare_sample_clock(stream, self.core.root, next.segment.config.ratio).map_err(|e| e.to_string());
        match declared {
            Ok(handle) => {
                let clock = Clock { domain: handle.id(), origin, n: next.n() };
                self.pool = HostPool::new(self.core.block_len * channels.count().max(1) as usize * 8);
                self.stream = Some(Stream::new(clock, handle, channels, self.last_stop.take()));
                let mut streams = lock(&self.core.streams);
                streams.started(key, origin, next.segment.config);
                drop(streams);
                drop(chain);
                self.last_samples = self.host.now();
                self.follow();
            }
            Err(error) => {
                let stop = self.core.device.rx_stop(self.k);
                if let Err(stop_error) = chain.stop_result(stop) {
                    self.core.device_failed(None, "stop", &stop_error);
                }
                chain.finish();
                drop(chain);
                self.refuse_segment(&next, None, crate::device::DeviceError { lost: false, message: format!("UR-25: {error}") });
            }
        }
        self.core.timing(json!({ "what": "rx_segment", "origin": origin, "channels": channels.count(), "at": self.core.now() }));
        true
    }

    /// The refusal names the `start_rx` Action whose configuration the device refused, as kind `command` (RM-22).
    fn refuse_segment(&mut self, next: &Planned, claim: Option<super::core::Claim>, error: crate::device::DeviceError) {
        let key = stream_key(Dir::Rx, self.k);
        let mut streams = lock(&self.core.streams);
        self.core.probe("refused_booking", Some(&streams));
        let refused = next.segment.by.is_some_and(|by| streams.refuse(&self.core, key, by));
        if let Some(claim) = claim {
            streams.give_back(claim);
        }
        drop(streams);
        if !refused {
            self.from = next.segment.origin.saturating_add(1);
        }
        self.core.device_failed(next.settings.as_ref().and_then(|settings| settings.action()), "command", &error);
    }

    /// How many samples to ask `rx_recv` for: while a cut is pending, no more than up to
    /// the cut, so that the last block ends at the cut and the stream is stopped about a
    /// delivery after it, whatever `block_len` (Review N, B1).
    fn recv_len(&self) -> usize {
        let block = self.core.block_len;
        let Some(stream) = self.stream.as_ref() else { return block };
        let chain = self.core.rx_chain[self.k].lock();
        let Some(cut) = chain.cut() else { return block };
        let left = stream.clock.at_or_after(cut) - stream.expected;
        left.clamp(1, block as i64) as usize
    }

    /// How long `rx_recv` may wait: while a cut is pending, no longer than to the cut plus
    /// `quiet_span`, so that a stream that has stopped yielding is ended soon after its
    /// cut and not at the next 100 ms timeout (UR-17; design-notes §11 F2); never under
    /// 1 ms (UHD truncates the timeout to whole milliseconds).
    fn recv_timeout(&self) -> Wall {
        let Some(stream) = self.stream.as_ref() else { return RECV_TIMEOUT };
        let chain = self.core.rx_chain[self.k].lock();
        let Some(cut) = chain.cut() else { return RECV_TIMEOUT };
        let left = i128::from(cut) + i128::from(quiet_span(&self.core, stream)) - i128::from(self.core.now());
        let ns = (left.max(0) * 1_000_000_000 / i128::from(self.core.mcr)).clamp(1_000_000, RECV_TIMEOUT.as_nanos() as i128);
        Wall::from_nanos(ns.min(u64::MAX as i128) as u64)
    }

    pub(super) fn receive(&mut self, result: RxRecv) {
        let now = self.core.now();
        if matches!(&result, RxRecv::Timeout) {
            self.timeout(now);
            return;
        }
        if matches!(&result, RxRecv::LateCommand) && self.core.rx_chain[self.k].lock().cut().is_some() {
            let cut = self.core.rx_chain[self.k].lock().cut().expect("the cut was checked");
            self.core.timing(json!({ "what": "rx_stop_late", "cut": cut, "at": now }));
            self.core.rx_chain[self.k].lock().request_end_now();
            self.try_end();
            return;
        }
        let Some(stream) = self.stream.as_mut() else { return };
        match result {
            RxRecv::Samples { first_tick, samples } => {
                self.last_samples = self.host.now();
                self.samples(first_tick, samples);
            }
            RxRecv::Overflow { out_of_sequence: false } => {
                stream.report(Pending::Overrun);
                self.core.stat("rx_overflows", 1);
            }
            RxRecv::Overflow { out_of_sequence: true } | RxRecv::BadPacket => {
                stream.report(Pending::Sequence);
                self.core.stat("rx_overflows", 1);
            }
            RxRecv::Alignment => stream.report(Pending::Alignment),
            RxRecv::LateCommand => {
                // UR-17: the device missed the timed start; the origin stays. The new start
                // waits the profile's start lead from now (spec 20, VF-6).
                let restart = match ezsdr_radio::timeline::lattice(now.saturating_add(self.core.ticks(ezsdr_radio::timeline::start_lead_ns(&self.core.description.timing))), stream.config.ratio) {
                    Ok(restart) => restart,
                    Err(error) => return self.core.command_rejected(None, "start", &format!("UR-17: {error}")),
                };
                let payload = serde_json::to_value(LateCommandPayload { key: None, requested: self.core.at(stream.start), applied: self.core.at(restart) }).expect("a payload");
                stream.report(Pending::MissedStart);
                self.core.emit(&self.core.stream_node(Dir::Rx, self.k), kinds::LATE_COMMAND, Severity::Warning, payload);
                self.core.timing(json!({ "what": "rx_restart", "requested": stream.start, "at": restart }));
                let result = {
                    let mut chain = self.core.rx_chain[self.k].lock();
                    if self.core.is_lost() || chain.device() == RxDevice::Lost { None } else { Some(chain.restart_result(restart, self.core.device.rx_start(self.k, restart))) }
                };
                match result {
                    None => self.try_end(),
                    Some(Err(error)) => self.core.device_failed(None, "start", &error),
                    Some(Ok(())) => {}
                }
            }
            RxRecv::Failed(error) if error.lost => self.core.device_lost(&error.message),
            RxRecv::Failed(error) => {
                self.core.stat("rx_errors", 1);
                self.core.timing(json!({ "what": "rx_error", "error": error.message }));
            }
            RxRecv::Timeout => unreachable!("timeouts are handled before borrowing the stream"),
        }
    }

    fn timeout(&mut self, now: i64) {
        let Some(stream) = self.stream.as_ref() else { return };
        let span = quiet_span(&self.core, stream);
        let quiet = self.host.now().saturating_sub(self.last_samples) >= Wall::from_nanos(self.core.ns(span).max(0) as u64);
        let cut = self.core.rx_chain[self.k].lock().cut();
        let ended = cut.is_some_and(|cut| quiet && i128::from(now) >= i128::from(cut) + i128::from(span));
        let lost = cut.is_none_or(|cut| now < cut) && i128::from(now) > i128::from(stream.start) + i128::from(self.core.mcr) && self.host.now().saturating_sub(self.last_samples) > Wall::from_secs(1);
        if ended {
            self.core.rx_chain[self.k].lock().request_end_now();
            self.try_end();
        } else if lost {
            self.core.device_lost("UR-29: the receive stream yielded nothing for 1 s");
        }
    }

    fn samples(&mut self, first_tick: i64, mut samples: Vec<Vec<Iq>>) {
        let clock = self.stream.as_ref().map(|stream| stream.clock).expect("a stream");
        let mut chain = self.core.rx_chain[self.k].lock();
        let segment = chain.segment().expect("an active RX stream has a chain segment");
        chain.first_sample(segment);
        let mut streams = lock(&self.core.streams);
        let key = stream_key(Dir::Rx, self.k);
        refresh_intent(&mut chain, &streams, key, self.stream.as_ref().expect("a stream"));
        let cut = chain.cut();
        let stream = self.stream.as_mut().expect("a stream");
        if first_tick < clock.origin || first_tick < stream.not_before {
            // The old stream's samples still in flight after its stop (Review M, N-3;
            // Review N, B1: after a late switch they can follow the new origin).
            self.core.stat("rx_before_origin", 1);
            drop(streams);
            drop(chain);
            return;
        }
        let offset = first_tick - clock.origin;
        if offset.rem_euclid(clock.n) != 0 {
            self.core.stat("rx_off_lattice", 1);
        }
        let mut k = (offset + clock.n / 2).div_euclid(clock.n);
        let mut len = samples.first().map_or(0, Vec::len) as i64;
        let mut ended = false;
        // The cut and what this block delivers are read and written together, so that a
        // `Stop` booked after them cuts after the block (RM-16's floor, `Item.delivered`).
        if let Some(cut_k) = cut.map(|cut| clock.at_or_after(cut)).filter(|cut_k| k + len >= *cut_k) {
            len = (cut_k - k).max(0);
            ended = true;
        }
        if k + len > stream.expected {
            let delivered = cut.map_or(k + len, |cut| (k + len).min(clock.at_or_after(cut)));
            streams.delivered(stream_key(Dir::Rx, self.k), clock.origin, stream.config, delivered);
        }
        drop(streams);
        drop(chain);
        if k < stream.expected {
            let trim = stream.expected - k;
            if trim >= len {
                self.core.stat("rx_overlapping", 1);
                if ended {
                    self.try_end();
                }
                return;
            }
            self.core.stat("rx_overlapping", 1);
            for channel in &mut samples {
                channel.drain(..trim as usize);
            }
            len -= trim;
            k = stream.expected;
        }
        if len > 0 {
            if let Some(handle) = stream.handle.take() {
                // RM-25, UR-17: the clock is registered at its first block.
                if let Err(error) = self.core.clocks.register_sample_clock(&handle, stream.clock.origin) {
                    self.core.reject_note(json!({ "clock_not_registered": error.to_string() }));
                }
            }
            let expected = stream.expected;
            let lost = (k > expected).then(|| (k - expected) as u64);
            let pending = std::mem::take(&mut stream.pending);
            let mut flags = BlockFlags::NONE;
            if lost.is_some() {
                // Each report adds its flag; a jump no report announced is a sequence
                // discontinuity (UR-18).
                flags = flags | BlockFlags::GAP_BEFORE;
                if pending.is_empty() {
                    flags = flags | BlockFlags::SEQ_DISCONTINUITY;
                }
                for cause in &pending {
                    flags = flags
                        | match cause {
                            Pending::Overrun => BlockFlags::RESTARTED,
                            Pending::MissedStart => BlockFlags::NONE,
                            Pending::Sequence => BlockFlags::SEQ_DISCONTINUITY,
                            Pending::Alignment => BlockFlags::ALIGNMENT,
                        };
                }
            }
            let channels = stream.channels.count().max(1) as usize;
            stream.expected = k + len;
            let at = TimePoint::new(clock.domain, expected);
            let lost_n = lost.unwrap_or(0);
            let reports: Vec<_> = pending
                .iter()
                .filter_map(|pending| match pending {
                    Pending::Overrun => Some(RxOverflowCause::Overrun),
                    Pending::Sequence => Some(RxOverflowCause::Sequence),
                    Pending::Alignment | Pending::MissedStart => None,
                })
                .collect();
            if let Some(payload) = ezsdr_radio::payloads::rx_overflow_payload(&reports, lost) {
                // UR-20: one hot-path event for the block's whole reported loss.
                if let Err(error) = self.core.events.emit(self.core.overflow(self.k), at, Severity::Warning, &payload.to_hot()) {
                    self.core.reject_note(json!({ "event_not_emitted": kinds::RX_OVERFLOW, "error": error.to_string() }));
                }
            }
            for pending in pending {
                match pending {
                    Pending::Overrun | Pending::Sequence => {}
                    Pending::Alignment => {
                        let payload = serde_json::to_value(AlignmentErrorPayload { lost: lost_n }).expect("a payload");
                        self.core.emit_at(&self.core.stream_node(Dir::Rx, self.k), kinds::ALIGNMENT_ERROR, Severity::Error, payload, at);
                    }
                    Pending::MissedStart => {}
                }
            }
            let n = len as usize;
            let bytes = self.pool.fill(channels * n * 8, |buffer| {
                for (c, channel) in samples.iter().take(channels).enumerate() {
                    for (i, sample) in channel.iter().take(n).enumerate() {
                        write_cf32(buffer, n, c, i, sample[0], sample[1]);
                    }
                }
            });
            let header = BlockHeader {
                first_sample_time: TimePoint::new(clock.domain, k),
                len: len as u32,
                channels: channels as u16,
                direction: Direction::Rx,
                valid: ChannelMask::full(channels as u16),
                flags,
                lost,
                contract: DataContractId::parse("ezsdr.stream.cf32").expect("cf32"),
            };
            match SampleBlock::new_host(header, HOST_MEMORY, bytes, 8) {
                Ok(block) => {
                    let block = Arc::new(block);
                    for link in self.core.links(self.k) {
                        if link.publish(block.clone()) != PublishOutcome::Accepted {
                            self.core.stat("link_drops_seen", 1);
                        }
                    }
                    self.core.stat("rx_blocks", 1);
                    self.core.stat("rx_samples", len);
                }
                Err(error) => self.core.timing(json!({ "what": "rx_block_refused", "error": error.to_string() })),
            }
            if self.first {
                self.first = false;
                // The stall's origin is the host clock's time; `host_delay_ms` is real
                // (plan 30 §4.3: the diagnostic stays wall time).
                self.stall_at = Some(self.host.now());
                self.core.timing(json!({
                    "what": "first_rx_block", "index": k, "host_delay_ms": self.started.elapsed().as_millis() as u64,
                }));
            }
        }
        if ended {
            self.try_end();
        }
    }
}

fn refresh_intent(chain: &mut RxChain, streams: &super::core::Streams, key: StreamKey, stream: &Stream) {
    let planned = streams.line(key).and_then(|line| line.segment(stream.clock.origin, stream.config));
    if let Some(cut) = planned.map_or(Some(0), |segment| segment.cut) {
        chain.request_cut(stream.clock.instant(cut.max(stream.expected)));
    }
    if matches!(streams.stop, Some((_, StopMode::Abort))) {
        chain.request_cut(stream.clock.instant(stream.expected));
    }
}

/// How long a stream past its cut must have been silent to be taken as ended: a block and
/// the delivery allowance, so that a slow link's last samples before the cut are waited
/// for (Review N, B2).
fn quiet_span(core: &Core, stream: &Stream) -> i64 {
    core.block_len as i64 * stream.clock.n + core.ticks(DELIVERY_ALLOWANCE_NS)
}

#[cfg(test)]
mod differential;

#[cfg(test)]
mod tests {
    use super::super::control::ColdConfig;
    use super::super::core::RxDevice;
    use super::*;
    use crate::device::Device;
    use ezsdr_kernel::module_api::Link;
    use ezsdr_kernel::stream::BackPressure;
    use ezsdr_kernel::stream::{ContinuityBuilder, DropCarry, GapCause};
    use ezsdr_radio::payloads::RxOverflowPayload;
    use ezsdr_radio::timeline::{Item, Kind, Line, Segment};
    use std::collections::BTreeMap;

    struct NoActions;
    impl ezsdr_kernel::module_api::ActionReceiver for NoActions {
        fn recv(&self) -> Option<ezsdr_kernel::module_api::Dispatched> {
            None
        }
    }

    fn link() -> Arc<dyn ezsdr_kernel::stream::DataLink> {
        let decl = ezsdr_kernel::stream::DataLinkDecl {
            id: ezsdr_kernel::id::DataLinkId::local(0),
            from: ezsdr_kernel::contract::PortRef { component: ezsdr_kernel::spec::Ident::parse("radio").unwrap(), port: ezsdr_kernel::spec::Ident::parse("rx").unwrap() },
            to: ezsdr_kernel::contract::PortRef { component: ezsdr_kernel::spec::Ident::parse("rec").unwrap(), port: ezsdr_kernel::spec::Ident::parse("in").unwrap() },
            contract: DataContractId::parse("ezsdr.stream.cf32").unwrap(),
            policy: BackPressure::DropOldest,
            capacity: 16,
        };
        ezsdr_link_host::HostLinkModule::new().create(&decl).unwrap()
    }

    /// uhd-rx with the stream T0 started at 0, `channels` at 1 MS/s on the 200 MHz root, and
    /// its timeline with this Module's terms.
    fn rx(core: &Arc<Core>, channels: usize) -> (Rx, Clock) {
        let (handle, clock) = rx_setup(core, channels);
        let set = Channels::first(channels as u32);
        core.device.rx_open(0, &(0..channels).collect::<Vec<usize>>()).unwrap();
        // The record of an open that arm made (UR-13).
        lock(&core.streams).record_open(stream_key(Dir::Rx, 0), Some(set));
        core.device.rx_start(0, clock.origin).unwrap();
        let (to_rx, cmds) = std::sync::mpsc::channel();
        std::mem::forget(to_rx);
        (Rx::new(core.clone(), 0, cmds, Some((handle, set, clock.origin)), None), clock)
    }

    /// An owner with no segment yet, its streamer not opened.
    fn rx_unopened(core: &Arc<Core>, channels: usize) -> (Rx, Clock) {
        let (_, clock) = rx_setup(core, channels);
        let (to_rx, cmds) = std::sync::mpsc::channel();
        std::mem::forget(to_rx);
        (Rx::new(core.clone(), 0, cmds, None, None), clock)
    }

    fn rx_setup(core: &Arc<Core>, channels: usize) -> (SampleClockHandle, Clock) {
        let stream = core.id.child("rx").unwrap();
        let handle = core.clocks.declare_sample_clock(stream, core.root, Rational::new(200, 1).unwrap()).unwrap();
        let clock = Clock { domain: handle.id(), origin: 0, n: 200 };
        let config = Config { channels: Channels::first(channels as u32), ratio: Rational::new(200, 1).unwrap() };
        lock(&core.streams).lines.insert(
            stream_key(Dir::Rx, 0),
            Line::new(ezsdr_radio::timeline::Stream {
                direction: Direction::Rx,
                origin: 0,
                config,
                lead: core.ticks(ezsdr_radio::timeline::start_lead_ns(&core.description.timing)),
                call: core.block_len as i64,
                allowance: core.ticks(DELIVERY_ALLOWANCE_NS),
            }),
        );
        (handle, clock)
    }

    /// The receive plan set to segments at 1 MS/s, one channel, each `(origin, cut, by)`,
    /// as bookings would have left it.
    fn plan(core: &Core, segments: &[(i64, Option<i64>, Option<u64>)]) {
        let config = Config { channels: Channels::first(1), ratio: Rational::new(200, 1).unwrap() };
        let mut streams = lock(&core.streams);
        let key = stream_key(Dir::Rx, 0);
        for by in segments.iter().filter_map(|(_, _, by)| *by) {
            streams.stream_mut(key).configs.insert(by, Arc::new(ColdConfig::new(BTreeMap::new())));
        }
        let mut line = streams.line(key).expect("the receive line").clone();
        line.plan = segments.iter().map(|&(origin, cut, by)| Segment { origin, cut, config, by }).collect();
        streams.lines.insert(key, line);
    }

    fn rx_with_plan(core: &Arc<Core>, segments: &[Segment]) -> (Rx, Clock) {
        rx_with_plan_on(core, 0, segments)
    }

    fn rx_with_plan_on(core: &Arc<Core>, k: usize, segments: &[Segment]) -> (Rx, Clock) {
        let first = segments[0];
        let handle = core.clocks.declare_sample_clock(core.stream_node(Dir::Rx, k), core.root, first.config.ratio).unwrap();
        let clock = Clock { domain: handle.id(), origin: first.origin, n: first.config.ratio.num() as i64 };
        let key = stream_key(Dir::Rx, k);
        {
            let mut streams = lock(&core.streams);
            let mut line = streams.line(key).unwrap().clone();
            line.plan = segments.to_vec();
            streams.lines.insert(key, line);
            for by in segments.iter().filter_map(|segment| segment.by) {
                streams.stream_mut(key).configs.insert(by, Arc::new(ColdConfig::new(core.description.defaults())));
            }
        }
        let channels = first.config.channels;
        core.device.rx_open(k, &channels.iter().map(|channel| channel as usize).collect::<Vec<_>>()).unwrap();
        lock(&core.streams).record_open(key, Some(channels));
        core.device.rx_start(k, first.origin).unwrap();
        let (to_rx, cmds) = std::sync::mpsc::channel();
        std::mem::forget(to_rx);
        (Rx::new(core.clone(), k, cmds, Some((handle, channels, first.origin)), None), clock)
    }

    /// Books a receive command as uhd-control does, its segment's configuration with it.
    fn book(core: &Core, e: i64, seq: u64, kind: Kind) {
        let key = stream_key(Dir::Rx, 0);
        let mut streams = lock(&core.streams);
        streams.stream_mut(key).configs.insert(seq, Arc::new(ColdConfig::new(BTreeMap::new())));
        assert!(streams.book(core, key, Item { e, seq, ready: 0, delivered: None, refused: false, kind }, None));
    }

    #[test]
    fn ur_24_an_abort_keeps_overflow_and_later_samples_off_the_link() {
        let output = link();
        let (core, _device, _time, _) = super::super::test_support::rig_with_links(vec![output.clone()]);
        let (mut rx, clock) = rx(&core, 2);

        rx.receive(RxRecv::Overflow { out_of_sequence: false });
        rx.command(RxCmd::Shutdown(StopMode::Abort));
        assert!(rx.stream.is_none(), "abort ends the stream at the first sample not delivered");
        rx.receive(samples(clock.instant(6_000), 6_000));

        assert!(output.receive().is_none(), "abort publishes no overflow/drain tail at or after its cut");
        assert_eq!(clock.origin, 0);
    }

    #[test]
    fn ur_24_fake_host_silence_delivers_no_block() {
        let device = crate::device::FakeDevice::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, faults: vec![crate::device::FakeFault::Silence(Wall::ZERO)], ..Default::default() });
        let _frozen = super::super::test_support::Frozen::of(&device);
        device.set_time(0);
        device.rx_open(0, &[0]).unwrap();
        device.rx_start(0, 0).unwrap();
        assert!(matches!(device.rx_recv(0, 2_000, Wall::ZERO, false), RxRecv::Timeout));
        device.set_time(400_000);
        assert!(matches!(device.rx_recv(0, 2_000, Wall::ZERO, false), RxRecv::Timeout), "no host block is delivered");
    }

    #[test]
    fn ur_24_a_cut_receive_uses_one_short_packet_and_preserves_successor_lead() {
        let spy = Arc::new(super::super::test_support::LockSpy::new(crate::device::FakeConfig {
            clock: crate::ClockMode::Manual,
            rx_latency: Wall::from_micros(6_500),
            rx_packet: Some(500),
            ..Default::default()
        }));
        let mut description = super::super::test_support::default_description(20_000);
        description.timing.rx_start_lead_ns = 3_000_000;
        let (core, time, _) = super::super::test_support::rig_on_description(spy.clone(), vec![link()], BTreeMap::new(), description);
        spy.watch(&core);
        spy.report_rx_packet_max(20_000);
        let frozen = super::super::test_support::Frozen::on(spy.host());
        spy.set_time(core.now());
        let (mut rx, clock) = rx(&core, 1);
        let now = clock.instant(46_000);
        let expected = 23_000;
        let cut_index = 43_000;
        let cut = clock.instant(cut_index);
        let call = core.block_len.max(spy.rx_packet_samples(0)) as i64;
        assert_eq!(call, 20_000);
        time.advance_to(core.at(now)).unwrap();
        spy.set_time(now);
        spy.apply(Dir::Rx, 0, &crate::device::Settings { rate: Some(1e6), freq: Some(1e9), ..Default::default() }).unwrap();
        spy.rx_start(0, now).unwrap();
        rx.stream.as_mut().unwrap().expected = expected;
        core.rx_chain[0].lock().request_cut(cut);
        let allowance = core.ticks(DELIVERY_ALLOWANCE_NS);
        let lead = core.ticks(core.description.timing.rx_start_lead_ns);
        let successor = cut + call * clock.n + allowance + lead;
        plan(&core, &[(0, Some(cut_index), None), (successor, None, Some(1))]);

        let gate = spy.stop_after("rx_recv");
        let (done, finished) = std::sync::mpsc::channel();
        let worker = core
            .device
            .host()
            .spawn("rx-short-packet-deadline", move || {
                rx.turn();
                let _ = done.send(rx);
            })
            .unwrap();
        // The worker is a participant before the clock is released, so the exact return time below does not depend on this thread.
        drop(frozen);
        gate.wait();
        let returned_at = spy.time_now().unwrap();
        assert_eq!(returned_at, now + core.ticks(7_000_000), "the one-packet receive returned after 500 samples and delivery latency");
        time.advance_to(core.at(returned_at)).unwrap();
        spy.set_time(returned_at);
        gate.release();
        let rx = finished.recv_timeout(Wall::from_secs(10)).expect("the owner completed its cut receive");
        worker.join().unwrap();

        assert!(rx.stream.is_none(), "the late packet is clipped at the cut and the owner stops");
        assert_eq!(spy.rx_requests(), [20_000], "the request is larger than the FakeDevice's actual 500-sample packets");
        assert!(successor - core.now() >= lead, "a repeated per-packet timeout consumed the successor's lead: successor={successor}, now={}", core.now());
    }

    #[test]
    fn ur_24_a_far_future_cut_keeps_pre_cut_receives_multi_packet() {
        let spy = Arc::new(super::super::test_support::LockSpy::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, rx_packet: Some(500), ..Default::default() }));
        let output = link();
        let (core, time, _) = super::super::test_support::rig_on_description(spy.clone(), vec![output.clone()], BTreeMap::new(), super::super::test_support::default_description(2_000));
        spy.watch(&core);
        let frozen = super::super::test_support::Frozen::on(spy.host());
        spy.set_time(core.now());
        let (mut rx, clock) = rx(&core, 1);
        let cut_at = core.ticks(40_000_000);
        plan(&core, &[(0, Some(clock.at_or_after(cut_at)), None)]);
        spy.rx_start(0, 0).unwrap();

        // Register before releasing the freeze; channel waits keep the owner busy between turns.
        let (turn, turns) = std::sync::mpsc::channel();
        let (first_done, first_turn) = std::sync::mpsc::channel();
        let (second_done, second_turn) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel::<()>();
        let worker = spy
            .host()
            .spawn("rx-far-future-cut", move || {
                turns.recv().unwrap();
                rx.turn();
                first_done.send(()).unwrap();
                turns.recv().unwrap();
                rx.turn();
                second_done.send(rx).unwrap();
                let _ = released.recv();
            })
            .unwrap();
        drop(frozen);

        turn.send(()).unwrap();
        first_turn.recv_timeout(Wall::from_secs(10)).expect("the pre-cut receive completed");
        assert_eq!(spy.rx_requests(), [2_000], "the booked cut does not shrink the ordinary request");
        assert_eq!(spy.rx_packet_modes(), [false], "a read that starts before the cut remains multi-packet");
        assert_eq!(received(&output).iter().map(|(_, _, len)| *len).collect::<Vec<_>>(), [2_000], "the pre-cut read returned four 500-sample packets");

        time.advance_to(core.at(cut_at)).unwrap();
        spy.set_time(cut_at);
        turn.send(()).unwrap();
        let _rx = second_turn.recv_timeout(Wall::from_secs(10)).expect("the post-cut receive completed");
        assert_eq!(spy.rx_requests(), [2_000, 2_000], "the request size is unchanged after the cut");
        assert_eq!(spy.rx_packet_modes(), [false, true], "only the read starting at/after the cut is one-packet");
        assert_eq!(received(&output).iter().map(|(_, _, len)| *len).collect::<Vec<_>>(), [500], "the post-cut read returns one device packet");
        drop(release);
        worker.join().unwrap();
    }

    #[test]
    fn rx_chain_abandon_unowned_records_cleanup_state() {
        let (core, device, _, _) = super::super::test_support::rig_with_links(vec![link()]);
        let (to_rx, cmds) = std::sync::mpsc::channel();
        std::mem::forget(to_rx);
        let mut rx = Rx::new(core.clone(), 0, cmds, None, None);
        core.rx_chain[0].lock().abandon_unowned();

        assert!(matches!(rx.turn(), Turn::Exit));

        assert_eq!(core.rx_chain[0].lock().device(), RxDevice::Uncertain);
        assert!(!device.calls().iter().any(|call| call.starts_with("rx_start") || call.starts_with("rx_recv")), "shutdown has no streamer to recover");
    }

    #[test]
    fn ur_24_nonloss_rx_stop_failure_leaves_the_chain_uncertain() {
        let spy = Arc::new(super::super::test_support::LockSpy::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, ..Default::default() }));
        let (core, time, events) = super::super::test_support::rig_on_description(spy.clone(), vec![link()], BTreeMap::new(), super::super::test_support::default_description(2_000));
        spy.watch(&core);
        let frozen = super::super::test_support::Frozen::on(spy.host());
        spy.set_time(core.now());
        let (mut rx, clock) = rx(&core, 1);
        let starts = spy.seen("rx_start");
        let stop_at = core.now();
        {
            let mut streams = lock(&core.streams);
            streams.stop = Some((stop_at, StopMode::Orderly));
        }
        core.rx_chain[0].lock().request_cut(stop_at);
        let flush_at = stop_at + core.block_len as i64 * clock.n;
        time.advance_to(core.at(flush_at)).unwrap();
        spy.set_time(flush_at);
        spy.refuse_rx_stop(crate::DeviceError::failed("test: scripted rx_stop refusal"));

        rx.try_end();
        drop(frozen);

        assert!(rx.stream.is_none());
        assert_eq!(spy.seen("rx_stop"), 1);
        assert_eq!(spy.seen("rx_start"), starts, "a failed stop starts no recovery");
        assert_eq!(spy.seen("rx_recv"), 0);
        assert_eq!(core.rx_chain[0].lock().device(), RxDevice::Uncertain, "a refused stop leaves the device fact uncertain");
        // The refusal is labelled as a stop (RM-22), not as an update.
        let refused: Vec<_> = events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == kinds::COMMAND_REJECTED).collect();
        assert_eq!(refused.len(), 1, "{refused:?}");
        assert_eq!(refused[0].payload["kind"], "stop");
    }

    #[test]
    fn ur_24_a_loss_during_rx_receive_finishes_without_recovery_calls() {
        let spy = Arc::new(super::super::test_support::LockSpy::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, ..Default::default() }));
        let (core, _time, events) = super::super::test_support::rig_on_description(spy.clone(), vec![link()], BTreeMap::new(), super::super::test_support::default_description(2_000));
        spy.watch(&core);
        let _frozen = super::super::test_support::Frozen::on(spy.host());
        spy.set_time(core.now());
        let (mut rx, _) = rx(&core, 1);

        rx.receive(RxRecv::Failed(crate::DeviceError { lost: true, message: "scripted loss during RX receive".to_owned() }));
        let calls_at_loss = spy.log().len();
        rx.try_end();

        assert_eq!(spy.log().len(), calls_at_loss, "finish makes no recovery call after loss: {:?}", spy.log());
        assert!(rx.stream.is_none());
        let chain = core.rx_chain[0].lock();
        assert_eq!(chain.device(), RxDevice::Lost);
        drop(chain);
        assert_eq!(events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).iter().filter(|event| event.kind.as_str() == ezsdr_kernel::event::EventKind::DEVICE_LOST).count(), 1);
    }

    #[test]
    fn ur_24_a_loss_published_before_rx_recv_skips_the_device_call() {
        let spy = Arc::new(super::super::test_support::LockSpy::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, ..Default::default() }));
        let (core, _time, events) = super::super::test_support::rig_on_description(spy.clone(), vec![link()], BTreeMap::new(), super::super::test_support::default_description(2_000));
        spy.watch(&core);
        let _frozen = super::super::test_support::Frozen::on(spy.host());

        let mut config = core.description.defaults();
        config.insert(super::super::core::key("radio.rx.c0.stream"), ezsdr_kernel::spec::Value::from(0));
        let (to_tx, _) = std::sync::mpsc::channel();
        let _control = super::super::control::Control::new(
            core.clone(),
            Arc::new(NoActions),
            vec![to_tx],
            config,
            false,
            super::super::control::Start { t0: 0, rx: vec![Some((Channels::first(1), 200)); core.description.stream_count(Dir::Rx)], tx: vec![None; core.description.stream_count(Dir::Tx)] },
        );
        let segment = Segment { origin: 0, cut: None, config: Config { channels: Channels::first(1), ratio: Rational::new(200, 1).unwrap() }, by: None };
        let (mut rx, _) = rx_with_plan(&core, &[segment]);

        let (entered, at_probe) = std::sync::mpsc::channel();
        let (resume, waiting) = std::sync::mpsc::channel();
        let waiting = std::sync::Mutex::new(waiting);
        let probe_core = core.clone();
        super::super::test_support::probe(&core, "rx_after_early_loss_check", move |_| {
            assert!(!probe_core.is_lost(), "RX paused after its early loss check");
            let _ = entered.send(());
            let _ = waiting.lock().unwrap().recv();
        });
        let (returned, turn_result) = std::sync::mpsc::channel();
        let owner = spy
            .host()
            .spawn("rx-loss-before-recv", move || {
                let turn = rx.turn();
                let _ = returned.send((rx, turn));
            })
            .unwrap();
        at_probe.recv_timeout(Wall::from_secs(10)).expect("RX passed its early loss check");

        core.device_lost("scripted loss after RX's early loss check");
        assert!(core.is_lost());
        assert_eq!(spy.seen("rx_recv"), 0, "RX has not called the device before resuming");
        resume.send(()).unwrap();
        let (rx, turn) = turn_result.recv_timeout(Wall::from_secs(10)).expect("RX finalized the lost chain");
        owner.join().unwrap();

        assert_eq!(turn, Turn::Idle);
        assert!(rx.stream.is_none());
        assert_eq!(spy.seen("rx_recv"), 0, "no receive call follows loss");
        let calls = spy.order();
        let lost = calls.iter().position(|call| *call == "mark_lost").expect("the device loss was published");
        assert!(!calls[lost + 1..].contains(&"rx_recv"), "no receive call follows loss: {calls:?}");
        let chain = core.rx_chain[0].lock();
        assert_eq!(chain.device(), RxDevice::Lost);
        assert_eq!(chain.segment(), None);
        drop(chain);
        assert_eq!(events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).iter().filter(|event| event.kind.as_str() == ezsdr_kernel::event::EventKind::DEVICE_LOST).count(), 1);
    }

    #[test]
    fn ur_29_a_lost_receive_owner_configure_releases_stream_lock_before_reporting() {
        let inner = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig::default()));
        let device: Arc<dyn Device> = Arc::new(super::super::test_support::ApplyFails::lost(inner.clone(), Dir::Rx, Some(0)));
        let (core, _time, events) = super::super::test_support::rig_on(device, vec![link()], 2_000);
        let (mut rx, _) = rx_unopened(&core, 1);
        plan(&core, &[(10_000, None, Some(1))]);

        assert!(rx.begin(), "the owner handled the lost configure: {:?}", inner.calls());
        assert!(core.is_lost());
        assert_eq!(lock(&core.streams).holder(stream_key(Dir::Rx, 0)), Holder::Free);
        assert!(!inner.calls().iter().any(|call| call.starts_with("rx_start")), "the lost owner began a segment: {:?}", inner.calls());
        assert!(
            events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().any(|event| event.kind.as_str() == ezsdr_kernel::event::EventKind::DEVICE_LOST)
        );
    }

    fn samples(first_tick: i64, n: usize) -> RxRecv {
        RxRecv::Samples { first_tick, samples: vec![vec![[0.0, 0.0]; n]] }
    }

    fn received(link: &Arc<dyn ezsdr_kernel::stream::DataLink>) -> Vec<(ClockDomainId, i64, u32)> {
        std::iter::from_fn(|| link.receive().map(|(b, _)| b))
            .map(|block| {
                let t = block.header().first_sample_time;
                (t.domain(), t.ticks_in(t.domain()).unwrap(), block.header().len) // own domain: each block's clock is returned beside its ticks
            })
            .collect()
    }

    use ezsdr_kernel::id::ClockDomainId;

    fn clocks(core: &Core) -> Vec<(i64, Option<i64>)> {
        core.clocks.sample_clock_records().iter().map(|record| (record.origin.ticks_in(record.root).unwrap(), record.ended_at.map(|end| end.ticks_in(record.root).unwrap()))).collect()
    }

    /// A layout with two receive front ends, so that stream 1 exists (RM-29).
    fn layout_description() -> ezsdr_radio::channels::Description {
        use ezsdr_kernel::spec::{Ident, Scalar, Value};
        let selector = std::collections::BTreeMap::from([
            (Ident::parse("rx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:B:ubx:0")])),
            (Ident::parse("tx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0")])),
        ]);
        let layout = ezsdr_radio::channels::from_selector(&selector).unwrap();
        ezsdr_radio::channels::Description::new(&layout, 2_000, &crate::profile::terms())
    }

    #[test]
    fn ur_19_a_later_receive_stream_s_overflow_names_its_node() {
        // UR-20, RM-29 (R19): stream 1's RX_OVERFLOW goes by stream 1's handle, from
        // `usrp/rx/s1`, with RM-24's bytes; stream 0's stay its own (the existing test).
        let link = link();
        let (core, _device, _time, events) = super::super::test_support::rig_with_links_on(1, vec![link.clone()], layout_description());
        let handle = core.clocks.declare_sample_clock(core.stream_node(Dir::Rx, 1), core.root, Rational::new(200, 1).unwrap()).unwrap();
        lock(&core.streams).lines.insert(
            stream_key(Dir::Rx, 1),
            Line::new(ezsdr_radio::timeline::Stream {
                direction: Direction::Rx,
                origin: 0,
                config: Config { channels: Channels::first(1), ratio: Rational::new(200, 1).unwrap() },
                lead: 0,
                call: 2_000,
                allowance: 0,
            }),
        );
        let (to_rx, cmds) = std::sync::mpsc::channel();
        std::mem::forget(to_rx);
        let mut rx = Rx::new(core.clone(), 1, cmds, Some((handle, Channels::first(1), 0)), None);
        let clock = rx.stream.as_ref().unwrap().clock;
        rx.receive(RxRecv::Overflow { out_of_sequence: false });
        rx.receive(samples(clock.instant(2), 2));
        let block = link.receive().expect("the block after the overflow").0;
        assert_eq!(block.header().flags, BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED);
        let got: Vec<_> =
            events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == kinds::RX_OVERFLOW).collect();
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].source, ezsdr_kernel::event::EventSource::Node { node: ezsdr_kernel::id::ResourceId::parse("usrp/rx/s1").unwrap() });
        assert_eq!(got[0].time, TimePoint::new(clock.domain, 0));
        assert_eq!(got[0].payload.as_array().map(Vec::len), Some(9), "RM-24's bytes: {}", got[0].payload);
        assert_eq!(RxOverflowPayload::from_payload(&got[0].payload).unwrap(), RxOverflowPayload { cause: RxOverflowCause::Overrun, lost: 2 });
    }

    #[test]
    fn ur_19_reports_before_one_block_keep_each_kind() {
        // A reported block gets one summary event; flags still retain every distinct report.
        for (reports, flags, cause, alignment) in [
            (Vec::new(), BlockFlags::GAP_BEFORE | BlockFlags::SEQ_DISCONTINUITY, None, false),
            (vec![RxRecv::Alignment], BlockFlags::GAP_BEFORE | BlockFlags::ALIGNMENT, None, true),
            (vec![RxRecv::Overflow { out_of_sequence: true }], BlockFlags::GAP_BEFORE | BlockFlags::SEQ_DISCONTINUITY, Some(RxOverflowCause::Sequence), false),
            (
                vec![RxRecv::Alignment, RxRecv::Overflow { out_of_sequence: true }, RxRecv::Alignment],
                BlockFlags::GAP_BEFORE | BlockFlags::SEQ_DISCONTINUITY | BlockFlags::ALIGNMENT,
                Some(RxOverflowCause::Sequence),
                true,
            ),
            (vec![RxRecv::Overflow { out_of_sequence: false }, RxRecv::Alignment], BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED | BlockFlags::ALIGNMENT, Some(RxOverflowCause::Overrun), true),
            (
                vec![RxRecv::Overflow { out_of_sequence: false }, RxRecv::Overflow { out_of_sequence: true }],
                BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED | BlockFlags::SEQ_DISCONTINUITY,
                Some(RxOverflowCause::Overrun),
                false,
            ),
            (vec![RxRecv::Overflow { out_of_sequence: false }, RxRecv::Overflow { out_of_sequence: false }], BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED, Some(RxOverflowCause::Overrun), false),
            (
                vec![RxRecv::Overflow { out_of_sequence: true }, RxRecv::Overflow { out_of_sequence: true }],
                BlockFlags::GAP_BEFORE | BlockFlags::SEQ_DISCONTINUITY,
                Some(RxOverflowCause::Sequence),
                false,
            ),
        ] {
            let link = link();
            let (core, _, _, events) = super::super::test_support::rig_with_links(vec![link.clone()]);
            let (mut rx, clock) = rx(&core, 2);
            for report in reports {
                rx.receive(report);
            }
            rx.receive(RxRecv::Samples { first_tick: clock.instant(2), samples: vec![vec![[0.0, 0.0]; 2]; 2] });
            let block = link.receive().expect("the block after the reports").0;
            assert_eq!((block.header().flags, block.header().lost), (flags, Some(2)));
            assert_eq!(block.header().valid, ezsdr_kernel::stream::ChannelMask::full(2), "a whole-stream gap");
            let got: Vec<_> = events
                .drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0))
                .into_iter()
                .filter(|e| [kinds::ALIGNMENT_ERROR, kinds::RX_OVERFLOW].contains(&e.kind.as_str()))
                .collect();
            for event in &got {
                assert_eq!(event.source, ezsdr_kernel::event::EventSource::Node { node: core.rx_id.clone() });
            }
            let overflow: Vec<_> = got.iter().filter(|event| event.kind.as_str() == kinds::RX_OVERFLOW).collect();
            assert_eq!(overflow.len(), if cause.is_some() { 1 } else { 0 }, "{got:?}");
            if let Some(cause) = cause {
                assert_eq!(overflow[0].time, TimePoint::new(clock.domain, 0));
                assert_eq!(overflow[0].payload.as_array().map(Vec::len), Some(9), "{}", overflow[0].payload);
                assert_eq!(RxOverflowPayload::from_payload(&overflow[0].payload).unwrap(), RxOverflowPayload { cause, lost: 2 });
            }
            let alignments: Vec<_> = got.iter().filter(|event| event.kind.as_str() == kinds::ALIGNMENT_ERROR).collect();
            assert_eq!(alignments.len(), if alignment { 1 } else { 0 }, "{got:?}");
            if alignment {
                assert_eq!(alignments[0].payload["lost"], 2);
            }
        }
    }

    #[test]
    fn ur_17_a_block_s_channels_are_planar_cf32() {
        // UR-17, SC-4: a block's bytes are the planar cf32 of its channels — channel `c`'s
        // samples at `[c · len, (c + 1) · len)`. Every sample carries its own value, so a
        // channel/index swap, a frozen index or a reversed order inside a channel moves one.
        let value = |channel: usize, index: usize| {
            let v = 10.0 * channel as f32 + index as f32 + 1.0;
            [v, -v]
        };
        let link = link();
        let (core, _, _, _) = super::super::test_support::rig_with_links(vec![link.clone()]);
        let (mut rx, clock) = rx(&core, 2);
        rx.receive(RxRecv::Samples { first_tick: clock.origin, samples: (0..2).map(|channel| (0..3).map(|index| value(channel, index)).collect()).collect() });
        let block = link.receive().expect("the block").0;
        let bytes = block.host_bytes().expect("a host block");
        assert_eq!((block.header().len, block.header().channels), (3, 2));
        for channel in 0..2 {
            for index in 0..3 {
                let [re, im] = value(channel, index);
                assert_eq!(ezsdr_hostmem::read_cf32(bytes, 3, channel, index), (re, im), "channel {channel}, sample {index}");
            }
        }
    }

    #[test]
    fn ur_17_the_missed_start_lead_is_measured_from_the_decision_s_now() {
        // UR-17: a missed start restarts at the first lattice instant at or after the instant
        // the late start is decided at plus `radio.timing.rx_start_lead_ns` — measured from
        // the decision's `now`, not from the origin the start missed.
        let link = link();
        let device = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, ..crate::device::FakeConfig::default() }));
        let mut description = super::super::test_support::default_description(2_000);
        description.timing.rx_start_lead_ns = 19_000_005;
        let (core, time, events) = super::super::test_support::rig_on_description(device, vec![link], BTreeMap::new(), description);
        let (mut rx, clock) = rx(&core, 1);
        time.advance_to(core.at(core.ticks(40_000_000))).unwrap();
        rx.receive(RxRecv::LateCommand);
        let lead = core.ticks(ezsdr_radio::timeline::start_lead_ns(&core.description.timing));
        let expected = ezsdr_radio::timeline::lattice(core.ticks(40_000_000) + lead, Rational::new(clock.n as u64, 1).unwrap()).unwrap();
        assert!(expected > clock.origin + lead, "the lead was measured from the origin, not the decision");
        let row = lock(&core.rec).timing.iter().find(|row| row["what"] == "rx_restart").expect("the restart row").clone();
        assert_eq!(row["at"].as_i64(), Some(expected), "{row}");
        let late: Vec<_> = events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == kinds::LATE_COMMAND).collect();
        assert_eq!(late.len(), 1, "{late:?}");
        let payload: LateCommandPayload = serde_json::from_value(late[0].payload.clone()).unwrap();
        assert_eq!(payload.applied.ticks_in(core.root).unwrap(), expected, "{payload:?}");
    }

    #[test]
    fn rehearsal_b5_overflow() {
        // UR-19, UR-20, RM-17 (the B5 overflow, design note 37): a stalled reader's overrun is
        // one report, and the block the device resumes with carries what the stall lost —
        // `GAP_BEFORE` with `RESTARTED` and that count — which the continuity derivation a Sink
        // makes from the same header reads back as one `OverflowRestart` gap whose `lost` is the
        // event's. The rig's clock is the test's, so no capture write-out and no host deadline is
        // in the verdict (the bench rehearsal's `tests/fake.rs` copy is gone; the hardware's is
        // `hw_b5_overflow`).
        let link = link();
        let (core, _device, _time, events) = super::super::test_support::rig_with_links(vec![link.clone()]);
        let (mut rx, clock) = rx(&core, 1);
        // The overrun, then the first block after it: the device skipped samples 0…4 999.
        rx.receive(RxRecv::Overflow { out_of_sequence: false });
        rx.receive(samples(clock.instant(5_000), 2_000));
        let block = link.receive().expect("the block that resumes after the overflow").0;
        let header = block.header().clone();
        assert_eq!(header.first_sample_time, TimePoint::new(clock.domain, 5_000));
        assert_eq!((header.len, header.flags, header.lost), (2_000, BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED, Some(5_000)));
        let got: Vec<_> = events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == kinds::RX_OVERFLOW).collect();
        assert_eq!(got.len(), 1, "one event for one report: {got:?}");
        let payload = RxOverflowPayload::from_payload(&got[0].payload).unwrap();
        assert_eq!(payload, RxOverflowPayload { cause: RxOverflowCause::Overrun, lost: 5_000 });
        let mut builder = ContinuityBuilder::new(clock.domain, 1, false);
        builder.push(&header, DropCarry::default()).expect("the resumed block follows the gap it names");
        let map = builder.finish(DropCarry::default());
        assert_eq!(map.gaps.len(), 1, "{:?}", map.gaps);
        let gap = &map.gaps[0];
        assert_eq!(gap.cause, GapCause::OverflowRestart {});
        assert_eq!(gap.lost, Some(payload.lost), "the gap's lost is the overflow event's");
        assert_eq!((gap.start.ticks_in(clock.domain).unwrap(), gap.len, gap.link_dropped), (0, 5_000, 0));
        assert_eq!(map.end, TimePoint::new(clock.domain, 7_000));
        assert_eq!((map.valid[0].len(), map.valid[0][0].start, map.valid[0][0].len), (1, TimePoint::new(clock.domain, 5_000), 2_000));
    }

    #[test]
    fn ur_17_a_missed_start_without_another_report_is_not_an_overflow() {
        let link = link();
        let (core, _, _, events) = super::super::test_support::rig_with_links(vec![link.clone()]);
        let (mut rx, clock) = rx(&core, 1);
        rx.receive(RxRecv::LateCommand);
        let restart = lock(&core.rec).timing.iter().find(|row| row["what"] == "rx_restart").unwrap()["at"].as_i64().unwrap();
        let lost = ((restart - clock.origin) / clock.n) as u64;
        rx.receive(samples(restart, 2));
        let block = link.receive().expect("the restarted samples arrived").0;
        assert!(lost > 0);
        assert_eq!(block.header().flags, BlockFlags::GAP_BEFORE);
        assert_eq!(block.header().lost, Some(lost));
        let events = events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        assert_eq!(events.iter().filter(|event| event.kind.as_str() == kinds::LATE_COMMAND).count(), 1);
        assert_eq!(events.iter().filter(|event| event.kind.as_str() == kinds::RX_OVERFLOW).count(), 0);
    }

    #[test]
    fn ur_17_a_missed_start_and_overrun_share_the_block_summary() {
        let link = link();
        let (core, _, _, events) = super::super::test_support::rig_with_links(vec![link.clone()]);
        let (mut rx, clock) = rx(&core, 1);
        rx.receive(RxRecv::LateCommand);
        let restart = lock(&core.rec).timing.iter().find(|row| row["what"] == "rx_restart").unwrap()["at"].as_i64().unwrap();
        let missed = ((restart - clock.origin) / clock.n) as u64;
        rx.receive(RxRecv::Overflow { out_of_sequence: false });
        let first = restart + core.ticks(50_000_000);
        rx.receive(samples(first, 2));

        let block = link.receive().expect("the samples after both gaps arrived").0;
        assert_eq!(block.header().flags, BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED);
        assert_eq!(block.header().lost, Some(missed + 50_000));
        let events = events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        assert_eq!(events.iter().filter(|event| event.kind.as_str() == kinds::LATE_COMMAND).count(), 1);
        let overflows: Vec<_> = events.iter().filter(|event| event.kind.as_str() == kinds::RX_OVERFLOW).collect();
        assert_eq!(overflows.len(), 1, "{events:?}");
        assert_eq!(overflows[0].time, TimePoint::new(clock.domain, 0));
        assert_eq!(RxOverflowPayload::from_payload(&overflows[0].payload).unwrap(), RxOverflowPayload { cause: RxOverflowCause::Overrun, lost: block.header().lost.unwrap() });
    }

    #[test]
    fn ur_18_a_report_on_a_gapless_block_is_consumed_without_an_event() {
        let link = link();
        let (core, _, _, events) = super::super::test_support::rig_with_links(vec![link.clone()]);
        let (mut rx, clock) = rx(&core, 1);
        rx.receive(RxRecv::Overflow { out_of_sequence: false });
        rx.receive(samples(clock.instant(0), 2));
        let no_gap = link.receive().expect("the gapless block arrived").0;
        assert_eq!((no_gap.header().flags, no_gap.header().lost), (BlockFlags::NONE, None));
        rx.receive(samples(clock.instant(4), 2));
        let jump = link.receive().expect("the later unreported jump arrived").0;
        assert_eq!((jump.header().flags, jump.header().lost), (BlockFlags::GAP_BEFORE | BlockFlags::SEQ_DISCONTINUITY, Some(2)));
        assert_eq!(events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().filter(|event| event.kind.as_str() == kinds::RX_OVERFLOW).count(), 0);
    }

    #[test]
    fn ur_29_a_loss_after_a_configure_starts_no_receive_segment() {
        // UR-29 (L1): the device is found lost while the owner's configuration is in
        // flight; the owner issues no timed start and registers no clock.
        let inner = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig::default()));
        let holder = Arc::new(super::super::test_support::HoldApply::new(inner.clone(), Dir::Rx, Some(0)));
        let (core, _time, _events) = super::super::test_support::rig_on(holder.clone(), vec![link()], 2_000);
        let (mut rx, _clock) = rx_unopened(&core, 1);
        plan(&core, &[(core.ticks(50_000_000), None, Some(1))]);
        let (reached, release) = holder.arm();
        let worker = core.clone();
        let running = std::thread::spawn(move || rx.begin());
        reached.recv_timeout(Wall::from_secs(10)).expect("the owner's configure");
        worker.device_lost("test: lost during the configure");
        release.send(()).unwrap();
        assert!(running.join().unwrap(), "the owner found its segment");
        assert!(!inner.calls().iter().any(|call| call.starts_with("rx_start")), "{:?}", inner.calls());
        assert!(core.clocks.sample_clock_records().is_empty(), "{:?}", core.clocks.sample_clock_records());
        let streams = lock(&core.streams);
        assert_eq!(streams.first_undelivered(stream_key(Dir::Rx, 0)), None);
    }

    #[test]
    fn ur_29_a_begin_rechecks_loss_after_acquiring_the_chain_before_rx_start() {
        let spy = Arc::new(super::super::test_support::LockSpy::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, ..Default::default() }));
        let (core, _, _) = super::super::test_support::rig_on_description(spy.clone(), vec![link()], BTreeMap::new(), super::super::test_support::default_description(2_000));
        spy.watch(&core);
        let _frozen = super::super::test_support::Frozen::on(spy.host());
        let (mut rx, _) = rx_unopened(&core, 1);
        plan(&core, &[(0, None, Some(1))]);
        lock(&core.streams).stream_mut(stream_key(Dir::Rx, 0)).configs.insert(1, Arc::new(ColdConfig::new(core.description.defaults())));
        let next = lock(&core.streams).planned(stream_key(Dir::Rx, 0))[0].clone();
        assert!(core.owner_configure(Dir::Rx, 0, &next).unwrap(), "the segment is configured before the lock race");
        let chain = core.rx_chain[0].lock();
        let (done, returned) = std::sync::mpsc::channel();
        let owner = core
            .device
            .host()
            .spawn("rx-begin-loss-recheck", move || {
                let began = rx.begin();
                let _ = done.send((began, rx));
            })
            .unwrap();
        let deadline = Instant::now() + Wall::from_secs(10);
        while core.rx_chain[0].waiters() == 0 && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(core.rx_chain[0].waiters() > 0, "begin passed its pre-lock loss check and reached the chain lock");

        core.device_lost("test: loss after begin's pre-lock check");
        let starts_at_loss = spy.seen("rx_start");
        drop(chain);
        let (began, _rx) = returned.recv_timeout(Wall::from_secs(10)).expect("begin handled loss after acquiring the chain");
        owner.join().unwrap();

        assert!(began);
        assert_eq!(spy.seen("rx_start"), starts_at_loss, "begin made no device start call after loss");
        let calls = spy.log();
        let marked = calls.iter().position(|call| call == "mark_lost").expect("loss was published");
        assert!(!calls[marked + 1..].iter().any(|call| call.starts_with("rx_start") || call.starts_with("rx_close")), "no receive device call follows loss: {calls:?}");
        let chain = core.rx_chain[0].lock();
        assert_eq!(chain.device(), RxDevice::Lost);
        assert_eq!(chain.segment(), None);
        assert!(core.clocks.sample_clock_records().is_empty());
        assert_eq!(lock(&core.streams).first_undelivered(stream_key(Dir::Rx, 0)), None);
    }

    #[test]
    fn ur_26_a_receive_start_queued_by_the_stop_ends_its_owner_without_detaching() {
        // UR-26 (S16a): the owner does not re-read `Provider::stop` after its configuration;
        // the start it queued at its origin is issued, the plan cuts the segment there — no
        // block, and an untimed stop once the origin has passed —, and the owner leaves
        // through its own turn, so a stop joins it inside the 1 s bound instead of leaving it
        // detached.
        let inner = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig::default()));
        let holder = Arc::new(super::super::test_support::HoldApply::new(inner.clone(), Dir::Rx, Some(0)));
        let link = link();
        let (core, time, _events) = super::super::test_support::rig_on(holder.clone(), vec![link.clone()], 2_000);
        let origin = core.ticks(10_000_000);
        // The stream's first segment is cut with no sample, so that the plan's one segment is the
        // `start_rx`'s, at `origin`.
        lock(&core.streams).lines.insert(
            stream_key(Dir::Rx, 0),
            Line::new(ezsdr_radio::timeline::Stream {
                direction: Direction::Rx,
                origin: 0,
                config: Config { channels: Channels::first(1), ratio: Rational::new(200, 1).unwrap() },
                lead: 0,
                call: 2_000,
                allowance: 0,
            }),
        );
        book(&core, 0, 1, Kind::Stop);
        book(&core, origin, 2, Kind::Start);
        let (to_rx, cmds) = std::sync::mpsc::channel();
        let mut rx = Rx::new(core.clone(), 0, cmds, None, None);
        let (reached, release) = holder.arm();
        let (done, finished) = std::sync::mpsc::channel();
        let running = std::thread::spawn(move || {
            let began = rx.begin();
            let _ = done.send((began, rx));
        });
        reached.recv_timeout(Wall::from_secs(10)).expect("the owner's configure");
        {
            // As `Provider::stop(orderly)` books it, before the segment's origin.
            let mut streams = lock(&core.streams);
            let at = core.now();
            streams.stop = Some((at, StopMode::Orderly));
            streams.end(&core, at, false, false);
        }
        time.advance_to(core.at(origin)).unwrap();
        release.send(()).unwrap();
        let (began, mut rx) = finished.recv_timeout(Wall::from_secs(1)).expect("the owner's begin returned within the join bound");
        assert!(began, "the owner found its segment");
        assert!(inner.calls().iter().any(|call| call.starts_with("rx_start 0 ")), "the queued start is issued: {:?}", inner.calls());
        assert!(link.receive().is_none(), "nothing is delivered at the origin: {:?}", inner.calls());
        let stop = lock(&core.rec).timing.iter().find(|row| row["what"] == "rx_stop_untimed").cloned().expect("the untimed stop at the origin");
        assert_eq!((stop["cut"].as_i64().unwrap(), stop["at"].as_i64().unwrap()), (origin, origin), "{stop}");
        to_rx.send(RxCmd::Shutdown(StopMode::Orderly)).unwrap();
        assert_eq!(rx.turn(), Turn::Exit, "the owner ends through its own loop");
        {
            let streams = lock(&core.streams);
            assert_eq!(streams.first_undelivered(stream_key(Dir::Rx, 0)), None, "the ended segment leaves no delivered floor");
        }
        running.join().expect("the owner's begin thread");
    }

    #[test]
    fn ur_26_a_stream_s_delivered_is_cleared_alone() {
        // RM-16 (S12c): a stream's `finish` clears its own delivered floor; the other
        // stream's stands for the end that comes later.
        let (core, device, _time, _events) = super::super::test_support::rig_with_links_on(1, vec![link()], layout_description());
        let config = Config { channels: Channels::first(1), ratio: Rational::new(200, 1).unwrap() };
        {
            let mut streams = lock(&core.streams);
            streams.started(stream_key(Dir::Rx, 0), 0, config);
            streams.delivered(stream_key(Dir::Rx, 0), 0, config, 700);
        }
        let handle = core.clocks.declare_sample_clock(core.stream_node(Dir::Rx, 1), core.root, Rational::new(200, 1).unwrap()).unwrap();
        lock(&core.streams).lines.insert(
            stream_key(Dir::Rx, 1),
            Line::new(ezsdr_radio::timeline::Stream {
                direction: Direction::Rx,
                origin: 0,
                config: Config { channels: Channels::first(1), ratio: Rational::new(200, 1).unwrap() },
                lead: 0,
                call: 2_000,
                allowance: 0,
            }),
        );
        let (to_rx, cmds) = std::sync::mpsc::channel();
        std::mem::forget(to_rx);
        device.rx_open(1, &[0]).unwrap();
        lock(&core.streams).record_open(stream_key(Dir::Rx, 1), Some(Channels::first(1)));
        device.rx_start(1, 0).unwrap();
        let mut rx = Rx::new(core.clone(), 1, cmds, Some((handle, Channels::first(1), 0)), None);
        {
            let streams = lock(&core.streams);
            assert_eq!(streams.first_undelivered(stream_key(Dir::Rx, 1)), Some((0, config, 0)), "the T0 stream begins at its planned segment");
            assert_eq!(streams.first_undelivered(stream_key(Dir::Rx, 0)), Some((0, config, 700)), "stream 0's floor stands");
        }
        core.rx_chain[1].lock().stop_result(device.rx_stop(1)).unwrap();
        rx.finish();
        let streams = lock(&core.streams);
        assert_eq!(streams.first_undelivered(stream_key(Dir::Rx, 0)), Some((0, config, 700)), "stream 0's floor stands");
        assert_eq!(streams.first_undelivered(stream_key(Dir::Rx, 1)), None, "stream 1's floor is cleared");
    }

    #[test]
    fn ur_26_a_receive_start_of_no_planned_segment() {
        // RM-16 (P1b): a loss between the start and uhd-rx's creation of its T0 stream removes
        // the segment from the plan. uhd-rx still begins the stream with the delivered tuple it
        // started with; finishing records nothing, and the loss's end is untouched.
        let (core, _device, time, _events) = super::super::test_support::rig_with_links(vec![link()]);
        let key = stream_key(Dir::Rx, 0);
        let config = Config { channels: Channels::first(1), ratio: Rational::new(200, 1).unwrap() };
        let t0 = core.ticks(10_000_000);
        let loss = core.ticks(5_000_000);
        let node = core.stream_node(Dir::Rx, 0);
        let handle = core.clocks.declare_sample_clock(node, core.root, config.ratio).unwrap();
        {
            let mut streams = lock(&core.streams);
            streams.lines.insert(key, Line::new(ezsdr_radio::timeline::Stream { direction: Direction::Rx, origin: t0, config, lead: 0, call: 2_000, allowance: 0 }));
        }
        core.device.rx_open(0, &[0]).unwrap();
        lock(&core.streams).record_open(key, Some(Channels::first(1)));
        core.device.rx_start(0, t0).unwrap();
        time.advance_to(core.at(loss)).unwrap();
        core.device_lost("test: loss between start and RX owner creation");
        assert!(lock(&core.streams).planned(key).is_empty(), "the loss removed the T0 segment");
        let (to_rx, cmds) = std::sync::mpsc::channel();
        std::mem::forget(to_rx);
        let mut rx = Rx::new(core.clone(), 0, cmds, Some((handle, Channels::first(1), t0)), None);
        {
            let streams = lock(&core.streams);
            assert_eq!(streams.first_undelivered(key), Some((t0, config, 0)), "HEAD keeps the tuple");
        }
        rx.finish();
        let streams = lock(&core.streams);
        assert_eq!(streams.first_undelivered(key), None);
        assert!(streams.planned(key).is_empty(), "finishing invented no segment");
        let ends: Vec<_> = streams.line(key).unwrap().items.iter().filter(|item| matches!(item.kind, Kind::End { .. })).collect();
        assert_eq!(ends.len(), 1, "one end");
        assert_eq!(ends[0].e, loss, "the loss's end is unchanged");
    }

    #[test]
    fn ur_17_an_overlap_trimmed_to_nothing_is_dropped() {
        // UR-17: a block the device delivers again is dropped whole, one that overlaps what was
        // delivered is trimmed to its new samples, and neither is a gap.
        let link = link();
        let (core, _, _, _) = super::super::test_support::rig_with_links(vec![link.clone()]);
        let (mut rx, clock) = rx(&core, 1);
        for (first, n) in [(0, 2_000), (0, 2_000), (1_000, 2_000), (3_000, 1_000)] {
            rx.receive(samples(clock.instant(first), n));
        }
        let blocks: Vec<_> = std::iter::from_fn(|| link.receive().map(|(b, _)| b))
            .map(|block| (block.header().first_sample_time.ticks_in(clock.domain).unwrap(), block.header().len, block.header().flags, block.header().lost))
            .collect();
        assert_eq!(blocks, [(0, 2_000, BlockFlags::NONE, None), (2_000, 1_000, BlockFlags::NONE, None), (3_000, 1_000, BlockFlags::NONE, None)]);
        assert_eq!(lock(&core.rec).stats["rx_overlapping"], 2);
    }

    #[test]
    fn ur_26_a_late_start_after_abort_does_not_stop_again() {
        let (core, device, _, _) = super::super::test_support::rig();
        let (mut rx, _) = rx(&core, 1);
        lock(&core.streams).end(&core, 0, true, false);
        rx.poll();
        rx.receive(RxRecv::LateCommand);
        lock(&core.streams).end(&core, 0, true, false);
        rx.command(RxCmd::Shutdown(StopMode::Abort));
        assert_eq!(device.calls().iter().filter(|c| c.starts_with("rx_stop ")).count(), 1);
        assert_eq!(lock(&core.rec).applied.iter().filter(|r| r["key"] == "rx_stop").count(), 1);
        assert_eq!(device.calls().iter().filter(|c| c.starts_with("rx_start ")).count(), 1, "only the initial start");
    }

    #[test]
    fn ur_26_a_late_start_during_orderly_stop_is_recorded_once() {
        let (core, device, _, _) = super::super::test_support::rig();
        let (mut rx, _) = rx(&core, 1);
        // `Provider::stop(orderly)` at 0: the stream is cut at sample 0 (RM-16).
        lock(&core.streams).end(&core, 0, false, false);
        rx.poll();
        rx.receive(RxRecv::LateCommand);
        rx.receive(RxRecv::LateCommand);
        rx.command(RxCmd::Shutdown(StopMode::Abort));
        assert!(rx.stream.is_none());
        assert_eq!(device.calls().iter().filter(|c| c.starts_with("rx_stop ")).count(), 1);
        let rec = lock(&core.rec);
        let stops: Vec<_> = rec.applied.iter().filter(|r| r["key"] == "rx_stop").collect();
        assert_eq!(stops.len(), 1);
        assert_eq!(stops[0]["at"], serde_json::to_value(core.at(0)).unwrap());
        assert_eq!(device.calls().iter().filter(|c| c.starts_with("rx_start ")).count(), 1, "only the initial start");
    }

    #[test]
    fn ur_26_a_stopped_stream_keeps_its_end() {
        // RM-16, UR-26 (spec 21, VG-1; spec 22, VH-1): a `Stop` at 1 ms cuts the stream at
        // sample 1 000; `Provider::stop` from 5 ms keeps that end under `orderly`, and under
        // `abort` once 1 000 samples are delivered, while an abort with fewer delivered ends
        // the stream there, earlier: an end moves only earlier. With no `Stop` before it, an
        // abort ends the stream at the first sample not yet delivered as soon as uhd-rx reads
        // it, a stream already cut later included. The clock ends at the stream's end; the
        // device is stopped once.
        for (mode, delivered, end, stop) in [(StopMode::Orderly, 300, 1_000, true), (StopMode::Abort, 1_000, 1_000, true), (StopMode::Abort, 300, 300, true), (StopMode::Abort, 300, 300, false)] {
            let link = link();
            let (core, device, time, _) = super::super::test_support::rig_with_links(vec![link.clone()]);
            let (mut rx, clock) = rx(&core, 1);
            if stop {
                book(&core, core.ticks(1_000_000), 1, Kind::Stop);
            }
            rx.receive(samples(clock.instant(0), delivered as usize));
            time.advance_to(core.at(core.ticks(5_000_000))).unwrap();
            {
                // As `Provider::stop` books it.
                let mut streams = lock(&core.streams);
                streams.stop = Some((core.ticks(5_000_000), mode));
                streams.end(&core, core.ticks(5_000_000), mode == StopMode::Abort, false);
            }
            rx.poll();
            let case = format!("{mode:?}, {delivered} delivered, a Stop before: {stop}");
            if mode == StopMode::Abort {
                assert!(rx.stream.is_none(), "{case}: ended as soon as uhd-rx reads the abort");
                assert_eq!(device.calls().iter().filter(|c| c.starts_with("rx_stop ")).count(), 1, "{case}");
            }
            rx.command(RxCmd::Shutdown(mode));
            if delivered < 1_000 {
                rx.receive(samples(clock.instant(delivered), 6_000));
            }
            let blocks: i64 = received(&link).iter().map(|(_, _, len)| i64::from(*len)).sum();
            assert_eq!(blocks, end, "{case}");
            assert!(rx.stream.is_none(), "{case}");
            assert_eq!(clocks(&core), [(0, Some(clock.instant(end)))], "{case}");
            assert_eq!(device.calls().iter().filter(|c| c.starts_with("rx_stop ")).count(), 1, "{case}");
            assert_eq!(lock(&core.rec).applied.iter().filter(|r| r["key"] == "rx_stop").count(), 1, "{case}");
        }
    }

    #[test]
    fn ur_17_a_finite_segment_delivers_through_its_cut() {
        // UR-17, RM-16 (design note 37, the finite middle segment): a stream whose segment is
        // cut at sample 5 000 delivers what arrives up to that sample and no further, ends its
        // clock there, and its next segment begins at its own origin on a new clock whose
        // first sample is index 0. `Segment.cut` is a sample of the segment's own clock
        // (`timeline.rs`), so the cut is 5 000, not its instant: on the clock that instant is
        // 5 ms's root tick.
        let link = link();
        // The device's clock is held at 0, so the next segment's start is issued strictly
        // before its instant and the device delivers rather than reports a missed start.
        let device = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, ..crate::device::FakeConfig::default() }));
        let (core, time, _events) =
            super::super::test_support::rig_on_description_links(device.clone(), vec![Vec::new(), vec![link.clone()]], BTreeMap::new(), super::super::test_support::default_description(2_000));
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        super::super::test_support::device_at(&core, &time, &device, 0);
        let stream = core.id.child("rx/s1").unwrap();
        let handle = core.clocks.declare_sample_clock(stream, core.root, Rational::new(200, 1).unwrap()).unwrap();
        let clock = Clock { domain: handle.id(), origin: 0, n: 200 };
        let config = Config { channels: Channels::first(0).with(1), ratio: Rational::new(200, 1).unwrap() };
        {
            let mut streams = lock(&core.streams);
            let mut line = Line::new(ezsdr_radio::timeline::Stream { direction: Direction::Rx, origin: 0, config, lead: 0, call: 0, allowance: 0 });
            line.plan = vec![Segment { origin: 0, cut: Some(5_000), config, by: None }, Segment { origin: clock.instant(5_000), cut: None, config, by: Some(1) }];
            streams.lines.insert(stream_key(Dir::Rx, 1), line);
            streams.stream_mut(stream_key(Dir::Rx, 1)).configs.insert(1, Arc::new(ColdConfig::new(BTreeMap::new())));
        }
        let (to_rx, cmds) = std::sync::mpsc::channel();
        std::mem::forget(to_rx);
        let mut rx = Rx::new(core.clone(), 1, cmds, Some((handle, config.channels, 0)), None);
        rx.poll();
        // The device returns no more than the cut (the request is its bound:
        // `ur_17_a_request_ends_at_the_pending_cut`), so the segment delivers up to it.
        rx.receive(samples(clock.instant(0), 4_000));
        rx.receive(samples(clock.instant(4_000), 1_000));
        let blocks = received(&link);
        assert_eq!(blocks.iter().map(|(_, _, len)| i64::from(*len)).sum::<i64>(), 5_000, "the cut's own samples: {blocks:?}");
        assert!(blocks.iter().all(|(_, first, len)| *first + i64::from(*len) <= 5_000), "a block runs past the cut: {blocks:?}");
        // The old clock ends at its cut's instant — the next segment's own origin.
        assert_eq!(clocks(&core), [(0, Some(clock.instant(5_000)))], "the old clock does not end at its cut");
        // The next segment begins at its own origin, on a new clock whose first sample is 0.
        assert!(rx.begin(), "the next segment begins: {:?}", device.calls());
        let next = rx.stream.as_ref().map(|stream| (stream.clock.domain, stream.clock.origin));
        assert!(next.is_some_and(|(domain, origin)| domain != clock.domain && origin == clock.instant(5_000)), "the next segment's own clock: {next:?}");
        // Its start went out before its instant (the device's clock is still at 0), so the
        // device delivers its samples; a missed start would come back as `LateCommand`.
        assert!(device.time_now().unwrap() < clock.instant(5_000), "the device's clock reached the start before it was issued");
        super::super::test_support::device_at(&core, &time, &device, clock.instant(5_000) + 1_000 * 200);
        let got = device.rx_recv(1, 1_000, Wall::from_millis(100), false);
        assert!(matches!(got, RxRecv::Samples { .. }), "the next segment's start was late: {got:?}");
        rx.receive(got);
        let blocks = received(&link);
        assert_eq!(blocks.len(), 1, "the next segment's own block: {blocks:?}");
        assert_ne!(blocks[0].0, clock.domain, "the block is on the old clock: {blocks:?}");
        assert_eq!((blocks[0].1, blocks[0].2), (0, 1_000), "the new clock's first sample is not index 0: {blocks:?}");
    }

    #[test]
    fn ur_17_a_cut_is_never_before_what_was_delivered() {
        // RM-16, UR-17: a plan's cut before the first sample not yet delivered — the segment's,
        // or the segment dropped from the plan, the plan then empty or not — ends the stream at
        // that sample.
        for case in ["cut", "dropped", "empty"] {
            let link = link();
            let (core, _, _, _) = super::super::test_support::rig_with_links(vec![link.clone()]);
            let (mut rx, clock) = rx(&core, 1);
            rx.receive(samples(clock.instant(0), 3_000));
            match case {
                "cut" => plan(&core, &[(0, Some(1_000), None)]),
                "dropped" => plan(&core, &[(core.ticks(500_000_000), None, Some(9))]),
                _ => plan(&core, &[]),
            }
            rx.poll();
            rx.receive(samples(clock.instant(3_000), 1_000));
            assert!(rx.stream.is_none(), "{case}");
            assert_eq!(clocks(&core), [(0, Some(clock.instant(3_000)))], "{case}");
        }
    }

    #[test]
    fn ur_26_uhd_rx_follows_the_stop_it_reads() {
        // RM-16, UR-26: `Provider::stop(orderly)` from 5 ms books the stream's end, which
        // uhd-rx reads at its next turn: what arrives is published only up to 5 ms's sample,
        // and a later stop instant does not move the stream's end.
        let link = link();
        let (core, _, time, _) = super::super::test_support::rig_with_links(vec![link.clone()]);
        let (mut rx, clock) = rx(&core, 1);
        rx.receive(samples(clock.instant(0), 300));
        time.advance_to(core.at(core.ticks(8_000_000))).unwrap();
        lock(&core.streams).end(&core, core.ticks(5_000_000), false, false);
        lock(&core.streams).end(&core, core.ticks(7_000_000), false, false);
        rx.poll();
        rx.receive(samples(clock.instant(300), 10_000));
        let end: i64 = received(&link).iter().map(|(_, _, len)| i64::from(*len)).sum();
        assert_eq!(end, 5_000);
        assert_eq!(clocks(&core), [(0, Some(clock.instant(5_000)))]);
    }

    #[test]
    fn ur_29_a_loss_ends_the_stream_at_its_instant() {
        // RM-16, UR-29: the thread that finds the device lost books the loss with the first
        // sample uhd-rx has not delivered, and uhd-rx ends the stream at the loss's sample —
        // not at a later cut its plan already had —, never before what it delivered: lost at
        // 5 ms and noticed by uhd-rx at 7 ms, the end is 5 ms's sample; lost at 2 ms with 3 000
        // samples delivered, it is sample 3 000.
        for (lost, expected) in [(5_000_000, 5_000), (2_000_000, 3_000)] {
            let link = link();
            let (core, _, time, _) = super::super::test_support::rig_with_links(vec![link.clone()]);
            let (mut rx, clock) = rx(&core, 1);
            book(&core, core.ticks(10_000_000), 1, Kind::Stop);
            rx.poll();
            rx.receive(samples(clock.instant(0), 3_000));
            time.advance_to(core.at(core.ticks(lost))).unwrap();
            core.device_lost("test");
            time.advance_to(core.at(core.ticks(7_000_000))).unwrap();
            assert_eq!(rx.turn(), Turn::Idle);
            assert!(rx.stream.is_none());
            assert_eq!(clocks(&core), [(0, Some(clock.instant(expected)))], "lost at {lost}");
            assert_eq!(lock(&core.streams).planned(stream_key(Dir::Rx, 0))[0].segment.cut, Some(expected), "the plan's cut, lost at {lost}");
        }
    }

    #[test]
    fn ur_29_a_loss_found_after_uhd_rx_read_the_plan_ends_the_stream() {
        // RM-16, UR-29: a loss another thread finds after uhd-rx's turn read the plan — the
        // plan with no cut, or with a later one — still ends the stream and its clock at the
        // loss's sample: uhd-rx reads the plan again when it sees the device lost.
        for later_cut in [false, true] {
            let link = link();
            let (core, _, time, _) = super::super::test_support::rig_with_links(vec![link.clone()]);
            let (mut rx, clock) = rx(&core, 1);
            if later_cut {
                book(&core, core.ticks(10_000_000), 1, Kind::Stop);
            }
            rx.poll();
            rx.receive(samples(clock.instant(0), 3_000));
            time.advance_to(core.at(core.ticks(5_000_000))).unwrap();
            core.device_lost("test");
            rx.finish();
            assert!(rx.stream.is_none());
            assert_eq!(clocks(&core), [(0, Some(clock.instant(5_000)))], "a later cut: {later_cut}");
        }
    }

    #[test]
    fn ur_29_a_silent_stream_ends_at_a_cut_booked_during_its_wait() {
        // RM-16, UR-29: a loss, or `Provider::stop`'s orderly end, booked by another thread
        // while uhd-rx waits in a receive call ends a stream that has gone silent at the
        // plan's cut, not at the later one its turn read: `finish` reads the plan.
        for lost in [true, false] {
            let (core, _, time, _) = super::super::test_support::rig_with_links(vec![link()]);
            let (mut rx, clock) = rx(&core, 1);
            let ms = |n: i64| core.ticks(n * 1_000_000);
            book(&core, ms(10), 1, Kind::Stop);
            rx.receive(samples(clock.instant(0), 3_000));
            rx.poll();
            time.advance_to(core.at(ms(5))).unwrap();
            if lost {
                core.device_lost("test");
            } else {
                let mut streams = lock(&core.streams);
                streams.stop = Some((ms(5), StopMode::Orderly));
                streams.end(&core, ms(5), false, false);
            }
            time.advance_to(core.at(ms(50))).unwrap();
            rx.last_samples = rx.host.now();
            rx.host.sleep(Wall::from_millis(10));
            rx.receive(RxRecv::Timeout);
            assert!(rx.stream.is_none(), "lost: {lost}");
            let cut = lock(&core.streams).planned(stream_key(Dir::Rx, 0))[0].segment.cut;
            assert_eq!(cut, Some(5_000), "lost: {lost}");
            assert_eq!(clocks(&core), [(0, Some(clock.instant(5_000)))], "lost: {lost}");
        }
    }

    #[test]
    fn ur_29_a_loss_takes_effect_before_a_change_at_its_instant() {
        // RM-25: a loss counts as received with the faults, at the Run's start: a change booked
        // earlier at the loss's instant takes effect after it, so the loss cuts the running
        // segment, floored at the first sample uhd-rx has not delivered (RM-16), and the
        // change's cut never applies.
        let (core, _, time, _) = super::super::test_support::rig_with_links(vec![link()]);
        let (mut rx, clock) = rx(&core, 1);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        rx.receive(samples(clock.instant(0), 12_000));
        book(&core, ms(10), 1, Kind::Stop);
        time.advance_to(core.at(ms(10))).unwrap();
        core.device_lost("test");
        let planned: Vec<_> = lock(&core.streams).planned(stream_key(Dir::Rx, 0)).iter().map(|p| (p.segment.origin, p.segment.cut)).collect();
        assert_eq!(planned, [(0, Some(12_000))]);
    }

    #[test]
    fn ur_26_an_end_takes_delivered_only_from_the_segment_it_cuts() {
        // RM-16: an end carries the first sample uhd-rx has not delivered only when uhd-rx's
        // segment still runs at the end. A `Stop` at 10 ms cuts the stream at sample 10 000;
        // an abort at 12 ms, with 9 000 delivered, leaves that cut and plans no segment after
        // it — not the next segment cut at the previous one's sample index.
        let (core, _, time, _) = super::super::test_support::rig_with_links(vec![link()]);
        let (mut rx, clock) = rx(&core, 1);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        book(&core, ms(10), 1, Kind::Stop);
        book(&core, ms(11), 2, Kind::Start);
        rx.poll();
        rx.receive(samples(clock.instant(0), 9_000));
        time.advance_to(core.at(ms(12))).unwrap();
        lock(&core.streams).end(&core, ms(12), true, false);
        let planned: Vec<_> = lock(&core.streams).planned(stream_key(Dir::Rx, 0)).iter().map(|p| (p.segment.origin, p.segment.cut)).collect();
        assert_eq!(planned, [(0, Some(10_000))]);
    }

    #[test]
    fn ur_26_an_abort_stops_a_queued_start_at_shutdown() {
        // UR-26, RM-16: under `abort`, a segment whose timed start uhd-rx has queued and whose
        // origin has not passed is not stopped when uhd-rx reads the abort (#56), but at its
        // shutdown, once, before uhd-rx exits; it has no sample, so no clock.
        let (core, device, time, _) = super::super::test_support::rig_with_links(vec![link()]);
        let (mut rx, clock) = rx(&core, 1);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        plan(&core, &[(0, Some(10_000), None), (ms(65), None, Some(1))]);
        rx.poll();
        rx.receive(samples(0, 10_000));
        assert!(rx.begin());
        time.advance_to(core.at(ms(20))).unwrap();
        {
            let mut streams = lock(&core.streams);
            streams.stop = Some((ms(20), StopMode::Abort));
            streams.end(&core, ms(20), true, false);
        }
        rx.poll();
        let stops = || device.calls().iter().filter(|c| c.starts_with("rx_stop ")).count();
        assert_eq!(stops(), 1, "only the first stream's stop yet");
        rx.command(RxCmd::Shutdown(StopMode::Abort));
        assert!(rx.stream.is_none());
        assert_eq!(stops(), 2);
        assert_eq!(clocks(&core), [(0, Some(clock.instant(10_000)))]);
    }

    #[test]
    fn ur_26_uhd_rx_aborts_when_its_provider_is_gone() {
        // RM-16: with its command channel gone and no Provider::stop, uhd-rx aborts at the first
        // sample not yet delivered, shortening a future planned Stop and ending its clock there.
        let (core, device, _, _) = super::super::test_support::rig_with_links(vec![link()]);
        let (mut rx, clock) = rx(&core, 1);
        book(&core, clock.instant(1_000), 1, Kind::Stop);
        rx.receive(samples(clock.instant(0), 300));
        let (to_rx, cmds) = std::sync::mpsc::channel();
        drop(to_rx);
        rx.cmds = cmds;
        rx.poll();
        assert_eq!(core.rx_chain[0].lock().terminate(), Some(StopMode::Abort));
        assert!(rx.stream.is_none());
        assert_eq!(clocks(&core), [(0, Some(clock.instant(300)))]);
        assert_eq!(device.calls().iter().filter(|c| c.starts_with("rx_stop ")).count(), 1);
    }

    #[test]
    fn ur_17_a_request_ends_at_the_pending_cut() {
        // UR-17: uhd-rx asks the device for no more than up to a pending cut.
        let (core, _, _, _) = super::super::test_support::rig_with_links(vec![link()]);
        let (mut rx, clock) = rx(&core, 1);
        plan(&core, &[(0, Some(1_000), None)]);
        rx.poll();
        assert_eq!(rx.recv_len(), 1_000.min(core.block_len));
        rx.receive(samples(clock.instant(0), 300));
        assert_eq!(rx.recv_len(), 700.min(core.block_len));
    }

    #[test]
    fn ur_25_largest_aligned_cut_is_safe_in_rx_owner() {
        let (core, _, _, _) = super::super::test_support::rig();
        let (mut rx, _) = rx(&core, 1);
        plan(&core, &[(0, Some(i64::MAX.div_euclid(200)), None)]);
        rx.poll();
        assert_eq!(rx.recv_len(), core.block_len);
        assert_eq!(rx.recv_timeout(), super::RECV_TIMEOUT);
        rx.receive(crate::device::RxRecv::Timeout);
        assert!(rx.stream.is_some());
    }

    #[test]
    fn ur_25_a_stream_silent_before_e1_switches_before_e2() {
        // UR-25 (design note 37): a stream silent before its cut is stopped at the cut and its
        // successor begun before its origin. The receive wait is bounded by the cut and its
        // quiet span, not by the 100 ms timeout that would carry the switch past e2. On the
        // controlled clock: the last block and the cut are fixed instants.
        let (core, device, time, _) =
            super::super::test_support::rig_with(crate::device::FakeConfig { faults: vec![crate::device::FakeFault::Silence(Wall::ZERO)], ..Default::default() }, vec![link()]);
        let (mut rx, clock) = rx(&core, 1);
        rx.host = crate::host::HostClock::manual();
        let ms = |n: i64| core.ticks(n * 1_000_000);
        // Segment A ends at e1 = 10 ms; segment B, the switch, starts a receive start lead later.
        plan(&core, &[(0, Some(10_000), None), (ms(60), None, Some(1))]);
        rx.poll();
        rx.receive(samples(clock.instant(0), 5_000));
        // Silent from 5 ms: at the cut, the wait is no longer than to the cut plus its span.
        time.advance_to(core.at(ms(10))).unwrap();
        rx.poll();
        assert!(rx.stream.is_some(), "the stream runs up to its cut");
        // `recv_timeout` is cut + quiet span - now: a 2 000-sample block at 1 MS/s and the 3 ms
        // allowance are 5 ms, so from the 10 ms cut it is 5 ms, not the 100 ms default that
        // would carry the switch past e2 = 60 ms.
        assert_eq!(rx.recv_timeout(), Wall::from_millis(5), "the wait ends at the cut plus its span");
        // Advance the owner's controlled host clock rather than subtracting from real time.
        rx.host.advance_for_test(Wall::from_millis(6));
        rx.last_samples = Wall::ZERO;
        time.advance_to(core.at(ms(15))).unwrap();
        // Past the cut plus its span the wait floors at 1 ms (UHD truncates the timeout to
        // whole milliseconds), and the receive call is given exactly that.
        assert_eq!(rx.recv_timeout(), Wall::from_millis(1), "the wait floors at 1 ms");
        assert_eq!(rx.turn(), Turn::Busy);
        assert_eq!(device.min_recv_timeout(), Wall::from_millis(1), "the bounded wait reaches rx_recv");
        assert!(rx.stream.is_none(), "a stream silent past its cut is ended");
        let applied = lock(&core.rec).applied.clone();
        let stops: Vec<_> = applied.iter().filter(|r| r["key"] == "rx_stop").collect();
        assert_eq!(stops.len(), 1, "the silent stream's stop is recorded: {applied:?}");
        assert_eq!(stops[0]["at"]["ticks"].as_i64(), Some(ms(10)), "{:?}", stops[0]);
        assert_eq!(clocks(&core), [(0, Some(clock.instant(10_000)))], "its clock ends at the cut");
        // The successor begins before its origin, its timed start at that origin. Its
        // acceptance by the device is not this property's: the owner's switch is.
        assert!(rx.begin(), "the switch begins");
        let timing = lock(&core.rec).timing.clone();
        let switch = timing.iter().find(|r| r["what"] == "rx_segment").unwrap_or_else(|| panic!("no switch: {timing:?}"));
        assert_eq!(switch["origin"].as_i64(), Some(ms(60)), "{switch}");
        assert_eq!(switch["at"].as_i64(), Some(ms(15)), "{switch}");
        assert!(device.calls().iter().any(|c| *c == format!("rx_start 0 {}", ms(60))), "{:?}", device.calls());
    }

    #[test]
    fn ur_25_a_silent_stream_past_its_cut_records_its_stop() {
        let (core, _, time, _) = super::super::test_support::rig_with_links(vec![link()]);
        let (mut rx, clock) = rx(&core, 1);
        rx.host = crate::host::HostClock::manual();
        let ms = |n: i64| core.ticks(n * 1_000_000);
        plan(&core, &[(0, Some(10_000), None)]);
        rx.poll();
        rx.receive(samples(clock.instant(0), 5_000));

        rx.host.advance_for_test(Wall::from_millis(6));
        rx.last_samples = Wall::ZERO;
        time.advance_to(core.at(ms(15))).unwrap();
        rx.receive(RxRecv::Timeout);

        assert!(rx.stream.is_none(), "the silent stream past its cut is ended");
        let applied = lock(&core.rec).applied.clone();
        let stops: Vec<_> = applied.iter().filter(|row| row["key"] == "rx_stop").collect();
        assert_eq!(stops.len(), 1, "the silent stream's stop is recorded: {applied:?}");
        assert_eq!(stops[0]["at"]["ticks"].as_i64(), Some(ms(10)), "{:?}", stops[0]);
        assert_eq!(clocks(&core), [(0, Some(clock.instant(10_000)))]);
    }

    #[test]
    fn ur_25_a_second_change_follows_the_first() {
        // RM-25 (spec 22, VH-2): a second change before the first's origin replaces the
        // first's segment, which has no sample, so no clock: uhd-rx registers a clock at its
        // first block, whether the second change comes before or after it began the first.
        for began in [false, true] {
            let link = link();
            let (core, _, time, _) = super::super::test_support::rig_with_links(vec![link.clone()]);
            let (mut rx, clock) = rx(&core, 1);
            let ms = |n: i64| core.ticks(n * 1_000_000);
            plan(&core, &[(0, Some(10_000), None), (ms(65), None, Some(1))]);
            rx.poll();
            rx.receive(samples(0, 10_000));
            assert!(rx.stream.is_none());
            if began {
                assert!(rx.begin());
                plan(&core, &[(0, Some(10_000), None), (ms(115), None, Some(2))]);
                rx.poll();
                time.advance_to(core.at(ms(70))).unwrap();
                rx.receive(samples(ms(65), 1_000));
                assert!(rx.stream.is_none(), "the first's segment ends at its origin");
            } else {
                plan(&core, &[(0, Some(10_000), None), (ms(115), None, Some(2))]);
            }
            assert!(rx.begin());
            rx.receive(samples(ms(115), 1_000));
            let blocks = received(&link);
            assert_eq!(blocks.iter().map(|(_, first, len)| (*first, *len)).collect::<Vec<_>>(), [(0, 10_000), (0, 1_000)], "began {began}");
            assert_eq!(clocks(&core), [(0, Some(clock.instant(10_000))), (ms(115), None)], "began {began}");
        }
    }

    #[test]
    fn ur_13_a_failed_open_by_the_owner_is_opened_again() {
        // UR-13 (S7): an open by uhd-rx that fails leaves the stream's set unknown, so the next
        // segment opens the streamer again, though the owner recorded the same set before.
        let config = crate::device::FakeConfig { faults: vec![crate::device::FakeFault::RxOpenFails], ..Default::default() };
        let (core, device, _, _) = super::super::test_support::rig_with(config, vec![link()]);
        let (mut rx, _) = rx_unopened(&core, 1);
        rx.stream = None;
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let opens = || device.calls().iter().filter(|c| c.starts_with("rx_open ")).count();
        book(&core, ms(20), 2, Kind::Stop);
        book(&core, ms(40), 3, Kind::Start);
        book(&core, ms(60), 4, Kind::Start);
        assert!(rx.begin());
        assert!(rx.stream.is_none(), "the failed open begins no stream");
        assert_eq!(opens(), 1, "{:?}", device.calls());
        assert!(matches!(lock(&core.streams).stream(stream_key(Dir::Rx, 0)).set, crate::provider::core::DeviceSet::Unknown));
        assert!(rx.begin());
        assert_eq!(opens(), 2, "the next segment opens the streamer again: {:?}", device.calls());
    }

    #[test]
    fn ur_13_a_partly_applied_set_is_configured_whole_without_reopen() {
        // UR-13 (S25): channel 1 of {0,1} is refused after channel 0 applied. The record keeps
        // the set open and not configured, so the next segment configures both channels, and no
        // streamer is opened again.
        let inner = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig::default()));
        let device: Arc<dyn crate::device::Device> = Arc::new(super::super::test_support::ApplyFails::new(inner.clone(), Dir::Rx, Some(1)));
        let (core, _, _) = super::super::test_support::rig_on(device, vec![link()], 2_000);
        let (mut rx, _) = rx(&core, 2);
        core.rx_chain[0].lock().request_end_now();
        rx.try_end();
        assert!(rx.stream.is_none(), "the first segment is stopped before its successor is configured");
        let ms = |n: i64| core.ticks(n * 1_000_000);
        book(&core, ms(20), 2, Kind::Stop);
        book(&core, ms(40), 3, Kind::Start);
        assert!(rx.begin(), "the partly applied configure is reported");
        assert!(lock(&core.streams).stream(stream_key(Dir::Rx, 0)).configured.is_none());
        // The halted stream is stopped and started again.
        book(&core, ms(100), 4, Kind::Stop);
        book(&core, ms(110), 5, Kind::Start);
        assert!(rx.begin());
        let calls = inner.calls();
        assert_eq!(calls.iter().filter(|call| call.starts_with("rx_open")).count(), 1, "{calls:?}");
        assert_eq!(calls.iter().filter(|call| call.starts_with("apply rx 0")).count(), 2, "the next segment configures the first channel again: {calls:?}");
        assert_eq!(calls.iter().filter(|call| call.starts_with("apply rx 1")).count(), 1, "and the channel the device refused: {calls:?}");
        assert!(lock(&core.streams).stream(stream_key(Dir::Rx, 0)).configured.is_some_and(|by| by == 5), "{calls:?}");
    }

    #[test]
    fn ur_13_a_failed_configure_is_configured_again_without_reopen() {
        // UR-13 (S6, the owner's side): a configure that fails leaves the streamer open; the
        // next segment configures it without a second open.
        let config = crate::device::FakeConfig { faults: vec![crate::device::FakeFault::RxApplyFails], ..Default::default() };
        let (core, device, _, _) = super::super::test_support::rig_with(config, vec![link()]);
        let (mut rx, _) = rx(&core, 1);
        core.rx_chain[0].lock().request_end_now();
        rx.try_end();
        assert!(rx.stream.is_none(), "the first segment is stopped before its successor is configured");
        let ms = |n: i64| core.ticks(n * 1_000_000);
        book(&core, ms(20), 2, Kind::Stop);
        book(&core, ms(40), 3, Kind::Start);
        assert!(rx.begin(), "the failed configure is reported");
        // The halted stream is stopped and started again.
        book(&core, ms(100), 4, Kind::Stop);
        book(&core, ms(110), 5, Kind::Start);
        assert!(rx.begin());
        let calls = device.calls();
        assert_eq!(calls.iter().filter(|call| call.starts_with("rx_open")).count(), 1, "the initial open is reused: {calls:?}");
        assert_eq!(calls.iter().filter(|call| call.starts_with("apply")).count(), 2, "{calls:?}");
    }

    #[test]
    fn ur_25_a_refused_segment_is_booked_by_uhd_rx() {
        // RM-25: when the device refuses a segment's configuration, uhd-rx books the `start_rx` as
        // refused itself, which halts the stream, and reads the plan made so at once. The `start_rx`
        // after the owner's Stop at 10 ms begins after the call in progress there (2 ms at 1 MS/s),
        // the allowance (3 ms) and lead (50 ms): 65 ms.
        let refuse = crate::device::FakeFault::WrongRate { claimed: 1e6, applied: 1e6 - 1_000.0, nth: 0 };
        let (core, device, _, _) = super::super::test_support::rig_with(crate::device::FakeConfig { faults: vec![refuse], ..Default::default() }, vec![link()]);
        let (mut rx, _) = rx(&core, 1);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        book(&core, ms(10), 1, Kind::Stop);
        book(&core, ms(15), 2, Kind::Start);
        assert_eq!(lock(&core.streams).planned(stream_key(Dir::Rx, 0)).iter().map(|p| (p.segment.origin, p.segment.by)).collect::<Vec<_>>(), [(0, None), (ms(65), Some(2))]);
        rx.poll();
        rx.receive(samples(0, 10_000));
        assert!(rx.stream.is_none());
        {
            let streams = lock(&core.streams);
            assert_eq!(streams.first_undelivered(stream_key(Dir::Rx, 0)), None);
        }
        assert!(rx.begin());
        assert!(lock(&core.streams).line(stream_key(Dir::Rx, 0)).unwrap().items.iter().any(|item| item.seq == 2 && item.refused));
        // The owner learns of the refusal, stops the halted stream and starts it again.
        book(&core, ms(90), 3, Kind::Stop);
        book(&core, ms(95), 4, Kind::Start);
        assert!(rx.begin());
        let starts: Vec<_> = device.calls().into_iter().filter(|c| c.starts_with("rx_start ")).collect();
        assert_eq!(starts, ["rx_start 0 0".to_owned(), format!("rx_start 0 {}", ms(145))]);
        let stream = rx.stream.as_ref().expect("the resumed segment");
        assert_eq!(stream.clock.origin, ms(145), "the resumed segment begins from the refused one's cut");
        assert_eq!(stream.config.ratio.num(), 200);
        let streams = lock(&core.streams);
        assert_eq!(streams.first_undelivered(stream_key(Dir::Rx, 0)), Some((ms(145), stream.config, 0)));
        assert_eq!(streams.planned(stream_key(Dir::Rx, 0)).last().map(|planned| planned.segment.by), Some(Some(4)), "the command that began the resumed segment is the second Start's");
    }

    #[test]
    fn ur_17_an_ended_segment_prunes_the_items_before_it() {
        // #63: when uhd-rx ends a segment, the timeline's items before the command that began
        // it are pruned, with their segments' configurations: after a restart at 100 ms, another
        // at 300 ms and a `Stop` at 400 ms, the first pair goes once the second restart's segment
        // ends.
        let (core, _, _, _) = super::super::test_support::rig_with_links(vec![link()]);
        let (mut rx, _) = rx(&core, 1);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        book(&core, ms(10), 1, Kind::Stop);
        book(&core, ms(100), 2, Kind::Start);
        book(&core, ms(200), 3, Kind::Stop);
        book(&core, ms(300), 4, Kind::Start);
        book(&core, ms(400), 5, Kind::Stop);
        rx.poll();
        rx.receive(samples(0, 10_000));
        for (origin, len) in [(ms(100), 100_000), (ms(300), 100_000)] {
            assert!(rx.begin());
            rx.receive(samples(origin, len));
            assert!(rx.stream.is_none(), "the segment from {origin} ends at its cut");
        }
        let streams = lock(&core.streams);
        let items: Vec<u64> = streams.line(stream_key(Dir::Rx, 0)).unwrap().items.iter().map(|item| item.seq).collect();
        assert_eq!(items, [4, 5]);
        assert_eq!(streams.stream(stream_key(Dir::Rx, 0)).configs.keys().copied().collect::<Vec<_>>(), [4, 5]);
        assert_eq!(streams.planned(stream_key(Dir::Rx, 0)).len(), 3, "every segment stays in the plan");
    }

    #[test]
    fn ur_26_no_segment_begins_once_provider_stop_has_begun() {
        // UR-25, UR-26 (spec 22, VH-4; #58): `Provider::stop` ends the stream in the plan, so
        // uhd-rx starts no planned segment after it, the one a `start_rx` booked before it included.
        let (core, device, _, _) = super::super::test_support::rig_with_links(vec![link()]);
        let (mut rx, _) = rx(&core, 1);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        book(&core, ms(10), 1, Kind::Stop);
        book(&core, ms(11), 2, Kind::Start);
        lock(&core.streams).end(&core, ms(5), false, false);
        rx.poll();
        rx.receive(samples(0, 10_000));
        assert!(rx.stream.is_none());
        assert!(!rx.begin());
        assert_eq!(device.calls().iter().filter(|c| c.starts_with("rx_start ")).count(), 1, "no successor start: {:?}", device.calls());
    }

    #[test]
    fn ur_26_a_stop_before_a_queued_start_stops_after_its_origin() {
        // UR-17 (spec 22, VH-6; #56): a `Stop` that cancels a segment whose timed start uhd-rx
        // has already queued leaves that start standing, so the stream is stopped untimed only
        // once its origin has passed — at its first samples, or silent past it — and what
        // arrives is discarded: nothing on a new clock.
        for silent in [false, true] {
            let link = link();
            let (core, device, time, _) = super::super::test_support::rig_with_links(vec![link.clone()]);
            let (mut rx, _) = rx(&core, 1);
            let ms = |n: i64| core.ticks(n * 1_000_000);
            plan(&core, &[(0, Some(10_000), None), (ms(65), None, Some(1))]);
            rx.poll();
            rx.receive(samples(0, 10_000));
            assert!(rx.begin());
            assert!(device.calls().contains(&format!("rx_start 0 {}", ms(65))));
            // The `Stop` at 20 ms cuts the planned segment at its origin: the plan drops it.
            time.advance_to(core.at(ms(20))).unwrap();
            plan(&core, &[(0, Some(10_000), None)]);
            rx.poll();
            assert_eq!(device.calls().iter().filter(|c| c.starts_with("rx_stop ")).count(), 1, "only the first stream's stop yet");
            time.advance_to(core.at(ms(80))).unwrap();
            if silent {
                rx.last_samples = rx.host.now();
                rx.host.sleep(Wall::from_millis(10));
                rx.receive(RxRecv::Timeout);
            } else {
                rx.receive(samples(ms(65), 2_000));
            }
            assert!(rx.stream.is_none(), "silent {silent}");
            let stops: Vec<_> = lock(&core.rec).timing.iter().filter(|r| r["what"] == "rx_stop_untimed").map(|r| r["at"].as_i64().unwrap()).collect();
            assert_eq!(stops.len(), 2, "silent {silent}");
            assert!(stops[1] >= ms(65), "the second stop after the origin: {stops:?}");
            assert_eq!(received(&link).len(), 1, "silent {silent}: nothing on a new clock");
            assert_eq!(core.clocks.sample_clock_records().len(), 1, "silent {silent}");
        }
    }

    /// The transmit stream's line, as `Control::new` makes it for a transmit stream with one
    /// channel at 1 MS/s.
    fn tx_line(core: &Core) -> Line {
        Line::new(ezsdr_radio::timeline::Stream {
            direction: Direction::Tx,
            origin: 0,
            config: Config { channels: Channels::first(1), ratio: Rational::new(200, 1).unwrap() },
            lead: 0,
            call: core.block_len as i64,
            allowance: core.ticks(DELIVERY_ALLOWANCE_NS),
        })
    }

    #[test]
    fn ur_29_a_loss_ends_every_stream_once() {
        // RM-16, UR-29 (design 33 D15): the thread that finds the device lost books the loss
        // on every stream's line, once, each with the loss's instant and arrival order 0; a
        // `Provider::stop` booked after it books nothing more, and the receive stream's end
        // stays the loss's floor.
        let link = link();
        let (core, _, time, _) = super::super::test_support::rig_with_links(vec![link.clone()]);
        let (mut rx, clock) = rx(&core, 1);
        let (rx_key, tx_key) = (stream_key(Dir::Rx, 0), stream_key(Dir::Tx, 0));
        lock(&core.streams).lines.insert(tx_key, tx_line(&core));
        rx.receive(samples(clock.instant(0), 3_000));
        time.advance_to(core.at(core.ticks(2_000_000))).unwrap();
        core.device_lost("test");
        let ends = |stream: StreamKey| -> Vec<(i64, u64, bool)> {
            let streams = lock(&core.streams);
            streams
                .line(stream)
                .expect("the line")
                .items
                .iter()
                .filter_map(|item| match item.kind {
                    Kind::End { abort } => Some((item.e, item.seq, abort)),
                    _ => None,
                })
                .collect()
        };
        let loss = core.ticks(2_000_000);
        assert_eq!(ends(rx_key), [(loss, 0, false)], "the receive line");
        assert_eq!(ends(tx_key), [(loss, 0, false)], "the transmit line");
        assert_eq!(lock(&core.streams).planned(rx_key)[0].segment.cut, Some(3_000), "the loss's floor");
        // `Provider::stop` after the loss books nothing more: `ended` is the instance's.
        {
            let mut streams = lock(&core.streams);
            streams.stop = Some((core.ticks(5_000_000), StopMode::Abort));
            streams.end(&core, core.ticks(5_000_000), true, false);
        }
        assert_eq!(ends(rx_key), [(loss, 0, false)], "the receive line after the stop");
        assert_eq!(ends(tx_key), [(loss, 0, false)], "the transmit line after the stop");
        assert_eq!(lock(&core.streams).planned(rx_key)[0].segment.cut, Some(3_000), "the end moved");
    }

    #[test]
    fn ur_25_the_line_lock_is_free_inside_every_receive_device_call() {
        // UR-25 (design 33 D15): uhd-rx holds no line lock across its device calls — the
        // streamer's reopen, the direction's configuration, the start, the receive call and
        // the untimed stop — and the spy proves the sequence passed each one.
        let spy = Arc::new(super::super::test_support::LockSpy::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, ..crate::device::FakeConfig::default() }));
        let (core, _, _) = super::super::test_support::rig_on(spy.clone(), vec![link()], 2_000);
        spy.watch(&core);
        let (mut rx, clock) = rx(&core, 1);
        // The running stream's receive call.
        assert_eq!(rx.turn(), Turn::Busy);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let key = stream_key(Dir::Rx, 0);
        let configs = [Config { channels: Channels::first(1), ratio: Rational::new(100, 1).unwrap() }, Config { channels: Channels::first(2), ratio: Rational::new(200, 1).unwrap() }];
        {
            let mut streams = lock(&core.streams);
            let mut line = streams.line(key).expect("the receive line").clone();
            line.plan = vec![
                Segment { origin: 0, cut: Some(1_000), config: line.stream.config, by: None },
                Segment { origin: ms(65), cut: None, config: configs[0], by: Some(1) },
                Segment { origin: ms(200), cut: None, config: configs[1], by: Some(2) },
            ];
            streams.lines.insert(key, line);
            streams.stream_mut(key).configs.insert(1, Arc::new(ColdConfig::new(BTreeMap::new())));
            streams.stream_mut(key).configs.insert(2, Arc::new(ColdConfig::new(BTreeMap::new())));
        }
        // The cut ends the stream at the untimed stop; the next segment configures alone, the
        // one after it reopens the streamer for its channel count.
        rx.poll();
        rx.receive(samples(clock.instant(0), 1_000));
        assert!(rx.stream.is_none());
        assert!(rx.begin());
        core.rx_chain[0].lock().request_end_now();
        rx.try_end();
        assert!(rx.stream.is_none());
        assert!(rx.begin());
        for name in ["rx_open", "rx_start", "rx_recv", "rx_stop", "apply"] {
            assert!(spy.seen(name) > 0, "the spy never saw {name}");
        }
        assert_eq!(spy.seen("rx_open"), 2, "the reopen of the two-channel segment");
    }

    #[test]
    fn ur_25_a_refused_receive_owner_books_and_gives_back_in_one_line_section() {
        // UR-25 (S2, receive): uhd-rx books the refusal while its claim still holds the
        // streamer, in the section that holds the line.
        let refuse = crate::device::FakeFault::WrongRate { claimed: 1e6, applied: 1e6 - 1_000.0, nth: 0 };
        let (core, device, _, _) = super::super::test_support::rig_with(crate::device::FakeConfig { faults: vec![refuse], ..Default::default() }, vec![link()]);
        let (mut rx, _) = rx(&core, 1);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        book(&core, ms(10), 1, Kind::Stop);
        book(&core, ms(20), 2, Kind::Start);
        rx.poll();
        rx.receive(samples(0, 10_000));
        assert!(rx.stream.is_none());

        let (entered, reached) = std::sync::mpsc::channel();
        let (resume, wait) = std::sync::mpsc::channel();
        let wait = std::sync::Mutex::new(Some(wait));
        let (seen, holders) = std::sync::mpsc::channel();
        super::super::test_support::probe(&core, "refused_booking", move |streams| {
            if let Some(wait) = wait.lock().unwrap().take() {
                let streams = streams.expect("the caller's section");
                let _ = seen.send(streams.holder(stream_key(Dir::Rx, 0)));
                let _ = entered.send(());
                let _ = wait.recv();
            }
        });
        let owner = std::thread::spawn(move || {
            let mut rx = rx;
            assert!(rx.begin(), "the owner handled the refusal");
        });
        reached.recv_timeout(Wall::from_secs(10)).expect("the owner reached its refusal book");
        assert_eq!(holders.recv().unwrap(), Holder::Owner, "the claim still holds the stream");
        assert!(core.streams.try_lock().is_err(), "the refusal is booked under the line lock");
        let _ = resume.send(());
        owner.join().expect("the receive owner");
        let streams = lock(&core.streams);
        assert_eq!(streams.holder(stream_key(Dir::Rx, 0)), Holder::Free);
        assert!(!streams.reading(stream_key(Dir::Rx, 0)));
        assert!(streams.line(stream_key(Dir::Rx, 0)).unwrap().items.iter().any(|item| item.seq == 2 && item.refused), "the refusal is booked");
        drop(streams);
        assert_eq!(device.calls().iter().filter(|call| call.starts_with("rx_start")).count(), 1, "only the initial start: {:?}", device.calls());
    }

    #[test]
    fn ur_24_a_missed_start_after_a_loss_makes_no_restart_call() {
        // UR-24, UR-29: a missed-start retry that finds the device lost issues no `rx_start`.
        let spy = Arc::new(super::super::test_support::LockSpy::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, ..Default::default() }));
        let (core, _time, _events) = super::super::test_support::rig_on_description(spy.clone(), vec![link()], BTreeMap::new(), super::super::test_support::default_description(2_000));
        spy.watch(&core);
        let _frozen = super::super::test_support::Frozen::on(spy.host());
        let (mut rx, _) = rx(&core, 1);
        core.device_lost("test: lost before the missed start is handled");
        let starts = spy.seen("rx_start");
        rx.receive(RxRecv::LateCommand);
        assert_eq!(spy.seen("rx_start"), starts, "a restart call after the loss: {:?}", spy.order());
    }

    #[test]
    fn ur_29_a_loss_between_two_channels_of_a_configure_applies_no_more() {
        // UR-29, UR-36: a segment's configuration applies channel by channel; a loss published
        // after the first channel's apply stops the second's.
        let spy = Arc::new(super::super::test_support::LockSpy::new(crate::device::FakeConfig::default()));
        let (core, _time, _events) = super::super::test_support::rig_on_description(spy.clone(), vec![link()], BTreeMap::new(), super::super::test_support::default_description(2_000));
        spy.watch(&core);
        let (mut rx, _) = rx_unopened(&core, 2);
        {
            let config = Config { channels: Channels::first(2), ratio: Rational::new(200, 1).unwrap() };
            let mut streams = lock(&core.streams);
            let key = stream_key(Dir::Rx, 0);
            streams.stream_mut(key).configs.insert(1, Arc::new(ColdConfig::new(core.description.defaults())));
            let mut line = streams.line(key).unwrap().clone();
            line.plan = vec![Segment { origin: core.ticks(50_000_000), cut: None, config, by: Some(1) }];
            streams.lines.insert(key, line);
        }
        let applies = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (seen, lost) = (applies.clone(), core.clone());
        super::super::test_support::probe(&core, "apply", move |_| {
            if seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1 {
                lost.device_lost("test: lost between two channels");
            }
        });
        rx.begin();
        let order = spy.order();
        let marked = order.iter().position(|call| *call == "mark_lost").expect("the loss was published");
        assert!(!order[marked + 1..].contains(&"apply"), "an apply after the loss: {order:?}");
    }

    #[test]
    fn ur_29_an_owner_does_not_configure_a_segment_a_stop_dropped_from_the_plan() {
        // UR-25, UR-36: a Stop booked between the owner's read of the plan and its configure
        // drops the not yet running segment; the owner makes no device call for it.
        let device = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig::default()));
        let dynamic: Arc<dyn Device> = device.clone();
        let (core, _time, _events) = super::super::test_support::rig_on(dynamic, vec![link()], 2_000);
        let (_rx, _) = rx_unopened(&core, 1);
        plan(&core, &[(core.ticks(50_000_000), None, Some(1))]);
        let next = lock(&core.streams).planned(stream_key(Dir::Rx, 0))[0].clone();
        book(&core, core.ticks(1_000_000), 2, Kind::Stop);
        assert!(lock(&core.streams).planned(stream_key(Dir::Rx, 0)).iter().all(|planned| planned.segment != next.segment), "the Stop dropped the segment");
        let mark = device.calls().len();
        assert!(matches!(core.owner_configure(Dir::Rx, 0, &next), Ok(false)));
        assert!(device.calls()[mark..].is_empty(), "{:?}", &device.calls()[mark..]);
    }

    // ------------------------------------------------------------ note 47 stage 3: the configure and the start lead (RM-25, UR-25)

    /// uhd-rx with the manual clock held, one segment planned at `origin` begun by the command 1, and the owner's configure taking until
    /// `completion` (the probe moves the Core's clock and the device's together, as a slow `apply` does). Returns what `begin` left.
    fn begin_with_configure_ending_at(origin_ms: i64, completion_ns: i64) -> (Arc<Core>, Arc<crate::device::FakeDevice>, Vec<ezsdr_kernel::event::Event>, i64) {
        let config = crate::device::FakeConfig { clock: crate::ClockMode::Manual, ..crate::device::FakeConfig::default() };
        let (core, device, time, events) = super::super::test_support::rig_with(config, vec![link()]);
        let _frozen = super::super::test_support::Frozen::of(&device);
        super::super::test_support::device_at(&core, &time, &device, 0);
        let (mut rx, _clock) = rx_unopened(&core, 1);
        let origin = core.ticks(origin_ms * 1_000_000);
        plan(&core, &[(origin, None, Some(1))]);
        let completion = core.ticks(completion_ns);
        let (probe_core, probe_device) = (core.clone(), device.clone());
        super::super::test_support::probe(&core, "apply", move |_| super::super::test_support::device_at(&probe_core, &time, &probe_device, completion));
        assert!(rx.begin());
        let emitted = events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0));
        (core, device, emitted, origin)
    }

    fn short_lead_events(events: &[ezsdr_kernel::event::Event]) -> Vec<&ezsdr_kernel::event::Event> {
        events.iter().filter(|event| event.kind.as_str() == kinds::START_LEAD_SHORT).collect()
    }

    #[test]
    fn ur_25_a_configure_that_leaves_less_than_the_declared_lead_reports_it_and_starts_at_the_origin() {
        // The declared command lead is 5 ms (2 ms device lead and 3 ms delivery allowance). The origin is 100 ms; the configure ends 4.5 ms before it.
        let (core, device, events, origin) = begin_with_configure_ending_at(100, 95_500_000);
        let short = short_lead_events(&events);
        assert_eq!(short.len(), 1, "{events:?}");
        let payload: ezsdr_radio::payloads::StartLeadShortPayload = serde_json::from_value(short[0].payload.clone()).unwrap();
        assert_eq!(payload.requested, core.at(origin));
        assert_eq!(payload.configured, core.at(core.ticks(95_500_000)));
        assert_eq!((payload.lead_left_ns, payload.min_lead_ns), (4_500_000, 5_000_000));
        assert_eq!(short[0].source, ezsdr_kernel::event::EventSource::Node { node: core.stream_node(Dir::Rx, 0) });
        assert_eq!(short[0].severity, Severity::Warning);
        // The start stays at the planned origin; nothing is late.
        assert!(device.calls().iter().any(|call| *call == format!("rx_start 0 {origin}")), "{:?}", device.calls());
        assert!(events.iter().all(|event| event.kind.as_str() != kinds::LATE_COMMAND));
        assert_eq!(lock(&core.streams).first_undelivered(stream_key(Dir::Rx, 0)), Some((origin, Config { channels: Channels::first(1), ratio: Rational::new(200, 1).unwrap() }, 0)));
    }

    #[test]
    fn ur_25_a_configure_that_leaves_the_declared_lead_whole_reports_nothing() {
        // Exactly the declared 5 ms before the origin, 25 ms before it (inside the 50 ms start lead), and the lattice-rounded origin itself.
        for completion in [95_000_000, 75_000_000, 0] {
            let (core, device, events, origin) = begin_with_configure_ending_at(100, completion);
            assert!(short_lead_events(&events).is_empty(), "completion {completion}: {events:?}");
            assert!(device.calls().iter().any(|call| *call == format!("rx_start 0 {origin}")));
            drop(core);
        }
    }

    /// What the fake device produces after a configure that ended at `completion_ns` for a segment planned at 100 ms: `begin`, then the clock at the
    /// origin, one receive turn (the fake's `LateCommand` is the owner's input, not injected), then the clock past the restart and a second turn that
    /// delivers its first block. Returns the events, the first block's header and the restart instant `rx_restart` recorded.
    fn begin_and_recover_from_a_missed_start(completion_ns: i64, faults: Vec<crate::device::FakeFault>) -> (Arc<Core>, Vec<ezsdr_kernel::event::Event>, ezsdr_kernel::stream::BlockHeader, i64, i64) {
        let link = link();
        let config = crate::device::FakeConfig { clock: crate::ClockMode::Manual, faults, ..crate::device::FakeConfig::default() };
        let (core, device, time, events) = super::super::test_support::rig_with(config, vec![link.clone()]);
        let _frozen = super::super::test_support::Frozen::of(&device);
        super::super::test_support::device_at(&core, &time, &device, 0);
        let (mut rx, _clock) = rx_unopened(&core, 1);
        let origin = core.ticks(100_000_000);
        plan(&core, &[(origin, None, Some(1))]);
        let completion = core.ticks(completion_ns);
        let (probe_core, probe_device, probe_time) = (core.clone(), device.clone(), time.clone());
        super::super::test_support::probe(&core, "apply", move |_| super::super::test_support::device_at(&probe_core, &probe_time, &probe_device, completion));
        assert!(rx.begin());
        // The device's time reaches the origin (5 ms past it or the configure's end, so a missing late report fails the restart lookup below instead of
        // hanging the turn), and the fake reports the timed command late.
        super::super::test_support::device_at(&core, &time, &device, origin.max(completion) + core.ticks(5_000_000));
        rx.turn();
        let restart = lock(&core.rec).timing.iter().find(|row| row["what"] == "rx_restart").expect("the owner restarted the missed start")["at"].as_i64().unwrap();
        super::super::test_support::device_at(&core, &time, &device, restart + core.ticks(5_000_000));
        rx.turn();
        let header = link.receive().expect("the restarted stream's first block").0.header().clone();
        let emitted = events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0));
        (core, emitted, header, origin, restart)
    }

    #[test]
    fn ur_25_a_configure_that_ends_after_the_origin_is_a_missed_start_reported_once() {
        // The configure ends 20 ms past the origin: no pre-start report (the device's own late report owns it), and the missed start restarts once;
        // the first block carries GAP_BEFORE with the samples between the origin and the restart, and the origin is unchanged.
        let (core, emitted, header, origin, restart) = begin_and_recover_from_a_missed_start(120_000_000, Vec::new());
        assert!(short_lead_events(&emitted).is_empty(), "a configure past the origin never reports a short lead: {emitted:?}");
        let late: Vec<_> = emitted.iter().filter(|event| event.kind.as_str() == kinds::LATE_COMMAND).collect();
        assert_eq!(late.len(), 1, "{emitted:?}");
        let payload: LateCommandPayload = serde_json::from_value(late[0].payload.clone()).unwrap();
        assert_eq!((payload.requested, payload.applied), (core.at(origin), core.at(restart)));
        assert!(restart >= core.ticks(120_000_000), "the restart waits the start lead after now");
        assert_eq!((header.flags, header.lost), (BlockFlags::GAP_BEFORE, Some(((restart - origin) / 200) as u64)));
    }

    #[test]
    fn ur_25_a_short_lead_and_a_missed_start_are_each_reported_once() {
        // The configure ends 4.5 ms before the origin (a short lead), and the fake reports that timed start late anyway (`LateStart`): one
        // START_LEAD_SHORT, then one LATE_COMMAND, the restart emitting no second short lead, and the first block carries the gap to the restart.
        let (core, emitted, header, origin, restart) = begin_and_recover_from_a_missed_start(95_500_000, vec![crate::device::FakeFault::LateStart]);
        let order: Vec<&str> = emitted.iter().map(|event| event.kind.as_str()).filter(|kind| *kind == kinds::START_LEAD_SHORT || *kind == kinds::LATE_COMMAND).collect();
        assert_eq!(order, [kinds::START_LEAD_SHORT, kinds::LATE_COMMAND], "{emitted:?}");
        assert_eq!(emitted.iter().find(|event| event.kind.as_str() == kinds::LATE_COMMAND).map(|event| event.payload["requested"].clone()), Some(serde_json::to_value(core.at(origin)).unwrap()));
        assert_eq!((header.flags, header.lost), (BlockFlags::GAP_BEFORE, Some(((restart - origin) / 200) as u64)));
    }

    #[test]
    fn ur_25_a_refused_start_is_labelled_as_a_command_with_the_start_s_id() {
        // The start's refusal names the `start_rx` Action by its id with kind `command`; a failed `rx_stop` is kind `stop`.
        let device = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig { faults: vec![crate::device::FakeFault::RxApplyFails], ..crate::device::FakeConfig::default() }));
        let dynamic: Arc<dyn Device> = device.clone();
        let (core, _time, events) = super::super::test_support::rig_on(dynamic, vec![link()], 2_000);
        let (mut rx, _) = rx_unopened(&core, 1);
        plan(&core, &[(core.ticks(50_000_000), None, Some(1))]);
        lock(&core.streams).stream_mut(stream_key(Dir::Rx, 0)).configs.insert(1, Arc::new(ColdConfig::of(core.description.defaults(), ezsdr_kernel::event::ActionId(41))));
        rx.begin();
        let refused: Vec<_> = events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == kinds::COMMAND_REJECTED).collect();
        assert_eq!(refused.len(), 1, "{refused:?}");
        assert_eq!((refused[0].payload["kind"].as_str(), &refused[0].payload["action"]), (Some("command"), &serde_json::json!(41)));
    }

    #[test]
    fn ur_25_a_booking_the_timeline_cannot_represent_is_refused_with_the_kind_of_its_action() {
        // RM-25: a start whose origin overflows the tick range is refused at its booking, and the refusal is of kind `command`, the booked Action's.
        let (core, _device, _time, events) = super::super::test_support::rig_with_links(vec![link()]);
        let (_rx, _) = rx_unopened(&core, 1);
        let key = stream_key(Dir::Rx, 0);
        let mut streams = lock(&core.streams);
        assert!(streams.book(&core, key, Item { e: 0, seq: 1, ready: 0, delivered: None, refused: false, kind: Kind::Stop }, None));
        assert!(!streams.book(&core, key, Item { e: i64::MAX, seq: 2, ready: 0, delivered: None, refused: false, kind: Kind::Start }, Some(ezsdr_kernel::event::ActionId(2))));
        drop(streams);
        let refused: Vec<_> = events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == kinds::COMMAND_REJECTED).collect();
        assert_eq!(refused.len(), 1, "{refused:?}");
        assert_eq!((refused[0].payload["kind"].as_str(), &refused[0].payload["action"]), (Some("command"), &serde_json::json!(2)), "the booked Action's kind and id");
        assert!(refused[0].payload["reason"].as_str().unwrap().starts_with("RM-25:"));
    }

    #[test]
    fn ur_36_owner_configure_takes_each_command_s_own_snapshot() {
        // UR-36: each segment's configure applies its own command's snapshot, not the values of the one before, and a command with the same
        // values as the one before still configures (the device may have been retuned by a refusal's partial apply).
        let device = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig::default()));
        let dynamic: Arc<dyn Device> = device.clone();
        let (core, _time, _events) = super::super::test_support::rig_on(dynamic, vec![link()], 2_000);
        let (_rx, _) = rx_unopened(&core, 1);
        plan(&core, &[(core.ticks(50_000_000), None, Some(1)), (core.ticks(150_000_000), None, Some(2)), (core.ticks(250_000_000), None, Some(3))]);
        let snapshot = |frequency: f64| {
            let mut config = core.description.defaults();
            config.insert(super::super::core::key("radio.rx.c0.frequency_hz"), ezsdr_kernel::spec::Value::num(frequency).unwrap());
            Arc::new(ColdConfig::new(config))
        };
        {
            let mut streams = lock(&core.streams);
            let stream = stream_key(Dir::Rx, 0);
            streams.stream_mut(stream).configs.insert(1, snapshot(2.0e9));
            streams.stream_mut(stream).configs.insert(2, snapshot(3.0e9));
            streams.stream_mut(stream).configs.insert(3, snapshot(3.0e9));
        }
        let planned = lock(&core.streams).planned(stream_key(Dir::Rx, 0));
        let applies = || device.calls().iter().filter(|call| call.starts_with("apply rx 0")).cloned().collect::<Vec<_>>();
        assert!(core.owner_configure(Dir::Rx, 0, &planned[0]).unwrap());
        assert_eq!(applies().len(), 1);
        assert!(applies()[0].contains("freq=2000000000"), "{:?}", applies());
        assert!(core.owner_configure(Dir::Rx, 0, &planned[1]).unwrap());
        assert_eq!(applies().len(), 2);
        assert!(applies()[1].contains("freq=3000000000"), "the new command's own snapshot: {:?}", applies());
        assert!(core.owner_configure(Dir::Rx, 0, &planned[2]).unwrap());
        assert_eq!(applies().len(), 3, "a command with the same values configures again: {:?}", device.calls());
    }
}
