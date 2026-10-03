use std::collections::BTreeMap;
use std::sync::atomic::Ordering;

use crate::binding::{CheckStage, Violation};
use crate::event::{Action, ActionId};
use crate::module_api::Requested;
use crate::session::Admitter;
use crate::spec::{Coercion, Constraint, Ident, Key, Value, Warning};
use crate::time::AbsoluteDeadline;

use super::state::{Inst, Origin, Shared, contain, lock};

pub(super) struct AdmittedAction {
    pub(super) action: Action,
    pub(super) inst: Inst,
    pub(super) fragment: Ident,
    pub(super) coercions: Vec<Coercion>,
    pub(super) warnings: Vec<Warning>,
}

pub(super) fn admit(
    shared: &Shared,
    action: Action,
    origin: Origin,
) -> Result<AdmittedAction, Vec<Violation>> {
    let configuration = lock(&shared.configuration).clone();
    admit_with(shared, action, origin, &[], &configuration)
}

pub(super) fn admit_with(
    shared: &Shared,
    mut action: Action,
    origin: Origin,
    incoming: &[Coercion],
    configuration: &BTreeMap<Ident, BTreeMap<Key, Value>>,
) -> Result<AdmittedAction, Vec<Violation>> {
    if shared.frozen.load(Ordering::Acquire) {
        return Err(vec![violation(
            "ezsdr.dispatch",
            "RS-6: dispatch is frozen",
        )]);
    }
    if origin != Origin::Schedule && lock(&shared.machine).check_running().is_err() {
        return Err(vec![violation(
            "ezsdr.run_state",
            "RS-18: the Run is not Running",
        )]);
    }
    if origin == Origin::Module && matches!(action, Action::Stop { target: None }) {
        return Err(vec![violation(
            "ezsdr.target",
            "KC-24: a Module ends a Run only with Abort",
        )]);
    }
    // SB-4: every origin crosses this boundary before coercion or dispatch.
    let values = match &action {
        Action::UpdateParameter { key, value, .. } => value.check_nesting(key.as_str()),
        Action::TxBurst { metadata: values, .. }
        | Action::PeripheralCommand { params: values, .. } => values.iter()
            .try_for_each(|(key, value)| value.check_nesting(key.as_str())),
        _ => Ok(()),
    };
    values.map_err(|error| vec![violation("ezsdr.value", error.to_string())])?;
    let target = action.target().cloned();
    let (inst, fragment) = if let Some(target) = target {
        let (rewritten, inst, fragment) = super::pipeline::rewrite_spec_target(shared, &target)
            .map_err(|reason| vec![violation("ezsdr.target", reason)])?;
        action = super::pipeline::rewrite_action(&action, rewritten);
        (Some(inst), Some(fragment))
    } else {
        (None, None)
    };
    let empty_classes = BTreeMap::new();
    let declared_classes = match (&action, inst) {
        (Action::UpdateParameter { target, .. }, Some(Inst::Executor(_))) => {
            shared.ctx.component_classes_for(target)
        }
        _ => None,
    }.unwrap_or(&empty_classes);
    if let Action::UpdateParameter { key, class, .. } = &action {
        // SB-2: a component declaration cannot override a Provider/Sink key.
        let declared = declared_classes.get(key).copied()
        .or_else(|| shared.ctx.registry.key_decl(key).ok().and_then(|d| d.update_class));
        match declared {
            Some(declared) if declared == *class => {}
            Some(declared) => return Err(vec![violation(
                "ezsdr.update_class",
                format!("RS-52: update states {class:?} for {key}, which declares {declared:?}"),
            )]),
            None => return Err(vec![violation(
                "ezsdr.update_class",
                format!("RS-52: {key} declares no update class"),
            )]),
        }
    }
    let mut coercions = incoming.to_vec();
    if let Action::TxBurst {
        target,
        waveform,
        at,
        requested_at,
        late_policy,
        ..
    } = &mut action
    {
        if origin == Origin::Module && *late_policy == crate::stream::LatePolicy::RejectAtPlan {
            return Err(vec![violation(
                "ezsdr.late_policy",
                "KC-19: SC-27: RejectAtPlan needs a statically known target",
            )]);
        }
        let (Some(Inst::Provider(_)), Some(_)) = (inst, fragment.as_ref()) else {
            return Err(vec![violation(
                "ezsdr.target",
                format!("SC-23: {} is not a Provider stream", target.path),
            )]);
        };
        let record = shared
            .ctx
            .clocks
            .sample_clock_records()
            .into_iter()
            .rev()
            .find(|record| record.stream == *target && record.ended_at.is_none());
        let Some(record) = record else {
            return Err(vec![violation(
                "ezsdr.target",
                format!("SC-23: {target} has no running transmit SampleClock"),
            )]);
        };
        match crate::stream::admit_burst_target(&shared.ctx.clocks, at.time_point, record.domain) {
            Ok(admitted) => {
                *requested_at = admitted.requested_target.map(AbsoluteDeadline::new);
                *at = AbsoluteDeadline::new(admitted.target);
            }
            Err(error) => {
                return Err(vec![violation("ezsdr.target", format!("SC-23b: {error}"))]);
            }
        }
        // RS-44a: a burst's waveform is an input of this Run. KC-9 and KC-28 make that
        // true for the schedule and a Session before admission; nothing did for a
        // Module's burst, which reached its Provider naming bytes nobody holds (KE-2).
        match crate::module_api::InputStore::get(&*shared.store, &waveform.hash) {
            None => {
                return Err(vec![violation(
                    "ezsdr.input",
                    format!("RS-44a: {} is not an input of this Run", waveform.hash),
                )]);
            }
            Some(bytes) if bytes.len() as u64 != waveform.size_bytes => {
                return Err(vec![violation(
                    "ezsdr.input",
                    format!(
                        "RS-44a: input {} holds {} bytes, and the burst declares {}",
                        waveform.hash,
                        bytes.len(),
                        waveform.size_bytes
                    ),
                )]);
            }
            Some(_) => {}
        }
    }
    if origin != Origin::Module {
        if let (
            Action::UpdateParameter { key, value, .. },
            Some(Inst::Provider(index)),
            Some(fragment),
        ) = (&mut action, inst, fragment.as_ref())
        {
            let slot = &shared.providers[index];
            let current = configuration.get(fragment).cloned().unwrap_or_default();
            let mut constraints: BTreeMap<Key, Constraint> = current
                .iter()
                .filter(|(_, value)| value.is_scalar())
                .map(|(key, value)| {
                    (
                        key.clone(),
                        Constraint::Eq {
                            value: value.clone(),
                        },
                    )
                })
                .collect();
            let requested_value = value.clone();
            constraints.insert(
                key.clone(),
                Constraint::Eq {
                    value: requested_value.clone(),
                },
            );
            let Some(resource) = shared
                .routing()
                .and_then(|routing| routing.matched.get(fragment))
                .cloned()
            else {
                return Err(vec![violation(
                    "ezsdr.target",
                    format!("KC-23: no matched Provider for {fragment}"),
                )]);
            };
            let request = Requested {
                resource,
                constraints,
            };
            let report = {
                let guard = lock(&slot.object);
                contain(|| guard.coerce(&request))
            };
            let report = match report {
                Ok(report) => report,
                Err(error) => return Err(vec![coercion_violation(fragment, key, &error.message)]),
            };
            if !report.rejected.is_empty() {
                return Err(report
                    .rejected
                    .iter()
                    .map(|rejected| coercion_violation(fragment, &rejected.key, &rejected.reason))
                    .collect());
            }
            let Some(applied) = report.applied.get(key).cloned() else {
                return Err(vec![coercion_violation(
                    fragment,
                    key,
                    &format!("its Provider applied no value for {key}"),
                )]);
            };
            if applied != requested_value {
                let reason = report
                    .coercions
                    .iter()
                    .find(|coercion| coercion.key == *key)
                    .map(|coercion| coercion.reason.clone())
                    .unwrap_or_else(|| "RS-17: coerced by its Provider".to_owned());
                coercions.push(Coercion {
                    key: key.clone(),
                    requested: requested_value,
                    applied: applied.clone(),
                    reason,
                });
                *value = applied;
            }
        }
    }
    let mut proposed = BTreeMap::new();
    if let (Action::UpdateParameter { key, value, .. }, Some(fragment)) =
        (&action, fragment.as_ref())
    {
        proposed.insert(
            fragment.clone(),
            BTreeMap::from([(key.clone(), value.clone())]),
        );
    }
    let result = Admitter {
        checks: &shared.ctx.checks,
        environment: &shared.ctx.profile.environment,
        declared_classes,
        spec_coercion: &shared.ctx.spec.policies.coercion,
        registry: &shared.ctx.registry,
        is_session: shared.ctx.kind == crate::manifest::RunKind::Session,
    }
    .admit(configuration, &proposed, &coercions, CheckStage::Runtime)?;
    let (Some(inst), Some(fragment)) = (inst, fragment) else {
        return Err(vec![violation(
            "ezsdr.target",
            "KC-24: an Action needs a target",
        )]);
    };
    Ok(AdmittedAction {
        action,
        inst,
        fragment,
        coercions: result.coercions,
        warnings: result.warnings,
    })
}

/// Dispatches one admitted Action and returns its id with the instance and the
/// count KC-21a waits for. The caller holds the admission lock (KC-24a).
pub(super) fn dispatch(shared: &Shared, admitted: AdmittedAction) -> (ActionId, (Inst, u64)) {
    let id = ActionId(shared.next_action.fetch_add(1, Ordering::SeqCst));
    if let Action::UpdateParameter { key, value, .. } = &admitted.action {
        lock(&shared.configuration)
            .entry(admitted.fragment)
            .or_default()
            .insert(key.clone(), value.clone());
    }
    let pushed = shared.queue(admitted.inst).push(admitted.action);
    (id, (admitted.inst, pushed))
}

fn violation(check: &str, reason: impl Into<String>) -> Violation {
    Violation {
        check: crate::spec::Namespace::parse(check).expect("valid admission check"),
        key: None,
        requested: None,
        reason: reason.into(),
    }
}

fn coercion_violation(fragment: &Ident, key: &Key, reason: &str) -> Violation {
    Violation {
        check: crate::spec::Namespace::parse("ezsdr.coercion").expect("valid admission check"),
        key: Some(key.clone()),
        requested: None,
        reason: format!("RS-17: {fragment}: {reason}"),
    }
}
