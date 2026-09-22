//! JSON Schema generation for every document type — `00-overview.md` §6
//! (rules `OV-10`…`OV-13`).
//!
//! Rust types are the authoring source; the generated schema is committed under
//! `schemas/<name>.v<major>.json` and **is the contract**: non-Rust consumers read
//! it, and `schema_freeze` fails the build when regeneration does not reproduce it
//! byte for byte (OV-10).
//!
//! Is generating the schema still "schema-first" in the Vision's sense? Vision §10's
//! rule is that nothing is defined only as a Rust type and re-described by hand.
//! Nothing here is re-described by hand: the schema is generated, committed,
//! reviewed and is the artefact every other language reads.
//!
//! In-process types have no schema by design: `SampleBlock`, `BufferRef`, memory
//! pools and their handles, link handles, `EventSink`, `ActionReceiver`, every trait
//! object and `TimeAuthority` (X8).

use std::collections::BTreeMap;

use schemars::{JsonSchema, SchemaGenerator, generate::SchemaSettings};

/// The document major version every schema is currently generated at (OV-10, OV-12).
pub const SCHEMA_MAJOR: u32 = 1;

/// The pinned generator settings. Changing them is a deliberate change whose schema
/// diff is reviewed and whose `schema_freeze` failure is the gate (OV-11).
pub fn generator() -> SchemaGenerator {
    SchemaSettings::draft2020_12().into_generator()
}

/// The committed file name for a document type (OV-10).
pub fn file_name(name: &str) -> String {
    format!("{name}.v{SCHEMA_MAJOR}.json")
}

fn schema_of<T: JsonSchema>() -> serde_json::Value {
    serde_json::to_value(generator().into_root_schema_for::<T>())
        .expect("a generated schema is JSON")
}

/// Inserts one entry per document type. Only the inserts are generated: the function
/// is declared below, because `kernel_surface` does not read macro bodies and a public
/// item inside one is invisible to the allow-list that is the Kernel's own review
/// checklist (OV-23).
macro_rules! document_inserts {
    ($out:ident, $($name:literal => $t:ty),* $(,)?) => {
        $( $out.insert($name, schema_of::<$t>()); )*
    };
}

/// Every document type that has a committed schema, by file stem.
///
/// Serialisation conventions, so that non-Rust consumers get discriminators:
/// data-carrying enums use an internal tag, unit-only enums serialise as
/// `snake_case` strings, and namespaced opaque sections are `serde_json::Value`
/// with `additionalProperties: true` (OV-13).
///
/// Rule: OV-10, OV-13.
pub fn document_schemas() -> BTreeMap<&'static str, serde_json::Value> {
    let mut out = BTreeMap::new();
    document_inserts! {
        out,
    // Shared time definitions (TM-19).
    "clock_domain"          => crate::time::ClockDomain,
    "clock_relation"        => crate::time::ClockRelation,
    "time_point"            => crate::time::TimePoint,
    "duration"              => crate::time::Duration,
    "relative_budget"       => crate::time::RelativeBudget,
    "absolute_deadline"     => crate::time::AbsoluteDeadline,
    "sample_clock_record"   => crate::time::SampleClockRecord,
    // Stream Contract records (S18).
    "data_contract"         => crate::contract::DataContract,
    "data_link_decl"        => crate::stream::DataLinkDecl,
    "continuity_map"        => crate::stream::ContinuityMap,
    "gap"                   => crate::stream::Gap,
    "burst_record"          => crate::stream::BurstRecord,
    // Spec and binding (SB-9, SB-21).
    "experiment_spec"       => crate::spec::ExperimentSpec,
    "binding_profile"       => crate::binding::BindingProfile,
    "admission_result"      => crate::binding::AdmissionResult,
    "violation"             => crate::binding::Violation,
    "coercion"              => crate::spec::Coercion,
    "execution_plan"        => crate::plan::ExecutionPlan,
    "fragment"              => crate::plan::Fragment,
    "prepare_report"        => crate::plan::PrepareReport,
    "coerce_report"         => crate::module_api::CoerceReport,
    "requested"             => crate::module_api::Requested,
    // Module API (MA-46).
    "module_descriptor"     => crate::module_api::ModuleDescriptor,
    "vocabulary_descriptor" => crate::module_api::VocabularyDescriptor,
    "provider_instance"     => crate::module_api::ProviderInstance,
    "resource"              => crate::module_api::Resource,
    "executor_descriptor"   => crate::module_api::ExecutorDescriptor,
    "sink_descriptor"       => crate::module_api::SinkDescriptor,
    "link_descriptor"       => crate::module_api::LinkDescriptor,
    "authority_descriptor"  => crate::module_api::AuthorityDescriptor,
    "component_descriptor"  => crate::module_api::ComponentDescriptor,
    "island_decl"           => crate::module_api::IslandDecl,
    "execution_class"       => crate::module_api::ExecutionClass,
    "fidelity"              => crate::module_api::Fidelity,
    "module_error"          => crate::module_api::ModuleError,
    // Run, Session and Manifest (RS-49, RS-38).
    "action"                => crate::event::Action,
    "action_template"       => crate::event::ActionTemplate,
    "event"                 => crate::event::Event,
    "log_entry"             => crate::session::LogEntry,
    "lease"                 => crate::run::Lease,
    "policy"                => crate::policy::Policy,
    "stop_cause"            => crate::run::StopCause,
    "artifact_ref"          => crate::manifest::ArtifactRef,
    "content_hash"          => crate::hash::ContentHash,
    "manifest"              => crate::manifest::Manifest,
    }
    out
}

/// The committed form of a schema: pretty-printed JSON with a trailing newline, so
/// that a diff is readable and `schema_freeze`'s byte comparison is stable (OV-10).
pub fn render(schema: &serde_json::Value) -> String {
    let mut s = serde_json::to_string_pretty(schema).expect("a schema is JSON");
    s.push('\n');
    s
}
