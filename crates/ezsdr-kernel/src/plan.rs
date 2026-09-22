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
    let nodes: Vec<Ident> = ctx.components.keys().cloned().collect();
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
    for (from, to, _policy) in ctx.graph_links {
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

/// Component names whose supplying Module holds the Sink role, which is SC-21's
/// predicate. The Kernel reads the role from the `ModuleDescriptor` of the Module
/// that supplies the component, which for a Session comes from the placement's
/// `module` field.
///
/// On a **Spec Run** this is always empty: SB-25a says a Spec Run leaves `module`
/// unset, and `ComponentDescriptor.kind` is only `Processor | Reactor`, so nothing
/// in either document says which component is a Sink. SB-15's `Block`-into-Sink
/// refusal and SB-17's capture check therefore cannot fire there, and a Spec Run
/// that declares an output is refused rather than silently admitted. Raised as
/// finding D17; the runtime may supply the set directly through
/// [`CompileInputs::sink_components`] meanwhile.
///
/// Rule: MA-25, SB-25a, SC-21.
pub fn sink_components(
    profile: &BindingProfile,
    registry: &ModuleRegistry,
) -> BTreeSet<Ident> {
    profile
        .placements
        .components
        .iter()
        .filter(|(_, p)| {
            p.module.as_ref().is_some_and(|m| {
                registry.modules().any(|d| d.id == *m && d.roles.contains(&Role::Sink))
            })
        })
        .map(|(name, _)| name.clone())
        .collect()
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
    /// The Executor instance behind each Island: which Module supplies it and what
    /// it declares. MA-38 names the **instance**; no document names the Module, so
    /// the runtime supplies it and `plan()` refuses rather than inventing one
    /// (MA-19, MA-38; raised as finding D18).
    pub executors: &'a BTreeMap<Ident, (ModuleId, ExecutorDescriptor)>,
    /// The registered Link Modules, which MA-39's reachability check reads (MA-28).
    pub links: &'a [LinkDescriptor],
    /// The registered DataContracts, for SC-3 (SB-15).
    pub contracts: &'a ContractRegistry,
    /// Component names whose consumer Module holds the Sink role (SC-21, MA-25).
    pub sink_components: &'a BTreeSet<Ident>,
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
    let mut taken: BTreeSet<ResourceId> = BTreeSet::new();

    // Semantic validation (SB-2, SB-15, SB-17, SB-18, SB-25a).
    spec.check_key_prefixes()?;
    if !inputs.is_session {
        // SB-25a: "A Spec Run leaves it unset. A Session profile sets it." RS-12
        // builds a Session's implicit Spec *from* those very placements, so running
        // the check there would refuse every Session.
        crate::session::check_placement_modules(spec, profile)?;
    }
    for kind in spec.policies.failure.keys() {
        inputs.kinds.require(kind).map_err(|_| SpecError::Structural {
            reason: format!("SB-18: event kind {kind} is not registered"),
        })?;
    }
    check_graph_links(spec, inputs)?;
    check_outputs_have_sinks(spec, profile, inputs)?;

    // Binding resolution, then matching against the bound instance (SB-22, SB-34, SB-37).
    for (name, req) in &spec.resources {
        if !profile.bindings.contains_key(name) {
            return Err(SpecError::UnboundResource { name: name.clone() });
        }
        let provider = inputs.providers.get(name).ok_or_else(|| SpecError::Structural {
            reason: format!("SB-22: no Provider instance for binding {name}"),
        })?;
        let instance = provider.instance();
        let node =
            pick_node(&instance.tree, req, &taken).ok_or_else(|| SpecError::NoSingleInstance {
                name: name.clone(),
                constraint: format!("kind {}", req.kind),
            })?;
        taken.insert(node.id.clone());
        out.matched.insert(name.clone(), node.id.clone());
        match_constraints(name, req, node, *provider, inputs, &mut out)?;

        // `needs` may resolve to a sub-resource of another bound instance (SB-36).
        for (need_name, need) in &req.needs {
            let resolved = inputs
                .providers
                .values()
                .flat_map(|p| p.instance().tree.walk())
                .find(|n| n.kind == need.kind && need_satisfied(need, n))
                .ok_or_else(|| SpecError::NoSingleInstance {
                    name: need_name.clone(),
                    constraint: format!("needs {}", need.kind),
                })?;
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
/// SB-34 says two Spec resources "**may** bind to different sub-resources of one
/// instance" and does not say a node may not be shared, so a node already bound is
/// **preferred against**, not refused: of the nodes whose `kind` matches, a free one
/// that satisfies every constraint directly, then any free one, then a bound one
/// that satisfies, then any. Two resources therefore take different channels when
/// there are two, and share the device root when that is all there is — and nothing
/// satisfiable is refused. Falling back to "any" is what lets a coercible key reach
/// the Provider's `coerce` and a genuine rejection be reported against a real
/// candidate.
///
/// ponytail: greedy, in the order above, with no backtracking. A bipartite matching
/// is the upgrade if a profile ever needs one. SB-34's silence on exclusivity is
/// raised as D27.
fn pick_node<'a>(
    tree: &'a Resource,
    req: &ResourceReq,
    taken: &BTreeSet<ResourceId>,
) -> Option<&'a Resource> {
    let candidates: Vec<&Resource> =
        tree.walk().into_iter().filter(|n| n.kind == req.kind).collect();
    let satisfies_all = |n: &Resource| {
        req.requires.iter().all(|(k, c)| {
            n.capabilities.get(k).is_some_and(|cap| satisfies(c, cap).unwrap_or(false))
        })
    };
    let free = |n: &Resource| !taken.contains(&n.id);
    let pick = |f: &dyn Fn(&Resource) -> bool| candidates.iter().copied().find(|n| f(n));
    pick(&|n| free(n) && satisfies_all(n))
        .or_else(|| pick(&free))
        .or_else(|| pick(&satisfies_all))
        .or_else(|| candidates.first().copied())
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
    let port = |r: &PortRef| -> Option<&crate::contract::Port> {
        let c = spec.graph.components.get(&Ident::parse(&r.component).ok()?)?;
        c.ports.iter().find(|p| p.name == r.port)
    };
    for link in &spec.graph.links {
        if link.capacity == 0 {
            return Err(SpecError::Structural {
                reason: "SB-15: a link's capacity is mandatory and at least 1".to_owned(),
            });
        }
        if let (Some(from), Some(to)) = (port(&link.from), port(&link.to)) {
            inputs
                .contracts
                .check_link(&from.contract, &to.contract)
                .map_err(|e| SpecError::Structural { reason: e.to_string() })?;
        }
        let consumer = Ident::parse(&link.to.component).ok();
        let is_sink = consumer.is_some_and(|c| inputs.sink_components.contains(&c));
        let decl = DataLinkDecl {
            id: DataLinkId::local(0),
            from: link.from.clone(),
            to: link.to.clone(),
            contract: port(&link.to).map(|p| p.contract.clone()).unwrap_or_else(|| {
                crate::contract::DataContractId::parse("ezsdr.control").expect("a valid literal")
            }),
            policy: link.policy,
            capacity: link.capacity,
        };
        crate::stream::check_sink_link(&decl, is_sink)
            .map_err(|e| SpecError::Structural { reason: e.to_string() })?;
    }
    Ok(())
}

/// A capture whose source has no placed Sink is refused at `validate()` (SB-17).
fn check_outputs_have_sinks(
    spec: &ExperimentSpec,
    profile: &BindingProfile,
    inputs: &CompileInputs<'_>,
) -> Result<(), SpecError> {
    for output in &spec.outputs {
        // The Sink that serves this output is the one the samples reach: the
        // consumer of a link out of the named port or resource, or the named
        // component itself when it is the Sink.
        let source = match &output.source {
            crate::spec::OutputSource::Port { port } => port.component.clone(),
            crate::spec::OutputSource::Resource { resource } => resource.to_string(),
        };
        let reaches_a_sink = Ident::parse(&source).is_ok_and(|c| inputs.sink_components.contains(&c))
            || spec.graph.links.iter().any(|l| {
                l.from.component == source
                    && Ident::parse(&l.to.component)
                        .is_ok_and(|c| inputs.sink_components.contains(&c))
            });
        if !reaches_a_sink {
            return Err(SpecError::Structural {
                reason: format!(
                    "SB-17: output {} names source {source:?}, which reaches no placed Sink",
                    output.id
                ),
            });
        }
    }
    let _ = profile;
    Ok(())
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
/// Rule: SB-39, SB-40, MA-41.
pub fn plan(
    spec: &ExperimentSpec,
    profile: &BindingProfile,
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
        let after = arm_after_of(name, profile, inputs);
        for before in &after {
            edges.push((before.clone(), name.clone()));
        }
        fragments.push(Fragment {
            id: name.clone(),
            instance: binding.provider.clone(),
            role: Role::Provider,
            content: serde_json::to_value(&binding.selector).unwrap_or(serde_json::Value::Null),
            after,
        });
    }
    for island in &profile.placements.islands {
        let (module, _) = inputs.executors.get(&island.executor).ok_or_else(|| {
            SpecError::Structural {
                reason: format!(
                    "MA-38: island {} names executor instance {}, which no registered Module supplies",
                    island.id, island.executor
                ),
            }
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
        inputs.executors.iter().map(|(k, (_, d))| (k.clone(), d.clone())).collect();
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
    })
    .map_err(|e| SpecError::Structural { reason: e.message })?;
    check_sink_links(&declared_links(spec, inputs), &|r: &PortRef| {
        Ident::parse(&r.component).is_ok_and(|c| inputs.sink_components.contains(&c))
    })
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
