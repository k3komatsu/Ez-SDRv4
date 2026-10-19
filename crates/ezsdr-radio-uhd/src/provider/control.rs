//! uhd-control (UR-14): drains the `ActionReceiver` with MA-14b's loop, books each
//! Action — a stream's commands on its timeline, which it shares with the stream's owner —
//! releases held timed commands, and every 500 ms checks the reference and reads the
//! device's time. It never waits for a device instant.

use std::collections::BTreeMap;
use std::sync::mpsc::Sender;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use std::time::Duration as Wall;

use ezsdr_kernel::event::{Action, ActionId, Severity};
use ezsdr_kernel::id::{ClockDomainId, ResourceId};
use ezsdr_kernel::module_api::{ActionReceiver, Dispatched, Requested, UpdateClass};
use ezsdr_kernel::spec::{Constraint, Key, Value};
use ezsdr_kernel::stream::{BurstOpen, Direction, LateOutcome};
use ezsdr_kernel::time::{AbsoluteDeadline, Duration, Rational, TimePoint, TimeError};
use ezsdr_radio::payloads::{
    ClockLostPayload, ClockReference, CommandQueueFullPayload, LateCommandPayload, TimeErrorCause,
    TimeErrorOutcome, TimeErrorPayload,
};
use ezsdr_radio::timeline::{self, Channels, Config, Item, Kind, Line, Segment, StreamKey, NO_RELEASE};
use ezsdr_radio::kinds;
use serde_json::json;

use super::core::{assert_line_free, booked_stream, stream_key, Clock, Core, lock};
use super::mode::{current_key_direction, scalar_number};
use ezsdr_radio::Addressed;
use super::tx::{Held, TxCmd};
use crate::device::{DeviceError, Dir, Iq, Settings, decimation};
use crate::host::{ClockMutex, HostClock};
use crate::profile::{DELIVERY_ALLOWANCE_NS, DEVICE_LEAD_NS, RELEASE_WINDOW_NS};

const POLL: Wall = Wall::from_millis(1);
const CHECK: Wall = Wall::from_millis(500);

/// A held `hardware_timed` command (UR-24), of the channel its key names under the layout.
struct Timed {
    key: Key,
    dir: Dir,
    value: f64,
    /// The logical channel the layout's key names; `None` under the current keys.
    chan: Option<usize>,
}

impl Timed {
    fn settings(&self) -> Settings {
        if self.key.as_str().ends_with("frequency_hz") {
            Settings { freq: Some(self.value), ..Settings::default() }
        } else {
            Settings { gain: Some(self.value), ..Settings::default() }
        }
    }
}

/// The configuration a command's segment begins with, shared with its owner: the one in
/// effect at the segment's origin (UC-2's projection, UR-17), its rate and channel set the
/// segment's. Updates are projected in effective order, even when admitted after booking.
pub(crate) struct ColdConfig {
    base: Base,
    /// §4.7: the clock lock — a shared lock held across the device call in `configure`, so a
    /// waiting participant is idle and the manual clock advances while the holder waits.
    updates: ClockMutex<BTreeMap<(i64, u64), (usize, Settings)>>,
    /// Counts the changes to the projection a device never received as a timed command: a
    /// record-only release, a cancelled update (UR-36). A configure records the version it read
    /// under `updates`, so an owner configures again when the version has moved on.
    version: AtomicU64,
}

/// What a segment's configuration begins from (UR-12): the current keys' one `Settings`
/// for the direction, or the layout's numbered configuration at the segment's origin.
enum Base {
    Current(Settings),
    Layout(BTreeMap<Key, Value>),
}

impl ColdConfig {
    #[cfg(test)]
    pub fn new(base: Settings) -> Self {
        Self::with_clock(HostClock::real(), base, BTreeMap::new())
    }

    /// With the device's host clock (§4.7): the lock becomes the clock lock on a manual
    /// clock. `updates` is a copy the caller made of the held timed commands, so a configuration
    /// built inside the instance's line lock reads no held command and takes no lock of theirs
    /// (§4.7's order).
    pub(crate) fn with_clock(clock: HostClock, base: Settings, updates: BTreeMap<(i64, u64), (usize, Settings)>) -> Self {
        Self { base: Base::Current(base), updates: clock.clock_mutex(updates), version: AtomicU64::new(0) }
    }

    /// The layout's numbered configuration at the segment's origin (RM-30, UR-12), with
    /// the held timed commands projected into it the same way.
    pub(crate) fn layout_with_clock(clock: HostClock, base: BTreeMap<Key, Value>, updates: BTreeMap<(i64, u64), (usize, Settings)>) -> Self {
        Self { base: Base::Layout(base), updates: clock.clock_mutex(updates), version: AtomicU64::new(0) }
    }

    fn book(&self, order: (i64, u64), chan: usize, settings: Settings) {
        assert_line_free("a configuration's updates");
        self.updates.lock().insert(order, (chan, settings));
    }

    /// The configuration's version now (UR-36).
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }

    /// The projection changed in a way the device was not told: the version moves on, under
    /// `updates`, so that an owner's configure that read the old one is not taken for this one.
    /// The caller decides which segments the change reaches (UR-36).
    fn changed(&self) {
        assert_line_free("a configuration's updates");
        let _updates = self.updates.lock();
        self.version.fetch_add(1, Ordering::AcqRel);
    }

    fn issued(&self, updates: &mut BTreeMap<(i64, u64), (usize, Settings)>, order: (i64, u64), effective: i64, chan: usize, settings: Settings) {
        updates.remove(&order);
        updates.insert((effective, order.1), (chan, settings));
    }

    /// The configuration in effect at `origin` under the current keys (one for the
    /// direction), with the updates due by then.
    fn settings(&self, updates: &BTreeMap<(i64, u64), (usize, Settings)>, origin: i64) -> Settings {
        let mut settings = match &self.base {
            Base::Current(base) => base.clone(),
            Base::Layout(_) => Settings::default(),
        };
        for (_, (_, update)) in updates.range(..=(origin, u64::MAX)) {
            if let Some(freq) = update.freq { settings.freq = Some(freq); }
            if let Some(gain) = update.gain { settings.gain = Some(gain); }
        }
        settings
    }

    /// The configuration in effect at `origin` on logical channel `chan` (UR-12, RM-30):
    /// the layout's own channel values, with the updates due by then that name it.
    fn settings_on(&self, core: &Core, updates: &BTreeMap<(i64, u64), (usize, Settings)>, dir: Dir, origin: i64, chan: usize) -> Settings {
        let Base::Layout(base) = &self.base else { return self.settings(updates, origin) };
        let mut settings = core.mode.settings(base, dir, chan);
        for (update_chan, update) in updates.range(..=(origin, u64::MAX)).map(|(_, update)| update) {
            if *update_chan != chan { continue; }
            if let Some(freq) = update.freq { settings.freq = Some(freq); }
            if let Some(gain) = update.gain { settings.gain = Some(gain); }
        }
        settings
    }

    /// Configures the direction for `segment` (UR-12), each of its channels' own settings;
    /// returns the decimation and the version the configuration was read at (UR-36).
    pub fn configure(&self, core: &Core, dir: Dir, segment: &Segment, k: usize) -> Result<(i64, u64), DeviceError> {
        // Hold the snapshot lock across the call: an update cannot change the
        // projection while an owner is configuring from it (§4.7: the clock lock).
        assert_line_free("a configuration's updates");
        let updates = self.updates.lock();
        let version = self.version();
        let ratio = segment.config.ratio;
        let rate = core.mcr as f64 * ratio.den() as f64 / ratio.num() as f64;
        let channels: Vec<(usize, Settings)> = segment
            .config
            .channels
            .iter()
            .map(|chan| {
                let mut settings = self.settings_on(core, &updates, dir, segment.origin, chan as usize);
                settings.rate = Some(rate);
                (chan as usize, settings)
            })
            .collect();
        core.configure(dir, &channels, rate, k).map(|n| (n, version))
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
    /// Its transmit clock, registered when it was booked (KC-21a); a receive clock is
    /// registered by uhd-rx at its first block.
    pub clock: Option<Clock>,
}

impl Planned {
    /// The segment's channel set (RM-29): the stream's channels in channel order.
    pub fn set(&self) -> Channels {
        self.segment.config.channels
    }

    /// The version of the segment's configuration now, if it has one (UR-36).
    pub fn version(&self) -> Option<u64> {
        self.settings.as_ref().map(|settings| settings.version())
    }

    /// Root ticks per sample.
    pub fn n(&self) -> i64 {
        self.segment.config.ratio.num() as i64
    }

    /// Configures the direction for the segment (UR-12), as [`ColdConfig::configure`].
    pub fn configure(&self, core: &Core, dir: Dir) -> Result<(i64, u64), DeviceError> {
        match &self.settings {
            Some(settings) => settings.configure(core, dir, &self.segment, self.k),
            None => Err(DeviceError::failed("UR-25: the segment has no configuration")),
        }
    }
}

pub(crate) struct Control {
    pub core: Arc<Core>,
    pub actions: Arc<dyn ActionReceiver>,
    pub to_tx: Sender<TxCmd>,
    /// The configuration at the Run's start, with every `hardware_timed` value issued or, for
    /// a direction with no channel, recorded since (UR-24).
    pub config: BTreeMap<Key, Value>,
    pub reference_monitored: bool,
    /// The Run's T0 (UR-15): the instant `start` began the device's streams at, which a
    /// layout `cold` change of the receive side is floored at whatever its line holds,
    /// as MockRadio's is (RM-25).
    t0: i64,
    /// The booked `cold` values, by effective instant and arrival (UC-2).
    colds: BTreeMap<(i64, u64), (Key, Value)>,
    held: BTreeMap<(i64, u64), Timed>,
    released: Vec<i64>,
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

/// The streams at the Run's start (UR-15): the receive stream's first segment at T0, when
/// it has a channel and a link, and the transmit clock registered at `arm`.
pub(crate) struct Start {
    pub t0: i64,
    pub rx: Option<(Channels, i64)>,
    pub tx: Option<(Clock, Channels)>,
}

impl Control {
    /// uhd-control, with the streams' timelines shared from the Run's start.
    pub fn new(
        core: Arc<Core>,
        actions: Arc<dyn ActionReceiver>,
        to_tx: Sender<TxCmd>,
        config: BTreeMap<Key, Value>,
        reference_monitored: bool,
        start: Start,
    ) -> Control {
        let host = core.device.host();
        // RM-25, VH-4: this Module's terms — each direction's start lead, and, for a receive
        // segment after a cut, the receive call in progress there and the delivery
        // allowance (UR-25). The call term is the receive line's, stream 0's, the one line
        // the current keys have; 1b-4d reads each stream's own packet size.
        let call = core.block_len.max(core.device.rx_packet_samples(0)) as i64;
        let allowance = core.ticks(DELIVERY_ALLOWANCE_NS);
        let line = |key: StreamKey, origin: i64, channels: Channels, n: i64| Line::new(timeline::Stream {
            direction: key.0,
            origin,
            config: Config { channels, ratio: Rational::new(n.max(1) as u64, 1).expect("a positive ratio") },
            lead: core.ticks(ezsdr_radio::timeline::start_lead_ns(&core.mode.timing(), key.0)),
            call,
            allowance,
        });
        let n = |dir: Dir| {
            let rate = core.mode.stream_rate(&config, dir, 0);
            decimation(core.mcr, rate).map_or(1, |n| n as i64)
        };
        let rx = (!core.links.is_empty()).then(|| {
            let (channels, n) = start.rx.unwrap_or((Channels::first(0), n(Dir::Rx)));
            line(booked_stream(Dir::Rx), start.t0, channels, n)
        });
        let tx = match start.tx {
            Some((clock, channels)) => line(booked_stream(Dir::Tx), clock.origin, channels, clock.n),
            None => line(booked_stream(Dir::Tx), 0, Channels::first(0), n(Dir::Tx)),
        };
        {
            let mut streams = lock(&core.streams);
            streams.stream_mut(booked_stream(Dir::Tx)).clocks = start.tx.map(|(clock, _)| (clock, false)).into_iter().collect();
            if let Some(rx) = rx {
                streams.lines.insert(booked_stream(Dir::Rx), rx);
            }
            streams.lines.insert(booked_stream(Dir::Tx), tx);
            streams.stream_mut(booked_stream(Dir::Rx)).idle = start.rx.is_none();
            streams.stream_mut(booked_stream(Dir::Tx)).idle = start.tx.is_none();
        }
        Control {
            core,
            actions,
            to_tx,
            config,
            reference_monitored,
            t0: start.t0,
            colds: BTreeMap::new(),
            held: BTreeMap::new(),
            released: Vec::new(),
            clock_lost: false,
            failing_since: None,
            host,
        }
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
                self.release();
            }
            self.release();
            if self.host.now().saturating_sub(last_check) >= CHECK && !self.core.is_lost() {
                last_check = self.host.now();
                self.check_device();
            }
            self.host.sleep(POLL);
        }
    }

    /// UR-26: `Provider::stop`, which has booked the streams' end, drops every pending
    /// command but one already handed to the device; UR-30's plans are recorded.
    fn finish(&mut self) {
        self.cancel_held("Provider::stop");
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
                self.book_burst(id, node, addressed, waveform, repeat, at, requested_at, late_policy, metadata.is_empty());
            }
            Action::UpdateParameter { key, value, class, at, .. } => {
                let name = key.as_str();
                // RM-12: the node selects what the update may reach — the instance, and
                // under the layout one of its streams; the key decides what changes.
                let reachable = match addressed {
                    Addressed::Instance => true,
                    Addressed::Stream { .. } => self.core.mode.is_layout(),
                    Addressed::Other => false,
                };
                let updatable = match (class, self.core.mode.update_class(&key)) {
                    (UpdateClass::HardwareTimed, Some(UpdateClass::HardwareTimed)) => 1,
                    (UpdateClass::Cold, Some(UpdateClass::Cold)) => 2,
                    _ => 0,
                };
                if !reachable {
                    self.core.command_rejected(Some(id), "update_parameter", "UR-26: the target is not this device");
                } else if updatable == 1 {
                    self.book_timed(id, key, value, at);
                } else if updatable == 2 {
                    self.book_cold(id, key, value, at);
                } else {
                    self.core.command_rejected(Some(id), "update_parameter", &format!("UR-26: {name} is not updated at runtime by this Provider"));
                }
            }
            // RM-16, UR-26: a `Stop` cancels no update; on receive it cuts at its booking, on
            // transmit it ends the bursts, not the clock. `<id>` stops every stream; a
            // stream node stops that stream alone (RM-12, D14); under the layout the old
            // `<id>/rx` and `<id>/tx` are no targets.
            Action::Stop { target: Some(_) } => {
                let (rx, tx) = match addressed {
                    Addressed::Instance => (true, true),
                    Addressed::Stream { direction, k } => {
                        let dir = Dir::from(direction);
                        match (self.core.mode.is_layout(), dir) {
                            (true, _) => (k == 0 && dir == Dir::Rx, k == 0 && dir == Dir::Tx),
                            (false, Dir::Rx) => (true, false),
                            (false, Dir::Tx) => (false, true),
                        }
                    }
                    Addressed::Other => {
                        return self.core.command_rejected(Some(id), "stop", "UR-26: the Stop target is not this device or its streams");
                    }
                };
                if tx {
                    let _ = self.to_tx.send(TxCmd::Stop);
                }
                if rx {
                    let mut streams = lock(&self.core.streams);
                    let stream = booked_stream(Dir::Rx);
                    let (seq, now) = (streams.next_seq(), self.core.now());
                    streams.book(&self.core, stream, Item { e: now, seq, ready: now, release: NO_RELEASE, delivered: None, refused: false, kind: Kind::Stop });
                }
            }
            Action::Stop { target: None } | Action::Abort { .. } => {}
            Action::SetTimer { .. } => self.core.command_rejected(Some(id), "set_timer", "UR-26: not supported by this Provider"),
            // RM-12, RM-21: `start_rx` turns the receive stream on at its `at`, or at its booking
            // if that is later; it is never late. A stream the layout holds no channel for is
            // accepted and carries out nothing, as MockRadio's does (D14).
            Action::Command { verb, params, at, .. }
                if verb.as_str() == ezsdr_radio::START_RX && params.is_empty() =>
            {
                let Addressed::Stream { direction: Direction::Rx, k } = addressed else {
                    return self.core.command_rejected(Some(id), "command", "UR-26: not supported by this Provider");
                };
                if k != 0 {
                    return;
                }
                let requested = match self.ceil_root(at) {
                    Ok(requested) => requested,
                    Err(error) => return self.core.command_rejected(Some(id), "command", &format!("UR-26: start_rx's instant cannot be converted: {error}")),
                };
                // The current keys' configuration depends on no instant, so it is built
                // before the lock; the layout's is built inside it, from the instant's
                // configuration, and takes no lock of its own (UR-25's order).
                let current = (!self.core.mode.is_layout()).then(|| self.segment_config(Dir::Rx));
                // One critical section takes the instant and the arrival order and books the
                // start, so that nothing pruned meanwhile takes effect after them (#63).
                let mut streams = lock(&self.core.streams);
                let stream = booked_stream(Dir::Rx);
                let (seq, now) = (streams.next_seq(), self.core.now());
                let (e, _) = timeline::command_instant(streams.line(stream), requested, now, seq, false);
                let config = match current {
                    Some(config) => config,
                    None => Arc::new(ColdConfig::layout_with_clock(self.core.device.host(), self.config_at(e), self.held_updates(Dir::Rx))),
                };
                streams.stream_mut(stream).configs.insert(seq, config);
                streams.book(&self.core, stream, Item { e, seq, ready: now, release: NO_RELEASE, delivered: None, refused: false, kind: Kind::Start });
            }
            Action::Command { .. } => {
                self.core.command_rejected(Some(id), "command", "UR-26: not supported by this Provider")
            }
            Action::Emit { .. } => self.core.command_rejected(Some(id), "emit", "UR-26: not supported by this Provider"),
        }
    }

    /// What an Action's resolved node names (KC-23, RM-12, RM-29): the instance, one of its
    /// streams — `<id>/rx` and `<id>/tx` under the current keys, `<id>/rx/s<k>` and
    /// `<id>/tx/s<k>` under the layout — or something else. The judgement is the shared
    /// `ezsdr_radio::addressed`.
    fn addressed(&self, node: Option<&ResourceId>) -> Addressed {
        self.core.mode.addressed(&self.core, node)
    }

    /// The current keys' configuration a segment a command begins in `dir` starts from: the
    /// one in force, with the timed commands held for that direction (UC-2, UR-17).
    fn segment_config(&self, dir: Dir) -> Arc<ColdConfig> {
        Arc::new(ColdConfig::with_clock(self.core.device.host(), Core::settings(&self.config, dir), self.held_updates(dir)))
    }

    /// UC-2, UR-17: the timed commands held for `dir` as a segment configuration's own
    /// entries, each on the channel its key names. Data, not a call on the configuration, so
    /// that a configuration built under the instance's line lock takes no second lock:
    /// `ColdConfig::book`'s order (the configuration's `updates`, then `streams`) has no
    /// counterpart for a configuration no other thread has seen yet (UR-25).
    fn held_updates(&self, dir: Dir) -> BTreeMap<(i64, u64), (usize, Settings)> {
        self.held
            .iter()
            .filter(|(_, timed)| timed.dir == dir)
            .map(|(&order, timed)| (order, (timed.chan.unwrap_or(0), timed.settings())))
            .collect()
    }

    /// The segment configurations of `dir`, shared with its owner.
    fn projections(&self, dir: Dir) -> Vec<Arc<ColdConfig>> {
        lock(&self.core.streams).stream(booked_stream(dir)).configs.values().cloned().collect()
    }

    /// UR-36: the segment configurations of `dir` the timed command of instant `e` reaches —
    /// the ones whose version moves on when the device is not told of it (a release recorded
    /// with no channel, a cancellation). `chan` names the command's channel under the layout;
    /// `None` is every channel of its direction, which is the current keys' command. A segment
    /// whose origin precedes `e`, or which does not run the named channel, holds nothing of the
    /// command, so its configuration and its version stand.
    fn changed_projections(&self, dir: Dir, e: i64, chan: Option<usize>) -> Vec<Arc<ColdConfig>> {
        let streams = lock(&self.core.streams);
        let stream = booked_stream(dir);
        let Some(line) = streams.line(stream) else { return Vec::new() };
        streams.stream(stream).configs.iter().filter_map(|(seq, config)| {
            let segment = line.plan.iter().find(|segment| segment.by == Some(*seq))?;
            (e <= segment.origin && chan.is_none_or(|chan| segment.config.channels.contains(chan as u32))).then(|| config.clone())
        }).collect()
    }

    fn ceil_root(&self, at: Option<AbsoluteDeadline>) -> Result<Option<i64>, TimeError> {
        let Some(t) = at.map(|deadline| deadline.time_point) else { return Ok(None); };
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

    fn late_command(&self, key: Option<Key>, requested: i64, applied: i64) {
        let payload = serde_json::to_value(LateCommandPayload {
            key,
            requested: self.core.at(requested),
            applied: self.core.at(applied),
        })
        .expect("a payload");
        self.core.emit(&self.core.id, kinds::LATE_COMMAND, Severity::Warning, payload);
    }

    /// The configuration in force at `t`: the one at the Run's start with the timed values
    /// issued and the `cold` changes effective by `t` (UC-2; #61).
    fn config_at(&self, t: i64) -> BTreeMap<Key, Value> {
        let mut config = self.config.clone();
        for (key, value) in self.colds.range(..=(t, u64::MAX)).map(|(_, change)| change) {
            config.insert(key.clone(), value.clone());
        }
        config
    }

    /// The channel count of `dir` in force at `t`: none while a refusal halts it (RM-25, #64).
    fn channels_at(&self, dir: Dir, t: i64) -> usize {
        if lock(&self.core.streams).line(booked_stream(dir)).is_some_and(|line| line.halted(t)) {
            return 0;
        }
        Core::channels(&self.config_at(t), dir)
    }

    /// RM-27, UR-24: whether logical channel `chan` of `dir` is in a running stream at `t`
    /// — its `stream` is 0 in the configuration in force there and no refusal halts the
    /// stream. A timed update acts on the channel only then; otherwise it is recorded and
    /// the segment that later holds the channel applies it (item 7).
    fn channel_live(&self, dir: Dir, chan: usize, t: i64) -> bool {
        if lock(&self.core.streams).line(booked_stream(dir)).is_some_and(|line| line.halted(t)) {
            return false;
        }
        self.core.mode.stream_of(&self.config_at(t), dir, chan) == 0
    }

    // ------------------------------------------------------------ UR-24

    fn book_timed(&mut self, id: ActionId, key: Key, value: Value, at: Option<AbsoluteDeadline>) {
        let Some(v) = value.as_scalar().and_then(scalar_number) else {
            return self.core.command_rejected(Some(id), "update_parameter", "UR-24: the value is not a number");
        };
        // RM-12, RM-27: the key decides the direction and, under the layout, the one
        // channel a frequency or gain update acts on.
        let (dir, chan) = if self.core.mode.is_layout() {
            match ezsdr_radio::concrete(&key) {
                Some(ezsdr_radio::Concrete::Channel { rx, number, .. }) => {
                    let dir = if rx { Dir::Rx } else { Dir::Tx };
                    if (number as usize) < self.core.mode.channel_count(dir) {
                        (dir, Some(number as usize))
                    } else {
                        return self.core.command_rejected(Some(id), "update_parameter", &format!("UR-24: no such channel c{number}"));
                    }
                }
                _ => return self.core.command_rejected(Some(id), "update_parameter", &format!("UR-24: {key} is not a channel's frequency or gain")),
            }
        } else {
            (current_key_direction(&key), None)
        };
        let now = self.core.now();
        let requested = match self.ceil_root(at) {
            Ok(requested) => requested,
            Err(error) => return self.core.command_rejected(Some(id), "update_parameter", &format!("UR-24: explicit deadline cannot be converted: {error}")),
        };
        let Some(earliest) = now.checked_add(self.core.ticks(DEVICE_LEAD_NS)) else {
            return self.core.command_rejected(Some(id), "update_parameter", "UR-24: time arithmetic overflow");
        };
        let e = match requested {
            Some(requested) if requested < earliest => {
                self.late_command(Some(key.clone()), requested, earliest);
                earliest
            }
            Some(requested) => requested,
            None => earliest,
        };
        self.released.retain(|effective| *effective > now);
        let depth = self.core.mode.timing().command_queue_depth as usize;
        // The device queue holds one command per channel (Review L, NONBLOCKING 6); a
        // layout command names one channel, so it counts one slot (UR-24).
        let pending: usize = if self.core.mode.is_layout() {
            self.held.len() + self.released.len()
        } else {
            self.held.iter().map(|((e, _), t)| self.channels_at(t.dir, *e)).sum::<usize>() + self.released.len()
        };
        let cost = if self.core.mode.is_layout() { 1 } else { self.channels_at(dir, e) };
        if pending + cost > depth {
            let payload = serde_json::to_value(CommandQueueFullPayload { key: key.clone(), depth: depth as i64 }).expect("a payload");
            self.core.emit(&self.core.id, kinds::COMMAND_QUEUE_FULL, Severity::Error, payload);
            self.core.reject_note(json!({ "kind": "update_parameter", "reason": "UR-24: the command queue is full", "key": key }));
            return;
        }
        let seq = lock(&self.core.streams).next_seq();
        let timed = Timed { key, dir, value: v, chan };
        for config in self.projections(dir) {
            config.book((e, seq), timed.chan.unwrap_or(0), timed.settings());
        }
        self.held.insert((e, seq), timed);
    }

    /// Releases held commands within the release window, in effective order, each on the
    /// channels in force at its own instant; one for a direction with no channel then, or
    /// halted by a refusal, is recorded in the configuration, and enabling the direction
    /// applies it (UR-24; #61, #64). Under the layout one command acts on its own channel,
    /// and one for a channel no stream holds then is recorded the same way (item 7).
    pub(super) fn release(&mut self) {
        if self.core.is_lost() {
            return;
        }
        let now = self.core.now();
        let window = self.core.ticks(RELEASE_WINDOW_NS);
        self.released.retain(|effective| *effective > now);
        while let Some(entry) = self.held.first_entry() {
            let (e, seq) = *entry.key();
            if e - now > window {
                break;
            }
            let timed = entry.remove();
            let latest = self.released.iter().copied().max();
            let effective = latest.map_or(e, |latest| latest.max(e));
            // With no channel when the queue lets it act (none configured, or halted by a
            // refusal), it is recorded only, and the enable applies it.
            let chans: Vec<usize> = if self.core.mode.is_layout() {
                let chan = timed.chan.expect("a layout command names its channel");
                self.channel_live(timed.dir, chan, effective).then_some(chan).into_iter().collect()
            } else {
                (0..self.channels_at(timed.dir, effective)).collect()
            };
            if chans.is_empty() {
                // Nothing is issued to the device: the projection of every segment whose origin
                // this update reaches changed under a configuration its owner may have made, so
                // that one's version moves on (UR-36).
                for cold in self.changed_projections(timed.dir, e, timed.chan) {
                    cold.changed();
                }
                self.config.insert(timed.key.clone(), Value::num(timed.value).expect("UR-24: read from a Value"));
                lock(&self.core.rec).applied.push(json!({ "key": timed.key, "claimed": timed.value, "read_back": null, "at": self.core.at(e), "note": "no channel" }));
                continue;
            }
            if effective > e {
                // The device queue is in order: behind a later command it is late (UC-2).
                self.late_command(Some(timed.key.clone()), e, effective);
            }
            let settings = timed.settings();
            let projections = self.projections(timed.dir);
            // Order a timed issuance and its projections atomically against configure.
            assert_line_free("a timed command's release");
            let mut updates: Vec<_> = projections.iter().map(|cold| cold.updates.lock()).collect();
            for chan in &chans {
                if let Err(error) = self.core.device.apply(timed.dir, *chan, &settings, Some(effective)) {
                    self.core.device_failed(None, "update_parameter", &error);
                    return;
                }
                self.released.push(effective);
            }
            for (cold, updates) in projections.iter().zip(updates.iter_mut()) {
                cold.issued(updates, (e, seq), effective, timed.chan.unwrap_or(0), settings.clone());
            }
            drop(updates);
            self.config.insert(timed.key.clone(), Value::num(timed.value).expect("UR-24: read from a Value"));
            lock(&self.core.rec).applied.push(json!({
                "key": timed.key, "claimed": timed.value, "read_back": null, "at": self.core.at(effective), "issued": true,
            }));
        }
    }

    fn cancel_held(&mut self, why: &str) {
        let at = self.core.at(self.core.now());
        for ((e, seq), timed) in std::mem::take(&mut self.held) {
            // UR-36: a configuration whose projection at its segment's origin held the
            // cancelled command is configured again; one the command never reached keeps its
            // version, and with it its owner's trust in its record.
            let changed = self.changed_projections(timed.dir, e, timed.chan);
            let projections = self.projections(timed.dir);
            assert_line_free("a cancelled command's updates");
            let mut updates: Vec<_> = projections.iter().map(|cold| cold.updates.lock()).collect();
            for (cold, updates) in projections.iter().zip(updates.iter_mut()) {
                updates.remove(&(e, seq));
                if changed.iter().any(|one| Arc::ptr_eq(one, cold)) {
                    cold.version.fetch_add(1, Ordering::AcqRel);
                }
            }
            // Keep the projections locked until the cancellation is recorded, so an owner can
            // never apply this held value after that record.
            lock(&self.core.rec).applied.push(json!({
                "key": timed.key, "claimed": timed.value, "at": self.core.at(e), "cancelled": why, "cancelled_at": at,
            }));
        }
    }

    // ------------------------------------------------------------ UR-25

    /// UR-25: an enable from no stream whose owner is reading reports waits for the read to
    /// end before its first device call. The wait is the host clock's (UR-33), so uhd-control
    /// is idle and the device's time runs; `Provider::stop`'s mark and a loss end it too, and
    /// then no device call is made.
    fn await_reading(&self, stream: StreamKey) -> bool {
        self.core.probe("wait_reading", None);
        loop {
            let (reading, stop) = {
                let streams = lock(&self.core.streams);
                (streams.stream(stream).reading, streams.stop.is_some())
            };
            if stop || self.core.is_lost() {
                return false;
            }
            if !reading {
                return true;
            }
            self.host.sleep(POLL);
        }
    }

    /// UR-25: a `cold` update is booked when uhd-control takes it: its effective instant,
    /// `coerce` over the configuration projected there, and the stream's plan from the
    /// timeline, which its owner reads. Under the layout the key names its channel or its
    /// stream (RM-27), the floor is the instance's (RM-25), and only stream 0 runs
    /// (design 33 D14).
    fn book_cold(&mut self, id: ActionId, key: Key, value: Value, at: Option<AbsoluteDeadline>) {
        let layout = self.core.mode.is_layout();
        let dir = if layout {
            match ezsdr_radio::concrete(&key) {
                Some(ezsdr_radio::Concrete::Channel { rx, .. }) | Some(ezsdr_radio::Concrete::Stream { rx, .. }) => {
                    if rx { Dir::Rx } else { Dir::Tx }
                }
                None => return self.core.command_rejected(Some(id), "update_parameter", &format!("UR-26: {key} is not updated at runtime by this Provider")),
            }
        } else {
            current_key_direction(&key)
        };
        let requested = match self.ceil_root(at) {
            Ok(requested) => requested,
            Err(error) => return self.core.command_rejected(Some(id), "update_parameter", &format!("UR-25: explicit deadline cannot be converted: {error}")),
        };
        // The current keys' configuration the segment begins with depends on no instant, so
        // it is built before the lock: the lock order is the configuration's `updates`, then
        // `streams` (core.rs).
        let current = (!layout).then(|| self.segment_config(dir));
        // One critical section takes the instant and the arrival order and books the change,
        // so that nothing pruned meanwhile takes effect after it (#63). The layout's
        // configuration is built inside it and is new and unshared, so the order above has
        // no counterpart there: no other thread can hold its `updates` (UR-25).
        let mut streams = lock(&self.core.streams);
        let (seq, now) = (streams.next_seq(), self.core.now());
        let (e, late) = if layout {
            // RM-25: never before a still-pending `cold` change of the whole instance, nor
            // before the receive start, T0, which floors it whether or not that side has a
            // link yet, as MockRadio's `start_tick` does; the one place the rule lives
            // (item 11).
            let start = (dir == Dir::Rx).then_some(self.t0);
            let earlier = self.colds.keys().map(|(e, _)| *e);
            timeline::layout_instant(requested, now, start, earlier)
        } else {
            timeline::command_instant(streams.line(booked_stream(dir)), requested, now, seq, true)
        };
        // UC-2, VH-8: the instant's floor makes the change late, which MockRadio reports
        // before the reasons that may refuse it; the current keys report it after them, as
        // HEAD does.
        if late && layout {
            self.late_command(Some(key.clone()), requested.unwrap_or(now), e);
        }
        let before = self.config_at(e);
        let mut candidate = before.clone();
        candidate.insert(key.clone(), value.clone());
        // D14: a channel may not leave the one stream this Provider runs per direction.
        if let Some((dir, number, stream)) = self.core.mode.channel_in_later_stream(&candidate) {
            return self.core.command_rejected(Some(id), "update_parameter", &format!("UR-25: radio.{}.c{number}.stream names stream s{stream}; this Provider runs each direction's stream 0 alone", dir.name()));
        }
        let request = Requested {
            resource: self.core.id.clone(),
            constraints: candidate.iter().filter_map(|(k, v)| Some((k.clone(), Constraint::Eq { value: v.as_scalar()?.clone() }))).collect(),
        };
        let report = match self.core.mode.coerce(&self.core.id, &request) {
            Ok(report) => report,
            Err(error) => return self.core.command_rejected(Some(id), "update_parameter", &error.message),
        };
        if let Some(rejected) = report.rejected.first() {
            return self.core.command_rejected(Some(id), "update_parameter", &rejected.reason);
        }
        let value = report.applied.get(&key).cloned().unwrap_or(value);
        candidate.insert(key.clone(), value.clone());
        // RM-12, RM-27, D14: the key names the stream the change belongs to — a channel's
        // own stream at its effective instant — and that stream's channel set and rate.
        let target: Option<(Dir, usize)> = if layout {
            match ezsdr_radio::concrete(&key) {
                Some(ezsdr_radio::Concrete::Stream { rx, number }) => Some((if rx { Dir::Rx } else { Dir::Tx }, number as usize)),
                Some(ezsdr_radio::Concrete::Channel { rx, number, leaf: "antenna" }) => {
                    let dir = if rx { Dir::Rx } else { Dir::Tx };
                    let stream = self.core.mode.stream_of(&candidate, dir, number as usize);
                    (stream >= 0).then_some((dir, stream as usize))
                }
                Some(ezsdr_radio::Concrete::Channel { rx, number, leaf: "stream" }) => {
                    let dir = if rx { Dir::Rx } else { Dir::Tx };
                    let was = self.core.mode.stream_of(&before, dir, number as usize);
                    let is = self.core.mode.stream_of(&candidate, dir, number as usize);
                    if was == is {
                        None
                    } else if was < 0 {
                        Some((dir, is.max(0) as usize))
                    } else {
                        Some((dir, was as usize))
                    }
                }
                _ => None,
            }
        } else {
            Some((dir, 0))
        };
        let (dir, k) = target.unwrap_or((dir, 0));
        let channels = target.map_or_else(|| Channels::first(0), |(dir, k)| self.core.mode.stream_channels(&candidate, dir, k));
        let rate = self.core.mode.stream_rate(&candidate, dir, k);
        let n = match decimation(self.core.mcr, rate) {
            Some(n) => n as i64,
            None if channels.is_empty() => 0,
            None => return self.core.command_rejected(Some(id), "update_parameter", &format!("UR-12: {rate} S/s is no decimation of {} Hz", self.core.mcr)),
        };
        if late && !layout {
            self.late_command(Some(key.clone()), requested.unwrap_or(now), e);
        }
        let stream = stream_key(dir, k);
        // A membership change out of a stream is booked even when it empties the set — the
        // cut is what ends the running segment (RM-25) —; only a change that names no
        // stream at all (or whose stream is where the channel already was, or holds no
        // channel) takes its value alone.
        let leaves = layout && target.is_some() && matches!(ezsdr_radio::concrete(&key), Some(ezsdr_radio::Concrete::Channel { leaf: "stream", .. }));
        let empty_layout = layout && channels.is_empty() && !leaves;
        let (old, idle, planned) = {
            let line = streams.line(stream);
            // The stream's own state is read only for a stream a channel enters — stream 0
            // in this stage (D14) —: one with no channel and no line has none (RM-29).
            if !channels.is_empty() && line.is_some() {
                (line.map(|line| line.stream.config), streams.stream(stream).idle, line.is_some_and(|line| line.plan.iter().all(|segment| segment.cut.is_some())))
            } else {
                (line.map(|line| line.stream.config), false, false)
            }
        };
        let Some(old) = old else {
            // A receive side with no link has no stream: the change only takes its value.
            self.colds.insert((e, seq), (key, value));
            return;
        };
        if empty_layout {
            // A layout stream no channel is in takes its value and nothing else (RM-21): the
            // device is not asked, so the row reads no antenna back (UR-30).
            lock(&self.core.rec).applied.push(json!({ "key": key, "claimed": value, "read_back": null, "at": self.core.at(e) }));
            self.colds.insert((e, seq), (key, value));
            return;
        }
        let ratio = if n > 0 { Rational::new(n as u64, 1).expect("a decimation") } else { old.ratio };
        let config = Config { channels, ratio };
        let segment_config = match current {
            Some(config) => config,
            None => Arc::new(ColdConfig::layout_with_clock(self.core.device.host(), candidate.clone(), self.held_updates(dir))),
        };
        // RM-25, VH-4: an enable from no stream — its owner idle, nothing planned to run —
        // is configured by uhd-control itself, which opens the streamer, and is ready when
        // that configuration ends; any other change by the owner at the cut (UR-17, UR-23).
        let from_none = !channels.is_empty() && idle && planned;
        let mut ready = now;
        let mut failure: Option<DeviceError> = None;
        let version = segment_config.version();
        let mut reading = false;
        let mut stopped = false;
        if from_none {
            // UR-25: the streamer is uhd-control's from this section — the one that read the
            // owner's idle mark and the plan — until the section that books the change, on
            // every path. An owner that finds it set makes no device call.
            let state = streams.stream_mut(stream);
            state.busy = true;
            reading = state.reading;
            stopped = streams.stop.is_some();
        }
        if from_none {
            // Its owner is idle: nothing it could prune comes meanwhile, so the lock is
            // released for the device calls alone and taken again for the book. `k` is 0
            // here: a stream the layout holds a channel for is stream 0 (D14).
            drop(streams);
            // UR-29, UR-25: a loss, or `Provider::stop` read from its mark before any call,
            // ends the enable with no device call; the stop's own path books the end.
            let may_call = if reading { self.await_reading(stream) } else { !self.core.is_lost() && !stopped };
            let mut opened: Option<Result<(), DeviceError>> = None;
            let mut checked: Option<Result<(), DeviceError>> = None;
            let mut configured: Option<Result<i64, DeviceError>> = None;
            if may_call {
                let open = self.core.open_device(dir, k, channels);
                let check = if open.is_ok() && !self.core.is_lost() { self.core.check_opened(dir, channels) } else { Ok(()) };
                // The configuration in effect at `e`, the timed updates held for it included (UC-2).
                let entries: Vec<(usize, Settings)> = channels
                    .iter()
                    .map(|chan| {
                        let mut settings = self.core.mode.settings(&candidate, dir, chan as usize);
                        settings.rate = Some(rate);
                        // A layout command names its channel; a current-keys one belongs to every
                        // channel of its direction, as the projection applies it (UC-2).
                        for timed in self.held.range(..=(e, u64::MAX)).map(|(_, timed)| timed).filter(|timed| timed.dir == dir && timed.chan.is_none_or(|timed_chan| timed_chan == chan as usize)) {
                            let update = timed.settings();
                            settings.freq = update.freq.or(settings.freq);
                            settings.gain = update.gain.or(settings.gain);
                        }
                        (chan as usize, settings)
                    })
                    .collect();
                // UR-29: a loss between two channels stops the rest; `Core::configure` looks
                // before each one's apply.
                let config = if open.is_ok() && check.is_ok() && !self.core.is_lost() {
                    Some(self.core.configure(dir, &entries, rate, k))
                } else {
                    None
                };
                opened = Some(open);
                checked = Some(check);
                configured = config;
            }
            ready = self.core.now();
            streams = lock(&self.core.streams);
            // UR-13, UR-25: the record of the open and of the configuration is only what their own
            // outcomes made known.
            if let (Some(opened), Some(checked)) = (&opened, &checked) {
                streams.record_opened(stream, channels, opened, checked);
            }
            if !may_call || self.core.is_lost() || streams.stop.is_some() {
                // UR-25: the ownership goes back, and nothing of the enable is booked.
                streams.stream_mut(stream).busy = false;
                drop(streams);
                return;
            }
            failure = match (opened, checked) {
                (Some(Err(error)), _) => Some(error),
                (Some(Ok(())), Some(Err(error))) => Some(error),
                (Some(Ok(())), Some(Ok(()))) => configured.and_then(Result::err),
                _ => None,
            };
        }
        if from_none {
            streams.record_configured(stream, failure.is_none().then_some((seq, version)));
        }
        streams.stream_mut(stream).configs.insert(seq, segment_config);
        // RM-25: the configuration uhd-control's own enable applied was refused. The change is
        // booked refused in this section, as its owner books a refusal it finds — the values
        // stay the configuration of record, the stream is halted, and the next enable applies
        // them —, and then the refusal is reported.
        self.core.probe("cold_booking", if from_none { Some(&*streams) } else { None });
        if streams.book(&self.core, stream, Item { e, seq, ready, release: NO_RELEASE, delivered: None, refused: failure.is_some(), kind: Kind::Cold(config) }) {
            self.colds.insert((e, seq), (key.clone(), value));
            self.core.timing(json!({ "what": "cold_change", "key": key, "e": e, "booked_at": now, "ready": ready }));
        } else {
            streams.stream_mut(stream).configs.remove(&seq);
        }
        if from_none {
            streams.stream_mut(stream).busy = false;
        }
        drop(streams);
        if let Some(error) = failure {
            self.core.device_failed(Some(id), "update_parameter", &error);
        }
    }

    // ------------------------------------------------------------ UR-21

    #[allow(clippy::too_many_arguments)]
    fn book_burst(
        &mut self,
        id: ActionId,
        node: Option<&ezsdr_kernel::id::ResourceId>,
        addressed: Addressed,
        waveform: ezsdr_kernel::manifest::ArtifactRef,
        repeat: bool,
        at: AbsoluteDeadline,
        requested_at: Option<AbsoluteDeadline>,
        late_policy: ezsdr_kernel::stream::LatePolicy,
        metadata_empty: bool,
    ) {
        let reject = |reason: &str| self.core.command_rejected(Some(id), "tx_burst", reason);
        // KC-21a: the last transmit clock registered, while its segment has not ended.
        let current = {
            let streams = lock(&self.core.streams);
            let stream = booked_stream(Dir::Tx);
            let last = streams.line(stream).and_then(|line| line.plan.last()).filter(|segment| segment.cut.is_none());
            streams.stream(stream).clocks.last().map(|(clock, _)| *clock).zip(last.map(|segment| segment.config.channels.count() as usize))
        };
        // RM-12, D14: under the layout the target must be a transmit stream's node — a
        // stream without a channel is the same refusal as a silent one (UR-21); the
        // current keys keep their order, the clock first.
        if self.core.mode.is_layout() {
            match addressed {
                Addressed::Stream { direction: Direction::Tx, k: 0 } => {}
                Addressed::Stream { direction: Direction::Tx, .. } => return reject("UR-21: no transmit channel is configured"),
                _ => return reject("UR-21: the target is not this device's transmit stream"),
            }
            if current.is_none() {
                return reject("UR-21: no transmit channel is configured");
            }
        } else {
            if current.is_none() {
                return reject("UR-21: no transmit channel is configured");
            }
            if node != Some(&self.core.tx_id) {
                return reject("UR-21: the target is not this device's transmit stream");
            }
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
        if repeat && len as u64 > self.core.mode.repeat_max_samples() {
            return reject("UR-21: the repeated waveform exceeds radio.tx.repeat_max_samples");
        }
        if !metadata_empty {
            return reject("UR-21: TxBurst.metadata must be empty");
        }
        // RM-14 on the device lead, RM-15's origin bound (VE-2; N-P0-1).
        let lead = self.core.ticks(DEVICE_LEAD_NS);
        let now = timeline::decided_at(self.core.now(), clock.origin, lead);
        let now_k = clock.at_or_after(now);
        let decided = late_policy.decide(
            &self.core.clocks,
            at.time_point,
            TimePoint::new(clock.domain, now_k),
            Duration::new(ClockDomainId::HOST_MONOTONIC, DEVICE_LEAD_NS),
        );
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
            let payload = serde_json::to_value(TimeErrorPayload {
                cause: TimeErrorCause::Late,
                outcome,
                late_by_ns: late_by.ticks_in(ClockDomainId::HOST_MONOTONIC).unwrap_or(i64::MAX),
                target: at.time_point,
            })
            .expect("a payload");
            // The burst's stream is 0 in this stage (D14): another is refused before here.
            self.core.emit(&self.core.stream_node(Dir::Tx, 0), kinds::TIME_ERROR, Severity::Error, payload);
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
        let open = BurstOpen {
            waveform_len: u32::try_from(len).ok(),
            late: (!matches!(outcome, LateOutcome::OnTime {})).then_some(outcome),
            requested_target: requested,
        };
        let _ = self.to_tx.send(TxCmd::Burst(Held {
            k: start,
            domain: clock.domain,
            samples: Arc::new(samples),
            repeat,
            open,
            policy: late_policy,
            target: at.time_point,
        }));
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

    use super::*;
    use super::super::core::key;
    use crate::device::Device;
    use ezsdr_kernel::spec::Scalar;
    use ezsdr_kernel::stream::{BackPressure, BlockRef, DataLink, DropCarry, PublishOutcome};

    struct NoActions;
    impl ActionReceiver for NoActions {
        fn recv(&self) -> Option<ezsdr_kernel::module_api::Dispatched> { None }
    }

    /// A receive link that takes nothing, so that the receive side has a stream.
    struct NoLink;
    impl DataLink for NoLink {
        fn publish(&self, _block: BlockRef) -> PublishOutcome { PublishOutcome::Accepted }
        fn receive(&self) -> Option<(BlockRef, DropCarry)> { None }
        fn drops(&self) -> u64 { 0 }
        fn take_drop_carry(&self) -> DropCarry { DropCarry::default() }
        fn policy(&self) -> BackPressure { BackPressure::DropOldest }
    }

    fn rig() -> (Arc<Core>, Arc<crate::device::FakeDevice>, Arc<ezsdr_kernel::time::ManualTimeAuthority>) {
        let (core, device, time, _) = crate::provider::test_support::rig_with_links(vec![Arc::new(NoLink)]);
        (core, device, time)
    }

    /// uhd-control with one channel at 1 MS/s on each direction in `on` from 0.
    fn control(core: &Arc<Core>, on: &[Dir]) -> (Control, Receiver<TxCmd>) {
        let (to_tx, tx) = channel();
        let mut config = core.mode.defaults();
        let mut start = Start { t0: 0, rx: None, tx: None };
        for dir in on {
            if !core.mode.is_layout() {
                config.insert(key(&format!("radio.{}.channels", dir.name())), Value::from(1));
            }
            match dir {
                Dir::Rx => start.rx = Some((Channels::first(1), 200)),
                Dir::Tx => start.tx = Some((core.register(Dir::Tx, 200, 0).unwrap(), Channels::first(1))),
            }
        }
        (Control::new(core.clone(), Arc::new(NoActions), to_tx, config, false, start), tx)
    }

    /// The plan of `dir`'s stream as its owner reads it.
    fn plan_of(core: &Core, dir: Dir) -> Vec<Planned> {
        lock(&core.streams).planned(booked_stream(dir))
    }

    /// What a stream's line holds, for comparing it across another stream's change: its
    /// items (`e`, `seq`, `refused`), its clocks (`origin`, `ended`) and its plan
    /// (`origin`, `cut`).
    type Held = (Vec<(i64, u64, bool)>, Vec<(i64, bool)>, Vec<(i64, i64)>);

    #[test]
    fn ur_13_a_failed_open_from_no_stream_is_opened_again_by_the_owner() {
        // UR-13 (S4): an enable from no stream whose open fails records the stream's set as
        // unknown, so the next enable from no stream opens the streamer again.
        use crate::device::{FakeConfig, FakeFault};
        let config = FakeConfig { faults: vec![FakeFault::RxOpenFails], ..Default::default() };
        let (core, device, _, _) = crate::provider::test_support::rig_with(config, vec![Arc::new(NoLink)]);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        // The receive stream is stopped and its owner idle: an enable from no stream.
        control.book(Dispatched::new(ActionId(1), Action::Stop { target: Some(super::super::test_support::target_of(&core.rx_id)) }, Some(core.rx_id.clone())));
        lock(&core.streams).stream_mut(booked_stream(Dir::Rx)).idle = true;
        control.book_cold(ActionId(1), key("radio.rx.sample_rate_hz"), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(100)))));
        assert!(matches!(lock(&core.streams).stream(booked_stream(Dir::Rx)).set, super::super::core::DeviceSet::Unknown));
        lock(&core.streams).stream_mut(booked_stream(Dir::Rx)).idle = true;
        control.book_cold(ActionId(2), key("radio.rx.sample_rate_hz"), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(200)))));
        assert_eq!(device.calls().iter().filter(|call| call.starts_with("rx_open")).count(), 2, "{:?}", device.calls());
    }

    /// Books an enable of the receive stream at `ms` from no stream, the owner idle (UR-13).
    fn enable_from_none(control: &mut Control, core: &Core, id: u64, ms: i64, rate: f64) {
        lock(&core.streams).stream_mut(booked_stream(Dir::Rx)).idle = true;
        control.book_cold(ActionId(id), key("radio.rx.sample_rate_hz"), Value::num(rate).unwrap(), Some(AbsoluteDeadline::new(core.at(core.ticks(ms * 1_000_000)))));
    }

    #[test]
    fn ur_13_a_rate_refusal_from_no_stream_is_reapplied_and_starts_nothing() {
        // UR-13 (S5): the rate comparison of an enable's open refuses, and the open is redone
        // at the next enable from no stream: the stored refusal returns, so nothing is applied.
        let (core, device, _, _) = crate::provider::test_support::rig_with(crate::device::FakeConfig::default(), vec![Arc::new(NoLink)]);
        // The first comparison refuses (a rate read of a channel the device does not have).
        assert!(core.check_rates(Dir::Rx, 99).is_err());
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        control.book(Dispatched::new(ActionId(1), Action::Stop { target: Some(super::super::test_support::target_of(&core.rx_id)) }, Some(core.rx_id.clone())));
        enable_from_none(&mut control, &core, 2, 100, 2e6);
        enable_from_none(&mut control, &core, 3, 200, 2e6);
        let calls = device.calls();
        assert_eq!(calls.iter().filter(|call| call.starts_with("rx_open")).count(), 2, "{calls:?}");
        assert!(calls.iter().all(|call| !call.starts_with("apply")), "{calls:?}");
    }

    #[test]
    fn ur_24_a_timed_update_after_a_from_none_configure_is_sent_without_configuring_again() {
        // UR-24 (S11a): an update the enable's segment holds at its origin and the device
        // receives as its own timed command leaves the configuration's version standing, so the
        // owner, whose record holds the segment's command and version, starts it without
        // configuring the direction again.
        let (core, device, time) = rig();
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.channels"), Value::from(0));
        let ms = |n: i64| core.ticks(n * 1_000_000);
        channels_from_none(&mut control, &core, 2, 100, 1);
        let planned = plan_of(&core, Dir::Rx);
        let (by, version) = (planned[0].segment.by.expect("the enable"), planned[0].version().expect("its settings"));
        // Due at the segment's own origin, where the channel is in force: a timed command.
        control.book_timed(ActionId(3), key("radio.rx.gain_db"), Value::num(3.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(100)))));
        time.advance_to(core.at(ms(98))).unwrap();
        control.release();
        let timed: Vec<String> = device.calls().into_iter().filter(|call| call.starts_with("apply") && !call.ends_with("at=-")).collect();
        assert_eq!(timed.len(), 1, "{:?}", device.calls());
        assert!(timed[0].contains("gain=3"), "{timed:?}");
        let before = device.calls().len();
        let mut rx = owner_rx(&core);
        assert!(rx.begin(), "the owner begins the enable's segment");
        let owner = device.calls()[before..].to_vec();
        assert!(owner.iter().all(|call| call.starts_with("rx_start")), "the segment is not configured again: {owner:?}");
        assert_eq!(plan_of(&core, Dir::Rx)[0].version(), Some(version), "the device received the update: no new version");
        assert_eq!(lock(&core.streams).stream(booked_stream(Dir::Rx)).configured, Some((by, version)));
    }

    #[test]
    fn ur_24_a_record_only_release_after_a_from_none_configure_makes_the_owner_configure_again() {
        // UR-24 (S11b): the enable from no stream at 100 ms is configured by uhd-control; a
        // timed gain due at 50 ms, before that segment's origin, is released while no channel
        // holds it and is only recorded. The version moves on, so the owner configures the
        // segment again rather than trusting its record, and the gain reaches the device then.
        let (core, device, time) = rig();
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.channels"), Value::from(0));
        let ms = |n: i64| core.ticks(n * 1_000_000);
        channels_from_none(&mut control, &core, 2, 100, 1);
        let version = plan_of(&core, Dir::Rx)[0].version().expect("the enable's settings");
        control.book_timed(ActionId(3), key("radio.rx.gain_db"), Value::num(3.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(50)))));
        let before = device.calls().len();
        time.advance_to(core.at(ms(48))).unwrap();
        control.release();
        assert_eq!(device.calls().len(), before, "a recorded release reaches no device: {:?}", device.calls());
        assert_eq!(plan_of(&core, Dir::Rx)[0].version(), Some(version + 1), "the recorded release did not move the version");
        let mark = device.calls().len();
        let mut rx = owner_rx(&core);
        assert!(rx.begin(), "the owner begins the enable's segment");
        let owner = device.calls()[mark..].to_vec();
        assert!(owner.iter().all(|call| !call.starts_with("rx_open")), "the record holds the set: {owner:?}");
        assert_eq!(owner.iter().filter(|call| call.starts_with("apply")).count(), 1, "the segment is configured again: {owner:?}");
        assert!(owner.iter().any(|call| call.contains("gain=3")), "the recorded gain reaches the device: {owner:?}");
    }

    #[test]
    fn ur_24_a_cancelled_projected_update_makes_the_owner_configure_again() {
        // UR-24 (S11c): cancelling a held update the segment's projection at its origin holds
        // moves the version on, so the owner configures the segment again rather than trusting
        // its record; cancelling one the origin never held leaves the version and that trust
        // standing.
        let (core, _, _) = rig();
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.channels"), Value::from(0));
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        channels_from_none(&mut control, &core, 2, 1000, 1);
        let planned = plan_of(&core, Dir::Rx);
        let (by, version) = (planned[0].segment.by.expect("the enable"), planned[0].version().expect("its settings"));
        // Due after the origin: the projection at the origin never held it, so cancelling it
        // leaves the segment — and the record the enable wrote — standing.
        control.book_timed(ActionId(3), key("radio.rx.gain_db"), Value::num(4.0).unwrap(), at(1500));
        control.cancel_held("Provider::stop");
        let after = plan_of(&core, Dir::Rx)[0].version().expect("the settings");
        assert_eq!(after, version, "an update past the origin moved the version");
        assert_eq!(lock(&core.streams).owner_step(booked_stream(Dir::Rx), Channels::first(1), Some(by), Some(after)), super::super::core::Owner::Nothing);
        // Due before the origin: its cancellation moves the version on, and the owner configures
        // the segment again rather than trusting its record.
        control.book_timed(ActionId(4), key("radio.rx.gain_db"), Value::num(3.0).unwrap(), at(500));
        control.cancel_held("Provider::stop");
        let after = plan_of(&core, Dir::Rx)[0].version().expect("the settings");
        assert_eq!(after, version + 1, "the cancellation did not move the version");
        assert_eq!(lock(&core.streams).owner_step(booked_stream(Dir::Rx), Channels::first(1), Some(by), Some(after)), super::super::core::Owner::Configure);
    }

    #[test]
    fn ur_24_a_cancelled_update_of_another_layout_channel_leaves_the_version() {
        // UR-24 (S11c, layout): a held update of channel 1, due before the origin of channel 0's
        // segment, is cancelled; the segment runs channel 0 alone, so the update never reached
        // it, and its version and the owner's trust in its record stand.
        let selector = BTreeMap::from([
            (ezsdr_kernel::spec::Ident::parse("keys").unwrap(), Value::from("channel".to_owned())),
            (ezsdr_kernel::spec::Ident::parse("rx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:B:ubx:0")])),
            (ezsdr_kernel::spec::Ident::parse("tx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0")])),
        ]);
        let layout = ezsdr_radio::channels::from_selector(&selector).unwrap();
        let mode = super::super::mode::Mode::Layout(Box::new(ezsdr_radio::channels::Description::new(&layout, 2_000, &crate::profile::terms())));
        let (core, _time, _) = crate::provider::test_support::rig_on_mode(Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig::default())), vec![Arc::new(NoLink)], BTreeMap::new(), mode);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        lock(&core.streams).stream_mut(booked_stream(Dir::Rx)).idle = true;
        control.book_cold(ActionId(1), key("radio.rx.s0.sample_rate_hz"), Value::num(2e6).unwrap(), at(1000));
        let by = plan_of(&core, Dir::Rx).iter().find(|p| p.segment.by.is_some() && p.segment.config.channels.contains(0)).and_then(|p| p.segment.by).expect("channel 0's segment");
        let version = plan_of(&core, Dir::Rx).iter().find(|p| p.segment.by == Some(by)).and_then(|p| p.version()).expect("its settings");
        // The owner's record: channel 0's streamer is open, and configured for the segment.
        lock(&core.streams).record_open(booked_stream(Dir::Rx), Some(Channels::first(1)));
        lock(&core.streams).record_configured(booked_stream(Dir::Rx), Some((by, version)));
        control.book_timed(ActionId(3), key("radio.rx.c1.gain_db"), Value::num(4.0).unwrap(), at(100));
        assert_eq!(control.held.len(), 1, "the channel 1 update is held");
        control.cancel_held("Provider::stop");
        let after = plan_of(&core, Dir::Rx).iter().find(|p| p.segment.by == Some(by)).and_then(|p| p.version()).expect("the settings");
        assert_eq!(after, version, "a cancelled update of another channel moved the version");
        assert_eq!(lock(&core.streams).owner_step(booked_stream(Dir::Rx), Channels::first(1), Some(by), Some(after)), super::super::core::Owner::Nothing);
    }

    #[test]
    fn ur_17_a_prune_keeps_the_device_record_and_the_configure_mark() {
        // UR-17 (S16): pruning an ended segment leaves the stream's device record alone: its set
        // and its configure mark stay, so the next segment is not configured again for nothing.
        let (core, _, _) = rig();
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        control.book_cold(ActionId(1), key("radio.rx.sample_rate_hz"), Value::num(2e6).unwrap(), at(100));
        control.book_cold(ActionId(1), key("radio.rx.sample_rate_hz"), Value::num(1e6).unwrap(), at(200));
        let rx = booked_stream(Dir::Rx);
        let second = plan_of(&core, Dir::Rx)[2].segment.by.unwrap();
        lock(&core.streams).record_open(rx, Some(Channels::first(1)));
        lock(&core.streams).record_configured(rx, Some((second, 7)));
        lock(&core.streams).prune(rx, Some(second));
        let streams = lock(&core.streams);
        assert_eq!(streams.stream(rx).set, super::super::core::DeviceSet::Open(Channels::first(1)));
        assert_eq!(streams.stream(rx).configured, Some((second, 7)));
    }

    /// Books a change of the receive stream's channel count at `ms` from no stream (UR-13): the
    /// owner is idle, as its `read` leaves it.
    fn channels_from_none(control: &mut Control, core: &Core, id: u64, ms: i64, channels: i64) {
        lock(&core.streams).stream_mut(booked_stream(Dir::Rx)).idle = true;
        control.book_cold(ActionId(id), key("radio.rx.channels"), Value::from(channels), Some(AbsoluteDeadline::new(core.at(core.ticks(ms * 1_000_000)))));
    }

    /// The device calls `rx_open` and `rx_start` of a receive owner's begins, in order.
    fn owner_rx_calls(calls: &[String]) -> Vec<String> {
        calls.iter().filter(|c| c.starts_with("rx_open") || c.starts_with("rx_start")).cloned().collect()
    }

    /// A receive owner that begins its planned segments from no stream, its commands unread.
    fn owner_rx(core: &Arc<Core>) -> super::super::rx::Rx {
        let (to_rx, cmds) = channel();
        std::mem::forget(to_rx);
        super::super::rx::Rx::new(core.clone(), 0, cmds, None, None)
    }

    #[test]
    fn ur_13_a_from_none_segment_the_owner_never_started_is_opened_again() {
        // UR-13 (S1): uhd-control opens and configures B = {0,1} from no stream, the owner has not
        // begun it, and a later enable from no stream opens C = {0}. The owner opens B again
        // before it begins B, and C again before it begins C: the record of the last open does not
        // stand for the set of B.
        let (core, device, _) = rig();
        let (mut control, _tx) = control(&core, &[]);
        channels_from_none(&mut control, &core, 2, 100, 2);
        channels_from_none(&mut control, &core, 3, 200, 0);
        channels_from_none(&mut control, &core, 4, 300, 1);
        assert_eq!(owner_rx_calls(&device.calls()), ["rx_open 0 [0, 1]", "rx_open 0 [0]"], "{:?}", device.calls());
        let mut rx = owner_rx(&core);
        let mark = device.calls().len();
        for _ in 0..4 {
            rx.begin();
        }
        let owner = owner_rx_calls(&device.calls()[mark..]);
        assert_eq!(owner.first().map(String::as_str), Some("rx_open 0 [0, 1]"), "B opened again before it begins: {owner:?}");
        assert!(owner.iter().any(|c| c == "rx_open 0 [0]"), "C opened again before it begins: {owner:?}");
    }

    #[test]
    fn ur_25_an_owner_begins_a_from_none_enable_without_a_second_open() {
        // UR-25 (S3, receive): an enable from no stream is opened and configured by uhd-control, and
        // the owner, which finds the record holds its set and configuration, only starts the segment.
        let (core, device, _) = rig();
        let (mut control, _tx) = control(&core, &[]);
        channels_from_none(&mut control, &core, 2, 100, 1);
        let mark = device.calls().len();
        let mut rx = owner_rx(&core);
        assert!(rx.begin(), "the owner begins the enable's segment");
        let owner = device.calls()[mark..].to_vec();
        assert!(owner.iter().all(|c| !c.starts_with("rx_open") && !c.starts_with("apply")), "no second open or configure: {owner:?}");
        assert_eq!(owner.iter().filter(|c| c.starts_with("rx_start")).count(), 1, "{owner:?}");
    }

    #[test]
    fn ur_25_a_refused_receive_enable_is_booked_and_the_next_enable_starts() {
        // UR-25 (S21): an enable from no stream whose configure is refused is booked refused, and
        // the next enable from no stream opens and configures its own set: nothing of the refused
        // one is taken for it.
        let config = crate::device::FakeConfig { faults: vec![crate::device::FakeFault::RxApplyFails], ..Default::default() };
        let (core, device, _, _) = crate::provider::test_support::rig_with(config, vec![Arc::new(NoLink)]);
        let (mut control, _tx) = control(&core, &[]);
        channels_from_none(&mut control, &core, 2, 100, 1);
        assert!(lock(&core.streams).line(booked_stream(Dir::Rx)).is_some_and(|line| line.items.iter().any(|item| item.refused)), "the refused enable is booked refused");
        channels_from_none(&mut control, &core, 3, 150, 0);
        channels_from_none(&mut control, &core, 4, 200, 1);
        let calls = device.calls();
        assert_eq!(calls.iter().filter(|c| c.starts_with("rx_open")).count(), 2, "{calls:?}");
        assert!(calls.iter().filter(|c| c.starts_with("apply rx 0")).count() >= 1, "the next enable configures: {calls:?}");
        // The owner begins the next enable's segment, on the record the enable left (UR-13).
        let mut rx = owner_rx(&core);
        let mark = device.calls().len();
        assert!(rx.begin(), "the owner begins the next enable's segment");
        let owner = device.calls()[mark..].to_vec();
        assert!(owner.iter().all(|c| c.starts_with("rx_start")), "the record holds the enable: {owner:?}");
    }

    #[test]
    fn ur_14_booking_does_not_wait_for_a_future_cold_switch() {
        let (core, _, _) = rig();
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        // On its own thread, so that a booking that waited for the device instant, which
        // never comes on this clock, fails the test instead of hanging it (UR-14).
        let (done, finished) = channel();
        let at = core.at(core.ticks(1_000_000_000));
        let booking = std::thread::spawn(move || {
            control.book_cold(ActionId(1), key("radio.rx.sample_rate_hz"), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(at)));
            control.book_timed(ActionId(1), key("radio.rx.gain_db"), Value::num(3.0).unwrap(), None);
            control.release();
            let _ = done.send(());
        });
        assert!(finished.recv_timeout(Wall::from_secs(10)).is_ok(), "uhd-control waited for a device instant");
        booking.join().unwrap();
        assert_eq!(core.now(), 0, "booking never advanced to the future device instant");
        // The plan cuts the stream at 1 s's sample (1 000 000 at 1 MS/s) and starts the next
        // segment after the receive call there, 3 ms and the start lead (RM-25).
        let plan = plan_of(&core, Dir::Rx);
        assert_eq!(plan[0].segment.cut, Some(1_000_000));
        assert_eq!(plan[1].segment.origin, core.ticks(1_055_000_000));
        assert!(lock(&core.rec).applied.iter().any(|r|
            r["key"] == "radio.rx.gain_db" && r["issued"] == true && r["at"]["ticks"] == core.ticks(2_000_000)));
        assert!(lock(&core.rec).rejected.is_empty());
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
        let waveform = ezsdr_kernel::manifest::ingest_input(
            ezsdr_kernel::spec::Ident::parse("waveform").unwrap(),
            ezsdr_kernel::spec::Namespace::parse("ezsdr.input").unwrap(),
            format!("mem:{hash}"),
            &bytes,
        );
        let (core, _device, time, events) = crate::provider::test_support::rig_with_inputs(BTreeMap::from([(hash, Arc::<[u8]>::from(bytes))]));
        let (mut control, to_tx) = control(&core, &[Dir::Tx]);
        let clock = plan_of(&core, Dir::Tx)[0].clock.expect("the transmit clock");
        // Booking 10 ms in: the requested start (sample 0) is far behind the 2 ms device lead.
        time.advance_to(core.at(core.ticks(10_000_000))).unwrap();
        let requested = TimePoint::new(clock.domain, 0);
        control.book(Dispatched::new(ActionId(1), Action::TxBurst {
            target: super::super::test_support::target_of(&core.tx_id),
            waveform,
            repeat: false,
            at: AbsoluteDeadline::new(requested),
            requested_at: None,
            late_policy: ezsdr_kernel::stream::LatePolicy::SendAsapAndFlag,
            metadata: BTreeMap::new(),
        }, Some(core.tx_id.clone())));
        let verdict = events.drain(ezsdr_kernel::time::TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).into_iter().find(|e| e.kind.as_str() == kinds::TIME_ERROR).expect("the SendAsap verdict at booking");
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
    fn ur_30_the_recorded_plan_has_no_refused_segment() {
        // UR-30, RM-25: a refusal the owner booked is in the one plan `Provider::stop` records,
        // which has no segment for it.
        let (core, _, time) = rig();
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        control.book_cold(ActionId(1), key("radio.rx.sample_rate_hz"), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(100)))));
        control.book_cold(ActionId(1), key("radio.rx.sample_rate_hz"), Value::num(1e6).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(300)))));
        let refused = plan_of(&core, Dir::Rx)[1].segment.by.unwrap();
        assert!(lock(&core.streams).refuse(&core, booked_stream(Dir::Rx), refused));
        time.advance_to(core.at(ms(500))).unwrap();
        lock(&core.streams).end(&core, ms(500), false, false);
        control.finish();
        let rec = lock(&core.rec);
        let row = rec.timing.iter().find(|r| r["what"] == "plan").expect("the plan row");
        let segments: Vec<(i64, i64)> = row["rx"].as_array().unwrap().iter().map(|s| (s["origin"].as_i64().unwrap(), s["ticks_per_sample"][0].as_i64().unwrap())).collect();
        // The first segment and the second change's, at 1 MS/s; none at the refused 2 MS/s.
        assert_eq!(segments, [(0, 200), (ms(300), 200)], "{row}");
    }

    #[test]
    fn ur_25_a_cold_change_that_arrives_after_a_later_one_follows_it() {
        // RM-25, VH-2: a `cold` change never takes effect before one of its stream booked
        // earlier: it takes that one's instant, after it in the plan, and is late (UC-2).
        let (core, _, _, events) = crate::provider::test_support::rig_with_links(vec![Arc::new(NoLink)]);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        control.book_cold(ActionId(1), key("radio.rx.sample_rate_hz"), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(300)))));
        control.book_cold(ActionId(1), key("radio.rx.sample_rate_hz"), Value::num(1e6).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(100)))));
        let late: Vec<_> = events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|e| e.kind.as_str() == kinds::LATE_COMMAND).collect();
        assert_eq!(late.len(), 1);
        assert_eq!((late[0].payload["requested"]["ticks"].as_i64(), late[0].payload["applied"]["ticks"].as_i64()), (Some(ms(100)), Some(ms(300))));
        let plan = plan_of(&core, Dir::Rx);
        assert_eq!(plan.iter().map(|planned| planned.segment.by).collect::<Vec<_>>(), [None, Some(2)]);
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
            lock(&core.streams).stream_mut(booked_stream(Dir::Rx)).delivered = delivered.map(|k| (0, config, k));
            lock(&core.streams).end(&core, core.ticks(5_000_000), false, false);
            assert_eq!(plan_of(&core, Dir::Rx)[0].segment.cut, Some(cut));
        }
    }

    #[test]
    fn ur_26_a_change_booked_after_the_stop_instant_ends_its_clock() {
        // VH-2, UR-26: a transmit change uhd-control books after `Provider::stop`'s instant but
        // before it sees the stop leaves the plan at the stop, and the clock it registered
        // ends at its origin: no transmit clock stays open.
        let (core, _, time) = rig();
        let (mut control, _tx) = control(&core, &[Dir::Tx]);
        time.advance_to(core.at(core.ticks(10_000_000))).unwrap();
        control.book_cold(ActionId(1), key("radio.tx.sample_rate_hz"), Value::num(2e6).unwrap(), None);
        lock(&core.streams).end(&core, core.ticks(5_000_000), false, false);
        let clocks: Vec<_> = core.clocks.sample_clock_records().iter().map(|r| (r.origin.ticks_in(r.root).unwrap(), r.ended_at.map(|end| end.ticks_in(r.root).unwrap()))).collect();
        assert_eq!(clocks.len(), 2, "{clocks:?}");
        assert_eq!(clocks[1], (clocks[1].0, Some(clocks[1].0)), "{clocks:?}");
    }

    #[test]
    fn ur_25_later_timed_update_survives_cold_switch() {
        // UC-2, UR-25: the configuration of the segment a change begins holds the timed update
        // due before its origin, booked after the change.
        let mut observed = Vec::new();
        for dir in [Dir::Rx, Dir::Tx] {
            let (core, device, _) = rig();
            match dir { Dir::Rx => device.rx_open(0, &[0]).unwrap(), Dir::Tx => device.tx_open(0, &[0]).unwrap() }
            let (mut control, _tx) = control(&core, &[dir]);
            control.book_cold(ActionId(1), key(&format!("radio.{}.sample_rate_hz", dir.name())), Value::num(2e6).unwrap(),
                Some(AbsoluteDeadline::new(core.at(core.ticks(1_000_000_000)))));
            let gain = key(&format!("radio.{}.gain_db", dir.name()));
            control.book_timed(ActionId(1), gain.clone(), Value::num(3.0).unwrap(), None);
            control.release();
            assert_eq!(control.config[&gain], Value::num(3.0).unwrap());
            assert!(lock(&core.rec).rejected.is_empty());
            let plan = plan_of(&core, dir);
            plan[1].configure(&core, dir).unwrap();
            let settings = plan[1].settings.clone().unwrap();
            observed.push((dir.name(), settings.settings(&settings.updates.lock(), plan[1].segment.origin).gain));
        }
        assert_eq!(observed, vec![("rx", Some(3.0)), ("tx", Some(3.0))], "both cold switches must preserve the later update due before the new origin");
    }

    #[test]
    fn ur_25_switch_projects_timed_updates_in_effective_order() {
        let cold = ColdConfig::new(Settings { gain: Some(0.0), freq: Some(1e9), ..Settings::default() });
        cold.book((70, 1), 0, Settings { gain: Some(7.0), ..Settings::default() });
        cold.book((50, 2), 0, Settings { gain: Some(5.0), ..Settings::default() });
        cold.book((101, 3), 0, Settings { gain: Some(9.0), freq: Some(2e9), ..Settings::default() });
        cold.book((100, 4), 0, Settings { freq: Some(1.1e9), ..Settings::default() });
        assert_eq!(cold.settings(&cold.updates.lock(), 100).gain, Some(7.0));
        assert_eq!(cold.settings(&cold.updates.lock(), 100).freq, Some(1.1e9));
        // A command delayed by the device queue takes its actual effective order.
        cold.issued(&mut cold.updates.lock(), (50, 2), 80, 0, Settings { gain: Some(5.0), ..Settings::default() });
        assert_eq!(cold.settings(&cold.updates.lock(), 100).gain, Some(5.0));
        cold.issued(&mut cold.updates.lock(), (100, 4), 101, 0, Settings { freq: Some(1.1e9), ..Settings::default() });
        assert_eq!(cold.settings(&cold.updates.lock(), 100).freq, Some(1e9));
    }

    #[test]
    fn ur_26_a_stop_cancels_no_update() {
        // RM-16, UR-26 (spec 22, VH-6): a `Stop` of a stream or of the device cancels no held
        // timed command, and the projection of a segment booked after it keeps it; only
        // `Provider::stop` drops what is held.
        for target in ["usrp/rx", "usrp/tx", "usrp"] {
            let (core, device, _) = rig();
            device.tx_open(0, &[0]).unwrap();
            let (mut control, _tx) = control(&core, &[Dir::Rx, Dir::Tx]);
            let gain = key("radio.tx.gain_db");
            control.book_timed(ActionId(1), gain.clone(), Value::num(3.0).unwrap(), Some(AbsoluteDeadline::new(core.at(core.ticks(1_040_000_000)))));
            let node = ezsdr_kernel::id::ResourceId::parse(target).unwrap();
            control.book(Dispatched::new(ActionId(1), Action::Stop { target: Some(super::super::test_support::target_of(&node)) }, Some(node)));
            control.book_cold(ActionId(1), key("radio.tx.sample_rate_hz"), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(core.at(core.ticks(1_000_000_000)))));
            assert_eq!(control.held.len(), 1, "{target}");
            assert!(!lock(&core.rec).applied.iter().any(|r| r.get("cancelled").is_some()), "{target}");
            let plan = plan_of(&core, Dir::Tx);
            let settings = plan[1].settings.clone().unwrap();
            assert_eq!(settings.settings(&settings.updates.lock(), plan[1].segment.origin).gain, Some(3.0), "{target}");
            control.cancel_held("Provider::stop");
            assert!(control.held.is_empty());
            assert_eq!(lock(&core.rec).applied.iter().filter(|r| r["cancelled"] == "Provider::stop").count(), 1, "{target}");
        }
    }

    #[test]
    fn ur_25_cold_deadline_overflow_is_rejected_without_panic() {
        let aligned_max = i64::MAX.div_euclid(200) * 200;
        for dir in [Dir::Rx, Dir::Tx] {
            for (at, now) in [(i64::MAX, 0), (aligned_max, 0), (0, i64::MAX - 1)] {
                let (mut core, device, _) = rig();
                let time = Arc::new(ezsdr_kernel::time::ManualTimeAuthority::new(core.clocks.clone(), core.root, &[],
                    ezsdr_kernel::module_api::Pacing::Device).unwrap());
                Arc::get_mut(&mut core).unwrap().time = time.clone();
                let (mut control, _tx) = control(&core, &[dir]);
                time.advance_to(core.at(now)).unwrap();
                let config = control.config_at(i64::MAX);
                let records = core.clocks.sample_clock_records();
                let version = lock(&core.streams).version;
                control.book_cold(ActionId(1), key(&format!("radio.{}.sample_rate_hz", dir.name())), Value::num(2e6).unwrap(),
                    Some(AbsoluteDeadline::new(core.at(at))));
                assert_eq!(control.config_at(i64::MAX), config);
                assert_eq!(core.clocks.sample_clock_records(), records);
                assert!(lock(&core.rec).rejected.iter().any(|row| row["reason"].as_str().unwrap().contains("verflow")), "{:?}", lock(&core.rec).rejected);
                assert_eq!(lock(&core.streams).version, version, "the plan is unchanged");
                assert!(device.calls().is_empty());
            }
        }
        let n = Rational::new(200, 1).unwrap();
        assert_eq!(timeline::lattice(aligned_max, n).unwrap(), aligned_max);
        assert_eq!(timeline::lattice(-201, n).unwrap(), -200);
        assert_eq!(timeline::lattice(i64::MIN, n).unwrap(), i64::MIN + 8);
        // A cold disable needs its cut only, so the largest aligned instant remains valid.
        let (core, _, _) = rig();
        let (mut control, _tx) = control(&core, &[Dir::Tx]);
        control.book_cold(ActionId(1), key("radio.tx.channels"), Value::from(0), Some(AbsoluteDeadline::new(core.at(aligned_max))));
        assert_eq!(core.clocks.sample_clock_records()[0].ended_at, Some(core.at(aligned_max)));
        let plan = plan_of(&core, Dir::Tx);
        assert_eq!(plan.len(), 1);
    }

    #[test]
    fn ur_24_invalid_explicit_deadline_is_not_asap() {
        for dir in [Dir::Rx, Dir::Tx] {
            for cold in [false, true] {
                for on in [false, true] {
                    for invalid in 0..3 {
                        let (core, device, _) = rig();
                        let dirs = [dir];
                        let (mut control, _tx) = control(&core, if on { &dirs[..] } else { &[] });
                        let old = core.register(dir, 200, 0).unwrap();
                        let records = core.clocks.sample_clock_records();
                        let at = match invalid {
                            0 => TimePoint::new(ClockDomainId::HOST_MONOTONIC, 1_000_000_000),
                            1 => TimePoint::new(ClockDomainId::local(999), 10),
                            _ => TimePoint::new(old.domain, i64::MAX),
                        };
                        let config = control.config_at(i64::MAX);
                        let version = lock(&core.streams).version;
                        let name = format!("radio.{}.{}", dir.name(), if cold { "sample_rate_hz" } else { "gain_db" });
                        if cold { control.book_cold(ActionId(1), key(&name), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(at))); }
                        else { control.book_timed(ActionId(1), key(&name), Value::num(3.0).unwrap(), Some(AbsoluteDeadline::new(at))); }
                        assert!(control.held.is_empty()); assert_eq!(control.config_at(i64::MAX), config);
                        assert_eq!(lock(&core.streams).version, version, "the plan is unchanged");
                        assert_eq!(core.clocks.sample_clock_records(), records);
                        assert!(device.calls().is_empty());
                        assert!(lock(&core.rec).rejected.iter().any(|row| row["reason"].as_str().unwrap().contains("explicit deadline")));
                    }
                }
            }
        }
    }

    #[test]
    fn ur_24_valid_deadlines_and_omission_preserve_their_meaning() {
        use ezsdr_kernel::time::{ClockDomain, Rational};
        for case in 0..3 {
            let (core, _, _) = rig();
            let derived = core.clocks.allocate_id().unwrap();
            core.clocks.register(ClockDomain::derived(derived, core.root, Rational::new(2, 3).unwrap(), 0)).unwrap();
            let (mut control, _tx) = control(&core, &[Dir::Tx]);
            let (at, expected) = match case {
                0 => (None, core.ticks(DEVICE_LEAD_NS)),
                1 => (Some(AbsoluteDeadline::new(core.at(600_000))), 600_000),
                _ => (Some(AbsoluteDeadline::new(TimePoint::new(derived, 900_001))), 600_001),
            };
            control.book_timed(ActionId(1), key("radio.tx.gain_db"), Value::num(3.0).unwrap(), at);
            assert_eq!(control.held.keys().map(|order| order.0).collect::<Vec<_>>(), vec![expected]);
            assert!(lock(&core.rec).rejected.is_empty());
        }
    }

    #[test]
    fn ur_24_a_timed_update_acts_on_the_configuration_in_force_at_its_instant() {
        // #61: a timed update due before a booked change to 0 channels is issued on the
        // channel in force at its own instant, and one due after it is recorded, at its
        // instant, as having no channel; same-instant updates keep their arrival order (UC-2).
        let (core, device, time) = rig();
        device.tx_open(0, &[0]).unwrap();
        let (mut control, _tx) = control(&core, &[Dir::Tx]);
        control.book_cold(ActionId(1), key("radio.tx.channels"), Value::from(0), Some(AbsoluteDeadline::new(core.at(core.ticks(100_000_000)))));
        for (ms, gain) in [(50, 1.0), (50, 2.0), (150, 4.0)] {
            control.book_timed(ActionId(1), key("radio.tx.gain_db"), Value::num(gain).unwrap(), Some(AbsoluteDeadline::new(core.at(core.ticks(ms * 1_000_000)))));
        }
        time.advance_to(core.at(core.ticks(60_000_000))).unwrap();
        control.release();
        time.advance_to(core.at(core.ticks(160_000_000))).unwrap();
        control.release();
        let rows: Vec<_> = lock(&core.rec).applied.iter().filter(|r| r["key"] == "radio.tx.gain_db")
            .map(|r| (r["claimed"].as_f64().unwrap(), r["issued"] == true, r["note"] == "no channel")).collect();
        assert_eq!(rows, [(1.0, true, false), (2.0, true, false), (4.0, false, true)]);
        assert_eq!(device.calls().iter().filter(|call| call.starts_with("apply tx")).count(), 2, "{:?}", device.calls());
    }

    #[test]
    fn ur_24_a_direction_halted_by_a_refusal_has_no_channel() {
        // #64, RM-25: a refused change stays the configuration of record, but the direction it
        // halts has no channel for a timed update due before the enable that starts it again:
        // the update is recorded, applies nothing, and that enable's segment applies it. A
        // direction a `Stop` turned off is not halted: its update is issued.
        for (dir, halt, enable) in [(Dir::Rx, true, "cold"), (Dir::Rx, true, "start_rx"), (Dir::Tx, true, "cold"), (Dir::Rx, false, "start_rx")] {
            let case = format!("{} halt={halt} {enable}", dir.name());
            let (core, device, time) = rig();
            match dir { Dir::Rx => device.rx_open(0, &[0]).unwrap(), Dir::Tx => device.tx_open(0, &[0]).unwrap() }
            let (mut control, _tx) = control(&core, &[dir]);
            let ms = |n: i64| core.ticks(n * 1_000_000);
            let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
            let rate = key(&format!("radio.{}.sample_rate_hz", dir.name()));
            let gain = key(&format!("radio.{}.gain_db", dir.name()));
            if halt {
                control.book_cold(ActionId(1), rate.clone(), Value::num(2e6).unwrap(), at(100));
                let refused = plan_of(&core, dir)[1].segment.by.unwrap();
                assert!(lock(&core.streams).refuse(&core, booked_stream(dir), refused), "{case}");
            } else {
                control.book(Dispatched::new(ActionId(1), Action::Stop { target: Some(super::super::test_support::target_of(&core.rx_id)) }, Some(core.rx_id.clone())));
            }
            control.book_timed(ActionId(1), gain.clone(), Value::num(3.0).unwrap(), at(200));
            match enable {
                "cold" => control.book_cold(ActionId(1), rate.clone(), Value::num(1e6).unwrap(), at(300)),
                _ => control.book(Dispatched::new(ActionId(1), Action::Command {
                    target: super::super::test_support::target_of(&core.rx_id),
                    verb: ezsdr_kernel::spec::Ident::parse(ezsdr_radio::START_RX).unwrap(),
                    params: BTreeMap::new(),
                    at: at(300),
                }, Some(core.rx_id.clone()))),
            }
            time.advance_to(core.at(ms(199))).unwrap();
            control.release();
            let applied = device.calls().iter().filter(|call| call.starts_with(&format!("apply {}", dir.name())) && call.contains("gain=3")).count();
            assert_eq!(applied, usize::from(!halt), "{case}: {:?}", device.calls());
            assert!(lock(&core.rec).applied.iter().any(|r| r["key"] == gain.as_str() && (r["note"] == "no channel") == halt), "{case}");
            assert_eq!(control.config[&gain], Value::num(3.0).unwrap(), "{case}");
            if halt {
                assert_eq!(control.config_at(ms(200))[&rate], Value::num(2e6).unwrap(), "{case}: the refused value is of record");
            }
            let planned = plan_of(&core, dir).pop().unwrap();
            assert!(planned.segment.origin >= ms(300), "{case}");
            let settings = planned.settings.unwrap();
            assert_eq!(settings.settings(&settings.updates.lock(), planned.segment.origin).gain, Some(3.0), "{case}");
        }
    }

    #[test]
    fn ur_24_a_halted_direction_fills_no_queue_slot() {
        // #64, UR-24: an update for a direction halted by a refusal — from the refusal's own
        // instant on — counts no channel against the queue depth, neither as the new command
        // nor while held, so a full queue still takes it.
        let (core, device, _time) = rig();
        device.rx_open(0, &[0]).unwrap();
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let gain = key("radio.rx.gain_db");
        control.book_cold(ActionId(1), key("radio.rx.sample_rate_hz"), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(100)))));
        let refused = plan_of(&core, Dir::Rx)[1].segment.by.unwrap();
        assert!(lock(&core.streams).refuse(&core, booked_stream(Dir::Rx), refused));
        for i in 0..core.mode.timing().command_queue_depth {
            control.book_timed(ActionId(1), gain.clone(), Value::num(1.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(50) + i))));
        }
        for t in [ms(100), ms(200)] {
            control.book_timed(ActionId(1), gain.clone(), Value::num(2.0).unwrap(), Some(AbsoluteDeadline::new(core.at(t))));
        }
        assert!(lock(&core.rec).rejected.is_empty(), "{:?}", lock(&core.rec).rejected);
        assert_eq!(control.held.len(), core.mode.timing().command_queue_depth as usize + 2);
    }

    #[test]
    fn ur_24_the_device_queue_order_is_the_effective_order() {
        // UC-2 for the device's in-order queue: a command whose instant precedes one already
        // released is applied at the released one's, with LATE_COMMAND, so the instants the
        // device is handed never decrease (Review L, L07). The case is built on the controlled
        // clock: the later command released inside the 3 ms window, then an earlier one booked
        // (still a device lead ahead, so not late at receipt) and released behind it.
        let (core, device, time, events) = crate::provider::test_support::rig_with_links(vec![Arc::new(NoLink)]);
        device.rx_open(0, &[0]).unwrap();
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let us = |n: i64| core.ticks(n * 1_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(us(n))));
        let gain = key("radio.rx.gain_db");
        control.book_timed(ActionId(1), gain.clone(), Value::num(1.0).unwrap(), at(10_000));
        time.advance_to(core.at(us(7_500))).unwrap();
        control.release();
        control.book_timed(ActionId(2), gain.clone(), Value::num(2.0).unwrap(), at(9_700));
        control.release();
        let ats: Vec<i64> = device.calls().iter().filter(|c| c.starts_with("apply rx 0") && !c.contains("at=-"))
            .map(|c| c.split("at=").nth(1).unwrap().split(' ').next().unwrap().parse().unwrap()).collect();
        assert_eq!(ats, [us(10_000), us(10_000)], "the earlier command handed to the device out of order: {:?}", device.calls());
        let late: Vec<_> = events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|e| e.kind.as_str() == kinds::LATE_COMMAND).collect();
        assert_eq!(late.len(), 1, "{late:?}");
        assert_eq!((late[0].payload["requested"]["ticks"].as_i64(), late[0].payload["applied"]["ticks"].as_i64()), (Some(us(9_700)), Some(us(10_000))));
    }

    #[test]
    fn ur_24_a_late_update_halted_before_it_can_act_has_no_channel() {
        // #64, UR-24: an update late behind a released one acts at that one's instant; a refusal
        // between its own instant and then leaves it no channel, so it is recorded, not issued.
        let (core, device, time) = rig();
        device.rx_open(0, &[0]).unwrap();
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let us = |n: i64| core.ticks(n * 1_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(us(n))));
        let gain = key("radio.rx.gain_db");
        control.book_cold(ActionId(1), key("radio.rx.sample_rate_hz"), Value::num(2e6).unwrap(), at(300_000));
        let refused = plan_of(&core, Dir::Rx)[1].segment.by.unwrap();
        control.book_timed(ActionId(1), gain.clone(), Value::num(1.0).unwrap(), at(300_000));
        time.advance_to(core.at(us(297_500))).unwrap();
        control.release();
        assert!(lock(&core.streams).refuse(&core, booked_stream(Dir::Rx), refused));
        control.book_timed(ActionId(1), gain.clone(), Value::num(2.0).unwrap(), at(299_600));
        control.release();
        let rows: Vec<_> = lock(&core.rec).applied.iter().filter(|r| r["key"] == "radio.rx.gain_db")
            .map(|r| (r["claimed"].as_f64().unwrap(), r["issued"] == true, r["note"] == "no channel")).collect();
        assert_eq!(rows, [(1.0, true, false), (2.0, false, true)]);
        assert!(!device.calls().iter().any(|call| call.starts_with("apply rx") && call.contains("gain=2")), "{:?}", device.calls());
    }

    #[test]
    fn ur_25_a_receive_enable_without_a_link_is_only_recorded() {
        // UR-25 (design 33 D15): a receive side with no link has no stream, so a `cold`
        // enable of it takes its value and nothing else — no line to book on, no plan, no
        // streamer and no configuration. The link is what makes the receive stream (UR-15).
        let (core, device, _, _) = crate::provider::test_support::rig();
        let (mut control, _tx) = control(&core, &[Dir::Tx]);
        control.book_cold(ActionId(1), key("radio.rx.channels"), Value::from(1), None);
        assert!(plan_of(&core, Dir::Rx).is_empty(), "an enable without a link planned a segment");
        assert_eq!(control.config_at(i64::MAX)[&key("radio.rx.channels")], Value::from(1), "the value is not of record");
        assert!(!lock(&core.rec).timing.iter().any(|r| r["what"] == "cold_change"), "{:?}", lock(&core.rec).timing);
        assert!(lock(&core.rec).rejected.is_empty(), "{:?}", lock(&core.rec).rejected);
        assert!(device.calls().is_empty(), "{:?}", device.calls());
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
            streams.line(stream).expect("the line").items.iter().filter_map(|item| match item.kind {
                Kind::End { abort } => Some((item.e, item.seq, abort)),
                _ => None,
            }).collect()
        };
        assert_eq!(ends(rx), [(ms(5), seq, false)], "the receive line");
        assert_eq!(ends(tx), [(ms(5), seq, false)], "the transmit line");
        assert_eq!(lock(&core.streams).seq, seq, "the second end took a new arrival order");
    }

    #[test]
    fn ur_24_a_refusal_halts_only_its_own_stream() {
        // RM-25, UR-24, UR-25 (design 33 D15): a refused change halts its own stream alone —
        // the instance's other stream keeps its plan, its items and its clocks, and its timed
        // update is issued there; the halted direction's is recorded with no channel.
        for refused in [Dir::Rx, Dir::Tx] {
            let other = if refused == Dir::Rx { Dir::Tx } else { Dir::Rx };
            let case = format!("{} refused", refused.name());
            let (core, device, time) = rig();
            device.rx_open(0, &[0]).unwrap();
            device.tx_open(0, &[0]).unwrap();
            let (mut control, _tx) = control(&core, &[Dir::Rx, Dir::Tx]);
            let ms = |n: i64| core.ticks(n * 1_000_000);
            let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
            let rate = key(&format!("radio.{}.sample_rate_hz", refused.name()));
            let gain = |dir: Dir| key(&format!("radio.{}.gain_db", dir.name()));
            control.book_cold(ActionId(1), rate.clone(), Value::num(2e6).unwrap(), at(100));
            let seq = plan_of(&core, refused)[1].segment.by.unwrap();
            assert!(lock(&core.streams).refuse(&core, booked_stream(refused), seq), "{case}");
            // What the other stream holds the refusal shows nothing of.
            let snapshot = |dir: Dir| -> Held {
                let streams = lock(&core.streams);
                let key = booked_stream(dir);
                let items = streams.line(key).expect("a line").items.iter().map(|item| (item.e, item.seq, item.refused)).collect();
                let planned = streams.planned(key).into_iter().map(|planned| (planned.segment.origin, planned.segment.cut.unwrap_or(i64::MIN))).collect();
                (items, streams.stream(key).clocks.iter().map(|(clock, ended)| (clock.origin, *ended)).collect(), planned)
            };
            let before = snapshot(other);
            control.book_timed(ActionId(1), gain(Dir::Rx), Value::num(3.0).unwrap(), at(200));
            control.book_timed(ActionId(1), gain(Dir::Tx), Value::num(4.0).unwrap(), at(200));
            time.advance_to(core.at(ms(199))).unwrap();
            control.release();
            let calls = device.calls();
            let applied = |dir: Dir, gain: f64| calls.iter().filter(|c| c.starts_with(&format!("apply {} ", dir.name())) && c.contains(&format!("gain={gain}"))).count();
            let (refused_gain, other_gain) = if refused == Dir::Rx { (3.0, 4.0) } else { (4.0, 3.0) };
            assert_eq!(applied(refused, refused_gain), 0, "{case}: the halted direction applied its update: {calls:?}");
            assert_eq!(applied(other, other_gain), 1, "{case}: {:?}", calls);
            assert!(lock(&core.rec).applied.iter().any(|r| r["key"] == gain(refused).as_str() && r["note"] == "no channel"), "{case}");
            assert!(lock(&core.rec).applied.iter().any(|r| r["key"] == gain(other).as_str() && r["issued"] == true && r["at"]["ticks"] == ms(200)), "{case}");
            assert_eq!(snapshot(other), before, "{case}: the refusal changed the other stream");
        }
    }

    #[test]
    fn ur_25_a_cold_change_and_a_timed_update_at_one_instant_keep_their_own_instants() {
        // UC-2, RM-25 (design 33 D15): the arrival order is the instance's one counter, a
        // `cold` change and a timed update at one instant keep their own instants — neither
        // floors the other — and the update at the change's own instant is in its segment's
        // configuration. A timed update before the change stays where it was booked.
        for cold_first in [true, false] {
            let case = format!("cold first: {cold_first}");
            let (core, _, _) = rig();
            let (mut control, _tx) = control(&core, &[Dir::Rx, Dir::Tx]);
            let ms = |n: i64| core.ticks(n * 1_000_000);
            let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
            let (rate, rx_gain, tx_gain) = (key("radio.rx.sample_rate_hz"), key("radio.rx.gain_db"), key("radio.tx.gain_db"));
            if cold_first {
                control.book_cold(ActionId(1), rate.clone(), Value::num(2e6).unwrap(), at(300));
            }
            control.book_timed(ActionId(1), rx_gain.clone(), Value::num(3.0).unwrap(), at(300));
            if !cold_first {
                control.book_cold(ActionId(1), rate.clone(), Value::num(2e6).unwrap(), at(300));
            }
            control.book_timed(ActionId(1), tx_gain.clone(), Value::num(4.0).unwrap(), at(300));
            control.book_timed(ActionId(1), rx_gain.clone(), Value::num(2.0).unwrap(), at(100));
            assert_eq!(lock(&core.streams).seq, 4, "{case}");
            // The receive update's arrival order is 2 after the change, 1 before it.
            let rx_seq = if cold_first { 2 } else { 1 };
            let held: Vec<(i64, u64)> = control.held.keys().copied().collect();
            assert_eq!(held, [(ms(100), 4), (ms(300), rx_seq), (ms(300), 3)], "{case}");
            let planned = plan_of(&core, Dir::Rx);
            let settings = planned[1].settings.clone().unwrap();
            assert_eq!(settings.settings(&settings.updates.lock(), planned[1].segment.origin).gain, Some(3.0), "{case}");
            assert!(lock(&core.rec).rejected.is_empty(), "{case}");
        }
    }

    #[test]
    fn ur_24_the_device_queue_is_one_across_the_directions() {
        // UC-2, UR-24 (design 33 D15): the device's queue is the instance's one, so a command
        // handed over later but due earlier waits for the instant already handed over, with
        // `LATE_COMMAND`, across the directions too (UR-24's one queue).
        let (core, device, time, events) = crate::provider::test_support::rig_with_links(vec![Arc::new(NoLink)]);
        device.rx_open(0, &[0]).unwrap();
        device.tx_open(0, &[0]).unwrap();
        let (mut control, _tx) = control(&core, &[Dir::Rx, Dir::Tx]);
        let us = |n: i64| core.ticks(n * 1_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(us(n))));
        control.book_timed(ActionId(1), key("radio.tx.gain_db"), Value::num(1.0).unwrap(), at(10_000));
        time.advance_to(core.at(us(7_500))).unwrap();
        control.release();
        control.book_timed(ActionId(2), key("radio.rx.gain_db"), Value::num(2.0).unwrap(), at(9_700));
        control.release();
        let ats: Vec<i64> = device.calls().iter().filter(|c| c.starts_with("apply rx 0") && !c.contains("at=-") && c.contains("gain=2"))
            .map(|c| c.split("at=").nth(1).unwrap().split(' ').next().unwrap().parse().unwrap()).collect();
        assert_eq!(ats, [us(10_000)], "the receive command was handed over out of the queue's order: {:?}", device.calls());
        let late: Vec<_> = events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|e| e.kind.as_str() == kinds::LATE_COMMAND).collect();
        assert_eq!(late.len(), 1, "{late:?}");
        assert_eq!((late[0].payload["requested"]["ticks"].as_i64(), late[0].payload["applied"]["ticks"].as_i64()), (Some(us(9_700)), Some(us(10_000))));
    }

    #[test]
    fn ur_25_the_line_lock_is_free_inside_every_control_device_call() {
        // UR-25 (design 33 D15): uhd-control holds no line lock across its device calls —
        // a stream enabled from no stream opens and configures it, and a released timed
        // command applies it — and the spy proves the sequence passed each call.
        let spy = Arc::new(crate::provider::test_support::LockSpy::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, ..crate::device::FakeConfig::default() }));
        let (core, time, _) = crate::provider::test_support::rig_on_inputs(spy.clone(), vec![Arc::new(NoLink)], 2_000, BTreeMap::new());
        spy.watch(&core);
        let (mut control, _tx) = control(&core, &[]);
        control.book_cold(ActionId(1), key("radio.rx.channels"), Value::from(1), None);
        control.book_cold(ActionId(1), key("radio.tx.channels"), Value::from(1), None);
        control.book_timed(ActionId(1), key("radio.rx.gain_db"), Value::num(3.0).unwrap(), Some(AbsoluteDeadline::new(core.at(core.ticks(200_000_000)))));
        time.advance_to(core.at(core.ticks(199_000_000))).unwrap();
        control.release();
        for name in ["rx_open", "tx_open", "apply"] {
            assert!(spy.seen(name) > 0, "the spy never saw {name}");
        }
        assert!(spy.seen("apply") >= 3, "the enable and the release did not apply: {}", spy.seen("apply"));
    }

    #[test]
    fn ur_25_the_cold_booking_books_in_the_section_that_takes_its_instant() {
        // UR-25, #63 (O1): the instant, the arrival order and the booking are one critical
        // section of the instance's line lock, so that nothing pruned between them takes
        // effect after the change. The witness is the events the booking emits: a change the
        // floor makes late, and one that is refused, are both reported while the lock that
        // books them is held, under the current keys and under the layout alike (the layout
        // reports the late change before its refusal, VH-8). A configuration built inside that
        // section is new and unshared, so it takes no second lock (UR-25's order).
        let selector = BTreeMap::from([
            (ezsdr_kernel::spec::Ident::parse("keys").unwrap(), Value::from("channel".to_owned())),
            (ezsdr_kernel::spec::Ident::parse("rx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:B:ubx:0")])),
            (ezsdr_kernel::spec::Ident::parse("tx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0")])),
        ]);
        let layout = ezsdr_radio::channels::from_selector(&selector).unwrap();
        let modes = [
            (super::super::mode::Mode::Current(Box::new(crate::profile::ubx_description(2_000))), None, "radio.rx.sample_rate_hz", ("radio.rx.channels", Value::from(3))),
            (super::super::mode::Mode::Layout(Box::new(ezsdr_radio::channels::Description::new(&layout, 2_000, &crate::profile::terms()))), Some(("radio.rx.c0.stream", Value::from(0))), "radio.rx.s0.sample_rate_hz", ("radio.rx.c0.stream", Value::from(1))),
        ];
        for (mode, member, rate, (refused, refusal)) in modes {
            let (core, _time, witness) = crate::provider::test_support::rig_on_mode_witness(
                Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig::default())),
                vec![Arc::new(NoLink)],
                BTreeMap::new(),
                mode,
            );
            let (mut control, _tx) = control(&core, &[Dir::Rx]);
            let ms = |n: i64| core.ticks(n * 1_000_000);
            // The layout's stream 0 gets its channel first, so that its rate key is declared.
            if let Some((name, value)) = member {
                control.book_cold(ActionId(1), key(name), value, Some(AbsoluteDeadline::new(core.at(ms(50)))));
            }
            control.book_cold(ActionId(1), key(rate), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(300)))));
            // Asked behind the first change, so the stream's floor raises it: a LATE_COMMAND.
            control.book_cold(ActionId(1), key(rate), Value::num(1e6).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(100)))));
            // A refusal at its booking: more channels than the profile has, or a channel in a later stream.
            control.book_cold(ActionId(1), key(refused), refusal, Some(AbsoluteDeadline::new(core.at(ms(200)))));
            let seen = witness.events();
            for kind in [kinds::LATE_COMMAND, kinds::COMMAND_REJECTED] {
                let rows: Vec<bool> = seen.iter().filter(|(name, _)| name == kind).map(|(_, held)| *held).collect();
                assert!(!rows.is_empty(), "{kind} was never emitted: {seen:?}");
                assert!(rows.iter().all(|held| *held), "{kind} was emitted outside the section that books it: {seen:?}");
            }
        }
    }

    #[test]
    fn ur_25_the_layout_cold_change_is_built_in_its_booking_section() {
        // UR-25, #63 (O1): an accepted layout change builds its configuration inside the
        // section that books it. An accepted change emits no event, so the witness cannot see
        // it: the spy records whether the line lock is held at each host clock read, and the
        // configuration reads the clock. `Control::new` reads it too, before the spy watches.
        let selector = BTreeMap::from([
            (ezsdr_kernel::spec::Ident::parse("keys").unwrap(), Value::from("channel".to_owned())),
            (ezsdr_kernel::spec::Ident::parse("rx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:B:ubx:0")])),
            (ezsdr_kernel::spec::Ident::parse("tx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0")])),
        ]);
        let layout = ezsdr_radio::channels::from_selector(&selector).unwrap();
        let mode = super::super::mode::Mode::Layout(Box::new(ezsdr_radio::channels::Description::new(&layout, 2_000, &crate::profile::terms())));
        let spy = Arc::new(crate::provider::test_support::LockSpy::new(crate::device::FakeConfig::default()));
        let (core, _time, _) = crate::provider::test_support::rig_on_mode(spy.clone(), vec![Arc::new(NoLink)], BTreeMap::new(), mode);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        spy.watch(&core);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        // Channel 0 joins stream 0, a rate is accepted at 300 ms, and a rate asked for at
        // 100 ms is floored behind it.
        control.book_cold(ActionId(1), key("radio.rx.c0.stream"), Value::from(0), Some(AbsoluteDeadline::new(core.at(ms(50)))));
        control.book_cold(ActionId(1), key("radio.rx.s0.sample_rate_hz"), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(300)))));
        control.book_cold(ActionId(1), key("radio.rx.s0.sample_rate_hz"), Value::num(1e6).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(100)))));
        let held = spy.host_held();
        assert!(!held.is_empty(), "no layout configuration was built");
        assert!(held.iter().all(|held| *held), "a layout configuration was built outside its booking section: {held:?}");
    }

    #[test]
    fn ur_25_a_cold_change_takes_the_line_lock_once_for_its_instant_and_its_booking() {
        // UR-25, #63 (O1): the instant, the arrival order and the booking are one critical
        // section of the line lock under either key set, so an accepted change takes the lock
        // once, whatever it does between the instant and the booking. An enable from no stream
        // takes it twice: its device calls are made with the lock released between the two.
        let selector = BTreeMap::from([
            (ezsdr_kernel::spec::Ident::parse("keys").unwrap(), Value::from("channel".to_owned())),
            (ezsdr_kernel::spec::Ident::parse("rx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:B:ubx:0")])),
            (ezsdr_kernel::spec::Ident::parse("tx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0")])),
        ]);
        let layout = ezsdr_radio::channels::from_selector(&selector).unwrap();
        let modes = [
            (super::super::mode::Mode::Current(Box::new(crate::profile::ubx_description(2_000))), None, "radio.rx.sample_rate_hz"),
            (super::super::mode::Mode::Layout(Box::new(ezsdr_radio::channels::Description::new(&layout, 2_000, &crate::profile::terms()))), Some(("radio.rx.c0.stream", Value::from(0))), "radio.rx.s0.sample_rate_hz"),
        ];
        for (mode, member, rate) in modes {
            let (core, _time, _) = crate::provider::test_support::rig_on_mode(Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig::default())), vec![Arc::new(NoLink)], BTreeMap::new(), mode);
            let (mut control, _tx) = control(&core, &[Dir::Rx]);
            let streams = std::ptr::from_ref(&core.streams) as usize;
            let ms = |n: i64| core.ticks(n * 1_000_000);
            if let Some((name, value)) = member {
                control.book_cold(ActionId(1), key(name), value, Some(AbsoluteDeadline::new(core.at(ms(50)))));
            }
            crate::provider::test_support::record_locks();
            control.book_cold(ActionId(1), key(rate), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(300)))));
            let taken = crate::provider::test_support::taken_locks();
            assert_eq!(taken.iter().filter(|address| **address == streams).count(), 1, "an accepted change took the line lock {taken:?}");
        }
        let (core, _time, _) = crate::provider::test_support::rig_on_mode(Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig::default())), vec![Arc::new(NoLink)], BTreeMap::new(), super::super::mode::Mode::Current(Box::new(crate::profile::ubx_description(2_000))));
        let (mut control, _tx) = control(&core, &[]);
        let streams = std::ptr::from_ref(&core.streams) as usize;
        crate::provider::test_support::record_locks();
        control.book_cold(ActionId(1), key("radio.rx.channels"), Value::from(1), None);
        let taken = crate::provider::test_support::taken_locks();
        assert_eq!(taken.iter().filter(|address| **address == streams).count(), 2, "an enable from no stream took the line lock {taken:?}");
    }

    #[test]
    fn ur_25_a_start_rx_takes_the_line_lock_once_for_its_instant_and_its_booking() {
        // RM-12, #63 (O1): start_rx's instant, its arrival order and its booking are one critical
        // section of the line lock under either key set, and the layout's configuration is built
        // inside it, so an accepted start takes the lock once.
        let selector = BTreeMap::from([
            (ezsdr_kernel::spec::Ident::parse("keys").unwrap(), Value::from("channel".to_owned())),
            (ezsdr_kernel::spec::Ident::parse("rx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:B:ubx:0")])),
            (ezsdr_kernel::spec::Ident::parse("tx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0")])),
        ]);
        let layout = ezsdr_radio::channels::from_selector(&selector).unwrap();
        let modes = [
            (super::super::mode::Mode::Current(Box::new(crate::profile::ubx_description(2_000))), None),
            (super::super::mode::Mode::Layout(Box::new(ezsdr_radio::channels::Description::new(&layout, 2_000, &crate::profile::terms()))), Some(("radio.rx.c0.stream", Value::from(0)))),
        ];
        for (mode, member) in modes {
            let (core, _time, _) = crate::provider::test_support::rig_on_mode(Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig::default())), vec![Arc::new(NoLink)], BTreeMap::new(), mode);
            let (mut control, _tx) = control(&core, &[Dir::Rx]);
            let streams = std::ptr::from_ref(&core.streams) as usize;
            let ms = |n: i64| core.ticks(n * 1_000_000);
            let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
            let node = core.stream_node(Dir::Rx, 0);
            if let Some((name, value)) = member {
                control.book_cold(ActionId(1), key(name), value, at(50));
            }
            crate::provider::test_support::record_locks();
            control.book(Dispatched::new(ActionId(1), Action::Command {
                target: super::super::test_support::target_of(&node),
                verb: ezsdr_kernel::spec::Ident::parse(ezsdr_radio::START_RX).unwrap(),
                params: BTreeMap::new(),
                at: at(300),
            }, Some(node)));
            let taken = crate::provider::test_support::taken_locks();
            assert_eq!(taken.iter().filter(|address| **address == streams).count(), 1, "an accepted start_rx took the line lock {taken:?}");
            assert!(lock(&core.streams).line(booked_stream(Dir::Rx)).unwrap().items.iter().any(|item| matches!(item.kind, Kind::Start)), "the start was not booked");
        }
    }

    #[test]
    fn ur_17_a_stream_s_prune_leaves_the_other_stream_alone() {
        // UR-17, RM-26, RM-34 (design 33 D15): pruning one stream's ended segment folds that
        // stream's items and configurations and nothing of the instance's other stream: the
        // transmit stream keeps its item and its configuration, and its command can still be
        // refused.
        let (core, _, _) = rig();
        let (mut control, _tx) = control(&core, &[Dir::Rx, Dir::Tx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        control.book_cold(ActionId(1), key("radio.rx.sample_rate_hz"), Value::num(2e6).unwrap(), at(100));
        control.book_cold(ActionId(1), key("radio.rx.sample_rate_hz"), Value::num(1e6).unwrap(), at(200));
        control.book_cold(ActionId(1), key("radio.tx.sample_rate_hz"), Value::num(2e6).unwrap(), at(100));
        let (rx, tx) = (booked_stream(Dir::Rx), booked_stream(Dir::Tx));
        let first = plan_of(&core, Dir::Rx)[1].segment.by.unwrap();
        let second = plan_of(&core, Dir::Rx)[2].segment.by.unwrap();
        let tx_seq = plan_of(&core, Dir::Tx)[1].segment.by.unwrap();
        // The receive segment the second change began has ended: the items before it fold.
        lock(&core.streams).prune(rx, Some(second));
        {
            let streams = lock(&core.streams);
            assert!(streams.line(rx).unwrap().items.iter().all(|item| item.seq != first), "the receive change was not pruned");
            assert!(!streams.stream(rx).configs.contains_key(&first), "the pruned receive configuration stays");
            assert!(streams.line(tx).unwrap().items.iter().any(|item| item.seq == tx_seq), "the transmit change was pruned with the receive stream");
            assert!(streams.stream(tx).configs.contains_key(&tx_seq), "the transmit configuration was pruned with the receive stream");
        }
        assert!(lock(&core.streams).refuse(&core, tx, tx_seq), "the transmit stream's command can no longer be refused");
    }

    // ------------------------------------------------------------ 1b-4d-1b: ownership, loss and stop

    /// The manual-clock `LockSpy` rig the ownership series runs on: the device's calls are
    /// gated and counted, and `links` gives the receive side a stream.
    fn spy_rig(links: bool) -> (Arc<Core>, Arc<crate::provider::test_support::LockSpy>, Arc<ezsdr_kernel::time::ManualTimeAuthority>) {
        use crate::ClockMode;
        use crate::device::FakeConfig;
        let spy = Arc::new(crate::provider::test_support::LockSpy::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() }));
        let links: Vec<Arc<dyn DataLink>> = if links { vec![Arc::new(NoLink)] } else { Vec::new() };
        let (core, time, _) = crate::provider::test_support::rig_on(spy.clone(), links, 2_000);
        (core, spy, time)
    }

    /// Books an enable of the receive stream's `channels` at `ms` from no stream, its owner
    /// idle, through the Action path (uhi-control's own booking).
    fn rx_enable_from_none(control: &mut Control, core: &Core, id: u64, ms: i64, channels: i64) {
        lock(&core.streams).stream_mut(booked_stream(Dir::Rx)).idle = true;
        control.book(Dispatched::new(ActionId(id), Action::UpdateParameter {
            target: super::super::test_support::target_of(&core.id),
            key: key("radio.rx.channels"),
            value: Value::from(channels),
            class: UpdateClass::Cold,
            at: Some(AbsoluteDeadline::new(core.at(core.ticks(ms * 1_000_000)))),
        }, Some(core.id.clone())));
    }

    /// How many `cold` changes the receive line holds (RM-25): an enable that books nothing
    /// leaves none, whatever the plan shows once an end has taken effect.
    fn rx_colds(core: &Core) -> usize {
        lock(&core.streams).line(booked_stream(Dir::Rx)).map_or(0, |line| line.items.iter().filter(|item| matches!(item.kind, Kind::Cold(_))).count())
    }

    /// The receive record, as the ownership tests read it.
    fn rx_record(core: &Core) -> (bool, bool, super::super::core::DeviceSet, Option<(u64, u64)>) {
        let streams = lock(&core.streams);
        let state = streams.stream(booked_stream(Dir::Rx));
        (state.busy, state.reading, state.set, state.configured)
    }

    #[test]
    fn ur_25_an_owner_waiting_behind_a_from_none_open_starts_nothing() {
        // UR-25 (S9, receive): while uhd-control's enable from no stream owns the streamer,
        // uhd-rx's turn makes no device call and moves neither its idle mark nor its cursor.
        // The first turn after the booking opens B's set again: the record holds C's.
        let (core, spy, _time) = spy_rig(true);
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.channels"), Value::from(0));
        channels_from_none(&mut control, &core, 2, 100, 2);
        channels_from_none(&mut control, &core, 3, 200, 0);
        let gate = spy.stop_before("rx_open");
        let host = core.device.host();
        let core_ = core.clone();
        let (done, finished) = channel();
        let control_thread = host.spawn("control", move || {
            let mut control = control;
            rx_enable_from_none(&mut control, &core_, 4, 300, 1);
            let _ = done.send(());
        }).expect("a participant thread");
        gate.wait();
        let mut rx = owner_rx(&core);
        let cursor = rx.cursor();
        let mark = spy.log().len();
        assert!(!rx.begin(), "the owner began a segment while uhd-control owns the streamer");
        assert_eq!(rx.cursor(), cursor, "the owner moved its cursor");
        assert!(lock(&core.streams).stream(booked_stream(Dir::Rx)).idle, "the owner moved its idle mark");
        assert_eq!(spy.log().len(), mark, "the owner called the device: {:?}", spy.log());
        gate.release();
        assert!(finished.recv_timeout(Wall::from_secs(10)).is_ok(), "the enable did not book");
        control_thread.join().expect("the control thread");
        let mark = spy.log().len();
        assert!(rx.begin(), "the owner begins B's segment");
        let opens: Vec<String> = spy.log()[mark..].iter().filter(|call| call.starts_with("rx_open")).cloned().collect();
        assert_eq!(opens.first().map(String::as_str), Some("rx_open 0 [0, 1]"), "the record's set is not C's: {opens:?}");
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
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.channels"), Value::from(0));
        channels_from_none(&mut control, &core, 2, 100, 1);
        lock(&core.streams).record_open(booked_stream(Dir::Rx), None);
        core.open_streamer(Dir::Rx, 0, Channels::first(1)).expect("the owner's open");
        let seen = during.lock().unwrap().clone();
        assert_eq!(seen, [super::super::core::DeviceSet::Unknown, super::super::core::DeviceSet::Unknown], "an open saw the set it was opening");
        assert_eq!(rx_record(&core).2, super::super::core::DeviceSet::Open(Channels::first(1)), "the open is recorded after its return");
        assert!(spy.seen("rx_open") >= 2, "the spy saw no open");
    }

    #[test]
    fn ur_25_a_from_none_holds_the_streamer_from_its_decision_to_its_booking() {
        // UR-25 (S24): an enable from no stream is uhd-control's from the section that decides
        // it through every device call to the booking itself.
        let (core, _spy, _time) = spy_rig(true);
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        for name in ["open", "rates", "apply"] {
            let seen = seen.clone();
            let weak = Arc::downgrade(&core);
            crate::provider::test_support::probe(&core, name, move |_| {
                if let Some(core) = weak.upgrade() {
                    seen.lock().unwrap().push(lock(&core.streams).stream(booked_stream(Dir::Rx)).busy);
                }
            });
        }
        {
            let seen = seen.clone();
            crate::provider::test_support::probe(&core, "cold_booking", move |streams| {
                seen.lock().unwrap().push(streams.expect("the booking's line").stream(booked_stream(Dir::Rx)).busy);
            });
        }
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.channels"), Value::from(0));
        channels_from_none(&mut control, &core, 2, 100, 1);
        let observed = seen.lock().unwrap().clone();
        assert_eq!(observed, [true, true, true, true], "an enable's section was not the streamer's: {observed:?}");
        let (busy, reading, _set, configured) = rx_record(&core);
        assert!(!busy && !reading, "the stream stayed uhd-control's");
        let planned = plan_of(&core, Dir::Rx);
        assert_eq!(configured, planned[0].segment.by.zip(planned[0].version()), "the configuration's own mark is not recorded");
    }

    #[test]
    fn ur_25_an_enable_behind_an_owner_s_open_is_a_cold_change() {
        // UR-25 (S24's reverse, the brief's): the owner has begun a segment — its idle mark is
        // false and its open is out — when uhd-control takes an enable of the same stream. It
        // opens and configures nothing, and books the change as any other cold change.
        let (core, spy, _time) = spy_rig(true);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let gate = spy.stop_before("rx_open");
        let host = core.device.host();
        let core_ = core.clone();
        let owner = host.spawn("rx", move || {
            let (to_rx, cmds) = channel();
            std::mem::forget(to_rx);
            let mut rx = super::super::rx::Rx::new(core_.clone(), 0, cmds, None, None);
            rx.begin();
        }).expect("a participant thread");
        gate.wait();
        assert!(!lock(&core.streams).stream(booked_stream(Dir::Rx)).idle, "the owner has not begun");
        let mark = spy.log().len();
        let planned = plan_of(&core, Dir::Rx).len();
        control.book_cold(ActionId(1), key("radio.rx.channels"), Value::from(1), Some(AbsoluteDeadline::new(core.at(core.ticks(100_000_000)))));
        assert_eq!(spy.log().len(), mark, "uhd-control called the device while the owner owns the streamer: {:?}", spy.log());
        let after_book = lock(&core.streams).stream(booked_stream(Dir::Rx)).idle;
        assert!(!after_book, "the enable's booking left the owner idle");
        assert_eq!(plan_of(&core, Dir::Rx).len(), planned + 1, "the enable was not booked as a cold change");
        gate.release();
        owner.join().expect("the owner thread");
        let after: Vec<String> = spy.log()[mark..].to_vec();
        assert!(after.iter().any(|call| call == "rx_open 0 [0]"), "the owner's own open did not happen: {after:?}");
        assert_eq!(rx_record(&core).2, super::super::core::DeviceSet::Open(Channels::first(1)), "the owner's open is not recorded: {after:?}");
    }

    #[test]
    fn ur_25_a_lost_stream_starts_no_rate_check_apply_or_booking_after_the_call() {
        // UR-25, UR-29 (S26): the open returned Ok and the device was recorded lost before
        // uhd-control's next look. The enable does no rate check, applies nothing and books
        // nothing; the open is recorded, and the ownership goes back.
        let (core, spy, _time) = spy_rig(true);
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.channels"), Value::from(0));
        let gate = spy.stop_after("rx_open");
        let host = core.device.host();
        let core_ = core.clone();
        let (done, finished) = channel();
        let control_thread = host.spawn("control", move || {
            let mut control = control;
            rx_enable_from_none(&mut control, &core_, 2, 100, 1);
            let _ = done.send(());
        }).expect("a participant thread");
        gate.wait();
        core.device_lost("test");
        gate.release();
        assert!(finished.recv_timeout(Wall::from_secs(10)).is_ok(), "the enable did not end");
        control_thread.join().expect("the control thread");
        assert_eq!((spy.seen("rates"), spy.seen("apply")), (0, 0), "a lost enable went on: {:?}", spy.log());
        assert_eq!(rx_colds(&core), 0, "a lost enable booked a change");
        let (busy, _reading, set, configured) = rx_record(&core);
        assert!(!busy, "the stream stayed uhd-control's");
        assert_eq!((set, configured), (super::super::core::DeviceSet::Open(Channels::first(1)), None), "the open is not recorded as the device returned it");
    }

    #[test]
    fn ur_25_a_from_none_leaves_the_stream_unbusy_on_every_path() {
        // UR-25 (S27 a–d): an enable from no stream gives the streamer back on its success, a
        // failed open, a refused rate comparison and a failed configure alike.
        use crate::ClockMode;
        use crate::device::{FakeConfig, FakeFault};
        for case in ["ok", "open fails", "rate refused", "configure fails"] {
            let config = FakeConfig {
                clock: ClockMode::Manual,
                faults: match case {
                    "open fails" => vec![FakeFault::RxOpenFails],
                    "configure fails" => vec![FakeFault::RxApplyFails],
                    _ => Vec::new(),
                },
                ..FakeConfig::default()
            };
            let (core, _device, _, _) = crate::provider::test_support::rig_with(config, vec![Arc::new(NoLink)]);
            if case == "rate refused" {
                assert!(core.check_rates(Dir::Rx, 99).is_err());
            }
            let (mut control, _tx) = control(&core, &[]);
            control.config.insert(key("radio.rx.channels"), Value::from(0));
            channels_from_none(&mut control, &core, 2, 100, 1);
            let (busy, reading, set, configured) = rx_record(&core);
            assert!(!busy && !reading, "{case}: the stream stayed uhd-control's");
            let open = super::super::core::DeviceSet::Open(Channels::first(1));
            let expected = if case == "ok" {
                (open, true)
            } else if case == "configure fails" {
                (open, false)
            } else {
                (super::super::core::DeviceSet::Unknown, false)
            };
            assert_eq!((set, configured.is_some()), expected, "{case}: the record is not its path's");
        }
    }

    #[test]
    fn ur_29_a_lost_device_stops_a_from_none_enable_between_channels() {
        // UR-29, UR-25 (S27 e): the device is lost while a two-channel configuration applies
        // its first channel. The second is not applied, the enable books nothing, and the
        // ownership goes back.
        let (core, spy, _time) = spy_rig(true);
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.channels"), Value::from(0));
        let gate = spy.stop_after("apply");
        let host = core.device.host();
        let core_ = core.clone();
        let (done, finished) = channel();
        let control_thread = host.spawn("control", move || {
            let mut control = control;
            rx_enable_from_none(&mut control, &core_, 2, 100, 2);
            let _ = done.send(());
        }).expect("a participant thread");
        gate.wait();
        core.device_lost("test");
        gate.release();
        assert!(finished.recv_timeout(Wall::from_secs(10)).is_ok(), "the enable did not end");
        control_thread.join().expect("the control thread");
        assert_eq!(spy.seen("apply"), 1, "the lost device applied another channel: {:?}", spy.log());
        assert_eq!(rx_colds(&core), 0, "a lost enable booked a change");
        let (busy, _reading, set, configured) = rx_record(&core);
        assert!(!busy, "the stream stayed uhd-control's");
        assert_eq!((set, configured), (super::super::core::DeviceSet::Open(Channels::first(2)), None), "the record is not what the calls made known");
    }

    #[test]
    fn ur_26_an_enable_from_no_stream_a_stop_recorded_after_its_open_books_nothing() {
        // UR-26, UR-25: the Provider's stop is recorded while the enable's open is out. The
        // enable goes on to no rate check and no configure, books nothing, and gives the
        // streamer back.
        let (core, spy, _time) = spy_rig(true);
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.channels"), Value::from(0));
        let gate = spy.stop_after("rx_open");
        let host = core.device.host();
        let core_ = core.clone();
        let (done, finished) = channel();
        let control_thread = host.spawn("control", move || {
            let mut control = control;
            rx_enable_from_none(&mut control, &core_, 2, 100, 1);
            let _ = done.send(());
        }).expect("a participant thread");
        gate.wait();
        {
            // As `Provider::stop` books it before its joins.
            let mut streams = lock(&core.streams);
            let at = core.now();
            streams.stop = Some((at, ezsdr_kernel::module_api::StopMode::Orderly));
            streams.end(&core, at, false, false);
        }
        gate.release();
        assert!(finished.recv_timeout(Wall::from_secs(10)).is_ok(), "the enable did not end");
        control_thread.join().expect("the control thread");
        // The calls already out are not recalled (UR-25 reads the stop at the region that
        // would book): nothing of the enable is recorded, and the streamer goes back.
        assert_eq!(rx_colds(&core), 0, "a stopped enable booked a change");
        let (busy, _reading, set, configured) = rx_record(&core);
        assert!(!busy && set == super::super::core::DeviceSet::Open(Channels::first(1)) && configured.is_none(), "the record is not what the open made known");
    }

    #[test]
    fn ur_25_an_enable_from_no_stream_after_the_provider_s_stop_calls_no_device() {
        // UR-25, UR-26 (S27's stop before the calls): the Provider's stop is recorded before
        // the enable is taken; no device call is made and nothing is booked.
        let (core, spy, _time) = spy_rig(true);
        let before = spy.log().len();
        {
            // As `Provider::stop` books it before its joins.
            let mut streams = lock(&core.streams);
            let at = core.now();
            streams.stop = Some((at, ezsdr_kernel::module_api::StopMode::Orderly));
            streams.end(&core, at, false, false);
        }
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.channels"), Value::from(0));
        channels_from_none(&mut control, &core, 2, 100, 1);
        assert_eq!(spy.log().len(), before, "a stopped Provider's enable called the device: {:?}", spy.log());
        assert_eq!(rx_colds(&core), 0, "a stopped Provider's enable booked a change");
        let (busy, reading, _set, _configured) = rx_record(&core);
        assert!(!busy && !reading, "the stream stayed uhd-control's");
    }

    #[test]
    fn ur_29_a_lost_stream_between_the_rate_check_and_the_configure_applies_nothing() {
        // UR-29, UR-25: the device is lost while the rate comparison runs; the enable enters no
        // configuration, books nothing, and gives the streamer back.
        let (core, spy, _time) = spy_rig(true);
        let applies = Arc::new(std::sync::Mutex::new(0usize));
        {
            let applies = applies.clone();
            crate::provider::test_support::probe(&core, "apply", move |_| *applies.lock().unwrap() += 1);
        }
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.channels"), Value::from(0));
        let gate = spy.stop_after("rates");
        let host = core.device.host();
        let core_ = core.clone();
        let (done, finished) = channel();
        let control_thread = host.spawn("control", move || {
            let mut control = control;
            rx_enable_from_none(&mut control, &core_, 2, 100, 1);
            let _ = done.send(());
        }).expect("a participant thread");
        gate.wait();
        core.device_lost("test");
        gate.release();
        assert!(finished.recv_timeout(Wall::from_secs(10)).is_ok(), "the enable did not end");
        control_thread.join().expect("the control thread");
        assert_eq!(*applies.lock().unwrap(), 0, "a lost enable entered a configuration: {:?}", spy.log());
        assert_eq!(rx_colds(&core), 0, "a lost enable booked a change");
        let (busy, _reading, set, configured) = rx_record(&core);
        assert!(!busy && set == super::super::core::DeviceSet::Open(Channels::first(1)) && configured.is_none(), "the record is not what the open made known");
    }

    #[test]
    #[should_panic(expected = "under the line lock")]
    fn ur_25_a_configuration_taken_under_the_line_lock_is_refused() {
        // UR-25 (S28, B5): the order is the configuration's `updates`, then the line; a
        // configure that holds the line already is refused by the test build's check.
        let (core, _device, _time) = rig();
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        control.book_cold(ActionId(1), key("radio.rx.sample_rate_hz"), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(core.at(core.ticks(100_000_000)))));
        let planned = plan_of(&core, Dir::Rx)[1].clone();
        let _line = lock(&core.streams);
        let _ = planned.configure(&core, Dir::Rx);
    }
}
