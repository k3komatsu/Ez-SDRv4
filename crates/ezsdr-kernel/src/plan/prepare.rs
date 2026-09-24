//! Validates and merges Module prepare reports before the Run proceeds (SB-30, SB-41, SB-42).

use std::collections::BTreeMap;

use crate::binding::{AdmissionResult, BindingProfile, CheckStage, satisfies, Violation};
use crate::module_api::ModuleError;
use crate::spec::{CapabilityValue, Constraint, ExperimentSpec, Namespace, Value};

use super::{CompileInputs, MergedPrepare, PrepareError, PrepareReport};

pub(super) fn check_effective_narrows(
    declared: &CapabilityValue,
    effective: &CapabilityValue,
) -> Result<(), ModuleError> {
    let within = |value: &Value| {
        satisfies(&Constraint::Eq { value: value.clone() }, declared).unwrap_or(false)
    };
    let narrows = match effective {
        CapabilityValue::One { value } => within(value),
        CapabilityValue::AnyOf { values } => values.iter().all(within),
        CapabilityValue::Range { min, max } => within(min) && within(max),
    };
    if narrows {
        Ok(())
    } else {
        Err(ModuleError::rejected(
            "MA-12: `effective` may narrow a declared capability and must not widen one".to_owned(),
        ))
    }
}

pub(super) fn collect_prepare(
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
    if let Err(reason) = super::validation::admission_is_this_runs(spec, inputs, admission) {
        return Err(PrepareError::Violations(vec![Violation {
            check: Namespace::parse("ezsdr.effective").expect("a valid literal"),
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
            if let Some(stray) = r
                .coercions
                .iter()
                .find(|c| !req.requires.contains_key(&c.key))
            {
                violations.push(Violation {
                    check: Namespace::parse("ezsdr.coercion").expect("a valid literal"),
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
            let policy = super::coercion::coercion_policy(
                spec_coercion.get(&c.key).copied(),
                is_session,
                registry.key_decl(&c.key).ok(),
            );
            match super::coercion::apply_coercion(policy, c) {
                Ok(None) => {}
                Ok(Some(w)) => warned.push(w),
                Err(_) => violations.push(Violation {
                    check: Namespace::parse("ezsdr.coercion").expect("a valid literal"),
                    key: Some(c.key.clone()),
                    requested: Some(c.requested.clone()),
                    reason: format!("SB-46: coercion to {:?} rejected at prepare", c.applied),
                }),
            }
        }
        r.warnings.extend(warned);
    }
    let merged = MergedPrepare::from_reports(ok);
    violations.extend(checks.run(
        environment,
        &merged.effective,
        &BTreeMap::new(),
        CheckStage::Prepare,
    ));

    // MA-12's second sentence, both halves. `effective` may **narrow** a declared
    // capability and must not widen one, and the Spec's own constraints are re-matched
    // against it — otherwise a Provider whose `prepare` disagrees with its `coerce`
    // puts a value the Spec never asked for into `run.effective()` and the Manifest,
    // and MA-12 is a rule with a predicate nobody calls (SB-30, SB-44).
    for (name, req) in &spec.resources {
        let Some(node_id) = admission.matched.get(name) else {
            continue;
        };
        let Some(provider) = providers.get(name) else {
            continue;
        };
        let instance = provider.instance();
        let Some(node) = instance.tree.walk_iter().find(|n| n.id == *node_id) else {
            continue;
        };
        // Each resource is judged by **its own** report, never by `merged.effective`:
        // SB-41 says the merge lets a later fragment's value win for a key two
        // fragments both name and that the Kernel does not interpret it. Reading the
        // merge here interpreted it per resource against data that cannot tell two
        // resources apart, so two channels asking their own line's declared rate
        // refused each other. A Provider fragment's id is the resource name.
        // SB-41: one report per fragment. A resource with none was not prepared, or its
        // report names another fragment, and every check below would be skipped for it
        // in silence (D105).
        let Some(report) = merged.reports.iter().find(|r| r.fragment == *name) else {
            violations.push(Violation {
                check: Namespace::parse("ezsdr.effective").expect("a valid literal"),
                key: None,
                requested: None,
                reason: format!("SB-41: no PrepareReport for fragment {name}"),
            });
            continue;
        };
        for (key, declared) in &node.capabilities {
            let Some(applied) = report.effective.get(key) else {
                continue;
            };
            let effective = CapabilityValue::One {
                value: applied.clone(),
            };
            if let Err(e) = check_effective_narrows(declared, &effective) {
                violations.push(Violation {
                    check: Namespace::parse("ezsdr.effective").expect("a valid literal"),
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
                        violations.push(Violation {
                            check: Namespace::parse("ezsdr.effective").expect("a valid literal"),
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
            let Some(applied) = report.effective.get(key) else {
                continue;
            };
            let effective = CapabilityValue::One {
                value: applied.clone(),
            };
            if !satisfies(constraint, &effective).unwrap_or(false) {
                violations.push(Violation {
                    check: Namespace::parse("ezsdr.effective").expect("a valid literal"),
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
    if violations.is_empty() {
        Ok(merged)
    } else {
        Err(PrepareError::Violations(violations))
    }
}
