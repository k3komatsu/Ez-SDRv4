//! RM-27…RM-33: the layout builder's keys, layout, tree, coercion, full-duplex
//! constraint, throughput limit and per-channel RF envelope (design 33 D1…D9).

use std::collections::BTreeMap;
use std::sync::Arc;

use ezsdr_kernel::binding::{AdmissionCheckRegistry, CheckStage, Violation};
use ezsdr_kernel::id::{MemoryDomainId, ResourceId};
use ezsdr_kernel::module_api::{CoerceReport, ModuleRegistry, RejectedRequest, Requested, UpdateClass};
use ezsdr_kernel::policy::EventKindRegistry;
use ezsdr_kernel::spec::{
    CapabilityValue, CoercionPolicy, Constraint, Ident, Key, Namespace, Scalar, Value, ValueKind,
};
use ezsdr_radio::channels::{self, Description, Layout};
use ezsdr_radio::x3xx::{self, FrontEnd, Image, ProviderTerms, Slot};
use ezsdr_radio::{keys, RfEnvelopeCheck};
use serde_json::{Value as JsonValue, json};

fn key(name: &str) -> Key {
    Key::parse(name).unwrap()
}

fn ident(name: &str) -> Ident {
    Ident::parse(name).unwrap()
}

fn num(value: f64) -> Scalar {
    Scalar::try_from(value).unwrap()
}

fn strings(values: &[&str]) -> Value {
    Value::List(values.iter().map(|value| Scalar::from(*value)).collect())
}

fn ints(values: &[i64]) -> Value {
    Value::List(values.iter().map(|value| Scalar::from(*value)).collect())
}

fn selector(pairs: &[(&str, Value)]) -> BTreeMap<Ident, Value> {
    pairs.iter().map(|(name, value)| (ident(name), value.clone())).collect()
}

/// A selector with the shared values of the owner's layout and the given directions.
fn layout_selector(rx: &[&str], tx: &[&str]) -> BTreeMap<Ident, Value> {
    selector(&[
        ("master_clock_hz", ints(&[200_000_000])),
        ("image", strings(&["xg"])),
        ("links", ints(&[2])),
        ("link_gbps", ints(&[10])),
        ("rx", strings(rx)),
        ("tx", strings(tx)),
    ])
}

fn terms() -> ProviderTerms {
    ProviderTerms {
        min_timed_command_lead_ns: 2_000_000,
        repeat_max_samples: 268_435_456,
        repeat_align_samples: 2,
    }
}

fn description_of(rx: &[&str], tx: &[&str]) -> Description {
    let layout = channels::from_selector(&layout_selector(rx, tx)).unwrap();
    Description::new(&layout, 2_000, &terms())
}

/// The owner's mixed layout: a UBX in slot A, two BasicRX channels and an LFTX in slot B.
fn owner_layout() -> Layout {
    channels::from_selector(&layout_selector(
        &["0:A:ubx:0", "0:B:basic_rx:0", "0:B:basic_rx:1"],
        &["0:A:ubx:0", "0:B:lf_tx:0"],
    ))
    .unwrap()
}

fn owner_description() -> Description {
    Description::new(&owner_layout(), 2_000, &terms())
}

fn eq_num(value: f64) -> Constraint {
    Constraint::Eq { value: num(value) }
}

fn eq_int(value: i64) -> Constraint {
    Constraint::Eq { value: Scalar::from(value) }
}

fn eq_str(value: &str) -> Constraint {
    Constraint::Eq { value: Scalar::from(value) }
}

fn request(pairs: &[(&str, Constraint)]) -> Requested {
    Requested {
        resource: ResourceId::parse("dev").unwrap(),
        constraints: pairs.iter().map(|(name, constraint)| (key(name), constraint.clone())).collect(),
    }
}

/// Every configuration key a layout defines, in direction and channel order (RM-30).
fn configuration_keys(description: &Description) -> Vec<Key> {
    let mut configuration = Vec::new();
    for (direction, count) in [("rx", description.rx.len()), ("tx", description.tx.len())] {
        for number in 0..count {
            for leaf in ["frequency_hz", "gain_db", "antenna", "stream"] {
                configuration.push(key(&keys::channel(direction, number as u32, leaf)));
            }
        }
        for k in 0..count {
            configuration.push(key(&keys::stream(direction, k as u32)));
        }
    }
    configuration
}

fn present_all(description: &Description) -> Requested {
    Requested {
        resource: ResourceId::parse("dev").unwrap(),
        constraints: configuration_keys(description)
            .into_iter()
            .map(|key| (key, Constraint::Present {}))
            .collect(),
    }
}

/// One instance's whole configuration (RM-30): the defaults overlaid with `pairs`.
fn configuration(description: &Description, pairs: &[(&str, Value)]) -> BTreeMap<Key, Value> {
    let mut configuration = description.defaults();
    for (name, value) in pairs {
        configuration.insert(key(name), value.clone());
    }
    configuration
}

fn value_of(report: &CoerceReport, name: &str) -> Value {
    report
        .applied
        .get(&key(name))
        .unwrap_or_else(|| panic!("{name} is applied: {:?}", report.rejected))
        .clone()
}

fn rejection<'a>(report: &'a CoerceReport, name: &str) -> &'a RejectedRequest {
    report
        .rejected
        .iter()
        .find(|rejection| rejection.key == key(name))
        .unwrap_or_else(|| panic!("{name} is refused: {:?}", report.rejected))
}

#[test]
fn rm_27_the_layout_builder_keys_are_declared() {
    let mut registry = ModuleRegistry::new();
    let mut checks = AdmissionCheckRegistry::new();
    let mut kinds = EventKindRegistry::new();
    ezsdr_radio::register(&mut registry, &mut checks, &mut kinds).unwrap();

    // The numbered declaration covers `c3`, and not its own spelling or an index with
    // a leading zero (SB-2a).
    let declaration = registry.key_decl(&key("radio.rx.c3.frequency_hz")).unwrap();
    assert_eq!(declaration.key.as_str(), "radio.rx.c.frequency_hz");
    assert_eq!(declaration.indexed, Some(2));
    assert!(registry.key_decl(&key("radio.rx.c.frequency_hz")).is_none());
    assert!(registry.key_decl(&key("radio.rx.c03.frequency_hz")).is_none());

    // The one place the concrete spellings are built.
    assert_eq!(keys::channel("rx", 3, "frequency_hz"), "radio.rx.c3.frequency_hz");
    assert_eq!(keys::channel("tx", 12, "stream"), "radio.tx.c12.stream");
    assert_eq!(keys::stream("tx", 2), "radio.tx.s2.sample_rate_hz");

    // Every declaration of RM-27's table, both directions.
    let rows = [
        ("frequency_hz", ValueKind::Num, true, CoercionPolicy::Reject, Some(UpdateClass::HardwareTimed)),
        ("gain_db", ValueKind::Num, true, CoercionPolicy::Warn, Some(UpdateClass::HardwareTimed)),
        ("antenna", ValueKind::Str, false, CoercionPolicy::Reject, Some(UpdateClass::Cold)),
        ("stream", ValueKind::Int, false, CoercionPolicy::Reject, Some(UpdateClass::Cold)),
        ("frequency_step_hz", ValueKind::Num, false, CoercionPolicy::Reject, None),
        ("gain_step_db", ValueKind::Num, false, CoercionPolicy::Reject, None),
        ("phase_behavior_on_retune", ValueKind::Str, false, CoercionPolicy::Reject, None),
        ("path_delay_samples", ValueKind::Int, false, CoercionPolicy::Reject, None),
    ];
    let declarations = ezsdr_radio::vocabulary().keys;
    let find = |name: &str| {
        declarations
            .iter()
            .find(|declaration| declaration.key.as_str() == name)
            .unwrap_or_else(|| panic!("{name} is declared"))
    };
    for direction in ["rx", "tx"] {
        for (leaf, kind, coercible, coercion_default, update_class) in rows {
            let declaration = find(&format!("radio.{direction}.c.{leaf}"));
            assert_eq!(declaration.indexed, Some(2), "{direction} {leaf}");
            assert_eq!(declaration.kind, kind, "{direction} {leaf}");
            assert_eq!(declaration.coercible, coercible, "{direction} {leaf}");
            assert_eq!(declaration.coercion_default, coercion_default, "{direction} {leaf}");
            assert_eq!(declaration.update_class, update_class, "{direction} {leaf}");
        }
        let declaration = find(&format!("radio.{direction}.s.sample_rate_hz"));
        assert_eq!(declaration.indexed, Some(2), "{direction} sample rate");
        assert_eq!(declaration.kind, ValueKind::Num);
        assert!(declaration.coercible);
        assert_eq!(declaration.coercion_default, CoercionPolicy::Reject);
        assert_eq!(declaration.update_class, Some(UpdateClass::Cold));
    }
}

#[test]
fn rm_28_the_layout_analysis_cases() {
    let Layout::X3xx { master_clock_hz, image, links, link_gbps, rx, tx } = owner_layout() else {
        panic!("an x3xx layout")
    };
    assert_eq!((master_clock_hz, image, links, link_gbps), (200_000_000, Image::Xg, 2, 10));
    assert_eq!(rx.len(), 3);
    assert_eq!(tx.len(), 2);
    assert_eq!((rx[0].slot, rx[0].front_end, rx[0].frontend), (Slot::A, FrontEnd::Ubx, 0));
    assert_eq!((rx[1].slot, rx[1].front_end, rx[1].frontend), (Slot::B, FrontEnd::BasicRx, 0));
    assert_eq!((rx[2].slot, rx[2].front_end, rx[2].frontend), (Slot::B, FrontEnd::BasicRx, 1));
    assert_eq!((tx[0].slot, tx[0].front_end), (Slot::A, FrontEnd::Ubx));
    assert_eq!((tx[1].slot, tx[1].front_end), (Slot::B, FrontEnd::LfTx));

    // A layout of slot B only, and one whose receive order is not UHD's default order,
    // are both admitted: this analysis has no prefix rule.
    assert!(channels::from_selector(&layout_selector(&["0:B:basic_rx:0"], &[])).is_ok());
    assert!(channels::from_selector(&layout_selector(&["0:B:basic_rx:1", "0:B:basic_rx:0"], &[])).is_ok());

    for (rx, tx, needle) in [
        (&["0:B:sbx:0"][..], &[][..], "does not describe"),
        (&[][..], &["0:B:basic_rx:0"][..], "does not transmit"),
        (&[][..], &["0:B:basic_tx:1"][..], "front end 1"),
        (&["0:B:ubx:1"][..], &[][..], "front end 1"),
        (&["0:B:basic_rx:0", "0:B:lf_rx:0"][..], &[][..], "already carries"),
        (&["0:B:ubx:0"][..], &["0:B:lf_tx:0"][..], "duplex board"),
        (&["0:B:basic_rx:0", "0:B:basic_rx:0"][..], &[][..], "repeats"),
        (&["1:B:basic_rx:0"][..], &[][..], "timekeeper"),
        (&["0:C:basic_rx:0"][..], &[][..], "slot"),
        (&[][..], &[][..], "names no front end"),
    ] {
        let error = channels::from_selector(&layout_selector(rx, tx)).unwrap_err();
        assert!(error.contains(needle), "`{error}` names {needle}");
    }
    // Each element's refusal names its position and value: the selector's own spelling
    // of the element where its parse refuses it, and the element's board, front end and
    // slot where the layout's own checks do (RM-28).
    for (rx, tx, position, value) in [
        (&["0:B:sbx:0"][..], &[][..], 0, "0:B:sbx:0"),
        (&[][..], &["0:B:basic_rx:0"][..], 0, "0:B:basic_rx:0"),
        (&[][..], &["0:B:basic_tx:1"][..], 0, "0:B:basic_tx:1"),
        (&["0:B:ubx:1"][..], &[][..], 0, "0:B:ubx:1"),
        (&["0:B:basic_rx:0", "0:B:lf_rx:0"][..], &[][..], 1, "`lf_rx` front end 0"),
        (&["0:B:basic_rx:0", "0:B:basic_rx:0"][..], &[][..], 1, "basic_rx"),
        (&["1:B:basic_rx:0"][..], &[][..], 0, "1:B:basic_rx:0"),
        (&["0:C:basic_rx:0"][..], &[][..], 0, "0:C:basic_rx:0"),
    ] {
        let error = channels::from_selector(&layout_selector(rx, tx)).unwrap_err();
        assert!(
            error.contains(&format!("position {position}")) && error.contains(value),
            "`{error}` names position {position} and {value}"
        );
    }
    // The layout's own checks name the element's board, front end and slot.
    for (rx, needles) in [
        (&["0:B:basic_rx:0", "0:B:lf_rx:0"][..], ["`lf_rx` front end 0", "slot B", "`basic_rx`"]),
        (&["0:B:basic_rx:0", "0:B:basic_rx:0"][..], ["`basic_rx` front end 0", "slot B", "position 1"]),
    ] {
        let error = channels::from_selector(&layout_selector(rx, &[])).unwrap_err();
        for needle in needles {
            assert!(error.contains(needle), "`{error}` names {needle}");
        }
    }
    // A slot's refusal names the slot and both boards.
    let error = channels::from_selector(&layout_selector(&["0:B:ubx:0"], &["0:B:lf_tx:0"])).unwrap_err();
    assert!(
        error.contains("position 0") && error.contains("slot B") && error.contains("ubx") && error.contains("lf_tx"),
        "{error}"
    );

    // The other two duplex boards parse, and the clock-compatibility table admits them
    // and the Basic/LF boards; it is consulted for every pair of different board kinds on
    // one timekeeper, so the owner's mixed layout above (a UBX in slot A with a BasicRX
    // and an LFTX in slot B) is admitted through it.
    for board in ["obx", "cbx"] {
        let element = format!("0:A:{board}:0");
        assert!(
            channels::from_selector(&layout_selector(&[element.as_str()], &[element.as_str()])).is_ok(),
            "{board}"
        );
    }
    assert!(x3xx::boards_compatible(FrontEnd::Ubx, FrontEnd::LfTx));
    assert!(x3xx::boards_compatible(FrontEnd::Cbx, FrontEnd::Cbx));

    // The shared lists have one entry per timekeeper.
    let mut shared = layout_selector(&["0:B:basic_rx:0"], &[]);
    shared.insert(ident("master_clock_hz"), ints(&[200_000_000, 200_000_000]));
    assert!(channels::from_selector(&shared).unwrap_err().contains("one-element list"));
    let mut shared = layout_selector(&["0:B:basic_rx:0"], &[]);
    shared.insert(ident("links"), ints(&[1, 2]));
    assert!(channels::from_selector(&shared).unwrap_err().contains("one-element list"));

    // An absent `link_gbps` follows the image: the HG's port 0 is 1 GbE, the XG's 10.
    let minimal = selector(&[("rx", strings(&["0:B:basic_rx:0"])), ("tx", strings(&[]))]);
    let description = Description::new(&channels::from_selector(&minimal).unwrap(), 2_000, &terms());
    assert_eq!(description.performance.rx_bytes_per_s, 100_000_000);
    let mut xg = minimal.clone();
    xg.insert(ident("image"), strings(&["xg"]));
    let description = Description::new(&channels::from_selector(&xg).unwrap(), 2_000, &terms());
    assert_eq!(description.performance.rx_bytes_per_s, 1_000_000_000);

    // The `ideal` layout: channel counts, 1 receive and 0 transmit by default.
    let ideal = channels::ideal_from_selector(&selector(&[("rx", Value::from(3_i64)), ("tx", Value::from(1_i64))]));
    assert_eq!(ideal, Ok(Layout::Ideal { rx: 3, tx: 1 }));
    assert_eq!(
        channels::ideal_from_selector(&selector(&[])),
        Ok(Layout::Ideal { rx: 1, tx: 0 })
    );
    assert!(channels::ideal_from_selector(&selector(&[("rx", Value::from(65_i64))])).is_err());
    assert!(channels::ideal_from_selector(&selector(&[("tx", Value::from(-1_i64))])).is_err());
}

#[test]
fn rm_29_the_layout_description_builds_the_stream_tree() {
    let description = owner_description();
    let id = ResourceId::parse("dev").unwrap();
    let domain = MemoryDomainId::local(41);
    let tree = description.tree(&id, domain);

    let children: Vec<(String, String)> = tree
        .children
        .iter()
        .map(|child| (child.id.path().to_owned(), child.kind.to_string()))
        .collect();
    assert_eq!(
        children,
        [
            ("dev/rx/s0", "radio.rx_stream"),
            ("dev/rx/s1", "radio.rx_stream"),
            ("dev/rx/s2", "radio.rx_stream"),
            ("dev/tx/s0", "radio.tx_stream"),
            ("dev/tx/s1", "radio.tx_stream"),
        ]
        .map(|(id, kind)| (id.to_owned(), kind.to_owned()))
    );
    let ports: Vec<&str> = tree.ports.iter().map(|port| port.name.as_str()).collect();
    assert_eq!(ports, ["rx0", "rx1", "rx2"]);
    assert!(tree.ports.iter().all(|port| port.contract.as_str() == "ezsdr.stream.cf32"));
    assert!(tree
        .children
        .iter()
        .all(|child| child.capabilities.is_empty() && child.ports.is_empty() && !child.shareable));
    assert!(tree.children.iter().all(|child| child.memory_domains.is_empty()));
    assert_eq!(tree.memory_domains, [domain]);
    // No channel node and no timekeeper node (RM-29).
    assert!(!tree.children.iter().any(|child| child.id.path() == "dev/rx/c0"));
    assert!(!tree.children.iter().any(|child| child.id.path() == "dev/t0"));
    assert!(!tree.capabilities.contains_key(&key("radio.rx.channels")));
    assert!(!tree.capabilities.contains_key(&key("radio.rx.coherent")));

    let capability = |name: &str| tree.capabilities.get(&key(name)).unwrap_or_else(|| panic!("{name}"));
    let range = |min: Scalar, max: Scalar| CapabilityValue::Range { min, max };
    let one = |value: Scalar| CapabilityValue::One { value };
    let names = |names: &[&str]| CapabilityValue::AnyOf {
        values: names.iter().map(|name| Scalar::from(*name)).collect(),
    };
    // The UBX: RM-9's values.
    assert_eq!(
        capability("radio.rx.c0.frequency_hz"),
        &range(num(10_000_000.0), num(6_000_000_000.0))
    );
    assert_eq!(capability("radio.rx.c0.gain_db"), &range(num(0.0), num(31.5)));
    assert_eq!(capability("radio.rx.c0.antenna"), &names(&["RX2", "TX/RX"]));
    assert_eq!(capability("radio.tx.c0.antenna"), &names(&["TX/RX"]));
    // The BasicRX rows: 0…100 MHz, `{0}` dB, four antenna modes.
    assert_eq!(capability("radio.rx.c1.frequency_hz"), &range(num(0.0), num(100_000_000.0)));
    assert_eq!(capability("radio.rx.c1.gain_db"), &range(num(0.0), num(0.0)));
    assert_eq!(capability("radio.rx.c1.antenna"), &names(&["A", "B", "AB", "BA"]));
    assert_eq!(capability("radio.rx.c2.antenna"), &names(&["A", "B", "AB", "BA"]));
    // The LFTX row: 0…32 MHz.
    assert_eq!(capability("radio.tx.c1.frequency_hz"), &range(num(0.0), num(32_000_000.0)));
    // Per-channel membership, step, phase and path delay.
    assert_eq!(capability("radio.rx.c0.stream"), &range(Scalar::Int(-1), Scalar::Int(2)));
    assert_eq!(capability("radio.tx.c1.stream"), &range(Scalar::Int(-1), Scalar::Int(1)));
    assert_eq!(
        capability("radio.rx.c0.phase_behavior_on_retune"),
        &one(Scalar::from("random_unless_timed_tune"))
    );
    assert_eq!(capability("radio.tx.c0.path_delay_samples"), &one(Scalar::Int(45)));
    assert_eq!(capability("radio.rx.c0.path_delay_samples"), &one(Scalar::Int(0)));
    assert_eq!(capability("radio.tx.c1.path_delay_samples"), &one(Scalar::Int(45)));
    assert_eq!(capability("radio.rx.c0.frequency_step_hz"), &one(num(1.0)));
    assert_eq!(capability("radio.rx.c0.gain_step_db"), &one(num(0.5)));
    assert_eq!(capability("radio.rx.c1.gain_step_db"), &one(num(0.0)));
    // A stream's rate grid is the DDC's, descending in the capability.
    let CapabilityValue::AnyOf { values } = capability("radio.rx.s0.sample_rate_hz") else {
        panic!("a rate grid is AnyOf")
    };
    let rates = x3xx::rates(200_000_000);
    assert_eq!(values.len(), rates.len());
    assert_eq!(values[0], num(200_000_000.0));
    assert_eq!(values[values.len() - 1], num(200_000_000.0 / 2040.0));
    assert_eq!(
        capability("radio.tx.s1.sample_rate_hz"),
        capability("radio.rx.s0.sample_rate_hz")
    );

    // The other two duplex boards keep their own rows: the OBX reaches 8.4 GHz, and the
    // CBX starts at 1.2 GHz, redraws its phase on every tune and defaults to 2.45 GHz.
    let obx = description_of(&["0:A:obx:0"], &[]);
    assert_eq!(obx.rx[0].frequency.min(), 10_000_000.0);
    assert_eq!(obx.rx[0].frequency.max(), 8_400_000_000.0);
    assert_eq!(obx.rx[0].default_frequency_hz, 1_000_000_000.0);
    let cbx = description_of(&["0:A:cbx:0"], &[]);
    assert_eq!(cbx.rx[0].frequency.min(), 1_200_000_000.0);
    assert_eq!(cbx.rx[0].frequency.max(), 6_000_000_000.0);
    assert_eq!(cbx.rx[0].phase_behavior_on_retune, "random");
    assert_eq!(cbx.rx[0].default_frequency_hz, 2_450_000_000.0);
}

#[test]
fn rm_29_the_ideal_layout_builds_its_tree_and_defaults() {
    let layout = channels::ideal_from_selector(&selector(&[
        ("rx", Value::from(3_i64)),
        ("tx", Value::from(1_i64)),
    ]))
    .unwrap();
    let description = Description::new(&layout, 2_000, &terms());
    let id = ResourceId::parse("dev").unwrap();
    let tree = description.tree(&id, MemoryDomainId::local(7));

    let children: Vec<&str> = tree.children.iter().map(|child| child.id.path()).collect();
    assert_eq!(children, ["dev/rx/s0", "dev/rx/s1", "dev/rx/s2", "dev/tx/s0"]);
    let ports: Vec<&str> = tree.ports.iter().map(|port| port.name.as_str()).collect();
    assert_eq!(ports, ["rx0", "rx1", "rx2"]);
    assert_eq!(description.profile.name, "ideal");
    let capability = |name: &str| tree.capabilities.get(&key(name)).unwrap_or_else(|| panic!("{name}"));
    // No grid, and every integer hertz.
    assert_eq!(
        capability("radio.rx.c0.frequency_hz"),
        &CapabilityValue::Range { min: num(0.0), max: num(1_000_000_000_000.0) }
    );
    assert_eq!(
        capability("radio.rx.s0.sample_rate_hz"),
        &CapabilityValue::Range { min: Scalar::Int(1), max: Scalar::Int(1_000_000_000) }
    );
    assert_eq!(
        capability("radio.rx.c0.phase_behavior_on_retune"),
        &CapabilityValue::One { value: Scalar::from("deterministic") }
    );
    assert_eq!(capability("radio.rx.c0.path_delay_samples"), &CapabilityValue::One { value: Scalar::Int(0) });
    assert_eq!(capability("radio.rx.c0.frequency_step_hz"), &CapabilityValue::One { value: num(0.0) });
    assert_eq!(capability("radio.rx.c0.gain_step_db"), &CapabilityValue::One { value: num(0.0) });
    // Its timing and throughput are the `ideal` profile's values (RM-29).
    assert_eq!(description.timing.min_timed_command_lead_ns, 0);
    assert_eq!(description.timing.startup_latency_ns, 0);
    assert_eq!(description.timing.command_queue_depth, i64::from(u32::MAX));
    assert_eq!(description.timing.rx_start_lead_ns, 0);
    assert_eq!(description.timing.tx_start_lead_ns, 0);
    assert_eq!(description.performance.rx_bytes_per_s, 1_i64 << 62);
    assert_eq!(description.performance.tx_bytes_per_s, 1_i64 << 62);
    assert_eq!(description.performance.wire_bytes_per_sample, 8);

    // RM-30's defaults for every configuration key.
    let report = description.coerce(&id, &present_all(&description)).unwrap();
    assert!(report.rejected.is_empty(), "{:?}", report.rejected);
    assert_eq!(report.applied.len(), configuration_keys(&description).len());
    assert_eq!(value_of(&report, "radio.rx.c0.stream"), Value::from(0_i64));
    assert_eq!(value_of(&report, "radio.rx.c1.stream"), Value::from(-1_i64));
    assert_eq!(value_of(&report, "radio.rx.c2.stream"), Value::from(-1_i64));
    assert_eq!(value_of(&report, "radio.tx.c0.stream"), Value::from(-1_i64));
    assert_eq!(value_of(&report, "radio.rx.c0.frequency_hz"), Value::from(num(1_000_000_000.0)));
    assert_eq!(value_of(&report, "radio.rx.c0.antenna"), Value::from("RX2"));
    assert_eq!(value_of(&report, "radio.tx.c0.antenna"), Value::from("TX/RX"));
    for name in [
        "radio.rx.s0.sample_rate_hz",
        "radio.rx.s1.sample_rate_hz",
        "radio.rx.s2.sample_rate_hz",
        "radio.tx.s0.sample_rate_hz",
    ] {
        assert_eq!(value_of(&report, name), Value::from(num(1_000_000.0)), "{name}");
    }
}

#[test]
fn rm_30_coerce_cases() {
    let description = owner_description();
    let id = ResourceId::parse("dev").unwrap();
    let coerce = |pairs: &[(&str, Constraint)]| description.coerce(&id, &request(pairs)).unwrap();

    // RM-30's defaults: exactly the layout's configuration keys, receive `c0` in
    // stream 0 and every other channel in none.
    let defaults_report = description.coerce(&id, &present_all(&description)).unwrap();
    assert!(defaults_report.rejected.is_empty(), "{:?}", defaults_report.rejected);
    assert_eq!(defaults_report.applied.len(), configuration_keys(&description).len());
    assert_eq!(value_of(&defaults_report, "radio.rx.c0.stream"), Value::from(0_i64));
    assert_eq!(value_of(&defaults_report, "radio.rx.c1.stream"), Value::from(-1_i64));
    assert_eq!(value_of(&defaults_report, "radio.rx.c2.stream"), Value::from(-1_i64));
    assert_eq!(value_of(&defaults_report, "radio.tx.c0.stream"), Value::from(-1_i64));
    assert_eq!(value_of(&defaults_report, "radio.tx.c1.stream"), Value::from(-1_i64));
    assert_eq!(
        value_of(&defaults_report, "radio.rx.c0.frequency_hz"),
        Value::from(num(1_000_000_000.0))
    );
    assert_eq!(value_of(&defaults_report, "radio.rx.c1.frequency_hz"), Value::from(num(0.0)));
    assert_eq!(value_of(&defaults_report, "radio.tx.c1.frequency_hz"), Value::from(num(0.0)));
    assert_eq!(value_of(&defaults_report, "radio.rx.c0.gain_db"), Value::from(num(0.0)));
    assert_eq!(value_of(&defaults_report, "radio.rx.c0.antenna"), Value::from("RX2"));
    assert_eq!(value_of(&defaults_report, "radio.rx.c1.antenna"), Value::from("AB"));
    assert_eq!(value_of(&defaults_report, "radio.tx.c1.antenna"), Value::from("AB"));
    assert_eq!(
        value_of(&defaults_report, "radio.rx.s0.sample_rate_hz"),
        Value::from(num(1_000_000.0))
    );
    assert!(coerce(&[]).applied.is_empty());

    // A number the layout has no channel or stream for is refused by name.
    let report = coerce(&[("radio.rx.c3.frequency_hz", eq_num(10_000_000.0))]);
    assert_eq!(rejection(&report, "radio.rx.c3.frequency_hz").reason, "RM-4: no such channel c3");
    let report = coerce(&[("radio.tx.c2.frequency_hz", eq_num(2_400_000_000.0))]);
    assert_eq!(rejection(&report, "radio.tx.c2.frequency_hz").reason, "RM-4: no such channel c2");
    let report = coerce(&[("radio.tx.s3.sample_rate_hz", eq_num(1_000_000.0))]);
    assert_eq!(rejection(&report, "radio.tx.s3.sample_rate_hz").reason, "RM-4: no such stream s3");

    // The BasicRX's own range: 10 MHz is inside 0…100 MHz, 200 MHz is outside it.
    let report = coerce(&[("radio.rx.c1.frequency_hz", eq_num(10_000_000.0))]);
    assert_eq!(value_of(&report, "radio.rx.c1.frequency_hz"), Value::from(num(10_000_000.0)));
    let report = coerce(&[("radio.rx.c1.frequency_hz", eq_num(200_000_000.0))]);
    assert!(rejection(&report, "radio.rx.c1.frequency_hz").reason.starts_with("RM-8: "));
    assert!(!report.applied.contains_key(&key("radio.rx.c1.frequency_hz")));

    // A BasicRX has no gain stage: 10 dB is refused, never rounded to 0.
    let report = coerce(&[("radio.rx.c1.gain_db", eq_num(10.0))]);
    assert!(rejection(&report, "radio.rx.c1.gain_db").reason.starts_with("RM-8: "));
    assert!(!report.applied.contains_key(&key("radio.rx.c1.gain_db")));
    assert!(report.coercions.iter().all(|coercion| coercion.key != key("radio.rx.c1.gain_db")));

    // The UBX rate grid: N = 258 and N = 2040 exactly, and N = 257 off the grid, where
    // RM-8 coerces and records the coercion: a Spec Run's default policy (`reject`)
    // refuses the request, a Session (SB-45's `warn`) or an explicit `warn` applies the
    // grid value with a warning, and an explicit `accept` applies it without one.
    let report = coerce(&[("radio.rx.s0.sample_rate_hz", eq_num(200_000_000.0 / 258.0))]);
    assert_eq!(
        value_of(&report, "radio.rx.s0.sample_rate_hz"),
        Value::from(num(200_000_000.0 / 258.0))
    );
    assert!(report.coercions.is_empty());
    let report = coerce(&[("radio.rx.s0.sample_rate_hz", eq_num(200_000_000.0 / 2040.0))]);
    assert_eq!(
        value_of(&report, "radio.rx.s0.sample_rate_hz"),
        Value::from(num(200_000_000.0 / 2040.0))
    );
    assert!(report.coercions.is_empty());
    let report = coerce(&[("radio.rx.s0.sample_rate_hz", eq_num(200_000_000.0 / 257.0))]);
    assert_eq!(report.coercions.len(), 1, "{:?}", report.coercions);
    assert_eq!(report.coercions[0].key, key("radio.rx.s0.sample_rate_hz"));
    assert_ne!(report.coercions[0].applied, Value::from(num(200_000_000.0 / 257.0)));

    // 19.5 Msps is nearest to 20 Msps on the X3xx DDC grid.
    let report = coerce(&[("radio.rx.s0.sample_rate_hz", eq_num(19_500_000.0))]);
    assert_eq!(value_of(&report, "radio.rx.s0.sample_rate_hz"), Value::from(num(20_000_000.0)));
    assert_eq!(report.coercions.len(), 1, "{:?}", report.coercions);
    assert_eq!(report.coercions[0].key, key("radio.rx.s0.sample_rate_hz"));
    assert_eq!(report.coercions[0].reason, "RM-8: nearest grid value");

    // An off-grid gain rounds up when the requested value is nearer the upper step.
    let report = coerce(&[("radio.tx.c0.gain_db", eq_num(31.3))]);
    assert_eq!(value_of(&report, "radio.tx.c0.gain_db"), Value::from(num(31.5)));
    assert_eq!(report.coercions.len(), 1, "{:?}", report.coercions);
    assert_eq!(report.coercions[0].key, key("radio.tx.c0.gain_db"));

    // A Set skips unsupported antennas and selects the first available member.
    let report = coerce(&[(
        "radio.rx.c0.antenna",
        Constraint::Set { values: vec![Scalar::from("LNA".to_owned()), Scalar::from("TX/RX".to_owned())] },
    )]);
    assert!(report.rejected.is_empty(), "{:?}", report.rejected);
    assert_eq!(value_of(&report, "radio.rx.c0.antenna"), Value::from("TX/RX"));
    assert!(report.coercions.is_empty());

    // Membership: an integer in -1…=channel count − 1, and a BasicRX antenna's names.
    let report = coerce(&[("radio.rx.c2.stream", eq_int(-1))]);
    assert_eq!(value_of(&report, "radio.rx.c2.stream"), Value::from(-1_i64));
    let report = coerce(&[("radio.rx.c2.stream", eq_int(2))]);
    assert_eq!(value_of(&report, "radio.rx.c2.stream"), Value::from(2_i64));
    let report = coerce(&[("radio.rx.c2.stream", eq_int(3))]);
    assert!(rejection(&report, "radio.rx.c2.stream").reason.starts_with("RM-8: "));
    let report = coerce(&[("radio.rx.c1.antenna", eq_str("A"))]);
    assert_eq!(value_of(&report, "radio.rx.c1.antenna"), Value::from("A"));
    let report = coerce(&[("radio.rx.c1.antenna", eq_str("RX2"))]);
    assert!(rejection(&report, "radio.rx.c1.antenna").reason.starts_with("RM-8: "));

    // A capability-only key is matched against the channel's own capability.
    let report = coerce(&[("radio.tx.c0.path_delay_samples", eq_int(45))]);
    assert_eq!(value_of(&report, "radio.tx.c0.path_delay_samples"), Value::from(45_i64));
    let report = coerce(&[("radio.tx.c0.path_delay_samples", eq_int(46))]);
    assert!(rejection(&report, "radio.tx.c0.path_delay_samples").reason.starts_with("RM-8: "));

    // One instance takes the layout builder's keys only: RM-4's configuration keys,
    // both directions, are not radio keys here.
    for direction in ["rx", "tx"] {
        for leaf in ["channels", "sample_rate_hz", "frequency_hz", "gain_db", "antenna"] {
            let name = format!("radio.{direction}.{leaf}");
            let report = coerce(&[(name.as_str(), Constraint::Present {})]);
            assert_eq!(
                rejection(&report, &name).reason,
                "RM-4: not a radio key",
                "{name}"
            );
        }
    }
    // Its other keys are no radio keys either, and neither is an unknown one.
    for name in [
        "radio.rx.step_hz",
        "radio.rx.coherent",
        "radio.rx.path_delay_samples",
        "radio.rx.phase_behavior_on_retune",
    ] {
        let report = coerce(&[(name, Constraint::Present {})]);
        assert_eq!(rejection(&report, name).reason, "RM-4: not a radio key", "{name}");
    }
    let report = coerce(&[("radio.rx.bogus", eq_int(1))]);
    assert_eq!(rejection(&report, "radio.rx.bogus").reason, "RM-4: not a radio key");
}

#[test]
fn rm_31_full_duplex_cases() {
    // A UBX in slot A and a BasicRX in slot B: only the UBX carries both directions.
    let description = description_of(&["0:A:ubx:0", "0:B:basic_rx:0"], &["0:A:ubx:0"]);
    let id = ResourceId::parse("dev").unwrap();
    let coerce = |pairs: &[(&str, Constraint)]| description.coerce(&id, &request(pairs)).unwrap();

    // While the UBX's transmit channel is in a stream, its receive antenna is RX2.
    let report = coerce(&[
        ("radio.tx.c0.stream", eq_int(0)),
        ("radio.rx.c0.antenna", eq_str("TX/RX")),
    ]);
    let refusal = rejection(&report, "radio.rx.c0.antenna");
    assert!(refusal.reason.contains("RM-31"), "{}", refusal.reason);
    assert!(refusal.reason.contains("ubx") && refusal.reason.contains("c0"), "{}", refusal.reason);
    assert!(!report.applied.contains_key(&key("radio.rx.c0.antenna")));
    assert!(coerce(&[
        ("radio.tx.c0.stream", eq_int(0)),
        ("radio.rx.c0.antenna", eq_str("RX2")),
    ])
    .rejected
    .is_empty());
    // No transmit channel in a stream: TX/RX is accepted, and a request that names only
    // the membership meets the default RX2.
    assert!(coerce(&[
        ("radio.tx.c0.stream", eq_int(-1)),
        ("radio.rx.c0.antenna", eq_str("TX/RX")),
    ])
    .rejected
    .is_empty());
    assert!(coerce(&[("radio.tx.c0.stream", eq_int(0))]).rejected.is_empty());

    // A different slot is a different board: no constraint.
    let other = description_of(&["0:A:ubx:0"], &["0:B:ubx:0"]);
    let report = other
        .coerce(
            &id,
            &request(&[
                ("radio.tx.c0.stream", eq_int(0)),
                ("radio.rx.c0.antenna", eq_str("TX/RX")),
            ]),
        )
        .unwrap();
    assert!(report.rejected.is_empty(), "{:?}", report.rejected);

    // Basic/LF boards carry one direction each: their antennas are unconstrained.
    let mixed = description_of(&["0:B:basic_rx:0"], &["0:B:lf_tx:0"]);
    let report = mixed
        .coerce(
            &id,
            &request(&[
                ("radio.tx.c0.stream", eq_int(0)),
                ("radio.rx.c0.antenna", eq_str("A")),
            ]),
        )
        .unwrap();
    assert!(report.rejected.is_empty(), "{:?}", report.rejected);

    // RM-31's judgement of a whole configuration needs no request: a transmit channel in
    // a stream beside a receive channel of the same board on another antenna is
    // inadmissible on its own, and so is the one-key update that lays `TX/RX` over a
    // configuration whose transmit channel already is in a stream.
    let both = configuration(
        &description,
        &[
            ("radio.tx.c0.stream", Value::from(0_i64)),
            ("radio.rx.c0.antenna", Value::from("TX/RX")),
        ],
    );
    let violations = description.violations(&both);
    assert_eq!(violations.len(), 1, "{violations:?}");
    assert_eq!(violations[0].0, key("radio.rx.c0.antenna"));
    assert!(
        violations[0].1.contains("RM-31") && violations[0].1.contains("ubx"),
        "{}",
        violations[0].1
    );
    let held = configuration(&description, &[("radio.tx.c0.stream", Value::from(0_i64))]);
    let mut updated = held.clone();
    updated.insert(key("radio.rx.c0.antenna"), Value::from("TX/RX"));
    assert_eq!(description.violations(&updated), violations);
    // The same configuration with `RX2`, and one whose transmit channel is in no stream,
    // are admissible.
    assert!(description
        .violations(&configuration(
            &description,
            &[
                ("radio.tx.c0.stream", Value::from(0_i64)),
                ("radio.rx.c0.antenna", Value::from("RX2")),
            ],
        ))
        .is_empty());
    assert!(description.violations(&held).is_empty());
}

#[test]
fn rm_32_throughput_cases() {
    // Two 10 GbE links do not raise the limit: one link's bytes per second is the cap.
    let description = description_of(&["0:A:ubx:0", "0:B:ubx:0"], &[]);
    let id = ResourceId::parse("dev").unwrap();
    let coerce = |pairs: &[(&str, Constraint)]| description.coerce(&id, &request(pairs)).unwrap();

    // One channel at 200 Msps is 800 MB/s on a 1 GB/s link.
    let report = coerce(&[
        ("radio.rx.c0.stream", eq_int(0)),
        ("radio.rx.s0.sample_rate_hz", eq_num(200_000_000.0)),
    ]);
    assert!(report.rejected.is_empty(), "{:?}", report.rejected);
    // Two channels on the same stream are 1.6 GB/s: refused on the rate key, even
    // though the selector names two links.
    let report = coerce(&[
        ("radio.rx.c0.stream", eq_int(0)),
        ("radio.rx.c1.stream", eq_int(0)),
        ("radio.rx.s0.sample_rate_hz", eq_num(200_000_000.0)),
    ]);
    let refusal = rejection(&report, "radio.rx.s0.sample_rate_hz");
    assert!(refusal.reason.starts_with("RM-32: "), "{}", refusal.reason);
    assert!(!report.applied.contains_key(&key("radio.rx.s0.sample_rate_hz")));
    // A 1 GbE link carries 100 MB/s: one channel at 200 Msps is refused.
    let mut slow = description_of(&["0:A:ubx:0", "0:B:ubx:0"], &[]);
    slow.performance.rx_bytes_per_s = 100_000_000;
    let report = slow
        .coerce(
            &id,
            &request(&[
                ("radio.rx.c0.stream", eq_int(0)),
                ("radio.rx.s0.sample_rate_hz", eq_num(200_000_000.0)),
            ]),
        )
        .unwrap();
    assert!(rejection(&report, "radio.rx.s0.sample_rate_hz")
        .reason
        .starts_with("RM-32: "));
    // The violation's own key goes first: only s1 holds a channel, and a request that
    // names both rates is refused on s1, not on the lower-numbered s0.
    let report = slow
        .coerce(
            &id,
            &request(&[
                ("radio.rx.c0.stream", eq_int(-1)),
                ("radio.rx.c1.stream", eq_int(1)),
                ("radio.rx.s0.sample_rate_hz", eq_num(20_000_000.0)),
                ("radio.rx.s1.sample_rate_hz", eq_num(200_000_000.0)),
            ]),
        )
        .unwrap();
    assert!(rejection(&report, "radio.rx.s1.sample_rate_hz").reason.starts_with("RM-32: "));
    assert!(report.rejected.iter().all(|refusal| refusal.key != key("radio.rx.s0.sample_rate_hz")));
    // The same link admits 20 Msps on one channel: 80 MB/s.
    assert!(slow
        .coerce(
            &id,
            &request(&[
                ("radio.rx.c0.stream", eq_int(0)),
                ("radio.rx.s0.sample_rate_hz", eq_num(20_000_000.0)),
            ]),
        )
        .unwrap()
        .rejected
        .is_empty());
    // A second channel at 20 Msps takes 160 MB/s: refused.
    let report = slow
        .coerce(
            &id,
            &request(&[
                ("radio.rx.c0.stream", eq_int(0)),
                ("radio.rx.c1.stream", eq_int(0)),
                ("radio.rx.s0.sample_rate_hz", eq_num(20_000_000.0)),
            ]),
        )
        .unwrap();
    assert!(report.rejected.iter().any(|r| r.key == key("radio.rx.s0.sample_rate_hz")));

    // The sum over the streams decides, not each stream on its own: two streams of
    // 200 Msps are 1.6 GB/s against the one link the limit is.
    let report = coerce(&[
        ("radio.rx.c0.stream", eq_int(0)),
        ("radio.rx.s0.sample_rate_hz", eq_num(200_000_000.0)),
        ("radio.rx.c1.stream", eq_int(1)),
        ("radio.rx.s1.sample_rate_hz", eq_num(200_000_000.0)),
    ]);
    let refusal = rejection(&report, "radio.rx.s0.sample_rate_hz");
    assert!(refusal.reason.starts_with("RM-32: rx needs"), "{}", refusal.reason);
    // Rates of two different streams add up too: 200 + 100 Msps is over the 250 Msps
    // that 1 GB/s at 4 wire bytes a sample carries, while 200 + 50 is exactly at it.
    let report = coerce(&[
        ("radio.rx.c0.stream", eq_int(0)),
        ("radio.rx.s0.sample_rate_hz", eq_num(200_000_000.0)),
        ("radio.rx.c1.stream", eq_int(1)),
        ("radio.rx.s1.sample_rate_hz", eq_num(100_000_000.0)),
    ]);
    assert!(rejection(&report, "radio.rx.s0.sample_rate_hz").reason.starts_with("RM-32: "));
    let report = coerce(&[
        ("radio.rx.c0.stream", eq_int(0)),
        ("radio.rx.s0.sample_rate_hz", eq_num(200_000_000.0)),
        ("radio.rx.c1.stream", eq_int(1)),
        ("radio.rx.s1.sample_rate_hz", eq_num(50_000_000.0)),
    ]);
    assert!(report.rejected.is_empty(), "{:?}", report.rejected);
    // The transmit direction is judged the same way.
    let transmitting = description_of(&[], &["0:A:ubx:0", "0:B:ubx:0"]);
    let report = transmitting
        .coerce(
            &id,
            &request(&[
                ("radio.tx.c0.stream", eq_int(0)),
                ("radio.tx.s0.sample_rate_hz", eq_num(200_000_000.0)),
                ("radio.tx.c1.stream", eq_int(1)),
                ("radio.tx.s1.sample_rate_hz", eq_num(200_000_000.0)),
            ]),
        )
        .unwrap();
    assert!(rejection(&report, "radio.tx.s0.sample_rate_hz").reason.starts_with("RM-32: "));

    // RM-32's judgement of a whole configuration needs no request, and names the
    // direction's first stream that holds a channel.
    let over = configuration(
        &description,
        &[
            ("radio.rx.c0.stream", Value::from(0_i64)),
            ("radio.rx.s0.sample_rate_hz", Value::from(num(200_000_000.0))),
            ("radio.rx.c1.stream", Value::from(1_i64)),
            ("radio.rx.s1.sample_rate_hz", Value::from(num(200_000_000.0))),
        ],
    );
    let violations = description.violations(&over);
    assert_eq!(violations.len(), 1, "{violations:?}");
    assert_eq!(violations[0].0, key("radio.rx.s0.sample_rate_hz"));
    assert!(violations[0].1.starts_with("RM-32: rx needs"), "{}", violations[0].1);
    assert!(description
        .violations(&configuration(
            &description,
            &[
                ("radio.rx.c0.stream", Value::from(0_i64)),
                ("radio.rx.s0.sample_rate_hz", Value::from(num(200_000_000.0))),
                ("radio.rx.c1.stream", Value::from(1_i64)),
                ("radio.rx.s1.sample_rate_hz", Value::from(num(50_000_000.0))),
            ],
        ))
        .is_empty());

    // A change that names only a channel's membership is refused when the configuration
    // it joins is over the link: the rate is not in the request.
    let mut pinched = description_of(&["0:A:ubx:0", "0:B:ubx:0"], &[]);
    pinched.performance.rx_bytes_per_s = 4;
    let report = pinched
        .coerce(&id, &request(&[("radio.rx.c1.stream", eq_int(0))]))
        .unwrap();
    assert!(rejection(&report, "radio.rx.c1.stream").reason.starts_with("RM-32: "));
    // A rate key comes before a membership key: the request's own rate key is the one
    // refused, though the violation's stream is another.
    let report = pinched
        .coerce(
            &id,
            &request(&[
                ("radio.rx.c1.stream", eq_int(0)),
                ("radio.rx.s1.sample_rate_hz", eq_num(1_000_000.0)),
            ]),
        )
        .unwrap();
    assert!(rejection(&report, "radio.rx.s1.sample_rate_hz").reason.starts_with("RM-32: "));
    assert!(!report.applied.contains_key(&key("radio.rx.s1.sample_rate_hz")));
    // A violation no requested key carries is refused under its own key, not dropped.
    let report = pinched
        .coerce(&id, &request(&[("radio.rx.c0.frequency_hz", eq_num(1_000_000_000.0))]))
        .unwrap();
    assert!(rejection(&report, "radio.rx.s0.sample_rate_hz").reason.starts_with("RM-32: "));
}

/// RM-33's check over one fragment's configuration.
fn check(values: &[(String, Value)], envelope: JsonValue) -> Vec<Violation> {
    let effective = BTreeMap::from([(
        ident("radio"),
        values
            .iter()
            .map(|(name, value)| (key(name), value.clone()))
            .collect(),
    )]);
    let mut checks = AdmissionCheckRegistry::new();
    checks.register(Arc::new(RfEnvelopeCheck));
    checks.run(
        &BTreeMap::from([(Namespace::parse("radio.rf_envelope").unwrap(), envelope)]),
        &effective,
        &BTreeMap::new(),
        CheckStage::Validate,
    )
}

fn setting(direction: &str, number: u32, leaf: &str, value: Value) -> (String, Value) {
    (keys::channel(direction, number, leaf), value)
}

fn violation_keys(violations: &[Violation]) -> Vec<&str> {
    violations
        .iter()
        .map(|violation| violation.key.as_ref().unwrap().as_str())
        .collect()
}

#[test]
fn rm_33_rm_19_per_channel_cases() {
    let envelope = json!({
        "allowed_bands": [{"lo_hz": 2_400_000_000.0, "hi_hz": 2_500_000_000.0}],
        "max_gain_db": 20.0,
        "tx_enabled": [true, false],
        "antenna_ports": ["TX/RX", "RX2", "A", "AB"]
    });

    // `tx_enabled`'s index is the logical transmit channel: c1 in a stream alone is
    // channel 1, which `[true, false]` disables, even though it is its stream's first
    // channel.
    let values = vec![
        setting("tx", 0, "frequency_hz", Value::from(num(2_600_000_000.0))),
        setting("tx", 0, "gain_db", Value::from(num(10.0))),
        setting("tx", 0, "antenna", Value::from("TX/RX")),
        setting("tx", 0, "stream", Value::from(-1_i64)),
        setting("tx", 1, "frequency_hz", Value::from(num(2_450_000_000.0))),
        setting("tx", 1, "gain_db", Value::from(num(10.0))),
        setting("tx", 1, "antenna", Value::from("TX/RX")),
        setting("tx", 1, "stream", Value::from(0_i64)),
    ];
    let violations = check(&values, envelope.clone());
    assert_eq!(violation_keys(&violations), ["radio.tx.c1.stream"], "{violations:?}");
    assert_eq!(
        violations[0].reason,
        "RM-19: radio: transmit channel c1 is not enabled"
    );
    // `[false, true]` enables c1 and says nothing about the idle c0.
    let mut enabled = envelope.clone();
    enabled["tx_enabled"] = json!([false, true]);
    assert!(check(&values, enabled).is_empty());

    // Every channel in a stream is judged on its own frequency, gain and antenna.
    let violations = check(
        &[
            setting("tx", 0, "frequency_hz", Value::from(num(2_600_000_000.0))),
            setting("tx", 0, "gain_db", Value::from(num(25.0))),
            setting("tx", 0, "antenna", Value::from("J1")),
            setting("tx", 0, "stream", Value::from(0_i64)),
        ],
        envelope.clone(),
    );
    assert_eq!(
        violation_keys(&violations),
        ["radio.tx.c0.frequency_hz", "radio.tx.c0.gain_db", "radio.tx.c0.antenna"],
        "{violations:?}"
    );
    assert!(violations[0].reason.starts_with("RM-19: radio: "));

    // A channel in no stream is not judged.
    let violations = check(
        &[
            setting("tx", 0, "frequency_hz", Value::from(num(2_600_000_000.0))),
            setting("tx", 0, "stream", Value::from(-1_i64)),
        ],
        envelope.clone(),
    );
    assert!(violations.is_empty(), "{violations:?}");

    // Receive antennas are judged per channel, only while the channel is in a stream.
    let mut rx = vec![
        setting("rx", 0, "antenna", Value::from("RX2")),
        setting("rx", 0, "stream", Value::from(0_i64)),
        setting("rx", 1, "antenna", Value::from("J1")),
        setting("rx", 1, "stream", Value::from(0_i64)),
    ];
    let violations = check(&rx, envelope.clone());
    assert_eq!(violation_keys(&violations), ["radio.rx.c1.antenna"], "{violations:?}");
    rx[3].1 = Value::from(-1_i64);
    assert!(check(&rx, envelope.clone()).is_empty());

    // A channel value of another kind is its own violation.
    let violations = check(
        &[
            setting("tx", 0, "frequency_hz", Value::from("2.45 GHz")),
            setting("tx", 0, "stream", Value::from(0_i64)),
        ],
        json!({"allowed_bands": [{"lo_hz": 2_400_000_000.0, "hi_hz": 2_500_000_000.0}]}),
    );
    assert_eq!(violation_keys(&violations), ["radio.tx.c0.frequency_hz"]);
}
