use ezsdr_kernel::spec::{Ident, Key, Scalar, Value};
use ezsdr_kernel::stream::Direction;
use ezsdr_radio::channels::{self, Description};
use ezsdr_radio::device::Grid;
use ezsdr_radio::measurement::{
    compare, GridValue, Load, Record, Source, StreamCondition, StreamRef, ValueRecord, Worst,
};
use ezsdr_radio::x3xx::ProviderTerms;
use ezsdr_radio::measurement;
use serde_json::{json, Value as JsonValue};

const FIXTURE: &str = include_str!("fixtures/measurement/records/valid.json");
const INVALID_COMPARISONS: &str = include_str!("fixtures/measurement/invalid-comparisons.json");

fn fixture_record() -> Record {
    serde_json::from_str(FIXTURE).expect("the checked-in measurement fixture is valid")
}

fn description(record: &Record, terms: ProviderTerms) -> Description {
    let layout = channels::from_selector(&record.selector).expect("the fixture selector is valid");
    Description::new(&layout, 2_000, &terms)
}

fn mock_terms() -> ProviderTerms {
    ProviderTerms {
        min_timed_command_lead_ns: 2_000_000,
        repeat_max_samples: 268_435_456,
        repeat_align_samples: 2,
    }
}

fn uhd_terms() -> ProviderTerms {
    ProviderTerms {
        min_timed_command_lead_ns: 5_000_000,
        repeat_max_samples: u64::from(u32::MAX),
        repeat_align_samples: 1,
    }
}

fn key_of(value: &ValueRecord) -> &str {
    match value {
        ValueRecord::Measured { key, .. } | ValueRecord::Unmeasured { key, .. } => key.as_str(),
    }
}

fn set_worst(record: &mut Record, name: &str, worst: Worst) {
    let Some(ValueRecord::Measured { worst: value, .. }) = record.values.iter_mut().find(|value| key_of(value) == name) else {
        panic!("fixture has no measured value for {name}");
    };
    *value = worst;
}

fn assert_clear(record: &Record, description: &Description) {
    let violations = compare(record, description);
    assert!(violations.is_empty(), "{}", violations.iter().map(|v| format!("{}: {}", v.key, v.reason)).collect::<Vec<_>>().join("\n"));
}

fn assert_rejected(record: &Record, description: &Description, key: &str, reason: &str) {
    let violations = compare(record, description);
    assert!(violations.iter().any(|violation| violation.key == key && violation.reason.contains(reason)), "expected {key} / {reason:?}, got {violations:?}");
}

#[test]
fn rm_35_measurement_fixture_round_trips_and_accepts_unmeasured_values() {
    let record = fixture_record();
    let encoded = serde_json::to_value(&record).unwrap();
    assert_eq!(serde_json::from_value::<Record>(encoded).unwrap(), record);
    assert_eq!(record.load, Load::None);
    assert!(record.values.iter().any(|value| matches!(value, ValueRecord::Unmeasured { .. })));
    assert_clear(&record, &description(&record, mock_terms()));
}

#[test]
fn rm_35_each_value_rule_accepts_equal_or_stricter_claims_and_rejects_looser_ones() {
    let mut record = fixture_record();
    let description = description(&record, mock_terms());

    let key = "radio.timing.startup_latency_ns";
    set_worst(&mut record, key, Worst::Number(2_000_000_000.0));
    assert_clear(&record, &description);
    set_worst(&mut record, key, Worst::Number(2_000_000_001.0));
    assert_rejected(&record, &description, key, "below measured");
    set_worst(&mut record, key, Worst::Number(1_999_999_999.0));
    assert_clear(&record, &description);

    let key = "radio.timing.command_queue_depth";
    set_worst(&mut record, key, Worst::Number(16.0));
    assert_clear(&record, &description);
    set_worst(&mut record, key, Worst::Number(15.0));
    assert_rejected(&record, &description, key, "exceeds measured");
    set_worst(&mut record, key, Worst::Number(17.0));
    assert_clear(&record, &description);

    let key = "radio.rx.c0.frequency_hz";
    set_worst(&mut record, key, Worst::Grid(GridValue::Step { lo: 10_000_000.0, hi: 6_000_000_000.0, step: 1.0 }));
    assert_clear(&record, &description);
    set_worst(&mut record, key, Worst::Grid(GridValue::Step { lo: 10_000_000.0, hi: 6_000_000_000.0, step: 2.0 }));
    assert_rejected(&record, &description, key, "not a subset");
    set_worst(&mut record, key, Worst::Grid(GridValue::Step { lo: 9_999_999.0, hi: 6_000_000_001.0, step: 1.0 }));
    assert_clear(&record, &description);

    let key = "radio.rx.c0.antenna";
    set_worst(&mut record, key, Worst::Texts(vec!["RX2".to_owned(), "TX/RX".to_owned()]));
    assert_clear(&record, &description);
    set_worst(&mut record, key, Worst::Texts(vec!["RX2".to_owned()]));
    assert_rejected(&record, &description, key, "not a subset");
    set_worst(&mut record, key, Worst::Texts(vec!["RX2".to_owned(), "TX/RX".to_owned(), "A".to_owned()]));
    assert_clear(&record, &description);

    let key = "radio.tx.repeat_align_samples";
    set_worst(&mut record, key, Worst::Number(2.0));
    assert_clear(&record, &description);
    set_worst(&mut record, key, Worst::Number(3.0));
    assert_rejected(&record, &description, key, "not a positive multiple");
    set_worst(&mut record, key, Worst::Number(1.0));
    assert_clear(&record, &description);

    let key = "radio.rx.c0.phase_behavior_on_retune";
    set_worst(&mut record, key, Worst::Text("random_unless_timed_tune".to_owned()));
    assert_clear(&record, &description);
    set_worst(&mut record, key, Worst::Text("deterministic".to_owned()));
    assert_clear(&record, &description);
    set_worst(&mut record, key, Worst::Text("random".to_owned()));
    assert_rejected(&record, &description, key, "stronger");
    set_worst(&mut record, key, Worst::Text("random_unless_timed_tune".to_owned()));

    let key = "radio.full_duplex";
    let mut conservative = description.clone();
    conservative.full_duplex = false;
    assert_clear(&record, &conservative);
    set_worst(&mut record, key, Worst::Boolean(false));
    assert_rejected(&record, &description, key, "claims true");
    set_worst(&mut record, key, Worst::Boolean(true));

    let key = "radio.hardware_time";
    let mut conservative = description.clone();
    conservative.hardware_time = false;
    assert_clear(&record, &conservative);
    set_worst(&mut record, key, Worst::Boolean(false));
    assert_rejected(&record, &description, key, "claims true");
    set_worst(&mut record, key, Worst::Boolean(true));

    for key in ["radio.perf.rx_bytes_per_s", "radio.perf.tx_bytes_per_s"] {
        set_worst(&mut record, key, Worst::Number(99_999_999.0));
        assert_rejected(&record, &description, key, "exceeds measured");
        set_worst(&mut record, key, Worst::Number(100_000_001.0));
        assert_clear(&record, &description);
        set_worst(&mut record, key, Worst::Number(100_000_000.0));
    }

    let key = "radio.perf.wire_bytes_per_sample";
    set_worst(&mut record, key, Worst::Number(4.0));
    assert_clear(&record, &description);
    set_worst(&mut record, key, Worst::Number(5.0));
    assert_rejected(&record, &description, key, "differs from measured");
}

#[test]
fn rm_35_invalid_comparison_fixture_is_rejected_for_each_rule() {
    let record: Record = serde_json::from_str(INVALID_COMPARISONS).expect("the invalid comparison fixture is a valid record");
    let violations = compare(&record, &description(&record, mock_terms()));
    for (key, reason) in [
        ("radio.full_duplex", "claims true"),
        ("radio.timing.startup_latency_ns", "below measured"),
        ("radio.timing.command_queue_depth", "exceeds measured"),
        ("radio.rx.c0.frequency_hz", "not a subset"),
        ("radio.rx.c0.antenna", "not a subset"),
        ("radio.rx.s0.sample_rate_hz", "not a subset"),
        ("radio.tx.repeat_align_samples", "not a positive multiple"),
        ("radio.rx.c0.phase_behavior_on_retune", "stronger"),
        ("radio.perf.wire_bytes_per_sample", "differs from measured"),
    ] {
        assert!(violations.iter().any(|violation| violation.key == key && violation.reason.contains(reason)), "expected {key} / {reason:?}, got {violations:?}");
    }
}

#[test]
fn rm_35_stream_rate_grid_containment_is_checked() {
    let mut record = fixture_record();
    let desc = description(&record, mock_terms());
    let key = Key::parse("radio.rx.s0.sample_rate_hz").unwrap();
    let profile_grid = measurement::profile_values(&desc).remove(&key).unwrap();
    let index = record.values.iter().position(|value| key_of(value) == key.as_str()).unwrap();
    record.values[index] = ValueRecord::Measured {
        key: key.clone(),
        worst: profile_grid.clone(),
        trials: 1,
        condition: "baseline".to_owned(),
        test: "fixture".to_owned(),
    };
    assert_clear(&record, &desc);

    let Worst::Grid(GridValue::Values { mut values }) = profile_grid else {
        panic!("the X3xx stream rate profile is a finite Values grid");
    };
    let middle = values.len() / 2;
    values.remove(middle);
    set_worst(&mut record, key.as_str(), Worst::Grid(GridValue::Values { values }));
    assert_rejected(&record, &desc, key.as_str(), "not a subset");
}

#[test]
fn rm_35_step_and_values_grids_compare_by_all_members() {
    let mut record = fixture_record();
    let mut desc = description(&record, mock_terms());
    desc.rx[0].frequency = Grid::Step { lo: 1.0, hi: 5.0, step: 1.0 };
    let frequency = "radio.rx.c0.frequency_hz";
    set_worst(&mut record, frequency, Worst::Grid(GridValue::Values { values: vec![1.0, 3.0, 5.0] }));
    assert_rejected(&record, &desc, frequency, "not a subset");
    set_worst(&mut record, frequency, Worst::Grid(GridValue::Values { values: vec![1.0, 2.0, 3.0, 4.0, 5.0] }));
    assert_clear(&record, &desc);

    desc.rates = Grid::Values(vec![1_000_000.0, 3_000_000.0, 5_000_000.0]);
    let rate = Key::parse("radio.rx.s0.sample_rate_hz").unwrap();
    let index = record.values.iter().position(|value| key_of(value) == rate.as_str()).unwrap();
    record.values[index] = ValueRecord::Measured {
        key: rate.clone(),
        worst: Worst::Grid(GridValue::Step { lo: 1_000_000.0, hi: 5_000_000.0, step: 0.0 }),
        trials: 1,
        condition: "baseline".to_owned(),
        test: "fixture".to_owned(),
    };
    assert_clear(&record, &desc);
    set_worst(&mut record, rate.as_str(), Worst::Grid(GridValue::Step { lo: 1_000_000.0, hi: 5_000_000.0, step: 4_000_000.0 }));
    assert_rejected(&record, &desc, rate.as_str(), "not a subset");
}

#[test]
fn rm_35_singleton_step_grid_is_contained_without_a_step_ratio() {
    let mut record = fixture_record();
    let mut desc = description(&record, mock_terms());
    desc.rx[0].frequency = Grid::Step { lo: 6.0, hi: 6.0, step: 2.0 };
    let key = "radio.rx.c0.frequency_hz";
    set_worst(&mut record, key, Worst::Grid(GridValue::Step { lo: 0.0, hi: 6.0, step: 3.0 }));
    assert_clear(&record, &desc);

    set_worst(&mut record, key, Worst::Grid(GridValue::Step { lo: 0.0, hi: 5.0, step: 3.0 }));
    assert_rejected(&record, &desc, key, "not a subset");
}

#[test]
fn rm_35_integer_profile_grid_compares_values_integer_and_step_measurements() {
    let mut record = fixture_record();
    let mut desc = description(&record, mock_terms());
    desc.rx[0].frequency = Grid::Integer { lo: 0, hi: 6 };
    let key = "radio.rx.c0.frequency_hz";
    set_worst(&mut record, "radio.rx.c0.frequency_step_hz", Worst::Number(0.0));

    set_worst(&mut record, key, Worst::Grid(GridValue::Values { values: vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0] }));
    assert_clear(&record, &desc);
    set_worst(&mut record, key, Worst::Grid(GridValue::Values { values: vec![0.0, 2.0, 3.0, 4.0, 5.0, 6.0] }));
    assert_rejected(&record, &desc, key, "not a subset");

    set_worst(&mut record, key, Worst::Grid(GridValue::Integer { lo: 0, hi: 6 }));
    assert_clear(&record, &desc);
    set_worst(&mut record, key, Worst::Grid(GridValue::Integer { lo: 1, hi: 6 }));
    assert_rejected(&record, &desc, key, "not a subset");

    set_worst(&mut record, key, Worst::Grid(GridValue::Step { lo: 0.0, hi: 6.0, step: 1.0 }));
    assert_clear(&record, &desc);
    set_worst(&mut record, key, Worst::Grid(GridValue::Step { lo: 0.0, hi: 6.0, step: 2.0 }));
    assert_rejected(&record, &desc, key, "not a subset");
}

#[test]
fn rm_35_rejects_numeric_values_at_the_exact_f64_limit() {
    let mut record = fixture_record();
    let mut desc = description(&record, mock_terms());
    desc.repeat_max_samples = 9_007_199_254_740_993;
    let key = "radio.tx.repeat_max_samples";
    set_worst(&mut record, key, Worst::Number(9_007_199_254_740_992.0));
    assert_rejected(&record, &desc, key, "exact f64 limit");
}

#[test]
fn rm_35_integer_grid_containment_rejects_inexact_endpoints() {
    let mut record = fixture_record();
    let mut desc = description(&record, mock_terms());
    desc.rx[0].frequency = Grid::Integer { lo: 9_007_199_254_740_992, hi: 9_007_199_254_740_993 };
    let key = "radio.rx.c0.frequency_hz";
    set_worst(
        &mut record,
        key,
        Worst::Grid(GridValue::Step {
            lo: 9_007_199_254_740_992.0,
            hi: 9_007_199_254_740_992.0,
            step: 0.0,
        }),
    );
    assert_rejected(&record, &desc, key, "not a subset");
}

#[test]
fn rm_35_values_must_cover_each_comparable_capability_once() {
    let mut record = fixture_record();
    let desc = description(&record, mock_terms());
    assert_clear(&record, &desc);

    let missing = "radio.tx.c0.path_delay_samples";
    record.values.retain(|value| key_of(value) != missing);
    assert_rejected(&record, &desc, missing, "missing");

    let mut record = fixture_record();
    let duplicate = record.values.iter().find(|value| key_of(value) == "radio.full_duplex").unwrap().clone();
    record.values.push(duplicate);
    assert_rejected(&record, &description(&record, mock_terms()), "radio.full_duplex", "more than once");

    let mut record = fixture_record();
    record.values.push(ValueRecord::Unmeasured {
        key: Key::parse("radio.perf.unknown").unwrap(),
        reason: "fixture".to_owned(),
    });
    assert_rejected(&record, &description(&record, mock_terms()), "radio.perf.unknown", "unknown");

    let mut record = fixture_record();
    record.values.push(ValueRecord::Unmeasured {
        key: Key::parse("radio.rx.c0.stream").unwrap(),
        reason: "fixture".to_owned(),
    });
    assert_rejected(&record, &description(&record, mock_terms()), "radio.rx.c0.stream", "not recorded");

    let mut record = fixture_record();
    record.values.push(ValueRecord::Unmeasured {
        key: Key::parse("radio.rx.block_len").unwrap(),
        reason: "fixture".to_owned(),
    });
    assert_rejected(&record, &description(&record, mock_terms()), "radio.rx.block_len", "not recorded");
}

#[test]
fn rm_35_schema_source_and_profile_are_checked() {
    let mut json: JsonValue = serde_json::from_str(FIXTURE).unwrap();
    json["schema"] = json!("ezsdr.radio.measurement.v2");
    assert!(serde_json::from_value::<Record>(json).is_err());

    let mut record = fixture_record();
    let desc = description(&record, mock_terms());
    record.source = Source::Fake;
    assert_rejected(&record, &desc, "source", "fake-device");

    let mut record = fixture_record();
    record.profile.name = "other".to_owned();
    assert_rejected(&record, &description(&record, mock_terms()), "profile", "does not match");

    let mut record = fixture_record();
    record.profile.version.major += 1;
    assert_rejected(&record, &description(&record, mock_terms()), "profile", "does not match");
}

#[test]
fn rm_35_accepts_observation_only_records_with_unmeasured_values() {
    let mut record = fixture_record();
    record.values = record
        .values
        .into_iter()
        .map(|value| match value {
            ValueRecord::Measured { key, .. } => ValueRecord::Unmeasured { key, reason: "not measured".to_owned() },
            value => value,
        })
        .collect();
    assert_clear(&record, &description(&record, mock_terms()));
}

#[test]
fn rm_35_value_and_observation_metadata_is_validated() {
    let mut record = fixture_record();
    let Some(ValueRecord::Unmeasured { reason, .. }) = record.values.iter_mut().find(|value| matches!(value, ValueRecord::Unmeasured { .. })) else {
        panic!("fixture has an unmeasured value");
    };
    *reason = "  ".to_owned();
    assert_rejected(&record, &description(&record, mock_terms()), "radio.rx.s0.sample_rate_hz", "needs a reason");

    let mut record = fixture_record();
    let Some(ValueRecord::Measured { trials, .. }) = record.values.iter_mut().find(|value| matches!(value, ValueRecord::Measured { .. })) else {
        panic!("fixture has a measured value");
    };
    *trials = 0;
    assert_rejected(&record, &description(&record, mock_terms()), "radio.full_duplex", "at least one trial");

    let mut record = fixture_record();
    let Some(ValueRecord::Measured { test, .. }) = record.values.iter_mut().find(|value| matches!(value, ValueRecord::Measured { .. })) else {
        panic!("fixture has a measured value");
    };
    *test = String::new();
    assert_rejected(&record, &description(&record, mock_terms()), "radio.full_duplex", "test name");

    let mut record = fixture_record();
    record.observations[0].name.clear();
    assert_rejected(&record, &description(&record, mock_terms()), "observations", "needs a name");

    let mut record = fixture_record();
    record.observations[0].test.clear();
    assert_rejected(&record, &description(&record, mock_terms()), "observations", "test and at least one trial");

    let mut record = fixture_record();
    record.observations[0].trials = 0;
    assert_rejected(&record, &description(&record, mock_terms()), "observations", "at least one trial");
}

#[test]
fn rm_35_conditions_and_references_match_the_layout() {
    let record = fixture_record();
    let description = description(&record, mock_terms());

    let mut changed = record.clone();
    let duplicate = changed.conditions[0].clone();
    changed.conditions.push(duplicate);
    assert_rejected(&changed, &description, "conditions", "duplicate condition id");

    let mut changed = record.clone();
    changed.conditions[0].streams[0].stream = 1;
    assert_rejected(&changed, &description, "conditions.baseline.streams.1.Rx", "outside this direction");

    let mut changed = record.clone();
    changed.conditions[0].streams[0].channels = vec![1];
    assert_rejected(&changed, &description, "conditions.baseline.streams.0.Rx", "outside this direction's layout");

    let mut changed = record.clone();
    changed.conditions[0].streams[0].sample_rate_hz = 1_000_001.0;
    assert_rejected(&changed, &description, "conditions.baseline.streams.0.Rx", "not on the description's rate grid");

    let mut changed = record.clone();
    let duplicate = changed.conditions[0].streams[0].clone();
    changed.conditions[0].streams.push(duplicate);
    assert_rejected(&changed, &description, "conditions.baseline.streams.0.Rx", "declared more than once");

    let mut changed = record.clone();
    changed.conditions[0].streams[0].channels = vec![0, 0];
    let violations = compare(&changed, &description);
    assert!(violations.iter().any(|violation| violation.key == "conditions.baseline.streams.0.Rx" && violation.reason.contains("repeated in the stream")), "{violations:?}");
    assert!(!violations.iter().any(|violation| violation.reason.contains("more than one stream")), "a duplicate in one stream is not a cross-stream conflict: {violations:?}");

    let mut changed = record.clone();
    changed.conditions[0].concurrent.push(StreamRef { direction: Direction::Rx, stream: 1 });
    assert_rejected(&changed, &description, "conditions.concurrent", "not declared");

    let mut changed = record.clone();
    changed.conditions[0].concurrent.push(StreamRef { direction: Direction::Rx, stream: 0 });
    assert_rejected(&changed, &description, "conditions.concurrent", "listed more than once");

    let mut changed = record.clone();
    let Some(ValueRecord::Measured { condition, .. }) = changed.values.iter_mut().find(|value| key_of(value) == "radio.full_duplex") else {
        panic!("fixture has a measured full-duplex key");
    };
    *condition = "missing".to_owned();
    assert_rejected(&changed, &description, "radio.full_duplex", "not declared");

    let mut changed = record.clone();
    changed.observations[0].condition = Some("missing".to_owned());
    assert_rejected(&changed, &description, "observations", "not declared");
}

#[test]
fn rm_35_condition_validation_keeps_direction_in_stream_channel_and_concurrent_checks() {
    let mut record = fixture_record();
    record.selector.insert(
        Ident::parse("rx").unwrap(),
        Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:B:ubx:0")]),
    );
    let description = description(&record, mock_terms());

    let mut changed = record.clone();
    changed.conditions[0].streams.push(StreamCondition {
        direction: Direction::Rx,
        stream: 2,
        channels: Vec::new(),
        sample_rate_hz: 1_000_000.0,
    });
    assert_rejected(&changed, &description, "conditions.baseline.streams.2.Rx", "outside this direction");

    let mut changed = record.clone();
    changed.conditions[0].streams.push(StreamCondition {
        direction: Direction::Rx,
        stream: 1,
        channels: vec![1],
        sample_rate_hz: 1_000_000.0,
    });
    for key in measurement::profile_values(&description).into_keys() {
        if measurement::rule(key.as_str()).is_some_and(|rule| rule != measurement::Rule::NotCompared)
            && !changed.values.iter().any(|value| key_of(value) == key.as_str())
        {
            changed.values.push(ValueRecord::Unmeasured { key, reason: "not measured".to_owned() });
        }
    }
    assert_clear(&changed, &description);

    let mut changed = record.clone();
    changed.conditions[0].streams.push(StreamCondition {
        direction: Direction::Tx,
        stream: 1,
        channels: Vec::new(),
        sample_rate_hz: 1_000_000.0,
    });
    assert_rejected(&changed, &description, "conditions.baseline.streams.1.Tx", "outside this direction");

    let mut changed = record.clone();
    changed.conditions[0].streams[0].channels = vec![2];
    assert_rejected(&changed, &description, "conditions.baseline.streams.0.Rx", "outside this direction's layout");

    let mut changed = record.clone();
    changed.conditions[0].streams[1].channels = vec![1];
    assert_rejected(&changed, &description, "conditions.baseline.streams.0.Tx", "outside this direction's layout");

    let mut changed = record;
    changed.conditions[0].streams.retain(|stream| stream.direction == Direction::Rx);
    changed.conditions[0].concurrent = vec![StreamRef { direction: Direction::Tx, stream: 0 }];
    assert_rejected(&changed, &description, "conditions.concurrent", "not declared");
}

#[test]
fn rm_35_a_channel_cannot_belong_to_two_streams_in_one_direction() {
    let mut record = fixture_record();
    record.selector.insert(
        Ident::parse("rx").unwrap(),
        Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:B:ubx:0")]),
    );
    record.conditions[0].streams.push(StreamCondition {
        direction: Direction::Rx,
        stream: 1,
        channels: vec![0],
        sample_rate_hz: 1_000_000.0,
    });
    let description = description(&record, mock_terms());
    assert_rejected(&record, &description, "conditions.baseline.streams.1.Rx", "more than one stream");
}

#[test]
fn rm_35_each_provider_is_compared_with_its_own_description() {
    let mut record = fixture_record();
    set_worst(&mut record, "radio.timing.min_timed_command_lead_ns", Worst::Number(3_000_000.0));
    let mock = description(&record, mock_terms());
    let uhd = description(&record, uhd_terms());

    assert_rejected(&record, &mock, "radio.timing.min_timed_command_lead_ns", "below measured");
    assert_clear(&record, &uhd);
}

#[test]
fn rm_35_rule_labels_match_the_declared_comparison_classes() {
    for (key, expected) in [
        ("radio.timing.min_timed_command_lead_ns", measurement::Rule::LowerBound),
        ("radio.timing.command_queue_depth", measurement::Rule::UpperBound),
        ("radio.timing.release_window_ns", measurement::Rule::LowerBound),
        ("radio.timing.overflow_restart_gap_ns", measurement::Rule::LowerBound),
        ("radio.timing.rx_start_lead_ns", measurement::Rule::LowerBound),
        ("radio.timing.tx_start_lead_ns", measurement::Rule::LowerBound),
        ("radio.timing.tx_in_flight_ns", measurement::Rule::LowerBound),
        ("radio.timing.startup_latency_ns", measurement::Rule::LowerBound),
        ("radio.rx.c0.frequency_hz", measurement::Rule::Subset),
        ("radio.rx.c1.frequency_step_hz", measurement::Rule::LowerBound),
        ("radio.tx.c1.gain_step_db", measurement::Rule::LowerBound),
        ("radio.tx.repeat_align_samples", measurement::Rule::Multiple),
        ("radio.tx.repeat_max_samples", measurement::Rule::UpperBound),
        ("radio.rx.c0.phase_behavior_on_retune", measurement::Rule::Guarantee),
        ("radio.full_duplex", measurement::Rule::TrueImplies),
        ("radio.hardware_time", measurement::Rule::TrueImplies),
        ("radio.perf.rx_bytes_per_s", measurement::Rule::UpperBound),
        ("radio.perf.tx_bytes_per_s", measurement::Rule::UpperBound),
        ("radio.perf.wire_bytes_per_sample", measurement::Rule::Equal),
        ("radio.tx.c1.path_delay_samples", measurement::Rule::Equal),
        ("radio.tx.c0.stream", measurement::Rule::NotCompared),
        ("radio.rx.block_len", measurement::Rule::NotCompared),
        ("radio.rx.s1.sample_rate_hz", measurement::Rule::Subset),
    ] {
        assert_eq!(measurement::rule(key), Some(expected), "{key}");
    }
}
