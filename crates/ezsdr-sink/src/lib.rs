//! Ez-SDR v4 Sink Vocabulary sink 1.0.0 (plan/phase2/10-host-data-path.md).
#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::collections::BTreeMap;

use ezsdr_kernel::binding::AdmissionCheckRegistry;
use ezsdr_kernel::event::{EventKind, Severity};
use ezsdr_kernel::module_api::{
    CompileRule, ModuleError, ModuleRegistry, UpdateClass, VerbDecl, Version,
    VocabularyDescriptor,
};
use ezsdr_kernel::policy::{EventKindDecl, EventKindRegistry, Reaction};
use ezsdr_kernel::spec::{CoercionPolicy, Ident, Key, KeyDecl, Namespace, ValueKind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The Sink Vocabulary id and key prefix (HD-6).
pub const VOCABULARY: &str = "sink";
/// The sample-count key requested by the `capture` verb (HD-6).
pub const CAPTURE_SAMPLES: &str = "sink.capture_samples";
/// The kind of artifact written by the capture Sink (HD-6).
pub const CAPTURE_ARTIFACT_KIND: &str = "sink.capture";
/// The event kind emitted when a capture request cannot be served (HD-14).
pub const REQUEST_REJECTED: &str = "sink.REQUEST_REJECTED";

fn sink_namespace() -> Namespace {
    Namespace::parse(VOCABULARY).expect("a valid Vocabulary namespace")
}

/// The one event kind HD-6 declares, which `vocabulary()` and `register()` both carry.
fn request_rejected() -> EventKindDecl {
    EventKindDecl {
        kind: EventKind::parse(REQUEST_REJECTED).expect("a valid Sink event kind"),
        default: Reaction::Continue,
        severity: Severity::Warning,
    }
}

/// Describes the `sink` Vocabulary 1.0.0 (HD-6).
pub fn vocabulary() -> VocabularyDescriptor {
    let namespace = sink_namespace();
    VocabularyDescriptor {
        id: namespace.clone(),
        version: Version::new(1, 0, 0),
        prefix: namespace,
        keys: vec![KeyDecl {
            key: Key::parse(CAPTURE_SAMPLES).expect("a valid Sink key"),
            kind: ValueKind::Int,
            coercible: false,
            coercion_default: CoercionPolicy::Reject,
            update_class: Some(UpdateClass::BlockBoundary),
        }],
        event_kinds: vec![request_rejected()],
        verbs: vec![VerbDecl {
            verb: Ident::parse("capture").expect("a valid Session verb"),
            compiles_to: CompileRule::UpdateParameter {
                key: Key::parse(CAPTURE_SAMPLES).expect("a valid Sink key"),
                class: UpdateClass::BlockBoundary,
            },
        }],
        checks: Vec::new(),
    }
}

/// Registers the Vocabulary and its event kind under owner `sink` (HD-6).
pub fn register(
    registry: &mut ModuleRegistry,
    _checks: &mut AdmissionCheckRegistry,
    kinds: &mut EventKindRegistry,
) -> Result<(), ModuleError> {
    registry.register_vocabulary(vocabulary())?;
    kinds
        .register(Some(sink_namespace()), request_rejected())
        .map_err(|error| ModuleError::rejected(format!("HD-6: {error}")))
}

/// Payload for a rejected capture request or unsupported Action (HD-14).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestRejectedPayload {
    /// The Action's kind tag for a rejected Action (HD-14).
    pub action: String,
    /// The reason the Sink could not carry out the request (HD-14).
    pub reason: String,
}

/// Generates the Sink Vocabulary's committed schemas (HD-14).
pub fn document_schemas() -> BTreeMap<&'static str, serde_json::Value> {
    BTreeMap::from([(
        "request_rejected_payload",
        serde_json::to_value(
            ezsdr_kernel::schema::generator()
                .into_root_schema_for::<RequestRejectedPayload>(),
        )
        .expect("a generated schema is JSON"),
    )])
}