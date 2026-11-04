//! uhd-control (UR-14): drains the `ActionReceiver` with MA-14b's loop, books each
//! Action — a stream's commands on its timeline, which it shares with the stream's owner —
//! and every 500 ms checks the reference and reads the device's time. It never waits for a
//! device instant.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;

use std::time::Duration as Wall;

use ezsdr_kernel::event::{Action, ActionId, Severity};
use ezsdr_kernel::id::{ClockDomainId, ResourceId};
use ezsdr_kernel::module_api::{ActionReceiver, Dispatched};
use ezsdr_kernel::spec::{Key, Value};
use ezsdr_kernel::stream::{BurstOpen, Direction, LateOutcome};
use ezsdr_kernel::time::{AbsoluteDeadline, Duration, Rational, TimeError, TimePoint};
use ezsdr_radio::kinds;
use ezsdr_radio::payloads::{ClockLostPayload, ClockReference, TimeErrorCause, TimeErrorOutcome, TimeErrorPayload};
use ezsdr_radio::timeline::{self, Channels, Config, Item, Kind, Line, Segment, StreamKey};
use serde_json::json;

#[cfg(test)]
use super::core::booked_stream;
use super::core::{Clock, Core, lock, stream_key};
use super::mode::DescriptionExt;
use super::tx::{Held, TxCmd};
use crate::device::{DeviceError, Dir, Iq, Settings, decimation};
use crate::host::HostClock;
use crate::profile::{DELIVERY_ALLOWANCE_NS, DEVICE_LEAD_NS};
use ezsdr_radio::Addressed;

const POLL: Wall = Wall::from_millis(1);
const CHECK: Wall = Wall::from_millis(500);

/// The configuration a segment begins with, shared with its owner: the one in effect at the
/// segment's origin (UR-12, UR-17), its rate and channel set the segment's. A Run's settings
/// change only at its start (note 47), so this is the configuration `start_rx` snapshots.
pub(crate) struct ColdConfig {
    base: BTreeMap<Key, Value>,
}

impl ColdConfig {
    pub(crate) fn new(base: BTreeMap<Key, Value>) -> Self {
        Self { base }
    }

    /// Configures the direction for `segment` (UR-12), each of its channels' own settings;
    /// returns the decimation.
    pub fn configure(&self, core: &Core, dir: Dir, segment: &Segment, k: usize) -> Result<i64, DeviceError> {
        let ratio = segment.config.ratio;
        let rate = core.mcr as f64 * ratio.den() as f64 / ratio.num() as f64;
        let channels: Vec<(usize, Settings)> = segment
            .config
            .channels
            .iter()
            .map(|chan| {
                let mut settings = core.description.settings(&self.base, dir, k, chan as usize);
                settings.rate = Some(rate);
                (chan as usize, settings)
            })
            .collect();
        core.configure(dir, &channels, rate, k)
    }
}

/// A planned segment as the stream's owner reads it (UR-25).
#[derive(Clone)]
pub(crate) struct Planned {
    pub segment: Segment,
    /// The stream's number (RM-29), which an applied rate row names: the rate is the
    /// stream's key under the layout.
    pub k: usize,
    /// The configuration in effect at its origin; none for the stream's first segment, or one
    /// whose command is pruned, which the owner has begun already.
    pub settings: Option<Arc<ColdConfig>>,
    /// The command that began it was refused at its instant, by uhd-control or by the stream's
    /// owner; a refused segment halts its stream (UR-25).
    pub refused: bool,
}

impl Planned {
    /// The segment's channel set (RM-29): the stream's channels in channel order.
    pub fn set(&self) -> Channels {
        self.segment.config.channels
    }

    /// Root ticks per sample.
    pub fn n(&self) -> i64 {
        self.segment.config.ratio.num() as i64
    }

    /// Configures the direction for the segment (UR-12), as [`ColdConfig::configure`].
    pub fn configure(&self, core: &Core, dir: Dir) -> Result<i64, DeviceError> {
        match &self.settings {
            Some(settings) => settings.configure(core, dir, &self.segment, self.k),
            None => Err(DeviceError::failed("UR-25: the segment has no configuration")),
        }
    }
}

pub(crate) struct Control {
    pub core: Arc<Core>,
    pub actions: Arc<dyn ActionReceiver>,
    /// One sender per transmit stream (RM-29).
    pub to_tx: Vec<Sender<TxCmd>>,
    /// The configuration at the Run's start, which a `start_rx` segment begins with (note 47: the
    /// Run's settings change only at its start).
    pub config: BTreeMap<Key, Value>,
    pub reference_monitored: bool,
    clock_lost: bool,
    /// When the device reads began failing without a lost-device error (Review S, S-B1);
    /// in the host clock's time (plan 30 §4.3).
    failing_since: Option<Wall>,
    /// The device's host clock (plan 30 §4.3): the control poll and its checks run on it.
    host: HostClock,
}

/// How long the device reads may fail, whatever the error, before the device is lost: a
/// dead link's control requests time out in UHD as `op_timeout`, which its C API returns as
/// `UHD_ERROR_EXCEPT`, not counted lost (UR-29; Review S, S-B1).
const FAILING_FOR: Wall = Wall::from_secs(1);

/// The streams at the Run's start (UR-15): each receive stream's first segment at T0, when
/// it has a channel and a link, and each transmit stream's clock registered at `arm`.
pub(crate) struct Start {
    pub t0: i64,
    /// Per receive stream `k`: its channel set and root ticks per sample, when it starts.
    pub rx: Vec<Option<(Channels, i64)>>,
    /// Per transmit stream `k`: its clock and channel set, when it has channels.
    pub tx: Vec<Option<(Clock, Channels)>>,
}

impl Control {
    /// uhd-control, with the streams' timelines shared from the Run's start.
    pub fn new(core: Arc<Core>, actions: Arc<dyn ActionReceiver>, to_tx: Vec<Sender<TxCmd>>, config: BTreeMap<Key, Value>, reference_monitored: bool, start: Start) -> Control {
        let host = core.device.host();
        // RM-25, VH-4: this Module's terms — each direction's start lead, and, for a receive
        // segment after a cut, the receive call in progress there and the delivery
        // allowance (UR-25). The call term is each receive stream's own packet size.
        let allowance = core.ticks(DELIVERY_ALLOWANCE_NS);
        let release_window = core.ticks(core.description.timing.release_window_ns);
        let line = |key: StreamKey, origin: i64, channels: Channels, n: i64, call: i64| {
            Line::new(timeline::Stream {
                direction: key.0,
                origin,
                config: Config { channels, ratio: Rational::new(n.max(1) as u64, 1).expect("a positive ratio") },
                lead: core.ticks(ezsdr_radio::timeline::start_lead_ns(&core.description.timing, key.0)),
                call,
                release_window: if key.0 == Direction::Rx { release_window } else { 0 },
                allowance,
            })
        };
        let n = |dir: Dir, k: usize| {
            let rate = core.description.stream_rate(&config, dir, k);
            decimation(core.mcr, rate).map_or(1, |n| n as i64)
        };
        let rx_count = core.description.stream_count(Dir::Rx);
        let tx_count = core.description.stream_count(Dir::Tx);
        let mut rx = Vec::new();
        for k in 0..rx_count {
            if core.links(k).is_empty() {
                rx.push(None);
                continue;
            }
            let call = core.block_len.max(core.device.rx_packet_samples(k)) as i64;
            let (channels, n) = start.rx.get(k).copied().flatten().unwrap_or((Channels::first(0), n(Dir::Rx, k)));
            rx.push(Some(line(stream_key(Dir::Rx, k), start.t0, channels, n, call)));
        }
        let mut tx = Vec::new();
        for k in 0..tx_count {
            tx.push(match start.tx.get(k).copied().flatten() {
                Some((clock, channels)) => line(stream_key(Dir::Tx, k), clock.origin, channels, clock.n, core.block_len as i64),
                None => line(stream_key(Dir::Tx, k), 0, Channels::first(0), n(Dir::Tx, k), core.block_len as i64),
            });
        }
        {
            let mut streams = lock(&core.streams);
            for (k, line) in rx.iter().enumerate() {
                if let Some(line) = line.clone() {
                    streams.lines.insert(stream_key(Dir::Rx, k), line);
                }
            }
            for (k, line) in tx.iter().enumerate() {
                streams.stream_mut(stream_key(Dir::Tx, k)).clocks = start.tx.get(k).copied().flatten().map(|(clock, _)| clock).into_iter().collect();
                streams.lines.insert(stream_key(Dir::Tx, k), line.clone());
            }
        }
        Control { core, actions, to_tx, config, reference_monitored, clock_lost: false, failing_since: None, host }
    }

    pub fn run(mut self) {
        let mut last_check = self.host.now();
        loop {
            if lock(&self.core.streams).stop.is_some() {
                return self.finish();
            }
            // MA-14b: each Action is booked before the next `recv()`.
            while let Some(dispatched) = self.actions.recv() {
                if !self.core.is_lost() {
                    self.book(dispatched);
                }
            }
            if self.host.now().saturating_sub(last_check) >= CHECK && !self.core.is_lost() {
                last_check = self.host.now();
                self.check_device();
            }
            self.host.sleep(POLL);
        }
    }

    /// UR-26: `Provider::stop`, or a loss, which has booked the streams' end, records UR-30's
    /// plans.
    pub(super) fn finish(&mut self) {
        lock(&self.core.streams).ended_at().expect("UR-26: the end was booked before uhd-control finishes");
        let plan = lock(&self.core.streams).record();
        self.core.timing(plan);
    }

    /// `node` is what the Kernel resolved the target to (KC-23), which is what a
    /// Provider matches on.
    pub(super) fn book(&mut self, dispatched: Dispatched) {
        let id = dispatched.id;
        let node = dispatched.node.as_ref();
        let addressed = self.addressed(node);
        match dispatched.action {
            Action::TxBurst { waveform, repeat, at, requested_at, late_policy, metadata, .. } => {
                self.book_burst(id, addressed, waveform, repeat, at, requested_at, late_policy, metadata.is_empty());
            }
            // Note 47: a radio key has no update class, so the Kernel refuses a Run-time update of one
            // before dispatch; this is the Provider's own guard for a harness that calls it directly.
            Action::UpdateParameter { key, .. } => {
                // RM-12: the node selects what the update may reach: the instance and its streams.
                if matches!(addressed, Addressed::Other) {
                    self.core.command_rejected(Some(id), "update_parameter", "UR-26: the target is not this device");
                } else {
                    self.core.command_rejected(Some(id), "update_parameter", &format!("UR-26: {key} is not updated at runtime by this Provider"));
                }
            }
            // RM-16, UR-26: a `Stop` cancels no update; on receive it cuts at its booking, on
            // transmit it ends the bursts, not the clock. `<id>` stops every stream; a
            // stream node stops that stream alone (RM-12, D14); under the layout the old
            // `<id>/rx` and `<id>/tx` are no targets.
            Action::Stop { target: Some(_) } => {
                let (rx, tx): (Vec<usize>, Vec<usize>) = match addressed {
                    Addressed::Instance => ((0..self.core.description.stream_count(Dir::Rx)).collect(), (0..self.core.description.stream_count(Dir::Tx)).collect()),
                    Addressed::Stream { direction, k } => {
                        let dir = Dir::from(direction);
                        ((dir == Dir::Rx).then_some(k).into_iter().collect(), (dir == Dir::Tx).then_some(k).into_iter().collect())
                    }
                    Addressed::Other => {
                        return self.core.command_rejected(Some(id), "stop", "UR-26: the Stop target is not this device or its streams");
                    }
                };
                for k in tx {
                    let stream = stream_key(Dir::Tx, k);
                    let mut streams = lock(&self.core.streams);
                    let now = self.core.now();
                    let seq = streams.next_seq();
                    streams.book(&self.core, stream, Item { e: now, seq, ready: now, delivered: None, refused: false, kind: Kind::Stop });
                    drop(streams);
                    if let Some(to_tx) = self.to_tx.get(k) {
                        let _ = to_tx.send(TxCmd::Stop);
                    }
                }
                if !rx.is_empty() {
                    let mut streams = lock(&self.core.streams);
                    let now = self.core.now();
                    for k in rx {
                        let stream = stream_key(Dir::Rx, k);
                        let seq = streams.next_seq();
                        streams.book(&self.core, stream, Item { e: now, seq, ready: now, delivered: None, refused: false, kind: Kind::Stop });
                    }
                }
            }
            Action::Stop { target: None } | Action::Abort { .. } => {}
            Action::SetTimer { .. } => self.core.command_rejected(Some(id), "set_timer", "UR-26: not supported by this Provider"),
            // RM-12, RM-21: `start_rx` turns the receive stream on at its `at`, or at its booking
            // if that is later; it is never late. A stream the layout holds no channel for is
            // accepted and carries out nothing, as MockRadio's does (D14).
            Action::Command { verb, params, at, .. } if verb.as_str() == ezsdr_radio::START_RX && params.is_empty() => {
                let Addressed::Stream { direction: Direction::Rx, k } = addressed else {
                    return self.core.command_rejected(Some(id), "command", "UR-26: not supported by this Provider");
                };
                let requested = match self.ceil_root(at) {
                    Ok(requested) => requested,
                    Err(error) => return self.core.command_rejected(Some(id), "command", &format!("UR-26: start_rx's instant cannot be converted: {error}")),
                };
                // One critical section takes the instant and the arrival order and books the
                // start, so that nothing pruned meanwhile takes effect after them (#63).
                let mut streams = lock(&self.core.streams);
                let stream = stream_key(Dir::Rx, k);
                let (seq, now) = (streams.next_seq(), self.core.now());
                let e = timeline::command_instant(streams.line(stream), requested, now);
                streams.stream_mut(stream).configs.insert(seq, Arc::new(ColdConfig::new(self.config.clone())));
                streams.book(&self.core, stream, Item { e, seq, ready: now, delivered: None, refused: false, kind: Kind::Start });
            }
            Action::Command { .. } => self.core.command_rejected(Some(id), "command", "UR-26: not supported by this Provider"),
            Action::Emit { .. } => self.core.command_rejected(Some(id), "emit", "UR-26: not supported by this Provider"),
        }
    }

    /// What an Action's resolved node names (KC-23, RM-12, RM-29): the instance, one of its
    /// streams, or something else. The judgement is the shared `ezsdr_radio::addressed`.
    fn addressed(&self, node: Option<&ResourceId>) -> Addressed {
        self.core.description.addressed(&self.core, node)
    }

    fn ceil_root(&self, at: Option<AbsoluteDeadline>) -> Result<Option<i64>, TimeError> {
        let Some(t) = at.map(|deadline| deadline.time_point) else {
            return Ok(None);
        };
        let root = self.core.root;
        if let Ok(ticks) = t.ticks_in(root) {
            return Ok(Some(ticks));
        }
        let ticks = match self.core.clocks.convert(t, root)? {
            ezsdr_kernel::time::Converted::Exact { point } => point.ticks_in(root)?,
            ezsdr_kernel::time::Converted::Inexact { floor, .. } => floor.ticks_in(root)?.checked_add(1).ok_or(TimeError::Overflow)?,
        };
        Ok(Some(ticks))
    }

    // ------------------------------------------------------------ UR-24

    // ------------------------------------------------------------ UR-25

    // ------------------------------------------------------------ UR-21

    #[allow(clippy::too_many_arguments)]
    fn book_burst(
        &mut self,
        id: ActionId,
        addressed: Addressed,
        waveform: ezsdr_kernel::manifest::ArtifactRef,
        repeat: bool,
        at: AbsoluteDeadline,
        requested_at: Option<AbsoluteDeadline>,
        late_policy: ezsdr_kernel::stream::LatePolicy,
        metadata_empty: bool,
    ) {
        let reject = |reason: &str| self.core.command_rejected(Some(id), "tx_burst", reason);
        // RM-12, RM-29: the target must be a transmit stream's node.
        let k = match addressed {
            Addressed::Stream { direction: Direction::Tx, k } => k,
            _ => return reject("UR-21: the target is not this device's transmit stream"),
        };
        // KC-21a: the last transmit clock registered, while its segment has not ended.
        let current = {
            let streams = lock(&self.core.streams);
            let stream = stream_key(Dir::Tx, k);
            let last = streams.line(stream).and_then(|line| line.plan.last()).filter(|segment| segment.cut.is_none());
            streams.stream(stream).clocks.last().copied().zip(last.map(|segment| segment.config.channels.count() as usize))
        };
        if current.is_none() {
            return reject("UR-21: no transmit channel is configured");
        }
        let Some((clock, channels)) = current else {
            return reject("UR-21: no transmit channel is configured");
        };
        if at.time_point.domain() != clock.domain {
            return reject("UR-21: the start is not on the current transmit SampleClock");
        }
        let unit = 8 * channels as u64;
        if waveform.size_bytes == 0 || waveform.size_bytes % unit != 0 {
            return reject("UR-21: the waveform size is not a positive multiple of 8 · channels");
        }
        let Some(bytes) = self.core.inputs.get(&waveform.hash).filter(|b| b.len() as u64 == waveform.size_bytes) else {
            return reject("UR-21: the waveform's bytes are not an input of this Run");
        };
        let len = bytes.len() / unit as usize;
        let mut samples: Vec<Vec<Iq>> = vec![Vec::with_capacity(len); channels];
        for frame in bytes.chunks_exact(unit as usize) {
            for (chan, pair) in frame.chunks_exact(8).enumerate() {
                let re = f32::from_le_bytes(pair[0..4].try_into().expect("four bytes"));
                let im = f32::from_le_bytes(pair[4..8].try_into().expect("four bytes"));
                if !re.is_finite() || !im.is_finite() {
                    return reject("UR-21: a waveform sample is not finite");
                }
                samples[chan].push([re, im]);
            }
        }
        if repeat && len as u64 > self.core.description.repeat_max_samples {
            return reject("UR-21: the repeated waveform exceeds radio.tx.repeat_max_samples");
        }
        if !metadata_empty {
            return reject("UR-21: TxBurst.metadata must be empty");
        }
        // RM-14 on the device lead, RM-15's origin bound (VE-2; N-P0-1).
        let lead = self.core.ticks(DEVICE_LEAD_NS);
        let now = timeline::decided_at(self.core.now(), clock.origin, lead);
        let now_k = clock.at_or_after(now);
        let decided = late_policy.decide(&self.core.clocks, at.time_point, TimePoint::new(clock.domain, now_k), Duration::new(ClockDomainId::HOST_MONOTONIC, DEVICE_LEAD_NS));
        let outcome = match decided {
            Ok(outcome) => outcome,
            Err(error) => return reject(&format!("UR-21: {error}")),
        };
        let mut start = match at.time_point.ticks_in(clock.domain) {
            Ok(k) => k,
            Err(error) => return reject(&format!("UR-21: {error}")),
        };
        let mut requested = requested_at.map(|d| d.time_point);
        let time_error = |outcome: TimeErrorOutcome, late_by: Duration| {
            let payload =
                serde_json::to_value(TimeErrorPayload { cause: TimeErrorCause::Late, outcome, late_by_ns: late_by.ticks_in(ClockDomainId::HOST_MONOTONIC).unwrap_or(i64::MAX), target: at.time_point })
                    .expect("a payload");
            // The burst's own stream (RM-29).
            self.core.emit(&self.core.stream_node(Dir::Tx, k), kinds::TIME_ERROR, Severity::Error, payload);
        };
        match outcome {
            LateOutcome::OnTime {} => {}
            LateOutcome::SendAsap { late_by } => {
                start = clock.at_or_after(clock.instant(now_k) + lead);
                requested = requested.or(Some(at.time_point));
                let held = lock(&self.core.held).contains(&(clock.domain, start));
                if held {
                    time_error(TimeErrorOutcome::Refused, late_by);
                    return reject("UR-21: the moved start is a held burst's");
                }
                time_error(TimeErrorOutcome::SendAsap, late_by);
            }
            LateOutcome::Drop { late_by } => {
                time_error(TimeErrorOutcome::Drop, late_by);
                return self.core.reject_note(json!({ "kind": "tx_burst", "reason": "UR-21: the late policy dropped the burst" }));
            }
            LateOutcome::PlanViolation { late_by } => {
                time_error(TimeErrorOutcome::PlanViolation, late_by);
                return self.core.reject_note(json!({ "kind": "tx_burst", "reason": "UR-21: the planned burst arrived late" }));
            }
        }
        if clock.instant(start) < self.core.start_up.load(Ordering::Acquire) {
            return reject("UR-21: the burst begins before the device's start-up");
        }
        if !lock(&self.core.held).insert((clock.domain, start)) {
            return reject("UR-21: a held burst already has this start");
        }
        let open = BurstOpen { waveform_len: u32::try_from(len).ok(), late: (!matches!(outcome, LateOutcome::OnTime {})).then_some(outcome), requested_target: requested };
        let _ = self
            .to_tx
            .get(k)
            .map(|to_tx| to_tx.send(TxCmd::Burst(Held { k: start, domain: clock.domain, samples: Arc::new(samples), waveform_len: len, repeat, open, policy: late_policy, target: at.time_point })));
    }

    // ------------------------------------------------------------ UR-27, UR-29

    fn check_device(&mut self) {
        if self.reference_monitored && !self.clock_lost {
            match self.core.device.ref_locked() {
                Ok(Some(false)) => {
                    self.clock_lost = true;
                    let payload = serde_json::to_value(ClockLostPayload { reference: ClockReference::Frequency }).expect("a payload");
                    self.core.emit(&self.core.id, kinds::CLOCK_LOST, Severity::Fatal, payload);
                }
                Ok(_) => {}
                Err(error) if error.lost => return self.core.device_lost(&error.message),
                // Recorded only: a device whose time it can read is not gone (Review T, NB-T1).
                Err(error) => self.core.timing(json!({ "what": "ref_locked_failed", "error": error.message })),
            }
        }
        // Time reads failing for 1 s from the start of the first that failed: the device is
        // gone, however UHD classified the error (one read on a dead link waits out UHD's
        // timeouts); a read that succeeds starts the count again.
        let begun = self.host.now();
        match self.core.device.time_now() {
            Ok(_) => self.failing_since = None,
            Err(error) if error.lost => self.core.device_lost(&error.message),
            Err(error) => {
                self.core.timing(json!({ "what": "time_read_failed", "error": error.message }));
                let since = *self.failing_since.get_or_insert(begun);
                if self.host.now().saturating_sub(since) >= FAILING_FOR {
                    self.core.device_lost(&format!("UR-29: the device time reads have failed for 1 s: {}", error.message));
                }
            }
        }
    }
}

#[cfg(test)]
mod differential;

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{Receiver, channel};

    use super::super::core::key;
    use super::*;
    use crate::provider::test_support::device_at;
    use ezsdr_kernel::spec::Scalar;
    use ezsdr_kernel::stream::{BackPressure, BlockRef, DataLink, DropCarry, PublishOutcome};

    struct NoActions;
    impl ActionReceiver for NoActions {
        fn recv(&self) -> Option<ezsdr_kernel::module_api::Dispatched> {
            None
        }
    }

    /// A receive link that takes nothing, so that the receive side has a stream.
    struct NoLink;
    impl DataLink for NoLink {
        fn publish(&self, _block: BlockRef) -> PublishOutcome {
            PublishOutcome::Accepted
        }
        fn receive(&self) -> Option<(BlockRef, DropCarry)> {
            None
        }
        fn drops(&self) -> u64 {
            0
        }
        fn take_drop_carry(&self) -> DropCarry {
            DropCarry::default()
        }
        fn policy(&self) -> BackPressure {
            BackPressure::DropOldest
        }
    }

    fn rig() -> (Arc<Core>, Arc<crate::device::FakeDevice>, Arc<ezsdr_kernel::time::ManualTimeAuthority>) {
        let (core, device, time, _) = crate::provider::test_support::rig_with_links(vec![Arc::new(NoLink)]);
        (core, device, time)
    }

    /// uhd-control with one channel at 1 MS/s on each direction in `on` from 0.
    fn control(core: &Arc<Core>, on: &[Dir]) -> (Control, Receiver<TxCmd>) {
        let (to_tx, tx) = channel();
        let mut config = core.description.defaults();
        let mut start = Start { t0: 0, rx: vec![None; core.description.stream_count(Dir::Rx)], tx: vec![None; core.description.stream_count(Dir::Tx)] };
        config.insert(key("radio.rx.c0.stream"), Value::from(if on.contains(&Dir::Rx) { 0 } else { -1 }));
        config.insert(key("radio.tx.c0.stream"), Value::from(if on.contains(&Dir::Tx) { 0 } else { -1 }));
        for dir in on {
            match dir {
                Dir::Rx => {
                    start.rx[0] = Some((Channels::first(1), 200));
                }
                Dir::Tx => {
                    start.tx[0] = Some((core.register(Dir::Tx, 200, 0).unwrap(), Channels::first(1)));
                }
            }
        }
        (Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], config, false, start), tx)
    }

    /// The plan of `dir`'s stream as its owner reads it.
    fn plan_of(core: &Core, dir: Dir) -> Vec<Planned> {
        lock(&core.streams).planned(booked_stream(dir))
    }

    /// Transmit stream 0's clock, registered at `arm` (RM-25).
    fn tx_clock(core: &Core) -> Clock {
        lock(&core.streams).stream(booked_stream(Dir::Tx)).clocks[0]
    }

    /// The device's clock held still from its epoch, tick 0, asserted: every instant a test sets
    /// later is past it (`Frozen`).
    fn hold(core: &Core, time: &ezsdr_kernel::time::ManualTimeAuthority, device: &crate::device::FakeDevice) -> crate::provider::test_support::Frozen {
        let frozen = crate::provider::test_support::Frozen::of(device);
        device_at(core, time, device, 0);
        frozen
    }

    /// The layout (RM-27): two receive front ends, one transmit.
    fn layout_description() -> ezsdr_radio::channels::Description {
        let selector = BTreeMap::from([
            (ezsdr_kernel::spec::Ident::parse("rx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:B:ubx:0")])),
            (ezsdr_kernel::spec::Ident::parse("tx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0")])),
        ]);
        let layout = ezsdr_radio::channels::from_selector(&selector).unwrap();
        ezsdr_radio::channels::Description::new(&layout, 2_000, &crate::profile::terms())
    }

    #[test]
    fn ur_34_a_transmit_stop_books_the_wait_exit_and_queues_the_owner_stop() {
        let (core, _device, _time) = rig();
        let (mut control, owner) = control(&core, &[Dir::Tx]);
        control.book(Dispatched::new(ActionId(1), Action::Stop { target: Some(super::super::test_support::target_of(&core.tx_id)) }, Some(core.tx_id.clone())));
        let stream = booked_stream(Dir::Tx);
        let stop = lock(&core.streams).line(stream).unwrap().items.iter().find(|item| item.kind == Kind::Stop).copied().unwrap();
        assert!(matches!(owner.try_recv().unwrap(), TxCmd::Stop));
        assert_eq!(stop.e, core.now());
    }

    #[test]
    fn ur_21_the_receipt_site_reserves_the_moved_start_before_any_hand_over() {
        // UR-21 (plan 30 stage A): the receipt site's `SendAsap` move reserves the moved start
        // before uhd-tx decides again at its first hand-over — that reservation is what the
        // moved start's collision, its start-up refusal and the handed-over start observe.
        // Removing the move leaves the requested start reserved instead (row J315).
        let samples: Vec<(f32, f32)> = vec![(0.25, 0.0), (0.5, 0.0), (0.75, 0.0), (1.0, 0.0)];
        let bytes: Vec<u8> = samples.iter().flat_map(|(re, im)| [re.to_le_bytes(), im.to_le_bytes()].concat()).collect();
        let hash = ezsdr_kernel::hash::ContentHash::of_bytes(&bytes);
        let waveform =
            ezsdr_kernel::manifest::ingest_input(ezsdr_kernel::spec::Ident::parse("waveform").unwrap(), ezsdr_kernel::spec::Namespace::parse("ezsdr.input").unwrap(), format!("mem:{hash}"), &bytes);
        let (core, _device, time, events) = crate::provider::test_support::rig_with_inputs(BTreeMap::from([(hash, Arc::<[u8]>::from(bytes))]));
        let (mut control, to_tx) = control(&core, &[Dir::Tx]);
        let clock = tx_clock(&core);
        // Booking 10 ms in: the requested start (sample 0) is far behind the 2 ms device lead.
        time.advance_to(core.at(core.ticks(10_000_000))).unwrap();
        let requested = TimePoint::new(clock.domain, 0);
        control.book(Dispatched::new(
            ActionId(1),
            Action::TxBurst {
                target: super::super::test_support::target_of(&core.tx_id),
                waveform,
                repeat: false,
                at: AbsoluteDeadline::new(requested),
                requested_at: None,
                late_policy: ezsdr_kernel::stream::LatePolicy::SendAsapAndFlag,
                metadata: BTreeMap::new(),
            },
            Some(core.tx_id.clone()),
        ));
        let verdict =
            events.drain(ezsdr_kernel::time::TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).into_iter().find(|e| e.kind.as_str() == kinds::TIME_ERROR).expect("the SendAsap verdict at booking");
        assert_eq!(verdict.payload["outcome"], "send_asap", "{verdict:?}");
        // Still between the booking and uhd-tx's first hand-over: what uhd-control hands over
        // is the moved start, and the moved start is what its reservation holds.
        let TxCmd::Burst(held) = to_tx.try_recv().expect("the receipt site handed the burst to uhd-tx") else { panic!("not a burst") };
        let moved = held.k;
        assert_ne!(moved, requested.ticks_in(clock.domain).unwrap(), "the receipt move was not applied");
        assert!(clock.instant(moved) >= core.ticks(10_000_000) + core.ticks(DEVICE_LEAD_NS), "the moved start is not the device lead past the booking instant: {}", clock.instant(moved));
        assert_eq!(lock(&core.held).iter().copied().collect::<Vec<_>>(), vec![(clock.domain, moved)], "the moved start is reserved before the hand-over");
    }

    #[test]
    fn ur_21_production_booking_preserves_the_original_repeat_length() {
        let samples: Vec<[f32; 2]> = (0..997).map(|i| [i as f32 / 1_000.0, 0.25]).collect();
        let bytes: Vec<u8> = samples.iter().flat_map(|sample| [sample[0].to_le_bytes(), sample[1].to_le_bytes()].concat()).collect();
        let hash = ezsdr_kernel::hash::ContentHash::of_bytes(&bytes);
        let waveform =
            ezsdr_kernel::manifest::ingest_input(ezsdr_kernel::spec::Ident::parse("waveform").unwrap(), ezsdr_kernel::spec::Namespace::parse("ezsdr.input").unwrap(), format!("mem:{hash}"), &bytes);
        let (core, _device, time, _) = crate::provider::test_support::rig_with_inputs(BTreeMap::from([(hash, Arc::<[u8]>::from(bytes))]));
        let (mut control, to_tx) = control(&core, &[Dir::Tx]);
        let clock = tx_clock(&core);
        time.advance_to(core.at(core.ticks(10_000_000))).unwrap();
        control.book(Dispatched::new(
            ActionId(1),
            Action::TxBurst {
                target: super::super::test_support::target_of(&core.tx_id),
                waveform,
                repeat: true,
                at: AbsoluteDeadline::new(TimePoint::new(clock.domain, 20_000)),
                requested_at: None,
                late_policy: ezsdr_kernel::stream::LatePolicy::DropAndFlag,
                metadata: BTreeMap::new(),
            },
            Some(core.tx_id.clone()),
        ));
        let TxCmd::Burst(held) = to_tx.try_recv().expect("production booking hands the repeat to uhd-tx") else { panic!("not a burst") };
        assert_eq!(held.waveform_len, 997);
        assert_eq!(held.open.waveform_len, Some(997));
        assert_eq!(held.samples[0].len(), 997, "booking stores the untiled waveform");
        assert!(held.repeat);
    }

    #[test]
    fn rm_14_the_handed_burst_keeps_its_requested_at() {
        // RM-14 (SC-23a): `BurstOpen.requested_target` is SC-23a's `requested_at` when
        // admission already advanced the target — for a burst on time at its `at` and for one
        // `SendAsap` moves —, never the `at` the burst starts at.
        let samples: Vec<(f32, f32)> = vec![(0.25, 0.0), (0.5, 0.0)];
        let bytes: Vec<u8> = samples.iter().flat_map(|(re, im)| [re.to_le_bytes(), im.to_le_bytes()].concat()).collect();
        let hash = ezsdr_kernel::hash::ContentHash::of_bytes(&bytes);
        let waveform =
            ezsdr_kernel::manifest::ingest_input(ezsdr_kernel::spec::Ident::parse("waveform").unwrap(), ezsdr_kernel::spec::Namespace::parse("ezsdr.input").unwrap(), format!("mem:{hash}"), &bytes);
        let (core, _device, time, _events) = crate::provider::test_support::rig_with_inputs(BTreeMap::from([(hash, Arc::<[u8]>::from(bytes))]));
        let (mut control, to_tx) = control(&core, &[Dir::Tx]);
        let clock = tx_clock(&core);
        time.advance_to(core.at(core.ticks(10_000_000))).unwrap();
        // On time at 20 ms, asked for at 5 ms.
        let requested = TimePoint::new(clock.domain, 5_000);
        control.book(Dispatched::new(
            ActionId(1),
            Action::TxBurst {
                target: super::super::test_support::target_of(&core.tx_id),
                waveform: waveform.clone(),
                repeat: false,
                at: AbsoluteDeadline::new(TimePoint::new(clock.domain, 20_000)),
                requested_at: Some(AbsoluteDeadline::new(requested)),
                late_policy: ezsdr_kernel::stream::LatePolicy::SendAsapAndFlag,
                metadata: BTreeMap::new(),
            },
            Some(core.tx_id.clone()),
        ));
        let TxCmd::Burst(held) = to_tx.try_recv().expect("the receipt site handed the burst to uhd-tx") else { panic!("not a burst") };
        assert_eq!(held.k, 20_000, "an on-time burst starts at its `at`");
        assert_eq!(held.open.requested_target, Some(requested), "the on-time burst's requested target is the `requested_at`");
        // Late: its `at` is behind the booking, asked for at 2 ms; `SendAsap` moves the start.
        let requested = TimePoint::new(clock.domain, 2_000);
        control.book(Dispatched::new(
            ActionId(2),
            Action::TxBurst {
                target: super::super::test_support::target_of(&core.tx_id),
                waveform,
                repeat: false,
                at: AbsoluteDeadline::new(TimePoint::new(clock.domain, 0)),
                requested_at: Some(AbsoluteDeadline::new(requested)),
                late_policy: ezsdr_kernel::stream::LatePolicy::SendAsapAndFlag,
                metadata: BTreeMap::new(),
            },
            Some(core.tx_id.clone()),
        ));
        let TxCmd::Burst(held) = to_tx.try_recv().expect("the receipt site handed the moved burst to uhd-tx") else { panic!("not a burst") };
        assert_eq!(held.k, 12_000, "the moved start is the device lead past the booking");
        assert_eq!(held.open.requested_target, Some(requested), "the moved burst's requested target is its `requested_at`, not its moved start");
    }

    #[test]
    fn ur_21_a_burst_before_start_up_is_refused() {
        // UR-21: a start before the device's start-up — the arm's synchronisation end — is
        // refused at booking. On the controlled clock: the session path read the target from
        // a time the test thread's own delay could pass, and the refusal then came from the
        // late policy instead.
        let samples: Vec<(f32, f32)> = vec![(0.25, 0.0), (0.5, 0.0)];
        let bytes: Vec<u8> = samples.iter().flat_map(|(re, im)| [re.to_le_bytes(), im.to_le_bytes()].concat()).collect();
        let hash = ezsdr_kernel::hash::ContentHash::of_bytes(&bytes);
        let waveform =
            ezsdr_kernel::manifest::ingest_input(ezsdr_kernel::spec::Ident::parse("waveform").unwrap(), ezsdr_kernel::spec::Namespace::parse("ezsdr.input").unwrap(), format!("mem:{hash}"), &bytes);
        let (core, _device, _time, events) = crate::provider::test_support::rig_with_inputs(BTreeMap::from([(hash, Arc::<[u8]>::from(bytes))]));
        let (mut control, to_tx) = control(&core, &[Dir::Tx]);
        core.start_up.store(core.ticks(1_000_000_000), std::sync::atomic::Ordering::Release);
        let clock = tx_clock(&core);
        // 20 ms ahead: past the device lead, so the late policy is not what refuses it, and
        // before the start-up.
        let target = TimePoint::new(clock.domain, 20_000);
        control.book(Dispatched::new(
            ActionId(1),
            Action::TxBurst {
                target: super::super::test_support::target_of(&core.tx_id),
                waveform,
                repeat: false,
                at: AbsoluteDeadline::new(target),
                requested_at: None,
                late_policy: ezsdr_kernel::stream::LatePolicy::DropAndFlag,
                metadata: BTreeMap::new(),
            },
            Some(core.tx_id.clone()),
        ));
        let rejected = lock(&core.rec).rejected.clone();
        assert!(rejected.iter().any(|row| row["reason"].as_str().unwrap_or("").starts_with("UR-21: the burst begins before the device's start-up")), "{rejected:?}");
        assert!(to_tx.try_recv().is_err(), "a refused burst was handed to uhd-tx");
        // The refusal reaches the caller as `COMMAND_REJECTED`, naming the Action it answers.
        let event = events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).into_iter().find(|event| event.kind.as_str() == kinds::COMMAND_REJECTED).expect("COMMAND_REJECTED");
        assert_eq!(event.payload["reason"], "UR-21: the burst begins before the device's start-up", "{event:?}");
        assert_eq!(event.payload["action"], 1, "{event:?}");
    }

    /// uhd-control with a transmit clock at 2 MS/s (100 root ticks a sample) whose origin is `origin` root ticks (RM-25).
    fn control_with_tx_origin(core: &Arc<Core>, origin: i64) -> (Control, Receiver<TxCmd>) {
        let (to_tx, tx) = channel();
        let mut config = core.description.defaults();
        config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        config.insert(key("radio.tx.c0.stream"), Value::from(0));
        let mut start = Start { t0: 0, rx: vec![None; core.description.stream_count(Dir::Rx)], tx: vec![None; core.description.stream_count(Dir::Tx)] };
        start.tx[0] = Some((core.register(Dir::Tx, 100, origin).unwrap(), Channels::first(1)));
        (Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], config, false, start), tx)
    }

    #[test]
    fn ur_21_a_burst_before_its_clock_s_origin_is_late() {
        // RM-14, RM-15, KC-21a: a start before the transmit clock's origin is decided at
        // the device lead before that origin (`timeline::decided_at`), so a late policy moves
        // it onto the origin rather than refusing it for want of a clock. On the controlled
        // clock, the clock's origin is 50 ms ahead (`arm`'s lattice instant lies ahead of the Run's start).
        let samples: Vec<(f32, f32)> = vec![(0.25, 0.0), (0.5, 0.0), (0.75, 0.0), (1.0, 0.0)];
        let bytes: Vec<u8> = samples.iter().flat_map(|(re, im)| [re.to_le_bytes(), im.to_le_bytes()].concat()).collect();
        let hash = ezsdr_kernel::hash::ContentHash::of_bytes(&bytes);
        let waveform =
            ezsdr_kernel::manifest::ingest_input(ezsdr_kernel::spec::Ident::parse("waveform").unwrap(), ezsdr_kernel::spec::Namespace::parse("ezsdr.input").unwrap(), format!("mem:{hash}"), &bytes);
        let (core, _device, _time, events) = crate::provider::test_support::rig_with_inputs(BTreeMap::from([(hash, Arc::<[u8]>::from(bytes))]));
        let (mut control, to_tx) = control_with_tx_origin(&core, core.ticks(50_000_000));
        let plan = plan_of(&core, Dir::Tx);
        assert_eq!(plan.len(), 1, "the clock's segment is planned");
        let clock = tx_clock(&core);
        assert_eq!(plan[0].segment.origin, core.ticks(50_000_000), "the clock's origin");
        // 5 ms before that origin: more than the 2 ms device lead away, so the booking is late
        // and the moved start is the origin (10 000 samples at 2 MS/s).
        let target = TimePoint::new(clock.domain, -10_000);
        control.book(Dispatched::new(
            ActionId(2),
            Action::TxBurst {
                target: super::super::test_support::target_of(&core.tx_id),
                waveform,
                repeat: false,
                at: AbsoluteDeadline::new(target),
                requested_at: None,
                late_policy: ezsdr_kernel::stream::LatePolicy::SendAsapAndFlag,
                metadata: BTreeMap::new(),
            },
            Some(core.tx_id.clone()),
        ));
        let verdict =
            events.drain(ezsdr_kernel::time::TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).into_iter().find(|e| e.kind.as_str() == kinds::TIME_ERROR).expect("the SendAsap verdict at booking");
        assert_eq!(verdict.payload["outcome"], "send_asap", "{verdict:?}");
        assert_eq!(verdict.payload["target"], serde_json::to_value(target).unwrap(), "{verdict:?}");
        let TxCmd::Burst(held) = to_tx.try_recv().expect("the receipt site handed the burst to uhd-tx") else { panic!("not a burst") };
        assert_eq!(held.k, 0, "the moved start is not the clock's origin");
        assert_eq!(clock.instant(held.k), plan[0].segment.origin);
        assert_eq!(lock(&core.held).iter().copied().collect::<Vec<_>>(), vec![(clock.domain, 0)], "the origin is reserved before the hand-over");
    }

    #[test]
    fn ur_21_an_untimed_send_is_on_time_after_its_delivery() {
        // KG-8 (spike K6), on the controlled clock: the Kernel stamps an untimed send at its receipt plus the profile's 5 ms lead
        // (the 2 ms device lead and the 3 ms delivery allowance), and uhd-control books it when it next reads it. Read within
        // the allowance it is on time; read so late that less than the device lead is left it is late, and dropped under
        // `drop_and_flag`. The fake's thread timing cannot move either: the test sets the instants.
        let samples: Vec<(f32, f32)> = vec![(0.25, 0.0), (0.5, 0.0)];
        let bytes: Vec<u8> = samples.iter().flat_map(|(re, im)| [re.to_le_bytes(), im.to_le_bytes()].concat()).collect();
        let hash = ezsdr_kernel::hash::ContentHash::of_bytes(&bytes);
        let waveform =
            ezsdr_kernel::manifest::ingest_input(ezsdr_kernel::spec::Ident::parse("waveform").unwrap(), ezsdr_kernel::spec::Namespace::parse("ezsdr.input").unwrap(), format!("mem:{hash}"), &bytes);
        let (core, _device, time, events) = crate::provider::test_support::rig_with_inputs(BTreeMap::from([(hash, Arc::<[u8]>::from(bytes))]));
        let (mut control, to_tx) = control(&core, &[Dir::Tx]);
        let clock = tx_clock(&core);
        let lead = core.ticks(crate::profile::DEVICE_LEAD_NS + crate::profile::DELIVERY_ALLOWANCE_NS);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        // (milliseconds from receipt to uhd-control's read, whether the burst is late): 1 ms is a poll, 3 ms leaves exactly the device lead.
        for (n, (read_after, late)) in [(1, false), (3, false), (4, true)].into_iter().enumerate() {
            let receipt = ms(100 * (n as i64 + 1));
            time.advance_to(core.at(receipt)).unwrap();
            let target = TimePoint::new(clock.domain, clock.at_or_after(receipt + lead));
            time.advance_to(core.at(receipt + ms(read_after))).unwrap();
            control.book(Dispatched::new(
                ActionId(n as u64 + 1),
                Action::TxBurst {
                    target: super::super::test_support::target_of(&core.tx_id),
                    waveform: waveform.clone(),
                    repeat: false,
                    at: AbsoluteDeadline::new(target),
                    requested_at: None,
                    late_policy: ezsdr_kernel::stream::LatePolicy::DropAndFlag,
                    metadata: BTreeMap::new(),
                },
                Some(core.tx_id.clone()),
            ));
            let errors: Vec<_> = events.drain(ezsdr_kernel::time::TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|e| e.kind.as_str() == kinds::TIME_ERROR).collect();
            assert_eq!(errors.len(), usize::from(late), "read {read_after} ms after receipt: {errors:?}");
            assert_eq!(to_tx.try_recv().is_ok(), !late, "read {read_after} ms after receipt: a late `drop_and_flag` burst is not handed to uhd-tx");
            if late {
                assert_eq!(errors[0].payload["outcome"], "drop", "{errors:?}");
            }
        }
    }

    #[test]
    fn ur_21_an_untimed_burst_before_the_clock_s_origin_is_on_time_at_the_origin() {
        // RS-19, on the controlled clock: with no `at`, a burst before the transmit clock's origin (50 ms ahead) is stamped at that
        // origin. uhd-control reads it a poll later and it is on time at the origin; read 49 ms in, with less than the 2 ms
        // device lead left, it is late.
        let samples: Vec<(f32, f32)> = vec![(0.25, 0.0), (0.5, 0.0)];
        let bytes: Vec<u8> = samples.iter().flat_map(|(re, im)| [re.to_le_bytes(), im.to_le_bytes()].concat()).collect();
        let hash = ezsdr_kernel::hash::ContentHash::of_bytes(&bytes);
        let waveform =
            ezsdr_kernel::manifest::ingest_input(ezsdr_kernel::spec::Ident::parse("waveform").unwrap(), ezsdr_kernel::spec::Namespace::parse("ezsdr.input").unwrap(), format!("mem:{hash}"), &bytes);
        for (read_after_ms, late) in [(1, false), (49, true)] {
            let (core, _device, time, events) = crate::provider::test_support::rig_with_inputs(BTreeMap::from([(hash.clone(), Arc::<[u8]>::from(bytes.clone()))]));
            let (mut control, to_tx) = control_with_tx_origin(&core, core.ticks(50_000_000));
            let plan = plan_of(&core, Dir::Tx);
            let clock = tx_clock(&core);
            assert_eq!(plan[0].segment.origin, core.ticks(50_000_000), "the clock's origin");
            time.advance_to(core.at(core.ticks(read_after_ms * 1_000_000))).unwrap();
            events.drain(ezsdr_kernel::time::TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0));
            control.book(Dispatched::new(
                ActionId(2),
                Action::TxBurst {
                    target: super::super::test_support::target_of(&core.tx_id),
                    waveform: waveform.clone(),
                    repeat: false,
                    at: AbsoluteDeadline::new(TimePoint::new(clock.domain, 0)),
                    requested_at: None,
                    late_policy: ezsdr_kernel::stream::LatePolicy::DropAndFlag,
                    metadata: BTreeMap::new(),
                },
                Some(core.tx_id.clone()),
            ));
            let errors = events.drain(ezsdr_kernel::time::TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|e| e.kind.as_str() == kinds::TIME_ERROR).count();
            assert_eq!(errors, usize::from(late), "read {read_after_ms} ms after the change");
            match to_tx.try_recv() {
                Ok(TxCmd::Burst(held)) => assert!(!late && held.k == 0, "the burst starts at the origin: {}", held.k),
                _ => assert!(late, "an on-time burst is handed to uhd-tx"),
            }
        }
    }

    #[test]
    fn ur_26_an_orderly_stop_cuts_at_its_instant() {
        // RM-16, UR-26 (Review M, N-6): `Provider::stop`'s cut is booked at the stop instant:
        // under `orderly` the first sample at or after it, or, once uhd-rx has delivered past
        // it, the first sample it has not delivered.
        for (delivered, cut) in [(None, 5_000), (Some(6_000), 6_000)] {
            let (core, _, time) = rig();
            let (_control, _tx) = control(&core, &[Dir::Rx]);
            time.advance_to(core.at(core.ticks(8_000_000))).unwrap();
            let config = plan_of(&core, Dir::Rx)[0].segment.config;
            if let Some(k) = delivered {
                let mut streams = lock(&core.streams);
                streams.started(booked_stream(Dir::Rx), 0, config);
                streams.delivered(booked_stream(Dir::Rx), 0, config, k);
            }
            lock(&core.streams).end(&core, core.ticks(5_000_000), false, false);
            assert_eq!(plan_of(&core, Dir::Rx)[0].segment.cut, Some(cut));
        }
    }

    #[test]
    fn ur_26_a_stop_ends_every_line_once() {
        // RM-16, UR-26 (design 33 D15): a loss or `Provider::stop` books one `End` on every
        // stream's line, each with the same instant and arrival order, and a second end
        // books nothing: `ended` is the instance's, not a stream's.
        let (core, _, _) = rig();
        let (_control, _tx) = control(&core, &[Dir::Rx, Dir::Tx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let (rx, tx) = (booked_stream(Dir::Rx), booked_stream(Dir::Tx));
        lock(&core.streams).end(&core, ms(5), false, false);
        let seq = lock(&core.streams).seq;
        lock(&core.streams).end(&core, ms(7), true, false);
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
        assert_eq!(ends(rx), [(ms(5), seq, false)], "the receive line");
        assert_eq!(ends(tx), [(ms(5), seq, false)], "the transmit line");
        assert_eq!(lock(&core.streams).seq, seq, "the second end took a new arrival order");
    }

    #[test]
    fn ur_25_a_start_rx_takes_the_line_lock_once_for_its_instant_and_its_booking() {
        // RM-12, #63 (O1): start_rx's instant, its arrival order and its booking are one critical
        // section of the line lock, including its configuration snapshot.
        let selector = BTreeMap::from([
            (ezsdr_kernel::spec::Ident::parse("rx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:B:ubx:0")])),
            (ezsdr_kernel::spec::Ident::parse("tx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0")])),
        ]);
        let layout = ezsdr_radio::channels::from_selector(&selector).unwrap();
        let description = ezsdr_radio::channels::Description::new(&layout, 2_000, &crate::profile::terms());
        let (core, _time, _) =
            crate::provider::test_support::rig_on_description(Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig::default())), vec![Arc::new(NoLink)], BTreeMap::new(), description);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let streams = std::ptr::from_ref(&core.streams) as usize;
        let instant = core.ticks(300_000_000);
        let at = Some(AbsoluteDeadline::new(core.at(instant)));
        let node = core.stream_node(Dir::Rx, 0);
        crate::provider::test_support::record_locks();
        control.book(Dispatched::new(
            ActionId(1),
            Action::Command { target: super::super::test_support::target_of(&node), verb: ezsdr_kernel::spec::Ident::parse(ezsdr_radio::START_RX).unwrap(), params: BTreeMap::new(), at },
            Some(node),
        ));
        let taken = crate::provider::test_support::taken_locks();
        assert_eq!(taken.iter().filter(|address| **address == streams).count(), 1, "an accepted start_rx took the line lock {taken:?}");
        let start = lock(&core.streams).line(booked_stream(Dir::Rx)).unwrap().items.iter().find(|item| matches!(item.kind, Kind::Start)).map(|item| item.e);
        assert_eq!(start, Some(instant), "start_rx ignored its requested instant");
    }

    // ------------------------------------------------------------ 1b-4d-1b: ownership, loss and stop

    /// The manual-clock `LockSpy` rig the ownership series runs on: the device's calls are
    /// gated and counted, and `links` gives the receive side a stream.
    fn spy_rig(links: bool) -> (Arc<Core>, Arc<crate::provider::test_support::LockSpy>, Arc<ezsdr_kernel::time::ManualTimeAuthority>) {
        use crate::ClockMode;
        let spy = Arc::new(crate::provider::test_support::LockSpy::new(crate::device::FakeConfig { clock: ClockMode::Manual, ..crate::device::FakeConfig::default() }));
        let links: Vec<Arc<dyn DataLink>> = if links { vec![Arc::new(NoLink)] } else { Vec::new() };
        let (core, time, _) = crate::provider::test_support::rig_on(spy.clone(), links, 2_000);
        (core, spy, time)
    }

    #[test]
    fn ur_25_a_device_call_passes_while_another_thread_holds_the_line() {
        // UR-25 (S13's control): the line's check is the caller's own guard, not a lock the
        // spy cannot take — another thread holding the line while this one calls the device
        // is no violation.
        let (core, spy, _time) = spy_rig(false);
        let (ready, held) = channel();
        let (release, done) = channel();
        let core_ = core.clone();
        let holder = std::thread::spawn(move || {
            let _line = lock(&core_.streams);
            let _ = ready.send(());
            let _ = done.recv();
        });
        held.recv_timeout(Wall::from_secs(10)).expect("the other thread holds the line");
        core.device.rx_open(0, &[0]).expect("a device call while another thread holds the line");
        release.send(()).unwrap();
        holder.join().unwrap();
        assert_eq!(spy.seen("rx_open"), 1);
    }

    #[test]
    fn ur_25_an_open_is_recorded_only_after_the_device_returns() {
        // UR-25 (S13b): an open writes the stream's record only after the device returned, so
        // the call itself never sees the set it is opening.
        let (core, spy, _time) = spy_rig(true);
        let during = Arc::new(std::sync::Mutex::new(Vec::new()));
        {
            let during = during.clone();
            let weak = Arc::downgrade(&core);
            crate::provider::test_support::probe(&core, "open", move |_| {
                if let Some(core) = weak.upgrade() {
                    during.lock().unwrap().push(lock(&core.streams).stream(booked_stream(Dir::Rx)).set);
                }
            });
        }
        let (_control, _tx) = control(&core, &[]);
        lock(&core.streams).record_open(booked_stream(Dir::Rx), None);
        core.open_streamer(Dir::Rx, 0, Channels::first(1)).expect("the owner's open");
        let seen = during.lock().unwrap().clone();
        assert_eq!(seen, [super::super::core::DeviceSet::Unknown], "an open saw the set it was opening");
        assert_eq!(lock(&core.streams).stream(booked_stream(Dir::Rx)).set, super::super::core::DeviceSet::Open(Channels::first(1)), "the open is recorded after its return");
        assert!(spy.seen("rx_open") >= 1, "the spy saw no open");
    }

    /// The events of `kind` since the last drain.
    fn events_of(events: &Arc<ezsdr_kernel::event::EventCollector>, kind: &str) -> Vec<ezsdr_kernel::event::Event> {
        events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == kind).collect()
    }

    #[test]
    fn ur_26_d14_a_layout_instance_names_its_streams() {
        // RM-12, RM-29, D14 (design note 37): under the layout the instance's node and its
        // streams' nodes (`<id>/rx/s<k>`) are the targets, and a stream node's `Stop` stops that
        // stream alone: a transmit burst held ahead of its instant is not cancelled, and the
        // transmit clock is untouched. The held burst's own play through its instant is
        // `ur_23_a_burst_booked_at_a_sent_burst_s_end_continues_it`'s rig.
        let samples: Vec<(f32, f32)> = (0..1_000).map(|i| (i as f32 / 1_000.0, 0.0)).collect();
        let bytes: Vec<u8> = samples.iter().flat_map(|(re, im)| [re.to_le_bytes(), im.to_le_bytes()].concat()).collect();
        let hash = ezsdr_kernel::hash::ContentHash::of_bytes(&bytes);
        let waveform =
            ezsdr_kernel::manifest::ingest_input(ezsdr_kernel::spec::Ident::parse("waveform").unwrap(), ezsdr_kernel::spec::Namespace::parse("ezsdr.input").unwrap(), format!("mem:{hash}"), &bytes);
        let device = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..crate::device::FakeConfig::default() }));
        let links = vec![vec![Arc::new(NoLink) as Arc<dyn DataLink>]];
        let (core, time, events) = crate::provider::test_support::rig_on_description_links(device.clone(), links, BTreeMap::from([(hash, Arc::<[u8]>::from(bytes))]), layout_description());
        let _frozen = hold(&core, &time, &device);
        let (mut control, to_tx) = control(&core, &[Dir::Rx, Dir::Tx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let clock = tx_clock(&core);
        let k = (ms(30) - clock.origin) / clock.n;
        assert!(clock.instant(k) >= ms(30), "the burst is booked ahead of its instant");
        control.book(Dispatched::new(
            ActionId(1),
            Action::TxBurst {
                target: super::super::test_support::target_of(&ResourceId::parse("usrp/tx/s0").unwrap()),
                waveform,
                repeat: false,
                at: AbsoluteDeadline::new(TimePoint::new(clock.domain, k)),
                requested_at: None,
                late_policy: ezsdr_kernel::stream::LatePolicy::SendAsapAndFlag,
                metadata: BTreeMap::new(),
            },
            Some(ResourceId::parse("usrp/tx/s0").unwrap()),
        ));
        assert!(matches!(to_tx.try_recv(), Ok(TxCmd::Burst(_))), "the burst was not handed to uhd-tx");
        let held = lock(&core.held).len();
        assert_eq!(held, 1, "the burst is not held ahead of its instant");
        control.book(Dispatched::new(
            ActionId(2),
            Action::Stop { target: Some(super::super::test_support::target_of(&ResourceId::parse("usrp/rx/s0").unwrap())) },
            Some(ResourceId::parse("usrp/rx/s0").unwrap()),
        ));
        assert!(to_tx.try_recv().is_err(), "the receive stream's Stop reached uhd-tx");
        assert_eq!(lock(&core.held).len(), held, "the receive stream's Stop cancelled the transmit burst");
        assert_eq!(
            lock(&core.streams).line(stream_key(Dir::Tx, 0)).map_or(0, |line| line.items.iter().filter(|item| item.kind == Kind::Stop).count()),
            0,
            "the receive Stop was booked on the transmit line"
        );
        assert_eq!(lock(&core.streams).line(stream_key(Dir::Rx, 0)).map_or(0, |line| line.items.iter().filter(|item| item.kind == Kind::Stop).count()), 1, "the receive stream is not stopped");
        assert!(events_of(&events, kinds::COMMAND_REJECTED).is_empty(), "{:?}", device.calls());
    }

    /// Hands out its Actions in order, and notes each `recv` that comes before the previous
    /// Action's refusal is on record.
    struct InOrder {
        queue: std::sync::Mutex<std::collections::VecDeque<Action>>,
        handed: std::sync::atomic::AtomicU64,
        core: Arc<Core>,
        early: std::sync::Mutex<Vec<u64>>,
    }
    impl ActionReceiver for InOrder {
        fn recv(&self) -> Option<Dispatched> {
            let last = self.handed.load(std::sync::atomic::Ordering::SeqCst);
            if last > 0 && (lock(&self.core.rec).rejected.len() as u64) < last {
                self.early.lock().unwrap().push(last);
            }
            let action = self.queue.lock().unwrap().pop_front()?;
            let id = self.handed.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            Some(Dispatched::new(ActionId(id), action, None))
        }
    }

    #[test]
    fn ur_14_each_action_is_booked_before_the_next_recv() {
        // MA-14b, KC-21a: a submit returns once its Action is booked, with its refusal on
        // record, so the loop takes the next Action only after booking the one it holds.
        let (core, _device, _time) = rig();
        let timer = || Action::SetTimer { target: crate::provider::test_support::target_of(&core.id), at: AbsoluteDeadline::new(TimePoint::new(core.root, 1)), token: 1 };
        let receiver = Arc::new(InOrder {
            queue: std::sync::Mutex::new([timer(), timer()].into_iter().collect()),
            handed: std::sync::atomic::AtomicU64::new(0),
            core: core.clone(),
            early: std::sync::Mutex::new(Vec::new()),
        });
        let (to_tx, _tx) = channel();
        let start = Start { t0: 0, rx: vec![None; core.description.stream_count(Dir::Rx)], tx: vec![None; core.description.stream_count(Dir::Tx)] };
        let control = Control::new(core.clone(), receiver.clone(), vec![to_tx], core.description.defaults(), false, start);
        let handle = std::thread::spawn(move || control.run());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while lock(&core.rec).rejected.len() < 2 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        lock(&core.streams).stop = Some((core.now(), ezsdr_kernel::module_api::StopMode::Orderly));
        handle.join().unwrap();
        assert_eq!(lock(&core.rec).rejected.len(), 2, "both Actions were booked");
        assert!(receiver.early.lock().unwrap().is_empty(), "a recv came before the previous Action was booked: {:?}", receiver.early.lock().unwrap());
    }
}
