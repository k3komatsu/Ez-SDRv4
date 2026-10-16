//! The documentation audit's D3: from one selector, MockRadio and the UHD Module describe
//! one device alike, because both build their description with `ezsdr_radio::x3xx`
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

/// The shared selector of one layout (design 33 D5).
fn selector(rx: &[String], tx: &[String]) -> BTreeMap<Ident, Value> {
    let list = |elements: &[String]| Value::List(elements.iter().map(|e| Scalar::from(e.as_str())).collect());
    BTreeMap::from([
        (Ident::parse("rx").unwrap(), list(rx)),
        (Ident::parse("tx").unwrap(), list(tx)),
    ])
}

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
const BOARD_KEYS: [&str; 17] = [
    "radio.rx.channels",
    "radio.tx.channels",
    "radio.rx.sample_rate_hz",
    "radio.tx.sample_rate_hz",
    "radio.rx.frequency_hz",
    "radio.tx.frequency_hz",
    "radio.rx.frequency_step_hz",
    "radio.tx.frequency_step_hz",
    "radio.rx.gain_db",
    "radio.tx.gain_db",
    "radio.rx.gain_step_db",
    "radio.tx.gain_step_db",
    "radio.rx.antenna",
    "radio.tx.antenna",
    "radio.rx.coherent",
    "radio.phase_behavior_on_retune",
    "radio.rx.block_len",
];

/// A provider's `coerce` answer for one `Present` constraint, which is its default.
fn default_of(provider: &dyn Provider, key: &str) -> Value {
    let id = provider.instance().id.clone();
    let request = Requested {
        resource: id,
        constraints: BTreeMap::from([(Key::parse(key).unwrap(), Constraint::Present {})]),
    };
    let report = provider.coerce(&request).expect("a Present constraint is coercible");
    report.applied[&Key::parse(key).unwrap()].clone()
}

#[test]
fn d3_the_two_providers_describe_one_layout_alike() {
    // The front end name UHD reports for each board (UR-5's comparison).
    for (board, front) in [("ubx", "UBX"), ("obx", "OBX"), ("cbx", "CBX-120")] {
        let elements = vec![format!("0:A:{board}:0"), format!("0:B:{board}:0")];
        let selector = selector(&elements, &elements);
        let mock = MockRadio::from_binding(&mock_binding(selector.clone())).unwrap();
        let device = Arc::new(FakeDevice::new(FakeConfig { front_ends: vec![front, front], ..FakeConfig::default() })) as Arc<dyn ezsdr_radio_uhd::Device>;
        let uhd = UhdRadio::from_binding(&uhd_binding(selector), device).unwrap();
        let mock_capabilities = &mock.instance().tree.capabilities;
        let uhd_capabilities = &uhd.instance().tree.capabilities;
        for name in BOARD_KEYS {
            let key = Key::parse(name).unwrap();
            assert_eq!(
                mock_capabilities.get(&key),
                uhd_capabilities.get(&key),
                "{board}: `{name}` differs between MockRadio and the UHD Module",
            );
        }
        // The board's default frequency reaches both directions, the same on both
        // Providers (RM-5, design 33 D12).
        for key in ["radio.rx.frequency_hz", "radio.tx.frequency_hz"] {
            assert_eq!(default_of(&mock, key), default_of(&uhd, key), "{board}: the default `{key}` differs");
        }
        if board == "cbx" {
            assert_eq!(default_of(&mock, "radio.rx.frequency_hz"), Value::num(2_450_000_000.0).unwrap());
            assert_eq!(
                mock_capabilities[&Key::parse("radio.phase_behavior_on_retune").unwrap()],
                CapabilityValue::One { value: Scalar::from("random".to_owned()) },
            );
        } else {
            assert_eq!(default_of(&mock, "radio.rx.frequency_hz"), Value::num(1_000_000_000.0).unwrap());
        }
    }
}

#[test]
fn d3_the_layout_decides_the_channels_on_both_providers() {
    // Asymmetric layouts: the length of each direction's list is its channel count, and
    // the rate grid does not depend on the layout at all.
    let selector = selector(&["0:A:ubx:0".to_owned()], &["0:A:ubx:0".to_owned(), "0:B:ubx:0".to_owned()]);
    let mock = MockRadio::from_binding(&mock_binding(selector.clone())).unwrap();
    let device = Arc::new(FakeDevice::new(FakeConfig::default())) as Arc<dyn ezsdr_radio_uhd::Device>;
    let uhd = UhdRadio::from_binding(&uhd_binding(selector), device).unwrap();
    let range = |min, max| CapabilityValue::Range { min, max };
    for provider in [&mock as &dyn Provider, &uhd as &dyn Provider] {
        let capabilities = &provider.instance().tree.capabilities;
        assert_eq!(capabilities[&Key::parse("radio.rx.channels").unwrap()], range(Scalar::from(0), Scalar::from(1)));
        assert_eq!(capabilities[&Key::parse("radio.tx.channels").unwrap()], range(Scalar::from(0), Scalar::from(2)));
    }
    let rates = |provider: &dyn Provider| provider.instance().tree.capabilities[&Key::parse("radio.rx.sample_rate_hz").unwrap()].clone();
    assert_eq!(rates(&mock), rates(&uhd));
    let CapabilityValue::AnyOf { values } = rates(&mock) else { panic!("a Values grid is an AnyOf capability") };
    assert_eq!(values.len(), 639);
}

#[test]
fn d3_a_layout_without_receive_channels_requests_none_on_both_providers() {
    // RM-5's default receive channel count follows the layout (design 33 D5): with
    // `rx: []` a Spec that does not request `radio.rx.channels` gets 0 from either
    // Provider, so neither opens a receive stream on a device with no receive front end.
    let selector = selector(&[], &["0:A:ubx:0".to_owned()]);
    let mock = MockRadio::from_binding(&mock_binding(selector.clone())).unwrap();
    let device = Arc::new(FakeDevice::new(FakeConfig::default())) as Arc<dyn ezsdr_radio_uhd::Device>;
    let uhd = UhdRadio::from_binding(&uhd_binding(selector), device).unwrap();
    let range = |min, max| CapabilityValue::Range { min, max };
    for provider in [&mock as &dyn Provider, &uhd as &dyn Provider] {
        let capabilities = &provider.instance().tree.capabilities;
        assert_eq!(capabilities[&Key::parse("radio.rx.channels").unwrap()], range(Scalar::from(0), Scalar::from(0)));
        assert_eq!(capabilities[&Key::parse("radio.tx.channels").unwrap()], range(Scalar::from(0), Scalar::from(1)));
        assert_eq!(default_of(provider, "radio.rx.channels"), Value::from(0));
        assert_eq!(default_of(provider, "radio.tx.channels"), Value::from(0));
    }
}

#[test]
fn d3_a_cbx_layout_is_not_coherent_on_either_provider() {
    // RM-9: `radio.rx.coherent` is the board's (design 33 D13). A CBX's channels align in
    // samples but their phases do not, so both Providers refuse a Spec that requires
    // `Eq(true)` on it; a UBX's accept the same request.
    let elements = |board: &str| vec![format!("0:A:{board}:0"), format!("0:B:{board}:0")];
    for (board, front, coherent) in [("cbx", "CBX-120", false), ("ubx", "UBX", true)] {
        let selector = selector(&elements(board), &elements(board));
        let mock = MockRadio::from_binding(&mock_binding(selector.clone())).unwrap();
        let device = Arc::new(FakeDevice::new(FakeConfig { front_ends: vec![front, front], ..FakeConfig::default() })) as Arc<dyn ezsdr_radio_uhd::Device>;
        let uhd = UhdRadio::from_binding(&uhd_binding(selector), device).unwrap();
        for provider in [&mock as &dyn Provider, &uhd as &dyn Provider] {
            assert_eq!(default_of(provider, "radio.rx.coherent"), Value::from(coherent), "{board}");
            let request = Requested {
                resource: provider.instance().id.clone(),
                constraints: BTreeMap::from([(
                    Key::parse("radio.rx.coherent").unwrap(),
                    Constraint::Eq { value: Scalar::Bool(true) },
                )]),
            };
            let report = provider.coerce(&request).unwrap();
            if coherent {
                assert_eq!(report.applied[&Key::parse("radio.rx.coherent").unwrap()], Value::from(true), "{board}");
                assert!(report.rejected.is_empty(), "{board}: {:?}", report.rejected);
            } else {
                let refusal = report.rejected.iter().find(|r| r.key.as_str() == "radio.rx.coherent").unwrap_or_else(|| panic!("{board}: {report:?}"));
                assert!(refusal.reason.starts_with("RM-8: "), "{board}: {}", refusal.reason);
            }
        }
    }
}

#[test]
fn d3_the_layout_is_a_node_id_of_neither_provider() {
    // The layout names front ends, not nodes: the tree keeps one device and its two
    // stream children on both Providers (RM-2).
    let selector = selector(&["0:A:ubx:0".to_owned()], &["0:A:ubx:0".to_owned()]);
    let mock = MockRadio::from_binding(&mock_binding(selector.clone())).unwrap();
    let uhd = UhdRadio::from_binding(&uhd_binding(selector), Arc::new(FakeDevice::new(FakeConfig::default())) as Arc<dyn ezsdr_radio_uhd::Device>).unwrap();
    for (provider, id) in [(&mock as &dyn Provider, "mock"), (&uhd as &dyn Provider, "usrp")] {
        let tree = &provider.instance().tree;
        assert_eq!(tree.id, ResourceId::parse(id).unwrap());
        let children: Vec<_> = tree.children.iter().map(|child| child.id.path()).collect();
        assert_eq!(children, [format!("{id}/rx"), format!("{id}/tx")]);
    }
}
