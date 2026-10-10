//! The Modules this server compiles in, and a Run's Assembly built from them (EA-7).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use ezsdr_exec_native::{Implementation, NativeExecutor};
use ezsdr_kernel::binding::{AdmissionCheckRegistry, BindingProfile};
use ezsdr_kernel::coordinator::Assembly;
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::module_api::{Authority, Executor, Factories, Link, ModuleRef, ModuleRegistry, Provider, Sink};
use ezsdr_kernel::policy::EventKindRegistry;
use ezsdr_kernel::spec::{Ident, Scalar, Value};
use ezsdr_kernel::time::ClockRegistry;

const RADIO: &str = "ezsdr.radio.mock";
const SINK: &str = "ezsdr.sink.capture";
const EXECUTOR: &str = "ezsdr.exec.native";
const AUTHORITY: &str = "ezsdr.sim-engine";
const LINK: &str = "ezsdr.link.host";
const UHD: &str = "ezsdr.radio.uhd";

/// How the server opens the USRP a binding's `args` names (EA-7, VE-5). A test
/// embedding passes one returning a `FakeDevice`; no document can (GZ-9).
pub type OpenDevice = Arc<dyn Fn(&str) -> Result<Arc<dyn ezsdr_radio_uhd::Device>, String> + Send + Sync>;

/// The binary's device: UHD's, when the server is built with the feature `uhd` (VE-5).
fn open_uhd(args: &str) -> Result<Arc<dyn ezsdr_radio_uhd::Device>, String> {
    if cfg!(feature = "uhd") {
        ezsdr_radio_uhd::open(args)
    } else {
        Err("EA-7: ezsdr.radio.uhd: this server was built without UHD; rebuild ezsdr-server with --features uhd".to_owned())
    }
}

/// The registries with every Vocabulary and Module of the catalogue (EA-7).
fn registries() -> Result<(ModuleRegistry, AdmissionCheckRegistry, EventKindRegistry), String> {
    let mut registry = ModuleRegistry::new();
    let mut checks = AdmissionCheckRegistry::new();
    let mut kinds = EventKindRegistry::with_kernel_kinds();
    let fail = |what: &str, error: &dyn std::fmt::Display| format!("EA-7: cannot register {what}: {error}");
    ezsdr_radio::register(&mut registry, &mut checks, &mut kinds).map_err(|e| fail("radio", &e))?;
    ezsdr_sim::register(&mut registry, &mut checks, &mut kinds).map_err(|e| fail("sim", &e))?;
    ezsdr_sink::register(&mut registry, &mut checks, &mut kinds).map_err(|e| fail("sink", &e))?;
    let roles = [
        (ezsdr_sim_engine::descriptor(), Factories { authority: true, ..Factories::default() }),
        (ezsdr_mock_radio::descriptor(), Factories { provider: true, ..Factories::default() }),
        (ezsdr_link_host::descriptor(), Factories { link: true, ..Factories::default() }),
        (ezsdr_sink_capture::descriptor(), Factories { sink: true, ..Factories::default() }),
        (ezsdr_exec_native::descriptor(), Factories { executor: true, ..Factories::default() }),
        (ezsdr_radio_uhd::descriptor(), ezsdr_radio_uhd::factories()),
    ];
    for (descriptor, factories) in roles {
        let id = descriptor.id.clone();
        registry.register(descriptor, factories).map_err(|e| fail(id.as_str(), &e))?;
    }
    registry
        .register_link_descriptor(ezsdr_link_host::link_descriptor())
        .map_err(|e| fail(LINK, &e))?;
    Ok((registry, checks, kinds))
}

/// Builds the Assembly of one Run from its BindingProfile document: every binding
/// must name a Module of the catalogue, the native Executor gets `implementations`, and
/// a binding of `ezsdr.radio.uhd` its device from `open_device` (UHD's when `None`)
/// (EA-7). The refusal is a message naming the binding.
pub fn assemble(
    profile_doc: &serde_json::Value,
    inputs: BTreeMap<ContentHash, Vec<u8>>,
    implementations: Vec<Implementation>,
    open_device: Option<&OpenDevice>,
) -> Result<Assembly, String> {
    let profile = BindingProfile::from_json(profile_doc).map_err(|error| error.to_string())?;
    let (registry, checks, kinds) = registries()?;
    let clocks = Arc::new(ClockRegistry::new());
    let authority_binding = profile
        .bindings
        .get(&profile.authority)
        .ok_or_else(|| format!("EA-7: the authority {} is not bound", profile.authority))?;
    if ![AUTHORITY, UHD].contains(&authority_binding.module.id.as_str()) {
        return Err(format!("EA-7: the authority must be {AUTHORITY} or {UHD}, not {}", authority_binding.module.id));
    }
    if profile.bindings.values().filter(|binding| binding.module.id.as_str() == UHD).count() > 1 {
        return Err("EA-7: one USRP per Run in this server (Phase 7)".to_owned());
    }
    let mut authority: Option<Box<dyn Authority>> = None;
    let medium = ezsdr_sim::channel::Medium::new();
    let mut providers: BTreeMap<Ident, Box<dyn Provider>> = BTreeMap::new();
    let mut sinks: BTreeMap<Ident, Box<dyn Sink>> = BTreeMap::new();
    let mut executors: BTreeMap<Ident, Box<dyn Executor>> = BTreeMap::new();
    for (name, binding) in &profile.bindings {
        let built = |error: &dyn std::fmt::Display| format!("EA-7: {name}: {error}");
        match binding.module.id.as_str() {
            RADIO => {
                let radio = ezsdr_mock_radio::MockRadio::from_binding(binding).map_err(|e| built(&e))?;
                providers.insert(name.clone(), Box::new(radio.with_medium(medium.clone())));
            }
            SINK => {
                let sink = ezsdr_sink_capture::CaptureSink::from_binding(binding).map_err(|e| built(&e))?;
                sinks.insert(name.clone(), Box::new(sink));
            }
            EXECUTOR => {
                let executor = NativeExecutor::new(implementations.clone()).map_err(|e| built(&e))?;
                executors.insert(name.clone(), Box::new(executor));
            }
            AUTHORITY if *name == profile.authority => {
                let engine = ezsdr_sim_engine::SimEngine::from_binding(binding, clocks.clone()).map_err(|e| built(&e))?;
                authority = Some(Box::new(engine));
            }
            UHD => {
                let selector = |key: &str| match binding.selector.get(&Ident::parse(key).expect("a selector key")) {
                    Some(Value::Scalar(Scalar::Str(value))) => value.clone(),
                    _ => "internal".to_owned(),
                };
                let Some(Value::Scalar(Scalar::Str(args))) = binding.selector.get(&Ident::parse("args").expect("a selector key")) else {
                    return Err(format!("EA-7: {name}: UR-5: the selector needs `args`"));
                };
                // UR-7's reopen (design-notes §18): an Authority refused because the X300's
                // reference PLL did not lock is built again once on a new open of the device,
                // unless the binding's `reopen_on_unlock` is false; a second refusal stands.
                let reopen = !matches!(binding.selector.get(&Ident::parse("reopen_on_unlock").expect("a selector key")), Some(Value::Scalar(Scalar::Bool(false))));
                let mut first_error: Option<String> = None;
                loop {
                    let device = match open_device {
                        Some(open) => open(args),
                        None => open_uhd(args),
                    }
                    .map_err(|e| if e.starts_with("EA-7") { e } else { built(&e) })?;
                    let mut radio = ezsdr_radio_uhd::UhdRadio::from_binding(binding, device.clone()).map_err(|e| built(&e))?;
                    if let Some(error) = &first_error {
                        radio = radio.with_reopened(error.clone());
                    }
                    if *name != profile.authority {
                        providers.insert(name.clone(), Box::new(radio));
                        break;
                    }
                    match ezsdr_radio_uhd::DeviceAuthority::new(device, clocks.clone(), &selector("clock_source"), &selector("time_source"), args) {
                        Ok(time) => {
                            providers.insert(name.clone(), Box::new(radio));
                            authority = Some(Box::new(time));
                            break;
                        }
                        // The radio and the device go here, before the next open.
                        Err(error) if reopen && first_error.is_none() && error.starts_with(ezsdr_radio_uhd::REFERENCE_DID_NOT_LOCK) => {
                            first_error = Some(error);
                        }
                        Err(error) => return Err(built(&error)),
                    }
                }
            }
            other => return Err(format!("EA-7: {name}: no Module {other} in this server")),
        }
    }
    let link_modules: BTreeSet<ModuleRef> = profile.placements.links.iter().map(|placement| placement.link.clone()).collect();
    let mut links: BTreeMap<ModuleRef, Box<dyn Link>> = BTreeMap::new();
    for module in link_modules {
        if module.id.as_str() != LINK {
            return Err(format!("EA-7: no Link Module {} in this server", module.id));
        }
        links.insert(module, Box::new(ezsdr_link_host::HostLinkModule::new()));
    }
    let mut assembly = Assembly::new(authority.expect("the authority's binding was built above"), clocks);
    assembly.registry = registry;
    assembly.checks = checks;
    assembly.kinds = kinds;
    assembly.providers = providers;
    assembly.executors = executors;
    assembly.sinks = sinks;
    assembly.links = links;
    assembly.inputs = inputs;
    Ok(assembly)
}

/// The Assembly a device-paced Session hands `run_child`: no Provider, Sink, Executor
/// or Link, and no device opened; the Kernel refuses the child before reading it
/// (EA-14, KC-37a, KG-12). Its Authority is a Simulation Engine only because an
/// Assembly must hold one.
pub fn empty_assembly() -> Assembly {
    let clocks = Arc::new(ClockRegistry::new());
    Assembly::new(Box::new(ezsdr_sim_engine::SimEngine::new(clocks.clone()).expect("a fresh registry takes a root")), clocks)
}

/// The BindingProfile `connect` uses when the client names none (EA-9).
pub fn default_profile(dir: &str) -> serde_json::Value {
    serde_json::json!({
        "version": 1,
        "bindings": {
            "radio": {
                "module": { "id": RADIO, "version": { "major": 2, "minor": 0, "patch": 0 } },
                "selector": { "id": "radio" },
                "profile": { "name": "x310-like", "version": { "major": 2, "minor": 0, "patch": 0 } }
            },
            "rec": {
                "module": { "id": SINK, "version": { "major": 1, "minor": 2, "patch": 0 } },
                "selector": { "dir": dir },
                "feed": { "port": { "component": "radio", "port": "rx" }, "policy": "drop_oldest", "capacity": 64 }
            },
            "sim": {
                "module": { "id": AUTHORITY, "version": { "major": 1, "minor": 0, "patch": 0 } },
                "selector": {}
            }
        },
        "authority": "sim",
        "placements": {
            "links": [{
                "link": { "id": LINK, "version": { "major": 1, "minor": 0, "patch": 0 } },
                "from": { "component": "radio", "port": "rx" },
                "to": { "component": "rec", "port": "in" }
            }]
        },
        "environment": {
            "ezsdr.time": { "class": "simulation", "start_lead_ns": 2_000_000_000u64 },
            "sim.seed": 0,
            "sim.channel": { "couplings": [{ "tx": "radio", "tx_channel": 0, "rx": "radio", "rx_channel": 0, "gain_db": 0.0 }] }
        }
    })
}
