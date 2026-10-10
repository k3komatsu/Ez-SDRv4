//! RM-26: `ezsdr_radio::device` against the `x310-like` values (Phase 7, VE-1).

use ezsdr_kernel::spec::Scalar;
use std::collections::BTreeMap;

use ezsdr_kernel::id::ResourceId;
use ezsdr_kernel::module_api::{ProfileRef, Requested, Version};
use ezsdr_kernel::spec::{CapabilityValue, Constraint, Key, Value};
use ezsdr_radio::device::{DeviceDescription, Grid};
use ezsdr_radio::{PerformanceEnvelope, TimingEnvelope, keys};

fn key(name: &str) -> Key {
    Key::parse(name).unwrap()
}

fn x310_like() -> DeviceDescription {
    DeviceDescription {
        profile: ProfileRef { name: "x310-like".to_owned(), version: Version::new(1, 2, 0) },
        rates: Grid::Values((1..=512).rev().map(|n| 200_000_000.0 / f64::from(n)).collect()),
        whole_hertz_rates: false,
        frequency: Grid::Step { lo: 10_000_000.0, hi: 6_000_000_000.0, step: 1.0 },
        gain: Grid::Step { lo: 0.0, hi: 31.5, step: 0.5 },
        max_channels: 2,
        rx_antennas: vec!["RX2".to_owned(), "TX/RX".to_owned()],
        tx_antennas: vec!["TX/RX".to_owned()],
        coherent: true,
        full_duplex: true,
        hardware_time: true,
        phase_behavior_on_retune: "random_unless_timed_tune".to_owned(),
        repeat_max_samples: 268_435_456,
        repeat_align_samples: 2,
        block_len: 2_000,
        tx_path_delay_samples: 45,
        rx_path_delay_samples: 0,
        timing: TimingEnvelope {
            min_timed_command_lead_ns: 2_000_000,
            startup_latency_ns: 2_000_000_000,
            command_queue_depth: 16,
            overflow_restart_gap_ns: 50_000_000,
            start_lead_ns: 50_000_000,
        },
        performance: PerformanceEnvelope {
            rx_bytes_per_s: 1_000_000_000,
            tx_bytes_per_s: 1_000_000_000,
            wire_bytes_per_sample: 4,
        },
        defaults: DeviceDescription::x310_defaults(),
    }
}

/// The capabilities MockRadio 1.2.0's `Profile::tree` built for `x310-like`, written
/// out here independently of the code under test (MR-4).
fn mockradio_1_2_capabilities() -> BTreeMap<Key, CapabilityValue> {
    let range = |min, max| CapabilityValue::Range { min, max };
    let one = |value| CapabilityValue::One { value };
    let mut expected = BTreeMap::new();
    for direction in ["rx", "tx"] {
        expected.insert(key(&format!("radio.{direction}.channels")), range(Scalar::from(0), Scalar::from(2)));
        expected.insert(
            key(&format!("radio.{direction}.sample_rate_hz")),
            CapabilityValue::AnyOf {
                values: (1..=512).map(|n| Scalar::try_from(200_000_000.0 / f64::from(n)).unwrap()).collect(),
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
        (keys::START_LEAD_NS, Scalar::from(50_000_000)),
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
fn rm_26_the_description_builds_the_tree_mockradio_built() {
    let id = ResourceId::parse("dev").unwrap();
    let tree = x310_like().tree(&id);
    assert_eq!(tree.id, id);
    assert_eq!(tree.kind.as_str(), "radio.device");
    assert_eq!(tree.capabilities, mockradio_1_2_capabilities());
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
    assert!(!tree.shareable);
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
    let description = x310_like();
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

    // Four channels exceed the device's two (RM-8).
    let report = coerce(&[(keys::RX_CHANNELS, eq(Scalar::from(4)))]);
    assert!(report.rejected[0].reason.starts_with("RM-8: "), "{:?}", report.rejected);

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
    let whole = DeviceDescription { whole_hertz_rates: true, rates: Grid::Integer { lo: 1, hi: 1_000_000_000 }, ..x310_like() };
    let report = whole.coerce(&dev, &request(&[(keys::RX_SAMPLE_RATE_HZ, eq(Scalar::try_from(1.5).unwrap()))])).unwrap();
    assert_eq!(report.rejected[0].reason, "RM-8: this device represents a rate as whole hertz");
}
