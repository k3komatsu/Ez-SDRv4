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
use super::core::{stream_key, Clock, Core, Holder, Streams, lock};
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
    /// Samples at or after this root tick are discarded, and the stream ends there: it is
    /// stopped untimed when its samples reach the cut, since the X3x0 ignores a stop's
    /// time (UR-17; design-notes §11 F1).
    cut: Option<i64>,
    /// The device was told to stop (at the cut, or by an abort): not again.
    stopped: bool,
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
            cut: None,
            stopped: false,
            not_before: not_before.unwrap_or(i64::MIN),
        }
    }

    /// Keeps each distinct report until the next block; repeats of one kind coalesce.
    fn report(&mut self, pending: Pending) {
        if !self.pending.contains(&pending) {
            self.pending.push(pending);
        }
    }

    /// RM-16: the stream ends at `cut`, or earlier if it already ends there.
    fn cut_at(&mut self, cut: i64) {
        self.cut = Some(self.cut.map_or(cut, |set| set.min(cut)));
    }

    /// UR-17: the stream takes its cut from the plan, never before the first sample it has
    /// not delivered; one the plan no longer has was cut at its origin before its first
    /// sample, and its timed start may be queued, so it is stopped untimed only after that
    /// origin, what arrives discarded (UR-17; #56). Once `Provider::stop(abort)` has begun,
    /// the stream ends at the first sample not yet delivered, a segment still being delivered
    /// up to an earlier cut included (RM-16).
    fn follow(&mut self, streams: &Streams, stream: StreamKey) {
        let planned = streams.line(stream).and_then(|line| line.segment(self.clock.origin, self.config));
        if let Some(cut) = planned.map_or(Some(0), |segment| segment.cut) {
            self.cut_at(self.clock.instant(cut.max(self.expected)));
        }
        if matches!(streams.stop, Some((_, StopMode::Abort))) {
            self.cut_at(self.clock.instant(self.expected));
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
    exit: bool,
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
        let mut streams = lock(&core.streams);
        let active = stream.as_ref().and_then(|stream| streams.planned(key).into_iter()
            .find(|planned| planned.segment.origin == stream.clock.origin && planned.segment.config == stream.config)
            .map(|planned| (planned.segment.origin, planned.segment.config, planned.segment.by)));
        if let Some(stream) = stream.as_ref() {
            streams.started(key, active, stream.clock.origin, stream.config);
        }
        drop(streams);
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
            exit: false,
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

    /// RM-16, UR-17: the stream takes its cut from the plan; one cut at or before the first
    /// sample not yet delivered, once its origin has passed, has nothing more to deliver and
    /// is stopped and ended now — an abort's, or a `Stop` of a stream delivered past it.
    fn follow(&mut self) {
        let Some(stream) = self.stream.as_mut() else { return };
        stream.follow(&lock(&self.core.streams), stream_key(Dir::Rx, self.k));
        let done = stream.cut.is_some_and(|cut| stream.clock.at_or_after(cut) <= stream.expected) && self.core.now() >= stream.clock.origin;
        if done {
            if !stream.stopped {
                stream.stopped = true;
                self.last_stop = stop_at_cut(&self.core, self.k, stream.cut.expect("a cut")).or(self.last_stop);
            }
            self.finish();
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
        if self.exit && self.stream.is_none() {
            return Turn::Exit;
        }
        if self.core.is_lost() {
            // RM-16: the stream and its clock end at the loss's cut (`finish` reads the plan).
            self.finish();
            return if self.exit { Turn::Exit } else { Turn::Idle };
        }
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
        let result = self.core.device.rx_recv(self.k, self.recv_len(), self.recv_timeout());
        self.receive(result);
        Turn::Busy
    }

    fn command(&mut self, cmd: RxCmd) {
        let RxCmd::Shutdown(mode) = cmd;
        self.exit = true;
        if mode == StopMode::Abort {
            // RM-16: an abort ends the stream uhd-rx has at the first sample not yet delivered,
            // a segment still being delivered up to an earlier cut included.
            self.follow();
            if let Some(stream) = self.stream.as_mut() {
                stream.cut_at(stream.clock.instant(stream.expected));
                if !stream.stopped {
                    stream.stopped = true;
                    self.last_stop = stop_at_cut(&self.core, self.k, stream.cut.expect("set above")).or(self.last_stop);
                }
            }
            self.finish();
        }
    }

    /// The earliest origin of a planned segment not yet begun, as a test reads it (UR-25): an
    /// owner held behind an enable from no stream does not move it.
    #[cfg(test)]
    pub(super) fn cursor(&self) -> i64 {
        self.from
    }

    /// RM-25, UR-17: the next planned segment begins: the streamer reopened when its
    /// channel set changed, the direction configured with the configuration in effect at its
    /// origin — unless uhd-control did both for an enable from no stream — and a timed start
    /// issued there. A configuration the device refuses halts the stream: uhd-rx books the
    /// refusal, and the plan it then reads may begin the next segment earlier (RM-25).
    pub(super) fn begin(&mut self) -> bool {
        let key = stream_key(Dir::Rx, self.k);
        let next: Option<Planned> = {
            let mut streams = lock(&self.core.streams);
            if streams.holder(key) != Holder::Free {
                // UR-25: uhd-control's enable from no stream (or a move's release) owns the
                // streamer this turn; the plan is read again at the next, and the phase and
                // the cursor stand.
                return false;
            }
            let next = streams.planned(key).into_iter().find(|planned| planned.segment.origin >= self.from);
            streams.search(key, next.is_some());
            next
        };
        let Some(next) = next else {
            self.core.release_unneeded(Dir::Rx, self.k);
            return false;
        };
        let origin = next.segment.origin;
        // The record decides what is opened and configured (UR-13, UR-36).
        match self.core.owner_configure(Dir::Rx, self.k, &next) {
            Ok(true) => {}
            Ok(false) => return true,
            Err((error, claim)) => {
                // Not begun. A refusal the plan cannot take — its command pruned — skips the
                // segment. The refusal is booked while the claim still holds the stream, in
                // this one section, so no owner finds it free before the line holds it.
                let mut streams = lock(&self.core.streams);
                self.core.probe("refused_booking", Some(&streams));
                let refused = next.segment.by.is_some_and(|by| streams.refuse(&self.core, key, by));
                streams.give_back(claim);
                drop(streams);
                if !refused {
                    self.from = origin.saturating_add(1);
                }
                self.core.device_failed(None, "update_parameter", &error);
                return true;
            }
        }
        self.from = origin.saturating_add(1);
        // UR-29: another thread found the device lost while the configuration ran; no start
        // is issued and no segment begins.
        if self.core.is_lost() {
            return true;
        }
        let channels = next.set();
        if let Err(error) = self.core.device.rx_start(self.k, origin) {
            self.core.device_failed(None, "update_parameter", &error);
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
                streams.started(key, Some((origin, next.segment.config, next.segment.by)), origin, next.segment.config);
                drop(streams);
                self.last_samples = self.host.now();
                self.follow();
            }
            Err(error) => self.core.command_rejected(None, "update_parameter", &format!("UR-25: {error}")),
        }
        self.core.timing(json!({ "what": "rx_segment", "origin": origin, "channels": channels.count(), "at": self.core.now() }));
        true
    }

    /// The stream ends: its clock, if it has one, ends at its cut (RM-16, UR-17), and the
    /// items before the command that began it are pruned (#63). The cut is read from the
    /// plan here, since another thread may have booked an earlier one — a loss, or
    /// `Provider::stop`'s end — after this turn read it, while a receive call waited.
    fn finish(&mut self) {
        let Some(mut stream) = self.stream.take() else { return };
        let key = stream_key(Dir::Rx, self.k);
        let mut streams = lock(&self.core.streams);
        stream.follow(&streams, key);
        if let (None, Some(cut)) = (&stream.handle, stream.cut) {
            if let Err(error) = self.core.clocks.end(stream.clock.domain, self.core.at(cut)) {
                self.core.reject_note(json!({ "clock_not_ended": error.to_string() }));
            }
        }
        if let Some(active) = streams.finished(key) { streams.complete(key, active); }
        let by = streams.line(key).and_then(|line| line.segment(stream.clock.origin, stream.config)).and_then(|segment| segment.by);
        streams.prune(key, by);
    }

    /// How many samples to ask `rx_recv` for: while a cut is pending, no more than up to
    /// the cut, so that the last block ends at the cut and the stream is stopped about a
    /// delivery after it, whatever `block_len` (Review N, B1).
    fn recv_len(&self) -> usize {
        let block = self.core.block_len;
        let Some(stream) = self.stream.as_ref() else { return block };
        let Some(cut) = stream.cut.filter(|_| !stream.stopped) else { return block };
        (stream.clock.at_or_after(cut) - stream.expected).clamp(1, block as i64) as usize
    }

    /// How long `rx_recv` may wait: while a cut is pending, no longer than to the cut plus
    /// `quiet_span`, so that a stream that has stopped yielding is ended soon after its
    /// cut and not at the next 100 ms timeout (UR-17; design-notes §11 F2); never under
    /// 1 ms (UHD truncates the timeout to whole milliseconds).
    fn recv_timeout(&self) -> Wall {
        let Some(stream) = self.stream.as_ref() else { return RECV_TIMEOUT };
        let Some(cut) = stream.cut else { return RECV_TIMEOUT };
        let left = i128::from(cut) + i128::from(quiet_span(&self.core, stream)) - i128::from(self.core.now());
        let ns = (left.max(0) * 1_000_000_000 / i128::from(self.core.mcr))
            .clamp(1_000_000, RECV_TIMEOUT.as_nanos() as i128);
        Wall::from_nanos(ns as u64)
    }

    fn receive(&mut self, result: RxRecv) {
        let now = self.core.now();
        let Some(stream) = self.stream.as_mut() else { return };
        match result {
            RxRecv::Samples { first_tick, samples } => {
                self.last_samples = self.host.now();
                self.samples(first_tick, samples);
            }
            RxRecv::Timeout => {
                // Past the cut and its allowance, and silent for as long: a stream still
                // delivering, however late, is waited for (Review N, B2).
                let span = quiet_span(&self.core, stream);
                // The two unit tests that exercise this wait a 10 ms silence of the device's
                // host clock — about 3× the ≈ 4 ms `quiet_span` at 1 MS/s (a 2 000-sample
                // block plus the 3 ms delivery allowance). UR-29's lost-device monitor below
                // is the 1 s one; this quiet test is not.
                let quiet = self.host.now().saturating_sub(self.last_samples) >= Wall::from_nanos(self.core.ns(span).max(0) as u64);
                if let Some(cut) = stream.cut.filter(|cut| quiet && i128::from(now) >= i128::from(*cut) + i128::from(span)) {
                    // Silent past its cut: stopped all the same, never left streaming.
                    if !stream.stopped {
                        self.last_stop = stop_at_cut(&self.core, self.k, cut).or(self.last_stop);
                    }
                    self.finish();
                } else if stream.cut.is_none_or(|cut| now < cut)
                    && i128::from(now) > i128::from(stream.start) + i128::from(self.core.mcr)
                    && self.host.now().saturating_sub(self.last_samples) > Wall::from_secs(1)
                {
                    self.core.device_lost("UR-29: the receive stream yielded nothing for 1 s");
                }
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
            RxRecv::LateCommand if stream.cut.is_some() => {
                // A late start of a stream that is ending: stop it untimed, never restart
                // it (Review M, P1-A).
                self.core.timing(json!({ "what": "rx_stop_late", "cut": stream.cut, "at": now }));
                if !stream.stopped {
                    if let Some(issued) = stop_at_cut(&self.core, self.k, stream.cut.expect("a cut")) {
                        stream.stopped = true;
                        self.last_stop = Some(issued);
                    }
                }
            }
            RxRecv::LateCommand => {
                // UR-17: the device missed the timed start; the origin stays. The new start
                // waits the profile's start lead from now (spec 20, VF-6).
                let restart = match ezsdr_radio::timeline::lattice(now.saturating_add(self.core.ticks(ezsdr_radio::timeline::start_lead_ns(&self.core.description.timing, Direction::Rx))), stream.config.ratio) {
                    Ok(restart) => restart,
                    Err(error) => return self.core.command_rejected(None, "start", &format!("UR-17: {error}")),
                };
                let payload = serde_json::to_value(LateCommandPayload {
                    key: None,
                    requested: self.core.at(stream.start),
                    applied: self.core.at(restart),
                })
                .expect("a payload");
                stream.report(Pending::MissedStart);
                self.core.emit(&self.core.stream_node(Dir::Rx, self.k), kinds::LATE_COMMAND, Severity::Warning, payload);
                self.core.timing(json!({ "what": "rx_restart", "requested": stream.start, "at": restart }));
                if let Err(error) = self.core.device.rx_start(self.k, restart) {
                    self.core.device_failed(None, "start", &error);
                }
            }
            RxRecv::Failed(error) if error.lost => self.core.device_lost(&error.message),
            RxRecv::Failed(error) => {
                self.core.stat("rx_errors", 1);
                self.core.timing(json!({ "what": "rx_error", "error": error.message }));
            }
        }
    }

    fn samples(&mut self, first_tick: i64, mut samples: Vec<Vec<Iq>>) {
        let stream = self.stream.as_mut().expect("a stream");
        let clock = stream.clock;
        if first_tick < clock.origin || first_tick < stream.not_before {
            // The old stream's samples still in flight after its stop (Review M, N-3;
            // Review N, B1: after a late switch they can follow the new origin).
            self.core.stat("rx_before_origin", 1);
            return;
        }
        let offset = first_tick - clock.origin;
        if offset.rem_euclid(clock.n) != 0 {
            self.core.stat("rx_off_lattice", 1);
        }
        let mut k = (offset + clock.n / 2).div_euclid(clock.n);
        let mut len = samples.first().map_or(0, Vec::len) as i64;
        let mut ended = false;
        {
            // The cut and what this block delivers are read and written together, so that a
            // `Stop` booked after them cuts after the block (RM-16's floor, `Item.delivered`).
            let mut streams = lock(&self.core.streams);
            stream.follow(&streams, stream_key(Dir::Rx, self.k));
            if let Some(cut_k) = stream.cut.map(|cut| clock.at_or_after(cut)).filter(|cut_k| k + len >= *cut_k) {
                len = (cut_k - k).max(0);
                ended = true;
            }
            if k + len > stream.expected {
                streams.delivered(stream_key(Dir::Rx, self.k), clock.origin, stream.config, k + len);
            }
        }
        if let Some(cut) = stream.cut.filter(|_| ended && !stream.stopped) {
            // UR-25: the samples reached the cut, so the stream is stopped untimed now.
            stream.stopped = true;
            self.last_stop = stop_at_cut(&self.core, self.k, cut).or(self.last_stop);
        }
        if k < stream.expected {
            let trim = stream.expected - k;
            if trim >= len {
                self.core.stat("rx_overlapping", 1);
                if ended {
                    self.finish();
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
            let reports: Vec<_> = pending.iter().filter_map(|pending| match pending {
                Pending::Overrun => Some(RxOverflowCause::Overrun),
                Pending::Sequence => Some(RxOverflowCause::Sequence),
                Pending::Alignment | Pending::MissedStart => None,
            }).collect();
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
            self.finish();
        }
    }
}

/// How long a stream past its cut must have been silent to be taken as ended: a block and
/// the delivery allowance, so that a slow link's last samples before the cut are waited
/// for (Review N, B2).
fn quiet_span(core: &Core, stream: &Stream) -> i64 {
    core.block_len as i64 * stream.clock.n + core.ticks(DELIVERY_ALLOWANCE_NS)
}

/// Stops the stream untimed once its samples reached the cut (or at an abort's), and
/// records it (UR-25, UR-30): the X3x0 carries out a timed stop at once, before its time
/// (§11 F1). Returns the instant it was issued.
fn stop_at_cut(core: &Core, k: usize, cut: i64) -> Option<i64> {
    if core.is_lost() {
        return None;
    }
    let now = core.now();
    core.timing(json!({ "what": "rx_stop_untimed", "cut": cut, "at": now }));
    match core.device.rx_stop(k, None) {
        Ok(()) => {
            lock(&core.rec).applied.push(json!({ "key": "rx_stop", "at": core.at(cut), "issued": core.at(now) }));
            // The device produces samples until the stop reaches it (the bench: ~0.4 ms
            // into the call): the next stream drops anything before the call's return
            // plus a device lead (Review O, N-3); it starts a restart lead later anyway.
            Some(core.now() + core.ticks(DEVICE_LEAD_NS))
        }
        Err(error) => {
            core.device_failed(None, "update_parameter", &error);
            None
        }
    }
}

#[cfg(test)]
mod differential;

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::control::ColdConfig;
    use crate::device::Device;
    use ezsdr_kernel::module_api::Link;
    use ezsdr_kernel::stream::BackPressure;
    use ezsdr_radio::payloads::RxOverflowPayload;
    use ezsdr_radio::timeline::{Item, Kind, Line, Segment, NO_RELEASE};
    use std::collections::BTreeMap;
    use super::super::core::Phase;

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
        let (rx, clock) = rx_unopened(core, channels);
        core.device.rx_open(0, &(0..channels).collect::<Vec<usize>>()).unwrap();
        // The record of an open that arm made (UR-13).
        lock(&core.streams).record_open(stream_key(Dir::Rx, 0), Some(Channels::first(channels as u32)));
        (rx, clock)
    }

    /// The receive side of `rx`, its streamer not opened yet.
    fn rx_unopened(core: &Arc<Core>, channels: usize) -> (Rx, Clock) {
        let stream = core.id.child("rx").unwrap();
        let handle = core.clocks.declare_sample_clock(stream, core.root, Rational::new(200, 1).unwrap()).unwrap();
        let clock = Clock { domain: handle.id(), origin: 0, n: 200 };
        let config = Config { channels: Channels::first(channels as u32), ratio: Rational::new(200, 1).unwrap() };
        lock(&core.streams).lines.insert(stream_key(Dir::Rx, 0), Line::new(ezsdr_radio::timeline::Stream {
            direction: Direction::Rx,
            origin: 0,
            config,
            lead: core.ticks(ezsdr_radio::timeline::start_lead_ns(&core.description.timing, Direction::Rx)),
            call: core.block_len as i64,
            allowance: core.ticks(DELIVERY_ALLOWANCE_NS),
        }));
        let (to_rx, cmds) = std::sync::mpsc::channel();
        // No `Provider::stop` comes unless a test sends it.
        std::mem::forget(to_rx);
        (Rx::new(core.clone(), 0, cmds, Some((handle, Channels::first(channels as u32), 0)), None), clock)
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

    /// Books a receive command as uhd-control does, its segment's configuration with it.
    fn book(core: &Core, e: i64, seq: u64, kind: Kind) {
        let key = stream_key(Dir::Rx, 0);
        let mut streams = lock(&core.streams);
        streams.stream_mut(key).configs.insert(seq, Arc::new(ColdConfig::new(BTreeMap::new())));
        assert!(streams.book(core, key, Item { e, seq, ready: 0, release: NO_RELEASE, delivered: None, refused: false, kind }));
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
        assert!(events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter()
            .any(|event| event.kind.as_str() == ezsdr_kernel::event::EventKind::DEVICE_LOST));
    }

    #[test]
    fn ur_34_a_waited_receive_open_exits_when_stop_removes_its_segment() {
        let (core, device, _time, events) = super::super::test_support::rig_with_links(vec![link()]);
        let from = stream_key(Dir::Rx, 1);
        let to = stream_key(Dir::Rx, 0);
        let ratio = Rational::new(200, 1).unwrap();
        let line = |channels| Line::new(ezsdr_radio::timeline::Stream {
            direction: Direction::Rx,
            origin: 0,
            config: Config { channels, ratio },
            lead: 0,
            call: 0,
            allowance: 0,
        });
        device.rx_open(1, &[1]).unwrap();
        {
            let mut streams = lock(&core.streams);
            *streams = Streams::with_streams(2, 1);
            streams.lines.insert(from, line(Channels::first(2).without(0)));
            streams.lines.insert(to, line(Channels::first(1)));
            streams.stream_mut(from).set = super::super::core::DeviceSet::Open(Channels::first(2).without(0));
            streams.seq = 1;
            streams.lines.book_move(from, to, 1, Some(10_000), 0, 1, 0).unwrap();
            streams.changed(&core, from);
            streams.changed(&core, to);
        }
        let next = lock(&core.streams).planned(to).into_iter().find(|planned| planned.segment.by == Some(1)).unwrap();
        let (to_rx, cmds) = std::sync::mpsc::channel();
        std::mem::forget(to_rx);
        let mut rx = Rx::new(core.clone(), 0, cmds, None, None);
        rx.from = next.segment.origin;
        let from_cursor = rx.cursor();
        let (entered, reached) = std::sync::mpsc::channel();
        super::super::test_support::probe(&core, "wait_release", move |_| { let _ = entered.send(()); });
        let (done, finished) = std::sync::mpsc::channel();
        let owner = core.device.host().spawn("rx-wait-stop", move || {
            let began = rx.begin();
            let _ = done.send((began, rx.cursor()));
        }).unwrap();
        assert!(reached.recv_timeout(Wall::from_secs(10)).is_ok(), "the owner reached its wait");
        {
            let mut streams = lock(&core.streams);
            let seq = streams.next_seq();
            assert!(streams.book(&core, to, Item { e: 0, seq, ready: 0, release: NO_RELEASE, delivered: None, refused: false, kind: Kind::Stop }));
        }
        owner.join().unwrap();
        let (began, cursor) = finished.recv_timeout(Wall::from_secs(10)).unwrap();
        assert!(began, "the owner handled the planned turn");
        assert_eq!(cursor, from_cursor, "an interrupted wait did not advance RX `from`");
        assert!(!device.calls().iter().any(|call| call == "rx_open 0 [0, 1]"), "the removed segment was not opened: {:?}", device.calls());
        assert_eq!(lock(&core.streams).holder(to), Holder::Free);
        assert!(!lock(&core.streams).line(to).unwrap().items.iter().any(|item| item.seq == 1 && item.refused), "Stop is not a device refusal");
        assert!(!events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter()
            .any(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED), "Stop interruption emits no refusal");
    }

    fn rate(rate: u64) -> Kind {
        Kind::Cold(Config { channels: Channels::first(1), ratio: Rational::new(200_000_000 / rate, 1).unwrap() })
    }

    fn samples(first_tick: i64, n: usize) -> RxRecv {
        RxRecv::Samples { first_tick, samples: vec![vec![[0.0, 0.0]; n]] }
    }

    fn received(link: &Arc<dyn ezsdr_kernel::stream::DataLink>) -> Vec<(ClockDomainId, i64, u32)> {
        std::iter::from_fn(|| link.receive().map(|(b, _)| b)).map(|block| {
            let t = block.header().first_sample_time;
            (t.domain(), t.ticks_in(t.domain()).unwrap(), block.header().len) // own domain: each block's clock is returned beside its ticks
        }).collect()
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
        lock(&core.streams).lines.insert(stream_key(Dir::Rx, 1), Line::new(ezsdr_radio::timeline::Stream {
            direction: Direction::Rx,
            origin: 0,
            config: Config { channels: Channels::first(1), ratio: Rational::new(200, 1).unwrap() },
            lead: 0,
            call: 2_000,
            allowance: 0,
        }));
        let (to_rx, cmds) = std::sync::mpsc::channel();
        std::mem::forget(to_rx);
        let mut rx = Rx::new(core.clone(), 1, cmds, Some((handle, Channels::first(1), 0)), None);
        let clock = rx.stream.as_ref().unwrap().clock;
        rx.receive(RxRecv::Overflow { out_of_sequence: false });
        rx.receive(samples(clock.instant(2), 2));
        let block = link.receive().expect("the block after the overflow").0;
        assert_eq!(block.header().flags, BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED);
        let got: Vec<_> = events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
            .filter(|event| event.kind.as_str() == kinds::RX_OVERFLOW)
            .collect();
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
            (vec![RxRecv::Alignment, RxRecv::Overflow { out_of_sequence: true }, RxRecv::Alignment], BlockFlags::GAP_BEFORE | BlockFlags::SEQ_DISCONTINUITY | BlockFlags::ALIGNMENT, Some(RxOverflowCause::Sequence), true),
            (vec![RxRecv::Overflow { out_of_sequence: false }, RxRecv::Alignment], BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED | BlockFlags::ALIGNMENT, Some(RxOverflowCause::Overrun), true),
            (vec![RxRecv::Overflow { out_of_sequence: false }, RxRecv::Overflow { out_of_sequence: true }], BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED | BlockFlags::SEQ_DISCONTINUITY, Some(RxOverflowCause::Overrun), false),
            (vec![RxRecv::Overflow { out_of_sequence: false }, RxRecv::Overflow { out_of_sequence: false }], BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED, Some(RxOverflowCause::Overrun), false),
            (vec![RxRecv::Overflow { out_of_sequence: true }, RxRecv::Overflow { out_of_sequence: true }], BlockFlags::GAP_BEFORE | BlockFlags::SEQ_DISCONTINUITY, Some(RxOverflowCause::Sequence), false),
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
            let got: Vec<_> = events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
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
            if alignment { assert_eq!(alignments[0].payload["lost"], 2); }
        }
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
        assert_eq!(RxOverflowPayload::from_payload(&overflows[0].payload).unwrap(), RxOverflowPayload {
            cause: RxOverflowCause::Overrun,
            lost: block.header().lost.unwrap(),
        });
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
        assert_eq!(events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0))
            .iter().filter(|event| event.kind.as_str() == kinds::RX_OVERFLOW).count(), 0);
    }

    #[test]
    fn ur_29_a_loss_after_a_configure_starts_no_receive_segment() {
        // UR-29 (L1): the device is found lost while the owner's configuration is in
        // flight; the owner issues no timed start and registers no clock.
        let inner = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig::default()));
        let holder = Arc::new(super::super::test_support::HoldApply::new(inner.clone(), Dir::Rx, Some(0)));
        let (core, _time, _events) = super::super::test_support::rig_on(holder.clone(), vec![link()], 2_000);
        let (mut rx, _clock) = rx_unopened(&core, 1);
        book(&core, core.ticks(50_000_000), 1, rate(1_000_000));
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
        assert_eq!(streams.phase(stream_key(Dir::Rx, 0)), Phase::Pending, "a lost configure leaves the owner pending");
        assert!(streams.active(stream_key(Dir::Rx, 0)).is_none());
        assert_eq!(streams.first_undelivered(stream_key(Dir::Rx, 0)), None);
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
        // A stream with no channel before the change, so that the plan's one segment is the
        // change's, at `origin`.
        lock(&core.streams).lines.insert(stream_key(Dir::Rx, 0), Line::new(ezsdr_radio::timeline::Stream {
            direction: Direction::Rx,
            origin: 0,
            config: Config { channels: Channels::first(0), ratio: Rational::new(200, 1).unwrap() },
            lead: 0,
            call: 2_000,
            allowance: 0,
        }));
        book(&core, origin, 1, rate(1_000_000));
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
            assert_eq!(streams.phase(stream_key(Dir::Rx, 0)), Phase::Pending, "the ended segment leaves the owner pending");
            assert_eq!(streams.first_undelivered(stream_key(Dir::Rx, 0)), None);
        }
        running.join().expect("the owner's begin thread");
    }

    #[test]
    fn ur_26_a_stream_s_delivered_is_cleared_alone() {
        // RM-16 (S12c): a stream's `finish` clears its own delivered floor; the other
        // stream's stands for the end that comes later.
        let (core, _device, _time, _events) = super::super::test_support::rig_with_links_on(1, vec![link()], layout_description());
        let config = Config { channels: Channels::first(1), ratio: Rational::new(200, 1).unwrap() };
        {
            let mut streams = lock(&core.streams);
            streams.started(stream_key(Dir::Rx, 0), Some((0, config, None)), 0, config);
            streams.delivered(stream_key(Dir::Rx, 0), 0, config, 700);
        }
        let handle = core.clocks.declare_sample_clock(core.stream_node(Dir::Rx, 1), core.root, Rational::new(200, 1).unwrap()).unwrap();
        lock(&core.streams).lines.insert(stream_key(Dir::Rx, 1), Line::new(ezsdr_radio::timeline::Stream {
            direction: Direction::Rx,
            origin: 0,
            config: Config { channels: Channels::first(1), ratio: Rational::new(200, 1).unwrap() },
            lead: 0,
            call: 2_000,
            allowance: 0,
        }));
        let (to_rx, cmds) = std::sync::mpsc::channel();
        std::mem::forget(to_rx);
        let mut rx = Rx::new(core.clone(), 1, cmds, Some((handle, Channels::first(1), 0)), None);
        {
            let streams = lock(&core.streams);
            assert_eq!(streams.phase(stream_key(Dir::Rx, 1)), Phase::Active {
                segment: Some((0, config, None)),
                delivered: Some((0, config, 0)),
            }, "the T0 stream begins at its planned segment");
            assert_eq!(streams.first_undelivered(stream_key(Dir::Rx, 0)), Some((0, config, 700)), "stream 0's floor stands");
        }
        rx.finish();
        let streams = lock(&core.streams);
        assert_eq!(streams.first_undelivered(stream_key(Dir::Rx, 0)), Some((0, config, 700)), "stream 0's floor stands");
        assert_eq!(streams.first_undelivered(stream_key(Dir::Rx, 1)), None, "stream 1's floor is cleared");
        assert_eq!(streams.phase(stream_key(Dir::Rx, 1)), Phase::Pending);
    }

    #[test]
    fn ur_26_a_receive_start_of_no_planned_segment() {
        // RM-16 (P1b): a loss between the start and uhd-rx's creation of its T0 stream removes
        // the segment from the plan. uhd-rx still begins the stream: its phase is active with
        // no segment and the delivered tuple it started with; finishing records nothing, and
        // the loss's end is untouched.
        let (core, _device, _time, _events) = super::super::test_support::rig_with_links(vec![link()]);
        let key = stream_key(Dir::Rx, 0);
        let config = Config { channels: Channels::first(1), ratio: Rational::new(200, 1).unwrap() };
        let t0 = core.ticks(10_000_000);
        let loss = core.ticks(5_000_000);
        let node = core.stream_node(Dir::Rx, 0);
        let handle = core.clocks.declare_sample_clock(node, core.root, config.ratio).unwrap();
        {
            let mut streams = lock(&core.streams);
            streams.lines.insert(key, Line::new(ezsdr_radio::timeline::Stream {
                direction: Direction::Rx,
                origin: t0,
                config,
                lead: 0,
                call: 2_000,
                allowance: 0,
            }));
            streams.end(&core, loss, false, false);
            assert!(streams.planned(key).is_empty(), "the loss removed the T0 segment");
        }
        let (to_rx, cmds) = std::sync::mpsc::channel();
        std::mem::forget(to_rx);
        let mut rx = Rx::new(core.clone(), 0, cmds, Some((handle, Channels::first(1), t0)), None);
        {
            let streams = lock(&core.streams);
            assert_eq!(streams.phase(key), Phase::Active { segment: None, delivered: Some((t0, config, 0)) });
            assert_eq!(streams.first_undelivered(key), Some((t0, config, 0)), "HEAD keeps the tuple");
        }
        rx.finish();
        let streams = lock(&core.streams);
        assert_eq!(streams.phase(key), Phase::Pending);
        assert_eq!(streams.first_undelivered(key), None);
        assert!(streams.stream(key).completed.is_empty(), "no segment to record");
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
            .map(|block| (block.header().first_sample_time.ticks_in(clock.domain).unwrap(), block.header().len, block.header().flags, block.header().lost)).collect();
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
        assert!(!device.calls().iter().any(|c| c.starts_with("rx_start ")));
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
        assert!(!device.calls().iter().any(|c| c.starts_with("rx_start ")));
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
        for (mode, delivered, end, stop) in [
            (StopMode::Orderly, 300, 1_000, true),
            (StopMode::Abort, 1_000, 1_000, true),
            (StopMode::Abort, 300, 300, true),
            (StopMode::Abort, 300, 300, false),
        ] {
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
            book(&core, core.ticks(10_000_000), 1, rate(2_000_000));
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
                book(&core, core.ticks(10_000_000), 1, rate(2_000_000));
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
            book(&core, ms(10), 1, rate(2_000_000));
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
        book(&core, ms(10), 1, rate(2_000_000));
        time.advance_to(core.at(ms(10))).unwrap();
        core.device_lost("test");
        let planned: Vec<_> = lock(&core.streams).planned(stream_key(Dir::Rx, 0)).iter().map(|p| (p.segment.origin, p.segment.cut)).collect();
        assert_eq!(planned, [(0, Some(12_000))]);
    }

    #[test]
    fn ur_26_an_end_takes_delivered_only_from_the_segment_it_cuts() {
        // RM-16: an end carries the first sample uhd-rx has not delivered only when uhd-rx's
        // segment still runs at the end. A change at 10 ms cuts the stream at sample 10 000;
        // an abort at 12 ms, with 9 000 delivered, leaves that cut and plans no segment after
        // it — not the next segment cut at the previous one's sample index.
        let (core, _, time, _) = super::super::test_support::rig_with_links(vec![link()]);
        let (mut rx, clock) = rx(&core, 1);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        book(&core, ms(10), 1, rate(2_000_000));
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
        // RM-16: with its command channel gone and no `Provider::stop` booked, uhd-rx aborts:
        // the stream ends at the first sample not yet delivered, its clock there, stopped once.
        let (core, device, _, _) = super::super::test_support::rig_with_links(vec![link()]);
        let (mut rx, clock) = rx(&core, 1);
        rx.receive(samples(clock.instant(0), 300));
        let (to_rx, cmds) = std::sync::mpsc::channel();
        drop(to_rx);
        rx.cmds = cmds;
        rx.poll();
        assert!(rx.exit && rx.stream.is_none());
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
        let (core, device, time, _) = super::super::test_support::rig_with_links(vec![link()]);
        let (mut rx, clock) = rx(&core, 1);
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
        // The stream went quiet more than its span ago: the receive call's own wait is the
        // bounded one, and its timeout finds the stream past cut + span. A longer wait here
        // only strengthens the quiet check, and the wait's own value is what is asserted.
        rx.last_samples = rx.host.now();
        rx.host.sleep(Wall::from_millis(6));
        time.advance_to(core.at(ms(15))).unwrap();
        // Past the cut plus its span the wait floors at 1 ms (UHD truncates the timeout to
        // whole milliseconds), and the receive call is given exactly that.
        assert_eq!(rx.recv_timeout(), Wall::from_millis(1), "the wait floors at 1 ms");
        rx.turn();
        assert_eq!(device.min_recv_timeout(), Wall::from_millis(1), "the receive call was given the bounded wait: {:?}", device.min_recv_timeout());
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
        book(&core, ms(20), 2, rate(1_000_000));
        book(&core, ms(40), 3, Kind::Start);
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
        let ms = |n: i64| core.ticks(n * 1_000_000);
        {
            let key = stream_key(Dir::Rx, 0);
            let mut streams = lock(&core.streams);
            streams.stream_mut(key).configs.insert(2, Arc::new(ColdConfig::new(BTreeMap::new())));
            let kind = Kind::Cold(Config { channels: Channels::first(2), ratio: Rational::new(200, 1).unwrap() });
            assert!(streams.book(&core, key, Item { e: ms(20), seq: 2, ready: 0, release: NO_RELEASE, delivered: None, refused: false, kind }));
        }
        book(&core, ms(40), 3, Kind::Start);
        assert!(rx.begin(), "the partly applied configure is reported");
        assert!(lock(&core.streams).stream(stream_key(Dir::Rx, 0)).configured.is_none());
        assert!(rx.begin());
        let calls = inner.calls();
        assert_eq!(calls.iter().filter(|call| call.starts_with("rx_open")).count(), 1, "{calls:?}");
        assert_eq!(calls.iter().filter(|call| call.starts_with("apply rx 0")).count(), 2, "the next segment configures the first channel again: {calls:?}");
        assert_eq!(calls.iter().filter(|call| call.starts_with("apply rx 1")).count(), 1, "and the channel the device refused: {calls:?}");
        assert!(lock(&core.streams).stream(stream_key(Dir::Rx, 0)).configured.is_some_and(|by| by == 3), "{calls:?}");
    }

    #[test]
    fn ur_13_a_failed_configure_is_configured_again_without_reopen() {
        // UR-13 (S6, the owner's side): a configure that fails leaves the streamer open; the
        // next segment configures it without a second open.
        let config = crate::device::FakeConfig { faults: vec![crate::device::FakeFault::RxApplyFails], ..Default::default() };
        let (core, device, _, _) = super::super::test_support::rig_with(config, vec![link()]);
        let (mut rx, _) = rx_unopened(&core, 1);
        rx.stream = None;
        lock(&core.streams).record_open(stream_key(Dir::Rx, 0), Some(Channels::first(1)));
        let ms = |n: i64| core.ticks(n * 1_000_000);
        book(&core, ms(20), 2, rate(1_000_000));
        book(&core, ms(40), 3, Kind::Start);
        assert!(rx.begin(), "the failed configure is reported");
        assert!(rx.begin());
        let calls = device.calls();
        assert_eq!(calls.iter().filter(|call| call.starts_with("rx_open")).count(), 0, "{calls:?}");
        assert_eq!(calls.iter().filter(|call| call.starts_with("apply")).count(), 2, "{calls:?}");
    }

    #[test]
    fn ur_25_a_refused_segment_is_booked_by_uhd_rx() {
        // RM-25: when the device refuses a segment's configuration, uhd-rx books the change as
        // refused itself, which halts the stream, and reads the plan made so at once: a
        // `start_rx` that was a no-op resumes the stream, at the refused segment's own origin,
        // which the refusal does not count as begun.
        let refuse = crate::device::FakeFault::WrongRate { claimed: 2e6, applied: 2e6 - 1_000.0, nth: 0 };
        let (core, device, _, _) = super::super::test_support::rig_with(crate::device::FakeConfig { faults: vec![refuse], ..Default::default() }, vec![link()]);
        let (mut rx, _) = rx(&core, 1);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        book(&core, ms(10), 1, rate(2_000_000));
        book(&core, ms(20), 2, Kind::Start);
        assert_eq!(lock(&core.streams).planned(stream_key(Dir::Rx, 0)).iter().map(|p| (p.segment.origin, p.segment.by)).collect::<Vec<_>>(), [(0, None), (ms(65), Some(1))]);
        rx.poll();
        rx.receive(samples(0, 10_000));
        assert!(rx.stream.is_none());
        {
            let streams = lock(&core.streams);
            assert_eq!(streams.phase(stream_key(Dir::Rx, 0)), Phase::Pending, "a refused segment leaves the owner pending");
            assert!(streams.active(stream_key(Dir::Rx, 0)).is_none());
            assert_eq!(streams.first_undelivered(stream_key(Dir::Rx, 0)), None);
        }
        assert!(rx.begin());
        assert!(lock(&core.streams).line(stream_key(Dir::Rx, 0)).unwrap().items.iter().any(|item| item.seq == 1 && item.refused));
        assert!(rx.begin());
        let starts: Vec<_> = device.calls().into_iter().filter(|c| c.starts_with("rx_start ")).collect();
        assert_eq!(starts, [format!("rx_start 0 {}", ms(65))]);
        assert_eq!(rx.stream.as_ref().map(|stream| stream.config.ratio.num()), Some(100), "the refused rate is the stream's (KC-27)");
        let streams = lock(&core.streams);
        let active = streams.active(stream_key(Dir::Rx, 0)).expect("the resumed segment");
        assert_eq!((active.0, active.2), (ms(65), Some(2)), "the command that began the resumed segment is the Start's");
        assert_eq!(active.1.ratio.num(), 100);
        assert_eq!(streams.first_undelivered(stream_key(Dir::Rx, 0)), Some((ms(65), active.1, 0)));
    }

    #[test]
    fn ur_17_an_ended_segment_prunes_the_items_before_it() {
        // #63: when uhd-rx ends a segment, the timeline's items before the command that began
        // it are pruned, with their segments' configurations: after a change at 10 ms, one at
        // 200 ms and a `Stop` at 300 ms, the first change goes once the second's segment ends.
        let (core, _, _, _) = super::super::test_support::rig_with_links(vec![link()]);
        let (mut rx, _) = rx(&core, 1);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        book(&core, ms(10), 1, rate(2_000_000));
        book(&core, ms(200), 2, rate(1_000_000));
        book(&core, ms(300), 3, Kind::Stop);
        rx.poll();
        rx.receive(samples(0, 10_000));
        for (origin, len) in [(ms(65), 270_000), (ms(254), 46_000)] {
            assert!(rx.begin());
            rx.receive(samples(origin, len));
            assert!(rx.stream.is_none(), "the segment from {origin} ends at its cut");
        }
        let streams = lock(&core.streams);
        let items: Vec<u64> = streams.line(stream_key(Dir::Rx, 0)).unwrap().items.iter().map(|item| item.seq).collect();
        assert_eq!(items, [2, 3]);
        assert_eq!(streams.stream(stream_key(Dir::Rx, 0)).configs.keys().copied().collect::<Vec<_>>(), [2, 3]);
        assert_eq!(streams.planned(stream_key(Dir::Rx, 0)).len(), 3, "every segment stays in the plan");
    }

    #[test]
    fn ur_26_no_segment_begins_once_provider_stop_has_begun() {
        // UR-25, UR-26 (spec 22, VH-4; #58): `Provider::stop` ends the stream in the plan, so
        // uhd-rx starts no planned segment after it, the one a change booked before it included.
        let (core, device, _, _) = super::super::test_support::rig_with_links(vec![link()]);
        let (mut rx, _) = rx(&core, 1);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        book(&core, ms(10), 1, rate(2_000_000));
        lock(&core.streams).end(&core, ms(5), false, false);
        rx.poll();
        rx.receive(samples(0, 10_000));
        assert!(rx.stream.is_none());
        assert!(!rx.begin());
        assert!(!device.calls().iter().any(|c| c.starts_with("rx_start ")), "{:?}", device.calls());
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

    /// What a stream's line holds, for comparing it across another stream's change: its
    /// items (`e`, `seq`, `refused`), its plan (`origin`, `cut`, `channels`, `by`) and its
    /// clocks (`origin`, `ended`).
    type Held = (Vec<(i64, u64, bool)>, Vec<(i64, Option<i64>, Vec<u32>, Option<u64>)>, Vec<(i64, bool)>);

    /// The transmit stream's line, as `Control::new` makes it for a transmit stream with one
    /// channel at 1 MS/s.
    fn tx_line(core: &Core) -> Line {
        Line::new(ezsdr_radio::timeline::Stream {
            direction: Direction::Tx,
            origin: 0,
            config: Config { channels: Channels::first(1), ratio: Rational::new(200, 1).unwrap() },
            lead: core.ticks(ezsdr_radio::timeline::start_lead_ns(&core.description.timing, Direction::Tx)),
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
            streams.line(stream).expect("the line").items.iter().filter_map(|item| match item.kind {
                Kind::End { abort } => Some((item.e, item.seq, abort)),
                _ => None,
            }).collect()
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
    fn ur_25_a_refusal_leaves_the_transmit_stream_alone() {
        // RM-25 (design 33 D15): a configuration uhd-rx's device refuses halts the receive
        // stream alone: the transmit stream's items, plan and clocks are untouched.
        let refuse = crate::device::FakeFault::WrongRate { claimed: 2e6, applied: 2e6 - 1_000.0, nth: 0 };
        let (core, device, _, _) = super::super::test_support::rig_with(crate::device::FakeConfig { faults: vec![refuse], ..Default::default() }, vec![link()]);
        let (mut rx, _) = rx(&core, 1);
        let (rx_key, tx_key) = (stream_key(Dir::Rx, 0), stream_key(Dir::Tx, 0));
        let ms = |n: i64| core.ticks(n * 1_000_000);
        {
            // The transmit line holds a booked change, so its items, plan and clocks are
            // something a refusal could move.
            let mut streams = lock(&core.streams);
            streams.lines.insert(tx_key, tx_line(&core));
            streams.stream_mut(tx_key).configs.insert(1, Arc::new(ColdConfig::new(BTreeMap::new())));
            assert!(streams.book(&core, tx_key, Item { e: ms(20), seq: 1, ready: ms(20), release: NO_RELEASE, delivered: None, refused: false, kind: Kind::Cold(Config { channels: Channels::first(1), ratio: Rational::new(100, 1).unwrap() }) }));
        }
        let snapshot = |key: StreamKey| -> Held {
            let streams = lock(&core.streams);
            (streams.line(key).expect("a line").items.iter().map(|item| (item.e, item.seq, item.refused)).collect(),
             streams.planned(key).into_iter().map(|planned| (planned.segment.origin, planned.segment.cut, planned.segment.config.channels.iter().collect::<Vec<u32>>(), planned.segment.by)).collect(),
             streams.stream(key).clocks.iter().map(|(clock, ended)| (clock.origin, *ended)).collect())
        };
        let before = snapshot(tx_key);
        assert_eq!((before.0.len(), before.1.len(), before.2.len()), (1, 2, 2), "the transmit line holds its change, its two segments and its clocks: {before:?}");
        book(&core, ms(10), 1, rate(2_000_000));
        rx.poll();
        rx.receive(samples(0, 10_000));
        assert!(rx.stream.is_none());
        assert!(rx.begin());
        assert!(lock(&core.streams).line(rx_key).unwrap().items.iter().any(|item| item.seq == 1 && item.refused), "the receive refusal was not booked");
        assert_eq!(snapshot(tx_key), before, "the receive refusal changed the transmit stream");
        assert!(device.calls().iter().any(|call| call.starts_with("apply rx 0 rate=2000000")), "{:?}", device.calls());
    }

    #[test]
    fn ur_25_a_refusal_of_a_layout_stream_leaves_the_other_stream_alone() {
        // RM-25, RM-29 (design 33 D15): a configuration stream 0's device refuses is booked
        // refused on stream 0's line alone; stream 1's line, with a booked change of its
        // own, keeps its items, plan and clocks.
        let refuse = crate::device::FakeFault::WrongRate { claimed: 2e6, applied: 2e6 - 1_000.0, nth: 0 };
        let device = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig { faults: vec![refuse], ..Default::default() }));
        let (core, _, _) = super::super::test_support::rig_on_description(device, vec![link()], Default::default(), layout_description());
        let (mut rx, _) = rx(&core, 1);
        let (rx_key, other) = (stream_key(Dir::Rx, 0), stream_key(Dir::Rx, 1));
        let ms = |n: i64| core.ticks(n * 1_000_000);
        {
            let mut streams = lock(&core.streams);
            streams.lines.insert(other, Line::new(ezsdr_radio::timeline::Stream {
                direction: Direction::Rx,
                origin: 0,
                config: Config { channels: Channels::first(0).with(1), ratio: Rational::new(200, 1).unwrap() },
                lead: 0,
                call: 0,
                allowance: 0,
            }));
            streams.stream_mut(other).configs.insert(1, Arc::new(ColdConfig::new(BTreeMap::new())));
            assert!(streams.book(&core, other, Item { e: ms(30), seq: 1, ready: ms(30), release: NO_RELEASE, delivered: None, refused: false, kind: Kind::Cold(Config { channels: Channels::first(0).with(1), ratio: Rational::new(100, 1).unwrap() }) }));
        }
        let snapshot = |key: StreamKey| -> Held {
            let streams = lock(&core.streams);
            (streams.line(key).expect("a line").items.iter().map(|item| (item.e, item.seq, item.refused)).collect(),
             streams.planned(key).into_iter().map(|planned| (planned.segment.origin, planned.segment.cut, planned.segment.config.channels.iter().collect::<Vec<u32>>(), planned.segment.by)).collect(),
             streams.stream(key).clocks.iter().map(|(clock, ended)| (clock.origin, *ended)).collect())
        };
        let before = snapshot(other);
        assert_eq!((before.0.len(), before.1.len()), (1, 2), "stream 1 holds its change and its two segments: {before:?}");
        book(&core, ms(10), 1, rate(2_000_000));
        rx.poll();
        rx.receive(samples(0, 10_000));
        assert!(rx.stream.is_none());
        assert!(rx.begin());
        assert!(lock(&core.streams).line(rx_key).unwrap().items.iter().any(|item| item.seq == 1 && item.refused), "stream 0's refusal was not booked");
        assert_eq!(snapshot(other), before, "stream 0's refusal changed stream 1");
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
        let configs = [
            Config { channels: Channels::first(1), ratio: Rational::new(100, 1).unwrap() },
            Config { channels: Channels::first(2), ratio: Rational::new(200, 1).unwrap() },
        ];
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
        rx.finish();
        assert!(rx.begin());
        for name in ["rx_open", "rx_start", "rx_recv", "rx_stop", "apply"] {
            assert!(spy.seen(name) > 0, "the spy never saw {name}");
        }
        assert_eq!(spy.seen("rx_open"), 2, "the reopen of the two-channel segment");
    }

    #[test]
    fn ur_25_a_refused_receive_owner_books_and_gives_back_in_one_line_section() {
        // UR-25 (S2, receive): uhd-rx books the refusal while its claim still holds the
        // streamer, in the section that holds the line; a move's source keeps `Owner { seq }`
        // until both lines hold the refusal.
        let refuse = crate::device::FakeFault::WrongRate { claimed: 2e6, applied: 2e6 - 1_000.0, nth: 0 };
        let (core, device, _, _) = super::super::test_support::rig_with(crate::device::FakeConfig { faults: vec![refuse], ..Default::default() }, vec![link()]);
        let (mut rx, _) = rx(&core, 1);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        book(&core, ms(10), 1, rate(2_000_000));
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
        assert_eq!(holders.recv().unwrap(), Holder::Owner { seq: Some(1) }, "the claim still holds the stream");
        assert!(core.streams.try_lock().is_err(), "the refusal is booked under the line lock");
        let _ = resume.send(());
        owner.join().expect("the receive owner");
        let streams = lock(&core.streams);
        assert_eq!(streams.holder(stream_key(Dir::Rx, 0)), Holder::Free);
        assert!(!streams.reading(stream_key(Dir::Rx, 0)));
        assert!(streams.line(stream_key(Dir::Rx, 0)).unwrap().items.iter().any(|item| item.seq == 1 && item.refused), "the refusal is booked");
        drop(streams);
        assert!(!device.calls().iter().any(|call| call.starts_with("rx_start")), "{:?}", device.calls());
    }

}
