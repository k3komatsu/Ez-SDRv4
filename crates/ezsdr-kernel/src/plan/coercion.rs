//! Resolves and applies the Spec's coercion policy (SB-45, SB-46).

use crate::spec::{Coercion, CoercionPolicy, KeyDecl, Namespace, SpecError, Warning};

pub(super) fn coercion_policy(
    spec_override: Option<CoercionPolicy>,
    is_session: bool,
    decl: Option<&KeyDecl>,
) -> CoercionPolicy {
    if let Some(p) = spec_override {
        return p;
    }
    if is_session {
        return CoercionPolicy::Warn;
    }
    decl.map(|d| d.coercion_default)
        .unwrap_or(CoercionPolicy::Warn)
}

pub(super) fn apply_coercion(
    policy: CoercionPolicy,
    coercion: &Coercion,
) -> Result<Option<Warning>, SpecError> {
    match policy {
        CoercionPolicy::Reject => Err(SpecError::CoercionRejected(coercion.clone())),
        CoercionPolicy::Warn => Ok(Some(Warning {
            source: Namespace::parse("ezsdr.coercion").expect("a valid literal"),
            message: format!(
                "{} was coerced from {:?} to {:?}: {}",
                coercion.key, coercion.requested, coercion.applied, coercion.reason
            ),
        })),
        CoercionPolicy::Accept => Ok(None),
    }
}
