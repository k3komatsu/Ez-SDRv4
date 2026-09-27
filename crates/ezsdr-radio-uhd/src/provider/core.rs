//! What the Provider's three threads share (UR-14…UR-30).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use ezsdr_kernel::event::{Event, EventHandle, EventKind, EventSink, Severity};
use ezsdr_kernel::id::{ClockDomainId, ResourceId};
use ezsdr_kernel::module_api::InputStore;
use ezsdr_kernel::spec::{Key, Value};
use ezsdr_kernel::stream::DataLink;
use ezsdr_kernel::time::{ClockRegistry, Rational, TimeAuthority, TimePoint};
use ezsdr_radio::device::DeviceDescription;
use ezsdr_radio::payloads::CommandRejectedPayload;
use ezsdr_radio::{keys, kinds};
use serde_json::{Value as Json, json};

use crate::device::{Device, DeviceError, Dir, Settings, exact_decimation};

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn key(name: &str) -> Key {
    Key::parse(name).expect("a declared radio key")
}

/// A registered stream clock: `N` root ticks per sample from `origin` (UR-12, RM-25).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Clock {
    pub domain: ClockDomainId,
    pub origin: i64,
    pub n: i64,
}

impl Clock {
    /// The root tick of sample `k`.
    pub fn instant(&self, k: i64) -> i64 {
        self.origin + k * self.n
    }

    /// The first sample at or after root tick `t`.
    pub fn at_or_after(&self, t: i64) -> i64 {
        (t - self.origin + self.n - 1).div_euclid(self.n)
    }
}

/// RM-25: the first whole multiple of `n` at or after `t`.
pub(crate) fn lattice(t: i64, n: i64) -> i64 {
    (t + n - 1).div_euclid(n) * n
}

/// uhd-control's view of the two streams, which it books (UR-25).
#[derive(Default)]
pub(crate) struct Streams {
    pub rx: Option<Clock>,
    pub tx: Option<Clock>,
    pub tx_channels: usize,
}

/// Everything the threads record; written to the sections at `cleanup` (UR-30).
pub(crate) struct Records {
    pub applied: Vec<Json>,
    pub bursts: Vec<Json>,
    pub device_async: Vec<Json>,
    pub rejected: Vec<Json>,
    pub timing: Vec<Json>,
    pub stats: BTreeMap<&'static str, i64>,
}

impl Default for Records {
    fn default() -> Self {
        let stats = [
            "rx_blocks",
            "rx_samples",
            "rx_overflows",
            "rx_errors",
            "rx_off_lattice",
            "rx_overlapping",
            "link_drops_seen",
            "tx_bursts",
            "tx_samples",
        ]
        .into_iter()
        .map(|name| (name, 0))
        .collect();
        Records {
            applied: Vec::new(),
            bursts: Vec::new(),
            device_async: Vec::new(),
            rejected: Vec::new(),
            timing: Vec::new(),
            stats,
        }
    }
}

/// The context `prepare` gathered, shared by uhd-control, uhd-rx and uhd-tx.
pub(crate) struct Core {
    pub device: Arc<dyn Device>,
    pub id: ResourceId,
    pub rx_id: ResourceId,
    pub tx_id: ResourceId,
    pub mcr: u64,
    pub root: ClockDomainId,
    pub time: Arc<dyn TimeAuthority>,
    pub clocks: Arc<ClockRegistry>,
    pub events: Arc<dyn EventSink>,
    pub overflow: EventHandle,
    pub inputs: Arc<dyn InputStore>,
    pub links: Vec<Arc<dyn DataLink>>,
    pub description: DeviceDescription,
    pub block_len: usize,
    pub rec: Mutex<Records>,
    pub streams: Mutex<Streams>,
    /// Held burst starts per transmit clock, for UR-21's duplicate refusal.
    pub held: Mutex<BTreeSet<(ClockDomainId, i64)>>,
    /// `S`, the end of the device's start-up (UR-13).
    pub start_up: AtomicI64,
    lost: AtomicBool,
}

impl Core {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        device: Arc<dyn Device>,
        id: ResourceId,
        root: ClockDomainId,
        time: Arc<dyn TimeAuthority>,
        clocks: Arc<ClockRegistry>,
        events: Arc<dyn EventSink>,
        inputs: Arc<dyn InputStore>,
        links: Vec<Arc<dyn DataLink>>,
        description: DeviceDescription,
    ) -> Core {
        let rx_id = id.child("rx").expect("a valid stream id");
        let tx_id = id.child("tx").expect("a valid stream id");
        let overflow = events.resolve(&rx_id, &EventKind::parse(kinds::RX_OVERFLOW).expect("a radio kind"));
        Core {
            mcr: device.master_clock_rate(),
            block_len: description.block_len as usize,
            device,
            id,
            rx_id,
            tx_id,
            root,
            time,
            clocks,
            events,
            overflow,
            inputs,
            links,
            description,
            rec: Mutex::new(Records::default()),
            streams: Mutex::new(Streams::default()),
            held: Mutex::new(BTreeSet::new()),
            start_up: AtomicI64::new(i64::MIN),
            lost: AtomicBool::new(false),
        }
    }

    /// The current instant on the primary root.
    pub fn now(&self) -> i64 {
        self.time.now(self.root).map(|t| t.ticks).unwrap_or(i64::MIN)
    }

    pub fn at(&self, ticks: i64) -> TimePoint {
        TimePoint::new(self.root, ticks)
    }

    /// Nanoseconds as root ticks, rounded up.
    pub fn ticks(&self, ns: i64) -> i64 {
        ((ns as i128 * self.mcr as i128 + 999_999_999) / 1_000_000_000) as i64
    }

    /// Root ticks as nanoseconds.
    pub fn ns(&self, ticks: i64) -> i64 {
        (ticks as i128 * 1_000_000_000 / self.mcr as i128) as i64
    }

    pub fn is_lost(&self) -> bool {
        self.lost.load(Ordering::Acquire)
    }

    pub fn stat(&self, name: &'static str, n: i64) {
        *lock(&self.rec).stats.entry(name).or_default() += n;
    }

    pub fn timing(&self, row: Json) {
        lock(&self.rec).timing.push(row);
    }

    pub fn reject_note(&self, row: Json) {
        lock(&self.rec).rejected.push(row);
    }

    /// A control-path event from `source` at `time` (RM-11, UR-20).
    pub fn emit_at(&self, source: &ResourceId, kind: &str, severity: Severity, payload: Json, time: TimePoint) {
        let event = Event {
            source: source.clone(),
            time,
            severity,
            kind: EventKind::parse(kind).expect("a radio kind"),
            payload,
        };
        if let Err(error) = self.events.emit_control(event) {
            self.reject_note(json!({ "event_not_emitted": kind, "error": error.to_string() }));
        }
    }

    pub fn emit(&self, source: &ResourceId, kind: &str, severity: Severity, payload: Json) {
        self.emit_at(source, kind, severity, payload, self.at(self.now()));
    }

    /// `radio.COMMAND_REJECTED` from the device, and its reason recorded (UR-21, UR-30).
    pub fn command_rejected(&self, action: &str, reason: &str) {
        self.reject_note(json!({ "action": action, "reason": reason, "at": self.at(self.now()) }));
        let payload = serde_json::to_value(CommandRejectedPayload { action: action.to_owned(), reason: reason.to_owned() })
            .expect("a payload");
        self.emit(&self.id, kinds::COMMAND_REJECTED, Severity::Error, payload);
    }

    /// MA-9a: `DEVICE_LOST`, once, from this Provider; then no thread uses the device.
    pub fn device_lost(&self, message: &str) {
        if self.lost.swap(true, Ordering::AcqRel) {
            return;
        }
        self.timing(json!({ "what": "device_lost", "message": message, "at": self.now() }));
        self.emit(
            &self.id,
            EventKind::DEVICE_LOST,
            Severity::Fatal,
            json!({ "message": message }),
        );
    }

    /// A device failure: `DEVICE_LOST` when the device is gone, else the reason.
    pub fn device_failed(&self, action: &str, error: &DeviceError) {
        if error.lost {
            self.device_lost(&error.message);
        } else {
            self.command_rejected(action, &error.message);
        }
    }

    /// A direction's full configuration from a configuration map (UR-12).
    pub fn settings(config: &BTreeMap<Key, Value>, dir: Dir) -> Settings {
        let number = |name: &str| match config.get(&key(&format!("radio.{}.{name}", dir.name()))) {
            Some(Value::Num(v)) => Some(*v),
            Some(Value::Int(v)) => Some(*v as f64),
            _ => None,
        };
        Settings {
            rate: number("sample_rate_hz"),
            freq: number("frequency_hz"),
            gain: number("gain_db"),
            antenna: match config.get(&key(&format!("radio.{}.antenna", dir.name()))) {
                Some(Value::Str(s)) => Some(s.clone()),
                _ => None,
            },
        }
    }

    pub fn channels(config: &BTreeMap<Key, Value>, dir: Dir) -> usize {
        let name = match dir {
            Dir::Rx => keys::RX_CHANNELS,
            Dir::Tx => keys::TX_CHANNELS,
        };
        match config.get(&key(name)) {
            Some(Value::Int(n)) if *n > 0 => *n as usize,
            _ => 0,
        }
    }

    /// **Configuring a direction** (UR-12): the whole configuration on every channel,
    /// untimed, read back; the rate must be exactly the claim, an exact division of
    /// the master clock. Returns that division `N`.
    pub fn configure(&self, dir: Dir, channels: usize, settings: &Settings) -> Result<i64, DeviceError> {
        let claim = settings.rate.unwrap_or(0.0);
        for chan in 0..channels {
            let applied = self.device.apply(dir, chan, settings, None)?;
            let at = self.at(self.now());
            let mut rec = lock(&self.rec);
            for (name, claimed, read) in [
                ("sample_rate_hz", settings.rate, applied.rate),
                ("frequency_hz", settings.freq, applied.freq),
                ("gain_db", settings.gain, applied.gain),
            ] {
                rec.applied.push(json!({
                    "key": format!("radio.{}.{name}", dir.name()), "channel": chan,
                    "claimed": claimed, "read_back": read, "difference": claimed.map(|c| read - c), "at": at,
                }));
            }
            if let Some(antenna) = &settings.antenna {
                rec.applied.push(json!({ "key": format!("radio.{}.antenna", dir.name()), "channel": chan, "claimed": antenna, "at": at }));
            }
            drop(rec);
            if applied.rate != claim {
                return Err(DeviceError::failed(format!("UR-12: the device applied {} S/s for the claimed {claim}", applied.rate)));
            }
        }
        exact_decimation(self.mcr, claim)
            .map(|n| n as i64)
            .ok_or_else(|| DeviceError::failed(format!("UR-12: the device applied {claim} S/s for the claimed {claim}, which is no exact division of {} Hz", self.mcr)))
    }

    /// Declares and registers a stream clock with origin `origin` (UR-12, RM-25).
    pub fn register(&self, dir: Dir, n: i64, origin: i64) -> Result<Clock, String> {
        let stream = self.id.child(dir.name()).map_err(|e| e.to_string())?;
        let ratio = Rational::new(n as u64, 1).map_err(|e| e.to_string())?;
        let handle = self.clocks.declare_sample_clock(stream, self.root, ratio).map_err(|e| e.to_string())?;
        let domain = self.clocks.register_sample_clock(&handle, origin).map_err(|e| e.to_string())?;
        Ok(Clock { domain, origin, n })
    }
}
