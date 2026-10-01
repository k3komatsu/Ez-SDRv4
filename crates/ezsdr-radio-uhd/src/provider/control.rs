//! uhd-control (UR-14): drains the `ActionReceiver` with MA-14b's loop, books each
//! Action and hands device work to its owner, releases held timed commands, and
//! every 500 ms checks the reference and reads the device's time. It never waits
//! for a device instant.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration as Wall, Instant};

use ezsdr_kernel::event::{Action, Severity};
use ezsdr_kernel::id::ClockDomainId;
use ezsdr_kernel::module_api::{ActionReceiver, Requested, UpdateClass};
use ezsdr_kernel::spec::{Constraint, Key, Value};
use ezsdr_kernel::stream::{BurstOpen, LateOutcome};
use ezsdr_kernel::time::{AbsoluteDeadline, Duration, TimePoint};
use ezsdr_radio::payloads::{
    ClockLostPayload, ClockReference, CommandQueueFullPayload, LateCommandPayload, TimeErrorCause,
    TimeErrorOutcome, TimeErrorPayload,
};
use ezsdr_radio::{keys, kinds};
use serde_json::json;

use super::core::{Core, lattice, lock};
use super::rx::RxCmd;
use super::tx::{Held, TxCmd};
use crate::device::{Dir, Iq, Settings, decimation};
use crate::profile::{DELIVERY_ALLOWANCE_NS, DEVICE_LEAD_NS, RELEASE_WINDOW_NS, RESTART_LEAD_NS};

const POLL: Wall = Wall::from_millis(1);
const CHECK: Wall = Wall::from_millis(500);

/// A held `hardware_timed` command (UR-24).
struct Timed {
    key: Key,
    dir: Dir,
    value: f64,
}

pub(crate) struct Control {
    pub core: Arc<Core>,
    pub actions: Arc<dyn ActionReceiver>,
    pub to_tx: Sender<TxCmd>,
    pub to_rx: Sender<RxCmd>,
    pub config: BTreeMap<Key, Value>,
    pub reference_monitored: bool,
    held: BTreeMap<(i64, u64), Timed>,
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
                    let _ = self.to_tx.send(TxCmd::Stop);
                    let _ = self.to_rx.send(RxCmd::Stop);
                    self.cancel_held("Stop");
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

    fn ceil_root(&self, at: Option<AbsoluteDeadline>) -> Option<i64> {
        let t = at?.time_point;
        if t.domain == self.core.root {
            return Some(t.ticks);
        }
        self.core.clocks.convert(t, self.core.root).ok().map(|c| match c {
            ezsdr_kernel::time::Converted::Exact { point } => point.ticks,
            ezsdr_kernel::time::Converted::Inexact { floor, .. } => floor.ticks + 1,
        })
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
        let earliest = now + self.core.ticks(DEVICE_LEAD_NS);
        let e = match self.ceil_root(at) {
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
        self.held.insert((e, self.seq), Timed { key, dir, value: v });
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
            let (e, _) = *entry.key();
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
            let settings = if timed.key.as_str().ends_with("frequency_hz") {
                Settings { freq: Some(timed.value), ..Settings::default() }
            } else {
                Settings { gain: Some(timed.value), ..Settings::default() }
            };
            for chan in 0..Core::channels(&self.config, timed.dir) {
                if let Err(error) = self.core.device.apply(timed.dir, chan, &settings, Some(effective)) {
                    self.core.device_failed("update_parameter", &error);
                    return;
                }
                self.released.push(effective);
            }
            self.config.insert(timed.key.clone(), Value::Num(timed.value));
            lock(&self.core.rec).applied.push(json!({
                "key": timed.key, "claimed": timed.value, "read_back": null, "at": self.core.at(effective), "issued": true,
            }));
        }
    }

    fn cancel_held(&mut self, why: &str) {
        let at = self.core.at(self.core.now());
        for ((e, _), timed) in std::mem::take(&mut self.held) {
            lock(&self.core.rec).applied.push(json!({
                "key": timed.key, "claimed": timed.value, "at": self.core.at(e), "cancelled": why, "cancelled_at": at,
            }));
        }
    }

    // ------------------------------------------------------------ UR-25

    fn book_cold(&mut self, key: Key, value: Value, at: Option<AbsoluteDeadline>) {
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
        let dir = if key.as_str().starts_with("radio.rx.") { Dir::Rx } else { Dir::Tx };
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
        let requested = self.ceil_root(at);
        match old {
            None if channels > 0 && lock(&self.core.streams).draining[dir as usize].is_some_and(|e1| now < e1) => {
                // The old stream still runs to its e₁ (Review M, N-4).
                self.core.command_rejected("update_parameter", "UR-25: the stream changed to 0 channels is still draining; enable it after its e₁");
            }
            None if channels > 0 && (dir == Dir::Tx || !self.core.links.is_empty()) => {
                if let Some(t) = requested.filter(|t| *t < now) {
                    self.late_command(Some(key.clone()), t, now);
                }
                self.config = candidate;
                self.enable(dir, channels, n, requested.unwrap_or(now).max(now));
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
                    Dir::Rx => self.core.block_len.max(self.core.device.rx_packet_samples()) as i64 * old.n + self.core.ticks(DELIVERY_ALLOWANCE_NS),
                    Dir::Tx => 0,
                };
                let earliest = now + restart + in_progress;
                let e = match requested {
                    Some(t) if t < earliest => {
                        self.late_command(Some(key.clone()), t, earliest);
                        earliest
                    }
                    Some(t) => t,
                    None => earliest,
                };
                let e1 = lattice(e, old.n);
                if let Err(error) = self.core.clocks.end(old.domain, self.core.at(e1)) {
                    return self.core.command_rejected("update_parameter", &format!("UR-25: {error}"));
                }
                let clock = if channels > 0 {
                    let e2 = lattice(e1 + restart, n);
                    match self.core.register(dir, n, e2) {
                        Ok(clock) => Some(clock),
                        Err(error) => return self.core.command_rejected("update_parameter", &format!("UR-25: {error}")),
                    }
                } else {
                    None
                };
                self.config = candidate;
                let settings = self.settings_at(dir, clock.map_or(e1, |c| c.origin));
                {
                    let mut streams = lock(&self.core.streams);
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

    /// The direction's configuration in effect at `e2`: the configuration, and every
    /// held command due by then; later ones stay held (UR-25; UC-2).
    fn settings_at(&self, dir: Dir, e2: i64) -> Settings {
        let mut config = self.config.clone();
        for ((e, _), timed) in &self.held {
            if *e <= e2 && timed.dir == dir {
                config.insert(timed.key.clone(), Value::Num(timed.value));
            }
        }
        Core::settings(&config, dir)
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
        let origin = match dir {
            Dir::Tx => lattice(e.max(now), n),
            Dir::Rx => lattice(e.max(now + self.core.ticks(RESTART_LEAD_NS)), n),
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
