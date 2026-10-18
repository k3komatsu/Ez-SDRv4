//! `UhdRadio`: the Radio Provider role of `ezsdr.radio.uhd` (UR-5, UR-10…UR-16,
//! UR-26, UR-30). It is not stepped; three threads of its own do the work.

mod control;
mod core;
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
use ezsdr_radio::device::DeviceDescription;
use ezsdr_radio::payloads::LateCommandPayload;
use ezsdr_radio::x3xx;
use ezsdr_radio::{keys, kinds};
use serde_json::{Value as Json, json};

use self::core::{Clock, Core, lock};
use self::control::Control;
use self::rx::{Rx, RxCmd};
use self::tx::{Tx, TxCmd};
use crate::device::{Device, Dir};
use crate::profile::{self, MASTER_CLOCK_HZ};

fn rejected(message: impl Into<String>) -> ModuleError {
    ModuleError::rejected(message)
}

/// `ezsdr.radio.uhd.<suffix>`; the Kernel files it under the instance's source root (RS-39).
fn section(suffix: &str) -> Namespace {
    Namespace::parse(&format!("ezsdr.radio.uhd.{suffix}")).expect("a section name")
}

const JOIN_BOUND: Wall = Wall::from_secs(1);

/// A stream the layout has, as `prepare` declared it (RM-29, UR-12): its number, the
/// SampleClock handle when it has channels, its channel count, and the transmit clock `arm`
/// registered at its origin (UR-13).
#[derive(Clone)]
struct PreparedStream {
    k: usize,
    handle: Option<SampleClockHandle>,
    channels: usize,
    clock: Option<Clock>,
}

struct Prepared {
    core: Arc<Core>,
    actions: Arc<dyn ActionReceiver>,
    config: BTreeMap<Key, Value>,
    /// One entry per receive stream of the layout (RM-29): the current keys have one, `k = 0`.
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
    threads: Vec<(&'static str, JoinHandle<()>)>,
}

/// The UHD Radio Provider (spec 18).
pub struct UhdRadio {
    instance: ProviderInstance,
    device: Arc<dyn Device>,
    clock_source: String,
    /// The epoch of the root its own Authority registers (UR-6).
    epoch: ezsdr_kernel::time::EpochRef,
    description: DeviceDescription,
    rx_stall: Option<(Wall, Wall)>,
    /// The first open's error when the device was opened again on it (UR-7's reopen).
    reopened: Option<String>,
    prepare_called: bool,
    prepared: Option<Prepared>,
    running: Option<Running>,
    stopped: bool,
    cleaned: bool,
    detached: BTreeSet<&'static str>,
}

impl UhdRadio {
    /// Builds the Provider from its binding and the device the runtime opened for it
    /// (UR-5, UR-6, UR-10).
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
        let selection = x3xx::from_selector(&binding.selector).map_err(|why| rejected(format!("UR-5: {why}")))?;
        let mcr = device.master_clock_rate();
        if mcr != MASTER_CLOCK_HZ {
            return Err(rejected(format!(
                "UR-5: profile {} needs a 200 MHz master clock; the device runs at {mcr} Hz",
                x3xx::PROFILE
            )));
        }
        let description = profile::description(&selection, block_len);
        // The layout's front ends must be the device's: UHD clips a tune outside its front
        // end's range without a warning, past the RF envelope's check. Only the layout's
        // channels: slot B empty is a channel on UHD's unknown board.
        for (dir, name, elements) in [(Dir::Rx, "rx", &selection.rx), (Dir::Tx, "tx", &selection.tx)] {
            let need = elements.len();
            let have = device.channels(dir);
            if have < need {
                return Err(rejected(format!(
                    "UR-5: profile {} needs {need} {name} channels; the device has {have}",
                    x3xx::PROFILE
                )));
            }
            for (chan, element) in elements.iter().enumerate() {
                let front_end = device.front_end(dir, chan).map_err(|e| rejected(format!("UR-5: {}", e.message)))?;
                if !front_end.starts_with(element.board.uhd_name()) {
                    return Err(rejected(format!(
                        "UR-5: profile {} needs {} front ends; {name} channel {chan} is `{front_end}`",
                        x3xx::PROFILE,
                        element.board.uhd_name()
                    )));
                }
            }
        }
        let mut describe = device.describe();
        if let Some(object) = describe.as_object_mut() {
            object.insert("args".to_owned(), json!(args));
        }
        let mut sections = BTreeMap::new();
        sections.insert(section("device"), describe);
        sections.insert(section("envelope"), serde_json::to_value(description.envelope()).expect("an envelope"));
        let timing = description.timing;
        let instance = ProviderInstance {
            id: resource.clone(),
            module: crate::module_ref(),
            profile: binding.profile.clone(),
            tree: description.tree(&resource, HOST_MEMORY),
            fidelity: device.fidelity(),
            driving: Driving { stepped: false },
            arm_after: Vec::new(),
            min_command_lead: Some(Duration::new(ClockDomainId::HOST_MONOTONIC, timing.min_timed_command_lead_ns)),
            sections,
        };
        Ok(UhdRadio {
            instance,
            device,
            clock_source,
            epoch,
            description,
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
    /// block: a real overflow for bench step B5 (UR-34). Only Rust code reaches it (GZ-9).
    pub fn with_rx_stall(mut self, after: Wall, duration: Wall) -> UhdRadio {
        self.rx_stall = Some((after, duration));
        self
    }

    fn prepared(&self) -> Result<&Prepared, ModuleError> {
        self.prepared.as_ref().ok_or_else(|| rejected("UR-12: prepare must come first"))
    }

    fn spawn(&mut self, core: &Arc<Core>, name: &'static str, body: impl FnOnce() + Send + 'static) -> Result<(), ModuleError> {
        let core_ = core.clone();
        // §4.2, M1: the host clock registers the participant before the thread runs.
        let host = core.device.host();
        let handle = host
            .spawn(name, move || {
                if catch_unwind(AssertUnwindSafe(body)).is_err() {
                    // UR-16: a panic is recorded; the stream the thread served is gone.
                    core_.reject_note(json!({ "thread": name, "reason": "panicked" }));
                    if name != "uhd-control" {
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

    fn join(&mut self, core: &Core, name: &'static str) {
        let Some(running) = self.running.as_mut() else { return };
        let Some(position) = running.threads.iter().position(|(n, _)| *n == name) else { return };
        let (_, handle) = running.threads.remove(position);
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
    }
}

impl Provider for UhdRadio {
    fn instance(&self) -> &ProviderInstance {
        &self.instance
    }

    /// RM-26's coercion over the profile: pure, no device call (UR-11, MA-11).
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
        let mut config = self.description.defaults.clone();
        for (key, value) in report.applied {
            if keys::CONFIGURATION.contains(&key.as_str()) {
                config.insert(key, value);
            }
        }
        for dir in [Dir::Rx, Dir::Tx] {
            let n = Core::channels(&config, dir);
            if n > self.device.channels(dir) {
                return Err(rejected(format!("UR-12: {n} {} channels, and the device has {}", dir.name(), self.device.channels(dir))));
            }
        }
        let mut links = Vec::new();
        for attached in &ctx.links {
            match &attached.endpoint {
                Endpoint::StreamOut(link) if attached.component == f.id && attached.port.as_str() == "rx" => {
                    if link.policy() == BackPressure::Block {
                        return Err(rejected("UR-12: a device cannot wait for a Block link; it overflows instead"));
                    }
                    links.push(link.clone());
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
        // One entry per stream of the layout (RM-29): the current keys have one in each
        // direction, `k = 0`, and a stream with no channel declares no SampleClock (UR-12).
        let mut rx = PreparedStream { k: 0, handle: None, channels: 0, clock: None };
        let mut tx = PreparedStream { k: 0, handle: None, channels: 0, clock: None };
        for (stream, dir) in [(&mut rx, Dir::Rx), (&mut tx, Dir::Tx)] {
            let channels = Core::channels(&config, dir);
            stream.channels = channels;
            if channels == 0 {
                continue;
            }
            let n = core
                .configure(dir, channels, &Core::settings(&config, dir))
                .map_err(|e| rejected(e.message))?;
            let node = self.instance.id.child(dir.name()).map_err(|e| rejected(format!("UR-12: {e}")))?;
            let ratio = ezsdr_kernel::time::Rational::new(n as u64, 1).map_err(|e| rejected(format!("UR-12: {e}")))?;
            let handle = ctx.clocks.declare_sample_clock(node, root, ratio).map_err(|e| rejected(format!("UR-12: {e}")))?;
            stream.handle = Some(handle);
        }
        self.prepared = Some(Prepared { core, actions: ctx.actions, config: config.clone(), rx: vec![rx], tx: vec![tx] });
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
        // UR-13: the receive streamer of every stream with channels, when a link is attached;
        // a receive side with no link has no stream.
        for stream in &mut prepared.rx {
            if stream.channels == 0 || core.links.is_empty() {
                continue;
            }
            self.device.rx_open(stream.k, &(0..stream.channels).collect::<Vec<usize>>()).map_err(|e| rejected(format!("UR-13: {e}")))?;
        }
        for stream in &mut prepared.tx {
            if stream.channels == 0 {
                continue;
            }
            self.device.tx_open(stream.k, &(0..stream.channels).collect::<Vec<usize>>()).map_err(|e| rejected(format!("UR-13: {e}")))?;
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
        // The current keys give each direction one stream, so one command channel and one
        // thread each; 1b-4c gives every stream its own (design 33 D15). A column of another
        // length is a layout this stage does not start, refused rather than half started.
        let [rx_stream] = <[PreparedStream; 1]>::try_from(rx_streams).map_err(|_| rejected("UR-15: this stage starts one receive stream"))?;
        let [tx_stream] = <[PreparedStream; 1]>::try_from(tx_streams).map_err(|_| rejected("UR-15: this stage starts one transmit stream"))?;
        // RM-25, UR-15: the receive stream's first segment starts at T0 when its streamer is
        // open — it has channels and a link —, its clock registered by uhd-rx at its first
        // block.
        let rx_first = rx_stream.handle.as_ref().filter(|_| !core.links.is_empty()).map(|handle| (handle.clone(), rx_stream.channels, t0));
        if rx_first.is_some() {
            self.device.rx_start(rx_stream.k, t0).map_err(|e| rejected(format!("UR-15: {e}")))?;
        }
        let start = control::Start {
            t0,
            rx: rx_first.as_ref().map(|(handle, channels, _)| (*channels, handle.root_ticks_per_tick().num() as i64)),
            tx: tx_stream.clock.map(|clock| (clock, tx_stream.channels)),
        };
        core.timing(json!({ "what": "start", "t0": t0, "lead_ns": core.ns(t0 - now) }));
        let (to_tx, from_control_tx) = mpsc::channel();
        let (to_rx, from_control_rx) = mpsc::channel();
        self.running = Some(Running { to_tx: vec![to_tx.clone()], to_rx: vec![to_rx], threads: Vec::new() });
        // The streams' timelines first, which uhd-rx and uhd-tx read from their first turn.
        let control = Control::new(core.clone(), actions, to_tx, config, self.clock_source != "internal", start);
        // uhd-rx and uhd-tx start whether or not their direction has a stream (UR-15).
        let rx = Rx::new(core.clone(), rx_stream.k, from_control_rx, rx_first, self.rx_stall);
        self.spawn(&core, "uhd-rx", move || rx.run())?;
        let tx = Tx::new(core.clone(), tx_stream.k, from_control_tx, tx_stream.clock, tx_stream.channels);
        self.spawn(&core, "uhd-tx", move || tx.run())?;
        self.spawn(&core, "uhd-control", move || control.run())?;
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
        // Transmit before receive (RM-16): every transmit stream's thread first.
        for to_tx in &to_tx {
            let _ = to_tx.send(TxCmd::Shutdown);
        }
        self.join(&core, "uhd-tx");
        for to_rx in &to_rx {
            let _ = to_rx.send(RxCmd::Shutdown(mode));
        }
        self.join(&core, "uhd-rx");
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

    struct NoActions;
    impl ActionReceiver for NoActions {
        fn recv(&self) -> Option<ezsdr_kernel::module_api::Dispatched> { None }
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
        let stream = PreparedStream { k: 0, handle: None, channels: 0, clock: None };
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
    fn ur_15_a_second_stream_is_refused_rather_than_half_started() {
        // UR-15 (design 33 D15): the current keys give each direction one stream. A column of
        // another length is refused before anything starts, not started with its rest
        // silently dropped.
        for (rx, tx, says) in [(2, 1, "one receive stream"), (1, 2, "one transmit stream"), (0, 1, "one receive stream"), (1, 0, "one transmit stream")] {
            let mut radio = prepared_with(rx, tx);
            let error = radio.start(None).unwrap_err();
            let case = format!("{rx} receive, {tx} transmit");
            assert_eq!(error.kind, ModuleErrorKind::Rejected, "{case}");
            assert!(error.message.contains(says), "{case}: {}", error.message);
            assert!(radio.running.is_none(), "{case}: threads were started");
        }
    }
}
