//! The Manifest envelope and `ArtifactRef` —
//! `04-run-and-session.md` RS-38…RS-47 (Vision §50, §51).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::binding::AdmissionResult;
use crate::event::{CounterRow, Event, EventKind};
use crate::hash::{ContentHash, HashError};
use crate::id::RunId;
use crate::module_api::{ExecutionClass, Fidelity, ModuleRef, ProfileRef, Version};
use crate::plan::{ExecutionPlan, PrepareReport};
use crate::run::{CleanupFailure, Lease, StopCause, Termination, TransitionRecord};
use crate::spec::{Ident, Namespace};
use crate::stream::ContinuityMap;
use crate::time::{ClockDomain, ClockRelation, SampleClockRecord, TimePoint};

/// Large data is always by reference: a locator, a content hash and a size, never
/// the bytes (RS-44, Vision §50, §51).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ArtifactRef {
    /// The artifact's name inside its Run (SB-17).
    pub id: Ident,
    /// Its kind; a Sink Module's namespace (MA-25).
    pub kind: Namespace,
    /// Where it lives (RS-44).
    pub uri: String,
    /// The hash of its bytes (RS-45).
    pub hash: ContentHash,
    /// How big it is (RS-44).
    pub size_bytes: u64,
    /// True when it was still open at cleanup, or the Run aborted (RS-26, RS-44).
    pub partial: bool,
    /// What `mark_artifact` recorded against it (RS-30).
    #[serde(default)]
    pub marks: Vec<ArtifactMark>,
    /// The continuity metadata of SC-30, in SampleClock order. There is more than
    /// one whenever the capture spans a rate change (TM-13c) or a channel-count
    /// change (SC-30a): each ends one map and starts the next (RS-40).
    #[serde(default)]
    pub continuity: Vec<ContinuityMap>,
}

/// One `mark_artifact` reaction recorded against an artifact (RS-30).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ArtifactMark {
    /// Which kind triggered it (RS-30).
    pub kind: EventKind,
    /// When (RS-30).
    pub time: TimePoint,
}

/// The `run` section of the envelope (RS-38).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RunSection {
    /// Which Run (RS-1).
    pub id: RunId,
    /// Spec Run or Session (RS-1, RS-12).
    pub kind: RunKind,
    /// The parent Run, for a child (RS-25).
    pub parent: Option<RunId>,
    /// Fixed at binding resolution and never changed during the Run (RS-42).
    pub execution_class: ExecutionClass,
    /// The weakest value per aspect over the bound Providers (RS-41, MA-42).
    pub fidelity: Fidelity,
    /// Every state transition, with its runtime instant and host UTC time (RS-5).
    pub transitions: Vec<TransitionRecord>,
    /// May be claimed only for the Simulation class with a recorded seed (RS-42).
    pub deterministic: bool,
}

/// What kind of Run this is (RS-1, RS-12).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunKind {
    /// Driven by an explicit ExperimentSpec.
    Spec,
    /// A Session, whose Spec is implicit and hashed like any other (RS-12).
    Session,
}

/// The `spec` section: the body after migration, its hash, and — when it was
/// migrated — the original version and hash (RS-38, SB-49).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SpecSection {
    /// The hash of the body after migration (RS-45).
    pub hash: ContentHash,
    /// The body itself (RS-38).
    pub body: serde_json::Value,
    /// The version the document declared before migration (SB-49).
    pub original_version: Option<u32>,
    /// Its hash before migration (SB-49).
    pub original_hash: Option<ContentHash>,
}

/// The `binding` section: the BindingProfile body including its `environment`,
/// recorded verbatim (RS-38, SB-27).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BindingSection {
    /// The hash of the body (RS-45).
    pub hash: ContentHash,
    /// The body, environment included, byte for byte (SB-27, RS-43).
    pub body: serde_json::Value,
}

/// One Module the Run used (RS-38).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ModuleEntry {
    /// Which Module and version (MA-31).
    pub module: ModuleRef,
    /// Its code's hash (RS-45).
    pub impl_hash: Option<ContentHash>,
    /// Its declared profile (Vision §33).
    pub profile: Option<ProfileRef>,
}

/// The `clocks` section (RS-38, TM-13d, TM-18).
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ClocksSection {
    /// Every registered domain (TM-12).
    #[serde(default)]
    pub domains: Vec<ClockDomain>,
    /// Every relation, including the Run's root to UTC with its uncertainty (TM-18).
    #[serde(default)]
    pub relations: Vec<ClockRelation>,
    /// One record per SampleClock, in order, per stream (TM-13d).
    #[serde(default)]
    pub sample_clocks: Vec<SampleClockRecord>,
}

/// The `events` section: the complete counter table, including rows whose count is
/// zero, and the bodies that were delivered (RS-33, RS-38).
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct EventsSection {
    /// Every `(source, kind)` row reachable in the plan (RS-33).
    #[serde(default)]
    pub counters: Vec<CounterRow>,
    /// The bodies that survived the ring (RS-34).
    #[serde(default)]
    pub delivered: Vec<Event>,
}

/// The `termination` section (RS-3, RS-8a, RS-10).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TerminationSection {
    /// How it ended (RS-3).
    pub reason: Termination,
    /// When, in Run time (RS-5).
    pub at: Option<TimePoint>,
    /// When, in UTC nanoseconds (RS-5).
    pub host_utc_nanos: i64,
    /// Steps that failed or timed out; every step is attempted even when an earlier
    /// one failed (RS-6, RS-8a).
    #[serde(default)]
    pub cleanup_failures: Vec<CleanupFailure>,
    /// An abort raised while an orderly stop was in progress is recorded here
    /// alongside the original cause (RS-10).
    #[serde(default)]
    pub also: Vec<StopCause>,
}

/// The `prepare` section (RS-38, SB-41).
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PrepareSection {
    /// One report per fragment (SB-41).
    #[serde(default)]
    pub reports: Vec<PrepareReport>,
    /// The merged effective configuration (SB-41).
    #[serde(default)]
    pub merged_effective: BTreeMap<crate::spec::Key, crate::spec::Value>,
}

/// A Kernel envelope with namespaced Module sections. The Kernel writes the
/// envelope; each Module writes its own section; no Provider-specific field ever
/// requires a Kernel change.
///
/// Vision §50 lists "random seeds" and an optional environment capture among the
/// envelope's contents; neither is an envelope field here. The Kernel owns no
/// random number generator, so a seed is something the environment declared, and
/// the environment is already recorded verbatim under `binding.body`; the
/// environment capture has no Kernel-defined content at all, so it belongs under
/// `sections` as `ezsdr.capture` (RS-43).
///
/// Rule: RS-38…RS-47. Vision §50, Finding 19.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Manifest {
    /// Mandatory document major, as on every other Kernel document; Phase 1 supports
    /// exactly `{1}` and [`crate::spec::check_version`] is what refuses another
    /// (RS-38, SB-47, SB-48). Vision §10 requires it of the Manifest by name.
    pub version: u32,
    /// Identity, class, fidelity and the transition sequence (RS-38).
    pub run: RunSection,
    /// The Spec after migration, with its hashes (RS-38, SB-49).
    pub spec: SpecSection,
    /// The BindingProfile, environment included (RS-38, SB-27).
    pub binding: BindingSection,
    /// The plan as bound (RS-38).
    pub plan: Option<ExecutionPlan>,
    /// The prepare reports and the merged effective configuration (SB-41).
    #[serde(default)]
    pub prepare: PrepareSection,
    /// What `validate()` matched, rejected and refused (SB-38).
    #[serde(default)]
    pub admission: AdmissionResult,
    /// The Modules and Vocabulary versions in use (RS-38).
    #[serde(default)]
    pub modules: Vec<ModuleEntry>,
    /// Vocabulary namespace to version (RS-38).
    #[serde(default)]
    pub vocabularies: BTreeMap<Namespace, Version>,
    /// Component name to implementation hash (RS-45).
    #[serde(default)]
    pub components: BTreeMap<Ident, ContentHash>,
    /// Artifacts the Run consumed, by reference; a client waveform is ingested here
    /// before the Action referencing it is admitted (RS-44a).
    #[serde(default)]
    pub inputs: Vec<ArtifactRef>,
    /// Domains, relations and SampleClock records (TM-13d, TM-18).
    #[serde(default)]
    pub clocks: ClocksSection,
    /// Counters and delivered bodies (RS-33).
    #[serde(default)]
    pub events: EventsSection,
    /// How the Run was held (RS-21).
    pub lease: Lease,
    /// The Session action log; absent for a Spec Run (RS-15).
    #[serde(default)]
    pub action_log: Vec<crate::session::LogEntry>,
    /// How it ended (RS-3).
    pub termination: TerminationSection,
    /// The artifacts it produced (RS-44).
    #[serde(default)]
    pub artifacts: Vec<ArtifactRef>,
    /// Namespaced Module sections, including `ezsdr.capture` when the profile asks
    /// (RS-38, RS-39, RS-43).
    #[serde(default)]
    pub sections: BTreeMap<Namespace, serde_json::Value>,
    /// The Manifest's own hash, computed over the Manifest with this field removed
    /// and stored beside it, never inside the hashed body. Skipped when absent, so
    /// that the hashed text does not carry `"hash": null` — a consumer implementing
    /// RS-46 as written would otherwise compute a different digest (RS-46, OV-17).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<ContentHash>,
}

/// `mark_artifact` records the kind and time against every artifact open at that
/// moment, and those marks appear in the `ArtifactRef`.
///
/// Rule: RS-30.
pub fn mark_open_artifacts(open: &mut [ArtifactRef], kind: EventKind, time: TimePoint) {
    // Every artifact open at that moment, with no exception: MA-26 makes `partial`
    // the ordinary state of an artifact still open on an abort.
    for a in open.iter_mut() {
        a.marks.push(ArtifactMark { kind: kind.clone(), time });
    }
}

/// Ingests a client-supplied waveform as an input artifact **before** the Action
/// that references it is admitted: its bytes are stored, hashed by RS-45 and listed
/// in `inputs`, and the Action carries the resulting reference. An Action whose
/// reference resolves to nothing is rejected.
///
/// Vision §58 #13 requires the Easy API alone to produce a Manifest carrying the
/// waveform hash, and without this nothing turns the client's array into something
/// hashable.
///
/// Rule: RS-44a, RS-45.
pub fn ingest_input(
    id: Ident,
    kind: Namespace,
    uri: impl Into<String>,
    bytes: &[u8],
) -> ArtifactRef {
    ArtifactRef {
        id,
        kind,
        uri: uri.into(),
        hash: ContentHash::of_bytes(bytes),
        size_bytes: bytes.len() as u64,
        partial: false,
        marks: Vec::new(),
        continuity: Vec::new(),
    }
}

impl Manifest {
    /// Computes the Manifest's own hash over the body with `hash` removed, and
    /// stores it beside the body (RS-46, OV-17).
    pub fn seal(&mut self) -> Result<ContentHash, HashError> {
        // RS-42: `deterministic` may be claimed only for the Simulation class. The
        // Manifest is the artifact the claim lands in, so this is where it is
        // enforced rather than trusted.
        self.run.deterministic =
            self.run.deterministic && self.run.execution_class.may_claim_determinism();
        self.hash = None;
        let hash = ContentHash::of(&self)?;
        self.hash = Some(hash.clone());
        Ok(hash)
    }

    /// Records a Module's section, refusing a write outside its own registered
    /// namespace. The transmit burst records of SC-28 and a Provider's envelope go
    /// in that Module's section, not in the envelope (RS-39).
    pub fn write_section(
        &mut self,
        owner: &Namespace,
        section: Namespace,
        content: serde_json::Value,
    ) -> Result<(), crate::run::RunError> {
        if !section.is_under(owner) {
            return Err(crate::run::RunError::SectionNamespaceForbidden {
                ns: owner.to_string(),
            });
        }
        self.sections.insert(section, content);
        Ok(())
    }
}
