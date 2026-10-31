//! The documentation audit's D3: from one selector, MockRadio and the UHD Module describe
//! one device alike, because both build their description with `ezsdr_radio::channels`
//! (design 33 D5). Board-derived values only: the two Providers differ by design in their
//! command lead and repeat constraints at this stage, and those are not the board's.

use std::collections::BTreeMap;

use ezsdr_kernel::binding::Binding;
use ezsdr_kernel::id::{ModuleId, ResourceId};
use ezsdr_kernel::module_api::{ModuleRef, ProfileRef, Provider, Requested, Version};
use ezsdr_kernel::spec::{CapabilityValue, Constraint, Ident, Key, Scalar, Value};
use ezsdr_mock_radio::MockRadio;
use std::sync::Arc;

use ezsdr_radio_uhd::{FakeConfig, FakeDevice, UhdRadio};

fn mock_binding(selector: BTreeMap<Ident, Value>) -> Binding {
    Binding {
        module: ModuleRef { id: ModuleId::parse("ezsdr.radio.mock").unwrap(), version: Version::new(2, 0, 0) },
        selector,
        profile: Some(ProfileRef { name: "x3xx".to_owned(), version: Version::new(0, 1, 0) }),
        feed: None,
    }
}

fn uhd_binding(mut selector: BTreeMap<Ident, Value>) -> Binding {
    selector.insert(Ident::parse("args").unwrap(), Value::from("type=fake".to_owned()));
    Binding {
        module: ModuleRef { id: ModuleId::parse("ezsdr.radio.uhd").unwrap(), version: Version::new(0, 4, 0) },
        selector,
        profile: Some(ProfileRef { name: "x3xx".to_owned(), version: Version::new(0, 1, 0) }),
        feed: None,
    }
}

/// The capability keys that come from the layout's board: the ones D3 asks about.
const BOARD_KEYS: [&str; 19] = [
    "radio.rx.c0.frequency_hz",
    "radio.tx.c0.frequency_hz",
    "radio.rx.s0.sample_rate_hz",
    "radio.tx.s0.sample_rate_hz",
    "radio.rx.c0.frequency_step_hz",
    "radio.tx.c0.frequency_step_hz",
    "radio.rx.c0.gain_db",
    "radio.tx.c0.gain_db",
    "radio.rx.c0.gain_step_db",
    "radio.tx.c0.gain_step_db",
    "radio.rx.c0.antenna",
    "radio.tx.c0.antenna",
    "radio.rx.c0.phase_behavior_on_retune",
    "radio.tx.c0.phase_behavior_on_retune",
    "radio.rx.c0.path_delay_samples",
    "radio.tx.c0.path_delay_samples",
    "radio.full_duplex",
    "radio.hardware_time",
    "radio.rx.block_len",
];

/// A provider's `coerce` answer for one `Present` constraint, which is its default.
fn default_of(provider: &dyn Provider, key: &str) -> Value {
    let id = provider.instance().id.clone();
    let request = Requested { resource: id, constraints: BTreeMap::from([(Key::parse(key).unwrap(), Constraint::Present {})]) };
    let report = provider.coerce(&request).expect("a Present constraint is coercible");
    report.applied[&Key::parse(key).unwrap()].clone()
}

#[test]
fn d3_the_two_providers_describe_one_layout_alike() {
    // The front end name UHD reports for each board (UR-5's comparison).
    for (board, front) in [("ubx", "UBX"), ("obx", "OBX"), ("cbx", "CBX-120")] {
        let elements = vec![format!("0:A:{board}:0"), format!("0:B:{board}:0")];
        let selector = ezsdr_acceptance::layout::x3xx_selector(&elements, &elements);
        let mock = MockRadio::from_binding(&mock_binding(selector.clone())).unwrap();
        let device = Arc::new(FakeDevice::new(FakeConfig { front_ends: vec![front, front], ..FakeConfig::default() })) as Arc<dyn ezsdr_radio_uhd::Device>;
        let uhd = UhdRadio::from_binding(&uhd_binding(selector), device).unwrap();
        let mock_capabilities = &mock.instance().tree.capabilities;
        let uhd_capabilities = &uhd.instance().tree.capabilities;
        for name in BOARD_KEYS {
            let key = Key::parse(name).unwrap();
            assert_eq!(mock_capabilities.get(&key), uhd_capabilities.get(&key), "{board}: `{name}` differs between MockRadio and the UHD Module",);
        }
        // The board's default frequency reaches both directions, the same on both
        // Providers (RM-5, design 33 D12).
        for key in ["radio.rx.c0.frequency_hz", "radio.tx.c0.frequency_hz"] {
            assert_eq!(default_of(&mock, key), default_of(&uhd, key), "{board}: the default `{key}` differs");
        }
        if board == "cbx" {
            assert_eq!(default_of(&mock, "radio.rx.c0.frequency_hz"), Value::num(2_450_000_000.0).unwrap());
            assert_eq!(mock_capabilities[&Key::parse("radio.rx.c0.phase_behavior_on_retune").unwrap()], CapabilityValue::One { value: Scalar::from("random".to_owned()) },);
        } else {
            assert_eq!(default_of(&mock, "radio.rx.c0.frequency_hz"), Value::num(1_000_000_000.0).unwrap());
        }
    }
}

#[test]
fn d3_the_layout_decides_the_channels_on_both_providers() {
    // Asymmetric layouts: the length of each direction's list is its channel count, and
    // the rate grid does not depend on the layout at all.
    let selector = ezsdr_acceptance::layout::x3xx_selector(&["0:A:ubx:0".to_owned()], &["0:A:ubx:0".to_owned(), "0:B:ubx:0".to_owned()]);
    let mock = MockRadio::from_binding(&mock_binding(selector.clone())).unwrap();
    let device = Arc::new(FakeDevice::new(FakeConfig::default())) as Arc<dyn ezsdr_radio_uhd::Device>;
    let uhd = UhdRadio::from_binding(&uhd_binding(selector), device).unwrap();
    let range = |min, max| CapabilityValue::Range { min, max };
    for provider in [&mock as &dyn Provider, &uhd as &dyn Provider] {
        let capabilities = &provider.instance().tree.capabilities;
        assert_eq!(capabilities[&Key::parse("radio.rx.c0.stream").unwrap()], range(Scalar::from(-1), Scalar::from(0)));
        for key in ["radio.tx.c0.stream", "radio.tx.c1.stream"] {
            assert_eq!(capabilities[&Key::parse(key).unwrap()], range(Scalar::from(-1), Scalar::from(1)));
        }
    }
    let rates = |provider: &dyn Provider| provider.instance().tree.capabilities[&Key::parse("radio.rx.s0.sample_rate_hz").unwrap()].clone();
    assert_eq!(rates(&mock), rates(&uhd));
    let CapabilityValue::AnyOf { values } = rates(&mock) else { panic!("a Values grid is an AnyOf capability") };
    assert_eq!(values.len(), 639);
}

#[test]
fn d3_a_layout_without_receive_channels_requests_none_on_both_providers() {
    // An empty receive layout has no receive channel or stream keys.
    let selector = ezsdr_acceptance::layout::x3xx_selector(&[] as &[&str], &["0:A:ubx:0"]);
    let mock = MockRadio::from_binding(&mock_binding(selector.clone())).unwrap();
    let device = Arc::new(FakeDevice::new(FakeConfig::default())) as Arc<dyn ezsdr_radio_uhd::Device>;
    let uhd = UhdRadio::from_binding(&uhd_binding(selector), device).unwrap();
    for provider in [&mock as &dyn Provider, &uhd as &dyn Provider] {
        let capabilities = &provider.instance().tree.capabilities;
        assert!(!capabilities.keys().any(|key| key.as_str().starts_with("radio.rx.c") || key.as_str().starts_with("radio.rx.s")));
        assert!(capabilities.contains_key(&Key::parse("radio.tx.c0.stream").unwrap()));
        assert_eq!(default_of(provider, "radio.tx.c0.stream"), Value::from(-1));
    }
}

#[test]
fn d3_a_cbx_layout_is_not_coherent_on_either_provider() {
    // A layout describes the board's per-channel phase behavior instead of a direction-wide
    // `radio.rx.coherent` capability (design 33 D13).
    let elements = |board: &str| vec![format!("0:A:{board}:0"), format!("0:B:{board}:0")];
    for (board, front, coherent) in [("cbx", "CBX-120", false), ("ubx", "UBX", true)] {
        let channels = elements(board);
        let selector = ezsdr_acceptance::layout::x3xx_selector(&channels, &channels);
        let mock = MockRadio::from_binding(&mock_binding(selector.clone())).unwrap();
        let device = Arc::new(FakeDevice::new(FakeConfig { front_ends: vec![front, front], ..FakeConfig::default() })) as Arc<dyn ezsdr_radio_uhd::Device>;
        let uhd = UhdRadio::from_binding(&uhd_binding(selector), device).unwrap();
        for provider in [&mock as &dyn Provider, &uhd as &dyn Provider] {
            let phase = if coherent { "random_unless_timed_tune" } else { "random" };
            assert_eq!(default_of(provider, "radio.rx.c0.phase_behavior_on_retune"), Value::from(phase), "{board}");
            let request =
                Requested { resource: provider.instance().id.clone(), constraints: BTreeMap::from([(Key::parse("radio.rx.coherent").unwrap(), Constraint::Eq { value: Scalar::from(true) })]) };
            let report = provider.coerce(&request).unwrap();
            assert!(report.applied.is_empty(), "{board}");
            assert_eq!(report.rejected.len(), 1, "{board}");
            assert!(report.rejected[0].reason.contains("RM-4: not a radio key"), "{board}: {:?}", report.rejected);
        }
    }
}

#[test]
fn d3_the_layout_is_a_node_id_of_neither_provider() {
    // The layout names front ends, not nodes: the tree keeps one device and its two
    // stream children on both Providers (RM-2).
    let selector = ezsdr_acceptance::layout::x3xx_selector(&["0:A:ubx:0".to_owned()], &["0:A:ubx:0".to_owned()]);
    let mock = MockRadio::from_binding(&mock_binding(selector.clone())).unwrap();
    let uhd = UhdRadio::from_binding(&uhd_binding(selector), Arc::new(FakeDevice::new(FakeConfig::default())) as Arc<dyn ezsdr_radio_uhd::Device>).unwrap();
    for (provider, id) in [(&mock as &dyn Provider, "mock"), (&uhd as &dyn Provider, "usrp")] {
        let tree = &provider.instance().tree;
        assert_eq!(tree.id, ResourceId::parse(id).unwrap());
        let children: Vec<_> = tree.children.iter().map(|child| child.id.path()).collect();
        assert_eq!(children, [format!("{id}/rx/s0"), format!("{id}/tx/s0")]);
    }
}
