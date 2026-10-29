//! uhd-control (UR-14): drains the `ActionReceiver` with MA-14b's loop, books each
//! Action — a stream's commands on its timeline, which it shares with the stream's owner —
//! releases held timed commands, and every 500 ms checks the reference and reads the
//! device's time. It never waits for a device instant.

use std::collections::BTreeMap;
use std::sync::mpsc::Sender;
use std::sync::atomic::{AtomicBool, Ordering};
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

use super::core::{assert_line_free, stream_key, Clock, Core, Holder, lock};
#[cfg(test)]
use super::core::booked_stream;
use super::mode::{DescriptionExt, scalar_number};
use ezsdr_radio::Addressed;
use super::tx::{Held, TxCmd};
use crate::device::{DeviceError, Dir, Iq, Settings, decimation};
use crate::host::{ClockMutex, HostClock};
use crate::profile::{DELIVERY_ALLOWANCE_NS, DEVICE_LEAD_NS};

const POLL: Wall = Wall::from_millis(1);
const CHECK: Wall = Wall::from_millis(500);

/// A held `hardware_timed` command (UR-24), of the channel its key names under the layout.
#[derive(Clone)]
struct Timed {
    key: Key,
    dir: Dir,
    value: f64,
    /// The logical channel the key names.
    chan: usize,
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

/// A held `hardware_timed` command the release took up, before its instant comes (UR-24):
/// `live` says whether a channel was in force at that instant (a later segment's channel
/// records `note: "no channel"`), `given` whether the device was handed it at all — one with
/// no channel to give it on is recorded `no channel` without `issued` —, and `handed_at` the
/// device's time read just before the last channel's apply, which is RM-16's hand-over.
struct Given {
    timed: Timed,
    live: bool,
    given: bool,
    handed_at: Option<i64>,
}

/// How a released receive frequency reaches the device (UR-24): as a timed command, untimed
/// at once because the destination segment was configured already, or not at all because
/// that segment's configure will give it.
enum Reach {
    Timed,
    Untimed,
    Deferred,
}

/// What became of a command the release took up (UR-24, UR-29): given to the device — with
/// the device's time read just before its last channel's apply —, refused by it (no row, and
/// the queue slot stays until the instant), or not given because another thread found the
/// device lost (it stays held for the end's settlement, design 38 D2 (4)).
enum Handed {
    Given(i64),
    Refused,
    Lost,
}

/// The configuration a command's segment begins with, shared with its owner: the one in
/// effect at the segment's origin (UC-2's projection, UR-17), its rate and channel set the
/// segment's. Updates are projected in effective order, even when admitted after booking.
pub(crate) struct ColdConfig {
    base: BTreeMap<Key, Value>,
    /// §4.7: the clock lock — a shared lock held across the device call in `configure`, so a
    /// waiting participant is idle and the manual clock advances while the holder waits.
    updates: ClockMutex<BTreeMap<(i64, u64), (usize, Settings)>>,
    /// UR-25: whether the device was configured from this segment's projection. `configure`
    /// sets it while it still holds `updates`, after the call that read the projection returned,
    /// so a booking either is in what the device was given or waits for the lock and finds this
    /// set; uhd-control's enable from no stream sets it when its own configuration succeeded.
    configured: AtomicBool,
}

impl ColdConfig {
    #[cfg(test)]
    pub fn new(base: BTreeMap<Key, Value>) -> Self {
        Self {
            base,
            updates: HostClock::real().clock_mutex(BTreeMap::new()),
            configured: AtomicBool::new(false),
        }
    }

    /// The numbered configuration at the segment's origin (RM-30, UR-12), with held timed
    /// commands projected into it. The caller copies those commands before the line lock.
    pub(crate) fn with_clock(clock: HostClock, base: BTreeMap<Key, Value>, updates: BTreeMap<(i64, u64), (usize, Settings)>) -> Self {
        Self { base, updates: clock.clock_mutex(updates), configured: AtomicBool::new(false) }
    }

    /// Test only: whether the projection's lock is held now.
    #[cfg(test)]
    pub(crate) fn updates_held(&self) -> bool {
        self.updates.is_held()
    }

    /// Test only: the threads waiting for the projection's lock.
    #[cfg(test)]
    pub(crate) fn updates_waiters(&self) -> usize {
        self.updates.waiters()
    }

    /// UR-25: whether the device was configured from this segment's projection (see `configured`).
    pub(crate) fn is_configured(&self) -> bool {
        self.configured.load(Ordering::SeqCst)
    }

    fn book(&self, order: (i64, u64), chan: usize, settings: Settings) {
        assert_line_free("a configuration's updates");
        self.updates.lock().insert(order, (chan, settings));
    }

    /// The configuration in effect at `origin` on logical channel `chan` (UR-12, RM-30):
    /// the layout's own channel values, with the updates due by then that name it.
    fn settings_on(&self, core: &Core, updates: &BTreeMap<(i64, u64), (usize, Settings)>, dir: Dir, k: usize, origin: i64, chan: usize) -> Settings {
        let mut settings = core.description.settings(&self.base, dir, k, chan);
        for (update_chan, update) in updates.range(..=(origin, u64::MAX)).map(|(_, update)| update) {
            if *update_chan != chan { continue; }
            if let Some(freq) = update.freq { settings.freq = Some(freq); }
            if let Some(gain) = update.gain { settings.gain = Some(gain); }
        }
        settings
    }

    /// Configures the direction for `segment` (UR-12), each of its channels' own settings;
    /// returns the decimation.
    pub fn configure(&self, core: &Core, dir: Dir, segment: &Segment, k: usize) -> Result<i64, DeviceError> {
        // Hold the snapshot lock across the call: an update cannot change the
        // projection while an owner is configuring from it (§4.7: the clock lock).
        assert_line_free("a configuration's updates");
        let updates = self.updates.lock();
        let ratio = segment.config.ratio;
        let rate = core.mcr as f64 * ratio.den() as f64 / ratio.num() as f64;
        let channels: Vec<(usize, Settings)> = segment
            .config
            .channels
            .iter()
            .map(|chan| {
                let mut settings = self.settings_on(core, &updates, dir, k, segment.origin, chan as usize);
                settings.rate = Some(rate);
                (chan as usize, settings)
            })
            .collect();
        let configured = core.configure(dir, &channels, rate, k);
        // Still holding `updates`: a booking made while the call ran waits for the lock and finds this.
        core.probe("mark_publishing", None);
        if configured.is_ok() {
            self.configured.store(true, Ordering::SeqCst);
        }
        configured
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
    /// The configuration at the Run's start, with every `hardware_timed` value whose instant has
    /// passed since, released or without a channel to give it on (UR-24): what a segment
    /// beginning now starts from.
    pub config: BTreeMap<Key, Value>,
    pub reference_monitored: bool,
    /// The Run's T0 (UR-15): the instant `start` began the device's streams at, which a
    /// layout `cold` change of the receive side is floored at whatever its line holds,
    /// as MockRadio's is (RM-25).
    t0: i64,
    /// The booked `cold` values, by effective instant and arrival (UC-2).
    colds: BTreeMap<(i64, u64), (Key, Value)>,
    held: BTreeMap<(i64, u64), Timed>,
    /// The commands given to the device, by the instant each takes effect and its arrival
    /// (UR-24): a segment configuration made afterwards holds each at that instant, not from
    /// the booking (`held_updates`), and each records its row when its instant comes; `live`
    /// says whether a channel was in force then, so that a later segment's channel records
    /// `note: "no channel"`.
    released_at: BTreeMap<(i64, u64), Given>,
    released: Vec<i64>,
    /// Every booked command's instant that has not passed (UR-24, design 38 D2 (1), (2)):
    /// the floor a later booking takes and the queue's depth, whether the command is held,
    /// given or was refused by the device.
    booked: Vec<i64>,
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
    pub fn new(
        core: Arc<Core>,
        actions: Arc<dyn ActionReceiver>,
        to_tx: Vec<Sender<TxCmd>>,
        config: BTreeMap<Key, Value>,
        reference_monitored: bool,
        start: Start,
    ) -> Control {
        let host = core.device.host();
        // RM-25, VH-4: this Module's terms — each direction's start lead, and, for a receive
        // segment after a cut, the receive call in progress there and the delivery
        // allowance (UR-25). The call term is each receive stream's own packet size.
        let allowance = core.ticks(DELIVERY_ALLOWANCE_NS);
        let line = |key: StreamKey, origin: i64, channels: Channels, n: i64, call: i64| Line::new(timeline::Stream {
            direction: key.0,
            origin,
            config: Config { channels, ratio: Rational::new(n.max(1) as u64, 1).expect("a positive ratio") },
            lead: core.ticks(ezsdr_radio::timeline::start_lead_ns(&core.description.timing, key.0)),
            call,
            allowance,
        });
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
                streams.starting(stream_key(Dir::Rx, k), start.rx.get(k).copied().flatten().is_some());
            }
            for (k, line) in tx.iter().enumerate() {
                streams.stream_mut(stream_key(Dir::Tx, k)).clocks = start.tx.get(k).copied().flatten().map(|(clock, _)| (clock, false)).into_iter().collect();
                streams.lines.insert(stream_key(Dir::Tx, k), line.clone());
                streams.starting(stream_key(Dir::Tx, k), start.tx.get(k).copied().flatten().is_some());
            }
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
            released_at: BTreeMap::new(),
            released: Vec::new(),
            booked: Vec::new(),
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

    /// UR-26: `Provider::stop`, or a loss, which has booked the streams' end, settles every
    /// command still pending at its own instant `S` (UR-30's plans are recorded).
    fn finish(&mut self) {
        let (at, lost) = lock(&self.core.streams).ended_at().expect("UR-26: the end was booked before uhd-control finishes");
        let why = if lost { "UR-29: device lost" } else { "Provider::stop" };
        self.settle(at, why, lost);
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
            Action::UpdateParameter { key, value, class, at, .. } => {
                let name = key.as_str();
                // RM-12: the node selects what the update may reach — the instance, and
                // under the layout one of its streams; the key decides what changes.
                let reachable = match addressed {
                    Addressed::Instance => true,
                    Addressed::Stream { .. } => true,
                    Addressed::Other => false,
                };
                let updatable = match (class, ezsdr_radio::channels::update_class(&key)) {
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
                    streams.book(&self.core, stream, Item { e: now, seq, ready: now, release: NO_RELEASE, delivered: None, refused: false, kind: Kind::Stop });
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
                        streams.book(&self.core, stream, Item { e: now, seq, ready: now, release: NO_RELEASE, delivered: None, refused: false, kind: Kind::Stop });
                    }
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
                let requested = match self.ceil_root(at) {
                    Ok(requested) => requested,
                    Err(error) => return self.core.command_rejected(Some(id), "command", &format!("UR-26: start_rx's instant cannot be converted: {error}")),
                };
                // One critical section takes the instant and the arrival order and books the
                // start, so that nothing pruned meanwhile takes effect after them (#63).
                let mut streams = lock(&self.core.streams);
                let stream = stream_key(Dir::Rx, k);
                let (seq, now) = (streams.next_seq(), self.core.now());
                let (e, _) = timeline::command_instant(streams.line(stream), requested, now, seq, false);
                let config = Arc::new(ColdConfig::with_clock(self.core.device.host(), self.config_at(e), self.held_updates(Dir::Rx)));
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
    /// streams, or something else. The judgement is the shared `ezsdr_radio::addressed`.
    fn addressed(&self, node: Option<&ResourceId>) -> Addressed {
        self.core.description.addressed(&self.core, node)
    }

    /// UC-2, UR-17: the timed commands held for `dir` as a segment configuration's own
    /// entries, each on the channel its key names, with the released ones at the instant each
    /// takes effect. Data, not a call on the configuration, so that a configuration built
    /// under the instance's line lock takes no second lock: `ColdConfig::book`'s order (the
    /// configuration's `updates`, then `streams`) has no counterpart for a configuration no
    /// other thread has seen yet (UR-25).
    fn held_updates(&self, dir: Dir) -> BTreeMap<(i64, u64), (usize, Settings)> {
        self.held
            .iter()
            .map(|(&order, timed)| (order, timed))
            .chain(self.released_at.iter().map(|(&order, given)| (order, &given.timed)))
            .filter(|(_, timed)| timed.dir == dir)
            .map(|(order, timed)| (order, (timed.chan, timed.settings())))
            .collect()
    }

    /// The segment configurations of `dir`, shared with its owners (every stream's).
    fn projections(&self, dir: Dir) -> Vec<Arc<ColdConfig>> {
        let streams = lock(&self.core.streams);
        (0..streams.count(dir)).flat_map(|k| streams.stream(stream_key(dir, k)).configs.values().cloned()).collect()
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
    /// released and the `cold` changes effective by `t` (UC-2; #61).
    pub(super) fn config_at(&self, t: i64) -> BTreeMap<Key, Value> {
        let mut config = self.config.clone();
        for (key, value) in self.colds.range(..=(t, u64::MAX)).map(|(_, change)| change) {
            config.insert(key.clone(), value.clone());
        }
        for given in self.released_at.range(..=(t, u64::MAX)).map(|(_, given)| &given.timed) {
            config.insert(given.key.clone(), Value::num(given.value).expect("UR-24: read from a Value"));
        }
        config
    }

    /// RM-27, UR-24: whether logical channel `chan` of `dir` is in a running stream at `t`
    /// — the stream that holds it is not halted and no refusal stops it. A timed update acts
    /// on the channel then; otherwise it is given to the later segments that run the channel,
    /// or held for one where none does (`targets`).
    fn channel_live(&self, dir: Dir, chan: usize, t: i64) -> bool {
        let stream = self.core.description.stream_of(&self.config_at(t), dir == Dir::Rx, chan as u32);
        if stream < 0 {
            return false;
        }
        let key = stream_key(dir, stream as usize);
        !lock(&self.core.streams).line(key).is_some_and(|line| line.halted(t))
    }

    /// UR-24: the channel a timed command is given on at `t` — in force there, or, with none
    /// in force, in a segment planned to begin at or after `t` that is not refused.
    fn targets(&self, dir: Dir, chan: usize, t: i64) -> (Vec<usize>, bool) {
        if self.channel_live(dir, chan, t) {
            return (vec![chan], true);
        }
        // `planned` takes the line's lock once after `channel_live` lets it go (UR-25's order).
        let planned: Vec<Planned> = {
            let streams = lock(&self.core.streams);
            (0..streams.count(dir)).flat_map(|k| streams.planned(stream_key(dir, k))).collect()
        };
        let reaches = planned.iter().any(|planned|
            !planned.refused && planned.segment.origin >= t && planned.segment.config.channels.contains(chan as u32));
        (reaches.then_some(chan).into_iter().collect(), false)
    }

    // ------------------------------------------------------------ UR-24

    fn book_timed(&mut self, id: ActionId, key: Key, value: Value, at: Option<AbsoluteDeadline>) {
        let Some(v) = value.as_scalar().and_then(scalar_number) else {
            return self.core.command_rejected(Some(id), "update_parameter", "UR-24: the value is not a number");
        };
        // RM-12, RM-27: the key decides the direction and, under the layout, the one
        // channel a frequency or gain update acts on.
        let (dir, chan) = match ezsdr_radio::concrete(&key) {
            Some(ezsdr_radio::Concrete::Channel { rx, number, .. }) => {
                let dir = if rx { Dir::Rx } else { Dir::Tx };
                if (number as usize) < self.core.description.channel_count(dir) {
                    (dir, number as usize)
                } else {
                    return self.core.command_rejected(Some(id), "update_parameter", &format!("UR-24: no such channel c{number}"));
                }
            }
            _ => return self.core.command_rejected(Some(id), "update_parameter", &format!("UR-24: {key} is not a channel's frequency or gain")),
        };
        let now = self.core.now();
        let requested = match self.ceil_root(at) {
            Ok(requested) => requested,
            Err(error) => return self.core.command_rejected(Some(id), "update_parameter", &format!("UR-24: explicit deadline cannot be converted: {error}")),
        };
        let lead = self.core.ticks(DEVICE_LEAD_NS);
        let window = self.core.ticks(self.core.description.timing.release_window_ns);
        // UR-24 (design 38 D2 (1)): the floor is the device lead or the instant of a command
        // booked earlier inside the release window, whichever is later — whether that command
        // was given, waits for a channel or the device refused it. The judgement is
        // `ezsdr_radio::timeline`'s one function, shared with MockRadio.
        let (e, late) = timeline::timed_instant(requested, now, lead, window, self.booked.iter().copied());
        if late {
            self.late_command(Some(key.clone()), requested.expect("a late command names its requested instant"), e);
        }
        let depth = self.core.description.timing.command_queue_depth as usize;
        // UR-24 (design 38 D2 (2), S4): one slot per command whose instant has not come,
        // whether held, given or refused.
        let pending = self.booked.iter().filter(|effective| **effective > now).count();
        if pending + 1 > depth {
            let payload = serde_json::to_value(CommandQueueFullPayload { key: key.clone(), depth: depth as i64 }).expect("a payload");
            self.core.emit(&self.core.id, kinds::COMMAND_QUEUE_FULL, Severity::Error, payload);
            self.core.reject_note(json!({ "kind": "update_parameter", "reason": "UR-24: the command queue is full", "key": key }));
            return;
        }
        self.booked.retain(|effective| *effective > now);
        self.booked.push(e);
        let seq = lock(&self.core.streams).next_seq();
        let timed = Timed { key, dir, value: v, chan };
        for config in self.projections(dir) {
            config.book((e, seq), timed.chan, timed.settings());
        }
        self.held.insert((e, seq), timed);
    }

    /// Releases held commands within the release window, in instant order, each on the
    /// channels UR-24 gives it at its instant: those in force then, or, with none in force,
    /// those of the later segments it reaches. One with neither stays held, and the release
    /// stops there, so that nothing behind it reaches the device before its instant; one whose
    /// instant passes with no channel is recorded `no channel` and is the configuration of
    /// record from then on (UR-24; #61, #64).
    pub(super) fn release(&mut self) {
        if self.core.is_lost() {
            return;
        }
        let now = self.core.now();
        let window = self.core.ticks(self.core.description.timing.release_window_ns);
        self.released.retain(|effective| *effective > now);
        self.booked.retain(|effective| *effective > now);
        while let Some(((e, seq), dir, chan)) = self.held.first_key_value().map(|(&order, timed)| (order, timed.dir, timed.chan)) {
            if e - now > window {
                break;
            }
            let (chans, live) = self.targets(dir, chan, e);
            if chans.is_empty() {
                if e > now {
                    // It waits for a channel a segment booked since may bring: it stays held,
                    // and the pass stops, so that no command behind it is given before its
                    // instant (UR-24).
                    break;
                }
                // Its instant passed with no channel to give it on: it is recorded there, and
                // becomes the configuration of record with the given ones, in instant order
                // (UR-24, UR-36).
                let timed = self.held.remove(&(e, seq)).expect("a held command");
                self.released_at.insert((e, seq), Given { timed, live: false, given: false, handed_at: None });
                continue;
            }
            // The instant is the booking's: the floor there keeps it behind every command
            // booked before it, so the device queue's order cannot move it.
            let timed = self.held.get(&(e, seq)).expect("a held command").clone();
            match self.give(e, timed.clone(), &chans) {
                // UR-24: it is the device's now; its row comes when its instant does.
                Handed::Given(handed_at) => {
                    self.held.remove(&(e, seq));
                    self.released_at.insert((e, seq), Given { timed, live, given: true, handed_at: Some(handed_at) });
                }
                // UR-24: a refusal other than by a lost device records no row and leaves the
                // command in every projected configuration; the booking's slot stays until
                // its instant (design 38 D2 (2)).
                Handed::Refused => {
                    self.held.remove(&(e, seq));
                    return;
                }
                // UR-29, design 38 D2 (4): not given, so it stays held; the end settles it.
                Handed::Lost => return,
            }
        }
        // UR-26, UR-29, design 38 D2 (4): a loss or a stop another thread booked ends the Run
        // first, so no row here may settle an instant at the end's own. Holding the streams'
        // lock across the check and the rows is what orders them against the end's publication:
        // `finish` settles every command still pending at `S`, `e <= S` and `e > S` alike, so a
        // stop published before or during this pass leaves its rows to it.
        let mut folded = Vec::new();
        {
            let streams = lock(&self.core.streams);
            if streams.ended_at().is_some() {
                return;
            }
            // A command whose instant has passed is recorded there and is the configuration of
            // record, which a segment beginning from now on begins with; one not yet in effect
            // stays an entry of every projection made meanwhile (`held_updates`). Held commands
            // are visited in instant order, so nothing enters `released_at` before an instant
            // already there: this fold is the instant order, and the rows are written in it.
            for order in self.released_at.range(..=(now, u64::MAX)).map(|(order, _)| *order).collect::<Vec<_>>() {
                let Given { timed, live, given, .. } = self.released_at.remove(&order).expect("a released command");
                self.record_given(order.0, &timed, live, given);
                folded.push(timed);
            }
        }
        for timed in folded {
            self.of_record(timed);
        }
    }

    /// UR-24: the `applied` row of a command whose instant `e` has come: `issued`, with
    /// `note: "no channel"` when it was given to a later segment because none was in force
    /// at `e`, or `note: "no channel"` alone when there was no channel to give it on.
    fn record_given(&self, e: i64, timed: &Timed, live: bool, given: bool) {
        let mut row = json!({ "key": timed.key, "claimed": timed.value, "read_back": null, "at": self.core.at(e) });
        if given {
            row["issued"] = json!(true);
            if !live {
                row["note"] = json!("no channel");
            }
        } else {
            row["note"] = json!("no channel");
        }
        lock(&self.core.rec).applied.push(row);
    }

    /// UR-24: `timed` becomes the configuration of record, which a segment beginning at or
    /// after its instant starts from — released, or with no channel its instant passed with.
    fn of_record(&mut self, timed: Timed) {
        self.config.insert(timed.key, Value::num(timed.value).expect("UR-24: read from a Value"));
    }

    /// UR-24: how a receive frequency due at `e` on `chan` reaches the device. The caller holds
    /// every projection's `updates`, so a configure is either over, with its mark set, or has
    /// not read the projection yet.
    fn reach(&self, chan: usize, e: i64) -> Reach {
        let planned: Vec<Planned> = {
            let streams = lock(&self.core.streams);
            (0..streams.count(Dir::Rx)).flat_map(|k| streams.planned(stream_key(Dir::Rx, k))).collect()
        };
        let holds = |planned: &&Planned| !planned.refused && planned.segment.config.channels.contains(chan as u32);
        // A segment of the channel whose samples pass at `e` (RM-36).
        if timeline::covering(planned.iter().filter(|planned| !planned.refused).map(|planned| &planned.segment), chan as u32, e).is_some() {
            return Reach::Timed;
        }
        // The segment the value is for: the first to begin after `e` that runs at all. One with
        // no configuration is the stream's first, which `prepare` configured, or one its owner
        // has begun; the mark of no other projection counts.
        let next = planned.iter().filter(holds)
            .filter(|planned| planned.segment.origin > e && planned.segment.cut.is_none_or(|cut| planned.segment.instant(cut).is_ok_and(|end| end > planned.segment.origin)))
            .min_by_key(|planned| planned.segment.origin);
        match next {
            Some(planned) if planned.settings.as_ref().is_none_or(|settings| settings.is_configured()) => Reach::Untimed,
            _ => Reach::Deferred,
        }
    }

    /// UR-24: gives the released command to the device at its instant, on `chans`; its row is
    /// written when the instant comes. A refusal other than by a lost device is reported, and
    /// a lost device found before the call is reported already (UR-29).
    fn give(&mut self, e: i64, timed: Timed, chans: &[usize]) -> Handed {
        let settings = timed.settings();
        let projections = self.projections(timed.dir);
        // Order a timed issuance against configure.
        assert_line_free("a timed command's release");
        let updates: Vec<_> = projections.iter().map(|cold| cold.updates.lock()).collect();
        let _tx_dsp = if timed.dir == Dir::Tx { self.core.lock_tx_dsp(chans) } else { Vec::new() };
        let mut handed_at = None;
        for chan in chans {
            self.core.probe("before_timed_apply", None);
            // UR-29: another thread may publish loss after target selection and before this
            // channel's apply; the lost device is never called, and the command is not given.
            if self.core.is_lost() {
                return Handed::Lost;
            }
            // RM-16, UR-26: the device's time just before the call is the hand-over's; a call
            // that straddles `S` counts at its start, since the device returns no acceptance
            // instant (UR-24, `device.rs`'s `Applied`).
            handed_at = Some(self.core.now());
            // UR-24: a receive frequency reaches the device as a timed command only while samples
            // pass the channel's DDC at `e`; a segment's configure, or this release untimed
            // where it has been configured already, gives it otherwise.
            let reach = if timed.dir == Dir::Rx && settings.freq.is_some() { self.reach(*chan, e) } else { Reach::Timed };
            let applied = match reach {
                Reach::Timed => Some(self.core.device.apply(timed.dir, *chan, &settings, Some(e))),
                Reach::Untimed => Some(self.core.device.apply(timed.dir, *chan, &settings, None)),
                Reach::Deferred => None,
            };
            if let Some(Err(error)) = applied {
                // UR-24: a refusal, other than by a lost device, is reported and records no row;
                // the entry stays in every configuration it was projected into, and the channels
                // the device took keep the value it took.
                drop(updates);
                self.core.device_failed(None, "update_parameter", &error);
                // UR-29, design 38 D2 (4): a lost device is not a refusal — the command was
                // never handed over, so it stays pending for the end's settlement.
                return if error.lost { Handed::Lost } else { Handed::Refused };
            }
            self.released.push(e);
        }
        drop(updates);
        Handed::Given(handed_at.expect("UR-24: a command is given on a channel"))
    }

    /// UR-26, UR-30, design 38 D2 (4): the end at its own instant `S` settles every command
    /// the device was not given by `S`, and the rows of the given ones whose instant came.
    /// One due by `S` (before `S` for a loss) takes the result it would have had: `no channel`
    /// for a channel-less one, cancelled for a live one the release did not hand over in time
    /// — RM-16's Ceiling, which the timeline's comparison keeps out — and issued for one
    /// handed over by `S`. One due after `S` is cancelled like the held ones, whether held or
    /// given; a loss at `S` counts before a stop at `S`, and a stop settles `e == S`.
    fn settle(&mut self, at: i64, why: &str, lost: bool) {
        let mut orders: Vec<(i64, u64)> = self.held.keys().chain(self.released_at.keys()).copied().collect();
        orders.sort_unstable();
        for order in orders {
            if let Some(timed) = self.held.remove(&order) {
                let projections = self.projections(timed.dir);
                assert_line_free("a cancelled command's updates");
                let mut updates: Vec<_> = projections.iter().map(|cold| cold.updates.lock()).collect();
                for updates in updates.iter_mut() {
                    updates.remove(&order);
                }
                let due = if lost { order.0 < at } else { order.0 <= at };
                let (chans, _) = if due { self.targets(timed.dir, timed.chan, order.0) } else { (Vec::new(), false) };
                // Keep the projections locked until the row is recorded, so an owner can never
                // apply this held value after it.
                let row = if due && chans.is_empty() {
                    json!({ "key": timed.key, "claimed": timed.value, "read_back": null, "at": self.core.at(order.0), "note": "no channel" })
                } else {
                    json!({ "key": timed.key, "claimed": timed.value, "at": self.core.at(order.0), "cancelled": why, "cancelled_at": self.core.at(at) })
                };
                lock(&self.core.rec).applied.push(row);
            } else if let Some(Given { timed, live, given, handed_at }) = self.released_at.remove(&order) {
                let due = if lost { order.0 < at } else { order.0 <= at };
                // RM-16, UR-26: a command the device had by `S` keeps the row it would have
                // had; one whose call began after `S` is the Ceiling — at `S` it had not been
                // handed over — and is cancelled at `S`. A channel-less one was never given.
                let handed = handed_at.is_none_or(|h| if lost { h < at } else { h <= at });
                if due && handed {
                    self.record_given(order.0, &timed, live, given);
                } else {
                    lock(&self.core.rec).applied.push(json!({
                        "key": timed.key, "claimed": timed.value, "at": self.core.at(order.0), "cancelled": why, "cancelled_at": self.core.at(at),
                    }));
                }
            }
        }
    }

    /// UR-26, UR-30: cancels the held commands at `at`, recording their rows.
    #[cfg(test)]
    fn cancel_held(&mut self, why: &str, at: i64) {
        self.settle(at, why, false);
    }

    // ------------------------------------------------------------ UR-25

    /// UR-25: an enable from no stream whose owner is reading reports waits for the read to
    /// end before its first device call. The wait is the host clock's (UR-33), so uhd-control
    /// is idle and the device's time runs; `Provider::stop`'s mark and a loss end it too, and
    /// then no device call is made.
    fn await_reading(&self, stream: StreamKey) -> bool {
        self.core.probe("wait_reading", None);
        loop {
            let (may_call, stop) = {
                let streams = lock(&self.core.streams);
                (streams.may_call_from_none(stream), streams.stop.is_some())
            };
            if stop || self.core.is_lost() {
                return false;
            }
            if may_call {
                self.core.probe("reading_done", None);
                return true;
            }
            // UR-25: the judgement that the read is still in flight has just been made; the
            // probe lets a test see the wait from here, with no device call made yet.
            self.core.probe("reading_wait", None);
            self.host.sleep(POLL);
        }
    }

    /// UR-25: a `cold` update is booked when uhd-control takes it: its effective instant,
    /// `coerce` over the configuration projected there, and the stream's plan from the
    /// timeline, which its owner reads. The key names its channel or its stream (RM-27),
    /// and the floor is the instance's (RM-25).
    fn book_cold(&mut self, id: ActionId, key: Key, value: Value, at: Option<AbsoluteDeadline>) {
        let dir = match ezsdr_radio::concrete(&key) {
            Some(ezsdr_radio::Concrete::Channel { rx, .. }) | Some(ezsdr_radio::Concrete::Stream { rx, .. }) => {
                if rx { Dir::Rx } else { Dir::Tx }
            }
            None => return self.core.command_rejected(Some(id), "update_parameter", &format!("UR-26: {key} is not updated at runtime by this Provider")),
        };
        let requested = match self.ceil_root(at) {
            Ok(requested) => requested,
            Err(error) => return self.core.command_rejected(Some(id), "update_parameter", &format!("UR-25: explicit deadline cannot be converted: {error}")),
        };
        // One critical section takes the instant and the arrival order and books the change,
        // so that nothing pruned meanwhile takes effect after it (#63).
        let mut streams = lock(&self.core.streams);
        let (seq, now) = (streams.next_seq(), self.core.now());
        // RM-25: never before a still-pending `cold` change of the whole instance, nor before
        // the receive start, T0, which floors it whether or not that side has a link yet.
        let start = (dir == Dir::Rx).then_some(self.t0);
        let earlier = self.colds.keys().map(|(e, _)| *e);
        let (e, late) = timeline::layout_instant(requested, now, start, earlier);
        if late {
            self.late_command(Some(key.clone()), requested.unwrap_or(now), e);
        }
        let before = self.config_at(e);
        let mut candidate = before.clone();
        candidate.insert(key.clone(), value.clone());
        let request = Requested {
            resource: self.core.id.clone(),
            constraints: candidate.iter().filter_map(|(k, v)| Some((k.clone(), Constraint::Eq { value: v.as_scalar()?.clone() }))).collect(),
        };
        let report = match self.core.description.coerce(&self.core.id, &request) {
            Ok(report) => report,
            Err(error) => return self.core.command_rejected(Some(id), "update_parameter", &error.message),
        };
        if let Some(rejected) = report.rejected.first() {
            return self.core.command_rejected(Some(id), "update_parameter", &rejected.reason);
        }
        let value = report.applied.get(&key).cloned().unwrap_or(value);
        candidate.insert(key.clone(), value.clone());
        if let Some(ezsdr_radio::Concrete::Channel { rx, number, leaf: "stream" }) = ezsdr_radio::concrete(&key) {
            let dir = if rx { Dir::Rx } else { Dir::Tx };
            let was = self.core.description.stream_of(&before, rx, number);
            let is = self.core.description.stream_of(&candidate, rx, number);
            if was >= 0 && is >= 0 && was != is {
                let from = stream_key(dir, was as usize);
                let to = stream_key(dir, is as usize);
                if streams.line(from).is_none() || streams.line(to).is_none() {
                    return self.core.command_rejected(
                        Some(id), "update_parameter",
                        &format!("RM-34: no line for move of channel c{number} from stream s{was} to s{is}"),
                    );
                }
                let mut lines = streams.lines.clone();
                match lines.book_move(from, to, number, Some(e), now, seq, now) {
                    Ok((effective, _)) if effective == e => streams.lines = lines,
                    Ok((effective, _)) => return self.core.command_rejected(
                        Some(id), "update_parameter",
                        &format!("RM-34: move's line instant {effective} differs from the Provider instant {e}"),
                    ),
                    Err(error) => return self.core.command_rejected(
                        Some(id), "update_parameter",
                        &format!("RM-34: move of channel c{number} from stream s{was} to s{is} was refused ({error:?})"),
                    ),
                }
                streams.changed(&self.core, from);
                streams.changed(&self.core, to);
                let host = self.core.device.host();
                let updates = self.held_updates(dir);
                streams.stream_mut(from).configs.insert(seq, Arc::new(ColdConfig::with_clock(host.clone(), candidate.clone(), updates.clone())));
                streams.stream_mut(to).configs.insert(seq, Arc::new(ColdConfig::with_clock(host, candidate, updates)));
                self.colds.insert((e, seq), (key.clone(), value.clone()));
                lock(&self.core.rec).applied.push(json!({ "key": key, "claimed": value, "read_back": null, "at": self.core.at(e) }));
                self.core.timing(json!({ "what": "move", "sequence": seq, "channel": number, "from": was, "to": is, "e": e, "booked_at": now, "ready": now }));
                return;
            }
        }
        // RM-12, RM-27, RM-29: the key names the stream the change belongs to — a channel's
        // own stream at its effective instant — and that stream's channel set and rate.
        let target: Option<(Dir, usize)> = match ezsdr_radio::concrete(&key) {
            Some(ezsdr_radio::Concrete::Stream { rx, number }) => Some((if rx { Dir::Rx } else { Dir::Tx }, number as usize)),
            Some(ezsdr_radio::Concrete::Channel { rx, number, leaf: "antenna" }) => {
                let dir = if rx { Dir::Rx } else { Dir::Tx };
                let stream = self.core.description.stream_of(&candidate, rx, number);
                (stream >= 0).then_some((dir, stream as usize))
            }
            Some(ezsdr_radio::Concrete::Channel { rx, number, leaf: "stream" }) => {
                let dir = if rx { Dir::Rx } else { Dir::Tx };
                let was = self.core.description.stream_of(&before, rx, number);
                let is = self.core.description.stream_of(&candidate, rx, number);
                if was == is {
                    None
                } else if was < 0 {
                    Some((dir, is.max(0) as usize))
                } else {
                    Some((dir, was as usize))
                }
            }
            _ => None,
        };
        let (dir, k) = target.unwrap_or((dir, 0));
        let channels = target.map_or_else(|| Channels::first(0), |(dir, k)| self.core.description.stream_channels(&candidate, dir == Dir::Rx, k as u32));
        let rate = self.core.description.stream_rate(&candidate, dir, k);
        let n = match decimation(self.core.mcr, rate) {
            Some(n) => n as i64,
            None if channels.is_empty() => 0,
            None => return self.core.command_rejected(Some(id), "update_parameter", &format!("UR-12: {rate} S/s is no decimation of {} Hz", self.core.mcr)),
        };
        let stream = stream_key(dir, k);
        // A membership change out of a stream is booked even when it empties the set — the
        // cut is what ends the running segment (RM-25) —; only a change that names no
        // stream at all (or whose stream is where the channel already was, or holds no
        // channel) takes its value alone.
        let leaves = target.is_some() && matches!(ezsdr_radio::concrete(&key), Some(ezsdr_radio::Concrete::Channel { leaf: "stream", .. }));
        let empty = channels.is_empty() && !leaves;
        let (old, idle, planned) = {
            let line = streams.line(stream);
            // The stream's own state is read only for a stream a channel enters (RM-29):
            // one with no channel and no line has none.
            if !channels.is_empty() && line.is_some() {
                (line.map(|line| line.stream.config), streams.is_idle(stream), line.is_some_and(|line| line.plan.iter().all(|segment| segment.cut.is_some())))
            } else {
                (line.map(|line| line.stream.config), false, false)
            }
        };
        let Some(old) = old else {
            // A receive side with no link has no stream: the change only takes its value.
            self.colds.insert((e, seq), (key, value));
            return;
        };
        if empty {
            // A stream no channel is in takes its value and nothing else (RM-21).
            lock(&self.core.rec).applied.push(json!({ "key": key, "claimed": value, "read_back": null, "at": self.core.at(e) }));
            self.colds.insert((e, seq), (key, value));
            return;
        }
        let ratio = if n > 0 { Rational::new(n as u64, 1).expect("a decimation") } else { old.ratio };
        let config = Config { channels, ratio };
        let segment_config = Arc::new(ColdConfig::with_clock(self.core.device.host(), candidate.clone(), self.held_updates(dir)));
        // RM-25, VH-4: an enable from no stream — its owner idle, nothing planned to run —
        // is configured by uhd-control itself, which opens the streamer, and is ready when
        // that configuration ends; any other change by the owner at the cut (UR-17, UR-23).
        let wants_from_none = !channels.is_empty() && idle && planned && streams.holder(stream) == Holder::Free
            && streams.held_elsewhere(stream, channels).is_none()
            && !streams.needed_before(stream, channels, i64::MAX);
        let mut from_none_claim = if wants_from_none { streams.claim_from_none(stream) } else { None };
        let from_none = from_none_claim.is_some();
        let mut ready = now;
        let mut failure: Option<DeviceError> = None;
        let mut reading = false;
        let mut stopped = false;
        if from_none {
            // UR-25: the streamer is uhd-control's from this section — the one that read the
            // owner's idle mark and the plan — until the section that books the change, on
            // every path. An owner that finds the claim makes no device call.
            reading = streams.reading(stream);
            stopped = streams.stop.is_some();
        }
        if from_none {
            // Its owner is idle: nothing it could prune comes meanwhile, so the lock is
            // released for the device calls alone and taken again for the book. The stream
            // is the one the change names (`k`).
            drop(streams);
            // UR-29, UR-25: a loss, or `Provider::stop` read from its mark before any call,
            // ends the enable with no device call; the stop's own path books the end.
            let may_call = if reading { self.await_reading(stream) } else { !self.core.is_lost() && !stopped };
            let mut opened: Option<Result<(), DeviceError>> = None;
            let mut checked: Option<Result<(), DeviceError>> = None;
            let mut configured: Option<Result<i64, DeviceError>> = None;
            if may_call {
                let open = if dir == Dir::Tx {
                    self.core.open_tx_device(k, channels).map(|_| ())
                } else {
                    self.core.open_device(dir, k, channels)
                };
                let check = if open.is_ok() && !self.core.is_lost() { self.core.check_opened(dir, channels) } else { Ok(()) };
                if open.is_ok() && check.is_err() && !self.core.is_lost() {
                    self.core.close_device(dir, k);
                }
                // The configuration in effect at `e`, the timed updates held for it included (UC-2).
                let entries: Vec<(usize, Settings)> = channels
                    .iter()
                    .map(|chan| {
                        let mut settings = self.core.description.settings(&candidate, dir, k, chan as usize);
                        settings.rate = Some(rate);
                        // Apply held updates due by this segment's origin to their channel.
                        for timed in self.held.range(..=(e, u64::MAX)).map(|(_, timed)| timed).filter(|timed| timed.dir == dir && timed.chan == chan as usize) {
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
                if let Some(claim) = from_none_claim.take() { streams.give_back(claim); }
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
            streams.record_configured(stream, failure.is_none().then_some(seq));
            // UR-25: the enable's own configuration carried the projection's updates.
            if failure.is_none() {
                segment_config.configured.store(true, Ordering::SeqCst);
            }
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
        if let Some(claim) = from_none_claim.take() { streams.give_back(claim); }
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
            streams.stream(stream).clocks.last().map(|(clock, _)| *clock).zip(last.map(|segment| segment.config.channels.count() as usize))
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
        let open = BurstOpen {
            waveform_len: u32::try_from(len).ok(),
            late: (!matches!(outcome, LateOutcome::OnTime {})).then_some(outcome),
            requested_target: requested,
        };
        let _ = self.to_tx.get(k).map(|to_tx| to_tx.send(TxCmd::Burst(Held {
            k: start,
            domain: clock.domain,
            samples: Arc::new(samples),
            waveform_len: len,
            repeat,
            open,
            policy: late_policy,
            target: at.time_point,
        })));
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
    use crate::provider::test_support::device_at;
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

    fn pause_core_at(core: &Core, name: &'static str) -> (Receiver<()>, std::sync::mpsc::Sender<()>) {
        let (entered, reached) = channel();
        let (resume, waiting) = channel();
        let waiting = std::sync::Mutex::new(waiting);
        crate::provider::test_support::probe(core, name, move |_| {
            let _ = entered.send(());
            let _ = waiting.lock().unwrap().recv();
        });
        (reached, resume)
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
        lock(&core.streams).search(booked_stream(Dir::Rx), false);
        control.book_cold(ActionId(1), key("radio.rx.s0.sample_rate_hz"), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(100)))));
        assert!(matches!(lock(&core.streams).stream(booked_stream(Dir::Rx)).set, super::super::core::DeviceSet::Unknown));
        lock(&core.streams).search(booked_stream(Dir::Rx), false);
        control.book_cold(ActionId(2), key("radio.rx.s0.sample_rate_hz"), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(200)))));
        assert_eq!(device.calls().iter().filter(|call| call.starts_with("rx_open")).count(), 2, "{:?}", device.calls());
    }

    /// Books an enable of the receive stream at `ms` from no stream, the owner idle (UR-13).
    fn enable_from_none(control: &mut Control, core: &Core, id: u64, ms: i64, rate: f64) {
        lock(&core.streams).search(booked_stream(Dir::Rx), false);
        control.book_cold(ActionId(id), key("radio.rx.s0.sample_rate_hz"), Value::num(rate).unwrap(), Some(AbsoluteDeadline::new(core.at(core.ticks(ms * 1_000_000)))));
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
    fn ur_25_a_transmit_refusal_from_no_stream_starts_no_clock() {
        // UR-25 (design note 37, the 74th row): a transmit enable from no stream whose rate
        // comparison refuses starts no clock, and its owner moves on at once — the refused
        // segment's clock ended at its origin —, so an enable booked while that origin is
        // still ahead is taken from no stream and refused under its own Action, not held
        // behind it. The refusal is the stored transmit comparison `check_rates` keeps for
        // every open of the direction; the controlled clock fixes both bookings.
        let (core, device, time, events) = manual_rig_events();
        let frozen = hold(&core, &time, &device);
        assert!(core.check_rates(Dir::Tx, 99).is_err(), "the transmit comparison refuses");
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.tx.c0.stream"), Value::from(-1));
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        // The owner, idle and with no clock, reads an empty plan.
        let (_to_tx, cmds) = channel();
        let mut tx = super::super::tx::Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        assert!(tx.read(None));
        drop(frozen);
        control.book_cold(ActionId(1), key("radio.tx.c0.stream"), Value::from(0), at(200));
        assert!(device.calls().iter().any(|call| call == "tx_open 0 [0]"), "the enable's open: {:?}", device.calls());
        assert!(!device.calls().iter().any(|call| call.starts_with("apply")), "a refused enable applies nothing: {:?}", device.calls());
        // The owner reads the refused segment and skips it at once, before its origin.
        assert!(tx.read(None));
        assert!(tx.step(), "the owner's turn");
        assert!(core.now() < ms(200), "the refused segment's origin is still ahead: {}", core.now());
        assert!(lock(&core.streams).is_idle(stream_key(Dir::Tx, 0)), "the owner did not move on, or adopted a clock");
        assert!(timing_of(&core, "tx_switch").is_empty(), "a refused segment was adopted: {:?}", timing_of(&core, "tx_switch"));
        let before = device.calls().len();
        // The next enable, its instant floored at the first change's, is taken from no stream
        // and refused under its own Action before that origin.
        control.book_cold(ActionId(2), key("radio.tx.c1.stream"), Value::from(0), at(60));
        assert_eq!(device.calls()[before..].iter().filter(|call| call.starts_with("tx_open")).count(), 1, "the second enable's own open: {:?}", device.calls());
        let rejected = events_of(&events, kinds::COMMAND_REJECTED);
        assert_eq!(rejected.len(), 2, "{rejected:?} {:?}", device.calls());
        assert_eq!(rejected.iter().map(|event| event.payload["action"].as_u64()).collect::<Vec<_>>(), [Some(1), Some(2)], "an enable was refused by an owner, not under its Action: {rejected:?}");
        assert!(rejected.iter().all(|event| event.payload["reason"].as_str().is_some_and(|reason| reason.starts_with("UR-13:"))), "{rejected:?}");
        assert!(rejected.iter().all(|event| event.time.ticks_in(core.root).unwrap() < ms(200)), "an enable was processed at or after the refused origin: {rejected:?}");
        assert!(lock(&core.streams).is_idle(stream_key(Dir::Tx, 0)), "a refused enable adopted a clock");
        let sends: Vec<_> = device.calls().into_iter().filter(|call| call.starts_with("tx_send")).collect();
        assert_eq!(sends, ["tx_send 0 n=2 at=- sob=true eob=true", "tx_send 0 n=2 at=- sob=true eob=true"],
            "each refused streamer generation gets its own T3 flush: {:?}", sends);
    }

    #[test]
    fn ur_22_from_none_open_waits_for_t3_flush_ack_before_timed_tx_dsp() {
        let spy = Arc::new(crate::provider::test_support::LockSpy::new(crate::device::FakeConfig {
            clock: crate::ClockMode::Manual, ..crate::device::FakeConfig::default()
        }));
        let (core, _, _) = crate::provider::test_support::rig_on(spy.clone(), vec![Arc::new(NoLink)], 2_000);
        spy.watch(&core);
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.tx.c0.stream"), Value::from(-1));
        control.book(Dispatched::new(ActionId(1), Action::Stop {
            target: Some(super::super::test_support::target_of(&core.tx_id)),
        }, Some(core.tx_id.clone())));
        lock(&core.streams).search(booked_stream(Dir::Tx), false);
        let at = AbsoluteDeadline::new(core.at(core.ticks(100_000_000)));
        super::super::core::assert_tx_open_flush_barrier(core, spy, move || {
            control.book_cold(ActionId(2), key("radio.tx.c0.stream"), Value::from(0), Some(at));
        });
    }

    #[test]
    fn ur_22_timed_control_give_waits_for_t3_flush_ack() {
        let spy = Arc::new(crate::provider::test_support::LockSpy::new(crate::device::FakeConfig {
            clock: crate::ClockMode::Manual, ..crate::device::FakeConfig::default()
        }));
        let (core, _, _) = crate::provider::test_support::rig_on(spy.clone(), vec![Arc::new(NoLink)], 2_000);
        spy.watch(&core);
        let gate = spy.stop_before("tx_async");
        let (open_done, opened) = std::sync::mpsc::channel();
        let opener = core.device.host().spawn("control-tx-flush", {
            let core = core.clone();
            move || { let _ = open_done.send(core.open_tx_device(0, Channels::first(1))); }
        }).unwrap();
        gate.wait();

        let (mut control, _tx) = control(&core, &[]);
        let timed = Timed { key: key("radio.tx.c0.gain_db"), dir: Dir::Tx, value: 1.0, chan: 0 };
        let (release_done, released) = std::sync::mpsc::channel();
        let release = core.device.host().spawn("control-timed-give", move || {
            let _ = release_done.send(control.give(10_000, timed, &[0]));
        }).unwrap();
        let deadline = std::time::Instant::now() + Wall::from_secs(10);
        while core.tx_dsp_waiters(0) == 0 && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(core.tx_dsp_waiters(0) > 0, "Control::give did not wait on the flush's DSP gate");
        assert_eq!(spy.seen("apply"), 0, "the real timed release crossed the flush ack barrier");

        gate.release();
        opener.join().unwrap();
        release.join().unwrap();
        assert!(opened.recv().unwrap().unwrap().acknowledged, "the open flush ack was not consumed");
        assert!(matches!(released.recv().unwrap(), Handed::Given(_)));
        let order = spy.order();
        let flush = order.iter().position(|call| *call == "tx_send").unwrap();
        let ack = order.iter().position(|call| *call == "tx_async").unwrap();
        let apply = order.iter().position(|call| *call == "apply").unwrap();
        assert!(flush < ack && ack < apply, "flush -> ack read -> Control::give: {order:?}");
    }

    #[test]
    fn ur_24_a_timed_update_after_a_from_none_configure_is_sent_without_configuring_again() {
        // UR-24 (S11a): an update the enable's segment holds at its origin and the device
        // receives as its own timed command leaves the record holding the segment's command, so
        // the owner starts it without configuring the direction again.
        let (core, device, time) = rig();
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        let ms = |n: i64| core.ticks(n * 1_000_000);
        channels_from_none(&mut control, &core, 2, 100, 1);
        let planned = plan_of(&core, Dir::Rx);
        let by = planned[0].segment.by.expect("the enable");
        // Due at the segment's own origin, where the channel is in force: a timed command.
        control.book_timed(ActionId(3), key("radio.rx.c0.gain_db"), Value::num(3.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(100)))));
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
        assert_eq!(lock(&core.streams).stream(booked_stream(Dir::Rx)).configured, Some(by));
    }

    /// The manual-clock rig a series that reads the device's own settings runs on: the fake's
    /// queue is wide enough that a wrong release never blocks it, and its clock is the one a
    /// test sets (`FakeConfig::command_queue`).
    fn manual_rig() -> (Arc<Core>, Arc<crate::device::FakeDevice>, Arc<ezsdr_kernel::time::ManualTimeAuthority>) {
        let config = crate::device::FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..crate::device::FakeConfig::default() };
        let (core, device, time, _) = crate::provider::test_support::rig_with(config, vec![Arc::new(NoLink)]);
        (core, device, time)
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

    /// The manual-clock rig on that layout, with the receive side's stream.
    fn layout_rig() -> (Arc<Core>, Arc<crate::device::FakeDevice>, Arc<ezsdr_kernel::time::ManualTimeAuthority>) {
        let config = crate::device::FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..crate::device::FakeConfig::default() };
        let device = Arc::new(crate::device::FakeDevice::new(config));
        let (core, time, _) = crate::provider::test_support::rig_on_description(device.clone(), vec![Arc::new(NoLink)], BTreeMap::new(), layout_description());
        (core, device, time)
    }

    /// The layout's rig with two receive streams' links, so that a join names stream 1.
    fn two_stream_rig() -> (Arc<Core>, Arc<crate::device::FakeDevice>, Arc<ezsdr_kernel::time::ManualTimeAuthority>, Arc<ezsdr_kernel::event::EventCollector>) {
        let config = crate::device::FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..crate::device::FakeConfig::default() };
        let device = Arc::new(crate::device::FakeDevice::new(config));
        let (core, time, events) = crate::provider::test_support::rig_on_description_links(device.clone(), vec![vec![Arc::new(NoLink)], vec![Arc::new(NoLink)]], BTreeMap::new(), layout_description());
        (core, device, time, events)
    }

    #[test]
    fn ur_34_a_from_none_join_yields_to_an_earlier_unprocessed_segment() {
        let selector = BTreeMap::from([
            (ezsdr_kernel::spec::Ident::parse("rx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:B:basic_rx:0"), Scalar::from("0:B:basic_rx:1")])),
            (ezsdr_kernel::spec::Ident::parse("tx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0")])),
        ]);
        let layout = ezsdr_radio::channels::from_selector(&selector).unwrap();
        let description = ezsdr_radio::channels::Description::new(&layout, 2_000, &crate::profile::terms());
        let device = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig { front_ends: vec!["UBX", "BasicRX"], ..crate::device::FakeConfig::default() }));
        let links = (0..3).map(|_| vec![Arc::new(NoLink) as Arc<dyn DataLink>]).collect();
        let (core, _time, _) = crate::provider::test_support::rig_on_description_links(device.clone(), links, BTreeMap::new(), description);
        let mut values = core.description.defaults();
        for (channel, stream) in [(0, 0), (1, -1), (2, -1)] {
            values.insert(key(&format!("radio.rx.c{channel}.stream")), Value::from(stream));
        }
        let start = Start { t0: 0, rx: vec![Some((Channels::first(1), 200)), None, None], tx: vec![None] };
        let (to_tx, _tx) = channel();
        let mut control = Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], values, false, start);
        device.rx_open(0, &[0]).unwrap();
        let (a, b, c) = (stream_key(Dir::Rx, 0), stream_key(Dir::Rx, 1), stream_key(Dir::Rx, 2));
        let mut streams = lock(&core.streams);
        streams.record_open(a, Some(Channels::first(1)));
        streams.search(a, false);
        let a_segment = streams.planned(a).into_iter().find(|planned| planned.segment.by.is_none()).expect("the initial source segment");
        drop(streams);

        let move_e = core.now() + core.ticks(100_000_000);
        let move_seq = lock(&core.streams).seq + 1;
        control.book_cold(ActionId(1), key("radio.rx.c0.stream"), Value::from(1), Some(AbsoluteDeadline::new(core.at(move_e))));
        let leave_e = move_e + core.ticks(200_000_000);
        control.book_cold(ActionId(2), key("radio.rx.c0.stream"), Value::from(-1), Some(AbsoluteDeadline::new(core.at(leave_e))));
        let next_b = lock(&core.streams).planned(b).into_iter().find(|planned| planned.segment.by == Some(move_seq)).expect("B's finite segment");
        assert!(next_b.segment.cut.is_some_and(|cut| next_b.segment.instant(cut).unwrap() > next_b.segment.origin));

        let (b_waiting, b_waited) = channel();
        crate::provider::test_support::probe(&core, "wait_release", move |_| { let _ = b_waiting.send(()); });
        let (open_entered, open_reached) = channel();
        let (open_resume, open_wait) = channel();
        let open_wait = std::sync::Mutex::new(open_wait);
        let open_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        crate::provider::test_support::probe(&core, "open", {
            let open_count = open_count.clone();
            move |_| {
                if open_count.fetch_add(1, Ordering::SeqCst) == 0 {
                    let _ = open_entered.send(());
                    let _ = open_wait.lock().unwrap().recv();
                }
            }
        });
        let owner_core = core.clone();
        let (b_done, b_result_rx) = channel();
        let next_b_for_owner = next_b.clone();
        let b_owner = core.device.host().spawn("rx-chain-B", move || { let _ = b_done.send(owner_core.owner_configure(Dir::Rx, 1, &next_b_for_owner)); }).unwrap();
        assert!(b_waited.recv_timeout(Wall::from_secs(10)).is_ok(), "B reached its release wait");
        {
            let mut streams = lock(&core.streams);
            streams.complete(a, (a_segment.segment.origin, a_segment.segment.config, a_segment.segment.by));
        }
        assert!(core.release_unneeded(Dir::Rx, 0), "A releases its channel");
        assert!(open_reached.recv_timeout(Wall::from_secs(10)).is_ok(), "B is paused after A's release and before its open");

        let calls_before_join = device.calls();
        let join_e = leave_e + core.ticks(200_000_000);
        control.book_cold(ActionId(3), key("radio.rx.c0.stream"), Value::from(2), Some(AbsoluteDeadline::new(core.at(join_e))));
        let c_opened_before_b = device.calls().iter().any(|call| call == "rx_open 2 [0]");
        let no_control_device_call = device.calls() == calls_before_join;
        let join_seq = lock(&core.streams).seq;
        let next_c = lock(&core.streams).planned(c).into_iter().find(|planned| planned.segment.by == Some(join_seq)).expect("C's joined segment");
        let (c_waiting, c_waited) = channel();
        crate::provider::test_support::probe(&core, "wait_release", move |_| { let _ = c_waiting.send(()); });
        let owner_core = core.clone();
        let (c_done, c_result_rx) = channel();
        let c_owner = core.device.host().spawn("rx-chain-C", move || { let _ = c_done.send(owner_core.owner_configure(Dir::Rx, 2, &next_c)); }).unwrap();
        let c_reached_wait = c_waited.recv_timeout(Wall::from_secs(10)).is_ok();

        let _ = open_resume.send(());
        b_owner.join().unwrap();
        let b_result = b_result_rx.recv_timeout(Wall::from_secs(10)).unwrap();
        {
            let mut streams = lock(&core.streams);
            streams.complete(b, (next_b.segment.origin, next_b.segment.config, next_b.segment.by));
            streams.search(b, false);
        }
        let b_released = core.release_unneeded(Dir::Rx, 1);
        c_owner.join().unwrap();
        let c_result = c_result_rx.recv_timeout(Wall::from_secs(10)).unwrap();
        let calls = device.calls();
        let position = |call: &str| calls.iter().position(|entry| entry == call).unwrap_or_else(|| panic!("{call}: {calls:?}"));

        assert!(!c_opened_before_b, "uhd-control made no device call for C while B's segment was unprocessed: {calls:?}");
        assert!(no_control_device_call, "booking C's join did not call the device: {calls:?}");
        assert!(c_reached_wait, "C's owner waited for B's segment");
        assert!(b_result.unwrap(), "B opens successfully before C");
        assert!(b_released, "B releases c0 after its finite segment");
        assert!(c_result.unwrap(), "C opens after B releases c0");
        assert!(position("rx_close 0") < position("rx_open 1 [0]"), "A releases before B opens: {calls:?}");
        assert!(position("rx_open 1 [0]") < position("rx_close 1"), "B opens before it releases: {calls:?}");
        assert!(position("rx_close 1") < position("rx_open 2 [0]"), "C opens after B releases: {calls:?}");
    }

    #[test]
    fn ur_34_a_control_books_the_next_action_while_an_owner_close_is_gated() {
        // A6: the owner is blocked inside its release call; uhd-control must still book the
        // following Action rather than waiting for the device.
        let spy = Arc::new(crate::provider::test_support::LockSpy::new(crate::device::FakeConfig {
            clock: crate::ClockMode::Manual, ..crate::device::FakeConfig::default()
        }));
        let (core, _time, _events) = crate::provider::test_support::rig_on_description_links(
            spy.clone(), vec![vec![Arc::new(NoLink)], vec![Arc::new(NoLink)]], BTreeMap::new(), layout_description());
        spy.watch(&core);
        let mut values = core.description.defaults();
        values.insert(key("radio.rx.c1.stream"), Value::from(1));
        let start = Start {
            t0: 0,
            rx: vec![Some((Channels::first(1), 200)), Some((Channels::first(2).without(0), 200))],
            tx: vec![None],
        };
        let (to_tx, _tx) = channel();
        let mut control = Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], values, false, start);
        spy.rx_open(0, &[0]).unwrap();
        spy.rx_open(1, &[1]).unwrap();
        {
            let mut streams = lock(&core.streams);
            streams.stream_mut(stream_key(Dir::Rx, 0)).set = super::super::core::DeviceSet::Open(Channels::first(1));
            streams.stream_mut(stream_key(Dir::Rx, 1)).set = super::super::core::DeviceSet::Open(Channels::first(2).without(0));
            streams.search(stream_key(Dir::Rx, 1), false);
        }
        let move_e = core.ticks(100_000_000);
        control.book_cold(ActionId(1), key("radio.rx.c1.stream"), Value::from(0), Some(AbsoluteDeadline::new(core.at(move_e))));
        let gate = spy.stop_before("rx_close");
        let owner_core = core.clone();
        let owner = std::thread::spawn(move || owner_core.release_unneeded(Dir::Rx, 1));
        gate.wait();

        let rate_key = key("radio.rx.s0.sample_rate_hz");
        let next_e = move_e + core.ticks(20_000_000);
        let expected_seq = lock(&core.streams).seq + 1;
        let (booked, finished) = channel();
        let booking = std::thread::spawn(move || {
            control.book_cold(ActionId(2), rate_key.clone(), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(core.at(next_e))));
            let _ = booked.send(control.colds.contains_key(&(next_e, expected_seq)));
        });
        let did_book = finished.recv_timeout(Wall::from_secs(10)).unwrap_or(false);
        gate.release();
        booking.join().unwrap();
        assert!(owner.join().unwrap(), "the owner released the unneeded streamer");
        assert!(did_book, "the second Action was booked before the close gate was released");
    }

    #[test]
    fn ur_34_a_move_books_one_item_on_both_lines_and_both_configurations() {
        let (core, device, _time, _events) = two_stream_rig();
        let mut config = core.description.defaults();
        config.insert(key("radio.rx.c1.stream"), Value::from(0));
        let start = Start {
            t0: 0,
            rx: vec![Some((Channels::first(2), 100)), Some((Channels::first(0), 200))],
            tx: vec![None],
        };
        let (to_tx, _tx) = channel();
        let mut control = Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], config, false, start);
        let e = core.ticks(100_000_000);
        control.book_cold(ActionId(1), key("radio.rx.c1.stream"), Value::from(1), Some(AbsoluteDeadline::new(core.at(e))));
        let streams = lock(&core.streams);
        let from = streams.line(stream_key(Dir::Rx, 0)).unwrap();
        let to = streams.line(stream_key(Dir::Rx, 1)).unwrap();
        let from_item = from.items.iter().find(|item| item.seq == 1).expect("the source's move item");
        let to_item = to.items.iter().find(|item| item.seq == 1).expect("the destination's move item");
        assert_eq!((from_item.e, to_item.e, from_item.seq, to_item.seq), (e, e, 1, 1));
        assert!(matches!(from_item.kind, Kind::Cold(config) if !config.channels.contains(1)));
        assert!(matches!(to_item.kind, Kind::Cold(config) if config.channels.contains(1)));
        assert!(streams.stream(stream_key(Dir::Rx, 0)).configs.contains_key(&1));
        assert!(streams.stream(stream_key(Dir::Rx, 1)).configs.contains_key(&1));
        assert!(!Arc::ptr_eq(&streams.stream(stream_key(Dir::Rx, 0)).configs[&1], &streams.stream(stream_key(Dir::Rx, 1)).configs[&1]),
            "each line's projection has its own update lock");
        drop(streams);
        assert!(control.colds.contains_key(&(e, 1)), "the move is in the layout floor");
        let rec = lock(&core.rec);
        let row = rec.applied.iter().find(|row| row["key"] == "radio.rx.c1.stream").expect("the logical update row");
        assert_eq!(row["claimed"], 1);
        assert!(row["read_back"].is_null());
        assert_eq!(row["at"]["ticks"], e);
        let move_row = rec.timing.iter().find(|row| row["what"] == "move").expect("the move timing row");
        assert_eq!((move_row["sequence"].as_u64(), move_row["channel"].as_u64(), move_row["from"].as_i64(), move_row["to"].as_i64()), (Some(1), Some(1), Some(0), Some(1)));
        drop(rec);
        assert!(device.calls().is_empty(), "booking does not call the device: {:?}", device.calls());
    }

    #[test]
    fn ur_34_a_move_to_a_stream_without_a_line_uses_the_mock_reason() {
        let config = crate::device::FakeConfig { clock: crate::ClockMode::Manual, ..crate::device::FakeConfig::default() };
        let device = Arc::new(crate::device::FakeDevice::new(config));
        let (core, _time, events) = crate::provider::test_support::rig_on_description(device.clone(), vec![Arc::new(NoLink)], BTreeMap::new(), layout_description());
        let mut values = core.description.defaults();
        values.insert(key("radio.rx.c1.stream"), Value::from(0));
        let start = Start { t0: 0, rx: vec![Some((Channels::first(2), 100)), None], tx: vec![None] };
        let (to_tx, _tx) = channel();
        let mut control = Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], values, false, start);
        let e = core.ticks(100_000_000);
        control.book_cold(ActionId(1), key("radio.rx.c1.stream"), Value::from(1), Some(AbsoluteDeadline::new(core.at(e))));
        let rejected = events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        let reason = rejected.iter().find(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED)
            .unwrap().payload["reason"].as_str().unwrap();
        assert_eq!(reason, "RM-34: no line for move of channel c1 from stream s0 to s1");
        assert!(device.calls().is_empty(), "{:?}", device.calls());
    }

    #[test]
    fn ur_34_a_second_move_and_a_move_after_refusal_and_prune_keep_projection_aligned() {
        let (core, _device, _time, events) = two_stream_rig();
        let mut values = core.description.defaults();
        values.insert(key("radio.rx.c1.stream"), Value::from(0));
        let start = Start { t0: 0, rx: vec![Some((Channels::first(2), 100)), Some((Channels::first(0), 200))], tx: vec![None] };
        let (to_tx, _tx) = channel();
        let mut control = Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], values, false, start);
        let at = |ms: i64| Some(AbsoluteDeadline::new(core.at(core.ticks(ms * 1_000_000))));
        control.book_cold(ActionId(1), key("radio.rx.c1.stream"), Value::from(1), at(100));
        control.book_cold(ActionId(2), key("radio.rx.c1.stream"), Value::from(0), at(200));
        {
            let mut streams = lock(&core.streams);
            assert!(streams.refuse(&core, stream_key(Dir::Rx, 0), 1), "the first move is refused on both lines");
            let before = core.ticks(200_000_000) + 1;
            streams.lines.prune_stream(stream_key(Dir::Rx, 0), before);
            streams.lines.prune_stream(stream_key(Dir::Rx, 1), before);
        }
        control.book_cold(ActionId(3), key("radio.rx.c1.stream"), Value::from(1), at(400));
        let streams = lock(&core.streams);
        let from = streams.line(stream_key(Dir::Rx, 0)).unwrap();
        let to = streams.line(stream_key(Dir::Rx, 1)).unwrap();
        assert!(from.items.iter().any(|item| item.seq == 3), "the projected source still holds c1: {from:?}");
        assert!(to.items.iter().any(|item| item.seq == 3), "the third move is on the destination line: {to:?}");
        assert!(streams.stream(stream_key(Dir::Rx, 0)).configs.contains_key(&3));
        assert!(streams.stream(stream_key(Dir::Rx, 1)).configs.contains_key(&3));
        assert!(!streams.lines.config_at(stream_key(Dir::Rx, 0), core.ticks(400_000_000)).channels.contains(1));
        assert!(streams.lines.config_at(stream_key(Dir::Rx, 1), core.ticks(400_000_000)).channels.contains(1));
        drop(streams);
        assert!(events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter()
            .all(|event| event.kind.as_str() != ezsdr_radio::kinds::COMMAND_REJECTED));
    }

    #[test]
    fn ur_34_a_transmit_stop_books_the_wait_exit_and_queues_the_owner_stop() {
        let (core, _device, _time) = rig();
        let (mut control, owner) = control(&core, &[Dir::Tx]);
        control.book(Dispatched::new(ActionId(1), Action::Stop {
            target: Some(super::super::test_support::target_of(&core.tx_id)),
        }, Some(core.tx_id.clone())));
        let stream = booked_stream(Dir::Tx);
        let stop = lock(&core.streams).line(stream).unwrap().items.iter().find(|item| item.kind == Kind::Stop).copied().unwrap();
        assert!(matches!(owner.try_recv().unwrap(), TxCmd::Stop));
        assert_eq!(stop.e, core.now());
    }

    #[test]
    fn ur_24_the_command_queue_is_counted_per_instance_across_streams() {
        // UR-24 (S10): the candidate depth is shared across streams; future reservations fill
        // it, and a released command continues to take its slot.
        let config = crate::device::FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..crate::device::FakeConfig::default() };
        let device = Arc::new(crate::device::FakeDevice::new(config));
        let mut description = layout_description();
        description.timing.command_queue_depth = 3;
        let (core, time, events) = crate::provider::test_support::rig_on_description_links(
            device.clone(),
            vec![vec![Arc::new(NoLink)], vec![Arc::new(NoLink)]],
            BTreeMap::new(),
            description,
        );
        let _frozen = hold(&core, &time, &device);
        let (to_tx, _tx) = channel();
        let start = Start { t0: 0, rx: vec![Some((Channels::first(1), 200)), Some((Channels::first(1), 200))], tx: vec![None] };
        let mut control = Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], core.description.defaults(), false, start);
        control.config.insert(key("radio.rx.c0.stream"), Value::from(0));
        control.config.insert(key("radio.rx.c1.stream"), Value::from(1));
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        for i in 0..3 {
            let (key, channel) = if i % 2 == 0 { (key("radio.rx.c0.gain_db"), ActionId(i as u64 + 1)) } else { (key("radio.rx.c1.gain_db"), ActionId(i as u64 + 1)) };
            let value = Value::num(i as f64 + 1.0).unwrap();
            control.book_timed(channel, key, value, at(5_000 + i));
        }
        assert_eq!(control.held.len(), 3, "{:?}", control.held.keys().collect::<Vec<_>>());
        assert!(device.calls().iter().all(|call| !call.starts_with("apply")), "{:?}", device.calls());
        let _ = events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        control.book_timed(ActionId(20), key("radio.rx.c0.gain_db"), Value::num(9.0).unwrap(), at(5_500));
        let full = events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        assert_eq!(full.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_QUEUE_FULL).count(), 1, "{:?}", full.iter().map(|event| event.kind.as_str()).collect::<Vec<_>>());
        assert_eq!(full.iter().find(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_QUEUE_FULL).unwrap().payload["depth"], 3, "{full:?}");
        assert_eq!(control.held.len(), 3, "the refused command took no slot");
        // Release the first: its slot is still counted (UR-24: held plus released).
        time.advance_to(core.at(ms(4_997))).unwrap();
        control.release();
        assert_eq!((control.held.len(), control.released.len()), (2, 1), "{:?}", control.held.keys().collect::<Vec<_>>());
        control.book_timed(ActionId(21), key("radio.rx.c1.gain_db"), Value::num(7.0).unwrap(), at(7_000));
        assert_eq!(control.held.len(), 2, "the released one still fills its slot");
    }

    #[test]
    fn ur_24_a_timed_update_reaches_a_later_stream_s_pending_segment() {
        // UR-24 (R20): with no channel in force, a timed command reaches the later segment
        // that brings its channel — stream 1's, and not stream 0's.
        let (core, device, time, _events) = two_stream_rig();
        let _frozen = hold(&core, &time, &device);
        let (to_tx, _tx) = channel();
        let start = Start { t0: 0, rx: vec![Some((Channels::first(1), 200)), None], tx: vec![None] };
        let mut control = Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], core.description.defaults(), false, start);
        control.config.insert(key("radio.rx.c0.stream"), Value::from(0));
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        control.book_cold(ActionId(1), key("radio.rx.c1.stream"), Value::from(1), at(300));
        control.book_timed(ActionId(2), key("radio.rx.c1.frequency_hz"), Value::num(2.1e9).unwrap(), at(200));
        time.advance_to(core.at(ms(197))).unwrap();
        control.release();
        // UR-24 (R1): stream 1's segment was configured by uhd-control's own enable and nothing
        // streams the channel before it, so the value is applied untimed at its release and is
        // not given as a timed command (no sample would take it).
        let given: Vec<String> = device.calls().into_iter().filter(|call| call.starts_with("apply rx 1") && call.contains("freq=2100000000") && !call.ends_with("at=-")).collect();
        assert!(given.is_empty(), "{:?}", device.calls());
        let untimed: Vec<String> = device.calls().into_iter().filter(|call| call.starts_with("apply rx 1") && call.contains("freq=2100000000") && call.ends_with("at=-")).collect();
        assert_eq!(untimed.len(), 1, "{:?}", device.calls());
        assert!(!device.calls().iter().any(|call| call.starts_with("apply rx 0") && call.contains("freq=2100000000")), "{:?}", device.calls());
        assert_eq!(device.dsp_pending(Dir::Rx, 1), 0);
        // UR-24, design 38 D2 (3): the row is written when the instant comes, not when given.
        time.advance_to(core.at(ms(200))).unwrap();
        control.release();
        let row = lock(&core.rec).applied.iter().find(|row| row["key"] == "radio.rx.c1.frequency_hz" && row["issued"] == true).cloned().expect("the issued row");
        assert_eq!(row["note"], "no channel", "{row}");
    }

    #[test]
    fn ur_24_a_release_without_a_live_channel_reaches_the_pending_segment() {
        // UR-24 (S1): a timed gain due before the origin of the enable's segment, with no
        // channel in force at its instant, is given to the device as its own timed command on
        // that segment's channel and recorded `no channel`; the owner, whose record holds the
        // segment's command, starts it without configuring the direction again.
        let (core, device, time) = manual_rig();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        let ms = |n: i64| core.ticks(n * 1_000_000);
        channels_from_none(&mut control, &core, 2, 100, 1);
        let planned = plan_of(&core, Dir::Rx);
        let (by, origin) = (planned[0].segment.by.expect("the enable"), planned[0].segment.origin);
        let before = device.calls().len();
        assert!(owner_rx(&core).begin(), "the owner begins the enable's segment");
        assert_eq!((lock(&core.streams).stream(booked_stream(Dir::Rx)).configured, device.calls()[before..].iter().any(|call| call.starts_with("apply"))), (Some(by), false), "the owner configured a segment the record holds");
        let gain = key("radio.rx.c0.gain_db");
        let mark = device.calls().len();
        control.book_timed(ActionId(3), gain.clone(), Value::num(3.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(50)))));
        time.advance_to(core.at(ms(47))).unwrap();
        control.release();
        let timed: Vec<String> = device.calls()[mark..].iter().filter(|call| call.starts_with("apply rx 0") && !call.ends_with("at=-")).cloned().collect();
        assert_eq!(timed.len(), 1, "{:?}", device.calls());
        assert!(timed[0].contains("gain=3") && timed[0].contains(&format!("at={} effective={}", ms(50), ms(50))), "{timed:?}");
        assert_eq!(control.released, [ms(50)]);
        // UR-24, design 38 D2 (3): the row comes at the instant.
        time.advance_to(core.at(ms(50))).unwrap();
        control.release();
        let row = lock(&core.rec).applied.iter().rfind(|r| r["key"] == gain.as_str() && r["issued"] == true).cloned().expect("the record row");
        assert_eq!((row["at"]["ticks"].as_i64(), row["note"].as_str(), row["issued"].as_bool()), (Some(ms(50)), Some("no channel"), Some(true)), "{row}");
        device_at(&core, &time, &device, origin + core.ticks(1_000_000));
        assert_eq!(device.read_back(Dir::Rx, 0).gain, Some(3.0));
    }

    #[test]
    fn ur_24_a_cancelled_held_update_leaves_the_projection() {
        // UR-24 (S11c): cancelling a held update takes its entry out of every segment's
        // projection, so no configure applies it; the row records the cancellation, and the
        // record of the device's configuration stands as the enable left it.
        let (core, _, _) = rig();
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        channels_from_none(&mut control, &core, 2, 1000, 1);
        let by = plan_of(&core, Dir::Rx)[0].segment.by.expect("the enable");
        let gain = key("radio.rx.c0.gain_db");
        control.book_timed(ActionId(3), gain.clone(), Value::num(4.0).unwrap(), at(500));
        let settings = plan_of(&core, Dir::Rx)[0].settings.clone().expect("the enable's settings");
        assert_eq!(settings.settings_on(&core, &settings.updates.lock(), Dir::Rx, 0, ms(600), 0).gain, Some(4.0), "the projection holds the update");
        control.cancel_held("Provider::stop", core.now());
        assert_eq!(settings.settings_on(&core, &settings.updates.lock(), Dir::Rx, 0, ms(600), 0).gain, Some(0.0), "the cancellation left the entry in the projection");
        assert!(lock(&core.rec).applied.iter().any(|r| r["key"] == gain.as_str() && r["cancelled"] == "Provider::stop"), "{:?}", lock(&core.rec).applied);
        assert_eq!(lock(&core.streams).stream(booked_stream(Dir::Rx)).configured, Some(by), "the cancellation moved the record");
    }

    #[test]
    fn ur_24_a_command_waiting_for_a_channel_is_given_before_a_later_one() {
        // UR-24 (S12f): a command held for want of a channel is given once a segment brings one
        // at its instant, and it is given before a command at a later instant: the device
        // queue's order is the instant order (UC-2).
        let (core, device, time) = manual_rig();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[Dir::Tx]);
        control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        let us = |n: i64| core.ticks(n * 1_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(us(n))));
        let gain = key("radio.rx.c0.gain_db");
        lock(&core.streams).search(booked_stream(Dir::Rx), true);
        control.book_timed(ActionId(2), gain.clone(), Value::num(1.0).unwrap(), at(10_000));
        time.advance_to(core.at(us(7_500))).unwrap();
        control.release();
        control.book_timed(ActionId(3), gain.clone(), Value::num(2.0).unwrap(), at(11_000));
        time.advance_to(core.at(us(8_500))).unwrap();
        // A channel comes into force before the waiting command's instant.
        lock(&core.streams).search(booked_stream(Dir::Rx), false);
        control.book_cold(ActionId(4), key("radio.rx.c0.stream"), Value::from(0), at(9_500));
        control.release();
        let applied: Vec<(String, String)> = device.calls().into_iter()
            .filter(|call| call.starts_with("apply rx 0") && call.contains("gain=") && !call.ends_with("at=-"))
            .map(|call| {
                let gain = call.split("gain=").nth(1).unwrap().split(' ').next().unwrap().to_owned();
                let at = call.split("at=").nth(1).unwrap().split(' ').next().unwrap().to_owned();
                (gain, at)
            })
            .collect();
        assert_eq!(applied, [("1".to_owned(), us(10_000).to_string()), ("2".to_owned(), us(11_000).to_string())], "the device queue's order is not the effective order: {:?}", device.calls());
        // UR-24, design 38 D2 (3): the rows come at their instants.
        time.advance_to(core.at(us(11_000))).unwrap();
        control.release();
        assert_eq!(rows_of(&core, &gain), [(us(10_000), true, false), (us(11_000), true, false)]);
    }

    #[test]
    fn ur_24_a_command_waiting_for_a_channel_keeps_a_later_booking_behind_it() {
        // UR-24: a command held for want of a channel moves no later booking: the one booked
        // after it, due earlier, is given first, and the channel a `cold` change brought before
        // the next pass gives the waiting one its own instant there — the device's order is the
        // instant order (UC-2), and no release moves an instant the booking settled.
        let (core, device, time, events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[Dir::Tx]);
        control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        let us = |n: i64| core.ticks(n * 1_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(us(n))));
        let (rx, tx) = (key("radio.rx.c0.gain_db"), key("radio.tx.c0.gain_db"));
        time.advance_to(core.at(us(97_600))).unwrap();
        control.book_timed(ActionId(1), rx.clone(), Value::num(3.0).unwrap(), at(101_000));
        control.release();
        assert_eq!(control.held.keys().map(|(e, _)| *e).collect::<Vec<_>>(), [us(101_000)], "a command with no channel is not held");
        control.book_timed(ActionId(2), tx.clone(), Value::num(4.0).unwrap(), at(99_800));
        lock(&core.streams).search(booked_stream(Dir::Rx), false);
        control.book_cold(ActionId(3), key("radio.rx.c0.stream"), Value::from(0), at(100_800));
        control.release();
        time.advance_to(core.at(us(98_000))).unwrap();
        control.release();
        let applied: Vec<(String, i64)> = device.calls().into_iter()
            .filter(|call| call.starts_with("apply") && call.contains("gain=") && !call.ends_with("at=-"))
            .map(|call| (call.split(' ').nth(1).expect("a direction").to_owned(), call.split("at=").nth(1).expect("an instant").split(' ').next().unwrap().parse().unwrap()))
            .collect();
        assert_eq!(applied, [("tx".to_owned(), us(99_800)), ("rx".to_owned(), us(101_000))], "the device's order is not the instant order: {:?}", device.calls());
        assert!(events_of(&events, kinds::LATE_COMMAND).is_empty(), "a command outside the window was made late");
        time.advance_to(core.at(us(101_000))).unwrap();
        control.release();
        assert_eq!((rows_of(&core, &tx), rows_of(&core, &rx)), (vec![(us(99_800), true, false)], vec![(us(101_000), true, false)]));
    }

    #[test]
    fn ur_24_a_command_waiting_for_a_channel_holds_back_the_one_behind_it() {
        // UR-24: the release stops at a command held for want of a channel, so one behind it,
        // due later and with a channel in force, is not given while it waits; once its instant
        // passes with no channel it is recorded `no channel`, the configuration of record from
        // then on, and the one behind it is given at its own instant.
        let (core, device, time) = manual_rig();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[Dir::Tx]);
        control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        let us = |n: i64| core.ticks(n * 1_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(us(n))));
        let (rx, tx) = (key("radio.rx.c0.gain_db"), key("radio.tx.c0.gain_db"));
        time.advance_to(core.at(us(98_500))).unwrap();
        control.book_timed(ActionId(1), rx.clone(), Value::num(3.0).unwrap(), at(100_500));
        control.book_timed(ActionId(2), tx.clone(), Value::num(4.0).unwrap(), at(101_000));
        control.release();
        let given = |calls: &[String]| calls.iter().filter(|call| call.starts_with("apply") && call.contains("gain=") && !call.ends_with("at=-")).count();
        assert_eq!((control.held.len(), given(&device.calls())), (2, 0), "the command behind the waiting one was given: {:?}", device.calls());
        time.advance_to(core.at(us(100_600))).unwrap();
        control.release();
        let applied: Vec<(String, i64)> = device.calls().into_iter()
            .filter(|call| call.starts_with("apply") && call.contains("gain=") && !call.ends_with("at=-"))
            .map(|call| (call.split(' ').nth(1).expect("a direction").to_owned(), call.split("at=").nth(1).expect("an instant").split(' ').next().unwrap().parse().unwrap()))
            .collect();
        assert_eq!(applied, [("tx".to_owned(), us(101_000))], "{:?}", device.calls());
        // UR-24, design 38 D2 (3): the given row comes at its instant; the `no channel` one
        // was written when its instant passed.
        time.advance_to(core.at(us(101_000))).unwrap();
        control.release();
        assert_eq!((rows_of(&core, &rx), rows_of(&core, &tx)), (vec![(us(100_500), false, true)], vec![(us(101_000), true, false)]));
        assert_eq!(control.config_at(i64::MAX)[&rx], Value::num(3.0).unwrap(), "the command whose instant passed with no channel is not the configuration of record");
    }

    #[test]
    fn ur_24_a_stop_cancels_a_command_waiting_for_a_channel() {
        // UR-24, UR-26, UR-36 (S11d): a command held for want of a channel was never the
        // device's; `Provider::stop` cancels it before its instant, takes its entry out of every
        // projection, and the instant passes with no release row and no device call.
        let (core, device, time) = manual_rig();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[Dir::Tx]);
        control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        let us = |n: i64| core.ticks(n * 1_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(us(n))));
        channels_from_none(&mut control, &core, 2, 100, 1);
        control.book_cold(ActionId(3), key("radio.rx.c0.stream"), Value::from(-1), at(100_200));
        let projection = plan_of(&core, Dir::Rx)[0].settings.clone().expect("the enable's settings");
        time.advance_to(core.at(us(97_500))).unwrap();
        control.book_timed(ActionId(4), key("radio.tx.c0.gain_db"), Value::num(1.0).unwrap(), at(100_500));
        control.release();
        let gain = key("radio.rx.c0.gain_db");
        time.advance_to(core.at(us(97_600))).unwrap();
        control.book_timed(ActionId(5), gain.clone(), Value::num(3.0).unwrap(), at(99_700));
        control.release();
        control.cancel_held("Provider::stop", core.now());
        assert_eq!(projection.settings_on(&core, &projection.updates.lock(), Dir::Rx, 0, us(100_500), 0).gain, Some(0.0), "the cancelled command stayed in the projection");
        device_at(&core, &time, &device, us(101_000));
        control.release();
        assert_eq!((rows_of(&core, &gain), device.calls().iter().filter(|call| call.contains("gain=3")).count()), (vec![(us(100_500), false, false)], 0), "the cancelled command was released or applied");
    }

    #[test]
    fn ur_17_a_prune_keeps_the_device_record_and_the_configure_mark() {
        // UR-17 (S16): pruning an ended segment leaves the stream's device record alone: its set
        // and its configure mark stay, so the next segment is not configured again for nothing.
        let (core, _, _) = rig();
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        control.book_cold(ActionId(1), key("radio.rx.s0.sample_rate_hz"), Value::num(2e6).unwrap(), at(100));
        control.book_cold(ActionId(1), key("radio.rx.s0.sample_rate_hz"), Value::num(1e6).unwrap(), at(200));
        let rx = booked_stream(Dir::Rx);
        let second = plan_of(&core, Dir::Rx)[2].segment.by.unwrap();
        lock(&core.streams).record_open(rx, Some(Channels::first(1)));
        lock(&core.streams).record_configured(rx, Some(second));
        lock(&core.streams).prune(rx, Some(second));
        let streams = lock(&core.streams);
        assert_eq!(streams.stream(rx).set, super::super::core::DeviceSet::Open(Channels::first(1)));
        assert_eq!(streams.stream(rx).configured, Some(second));
    }

    /// Books a receive-channel membership change at `ms` from no stream (UR-13): the
    /// owner is idle, as its `read` leaves it.
    fn channels_from_none(control: &mut Control, core: &Core, id: u64, ms: i64, channels: i64) {
        lock(&core.streams).search(booked_stream(Dir::Rx), false);
        control.book_cold(ActionId(id), key("radio.rx.c0.stream"), Value::from(if channels > 0 { 0 } else { -1 }), Some(AbsoluteDeadline::new(core.at(core.ticks(ms * 1_000_000)))));
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
        // UR-13: uhd-control opens B = {0,1} from no stream, then C = {0}; before the owner
        // begins B, it must reopen B rather than mistake C's streamer for B's.
        let (core, device, _) = rig();
        let (mut control, _tx) = control(&core, &[]);
        let at = |ms: i64| Some(AbsoluteDeadline::new(core.at(core.ticks(ms * 1_000_000))));
        control.config.insert(key("radio.rx.c0.stream"), Value::from(0));
        control.config.insert(key("radio.rx.c1.stream"), Value::from(0));
        control.book_cold(ActionId(2), key("radio.rx.s0.sample_rate_hz"), Value::num(1e6).unwrap(), at(100));
        control.book_cold(ActionId(3), key("radio.rx.c1.stream"), Value::from(-1), at(200));
        control.book_cold(ActionId(4), key("radio.rx.c0.stream"), Value::from(-1), at(200));
        control.book_cold(ActionId(5), key("radio.rx.c0.stream"), Value::from(0), at(300));
        let opened_by_control = owner_rx_calls(&device.calls());
        assert_eq!(opened_by_control, ["rx_open 0 [0, 1]", "rx_open 0 [0]"], "{opened_by_control:?}");
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
        // RM-25 (design 43): the refused enable ran for planning until the next change cut it, so
        // the next enable begins where it does with nothing refused.
        let (plain, _, _) = rig();
        let (mut plain_control, _plain_tx) = self::control(&plain, &[]);
        channels_from_none(&mut plain_control, &plain, 2, 100, 1);
        channels_from_none(&mut plain_control, &plain, 3, 150, 0);
        channels_from_none(&mut plain_control, &plain, 4, 200, 1);
        let origins = |core: &Core| plan_of(core, Dir::Rx).iter().map(|planned| planned.segment.origin).collect::<Vec<_>>();
        assert_eq!(origins(&core).last(), origins(&plain).last(), "the next enable's origin is the unrefused plan's");
    }

    #[test]
    fn ur_14_booking_does_not_wait_for_a_future_cold_switch() {
        let (core, _device, time) = rig();
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        // On its own thread, so that a booking that waited for the device instant, which
        // never comes on this clock, fails the test instead of hanging it (UR-14).
        let (done, finished) = channel();
        let at = core.at(core.ticks(1_000_000_000));
        let booking = std::thread::spawn(move || {
            control.book_cold(ActionId(1), key("radio.rx.s0.sample_rate_hz"), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(at)));
            control.book_timed(ActionId(1), key("radio.rx.c0.gain_db"), Value::num(3.0).unwrap(), None);
            control.release();
            let _ = done.send(control);
        });
        let mut control = finished.recv_timeout(Wall::from_secs(10)).expect("uhd-control waited for a device instant");
        booking.join().unwrap();
        assert_eq!(core.now(), 0, "booking never advanced to the future device instant");
        // The plan cuts the stream at 1 s's sample (1 000 000 at 1 MS/s) and starts the next
        // segment after the receive call there, 3 ms and the start lead (RM-25).
        let plan = plan_of(&core, Dir::Rx);
        assert_eq!(plan[0].segment.cut, Some(1_000_000));
        assert_eq!(plan[1].segment.origin, core.ticks(1_055_000_000));
        // UR-24, design 38 D2 (3): the row comes at the command's instant.
        time.advance_to(core.at(core.ticks(2_000_000))).unwrap();
        control.release();
        assert!(lock(&core.rec).applied.iter().any(|r|
            r["key"] == "radio.rx.c0.gain_db" && r["issued"] == true && r["at"]["ticks"] == core.ticks(2_000_000)));
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
    fn ur_21_production_booking_preserves_the_original_repeat_length() {
        let samples: Vec<[f32; 2]> = (0..997).map(|i| [i as f32 / 1_000.0, 0.25]).collect();
        let bytes: Vec<u8> = samples.iter().flat_map(|sample| [sample[0].to_le_bytes(), sample[1].to_le_bytes()].concat()).collect();
        let hash = ezsdr_kernel::hash::ContentHash::of_bytes(&bytes);
        let waveform = ezsdr_kernel::manifest::ingest_input(
            ezsdr_kernel::spec::Ident::parse("waveform").unwrap(),
            ezsdr_kernel::spec::Namespace::parse("ezsdr.input").unwrap(),
            format!("mem:{hash}"),
            &bytes,
        );
        let (core, _device, time, _) = crate::provider::test_support::rig_with_inputs(BTreeMap::from([(hash, Arc::<[u8]>::from(bytes))]));
        let (mut control, to_tx) = control(&core, &[Dir::Tx]);
        let clock = plan_of(&core, Dir::Tx)[0].clock.expect("the transmit clock");
        time.advance_to(core.at(core.ticks(10_000_000))).unwrap();
        control.book(Dispatched::new(ActionId(1), Action::TxBurst {
            target: super::super::test_support::target_of(&core.tx_id),
            waveform,
            repeat: true,
            at: AbsoluteDeadline::new(TimePoint::new(clock.domain, 20_000)),
            requested_at: None,
            late_policy: ezsdr_kernel::stream::LatePolicy::DropAndFlag,
            metadata: BTreeMap::new(),
        }, Some(core.tx_id.clone())));
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
        let waveform = ezsdr_kernel::manifest::ingest_input(
            ezsdr_kernel::spec::Ident::parse("waveform").unwrap(),
            ezsdr_kernel::spec::Namespace::parse("ezsdr.input").unwrap(),
            format!("mem:{hash}"),
            &bytes,
        );
        let (core, _device, time, _events) = crate::provider::test_support::rig_with_inputs(BTreeMap::from([(hash, Arc::<[u8]>::from(bytes))]));
        let (mut control, to_tx) = control(&core, &[Dir::Tx]);
        let clock = plan_of(&core, Dir::Tx)[0].clock.expect("the transmit clock");
        time.advance_to(core.at(core.ticks(10_000_000))).unwrap();
        // On time at 20 ms, asked for at 5 ms.
        let requested = TimePoint::new(clock.domain, 5_000);
        control.book(Dispatched::new(ActionId(1), Action::TxBurst {
            target: super::super::test_support::target_of(&core.tx_id),
            waveform: waveform.clone(),
            repeat: false,
            at: AbsoluteDeadline::new(TimePoint::new(clock.domain, 20_000)),
            requested_at: Some(AbsoluteDeadline::new(requested)),
            late_policy: ezsdr_kernel::stream::LatePolicy::SendAsapAndFlag,
            metadata: BTreeMap::new(),
        }, Some(core.tx_id.clone())));
        let TxCmd::Burst(held) = to_tx.try_recv().expect("the receipt site handed the burst to uhd-tx") else { panic!("not a burst") };
        assert_eq!(held.k, 20_000, "an on-time burst starts at its `at`");
        assert_eq!(held.open.requested_target, Some(requested), "the on-time burst's requested target is the `requested_at`");
        // Late: its `at` is behind the booking, asked for at 2 ms; `SendAsap` moves the start.
        let requested = TimePoint::new(clock.domain, 2_000);
        control.book(Dispatched::new(ActionId(2), Action::TxBurst {
            target: super::super::test_support::target_of(&core.tx_id),
            waveform,
            repeat: false,
            at: AbsoluteDeadline::new(TimePoint::new(clock.domain, 0)),
            requested_at: Some(AbsoluteDeadline::new(requested)),
            late_policy: ezsdr_kernel::stream::LatePolicy::SendAsapAndFlag,
            metadata: BTreeMap::new(),
        }, Some(core.tx_id.clone())));
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
        let waveform = ezsdr_kernel::manifest::ingest_input(
            ezsdr_kernel::spec::Ident::parse("waveform").unwrap(),
            ezsdr_kernel::spec::Namespace::parse("ezsdr.input").unwrap(),
            format!("mem:{hash}"),
            &bytes,
        );
        let (core, _device, _time, events) = crate::provider::test_support::rig_with_inputs(BTreeMap::from([(hash, Arc::<[u8]>::from(bytes))]));
        let (mut control, to_tx) = control(&core, &[Dir::Tx]);
        core.start_up.store(core.ticks(1_000_000_000), std::sync::atomic::Ordering::Release);
        let clock = plan_of(&core, Dir::Tx)[0].clock.expect("the transmit clock");
        // 20 ms ahead: past the device lead, so the late policy is not what refuses it, and
        // before the start-up.
        let target = TimePoint::new(clock.domain, 20_000);
        control.book(Dispatched::new(ActionId(1), Action::TxBurst {
            target: super::super::test_support::target_of(&core.tx_id),
            waveform,
            repeat: false,
            at: AbsoluteDeadline::new(target),
            requested_at: None,
            late_policy: ezsdr_kernel::stream::LatePolicy::DropAndFlag,
            metadata: BTreeMap::new(),
        }, Some(core.tx_id.clone())));
        let rejected = lock(&core.rec).rejected.clone();
        assert!(rejected.iter().any(|row| row["reason"].as_str().unwrap_or("").starts_with("UR-21: the burst begins before the device's start-up")), "{rejected:?}");
        assert!(to_tx.try_recv().is_err(), "a refused burst was handed to uhd-tx");
        // The refusal reaches the caller as `COMMAND_REJECTED`, naming the Action it answers.
        let event = events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
            .find(|event| event.kind.as_str() == kinds::COMMAND_REJECTED).expect("COMMAND_REJECTED");
        assert_eq!(event.payload["reason"], "UR-21: the burst begins before the device's start-up", "{event:?}");
        assert_eq!(event.payload["action"], 1, "{event:?}");
    }

    #[test]
    fn ur_21_a_burst_before_its_clock_s_origin_is_late() {
        // RM-14, RM-15, KC-21a: a start before a new transmit clock's origin is decided at
        // the device lead before that origin (`timeline::decided_at`), so a late policy moves
        // it onto the origin rather than refusing it for want of a clock. On the controlled
        // clock: a Spec cannot reserve this — its offsets resolve to the clock declared at
        // prepare, and a target on a later clock's domain is refused as another clock's.
        let samples: Vec<(f32, f32)> = vec![(0.25, 0.0), (0.5, 0.0), (0.75, 0.0), (1.0, 0.0)];
        let bytes: Vec<u8> = samples.iter().flat_map(|(re, im)| [re.to_le_bytes(), im.to_le_bytes()].concat()).collect();
        let hash = ezsdr_kernel::hash::ContentHash::of_bytes(&bytes);
        let waveform = ezsdr_kernel::manifest::ingest_input(
            ezsdr_kernel::spec::Ident::parse("waveform").unwrap(),
            ezsdr_kernel::spec::Namespace::parse("ezsdr.input").unwrap(),
            format!("mem:{hash}"),
            &bytes,
        );
        let (core, _device, _time, events) = crate::provider::test_support::rig_with_inputs(BTreeMap::from([(hash, Arc::<[u8]>::from(bytes))]));
        let (mut control, to_tx) = control(&core, &[Dir::Tx]);
        // Booked now: the new clock's origin is its start lead after the old segment's cut,
        // 50 ms at 1 MS/s to 2 MS/s (100 root ticks per sample).
        control.book_cold(ActionId(1), key("radio.tx.s0.sample_rate_hz"), Value::num(2e6).unwrap(), None);
        let plan = plan_of(&core, Dir::Tx);
        assert_eq!(plan.len(), 2, "the change's segment is planned");
        let clock = plan[1].clock.expect("the change's clock");
        assert_eq!(plan[1].segment.origin, core.ticks(50_000_000), "the new clock's origin");
        // 5 ms before that origin: more than the 2 ms device lead away, so the booking is late
        // and the moved start is the origin (10 000 samples at 2 MS/s).
        let target = TimePoint::new(clock.domain, -10_000);
        control.book(Dispatched::new(ActionId(2), Action::TxBurst {
            target: super::super::test_support::target_of(&core.tx_id),
            waveform,
            repeat: false,
            at: AbsoluteDeadline::new(target),
            requested_at: None,
            late_policy: ezsdr_kernel::stream::LatePolicy::SendAsapAndFlag,
            metadata: BTreeMap::new(),
        }, Some(core.tx_id.clone())));
        let verdict = events.drain(ezsdr_kernel::time::TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
            .find(|e| e.kind.as_str() == kinds::TIME_ERROR).expect("the SendAsap verdict at booking");
        assert_eq!(verdict.payload["outcome"], "send_asap", "{verdict:?}");
        assert_eq!(verdict.payload["target"], serde_json::to_value(target).unwrap(), "{verdict:?}");
        let TxCmd::Burst(held) = to_tx.try_recv().expect("the receipt site handed the burst to uhd-tx") else { panic!("not a burst") };
        assert_eq!(held.k, 0, "the moved start is not the new clock's origin");
        assert_eq!(clock.instant(held.k), plan[1].segment.origin);
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
        let waveform = ezsdr_kernel::manifest::ingest_input(
            ezsdr_kernel::spec::Ident::parse("waveform").unwrap(),
            ezsdr_kernel::spec::Namespace::parse("ezsdr.input").unwrap(),
            format!("mem:{hash}"),
            &bytes,
        );
        let (core, _device, time, events) = crate::provider::test_support::rig_with_inputs(BTreeMap::from([(hash, Arc::<[u8]>::from(bytes))]));
        let (mut control, to_tx) = control(&core, &[Dir::Tx]);
        let clock = plan_of(&core, Dir::Tx)[0].clock.expect("the transmit clock");
        let lead = core.ticks(crate::profile::DEVICE_LEAD_NS + crate::profile::DELIVERY_ALLOWANCE_NS);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        // (milliseconds from receipt to uhd-control's read, whether the burst is late): 1 ms is a poll, 3 ms leaves exactly the device lead.
        for (n, (read_after, late)) in [(1, false), (3, false), (4, true)].into_iter().enumerate() {
            let receipt = ms(100 * (n as i64 + 1));
            time.advance_to(core.at(receipt)).unwrap();
            let target = TimePoint::new(clock.domain, clock.at_or_after(receipt + lead));
            time.advance_to(core.at(receipt + ms(read_after))).unwrap();
            control.book(Dispatched::new(ActionId(n as u64 + 1), Action::TxBurst {
                target: super::super::test_support::target_of(&core.tx_id),
                waveform: waveform.clone(),
                repeat: false,
                at: AbsoluteDeadline::new(target),
                requested_at: None,
                late_policy: ezsdr_kernel::stream::LatePolicy::DropAndFlag,
                metadata: BTreeMap::new(),
            }, Some(core.tx_id.clone())));
            let errors: Vec<_> = events.drain(ezsdr_kernel::time::TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|e| e.kind.as_str() == kinds::TIME_ERROR).collect();
            assert_eq!(errors.len(), usize::from(late), "read {read_after} ms after receipt: {errors:?}");
            assert_eq!(to_tx.try_recv().is_ok(), !late, "read {read_after} ms after receipt: a late `drop_and_flag` burst is not handed to uhd-tx");
            if late { assert_eq!(errors[0].payload["outcome"], "drop", "{errors:?}"); }
        }
    }

    #[test]
    fn ur_21_an_untimed_burst_after_a_cold_change_is_on_time_at_its_clock_s_origin() {
        // RS-19, on the controlled clock: with no `at`, a burst after a cold rate change is stamped at the new transmit clock's
        // origin, a 50 ms start lead after the change. uhd-control reads it a poll later and it is on time at the origin; read
        // 49 ms after the change, with less than the 2 ms device lead left, it is late.
        let samples: Vec<(f32, f32)> = vec![(0.25, 0.0), (0.5, 0.0)];
        let bytes: Vec<u8> = samples.iter().flat_map(|(re, im)| [re.to_le_bytes(), im.to_le_bytes()].concat()).collect();
        let hash = ezsdr_kernel::hash::ContentHash::of_bytes(&bytes);
        let waveform = ezsdr_kernel::manifest::ingest_input(
            ezsdr_kernel::spec::Ident::parse("waveform").unwrap(),
            ezsdr_kernel::spec::Namespace::parse("ezsdr.input").unwrap(),
            format!("mem:{hash}"),
            &bytes,
        );
        for (read_after_ms, late) in [(1, false), (49, true)] {
            let (core, _device, time, events) = crate::provider::test_support::rig_with_inputs(BTreeMap::from([(hash.clone(), Arc::<[u8]>::from(bytes.clone()))]));
            let (mut control, to_tx) = control(&core, &[Dir::Tx]);
            control.book_cold(ActionId(1), key("radio.tx.s0.sample_rate_hz"), Value::num(2e6).unwrap(), None);
            let plan = plan_of(&core, Dir::Tx);
            let clock = plan[1].clock.expect("the change's clock");
            assert_eq!(plan[1].segment.origin, core.ticks(50_000_000), "the new clock's origin");
            time.advance_to(core.at(core.ticks(read_after_ms * 1_000_000))).unwrap();
            events.drain(ezsdr_kernel::time::TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0));
            control.book(Dispatched::new(ActionId(2), Action::TxBurst {
                target: super::super::test_support::target_of(&core.tx_id),
                waveform: waveform.clone(),
                repeat: false,
                at: AbsoluteDeadline::new(TimePoint::new(clock.domain, 0)),
                requested_at: None,
                late_policy: ezsdr_kernel::stream::LatePolicy::DropAndFlag,
                metadata: BTreeMap::new(),
            }, Some(core.tx_id.clone())));
            let errors = events.drain(ezsdr_kernel::time::TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|e| e.kind.as_str() == kinds::TIME_ERROR).count();
            assert_eq!(errors, usize::from(late), "read {read_after_ms} ms after the change");
            match to_tx.try_recv() {
                Ok(TxCmd::Burst(held)) => assert!(!late && held.k == 0, "the burst starts at the origin: {}", held.k),
                _ => assert!(late, "an on-time burst is handed to uhd-tx"),
            }
        }
    }

    #[test]
    fn ur_30_the_recorded_plan_has_no_refused_segment() {
        // UR-30, RM-25: a refusal the owner booked is in the one plan `Provider::stop` records,
        // which has no segment for it.
        let (core, _, time) = rig();
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        control.book_cold(ActionId(1), key("radio.rx.s0.sample_rate_hz"), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(100)))));
        control.book_cold(ActionId(1), key("radio.rx.s0.sample_rate_hz"), Value::num(1e6).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(300)))));
        let refused = plan_of(&core, Dir::Rx)[1].segment.by.unwrap();
        assert!(lock(&core.streams).refuse(&core, booked_stream(Dir::Rx), refused));
        time.advance_to(core.at(ms(500))).unwrap();
        // Design 38 D2 (4): the end's `S` is the stop's, booked explicitly as `Provider::stop`
        // books it (UR-26); `finish` reads no other.
        lock(&core.streams).stop = Some((ms(500), ezsdr_kernel::module_api::StopMode::Orderly));
        lock(&core.streams).end(&core, ms(500), false, false);
        control.finish();
        let rec = lock(&core.rec);
        let row = rec.timing.iter().find(|r| r["what"] == "plan").expect("the plan row");
        let segments: Vec<(i64, i64)> = row["rx"].as_array().unwrap().iter().map(|s| (s["origin"].as_i64().unwrap(), s["ticks_per_sample"][0].as_i64().unwrap())).collect();
        // The first segment and the second change's, at 1 MS/s; none at the refused 2 MS/s. The
        // refused segment runs for planning until the second change cuts it at 300 ms, so the second
        // begins after that cut, the receive call in progress (1 ms at 2 MS/s), the allowance (3 ms)
        // and the lead (50 ms): 354 ms.
        assert_eq!(segments, [(0, 200), (ms(354), 200)], "{row}");
        // The row names the command that began each: the second change's arrival order is 2.
        let by: Vec<Option<u64>> = row["rx"].as_array().unwrap().iter().map(|s| s["sequence"].as_u64()).collect();
        assert_eq!(by, [None, Some(2)], "{row}");
    }

    #[test]
    fn ur_25_a_cold_change_that_arrives_after_a_later_one_follows_it() {
        // RM-25, VH-2: a `cold` change never takes effect before one of its stream booked
        // earlier: it takes that one's instant, after it in the plan, and is late (UC-2).
        let (core, _, _, events) = crate::provider::test_support::rig_with_links(vec![Arc::new(NoLink)]);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        control.book_cold(ActionId(1), key("radio.rx.s0.sample_rate_hz"), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(300)))));
        control.book_cold(ActionId(1), key("radio.rx.s0.sample_rate_hz"), Value::num(1e6).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(100)))));
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
            if let Some(k) = delivered {
                let mut streams = lock(&core.streams);
                streams.started(booked_stream(Dir::Rx), Some((0, config, None)), 0, config);
                streams.delivered(booked_stream(Dir::Rx), 0, config, k);
            }
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
        control.book_cold(ActionId(1), key("radio.tx.s0.sample_rate_hz"), Value::num(2e6).unwrap(), None);
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
            control.book_cold(ActionId(1), key(&format!("radio.{}.s0.sample_rate_hz", dir.name())), Value::num(2e6).unwrap(),
                Some(AbsoluteDeadline::new(core.at(core.ticks(1_000_000_000)))));
            let gain = key(&format!("radio.{}.c0.gain_db", dir.name()));
            control.book_timed(ActionId(1), gain.clone(), Value::num(3.0).unwrap(), None);
            control.release();
            assert_eq!(control.config_at(i64::MAX)[&gain], Value::num(3.0).unwrap());
            assert!(lock(&core.rec).rejected.is_empty());
            let plan = plan_of(&core, dir);
            plan[1].configure(&core, dir).unwrap();
            observed.push((dir.name(), device.read_back(dir, 0).gain));
        }
        assert_eq!(observed, vec![("rx", Some(3.0)), ("tx", Some(3.0))], "both cold switches must preserve the later update due before the new origin");
    }

    #[test]
    fn ur_25_switch_projects_timed_updates_in_effective_order() {
        // UC-2, UR-25: a segment's configuration applies the timed updates in instant order,
        // and one at its origin's own instant counts.
        let (core, device, _, _) = crate::provider::test_support::rig();
        device.rx_open(0, &[0]).unwrap();
        let mut base = core.description.defaults();
        base.insert(key("radio.rx.c0.gain_db"), Value::num(0.0).unwrap());
        base.insert(key("radio.rx.c0.frequency_hz"), Value::num(1e9).unwrap());
        let cold = ColdConfig::new(base);
        cold.book((70, 1), 0, Settings { gain: Some(7.0), ..Settings::default() });
        cold.book((50, 2), 0, Settings { gain: Some(5.0), ..Settings::default() });
        cold.book((101, 3), 0, Settings { gain: Some(9.0), freq: Some(2e9), ..Settings::default() });
        cold.book((100, 4), 0, Settings { freq: Some(1.1e9), ..Settings::default() });
        assert_eq!(cold.settings_on(&core, &cold.updates.lock(), Dir::Rx, 0, 49, 0).gain, Some(0.0));
        let segment = ezsdr_radio::timeline::Segment {
            origin: 100,
            cut: None,
            config: Config { channels: Channels::first(1), ratio: Rational::new(200, 1).unwrap() },
            by: Some(4),
        };
        cold.configure(&core, Dir::Rx, &segment, 0).unwrap();
        let applied = device.read_back(Dir::Rx, 0);
        assert_eq!((applied.gain, applied.freq), (Some(7.0), Some(1.1e9)));
        let updates = cold.updates.lock();
        let after = cold.settings_on(&core, &updates, Dir::Rx, 0, 101, 0);
        assert_eq!((after.gain, after.freq), (Some(9.0), Some(2e9)));
    }

    #[test]
    fn ur_26_a_stop_cancels_no_update() {
        // RM-16, UR-26 (spec 22, VH-6): a `Stop` of a stream or of the device cancels no held
        // timed command, and the projection of a segment booked after it keeps it; only
        // `Provider::stop` drops what is held.
        for target in ["usrp/rx/s0", "usrp/tx/s0", "usrp"] {
            let (core, device, _) = rig();
            device.tx_open(0, &[0]).unwrap();
            let (mut control, _tx) = control(&core, &[Dir::Rx, Dir::Tx]);
            let gain = key("radio.tx.c0.gain_db");
            control.book_timed(ActionId(1), gain.clone(), Value::num(3.0).unwrap(), Some(AbsoluteDeadline::new(core.at(core.ticks(1_040_000_000)))));
            let node = ezsdr_kernel::id::ResourceId::parse(target).unwrap();
            control.book(Dispatched::new(ActionId(1), Action::Stop { target: Some(super::super::test_support::target_of(&node)) }, Some(node)));
            control.book_cold(ActionId(1), key("radio.tx.s0.sample_rate_hz"), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(core.at(core.ticks(1_000_000_000)))));
            assert_eq!(control.held.len(), 1, "{target}");
            assert!(!lock(&core.rec).applied.iter().any(|r| r.get("cancelled").is_some()), "{target}");
            let plan = plan_of(&core, Dir::Tx);
            let settings = plan[1].settings.clone().unwrap();
            assert_eq!(settings.settings_on(&core, &settings.updates.lock(), Dir::Tx, 0, plan[1].segment.origin, 0).gain, Some(3.0), "{target}");
            control.cancel_held("Provider::stop", core.now());
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
                control.book_cold(ActionId(1), key(&format!("radio.{}.s0.sample_rate_hz", dir.name())), Value::num(2e6).unwrap(),
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
        control.book_cold(ActionId(1), key("radio.tx.c0.stream"), Value::from(-1), Some(AbsoluteDeadline::new(core.at(aligned_max))));
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
                        let name = if cold {
                            format!("radio.{}.s0.sample_rate_hz", dir.name())
                        } else {
                            format!("radio.{}.c0.gain_db", dir.name())
                        };
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
            control.book_timed(ActionId(1), key("radio.tx.c0.gain_db"), Value::num(3.0).unwrap(), at);
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
        control.book_cold(ActionId(1), key("radio.tx.c0.stream"), Value::from(-1), Some(AbsoluteDeadline::new(core.at(core.ticks(100_000_000)))));
        for (ms, gain) in [(50, 1.0), (50, 2.0), (150, 4.0)] {
            control.book_timed(ActionId(1), key("radio.tx.c0.gain_db"), Value::num(gain).unwrap(), Some(AbsoluteDeadline::new(core.at(core.ticks(ms * 1_000_000)))));
        }
        time.advance_to(core.at(core.ticks(60_000_000))).unwrap();
        control.release();
        time.advance_to(core.at(core.ticks(160_000_000))).unwrap();
        control.release();
        let rows: Vec<_> = lock(&core.rec).applied.iter().filter(|r| r["key"] == "radio.tx.c0.gain_db")
            .map(|r| (r["claimed"].as_f64().unwrap(), r["issued"] == true, r["note"] == "no channel")).collect();
        assert_eq!(rows, [(1.0, true, false), (2.0, true, false), (4.0, false, true)]);
        assert_eq!(device.calls().iter().filter(|call| call.starts_with("apply tx")).count(), 2, "{:?}", device.calls());
    }

    #[test]
    fn ur_24_a_direction_halted_by_a_refusal_has_no_channel() {
        // #64, RM-25, UR-24 (S13): a refused change stays the configuration of record, and the
        // direction it halts has no channel in force for a timed gain due before the enable
        // that starts it again — so the gain is given to the device at its own instant on that
        // enable's channel and recorded `no channel`, and the enable's own configuration then
        // holds it too. A direction a `Stop` turned off is not halted: its gain is issued.
        for (dir, halt, enable) in [(Dir::Rx, true, "cold"), (Dir::Rx, true, "start_rx"), (Dir::Tx, true, "cold"), (Dir::Rx, false, "start_rx")] {
            let case = format!("{} halt={halt} {enable}", dir.name());
            let (core, device, time) = rig();
            match dir { Dir::Rx => device.rx_open(0, &[0]).unwrap(), Dir::Tx => device.tx_open(0, &[0]).unwrap() }
            let (mut control, _tx) = control(&core, &[dir]);
            let ms = |n: i64| core.ticks(n * 1_000_000);
            let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
            let rate = key(&format!("radio.{}.s0.sample_rate_hz", dir.name()));
            let gain = key(&format!("radio.{}.c0.gain_db", dir.name()));
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
            let applied = device.calls().iter().filter(|call| call.starts_with(&format!("apply {}", dir.name())) && call.contains("gain=3") && call.contains(&format!("at={} ", ms(200)))).count();
            // UR-24, design 38 D2 (3): the row comes at the instant.
            time.advance_to(core.at(ms(200))).unwrap();
            control.release();
            let row: Vec<_> = lock(&core.rec).applied.iter().filter(|r| r["key"] == gain.as_str()).map(|r| (r["at"]["ticks"].as_i64(), r["issued"] == true, r["note"] == "no channel")).collect();
            assert_eq!((applied, row), (1, vec![(Some(ms(200)), true, halt)]), "{case}: {:?}", device.calls());
            assert_eq!(control.config_at(ms(200))[&gain], Value::num(3.0).unwrap(), "{case}: the recorded value is of record from its instant");
            if halt {
                assert_eq!(control.config_at(ms(200))[&rate], Value::num(2e6).unwrap(), "{case}: the refused value is of record");
            }
            let planned = plan_of(&core, dir).pop().unwrap();
            assert!(planned.segment.origin >= ms(300), "{case}");
            let settings = planned.settings.unwrap();
            assert_eq!(settings.settings_on(&core, &settings.updates.lock(), dir, 0, planned.segment.origin, 0).gain, Some(3.0), "{case}");
        }
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
        let gain = key("radio.rx.c0.gain_db");
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
    fn ur_24_hardware_timed_updates() {
        // UR-24, UC-6 (design note 37): the device lead, a late update's applied instant, the
        // command queue's depth and both directions. Booked on the controlled clock, so the
        // 1 ms and 40 ms instants are fixed whatever the test thread does.
        let (core, device, time, events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        device.rx_open(0, &[0]).unwrap();
        device.tx_open(0, &[0]).unwrap();
        let (mut control, _tx) = control(&core, &[Dir::Rx, Dir::Tx]);
        let us = |n: i64| core.ticks(n * 1_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(us(n))));
        let rx_freq = key("radio.rx.c0.frequency_hz");
        let tx_gain = key("radio.tx.c0.gain_db");
        // The receipt at 100 ms: 1 ms ahead is inside the 2 ms device lead, 40 ms is not.
        time.advance_to(core.at(us(100_000))).unwrap();
        control.book_timed(ActionId(1), rx_freq.clone(), Value::num(2.1e9).unwrap(), at(101_000));
        control.book_timed(ActionId(2), tx_gain.clone(), Value::num(3.0).unwrap(), at(140_000));
        time.advance_to(core.at(us(101_500))).unwrap();
        control.release();
        time.advance_to(core.at(us(138_000))).unwrap();
        control.release();
        // The late one is applied where LATE_COMMAND says, a device lead after its receipt.
        let late = events_of(&events, kinds::LATE_COMMAND);
        assert_eq!(late.len(), 1, "{late:?}");
        assert_eq!(late[0].payload["key"], "radio.rx.c0.frequency_hz");
        let applied = device.calls();
        let rx = applied.iter().find(|c| c.starts_with("apply rx 0") && c.contains("freq=2100000000") && !c.ends_with("at=-")).unwrap_or_else(|| panic!("{applied:?}"));
        let applied_at: i64 = rx.split("at=").nth(1).unwrap().split(' ').next().unwrap().parse().unwrap();
        assert_eq!(Some(applied_at), late[0].payload["applied"]["ticks"].as_i64(), "{rx}");
        assert_eq!(applied_at, us(102_000), "the late one's applied instant: {rx}");
        let tx = applied.iter().find(|c| c.starts_with("apply tx 0") && c.contains("gain=3")).unwrap_or_else(|| panic!("{applied:?}"));
        assert!(tx.contains(&format!("at={} ", us(140_000))), "{tx}");
        // 17 pending at +5 s: the 17th is refused (UR-24's depth 16).
        time.advance_to(core.at(us(141_000))).unwrap();
        for i in 0..17 {
            control.book_timed(ActionId(3), tx_gain.clone(), Value::num(4.0).unwrap(), at(5_000_000 + i));
        }
        let full = events_of(&events, kinds::COMMAND_QUEUE_FULL);
        assert_eq!(full.len(), 1, "{full:?}");
        assert_eq!(full[0].payload, json!({ "key": "radio.tx.c0.gain_db", "depth": 16 }));
        assert_eq!(control.held.len(), 16);
    }

    #[test]
    fn ur_24_a_far_future_update_does_not_delay_a_nearer_one() {
        // UR-24, UC-2 (design note 37): a far-future timed update does not delay a nearer one
        // on the device's in-order queue. On the controlled clock, the near instant cannot pass
        // while the test thread is descheduled.
        let (core, device, time, events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        device.rx_open(0, &[0]).unwrap();
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        let freq = key("radio.rx.c0.frequency_hz");
        let released = |device: &crate::device::FakeDevice| -> Vec<(i64, i64)> {
            device.calls().iter()
                .filter(|c| c.starts_with("apply rx 0 rate=- freq=") && !c.contains("at=-"))
                .map(|c| {
                    let at: i64 = c.split("at=").nth(1).unwrap().split(' ').next().unwrap().parse().unwrap();
                    let effective: i64 = c.split("effective=").nth(1).unwrap().parse().unwrap();
                    (at, effective)
                })
                .collect()
        };
        control.book_timed(ActionId(1), freq.clone(), Value::num(2.1e9).unwrap(), at(500));
        control.book_timed(ActionId(2), freq.clone(), Value::num(2.2e9).unwrap(), at(20));
        // The near one's release window: only it is handed over, at its instant.
        time.advance_to(core.at(ms(17))).unwrap();
        control.release();
        assert_eq!(released(&device), vec![(ms(20), ms(20))], "the near one alone, at its instant: {:?}", device.calls());
        assert_eq!(control.held.len(), 1, "the far one is still held");
        // Past the far one's window it is released too, at its own instant.
        time.advance_to(core.at(ms(497))).unwrap();
        control.release();
        assert_eq!(released(&device), vec![(ms(20), ms(20)), (ms(500), ms(500))], "{:?}", device.calls());
        assert!(control.held.is_empty());
        assert!(events_of(&events, kinds::LATE_COMMAND).is_empty());
    }

    #[test]
    fn ur_25_rx_channels_from_zero_starts_at_its_instant() {
        // RM-25 (design note 37): a stream enabled from 0 channels starts at or after its
        // effective instant. On the controlled clock, the device stands still at 0, so the
        // 300 ms start cannot have passed.
        let (core, device, time) = manual_rig();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        let ms = |n: i64| core.ticks(n * 1_000_000);
        channels_from_none(&mut control, &core, 2, 300, 1);
        // The timed start is issued by the owner once the enable is configured.
        let mut rx = owner_rx(&core);
        assert!(rx.begin(), "the owner begins the enable's segment");
        let calls = device.calls();
        let start = calls.iter().rev().find(|c| c.starts_with("rx_start ")).unwrap_or_else(|| panic!("{calls:?}"));
        let origin: i64 = start["rx_start 0 ".len()..].parse().unwrap_or_else(|_| panic!("{start}"));
        assert!(origin >= ms(300) && origin < ms(300) + 200, "origin {origin}, asked {}", ms(300));
        assert!(origin > device.time_now().unwrap(), "the start is ahead of the device's own clock: {start}");
        // The segment's clock is declared at the begin; its registration at the first block and
        // that block's origin are pinned by `ur_25_a_second_change_follows_the_first` in
        // `src/provider/rx.rs`.
        let declared: Vec<_> = core.clocks.declared_sample_clocks().into_iter().filter(|h| h.stream() == &ResourceId::parse("usrp/rx/s0").unwrap()).collect();
        assert_eq!(declared.len(), 1, "the segment's stream declares its clock: {declared:?}");
        assert_eq!(declared[0].root_ticks_per_tick(), Rational::new(200, 1).unwrap(), "at the segment's rate");
        assert!(lock(&core.rec).rejected.is_empty(), "{:?}", lock(&core.rec).rejected);
    }

    #[test]
    fn ur_25_a_timed_update_at_the_cold_change_s_instant_reaches_the_switch() {
        // UC-2, RM-25 (design note 37): a timed update at a `cold` change's own instant is in
        // the configuration its switch applies, whatever the arrival order, and is released at
        // its own instant; neither floors the other. On the controlled clock.
        for timed_first in [true, false] {
            let case = format!("timed first: {timed_first}");
            let (core, device, time, events) = manual_rig_events();
            let _frozen = hold(&core, &time, &device);
            device.rx_open(0, &[0]).unwrap();
            let (mut control, _tx) = control(&core, &[Dir::Rx]);
            let ms = |n: i64| core.ticks(n * 1_000_000);
            let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
            let rate = key("radio.rx.s0.sample_rate_hz");
            let freq = key("radio.rx.c0.frequency_hz");
            if timed_first {
                control.book_timed(ActionId(1), freq.clone(), Value::num(2.1e9).unwrap(), at(300));
            }
            control.book_cold(ActionId(2), rate.clone(), Value::num(2e6).unwrap(), at(300));
            if !timed_first {
                control.book_timed(ActionId(3), freq.clone(), Value::num(2.1e9).unwrap(), at(300));
            }
            let plan = plan_of(&core, Dir::Rx);
            assert_eq!(plan.len(), 2, "{case}: {:?}", plan.iter().map(|p| p.segment.origin).collect::<Vec<_>>());
            // UR-24 (R1): 300 ms is the old segment's cut, so no sample passes the DDC at it, and
            // the switch is not configured yet (its owner configures after the old segment ends):
            // the release at 298 ms calls the device for nothing.
            time.advance_to(core.at(ms(298))).unwrap();
            control.release();
            assert!(!device.calls().iter().any(|c| c.starts_with("apply rx 0")), "{case}: the release before the cut: {:?}", device.calls());
            assert!(events_of(&events, kinds::LATE_COMMAND).is_empty(), "{case}");
            // UR-24, design 38 D2 (3): the released row comes at the instant.
            time.advance_to(core.at(ms(300))).unwrap();
            control.release();
            // The switch's configuration is the projection at its origin, the timed update in it.
            plan[1].configure(&core, Dir::Rx).expect("the switch configures");
            let calls = device.calls();
            let switched = calls.iter().filter(|c| c.starts_with("apply rx 0") && c.contains("rate=2000000") && c.contains("freq=2100000000") && c.ends_with("at=-")).count();
            assert_eq!(switched, 1, "{case}: the switch's configuration: {calls:?}");
            assert!(calls.iter().all(|c| !c.starts_with("apply rx 0") || c.ends_with("at=-")), "{case}: no timed command: {calls:?}");
            assert_eq!(device.dsp_pending(Dir::Rx, 0), 0, "{case}");
            assert!(lock(&core.rec).rejected.is_empty(), "{case}");
            let rec = lock(&core.rec);
            let rows: Vec<_> = rec.applied.iter().filter(|r| r["key"] == "radio.rx.c0.frequency_hz" && r["issued"] == true && r["at"]["ticks"] == ms(300)).collect();
            assert_eq!(rows.len(), 1, "{case}: {:?}", rec.applied);
        }
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
        let gain = key("radio.rx.c0.gain_db");
        control.book_cold(ActionId(1), key("radio.rx.s0.sample_rate_hz"), Value::num(2e6).unwrap(), at(300_000));
        let refused = plan_of(&core, Dir::Rx)[1].segment.by.unwrap();
        control.book_timed(ActionId(1), gain.clone(), Value::num(1.0).unwrap(), at(300_000));
        time.advance_to(core.at(us(297_500))).unwrap();
        control.release();
        assert!(lock(&core.streams).refuse(&core, booked_stream(Dir::Rx), refused));
        control.book_timed(ActionId(1), gain.clone(), Value::num(2.0).unwrap(), at(299_600));
        control.release();
        // Its instant is ahead: it stays held, and is settled once it has passed with no channel
        // (UR-24).
        time.advance_to(core.at(us(300_100))).unwrap();
        control.release();
        let rows: Vec<_> = lock(&core.rec).applied.iter().filter(|r| r["key"] == "radio.rx.c0.gain_db")
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
        control.book_cold(ActionId(1), key("radio.rx.c0.stream"), Value::from(0), None);
        assert!(plan_of(&core, Dir::Rx).is_empty(), "an enable without a link planned a segment");
        assert_eq!(control.config_at(i64::MAX)[&key("radio.rx.c0.stream")], Value::from(0), "the value is not of record");
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
            let rate = key(&format!("radio.{}.s0.sample_rate_hz", refused.name()));
            let gain = |dir: Dir| key(&format!("radio.{}.c0.gain_db", dir.name()));
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
            // The halted direction's command is settled once its instant has passed (UR-24).
            time.advance_to(core.at(ms(200))).unwrap();
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
            let (rate, rx_gain, tx_gain) = (key("radio.rx.s0.sample_rate_hz"), key("radio.rx.c0.gain_db"), key("radio.tx.c0.gain_db"));
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
            assert_eq!(settings.settings_on(&core, &settings.updates.lock(), Dir::Rx, 0, planned[1].segment.origin, 0).gain, Some(3.0), "{case}");
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
        control.book_timed(ActionId(1), key("radio.tx.c0.gain_db"), Value::num(1.0).unwrap(), at(10_000));
        time.advance_to(core.at(us(7_500))).unwrap();
        control.release();
        control.book_timed(ActionId(2), key("radio.rx.c0.gain_db"), Value::num(2.0).unwrap(), at(9_700));
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
        control.book_cold(ActionId(1), key("radio.rx.c0.stream"), Value::from(0), None);
        control.book_cold(ActionId(1), key("radio.tx.c0.stream"), Value::from(0), None);
        control.book_timed(ActionId(1), key("radio.rx.c0.gain_db"), Value::num(3.0).unwrap(), Some(AbsoluteDeadline::new(core.at(core.ticks(200_000_000)))));
        time.advance_to(core.at(core.ticks(199_000_000))).unwrap();
        control.release();
        for name in ["rx_open", "tx_open", "apply"] {
            assert!(spy.seen(name) > 0, "the spy never saw {name}");
        }
        assert!(spy.seen("apply") >= 3, "the enable and the release did not apply: {}", spy.seen("apply"));
        // The release that gives a command to a later segment's channel passes the same way.
        let spy = Arc::new(crate::provider::test_support::LockSpy::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, ..crate::device::FakeConfig::default() }));
        let (core, time, _) = crate::provider::test_support::rig_on_inputs(spy.clone(), vec![Arc::new(NoLink)], 2_000, BTreeMap::new());
        spy.watch(&core);
        let (mut again, _tx) = self::control(&core, &[]);
        again.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        channels_from_none(&mut again, &core, 2, 100, 1);
        again.book_timed(ActionId(3), key("radio.rx.c0.gain_db"), Value::num(3.0).unwrap(), Some(AbsoluteDeadline::new(core.at(core.ticks(50_000_000)))));
        time.advance_to(core.at(core.ticks(47_000_000))).unwrap();
        again.release();
        assert_eq!((spy.seen("apply"), spy.log().iter().filter(|call| call.starts_with("apply rx 0") && !call.ends_with("at=-")).count()), (2, 1), "the enable's configure and the timed release did not apply: {:?}", spy.log());
    }

    #[test]
    fn ur_25_the_cold_booking_books_in_the_section_that_takes_its_instant() {
        // UR-25, #63 (O1): the instant, the arrival order and the booking are one critical
        // section of the instance's line lock, so that nothing pruned between them takes
        // effect after the change. The witness is the events the booking emits: a change the
        // floor makes late, and one that is refused, are both reported while the lock that
        // books them is held. The layout reports the late change before its refusal (VH-8).
        // A configuration built inside that section is new and unshared, so it takes no
        // second lock (UR-25's order).
        let selector = BTreeMap::from([
            (ezsdr_kernel::spec::Ident::parse("rx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:B:ubx:0")])),
            (ezsdr_kernel::spec::Ident::parse("tx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0")])),
        ]);
        let layout = ezsdr_radio::channels::from_selector(&selector).unwrap();
        let description = ezsdr_radio::channels::Description::new(&layout, 2_000, &crate::profile::terms());
        let (core, _time, witness) = crate::provider::test_support::rig_on_description_witness(
            Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig::default())),
            vec![Arc::new(NoLink)],
            BTreeMap::new(),
            description,
        );
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        control.book_cold(ActionId(1), key("radio.rx.s0.sample_rate_hz"), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(300)))));
        // Asked behind the first change, so the stream floor raises it: a LATE_COMMAND.
        control.book_cold(ActionId(1), key("radio.rx.s0.sample_rate_hz"), Value::num(1e6).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(100)))));
        control.book_cold(ActionId(1), key("radio.rx.c0.stream"), Value::from(1), Some(AbsoluteDeadline::new(core.at(ms(200)))));
        let seen = witness.events();
        for kind in [kinds::LATE_COMMAND, kinds::COMMAND_REJECTED] {
            let rows: Vec<bool> = seen.iter().filter(|(name, _)| name == kind).map(|(_, held)| *held).collect();
            assert!(!rows.is_empty(), "{kind} was never emitted: {seen:?}");
            assert!(rows.iter().all(|held| *held), "{kind} was emitted outside the section that books it: {seen:?}");
        }
    }

    #[test]
    fn ur_25_the_layout_cold_change_is_built_in_its_booking_section() {
        // UR-25, #63 (O1): an accepted layout change builds its configuration inside the
        // section that books it. An accepted change emits no event, so the witness cannot see
        // it: the spy records whether the line lock is held at each host clock read, and the
        // configuration reads the clock. `Control::new` reads it too, before the spy watches.
        let selector = BTreeMap::from([
            (ezsdr_kernel::spec::Ident::parse("rx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:B:ubx:0")])),
            (ezsdr_kernel::spec::Ident::parse("tx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0")])),
        ]);
        let layout = ezsdr_radio::channels::from_selector(&selector).unwrap();
        let description = ezsdr_radio::channels::Description::new(&layout, 2_000, &crate::profile::terms());
        let spy = Arc::new(crate::provider::test_support::LockSpy::new(crate::device::FakeConfig::default()));
        let (core, _time, _) = crate::provider::test_support::rig_on_description(spy.clone(), vec![Arc::new(NoLink)], BTreeMap::new(), description);
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
        // section of the line lock, so an accepted change takes the lock once. An enable from no stream
        // takes it twice: its device calls are made with the lock released between the two.
        let selector = BTreeMap::from([
            (ezsdr_kernel::spec::Ident::parse("rx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:B:ubx:0")])),
            (ezsdr_kernel::spec::Ident::parse("tx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0")])),
        ]);
        let layout = ezsdr_radio::channels::from_selector(&selector).unwrap();
        let description = ezsdr_radio::channels::Description::new(&layout, 2_000, &crate::profile::terms());
        let (core, _time, _) = crate::provider::test_support::rig_on_description(Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig::default())), vec![Arc::new(NoLink)], BTreeMap::new(), description);
        let (mut ctrl, _tx) = control(&core, &[Dir::Rx]);
        let streams = std::ptr::from_ref(&core.streams) as usize;
        let ms = |n: i64| core.ticks(n * 1_000_000);
        crate::provider::test_support::record_locks();
        ctrl.book_cold(ActionId(1), key("radio.rx.s0.sample_rate_hz"), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(300)))));
        let taken = crate::provider::test_support::taken_locks();
        assert_eq!(taken.iter().filter(|address| **address == streams).count(), 1, "an accepted change took the line lock {taken:?}");

        let (core, _time, _) = crate::provider::test_support::rig_on_inputs(Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig::default())), vec![Arc::new(NoLink)], 2_000, BTreeMap::new());
        let (mut ctrl, _tx) = control(&core, &[]);
        ctrl.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        let streams = std::ptr::from_ref(&core.streams) as usize;
        crate::provider::test_support::record_locks();
        ctrl.book_cold(ActionId(1), key("radio.rx.c0.stream"), Value::from(0), None);
        let taken = crate::provider::test_support::taken_locks();
        assert_eq!(taken.iter().filter(|address| **address == streams).count(), 2, "an enable from no stream took the line lock {taken:?}");
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
        let (core, _time, _) = crate::provider::test_support::rig_on_description(Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig::default())), vec![Arc::new(NoLink)], BTreeMap::new(), description);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let streams = std::ptr::from_ref(&core.streams) as usize;
        let instant = core.ticks(300_000_000);
        let at = Some(AbsoluteDeadline::new(core.at(instant)));
        let node = core.stream_node(Dir::Rx, 0);
        crate::provider::test_support::record_locks();
        control.book(Dispatched::new(ActionId(1), Action::Command {
            target: super::super::test_support::target_of(&node),
            verb: ezsdr_kernel::spec::Ident::parse(ezsdr_radio::START_RX).unwrap(),
            params: BTreeMap::new(),
            at,
        }, Some(node)));
        let taken = crate::provider::test_support::taken_locks();
        assert_eq!(taken.iter().filter(|address| **address == streams).count(), 1, "an accepted start_rx took the line lock {taken:?}");
        let start = lock(&core.streams).line(booked_stream(Dir::Rx)).unwrap().items.iter().find(|item| matches!(item.kind, Kind::Start)).map(|item| item.e);
        assert_eq!(start, Some(instant), "start_rx ignored its requested instant");
    }

    #[test]
    fn ur_17_a_stream_s_prune_leaves_the_other_stream_alone() {
        // UR-17, RM-25, RM-34 (design 33 D15): pruning one stream's ended segment folds that
        // stream's items and configurations and nothing of the instance's other stream: the
        // transmit stream keeps its item and its configuration, and its command can still be
        // refused.
        let (core, _, _) = rig();
        let (mut control, _tx) = control(&core, &[Dir::Rx, Dir::Tx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        control.book_cold(ActionId(1), key("radio.rx.s0.sample_rate_hz"), Value::num(2e6).unwrap(), at(100));
        control.book_cold(ActionId(1), key("radio.rx.s0.sample_rate_hz"), Value::num(1e6).unwrap(), at(200));
        control.book_cold(ActionId(1), key("radio.tx.s0.sample_rate_hz"), Value::num(2e6).unwrap(), at(100));
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
        let spy = Arc::new(crate::provider::test_support::LockSpy::new(crate::device::FakeConfig { clock: ClockMode::Manual, ..crate::device::FakeConfig::default() }));
        let links: Vec<Arc<dyn DataLink>> = if links { vec![Arc::new(NoLink)] } else { Vec::new() };
        let (core, time, _) = crate::provider::test_support::rig_on(spy.clone(), links, 2_000);
        (core, spy, time)
    }

    /// Books an enable of the receive stream's `channels` at `ms` from no stream, its owner
    /// idle, through the Action path (uhi-control's own booking).
    fn rx_enable_from_none(control: &mut Control, core: &Core, id: u64, ms: i64, channels: i64) {
        lock(&core.streams).search(booked_stream(Dir::Rx), false);
        control.book(Dispatched::new(ActionId(id), Action::UpdateParameter {
            target: super::super::test_support::target_of(&core.id),
            key: key("radio.rx.c0.stream"),
            value: Value::from(if channels > 0 { 0 } else { -1 }),
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
    fn rx_record(core: &Core) -> (bool, bool, super::super::core::DeviceSet, Option<u64>) {
        let streams = lock(&core.streams);
        let k = booked_stream(Dir::Rx);
        let state = streams.stream(k);
        (streams.holder(k) != Holder::Free, streams.reading(k), state.set, state.configured)
    }

    #[test]
    fn ur_25_an_owner_waiting_behind_a_from_none_open_starts_nothing() {
        // UR-25 (S9, receive): while uhd-control's enable from no stream owns the streamer,
        // uhd-rx's turn makes no device call and moves neither its idle mark nor its cursor.
        // The first turn after the booking opens B's set again: the record holds C's.
        let (core, spy, _time) = spy_rig(true);
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.c0.stream"), Value::from(0));
        control.config.insert(key("radio.rx.c1.stream"), Value::from(0));
        let at = |ms: i64| Some(AbsoluteDeadline::new(core.at(core.ticks(ms * 1_000_000))));
        control.book_cold(ActionId(2), key("radio.rx.s0.sample_rate_hz"), Value::num(1e6).unwrap(), at(100));
        control.book_cold(ActionId(3), key("radio.rx.c1.stream"), Value::from(-1), at(200));
        control.book_cold(ActionId(4), key("radio.rx.c0.stream"), Value::from(-1), at(250));
        let gate = spy.stop_before("rx_open");
        let host = core.device.host();
        let core_ = core.clone();
        let (done, finished) = channel();
        let control_thread = host.spawn("control", move || {
            let mut control = control;
            rx_enable_from_none(&mut control, &core_, 5, 300, 1);
            let _ = done.send(());
        }).expect("a participant thread");
        gate.wait();
        let mut rx = owner_rx(&core);
        let cursor = rx.cursor();
        let mark = spy.log().len();
        assert!(!rx.begin(), "the owner began a segment while uhd-control owns the streamer");
        assert_eq!(rx.cursor(), cursor, "the owner moved its cursor");
        assert!(lock(&core.streams).is_idle(booked_stream(Dir::Rx)), "the owner moved its phase");
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
        control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
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
                    seen.lock().unwrap().push(lock(&core.streams).holder(booked_stream(Dir::Rx)) == Holder::FromNone);
                }
            });
        }
        {
            let seen = seen.clone();
            crate::provider::test_support::probe(&core, "cold_booking", move |streams| {
                seen.lock().unwrap().push(streams.expect("the booking's line").holder(booked_stream(Dir::Rx)) == Holder::FromNone);
            });
        }
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        channels_from_none(&mut control, &core, 2, 100, 1);
        let observed = seen.lock().unwrap().clone();
        assert_eq!(observed, [true, true, true, true], "an enable's section was not the streamer's: {observed:?}");
        let (busy, reading, _set, configured) = rx_record(&core);
        assert!(!busy && !reading, "the stream stayed uhd-control's");
        let planned = plan_of(&core, Dir::Rx);
        assert_eq!(configured, planned[0].segment.by, "the configuration's own mark is not recorded");
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
        assert!(!lock(&core.streams).is_idle(booked_stream(Dir::Rx)), "the owner has not begun");
        let mark = spy.log().len();
        let planned = plan_of(&core, Dir::Rx).len();
        control.book_cold(ActionId(1), key("radio.rx.s0.sample_rate_hz"), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(core.at(core.ticks(100_000_000)))));
        assert_eq!(spy.log().len(), mark, "uhd-control called the device while the owner owns the streamer: {:?}", spy.log());
        let after_book = lock(&core.streams).is_idle(booked_stream(Dir::Rx));
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
        let rate_checks = Arc::new(std::sync::Mutex::new(0usize));
        {
            let rate_checks = rate_checks.clone();
            crate::provider::test_support::probe(&core, "rates", move |_| *rate_checks.lock().unwrap() += 1);
        }
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        let (returned, resume) = pause_core_at(&core, "open_returned");
        let host = core.device.host();
        let core_ = core.clone();
        let (done, finished) = channel();
        let control_thread = host.spawn("control", move || {
            let mut control = control;
            rx_enable_from_none(&mut control, &core_, 2, 100, 1);
            let _ = done.send(());
        }).expect("a participant thread");
        assert!(returned.recv_timeout(Wall::from_secs(10)).is_ok(), "the open returned to Core");
        core.device_lost("test");
        let _ = resume.send(());
        assert!(finished.recv_timeout(Wall::from_secs(10)).is_ok(), "the enable did not end");
        control_thread.join().expect("the control thread");
        assert_eq!(*rate_checks.lock().unwrap(), 0, "a lost enable did not enter a rate check");
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
            control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
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
        // UR-29, UR-25 (S27 e): the device is lost while a two-channel stream configuration applies
        // its first channel. The second is not applied, the enable books nothing, and the
        // ownership goes back.
        let (core, spy, _time) = spy_rig(true);
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.c0.stream"), Value::from(0));
        control.config.insert(key("radio.rx.c1.stream"), Value::from(0));
        let gate = spy.stop_after("apply");
        let host = core.device.host();
        let core_ = core.clone();
        let (done, finished) = channel();
        let control_thread = host.spawn("control", move || {
            let mut control = control;
            control.book_cold(ActionId(2), key("radio.rx.s0.sample_rate_hz"), Value::num(1e6).unwrap(), Some(AbsoluteDeadline::new(core_.at(core_.ticks(100_000_000)))));
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
        control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
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
        control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        channels_from_none(&mut control, &core, 2, 100, 1);
        assert_eq!(spy.log().len(), before, "a stopped Provider's enable called the device: {:?}", spy.log());
        assert_eq!(rx_colds(&core), 0, "a stopped Provider's enable booked a change");
        let (busy, reading, _set, _configured) = rx_record(&core);
        assert!(!busy && !reading, "the stream stayed uhd-control's");
    }

    #[test]
    fn ur_29_a_lost_stream_between_the_rate_check_and_the_configure_applies_nothing() {
        // UR-29, UR-25: the device is lost after the rate comparison returns but before the
        // enable decides whether to configure; it books nothing and gives the streamer back.
        let (core, spy, _time) = spy_rig(true);
        let applies = Arc::new(std::sync::Mutex::new(0usize));
        {
            let applies = applies.clone();
            crate::provider::test_support::probe(&core, "apply", move |_| *applies.lock().unwrap() += 1);
        }
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        let (returned, resume) = pause_core_at(&core, "rates_returned");
        let host = core.device.host();
        let core_ = core.clone();
        let (done, finished) = channel();
        let control_thread = host.spawn("control", move || {
            let mut control = control;
            rx_enable_from_none(&mut control, &core_, 2, 100, 1);
            let _ = done.send(());
        }).expect("a participant thread");
        assert!(returned.recv_timeout(Wall::from_secs(10)).is_ok(), "the rate comparison returned to Core");
        core.device_lost("test");
        let _ = resume.send(());
        assert!(finished.recv_timeout(Wall::from_secs(10)).is_ok(), "the enable did not end");
        control_thread.join().expect("the control thread");
        assert_eq!(*applies.lock().unwrap(), 0, "a lost enable entered a configuration: {:?}", spy.log());
        assert_eq!(rx_colds(&core), 0, "a lost enable booked a change");
        let (busy, _reading, set, configured) = rx_record(&core);
        assert!(!busy && set == super::super::core::DeviceSet::Open(Channels::first(1)) && configured.is_none(), "the record is not what the open made known");
    }

    /// `manual_rig` with the events the booking emits.
    fn manual_rig_events() -> (Arc<Core>, Arc<crate::device::FakeDevice>, Arc<ezsdr_kernel::time::ManualTimeAuthority>, Arc<ezsdr_kernel::event::EventCollector>) {
        let config = crate::device::FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..crate::device::FakeConfig::default() };
        crate::provider::test_support::rig_with(config, vec![Arc::new(NoLink)])
    }

    /// The events of `kind` since the last drain.
    fn events_of(events: &Arc<ezsdr_kernel::event::EventCollector>, kind: &str) -> Vec<ezsdr_kernel::event::Event> {
        events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == kind).collect()
    }

    /// The released rows of `key`, as their instant, whether `issued`, and whether `no channel`;
    /// a configure's rows are not the release's (theirs read the device back).
    fn rows_of(core: &Core, key: &Key) -> Vec<(i64, bool, bool)> {
        lock(&core.rec).applied.iter().filter(|row| row["key"] == key.as_str() && row["read_back"].is_null())
            .map(|row| (row["at"]["ticks"].as_i64().expect("an instant"), row["issued"] == true, row["note"] == "no channel")).collect()
    }

    #[test]
    fn ur_24_a_release_without_a_live_channel_precedes_the_owner_s_configure() {
        // UR-24 (S2): with a segment planned but not yet configured, the release gives the gain
        // to the device as its own timed command at its instant; the owner's configure applies
        // the same value a second time. What the record holds decides no release.
        let (core, device, time) = manual_rig();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        let ms = |n: i64| core.ticks(n * 1_000_000);
        lock(&core.streams).search(booked_stream(Dir::Rx), true);
        control.book_cold(ActionId(2), key("radio.rx.c0.stream"), Value::from(0), Some(AbsoluteDeadline::new(core.at(ms(100)))));
        let origin = plan_of(&core, Dir::Rx)[0].segment.origin;
        let gain = key("radio.rx.c0.gain_db");
        control.book_timed(ActionId(3), gain.clone(), Value::num(3.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(50)))));
        time.advance_to(core.at(ms(47))).unwrap();
        control.release();
        let timed: Vec<String> = device.calls().iter().filter(|call| call.starts_with("apply rx 0") && call.contains("gain=3") && !call.ends_with("at=-")).cloned().collect();
        assert_eq!(lock(&core.streams).stream(booked_stream(Dir::Rx)).configured, None, "the owner's enable is in the record");
        let mark = device.calls().len();
        assert!(owner_rx(&core).begin(), "the owner begins the planned segment");
        let owner = device.calls()[mark..].to_vec();
        assert_eq!(timed.len(), 1, "{:?}", device.calls());
        assert_eq!(owner.iter().filter(|call| call.starts_with("apply rx 0") && call.ends_with("at=-") && call.contains("gain=3")).count(), 1, "the owner's configure applies the value again: {owner:?}");
        device_at(&core, &time, &device, origin + core.ticks(1_000_000));
        assert_eq!(device.read_back(Dir::Rx, 0).gain, Some(3.0));
    }

    #[test]
    fn ur_24_a_release_during_a_configure_s_return_reaches_the_device() {
        // UR-24 (S3): a gain booked while the owner's configure is returning, its record not yet
        // published, still reaches the device as its timed command; the owner's record then
        // holds the segment's command.
        let (core, device, time) = manual_rig();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        let ms = |n: i64| core.ticks(n * 1_000_000);
        lock(&core.streams).search(booked_stream(Dir::Rx), true);
        control.book_cold(ActionId(2), key("radio.rx.c0.stream"), Value::from(0), Some(AbsoluteDeadline::new(core.at(ms(100)))));
        let planned = plan_of(&core, Dir::Rx);
        let (by, origin) = (planned[0].segment.by.expect("the enable"), planned[0].segment.origin);
        let (entered, entered_rx) = channel();
        let (go, leave) = channel();
        let leave = std::sync::Mutex::new(leave);
        crate::provider::test_support::probe(&core, "configured", move |_| {
            let _ = entered.send(());
            let _ = leave.lock().unwrap().recv();
        });
        let host = core.device.host();
        let core_ = core.clone();
        let owner = host.spawn("rx", move || {
            let (to_rx, cmds) = channel();
            std::mem::forget(to_rx);
            let mut rx = super::super::rx::Rx::new(core_.clone(), 0, cmds, None, None);
            rx.begin();
        }).expect("a participant thread");
        entered_rx.recv_timeout(Wall::from_secs(10)).expect("the owner reached its configure's return");
        assert_eq!(lock(&core.streams).stream(booked_stream(Dir::Rx)).configured, None, "the record is published before the window");
        let gain = key("radio.rx.c0.gain_db");
        let mark = device.calls().len();
        control.book_timed(ActionId(3), gain.clone(), Value::num(3.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(50)))));
        time.advance_to(core.at(ms(47))).unwrap();
        control.release();
        let calls = device.calls();
        let timed: Vec<usize> = (mark..calls.len()).filter(|index| calls[*index].starts_with("apply rx 0") && !calls[*index].ends_with("at=-")).collect();
        let configured = calls[..mark].iter().rposition(|call| call.starts_with("apply rx 0") && call.ends_with("at=-")).expect("the owner configured the segment");
        assert_eq!(timed.len(), 1, "{calls:?}");
        assert!(calls[configured].contains("gain=0 ") && calls[timed[0]].contains(&format!("at={} effective={}", ms(50), ms(50))), "{calls:?}");
        go.send(()).unwrap();
        owner.join().expect("the owner thread");
        assert_eq!(lock(&core.streams).stream(booked_stream(Dir::Rx)).configured, Some(by));
        assert!(device.calls().iter().any(|call| call == &format!("rx_start 0 {origin}")), "the owner did not start the segment at its origin");
    }

    #[test]
    fn ur_24_a_release_without_a_live_channel_takes_the_command_queue_slots() {
        // UR-24 (S4): a command no channel is in force for still takes one device queue slot,
        // counted at its booking; the seventeenth exceeds the profile's depth and is refused
        // there, and the sixteen released reach the device in instant order.
        let (core, device, time, events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let us = |n: i64| core.ticks(n * 1_000);
        channels_from_none(&mut control, &core, 2, 100, 1);
        let gain = key("radio.rx.c0.gain_db");
        for i in 0..8 {
            control.book_timed(ActionId(1), gain.clone(), Value::num(1.0 + i as f64).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(2) + us(50) * i))));
        }
        for i in 0..8 {
            control.book_timed(ActionId(1), gain.clone(), Value::num(9.0 + i as f64).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(10) + us(50) * i))));
        }
        control.book_timed(ActionId(1), gain.clone(), Value::num(17.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(10) + us(500)))));
        let full = events_of(&events, kinds::COMMAND_QUEUE_FULL);
        assert_eq!((full.len(), full[0].payload["depth"].as_i64(), lock(&core.rec).rejected.len(), control.held.len()), (1, Some(16), 1, 16), "{:?}", lock(&core.rec).rejected);
        control.release();
        assert_eq!(control.released.len(), 8);
        time.advance_to(core.at(ms(7) + us(500))).unwrap();
        control.release();
        let applies: Vec<String> = device.calls().iter().filter(|call| call.starts_with("apply rx 0") && call.contains("gain=") && !call.ends_with("at=-")).cloned().collect();
        let gains: Vec<f64> = applies.iter().map(|call| call.split("gain=").nth(1).unwrap().split(' ').next().unwrap().parse().unwrap()).collect();
        assert_eq!((control.released.len(), gains), (8, (1..=16).map(f64::from).collect::<Vec<_>>()), "{applies:?}");
        assert!(applies[0].contains(&format!("at={} ", ms(2))) && applies[15].contains(&format!("at={} ", ms(10) + us(350))), "{applies:?}");
        device_at(&core, &time, &device, ms(11));
        assert_eq!(device.read_back(Dir::Rx, 0).gain, Some(16.0));
    }

    #[test]
    fn ur_24_a_release_after_its_instant_applies_on_arrival() {
        // UR-24 (S5): a release late by its own instant is the live branch's, with no new
        // `LATE_COMMAND`; the device applies the command when it arrives.
        let (core, device, time, events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let gain = key("radio.rx.c0.gain_db");
        control.book_timed(ActionId(1), gain.clone(), Value::num(3.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(50)))));
        time.advance_to(core.at(ms(51))).unwrap();
        device_at(&core, &time, &device, ms(51));
        control.release();
        let timed: Vec<String> = device.calls().iter().filter(|call| call.starts_with("apply rx 0") && !call.ends_with("at=-")).cloned().collect();
        assert!(events_of(&events, kinds::LATE_COMMAND).is_empty(), "a late release reported a new late command");
        assert_eq!(timed.len(), 1, "{:?}", device.calls());
        assert!(timed[0].contains(&format!("at={} effective={}", ms(50), ms(51))), "{timed:?}");
        assert_eq!((rows_of(&core, &gain), device.read_back(Dir::Rx, 0).gain), (vec![(ms(50), true, false)], Some(3.0)));
    }

    #[test]
    fn ur_24_a_release_without_a_live_channel_or_a_later_segment_applies_nothing() {
        // UR-24 (S6): with no channel in force and no segment from its instant on, the update is
        // recorded at its own instant and applies nothing.
        let (core, device, time) = manual_rig();
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let gain = key("radio.rx.c0.gain_db");
        control.book_timed(ActionId(1), gain.clone(), Value::num(3.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(50)))));
        time.advance_to(core.at(ms(47))).unwrap();
        control.release();
        assert!(device.calls().is_empty(), "{:?}", device.calls());
        // The instant passes with no channel to give it on: the row is written then (UR-24).
        time.advance_to(core.at(ms(51))).unwrap();
        control.release();
        assert_eq!((rows_of(&core, &gain), control.released.len()), (vec![(ms(50), false, true)], 0));
        assert_eq!(control.config_at(i64::MAX)[&gain], Value::num(3.0).unwrap(), "the recorded value is not of record");
    }

    #[test]
    fn ur_24_a_released_command_and_one_without_a_channel_settle_in_instant_order() {
        // UR-24 (S19): a command released at an earlier instant and one whose instant passed
        // with no channel settle into the configuration of record in instant order, not in the
        // order the pass met them: the later instant's value is the one a segment booked
        // afterwards begins from.
        let (core, device, time) = manual_rig();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let us = |n: i64| core.ticks(n * 1_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(us(n))));
        let gain = key("radio.rx.c0.gain_db");
        // The channel leaves at 101 ms, and no later enable brings it back.
        control.book_cold(ActionId(2), key("radio.rx.c0.stream"), Value::from(-1), at(101_000));
        // A, at 100 ms, is released at 97.5 ms and stays in `released_at` until its instant.
        control.book_timed(ActionId(3), gain.clone(), Value::num(1.0).unwrap(), at(100_000));
        time.advance_to(core.at(us(97_500))).unwrap();
        control.release();
        assert!(control.held.is_empty(), "A did not leave the held list: {:?}", device.calls());
        // B, at 102 ms, is booked behind it.
        control.book_timed(ActionId(4), gain.clone(), Value::num(2.0).unwrap(), at(102_000));
        // The next pass is at 102.5 ms: B's instant passed with no channel, and the fold must
        // not put A's earlier value over it.
        time.advance_to(core.at(us(102_500))).unwrap();
        control.release();
        assert_eq!(rows_of(&core, &gain), [(us(100_000), true, false), (us(102_000), false, true)], "{:?}", device.calls());
        assert_eq!(control.config_at(i64::MAX)[&gain], Value::num(2.0).unwrap(), "the earlier released command was folded last");
        // A segment booked afterwards begins from the later instant's value.
        control.book_cold(ActionId(5), key("radio.rx.c0.stream"), Value::from(0), at(200_000));
        let later = plan_of(&core, Dir::Rx).pop().expect("the later enable");
        later.configure(&core, Dir::Rx).expect("the later enable configures");
        assert_eq!(device.read_back(Dir::Rx, 0).gain, Some(2.0), "the later enable configured another value");
    }

    #[test]
    fn ur_24_a_layout_release_reaches_the_later_segment_s_channel() {
        // UR-24 (S7): under the layout a command acts on its own channel alone, on the later
        // segment that runs it — not on the first segment planned, and not on the direction's
        // other channel.
        let (core, device, time) = layout_rig();
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        lock(&core.streams).search(booked_stream(Dir::Rx), true);
        let stream_of_c1 = key("radio.rx.c1.stream");
        control.book_cold(ActionId(1), stream_of_c1.clone(), Value::from(0), at(100));
        control.book_cold(ActionId(1), stream_of_c1.clone(), Value::from(-1), at(200));
        control.book_cold(ActionId(1), stream_of_c1.clone(), Value::from(0), at(300));
        let gain = key("radio.rx.c1.gain_db");
        control.book_timed(ActionId(2), gain.clone(), Value::num(3.0).unwrap(), at(240));
        time.advance_to(core.at(ms(237) + core.ticks(500_000))).unwrap();
        control.release();
        let one: Vec<String> = device.calls().into_iter().filter(|call| call.starts_with("apply rx 1") && call.contains("gain=3") && call.contains(&format!("at={} ", ms(240)))).collect();
        assert_eq!(one.len(), 1, "{:?}", plan_of(&core, Dir::Rx).iter().map(|planned| planned.segment.config.channels.count()).collect::<Vec<_>>());
        assert!(!device.calls().iter().any(|call| call.starts_with("apply rx 0") && call.contains("gain=3")), "the command acted on the direction's other channel: {:?}", device.calls());
    }

    #[test]
    fn ur_24_a_release_for_a_refused_transmit_segment_applies_nothing() {
        // UR-24 (S8, transmit): the only segment from the gain's instant on is a refused one, so
        // no channel is in force and none is planned that would hold the gain: it is recorded
        // and applies nothing.
        let (core, device, time) = manual_rig();
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.tx.c0.stream"), Value::from(-1));
        let ms = |n: i64| core.ticks(n * 1_000_000);
        lock(&core.streams).search(booked_stream(Dir::Tx), true);
        control.book_cold(ActionId(2), key("radio.tx.c0.stream"), Value::from(0), Some(AbsoluteDeadline::new(core.at(ms(100)))));
        let by = plan_of(&core, Dir::Tx)[0].segment.by.expect("the enable");
        assert!(lock(&core.streams).refuse(&core, booked_stream(Dir::Tx), by));
        let gain = key("radio.tx.c0.gain_db");
        control.book_timed(ActionId(3), gain.clone(), Value::num(3.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(50)))));
        time.advance_to(core.at(ms(47))).unwrap();
        control.release();
        // Its instant passes with no channel to give it on: the row is written then (UR-24).
        time.advance_to(core.at(ms(51))).unwrap();
        control.release();
        assert!(!device.calls().iter().any(|call| call.starts_with("apply tx")), "{:?}", device.calls());
        assert_eq!(rows_of(&core, &gain), [(ms(50), false, true)]);
        assert!(plan_of(&core, Dir::Tx).iter().any(|planned| planned.refused), "the refused segment left the plan");
    }

    #[test]
    fn ur_24_a_release_for_a_refused_receive_change_has_no_candidate() {
        // UR-24 (S8, receive): a refused receive change is left out of the stream's plan, so a
        // gain due before the enable it would have begun has no candidate and applies nothing.
        let (core, device, time) = manual_rig();
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        let ms = |n: i64| core.ticks(n * 1_000_000);
        lock(&core.streams).search(booked_stream(Dir::Rx), true);
        control.book_cold(ActionId(2), key("radio.rx.c0.stream"), Value::from(0), Some(AbsoluteDeadline::new(core.at(ms(100)))));
        let by = plan_of(&core, Dir::Rx)[0].segment.by.expect("the enable");
        assert!(lock(&core.streams).refuse(&core, booked_stream(Dir::Rx), by));
        let gain = key("radio.rx.c0.gain_db");
        control.book_timed(ActionId(3), gain.clone(), Value::num(3.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(50)))));
        time.advance_to(core.at(ms(47))).unwrap();
        control.release();
        // Its instant passes with no channel to give it on: the row is written then (UR-24).
        time.advance_to(core.at(ms(51))).unwrap();
        control.release();
        assert!(device.calls().is_empty(), "{:?}", device.calls());
        assert_eq!(rows_of(&core, &gain), [(ms(50), false, true)]);
        assert!(plan_of(&core, Dir::Rx).is_empty(), "the refused receive segment stayed in the plan");
    }

    #[test]
    fn ur_24_a_release_stays_when_its_segment_is_refused_after_it() {
        // UR-24 (S8b): a refusal booked after the release takes nothing back: the device applied
        // the gain at its instant, and the stream's end retires the command from the queue.
        let (core, device, time, events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        let ms = |n: i64| core.ticks(n * 1_000_000);
        lock(&core.streams).search(booked_stream(Dir::Rx), true);
        control.book_cold(ActionId(2), key("radio.rx.c0.stream"), Value::from(0), Some(AbsoluteDeadline::new(core.at(ms(100)))));
        let by = plan_of(&core, Dir::Rx)[0].segment.by.expect("the enable");
        let gain = key("radio.rx.c0.gain_db");
        control.book_timed(ActionId(3), gain.clone(), Value::num(3.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(50)))));
        time.advance_to(core.at(ms(47))).unwrap();
        control.release();
        let applies = device.calls().iter().filter(|call| call.starts_with("apply rx 0") && !call.ends_with("at=-")).count();
        assert!(lock(&core.streams).refuse(&core, booked_stream(Dir::Rx), by));
        device_at(&core, &time, &device, ms(51));
        time.advance_to(core.at(ms(51))).unwrap();
        control.release();
        assert_eq!((applies, control.released.len(), events_of(&events, kinds::COMMAND_REJECTED).len()), (1, 0, 0), "{:?}", device.calls());
        assert_eq!(device.read_back(Dir::Rx, 0).gain, Some(3.0));
        assert_eq!(rows_of(&core, &gain), [(ms(50), true, true)]);
    }

    #[test]
    fn ur_29_a_lost_device_applies_no_released_update() {
        // UR-29 (S9 a): a release on a lost device makes no device call and records no row.
        let (core, spy, _time) = spy_rig(true);
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        let ms = |n: i64| core.ticks(n * 1_000_000);
        lock(&core.streams).search(booked_stream(Dir::Rx), true);
        control.book_cold(ActionId(2), key("radio.rx.c0.stream"), Value::from(0), Some(AbsoluteDeadline::new(core.at(ms(100)))));
        let gain = key("radio.rx.c0.gain_db");
        control.book_timed(ActionId(3), gain.clone(), Value::num(3.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(50)))));
        core.device_lost("the test's loss");
        control.release();
        assert_eq!(spy.seen("apply"), 0, "a lost device was called: {:?}", spy.log());
        assert!(core.is_lost() && rows_of(&core, &gain).is_empty(), "{:?}", lock(&core.rec).applied);
    }

    #[test]
    fn ur_29_a_loss_published_before_a_release_stops_the_call() {
        // UR-29: the loss can be published after targets are selected but before the one apply.
        let (core, device, time, events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let gain = key("radio.rx.c0.gain_db");
        control.book_timed(ActionId(1), gain.clone(), Value::num(3.0).unwrap(), None);
        let core_ = core.clone();
        crate::provider::test_support::probe(&core, "before_timed_apply", move |_| core_.device_lost("test loss"));
        control.release();
        assert!(core.is_lost());
        assert!(!device.calls().iter().any(|call| call.starts_with("apply")), "the lost device was called: {:?}", device.calls());
        assert!(rows_of(&core, &gain).is_empty(), "a row was written after the loss: {:?}", lock(&core.rec).applied);
        assert_eq!(events_of(&events, ezsdr_kernel::event::EventKind::DEVICE_LOST).len(), 1);
    }

    #[test]
    fn ur_29_a_lost_timed_release_publishes_device_loss() {
        // UR-29: an apply that itself reports a lost device publishes the loss and writes no row.
        let inner = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..crate::device::FakeConfig::default() }));
        let fails = Arc::new(crate::provider::test_support::ApplyFails::timed_lost(inner.clone(), Dir::Rx, Some(0)));
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(fails.clone(), vec![Arc::new(NoLink)], 2_000, BTreeMap::new());
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        let ms = |n: i64| core.ticks(n * 1_000_000);
        lock(&core.streams).search(booked_stream(Dir::Rx), true);
        control.book_cold(ActionId(2), key("radio.rx.c0.stream"), Value::from(0), Some(AbsoluteDeadline::new(core.at(ms(100)))));
        let gain = key("radio.rx.c0.gain_db");
        control.book_timed(ActionId(3), gain.clone(), Value::num(3.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(50)))));
        time.advance_to(core.at(ms(47))).unwrap();
        control.release();
        assert!(core.is_lost());
        assert_eq!(fails.attempts(), [(Dir::Rx, 0, Some(ms(50)))]);
        assert!(rows_of(&core, &gain).is_empty());
        assert_eq!(events_of(&events, ezsdr_kernel::event::EventKind::DEVICE_LOST).len(), 1);
        // The command was never handed over: the end settles it cancelled at the loss's instant.
        control.finish();
        assert_eq!((rows_of(&core, &gain), cancelled_at(&core)), (vec![(ms(50), false, false)], vec![ms(47)]), "{:?}", lock(&core.rec).applied);
    }

    #[test]
    fn ur_29_a_failed_released_update_is_a_device_failure() {
        // UR-29 (S9 b): a timed apply the device refuses, other than by a lost device, is
        // reported and records no row; its entry stays in the projection, so a later configure
        // of the segment applies the value the device never took.
        let inner = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..crate::device::FakeConfig::default() }));
        let fails = Arc::new(crate::provider::test_support::ApplyFails::timed(inner.clone(), Dir::Rx, None));
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(fails.clone(), vec![Arc::new(NoLink)], 2_000, BTreeMap::new());
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        let ms = |n: i64| core.ticks(n * 1_000_000);
        lock(&core.streams).search(booked_stream(Dir::Rx), true);
        control.book_cold(ActionId(2), key("radio.rx.c0.stream"), Value::from(0), Some(AbsoluteDeadline::new(core.at(ms(100)))));
        let gain = key("radio.rx.c0.gain_db");
        control.book_timed(ActionId(3), gain.clone(), Value::num(3.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(50)))));
        time.advance_to(core.at(ms(47))).unwrap();
        control.release();
        let projection = plan_of(&core, Dir::Rx)[0].settings.clone().expect("the settings");
        assert_eq!((fails.attempts(), events_of(&events, kinds::COMMAND_REJECTED).len()), (vec![(Dir::Rx, 0, Some(ms(50)))], 1));
        assert!(lock(&core.rec).rejected.iter().any(|row| row["kind"] == "update_parameter" && row["reason"].as_str().unwrap_or("").contains("refusal")), "{:?}", lock(&core.rec).rejected);
        assert!(rows_of(&core, &gain).is_empty(), "a refused release left a row");
        assert_eq!(projection.settings_on(&core, &projection.updates.lock(), Dir::Rx, 0, ms(200), 0).gain, Some(3.0), "the projection lost the refused release");
        // Its instant passes: a refused release is not the configuration of record (UR-24).
        time.advance_to(core.at(ms(52))).unwrap();
        control.release();
        assert_eq!((control.config_at(i64::MAX)[&gain].clone(), rows_of(&core, &gain)), (Value::num(0.0).unwrap(), vec![]), "a refused release became the configuration of record");
    }

    #[test]
    fn ur_29_a_loss_after_a_release_s_command_stops_the_one_behind_it() {
        // UR-29 (S9 g): a loss another thread publishes after a released command's apply
        // returned stops the release before the next held command: one whose instant has passed
        // with no channel writes no row, and the device is not called again.
        let spy = Arc::new(crate::provider::test_support::LockSpy::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, ..crate::device::FakeConfig::default() }));
        let (core, time, events) = crate::provider::test_support::rig_on(spy.clone(), vec![Arc::new(NoLink)], 2_000);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        let gain = key("radio.rx.c0.gain_db");
        // The channel leaves between A's instant and B's, so B is due with no channel then.
        control.book_cold(ActionId(2), key("radio.rx.c0.stream"), Value::from(-1), at(50));
        control.book_timed(ActionId(3), gain.clone(), Value::num(1.0).unwrap(), at(49));
        control.book_timed(ActionId(4), gain.clone(), Value::num(2.0).unwrap(), at(51));
        time.advance_to(core.at(ms(45))).unwrap();
        control.release();
        assert_eq!(control.held.len(), 2, "a command was released beyond the window: {:?}", spy.log());
        time.advance_to(core.at(ms(52))).unwrap();
        let gate = spy.stop_after("apply");
        let core_ = core.clone();
        let publisher = core.device.host().spawn("loss", move || {
            gate.wait();
            core_.device_lost("the test's loss");
            gate.release();
        }).expect("a participant thread");
        control.release();
        publisher.join().expect("the publisher thread");
        let applies: Vec<String> = spy.log().into_iter().filter(|call| call.starts_with("apply")).collect();
        assert_eq!((spy.seen("apply"), applies.len()), (1, 1), "the command behind the stopped release was given: {applies:?}");
        assert!(applies[0].starts_with("apply rx 0 ") && applies[0].contains("gain=1") && applies[0].contains(&format!("at={} ", ms(49))), "{applies:?}");
        // UR-26, design 38 D2 (4), RM-16's Ceiling: the end settles A at the loss's own instant —
        // its call began there, so it was not handed over by `S` — and B (due before it, given
        // nowhere, its channel gone) `no channel` — a row, not issued.
        control.finish();
        assert_eq!(rows_of(&core, &gain), [(ms(49), false, false), (ms(51), false, true)], "the command behind the loss wrote no row: {:?}", lock(&core.rec).applied);
        assert_eq!(cancelled_at(&core), [ms(52)], "the loss's instant settled the command handed at it");
        assert_eq!(events_of(&events, ezsdr_kernel::event::EventKind::DEVICE_LOST).len(), 1, "the test's own loss was not published once: {:?}", lock(&core.rec).timing);
    }

    #[test]
    fn ur_24_a_refused_release_records_no_row_and_keeps_the_projection() {
        // UR-24 (S9 e): a release the device refuses records no row and no configuration, and
        // the entry stays projected; the owner of a segment the record already holds configures
        // nothing, so the device keeps the value it had until a configure reads the projection.
        let inner = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..crate::device::FakeConfig::default() }));
        let fails = Arc::new(crate::provider::test_support::ApplyFails::timed(inner.clone(), Dir::Rx, None));
        let (core, time, _events) = crate::provider::test_support::rig_on_inputs(fails.clone(), vec![Arc::new(NoLink)], 2_000, BTreeMap::new());
        let _frozen = hold(&core, &time, &inner);
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        let ms = |n: i64| core.ticks(n * 1_000_000);
        channels_from_none(&mut control, &core, 2, 100, 1);
        let planned = plan_of(&core, Dir::Rx);
        let (by, origin) = (planned[0].segment.by.expect("the enable"), planned[0].segment.origin);
        let gain = key("radio.rx.c0.gain_db");
        control.book_timed(ActionId(3), gain.clone(), Value::num(3.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(50)))));
        time.advance_to(core.at(ms(47))).unwrap();
        control.release();
        assert!(owner_rx(&core).begin(), "the owner begins the enable's segment");
        assert_eq!(fails.attempts(), vec![(Dir::Rx, 0, None), (Dir::Rx, 0, Some(ms(50)))], "the owner configured a segment the record holds");
        assert_eq!(lock(&core.streams).stream(booked_stream(Dir::Rx)).configured, Some(by));
        device_at(&core, &time, &inner, origin + core.ticks(1_000_000));
        let projection = plan_of(&core, Dir::Rx)[0].settings.clone().expect("the settings");
        assert!(rows_of(&core, &gain).is_empty() && control.config[&gain] == Value::num(0.0).unwrap(), "the refused release wrote a row or the configuration of record");
        assert_eq!((inner.read_back(Dir::Rx, 0).gain, projection.settings_on(&core, &projection.updates.lock(), Dir::Rx, 0, origin, 0).gain), (Some(0.0), Some(3.0)), "the refused release is not the projection's");
    }

    #[test]
    fn ur_24_a_release_behind_a_released_command_keeps_the_device_queue_order() {
        // UR-24 (S12): a command booked behind one already released takes that one's instant at
        // its booking, with `LATE_COMMAND`, and the projection holds it there from then on; it is
        // given to the device at that instant, and its row reads it.
        let (core, device, time, events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let us = |n: i64| core.ticks(n * 1_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(us(n))));
        let gain = key("radio.rx.c0.gain_db");
        control.book_timed(ActionId(1), gain.clone(), Value::num(1.0).unwrap(), at(10_000));
        let first_seq = lock(&core.streams).seq;
        time.advance_to(core.at(us(7_500))).unwrap();
        control.release();
        lock(&core.streams).search(booked_stream(Dir::Rx), true);
        control.book_cold(ActionId(2), key("radio.rx.c0.stream"), Value::from(-1), at(9_000));
        control.book_cold(ActionId(3), key("radio.rx.c0.stream"), Value::from(0), at(100_000));
        control.book_timed(ActionId(4), gain.clone(), Value::num(2.0).unwrap(), at(9_700));
        let seq = lock(&core.streams).seq;
        let last = plan_of(&core, Dir::Rx).pop().expect("the later enable");
        let settings = last.settings.expect("its settings");
        let late = events_of(&events, kinds::LATE_COMMAND);
        assert_eq!(
            (late.len(), late[0].payload["requested"]["ticks"].as_i64(), late[0].payload["applied"]["ticks"].as_i64()),
            (1, Some(us(9_700)), Some(us(10_000))),
            "the command is not late at its booking: {late:?}",
        );
        assert_eq!(
            (control.held.keys().copied().collect::<Vec<_>>(), settings.updates.lock().keys().copied().collect::<Vec<_>>()),
            (vec![(us(10_000), seq)], vec![(us(10_000), first_seq), (us(10_000), seq)]),
            "the booking's floor did not reach the held list and the projection",
        );
        time.advance_to(core.at(us(7_600))).unwrap();
        control.release();
        let applied: Vec<String> = device.calls().into_iter().filter(|call| call.starts_with("apply rx 0") && call.contains("gain=2")).collect();
        let first = device.calls().iter().position(|call| call.starts_with("apply rx 0") && call.contains("gain=1")).expect("the first release");
        let second = device.calls().iter().position(|call| call.starts_with("apply rx 0") && call.contains("gain=2")).expect("the second release");
        assert_eq!((first < second, control.released.clone()), (true, vec![us(10_000), us(10_000)]), "{applied:?}");
        assert!(applied[0].contains(&format!("at={} effective={}", us(10_000), us(10_000))), "{applied:?}");
        assert_eq!(settings.updates.lock().keys().copied().collect::<Vec<_>>(), [(us(10_000), first_seq), (us(10_000), seq)], "the release moved the projection");
        // UR-24, design 38 D2 (3): the rows come at the instant the device applies at.
        time.advance_to(core.at(us(10_000))).unwrap();
        control.release();
        assert_eq!(rows_of(&core, &gain), [(us(10_000), true, false), (us(10_000), true, true)], "the rows do not read the instant the device applies at");
        device_at(&core, &time, &device, us(10_000) + 1);
        assert_eq!(device.read_back(Dir::Rx, 0).gain, Some(2.0));
    }

    #[test]
    fn ur_24_a_command_waiting_for_a_channel_is_booked_at_the_instant_it_can_take_effect() {
        // UR-24 (S12c): a command booked behind a released one is late at its booking, and its
        // instant is settled there: it waits for a channel that never comes, and its instant
        // passes with no channel, the row `no channel` there; no projection moves — the enable's
        // segment keeps the value it began with, and its owner, whose record holds its
        // configuration, configures nothing.
        let (core, device, time, events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[Dir::Tx]);
        control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        let us = |n: i64| core.ticks(n * 1_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(us(n))));
        channels_from_none(&mut control, &core, 2, 100, 1);
        control.book_cold(ActionId(3), key("radio.rx.c0.stream"), Value::from(-1), at(100_200));
        let projection = plan_of(&core, Dir::Rx)[0].settings.clone().expect("the enable's settings");
        time.advance_to(core.at(us(97_500))).unwrap();
        // One live command released first: the device queue's order.
        control.book_timed(ActionId(4), key("radio.tx.c0.gain_db"), Value::num(1.0).unwrap(), at(100_500));
        control.release();
        let gain = key("radio.rx.c0.gain_db");
        time.advance_to(core.at(us(97_600))).unwrap();
        control.book_timed(ActionId(5), gain.clone(), Value::num(3.0).unwrap(), at(99_700));
        let seq = projection.updates.lock().keys().copied().next().expect("the booked update").1;
        let late = events_of(&events, kinds::LATE_COMMAND);
        assert_eq!(
            (late.len(), late[0].payload["requested"]["ticks"].as_i64(), late[0].payload["applied"]["ticks"].as_i64()),
            (1, Some(us(99_700)), Some(us(100_500))),
            "the booking did not floor the command at the released one's instant: {late:?}",
        );
        control.release();
        let projected = projection.updates.lock();
        assert_eq!(
            (projected.keys().copied().collect::<Vec<_>>(), projection.settings_on(&core, &projected, Dir::Rx, 0, us(100_000), 0).gain, control.released.clone()),
            (vec![(us(100_500), seq)], Some(0.0), vec![us(100_500)]),
            "the waiting command's release moved its projection",
        );
        drop(projected);
        let mark = device.calls().len();
        assert!(owner_rx(&core).begin(), "the owner begins the enable's segment");
        assert!(!device.calls()[mark..].iter().any(|call| call.starts_with("apply") || call.starts_with("rx_open")), "the owner configured the segment again: {:?}", device.calls());
        // Its instant passes with no channel to give it on: the row is written then (UR-24).
        device_at(&core, &time, &device, us(101_000));
        control.release();
        assert_eq!(rows_of(&core, &gain), [(us(100_500), false, true)], "the waiting command's row");
        assert_eq!(device.read_back(Dir::Rx, 0).gain, Some(0.0));
    }

    #[test]
    fn ur_24_a_segment_booked_while_a_command_waits_for_a_channel_holds_it_at_its_instant() {
        // UR-24 (S12d): a segment booked while a command waits for a channel begins at its own
        // origin, inside the wait: the projection keeps the old value there and holds the
        // waiting value from its instant, and the release, finding the channel the segment
        // brought, gives the device that value at the instant — the record and the device's
        // settings agree.
        let (core, device, time) = manual_rig();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[]);
        let us = |n: i64| core.ticks(n * 1_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(us(n))));
        lock(&core.streams).search(booked_stream(Dir::Tx), true);
        // The stream's channel leaves long before the instant, so a segment booked now may
        // begin before it: the receive call's floor is the cut's plus one call.
        control.book_cold(ActionId(1), key("radio.tx.c0.stream"), Value::from(0), at(10_000));
        control.book_cold(ActionId(2), key("radio.tx.c0.stream"), Value::from(-1), at(20_000));
        time.advance_to(core.at(us(97_600))).unwrap();
        let gain = key("radio.tx.c0.gain_db");
        control.book_timed(ActionId(3), gain.clone(), Value::num(3.0).unwrap(), at(100_500));
        control.release();
        // The segment brings the channel at 100.3, inside the command's wait: the release gives
        // the device the value at its instant (UR-24).
        control.book_cold(ActionId(4), key("radio.tx.c0.stream"), Value::from(0), at(100_300));
        control.release();
        let later = plan_of(&core, Dir::Tx).pop().expect("the later enable");
        let settings = later.settings.expect("its settings");
        let updates = settings.updates.lock();
        assert_eq!(
            (settings.settings_on(&core, &updates, Dir::Tx, 0, us(100_300), 0).gain, settings.settings_on(&core, &updates, Dir::Tx, 0, us(100_500), 0).gain),
            (Some(0.0), Some(3.0)),
            "the segment booked while the command waits does not hold the value at its instant",
        );
        drop(updates);
        device_at(&core, &time, &device, us(100_400));
        let before = device.read_back(Dir::Tx, 0).gain;
        device_at(&core, &time, &device, us(100_600));
        // UR-24, design 38 D2 (3): the row is written when the instant has come.
        control.release();
        assert_eq!((before, device.read_back(Dir::Tx, 0).gain, rows_of(&core, &gain)), (Some(0.0), Some(3.0), vec![(us(100_500), true, false)]), "the device did not take the waited-for value at its instant");
        assert_eq!(control.config_at(us(101_000))[&gain], Value::num(3.0).unwrap(), "the given release is not the configuration of record after its instant");
    }

    #[test]
    fn ur_24_a_segment_begun_before_a_late_command_s_instant_configures_the_old_value() {
        // UR-24 (S12e): the owner's configure comes after the late booking and before the
        // release, and it reads the instant the booking settled: the segment beginning before it
        // goes to the device with the value in force at its origin, not with the released value.
        let (core, device, time) = manual_rig();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[Dir::Tx]);
        control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        let us = |n: i64| core.ticks(n * 1_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(us(n))));
        lock(&core.streams).search(booked_stream(Dir::Rx), true);
        control.book_cold(ActionId(2), key("radio.rx.c0.stream"), Value::from(0), at(100_000));
        time.advance_to(core.at(us(97_500))).unwrap();
        control.book_timed(ActionId(4), key("radio.tx.c0.gain_db"), Value::num(1.0).unwrap(), at(100_500));
        control.release();
        let gain = key("radio.rx.c0.gain_db");
        time.advance_to(core.at(us(97_600))).unwrap();
        control.book_timed(ActionId(5), gain.clone(), Value::num(3.0).unwrap(), at(99_700));
        let mark = device.calls().len();
        assert!(owner_rx(&core).begin(), "the owner begins the segment");
        let owner: Vec<String> = device.calls()[mark..].to_vec();
        control.release();
        assert!(!owner.iter().any(|call| call.contains("gain=3 ")), "the owner configured the released value: {owner:?}");
        device_at(&core, &time, &device, us(100_000) + 1);
        assert_eq!(device.read_back(Dir::Rx, 0).gain, Some(0.0), "the device took the released value before its instant");
    }

    #[test]
    fn ur_25_an_owner_configures_a_segment_the_record_s_command_is_not_for() {
        // UR-36 (S14): the record names the command of the configuration the device was given —
        // the second enable's — and the owner of the first segment configures it, the sets being
        // equal: the record's set alone is no reason to trust another command's configuration.
        let (core, device, _time) = manual_rig();
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        channels_from_none(&mut control, &core, 2, 100, 1);
        control.book_timed(ActionId(3), key("radio.rx.c0.gain_db"), Value::num(1.0).unwrap(), at(100));
        control.book_cold(ActionId(4), key("radio.rx.c0.stream"), Value::from(-1), at(200));
        control.book_timed(ActionId(5), key("radio.rx.c0.gain_db"), Value::num(2.0).unwrap(), at(300));
        channels_from_none(&mut control, &core, 6, 300, 1);
        let by = plan_of(&core, Dir::Rx).iter().find_map(|planned| planned.segment.by).expect("the first enable");
        let second = lock(&core.streams).stream(booked_stream(Dir::Rx)).configured.expect("the second enable's command");
        let mark = device.calls().len();
        let mut rx = owner_rx(&core);
        assert_ne!(second, by, "the record is the first segment's command, not the second enable's");
        assert!(rx.begin(), "the owner begins the first segment");
        let owner = device.calls()[mark..].to_vec();
        assert_eq!(lock(&core.streams).stream(booked_stream(Dir::Rx)).configured, Some(by), "the owner did not configure the segment its command is not the record's");
        assert!(owner.iter().all(|call| !call.starts_with("rx_open")), "the record's set is the segment's: {owner:?}");
        assert_eq!(owner.iter().filter(|call| call.starts_with("apply rx 0") && call.contains("gain=1 ")).count(), 1, "{owner:?}");
        assert!(!owner.iter().any(|call| call.contains("gain=2 ")), "the second enable's value was applied: {owner:?}");
    }

    #[test]
    fn ur_24_a_release_after_a_receive_channel_leaves_keeps_the_setting_until_its_instant() {
        // UR-24 (S16): a gain due just after the receive channel leaves is given to the device
        // at its instant on the later segment's channel; the device holds it only from then.
        let (core, device, time) = manual_rig();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        lock(&core.streams).search(booked_stream(Dir::Rx), true);
        control.book_cold(ActionId(1), key("radio.rx.c0.stream"), Value::from(-1), at(100));
        control.book_cold(ActionId(2), key("radio.rx.c0.stream"), Value::from(0), at(200));
        let gain = key("radio.rx.c0.gain_db");
        let e = ms(100) + core.ticks(100_000);
        control.book_timed(ActionId(3), gain.clone(), Value::num(3.0).unwrap(), Some(AbsoluteDeadline::new(core.at(e))));
        time.advance_to(core.at(ms(97) + core.ticks(500_000))).unwrap();
        control.release();
        let applies = device.calls().iter().filter(|call| call.starts_with("apply rx 0") && call.contains("gain=3")).count();
        device_at(&core, &time, &device, ms(100) - 1);
        let before_cut = device.read_back(Dir::Rx, 0).gain;
        device_at(&core, &time, &device, e - 1);
        let before = device.read_back(Dir::Rx, 0).gain;
        device_at(&core, &time, &device, e + 1);
        assert_eq!((applies, before_cut, before), (1, Some(0.0), Some(0.0)), "{:?}", device.calls());
        assert_eq!(device.read_back(Dir::Rx, 0).gain, Some(3.0));
    }

    #[test]
    fn ur_24_a_release_after_a_transmit_channel_leaves_keeps_the_setting_until_its_instant() {
        // UR-24 (S17): the same on the transmit side: uhd-control's timed apply reaches the
        // device at its instant, on the later segment's channel, with the streamer's own state
        // untouched.
        let (core, device, time) = manual_rig();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[Dir::Tx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        lock(&core.streams).search(booked_stream(Dir::Tx), true);
        control.book_cold(ActionId(1), key("radio.tx.c0.stream"), Value::from(-1), at(100));
        control.book_cold(ActionId(2), key("radio.tx.c0.stream"), Value::from(0), at(200));
        let gain = key("radio.tx.c0.gain_db");
        let e = ms(100) + core.ticks(100_000);
        control.book_timed(ActionId(3), gain.clone(), Value::num(3.0).unwrap(), Some(AbsoluteDeadline::new(core.at(e))));
        time.advance_to(core.at(ms(97) + core.ticks(500_000))).unwrap();
        control.release();
        let applies = device.calls().iter().filter(|call| call.starts_with("apply tx 0") && call.contains("gain=3")).count();
        device_at(&core, &time, &device, ms(100) - 1);
        let before_cut = device.read_back(Dir::Tx, 0).gain;
        device_at(&core, &time, &device, e - 1);
        let before = device.read_back(Dir::Tx, 0).gain;
        device_at(&core, &time, &device, e + 1);
        assert_eq!((applies, before_cut, before), (1, Some(0.0), Some(0.0)), "{:?}", device.calls());
        assert_eq!(device.read_back(Dir::Tx, 0).gain, Some(3.0));
    }

    #[test]
    fn ur_24_a_layout_release_after_a_channel_leaves_keeps_the_setting_until_its_instant() {
        // UR-24 (S18): under the layout the command acts on the numbered channel alone — the one
        // that left and joins again — and only from its instant.
        let (core, device, time) = layout_rig();
        let _frozen = hold(&core, &time, &device);
        let (to_tx, _tx) = channel();
        let mut control = Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], core.description.defaults(), false, Start { t0: 0, rx: vec![Some((Channels::first(2), 200)), None], tx: vec![None, None] });
        control.config.insert(key("radio.rx.c1.stream"), Value::from(0));
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        assert_eq!(plan_of(&core, Dir::Rx)[0].segment.config.channels, Channels::first(2), "the initial segment is not both front ends'");
        lock(&core.streams).search(booked_stream(Dir::Rx), true);
        let stream_of_c1 = key("radio.rx.c1.stream");
        control.book_cold(ActionId(1), stream_of_c1.clone(), Value::from(-1), at(100));
        control.book_cold(ActionId(2), stream_of_c1.clone(), Value::from(0), at(200));
        let gain = key("radio.rx.c1.gain_db");
        let e = ms(100) + core.ticks(100_000);
        control.book_timed(ActionId(3), gain.clone(), Value::num(3.0).unwrap(), Some(AbsoluteDeadline::new(core.at(e))));
        time.advance_to(core.at(ms(97) + core.ticks(500_000))).unwrap();
        control.release();
        let applies: Vec<String> = device.calls().into_iter().filter(|call| call.starts_with("apply rx") && call.contains("gain=3")).collect();
        device_at(&core, &time, &device, ms(100) - 1);
        let before_cut = device.read_back(Dir::Rx, 1).gain;
        device_at(&core, &time, &device, e - 1);
        let before = device.read_back(Dir::Rx, 1).gain;
        device_at(&core, &time, &device, e + 1);
        assert_eq!((applies.len(), before_cut, before), (1, Some(0.0), Some(0.0)), "{applies:?}");
        assert!(applies[0].starts_with("apply rx 1"), "the command acted on another channel: {applies:?}");
        assert_eq!(device.read_back(Dir::Rx, 1).gain, Some(3.0));
    }

    /// The timing rows of `what`, as the booking recorded them.
    fn timing_of(core: &Core, what: &str) -> Vec<serde_json::Value> {
        lock(&core.rec).timing.iter().filter(|row| row["what"] == what).cloned().collect()
    }

    #[test]
    fn ur_25_a_receive_enable_counts_its_start_lead_from_the_end_of_its_configuration() {
        // UR-25, RM-25 (spec 20, VF-6; design note 37): the `cold_change` row records the end
        // of the enable's configuration as `ready`, not the instant the change was booked at,
        // and the segment's origin is the first lattice instant at or after both `e` and that
        // end plus the profile's start lead. The controlled clock sets the end itself, inside
        // the configuration: the return side of the configuration's untimed `apply` holds the
        // enable, the clock stands at 0 while it runs and moves to 200 ms before the
        // configuration returns. A `ready` read before or at the booking is 0 then, so the
        // row's 200 ms can only be the configuration's end.
        let config = crate::device::FakeConfig { clock: crate::ClockMode::Manual, apply_delay: Wall::from_millis(80), ..crate::device::FakeConfig::default() };
        let device = Arc::new(crate::provider::test_support::LockSpy::new(config));
        let (core, time, _events) = crate::provider::test_support::rig_on(device.clone(), vec![Arc::new(NoLink)], 2_000);
        device.watch(&core);
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        let ms = |n: i64| core.ticks(n * 1_000_000);
        // The configuration's own apply: the gate is after the device returned, before the
        // section reads the clock again — inside the configuration, not before it.
        let gate = device.stop_after("apply");
        let core_ = core.clone();
        let host = device.host();
        let (done, finished) = channel();
        let booking = host.spawn("from-none-enable", move || {
            let mut control = control;
            lock(&core_.streams).search(booked_stream(Dir::Rx), false);
            control.book_cold(ActionId(2), key("radio.rx.c0.stream"), Value::from(0), Some(AbsoluteDeadline::new(core_.at(core_.ticks(100_000_000)))));
            let _ = done.send(());
        }).expect("a participant thread");
        gate.wait();
        assert_eq!(core.now(), 0, "the configuration had not begun at the clock's start");
        time.advance_to(core.at(ms(200))).unwrap();
        gate.release();
        assert!(finished.recv_timeout(Wall::from_secs(10)).is_ok(), "the enable did not book");
        booking.join().expect("the enable's thread");
        let row = timing_of(&core, "cold_change").into_iter().find(|row| row["key"] == "radio.rx.c0.stream").expect("the enable's row");
        let (e, ready, booked_at) = (row["e"].as_i64().unwrap(), row["ready"].as_i64().unwrap(), row["booked_at"].as_i64().unwrap());
        assert_eq!(booked_at, 0, "the enable was booked at the clock's start: {row}");
        // (a) `ready` is the configuration's end, not the booking's instant.
        assert_eq!(ready, ms(200), "the row does not record the end of the configuration: {row}");
        // (b) the origin is the first lattice instant at or after both `e` and that end + lead.
        let lead = core.ticks(ezsdr_radio::timeline::start_lead_ns(&core.description.timing, Direction::Rx));
        let planned = plan_of(&core, Dir::Rx);
        let origin = planned[0].segment.origin;
        let n = planned[0].segment.config.ratio.num() as i64;
        assert_eq!(origin, (e.max(ready + lead) + n - 1) / n * n, "the origin is not on the configuration's end: {row}, lead {lead}");
        assert!(origin > e, "the scene put the configuration's end past `e`: {row}");
        // The owner starts the segment at that origin, so the device is asked for no earlier one.
        let mut rx = owner_rx(&core);
        assert!(rx.begin(), "the owner begins the enable's segment");
        let calls = device.log();
        let start = calls.iter().rev().find(|call| call.starts_with("rx_start 0 ")).unwrap_or_else(|| panic!("{calls:?}"));
        assert_eq!(start["rx_start 0 ".len()..].parse::<i64>().unwrap_or_else(|_| panic!("{start}")), origin, "{start}");
    }

    #[test]
    fn ur_25_the_switch_applies_the_configuration_in_effect_at_e2() {
        // UR-25 (Review K, N-P2-9; design note 37): a retune held for an instant after the
        // switch's `e₂` is in the configuration the switch applies — its instant is before the
        // segment's own origin, which the cut and its lead put past `e₂` — and its release is
        // still its own instant. On the controlled clock.
        let (core, device, time, events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        device.rx_open(0, &[0]).unwrap();
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        let freq = key("radio.rx.c0.frequency_hz");
        control.book_timed(ActionId(1), freq.clone(), Value::num(2.1e9).unwrap(), at(320));
        control.book_cold(ActionId(2), key("radio.rx.s0.sample_rate_hz"), Value::num(2e6).unwrap(), at(300));
        let plan = plan_of(&core, Dir::Rx);
        assert_eq!(plan.len(), 2, "{:?}", plan.iter().map(|p| p.segment.origin).collect::<Vec<_>>());
        assert!(plan[1].segment.origin > ms(320), "the scene puts the origin past the retune: {:?}", plan[1].segment.origin);
        // The change is a stream the Run started: uhd-control's own from-none enable configures
        // nothing at booking (UR-25, VH-4), so the apply below is the switch's alone.
        assert!(!device.calls().iter().any(|call| call.starts_with("apply rx 0") && call.ends_with("at=-")), "the change was configured at its booking: {:?}", device.calls());
        plan[1].configure(&core, Dir::Rx).expect("the switch configures");
        let calls = device.calls();
        assert_eq!(calls.iter().filter(|call| call.starts_with("apply rx 0") && call.contains("rate=2000000") && call.contains("freq=2100000000") && call.ends_with("at=-")).count(), 1,
            "the switch's configuration does not carry the held retune: {calls:?}");
        // The held retune is still released at its own instant.
        time.advance_to(core.at(ms(318))).unwrap();
        control.release();
        let calls = device.calls();
        // UR-24 (R1): 320 ms is after the cut and before the switch's origin: no sample passes the
        // DDC then, so the retune is no timed command; the switch (configured here) already
        // carries it, and the release applies it untimed once more.
        assert_eq!(calls.iter().filter(|call| call.starts_with("apply rx 0") && call.contains("freq=2100000000") && !call.ends_with("at=-")).count(), 0,
            "the retune was given as a timed command: {calls:?}");
        assert_eq!(calls.iter().filter(|call| call.starts_with("apply rx 0") && call.contains("rate=-") && call.contains("freq=2100000000") && call.ends_with("at=-")).count(), 1,
            "the retune was not applied untimed: {calls:?}");
        assert_eq!(device.dsp_pending(Dir::Rx, 0), 0);
        assert!(events_of(&events, kinds::LATE_COMMAND).is_empty());
    }

    #[test]
    fn ur_25_an_enable_at_a_timed_update_s_instant_applies_it() {
        // UC-2, UR-25 (design 33 D15; design note 37): an enable from no stream whose `cold`
        // change is booked after a timed update at the same instant carries that update in the
        // configuration it applies itself; the change booked first does not, and the release
        // delivers it. On the controlled clock the order and the instants are fixed.
        for timed_first in [true, false] {
            let case = format!("timed first: {timed_first}");
            let (core, device, time, events) = manual_rig_events();
            let _frozen = hold(&core, &time, &device);
            let (mut control, _tx) = control(&core, &[]);
            control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
            let ms = |n: i64| core.ticks(n * 1_000_000);
            let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
            let freq = key("radio.rx.c0.frequency_hz");
            if timed_first {
                control.book_timed(ActionId(1), freq.clone(), Value::num(2.1e9).unwrap(), at(300));
            }
            lock(&core.streams).search(booked_stream(Dir::Rx), false);
            control.book_cold(ActionId(2), key("radio.rx.c0.stream"), Value::from(0), at(300));
            if !timed_first {
                control.book_timed(ActionId(3), freq.clone(), Value::num(2.1e9).unwrap(), at(300));
            }
            // The release window of the timed update, and the enable's own configuration.
            time.advance_to(core.at(ms(298))).unwrap();
            control.release();
            let calls = device.calls();
            let configured = calls.iter().filter(|call| call.starts_with("apply rx 0") && call.contains("freq=2100000000") && call.ends_with("at=-")).count();
            assert_eq!(configured, usize::from(timed_first), "{case}: the enable's configuration: {calls:?}");
            let timed = calls.iter().filter(|call| call.starts_with("apply rx 0") && call.contains("freq=2100000000") && call.contains(&format!("at={} effective=", ms(300)))).count();
            assert_eq!(timed, 1, "{case}: the timed release: {calls:?}");
            assert!(events_of(&events, kinds::LATE_COMMAND).is_empty(), "{case}");
            // UR-24, design 38 D2 (3): the released row comes at the instant. The enable's own
            // configure row is not the release's (`rows_of` reads the ones that are).
            time.advance_to(core.at(ms(300))).unwrap();
            control.release();
            assert_eq!(rows_of(&core, &freq), vec![(ms(300), true, false)], "{case}: the update's released row");
        }
    }

    /// The layout's control with `start`'s receive stream 0 on `channels` and `config`'s own
    /// values: the two receive channels the tests place and move (RM-27).
    fn layout_control(core: &Arc<Core>, channels: Channels, config: &[(&str, Value)]) -> Control {
        let (to_tx, _tx) = channel();
        let mut values = core.description.defaults();
        for (name, value) in config {
            values.insert(key(name), value.clone());
        }
        let start = Start { t0: 0, rx: vec![Some((channels, 200))], tx: vec![None] };
        Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], values, false, start)
    }

    #[test]
    fn ur_24_a_channel_keyed_timed_update_acts_on_its_own_channel() {
        // UR-24, item 7 (design note 37): a numbered channel key's timed update acts on that
        // channel alone — one command, one channel — and the stream untouched. On the controlled
        // clock the 1 ms apart instants cannot pass.
        for only_one in [false, true] {
            let case = format!("only_one: {only_one}");
            let (core, device, time, events) = {
                let config = crate::device::FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..crate::device::FakeConfig::default() };
                crate::provider::test_support::rig_with(config, vec![Arc::new(NoLink)])
            };
            let _frozen = hold(&core, &time, &device);
            let channels = if only_one { Channels::first(2).without(0) } else { Channels::first(2) };
            let mut control = layout_control(&core, channels, &[("radio.rx.c0.stream", Value::from(if only_one { -1 } else { 0 })), ("radio.rx.c1.stream", Value::from(0))]);
            let ms = |n: i64| core.ticks(n * 1_000_000);
            let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
            let freq = key("radio.rx.c1.frequency_hz");
            let gain = key("radio.rx.c1.gain_db");
            control.book_timed(ActionId(1), freq.clone(), Value::num(2.1e9).unwrap(), at(40));
            control.book_timed(ActionId(2), gain.clone(), Value::num(3.0).unwrap(), at(41));
            time.advance_to(core.at(ms(39) + core.ticks(500_000))).unwrap();
            control.release();
            let log = device.calls();
            let freq_at = |prefix: &str| log.iter().filter(|call| call.starts_with(prefix) && call.contains("freq=2100000000") && call.contains(&format!("at={} effective=", ms(40)))).count();
            assert_eq!(freq_at("apply rx 1 "), 1, "{case}: {log:?}");
            assert_eq!(freq_at("apply rx 0 "), 0, "{case}: {log:?}");
            assert_eq!(log.iter().filter(|call| call.starts_with("apply rx 1 ") && call.contains("gain=3") && !call.contains("at=-")).count(), 1, "{case}: {log:?}");
            assert_eq!(events_of(&events, kinds::COMMAND_REJECTED).len(), 0, "{case}: {log:?}");
            assert_eq!(events_of(&events, kinds::LATE_COMMAND).len(), 0, "{case}: {log:?}");
            assert_eq!(lock(&core.streams).line(booked_stream(Dir::Rx)).map_or(0, |line| line.plan.iter().filter(|segment| segment.cut.is_some()).count()), 0, "a timed update cuts nothing");
            // UR-24, design 38 D2 (3): the row comes at the instant.
            time.advance_to(core.at(ms(41))).unwrap();
            control.release();
            assert_eq!(rows_of(&core, &freq), vec![(ms(40), true, false)], "{case}");
        }
    }

    #[test]
    fn ur_24_a_channel_keyed_command_counts_one_queue_slot() {
        // UR-24, item 7 (design note 37): each numbered channel key's command is one
        // device-queue slot, held for its own instant.
        let (core, device, time, events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        let mut control = layout_control(&core, Channels::first(2), &[("radio.rx.c0.stream", Value::from(0)), ("radio.rx.c1.stream", Value::from(0))]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let far = ms(5_000);
        for i in 0..8 {
            control.book_timed(ActionId(10 + i), key("radio.rx.c0.gain_db"), Value::num(1.0).unwrap(), Some(AbsoluteDeadline::new(core.at(far + i as i64))));
            control.book_timed(ActionId(30 + i), key("radio.rx.c1.gain_db"), Value::num(2.0).unwrap(), Some(AbsoluteDeadline::new(core.at(far + 100 + i as i64))));
        }
        assert!(events_of(&events, kinds::COMMAND_QUEUE_FULL).is_empty(), "16 channel commands are 16 slots");
        assert_eq!(control.held.len(), 16);
        control.book_timed(ActionId(50), key("radio.rx.c1.gain_db"), Value::num(3.0).unwrap(), Some(AbsoluteDeadline::new(core.at(far + 200))));
        let full = events_of(&events, kinds::COMMAND_QUEUE_FULL);
        assert_eq!(full.len(), 1, "the 17th is refused");
        assert_eq!(full[0].payload["depth"], 16, "{:?}", full[0].payload);
        assert_eq!(control.held.len(), 16, "the refused one is held");
    }

    #[test]
    fn ur_25_a_layout_cold_change_is_floored_at_every_earlier_cold_change_of_the_instance() {
        // RM-25, item 11 (design note 37): a layout instance's `cold` change is floored at every
        // earlier, still pending `cold` change of the whole instance — across directions — and
        // its requested instant is the LATE_COMMAND's. On the controlled clock.
        // One receive channel on slot A and the transmit one on slot B, so that the receive
        // antenna may be `TX/RX` while the transmit channel is in a stream (RM-31).
        let selector = BTreeMap::from([
            (ezsdr_kernel::spec::Ident::parse("rx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0")])),
            (ezsdr_kernel::spec::Ident::parse("tx").unwrap(), Value::List(vec![Scalar::from("0:B:ubx:0")])),
        ]);
        let layout = ezsdr_radio::channels::from_selector(&selector).unwrap();
        let description = ezsdr_radio::channels::Description::new(&layout, 2_000, &crate::profile::terms());
        let config = crate::device::FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..crate::device::FakeConfig::default() };
        let device = Arc::new(crate::device::FakeDevice::new(config));
        let (core, time, events) = crate::provider::test_support::rig_on_description_links(device.clone(), vec![vec![Arc::new(NoLink)]], BTreeMap::new(), description);
        let _frozen = hold(&core, &time, &device);
        let (to_tx, _tx) = channel();
        let mut values = core.description.defaults();
        values.insert(key("radio.rx.c0.stream"), Value::from(0));
        values.insert(key("radio.tx.c0.stream"), Value::from(0));
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let start = Start { t0: 0, rx: vec![Some((Channels::first(1), 200))], tx: vec![Some((clock, Channels::first(1)))] };
        let mut control = Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], values, false, start);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        control.book_cold(ActionId(1), key("radio.tx.s0.sample_rate_hz"), Value::num(2e6).unwrap(), at(50));
        control.book_cold(ActionId(2), key("radio.rx.c0.antenna"), Value::from("TX/RX"), at(10));
        let late = events_of(&events, kinds::LATE_COMMAND);
        assert_eq!(late.len(), 1, "{late:?} {:?}", device.calls());
        assert_eq!(late[0].payload["key"], "radio.rx.c0.antenna");
        assert_eq!(late[0].payload["requested"]["ticks"], ms(10));
        assert_eq!(late[0].payload["applied"]["ticks"], ms(50));
        assert!(events_of(&events, kinds::COMMAND_REJECTED).is_empty(), "{:?}", device.calls());
        let changes = timing_of(&core, "cold_change");
        assert_eq!(changes.len(), 2, "{changes:?}");
        assert!(changes.iter().all(|row| row["e"] == ms(50)), "a change was not floored at the pending one: {changes:?}");
        // Each direction's cut is the floored instant, and the switch's configuration carries
        // the floored change; the other line's own change is the first sample at it.
        for dir in [Dir::Rx, Dir::Tx] {
            let plan = plan_of(&core, dir);
            let cut = plan[0].segment.cut.expect("the cut");
            assert_eq!(plan[0].segment.instant(cut).unwrap(), ms(50), "{dir:?}: the cut is not the floored instant");
            plan[1].configure(&core, dir).expect("the switch configures");
        }
        let calls = device.calls();
        assert_eq!(calls.iter().filter(|call| call.starts_with("apply rx 0 ") && call.contains("antenna=TX/RX") && call.ends_with("at=-")).count(), 1, "{calls:?}");
        assert!(calls.iter().any(|call| call.starts_with("apply tx 0 ") && call.contains("rate=2000000") && call.ends_with("at=-")), "{calls:?}");
    }

    #[test]
    fn ur_25_a_layout_cold_change_of_a_linkless_receive_side_is_floored_at_t0() {
        // RM-25 (round 1, P2-4; design note 37): the receive start T0 floors a layout `cold`
        // change of the receive side even when that side has no link yet and no line of its own,
        // and a change asked before T0 is late, recorded at T0: both the first and the second,
        // whose instant the first's recorded one floors. On the controlled clock.
        let config = crate::device::FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..crate::device::FakeConfig::default() };
        let device = Arc::new(crate::device::FakeDevice::new(config));
        let (core, time, events) = crate::provider::test_support::rig_on_description(device.clone(), Vec::new(), BTreeMap::new(), layout_description());
        let _frozen = hold(&core, &time, &device);
        let (to_tx, _tx) = channel();
        let t0 = core.ticks(2_000_000_000 + 1_000_000);
        let start = Start { t0, rx: vec![None], tx: vec![None] };
        let mut control = Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], core.description.defaults(), false, start);
        control.book_cold(ActionId(1), key("radio.rx.c0.antenna"), Value::from("TX/RX"), None);
        let late = events_of(&events, kinds::LATE_COMMAND);
        assert_eq!(late.len(), 1, "{late:?} {:?}", device.calls());
        assert_eq!(late[0].payload["key"], "radio.rx.c0.antenna");
        assert_eq!(late[0].payload["applied"]["ticks"], t0, "the floor is T0: {late:?}");
        assert!(late[0].payload["requested"]["ticks"].as_i64().unwrap() < t0, "{late:?}");
        control.book_cold(ActionId(2), key("radio.rx.c1.stream"), Value::from(-1), None);
        let late = events_of(&events, kinds::LATE_COMMAND);
        assert_eq!(late.len(), 1, "the recorded instant floors the next change: {late:?}");
        assert_eq!(late[0].payload["applied"]["ticks"], t0, "{late:?}");
        assert!(timing_of(&core, "cold_change").is_empty(), "a linkless side books no change: {:?}", timing_of(&core, "cold_change"));
    }

    #[test]
    fn ur_24_the_layout_destination_follows_the_effective_membership() {
        // RM-27, UR-24, major 4(b) (design note 37): a numbered channel's timed update is
        // judged against the membership in effect at its own instant, not the one at its
        // arrival — whichever of the join and the tune arrived first. On the controlled clock,
        // the join at 40 ms and the tune at 120 ms cannot move.
        for join_first in [false, true] {
            let (core, device, time, events) = manual_rig_events();
            let _frozen = hold(&core, &time, &device);
            let mut control = layout_control(&core, Channels::first(1), &[("radio.rx.c0.stream", Value::from(0)), ("radio.rx.c1.stream", Value::from(-1))]);
            let ms = |n: i64| core.ticks(n * 1_000_000);
            let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
            let freq = key("radio.rx.c1.frequency_hz");
            let join = |control: &mut Control| control.book_cold(ActionId(1), key("radio.rx.c1.stream"), Value::from(0), at(40));
            let tune = |control: &mut Control| control.book_timed(ActionId(2), freq.clone(), Value::num(2.1e9).unwrap(), at(120));
            if join_first {
                join(&mut control);
                tune(&mut control);
            } else {
                tune(&mut control);
                join(&mut control);
            }
            time.advance_to(core.at(ms(118))).unwrap();
            control.release();
            let log = device.calls();
            let applied = log.iter().filter(|call| call.starts_with("apply rx 1 ") && call.contains("freq=2100000000") && call.contains(&format!("at={} effective=", ms(120)))).count();
            assert_eq!(applied, 1, "join_first={join_first}: the tune reaches channel 1 on its new segment: {log:?}");
            assert!(events_of(&events, kinds::COMMAND_REJECTED).is_empty(), "join_first={join_first}: {log:?}");
            // UR-24, design 38 D2 (3): the row comes at the instant.
            time.advance_to(core.at(ms(120))).unwrap();
            control.release();
            assert_eq!(rows_of(&core, &freq), vec![(ms(120), true, false)], "join_first={join_first}");
        }
    }

    /// The layout with two channels each way — and so two streams each way (RM-29).
    fn both_description() -> ezsdr_radio::channels::Description {
        let selector = BTreeMap::from([
            (ezsdr_kernel::spec::Ident::parse("rx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:B:ubx:0")])),
            (ezsdr_kernel::spec::Ident::parse("tx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:B:ubx:0")])),
        ]);
        let layout = ezsdr_radio::channels::from_selector(&selector).unwrap();
        ezsdr_radio::channels::Description::new(&layout, 2_000, &crate::profile::terms())
    }

    /// `both_description`'s manual rig, each receive stream with a link.
    fn both_rig() -> (Arc<Core>, Arc<crate::device::FakeDevice>, Arc<ezsdr_kernel::time::ManualTimeAuthority>, Arc<ezsdr_kernel::event::EventCollector>) {
        let config = crate::device::FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..crate::device::FakeConfig::default() };
        let device = Arc::new(crate::device::FakeDevice::new(config));
        let links = vec![vec![Arc::new(NoLink) as Arc<dyn DataLink>], vec![Arc::new(NoLink)]];
        let (core, time, events) = crate::provider::test_support::rig_on_description_links(device.clone(), links, BTreeMap::new(), both_description());
        (core, device, time, events)
    }

    /// The plan of stream `k` of `dir`.
    fn plan_of_stream(core: &Core, dir: Dir, k: usize) -> Vec<Planned> {
        lock(&core.streams).planned(stream_key(dir, k))
    }

    /// A plan as its segments' origin, cut instant, channels and refusal: what a message prints.
    fn plan_rows(plan: &[Planned]) -> Vec<(i64, Option<i64>, Channels, bool)> {
        plan.iter().map(|planned| (planned.segment.origin, planned.segment.cut.map(|cut| planned.segment.instant(cut).unwrap()), planned.segment.config.channels, planned.refused)).collect()
    }

    /// `both_rig`'s control with channel 0 in stream 0 and channel 1 in stream 1 each way.
    fn both_control(core: &Arc<Core>) -> Control {
        let (to_tx, _tx) = channel();
        let mut values = core.description.defaults();
        for name in ["radio.rx.c0.stream", "radio.tx.c0.stream"] {
            values.insert(key(name), Value::from(0));
        }
        for name in ["radio.rx.c1.stream", "radio.tx.c1.stream"] {
            values.insert(key(name), Value::from(1));
        }
        let rx = vec![Some((Channels::first(1), 200)), Some((Channels::first(2).without(0), 200))];
        let tx = vec![
            Some((core.register_in(Dir::Tx, 0, 200, 0).unwrap(), Channels::first(1))),
            Some((core.register_in(Dir::Tx, 1, 200, 0).unwrap(), Channels::first(2).without(0))),
        ];
        Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], values, false, Start { t0: 0, rx, tx })
    }

    #[test]
    fn ur_25_a_same_size_membership_swap_reopens_the_streamer() {
        // Item 10 (M1; design note 37): two membership changes at one instant leave the same
        // *count* — `{0}` becomes `{1}` — so a comparison by count reopens nothing and the
        // streamer keeps serving device channel 0 as the logical channel 1; the set is what
        // decides. On the controlled clock the two changes are at one instant, and the new
        // segment's streamer is opened for its own set.
        let (core, device, time, events) = both_rig();
        let _frozen = hold(&core, &time, &device);
        let mut values = core.description.defaults();
        values.insert(key("radio.rx.c0.stream"), Value::from(0));
        values.insert(key("radio.rx.c1.stream"), Value::from(-1));
        let (to_tx, _tx) = channel();
        let mut control = Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], values, false,
            Start { t0: 0, rx: vec![Some((Channels::first(1), 200)), None], tx: vec![Some((core.register_in(Dir::Tx, 0, 200, 0).unwrap(), Channels::first(1))), None] });
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        lock(&core.streams).search(booked_stream(Dir::Rx), true);
        control.book_cold(ActionId(1), key("radio.rx.c0.stream"), Value::from(-1), at(100));
        control.book_cold(ActionId(2), key("radio.rx.c1.stream"), Value::from(0), at(100));
        let plan = plan_of_stream(&core, Dir::Rx, 0);
        let last = plan.last().expect("the new segment");
        assert_eq!(last.segment.config.channels, Channels::first(2).without(0), "the new segment is not `{{1}}`: {:?}", last.segment.config.channels);
        let cut = plan.iter().find(|planned| planned.segment.cut.is_some()).expect("the cut").segment;
        assert_eq!(cut.instant(cut.cut.unwrap()).unwrap(), ms(100), "{:?}", plan_rows(&plan));
        assert!(last.segment.origin > ms(100), "the new segment begins after the cut's restart: {:?}", plan_rows(&plan));
        let changes = timing_of(&core, "cold_change");
        assert_eq!(changes.len(), 2, "the two changes at one instant: {changes:?}");
        assert!(changes.iter().all(|row| row["e"] == ms(100)), "a change was booked at another instant: {changes:?}");
        // The owner's switch opens the new set's streamer, whatever the count says: the record
        // holds the old set `{0}`, whose count is the new one's. (The owner itself and the
        // samples of its new segment are `rx.rs`'s and `tx.rs`'s same-size tests.)
        let stream = stream_key(Dir::Rx, 0);
        {
            let mut streams = lock(&core.streams);
            streams.record_open(stream, Some(Channels::first(1)));
            streams.record_configured(stream, None);
        }
        let new = plan.last().unwrap().clone();
        assert!(core.owner_configure(Dir::Rx, 0, &new).unwrap(), "the owner begins the new segment");
        let calls = device.calls();
        let opened: Vec<&String> = calls.iter().filter(|call| call.starts_with("rx_open 0 ")).collect();
        assert_eq!(opened, ["rx_open 0 [1]"], "the streamer was not reopened for the new set: {calls:?}");
        assert!(calls.iter().any(|call| call.starts_with("apply rx 1 ") && call.ends_with("at=-")), "the new set's channel is configured: {calls:?}");
        assert!(calls.iter().all(|call| !call.starts_with("apply rx 0 ") || !call.ends_with("at=-")), "c0 left its stream: {calls:?}");
        assert!(events_of(&events, kinds::COMMAND_REJECTED).is_empty(), "{calls:?}");
        // The transmit sibling: the same one-channel count change `{0}` → `{1}` on its own
        // streamer, which a count comparison would leave serving device channel 0.
        let (core, device, time, events) = both_rig();
        let frozen = hold(&core, &time, &device);
        let mut values = core.description.defaults();
        values.insert(key("radio.tx.c0.stream"), Value::from(0));
        values.insert(key("radio.tx.c1.stream"), Value::from(-1));
        let (to_tx, _tx) = channel();
        let mut control = Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], values, false,
            Start { t0: 0, rx: vec![None, None], tx: vec![Some((core.register_in(Dir::Tx, 0, 200, 0).unwrap(), Channels::first(1))), None] });
        control.book_cold(ActionId(1), key("radio.tx.c0.stream"), Value::from(-1), at(100));
        control.book_cold(ActionId(2), key("radio.tx.c1.stream"), Value::from(0), at(100));
        let plan = plan_of_stream(&core, Dir::Tx, 0);
        assert_eq!(plan.last().expect("the new segment").segment.config.channels, Channels::first(2).without(0), "{:?}", plan_rows(&plan));
        {
            let mut streams = lock(&core.streams);
            streams.record_open(stream_key(Dir::Tx, 0), Some(Channels::first(1)));
            streams.record_configured(stream_key(Dir::Tx, 0), None);
        }
        drop(frozen);
        assert!(core.owner_configure(Dir::Tx, 0, plan.last().unwrap()).unwrap(), "the owner begins the new transmit segment");
        let calls = device.calls();
        let opened: Vec<&String> = calls.iter().filter(|call| call.starts_with("tx_open 0 ")).collect();
        assert_eq!(opened, ["tx_open 0 [1]"], "the transmit streamer was not reopened for the new set: {calls:?}");
        assert!(calls.iter().any(|call| call.starts_with("apply tx 1 ") && call.ends_with("at=-")), "the new set's channel is configured: {calls:?}");
        assert!(calls.iter().all(|call| !call.starts_with("apply tx 0 ") || !call.ends_with("at=-")), "c0 left its stream: {calls:?}");
        assert!(events_of(&events, kinds::COMMAND_REJECTED).is_empty(), "{calls:?}");
    }

    #[test]
    fn ur_34_a_timed_update_crossing_the_move_reaches_each_streamer_channel_at_its_instant() {
        // S8/A7 (design note 37): `apply` names device channel 1, not a stream number; compare
        // issued instants and projection/effective values on either side of the membership move.
        // On the controlled clock the move at 200 ms, the two updates, and each read-back are
        // fixed instants.
        let (core, device, time, events) = both_rig();
        let _frozen = hold(&core, &time, &device);
        let mut values = core.description.defaults();
        values.insert(key("radio.rx.c0.stream"), Value::from(0));
        values.insert(key("radio.rx.c1.stream"), Value::from(0));
        let (to_tx, _tx) = channel();
        let mut control = Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], values, false,
            Start { t0: 0, rx: vec![Some((Channels::first(2), 200)), None], tx: vec![Some((core.register_in(Dir::Tx, 0, 200, 0).unwrap(), Channels::first(1))), None] });
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        let freq = key("radio.rx.c1.frequency_hz");
        lock(&core.streams).search(booked_stream(Dir::Rx), true);
        control.book_cold(ActionId(1), key("radio.rx.c1.stream"), Value::from(1), at(200));
        control.book_timed(ActionId(2), freq.clone(), Value::num(10e6).unwrap(), at(170));
        control.book_timed(ActionId(3), freq.clone(), Value::num(20e6).unwrap(), at(280));
        // Before the move: the earlier update is issued at its instant and in force.
        time.advance_to(core.at(ms(168))).unwrap();
        control.release();
        let calls = device.calls();
        assert!(calls.iter().any(|call| call.starts_with("apply rx 1 ") && call.contains("freq=10000000") && call.contains(&format!("at={} effective={}", ms(170), ms(170)))), "{calls:?}");
        device_at(&core, &time, &device, ms(180));
        // The channel is in stream 0's running segment at 170 ms, so the update is a timed command;
        // no sample passes the unit rig's DDC, so it waits in the FIFO (UR-33).
        assert_eq!(device.dsp_pending(Dir::Rx, 1), 1, "the earlier update waits for a sample");
        // The move's destination configures the projected settings, and the earlier value stands.
        let destination = plan_of_stream(&core, Dir::Rx, 1).into_iter().find(|planned| planned.segment.config.channels.contains(1)).expect("the moved channel's destination segment");
        destination.configure(&core, Dir::Rx).expect("the destination configures");
        let calls = device.calls();
        assert!(calls.iter().any(|call| call.starts_with("apply rx 1 ") && call.contains("freq=10000000") && call.ends_with("at=-")), "the destination's projected configuration: {calls:?}");
        device_at(&core, &time, &device, ms(270));
        assert_eq!(device.freq_at(Dir::Rx, 1, ms(270)), 10e6, "the earlier value remains effective before the later update");
        // The later update at its own instant, and in force after it.
        time.advance_to(core.at(ms(278))).unwrap();
        control.release();
        device_at(&core, &time, &device, ms(300));
        assert_eq!(device.dsp_pending(Dir::Rx, 1), 2, "the later timed update waits behind the earlier one for samples");
        let calls = device.calls();
        assert!(calls.iter().any(|call| call.starts_with("apply rx 1 ") && call.contains("freq=20000000") && call.contains(&format!("at={} effective={}", ms(280), ms(280)))), "{calls:?}");
        assert!(events_of(&events, kinds::COMMAND_REJECTED).is_empty(), "{calls:?}");
        assert!(events_of(&events, kinds::COMMAND_QUEUE_FULL).is_empty(), "{calls:?}");
        // UR-24, design 38 D2 (3): the rows come at their instants.
        control.release();
        assert_eq!(rows_of(&core, &freq), vec![(ms(170), true, false), (ms(280), true, false)]);
    }

    #[test]
    fn ur_34_a_cold_change_before_the_move_s_release_is_floored_at_the_move() {
        // S10/A7 (design note 37): a `cold` change whose instant precedes a pending move's is
        // floored at the move's `e`, so the destination's first segment is the move's own and
        // the change is part of it. On the controlled clock.
        let (core, device, time, events) = both_rig();
        let _frozen = hold(&core, &time, &device);
        let mut control = both_control(&core);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        lock(&core.streams).search(booked_stream(Dir::Rx), true);
        control.book_cold(ActionId(1), key("radio.rx.c0.stream"), Value::from(1), at(100));
        control.book_cold(ActionId(2), key("radio.rx.s1.sample_rate_hz"), Value::num(2e6).unwrap(), at(50));
        let late = events_of(&events, kinds::LATE_COMMAND);
        assert_eq!(late.len(), 1, "{late:?} {:?}", device.calls());
        assert_eq!(late[0].payload["key"], "radio.rx.s1.sample_rate_hz");
        assert_eq!(late[0].payload["requested"]["ticks"], ms(50));
        assert_eq!(late[0].payload["applied"]["ticks"], ms(100));
        assert!(events_of(&events, kinds::COMMAND_REJECTED).is_empty(), "{:?}", device.calls());
        let plan = plan_of_stream(&core, Dir::Rx, 1);
        assert!(plan.iter().any(|planned| planned.segment.config.channels == Channels::first(2)), "the destination's segment is not `{{0, 1}}`: {:?}", plan_rows(&plan));
        let running = plan.iter().filter(|planned| planned.segment.cut.is_none()).max_by_key(|planned| planned.segment.origin).expect("the running segment");
        assert!(running.segment.origin >= ms(100), "the floored segment begins before the move: {:?}", running.segment.origin);
        assert_eq!(running.segment.config.ratio, Rational::new(100, 1).unwrap(), "the destination's segment is not at the change's rate: {:?}", plan_rows(&plan));
    }

    #[test]
    fn ur_34_two_moves_at_one_instant_swap_their_receive_channels_without_deadlock() {
        // A2 (design note 37): two moves at one instant swap the channels; each line acquires
        // its new set at that same instant, so no zero-duration segment holding both channels
        // is ever planned for an owner to start. On the controlled clock both are booked at
        // `e` and neither wait can be what lets them through.
        let (core, device, time, events) = both_rig();
        let _frozen = hold(&core, &time, &device);
        let mut control = both_control(&core);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        lock(&core.streams).search(stream_key(Dir::Rx, 0), true);
        lock(&core.streams).search(stream_key(Dir::Rx, 1), true);
        control.book_cold(ActionId(1), key("radio.rx.c0.stream"), Value::from(1), at(100));
        control.book_cold(ActionId(2), key("radio.rx.c1.stream"), Value::from(0), at(100));
        assert!(events_of(&events, kinds::COMMAND_REJECTED).is_empty(), "{:?}", device.calls());
        let moves = timing_of(&core, "move");
        assert_eq!(moves.len(), 2, "{moves:?}");
        assert!(moves.iter().all(|row| row["e"] == ms(100)), "{moves:?}");
        let both = Channels::first(2);
        let mut origins = Vec::new();
        for (k, want) in [(0, Channels::first(2).without(0)), (1, Channels::first(2).without(1))] {
            let plan = plan_of_stream(&core, Dir::Rx, k);
            assert_eq!(plan.last().expect("the new segment").segment.config.channels, want, "stream {k}: {:?}", plan_rows(&plan));
            assert!(plan.iter().all(|planned| planned.segment.config.channels != both), "stream {k} plans a segment holding both channels: {:?}", plan_rows(&plan));
            let cut = plan.iter().find(|planned| planned.segment.cut.is_some()).expect("the cut").segment;
            assert_eq!(cut.instant(cut.cut.unwrap()).unwrap(), ms(100), "stream {k}'s cut is not the one instant: {:?}", plan_rows(&plan));
            origins.push(plan.last().unwrap().segment.origin);
        }
        assert!(origins.iter().all(|origin| *origin > ms(100)), "a destination begins before its cut's restart: {origins:?}");
        // The owners' own exchange — each receives its old segment through the cut, finishes
        // it, and its begin of the destination closes the old streamer and waits for the
        // other's — is `ur_34_two_moves_at_one_instant_swap_their_receive_channels_without_deadlock`
        // in `src/provider/rx.rs`, on the real owner; this test is the plan it carries out.
    }

    #[test]
    fn ur_34_two_moves_at_one_instant_swap_their_transmit_channels_without_deadlock() {
        // A2/A10 (design note 37): the same exchange on TX. Each line's new set begins at the
        // one instant, so the union set is never a planned segment an owner could open.
        let (core, device, time, events) = both_rig();
        let _frozen = hold(&core, &time, &device);
        let mut control = both_control(&core);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        control.book_cold(ActionId(1), key("radio.tx.c0.stream"), Value::from(1), at(100));
        control.book_cold(ActionId(2), key("radio.tx.c1.stream"), Value::from(0), at(100));
        assert!(events_of(&events, kinds::COMMAND_REJECTED).is_empty(), "{:?}", device.calls());
        let both = Channels::first(2);
        let mut origins = Vec::new();
        for (k, want) in [(0, Channels::first(2).without(0)), (1, Channels::first(2).without(1))] {
            let plan = plan_of_stream(&core, Dir::Tx, k);
            assert_eq!(plan.last().expect("the new segment").segment.config.channels, want, "stream {k}: {:?}", plan_rows(&plan));
            // The one zero-length segment the exchange's two moves leave has no samples: no
            // owner can run it (A2).
            for planned in plan.iter().filter(|planned| planned.segment.config.channels == both) {
                let cut = planned.segment.cut.expect("a segment with both channels is cut");
                assert_eq!(planned.segment.instant(cut).unwrap(), planned.segment.origin, "stream {k} plans a segment holding both channels with samples: {:?}", plan_rows(&plan));
            }
            let cut = plan.iter().find(|planned| planned.segment.cut.is_some()).expect("the cut").segment;
            assert_eq!(cut.instant(cut.cut.unwrap()).unwrap(), ms(100), "stream {k}: {:?}", plan_rows(&plan));
            origins.push(plan.last().unwrap().segment.origin);
        }
        assert!(origins.iter().all(|origin| *origin > ms(100)), "a destination begins before its cut's restart: {origins:?}");
    }

    #[test]
    fn ur_34_a_finite_middle_segment_releases_a_channel_along_a_move_chain() {
        // A2 (design note 37): B's segment has a finite cut at e₂; the channel C takes begins
        // only there, so B runs between the two instants and releases c1 to C at its own cut.
        // On the controlled clock the two instants are 150 ms apart. B's delivery through its
        // cut is `ur_17_a_finite_segment_delivers_through_its_cut` in `src/provider/rx.rs`, and
        // the wait for it `ur_34_a_waited_open_waits_for_a_finite_unprocessed_move_segment` in
        // this file.
        let selector = BTreeMap::from([
            (ezsdr_kernel::spec::Ident::parse("rx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:B:basic_rx:0"), Scalar::from("0:B:basic_rx:1")])),
            (ezsdr_kernel::spec::Ident::parse("tx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0")])),
        ]);
        let layout = ezsdr_radio::channels::from_selector(&selector).unwrap();
        let description = ezsdr_radio::channels::Description::new(&layout, 2_000, &crate::profile::terms());
        let config = crate::device::FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, front_ends: vec!["UBX", "BasicRX"], ..crate::device::FakeConfig::default() };
        let device = Arc::new(crate::device::FakeDevice::new(config));
        let links = (0..3).map(|_| vec![Arc::new(NoLink) as Arc<dyn DataLink>]).collect();
        let (core, time, events) = crate::provider::test_support::rig_on_description_links(device.clone(), links, BTreeMap::new(), description);
        let _frozen = hold(&core, &time, &device);
        let (to_tx, _tx) = channel();
        let mut values = core.description.defaults();
        values.insert(key("radio.rx.c0.stream"), Value::from(-1));
        values.insert(key("radio.rx.c1.stream"), Value::from(0));
        values.insert(key("radio.rx.c2.stream"), Value::from(-1));
        let mut control = Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], values, false,
            Start { t0: 0, rx: vec![Some((Channels::first(2).without(0), 200)), None, None], tx: vec![Some((core.register_in(Dir::Tx, 0, 200, 0).unwrap(), Channels::first(1)))] });
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        lock(&core.streams).search(stream_key(Dir::Rx, 0), true);
        control.book_cold(ActionId(1), key("radio.rx.c1.stream"), Value::from(1), at(100));
        control.book_cold(ActionId(2), key("radio.rx.c1.stream"), Value::from(2), at(250));
        assert!(events_of(&events, kinds::COMMAND_REJECTED).is_empty(), "{:?}", device.calls());
        let middle = plan_of_stream(&core, Dir::Rx, 1).into_iter().find(|planned| planned.segment.cut.is_some() && planned.segment.config.channels == Channels::first(2).without(0)).expect("B's finite segment");
        let cut = middle.segment.cut.expect("B's segment is finite");
        let cut_end = middle.segment.instant(cut).unwrap();
        assert_eq!(cut_end, ms(250), "B's cut is not e₂");
        assert!(cut_end > middle.segment.origin, "B's segment holds no samples");
        assert_eq!(middle.segment.config.channels, Channels::first(2).without(0), "B's segment is not `{{1}}`");
        let destination = plan_of_stream(&core, Dir::Rx, 2).into_iter().find(|planned| planned.segment.config.channels.contains(1)).expect("C's segment");
        assert!(destination.segment.origin > cut_end, "C begins before B's cut: {:?}", plan_rows(&plan_of_stream(&core, Dir::Rx, 2)));
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
        let waveform = ezsdr_kernel::manifest::ingest_input(
            ezsdr_kernel::spec::Ident::parse("waveform").unwrap(),
            ezsdr_kernel::spec::Namespace::parse("ezsdr.input").unwrap(),
            format!("mem:{hash}"),
            &bytes,
        );
        let device = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..crate::device::FakeConfig::default() }));
        let links = vec![vec![Arc::new(NoLink) as Arc<dyn DataLink>]];
        let (core, time, events) = crate::provider::test_support::rig_on_description_links(device.clone(), links, BTreeMap::from([(hash, Arc::<[u8]>::from(bytes))]), layout_description());
        let _frozen = hold(&core, &time, &device);
        let (mut control, to_tx) = control(&core, &[Dir::Rx, Dir::Tx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let clock = plan_of(&core, Dir::Tx)[0].clock.expect("the transmit clock");
        let k = (ms(30) - clock.origin) / clock.n;
        assert!(clock.instant(k) >= ms(30), "the burst is booked ahead of its instant");
        control.book(Dispatched::new(ActionId(1), Action::TxBurst {
            target: super::super::test_support::target_of(&ResourceId::parse("usrp/tx/s0").unwrap()),
            waveform,
            repeat: false,
            at: AbsoluteDeadline::new(TimePoint::new(clock.domain, k)),
            requested_at: None,
            late_policy: ezsdr_kernel::stream::LatePolicy::SendAsapAndFlag,
            metadata: BTreeMap::new(),
        }, Some(ResourceId::parse("usrp/tx/s0").unwrap())));
        assert!(matches!(to_tx.try_recv(), Ok(TxCmd::Burst(_))), "the burst was not handed to uhd-tx");
        let held = lock(&core.held).len();
        assert_eq!(held, 1, "the burst is not held ahead of its instant");
        control.book(Dispatched::new(ActionId(2), Action::Stop { target: Some(super::super::test_support::target_of(&ResourceId::parse("usrp/rx/s0").unwrap())) }, Some(ResourceId::parse("usrp/rx/s0").unwrap())));
        assert!(to_tx.try_recv().is_err(), "the receive stream's Stop reached uhd-tx");
        assert_eq!(lock(&core.held).len(), held, "the receive stream's Stop cancelled the transmit burst");
        assert_eq!(lock(&core.streams).line(stream_key(Dir::Tx, 0)).map_or(0, |line| line.items.iter().filter(|item| item.kind == Kind::Stop).count()), 0, "the receive Stop was booked on the transmit line");
        assert_eq!(lock(&core.streams).line(stream_key(Dir::Rx, 0)).map_or(0, |line| line.items.iter().filter(|item| item.kind == Kind::Stop).count()), 1, "the receive stream is not stopped");
        assert!(events_of(&events, kinds::COMMAND_REJECTED).is_empty(), "{:?}", device.calls());
    }

    #[test]
    fn ur_25_a_floor_raised_layout_join_is_judged_at_its_instant() {
        // RM-25, RM-31, major 4(a) (design note 37): a join the instance floor raises takes
        // effect at the raised instant, where the reserved antenna has arrived, and the
        // run-time judgement is over that instant's configuration; the refusal leaves neither
        // the value nor the instant in the floor. On the controlled clock the raised instant
        // is 50 ms and the judge's configuration cannot move.
        let (core, device, time, events) = manual_rig_events();
        let frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        let antenna = key("radio.rx.c0.antenna");
        control.book_cold(ActionId(1), antenna.clone(), Value::from("TX/RX"), at(50));
        control.book_cold(ActionId(2), key("radio.tx.c0.stream"), Value::from(0), at(10));
        let refused = events_of(&events, kinds::COMMAND_REJECTED);
        assert_eq!(refused.len(), 1, "{refused:?} {:?}", device.calls());
        let reason = refused[0].payload["reason"].as_str().unwrap().to_owned();
        assert!(reason.starts_with("RM-31:"), "{reason}");
        assert!(device.calls().iter().all(|call| !call.starts_with("tx_open")), "no transmit streamer was opened: {:?}", device.calls());
        assert!(lock(&core.streams).stream(stream_key(Dir::Tx, 0)).clocks.is_empty(), "the refused join started a transmit clock");
        // The refusal left neither the value nor its instant behind: the antenna change back is
        // booked at its own instant, and the same join after it is taken.
        control.book_cold(ActionId(3), antenna.clone(), Value::from("RX2"), at(60));
        drop(frozen);
        control.book_cold(ActionId(4), key("radio.tx.c0.stream"), Value::from(0), at(120));
        assert_eq!(events_of(&events, kinds::COMMAND_REJECTED).len(), 0, "the later join is taken: {:?}", device.calls());
        let changes = timing_of(&core, "cold_change");
        let antennas: Vec<i64> = changes.iter().filter(|row| row["key"] == "radio.rx.c0.antenna").map(|row| row["e"].as_i64().unwrap()).collect();
        assert_eq!(antennas, [ms(50), ms(60)], "the refused instant is no floor: {changes:?}");
        assert_eq!(changes.iter().filter(|row| row["key"] == "radio.tx.c0.stream").map(|row| row["e"].as_i64().unwrap()).collect::<Vec<_>>(), [ms(120)], "the later join is not at its own instant: {changes:?}");
        let plan = plan_of(&core, Dir::Tx);
        assert!(plan.len() == 1 && plan[0].segment.origin == ms(120), "the later join begins no transmit segment: {:?}", plan_rows(&plan));
        plan[0].configure(&core, Dir::Tx).expect("the joined segment configures");
        assert!(device.calls().iter().any(|call| call.starts_with("apply tx 0 rate=") && call.ends_with("at=-")), "{:?}", device.calls());
    }

    #[test]
    #[should_panic(expected = "under the line lock")]
    fn ur_25_a_configuration_taken_under_the_line_lock_is_refused() {
        // UR-25 (S28, B5): the order is the configuration's `updates`, then the line; a
        // configure that holds the line already is refused by the test build's check.
        let (core, _device, _time) = rig();
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        control.book_cold(ActionId(1), key("radio.rx.s0.sample_rate_hz"), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(core.at(core.ticks(100_000_000)))));
        let planned = plan_of(&core, Dir::Rx)[1].clone();
        let _line = lock(&core.streams);
        let _ = planned.configure(&core, Dir::Rx);
    }

    // ------------------------------------------------ design 38: the floor, the window, the end

    /// The timed applies of one direction's gain, in call order, as `(gain, at)`.
    fn timed_gains(device: &crate::device::FakeDevice, dir: &str) -> Vec<(f64, i64)> {
        device.calls().into_iter()
            .filter(|call| call.starts_with(&format!("apply {dir} ")) && call.contains("gain=") && !call.ends_with("at=-"))
            .map(|call| {
                let gain = call.split("gain=").nth(1).unwrap().split(' ').next().unwrap().parse().unwrap();
                let at = call.split("at=").nth(1).unwrap().split(' ').next().unwrap().parse().unwrap();
                (gain, at)
            })
            .collect()
    }

    /// `Provider::stop` at `at`, as the Module books it (UR-26).
    fn stop_end(core: &Core, at: i64) {
        let mut streams = lock(&core.streams);
        streams.stop = Some((at, ezsdr_kernel::module_api::StopMode::Orderly));
        streams.end(core, at, false, false);
    }

    /// The `cancelled_at` of every cancelled row (UR-30).
    fn cancelled_at(core: &Core) -> Vec<i64> {
        lock(&core.rec).applied.iter().filter(|row| !row["cancelled"].is_null())
            .map(|row| row["cancelled_at"]["ticks"].as_i64().unwrap()).collect()
    }

    /// The reason of every cancelled row (UR-30): the stop's or the loss's, whichever ended the Run.
    fn cancelled_reasons(core: &Core) -> Vec<String> {
        lock(&core.rec).applied.iter().filter(|row| !row["cancelled"].is_null())
            .map(|row| row["cancelled"].as_str().unwrap().to_owned()).collect()
    }

    #[test]
    fn ur_24_a_booked_command_floors_a_later_one_inside_the_window() {
        // UR-24, design 38 D2 (1), series 1: a command booked at 7 ms for 9.5 ms is floored at
        // the 10 ms one booked at 0 ms — 10 ≤ 7 + the release window — and reported late; at
        // 7 ms both are inside the window and go to the device at their one instant.
        let (core, device, time, events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        let gain = key("radio.rx.c0.gain_db");
        control.book_timed(ActionId(1), gain.clone(), Value::num(1.0).unwrap(), at(10));
        time.advance_to(core.at(ms(7))).unwrap();
        let half = ms(9) + core.ticks(500_000);
        control.book_timed(ActionId(2), gain.clone(), Value::num(2.0).unwrap(), Some(AbsoluteDeadline::new(core.at(half))));
        let late = events_of(&events, kinds::LATE_COMMAND);
        assert_eq!(
            (late.len(), late[0].payload["key"].as_str(), late[0].payload["requested"]["ticks"].as_i64(), late[0].payload["applied"]["ticks"].as_i64()),
            (1, Some(gain.as_str()), Some(half), Some(ms(10))),
            "{late:?}",
        );
        control.release();
        assert!(control.held.is_empty(), "a command inside the window stayed held: {:?}", control.held.keys().collect::<Vec<_>>());
        assert_eq!(timed_gains(&device, "rx"), [(1.0, ms(10)), (2.0, ms(10))], "the device's order: {:?}", device.calls());
        time.advance_to(core.at(ms(9))).unwrap();
        control.release();
        assert!(rows_of(&core, &gain).is_empty(), "a row before the instant: {:?}", lock(&core.rec).applied);
        time.advance_to(core.at(ms(10))).unwrap();
        control.release();
        assert_eq!(rows_of(&core, &gain), [(ms(10), true, false), (ms(10), true, false)]);
    }

    #[test]
    fn ur_24_an_omitted_deadline_takes_the_floor() {
        // UR-24, design 38 D2 (1), series 1b: the floor, not `now + lead`, is an omitted `at`'s
        // instant.
        let (core, device, time, events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let gain = key("radio.rx.c0.gain_db");
        control.book_timed(ActionId(1), gain.clone(), Value::num(1.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(10)))));
        time.advance_to(core.at(ms(7))).unwrap();
        control.book_timed(ActionId(2), gain.clone(), Value::num(2.0).unwrap(), None);
        control.release();
        time.advance_to(core.at(ms(10))).unwrap();
        control.release();
        assert!(events_of(&events, kinds::LATE_COMMAND).is_empty(), "an omitted deadline is not late");
        assert_eq!(rows_of(&core, &gain), [(ms(10), true, false), (ms(10), true, false)]);
        assert_eq!(timed_gains(&device, "rx"), [(1.0, ms(10)), (2.0, ms(10))]);
    }

    #[test]
    fn ur_24_the_window_edge_excludes_one_tick() {
        // UR-24, design 38 D2 (1), series 2: a receipt one root tick (5 ns) before 7 ms leaves
        // the 10 ms command outside the 3 ms window, so 9.5 ms holds.
        let (core, device, time, events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let gain = key("radio.rx.c0.gain_db");
        control.book_timed(ActionId(1), gain.clone(), Value::num(1.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(10)))));
        time.advance_to(core.at(ms(7) - 1)).unwrap();
        let half = ms(9) + core.ticks(500_000);
        control.book_timed(ActionId(2), gain.clone(), Value::num(2.0).unwrap(), Some(AbsoluteDeadline::new(core.at(half))));
        assert!(events_of(&events, kinds::LATE_COMMAND).is_empty(), "a tick outside the window floored the command");
        control.release();
        time.advance_to(core.at(ms(10))).unwrap();
        control.release();
        assert_eq!(rows_of(&core, &gain), [(half, true, false), (ms(10), true, false)]);
        assert_eq!(timed_gains(&device, "rx"), [(2.0, half), (1.0, ms(10))]);
    }

    #[test]
    fn ur_24_a_command_beyond_the_window_keeps_its_instant() {
        // UR-24, design 38 D2 (1), series 3: a command due past the window is not floored by
        // the one inside it and goes to the device when the window reaches it.
        let (core, device, time, events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        let gain = key("radio.rx.c0.gain_db");
        control.book_timed(ActionId(1), gain.clone(), Value::num(1.0).unwrap(), at(10));
        time.advance_to(core.at(ms(7))).unwrap();
        control.book_timed(ActionId(2), gain.clone(), Value::num(2.0).unwrap(), at(11));
        control.release();
        assert_eq!(timed_gains(&device, "rx"), [(1.0, ms(10))], "at 7 ms only the 10 ms command is in the window");
        time.advance_to(core.at(ms(8))).unwrap();
        control.release();
        assert_eq!(timed_gains(&device, "rx"), [(1.0, ms(10)), (2.0, ms(11))], "at 8 ms the 11 ms command is in the window");
        assert!(events_of(&events, kinds::LATE_COMMAND).is_empty(), "a command beyond the window was made late");
        time.advance_to(core.at(ms(11))).unwrap();
        control.release();
        assert_eq!(rows_of(&core, &gain), [(ms(10), true, false), (ms(11), true, false)]);
    }

    #[test]
    fn ur_24_a_command_waiting_for_a_channel_floors_a_later_booking() {
        // UR-24, design 38 D2 (1), series 4: a channel-less head floors a later command whatever
        // the release can do with it; the head records `no channel` and the later one is given
        // behind it at the floored instant.
        let (core, device, time, events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[Dir::Tx]);
        control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        let (rx, tx) = (key("radio.rx.c0.gain_db"), key("radio.tx.c0.gain_db"));
        control.book_timed(ActionId(1), rx.clone(), Value::num(1.0).unwrap(), at(10));
        time.advance_to(core.at(ms(7))).unwrap();
        let half = ms(9) + core.ticks(500_000);
        control.book_timed(ActionId(2), tx.clone(), Value::num(2.0).unwrap(), Some(AbsoluteDeadline::new(core.at(half))));
        let late = events_of(&events, kinds::LATE_COMMAND);
        assert_eq!(
            (late.len(), late[0].payload["key"].as_str(), late[0].payload["requested"]["ticks"].as_i64(), late[0].payload["applied"]["ticks"].as_i64()),
            (1, Some(tx.as_str()), Some(half), Some(ms(10))),
            "the channel-less head did not floor the transmit booking: {late:?}",
        );
        control.release();
        time.advance_to(core.at(ms(10))).unwrap();
        control.release();
        assert_eq!((rows_of(&core, &rx), rows_of(&core, &tx)), (vec![(ms(10), false, true)], vec![(ms(10), true, false)]));
        assert_eq!(timed_gains(&device, "tx"), [(2.0, ms(10))]);
        assert!(timed_gains(&device, "rx").is_empty());
    }

    #[test]
    fn ur_24_a_refused_release_floors_the_next_booking() {
        // UR-24, design 38 D2 (1), series 5: a command the device refused floors a later one as
        // a given one does, and leaves no row; the next release is given.
        let inner = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..crate::device::FakeConfig::default() }));
        let fails = Arc::new(crate::provider::test_support::ApplyFails::timed(inner.clone(), Dir::Rx, None));
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(fails.clone(), vec![Arc::new(NoLink)], 2_000, BTreeMap::new());
        let _frozen = hold(&core, &time, &inner);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let gain = key("radio.rx.c0.gain_db");
        control.book_timed(ActionId(1), gain.clone(), Value::num(1.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(10)))));
        time.advance_to(core.at(ms(7))).unwrap();
        control.release();
        assert_eq!(fails.attempts(), [(Dir::Rx, 0, Some(ms(10)))], "A was not attempted at its instant");
        assert!(rows_of(&core, &gain).is_empty(), "a refused release wrote a row: {:?}", lock(&core.rec).applied);
        time.advance_to(core.at(ms(7) + core.ticks(200_000))).unwrap();
        let half = ms(9) + core.ticks(500_000);
        control.book_timed(ActionId(2), gain.clone(), Value::num(2.0).unwrap(), Some(AbsoluteDeadline::new(core.at(half))));
        let late = events_of(&events, kinds::LATE_COMMAND);
        assert_eq!(
            late.iter().map(|event| (event.payload["key"].as_str().unwrap().to_owned(), event.payload["requested"]["ticks"].as_i64().unwrap(), event.payload["applied"]["ticks"].as_i64().unwrap())).collect::<Vec<_>>(),
            [(gain.as_str().to_owned(), half, ms(10))],
            "the refused command did not floor B: {late:?}",
        );
        control.release();
        time.advance_to(core.at(ms(10))).unwrap();
        control.release();
        let claims: Vec<f64> = lock(&core.rec).applied.iter().filter(|row| row["key"] == gain.as_str()).map(|row| row["claimed"].as_f64().unwrap()).collect();
        assert_eq!((claims, rows_of(&core, &gain)), (vec![2.0], vec![(ms(10), true, false)]), "B did not take A's instant or A left a row: {:?}", lock(&core.rec).applied);
        assert_eq!(timed_gains(&inner, "rx"), [(2.0, ms(10))]);
    }

    #[test]
    fn ur_24_a_refused_release_holds_its_queue_slot_until_its_instant() {
        // UR-24, design 38 D2 (2), series 5b: a refused command keeps its queue slot until its
        // instant; a booking then is late first and full second, and one after the instant is
        // taken.
        let inner = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..crate::device::FakeConfig::default() }));
        let fails = Arc::new(crate::provider::test_support::ApplyFails::timed(inner.clone(), Dir::Rx, None));
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(fails.clone(), vec![Arc::new(NoLink)], 2_000, BTreeMap::new());
        let _frozen = hold(&core, &time, &inner);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let gain = key("radio.rx.c0.gain_db");
        control.book_timed(ActionId(1), gain.clone(), Value::num(1.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(10)))));
        for i in 0..15i64 {
            control.book_timed(ActionId(2 + i as u64), gain.clone(), Value::num(2.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(20 + i)))));
        }
        time.advance_to(core.at(ms(7))).unwrap();
        control.release();
        assert_eq!(fails.attempts(), [(Dir::Rx, 0, Some(ms(10)))]);
        time.advance_to(core.at(ms(7) + core.ticks(200_000))).unwrap();
        let half = ms(9) + core.ticks(500_000);
        control.book_timed(ActionId(20), gain.clone(), Value::num(3.0).unwrap(), Some(AbsoluteDeadline::new(core.at(half))));
        let seen: Vec<_> = events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter()
            .filter(|event| event.kind.as_str() == kinds::LATE_COMMAND || event.kind.as_str() == kinds::COMMAND_QUEUE_FULL).collect();
        let order: Vec<&str> = seen.iter().map(|event| event.kind.as_str()).collect();
        assert_eq!(order, [kinds::LATE_COMMAND, kinds::COMMAND_QUEUE_FULL], "the refused command freed its slot: {seen:?}");
        assert_eq!(
            (seen[0].payload["key"].as_str(), seen[0].payload["requested"]["ticks"].as_i64(), seen[0].payload["applied"]["ticks"].as_i64(), seen[1].payload["depth"].as_i64()),
            (Some(gain.as_str()), Some(half), Some(ms(10)), Some(16)),
        );
        assert_eq!(control.held.len(), 15, "the late-and-full command took a slot: {:?}", control.held.keys().collect::<Vec<_>>());
        time.advance_to(core.at(ms(10))).unwrap();
        control.release();
        control.book_timed(ActionId(21), gain.clone(), Value::num(4.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(13)))));
        assert_eq!(control.held.len(), 16, "the slot the refused command took was not free after its instant: {:?}", control.held.keys().collect::<Vec<_>>());
    }

    #[test]
    fn ur_24_a_channel_leaving_after_the_release_does_not_move_the_floor() {
        // UR-24, design 38 D2 (1), series 6: the floor is the booking's history: the channel
        // leaving at 7.2 ms leaves the 10 ms command's floor for a 7.3 ms booking.
        let (core, device, time, events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        let gain = key("radio.rx.c0.gain_db");
        control.book_timed(ActionId(1), gain.clone(), Value::num(1.0).unwrap(), at(10));
        time.advance_to(core.at(ms(7))).unwrap();
        control.release();
        assert_eq!(timed_gains(&device, "rx"), [(1.0, ms(10))], "A was not given before the channel left");
        time.advance_to(core.at(ms(7) + core.ticks(200_000))).unwrap();
        control.book_cold(ActionId(2), key("radio.rx.c0.stream"), Value::from(-1), Some(AbsoluteDeadline::new(core.at(core.now()))));
        time.advance_to(core.at(ms(7) + core.ticks(300_000))).unwrap();
        let half = ms(9) + core.ticks(500_000);
        control.book_timed(ActionId(3), gain.clone(), Value::num(2.0).unwrap(), Some(AbsoluteDeadline::new(core.at(half))));
        let late = events_of(&events, kinds::LATE_COMMAND);
        assert_eq!(
            late.iter().map(|event| (event.payload["key"].as_str().unwrap().to_owned(), event.payload["requested"]["ticks"].as_i64().unwrap(), event.payload["applied"]["ticks"].as_i64().unwrap())).collect::<Vec<_>>(),
            [(gain.as_str().to_owned(), half, ms(10))],
            "the channel's leaving moved the floor: {late:?}",
        );
        control.release();
        time.advance_to(core.at(ms(10))).unwrap();
        control.release();
        assert_eq!((rows_of(&core, &gain), timed_gains(&device, "rx")), (vec![(ms(10), true, false), (ms(10), false, true)], vec![(1.0, ms(10))]));
    }

    #[test]
    fn ur_26_a_stop_leaves_each_update_its_row_at_its_instant() {
        // UR-24, UR-26, design 38 D2 (4), series 7: an Action's `Stop` cancels no update; each
        // is given and recorded at its own instant.
        let (core, device, time, events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        let gain = key("radio.rx.c0.gain_db");
        control.book_timed(ActionId(1), gain.clone(), Value::num(1.0).unwrap(), at(10));
        control.book_timed(ActionId(2), gain.clone(), Value::num(2.0).unwrap(), at(12));
        time.advance_to(core.at(ms(8))).unwrap();
        control.book(ezsdr_kernel::module_api::Dispatched::new(ActionId(3), Action::Stop { target: Some(super::super::test_support::target_of(&core.id)) }, Some(core.id.clone())));
        assert!(events_of(&events, kinds::COMMAND_REJECTED).is_empty());
        time.advance_to(core.at(ms(10))).unwrap();
        control.release();
        time.advance_to(core.at(ms(12))).unwrap();
        control.release();
        assert_eq!(rows_of(&core, &gain), [(ms(10), true, false), (ms(12), true, false)]);
        assert!(events_of(&events, kinds::LATE_COMMAND).is_empty());
        assert_eq!(timed_gains(&device, "rx"), [(1.0, ms(10)), (2.0, ms(12))]);
    }

    #[test]
    fn ur_26_provider_stop_records_a_given_and_a_held_update_alike() {
        // UR-26, UR-30, design 38 D2 (4), series 8: a given and a held command due after the
        // stop are both cancelled at the stop's instant; a loss at that instant reads the same.
        for loss in [false, true] {
            let (core, device, time, _events) = manual_rig_events();
            let _frozen = hold(&core, &time, &device);
            let (mut control, _tx) = control(&core, &[Dir::Rx]);
            let ms = |n: i64| core.ticks(n * 1_000_000);
            let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
            let gain = key("radio.rx.c0.gain_db");
            control.book_timed(ActionId(1), gain.clone(), Value::num(1.0).unwrap(), at(10));
            time.advance_to(core.at(ms(7))).unwrap();
            control.release();
            control.book_timed(ActionId(2), gain.clone(), Value::num(2.0).unwrap(), at(12));
            time.advance_to(core.at(ms(8))).unwrap();
            if loss {
                core.device_lost("the test's loss");
            } else {
                stop_end(&core, ms(8));
            }
            time.advance_to(core.at(ms(10) + core.ticks(600_000))).unwrap();
            control.finish();
            assert_eq!(rows_of(&core, &gain), [(ms(10), false, false), (ms(12), false, false)], "loss={loss}: {:?}", lock(&core.rec).applied);
            assert_eq!(cancelled_at(&core), [ms(8), ms(8)], "loss={loss}: the end's instant is not the stop's");
            assert_eq!(timed_gains(&device, "rx"), [(1.0, ms(10))], "loss={loss}: a device call after the end");
        }
    }

    #[test]
    fn ur_26_an_applied_update_keeps_its_row_after_a_later_stop() {
        // Design 38 D2 (4), series 9: an update whose instant came before the stop keeps its
        // issued row.
        let (core, device, time, _events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let gain = key("radio.rx.c0.gain_db");
        control.book_timed(ActionId(1), gain.clone(), Value::num(1.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(10)))));
        time.advance_to(core.at(ms(7))).unwrap();
        control.release();
        time.advance_to(core.at(ms(10))).unwrap();
        control.release();
        time.advance_to(core.at(ms(12))).unwrap();
        stop_end(&core, ms(12));
        control.finish();
        assert_eq!(rows_of(&core, &gain), [(ms(10), true, false)]);
        assert!(cancelled_at(&core).is_empty());
    }

    #[test]
    fn ur_26_a_loss_settles_what_the_last_release_gave() {
        // UR-26, design 38 D2 (4), series 9b: the loss's instant, not the finish's, settles a
        // command given before it: its row is issued.
        let (core, device, time, _events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let gain = key("radio.rx.c0.gain_db");
        control.book_timed(ActionId(1), gain.clone(), Value::num(1.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(10)))));
        time.advance_to(core.at(ms(7))).unwrap();
        control.release();
        time.advance_to(core.at(ms(9) + core.ticks(600_000))).unwrap();
        control.release();
        assert!(rows_of(&core, &gain).is_empty(), "a row before the instant");
        time.advance_to(core.at(ms(10) + core.ticks(200_000))).unwrap();
        core.device_lost("the test's loss");
        time.advance_to(core.at(ms(10) + core.ticks(600_000))).unwrap();
        control.finish();
        assert_eq!(rows_of(&core, &gain), [(ms(10), true, false)], "{:?}", lock(&core.rec).applied);
        assert_eq!(timed_gains(&device, "rx"), [(1.0, ms(10))]);
    }

    #[test]
    fn ur_26_a_stop_settles_by_its_instant_not_by_the_finish() {
        // UR-26, design 38 D2 (4), series 9b: a command due after the stop's instant is held
        // until the finish at 10.6 ms and still settled by the stop's 9.9 ms.
        let (core, _device, time, _events) = manual_rig_events();
        let _frozen = hold(&core, &time, &_device);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let gain = key("radio.rx.c0.gain_db");
        control.book_timed(ActionId(1), gain.clone(), Value::num(1.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(10)))));
        time.advance_to(core.at(ms(7))).unwrap();
        control.release();
        time.advance_to(core.at(ms(9) + core.ticks(600_000))).unwrap();
        control.release();
        time.advance_to(core.at(ms(9) + core.ticks(900_000))).unwrap();
        stop_end(&core, ms(9) + core.ticks(900_000));
        time.advance_to(core.at(ms(10) + core.ticks(600_000))).unwrap();
        control.finish();
        assert_eq!(rows_of(&core, &gain), [(ms(10), false, false)], "{:?}", lock(&core.rec).applied);
        assert_eq!(cancelled_at(&core), [ms(9) + core.ticks(900_000)], "the finish's instant settled it, not the stop's");
    }

    #[test]
    fn ur_26_a_release_after_the_stop_was_published_leaves_the_row_to_the_end() {
        // UR-26, design 38 D2 (4): uhd-control passed its loop's stop check and the stop (9.9 ms
        // or 10.2 ms) is booked while a release is still to run at 10.6 ms. The command it had
        // given at 7 ms stays for the end, which settles it at `S`: cancelled when its instant is
        // after `S`, and its issued row alone when it is due by `S`.
        for (stop, cancelled) in [((9, 900_000), true), ((10, 200_000), false)] {
            let (core, device, time, _events) = manual_rig_events();
            let _frozen = hold(&core, &time, &device);
            let (mut control, _tx) = control(&core, &[Dir::Rx]);
            let ms = |n: i64| core.ticks(n * 1_000_000);
            let gain = key("radio.rx.c0.gain_db");
            let at = ms(stop.0) + core.ticks(stop.1);
            control.book_timed(ActionId(1), gain.clone(), Value::num(1.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(10)))));
            time.advance_to(core.at(ms(7))).unwrap();
            control.release();
            assert_eq!(timed_gains(&device, "rx"), [(1.0, ms(10))], "the command was not given at 7 ms");
            time.advance_to(core.at(at)).unwrap();
            stop_end(&core, at);
            time.advance_to(core.at(ms(10) + core.ticks(600_000))).unwrap();
            control.release();
            assert!(rows_of(&core, &gain).is_empty(), "the release after the end was booked wrote the row: {:?}", lock(&core.rec).applied);
            control.finish();
            if cancelled {
                assert_eq!(rows_of(&core, &gain), [(ms(10), false, false)], "the given command was issued after `S` or not cancelled: {:?}", lock(&core.rec).applied);
                assert_eq!(cancelled_at(&core), [at], "the end's instant settled it, not the finish's");
            } else {
                assert_eq!(rows_of(&core, &gain), [(ms(10), true, false)], "the end wrote the issued row twice or not at all: {:?}", lock(&core.rec).applied);
                assert!(cancelled_at(&core).is_empty(), "a command due by `S` was cancelled: {:?}", lock(&core.rec).applied);
            }
        }
    }

    #[test]
    fn ur_26_a_command_given_after_the_stop_s_instant_is_cancelled_at_it() {
        // UR-26, RM-16's Ceiling, design 38 D2 (4): a pass that runs at 10.6 ms, after the stop
        // booked at 10.2 ms, hands the command due at 10 ms to the device then. At `S` it had not
        // been handed over, so it is the end's and is cancelled at `S`, not issued.
        let (core, device, time, _events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let gain = key("radio.rx.c0.gain_db");
        control.book_timed(ActionId(1), gain.clone(), Value::num(1.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(10)))));
        time.advance_to(core.at(ms(10) + core.ticks(200_000))).unwrap();
        stop_end(&core, ms(10) + core.ticks(200_000));
        time.advance_to(core.at(ms(10) + core.ticks(600_000))).unwrap();
        control.release();
        assert_eq!(timed_gains(&device, "rx"), [(1.0, ms(10))], "the pass did not hand the command to the device after `S`");
        assert!(rows_of(&core, &gain).is_empty(), "{:?}", lock(&core.rec).applied);
        control.finish();
        assert_eq!(rows_of(&core, &gain), [(ms(10), false, false)], "a command not handed over by `S` was issued: {:?}", lock(&core.rec).applied);
        assert_eq!(cancelled_at(&core), [ms(10) + core.ticks(200_000)], "the end's instant settled it, not the pass's");
        assert_eq!(cancelled_reasons(&core), ["Provider::stop".to_owned()]);
    }

    #[test]
    fn ur_26_a_stop_between_two_hand_overs_judges_each_at_its_apply() {
        // UR-26, RM-16's Ceiling: a command is handed over by `S` when the device's time read
        // just before its apply is by `S` — at `S` for a stop. One pass at 9.9 ms hands A, and
        // the probe publishes the stop before B's apply: a cycle that handed B at `S` itself
        // keeps its issued row, one that handed it after `S` is cancelled there.
        for (gap, issued) in [(200_000, true), (300_000, false)] {
            let (core, device, time, _events) = manual_rig_events();
            let _frozen = hold(&core, &time, &device);
            let (mut control, _tx) = control(&core, &[Dir::Rx]);
            let ms = |n: i64| core.ticks(n * 1_000_000);
            let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
            let gain = key("radio.rx.c0.gain_db");
            control.book_timed(ActionId(1), gain.clone(), Value::num(1.0).unwrap(), at(10));
            control.book_timed(ActionId(2), gain.clone(), Value::num(2.0).unwrap(), at(10));
            let stop_at = ms(10) + core.ticks(200_000);
            let second = ms(10) + core.ticks(gap);
            let applies = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let (core_, time_, applies_) = (core.clone(), time.clone(), applies.clone());
            crate::provider::test_support::probe(&core, "before_timed_apply", move |_| {
                if applies_.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1 {
                    time_.advance_to(core_.at(second)).expect("the clock reaches the second apply");
                    stop_end(&core_, stop_at);
                }
            });
            time.advance_to(core.at(ms(9) + core.ticks(900_000))).unwrap();
            control.release();
            assert_eq!(timed_gains(&device, "rx"), [(1.0, ms(10)), (2.0, ms(10))], "gap={gap}: a command was not handed over");
            assert!(rows_of(&core, &gain).is_empty(), "gap={gap}: a row after the end was booked: {:?}", lock(&core.rec).applied);
            control.finish();
            if issued {
                assert_eq!(rows_of(&core, &gain), [(ms(10), true, false), (ms(10), true, false)], "gap={gap}: a command handed at `S` was not issued: {:?}", lock(&core.rec).applied);
                assert!(cancelled_at(&core).is_empty(), "gap={gap}: a command handed by `S` was cancelled: {:?}", lock(&core.rec).applied);
            } else {
                assert_eq!(rows_of(&core, &gain), [(ms(10), true, false), (ms(10), false, false)], "gap={gap}: a command handed after `S` was issued: {:?}", lock(&core.rec).applied);
                assert_eq!(cancelled_at(&core), [stop_at], "gap={gap}: the end's instant settled the second command");
            }
        }
    }

    #[test]
    fn ur_26_a_refusal_behind_a_handed_command_still_settles_it_at_the_stop() {
        // UR-26, RM-16's Ceiling: a release at 10.6 ms hands A after the stop at 10.2 ms, then
        // the device refuses B and the release returns at once. A's row must still be the
        // end's: it was handed after `S`, so it is cancelled there, and B has no row.
        let inner = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..crate::device::FakeConfig::default() }));
        let fails = Arc::new(crate::provider::test_support::ApplyFails::timed(inner.clone(), Dir::Tx, None));
        let (core, time, _events) = crate::provider::test_support::rig_on_inputs(fails.clone(), vec![Arc::new(NoLink)], 2_000, BTreeMap::new());
        let _frozen = hold(&core, &time, &inner);
        let (mut control, _tx) = control(&core, &[Dir::Rx, Dir::Tx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let (a, b) = (key("radio.rx.c0.gain_db"), key("radio.tx.c0.gain_db"));
        control.book_timed(ActionId(1), a.clone(), Value::num(1.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(10)))));
        control.book_timed(ActionId(2), b.clone(), Value::num(2.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(10)))));
        let stop_at = ms(10) + core.ticks(200_000);
        time.advance_to(core.at(stop_at)).unwrap();
        stop_end(&core, stop_at);
        time.advance_to(core.at(ms(10) + core.ticks(600_000))).unwrap();
        control.release();
        assert_eq!(fails.attempts(), [(Dir::Rx, 0, Some(ms(10))), (Dir::Tx, 0, Some(ms(10)))], "the release did not reach the second command");
        assert_eq!(timed_gains(&inner, "rx"), [(1.0, ms(10))], "A was not handed over");
        assert!(rows_of(&core, &a).is_empty() && rows_of(&core, &b).is_empty(), "a row before the end: {:?}", lock(&core.rec).applied);
        control.finish();
        assert_eq!(rows_of(&core, &a), [(ms(10), false, false)], "A was issued after `S`: {:?}", lock(&core.rec).applied);
        assert_eq!(rows_of(&core, &b), [], "the refused command left a row: {:?}", lock(&core.rec).applied);
        assert_eq!(cancelled_at(&core), [stop_at], "the end's instant settled A");
        assert_eq!(cancelled_reasons(&core), ["Provider::stop".to_owned()]);
    }

    #[test]
    fn ur_26_a_lost_apply_behind_a_handed_command_settles_both_at_the_loss() {
        // UR-26, RM-16's Ceiling: a release at 10.6 ms hands A, then B's apply reports the
        // device lost at that instant. A's call began at the loss's own `S`, so it was not
        // handed by it: A is cancelled there, and B stays held and is cancelled there too.
        let inner = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..crate::device::FakeConfig::default() }));
        let fails = Arc::new(crate::provider::test_support::ApplyFails::timed_lost(inner.clone(), Dir::Tx, None));
        let (core, time, _events) = crate::provider::test_support::rig_on_inputs(fails.clone(), vec![Arc::new(NoLink)], 2_000, BTreeMap::new());
        let _frozen = hold(&core, &time, &inner);
        let (mut control, _tx) = control(&core, &[Dir::Rx, Dir::Tx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let (a, b) = (key("radio.rx.c0.gain_db"), key("radio.tx.c0.gain_db"));
        control.book_timed(ActionId(1), a.clone(), Value::num(1.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(10)))));
        control.book_timed(ActionId(2), b.clone(), Value::num(2.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(10)))));
        let loss_at = ms(10) + core.ticks(600_000);
        time.advance_to(core.at(loss_at)).unwrap();
        control.release();
        assert!(core.is_lost(), "the loss was not published");
        assert_eq!(timed_gains(&inner, "rx"), [(1.0, ms(10))], "A was not handed over");
        assert!(rows_of(&core, &a).is_empty() && rows_of(&core, &b).is_empty(), "a row before the end: {:?}", lock(&core.rec).applied);
        control.finish();
        assert_eq!(rows_of(&core, &a), [(ms(10), false, false)], "A was issued: {:?}", lock(&core.rec).applied);
        assert_eq!(rows_of(&core, &b), [(ms(10), false, false)], "B was not settled as held: {:?}", lock(&core.rec).applied);
        assert_eq!(cancelled_at(&core), [loss_at, loss_at], "the loss's instant settled both");
        assert_eq!(cancelled_reasons(&core), ["UR-29: device lost".to_owned(), "UR-29: device lost".to_owned()]);
    }

    #[test]
    fn ur_26_an_instant_at_the_stop_applies_and_one_at_the_loss_does_not() {
        // UR-26, design 38 D2 (4), series 9c: at one instant the stop settles what is due there
        // first, and the loss itself first.
        for loss in [false, true] {
            let (core, device, time, _events) = manual_rig_events();
            let _frozen = hold(&core, &time, &device);
            let (mut control, _tx) = control(&core, &[Dir::Rx]);
            let ms = |n: i64| core.ticks(n * 1_000_000);
            let gain = key("radio.rx.c0.gain_db");
            control.book_timed(ActionId(1), gain.clone(), Value::num(1.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(10)))));
            time.advance_to(core.at(ms(7))).unwrap();
            control.release();
            time.advance_to(core.at(ms(10))).unwrap();
            if loss {
                core.device_lost("the test's loss");
            } else {
                stop_end(&core, ms(10));
            }
            time.advance_to(core.at(ms(10) + core.ticks(600_000))).unwrap();
            control.finish();
            let expected = if loss { vec![(ms(10), false, false)] } else { vec![(ms(10), true, false)] };
            assert_eq!(rows_of(&core, &gain), expected, "loss={loss}: {:?}", lock(&core.rec).applied);
            assert_eq!(cancelled_at(&core), if loss { vec![ms(10)] } else { Vec::new() }, "loss={loss}");
            assert_eq!(timed_gains(&device, "rx"), [(1.0, ms(10))]);
        }
    }

    #[test]
    fn ur_24_a_due_command_behind_a_channel_less_head_is_cancelled_at_the_loss() {
        // Design 38 D2 (4), series 9d: with the release stopped at a channel-less head, a live
        // command due at the same instant before the loss is not given; the end settles the head
        // `no channel` and the other cancelled, in that order. A stop at that instant reads the
        // same.
        for loss in [false, true] {
            let (core, device, time, _events) = manual_rig_events();
            let _frozen = hold(&core, &time, &device);
            let (mut control, _tx) = control(&core, &[Dir::Tx]);
            control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
            let ms = |n: i64| core.ticks(n * 1_000_000);
            let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
            let (rx, tx) = (key("radio.rx.c0.gain_db"), key("radio.tx.c0.gain_db"));
            control.book_timed(ActionId(1), rx.clone(), Value::num(1.0).unwrap(), at(10));
            time.advance_to(core.at(ms(7))).unwrap();
            control.book_timed(ActionId(2), tx.clone(), Value::num(2.0).unwrap(), at(10));
            control.release();
            time.advance_to(core.at(ms(9) + core.ticks(600_000))).unwrap();
            control.release();
            assert_eq!((control.held.len(), timed_gains(&device, "tx").len()), (2, 0), "loss={loss}: a command behind the head was given");
            time.advance_to(core.at(ms(10) + core.ticks(200_000))).unwrap();
            if loss {
                core.device_lost("the test's loss");
            } else {
                stop_end(&core, ms(10) + core.ticks(200_000));
            }
            time.advance_to(core.at(ms(10) + core.ticks(600_000))).unwrap();
            control.finish();
            let rows: Vec<(i64, bool, bool)> = lock(&core.rec).applied.iter()
                .filter(|row| row["key"] == rx.as_str() || row["key"] == tx.as_str())
                .map(|row| (row["at"]["ticks"].as_i64().unwrap(), row["issued"] == true, row["note"] == "no channel"))
                .collect();
            assert_eq!(rows, [(ms(10), false, true), (ms(10), false, false)], "loss={loss}: the end's rows or their order: {:?}", lock(&core.rec).applied);
            assert!(timed_gains(&device, "tx").is_empty(), "loss={loss}: the Ceiling command was given");
        }
    }

    #[test]
    fn ur_24_two_updates_at_one_instant_write_their_rows_in_booking_order() {
        // UR-24, design 38 D2 (3), series 10: two commands at one instant are given and recorded
        // in booking order, and the later value is the one in force.
        let (core, device, time, _events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        let gain = key("radio.rx.c0.gain_db");
        control.book_timed(ActionId(1), gain.clone(), Value::num(1.0).unwrap(), at(10));
        time.advance_to(core.at(ms(7))).unwrap();
        control.book_timed(ActionId(2), gain.clone(), Value::num(2.0).unwrap(), at(10));
        control.release();
        time.advance_to(core.at(ms(10))).unwrap();
        control.release();
        assert_eq!(rows_of(&core, &gain), [(ms(10), true, false), (ms(10), true, false)]);
        let claims: Vec<f64> = lock(&core.rec).applied.iter().filter(|row| row["key"] == gain.as_str()).map(|row| row["claimed"].as_f64().unwrap()).collect();
        assert_eq!((claims, timed_gains(&device, "rx")), (vec![1.0, 2.0], vec![(1.0, ms(10)), (2.0, ms(10))]));
        assert_eq!(control.config_at(i64::MAX)[&gain], Value::num(2.0).unwrap());
    }

    #[test]
    fn ur_24_a_late_command_at_a_full_queue_reports_late_first() {
        // UR-24, design 38 D2 (1), (2), series 11: the late judgement precedes the queue's, and
        // a command that takes effect frees its slot.
        let (core, device, time, events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let gain = key("radio.rx.c0.gain_db");
        for i in 0..16i64 {
            control.book_timed(ActionId(1 + i as u64), gain.clone(), Value::num(1.0 + i as f64).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(5 + 4 * i)))));
        }
        control.book_timed(ActionId(20), gain.clone(), Value::num(20.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(1)))));
        let seen = events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        let order: Vec<&str> = seen.iter().map(|event| event.kind.as_str()).collect();
        assert_eq!(order, [kinds::LATE_COMMAND, kinds::COMMAND_QUEUE_FULL], "the late judgement did not precede the queue's: {seen:?}");
        assert_eq!(
            (seen[0].payload["key"].as_str(), seen[0].payload["requested"]["ticks"].as_i64(), seen[0].payload["applied"]["ticks"].as_i64(), seen[1].payload["depth"].as_i64()),
            (Some(gain.as_str()), Some(ms(1)), Some(ms(2)), Some(16)),
        );
        assert_eq!(control.held.len(), 16, "the late-and-full command took a slot");
        time.advance_to(core.at(ms(5))).unwrap();
        control.release();
        assert_eq!(control.held.len(), 15, "the applied command kept its slot");
        control.book_timed(ActionId(21), gain.clone(), Value::num(21.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(30)))));
        assert_eq!(control.held.len(), 16, "the applied command's slot was not free");
    }

    #[test]
    fn ur_24_the_row_is_written_at_its_instant() {
        // UR-24, design 38 D2 (3): a given command has no row before its instant, and its device
        // call is made when the window reaches it.
        let (core, device, time, _events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let gain = key("radio.rx.c0.gain_db");
        control.book_timed(ActionId(1), gain.clone(), Value::num(1.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(10)))));
        time.advance_to(core.at(ms(7))).unwrap();
        control.release();
        assert_eq!(timed_gains(&device, "rx"), [(1.0, ms(10))], "the command was not given inside the window");
        time.advance_to(core.at(ms(10) - 1)).unwrap();
        control.release();
        assert!(rows_of(&core, &gain).is_empty(), "a row before the instant: {:?}", lock(&core.rec).applied);
        time.advance_to(core.at(ms(10))).unwrap();
        control.release();
        assert_eq!(rows_of(&core, &gain), [(ms(10), true, false)]);
    }

    #[test]
    fn ur_24_the_release_window_comes_from_the_description() {
        // Design 38 D1, S5: the floor and the release both read the description's window. At 10
        // ms, the 12 ms command is given at 2 ms and not one tick earlier, and it floors a 9.5 ms
        // booking at 7 ms; at 3 ms (the constant) neither holds.
        let device = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..crate::device::FakeConfig::default() }));
        let mut description = crate::provider::test_support::default_description(2_000);
        description.timing.release_window_ns = 10_000_000;
        let (core, time, events) = crate::provider::test_support::rig_on_description(device.clone(), vec![Arc::new(NoLink)], BTreeMap::new(), description);
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        let gain = key("radio.rx.c0.gain_db");
        control.book_timed(ActionId(1), gain.clone(), Value::num(1.0).unwrap(), at(12));
        time.advance_to(core.at(ms(2) - 1)).unwrap();
        control.release();
        assert!(timed_gains(&device, "rx").is_empty(), "a tick before the window's edge released the command");
        time.advance_to(core.at(ms(2))).unwrap();
        control.release();
        assert_eq!(timed_gains(&device, "rx"), [(1.0, ms(12))], "the window did not come from the description");
        time.advance_to(core.at(ms(7))).unwrap();
        let half = ms(9) + core.ticks(500_000);
        control.book_timed(ActionId(2), gain.clone(), Value::num(2.0).unwrap(), Some(AbsoluteDeadline::new(core.at(half))));
        let late = events_of(&events, kinds::LATE_COMMAND);
        assert_eq!(
            late.iter().map(|event| (event.payload["key"].as_str().unwrap().to_owned(), event.payload["requested"]["ticks"].as_i64().unwrap(), event.payload["applied"]["ticks"].as_i64().unwrap())).collect::<Vec<_>>(),
            [(gain.as_str().to_owned(), half, ms(12))],
            "the description's window did not floor B: {late:?}",
        );
        control.release();
        time.advance_to(core.at(ms(12) - 1)).unwrap();
        control.release();
        assert!(rows_of(&core, &gain).is_empty(), "a row before the instant: {:?}", lock(&core.rec).applied);
        time.advance_to(core.at(ms(12))).unwrap();
        control.release();
        assert_eq!(rows_of(&core, &gain), [(ms(12), true, false), (ms(12), true, false)]);
    }

    #[test]
    fn measurement_candidate_directional_start_leads_set_cold_change_origins() {
        for (dir, lead_ns, field) in [
            (Dir::Rx, 333_000_000, "radio.rx.s0.sample_rate_hz"),
            (Dir::Tx, 277_000_000, "radio.tx.s0.sample_rate_hz"),
        ] {
            let device = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..crate::device::FakeConfig::default() }));
            let mut description = layout_description();
            match dir {
                Dir::Rx => description.timing.rx_start_lead_ns = lead_ns,
                Dir::Tx => description.timing.tx_start_lead_ns = lead_ns,
            }
            let (core, _time, _events) = crate::provider::test_support::rig_on_description(device, vec![Arc::new(NoLink)], BTreeMap::new(), description);
            let (mut control, _tx) = control(&core, &[dir]);
            let requested = core.ticks(100_000_000);
            control.book_cold(ActionId(1), key(field), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(core.at(requested))));
            let planned = plan_of(&core, dir);
            assert_eq!(planned.len(), 2, "{dir:?}: expected the original and changed segments");
            let first = &planned[0].segment;
            let cut = first.instant(first.cut.expect("the old segment is cut")).unwrap();
            let rx_ready = if dir == Dir::Rx {
                core.ticks(crate::profile::DELIVERY_ALLOWANCE_NS)
                    + core.block_len.max(core.device.rx_packet_samples(0)) as i64 * first.config.ratio.num() as i64
            } else { 0 };
            assert_eq!(planned[1].segment.origin, cut + rx_ready + core.ticks(lead_ns), "{dir:?}: the named cold rate change must read its directional candidate lead");
        }
    }

    #[test]
    fn ur_26_a_loss_then_a_stop_keeps_the_loss_s_instant_and_rows() {
        // Design 38 D2 (4), S6: the first end's instant, reason and rows stand; a later stop adds
        // no row and publishes no second loss.
        let (core, device, time, events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let gain = key("radio.rx.c0.gain_db");
        control.book_timed(ActionId(1), gain.clone(), Value::num(1.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(12)))));
        time.advance_to(core.at(ms(8))).unwrap();
        core.device_lost("the test's loss");
        time.advance_to(core.at(ms(10) + core.ticks(600_000))).unwrap();
        control.finish();
        let rows = rows_of(&core, &gain);
        assert_eq!((rows.clone(), cancelled_at(&core), cancelled_reasons(&core)), (vec![(ms(12), false, false)], vec![ms(8)], vec!["UR-29: device lost".to_owned()]));
        assert_eq!(events_of(&events, ezsdr_kernel::event::EventKind::DEVICE_LOST).len(), 1);
        // A second notification of the same loss moves nothing and publishes nothing.
        core.device_lost("the test's loss again");
        control.finish();
        assert_eq!((rows_of(&core, &gain), cancelled_at(&core), cancelled_reasons(&core)), (rows.clone(), vec![ms(8)], vec!["UR-29: device lost".to_owned()]), "the second notification moved the end");
        assert!(events_of(&events, ezsdr_kernel::event::EventKind::DEVICE_LOST).is_empty(), "the second notification published the loss again");
        stop_end(&core, ms(10) + core.ticks(600_000));
        control.finish();
        control.finish();
        assert_eq!((rows_of(&core, &gain), cancelled_at(&core), cancelled_reasons(&core)), (rows, vec![ms(8)], vec!["UR-29: device lost".to_owned()]), "the stop recorded the loss again");
        assert_eq!(events_of(&events, ezsdr_kernel::event::EventKind::DEVICE_LOST).len(), 0, "the second finish published the loss again");
    }

    #[test]
    fn ur_26_a_stop_then_a_loss_keeps_the_stop_s_instant_and_rows() {
        // Design 38 D2 (4), S6: with the stop first, a later loss moves no instant and adds no
        // row, its reason stays the stop's, and a second finish publishes no second loss.
        let (core, device, time, events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let gain = key("radio.rx.c0.gain_db");
        control.book_timed(ActionId(1), gain.clone(), Value::num(1.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(12)))));
        time.advance_to(core.at(ms(8))).unwrap();
        stop_end(&core, ms(8));
        time.advance_to(core.at(ms(10) + core.ticks(600_000))).unwrap();
        control.finish();
        let rows = rows_of(&core, &gain);
        assert_eq!((rows.clone(), cancelled_at(&core), cancelled_reasons(&core)), (vec![(ms(12), false, false)], vec![ms(8)], vec!["Provider::stop".to_owned()]));
        core.device_lost("the test's loss");
        assert_eq!(events_of(&events, ezsdr_kernel::event::EventKind::DEVICE_LOST).len(), 1);
        control.finish();
        control.finish();
        assert_eq!((rows_of(&core, &gain), cancelled_at(&core), cancelled_reasons(&core)), (rows, vec![ms(8)], vec!["Provider::stop".to_owned()]), "the loss after the stop wrote another row");
        assert_eq!(events_of(&events, ezsdr_kernel::event::EventKind::DEVICE_LOST).len(), 0, "a later finish published the loss again");
    }

    #[test]
    fn ur_24_a_command_the_loss_stops_from_being_given_settles_at_the_end() {
        // Design 38 D2 (4), S3, series 8b: a command the release took up but did not hand over
        // because another thread published the loss stays pending and is cancelled at `S`,
        // whether its instant is after, at or before it — never `issued`.
        for (label, release_at, instant) in [("after", 7, 10), ("at", 10, 10), ("before", 11, 10)] {
            let (core, device, time, _events) = manual_rig_events();
            let _frozen = hold(&core, &time, &device);
            let (mut control, _tx) = control(&core, &[Dir::Rx]);
            let ms = |n: i64| core.ticks(n * 1_000_000);
            let gain = key("radio.rx.c0.gain_db");
            control.book_timed(ActionId(1), gain.clone(), Value::num(1.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(instant)))));
            time.advance_to(core.at(ms(release_at))).unwrap();
            let core_ = core.clone();
            crate::provider::test_support::probe(&core, "before_timed_apply", move |_| core_.device_lost("the test's loss"));
            control.release();
            assert!(core.is_lost(), "{label}");
            assert_eq!(timed_gains(&device, "rx"), [], "{label}: the lost device was called");
            time.advance_to(core.at(ms(release_at) + core.ticks(600_000))).unwrap();
            control.finish();
            assert_eq!(rows_of(&core, &gain), [(ms(instant), false, false)], "{label}: {:?}", lock(&core.rec).applied);
            assert_eq!(cancelled_at(&core), [ms(release_at)], "{label}: the end's instant");
        }
    }

    #[test]
    fn ur_24_a_command_due_before_the_receipt_takes_no_slot() {
        // Design 38 D2 (2), S4, series 11b: at a receipt of 20 ms the sixteen commands booked
        // for 2…17 ms have met their instant in the history and take no slot.
        let (core, device, time, events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let gain = key("radio.rx.c0.gain_db");
        for i in 1..=16i64 {
            control.book_timed(ActionId(i as u64), gain.clone(), Value::num(1.0 + i as f64).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(1 + i)))));
        }
        time.advance_to(core.at(ms(20))).unwrap();
        control.book_timed(ActionId(17), gain.clone(), Value::num(20.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(22)))));
        assert!(events_of(&events, kinds::COMMAND_QUEUE_FULL).is_empty(), "the met instants were counted");
        assert_eq!(control.held.len(), 17, "{:?}", control.held.keys().collect::<Vec<_>>());
        control.release();
        time.advance_to(core.at(ms(22))).unwrap();
        control.release();
        assert!(rows_of(&core, &gain).iter().any(|(at, issued, _)| *at == ms(22) && *issued), "{:?}", lock(&core.rec).applied);
    }

    #[test]
    fn ur_24_a_command_due_at_its_receipt_takes_no_slot() {
        // Design 38 D2 (2), S4, series 11c: at a receipt equal to the earliest instant that
        // command has met it and takes no slot; one tick earlier it still does, and the queue is
        // full.
        let booked = |at_the_instant: bool| -> usize {
            let (core, device, time, events) = manual_rig_events();
            let _frozen = hold(&core, &time, &device);
            let (mut control, _tx) = control(&core, &[Dir::Rx]);
            let ms = |n: i64| core.ticks(n * 1_000_000);
            let gain = key("radio.rx.c0.gain_db");
            for i in 1..=16i64 {
                control.book_timed(ActionId(i as u64), gain.clone(), Value::num(1.0 + i as f64).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(1 + i)))));
            }
            time.advance_to(core.at(if at_the_instant { ms(2) } else { ms(2) - 1 })).unwrap();
            control.book_timed(ActionId(17), gain.clone(), Value::num(20.0).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(30)))));
            let full = events_of(&events, kinds::COMMAND_QUEUE_FULL).len();
            drop(_frozen);
            full
        };
        assert_eq!(booked(true), 0, "an instant met at the receipt took a slot");
        assert_eq!(booked(false), 1, "a command one tick short of its instant took no slot");
    }

    #[test]
    fn ur_29_a_loss_published_during_a_release_settles_the_channel_less_behind_it() {
        // UR-29, design 38 D2 (4), the loss's order against the release's own rows: a loss
        // published during the pass's first apply ends the Run first, and that command's call
        // began at the loss's own instant `S`, so RM-16's Ceiling gives it no issued row; a
        // channel-less command due at the loss's instant is the end's cancelled command, not
        // `no channel`.
        let spy = Arc::new(crate::provider::test_support::LockSpy::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..crate::device::FakeConfig::default() }));
        let (core, time, _events) = crate::provider::test_support::rig_on(spy.clone(), vec![Arc::new(NoLink)], 2_000);
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        let (live, gone) = (key("radio.rx.c0.gain_db"), key("radio.rx.c1.gain_db"));
        control.book_timed(ActionId(1), live.clone(), Value::num(1.0).unwrap(), at(9));
        control.book_timed(ActionId(2), gone.clone(), Value::num(2.0).unwrap(), at(10));
        time.advance_to(core.at(ms(10))).unwrap();
        let gate = spy.stop_after("apply");
        let core_ = core.clone();
        let publisher = core.device.host().spawn("loss", move || {
            gate.wait();
            core_.device_lost("the test's loss");
            gate.release();
        }).expect("a participant thread");
        control.release();
        publisher.join().expect("the publisher thread");
        assert_eq!(spy.seen("apply"), 1, "the lost device was called again: {:?}", spy.log());
        assert!(rows_of(&core, &live).is_empty() && rows_of(&core, &gone).is_empty(), "a row was written in the loss's pass: {:?}", lock(&core.rec).applied);
        control.finish();
        assert_eq!(rows_of(&core, &live), [(ms(9), false, false)], "the command whose call began at `S` was issued: {:?}", lock(&core.rec).applied);
        assert_eq!(rows_of(&core, &gone), [(ms(10), false, false)], "the channel-less command was not the end's: {:?}", lock(&core.rec).applied);
        assert_eq!((cancelled_at(&core), cancelled_reasons(&core)), (vec![ms(10), ms(10)], vec!["UR-29: device lost".to_owned(), "UR-29: device lost".to_owned()]));
    }

    // ------------------------------------------------------------ R1: a receive frequency and the DDC (UR-24, UR-33)

    /// A running receive segment's `uhd-control` on the manual rig, the receive stream
    /// `Stop`ped at the device's instant 0, so that no sample passes after it.
    fn stopped_receive() -> (Arc<Core>, Arc<crate::device::FakeDevice>, Arc<ezsdr_kernel::time::ManualTimeAuthority>, Arc<ezsdr_kernel::event::EventCollector>, Control, crate::provider::test_support::Frozen) {
        let (core, device, time, events) = manual_rig_events();
        let frozen = hold(&core, &time, &device);
        device.rx_open(0, &[0]).unwrap();
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        control.book(Dispatched::new(ActionId(1), Action::Stop { target: Some(super::super::test_support::target_of(&core.rx_id)) }, Some(core.rx_id.clone())));
        (core, device, time, events, control, frozen)
    }

    /// Books a timed update of `name` to `value` for `e_ms` and releases it in its window.
    fn timed_at(control: &mut Control, core: &Core, time: &ezsdr_kernel::time::ManualTimeAuthority, id: u64, name: &str, value: f64, e_ms: i64) {
        let ms = |n: i64| core.ticks(n * 1_000_000);
        time.advance_to(core.at(ms(e_ms) - core.ticks(4_000_000))).ok();
        control.book_timed(ActionId(id), key(name), Value::num(value).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(e_ms)))));
        time.advance_to(core.at(ms(e_ms) - core.ticks(500_000))).unwrap();
        control.release();
    }

    #[test]
    fn ur_24_a_stopped_receive_channel_is_given_no_timed_frequency() {
        // UR-24 (R1, series 1a): no sample passes the DDC of a stopped channel, so a timed
        // frequency is no device command; its rows are the ones a given command has.
        let (core, device, time, events, mut control, _frozen) = stopped_receive();
        let freq = key("radio.rx.c0.frequency_hz");
        for k in 0..40 {
            timed_at(&mut control, &core, &time, 10 + k, "radio.rx.c0.frequency_hz", 2.0e9 + k as f64 * 1e6, 100 + 5 * k as i64);
        }
        let ms = |n: i64| core.ticks(n * 1_000_000);
        time.advance_to(core.at(ms(400))).unwrap();
        control.release();
        assert!(!device.calls().iter().any(|call| call.starts_with("apply rx 0") && call.contains("freq=2")), "{:?}", device.calls());
        assert_eq!(device.dsp_pending(Dir::Rx, 0), 0);
        let rows = rows_of(&core, &freq);
        assert_eq!(rows.len(), 40, "{rows:?}");
        assert!(rows.iter().all(|(_, issued, no_channel)| *issued && !*no_channel), "a stopped channel's command is issued, with no note: {rows:?}");
        assert!(events_of(&events, kinds::COMMAND_REJECTED).is_empty());
    }

    #[test]
    fn ur_24_a_stopped_receive_channel_still_takes_its_gain() {
        // UR-24 (R1, series 1b): the radio takes a gain whatever streams; only a frequency waits
        // for a sample.
        let (core, device, time, _events, mut control, _frozen) = stopped_receive();
        timed_at(&mut control, &core, &time, 10, "radio.rx.c0.gain_db", 3.0, 100);
        timed_at(&mut control, &core, &time, 11, "radio.rx.c0.frequency_hz", 2.0e9, 120);
        let calls = device.calls();
        assert_eq!(calls.iter().filter(|call| call.starts_with("apply rx 0") && call.contains("gain=3") && !call.ends_with("at=-")).count(), 1, "{calls:?}");
        assert_eq!(calls.iter().filter(|call| call.starts_with("apply rx 0") && call.contains("freq=2")).count(), 0, "{calls:?}");
    }

    #[test]
    fn ur_24_a_frequency_for_a_far_planned_segment_is_deferred_to_its_configure() {
        // UR-24 (R1, series 2a, 2b): between a cut and the next segment's origin no sample
        // passes, and that segment is not configured yet: nothing is applied, and its configure
        // carries the value of the latest instant, however the updates arrived.
        let (core, device, time, _events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        device.rx_open(0, &[0]).unwrap();
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        control.book_cold(ActionId(1), key("radio.rx.s0.sample_rate_hz"), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(5_000)))));
        let plan = plan_of(&core, Dir::Rx);
        assert_eq!(plan.len(), 2);
        assert!(plan[1].segment.origin > ms(5_001), "the segment begins after the gap values' instants");
        // The values' instants descend, so that the last booked is not the last by instant.
        let mut instants = Vec::new();
        for (k, e) in [5_040, 5_030, 5_020, 5_010].into_iter().enumerate() {
            control.book_timed(ActionId(10 + k as u64), key("radio.rx.c0.frequency_hz"), Value::num(2.0e9 + k as f64 * 1e6).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(e)))));
            instants.push(e);
        }
        instants.sort_unstable();
        for e in instants {
            time.advance_to(core.at(ms(e) - core.ticks(500_000))).unwrap();
            control.release();
        }
        assert!(!device.calls().iter().any(|call| call.starts_with("apply rx 0") && call.contains("freq=2")), "{:?}", device.calls());
        assert_eq!(device.dsp_pending(Dir::Rx, 0), 0);
        plan[1].configure(&core, Dir::Rx).expect("the segment configures");
        let configured: Vec<String> = device.calls().into_iter().filter(|call| call.starts_with("apply rx 0") && call.contains("rate=2000000") && call.ends_with("at=-")).collect();
        assert_eq!(configured.len(), 1, "{:?}", device.calls());
        assert!(configured[0].contains("freq=2000000000"), "the value of the latest instant, 2.0e9 (booked first, due at 5 040 ms): {configured:?}");
    }

    #[test]
    fn ur_24_a_frequency_inside_a_running_segment_is_given_timed() {
        // UR-24 (R1, series 4): a sample passes at `e`, so the DDC takes it as a timed command.
        let (core, device, time, _events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        device.rx_open(0, &[0]).unwrap();
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        timed_at(&mut control, &core, &time, 10, "radio.rx.c0.frequency_hz", 2.1e9, 100);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let given: Vec<String> = device.calls().into_iter().filter(|call| call.starts_with("apply rx 0") && call.contains("freq=2100000000") && call.contains(&format!("at={} effective=", ms(100)))).collect();
        assert_eq!(given.len(), 1, "{:?}", device.calls());
        assert_eq!(device.dsp_pending(Dir::Rx, 0), 1, "no sample passes the unit rig's DDC");
    }

    #[test]
    fn ur_24_a_refusal_booked_after_a_release_leaves_the_pending_segments_value_to_its_configure() {
        // UR-24, RM-25 (design 43): S2 (C1) and S3 (C2) are planned; a value due one tick before
        // S3's origin o3 has no covering segment at its release, and S3 is not configured, so it
        // waits for S3's configure. C1 is refused after the release: S3 still begins at o3, so
        // its configure carries the value (untimed, `effective <= origin`) and the device holds
        // it from o3 on, with no timed command left in the DDC.
        let (core, device, time, _events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        device.rx_open(0, &[0]).unwrap();
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        control.book_cold(ActionId(1), key("radio.rx.s0.sample_rate_hz"), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(5_000)))));
        control.book_cold(ActionId(2), key("radio.rx.s0.sample_rate_hz"), Value::num(1e6).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(5_070)))));
        let planned = plan_of(&core, Dir::Rx);
        assert_eq!(planned.len(), 3, "S1, S2 and S3");
        let (o3, refused) = (planned[2].segment.origin, planned[1].segment.by.expect("C1's segment"));
        let e = o3 - 1;
        time.advance_to(core.at(e - ms(4))).unwrap();
        control.book_timed(ActionId(10), key("radio.rx.c0.frequency_hz"), Value::num(2.1e9).unwrap(), Some(AbsoluteDeadline::new(core.at(e))));
        time.advance_to(core.at(e - ms(3))).unwrap();
        control.release();
        assert!(!device.calls().iter().any(|call| call.starts_with("apply rx 0") && call.contains("freq=2100000000")), "the value waits for S3's configure: {:?}", device.calls());
        assert!(lock(&core.streams).refuse(&core, booked_stream(Dir::Rx), refused), "C1 is refused after the release");
        let planned = plan_of(&core, Dir::Rx);
        assert_eq!(planned.len(), 2, "S2 is left out");
        assert_eq!(planned[1].segment.origin, o3, "S3 still begins at o3");
        planned[1].configure(&core, Dir::Rx).expect("S3 configures");
        let calls = device.calls();
        let carried: Vec<&String> = calls.iter().filter(|call| call.starts_with("apply rx 0") && call.contains("freq=2100000000")).collect();
        assert_eq!(carried.len(), 1, "{calls:?}");
        assert!(carried[0].ends_with("at=-"), "the configure writes it untimed: {carried:?}");
        assert!(calls.iter().all(|call| !call.starts_with("apply rx 0") || call.ends_with("at=-")), "no timed command: {calls:?}");
        assert_eq!(device.dsp_pending(Dir::Rx, 0), 0);
        assert_eq!(device.freq_at(Dir::Rx, 0, o3), 2.1e9);
    }

    #[test]
    fn ur_24_a_frequency_due_at_a_segment_origin_is_given_timed_and_the_configure_carries_it() {
        // UR-24, RM-25 (design 43): as the refusal test, with the value due at S3's origin o3
        // itself. S3 covers its own origin, so the release gives it timed (a refusal changes
        // nothing about it); S3's configure carries it too (`effective <= origin`); the DDC takes
        // the timed entry with S3's first sample, so none is left waiting.
        let (core, device, time, _events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        device.rx_open(0, &[0]).unwrap();
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        control.book_cold(ActionId(1), key("radio.rx.s0.sample_rate_hz"), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(5_000)))));
        control.book_cold(ActionId(2), key("radio.rx.s0.sample_rate_hz"), Value::num(1e6).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(5_070)))));
        let planned = plan_of(&core, Dir::Rx);
        let (o3, refused) = (planned[2].segment.origin, planned[1].segment.by.expect("C1's segment"));
        let e = o3;
        time.advance_to(core.at(e - ms(4))).unwrap();
        control.book_timed(ActionId(10), key("radio.rx.c0.frequency_hz"), Value::num(2.1e9).unwrap(), Some(AbsoluteDeadline::new(core.at(e))));
        time.advance_to(core.at(e - ms(3))).unwrap();
        control.release();
        let timed = |calls: &[String]| calls.iter().filter(|call| call.starts_with("apply rx 0") && call.contains("freq=2100000000") && call.contains(&format!("at={e} effective={e}"))).count();
        assert_eq!(timed(&device.calls()), 1, "S3 covers its origin: {:?}", device.calls());
        assert_eq!(device.dsp_pending(Dir::Rx, 0), 1, "no sample has passed yet");
        assert!(lock(&core.streams).refuse(&core, booked_stream(Dir::Rx), refused));
        let planned = plan_of(&core, Dir::Rx);
        assert_eq!(planned[1].segment.origin, o3);
        planned[1].configure(&core, Dir::Rx).expect("S3 configures");
        let calls = device.calls();
        assert_eq!(calls.iter().filter(|call| call.starts_with("apply rx 0") && call.contains("freq=2100000000") && call.ends_with("at=-")).count(), 1, "the configure carries the value too: {calls:?}");
        assert_eq!(timed(&calls), 1, "{calls:?}");
        // S3's first sample takes the entry.
        device.rx_stop(0, None).unwrap();
        device.rx_start(0, o3).unwrap();
        device_at(&core, &time, &device, o3 + ms(10));
        assert_eq!(device.dsp_pending(Dir::Rx, 0), 0, "S3's first sample took the entry: {:?}", device.dsp_log(Dir::Rx, 0));
        assert_eq!(device.freq_at(Dir::Rx, 0, o3), 2.1e9);
    }

    #[test]
    fn ur_24_a_deferred_value_survives_a_start_booked_at_the_release_with_lead_equal_to_the_window() {
        // UR-24, RM-6 (design 43): the start lead equals the release window (3 ms, the least RM-6
        // allows). A value due at 100 ms for a stopped channel is deferred at its release, 97 ms; a
        // `start_rx` booked then begins its segment at 97 + 3 = 100 ms, the value's own instant,
        // and the configure carries it (`effective <= origin`): nothing is written timed.
        let device = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..crate::device::FakeConfig::default() }));
        let mut description = crate::provider::test_support::default_description(2_000);
        description.timing.rx_start_lead_ns = description.timing.release_window_ns;
        assert_eq!(description.timing.rx_start_lead_ns, 3_000_000);
        description.timing.check_start_lead().expect("RM-6 allows lead = window");
        let (core, time, _events) = crate::provider::test_support::rig_on_description(device.clone(), vec![Arc::new(NoLink)], BTreeMap::new(), description);
        let _frozen = hold(&core, &time, &device);
        device.rx_open(0, &[0]).unwrap();
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        control.book(Dispatched::new(ActionId(1), Action::Stop { target: Some(super::super::test_support::target_of(&core.rx_id)) }, Some(core.rx_id.clone())));
        let e = ms(100);
        time.advance_to(core.at(e - ms(4))).unwrap();
        control.book_timed(ActionId(10), key("radio.rx.c0.frequency_hz"), Value::num(2.1e9).unwrap(), Some(AbsoluteDeadline::new(core.at(e))));
        time.advance_to(core.at(e - ms(3))).unwrap();
        control.release();
        assert!(!device.calls().iter().any(|call| call.starts_with("apply rx 0") && call.contains("freq=2100000000")), "deferred: {:?}", device.calls());
        let node = core.stream_node(Dir::Rx, 0);
        control.book(Dispatched::new(ActionId(2), Action::Command {
            target: super::super::test_support::target_of(&node),
            verb: ezsdr_kernel::spec::Ident::parse(ezsdr_radio::START_RX).unwrap(),
            params: BTreeMap::new(),
            at: None,
        }, Some(node)));
        let planned = plan_of(&core, Dir::Rx);
        assert_eq!(planned.last().expect("the start's segment").segment.origin, e, "97 ms plus the 3 ms lead");
        planned.last().unwrap().configure(&core, Dir::Rx).expect("the segment configures");
        let calls = device.calls();
        let carried: Vec<&String> = calls.iter().filter(|call| call.starts_with("apply rx 0") && call.contains("freq=2100000000")).collect();
        assert_eq!(carried.len(), 1, "{calls:?}");
        assert!(carried[0].ends_with("at=-"), "{carried:?}");
        assert_eq!(device.freq_at(Dir::Rx, 0, e), 2.1e9);
    }

    #[test]
    fn ur_25_a_move_after_a_refused_source_change_configures_the_destination_at_the_unchanged_origin() {
        // RM-25, RM-34 (design 43): stream 1's rate change (due at 10 ms; its segment would begin
        // at 65 ms) is refused after a move of channel 1 into stream 0 is booked for 100 ms. The
        // refused segment counts as one the source holds until the move cuts it, so the
        // destination's segment (stream 0 has no channel) begins where it would had the change not
        // been refused, a lead after 100 ms, and its configure reaches the device at that plan.
        let run = |refuse: bool| {
            let (core, device, time, _events) = two_stream_rig();
            let _frozen = hold(&core, &time, &device);
            let mut values = core.description.defaults();
            values.insert(key("radio.rx.c0.stream"), Value::from(-1));
            values.insert(key("radio.rx.c1.stream"), Value::from(1));
            let start = Start { t0: 0, rx: vec![None, Some((Channels::first(2).without(0), 200))], tx: vec![None] };
            let (to_tx, _tx) = channel();
            let mut control = Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], values, false, start);
            device.rx_open(1, &[1]).unwrap();
            let (s0, s1) = (stream_key(Dir::Rx, 0), stream_key(Dir::Rx, 1));
            {
                let mut streams = lock(&core.streams);
                streams.record_open(s0, None);
                streams.record_open(s1, Some(Channels::first(2).without(0)));
            }
            let ms = |n: i64| core.ticks(n * 1_000_000);
            control.book_cold(ActionId(1), key("radio.rx.s1.sample_rate_hz"), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(10)))));
            control.book_cold(ActionId(2), key("radio.rx.c1.stream"), Value::from(0), Some(AbsoluteDeadline::new(core.at(ms(100)))));
            let changed = plan_of_stream(&core, Dir::Rx, 1).into_iter().find(|planned| planned.segment.by.is_some()).expect("the source change's segment");
            assert_eq!(changed.segment.origin, ms(65), "the segment the refusal will leave out");
            if refuse {
                assert!(lock(&core.streams).refuse(&core, s1, changed.segment.by.unwrap()));
                assert!(plan_of_stream(&core, Dir::Rx, 1).iter().all(|planned| planned.segment.by != changed.segment.by));
            }
            let destination = plan_of_stream(&core, Dir::Rx, 0).into_iter().find(|planned| planned.segment.config.channels.contains(1)).expect("the destination segment");
            let mark = device.calls().len();
            destination.configure(&core, Dir::Rx).expect("the destination configures");
            let configured = device.calls()[mark..].iter().filter(|call| call.starts_with("apply rx 1")).count();
            (destination.segment.origin, configured, ms(150))
        };
        let (plain, plain_calls, expected) = run(false);
        let (refused, refused_calls, _) = run(true);
        assert_eq!(plain, expected, "the source's release, 100 ms, and the lead");
        assert_eq!(refused, plain, "the destination's origin does not move");
        assert!(plain_calls >= 1 && refused_calls == plain_calls, "{plain_calls} {refused_calls}");
    }

    /// A two-sample waveform input and its artifact, for the transmit tests that submit a burst.
    fn tx_waveform() -> (ezsdr_kernel::hash::ContentHash, Arc<[u8]>, ezsdr_kernel::manifest::ArtifactRef) {
        let bytes: Vec<u8> = [(0.25f32, 0.0f32), (0.5, 0.0)].iter().flat_map(|(re, im)| [re.to_le_bytes(), im.to_le_bytes()].concat()).collect();
        let hash = ezsdr_kernel::hash::ContentHash::of_bytes(&bytes);
        let waveform = ezsdr_kernel::manifest::ingest_input(
            ezsdr_kernel::spec::Ident::parse("waveform").unwrap(),
            ezsdr_kernel::spec::Namespace::parse("ezsdr.input").unwrap(),
            format!("mem:{hash}"),
            &bytes,
        );
        (hash, Arc::from(bytes), waveform)
    }

    /// Submits a burst at sample `k` of `clock` and returns the root instant uhd-control hands uhd-tx
    /// for its first sample, with the sample it names; none when it was not handed over.
    fn submit_burst(control: &mut Control, core: &Core, to_tx: &Receiver<TxCmd>, waveform: &ezsdr_kernel::manifest::ArtifactRef, clock: Clock, k: i64) -> Option<(i64, i64)> {
        control.book(Dispatched::new(ActionId(90), Action::TxBurst {
            target: super::super::test_support::target_of(&core.tx_id),
            waveform: waveform.clone(),
            repeat: false,
            at: AbsoluteDeadline::new(TimePoint::new(clock.domain, k)),
            requested_at: None,
            late_policy: ezsdr_kernel::stream::LatePolicy::DropAndFlag,
            metadata: BTreeMap::new(),
        }, Some(core.tx_id.clone())));
        match to_tx.try_recv() {
            Ok(TxCmd::Burst(held)) => {
                assert_eq!(held.domain, clock.domain, "the burst names the successor's clock");
                Some((clock.instant(held.k), held.k))
            }
            _ => None,
        }
    }

    /// The transmit stream's plan as (segment origin, registered clock origin, the instant that
    /// clock ends at) for each segment, in ticks.
    fn transmit_clocks(core: &Core) -> Vec<(i64, Option<i64>, Option<i64>)> {
        let records = core.clocks.sample_clock_records();
        plan_of(core, Dir::Tx).iter().map(|planned| {
            let record = planned.clock.and_then(|clock| records.iter().find(|record| record.domain == clock.domain));
            (planned.segment.origin, planned.clock.map(|clock| clock.origin), record.and_then(|record| record.ended_at).map(|at| at.ticks_in(record.unwrap().root).unwrap()))
        }).collect()
    }

    #[test]
    fn ur_25_a_transmit_change_after_a_refusal_begins_as_if_not_refused() {
        // RM-25 (design 43): C (2 MS/s) due at 100 ms is refused, N (4 MS/s) is due at 300 ms. N
        // begins where it would had C not been refused: C is cut at 300 ms (it begins at 150 ms)
        // and N begins a lead (50 ms) after that, at 350 ms, whether N was booked before the
        // refusal, after it, or between C's instant and the refusal's record; the clock registered
        // for N keeps that origin. The refused clock ends at N's cut (300 ms) when N was booked
        // while C could still be cut, and at its own origin (150 ms) when it was booked after.
        for (when, ended) in [("before", 300), ("after", 150), ("between", 300)] {
            let (hash, bytes, waveform) = tx_waveform();
            let (core, _device, time, _events) = crate::provider::test_support::rig_with_inputs(BTreeMap::from([(hash, bytes)]));
            let (mut control, to_tx) = control(&core, &[Dir::Tx]);
            let ms = |n: i64| core.ticks(n * 1_000_000);
            let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
            control.book_cold(ActionId(1), key("radio.tx.s0.sample_rate_hz"), Value::num(2e6).unwrap(), at(100));
            let refused = plan_of(&core, Dir::Tx)[1].segment.by.expect("C's clock");
            assert_eq!(plan_of(&core, Dir::Tx)[1].segment.origin, ms(150));
            let refuse = |core: &Arc<Core>| assert!(lock(&core.streams).refuse(core, booked_stream(Dir::Tx), refused));
            if when == "before" {
                control.book_cold(ActionId(2), key("radio.tx.s0.sample_rate_hz"), Value::num(4e6).unwrap(), at(300));
                time.advance_to(core.at(ms(100))).unwrap();
                refuse(&core);
            } else if when == "after" {
                time.advance_to(core.at(ms(100))).unwrap();
                refuse(&core);
                control.book_cold(ActionId(2), key("radio.tx.s0.sample_rate_hz"), Value::num(4e6).unwrap(), at(300));
            } else {
                time.advance_to(core.at(ms(100))).unwrap();
                control.book_cold(ActionId(2), key("radio.tx.s0.sample_rate_hz"), Value::num(4e6).unwrap(), at(300));
                refuse(&core);
            }
            let clocks = transmit_clocks(&core);
            assert_eq!(clocks.len(), 3, "{when}: {clocks:?}");
            assert_eq!(clocks[2].0, ms(350), "{when}: the plan's origin: {clocks:?}");
            assert_eq!(clocks[2].1, Some(ms(350)), "{when}: the registered clock's origin: {clocks:?}");
            assert_eq!(clocks[1].2, Some(ms(ended)), "{when}: the refused clock's end: {clocks:?}");
            // A burst at sample 16 000 of N's clock (4 MS/s, 4 ms) is handed to uhd-tx for 354 ms.
            let clock = plan_of(&core, Dir::Tx)[2].clock.expect("N's clock");
            assert_eq!(submit_burst(&mut control, &core, &to_tx, &waveform, clock, 16_000), Some((ms(354), 16_000)), "{when}");
        }
    }

    #[test]
    fn ur_25_a_transmit_change_after_repeated_refusals_begins_as_if_none_were_refused() {
        // RM-25 (design 43): C1 (2 MS/s, 100 ms) and C2 (4 MS/s, 200 ms) are refused, each booked
        // once the one before it was; N (1 MS/s, 500 ms) begins at 550 ms: C1 is cut at 200, C2
        // begins at 250 and is cut at 500. The refused clocks end at their own origins, 150 and 250.
        let (hash, bytes, waveform) = tx_waveform();
        let (core, _device, time, _events) = crate::provider::test_support::rig_with_inputs(BTreeMap::from([(hash, bytes)]));
        let (mut control, to_tx) = control(&core, &[Dir::Tx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        control.book_cold(ActionId(1), key("radio.tx.s0.sample_rate_hz"), Value::num(2e6).unwrap(), at(100));
        time.advance_to(core.at(ms(100))).unwrap();
        let first = plan_of(&core, Dir::Tx)[1].segment.by.unwrap();
        assert!(lock(&core.streams).refuse(&core, booked_stream(Dir::Tx), first));
        control.book_cold(ActionId(2), key("radio.tx.s0.sample_rate_hz"), Value::num(4e6).unwrap(), at(200));
        time.advance_to(core.at(ms(200))).unwrap();
        let second = plan_of(&core, Dir::Tx)[2].segment.by.unwrap();
        assert!(lock(&core.streams).refuse(&core, booked_stream(Dir::Tx), second));
        control.book_cold(ActionId(3), key("radio.tx.s0.sample_rate_hz"), Value::num(1e6).unwrap(), at(500));
        let clocks = transmit_clocks(&core);
        assert_eq!(clocks.len(), 4, "{clocks:?}");
        assert_eq!((clocks[1].0, clocks[1].2), (ms(150), Some(ms(150))), "{clocks:?}");
        assert_eq!((clocks[2].0, clocks[2].2), (ms(250), Some(ms(250))), "{clocks:?}");
        assert_eq!((clocks[3].0, clocks[3].1), (ms(550), Some(ms(550))), "{clocks:?}");
        // A burst at sample 1 000 of N's clock (1 MS/s, 1 ms) is handed to uhd-tx for 551 ms.
        time.advance_to(core.at(ms(500))).unwrap();
        let clock = plan_of(&core, Dir::Tx)[3].clock.expect("N's clock");
        assert_eq!(submit_burst(&mut control, &core, &to_tx, &waveform, clock, 1_000), Some((ms(551), 1_000)));
    }

    #[test]
    fn ur_24_a_frequency_waits_for_the_next_segment_that_holds_its_channel() {
        // UR-24 (R1): the segment a value is for is the next one that holds its channel. Stream 0's
        // next segment A (channel 0, configured) begins at 155 ms, before stream 1's B (channel 1,
        // not configured) at 205 ms; a frequency of channel 1 due at 152 ms, covered by nothing, is
        // not written at its release as if A were its segment: B's configure carries it.
        let (core, device, time, _events) = two_stream_rig();
        let _frozen = hold(&core, &time, &device);
        let mut values = core.description.defaults();
        values.insert(key("radio.rx.c1.stream"), Value::from(1));
        let start = Start { t0: 0, rx: vec![Some((Channels::first(1), 200)), Some((Channels::first(2).without(0), 200))], tx: vec![None] };
        let (to_tx, _tx) = channel();
        let mut control = Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], values, false, start);
        device.rx_open(0, &[0]).unwrap();
        device.rx_open(1, &[1]).unwrap();
        let (s0, s1) = (stream_key(Dir::Rx, 0), stream_key(Dir::Rx, 1));
        {
            let mut streams = lock(&core.streams);
            streams.record_open(s0, Some(Channels::first(1)));
            streams.record_open(s1, Some(Channels::first(2).without(0)));
        }
        let ms = |n: i64| core.ticks(n * 1_000_000);
        control.book_cold(ActionId(1), key("radio.rx.s0.sample_rate_hz"), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(100)))));
        control.book_cold(ActionId(2), key("radio.rx.s1.sample_rate_hz"), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(150)))));
        let a = plan_of_stream(&core, Dir::Rx, 0).into_iter().find(|planned| planned.segment.by.is_some()).expect("A");
        let b = plan_of_stream(&core, Dir::Rx, 1).into_iter().find(|planned| planned.segment.by.is_some()).expect("B");
        a.configure(&core, Dir::Rx).expect("A configures");
        let first_end = plan_of_stream(&core, Dir::Rx, 1)[0].segment.instant(plan_of_stream(&core, Dir::Rx, 1)[0].segment.cut.expect("the first segment is cut")).unwrap();
        let e = ms(152);
        assert!(first_end <= e && e < a.segment.origin && a.segment.origin < b.segment.origin, "{first_end} {e} {} {}", a.segment.origin, b.segment.origin);
        time.advance_to(core.at(e - ms(4))).unwrap();
        control.book_timed(ActionId(10), key("radio.rx.c1.frequency_hz"), Value::num(2.2e9).unwrap(), Some(AbsoluteDeadline::new(core.at(e))));
        time.advance_to(core.at(e - core.ticks(500_000))).unwrap();
        control.release();
        assert!(!device.calls().iter().any(|call| call.starts_with("apply rx 1") && call.contains("freq=2200000000")), "written for the wrong segment: {:?}", device.calls());
        b.configure(&core, Dir::Rx).expect("B configures");
        let carried: Vec<String> = device.calls().into_iter().filter(|call| call.starts_with("apply rx 1") && call.contains("freq=2200000000")).collect();
        assert_eq!(carried.len(), 1, "{:?}", device.calls());
        assert!(carried[0].ends_with("at=-"), "{carried:?}");
    }

    #[test]
    fn ur_24_the_frequency_boundaries_at_the_origin_and_the_cut() {
        // UR-24 (R1, series 4c): a segment covers `e` from its origin on and until its cut's
        // instant, which is not covered; the cut is known when the value is released.
        let ms = |n: i64| n * 1_000_000;
        let one_tick = |core: &Core| core.ticks(1);
        for (case, at_cut_minus, covered) in [("e is the cut's instant", 0i64, false), ("e is one tick before the cut's instant", 1, true)] {
            let (core, device, time, _events) = manual_rig_events();
            let _frozen = hold(&core, &time, &device);
            device.rx_open(0, &[0]).unwrap();
            let (mut control, _tx) = control(&core, &[Dir::Rx]);
            control.book_cold(ActionId(1), key("radio.rx.s0.sample_rate_hz"), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(core.at(core.ticks(ms(300))))));
            let plan = plan_of(&core, Dir::Rx);
            let cut = plan[0].segment.instant(plan[0].segment.cut.expect("the first segment is cut")).unwrap();
            let e = cut - at_cut_minus * one_tick(&core);
            time.advance_to(core.at(e - core.ticks(4_000_000))).ok();
            control.book_timed(ActionId(2), key("radio.rx.c0.frequency_hz"), Value::num(2.1e9).unwrap(), Some(AbsoluteDeadline::new(core.at(e))));
            time.advance_to(core.at(e - core.ticks(1_000_000))).unwrap();
            control.release();
            let timed = device.calls().iter().filter(|call| call.starts_with("apply rx 0") && call.contains("freq=2100000000") && !call.ends_with("at=-")).count();
            assert_eq!(timed, usize::from(covered), "{case}: {:?}", device.calls());
        }
        // The origin: a value due at the new segment's origin is covered, one tick before is not.
        for (case, before, covered) in [("e is the origin", 0i64, true), ("e is one tick before the origin", 1, false)] {
            let (core, device, time, _events) = manual_rig_events();
            let _frozen = hold(&core, &time, &device);
            device.rx_open(0, &[0]).unwrap();
            let (mut control, _tx) = control(&core, &[Dir::Rx]);
            control.book_cold(ActionId(1), key("radio.rx.s0.sample_rate_hz"), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(core.at(core.ticks(ms(300))))));
            let plan = plan_of(&core, Dir::Rx);
            let e = plan[1].segment.origin - before * one_tick(&core);
            time.advance_to(core.at(e - core.ticks(4_000_000))).ok();
            control.book_timed(ActionId(2), key("radio.rx.c0.frequency_hz"), Value::num(2.1e9).unwrap(), Some(AbsoluteDeadline::new(core.at(e))));
            time.advance_to(core.at(e - core.ticks(1_000_000))).unwrap();
            control.release();
            let timed = device.calls().iter().filter(|call| call.starts_with("apply rx 0") && call.contains("freq=2100000000") && !call.ends_with("at=-")).count();
            assert_eq!(timed, usize::from(covered), "{case}: {:?}", device.calls());
        }
    }

    #[test]
    fn ur_24_a_gap_value_for_a_move_destination_waits_for_the_source_to_release() {
        // UR-24 (R1, M1): channel 1 moves from stream 1 to stream 0 at 100 ms; stream 0's owner
        // waits in `await_release` for stream 1's streamer. A frequency for 101 ms, released at
        // 98.5 ms while stream 1's segment still runs, reaches no device call — the destination
        // is not configured, so its configure carries the value after the release.
        let (core, device, time, _events) = two_stream_rig();
        let frozen = hold(&core, &time, &device);
        let mut values = core.description.defaults();
        values.insert(key("radio.rx.c1.stream"), Value::from(1));
        let start = Start { t0: 0, rx: vec![Some((Channels::first(1), 200)), Some((Channels::first(2).without(0), 200))], tx: vec![None] };
        let (to_tx, _tx) = channel();
        let mut control = Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], values, false, start);
        device.rx_open(0, &[0]).unwrap();
        device.rx_open(1, &[1]).unwrap();
        let (s0, s1) = (stream_key(Dir::Rx, 0), stream_key(Dir::Rx, 1));
        {
            let mut streams = lock(&core.streams);
            streams.record_open(s0, Some(Channels::first(1)));
            streams.record_open(s1, Some(Channels::first(2).without(0)));
            streams.search(s1, false);
        }
        let ms = |n: i64| core.ticks(n * 1_000_000);
        control.book_cold(ActionId(1), key("radio.rx.c1.stream"), Value::from(0), Some(AbsoluteDeadline::new(core.at(ms(100)))));
        let source = plan_of_stream(&core, Dir::Rx, 1).into_iter().find(|planned| planned.segment.by.is_none()).expect("stream 1's first segment");
        let destination = plan_of_stream(&core, Dir::Rx, 0).into_iter().find(|planned| planned.segment.config.channels.contains(1)).expect("the destination segment");
        assert!(destination.segment.origin > ms(101), "the destination begins after the value's instant");
        let (waiting, waited) = channel();
        crate::provider::test_support::probe(&core, "wait_release", move |_| { let _ = waiting.send(()); });
        let owner_core = core.clone();
        let (done, result) = channel();
        let destination_for_owner = destination.clone();
        let owner = core.device.host().spawn("rx-chain-0", move || { let _ = done.send(owner_core.owner_configure(Dir::Rx, 0, &destination_for_owner)); }).unwrap();
        assert!(waited.recv_timeout(Wall::from_secs(10)).is_ok(), "the destination's owner waits for the source's release");
        // The value, released before the source's cut while the destination waits.
        time.advance_to(core.at(ms(95))).unwrap();
        control.book_timed(ActionId(2), key("radio.rx.c1.frequency_hz"), Value::num(2.1e9).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(101)))));
        time.advance_to(core.at(ms(98) + core.ticks(500_000))).unwrap();
        let before = device.calls().len();
        control.release();
        assert_eq!(device.calls().len(), before, "no device call at the release: {:?}", &device.calls()[before..]);
        assert_eq!(device.dsp_pending(Dir::Rx, 1), 0);
        assert_eq!(device.freq_at(Dir::Rx, 1, ms(99)), 1e9, "the source's last samples keep their value");
        // The source releases; the destination's owner configures, and its configuration carries the value.
        {
            let mut streams = lock(&core.streams);
            streams.complete(s1, (source.segment.origin, source.segment.config, source.segment.by));
        }
        assert!(core.release_unneeded(Dir::Rx, 1), "the source releases its streamer");
        drop(frozen);
        owner.join().unwrap();
        assert!(result.recv_timeout(Wall::from_secs(10)).unwrap().unwrap(), "the destination opens");
        let configured = device.calls().iter().filter(|call| call.starts_with("apply rx 1 ") && call.contains("freq=2100000000") && call.ends_with("at=-")).count();
        assert_eq!(configured, 1, "the destination's configure carries the value: {:?}", device.calls());
    }

    #[test]
    fn ur_24_a_gap_value_for_a_configured_segment_is_applied_untimed_at_its_release() {
        // UR-24 (R1, M2, series 3a): an enable from no stream configured its segment; nothing
        // streams the channel before its origin, so a value for the gap is applied untimed at
        // its release, and the owner, whose record holds the segment's command, configures nothing.
        let (core, device, time, _events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        let ms = |n: i64| core.ticks(n * 1_000_000);
        channels_from_none(&mut control, &core, 2, 100, 1);
        let planned = plan_of(&core, Dir::Rx);
        assert!(planned[0].settings.as_ref().is_some_and(|settings| settings.is_configured()), "the enable's own configuration set the mark");
        let origin = planned[0].segment.origin;
        let before = device.calls().len();
        // Three instants in the gap between the enable's booking and its segment's origin.
        for k in 0..3u64 {
            let e = origin - ms(30 - 10 * k as i64);
            assert!(e > ms(10), "the gap is wide enough: origin {origin}");
            time.advance_to(core.at(e - core.ticks(4_000_000))).ok();
            control.book_timed(ActionId(10 + k), key("radio.rx.c0.frequency_hz"), Value::num(2.0e9 + k as f64 * 1e6).unwrap(), Some(AbsoluteDeadline::new(core.at(e))));
            time.advance_to(core.at(e - core.ticks(500_000))).unwrap();
            control.release();
        }
        let calls = device.calls()[before..].to_vec();
        assert!(calls.iter().all(|call| call.ends_with("at=-")), "no timed command: {calls:?}");
        let freqs: Vec<&String> = calls.iter().filter(|call| call.starts_with("apply rx 0") && call.contains("freq=2")).collect();
        assert_eq!(freqs.len(), 3, "{calls:?}");
        assert!(freqs[2].contains("freq=2002000000"), "{freqs:?}");
        assert_eq!(device.dsp_pending(Dir::Rx, 0), 0);
        let mark = device.calls().len();
        assert!(owner_rx(&core).begin(), "the owner begins the enable's segment");
        assert!(device.calls()[mark..].iter().all(|call| call.starts_with("rx_start")), "the owner configures nothing again: {:?}", &device.calls()[mark..]);
    }

    #[test]
    fn ur_24_a_frequency_before_t0_for_the_first_segment_is_applied_untimed() {
        // UR-24 (R1, M6): the Run's first segment was configured by `prepare`, and nothing
        // streams its channel before T0: a value for `now < e < T0` is applied untimed at its
        // release, and T0's first sample has it.
        let (core, device, time, _events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        let (to_tx, _tx) = channel();
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let mut config = core.description.defaults();
        config.insert(key("radio.rx.c0.stream"), Value::from(0));
        config.insert(key("radio.tx.c0.stream"), Value::from(-1));
        let start = Start { t0: ms(100), rx: vec![Some((Channels::first(1), 200)), None], tx: vec![None] };
        let mut control = Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], config, false, start);
        device.rx_open(0, &[0]).unwrap();
        timed_at(&mut control, &core, &time, 10, "radio.rx.c0.frequency_hz", 2.1e9, 50);
        let calls = device.calls();
        assert!(!calls.iter().any(|call| call.starts_with("apply rx 0") && call.contains("freq=2100000000") && !call.ends_with("at=-")), "no timed command: {calls:?}");
        assert_eq!(calls.iter().filter(|call| call.starts_with("apply rx 0") && call.contains("freq=2100000000") && call.ends_with("at=-")).count(), 1, "{calls:?}");
        assert_eq!(device.dsp_pending(Dir::Rx, 0), 0);
        assert_eq!(device.freq_at(Dir::Rx, 0, ms(100)), 2.1e9, "T0's first sample has the value");
    }

    #[test]
    fn ur_24_a_later_cut_that_removes_a_configured_segment_leaves_no_stale_mark() {
        // UR-24 (R1, M5a): a configured segment a cut at its origin makes empty is not the
        // value's destination, and its mark is not read. (The plan leaves such a segment out, so
        // the destination's exclusion of an empty one is not reached here: it is a guard.)
        let (core, device, time, _events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[]);
        control.config.insert(key("radio.rx.c0.stream"), Value::from(-1));
        let ms = |n: i64| core.ticks(n * 1_000_000);
        channels_from_none(&mut control, &core, 2, 100, 1);
        let origin = plan_of(&core, Dir::Rx)[0].segment.origin;
        assert_eq!(origin, ms(100), "the segment begins at the enable's instant (nothing was in flight)");
        time.advance_to(core.at(ms(100))).unwrap();
        let configured = plan_of(&core, Dir::Rx)[0].clone();
        assert!(configured.settings.as_ref().is_some_and(|settings| settings.is_configured()));
        // The stream stops at 100 ms, the segment's own origin: it never runs a sample.
        control.book(Dispatched::new(ActionId(3), Action::Stop { target: Some(super::super::test_support::target_of(&core.rx_id)) }, Some(core.rx_id.clone())));
        // The plan leaves a segment a cut empties out (RM-25), or keeps it with a zero length.
        let plan = plan_of(&core, Dir::Rx);
        let empty = plan.first().is_none_or(|planned| planned.segment.cut.is_some_and(|cut| planned.segment.instant(cut).unwrap() <= planned.segment.origin));
        assert!(empty, "the stop empties the segment: {:?}", plan_rows(&plan));
        let before = device.calls().len();
        timed_at(&mut control, &core, &time, 10, "radio.rx.c0.frequency_hz", 2.1e9, 108);
        let calls = device.calls()[before..].to_vec();
        assert!(!calls.iter().any(|call| call.contains("freq=2100000000")), "the emptied segment's mark was used: {calls:?}");
    }

    /// The manual rig with the receive stream's plan cut at 300 ms by a rate change: the first
    /// segment ends there, the second (not configured) begins after the gap's values.
    fn gap_rig() -> (Arc<Core>, Arc<crate::device::FakeDevice>, Arc<ezsdr_kernel::time::ManualTimeAuthority>, Control, Vec<Planned>, crate::provider::test_support::Frozen) {
        let (core, device, time, _events) = manual_rig_events();
        let frozen = hold(&core, &time, &device);
        device.rx_open(0, &[0]).unwrap();
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        control.book_cold(ActionId(1), key("radio.rx.s0.sample_rate_hz"), Value::num(2e6).unwrap(), Some(AbsoluteDeadline::new(core.at(core.ticks(300_000_000)))));
        let plan = plan_of(&core, Dir::Rx);
        assert_eq!(plan.len(), 2);
        time.advance_to(core.at(core.ticks(300_000_000))).unwrap();
        (core, device, time, control, plan, frozen)
    }

    /// Runs the owner's configure of `plan[1]` on a participant thread paused at `probe_name`
    /// (the section it holds open); returns the handshake and the join.
    fn paused_configure(core: &Arc<Core>, planned: Planned, probe_name: &'static str) -> (Receiver<()>, std::sync::mpsc::Sender<()>, Receiver<bool>) {
        let (reached, resume) = pause_core_at(core, probe_name);
        let owner_core = core.clone();
        let (done, result) = channel();
        core.device.host().spawn("rx-chain-0", move || { let _ = done.send(owner_core.owner_configure(Dir::Rx, 0, &planned).unwrap_or(false)); }).unwrap();
        (reached, resume, result)
    }

    /// Waits until `n` threads are inside the wait for `config`'s projection lock (a handshake
    /// that they have reached it and its holder keeps them there), within a bound.
    fn until_waiting(config: &ColdConfig, n: usize, what: &str) {
        let deadline = std::time::Instant::now() + Wall::from_secs(10);
        while config.updates_waiters() < n {
            assert!(std::time::Instant::now() < deadline, "{what}: no thread reached the projection's lock");
            std::thread::sleep(Wall::from_millis(1));
        }
    }

    #[test]
    fn ur_25_the_mark_is_published_while_the_projection_lock_is_held() {
        // UR-25 (R1): `configure` sets the mark before it lets the projection's lock go, so a
        // booking that waited for the lock finds it set. Observed at the boundary itself, in the
        // probe just before the store: the lock is held there.
        let (core, _device, _time, _control, plan, _frozen) = gap_rig();
        let settings = plan[1].settings.clone().expect("the segment's configuration");
        let held = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (settings_in_probe, held_in_probe) = (settings.clone(), held.clone());
        crate::provider::test_support::probe(&core, "mark_publishing", move |_| {
            held_in_probe.store(settings_in_probe.updates_held() && !settings_in_probe.is_configured(), Ordering::SeqCst);
        });
        plan[1].configure(&core, Dir::Rx).expect("the segment configures");
        assert!(held.load(Ordering::SeqCst), "the lock was held and the mark not yet set when the mark was published");
        assert!(settings.is_configured() && !settings.updates_held());
    }

    #[test]
    fn ur_24_a_value_booked_just_after_the_owner_configures_is_not_lost() {
        // UR-24 (R1, M3): the owner's configure has returned and has not published its record
        // yet (`configured`): the mark is already set, inside the `updates` lock, so a value
        // booked and released in that gap is applied untimed and not deferred to a configure that
        // is over.
        let (core, device, time, mut control, plan, _frozen) = gap_rig();
        let (reached, resume, result) = paused_configure(&core, plan[1].clone(), "configured");
        assert!(reached.recv_timeout(Wall::from_secs(10)).is_ok(), "the owner configured and paused before its record");
        assert!(plan[1].settings.as_ref().unwrap().is_configured());
        let ms = |n: i64| core.ticks(n * 1_000_000);
        control.book_timed(ActionId(2), key("radio.rx.c0.frequency_hz"), Value::num(2.1e9).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(310)))));
        time.advance_to(core.at(ms(310) - core.ticks(500_000))).unwrap();
        control.release();
        assert_eq!(device.freq_at(Dir::Rx, 0, ms(311)), 2.1e9, "the value was applied: {:?}", device.calls());
        assert!(!device.calls().iter().any(|call| call.contains("freq=2100000000") && !call.ends_with("at=-")), "never as a timed command: {:?}", device.calls());
        let _ = resume.send(());
        assert!(result.recv_timeout(Wall::from_secs(10)).unwrap());
    }

    #[test]
    fn ur_24_a_booking_during_a_configure_waits_and_the_release_after_it_applies_untimed() {
        // UR-24 (R1, M4a): while the owner's configure holds the segment's `updates` (paused
        // inside its first apply), a booking waits for the lock; once the configure is over the
        // release applies the value untimed, after the configure's own writes.
        let (core, device, time, control, plan, _frozen) = gap_rig();
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let (reached, resume, result) = paused_configure(&core, plan[1].clone(), "apply");
        assert!(reached.recv_timeout(Wall::from_secs(10)).is_ok(), "the configure is inside its first apply");
        let (booked, booked_rx) = channel();
        let (go, go_rx) = channel::<()>();
        let (done, done_rx) = channel();
        let at = Some(AbsoluteDeadline::new(core.at(ms(310))));
        let mut control = control;
        core.device.host().spawn("uhd-control", move || {
            control.book_timed(ActionId(2), key("radio.rx.c0.frequency_hz"), Value::num(2.1e9).unwrap(), at);
            let _ = booked.send(());
            let _ = go_rx.recv();
            control.release();
            let _ = done.send(());
        }).unwrap();
        until_waiting(plan[1].settings.as_ref().unwrap(), 1, "the booking");
        assert!(booked_rx.try_recv().is_err(), "the booking waits for the configure's lock");
        let _ = resume.send(());
        assert!(result.recv_timeout(Wall::from_secs(10)).unwrap());
        assert!(booked_rx.recv_timeout(Wall::from_secs(10)).is_ok(), "the booking completes after the configure");
        time.advance_to(core.at(ms(310) - core.ticks(500_000))).unwrap();
        let _ = go.send(());
        assert!(done_rx.recv_timeout(Wall::from_secs(10)).is_ok());
        let calls = device.calls();
        let configure = calls.iter().position(|call| call.starts_with("apply rx 0") && call.contains("rate=2000000") && call.ends_with("at=-")).expect("the configure's write");
        let release = calls.iter().position(|call| call.starts_with("apply rx 0") && call.contains("rate=-") && call.contains("freq=2100000000") && call.ends_with("at=-")).expect("the release's untimed write");
        assert!(configure < release, "{calls:?}");
        assert_eq!(device.freq_at(Dir::Rx, 0, ms(311)), 2.1e9);
    }

    #[test]
    fn ur_24_a_release_waits_for_a_configure_that_carried_its_booking() {
        // UR-24 (R1, M4b): the booking is in the configure's snapshot; a release started while
        // the configure holds the lock waits for it, reads the mark after it, and applies the
        // value once more untimed — a redundant write this pins (UR-25's mark is read under the
        // projections' locks).
        let (core, device, time, mut control, plan, _frozen) = gap_rig();
        let ms = |n: i64| core.ticks(n * 1_000_000);
        control.book_timed(ActionId(2), key("radio.rx.c0.frequency_hz"), Value::num(2.1e9).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(310)))));
        time.advance_to(core.at(ms(310) - core.ticks(500_000))).unwrap();
        let (reached, resume, result) = paused_configure(&core, plan[1].clone(), "apply");
        assert!(reached.recv_timeout(Wall::from_secs(10)).is_ok());
        let (done, done_rx) = channel();
        core.device.host().spawn("uhd-control", move || { control.release(); let _ = done.send(()); }).unwrap();
        until_waiting(plan[1].settings.as_ref().unwrap(), 1, "the release");
        assert!(done_rx.try_recv().is_err(), "the release waits for the configure's lock");
        let _ = resume.send(());
        assert!(result.recv_timeout(Wall::from_secs(10)).unwrap());
        assert!(done_rx.recv_timeout(Wall::from_secs(10)).is_ok());
        let calls = device.calls();
        let configure = calls.iter().position(|call| call.starts_with("apply rx 0") && call.contains("rate=2000000") && call.contains("freq=2100000000") && call.ends_with("at=-")).expect("the configure carried the value");
        let release = calls.iter().rposition(|call| call.starts_with("apply rx 0") && call.contains("rate=-") && call.contains("freq=2100000000") && call.ends_with("at=-")).expect("the release's untimed write");
        assert!(configure < release, "{calls:?}");
    }

    #[test]
    fn ur_24_a_move_round_trip_does_not_use_the_first_segment_s_mark() {
        // UR-24 (R1, M5b): channel 1 leaves stream 1 for stream 0 at 100 ms and returns at 200 ms.
        // Stream 1's first segment counts as configured (`prepare`'s); the return's segment is not.
        // A value for 201 ms, released at 198.5 ms while the middle segment still runs, is the
        // return segment's: no device call is made for it, its own mark decides.
        let (core, device, time, _events) = two_stream_rig();
        let _frozen = hold(&core, &time, &device);
        let mut values = core.description.defaults();
        values.insert(key("radio.rx.c1.stream"), Value::from(1));
        let start = Start { t0: 0, rx: vec![Some((Channels::first(1), 200)), Some((Channels::first(2).without(0), 200))], tx: vec![None] };
        let (to_tx, _tx) = channel();
        let mut control = Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], values, false, start);
        device.rx_open(0, &[0]).unwrap();
        device.rx_open(1, &[1]).unwrap();
        let ms = |n: i64| core.ticks(n * 1_000_000);
        control.book_cold(ActionId(1), key("radio.rx.c1.stream"), Value::from(0), Some(AbsoluteDeadline::new(core.at(ms(100)))));
        control.book_cold(ActionId(2), key("radio.rx.c1.stream"), Value::from(1), Some(AbsoluteDeadline::new(core.at(ms(200)))));
        let back = plan_of_stream(&core, Dir::Rx, 1).into_iter().find(|planned| planned.segment.by.is_some() && planned.segment.config.channels.contains(1)).expect("the returned segment");
        assert!(back.settings.as_ref().is_some_and(|settings| !settings.is_configured()), "the returned segment is not configured");
        assert!(back.segment.origin > ms(201));
        time.advance_to(core.at(ms(195))).unwrap();
        control.book_timed(ActionId(3), key("radio.rx.c1.frequency_hz"), Value::num(2.1e9).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(201)))));
        time.advance_to(core.at(ms(198) + core.ticks(500_000))).unwrap();
        let before = device.calls().len();
        control.release();
        assert_eq!(&device.calls()[before..], &[] as &[String], "no call for the return segment's value");
        assert_eq!(device.dsp_pending(Dir::Rx, 1), 0);
    }

    #[test]
    fn ur_29_a_deferred_frequency_stops_at_a_loss_like_a_given_one() {
        // UR-29 (R1, 12c): the loss is published before the channel's decision; the lost device
        // is not called, no row is written, and the command stays held.
        let (core, device, time, _events, mut control, _frozen) = stopped_receive();
        let ms = |n: i64| core.ticks(n * 1_000_000);
        time.advance_to(core.at(ms(95))).unwrap();
        control.book_timed(ActionId(10), key("radio.rx.c0.frequency_hz"), Value::num(2.1e9).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(100)))));
        time.advance_to(core.at(ms(98))).unwrap();
        let core_ = core.clone();
        crate::provider::test_support::probe(&core, "before_timed_apply", move |_| core_.device_lost("test loss"));
        control.release();
        assert!(core.is_lost());
        assert!(rows_of(&core, &key("radio.rx.c0.frequency_hz")).is_empty(), "{:?}", lock(&core.rec).applied);
        assert_eq!(control.held.len(), 1, "the command stays held for the end's settlement");
        assert!(!device.calls().iter().any(|call| call.starts_with("apply")), "{:?}", device.calls());
    }

    #[test]
    fn ur_24_deferred_frequency_rows_are_the_expected_ones() {
        // UR-24 (R1, 12d): the rows a frequency the device was not called for has are the ones a
        // given command has — explicit rows, not only a pair compared — whatever its channel's
        // state: a stopped channel and a gap value of a channel the stream holds are issued with
        // no note (a channel of no stream: `ur_24_a_timed_update_reaches_a_later_stream_s_…`
        // has `no channel`); a stop at `e` settles it issued, a loss at `e` cancels it.
        let freq = key("radio.rx.c0.frequency_hz");
        let ms = |core: &Core, n: i64| core.ticks(n * 1_000_000);
        // Stopped: (e, issued, no channel) = (e, true, false).
        let (core, _device, time, _events, mut control, _frozen) = stopped_receive();
        timed_at(&mut control, &core, &time, 10, "radio.rx.c0.frequency_hz", 2.1e9, 100);
        time.advance_to(core.at(ms(&core, 100))).unwrap();
        control.release();
        assert_eq!(rows_of(&core, &freq), [(ms(&core, 100), true, false)]);
        // A gap value of a channel the stream holds, released before the next segment begins.
        let (core, _device, time, mut control, plan, _frozen) = gap_rig();
        let e = plan[1].segment.origin - ms(&core, 10);
        time.advance_to(core.at(e - core.ticks(4_000_000))).ok();
        control.book_timed(ActionId(10), freq.clone(), Value::num(2.1e9).unwrap(), Some(AbsoluteDeadline::new(core.at(e))));
        time.advance_to(core.at(e - core.ticks(500_000))).unwrap();
        control.release();
        time.advance_to(core.at(e)).unwrap();
        control.release();
        assert_eq!(rows_of(&core, &freq), [(e, true, false)], "a gap value is recorded at its instant: {:?}", lock(&core.rec).applied);
        // `Provider::stop` at `e` settles a stopped channel's value issued; a loss at `e` does not.
        // And one due after the stop's instant is cancelled, whatever settled it.
        for (lost, early, expected) in [(false, 0, "issued"), (true, 0, "cancelled"), (false, 1, "cancelled")] {
            let (core, _device, time, _events, mut control, _frozen) = stopped_receive();
            timed_at(&mut control, &core, &time, 10, "radio.rx.c0.frequency_hz", 2.1e9, 100);
            let at = ms(&core, 100) - early * ms(&core, 1);
            control.settle(at, "Provider::stop", lost);
            let rows: Vec<_> = lock(&core.rec).applied.iter().filter(|row| row["key"] == freq.as_str()).cloned().collect();
            assert_eq!(rows.len(), 1, "{rows:?}");
            let state = if rows[0].get("cancelled").is_some() { "cancelled" } else if rows[0]["issued"] == true { "issued" } else { "other" };
            assert_eq!(state, expected, "lost: {lost}, stop {early} ms before e: {rows:?}");
        }
    }

    #[test]
    fn ur_25_a_transmit_channel_that_leaves_and_returns_keeps_its_old_timed_value() {
        // UR-25 (S2, a Ceiling): a timed transmit frequency V1 waits in the DUC for a burst; the
        // channel leaves its stream, a value V2 is recorded while it is out, and the return's
        // configuration writes V2 untimed. The first burst after the return is a timed send whose
        // first sample takes V1, which was never taken: the device ends on V1 and the record says
        // V2. The test pins that (the fake's FIFO outlives the streamer, INFERRED; KNOWN_LIMITATIONS.md).
        let (core, device, time, _events) = manual_rig_events();
        let frozen = hold(&core, &time, &device);
        let (mut control, _tx) = control(&core, &[Dir::Tx]);
        device.tx_open(0, &[0]).unwrap();
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
        let (v1, v2) = (2.1e9, 2.2e9);
        let name = "radio.tx.c0.frequency_hz";
        // V1 between bursts: the transmit channel is in its stream, so the DUC is given it.
        timed_at(&mut control, &core, &time, 10, name, v1, 50);
        assert_eq!(device.dsp_pending(Dir::Tx, 0), 1);
        // The channel leaves at 60 ms; V2, due at 80 ms with no stream, is recorded.
        control.book_cold(ActionId(11), key("radio.tx.c0.stream"), Value::from(-1), at(60));
        time.advance_to(core.at(ms(70))).unwrap();
        control.book_timed(ActionId(12), key(name), Value::num(v2).unwrap(), at(80));
        time.advance_to(core.at(ms(81))).unwrap();
        control.release();
        assert_eq!(device.dsp_pending(Dir::Tx, 0), 1, "V2 was given to no DUC");
        // It returns at 100 ms; the enable's own configuration writes the record's V2.
        lock(&core.streams).search(stream_key(Dir::Tx, 0), false);
        time.advance_to(core.at(ms(90))).unwrap();
        drop(frozen);
        control.book_cold(ActionId(13), key("radio.tx.c0.stream"), Value::from(0), at(100));
        let _frozen_after_open = crate::provider::test_support::Frozen::of(&device);
        assert!(device.calls().iter().any(|call| call.starts_with("apply tx 0") && call.contains("freq=2200000000") && call.ends_with("at=-")), "{:?}", device.calls());
        // The first timed burst after the return takes V1.
        device_at(&core, &time, &device, ms(195));
        let burst = ms(200);
        let wave = vec![[0.5f32, 0.25]; 1_000];
        device.tx_open(0, &[0]).unwrap();
        device.tx_send(0, &[&wave], Some(burst), true, true, Wall::from_secs(1)).unwrap();
        device_at(&core, &time, &device, ms(210));
        assert_eq!(device.freq_at(Dir::Tx, 0, burst - 1), v2, "the return's configuration: {:?}", device.dsp_log(Dir::Tx, 0));
        assert_eq!(device.freq_at(Dir::Tx, 0, burst), v1, "the old value, taken by the burst's first sample: {:?}", device.dsp_log(Dir::Tx, 0));
        assert_eq!(control.config_at(i64::MAX)[&key(name)], Value::num(v2).unwrap(), "the record says V2");
    }

    #[test]
    fn ur_24_a_frequency_just_after_an_origin_is_given_timed_and_follows_the_configure() {
        // UR-24 (R1, series 5): a value due just after the second segment's origin (inside the
        // release window of its configure) is covered by that segment: a timed command, given after
        // the segment's own untimed configuration, which the command's sample overwrites later.
        let (core, device, time, mut control, plan, _frozen) = gap_rig();
        plan[1].configure(&core, Dir::Rx).expect("the segment configures");
        let e = plan[1].segment.origin + core.ticks(1_000_000);
        time.advance_to(core.at(e - core.ticks(4_000_000))).ok();
        control.book_timed(ActionId(2), key("radio.rx.c0.frequency_hz"), Value::num(2.1e9).unwrap(), Some(AbsoluteDeadline::new(core.at(e))));
        time.advance_to(core.at(e - core.ticks(500_000))).unwrap();
        control.release();
        let calls = device.calls();
        let configure = calls.iter().position(|call| call.starts_with("apply rx 0") && call.contains("rate=2000000") && call.ends_with("at=-")).expect("the configure's write");
        let timed = calls.iter().position(|call| call.starts_with("apply rx 0") && call.contains("freq=2100000000") && call.contains(&format!("at={e} effective={e}"))).expect("the timed command");
        assert!(configure < timed, "{calls:?}");
        assert_eq!(calls.iter().filter(|call| call.starts_with("apply rx 0") && call.contains("freq=2100000000") && call.ends_with("at=-")).count(), 0, "{calls:?}");
        assert_eq!(device.dsp_pending(Dir::Rx, 0), 1, "no sample passes the unit rig's DDC");
    }

    #[test]
    fn ur_24_a_cut_booked_after_the_release_leaves_the_entry_for_the_next_segment() {
        // UR-24 (R1, series 6, a current limitation — R2 waits for bench P4): a value V1 given
        // as a timed command, and then a cut booked before its instant, stays in the DDC; a
        // later value V2 is given to no device (the stream is stopped) and the restart's
        // configuration writes it untimed, and the restart's first sample takes V1 over it.
        // INFERRED: that the owner's untimed stop precedes the entry's instant in the device.
        let (core, device, time, _events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        device.rx_open(0, &[0]).unwrap();
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let (v1, v2) = (2.1e9, 2.2e9);
        timed_at(&mut control, &core, &time, 10, "radio.rx.c0.frequency_hz", v1, 100);
        assert_eq!(device.dsp_pending(Dir::Rx, 0), 1);
        control.book(Dispatched::new(ActionId(11), Action::Stop { target: Some(super::super::test_support::target_of(&core.rx_id)) }, Some(core.rx_id.clone())));
        timed_at(&mut control, &core, &time, 12, "radio.rx.c0.frequency_hz", v2, 120);
        assert_eq!(device.dsp_pending(Dir::Rx, 0), 1, "V2 was given to no DDC");
        // The owner's stop and the restart: the configuration writes V2, the first sample takes V1.
        device.rx_stop(0, None).unwrap();
        device.apply(Dir::Rx, 0, &Settings { freq: Some(v2), ..Settings::default() }, None).unwrap();
        let restart = ms(200);
        device.rx_start(0, restart).unwrap();
        device_at(&core, &time, &device, ms(210));
        assert_eq!(device.freq_at(Dir::Rx, 0, restart - 1), v2, "the restart's configuration");
        assert_eq!(device.freq_at(Dir::Rx, 0, restart), v1, "the old entry, taken by the restart's first sample: {:?}", device.dsp_log(Dir::Rx, 0));
    }

    #[test]
    fn ur_24_a_frequency_of_a_channel_the_running_segment_does_not_hold_is_deferred() {
        // UR-24 (R1, series 7): channel 1 joins stream 0 at 100 ms; a value for it due at 80 ms is
        // for the new segment, not for the running one that holds channel 0 only — no timed
        // command —, while channel 0's own value due at 80 ms is a timed command.
        let (core, device, time, _events) = manual_rig_events();
        let _frozen = hold(&core, &time, &device);
        let mut control = layout_control(&core, Channels::first(1), &[("radio.rx.c0.stream", Value::from(0)), ("radio.rx.c1.stream", Value::from(-1))]);
        device.rx_open(0, &[0]).unwrap();
        let ms = |n: i64| core.ticks(n * 1_000_000);
        control.book_cold(ActionId(1), key("radio.rx.c1.stream"), Value::from(0), Some(AbsoluteDeadline::new(core.at(ms(100)))));
        let plan = plan_of(&core, Dir::Rx);
        assert_eq!(plan.len(), 2, "{:?}", plan_rows(&plan));
        time.advance_to(core.at(ms(70))).unwrap();
        control.book_timed(ActionId(2), key("radio.rx.c0.frequency_hz"), Value::num(2.0e9).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(80)))));
        control.book_timed(ActionId(3), key("radio.rx.c1.frequency_hz"), Value::num(2.1e9).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(80)))));
        time.advance_to(core.at(ms(80) - core.ticks(500_000))).unwrap();
        control.release();
        let calls = device.calls();
        assert_eq!(calls.iter().filter(|call| call.starts_with("apply rx 0 ") && call.contains("freq=2000000000") && call.contains("at=")).count(), 1, "{calls:?}");
        assert_eq!(calls.iter().filter(|call| call.starts_with("apply rx 1 ") && call.contains("freq=2100000000")).count(), 0, "channel 1's value is the new segment's: {calls:?}");
        plan[1].configure(&core, Dir::Rx).expect("the new segment configures");
        assert_eq!(device.calls().iter().filter(|call| call.starts_with("apply rx 1 ") && call.contains("freq=2100000000") && call.ends_with("at=-")).count(), 1, "its configure carries the value: {:?}", device.calls());
    }

    #[test]
    fn ur_24_a_refused_untimed_release_is_a_refusal_like_a_timed_one() {
        // UR-24 (R1, series 3d): the untimed write a release makes for a configured segment is
        // refused by the device: it is reported as a device failure, records no row, and the
        // value stays in the projection — as a timed command's refusal does (UR-29).
        let inner = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, command_queue: 64, ..crate::device::FakeConfig::default() }));
        let fails = Arc::new(crate::provider::test_support::ApplyFails::new(inner.clone(), Dir::Rx, None));
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(fails.clone(), vec![Arc::new(NoLink)], 2_000, BTreeMap::new());
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let (to_tx, _tx) = channel();
        let mut config = core.description.defaults();
        config.insert(key("radio.rx.c0.stream"), Value::from(0));
        config.insert(key("radio.tx.c0.stream"), Value::from(-1));
        let start = Start { t0: ms(100), rx: vec![Some((Channels::first(1), 200)), None], tx: vec![None] };
        let mut control = Control::new(core.clone(), Arc::new(NoActions), vec![to_tx], config, false, start);
        let freq = key("radio.rx.c0.frequency_hz");
        control.book_timed(ActionId(2), freq.clone(), Value::num(2.1e9).unwrap(), Some(AbsoluteDeadline::new(core.at(ms(50)))));
        time.advance_to(core.at(ms(47))).unwrap();
        control.release();
        assert_eq!(fails.attempts(), vec![(Dir::Rx, 0, None)], "the release's one write was untimed");
        assert_eq!(events_of(&events, kinds::COMMAND_REJECTED).len(), 1);
        assert!(rows_of(&core, &freq).is_empty(), "a refused release left a row: {:?}", lock(&core.rec).applied);
        assert!(control.held.is_empty(), "the refused command is not held again");
        assert_ne!(control.config.get(&freq), Some(&Value::num(2.1e9).unwrap()), "a refused release is not the configuration of record");
        let _ = inner;
    }
}
