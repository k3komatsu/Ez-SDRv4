//! Validates graph endpoints, link contracts, and output feeds (SB-15, SB-17).

use std::collections::{BTreeMap, BTreeSet};

use crate::contract::{DataContractId, PortDirection, PortRef};
use crate::id::{DataLinkId, ResourceId};
use crate::spec::{ExperimentSpec, Ident, SpecError};
use crate::stream::DataLinkDecl;

use super::CompileInputs;

/// `validate()`'s endpoint checks, which need the matched nodes, and which `plan()`
/// runs again over the `AdmissionResult`'s (SB-15, SB-15a, SB-17, SC-3, SC-21, D99).
///
/// Rule: SB-T4.
pub(super) fn check_endpoints(
    spec: &ExperimentSpec,
    inputs: &CompileInputs<'_>,
    matched: &BTreeMap<Ident, ResourceId>,
) -> Result<(), SpecError> {
    check_graph_links(spec, inputs, matched)?;
    check_outputs(spec, inputs, matched)
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
                (&link.from, from_dir, PortDirection::Out),
                (&link.to, to_dir, PortDirection::In),
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
                .map_err(|e| SpecError::Structural {
                    reason: e.to_string(),
                })?;
        }
        // SC-19's mandatory policy is checked above; a Sink is never a link's
        // consumer here, because a Sink is bound rather than placed and its own
        // link is the `outputs[]` entry that `check_outputs` validates (SB-17).
    }
    Ok(())
}

/// A feed's consumer end, `{output id, "in"}`: how a `LinkPlacement` and the plan's
/// `links` name the link into a bound Sink (SB-25, D76).
pub(super) fn feed_end(output: &Ident) -> PortRef {
    PortRef {
        component: output.clone(),
        port: Ident::parse("in").expect("a valid literal"),
    }
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
                DataContractId::parse("ezsdr.control").expect("a valid literal")
            }),
            policy: feed.policy,
            capacity: feed.capacity,
        };
        crate::stream::check_sink_link(&decl, true).map_err(|e| SpecError::Structural {
            reason: e.to_string(),
        })?;

        // The source port must exist, on a component or on a bound resource.
        let (direction, contract) = source.ok_or_else(|| SpecError::Structural {
            reason: format!(
                "SB-17: output {} names source port {}:{}, which does not exist",
                output.id, feed.port.component, feed.port.port
            ),
        })?;
        // SB-15a: an output's feed leaves an `out` port. A Sink records what a port
        // produces, so a feed from an `in` port names the wrong end of the stream.
        if direction != PortDirection::Out {
            return Err(SpecError::Structural {
                reason: format!(
                    "SB-15a: output {}'s feed names {}:{}, which is an {direction:?} port",
                    output.id, feed.port.component, feed.port.port
                ),
            });
        }

        // MA-25: the bound Sink must write this artifact kind and accept the
        // source port's contract.
        let d =
            super::supplied(inputs.sinks, &output.id, "Sink instance")?.descriptor();
        if !d.artifact_kinds.contains(&output.kind) {
            return Err(SpecError::Structural {
                reason: format!(
                    "SB-17: output {} wants artifact kind {}, which its Sink does not write",
                    output.id, output.kind
                ),
            });
        }
        if !d
            .contracts
            .iter()
            .any(|c| inputs.contracts.check_link(&contract, c).is_ok())
        {
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
pub(super) fn endpoint_port(
    r: &PortRef,
    spec: &ExperimentSpec,
    inputs: &CompileInputs<'_>,
    matched: &BTreeMap<Ident, ResourceId>,
) -> Option<(PortDirection, DataContractId)> {
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
    // and the hardware will not honour. `matched` holds resources only, so only a
    // resource slot's Provider is read (SB-22).
    let bound = matched.get(name)?;
    let provider = inputs.providers.get(name)?;
    provider
        .instance()
        .tree
        .walk_iter()
        .find(|n| n.id == *bound)?
        .ports
        .iter()
        .find(|p| p.name == r.port)
        .map(|p| (p.direction, p.contract.clone()))
}
