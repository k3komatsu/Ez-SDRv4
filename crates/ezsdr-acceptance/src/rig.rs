//! Real Module assembly and artifact helpers for the acceptance tests.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use ezsdr_kernel::binding::{AdmissionCheckRegistry, BindingProfile};
use ezsdr_kernel::contract::ContractRegistry;
use ezsdr_kernel::coordinator::Assembly;
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::manifest::{ArtifactRef, Manifest};
use ezsdr_kernel::module_api::{Factories, ModuleRef, ModuleRegistry, Provider, Sink, Version};
use ezsdr_kernel::policy::EventKindRegistry;
use ezsdr_kernel::spec::Ident;
use ezsdr_kernel::time::ClockRegistry;
use serde_json::{json, Value as JsonValue};

/// A test-specific temporary capture directory that is removed on drop.
pub struct TempDir(pub PathBuf);

impl TempDir {
    /// Creates a unique capture directory under the host temporary directory.
    pub fn new(test: &str) -> TempDir {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let safe: String = test.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' }).collect();
        let path = std::env::temp_dir().join(format!(
            "ezsdr-acc-{}-{safe}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(&path).expect("create capture directory");
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Builds the portable BindingProfile used by Spec Runs.
pub fn spec_profile(profile: &str, selector: JsonValue, dir: &Path, environment: JsonValue) -> JsonValue {
    profile_document(profile, selector, dir, environment, false)
}

/// Builds the BindingProfile used by Sessions, with a feeder link on the recorder.
pub fn session_profile(profile: &str, selector: JsonValue, dir: &Path, environment: JsonValue) -> JsonValue {
    profile_document(profile, selector, dir, environment, true)
}

/// The `ezsdr.time` section every acceptance profile carries, under the caller's own
/// sections. A 2 s start lead is MR-11's floor for an x310-like profile.
fn env_section(extra: JsonValue) -> serde_json::Map<String, JsonValue> {
    let mut env = serde_json::Map::new();
    env.insert("ezsdr.time".to_owned(), json!({ "class": "simulation", "start_lead_ns": 2_000_000_000u64 }));
    if let Some(extra) = extra.as_object() {
        env.extend(extra.clone());
    }
    env
}

/// The capture Sink's binding, fed from `component`'s receive port when a Session
/// profile is one and left unfed for a Spec Run, which declares its feed in `outputs[]`.
fn recorder(dir: &Path, component: Option<&str>) -> JsonValue {
    let mut recorder = json!({
        "module": { "id": "ezsdr.sink.capture", "version": { "major": 1, "minor": 1, "patch": 0 } },
        "selector": { "dir": dir.to_string_lossy() }
    });
    if let Some(component) = component {
        recorder["feed"] = json!({
            "port": { "component": component, "port": "rx" },
            "policy": "drop_oldest",
            "capacity": 64
        });
    }
    recorder
}

fn profile_document(profile: &str, selector: JsonValue, dir: &Path, environment: JsonValue, session: bool) -> JsonValue {
    json!({
        "version": 1,
        "bindings": {
            "radio": {
                "module": { "id": "ezsdr.radio.mock", "version": { "major": 1, "minor": 2, "patch": 0 } },
                "selector": selector,
                "profile": { "name": profile, "version": { "major": 1, "minor": 1, "patch": 0 } }
            },
            "rec": recorder(dir, session.then_some("radio")),
            "sim": {
                "module": { "id": "ezsdr.sim-engine", "version": { "major": 1, "minor": 0, "patch": 0 } },
                "selector": {}
            }
        },
        "authority": "sim",
        "placements": {
            "links": [{
                "link": link_module(),
                "from": { "component": "radio", "port": "rx" },
                "to": { "component": "rec", "port": "in" }
            }]
        },
        "environment": env_section(environment)
    })
}

/// Builds the BindingProfile of [`crate::experiments::link`]: one simulated radio per
/// resource (device ids `dev_tx` and `dev_rx`, whatever the resources are called), the
/// recorder on `rx`, and `environment` (with the rig's time section).
pub fn link_profile(profile: &str, tx: &str, rx: &str, rx_jitter: bool, dir: &Path, environment: JsonValue) -> JsonValue {
    link_document(profile, tx, rx, rx_jitter, dir, environment, false)
}

/// Builds the Session BindingProfile of two simulated radios `tx` and `rx`, with the
/// recorder fed from `rx`'s receive port.
pub fn link_session_profile(profile: &str, tx: &str, rx: &str, dir: &Path, environment: JsonValue) -> JsonValue {
    link_document(profile, tx, rx, false, dir, environment, true)
}

fn link_document(profile: &str, tx: &str, rx: &str, rx_jitter: bool, dir: &Path, environment: JsonValue, session: bool) -> JsonValue {
    let radio = |id: String, jitter: bool| json!({
        "module": { "id": "ezsdr.radio.mock", "version": { "major": 1, "minor": 2, "patch": 0 } },
        "selector": { "id": id, "block_len_jitter": jitter },
        "profile": { "name": profile, "version": { "major": 1, "minor": 1, "patch": 0 } }
    });
    let mut bindings = serde_json::Map::new();
    bindings.insert(tx.to_owned(), radio("dev_tx".to_owned(), false));
    bindings.insert(rx.to_owned(), radio("dev_rx".to_owned(), rx_jitter));
    bindings.insert("rec".to_owned(), recorder(dir, session.then_some(rx)));
    bindings.insert("sim".to_owned(), json!({
        "module": { "id": "ezsdr.sim-engine", "version": { "major": 1, "minor": 0, "patch": 0 } },
        "selector": {}
    }));
    json!({
        "version": 1,
        "bindings": bindings,
        "authority": "sim",
        "placements": {
            "links": [{ "link": link_module(), "from": { "component": rx, "port": "rx" }, "to": { "component": "rec", "port": "in" } }]
        },
        "environment": env_section(environment)
    })
}

/// Builds the four real Modules and Vocabulary registries named by a BindingProfile.
pub fn assemble(profile_doc: &JsonValue, inputs: BTreeMap<ContentHash, Vec<u8>>) -> Assembly {
    let profile = BindingProfile::from_json(profile_doc).expect("valid acceptance BindingProfile");
    let mut registry = ModuleRegistry::new();
    let mut checks = AdmissionCheckRegistry::new();
    let mut kinds = EventKindRegistry::with_kernel_kinds();
    ezsdr_radio::register(&mut registry, &mut checks, &mut kinds).expect("register radio Vocabulary");
    ezsdr_sim::register(&mut registry, &mut checks, &mut kinds).expect("register sim Vocabulary");
    ezsdr_sink::register(&mut registry, &mut checks, &mut kinds).expect("register sink Vocabulary");
    registry.register(ezsdr_sim_engine::descriptor(), Factories { authority: true, ..Factories::default() }).expect("register simulation Engine");
    registry.register(ezsdr_mock_radio::descriptor(), Factories { provider: true, ..Factories::default() }).expect("register Provider");
    registry.register(ezsdr_link_host::descriptor(), Factories { link: true, ..Factories::default() }).expect("register host Link");
    registry.register_link_descriptor(ezsdr_link_host::link_descriptor()).expect("register host Link descriptor");
    registry.register(ezsdr_sink_capture::descriptor(), Factories { sink: true, ..Factories::default() }).expect("register capture Sink");

    let clocks = Arc::new(ClockRegistry::new());
    let authority_binding = profile.bindings.get(&profile.authority).expect("authority binding");
    let authority = Box::new(ezsdr_sim_engine::SimEngine::from_binding(authority_binding, clocks.clone()).expect("build simulation Engine"));
    let mut providers: BTreeMap<Ident, Box<dyn Provider>> = BTreeMap::new();
    let mut sinks: BTreeMap<Ident, Box<dyn Sink>> = BTreeMap::new();
    let medium = ezsdr_sim::channel::Medium::new();
    for (name, binding) in &profile.bindings {
        match binding.module.id.as_str() {
            "ezsdr.radio.mock" => {
                let radio = ezsdr_mock_radio::MockRadio::from_binding(binding).expect("build Provider").with_medium(medium.clone());
                providers.insert(name.clone(), Box::new(radio));
            }
            "ezsdr.sink.capture" => {
                sinks.insert(name.clone(), Box::new(ezsdr_sink_capture::CaptureSink::from_binding(binding).expect("build capture Sink")));
            }
            _ => {}
        }
    }
    let link_modules: BTreeSet<ModuleRef> = profile.placements.links.iter().map(|placement| placement.link.clone()).collect();
    let links = link_modules.into_iter().map(|module| (module, Box::new(ezsdr_link_host::HostLinkModule::new()) as Box<dyn ezsdr_kernel::module_api::Link>)).collect();
    Assembly {
        registry,
        checks,
        kinds,
        contracts: ContractRegistry::with_standard_contracts(),
        clocks,
        host_clock: Arc::new(ezsdr_kernel::run::SystemHostClock::new()),
        providers,
        executors: BTreeMap::new(),
        sinks,
        authority,
        links,
        inputs,
    }
}

/// Removes run-unique fields from a Manifest for deterministic comparisons.
pub fn determinism_projection(manifest: &Manifest) -> JsonValue {
    fn strip_utc(value: &mut JsonValue) {
        match value {
            JsonValue::Object(object) => {
                object.remove("host_utc_nanos");
                for value in object.values_mut() { strip_utc(value); }
            }
            JsonValue::Array(values) => values.iter_mut().for_each(strip_utc),
            _ => {}
        }
    }
    let mut value = serde_json::to_value(manifest).expect("Manifest serializes");
    if let Some(object) = value.as_object_mut() { object.remove("hash"); }
    if let Some(run) = value.get_mut("run").and_then(JsonValue::as_object_mut) { run.remove("id"); }
    if let Some(artifacts) = value.get_mut("artifacts").and_then(JsonValue::as_array_mut) {
        for artifact in artifacts {
            if let Some(object) = artifact.as_object_mut() { object.remove("uri"); }
        }
    }
    strip_utc(&mut value);
    value
}

/// Reads interleaved little-endian cf32 samples from a capture artifact.
pub fn read_capture(artifact: &ArtifactRef, channels: usize) -> Vec<Vec<(f32, f32)>> {
    assert!(channels > 0, "a capture needs at least one channel");
    let path = artifact.uri.strip_prefix("file://").expect("capture URI is a file URI");
    let bytes = fs::read(path).expect("read capture file");
    let frame_bytes = channels.checked_mul(8).expect("capture frame size fits");
    assert_eq!(bytes.len() % frame_bytes, 0, "capture ends on a complete frame");
    let mut output = vec![Vec::with_capacity(bytes.len() / frame_bytes); channels];
    for frame in bytes.chunks_exact(frame_bytes) {
        for (channel, samples) in output.iter_mut().enumerate() {
            let offset = channel * 8;
            samples.push((
                f32::from_le_bytes(frame[offset..offset + 4].try_into().expect("real sample")),
                f32::from_le_bytes(frame[offset + 4..offset + 8].try_into().expect("imaginary sample")),
            ));
        }
    }
    output
}

fn link_module() -> ModuleRef {
    ModuleRef {
        id: ezsdr_link_host::descriptor().id,
        version: Version::new(1, 0, 0),
    }
}
