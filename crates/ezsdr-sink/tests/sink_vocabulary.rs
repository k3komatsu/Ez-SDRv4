use ezsdr_kernel::spec::Scalar;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use ezsdr_kernel::binding::AdmissionCheckRegistry;
use ezsdr_kernel::event::{Action, EventKind, Severity};
use ezsdr_kernel::id::{ClockDomainId, ResourceId};
use ezsdr_kernel::module_api::{CompileRule, ModuleRegistry, UpdateClass, Version};
use ezsdr_kernel::policy::{EventKindRegistry, Reaction};
use ezsdr_kernel::session::SessionAction;
use ezsdr_kernel::spec::{CoercionPolicy, Ident, Key, Namespace, Value, ValueKind};
use ezsdr_kernel::time::TimePoint;
use ezsdr_sink::RequestRejectedPayload;
use serde_json::json;

fn schemas_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/sink")
}

#[test]
fn hd_06_vocabulary() {
    let mut registry = ModuleRegistry::new();
    let mut checks = AdmissionCheckRegistry::new();
    let mut kinds = EventKindRegistry::new();
    ezsdr_sink::register(&mut registry, &mut checks, &mut kinds).unwrap();

    let descriptor = registry.vocabulary(&Namespace::parse("sink").unwrap()).unwrap();
    assert_eq!(descriptor.id, Namespace::parse("sink").unwrap());
    assert_eq!(descriptor.version, Version::new(1, 1, 0));
    assert_eq!(descriptor.prefix, Namespace::parse("sink").unwrap());
    assert!(descriptor.checks.is_empty());
    assert_eq!(descriptor.keys.len(), 1);
    let key = &descriptor.keys[0];
    assert_eq!(key.key.as_str(), "sink.capture_samples");
    assert_eq!(key.kind, ValueKind::Int);
    assert!(!key.coercible);
    assert_eq!(key.coercion_default, CoercionPolicy::Reject);
    assert_eq!(key.update_class, Some(UpdateClass::BlockBoundary));
    assert_eq!(descriptor.event_kinds.len(), 2);
    let event = &descriptor.event_kinds[0];
    assert_eq!(event.kind.as_str(), "sink.REQUEST_REJECTED");
    assert_eq!(event.default, Reaction::Continue);
    assert_eq!(event.severity, Severity::Warning);
    assert_eq!(
        kinds.get(&EventKind::parse("sink.REQUEST_REJECTED").unwrap()),
        Some(event)
    );
    // Phase 6, VD-1 (HD-16).
    let written = &descriptor.event_kinds[1];
    assert_eq!(written.kind.as_str(), ezsdr_sink::CAPTURE_WRITTEN);
    assert_eq!(written.default, Reaction::Continue);
    assert_eq!(written.severity, Severity::Info);
    assert_eq!(kinds.get(&written.kind), Some(written));
    assert_eq!(descriptor.verbs.len(), 1);
    assert_eq!(descriptor.verbs[0].verb.as_str(), "capture");
    assert!(matches!(
        &descriptor.verbs[0].compiles_to,
        CompileRule::UpdateParameter { key, class }
            if key.as_str() == "sink.capture_samples" && *class == UpdateClass::BlockBoundary
    ));

    let action = SessionAction::Vocabulary {
        ns: Namespace::parse("sink").unwrap(),
        verb: Ident::parse("capture").unwrap(),
        target: ResourceId::parse("rec").unwrap(),
        at: None,
        params: [
            (Key::parse("sink.capture_samples").unwrap(), Value::from(1000)),
        ]
        .into_iter()
        .collect(),
    };
    let sinks: BTreeSet<Ident> = [Ident::parse("rec").unwrap()].into_iter().collect();
    let compiled = ezsdr_kernel::session::compile(
        &action,
        &registry,
        &BTreeMap::new(),
        &sinks,
        TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0),
        None,
    )
    .unwrap();
    assert_eq!(compiled.actions.len(), 1);
    assert!(matches!(
        &compiled.actions[0],
        Action::UpdateParameter { target, key, value: Value::Scalar(Scalar::Int(1000)), class, .. }
            if target == &ResourceId::parse("sink/rec").unwrap()
                && key.as_str() == "sink.capture_samples"
                && *class == UpdateClass::BlockBoundary
    ));
}

#[test]
fn hd_14_schema_freeze() {
    let dir = schemas_dir();
    let update = std::env::var("EZSDR_UPDATE_SCHEMAS").is_ok();
    if update {
        std::fs::create_dir_all(&dir).expect("schemas/sink is writable");
    }
    let schemas = ezsdr_sink::document_schemas();
    let mut stale = Vec::new();
    if !update {
        for entry in std::fs::read_dir(&dir).expect("schemas/sink is readable") {
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
            std::fs::write(&path, &rendered).expect("schemas/sink is writable");
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
    assert!(stale.is_empty(), "HD-14: schema freeze failed:\n{}", stale.join("\n"));

    let payload = RequestRejectedPayload {
        action: "capture".to_owned(),
        reason: "HD-14: N must be positive".to_owned(),
        request: None,
    };
    assert_eq!(
        serde_json::to_value(payload).unwrap(),
        json!({"action": "capture", "reason": "HD-14: N must be positive"})
    );
}
