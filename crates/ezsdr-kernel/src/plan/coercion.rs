//! Resolves and applies the Spec's coercion policy (SB-45, SB-46).

use crate::binding::Violation;
use crate::module_api::ModuleRegistry;
use crate::spec::{Coercion, CoercionPolicy, Constraint, ExperimentSpec, Ident, Key, KeyDecl, Namespace, ResourceReq, SpecError, Value, Warning};

/// SB-2, SB-6: why `value` is not of the kind `key`'s declaration gives it, `None` for a value of that kind and for a key with no declaration (an `ext.` key has none, MA-34).
pub(super) fn reported_kind_error(registry: &ModuleRegistry, key: &Key, value: &Value) -> Option<SpecError> {
    if key.is_extension() {
        return None;
    }
    crate::binding::check_value_kind(registry.key_decl(key)?, key, value).err()
}

/// SB-6: what the Spec asked of `key` when it asked one exact value (`Eq`); a range or a set asks no one value, and a key the Spec does not name asks none. A Violation about a Module's report carries this as `requested`, never the reported value.
pub(super) fn exact_request(req: Option<&ResourceReq>, key: &Key) -> Option<Value> {
    match req?.requires.get(key)? {
        Constraint::Eq { value } => Some(Value::Scalar(value.clone())),
        _ => None,
    }
}

/// SB-6: what the Spec asked of `key` for a fragment at `prepare`: a Provider fragment's `Eq` value (`exact_request`), a Sink fragment's own `params` value, which is its requested configuration (SB-30); a fragment the Spec does not request for, none.
pub(super) fn fragment_request(spec: &ExperimentSpec, fragment: &Ident, key: &Key) -> Option<Value> {
    match spec.outputs.iter().find(|output| output.id == *fragment) {
        Some(output) => output.params.get(key).cloned(),
        None => exact_request(spec.resources.get(fragment), key),
    }
}

/// SB-44: the violation for a reported coercion whose `requested` is not what the experimenter asked for its key, `None` when it is. `asked` is `exact_request` or `fragment_request`; a key with no exact request is not checked, and `whose` names the reporting Module in the reason.
pub(super) fn false_request(asked: Option<Value>, c: &Coercion, whose: &str) -> Option<Violation> {
    let asked = asked.filter(|asked| *asked != c.requested)?;
    Some(Violation {
        check: Namespace::parse("ezsdr.coercion").expect("a valid literal"),
        key: Some(c.key.clone()),
        reason: format!("SB-44: {whose} reports a coercion of {} whose requested value is {:?}, but the Spec asked {:?}", c.key, c.requested, asked),
        requested: Some(asked),
    })
}

pub(super) fn coercion_policy(spec_override: Option<CoercionPolicy>, is_session: bool, decl: Option<&KeyDecl>) -> CoercionPolicy {
    if let Some(p) = spec_override {
        return p;
    }
    if is_session {
        return CoercionPolicy::Warn;
    }
    decl.map(|d| d.coercion_default).unwrap_or(CoercionPolicy::Warn)
}

pub(super) fn apply_coercion(policy: CoercionPolicy, coercion: &Coercion) -> Result<Option<Warning>, SpecError> {
    match policy {
        CoercionPolicy::Reject => Err(SpecError::CoercionRejected(coercion.clone())),
        CoercionPolicy::Warn => Ok(Some(Warning {
            source: Namespace::parse("ezsdr.coercion").expect("a valid literal"),
            message: format!("{} was coerced from {:?} to {:?}: {}", coercion.key, coercion.requested, coercion.applied, coercion.reason),
        })),
        CoercionPolicy::Accept => Ok(None),
    }
}
