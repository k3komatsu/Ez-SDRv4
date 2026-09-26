//! `ezsdr.radio.uhd` as a Provider of the `radio` Vocabulary: a hardware Provider,
//! so never stepped (MA-15). Its own threads do the work the coordinator's step loop
//! does for MockRadio:
//!
//! - **rx**: `recv` → Stream Contract block → publish on the Links → schedule a wakeup
//!   so the coordinator steps the Sinks;
//! - **control**: polls the `ActionReceiver` (pull-only, MA-14) for admitted Actions;
//! - **tx**: sends bursts, feeding the Kernel's `BurstTracker` so the Manifest gets
//!   the same SC-28 burst records a Mock writes;
//! - **async**: turns UHD's asynchronous TX reports into `radio.*` events.
//!
//! `instance()` and `coerce()` come from a MockRadio built with the chosen grid
//! (`x310-like` or `ideal`): MA-11 says coerce is pure and needs no hardware, so the
//! only static device description this repository has is the Mock profile. What the
//! device then actually applies is recorded next to what the grid claimed.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration as StdDuration, Instant};

use ezsdr_hostmem::{HOST_MEMORY, HostPool, write_cf32};
use ezsdr_kernel::binding::Binding;
use ezsdr_kernel::contract::DataContractId;
use ezsdr_kernel::event::{Action, Event, EventKind, EventSink, Severity};
use ezsdr_kernel::id::{ClockDomainId, ResourceId};
use ezsdr_kernel::module_api::{
    ActionReceiver, CoerceReport, CoercionFidelity, Driving, Endpoint, EnvelopeFidelity, ExecutionClass,
    Fidelity, InputStore, ModuleError, ModuleErrorKind, PrepareContext, Provider, ProviderInstance,
    Requested, RfFidelity, Role, StopMode, TransportFidelity, Version,
};
use ezsdr_kernel::plan::{Fragment, PrepareReport};
use ezsdr_kernel::spec::{Ident, Key, Namespace, Value};
use ezsdr_kernel::stream::{
    BackPressure, BlockFlags, BlockHeader, BurstOpen, BurstStep, BurstTracker, ChannelMask, DataLink, Direction,
    LateOutcome, PublishOutcome, SampleBlock,
};
use ezsdr_kernel::time::{ClockRegistry, Converted, Duration, Rational, SampleClockHandle, TimeAuthority, TimeError, TimePoint};
use ezsdr_mock_radio::MockRadio;
use ezsdr_radio::keys;
use ezsdr_radio::payloads::{
    CommandRejectedPayload, RxOverflowCause, RxOverflowPayload, TimeErrorCause, TimeErrorOutcome, TimeErrorPayload,
};
use serde_json::{Value as Json, json};

use crate::device::{Device, Dir, Iq, RxRecv, Settings};

fn reject(msg: impl Into<String>) -> ModuleError {
    ModuleError::rejected(msg)
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// MockRadio's defaults (`ezsdr-mock-radio/src/coerce.rs`, private there): what a
/// Provider applies for a key the Spec does not constrain.
fn defaults() -> BTreeMap<Key, Value> {
    let k = |s: &str| Key::parse(s).expect("radio key");
    BTreeMap::from([
        (k(keys::RX_CHANNELS), Value::Int(1)),
        (k(keys::TX_CHANNELS), Value::Int(0)),
        (k(keys::RX_SAMPLE_RATE_HZ), Value::Num(1e6)),
        (k(keys::TX_SAMPLE_RATE_HZ), Value::Num(1e6)),
        (k(keys::RX_FREQUENCY_HZ), Value::Num(1e9)),
        (k(keys::TX_FREQUENCY_HZ), Value::Num(1e9)),
        (k(keys::RX_GAIN_DB), Value::Num(0.0)),
        (k(keys::TX_GAIN_DB), Value::Num(0.0)),
        (k(keys::RX_ANTENNA), Value::Str("RX2".into())),
        (k(keys::TX_ANTENNA), Value::Str("TX/RX".into())),
    ])
}

fn num(c: &BTreeMap<Key, Value>, k: &str) -> f64 {
    match c.get(&Key::parse(k).expect("radio key")) {
        Some(Value::Num(v)) => *v,
        Some(Value::Int(v)) => *v as f64,
        _ => 0.0,
    }
}

fn int(c: &BTreeMap<Key, Value>, k: &str) -> i64 {
    match c.get(&Key::parse(k).expect("radio key")) {
        Some(Value::Int(v)) => *v,
        _ => 0,
    }
}

fn string(c: &BTreeMap<Key, Value>, k: &str) -> Option<String> {
    match c.get(&Key::parse(k).expect("radio key")) {
        Some(Value::Str(v)) => Some(v.clone()),
        _ => None,
    }
}

/// Selector options, all optional.
#[derive(Clone, Debug)]
struct Opts {
    /// Samples per receive block.
    block_len: usize,
    /// The least lead the Provider claims for a timed command (`min_command_lead`).
    min_lead_ns: i64,
    /// The least lead `start` accepts between now and T0.
    start_margin_ns: i64,
    /// Stall the receive loop once, this long after start, to provoke an overflow.
    stall_after_ms: Option<u64>,
    /// For how long.
    stall_ms: u64,
}

/// Everything the threads write, folded into Manifest sections at `cleanup`.
#[derive(Default)]
struct Records {
    stats: BTreeMap<&'static str, i64>,
    coercion: Vec<Json>,
    bursts: Vec<Json>,
    device_async: Vec<Json>,
    rejected: Vec<Json>,
    timing: Vec<Json>,
    rx_errors: Vec<Json>,
}

impl Records {
    fn add(&mut self, k: &'static str, n: i64) {
        *self.stats.entry(k).or_default() += n;
    }
}

/// Emits `radio.*` events from any thread. `emit_control` is documented as the control
/// path; a hardware Provider's events are born on its own threads (spike finding).
#[derive(Clone)]
struct Emitter {
    events: Arc<dyn EventSink>,
    id: ResourceId,
    rec: Arc<Mutex<Records>>,
}

impl Emitter {
    fn emit(&self, suffix: &str, kind: &str, severity: Severity, payload: Json, at: TimePoint) {
        let source = if suffix.is_empty() { self.id.clone() } else { self.id.child(suffix).unwrap_or_else(|_| self.id.clone()) };
        let event = Event { source, time: at, severity, kind: EventKind::parse(kind).expect("radio kind"), payload };
        if let Err(e) = self.events.emit_control(event) {
            lock(&self.rec).rejected.push(json!({ "event_not_emitted": kind, "error": format!("{e:?}") }));
        }
    }

    fn command_rejected(&self, action: &str, reason: &str, at: TimePoint) {
        lock(&self.rec).rejected.push(json!({ "action": action, "reason": reason, "at": at }));
        let p = serde_json::to_value(CommandRejectedPayload { action: action.into(), reason: reason.into() }).unwrap();
        self.emit("", ezsdr_radio::kinds::COMMAND_REJECTED, Severity::Error, p, at);
    }
}

/// What `prepare` learned and handed over.
struct Prepared {
    root: ClockDomainId,
    clocks: Arc<ClockRegistry>,
    time: Arc<dyn TimeAuthority>,
    actions: Arc<dyn ActionReceiver>,
    inputs: Arc<dyn InputStore>,
    links: Vec<Arc<dyn DataLink>>,
    emitter: Emitter,
    rx_chans: usize,
    tx_chans: usize,
    rx_handle: Option<SampleClockHandle>,
    tx_handle: Option<SampleClockHandle>,
    tx: Option<(ClockDomainId, i64)>,
    config: BTreeMap<Key, Value>,
}

enum TxCmd {
    Burst { k: i64, samples: Vec<Vec<Iq>>, repeat: bool, open: BurstOpen },
    /// Ends the running burst (a Stop Action on the stream).
    Stop,
    /// Ends the running burst and the thread (the Provider's `stop`).
    Shutdown,
}

type Handles = Arc<Mutex<Vec<(&'static str, JoinHandle<()>)>>>;

/// The running transmit side: its SampleClock and the tx thread's queue.
struct TxRun {
    domain: ClockDomainId,
    origin: i64,
    ratio: i64,
    chans: usize,
    cmd: mpsc::Sender<TxCmd>,
}

/// What starting the transmit side needs; shared by `start` and by the control
/// thread, which starts it on a cold `radio.tx.channels` change.
#[derive(Clone)]
struct TxSpawn {
    device: Arc<dyn Device>,
    rec: Arc<Mutex<Records>>,
    emitter: Emitter,
    stop: Arc<AtomicBool>,
    time: Arc<dyn TimeAuthority>,
    root: ClockDomainId,
    rate: i64,
    handles: Handles,
    state: Arc<Mutex<Option<TxRun>>>,
}

impl TxSpawn {
    fn spawn(&self, domain: ClockDomainId, origin: i64, ratio: i64, chans: usize) {
        let (send, recv) = mpsc::channel();
        let job = TxJob {
            device: self.device.clone(),
            rec: self.rec.clone(),
            stop: self.stop.clone(),
            cmds: recv,
            domain,
            origin,
            ratio,
            rate: self.rate,
            time: self.time.clone(),
            root: self.root,
        };
        let mut h = lock(&self.handles);
        h.push(("tx", std::thread::Builder::new().name("uhd-tx".into()).spawn(move || job.run()).unwrap()));
        let (dev, em, rec, st, root) = (self.device.clone(), self.emitter.clone(), self.rec.clone(), self.stop.clone(), self.root);
        h.push(("async", std::thread::Builder::new().name("uhd-async".into()).spawn(move || async_loop(dev, em, rec, st, root)).unwrap()));
        *lock(&self.state) = Some(TxRun { domain, origin, ratio, chans, cmd: send });
    }
}

struct Threads {
    control_stop: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    rx_stop: Arc<AtomicBool>,
    tx: Arc<Mutex<Option<TxRun>>>,
    handles: Handles,
}

/// The UHD Provider.
pub struct UhdRadio {
    instance: ProviderInstance,
    section_id: Ident,
    device: Arc<dyn Device>,
    grid: MockRadio,
    opts: Opts,
    rec: Arc<Mutex<Records>>,
    prep: Option<Prepared>,
    threads: Option<Threads>,
}

fn section(id: &Ident, suffix: &str) -> Namespace {
    Namespace::parse(&format!("ezsdr.radio.uhd.{id}.{suffix}")).expect("section name")
}

impl UhdRadio {
    /// Builds the Provider from its binding and the device its `args` opened.
    pub fn from_binding(binding: &Binding, device: Arc<dyn Device>) -> Result<UhdRadio, ModuleError> {
        if binding.module != crate::module_ref() {
            return Err(reject("uhd: binding names another Module"));
        }
        let allowed = ["args", "clock_source", "time_source", "id", "grid", "block_len", "min_lead_ns", "start_margin_ns", "rx_stall_after_ms", "rx_stall_ms"];
        if let Some(k) = binding.selector.keys().find(|k| !allowed.contains(&k.as_str())) {
            return Err(reject(format!("uhd: unsupported selector key `{k}`")));
        }
        let get = |k: &str| binding.selector.get(&Ident::parse(k).expect("selector key"));
        let int_opt = |k: &str| match get(k) {
            None => Ok(None),
            Some(Value::Int(v)) if *v >= 0 => Ok(Some(*v)),
            _ => Err(reject(format!("uhd: selector `{k}` must be a non-negative Int"))),
        };
        let id = match get("id") {
            None => "usrp".to_owned(),
            Some(Value::Str(s)) => s.clone(),
            _ => return Err(reject("uhd: selector `id` must be a string")),
        };
        let grid_name = match get("grid") {
            None => "x310-like".to_owned(),
            Some(Value::Str(s)) if s == "x310-like" || s == "ideal" => s.clone(),
            _ => return Err(reject("uhd: selector `grid` must be x310-like or ideal")),
        };
        let opts = Opts {
            block_len: int_opt("block_len")?.unwrap_or(2000) as usize,
            min_lead_ns: int_opt("min_lead_ns")?.unwrap_or(2_000_000),
            start_margin_ns: int_opt("start_margin_ns")?.unwrap_or(100_000_000),
            stall_after_ms: int_opt("rx_stall_after_ms")?.map(|v| v as u64),
            stall_ms: int_opt("rx_stall_ms")?.unwrap_or(500) as u64,
        };
        // The grid: a MockRadio with the same resource id, used only for its tree and
        // its pure `coerce` (MA-10, MA-11).
        let mock_binding = Binding {
            module: ezsdr_kernel::module_api::ModuleRef {
                id: ezsdr_kernel::id::ModuleId::parse("ezsdr.radio.mock").unwrap(),
                version: Version::new(1, 1, 0),
            },
            selector: BTreeMap::from([(Ident::parse("id").unwrap(), Value::Str(id.clone()))]),
            profile: Some(ezsdr_kernel::module_api::ProfileRef { name: grid_name.clone(), version: Version::new(1, 1, 0) }),
            feed: None,
        };
        let grid = MockRadio::from_binding(&mock_binding)?;
        let section_id = Ident::parse(&id).map_err(|_| reject("uhd: selector `id` must match ^[a-z][a-z0-9_]*$"))?;
        let mut sections = BTreeMap::new();
        sections.insert(section(&section_id, "device"), json!({ "grid": grid_name, "device": device.describe() }));
        let real = Fidelity {
            timing: EnvelopeFidelity::Real,
            continuity: EnvelopeFidelity::Real,
            coercion: CoercionFidelity::Real,
            rf: RfFidelity::Real,
            transport: TransportFidelity::Real,
        };
        let instance = ProviderInstance {
            id: grid.instance().id.clone(),
            module: crate::module_ref(),
            profile: None,
            tree: grid.instance().tree.clone(),
            fidelity: real,
            driving: Driving { stepped: false },
            arm_after: Vec::new(),
            min_command_lead: (opts.min_lead_ns > 0)
                .then(|| Duration::new(ClockDomainId::HOST_MONOTONIC, opts.min_lead_ns)),
            sections,
        };
        Ok(UhdRadio { instance, section_id, device, grid, opts, rec: Arc::default(), prep: None, threads: None })
    }

    fn prep(&self) -> Result<&Prepared, ModuleError> {
        self.prep.as_ref().ok_or_else(|| reject("uhd: not prepared"))
    }

    fn now_root(&self) -> Result<i64, ModuleError> {
        let p = self.prep()?;
        Ok(p.time.now(p.root).map_err(|e| reject(format!("uhd: now: {e}")))?.ticks)
    }

    /// Joins every thread, then schedules one last wakeup so the coordinator's drain
    /// steps the Sinks over the final blocks.
    fn stop_threads(&mut self) {
        let Some(t) = self.threads.take() else { return };
        t.rx_stop.store(true, Ordering::Release);
        if let Some(tx) = lock(&t.tx).as_ref() {
            let _ = tx.cmd.send(TxCmd::Shutdown);
        }
        // Control first, so it cannot start a transmit side while we join; then tx and
        // rx (they flush their streams); a moment for the last BURST_ACK; then async.
        let control: Vec<_> = {
            let mut h = lock(&t.handles);
            let i = h.iter().position(|(n, _)| *n == "control");
            i.map(|i| h.remove(i)).into_iter().collect()
        };
        t.control_stop.store(true, Ordering::Release);
        let mut join = |(name, h): (&'static str, JoinHandle<()>)| {
            if h.join().is_err() {
                lock(&self.rec).rejected.push(json!({ "thread_panicked": name }));
            }
        };
        control.into_iter().for_each(&mut join);
        if let Some(tx) = lock(&t.tx).as_ref() {
            let _ = tx.cmd.send(TxCmd::Shutdown);
        }
        let all: Vec<_> = lock(&t.handles).drain(..).collect();
        let (flushing, rest): (Vec<_>, Vec<_>) = all.into_iter().partition(|(n, _)| *n == "tx" || *n == "rx");
        let mut join = |(name, h): (&'static str, JoinHandle<()>)| {
            if h.join().is_err() {
                lock(&self.rec).rejected.push(json!({ "thread_panicked": name }));
            }
        };
        flushing.into_iter().for_each(&mut join);
        std::thread::sleep(StdDuration::from_millis(200));
        t.stop.store(true, Ordering::Release);
        rest.into_iter().for_each(&mut join);
        if let Some(p) = &self.prep {
            if let Ok(now) = p.time.now(p.root) {
                let _ = p.time.schedule(now, Box::new(|_| {}));
            }
        }
    }

    fn fold_sections(&mut self) {
        let r = lock(&self.rec);
        let put = |sections: &mut BTreeMap<Namespace, Json>, suffix: &str, v: Json| {
            sections.insert(section(&self.section_id, suffix), v);
        };
        let s = &mut self.instance.sections;
        put(s, "stats", json!(r.stats));
        put(s, "coercion", Json::Array(r.coercion.clone()));
        put(s, "bursts", Json::Array(r.bursts.clone()));
        put(s, "async", Json::Array(r.device_async.clone()));
        put(s, "rejected", Json::Array(r.rejected.clone()));
        put(s, "timing", Json::Array(r.timing.clone()));
        put(s, "rx_errors", Json::Array(r.rx_errors.clone()));
    }
}

impl Provider for UhdRadio {
    fn instance(&self) -> &ProviderInstance {
        &self.instance
    }

    fn coerce(&self, request: &Requested) -> Result<CoerceReport, ModuleError> {
        self.grid.coerce(request)
    }

    fn prepare(&mut self, f: &Fragment, ctx: PrepareContext) -> Result<PrepareReport, ModuleError> {
        if self.prep.is_some() {
            return Err(reject("uhd: prepare was already called"));
        }
        if !matches!(ctx.class, ExecutionClass::HardwareInLoop | ExecutionClass::Hardware) {
            return Err(ModuleError {
                kind: ModuleErrorKind::Unsupported,
                message: format!("uhd: a hardware Provider cannot run in class {:?}", ctx.class),
                detail: Json::Null,
            });
        }
        if f.instance != crate::module_ref() || f.role != Role::Provider {
            return Err(reject("uhd: fragment identity or role does not match"));
        }
        let requested: Requested = serde_json::from_value(
            f.content.get("requested").cloned().ok_or_else(|| reject("uhd: fragment has no requested"))?,
        )
        .map_err(|e| reject(format!("uhd: invalid Requested: {e}")))?;
        let report = self.coerce(&requested)?;
        if !report.rejected.is_empty() {
            return Err(reject(format!("uhd: rejected requests: {:?}", report.rejected)));
        }
        let mut config = defaults();
        for (k, v) in report.applied {
            if keys::CONFIGURATION.contains(&k.as_str()) {
                config.insert(k, v);
            }
        }
        let root = ctx.time.primary_root();
        let root_rate = ctx.clocks.nominal_rate(root).map_err(|e| reject(format!("uhd: {e}")))?;
        let tick_rate = self.device.tick_rate();
        if root_rate != Rational::new(tick_rate, 1).unwrap() {
            return Err(reject(format!(
                "uhd: the primary root runs at {root_rate:?}, not this device's {tick_rate} Hz master clock; bind the Authority to the same device"
            )));
        }
        let rx_chans = int(&config, keys::RX_CHANNELS).max(0) as usize;
        let tx_chans = int(&config, keys::TX_CHANNELS).max(0) as usize;
        for (dir, n) in [(Dir::Rx, rx_chans), (Dir::Tx, tx_chans)] {
            if n > self.device.channels(dir) {
                return Err(reject(format!("uhd: {n} {dir:?} channels asked, the device has {}", self.device.channels(dir))));
            }
        }
        // Apply to the device, and record what it did next to what the grid claimed.
        let mut handles = [None, None];
        for (slot, (dir, name, n)) in [(Dir::Rx, "rx", rx_chans), (Dir::Tx, "tx", tx_chans)].into_iter().enumerate() {
            if n == 0 {
                continue;
            }
            let claimed = Settings {
                rate: Some(num(&config, &format!("radio.{name}.sample_rate_hz"))),
                freq: Some(num(&config, &format!("radio.{name}.frequency_hz"))),
                gain: Some(num(&config, &format!("radio.{name}.gain_db"))),
                antenna: string(&config, &format!("radio.{name}.antenna")),
            };
            let mut rate = 0.0;
            for chan in 0..n {
                let applied = self.device.apply(dir, chan, &claimed, None).map_err(|e| reject(format!("uhd: {e}")))?;
                rate = applied.rate;
                lock(&self.rec).coercion.push(json!({
                    "dir": name, "chan": chan,
                    "rate": { "claimed": claimed.rate, "device": applied.rate, "equal": claimed.rate == Some(applied.rate) },
                    "freq": { "claimed": claimed.freq, "device": applied.freq, "diff_hz": applied.freq - claimed.freq.unwrap_or(0.0) },
                    "gain": { "claimed": claimed.gain, "device": applied.gain, "equal": claimed.gain == Some(applied.gain) },
                }));
            }
            // A SampleClock is an exact division of the root (TM-13a): the device's
            // actual rate must be master clock / integer.
            let n_div = (tick_rate as f64 / rate).round();
            if n_div < 1.0 || (tick_rate as f64 / n_div - rate).abs() > 1e-6 {
                return Err(reject(format!("uhd: device rate {rate} is not {tick_rate}/N; no exact SampleClock")));
            }
            let stream = self.instance.id.child(name).map_err(|e| reject(format!("uhd: {e}")))?;
            handles[slot] = Some(
                ctx.clocks
                    .declare_sample_clock(stream, root, Rational::new(n_div as u64, 1).unwrap())
                    .map_err(|e| reject(format!("uhd: {e}")))?,
            );
        }
        let mut links = Vec::new();
        for attached in &ctx.links {
            if attached.component != f.id || attached.port.as_str() != "rx" {
                return Err(reject("uhd: only this fragment's rx port may be attached"));
            }
            match &attached.endpoint {
                Endpoint::StreamOut(link) => links.push(link.clone()),
                _ => return Err(reject("uhd: rx must attach to a StreamOut link")),
            }
        }
        if links.iter().any(|l| l.policy() == BackPressure::Block) {
            return Err(reject("uhd: a device cannot wait for a Block link; its ring overflows instead"));
        }
        let [rx_handle, tx_handle] = handles;
        let emitter = Emitter { events: ctx.events.clone(), id: self.instance.id.clone(), rec: self.rec.clone() };
        self.prep = Some(Prepared {
            root,
            clocks: ctx.clocks,
            time: ctx.time,
            actions: ctx.actions,
            inputs: ctx.inputs,
            links,
            emitter,
            rx_chans,
            tx_chans,
            rx_handle,
            tx_handle,
            tx: None,
            config: config.clone(),
        });
        Ok(PrepareReport { fragment: f.id.clone(), effective: config, coercions: report.coercions, warnings: Vec::new() })
    }

    fn arm(&mut self) -> Result<(), ModuleError> {
        let now = self.now_root()?;
        let p = self.prep.as_mut().ok_or_else(|| reject("uhd: prepare must precede arm"))?;
        if p.rx_chans > 0 && !p.links.is_empty() {
            self.device.rx_open(p.rx_chans).map_err(|e| reject(format!("uhd: {e}")))?;
        }
        if let Some(h) = &p.tx_handle {
            self.device.tx_open(p.tx_chans).map_err(|e| reject(format!("uhd: {e}")))?;
            // TX's SampleClock starts now, as MockRadio's does (MR-9): admission needs a
            // running transmit clock before the first TxBurst is dispatched (SC-23).
            let domain = p.clocks.register_sample_clock(h, now).map_err(|e| reject(format!("uhd: {e}")))?;
            p.tx = Some((domain, now));
        }
        Ok(())
    }

    fn start(&mut self, at: Option<TimePoint>) -> Result<(), ModuleError> {
        let now = self.now_root()?;
        let p = self.prep()?;
        let target = match at {
            None => now,
            Some(at) if at.domain == p.root => at.ticks,
            Some(at) => match p.clocks.convert(at, p.root).map_err(|e| reject(format!("uhd: {e}")))? {
                Converted::Exact { point } => point.ticks,
                Converted::Inexact { floor, .. } => floor.ticks + 1,
            },
        };
        let rate = self.device.tick_rate() as i64;
        let margin = (self.opts.start_margin_ns as i128 * rate as i128 / 1_000_000_000) as i64;
        lock(&self.rec).timing.push(json!({
            "what": "start", "t0": target, "now": now,
            "lead_ns": (target - now) as i128 * 1_000_000_000 / rate as i128,
        }));
        if target - now < margin {
            return Err(reject(format!(
                "uhd: T0 is {} ns ahead, less than start_margin_ns {}; raise ezsdr.time.start_lead_ns",
                (target - now) as i128 * 1_000_000_000 / rate as i128,
                self.opts.start_margin_ns
            )));
        }
        let stop = Arc::new(AtomicBool::new(false));
        let rx_stop = Arc::new(AtomicBool::new(false));
        let mut handles = Vec::new();
        let mut rx_grid = None;

        if let (Some(h), false) = (&p.rx_handle, p.links.is_empty()) {
            let ratio = h.root_ticks_per_tick.num() as i64;
            // The TX SampleClock started at arm (MR-9's rule), at an arbitrary device
            // tick; T0 is another. A burst at "T0 + n samples" is then rounded up to the
            // TX grid (SC-23b) and leaves (T0 − arm) mod ratio ticks after RX sample n.
            // On the Mock the two grids coincide because virtual T0 − arm is a round
            // lead; on a device they do not (spike finding K3). Workaround: start RX on
            // the TX grid, so sample n is the same instant on both.
            let mut target = target;
            if let (Some((_, tx_origin)), Some(txh)) = (p.tx, &p.tx_handle) {
                let tx_ratio = txh.root_ticks_per_tick.num() as i64;
                let off = (target - tx_origin).rem_euclid(tx_ratio);
                if off != 0 && tx_ratio == ratio {
                    target += tx_ratio - off;
                }
                lock(&self.rec).timing.push(json!({ "what": "rx_origin_shift_to_tx_grid", "ticks": if off == 0 { 0 } else { tx_ratio - off }, "tx_ratio": tx_ratio, "rx_ratio": ratio }));
            }
            let domain = p.clocks.register_sample_clock(h, target).map_err(|e| reject(format!("uhd: {e}")))?;
            rx_grid = Some((target, ratio));
            self.device.rx_start(target).map_err(|e| reject(format!("uhd: {e}")))?;
            // A wakeup must always be pending while the device streams: with an empty
            // agenda `next_wakeup` returns None and the coordinator completes a Spec
            // Run, although samples are still arriving (spike finding K2). Per-block
            // wakeups from the rx thread race the Authority; a self-renewing heartbeat
            // does not, because its callback re-arms inside `next_wakeup`.
            let period = (self.opts.block_len as i64 * ratio).max(rate / 200);
            heartbeat(p.time.clone(), TimePoint::new(p.root, target + period), period, stop.clone())
                .map_err(|e| reject(format!("uhd: {e}")))?;
            let job = RxJob {
                device: self.device.clone(),
                links: p.links.clone(),
                emitter: p.emitter.clone(),
                rec: self.rec.clone(),
                rx_stop: rx_stop.clone(),
                domain,
                origin: target,
                ratio,
                channels: p.rx_chans,
                opts: self.opts.clone(),
                rate,
            };
            handles.push(("rx", std::thread::Builder::new().name("uhd-rx".into()).spawn(move || job.run()).unwrap()));
        }

        let handles: Handles = Arc::new(Mutex::new(handles));
        let spawn = TxSpawn {
            device: self.device.clone(),
            rec: self.rec.clone(),
            emitter: p.emitter.clone(),
            stop: stop.clone(),
            time: p.time.clone(),
            root: p.root,
            rate,
            handles: handles.clone(),
            state: Arc::new(Mutex::new(None)),
        };
        if let (Some(h), Some((domain, origin))) = (&p.tx_handle, p.tx) {
            spawn.spawn(domain, origin, h.root_ticks_per_tick.num() as i64, p.tx_chans);
        }
        let control_stop = Arc::new(AtomicBool::new(false));
        let control = Control {
            actions: p.actions.clone(),
            inputs: p.inputs.clone(),
            clocks: p.clocks.clone(),
            config: p.config.clone(),
            spawn: spawn.clone(),
            stop: control_stop.clone(),
            rx_stop: rx_stop.clone(),
            id: self.instance.id.clone(),
            rx_chans: p.rx_chans,
            rx_grid,
            min_lead_ns: self.opts.min_lead_ns,
        };
        lock(&handles).push(("control", std::thread::Builder::new().name("uhd-control".into()).spawn(move || control.run()).unwrap()));
        self.threads = Some(Threads { control_stop, stop, rx_stop, tx: spawn.state, handles });
        Ok(())
    }

    fn stop(&mut self, _mode: StopMode) -> Result<(), ModuleError> {
        // Orderly and abort are the same here: the device ring is drained in both,
        // because the samples already on the wire are real (spike simplification).
        self.stop_threads();
        Ok(())
    }

    fn cleanup(&mut self) {
        self.stop_threads();
        self.device.close_streams();
        if self.prep.take().is_some() {
            self.fold_sections();
        }
    }
}

// ---------------------------------------------------------------- rx

struct RxJob {
    device: Arc<dyn Device>,
    links: Vec<Arc<dyn DataLink>>,
    emitter: Emitter,
    rec: Arc<Mutex<Records>>,
    rx_stop: Arc<AtomicBool>,
    domain: ClockDomainId,
    origin: i64,
    ratio: i64,
    channels: usize,
    opts: Opts,
    rate: i64,
}

impl RxJob {
    fn run(self) {
        let started = Instant::now();
        let mut pool = HostPool::new(self.opts.block_len * self.channels * 8);
        let mut expected: i64 = 0;
        let mut flags = BlockFlags::NONE;
        let mut overflow: Option<bool> = None;
        let mut stalled = false;
        let mut first = true;
        let mut draining = false;
        loop {
            if !draining && self.rx_stop.load(Ordering::Acquire) {
                let _ = self.device.rx_stop();
                draining = true;
            }
            if let (Some(after), false) = (self.opts.stall_after_ms, stalled) {
                if started.elapsed() >= StdDuration::from_millis(after) {
                    std::thread::sleep(StdDuration::from_millis(self.opts.stall_ms));
                    stalled = true;
                    lock(&self.rec).timing.push(json!({ "what": "rx_stall", "after_ms": after, "ms": self.opts.stall_ms }));
                }
            }
            let got = self.device.rx_recv(self.opts.block_len, if draining { 0.05 } else { 0.1 });
            let (first_tick, samples) = match got {
                RxRecv::Timeout if draining => break,
                RxRecv::Timeout => continue,
                RxRecv::Overflow { out_of_sequence } => {
                    flags = flags | BlockFlags::GAP_BEFORE
                        | if out_of_sequence { BlockFlags::SEQ_DISCONTINUITY } else { BlockFlags::RESTARTED };
                    overflow = Some(out_of_sequence);
                    lock(&self.rec).add("rx_overflows", 1);
                    continue;
                }
                RxRecv::Error(e) => {
                    let mut r = lock(&self.rec);
                    r.add("rx_errors", 1);
                    if r.rx_errors.len() < 32 {
                        r.rx_errors.push(json!({ "error": e, "after_ms": started.elapsed().as_millis() as u64 }));
                    }
                    if draining {
                        break;
                    }
                    continue;
                }
                RxRecv::Samples { first_tick, samples } => (first_tick, samples),
            };
            let len = samples[0].len() as i64;
            let off = first_tick - self.origin;
            if off.rem_euclid(self.ratio) != 0 {
                lock(&self.rec).add("rx_off_grid_blocks", 1);
            }
            let k = off.div_euclid(self.ratio);
            if first {
                first = false;
                lock(&self.rec).timing.push(json!({
                    "what": "first_rx_block", "k": k, "first_tick": first_tick, "origin": self.origin,
                    "host_ms_after_start_call": started.elapsed().as_millis() as u64,
                }));
            }
            let lost = if k > expected {
                flags = flags | BlockFlags::GAP_BEFORE;
                if overflow.is_none() {
                    // A time jump UHD did not announce as an overflow.
                    flags = flags | BlockFlags::SEQ_DISCONTINUITY;
                }
                Some((k - expected) as u64)
            } else {
                None
            };
            if k < expected {
                lock(&self.rec).add("rx_overlapping_blocks", 1);
                continue;
            }
            if let Some(oos) = overflow.take() {
                let lost_n = lost.unwrap_or(0);
                let gap_ns = (lost_n as i128 * self.ratio as i128 * 1_000_000_000 / self.rate as i128) as i64;
                let p = serde_json::to_value(RxOverflowPayload {
                    cause: if oos { RxOverflowCause::Sequence } else { RxOverflowCause::Overrun },
                    lost: lost_n,
                    restart_gap_ns: gap_ns,
                })
                .unwrap();
                self.emitter.emit("rx", ezsdr_radio::kinds::RX_OVERFLOW, Severity::Warning, p, TimePoint::new(self.domain, expected));
                lock(&self.rec).timing.push(json!({ "what": "overflow", "at_k": expected, "lost": lost_n, "restart_gap_ns": gap_ns }));
            }
            let n = len as usize;
            let bytes = pool.fill(self.channels * n * 8, |buf| {
                for (c, ch) in samples.iter().enumerate() {
                    for (i, s) in ch.iter().enumerate() {
                        write_cf32(buf, n, c, i, s[0], s[1]);
                    }
                }
            });
            let header = BlockHeader {
                first_sample_time: TimePoint::new(self.domain, k),
                len: n as u32,
                channels: self.channels as u16,
                direction: Direction::Rx,
                valid: ChannelMask::full(self.channels as u16),
                flags,
                lost,
                contract: DataContractId::parse("ezsdr.stream.cf32").unwrap(),
            };
            match SampleBlock::new_host(header, HOST_MEMORY, bytes, 8) {
                Ok(block) => {
                    let block = Arc::new(block);
                    for l in &self.links {
                        if l.publish(block.clone()) != PublishOutcome::Accepted {
                            lock(&self.rec).add("link_drops_seen", 1);
                        }
                    }
                }
                Err(e) => lock(&self.rec).rx_errors.push(json!({ "block": e.to_string() })),
            }
            {
                let mut r = lock(&self.rec);
                r.add("rx_blocks", 1);
                r.add("rx_samples", len);
            }
            flags = BlockFlags::NONE;
            expected = k + len;
        }
    }
}

/// Schedules a wakeup at `at` whose callback schedules the next one `period` later,
/// until `stop` is set.
fn heartbeat(time: Arc<dyn TimeAuthority>, at: TimePoint, period: i64, stop: Arc<AtomicBool>) -> Result<(), TimeError> {
    let t = time.clone();
    time.schedule(
        at,
        Box::new(move |fired| {
            if !stop.load(Ordering::Acquire) {
                let _ = heartbeat(t, TimePoint::new(fired.domain, fired.ticks + period), period, stop);
            }
        }),
    )
    .map(|_| ())
}

// ---------------------------------------------------------------- control

struct Control {
    actions: Arc<dyn ActionReceiver>,
    inputs: Arc<dyn InputStore>,
    clocks: Arc<ClockRegistry>,
    config: BTreeMap<Key, Value>,
    spawn: TxSpawn,
    stop: Arc<AtomicBool>,
    rx_stop: Arc<AtomicBool>,
    id: ResourceId,
    rx_chans: usize,
    /// The receive clock's (origin, ratio), so a late-started transmit clock can be
    /// put on the same grid (finding K3).
    rx_grid: Option<(i64, i64)>,
    min_lead_ns: i64,
}

impl Control {
    fn run(self) {
        while !self.stop.load(Ordering::Acquire) {
            let mut any = false;
            while let Some(a) = self.actions.recv() {
                any = true;
                self.handle(a);
            }
            if !any {
                std::thread::sleep(StdDuration::from_millis(1));
            }
        }
    }

    fn now(&self) -> TimePoint {
        let (t, root) = (&self.spawn.time, self.spawn.root);
        t.now(root).unwrap_or(TimePoint::new(root, 0))
    }

    fn rejected(&self, action: &str, reason: &str) {
        self.spawn.emitter.command_rejected(action, reason, self.now());
    }

    fn handle(&self, action: Action) {
        let now = self.now();
        let rate = self.spawn.rate;
        match action {
            Action::TxBurst { target, waveform, repeat, at, requested_at, late_policy, metadata } => {
                let Some((domain, origin, ratio, chans, cmd)) =
                    lock(&self.spawn.state).as_ref().map(|t| (t.domain, t.origin, t.ratio, t.chans, t.cmd.clone()))
                else {
                    return self.rejected("tx_burst", "uhd: no transmit channel");
                };
                let unit = chans as u64 * 8;
                if target != self.id.child("tx").unwrap()
                    || at.time_point.domain != domain
                    || waveform.size_bytes == 0
                    || waveform.size_bytes % unit != 0
                    || !metadata.is_empty()
                {
                    return self.rejected("tx_burst", "uhd: target, clock, size or metadata invalid");
                }
                let Some(bytes) = self.inputs.get(&waveform.hash) else {
                    return self.rejected("tx_burst", "uhd: waveform bytes are not a Run input");
                };
                // Frames of `chans` interleaved cf32 (RM-13) → planar per channel.
                let mut samples = vec![Vec::with_capacity(bytes.len() / unit as usize); chans];
                for frame in bytes.chunks_exact(unit as usize) {
                    for (c, s) in frame.chunks_exact(8).enumerate() {
                        samples[c].push([
                            f32::from_le_bytes(s[0..4].try_into().unwrap()),
                            f32::from_le_bytes(s[4..8].try_into().unwrap()),
                        ]);
                    }
                }
                let now_tx = match self.clocks.convert(now, domain) {
                    Ok(Converted::Exact { point }) => point,
                    Ok(Converted::Inexact { floor, .. }) => TimePoint::new(domain, floor.ticks + 1),
                    Err(e) => return self.rejected("tx_burst", &format!("uhd: {e}")),
                };
                let lead = Duration::new(ClockDomainId::HOST_MONOTONIC, self.min_lead_ns);
                let policy = match late_policy.decide(&self.clocks, at.time_point, now_tx, lead) {
                    Ok(p) => p,
                    Err(e) => return self.rejected("tx_burst", &format!("uhd: {e}")),
                };
                let lead_ticks = self.min_lead_ns as i128 * rate as i128 / 1_000_000_000;
                let ahead_ns = (at.time_point.ticks - now_tx.ticks) as i128 * ratio as i128 * 1_000_000_000 / rate as i128;
                lock(&self.spawn.rec).timing.push(json!({
                    "what": "tx_burst_admitted", "target_k": at.time_point.ticks, "now_k": now_tx.ticks,
                    "ahead_ns": ahead_ns as i64, "policy": format!("{policy:?}"),
                }));
                let k = match policy {
                    LateOutcome::OnTime {} => at.time_point.ticks,
                    LateOutcome::SendAsap { late_by } => {
                        let earliest = now.ticks + lead_ticks as i64;
                        let k = (earliest - origin + ratio - 1).div_euclid(ratio);
                        self.time_error(TimeErrorOutcome::SendAsap, late_by, at.time_point);
                        k
                    }
                    LateOutcome::Drop { late_by } => return self.time_error(TimeErrorOutcome::Drop, late_by, at.time_point),
                    LateOutcome::PlanViolation { late_by } => {
                        return self.time_error(TimeErrorOutcome::PlanViolation, late_by, at.time_point);
                    }
                };
                let open = BurstOpen {
                    waveform_len: u32::try_from(samples[0].len()).ok(),
                    late: (!matches!(policy, LateOutcome::OnTime {})).then_some(policy),
                    requested_target: requested_at.map(|d| d.time_point),
                };
                if cmd.send(TxCmd::Burst { k, samples, repeat, open }).is_err() {
                    self.rejected("tx_burst", "uhd: the transmit thread has ended");
                }
            }
            Action::UpdateParameter { target, key, value, .. } if key.as_str() == keys::TX_CHANNELS && target == self.id => {
                self.start_tx(value, now);
            }
            Action::UpdateParameter { target, key, value, at, .. } => {
                let tx_chans = lock(&self.spawn.state).as_ref().map_or(0, |t| t.chans);
                let dir_of = |k: &str| match k {
                    keys::RX_FREQUENCY_HZ | keys::RX_GAIN_DB => Some((Dir::Rx, self.rx_chans)),
                    keys::TX_FREQUENCY_HZ | keys::TX_GAIN_DB => Some((Dir::Tx, tx_chans)),
                    _ => None,
                };
                let v = match value {
                    Value::Num(v) => v,
                    Value::Int(v) => v as f64,
                    _ => f64::NAN,
                };
                let Some((dir, n)) = dir_of(key.as_str()).filter(|_| target == self.id && v.is_finite()) else {
                    return self.rejected("update_parameter", "uhd: only frequency, gain and 0→N tx channels update at runtime");
                };
                let is_freq = key.as_str().ends_with("frequency_hz");
                let s = Settings { freq: is_freq.then_some(v), gain: (!is_freq).then_some(v), ..Settings::default() };
                let tick = at.and_then(|d| match self.clocks.convert(d.time_point, self.spawn.root) {
                    Ok(Converted::Exact { point }) => Some(point.ticks),
                    Ok(Converted::Inexact { floor, .. }) => Some(floor.ticks + 1),
                    Err(_) => None,
                });
                for chan in 0..n {
                    match self.spawn.device.apply(dir, chan, &s, tick) {
                        Ok(a) => lock(&self.spawn.rec).timing.push(json!({
                            "what": "update", "key": key, "value": v, "at": tick, "now": now.ticks, "device": format!("{a:?}"),
                        })),
                        Err(e) => return self.rejected("update_parameter", &e),
                    }
                }
            }
            Action::Stop { target } => {
                let rx = self.id.child("rx").unwrap();
                let tx = self.id.child("tx").unwrap();
                let whole = target.is_none() || target.as_ref() == Some(&self.id);
                if whole || target.as_ref() == Some(&rx) {
                    self.rx_stop.store(true, Ordering::Release);
                }
                if whole || target.as_ref() == Some(&tx) {
                    if let Some(t) = lock(&self.spawn.state).as_ref() {
                        let _ = t.cmd.send(TxCmd::Stop);
                    }
                }
                if !whole && target.as_ref() != Some(&rx) && target.as_ref() != Some(&tx) {
                    self.rejected("stop", "uhd: Stop target is not this device or its stream");
                }
            }
            other => {
                let name = format!("{other:?}");
                let name = name.split([' ', '{']).next().unwrap_or("action").to_owned();
                self.rejected(&name, "uhd: Action not supported");
            }
        }
    }

    /// A cold `radio.tx.channels` change from 0: configure, open and start the
    /// transmit side now, as MockRadio's cold change re-registers its clock (MR-18).
    /// It runs on this thread, after `submit` returned: the next Session Action is
    /// admitted before the clock exists unless the caller waits (spike finding K5).
    fn start_tx(&self, value: Value, now: TimePoint) {
        let n = match value {
            Value::Int(n) if n >= 1 => n as usize,
            _ => return self.rejected("update_parameter", "uhd: radio.tx.channels must go from 0 to N ≥ 1"),
        };
        if lock(&self.spawn.state).is_some() {
            return self.rejected("update_parameter", "uhd: the transmit side is already running");
        }
        let dev = &self.spawn.device;
        let s = Settings {
            rate: Some(num(&self.config, keys::TX_SAMPLE_RATE_HZ)),
            freq: Some(num(&self.config, keys::TX_FREQUENCY_HZ)),
            gain: Some(num(&self.config, keys::TX_GAIN_DB)),
            antenna: string(&self.config, keys::TX_ANTENNA),
        };
        let mut rate = 0.0;
        for chan in 0..n {
            match dev.apply(Dir::Tx, chan, &s, None) {
                Ok(a) => rate = a.rate,
                Err(e) => return self.rejected("update_parameter", &e),
            }
        }
        let tick_rate = dev.tick_rate() as f64;
        let ratio = (tick_rate / rate).round() as i64;
        let result = (|| -> Result<(ClockDomainId, i64), String> {
            let stream = self.id.child("tx").map_err(|e| e.to_string())?;
            let h = self.clocks.declare_sample_clock(stream, self.spawn.root, Rational::new(ratio as u64, 1).unwrap()).map_err(|e| e.to_string())?;
            dev.tx_open(n)?;
            let origin = match self.rx_grid {
                Some((o, r)) if r == ratio => o + (now.ticks - o + r - 1).div_euclid(r) * r,
                _ => now.ticks,
            };
            Ok((self.clocks.register_sample_clock(&h, origin).map_err(|e| e.to_string())?, origin))
        })();
        match result {
            Ok((domain, origin)) => {
                self.spawn.spawn(domain, origin, ratio, n);
                lock(&self.spawn.rec).timing.push(json!({ "what": "tx_started_by_cold_change", "origin": origin, "ratio": ratio }));
            }
            Err(e) => self.rejected("update_parameter", &format!("uhd: {e}")),
        }
    }

    fn time_error(&self, outcome: TimeErrorOutcome, late_by: Duration, target: TimePoint) {
        let p = serde_json::to_value(TimeErrorPayload {
            cause: TimeErrorCause::Late,
            outcome,
            late_by_ns: late_by.ticks,
            target,
        })
        .unwrap();
        self.spawn.emitter.emit("tx", ezsdr_radio::kinds::TIME_ERROR, Severity::Error, p, self.now());
        if outcome != TimeErrorOutcome::SendAsap {
            lock(&self.spawn.rec).rejected.push(json!({ "action": "tx_burst", "reason": format!("late: {outcome:?}"), "target": target }));
        }
    }
}

// ---------------------------------------------------------------- tx

struct TxJob {
    device: Arc<dyn Device>,
    rec: Arc<Mutex<Records>>,
    stop: Arc<AtomicBool>,
    cmds: mpsc::Receiver<TxCmd>,
    domain: ClockDomainId,
    origin: i64,
    ratio: i64,
    rate: i64,
    time: Arc<dyn TimeAuthority>,
    root: ClockDomainId,
}

impl TxJob {
    fn header(&self, k: i64, len: usize, channels: usize, flags: BlockFlags) -> BlockHeader {
        BlockHeader {
            first_sample_time: TimePoint::new(self.domain, k),
            len: len as u32,
            channels: channels as u16,
            direction: Direction::Tx,
            valid: ChannelMask::full(channels as u16),
            flags,
            lost: None,
            contract: DataContractId::parse("ezsdr.stream.cf32").unwrap(),
        }
    }

    fn record(&self, step: Result<BurstStep, ezsdr_kernel::stream::StreamError>) {
        match step {
            Ok(BurstStep::Ended { record }) => lock(&self.rec).bursts.push(serde_json::to_value(record).unwrap()),
            Ok(BurstStep::Discontinuity { closed, then_ended, .. }) => {
                let mut r = lock(&self.rec);
                r.bursts.push(serde_json::to_value(closed).unwrap());
                if let Some(e) = then_ended {
                    r.bursts.push(serde_json::to_value(e).unwrap());
                }
            }
            Ok(_) => {}
            Err(e) => lock(&self.rec).rejected.push(json!({ "burst_tracker": e.to_string() })),
        }
    }

    fn run(self) {
        let mut tracker = BurstTracker::new(self.domain);
        loop {
            let cmd = match self.cmds.recv_timeout(StdDuration::from_millis(10)) {
                Ok(c) => c,
                Err(mpsc::RecvTimeoutError::Timeout) if !self.stop.load(Ordering::Acquire) => continue,
                Err(_) => break,
            };
            let (k, samples, repeat, open) = match cmd {
                TxCmd::Burst { k, samples, repeat, open } => (k, samples, repeat, open),
                TxCmd::Stop => continue,
                TxCmd::Shutdown => break,
            };
            let at = self.origin + k * self.ratio;
            let len = samples[0].len();
            let channels = samples.len();
            let refs: Vec<&[Iq]> = samples.iter().map(|s| s.as_slice()).collect();
            let now = self.time.now(self.root).map(|t| t.ticks).unwrap_or(at);
            let wait_s = ((at - now).max(0) as f64 + (len as i64 * self.ratio) as f64) / self.rate as f64 + 1.0;
            let sob_flags = BlockFlags::START_OF_BURST | if repeat { BlockFlags::NONE } else { BlockFlags::END_OF_BURST };
            let sent = self.device.tx_send(&refs, Some(at), true, !repeat, wait_s);
            {
                let mut r = lock(&self.rec);
                r.add("tx_bursts", 1);
                r.timing.push(json!({ "what": "tx_first_send", "k": k, "at": at, "now": now, "len": len, "repeat": repeat, "sent": format!("{sent:?}") }));
            }
            if sent.is_err() {
                continue;
            }
            self.record(tracker.on_block(&self.header(k, len, channels, sob_flags), Some(open)));
            if !repeat {
                continue;
            }
            // Continuous repeat: the same waveform back to back until stopped; the wrap
            // must be seamless (Vision §61 v3 behaviour 1).
            let mut next = k + len as i64;
            let mut exit = false;
            loop {
                match self.cmds.try_recv() {
                    Ok(TxCmd::Stop) => break,
                    Ok(TxCmd::Shutdown) | Err(mpsc::TryRecvError::Disconnected) => {
                        exit = true;
                        break;
                    }
                    Ok(TxCmd::Burst { .. }) => {
                        lock(&self.rec).rejected.push(json!({ "action": "tx_burst", "reason": "uhd: a repeat is running" }));
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                }
                if self.device.tx_send(&refs, None, false, false, 1.0).is_err() {
                    break;
                }
                self.record(tracker.on_block(&self.header(next, len, channels, BlockFlags::NONE), None));
                next += len as i64;
            }
            let empty: Vec<&[Iq]> = vec![&[]; channels];
            let _ = self.device.tx_send(&empty, None, false, true, 1.0);
            if let Some(r) = tracker.stop() {
                lock(&self.rec).bursts.push(serde_json::to_value(r).unwrap());
            }
            if exit {
                break;
            }
        }
    }
}

fn async_loop(dev: Arc<dyn Device>, em: Emitter, rec: Arc<Mutex<Records>>, stop: Arc<AtomicBool>, root: ClockDomainId) {
    while !stop.load(Ordering::Acquire) {
        let Some(e) = dev.tx_async(0.1) else { continue };
        lock(&rec).device_async.push(json!({ "code": e.code, "tick": e.tick, "channel": e.channel }));
        let at = TimePoint::new(root, e.tick.unwrap_or(0));
        match e.code {
            "time_error" => {
                // The device dropped a burst it received late. The Provider had already
                // admitted it, so none of RM-22's four outcomes says "the device refused".
                let p = serde_json::to_value(TimeErrorPayload {
                    cause: TimeErrorCause::Late,
                    outcome: TimeErrorOutcome::Refused,
                    late_by_ns: 0,
                    target: at,
                })
                .unwrap();
                em.emit("tx", ezsdr_radio::kinds::TIME_ERROR, Severity::Error, p, at);
            }
            "underflow" | "underflow_in_packet" => {
                em.emit("tx", ezsdr_radio::kinds::TX_UNDERFLOW, Severity::Warning, json!({ "device_code": e.code }), at);
            }
            _ => {}
        }
    }
}
