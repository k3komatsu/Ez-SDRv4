//! uhd-control (UR-14): drains the `ActionReceiver` with MA-14b's loop, books each
//! Action — a stream's commands on its timeline, which it shares with the stream's owner —
//! releases held timed commands, and every 500 ms checks the reference and reads the
//! device's time. It never waits for a device instant.

use std::collections::BTreeMap;
use std::sync::mpsc::Sender;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use std::time::{Duration as Wall, Instant};

use ezsdr_kernel::event::{Action, Severity};
use ezsdr_kernel::id::ClockDomainId;
use ezsdr_kernel::module_api::{ActionReceiver, Requested, UpdateClass};
use ezsdr_kernel::spec::{Constraint, Key, Value};
use ezsdr_kernel::stream::{BurstOpen, Direction, LateOutcome};
use ezsdr_kernel::time::{AbsoluteDeadline, Duration, Rational, TimePoint, TimeError};
use ezsdr_radio::payloads::{
    ClockLostPayload, ClockReference, CommandQueueFullPayload, LateCommandPayload, TimeErrorCause,
    TimeErrorOutcome, TimeErrorPayload,
};
use ezsdr_radio::timeline::{self, Config, Item, Kind, Line, Segment};
use ezsdr_radio::{keys, kinds};
use serde_json::json;

use super::core::{Clock, Core, lock};
use super::tx::{Held, TxCmd};
use crate::device::{DeviceError, Dir, Iq, Settings, decimation};
use crate::profile::{DELIVERY_ALLOWANCE_NS, DEVICE_LEAD_NS, RELEASE_WINDOW_NS};

const POLL: Wall = Wall::from_millis(1);
const CHECK: Wall = Wall::from_millis(500);

/// A held `hardware_timed` command (UR-24).
struct Timed {
    key: Key,
    dir: Dir,
    value: f64,
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
/// effect at the segment's origin (UC-2's projection, UR-17), its rate and channel count the
/// segment's. Updates are projected in effective order, even when admitted after booking.
pub(crate) struct ColdConfig {
    base: Settings,
    updates: Mutex<BTreeMap<(i64, u64), Settings>>,
}

impl ColdConfig {
    pub fn new(base: Settings) -> Self {
        Self { base, updates: Mutex::new(BTreeMap::new()) }
    }

    fn book(&self, order: (i64, u64), settings: Settings) {
        lock(&self.updates).insert(order, settings);
    }

    fn issued(&self, updates: &mut BTreeMap<(i64, u64), Settings>, order: (i64, u64), effective: i64, settings: Settings) {
        updates.remove(&order);
        updates.insert((effective, order.1), settings);
    }

    /// The configuration in effect at `origin`.
    fn settings(&self, updates: &BTreeMap<(i64, u64), Settings>, origin: i64) -> Settings {
        let mut settings = self.base.clone();
        for update in updates.range(..=(origin, u64::MAX)).map(|(_, update)| update) {
            if let Some(freq) = update.freq { settings.freq = Some(freq); }
            if let Some(gain) = update.gain { settings.gain = Some(gain); }
        }
        settings
    }

    /// Configures the direction for `segment` (UR-12).
    pub fn configure(&self, core: &Core, dir: Dir, segment: &Segment) -> Result<i64, DeviceError> {
        // Hold the snapshot lock across the call: an update cannot change the
        // projection while an owner is configuring from it.
        let updates = lock(&self.updates);
        let ratio = segment.config.ratio;
        let rate = core.mcr as f64 * ratio.den() as f64 / ratio.num() as f64;
        let settings = Settings { rate: Some(rate), ..self.settings(&updates, segment.origin) };
        core.configure(dir, usize::from(segment.config.channels), &settings)
    }
}

/// A planned segment as the stream's owner reads it (UR-25).
#[derive(Clone)]
pub(crate) struct Planned {
    pub segment: Segment,
    /// The configuration in effect at its origin; none for the stream's first segment, or one
    /// whose command is pruned, which the owner has begun already.
    pub settings: Option<Arc<ColdConfig>>,
    /// uhd-control opened the streamer and configured the direction for it: an enable from no
    /// stream (UR-25).
    pub configured: bool,
    /// Its transmit clock, registered when it was booked (KC-21a); a receive clock is
    /// registered by uhd-rx at its first block.
    pub clock: Option<Clock>,
}

impl Planned {
    pub fn channels(&self) -> usize {
        usize::from(self.segment.config.channels)
    }

    /// Root ticks per sample.
    pub fn n(&self) -> i64 {
        self.segment.config.ratio.num() as i64
    }

    /// Configures the direction for the segment (UR-12).
    pub fn configure(&self, core: &Core, dir: Dir) -> Result<i64, DeviceError> {
        match &self.settings {
            Some(settings) => settings.configure(core, dir, &self.segment),
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
    /// The booked `cold` values, by effective instant and arrival (UC-2).
    colds: BTreeMap<(i64, u64), (Key, Value)>,
    held: BTreeMap<(i64, u64), Timed>,
    released: Vec<i64>,
    clock_lost: bool,
    /// When the device reads began failing without a lost-device error (Review S, S-B1).
    failing_since: Option<Instant>,
}

/// How long the device reads may fail, whatever the error, before the device is lost: a
/// dead link's control requests time out in UHD as `op_timeout`, which its C API returns as
/// `UHD_ERROR_EXCEPT`, not counted lost (UR-29; Review S, S-B1).
const FAILING_FOR: Wall = Wall::from_secs(1);

fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Num(v) if v.is_finite() => Some(*v),
        Value::Int(v) => Some(*v as f64),
        _ => None,
    }
}

/// The streams at the Run's start (UR-15): the receive stream's first segment at T0, when
/// it has a channel and a link, and the transmit clock registered at `arm`.
pub(crate) struct Start {
    pub t0: i64,
    pub rx: Option<(usize, i64)>,
    pub tx: Option<(Clock, usize)>,
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
        // RM-25, VH-4: this Module's terms — the start lead, and, for a receive segment after
        // a cut, the receive call in progress there and the delivery allowance (UR-25).
        let lead = core.ticks(core.description.timing.start_lead_ns);
        let call = core.block_len.max(core.device.rx_packet_samples()) as i64;
        let allowance = core.ticks(DELIVERY_ALLOWANCE_NS);
        let line = |direction: Direction, origin: i64, channels: usize, n: i64| Line::new(timeline::Stream {
            direction,
            origin,
            config: Config { channels: channels as u16, ratio: Rational::new(n.max(1) as u64, 1).expect("a positive ratio") },
            lead,
            call,
            allowance,
        });
        let n = |dir: Dir| {
            let rate = Core::settings(&config, dir).rate.unwrap_or(0.0);
            decimation(core.mcr, rate).map_or(1, |n| n as i64)
        };
        let rx = (!core.links.is_empty()).then(|| {
            let (channels, n) = start.rx.unwrap_or((0, n(Dir::Rx)));
            line(Direction::Rx, start.t0, channels, n)
        });
        let tx = match start.tx {
            Some((clock, channels)) => line(Direction::Tx, clock.origin, channels, clock.n),
            None => line(Direction::Tx, 0, 0, n(Dir::Tx)),
        };
        {
            let mut streams = lock(&core.streams);
            streams.tx_clocks = start.tx.map(|(clock, _)| (clock, false)).into_iter().collect();
            streams.lines = [rx, Some(tx)];
            streams.idle = [start.rx.is_none(), start.tx.is_none()];
        }
        Control {
            core,
            actions,
            to_tx,
            config,
            reference_monitored,
            colds: BTreeMap::new(),
            held: BTreeMap::new(),
            released: Vec::new(),
            clock_lost: false,
            failing_since: None,
        }
    }

    pub fn run(mut self) {
        let mut last_check = Instant::now();
        loop {
            if lock(&self.core.streams).stop.is_some() {
                return self.finish();
            }
            // MA-14b: each Action is booked before the next `recv()`.
            while let Some(action) = self.actions.recv() {
                if !self.core.is_lost() {
                    self.book(action);
                }
                self.release();
            }
            self.release();
            if last_check.elapsed() >= CHECK && !self.core.is_lost() {
                last_check = Instant::now();
                self.check_device();
            }
            std::thread::sleep(POLL);
        }
    }

    /// UR-26: `Provider::stop`, which has booked the streams' end, drops every pending
    /// command but one already handed to the device; UR-30's plans are recorded.
    fn finish(&mut self) {
        self.cancel_held("Provider::stop");
        let plan = lock(&self.core.streams).record();
        self.core.timing(plan);
    }

    pub(super) fn book(&mut self, action: Action) {
        match action {
            Action::TxBurst { target, waveform, repeat, at, requested_at, late_policy, metadata } => {
                self.book_burst(target, waveform, repeat, at, requested_at, late_policy, metadata.is_empty());
            }
            Action::UpdateParameter { target, key, value, class, at } => {
                let name = key.as_str();
                if target != self.core.id {
                    self.core.command_rejected("update_parameter", "UR-26: the target is not this device");
                } else if class == UpdateClass::HardwareTimed
                    && matches!(name, keys::RX_FREQUENCY_HZ | keys::TX_FREQUENCY_HZ | keys::RX_GAIN_DB | keys::TX_GAIN_DB)
                {
                    self.book_timed(key, value, at);
                } else if class == UpdateClass::Cold
                    && matches!(name, keys::RX_CHANNELS | keys::TX_CHANNELS | keys::RX_SAMPLE_RATE_HZ | keys::TX_SAMPLE_RATE_HZ)
                {
                    self.book_cold(key, value, at);
                } else {
                    self.core.command_rejected("update_parameter", &format!("UR-26: {name} is not updated at runtime by this Provider"));
                }
            }
            // RM-16, UR-26: a `Stop` cancels no update; on receive it cuts at its booking, on
            // transmit it ends the bursts, not the clock.
            Action::Stop { target: Some(target) } => {
                let device = target == self.core.id;
                if !device && target != self.core.tx_id && target != self.core.rx_id {
                    return self.core.command_rejected("stop", "UR-26: the Stop target is not this device or its streams");
                }
                if device || target == self.core.tx_id {
                    let _ = self.to_tx.send(TxCmd::Stop);
                }
                if device || target == self.core.rx_id {
                    let mut streams = lock(&self.core.streams);
                    let (seq, now) = (streams.next_seq(), self.core.now());
                    streams.book(&self.core, Dir::Rx, Item { e: now, seq, ready: now, delivered: None, refused: false, kind: Kind::Stop });
                }
            }
            Action::Stop { target: None } | Action::Abort { .. } => {}
            Action::SetTimer { .. } => self.core.command_rejected("set_timer", "UR-26: not supported by this Provider"),
            // RM-12, RM-21: `start_rx` turns the receive stream on at its `at`, or at its booking
            // if that is later; it is never late.
            Action::PeripheralCommand { target, verb, params, at }
                if verb.as_str() == ezsdr_radio::START_RX && target == self.core.rx_id && params.is_empty() =>
            {
                let requested = match self.ceil_root(at) {
                    Ok(requested) => requested,
                    Err(error) => return self.core.command_rejected("peripheral_command", &format!("UR-26: start_rx's instant cannot be converted: {error}")),
                };
                let config = self.segment_config(Dir::Rx);
                let mut streams = lock(&self.core.streams);
                let (seq, now) = (streams.next_seq(), self.core.now());
                let (e, _) = timeline::command_instant(streams.lines[Dir::Rx as usize].as_ref(), requested, now, seq, false);
                streams.configs[Dir::Rx as usize].insert(seq, (config, false));
                streams.book(&self.core, Dir::Rx, Item { e, seq, ready: now, delivered: None, refused: false, kind: Kind::Start });
            }
            Action::PeripheralCommand { .. } => {
                self.core.command_rejected("peripheral_command", "UR-26: not supported by this Provider")
            }
            Action::Emit { .. } => self.core.command_rejected("emit", "UR-26: not supported by this Provider"),
        }
    }

    /// The configuration a segment a command begins in `dir` starts from: the one in force,
    /// with the timed commands held for that direction (UC-2, UR-17).
    fn segment_config(&self, dir: Dir) -> Arc<ColdConfig> {
        let config = Arc::new(ColdConfig::new(Core::settings(&self.config, dir)));
        for (&order, timed) in self.held.iter().filter(|(_, timed)| timed.dir == dir) {
            config.book(order, timed.settings());
        }
        config
    }

    /// The segment configurations of `dir`, shared with its owner.
    fn projections(&self, dir: Dir) -> Vec<Arc<ColdConfig>> {
        lock(&self.core.streams).configs[dir as usize].values().map(|(config, _)| config.clone()).collect()
    }

    fn ceil_root(&self, at: Option<AbsoluteDeadline>) -> Result<Option<i64>, TimeError> {
        let Some(t) = at.map(|deadline| deadline.time_point) else { return Ok(None); };
        if t.domain == self.core.root {
            return Ok(Some(t.ticks));
        }
        let ticks = match self.core.clocks.convert(t, self.core.root)? {
            ezsdr_kernel::time::Converted::Exact { point } => point.ticks,
            ezsdr_kernel::time::Converted::Inexact { floor, .. } => floor.ticks.checked_add(1).ok_or(TimeError::Overflow)?,
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
        if lock(&self.core.streams).lines[dir as usize].as_ref().is_some_and(|line| line.halted(t)) {
            return 0;
        }
        Core::channels(&self.config_at(t), dir)
    }

    // ------------------------------------------------------------ UR-24

    fn book_timed(&mut self, key: Key, value: Value, at: Option<AbsoluteDeadline>) {
        let Some(v) = number(&value) else {
            return self.core.command_rejected("update_parameter", "UR-24: the value is not a number");
        };
        let dir = if key.as_str().starts_with("radio.rx.") { Dir::Rx } else { Dir::Tx };
        let now = self.core.now();
        let requested = match self.ceil_root(at) {
            Ok(requested) => requested,
            Err(error) => return self.core.command_rejected("update_parameter", &format!("UR-24: explicit deadline cannot be converted: {error}")),
        };
        let Some(earliest) = now.checked_add(self.core.ticks(DEVICE_LEAD_NS)) else {
            return self.core.command_rejected("update_parameter", "UR-24: time arithmetic overflow");
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
        let depth = self.core.description.timing.command_queue_depth as usize;
        // The device queue holds one command per channel (Review L, NONBLOCKING 6).
        let pending: usize = self.held.iter().map(|((e, _), t)| self.channels_at(t.dir, *e)).sum::<usize>() + self.released.len();
        if pending + self.channels_at(dir, e) > depth {
            let payload = serde_json::to_value(CommandQueueFullPayload { key: key.clone(), depth: depth as i64 }).expect("a payload");
            self.core.emit(&self.core.id, kinds::COMMAND_QUEUE_FULL, Severity::Error, payload);
            self.core.reject_note(json!({ "action": "update_parameter", "reason": "UR-24: the command queue is full", "key": key }));
            return;
        }
        let seq = lock(&self.core.streams).next_seq();
        let timed = Timed { key, dir, value: v };
        for config in self.projections(dir) {
            config.book((e, seq), timed.settings());
        }
        self.held.insert((e, seq), timed);
    }

    /// Releases held commands within the release window, in effective order, each on the
    /// channels in force at its own instant; one for a direction with no channel then, or
    /// halted by a refusal, is recorded in the configuration, and enabling the direction
    /// applies it (UR-24; #61, #64).
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
            let channels = self.channels_at(timed.dir, effective);
            if channels == 0 {
                self.config.insert(timed.key.clone(), Value::Num(timed.value));
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
            let mut updates: Vec<_> = projections.iter().map(|cold| lock(&cold.updates)).collect();
            for chan in 0..channels {
                if let Err(error) = self.core.device.apply(timed.dir, chan, &settings, Some(effective)) {
                    self.core.device_failed("update_parameter", &error);
                    return;
                }
                self.released.push(effective);
            }
            for (cold, updates) in projections.iter().zip(updates.iter_mut()) {
                cold.issued(updates, (e, seq), effective, settings.clone());
            }
            drop(updates);
            self.config.insert(timed.key.clone(), Value::Num(timed.value));
            lock(&self.core.rec).applied.push(json!({
                "key": timed.key, "claimed": timed.value, "read_back": null, "at": self.core.at(effective), "issued": true,
            }));
        }
    }

    fn cancel_held(&mut self, why: &str) {
        let at = self.core.at(self.core.now());
        for ((e, seq), timed) in std::mem::take(&mut self.held) {
            let projections = self.projections(timed.dir);
            let mut updates: Vec<_> = projections.iter().map(|cold| lock(&cold.updates)).collect();
            for updates in &mut updates { updates.remove(&(e, seq)); }
            // Keep the projections locked until the cancellation is recorded, so an owner can
            // never apply this held value after that record.
            lock(&self.core.rec).applied.push(json!({
                "key": timed.key, "claimed": timed.value, "at": self.core.at(e), "cancelled": why, "cancelled_at": at,
            }));
        }
    }

    // ------------------------------------------------------------ UR-25

    /// UR-25: a `cold` update is booked when uhd-control takes it: its effective instant,
    /// `coerce` over the configuration projected there, and the stream's plan from the
    /// timeline, which its owner reads.
    fn book_cold(&mut self, key: Key, value: Value, at: Option<AbsoluteDeadline>) {
        let dir = if key.as_str().starts_with("radio.rx.") { Dir::Rx } else { Dir::Tx };
        let requested = match self.ceil_root(at) {
            Ok(requested) => requested,
            Err(error) => return self.core.command_rejected("update_parameter", &format!("UR-25: explicit deadline cannot be converted: {error}")),
        };
        let segment_config = self.segment_config(dir);
        // The instant is taken under the lock that books it, so that nothing pruned meanwhile
        // takes effect after it (#63).
        let mut streams = lock(&self.core.streams);
        let (seq, now) = (streams.next_seq(), self.core.now());
        let (e, late) = timeline::command_instant(streams.lines[dir as usize].as_ref(), requested, now, seq, true);
        let mut candidate = self.config_at(e);
        candidate.insert(key.clone(), value.clone());
        let request = Requested {
            resource: self.core.id.clone(),
            constraints: candidate.iter().map(|(k, v)| (k.clone(), Constraint::Eq { value: v.clone() })).collect(),
        };
        let report = match self.core.description.coerce(&self.core.id, &request) {
            Ok(report) => report,
            Err(error) => return self.core.command_rejected("update_parameter", &error.message),
        };
        if let Some(rejected) = report.rejected.first() {
            return self.core.command_rejected("update_parameter", &rejected.reason);
        }
        let value = report.applied.get(&key).cloned().unwrap_or(value);
        candidate.insert(key.clone(), value.clone());
        let channels = Core::channels(&candidate, dir);
        let rate = Core::settings(&candidate, dir).rate.unwrap_or(0.0);
        let n = match decimation(self.core.mcr, rate) {
            Some(n) => n as i64,
            None if channels == 0 => 0,
            None => return self.core.command_rejected("update_parameter", &format!("UR-12: {rate} S/s is no decimation 1…512 of {} Hz", self.core.mcr)),
        };
        if late {
            self.late_command(Some(key.clone()), requested.unwrap_or(now), e);
        }
        let Some(old) = streams.lines[dir as usize].as_ref().map(|line| line.stream.config) else {
            // A receive side with no link has no stream: the change only takes its value.
            self.colds.insert((e, seq), (key, value));
            return;
        };
        let ratio = if n > 0 { Rational::new(n as u64, 1).expect("a decimation") } else { old.ratio };
        // RM-25, VH-4: an enable from no stream — its owner idle, nothing planned to run —
        // is configured by uhd-control itself, which opens the streamer, and is ready when
        // that configuration ends; any other change by the owner at the cut (UR-17, UR-23).
        let from_none = channels > 0 && streams.idle[dir as usize] && streams.lines[dir as usize].as_ref().is_some_and(|line| line.plan.iter().all(|segment| segment.cut.is_some()));
        let mut ready = now;
        if from_none {
            // Its owner is idle: nothing it could prune comes meanwhile.
            drop(streams);
            let opened = match dir {
                Dir::Rx => self.core.device.rx_open(channels),
                Dir::Tx => self.core.device.tx_open(channels),
            };
            // The configuration in effect at `e`, the timed updates held for it included (UC-2).
            let mut settings = Core::settings(&candidate, dir);
            for timed in self.held.range(..=(e, u64::MAX)).map(|(_, timed)| timed).filter(|timed| timed.dir == dir) {
                let update = timed.settings();
                settings.freq = update.freq.or(settings.freq);
                settings.gain = update.gain.or(settings.gain);
            }
            if let Err(error) = opened.and_then(|()| self.core.configure(dir, channels, &settings)) {
                return self.core.device_failed("update_parameter", &error);
            }
            ready = self.core.now();
            streams = lock(&self.core.streams);
        }
        let config = Config { channels: channels as u16, ratio };
        streams.configs[dir as usize].insert(seq, (segment_config, from_none));
        if streams.book(&self.core, dir, Item { e, seq, ready, delivered: None, refused: false, kind: Kind::Cold(config) }) {
            self.colds.insert((e, seq), (key.clone(), value));
            self.core.timing(json!({ "what": "cold_change", "key": key, "e": e, "booked_at": now, "ready": ready }));
        } else {
            streams.configs[dir as usize].remove(&seq);
        }
    }

    // ------------------------------------------------------------ UR-21

    #[allow(clippy::too_many_arguments)]
    fn book_burst(
        &mut self,
        target: ezsdr_kernel::id::ResourceId,
        waveform: ezsdr_kernel::manifest::ArtifactRef,
        repeat: bool,
        at: AbsoluteDeadline,
        requested_at: Option<AbsoluteDeadline>,
        late_policy: ezsdr_kernel::stream::LatePolicy,
        metadata_empty: bool,
    ) {
        let reject = |reason: &str| self.core.command_rejected("tx_burst", reason);
        // KC-21a: the last transmit clock registered, while its segment has not ended.
        let current = {
            let streams = lock(&self.core.streams);
            let last = streams.lines[Dir::Tx as usize].as_ref().and_then(|line| line.plan.last()).filter(|segment| segment.cut.is_none());
            streams.tx_clocks.last().map(|(clock, _)| *clock).zip(last.map(|segment| usize::from(segment.config.channels)))
        };
        let Some((clock, channels)) = current else {
            return reject("UR-21: no transmit channel is configured");
        };
        if target != self.core.tx_id {
            return reject("UR-21: the target is not this device's transmit stream");
        }
        if at.time_point.domain != clock.domain {
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
        let mut start = at.time_point.ticks;
        let mut requested = requested_at.map(|d| d.time_point);
        let time_error = |outcome: TimeErrorOutcome, late_by: Duration| {
            let payload = serde_json::to_value(TimeErrorPayload {
                cause: TimeErrorCause::Late,
                outcome,
                late_by_ns: late_by.ticks,
                target: at.time_point,
            })
            .expect("a payload");
            self.core.emit(&self.core.tx_id, kinds::TIME_ERROR, Severity::Error, payload);
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
                return self.core.reject_note(json!({ "action": "tx_burst", "reason": "UR-21: the late policy dropped the burst" }));
            }
            LateOutcome::PlanViolation { late_by } => {
                time_error(TimeErrorOutcome::PlanViolation, late_by);
                return self.core.reject_note(json!({ "action": "tx_burst", "reason": "UR-21: the planned burst arrived late" }));
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
        let begun = Instant::now();
        match self.core.device.time_now() {
            Ok(_) => self.failing_since = None,
            Err(error) if error.lost => self.core.device_lost(&error.message),
            Err(error) => {
                self.core.timing(json!({ "what": "time_read_failed", "error": error.message }));
                let since = *self.failing_since.get_or_insert(begun);
                if since.elapsed() >= FAILING_FOR {
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
    use ezsdr_kernel::stream::{BackPressure, BlockRef, DataLink, DropCarry, PublishOutcome};

    struct NoActions;
    impl ActionReceiver for NoActions {
        fn recv(&self) -> Option<Action> { None }
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
        let mut config = core.description.defaults.clone();
        let mut start = Start { t0: 0, rx: None, tx: None };
        for dir in on {
            config.insert(key(&format!("radio.{}.channels", dir.name())), Value::Int(1));
            match dir {
                Dir::Rx => start.rx = Some((1, 200)),
                Dir::Tx => start.tx = Some((core.register(Dir::Tx, 200, 0).unwrap(), 1)),
            }
        }
        (Control::new(core.clone(), Arc::new(NoActions), to_tx, config, false, start), tx)
    }

    /// The plan of `dir` as its owner reads it.
    fn plan_of(core: &Core, dir: Dir) -> Vec<Planned> {
        lock(&core.streams).planned(dir)
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
            control.book_cold(key("radio.rx.sample_rate_hz"), Value::Num(2e6), Some(AbsoluteDeadline::new(at)));
            control.book_timed(key("radio.rx.gain_db"), Value::Num(3.0), None);
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
    fn ur_30_the_recorded_plan_has_no_refused_segment() {
        // UR-30, RM-25: a refusal the owner booked is in the one plan `Provider::stop` records,
        // which has no segment for it.
        let (core, _, time) = rig();
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        control.book_cold(key("radio.rx.sample_rate_hz"), Value::Num(2e6), Some(AbsoluteDeadline::new(core.at(ms(100)))));
        control.book_cold(key("radio.rx.sample_rate_hz"), Value::Num(1e6), Some(AbsoluteDeadline::new(core.at(ms(300)))));
        let refused = plan_of(&core, Dir::Rx)[1].segment.by.unwrap();
        assert!(lock(&core.streams).refuse(&core, Dir::Rx, refused));
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
        control.book_cold(key("radio.rx.sample_rate_hz"), Value::Num(2e6), Some(AbsoluteDeadline::new(core.at(ms(300)))));
        control.book_cold(key("radio.rx.sample_rate_hz"), Value::Num(1e6), Some(AbsoluteDeadline::new(core.at(ms(100)))));
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
            lock(&core.streams).delivered = delivered.map(|k| (0, config, k));
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
        control.book_cold(key("radio.tx.sample_rate_hz"), Value::Num(2e6), None);
        lock(&core.streams).end(&core, core.ticks(5_000_000), false, false);
        let clocks: Vec<_> = core.clocks.sample_clock_records().iter().map(|r| (r.origin.ticks, r.ended_at.map(|end| end.ticks))).collect();
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
            match dir { Dir::Rx => device.rx_open(1).unwrap(), Dir::Tx => device.tx_open(1).unwrap() }
            let (mut control, _tx) = control(&core, &[dir]);
            control.book_cold(key(&format!("radio.{}.sample_rate_hz", dir.name())), Value::Num(2e6),
                Some(AbsoluteDeadline::new(core.at(core.ticks(1_000_000_000)))));
            let gain = key(&format!("radio.{}.gain_db", dir.name()));
            control.book_timed(gain.clone(), Value::Num(3.0), None);
            control.release();
            assert_eq!(control.config[&gain], Value::Num(3.0));
            assert!(lock(&core.rec).rejected.is_empty());
            let plan = plan_of(&core, dir);
            plan[1].configure(&core, dir).unwrap();
            let settings = plan[1].settings.clone().unwrap();
            observed.push((dir.name(), settings.settings(&lock(&settings.updates), plan[1].segment.origin).gain));
        }
        assert_eq!(observed, vec![("rx", Some(3.0)), ("tx", Some(3.0))], "both cold switches must preserve the later update due before the new origin");
    }

    #[test]
    fn ur_25_switch_projects_timed_updates_in_effective_order() {
        let cold = ColdConfig::new(Settings { gain: Some(0.0), freq: Some(1e9), ..Settings::default() });
        cold.book((70, 1), Settings { gain: Some(7.0), ..Settings::default() });
        cold.book((50, 2), Settings { gain: Some(5.0), ..Settings::default() });
        cold.book((101, 3), Settings { gain: Some(9.0), freq: Some(2e9), ..Settings::default() });
        cold.book((100, 4), Settings { freq: Some(1.1e9), ..Settings::default() });
        assert_eq!(cold.settings(&lock(&cold.updates), 100).gain, Some(7.0));
        assert_eq!(cold.settings(&lock(&cold.updates), 100).freq, Some(1.1e9));
        // A command delayed by the device queue takes its actual effective order.
        cold.issued(&mut lock(&cold.updates), (50, 2), 80, Settings { gain: Some(5.0), ..Settings::default() });
        assert_eq!(cold.settings(&lock(&cold.updates), 100).gain, Some(5.0));
        cold.issued(&mut lock(&cold.updates), (100, 4), 101, Settings { freq: Some(1.1e9), ..Settings::default() });
        assert_eq!(cold.settings(&lock(&cold.updates), 100).freq, Some(1e9));
    }

    #[test]
    fn ur_26_a_stop_cancels_no_update() {
        // RM-16, UR-26 (spec 22, VH-6): a `Stop` of a stream or of the device cancels no held
        // timed command, and the projection of a segment booked after it keeps it; only
        // `Provider::stop` drops what is held.
        for target in ["usrp/rx", "usrp/tx", "usrp"] {
            let (core, device, _) = rig();
            device.tx_open(1).unwrap();
            let (mut control, _tx) = control(&core, &[Dir::Rx, Dir::Tx]);
            let gain = key("radio.tx.gain_db");
            control.book_timed(gain.clone(), Value::Num(3.0), Some(AbsoluteDeadline::new(core.at(core.ticks(1_040_000_000)))));
            control.book(Action::Stop { target: Some(ezsdr_kernel::id::ResourceId::parse(target).unwrap()) });
            control.book_cold(key("radio.tx.sample_rate_hz"), Value::Num(2e6), Some(AbsoluteDeadline::new(core.at(core.ticks(1_000_000_000)))));
            assert_eq!(control.held.len(), 1, "{target}");
            assert!(!lock(&core.rec).applied.iter().any(|r| r.get("cancelled").is_some()), "{target}");
            let plan = plan_of(&core, Dir::Tx);
            let settings = plan[1].settings.clone().unwrap();
            assert_eq!(settings.settings(&lock(&settings.updates), plan[1].segment.origin).gain, Some(3.0), "{target}");
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
                control.book_cold(key(&format!("radio.{}.sample_rate_hz", dir.name())), Value::Num(2e6),
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
        control.book_cold(key("radio.tx.channels"), Value::Int(0), Some(AbsoluteDeadline::new(core.at(aligned_max))));
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
                        if cold { control.book_cold(key(&name), Value::Num(2e6), Some(AbsoluteDeadline::new(at))); }
                        else { control.book_timed(key(&name), Value::Num(3.0), Some(AbsoluteDeadline::new(at))); }
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
            control.book_timed(key("radio.tx.gain_db"), Value::Num(3.0), at);
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
        device.tx_open(1).unwrap();
        let (mut control, _tx) = control(&core, &[Dir::Tx]);
        control.book_cold(key("radio.tx.channels"), Value::Int(0), Some(AbsoluteDeadline::new(core.at(core.ticks(100_000_000)))));
        for (ms, gain) in [(50, 1.0), (50, 2.0), (150, 4.0)] {
            control.book_timed(key("radio.tx.gain_db"), Value::Num(gain), Some(AbsoluteDeadline::new(core.at(core.ticks(ms * 1_000_000)))));
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
            match dir { Dir::Rx => device.rx_open(1).unwrap(), Dir::Tx => device.tx_open(1).unwrap() }
            let (mut control, _tx) = control(&core, &[dir]);
            let ms = |n: i64| core.ticks(n * 1_000_000);
            let at = |n: i64| Some(AbsoluteDeadline::new(core.at(ms(n))));
            let rate = key(&format!("radio.{}.sample_rate_hz", dir.name()));
            let gain = key(&format!("radio.{}.gain_db", dir.name()));
            if halt {
                control.book_cold(rate.clone(), Value::Num(2e6), at(100));
                let refused = plan_of(&core, dir)[1].segment.by.unwrap();
                assert!(lock(&core.streams).refuse(&core, dir, refused), "{case}");
            } else {
                control.book(Action::Stop { target: Some(core.rx_id.clone()) });
            }
            control.book_timed(gain.clone(), Value::Num(3.0), at(200));
            match enable {
                "cold" => control.book_cold(rate.clone(), Value::Num(1e6), at(300)),
                _ => control.book(Action::PeripheralCommand {
                    target: core.rx_id.clone(),
                    verb: ezsdr_kernel::spec::Ident::parse(ezsdr_radio::START_RX).unwrap(),
                    params: BTreeMap::new(),
                    at: at(300),
                }),
            }
            time.advance_to(core.at(ms(199))).unwrap();
            control.release();
            let applied = device.calls().iter().filter(|call| call.starts_with(&format!("apply {}", dir.name())) && call.contains("gain=3")).count();
            assert_eq!(applied, usize::from(!halt), "{case}: {:?}", device.calls());
            assert!(lock(&core.rec).applied.iter().any(|r| r["key"] == gain.as_str() && (r["note"] == "no channel") == halt), "{case}");
            assert_eq!(control.config[&gain], Value::Num(3.0), "{case}");
            if halt {
                assert_eq!(control.config_at(ms(200))[&rate], Value::Num(2e6), "{case}: the refused value is of record");
            }
            let planned = plan_of(&core, dir).pop().unwrap();
            assert!(planned.segment.origin >= ms(300), "{case}");
            let settings = planned.settings.unwrap();
            assert_eq!(settings.settings(&lock(&settings.updates), planned.segment.origin).gain, Some(3.0), "{case}");
        }
    }

    #[test]
    fn ur_24_a_halted_direction_fills_no_queue_slot() {
        // #64, UR-24: an update for a direction halted by a refusal — from the refusal's own
        // instant on — counts no channel against the queue depth, neither as the new command
        // nor while held, so a full queue still takes it.
        let (core, device, _time) = rig();
        device.rx_open(1).unwrap();
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let ms = |n: i64| core.ticks(n * 1_000_000);
        let gain = key("radio.rx.gain_db");
        control.book_cold(key("radio.rx.sample_rate_hz"), Value::Num(2e6), Some(AbsoluteDeadline::new(core.at(ms(100)))));
        let refused = plan_of(&core, Dir::Rx)[1].segment.by.unwrap();
        assert!(lock(&core.streams).refuse(&core, Dir::Rx, refused));
        for i in 0..core.description.timing.command_queue_depth {
            control.book_timed(gain.clone(), Value::Num(1.0), Some(AbsoluteDeadline::new(core.at(ms(50) + i))));
        }
        for t in [ms(100), ms(200)] {
            control.book_timed(gain.clone(), Value::Num(2.0), Some(AbsoluteDeadline::new(core.at(t))));
        }
        assert!(lock(&core.rec).rejected.is_empty(), "{:?}", lock(&core.rec).rejected);
        assert_eq!(control.held.len(), core.description.timing.command_queue_depth as usize + 2);
    }

    #[test]
    fn ur_24_a_late_update_halted_before_it_can_act_has_no_channel() {
        // #64, UR-24: an update late behind a released one acts at that one's instant; a refusal
        // between its own instant and then leaves it no channel, so it is recorded, not issued.
        let (core, device, time) = rig();
        device.rx_open(1).unwrap();
        let (mut control, _tx) = control(&core, &[Dir::Rx]);
        let us = |n: i64| core.ticks(n * 1_000);
        let at = |n: i64| Some(AbsoluteDeadline::new(core.at(us(n))));
        let gain = key("radio.rx.gain_db");
        control.book_cold(key("radio.rx.sample_rate_hz"), Value::Num(2e6), at(300_000));
        let refused = plan_of(&core, Dir::Rx)[1].segment.by.unwrap();
        control.book_timed(gain.clone(), Value::Num(1.0), at(300_000));
        time.advance_to(core.at(us(297_500))).unwrap();
        control.release();
        assert!(lock(&core.streams).refuse(&core, Dir::Rx, refused));
        control.book_timed(gain.clone(), Value::Num(2.0), at(299_600));
        control.release();
        let rows: Vec<_> = lock(&core.rec).applied.iter().filter(|r| r["key"] == "radio.rx.gain_db")
            .map(|r| (r["claimed"].as_f64().unwrap(), r["issued"] == true, r["note"] == "no channel")).collect();
        assert_eq!(rows, [(1.0, true, false), (2.0, false, true)]);
        assert!(!device.calls().iter().any(|call| call.starts_with("apply rx") && call.contains("gain=2")), "{:?}", device.calls());
    }
}
