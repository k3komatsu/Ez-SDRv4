//! The documentation audit's D3: from one selector, MockRadio and the UHD Module describe
//! one device alike, because both build their description with `ezsdr_radio::channels`
//! (design 33 D5). Board-derived values, and every comparable value but the repeat pair: the
//! two Providers differ by design in their repeat constraints only, which are not the board's.

use std::collections::BTreeMap;

use ezsdr_kernel::binding::Binding;
use ezsdr_kernel::id::{ModuleId, ResourceId};
use ezsdr_kernel::module_api::{ModuleRef, ProfileRef, Provider, Requested, Version};
use ezsdr_kernel::spec::{CapabilityValue, Constraint, Ident, Key, Scalar, Value};
use ezsdr_mock_radio::MockRadio;
use ezsdr_radio::channels::{self, Description};
use ezsdr_radio::measurement::{self, Rule};
use std::sync::Arc;

use ezsdr_radio_uhd::{ClockMode, FakeConfig, FakeDevice, UhdRadio};

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
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, front_ends: vec![front, front], ..FakeConfig::default() })) as Arc<dyn ezsdr_radio_uhd::Device>;
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
fn d3_the_two_providers_refuse_a_transmit_rate_below_one_msps_alike() {
    // KL-40: both Providers build their grid from the layout, so a transmit stream below 1 MS/s on `x3xx` is declared and refused alike:
    // the capability is the 200 rates 1 to 200 MS/s, 0.5 MS/s is rejected with the same reason on both and never coerced to 1 MS/s, and a
    // receive stream keeps 0.5 MS/s.
    let elements = vec!["0:A:ubx:0".to_owned(), "0:B:ubx:0".to_owned()];
    let selector = ezsdr_acceptance::layout::x3xx_selector(&elements, &elements);
    let mock = MockRadio::from_binding(&mock_binding(selector.clone())).unwrap();
    let device = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, front_ends: vec!["UBX", "UBX"], ..FakeConfig::default() })) as Arc<dyn ezsdr_radio_uhd::Device>;
    let uhd = UhdRadio::from_binding(&uhd_binding(selector), device).unwrap();
    let tx = Key::parse("radio.tx.s0.sample_rate_hz").unwrap();
    for provider in [&mock as &dyn Provider, &uhd as &dyn Provider] {
        let CapabilityValue::AnyOf { values } = &provider.instance().tree.capabilities[&tx] else { panic!("a rate grid is AnyOf") };
        assert_eq!(values.len(), 200);
        assert_eq!((values[0].clone(), values[values.len() - 1].clone()), (Scalar::try_from(200e6).unwrap(), Scalar::try_from(1e6).unwrap()));
        let id = provider.instance().id.clone();
        let ask = |key: &str, rate: f64| Requested { resource: id.clone(), constraints: BTreeMap::from([(Key::parse(key).unwrap(), Constraint::Eq { value: Scalar::try_from(rate).unwrap() })]) };
        let low = provider.coerce(&ask("radio.tx.s0.sample_rate_hz", 500_000.0)).unwrap();
        assert_eq!(low.rejected.len(), 1, "{:?}", low.rejected);
        assert!(low.rejected[0].reason.starts_with("RM-8: 500000 is outside 1000000..200000000"), "{}", low.rejected[0].reason);
        assert!(low.coercions.is_empty() && !low.applied.contains_key(&tx));
        let rx = provider.coerce(&ask("radio.rx.s0.sample_rate_hz", 500_000.0)).unwrap();
        assert!(rx.rejected.is_empty() && rx.coercions.is_empty(), "receive keeps 0.5 MS/s: {rx:?}");
    }
    let (m, u) = (&mock as &dyn Provider, &uhd as &dyn Provider);
    assert_eq!(m.instance().tree.capabilities[&tx], u.instance().tree.capabilities[&tx], "the two Providers declare one transmit grid");
}

#[test]
fn d3_a_named_selector_gives_each_provider_its_unnamed_description() {
    // RM-28: within each Provider the named selector builds the tree of the plain one; across Providers
    // only the board-derived surfaces are compared, as D3 does (the Providers differ in command lead).
    for (board, front) in [("ubx", "UBX"), ("obx", "OBX"), ("cbx", "CBX-120")] {
        let plain = vec![format!("0:A:{board}:0"), format!("0:B:{board}:0")];
        let named = vec![format!("0:A:{board}:0#in"), format!("0:B:{board}:0#aux")];
        let named_out = vec![format!("0:A:{board}:0#out"), format!("0:B:{board}:0")];
        let device = || Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, front_ends: vec![front, front], ..FakeConfig::default() })) as Arc<dyn ezsdr_radio_uhd::Device>;
        let mock = |rx: &[String], tx: &[String]| MockRadio::from_binding(&mock_binding(ezsdr_acceptance::layout::x3xx_selector(rx, tx))).unwrap();
        let uhd = |rx: &[String], tx: &[String]| UhdRadio::from_binding(&uhd_binding(ezsdr_acceptance::layout::x3xx_selector(rx, tx)), device()).unwrap();
        let (plain_mock, named_mock) = (mock(&plain, &plain), mock(&named, &named_out));
        let (plain_uhd, named_uhd) = (uhd(&plain, &plain), uhd(&named, &named_out));
        assert_eq!(plain_mock.instance().tree, named_mock.instance().tree, "{board}: MockRadio's tree");
        assert_eq!(plain_uhd.instance().tree, named_uhd.instance().tree, "{board}: the UHD Module's tree");
        for name in BOARD_KEYS {
            let key = Key::parse(name).unwrap();
            assert_eq!(named_mock.instance().tree.capabilities.get(&key), named_uhd.instance().tree.capabilities.get(&key), "{board}: `{name}`");
        }
        for key in ["radio.rx.c0.frequency_hz", "radio.tx.c0.frequency_hz"] {
            assert_eq!(default_of(&named_mock, key), default_of(&named_uhd, key), "{board}: the default `{key}`");
        }
    }
}

#[test]
fn d3_the_layout_decides_the_channels_on_both_providers() {
    // Asymmetric layouts: the length of each direction's list is its channel count, and
    // the rate grid does not depend on the layout at all.
    let selector = ezsdr_acceptance::layout::x3xx_selector(&["0:A:ubx:0".to_owned()], &["0:A:ubx:0".to_owned(), "0:B:ubx:0".to_owned()]);
    let mock = MockRadio::from_binding(&mock_binding(selector.clone())).unwrap();
    let device = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() })) as Arc<dyn ezsdr_radio_uhd::Device>;
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
fn d3_the_two_providers_default_every_channel_to_no_stream() {
    // RM-30: no channel of either direction is in a stream by default, on both Providers alike.
    let selector = ezsdr_acceptance::layout::x3xx_selector(&["0:A:ubx:0", "0:B:ubx:0"], &["0:A:ubx:0", "0:B:ubx:0"]);
    let mock = MockRadio::from_binding(&mock_binding(selector.clone())).unwrap();
    let device = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() })) as Arc<dyn ezsdr_radio_uhd::Device>;
    let uhd = UhdRadio::from_binding(&uhd_binding(selector), device).unwrap();
    for provider in [&mock as &dyn Provider, &uhd as &dyn Provider] {
        for direction in ["rx", "tx"] {
            for channel in 0..2 {
                assert_eq!(default_of(provider, &format!("radio.{direction}.c{channel}.stream")), Value::from(-1), "{direction} c{channel}");
            }
        }
    }
}

#[test]
fn d3_a_layout_without_receive_channels_requests_none_on_both_providers() {
    // An empty receive layout has no receive channel or stream keys.
    let selector = ezsdr_acceptance::layout::x3xx_selector(&[] as &[&str], &["0:A:ubx:0"]);
    let mock = MockRadio::from_binding(&mock_binding(selector.clone())).unwrap();
    let device = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() })) as Arc<dyn ezsdr_radio_uhd::Device>;
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
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, front_ends: vec![front, front], ..FakeConfig::default() })) as Arc<dyn ezsdr_radio_uhd::Device>;
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
    let uhd = UhdRadio::from_binding(&uhd_binding(selector), Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() })) as Arc<dyn ezsdr_radio_uhd::Device>).unwrap();
    for (provider, id) in [(&mock as &dyn Provider, "mock"), (&uhd as &dyn Provider, "usrp")] {
        let tree = &provider.instance().tree;
        assert_eq!(tree.id, ResourceId::parse(id).unwrap());
        let children: Vec<_> = tree.children.iter().map(|child| child.id.path()).collect();
        assert_eq!(children, [format!("{id}/rx/s0"), format!("{id}/tx/s0")]);
    }
}

/// Stage 4a (proposal §6.3): the Vocabulary's one set of values reaches both Providers. For every selector and every key whose measurement rule
/// compares it, the two Providers' profile values are equal except the repeat pair (Replay's size and alignment against the host stream's).
#[test]
fn d3_the_two_providers_share_every_comparable_value_but_the_repeat_pair() {
    let elements = |board: &str| vec![format!("0:A:{board}:0"), format!("0:B:{board}:0")];
    let with = |mut selector: BTreeMap<Ident, Value>, extra: &[(&str, Value)]| {
        for (key, value) in extra {
            selector.insert(Ident::parse(key).unwrap(), value.clone());
        }
        selector
    };
    let ubx = elements("ubx");
    let selectors = [
        ezsdr_acceptance::layout::x3xx_selector(&ubx, &ubx),
        with(ezsdr_acceptance::layout::x3xx_selector(&ubx, &ubx), &[("link_gbps", Value::List(vec![Scalar::Int(10)]))]),
        with(ezsdr_acceptance::layout::x3xx_selector(&ubx, &ubx), &[("image", Value::List(vec![Scalar::from("xg")]))]),
        ezsdr_acceptance::layout::x3xx_selector(&elements("cbx")[..1], &elements("cbx")[..1]),
    ];
    for selector in selectors {
        let layout = channels::from_selector(&selector).unwrap();
        let mock = Description::new(&layout, ezsdr_mock_radio::BLOCK_LEN, &ezsdr_mock_radio::Profile::terms());
        let uhd = Description::new(&layout, ezsdr_radio_uhd::profile::DEFAULT_BLOCK_LEN, &ezsdr_radio_uhd::profile::terms());
        let (mock, uhd) = (measurement::profile_values(&mock), measurement::profile_values(&uhd));
        assert_eq!(mock.keys().collect::<Vec<_>>(), uhd.keys().collect::<Vec<_>>());
        let differing: Vec<_> = mock.iter().filter(|(key, value)| measurement::rule(key.as_str()) != Some(Rule::NotCompared) && uhd[*key] != **value).map(|(key, _)| key.as_str().to_owned()).collect();
        assert_eq!(differing, ["radio.tx.repeat_align_samples", "radio.tx.repeat_max_samples"], "{selector:?}");
    }
}

/// Stage 4a: one 10 GbE link carries 800 MB/s a direction on both Providers: one channel at 200 MS/s and two of 100 MS/s are admitted, and
/// 200 + 100 and 200 + 50 MS/s are refused alike, in both directions (RM-32), with the limit in the reason; a 1 GbE link (the default selector) carries 25 MS/s.
#[test]
fn s4a_both_providers_admit_what_one_10gbe_link_carries_and_refuse_the_rest_alike() {
    let elements = vec!["0:A:ubx:0".to_owned(), "0:B:ubx:0".to_owned()];
    let mut ten = ezsdr_acceptance::layout::x3xx_selector(&elements, &elements);
    ten.insert(Ident::parse("link_gbps").unwrap(), Value::List(vec![Scalar::Int(10)]));
    let one = ezsdr_acceptance::layout::x3xx_selector(&elements, &elements);
    let device = || Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, front_ends: vec!["UBX", "UBX"], ..FakeConfig::default() })) as Arc<dyn ezsdr_radio_uhd::Device>;
    for (selector, cases) in
        [(ten, vec![(200e6, None, true), (100e6, Some(100e6), true), (200e6, Some(100e6), false), (200e6, Some(50e6), false)]), (one, vec![(25e6, None, true), (30e6, None, false)])]
    {
        let mock = MockRadio::from_binding(&mock_binding(selector.clone())).unwrap();
        let uhd = UhdRadio::from_binding(&uhd_binding(selector), device()).unwrap();
        for (direction, (first, second, admitted)) in ["rx", "tx"].into_iter().flat_map(|direction| cases.iter().map(move |case| (direction, *case))) {
            let mut reports = Vec::new();
            for provider in [&mock as &dyn Provider, &uhd as &dyn Provider] {
                let key = |name: String| Key::parse(&format!("radio.{direction}.{name}")).unwrap();
                let mut constraints = BTreeMap::from([
                    (key("c0.stream".into()), Constraint::Eq { value: Scalar::Int(0) }),
                    (key("s0.sample_rate_hz".into()), Constraint::Eq { value: Scalar::try_from(first).unwrap() }),
                ]);
                if let Some(second) = second {
                    constraints.insert(key("c1.stream".into()), Constraint::Eq { value: Scalar::Int(1) });
                    constraints.insert(key("s1.sample_rate_hz".into()), Constraint::Eq { value: Scalar::try_from(second).unwrap() });
                }
                let report = provider.coerce(&Requested { resource: provider.instance().id.clone(), constraints }).unwrap();
                assert_eq!(report.rejected.is_empty(), admitted, "{direction} {first} + {second:?}: {:?}", report.rejected);
                if !admitted {
                    assert!(report.rejected.iter().any(|refusal| refusal.reason.starts_with(&format!("RM-32: {direction} needs"))), "{:?}", report.rejected);
                }
                reports.push(report.rejected.iter().map(|refusal| (refusal.key.clone(), refusal.reason.clone())).collect::<Vec<_>>());
            }
            assert_eq!(reports[0], reports[1], "{direction} {first} + {second:?}: the Providers refuse alike");
        }
    }
}
