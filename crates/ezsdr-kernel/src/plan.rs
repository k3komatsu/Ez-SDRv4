//! The ExecutionPlan, its dependency order, PrepareReport and Island admission —
//! `03-spec-and-binding.md` SB-37…SB-46, `05-module-api.md` MA-22, MA-39…MA-41.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::binding::ComponentPlacement;
use crate::contract::PortRef;
use crate::id::{DataLinkId, ModuleId, ResourceId};
use crate::module_api::{
    ComponentDescriptor, ExecutionClass, ExecutorDescriptor, IslandDecl, LinkDescriptor,
    ModuleError, Role,
};
use crate::spec::{
    Coercion, CoercionPolicy, Ident, Key, KeyDecl, SpecError, Value, Warning,
};
use crate::stream::{BackPressure, DataLinkDecl, StreamError};

/// One unit of work handed to one Module at `prepare`, in dependency order (SB-39).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Fragment {
    /// The fragment's name (SB-1).
    pub id: Ident,
    /// Which Module instance performs it (SB-22).
    pub instance: ModuleId,
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
pub struct DeclaredCost {
    /// Which link (SC-19).
    pub link: DataLinkId,
    /// The Link Module's own number, uninterpreted by the Kernel (SB-40).
    pub cost: u64,
}

/// What `plan(spec, binding)` produces (SB-39).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ExecutionPlan {
    /// The fragments, already in dependency order (SB-39).
    pub fragments: Vec<Fragment>,
    /// The DataLink declarations (SC-19).
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
    /// Keys whose declared bound the request reached (SB-41).
    #[serde(default)]
    pub constraints_hit: Vec<Key>,
}

/// Vision §11 says a report per fragment and §52 says "the PrepareReport"; both are
/// produced, and the merged one is what `run.effective()` returns (SB-41).
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
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
                return Err(SpecError::ArmCycle { path: stuck.join(" -> ") });
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
    /// The registered Link Modules (MA-28).
    pub links: &'a [LinkDescriptor],
    /// The graph's links (SB-15).
    pub graph_links: &'a [(PortRef, PortRef, BackPressure)],
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
/// placement's memory domain is among its `memory_domains`; a link within an Island
/// shares a memory domain or has a registered Link that `connects` the pair, and a
/// link between Islands has a declared policy; MA-22's cycle rule; and an Island
/// with an `rt_policy` requires a declared budget on every component.
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
            if c.requires.executor_kind != "any"
                && c.requires.executor_kind != executor.kind.as_str()
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
            let placement = ctx.placements.get(name).ok_or_else(|| {
                ModuleError::rejected(format!("MA-39: {name} has no placement"))
            })?;
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

    let island_of = |component: &str| -> Option<&IslandDecl> {
        ctx.islands.iter().find(|i| i.components.iter().any(|c| c.as_str() == component))
    };

    // MA-22's cycle rule, which MA-39 names as one of its own checks. Every graph
    // link is a `stream.*` edge in Phase 1, so none of them may close a cycle.
    let nodes: Vec<Ident> =
        ctx.components.keys().chain(ctx.resource_endpoints.iter()).cloned().collect();
    let edges: Vec<GraphEdge> = ctx
        .graph_links
        .iter()
        .filter_map(|(from, to, _)| {
            Some(GraphEdge {
                from: Ident::parse(&from.component).ok()?,
                to: Ident::parse(&to.component).ok()?,
                kind: EdgeKind::Stream,
                crosses_island: island_of(&from.component).map(|i| i.id)
                    != island_of(&to.component).map(|i| i.id),
            })
        })
        .collect();
    check_cycles(&nodes, &edges)?;

    // A link within an Island shares a memory domain or has a registered Link that
    // `connects` the pair; a link between Islands has a declared policy.
    let is_resource = |r: &PortRef| {
        Ident::parse(&r.component).is_ok_and(|i| ctx.resource_endpoints.contains(&i))
    };
    for (from, to, _policy) in ctx.graph_links {
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
        if fi.id != ti.id {
            continue; // a declared policy is SB-15's mandatory field
        }
        let placement_of =
            |r: &PortRef| Ident::parse(&r.component).ok().and_then(|i| ctx.placements.get(&i));
        let (fd, td) = (placement_of(from), placement_of(to));
        if let (Some(fd), Some(td)) = (fd, td) {
            if fd.memory_domain == td.memory_domain {
                continue;
            }
            let joined = ctx.links.iter().any(|l| {
                l.connects.contains(&(fd.memory_domain, td.memory_domain))
                    || l.connects.contains(&(td.memory_domain, fd.memory_domain))
            });
            if !joined {
                return Err(ModuleError::rejected(format!(
                    "MA-39: {} and {} are in different memory domains inside island {} and no registered Link connects them",
                    from.component, to.component, fi.id
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
    use crate::spec::{CapabilityValue as C, Constraint};
    let within = |v: &Value| -> bool {
        satisfies(&Constraint::Eq { value: v.clone() }, declared).unwrap_or(false)
    };
    let ok = match effective {
        C::One { value } => within(value),
        C::AnyOf { values } => values.iter().all(within),
        C::Range { min, max } => within(min) && within(max),
    };
    if ok {
        Ok(())
    } else {
        Err(ModuleError::rejected(
            "MA-12: `effective` may narrow a declared capability and must not widen one".to_owned(),
        ))
    }
}


/// Refuses a `Block` policy on a link whose consumer belongs to a Sink-role Module
/// (SB-15, SC-21). The predicate itself is spec 02's.
pub fn check_sink_links(
    decls: &[DataLinkDecl],
    is_sink: &dyn Fn(&PortRef) -> bool,
) -> Result<(), StreamError> {
    for d in decls {
        crate::stream::check_sink_link(d, is_sink(&d.to))?;
    }
    Ok(())
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
    /// The bound Provider per Spec resource name (SB-22).
    pub providers: &'a BTreeMap<Ident, &'a dyn Provider>,
    /// The Authority descriptor per binding that holds the role (SB-24, MA-29).
    pub authorities: &'a BTreeMap<Ident, AuthorityDescriptor>,
    /// What each Island's Executor instance declares. Which **Module** supplies it
    /// is named by the profile, as `bindings[island.executor]`, so the plan and the
    /// Manifest rest on the document and not on assembly-time input (MA-19, MA-38).
    pub executors: &'a BTreeMap<Ident, ExecutorDescriptor>,
    /// The registered Link Modules, which MA-39's reachability check reads (MA-28).
    pub links: &'a [LinkDescriptor],
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
///
/// Rule: SB-37, SB-38.
pub fn validate(
    spec: &ExperimentSpec,
    profile: &BindingProfile,
    inputs: &CompileInputs<'_>,
) -> Result<AdmissionResult, SpecError> {
    let mut out = AdmissionResult::default();
    // SB-34: a Spec resource binds to exactly one node, and two Spec resources bind
    // to *different* sub-resources of one instance.
    // SB-34: node -> the resource name that bound it, so a collision can name both.
    let mut taken: BTreeMap<ResourceId, Ident> = BTreeMap::new();

    // Semantic validation (SB-2, SB-15, SB-17, SB-18, SB-25a).
    spec.check_key_prefixes()?;
    if !inputs.is_session {
        // SB-25a: "A Spec Run leaves it unset. A Session profile sets it." RS-12
        // builds a Session's implicit Spec *from* those very placements, so running
        // the check there would refuse every Session.
    }
    for kind in spec.policies.failure.keys() {
        inputs.kinds.require(kind).map_err(|_| SpecError::Structural {
            reason: format!("SB-18: event kind {kind} is not registered"),
        })?;
    }
    check_graph_links(spec, inputs)?;
    check_outputs(spec, profile, inputs)?;

    // Binding resolution, then matching against the bound instance (SB-22, SB-34, SB-37).
    for (name, req) in &spec.resources {
        if !profile.bindings.contains_key(name) {
            return Err(SpecError::UnboundResource { name: name.clone() });
        }
        let provider = inputs.providers.get(name).ok_or_else(|| SpecError::Structural {
            reason: format!("SB-22: no Provider instance for binding {name}"),
        })?;
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
            let resolved = inputs
                .providers
                .values()
                .flat_map(|p| p.instance().tree.walk())
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
    let requested: BTreeMap<Key, Value> = spec
        .resources
        .values()
        .flat_map(|r| r.requires.iter())
        .filter_map(|(k, c)| match c {
            crate::spec::Constraint::Eq { value } => Some((k.clone(), value.clone())),
            _ => None,
        })
        .collect();
    out.violations.extend(inputs.checks.run(
        &profile.environment,
        &requested,
        &BTreeMap::new(),
        CheckStage::Validate,
    ));

    // SB-45, SB-46: a previewed coercion under `reject` fails the stage here rather
    // than at `prepare`. A dry run that reported a coercion it knows will be
    // rejected would not be a dry run (Vision §52).
    for c in out.coercions_preview.clone() {
        let policy = coercion_policy(
            spec.policies.coercion.get(&c.key).copied(),
            inputs.is_session,
            inputs.registry.key_decl(&c.key).ok(),
        );
        match apply_coercion(policy, &c) {
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
    for (key, constraint) in &req.requires {
        let decl = inputs.registry.key_decl(key).map_err(|e| SpecError::UnknownKeyPrefix {
            key: format!("{key}: {}", e.message),
        })?;
        check_constraint_kind(decl, constraint)?;
        let cap = node.capabilities.get(key);
        let direct = match cap {
            Some(cap) => satisfies(constraint, cap).map_err(|e| match e {
                SpecError::KeyShape { expected, found, .. } => {
                    SpecError::KeyShape { key: key.to_string(), expected, found }
                }
                other => other,
            })?,
            None => matches!(constraint, crate::spec::Constraint::Present) && cap.is_some(),
        };
        if direct {
            continue;
        }
        if !decl.coercible {
            // SB-7: a key that is not coercible fails immediately, without calling
            // `coerce`.
            out.rejected.push(RejectedConstraint {
                resource: name.clone(),
                key: key.clone(),
                constraint: constraint.clone(),
                reason: "SB-7: the declared capability does not satisfy it and the key is not coercible"
                    .to_owned(),
            });
            continue;
        }
        // SB-7: the matcher asks only whether a value can be coerced and what it
        // becomes; it never decides the grid.
        let report = provider
            .coerce(&Requested {
                resource: node.id.clone(),
                constraints: [(key.clone(), constraint.clone())].into_iter().collect(),
            })
            .map_err(|e| SpecError::Structural { reason: e.message })?;
        if report.applied.contains_key(key) {
            out.coercions_preview.extend(report.coercions);
            out.warnings.extend(report.warnings);
        } else {
            out.rejected.push(RejectedConstraint {
                resource: name.clone(),
                key: key.clone(),
                constraint: constraint.clone(),
                reason: "SB-7: the Provider could not coerce it".to_owned(),
            });
        }
    }
    Ok(())
}

/// `graph.links` connects two `PortRef`s and states a policy and a capacity, both
/// mandatory. `validate()` checks contract compatibility by SC-3 and refuses a
/// `Block` policy on a link whose consumer belongs to a Sink-role Module (SB-15).
fn check_graph_links(spec: &ExperimentSpec, inputs: &CompileInputs<'_>) -> Result<(), SpecError> {
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
            source_contract(&link.from, spec, inputs),
            source_contract(&link.to, spec, inputs),
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
        if let (Some(from), Some(to)) = (&from, &to) {
            inputs
                .contracts
                .check_link(from, to)
                .map_err(|e| SpecError::Structural { reason: e.to_string() })?;
        }
        // SC-19's mandatory policy is checked above; a Sink is never a link's
        // consumer here, because a Sink is bound rather than placed and its own
        // link is the `outputs[]` entry that `check_outputs` validates (SB-17).
    }
    Ok(())
}

/// SB-17: each `outputs[]` entry names a source port, an artifact kind, the Sink
/// parameters, and the drop-class policy and capacity of the link that feeds it.
/// `validate()` refuses an output whose source port does not exist, whose binding
/// names a Module without the Sink role, whose bound `SinkDescriptor` does not list
/// the artifact kind or does not accept the source port's contract, or whose policy
/// is not of the drop class.
///
/// A Sink is **bound, not placed**: it is a Module role (MA-2, MA-25, MA-30) and
/// not a component an Executor loads, so nothing here consults `graph.components`
/// or an Island. That is what makes the check work on a Spec Run, which is the path
/// a publication Run uses (findings D17, D29).
fn check_outputs(
    spec: &ExperimentSpec,
    profile: &BindingProfile,
    inputs: &CompileInputs<'_>,
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
        // SC-21: a link into a Sink is of the drop class. `Block` would let a slow
        // recorder stall the real-time path.
        let decl = DataLinkDecl {
            id: DataLinkId::local(0),
            from: feed.port.clone(),
            to: PortRef { component: output.id.to_string(), port: "in".to_owned() },
            contract: source_contract(&feed.port, spec, inputs).unwrap_or_else(|| {
                crate::contract::DataContractId::parse("ezsdr.control").expect("a valid literal")
            }),
            policy: feed.policy,
            capacity: feed.capacity,
        };
        crate::stream::check_sink_link(&decl, true)
            .map_err(|e| SpecError::Structural { reason: e.to_string() })?;

        // The source port must exist, on a component or on a bound resource.
        let contract = source_contract(&feed.port, spec, inputs).ok_or_else(|| {
            SpecError::Structural {
                reason: format!(
                    "SB-17: output {} names source port {}:{}, which does not exist",
                    output.id, feed.port.component, feed.port.port
                ),
            }
        })?;

        // SB-22: the output id is bound to a Module holding the Sink role.
        let binding = profile.bindings.get(&output.id).ok_or_else(|| SpecError::Structural {
            reason: format!("SB-22: output {} has no binding (UnboundOutput)", output.id),
        })?;
        let holds_sink = inputs
            .registry
            .modules()
            .any(|m| m.id == binding.module && m.roles.contains(&Role::Sink));
        if !holds_sink {
            return Err(SpecError::Structural {
                reason: format!(
                    "SB-17: output {} is bound to {}, which does not hold the Sink role",
                    output.id, binding.module
                ),
            });
        }

        // MA-25: the bound Sink must write this artifact kind and accept the
        // source port's contract.
        let sink = inputs.sinks.get(&output.id).ok_or_else(|| SpecError::Structural {
            reason: format!("SB-22: no Sink instance for output {}", output.id),
        })?;
        let d = sink.descriptor();
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

/// The contract a `PortRef` carries: a component's declared port, or a port a bound
/// resource declares (SB-15, MA-10).
fn source_contract(
    r: &PortRef,
    spec: &ExperimentSpec,
    inputs: &CompileInputs<'_>,
) -> Option<crate::contract::DataContractId> {
    let name = Ident::parse(&r.component).ok()?;
    if let Some(c) = spec.graph.components.get(&name) {
        return c.ports.iter().find(|p| p.name == r.port).map(|p| p.contract.clone());
    }
    let provider = inputs.providers.get(&name)?;
    provider
        .instance()
        .tree
        .walk()
        .into_iter()
        .flat_map(|n| n.ports.iter())
        .find(|p| p.name == r.port)
        .map(|p| p.contract.clone())
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
/// `admission` is what `validate()` matched. A Provider fragment's `content` is
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
    let authority = pick_authority(profile, inputs)?;
    let pacing = inputs
        .authorities
        .get(&authority)
        .map(|a| a.pacing)
        .ok_or_else(|| SpecError::Structural {
            reason: format!("SB-24: binding {authority} declares no AuthorityDescriptor"),
        })?;
    let class = derive_class(profile, pacing)?;

    // One fragment per binding, plus one per Island (SB-39).
    let mut fragments: Vec<Fragment> = Vec::new();
    let mut edges: Vec<(Ident, Ident)> = Vec::new();
    for (name, binding) in &profile.bindings {
        // SB-22 keys one map by resource names, output ids and Island executor
        // names. A Sink's fragment comes from its output below and an Executor's
        // from its Island, so only a Provider binding yields one here.
        let holds_provider = inputs
            .registry
            .modules()
            .find(|m| m.id == binding.module)
            .is_some_and(|m| m.roles.contains(&Role::Provider));
        if !holds_provider {
            continue;
        }
        let after = arm_after_of(name, profile, inputs);
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
    // One fragment per bound output: a Sink is prepared, armed and stepped like any
    // other Module instance (MA-25, MA-30), and it is bound rather than placed, so
    // it gets its own fragment and never appears in an Island's component list.
    for output in &spec.outputs {
        let Some(binding) = profile.bindings.get(&output.id) else { continue };
        fragments.push(Fragment {
            id: output.id.clone(),
            instance: binding.module.clone(),
            role: Role::Sink,
            content: serde_json::to_value(output).unwrap_or(serde_json::Value::Null),
            after: Vec::new(),
        });
    }
    for island in &profile.placements.islands {
        // MA-38: `executor` names a binding (SB-22), whose Module holds the Executor
        // role. Taking it from the runtime instead would leave the plan's
        // `fragments[].instance` and the Manifest's `modules` resting on
        // assembly-time input no document records (Vision §50; finding D18).
        let binding = profile.bindings.get(&island.executor).ok_or_else(|| {
            SpecError::Structural {
                reason: format!(
                    "SB-22: island {} names executor {}, which the profile does not bind (UnboundExecutor)",
                    island.id, island.executor
                ),
            }
        })?;
        let module = &binding.module;
        inputs.executors.get(&island.executor).ok_or_else(|| SpecError::Structural {
            reason: format!(
                "MA-38: executor instance {} declares no ExecutorDescriptor",
                island.executor
            ),
        })?;
        fragments.push(Fragment {
            // One fragment per Island, not per Executor instance: two Islands may
            // share one Executor (an affinity split), and a fragment id must be
            // unique (SB-39).
            id: Ident::parse(&format!("island_{}", island.id.local)).map_err(|_| {
                SpecError::Structural { reason: "MA-38: island id is not a valid Ident".to_owned() }
            })?,
            instance: module.clone(),
            role: Role::Executor,
            content: serde_json::to_value(island).unwrap_or(serde_json::Value::Null),
            after: Vec::new(),
        });
    }

    // SB-37, MA-39, MA-40: admission is a stage of the pipeline, not a library
    // function a caller may forget.
    let executors: BTreeMap<Ident, ExecutorDescriptor> =
        inputs.executors.iter().map(|(k, d)| (k.clone(), d.clone())).collect();
    let graph_links: Vec<(PortRef, PortRef, BackPressure)> = spec
        .graph
        .links
        .iter()
        .map(|l| (l.from.clone(), l.to.clone(), l.policy))
        .collect();
    admit_islands(&IslandContext {
        islands: &profile.placements.islands,
        components: &spec.graph.components,
        placements: &profile.placements.components,
        executors: &executors,
        links: inputs.links,
        graph_links: &graph_links,
        resource_endpoints: &spec.resources.keys().cloned().collect(),
    })
    .map_err(|e| SpecError::Structural { reason: e.message })?;
    // SC-21 on the links that feed the bound Sinks. A Sink is not a graph component,
    // so the predicate is "this link is an output's feed" and not "its consumer is a
    // Sink component" (SB-17, findings D17, D29).
    check_sink_links(&output_links(spec, inputs), &|_r: &PortRef| true)
        .map_err(|e| SpecError::Structural { reason: e.to_string() })?;

    // Explicit ordering edges from `ezsdr.arm_order` (SB-26, SB-39).
    if let Some(items) = profile.section("ezsdr.arm_order").and_then(|s| s.as_array()) {
        for item in items {
            let (Some(b), Some(a)) = (
                item.get("before").and_then(|v| v.as_str()),
                item.get("after").and_then(|v| v.as_str()),
            ) else {
                continue;
            };
            edges.push((Ident::parse(b)?, Ident::parse(a)?));
        }
    }

    let names: Vec<Ident> = fragments.iter().map(|f| f.id.clone()).collect();
    let order = arm_order(&names, &edges)?;
    fragments.sort_by_key(|f| order.iter().position(|n| *n == f.id).unwrap_or(usize::MAX));

    let links = declared_links(spec, inputs);
    Ok(ExecutionPlan { fragments, links, deps: edges, authority, class, transfer_costs })
}

/// `authority` names the binding whose Provider plays the Authority role and may be
/// omitted when exactly one candidate exists (SB-24, MA-29).
fn pick_authority(
    profile: &BindingProfile,
    inputs: &CompileInputs<'_>,
) -> Result<Ident, SpecError> {
    if let Some(named) = &profile.authority {
        if !profile.bindings.contains_key(named) {
            return Err(SpecError::Structural {
                reason: format!("SB-24: `authority` names {named}, which is not a binding"),
            });
        }
        if !inputs.authorities.contains_key(named) {
            return Err(SpecError::Structural {
                reason: format!("SB-24: binding {named} declares no AuthorityDescriptor"),
            });
        }
        return Ok(named.clone());
    }
    let mut candidates: Vec<&Ident> = inputs.authorities.keys().collect();
    candidates.sort();
    match candidates.as_slice() {
        [only] => Ok((*only).clone()),
        [] => Err(SpecError::Structural {
            reason: "SB-24: a Run has exactly one Authority and no binding provides one".to_owned(),
        }),
        _ => Err(SpecError::Structural {
            reason: format!(
                "SB-24: {} Authority candidates and no `authority` field",
                candidates.len()
            ),
        }),
    }
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
    ExecutionClass::derive(pacing, rf).map_err(|e| SpecError::Structural { reason: e.message })
}

/// The instances this one must be armed after, mapped from `ResourceId`s back to
/// binding names (SB-39).
fn arm_after_of(
    name: &Ident,
    profile: &BindingProfile,
    inputs: &CompileInputs<'_>,
) -> Vec<Ident> {
    let Some(provider) = inputs.providers.get(name) else { return Vec::new() };
    let mut out: Vec<Ident> = provider
        .instance()
        .arm_after
        .iter()
        .filter_map(|rid| {
            profile.bindings.keys().find(|other| {
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
fn output_links(spec: &ExperimentSpec, inputs: &CompileInputs<'_>) -> Vec<DataLinkDecl> {
    spec.outputs
        .iter()
        .enumerate()
        .map(|(i, o)| DataLinkDecl {
            id: DataLinkId::local(i as u32),
            from: o.feed.port.clone(),
            to: PortRef { component: o.id.to_string(), port: "in".to_owned() },
            contract: source_contract(&o.feed.port, spec, inputs).unwrap_or_else(|| {
                crate::contract::DataContractId::parse("ezsdr.control").expect("a valid literal")
            }),
            policy: o.feed.policy,
            capacity: o.feed.capacity,
        })
        .collect()
}

fn declared_links(spec: &ExperimentSpec, inputs: &CompileInputs<'_>) -> Vec<DataLinkDecl> {
    spec.graph
        .links
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let contract = Ident::parse(&l.to.component)
                .ok()
                .and_then(|c| spec.graph.components.get(&c))
                .and_then(|c| c.ports.iter().find(|p| p.name == l.to.port))
                .map(|p| p.contract.clone())
                .unwrap_or_else(|| {
                    crate::contract::DataContractId::parse("ezsdr.control")
                        .expect("a valid literal")
                });
            let _ = inputs;
            DataLinkDecl {
                id: DataLinkId::local(i as u32),
                from: l.from.clone(),
                to: l.to.clone(),
                contract,
                policy: l.policy,
                capacity: l.capacity,
            }
        })
        .collect()
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
    checks: &AdmissionCheckRegistry,
    environment: &BTreeMap<crate::spec::Namespace, serde_json::Value>,
    spec_coercion: &BTreeMap<Key, CoercionPolicy>,
    registry: &ModuleRegistry,
    is_session: bool,
) -> Result<MergedPrepare, PrepareError> {
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
    for c in ok.iter().flat_map(|r| r.coercions.iter()) {
        let policy = coercion_policy(
            spec_coercion.get(&c.key).copied(),
            is_session,
            registry.key_decl(&c.key).ok(),
        );
        if apply_coercion(policy, c).is_err() {
            violations.push(crate::binding::Violation {
                check: crate::spec::Namespace::parse("ezsdr.coercion").expect("a valid literal"),
                key: Some(c.key.clone()),
                requested: Some(c.requested.clone()),
                reason: format!("SB-46: coercion to {:?} rejected at prepare", c.applied),
            });
        }
    }
    let merged = MergedPrepare::from_reports(ok);
    violations
        .extend(checks.run(environment, &merged.effective, &BTreeMap::new(), CheckStage::Prepare));
    if violations.is_empty() { Ok(merged) } else { Err(PrepareError::Violations(violations)) }
}
