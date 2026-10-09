use std::collections::BTreeMap;
use std::path::PathBuf;

use ezsdr_kernel::binding::{AdmissionCheckRegistry, CheckStage};
use ezsdr_kernel::module_api::{ModuleRegistry, Version};
use ezsdr_kernel::policy::EventKindRegistry;
use ezsdr_kernel::spec::{Ident, Key, Namespace, Value};
use ezsdr_kernel::binding::AdmissionCheck;
use ezsdr_sim::{FaultKind, SimRng};
use serde_json::{Value as JsonValue, json};

fn section(key: &str, value: JsonValue) -> BTreeMap<Namespace, JsonValue> {
    BTreeMap::from([(Namespace::parse(key).unwrap(), value)])
}

fn schemas_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/sim")
}

#[test]
fn se_01_register_adds_the_descriptor_and_the_three_checks() {
    let mut registry = ModuleRegistry::new();
    let mut checks = AdmissionCheckRegistry::new();
    let mut kinds = EventKindRegistry::new();
    ezsdr_sim::register(&mut registry, &mut checks, &mut kinds).unwrap();

    let descriptor = registry.vocabulary(&Namespace::parse("sim").unwrap()).unwrap();
    assert_eq!(descriptor.id, Namespace::parse("sim").unwrap());
    assert_eq!(descriptor.version, Version::new(1, 1, 0));
    assert_eq!(descriptor.prefix, Namespace::parse("sim").unwrap());
    assert!(descriptor.keys.is_empty());
    assert!(descriptor.event_kinds.is_empty());
    assert!(descriptor.verbs.is_empty());
    assert_eq!(
        descriptor.checks,
        ["sim.seed", "sim.faults", "sim.channel"].map(|s| Namespace::parse(s).unwrap())
    );

    let environment = BTreeMap::from([
        (Namespace::parse("sim.seed").unwrap(), json!("bad")),
        (Namespace::parse("sim.faults").unwrap(), json!({})),
        (Namespace::parse("sim.channel").unwrap(), json!([])),
    ]);
    let violations = checks.run(
        &environment,
        &BTreeMap::new(),
        &BTreeMap::new(),
        CheckStage::Validate,
    );
    assert_eq!(violations.len(), 3);
    assert_eq!(violations[0].check, Namespace::parse("sim.seed").unwrap());
    assert_eq!(violations[1].check, Namespace::parse("sim.faults").unwrap());
    assert_eq!(violations[2].check, Namespace::parse("sim.channel").unwrap());
    assert!(violations[..2].iter().all(|item| item.reason.starts_with("SE-5: ")));
    assert!(violations[2].reason.starts_with("CH-2: "));

}

#[test]
fn se_02_seed_reader() {
    assert_eq!(ezsdr_sim::seed(&BTreeMap::new()).unwrap(), 0);
    assert_eq!(ezsdr_sim::seed(&section("sim.seed", json!(42))).unwrap(), 42);
    assert_eq!(
        ezsdr_sim::seed(&section("sim.seed", json!(u64::MAX))).unwrap(),
        u64::MAX
    );
    for value in [json!(-1), json!("42"), json!(1.5)] {
        assert!(ezsdr_sim::seed(&section("sim.seed", value)).is_err());
    }
}

#[test]
fn se_03_faults_reader() {
    assert!(ezsdr_sim::faults(&BTreeMap::new()).unwrap().is_empty());
    let input = json!([
        {"at_ns": 0, "fault": "rx_overflow", "target": "radio"},
        {"at_ns": 5, "fault": "rx_sequence_error", "target": "radio"},
        {"at_ns": 10, "fault": "device_lost", "target": "radio"}
    ]);
    let entries = ezsdr_sim::faults(&section("sim.faults", input)).unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].fault, FaultKind::RxOverflow);
    assert_eq!(entries[1].fault, FaultKind::RxSequenceError);
    assert_eq!(entries[2].fault, FaultKind::DeviceLost);
    assert_eq!(entries[0].target, Ident::parse("radio").unwrap());
    assert_eq!(entries[0].at_ns, 0);

    for value in [
        json!([{"at_ns": 0, "fault": "unknown", "target": "radio"}]),
        json!([{"at_ns": 0, "fault": "rx_overflow", "target": "radio", "extra": true}]),
        json!([{"at_ns": -1, "fault": "rx_overflow", "target": "radio"}]),
        json!({"not": "an array"}),
        json!([{"at_ns": (1_u64 << 62) + 1, "fault": "rx_overflow", "target": "radio"}]),
    ] {
        assert!(ezsdr_sim::faults(&section("sim.faults", value)).is_err());
    }
    assert_eq!(
        ezsdr_sim::faults(&section(
            "sim.faults",
            json!([{"at_ns": 1_u64 << 62, "fault": "rx_overflow", "target": "radio"}])
        ))
        .unwrap()[0]
            .at_ns,
        1_u64 << 62
    );
}

#[test]
fn se_05_checks() {
    let radio = Ident::parse("radio").unwrap();
    let effective = BTreeMap::from([(radio.clone(), BTreeMap::<Key, Value>::new())]);
    let empty = BTreeMap::new();
    let valid = BTreeMap::from([
        (Namespace::parse("sim.seed").unwrap(), json!(42)),
        (
            Namespace::parse("sim.faults").unwrap(),
            json!([{"at_ns": 2, "fault": "rx_overflow", "target": "radio"}]),
        ),
    ]);
    assert!(ezsdr_sim::SeedCheck
        .check(
            &valid[&Namespace::parse("sim.seed").unwrap()],
            &effective,
            &empty,
            CheckStage::Validate
        )
        .is_empty());
    assert!(ezsdr_sim::FaultsCheck
        .check(
            &valid[&Namespace::parse("sim.faults").unwrap()],
            &effective,
            &empty,
            CheckStage::Validate
        )
        .is_empty());

    let unknown = json!([{"at_ns": 2, "fault": "rx_overflow", "target": "nobody"}]);
    let violations = ezsdr_sim::FaultsCheck.check(&unknown, &effective, &empty, CheckStage::Validate);
    assert_eq!(violations.len(), 1);
    assert!(violations[0].reason.contains("nobody"));
    assert!(violations[0].reason.starts_with("SE-5: "));

    let malformed_seed = ezsdr_sim::SeedCheck.check(&json!("bad"), &effective, &empty, CheckStage::Validate);
    assert_eq!(malformed_seed.len(), 1);
    assert_eq!(malformed_seed[0].check, Namespace::parse("sim.seed").unwrap());
}

#[test]
fn se_06_splitmix_vectors() {
    let mut first = SimRng::new(0, "");
    assert_eq!(first.next_u64(), 0xc3817c016ba4ff30);
    assert_eq!(first.next_u64(), 0x100cdaacc0bc9316);
    assert_eq!(first.next_u64(), 0x54c3a569ecf61b1b);

    let mut second = SimRng::new(42, "mock/rx");
    assert_eq!(second.next_u64(), 0xf7aebfe5ed07745b);
    assert_eq!(second.next_u64(), 0x0c111bab322a67d5);
    assert_eq!(second.next_u64(), 0xd7ecc327e164609a);

    let mut bounded = SimRng::new(1, "bounded");
    for _ in 0..10_000 {
        assert!(bounded.below(10) < 10);
    }
    assert_eq!(bounded.below(0), 0);
}

#[test]
fn se_12_schema_freeze() {
    let dir = schemas_dir();
    let update = std::env::var("EZSDR_UPDATE_SCHEMAS").is_ok();
    if update {
        std::fs::create_dir_all(&dir).expect("schemas/sim is writable");
    }
    let mut stale = Vec::new();
    let schemas = ezsdr_sim::document_schemas();
    if !update {
        for entry in std::fs::read_dir(&dir).expect("schemas/sim is readable") {
            let file = entry.expect("an entry").file_name().to_string_lossy().into_owned();
            if file.ends_with(".json") && !schemas.keys().any(|name| ezsdr_kernel::schema::file_name(name) == file) {
                stale.push(format!("{file}: no registered document generates it"));
            }
        }
    }
    for (name, schema) in schemas {
        let path = dir.join(ezsdr_kernel::schema::file_name(name));
        let rendered = ezsdr_kernel::schema::render(&schema);
        if update {
            std::fs::write(&path, &rendered).expect("schemas/sim is writable");
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(committed) => {
                if let Some(how) = ezsdr_kernel::schema::drift(&committed, &rendered) {
                    stale.push(format!("{name}: differs from the committed schema {how}"));
                }
            }
            Err(_) => stale.push(format!("{name}: no committed schema at {}", path.display())),
        }
    }
    assert!(stale.is_empty(), "SE-12: schema freeze failed:\n{}", stale.join("\n"));
}
