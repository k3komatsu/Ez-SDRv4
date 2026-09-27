//! Ez-SDR v4 Module `ezsdr.radio.uhd` 0.1.0 (plan/phase7/18-uhd-radio.md): a Radio
//! Provider and a device-paced Time Authority for one USRP X310 per Run, on UHD's own
//! C API. Everything but [`open`]'s device runs on [`FakeDevice`] without hardware.
#![deny(unsafe_code)]
#![warn(missing_docs)]

mod authority;
pub mod device;
pub mod profile;
mod provider;
#[cfg(feature = "uhd")]
#[allow(unsafe_code)]
mod uhd;

use std::sync::Arc;

use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::ModuleId;
use ezsdr_kernel::module_api::{
    Deployment, Factories, KERNEL_API, ModuleDescriptor, ModuleRef, Role, Version, VersionReq, VocabularyRequirement,
};
use ezsdr_kernel::spec::Namespace;

pub use authority::DeviceAuthority;
pub use device::{
    Applied, Device, DeviceError, Dir, FakeConfig, FakeDevice, FakeFault, Iq, RxRecv, Settings, TxCode, TxReport,
    decimation, from_time_spec, to_time_spec,
};
pub use provider::UhdRadio;
/// UHD's own calls for the C API tests and the bench (feature `uhd`, UR-3).
#[cfg(feature = "uhd")]
pub use uhd::{UhdDevice, find as uhd_find, streamer_lifecycle as uhd_streamer_lifecycle, struct_sizes as uhd_struct_sizes};

/// `ezsdr.radio.uhd` 0.1.0 (UR-1).
pub fn module_ref() -> ModuleRef {
    ModuleRef {
        id: ModuleId::parse("ezsdr.radio.uhd").expect("a valid Module id"),
        version: Version::new(0, 1, 0),
    }
}

/// The Module descriptor: one Module, the Provider and the Authority roles (UR-1).
pub fn descriptor() -> ModuleDescriptor {
    let module = module_ref();
    ModuleDescriptor {
        id: module.id,
        version: module.version,
        kernel_api: KERNEL_API,
        roles: vec![Role::Provider, Role::Authority],
        vocabularies: vec![VocabularyRequirement {
            id: Namespace::parse(ezsdr_radio::VOCABULARY).expect("the radio namespace"),
            req: VersionReq(Version::new(1, 3, 0)),
        }],
        deployment: Deployment::InProcess {},
        impl_hash: Some(ContentHash::of_bytes(b"ezsdr.radio.uhd 0.1.0")),
    }
}

/// The roles a runtime registers the descriptor with (UR-1).
pub fn factories() -> Factories {
    Factories { provider: true, authority: true, ..Factories::default() }
}

/// Opens the device UHD's device string `args` names (UR-4). It never returns a
/// [`FakeDevice`], whatever `args` says (GZ-9).
pub fn open(args: &str) -> Result<Arc<dyn Device>, String> {
    #[cfg(feature = "uhd")]
    {
        uhd::UhdDevice::open(args).map(|device| Arc::new(device) as Arc<dyn Device>)
    }
    #[cfg(not(feature = "uhd"))]
    {
        let _ = args;
        Err("ezsdr.radio.uhd: built without UHD (feature `uhd`)".to_owned())
    }
}
