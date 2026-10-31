use ezsdr_kernel::spec::Scalar;
use std::path::PathBuf;

use ezsdr_kernel::binding::AdmissionCheckRegistry;
use ezsdr_kernel::event::{Action, EventKind, Severity};
use ezsdr_kernel::id::ClockDomainId;
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
    assert_eq!(kinds.get(&EventKind::parse("sink.REQUEST_REJECTED").unwrap()), Some(event));
    // Phase 6, VD-1 (HD-16).
    let written = &descriptor.event_kinds[1];
    assert_eq!(written.kind.as_str(), ezsdr_sink::CAPTURE_WRITTEN);
    assert_eq!(written.default, Reaction::Continue);
    assert_eq!(written.severity, Severity::Info);
    assert_eq!(kinds.get(&written.kind), Some(written));
    // RS-14, RS-52 (pre-freeze item 3): the verb names its key and states no class;
    // the class comes from the `sink.capture_samples` declaration at admission.
    assert_eq!(descriptor.verbs.len(), 1);
    assert_eq!(descriptor.verbs[0].verb.as_str(), "capture");
    assert!(matches!(
        &descriptor.verbs[0].compiles_to,
        CompileRule::UpdateParameter { key } if key.as_str() == "sink.capture_samples"
    ));
}

/// What one dispatched Action carried, as the Sink read it (MA-14).
type Received = (Action, Option<ezsdr_kernel::id::ResourceId>);

/// A Sink double that drains what it was dispatched, so a test can read it back
/// (MA-14).
struct DrainSink {
    descriptor: ezsdr_kernel::module_api::SinkDescriptor,
    actions: Option<std::sync::Arc<dyn ezsdr_kernel::module_api::ActionReceiver>>,
    received: std::sync::Arc<std::sync::Mutex<Vec<Received>>>,
}

impl DrainSink {
    fn drain(&mut self) {
        let Some(actions) = &self.actions else {
            return;
        };
        while let Some(dispatched) = actions.recv() {
            self.received.lock().unwrap_or_else(|e| e.into_inner()).push((dispatched.action, dispatched.node));
        }
    }
}

impl ezsdr_kernel::module_api::Sink for DrainSink {
    fn descriptor(&self) -> &ezsdr_kernel::module_api::SinkDescriptor {
        &self.descriptor
    }

    fn prepare(&mut self, _f: &ezsdr_kernel::plan::Fragment, ctx: ezsdr_kernel::module_api::PrepareContext) -> Result<ezsdr_kernel::plan::PrepareReport, ezsdr_kernel::module_api::ModuleError> {
        self.actions = Some(ctx.actions);
        Ok(ezsdr_kernel::plan::PrepareReport { effective: std::collections::BTreeMap::new(), coercions: Vec::new(), warnings: Vec::new() })
    }

    fn arm(&mut self) -> Result<(), ezsdr_kernel::module_api::ModuleError> {
        Ok(())
    }

    fn start(&mut self) -> Result<(), ezsdr_kernel::module_api::ModuleError> {
        Ok(())
    }

    fn step(&mut self, _until: TimePoint) -> Result<ezsdr_kernel::module_api::StepOutcome, ezsdr_kernel::module_api::ModuleError> {
        self.drain();
        Ok(ezsdr_kernel::module_api::StepOutcome { progressed: false })
    }

    fn stop(&mut self, _mode: ezsdr_kernel::module_api::StopMode) -> Result<Vec<ezsdr_kernel::manifest::ArtifactRef>, ezsdr_kernel::module_api::ModuleError> {
        self.drain();
        Ok(Vec::new())
    }

    fn cleanup(&mut self) {
        self.drain();
    }
}

/// A Link double: the Session's output feed is a declared data link, so a
/// `LinkPlacement` and a Link object must exist for it (SB-25, KC-10). The capture
/// this test drives never publishes a block, so the link carries none.
struct NullLink {
    descriptor: ezsdr_kernel::module_api::LinkDescriptor,
}

impl NullLink {
    fn new() -> NullLink {
        NullLink {
            descriptor: ezsdr_kernel::module_api::LinkDescriptor {
                module: ezsdr_kernel::module_api::ModuleRef { id: ezsdr_kernel::id::ModuleId::parse("ezsdr.sink.test.link").unwrap(), version: Version::new(1, 0, 0) },
                kind: Namespace::parse("sink.test.link").unwrap(),
                connects: vec![(ezsdr_kernel::id::MemoryDomainId::local(0), ezsdr_kernel::id::MemoryDomainId::local(0))],
                policies: vec![ezsdr_kernel::stream::BackPressure::DropOldest],
                cross_process: false,
            },
        }
    }
}

impl ezsdr_kernel::module_api::Link for NullLink {
    fn descriptor(&self) -> &ezsdr_kernel::module_api::LinkDescriptor {
        &self.descriptor
    }

    fn create(&self, _decl: &ezsdr_kernel::stream::DataLinkDecl) -> Result<std::sync::Arc<dyn ezsdr_kernel::stream::DataLink>, ezsdr_kernel::module_api::ModuleError> {
        Ok(std::sync::Arc::new(NullDataLink))
    }
}

struct NullDataLink;

impl ezsdr_kernel::stream::DataLink for NullDataLink {
    fn publish(&self, _b: ezsdr_kernel::stream::BlockRef) -> ezsdr_kernel::stream::PublishOutcome {
        ezsdr_kernel::stream::PublishOutcome::Full
    }

    fn receive(&self) -> Option<(ezsdr_kernel::stream::BlockRef, ezsdr_kernel::stream::DropCarry)> {
        None
    }

    fn drops(&self) -> u64 {
        0
    }

    fn take_drop_carry(&self) -> ezsdr_kernel::stream::DropCarry {
        ezsdr_kernel::stream::DropCarry::default()
    }

    fn policy(&self) -> ezsdr_kernel::stream::BackPressure {
        ezsdr_kernel::stream::BackPressure::DropOldest
    }
}

/// A Provider double that declares one `rx` port and does nothing: the Session's
/// output feed must name a source port that exists (SB-17, SC-3).
struct NullProvider {
    instance: ezsdr_kernel::module_api::ProviderInstance,
}

impl NullProvider {
    fn new(contract: ezsdr_kernel::contract::DataContractId) -> NullProvider {
        let module = ezsdr_kernel::module_api::ModuleRef { id: ezsdr_kernel::id::ModuleId::parse("ezsdr.sink.test.provider").unwrap(), version: Version::new(1, 0, 0) };
        NullProvider {
            instance: ezsdr_kernel::module_api::ProviderInstance {
                id: ezsdr_kernel::id::ResourceId::parse("radio").unwrap(),
                module,
                profile: None,
                tree: ezsdr_kernel::module_api::Resource {
                    id: ezsdr_kernel::id::ResourceId::parse("radio").unwrap(),
                    kind: Namespace::parse("sink.test.radio").unwrap(),
                    capabilities: std::collections::BTreeMap::new(),
                    children: Vec::new(),
                    ports: vec![ezsdr_kernel::contract::Port { name: Ident::parse("rx").unwrap(), direction: ezsdr_kernel::contract::PortDirection::Out, contract }],
                    memory_domains: vec![ezsdr_kernel::id::MemoryDomainId::local(0)],
                    shareable: false,
                },
                fidelity: ezsdr_kernel::module_api::Fidelity::NONE,
                driving: ezsdr_kernel::module_api::Driving { stepped: true },
                arm_after: Vec::new(),
                min_command_lead: None,
                sections: std::collections::BTreeMap::new(),
            },
        }
    }
}

impl ezsdr_kernel::module_api::Provider for NullProvider {
    fn instance(&self) -> &ezsdr_kernel::module_api::ProviderInstance {
        &self.instance
    }

    fn coerce(&self, _request: &ezsdr_kernel::module_api::Requested) -> Result<ezsdr_kernel::module_api::CoerceReport, ezsdr_kernel::module_api::ModuleError> {
        Ok(ezsdr_kernel::module_api::CoerceReport::default())
    }

    fn prepare(&mut self, _f: &ezsdr_kernel::plan::Fragment, _ctx: ezsdr_kernel::module_api::PrepareContext) -> Result<ezsdr_kernel::plan::PrepareReport, ezsdr_kernel::module_api::ModuleError> {
        Ok(ezsdr_kernel::plan::PrepareReport { effective: std::collections::BTreeMap::new(), coercions: Vec::new(), warnings: Vec::new() })
    }

    fn arm(&mut self) -> Result<(), ezsdr_kernel::module_api::ModuleError> {
        Ok(())
    }

    fn start(&mut self, _at: Option<TimePoint>) -> Result<(), ezsdr_kernel::module_api::ModuleError> {
        Ok(())
    }

    fn stop(&mut self, _mode: ezsdr_kernel::module_api::StopMode) -> Result<(), ezsdr_kernel::module_api::ModuleError> {
        Ok(())
    }

    fn cleanup(&mut self) {}
}

/// A Session whose Sink is the `sink` Vocabulary's recorder, and the actions the
/// Sink was dispatched through a Run (KC-28, KC-25).
#[test]
fn hd_06_capture_reaches_the_sink_as_a_timed_update() {
    use ezsdr_kernel::coordinator::{Assembly, connect};
    use ezsdr_kernel::module_api::{AuthorityDescriptor, Pacing, Role};
    use ezsdr_kernel::run::Lease;
    use ezsdr_kernel::time::{ClockDomain, ClockRegistry, EpochRef, ManualTimeAuthority, Rational, TimeAuthority};

    let mut registry = ModuleRegistry::new();
    let mut checks = AdmissionCheckRegistry::new();
    let mut kinds = EventKindRegistry::new();
    ezsdr_sink::register(&mut registry, &mut checks, &mut kinds).unwrap();
    registry
        .register(
            ezsdr_kernel::module_api::ModuleDescriptor {
                id: ezsdr_kernel::id::ModuleId::parse("ezsdr.sink.capture").unwrap(),
                version: Version::new(1, 0, 0),
                kernel_api: Version::new(4, 0, 0),
                roles: vec![Role::Sink],
                vocabularies: Vec::new(),
                impl_hash: None,
            },
            ezsdr_kernel::module_api::Factories { sink: true, ..ezsdr_kernel::module_api::Factories::default() },
        )
        .expect("the recorder Module registers");
    // SB-22a: an output id and the `authority` name share one namespace, so the
    // Session's Authority is its own Module.
    registry
        .register(
            ezsdr_kernel::module_api::ModuleDescriptor {
                id: ezsdr_kernel::id::ModuleId::parse("ezsdr.sink.test.authority").unwrap(),
                version: Version::new(1, 0, 0),
                kernel_api: Version::new(4, 0, 0),
                roles: vec![Role::Authority],
                vocabularies: Vec::new(),
                impl_hash: None,
            },
            ezsdr_kernel::module_api::Factories { authority: true, ..ezsdr_kernel::module_api::Factories::default() },
        )
        .expect("the test Authority Module registers");
    registry
        .register(
            ezsdr_kernel::module_api::ModuleDescriptor {
                id: ezsdr_kernel::id::ModuleId::parse("ezsdr.sink.test.provider").unwrap(),
                version: Version::new(1, 0, 0),
                kernel_api: Version::new(4, 0, 0),
                roles: vec![Role::Provider],
                vocabularies: Vec::new(),
                impl_hash: None,
            },
            ezsdr_kernel::module_api::Factories { provider: true, ..ezsdr_kernel::module_api::Factories::default() },
        )
        .expect("the test Provider Module registers");
    registry
        .register(
            ezsdr_kernel::module_api::ModuleDescriptor {
                id: ezsdr_kernel::id::ModuleId::parse("ezsdr.sink.test.link").unwrap(),
                version: Version::new(1, 0, 0),
                kernel_api: Version::new(4, 0, 0),
                roles: vec![Role::Link],
                vocabularies: Vec::new(),
                impl_hash: None,
            },
            ezsdr_kernel::module_api::Factories { link: true, ..ezsdr_kernel::module_api::Factories::default() },
        )
        .expect("the test Link Module registers");
    registry.register_link_descriptor(NullLink::new().descriptor).expect("the test Link descriptor is registered under its Module");

    let clocks = std::sync::Arc::new(ClockRegistry::new());
    let root = clocks.allocate_id().unwrap();
    clocks.register(ClockDomain::root(root, Rational::new(1_000_000_000, 1).unwrap(), EpochRef::Arbitrary { set_by: "sink.test".to_owned() })).unwrap();
    let manual = std::sync::Arc::new(ManualTimeAuthority::new(clocks.clone(), root, &[], Pacing::FreeRunning).unwrap());
    struct Sim(pub std::sync::Arc<ManualTimeAuthority>, pub AuthorityDescriptor);
    impl ezsdr_kernel::module_api::Authority for Sim {
        fn descriptor(&self) -> &AuthorityDescriptor {
            &self.1
        }
        fn time(&self) -> std::sync::Arc<dyn TimeAuthority> {
            self.0.clone()
        }
        fn next_wakeup(&self) -> Option<TimePoint> {
            self.0.next_due().inspect(|t| {
                let _ = self.0.advance_to(*t);
            })
        }
    }
    let mut assembly = Assembly::new(
        Box::new(Sim(
            manual,
            AuthorityDescriptor {
                module: ezsdr_kernel::module_api::ModuleRef { id: ezsdr_kernel::id::ModuleId::parse("ezsdr.sink.test.authority").unwrap(), version: Version::new(1, 0, 0) },
                governs: vec![root, ClockDomainId::HOST_MONOTONIC],
                pacing: Pacing::FreeRunning,
            },
        )),
        clocks,
    );
    assembly.registry = registry;
    assembly.checks = checks;
    assembly.kinds = kinds;
    let contract = ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.cf32").unwrap();
    assembly.providers.insert(Ident::parse("radio").unwrap(), Box::new(NullProvider::new(contract.clone())));
    assembly.links.insert(ezsdr_kernel::module_api::ModuleRef { id: ezsdr_kernel::id::ModuleId::parse("ezsdr.sink.test.link").unwrap(), version: Version::new(1, 0, 0) }, Box::new(NullLink::new()));
    let received = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    assembly.sinks.insert(
        Ident::parse("rec").unwrap(),
        Box::new(DrainSink {
            descriptor: ezsdr_kernel::module_api::SinkDescriptor {
                module: ezsdr_kernel::module_api::ModuleRef { id: ezsdr_kernel::id::ModuleId::parse("ezsdr.sink.capture").unwrap(), version: Version::new(1, 0, 0) },
                kind: Namespace::parse("sink.recorder").unwrap(),
                memory_domains: vec![ezsdr_kernel::id::MemoryDomainId::local(0)],
                contracts: vec![contract],
                artifact_kinds: vec![Namespace::parse(ezsdr_sink::CAPTURE_ARTIFACT_KIND).unwrap()],
            },
            actions: None,
            received: received.clone(),
        }),
    );
    let profile = json!({
        "version": 1,
        "bindings": {
            "radio": {
                "module": { "id": "ezsdr.sink.test.provider", "version": { "major": 1, "minor": 0, "patch": 0 } }
            },
            "rec": {
                "module": { "id": "ezsdr.sink.capture", "version": { "major": 1, "minor": 0, "patch": 0 } },
                "feed": {
                    "port": { "component": "radio", "port": "rx" },
                    "policy": "drop_oldest", "capacity": 4
                }
            },
            "auth": {
                "module": { "id": "ezsdr.sink.test.authority", "version": { "major": 1, "minor": 0, "patch": 0 } }
            }
        },
        "authority": "auth",
        "placements": {
            "links": [{
                "link": { "id": "ezsdr.sink.test.link", "version": { "major": 1, "minor": 0, "patch": 0 } },
                "from": { "component": "radio", "port": "rx" },
                "to": { "component": "rec", "port": "in" }
            }]
        }
    });
    let mut run = connect(&profile, assembly, Lease::attached()).expect("the Session connects");
    let entry = run
        .submit(
            SessionAction::Vocabulary {
                ns: Namespace::parse("sink").unwrap(),
                verb: Ident::parse("capture").unwrap(),
                target: ezsdr_kernel::event::Target::Output { output: Ident::parse("rec").unwrap() },
                at: Some(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 100)),
                params: [(Key::parse("sink.capture_samples").unwrap(), Value::from(1000))].into_iter().collect(),
            },
            None,
        )
        .expect("the capture is well formed");
    assert!(matches!(entry.outcome, ezsdr_kernel::session::Outcome::Admitted { .. }), "{entry:?}");
    let _ = run.finish();
    let received = received.lock().unwrap_or_else(|e| e.into_inner());
    let (action, _) = received.first().expect("the Sink was dispatched the capture");
    match action {
        Action::UpdateParameter { key, value, at, class, .. } => {
            assert_eq!(key.as_str(), "sink.capture_samples");
            assert_eq!(*value, Value::Scalar(Scalar::Int(1000)));
            assert_eq!(at.as_ref().map(|d| d.time_point), Some(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 100)), "capture(n, at:) keeps its instant (RS-19)");
            // RS-52 (review P2-2): the class the Action reaches the Sink with is the
            // one `sink.capture_samples` declares, read at KC-24's step 5.
            assert_eq!(*class, ezsdr_kernel::module_api::UpdateClass::BlockBoundary, "the declaration's class, not the compiler's");
        }
        other => panic!("expected a timed UpdateParameter, got {other:?}"),
    }
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
            if file.ends_with(".json") && !schemas.keys().any(|name| ezsdr_kernel::schema::file_name(name) == file) {
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

    let payload = RequestRejectedPayload { action: ezsdr_kernel::event::ActionId(3), kind: "update_parameter".to_owned(), reason: "HD-14: N must be positive".to_owned() };
    assert_eq!(serde_json::to_value(payload).unwrap(), json!({"action": 3, "kind": "update_parameter", "reason": "HD-14: N must be positive"}));
}
