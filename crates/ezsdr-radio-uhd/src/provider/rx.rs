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

use super::core::{Clock, Core, lattice, lock};
use crate::device::{Dir, Iq, RxRecv, Settings};
use crate::profile::RESTART_LEAD_NS;

pub(crate) enum RxCmd {
    /// A stream enabled from 0 channels, already configured and started (UR-25).
    Enable { clock: Clock, channels: usize },
    /// A `cold` change: the old stream ends at `e1` (UR-25).
    Switch { e1: i64, clock: Option<Clock>, channels: usize, settings: Settings },
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
    None,
    Overrun,
    Sequence,
    Alignment,
    MissedStart,
}

struct Stream {
    clock: Clock,
    channels: usize,
    expected: i64,
    pending: Pending,
    /// The timed start's instant: T0, or a change's `e₂` (UR-17).
    start: i64,
    /// Samples at or after this root tick are discarded, and the stream ends there: it is
    /// stopped untimed when its samples reach the cut, since the X3x0 ignores a stop's
    /// time (UR-25; design-notes §11 F1).
    cut: Option<i64>,
}

struct Switch {
    clock: Option<Clock>,
    channels: usize,
    /// The channel count before the change: the streamer is reopened only if it differs.
    from: usize,
    settings: Settings,
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
}

const RECV_TIMEOUT: Wall = Wall::from_millis(100);

impl Rx {
    pub fn new(core: Arc<Core>, cmds: Receiver<RxCmd>, clock: Option<Clock>, channels: usize, stall: Option<(Wall, Wall)>) -> Rx {
        let pool = HostPool::new(core.block_len * channels.max(1) * 8);
        Rx {
            stream: clock.map(|clock| Stream { clock, channels, expected: 0, pending: Pending::None, start: clock.origin, cut: None }),
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
            let result = self.core.device.rx_recv(self.core.block_len, self.recv_timeout());
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
                self.stream = Some(Stream { clock, channels, expected: 0, pending: Pending::None, start: clock.origin, cut: None });
                self.last_samples = Instant::now();
            }
            RxCmd::Switch { .. } if self.stopping => {}
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
                self.stopping = true;
                match mode {
                    StopMode::Orderly => self.stop_at(at, at + tail),
                    StopMode::Abort => {
                        if let Some(stream) = self.stream.as_mut() {
                            stream.cut = Some(at);
                        }
                        self.core.timing(json!({ "what": "rx_stop", "at": at, "until": at, "mode": "Abort" }));
                        if self.stream.is_some() && !self.core.is_lost() {
                            let _ = self.core.device.rx_stop(None);
                        }
                    }
                }
            }
            RxCmd::Shutdown(mode) => {
                self.exit = true;
                self.switch = None;
                match mode {
                    StopMode::Orderly => self.stop_orderly(tail),
                    StopMode::Abort => {
                        if self.stream.take().is_some() && !self.core.is_lost() {
                            let _ = self.core.device.rx_stop(None);
                        }
                    }
                }
            }
        }
    }

    /// How long `rx_recv` may wait: while a cut is pending, no longer than to the cut
    /// plus one block, so that a stream that has stopped yielding is ended within a
    /// block of its cut and not at the next 100 ms timeout (UR-25; design-notes §11 F2).
    fn recv_timeout(&self) -> Wall {
        let Some((cut, n)) = self.stream.as_ref().and_then(|s| s.cut.map(|cut| (cut, s.clock.n))) else { return RECV_TIMEOUT };
        let left = cut - self.core.now() + self.core.block_len as i64 * n;
        Wall::from_nanos(self.core.ns(left.max(0)) as u64).clamp(Wall::from_millis(1), RECV_TIMEOUT)
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
                if let Some(cut) = stream.cut.filter(|cut| now >= *cut) {
                    // Silent past its cut: stopped all the same, never left streaming.
                    self.stream = None;
                    stop_at_cut(&self.core, cut);
                } else if stream.cut.is_none()
                    && now > stream.start + self.core.mcr as i64
                    && self.last_samples.elapsed() > Wall::from_secs(1)
                {
                    self.core.device_lost("UR-29: the receive stream yielded nothing for 1 s");
                }
            }
            RxRecv::Overflow { out_of_sequence: false } => {
                stream.pending = Pending::Overrun;
                self.core.stat("rx_overflows", 1);
            }
            RxRecv::Overflow { out_of_sequence: true } | RxRecv::BadPacket => {
                stream.pending = Pending::Sequence;
                self.core.stat("rx_overflows", 1);
            }
            RxRecv::Alignment => stream.pending = Pending::Alignment,
            RxRecv::LateCommand if stream.cut.is_some() => {
                // A late start of a stream that is ending: stop it untimed, never restart
                // it (Review M, P1-A).
                self.core.timing(json!({ "what": "rx_stop_late", "cut": stream.cut, "at": now }));
                if let Err(error) = self.core.device.rx_stop(None) {
                    self.core.device_failed("stop", &error);
                }
            }
            RxRecv::LateCommand => {
                // UR-17: the device missed the timed start; the origin stays.
                let restart = lattice(now + self.core.ticks(RESTART_LEAD_NS), stream.clock.n);
                let payload = serde_json::to_value(LateCommandPayload {
                    key: None,
                    requested: self.core.at(stream.start),
                    applied: self.core.at(restart),
                })
                .expect("a payload");
                stream.pending = Pending::MissedStart;
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
        if first_tick < clock.origin {
            // The old stream's samples still in flight after its stop (Review M, N-3).
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
                stop_at_cut(&self.core, cut);
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
            let pending = std::mem::replace(&mut stream.pending, Pending::None);
            let mut flags = BlockFlags::NONE;
            if lost.is_some() {
                flags = flags
                    | BlockFlags::GAP_BEFORE
                    | match pending {
                        Pending::Overrun => BlockFlags::RESTARTED,
                        Pending::MissedStart => BlockFlags::NONE,
                        Pending::None | Pending::Sequence | Pending::Alignment => BlockFlags::SEQ_DISCONTINUITY,
                    };
            }
            let channels = stream.channels.max(1);
            stream.expected = k + len;
            let at = TimePoint::new(clock.domain, expected);
            let lost_n = lost.unwrap_or(0);
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
                Pending::None | Pending::MissedStart => {}
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
            return;
        };
        // UR-25: the streamer is reopened when the channel count changed, and only then.
        let reopened = if switch.channels != switch.from { self.core.device.rx_open(switch.channels) } else { Ok(()) };
        let configured = reopened.and_then(|()| self.core.configure(Dir::Rx, switch.channels, &switch.settings));
        let started = configured.and_then(|_| self.core.device.rx_start(new.origin));
        match started {
            Ok(()) => {
                self.pool = HostPool::new(self.core.block_len * switch.channels.max(1) * 8);
                self.stream = Some(Stream {
                    clock: new,
                    channels: switch.channels,
                    expected: 0,
                    pending: Pending::None,
                    start: new.origin,
                    cut: None,
                });
                self.last_samples = Instant::now();
            }
            Err(error) => {
                let _ = self.core.clocks.end(new.domain, self.core.at(new.origin));
                lock(&self.core.streams).rx = None;
                self.core.device_failed("update_parameter", &error);
            }
        }
        self.core.timing(json!({ "what": "rx_switch", "e1": switch.e1, "e2": new.origin, "at": self.core.now() }));
    }
}

/// Stops the stream untimed once its samples reached the cut, and records it (UR-25,
/// UR-30): the X3x0 carries out a timed stop at once, before its time (§11 F1).
fn stop_at_cut(core: &Core, cut: i64) {
    if core.is_lost() {
        return;
    }
    let now = core.now();
    core.timing(json!({ "what": "rx_stop_untimed", "cut": cut, "at": now }));
    match core.device.rx_stop(None) {
        Ok(()) => lock(&core.rec).applied.push(json!({ "key": "rx_stop", "at": core.at(cut), "issued": core.at(now) })),
        Err(error) => core.device_failed("update_parameter", &error),
    }
}
