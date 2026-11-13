//! uhd-control (UR-14): drains the `ActionReceiver` with MA-14b's loop, books each
//! Action — a stream's commands on its timeline, which it shares with the stream's owner —
//! and every 500 ms checks the reference and reads the device's time. It never waits for a
//! device instant.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use std::time::Duration as Wall;

use ezsdr_kernel::event::{Action, ActionId, Severity};
use ezsdr_kernel::id::{ClockDomainId, ResourceId};
use ezsdr_kernel::module_api::{ActionReceiver, Dispatched};
use ezsdr_kernel::spec::{Key, Value};
use ezsdr_kernel::stream::{BurstOpen, Direction, LateOutcome};
use ezsdr_kernel::time::{AbsoluteDeadline, Duration, Rational, TimeError, TimePoint};
use ezsdr_radio::kinds;
use ezsdr_radio::payloads::{ClockLostPayload, ClockReference, TimeErrorCause, TimeErrorOutcome, TimeErrorPayload};
use ezsdr_radio::staging::{Bookings, Overlap, Staging, SwitchRefusal, handover_horizon, ordinary_k};
use ezsdr_radio::timeline::{self, Channels, Config, Item, Kind, Line, Segment, StreamKey};
use serde_json::json;

#[cfg(test)]
use super::core::booked_stream;
use super::core::{Clock, Core, lock, stream_key};
use super::mode::DescriptionExt;
use super::tx::Held;
use crate::device::{DeviceError, Dir, Iq, Settings, decimation};
use crate::host::HostClock;
use crate::profile::{DELIVERY_ALLOWANCE_NS, DEVICE_LEAD_NS};
use ezsdr_radio::Addressed;

const POLL: Wall = Wall::from_millis(1);
const CHECK: Wall = Wall::from_millis(500);

/// The configuration a segment begins with, shared with its owner: the one in effect at the
/// segment's origin (UR-12, UR-17), its rate and channel set the segment's. It is the
/// configuration `start_rx` snapshots at its booking (RM-21, UR-26).
pub(crate) struct ColdConfig {
    base: BTreeMap<Key, Value>,
    /// The Action that took this snapshot, which a refusal of its configuration names.
    action: Option<ActionId>,
    /// The transmit stream's generation the snapshot carries (RM-21); none for a receive start.
    generation: Option<u64>,
}

impl ColdConfig {
    #[cfg(test)]
    pub(crate) fn new(base: BTreeMap<Key, Value>) -> Self {
        Self { base, action: None, generation: None }
    }

    /// The snapshot of the `start_rx` `id`.
    pub(crate) fn of(base: BTreeMap<Key, Value>, id: ActionId) -> Self {
        Self { base, action: Some(id), generation: None }
    }

    /// The snapshot of the transmit burst `id` at the stream's `generation` (RM-21).
    pub(crate) fn of_tx(base: BTreeMap<Key, Value>, id: ActionId, generation: u64) -> Self {
        Self { base, action: Some(id), generation: Some(generation) }
    }

    pub(crate) fn action(&self) -> Option<ActionId> {
        self.action
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

    /// Configures transmit stream `k`'s `channels` to this snapshot's values, untimed (R4). The rate
    /// is the Run's, so it is not set; `rate` is what the read-back is checked against.
    pub fn configure_tx(&self, core: &Core, channels: Channels, k: usize, rate: f64) -> Result<(), DeviceError> {
        let channels: Vec<(usize, Settings)> = channels
            .iter()
            .map(|chan| {
                let mut settings = core.description.settings(&self.base, Dir::Tx, k, chan as usize);
                settings.rate = None;
                (chan as usize, settings)
            })
            .collect();
        core.configure(Dir::Tx, &channels, rate, k).map(|_| ())
    }

    pub(crate) fn generation(&self) -> Option<u64> {
        self.generation
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
    /// The receive values of record: the Run's configuration and the values a stopped stream accepted,
    /// which a `start_rx` snapshots at its booking (RM-21, UR-26).
    pub staging: Staging,
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
    pub fn new(core: Arc<Core>, actions: Arc<dyn ActionReceiver>, config: BTreeMap<Key, Value>, reference_monitored: bool, start: Start) -> Control {
        let host = core.device.host();
        // RM-25, VH-4: this Module's terms — each direction's start lead, and, for a receive
        // segment after a cut, the receive call in progress there and the delivery
        // allowance (UR-25). The call term is each receive stream's own packet size.
        let allowance = core.ticks(DELIVERY_ALLOWANCE_NS);
        let line = |key: StreamKey, origin: i64, channels: Channels, n: i64, call: i64| {
            Line::new(timeline::Stream {
                direction: key.0,
                origin,
                config: Config { channels, ratio: Rational::new(n.max(1) as u64, 1).expect("a positive ratio") },
                lead: match key.0 {
                    Direction::Rx => core.ticks(ezsdr_radio::timeline::start_lead_ns(&core.description.timing)),
                    Direction::Tx => 0,
                },
                call,
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
                let clock = start.tx.get(k).copied().flatten().map(|(clock, _)| clock);
                let stream = streams.stream_mut(stream_key(Dir::Tx, k));
                stream.clock = clock;
                if let Some(clock) = clock {
                    stream.arm_record(core.mcr, clock);
                }
                streams.lines.insert(stream_key(Dir::Tx, k), line.clone());
            }
        }
        Control { core, actions, staging: Staging::new(config), reference_monitored, clock_lost: false, failing_since: None, host }
    }

    pub fn run(mut self) {
        let mut last_check = self.host.now();
        loop {
            if lock(&self.core.streams).stop.is_some() {
                return;
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

    /// `node` is what the Kernel resolved the target to (KC-23), which is what a
    /// Provider matches on.
    pub(super) fn book(&mut self, dispatched: Dispatched) {
        let id = dispatched.id;
        let node = dispatched.node.as_ref();
        let addressed = self.addressed(node);
        let untimed = dispatched.untimed;
        match dispatched.action {
            Action::TxBurst { waveform, repeat, at, requested_at, late_policy, metadata, .. } => {
                self.book_burst(id, addressed, waveform, repeat, at, requested_at, late_policy, &metadata, untimed);
            }
            // RM-21, UR-26: a receive channel's frequency, gain or antenna is staged for the next `start_rx`
            // of its stream when the stream is not streaming, with no device call, and refused with the
            // cause otherwise. The Kernel refuses a classless key first; its refusal here is the Provider's
            // own guard for a harness that calls it directly.
            Action::UpdateParameter { key, value, at, .. } => {
                // RM-12: the node selects what the update may reach: the instance and its streams.
                if matches!(addressed, Addressed::Other) {
                    return self.core.command_rejected(Some(id), "update_parameter", "UR-26: the target is not this device");
                }
                let streams = lock(&self.core.streams);
                let receipt = self.core.now();
                let staged = self.staging.set(
                    &key,
                    value,
                    at.is_some(),
                    receipt,
                    |k| streams.line(stream_key(Dir::Rx, k as usize)).is_some_and(Line::streaming),
                    |k| streams.tx.get(k as usize).map_or(Bookings::unarmed(), |stream| stream.book.record.clone()),
                );
                drop(streams);
                if let Err(refusal) = staged {
                    self.core.command_rejected(Some(id), "update_parameter", &format!("UR-26: {}", refusal.reason()));
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
                    // RM-16: a Stop with no bound that fits the stream's clock is refused with its id, nothing changed; the other streams go on.
                    let window = self.core.tx_in_flight();
                    if streams.stream_mut(stream).clock.is_some_and(|clock| !ezsdr_radio::staging::stop_fits(now, window, |t| clock.try_at_or_after(t), |k| clock.try_instant(k))) {
                        drop(streams);
                        self.core.command_rejected(Some(id), "stop", "UR-26: the Stop's bound overflows the transmit clock");
                        continue;
                    }
                    let seq = streams.next_seq();
                    if streams.book(&self.core, stream, Item { e: now, seq, ready: now, delivered: None, refused: false, kind: Kind::Stop }, Some(id)) {
                        if let Some(bound) = streams.stream_mut(stream).stop_at(now, self.core.tx_in_flight(), false) {
                            self.core.timing(json!({ "what": "tx_stop_applied", "stream": k, "at": now, "bound": bound }));
                        }
                    }
                    // The Stop reaches uhd-tx whether or not the line booked it, in receipt order with the bookings (UR-26).
                    streams.stream_mut(stream).book.list.halt(ezsdr_radio::CancelReason::Stop);
                    self.core.release_streams(streams);
                }
                if !rx.is_empty() {
                    let mut streams = lock(&self.core.streams);
                    let now = self.core.now();
                    for k in rx {
                        let stream = stream_key(Dir::Rx, k);
                        let seq = streams.next_seq();
                        streams.book(&self.core, stream, Item { e: now, seq, ready: now, delivered: None, refused: false, kind: Kind::Stop }, Some(id));
                    }
                    self.core.release_streams(streams);
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
                // RM-21: a stream that is streaming (a booked start not yet begun counts) takes no start; the line is unchanged.
                let snapshot = match self.staging.start(k as u32, streams.line(stream).is_some_and(Line::streaming)) {
                    Ok(snapshot) => snapshot,
                    Err(refusal) => {
                        drop(streams);
                        return self.core.command_rejected(Some(id), "command", &format!("UR-26: {}", refusal.reason()));
                    }
                };
                let (seq, now) = (streams.next_seq(), self.core.now());
                let e = timeline::command_instant(streams.line(stream), requested, now);
                streams.stream_mut(stream).configs.insert(seq, Arc::new(ColdConfig::of(snapshot, id)));
                streams.book(&self.core, stream, Item { e, seq, ready: now, delivered: None, refused: false, kind: Kind::Start }, Some(id));
                self.core.release_streams(streams);
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
        metadata: &BTreeMap<Key, Value>,
        untimed: bool,
    ) {
        let reject = |reason: &str| self.core.command_rejected(Some(id), "tx_burst", reason);
        // RM-37: a repeat with the switch mark replaces the repeat playing on its stream; the mark on anything else is refused.
        let switch = ezsdr_radio::staging::is_switch(repeat, late_policy, metadata);
        // RM-12, RM-29: the target must be a transmit stream's node.
        let k = match addressed {
            Addressed::Stream { direction: Direction::Tx, k } => k,
            _ => return reject("UR-21: the target is not this device's transmit stream"),
        };
        // KC-21a: the transmit clock registered at `arm`, while its segment has not ended.
        let current = {
            let streams = lock(&self.core.streams);
            let stream = stream_key(Dir::Tx, k);
            let last = streams.line(stream).and_then(|line| line.plan.last()).filter(|segment| segment.cut.is_none());
            streams.stream(stream).clock.zip(last.map(|segment| segment.config.channels.count() as usize))
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
        if !(metadata.is_empty() || switch) {
            return reject("UR-21: TxBurst.metadata must be empty or the repeat-switch mark");
        }
        let (generation, values) = self.staging.tx_snapshot(k as u32);
        let settings = Arc::new(ColdConfig::of_tx(values, id, generation));
        self.core.probe("tx_book_cut_checked", None);
        // One critical section from the instant read to the booking on the list: what a later Stop or `Provider::stop` finds is exactly what was accepted before it (UR-21, UR-26).
        let time_error = |outcome: TimeErrorOutcome, late_by: Duration| {
            let payload = serde_json::to_value(TimeErrorPayload {
                cause: TimeErrorCause::Late,
                outcome,
                late_by_ns: late_by.ticks_in(ClockDomainId::HOST_MONOTONIC).unwrap_or(i64::MAX),
                target: at.time_point,
                action: None,
            })
            .expect("a payload");
            // The burst's own stream (RM-29).
            self.core.emit(&self.core.stream_node(Dir::Tx, k), kinds::TIME_ERROR, Severity::Error, payload);
        };
        // An early refusal or drop is decided inside the section and emitted after the guard is released (an emit may block on the collector).
        let mut send_asap = None;
        let early: Result<Option<&str>, Early> = 'section: {
            let mut streams = lock(&self.core.streams);
            // The cut was checked above in another section: a `Provider::stop` cut may have come in between, and a booking after a cut is refused.
            if streams.line(stream_key(Dir::Tx, k)).and_then(|line| line.plan.last()).is_none_or(|segment| segment.cut.is_some()) {
                break 'section Err(Early::Reject("UR-21: no transmit channel is configured".into()));
            }
            // RM-14 on the device lead, RM-15's origin bound (VE-2; N-P0-1).
            let lead = self.core.device_lead();
            let receipt = self.core.now();
            let now = timeline::decided_at(receipt, clock.origin, lead);
            let now_k = clock.at_or_after(now);
            // RM-37: a switch's boundary is not judged late at receipt: it is the first of its grid the device can still take (UR-38).
            // RM-15: an untimed burst's late policy is not consulted; it is placed (below).
            let decided = if switch || untimed {
                Ok(LateOutcome::OnTime {})
            } else {
                late_policy.decide(&self.core.clocks, at.time_point, TimePoint::new(clock.domain, now_k), Duration::new(ClockDomainId::HOST_MONOTONIC, DEVICE_LEAD_NS))
            };
            let mut outcome = match decided {
                Ok(outcome) => outcome,
                Err(error) => break 'section Err(Early::Reject(format!("UR-21: {error}"))),
            };
            let mut start = match at.time_point.ticks_in(clock.domain) {
                Ok(k) => k,
                Err(error) => break 'section Err(Early::Reject(format!("UR-21: {error}"))),
            };
            let mut requested = requested_at.map(|d| d.time_point);
            match outcome {
                LateOutcome::OnTime {} => {}
                LateOutcome::SendAsap { late_by } => {
                    requested = requested.or(Some(at.time_point));
                    send_asap = Some(late_by);
                }
                LateOutcome::Drop { late_by } => {
                    break 'section Err(Early::Dropped(TimeErrorOutcome::Drop, late_by, "UR-21: the late policy dropped the burst"));
                }
                LateOutcome::PlanViolation { late_by } => {
                    break 'section Err(Early::Dropped(TimeErrorOutcome::PlanViolation, late_by, "UR-21: the planned burst arrived late"));
                }
            }
            let join_by = |e: i64| self.core.join_by(clock, e);
            let ordinary = ordinary_k(now, lead, |t| clock.at_or_after(t), |k| clock.instant(k));
            let window = self.core.ticks(self.core.description.timing.tx_in_flight_ns);
            let horizon = handover_horizon(receipt, window, |t| clock.at_or_after(t));
            let refusal = {
                let stream = streams.stream_mut(stream_key(Dir::Tx, k));
                let playing = stream.book.playing;
                let at_k = start;
                stream.book.record.prune(clock.at_or_after(receipt));
                let rec_len = (!repeat).then_some(len as i64);
                let mut unplaced = false;
                if untimed && !switch {
                    match stream.book.record.place_untimed(start, ordinary, rec_len, receipt, join_by, horizon) {
                        Some(free) => start = free,
                        None => unplaced = true,
                    }
                } else if send_asap.is_some() {
                    match stream.book.record.place(ordinary, rec_len, receipt, join_by, horizon) {
                        Some(free) => start = free,
                        None => unplaced = true,
                    }
                } else if late_policy == ezsdr_kernel::stream::LatePolicy::SendAsapAndFlag && !switch && matches!(outcome, LateOutcome::OnTime {}) {
                    // RM-16 (F36): a timed SendAsap burst that meets only the Stop region moves past it, with TIME_ERROR{send_asap}.
                    match stream.book.record.region_move(start, rec_len, receipt, join_by, horizon) {
                        Ok(moved) if moved != start => {
                            let late_by = match self.core.late_by(clock, moved - start) {
                                Ok(late_by) => late_by,
                                Err(error) => break 'section Err(Early::Reject(format!("UR-21: {error}"))),
                            };
                            start = moved;
                            outcome = LateOutcome::SendAsap { late_by };
                            send_asap = Some(late_by);
                            requested = requested.or(Some(at.time_point));
                        }
                        Ok(_) => {}
                        Err(_) => unplaced = true,
                    }
                }
                let switch_bound = clock.at_or_after(receipt.saturating_add(window).saturating_add(lead));
                // A switch's boundary previewed for the overflow and start-up checks, which refuse before the book changes; the booking takes
                // the one `receive_switch` returns.
                if switch {
                    start = stream.book.switch_boundary(start, switch_bound).unwrap_or(start);
                }
                // The instants are computed before anything is booked: one that does not fit is a refusal, not a booked chain.
                let end = (!repeat).then(|| clock.try_instant(start.saturating_add(len as i64)));
                let overflow = "UR-21: the burst time overflows the transmit clock";
                let refusal = if clock.try_instant(start).is_none() || end.is_some_and(|end| end.is_none()) {
                    Some(overflow)
                } else if clock.instant(start) < self.core.start_up.load(Ordering::Acquire) {
                    Some("UR-21: the burst begins before the device's start-up")
                } else if switch {
                    match stream.book.receive_switch(id, len as i64, at_k, switch_bound, untimed, clock.at_or_after(receipt)) {
                        Ok(accepted) => {
                            debug_assert_eq!(start, accepted.boundary, "the previewed switch boundary is the book's");
                            start = accepted.boundary;
                            requested = requested.or(accepted.requested_target.map(|_| at.time_point));
                            None
                        }
                        Err(SwitchRefusal::NotStarted) => Some("UR-38: the repeat the switch replaces has not started on the device"),
                        Err(SwitchRefusal::NoPlayingRepeat | SwitchRefusal::PendingSwitch) => Some("UR-21: a repeat switch needs a playing repeat and no pending switch"),
                        Err(SwitchRefusal::TimeOverflow) => Some(overflow),
                    }
                } else if unplaced {
                    Some("UR-21: the burst overlaps an accepted booking")
                } else {
                    stream.book.record.accept(start, (!repeat).then_some(len as i64), receipt, join_by, horizon).err().map(|overlap| match overlap {
                        Overlap::Booking => "UR-21: the burst overlaps an accepted booking",
                        Overlap::Handed => "UR-21: the burst starts before a booking that may already be handed",
                    })
                };
                if refusal.is_none() {
                    // The row exists before uhd-tx can read the switch.
                    if let (true, Some(from)) = (switch, playing) {
                        let row = ezsdr_radio::staging::SwitchRecord {
                            action_id: id,
                            predecessor: from.id,
                            selected_at: self.core.at(receipt),
                            old_period: from.period,
                            new_period: len as i64,
                            boundary: start,
                            outcome: "pending",
                        };
                        lock(&self.core.rec).switches.push(serde_json::to_value(row).expect("a switch row"));
                    }
                    let open = BurstOpen { waveform_len: u32::try_from(len).ok(), late: (!matches!(outcome, LateOutcome::OnTime {})).then_some(outcome), requested_target: requested };
                    let held =
                        Held { k: start, domain: clock.domain, samples: Arc::new(samples), waveform_len: len, repeat, switch, untimed, open, policy: late_policy, target: at.time_point, settings };
                    stream.book.play(repeat, id, start, len as i64);
                    stream.book.book(held, end.flatten());
                }
                refusal
            };
            drop(streams);
            Ok(refusal)
        };
        let refusal = match early {
            Ok(refusal) => refusal,
            Err(Early::Reject(reason)) => return reject(&reason),
            Err(Early::Dropped(outcome, late_by, reason)) => {
                time_error(outcome, late_by);
                return self.core.reject_note(json!({ "kind": "tx_burst", "reason": reason }));
            }
        };
        if let Some(reason) = refusal {
            if let Some(late_by) = send_asap {
                time_error(TimeErrorOutcome::Refused, late_by);
            }
            return reject(reason);
        }
        if let Some(late_by) = send_asap {
            time_error(TimeErrorOutcome::SendAsap, late_by);
        }
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

/// An early exit of `book_burst`, decided under the `streams` guard and emitted after it is released.
enum Early {
    Reject(String),
    Dropped(TimeErrorOutcome, Duration, &'static str),
}

#[cfg(test)]
mod differential;

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;

    use super::super::core::key;
    use super::*;
    use crate::provider::test_support::device_at;
    use ezsdr_kernel::spec::Scalar;
    use ezsdr_kernel::stream::{BackPressure, BlockRef, DataLink, DropCarry, PublishOutcome};
    use ezsdr_radio::staging::{Read, Refusal, Until};

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
    /// The booking uhd-control put on transmit stream 0's list, if the next receipt is one (UR-21).
    fn next_booking(core: &Core) -> Option<Held> {
        match lock(&core.streams).stream_mut(stream_key(Dir::Tx, 0)).book.list.read() {
            Some(Read::Booking(held)) => Some(held),
            _ => None,
        }
    }

    /// Every booking left on transmit stream 0's list, Stops skipped, in receipt order.
    fn booked(core: &Core) -> Vec<Held> {
        let mut streams = lock(&core.streams);
        std::iter::from_fn(|| streams.stream_mut(stream_key(Dir::Tx, 0)).book.list.read()).filter_map(|read| if let Read::Booking(held) = read { Some(held) } else { None }).collect()
    }

    /// Whether the next receipt on transmit stream 0's list is a Stop (UR-26).
    fn stop_halted(core: &Core) -> bool {
        matches!(lock(&core.streams).stream_mut(stream_key(Dir::Tx, 0)).book.list.read(), Some(Read::Halt { reason: ezsdr_radio::CancelReason::Stop, .. }))
    }

    fn control(core: &Arc<Core>, on: &[Dir]) -> Control {
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
        Control::new(core.clone(), Arc::new(NoActions), config, false, start)
    }

    /// The plan of `dir`'s stream as its owner reads it.
    fn plan_of(core: &Core, dir: Dir) -> Vec<Planned> {
        lock(&core.streams).planned(booked_stream(dir))
    }

    /// Transmit stream 0's clock, registered at `arm` (RM-25).
    fn tx_clock(core: &Core) -> Clock {
        lock(&core.streams).stream(booked_stream(Dir::Tx)).clock.expect("the transmit clock is registered at arm")
    }

    /// The device's clock held still from its epoch, tick 0, asserted: every instant a test sets
    /// later is past it (`clock_guard`).
    fn hold(core: &Core, time: &ezsdr_kernel::time::ManualTimeAuthority, device: &crate::device::FakeDevice) -> crate::host::Driver {
        let guard = crate::provider::test_support::clock_guard(&crate::device::Device::host(device));
        device_at(core, time, device, 0);
        guard
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
        let mut control = control(&core, &[Dir::Tx]);
        control.book(Dispatched::new(ActionId(1), Action::Stop { target: Some(super::super::test_support::target_of(&core.tx_id)) }, Some(core.tx_id.clone())));
        let stream = booked_stream(Dir::Tx);
        let stop = lock(&core.streams).line(stream).unwrap().items.iter().find(|item| item.kind == Kind::Stop).copied().unwrap();
        assert!(stop_halted(&core));
        assert_eq!(stop.e, core.now());
    }

    #[test]
    fn ur_30_a_transmit_stop_records_the_bound_its_bookings_end_at() {
        // RM-16 (#83): the `tx_stop_applied` row holds the Stop's root instant and the bound the bookings end at, the values the
        // parity test compares the repeat with. The bound is the first sample at or after `at + tx_in_flight`, plus five (as S3 states).
        let (core, mut control, time, _events) = tx_rig();
        book_tx_burst(&mut control, &core, 1, 10_000, true, ezsdr_kernel::stream::LatePolicy::SendAsapAndFlag);
        let stop = core.ticks(5_000_000);
        time.advance_to(core.at(stop)).unwrap();
        control.book(Dispatched::new(ActionId(2), Action::Stop { target: Some(crate::provider::test_support::target_of(&core.tx_id)) }, Some(core.tx_id.clone())));
        let clock = tx_clock(&core);
        let bound = clock.instant(clock.at_or_after(stop + core.tx_in_flight()) + 5);
        let row = lock(&core.rec).timing.iter().find(|row| row["what"] == "tx_stop_applied").cloned().expect("the Stop row");
        assert_eq!((row["stream"].as_u64(), row["at"].as_i64(), row["bound"].as_i64()), (Some(0), Some(stop), Some(bound)));
    }

    #[test]
    fn ur_21_an_untimed_repeat_after_a_stop_starts_at_the_bound_or_its_ordinary_start() {
        // RM-15, RM-16 (#83), on the controlled clock: a Stop at 5 ms bounds the stream at 10 ms of samples past it, plus five (the bound,
        // 15 ms here). An untimed repeat received inside that region (7 ms) starts at the bound; one received after it (20 ms) starts at
        // its ordinary start, which the device lead after its receipt sets. The test sets the instants, so no thread timing moves either.
        // The switch between the two is pinned at its edge: the smallest receipt whose ordinary start reaches the bound (found by search, with
        // the same `ordinary_k` the test uses), the tick before it and the tick after it each start at the later of the two.
        let held_at = |receipt: i64| {
            let (core, mut control, time, _events) = tx_rig();
            book_tx_burst(&mut control, &core, 1, 10_000, true, ezsdr_kernel::stream::LatePolicy::SendAsapAndFlag);
            let stop = core.ticks(5_000_000);
            time.advance_to(core.at(stop)).unwrap();
            control.book(Dispatched::new(ActionId(2), Action::Stop { target: Some(crate::provider::test_support::target_of(&core.tx_id)) }, Some(core.tx_id.clone())));
            let clock = tx_clock(&core);
            let bound = clock.at_or_after(stop + core.tx_in_flight()) + 5;
            time.advance_to(core.at(receipt)).unwrap();
            let ordinary =
                ezsdr_radio::staging::ordinary_k(ezsdr_radio::timeline::decided_at(receipt, clock.origin, core.device_lead()), core.device_lead(), |t| clock.at_or_after(t), |k| clock.instant(k));
            while next_booking(&core).is_some() {}
            // The Kernel stamps an untimed burst at its receipt; the Provider takes the later of that and the ordinary start.
            control.book(tx_burst_dispatch(&core, 3, clock.at_or_after(receipt), true, ezsdr_kernel::stream::LatePolicy::DropAndFlag, true));
            let held = next_booking(&core).expect("the untimed repeat is handed to uhd-tx");
            (held.k, bound, ordinary)
        };
        let (probe, ..) = tx_rig();
        let ms = |n: i64| probe.ticks(n * 1_000_000);
        for (receipt_ms, inside) in [(7, true), (20, false)] {
            let (k, bound, ordinary) = held_at(ms(receipt_ms));
            let expected = if inside { bound } else { ordinary };
            assert_eq!(k, expected, "received at {receipt_ms} ms: the bound is {bound}, the ordinary start {ordinary}");
        }
        let clock = tx_clock(&probe);
        let bound = clock.at_or_after(probe.ticks(5_000_000) + probe.tx_in_flight()) + 5;
        let ordinary_at = |receipt: i64| {
            ezsdr_radio::staging::ordinary_k(ezsdr_radio::timeline::decided_at(receipt, clock.origin, probe.device_lead()), probe.device_lead(), |t| clock.at_or_after(t), |k| clock.instant(k))
        };
        let (mut low, mut high) = (ms(7), ms(20));
        assert!(ordinary_at(low) < bound && ordinary_at(high) >= bound, "the switch lies between 7 and 20 ms");
        while high - low > 1 {
            let mid = low + (high - low) / 2;
            if ordinary_at(mid) >= bound { high = mid } else { low = mid }
        }
        let switch = high;
        for receipt in [switch - 1, switch, switch + 1] {
            let (k, held_bound, ordinary) = held_at(receipt);
            assert_eq!(held_bound, bound, "the bound does not depend on the receipt");
            assert_eq!(k, bound.max(ordinary), "received at tick {receipt} (the switch is {switch}): the bound is {bound}, the ordinary start {ordinary}");
        }
        assert!(ordinary_at(switch) >= bound && ordinary_at(switch - 1) < bound, "{switch} is the smallest receipt whose ordinary start reaches the bound");
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
        let mut control = control(&core, &[Dir::Tx]);
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
        let held = next_booking(&core).expect("the receipt site handed the burst to uhd-tx");
        let moved = held.k;
        assert_ne!(moved, requested.ticks_in(clock.domain).unwrap(), "the receipt move was not applied");
        assert!(clock.instant(moved) >= core.ticks(10_000_000) + core.ticks(DEVICE_LEAD_NS), "the moved start is not the device lead past the booking instant: {}", clock.instant(moved));
        let booked = lock(&core.streams).stream(stream_key(Dir::Tx, 0)).book.record.stageable(core.now()).err();
        assert_eq!(booked, Some(Refusal::TxBusy { until: Until::At(clock.instant(moved + held.waveform_len as i64)) }), "the moved start is booked before the hand-over");
    }

    #[test]
    fn ur_21_production_booking_preserves_the_original_repeat_length() {
        let samples: Vec<[f32; 2]> = (0..997).map(|i| [i as f32 / 1_000.0, 0.25]).collect();
        let bytes: Vec<u8> = samples.iter().flat_map(|sample| [sample[0].to_le_bytes(), sample[1].to_le_bytes()].concat()).collect();
        let hash = ezsdr_kernel::hash::ContentHash::of_bytes(&bytes);
        let waveform =
            ezsdr_kernel::manifest::ingest_input(ezsdr_kernel::spec::Ident::parse("waveform").unwrap(), ezsdr_kernel::spec::Namespace::parse("ezsdr.input").unwrap(), format!("mem:{hash}"), &bytes);
        let (core, _device, time, _) = crate::provider::test_support::rig_with_inputs(BTreeMap::from([(hash, Arc::<[u8]>::from(bytes))]));
        let mut control = control(&core, &[Dir::Tx]);
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
        let held = next_booking(&core).expect("production booking hands the repeat to uhd-tx");
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
        let mut control = control(&core, &[Dir::Tx]);
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
        let held = next_booking(&core).expect("the receipt site handed the burst to uhd-tx");
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
        let held = next_booking(&core).expect("the receipt site handed the moved burst to uhd-tx");
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
        let mut control = control(&core, &[Dir::Tx]);
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
        assert!(next_booking(&core).is_none(), "a refused burst was handed to uhd-tx");
        // The refusal reaches the caller as `COMMAND_REJECTED`, naming the Action it answers.
        let event = events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).into_iter().find(|event| event.kind.as_str() == kinds::COMMAND_REJECTED).expect("COMMAND_REJECTED");
        assert_eq!(event.payload["reason"], "UR-21: the burst begins before the device's start-up", "{event:?}");
        assert_eq!(event.payload["action"], 1, "{event:?}");
    }

    #[test]
    fn ur_25_a_transmit_stream_has_one_clock_for_the_run() {
        // RM-25, UR-21: a transmit stream's clock is registered once, at arm; every burst after it is booked on that clock.
        let samples: Vec<(f32, f32)> = vec![(0.25, 0.0), (0.5, 0.0)];
        let bytes: Vec<u8> = samples.iter().flat_map(|(re, im)| [re.to_le_bytes(), im.to_le_bytes()].concat()).collect();
        let hash = ezsdr_kernel::hash::ContentHash::of_bytes(&bytes);
        let waveform =
            ezsdr_kernel::manifest::ingest_input(ezsdr_kernel::spec::Ident::parse("waveform").unwrap(), ezsdr_kernel::spec::Namespace::parse("ezsdr.input").unwrap(), format!("mem:{hash}"), &bytes);
        let (core, _device, _time, _events) = crate::provider::test_support::rig_with_inputs(BTreeMap::from([(hash, Arc::<[u8]>::from(bytes))]));
        let mut control = control(&core, &[Dir::Tx]);
        let clock = tx_clock(&core);
        for (id, at) in [(1, 20_000), (2, 40_000)] {
            control.book(Dispatched::new(
                ActionId(id),
                Action::TxBurst {
                    target: super::super::test_support::target_of(&core.tx_id),
                    waveform: waveform.clone(),
                    repeat: false,
                    at: AbsoluteDeadline::new(TimePoint::new(clock.domain, at)),
                    requested_at: None,
                    late_policy: ezsdr_kernel::stream::LatePolicy::DropAndFlag,
                    metadata: BTreeMap::new(),
                },
                Some(core.tx_id.clone()),
            ));
            assert!(lock(&core.rec).rejected.is_empty(), "{:?}", lock(&core.rec).rejected);
            let held = next_booking(&core).expect("the burst is handed to uhd-tx");
            assert_eq!(held.domain, clock.domain);
            assert_eq!(lock(&core.streams).stream(booked_stream(Dir::Tx)).clock.map(|c| c.domain), Some(clock.domain));
        }
    }

    /// uhd-control with a transmit clock at 2 MS/s (100 root ticks a sample) whose origin is `origin` root ticks (RM-25).
    fn control_with_tx_origin(core: &Arc<Core>, origin: i64) -> Control {
        let mut config = core.description.defaults();
        config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        config.insert(key("radio.tx.c0.stream"), Value::from(0));
        let mut start = Start { t0: 0, rx: vec![None; core.description.stream_count(Dir::Rx)], tx: vec![None; core.description.stream_count(Dir::Tx)] };
        start.tx[0] = Some((core.register(Dir::Tx, 100, origin).unwrap(), Channels::first(1)));
        Control::new(core.clone(), Arc::new(NoActions), config, false, start)
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
        let mut control = control_with_tx_origin(&core, core.ticks(50_000_000));
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
        let held = next_booking(&core).expect("the receipt site handed the burst to uhd-tx");
        assert_eq!(held.k, 0, "the moved start is not the clock's origin");
        assert_eq!(clock.instant(held.k), plan[0].segment.origin);
        let booked = lock(&core.streams).stream(stream_key(Dir::Tx, 0)).book.record.stageable(core.now()).err();
        assert_eq!(booked, Some(Refusal::TxBusy { until: Until::At(clock.instant(held.k + held.waveform_len as i64)) }), "the origin is booked before the hand-over");
    }

    #[test]
    fn ur_21_an_untimed_send_is_placed_after_its_delivery() {
        // KG-8 (spike K6), on the controlled clock: the Kernel stamps an untimed send at its receipt plus the profile's 5 ms lead
        // (the 2 ms device lead and the 3 ms delivery allowance), and uhd-control books it when it next reads it. Its start is the later
        // of that instant and the ordinary start at the read, placed by the one untimed rule: no `TIME_ERROR`, whatever the read's delay,
        // and the late policy (`drop_and_flag` here) is not consulted. The fake's thread timing cannot move either: the test sets the instants.
        let samples: Vec<(f32, f32)> = vec![(0.25, 0.0), (0.5, 0.0)];
        let bytes: Vec<u8> = samples.iter().flat_map(|(re, im)| [re.to_le_bytes(), im.to_le_bytes()].concat()).collect();
        let hash = ezsdr_kernel::hash::ContentHash::of_bytes(&bytes);
        let waveform =
            ezsdr_kernel::manifest::ingest_input(ezsdr_kernel::spec::Ident::parse("waveform").unwrap(), ezsdr_kernel::spec::Namespace::parse("ezsdr.input").unwrap(), format!("mem:{hash}"), &bytes);
        let (core, _device, time, events) = crate::provider::test_support::rig_with_inputs(BTreeMap::from([(hash, Arc::<[u8]>::from(bytes))]));
        let mut control = control(&core, &[Dir::Tx]);
        let clock = tx_clock(&core);
        let lead = core.ticks(crate::profile::DEVICE_LEAD_NS + crate::profile::DELIVERY_ALLOWANCE_NS);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        // (milliseconds from receipt to uhd-control's read): 1 and 3 ms leave the send's own instant binding, 4 ms leaves less than the device lead, so the ordinary start binds.
        for (n, read_after) in [1, 3, 4].into_iter().enumerate() {
            let receipt = ms(100 * (n as i64 + 1));
            time.advance_to(core.at(receipt)).unwrap();
            let target = clock.at_or_after(receipt + lead);
            let read = receipt + ms(read_after);
            time.advance_to(core.at(read)).unwrap();
            control.book(Dispatched::new_untimed(
                ActionId(n as u64 + 1),
                Action::TxBurst {
                    target: super::super::test_support::target_of(&core.tx_id),
                    waveform: waveform.clone(),
                    repeat: false,
                    at: AbsoluteDeadline::new(TimePoint::new(clock.domain, target)),
                    requested_at: None,
                    late_policy: ezsdr_kernel::stream::LatePolicy::DropAndFlag,
                    metadata: BTreeMap::new(),
                },
                Some(core.tx_id.clone()),
            ));
            let errors = events.drain(ezsdr_kernel::time::TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|e| e.kind.as_str() == kinds::TIME_ERROR).count();
            assert_eq!(errors, 0, "read {read_after} ms after receipt: an untimed burst has no TIME_ERROR");
            let held = next_booking(&core).expect("the untimed burst is handed to uhd-tx");
            let ordinary =
                ezsdr_radio::staging::ordinary_k(ezsdr_radio::timeline::decided_at(read, clock.origin, core.device_lead()), core.device_lead(), |t| clock.at_or_after(t), |k| clock.instant(k));
            assert_eq!(held.k, target.max(ordinary), "read {read_after} ms after receipt");
        }
    }

    #[test]
    fn ur_21_an_untimed_burst_before_the_clock_s_origin_is_placed() {
        // RS-19, on the controlled clock: with no `at`, a burst before the transmit clock's origin (50 ms ahead) is stamped at that
        // origin. uhd-control reads it a poll later and it starts on time at the origin; read 49 ms in, with less than the 2 ms
        // device lead left, it is placed after its ordinary start, with no `TIME_ERROR` (the late policy is not consulted).
        let samples: Vec<(f32, f32)> = vec![(0.25, 0.0), (0.5, 0.0)];
        let bytes: Vec<u8> = samples.iter().flat_map(|(re, im)| [re.to_le_bytes(), im.to_le_bytes()].concat()).collect();
        let hash = ezsdr_kernel::hash::ContentHash::of_bytes(&bytes);
        let waveform =
            ezsdr_kernel::manifest::ingest_input(ezsdr_kernel::spec::Ident::parse("waveform").unwrap(), ezsdr_kernel::spec::Namespace::parse("ezsdr.input").unwrap(), format!("mem:{hash}"), &bytes);
        for read_after_ms in [1, 49] {
            let (core, _device, time, events) = crate::provider::test_support::rig_with_inputs(BTreeMap::from([(hash.clone(), Arc::<[u8]>::from(bytes.clone()))]));
            let mut control = control_with_tx_origin(&core, core.ticks(50_000_000));
            let plan = plan_of(&core, Dir::Tx);
            let clock = tx_clock(&core);
            assert_eq!(plan[0].segment.origin, core.ticks(50_000_000), "the clock's origin");
            let read = core.ticks(read_after_ms * 1_000_000);
            time.advance_to(core.at(read)).unwrap();
            events.drain(ezsdr_kernel::time::TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0));
            control.book(Dispatched::new_untimed(
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
            assert_eq!(errors, 0, "read {read_after_ms} ms after the change");
            let held = next_booking(&core).expect("the untimed burst is handed to uhd-tx");
            let ordinary =
                ezsdr_radio::staging::ordinary_k(ezsdr_radio::timeline::decided_at(read, clock.origin, core.device_lead()), core.device_lead(), |t| clock.at_or_after(t), |k| clock.instant(k));
            assert_eq!(held.k, ordinary.max(0), "read {read_after_ms} ms after the change");
        }
    }

    /// The Stop-region rig: a repeat at sample 2 500 booked at device time 0, then a `Stop` there that cancels it, so the Stop region
    /// is `[0, stop)`. Returns the Provider's control, the handed bursts, the transmit clock, the waveform's reference and the events.
    #[allow(clippy::type_complexity)]
    fn stopped_repeat(late_policy: ezsdr_kernel::stream::LatePolicy) -> (Control, Clock, ezsdr_kernel::manifest::ArtifactRef, Arc<ezsdr_kernel::event::EventCollector>, Arc<Core>) {
        let samples: Vec<(f32, f32)> = vec![(0.25, 0.0), (0.5, 0.0)];
        let bytes: Vec<u8> = samples.iter().flat_map(|(re, im)| [re.to_le_bytes(), im.to_le_bytes()].concat()).collect();
        let hash = ezsdr_kernel::hash::ContentHash::of_bytes(&bytes);
        let waveform =
            ezsdr_kernel::manifest::ingest_input(ezsdr_kernel::spec::Ident::parse("waveform").unwrap(), ezsdr_kernel::spec::Namespace::parse("ezsdr.input").unwrap(), format!("mem:{hash}"), &bytes);
        let (core, _device, _time, events) = crate::provider::test_support::rig_with_inputs(BTreeMap::from([(hash, Arc::<[u8]>::from(bytes))]));
        let mut control = control(&core, &[Dir::Tx]);
        let clock = tx_clock(&core);
        control.book(Dispatched::new(
            ActionId(1),
            Action::TxBurst {
                target: super::super::test_support::target_of(&core.tx_id),
                waveform: waveform.clone(),
                repeat: true,
                at: AbsoluteDeadline::new(TimePoint::new(clock.domain, 2_500)),
                requested_at: None,
                late_policy,
                metadata: BTreeMap::new(),
            },
            Some(core.tx_id.clone()),
        ));
        control.book(Dispatched::new(ActionId(2), Action::Stop { target: Some(super::super::test_support::target_of(&core.tx_id)) }, Some(core.tx_id.clone())));
        (control, clock, waveform, events, core)
    }

    #[test]
    fn ur_21_s16_timed_send_asap_in_the_stop_region_moves_with_time_error() {
        // RM-16 (F36, UHD): a timed `SendAsap` burst on time at receipt whose start is in the Stop region moves past it, with one
        // `TIME_ERROR { send_asap }`, and its record is late by the same amount and reports its `at` as the requested target.
        let (mut control, clock, waveform, events, core) = stopped_repeat(ezsdr_kernel::stream::LatePolicy::SendAsapAndFlag);
        let at = TimePoint::new(clock.domain, 2_010);
        control.book(Dispatched::new(
            ActionId(3),
            Action::TxBurst {
                target: super::super::test_support::target_of(&core.tx_id),
                waveform,
                repeat: false,
                at: AbsoluteDeadline::new(at),
                requested_at: None,
                late_policy: ezsdr_kernel::stream::LatePolicy::SendAsapAndFlag,
                metadata: BTreeMap::new(),
            },
            Some(core.tx_id.clone()),
        ));
        let verdicts: Vec<_> = events.drain(ezsdr_kernel::time::TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|e| e.kind.as_str() == kinds::TIME_ERROR).collect();
        assert_eq!(verdicts.len(), 1, "{verdicts:?}");
        assert_eq!(verdicts[0].payload["outcome"], "send_asap", "{verdicts:?}");
        let bursts: Vec<_> = booked(&core);
        let moved = bursts.iter().find(|held| !held.repeat).expect("the moved burst is handed to uhd-tx");
        assert!(moved.k > 2_010, "the burst starts after its `at`, past the Stop region: {}", moved.k);
        // The late amount is the whole move, in nanoseconds of the root clock (UR-21).
        let late_ns = (moved.k - 2_010) * clock.n * 1_000_000_000 / core.ticks(1_000_000_000);
        assert_eq!(verdicts[0].payload["late_by_ns"], late_ns, "{verdicts:?}");
        assert_eq!(moved.open.requested_target, Some(at), "the record reports the instant the caller asked for");
        assert!(matches!(moved.open.late, Some(LateOutcome::SendAsap { .. })), "{:?}", moved.open.late);
    }

    #[test]
    fn ur_21_s16_timed_drop_in_the_stop_region_is_refused() {
        // RM-16 (F36, UHD): a timed `DropAndFlag` burst whose start is in the Stop region is refused, with no `TIME_ERROR`.
        let (mut control, clock, waveform, events, core) = stopped_repeat(ezsdr_kernel::stream::LatePolicy::DropAndFlag);
        control.book(Dispatched::new(
            ActionId(3),
            Action::TxBurst {
                target: super::super::test_support::target_of(&core.tx_id),
                waveform,
                repeat: false,
                at: AbsoluteDeadline::new(TimePoint::new(clock.domain, 2_010)),
                requested_at: None,
                late_policy: ezsdr_kernel::stream::LatePolicy::DropAndFlag,
                metadata: BTreeMap::new(),
            },
            Some(core.tx_id.clone()),
        ));
        let events = events.drain(ezsdr_kernel::time::TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0));
        assert!(events.iter().all(|e| e.kind.as_str() != kinds::TIME_ERROR), "{events:?}");
        assert_eq!(events.iter().filter(|e| e.kind.as_str() == kinds::COMMAND_REJECTED).count(), 1, "{events:?}");
        assert!(booked(&core).iter().all(|held| held.repeat), "a refused burst is not handed to uhd-tx");
    }

    #[test]
    fn ur_26_an_orderly_stop_cuts_at_its_instant() {
        // RM-16, UR-26 (Review M, N-6): `Provider::stop`'s cut is booked at the stop instant:
        // under `orderly` the first sample at or after it, or, once uhd-rx has delivered past
        // it, the first sample it has not delivered.
        for (delivered, cut) in [(None, 5_000), (Some(6_000), 6_000)] {
            let (core, _, time) = rig();
            let _control = control(&core, &[Dir::Rx]);
            time.advance_to(core.at(core.ticks(8_000_000))).unwrap();
            let config = plan_of(&core, Dir::Rx)[0].segment.config;
            if let Some(k) = delivered {
                let mut streams = lock(&core.streams);
                streams.started(booked_stream(Dir::Rx), 0, config);
                streams.delivered(booked_stream(Dir::Rx), 0, config, k);
            }
            crate::provider::test_support::end_streams(&core, core.ticks(5_000_000), false, false);
            assert_eq!(plan_of(&core, Dir::Rx)[0].segment.cut, Some(cut));
        }
    }

    #[test]
    fn ur_26_a_stop_ends_every_line_once() {
        // RM-16, UR-26 (design 33 D15): a loss or `Provider::stop` books one `End` on every
        // stream's line, each with the same instant and arrival order, and a second end
        // books nothing: `ended` is the instance's, not a stream's.
        let (core, _, _) = rig();
        let _control = control(&core, &[Dir::Rx, Dir::Tx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let (rx, tx) = (booked_stream(Dir::Rx), booked_stream(Dir::Tx));
        crate::provider::test_support::end_streams(&core, ms(5), false, false);
        let seq = lock(&core.streams).seq;
        crate::provider::test_support::end_streams(&core, ms(7), true, false);
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
        let (core, _time, _) = crate::provider::test_support::rig_on_description(
            Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, ..crate::device::FakeConfig::default() })),
            vec![Arc::new(NoLink)],
            BTreeMap::new(),
            description,
        );
        let mut control = control(&core, &[Dir::Rx]);
        let streams = std::ptr::from_ref(&core.streams) as usize;
        let instant = core.ticks(300_000_000);
        let at = Some(AbsoluteDeadline::new(core.at(instant)));
        let node = core.stream_node(Dir::Rx, 0);
        // A start of a streaming stream is refused (RM-21), so the stream is stopped first.
        control.book(Dispatched::new(ActionId(1), Action::Stop { target: Some(super::super::test_support::target_of(&node)) }, Some(node.clone())));
        crate::provider::test_support::record_locks();
        control.book(Dispatched::new(
            ActionId(2),
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
        let _control = control(&core, &[]);
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
        let device = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, ..crate::device::FakeConfig::default() }));
        let links = vec![vec![Arc::new(NoLink) as Arc<dyn DataLink>]];
        let (core, time, events) = crate::provider::test_support::rig_on_description_links(device.clone(), links, BTreeMap::from([(hash, Arc::<[u8]>::from(bytes))]), layout_description());
        let _guard = hold(&core, &time, &device);
        let mut control = control(&core, &[Dir::Rx, Dir::Tx]);
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
        assert!(next_booking(&core).is_some(), "the burst was not booked on the list");
        let booked = lock(&core.streams).stream(stream_key(Dir::Tx, 0)).book.record.stageable(core.now()).err();
        assert!(booked.is_some(), "the burst is not booked ahead of its instant");
        control.book(Dispatched::new(
            ActionId(2),
            Action::Stop { target: Some(super::super::test_support::target_of(&ResourceId::parse("usrp/rx/s0").unwrap())) },
            Some(ResourceId::parse("usrp/rx/s0").unwrap()),
        ));
        assert!(!stop_halted(&core), "the receive stream's Stop reached uhd-tx");
        assert_eq!(lock(&core.streams).stream(stream_key(Dir::Tx, 0)).book.record.stageable(core.now()).err(), booked, "the receive stream's Stop cancelled the transmit burst");
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
        let start = Start { t0: 0, rx: vec![None; core.description.stream_count(Dir::Rx)], tx: vec![None; core.description.stream_count(Dir::Tx)] };
        let control = Control::new(core.clone(), receiver.clone(), core.description.defaults(), false, start);
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

    // ------------------------------------------------------------ note 47 stage 3: staging and the snapshot (RM-21, UR-26)

    /// A receive-only rig with its events: uhd-control booking on the manual clock held at 0.
    fn staging_rig() -> (Arc<Core>, Control, Arc<crate::device::FakeDevice>, Arc<ezsdr_kernel::event::EventCollector>, crate::host::Driver) {
        let config = crate::device::FakeConfig::default();
        let (core, device, time, events) = crate::provider::test_support::rig_with(config, vec![Arc::new(NoLink)]);
        let guard = hold(&core, &time, &device);
        let control = control(&core, &[Dir::Rx]);
        (core, control, device, events, guard)
    }

    fn book_value(control: &mut Control, core: &Core, id: u64, name: &str, value: Value, at: Option<i64>) {
        let action = Action::UpdateParameter {
            target: crate::provider::test_support::target_of(&core.id),
            key: key(name),
            value,
            class: ezsdr_kernel::module_api::UpdateClass::Cold,
            at: at.map(|ticks| AbsoluteDeadline::new(core.at(ticks))),
        };
        control.book(Dispatched::new(ActionId(id), action, Some(core.id.clone())));
    }

    fn book_stop(control: &mut Control, core: &Core, id: u64) {
        control.book(Dispatched::new(ActionId(id), Action::Stop { target: Some(crate::provider::test_support::target_of(&core.rx_id)) }, Some(core.rx_id.clone())));
    }

    fn book_start(control: &mut Control, core: &Core, id: u64, at: Option<i64>) {
        let action = Action::Command {
            target: crate::provider::test_support::target_of(&core.rx_id),
            verb: ezsdr_kernel::spec::Ident::parse("start_rx").unwrap(),
            params: BTreeMap::new(),
            at: at.map(|ticks| AbsoluteDeadline::new(core.at(ticks))),
        };
        control.book(Dispatched::new(ActionId(id), action, Some(core.rx_id.clone())));
    }

    fn frequency(value: f64) -> Value {
        Value::num(value).unwrap()
    }

    /// The frequency of each `start_rx` snapshot of receive stream 0, by the command's arrival order.
    fn snapshots(core: &Core) -> Vec<f64> {
        let streams = lock(&core.streams);
        let stream = stream_key(Dir::Rx, 0);
        let seqs: Vec<u64> = streams.line(stream).unwrap().items.iter().filter(|item| item.kind == Kind::Start).map(|item| item.seq).collect();
        let name = key("radio.rx.c0.frequency_hz");
        seqs.iter()
            .map(|seq| match streams.stream(stream).configs[seq].base.get(&name) {
                Some(Value::Scalar(Scalar::Num(value))) => value.get(),
                other => panic!("no frequency in the snapshot: {other:?}"),
            })
            .collect()
    }

    #[test]
    fn ur_26_a_value_for_a_streaming_stream_is_refused_without_a_device_call() {
        let (core, mut control, device, events, _guard) = staging_rig();
        let calls = device.calls().len();
        book_value(&mut control, &core, 7, "radio.rx.c0.frequency_hz", frequency(2.4e9), None);
        let refused = events_of(&events, kinds::COMMAND_REJECTED);
        assert_eq!(refused.len(), 1);
        assert_eq!(refused[0].payload["action"], 7, "the refusal names the Action");
        assert_eq!(refused[0].payload["kind"], "update_parameter");
        let reason = refused[0].payload["reason"].as_str().unwrap();
        assert!(reason.starts_with("UR-26: RM-21:") && reason.contains("streaming"), "{reason}");
        assert_eq!(device.calls().len(), calls, "no device call");
        assert!(lock(&core.streams).stream(stream_key(Dir::Rx, 0)).configs.is_empty());
        // After a `Stop` and a start the snapshot still holds the value the Run began with.
        let begun = core.description.defaults()[&key("radio.rx.c0.frequency_hz")].clone();
        book_stop(&mut control, &core, 8);
        book_start(&mut control, &core, 9, None);
        assert_eq!(snapshots(&core).len(), 1);
        assert_eq!(
            snapshots(&core)[0],
            match begun {
                Value::Scalar(Scalar::Num(value)) => value.get(),
                other => panic!("{other:?}"),
            }
        );
    }

    #[test]
    fn ur_26_the_last_value_before_a_start_is_the_one_it_snapshots_and_a_second_stop_leaves_it() {
        let (core, mut control, _device, events, _guard) = staging_rig();
        book_stop(&mut control, &core, 1);
        book_value(&mut control, &core, 2, "radio.rx.c0.frequency_hz", frequency(2.0e9), None);
        book_value(&mut control, &core, 3, "radio.rx.c0.frequency_hz", frequency(3.0e9), None);
        book_start(&mut control, &core, 4, None);
        // `Stop`, a value, `Stop`, `start_rx`: the second Stop clears nothing.
        book_stop(&mut control, &core, 5);
        book_value(&mut control, &core, 6, "radio.rx.c0.frequency_hz", frequency(4.0e9), None);
        book_stop(&mut control, &core, 7);
        book_start(&mut control, &core, 8, None);
        // `Stop`, `start_rx` with nothing staged: the previous value is the configuration of record.
        book_stop(&mut control, &core, 9);
        book_start(&mut control, &core, 10, None);
        assert!(events_of(&events, kinds::COMMAND_REJECTED).is_empty());
        // Each snapshot is its own copy: the first is not changed by the later values.
        assert_eq!(snapshots(&core), [3.0e9, 4.0e9, 4.0e9]);
    }

    #[test]
    fn ur_26_a_value_after_a_booked_start_is_refused_even_before_its_instant() {
        let (core, mut control, _device, events, _guard) = staging_rig();
        let ms = |n: i64| core.ticks(n * 1_000_000);
        book_stop(&mut control, &core, 1);
        book_start(&mut control, &core, 2, Some(ms(50)));
        book_value(&mut control, &core, 3, "radio.rx.c0.frequency_hz", frequency(2.4e9), None);
        assert_eq!(events_of(&events, kinds::COMMAND_REJECTED).len(), 1, "the start is booked, so the value would be lost silently");
        assert_ne!(snapshots(&core)[0], 2.4e9);
    }

    #[test]
    fn ur_26_a_stop_after_a_booked_future_start_does_not_make_the_stream_stopped() {
        // The line replayed in the order the commands take effect: the start at +1 s still begins its segment with its snapshot, so the value after
        // the second `Stop` is refused; an arrival-order flag would accept it while every sample came at the old value.
        let (core, mut control, device, events, _guard) = staging_rig();
        let begun = snapshots_default(&core);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        book_stop(&mut control, &core, 1);
        book_start(&mut control, &core, 2, Some(ms(1_000)));
        book_stop(&mut control, &core, 3);
        let calls = device.calls().len();
        book_value(&mut control, &core, 4, "radio.rx.c0.frequency_hz", frequency(2.4e9), None);
        assert_eq!(events_of(&events, kinds::COMMAND_REJECTED).len(), 1);
        assert_eq!(device.calls().len(), calls);
        assert_eq!(snapshots(&core), [begun]);
        // The plan: the start still begins its segment at +1 s.
        let plan = plan_of(&core, Dir::Rx);
        assert!(
            plan.iter().any(|planned| planned.segment.by == Some(2) && planned.segment.origin >= ms(1_000)),
            "{:?}",
            plan.iter().map(|planned| (planned.segment.origin, planned.segment.by)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn ur_26_a_start_of_a_streaming_stream_is_refused_with_the_start_s_id_and_leaves_the_line_unchanged() {
        // RM-21: the Run begins streaming, so a first `start_rx` is refused, naming the Action with kind `command`; after a `Stop` a start booked ahead
        // (+1 s) counts as streaming too, so a second one at +200 ms is refused and the line keeps the first (never revised to an earlier origin).
        let (core, mut control, _device, events, _guard) = staging_rig();
        let ms = |n: i64| core.ticks(n * 1_000_000);
        book_start(&mut control, &core, 5, None);
        let refused = events_of(&events, kinds::COMMAND_REJECTED);
        assert_eq!(refused.len(), 1, "{refused:?}");
        assert_eq!((refused[0].payload["kind"].as_str(), &refused[0].payload["action"]), (Some("command"), &serde_json::json!(5)));
        let reason = refused[0].payload["reason"].as_str().unwrap();
        assert!(reason.starts_with("UR-26: RM-21:") && reason.contains("streaming") && reason.contains("Stop"), "{reason}");
        assert!(snapshots(&core).is_empty() && lock(&core.streams).stream(stream_key(Dir::Rx, 0)).configs.is_empty(), "a refused start leaves no snapshot");
        book_stop(&mut control, &core, 6);
        book_start(&mut control, &core, 7, Some(ms(1_000)));
        book_start(&mut control, &core, 8, Some(ms(200)));
        let refused = events_of(&events, kinds::COMMAND_REJECTED);
        assert_eq!(refused.len(), 1, "{refused:?}");
        assert_eq!(refused[0].payload["action"], 8);
        let starts: Vec<u64> = lock(&core.streams).line(stream_key(Dir::Rx, 0)).unwrap().items.iter().filter(|item| item.kind == Kind::Start).map(|item| item.seq).collect();
        assert_eq!(starts.len(), 1, "the line holds the first start alone");
        let plan = plan_of(&core, Dir::Rx);
        assert_eq!(plan.iter().filter_map(|planned| planned.segment.by).collect::<Vec<_>>(), starts, "only the first start is in the plan");
        assert!(plan.iter().any(|planned| planned.segment.by == Some(starts[0]) && planned.segment.origin >= ms(1_000)));
        assert_eq!(snapshots(&core).len(), 1);
    }

    #[test]
    fn ur_25_a_booking_the_line_cannot_represent_is_refused_with_the_start_s_id() {
        // RM-25: a start whose origin overflows the tick range is refused at its booking and the refusal names the Action, so the submit reply (EA-11)
        // can match it; the line keeps no item for it.
        let (core, mut control, _device, events, _guard) = staging_rig();
        book_stop(&mut control, &core, 1);
        book_start(&mut control, &core, 2, Some(i64::MAX));
        let refused = events_of(&events, kinds::COMMAND_REJECTED);
        assert_eq!(refused.len(), 1, "{refused:?}");
        assert_eq!((refused[0].payload["kind"].as_str(), &refused[0].payload["action"]), (Some("command"), &serde_json::json!(2)));
        assert!(refused[0].payload["reason"].as_str().unwrap().starts_with("RM-25:"));
        assert_eq!(lock(&core.streams).line(stream_key(Dir::Rx, 0)).unwrap().items.iter().filter(|item| item.kind == Kind::Start).count(), 0);
    }

    fn snapshots_default(core: &Core) -> f64 {
        match core.description.defaults()[&key("radio.rx.c0.frequency_hz")] {
            Value::Scalar(Scalar::Num(value)) => value.get(),
            ref other => panic!("{other:?}"),
        }
    }

    #[test]
    fn ur_26_a_staged_value_with_an_instant_or_another_target_is_refused_and_the_instance_and_its_streams_are_accepted() {
        let (core, mut control, _device, events, _guard) = staging_rig();
        book_stop(&mut control, &core, 1);
        book_value(&mut control, &core, 2, "radio.rx.c0.frequency_hz", frequency(2.0e9), Some(core.ticks(5_000_000)));
        let other = ResourceId::parse("elsewhere").unwrap();
        control.book(Dispatched::new(
            ActionId(3),
            Action::UpdateParameter {
                target: crate::provider::test_support::target_of(&other),
                key: key("radio.rx.c0.frequency_hz"),
                value: frequency(2.0e9),
                class: ezsdr_kernel::module_api::UpdateClass::Cold,
                at: None,
            },
            Some(other),
        ));
        let refused = events_of(&events, kinds::COMMAND_REJECTED);
        assert_eq!(refused.len(), 2);
        assert!(refused[0].payload["reason"].as_str().unwrap().contains("instant"));
        assert_eq!(refused[1].payload["reason"], "UR-26: the target is not this device");
        // The instance's node and a stream's node both accept (RM-12: the key decides what changes).
        for (id, node) in [(4, core.id.clone()), (5, core.rx_id.clone())] {
            control.book(Dispatched::new(
                ActionId(id),
                Action::UpdateParameter {
                    target: crate::provider::test_support::target_of(&node),
                    key: key("radio.rx.c0.frequency_hz"),
                    value: frequency(2.0e9 + id as f64),
                    class: ezsdr_kernel::module_api::UpdateClass::Cold,
                    at: None,
                },
                Some(node),
            ));
        }
        assert!(events_of(&events, kinds::COMMAND_REJECTED).is_empty());
        book_start(&mut control, &core, 6, None);
        assert_eq!(snapshots(&core), [2.0e9 + 5.0], "the later value, the refused ones kept nothing");
    }

    #[test]
    fn ur_26_a_peer_stream_s_state_does_not_matter_and_an_instance_stop_stops_both() {
        // S5 on the layout (two receive streams, c0 in stream 0 and c1 in stream 1): after a `Stop` of stream 0 only c0's value is accepted, and
        // stream 1's line takes nothing; a `Stop` of the instance makes c1's value acceptable as well.
        let device = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, ..crate::device::FakeConfig::default() }));
        let links = vec![vec![Arc::new(NoLink) as Arc<dyn DataLink>], vec![Arc::new(NoLink) as Arc<dyn DataLink>]];
        let (core, time, events) = crate::provider::test_support::rig_on_description_links(device.clone(), links, BTreeMap::new(), layout_description());
        let _guard = hold(&core, &time, &device);
        let mut config = core.description.defaults();
        config.insert(key("radio.rx.c0.stream"), Value::from(0));
        config.insert(key("radio.rx.c1.stream"), Value::from(1));
        let start = Start { t0: 0, rx: vec![Some((Channels::first(0).with(0), 200)), Some((Channels::first(0).with(1), 200))], tx: vec![None; core.description.stream_count(Dir::Tx)] };
        let mut control = Control::new(core.clone(), Arc::new(NoActions), config, false, start);
        let calls = device.calls().len();
        control.book(Dispatched::new(
            ActionId(1),
            Action::Stop { target: Some(crate::provider::test_support::target_of(&ResourceId::parse("usrp/rx/s0").unwrap())) },
            Some(ResourceId::parse("usrp/rx/s0").unwrap()),
        ));
        book_value(&mut control, &core, 2, "radio.rx.c0.frequency_hz", frequency(2.0e9), None);
        book_value(&mut control, &core, 3, "radio.rx.c1.frequency_hz", frequency(2.0e9), None);
        let refused = events_of(&events, kinds::COMMAND_REJECTED);
        assert_eq!(refused.len(), 1, "{refused:?}");
        assert_eq!(refused[0].payload["action"], 3, "stream 1 is streaming");
        assert_eq!(lock(&core.streams).line(stream_key(Dir::Rx, 1)).unwrap().items.len(), 0, "stream 1's line took nothing");
        assert_eq!(device.calls().len(), calls, "no device call for either value");
        control.book(Dispatched::new(ActionId(4), Action::Stop { target: Some(crate::provider::test_support::target_of(&core.id)) }, Some(core.id.clone())));
        book_value(&mut control, &core, 5, "radio.rx.c1.frequency_hz", frequency(2.0e9), None);
        assert!(events_of(&events, kinds::COMMAND_REJECTED).is_empty(), "the instance's Stop stopped stream 1 too");
    }

    // ------------------------------------------------------------ note 47 stage 4, part (b) b2: transmit values (RM-21, UR-26)

    /// Four transmit samples, the one waveform the transmit tests book.
    fn four_samples() -> Vec<u8> {
        [(0.25f32, 0.0f32), (0.5, 0.0), (0.75, 0.0), (1.0, 0.0)].iter().flat_map(|(re, im)| [re.to_le_bytes(), im.to_le_bytes()].concat()).collect()
    }

    /// Transmit stream 0's clock and its input (the four samples), with `control` on transmit only.
    fn tx_rig() -> (Arc<Core>, Control, Arc<ezsdr_kernel::time::ManualTimeAuthority>, Arc<ezsdr_kernel::event::EventCollector>) {
        let bytes = four_samples();
        let hash = ezsdr_kernel::hash::ContentHash::of_bytes(&bytes);
        let (core, _device, time, events) = crate::provider::test_support::rig_with_inputs(BTreeMap::from([(hash, Arc::<[u8]>::from(bytes))]));
        let control = control(&core, &[Dir::Tx]);
        (core, control, time, events)
    }

    /// Books a burst of the four samples at sample `start` of transmit stream 0's clock.
    fn book_tx_burst(control: &mut Control, core: &Core, id: u64, start: i64, repeat: bool, late_policy: ezsdr_kernel::stream::LatePolicy) {
        control.book(tx_burst_dispatch(core, id, start, repeat, late_policy, false));
    }

    /// The burst of `book_tx_burst` as the Kernel dispatches it: untimed (RS-19) when `untimed`.
    fn tx_burst_dispatch(core: &Core, id: u64, start: i64, repeat: bool, late_policy: ezsdr_kernel::stream::LatePolicy, untimed: bool) -> Dispatched {
        let bytes = four_samples();
        let hash = ezsdr_kernel::hash::ContentHash::of_bytes(&bytes);
        let waveform =
            ezsdr_kernel::manifest::ingest_input(ezsdr_kernel::spec::Ident::parse("waveform").unwrap(), ezsdr_kernel::spec::Namespace::parse("ezsdr.input").unwrap(), format!("mem:{hash}"), &bytes);
        let clock = tx_clock(core);
        let at = AbsoluteDeadline::new(TimePoint::new(clock.domain, start));
        let action = Action::TxBurst { target: crate::provider::test_support::target_of(&core.tx_id), waveform, repeat, at, requested_at: None, late_policy, metadata: BTreeMap::new() };
        if untimed { Dispatched::new_untimed(ActionId(id), action, Some(core.tx_id.clone())) } else { Dispatched::new(ActionId(id), action, Some(core.tx_id.clone())) }
    }

    fn tx_frequency(settings: &ColdConfig) -> f64 {
        match settings.base.get(&key("radio.tx.c0.frequency_hz")) {
            Some(Value::Scalar(Scalar::Num(value))) => value.get(),
            other => panic!("no transmit frequency in the snapshot: {other:?}"),
        }
    }

    const TX_FREQUENCY: &str = "radio.tx.c0.frequency_hz";

    #[test]
    fn ur_21_a_value_while_a_burst_is_booked_is_refused_with_its_action_id() {
        // S2: a burst booked for the future keeps its stream busy to its end; the value is refused with its own Action id.
        let (core, mut control, _time, events) = tx_rig();
        book_tx_burst(&mut control, &core, 1, 10_000, false, ezsdr_kernel::stream::LatePolicy::SendAsapAndFlag);
        book_value(&mut control, &core, 2, TX_FREQUENCY, frequency(2.4e9), None);
        let refused = events_of(&events, kinds::COMMAND_REJECTED);
        assert_eq!(refused.len(), 1, "{refused:?}");
        assert_eq!((refused[0].payload["kind"].as_str(), &refused[0].payload["action"]), (Some("update_parameter"), &serde_json::json!(2)));
        assert!(refused[0].payload["reason"].as_str().unwrap().starts_with("UR-26: RM-21:"), "{refused:?}");
    }

    #[test]
    fn ur_21_a_value_after_the_hand_over_and_before_the_end_is_refused() {
        // F2 (#79): the burst's reservation and hand-over do not free the stream; only its booked end does.
        let (core, mut control, _time, events) = tx_rig();
        book_tx_burst(&mut control, &core, 1, 10_000, false, ezsdr_kernel::stream::LatePolicy::SendAsapAndFlag);
        next_booking(&core).expect("the burst is handed over at booking");
        book_value(&mut control, &core, 2, TX_FREQUENCY, frequency(2.4e9), None);
        assert_eq!(events_of(&events, kinds::COMMAND_REJECTED).len(), 1, "a value after the hand-over is refused until the burst's end");
    }

    #[test]
    fn ur_23_a_value_at_the_exclusive_end_of_a_burst_is_accepted() {
        // S5: one sample before the exclusive end is refused, the end itself is accepted.
        let (core, mut control, time, events) = tx_rig();
        book_tx_burst(&mut control, &core, 1, 10_000, false, ezsdr_kernel::stream::LatePolicy::SendAsapAndFlag);
        let end = tx_clock(&core).instant(10_004);
        time.advance_to(core.at(end - 1)).unwrap();
        book_value(&mut control, &core, 2, TX_FREQUENCY, frequency(2.4e9), None);
        assert_eq!(events_of(&events, kinds::COMMAND_REJECTED).len(), 1, "one sample before the end is refused");
        time.advance_to(core.at(end)).unwrap();
        book_value(&mut control, &core, 3, TX_FREQUENCY, frequency(2.5e9), None);
        assert!(events_of(&events, kinds::COMMAND_REJECTED).is_empty(), "at the exclusive end the value is accepted");
    }

    #[test]
    fn ur_25_the_last_value_before_a_burst_is_the_one_it_plays() {
        // S4: the last value before the burst is the one it snapshots, with the stream's generation; a value after the booking is refused.
        let (core, mut control, _time, events) = tx_rig();
        book_value(&mut control, &core, 1, TX_FREQUENCY, frequency(2.0e9), None);
        book_value(&mut control, &core, 2, TX_FREQUENCY, frequency(3.0e9), None);
        book_tx_burst(&mut control, &core, 3, 10_000, false, ezsdr_kernel::stream::LatePolicy::SendAsapAndFlag);
        book_value(&mut control, &core, 4, TX_FREQUENCY, frequency(4.0e9), None);
        let refused = events_of(&events, kinds::COMMAND_REJECTED);
        assert_eq!(refused.iter().map(|event| event.payload["action"].clone()).collect::<Vec<_>>(), [serde_json::json!(4)]);
        let held = next_booking(&core).expect("the burst is handed over");
        assert_eq!(tx_frequency(&held.settings), 3.0e9);
        assert_eq!(held.settings.generation, Some(2), "two accepted values, the burst takes the generation of its booking");
    }

    #[test]
    fn ur_23_a_value_is_taken_only_after_a_stopped_repeat_s_in_flight_window() {
        // S3: a repeat keeps the stream busy; a `Stop` at `s` leaves it busy until the first sample at or after `s + tx_in_flight`, plus five (RM-16), and a value at that bound is taken.
        let (core, mut control, time, events) = tx_rig();
        book_tx_burst(&mut control, &core, 1, 10_000, true, ezsdr_kernel::stream::LatePolicy::SendAsapAndFlag);
        book_value(&mut control, &core, 2, TX_FREQUENCY, frequency(2.4e9), None);
        assert_eq!(events_of(&events, kinds::COMMAND_REJECTED).len(), 1, "a value while the repeat plays is refused");
        let stop = core.ticks(5_000_000);
        time.advance_to(core.at(stop)).unwrap();
        control.book(Dispatched::new(ActionId(3), Action::Stop { target: Some(crate::provider::test_support::target_of(&core.tx_id)) }, Some(core.tx_id.clone())));
        let clock = tx_clock(&core);
        let bound = clock.instant(clock.at_or_after(stop + core.tx_in_flight()) + 5);
        time.advance_to(core.at(bound - 1)).unwrap();
        book_value(&mut control, &core, 4, TX_FREQUENCY, frequency(2.5e9), None);
        assert_eq!(events_of(&events, kinds::COMMAND_REJECTED).len(), 1, "one tick before the bound is refused");
        time.advance_to(core.at(bound)).unwrap();
        book_value(&mut control, &core, 5, TX_FREQUENCY, frequency(2.6e9), None);
        assert!(events_of(&events, kinds::COMMAND_REJECTED).is_empty(), "at the bound the value is taken");
    }

    #[test]
    fn ur_23_a_provider_stop_ends_a_repeat_at_its_in_flight_window() {
        // RM-16, VH-2: `Provider::stop` (the stream ends at `at`) bounds the repeat as a `Stop` does.
        let (core, mut control, time, events) = tx_rig();
        book_tx_burst(&mut control, &core, 1, 10_000, true, ezsdr_kernel::stream::LatePolicy::SendAsapAndFlag);
        let stop = core.ticks(5_000_000);
        time.advance_to(core.at(stop)).unwrap();
        crate::provider::test_support::end_streams(&core, stop, false, false);
        let clock = tx_clock(&core);
        let bound = clock.instant(clock.at_or_after(stop + core.tx_in_flight()) + 5);
        time.advance_to(core.at(bound - 1)).unwrap();
        book_value(&mut control, &core, 2, TX_FREQUENCY, frequency(2.4e9), None);
        assert_eq!(events_of(&events, kinds::COMMAND_REJECTED).len(), 1, "one tick before the bound is refused");
        time.advance_to(core.at(bound)).unwrap();
        book_value(&mut control, &core, 3, TX_FREQUENCY, frequency(2.5e9), None);
        assert!(events_of(&events, kinds::COMMAND_REJECTED).is_empty(), "at the bound the value is taken");
    }

    #[test]
    fn ur_21_a_burst_dropped_at_booking_is_not_booked() {
        // S-book (P4): a booking its late policy drops occupies no span. At 10 ms the device lead
        // ends at 12 000: the burst at 11 999 is late and would span [11 999, 12 004), so the
        // burst at 12 002, on time at receipt, is accepted.
        let (core, mut control, time, events) = tx_rig();
        time.advance_to(core.at(core.ticks(10_000_000))).unwrap();
        book_tx_burst(&mut control, &core, 1, 11_999, false, ezsdr_kernel::stream::LatePolicy::DropAndFlag);
        assert!(next_booking(&core).is_none(), "the dropped burst is not handed over");
        book_value(&mut control, &core, 3, TX_FREQUENCY, frequency(2.5e9), None);
        assert!(events_of(&events, kinds::COMMAND_REJECTED).is_empty(), "a dropped burst leaves the stream unbooked: the value is taken");
        book_tx_burst(&mut control, &core, 2, 12_002, false, ezsdr_kernel::stream::LatePolicy::DropAndFlag);
        assert!(events_of(&events, kinds::COMMAND_REJECTED).is_empty(), "a dropped burst occupies no span");
        assert!(next_booking(&core).is_some(), "the on-time burst is handed over");
    }

    #[test]
    fn ur_21_a_value_is_judged_by_a_booking_time_move() {
        // R2, S2: a burst the booking moves (SendAsap, the device is late at 15 ms) keeps the stream busy to its moved end, not its requested one.
        let (core, mut control, time, events) = tx_rig();
        time.advance_to(core.at(core.ticks(15_000_000))).unwrap();
        book_tx_burst(&mut control, &core, 1, 10_000, false, ezsdr_kernel::stream::LatePolicy::SendAsapAndFlag);
        let held = next_booking(&core).expect("the moved burst is handed over at booking");
        assert!(held.k > 10_004, "the booking moved the burst past its requested end: {}", held.k);
        let end = tx_clock(&core).instant(held.k + 4);
        time.advance_to(core.at(end - 1)).unwrap();
        book_value(&mut control, &core, 2, TX_FREQUENCY, frequency(2.4e9), None);
        assert_eq!(events_of(&events, kinds::COMMAND_REJECTED).len(), 1, "one sample before the moved end is refused");
        time.advance_to(core.at(end)).unwrap();
        book_value(&mut control, &core, 3, TX_FREQUENCY, frequency(2.5e9), None);
        assert!(events_of(&events, kinds::COMMAND_REJECTED).is_empty(), "at the moved end the value is accepted");
    }
}
