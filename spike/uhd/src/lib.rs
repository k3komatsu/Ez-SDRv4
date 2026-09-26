//! Spike: Module `ezsdr.radio.uhd` 0.1.0, a UHD Provider plus a device-paced
//! Authority, run by the unmodified Phase 3 coordinator. Throwaway code that tests
//! the Kernel; the findings live in `plan/spikes/`, not here.

pub mod authority;
pub mod device;
mod ffi;
pub mod provider;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use ezsdr_kernel::binding::{AdmissionCheckRegistry, Binding, BindingProfile};
use ezsdr_kernel::contract::ContractRegistry;
use ezsdr_kernel::coordinator::Assembly;
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::ModuleId;
use ezsdr_kernel::module_api::{
    Deployment, Factories, KERNEL_API, Link, ModuleDescriptor, ModuleRef, ModuleRegistry, Provider, Role, Sink,
    Version, VersionReq, VocabularyRequirement,
};
use ezsdr_kernel::policy::EventKindRegistry;
use ezsdr_kernel::spec::{Ident, Namespace, Value};
use ezsdr_kernel::time::ClockRegistry;
use serde_json::{Value as Json, json};

pub use authority::DeviceAuthority;
pub use device::{Device, FakeDevice, UhdDevice};
pub use provider::UhdRadio;

/// `ezsdr.radio.uhd` 0.1.0.
pub fn module_ref() -> ModuleRef {
    ModuleRef { id: ModuleId::parse("ezsdr.radio.uhd").expect("valid"), version: Version::new(0, 1, 0) }
}

/// One Module, two roles: the Provider and the Authority share one device handle.
pub fn descriptor() -> ModuleDescriptor {
    ModuleDescriptor {
        id: module_ref().id,
        version: module_ref().version,
        kernel_api: KERNEL_API,
        roles: vec![Role::Provider, Role::Authority],
        vocabularies: vec![VocabularyRequirement {
            id: Namespace::parse("radio").expect("valid"),
            req: VersionReq(Version::new(1, 1, 0)),
        }],
        deployment: Deployment::InProcess {},
        impl_hash: Some(ContentHash::of_bytes(b"ezsdr.radio.uhd 0.1.0 spike")),
    }
}

fn selector_str(b: &Binding, k: &str) -> Option<String> {
    match b.selector.get(&Ident::parse(k).expect("key")) {
        Some(Value::Str(s)) => Some(s.clone()),
        _ => None,
    }
}

/// Opens (once per distinct `args`) the devices a profile's `ezsdr.radio.uhd`
/// bindings name. `args = "fake"` is the wall-clock fake.
pub fn open_devices(profile: &BindingProfile) -> Result<BTreeMap<String, Arc<dyn Device>>, String> {
    let mut out: BTreeMap<String, Arc<dyn Device>> = BTreeMap::new();
    for b in profile.bindings.values().filter(|b| b.module == module_ref()) {
        let args = selector_str(b, "args").unwrap_or_default();
        if out.contains_key(&args) {
            continue;
        }
        let dev: Arc<dyn Device> = if args == "fake" { Arc::new(FakeDevice::new()) } else { Arc::new(UhdDevice::open(&args)?) };
        out.insert(args, dev);
    }
    Ok(out)
}

/// The acceptance rig's `assemble`, with the Mock and the Simulation Engine swapped
/// for this Module. The registries and every other Module are the same.
pub fn assemble(
    profile_doc: &Json,
    inputs: BTreeMap<ContentHash, Vec<u8>>,
    devices: &BTreeMap<String, Arc<dyn Device>>,
) -> Result<Assembly, String> {
    let profile = BindingProfile::from_json(profile_doc).map_err(|e| e.to_string())?;
    let mut registry = ModuleRegistry::new();
    let mut checks = AdmissionCheckRegistry::new();
    let mut kinds = EventKindRegistry::with_kernel_kinds();
    let e = |x: ezsdr_kernel::module_api::ModuleError| x.to_string();
    ezsdr_radio::register(&mut registry, &mut checks, &mut kinds).map_err(e)?;
    ezsdr_sink::register(&mut registry, &mut checks, &mut kinds).map_err(e)?;
    registry.register(descriptor(), Factories { provider: true, authority: true, ..Factories::default() }).map_err(e)?;
    registry.register(ezsdr_link_host::descriptor(), Factories { link: true, ..Factories::default() }).map_err(e)?;
    registry.register_link_descriptor(ezsdr_link_host::link_descriptor()).map_err(e)?;
    registry.register(ezsdr_sink_capture::descriptor(), Factories { sink: true, ..Factories::default() }).map_err(e)?;

    let clocks = Arc::new(ClockRegistry::new());
    let ab = profile.bindings.get(&profile.authority).ok_or("no authority binding")?;
    let dev_of = |b: &Binding| devices.get(&selector_str(b, "args").unwrap_or_default()).cloned().ok_or("device not opened");
    let authority = Box::new(DeviceAuthority::new(
        dev_of(ab)?,
        clocks.clone(),
        selector_str(ab, "clock_source").as_deref(),
        selector_str(ab, "time_source").as_deref(),
    )?);
    let mut providers: BTreeMap<Ident, Box<dyn Provider>> = BTreeMap::new();
    let mut sinks: BTreeMap<Ident, Box<dyn Sink>> = BTreeMap::new();
    for (name, b) in &profile.bindings {
        match b.module.id.as_str() {
            "ezsdr.radio.uhd" => {
                providers.insert(name.clone(), Box::new(UhdRadio::from_binding(b, dev_of(b)?).map_err(e)?));
            }
            "ezsdr.sink.capture" => {
                sinks.insert(name.clone(), Box::new(ezsdr_sink_capture::CaptureSink::from_binding(b).map_err(e)?));
            }
            _ => {}
        }
    }
    let link_modules: BTreeSet<ModuleRef> = profile.placements.links.iter().map(|p| p.link.clone()).collect();
    let links = link_modules
        .into_iter()
        .map(|m| (m, Box::new(ezsdr_link_host::HostLinkModule::new()) as Box<dyn Link>))
        .collect();
    Ok(Assembly {
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
    })
}

/// Options for [`profile`]: the radio binding's selector and the environment.
pub struct ProfileOpts<'a> {
    /// UHD device args, or `fake`.
    pub args: &'a str,
    /// Extra keys for the radio binding's selector (`grid`, `min_lead_ns`, …).
    pub radio: Json,
    /// `internal` or `external` (10 MHz).
    pub clock_source: &'a str,
    /// `internal` or `external` (PPS).
    pub time_source: &'a str,
    /// `cabled` (→ hardware_in_loop) or `over_the_air` (→ hardware).
    pub rf_path: &'a str,
    /// T0 lead after arm.
    pub start_lead_ns: u64,
}

/// [`profile`] for a Session: the recorder binding carries its feed, as the acceptance
/// rig's `session_profile` does (SB-22c).
pub fn session_profile(o: &ProfileOpts, dir: &Path) -> Json {
    let mut doc = profile(o, dir);
    doc["bindings"]["rec"]["feed"] = json!({ "port": { "component": "radio", "port": "rx" }, "policy": "drop_oldest", "capacity": 64 });
    doc
}

/// A Spec-Run BindingProfile for one USRP resource `radio` and the capture Sink `rec`:
/// the acceptance rig's `spec_profile` with the Mock binding and the `sim` Authority
/// replaced. The Spec it runs is unchanged (Vision §59).
pub fn profile(o: &ProfileOpts, dir: &Path) -> Json {
    let class = if o.rf_path == "over_the_air" { "hardware" } else { "hardware_in_loop" };
    let mut radio_sel = json!({ "args": o.args, "clock_source": o.clock_source, "time_source": o.time_source });
    if let Some(extra) = o.radio.as_object() {
        radio_sel.as_object_mut().unwrap().extend(extra.clone());
    }
    json!({
        "version": 1,
        "bindings": {
            "radio": { "module": { "id": "ezsdr.radio.uhd", "version": { "major": 0, "minor": 1, "patch": 0 } }, "selector": radio_sel },
            "rec": {
                "module": { "id": "ezsdr.sink.capture", "version": { "major": 1, "minor": 0, "patch": 0 } },
                "selector": { "dir": dir.to_string_lossy() }
            },
        },
        // One binding holds both roles, as the Kernel's own coordinator tests bind a
        // Provider that is also the Authority. A second binding of the same Module for
        // the Authority breaks a Session: the implicit Spec makes every Provider-role
        // binding a resource (session.rs `implicit_spec`, row 3).
        "authority": "radio",
        "placements": {
            "links": [{
                "link": { "id": ezsdr_link_host::descriptor().id, "version": { "major": 1, "minor": 0, "patch": 0 } },
                "from": { "component": "radio", "port": "rx" },
                "to": { "component": "rec", "port": "in" }
            }]
        },
        "environment": {
            "ezsdr.time": { "class": class, "start_lead_ns": o.start_lead_ns },
            "ezsdr.rf_path": { "path": o.rf_path }
        }
    })
}
