//! RM-26: `ezsdr_radio::device` against the `x3xx` values (Phase 7, VE-1; design 33 D5).

use ezsdr_kernel::spec::Scalar;
use std::collections::BTreeMap;

use ezsdr_kernel::id::ResourceId;
use ezsdr_kernel::module_api::{ProfileRef, Requested, Version};
use ezsdr_kernel::spec::{CapabilityValue, Constraint, Key, Value};
use ezsdr_radio::device::{DeviceDescription, Grid};
use ezsdr_radio::x3xx::{self, Board, ProviderTerms, Selection, Slot};
use ezsdr_radio::keys;

fn key(name: &str) -> Key {
    Key::parse(name).unwrap()
}

/// The `x3xx` decimations, written out here from the design's rule rather than read from
/// the crate under test (design 33 D5).
fn decimations() -> Vec<u64> {
    let mut set = vec![1u64];
    for h in 0..=3u64 {
        for c in 1..=255u64 {
            set.push((1 << h) * c);
        }
    }
    set.sort_unstable();
    set.dedup();
    set
}

/// The `x3xx` description of two UBX, both directions, with the Mock's own terms.
fn x3xx_description() -> DeviceDescription {
    let terms = ProviderTerms {
        min_timed_command_lead_ns: 2_000_000,
        repeat_max_samples: 268_435_456,
        repeat_align_samples: 2,
    };
    x3xx::description(&Selection::of(Board::Ubx, &[Slot::A, Slot::B], &[Slot::A, Slot::B]), 2_000, &terms)
}

/// The same values, written out here independently of the crate under test (RM-4).
fn capabilities() -> BTreeMap<Key, CapabilityValue> {
    let range = |min, max| CapabilityValue::Range { min, max };
    let one = |value| CapabilityValue::One { value };
    let mut expected = BTreeMap::new();
    for direction in ["rx", "tx"] {
        expected.insert(key(&format!("radio.{direction}.channels")), range(Scalar::from(0), Scalar::from(2)));
        expected.insert(
            key(&format!("radio.{direction}.sample_rate_hz")),
            CapabilityValue::AnyOf {
                values: decimations().iter().map(|n| Scalar::try_from(200_000_000.0 / *n as f64).unwrap()).collect(),
            },
        );
        expected.insert(
            key(&format!("radio.{direction}.frequency_hz")),
            range(Scalar::try_from(10_000_000.0).unwrap(), Scalar::try_from(6_000_000_000.0).unwrap()),
        );
        expected.insert(key(&format!("radio.{direction}.gain_db")), range(Scalar::try_from(0.0).unwrap(), Scalar::try_from(31.5).unwrap()));
        expected.insert(key(&format!("radio.{direction}.frequency_step_hz")), one(Scalar::try_from(1.0).unwrap()));
        expected.insert(key(&format!("radio.{direction}.gain_step_db")), one(Scalar::try_from(0.5).unwrap()));
    }
    let names = |names: &[&str]| CapabilityValue::AnyOf {
        values: names.iter().map(|name| Scalar::from(*name)).collect(),
    };
    expected.insert(key(keys::RX_ANTENNA), names(&["RX2", "TX/RX"]));
    expected.insert(key(keys::TX_ANTENNA), names(&["TX/RX"]));
    for (name, value) in [
        (keys::RX_COHERENT, Scalar::from(true)),
        (keys::FULL_DUPLEX, Scalar::from(true)),
        (keys::HARDWARE_TIME, Scalar::from(true)),
        (keys::PHASE_BEHAVIOR_ON_RETUNE, Scalar::from("random_unless_timed_tune".to_owned())),
        (keys::TX_REPEAT_MAX_SAMPLES, Scalar::from(268_435_456)),
        (keys::TX_REPEAT_ALIGN_SAMPLES, Scalar::from(2)),
        (keys::RX_BLOCK_LEN, Scalar::from(2_000)),
        (keys::MIN_TIMED_COMMAND_LEAD_NS, Scalar::from(2_000_000)),
        (keys::STARTUP_LATENCY_NS, Scalar::from(2_000_000_000)),
        (keys::COMMAND_QUEUE_DEPTH, Scalar::from(16)),
        (keys::OVERFLOW_RESTART_GAP_NS, Scalar::from(50_000_000)),
        (keys::RX_START_LEAD_NS, Scalar::from(50_000_000)),
        (keys::TX_START_LEAD_NS, Scalar::from(50_000_000)),
        (keys::RX_BYTES_PER_S, Scalar::from(1_000_000_000)),
        (keys::TX_BYTES_PER_S, Scalar::from(1_000_000_000)),
        (keys::WIRE_BYTES_PER_SAMPLE, Scalar::from(4)),
        (keys::TX_PATH_DELAY_SAMPLES, Scalar::from(45)),
        (keys::RX_PATH_DELAY_SAMPLES, Scalar::from(0)),
    ] {
        expected.insert(key(name), one(value));
    }
    expected
}

#[test]
fn rm_26_the_description_builds_the_tree() {
    let id = ResourceId::parse("dev").unwrap();
    let domain = ezsdr_kernel::id::MemoryDomainId::local(37);
    let tree = x3xx_description().tree(&id, domain);
    assert_eq!(tree.id, id);
    assert_eq!(tree.kind.as_str(), "radio.device");
    assert_eq!(tree.capabilities, capabilities());
    let children: Vec<(ResourceId, String)> = tree
        .children
        .iter()
        .map(|child| (child.id.clone(), child.kind.to_string()))
        .collect();
    assert_eq!(
        children,
        [
            (ResourceId::parse("dev/rx").unwrap(), "radio.rx_stream".to_owned()),
            (ResourceId::parse("dev/tx").unwrap(), "radio.tx_stream".to_owned())
        ]
    );
    assert!(tree.children.iter().all(|child| child.capabilities.is_empty() && child.ports.is_empty() && !child.shareable));
    assert_eq!(tree.ports.len(), 1);
    assert_eq!((tree.ports[0].name.as_str(), tree.ports[0].contract.as_str()), ("rx", "ezsdr.stream.cf32"));
    assert_eq!(tree.memory_domains, [domain]);
    assert!(tree.children.iter().all(|child| child.memory_domains.is_empty()));
    assert!(!tree.shareable);
}

#[test]
fn rm_26_the_description_is_the_one_profile_both_providers_name() {
    // Design 33 D5: the profile is `x3xx` 0.1.0, and a `ProviderTerms` value reaches the
    // description, so a description says whose value each Module sharing it contributed.
    let description = x3xx_description();
    assert_eq!(description.profile, ProfileRef { name: "x3xx".to_owned(), version: Version::new(0, 1, 0) });
    assert_eq!(x3xx::profile_ref(), description.profile);
    assert_eq!(description.rx_max_channels, 2);
    assert_eq!(description.tx_max_channels, 2);
    let terms = ProviderTerms {
        min_timed_command_lead_ns: 5_000_000,
        repeat_max_samples: u64::from(u32::MAX),
        repeat_align_samples: 1,
    };
    let uhd = x3xx::description(&Selection::of(Board::Ubx, &[Slot::A], &[]), 2_000, &terms);
    assert_eq!(uhd.timing.min_timed_command_lead_ns, 5_000_000);
    assert_eq!(uhd.repeat_max_samples, u64::from(u32::MAX));
    assert_eq!((uhd.rx_max_channels, uhd.tx_max_channels), (1, 0));
    // The boards' own values: a CBX has no timed phase sync, so its channels are aligned
    // but not coherent, and its default is 2.45 GHz (RM-9, design 33 D13).
    let cbx = x3xx::description(&Selection::of(Board::Cbx, &[Slot::A], &[Slot::A]), 2_000, &terms);
    assert_eq!(cbx.phase_behavior_on_retune, "random");
    assert!(!cbx.coherent);
    assert_eq!(cbx.frequency.min(), 1_200_000_000.0);
    assert_eq!(cbx.defaults[&key(keys::RX_FREQUENCY_HZ)], Value::num(2_450_000_000.0).unwrap());
    // A layout with no receive front end defaults to no receive channel (RM-5, design 33 D5).
    let tx_only = x3xx::description(&Selection::of(Board::Cbx, &[], &[Slot::A]), 2_000, &terms);
    assert_eq!((tx_only.rx_max_channels, tx_only.defaults[&key(keys::RX_CHANNELS)].clone()), (0, Value::from(0)));
    // The link's speed decides the transport: 1 GbE is a tenth of 10 GbE.
    let mut slow = Selection::of(Board::Ubx, &[Slot::A], &[Slot::A]);
    slow.link_gbps = 1;
    let slow = x3xx::description(&slow, 2_000, &terms);
    assert_eq!((slow.performance.rx_bytes_per_s, slow.performance.tx_bytes_per_s), (100_000_000, 100_000_000));
}

#[test]
fn rm_26_the_decimation_set_is_the_ddc_s() {
    // 639 values; `{1}` is in it, the CIC's 255 and three halfbands' 2040 are, and one
    // past the halfbands (4080) and an odd CIC value above 255 (257) are not (design 33 D5).
    let set = x3xx::decimations();
    assert_eq!(set.len(), 639);
    assert_eq!(set[0], 1);
    assert!(x3xx::is_decimation(1) && x3xx::is_decimation(255) && x3xx::is_decimation(256) && x3xx::is_decimation(2040));
    assert!(!x3xx::is_decimation(257) && !x3xx::is_decimation(2041) && !x3xx::is_decimation(4080));
    // The rate grid is ascending and exactly `200e6 / N` (RM-8's `Values`).
    let rates = x3xx::rates(200_000_000);
    assert_eq!(rates.len(), 639);
    assert_eq!(rates[0], 200_000_000.0 / 2040.0);
    assert_eq!(*rates.last().unwrap(), 200_000_000.0);
    assert!(rates.windows(2).all(|pair| pair[0] < pair[1]));
    // `decimation` is exact and refuses what the DDC cannot make; `clip_decimation` is
    // UHD's `coerce_decim` (UR-12, UR-33).
    assert_eq!(x3xx::decimation(200_000_000, 200e6 / 3.0), Some(3));
    assert_eq!(x3xx::decimation(200_000_000, 19.5e6), None);
    assert_eq!(x3xx::decimation(200_000_000, 200e6 / 257.0), None);
    assert_eq!(x3xx::decimation(200_000_000, 200e6 / 2040.0), Some(2040));
    assert_eq!(x3xx::clip_decimation(200_000_000, 200e6 / 257.0), 256);
    assert_eq!(x3xx::clip_decimation(200_000_000, 200e6 / 257.4), 258);
    assert_eq!(x3xx::clip_decimation(200_000_000, 200e6 / 4080.0), 2040);
}

#[test]
fn rm_26_the_layout_parser_checks_the_shared_selector() {
    use std::collections::BTreeMap as Map;
    let selector = |rx: &[&str], tx: &[&str]| -> Map<ezsdr_kernel::spec::Ident, Value> {
        Map::from([
            (ezsdr_kernel::spec::Ident::parse("rx").unwrap(), Value::List(rx.iter().map(|e| Scalar::from(*e)).collect())),
            (ezsdr_kernel::spec::Ident::parse("tx").unwrap(), Value::List(tx.iter().map(|e| Scalar::from(*e)).collect())),
        ])
    };
    let ok = x3xx::from_selector(&selector(&["0:A:ubx:0", "0:B:ubx:0"], &["0:A:ubx:0"])).unwrap();
    assert_eq!((ok.board, ok.master_clock_hz, ok.image.name(), ok.links, ok.link_gbps), (Board::Ubx, 200_000_000, "hg", 1, 10));
    assert_eq!(ok.rx.len(), 2);
    assert_eq!(ok.tx.len(), 1);
    for bad in [
        selector(&["0:A:ubx"], &["0:A:ubx:0"]),
        selector(&["1:A:ubx:0"], &["0:A:ubx:0"]),
        selector(&["0:A:sbx:0"], &["0:A:ubx:0"]),
        selector(&["0:C:ubx:0"], &["0:A:ubx:0"]),
        selector(&["0:A:ubx:1"], &["0:A:ubx:0"]),
        selector(&["0:A:ubx:0", "0:A:ubx:0"], &["0:A:ubx:0"]),
        selector(&["0:A:ubx:0", "0:B:ubx:0", "0:A:ubx:0"], &["0:A:ubx:0"]),
        selector(&["0:A:ubx:0"], &["0:A:obx:0"]),
        selector(&[], &[]),
    ] {
        assert!(x3xx::from_selector(&bad).is_err(), "{bad:?}");
    }
    // RM-28, design 33 D5: any slot order is the channel order — the layout need not be a
    // prefix of UHD's default order —, and the order is what the element list says.
    let reversed = x3xx::from_selector(&selector(&["0:B:ubx:0", "0:A:ubx:0"], &["0:B:ubx:0"])).unwrap();
    assert_eq!((reversed.rx[0].slot, reversed.rx[1].slot), (Slot::B, Slot::A));
    assert_eq!(reversed.tx[0].slot, Slot::B);
    let one = x3xx::from_selector(&selector(&["0:B:ubx:0"], &["0:A:ubx:0"])).unwrap();
    assert_eq!(one.rx[0].slot, Slot::B);
    // The refusal names the position and the repeated pair (design 33 D5).
    let reason = x3xx::from_selector(&selector(&["0:A:ubx:0", "0:A:ubx:0"], &["0:A:ubx:0"])).unwrap_err();
    assert!(reason.contains("position 1") && reason.contains("repeats"), "{reason}");
}

fn request(pairs: &[(&str, Constraint)]) -> Requested {
    Requested {
        resource: ResourceId::parse("dev").unwrap(),
        constraints: pairs.iter().map(|(name, c)| (key(name), c.clone())).collect(),
    }
}

fn eq(value: Scalar) -> Constraint {
    Constraint::Eq { value }
}

#[test]
fn rm_26_coerce_cases() {
    let description = x3xx_description();
    let dev = ResourceId::parse("dev").unwrap();
    let coerce = |pairs: &[(&str, Constraint)]| description.coerce(&dev, &request(pairs)).unwrap();

    // 19.5 Msps snaps to 20 Msps with a coercion (RM-8).
    let report = coerce(&[(keys::RX_SAMPLE_RATE_HZ, eq(Scalar::try_from(19_500_000.0).unwrap()))]);
    assert_eq!(report.applied[&key(keys::RX_SAMPLE_RATE_HZ)], Value::num(20_000_000.0).unwrap());
    assert_eq!(report.coercions[0].reason, "RM-8: nearest grid value");

    // A tie goes to the lower grid value (RM-8): 30.25 dB lies between 30.0 and 30.5.
    let report = coerce(&[(keys::RX_GAIN_DB, eq(Scalar::try_from(30.25).unwrap()))]);
    assert_eq!(report.applied[&key(keys::RX_GAIN_DB)], Value::num(30.0).unwrap());

    // 7 GHz is outside the tuning range (RM-8).
    let report = coerce(&[(keys::RX_FREQUENCY_HZ, eq(Scalar::try_from(7.0e9).unwrap()))]);
    assert!(report.rejected[0].reason.starts_with("RM-8: "), "{:?}", report.rejected);

    // Gain 31.7 is outside 0..31.5 and refused; 31.3 snaps to 31.5.
    let report = coerce(&[(keys::TX_GAIN_DB, eq(Scalar::try_from(31.3).unwrap()))]);
    assert_eq!(report.applied[&key(keys::TX_GAIN_DB)], Value::num(31.5).unwrap());
    assert_eq!(report.coercions.len(), 1);

    // Two channels at 200 Msps need 1.6 GB/s on a 1 GB/s link: RM-7 names the rate key,
    // in either direction.
    for direction in ["rx", "tx"] {
        let channels = format!("radio.{direction}.channels");
        let rate = format!("radio.{direction}.sample_rate_hz");
        let report = coerce(&[
            (channels.as_str(), eq(Scalar::from(2))),
            (rate.as_str(), eq(Scalar::try_from(200_000_000.0).unwrap())),
        ]);
        let refusal = report.rejected.iter().find(|r| r.reason.starts_with("RM-7: ")).expect("RM-7 refuses");
        assert_eq!(refusal.key, key(&rate), "{direction}");
    }

    // Four channels exceed the device's two (RM-8), in each direction.
    for direction in ["rx", "tx"] {
        let report = coerce(&[(format!("radio.{direction}.channels").as_str(), eq(Scalar::from(4)))]);
        assert!(report.rejected[0].reason.starts_with("RM-8: "), "{:?}", report.rejected);
    }

    // A Set of antennas picks the first available one.
    let report = coerce(&[(
        keys::RX_ANTENNA,
        Constraint::Set { values: vec![Scalar::from("LNA".to_owned()), Scalar::from("TX/RX".to_owned())] },
    )]);
    assert_eq!(report.applied[&key(keys::RX_ANTENNA)], Value::from("TX/RX".to_owned()));

    // Present of every configuration key gives RM-5's defaults.
    let all: Vec<(&str, Constraint)> = keys::CONFIGURATION.iter().map(|name| (*name, Constraint::Present {})).collect();
    let report = coerce(&all);
    assert!(report.rejected.is_empty(), "{:?}", report.rejected);
    assert_eq!(report.applied, DeviceDescription::x310_defaults());

    // A key outside RM-4 is refused under RM-4.
    let report = coerce(&[("radio.rx.bogus", eq(Scalar::from(1)))]);
    assert_eq!(report.rejected[0].reason, "RM-4: not a radio key");

    // Whole hertz, when the description says so.
    let whole = DeviceDescription { whole_hertz_rates: true, rates: Grid::Integer { lo: 1, hi: 1_000_000_000 }, ..x3xx_description() };
    let report = whole.coerce(&dev, &request(&[(keys::RX_SAMPLE_RATE_HZ, eq(Scalar::try_from(1.5).unwrap()))])).unwrap();
    assert_eq!(report.rejected[0].reason, "RM-8: this device represents a rate as whole hertz");
}

#[test]
fn rm_26_the_envelope_carries_each_directions_start_lead() {
    // RM-20: the Manifest's envelope holds both leads (design 33 D5).
    let envelope = x3xx_description().envelope();
    assert_eq!(envelope.timing.rx_start_lead_ns, 50_000_000);
    assert_eq!(envelope.timing.tx_start_lead_ns, 50_000_000);
    let value = serde_json::to_value(&envelope).unwrap();
    let timing = value["timing"].as_object().unwrap();
    assert!(timing.contains_key("rx_start_lead_ns") && timing.contains_key("tx_start_lead_ns"));
    assert!(!timing.contains_key("start_lead_ns"));
}

#[test]
fn ur_13_the_rate_set_comparison_expands_ranges_and_ignores_order() {
    // UR-13, design 33 D5: the device's `(start, stop, step)` ranges are expanded and
    // compared as a set, in any order; a range is the values UHD would enumerate, and an
    // interval that is not one value ('step' 0, or start == stop as one) is handled.
    use ezsdr_radio::device::{Grid, rate_set_matches};
    let values = Grid::Values(vec![1.0, 2.5, 4.0]);
    assert!(rate_set_matches(&values, &[(1.0, 1.0, 0.0), (2.5, 2.5, 0.0), (4.0, 4.0, 0.0)]));
    assert!(rate_set_matches(&values, &[(4.0, 4.0, 0.0), (1.0, 2.5, 1.5)]), "a range expands");
    assert!(rate_set_matches(&values, &[(4.0, 4.0, 0.0), (2.5, 2.5, 0.0), (1.0, 1.0, 0.0)]), "the order does not matter");
    assert!(!rate_set_matches(&values, &[(1.0, 1.0, 0.0), (2.5, 2.5, 0.0)]), "a missing value");
    assert!(!rate_set_matches(&values, &[(1.0, 2.5, 1.5), (4.0, 4.0, 0.0), (9.0, 9.0, 0.0)]), "an extra value");
    assert!(!rate_set_matches(&values, &[(1.0, 4.0, 1.0)]), "a different set");
    assert!(rate_set_matches(&Grid::Step { lo: 1.0, hi: 4.0, step: 1.5 }, &[(1.0, 2.5, 1.5), (4.0, 4.0, 0.0)]));
    assert!(!rate_set_matches(&Grid::Step { lo: 0.0, hi: 1.0, step: 0.0 }, &[(1.0, 1.0, 0.0)]), "an interval is not a finite set");
}

#[test]
fn rm_38_the_keys_selector_is_read_once_for_both_providers() {
    // RM-38 (design 33 D5) and UR-5: `keys` is absent (the current keys), the string
    // `channel` (the layout's), or refused — the one judgement both Providers read.
    use std::collections::BTreeMap;
    use ezsdr_kernel::spec::Ident;
    let selector = |value: Option<Value>| -> BTreeMap<Ident, Value> {
        let mut selector = BTreeMap::new();
        if let Some(value) = value {
            selector.insert(Ident::parse("keys").unwrap(), value);
        }
        selector
    };
    assert_eq!(ezsdr_radio::channel_keys(&selector(None)), Ok(false));
    assert_eq!(ezsdr_radio::channel_keys(&selector(Some(Value::from("channel")))), Ok(true));
    for value in [
        Value::from("layout"),
        Value::from("Channel"),
        Value::from("channel "),
        Value::from(""),
        Value::from(1),
        Value::from(true),
        Value::List(vec![Scalar::from("channel")]),
    ] {
        assert_eq!(ezsdr_radio::channel_keys(&selector(Some(value.clone()))), Err("selector `keys` must be the string `channel`".to_owned()), "{value:?}");
    }
}
