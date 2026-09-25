//! Validates Spec structure, bindings, and admission results before planning or preparation
//! (SB-37, SB-T4).

use std::collections::{BTreeMap, BTreeSet};

use crate::binding::{
    AdmissionResult, BindingProfile, CheckStage, Violation, check_constraint_kind,
};
use crate::id::ResourceId;
use crate::module_api::{ModuleRef, ModuleRegistry, Role};
use crate::spec::{Constraint, ExperimentSpec, Ident, Key, Namespace, SpecError, Value};

use super::CompileInputs;

/// SB-30's first point: every registered check whose section is present, against the
/// requested configuration. Checks are pure (SB-31), so `plan()` runs this again
/// rather than trusting a result's `violations` (SB-39, D99).
///
/// Rule: SB-30.
pub(super) fn requested_violations(
    spec: &ExperimentSpec,
    profile: &BindingProfile,
    inputs: &CompileInputs<'_>,
) -> Vec<Violation> {
    // SB-30 (KA-4): one entry per fragment, never merged, so that two fragments naming
    // one key are each judged by their own value.
    let mut requested: BTreeMap<Ident, BTreeMap<Key, Value>> = BTreeMap::new();
    let eq_values = |requires: &BTreeMap<Key, Constraint>| -> Vec<(Key, Value)> {
        requires
            .iter()
            .filter_map(|(k, c)| match c {
                Constraint::Eq { value } => Some((k.clone(), value.clone())),
                _ => None,
            })
            .collect()
    };
    for (name, r) in &spec.resources {
        // The resource's needs are bound within its fragment (SB-36), so their values
        // are the fragment's too; the resource's own value wins for a key both name.
        let mut values: BTreeMap<Key, Value> = BTreeMap::new();
        for need in r.needs.values() {
            values.extend(eq_values(&need.requires));
        }
        values.extend(eq_values(&r.requires));
        requested.insert(name.clone(), values);
    }
    for o in &spec.outputs {
        requested.insert(o.id.clone(), o.params.clone());
    }
    inputs.checks.run(
        &profile.environment,
        &requested,
        &BTreeMap::new(),
        CheckStage::Validate,
    )
}

pub(super) fn validate(
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
            let resolved = inputs
                .providers
                .iter()
                .filter(|(bound, _)| spec.resources.contains_key(*bound))
                .flat_map(|(_, p)| p.instance().tree.walk_iter())
                .find(|n| {
                    n.kind == need.kind
                        && super::matching::need_satisfied(need, n)
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
            out.matched.insert(
                super::matching::need_key(name, need_name),
                resolved.id.clone(),
            );
        }
    }

    // The registered admission checks, against the requested configuration (SB-30).
    out.violations
        .extend(requested_violations(spec, profile, inputs));

    // SB-15 and SB-17 run **after** matching, because a resource endpoint's port is
    // one the *bound node* declares (MA-10) and `matched` does not exist before it.
    super::links::check_endpoints(spec, inputs, &out.matched)?;

    // SB-45, SB-46: a previewed coercion under `reject` fails the stage here rather
    // than at `prepare`. A dry run that reported a coercion it knows will be
    // rejected would not be a dry run (Vision §52).
    for previewed in out.coercions_preview.clone() {
        let c = &previewed.coercion;
        let policy = super::coercion::coercion_policy(
            spec.policies.coercion.get(&c.key).copied(),
            inputs.is_session,
            inputs.registry.key_decl(&c.key).ok(),
        );
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

/// The binding model over the slots the two documents name (SB-22…SB-22h, SB-24,
/// tables SB-T1…SB-T3). The runtime's maps are read under slot names only, through
/// [`supplied`](super::supplied); an entry under any other name is never looked at (SB-22).
///
/// Rule: SB-22, SB-22a…SB-22h, SB-24, SB-3, SB-36.
pub(super) fn check_bindings(
    spec: &ExperimentSpec,
    profile: &BindingProfile,
    inputs: &CompileInputs<'_>,
) -> Result<(), SpecError> {
    let registry = inputs.registry;
    let authority = &profile.authority;
    let rides = spec.resources.contains_key(authority);
    let executors: BTreeSet<&Ident> = profile
        .placements
        .islands
        .iter()
        .map(|i| &i.executor)
        .collect();

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
        .chain(
            spec.graph
                .components
                .keys()
                .map(|n| (n, "a graph component")),
        )
        .chain(spec.outputs.iter().map(|o| (&o.id, "an output id")))
        .chain(
            profile
                .placements
                .islands
                .iter()
                .map(|i| (&i.executor, "an Island executor")),
        )
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
                sets: if first == set {
                    format!("{set} declared twice")
                } else {
                    format!("{first} and {set}")
                },
            });
        }
    }
    // SB-36 records a need under `<resource>_<need>`, and `matched` is one map, so a
    // key equal to another resource's name silently overwrote the need's record —
    // always the need's, because `X < X_need` orders the resource's insert second.
    for (name, req) in &spec.resources {
        for need in req.needs.keys() {
            let k = super::matching::need_key(name, need);
            if spec.resources.contains_key(&k) {
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
        let instance = super::supplied(inputs.providers, name, "Provider instance")?.instance();
        version(name, "Provider instance", &binding.module, &instance.module)?;
        check_rid(
            &format!("the instance bound to {name} has id"),
            &instance.id,
        )?;
        for n in instance.tree.walk_iter() {
            check_rid(
                &format!("the instance bound to {name} declares node"),
                &n.id,
            )?;
        }
        for a in &instance.arm_after {
            check_rid(&format!("the instance bound to {name} arms after"), a)?;
        }
        // SB-22f (KA-7): the one envelope value the Kernel reads is in host.monotonic
        // and not negative.
        if let Some(lead) = instance.min_command_lead {
            if lead.domain != crate::id::ClockDomainId::HOST_MONOTONIC || lead.ticks < 0 {
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
        let binding = profile
            .bindings
            .get(name)
            .ok_or_else(|| SpecError::Structural {
                reason: format!("SB-22d: output {name} has no binding (UnboundOutput)"),
            })?;
        require_role(registry, name, &binding.module, Role::Sink)?;
        let d = super::supplied(inputs.sinks, name, "Sink instance")?.descriptor();
        version(name, "Sink instance", &binding.module, &d.module)?;
        // MA-25 / D86: a Sink that reads from no memory domain cannot be fed, yet a
        // resource feed skips the domain check (D31) and would admit it.
        if d.memory_domains.is_empty() {
            return Err(SpecError::Structural {
                reason: format!(
                    "MA-25: output {name}'s Sink declares no memory domain it reads from"
                ),
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
        let d = super::supplied(inputs.executors, name, "ExecutorDescriptor")?;
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
    let binding = profile
        .bindings
        .get(authority)
        .ok_or_else(|| SpecError::Structural {
            reason: format!("SB-24: `authority` names {authority}, which is not a binding"),
        })?;
    require_role(registry, authority, &binding.module, Role::Authority)?;
    let a = super::supplied(inputs.authorities, authority, "AuthorityDescriptor")?;
    version(authority, "Authority", &binding.module, &a.module)?;
    if let Some(d) = a.governs.iter().find(|d| !d.node.is_local()) {
        return Err(not_local(format!(
            "Authority {authority}'s governed domain {d}"
        )));
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
            (Some(feed), Some(o)) if o.feed != *feed => {
                Some("carries a `feed` that is not its output's")
            }
            (Some(_), None) => Some("carries `feed` and fills no Sink slot"),
            (None, Some(_)) if inputs.is_session => {
                Some("fills a Session's Sink slot and carries no `feed`")
            }
            _ => None,
        };
        if let Some(why) = refused {
            return Err(SpecError::Structural {
                reason: format!("SB-22g: binding {name} {why}"),
            });
        }
    }

    // SB-22h reserves the first path segment `sink` for a bound Sink's address, so a
    // Provider may not declare a node under it: without this the `sink/` prefix only
    // narrowed the collision from "any output id" to "a Provider that names a node
    // `sink`" — `ResourceId::parse("sink/rec")` is a legal Provider node path.
    for name in spec.resources.keys() {
        let instance = super::supplied(inputs.providers, name, "Provider instance")?.instance();
        if let Some(clash) = instance.tree.walk_iter().find(|n| {
            // SB-22h (KA-14): also `kernel`, the Kernel's own event source, and
            // `unforeseen`, RS-33's fallback row, and every `island_<n>`, an
            // Executor's event source (KC-8).
            n.id.segments().next().is_some_and(|s| {
                matches!(s, "sink" | "kernel" | "unforeseen") || s.starts_with("island_")
            })
        }) {
            return Err(SpecError::Structural {
                reason: format!(
                    "SB-22h: instance bound to {name} declares node {}, and the first path \
                     segments `sink`, `kernel`, `unforeseen` and `island_<n>` are reserved",
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
    let mut by_description: BTreeMap<(ModuleRef, String), (&Ident, &ResourceId)> = BTreeMap::new();
    let mut paths: BTreeMap<ResourceId, &Ident> = BTreeMap::new();
    for (name, binding) in profile
        .bindings
        .iter()
        .filter(|(n, _)| spec.resources.contains_key(*n))
    {
        let instance = super::supplied(inputs.providers, name, "Provider instance")?.instance();
        let description = binding_description(binding);
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
        for node in instance.tree.walk_iter() {
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

/// `validate()`'s structural checks, which `plan()` runs again (SB-39, D99): the
/// Spec's keys and Vocabulary majors (SB-2, SB-11), its failure policy (SB-18), the
/// ids a document carries as Rust values (X7, SB-1), the binding model (SB-22…SB-22h,
/// SB-24), the schedule (SB-16, RS-52) and the component descriptors (MA-37). None of
/// them calls a Provider's `coerce`, so running them twice changes nothing.
///
/// Rule: SB-T4.
pub(super) fn check_structure(
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
        let v = inputs
            .registry
            .vocabulary(&req.id)
            .ok_or_else(|| SpecError::Structural {
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
        inputs
            .kinds
            .require(kind)
            .map_err(|_| SpecError::Structural {
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
            let declared = inputs
                .registry
                .key_decl(key)
                .ok()
                .and_then(|d| d.update_class);
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
        c.validate(&contract_ids)
            .map_err(|e| SpecError::Structural { reason: e.message })?;
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
                let owned = |id: &str| {
                    owner
                        .strip_prefix(id)
                        .is_some_and(|rest| rest.starts_with('.'))
                };
                if !inputs.registry.modules().any(|m| owned(m.id.as_str())) {
                    return Err(SpecError::UnknownKeyPrefix {
                        key: format!("{key}: MA-34: no registered Module owns this `ext.` prefix"),
                    });
                }
            } else {
                let d = inputs
                    .registry
                    .key_decl(key)
                    .map_err(|e| SpecError::UnknownKeyPrefix {
                        key: format!("{key}: {}", e.message),
                    })?;
                check_constraint_kind(d, constraint)?;
            }
        }
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
            return Err(not_local(format!(
                "component {name}'s memory domain {}",
                placement.memory_domain
            )));
        }
    }
    Ok(())
}

fn not_local(what: String) -> SpecError {
    SpecError::Structural {
        reason: format!("X7: {what} is not on the local node"),
    }
}

/// A binding's description `(module, selector, profile)`, which is an instance's
/// identity (SB-3): two bindings with equal descriptions name one instance. Shared by
/// `validate` and the coordinator's grouping (KC-4), so the two cannot disagree.
pub(crate) fn binding_description(binding: &crate::binding::Binding) -> (ModuleRef, String) {
    (
        binding.module.clone(),
        format!(
            "{}|{}",
            serde_json::to_string(&binding.selector).unwrap_or_default(),
            serde_json::to_string(&binding.profile).unwrap_or_default()
        ),
    )
}

/// A `ResourceId` handed in as a Rust value: on the local node (X7) and with a path of
/// SB-1's grammar, because its fields are public and a Rust caller can build one
/// unparsed — which would reach a Manifest the Kernel's own deserialiser refuses.
///
/// Rule: X7, SB-1.
pub(crate) fn check_rid(what: &str, id: &ResourceId) -> Result<(), SpecError> {
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
    let Some(m) = registry
        .modules()
        .find(|m| crate::module_api::is_module(m, module))
    else {
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

/// SB-39's guard: an `AdmissionResult` is this Run's only if `matched` holds exactly
/// this Spec's resources and needs, each resource's node is one its bound instance
/// declares, and each need's node is one an instance bound to a resource of this Spec
/// declares (SB-36). "Admitted" alone is satisfied by an empty result, and a stale one
/// names nodes of an instance this profile no longer binds (D99). The Manifest records
/// the whole `matched` (SB-38), so an entry `plan()` does not read is still checked
/// (D107).
///
/// Rule: SB-39, SB-36, SB-30.
pub(super) fn admission_is_this_runs(
    spec: &ExperimentSpec,
    inputs: &CompileInputs<'_>,
    admission: &AdmissionResult,
) -> Result<(), String> {
    let declares = |resource: &Ident, node: &ResourceId| {
        inputs
            .providers
            .get(resource)
            .is_some_and(|p| p.instance().tree.walk_iter().any(|n| n.id == *node))
    };
    let not_this_runs = |why: String| format!("this `AdmissionResult` is not this Run's: {why}");
    let mut expected = BTreeSet::new();
    for (name, req) in &spec.resources {
        let Some(node) = admission.matched.get(name) else {
            return Err(not_this_runs(format!("{name} has no matched node")));
        };
        if !declares(name, node) {
            return Err(not_this_runs(format!(
                "{name}'s matched node {node} is not one its bound instance declares"
            )));
        }
        expected.insert(name.clone());
        for need in req.needs.keys() {
            let key = super::matching::need_key(name, need);
            let Some(node) = admission.matched.get(&key) else {
                return Err(not_this_runs(format!(
                    "{name}'s need {need} has no matched node"
                )));
            };
            if !spec.resources.keys().any(|r| declares(r, node)) {
                return Err(not_this_runs(format!(
                    "{name}'s need {need} names node {node}, which no instance bound to this \
                     Spec's resources declares"
                )));
            }
            expected.insert(key);
        }
    }
    if let Some(extra) = admission.matched.keys().find(|k| !expected.contains(*k)) {
        return Err(not_this_runs(format!(
            "{extra} is neither a resource nor a need of this Spec"
        )));
    }
    Ok(())
}
