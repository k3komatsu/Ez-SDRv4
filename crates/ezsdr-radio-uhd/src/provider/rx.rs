//! uhd-rx: owns the receive streamer. It turns `rx_recv` results into Stream Contract
//! blocks on every attached link, gaps into flags, losses into events, and performs the
//! receive side of a `cold` change and of a stop (UR-17…UR-20, UR-25, UR-26, UR-29).

use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration as Wall, Instant};

use ezsdr_hostmem::{HOST_MEMORY, HostPool, write_cf32};
use ezsdr_kernel::contract::DataContractId;
use ezsdr_kernel::event::Severity;
use ezsdr_kernel::module_api::StopMode;
use ezsdr_kernel::stream::{BlockFlags, BlockHeader, ChannelMask, Direction, PublishOutcome, SampleBlock};
use ezsdr_kernel::time::TimePoint;
use ezsdr_radio::kinds;
use ezsdr_radio::payloads::{AlignmentErrorPayload, LateCommandPayload, RxOverflowCause, RxOverflowPayload};
use serde_json::json;

use super::control::ColdConfig;
use super::core::{Clock, Core, lattice, lock};
use crate::device::{Dir, Iq, RxRecv};
use crate::profile::{DELIVERY_ALLOWANCE_NS, DEVICE_LEAD_NS};

pub(crate) enum RxCmd {
    /// A stream enabled from 0 channels, already configured and started (UR-25).
    Enable { clock: Clock, channels: usize },
    /// A `cold` change: the old stream ends at `e1` (UR-25).
    Switch { e1: i64, clock: Option<Clock>, channels: usize, settings: Arc<ColdConfig> },
    /// `Stop` for `<id>/rx` or `<id>`: RM-16's tail (UR-26).
    Stop,
    /// `Provider::stop` began at `at`: the stream ends at `at` plus the tail under
    /// `orderly` and at `at` under `abort` (RM-16, UR-26).
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
    channels: usize,
    expected: i64,
    /// The distinct reports since the last block, in arrival order (UR-17…UR-19).
    pending: Vec<Pending>,
    /// The timed start's instant: T0, or a change's `e₂` (UR-17).
    start: i64,
    /// Samples at or after this root tick are discarded, and the stream ends there: it is
    /// stopped untimed when its samples reach the cut, since the X3x0 ignores a stop's
    /// time (UR-25; design-notes §11 F1).
    cut: Option<i64>,
    /// The device was told to stop (at the cut, or by an abort): not again.
    stopped: bool,
    /// Blocks before this root tick are the previous stream's, still in flight after its
    /// untimed stop, whatever this stream's origin (Review N, B1).
    not_before: i64,
}

impl Stream {
    fn new(clock: Clock, channels: usize, not_before: Option<i64>) -> Stream {
        Stream {
            clock,
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
}

struct Switch {
    clock: Option<Clock>,
    channels: usize,
    /// The channel count before the change: the streamer is reopened only if it differs.
    from: usize,
    settings: Arc<ColdConfig>,
    e1: i64,
}

pub(crate) struct Rx {
    core: Arc<Core>,
    cmds: Receiver<RxCmd>,
    stream: Option<Stream>,
    switch: Option<Switch>,
    pool: HostPool,
    started: Instant,
    last_samples: Instant,
    stall: Option<(Wall, Wall)>,
    first: bool,
    first_block: Option<Instant>,
    exit: bool,
    /// `Provider::stop` has begun: a later switch no longer applies (Review M, N-1).
    stopping: bool,
    /// When the last untimed stop was issued: the next stream drops what precedes it.
    last_stop: Option<i64>,
}

const RECV_TIMEOUT: Wall = Wall::from_millis(100);

impl Rx {
    pub fn new(core: Arc<Core>, cmds: Receiver<RxCmd>, clock: Option<Clock>, channels: usize, stall: Option<(Wall, Wall)>) -> Rx {
        let pool = HostPool::new(core.block_len * channels.max(1) * 8);
        Rx {
            stream: clock.map(|clock| Stream::new(clock, channels, None)),
            core,
            cmds,
            switch: None,
            pool,
            started: Instant::now(),
            last_samples: Instant::now(),
            stall,
            first: true,
            first_block: None,
            exit: false,
            stopping: false,
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
    }

    pub fn run(mut self) {
        loop {
            self.poll();
            if self.exit && self.stream.is_none() {
                return;
            }
            if self.core.is_lost() {
                if self.exit {
                    return;
                }
                std::thread::sleep(Wall::from_millis(1));
                continue;
            }
            if self.stream.is_none() {
                if self.switch.is_some() {
                    self.do_switch();
                } else {
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
            // A stop that arrived during the wait cuts this block too (RM-16).
            self.poll();
            self.receive(result);
        }
    }

    fn command(&mut self, cmd: RxCmd) {
        let tail = self.core.ticks(self.core.description.timing.stop_tail_ns);
        match cmd {
            RxCmd::Enable { clock, channels } => {
                self.pool = HostPool::new(self.core.block_len * channels.max(1) * 8);
                self.stream = Some(Stream::new(clock, channels, self.last_stop.take()));
                self.last_samples = Instant::now();
            }
            RxCmd::Switch { .. } if self.stopping => {
                lock(&self.core.streams).switching[Dir::Rx as usize] = false;
            }
            RxCmd::Switch { e1, clock, channels, settings } => {
                let from = self.stream.as_ref().map_or(0, |stream| stream.channels);
                self.switch = Some(Switch { clock, channels, from, settings, e1 });
                if let Some(stream) = self.stream.as_mut() {
                    // A cut already set (a `Stop` for `<id>/rx`) stays if it is earlier.
                    if stream.cut.is_none_or(|cut| e1 < cut) {
                        stream.cut = Some(e1);
                    }
                }
            }
            RxCmd::Stop => self.stop_orderly(tail),
            RxCmd::Cut { at, mode } => {
                self.switch = None;
                lock(&self.core.streams).switching[Dir::Rx as usize] = false;
                self.stopping = true;
                match mode {
                    StopMode::Orderly => self.stop_at(at, at + tail),
                    StopMode::Abort => {
                        self.core.timing(json!({ "what": "rx_stop", "at": at, "until": at, "mode": "Abort" }));
                        if let Some(stream) = self.stream.as_mut() {
                            stream.cut = Some(at);
                            if !stream.stopped {
                                if let Some(issued) = stop_at_cut(&self.core, at) {
                                    stream.stopped = true;
                                    self.last_stop = Some(issued);
                                }
                            }
                        }
                    }
                }
            }
            RxCmd::Shutdown(mode) => {
                self.exit = true;
                self.switch = None;
                lock(&self.core.streams).switching[Dir::Rx as usize] = false;
                match mode {
                    StopMode::Orderly => self.stop_orderly(tail),
                    StopMode::Abort => {
                        if let Some(stream) = self.stream.take().filter(|stream| !stream.stopped) {
                            self.last_stop = stop_at_cut(&self.core, stream.cut.unwrap_or_else(|| self.core.now()))
                                .or(self.last_stop);
                        }
                    }
                }
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
    /// cut and not at the next 100 ms timeout (UR-25; design-notes §11 F2); never under
    /// 1 ms (UHD truncates the timeout to whole milliseconds).
    fn recv_timeout(&self) -> Wall {
        let Some(stream) = self.stream.as_ref() else { return RECV_TIMEOUT };
        let Some(cut) = stream.cut else { return RECV_TIMEOUT };
        let left = i128::from(cut) + i128::from(quiet_span(&self.core, stream)) - i128::from(self.core.now());
        let ns = (left.max(0) * 1_000_000_000 / i128::from(self.core.mcr))
            .clamp(1_000_000, RECV_TIMEOUT.as_nanos() as i128);
        Wall::from_nanos(ns as u64)
    }

    fn stop_orderly(&mut self, tail: i64) {
        let now = self.core.now();
        self.stop_at(now, now + tail);
    }

    /// RM-16's tail: the stream delivers up to `cut` and then ends.
    fn stop_at(&mut self, now: i64, cut: i64) {
        if let Some(stream) = self.stream.as_mut() {
            if stream.cut.is_some_and(|c| c <= cut) {
                return;
            }
            stream.cut = Some(stream.cut.map_or(cut, |c| c.min(cut)));
            let cut = stream.cut.expect("set above");
            // No timed stop (§11 F1): the stream is stopped untimed when its samples reach
            // the cut (`samples`), and the cut discards the samples after it either way.
            self.core.timing(json!({ "what": "rx_stop", "at": now, "until": cut }));
        }
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
                    let stopped = stream.stopped;
                    self.stream = None;
                    if !stopped {
                        self.last_stop = stop_at_cut(&self.core, cut).or(self.last_stop);
                    }
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
                let restart = match lattice(i128::from(now) + i128::from(self.core.ticks(self.core.description.timing.start_lead_ns)), stream.clock.n) {
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
        if self.stream.is_none() && self.switch.is_some() {
            self.do_switch();
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
                    self.stream = None;
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
            self.stream = None;
        }
    }

    /// The receive side of a `cold` change, once the old stream reached `e1` (UR-25).
    fn do_switch(&mut self) {
        let Some(switch) = self.switch.take() else { return };
        let Some(new) = switch.clock else {
            self.core.timing(json!({ "what": "rx_switch", "e1": switch.e1, "e2": null }));
            lock(&self.core.streams).switching[Dir::Rx as usize] = false;
            return;
        };
        // UR-25: the streamer is reopened when the channel count changed, and only then.
        let reopened = if switch.channels != switch.from { self.core.device.rx_open(switch.channels) } else { Ok(()) };
        let configured = reopened.and_then(|()| switch.settings.configure(&self.core, Dir::Rx, switch.channels));
        let started = configured.and_then(|_| self.core.device.rx_start(new.origin));
        match started {
            Ok(()) => {
                self.pool = HostPool::new(self.core.block_len * switch.channels.max(1) * 8);
                self.stream = Some(Stream::new(new, switch.channels, self.last_stop.take()));
                self.last_samples = Instant::now();
            }
            Err(error) => {
                let _ = self.core.clocks.end(new.domain, self.core.at(new.origin));
                lock(&self.core.streams).rx = None;
                self.core.device_failed("update_parameter", &error);
            }
        }
        self.core.timing(json!({ "what": "rx_switch", "e1": switch.e1, "e2": new.origin, "at": self.core.now() }));
        lock(&self.core.streams).switching[Dir::Rx as usize] = false;
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

    #[test]
    fn ur_19_reports_before_one_block_keep_each_kind() {
        // Reports of different kinds before the next block: that block carries each one's flag
        // and each one's event is emitted with its time jump; a repeated kind is one event
        // (UR-17…UR-19). RX_OVERFLOW goes by the hot path, so only the set is compared.
        use ezsdr_kernel::module_api::Link;
        use ezsdr_kernel::stream::BackPressure;
        let alignment = (kinds::ALIGNMENT_ERROR, BlockFlags::SEQ_DISCONTINUITY);
        let sequence = (kinds::RX_OVERFLOW, BlockFlags::SEQ_DISCONTINUITY);
        let overrun = (kinds::RX_OVERFLOW, BlockFlags::RESTARTED);
        for (reports, expected) in [
            (vec![RxRecv::Alignment], vec![alignment]),
            (vec![RxRecv::Overflow { out_of_sequence: true }], vec![sequence]),
            (vec![RxRecv::Alignment, RxRecv::Overflow { out_of_sequence: true }, RxRecv::Alignment], vec![alignment, sequence]),
            (vec![RxRecv::Overflow { out_of_sequence: false }, RxRecv::Alignment], vec![overrun, alignment]),
        ] {
            let decl = ezsdr_kernel::stream::DataLinkDecl {
                id: ezsdr_kernel::id::DataLinkId::local(0),
                from: ezsdr_kernel::contract::PortRef { component: ezsdr_kernel::spec::Ident::parse("radio").unwrap(), port: ezsdr_kernel::spec::Ident::parse("rx").unwrap() },
                to: ezsdr_kernel::contract::PortRef { component: ezsdr_kernel::spec::Ident::parse("rec").unwrap(), port: ezsdr_kernel::spec::Ident::parse("in").unwrap() },
                contract: DataContractId::parse("ezsdr.stream.cf32").unwrap(),
                policy: BackPressure::DropOldest,
                capacity: 4,
            };
            let link = ezsdr_link_host::HostLinkModule::new().create(&decl).unwrap();
            let (core, _, _, events) = super::super::test_support::rig_with_links(vec![link.clone()]);
            let clock = core.register(Dir::Rx, 200, 0).unwrap();
            let (_to_rx, cmds) = std::sync::mpsc::channel();
            let mut rx = Rx::new(core.clone(), cmds, Some(clock), 2, None);
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
        let clock = core.register(Dir::Rx, 200, 0).unwrap();
        let (_to_rx, cmds) = std::sync::mpsc::channel();
        let mut rx = Rx::new(core.clone(), cmds, Some(clock), 1, None);
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
        let clock = core.register(Dir::Rx, 200, 0).unwrap();
        let (_to_rx, cmds) = std::sync::mpsc::channel();
        let mut rx = Rx::new(core.clone(), cmds, Some(clock), 1, None);
        rx.command(RxCmd::Cut { at: 0, mode: StopMode::Orderly });
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
    fn ur_25_largest_aligned_cut_is_safe_in_rx_owner() {
        let (core, _, _, _) = super::super::test_support::rig();
        let old = core.register(Dir::Rx, 200, 0).unwrap();
        let e1 = i64::MAX.div_euclid(old.n) * old.n;
        let (_to_rx, cmds) = std::sync::mpsc::channel();
        let mut rx = super::Rx::new(core.clone(), cmds, Some(old), 1, None);
        rx.command(super::RxCmd::Switch { e1, clock: None, channels: 0,
            settings: Arc::new(super::ColdConfig::new(e1, crate::device::Settings::default())) });
        assert_eq!(rx.recv_len(), core.block_len);
        assert_eq!(rx.recv_timeout(), super::RECV_TIMEOUT);
        rx.receive(crate::device::RxRecv::Timeout);
        assert!(rx.stream.is_some());
    }

}
