//! uhd-rx: owns the receive streamer. It carries out the receive plan uhd-control hands it
//! — each segment configured, started at its origin, its clock registered at its first
//! block and ended at its cut — and turns `rx_recv` results into Stream Contract blocks on
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

use super::control::{Plan, Planned};
use super::core::{Clock, Core, lock};
use crate::device::{Dir, Iq, RxRecv};
use crate::profile::{DELIVERY_ALLOWANCE_NS, DEVICE_LEAD_NS};

pub(crate) enum RxCmd {
    /// The receive plan uhd-control booked: every segment with its configuration (UR-25).
    Plan(Plan),
    /// `Provider::stop` began at `at`: no segment begins any more, and under `abort` the
    /// stream ends at once, at the first sample not yet delivered; under `orderly` it ends at
    /// the cut uhd-control books at `at` (RM-16, UR-26).
    Cut { at: i64, mode: StopMode },
    /// `Provider::stop` (UR-26).
    Shutdown(StopMode),
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
}

pub(crate) struct Rx {
    core: Arc<Core>,
    cmds: Receiver<RxCmd>,
    stream: Option<Stream>,
    plan: Plan,
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
    /// `Provider::stop` has begun, at this instant and in this mode: no segment begins any
    /// more (UR-25; Review M, N-1).
    stopping: Option<(i64, StopMode)>,
    /// The command whose segment the device refused: nothing begins until a plan made
    /// without it arrives (RM-25).
    refused: Option<u64>,
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
        Rx {
            from: stream.as_ref().map_or(i64::MIN, |stream| stream.clock.origin + 1),
            opened: channels,
            stream,
            plan: Arc::new(Vec::new()),
            core,
            cmds,
            pool,
            started: Instant::now(),
            last_samples: Instant::now(),
            stall,
            first: true,
            first_block: None,
            exit: false,
            stopping: None,
            refused: None,
            last_stop: None,
        }
    }

    /// Carries out every command that has arrived.
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

    pub fn run(mut self) {
        loop {
            self.poll();
            if self.exit && self.stream.is_none() {
                return;
            }
            if self.core.is_lost() {
                // The loss's cut, from the plan: the stream and its clock end there.
                if self.stream.as_ref().is_some_and(|stream| stream.cut.is_some()) {
                    self.finish();
                }
                if self.exit {
                    return;
                }
                std::thread::sleep(Wall::from_millis(1));
                continue;
            }
            if self.stream.is_none() {
                if !self.begin() {
                    std::thread::sleep(Wall::from_millis(1));
                }
                continue;
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
            let result = self.core.device.rx_recv(self.recv_len(), self.recv_timeout());
            // A cut that arrived during the wait cuts this block too (RM-16).
            self.poll();
            self.receive(result);
        }
    }

    fn command(&mut self, cmd: RxCmd) {
        match cmd {
            RxCmd::Plan(plan) => self.plan = plan,
            RxCmd::Cut { at, mode } => {
                if self.stopping.is_some() {
                    return;
                }
                self.stopping = Some((at, mode));
                // RM-16: under `abort` the stream ends at the first sample not yet delivered.
                if let Some(stream) = self.stream.as_mut().filter(|_| mode == StopMode::Abort) {
                    stream.cut_at(stream.clock.instant(stream.expected));
                    if !stream.stopped {
                        if let Some(issued) = stop_at_cut(&self.core, stream.cut.expect("set above")) {
                            stream.stopped = true;
                            self.last_stop = Some(issued);
                        }
                    }
                }
            }
            RxCmd::Shutdown(mode) => {
                self.exit = true;
                self.command(RxCmd::Cut { at: self.core.now(), mode });
                // RM-16: a stream uhd-control left without a cut — it did not book the stop —
                // ends at the first sample at or after the stop instant.
                self.follow();
                if let (Some((at, _)), Some(stream)) = (self.stopping, self.stream.as_mut()) {
                    if stream.cut.is_none() {
                        stream.cut_at(stream.clock.instant(stream.clock.at_or_after(at).max(stream.expected)));
                    }
                }
                if mode == StopMode::Abort && self.stream.is_some() {
                    self.finish();
                }
            }
        }
    }

    /// UR-17: the stream being delivered takes its cut from the plan, never before the first
    /// sample it has not delivered; one the plan no longer has was cut at its origin before
    /// its first sample, and its timed start may be queued, so it is stopped untimed only
    /// after that origin, what arrives discarded (UR-17; #56).
    fn follow(&mut self) {
        let Some(stream) = self.stream.as_mut() else { return };
        let planned = self.plan.iter().find(|planned| planned.segment.origin == stream.clock.origin && planned.segment.config == stream.config);
        match planned {
            Some(planned) => {
                if let Some(cut) = planned.segment.cut {
                    let cut = stream.clock.instant(cut.max(stream.expected));
                    stream.cut_at(cut);
                }
            }
            None if !self.plan.is_empty() => stream.cut_at(stream.clock.instant(stream.expected)),
            None => {}
        }
    }

    /// RM-25, UR-17: the next planned segment begins: the streamer reopened if its channel
    /// count changed, the direction configured with the configuration in effect at its
    /// origin — unless uhd-control did both for an enable from no stream — and a timed start
    /// issued there. A configuration the device refuses halts the stream (RM-25).
    fn begin(&mut self) -> bool {
        if let Some(by) = self.refused {
            if self.plan.iter().any(|planned| planned.segment.by == Some(by)) {
                return false;
            }
            self.refused = None;
        }
        let next: Option<Planned> = self.plan.iter().find(|planned| planned.segment.origin >= self.from).cloned();
        let Some(next) = next.filter(|_| self.stopping.is_none()) else {
            lock(&self.core.streams).idle[Dir::Rx as usize] = true;
            return false;
        };
        lock(&self.core.streams).idle[Dir::Rx as usize] = false;
        let (origin, channels) = (next.segment.origin, next.channels());
        self.from = origin.saturating_add(1);
        let configured = if next.configured {
            Ok(())
        } else {
            let reopened = if channels != self.opened { self.core.device.rx_open(channels) } else { Ok(()) };
            reopened.and_then(|()| next.settings.configure(&self.core, Dir::Rx, channels).map(|_| ()))
        };
        if let Err(error) = configured {
            if let Some(by) = next.segment.by {
                lock(&self.core.streams).refused.push((Dir::Rx, by));
                self.refused = Some(by);
            }
            self.opened = 0;
            self.core.device_failed("update_parameter", &error);
            return true;
        }
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
                self.last_samples = Instant::now();
                self.follow();
            }
            Err(error) => self.core.command_rejected("update_parameter", &format!("UR-25: {error}")),
        }
        self.core.timing(json!({ "what": "rx_segment", "origin": origin, "channels": channels, "at": self.core.now() }));
        true
    }

    /// The stream ends: its clock, if it has one, ends at its cut (RM-16, UR-17), and
    /// `Provider::stop`'s cut is recorded.
    fn finish(&mut self) {
        let Some(stream) = self.stream.take() else { return };
        if let (Some((at, mode)), Some(cut)) = (self.stopping, stream.cut) {
            self.core.timing(json!({ "what": "rx_stop", "at": at, "until": cut, "mode": format!("{mode:?}") }));
        }
        if let (None, Some(cut)) = (&stream.handle, stream.cut) {
            if let Err(error) = self.core.clocks.end(stream.clock.domain, self.core.at(cut)) {
                self.core.reject_note(json!({ "clock_not_ended": error.to_string() }));
            }
        }
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
        if let Some(cut) = stream.cut {
            let cut_k = clock.at_or_after(cut);
            if k + len >= cut_k {
                // UR-25: the samples reached the cut, so the stream is stopped untimed now.
                if !stream.stopped {
                    stream.stopped = true;
                    self.last_stop = stop_at_cut(&self.core, cut).or(self.last_stop);
                }
                len = (cut_k - k).max(0);
                ended = true;
            }
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
                            Pending::Sequence | Pending::Alignment => BlockFlags::SEQ_DISCONTINUITY,
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
mod tests {
    use super::*;
    use super::super::control::ColdConfig;
    use ezsdr_kernel::module_api::Link;
    use ezsdr_kernel::stream::BackPressure;
    use ezsdr_radio::timeline::Segment;

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

    /// uhd-rx with the stream T0 started at 0, `channels` at 1 MS/s on the 200 MHz root.
    fn rx(core: &Arc<Core>, channels: usize) -> (Rx, Clock) {
        let stream = core.id.child("rx").unwrap();
        let handle = core.clocks.declare_sample_clock(stream, core.root, Rational::new(200, 1).unwrap()).unwrap();
        let clock = Clock { domain: handle.id, origin: 0, n: 200 };
        let (_to_rx, cmds) = std::sync::mpsc::channel();
        core.device.rx_open(channels).unwrap();
        (Rx::new(core.clone(), cmds, Some((handle, channels, 0)), None), clock)
    }

    /// A plan of receive segments at 1 MS/s, one channel, each `(origin, cut, by)`.
    fn plan(segments: &[(i64, Option<i64>, Option<u64>)]) -> RxCmd {
        let config = Config { channels: 1, ratio: Rational::new(200, 1).unwrap() };
        RxCmd::Plan(Arc::new(segments.iter().map(|&(origin, cut, by)| Planned {
            segment: Segment { origin, cut, config, by },
            settings: Arc::new(ColdConfig::new(origin, crate::device::Settings { rate: Some(1e6), ..crate::device::Settings::default() })),
            configured: false,
            clock: None,
        }).collect()))
    }

    fn samples(first_tick: i64, n: usize) -> RxRecv {
        RxRecv::Samples { first_tick, samples: vec![vec![[0.0, 0.0]; n]] }
    }

    fn received(link: &Arc<dyn ezsdr_kernel::stream::DataLink>) -> Vec<(ClockDomainId, i64, u32)> {
        std::iter::from_fn(|| link.receive()).map(|block| (block.header().first_sample_time.domain, block.header().first_sample_time.ticks, block.header().len)).collect()
    }

    use ezsdr_kernel::id::ClockDomainId;

    fn clocks(core: &Core) -> Vec<(i64, Option<i64>)> {
        core.clocks.sample_clock_records().iter().map(|record| (record.origin.ticks, record.ended_at.map(|end| end.ticks))).collect()
    }

    #[test]
    fn ur_19_reports_before_one_block_keep_each_kind() {
        // Reports of different kinds before the next block: that block carries each one's flag
        // and each one's event is emitted with its time jump; a repeated kind is one event
        // (UR-17…UR-19). RX_OVERFLOW goes by the hot path, so only the set is compared.
        let alignment = (kinds::ALIGNMENT_ERROR, BlockFlags::SEQ_DISCONTINUITY);
        let sequence = (kinds::RX_OVERFLOW, BlockFlags::SEQ_DISCONTINUITY);
        let overrun = (kinds::RX_OVERFLOW, BlockFlags::RESTARTED);
        for (reports, expected) in [
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
            let block = link.receive().expect("the block after the reports");
            let flags = expected.iter().fold(BlockFlags::GAP_BEFORE, |flags, (_, flag)| flags | *flag);
            assert_eq!((block.header().flags, block.header().lost), (flags, Some(2)));
            let got: Vec<_> = events.drain().into_iter()
                .filter(|e| [kinds::ALIGNMENT_ERROR, kinds::RX_OVERFLOW].contains(&e.kind.as_str()))
                .collect();
            for event in &got {
                let lost = match event.kind.as_str() {
                    kinds::RX_OVERFLOW => RxOverflowPayload::from_payload(&event.payload).unwrap().lost,
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
    fn ur_26_a_late_start_after_abort_does_not_stop_again() {
        let (core, device, _, _) = super::super::test_support::rig();
        let (mut rx, _) = rx(&core, 1);
        rx.command(RxCmd::Cut { at: 0, mode: StopMode::Abort });
        rx.receive(RxRecv::LateCommand);
        rx.command(RxCmd::Cut { at: 0, mode: StopMode::Abort });
        rx.command(RxCmd::Shutdown(StopMode::Abort));
        assert_eq!(device.calls().iter().filter(|c| c.starts_with("rx_stop ")).count(), 1);
        assert_eq!(lock(&core.rec).applied.iter().filter(|r| r["key"] == "rx_stop").count(), 1);
        assert!(!device.calls().iter().any(|c| c.starts_with("rx_start ")));
    }

    #[test]
    fn ur_26_a_late_start_during_orderly_stop_is_recorded_once() {
        let (core, device, time, _) = super::super::test_support::rig();
        let (mut rx, _) = rx(&core, 1);
        rx.command(RxCmd::Cut { at: 0, mode: StopMode::Orderly });
        // uhd-control's plan with the stop's cut, at sample 0 (RM-16).
        rx.command(plan(&[(0, Some(0), None)]));
        rx.follow();
        let cut = rx.stream.as_ref().unwrap().cut.unwrap();
        rx.receive(RxRecv::LateCommand);
        rx.receive(RxRecv::LateCommand);
        assert!(rx.stream.as_ref().unwrap().stopped);
        time.advance_to(core.at(cut + core.ticks(20_000_000))).unwrap();
        rx.last_samples = Instant::now() - Wall::from_secs(1);
        rx.receive(RxRecv::Timeout);
        rx.command(RxCmd::Shutdown(StopMode::Abort));
        assert!(rx.stream.is_none());
        assert_eq!(device.calls().iter().filter(|c| c.starts_with("rx_stop ")).count(), 1);
        let rec = lock(&core.rec);
        let stops: Vec<_> = rec.applied.iter().filter(|r| r["key"] == "rx_stop").collect();
        assert_eq!(stops.len(), 1);
        assert_eq!(stops[0]["at"], serde_json::to_value(core.at(cut)).unwrap());
        assert!(!device.calls().iter().any(|c| c.starts_with("rx_start ")));
    }

    #[test]
    fn ur_26_a_stopped_stream_keeps_its_end() {
        // RM-16, UR-26 (spec 21, VG-1; spec 22, VH-1): a cut from the plan — a `Stop`'s at sample
        // 1 000 — is kept when `Provider::stop` comes at 5 ms, under `orderly` as under `abort`
        // once 1 000 samples are delivered, and an abort with fewer delivered ends the stream
        // there, earlier: an end moves only earlier. The clock ends at the stream's end.
        for (mode, delivered, end) in [
            (StopMode::Orderly, 300, 1_000),
            (StopMode::Abort, 1_000, 1_000),
            (StopMode::Abort, 300, 300),
        ] {
            let link = link();
            let (core, device, time, _) = super::super::test_support::rig_with_links(vec![link.clone()]);
            let (mut rx, clock) = rx(&core, 1);
            rx.command(plan(&[(0, Some(1_000), None)]));
            rx.follow();
            rx.receive(samples(clock.instant(0), delivered as usize));
            time.advance_to(core.at(core.ticks(5_000_000))).unwrap();
            rx.command(RxCmd::Cut { at: core.ticks(5_000_000), mode });
            // As the run loop's poll does after each command: the plan's later cut leaves the
            // abort's earlier one.
            rx.follow();
            let case = format!("{mode:?}, {delivered} delivered");
            // An abort stops the device at once (VG-1).
            if mode == StopMode::Abort {
                assert_eq!(device.calls().iter().filter(|c| c.starts_with("rx_stop ")).count(), 1, "{case}");
            }
            if delivered < 1_000 {
                rx.receive(samples(clock.instant(delivered), 6_000));
            }
            // The stop's row, written as the stream ends; none for one that had ended at its
            // cut already.
            let rows: Vec<_> = lock(&core.rec).timing.iter().filter(|r| r["what"] == "rx_stop")
                .map(|r| (r["at"].as_i64().unwrap(), r["until"].as_i64().unwrap())).collect();
            let expected: Vec<(i64, i64)> = if delivered == 1_000 { Vec::new() } else { vec![(core.ticks(5_000_000), clock.instant(end))] };
            assert_eq!(rows, expected, "{case}");
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
        // or the segment dropped from the plan — ends the stream at that sample.
        for dropped in [false, true] {
            let link = link();
            let (core, _, _, _) = super::super::test_support::rig_with_links(vec![link.clone()]);
            let (mut rx, clock) = rx(&core, 1);
            rx.command(plan(&[(0, None, None)]));
            rx.follow();
            rx.receive(samples(clock.instant(0), 3_000));
            rx.command(if dropped { plan(&[(core.ticks(500_000_000), None, Some(9))]) } else { plan(&[(0, Some(1_000), None)]) });
            rx.follow();
            rx.receive(samples(clock.instant(3_000), 1_000));
            assert!(rx.stream.is_none(), "dropped {dropped}");
            assert_eq!(clocks(&core), [(0, Some(clock.instant(3_000)))], "dropped {dropped}");
        }
    }

    #[test]
    fn ur_26_an_orderly_shutdown_without_a_booked_stop_cuts_at_the_stop() {
        // UR-26, RM-16: at `Provider::stop(orderly)`, a stream whose plan carries no cut —
        // uhd-control did not book the stop — ends at the first sample at or after the stop
        // instant.
        let link = link();
        let (core, _, time, _) = super::super::test_support::rig_with_links(vec![link.clone()]);
        let (mut rx, clock) = rx(&core, 1);
        rx.command(plan(&[(0, None, None)]));
        rx.follow();
        rx.receive(samples(clock.instant(0), 300));
        time.advance_to(core.at(core.ticks(8_000_000))).unwrap();
        rx.command(RxCmd::Cut { at: core.ticks(5_000_000), mode: StopMode::Orderly });
        rx.command(RxCmd::Shutdown(StopMode::Orderly));
        rx.receive(samples(clock.instant(300), 10_000));
        let end: i64 = received(&link).iter().map(|(_, _, len)| i64::from(*len)).sum();
        assert_eq!(end, 5_000);
        assert_eq!(clocks(&core), [(0, Some(clock.instant(5_000)))]);
    }

    #[test]
    fn ur_17_a_request_ends_at_the_pending_cut() {
        // UR-17: uhd-rx asks the device for no more than up to a pending cut.
        let (core, _, _, _) = super::super::test_support::rig_with_links(vec![link()]);
        let (mut rx, clock) = rx(&core, 1);
        rx.command(plan(&[(0, Some(1_000), None)]));
        rx.follow();
        assert_eq!(rx.recv_len(), 1_000.min(core.block_len));
        rx.receive(samples(clock.instant(0), 300));
        assert_eq!(rx.recv_len(), 700.min(core.block_len));
    }

    #[test]
    fn ur_25_largest_aligned_cut_is_safe_in_rx_owner() {
        let (core, _, _, _) = super::super::test_support::rig();
        let (mut rx, _) = rx(&core, 1);
        rx.command(plan(&[(0, Some(i64::MAX.div_euclid(200)), None)]));
        rx.follow();
        assert_eq!(rx.recv_len(), core.block_len);
        assert_eq!(rx.recv_timeout(), super::RECV_TIMEOUT);
        rx.receive(crate::device::RxRecv::Timeout);
        assert!(rx.stream.is_some());
    }

    #[test]
    fn ur_25_a_second_change_follows_the_first() {
        // RM-25 (spec 22, VH-2): a second change before the first's origin replaces the
        // first's segment, which has no sample, so no clock: uhd-rx registers a clock at its
        // first block, whether the second plan comes before or after it began the first.
        for began in [false, true] {
            let link = link();
            let (core, _, time, _) = super::super::test_support::rig_with_links(vec![link.clone()]);
            let (mut rx, clock) = rx(&core, 1);
            let ms = |n: i64| core.ticks(n * 1_000_000);
            rx.command(plan(&[(0, Some(10_000), None), (ms(65), None, Some(1))]));
            rx.follow();
            rx.receive(samples(0, 10_000));
            assert!(rx.stream.is_none());
            let second = plan(&[(0, Some(10_000), None), (ms(115), None, Some(2))]);
            if began {
                assert!(rx.begin());
                rx.command(second);
                rx.follow();
                time.advance_to(core.at(ms(70))).unwrap();
                rx.receive(samples(ms(65), 1_000));
                assert!(rx.stream.is_none(), "the first's segment ends at its origin");
            } else {
                rx.command(second);
            }
            assert!(rx.begin());
            rx.receive(samples(ms(115), 1_000));
            let blocks = received(&link);
            assert_eq!(blocks.iter().map(|(_, first, len)| (*first, *len)).collect::<Vec<_>>(), [(0, 10_000), (0, 1_000)], "began {began}");
            assert_eq!(clocks(&core), [(0, Some(clock.instant(10_000))), (ms(115), None)], "began {began}");
        }
    }

    #[test]
    fn ur_25_a_refused_segment_waits_for_the_plan_made_without_it() {
        // RM-25: once the device refuses a segment's configuration, uhd-rx begins nothing until
        // uhd-control's plan made without that segment arrives, whose next segment begins
        // earlier than the old plan placed it.
        let (core, device, _, _) = super::super::test_support::rig_with_links(vec![link()]);
        let (mut rx, _) = rx(&core, 1);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let planned = |origin: i64, cut: Option<i64>, by: Option<u64>, rate: f64| Planned {
            segment: Segment { origin, cut, config: Config { channels: 1, ratio: Rational::new(200, 1).unwrap() }, by },
            settings: Arc::new(ColdConfig::new(origin, crate::device::Settings { rate: Some(rate), ..crate::device::Settings::default() })),
            configured: false,
            clock: None,
        };
        rx.command(RxCmd::Plan(Arc::new(vec![planned(0, Some(10_000), None, 1e6), planned(ms(65), Some(189_000), Some(1), 3.3e6), planned(ms(254), None, Some(2), 1e6)])));
        rx.follow();
        rx.receive(samples(0, 10_000));
        assert!(rx.stream.is_none());
        assert!(rx.begin());
        assert_eq!(lock(&core.streams).refused, [(Dir::Rx, 1)]);
        assert!(!rx.begin(), "the refused plan is not carried out");
        rx.command(RxCmd::Plan(Arc::new(vec![planned(0, Some(10_000), None, 1e6), planned(ms(200), None, Some(2), 1e6)])));
        assert!(rx.begin());
        let starts: Vec<_> = device.calls().into_iter().filter(|c| c.starts_with("rx_start ")).collect();
        assert_eq!(starts, [format!("rx_start {}", ms(200))]);
    }

    #[test]
    fn ur_26_no_segment_begins_once_provider_stop_has_begun() {
        // UR-25, UR-26 (spec 22, VH-4; #58): once `Provider::stop` has begun, uhd-rx starts no
        // planned segment, the one a change booked before it included.
        let (core, device, _, _) = super::super::test_support::rig_with_links(vec![link()]);
        let (mut rx, _) = rx(&core, 1);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        rx.command(plan(&[(0, Some(10_000), None), (ms(65), None, Some(1))]));
        rx.follow();
        rx.command(RxCmd::Cut { at: ms(5), mode: StopMode::Orderly });
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
            rx.command(plan(&[(0, Some(10_000), None), (ms(65), None, Some(1))]));
            rx.follow();
            rx.receive(samples(0, 10_000));
            assert!(rx.begin());
            assert!(device.calls().contains(&format!("rx_start {}", ms(65))));
            // The `Stop` at 20 ms cuts the planned segment at its origin: the plan drops it.
            time.advance_to(core.at(ms(20))).unwrap();
            rx.command(plan(&[(0, Some(10_000), None)]));
            rx.follow();
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
