//! The ExecutionPlan, its dependency order, PrepareReport and Island admission —
//! `03-spec-and-binding.md` SB-37…SB-46, `05-module-api.md` MA-22, MA-39…MA-41.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::binding::{ComponentPlacement, LinkPlacement};
use crate::contract::PortRef;
use crate::id::{DataLinkId, ResourceId};
use crate::module_api::{
    ComponentDescriptor, ExecutionClass, ExecutorDescriptor, IslandDecl, LinkDescriptor,
    ModuleError, ModuleRef, Role, SinkDescriptor,
};
use crate::spec::{
    Coercion, CoercionPolicy, Ident, Key, KeyDecl, SpecError, Value, Warning};
use crate::stream::{BackPressure, DataLinkDecl};

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
/// never uses it to choose a placement; Phase 1 neither measures nor optimises.
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

/// Vision §11 says a report per fragment and §52 says "the PrepareReport"; both are
/// produced, and the merged one is what `run.effective()` returns (SB-41).
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MergedPrepare {
    /// One report per fragment, in dependency order (SB-41).
    pub reports: Vec<PrepareReport>,
    /// Every fragment's `effective`, merged (SB-41).
    pub effective: BTreeMap<Key, Value>,
}

impl MergedPrepare {
    /// Merges the reports in order; a later fragment's value wins for a key two
    /// fragments both name, which the Kernel does not otherwise interpret (SB-41).
    pub fn from_reports(reports: Vec<PrepareReport>) -> MergedPrepare {
        let mut effective = BTreeMap::new();
        for r in &reports {
            effective.extend(r.effective.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        MergedPrepare { reports, effective }
    }
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
    let mut incoming: BTreeMap<&Ident, BTreeSet<&Ident>> =
        nodes.iter().map(|n| (n, BTreeSet::new())).collect();
    if incoming.len() != nodes.len() {
        // A duplicate name would collapse in the map and make the loop below exit
        // through the cycle branch with nothing to name.
        let mut seen = BTreeSet::new();
        let dup = nodes.iter().find(|n| !seen.insert(*n)).expect("lengths differ");
        return Err(SpecError::Structural {
            reason: format!("SB-39: two fragments share the id {dup}"),
        });
    }
    for (before, after) in edges {
        // A dangling edge is a defect in the profile, not an ordering; reported as
        // itself rather than silently dropped or turned into a cycle.
        if !incoming.contains_key(before) || !incoming.contains_key(after) {
            return Err(SpecError::Structural {
                reason: format!("SB-39: ordering edge {before} -> {after} names an unknown fragment"),
            });
        }
        incoming.get_mut(after).expect("checked above").insert(before);
    }
    let mut out: Vec<Ident> = Vec::with_capacity(nodes.len());
    let mut done: BTreeSet<&Ident> = BTreeSet::new();
    while out.len() < nodes.len() {
        // Ties by name, so the order does not depend on input order (SB-39).
        let next = incoming
            .iter()
            .filter(|(n, _)| !done.contains(*n))
            .filter(|(_, deps)| deps.iter().all(|d| done.contains(d)))
            .map(|(n, _)| *n)
            .min();
        match next {
            Some(n) => {
                done.insert(n);
                out.push(n.clone());
            }
            None => {
                let stuck: Vec<&str> =
                    nodes.iter().filter(|n| !done.contains(n)).map(|n| n.as_str()).collect();
                return Err(SpecError::ArmCycle { path: stuck.join(" -> "),
                });
            }
        }
    }
    Ok(out)
}

/// The inverse of the arm order: the device that was armed first is released last
/// (RS-8).
pub fn release_order(arm_order: &[Ident]) -> Vec<Ident> {
    arm_order.iter().rev().cloned().collect()
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
    if let Some(p) = spec_override {
        return p;
    }
    if is_session {
        return CoercionPolicy::Warn;
    }
    decl.map(|d| d.coercion_default).unwrap_or(CoercionPolicy::Warn)
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
    match policy {
        CoercionPolicy::Reject => Err(SpecError::CoercionRejected(coercion.clone())),
        CoercionPolicy::Warn => Ok(Some(Warning {
            source: crate::spec::Namespace::parse("ezsdr.coercion").expect("a valid literal"),
            message: format!(
                "{} was coerced from {:?} to {:?}: {}",
                coercion.key, coercion.requested, coercion.applied, coercion.reason
            ),
        })),
        CoercionPolicy::Accept => Ok(None),
    }
}

// ---------------------------------------------------------------- island admission

/// What an Island admission check needs to know about the Run (MA-39).
pub struct IslandContext<'a> {
    /// The Islands being admitted (MA-38).
    pub islands: &'a [IslandDecl],
    /// Every component of the Spec's graph (SB-15).
    pub components: &'a BTreeMap<Ident, ComponentDescriptor>,
    /// Where each component is placed (SB-25).
    pub placements: &'a BTreeMap<Ident, ComponentPlacement>,
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
/// or the Executor's kind, `impl.kind` is among its `impl_kinds`, and the
/// placement's memory domain is among its `memory_domains`; every data link —
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
    // SB-25 / D76: a placement names its data link by its two ends. Coverage is
    // exact over graph links and output feeds, so neither an omitted nor a stale
    // placement is ignored, and a pair named twice is ambiguous rather than
    // first-wins.
    let pair = |from: &PortRef, to: &PortRef| {
        format!("{}.{} -> {}.{}", from.component, from.port, to.component, to.port)
    };
    let mut selected: BTreeMap<(&PortRef, &PortRef), &LinkPlacement> = BTreeMap::new();
    for p in ctx.link_placements {
        if selected.insert((&p.from, &p.to), p).is_some() {
            return Err(ModuleError::rejected(format!(
                "SB-25: link placement {} appears more than once",
                pair(&p.from, &p.to)
            )));
        }
    }
    let mut expected: BTreeSet<(&PortRef, &PortRef)> = BTreeSet::new();
    for (from, to, policy) in ctx.graph_links.iter().chain(ctx.feed_links) {
        let key = pair(from, to);
        if !expected.insert((from, to)) {
            return Err(ModuleError::rejected(format!(
                "SB-25: data link {key} is declared twice, so no placement can name one of them"
            )));
        }
        let placement = selected.get(&(from, to)).ok_or_else(|| {
            ModuleError::rejected(format!("SB-25: data link {key} has no link placement"))
        })?;
        let descriptor = ctx.links.get(&placement.link).ok_or_else(|| {
            ModuleError::rejected(format!(
                "MA-28: link placement {key} names Module {} {}, which has no registered Link descriptor",
                placement.link.id, placement.link.version
            ))
        })?;
        if descriptor.cross_process {
            return Err(ModuleError::rejected(format!(
                "MA-28: link placement {key} selects Module {} {}, whose cross_process capability is unsupported in v4.0",
                placement.link.id, placement.link.version
            )));
        }
        if !descriptor.policies.contains(policy) {
            return Err(ModuleError::rejected(format!(
                "MA-28: link placement {key} selects Module {} {}, which does not implement policy {policy:?}",
                placement.link.id, placement.link.version
            )));
        }
    }
    if let Some(stale) = ctx.link_placements.iter().find(|p| !expected.contains(&(&p.from, &p.to))) {
        return Err(ModuleError::rejected(format!(
            "SB-25: link placement {} names no graph link or output feed",
            pair(&stale.from, &stale.to)
        )));
    }

    // Every component placed exactly once.
    let mut placed: BTreeMap<&Ident, usize> = ctx.components.keys().map(|c| (c, 0)).collect();
    for island in ctx.islands {
        for c in &island.components {
            match placed.get_mut(c) {
                Some(n) => *n += 1,
                None => {
                    return Err(ModuleError::rejected(format!(
                        "MA-39: island {} places unknown component {c}",
                        island.id
                    )));
                }
            }
        }
    }
    if let Some((c, n)) = placed.iter().find(|(_, n)| **n != 1) {
        return Err(ModuleError::rejected(format!(
            "MA-39: component {c} is placed {n} times, not exactly once"
        )));
    }

    for island in ctx.islands {
        let executor = ctx.executors.get(&island.executor).ok_or_else(|| {
            ModuleError::rejected(format!(
                "MA-39: island {} names unknown executor {}",
                island.id, island.executor
            ))
        })?;
        for name in &island.components {
            let c = &ctx.components[name];
            if c.requires.executor_kind.as_str() != "any"
                && c.requires.executor_kind != executor.kind
            {
                return Err(ModuleError::rejected(format!(
                    "MA-39: {name} requires executor kind {} and island {} is {}",
                    c.requires.executor_kind, island.id, executor.kind
                )));
            }
            if !executor.impl_kinds.contains(&c.implementation.kind) {
                return Err(ModuleError::rejected(format!(
                    "MA-39: {name}'s impl kind {} is not among executor {}'s impl_kinds",
                    c.implementation.kind, island.executor
                )));
            }
            let placement = ctx.placements.get(name).ok_or_else(|| ModuleError::rejected(format!("MA-39: {name} has no placement")))?;
            if !executor.memory_domains.contains(&placement.memory_domain) {
                return Err(ModuleError::rejected(format!(
                    "MA-39: {name}'s memory domain {} is not among executor {}'s memory_domains",
                    placement.memory_domain, island.executor
                )));
            }
            // An Island with an rt_policy requires a declared budget on every component.
            if island.rt_policy.is_some() && c.timing.budget.is_none() {
                return Err(ModuleError::rejected(format!(
                    "MA-39: island {} has an rt_policy and {name} declares no budget",
                    island.id
                )));
            }
        }
    }

    let island_of = |component: &Ident| -> Option<&IslandDecl> {
        ctx.islands.iter().find(|i| i.components.contains(component))
    };

    // MA-22's cycle rule, which MA-39 names as one of its own checks. Every graph
    // link is a `stream.*` edge in Phase 1, so none of them may close a cycle.
    let nodes: Vec<Ident> =
        ctx.components.keys().chain(ctx.resource_endpoints.iter()).cloned().collect();
    let edges: Vec<GraphEdge> = ctx
        .graph_links
        .iter()
        .map(|(from, to, _)| GraphEdge {
            from: from.component.clone(),
            to: to.component.clone(),
            kind: EdgeKind::Stream,
            crosses_island: island_of(&from.component).map(|i| i.id)
                != island_of(&to.component).map(|i| i.id),
        })
        .collect();
    check_cycles(&nodes, &edges)?;

    // A link whose two ends share a memory domain needs nothing more; otherwise its
    // selected Link (SB-25) `connects` the pair, whether the ends are in one Island
    // or in two (D77).
    let is_resource = |r: &PortRef| ctx.resource_endpoints.contains(&r.component);
    let placement_of = |r: &PortRef| ctx.placements.get(&r.component);
    // D81: a feed's consumer is the bound Sink, whose readable domains its descriptor
    // declares. A resource producer is skipped as for a graph link (D31).
    for (from, to, _policy) in ctx.feed_links {
        if is_resource(from) {
            continue;
        }
        let fd = placement_of(from).ok_or_else(|| {
            ModuleError::rejected(format!(
                "MA-39: feed {}:{} -> {} leaves an unplaced component",
                from.component, from.port, to.component
            ))
        })?;
        let sink = ctx.sinks.get(&to.component).ok_or_else(|| {
            ModuleError::rejected(format!("SB-22f: no Sink instance for output {}", to.component))
        })?;
        if sink.memory_domains.contains(&fd.memory_domain) {
            continue;
        }
        let placement = selected[&(from, to)];
        let joined = ctx.links[&placement.link].connects.iter().any(|(a, b)| {
            (*a == fd.memory_domain && sink.memory_domains.contains(b))
                || (*b == fd.memory_domain && sink.memory_domains.contains(a))
        });
        if !joined {
            return Err(ModuleError::rejected(format!(
                "MA-39: {} is in memory domain {} and output {}'s Sink reads [{}]; selected Link Module {} {} does not connect them",
                from.component,
                fd.memory_domain,
                to.component,
                sink.memory_domains.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(", "),
                placement.link.id,
                placement.link.version
            )));
        }
    }
    for (from, to, _policy) in ctx.graph_links {
        let placement = selected[&(from, to)];
        let descriptor = &ctx.links[&placement.link];
        // A resource endpoint is not placed, so a link that touches one has nothing
        // to compare Islands or memory domains against here. Its producer-side
        // memory domain is Vocabulary content and a Phase 2 value (finding D31).
        if is_resource(from) || is_resource(to) {
            continue;
        }
        let (fi, ti) = (island_of(&from.component), island_of(&to.component));
        let (Some(fi), Some(ti)) = (fi, ti) else {
            return Err(ModuleError::rejected(format!(
                "MA-39: link {}:{} -> {}:{} touches an unplaced component",
                from.component, from.port, to.component, to.port
            )));
        };
        let (fd, td) = (placement_of(from), placement_of(to));
        if let (Some(fd), Some(td)) = (fd, td) {
            if fd.memory_domain == td.memory_domain {
                continue;
            }
            let joined = descriptor.connects.iter().any(|pair| {
                pair == &(fd.memory_domain, td.memory_domain)
                    || pair == &(td.memory_domain, fd.memory_domain)
            });
            if !joined {
                return Err(ModuleError::rejected(format!(
                    "MA-39: {} (island {}) and {} (island {}) are in different memory domains and selected Link Module {} {} does not connect them",
                    from.component, fi.id, to.component, ti.id, placement.link.id, placement.link.version
                )));
            }
        }
    }
    Ok(())
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
    let closing: Vec<(Ident, Ident)> = edges
        .iter()
        .filter(|e| {
            // Only an Event or Action edge crossing an Island boundary may close a cycle.
            !(matches!(e.kind, EdgeKind::Event | EdgeKind::Action) && e.crosses_island)
        })
        .map(|e| (e.from.clone(), e.to.clone()))
        .collect();
    arm_order(nodes, &closing).map(|_| ()).map_err(|_| {
        ModuleError::rejected(
            "MA-22: a cycle is legal only when every edge that closes it is an Event or Action edge crossing an Island boundary"
                .to_owned(),
        )
    })
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
    use crate::spec::{CapabilityValue, Constraint};
    let within = |v: &Value| -> bool {
        satisfies(&Constraint::Eq { value: v.clone() }, declared).unwrap_or(false)
    };
    let ok = match effective {
        CapabilityValue::One { value } => within(value),
        CapabilityValue::AnyOf { values } => values.iter().all(within),
        CapabilityValue::Range { min, max } => within(min) && within(max),
    };
    if ok {
        Ok(())
    } else {
        Err(ModuleError::rejected(
            "MA-12: `effective` may narrow a declared capability and must not widen one".to_owned(),
        ))
    }
}


// ---------------------------------------------------------------- the compile pipeline

use crate::binding::{
    AdmissionCheckRegistry, AdmissionResult, BindingProfile, CheckStage, check_constraint_kind,
    satisfies,
};
use crate::contract::ContractRegistry;
use crate::module_api::{
    AuthorityDescriptor, ModuleRegistry, Pacing, Provider, Requested, Resource, RfPath,
};
use crate::policy::EventKindRegistry;
use crate::spec::{ExperimentSpec, RejectedConstraint, ResourceReq};

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
    check_structure(spec, profile, inputs)?;
    let mut out = AdmissionResult::default();
    // SB-34: node -> the resource name that bound it, so a collision can name both.
    let mut taken: BTreeMap<ResourceId, Ident> = BTreeMap::new();

    // Matching against the bound instance (SB-34, SB-37). `check_structure` has
    // established that every resource slot is bound and has its Provider (SB-22f).
    for (name, req) in &spec.resources {
        let provider = supplied(inputs.providers, name, "Provider instance")?;
        let instance = provider.instance();
        let node = pick_node(&instance.tree, name, req, &taken)?;
        taken.insert(node.id.clone(), name.clone());
        out.matched.insert(name.clone(), node.id.clone());
        match_constraints(name, req, node, *provider, inputs, &mut out)?;

        // `needs` may resolve to a sub-resource of another bound instance (SB-36).
        for (need_name, need) in &req.needs {
            // SB-34 applies here too: a need consumes the node it resolves to
            // unless the Provider declared it shareable, so two needs of two
            // resources take two lines when there are two.
            // Only the instances bound to this Spec's resources: an instance handed
            // in under a name no resource binds is never read (SB-22, SB-36).
            let resolved = inputs
                .providers
                .iter()
                .filter(|(bound, _)| spec.resources.contains_key(*bound))
                .flat_map(|(_, p)| p.instance().tree.walk())
                .find(|n| {
                    n.kind == need.kind
                        && need_satisfied(need, n)
                        && (n.shareable || !taken.contains_key(&n.id))
                })
                .ok_or_else(|| SpecError::NoSingleInstance {
                    name: need_name.clone(),
                    constraint: format!("needs {}", need.kind),
                })?;
            taken.insert(resolved.id.clone(), need_name.clone());
            // SB-36 records the resolution in `matched`, and a need's name is
            // scoped to its resource: SB-36's own example calls one `gpio`, which
            // two peripherals would share. Qualified so that two do not collapse.
            let key = Ident::parse(&format!("{name}_{need_name}")).unwrap_or_else(|_| name.clone());
            out.matched.insert(key, resolved.id.clone());
        }
    }

    // The registered admission checks, against the requested configuration (SB-30).
    out.violations.extend(requested_violations(spec, profile, inputs));

    // SB-15 and SB-17 run **after** matching, because a resource endpoint's port is
    // one the *bound node* declares (MA-10) and `matched` does not exist before it.
    check_endpoints(spec, inputs, &out.matched)?;

    // SB-45, SB-46: a previewed coercion under `reject` fails the stage here rather
    // than at `prepare`. A dry run that reported a coercion it knows will be
    // rejected would not be a dry run (Vision §52).
    for previewed in out.coercions_preview.clone() {
        let c = &previewed.coercion;
        let policy = coercion_policy(
            spec.policies.coercion.get(&c.key).copied(),
            inputs.is_session,
            inputs.registry.key_decl(&c.key).ok(),
        );
        match apply_coercion(policy, c) {
            Ok(None) => {}
            Ok(Some(w)) => out.warnings.push(w),
            Err(_) => out.violations.push(crate::binding::Violation {
                check: crate::spec::Namespace::parse("ezsdr.coercion").expect("a valid literal"),
                key: Some(c.key.clone()),
                requested: Some(c.requested.clone()),
                reason: format!("SB-46: coercion to {:?} rejected", c.applied),
            }),
        }
    }
    Ok(out)
}

/// `validate()`'s structural checks, which `plan()` runs again (SB-39, D99): the
/// Spec's keys and Vocabulary majors (SB-2, SB-11), its failure policy (SB-18), the
/// ids a document carries as Rust values (X7, SB-1), the binding model (SB-22…SB-22h,
/// SB-24), the schedule (SB-16, RS-52) and the component descriptors (MA-37). None of
/// them calls a Provider's `coerce`, so running them twice changes nothing.
///
/// Rule: SB-T4.
fn check_structure(
    spec: &ExperimentSpec,
    profile: &BindingProfile,
    inputs: &CompileInputs<'_>,
) -> Result<(), SpecError> {
    spec.check_key_prefixes()?;
    check_keys(spec, inputs)?;
    // SB-11: `requirements.vocabularies` lists the Vocabulary **majors** this Spec's
    // keys belong to. Nothing read `major`, so a Spec declaring `test 2` ran against
    // the registered `test 1.0.0` and SB-11's list was a field with no reader — the
    // same shape D40 withdrew `constraints_hit` for.
    for req in &spec.requirements.vocabularies {
        let v = inputs.registry.vocabulary(&req.id).ok_or_else(|| SpecError::Structural {
            reason: format!("SB-11: Vocabulary {} is not registered", req.id),
        })?;
        if v.version.major != req.major {
            return Err(SpecError::Structural {
                reason: format!(
                    "SB-11: this Spec's keys belong to {} major {}, and the \
                     registered Vocabulary is {}",
                    req.id, req.major, v.version
                ),
            });
        }
    }
    for kind in spec.policies.failure.keys() {
        inputs.kinds.require(kind).map_err(|_| SpecError::Structural {
            reason: format!("SB-18: event kind {kind} is not registered"),
        })?;
    }
    check_local_ids(spec, profile)?;
    check_bindings(spec, profile, inputs)?;
    // SB-16: a `SpecTime` is "a resource name plus an offset in **that resource's**
    // stream clock", so the name must be one the Spec declares. Nothing read
    // `spec.schedule` at all, so a clock name bound to nothing passed every stage and
    // the entry left no trace in the plan.
    for entry in &spec.schedule {
        if !spec.resources.contains_key(&entry.at.clock) {
            return Err(SpecError::Structural {
                reason: format!(
                    "SB-16: schedule entry's clock {} is not a resource this Spec declares",
                    entry.at.clock
                ),
            });
        }
        // RS-52: "`UpdateParameter.class` is the parameter's declared update class",
        // and "an Action whose class was not declared is rejected at admission". For a
        // Spec the admission stage is `validate`, and nothing compared the two: a
        // schedule entry could state `hardware_timed` for a key that declares no class
        // at all, and the Action reached `arm` with a class nobody declared.
        if let crate::event::ActionTemplate::UpdateParameter { key, class, .. } = &entry.action {
            // The class comes from the key's `KeyDecl` and from nowhere else. SB-2:
            // "This is where a **Provider** parameter's class is declared, and the
            // only place it can be … whose parameters are Vocabulary keys and never a
            // `ComponentDescriptor`'s `params`." A schedule entry's target is a Spec
            // resource or `sink/<output id>` by SB-16, checked just below, so it is
            // never a component: consulting `graph.components` first let an unrelated
            // component's parameter list supply a class for a Provider key that
            // declares none, and shadow the declared class of one that does.
            let declared = inputs.registry.key_decl(key).ok().and_then(|d| d.update_class);
            match declared {
                None => {
                    return Err(SpecError::Structural {
                        reason: format!(
                            "RS-52: schedule entry updates {key}, which declares no update class"
                        ),
                    });
                }
                Some(d) if d != *class => {
                    return Err(SpecError::Structural {
                        reason: format!(
                            "RS-52: schedule entry states {class:?} for {key}, which declares {d:?}"
                        ),
                    });
                }
                Some(_) => {}
            }
        }
        // SB-16: a Spec's target is **Spec-relative** — its first segment names a Spec
        // resource, or the target is `sink/<output id>` — so that one Spec runs on Mock
        // and on hardware (§59, §61) without naming a device's own node tree. `arm`
        // rewrites it through `admission.matched`, as it resolves the `SpecTime`.
        if let Some(target) = entry.action.target() {
            let first = target.segments().next().unwrap_or_default();
            let known = if first == "sink" {
                target
                    .segments()
                    .nth(1)
                    .is_some_and(|o| spec.outputs.iter().any(|x| x.id.as_str() == o))
            } else {
                Ident::parse(first).is_ok_and(|n| spec.resources.contains_key(&n))
            };
            if !known {
                return Err(SpecError::Structural {
                    reason: format!(
                        "SB-16: schedule entry's target {target} names no resource or output \
                         this Spec declares"
                    ),
                });
            }
        }
    }
    // MA-37: a descriptor is validated before anything reads its ports, or a
    // duplicate port name silently resolves a link to whichever came first.
    let contract_ids = inputs.contracts.ids();
    for c in spec.graph.components.values() {
        c.validate(&contract_ids).map_err(|e| SpecError::Structural { reason: e.message })?;
    }
    Ok(())
}

/// SB-2 and SB-6 for every constraint a resource states: an `ext.` key's Module id
/// names a registered Module, any other key has its `KeyDecl`, and the constraint's
/// scalars are of that declaration's kind. None of it needs an instance, so it is a
/// structural check, which `plan()` runs again (SB-T4, D99).
///
/// Rule: SB-2, SB-6, MA-34.
fn check_keys(spec: &ExperimentSpec, inputs: &CompileInputs<'_>) -> Result<(), SpecError> {
    for req in spec.resources.values() {
        for (key, constraint) in &req.requires {
            if key.is_extension() {
                // MA-34 writes `ext.<module-id>.<path>`, so the owner is a **registered**
                // Module. Accepting any `ext.…` would make the escape hatch unowned: no
                // declaration, no shape check and nobody accountable for the meaning.
                // On a segment boundary: `starts_with` alone made
                // `ext.ezsdr.test.providerx.thing` "owned" by `ezsdr.test.provider`, and a
                // single-segment id such as `ezsdr` would own every `ext.ezsdr.*` key.
                let owner = key.as_str().trim_start_matches("ext.");
                let owned = |id: &str| owner.strip_prefix(id).is_some_and(|rest| rest.starts_with('.'));
                if !inputs.registry.modules().any(|m| owned(m.id.as_str())) {
                    return Err(SpecError::UnknownKeyPrefix {
                        key: format!("{key}: MA-34: no registered Module owns this `ext.` prefix"),
                    });
                }
            } else {
                let d = inputs.registry.key_decl(key).map_err(|e| SpecError::UnknownKeyPrefix {
                    key: format!("{key}: {}", e.message),
                })?;
                check_constraint_kind(d, constraint)?;
            }
        }
    }
    Ok(())
}

/// SB-30's first point: every registered check whose section is present, against the
/// requested configuration. Checks are pure (SB-31), so `plan()` runs this again
/// rather than trusting a result's `violations` (SB-39, D99).
///
/// Rule: SB-30.
fn requested_violations(
    spec: &ExperimentSpec,
    profile: &BindingProfile,
    inputs: &CompileInputs<'_>,
) -> Vec<crate::binding::Violation> {
    let requested: BTreeMap<Key, Value> = spec
        .resources
        .values()
        .flat_map(|r| r.requires.iter())
        .filter_map(|(k, c)| match c {
            crate::spec::Constraint::Eq { value } => Some((k.clone(), value.clone())),
            _ => None,
        })
        .collect();
    inputs.checks.run(&profile.environment, &requested, &BTreeMap::new(), CheckStage::Validate)
}

/// `validate()`'s endpoint checks, which need the matched nodes, and which `plan()`
/// runs again over the `AdmissionResult`'s (SB-15, SB-15a, SB-17, SC-3, SC-21, D99).
///
/// Rule: SB-T4.
fn check_endpoints(
    spec: &ExperimentSpec,
    inputs: &CompileInputs<'_>,
    matched: &BTreeMap<Ident, ResourceId>,
) -> Result<(), SpecError> {
    check_graph_links(spec, inputs, matched)?;
    check_outputs(spec, inputs, matched)
}

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

/// The node of the instance's tree this resource binds to.
///
/// SB-34: a node is bound by at most one Spec resource unless the Provider declares
/// it `shareable` (MA-10). Of the nodes whose `kind` matches and that are available
/// under that rule, one that satisfies every constraint directly is preferred, then
/// any — the fallback is what lets a coercible key reach the Provider's `coerce` and
/// a genuine rejection be reported against a real candidate. When every kind match
/// is taken the binding is **refused** with `NodeAlreadyBound` rather than resolved
/// by sharing: handing one physical channel to two intents with no diagnostic is the
/// failure D27 was raised about.
///
/// ponytail: greedy, no backtracking. A bipartite matching is the upgrade if a
/// profile ever needs one; a failed greedy pick is now at least a diagnostic.
fn pick_node<'a>(
    tree: &'a Resource,
    name: &Ident,
    req: &ResourceReq,
    taken: &BTreeMap<ResourceId, Ident>,
) -> Result<&'a Resource, SpecError> {
    let candidates: Vec<&Resource> =
        tree.walk().into_iter().filter(|n| n.kind == req.kind).collect();
    let satisfies_all = |n: &Resource| {
        req.requires.iter().all(|(k, c)| {
            n.capabilities.get(k).is_some_and(|cap| satisfies(c, cap).unwrap_or(false))
        })
    };
    let free = |n: &Resource| !taken.contains_key(&n.id);
    let pick = |f: &dyn Fn(&Resource) -> bool| candidates.iter().copied().find(|n| f(n));
    // A free node first, so sharing is what SB-34 *allows* and not what it chooses:
    // two resources take two lines when there are two, even if both are shareable.
    let order: [&dyn Fn(&Resource) -> bool; 4] = [
        &|n| free(n) && satisfies_all(n),
        &|n| free(n),
        &|n| n.shareable && satisfies_all(n),
        &|n| n.shareable,
    ];
    for f in order {
        if let Some(n) = pick(f) {
            return Ok(n);
        }
    }
    // Every kind match is taken and exclusive: name the first binder, so the author
    // sees which two intents collided rather than a bare "no instance".
    if let Some(n) = candidates.first() {
        let first = taken.get(&n.id).cloned().unwrap_or_else(|| name.clone());
        return Err(SpecError::NodeAlreadyBound {
            node: n.id.to_string(),
            first,
            second: name.clone(),
        });
    }
    Err(SpecError::NoSingleInstance {
        name: name.clone(),
        constraint: format!("kind {}", req.kind),
    })
}

fn need_satisfied(need: &crate::spec::SubResourceReq, node: &Resource) -> bool {
    need.requires.iter().all(|(k, c)| {
        node.capabilities.get(k).is_some_and(|cap| satisfies(c, cap).unwrap_or(false))
    })
}

/// Matches a resource's constraints against the bound node's capabilities, offering
/// a coercible key to the Provider's `coerce` when the declared capability does not
/// satisfy it directly (SB-6, SB-7).
fn match_constraints(
    name: &Ident,
    req: &ResourceReq,
    node: &Resource,
    provider: &dyn Provider,
    inputs: &CompileInputs<'_>,
    out: &mut AdmissionResult,
) -> Result<(), SpecError> {
    // The coercible keys the declared capability did not satisfy directly, collected
    // so that `coerce` is called once with the whole request (SB-7, SB-44).
    let mut to_coerce: Vec<Key> = Vec::new();
    for (key, constraint) in &req.requires {
        // SB-2 and SB-6 were checked structurally (`check_keys`), so a non-`ext.` key
        // has its declaration; an `ext.` key has none by construction (MA-34).
        let decl = if key.is_extension() { None } else { inputs.registry.key_decl(key).ok() };
        let cap = node.capabilities.get(key);
        let direct = match cap {
            Some(cap) => satisfies(constraint, cap).map_err(|e| match e {
                SpecError::KeyShape { expected, found, .. } => SpecError::KeyShape { key: key.to_string(), expected, found,
                },
                other => other,
            })?,
            // SB-6: an absent capability satisfies nothing, `Present` included —
            // `Present` asks whether the *capability* is declared.
            None => false,
        };
        if direct {
            continue;
        }
        // SB-7: a key that is not coercible fails immediately, without calling
        // `coerce`. An `ext.` key has no declaration, so it is not coercible either:
        // an Extension has nothing that says a Provider may reinterpret its value.
        if !decl.is_some_and(|d| d.coercible) {
            out.rejected.push(RejectedConstraint {
                resource: name.clone(),
                key: key.clone(),
                constraint: constraint.clone(),
                reason: "SB-7: the declared capability does not satisfy it and the key is not coercible"
                    .to_owned(),
            });
            continue;
        }
        to_coerce.push(key.clone());
    }

    // SB-7: **one** call per bound node, with the resource's whole `requires` map.
    // Calling once per key with a single-key request made SB-44's "the same request"
    // unsatisfiable, because a fragment carries the whole map and a Provider replays
    // `coerce` over all of it — so for a Provider whose keys interact (rate times
    // decimation) the two answers differ legitimately and the Kernel's own check at
    // `prepare` could only be approximate. With one call it is exact, by MA-11's
    // determinism. The matcher still asks only what a value becomes; it never decides
    // the grid.
    if to_coerce.is_empty() {
        return Ok(());
    }
    let report = provider
        .coerce(&Requested { resource: node.id.clone(), constraints: req.requires.clone(),
        })
        .map_err(|e| SpecError::Structural { reason: e.message })?;
    for key in to_coerce {
        if report.applied.contains_key(&key) {
            continue;
        }
        out.rejected.push(RejectedConstraint {
            resource: name.clone(),
            key: key.clone(),
            constraint: req.requires[&key].clone(),
            reason: "SB-7: the Provider could not coerce it".to_owned(),
        });
    }
    // SB-44: a `Coercion` for a key the request does not name is a malformed report at
    // this stage for the same reason D42 made it one at `prepare` — nothing was
    // requested, so nothing was coerced. D42 guarded `prepare` only, and the same
    // report reaching `validate` entered `coercions_preview` unchecked: the SB-45/46
    // policy loop then fired that key's default for a key nobody asked about, and the
    // Manifest recorded a substitution of a value nobody requested.
    let (strays, previewed): (Vec<_>, Vec<_>) =
        report.coercions.into_iter().partition(|c| !req.requires.contains_key(&c.key));
    for stray in &strays {
        out.violations.push(crate::binding::Violation {
            check: crate::spec::Namespace::parse("ezsdr.coercion").expect("a valid literal"),
            key: Some(stray.key.clone()),
            requested: Some(stray.requested.clone()),
            reason: format!(
                "SB-44: {name}'s Provider reports a coercion of {}, which its request does \
                 not name",
                stray.key
            ),
        });
    }
    // A stray is refused above and **not recorded**: `coercions_preview` reaches the
    // Manifest through `AdmissionResult` (SB-38), and leaving it there both recorded a
    // substitution of a value nobody requested and fired that key's own policy default
    // — the two harms the refusal exists to prevent.
    out.coercions_preview.extend(previewed.into_iter().map(|coercion| crate::binding::PreviewedCoercion { resource: name.clone(), coercion,
                }),
        );
    out.warnings.extend(report.warnings);
    Ok(())
}

/// `graph.links` connects two `PortRef`s and states a policy and a capacity, both
/// mandatory. `validate()` checks contract compatibility by SC-3 and refuses a
/// `Block` policy on a link whose consumer belongs to a Sink-role Module (SB-15).
fn check_graph_links(
    spec: &ExperimentSpec,
    inputs: &CompileInputs<'_>,
    matched: &BTreeMap<Ident, ResourceId>,
) -> Result<(), SpecError> {
    // SB-15 / D76: a link is identified by its two ends, which is how a
    // `LinkPlacement` names it, so a pair may appear only once.
    let mut pairs = BTreeSet::new();
    for link in &spec.graph.links {
        if !pairs.insert((&link.from, &link.to)) {
            return Err(SpecError::Structural {
                reason: format!(
                    "SB-15: link {}.{} -> {}.{} is declared twice",
                    link.from.component, link.from.port, link.to.component, link.to.port
                ),
            });
        }
    }
    for link in &spec.graph.links {
        if link.capacity == 0 {
            return Err(SpecError::Structural {
                reason: "SB-15: a link's capacity is mandatory and at least 1".to_owned(),
            });
        }
        // SB-15: an endpoint is a component's port or a port a bound resource
        // declares. A resource endpoint is what lets a Spec connect a Provider's
        // stream to the graph at all (MA-10; finding D31).
        let (from, to) = (
            endpoint_port(&link.from, spec, inputs, matched),
            endpoint_port(&link.to, spec, inputs, matched),
        );
        for (r, c) in [(&link.from, &from), (&link.to, &to)] {
            if c.is_none() {
                return Err(SpecError::Structural {
                    reason: format!(
                        "SB-15: link endpoint {}:{} names no declared port of a component or a bound resource",
                        r.component, r.port
                    ),
                });
            }
        }
        if let (Some((from_dir, from)), Some((to_dir, to))) = (&from, &to) {
            // SB-15a, **before** SC-3: a contract check between a producer and a
            // consumer is well posed only once which is which has been established.
            // `Port.direction` was carried through the whole compile path and never
            // read, so `a.in -> b.in` validated and planned and SC-3 was evaluated on
            // an orientation nothing had checked.
            for (r, d, want) in [
                (&link.from, from_dir, crate::contract::PortDirection::Out),
                (&link.to, to_dir, crate::contract::PortDirection::In),
            ] {
                if *d != want {
                    return Err(SpecError::Structural {
                        reason: format!(
                            "SB-15a: link endpoint {}:{} is an {d:?} port and this end of a \
                             link is {want:?}",
                            r.component, r.port
                        ),
                    });
                }
            }
            inputs
                .contracts
                .check_link(from, to)
                .map_err(|e| SpecError::Structural { reason: e.to_string(),
                })?;
        }
        // SC-19's mandatory policy is checked above; a Sink is never a link's
        // consumer here, because a Sink is bound rather than placed and its own
        // link is the `outputs[]` entry that `check_outputs` validates (SB-17).
    }
    Ok(())
}

/// X7 and SB-1 for the ids a document carries that a Rust caller can build unparsed —
/// scheduled Action targets, Island ids and component memory domains. The ids the
/// runtime supplies are checked with the slot they belong to (SB-22f); an id read
/// from a document is refused by its deserialiser (D91, D95).
///
/// Rule: X7, SB-1.
fn check_local_ids(spec: &ExperimentSpec, profile: &BindingProfile) -> Result<(), SpecError> {
    for entry in &spec.schedule {
        if let Some(t) = entry.action.target() {
            check_rid("scheduled Action target", t)?;
        }
    }
    for island in &profile.placements.islands {
        if !island.id.node.is_local() {
            return Err(not_local(format!("island {}", island.id)));
        }
    }
    for (name, placement) in &profile.placements.components {
        if !placement.memory_domain.node.is_local() {
            return Err(not_local(format!("component {name}'s memory domain {}", placement.memory_domain)));
        }
    }
    Ok(())
}

fn not_local(what: String) -> SpecError {
    SpecError::Structural { reason: format!("X7: {what} is not on the local node") }
}

/// A `ResourceId` handed in as a Rust value: on the local node (X7) and with a path of
/// SB-1's grammar, because its fields are public and a Rust caller can build one
/// unparsed — which would reach a Manifest the Kernel's own deserialiser refuses.
///
/// Rule: X7, SB-1.
fn check_rid(what: &str, id: &ResourceId) -> Result<(), SpecError> {
    if !id.node.is_local() {
        return Err(not_local(format!("{what} {id}")));
    }
    if ResourceId::parse(&id.path).is_err() {
        return Err(SpecError::Structural {
            reason: format!("SB-1: {what} {id} is not a path of SB-1's grammar"),
        });
    }
    Ok(())
}

/// SB-22e: the registered Module a binding names holds `role`. A version that is not
/// registered is refused as such, and a registered one without the role is
/// `WrongBindingRole`. One function, because four copies of the question had grown
/// four answers — and a Session and a Spec Run had reported one condition as two
/// different errors.
///
/// Rule: SB-22e, MA-1.
pub(crate) fn require_role(
    registry: &ModuleRegistry,
    name: &Ident,
    module: &ModuleRef,
    role: Role,
) -> Result<(), SpecError> {
    let Some(m) = registry.modules().find(|m| crate::module_api::is_module(m, module)) else {
        return Err(not_registered(name, module));
    };
    if m.roles.contains(&role) {
        return Ok(());
    }
    Err(SpecError::WrongBindingRole {
        name: name.clone(),
        expected: format!("{role:?}"),
        module: format!("{} {}", module.id, module.version),
    })
}

/// SB-22e's refusal of a Module version nobody registered, shared with the Session
/// derivation so that one condition has one error on both Run kinds.
pub(crate) fn not_registered(name: &Ident, module: &ModuleRef) -> SpecError {
    SpecError::Structural {
        reason: format!(
            "SB-22e: binding {name} names Module {} {}, which is not registered",
            module.id, module.version
        ),
    }
}

/// A feed's consumer end, `{output id, "in"}`: how a `LinkPlacement` and the plan's
/// `links` name the link into a bound Sink (SB-25, D76).
fn feed_end(output: &Ident) -> PortRef {
    PortRef { component: output.clone(), port: Ident::parse("in").expect("a valid literal") }
}

/// The binding model over the slots the two documents name (SB-22…SB-22h, SB-24,
/// tables SB-T1…SB-T3). The runtime's maps are read under slot names only, through
/// [`supplied`]; an entry under any other name is never looked at (SB-22).
///
/// Rule: SB-22, SB-22a…SB-22h, SB-24, SB-3, SB-36.
fn check_bindings(
    spec: &ExperimentSpec,
    profile: &BindingProfile,
    inputs: &CompileInputs<'_>,
) -> Result<(), SpecError> {
    let registry = inputs.registry;
    let authority = &profile.authority;
    let rides = spec.resources.contains_key(authority);
    let executors: BTreeSet<&Ident> =
        profile.placements.islands.iter().map(|i| &i.executor).collect();

    // SB-22a: one namespace over the six kinds of name of SB-T1. `PortRef.component`
    // resolves a component before a resource and a feed's consumer end is `{output
    // id, "in"}`, so any collision among those is an ambiguity; an Island's fragment
    // is `island_<local>` and a dedicated Authority's is its name, so those are taken
    // too (D85; the Authority's since D96). One executor may serve several Islands, and
    // `authority` may be a resource, which the Authority then rides on (SB-24).
    let island_ids: Vec<Ident> = profile
        .placements
        .islands
        .iter()
        .filter_map(|i| Ident::parse(&format!("island_{}", i.id.local)).ok())
        .collect();
    let own_authority = (!rides).then_some(authority);
    let mut names: BTreeMap<&Ident, &str> = BTreeMap::new();
    let sets = spec
        .resources
        .keys()
        .map(|n| (n, "a resource"))
        .chain(spec.graph.components.keys().map(|n| (n, "a graph component")))
        .chain(spec.outputs.iter().map(|o| (&o.id, "an output id")))
        .chain(profile.placements.islands.iter().map(|i| (&i.executor, "an Island executor")))
        .chain(island_ids.iter().map(|n| (n, "an Island fragment id")))
        .chain(own_authority.map(|a| (a, "the `authority` binding")));
    for (name, set) in sets {
        // SB-16 reads a target's first segment `sink` as a Sink's address, so a name
        // `sink` is one a target could never reach (SB-22a, SB-22h).
        if name.as_str() == "sink" {
            return Err(SpecError::DuplicateBindingName {
                name: name.clone(),
                sets: format!("{set} and the reserved Sink-address segment"),
            });
        }
        if let Some(first) = names.insert(name, set) {
            if first == "an Island executor" && set == first {
                continue;
            }
            return Err(SpecError::DuplicateBindingName {
                name: name.clone(),
                sets: if first == set { format!("{set} declared twice") } else { format!("{first} and {set}") },
            });
        }
    }
    // SB-36 records a need under `<resource>_<need>`, and `matched` is one map, so a
    // key equal to another resource's name silently overwrote the need's record —
    // always the need's, because `X < X_need` orders the resource's insert second.
    for (name, req) in &spec.resources {
        for need in req.needs.keys() {
            if let Some(k) = Ident::parse(&format!("{name}_{need}")).ok().filter(|k| spec.resources.contains_key(k)) {
                return Err(SpecError::DuplicateBindingName {
                    name: k,
                    sets: format!("a resource and {name}'s need {need}"),
                });
            }
        }
    }

    // One slot at a time: bound (SB-22d), to a Module holding its role (SB-22e), with
    // the object the runtime supplies naming the binding's version and carrying only
    // local, well-formed ids (SB-22f) — role before instance, as SB-22c orders it.
    let version = |name: &Ident, what: &str, bound: &ModuleRef, reported: &ModuleRef| {
        if bound == reported {
            return Ok(());
        }
        Err(SpecError::Structural {
            reason: format!(
                "SB-22f: binding {name} names Module {} {}, but its {what} is {} {}",
                bound.id, bound.version, reported.id, reported.version
            ),
        })
    };
    for name in spec.resources.keys() {
        let binding = profile
            .bindings
            .get(name)
            .ok_or_else(|| SpecError::UnboundResource { name: name.clone() })?;
        require_role(registry, name, &binding.module, Role::Provider)?;
        let instance = supplied(inputs.providers, name, "Provider instance")?.instance();
        version(name, "Provider instance", &binding.module, &instance.module)?;
        check_rid(&format!("the instance bound to {name} has id"), &instance.id)?;
        for n in instance.tree.walk() {
            check_rid(&format!("the instance bound to {name} declares node"), &n.id)?;
        }
        for a in &instance.arm_after {
            check_rid(&format!("the instance bound to {name} arms after"), a)?;
        }
    }
    for output in &spec.outputs {
        let name = &output.id;
        let binding = profile.bindings.get(name).ok_or_else(|| SpecError::Structural {
            reason: format!("SB-22d: output {name} has no binding (UnboundOutput)"),
        })?;
        require_role(registry, name, &binding.module, Role::Sink)?;
        let d = supplied(inputs.sinks, name, "Sink instance")?.descriptor();
        version(name, "Sink instance", &binding.module, &d.module)?;
        // MA-25 / D86: a Sink that reads from no memory domain cannot be fed, yet a
        // resource feed skips the domain check (D31) and would admit it.
        if d.memory_domains.is_empty() {
            return Err(SpecError::Structural {
                reason: format!("MA-25: output {name}'s Sink declares no memory domain it reads from"),
            });
        }
        if let Some(m) = d.memory_domains.iter().find(|m| !m.node.is_local()) {
            return Err(not_local(format!("output {name}'s Sink memory domain {m}")));
        }
    }
    for name in &executors {
        let binding = profile.bindings.get(*name).ok_or_else(|| SpecError::Structural {
            reason: format!("SB-22d: an Island names executor {name}, which the profile does not bind (UnboundExecutor)"),
        })?;
        require_role(registry, name, &binding.module, Role::Executor)?;
        let d = supplied(inputs.executors, name, "ExecutorDescriptor")?;
        version(name, "Executor instance", &binding.module, &d.module)?;
        // MA-18 / D86: admission reads `memory_domains`; an Executor that reaches
        // none can host no component.
        if d.memory_domains.is_empty() {
            return Err(SpecError::Structural {
                reason: format!("MA-18: executor {name} declares no memory domain it can reach"),
            });
        }
        if let Some(m) = d.memory_domains.iter().find(|m| !m.node.is_local()) {
            return Err(not_local(format!("executor {name}'s memory domain {m}")));
        }
    }
    // SB-24: `authority` names a binding whose Module holds Authority — a resource the
    // Authority rides on as much as a slot of its own — and whose supplied descriptor
    // names that binding's version (SB-22e, D98).
    let binding = profile.bindings.get(authority).ok_or_else(|| SpecError::Structural {
        reason: format!("SB-24: `authority` names {authority}, which is not a binding"),
    })?;
    require_role(registry, authority, &binding.module, Role::Authority)?;
    let a = supplied(inputs.authorities, authority, "AuthorityDescriptor")?;
    version(authority, "Authority", &binding.module, &a.module)?;
    if let Some(d) = a.governs.iter().find(|d| !d.node.is_local()) {
        return Err(not_local(format!("Authority {authority}'s governed domain {d}")));
    }

    // SB-22g: on a Spec Run no binding carries `feed`, since `outputs[]` declares each
    // feed; on a Session a binding carries one exactly when it fills a Sink slot, and
    // it is that output's feed. Anything else is a second, unread source of truth —
    // including a policy SC-21 restricts and nothing would have looked at.
    for (name, b) in &profile.bindings {
        let output = spec.outputs.iter().find(|o| &o.id == name);
        let refused = match (&b.feed, output) {
            (Some(_), _) if !inputs.is_session => Some(
                "carries `feed`, which only a Session profile may; a Spec Run declares it in `outputs[]`",
            ),
            (Some(feed), Some(o)) if o.feed != *feed => Some("carries a `feed` that is not its output's"),
            (Some(_), None) => Some("carries `feed` and fills no Sink slot"),
            (None, Some(_)) if inputs.is_session => Some("fills a Session's Sink slot and carries no `feed`"),
            _ => None,
        };
        if let Some(why) = refused {
            return Err(SpecError::Structural { reason: format!("SB-22g: binding {name} {why}") });
        }
    }

    // SB-22h reserves the first path segment `sink` for a bound Sink's address, so a
    // Provider may not declare a node under it: without this the `sink/` prefix only
    // narrowed the collision from "any output id" to "a Provider that names a node
    // `sink`" — `ResourceId::parse("sink/rec")` is a legal Provider node path.
    for name in spec.resources.keys() {
        let instance = supplied(inputs.providers, name, "Provider instance")?.instance();
        if let Some(clash) = instance.tree.walk().into_iter().find(|n| n.id.segments().next() == Some("sink")) {
            return Err(SpecError::Structural {
                reason: format!(
                    "SB-22h: instance bound to {name} declares node {}, and the first path \
                     segment `sink` is reserved for a bound Sink's address",
                    clash.id
                ),
            });
        }
    }

    // SB-3: a `ResourceId` carries no instance qualification, so two bound instances
    // declaring one node path are one address — for the matcher's `taken` set, for
    // `arm_after` resolution and for `Event.source` attribution alike. Identity is the
    // **binding description** `(module, selector, profile)`: two bindings with equal
    // descriptions are one instance (SB-23 reads `instances: 2` that way) and must
    // report one `instance().id`, or the runtime handed the Kernel two objects for one
    // description and the Manifest could not reproduce it.
    let mut by_description: BTreeMap<(&ModuleRef, String), (&Ident, &ResourceId)> = BTreeMap::new();
    let mut paths: BTreeMap<ResourceId, &Ident> = BTreeMap::new();
    for (name, binding) in profile.bindings.iter().filter(|(n, _)| spec.resources.contains_key(*n)) {
        let instance = supplied(inputs.providers, name, "Provider instance")?.instance();
        let description = (
            &binding.module,
            format!(
                "{}|{}",
                serde_json::to_string(&binding.selector).unwrap_or_default(),
                serde_json::to_string(&binding.profile).unwrap_or_default()
            ),
        );
        if let Some((first, first_id)) = by_description.insert(description, (name, &instance.id)) {
            if *first_id != instance.id {
                return Err(SpecError::Structural {
                    reason: format!(
                        "SB-3: {first} and {name} carry one binding description and so name one \
                         instance, but their Providers report {first_id} and {}",
                        instance.id
                    ),
                });
            }
            continue; // one instance bound twice is SB-34's case, not this
        }
        for node in instance.tree.walk() {
            if let Some(first) = paths.insert(node.id.clone(), name) {
                if first != name {
                    return Err(SpecError::Structural {
                        reason: format!(
                            "SB-3: the instances bound to {first} and {name} both declare node \
                             {}, and a ResourceId carries no instance qualification",
                            node.id
                        ),
                    });
                }
            }
        }
    }

    // SB-22d / D89: a binding that fills no slot is read by no check — its Module
    // reference could be an unregistered version and still reach the Manifest, which
    // records the profile verbatim (RS-38). Last, so that a misspelt output binding is
    // `UnboundOutput` and a stray `feed` is SB-22g's.
    if let Some(name) = profile.bindings.keys().find(|n| {
        !spec.resources.contains_key(*n)
            && !spec.outputs.iter().any(|o| &o.id == *n)
            && !executors.contains(n)
            && *n != authority
    }) {
        return Err(SpecError::Structural {
            reason: format!("SB-22d: binding {name} plays no role in this Run"),
        });
    }
    Ok(())
}

/// SB-17: each `outputs[]` entry names a source port, an artifact kind, the Sink
/// parameters, and the drop-class policy and capacity of the link that feeds it.
/// `validate()` refuses an output whose source port does not exist or is not an `out`
/// port (SB-15a), whose feed is not of the drop class (SC-21) or has no capacity
/// (SC-19), or whose bound `SinkDescriptor` does not list the artifact kind or does
/// not accept the source port's contract (SC-3). That the output is bound, to a Sink,
/// at its binding's version, is SB-22's.
///
/// A Sink is **bound, not placed**: it is a Module role (MA-2, MA-25, MA-30) and
/// not a component an Executor loads, so nothing here consults `graph.components`
/// or an Island. That is what makes the check work on a Spec Run, which is the path
/// a publication Run uses (findings D17, D29).
fn check_outputs(
    spec: &ExperimentSpec,
    inputs: &CompileInputs<'_>,
    matched: &BTreeMap<Ident, ResourceId>,
) -> Result<(), SpecError> {
    for output in &spec.outputs {
        let feed = &output.feed;
        if feed.capacity == 0 {
            return Err(SpecError::Structural {
                reason: format!(
                    "SB-17, SC-19: output {}'s capacity is mandatory and at least 1",
                    output.id
                ),
            });
        }
        let source = endpoint_port(&feed.port, spec, inputs, matched);
        // SC-21: a link into a Sink is of the drop class. `Block` would let a slow
        // recorder stall the real-time path.
        let decl = DataLinkDecl {
            id: DataLinkId::local(0),
            from: feed.port.clone(),
            to: feed_end(&output.id),
            contract: source.as_ref().map(|(_, c)| c.clone()).unwrap_or_else(|| {
                crate::contract::DataContractId::parse("ezsdr.control").expect("a valid literal")
            }),
            policy: feed.policy,
            capacity: feed.capacity,
        };
        crate::stream::check_sink_link(&decl, true)
            .map_err(|e| SpecError::Structural { reason: e.to_string() })?;

        // The source port must exist, on a component or on a bound resource.
        let (direction, contract) = source.ok_or_else(|| SpecError::Structural {
            reason: format!(
                "SB-17: output {} names source port {}:{}, which does not exist",
                output.id, feed.port.component, feed.port.port
            ),
        })?;
        // SB-15a: an output's feed leaves an `out` port. A Sink records what a port
        // produces, so a feed from an `in` port names the wrong end of the stream.
        if direction != crate::contract::PortDirection::Out {
            return Err(SpecError::Structural {
                reason: format!(
                    "SB-15a: output {}'s feed names {}:{}, which is an {direction:?} port",
                    output.id, feed.port.component, feed.port.port
                ),
            });
        }

        // MA-25: the bound Sink must write this artifact kind and accept the
        // source port's contract.
        let d = supplied(inputs.sinks, &output.id, "Sink instance")?.descriptor();
        if !d.artifact_kinds.contains(&output.kind) {
            return Err(SpecError::Structural {
                reason: format!(
                    "SB-17: output {} wants artifact kind {}, which its Sink does not write",
                    output.id, output.kind
                ),
            });
        }
        if !d.contracts.iter().any(|c| inputs.contracts.check_link(&contract, c).is_ok()) {
            return Err(SpecError::Structural {
                reason: format!(
                    "SB-17, SC-3: output {}'s source carries {contract}, which its Sink does not consume",
                    output.id
                ),
            });
        }
    }
    Ok(())
}

/// The direction and contract a link endpoint declares, on a component or on the
/// **bound node** of a resource (SC-1, SB-15, MA-10).
fn endpoint_port(
    r: &PortRef,
    spec: &ExperimentSpec,
    inputs: &CompileInputs<'_>,
    matched: &BTreeMap<Ident, ResourceId>,
) -> Option<(crate::contract::PortDirection, crate::contract::DataContractId)> {
    let name = &r.component;
    if let Some(c) = spec.graph.components.get(name) {
        return c
            .ports
            .iter()
            .find(|p| p.name == r.port)
            .map(|p| (p.direction, p.contract.clone()));
    }
    // SB-15: "a resource port is one the **bound node** declares". Searching the
    // whole instance would accept a port some other sub-resource declares and would
    // check SC-3 against that node's contract — a link the Kernel calls compatible
    // and the hardware will not honour. Only a resource slot's Provider is read
    // (SB-22); `matched` also holds need keys, which are not endpoints.
    if !spec.resources.contains_key(name) {
        return None;
    }
    let provider = inputs.providers.get(name)?;
    let bound = matched.get(name)?;
    provider
        .instance()
        .tree
        .walk()
        .into_iter()
        .find(|n| n.id == *bound)?
        .ports
        .iter()
        .find(|p| p.name == r.port)
        .map(|p| (p.direction, p.contract.clone()))
}

/// A Provider fragment's content: the binding's selector, plus the request the
/// matcher resolved for the Spec resource of the same name. `requested` is absent
/// only for a binding that names no Spec resource, which SB-22 does not produce for
/// a Provider (SB-39, SB-44, MA-12).
fn provider_content(
    name: &Ident,
    binding: &crate::binding::Binding,
    spec: &ExperimentSpec,
    admission: &AdmissionResult,
) -> serde_json::Value {
    let requested = admission.matched.get(name).map(|node| Requested {
        resource: node.clone(),
        constraints: spec
            .resources
            .get(name)
            .map(|r| r.requires.clone())
            .unwrap_or_default(),
    });
    let mut out = serde_json::Map::new();
    out.insert(
        "selector".to_owned(),
        serde_json::to_value(&binding.selector).unwrap_or(serde_json::Value::Null),
    );
    if let Some(r) = requested {
        if let Ok(v) = serde_json::to_value(&r) {
            out.insert("requested".to_owned(), v);
        }
    }
    serde_json::Value::Object(out)
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
    check_structure(spec, profile, inputs)?;
    // SB-30's first point: "A non-empty violation list fails the stage, and nothing
    // transmits until every check at every applicable stage has passed." `validate`
    // *reports* — SB-38 requires the rejections and violations in the Manifest — so
    // the refusal has to happen here, where the next thing produced is an armable
    // plan. Leaving it to the caller is what left an RF-envelope refusal at the
    // validate point with nothing between it and a transmitter (Vision invariant 42).
    if !admission.is_admitted() {
        return Err(admission
            .clone()
            .into_result()
            .expect_err("a non-admitted result yields an error"));
    }
    // The checks are pure (SB-31), so a result computed for another `environment`
    // is not believed either (SB-39, D99).
    if let Some(v) = requested_violations(spec, profile, inputs).into_iter().next() {
        return Err(SpecError::Violation(v));
    }
    admission_is_this_runs(spec, inputs, admission)
        .map_err(|reason| SpecError::Structural { reason: format!("SB-39: {reason}") })?;
    check_endpoints(spec, inputs, &admission.matched)?;
    let authority = profile.authority.clone();
    let pacing = supplied(inputs.authorities, &authority, "AuthorityDescriptor")?.pacing;
    let class = derive_class(profile, pacing)?;

    // One fragment per slot (SB-T1): a Provider per resource, the Authority when it is
    // a slot of its own, a Sink per output, and an Executor per Island.
    let bound = |name: &Ident| -> Result<ModuleRef, SpecError> {
        profile.bindings.get(name).map(|b| b.module.clone()).ok_or_else(|| SpecError::Structural {
            reason: format!("SB-22d: {name} has no binding"),
        })
    };
    let mut fragments: Vec<Fragment> = Vec::new();
    let mut edges: Vec<(Ident, Ident)> = Vec::new();
    for name in spec.resources.keys() {
        let binding = supplied(&profile.bindings, name, "binding")?;
        let after = arm_after_of(name, spec, inputs);
        for before in &after {
            edges.push((before.clone(), name.clone()));
        }
        fragments.push(Fragment {
            id: name.clone(),
            instance: binding.module.clone(),
            role: Role::Provider,
            content: provider_content(name, binding, spec, admission),
            after,
        });
    }
    // SB-24: an Authority that is a slot of its own gets a fragment of role
    // Authority. It has no lifecycle (MA-2), so this is a record: it is how the
    // Module and version that chose the ExecutionClass reach the plan and the
    // Manifest (D92, D98).
    // ponytail: no ordering edges; whether Providers arm after it is TM-16a/MA-30's
    // Phase 2 call, plan content rather than schema.
    if !spec.resources.contains_key(&authority) {
        let binding = supplied(&profile.bindings, &authority, "binding")?;
        fragments.push(Fragment {
            id: authority.clone(),
            instance: binding.module.clone(),
            role: Role::Authority,
            content: serde_json::json!({ "selector": binding.selector }),
            after: Vec::new(),
        });
    }
    // One fragment per bound output: a Sink is prepared, armed and stepped like any
    // other Module instance (MA-25, MA-30), and it is bound rather than placed, so
    // it gets its own fragment and never appears in an Island's component list.
    for output in &spec.outputs {
        fragments.push(Fragment {
            id: output.id.clone(),
            instance: bound(&output.id)?,
            role: Role::Sink,
            content: serde_json::to_value(output).unwrap_or(serde_json::Value::Null),
            after: Vec::new(),
        });
    }
    for island in &profile.placements.islands {
        fragments.push(Fragment {
            // One fragment per Island, not per Executor instance: two Islands may
            // share one Executor (an affinity split), and a fragment id must be
            // unique (SB-22a, SB-39).
            id: Ident::parse(&format!("island_{}", island.id.local)).map_err(|_| {
                SpecError::Structural { reason: "MA-38: island id is not a valid Ident".to_owned() }
            })?,
            instance: bound(&island.executor)?,
            role: Role::Executor,
            content: serde_json::to_value(island).unwrap_or(serde_json::Value::Null),
            after: Vec::new(),
        });
    }

    // SB-37, MA-39, MA-40: admission is a stage of the pipeline, not a library
    // function a caller may forget. Its descriptors are read under slot names only.
    let mut executors: BTreeMap<Ident, ExecutorDescriptor> = BTreeMap::new();
    for island in &profile.placements.islands {
        let d = supplied(inputs.executors, &island.executor, "ExecutorDescriptor")?;
        executors.insert(island.executor.clone(), d.clone());
    }
    let mut sinks: BTreeMap<Ident, SinkDescriptor> = BTreeMap::new();
    for output in &spec.outputs {
        let d = supplied(inputs.sinks, &output.id, "Sink instance")?.descriptor();
        sinks.insert(output.id.clone(), d.clone());
    }
    let graph_links: Vec<(PortRef, PortRef, BackPressure)> = spec
        .graph
        .links
        .iter()
        .map(|l| (l.from.clone(), l.to.clone(), l.policy))
        .collect();
    let feeds = output_links(spec, inputs, &admission.matched);
    let feed_links: Vec<(PortRef, PortRef, BackPressure)> =
        feeds.iter().map(|d| (d.from.clone(), d.to.clone(), d.policy)).collect();
    admit_islands(&IslandContext {
        islands: &profile.placements.islands,
        components: &spec.graph.components,
        placements: &profile.placements.components,
        executors: &executors,
        links: inputs.registry.link_descriptors(),
        link_placements: &profile.placements.links,
        graph_links: &graph_links,
        feed_links: &feed_links,
        sinks: &sinks,
        resource_endpoints: &spec.resources.keys().cloned().collect(),
    })
    .map_err(|e| SpecError::Structural { reason: e.message })?;

    // Explicit ordering edges from `ezsdr.arm_order` (SB-26, SB-39).
    //
    // A malformed entry is refused, not skipped. SB-39 exists because `v3.0.20`
    // records that with a shared PPS the device that sources it must be started
    // first or start-up fails, and its answer is that the order is a property of the
    // plan. An entry whose `before`/`after` is missing or misspelled contributed no
    // edge and left the fragments in name order, so `zsource`/`asink` armed the PPS
    // source second — the exact v3 failure — with no diagnostic anywhere. This
    // follows MA-41's clause for the sections the Kernel reads: ignoring a
    // misspelling turns it into agreement.
    if let Some(section) = profile.section("ezsdr.arm_order") {
        let items = section.as_array().ok_or_else(|| SpecError::Structural {
            reason: "SB-39: ezsdr.arm_order is present and is not an array".to_owned(),
        })?;
        for item in items {
            let (Some(b), Some(a)) = (
                item.get("before").and_then(|v| v.as_str()),
                item.get("after").and_then(|v| v.as_str()),
            ) else {
                return Err(SpecError::Structural {
                    reason: format!(
                        "SB-39: ezsdr.arm_order entry {item} does not name both `before` and \
                         `after` as strings"
                    ),
                });
            };
            edges.push((Ident::parse(b)?, Ident::parse(a)?));
        }
    }

    let names: Vec<Ident> = fragments.iter().map(|f| f.id.clone()).collect();
    let order = arm_order(&names, &edges)?;
    fragments.sort_by_key(|f| order.iter().position(|n| *n == f.id).unwrap_or(usize::MAX));

    // The plan carries every data link a Link Module is created for (MA-27a): the
    // graph's, then the output feeds', numbered on from them so no two share an id
    // (D75).
    let mut links = declared_links(spec, inputs, &admission.matched)?;
    let base = links.len() as u32;
    links.extend(feeds.into_iter().enumerate().map(|(i, mut feed)| {
        feed.id = DataLinkId::local(base + i as u32);
        feed
    }));
    Ok(ExecutionPlan { fragments, links, deps: edges, authority, class, transfer_costs,
    })
}

/// Derives the ExecutionClass from the environment and cross-checks it against the
/// Authority's pacing (MA-41, SB-39).
fn derive_class(profile: &BindingProfile, pacing: Pacing) -> Result<ExecutionClass, SpecError> {
    // MA-41 exists because "a class that was merely declared could lie". A section
    // the Kernel cannot parse is refused rather than defaulted: defaulting turns a
    // misspelled `over_the_air` into `Simulation`, the one class that may claim
    // determinism (RS-42).
    let rf = match profile.section("ezsdr.rf_path") {
        None => RfPath::Simulated,
        Some(section) => match section.get("path").and_then(|v| v.as_str()) {
            Some("simulated") => RfPath::Simulated,
            Some("cabled") => RfPath::Cabled,
            Some("over_the_air") => RfPath::OverTheAir,
            other => {
                return Err(SpecError::Structural {
                    reason: format!(
                        "MA-41: ezsdr.rf_path.path must be simulated, cabled or over_the_air, not {other:?}"
                    ),
                });
            }
        },
    };
    let derived = ExecutionClass::derive(pacing, rf)
        .map_err(|e| SpecError::Structural { reason: e.message })?;

    // SB-26 lists `ezsdr.time` as a section the Kernel reads, and Vision §8 writes
    // `time: { class: simulation }` in the environment. MA-41's whole argument is
    // that "a class that was merely declared could lie", which is only a rule if the
    // declaration is compared with the derivation; nothing read the section at all,
    // so a profile could declare `simulation` on a Hardware Run and be believed by
    // every reader of its own text while the Manifest recorded something else.
    if let Some(section) = profile.section("ezsdr.time") {
        // MA-41 in full: "an absent, non-string or unrecognised `class` is refused
        // rather than ignored". Reading it through `and_then(as_str)` refused the
        // unrecognised *value* alone, so the two cases the clause names first —
        // `{ "clas": "hardware" }` and `{ "class": 3 }` — planned as `Simulation`,
        // which is the class that may claim determinism (RS-42). That is the whole
        // failure the clause was written against: a misspelling turning into
        // agreement.
        let declared = section
            .get("class")
            .ok_or_else(|| SpecError::Structural {
                reason: "MA-41: ezsdr.time is present and declares no `class`".to_owned(),
            })?
            .as_str()
            .ok_or_else(|| SpecError::Structural {
                reason: "MA-41: ezsdr.time.class is not a string".to_owned(),
            })?;
        {
            let matches_derived = match derived {
                ExecutionClass::Simulation => declared == "simulation",
                ExecutionClass::RealtimeEmulation => declared == "realtime_emulation",
                ExecutionClass::HardwareInLoop => declared == "hardware_in_loop",
                ExecutionClass::Hardware => declared == "hardware",
            };
            if !matches_derived {
                return Err(SpecError::Structural {
                    reason: format!(
                        "MA-41: ezsdr.time.class declares {declared:?}, and the class derived \
                         from the Authority's pacing and ezsdr.rf_path is \
                         {derived:?}"
                    ),
                });
            }
        }
    }
    Ok(derived)
}

/// The instances this one must be armed after, mapped from `ResourceId`s back to
/// binding names (SB-39).
fn arm_after_of(name: &Ident, spec: &ExperimentSpec, inputs: &CompileInputs<'_>) -> Vec<Ident> {
    let Some(provider) = inputs.providers.get(name) else { return Vec::new();
    };
    // Only resource slots' Providers are read (SB-22); an entry naming an instance
    // this profile does not bind contributes no edge (SB-39, D49).
    let mut out: Vec<Ident> = provider
        .instance()
        .arm_after
        .iter()
        .filter_map(|rid| {
            spec.resources.keys().find(|other| {
                inputs.providers.get(*other).is_some_and(|p| p.instance().id == *rid)
            })
        })
        .cloned()
        .collect();
    out.sort();
    out
}

/// The DataLink declarations the plan carries (SB-39, SC-19).
/// The links that feed the bound Sinks: one per `outputs[]` entry. An output *is*
/// the declaration of its link, so SC-21's drop-class rule applies to these and not
/// to `graph.links`, whose consumers are components (SB-17, SC-19, SC-21).
fn output_links(
    spec: &ExperimentSpec,
    inputs: &CompileInputs<'_>,
    matched: &BTreeMap<Ident, ResourceId>,
) -> Vec<DataLinkDecl> {
    spec.outputs
        .iter()
        .enumerate()
        .map(|(i, o)| DataLinkDecl {
            id: DataLinkId::local(i as u32),
            from: o.feed.port.clone(),
            to: feed_end(&o.id),
            contract: endpoint_port(&o.feed.port, spec, inputs, matched)
                .map(|(_, c)| c)
                .unwrap_or_else(|| {
                    crate::contract::DataContractId::parse("ezsdr.control")
                        .expect("a valid literal")
                }),
            policy: o.feed.policy,
            capacity: o.feed.capacity,
        })
        .collect()
}

/// The plan's `links`. The contract is resolved the way `validate` resolved it for
/// SC-3 — through [`endpoint_port`], which reads a resource endpoint's port off the
/// **bound node** — and not off `graph.components` alone. Resolving components only
/// and defaulting to `ezsdr.control` gave every link whose consumer is a resource
/// port, which is Vision §7's own `PHY → Radio Port` example, a contract the Kernel
/// had already checked as something else: the plan reaches the Manifest (RS-38) and is
/// what a Link Module's `create` receives (MA-27), so the default was a wrong answer
/// rather than a missing one.
///
/// Rule: SB-39, SB-15, SC-3, MA-27.
fn declared_links(
    spec: &ExperimentSpec,
    inputs: &CompileInputs<'_>,
    matched: &BTreeMap<Ident, ResourceId>,
) -> Result<Vec<DataLinkDecl>, SpecError> {
    spec.graph
        .links
        .iter()
        .enumerate()
        .map(|(i, l)| {
            // `plan` re-ran SB-15 over the same `matched` before this (D99), so a `None`
            // here is unreachable; it stays an error rather than a panic.
            let (_, contract) = endpoint_port(&l.to, spec, inputs, matched).ok_or_else(|| {
                SpecError::Structural {
                    reason: format!(
                        "SB-15: link {i}'s consumer {}.{} names no port of a component or of \
                         a bound node",
                        l.to.component, l.to.port
                    ),
                }
            })?;
            Ok(DataLinkDecl {
                id: DataLinkId::local(i as u32),
                from: l.from.clone(),
                to: l.to.clone(),
                contract,
                policy: l.policy,
                capacity: l.capacity,
            })
        })
        .collect()
}

/// SB-39's guard: an `AdmissionResult` is this Run's only if every Spec resource has a
/// matched node and that node is one the resource's bound instance declares.
/// "Admitted" alone is satisfied by an empty result, and a stale one names nodes of an
/// instance this profile no longer binds (D99).
///
/// Rule: SB-39, SB-30.
fn admission_is_this_runs(
    spec: &ExperimentSpec,
    inputs: &CompileInputs<'_>,
    admission: &AdmissionResult,
) -> Result<(), String> {
    for name in spec.resources.keys() {
        let Some(node) = admission.matched.get(name) else {
            return Err(format!("this `AdmissionResult` is not this Run's: {name} has no matched node"));
        };
        let declared = inputs
            .providers
            .get(name)
            .is_some_and(|p| p.instance().tree.walk().into_iter().any(|n| n.id == *node));
        if !declared {
            return Err(format!(
                "this `AdmissionResult` is not this Run's: {name}'s matched node {node} is not one \
                 its bound instance declares"
            ));
        }
    }
    Ok(())
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

/// `prepare(plan, ctx)` calls each fragment in dependency order and collects one
/// `PrepareReport` per fragment plus a merged effective configuration, then runs
/// every registered admission check against that applied configuration — SB-30's
/// second point, which exists because a coercion can move an applied value outside
/// a limit the requested value respected. Any fragment's failure fails the whole
/// transaction, and the Run moves to cleanup.
///
/// The checks are run here rather than by the caller so that SB-30's second point
/// cannot be forgotten.
///
/// Rule: SB-30, SB-41, SB-42.
pub fn collect_prepare(
    reports: Vec<Result<PrepareReport, ModuleError>>,
    spec: &ExperimentSpec,
    profile: &BindingProfile,
    inputs: &CompileInputs<'_>,
    admission: &AdmissionResult,
) -> Result<MergedPrepare, PrepareError> {
    let checks = inputs.checks;
    let environment = &profile.environment;
    let spec_coercion = &spec.policies.coercion;
    let registry = inputs.registry;
    let is_session = inputs.is_session;
    let providers = inputs.providers;
    // SB-30: `plan` is not the only gate, because `collect_prepare` does not require
    // `plan`'s output. A caller that skipped `plan` would run `prepare` against a
    // rejected admission and drop every `validate` violation on the floor.
    if !admission.is_admitted() {
        return Err(PrepareError::Violations(admission.violations.clone()));
    }
    // SB-39's guard, as `plan` applies it: without it a default, stale or foreign
    // result would make every per-resource lookup below miss, and MA-12's narrowing
    // check, SB-44's preview comparison and the constraint re-match would all be
    // skipped in silence while a report applied any value at all (D99).
    if let Err(reason) = admission_is_this_runs(spec, inputs, admission) {
        return Err(PrepareError::Violations(vec![crate::binding::Violation {
            check: crate::spec::Namespace::parse("ezsdr.effective").expect("a valid literal"),
            key: None,
            requested: None,
            reason: format!("SB-39: {reason}"),
        }]));
    }
    let mut ok = Vec::with_capacity(reports.len());
    for (index, r) in reports.into_iter().enumerate() {
        match r {
            Ok(report) => ok.push(report),
            Err(error) => return Err(PrepareError::Fragment { index, error }),
        }
    }
    let mut violations = Vec::new();
    // SB-46 names "the stage", not "the validate stage": a Provider whose `prepare`
    // reports a coercion on a `reject` key must be refused here as well. SB-44 makes
    // the two agree only as a producer obligation, so the gap is reachable.
    // SB-46 in full: under `reject` the stage fails, under `warn` the coercion is
    // applied, recorded **and warned**, under `accept` applied and recorded. Testing
    // only `.is_err()` computed the warning and threw it away, so `warn` — which is
    // every key on the Session path (SB-45) — recorded the coercion and warned about
    // nothing. The warning goes on the fragment's own report, which already carries a
    // `warnings` field and is already in the schema.
    for r in &mut ok {
        // SB-44: a `Coercion` whose key the fragment never requested is a **malformed
        // report** — nothing was requested, so nothing was coerced, and a Provider
        // choosing its own default is MA-12's narrowing case, which belongs in
        // `effective` alone. Left to the policy loop, the key's default fired for a
        // key nobody named, and the Manifest recorded a "coercion" of a value nobody
        // asked for (SB-42, SB-46).
        if let Some(req) = spec.resources.get(&r.fragment) {
            if let Some(stray) =
                r.coercions.iter().find(|c| !req.requires.contains_key(&c.key))
            {
                violations.push(crate::binding::Violation {
                    check: crate::spec::Namespace::parse("ezsdr.coercion")
                        .expect("a valid literal"),
                    key: Some(stray.key.clone()),
                    requested: Some(stray.requested.clone()),
                    reason: format!(
                        "SB-44: {} reports a coercion of {}, which its request does not name",
                        r.fragment, stray.key
                    ),
                });
            }
        }
        let mut warned = Vec::new();
        for c in &r.coercions {
            let policy = coercion_policy(
                spec_coercion.get(&c.key).copied(),
                is_session,
                registry.key_decl(&c.key).ok(),
            );
            match apply_coercion(policy, c) {
                Ok(None) => {}
                Ok(Some(w)) => warned.push(w),
                Err(_) => violations.push(crate::binding::Violation {
                    check: crate::spec::Namespace::parse("ezsdr.coercion")
                        .expect("a valid literal"),
                    key: Some(c.key.clone()),
                    requested: Some(c.requested.clone()),
                    reason: format!("SB-46: coercion to {:?} rejected at prepare", c.applied),
                }),
            }
        }
        r.warnings.extend(warned);
    }
    let merged = MergedPrepare::from_reports(ok);
    violations
        .extend(checks.run(environment, &merged.effective, &BTreeMap::new(), CheckStage::Prepare,
    ));

    // MA-12's second sentence, both halves. `effective` may **narrow** a declared
    // capability and must not widen one, and the Spec's own constraints are re-matched
    // against it — otherwise a Provider whose `prepare` disagrees with its `coerce`
    // puts a value the Spec never asked for into `run.effective()` and the Manifest,
    // and MA-12 is a rule with a predicate nobody calls (SB-30, SB-44).
    for (name, req) in &spec.resources {
        let Some(node_id) = admission.matched.get(name) else { continue;
        };
        let Some(provider) = providers.get(name) else { continue;
        };
        let instance = provider.instance();
        let Some(node) = instance.tree.walk().into_iter().find(|n| n.id == *node_id) else {
            continue;
        };
        // Each resource is judged by **its own** report, never by `merged.effective`:
        // SB-41 says the merge lets a later fragment's value win for a key two
        // fragments both name and that the Kernel does not interpret it. Reading the
        // merge here interpreted it per resource against data that cannot tell two
        // resources apart, so two channels asking their own line's declared rate
        // refused each other. A Provider fragment's id is the resource name.
        let Some(report) = merged.reports.iter().find(|r| r.fragment == *name) else {
            continue;
        };
        for (key, declared) in &node.capabilities {
            let Some(applied) = report.effective.get(key) else { continue;
            };
            let effective = crate::spec::CapabilityValue::One { value: applied.clone(),
            };
            if let Err(e) = check_effective_narrows(declared, &effective) {
                violations.push(crate::binding::Violation {
                    check: crate::spec::Namespace::parse("ezsdr.effective")
                        .expect("a valid literal"),
                    key: Some(key.clone()),
                    requested: Some(applied.clone()),
                    reason: e.message,
                });
            }
        }
        for (key, constraint) in &req.requires {
            // SB-46: a coercion the **Kernel** admitted at `validate` was already
            // judged by the policy loop above, and a coercion is by definition a key
            // whose applied value does not satisfy what was requested — so
            // re-matching it here would refuse every coercion by construction and
            // leave SB-46's `accept` and `warn` branches unreachable. MA-12 catches an
            // **undeclared** change; a declared one is SB-46's.
            //
            // The exclusion is keyed on `admission.coercions_preview`, which the
            // Kernel computed by calling `coerce` itself (MA-11), and **not** on the
            // report's own `coercions` list: keying it on the report made naming a key
            // there a self-issued exemption, so a Provider could put any value its
            // node declares — or any value at all, for a key the node does not declare
            // — into `effective` and the Manifest, while the Manifest recorded the
            // substitution as a legitimate coercion. What is checked instead is
            // SB-44's own obligation: the value applied is the one `coerce` returned.
            //
            // Scoped to **this resource**. A key alone does not identify a preview:
            // two resources may constrain one key on two devices, and `coerce` is
            // called per bound node (SB-7). Matching on the key alone charged `a`'s
            // coercion to `b`, so `b` — satisfied directly, never passed to `coerce` —
            // was refused for not applying a value nobody computed for it. That is the
            // ordinary two-channel Spec whenever one channel coerces.
            if let Some(preview) = admission
                .coercions_preview
                .iter()
                .filter(|p| p.resource == *name)
                .map(|p| &p.coercion)
                .find(|c| c.key == *key)
            {
                if let Some(applied) = report.effective.get(key) {
                    if *applied != preview.applied {
                        violations.push(crate::binding::Violation {
                            check: crate::spec::Namespace::parse("ezsdr.effective")
                                .expect("a valid literal"),
                            key: Some(key.clone()),
                            requested: Some(applied.clone()),
                            reason: format!(
                                "SB-44: {name}'s effective {key} is {applied:?}, and `coerce` \
                                 returned {:?} for it at validate",
                                preview.applied
                            ),
                        });
                    }
                }
                continue;
            }
            let Some(applied) = report.effective.get(key) else { continue;
            };
            let effective = crate::spec::CapabilityValue::One { value: applied.clone(),
            };
            if !satisfies(constraint, &effective).unwrap_or(false) {
                violations.push(crate::binding::Violation {
                    check: crate::spec::Namespace::parse("ezsdr.effective")
                        .expect("a valid literal"),
                    key: Some(key.clone()),
                    requested: Some(applied.clone()),
                    reason: format!(
                        "MA-12 (re-match): {name}'s effective {key} does not satisfy the \
                         Spec's constraint"
                    ),
                });
            }
        }
    }
    if violations.is_empty() { Ok(merged) } else { Err(PrepareError::Violations(violations)) }
}
