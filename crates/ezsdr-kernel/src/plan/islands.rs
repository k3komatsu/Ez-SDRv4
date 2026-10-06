//! Validates declared Island placements and their links (MA-39, MA-40).

use std::collections::{BTreeMap, BTreeSet};

use crate::binding::LinkPlacement;
use crate::contract::PortRef;
use crate::id::{IslandId, MemoryDomainId};
use crate::module_api::ModuleError;
use crate::spec::Ident;

use super::{EdgeKind, GraphEdge, IslandContext};

pub(super) fn admit_islands(ctx: &IslandContext<'_>) -> Result<(), ModuleError> {
    // SB-25 / D76: a placement names its data link by its two ends. Coverage is
    // exact over graph links and output feeds, so neither an omitted nor a stale
    // placement is ignored, and a pair named twice is ambiguous rather than
    // first-wins.
    let pair = |from: &PortRef, to: &PortRef| {
        format!(
            "{}.{} -> {}.{}",
            from.component, from.port, to.component, to.port
        )
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
    if let Some(stale) = ctx
        .link_placements
        .iter()
        .find(|p| !expected.contains(&(&p.from, &p.to)))
    {
        return Err(ModuleError::rejected(format!(
            "SB-25: link placement {} names no graph link or output feed",
            pair(&stale.from, &stale.to)
        )));
    }

    // Every component placed exactly once.
    let mut placed: BTreeMap<&Ident, usize> = ctx.components.keys().map(|c| (c, 0)).collect();
    for island in ctx.islands {
        for c in island.components.iter().map(|entry| &entry.component) {
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
        for entry in &island.components {
            let name = &entry.component;
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
            // The memory domain its Island states for it (SB-25, MA-39).
            if !executor.memory_domains.contains(&entry.memory_domain) {
                return Err(ModuleError::rejected(format!(
                    "MA-39: {name}'s memory domain {} is not among executor {}'s memory_domains",
                    entry.memory_domain, island.executor
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

    let island_by_component: BTreeMap<&Ident, IslandId> = ctx
        .islands
        .iter()
        .flat_map(|island| {
            island
                .components
                .iter()
                .map(move |entry| (&entry.component, island.id))
        })
        .collect();

    // MA-22's cycle rule, which MA-39 names as one of its own checks. Every graph
    // link is a `stream.*` edge in Phase 1, so none of them may close a cycle.
    let nodes: Vec<Ident> = ctx
        .components
        .keys()
        .chain(ctx.resource_endpoints.iter())
        .cloned()
        .collect();
    let edges: Vec<GraphEdge> = ctx
        .graph_links
        .iter()
        .map(|(from, to, _)| GraphEdge {
            from: from.component.clone(),
            to: to.component.clone(),
            kind: EdgeKind::Stream,
            crosses_island: island_by_component.get(&from.component)
                != island_by_component.get(&to.component),
        })
        .collect();
    super::graph::check_cycles(&nodes, &edges)?;

    // A link whose two ends share a memory domain needs nothing more; otherwise its
    // selected Link (SB-25) `connects` the pair, whether the ends are in one Island
    // or in two (D77).
    let is_resource = |r: &PortRef| ctx.resource_endpoints.contains(&r.component);
    // Each component's memory domain is the one its Island's entry states (SB-25).
    let domains: BTreeMap<&Ident, MemoryDomainId> = ctx
        .islands
        .iter()
        .flat_map(|island| &island.components)
        .map(|entry| (&entry.component, entry.memory_domain))
        .collect();
    let domain_of = |r: &PortRef| domains.get(&r.component).copied();
    // D81: a feed's consumer is the bound Sink, whose readable domains its descriptor
    // declares. A resource producer is skipped as for a graph link (D31).
    for (from, to, _policy) in ctx.feed_links {
        if is_resource(from) {
            continue;
        }
        let fd = domain_of(from).ok_or_else(|| {
            ModuleError::rejected(format!(
                "MA-39: feed {}:{} -> {} leaves an unplaced component",
                from.component, from.port, to.component
            ))
        })?;
        let sink = ctx.sinks.get(&to.component).ok_or_else(|| {
            ModuleError::rejected(format!(
                "SB-22f: no Sink instance for output {}",
                to.component
            ))
        })?;
        if sink.memory_domains.contains(&fd) {
            continue;
        }
        let placement = selected[&(from, to)];
        let joined = ctx.links[&placement.link].connects.iter().any(|(a, b)| {
            (*a == fd && sink.memory_domains.contains(b))
                || (*b == fd && sink.memory_domains.contains(a))
        });
        if !joined {
            return Err(ModuleError::rejected(format!(
                "MA-39: {} is in memory domain {} and output {}'s Sink reads [{}]; selected Link Module {} {} does not connect them",
                from.component,
                fd,
                to.component,
                sink.memory_domains
                    .iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
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
        let (fi, ti) = (
            island_by_component.get(&from.component),
            island_by_component.get(&to.component),
        );
        let (Some(fi), Some(ti)) = (fi, ti) else {
            return Err(ModuleError::rejected(format!(
                "MA-39: link {}:{} -> {}:{} touches an unplaced component",
                from.component, from.port, to.component, to.port
            )));
        };
        let (fd, td) = (domain_of(from), domain_of(to));
        if let (Some(fd), Some(td)) = (fd, td) {
            if fd == td {
                continue;
            }
            let joined = descriptor.connects.iter().any(|pair| pair == &(fd, td) || pair == &(td, fd));
            if !joined {
                return Err(ModuleError::rejected(format!(
                    "MA-39: {} (island {}) and {} (island {}) are in different memory domains and selected Link Module {} {} does not connect them",
                    from.component,
                    fi,
                    to.component,
                    ti,
                    placement.link.id,
                    placement.link.version
                )));
            }
        }
    }
    Ok(())
}
