//! The ExecutionPlan, its dependency order, PrepareReport and Island admission —
//! `03-spec-and-binding.md` SB-37…SB-46, `05-module-api.md` MA-22, MA-39…MA-41.

mod coercion;
mod compile;
mod graph;
mod islands;
mod links;
mod matching;
mod prepare;
mod validation;

pub(crate) use validation::{binding_description, check_rid, not_registered, require_role};

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::binding::LinkPlacement;
use crate::contract::PortRef;
use crate::id::DataLinkId;
use crate::module_api::{
    ComponentDescriptor, ExecutionClass, ExecutorDescriptor, IslandDecl, LinkDescriptor,
    ModuleError, ModuleRef, Role, SinkDescriptor,
};
use crate::spec::{Coercion, CoercionPolicy, Ident, Key, KeyDecl, SpecError, Value, Warning};
use crate::stream::{BackPressure, DataLinkDecl};

/// What the runtime supplied under a slot's name (SB-22f): the only way a
/// runtime-supplied map is read, so nothing supplied under another name is (SB-22).
fn supplied<'m, V>(
    map: &'m BTreeMap<Ident, V>,
    name: &Ident,
    what: &str,
) -> Result<&'m V, SpecError> {
    map.get(name).ok_or_else(|| SpecError::Structural {
        reason: format!("SB-22f: no {what} for binding {name}"),
    })
}

/// One unit of work handed to one Module at `prepare`, in dependency order (SB-39).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Fragment {
    /// The fragment's name (SB-1).
    pub id: Ident,
    /// Which exact Module version performs it, as its binding names it (SB-22, D78).
    pub instance: ModuleRef,
    /// In which role (MA-1).
    pub role: Role,
    /// What to do; namespaced content the Kernel does not interpret (SB-39).
    pub content: serde_json::Value,
    /// Fragments that must be prepared and armed first (SB-39).
    #[serde(default)]
    pub after: Vec<Ident>,
}

/// A number the Link Module declares, not a measurement. The Core reports it and
/// never uses it to choose a placement; the Core neither measures nor optimises.
///
/// Rule: SB-40. Vision §20, §31, §63.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeclaredCost {
    /// Which link (SC-19).
    pub link: DataLinkId,
    /// The Link Module's own number, uninterpreted by the Kernel (SB-40).
    pub cost: u64,
}

/// What `plan(spec, binding)` produces (SB-39).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecutionPlan {
    /// The fragments, already in dependency order (SB-39).
    pub fragments: Vec<Fragment>,
    /// The DataLink declarations: the graph's links, then the output feeds, numbered
    /// on from them (SC-19, MA-27a, D75).
    pub links: Vec<DataLinkDecl>,
    /// The dependency edges, `(before, after)` (SB-39).
    pub deps: Vec<(Ident, Ident)>,
    /// Which binding keeps time (SB-24).
    pub authority: Ident,
    /// Derived from the environment and cross-checked against the Authority's
    /// pacing (MA-41, SB-39).
    pub class: ExecutionClass,
    /// Declared, never measured (SB-40).
    #[serde(default)]
    pub transfer_costs: Vec<DeclaredCost>,
}

/// What one fragment's `prepare` returned (SB-41, MA-12).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PrepareReport {
    /// Which fragment (SB-41).
    pub fragment: Ident,
    /// The applied configuration. `effective` may narrow a declared capability and
    /// must not widen one (MA-12).
    #[serde(default)]
    pub effective: BTreeMap<Key, Value>,
    /// What the Provider changed; these must equal what `coerce` returned for the
    /// same request (MA-12, SB-44).
    #[serde(default)]
    pub coercions: Vec<Coercion>,
    /// Non-fatal notes (SB-41).
    #[serde(default)]
    pub warnings: Vec<Warning>,
}

/// Orders fragments by their dependency edges, breaking ties by `Ident` so that a
/// plan is deterministic. A cycle is `ArmCycle`.
///
/// Dependency edges come from the explicit `ezsdr.arm_order` section and from each
/// Provider instance's declared `arm_after`, which is how the device that sources
/// PPS is armed before the devices that consume it.
///
/// Rule: SB-39, decision B6.
pub fn arm_order(nodes: &[Ident], edges: &[(Ident, Ident)]) -> Result<Vec<Ident>, SpecError> {
    graph::arm_order(nodes, edges)
}

/// The inverse of the arm order: the device that was armed first is released last
/// (RS-8).
pub fn release_order(arm_order: &[Ident]) -> Vec<Ident> {
    graph::release_order(arm_order)
}

/// The coercion policy for a key, resolved in SB-45's order: the Spec's
/// `policies.coercion` entry; then, for a Session, `warn`; then the key's
/// `coercion_default`; then `warn`.
///
/// Re-review R16 asks for the split by Run kind, and this is where it lands: an
/// interactive Session warns, while a Spec Run takes the Vocabulary's default.
///
/// Rule: SB-45.
pub fn coercion_policy(
    spec_override: Option<CoercionPolicy>,
    is_session: bool,
    decl: Option<&KeyDecl>,
) -> CoercionPolicy {
    coercion::coercion_policy(spec_override, is_session, decl)
}

/// Applies SB-46: under `reject` a coercion fails the stage; under `warn` it is
/// applied, recorded and warned; under `accept` it is applied and recorded. In all
/// three cases it appears in the PrepareReport and in the Manifest.
///
/// Rule: SB-46.
pub fn apply_coercion(
    policy: CoercionPolicy,
    coercion: &Coercion,
) -> Result<Option<Warning>, SpecError> {
    coercion::apply_coercion(policy, coercion)
}

// ---------------------------------------------------------------- island admission

/// What an Island admission check needs to know about the Run (MA-39).
pub struct IslandContext<'a> {
    /// The Islands being admitted, each listing its components with their memory
    /// domains (MA-38, SB-25).
    pub islands: &'a [IslandDecl],
    /// Every component of the Spec's graph (SB-15).
    pub components: &'a BTreeMap<Ident, ComponentDescriptor>,
    /// The Executor instance behind each Island (MA-18).
    pub executors: &'a BTreeMap<Ident, ExecutorDescriptor>,
    /// The registered Link descriptors, keyed by Module id and version (MA-28).
    pub links: &'a BTreeMap<ModuleRef, LinkDescriptor>,
    /// The selected Link Module for each data link, named by its endpoints (SB-25,
    /// D76).
    pub link_placements: &'a [LinkPlacement],
    /// The graph's links (SB-15).
    pub graph_links: &'a [(PortRef, PortRef, BackPressure)],
    /// The links feeding the Spec's outputs, whose consumer end is `{output id,
    /// "in"}`. They need a Link like any other data link; they join no component, so
    /// they take no part in the cycle check, and their consumer's memory domains
    /// are the bound Sink's (SB-17, SB-25, D75, D81).
    pub feed_links: &'a [(PortRef, PortRef, BackPressure)],
    /// The bound Sink's descriptor per output id, for the feed half of the
    /// memory-domain check (MA-25, MA-39, D81).
    pub sinks: &'a BTreeMap<Ident, SinkDescriptor>,
    /// The Spec resource names a link may name as an endpoint. A resource endpoint
    /// is a node of the graph but is placed in no Island — it lives on its bound
    /// Provider, not inside an Executor — so MA-39 asks for no placement and
    /// MA-22's cycle graph counts it as a node rather than an unknown (SB-15,
    /// MA-10; finding D31).
    pub resource_endpoints: &'a BTreeSet<Ident>,
}

/// Island admission.
///
/// Checks: every component placed exactly once; `requires.executor_kind` is `any`
/// or the Executor's kind, `impl.kind` is among its `impl_kinds`, and the memory
/// domain its Island states for it is among its `memory_domains`; every data link —
/// graph link or output feed — has exactly one `LinkPlacement` naming its ends, whose
/// selected descriptor supports its policy and, when the two ends are placed in
/// different memory domains, in one Island or in two, connects the pair; MA-22's
/// cycle rule; and an Island with an `rt_policy` requires a declared budget on every
/// component.
///
/// Admission **rejects**; it never creates, merges or moves an Island, and a
/// rejection names the rule it failed.
///
/// *The arithmetic feasibility check, that the sum of the budgets fits the block
/// period, needs declared port rates from the Radio Model; Phase 1 checks presence
/// only. Forward obligation, Phase 2.*
///
/// Rule: MA-39, MA-40.
pub fn admit_islands(ctx: &IslandContext<'_>) -> Result<(), ModuleError> {
    islands::admit_islands(ctx)
}

/// What kind of edge joins two components (MA-22).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum EdgeKind {
    /// A `stream.*` link carrying blocks (SC-19).
    Stream,
    /// An Event edge (RS-31).
    Event,
    /// An Action edge (RS-48).
    Action,
}

/// One edge of the plan's graph (MA-22).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GraphEdge {
    /// The producing component.
    pub from: Ident,
    /// The consuming component.
    pub to: Ident,
    /// What it carries (MA-22).
    pub kind: EdgeKind,
    /// Whether it crosses an Island boundary (MA-22).
    pub crosses_island: bool,
}

/// Plan admission rejects a cycle formed by `stream.*` links. A cycle is legal only
/// when every edge that closes it is an Event or Action edge crossing an Island
/// boundary. Adaptive feedback inside a component is state, not structure.
///
/// Rule: MA-22. Vision §19.
pub fn check_cycles(nodes: &[Ident], edges: &[GraphEdge]) -> Result<(), ModuleError> {
    graph::check_cycles(nodes, edges)
}

/// `effective` may narrow a declared capability and **must not** widen one; the
/// coordinator then re-runs constraint matching over `effective`, which is also
/// Vision §52's re-check of applied values.
///
/// Rule: MA-12, SB-30.
pub fn check_effective_narrows(
    declared: &crate::spec::CapabilityValue,
    effective: &crate::spec::CapabilityValue,
) -> Result<(), ModuleError> {
    prepare::check_effective_narrows(declared, effective)
}

// ---------------------------------------------------------------- the compile pipeline

use crate::binding::{AdmissionCheckRegistry, AdmissionResult, BindingProfile};
use crate::contract::ContractRegistry;
use crate::module_api::{AuthorityDescriptor, ModuleRegistry, Provider};
use crate::policy::EventKindRegistry;
use crate::spec::ExperimentSpec;

/// What the compile pipeline needs beyond the two documents.
///
/// The Kernel talks to Providers through the role trait, never through a callback:
/// `validate()` calls `instance()` and `coerce()` and nothing else, which is what
/// makes it a true dry run (MA-11, Vision §52).
///
/// Rule: SB-37, SB-38.
pub struct CompileInputs<'a> {
    /// Modules and Vocabularies (MA-32).
    pub registry: &'a ModuleRegistry,
    /// The registered admission checks (SB-29).
    pub checks: &'a AdmissionCheckRegistry,
    /// The registered event kinds, for `policies.failure` (SB-18).
    pub kinds: &'a EventKindRegistry,
    /// The bound Provider per Spec resource name (SB-22f). An entry under any other
    /// name is never read (SB-22).
    pub providers: &'a BTreeMap<Ident, &'a dyn Provider>,
    /// The Authority's descriptor, under the name `authority` gives (SB-24, SB-22f,
    /// MA-29). An entry under any other name is never read (SB-22).
    pub authorities: &'a BTreeMap<Ident, AuthorityDescriptor>,
    /// What each Island's Executor instance declares. Which **Module** supplies it
    /// is named by the profile, as `bindings[island.executor]`, so the plan and the
    /// Manifest rest on the document and not on assembly-time input (MA-18, MA-38).
    pub executors: &'a BTreeMap<Ident, ExecutorDescriptor>,
    /// The registered DataContracts, for SC-3 (SB-15).
    pub contracts: &'a ContractRegistry,
    /// The bound Sink per output id, from `bindings` (SB-17, SB-22, MA-25). A Sink
    /// is bound, never placed in an Island: it is a Module role, not a component an
    /// Executor loads.
    pub sinks: &'a BTreeMap<Ident, &'a dyn crate::module_api::Sink>,
    /// Whether this is a Session, which changes the coercion default (SB-45).
    pub is_session: bool,
}

/// The stages run in this order, which is re-review R4's correction: schema
/// validation, semantic validation, resource resolution, binding resolution,
/// capability matching against the **bound** instances, plan construction,
/// admission checks, `prepare`, `arm`. Matching cannot precede binding, because the
/// capabilities being matched are those of the instance that binding chose.
///
/// `validate` returns the matched resources, the rejected constraints, the envelope
/// violations, a preview of the coercions and the warnings. It touches no hardware.
/// Its structural and endpoint checks are functions of their own because `plan()`
/// runs them again rather than presuming that this ran (SB-39, D99).
///
/// Rule: SB-37, SB-38, SB-T4.
pub fn validate(
    spec: &ExperimentSpec,
    profile: &BindingProfile,
    inputs: &CompileInputs<'_>,
) -> Result<AdmissionResult, SpecError> {
    validation::validate(spec, profile, inputs)
}

/// `plan(spec, binding)` adds the fragments, the DataLink declarations, the
/// dependency edges, the Authority, the derived ExecutionClass and the declared
/// transfer costs.
///
/// Dependency edges come from the explicit `ezsdr.arm_order` section and from each
/// Provider instance's declared `arm_after`, which is how the device that sources
/// PPS is armed before the devices that consume it. A cycle is `ArmCycle`.
/// Fragments with no edge between them are ordered by their `Ident`, so a plan is
/// deterministic.
///
/// `plan` does not presume that `validate()` ran: it runs `validate()`'s structural
/// and endpoint checks again, through the same functions, and takes from `admission`
/// only what matching produced (SB-39, D99). A Provider fragment's `content` is
/// `{ selector, requested }`, where `requested` names the node the matcher bound and
/// the constraints the Spec asked of it: without it `prepare` receives the selector
/// alone, and MA-12's rule that a report's coercions equal what `coerce` returned
/// for the same request has no request to be about (SB-39, SB-44, MA-12).
///
/// Rule: SB-39, SB-40, SB-44, MA-12, MA-41.
pub fn plan(
    spec: &ExperimentSpec,
    profile: &BindingProfile,
    admission: &AdmissionResult,
    inputs: &CompileInputs<'_>,
    transfer_costs: Vec<DeclaredCost>,
) -> Result<ExecutionPlan, SpecError> {
    compile::plan(spec, profile, admission, inputs, transfer_costs)
}

/// Why `prepare` failed (SB-30, SB-42).
#[derive(Clone, PartialEq, Debug)]
pub enum PrepareError {
    /// A fragment failed, which fails the whole transaction (SB-42).
    Fragment {
        /// Its index in dependency order.
        index: usize,
        /// What the Module said (MA-9).
        error: ModuleError,
    },
    /// A registered admission check refused the **applied** configuration. Not
    /// redundant with `validate`: a coercion can move an applied value outside a
    /// limit that the requested value respected (SB-30).
    Violations(Vec<crate::binding::Violation>),
}

/// `prepare(plan, ctx)` calls each fragment in plan order and collects one
/// `PrepareReport` per fragment; this returns them in the order received, which is
/// that order, and builds no merged configuration from them. It runs every registered
/// admission check against the per-fragment configuration
/// `{ report.fragment: report.effective }` — SB-30's second point, which exists
/// because a coercion can move an applied value outside a limit the requested value
/// respected. Any fragment's failure fails the whole transaction, and the Run moves
/// to cleanup.
///
/// The checks are run here rather than by the caller so that SB-30's second point
/// cannot be forgotten.
///
/// Rule: SB-30, SB-41, SB-42 (spec 20, KH-1).
pub fn collect_prepare(
    reports: Vec<Result<PrepareReport, ModuleError>>,
    spec: &ExperimentSpec,
    profile: &BindingProfile,
    inputs: &CompileInputs<'_>,
    admission: &AdmissionResult,
) -> Result<Vec<PrepareReport>, PrepareError> {
    prepare::collect_prepare(reports, spec, profile, inputs, admission)
}
