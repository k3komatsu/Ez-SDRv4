//! `UhdRadio`: the Radio Provider role of `ezsdr.radio.uhd` (UR-5, UR-10…UR-16,
//! UR-26, UR-30). It is not stepped; three threads of its own do the work.

mod control;
mod core;
mod mode;
mod rx;
mod tx;
#[cfg(test)]
mod test_support;
/// VH-8's seeded sequences (spec 22), shared with the other crates' tests by path.
#[cfg(test)]
#[path = "../../../ezsdr-radio/tests/generator/mod.rs"]
mod generator;

use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration as Wall, Instant};

use ezsdr_hostmem::HOST_MEMORY;
use ezsdr_kernel::binding::Binding;
use ezsdr_kernel::event::{EventSource, Severity};
use ezsdr_kernel::id::{ClockDomainId, ResourceId};
use ezsdr_kernel::module_api::{
    ActionReceiver, CoerceReport, Driving, Endpoint, ExecutionClass, ModuleError,
    ModuleErrorKind, PrepareContext, Provider, ProviderInstance, Requested, Role, StopMode,
};
use ezsdr_kernel::plan::{Fragment, PrepareReport};
use ezsdr_kernel::spec::{Ident, Key, Namespace, Scalar, Value};
use ezsdr_kernel::stream::BackPressure;
use ezsdr_kernel::time::{Duration, SampleClockHandle, TimePoint};
use ezsdr_radio::channels;
use ezsdr_radio::payloads::LateCommandPayload;
use ezsdr_radio::timeline::Channels;
use ezsdr_radio::x3xx;
use ezsdr_radio::kinds;
use serde_json::{Value as Json, json};

use self::core::{Clock, Core, lock};
use self::control::Control;
use self::mode::DescriptionExt;
use self::rx::{Rx, RxCmd};
use self::tx::{Tx, TxCmd};
use crate::device::{Device, Dir, Settings};
use crate::profile::{self, MASTER_CLOCK_HZ};

fn rejected(message: impl Into<String>) -> ModuleError {
    ModuleError::rejected(message)
}

/// The refusal `arm` gives a failed open (UR-13). A rate refusal from `check_rates` already
/// carries its `UR-13:` label, so only an open's device error gets one here.
fn open_refusal(error: crate::device::DeviceError) -> String {
    if error.message.starts_with("UR-13:") { error.message } else { format!("UR-13: {}", error.message) }
}

/// `ezsdr.radio.uhd.<suffix>`; the Kernel files it under the instance's source root (RS-39).
fn section(suffix: &str) -> Namespace {
    Namespace::parse(&format!("ezsdr.radio.uhd.{suffix}")).expect("a section name")
}

fn description_surfaces(description: &channels::Description, id: &ResourceId) -> (ezsdr_kernel::module_api::Resource, Duration, Json) {
    let timing = description.timing;
    (
        description.tree(id, HOST_MEMORY),
        Duration::new(ClockDomainId::HOST_MONOTONIC, timing.min_timed_command_lead_ns),
        serde_json::to_value(description.envelope()).expect("an envelope"),
    )
}

const JOIN_BOUND: Wall = Wall::from_secs(1);

/// A receive stream's owner thread (UR-15, design 33 D15): one per declared stream, in
/// both key sets.
fn rx_name(k: usize) -> String {
    format!("uhd-rx-s{k}")
}

/// A transmit stream's owner thread (UR-15, design 33 D15).
fn tx_name(k: usize) -> String {
    format!("uhd-tx-s{k}")
}

/// A stream the layout has, as `prepare` declared it (RM-29, UR-12): its number, the
/// SampleClock handle when it has channels, its channel set, and the transmit clock `arm`
/// registered at its origin (UR-13).
#[derive(Clone)]
struct PreparedStream {
    k: usize,
    handle: Option<SampleClockHandle>,
    channels: Channels,
    clock: Option<Clock>,
}

struct Prepared {
    core: Arc<Core>,
    actions: Arc<dyn ActionReceiver>,
    config: BTreeMap<Key, Value>,
    /// One entry per receive stream of the layout (RM-29).
    rx: Vec<PreparedStream>,
    /// One entry per transmit stream of the layout.
    tx: Vec<PreparedStream>,
}

struct Running {
    /// One sender per transmit stream: `Provider::stop` shuts every transmit stream down
    /// before the first receive one (RM-16, design 33 D15).
    to_tx: Vec<Sender<TxCmd>>,
    /// One sender per receive stream.
    to_rx: Vec<Sender<RxCmd>>,
    threads: Vec<(String, JoinHandle<()>)>,
}

/// The serials of the live `UhdRadio`s of this process (UR-5, design 33 D5): a second
/// binding of one device is refused before it touches the device, and dropping the
/// instance releases its serial.
fn serials() -> &'static std::sync::Mutex<BTreeSet<String>> {
    static SERIALS: std::sync::OnceLock<std::sync::Mutex<BTreeSet<String>>> = std::sync::OnceLock::new();
    SERIALS.get_or_init(|| std::sync::Mutex::new(BTreeSet::new()))
}

/// One live instance's hold on the device's serial (UR-5): the check and the insert under
/// one lock (`register_serial`, so that two threads cannot both pass), the release on
/// drop — the device need not be reachable then.
struct SerialGuard {
    serial: String,
}

impl Drop for SerialGuard {
    fn drop(&mut self) {
        serials().lock().unwrap_or_else(|e| e.into_inner()).remove(&self.serial);
    }
}

/// Registers `serial` for one live `UhdRadio`, or refuses a second (UR-5, design 33 D5).
fn register_serial(serial: &str) -> Result<SerialGuard, ModuleError> {
    let mut live = serials().lock().unwrap_or_else(|e| e.into_inner());
    if !live.insert(serial.to_owned()) {
        return Err(rejected(format!("UR-5: the serial {serial} is already bound by a live ezsdr.radio.uhd instance")));
    }
    drop(live);
    Ok(SerialGuard { serial: serial.to_owned() })
}

/// The subdev spec markup of a direction's elements (UR-5, H2): `<slot>:<front end>` per
/// element, in the layout's order, one ASCII space between them (design 33 D5).
fn subdev_markup(elements: &[(x3xx::Slot, u32)]) -> String {
    elements
        .iter()
        .map(|(slot, fe)| format!("{}:{fe}", if *slot == x3xx::Slot::A { "A" } else { "B" }))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The `(slot, front end)` pairs of a layout's direction, in its order (design 33 D5).
fn layout_elements(elements: &[channels::Element]) -> Vec<(x3xx::Slot, u32)> {
    elements.iter().map(|element| (element.slot, element.frontend)).collect()
}

/// UR-5 (design 33 D5): whether the device's front end name `found` is the layout's
/// element — its board's name as UHD begins it, and, for a Basic/LF board, the front end
/// number its name carries (`BasicRX (1)`, `db_basic_and_lf.cpp:101-111`).
fn front_end_matches(element: &channels::Element, found: &str) -> bool {
    match element.front_end.board() {
        Some(board) => found.starts_with(board.uhd_name()),
        None => {
            let base = match element.front_end {
                x3xx::FrontEnd::BasicRx => "BasicRX",
                x3xx::FrontEnd::LfRx => "LFRX",
                x3xx::FrontEnd::BasicTx => "BasicTX",
                x3xx::FrontEnd::LfTx => "LFTX",
                _ => return false,
            };
            found.starts_with(base) && found.contains(&format!("({})", element.frontend))
        }
    }
}

/// The UHD Radio Provider (spec 18).
pub struct UhdRadio {
    instance: ProviderInstance,
    device: Arc<dyn Device>,
    clock_source: String,
    /// The epoch of the root its own Authority registers (UR-6).
    epoch: ezsdr_kernel::time::EpochRef,
    /// The description built from this binding's layout (RM-29).
    description: channels::Description,
    /// Its hold on the device's serial (UR-5): released when this instance drops.
    _serial: SerialGuard,
    rx_stall: Option<(Wall, Wall)>,
    /// The first open's error when the device was opened again on it (UR-7's reopen).
    reopened: Option<String>,
    prepare_called: bool,
    prepared: Option<Prepared>,
    running: Option<Running>,
    stopped: bool,
    cleaned: bool,
    detached: BTreeSet<String>,
}

impl UhdRadio {
    /// Builds the Provider from its binding and the device the runtime opened for it
    /// (UR-5, UR-6, UR-10). The order is the selector, the serial, the master clock, the
    /// subdev spec and the front ends (design 33 D5's opening checks).
    pub fn from_binding(binding: &Binding, device: Arc<dyn Device>) -> Result<UhdRadio, ModuleError> {
        if binding.module != crate::module_ref() {
            return Err(rejected("UR-5: the binding names another Module"));
        }
        let Some(profile_ref) = binding.profile.as_ref() else {
            return Err(rejected("UR-5: the profile must be x3xx 0.1.0"));
        };
        if profile_ref != &profile::profile_ref() {
            return Err(rejected("UR-5: the profile must be x3xx 0.1.0"));
        }
        if binding.feed.is_some() {
            return Err(rejected("UR-5: a Provider binding carries no feed"));
        }
        let allowed: Vec<&str> = x3xx::SELECTOR_KEYS
            .into_iter()
            .chain(["args", "id", "clock_source", "time_source", "block_len", "reopen_on_unlock"])
            .collect();
        if let Some(name) = binding.selector.keys().find(|k| !allowed.contains(&k.as_str())) {
            return Err(rejected(format!("UR-5: unsupported selector key `{name}`")));
        }
        let get = |name: &str| binding.selector.get(&Ident::parse(name).expect("a selector key"));
        let string = |name: &str| match get(name) {
            None => Ok(None),
            Some(Value::Scalar(Scalar::Str(s))) => Ok(Some(s.clone())),
            Some(_) => Err(rejected(format!("UR-5: selector `{name}` must be a string"))),
        };
        let args = string("args")?.ok_or_else(|| rejected("UR-5: the selector needs `args`"))?;
        let id = string("id")?.unwrap_or_else(|| "usrp".to_owned());
        let resource = ResourceId::parse(&id)
            .ok()
            .filter(|resource| resource.segments().count() == 1)
            .ok_or_else(|| rejected("UR-5: selector `id` must be one resource segment"))?;
        let source = |name: &str| -> Result<String, ModuleError> {
            let value = string(name)?.unwrap_or_else(|| "internal".to_owned());
            if ["internal", "external", "gpsdo"].contains(&value.as_str()) {
                Ok(value)
            } else {
                Err(rejected(format!("UR-5: selector `{name}` must be internal, external or gpsdo")))
            }
        };
        let clock_source = source("clock_source")?;
        let epoch = crate::authority::epoch(&clock_source, &source("time_source")?, &args);
        if get("reopen_on_unlock").is_some_and(|v| !matches!(v, Value::Scalar(Scalar::Bool(_)))) {
            return Err(rejected("UR-5: selector `reopen_on_unlock` must be a Bool"));
        }
        let block_len = match get("block_len") {
            None => profile::DEFAULT_BLOCK_LEN,
            Some(Value::Scalar(Scalar::Int(n))) if (1..=65_536).contains(n) => *n as u32,
            Some(_) => return Err(rejected("UR-5: selector `block_len` must be an Int in 1..=65536")),
        };
        let layout = channels::from_selector(&binding.selector).map_err(|why| rejected(format!("UR-5: {why}")))?;
        // UR-5, design 33 D5: the serial is the device's identity and the first device
        // call; a live instance already bound to it is refused before anything is written
        // to the device (H4: a second binding would silently overwrite the first's spec).
        let serial = device.serial().map_err(|e| rejected(format!("UR-5: {}", e.message)))?;
        let registration = register_serial(&serial)?;
        let mcr = device.master_clock_rate();
        if mcr != MASTER_CLOCK_HZ {
            return Err(rejected(format!(
                "UR-5: profile {} needs a 200 MHz master clock; the device runs at {mcr} Hz",
                x3xx::PROFILE
            )));
        }
        let description = channels::Description::new(&layout, block_len, &profile::terms());
        // UR-5, H1, H2: the device's channel map is set from the layout — one call per
        // direction with elements, `<slot>:<front end>` in logical order — before its
        // channels and front ends are read; a direction with no element keeps its map.
        let (rx_elements, tx_elements) = match &layout {
            channels::Layout::X3xx { rx, tx, .. } => (layout_elements(rx), layout_elements(tx)),
            _ => unreachable!("UR-5: a UHD binding's layout is x3xx"),
        };
        let mut subdev_spec = BTreeMap::new();
        for (dir, elements) in [(Dir::Rx, &rx_elements), (Dir::Tx, &tx_elements)] {
            if elements.is_empty() {
                subdev_spec.insert(dir.name().to_owned(), Json::Null);
                continue;
            }
            let markup = subdev_markup(elements);
            device.set_subdev_spec(dir, &markup).map_err(|e| rejected(format!("UR-5: {}", e.message)))?;
            subdev_spec.insert(dir.name().to_owned(), json!(markup));
        }
        // The layout's front ends must be the device's: UHD clips a tune outside its front
        // end's range without a warning, past the RF envelope's check. Only the layout's
        // channels: slot B empty is a channel on UHD's unknown board. The spec set from the
        // elements above is what makes the device's channels those elements' (H1): a device
        // with fewer refuses the spec itself, with UHD's text.
        let channels::Layout::X3xx { rx, tx, .. } = &layout else {
            unreachable!("UR-5: a UHD binding's layout is x3xx")
        };
        for (dir, name, elements) in [(Dir::Rx, "rx", rx), (Dir::Tx, "tx", tx)] {
            for (chan, element) in elements.iter().enumerate() {
                let front_end = device.front_end(dir, chan).map_err(|e| rejected(format!("UR-5: {}", e.message)))?;
                if !front_end_matches(element, &front_end) {
                    return Err(rejected(format!(
                        "UR-5: the layout's {name} channel {chan} is `{front_end}`, not `{}` front end {}",
                        element.front_end.name(),
                        element.frontend
                    )));
                }
            }
        }
        let mut describe = device.describe();
        if let Some(object) = describe.as_object_mut() {
            object.insert("args".to_owned(), json!(args));
            object.insert("serial".to_owned(), json!(serial));
            object.insert("subdev_spec".to_owned(), serde_json::to_value(subdev_spec).expect("a spec object"));
            object.insert("rates_checked".to_owned(), json!({ "rx": false, "tx": false }));
        }
        let mut sections = BTreeMap::new();
        sections.insert(section("device"), describe);
        let (tree, min_command_lead, envelope) = description_surfaces(&description, &resource);
        sections.insert(section("envelope"), envelope);
        let instance = ProviderInstance {
            id: resource.clone(),
            module: crate::module_ref(),
            profile: binding.profile.clone(),
            tree,
            fidelity: device.fidelity(),
            driving: Driving { stepped: false },
            arm_after: Vec::new(),
            min_command_lead: Some(min_command_lead),
            sections,
        };
        Ok(UhdRadio {
            instance,
            device,
            clock_source,
            epoch,
            description,
            _serial: registration,
            rx_stall: None,
            reopened: None,
            prepare_called: false,
            prepared: None,
            running: None,
            stopped: false,
            cleaned: false,
            detached: BTreeSet::new(),
        })
    }

    /// Notes that this device was opened again after the first open's reference did not
    /// lock (UR-7's reopen, done by whoever builds the Assembly): recorded in `timing`.
    pub fn with_reopened(mut self, first_error: String) -> UhdRadio {
        self.reopened = Some(first_error);
        self
    }

    /// Makes uhd-rx stop calling `rx_recv` once, for `duration`, `after` its first
    /// block: a real overflow for bench step B5 (UR-34), on stream 0, the one the current
    /// keys declare. Only Rust code reaches it (GZ-9).
    pub fn with_rx_stall(mut self, after: Wall, duration: Wall) -> UhdRadio {
        self.rx_stall = Some((after, duration));
        self
    }

    /// Rebuilds this measurement instance with candidate timing values; the ordinary
    /// construction path continues to use the profile's terms.
    ///
    /// # Panics
    ///
    /// Panics when `timing` breaks RM-6, as a description built by `Description::new` would.
    pub fn with_measurement_timing(mut self, timing: ezsdr_radio::TimingEnvelope) -> UhdRadio {
        timing.check_start_lead().unwrap_or_else(|error| panic!("{error}"));
        self.description.timing = timing;
        let (tree, min_command_lead, envelope) = description_surfaces(&self.description, &self.instance.id);
        self.instance.tree = tree;
        self.instance.min_command_lead = Some(min_command_lead);
        self.instance.sections.insert(section("envelope"), envelope);
        self
    }

    /// Rebuilds this measurement instance with a candidate PerformanceEnvelope (RM-32), so that a Run can be
    /// admitted above the profile's link limit: an admission ceiling for a measurement, not the link's
    /// capacity. The ordinary construction path continues to use the profile's link.
    pub fn with_measurement_performance(mut self, performance: ezsdr_radio::PerformanceEnvelope) -> UhdRadio {
        self.description.performance = performance;
        let timing = self.description.timing;
        self.with_measurement_timing(timing)
    }

    fn prepared(&self) -> Result<&Prepared, ModuleError> {
        self.prepared.as_ref().ok_or_else(|| rejected("UR-12: prepare must come first"))
    }

    fn spawn(&mut self, core: &Arc<Core>, name: String, body: impl FnOnce() + Send + 'static) -> Result<(), ModuleError> {
        let core_ = core.clone();
        // §4.2, M1: the host clock registers the participant before the thread runs.
        let host = core.device.host();
        let reported = name.clone();
        let is_control = name == "uhd-control";
        let handle = host
            .spawn(&name, move || {
                if catch_unwind(AssertUnwindSafe(body)).is_err() {
                    // UR-16: a panic is recorded; the stream the thread served is gone.
                    core_.reject_note(json!({ "thread": reported, "reason": "panicked" }));
                    if !is_control {
                        core_.device_lost("a Provider thread panicked");
                    }
                }
            })
            .map_err(|e| rejected(format!("UR-15: {name}: {e}")))?;
        if let Some(running) = self.running.as_mut() {
            running.threads.push((name, handle));
        }
        Ok(())
    }

    fn join(&mut self, core: &Core, name: &str) {
        let Some(running) = self.running.as_mut() else { return };
        let Some(position) = running.threads.iter().position(|(n, _)| n == name) else { return };
        let (name, handle) = running.threads.remove(position);
        let until = Instant::now() + JOIN_BOUND;
        while !handle.is_finished() && Instant::now() < until {
            std::thread::sleep(Wall::from_millis(1));
        }
        if handle.is_finished() {
            let _ = handle.join();
        } else {
            // UR-16: left detached; its streamer is not closed in cleanup.
            core.reject_note(json!({ "thread": name, "reason": "did not join within 1 s; left detached" }));
            self.detached.insert(name);
        }
    }

    fn write_sections(&mut self, core: &Core) {
        let rec = lock(&core.rec);
        let sections = &mut self.instance.sections;
        let mut put = |suffix: &str, value: Json| {
            sections.insert(section(suffix), value);
        };
        put("applied", Json::Array(rec.applied.clone()));
        put("bursts", Json::Array(rec.bursts.clone()));
        put("async", Json::Array(rec.device_async.clone()));
        put("rejected", Json::Array(rec.rejected.clone()));
        put("timing", Json::Array(rec.timing.clone()));
        put("stats", json!(rec.stats));
        // UR-13, UR-30: whether each direction's rate set was compared; the section keeps
        // both fields whatever the answer.
        if let Some(device) = sections.get_mut(&section("device")).and_then(|value| value.as_object_mut()) {
            device.insert("rates_checked".to_owned(), json!({ "rx": core.rates_checked(Dir::Rx), "tx": core.rates_checked(Dir::Tx) }));
        }
    }
}

impl Provider for UhdRadio {
    fn instance(&self) -> &ProviderInstance {
        &self.instance
    }

    /// RM-29's coercion over the profile: pure, no device call (UR-11, MA-11).
    fn coerce(&self, request: &Requested) -> Result<CoerceReport, ModuleError> {
        self.description.coerce(&self.instance.id, request)
    }

    fn prepare(&mut self, f: &Fragment, ctx: PrepareContext) -> Result<PrepareReport, ModuleError> {
        if !matches!(ctx.class, ExecutionClass::HardwareInLoop | ExecutionClass::Hardware) {
            return Err(ModuleError {
                kind: ModuleErrorKind::Unsupported,
                message: format!("UR-12: ezsdr.radio.uhd runs HardwareInLoop or Hardware, not {:?}", ctx.class),
                detail: Json::Null,
            });
        }
        if std::mem::replace(&mut self.prepare_called, true) {
            return Err(rejected("UR-12: prepare was already called; one fragment per instance"));
        }
        if f.instance != crate::module_ref() || f.role != Role::Provider {
            return Err(rejected("UR-12: the fragment is not this Module's Provider"));
        }
        let requested: Requested = f
            .content
            .get("requested")
            .cloned()
            .ok_or_else(|| rejected("UR-12: the fragment has no requested configuration"))
            .and_then(|r| serde_json::from_value(r).map_err(|e| rejected(format!("UR-12: invalid Requested: {e}"))))?;
        let report = self.coerce(&requested)?;
        if let Some(refused) = report.rejected.first() {
            return Err(rejected(format!("UR-12: {}", refused.reason)));
        }
        // UR-6: the primary root is this device's.
        let root = ctx.time.primary_root();
        let epoch = match ctx.clocks.get(root).map(|d| d.kind) {
            Ok(ezsdr_kernel::time::ClockDomainKind::Root { epoch, .. }) => Some(epoch),
            _ => None,
        };
        if epoch.as_ref() != Some(&self.epoch) {
            return Err(rejected("UR-6: the primary root is not this device's; bind the Authority to the radio's own binding"));
        }
        let mut config = self.description.defaults();
        for (key, value) in report.applied {
            if channels::is_configuration(&key) {
                config.insert(key, value);
            }
        }
        // UR-12, RM-29: each receive stream takes its own port's links; a link on another
        // port is refused.
        let rx_count = self.description.stream_count(Dir::Rx);
        let mut links: Vec<Vec<Arc<dyn ezsdr_kernel::stream::DataLink>>> = vec![Vec::new(); rx_count];
        for attached in &ctx.links {
            match &attached.endpoint {
                Endpoint::StreamOut(link) if attached.component == f.id => {
                    let port = attached.port.as_str();
                    let Some(k) = (0..rx_count).find(|k| self.description.rx_port(*k) == port) else {
                        return Err(rejected("UR-12: only a StreamOut link on this fragment's rx port may be attached"));
                    };
                    if link.policy() == BackPressure::Block {
                        return Err(rejected("UR-12: a device cannot wait for a Block link; it overflows instead"));
                    }
                    links[k].push(link.clone());
                }
                _ => return Err(rejected("UR-12: only a StreamOut link on this fragment's rx port may be attached")),
            }
        }
        // KC-8: the Kernel names this instance's source root; its streams are below it.
        let EventSource::Node { node: source_root } = ctx.source else {
            return Err(rejected("KC-8: a Provider's event source root is a node"));
        };
        let core = Arc::new(Core::new(
            self.device.clone(),
            source_root,
            root,
            ctx.time.clone(),
            ctx.clocks.clone(),
            ctx.events.clone(),
            ctx.inputs.clone(),
            links,
            self.description.clone(),
        )?);
        if let Some(error) = &self.reopened {
            core.timing(json!({ "what": "reopened_on_unlock", "first_error": error }));
        }
        // One entry per declared stream (RM-29): a stream with no channel declares no
        // SampleClock (UR-12).
        let prepared = |dir: Dir| -> Result<Vec<PreparedStream>, ModuleError> {
            let mut streams = Vec::new();
            for k in 0..self.description.stream_count(dir) {
                let channels = self.description.stream_channels(&config, dir == Dir::Rx, k as u32);
                let mut stream = PreparedStream { k, handle: None, channels, clock: None };
                if !channels.is_empty() {
                    let rate = self.description.stream_rate(&config, dir, k);
                    let entries: Vec<(usize, Settings)> = channels
                        .iter()
                        .map(|chan| {
                            let mut settings = self.description.settings(&config, dir, k, chan as usize);
                            settings.rate = Some(rate);
                            (chan as usize, settings)
                        })
                        .collect();
                    let n = core.configure(dir, &entries, rate, k).map_err(|e| rejected(e.message))?;
                    let node = self.description.stream_node(&self.instance.id, dir, k);
                    let ratio = ezsdr_kernel::time::Rational::new(n as u64, 1).map_err(|e| rejected(format!("UR-12: {e}")))?;
                    stream.handle = Some(ctx.clocks.declare_sample_clock(node, root, ratio).map_err(|e| rejected(format!("UR-12: {e}")))?);
                }
                streams.push(stream);
            }
            Ok(streams)
        };
        let rx = prepared(Dir::Rx)?;
        let tx = prepared(Dir::Tx)?;
        self.prepared = Some(Prepared { core, actions: ctx.actions, config: config.clone(), rx, tx });
        Ok(PrepareReport { effective: config, coercions: report.coercions, warnings: Vec::new() })
    }

    fn arm(&mut self) -> Result<(), ModuleError> {
        let clock_source = self.clock_source.clone();
        let prepared = self.prepared.as_mut().ok_or_else(|| rejected("UR-13: prepare must precede arm"))?;
        let core = prepared.core.clone();
        let a = core.now();
        let s = a + core.ticks(core.description.timing.startup_latency_ns);
        core.start_up.store(s, Ordering::Release);
        if clock_source != "internal" {
            match self.device.ref_locked() {
                Ok(Some(false)) => return Err(rejected(format!("UR-13: the {clock_source} reference is not locked"))),
                Ok(_) => {}
                Err(error) => return Err(rejected(format!("UR-13: {error}"))),
            }
        }
        // UR-13: the receive streamer of every stream with channels, when its port has a
        // link; a receive stream with no link has no stream. The rates the device reports
        // are the profile's, compared after each direction's first streamer opened
        // (design 33 D5).
        for stream in &mut prepared.rx {
            if stream.channels.is_empty() || core.links(stream.k).is_empty() {
                continue;
            }
            // UR-13: the record says what the streamer now holds; a failed open or rate check holds none.
            core.open_streamer(Dir::Rx, stream.k, stream.channels).map_err(|e| rejected(open_refusal(e)))?;
        }
        for stream in &mut prepared.tx {
            if stream.channels.is_empty() {
                continue;
            }
            core.open_streamer(Dir::Tx, stream.k, stream.channels).map_err(|e| rejected(open_refusal(e)))?;
            // RM-25: the first lattice instant at or after the arm instant.
            let handle = stream.handle.clone().expect("a stream with channels declared its clock");
            let n = handle.root_ticks_per_tick().num() as i64;
            let origin = ezsdr_radio::timeline::lattice(a, handle.root_ticks_per_tick()).map_err(|e| rejected(format!("UR-13: {e}")))?;
            let domain = core.clocks.register_sample_clock(&handle, origin).map_err(|e| rejected(format!("UR-13: {e}")))?;
            stream.clock = Some(Clock { domain, origin, n });
        }
        core.timing(json!({ "what": "arm", "at": a, "start_up_until": s }));
        Ok(())
    }

    fn start(&mut self, at: Option<TimePoint>) -> Result<(), ModuleError> {
        let (core, actions, config, rx_streams, tx_streams) = {
            let prepared = self.prepared()?;
            (
                prepared.core.clone(),
                prepared.actions.clone(),
                prepared.config.clone(),
                prepared.rx.clone(),
                prepared.tx.clone(),
            )
        };
        let now = core.now();
        let t0 = match at {
            None => now,
            Some(t) => {
                let root_tick = match core.clocks.convert(t, core.root) {
                    Ok(ezsdr_kernel::time::Converted::Exact { point }) => point.ticks_in(core.root),
                    Ok(ezsdr_kernel::time::Converted::Inexact { floor, .. }) => floor.ticks_in(core.root).map(|k| k + 1),
                    Err(e) => Err(e),
                };
                root_tick.map_err(|e| rejected(format!("UR-15: {e}")))?
            }
        };
        let s = core.start_up.load(Ordering::Acquire);
        if t0 < s {
            let payload = serde_json::to_value(LateCommandPayload { key: None, requested: core.at(t0), applied: core.at(s) })
                .expect("a payload");
            core.emit(&core.id, kinds::LATE_COMMAND, Severity::Warning, payload);
            return Err(rejected(format!(
                "UR-15: the start at {t0} precedes the device's start-up at {s}; the profile needs ezsdr.time.start_lead_ns ≥ {}",
                core.description.timing.startup_latency_ns
            )));
        }
        // RM-25, UR-15: each receive stream's first segment starts at T0 when its streamer
        // is open — it has channels and its port has a link —, its clock registered by
        // uhd-rx at its first block.
        let rx_first: Vec<Option<(SampleClockHandle, Channels, i64)>> = rx_streams
            .iter()
            .map(|stream| stream.handle.as_ref().filter(|_| !core.links(stream.k).is_empty()).map(|handle| (handle.clone(), stream.channels, t0)))
            .collect();
        let mut start = control::Start { t0, rx: Vec::new(), tx: Vec::new() };
        for stream in &rx_streams {
            start.rx.push(rx_first[stream.k].as_ref().map(|(handle, channels, _)| (*channels, handle.root_ticks_per_tick().num() as i64)));
        }
        for stream in &tx_streams {
            start.tx.push(stream.clock.map(|clock| (clock, stream.channels)));
        }
        core.timing(json!({ "what": "start", "t0": t0, "lead_ns": core.ns(t0 - now) }));
        let (to_tx, from_control_tx): (Vec<_>, Vec<_>) = (0..tx_streams.len()).map(|_| mpsc::channel()).unzip();
        let (to_rx, from_control_rx): (Vec<_>, Vec<_>) = (0..rx_streams.len()).map(|_| mpsc::channel()).unzip();
        let mut from_control_tx: Vec<Option<_>> = from_control_tx.into_iter().map(Some).collect();
        let mut from_control_rx: Vec<Option<_>> = from_control_rx.into_iter().map(Some).collect();
        self.running = Some(Running { to_tx: to_tx.clone(), to_rx: to_rx.clone(), threads: Vec::new() });
        // The streams' timelines first, which uhd-rx and uhd-tx read from their first turn.
        let control = Control::new(core.clone(), actions, to_tx, config, self.clock_source != "internal", start);
        let mut failed = None;
        for stream in &rx_streams {
            // UR-29, UR-15: a loss an earlier stream's owner has published stops the start
            // here — that stream is not begun, and no device call follows the loss.
            if core.is_lost() {
                failed = Some(format!("UR-15: the device was lost before stream {} was started", stream.k));
                break;
            }
            if rx_first[stream.k].is_some() {
                // UR-15: a later stream's failed start leaves the owners already begun to
                // cleanup, which joins them before closing the device's streams.
                if let Err(error) = self.device.rx_start(stream.k, t0) {
                    // UR-29: a lost device is published before those owners are stopped, so
                    // that no thread calls the device after it.
                    if error.lost {
                        core.device_lost(&error.message);
                    }
                    failed = Some(format!("UR-15: {error}"));
                    break;
                }
            }
            let first = rx_first[stream.k].clone();
            let rx = Rx::new(core.clone(), stream.k, from_control_rx[stream.k].take().expect("one receiver per stream"), first, (stream.k == 0).then_some(self.rx_stall).flatten());
            let name = rx_name(stream.k);
            if let Err(error) = self.spawn(&core, name, move || rx.run()) {
                failed = Some(error.message);
                break;
            }
            // A test holds this point to let this stream's owner publish a loss before the
            // next stream's start reads it (UR-29, UR-15).
            core.probe("start_stream", None);
        }
        if failed.is_none() {
            for stream in &tx_streams {
                let tx = Tx::new(core.clone(), stream.k, from_control_tx[stream.k].take().expect("one receiver per stream"), stream.clock, stream.channels);
                let name = tx_name(stream.k);
                if let Err(error) = self.spawn(&core, name, move || tx.run()) {
                    failed = Some(error.message);
                    break;
                }
            }
        }
        if failed.is_none() {
            if let Err(error) = self.spawn(&core, "uhd-control".to_owned(), move || control.run()) {
                failed = Some(error.message);
            }
        }
        if let Some(message) = failed {
            // UR-15: the start is refused, and every owner already begun is stopped before
            // `cleanup` closes the streamers (UR-16).
            let _ = self.stop(StopMode::Abort);
            return Err(rejected(message));
        }
        Ok(())
    }

    fn stop(&mut self, mode: StopMode) -> Result<(), ModuleError> {
        if std::mem::replace(&mut self.stopped, true) {
            return Ok(());
        }
        let Some(core) = self.prepared.as_ref().map(|p| p.core.clone()) else {
            return Ok(());
        };
        let Some((to_tx, to_rx)) = self.running.as_ref().map(|r| (r.to_tx.clone(), r.to_rx.clone())) else {
            return Ok(());
        };
        // RM-16, VH-2: the streams end at the stop instant, booked at once, which uhd-rx and
        // uhd-tx read at their next turn; then uhd-control cancels the held timed commands and
        // records the plans.
        let at = {
            let mut streams = lock(&core.streams);
            let at = core.now();
            streams.stop = Some((at, mode));
            streams.end(&core, at, mode == StopMode::Abort, false);
            at
        };
        self.join(&core, "uhd-control");
        // Transmit before receive (RM-16): every transmit stream's thread first, in its
        // stream's order.
        for to_tx in &to_tx {
            let _ = to_tx.send(TxCmd::Shutdown);
        }
        for k in 0..to_tx.len() {
            self.join(&core, &tx_name(k));
        }
        for to_rx in &to_rx {
            let _ = to_rx.send(RxCmd::Shutdown(mode));
        }
        for k in 0..to_rx.len() {
            self.join(&core, &rx_name(k));
        }
        core.timing(json!({ "what": "stop", "mode": format!("{mode:?}"), "at": at, "done": core.now() }));
        Ok(())
    }

    fn cleanup(&mut self) {
        if std::mem::replace(&mut self.cleaned, true) {
            return;
        }
        if self.running.is_some() && !self.stopped {
            let _ = self.stop(StopMode::Abort);
        }
        let Some(prepared) = self.prepared.take() else { return };
        let core = prepared.core;
        if !self.detached.is_empty() {
            // UR-16: a detached thread may still be inside a call on its streamer.
            core.reject_note(json!({ "leaked": "streamers", "because": self.detached.iter().collect::<Vec<_>>() }));
        } else if !core.is_lost() {
            self.device.close_streams();
        }
        self.write_sections(&core);
        self.running = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{FakeConfig, FakeDevice};
    use crate::ClockMode;
    use std::sync::Mutex;
    use super::core::stream_key;

    struct NoActions;
    impl ActionReceiver for NoActions {
        fn recv(&self) -> Option<ezsdr_kernel::module_api::Dispatched> { None }
    }

    /// A receive link that takes nothing, so that the receive side has a stream.
    struct NoLink;
    impl ezsdr_kernel::stream::DataLink for NoLink {
        fn publish(&self, _block: ezsdr_kernel::stream::BlockRef) -> ezsdr_kernel::stream::PublishOutcome { ezsdr_kernel::stream::PublishOutcome::Accepted }
        fn receive(&self) -> Option<(ezsdr_kernel::stream::BlockRef, ezsdr_kernel::stream::DropCarry)> { None }
        fn drops(&self) -> u64 { 0 }
        fn take_drop_carry(&self) -> ezsdr_kernel::stream::DropCarry { ezsdr_kernel::stream::DropCarry::default() }
        fn policy(&self) -> BackPressure { BackPressure::DropOldest }
    }

    #[test]
    fn ur_02_a_subdev_spec_markup_names_each_element_s_pair() {
        // UR-05, H2 (item 2's T0): the markup is `<slot>:<front end>` per element in the
        // list's own order, one ASCII space between them.
        assert_eq!(subdev_markup(&[(x3xx::Slot::A, 0), (x3xx::Slot::B, 0), (x3xx::Slot::B, 1)]), "A:0 B:0 B:1");
        assert_eq!(subdev_markup(&[(x3xx::Slot::B, 0), (x3xx::Slot::A, 0)]), "B:0 A:0");
        assert_eq!(subdev_markup(&[]), "");
    }

    /// A Provider bound to a fake device, whose `prepare` context is replaced by a `Prepared`
    /// of `rx` and `tx` streams with no channels, for `start`'s guard alone.
    fn prepared_with(rx: usize, tx: usize) -> UhdRadio {
        let binding: Binding = serde_json::from_value(json!({
            "module": { "id": "ezsdr.radio.uhd", "version": { "major": 0, "minor": 4, "patch": 0 } },
            "profile": { "name": "x3xx", "version": { "major": 0, "minor": 1, "patch": 0 } },
            "selector": { "args": "type=fake-rig", "rx": ["0:A:ubx:0"], "tx": ["0:A:ubx:0"] }
        })).unwrap();
        let device: Arc<dyn Device> = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() }));
        let mut radio = UhdRadio::from_binding(&binding, device).unwrap();
        let (core, _, _, _) = test_support::rig();
        // The declared streams of the layout, for `start`'s threads alone (RM-29).
        *lock(&core.streams) = super::core::Streams::with_streams(rx, tx);
        let stream = PreparedStream { k: 0, handle: None, channels: Channels::first(0), clock: None };
        radio.prepared = Some(Prepared {
            core,
            actions: Arc::new(NoActions),
            config: BTreeMap::new(),
            rx: (0..rx).map(|k| PreparedStream { k, ..stream.clone() }).collect(),
            tx: (0..tx).map(|k| PreparedStream { k, ..stream.clone() }).collect(),
        });
        radio
    }

    #[test]
    fn ur_13_arm_radiates_nothing() {
        // UR-13 (MA-13): `arm` sends only T3's two-zero flush and consumes its BurstAck; it
        // does not send any sample-bearing experiment burst.
        let binding: Binding = serde_json::from_value(json!({
            "module": { "id": "ezsdr.radio.uhd", "version": { "major": 0, "minor": 4, "patch": 0 } },
            "profile": { "name": "x3xx", "version": { "major": 0, "minor": 1, "patch": 0 } },
            "selector": { "args": "type=fake-rig", "rx": ["0:A:ubx:0"], "tx": ["0:A:ubx:0"] }
        })).unwrap();
        let (core, device, _time, _events) = test_support::rig();
        let mut radio = UhdRadio::from_binding(&binding, device.clone()).unwrap();
        let handle = core.clocks.declare_sample_clock(core.stream_node(Dir::Tx, 0), core.root, ezsdr_kernel::time::Rational::new(200, 1).unwrap()).unwrap();
        radio.prepared = Some(Prepared {
            core: core.clone(),
            actions: Arc::new(NoActions),
            config: BTreeMap::new(),
            rx: Vec::new(),
            tx: vec![PreparedStream { k: 0, handle: Some(handle), channels: Channels::first(1), clock: None }],
        });
        radio.arm().expect("arm");
        let calls = device.calls();
        let sends: Vec<_> = calls.iter().filter(|call| call.starts_with("tx_send")).collect();
        assert_eq!(sends, [&"tx_send 0 n=2 at=- sob=true eob=true"], "arm sends only its untimed T3 flush: {calls:?}");
        assert!(calls.iter().position(|call| call.starts_with("tx_open")).unwrap()
            < calls.iter().position(|call| call.starts_with("tx_send")).unwrap());
        assert_eq!(lock(&core.rec).device_async.iter().filter(|row| row["code"] == "BurstAck" && row["tx_flush"] == true).count(), 1,
            "the opener consumes the flush acknowledgement");
        assert_eq!(calls.iter().filter(|call| call.starts_with("tx_open")).count(), 1, "the transmit streamer's open: {calls:?}");
    }

    #[test]
    fn ur_22_arm_waits_for_t3_flush_ack_before_timed_tx_dsp() {
        let spy = Arc::new(crate::provider::test_support::LockSpy::new(FakeConfig {
            clock: ClockMode::Manual, ..FakeConfig::default()
        }));
        let (core, _, _) = test_support::rig_on(spy.clone(), Vec::new(), 2_000);
        spy.watch(&core);
        let binding: Binding = serde_json::from_value(json!({
            "module": { "id": "ezsdr.radio.uhd", "version": { "major": 0, "minor": 4, "patch": 0 } },
            "profile": { "name": "x3xx", "version": { "major": 0, "minor": 1, "patch": 0 } },
            "selector": { "args": "type=fake-rig", "rx": ["0:A:ubx:0"], "tx": ["0:A:ubx:0"] }
        })).unwrap();
        let mut radio = UhdRadio::from_binding(&binding, spy.clone()).unwrap();
        let handle = core.clocks.declare_sample_clock(core.stream_node(Dir::Tx, 0), core.root,
            ezsdr_kernel::time::Rational::new(200, 1).unwrap()).unwrap();
        radio.prepared = Some(Prepared {
            core: core.clone(), actions: Arc::new(NoActions), config: BTreeMap::new(), rx: Vec::new(),
            tx: vec![PreparedStream { k: 0, handle: Some(handle), channels: Channels::first(1), clock: None }],
        });
        super::core::assert_tx_open_flush_barrier(core, spy, move || { radio.arm().unwrap(); });
    }

    #[test]
    fn measurement_timing_candidate_updates_every_surface_and_the_start_boundary() {
        use ezsdr_kernel::spec::{CapabilityValue, Scalar};

        let binding: Binding = serde_json::from_value(json!({
            "module": { "id": "ezsdr.radio.uhd", "version": { "major": 0, "minor": 4, "patch": 0 } },
            "profile": { "name": "x3xx", "version": { "major": 0, "minor": 1, "patch": 0 } },
            "selector": { "args": "type=fake-rig", "rx": ["0:A:ubx:0"], "tx": ["0:A:ubx:0"] }
        })).unwrap();
        let candidate = ezsdr_radio::TimingEnvelope {
            min_timed_command_lead_ns: 11_000_001,
            startup_latency_ns: 12_000_002,
            command_queue_depth: 7,
            release_window_ns: 13_000_003,
            overflow_restart_gap_ns: 17_000_004,
            rx_start_lead_ns: 19_000_005,
            tx_start_lead_ns: 23_000_006,
            tx_in_flight_ns: 29_000_007,
        };
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() }));
        {
            let ordinary = UhdRadio::from_binding(&binding, device.clone()).unwrap();
            assert_eq!(ordinary.description.timing, x3xx::timing(&profile::terms()));
        }
        let mut radio = UhdRadio::from_binding(&binding, device.clone()).unwrap().with_measurement_timing(candidate);
        assert_eq!(radio.description.timing, candidate);
        assert_eq!(radio.instance.min_command_lead, Some(Duration::new(ClockDomainId::HOST_MONOTONIC, candidate.min_timed_command_lead_ns)));
        let fields = [
            ("min_timed_command_lead_ns", candidate.min_timed_command_lead_ns),
            ("startup_latency_ns", candidate.startup_latency_ns),
            ("command_queue_depth", candidate.command_queue_depth),
            ("release_window_ns", candidate.release_window_ns),
            ("overflow_restart_gap_ns", candidate.overflow_restart_gap_ns),
            ("rx_start_lead_ns", candidate.rx_start_lead_ns),
            ("tx_start_lead_ns", candidate.tx_start_lead_ns),
            ("tx_in_flight_ns", candidate.tx_in_flight_ns),
        ];
        let envelope = &radio.instance.sections[&section("envelope")];
        for (field, value) in fields {
            let key = Key::parse(&format!("radio.timing.{field}")).unwrap();
            assert_eq!(radio.instance.tree.capabilities[&key], CapabilityValue::One { value: Scalar::from(value) }, "{field}");
            assert_eq!(envelope["timing"][field], value, "{field}");
        }

        let (core, _time, _events) = test_support::rig_on_description(device, Vec::new(), BTreeMap::new(), radio.description.clone());
        *lock(&core.streams) = core::Streams::with_streams(0, 0);
        radio.prepared = Some(Prepared { core: core.clone(), actions: Arc::new(NoActions), config: BTreeMap::new(), rx: Vec::new(), tx: Vec::new() });
        let armed_at = core.now();
        radio.arm().unwrap();
        let boundary = armed_at + core.ticks(candidate.startup_latency_ns);
        assert_eq!(core.start_up.load(Ordering::Acquire), boundary);
        let error = radio.start(Some(core.at(boundary - 1))).unwrap_err();
        assert!(error.message.contains(&format!("precedes the device's start-up at {boundary}")), "{}", error.message);
        radio.start(Some(core.at(boundary))).unwrap();
        assert!(lock(&core.rec).timing.iter().any(|row| row["what"] == "start" && row["t0"] == boundary));
        radio.stop(StopMode::Orderly).unwrap();
        radio.cleanup();
    }

    #[test]
    fn ur_25_a_refused_check_at_arm_leaves_the_set_unknown() {
        // UR-25 (S20): `arm` opens the receive streamer and the direction's rate comparison
        // refuses; the record holds no set and no configuration, so the owner opens the
        // streamer again for the next segment, where the direction's stored refusal is
        // re-enforced (UR-13, UR-36).
        let binding: Binding = serde_json::from_value(json!({
            "module": { "id": "ezsdr.radio.uhd", "version": { "major": 0, "minor": 4, "patch": 0 } },
            "profile": { "name": "x3xx", "version": { "major": 0, "minor": 1, "patch": 0 } },
            "selector": { "args": "type=fake-rig", "rx": ["0:A:ubx:0"], "tx": ["0:A:ubx:0"] }
        })).unwrap();
        let device: Arc<dyn Device> = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() }));
        let mut radio = UhdRadio::from_binding(&binding, device).unwrap();
        let (core, _device, _time, _events) = test_support::rig_with_links(vec![Arc::new(NoLink)]);
        // The direction's first comparison, refused: `arm`'s own comparison re-enforces it.
        assert!(core.check_rates(Dir::Rx, 99).is_err());
        radio.prepared = Some(Prepared {
            core: core.clone(),
            actions: Arc::new(NoActions),
            config: BTreeMap::new(),
            rx: vec![PreparedStream { k: 0, handle: None, channels: Channels::first(1), clock: None }],
            tx: vec![PreparedStream { k: 0, handle: None, channels: Channels::first(0), clock: None }],
        });
        let error = radio.arm().unwrap_err();
        assert!(error.message.starts_with("UR-13:"), "{}", error.message);
        let streams = lock(&core.streams);
        let rx = streams.stream(stream_key(Dir::Rx, 0));
        assert!(matches!(rx.set, core::DeviceSet::Unknown), "the refused comparison records no set: {:?}", rx.set);
        assert_eq!(rx.configured, None, "and no configuration");
    }

    #[test]
    fn ur_36_a_successful_open_moves_the_generation_on_even_when_its_check_refuses() {
        // UR-36: every successful open is a new streamer, so the generation moves on; the rate
        // comparison that follows is refused by the direction's stored outcome, and then the set
        // and the configuration are unknown.
        let (core, _device, _time, _events) = test_support::rig_with_links(vec![Arc::new(NoLink)]);
        assert!(core.check_rates(Dir::Tx, 99).is_err());
        let before = lock(&core.streams).stream(stream_key(Dir::Tx, 0)).generation;
        assert!(core.open_streamer(Dir::Tx, 0, Channels::first(1)).is_err());
        let streams = lock(&core.streams);
        let tx = streams.stream(stream_key(Dir::Tx, 0));
        assert_eq!(tx.generation, before + 1, "the open succeeded: a new streamer");
        assert!(matches!(tx.set, core::DeviceSet::Unknown), "the refused comparison keeps no set: {:?}", tx.set);
        assert_eq!(tx.configured, None);
    }

    #[test]
    fn ur_36_a_failed_open_moves_no_generation() {
        // UR-36: a failed open is no new streamer, so the generation stands; the set is unknown.
        let config = FakeConfig { faults: vec![crate::device::FakeFault::RxOpenFails], ..FakeConfig::default() };
        let (core, _device, _time, _events) = test_support::rig_with(config, vec![Arc::new(NoLink)]);
        let before = lock(&core.streams).stream(stream_key(Dir::Rx, 0)).generation;
        assert!(core.open_streamer(Dir::Rx, 0, Channels::first(1)).is_err());
        let streams = lock(&core.streams);
        let rx = streams.stream(stream_key(Dir::Rx, 0));
        assert_eq!(rx.generation, before, "the open failed: no new streamer");
        assert!(matches!(rx.set, core::DeviceSet::Unknown), "{:?}", rx.set);
    }

    fn thread_names(radio: &UhdRadio) -> Vec<String> {
        radio.running.as_ref().unwrap().threads.iter().map(|(name, _)| name.clone()).collect()
    }

    #[test]
    fn ur_15_every_declared_stream_starts_its_own_thread() {
        // UR-15, RM-29 (U1): a layout declares a stream per channel and starts one owner per
        // stream, named for it, whether or not the stream holds a channel; `stop` joins all
        // five.
        let mut radio = prepared_with(2, 2);
        radio.start(None).unwrap();
        assert_eq!(thread_names(&radio), ["uhd-rx-s0", "uhd-rx-s1", "uhd-tx-s0", "uhd-tx-s1", "uhd-control"]);
        radio.stop(StopMode::Orderly).unwrap();
        assert!(radio.detached.is_empty(), "{:?}", radio.detached);
        assert!(radio.running.as_ref().unwrap().threads.is_empty(), "every thread was joined");
    }

    /// A layout with two receive front ends and one transmit one (P1b).
    fn two_rx_description() -> channels::Description {
        use ezsdr_kernel::spec::{Ident, Scalar, Value};
        let selector = BTreeMap::from([
            (Ident::parse("rx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:B:ubx:0")])),
            (Ident::parse("tx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0")])),
        ]);
        let layout = ezsdr_radio::channels::from_selector(&selector).unwrap();
        ezsdr_radio::channels::Description::new(&layout, 2_000, &crate::profile::terms())
    }

    /// A `FakeDevice` whose first receive call of stream 0 finds the device lost (P1b): that
    /// stream's owner publishes the loss while `start` goes on to stream 1.
    struct LostInReceive {
        inner: Arc<FakeDevice>,
        marked: Mutex<Option<Sender<()>>>,
    }

    impl Device for LostInReceive {
        fn describe(&self) -> Json { self.inner.describe() }
        fn fidelity(&self) -> ezsdr_kernel::module_api::Fidelity { self.inner.fidelity() }
        fn master_clock_rate(&self) -> u64 { self.inner.master_clock_rate() }
        fn channels(&self, dir: Dir) -> usize { self.inner.channels(dir) }
        fn front_end(&self, dir: Dir, chan: usize) -> Result<String, crate::DeviceError> { self.inner.front_end(dir, chan) }
        fn serial(&self) -> Result<String, crate::DeviceError> { self.inner.serial() }
        fn set_subdev_spec(&self, dir: Dir, markup: &str) -> Result<(), crate::DeviceError> { self.inner.set_subdev_spec(dir, markup) }
        fn rates(&self, dir: Dir, chan: usize) -> Result<Vec<(f64, f64, f64)>, crate::DeviceError> { self.inner.rates(dir, chan) }
        fn set_sources(&self, clock: &str, time: &str) -> Result<(), crate::DeviceError> { self.inner.set_sources(clock, time) }
        fn set_time_zero(&self, at_next_pps: bool) -> Result<(), crate::DeviceError> { self.inner.set_time_zero(at_next_pps) }
        fn time_now(&self) -> Result<i64, crate::DeviceError> { self.inner.time_now() }
        fn ref_locked(&self) -> Result<Option<bool>, crate::DeviceError> { self.inner.ref_locked() }
        fn apply(&self, dir: Dir, chan: usize, settings: &Settings, at: Option<i64>) -> Result<crate::device::Applied, crate::DeviceError> { self.inner.apply(dir, chan, settings, at) }
        fn rx_open(&self, stream: usize, channels: &[usize]) -> Result<(), crate::DeviceError> { self.inner.rx_open(stream, channels) }
        fn rx_start(&self, stream: usize, at: i64) -> Result<(), crate::DeviceError> { self.inner.rx_start(stream, at) }
        fn rx_packet_samples(&self, stream: usize) -> usize { self.inner.rx_packet_samples(stream) }
        fn rx_stop(&self, stream: usize, at: Option<i64>) -> Result<(), crate::DeviceError> { self.inner.rx_stop(stream, at) }
        fn rx_recv(&self, stream: usize, n: usize, timeout: Wall) -> crate::device::RxRecv {
            if stream == 0 {
                return crate::device::RxRecv::Failed(crate::DeviceError { lost: true, message: "test: lost in stream 0's receive".to_owned() });
            }
            self.inner.rx_recv(stream, n, timeout)
        }
        fn tx_open(&self, stream: usize, channels: &[usize]) -> Result<(), crate::DeviceError> { self.inner.tx_open(stream, channels) }
        fn tx_send(&self, stream: usize, samples: &[&[crate::Iq]], at: Option<i64>, sob: bool, eob: bool, timeout: Wall) -> Result<usize, crate::DeviceError> {
            self.inner.tx_send(stream, samples, at, sob, eob, timeout)
        }
        fn tx_async(&self, stream: usize, timeout: Wall) -> Result<Option<crate::device::TxReport>, crate::DeviceError> { self.inner.tx_async(stream, timeout) }
        fn rx_close(&self, stream: usize) { self.inner.rx_close(stream) }
        fn tx_close(&self, stream: usize) { self.inner.tx_close(stream) }
        fn close_streams(&self) { self.inner.close_streams() }
        fn mark_lost(&self) {
            self.inner.mark_lost();
            if let Some(marked) = self.marked.lock().unwrap().take() {
                let _ = marked.send(());
            }
        }
        fn host(&self) -> crate::host::HostClock { self.inner.host() }
        fn raise_thread_priority(&self) -> Result<(), crate::DeviceError> { self.inner.raise_thread_priority() }
    }

    #[test]
    fn ur_15_a_loss_published_by_an_owner_stops_the_next_stream_s_start() {
        // UR-29, UR-15 (P1b): stream 0's owner finds the device lost while `start` goes on;
        // the published loss stops stream 1 before any device call of its own.
        let binding: Binding = serde_json::from_value(json!({
            "module": { "id": "ezsdr.radio.uhd", "version": { "major": 0, "minor": 4, "patch": 0 } },
            "profile": { "name": "x3xx", "version": { "major": 0, "minor": 1, "patch": 0 } },
            "selector": { "args": "type=fake-rig",  "rx": ["0:A:ubx:0", "0:B:ubx:0"], "tx": ["0:A:ubx:0"] }
        })).unwrap();
        let inner = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() }));
        let (mark, marked) = mpsc::channel();
        let device: Arc<dyn Device> = Arc::new(LostInReceive { inner: inner.clone(), marked: Mutex::new(Some(mark)) });
        let mut radio = UhdRadio::from_binding(&binding, device.clone()).unwrap();
        device.rx_open(0, &[0]).unwrap();
        device.rx_open(1, &[1]).unwrap();
        let links: Vec<Vec<Arc<dyn ezsdr_kernel::stream::DataLink>>> = (0..2).map(|_| vec![Arc::new(NoLink) as Arc<dyn ezsdr_kernel::stream::DataLink>]).collect();
        let (core, _time, _events) = test_support::rig_on_description_links(device, links, BTreeMap::new(), two_rx_description());
        let handle = |k: usize| core.clocks.declare_sample_clock(core.stream_node(Dir::Rx, k), core.root, ezsdr_kernel::time::Rational::new(200, 1).unwrap()).unwrap();
        radio.prepared = Some(Prepared {
            core: core.clone(),
            actions: Arc::new(NoActions),
            config: BTreeMap::new(),
            rx: (0..2).map(|k| PreparedStream { k, handle: Some(handle(k)), channels: Channels::first(1), clock: None }).collect(),
            tx: Vec::new(),
        });
        let flagged = Mutex::new(Some(marked));
        test_support::probe(&core, "start_stream", move |_| {
            let Some(marked) = flagged.lock().unwrap().take() else { return };
            marked.recv_timeout(Wall::from_secs(10)).expect("stream 0's owner published the loss");
        });
        let error = radio.start(None).unwrap_err();
        assert_eq!(error.kind, ModuleErrorKind::Rejected, "{error:?}");
        assert!(error.message.starts_with("UR-15:"), "{}", error.message);
        assert!(!inner.calls().iter().any(|call| call.starts_with("rx_start 1")), "stream 1 was not started: {:?}", inner.calls());
        radio.cleanup();
        let calls = inner.calls();
        assert_eq!(calls.iter().filter(|call| *call == "mark_lost").count(), 1, "{calls:?}");
        assert!(!calls.iter().any(|call| call == "close_streams"), "a lost device's streamers are not closed: {calls:?}");
    }

    #[test]
    fn ur_26_the_control_thread_is_joined_before_every_stream() {
        // UR-26 (C1): `Provider::stop` joins uhd-control first, then the transmit streams in
        // order, then the receive streams. Five threads that never finish leave the order in
        // `rejected`.
        let mut radio = prepared_with(2, 2);
        let core = radio.prepared.as_ref().unwrap().core.clone();
        let mut senders = Vec::new();
        let mut threads = Vec::new();
        for name in ["uhd-control", "uhd-tx-s0", "uhd-tx-s1", "uhd-rx-s0", "uhd-rx-s1"] {
            let (release, wait) = mpsc::channel::<()>();
            senders.push(release);
            let handle = core.device.host().spawn(name, move || { let _ = wait.recv(); }).unwrap();
            threads.push((name.to_owned(), handle));
        }
        let (to_tx, _tx) = mpsc::channel();
        let (to_rx, _rx) = mpsc::channel();
        radio.running = Some(Running { to_tx: vec![to_tx.clone(), to_tx], to_rx: vec![to_rx.clone(), to_rx], threads });
        radio.stop(StopMode::Orderly).unwrap();
        radio.cleanup();
        drop(senders);
        let rejected = &radio.instance.sections[&section("rejected")];
        let order: Vec<String> = rejected.as_array().unwrap().iter()
            .filter(|row| row["reason"].as_str().is_some_and(|reason| reason.contains("did not join")))
            .map(|row| row["thread"].as_str().unwrap().to_owned()).collect();
        assert_eq!(order, ["uhd-control", "uhd-tx-s0", "uhd-tx-s1", "uhd-rx-s0", "uhd-rx-s1"], "{rejected}");
    }
}
