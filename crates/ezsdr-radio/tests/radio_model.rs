use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use ezsdr_kernel::binding::{AdmissionCheckRegistry, CheckStage, Violation};
use ezsdr_kernel::event::{EventKind, Severity};
use ezsdr_kernel::id::ClockDomainId;
use ezsdr_kernel::module_api::{CompileRule, ModuleRegistry, UpdateClass, Version};
use ezsdr_kernel::policy::{EventKindRegistry, Reaction};
use ezsdr_kernel::spec::{CoercionPolicy, Ident, Key, Namespace, Value, ValueKind};
use ezsdr_kernel::stream::LatePolicy;
use ezsdr_kernel::time::TimePoint;
use ezsdr_radio::payloads::{
    AlignmentErrorPayload, ClockLostPayload, ClockReference, CommandQueueFullPayload,
    CommandRejectedPayload, LateCommandPayload, RxOverflowCause, RxOverflowPayload,
    TimeErrorCause, TimeErrorOutcome, TimeErrorPayload, TxUnderflowCause, TxUnderflowPayload,
};
use ezsdr_radio::{RfEnvelopeCheck, keys};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value as JsonValue, json};

fn schemas_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/radio")
}

fn envelope_section(value: JsonValue) -> BTreeMap<Namespace, JsonValue> {
    BTreeMap::from([(Namespace::parse("radio.rf_envelope").unwrap(), value)])
}

fn radio_configuration(values: &[(&str, Value)]) -> BTreeMap<Ident, BTreeMap<Key, Value>> {
    BTreeMap::from([(
        Ident::parse("radio").unwrap(),
        values
            .iter()
            .map(|(key, value)| (Key::parse(key).unwrap(), value.clone()))
            .collect(),
    )])
}

fn run_check(
    section: JsonValue,
    effective: &BTreeMap<Ident, BTreeMap<Key, Value>>,
    proposed: &BTreeMap<Ident, BTreeMap<Key, Value>>,
    stage: CheckStage,
) -> Vec<Violation> {
    let mut checks = AdmissionCheckRegistry::new();
    checks.register(Arc::new(RfEnvelopeCheck));
    checks.run(&envelope_section(section), effective, proposed, stage)
}

fn base_rf_envelope() -> JsonValue {
    json!({
        "allowed_bands": [{"lo_hz": 2_400_000_000.0, "hi_hz": 2_500_000_000.0}],
        "max_gain_db": 20.0,
        "tx_enabled": [true, false],
        "antenna_ports": ["TX/RX", "RX2"]
    })
}

fn tx_values(channels: i64, frequency_hz: f64, gain_db: f64) -> Vec<(&'static str, Value)> {
    vec![
        (keys::TX_CHANNELS, Value::Int(channels)),
        (keys::TX_FREQUENCY_HZ, Value::Num(frequency_hz)),
        (keys::TX_GAIN_DB, Value::Num(gain_db)),
        (keys::TX_ANTENNA, Value::Str("TX/RX".to_owned())),
    ]
}

fn value_json(value: &Option<Value>) -> JsonValue {
    serde_json::to_value(value).unwrap()
}

#[test]
fn rm_01_register_adds_the_descriptor_the_check_and_the_kinds() {
    let mut registry = ModuleRegistry::new();
    let mut checks = AdmissionCheckRegistry::new();
    let mut kinds = EventKindRegistry::new();
    ezsdr_radio::register(&mut registry, &mut checks, &mut kinds).unwrap();

    let descriptor = registry
        .vocabulary(&Namespace::parse("radio").unwrap())
        .unwrap();
    assert_eq!(descriptor.id, Namespace::parse("radio").unwrap());
    assert_eq!(descriptor.version, Version::new(1, 4, 0));
    assert_eq!(descriptor.prefix, Namespace::parse("radio").unwrap());
    assert_eq!(descriptor.checks, [Namespace::parse("radio.rf_envelope").unwrap()]);

    let violations = checks.run(
        &envelope_section(json!({"allowed_bands": "not a list"})),
        &BTreeMap::new(),
        &BTreeMap::new(),
        CheckStage::Validate,
    );
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].check, Namespace::parse("radio.rf_envelope").unwrap());
    assert!(violations[0].reason.starts_with("RM-19: "));
    assert!(violations[0].key.is_none());
    assert!(violations[0].requested.is_none());

    assert_eq!(kinds.kinds().len(), 9);
    for declaration in &descriptor.event_kinds {
        assert_eq!(kinds.get(&declaration.kind), Some(declaration));
    }
}

#[test]
fn rm_01_register_twice_is_refused() {
    let mut registry = ModuleRegistry::new();
    let mut checks = AdmissionCheckRegistry::new();
    let mut kinds = EventKindRegistry::new();
    ezsdr_radio::register(&mut registry, &mut checks, &mut kinds).unwrap();
    assert!(ezsdr_radio::register(&mut registry, &mut checks, &mut kinds).is_err());

    assert_eq!(registry.vocabulary(&Namespace::parse("radio").unwrap()).unwrap().keys.len(), 33);
    assert_eq!(kinds.kinds().len(), 9);
    assert_eq!(
        checks
            .run(
                &envelope_section(json!({"allowed_bands": "not a list"})),
                &BTreeMap::new(),
                &BTreeMap::new(),
                CheckStage::Validate,
            )
            .len(),
        1,
    );
}

#[test]
fn rm_04_the_key_table_is_exactly_the_declared_one() {
    let cold = Some(UpdateClass::Cold);
    let timed = Some(UpdateClass::HardwareTimed);
    let none = None;
    let reject = CoercionPolicy::Reject;
    let warn = CoercionPolicy::Warn;
    let num = ValueKind::Num;
    let int = ValueKind::Int;
    let str_ = ValueKind::Str;
    let bool_ = ValueKind::Bool;
    let expected = [
        ("radio.rx.channels", int, false, reject, cold),
        ("radio.tx.channels", int, false, reject, cold),
        ("radio.rx.sample_rate_hz", num, true, reject, cold),
        ("radio.tx.sample_rate_hz", num, true, reject, cold),
        ("radio.rx.frequency_hz", num, true, reject, timed),
        ("radio.tx.frequency_hz", num, true, reject, timed),
        ("radio.rx.gain_db", num, true, warn, timed),
        ("radio.tx.gain_db", num, true, warn, timed),
        ("radio.rx.antenna", str_, false, reject, none),
        ("radio.tx.antenna", str_, false, reject, none),
        ("radio.rx.frequency_step_hz", num, false, reject, none),
        ("radio.tx.frequency_step_hz", num, false, reject, none),
        ("radio.rx.gain_step_db", num, false, reject, none),
        ("radio.tx.gain_step_db", num, false, reject, none),
        ("radio.rx.coherent", bool_, false, reject, none),
        ("radio.full_duplex", bool_, false, reject, none),
        ("radio.hardware_time", bool_, false, reject, none),
        ("radio.phase_behavior_on_retune", str_, false, reject, none),
        ("radio.tx.repeat_max_samples", int, false, reject, none),
        ("radio.tx.repeat_align_samples", int, false, reject, none),
        ("radio.rx.block_len", int, false, reject, none),
        ("radio.timing.min_timed_command_lead_ns", int, false, reject, none),
        ("radio.timing.startup_latency_ns", int, false, reject, none),
        ("radio.timing.stop_tail_ns", int, false, reject, none),
        ("radio.timing.command_queue_depth", int, false, reject, none),
        ("radio.timing.overflow_restart_gap_ns", int, false, reject, none),
        ("radio.timing.restart_lead_ns", int, false, reject, none),
        ("radio.timing.start_lead_ns", int, false, reject, none),
        ("radio.perf.rx_bytes_per_s", int, false, reject, none),
        ("radio.perf.tx_bytes_per_s", int, false, reject, none),
        ("radio.perf.wire_bytes_per_sample", int, false, reject, none),
        ("radio.tx.path_delay_samples", int, false, reject, none),
        ("radio.rx.path_delay_samples", int, false, reject, none),
    ];
    let declarations = ezsdr_radio::vocabulary().keys;
    assert_eq!(declarations.len(), expected.len());
    for (declaration, (key, kind, coercible, coercion_default, update_class))
        in declarations.iter().zip(expected)
    {
        assert_eq!(declaration.key.as_str(), key);
        assert_eq!(declaration.kind, kind);
        assert_eq!(declaration.coercible, coercible);
        assert_eq!(declaration.coercion_default, coercion_default);
        assert_eq!(declaration.update_class, update_class);
    }
    assert_eq!(keys::CONFIGURATION.len(), 10);
    assert_eq!(
        keys::CONFIGURATION,
        [
            "radio.rx.channels",
            "radio.tx.channels",
            "radio.rx.sample_rate_hz",
            "radio.tx.sample_rate_hz",
            "radio.rx.frequency_hz",
            "radio.tx.frequency_hz",
            "radio.rx.gain_db",
            "radio.tx.gain_db",
            "radio.rx.antenna",
            "radio.tx.antenna",
        ]
    );
}

#[test]
fn rm_10_the_kinds_are_registered_under_radio_with_their_defaults() {
    let expected = [
        ("radio.RX_OVERFLOW", Severity::Warning, Reaction::MarkArtifact),
        ("radio.TX_UNDERFLOW", Severity::Warning, Reaction::MarkArtifact),
        ("radio.TX_DISCONTINUITY", Severity::Warning, Reaction::MarkArtifact),
        ("radio.TIME_ERROR", Severity::Error, Reaction::MarkArtifact),
        ("radio.LATE_COMMAND", Severity::Warning, Reaction::MarkArtifact),
        ("radio.ALIGNMENT_ERROR", Severity::Error, Reaction::MarkArtifact),
        ("radio.CLOCK_LOST", Severity::Fatal, Reaction::Abort),
        ("radio.COMMAND_QUEUE_FULL", Severity::Error, Reaction::Abort),
        ("radio.COMMAND_REJECTED", Severity::Error, Reaction::MarkArtifact),
    ];
    let declarations = ezsdr_radio::vocabulary().event_kinds;
    assert_eq!(declarations.len(), expected.len());
    for (declaration, (name, severity, default)) in declarations.iter().zip(expected) {
        assert_eq!(declaration.kind.as_str(), name);
        assert!(declaration.kind.as_str().starts_with("radio."));
        assert_eq!(declaration.severity, severity);
        assert_eq!(declaration.default, default);
    }

    let mut kinds = EventKindRegistry::new();
    let mut registry = ModuleRegistry::new();
    ezsdr_radio::register(&mut registry, &mut AdmissionCheckRegistry::new(), &mut kinds).unwrap();
    for (name, severity, default) in expected {
        let declaration = kinds.get(&EventKind::parse(name).unwrap()).unwrap();
        assert_eq!(declaration.severity, severity);
        assert_eq!(declaration.default, default);
    }
}

#[test]
fn rm_12_the_verbs_compile_to_bursts() {
    let descriptor = ezsdr_radio::vocabulary();
    assert_eq!(descriptor.verbs.len(), 2);
    assert_eq!(descriptor.verbs[0].verb.as_str(), "start_repeat");
    assert!(matches!(
        descriptor.verbs[0].compiles_to,
        CompileRule::TxBurst {
            repeat: true,
            late_policy: LatePolicy::SendAsapAndFlag
        }
    ));
    assert_eq!(descriptor.verbs[1].verb.as_str(), "send");
    assert!(matches!(
        descriptor.verbs[1].compiles_to,
        CompileRule::TxBurst {
            repeat: false,
            late_policy: LatePolicy::DropAndFlag
        }
    ));

    let mut registry = ModuleRegistry::new();
    registry.register_vocabulary(descriptor).unwrap();
    assert_eq!(registry.vocabulary(&Namespace::parse("radio").unwrap()).unwrap().verbs.len(), 2);
}

#[test]
fn rm_19_rf_envelope_cases() {
    let empty = BTreeMap::new();
    let valid = radio_configuration(&tx_values(1, 2_450_000_000.0, 10.0));
    assert!(run_check(base_rf_envelope(), &valid, &empty, CheckStage::Validate).is_empty());

    let out_of_band = radio_configuration(&tx_values(1, 2_600_000_000.0, 10.0));
    let violations = run_check(base_rf_envelope(), &out_of_band, &empty, CheckStage::Validate);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].key.as_ref().unwrap().as_str(), keys::TX_FREQUENCY_HZ);
    assert_eq!(value_json(&violations[0].requested), json!(2_600_000_000.0));
    assert!(violations[0].reason.starts_with("RM-19: radio: "));

    let high_gain = radio_configuration(&tx_values(1, 2_450_000_000.0, 25.0));
    let violations = run_check(base_rf_envelope(), &high_gain, &empty, CheckStage::Validate);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].key.as_ref().unwrap().as_str(), keys::TX_GAIN_DB);

    let two_channels = radio_configuration(&tx_values(2, 2_450_000_000.0, 10.0));
    let violations = run_check(base_rf_envelope(), &two_channels, &empty, CheckStage::Validate);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].key.as_ref().unwrap().as_str(), keys::TX_CHANNELS);

    let mut globally_disabled_envelope = base_rf_envelope();
    globally_disabled_envelope["tx_enabled"] = json!(false);
    let violations = run_check(
        globally_disabled_envelope,
        &valid,
        &empty,
        CheckStage::Validate,
    );
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].key.as_ref().unwrap().as_str(), keys::TX_CHANNELS);

    let mut short_channel_envelope = base_rf_envelope();
    short_channel_envelope["tx_enabled"] = json!([true]);
    let violations = run_check(
        short_channel_envelope,
        &two_channels,
        &empty,
        CheckStage::Validate,
    );
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].key.as_ref().unwrap().as_str(), keys::TX_CHANNELS);

    let mut bad_tx_antenna = tx_values(1, 2_450_000_000.0, 10.0);
    bad_tx_antenna[3].1 = Value::Str("J1".to_owned());
    let bad_tx_antenna = radio_configuration(&bad_tx_antenna);
    let violations = run_check(base_rf_envelope(), &bad_tx_antenna, &empty, CheckStage::Validate);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].key.as_ref().unwrap().as_str(), keys::TX_ANTENNA);

    let disabled_tx = radio_configuration(&tx_values(0, 2_600_000_000.0, 25.0));
    assert!(run_check(base_rf_envelope(), &disabled_tx, &empty, CheckStage::Validate).is_empty());

    let bad_rx_antenna = radio_configuration(&[(keys::RX_ANTENNA, Value::Str("J1".to_owned()))]);
    let violations = run_check(base_rf_envelope(), &bad_rx_antenna, &empty, CheckStage::Validate);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].key.as_ref().unwrap().as_str(), keys::RX_ANTENNA);

    let fragments = BTreeMap::from([
        (
            Ident::parse("good_radio").unwrap(),
            radio_configuration(&tx_values(1, 2_450_000_000.0, 10.0))
                .remove(&Ident::parse("radio").unwrap())
                .unwrap(),
        ),
        (
            Ident::parse("bad_radio").unwrap(),
            radio_configuration(&tx_values(1, 2_600_000_000.0, 10.0))
                .remove(&Ident::parse("radio").unwrap())
                .unwrap(),
        ),
    ]);
    let violations = run_check(base_rf_envelope(), &fragments, &empty, CheckStage::Validate);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].key.as_ref().unwrap().as_str(), keys::TX_FREQUENCY_HZ);
    assert!(violations[0].reason.starts_with("RM-19: bad_radio: "));
}

#[test]
fn rm_19_a_malformed_section_is_one_violation() {
    let empty = BTreeMap::new();
    for section in [
        json!({"allowed_bands": "x"}),
        json!({"allowed_bands": [{"lo_hz": 10.0, "hi_hz": 5.0}]}),
        json!({"allowed_bands": [], "unknown": true}),
    ] {
        let violations = run_check(section, &empty, &empty, CheckStage::Prepare);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].reason.starts_with("RM-19: "));
        assert!(violations[0].key.is_none());
        assert!(violations[0].requested.is_none());
    }
}

#[test]
fn rm_19_the_proposed_value_is_judged() {
    let effective = radio_configuration(&tx_values(1, 2_450_000_000.0, 10.0));
    let proposed = radio_configuration(&[(keys::TX_FREQUENCY_HZ, Value::Num(2_600_000_000.0))]);
    let violations = run_check(base_rf_envelope(), &effective, &proposed, CheckStage::Runtime);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].key.as_ref().unwrap().as_str(), keys::TX_FREQUENCY_HZ);
    assert_eq!(value_json(&violations[0].requested), json!(2_600_000_000.0));
}

#[test]
fn rm_20_schema_freeze() {
    let dir = schemas_dir();
    let update = std::env::var("EZSDR_UPDATE_SCHEMAS").is_ok();
    if update {
        std::fs::create_dir_all(&dir).expect("schemas/radio is writable");
    }
    let schemas = ezsdr_radio::document_schemas();
    let mut stale = Vec::new();
    if !update {
        for entry in std::fs::read_dir(&dir).expect("schemas/radio is readable") {
            let file = entry.expect("a schema directory entry").file_name().to_string_lossy().into_owned();
            if file.ends_with(".json")
                && !schemas
                    .keys()
                    .any(|name| ezsdr_kernel::schema::file_name(name) == file)
            {
                stale.push(format!("{file}: no registered document generates it"));
            }
        }
    }
    for (name, schema) in schemas {
        let path = dir.join(ezsdr_kernel::schema::file_name(name));
        let rendered = ezsdr_kernel::schema::render(&schema);
        if update {
            std::fs::write(&path, &rendered).expect("schemas/radio is writable");
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(committed) if committed == rendered => {}
            Ok(_) => stale.push(format!("{name}: differs from the committed schema")),
            Err(_) => stale.push(format!("{name}: no committed schema at {}", path.display())),
        }
    }
    assert!(stale.is_empty(), "RM-20: schema freeze failed:\n{}", stale.join("\n"));
}

fn assert_payload_round_trip<T>(value: T, expected: JsonValue)
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let encoded = serde_json::to_value(&value).unwrap();
    assert_eq!(encoded, expected);
    assert_eq!(serde_json::from_value::<T>(encoded.clone()).unwrap(), value);
    let mut with_extra = encoded;
    with_extra
        .as_object_mut()
        .unwrap()
        .insert("extra".to_owned(), json!(true));
    assert!(serde_json::from_value::<T>(with_extra).is_err());
}

#[test]
fn rm_22_payloads_round_trip() {
    let target = TimePoint {
        domain: ClockDomainId::local(2),
        ticks: 123,
    };
    let target_json = serde_json::to_value(target).unwrap();
    assert_payload_round_trip(
        RxOverflowPayload {
            cause: RxOverflowCause::Overrun,
            lost: 3,
            restart_gap_ns: 50,
        },
        json!({"cause": "overrun", "lost": 3, "restart_gap_ns": 50}),
    );
    assert_payload_round_trip(
        TimeErrorPayload {
            cause: TimeErrorCause::UnclosedBurst,
            outcome: TimeErrorOutcome::PlanViolation,
            late_by_ns: 7,
            target,
        },
        json!({
            "cause": "unclosed_burst",
            "outcome": "plan_violation",
            "late_by_ns": 7,
            "target": target_json
        }),
    );
    assert_payload_round_trip(
        LateCommandPayload {
            key: None,
            requested: target,
            applied: TimePoint { ticks: 124, ..target },
        },
        json!({
            "key": null,
            "requested": target_json,
            "applied": {"domain": {"node": 0, "local": 2}, "ticks": 124}
        }),
    );
    assert_payload_round_trip(
        CommandQueueFullPayload {
            key: Key::parse(keys::TX_FREQUENCY_HZ).unwrap(),
            depth: 8,
        },
        json!({"key": keys::TX_FREQUENCY_HZ, "depth": 8}),
    );
    assert_payload_round_trip(
        CommandRejectedPayload {
            action: "tx_burst".to_owned(),
            reason: "bad waveform".to_owned(),
        },
        json!({"action": "tx_burst", "reason": "bad waveform"}),
    );
    // VE-3 (radio 1.3.0).
    assert_payload_round_trip(
        TxUnderflowPayload { cause: TxUnderflowCause::Starved },
        json!({"cause": "starved"}),
    );
    assert_payload_round_trip(
        TxUnderflowPayload { cause: TxUnderflowCause::Lost },
        json!({"cause": "lost"}),
    );
    assert_payload_round_trip(AlignmentErrorPayload { lost: 12 }, json!({"lost": 12}));
    assert_payload_round_trip(
        ClockLostPayload { reference: ClockReference::Frequency },
        json!({"reference": "frequency"}),
    );
    assert_payload_round_trip(
        TimeErrorPayload {
            cause: TimeErrorCause::Late,
            outcome: TimeErrorOutcome::LateAtDevice,
            late_by_ns: 0,
            target,
        },
        json!({"cause": "late", "outcome": "late_at_device", "late_by_ns": 0, "target": target_json}),
    );
}

#[test]
fn rm_22_late_at_device_is_snake_case() {
    assert_eq!(serde_json::to_value(TimeErrorOutcome::LateAtDevice).unwrap(), json!("late_at_device"));
}

#[test]
fn rm_24_the_hot_form_round_trips() {
    let overrun = RxOverflowPayload { cause: RxOverflowCause::Overrun, lost: 50_000, restart_gap_ns: 50_000_000 };
    // Pinned byte for byte: this layout is the contract a hardware Provider meets.
    assert_eq!(
        overrun.to_hot(),
        [0, 0x50, 0xc3, 0, 0, 0, 0, 0, 0, 0x80, 0xf0, 0xfa, 0x02, 0, 0, 0, 0]
    );
    for payload in [
        overrun,
        RxOverflowPayload { cause: RxOverflowCause::Sequence, lost: u64::MAX, restart_gap_ns: i64::MIN },
        RxOverflowPayload { cause: RxOverflowCause::Overrun, lost: 0, restart_gap_ns: -1 },
    ] {
        let bytes = payload.to_hot();
        assert_eq!(bytes.len(), 17);
        assert_eq!(RxOverflowPayload::from_hot(&bytes), Ok(payload));
    }
    let bytes = overrun.to_hot();
    assert!(RxOverflowPayload::from_hot(&bytes[..16]).unwrap_err().starts_with("RM-24"));
    let mut long = bytes.to_vec();
    long.push(0);
    assert!(RxOverflowPayload::from_hot(&long).unwrap_err().starts_with("RM-24"));
    let mut bad_cause = bytes;
    bad_cause[0] = 2;
    assert!(RxOverflowPayload::from_hot(&bad_cause).unwrap_err().starts_with("RM-24"));
}

#[test]
fn rm_24_a_delivered_payload_reads_in_either_form() {
    let payload = RxOverflowPayload { cause: RxOverflowCause::Sequence, lost: 2_000, restart_gap_ns: 0 };
    // The control path's object (RM-11) and the Kernel drain's byte array (RS-34).
    assert_eq!(RxOverflowPayload::from_payload(&serde_json::to_value(payload).unwrap()), Ok(payload));
    let drained = JsonValue::Array(payload.to_hot().iter().map(|byte| json!(byte)).collect());
    assert_eq!(RxOverflowPayload::from_payload(&drained), Ok(payload));
    // The drained array is the committed hot-form schema's shape (RM-24's reader schema).
    let hot: ezsdr_radio::payloads::RxOverflowHotPayload = serde_json::from_value(drained.clone()).unwrap();
    assert_eq!(hot.0, payload.to_hot());
    assert!(serde_json::from_value::<ezsdr_radio::payloads::RxOverflowHotPayload>(json!([0, 1, 2])).is_err());
    let mut not_a_byte = drained.clone();
    not_a_byte[3] = json!(256);
    assert!(RxOverflowPayload::from_payload(&not_a_byte).unwrap_err().starts_with("RM-24"));
    assert!(RxOverflowPayload::from_payload(&json!("overrun")).unwrap_err().starts_with("RM-24"));
}
