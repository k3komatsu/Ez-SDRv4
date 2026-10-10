//! What the Provider's three threads share (UR-14…UR-30).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use ezsdr_kernel::event::{Event, EventHandle, EventKind, EventSink, EventSource, Severity};
use ezsdr_kernel::id::{ClockDomainId, ResourceId};
use ezsdr_kernel::module_api::{InputStore, StopMode};
use ezsdr_kernel::spec::{Key, Scalar, Value};
use ezsdr_kernel::stream::DataLink;
use ezsdr_kernel::time::{ClockRegistry, Rational, TimeAuthority, TimePoint};
use ezsdr_radio::device::DeviceDescription;
use ezsdr_radio::payloads::CommandRejectedPayload;
use ezsdr_radio::timeline::{self, Config, Item, Kind, Line};
use ezsdr_radio::{keys, kinds};
use serde_json::{Value as Json, json};

use super::control::{ColdConfig, Planned};
use crate::device::{Device, DeviceError, Dir, Settings, decimation};

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
        i64::try_from(i128::from(self.origin) + i128::from(k) * i128::from(self.n))
            .expect("a representable stream instant")
    }

    /// The first sample at or after root tick `t`.
    pub fn at_or_after(&self, t: i64) -> i64 {
        i64::try_from((i128::from(t) - i128::from(self.origin) + i128::from(self.n) - 1)
            .div_euclid(i128::from(self.n))).expect("a representable sample index")
    }
}

/// Each stream's planned state, once, shared by uhd-control, uhd-rx and uhd-tx and booked by
/// whichever learns an event (maintenance note 23): uhd-control the commands, uhd-rx and
/// uhd-tx a configuration their device refused, the thread that finds the device lost the
/// loss, and `Provider::stop` the streams' end. Lock order: a segment configuration's
/// `updates`, then this, then `rec`; never held across a device call.
#[derive(Default)]
pub(crate) struct Streams {
    /// Per direction, the stream's timeline (RM-26); none for a receive side with no link.
    pub lines: [Option<Line>; 2],
    /// Per direction, the configuration each segment begins with, by the command that began
    /// it, and whether uhd-control configured it itself, for an enable from no stream (UR-25).
    pub configs: [BTreeMap<u64, (Arc<ColdConfig>, bool)>; 2],
    /// The transmit clocks registered, in the order of the transmit plan's segments, and
    /// whether each has ended (RM-25).
    pub tx_clocks: Vec<(Clock, bool)>,
    /// The arrival order of the last command booked (RM-25).
    pub seq: u64,
    /// The streams' end — a loss or `Provider::stop` — has been booked, once; a command booked
    /// after it takes no effect, by the timeline's `End` rule.
    pub ended: bool,
    /// `Provider::stop`'s instant and mode, once it has begun (UR-26).
    pub stop: Option<(i64, StopMode)>,
    /// uhd-rx's segment, as its origin and configuration, and the first sample of it not yet
    /// delivered: RM-16's floor, which reaches the plan as `Item.delivered`.
    pub delivered: Option<(i64, Config, i64)>,
    /// Per direction, the owner has no stream and nothing planned to begin (UR-25).
    pub idle: [bool; 2],
    /// Counts the changes of the plans, for uhd-tx to read its own again.
    pub version: u64,
}

impl Streams {
    pub fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    /// Books a command or fault on a stream's timeline; the end of the receive stream with the
    /// first sample uhd-rx has not delivered of the segment it cuts (RM-16). False when there is
    /// no stream, or when its plan cannot be represented, which refuses it.
    pub fn book(&mut self, core: &Core, dir: Dir, mut item: Item) -> bool {
        let delivered = self.delivered;
        let Some(line) = self.lines[dir as usize].as_mut() else { return false };
        if dir == Dir::Rx && matches!(item.kind, Kind::End { .. }) {
            let cuts = |origin, config| line.made(origin, config, item.e, item.seq).ok().flatten().is_some_and(|s| s.cut.is_none());
            item.delivered = item.delivered.or(delivered.filter(|(origin, config, _)| cuts(*origin, *config)).map(|(_, _, k)| k));
        }
        if let Err(error) = line.book(item) {
            core.command_rejected("update_parameter", &format!("RM-26: {error}"));
            return false;
        }
        self.changed(core, dir);
        true
    }

    /// RM-25, VH-2: the device refused the configuration of the segment command `seq` began;
    /// the command is booked as refused, which halts the stream. False when it is no longer
    /// booked.
    pub fn refuse(&mut self, core: &Core, dir: Dir, seq: u64) -> bool {
        let Some(line) = self.lines[dir as usize].as_mut() else { return false };
        match line.refuse(seq) {
            Ok(refused) => {
                self.changed(core, dir);
                refused
            }
            Err(error) => {
                core.command_rejected("update_parameter", &format!("RM-26: {error}"));
                false
            }
        }
    }

    /// RM-16, VH-2: a loss or `Provider::stop` at `at` ends both streams, once; a loss counts
    /// as received with the faults, at the Run's start (RM-25).
    pub fn end(&mut self, core: &Core, at: i64, abort: bool, lost: bool) {
        if std::mem::replace(&mut self.ended, true) {
            return;
        }
        let seq = if lost { 0 } else { self.next_seq() };
        for dir in [Dir::Rx, Dir::Tx] {
            self.book(core, dir, Item { e: at, seq, ready: at, delivered: None, refused: false, kind: Kind::End { abort } });
        }
    }

    /// #63: once its owner has ended the segment command `by` began, the items before that
    /// command are pruned: nothing booked later takes effect before it, and no command before
    /// it can be refused any more.
    pub fn prune(&mut self, dir: Dir, by: Option<u64>) {
        let Some(line) = self.lines[dir as usize].as_mut() else { return };
        let Some(e) = by.and_then(|by| line.items.iter().find(|item| item.seq == by)).map(|item| timeline::effective(&line.items, item)) else { return };
        line.prune(e);
        let kept: Vec<u64> = line.items.iter().map(|item| item.seq).collect();
        self.configs[dir as usize].retain(|seq, _| kept.contains(seq));
    }

    /// The plan of `dir`, each segment with its configuration and its transmit clock.
    pub fn planned(&self, dir: Dir) -> Vec<Planned> {
        let Some(line) = self.lines[dir as usize].as_ref() else { return Vec::new() };
        line.plan.iter().enumerate().map(|(index, segment)| {
            let config = segment.by.and_then(|by| self.configs[dir as usize].get(&by));
            Planned {
                segment: *segment,
                settings: config.map(|(settings, _)| settings.clone()),
                configured: config.is_some_and(|(_, configured)| *configured),
                clock: (dir == Dir::Tx).then(|| self.tx_clocks.get(index).map(|(clock, _)| *clock)).flatten(),
            }
        }).collect()
    }

    /// A plan changed: the transmit clocks follow it (RM-25, VH-2).
    fn changed(&mut self, core: &Core, dir: Dir) {
        self.version += 1;
        if dir == Dir::Tx {
            if let Err(error) = self.reconcile(core) {
                core.command_rejected("update_parameter", &format!("UR-25: {error}"));
            }
        }
    }

    /// Each transmit segment's clock, registered when its change is booked and ended at its
    /// cut once the plan has one; a clock registered for a change the plan no longer has — one
    /// booked after `Provider::stop`'s instant — ends at its origin (KC-21a, VH-2).
    fn reconcile(&mut self, core: &Core) -> Result<(), String> {
        let plan = self.lines[Dir::Tx as usize].as_ref().map_or_else(Vec::new, |line| line.plan.clone());
        for (index, segment) in plan.iter().enumerate() {
            if index == self.tx_clocks.len() {
                let clock = core.register(Dir::Tx, segment.config.ratio.num() as i64, segment.origin)?;
                self.tx_clocks.push((clock, false));
            }
            let (clock, ended) = &mut self.tx_clocks[index];
            if let Some(cut) = segment.cut.filter(|_| !*ended) {
                core.clocks.end(clock.domain, core.at(clock.instant(cut))).map_err(|e| e.to_string())?;
                *ended = true;
            }
        }
        for (clock, ended) in self.tx_clocks.iter_mut().skip(plan.len()).filter(|(_, ended)| !*ended) {
            if let Err(error) = core.clocks.end(clock.domain, core.at(clock.origin)) {
                core.reject_note(json!({ "clock_not_ended": error.to_string() }));
            }
            *ended = true;
        }
        Ok(())
    }

    /// UR-30: the plans the streams were carried out by, each segment's origin and end.
    pub fn record(&self) -> Json {
        let plan = |dir: Dir| -> Vec<Json> {
            self.lines[dir as usize].iter().flat_map(|line| &line.plan).map(|segment| json!({
                "origin": segment.origin,
                "end": segment.cut.and_then(|cut| segment.instant(cut).ok()),
                "channels": segment.config.channels,
                "ticks_per_sample": [segment.config.ratio.num(), segment.config.ratio.den()],
            })).collect()
        };
        json!({ "what": "plan", "rx": plan(Dir::Rx), "tx": plan(Dir::Tx) })
    }
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
            "rx_before_origin",
            "link_drops_seen",
            "tx_bursts",
            "tx_samples",
            "tx_errors",
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
        let overflow = events.resolve(&EventSource::Node { node: rx_id.clone() }, &EventKind::parse(kinds::RX_OVERFLOW).expect("a radio kind"));
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
        self.time.now(self.root).and_then(|t| t.ticks_in(self.root)).unwrap_or(i64::MIN)
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
            source: EventSource::Node { node: source.clone() },
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

    /// MA-9a: `DEVICE_LOST`, once, from this Provider; then no thread uses the device. The
    /// thread that finds the loss books it on both streams, before any thread can see the
    /// device lost (RM-16).
    pub fn device_lost(&self, message: &str) {
        let mut streams = lock(&self.streams);
        if self.lost.load(Ordering::Acquire) {
            return;
        }
        streams.end(self, self.now(), false, true);
        self.lost.store(true, Ordering::Release);
        drop(streams);
        // Before anything can free it: a lost device is never freed (F4).
        self.device.mark_lost();
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
            Some(Value::Scalar(Scalar::Num(v))) => Some(v.get()),
            Some(Value::Scalar(Scalar::Int(v))) => Some(*v as f64),
            _ => None,
        };
        Settings {
            rate: number("sample_rate_hz"),
            freq: number("frequency_hz"),
            gain: number("gain_db"),
            antenna: match config.get(&key(&format!("radio.{}.antenna", dir.name()))) {
                Some(Value::Scalar(Scalar::Str(s))) => Some(s.clone()),
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
            Some(Value::Scalar(Scalar::Int(n))) if *n > 0 => *n as usize,
            _ => 0,
        }
    }

    /// **Configuring a direction** (UR-12): the whole configuration on every channel,
    /// untimed, read back; the rate must be exactly the claim, a decimation of the
    /// master clock ([`crate::device::decimation`]). Returns that decimation `N`.
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
        decimation(self.mcr, claim)
            .map(|n| n as i64)
            .ok_or_else(|| DeviceError::failed(format!("UR-12: {claim} S/s is no decimation 1…512 of {} Hz", self.mcr)))
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
