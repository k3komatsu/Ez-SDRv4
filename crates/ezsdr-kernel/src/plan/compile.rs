//! Builds a deterministic `ExecutionPlan` from a Spec and its bindings (SB-39).

use std::collections::BTreeMap;

use crate::binding::{AdmissionResult, Binding, BindingProfile};
use crate::contract::{DataContractId, PortRef};
use crate::id::{DataLinkId, ResourceId};
use crate::module_api::{
    ExecutionClass, ExecutorDescriptor, ModuleRef, Pacing, Requested, RfPath, Role, SinkDescriptor,
};
use crate::spec::{ExperimentSpec, Ident, SpecError};
use crate::stream::{BackPressure, DataLinkDecl};

use super::{CompileInputs, DeclaredCost, ExecutionPlan, Fragment, IslandContext};

/// A Provider fragment's content: the binding's selector, plus the request the
/// matcher resolved for the Spec resource of the same name. `requested` is absent
/// only for a binding that names no Spec resource, which SB-22 does not produce for
/// a Provider (SB-39, SB-44, MA-12).
fn provider_content(
    name: &Ident,
    binding: &Binding,
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

pub(super) fn plan(
    spec: &ExperimentSpec,
    profile: &BindingProfile,
    admission: &AdmissionResult,
    inputs: &CompileInputs<'_>,
    transfer_costs: Vec<DeclaredCost>,
) -> Result<ExecutionPlan, SpecError> {
    super::validation::check_structure(spec, profile, inputs)?;
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
    if let Some(v) = super::validation::requested_violations(spec, profile, inputs)
        .into_iter()
        .next()
    {
        return Err(SpecError::Violation(v));
    }
    super::validation::admission_is_this_runs(spec, inputs, admission).map_err(|reason| {
        SpecError::Structural {
            reason: format!("SB-39: {reason}"),
        }
    })?;
    super::links::check_endpoints(spec, inputs, &admission.matched)?;
    let authority = profile.authority.clone();
    let pacing = super::supplied(inputs.authorities, &authority, "AuthorityDescriptor")?.pacing;
    let class = derive_class(profile, pacing)?;

    // One fragment per slot (SB-T1): a Provider per resource, the Authority when it is
    // a slot of its own, a Sink per output, and an Executor per Island.
    let bound = |name: &Ident| -> Result<ModuleRef, SpecError> {
        profile
            .bindings
            .get(name)
            .map(|b| b.module.clone())
            .ok_or_else(|| SpecError::Structural {
                reason: format!("SB-22d: {name} has no binding"),
            })
    };
    let mut fragments: Vec<Fragment> = Vec::new();
    let mut edges: Vec<(Ident, Ident)> = Vec::new();
    // If multiple resource names bind one Provider instance, the first name in
    // BTreeMap key order is the one its arm-after edges refer to (SB-3, SB-23).
    let mut resources_by_instance = BTreeMap::new();
    for name in spec.resources.keys() {
        if let Some(provider) = inputs.providers.get(name) {
            resources_by_instance
                .entry(provider.instance().id.clone())
                .or_insert_with(|| name.clone());
        }
    }
    for name in spec.resources.keys() {
        let binding = super::supplied(&profile.bindings, name, "binding")?;
        let after = arm_after_of(name, inputs, &resources_by_instance);
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
    // Manifest (D92, D98), through `instance`. Its content is empty: the selector is
    // the binding's, in the profile, and nothing reads it here (pre-freeze item 4).
    // ponytail: no ordering edges; whether Providers arm after it is TM-16a/MA-30's
    // Phase 2 call, plan content rather than schema.
    if !spec.resources.contains_key(&authority) {
        let binding = super::supplied(&profile.bindings, &authority, "binding")?;
        fragments.push(Fragment {
            id: authority.clone(),
            instance: binding.module.clone(),
            role: Role::Authority,
            content: serde_json::Value::Null,
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
                SpecError::Structural {
                    reason: "MA-38: island id is not a valid Ident".to_owned(),
                }
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
        let d = super::supplied(inputs.executors, &island.executor, "ExecutorDescriptor")?;
        executors.insert(island.executor.clone(), d.clone());
    }
    let mut sinks: BTreeMap<Ident, SinkDescriptor> = BTreeMap::new();
    for output in &spec.outputs {
        let d = super::supplied(inputs.sinks, &output.id, "Sink instance")?.descriptor();
        sinks.insert(output.id.clone(), d.clone());
    }
    let graph_links: Vec<(PortRef, PortRef, BackPressure)> = spec
        .graph
        .links
        .iter()
        .map(|l| (l.from.clone(), l.to.clone(), l.policy))
        .collect();
    let feeds = output_links(spec, inputs, &admission.matched);
    let feed_links: Vec<(PortRef, PortRef, BackPressure)> = feeds
        .iter()
        .map(|d| (d.from.clone(), d.to.clone(), d.policy))
        .collect();
    super::islands::admit_islands(&IslandContext {
        islands: &profile.placements.islands,
        components: &spec.graph.components,
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
    let order = super::graph::arm_order(&names, &edges)?;
    let order_index: BTreeMap<&Ident, usize> = order
        .iter()
        .enumerate()
        .map(|(index, name)| (name, index))
        .collect();
    fragments.sort_by_key(|f| order_index.get(&f.id).copied().unwrap_or(usize::MAX));

    // The plan carries every data link a Link Module is created for (MA-27a): the
    // graph's, then the output feeds', numbered on from them so no two share an id
    // (D75).
    let mut links = declared_links(spec, inputs, &admission.matched)?;
    let base = links.len() as u32;
    links.extend(feeds.into_iter().enumerate().map(|(i, mut feed)| {
        feed.id = DataLinkId::local(base + i as u32);
        feed
    }));
    Ok(ExecutionPlan {
        fragments,
        links,
        deps: edges,
        authority,
        class,
        transfer_costs,
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
        // MA-41 (KA-8): the section is the closed set `{ class, start_lead_ns }`.
        if let Some(obj) = section.as_object() {
            if let Some(extra) = obj
                .keys()
                .find(|k| !matches!(k.as_str(), "class" | "start_lead_ns"))
            {
                return Err(SpecError::Structural {
                    reason: format!(
                        "MA-41: ezsdr.time carries the field `{extra}`, which is not class or start_lead_ns"
                    ),
                });
            }
        }
        crate::binding::start_lead_ns(&profile.environment)?;
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
fn arm_after_of(
    name: &Ident,
    inputs: &CompileInputs<'_>,
    resources_by_instance: &BTreeMap<ResourceId, Ident>,
) -> Vec<Ident> {
    let Some(provider) = inputs.providers.get(name) else {
        return Vec::new();
    };
    // Only resource slots' Providers are read (SB-22); an entry naming an instance
    // this profile does not bind contributes no edge (SB-39, D49).
    let mut out: Vec<Ident> = provider
        .instance()
        .arm_after
        .iter()
        .filter_map(|rid| resources_by_instance.get(rid))
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
            to: super::links::feed_end(&o.id),
            contract: super::links::endpoint_port(&o.feed.port, spec, inputs, matched)
                .map(|(_, c)| c)
                .unwrap_or_else(|| {
                    DataContractId::parse("ezsdr.control").expect("a valid literal")
                }),
            policy: o.feed.policy,
            capacity: o.feed.capacity,
        })
        .collect()
}

/// The plan's `links`. The contract is resolved the way `validate` resolved it for
/// SC-3 — through [`endpoint_port`](super::links::endpoint_port), which reads a resource
/// endpoint's port off the **bound node** — and not off `graph.components` alone. Resolving components only
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
            let (_, contract) = super::links::endpoint_port(&l.to, spec, inputs, matched)
                .ok_or_else(|| SpecError::Structural {
                    reason: format!(
                        "SB-15: link {i}'s consumer {}.{} names no port of a component or of \
                         a bound node",
                        l.to.component, l.to.port
                    ),
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
