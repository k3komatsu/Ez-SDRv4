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
use super::core::{Clock, Core, Holder, Restore, RxChain, RxDevice, RxEntry, SegmentKey, lock, stream_key};
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
            streams.started(key, active, stream.clock.origin, stream.config);
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

    /// The chain lock orders plan intent with receive completion and timed handover.
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

    /// Stops only after every handed DDC entry has sample evidence, or after its bounded drain
    /// and one bounded flush attempt. The per-stream clock lock orders this final decision with
    /// control's handover of another timed frequency.
    pub(super) fn try_end(&mut self) {
        let Some(stream) = self.stream.as_ref() else { return };
        let clock = stream.clock;
        let channels = stream.channels;
        let provider_stop = lock(&self.core.streams).stop;
        let core = self.core.clone();
        let mut chain = core.rx_chain[self.k].lock();
        let streams = lock(&self.core.streams);
        refresh_intent(&mut chain, &streams, stream_key(Dir::Rx, self.k), stream);
        drop(streams);
        if self.core.is_lost() || matches!(chain.device(), RxDevice::Lost) {
            let entries = chain.lose();
            self.report_leftovers(entries, "device lost before pending DDC entries were drained");
            self.finish_locked(&mut chain);
            return;
        }
        let cut = chain.cut().unwrap_or_else(|| {
            assert!(chain.end_now());
            clock.instant(self.stream.as_ref().expect("a stream").expected)
        });
        let active = matches!(chain.device(), RxDevice::Armed { .. } | RxDevice::Streaming);
        let pending = !chain.debt().is_empty();
        let flush_deadline = pending.then(|| rx_drain_deadline(&self.core, self.k, clock.n, cut, chain.debt())).flatten();
        if let Some(deadline) = flush_deadline {
            let flush_at = deadline - i128::from(self.core.ticks(DELIVERY_ALLOWANCE_NS));
            if i128::from(self.core.now()) < flush_at {
                return;
            }
        }
        let restore_needed = provider_stop.is_some_and(|(at, _)| chain.latest().values().any(|entry| entry.order.0 > at || entry.accepted_at.is_some_and(|accepted| accepted > at)));
        if restore_needed && matches!(chain.restore(), Restore::NotAsked) {
            return;
        }

        let mut stopped = !active;
        if active {
            let now = self.core.now();
            self.core.timing(json!({ "what": "rx_stop_untimed", "cut": cut, "at": now }));
            let result = self.core.device.rx_stop(self.k, None);
            match chain.stop_result(result) {
                Ok(()) => {
                    lock(&self.core.rec).applied.push(json!({ "key": "rx_stop", "at": self.core.at(cut), "issued": self.core.at(now) }));
                    self.last_stop = Some(self.core.now() + self.core.ticks(DEVICE_LEAD_NS)).or(self.last_stop);
                    stopped = true;
                }
                Err(error) => {
                    self.core.device_failed(None, "update_parameter", &error);
                    if provider_stop.is_some() {
                        self.skip_restore_rx(&mut chain, &format!("receive stop failed; restore was not attempted: {}", error.message));
                    }
                }
            }
        }
        if stopped && !self.core.is_lost() && !chain.debt().is_empty() {
            if let Some(deadline) = flush_deadline {
                self.flush_rx(&mut chain, clock.n, channels, deadline);
            }
        }
        if self.core.is_lost() && !matches!(chain.device(), RxDevice::Lost) {
            let entries = chain.lose();
            self.report_leftovers(entries, "device lost before pending DDC entries were drained");
        }
        if provider_stop.is_some() && !self.core.is_lost() {
            if matches!(chain.device(), RxDevice::Off) {
                drop(chain);
                self.restore_rx();
                chain = core.rx_chain[self.k].lock();
            } else {
                self.skip_restore_rx(&mut chain, "receive stop failed; restore was not attempted");
            }
        }
        self.core.probe("rx_before_finish", None);
        self.finish_locked(&mut chain);
    }

    /// One immediate-start flush, bounded by the drain's remaining hard deadline.
    fn flush_rx(&self, chain: &mut RxChain, sample_ticks: i64, channels: Channels, deadline: i128) {
        let call = self.core.block_len.max(self.core.device.rx_packet_samples(self.k));
        let packet = self.core.device.rx_packet_samples(self.k);
        match chain.flush_start_result(self.core.device.rx_start_now(self.k)) {
            Err(error) => self.core.device_failed(None, "rx_drain", &error),
            Ok(()) => {
                while !chain.debt().is_empty() && i128::from(self.core.now()) < deadline && !self.core.is_lost() {
                    let left = (deadline - i128::from(self.core.now())).max(0);
                    let timeout_ns = left * 1_000_000_000 / i128::from(self.core.mcr);
                    if timeout_ns < 1_000_000 {
                        break;
                    }
                    let len = bounded_read_len(call, packet, left, sample_ticks);
                    if len == 0 {
                        break;
                    }
                    let timeout = Wall::from_nanos(timeout_ns.min(u64::MAX as i128) as u64);
                    match self.core.device.rx_recv(self.k, len, timeout, true) {
                        RxRecv::Samples { first_tick, samples } => chain.consume(sample_ticks, channels, first_tick, &samples),
                        RxRecv::Failed(error) if error.lost => {
                            self.core.device_lost(&error.message);
                            let entries = chain.lose();
                            self.report_leftovers(entries, "device lost before pending DDC entries were drained");
                        }
                        RxRecv::Failed(_error) => self.core.stat("rx_errors", 1),
                        RxRecv::Timeout | RxRecv::Overflow { .. } | RxRecv::Alignment | RxRecv::BadPacket | RxRecv::LateCommand => {}
                    }
                }
                if !self.core.is_lost() {
                    let result = self.core.device.rx_stop(self.k, None);
                    if let Err(error) = chain.flush_stop_result(result) {
                        self.core.device_failed(None, "rx_drain", &error);
                    }
                }
            }
        }
    }

    fn finish(&mut self) {
        let core = self.core.clone();
        let mut chain = core.rx_chain[self.k].lock();
        if let Some(stream) = self.stream.as_ref() {
            let streams = lock(&core.streams);
            refresh_intent(&mut chain, &streams, stream_key(Dir::Rx, self.k), stream);
        }
        if self.core.is_lost() {
            let entries = chain.lose();
            self.report_leftovers(entries, "device lost before pending DDC entries were drained");
        }
        self.finish_locked(&mut chain);
        if !self.core.is_lost() && lock(&self.core.streams).stop.is_some() {
            if matches!(chain.device(), RxDevice::Off) {
                drop(chain);
                self.restore_rx();
            } else {
                self.skip_restore_rx(&mut chain, "receive chain was not safely stopped; restore was not attempted");
            }
        }
    }

    fn finish_locked(&mut self, chain: &mut RxChain) {
        let stream = self.stream.take();
        let cut = chain.cut();
        let entries = chain.finish();
        self.report_leftovers(entries, "receive segment finished with an unconsumed DDC value");
        if let Some(stream) = stream {
            let key = stream_key(Dir::Rx, self.k);
            let mut streams = lock(&self.core.streams);
            if let (None, Some(cut)) = (&stream.handle, cut) {
                if let Err(error) = self.core.clocks.end(stream.clock.domain, self.core.at(cut)) {
                    self.core.reject_note(json!({ "clock_not_ended": error.to_string() }));
                }
            }
            if let Some(active) = streams.finished(key) {
                streams.complete(key, active);
            }
            let by = streams.line(key).and_then(|line| line.segment(stream.clock.origin, stream.config)).and_then(|segment| segment.by);
            streams.prune(key, by);
        }
    }

    fn report_leftovers(&self, entries: Vec<RxEntry>, reason: &str) {
        for entry in entries {
            self.core.reject_note(json!({
                "call": "rx_drain", "stream": self.k, "channel": entry.channel,
                "value": entry.value, "at": entry.order.0, "error": reason,
            }));
        }
    }

    /// Restores the receive frequency in force at Provider::stop's original S.
    fn restore_rx(&self) {
        let restore = self.core.rx_chain[self.k].lock().take_restore();
        let Some(restore) = restore else { return };
        self.core.probe("rx_restore_before_chain_scan", None);
        let elsewhere: Vec<_> = (0..self.core.rx_chain.len())
            .filter(|&k| k != self.k)
            .flat_map(|k| {
                let chain = self.core.rx_chain[k].lock();
                chain.debt().iter().chain(chain.left()).cloned().collect::<Vec<_>>()
            })
            .collect();
        let chain = self.core.rx_chain[self.k].lock();
        for (channel, value) in restore {
            if self.core.is_lost() {
                return;
            }
            let local = chain.debt().iter().chain(chain.left()).find(|entry| entry.channel == channel).cloned();
            let other = elsewhere.iter().find(|entry| entry.channel == channel).cloned();
            if let Some(entry) = local.or(other) {
                self.record_restore_failure(channel, value, &format!("not restored because timed DDC value {} at {} remains on an RX chain", entry.value, entry.order.0));
                continue;
            }
            let settings = crate::device::Settings { freq: Some(value), ..crate::device::Settings::default() };
            match self.core.device.apply(Dir::Rx, channel, &settings, None) {
                Ok(applied) => lock(&self.core.rec).applied.push(json!({
                    "key": self.core.description.applied_key(Dir::Rx, channel, self.k, "frequency_hz"),
                    "claimed": value, "read_back": applied.freq, "at": self.core.at(self.core.now()), "restore": true,
                })),
                Err(error) => {
                    self.record_restore_failure(channel, value, &error.message);
                    self.core.device_failed(None, "rx_restore", &error);
                    if error.lost {
                        return;
                    }
                }
            }
        }
    }

    fn skip_restore_rx(&self, chain: &mut RxChain, reason: &str) {
        let Some(restore) = chain.take_restore() else { return };
        for (channel, value) in restore {
            self.record_restore_failure(channel, value, reason);
        }
    }

    fn record_restore_failure(&self, channel: usize, value: f64, error: &str) {
        lock(&self.core.rec).applied.push(json!({
            "key": self.core.description.applied_key(Dir::Rx, channel, self.k, "frequency_hz"),
            "claimed": value, "read_back": null, "at": self.core.at(self.core.now()), "restore": true,
            "error": error,
        }));
        self.core.reject_note(json!({ "call": "rx_restore", "stream": self.k, "error": format!("channel c{channel} value {value}: {error}") }));
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
    /// refusal, and the plan it then reads keeps the next segment where it was (RM-25).
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
                streams.started(key, Some((origin, next.segment.config, next.segment.by)), origin, next.segment.config);
                drop(streams);
                drop(chain);
                self.last_samples = self.host.now();
                self.follow();
            }
            Err(error) => {
                let stop = self.core.device.rx_stop(self.k, None);
                if let Err(stop_error) = chain.stop_result(stop) {
                    self.core.device_failed(None, "update_parameter", &stop_error);
                }
                let entries = chain.finish();
                self.report_leftovers(entries, "receive segment could not register its sample clock");
                drop(chain);
                self.refuse_segment(&next, None, crate::device::DeviceError { lost: false, message: format!("UR-25: {error}") });
            }
        }
        self.core.timing(json!({ "what": "rx_segment", "origin": origin, "channels": channels.count(), "at": self.core.now() }));
        true
    }

    /// L1 never hands a timed value to an unbegun segment, so refusal has no new chain debt.
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
        self.core.device_failed(None, "update_parameter", &error);
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
        let mut len = if left <= 0 && !chain.debt().is_empty() { block } else { left.clamp(1, block as i64) as usize };
        if let Some(deadline) = rx_drain_deadline(&self.core, self.k, stream.clock.n, cut, chain.debt()) {
            let remaining = deadline - i128::from(self.core.now());
            let packet = self.core.device.rx_packet_samples(self.k);
            len = bounded_read_len(len, packet, remaining, stream.clock.n);
        }
        len
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
        let ns = rx_drain_deadline(&self.core, self.k, stream.clock.n, cut, chain.debt())
            .map_or(ns, |deadline| ns.min((deadline - i128::from(self.core.now())).max(0) * 1_000_000_000 / i128::from(self.core.mcr)));
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
                let restart = match ezsdr_radio::timeline::lattice(
                    now.saturating_add(self.core.ticks(ezsdr_radio::timeline::start_lead_ns(&self.core.description.timing, Direction::Rx))),
                    stream.config.ratio,
                ) {
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
        let (clock, channels) = self.stream.as_ref().map(|stream| (stream.clock, stream.channels)).expect("a stream");
        let mut chain = self.core.rx_chain[self.k].lock();
        let segment = chain.segment().expect("an active RX stream has a chain segment");
        chain.first_sample(segment);
        chain.consume(clock.n, channels, first_tick, &samples);
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

fn rx_drain_deadline(core: &Core, k: usize, sample_ticks: i64, cut: i64, pending: &[RxEntry]) -> Option<i128> {
    let latest = pending.iter().map(|entry| entry.order.0).max()?;
    let call = core.block_len.max(core.device.rx_packet_samples(k)) as i128;
    let allowance = i128::from(core.ticks(DELIVERY_ALLOWANCE_NS));
    let quiet = i128::from(cut) + i128::from(core.block_len as i64) * i128::from(sample_ticks);
    Some((i128::from(latest) + call * i128::from(sample_ticks)).max(quiet) + allowance)
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

/// Keep each UHD receive to one packet and to no more samples than fit before the deadline.
fn bounded_read_len(requested: usize, packet: usize, remaining_ticks: i128, sample_ticks: i64) -> usize {
    if remaining_ticks <= 0 {
        return 0;
    }
    let by_time = usize::try_from((remaining_ticks / i128::from(sample_ticks)).max(1)).unwrap_or(usize::MAX);
    requested.min(if packet == 0 { requested } else { packet }).min(by_time)
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
    use super::super::core::{Phase, RxDevice, Streams};
    use super::*;
    use crate::device::Device;
    use ezsdr_kernel::module_api::Link;
    use ezsdr_kernel::stream::BackPressure;
    use ezsdr_kernel::stream::{ContinuityBuilder, DropCarry, GapCause};
    use ezsdr_radio::payloads::RxOverflowPayload;
    use ezsdr_radio::timeline::{Item, Kind, Line, NO_RELEASE, Segment};
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

    fn rx_owner_on(core: &Arc<Core>, k: usize) -> Rx {
        let (to_rx, cmds) = std::sync::mpsc::channel();
        std::mem::forget(to_rx);
        Rx::new(core.clone(), k, cmds, None, None)
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
                lead: core.ticks(ezsdr_radio::timeline::start_lead_ns(&core.description.timing, Direction::Rx)),
                call: core.block_len as i64,
                release_window: core.ticks(core.description.timing.release_window_ns),
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

    fn device_at_spy(core: &Core, time: &ezsdr_kernel::time::ManualTimeAuthority, device: &super::super::test_support::LockSpy, ticks: i64) {
        if core.now() < ticks {
            time.advance_to(core.at(ticks)).unwrap();
        }
        device.set_time(ticks);
        assert_eq!((core.now(), device.time_now().unwrap()), (ticks, ticks));
    }

    #[allow(clippy::too_many_arguments)]
    fn fail_drain_to_left(
        core: &Arc<Core>,
        time: &Arc<ezsdr_kernel::time::ManualTimeAuthority>,
        device: &Arc<super::super::test_support::LockSpy>,
        control: &mut super::super::control::Control,
        frozen: super::super::test_support::Frozen,
        mut rx: Rx,
        k: usize,
        instant: i64,
        cut: i64,
        provider_stop_at: Option<i64>,
    ) -> (Rx, super::super::test_support::Frozen) {
        let device_at = |ticks| {
            if core.now() < ticks {
                time.advance_to(core.at(ticks)).unwrap();
            }
            device.set_time(ticks);
            assert_eq!((core.now(), device.time_now().unwrap()), (ticks, ticks));
        };
        let stalled = device.rx_recv(k, core.block_len, Wall::ZERO, false);
        assert!(matches!(&stalled, RxRecv::Timeout), "the old receive flow stalls: {stalled:?}; calls={:?}", device.log());
        if provider_stop_at.is_none() {
            core.rx_chain[k].lock().request_cut(cut);
        }
        let release = instant - core.ticks(core.description.timing.release_window_ns);
        device_at(release);
        control.release();
        assert_eq!(core.rx_chain[k].lock().debt().len(), 1, "Control hands the current segment's frequency");
        rx.receive(device.rx_recv(k, core.block_len, Wall::ZERO, false));
        assert_eq!(core.rx_chain[k].lock().debt().len(), 1, "the stalled normal receive cannot prove consumption");
        if let Some(at) = provider_stop_at {
            assert_eq!(at, cut);
            device_at(at);
            let mut streams = lock(&core.streams);
            streams.stop = Some((at, StopMode::Orderly));
            streams.end(core, at, false, false);
            drop(streams);
            control.finish();
        }

        let chain = core.rx_chain[k].lock();
        let deadline = rx_drain_deadline(core, k, 200, cut, chain.debt()).unwrap();
        drop(chain);
        let flush_at = deadline - i128::from(core.ticks(DELIVERY_ALLOWANCE_NS));
        device_at(i64::try_from(flush_at).unwrap());
        let gate = device.stop_after("rx_recv");
        let (done, finished) = std::sync::mpsc::channel();
        let worker = core
            .device
            .host()
            .spawn("rx-leftover-drain", move || {
                rx.try_end();
                let _ = done.send(rx);
            })
            .unwrap();
        drop(frozen);
        gate.wait();
        let frozen = super::super::test_support::Frozen::on(device.host());
        let deadline = i64::try_from(deadline).unwrap().max(device.time_now().unwrap());
        device_at_spy(core, time, device, deadline);
        gate.release();
        let rx = finished.recv_timeout(Wall::from_secs(10)).expect("the failed bounded flush completes");
        worker.join().unwrap();
        assert!(rx.stream.is_none());
        assert_eq!(device.dsp_pending(Dir::Rx, k), 1, "the normal drain and bounded flush leave the fake DDC value pending");
        assert_eq!(core.rx_chain[k].lock().left().len(), 1);
        assert_eq!(lock(&core.rec).rejected.iter().filter(|row| row["call"] == "rx_drain").count(), 1, "finish reports the leftover once");
        (rx, frozen)
    }

    /// Books a receive command as uhd-control does, its segment's configuration with it.
    fn book(core: &Core, e: i64, seq: u64, kind: Kind) {
        let key = stream_key(Dir::Rx, 0);
        let mut streams = lock(&core.streams);
        streams.stream_mut(key).configs.insert(seq, Arc::new(ColdConfig::new(BTreeMap::new())));
        assert!(streams.book(core, key, Item { e, seq, ready: 0, release: NO_RELEASE, delivered: None, refused: false, kind }));
    }

    fn accept_entry(chain: &mut RxChain, entry: RxEntry) {
        if !matches!(chain.device(), RxDevice::Armed { .. } | RxDevice::Streaming) {
            chain.adopt_start(SegmentKey { by: None, origin: entry.order.0 }, entry.order.0);
        }
        let accepted_at = entry.accepted_at.unwrap_or(entry.order.0);
        chain.hand(entry, || Ok(()), || accepted_at).unwrap();
    }

    #[test]
    fn ur_24_equal_time_rx_entries_need_distinct_samples_after_acceptance() {
        let mut chain = RxChain::default();
        chain.adopt_start(SegmentKey { by: None, origin: 0 }, 0);
        for entry in [
            RxEntry { order: (0, 1), channel: 0, value: 1.0, accepted_at: Some(200) },
            RxEntry { order: (0, 2), channel: 0, value: 2.0, accepted_at: Some(200) },
            RxEntry { order: (0, 3), channel: 1, value: 3.0, accepted_at: Some(0) },
        ] {
            accept_entry(&mut chain, entry);
        }
        let clock = Clock { domain: ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, origin: 0, n: 200 };
        let channels = Channels::first(2);
        let samples = vec![vec![[0.0, 0.0]; 1], vec![[0.0, 0.0]; 1]];
        chain.consume(clock.n, channels, 0, &samples);
        assert_eq!(chain.debt().iter().map(|entry| entry.order.1).collect::<Vec<_>>(), [1, 2], "the channel-1 sample and the pre-acceptance channel-0 sample cannot consume either channel-0 entry");
        chain.consume(clock.n, channels, 200, &samples);
        assert_eq!(chain.debt().iter().map(|entry| entry.order.1).collect::<Vec<_>>(), [2], "one sample consumes only one equal-time entry on channel 0");
        chain.consume(clock.n, channels, 400, &samples);
        assert!(chain.debt().is_empty());
    }

    #[test]
    fn ur_24_a_cut_after_timed_rx_release_drains_before_its_successor() {
        // Owner path: the timed handover is accepted first, a later cold booking cuts the
        // segment before its instant, and the block spanning the cut drains the DDC without
        // publishing its tail. A 20 ms receive call makes the minimum 3 ms lead observable.
        let device = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, rx_packet: Some(20_000), ..Default::default() }));
        let mut description = super::super::test_support::default_description(2_000);
        description.timing.rx_start_lead_ns = 3_000_000;
        let output = link();
        let (core, time, _) = super::super::test_support::rig_on_description(device.clone(), vec![output.clone()], BTreeMap::new(), description);
        let ticks = |ms: i64| core.ticks(ms * 1_000_000);
        let frozen = super::super::test_support::Frozen::of(&device);
        let t0 = core.now();
        device.set_time(t0);
        let (cut, e) = (t0 + ticks(8), t0 + ticks(10));
        device.apply(Dir::Rx, 0, &crate::device::Settings { rate: Some(1e6), freq: Some(1e9), ..Default::default() }, None).unwrap();
        device.rx_open(0, &[0]).unwrap();
        let mut config = core.description.defaults();
        config.insert(super::super::core::key("radio.rx.c0.stream"), ezsdr_kernel::spec::Value::from(0));
        let mut rx_start = vec![None; core.description.stream_count(Dir::Rx)];
        rx_start[0] = Some((Channels::first(1), 200));
        let start = super::super::control::Start { t0, rx: rx_start, tx: vec![None; core.description.stream_count(Dir::Tx)] };
        let (to_tx, _tx) = std::sync::mpsc::channel();
        let mut control = super::super::control::Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], config, false, start);
        lock(&core.streams).record_open(stream_key(Dir::Rx, 0), Some(Channels::first(1)));
        device.rx_start(0, t0).unwrap();
        let handle = core.clocks.declare_sample_clock(core.stream_node(Dir::Rx, 0), core.root, Rational::new(200, 1).unwrap()).unwrap();
        let clock = Clock { domain: handle.id(), origin: t0, n: 200 };
        let (to_rx, cmds) = std::sync::mpsc::channel();
        std::mem::forget(to_rx);
        let mut rx = Rx::new(core.clone(), 0, cmds, Some((handle, Channels::first(1), 0)), None);

        control.book_timed(
            ezsdr_kernel::event::ActionId(1),
            super::super::core::key("radio.rx.c0.frequency_hz"),
            ezsdr_kernel::spec::Value::num(1.001e9).unwrap(),
            Some(ezsdr_kernel::time::AbsoluteDeadline::new(core.at(e))),
        );
        super::super::test_support::device_at(&core, &time, &device, t0 + ticks(7));
        control.release();
        assert_eq!(core.rx_chain[0].lock().debt().len(), 1, "the accepted timed value is an RX drain obligation");
        assert_eq!(device.dsp_pending(Dir::Rx, 0), 1);

        let same_rate = Config { channels: Channels::first(1), ratio: Rational::new(200, 1).unwrap() };
        book(&core, cut, 2, Kind::Cold(same_rate));
        let planned = lock(&core.streams).planned(stream_key(Dir::Rx, 0));
        let successor = planned.last().unwrap().segment.origin;
        assert_eq!(planned[0].segment.instant(planned[0].segment.cut.unwrap()).unwrap(), cut);
        assert_eq!(successor, t0 + ticks(37), "cut + 3 ms release + 20 ms receive call + 3 ms allowance + 3 ms lead");

        super::super::test_support::device_at(&core, &time, &device, e);
        drop(frozen);
        let block = device.rx_recv(0, 20_000, Wall::from_millis(100), false);
        assert!(matches!(&block, RxRecv::Samples { first_tick: 0, .. }), "the real fake receive call spans the cut: {block:?}");
        rx.receive(block);

        assert!(rx.stream.is_none(), "the old owner stopped after the sample evidence");
        assert!(core.rx_chain[0].lock().debt().is_empty());
        assert_eq!(device.dsp_pending(Dir::Rx, 0), 0, "the DDC entry was consumed");
        let (published, _) = output.receive().expect("the pre-cut prefix is published");
        assert_eq!(published.header().len, ((cut - t0) / 200) as u32);
        assert_eq!(published.header().first_sample_time.ticks_in(clock.domain).unwrap(), 0);
        assert!(output.receive().is_none(), "the tail at or after the cut is not published");
        let published = lock(&core.rec).timing.iter().find(|row| row["what"] == "rx_stop_untimed").cloned().expect("the owner stopped");
        assert_eq!(published["cut"], cut);
        assert!(successor - published["at"].as_i64().unwrap() >= ticks(3), "the full minimum RX lead remains after the drained stop: {published}, successor={successor}");
    }

    #[test]
    fn ur_24_a_stop_decision_waits_for_a_gated_timed_rx_handover() {
        let spy = Arc::new(super::super::test_support::LockSpy::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..Default::default() }));
        let output = link();
        let (core, time, _) = super::super::test_support::rig_on(spy.clone(), vec![output.clone()], 2_000);
        spy.watch(&core);
        let t0 = core.now();
        spy.set_time(t0);
        let ticks = |ms: i64| core.ticks(ms * 1_000_000);
        core.device.apply(Dir::Rx, 0, &crate::device::Settings { rate: Some(1e6), freq: Some(1e9), ..Default::default() }, None).unwrap();
        core.device.rx_open(0, &[0]).unwrap();
        let mut config = core.description.defaults();
        config.insert(super::super::core::key("radio.rx.c0.stream"), ezsdr_kernel::spec::Value::from(0));
        let mut rx_start = vec![None; core.description.stream_count(Dir::Rx)];
        rx_start[0] = Some((Channels::first(1), 200));
        let start = super::super::control::Start { t0, rx: rx_start, tx: vec![None; core.description.stream_count(Dir::Tx)] };
        let (to_tx, _tx) = std::sync::mpsc::channel();
        let mut control = super::super::control::Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], config, false, start);
        lock(&core.streams).record_open(stream_key(Dir::Rx, 0), Some(Channels::first(1)));
        core.device.rx_start(0, t0).unwrap();
        let handle = core.clocks.declare_sample_clock(core.stream_node(Dir::Rx, 0), core.root, Rational::new(200, 1).unwrap()).unwrap();
        let clock = Clock { domain: handle.id(), origin: t0, n: 200 };
        let (to_rx, cmds) = std::sync::mpsc::channel();
        std::mem::forget(to_rx);
        let mut rx = Rx::new(core.clone(), 0, cmds, Some((handle, Channels::first(1), t0)), None);
        rx.receive(samples(t0, 2_000));

        let e = t0 + ticks(5);
        control.book_timed(
            ezsdr_kernel::event::ActionId(1),
            super::super::core::key("radio.rx.c0.frequency_hz"),
            ezsdr_kernel::spec::Value::num(1.001e9).unwrap(),
            Some(ezsdr_kernel::time::AbsoluteDeadline::new(core.at(e))),
        );
        let cut = t0 + ticks(8);
        book(&core, cut, 2, Kind::Cold(Config { channels: Channels::first(1), ratio: Rational::new(200, 1).unwrap() }));
        let release_at = e - ticks(core.description.timing.release_window_ns / 1_000_000);
        time.advance_to(core.at(release_at)).unwrap();
        spy.set_time(release_at);
        let gate = spy.stop_after("apply");
        let (control_done, control_returned) = std::sync::mpsc::channel();
        let control_thread = core
            .device
            .host()
            .spawn("rx-d9b-handover", move || {
                control.release();
                let _ = control_done.send(control);
            })
            .unwrap();
        gate.wait();

        let after_e = e + ticks(1);
        time.advance_to(core.at(after_e)).unwrap();
        spy.set_time(after_e);
        let (owner_done, owner_returned) = std::sync::mpsc::channel();
        let owner_thread = core
            .device
            .host()
            .spawn("rx-d9b-stop", move || {
                rx.receive(samples(after_e, 2_000));
                let _ = owner_done.send(rx);
            })
            .unwrap();
        let deadline = std::time::Instant::now() + Wall::from_secs(10);
        while !(core.rx_chain[0].is_held() && core.rx_chain[0].waiters() > 0) && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(core.rx_chain[0].is_held() && core.rx_chain[0].waiters() > 0, "the stop decision did not wait behind the handover lock");
        assert_eq!(spy.seen("rx_stop"), 0, "the chain stopped before the accepted handover was published");

        gate.release();
        let _control = control_returned.recv_timeout(Wall::from_secs(10)).expect("the timed handover completed");
        control_thread.join().unwrap();
        let rx = owner_returned.recv_timeout(Wall::from_secs(10)).expect("the receive owner completed");
        owner_thread.join().unwrap();

        assert!(rx.stream.is_none());
        assert!(core.rx_chain[0].lock().debt().is_empty(), "the accepted value was not drained");
        let order = spy.order();
        let apply = order.iter().rposition(|name| *name == "apply").unwrap();
        let stop = order.iter().position(|name| *name == "rx_stop").unwrap();
        assert!(apply < stop, "the timed apply and stop crossed: {order:?}");
        let (published, _) = output.receive().expect("the prefix before the cut is published");
        assert_eq!(published.header().first_sample_time.ticks_in(clock.domain).unwrap(), 0);
        let (tail_prefix, _) = output.receive().expect("the delivered pre-cut drain prefix");
        assert_eq!((tail_prefix.header().first_sample_time.ticks_in(clock.domain).unwrap(), tail_prefix.header().len), ((after_e - t0) / 200, 2_000));
        assert!(tail_prefix.header().first_sample_time.ticks_in(clock.domain).unwrap() + i64::from(tail_prefix.header().len) <= (cut - t0) / 200, "the tail at or after the cut was published");
        assert!(output.receive().is_none(), "the drain tail at or after the cut was published");
    }

    #[test]
    fn ur_24_a_completed_rx_owner_serializes_a_late_frequency() {
        let device = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..Default::default() }));
        let (core, time, _) = super::super::test_support::rig_on_description(device.clone(), vec![link()], BTreeMap::new(), super::super::test_support::default_description(2_000));
        let _frozen = super::super::test_support::Frozen::of(&device);
        let t0 = core.now();
        device.set_time(t0);
        let ticks = |ms: i64| core.ticks(ms * 1_000_000);
        core.device.apply(Dir::Rx, 0, &crate::device::Settings { rate: Some(1e6), freq: Some(1e9), ..Default::default() }, None).unwrap();
        core.device.rx_open(0, &[0]).unwrap();
        let mut config = core.description.defaults();
        config.insert(super::super::core::key("radio.rx.c0.stream"), ezsdr_kernel::spec::Value::from(0));
        let mut rx_start = vec![None; core.description.stream_count(Dir::Rx)];
        rx_start[0] = Some((Channels::first(1), 200));
        let start = super::super::control::Start { t0, rx: rx_start, tx: vec![None; core.description.stream_count(Dir::Tx)] };
        let (to_tx, _tx) = std::sync::mpsc::channel();
        let mut control = super::super::control::Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], config, false, start);
        lock(&core.streams).record_open(stream_key(Dir::Rx, 0), Some(Channels::first(1)));
        core.device.rx_start(0, t0).unwrap();
        let handle = core.clocks.declare_sample_clock(core.stream_node(Dir::Rx, 0), core.root, Rational::new(200, 1).unwrap()).unwrap();
        let (to_rx, cmds) = std::sync::mpsc::channel();
        let mut rx = Rx::new(core.clone(), 0, cmds, Some((handle, Channels::first(1), t0)), None);
        rx.receive(samples(t0, 2_000));

        let e = t0 + ticks(100);
        let cut = e + ticks(10);
        book(&core, cut, 2, Kind::Stop);
        control.book_timed(
            ezsdr_kernel::event::ActionId(1),
            super::super::core::key("radio.rx.c0.frequency_hz"),
            ezsdr_kernel::spec::Value::num(1.001e9).unwrap(),
            Some(ezsdr_kernel::time::AbsoluteDeadline::new(core.at(e))),
        );
        let release_at = e - core.ticks(core.description.timing.release_window_ns);
        super::super::test_support::device_at(&core, &time, &device, release_at);
        core.rx_chain[0].lock().request_cut(cut);

        let (owner_ready, owner_at_finish) = std::sync::mpsc::channel();
        let (resume_owner, owner_resume) = std::sync::mpsc::channel();
        let owner_resume = std::sync::Mutex::new(owner_resume);
        let gate_core = core.clone();
        super::super::test_support::probe(&core, "rx_before_finish", move |_| {
            assert!(gate_core.rx_chain[0].is_held(), "the owner retains the chain lock through completion publication");
            let _ = owner_ready.send(());
            let _ = owner_resume.lock().unwrap().recv();
        });
        let (owner_done, owner_returned) = std::sync::mpsc::channel();
        let owner = core
            .device
            .host()
            .spawn("rx-finish-frequency-race", move || {
                rx.try_end();
                drop(to_rx);
                let _ = owner_done.send(rx);
            })
            .unwrap();
        owner_at_finish.recv_timeout(Wall::from_secs(10)).expect("the stopped owner reached completion");

        let (control_ready, control_at_apply) = std::sync::mpsc::channel();
        super::super::test_support::probe(&core, "before_timed_apply", move |_| {
            let _ = control_ready.send(());
        });
        let (control_done, control_returned) = std::sync::mpsc::channel();
        let control_thread = core
            .device
            .host()
            .spawn("rx-late-frequency", move || {
                control.release();
                let _ = control_done.send(());
            })
            .unwrap();
        control_at_apply.recv_timeout(Wall::from_secs(10)).expect("control reached the timed handover");
        let deadline = std::time::Instant::now() + Wall::from_secs(10);
        while core.rx_chain[0].waiters() == 0 && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        let waited_for_completion = core.rx_chain[0].waiters() > 0;

        resume_owner.send(()).unwrap();
        let rx = owner_returned.recv_timeout(Wall::from_secs(10)).expect("the owner completed the segment");
        owner.join().unwrap();
        control_returned.recv_timeout(Wall::from_secs(10)).expect("the late update settled");
        control_thread.join().unwrap();

        assert!(waited_for_completion, "control did not serialize with the owner's completion");
        assert!(rx.stream.is_none());
        assert!(!device.calls().iter().any(|call| call.starts_with("apply rx 0") && call.contains("freq=1001000000")), "a completed segment received a late device write: {:?}", device.calls());
        assert!(core.rx_chain[0].lock().debt().is_empty(), "completion left a stale DDC obligation");
        assert!(lock(&core.streams).active(stream_key(Dir::Rx, 0)).is_none(), "the owner recorded completion before handing over the lock");
    }

    #[test]
    fn ur_24_provider_stop_drains_after_s_then_restores_the_frequency_in_force_at_s() {
        let device = Arc::new(super::super::test_support::LockSpy::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, rx_packet: Some(1_000), ..Default::default() }));
        let output = link();
        let (core, time, _) = super::super::test_support::rig_on_description(device.clone(), vec![output.clone()], BTreeMap::new(), super::super::test_support::default_description(2_000));
        device.watch(&core);
        let frozen = super::super::test_support::Frozen::on(device.host());
        let device_at = |ticks| {
            if core.now() < ticks {
                time.advance_to(core.at(ticks)).unwrap();
            }
            device.set_time(ticks);
            assert_eq!((core.now(), device.time_now().unwrap()), (ticks, ticks));
        };
        let t0 = core.now();
        device.set_time(t0);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let (at_s, a, b) = (t0 + ms(8), 1.001e9, 1.002e9);
        device.apply(Dir::Rx, 0, &crate::device::Settings { rate: Some(1e6), freq: Some(1e9), ..Default::default() }, None).unwrap();
        device.rx_open(0, &[0]).unwrap();

        let mut config = core.description.defaults();
        config.insert(super::super::core::key("radio.rx.c0.stream"), ezsdr_kernel::spec::Value::from(0));
        let frequency = super::super::core::key("radio.rx.c0.frequency_hz");
        let mut rx_start = vec![None; core.description.stream_count(Dir::Rx)];
        rx_start[0] = Some((Channels::first(1), 200));
        let start = super::super::control::Start { t0, rx: rx_start, tx: vec![None; core.description.stream_count(Dir::Tx)] };
        let (to_tx, _tx) = std::sync::mpsc::channel();
        let mut control = super::super::control::Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], config, false, start);
        lock(&core.streams).record_open(stream_key(Dir::Rx, 0), Some(Channels::first(1)));
        device.rx_start(0, t0).unwrap();
        let handle = core.clocks.declare_sample_clock(core.stream_node(Dir::Rx, 0), core.root, Rational::new(200, 1).unwrap()).unwrap();
        let (to_rx, cmds) = std::sync::mpsc::channel();
        std::mem::forget(to_rx);
        let mut rx = Rx::new(core.clone(), 0, cmds, Some((handle, Channels::first(1), t0)), None);

        control.book_timed(ezsdr_kernel::event::ActionId(1), frequency.clone(), ezsdr_kernel::spec::Value::num(a).unwrap(), Some(ezsdr_kernel::time::AbsoluteDeadline::new(core.at(t0 + ms(5)))));
        device_at(t0 + ms(2));
        control.release();
        device_at(t0 + ms(5));
        control.release();
        assert!(lock(&core.rec).applied.iter().any(|row| row["key"] == "radio.rx.c0.frequency_hz" && row["at"]["ticks"] == t0 + ms(5)), "the due A row was folded into the record");
        assert_eq!(core.rx_chain[0].lock().debt().len(), 1, "A's drain obligation survives row folding");
        control.book_timed(ezsdr_kernel::event::ActionId(2), frequency, ezsdr_kernel::spec::Value::num(b).unwrap(), Some(ezsdr_kernel::time::AbsoluteDeadline::new(core.at(t0 + ms(10)))));
        device_at(t0 + ms(7));
        control.release();
        assert_eq!(device.dsp_pending(Dir::Rx, 0), 1, "A took effect before S; only the post-S B remains in the DDC");
        assert!(device.dsp_log(Dir::Rx, 0).iter().any(|(at, value, timed)| *at <= t0 + ms(6) && *value == a && *timed));

        device_at(at_s);
        {
            let mut streams = lock(&core.streams);
            streams.stop = Some((at_s, StopMode::Orderly));
            streams.end(&core, at_s, false, false);
        }
        control.finish();
        assert!(matches!(core.rx_chain[0].lock().restore(), Restore::Planned(values) if values.get(&0) == Some(&a)));
        assert!(lock(&core.rec).applied.iter().any(|row| row["key"] == "radio.rx.c0.frequency_hz" && row["cancelled_at"]["ticks"] == at_s), "B is cancelled at the original S");

        for first in (0..((at_s - t0) / 200) as usize).step_by(2_000) {
            let len = (((at_s - t0) / 200) as usize - first).min(2_000);
            rx.receive(samples(t0 + first as i64 * 200, len));
        }
        assert_eq!(core.rx_chain[0].lock().debt().len(), 1, "the pre-S samples consume A but not B");
        core.rx_chain[0].lock().request_cut(at_s);
        let chain = core.rx_chain[0].lock();
        let deadline = rx_drain_deadline(&core, 0, 200, at_s, chain.debt()).unwrap();
        drop(chain);
        let flush_at = deadline - i128::from(core.ticks(DELIVERY_ALLOWANCE_NS));
        device_at(i64::try_from(flush_at).unwrap());
        drop(frozen);
        rx.try_end();

        assert!(rx.stream.is_none());
        assert_eq!(device.dsp_pending(Dir::Rx, 0), 0, "the stream consumed both timed values before stopping");
        let restored_at = device.time_now().unwrap();
        assert!(
            device.dsp_log(Dir::Rx, 0).iter().any(|(at, value, timed)| *at <= restored_at && *value == a && !timed),
            "the device is restored before another Run can configure it: {:?}",
            device.dsp_log(Dir::Rx, 0)
        );
        let rec = lock(&core.rec);
        let restores: Vec<_> = rec.applied.iter().filter(|row| row["restore"] == true).collect();
        assert_eq!(restores.len(), 1, "restoration happens once, after the pending value is drained: {restores:?}");
        assert_eq!((restores[0]["claimed"].as_f64(), restores[0]["read_back"].as_f64()), (Some(a), Some(a)));
        assert!(!rec.rejected.iter().any(|row| row["call"] == "rx_restore"), "restore was not attempted while the DDC value was pending: {:?}", rec.rejected);
        drop(rec);
        let order = device.order();
        let consumed = order.iter().rposition(|call| *call == "rx_recv").unwrap();
        let restored = order.iter().rposition(|call| *call == "apply").unwrap();
        assert!(consumed < restored, "the flush receive returned before the restore apply: {order:?}");
        assert!(
            device.rx_apply_pending().iter().any(|(channel, at, value, pending)| *channel == 0 && at.is_none() && *value == a && *pending == 0),
            "the restore apply observed the DDC entry consumed: {:?}",
            device.rx_apply_pending()
        );
        let published = received(&output);
        assert_eq!(published.iter().map(|(_, _, len)| *len).sum::<u32>(), ((at_s - t0) / 200) as u32, "the pre-S prefix is published");
        assert_eq!(published.iter().map(|(_, tick, _)| *tick).collect::<Vec<_>>(), [0, 2_000, 4_000, 6_000]);
        assert!(output.receive().is_none(), "no sample at or after S is published");
    }

    #[test]
    fn ur_24_multichannel_abort_keeps_overflow_and_drain_samples_off_the_link() {
        let device = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, ..Default::default() }));
        let output = link();
        let (core, time, _) = super::super::test_support::rig_on_description(device.clone(), vec![output.clone()], BTreeMap::new(), super::super::test_support::default_description(2_000));
        let _frozen = super::super::test_support::Frozen::of(&device);
        device.set_time(core.now());
        let (mut rx, clock) = rx(&core, 2);
        device.rx_start(0, 0).unwrap();
        let entries = [(core.ticks(1_000_000), 1, 0, 1.001e9), (core.ticks(2_000_000), 2, 1, 1.002e9), (core.ticks(2_000_000), 3, 1, 1.003e9)];
        {
            let mut chain = core.rx_chain[0].lock();
            for &(at, seq, channel, value) in &entries {
                device.apply(Dir::Rx, channel, &crate::device::Settings { freq: Some(value), ..Default::default() }, Some(at)).unwrap();
                let entry = super::super::core::RxEntry { order: (at, seq), channel, value, accepted_at: Some(0) };
                accept_entry(&mut chain, entry);
            }
        }

        rx.receive(RxRecv::Overflow { out_of_sequence: false });
        assert_eq!(core.rx_chain[0].lock().debt().len(), 3, "an overflow is not sample evidence");
        rx.command(RxCmd::Shutdown(StopMode::Abort));
        assert_eq!(core.rx_chain[0].lock().debt().len(), 3, "abort does not abandon handed DDC entries");
        let after_e = core.ticks(6_000_000);
        super::super::test_support::device_at(&core, &time, &device, after_e);
        let drained = device.rx_recv(0, 6_000, Wall::from_millis(10), false);
        assert!(matches!(&drained, RxRecv::Samples { first_tick: 0, .. }), "the fake DDC kept flowing through all handed entries: {drained:?}");
        rx.receive(drained);

        assert!(rx.stream.is_none());
        assert!(core.rx_chain[0].lock().debt().is_empty(), "samples on each channel prove all three commands consumed");
        assert_eq!(device.dsp_pending(Dir::Rx, 0), 0);
        assert_eq!(device.dsp_pending(Dir::Rx, 1), 0);
        assert!(output.receive().is_none(), "abort publishes no overflow/drain tail at or after its cut");
        assert_eq!(clock.origin, 0);
    }

    #[test]
    fn ur_24_fake_ddc_distinguishes_host_silence_from_a_stalled_ddc() {
        for (fault, pending) in [(crate::device::FakeFault::Silence(Wall::ZERO), 0), (crate::device::FakeFault::RxFlowStall(Wall::ZERO), 1)] {
            let device = crate::device::FakeDevice::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, faults: vec![fault], ..Default::default() });
            let _frozen = super::super::test_support::Frozen::of(&device);
            device.set_time(0);
            device.rx_open(0, &[0]).unwrap();
            device.rx_start(0, 0).unwrap();
            drop(_frozen);
            assert!(matches!(device.rx_recv(0, 2_000, Wall::from_millis(1), false), RxRecv::Timeout));
            let at = 200_000;
            device.apply(Dir::Rx, 0, &crate::device::Settings { freq: Some(1.001e9), ..Default::default() }, Some(at)).unwrap();
            device.set_time(400_000);
            assert!(matches!(device.rx_recv(0, 2_000, Wall::from_millis(1), false), RxRecv::Timeout), "no host block is delivered");
            assert_eq!(device.dsp_pending(Dir::Rx, 0), pending, "host silence and stopped chain flow are distinct");
            assert_eq!(device.dsp_log(Dir::Rx, 0).len(), usize::from(pending == 0), "only flowing DDC samples take the timed entry");
        }
    }

    #[test]
    fn ur_24_drain_deadline_runs_one_bounded_flush_and_consumes_or_reports() {
        for (succeeds, near_deadline) in [(true, false), (false, false), (false, true)] {
            let spy = Arc::new(super::super::test_support::LockSpy::new(crate::device::FakeConfig {
                clock: crate::ClockMode::Manual,
                rx_latency: Wall::from_millis(1),
                rx_packet: Some(1_000),
                faults: if succeeds {
                    vec![crate::device::FakeFault::RxFlowStall(Wall::ZERO)]
                } else {
                    vec![crate::device::FakeFault::RxFlowStall(Wall::ZERO), crate::device::FakeFault::RxFlowStall(Wall::from_micros(1))]
                },
                ..Default::default()
            }));
            let output = link();
            let mut description = super::super::test_support::default_description(20_000);
            description.timing.rx_start_lead_ns = description.timing.release_window_ns;
            let (core, time, _) = super::super::test_support::rig_on_description(spy.clone(), vec![output], BTreeMap::new(), description);
            spy.watch(&core);
            let frozen = super::super::test_support::Frozen::on(spy.host());
            spy.set_time(core.now());
            let (mut rx, clock) = rx(&core, 1);
            let start = core.now();
            let call = core.block_len.max(core.device.rx_packet_samples(0)) as i64;
            {
                let mut streams = lock(&core.streams);
                let key = stream_key(Dir::Rx, 0);
                let mut line = streams.line(key).unwrap().clone();
                line.stream.call = call;
                streams.lines.insert(key, line);
            }
            let call_ticks = call * clock.n;
            let allowance = core.ticks(DELIVERY_ALLOWANCE_NS);
            let release_window = core.ticks(core.description.timing.release_window_ns);
            let lead = core.ticks(core.description.timing.rx_start_lead_ns);
            assert!(release_window > 0);
            let successor = start + release_window + call_ticks + allowance + lead;
            plan(&core, &[(start, Some(start), None), (successor, None, Some(1))]);
            spy.apply(Dir::Rx, 0, &crate::device::Settings { rate: Some(1e6), freq: Some(1e9), ..Default::default() }, None).unwrap();
            let instant = start + release_window;
            spy.apply(Dir::Rx, 0, &crate::device::Settings { freq: Some(1.001e9), ..Default::default() }, Some(instant)).unwrap();
            assert!(matches!(spy.rx_recv(0, call as usize, Wall::ZERO, false), RxRecv::Timeout), "the old receive flow did not stall");
            assert_eq!(spy.dsp_pending(Dir::Rx, 0), 1, "the stalled old flow leaves the timed DDC entry pending");
            let entry = super::super::core::RxEntry { order: (instant, 1), channel: 0, value: 1.001e9, accepted_at: Some(start) };
            {
                let mut chain = core.rx_chain[0].lock();
                accept_entry(&mut chain, entry);
            }
            core.rx_chain[0].lock().request_cut(start);
            let flush_at = (instant + call_ticks).max(start + core.block_len as i64 * clock.n);
            let flush_deadline = flush_at + allowance;
            assert_eq!(flush_deadline, successor - lead, "the bounded flush ends at the shared post-cut readiness floor");
            let before_deadline = flush_at - 1;
            time.advance_to(core.at(before_deadline)).unwrap();
            spy.set_time(before_deadline);
            rx.try_end();
            assert_eq!(spy.seen("rx_stop"), 0, "the regular drain stopped before its packet-duration boundary");
            assert_eq!(spy.seen("rx_start"), 1, "the flush began before the packet-duration boundary");
            assert_eq!(spy.seen("rx_start_now"), 0);

            time.advance_to(core.at(flush_at)).unwrap();
            spy.set_time(flush_at);
            let start_gate = spy.stop_after("rx_start_now");
            let recv_gate = (!near_deadline).then(|| spy.stop_before("rx_recv"));
            let (done, finished) = std::sync::mpsc::channel();
            let host = core.device.host();
            let worker = host
                .spawn("rx-d9b-flush", move || {
                    rx.try_end();
                    let _ = done.send(rx);
                })
                .unwrap();
            start_gate.wait();
            assert_eq!(spy.seen("rx_stop"), 1, "the cut stop precedes the one flush");
            assert_eq!(spy.seen("rx_start"), 1);
            assert_eq!(spy.seen("rx_start_now"), 1);
            assert_eq!(spy.seen("rx_recv"), 1, "only the old-flow stall has read before the flush");
            if near_deadline {
                let last_safe = flush_deadline - core.ticks(500_000);
                time.advance_to(core.at(last_safe)).unwrap();
                spy.set_time(last_safe);
            }
            start_gate.release();
            if let Some(recv_gate) = recv_gate {
                recv_gate.wait();
                time.advance_to(core.at(flush_deadline)).unwrap();
                spy.set_time(flush_deadline);
                drop(frozen);
                recv_gate.release();
            } else {
                drop(frozen);
            }
            let rx = finished.recv_timeout(Wall::from_secs(10)).expect("the bounded owner drain completed");
            worker.join().unwrap();
            assert!(rx.stream.is_none());
            assert!(core.rx_chain[0].lock().debt().is_empty());
            assert_eq!(core.rx_chain[0].lock().left().len(), usize::from(!succeeds));
            assert_eq!(spy.seen("rx_start"), 1);
            assert_eq!(spy.seen("rx_start_now"), 1);
            assert_eq!(spy.seen("rx_recv"), 1 + usize::from(!near_deadline), "the old-flow stall occurs once; a flush read is skipped when less than 1 ms remains");
            let requested = spy.rx_requests();
            assert_eq!(requested.first(), Some(&(call as usize)));
            if near_deadline {
                assert_eq!(requested.len(), 1);
            } else {
                assert_eq!(requested.get(1), Some(&1_000), "a flush read cannot span multiple device packets");
            }
            assert_eq!(spy.seen("rx_stop"), 2, "the owner cuts once and stops the one flush once");
            let order = spy.order();
            let first_stop = order.iter().position(|name| *name == "rx_stop").unwrap();
            let flush_start = order.iter().position(|name| *name == "rx_start_now").unwrap();
            let final_stop = order.iter().rposition(|name| *name == "rx_stop").unwrap();
            assert!(first_stop < flush_start && flush_start < final_stop, "the owner did not stop, flush once, then stop: {order:?}");
            if !near_deadline {
                let flush_recv = order.iter().rposition(|name| *name == "rx_recv").unwrap();
                assert!(flush_start < flush_recv && flush_recv < final_stop, "the owner did not read between the flush start and stop: {order:?}");
            }
            assert!(successor - core.now() >= lead, "the flush consumed the successor's RX lead: successor={successor}, now={}", core.now());
            let rejected = lock(&core.rec).rejected.clone();
            assert_eq!(rejected.iter().filter(|row| row["call"] == "rx_drain").count(), usize::from(!succeeds));
            if !succeeds {
                assert!(rejected.iter().any(|row| row["channel"] == 0 && row["value"] == 1.001e9 && row["at"] == instant), "{rejected:?}");
            }
        }
    }

    #[test]
    fn ur_24_a_left_and_latest_survive_close_open_and_chain_reuse() {
        let spy = Arc::new(super::super::test_support::LockSpy::new(crate::device::FakeConfig {
            clock: crate::ClockMode::Manual,
            rx_packet: Some(1_000),
            faults: vec![crate::device::FakeFault::RxFlowStall(Wall::ZERO), crate::device::FakeFault::RxFlowStall(Wall::from_micros(1))],
            ..Default::default()
        }));
        let (core, time, _) = super::super::test_support::rig_on_description(spy.clone(), vec![link()], BTreeMap::new(), super::super::test_support::default_description(2_000));
        spy.watch(&core);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let t0 = core.now();
        let (old, successor) = (Config { channels: Channels::first(1), ratio: Rational::new(200, 1).unwrap() }, Config { channels: Channels::first(2), ratio: Rational::new(200, 1).unwrap() });
        let (cut, e, next_origin) = (t0 + ms(12), t0 + ms(8), t0 + ms(20));
        let config = core.description.defaults();
        let rx_start = {
            let mut starts = vec![None; core.description.stream_count(Dir::Rx)];
            starts[0] = Some((old.channels, 200));
            starts
        };
        let (to_tx, _tx) = std::sync::mpsc::channel();
        let mut control = super::super::control::Control::new(
            core.clone(),
            Arc::new(NoActions),
            vec![to_tx],
            config.clone(),
            false,
            super::super::control::Start { t0, rx: rx_start, tx: vec![None; core.description.stream_count(Dir::Tx)] },
        );
        let segments = [Segment { origin: t0, cut: Some((cut - t0) / 200), config: old, by: None }, Segment { origin: next_origin, cut: None, config: successor, by: Some(1) }];
        let frozen = super::super::test_support::Frozen::on(spy.host());
        device_at_spy(&core, &time, &spy, t0);
        let (mut rx, clock) = rx_with_plan(&core, &segments);
        control.book_timed(
            ezsdr_kernel::event::ActionId(1),
            super::super::core::key("radio.rx.c0.frequency_hz"),
            ezsdr_kernel::spec::Value::num(1.001e9).unwrap(),
            Some(ezsdr_kernel::time::AbsoluteDeadline::new(core.at(e))),
        );
        let (rx_after_drain, frozen) = fail_drain_to_left(&core, &time, &spy, &mut control, frozen, rx, 0, e, cut, None);
        rx = rx_after_drain;
        assert_eq!(core.rx_chain[0].lock().latest().get(&0).map(|entry| entry.value), Some(1.001e9));

        let mark = spy.log().len();
        assert!(rx.begin(), "the reused owner begins the configured successor");
        assert!(spy.log()[mark..].iter().any(|call| call == "rx_open 0 [0, 1]"), "the channel-set change reopens the idle chain: {:?}", spy.log());
        assert_eq!(spy.dsp_pending(Dir::Rx, 0), 1, "reopen preserves the old fake DDC value");
        let next_sample = next_origin + ms(2);
        device_at_spy(&core, &time, &spy, next_sample);
        let reused = spy.rx_recv(0, 2_000, Wall::ZERO, false);
        assert!(matches!(&reused, RxRecv::Samples { .. }), "the new segment supplies qualifying samples: {reused:?}");
        rx.receive(reused);
        assert_eq!(spy.dsp_pending(Dir::Rx, 0), 0, "the stale value is consumed by the reused chain");
        let chain = core.rx_chain[0].lock();
        assert!(chain.left().is_empty(), "sample evidence removes the reported leftover");
        assert_eq!(chain.latest().get(&0).map(|entry| entry.value), Some(1.001e9), "consumption does not discard latest");
        drop(chain);

        let (at_s, future) = (t0 + ms(39), t0 + ms(40));
        control.book_timed(
            ezsdr_kernel::event::ActionId(2),
            super::super::core::key("radio.rx.c0.frequency_hz"),
            ezsdr_kernel::spec::Value::num(1.002e9).unwrap(),
            Some(ezsdr_kernel::time::AbsoluteDeadline::new(core.at(future))),
        );
        device_at_spy(&core, &time, &spy, future - core.ticks(core.description.timing.release_window_ns));
        control.release();
        assert_eq!(core.rx_chain[0].lock().latest().get(&0).map(|entry| entry.value), Some(1.002e9));
        device_at_spy(&core, &time, &spy, at_s);
        {
            let mut streams = lock(&core.streams);
            streams.stop = Some((at_s, StopMode::Orderly));
            streams.end(&core, at_s, false, false);
        }
        control.finish();
        assert!(matches!(core.rx_chain[0].lock().restore(), Restore::Planned(values) if values.get(&0) == Some(&1.001e9)));
        let chain = core.rx_chain[0].lock();
        let deadline = rx_drain_deadline(&core, 0, clock.n, at_s, chain.debt()).unwrap();
        drop(chain);
        let flush_at = deadline - i128::from(core.ticks(DELIVERY_ALLOWANCE_NS));
        device_at_spy(&core, &time, &spy, i64::try_from(flush_at).unwrap());
        drop(frozen);
        rx.try_end();

        assert!(rx.stream.is_none());
        assert_eq!(spy.dsp_pending(Dir::Rx, 0), 0);
        assert!(core.rx_chain[0].lock().left().is_empty());
        assert_eq!(core.rx_chain[0].lock().latest().get(&0).map(|entry| entry.value), Some(1.002e9));
        assert_eq!(lock(&core.rec).rejected.iter().filter(|row| row["call"] == "rx_drain").count(), 1, "reuse did not report the same entry twice");
        assert!(lock(&core.rec).applied.iter().any(|row| row["restore"] == true && row["claimed"].as_f64() == Some(1.001e9)), "Provider::stop restores the value in force at S from retained latest");
        assert_eq!(spy.dsp_log(Dir::Rx, 0).last().map(|(_, value, timed)| (*value, *timed)), Some((1.001e9, false)), "inspect restoration before another configure: {:?}", spy.dsp_log(Dir::Rx, 0));
    }

    #[test]
    fn ur_24_provider_stop_skips_restore_for_left_on_this_chain() {
        let spy = Arc::new(super::super::test_support::LockSpy::new(crate::device::FakeConfig {
            clock: crate::ClockMode::Manual,
            rx_packet: Some(1_000),
            faults: vec![crate::device::FakeFault::RxFlowStall(Wall::ZERO), crate::device::FakeFault::RxFlowStall(Wall::from_micros(1))],
            ..Default::default()
        }));
        let (core, time, _) = super::super::test_support::rig_on_description(spy.clone(), vec![link()], BTreeMap::new(), super::super::test_support::default_description(2_000));
        spy.watch(&core);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let t0 = core.now();
        let (cut, e) = (t0 + ms(7), t0 + ms(8));
        let config = core.description.defaults();
        let mut rx_start = vec![None; core.description.stream_count(Dir::Rx)];
        rx_start[0] = Some((Channels::first(1), 200));
        let (to_tx, _tx) = std::sync::mpsc::channel();
        let mut control = super::super::control::Control::new(
            core.clone(),
            Arc::new(NoActions),
            vec![to_tx],
            config,
            false,
            super::super::control::Start { t0, rx: rx_start, tx: vec![None; core.description.stream_count(Dir::Tx)] },
        );
        let segment = Segment { origin: t0, cut: None, config: Config { channels: Channels::first(1), ratio: Rational::new(200, 1).unwrap() }, by: None };
        let frozen = super::super::test_support::Frozen::on(spy.host());
        device_at_spy(&core, &time, &spy, t0);
        let (rx, _) = rx_with_plan(&core, &[segment]);
        control.book_timed(
            ezsdr_kernel::event::ActionId(1),
            super::super::core::key("radio.rx.c0.frequency_hz"),
            ezsdr_kernel::spec::Value::num(1.001e9).unwrap(),
            Some(ezsdr_kernel::time::AbsoluteDeadline::new(core.at(e))),
        );
        let (mut rx, frozen) = fail_drain_to_left(&core, &time, &spy, &mut control, frozen, rx, 0, e, cut, None);

        assert_eq!(core.rx_chain[0].lock().left().len(), 1, "the earlier failed drain left the value on its owning chain");
        let at_s = cut;
        {
            let mut streams = lock(&core.streams);
            streams.stop = Some((at_s, StopMode::Orderly));
            streams.end(&core, at_s, false, false);
        }
        control.finish();
        assert!(matches!(core.rx_chain[0].lock().restore(), Restore::Planned(_)), "Provider::stop plans restoration from its original S");
        let calls_before_finish = spy.log().len();
        drop(frozen);
        rx.finish();

        assert_eq!(spy.log().len(), calls_before_finish, "Provider::stop does not overwrite an already-reported local leftover");
        assert!(matches!(core.rx_chain[0].lock().restore(), Restore::Done));
        assert_eq!(core.rx_chain[0].lock().left().len(), 1, "finish preserves same-chain left");
        assert_eq!(spy.dsp_pending(Dir::Rx, 0), 1, "the timed DDC value remains on the device");
        assert_eq!(spy.seen("apply"), 1, "only the original timed apply reached the device");
        assert!(lock(&core.rec).rejected.iter().any(|row| row["call"] == "rx_restore" && row["error"].as_str().is_some_and(|error| error.contains("remains on an RX chain"))));
        assert!(!lock(&core.rec).applied.iter().any(|row| row["restore"] == true && row["read_back"].is_number()));
    }

    #[test]
    fn ur_24_a_local_leftover_blocks_restore() {
        let (core, device, _, _) = super::super::test_support::rig_with(crate::device::FakeConfig { clock: crate::ClockMode::Manual, ..Default::default() }, vec![link()]);
        let (mut rx, _) = rx(&core, 1);
        let t0 = core.now();
        let instant = t0 + core.ticks(8_000_000);
        let mut chain = core.rx_chain[0].lock();
        chain
            .hand(
                RxEntry { order: (instant, 1), channel: 0, value: 1.001e9, accepted_at: None },
                || core.device.apply(Dir::Rx, 0, &crate::device::Settings { freq: Some(1.001e9), ..Default::default() }, Some(instant)).map(|_| ()),
                || core.now(),
            )
            .unwrap();
        chain.stop_result(core.device.rx_stop(0, None)).unwrap();
        chain.plan_restore(BTreeMap::from([(0, 1e9)]));
        rx.finish_locked(&mut chain);
        drop(chain);

        let calls_before_restore = device.calls().len();
        rx.restore_rx();

        assert!(rx.stream.is_none());
        assert_eq!(core.rx_chain[0].lock().left().len(), 1);
        assert!(device.calls()[calls_before_restore..].iter().all(|call| !call.starts_with("apply rx 0")), "restore must not overwrite a local leftover: {:?}", device.calls());
        assert!(lock(&core.rec).rejected.iter().any(|row| row["call"] == "rx_restore" && row["error"].as_str().is_some_and(|error| error.contains("remains on an RX chain"))));
    }

    #[test]
    fn ur_24_a_left_stays_on_the_old_chain_after_channel_move() {
        let spy = Arc::new(super::super::test_support::LockSpy::new(crate::device::FakeConfig {
            clock: crate::ClockMode::Manual,
            rx_packet: Some(1_000),
            faults: vec![crate::device::FakeFault::RxFlowStall(Wall::ZERO), crate::device::FakeFault::RxFlowStall(Wall::from_micros(1))],
            ..Default::default()
        }));
        let links = vec![vec![link()], vec![link()]];
        let (core, time, _) = super::super::test_support::rig_on_description_links(spy.clone(), links, BTreeMap::new(), layout_description());
        spy.watch(&core);
        let frozen = super::super::test_support::Frozen::on(spy.host());
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let (t0, e, cut) = (core.now(), ms(8), ms(12));
        let old = Channels::first(1);
        let mut config = core.description.defaults();
        config.insert(super::super::core::key("radio.rx.c0.stream"), ezsdr_kernel::spec::Value::from(0));
        config.insert(super::super::core::key("radio.rx.c1.stream"), ezsdr_kernel::spec::Value::from(-1));
        let (to_tx, _tx) = std::sync::mpsc::channel();
        let mut control = super::super::control::Control::new(
            core.clone(),
            Arc::new(NoActions),
            vec![to_tx],
            config,
            false,
            super::super::control::Start { t0, rx: vec![Some((old, 200)), None], tx: vec![None; core.description.stream_count(Dir::Tx)] },
        );
        let target = super::super::test_support::target_of(&core.id);
        control.book(ezsdr_kernel::module_api::Dispatched::new(
            ezsdr_kernel::event::ActionId(1),
            ezsdr_kernel::event::Action::UpdateParameter {
                target: target.clone(),
                key: super::super::core::key("radio.rx.c0.stream"),
                value: ezsdr_kernel::spec::Value::from(1),
                class: ezsdr_kernel::module_api::UpdateClass::Cold,
                at: Some(ezsdr_kernel::time::AbsoluteDeadline::new(core.at(cut))),
            },
            Some(core.id.clone()),
        ));
        control.book_timed(
            ezsdr_kernel::event::ActionId(2),
            super::super::core::key("radio.rx.c0.frequency_hz"),
            ezsdr_kernel::spec::Value::num(1.001e9).unwrap(),
            Some(ezsdr_kernel::time::AbsoluteDeadline::new(core.at(e))),
        );
        device_at_spy(&core, &time, &spy, t0);
        let handle = core.clocks.declare_sample_clock(core.stream_node(Dir::Rx, 0), core.root, Rational::new(200, 1).unwrap()).unwrap();
        spy.rx_open(0, &[0]).unwrap();
        lock(&core.streams).record_open(stream_key(Dir::Rx, 0), Some(old));
        spy.rx_start(0, t0).unwrap();
        let rx = Rx::new(core.clone(), 0, std::sync::mpsc::channel().1, Some((handle, old, t0)), None);
        let (mut rx, frozen) = fail_drain_to_left(&core, &time, &spy, &mut control, frozen, rx, 0, e, cut, None);
        {
            let old_chain = core.rx_chain[0].lock();
            assert_eq!(old_chain.left().len(), 1);
            assert_eq!(old_chain.left()[0].channel, 0);
        }
        assert_eq!(spy.dsp_pending(Dir::Rx, 0), 1, "the stale DDC value is still on the moved channel before destination configure");

        let destination = lock(&core.streams)
            .planned(stream_key(Dir::Rx, 1))
            .into_iter()
            .find(|planned| planned.segment.config.channels == Channels::first(1) && planned.segment.origin > cut)
            .expect("the move plans channel 0 on stream 1");
        assert!(!rx.begin(), "the completed source has no segment after its channel moves");
        assert!(spy.log().iter().any(|call| call == "rx_close 0"), "the source releases the moved channel: {:?}", spy.log());
        let mut rx1 = rx_owner_on(&core, 1);
        rx1.from = destination.segment.origin;
        device_at_spy(&core, &time, &spy, destination.segment.origin);
        assert!(rx1.begin(), "the moved channel's destination opens and starts");
        assert_eq!(spy.dsp_pending(Dir::Rx, 0), 0, "the destination flow consumes the old timed value at its first qualifying sample");

        let moved_sample = destination.segment.origin + ms(2);
        device_at_spy(&core, &time, &spy, moved_sample);
        let samples = spy.rx_recv(1, 2_000, Wall::ZERO, false);
        assert!(matches!(&samples, RxRecv::Samples { .. }), "the destination receives qualifying samples: {samples:?}");
        rx1.receive(samples);
        assert_eq!(spy.dsp_pending(Dir::Rx, 0), 0, "the destination stream physically consumes the old DDC value");
        assert_eq!(core.rx_chain[0].lock().left().len(), 1, "sample evidence on stream 1 cannot remove stream 0's reported leftover");

        let (at_s, future) = (destination.segment.origin + ms(19), destination.segment.origin + ms(20));
        control.book_timed(
            ezsdr_kernel::event::ActionId(3),
            super::super::core::key("radio.rx.c0.frequency_hz"),
            ezsdr_kernel::spec::Value::num(1.002e9).unwrap(),
            Some(ezsdr_kernel::time::AbsoluteDeadline::new(core.at(future))),
        );
        device_at_spy(&core, &time, &spy, future - core.ticks(core.description.timing.release_window_ns));
        control.release();
        assert_eq!(core.rx_chain[1].lock().debt().len(), 1, "the destination owns its new timed value");
        device_at_spy(&core, &time, &spy, at_s);
        {
            let mut streams = lock(&core.streams);
            streams.stop = Some((at_s, StopMode::Orderly));
            streams.end(&core, at_s, false, false);
        }
        control.finish();
        assert!(matches!(core.rx_chain[1].lock().restore(), Restore::Planned(values) if values.get(&0) == Some(&1.001e9)));

        device_at_spy(&core, &time, &spy, future + ms(2));
        for _ in 0..30 {
            if core.rx_chain[1].lock().debt().is_empty() {
                break;
            }
            let samples = spy.rx_recv(1, 2_000, Wall::ZERO, false);
            assert!(matches!(&samples, RxRecv::Samples { .. }), "the final in-flight value gets sample evidence: {samples:?}");
            rx1.receive(samples);
        }
        assert!(core.rx_chain[1].lock().debt().is_empty());
        assert_eq!(core.rx_chain[0].lock().left().len(), 1, "the destination did not transfer the leftover between chains");
        drop(frozen);
        rx1.try_end();

        assert!(rx1.stream.is_none());
        assert_eq!(spy.dsp_pending(Dir::Rx, 0), 0);
        assert_eq!(core.rx_chain[0].lock().left().len(), 1);
        let rejected = lock(&core.rec).rejected.clone();
        assert_eq!(rejected.iter().filter(|row| row["call"] == "rx_drain").count(), 1);
        assert!(
            rejected.iter().any(|row| row["call"] == "rx_restore" && row["error"].as_str().is_some_and(|error| error.contains("remains on an RX chain"))),
            "restore sees the old chain's leftover: {rejected:?}"
        );
        let restores: Vec<_> = lock(&core.rec).applied.iter().filter(|row| row["restore"] == true).cloned().collect();
        assert_eq!(restores.len(), 1, "the blocked restore is reported once: {restores:?}");
        assert!(restores[0]["read_back"].is_null() && restores[0]["error"].is_string(), "the restore was not applied over the old chain's leftover: {restores:?}");
        assert_eq!(
            spy.dsp_log(Dir::Rx, 0).last().map(|(_, value, timed)| (*value, *timed)),
            Some((1.002e9, true)),
            "inspect destination state before another configure: {:?}",
            spy.dsp_log(Dir::Rx, 0)
        );
        drop(rx);
    }

    #[test]
    fn ur_24_a_deadline_receive_uses_one_short_packet_and_preserves_successor_lead() {
        let spy = Arc::new(super::super::test_support::LockSpy::new(crate::device::FakeConfig {
            clock: crate::ClockMode::Manual,
            rx_latency: Wall::from_micros(6_500),
            rx_packet: Some(500),
            ..Default::default()
        }));
        let mut description = super::super::test_support::default_description(20_000);
        description.timing.rx_start_lead_ns = description.timing.release_window_ns;
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
        spy.apply(Dir::Rx, 0, &crate::device::Settings { rate: Some(1e6), freq: Some(1e9), ..Default::default() }, None).unwrap();
        spy.apply(Dir::Rx, 0, &crate::device::Settings { freq: Some(1.001e9), ..Default::default() }, Some(0)).unwrap();
        spy.rx_start(0, now).unwrap();
        let entry = super::super::core::RxEntry { order: (0, 1), channel: 0, value: 1.001e9, accepted_at: Some(now) };
        {
            let mut chain = core.rx_chain[0].lock();
            accept_entry(&mut chain, entry);
        }
        rx.stream.as_mut().unwrap().expected = expected;
        core.rx_chain[0].lock().request_cut(cut);
        let release = core.ticks(core.description.timing.release_window_ns);
        let allowance = core.ticks(DELIVERY_ALLOWANCE_NS);
        let lead = core.ticks(core.description.timing.rx_start_lead_ns);
        let successor = cut + release + call * clock.n + allowance + lead;
        plan(&core, &[(0, Some(cut_index), None), (successor, None, Some(1))]);
        let deadline = rx_drain_deadline(&core, 0, clock.n, cut, core.rx_chain[0].lock().debt()).unwrap();
        assert_eq!(deadline - i128::from(now), i128::from(core.ticks(20_000_000)), "the ordinary receive starts with one 20 ms packet-maximum window remaining");
        drop(frozen);

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
        gate.wait();
        let returned_at = spy.time_now().unwrap();
        assert_eq!(returned_at, now + core.ticks(7_000_000), "the one-packet receive returned after 500 samples and delivery latency");
        time.advance_to(core.at(returned_at)).unwrap();
        spy.set_time(returned_at);
        gate.release();
        let rx = finished.recv_timeout(Wall::from_secs(10)).expect("the owner completed its cut receive");
        worker.join().unwrap();

        assert!(rx.stream.is_none(), "the late packet is clipped at the cut and the owner stops");
        assert_eq!(core.rx_chain[0].lock().debt().len(), 0, "the first short packet consumed the timed DDC entry");
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
        assert!(core.rx_chain[0].lock().debt().is_empty());
        assert!(core.rx_chain[0].lock().left().is_empty());
        assert!(!device.calls().iter().any(|call| call.starts_with("rx_start_now") || call.starts_with("rx_recv")), "shutdown has no streamer to recover");
    }

    #[test]
    fn ur_24_nonloss_rx_stop_failure_reports_pending_and_skips_restore() {
        let spy = Arc::new(super::super::test_support::LockSpy::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, ..Default::default() }));
        let (core, time, _) = super::super::test_support::rig_on_description(spy.clone(), vec![link()], BTreeMap::new(), super::super::test_support::default_description(2_000));
        spy.watch(&core);
        let frozen = super::super::test_support::Frozen::on(spy.host());
        spy.set_time(core.now());
        let (mut rx, clock) = rx(&core, 1);
        let stop_at = core.now();
        let entry = super::super::core::RxEntry { order: (stop_at + 1, 1), channel: 0, value: 1.001e9, accepted_at: Some(stop_at + 1) };
        {
            let mut streams = lock(&core.streams);
            streams.stop = Some((stop_at, StopMode::Orderly));
        }
        core.rx_chain[0].lock().request_cut(stop_at);
        {
            let mut chain = core.rx_chain[0].lock();
            accept_entry(&mut chain, entry.clone());
            chain.plan_restore(BTreeMap::from([(0, 1e9)]));
        }
        let call = core.block_len.max(core.device.rx_packet_samples(0)) as i64;
        let flush_at = (entry.order.0 + call * clock.n).max(stop_at + core.block_len as i64 * clock.n);
        time.advance_to(core.at(flush_at)).unwrap();
        spy.set_time(flush_at);
        spy.refuse_rx_stop(crate::DeviceError::failed("test: scripted rx_stop refusal"));

        rx.try_end();
        drop(frozen);

        assert!(rx.stream.is_none());
        assert_eq!(spy.seen("rx_stop"), 1);
        assert_eq!(spy.seen("rx_start_now"), 0, "a failed stop does not start a recovery flush");
        assert_eq!(spy.seen("rx_recv"), 0);
        let chain = core.rx_chain[0].lock();
        assert_eq!(chain.device(), RxDevice::Uncertain, "a refused stop leaves the device fact uncertain");
        assert!(chain.debt().is_empty());
        assert_eq!(chain.left().len(), 1);
        assert_eq!((chain.left()[0].order, chain.left()[0].channel, chain.left()[0].value), (entry.order, entry.channel, entry.value));
        drop(chain);
        let rec = lock(&core.rec);
        assert!(
            rec.rejected.iter().any(|row| row["call"] == "rx_drain" && row["channel"] == 0 && row["error"].as_str().is_some_and(|error| error.contains("unconsumed DDC value"))),
            "{:?}",
            rec.rejected
        );
        assert!(rec.rejected.iter().any(|row| row["call"] == "rx_restore" && row["error"].as_str().is_some_and(|error| error.contains("restore was not attempted"))), "{:?}", rec.rejected);
        let skipped: Vec<_> = rec.applied.iter().filter(|row| row["restore"] == true).collect();
        assert_eq!(skipped.len(), 1, "{skipped:?}");
        assert!(skipped[0]["error"].as_str().is_some_and(|error| error.contains("restore was not attempted")), "{skipped:?}");
    }

    #[test]
    fn ur_24_provider_stop_skips_restore_when_flush_stop_fails() {
        let spy = Arc::new(super::super::test_support::LockSpy::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, rx_packet: Some(1_000), ..Default::default() }));
        let (core, time, _) = super::super::test_support::rig_on_description(spy.clone(), vec![link()], BTreeMap::new(), super::super::test_support::default_description(2_000));
        spy.watch(&core);
        let frozen = super::super::test_support::Frozen::on(spy.host());
        spy.set_time(core.now());
        let (mut rx, clock) = rx(&core, 1);
        core.device.apply(Dir::Rx, 0, &crate::device::Settings { rate: Some(1e6), freq: Some(1e9), ..Default::default() }, None).unwrap();
        core.device.rx_start(0, 0).unwrap();
        let stop_at = core.now();
        let entry = super::super::core::RxEntry { order: (stop_at + core.ticks(3_000_000), 1), channel: 0, value: 1.001e9, accepted_at: Some(stop_at) };
        core.device.apply(Dir::Rx, 0, &crate::device::Settings { freq: Some(entry.value), ..Default::default() }, Some(entry.order.0)).unwrap();
        {
            let mut streams = lock(&core.streams);
            streams.stop = Some((stop_at, StopMode::Orderly));
        }
        core.rx_chain[0].lock().request_cut(stop_at);
        {
            let mut chain = core.rx_chain[0].lock();
            accept_entry(&mut chain, entry.clone());
            chain.plan_restore(BTreeMap::from([(0, 1e9)]));
        }
        let call = core.block_len.max(core.device.rx_packet_samples(0)) as i64;
        let flush_at = entry.order.0 + call * clock.n;
        time.advance_to(core.at(flush_at)).unwrap();
        spy.set_time(flush_at);
        let applied = spy.seen("apply");
        let gate = spy.stop_after("rx_stop");
        let (done, finished) = std::sync::mpsc::channel();
        let host = core.device.host();
        let worker = host
            .spawn("rx-d9b-final-stop", move || {
                rx.try_end();
                let _ = done.send(rx);
            })
            .unwrap();
        gate.wait();
        assert_eq!(spy.seen("rx_stop"), 1, "the gated stop is the successful cut stop");
        spy.refuse_rx_stop(crate::DeviceError::failed("test: scripted final rx_stop refusal"));
        drop(frozen);
        gate.release();
        let rx = finished.recv_timeout(Wall::from_secs(10)).expect("the bounded drain finished");
        worker.join().unwrap();

        assert!(rx.stream.is_none());
        assert_eq!(spy.seen("rx_stop"), 2, "the refusal belongs to the flush's final stop");
        assert_eq!(spy.seen("rx_start_now"), 1);
        assert!(spy.seen("rx_recv") > 0, "the flush reads the post-S entry before its final stop fails");
        let order = spy.order();
        let last_recv = order.iter().rposition(|name| *name == "rx_recv").unwrap();
        let final_stop = order.iter().rposition(|name| *name == "rx_stop").unwrap();
        assert!(last_recv < final_stop, "the final stop follows the drain reads: {order:?}");
        assert_eq!(spy.seen("apply"), applied, "no restore apply follows the failed final stop");
        assert_eq!(core.rx_chain[0].lock().device(), RxDevice::Uncertain, "a refused final stop leaves the device fact uncertain");
        assert!(core.rx_chain[0].lock().debt().is_empty(), "the flush consumed the DDC entry before the stop refusal");
        let rec = lock(&core.rec);
        let skipped: Vec<_> = rec.applied.iter().filter(|row| row["restore"] == true).collect();
        assert_eq!(skipped.len(), 1, "the skipped restore is recorded: {skipped:?}");
        assert!(skipped[0]["error"].as_str().is_some_and(|error| error.contains("receive stop failed; restore was not attempted")), "{skipped:?}");
        assert!(
            rec.rejected.iter().any(|row| row["call"] == "rx_restore" && row["error"].as_str().is_some_and(|error| error.contains("receive stop failed; restore was not attempted"))),
            "{:?}",
            rec.rejected
        );
    }

    #[test]
    fn ur_24_a_loss_during_rx_flush_finishes_without_recovery_calls() {
        let spy = Arc::new(super::super::test_support::LockSpy::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, rx_packet: Some(2_000), ..Default::default() }));
        let (core, time, events) = super::super::test_support::rig_on_description(spy.clone(), vec![link()], BTreeMap::new(), super::super::test_support::default_description(2_000));
        spy.watch(&core);
        let frozen = super::super::test_support::Frozen::on(spy.host());
        spy.set_time(core.now());
        let (mut rx, clock) = rx(&core, 1);
        spy.rx_start(0, 0).unwrap();
        let entry = super::super::core::RxEntry { order: (core.ticks(1_000_000), 1), channel: 0, value: 1.001e9, accepted_at: Some(0) };
        spy.apply(Dir::Rx, 0, &crate::device::Settings { freq: Some(entry.value), ..Default::default() }, Some(entry.order.0)).unwrap();
        core.rx_chain[0].lock().request_cut(0);
        accept_entry(&mut core.rx_chain[0].lock(), entry);
        let flush_at = core.ticks(1_000_000) + core.block_len as i64 * clock.n;
        time.advance_to(core.at(flush_at)).unwrap();
        spy.set_time(flush_at);
        let gate = spy.stop_after("rx_start_now");
        let (done, finished) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            rx.try_end();
            let _ = done.send(rx);
        });
        gate.wait();
        assert_eq!(spy.seen("rx_stop"), 1, "the cut stop precedes the recovery start");
        assert_eq!(spy.seen("rx_recv"), 0);
        let after_loss_at = flush_at + core.ticks(1_000_000);
        time.advance_to(core.at(after_loss_at)).unwrap();
        spy.set_time(after_loss_at);
        core.device_lost("test: loss during RX flush");
        drop(frozen);
        gate.release();
        let rx = finished.recv_timeout(Wall::from_secs(10)).expect("the lost recovery ended promptly");
        worker.join().unwrap();

        assert!(rx.stream.is_none());
        assert_eq!(spy.seen("rx_start_now"), 1);
        assert_eq!(spy.seen("rx_recv"), 0, "recovery did not read after loss");
        assert_eq!(spy.seen("rx_stop"), 1, "recovery did not issue a final stop after loss");
        assert!(core.rx_chain[0].lock().debt().is_empty());
        assert_eq!(core.rx_chain[0].lock().left().len(), 1);
        assert!(lock(&core.rec).rejected.iter().any(|row| row["call"] == "rx_drain" && row["channel"] == 0));
        assert_eq!(events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).iter().filter(|event| event.kind.as_str() == ezsdr_kernel::event::EventKind::DEVICE_LOST).count(), 1);
    }

    #[test]
    fn ur_24_a_loss_during_rx_receive_finishes_without_recovery_calls() {
        let spy = Arc::new(super::super::test_support::LockSpy::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, ..Default::default() }));
        let (core, _time, events) = super::super::test_support::rig_on_description(spy.clone(), vec![link()], BTreeMap::new(), super::super::test_support::default_description(2_000));
        spy.watch(&core);
        let _frozen = super::super::test_support::Frozen::on(spy.host());
        spy.set_time(core.now());
        let (mut rx, _) = rx(&core, 1);
        let entry = RxEntry { order: (core.ticks(1_000_000), 1), channel: 0, value: 1.001e9, accepted_at: Some(0) };
        spy.apply(Dir::Rx, 0, &crate::device::Settings { freq: Some(entry.value), ..Default::default() }, Some(entry.order.0)).unwrap();
        accept_entry(&mut core.rx_chain[0].lock(), entry);
        assert_eq!(core.rx_chain[0].lock().debt().len(), 1);

        rx.receive(RxRecv::Failed(crate::DeviceError { lost: true, message: "scripted loss during RX receive".to_owned() }));
        let calls_at_loss = spy.log().len();
        rx.try_end();

        assert_eq!(spy.log().len(), calls_at_loss, "finish makes no recovery call after loss: {:?}", spy.log());
        assert!(rx.stream.is_none());
        let chain = core.rx_chain[0].lock();
        assert_eq!(chain.device(), RxDevice::Lost);
        assert!(chain.debt().is_empty());
        assert_eq!(chain.left().len(), 1);
        drop(chain);
        assert_eq!(lock(&core.rec).rejected.iter().filter(|row| row["call"] == "rx_drain" && row["channel"] == 0).count(), 1);
        assert_eq!(events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).iter().filter(|event| event.kind.as_str() == ezsdr_kernel::event::EventKind::DEVICE_LOST).count(), 1);
    }

    #[test]
    fn ur_24_a_loss_published_before_rx_recv_skips_the_device_call() {
        let spy = Arc::new(super::super::test_support::LockSpy::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..Default::default() }));
        let (core, time, events) = super::super::test_support::rig_on_description(spy.clone(), vec![link()], BTreeMap::new(), super::super::test_support::default_description(2_000));
        spy.watch(&core);
        let _frozen = super::super::test_support::Frozen::on(spy.host());

        let mut config = core.description.defaults();
        config.insert(super::super::core::key("radio.rx.c0.stream"), ezsdr_kernel::spec::Value::from(0));
        let (to_tx, _) = std::sync::mpsc::channel();
        let mut control = super::super::control::Control::new(
            core.clone(),
            Arc::new(NoActions),
            vec![to_tx],
            config,
            false,
            super::super::control::Start { t0: 0, rx: vec![Some((Channels::first(1), 200)); core.description.stream_count(Dir::Rx)], tx: vec![None; core.description.stream_count(Dir::Tx)] },
        );
        let segment = Segment { origin: 0, cut: None, config: Config { channels: Channels::first(1), ratio: Rational::new(200, 1).unwrap() }, by: None };
        let (mut rx, _) = rx_with_plan(&core, &[segment]);

        let window = core.ticks(core.description.timing.release_window_ns);
        let gap = window + core.ticks(10_000_000);
        let entries = [core.ticks(100_000_000), core.ticks(100_000_000) + gap, core.ticks(100_000_000) + gap * 2];
        let values = [1.001e9, 1.002e9, 1.003e9];
        let frequency = super::super::core::key("radio.rx.c0.frequency_hz");
        for (index, (&at, &value)) in entries.iter().zip(&values).enumerate() {
            control.book_timed(
                ezsdr_kernel::event::ActionId(index as u64 + 1),
                frequency.clone(),
                ezsdr_kernel::spec::Value::num(value).unwrap(),
                Some(ezsdr_kernel::time::AbsoluteDeadline::new(core.at(at))),
            );
        }
        for (index, &at) in entries[..2].iter().enumerate() {
            device_at_spy(&core, &time, &spy, at - window);
            control.release();
            assert_eq!(core.rx_chain[0].lock().debt().len(), index + 1);
        }

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

        device_at_spy(&core, &time, &spy, entries[2] - window);
        spy.refuse_rx_apply(crate::DeviceError { lost: true, message: "scripted loss in Control timed apply".to_owned() });
        control.release();
        assert!(core.is_lost(), "Control's failed timed apply published loss");
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
        assert!(chain.debt().is_empty());
        assert_eq!(chain.left().iter().map(|entry| entry.order.0).collect::<Vec<_>>(), entries[..2]);
        assert_eq!(chain.segment(), None);
        drop(chain);
        let reported: Vec<_> = lock(&core.rec).rejected.iter().filter(|row| row["call"] == "rx_drain").map(|row| row["at"].as_i64().unwrap()).collect();
        assert_eq!(reported, entries[..2], "each outstanding DDC entry is reported once");
        assert_eq!(events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).iter().filter(|event| event.kind.as_str() == ezsdr_kernel::event::EventKind::DEVICE_LOST).count(), 1);
    }

    #[test]
    fn ur_24_provider_stop_records_restore_failure_and_stops_after_loss() {
        for lost in [false, true] {
            let inner = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, ..Default::default() }));
            let wrapper: Arc<dyn Device> = if lost {
                Arc::new(super::super::test_support::ApplyFails::lost(inner.clone(), Dir::Rx, Some(0)))
            } else {
                Arc::new(super::super::test_support::ApplyFails::new(inner.clone(), Dir::Rx, Some(0)))
            };
            let output = link();
            let (core, _time, events) = super::super::test_support::rig_on_description(wrapper.clone(), vec![output], BTreeMap::new(), super::super::test_support::default_description(2_000));
            inner.set_time(0);
            let (mut rx, _) = rx(&core, 1);
            inner.rx_start(0, 0).unwrap();
            {
                let mut streams = lock(&core.streams);
                streams.stop = Some((0, StopMode::Orderly));
            }
            core.rx_chain[0].lock().request_cut(0);
            {
                let mut chain = core.rx_chain[0].lock();
                for (channel, value) in [(0, 1.001e9), (1, 1.003e9)] {
                    accept_entry(&mut chain, RxEntry { order: (1, channel as u64 + 1), channel, value, accepted_at: Some(1) });
                }
                chain.consume(200, Channels::first(2), 2, &[vec![[0.0, 0.0]], vec![[0.0, 0.0]]]);
                chain.plan_restore(BTreeMap::from([(0, 1.001e9), (1, 1.003e9)]));
            }

            rx.try_end();

            assert!(rx.stream.is_none());
            let rec = lock(&core.rec);
            let restore: Vec<_> = rec.applied.iter().filter(|row| row["restore"] == true).collect();
            assert_eq!(restore.len(), if lost { 1 } else { 2 }, "{restore:?}");
            assert_eq!(restore[0]["error"].as_str(), Some(if lost { "test: scripted loss on an apply" } else { "test: scripted refusal of an apply" }));
            assert!(rec.rejected.iter().any(|row| row["call"] == "rx_restore"));
            drop(rec);
            if lost {
                assert!(core.is_lost());
                assert!(!inner.calls().iter().any(|call| call == "apply rx 1"), "no later channel restore is attempted after loss");
                assert_eq!(events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).iter().filter(|event| event.kind.as_str() == ezsdr_kernel::event::EventKind::DEVICE_LOST).count(), 1);
            } else {
                assert!(!core.is_lost());
                assert!(inner.calls().iter().any(|call| call.starts_with("apply rx 1 ")), "a non-lost channel refusal does not skip the next restore");
            }
        }
    }

    #[test]
    fn ur_24_a_parallel_rx_restore_checks_do_not_nest_chain_locks() {
        use std::sync::Barrier;

        let device = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, ..Default::default() }));
        let (core, _, _) = super::super::test_support::rig_on_description_links(device.clone(), vec![vec![link()], vec![link()]], BTreeMap::new(), layout_description());
        let at = core.now();
        lock(&core.streams).stop = Some((at, StopMode::Orderly));
        core.rx_chain[0].lock().plan_restore(BTreeMap::from([(0, 1.001e9)]));
        core.rx_chain[1].lock().plan_restore(BTreeMap::from([(1, 1.002e9)]));

        let gate = Arc::new(Barrier::new(3));
        super::super::test_support::probe(&core, "rx_restore_before_chain_scan", {
            let gate = gate.clone();
            move |_| {
                gate.wait();
            }
        });
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let mut rx0 = rx_owner_on(&core, 0);
        let done = done_tx.clone();
        let owner0 = core
            .device
            .host()
            .spawn("rx-restore-0", move || {
                rx0.finish();
                done.send(0).unwrap();
            })
            .unwrap();
        let mut rx1 = rx_owner_on(&core, 1);
        let owner1 = core
            .device
            .host()
            .spawn("rx-restore-1", move || {
                rx1.finish();
                done_tx.send(1).unwrap();
            })
            .unwrap();

        gate.wait();
        let mut done = [done_rx.recv_timeout(Wall::from_secs(10)).unwrap(), done_rx.recv_timeout(Wall::from_secs(10)).unwrap()];
        done.sort_unstable();
        assert_eq!(done, [0, 1]);
        owner0.join().unwrap();
        owner1.join().unwrap();

        let restores: Vec<_> = lock(&core.rec).applied.iter().filter(|row| row["restore"] == true).cloned().collect();
        assert_eq!(restores.len(), 2, "both simultaneous owners restored their own channel: {restores:?}");
        assert!(device.calls().iter().any(|call| call.starts_with("apply rx 0 ")));
        assert!(device.calls().iter().any(|call| call.starts_with("apply rx 1 ")));
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

    #[test]
    fn ur_34_a_waited_receive_open_exits_when_stop_removes_its_segment() {
        let (core, device, _time, events) = super::super::test_support::rig_with_links(vec![link()]);
        let from = stream_key(Dir::Rx, 1);
        let to = stream_key(Dir::Rx, 0);
        let ratio = Rational::new(200, 1).unwrap();
        let line = |channels| Line::new(ezsdr_radio::timeline::Stream { direction: Direction::Rx, origin: 0, config: Config { channels, ratio }, lead: 0, call: 0, release_window: 0, allowance: 0 });
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
        super::super::test_support::probe(&core, "wait_release", move |_| {
            let _ = entered.send(());
        });
        let (done, finished) = std::sync::mpsc::channel();
        let owner = core
            .device
            .host()
            .spawn("rx-wait-stop", move || {
                let began = rx.begin();
                let _ = done.send((began, rx.cursor()));
            })
            .unwrap();
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
        assert!(
            !events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().any(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED),
            "Stop interruption emits no refusal"
        );
    }

    fn rate(rate: u64) -> Kind {
        Kind::Cold(Config { channels: Channels::first(1), ratio: Rational::new(200_000_000 / rate, 1).unwrap() })
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

    /// A layout of three receive channels in three streams — 0:A:ubx:0, 0:B:basic_rx:0,
    /// 0:B:basic_rx:1 — and one transmit channel (A2's chain).
    fn mixed_description() -> ezsdr_radio::channels::Description {
        use ezsdr_kernel::spec::{Ident, Scalar, Value};
        let selector = std::collections::BTreeMap::from([
            (Ident::parse("rx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:B:basic_rx:0"), Scalar::from("0:B:basic_rx:1")])),
            (Ident::parse("tx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0")])),
        ]);
        let layout = ezsdr_radio::channels::from_selector(&selector).unwrap();
        ezsdr_radio::channels::Description::new(&layout, 2_000, &crate::profile::terms())
    }

    #[test]
    fn ur_34_a_finite_middle_segment_releases_a_channel_along_a_move_chain() {
        // A2 (design note 37): B's segment has a finite cut at e₂, and C takes the channel
        // only from that cut. The owners carry it out: A releases its streamer at e₁, B
        // receives its own segment through e₂ and its owner's release lets C open, and C
        // starts after B's close. On the controlled clock the two instants are fixed.
        let device = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig {
            clock: crate::ClockMode::Manual,
            command_queue: 64,
            front_ends: vec!["UBX", "BasicRX"],
            ..crate::device::FakeConfig::default()
        }));
        let link1 = link();
        let links = vec![vec![link()], vec![link1.clone()], vec![link()]];
        let (core, time, events) = super::super::test_support::rig_on_description_links(device.clone(), links, BTreeMap::new(), mixed_description());
        // The device's clock is held at 0 until the owners start, so each destination's timed
        // start is far in the device's future whatever the test thread does before it.
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        super::super::test_support::device_at(&core, &time, &device, 0);
        let key = |name: &str| super::super::core::key(name);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(ezsdr_kernel::time::AbsoluteDeadline::new(core.at(ms(n))));
        let c1 = Channels::first(2).without(0);
        let mut values = core.description.defaults();
        values.insert(key("radio.rx.c0.stream"), ezsdr_kernel::spec::Value::from(-1));
        values.insert(key("radio.rx.c1.stream"), ezsdr_kernel::spec::Value::from(0));
        values.insert(key("radio.rx.c2.stream"), ezsdr_kernel::spec::Value::from(-1));
        let (to_tx, _tx) = std::sync::mpsc::channel();
        let mut control = super::super::control::Control::new(
            core.clone(),
            Arc::new(NoActions),
            vec![to_tx],
            values,
            false,
            super::super::control::Start { t0: 0, rx: vec![Some((c1, 100)), None, None], tx: vec![None] },
        );
        lock(&core.streams).search(stream_key(Dir::Rx, 0), true);
        device.rx_open(0, &[1]).unwrap();
        lock(&core.streams).record_open(stream_key(Dir::Rx, 0), Some(c1));
        let handle = core.clocks.declare_sample_clock(core.stream_node(Dir::Rx, 0), core.root, Rational::new(100, 1).unwrap()).unwrap();
        let target = super::super::test_support::target_of(&core.id);
        for (id, value, instant) in [(1, 1, 100), (2, 2, 250)] {
            control.book(ezsdr_kernel::module_api::Dispatched::new(
                ezsdr_kernel::event::ActionId(id),
                ezsdr_kernel::event::Action::UpdateParameter {
                    target: target.clone(),
                    key: key("radio.rx.c1.stream"),
                    value: ezsdr_kernel::spec::Value::from(value),
                    class: ezsdr_kernel::module_api::UpdateClass::Cold,
                    at: at(instant),
                },
                Some(core.id.clone()),
            ));
        }
        let middle = lock(&core.streams).planned(stream_key(Dir::Rx, 1)).into_iter().find(|planned| planned.segment.cut.is_some()).expect("B's finite segment");
        let b_origin = middle.segment.origin;
        let b_cut = middle.segment.cut.expect("B's segment is finite");
        assert_eq!(middle.segment.instant(b_cut).unwrap(), ms(250), "B's cut is not e₂");
        assert!(b_cut > 0 && middle.segment.config.channels == c1, "B's segment holds no samples: {:?}", middle.segment);
        let c_origin = lock(&core.streams).planned(stream_key(Dir::Rx, 2)).into_iter().find(|planned| planned.segment.config.channels == c1).expect("C's segment").segment.origin;
        assert!(c_origin > ms(250), "C begins before B's cut: {c_origin}");
        // A: its old segment through e₁, then its own release.
        let (a_done, a_finished) = std::sync::mpsc::channel();
        let worker = core.clone();
        let a = core
            .device
            .host()
            .spawn("rx-chain-A", move || {
                let (_to_rx, cmds) = std::sync::mpsc::channel();
                std::mem::forget(_to_rx);
                let mut rx = Rx::new(worker, 0, cmds, Some((handle, c1, 0)), None);
                let (mut tick, mut delivered) = (0i64, 0i64);
                while delivered < 200_000 {
                    let n = (200_000 - delivered).min(2_000);
                    rx.receive(samples(tick, n as usize));
                    tick += n * 100;
                    delivered += n;
                }
                let began = rx.begin();
                let _ = a_done.send(began);
            })
            .expect("a participant thread");
        // B: opens behind A's release, receives its finite segment through its cut, releases.
        let (b_done, b_finished) = std::sync::mpsc::channel();
        let worker = core.clone();
        let b = core
            .device
            .host()
            .spawn("rx-chain-B", move || {
                let (_to_rx, cmds) = std::sync::mpsc::channel();
                std::mem::forget(_to_rx);
                let mut rx = Rx::new(worker.clone(), 1, cmds, None, None);
                let opened = rx.begin();
                // The clock stood still from the start's issue to this read: the worker is a busy
                // participant (M1, M3), so the instant below is the one the start was issued at.
                let started_at = worker.device.time_now().expect("the device's time");
                let origin = rx.stream.as_ref().map(|stream| stream.clock.origin).unwrap_or(b_origin);
                let (mut tick, mut delivered) = (origin, 0i64);
                while delivered < b_cut {
                    let n = (b_cut - delivered).min(2_000);
                    rx.receive(samples(tick, n as usize));
                    tick += n * 200;
                    delivered += n;
                }
                let released = rx.begin();
                let _ = b_done.send((opened, released, started_at));
            })
            .expect("a participant thread");
        // C: opens behind B's release. It is the last owner, so it stays a participant past its
        // result until the test holds the clock again (M1): A and B cannot stay, since the
        // release wait of the owner behind each sleeps on the very clock they would freeze.
        let (c_done, c_finished) = std::sync::mpsc::channel();
        let (release_c, c_release) = std::sync::mpsc::channel();
        let worker = core.clone();
        let c = core
            .device
            .host()
            .spawn("rx-chain-C", move || {
                let (_to_rx, cmds) = std::sync::mpsc::channel();
                std::mem::forget(_to_rx);
                let mut rx = Rx::new(worker.clone(), 2, cmds, None, None);
                let began = rx.begin();
                let started_at = worker.device.time_now().expect("the device's time");
                let _ = c_done.send((began, rx.stream.as_ref().map(|stream| stream.clock.origin), started_at));
                let _ = c_release.recv();
            })
            .expect("a participant thread");
        // The waits poll the device's clock, so it must run from here on — with every worker a
        // participant already, so no clock time passes before one of them starts (M1).
        drop(_frozen);
        // Read the owners' results before joining any: a wait that never ends fails here
        // instead of hanging the suite on the join.
        assert!(!a_finished.recv_timeout(Wall::from_secs(10)).expect("A did not finish its turn"), "A has no next segment");
        let (b_opened, b_released, b_started_at) = b_finished.recv_timeout(Wall::from_secs(10)).expect("B did not finish its turn");
        assert!(b_opened, "B did not open after A's release: {:?}", device.calls());
        assert!(!b_released, "B has no next segment");
        assert!(b_started_at <= b_origin, "B's start was issued {b_started_at} at, after its origin {b_origin}");
        let (c_began, c_clock, c_started_at) = c_finished.recv_timeout(Wall::from_secs(10)).expect("C did not open after B's release");
        assert!(c_began, "C did not open after B's release: {:?}", device.calls());
        assert!(c_clock.is_some_and(|origin| origin == c_origin), "C's own clock: {c_clock:?} for {c_origin}");
        assert!(c_started_at <= c_origin, "C's start was issued {c_started_at} at, after its origin {c_origin}");
        // C, the last owner, holds the clock until the test holds it again (M1): one participant
        // is busy from the first hold's release to this one, so a delayed test thread leaves the
        // device at the instant the turns left it.
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        let _ = release_c.send(());
        for owner in [a, b, c] {
            owner.join().expect("a participant thread");
        }
        let calls = device.calls();
        let position = |call: &str| calls.iter().position(|entry| entry == call).unwrap_or_else(|| panic!("{call}: {calls:?}"));
        assert!(position("rx_close 0") < position("rx_open 1 [1]"), "A releases before B opens: {calls:?}");
        assert!(position("rx_open 1 [1]") < position("rx_stop 1 now"), "B opens before its cut: {calls:?}");
        assert!(position("rx_stop 1 now") < position("rx_close 1"), "B's cut precedes its release: {calls:?}");
        assert!(position("rx_close 1") < position("rx_open 2 [1]"), "C opens after B releases: {calls:?}");
        // C's start was not late: its streamer delivers its samples where a missed start would
        // come back as `LateCommand` (UR-17).
        super::super::test_support::device_at(&core, &time, &device, c_origin + 1_000 * 200);
        let got = device.rx_recv(2, 1_000, Wall::from_millis(100), false);
        assert!(matches!(got, RxRecv::Samples { first_tick: first, .. } if first == c_origin), "C's start was late: {got:?} {calls:?}");
        // B delivered its own segment up to its cut: the blocks the link still holds are
        // contiguous and the last ends there. (The link keeps its last 16, so the sum is not
        // the whole segment.)
        let blocks = received(&link1);
        assert!(!blocks.is_empty(), "B delivered nothing: {calls:?}");
        assert!(blocks.windows(2).all(|pair| pair[0].1 + i64::from(pair[0].2) == pair[1].1), "B's blocks are not contiguous: {blocks:?}");
        let last = *blocks.last().expect("a block");
        assert_eq!(last.1 + i64::from(last.2), b_cut, "B did not deliver through its cut {b_cut}: {blocks:?}");
        assert!(events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).into_iter().all(|event| event.kind.as_str() != kinds::COMMAND_REJECTED), "{calls:?}");
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
                release_window: 0,
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
        let lead = core.ticks(ezsdr_radio::timeline::start_lead_ns(&core.description.timing, Direction::Rx));
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
        assert_eq!(streams.phase(stream_key(Dir::Rx, 0)), Phase::Pending, "a lost configure leaves the owner pending");
        assert!(streams.active(stream_key(Dir::Rx, 0)).is_none());
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
        assert!(chain.debt().is_empty());
        assert!(core.clocks.sample_clock_records().is_empty());
        assert!(lock(&core.streams).active(stream_key(Dir::Rx, 0)).is_none());
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
        lock(&core.streams).lines.insert(
            stream_key(Dir::Rx, 0),
            Line::new(ezsdr_radio::timeline::Stream {
                direction: Direction::Rx,
                origin: 0,
                config: Config { channels: Channels::first(0), ratio: Rational::new(200, 1).unwrap() },
                lead: 0,
                call: 2_000,
                release_window: 0,
                allowance: 0,
            }),
        );
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
        let (core, device, _time, _events) = super::super::test_support::rig_with_links_on(1, vec![link()], layout_description());
        let config = Config { channels: Channels::first(1), ratio: Rational::new(200, 1).unwrap() };
        {
            let mut streams = lock(&core.streams);
            streams.started(stream_key(Dir::Rx, 0), Some((0, config, None)), 0, config);
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
                release_window: 0,
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
            assert_eq!(streams.phase(stream_key(Dir::Rx, 1)), Phase::Active { segment: Some((0, config, None)), delivered: Some((0, config, 0)) }, "the T0 stream begins at its planned segment");
            assert_eq!(streams.first_undelivered(stream_key(Dir::Rx, 0)), Some((0, config, 700)), "stream 0's floor stands");
        }
        core.rx_chain[1].lock().stop_result(device.rx_stop(1, None)).unwrap();
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
        let (core, _device, time, _events) = super::super::test_support::rig_with_links(vec![link()]);
        let key = stream_key(Dir::Rx, 0);
        let config = Config { channels: Channels::first(1), ratio: Rational::new(200, 1).unwrap() };
        let t0 = core.ticks(10_000_000);
        let loss = core.ticks(5_000_000);
        let node = core.stream_node(Dir::Rx, 0);
        let handle = core.clocks.declare_sample_clock(node, core.root, config.ratio).unwrap();
        {
            let mut streams = lock(&core.streams);
            streams.lines.insert(key, Line::new(ezsdr_radio::timeline::Stream { direction: Direction::Rx, origin: t0, config, lead: 0, call: 2_000, release_window: 0, allowance: 0 }));
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
        let config = Config { channels: Channels::first(2).without(0), ratio: Rational::new(200, 1).unwrap() };
        {
            let mut streams = lock(&core.streams);
            let mut line = Line::new(ezsdr_radio::timeline::Stream { direction: Direction::Rx, origin: 0, config, lead: 0, call: 0, release_window: 0, allowance: 0 });
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
    fn ur_34_two_moves_at_one_instant_swap_their_receive_channels_without_deadlock() {
        // A2 (design note 37): two receive moves at one instant swap the channels, and no
        // zero-duration segment holding both is planned. The owners carry the plan out
        // themselves: each receives its old segment through its cut, finishes it — the
        // completion the other's release wait reads — and its own begin of the destination
        // closes its old streamer and waits for the other's. On the controlled clock both
        // moves are booked at `e`, and neither wait can be what lets them through.
        let device = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..crate::device::FakeConfig::default() }));
        let links = vec![vec![link()], vec![link()]];
        let (core, time, events) = super::super::test_support::rig_on_description_links(device.clone(), links, BTreeMap::new(), layout_description());
        // The device's clock is held at 0 until the owners start: their destination's timed
        // start is then far in the device's future, whatever the test thread does before them.
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        super::super::test_support::device_at(&core, &time, &device, 0);
        let key = |name: &str| super::super::core::key(name);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(ezsdr_kernel::time::AbsoluteDeadline::new(core.at(ms(n))));
        let mut values = core.description.defaults();
        values.insert(key("radio.rx.c0.stream"), ezsdr_kernel::spec::Value::from(0));
        values.insert(key("radio.rx.c1.stream"), ezsdr_kernel::spec::Value::from(1));
        let (to_tx, _tx) = std::sync::mpsc::channel();
        let mut control = super::super::control::Control::new(
            core.clone(),
            Arc::new(NoActions),
            vec![to_tx],
            values,
            false,
            super::super::control::Start { t0: 0, rx: vec![Some((Channels::first(1), 200)), Some((Channels::first(2).without(0), 200))], tx: vec![None] },
        );
        lock(&core.streams).search(stream_key(Dir::Rx, 0), true);
        lock(&core.streams).search(stream_key(Dir::Rx, 1), true);
        let (c0, c1) = (Channels::first(1), Channels::first(2).without(0));
        device.rx_open(0, &[0]).unwrap();
        device.rx_open(1, &[1]).unwrap();
        {
            let mut streams = lock(&core.streams);
            streams.record_open(stream_key(Dir::Rx, 0), Some(c0));
            streams.record_open(stream_key(Dir::Rx, 1), Some(c1));
        }
        let h0 = core.clocks.declare_sample_clock(core.stream_node(Dir::Rx, 0), core.root, Rational::new(200, 1).unwrap()).unwrap();
        let h1 = core.clocks.declare_sample_clock(core.stream_node(Dir::Rx, 1), core.root, Rational::new(200, 1).unwrap()).unwrap();
        let target = super::super::test_support::target_of(&core.id);
        for (id, name, value) in [(1, "radio.rx.c0.stream", 1), (2, "radio.rx.c1.stream", 0)] {
            control.book(ezsdr_kernel::module_api::Dispatched::new(
                ezsdr_kernel::event::ActionId(id),
                ezsdr_kernel::event::Action::UpdateParameter {
                    target: target.clone(),
                    key: key(name),
                    value: ezsdr_kernel::spec::Value::from(value),
                    class: ezsdr_kernel::module_api::UpdateClass::Cold,
                    at: at(100),
                },
                Some(core.id.clone()),
            ));
        }
        let rejected = events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == kinds::COMMAND_REJECTED).collect::<Vec<_>>();
        assert!(rejected.is_empty(), "{rejected:?} {:?}", device.calls());
        let moves: Vec<_> = lock(&core.rec).timing.iter().filter(|row| row["what"] == "move").cloned().collect();
        assert_eq!(moves.len(), 2, "{moves:?}");
        assert!(moves.iter().all(|row| row["e"] == ms(100)), "a move is at another instant: {moves:?}");
        let mut owners = Vec::new();
        for (k, (handle, channels)) in [(0, (h0, c0)), (1, (h1, c1))] {
            let worker = core.clone();
            let (done, finished) = std::sync::mpsc::channel();
            let owner = core
                .device
                .host()
                .spawn("rx-swap", move || {
                    let (_to_rx, cmds) = std::sync::mpsc::channel();
                    std::mem::forget(_to_rx);
                    let mut rx = Rx::new(worker.clone(), k, cmds, Some((handle, channels, 0)), None);
                    // The old segment through its cut: 2 000-sample blocks up to sample 100 000.
                    let (mut tick, mut delivered) = (0i64, 0i64);
                    while delivered < 100_000 {
                        let n = (100_000 - delivered).min(2_000);
                        rx.receive(samples(tick, n as usize));
                        tick += n * 200;
                        delivered += n;
                    }
                    let began = rx.begin();
                    // The clock stood still from the start's issue to this read: the worker is a
                    // busy participant (M1, M3).
                    let started_at = worker.device.time_now().expect("the device's time");
                    let _ = done.send((began, rx.stream.as_ref().map(|stream| (stream.clock.domain, stream.clock.origin)), started_at));
                })
                .expect("a participant thread");
            owners.push((owner, finished));
        }
        // The waits poll the device's clock, so it must run from here on — with both owners
        // participants already, so no clock time passes before their starts (M1).
        drop(_frozen);
        let mut reached = Vec::new();
        for (_, finished) in &owners {
            let (began, clock, started_at) = finished.recv_timeout(Wall::from_secs(10)).expect("an owner did not reach its destination");
            reached.push((began, clock, started_at));
        }
        for (owner, _) in owners {
            owner.join().expect("the owner's thread");
        }
        for (k, (began, clock, started_at)) in reached.iter().enumerate() {
            assert!(*began, "stream {k}'s owner did not begin its destination: {:?}", device.calls());
            let (_, origin) = clock.expect("the destination's own clock");
            assert!(origin > ms(100), "stream {k}'s destination begins before the cut: {origin}");
            assert!(*started_at <= origin, "stream {k}'s start was issued at {started_at}, after its origin {origin}");
        }
        let calls = device.calls();
        // Each destination's streamer, and never one holding both channels: the intermediate
        // segment no owner could run is not a streamer either.
        assert!(!calls.iter().any(|call| call == "rx_open 0 [0, 1]" || call == "rx_open 1 [0, 1]"), "a union streamer was opened: {calls:?}");
        assert_eq!(calls.iter().filter(|call| call.as_str() == "rx_open 0 [1]").count(), 1, "stream 0's destination streamer: {calls:?}");
        assert_eq!(calls.iter().filter(|call| call.as_str() == "rx_open 1 [0]").count(), 1, "stream 1's destination streamer: {calls:?}");
        let both = Channels::first(2);
        for (k, want) in [(0, Channels::first(2).without(0)), (1, Channels::first(2).without(1))] {
            let plan = lock(&core.streams).planned(stream_key(Dir::Rx, k));
            assert_eq!(plan.last().expect("the destination").segment.config.channels, want, "stream {k}: {:?}", plan.iter().map(|p| p.segment.config.channels).collect::<Vec<_>>());
            assert!(plan.iter().all(|planned| planned.segment.config.channels != both), "stream {k} plans the union: {:?}", plan.iter().map(|p| p.segment.config.channels).collect::<Vec<_>>());
        }
        // Each old segment's completion is its own owner's, not the test's.
        let done = lock(&core.streams);
        for k in 0..2 {
            assert!(
                done.stream(stream_key(Dir::Rx, k)).completed.iter().any(|(origin, _, by)| *origin == 0 && by.is_none()),
                "stream {k}'s old segment is not completed by its owner: {:?}",
                done.stream(stream_key(Dir::Rx, k)).completed
            );
        }
    }

    #[test]
    fn ur_25_a_same_size_membership_swap_serves_the_new_segment_from_its_own_channel() {
        // M1 (design note 37): a swap that leaves the stream one channel — `{0}` becomes `{1}`
        // — reopens the streamer for the new *set*, so the new segment's samples are the new
        // channel's. The fake loops transmit channel `c` back on receive channel `c` (UR-33),
        // so a waveform on transmit channel 1 comes back with `im` 0.125 on receive channel 1
        // while channel 0's ramp has 0. On the controlled clock the old owner's segment ends at
        // its cut, and the destination's start is in the device's future when it is issued.
        let device = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..crate::device::FakeConfig::default() }));
        let first = link();
        let links = vec![vec![first.clone()], vec![link()]];
        let (core, time, _events) = super::super::test_support::rig_on_description_links(device.clone(), links, BTreeMap::new(), layout_description());
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        super::super::test_support::device_at(&core, &time, &device, 0);
        let key = |name: &str| super::super::core::key(name);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(ezsdr_kernel::time::AbsoluteDeadline::new(core.at(ms(n))));
        let mut values = core.description.defaults();
        values.insert(key("radio.rx.c0.stream"), ezsdr_kernel::spec::Value::from(0));
        values.insert(key("radio.rx.c1.stream"), ezsdr_kernel::spec::Value::from(-1));
        let (to_tx, _tx) = std::sync::mpsc::channel();
        let mut control = super::super::control::Control::new(
            core.clone(),
            Arc::new(NoActions),
            vec![to_tx],
            values,
            false,
            super::super::control::Start { t0: 0, rx: vec![Some((Channels::first(1), 200)), None], tx: vec![None, None] },
        );
        lock(&core.streams).search(stream_key(Dir::Rx, 0), true);
        let target = super::super::test_support::target_of(&core.id);
        for (id, name, value) in [(1, "radio.rx.c0.stream", -1), (2, "radio.rx.c1.stream", 0)] {
            control.book(ezsdr_kernel::module_api::Dispatched::new(
                ezsdr_kernel::event::ActionId(id),
                ezsdr_kernel::event::Action::UpdateParameter {
                    target: target.clone(),
                    key: key(name),
                    value: ezsdr_kernel::spec::Value::from(value),
                    class: ezsdr_kernel::module_api::UpdateClass::Cold,
                    at: at(100),
                },
                Some(core.id.clone()),
            ));
        }
        let plan = lock(&core.streams).planned(stream_key(Dir::Rx, 0));
        let cut = plan.iter().find(|planned| planned.segment.cut.is_some()).expect("the cut").segment;
        let cut_k = cut.cut.expect("the cut is a sample index");
        assert_eq!(cut.instant(cut_k).unwrap(), ms(100), "{cut:?}");
        let destination = plan.last().expect("the new segment").segment;
        assert_eq!(destination.config.channels, Channels::first(2).without(0), "{destination:?}");
        // The streamer the arm opened, its clock, and the owner: the old segment through it.
        let c0 = Channels::first(1);
        device.rx_open(0, &[0]).unwrap();
        lock(&core.streams).record_open(stream_key(Dir::Rx, 0), Some(c0));
        let handle = core.clocks.declare_sample_clock(core.stream_node(Dir::Rx, 0), core.root, Rational::new(200, 1).unwrap()).unwrap();
        let (to_rx, cmds) = std::sync::mpsc::channel();
        std::mem::forget(to_rx);
        let mut rx = Rx::new(core.clone(), 0, cmds, Some((handle, c0, 0)), None);
        rx.receive(samples(0, cut_k as usize));
        assert!(rx.stream.is_none(), "the old segment does not end at its cut: {:?}", device.calls());
        let old: u32 = std::iter::from_fn(|| first.receive().map(|(block, _)| block.header().len)).sum();
        assert_eq!(old as i64, cut_k, "the old segment's own samples");
        // The loopback the reopened streamer will read, on transmit channel 1 and 30 ms before
        // the destination's start, inside the send's 50 ms flow-control window.
        let wave: Vec<[f32; 2]> = (0..2_000).map(|i| [i as f32 / 4_000.0, 0.125]).collect();
        super::super::test_support::device_at(&core, &time, &device, destination.origin - ms(30));
        device.tx_open(1, &[1]).unwrap();
        device.tx_send(1, &[&wave], Some(destination.origin), true, true, Wall::from_secs(1)).unwrap();
        assert!(rx.begin(), "the owner begins the new segment");
        let (domain, origin) = rx.stream.as_ref().map(|stream| (stream.clock.domain, stream.clock.origin)).expect("the new segment's stream");
        assert_eq!(origin, destination.origin, "the new segment's own origin");
        let calls = device.calls();
        assert_eq!(calls.iter().filter(|call| call.as_str() == "rx_open 0 [1]").count(), 1, "the streamer is not reopened for the new set: {calls:?}");
        assert!(calls.iter().any(|call| call.starts_with("apply rx 1 ") && call.ends_with("at=-")), "the new set's channel is not configured: {calls:?}");
        // What the reopened streamer delivers: channel 1's loopback, sample for sample.
        super::super::test_support::device_at(&core, &time, &device, origin + 2_000 * 200);
        let got = match device.rx_recv(0, 2_000, Wall::from_millis(100), false) {
            RxRecv::Samples { first_tick, samples } => {
                assert_eq!(first_tick, origin, "the streamer's first sample");
                samples
            }
            other => panic!("the new streamer's samples: {other:?}"),
        };
        assert_eq!(got.len(), 1, "the new streamer carries one channel");
        assert_eq!(got[0][..3], wave[..3], "the new segment's samples are not channel 1's loopback: {:?}", &got[0][..3]);
        assert!(got[0].iter().all(|sample| sample[1] == 0.125), "the new segment's channel is not 1: {:?}", &got[0][..3]);
        // And the owner publishes them, on its new segment's own clock.
        rx.receive(RxRecv::Samples { first_tick: origin, samples: got });
        let block = first.receive().expect("the new segment's block").0;
        assert_eq!(block.header().first_sample_time, TimePoint::new(domain, 0), "the block is not the new clock's first samples");
        let (len, channels) = (block.header().len, block.header().channels);
        let bytes = block.host_bytes().expect("a host block");
        let samples: Vec<Iq> = (0..len as usize)
            .map(|i| {
                let (re, im) = ezsdr_hostmem::read_cf32(bytes, len as usize, 0, i);
                [re, im]
            })
            .collect();
        assert_eq!((channels, len), (1, 2_000), "{:?}", block.header());
        assert_eq!(samples[..3], wave[..3], "the delivered block is not channel 1's: {:?}", &samples[..3]);
        assert!(samples.iter().all(|sample| sample[1] == 0.125), "the delivered block is not channel 1's: {:?}", &samples[..3]);
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
        core.rx_chain[0].lock().request_end_now();
        rx.try_end();
        assert!(rx.stream.is_none(), "the first segment is stopped before its successor is configured");
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
        let (mut rx, _) = rx(&core, 1);
        core.rx_chain[0].lock().request_end_now();
        rx.try_end();
        assert!(rx.stream.is_none(), "the first segment is stopped before its successor is configured");
        let ms = |n: i64| core.ticks(n * 1_000_000);
        book(&core, ms(20), 2, rate(1_000_000));
        book(&core, ms(40), 3, Kind::Start);
        assert!(rx.begin(), "the failed configure is reported");
        assert!(rx.begin());
        let calls = device.calls();
        assert_eq!(calls.iter().filter(|call| call.starts_with("rx_open")).count(), 1, "the initial open is reused: {calls:?}");
        assert_eq!(calls.iter().filter(|call| call.starts_with("apply")).count(), 2, "{calls:?}");
    }

    #[test]
    fn ur_25_a_refused_segment_is_booked_by_uhd_rx() {
        // RM-25: when the device refuses a segment's configuration, uhd-rx books the change as
        // refused itself, which halts the stream, and reads the plan made so at once: a
        // `start_rx` that was a no-op resumes the stream. The refused segment (origin 68 ms) runs
        // for planning until that `start_rx` cuts it, with no sample, at its origin, so the new one
        // begins after the call in progress there (1 ms at 2 MS/s), the allowance (3 ms) and the
        // release window (3 ms), and lead (50 ms): 125 ms.
        let refuse = crate::device::FakeFault::WrongRate { claimed: 2e6, applied: 2e6 - 1_000.0, nth: 0 };
        let (core, device, _, _) = super::super::test_support::rig_with(crate::device::FakeConfig { faults: vec![refuse], ..Default::default() }, vec![link()]);
        let (mut rx, _) = rx(&core, 1);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        book(&core, ms(10), 1, rate(2_000_000));
        book(&core, ms(20), 2, Kind::Start);
        assert_eq!(lock(&core.streams).planned(stream_key(Dir::Rx, 0)).iter().map(|p| (p.segment.origin, p.segment.by)).collect::<Vec<_>>(), [(0, None), (ms(68), Some(1))]);
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
        assert_eq!(starts, ["rx_start 0 0".to_owned(), format!("rx_start 0 {}", ms(125))]);
        assert_eq!(rx.stream.as_ref().map(|stream| stream.config.ratio.num()), Some(100), "the refused rate is the stream's (KC-27)");
        let streams = lock(&core.streams);
        let active = streams.active(stream_key(Dir::Rx, 0)).expect("the resumed segment");
        assert_eq!((active.0, active.2), (ms(125), Some(2)), "the command that began the resumed segment is the Start's");
        assert_eq!(active.1.ratio.num(), 100);
        assert_eq!(streams.first_undelivered(stream_key(Dir::Rx, 0)), Some((ms(125), active.1, 0)));
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
        for (origin, len) in [(ms(68), 270_000), (ms(257), 43_000)] {
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
            release_window: 0,
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
            assert!(streams.book(
                &core,
                tx_key,
                Item {
                    e: ms(20),
                    seq: 1,
                    ready: ms(20),
                    release: NO_RELEASE,
                    delivered: None,
                    refused: false,
                    kind: Kind::Cold(Config { channels: Channels::first(1), ratio: Rational::new(100, 1).unwrap() })
                }
            ));
        }
        let snapshot = |key: StreamKey| -> Held {
            let streams = lock(&core.streams);
            (
                streams.line(key).expect("a line").items.iter().map(|item| (item.e, item.seq, item.refused)).collect(),
                streams
                    .planned(key)
                    .into_iter()
                    .map(|planned| (planned.segment.origin, planned.segment.cut, planned.segment.config.channels.iter().collect::<Vec<u32>>(), planned.segment.by))
                    .collect(),
                streams.stream(key).clocks.iter().map(|(clock, ended)| (clock.origin, *ended)).collect(),
            )
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
            streams.lines.insert(
                other,
                Line::new(ezsdr_radio::timeline::Stream {
                    direction: Direction::Rx,
                    origin: 0,
                    config: Config { channels: Channels::first(0).with(1), ratio: Rational::new(200, 1).unwrap() },
                    lead: 0,
                    call: 0,
                    release_window: 0,
                    allowance: 0,
                }),
            );
            streams.stream_mut(other).configs.insert(1, Arc::new(ColdConfig::new(BTreeMap::new())));
            assert!(streams.book(
                &core,
                other,
                Item {
                    e: ms(30),
                    seq: 1,
                    ready: ms(30),
                    release: NO_RELEASE,
                    delivered: None,
                    refused: false,
                    kind: Kind::Cold(Config { channels: Channels::first(0).with(1), ratio: Rational::new(100, 1).unwrap() })
                }
            ));
        }
        let snapshot = |key: StreamKey| -> Held {
            let streams = lock(&core.streams);
            (
                streams.line(key).expect("a line").items.iter().map(|item| (item.e, item.seq, item.refused)).collect(),
                streams
                    .planned(key)
                    .into_iter()
                    .map(|planned| (planned.segment.origin, planned.segment.cut, planned.segment.config.channels.iter().collect::<Vec<u32>>(), planned.segment.by))
                    .collect(),
                streams.stream(key).clocks.iter().map(|(clock, ended)| (clock.origin, *ended)).collect(),
            )
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
        assert_eq!(device.calls().iter().filter(|call| call.starts_with("rx_start")).count(), 1, "only the initial start: {:?}", device.calls());
    }
}
