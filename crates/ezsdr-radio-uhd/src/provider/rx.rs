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
use ezsdr_radio::payloads::{AlignmentErrorPayload, LateCommandPayload, RxOverflowCause, RxOverflowPayload};
use ezsdr_radio::timeline::Config;
use serde_json::json;

use super::control::Planned;
use super::core::{Clock, Core, Streams, lock};
use crate::device::{Dir, Iq, RxRecv};
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
    channels: usize,
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
    fn new(clock: Clock, handle: SampleClockHandle, channels: usize, not_before: Option<i64>) -> Stream {
        Stream {
            clock,
            handle: Some(handle),
            config: Config { channels: channels as u16, ratio: Rational::new(clock.n as u64, 1).expect("a positive ratio") },
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
    fn follow(&mut self, streams: &Streams) {
        let planned = streams.lines[Dir::Rx as usize].as_ref().and_then(|line| line.segment(self.clock.origin, self.config));
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
    cmds: Receiver<RxCmd>,
    stream: Option<Stream>,
    /// The earliest origin of a planned segment not yet begun.
    from: i64,
    /// The channel count the streamer is open for.
    opened: usize,
    pool: HostPool,
    started: Instant,
    last_samples: Instant,
    stall: Option<(Wall, Wall)>,
    first: bool,
    first_block: Option<Instant>,
    exit: bool,
    /// When the last untimed stop was issued: the next stream drops what precedes it.
    last_stop: Option<i64>,
}

const RECV_TIMEOUT: Wall = Wall::from_millis(100);

impl Rx {
    /// uhd-rx with the stream `start` started at T0, its clock declared (UR-15).
    pub fn new(core: Arc<Core>, cmds: Receiver<RxCmd>, first: Option<(SampleClockHandle, usize, i64)>, stall: Option<(Wall, Wall)>) -> Rx {
        let channels = first.as_ref().map_or(0, |(_, channels, _)| *channels);
        let pool = HostPool::new(core.block_len * channels.max(1) * 8);
        let stream = first.map(|(handle, channels, t0)| {
            let clock = Clock { domain: handle.id, origin: t0, n: handle.root_ticks_per_tick.num() as i64 };
            Stream::new(clock, handle, channels, None)
        });
        lock(&core.streams).delivered = stream.as_ref().map(|stream| (stream.clock.origin, stream.config, 0));
        Rx {
            from: stream.as_ref().map_or(i64::MIN, |stream| stream.clock.origin + 1),
            opened: channels,
            stream,
            core,
            cmds,
            pool,
            started: Instant::now(),
            last_samples: Instant::now(),
            stall,
            first: true,
            first_block: None,
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
        stream.follow(&lock(&self.core.streams));
        let done = stream.cut.is_some_and(|cut| stream.clock.at_or_after(cut) <= stream.expected) && self.core.now() >= stream.clock.origin;
        if done {
            if !stream.stopped {
                stream.stopped = true;
                self.last_stop = stop_at_cut(&self.core, stream.cut.expect("a cut")).or(self.last_stop);
            }
            self.finish();
        }
    }

    pub fn run(mut self) {
        loop {
            match self.turn() {
                Turn::Exit => return,
                Turn::Idle => std::thread::sleep(Wall::from_millis(1)),
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
            // Counted from the first block, so that `after` is into the stream.
            if self.first_block.is_some_and(|at| at.elapsed() >= after) {
                // UR-34's B5 builder: stop reading once, to provoke a real overflow.
                self.stall = None;
                self.core.timing(json!({ "what": "rx_stall", "ms": duration.as_millis() as u64 }));
                std::thread::sleep(duration);
            }
        }
        // A cut booked during the wait cuts this block too: `samples` reads the plan (RM-16).
        let result = self.core.device.rx_recv(self.recv_len(), self.recv_timeout());
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
                    self.last_stop = stop_at_cut(&self.core, stream.cut.expect("set above")).or(self.last_stop);
                }
            }
            self.finish();
        }
    }

    /// RM-25, UR-17: the next planned segment begins: the streamer reopened if its channel
    /// count changed, the direction configured with the configuration in effect at its
    /// origin — unless uhd-control did both for an enable from no stream — and a timed start
    /// issued there. A configuration the device refuses halts the stream: uhd-rx books the
    /// refusal, and the plan it then reads may begin the next segment earlier (RM-25).
    fn begin(&mut self) -> bool {
        let next: Option<Planned> = {
            let mut streams = lock(&self.core.streams);
            let next = streams.planned(Dir::Rx).into_iter().find(|planned| planned.segment.origin >= self.from);
            streams.idle[Dir::Rx as usize] = next.is_none();
            next
        };
        let Some(next) = next else { return false };
        let (origin, channels) = (next.segment.origin, next.channels());
        let configured = if next.configured {
            Ok(())
        } else {
            let reopened = if channels != self.opened { self.core.device.rx_open(channels) } else { Ok(()) };
            reopened.and_then(|()| next.configure(&self.core, Dir::Rx).map(|_| ()))
        };
        if let Err(error) = configured {
            // Not begun. A refusal the plan cannot take — its command pruned — skips the segment.
            let refused = next.segment.by.is_some_and(|by| lock(&self.core.streams).refuse(&self.core, Dir::Rx, by));
            if !refused {
                self.from = origin.saturating_add(1);
            }
            self.opened = 0;
            self.core.device_failed("update_parameter", &error);
            return true;
        }
        self.from = origin.saturating_add(1);
        self.opened = channels;
        if let Err(error) = self.core.device.rx_start(origin) {
            self.core.device_failed("update_parameter", &error);
            return true;
        }
        let declared = self.core.id.child(Dir::Rx.name()).map_err(|e| e.to_string())
            .and_then(|stream| self.core.clocks.declare_sample_clock(stream, self.core.root, next.segment.config.ratio).map_err(|e| e.to_string()));
        match declared {
            Ok(handle) => {
                let clock = Clock { domain: handle.id, origin, n: next.n() };
                self.pool = HostPool::new(self.core.block_len * channels.max(1) * 8);
                self.stream = Some(Stream::new(clock, handle, channels, self.last_stop.take()));
                lock(&self.core.streams).delivered = Some((origin, next.segment.config, 0));
                self.last_samples = Instant::now();
                self.follow();
            }
            Err(error) => self.core.command_rejected("update_parameter", &format!("UR-25: {error}")),
        }
        self.core.timing(json!({ "what": "rx_segment", "origin": origin, "channels": channels, "at": self.core.now() }));
        true
    }

    /// The stream ends: its clock, if it has one, ends at its cut (RM-16, UR-17), and the
    /// items before the command that began it are pruned (#63). The cut is read from the
    /// plan here, since another thread may have booked an earlier one — a loss, or
    /// `Provider::stop`'s end — after this turn read it, while a receive call waited.
    fn finish(&mut self) {
        let Some(mut stream) = self.stream.take() else { return };
        let mut streams = lock(&self.core.streams);
        stream.follow(&streams);
        if let (None, Some(cut)) = (&stream.handle, stream.cut) {
            if let Err(error) = self.core.clocks.end(stream.clock.domain, self.core.at(cut)) {
                self.core.reject_note(json!({ "clock_not_ended": error.to_string() }));
            }
        }
        streams.delivered = None;
        let by = streams.lines[Dir::Rx as usize].as_ref().and_then(|line| line.segment(stream.clock.origin, stream.config)).and_then(|segment| segment.by);
        streams.prune(Dir::Rx, by);
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
                self.last_samples = Instant::now();
                self.samples(first_tick, samples);
            }
            RxRecv::Timeout => {
                // Past the cut and its allowance, and silent for as long: a stream still
                // delivering, however late, is waited for (Review N, B2).
                let span = quiet_span(&self.core, stream);
                let quiet = self.last_samples.elapsed() >= Wall::from_nanos(self.core.ns(span) as u64);
                if let Some(cut) = stream.cut.filter(|cut| quiet && i128::from(now) >= i128::from(*cut) + i128::from(span)) {
                    // Silent past its cut: stopped all the same, never left streaming.
                    if !stream.stopped {
                        self.last_stop = stop_at_cut(&self.core, cut).or(self.last_stop);
                    }
                    self.finish();
                } else if stream.cut.is_none_or(|cut| now < cut)
                    && i128::from(now) > i128::from(stream.start) + i128::from(self.core.mcr)
                    && self.last_samples.elapsed() > Wall::from_secs(1)
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
                    if let Some(issued) = stop_at_cut(&self.core, stream.cut.expect("a cut")) {
                        stream.stopped = true;
                        self.last_stop = Some(issued);
                    }
                }
            }
            RxRecv::LateCommand => {
                // UR-17: the device missed the timed start; the origin stays. The new start
                // waits the profile's start lead from now (spec 20, VF-6).
                let restart = match ezsdr_radio::timeline::lattice(now.saturating_add(self.core.ticks(self.core.description.timing.start_lead_ns)), stream.config.ratio) {
                    Ok(restart) => restart,
                    Err(error) => return self.core.command_rejected("start", &format!("UR-17: {error}")),
                };
                let payload = serde_json::to_value(LateCommandPayload {
                    key: None,
                    requested: self.core.at(stream.start),
                    applied: self.core.at(restart),
                })
                .expect("a payload");
                stream.report(Pending::MissedStart);
                self.core.emit(&self.core.rx_id, kinds::LATE_COMMAND, Severity::Warning, payload);
                self.core.timing(json!({ "what": "rx_restart", "requested": stream.start, "at": restart }));
                if let Err(error) = self.core.device.rx_start(restart) {
                    self.core.device_failed("start", &error);
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
            stream.follow(&streams);
            if let Some(cut_k) = stream.cut.map(|cut| clock.at_or_after(cut)).filter(|cut_k| k + len >= *cut_k) {
                len = (cut_k - k).max(0);
                ended = true;
            }
            if k + len > stream.expected {
                streams.delivered = Some((clock.origin, stream.config, k + len));
            }
        }
        if let Some(cut) = stream.cut.filter(|_| ended && !stream.stopped) {
            // UR-25: the samples reached the cut, so the stream is stopped untimed now.
            stream.stopped = true;
            self.last_stop = stop_at_cut(&self.core, cut).or(self.last_stop);
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
            let channels = stream.channels.max(1);
            stream.expected = k + len;
            let at = TimePoint::new(clock.domain, expected);
            let lost_n = lost.unwrap_or(0);
            for pending in pending {
                match pending {
                    Pending::Overrun | Pending::Sequence => {
                        let payload = RxOverflowPayload {
                            cause: if pending == Pending::Overrun { RxOverflowCause::Overrun } else { RxOverflowCause::Sequence },
                            lost: lost_n,
                            restart_gap_ns: if pending == Pending::Overrun { self.core.ns(lost_n as i64 * clock.n) } else { 0 },
                        };
                        // UR-20: the hot path, with RM-24's bytes.
                        if let Err(error) = self.core.events.emit(self.core.overflow, at, Severity::Warning, &payload.to_hot()) {
                            self.core.reject_note(json!({ "event_not_emitted": kinds::RX_OVERFLOW, "error": error.to_string() }));
                        }
                    }
                    Pending::Alignment => {
                        let payload = serde_json::to_value(AlignmentErrorPayload { lost: lost_n }).expect("a payload");
                        self.core.emit_at(&self.core.rx_id, kinds::ALIGNMENT_ERROR, Severity::Error, payload, at);
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
                    for link in &self.core.links {
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
                self.first_block = Some(Instant::now());
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
fn stop_at_cut(core: &Core, cut: i64) -> Option<i64> {
    if core.is_lost() {
        return None;
    }
    let now = core.now();
    core.timing(json!({ "what": "rx_stop_untimed", "cut": cut, "at": now }));
    match core.device.rx_stop(None) {
        Ok(()) => {
            lock(&core.rec).applied.push(json!({ "key": "rx_stop", "at": core.at(cut), "issued": core.at(now) }));
            // The device produces samples until the stop reaches it (the bench: ~0.4 ms
            // into the call): the next stream drops anything before the call's return
            // plus a device lead (Review O, N-3); it starts a restart lead later anyway.
            Some(core.now() + core.ticks(DEVICE_LEAD_NS))
        }
        Err(error) => {
            core.device_failed("update_parameter", &error);
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
    use ezsdr_kernel::module_api::Link;
    use ezsdr_kernel::stream::BackPressure;
    use ezsdr_radio::timeline::{Item, Kind, Line, Segment};

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
        let stream = core.id.child("rx").unwrap();
        let handle = core.clocks.declare_sample_clock(stream, core.root, Rational::new(200, 1).unwrap()).unwrap();
        let clock = Clock { domain: handle.id, origin: 0, n: 200 };
        let config = Config { channels: channels as u16, ratio: Rational::new(200, 1).unwrap() };
        lock(&core.streams).lines[Dir::Rx as usize] = Some(Line::new(ezsdr_radio::timeline::Stream {
            direction: Direction::Rx,
            origin: 0,
            config,
            lead: core.ticks(core.description.timing.start_lead_ns),
            call: core.block_len as i64,
            allowance: core.ticks(DELIVERY_ALLOWANCE_NS),
        }));
        let (to_rx, cmds) = std::sync::mpsc::channel();
        // No `Provider::stop` comes unless a test sends it.
        std::mem::forget(to_rx);
        core.device.rx_open(channels).unwrap();
        (Rx::new(core.clone(), cmds, Some((handle, channels, 0)), None), clock)
    }

    /// The receive plan set to segments at 1 MS/s, one channel, each `(origin, cut, by)`,
    /// as bookings would have left it.
    fn plan(core: &Core, segments: &[(i64, Option<i64>, Option<u64>)]) {
        let config = Config { channels: 1, ratio: Rational::new(200, 1).unwrap() };
        let mut streams = lock(&core.streams);
        for by in segments.iter().filter_map(|(_, _, by)| *by) {
            streams.configs[Dir::Rx as usize].insert(by, (Arc::new(ColdConfig::new(crate::device::Settings::default())), false));
        }
        streams.lines[Dir::Rx as usize].as_mut().unwrap().plan = segments.iter().map(|&(origin, cut, by)| Segment { origin, cut, config, by }).collect();
    }

    /// Books a receive command as uhd-control does, its segment's configuration with it.
    fn book(core: &Core, e: i64, seq: u64, kind: Kind) {
        let mut streams = lock(&core.streams);
        streams.configs[Dir::Rx as usize].insert(seq, (Arc::new(ColdConfig::new(crate::device::Settings::default())), false));
        assert!(streams.book(core, Dir::Rx, Item { e, seq, ready: 0, delivered: None, refused: false, kind }));
    }

    fn rate(rate: u64) -> Kind {
        Kind::Cold(Config { channels: 1, ratio: Rational::new(200_000_000 / rate, 1).unwrap() })
    }

    fn samples(first_tick: i64, n: usize) -> RxRecv {
        RxRecv::Samples { first_tick, samples: vec![vec![[0.0, 0.0]; n]] }
    }

    fn received(link: &Arc<dyn ezsdr_kernel::stream::DataLink>) -> Vec<(ClockDomainId, i64, u32)> {
        std::iter::from_fn(|| link.receive().map(|(b, _)| b)).map(|block| (block.header().first_sample_time.domain, block.header().first_sample_time.ticks, block.header().len)).collect()
    }

    use ezsdr_kernel::id::ClockDomainId;

    fn clocks(core: &Core) -> Vec<(i64, Option<i64>)> {
        core.clocks.sample_clock_records().iter().map(|record| (record.origin.ticks, record.ended_at.map(|end| end.ticks))).collect()
    }

    #[test]
    fn ur_19_reports_before_one_block_keep_each_kind() {
        // Reports of different kinds before the next block: that block carries each one's flag,
        // on every channel, and each one's event is emitted with its time jump; a repeated kind
        // is one event; a jump no report announced is a sequence discontinuity with no event
        // (UR-17…UR-19). RX_OVERFLOW goes by the hot path (UR-20), so only the set is compared.
        let alignment = (kinds::ALIGNMENT_ERROR, BlockFlags::ALIGNMENT);
        let sequence = (kinds::RX_OVERFLOW, BlockFlags::SEQ_DISCONTINUITY);
        let overrun = (kinds::RX_OVERFLOW, BlockFlags::RESTARTED);
        for (reports, expected) in [
            (Vec::new(), Vec::new()),
            (vec![RxRecv::Alignment], vec![alignment]),
            (vec![RxRecv::Overflow { out_of_sequence: true }], vec![sequence]),
            (vec![RxRecv::Alignment, RxRecv::Overflow { out_of_sequence: true }, RxRecv::Alignment], vec![alignment, sequence]),
            (vec![RxRecv::Overflow { out_of_sequence: false }, RxRecv::Alignment], vec![overrun, alignment]),
        ] {
            let link = link();
            let (core, _, _, events) = super::super::test_support::rig_with_links(vec![link.clone()]);
            let (mut rx, clock) = rx(&core, 2);
            for report in reports {
                rx.receive(report);
            }
            rx.receive(RxRecv::Samples { first_tick: clock.instant(2), samples: vec![vec![[0.0, 0.0]; 2]; 2] });
            let block = link.receive().expect("the block after the reports").0;
            let unannounced = if expected.is_empty() { BlockFlags::SEQ_DISCONTINUITY } else { BlockFlags::NONE };
            let flags = expected.iter().fold(BlockFlags::GAP_BEFORE | unannounced, |flags, (_, flag)| flags | *flag);
            assert_eq!((block.header().flags, block.header().lost), (flags, Some(2)));
            assert_eq!(block.header().valid, ezsdr_kernel::stream::ChannelMask::full(2), "a whole-stream gap");
            let got: Vec<_> = events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
                .filter(|e| [kinds::ALIGNMENT_ERROR, kinds::RX_OVERFLOW].contains(&e.kind.as_str()))
                .collect();
            for event in &got {
                assert_eq!(event.source, core.rx_id);
                let lost = match event.kind.as_str() {
                    kinds::RX_OVERFLOW => {
                        assert!(event.payload.is_array(), "the hot path's bytes: {}", event.payload);
                        RxOverflowPayload::from_payload(&event.payload).unwrap().lost
                    }
                    _ => event.payload["lost"].as_u64().unwrap(),
                };
                assert_eq!(lost, 2);
            }
            let mut got: Vec<_> = got.iter().map(|e| e.kind.as_str()).collect();
            got.sort();
            let mut expected: Vec<_> = expected.iter().map(|(kind, _)| *kind).collect();
            expected.sort();
            assert_eq!(got, expected);
        }
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
            .map(|block| (block.header().first_sample_time.ticks, block.header().len, block.header().flags, block.header().lost)).collect();
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
            assert_eq!(lock(&core.streams).planned(Dir::Rx)[0].segment.cut, Some(expected), "the plan's cut, lost at {lost}");
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
            rx.last_samples = Instant::now() - Wall::from_secs(1);
            rx.receive(RxRecv::Timeout);
            assert!(rx.stream.is_none(), "lost: {lost}");
            let cut = lock(&core.streams).planned(Dir::Rx)[0].segment.cut;
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
        let planned: Vec<_> = lock(&core.streams).planned(Dir::Rx).iter().map(|p| (p.segment.origin, p.segment.cut)).collect();
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
        let planned: Vec<_> = lock(&core.streams).planned(Dir::Rx).iter().map(|p| (p.segment.origin, p.segment.cut)).collect();
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
        assert_eq!(lock(&core.streams).planned(Dir::Rx).iter().map(|p| (p.segment.origin, p.segment.by)).collect::<Vec<_>>(), [(0, None), (ms(65), Some(1))]);
        rx.poll();
        rx.receive(samples(0, 10_000));
        assert!(rx.stream.is_none());
        assert!(rx.begin());
        assert!(lock(&core.streams).lines[Dir::Rx as usize].as_ref().unwrap().items.iter().any(|item| item.seq == 1 && item.refused));
        assert!(rx.begin());
        let starts: Vec<_> = device.calls().into_iter().filter(|c| c.starts_with("rx_start ")).collect();
        assert_eq!(starts, [format!("rx_start {}", ms(65))]);
        assert_eq!(rx.stream.as_ref().map(|stream| stream.config.ratio.num()), Some(100), "the refused rate is the stream's (KC-27)");
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
        let items: Vec<u64> = streams.lines[Dir::Rx as usize].as_ref().unwrap().items.iter().map(|item| item.seq).collect();
        assert_eq!(items, [2, 3]);
        assert_eq!(streams.configs[Dir::Rx as usize].keys().copied().collect::<Vec<_>>(), [2, 3]);
        assert_eq!(streams.planned(Dir::Rx).len(), 3, "every segment stays in the plan");
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
            assert!(device.calls().contains(&format!("rx_start {}", ms(65))));
            // The `Stop` at 20 ms cuts the planned segment at its origin: the plan drops it.
            time.advance_to(core.at(ms(20))).unwrap();
            plan(&core, &[(0, Some(10_000), None)]);
            rx.poll();
            assert_eq!(device.calls().iter().filter(|c| c.starts_with("rx_stop ")).count(), 1, "only the first stream's stop yet");
            time.advance_to(core.at(ms(80))).unwrap();
            if silent {
                rx.last_samples = Instant::now() - Wall::from_secs(1);
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
}
