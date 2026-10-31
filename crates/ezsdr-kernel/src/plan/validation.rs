//! Validates Spec structure, bindings, and admission results before planning or preparation
//! (SB-37, SB-T4).

use std::collections::{BTreeMap, BTreeSet};

use crate::binding::{AdmissionResult, BindingProfile, CheckStage, Violation, check_constraint_kind, check_value_kind};
use crate::event::Target;
use crate::id::ResourceId;
use crate::module_api::{ModuleRef, ModuleRegistry, Role};
use crate::spec::{Constraint, ExperimentSpec, Ident, Key, KeyedEntry, Namespace, SpecError, Value};

use super::CompileInputs;

/// SB-30's first point: every registered check whose section is present, against the
/// requested configuration. Checks are pure (SB-31), so `plan()` runs this again
/// rather than trusting a result's `violations` (SB-39, D99).
///
/// Rule: SB-30.
pub(super) fn requested_violations(spec: &ExperimentSpec, profile: &BindingProfile, inputs: &CompileInputs<'_>) -> Vec<Violation> {
    // SB-30 (KA-4): one entry per fragment, never merged, so that two fragments naming
    // one key are each judged by their own value. A need's `Eq` values do not join its
    // resource's entry (item 3): a need is a resolution to a node, not a value this
    // fragment requests, and joining it let a need's value stand in a check's input
    // for a fragment that never asked for it.
    let mut requested: BTreeMap<Ident, BTreeMap<Key, Value>> = BTreeMap::new();
    let eq_values = |requires: &BTreeMap<Key, Constraint>| -> Vec<(Key, Value)> {
        requires
            .iter()
            .filter_map(|(k, c)| match c {
                Constraint::Eq { value } => Some((k.clone(), Value::Scalar(value.clone()))),
                _ => None,
            })
            .collect()
    };
    for (name, r) in &spec.resources {
        requested.insert(name.clone(), eq_values(&r.requires).into_iter().collect());
    }
    for o in &spec.outputs {
        requested.insert(o.id.clone(), o.params.clone());
    }
    inputs.checks.run(&profile.environment, &requested, &BTreeMap::new(), CheckStage::Validate)
}

pub(super) fn validate(spec: &ExperimentSpec, profile: &BindingProfile, inputs: &CompileInputs<'_>) -> Result<AdmissionResult, SpecError> {
    check_structure(spec, profile, inputs)?;
    let mut out = AdmissionResult::default();
    // SB-34: node -> the resource name that bound it, so a collision can name both.
    let mut taken: BTreeMap<ResourceId, Ident> = BTreeMap::new();

    // Matching against the bound instance (SB-34, SB-37). `check_structure` has
    // established that every resource slot is bound and has its Provider (SB-22f).
    for (name, req) in &spec.resources {
        let provider = super::supplied(inputs.providers, name, "Provider instance")?;
        let instance = provider.instance();
        let node = super::matching::pick_node(&instance.tree, name, req, &taken)?;
        taken.insert(node.id.clone(), name.clone());
        out.matched.insert(name.clone(), node.id.clone());
        super::matching::match_constraints(name, req, node, *provider, inputs, &mut out)?;

        // `needs` may resolve to a sub-resource of another bound instance (SB-36).
        for (need_name, need) in &req.needs {
            // SB-34 applies here too: a need consumes the node it resolves to
            // unless the Provider declared it shareable, so two needs of two
            // resources take two lines when there are two.
            // Only the instances bound to this Spec's resources: an instance handed
            // in under a name no resource binds is never read (SB-22, SB-36).
            let mut matching = inputs
                .providers
                .iter()
                .filter(|(bound, _)| spec.resources.contains_key(*bound))
                .flat_map(|(_, p)| p.instance().tree.walk_iter())
                .filter(|n| n.kind == need.kind && super::matching::need_satisfied(need, n));
            // A free node first, else the first shareable one already bound (SB-34).
            let (mut first_taken, mut first_shared, mut free) = (None, None, None);
            for n in matching.by_ref() {
                if !taken.contains_key(&n.id) {
                    free = Some(n);
                    break;
                }
                if n.shareable {
                    first_shared.get_or_insert(n);
                }
                first_taken.get_or_insert(n);
            }
            let resolved = free.or(first_shared);
            let resolved = match (resolved, first_taken) {
                (Some(n), _) => n,
                // Every satisfying node is taken and exclusive: refused as a
                // resource is (SB-34), naming the binder.
                (None, Some(n)) => return Err(SpecError::NodeAlreadyBound { node: n.id.to_string(), first: taken[&n.id].clone(), second: need_name.clone() }),
                (None, None) => return Err(SpecError::NoSingleInstance { name: need_name.clone(), constraint: format!("needs {}", need.kind) }),
            };
            taken.insert(resolved.id.clone(), need_name.clone());
            // SB-36: a need's name is scoped to its resource, so the resolution is
            // filed under both.
            out.needs.entry(name.clone()).or_default().insert(need_name.clone(), resolved.id.clone());
        }
    }

    // The registered admission checks, against the requested configuration (SB-30).
    out.violations.extend(requested_violations(spec, profile, inputs));

    // SB-15 and SB-17 run **after** matching, because a resource endpoint's port is
    // one the *bound node* declares (MA-10) and `matched` does not exist before it.
    super::links::check_endpoints(spec, inputs, &out.matched)?;

    // SB-45, SB-46: a previewed coercion under `reject` fails the stage here rather
    // than at `prepare`. A dry run that reported a coercion it knows will be
    // rejected would not be a dry run (Vision §52).
    for previewed in out.coercions_preview.clone() {
        let c = &previewed.coercion;
        let policy = super::coercion::coercion_policy(spec.policies.coercion.get(&c.key).copied(), inputs.is_session, inputs.registry.key_decl(&c.key));
        match super::coercion::apply_coercion(policy, c) {
            Ok(None) => {}
            Ok(Some(w)) => out.warnings.push(w),
            Err(_) => out.violations.push(Violation {
                check: Namespace::parse("ezsdr.coercion").expect("a valid literal"),
                key: Some(c.key.clone()),
                requested: Some(c.requested.clone()),
                reason: format!("SB-46: coercion to {:?} rejected", c.applied),
            }),
        }
    }
    Ok(out)
}

/// The binding model over the slots the two documents name (SB-22…SB-22g, SB-24,
/// tables SB-T1…SB-T3). The runtime's maps are read under slot names only, through
/// [`supplied`](super::supplied); an entry under any other name is never looked at (SB-22).
///
/// Rule: SB-22, SB-22a…SB-22g, SB-24, SB-3.
pub(super) fn check_bindings(spec: &ExperimentSpec, profile: &BindingProfile, inputs: &CompileInputs<'_>) -> Result<(), SpecError> {
    let registry = inputs.registry;
    let authority = &profile.authority;
    let rides = spec.resources.contains_key(authority);
    let executors: BTreeSet<&Ident> = profile.placements.islands.iter().map(|i| &i.executor).collect();

    // SB-22a: one namespace over the six kinds of name of SB-T1. `PortRef.component`
    // resolves a component before a resource and a feed's consumer end is `{output
    // id, "in"}`, so any collision among those is an ambiguity; an Island's fragment
    // is `island_<local>` and a dedicated Authority's is its name, so those are taken
    // too (D85; the Authority's since D96). One executor may serve several Islands, and
    // `authority` may be a resource, which the Authority then rides on (SB-24).
    let island_ids: Vec<Ident> = profile.placements.islands.iter().filter_map(|i| Ident::parse(&format!("island_{}", i.id.local)).ok()).collect();
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
        if let Some(first) = names.insert(name, set) {
            if first == "an Island executor" && set == first {
                continue;
            }
            return Err(SpecError::DuplicateBindingName { name: name.clone(), sets: if first == set { format!("{set} declared twice") } else { format!("{first} and {set}") } });
        }
    }
    // One slot at a time: bound (SB-22d), to a Module holding its role (SB-22e), with
    // the object the runtime supplies naming the binding's version (SB-22f) — role
    // before instance, as SB-22c orders it.
    let version = |name: &Ident, what: &str, bound: &ModuleRef, reported: &ModuleRef| {
        if bound == reported {
            return Ok(());
        }
        Err(SpecError::Structural { reason: format!("SB-22f: binding {name} names Module {} {}, but its {what} is {} {}", bound.id, bound.version, reported.id, reported.version) })
    };
    for name in spec.resources.keys() {
        let binding = profile.bindings.get(name).ok_or_else(|| SpecError::UnboundResource { name: name.clone() })?;
        require_role(registry, name, &binding.module, Role::Provider)?;
        let instance = super::supplied(inputs.providers, name, "Provider instance")?.instance();
        version(name, "Provider instance", &binding.module, &instance.module)?;
        // SB-22f (KA-7): the one envelope value the Kernel reads is in host.monotonic
        // and not negative.
        if let Some(lead) = instance.min_command_lead {
            if !lead.ticks_in(crate::id::ClockDomainId::HOST_MONOTONIC).is_ok_and(|t| t >= 0) {
                return Err(SpecError::Structural {
                    reason: format!(
                        "SB-22f: the instance bound to {name} declares a min_command_lead of \
                         {lead}, which is not a non-negative host.monotonic duration"
                    ),
                });
            }
        }
    }
    for output in &spec.outputs {
        let name = &output.id;
        let binding = profile.bindings.get(name).ok_or_else(|| SpecError::Structural { reason: format!("SB-22d: output {name} has no binding (UnboundOutput)") })?;
        require_role(registry, name, &binding.module, Role::Sink)?;
        let d = super::supplied(inputs.sinks, name, "Sink instance")?.descriptor();
        version(name, "Sink instance", &binding.module, &d.module)?;
        // MA-25 / D86: a Sink that reads from no memory domain cannot satisfy the
        // shared feed reachability rule (MA-39).
        if d.memory_domains.is_empty() {
            return Err(SpecError::Structural { reason: format!("MA-25: output {name}'s Sink declares no memory domain it reads from") });
        }
    }
    for name in &executors {
        let binding =
            profile.bindings.get(*name).ok_or_else(|| SpecError::Structural { reason: format!("SB-22d: an Island names executor {name}, which the profile does not bind (UnboundExecutor)") })?;
        require_role(registry, name, &binding.module, Role::Executor)?;
        let d = super::supplied(inputs.executors, name, "ExecutorDescriptor")?;
        version(name, "Executor instance", &binding.module, &d.module)?;
        // MA-18 / D86: admission reads `memory_domains`; an Executor that reaches
        // none can host no component.
        if d.memory_domains.is_empty() {
            return Err(SpecError::Structural { reason: format!("MA-18: executor {name} declares no memory domain it can reach") });
        }
    }
    // SB-24: `authority` names a binding whose Module holds Authority — a resource the
    // Authority rides on as much as a slot of its own — and whose supplied descriptor
    // names that binding's version (SB-22e, D98).
    let binding = profile.bindings.get(authority).ok_or_else(|| SpecError::Structural { reason: format!("SB-24: `authority` names {authority}, which is not a binding") })?;
    require_role(registry, authority, &binding.module, Role::Authority)?;
    let a = super::supplied(inputs.authorities, authority, "AuthorityDescriptor")?;
    version(authority, "Authority", &binding.module, &a.module)?;

    // SB-22g: on a Spec Run no binding carries `feed`, since `outputs[]` declares each
    // feed; on a Session a binding carries one exactly when it fills a Sink slot, and
    // it is that output's feed. Anything else is a second, unread source of truth —
    // including a policy SC-21 restricts and nothing would have looked at.
    for (name, b) in &profile.bindings {
        let output = spec.outputs.iter().find(|o| &o.id == name);
        let refused = match (&b.feed, output) {
            (Some(_), _) if !inputs.is_session => Some("carries `feed`, which only a Session profile may; a Spec Run declares it in `outputs[]`"),
            (Some(feed), Some(o)) if o.feed != *feed => Some("carries a `feed` that is not its output's"),
            (Some(_), None) => Some("carries `feed` and fills no Sink slot"),
            (None, Some(_)) if inputs.is_session => Some("fills a Session's Sink slot and carries no `feed`"),
            _ => None,
        };
        if let Some(why) = refused {
            return Err(SpecError::Structural { reason: format!("SB-22g: binding {name} {why}") });
        }
    }

    // SB-3: a `ResourceId` carries no instance qualification, so two bound instances
    // declaring one node path are one address — for the matcher's `taken` set, for
    // `arm_after` resolution and for `Event.source` attribution alike. Identity is the
    // **binding description** `(module, selector, profile)`: two bindings with equal
    // descriptions are one instance (SB-23 reads `instances: 2` that way) and must
    // report one `instance().id`, or the runtime handed the Kernel two objects for one
    // description and the Manifest could not reproduce it.
    let mut by_description: BTreeMap<(ModuleRef, String), (&Ident, &ResourceId)> = BTreeMap::new();
    let mut paths: BTreeMap<ResourceId, &Ident> = BTreeMap::new();
    for (name, binding) in profile.bindings.iter().filter(|(n, _)| spec.resources.contains_key(*n)) {
        let instance = super::supplied(inputs.providers, name, "Provider instance")?.instance();
        let description = binding_description(binding)?;
        let duplicate_description = if let Some((first, first_id)) = by_description.insert(description, (name, &instance.id)) {
            if *first_id != instance.id {
                return Err(SpecError::Structural {
                    reason: format!(
                        "SB-3: {first} and {name} carry one binding description and so name one \
                         instance, but their Providers report {first_id} and {}",
                        instance.id
                    ),
                });
            }
            true
        } else {
            false
        };
        for node in instance.tree.walk_iter() {
            if !node.ports.is_empty() && node.memory_domains.is_empty() {
                return Err(SpecError::Structural { reason: format!("MA-10: Provider node {} has ports but declares no memory domain", node.id) });
            }
            if !duplicate_description {
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
    }

    // SB-22d / D89: a binding that fills no slot is read by no check — its Module
    // reference could be an unregistered version and still reach the Manifest, which
    // records the profile (RS-38). Last, so that a misspelt output binding is
    // `UnboundOutput` and a stray `feed` is SB-22g's.
    if let Some(name) = profile.bindings.keys().find(|n| !spec.resources.contains_key(*n) && !spec.outputs.iter().any(|o| &o.id == *n) && !executors.contains(n) && *n != authority) {
        return Err(SpecError::Structural { reason: format!("SB-22d: binding {name} plays no role in this Run") });
    }
    Ok(())
}

/// `validate()`'s structural checks, which `plan()` runs again (SB-39, D99): the
/// Spec's keys and Vocabulary majors (SB-2, SB-11), its failure policy (SB-18), the
/// binding model (SB-22…SB-22g,
/// SB-24), the schedule (SB-16, RS-52) and the component descriptors (MA-37). None of
/// them calls a Provider's `coerce`, so running them twice changes nothing.
///
/// Rule: SB-T4.
pub(super) fn check_structure(spec: &ExperimentSpec, profile: &BindingProfile, inputs: &CompileInputs<'_>) -> Result<(), SpecError> {
    spec.check_key_prefixes()?;
    check_keys(spec, inputs)?;
    // SB-11: `requirements.vocabularies` lists the Vocabulary **majors** this Spec's
    // keys belong to. Nothing read `major`, so a Spec declaring `test 2` ran against
    // the registered `test 1.0.0` and SB-11's list was a field with no reader — the
    // same shape D40 withdrew `constraints_hit` for.
    for req in &spec.requirements.vocabularies {
        let v = inputs.registry.vocabulary(&req.id).ok_or_else(|| SpecError::Structural { reason: format!("SB-11: Vocabulary {} is not registered", req.id) })?;
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
        inputs.kinds.require(kind).map_err(|_| SpecError::Structural { reason: format!("SB-18: event kind {kind} is not registered") })?;
    }
    check_bindings(spec, profile, inputs)?;
    // MA-37: a descriptor is validated before anything reads its ports or its
    // `ParamDecl`s, or a duplicate port name silently resolves a link to whichever
    // came first and a duplicate parameter key reaches `declared_class` as RS-52
    // rather than MA-37. The schedule loop below reads `ParamDecl`s, so this runs
    // before it (review P2-5).
    let contract_ids = inputs.contracts.ids();
    for c in spec.graph.components.values() {
        c.validate(&contract_ids).map_err(|e| SpecError::Structural { reason: e.message })?;
    }
    // SB-16: a `SpecTime` is "a resource name plus an offset in **that resource's**
    // stream clock", so the name must be one the Spec declares. Nothing read
    // `spec.schedule` at all, so a clock name bound to nothing passed every stage and
    // the entry left no trace in the plan.
    for entry in &spec.schedule {
        if !spec.resources.contains_key(&entry.at.clock) {
            return Err(SpecError::Structural { reason: format!("SB-16: schedule entry's clock {} is not a resource this Spec declares", entry.at.clock) });
        }
        // SB-16, KC-23 (item 3): the target is classified first, so a target the Spec
        // does not declare in its role is refused before anything reads a class.
        // `validate` is the one classifier; the runtime routing is built from this
        // validated plan (review P2-4).
        if let Some(target) = entry.action.target() {
            spec.classify_target(target).map_err(|reason| SpecError::Structural { reason: format!("SB-16: schedule entry's target {reason}") })?;
        }
        // RS-52 (item 3): a scheduled `UpdateParameter` states no class — the
        // ActionTemplate has no `class` field — so there is no equality for
        // `validate` to check. A fixed or missing declaration is still a gap: the
        // entry names a parameter whose class no declaration gives, and it is
        // refused here (plan §5's `Failed { validate }`). The class itself is read
        // at KC-24's step 5, where `arm` admits the entry; a declaration cannot
        // change between `validate` and `arm`, so a Spec that validates cannot hit
        // the same gap there.
        if let crate::event::ActionTemplate::UpdateParameter { target, key, .. } = &entry.action {
            if spec.declared_class(target, key, inputs.registry).is_none() {
                return Err(SpecError::Structural { reason: format!("RS-52: schedule entry updates {key}, which declares no update class") });
            }
        }
    }
    // MA-37's descriptor validation already ran, above the schedule loop (review
    // P2-5), so nothing here reads a `ParamDecl` a malformed descriptor owns.
    Ok(())
}

/// SB-2 and SB-6 for every key position of a Spec. One scan
/// ([`ExperimentSpec::keyed_entries`]) covers the resource and need `requires`, the
/// output `params`, the coercion policies and the schedule's `UpdateParameter`s, so
/// the prefix check and this declaration check cannot look at different positions
/// (pre-freeze item 3). For each position: an `ext.` key's Module id names a
/// registered Module, any other key has its `KeyDecl`, and the constraint or value is
/// of that declaration's kind. A component's own parameter skips both — its schema is
/// opaque and its owner is the component (MA-34). None of it needs an instance, so it
/// is a structural check, which `plan()` runs again (SB-T4, D99).
///
/// Rule: SB-2, SB-6, SB-17, SB-16, MA-34.
fn check_keys(spec: &ExperimentSpec, inputs: &CompileInputs<'_>) -> Result<(), SpecError> {
    for entry in spec.keyed_entries() {
        let key = *match &entry {
            KeyedEntry::Constraint { key, .. } | KeyedEntry::Output { key, .. } | KeyedEntry::Coercion { key } => key,
            KeyedEntry::ScheduledUpdate { target, key, .. } => {
                if matches!(target, Target::Component { .. }) {
                    // MA-34 (item 3): a component's parameter key is owned by the
                    // component that declares it; the component's `schema` is opaque,
                    // so no Vocabulary prefix, `KeyDecl` or kind check applies.
                    continue;
                }
                key
            }
        };
        if key.is_extension() {
            // MA-34 writes `ext.<module-id>.<path>`, so the owner is a **registered**
            // Module. Accepting any `ext.…` would make the escape hatch unowned: no
            // declaration, no shape check and nobody accountable for the meaning.
            // On a segment boundary: `starts_with` alone made
            // `ext.ezsdr.test.providerx.thing` "owned" by `ezsdr.test.provider`, and a
            // single-segment id such as `ezsdr` would own every `ext.ezsdr.*` key.
            let owner = key.as_str().strip_prefix("ext.").expect("is_extension means the key starts with one `ext.`");
            let owned = |id: &str| owner.strip_prefix(id).is_some_and(|rest| rest.starts_with('.'));
            if !inputs.registry.modules().any(|m| owned(m.id.as_str())) {
                return Err(SpecError::UnknownKeyPrefix { key: format!("{key}: MA-34: no registered Module owns this `ext.` prefix") });
            }
            continue;
        }
        let Some(d) = inputs.registry.key_decl(key) else {
            return Err(SpecError::UnknownKeyPrefix { key: format!("{key}: MA-34: no registered Vocabulary declares this key") });
        };
        match &entry {
            KeyedEntry::Constraint { constraint, .. } => {
                check_constraint_kind(d, key, constraint)?;
            }
            KeyedEntry::Output { value, .. } | KeyedEntry::ScheduledUpdate { value, .. } => {
                check_value_kind(d, key, value)?;
            }
            KeyedEntry::Coercion { .. } => {}
        }
    }
    Ok(())
}

/// A binding's description `(module, selector, profile)`, which is an instance's
/// identity (SB-3): two bindings with equal descriptions name one instance. Shared by
/// `validate` and the coordinator's grouping (KC-4), so the two cannot disagree.
pub(crate) fn binding_description(binding: &crate::binding::Binding) -> Result<(ModuleRef, String), SpecError> {
    let json = |e: serde_json::Error| SpecError::Structural { reason: format!("SB-3: a binding description: {e}") };
    Ok((binding.module.clone(), format!("{}|{}", serde_json::to_string(&binding.selector).map_err(json)?, serde_json::to_string(&binding.profile).map_err(json)?)))
}

/// SB-22e: the registered Module a binding names holds `role`. A version that is not
/// registered is refused as such, and a registered one without the role is
/// `WrongBindingRole`. One function, because four copies of the question had grown
/// four answers — and a Session and a Spec Run had reported one condition as two
/// different errors.
///
/// Rule: SB-22e, MA-1.
pub(crate) fn require_role(registry: &ModuleRegistry, name: &Ident, module: &ModuleRef, role: Role) -> Result<(), SpecError> {
    let Some(m) = registry.modules().find(|m| crate::module_api::is_module(m, module)) else {
        return Err(not_registered(name, module));
    };
    if m.roles.contains(&role) {
        return Ok(());
    }
    Err(SpecError::WrongBindingRole { name: name.clone(), expected: format!("{role:?}"), module: format!("{} {}", module.id, module.version) })
}

/// SB-22e's refusal of a Module version nobody registered, shared with the Session
/// derivation so that one condition has one error on both Run kinds.
pub(crate) fn not_registered(name: &Ident, module: &ModuleRef) -> SpecError {
    SpecError::Structural { reason: format!("SB-22e: binding {name} names Module {} {}, which is not registered", module.id, module.version) }
}

/// SB-39's guard: an `AdmissionResult` is this Run's only if `matched` holds exactly
/// this Spec's resources and `needs` exactly their needs, each resource's node is one
/// its bound instance declares, and each need's node is one an instance bound to a
/// resource of this Spec declares (SB-36). "Admitted" alone is satisfied by an empty
/// result, and a stale one names nodes of an instance this profile no longer binds
/// (D99). The Manifest records the whole result (SB-38), so an entry `plan()` does not
/// read is still checked (D107).
///
/// Rule: SB-39, SB-36, SB-30.
pub(super) fn admission_is_this_runs(spec: &ExperimentSpec, inputs: &CompileInputs<'_>, admission: &AdmissionResult) -> Result<(), String> {
    let declares = |resource: &Ident, node: &ResourceId| inputs.providers.get(resource).is_some_and(|p| p.instance().tree.walk_iter().any(|n| n.id == *node));
    let not_this_runs = |why: String| format!("this `AdmissionResult` is not this Run's: {why}");
    for (name, req) in &spec.resources {
        let Some(node) = admission.matched.get(name) else {
            return Err(not_this_runs(format!("{name} has no matched node")));
        };
        if !declares(name, node) {
            return Err(not_this_runs(format!("{name}'s matched node {node} is not one its bound instance declares")));
        }
        let resolved = admission.needs.get(name);
        for need in req.needs.keys() {
            let Some(node) = resolved.and_then(|needs| needs.get(need)) else {
                return Err(not_this_runs(format!("{name}'s need {need} has no matched node")));
            };
            if !spec.resources.keys().any(|r| declares(r, node)) {
                return Err(not_this_runs(format!(
                    "{name}'s need {need} names node {node}, which no instance bound to this \
                     Spec's resources declares"
                )));
            }
        }
        if let Some(extra) = resolved.into_iter().flat_map(|needs| needs.keys()).find(|need| !req.needs.contains_key(*need)) {
            return Err(not_this_runs(format!("{extra} is not a need of {name}")));
        }
    }
    if let Some(extra) = admission.matched.keys().chain(admission.needs.keys()).find(|k| !spec.resources.contains_key(*k)) {
        return Err(not_this_runs(format!("{extra} is not a resource of this Spec")));
    }
    Ok(())
}
