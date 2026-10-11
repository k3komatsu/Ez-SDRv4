use std::collections::BTreeMap;
use std::sync::atomic::Ordering;

use crate::binding::{CheckStage, Violation};
use crate::event::{Action, ActionId, PendingAction};
use crate::module_api::Requested;
use crate::spec::{Coercion, Constraint, Ident, Key, Value, Warning};
use crate::time::AbsoluteDeadline;

use super::state::{Inst, Origin, Shared, contain, lock};

pub(super) struct AdmittedAction {
    /// As authored: the log and every record read in the Spec's own names (KC-23).
    pub(super) action: Action,
    /// The node a `{resource}` target was resolved to, which a Provider matches on.
    pub(super) node: Option<crate::id::ResourceId>,
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
    admit_with(
        shared,
        PendingAction::Action(action),
        origin,
        &[],
        &configuration,
    )
}

pub(super) fn admit_with(
    shared: &Shared,
    input: PendingAction,
    origin: Origin,
    incoming: &[Coercion],
    configuration: &BTreeMap<Ident, BTreeMap<Key, Value>>,
) -> Result<AdmittedAction, Vec<Violation>> {
    // 1. dispatch is not frozen (RS-6 step 1)
    if shared.frozen.load(Ordering::Acquire) {
        return Err(vec![violation(
            "ezsdr.dispatch",
            "RS-6: dispatch is frozen",
        )]);
    }
    // 2. for a Session or Module origin, the Run is Running (RS-18)
    if origin != Origin::Schedule && lock(&shared.machine).check_running().is_err() {
        return Err(vec![violation(
            "ezsdr.run_state",
            "RS-18: the Run is not Running",
        )]);
    }
    // 3. the target resolves in its own role (KC-23), and a Module's `Stop { target:
    // None }` is refused here. Before the payload is read, so a refusal can never
    // transcribe a payload this step never looked at. A class-pending request states
    // its own target, so it is resolved exactly like a finished Action's.
    if origin == Origin::Module
        && matches!(
            input,
            PendingAction::Action(Action::Stop { target: None })
        )
    {
        return Err(vec![violation(
            "ezsdr.target",
            "KC-24: a Module ends a Run only with Abort",
        )]);
    }
    // 3. the target resolved in its own role (KC-23). An Action with no target
    // (`Stop {}`, `Abort`) is not resolved.
    let (node, inst, fragment) = match input.target() {
        Some(target) => {
            let (node, inst, fragment) = super::pipeline::resolve_target(shared, target)
                .map_err(|reason| vec![violation("ezsdr.target", reason)])?;
            (node, Some(inst), Some(fragment))
        }
        None => (None, None, None),
    };
    // 4. SB-9a: every origin crosses this boundary before coercion or dispatch. The
    // refusal carries `requested: None` and a reason naming only the position and the
    // violation: a Map key that is not ASCII must never enter a rejection record the
    // log and the sealed Manifest will hold, and the offending key's own spelling is
    // exactly what must not be transcribed (KC-24; SB-9a). The `key` field is the
    // update's own key where there is one — never the offending map key, which need
    // not even parse as a `Key`; a `TxBurst`/`Command`/`Emit` payload sits under no key.
    let ascii = |key: Option<&Key>, position: &str| {
        vec![Violation {
            check: crate::spec::Namespace::parse("ezsdr.value").expect("valid admission check"),
            key: key.cloned(),
            requested: None,
            reason: format!("SB-9a: a map key under {position} is not ASCII"),
        }]
    };
    match &input {
        PendingAction::Update(pending) => {
            if pending.value.check_ascii_keys(pending.key.as_str()).is_err() {
                return Err(ascii(Some(&pending.key), pending.key.as_str()));
            }
        }
        PendingAction::Action(Action::UpdateParameter { key, value, .. }) => {
            if value.check_ascii_keys(key.as_str()).is_err() {
                return Err(ascii(Some(key), key.as_str()));
            }
        }
        PendingAction::Action(Action::TxBurst { metadata: values, .. }) => {
            if values
                .iter()
                .any(|(key, value)| value.check_ascii_keys(key.as_str()).is_err())
            {
                return Err(ascii(None, "metadata"));
            }
        }
        PendingAction::Action(Action::Command { params: values, .. }) => {
            if values
                .iter()
                .any(|(key, value)| value.check_ascii_keys(key.as_str()).is_err())
            {
                return Err(ascii(None, "params"));
            }
        }
        // Item 3: an `Emit`'s payload is a `Value` tree, so the check is recursive.
        // The refusal is still admission's, before dispatch: the Action still reaches
        // its target Module (which may not support it), and no collector receives it
        // directly.
        PendingAction::Action(Action::Emit { event, .. }) => {
            if crate::spec::check_ascii_keys(&event.payload).is_err() {
                return Err(ascii(None, "payload"));
            }
        }
        PendingAction::Action(_) => {}
    }
    // 5. the declaration's update class, and for a Module's finished Action the
    // class it states (RS-52, SB-2, MA-35). A class-pending request is completed
    // here, from the Spec's declaration, and a fixed or missing one is refused with
    // the same violation a finished Action's mismatching class gets.
    let mut action = match input {
        PendingAction::Action(action) => {
            if let Action::UpdateParameter { target, key, value, class, .. } = &action {
                // RS-52 (item 3): the declared class comes from the target's own
                // declaration — a `{resource}`/`{output}` key from its `KeyDecl`, a
                // `{component}` key from that component's `ParamDecl` — through the one
                // shared lookup, and a fixed or missing component declaration is not
                // resolved through a Vocabulary declaration.
                match shared.ctx.spec.declared_class(target, key, &shared.ctx.registry) {
                    Some(declared) if declared == *class => {}
                    Some(declared) => {
                        return Err(vec![violation_with(
                            "ezsdr.update_class",
                            key,
                            value,
                            format!(
                                "RS-52: update states {class:?} for {key}, which declares {declared:?}"
                            ),
                        )])
                    }
                    None => {
                        return Err(vec![violation_with(
                            "ezsdr.update_class",
                            key,
                            value,
                            format!("RS-52: {key} declares no update class"),
                        )])
                    }
                }
            }
            action
        }
        PendingAction::Update(pending) => {
            let Some(class) = shared.ctx.spec.declared_class(
                &pending.target,
                &pending.key,
                &shared.ctx.registry,
            ) else {
                return Err(vec![violation_with(
                    "ezsdr.update_class",
                    &pending.key,
                    &pending.value,
                    format!("RS-52: {} declares no update class", pending.key),
                )]);
            };
            Action::UpdateParameter {
                target: pending.target,
                key: pending.key,
                value: pending.value,
                class,
                at: pending.at,
            }
        }
    };
    // 6. an `UpdateParameter` whose owner is a `KeyDecl` has a requested value of
    // that declaration's kind (SB-2, SB-6). A component parameter has an opaque
    // schema and no kind check: its key is owned by the component's own `ParamDecl`,
    // and a Vocabulary key that happens to share the name is not consulted (item 3,
    // plan §5).
    if let Action::UpdateParameter { target, key, value, .. } = &action {
        if !matches!(target, crate::event::Target::Component { .. }) {
            if let Some(decl) = shared.ctx.registry.key_decl(key) {
                crate::binding::check_value_kind(decl, value)
                    .map_err(|error| vec![violation_with("ezsdr.value", key, value, error.to_string())])?;
            }
        }
    }
    // 7. for a `TxBurst`: … (KC-24 step 7)
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
        let (Some(Inst::Provider(_)), Some(node)) = (inst, node.as_ref()) else {
            return Err(vec![violation(
                "ezsdr.target",
                format!("SC-23: {target} is not a Provider stream"),
            )]);
        };
        let Some(record) = shared.running_clock(node) else {
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
    let (Some(inst), Some(fragment)) = (inst, fragment) else {
        return Err(vec![violation(
            "ezsdr.target",
            "KC-24: an Action needs a target",
        )]);
    };
    // 8–11. a Provider update from the Session or the schedule: the scalar rule
    // (SB-5), the matched Provider (KC-23), the coerce (KC-26) and the coerced
    // value's kind.
    let mut coercions = incoming.to_vec();
    if let (Action::UpdateParameter { key, value, .. }, Inst::Provider(index)) = (&mut action, inst) {
        if origin != Origin::Module {
            coerce_provider(
                shared,
                index,
                &fragment,
                key,
                value,
                &mut coercions,
                configuration,
            )?;
        }
    }
    // 12. `proposed` for an `UpdateParameter` is its key and value after step 10's
    // coercion, and every fragment an update addresses gets it — a Provider, a Sink
    // output or a component alike (KC-24 step 12; SB-30's third point). A Module's
    // update is not coerced but is still proposed.
    let mut proposed = BTreeMap::new();
    if let Action::UpdateParameter { key, value, .. } = &action {
        proposed.insert(
            fragment.clone(),
            BTreeMap::from([(key.clone(), value.clone())]),
        );
    }
    // 13. the registered checks and the coercion policy (SB-30, SB-45).
    let admitted = shared
        .admitter()
        .admit(configuration, &proposed, &coercions, CheckStage::Runtime)?;
    Ok(AdmittedAction {
        action,
        node,
        inst,
        fragment,
        coercions: admitted.coercions,
        warnings: admitted.warnings,
    })
}

/// KC-24 steps 8–11 for an `UpdateParameter` to a Provider fragment from the
/// Session or schedule origin: the value is a scalar (SB-5), the `matched` Provider
/// exists (KC-23), `coerce` runs (KA-6, KC-26), and the value it applied fits the
/// declaration. Step 12's registered checks run afterwards, on the caller's side.
///
/// Rule: KC-24, KC-26, SB-5.
#[allow(clippy::too_many_arguments)]
fn coerce_provider(
    shared: &Shared,
    index: usize,
    fragment: &Ident,
    key: &Key,
    value: &mut Value,
    coercions: &mut Vec<Coercion>,
    configuration: &BTreeMap<Ident, BTreeMap<Key, Value>>,
) -> Result<(), Vec<Violation>> {
    let slot = &shared.providers[index];
    let current = configuration.get(fragment).cloned().unwrap_or_default();
    let mut constraints: BTreeMap<Key, Constraint> = current
        .iter()
        .filter_map(|(key, value)| {
            let value = value.as_scalar()?.clone();
            Some((key.clone(), Constraint::Eq { value }))
        })
        .collect();
    let requested_value = value.clone();
    // 8. A Provider parameter is offered to `coerce` as an `Eq` constraint, whose
    // value is a scalar (SB-5).
    let Some(requested) = value.as_scalar().cloned() else {
        return Err(vec![violation_with(
            "ezsdr.value",
            key,
            &requested_value,
            format!("SB-5: {key} is a Provider parameter, whose value is a scalar"),
        )]);
    };
    constraints.insert(key.clone(), Constraint::Eq { value: requested });
    // 9. the `matched` Provider the coerce below would use exists. Step 3 already
    // required it for this fragment — `resolve_target` maps a resource to
    // `matched[resource]` — and routing is installed once, so this is its guarantee,
    // not a refusal (3a's P2-4: no unkillable branch).
    let resource = shared
        .routing()
        .and_then(|routing| routing.matched.get(fragment))
        .cloned()
        .expect("KC-24 step 3 resolved this Provider target through routing.matched");
    let request = Requested {
        resource,
        constraints,
    };
    let report = {
        let guard = lock(&slot.object);
        contain(|| guard.coerce(&request))
    };
    // 10. an error, a rejected constraint, or an applied value missing for the key
    // is refused (KA-6, KC-26).
    let report = match report {
        Ok(report) => report,
        Err(error) => {
            return Err(vec![coercion_violation(
                fragment,
                key,
                &requested_value,
                &error.message,
            )]);
        }
    };
    if !report.rejected.is_empty() {
        return Err(report
            .rejected
            .iter()
            .map(|rejected| {
                coercion_violation(fragment, &rejected.key, &requested_value, &rejected.reason)
            })
            .collect());
    }
    let Some(applied) = report.applied.get(key).cloned() else {
        return Err(vec![coercion_violation(
            fragment,
            key,
            &requested_value,
            &format!("its Provider applied no value for {key}"),
        )]);
    };
    // The comparison is of kinds, not of `Scalar`'s numeric `PartialEq`: `Int(3)` and
    // `Num(3.0)` are the same number but not the same value (SB-6), and a Provider
    // that applied one where the other was requested has coerced.
    if applied != requested_value || applied.kind() != requested_value.kind() {
        let reason = report
            .coercions
            .iter()
            .find(|coercion| coercion.key == *key)
            .map(|coercion| coercion.reason.clone())
            .unwrap_or_else(|| "RS-17: coerced by its Provider".to_owned());
        coercions.push(Coercion {
            key: key.clone(),
            requested: requested_value.clone(),
            applied: applied.clone(),
            reason,
        });
        *value = applied;
    }
    // 11. the value the coerce applied fits the declaration of step 6 as well, or
    // the update is refused and never recorded in the configuration (KC-25): a
    // Provider that snapped an `Int` key to a float, say, would otherwise write a
    // value the declaration does not admit.
    if let Some(decl) = shared.ctx.registry.key_decl(key) {
        if !crate::spec::kind_accepts(decl.kind, value.kind()) {
            return Err(vec![violation_with(
                "ezsdr.value",
                key,
                &requested_value,
                format!(
                    "SB-2: the coercion of {key} applied a value whose kind is not the declaration's"
                ),
            )]);
        }
    }
    Ok(())
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
    let pushed = shared.queue(admitted.inst).push(crate::module_api::Dispatched {
        id,
        action: admitted.action,
        node: admitted.node,
    });
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



/// A refusal that names the key and transcribes the requested value. KC-24 places
/// every such refusal at or after step 4, so that a payload whose Map key is not
/// ASCII is never transcribed into a rejection record.
fn violation_with(
    check: &str,
    key: &Key,
    requested: &Value,
    reason: impl Into<String>,
) -> Violation {
    Violation {
        check: crate::spec::Namespace::parse(check).expect("valid admission check"),
        key: Some(key.clone()),
        requested: Some(requested.clone()),
        reason: reason.into(),
    }
}

fn coercion_violation(fragment: &Ident, key: &Key, requested: &Value, reason: &str) -> Violation {
    violation_with(
        "ezsdr.coercion",
        key,
        requested,
        format!("RS-17: {fragment}: {reason}"),
    )
}
