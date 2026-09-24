//! Matches resource requests against bound Provider capabilities (SB-6, SB-34, SB-36).

use std::collections::BTreeMap;

use crate::binding::{satisfies, AdmissionResult, PreviewedCoercion, Violation};
use crate::id::ResourceId;
use crate::module_api::{Provider, Requested, Resource};
use crate::spec::{
    Ident, Key, Namespace, RejectedConstraint, ResourceReq, SpecError, SubResourceReq,
};

use super::CompileInputs;

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
pub(super) fn pick_node<'a>(
    tree: &'a Resource,
    name: &Ident,
    req: &ResourceReq,
    taken: &BTreeMap<ResourceId, Ident>,
) -> Result<&'a Resource, SpecError> {
    let candidates: Vec<&Resource> = tree.walk_iter().filter(|n| n.kind == req.kind).collect();
    let satisfies_all = |n: &Resource| {
        req.requires.iter().all(|(k, c)| {
            n.capabilities
                .get(k)
                .is_some_and(|cap| satisfies(c, cap).unwrap_or(false))
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

pub(super) fn need_satisfied(need: &SubResourceReq, node: &Resource) -> bool {
    need.requires.iter().all(|(k, c)| {
        node.capabilities
            .get(k)
            .is_some_and(|cap| satisfies(c, cap).unwrap_or(false))
    })
}

/// Matches a resource's constraints against the bound node's capabilities, offering
/// a coercible key to the Provider's `coerce` when the declared capability does not
/// satisfy it directly (SB-6, SB-7).
pub(super) fn match_constraints(
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
        let decl = if key.is_extension() {
            None
        } else {
            inputs.registry.key_decl(key).ok()
        };
        let cap = node.capabilities.get(key);
        let direct = match cap {
            Some(cap) => satisfies(constraint, cap).map_err(|e| match e {
                SpecError::KeyShape {
                    expected, found, ..
                } => SpecError::KeyShape {
                    key: key.to_string(),
                    expected,
                    found,
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
                reason:
                    "SB-7: the declared capability does not satisfy it and the key is not coercible"
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
        .coerce(&Requested {
            resource: node.id.clone(),
            constraints: req.requires.clone(),
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
    let (strays, previewed): (Vec<_>, Vec<_>) = report
        .coercions
        .into_iter()
        .partition(|c| !req.requires.contains_key(&c.key));
    for stray in &strays {
        out.violations.push(Violation {
            check: Namespace::parse("ezsdr.coercion").expect("a valid literal"),
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
    out.coercions_preview
        .extend(
            previewed
                .into_iter()
                .map(|coercion| PreviewedCoercion {
                    resource: name.clone(),
                    coercion,
                }),
        );
    out.warnings.extend(report.warnings);
    Ok(())
}

/// SB-36's key for a need in `matched`: `<resource>_<need>`.
///
/// Rule: SB-36.
pub(super) fn need_key(resource: &Ident, need: &Ident) -> Ident {
    Ident::parse(&format!("{resource}_{need}")).expect("two Idents joined by `_` are an Ident")
}
