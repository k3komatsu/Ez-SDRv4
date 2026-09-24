//! Fragment dependency ordering and cycle checks (SB-39, MA-22).

use std::collections::{BTreeMap, BTreeSet};

use crate::module_api::ModuleError;
use crate::spec::{Ident, SpecError};

use super::{EdgeKind, GraphEdge};

pub(super) fn arm_order(
    nodes: &[Ident],
    edges: &[(Ident, Ident)],
) -> Result<Vec<Ident>, SpecError> {
    let mut indegree: BTreeMap<&Ident, usize> = nodes.iter().map(|node| (node, 0)).collect();
    if indegree.len() != nodes.len() {
        // A duplicate name would collapse in the map and make edge lookup ambiguous.
        let mut seen = BTreeSet::new();
        let dup = nodes
            .iter()
            .find(|n| !seen.insert(*n))
            .expect("lengths differ");
        return Err(SpecError::Structural {
            reason: format!("SB-39: two fragments share the id {dup}"),
        });
    }
    let mut outgoing: BTreeMap<&Ident, BTreeSet<&Ident>> =
        nodes.iter().map(|node| (node, BTreeSet::new())).collect();
    for (before, after) in edges {
        // A dangling edge is a defect in the profile, not an ordering; reported as
        // itself rather than silently dropped or turned into a cycle.
        if !indegree.contains_key(before) || !indegree.contains_key(after) {
            return Err(SpecError::Structural {
                reason: format!(
                    "SB-39: ordering edge {before} -> {after} names an unknown fragment"
                ),
            });
        }
        if outgoing.get_mut(before).expect("checked above").insert(after) {
            *indegree.get_mut(after).expect("checked above") += 1;
        }
    }

    // Kahn's algorithm keeps the smallest currently-ready name first (SB-39).
    // Updating only affected successors avoids rescanning every edge each round.
    let mut ready: BTreeSet<&Ident> = indegree
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(node, _)| *node)
        .collect();
    let mut out: Vec<Ident> = Vec::with_capacity(nodes.len());
    while let Some(next) = ready.pop_first() {
        for after in outgoing[&next].iter() {
            let count = indegree.get_mut(after).expect("edge target is a node");
            *count -= 1;
            if *count == 0 {
                ready.insert(*after);
            }
        }
        out.push(next.clone());
    }
    if out.len() != nodes.len() {
        let done: BTreeSet<&Ident> = out.iter().collect();
        let stuck = nodes
            .iter()
            .filter(|node| !done.contains(node))
            .map(Ident::as_str)
            .collect::<Vec<_>>();
        return Err(SpecError::ArmCycle {
            path: stuck.join(" -> "),
        });
    }
    Ok(out)
}

pub(super) fn release_order(arm_order: &[Ident]) -> Vec<Ident> {
    arm_order.iter().rev().cloned().collect()
}

pub(super) fn check_cycles(nodes: &[Ident], edges: &[GraphEdge]) -> Result<(), ModuleError> {
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
