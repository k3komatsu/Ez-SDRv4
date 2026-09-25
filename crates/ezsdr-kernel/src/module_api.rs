//! Module API — `05-module-api.md` (rules `MA-n`).
//!
//! Three orthogonal axes: the **unit** (a Module: a crate or a process), the
//! **role** (Provider, Executor, Sink, Link, Authority) and the **deployment**
//! (InProcess, or the reserved Plugin). One Module may hold several roles;
//! `Plugin` names a deployment and never a role (MA-1).
//!
//! The execution ABI is the Executor's: the Kernel defines no `work` or `process`
//! signature and never inspects `impl` beyond its identity (MA-21).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::binding::Violation;
use crate::contract::{DataContractId, Port};
use crate::event::{Action, ActionId, EventSink};
use crate::hash::ContentHash;
use crate::id::{ClockDomainId, IslandId, MemoryDomainId, ModuleId, ResourceId, RunId};
use crate::manifest::ArtifactRef;
use crate::plan::{Fragment, PrepareReport};
use crate::policy::EventKindDecl;
use crate::spec::{
    CapabilityValue, Coercion, Constraint, Ident, Key, KeyDecl, Namespace, Value, Warning,
};
use crate::stream::{BackPressure, DataLink, DataLinkDecl};
use crate::time::{RelativeBudget, TimeAuthority, TimePoint};

// ---------------------------------------------------------------- versions

/// A release-only version. Module and Vocabulary versions carry a minor and a patch;
/// documents do not, which is decision B7 of `03-spec-and-binding.md`.
///
/// Rule: MA-33, decision B7.
#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Debug,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct Version {
    /// Breaking changes.
    pub major: u32,
    /// Additive changes.
    pub minor: u32,
    /// Fixes.
    pub patch: u32,
}

impl Version {
    /// A version (MA-33).
    pub const fn new(major: u32, minor: u32, patch: u32) -> Version {
        Version {
            major,
            minor,
            patch,
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// A caret requirement over a [`Version`] (MA-33).
#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Debug,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(transparent)]
pub struct VersionReq(pub Version);

impl VersionReq {
    /// The caret rule: `^a.b.c` is satisfied by `x.y.z` when `a == x` and, for
    /// `a > 0`, `(y, z) >= (b, c)`; for `a == 0`, `y == b` and `z >= c`.
    ///
    /// Rule: MA-33.
    pub fn matches(self, v: Version) -> bool {
        let r = self.0;
        if r.major != v.major {
            return false;
        }
        if r.major > 0 {
            (v.minor, v.patch) >= (r.minor, r.patch)
        } else {
            v.minor == r.minor && v.patch >= r.patch
        }
    }
}

// ---------------------------------------------------------------- axes

/// The five roles. There is exactly one trait each and no shared lifecycle
/// supertrait: one trait forced onto radios and executors alike is what audit
/// Finding 22 rejects, and Link and Authority have no lifecycle at all.
///
/// Rule: MA-1, MA-2.
#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Debug,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Implements a Resource Model.
    Provider,
    /// Implements a processing engine.
    Executor,
    /// Writes Artifacts.
    Sink,
    /// Implements a DataLink.
    Link,
    /// Implements the Time Authority.
    Authority,
}

impl Role {
    /// The stepping order's role rank: Provider before Executor before Sink (MA-30).
    pub fn step_rank(self) -> u8 {
        match self {
            Role::Provider => 0,
            Role::Executor => 1,
            Role::Sink => 2,
            Role::Link => 3,
            Role::Authority => 4,
        }
    }
}

/// How a Module is deployed. `Plugin` names a deployment, never a role, and is
/// reserved: Phase 1 refuses it as `Unsupported` (MA-1, MA-32, MA-46).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Deployment {
    /// A Rust trait, compiled in. Rust has no stable ABI, so this is the default
    /// and out-of-process is the Plugin path (Vision §62).
    InProcess {},
    /// Out-of-process, a typed protocol; reserved (MA-32, MA-46).
    Plugin {
        /// The protocol version.
        protocol: Version,
    },
}

// ---------------------------------------------------------------- class and fidelity

/// How a Time Authority paces its primary root; cross-checked against the derived
/// [`ExecutionClass`] by MA-41 (MA-29, TM-16a1).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
pub enum Pacing {
    /// Virtual time advances as fast as the model allows: the Simulation class.
    FreeRunning,
    /// Virtual time is paced against the wall clock: RealtimeEmulation, the class
    /// that exists to expose real deadlines (TM-16a1).
    WallPaced,
    /// A device timekeeper drives the Run: HardwareInLoop or Hardware.
    Device,
}

/// What the Run's RF path is, as the environment declares it (MA-41).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
pub enum RfPath {
    /// Entirely modelled.
    Simulated,
    /// Cabled, or partly simulated.
    Cabled,
    /// Over the air.
    OverTheAir,
}

/// What kind of Run this is, derived from the environment and cross-checked against
/// the Authority's [`Pacing`]; a class that was merely declared could lie.
///
/// Rule: MA-41, RS-42. Vision §14.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionClass {
    /// Simulation Engine, free-running, simulated RF path.
    Simulation,
    /// Simulation Engine, wall-paced, simulated RF path.
    RealtimeEmulation,
    /// Device timekeeper, cabled or partly simulated RF path.
    HardwareInLoop,
    /// Device timekeeper, over the air.
    Hardware,
}

impl ExecutionClass {
    /// MA-41's table. A simulated Authority cannot drive a real RF path.
    ///
    /// Rule: MA-41.
    pub fn derive(pacing: Pacing, rf: RfPath) -> Result<ExecutionClass, ModuleError> {
        Ok(match (pacing, rf) {
            (Pacing::FreeRunning, RfPath::Simulated) => ExecutionClass::Simulation,
            (Pacing::WallPaced, RfPath::Simulated) => ExecutionClass::RealtimeEmulation,
            (Pacing::Device, RfPath::Cabled | RfPath::Simulated) => ExecutionClass::HardwareInLoop,
            (Pacing::Device, RfPath::OverTheAir) => ExecutionClass::Hardware,
            (Pacing::FreeRunning | Pacing::WallPaced, RfPath::OverTheAir | RfPath::Cabled) => {
                return Err(ModuleError {
                    kind: ModuleErrorKind::Rejected,
                    message: "MA-41: a simulated Authority cannot drive a real RF path".to_owned(),
                    detail: serde_json::json!({ "pacing": pacing, "rf_path": rf }),
                });
            }
        })
    }

    /// `deterministic` may be claimed only for the Simulation class with a recorded
    /// seed; the other three are never deterministic (RS-42).
    pub fn may_claim_determinism(self) -> bool {
        self == ExecutionClass::Simulation
    }
}

/// The value set MA-42 gives timing and continuity: `none < envelope <
/// hardware_quirk < real`. `real` is added to Vision §14's sets, which stop at
/// `hardware_quirk` and leave a Hardware Run with nothing to record although the
/// same section says every Run records the vector.
///
/// Rule: MA-42.
#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Debug,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum EnvelopeFidelity {
    /// Not modelled at all; an `ideal` Mock profile declares this (Vision §13).
    None,
    /// A declared envelope is enforced.
    Envelope,
    /// A specific hardware quirk is emulated.
    HardwareQuirk,
    /// The real device.
    Real,
}

/// `none < grid < real` (MA-42).
#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Debug,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum CoercionFidelity {
    /// Not modelled.
    None,
    /// A declared grid is applied.
    Grid,
    /// The real device.
    Real,
}

/// `none < impairment_model < real` (MA-42).
#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Debug,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RfFidelity {
    /// Not modelled.
    None,
    /// An impairment model.
    ImpairmentModel,
    /// The real RF path.
    Real,
}

/// `none < model < real` (MA-42).
#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Debug,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum TransportFidelity {
    /// Not modelled.
    None,
    /// A transport model.
    Model,
    /// The real transport.
    Real,
}

/// What a Run's models actually claim, per aspect. A Run's value per aspect is the
/// **weakest** over its bound Providers, so a Hardware Run with a best-effort
/// MockPeripheral records `timing: envelope`.
///
/// Rule: MA-42, RS-41. Vision §14.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Fidelity {
    /// How faithfully timing is modelled.
    pub timing: EnvelopeFidelity,
    /// How faithfully continuity and gaps are modelled.
    pub continuity: EnvelopeFidelity,
    /// How faithfully coercion is modelled.
    pub coercion: CoercionFidelity,
    /// How faithfully the RF path is modelled.
    pub rf: RfFidelity,
    /// How faithfully transport is modelled.
    pub transport: TransportFidelity,
}

impl Fidelity {
    /// Every aspect `none`, which is what an `ideal` Mock profile declares and what
    /// a Run with no Providers records (MA-42, RS-41).
    pub const NONE: Fidelity = Fidelity {
        timing: EnvelopeFidelity::None,
        continuity: EnvelopeFidelity::None,
        coercion: CoercionFidelity::None,
        rf: RfFidelity::None,
        transport: TransportFidelity::None,
    };

    /// The weakest value per aspect, in that aspect's own order. A Run in which no
    /// Provider declares an aspect records `none` for it (MA-42, RS-41).
    pub fn weakest(declared: &[Fidelity]) -> Fidelity {
        declared.iter().fold(
            declared.first().copied().unwrap_or(Fidelity::NONE),
            |a, b| Fidelity {
                timing: a.timing.min(b.timing),
                continuity: a.continuity.min(b.continuity),
                coercion: a.coercion.min(b.coercion),
                rf: a.rf.min(b.rf),
                transport: a.transport.min(b.transport),
            },
        )
    }
}

// ---------------------------------------------------------------- descriptors

/// How a parameter may be changed while a Run is live (Vision §27).
///
/// Rule: MA-24, RS-52.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
pub enum UpdateClass {
    /// The target stops the affected function, applies the value and restarts it; a
    /// stream continues on a new SampleClock (UC-3, TM-13c).
    Cold,
    /// Applied from the first block or sample boundary at or after the effective
    /// instant; no block mixes the old and new value (UC-4).
    BlockBoundary,
    /// Applied to every sample processed after the Action arrives, with no torn state
    /// (UC-5).
    AtomicRealtime,
    /// Applied by the device at exactly the effective instant, which must respect the
    /// device's command lead (UC-6).
    HardwareTimed,
}

/// Every update class there is; an Action outside the set is rejected at
/// admission (MA-37, RS-17).
pub const UPDATE_CLASSES: &[UpdateClass] = &[
    UpdateClass::Cold,
    UpdateClass::BlockBoundary,
    UpdateClass::AtomicRealtime,
    UpdateClass::HardwareTimed,
];

/// One declared parameter of a component (MA-36).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ParamDecl {
    /// Which parameter (SB-2).
    pub key: Key,
    /// Its schema, opaque to the Kernel (MA-36).
    pub schema: serde_json::Value,
    /// How it may be changed (MA-24, RS-52).
    pub update_class: UpdateClass,
    /// Its default value (MA-36).
    pub default: Value,
}

/// A component's timing declaration (MA-36).
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComponentTiming {
    /// The processing budget. A descriptor never carries an `AbsoluteDeadline`:
    /// they are distinct types (TM-15) and distinct schema definitions (MA-37).
    pub budget: Option<RelativeBudget>,
    /// A hint at the block size the component prefers (MA-36).
    pub preferred_batch: Option<u32>,
    /// Whether it carries state across blocks (MA-36).
    pub stateful: bool,
    /// How many instances may run in parallel (MA-36).
    pub parallelism: Option<u32>,
}

/// What a component requires of its Executor. Requirements only: the placement
/// lives in the BindingProfile (MA-36, SB-25, re-review R21).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComponentRequires {
    /// The Executor kind, or `any`, which SB-1's `Namespace` grammar also spells (MA-36,
    /// MA-39).
    pub executor_kind: Namespace,
    /// Working memory, when it needs a declared amount (MA-36).
    pub memory_bytes: Option<u64>,
}

/// How a component's code is identified. The Kernel never inspects `impl` beyond
/// its identity (MA-21).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComponentImpl {
    /// The implementation kind; the Executor's `impl_kinds` must list it (MA-39).
    pub kind: Namespace,
    /// Its identity within that kind (MA-19b).
    pub id: String,
    /// Its content hash, which the Manifest records (RS-45, MA-37).
    pub hash: ContentHash,
}

/// What a Processor or Reactor declares about itself (MA-36).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComponentDescriptor {
    /// The component's name inside its document (SB-1).
    pub id: Ident,
    /// Whether it transforms samples or reacts to them (MA-36).
    pub kind: ComponentKind,
    /// Its ports (SC-1).
    pub ports: Vec<Port>,
    /// Its declared parameters (MA-24).
    #[serde(default)]
    pub params: Vec<ParamDecl>,
    /// Its timing declaration (MA-36).
    #[serde(default)]
    pub timing: ComponentTiming,
    /// What it requires of its Executor (MA-36).
    pub requires: ComponentRequires,
    /// How its code is identified (MA-21).
    #[serde(rename = "impl")]
    pub implementation: ComponentImpl,
}

/// What kind of component this is (MA-36).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
pub enum ComponentKind {
    /// Transforms samples.
    Processor,
    /// Reacts to samples by emitting Actions (MA-14a, Vision §19).
    Reactor,
}

impl ComponentDescriptor {
    /// Structural validation: unique port names, registered contract ids, update
    /// classes from the closed set, a finite budget, an `impl.hash` present.
    ///
    /// Rule: MA-37.
    pub fn validate(&self, contracts: &[DataContractId]) -> Result<(), ModuleError> {
        let reject = |message: String| ModuleError {
            kind: ModuleErrorKind::Rejected,
            message,
            detail: serde_json::Value::Null,
        };
        let mut seen = BTreeSet::new();
        for p in &self.ports {
            if !seen.insert(&p.name) {
                return Err(reject(format!("MA-37: duplicate port name {:?}", p.name)));
            }
            if !contracts.contains(&p.contract) {
                return Err(reject(format!(
                    "MA-37: contract {} is not registered",
                    p.contract
                )));
            }
        }
        // MA-37's "update classes from the closed set" and "an `impl.hash` present"
        // are enforced by the types: `UpdateClass` is a closed enum and
        // `ContentHash` only exists parsed, so both arrive as a deserialisation
        // refusal at the JSON boundary, which is where a non-Rust producer sends
        // them. `ma_37_update_class_and_hash_are_refused_at_the_boundary` covers it.
        if let Some(b) = self.timing.budget {
            if b.duration().ticks <= 0 {
                return Err(reject(
                    "MA-37: a declared budget must be finite and positive".to_owned(),
                ));
            }
        }
        Ok(())
    }
}

/// One node of a Provider's composite resource tree: a device with channels,
/// streams, a timekeeper, GPIO banks and sensors beneath it, each with its own
/// `ResourceId` and its own declared capabilities.
///
/// Rule: MA-10, SB-33.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Resource {
    /// This node's address in the tree (SB-3).
    pub id: ResourceId,
    /// What kind of node it is; a Spec resource's `kind` must match (SB-34).
    pub kind: Namespace,
    /// What it declares it can do (SB-6).
    #[serde(default)]
    pub capabilities: BTreeMap<Key, CapabilityValue>,
    /// Its sub-resources (SB-33).
    #[serde(default)]
    pub children: Vec<Resource>,
    /// The Ports this node carries, for a node that is a stream endpoint. A Spec
    /// links a component to a bound resource through one of these, which is how
    /// Vision §7's `PHY Processor -> SampleStream -> Radio Port` is expressible;
    /// without them `PortRef.component` could name a resource but no contract
    /// could be checked against it (MA-10, SB-15, SC-1, SC-3).
    #[serde(default)]
    pub ports: Vec<crate::contract::Port>,
    /// Whether more than one Spec resource may bind to this node. Default `false`:
    /// a channel or a stream is exclusive, while Vision §8's GPIO banks "may share
    /// the timekeeper" of §39, so which is which is the Provider's declaration and
    /// never the Kernel's knowledge (MA-10, SB-34, OV-21).
    #[serde(default)]
    pub shareable: bool,
}

impl Resource {
    /// This node and every node beneath it, depth first (SB-33, SB-34).
    pub fn walk(&self) -> Vec<&Resource> {
        self.walk_iter().collect()
    }

    /// Lazily walks this tree depth first without allocating a `Vec` per subtree.
    pub(crate) fn walk_iter(&self) -> impl Iterator<Item = &Resource> + '_ {
        let mut pending = vec![self];
        std::iter::from_fn(move || {
            let node = pending.pop()?;
            pending.extend(node.children.iter().rev());
            Some(node)
        })
    }
}

/// One bound Provider instance (MA-10).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProviderInstance {
    /// The instance's root resource id (SB-3).
    pub id: ResourceId,
    /// Which Module and version supplies it (MA-31).
    pub module: ModuleRef,
    /// The declared profile, when it has one (SB-22, Vision §33).
    pub profile: Option<ProfileRef>,
    /// The composite resource tree (MA-10, SB-33).
    pub tree: Resource,
    /// What this instance's models claim (MA-42).
    pub fidelity: Fidelity,
    /// Whether the coordinator steps it (MA-15, MA-30).
    pub driving: Driving,
    /// Instances that must be armed before this one, which is how the device that
    /// sources PPS is armed first (SB-39).
    #[serde(default)]
    pub arm_after: Vec<ResourceId>,
    /// The least lead this instance needs between receiving a timed Action and that
    /// Action's instant, in `host.monotonic`; absent means zero. The only envelope
    /// value the Kernel reads (MA-10, RS-19, KA-7).
    #[serde(default)]
    pub min_command_lead: Option<crate::time::Duration>,
    /// Namespaced Manifest content (RS-38, RS-39).
    #[serde(default)]
    pub sections: BTreeMap<Namespace, serde_json::Value>,
}

/// A Module by id and version (MA-31).
#[derive(
    Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct ModuleRef {
    /// The Module (SB-1, MA-31).
    pub id: ModuleId,
    /// Its version (MA-33).
    pub version: Version,
}

/// Whether `descriptor` is the exact Module version `module` names (SB-22, D78).
pub(crate) fn is_module(descriptor: &ModuleDescriptor, module: &ModuleRef) -> bool {
    descriptor.id == module.id && descriptor.version == module.version
}

/// A declared profile, by name and version. Vision §59 bumps a Mock profile's
/// version on measurement.
///
/// Rule: SB-22, MA-33 (decision B7).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileRef {
    /// The profile's name.
    pub name: String,
    /// Its version.
    pub version: Version,
}

/// Whether the coordinator steps this instance (MA-15, MA-30).
#[derive(
    Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct Driving {
    /// True for a model the coordinator drives; a hardware Provider never receives
    /// a `step` (MA-15).
    pub stepped: bool,
}

/// What a Provider was asked for; the input to `coerce` and the same input
/// `prepare` sees (SB-7, MA-11).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Requested {
    /// Which node of the tree (SB-34).
    pub resource: ResourceId,
    /// What was asked of it (SB-5).
    pub constraints: BTreeMap<Key, Constraint>,
}

/// A constraint the Provider refused outright (MA-11).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RejectedRequest {
    /// The key.
    pub key: Key,
    /// What was asked.
    pub requested: Constraint,
    /// Why, uninterpreted by the Kernel.
    pub reason: String,
}

/// What `coerce` returns. It is pure, deterministic and requires no hardware, which
/// is what makes `validate()` a true dry run (MA-11, Vision §52).
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CoerceReport {
    /// What the Provider would apply (SB-44).
    #[serde(default)]
    pub applied: BTreeMap<Key, Value>,
    /// What it would change, and why (SB-44).
    #[serde(default)]
    pub coercions: Vec<Coercion>,
    /// Non-fatal notes (SB-38).
    #[serde(default)]
    pub warnings: Vec<Warning>,
    /// What it refuses outright (MA-11).
    #[serde(default)]
    pub rejected: Vec<RejectedRequest>,
}

/// What an Executor declares. Admission reads `kind`, `memory_domains` and
/// `impl_kinds`, and `plan()` compares `module` with the binding; everything else is
/// opaque (MA-18, D82).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecutorDescriptor {
    /// The exact Module version this instance is; `plan()` refuses one that differs
    /// from its binding (SB-22, MA-38, D82).
    pub module: ModuleRef,
    /// The Executor kind a component's `requires.executor_kind` names (MA-39).
    pub kind: Namespace,
    /// The memory domains it can reach (MA-39).
    pub memory_domains: Vec<MemoryDomainId>,
    /// The implementation kinds it can load (MA-39).
    pub impl_kinds: Vec<Namespace>,
    /// Opaque to the Kernel (MA-18).
    #[serde(default)]
    pub capabilities: BTreeMap<Key, CapabilityValue>,
}

/// What a Sink declares (MA-25).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SinkDescriptor {
    /// The exact Module version this instance is; `validate()` refuses one that
    /// differs from its binding (SB-22, MA-25, D82).
    pub module: ModuleRef,
    /// The Sink kind (MA-25).
    pub kind: Namespace,
    /// The memory domains it can read a block from; a feed whose producer is placed
    /// elsewhere needs a selected Link that `connects` the pair (MA-25, MA-39, D81).
    pub memory_domains: Vec<MemoryDomainId>,
    /// The contracts it consumes (SC-3).
    pub contracts: Vec<DataContractId>,
    /// The artifact kinds it writes (RS-44).
    pub artifact_kinds: Vec<Namespace>,
}

/// What a Link declares. `connects` is the input to the memory-domain reachability
/// check of MA-39, and `policies` to the selection check of SB-25; `cross_process:
/// true` is refused in v4.0 (MA-28). The Kernel never plans a transfer (SB-40).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LinkDescriptor {
    /// The exact Module version this descriptor is; registration files it under this
    /// `module`, so MA-27a's equality check covers the version (MA-28, D82).
    pub module: ModuleRef,
    /// The Link kind (MA-27).
    pub kind: Namespace,
    /// Memory-domain pairs it joins (MA-39).
    pub connects: Vec<(MemoryDomainId, MemoryDomainId)>,
    /// The policies it implements (SC-19, MA-28).
    pub policies: Vec<BackPressure>,
    /// Whether it crosses a process boundary (MA-28).
    pub cross_process: bool,
}

/// What an Authority declares (MA-29).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuthorityDescriptor {
    /// The exact Module version this Authority is, which admission compares with its
    /// binding, so a profile hash names one version of the Module that picks the
    /// ExecutionClass (SB-22f, D98).
    pub module: ModuleRef,
    /// The domains it advances: a primary root, that root's derived domains,
    /// `host.monotonic`, and for a simulation Authority every root it simulates
    /// (TM-16a, MA-29).
    pub governs: Vec<ClockDomainId>,
    /// How it paces its primary root; MA-41 cross-checks it (MA-29).
    pub pacing: Pacing,
}

/// What a Module ships (MA-31).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModuleDescriptor {
    /// The Module (SB-1, MA-31).
    pub id: ModuleId,
    /// Its version (MA-33).
    pub version: Version,
    /// The Kernel API it was built against; a differing major is refused (MA-32).
    pub kernel_api: Version,
    /// The roles it holds; they must match the factories it registers (MA-31).
    pub roles: Vec<Role>,
    /// The Vocabularies it speaks (MA-32, MA-34).
    #[serde(default)]
    pub vocabularies: Vec<VocabularyRequirement>,
    /// InProcess, or the reserved Plugin (MA-32).
    pub deployment: Deployment,
    /// Its code's content hash, which the Manifest records (RS-45).
    pub impl_hash: Option<ContentHash>,
}

/// A Vocabulary a Module declares, with its caret requirement (MA-32).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VocabularyRequirement {
    /// The Vocabulary's namespace, which is also its key prefix (MA-34).
    pub id: Namespace,
    /// What versions satisfy it (MA-33).
    pub req: VersionReq,
}

/// How a Vocabulary's Session verb compiles to Kernel Actions (RS-13a, RS-14).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerbDecl {
    /// The verb (RS-13a).
    pub verb: Ident,
    /// What it compiles to; the Kernel dispatches on this and interprets nothing
    /// else (RS-14).
    pub compiles_to: CompileRule,
}

/// The compilations a Vocabulary may declare for its verbs (RS-14).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CompileRule {
    /// To an `UpdateParameter` on the named key, as `sink.capture` does (RS-14).
    UpdateParameter {
        /// Which parameter to set.
        key: Key,
        /// Under which update class (RS-52).
        class: UpdateClass,
    },
    /// To a `TxBurst`, as `radio.start_repeat` does (RS-14, SC-26).
    TxBurst {
        /// Whether the waveform repeats (SC-26).
        repeat: bool,
        /// What the burst does when its lead is short (SC-27, RS-51). Declared here
        /// rather than defaulted in `compile`, because which of the two a verb means
        /// is the Vocabulary's decision and the Kernel learns no Vocabulary word
        /// (OV-21). A Vocabulary wanting both behaviours declares two verbs.
        late_policy: crate::stream::LatePolicy,
    },
    /// To a `PeripheralCommand` carrying the verb (RS-14).
    PeripheralCommand {},
    /// To nothing: the verb is recorded in the log and dispatches no Action (RS-13).
    None {},
}

/// What the Kernel is obliged to enforce on a Vocabulary's behalf but never
/// interprets: its key declarations, its event kinds with their defaults, its
/// Session verbs and how each compiles, and its admission checks. Each of those
/// four is a place where an earlier draft had put Vocabulary content in the Kernel.
///
/// Rule: MA-35.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VocabularyDescriptor {
    /// The Vocabulary (MA-34).
    pub id: Namespace,
    /// Its version (MA-33).
    pub version: Version,
    /// The key prefix it owns (MA-34).
    pub prefix: Namespace,
    /// Its key declarations (SB-2).
    #[serde(default)]
    pub keys: Vec<KeyDecl>,
    /// Its event kinds with their defaults and severities (RS-27, RS-28).
    #[serde(default)]
    pub event_kinds: Vec<EventKindDecl>,
    /// Its Session verbs and how each compiles (RS-13a).
    #[serde(default)]
    pub verbs: Vec<VerbDecl>,
    /// The environment sections it checks (SB-29).
    #[serde(default)]
    pub checks: Vec<Namespace>,
}

/// An Island declaration: an Executor instance, the components placed on it, and
/// its optional affinity, real-time policy and preferred batch. Where it lives and
/// the requirement that every component be placed exactly once are SB-13 and SB-25.
///
/// Rule: MA-38.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IslandDecl {
    /// The Island (MA-27).
    pub id: IslandId,
    /// The Executor instance that runs it (MA-38).
    pub executor: Ident,
    /// The components placed on it (MA-38).
    pub components: Vec<Ident>,
    /// CPU affinity, when declared (MA-38).
    pub affinity: Option<Vec<u32>>,
    /// Real-time scheduling policy, when declared; MA-39 then requires a budget on
    /// every component (MA-38).
    pub rt_policy: Option<RtPolicy>,
    /// Preferred batch size (MA-38).
    pub batch: Option<u32>,
}

/// A real-time scheduling policy (MA-38).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RtPolicy {
    /// The scheduler name, uninterpreted by the Kernel.
    pub sched: String,
    /// The priority within it.
    pub priority: i32,
}

// ---------------------------------------------------------------- errors and handles

/// The five error kinds a Module may report. A fault crosses the boundary only as a
/// [`ModuleError`]: no panic crosses a trait boundary, and a foreign exception is
/// translated to a status (Vision §35).
///
/// Rule: MA-9.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
pub enum ModuleErrorKind {
    /// The request was well formed and refused.
    Rejected,
    /// The Module does not implement this at all; `deployment: Plugin` is one (MA-32).
    Unsupported,
    /// The call did not finish inside `PrepareContext.host_budget` (MA-8).
    Timeout,
    /// The device is gone; spec 04's Policy maps this to `abort` (MA-9, RS-27).
    DeviceLost,
    /// Anything else.
    Internal,
}

/// How a fault crosses a Module boundary (MA-9).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModuleError {
    /// Which kind (MA-9).
    pub kind: ModuleErrorKind,
    /// What happened, uninterpreted by the Kernel.
    pub message: String,
    /// Structured detail, uninterpreted by the Kernel.
    pub detail: serde_json::Value,
}

impl ModuleError {
    /// A `Rejected` error naming the rule it failed (MA-40).
    pub fn rejected(message: impl Into<String>) -> ModuleError {
        ModuleError {
            kind: ModuleErrorKind::Rejected,
            message: message.into(),
            detail: serde_json::Value::Null,
        }
    }
}

impl fmt::Display for ModuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for ModuleError {}

/// What one `step` did. `progressed` is true exactly when the instance consumed an
/// input or produced an output (MA-20).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StepOutcome {
    /// Whether anything moved (MA-20, MA-30).
    pub progressed: bool,
}

/// One attached end of a declared link, handed over at `prepare` (SC-19, MA-27).
#[derive(Clone)]
pub enum Endpoint {
    /// The consuming end of a stream link.
    StreamIn(Arc<dyn DataLink>),
    /// The producing end of a stream link.
    StreamOut(Arc<dyn DataLink>),
    /// The consuming end of an event queue; a Kernel handle, not a Link product (MA-28).
    EventIn,
    /// The producing end of an event queue (MA-28).
    EventOut,
}

/// A link end bound to one of the component's ports (MA-27).
#[derive(Clone)]
pub struct AttachedPort {
    /// Which component or resource the port belongs to (MA-27, KA-15).
    pub component: Ident,
    /// Which port (SC-1).
    pub port: Ident,
    /// Its end of the link (MA-27).
    pub endpoint: Endpoint,
}

/// Where a Module receives Actions. They arrive only after Kernel admission (MA-14).
pub trait ActionReceiver: Send + Sync {
    /// The next admitted Action for this Module, if any (MA-14).
    fn recv(&self) -> Option<Action>;
}

/// Where a Module **emits** an Action: it is submitted to `admit()` rather than to
/// another Module.
///
/// Without a sender the inbound queue would have no producer: Vision §5 and §19
/// define the Action set as what a Reactor emits into the real-time path, and §19's
/// worked example is a Reactor that receives a decoded packet and schedules a
/// `TxBurst`, so the reactive half of the architecture would have no interface at
/// all. Routing through `admit()` keeps invariant 42 true for a Reactor's Action
/// exactly as for a Session's.
///
/// Rule: MA-14a, MA-46.
pub trait ActionSubmitter: Send + Sync {
    /// Submits a proposed Action into `admit()` (RS-16). The admission result
    /// travels back, which is what makes the Plugin mapping of MA-46 a
    /// request-and-response stream.
    fn submit(&self, action: Action) -> Result<ActionId, Vec<Violation>>;
}

/// What a Module is handed at `prepare`. Every handle is shared, so a Module may keep
/// it and use it from `prepare` through `cleanup` (MA-5a, MA-6, MA-46).
pub struct PrepareContext {
    /// Which Run (RS-1).
    pub run: RunId,
    /// Which class, derived and cross-checked (MA-41).
    pub class: ExecutionClass,
    /// The Run's single source of "now" (TM-16a).
    pub time: Arc<dyn TimeAuthority>,
    /// The node's clock registry, through which a Provider declares and registers its
    /// SampleClocks (TM-13a, KA-2).
    pub clocks: Arc<crate::time::ClockRegistry>,
    /// Where to emit events (RS-31).
    pub events: Arc<dyn EventSink>,
    /// Where admitted Actions arrive (MA-14).
    pub actions: Arc<dyn ActionReceiver>,
    /// Where to emit an Action (MA-14a).
    pub actions_out: Arc<dyn ActionSubmitter>,
    /// The BindingProfile's `environment`, verbatim and read-only; a Module reads the
    /// sections its own Vocabularies define (MA-5a, SB-26).
    pub environment: Arc<BTreeMap<Namespace, serde_json::Value>>,
    /// The link ends bound to this fragment's ports (MA-27).
    pub links: Vec<AttachedPort>,
    /// The component descriptors in this Executor's Island, keyed by component name (MA-19).
    pub components: BTreeMap<Ident, ComponentDescriptor>,
    /// The bound on `prepare` and `arm`; a Module that cannot finish returns
    /// `Timeout` (MA-8).
    pub host_budget: RelativeBudget,
}

/// Why a Run is stopping, as it reaches a Module (MA-13, RS-9).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
pub enum StopMode {
    /// Deliver the declared tail (RS-9).
    Orderly,
    /// Do not (RS-9).
    Abort,
}

// ---------------------------------------------------------------- the five role traits

/// Implements a Resource Model.
///
/// Role traits are synchronous, object-safe and `Send`. No method is generic, none
/// returns `Self` or an opaque type, and no trait is `async`: an async trait would
/// not be object-safe without boxing and would pull an executor runtime into the
/// Kernel, which every Provider would then inherit.
///
/// Rule: MA-5, MA-10…MA-16.
pub trait Provider: Send {
    /// The composite resource tree, with capabilities declared per node, so that
    /// the matcher can bind at any node (MA-10).
    fn instance(&self) -> &ProviderInstance;
    /// Pure, deterministic and requiring no hardware: two calls with the same
    /// request return identical reports (MA-11).
    fn coerce(&self, request: &Requested) -> Result<CoerceReport, ModuleError>;
    /// Returns a report whose `coercions` equal what `coerce` returned for the same
    /// request. `effective` may narrow a declared capability and must not widen one
    /// (MA-12).
    fn prepare(&mut self, f: &Fragment, ctx: PrepareContext) -> Result<PrepareReport, ModuleError>;
    /// Reserves and synchronises, and radiates nothing (MA-13).
    fn arm(&mut self) -> Result<(), ModuleError>;
    /// Begins at that instant, or as soon as possible (MA-13).
    fn start(&mut self, at: Option<TimePoint>) -> Result<(), ModuleError>;
    /// Graceful under an orderly stop, delivering the declared tail; immediate
    /// under an abort (MA-13).
    fn stop(&mut self, mode: StopMode) -> Result<(), ModuleError>;
    /// Releases and restores baseline. Infallible and idempotent (MA-7, MA-13).
    fn cleanup(&mut self);
    /// A no-op by default; a Provider whose `driving.stepped` is true overrides it,
    /// and a hardware Provider never receives a `step` (MA-15).
    fn step(&mut self, _until: TimePoint) -> Result<StepOutcome, ModuleError> {
        Ok(StepOutcome { progressed: false })
    }
}

/// Implements a processing engine (MA-18…MA-24).
pub trait Executor: Send {
    /// What it can run (MA-18).
    fn descriptor(&self) -> &ExecutorDescriptor;
    /// Loads each component by its `impl` identity (MA-19).
    fn prepare(
        &mut self,
        island: &IslandDecl,
        ctx: PrepareContext,
    ) -> Result<PrepareReport, ModuleError>;
    /// Reserves (MA-7).
    fn arm(&mut self) -> Result<(), ModuleError>;
    /// Begins (MA-7).
    fn start(&mut self) -> Result<(), ModuleError>;
    /// Consumes every input at or before `until`, emits every output and event at
    /// or before `until`, emits nothing after it, never blocks on input, never
    /// calls `wait_until` (TM-16d), and reports `progressed` true exactly when it
    /// consumed an input or produced an output. Required, because without it a
    /// deterministic Run is a wish (MA-20).
    fn step(&mut self, until: TimePoint) -> Result<StepOutcome, ModuleError>;
    /// Stops (MA-7).
    fn stop(&mut self, mode: StopMode) -> Result<(), ModuleError>;
    /// Releases. Infallible and idempotent (MA-7).
    fn cleanup(&mut self);
}

/// Writes Artifacts (MA-25, MA-26).
pub trait Sink: Send {
    /// What it writes (MA-25).
    fn descriptor(&self) -> &SinkDescriptor;
    /// Prepares (MA-25).
    fn prepare(&mut self, f: &Fragment, ctx: PrepareContext) -> Result<PrepareReport, ModuleError>;
    /// Reserves (MA-25).
    fn arm(&mut self) -> Result<(), ModuleError>;
    /// Begins (MA-25).
    fn start(&mut self) -> Result<(), ModuleError>;
    /// Drains. Every Sink is stepped in the Simulation class and runs on a thread in
    /// the other three, so the decision is the class's and no descriptor flag
    /// carries it (MA-25, MA-30).
    fn step(&mut self, until: TimePoint) -> Result<StepOutcome, ModuleError>;
    /// Returns its artifact references even on an abort, with `partial` set. A Sink
    /// whose `stop` fails leaves its artifacts recorded as unknown (MA-26, RS-44).
    fn stop(&mut self, mode: StopMode) -> Result<Vec<ArtifactRef>, ModuleError>;
    /// Releases. Infallible and idempotent (MA-25).
    fn cleanup(&mut self);
}

/// Implements a DataLink. Links are created before the Providers, Executors and
/// Sinks are prepared, and dropped at cleanup; they have no lifecycle of their own.
///
/// Rule: MA-27, MA-28.
pub trait Link: Send + Sync {
    /// What it connects and with which policies (MA-28).
    fn descriptor(&self) -> &LinkDescriptor;
    /// Builds a link implementing the declared policy exactly (MA-27, SC-19).
    fn create(&self, decl: &DataLinkDecl) -> Result<Arc<dyn DataLink>, ModuleError>;
}

/// Implements the Time Authority. There is one instance per Run, named by the
/// BindingProfile (MA-29, SB-24).
pub trait Authority: Send + Sync {
    /// What it governs and how it paces (MA-29).
    fn descriptor(&self) -> &AuthorityDescriptor;
    /// The Run's `TimeAuthority` handle (TM-16a).
    fn time(&self) -> Arc<dyn TimeAuthority>;
    /// Advances the governed clocks to the earliest instant at which a scheduled
    /// callback is due, fires the callbacks due there in TM-16c order — including ones
    /// they schedule at that instant, up to TM-17b's cap per call — and returns the
    /// instant; with nothing scheduled it returns `None` and moves nothing (MA-29,
    /// MA-30, KA-11).
    fn next_wakeup(&self) -> Option<TimePoint>;
}

// ---------------------------------------------------------------- the stepping loop

/// One instance the coordinator steps, dispatched per role rather than through a
/// shared supertrait (MA-2, MA-30).
pub enum SteppedRef<'a> {
    /// A stepped Provider (MA-15).
    Provider(&'a mut dyn Provider),
    /// An Executor (MA-20).
    Executor(&'a mut dyn Executor),
    /// A Sink (MA-25).
    Sink(&'a mut dyn Sink),
}

impl SteppedRef<'_> {
    fn role(&self) -> Role {
        match self {
            SteppedRef::Provider(_) => Role::Provider,
            SteppedRef::Executor(_) => Role::Executor,
            SteppedRef::Sink(_) => Role::Sink,
        }
    }

    fn step(&mut self, until: TimePoint) -> Result<StepOutcome, ModuleError> {
        match self {
            SteppedRef::Provider(p) => p.step(until),
            SteppedRef::Executor(e) => e.step(until),
            SteppedRef::Sink(s) => s.step(until),
        }
    }
}

/// One entry of the coordinator's stepping table (MA-30).
pub struct SteppedInstance<'a> {
    /// The instance's name, which breaks ties within a role (MA-30).
    pub id: Ident,
    /// The instance itself (MA-30).
    pub inner: SteppedRef<'a>,
}

/// The cap on stepping rounds at one instant. A zero-latency Event or Action cycle
/// hits it; the Simulation Engine of Phase 2 adds delivery latency on inter-Island
/// Action edges, and TM-17b gives the Authority the same cap for the same reason
/// (MA-30).
pub const STEP_ROUND_CAP: usize = 1_000;

/// Steps every instance in a fixed order — role rank Provider before Executor
/// before Sink, then instance id — until no instance reports `progressed`.
///
/// The order is fixed by rule and not by registration, so determinism does not
/// depend on the order in which a runtime happened to assemble its Modules
/// (Vision §58 #3). Exceeding [`STEP_ROUND_CAP`] is `STEP_LIVELOCK`, which the
/// Policy table turns into an abort rather than a hang.
///
/// Rule: MA-30.
pub fn step_until_quiescent(
    instances: &mut [SteppedInstance<'_>],
    until: TimePoint,
    events: &dyn EventSink,
    coordinator: &ResourceId,
) -> Result<usize, ModuleError> {
    instances.sort_by(|a, b| {
        (a.inner.role().step_rank(), a.id.as_str())
            .cmp(&(b.inner.role().step_rank(), b.id.as_str()))
    });
    for round in 1..=STEP_ROUND_CAP {
        let mut progressed = false;
        for inst in instances.iter_mut() {
            progressed |= inst.inner.step(until)?.progressed;
        }
        if !progressed {
            return Ok(round);
        }
    }
    // RS-27 registers STEP_LIVELOCK as a kind the Kernel emits from "its own
    // stepping loop", and RS-28 gives it `abort` at `fatal`. Emitting it is what
    // puts it through RS-33's counters and RS-36's escalation flag; the returned
    // error is what stops this loop.
    let kind = crate::event::EventKind::parse(crate::event::EventKind::STEP_LIVELOCK)
        .expect("a Kernel kind is well formed");
    let _ = events.emit_control(crate::event::Event {
        source: coordinator.clone(),
        time: until,
        severity: crate::event::Severity::Fatal,
        kind,
        payload: serde_json::json!({ "rounds": STEP_ROUND_CAP }),
    });
    Err(ModuleError {
        kind: ModuleErrorKind::Internal,
        message: format!("MA-30: {} stepping rounds at one instant", STEP_ROUND_CAP),
        detail: serde_json::json!({ "event_kind": crate::event::EventKind::STEP_LIVELOCK }),
    })
}

// ---------------------------------------------------------------- the registry

/// Which roles a registration supplies factories for (MA-31).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Factories {
    /// A Provider factory is present.
    pub provider: bool,
    /// An Executor factory is present.
    pub executor: bool,
    /// A Sink factory is present.
    pub sink: bool,
    /// A Link factory is present.
    pub link: bool,
    /// An Authority factory is present.
    pub authority: bool,
}

impl Factories {
    /// Whether a factory for `role` is present (MA-31).
    pub fn has(self, role: Role) -> bool {
        match role {
            Role::Provider => self.provider,
            Role::Executor => self.executor,
            Role::Sink => self.sink,
            Role::Link => self.link,
            Role::Authority => self.authority,
        }
    }

    /// Every role this registration supplies (MA-31).
    pub fn roles(self) -> Vec<Role> {
        [
            Role::Provider,
            Role::Executor,
            Role::Sink,
            Role::Link,
            Role::Authority,
        ]
        .into_iter()
        .filter(|r| self.has(*r))
        .collect()
    }
}

/// The Kernel API version this crate implements; a Module whose `kernel_api.major`
/// differs is refused (MA-32).
pub const KERNEL_API: Version = Version::new(4, 0, 0);

/// The Modules and Vocabularies a runtime assembled.
///
/// Registration is explicit in the runtime's assembly code; there is no link-time
/// registration crate, whose order is opaque and whose failures are silent (MA-32).
#[derive(Default)]
pub struct ModuleRegistry {
    modules: BTreeMap<(ModuleId, Version), ModuleDescriptor>,
    vocabularies: BTreeMap<Namespace, VocabularyDescriptor>,
    link_descriptors: BTreeMap<ModuleRef, LinkDescriptor>,
}

impl ModuleRegistry {
    /// An empty registry (MA-32).
    pub fn new() -> ModuleRegistry {
        ModuleRegistry::default()
    }

    /// Registers a Vocabulary, whose keys, event kinds, verbs and checks the Kernel
    /// then enforces without interpreting them (MA-35).
    pub fn register_vocabulary(&mut self, v: VocabularyDescriptor) -> Result<(), ModuleError> {
        if self.vocabularies.contains_key(&v.id) {
            return Err(ModuleError::rejected(format!(
                "MA-32: vocabulary {} is already registered",
                v.id
            )));
        }
        // RS-14: a Session burst's target is resolved at `compile`, so `RejectAtPlan`
        // names a stage this Action never passes through and the verb would be
        // undispatchable (SC-27, RS-51).
        for verb in &v.verbs {
            if let CompileRule::TxBurst {
                late_policy: crate::stream::LatePolicy::RejectAtPlan,
                ..
            } = verb.compiles_to
            {
                return Err(ModuleError::rejected(format!(
                    "RS-14: verb {} compiles to a TxBurst with `RejectAtPlan`, which a Session \
                     burst never reaches",
                    verb.verb
                )));
            }
        }
        self.vocabularies.insert(v.id.clone(), v);
        Ok(())
    }

    /// Registers a Module. Fails when `kernel_api.major` differs from the Kernel's;
    /// a declared Vocabulary is absent or incompatible; a role has no factory or a
    /// factory no role; `(id, version)` is already registered; or `deployment` is
    /// `Plugin`, which is `Unsupported` in Phase 1.
    ///
    /// Rule: MA-31, MA-32.
    pub fn register(
        &mut self,
        d: ModuleDescriptor,
        factories: Factories,
    ) -> Result<(), ModuleError> {
        if let Deployment::Plugin { .. } = d.deployment {
            return Err(ModuleError {
                kind: ModuleErrorKind::Unsupported,
                message: "MA-32: deployment `Plugin` is reserved and unsupported in Phase 1"
                    .to_owned(),
                detail: serde_json::Value::Null,
            });
        }
        if d.kernel_api.major != KERNEL_API.major {
            return Err(ModuleError::rejected(format!(
                "MA-32: kernel_api major {} does not match the Kernel's {}",
                d.kernel_api.major, KERNEL_API.major
            )));
        }
        if self.modules.contains_key(&(d.id.clone(), d.version)) {
            return Err(ModuleError::rejected(format!(
                "MA-32: {} {} is already registered",
                d.id, d.version
            )));
        }
        for v in &d.vocabularies {
            match self.vocabularies.get(&v.id) {
                None => {
                    return Err(ModuleError::rejected(format!(
                        "MA-32: vocabulary {} is not registered",
                        v.id
                    )));
                }
                Some(reg) if !v.req.matches(reg.version) => {
                    return Err(ModuleError::rejected(format!(
                        "MA-32: vocabulary {} at {} does not satisfy ^{}",
                        v.id, reg.version, v.req.0
                    )));
                }
                Some(_) => {}
            }
        }
        for role in &d.roles {
            if !factories.has(*role) {
                return Err(ModuleError::rejected(format!(
                    "MA-31: role {role:?} has no factory"
                )));
            }
        }
        for role in factories.roles() {
            if !d.roles.contains(&role) {
                return Err(ModuleError::rejected(format!(
                    "MA-31: factory for {role:?} has no declared role"
                )));
            }
        }
        self.modules.insert((d.id.clone(), d.version), d);
        Ok(())
    }

    /// Registers the Link descriptor supplied by a registered Link Module, keyed by
    /// the `module` it declares, so a descriptor cannot be filed under another
    /// version than its own (D82). A descriptor for an unregistered Module or a
    /// Module without the Link role is refused; Phase 1 also refuses cross-process
    /// links (MA-28, D68).
    pub fn register_link_descriptor(
        &mut self,
        descriptor: LinkDescriptor,
    ) -> Result<(), ModuleError> {
        let module = descriptor.module.clone();
        if descriptor.cross_process {
            return Err(ModuleError::rejected(
                "MA-28: cross_process links are unsupported in v4.0".to_owned(),
            ));
        }
        let has_link_role = self
            .modules
            .get(&(module.id.clone(), module.version))
            .is_some_and(|registered| registered.roles.contains(&Role::Link));
        if !has_link_role {
            return Err(ModuleError::rejected(format!(
                "MA-28: {} {} is not a registered Link Module",
                module.id, module.version
            )));
        }
        if self.link_descriptors.contains_key(&module) {
            return Err(ModuleError::rejected(format!(
                "MA-28: {} {} already has a registered Link descriptor",
                module.id, module.version
            )));
        }
        self.link_descriptors.insert(module, descriptor);
        Ok(())
    }

    /// The registered Vocabulary, if any (MA-34).
    pub fn vocabulary(&self, id: &Namespace) -> Option<&VocabularyDescriptor> {
        self.vocabularies.get(id)
    }

    /// Every registered Module, in `(id, version)` order (MA-32).
    pub fn modules(&self) -> impl Iterator<Item = &ModuleDescriptor> {
        self.modules.values()
    }

    /// The registered Link descriptor selected by a BindingProfile (MA-28, SB-25).
    pub fn link_descriptor(&self, module: &ModuleRef) -> Option<&LinkDescriptor> {
        self.link_descriptors.get(module)
    }

    /// Every registered Link descriptor, keyed by Module id and version (MA-28, SB-25).
    pub fn link_descriptors(&self) -> &BTreeMap<ModuleRef, LinkDescriptor> {
        &self.link_descriptors
    }

    /// The `KeyDecl` for a key, resolved through the Vocabulary that owns its
    /// prefix. The Kernel refuses a key whose prefix belongs to no Vocabulary the
    /// declaring Module declares, naming the prefix, and validates the value's
    /// shape against the `KeyDecl`. It never interprets the meaning.
    ///
    /// Rule: MA-34, SB-2.
    pub fn key_decl(&self, key: &Key) -> Result<&KeyDecl, ModuleError> {
        self.vocabularies
            .values()
            .filter(|v| key.has_prefix(&v.prefix))
            .find_map(|v| v.keys.iter().find(|d| d.key == *key))
            .ok_or_else(|| {
                ModuleError::rejected(format!(
                    "MA-34: key {key} has a prefix no registered Vocabulary owns"
                ))
            })
    }

    /// The compilation a Vocabulary declares for one of its verbs (RS-13a, RS-14).
    pub fn verb(&self, ns: &Namespace, verb: &Ident) -> Option<&VerbDecl> {
        self.vocabularies
            .get(ns)?
            .verbs
            .iter()
            .find(|v| v.verb == *verb)
    }
}
