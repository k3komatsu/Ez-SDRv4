//! The Modules this server compiles in, and a Run's Assembly built from them (EA-7).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use ezsdr_exec_native::{Implementation, NativeExecutor};
use ezsdr_kernel::binding::{AdmissionCheckRegistry, BindingProfile};
use ezsdr_kernel::contract::ContractRegistry;
use ezsdr_kernel::coordinator::Assembly;
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::module_api::{Executor, Factories, Link, ModuleRef, ModuleRegistry, Provider, Sink};
use ezsdr_kernel::policy::EventKindRegistry;
use ezsdr_kernel::spec::Ident;
use ezsdr_kernel::time::ClockRegistry;

const RADIO: &str = "ezsdr.radio.mock";
const SINK: &str = "ezsdr.sink.capture";
const EXECUTOR: &str = "ezsdr.exec.native";
const AUTHORITY: &str = "ezsdr.sim-engine";
const LINK: &str = "ezsdr.link.host";

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
/// must name a Module of the catalogue, and the native Executor gets `implementations`
/// (EA-7). The refusal is a message naming the binding.
pub fn assemble(
    profile_doc: &serde_json::Value,
    inputs: BTreeMap<ContentHash, Vec<u8>>,
    implementations: Vec<Implementation>,
) -> Result<Assembly, String> {
    let profile = BindingProfile::from_json(profile_doc).map_err(|error| error.to_string())?;
    let (registry, checks, kinds) = registries()?;
    let clocks = Arc::new(ClockRegistry::new());
    let authority_binding = profile
        .bindings
        .get(&profile.authority)
        .ok_or_else(|| format!("EA-7: the authority {} is not bound", profile.authority))?;
    if authority_binding.module.id.as_str() != AUTHORITY {
        return Err(format!("EA-7: the authority must be {AUTHORITY}, not {}", authority_binding.module.id));
    }
    let authority = ezsdr_sim_engine::SimEngine::from_binding(authority_binding, clocks.clone())
        .map_err(|error| format!("EA-7: {}: {error}", profile.authority))?;
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
            AUTHORITY if *name == profile.authority => {}
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
    Ok(Assembly {
        registry,
        checks,
        kinds,
        contracts: ContractRegistry::with_standard_contracts(),
        clocks,
        host_clock: Arc::new(ezsdr_kernel::run::SystemHostClock::new()),
        providers,
        executors,
        sinks,
        authority: Box::new(authority),
        links,
        inputs,
    })
}

/// The BindingProfile `connect` uses when the client names none (EA-9).
pub fn default_profile(dir: &str) -> serde_json::Value {
    serde_json::json!({
        "version": 1,
        "bindings": {
            "radio": {
                "module": { "id": RADIO, "version": { "major": 1, "minor": 3, "patch": 0 } },
                "selector": { "id": "radio" },
                "profile": { "name": "x310-like", "version": { "major": 1, "minor": 1, "patch": 0 } }
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
