//! Resolves and applies the Spec's coercion policy (SB-45, SB-46).

use crate::module_api::ModuleRegistry;
use crate::spec::{Coercion, CoercionPolicy, Constraint, Key, KeyDecl, Namespace, ResourceReq, SpecError, Value, Warning};

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
