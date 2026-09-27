//! `UhdRadio`: the Radio Provider role of `ezsdr.radio.uhd` (UR-5, UR-10…UR-16,
//! UR-26, UR-30). It is not stepped; three threads of its own do the work.

mod control;
mod core;
mod rx;
mod tx;

use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration as Wall, Instant};

use ezsdr_kernel::binding::Binding;
use ezsdr_kernel::event::Severity;
use ezsdr_kernel::id::{ClockDomainId, ResourceId};
use ezsdr_kernel::module_api::{
    ActionReceiver, CoerceReport, Driving, Endpoint, ExecutionClass, ModuleError,
    ModuleErrorKind, PrepareContext, Provider, ProviderInstance, Requested, Role, StopMode,
};
use ezsdr_kernel::plan::{Fragment, PrepareReport};
use ezsdr_kernel::spec::{Ident, Key, Namespace, Value};
use ezsdr_kernel::stream::BackPressure;
use ezsdr_kernel::time::{Duration, EpochRef, SampleClockHandle, TimePoint};
use ezsdr_radio::device::DeviceDescription;
use ezsdr_radio::payloads::LateCommandPayload;
use ezsdr_radio::{keys, kinds};
use serde_json::{Value as Json, json};

use self::core::{Clock, Core, lattice, lock};
use self::control::Control;
use self::rx::{Rx, RxCmd};
use self::tx::{Tx, TxCmd};
use crate::device::{Device, Dir};
use crate::profile::{self, MASTER_CLOCK_HZ};

fn rejected(message: impl Into<String>) -> ModuleError {
    ModuleError::rejected(message)
}

fn section(id: &Ident, suffix: &str) -> Namespace {
    Namespace::parse(&format!("ezsdr.radio.uhd.{id}.{suffix}")).expect("a section name")
}

const JOIN_BOUND: Wall = Wall::from_secs(1);

struct Prepared {
    core: Arc<Core>,
    actions: Arc<dyn ActionReceiver>,
    config: BTreeMap<Key, Value>,
    rx: Option<(SampleClockHandle, usize)>,
    tx: Option<(SampleClockHandle, usize)>,
    rx_clock: Option<Clock>,
    tx_clock: Option<Clock>,
}

struct Running {
    stop: Arc<AtomicBool>,
    to_tx: Sender<TxCmd>,
    to_rx: Sender<RxCmd>,
    threads: Vec<(&'static str, JoinHandle<()>)>,
}

/// The UHD Radio Provider (spec 18).
pub struct UhdRadio {
    instance: ProviderInstance,
    section_id: Ident,
    device: Arc<dyn Device>,
    args: String,
    clock_source: String,
    description: DeviceDescription,
    rx_stall: Option<(Wall, Wall)>,
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
        if binding.profile.as_ref() != Some(&profile::profile_ref()) {
            return Err(rejected("UR-5: the profile must be x310-ubx 0.1.0"));
        }
        if binding.feed.is_some() {
            return Err(rejected("UR-5: a Provider binding carries no feed"));
        }
        let allowed = ["args", "id", "clock_source", "time_source", "block_len"];
        if let Some(name) = binding.selector.keys().find(|k| !allowed.contains(&k.as_str())) {
            return Err(rejected(format!("UR-5: unsupported selector key `{name}`")));
        }
        let get = |name: &str| binding.selector.get(&Ident::parse(name).expect("a selector key"));
        let string = |name: &str| match get(name) {
            None => Ok(None),
            Some(Value::Str(s)) => Ok(Some(s.clone())),
            Some(_) => Err(rejected(format!("UR-5: selector `{name}` must be a string"))),
        };
        let args = string("args")?.ok_or_else(|| rejected("UR-5: the selector needs `args`"))?;
        let id = string("id")?.unwrap_or_else(|| "usrp".to_owned());
        let section_id = Ident::parse(&id).map_err(|_| rejected("UR-5: selector `id` must match ^[a-z][a-z0-9_]*$"))?;
        let resource = ResourceId::parse(&id).map_err(|_| rejected("UR-5: selector `id` must be one resource segment"))?;
        let source = |name: &str| -> Result<String, ModuleError> {
            let value = string(name)?.unwrap_or_else(|| "internal".to_owned());
            if ["internal", "external", "gpsdo"].contains(&value.as_str()) {
                Ok(value)
            } else {
                Err(rejected(format!("UR-5: selector `{name}` must be internal, external or gpsdo")))
            }
        };
        let clock_source = source("clock_source")?;
        source("time_source")?;
        let block_len = match get("block_len") {
            None => profile::DEFAULT_BLOCK_LEN,
            Some(Value::Int(n)) if (1..=65_536).contains(n) => *n as u32,
            Some(_) => return Err(rejected("UR-5: selector `block_len` must be an Int in 1..=65536")),
        };
        let mcr = device.master_clock_rate();
        if mcr != MASTER_CLOCK_HZ {
            return Err(rejected(format!(
                "UR-5: profile x310-ubx needs a 200 MHz master clock; the device runs at {mcr} Hz"
            )));
        }
        let description = profile::description(block_len);
        let mut describe = device.describe();
        if let Some(object) = describe.as_object_mut() {
            object.insert("args".to_owned(), json!(args));
        }
        let mut sections = BTreeMap::new();
        sections.insert(section(&section_id, "device"), describe);
        sections.insert(section(&section_id, "envelope"), serde_json::to_value(description.envelope()).expect("an envelope"));
        let timing = description.timing;
        let instance = ProviderInstance {
            id: resource.clone(),
            module: crate::module_ref(),
            profile: binding.profile.clone(),
            tree: description.tree(&resource),
            fidelity: device.fidelity(),
            driving: Driving { stepped: false },
            arm_after: Vec::new(),
            min_command_lead: Some(Duration::new(ClockDomainId::HOST_MONOTONIC, timing.min_timed_command_lead_ns)),
            sections,
        };
        Ok(UhdRadio {
            instance,
            section_id,
            device,
            args,
            clock_source,
            description,
            rx_stall: None,
            prepare_called: false,
            prepared: None,
            running: None,
            stopped: false,
            cleaned: false,
            detached: BTreeSet::new(),
        })
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
        let handle = std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
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
        let id = self.section_id.clone();
        let sections = &mut self.instance.sections;
        let mut put = |suffix: &str, value: Json| {
            sections.insert(section(&id, suffix), value);
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
            Ok(ezsdr_kernel::time::ClockDomainKind::Root { epoch: EpochRef::Arbitrary { set_by }, .. }) => Some(set_by),
            _ => None,
        };
        let own = [
            format!("ezsdr.radio.uhd.set_time_now:{}", self.args),
            format!("ezsdr.radio.uhd.set_time_unknown_pps:{}", self.args),
        ];
        if !epoch.is_some_and(|e| own.contains(&e)) {
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
        let core = Arc::new(Core::new(
            self.device.clone(),
            self.instance.id.clone(),
            root,
            ctx.time.clone(),
            ctx.clocks.clone(),
            ctx.events.clone(),
            ctx.inputs.clone(),
            links,
            self.description.clone(),
        ));
        let mut handles = [None, None];
        for (slot, dir) in [Dir::Rx, Dir::Tx].into_iter().enumerate() {
            let channels = Core::channels(&config, dir);
            if channels == 0 {
                continue;
            }
            let n = core
                .configure(dir, channels, &Core::settings(&config, dir))
                .map_err(|e| rejected(e.message))?;
            let stream = self.instance.id.child(dir.name()).map_err(|e| rejected(format!("UR-12: {e}")))?;
            let ratio = ezsdr_kernel::time::Rational::new(n as u64, 1).map_err(|e| rejected(format!("UR-12: {e}")))?;
            let handle = ctx.clocks.declare_sample_clock(stream, root, ratio).map_err(|e| rejected(format!("UR-12: {e}")))?;
            handles[slot] = Some((handle, channels));
        }
        let [rx, tx] = handles;
        self.prepared = Some(Prepared { core, actions: ctx.actions, config: config.clone(), rx, tx, rx_clock: None, tx_clock: None });
        Ok(PrepareReport { fragment: f.id.clone(), effective: config, coercions: report.coercions, warnings: Vec::new() })
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
        if let Some((_, channels)) = &prepared.rx {
            if !core.links.is_empty() {
                self.device.rx_open(*channels).map_err(|e| rejected(format!("UR-13: {e}")))?;
            }
        }
        if let Some((handle, channels)) = &prepared.tx {
            self.device.tx_open(*channels).map_err(|e| rejected(format!("UR-13: {e}")))?;
            // RM-25: the first lattice instant at or after the arm instant.
            let n = handle.root_ticks_per_tick.num() as i64;
            let origin = lattice(a, n);
            let domain = core.clocks.register_sample_clock(handle, origin).map_err(|e| rejected(format!("UR-13: {e}")))?;
            let clock = Clock { domain, origin, n };
            prepared.tx_clock = Some(clock);
            let mut streams = lock(&core.streams);
            streams.tx = Some(clock);
            streams.tx_channels = *channels;
        }
        core.timing(json!({ "what": "arm", "at": a, "start_up_until": s }));
        Ok(())
    }

    fn start(&mut self, at: Option<TimePoint>) -> Result<(), ModuleError> {
        let (core, actions, config, rx, tx_clock, tx_channels) = {
            let prepared = self.prepared()?;
            (
                prepared.core.clone(),
                prepared.actions.clone(),
                prepared.config.clone(),
                prepared.rx.clone(),
                prepared.tx_clock,
                prepared.tx.as_ref().map_or(0, |(_, n)| *n),
            )
        };
        let now = core.now();
        let t0 = match at {
            None => now,
            Some(t) if t.domain == core.root => t.ticks,
            Some(t) => match core.clocks.convert(t, core.root).map_err(|e| rejected(format!("UR-15: {e}")))? {
                ezsdr_kernel::time::Converted::Exact { point } => point.ticks,
                ezsdr_kernel::time::Converted::Inexact { floor, .. } => floor.ticks + 1,
            },
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
        let mut rx_clock = None;
        let mut rx_channels = 0;
        if let Some((handle, channels)) = rx.filter(|_| !core.links.is_empty()) {
            let n = handle.root_ticks_per_tick.num() as i64;
            let domain = core.clocks.register_sample_clock(&handle, t0).map_err(|e| rejected(format!("UR-15: {e}")))?;
            let clock = Clock { domain, origin: t0, n };
            self.device.rx_start(t0).map_err(|e| rejected(format!("UR-15: {e}")))?;
            lock(&core.streams).rx = Some(clock);
            rx_clock = Some(clock);
            rx_channels = channels;
        }
        if let Some(prepared) = self.prepared.as_mut() {
            prepared.rx_clock = rx_clock;
        }
        core.timing(json!({ "what": "start", "t0": t0, "lead_ns": core.ns(t0 - now) }));
        let (to_tx, from_control_tx) = mpsc::channel();
        let (to_rx, from_control_rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        self.running = Some(Running { stop: stop.clone(), to_tx: to_tx.clone(), to_rx: to_rx.clone(), threads: Vec::new() });
        // uhd-rx and uhd-tx start whether or not their direction has a stream (UR-15).
        let rx = Rx::new(core.clone(), from_control_rx, rx_clock, rx_channels, self.rx_stall);
        self.spawn(&core, "uhd-rx", move || rx.run())?;
        let tx = Tx::new(core.clone(), from_control_tx, tx_clock, tx_channels);
        self.spawn(&core, "uhd-tx", move || tx.run())?;
        let control = Control::new(core.clone(), actions, to_tx, to_rx, config, self.clock_source != "internal");
        self.spawn(&core, "uhd-control", move || control.run(stop))?;
        Ok(())
    }

    fn stop(&mut self, mode: StopMode) -> Result<(), ModuleError> {
        if std::mem::replace(&mut self.stopped, true) {
            return Ok(());
        }
        let Some(core) = self.prepared.as_ref().map(|p| p.core.clone()) else {
            return Ok(());
        };
        let Some((stop, to_tx, to_rx)) = self.running.as_ref().map(|r| (r.stop.clone(), r.to_tx.clone(), r.to_rx.clone())) else {
            return Ok(());
        };
        let at = core.now();
        // uhd-control first: it cancels the held timed commands as it ends.
        stop.store(true, Ordering::Release);
        self.join(&core, "uhd-control");
        // Transmit before receive (RM-16).
        let _ = to_tx.send(TxCmd::Shutdown);
        self.join(&core, "uhd-tx");
        let _ = to_rx.send(RxCmd::Shutdown(mode));
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
