//! A controlled Provider clock for command-order tests without host deadlines.
use std::collections::BTreeMap;
use std::sync::Arc;
use ezsdr_kernel::event::EventCollector;
use ezsdr_kernel::id::ResourceId;
use ezsdr_kernel::module_api::{ModuleRegistry, Pacing};
use ezsdr_kernel::policy::{EventKindRegistry, Policy};
use ezsdr_kernel::time::{ClockDomain, ClockRegistry, EpochRef, ManualTimeAuthority, Rational};
use crate::device::{FakeConfig, FakeDevice};
use super::core::Core;

pub(super) fn rig() -> (Arc<Core>, Arc<FakeDevice>, Arc<ManualTimeAuthority>, Arc<EventCollector>) {
    let clocks = Arc::new(ClockRegistry::new());
    let root = clocks.allocate_id();
    clocks.register(ClockDomain::root(root, Rational::new(200_000_000, 1).unwrap(),
        EpochRef::Arbitrary { set_by: "uhd.test".to_owned() })).unwrap();
    let time = Arc::new(ManualTimeAuthority::new(clocks.clone(), root, &[], Pacing::FreeRunning).unwrap());
    let device = Arc::new(FakeDevice::new(FakeConfig::default()));
    let mut kinds = EventKindRegistry::with_kernel_kinds();
    ezsdr_radio::register(&mut ModuleRegistry::new(),
        &mut ezsdr_kernel::binding::AdmissionCheckRegistry::new(), &mut kinds).unwrap();
    let pairs: Vec<_> = ["usrp", "usrp/rx", "usrp/tx"].iter().flat_map(|s|
        kinds.kinds().into_iter().map(move |k| (ResourceId::parse(s).unwrap(), k))).collect();
    let events = Arc::new(EventCollector::new(&pairs, &kinds.kinds(), 4096, &Policy::default()));
    let core = Arc::new(Core::new(device.clone(), ResourceId::parse("usrp").unwrap(),
        root, time.clone(), clocks, events.clone(), Arc::new(BTreeMap::new()), Vec::new(),
        crate::profile::Profile::X310Ubx.description(2_000)));
    (core, device, time, events)
}
