//! uhd-control (UR-14): drains the `ActionReceiver` with MA-14b's loop, books each
//! Action and hands device work to its owner, releases held timed commands, and
//! every 500 ms checks the reference and reads the device's time. It never waits
//! for a device instant.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration as Wall, Instant};

use ezsdr_kernel::event::{Action, Severity};
use ezsdr_kernel::id::ClockDomainId;
use ezsdr_kernel::module_api::{ActionReceiver, Requested, UpdateClass};
use ezsdr_kernel::spec::{Constraint, Key, Value};
use ezsdr_kernel::stream::{BurstOpen, LateOutcome};
use ezsdr_kernel::time::{AbsoluteDeadline, Duration, TimePoint, TimeError};
use ezsdr_radio::payloads::{
    ClockLostPayload, ClockReference, CommandQueueFullPayload, LateCommandPayload, TimeErrorCause,
    TimeErrorOutcome, TimeErrorPayload,
};
use ezsdr_radio::{keys, kinds};
use serde_json::json;

use super::core::{Core, lattice, lock};
use super::rx::RxCmd;
use super::tx::{Held, TxCmd};
use crate::device::{DeviceError, Dir, Iq, Settings, decimation};
use crate::profile::{DELIVERY_ALLOWANCE_NS, DEVICE_LEAD_NS, RELEASE_WINDOW_NS, RESTART_LEAD_NS};

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

/// A cold switch's base configuration and timed updates, shared with its owner.
/// Updates are projected in effective order at e2, even when admitted after booking.
pub(crate) struct ColdConfig {
    e2: i64,
    base: Settings,
    updates: Mutex<BTreeMap<(i64, u64), Settings>>,
}

impl ColdConfig {
    pub fn new(e2: i64, base: Settings) -> Self {
        Self { e2, base, updates: Mutex::new(BTreeMap::new()) }
    }

    fn book(&self, order: (i64, u64), settings: Settings) {
        if order.0 <= self.e2 {
            lock(&self.updates).insert(order, settings);
        }
    }

    fn issued(&self, updates: &mut BTreeMap<(i64, u64), Settings>, order: (i64, u64), effective: i64, settings: Settings) {
        updates.remove(&order);
        if effective <= self.e2 {
            updates.insert((effective, order.1), settings);
        }
    }

    fn settings(&self, updates: &BTreeMap<(i64, u64), Settings>) -> Settings {
        let mut settings = self.base.clone();
        for update in updates.values() {
            if let Some(freq) = update.freq { settings.freq = Some(freq); }
            if let Some(gain) = update.gain { settings.gain = Some(gain); }
        }
        settings
    }

    pub fn configure(&self, core: &Core, dir: Dir, channels: usize) -> Result<i64, DeviceError> {
        // Hold the snapshot lock across the call: an update cannot change the
        // projection while an owner is configuring from it.
        let updates = lock(&self.updates);
        core.configure(dir, channels, &self.settings(&updates))
    }
}

pub(crate) struct Control {
    pub core: Arc<Core>,
    pub actions: Arc<dyn ActionReceiver>,
    pub to_tx: Sender<TxCmd>,
    pub to_rx: Sender<RxCmd>,
    pub config: BTreeMap<Key, Value>,
    pub reference_monitored: bool,
    held: BTreeMap<(i64, u64), Timed>,
    cold: [Option<Arc<ColdConfig>>; 2],
    released: Vec<i64>,
    seq: u64,
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

impl Control {
    pub fn new(
        core: Arc<Core>,
        actions: Arc<dyn ActionReceiver>,
        to_tx: Sender<TxCmd>,
        to_rx: Sender<RxCmd>,
        config: BTreeMap<Key, Value>,
        reference_monitored: bool,
    ) -> Control {
        Control {
            core,
            actions,
            to_tx,
            to_rx,
            config,
            reference_monitored,
            held: BTreeMap::new(),
            cold: [None, None],
            released: Vec::new(),
            seq: 0,
            clock_lost: false,
            failing_since: None,
        }
    }

    pub fn run(mut self, stop: Arc<AtomicBool>) {
        let mut last_check = Instant::now();
        while !stop.load(Ordering::Acquire) {
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
        self.cancel_held("Provider::stop");
    }

    fn book(&mut self, action: Action) {
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
            Action::Stop { target: Some(target) } => {
                if target == self.core.tx_id {
                    let _ = self.to_tx.send(TxCmd::Stop);
                } else if target == self.core.rx_id {
                    let _ = self.to_rx.send(RxCmd::Stop);
                } else if target == self.core.id {
                    self.cancel_held("Stop");
                    let _ = self.to_tx.send(TxCmd::Stop);
                    let _ = self.to_rx.send(RxCmd::Stop);
                } else {
                    self.core.command_rejected("stop", "UR-26: the Stop target is not this device or its streams");
                }
            }
            Action::Stop { target: None } | Action::Abort { .. } => {}
            Action::SetTimer { .. } => self.core.command_rejected("set_timer", "UR-26: not supported by this Provider"),
            Action::PeripheralCommand { .. } => {
                self.core.command_rejected("peripheral_command", "UR-26: not supported by this Provider")
            }
            Action::Emit { .. } => self.core.command_rejected("emit", "UR-26: not supported by this Provider"),
        }
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
        if Core::channels(&self.config, dir) == 0 {
            // No channel: recorded in the configuration; enabling applies it (UR-24).
            self.config.insert(key.clone(), value);
            lock(&self.core.rec).applied.push(json!({ "key": key, "claimed": v, "read_back": null, "at": self.core.at(e), "note": "no channel" }));
            return;
        }
        self.released.retain(|effective| *effective > now);
        let depth = self.core.description.timing.command_queue_depth as usize;
        // The device queue holds one command per channel (Review L, NONBLOCKING 6).
        let pending: usize = self.held.values().map(|t| Core::channels(&self.config, t.dir)).sum::<usize>() + self.released.len();
        if pending + Core::channels(&self.config, dir) > depth {
            let payload = serde_json::to_value(CommandQueueFullPayload { key: key.clone(), depth: depth as i64 }).expect("a payload");
            self.core.emit(&self.core.id, kinds::COMMAND_QUEUE_FULL, Severity::Error, payload);
            self.core.reject_note(json!({ "action": "update_parameter", "reason": "UR-24: the command queue is full", "key": key }));
            return;
        }
        self.seq += 1;
        let timed = Timed { key, dir, value: v };
        if let Some(cold) = self.pending_cold(dir) { cold.book((e, self.seq), timed.settings()); }
        self.held.insert((e, self.seq), timed);
    }

    /// Releases held commands within the release window, in effective order (UR-24).
    fn release(&mut self) {
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
            if effective > e {
                // The device queue is in order: behind a later command it is late (UC-2).
                self.late_command(Some(timed.key.clone()), e, effective);
            }
            let settings = timed.settings();
            let cold = self.pending_cold(timed.dir);
            // Order a timed issuance and its projection atomically against configure.
            let mut updates = cold.as_ref().map(|cold| lock(&cold.updates));
            for chan in 0..Core::channels(&self.config, timed.dir) {
                if let Err(error) = self.core.device.apply(timed.dir, chan, &settings, Some(effective)) {
                    self.core.device_failed("update_parameter", &error);
                    return;
                }
                self.released.push(effective);
            }
            if let (Some(cold), Some(updates)) = (&cold, &mut updates) {
                cold.issued(updates, (e, seq), effective, settings);
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
            let cold = self.pending_cold(timed.dir);
            let mut updates = cold.as_ref().map(|cold| lock(&cold.updates));
            if let Some(updates) = &mut updates { updates.remove(&(e, seq)); }
            // Keep the projection locked until its cancellation is recorded, so an
            // owner can never apply this held value after that record.
            lock(&self.core.rec).applied.push(json!({
                "key": timed.key, "claimed": timed.value, "at": self.core.at(e), "cancelled": why, "cancelled_at": at,
            }));
        }
    }

    // ------------------------------------------------------------ UR-25

    fn book_cold(&mut self, key: Key, value: Value, at: Option<AbsoluteDeadline>) {
        let dir = if key.as_str().starts_with("radio.rx.") { Dir::Rx } else { Dir::Tx };
        {
            let streams = lock(&self.core.streams);
            if streams.switching[dir as usize] {
                let disabled = match dir { Dir::Rx => streams.rx, Dir::Tx => streams.tx }.is_none();
                drop(streams);
                return self.core.command_rejected("update_parameter", if disabled {
                    "UR-25: the stream changed to 0 channels is still draining; wait for its pending cold change"
                } else {
                    "UR-25: a cold change is still pending for this direction"
                });
            }
        }
        let mut candidate = self.config.clone();
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
        candidate.insert(key.clone(), value);
        let channels = Core::channels(&candidate, dir);
        let rate = Core::settings(&candidate, dir).rate.unwrap_or(0.0);
        let n = if channels > 0 {
            match decimation(self.core.mcr, rate) {
                Some(n) => n as i64,
                None => {
                    return self.core.command_rejected("update_parameter", &format!("UR-12: {rate} S/s is no decimation 1…512 of {} Hz", self.core.mcr));
                }
            }
        } else {
            0
        };
        let old = {
            let streams = lock(&self.core.streams);
            match dir {
                Dir::Rx => streams.rx,
                Dir::Tx => streams.tx,
            }
        };
        let now = self.core.now();
        let requested = match self.ceil_root(at) {
            Ok(requested) => requested,
            Err(error) => return self.core.command_rejected("update_parameter", &format!("UR-25: explicit deadline cannot be converted: {error}")),
        };
        match old {
            None if channels > 0 && lock(&self.core.streams).draining[dir as usize].is_some_and(|e1| now < e1) => {
                // The old stream still runs to its e₁ (Review M, N-4).
                self.core.command_rejected("update_parameter", "UR-25: the stream changed to 0 channels is still draining; enable it after its e₁");
            }
            None if channels > 0 && (dir == Dir::Tx || !self.core.links.is_empty()) => {
                if let Some(t) = requested.filter(|t| *t < now) {
                    self.late_command(Some(key.clone()), t, now);
                }
                let e = requested.unwrap_or(now).max(now);
                let minimum = match dir {
                    Dir::Tx => i128::from(e),
                    Dir::Rx => i128::from(e).max(i128::from(now) + i128::from(self.core.ticks(RESTART_LEAD_NS))),
                };
                if let Err(error) = lattice(minimum, n) {
                    return self.core.command_rejected("update_parameter", &format!("UR-25: {error}"));
                }
                self.config = candidate;
                self.enable(dir, channels, n, e);
            }
            None => self.config = candidate,
            Some(old) => {
                let restart = self.core.ticks(RESTART_LEAD_NS);
                // A receive stream's e₁ also waits for the receive call already in progress,
                // which asks for a whole block and is not bounded by the cut: the longer of a
                // block and a packet, plus the delivery of its last packet (Review O, O-B2).
                // The call can end up to one packet more after it began, its block's last
                // sample in a packet that ends later; the restart lead's margin covers that
                // (a packet is at most 5.1 ms on the X300; Review P, NB-4).
                let in_progress = match dir {
                    Dir::Rx => self.core.block_len.max(self.core.device.rx_packet_samples()) as i128 * i128::from(old.n) + i128::from(self.core.ticks(DELIVERY_ALLOWANCE_NS)),
                    Dir::Tx => 0,
                };
                let earliest = match i64::try_from(i128::from(now) + i128::from(restart) + in_progress) {
                    Ok(earliest) => earliest,
                    Err(_) => return self.core.command_rejected("update_parameter", "UR-25: time arithmetic overflow"),
                };
                let e = match requested {
                    Some(t) if t < earliest => {
                        self.late_command(Some(key.clone()), t, earliest);
                        earliest
                    }
                    Some(t) => t,
                    None => earliest,
                };
                // Preflight both boundaries before ending the old clock or booking a new one.
                let boundaries = lattice(i128::from(e), old.n).and_then(|e1| {
                    let e2 = if channels > 0 {
                        Some(lattice(i128::from(e1) + i128::from(restart), n)?)
                    } else { None };
                    Ok((e1, e2))
                });
                let (e1, e2) = match boundaries {
                    Ok(boundaries) => boundaries,
                    Err(error) => return self.core.command_rejected("update_parameter", &format!("UR-25: {error}")),
                };
                if let Err(error) = self.core.clocks.end(old.domain, self.core.at(e1)) {
                    return self.core.command_rejected("update_parameter", &format!("UR-25: {error}"));
                }
                let clock = if let Some(e2) = e2 {
                    match self.core.register(dir, n, e2) {
                        Ok(clock) => Some(clock),
                        Err(error) => return self.core.command_rejected("update_parameter", &format!("UR-25: {error}")),
                    }
                } else {
                    None
                };
                self.config = candidate;
                let settings = Arc::new(ColdConfig::new(clock.map_or(e1, |c| c.origin), Core::settings(&self.config, dir)));
                for (&order, timed) in &self.held {
                    if timed.dir == dir { settings.book(order, timed.settings()); }
                }
                self.cold[dir as usize] = Some(settings.clone());
                {
                    let mut streams = lock(&self.core.streams);
                    streams.switching[dir as usize] = true;
                    streams.draining[dir as usize] = clock.is_none().then_some(e1);
                    match dir {
                        Dir::Rx => streams.rx = clock,
                        Dir::Tx => {
                            streams.tx = clock;
                            streams.tx_channels = channels;
                        }
                    }
                }
                self.core.timing(json!({ "what": "cold_change", "key": key, "e1": e1, "e2": clock.map(|c| c.origin), "booked_at": now }));
                match dir {
                    Dir::Rx => drop(self.to_rx.send(RxCmd::Switch { e1, clock, channels, settings })),
                    Dir::Tx => drop(self.to_tx.send(TxCmd::Switch { e1, clock, channels, settings })),
                }
            }
        }
    }

    fn pending_cold(&mut self, dir: Dir) -> Option<Arc<ColdConfig>> {
        if !lock(&self.core.streams).switching[dir as usize] {
            self.cold[dir as usize] = None;
        }
        self.cold[dir as usize].clone()
    }

    /// A change from 0 channels (UR-25; N-P0-1): uhd-control opens the streamer and
    /// configures the direction itself, then registers the clock and hands over.
    fn enable(&mut self, dir: Dir, channels: usize, n: i64, e: i64) {
        let opened = match dir {
            Dir::Rx => self.core.device.rx_open(channels),
            Dir::Tx => self.core.device.tx_open(channels),
        };
        let settings = Core::settings(&self.config, dir);
        let configured = opened.and_then(|()| self.core.configure(dir, channels, &settings));
        if let Err(error) = configured {
            return self.core.device_failed("update_parameter", &error);
        }
        // RM-25: at or after both `e` and the end of the configuration; a receive stream
        // also a restart lead ahead, for its timed start (Review L, P0-3).
        let now = self.core.now();
        let origin = match lattice(match dir {
            Dir::Tx => i128::from(e.max(now)),
            Dir::Rx => i128::from(e).max(i128::from(now) + i128::from(self.core.ticks(RESTART_LEAD_NS))),
        }, n) {
            Ok(origin) => origin,
            Err(error) => return self.core.command_rejected("update_parameter", &format!("UR-25: {error}")),
        };
        let clock = match self.core.register(dir, n, origin) {
            Ok(clock) => clock,
            Err(error) => return self.core.command_rejected("update_parameter", &format!("UR-25: {error}")),
        };
        if dir == Dir::Rx {
            if let Err(error) = self.core.device.rx_start(origin) {
                return self.core.device_failed("update_parameter", &error);
            }
        }
        {
            let mut streams = lock(&self.core.streams);
            match dir {
                Dir::Rx => streams.rx = Some(clock),
                Dir::Tx => {
                    streams.tx = Some(clock);
                    streams.tx_channels = channels;
                }
            }
        }
        self.core.timing(json!({ "what": "enabled", "dir": dir.name(), "origin": origin, "channels": channels }));
        match dir {
            Dir::Rx => drop(self.to_rx.send(RxCmd::Enable { clock, channels })),
            Dir::Tx => drop(self.to_tx.send(TxCmd::Enable { clock, channels })),
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
        let (clock, channels) = {
            let streams = lock(&self.core.streams);
            (streams.tx, streams.tx_channels)
        };
        let Some(clock) = clock.filter(|_| channels > 0) else {
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
        let now = self.core.now().max(clock.origin - lead);
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
mod tests {
    use super::*;
    use crate::device::Device;
    struct NoActions;
    impl ActionReceiver for NoActions {
        fn recv(&self) -> Option<Action> { None }
    }

    #[test]
    fn ur_14_booking_does_not_wait_for_a_future_cold_switch() {
        let (core, device, _, _) = crate::provider::test_support::rig();
        let clock = core.register(Dir::Rx, 200, 0).unwrap();
        lock(&core.streams).rx = Some(clock);
        device.rx_open(1).unwrap();
        let (to_tx, _tx) = std::sync::mpsc::channel();
        let (to_rx, rx) = std::sync::mpsc::channel();
        let mut config = core.description.defaults.clone();
        config.insert(super::super::core::key("radio.rx.channels"), Value::Int(1));
        let mut control = Control::new(core.clone(), Arc::new(NoActions), to_tx, to_rx, config, false);
        control.book_cold(super::super::core::key("radio.rx.sample_rate_hz"), Value::Num(2e6),
            Some(AbsoluteDeadline::new(core.at(core.ticks(1_000_000_000)))));
        control.book_timed(super::super::core::key("radio.rx.gain_db"), Value::Num(3.0), None);
        control.release();
        assert_eq!(core.now(), 0, "booking never advanced to the future device instant");
        assert!(matches!(rx.try_recv().unwrap(), RxCmd::Switch { e1, .. } if e1 == core.ticks(1_000_000_000)));
        assert!(lock(&core.rec).applied.iter().any(|r|
            r["key"] == "radio.rx.gain_db" && r["issued"] == true && r["at"]["ticks"] == core.ticks(2_000_000)));
        assert!(lock(&core.rec).rejected.is_empty());
    }

    #[test]
    fn ur_25_overlapping_cold_changes_are_refused_before_bookkeeping() {
        for dir in [Dir::Rx, Dir::Tx] {
            let (core, _, time, _) = crate::provider::test_support::rig();
            let clock = core.register(dir, 200, 0).unwrap();
            { let mut streams = lock(&core.streams);
                match dir { Dir::Rx => streams.rx = Some(clock), Dir::Tx => streams.tx = Some(clock) }
            }
            let (to_tx, tx) = std::sync::mpsc::channel();
            let (to_rx, rx) = std::sync::mpsc::channel();
            let mut config = core.description.defaults.clone();
            config.insert(super::super::core::key(&format!("radio.{}.channels", dir.name())), Value::Int(1));
            let mut control = Control::new(core.clone(), Arc::new(NoActions), to_tx, to_rx, config, false);
            let key = super::super::core::key(&format!("radio.{}.sample_rate_hz", dir.name()));
            control.book_cold(key.clone(), Value::Num(2e6), None);
            let records = core.clocks.sample_clock_records();
            assert_eq!(records.len(), 2);
            assert!(lock(&core.streams).switching[dir as usize]);
            control.book_cold(key.clone(), Value::Num(4e6), None);
            // Even after e2, completion is the owner's, not an inference from now.
            time.advance_to(core.at(records[1].origin.ticks + core.ticks(1_000_000))).unwrap();
            control.book_cold(key.clone(), Value::Num(4e6), None);
            assert_eq!(core.clocks.sample_clock_records(), records);
            assert_eq!(control.config[&key], Value::Num(2e6));
            assert_eq!(lock(&core.rec).rejected.len(), 2);
            match dir {
                Dir::Rx => { assert!(matches!(rx.try_recv().unwrap(), RxCmd::Switch { .. })); assert!(rx.try_recv().is_err()); }
                Dir::Tx => { assert!(matches!(tx.try_recv().unwrap(), TxCmd::Switch { .. })); assert!(tx.try_recv().is_err()); }
            }
        }
    }
    #[test]
    fn ur_25_later_timed_update_survives_cold_switch() {
        let mut observed = Vec::new();
        for dir in [Dir::Rx, Dir::Tx] {
            let (core, device, _, _) = crate::provider::test_support::rig();
            let clock = core.register(dir, 200, 0).unwrap();
            { let mut streams = lock(&core.streams);
              match dir { Dir::Rx => streams.rx = Some(clock), Dir::Tx => streams.tx = Some(clock) }
            }
            match dir { Dir::Rx => device.rx_open(1).unwrap(), Dir::Tx => device.tx_open(1).unwrap() }
            let (to_tx, tx) = std::sync::mpsc::channel();
            let (to_rx, rx) = std::sync::mpsc::channel();
            let mut config = core.description.defaults.clone();
            config.insert(super::super::core::key(&format!("radio.{}.channels", dir.name())), Value::Int(1));
            let mut control = Control::new(core.clone(), Arc::new(NoActions), to_tx, to_rx, config, false);
            control.book_cold(super::super::core::key(&format!("radio.{}.sample_rate_hz", dir.name())), Value::Num(2e6),
                Some(AbsoluteDeadline::new(core.at(core.ticks(1_000_000_000)))));
            let gain = super::super::core::key(&format!("radio.{}.gain_db", dir.name()));
            control.book_timed(gain.clone(), Value::Num(3.0), None);
            control.release();
            assert_eq!(control.config[&gain], Value::Num(3.0));
            assert!(lock(&core.rec).rejected.is_empty());
            let settings = match dir {
                Dir::Rx => match rx.try_recv().unwrap() { RxCmd::Switch { settings, .. } => settings, _ => panic!("switch") },
                Dir::Tx => match tx.try_recv().unwrap() { TxCmd::Switch { settings, .. } => settings, _ => panic!("switch") },
            };
            settings.configure(&core, dir, 1).unwrap();
            eprintln!("device calls: {:?}", device.calls());
            observed.push((dir.name(), settings.settings(&lock(&settings.updates)).gain));
        }
        assert_eq!(observed, vec![("rx", Some(3.0)), ("tx", Some(3.0))], "both cold switches must preserve the later update due before e2");
    }

    #[test]
    fn ur_25_switch_projects_timed_updates_in_effective_order() {
        let cold = ColdConfig::new(100, Settings { gain: Some(0.0), freq: Some(1e9), ..Settings::default() });
        cold.book((70, 1), Settings { gain: Some(7.0), ..Settings::default() });
        cold.book((50, 2), Settings { gain: Some(5.0), ..Settings::default() });
        cold.book((101, 3), Settings { gain: Some(9.0), freq: Some(2e9), ..Settings::default() });
        cold.book((100, 4), Settings { freq: Some(1.1e9), ..Settings::default() });
        assert_eq!(cold.settings(&lock(&cold.updates)).gain, Some(7.0));
        assert_eq!(cold.settings(&lock(&cold.updates)).freq, Some(1.1e9));
        // A command delayed by the device queue takes its actual effective order.
        cold.issued(&mut lock(&cold.updates), (50, 2), 80, Settings { gain: Some(5.0), ..Settings::default() });
        assert_eq!(cold.settings(&lock(&cold.updates)).gain, Some(5.0));
        cold.issued(&mut lock(&cold.updates), (100, 4), 101, Settings { freq: Some(1.1e9), ..Settings::default() });
        assert_eq!(cold.settings(&lock(&cold.updates)).freq, Some(1e9));
    }

    #[test]
    fn ur_26_device_stop_removes_cancelled_timed_gain_from_switch() {
        let (core, device, _, _) = crate::provider::test_support::rig();
        let old = core.register(Dir::Rx, 200, 0).unwrap();
        lock(&core.streams).rx = Some(old);
        device.rx_open(1).unwrap();
        let (to_tx, _tx) = std::sync::mpsc::channel();
        let (to_rx, rx) = std::sync::mpsc::channel();
        let mut config = core.description.defaults.clone();
        config.insert(super::super::core::key("radio.rx.channels"), Value::Int(1));
        let mut control = Control::new(core.clone(), Arc::new(NoActions), to_tx, to_rx, config, false);
        let gain = super::super::core::key("radio.rx.gain_db");
        control.book_timed(gain, Value::Num(3.0), Some(AbsoluteDeadline::new(core.at(core.ticks(1_040_000_000)))));
        control.book_cold(super::super::core::key("radio.rx.sample_rate_hz"), Value::Num(2e6),
            Some(AbsoluteDeadline::new(core.at(core.ticks(1_000_000_000)))));
        control.book(Action::Stop { target: Some(core.id.clone()) });
        assert!(control.held.is_empty());
        assert!(lock(&core.rec).applied.iter().any(|r| r["key"] == "radio.rx.gain_db" && r["cancelled"] == "Stop"));
        let RxCmd::Switch { settings, .. } = rx.try_recv().unwrap() else { panic!("Switch"); };
        assert!(matches!(rx.try_recv().unwrap(), RxCmd::Stop));
        settings.configure(&core, Dir::Rx, 1).unwrap();
        eprintln!("cancelled gain is still in Switch: {:?}; device calls: {:?}", settings.settings(&lock(&settings.updates)).gain, device.calls());
        assert_eq!(settings.settings(&lock(&settings.updates)).gain, Some(0.0), "device Stop must not leave a switch applying the timed gain just cancelled");
    }

    #[test]
    fn ur_26_cancellation_preserves_issued_updates_in_both_directions() {
        for dir in [Dir::Rx, Dir::Tx] {
            for field in ["gain_db", "frequency_hz"] {
                for before_switch in [false, true] {
                    let (core, device, _, _) = crate::provider::test_support::rig();
                    let clock = core.register(dir, 200, 0).unwrap();
                    { let mut streams = lock(&core.streams);
                        match dir { Dir::Rx => streams.rx = Some(clock), Dir::Tx => streams.tx = Some(clock) }
                    }
                    match dir { Dir::Rx => device.rx_open(1).unwrap(), Dir::Tx => device.tx_open(1).unwrap() }
                    let (to_tx, tx) = std::sync::mpsc::channel();
                    let (to_rx, rx) = std::sync::mpsc::channel();
                    let mut config = core.description.defaults.clone();
                    config.insert(super::super::core::key(&format!("radio.{}.channels", dir.name())), Value::Int(1));
                    let mut control = Control::new(core.clone(), Arc::new(NoActions), to_tx, to_rx, config, false);
                    let key = super::super::core::key(&format!("radio.{}.{field}", dir.name()));
                    let (issued, cancelled) = if field == "gain_db" { (1.0, 3.0) } else { (1.1e9, 1.2e9) };
                    if before_switch {
                        control.book_timed(key.clone(), Value::Num(issued), None);
                        control.release();
                        control.book_timed(key.clone(), Value::Num(cancelled), Some(AbsoluteDeadline::new(core.at(core.ticks(1_040_000_000)))));
                    }
                    control.book_cold(super::super::core::key(&format!("radio.{}.sample_rate_hz", dir.name())), Value::Num(2e6),
                        Some(AbsoluteDeadline::new(core.at(core.ticks(1_000_000_000)))));
                    if !before_switch {
                        control.book_timed(key.clone(), Value::Num(issued), None);
                        control.release();
                        control.book_timed(key.clone(), Value::Num(cancelled), Some(AbsoluteDeadline::new(core.at(core.ticks(1_040_000_000)))));
                    }
                    // Stream Stop keeps timed commands; device Stop cancels only held ones.
                    let target = match dir { Dir::Rx => core.rx_id.clone(), Dir::Tx => core.tx_id.clone() };
                    control.book(Action::Stop { target: Some(target) });
                    assert_eq!(control.held.len(), 1);
                    control.book(Action::Stop { target: Some(core.id.clone()) });
                    assert!(control.held.is_empty());
                    let settings = match dir {
                        Dir::Rx => match rx.try_recv().unwrap() { RxCmd::Switch { settings, .. } => settings, _ => panic!("switch") },
                        Dir::Tx => match tx.try_recv().unwrap() { TxCmd::Switch { settings, .. } => settings, _ => panic!("switch") },
                    };
                    settings.configure(&core, dir, 1).unwrap();
                    let projected = settings.settings(&lock(&settings.updates));
                    assert_eq!(if field == "gain_db" { projected.gain } else { projected.freq }, Some(issued));
                    assert_eq!(lock(&core.rec).applied.iter().filter(|r| r["key"] == key.as_str() && r["cancelled"] == "Stop").count(), 1);
                    assert!(lock(&core.rec).applied.iter().any(|r| r["key"] == key.as_str() && r["claimed"] == issued && r["issued"] == true));
                }
            }
        }
    }

    #[test]
    fn ur_25_cold_deadline_overflow_is_rejected_without_panic() {
        let aligned_max = i64::MAX.div_euclid(200) * 200;
        for dir in [Dir::Rx, Dir::Tx] {
            for (at, now) in [(i64::MAX, 0), (aligned_max, 0), (0, i64::MAX - 1)] {
                let (mut core, device, _, _) = crate::provider::test_support::rig();
                let time = Arc::new(ezsdr_kernel::time::ManualTimeAuthority::new(core.clocks.clone(), core.root, &[],
                    ezsdr_kernel::module_api::Pacing::Device).unwrap());
                Arc::get_mut(&mut core).unwrap().time = time.clone();
                let old = core.register(dir, 200, 0).unwrap();
                { let mut streams = lock(&core.streams);
                    match dir { Dir::Rx => streams.rx = Some(old), Dir::Tx => streams.tx = Some(old) }
                }
                time.advance_to(core.at(now)).unwrap();
                let (to_tx, tx) = std::sync::mpsc::channel();
                let (to_rx, rx) = std::sync::mpsc::channel();
                let mut config = core.description.defaults.clone();
                config.insert(super::super::core::key(&format!("radio.{}.channels", dir.name())), Value::Int(1));
                let mut control = Control::new(core.clone(), Arc::new(NoActions), to_tx, to_rx, config.clone(), false);
                control.book_cold(super::super::core::key(&format!("radio.{}.sample_rate_hz", dir.name())), Value::Num(2e6),
                    Some(AbsoluteDeadline::new(core.at(at))));
                assert_eq!(control.config, config);
                assert_eq!(core.clocks.sample_clock_records().len(), 1);
                assert_eq!(core.clocks.sample_clock_records()[0].ended_at, None);
                assert!(lock(&core.rec).rejected.iter().any(|row| row["reason"].as_str().unwrap().contains("overflow")));
                assert!(tx.try_recv().is_err()); assert!(rx.try_recv().is_err());
                assert!(device.calls().is_empty());
            }
        }
        assert_eq!(super::lattice(i128::from(aligned_max), 200).unwrap(), aligned_max);
        assert_eq!(super::lattice(-201, 200).unwrap(), -200);
        assert_eq!(super::lattice(i128::from(i64::MIN), 200).unwrap(), i64::MIN + 8);
        // A cold disable needs e1 only, so the largest aligned instant remains valid.
        let (core, _, _, _) = crate::provider::test_support::rig();
        let old = core.register(Dir::Tx, 200, 0).unwrap();
        lock(&core.streams).tx = Some(old);
        let (to_tx, tx) = std::sync::mpsc::channel(); let (to_rx, _rx) = std::sync::mpsc::channel();
        let mut config = core.description.defaults.clone();
        config.insert(super::super::core::key("radio.tx.channels"), Value::Int(1));
        let mut control = Control::new(core.clone(), Arc::new(NoActions), to_tx, to_rx, config, false);
        control.book_cold(super::super::core::key("radio.tx.channels"), Value::Int(0),
            Some(AbsoluteDeadline::new(core.at(aligned_max))));
        assert_eq!(core.clocks.sample_clock_records()[0].ended_at, Some(core.at(aligned_max)));
        assert!(matches!(tx.try_recv(), Ok(TxCmd::Switch { clock: None, .. })));
    }

    #[test]
    fn ur_24_invalid_explicit_deadline_is_not_asap() {
        for dir in [Dir::Rx, Dir::Tx] {
            for cold in [false, true] {
                for channels in [0, 1] {
                    for invalid in 0..3 {
                        let (core, device, _, _) = crate::provider::test_support::rig();
                        let old = core.register(dir, 200, 0).unwrap();
                        if channels > 0 {
                            let mut streams = lock(&core.streams);
                            match dir { Dir::Rx => streams.rx = Some(old), Dir::Tx => streams.tx = Some(old) }
                        }
                        let at = match invalid {
                            0 => TimePoint::new(ClockDomainId::HOST_MONOTONIC, 1_000_000_000),
                            1 => TimePoint::new(ClockDomainId::local(999), 10),
                            _ => TimePoint::new(old.domain, i64::MAX),
                        };
                        let (to_tx, tx) = std::sync::mpsc::channel(); let (to_rx, rx) = std::sync::mpsc::channel();
                        let mut config = core.description.defaults.clone();
                        config.insert(super::super::core::key(&format!("radio.{}.channels", dir.name())), Value::Int(channels));
                        let mut control = Control::new(core.clone(), Arc::new(NoActions), to_tx, to_rx, config.clone(), false);
                        let name = format!("radio.{}.{}", dir.name(), if cold { "sample_rate_hz" } else { "gain_db" });
                        if cold { control.book_cold(super::super::core::key(&name), Value::Num(2e6), Some(AbsoluteDeadline::new(at))); }
                        else { control.book_timed(super::super::core::key(&name), Value::Num(3.0), Some(AbsoluteDeadline::new(at))); }
                        assert!(control.held.is_empty()); assert_eq!(control.config, config);
                        assert!(tx.try_recv().is_err()); assert!(rx.try_recv().is_err());
                        assert_eq!(core.clocks.sample_clock_records().len(), 1);
                        assert_eq!(core.clocks.sample_clock_records()[0].ended_at, None);
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
            let (core, _, _, _) = crate::provider::test_support::rig();
            let derived = core.clocks.allocate_id().unwrap();
            core.clocks.register(ClockDomain::derived(derived, core.root, Rational::new(2, 3).unwrap(), 0)).unwrap();
            let (to_tx, _tx) = std::sync::mpsc::channel(); let (to_rx, _rx) = std::sync::mpsc::channel();
            let mut config = core.description.defaults.clone();
            config.insert(super::super::core::key("radio.tx.channels"), Value::Int(1));
            let mut control = Control::new(core.clone(), Arc::new(NoActions), to_tx, to_rx, config, false);
            let (at, expected) = match case {
                0 => (None, core.ticks(DEVICE_LEAD_NS)),
                1 => (Some(AbsoluteDeadline::new(core.at(600_000))), 600_000),
                _ => (Some(AbsoluteDeadline::new(TimePoint::new(derived, 900_001))), 600_001),
            };
            control.book_timed(super::super::core::key("radio.tx.gain_db"), Value::Num(3.0), at);
            assert_eq!(control.held.keys().map(|order| order.0).collect::<Vec<_>>(), vec![expected]);
            assert!(lock(&core.rec).rejected.is_empty());
        }
    }

}
