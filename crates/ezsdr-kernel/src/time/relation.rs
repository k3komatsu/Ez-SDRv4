//! Cross-root conversion by measurement (`01-time-model.md` TM-5, TM-14, TM-18).

use serde::{Deserialize, Serialize};

use super::{ClockRegistry, Duration, Rescaled, TimeError, TimePoint, UncertainTimePoint};
use crate::id::ClockDomainId;

/// The window over which a `ClockRelation` may be applied (TM-14).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Validity {
    /// First instant, in the source domain, at which the relation holds.
    pub from: TimePoint,
    /// Last instant, in the source domain; absent means the bound grows without limit.
    pub to: Option<TimePoint>,
}

/// A measured relation between two roots: the only path between domains that are
/// not exactly related, and the record every Run carries from its root to `utc`.
///
/// `drift` and `drift_uncertainty` are the only floating-point quantities in the
/// time model, and neither reaches a stored `TimePoint` unrounded (TM-14, X9).
///
/// Rule: TM-5, TM-14, TM-18. Vision §24, §50.
// The fields are private and `new`, which the deserialiser goes through, checks
// TM-14's shape, so a malformed relation cannot exist.
#[derive(Clone, PartialEq, Debug, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClockRelation {
    /// The domain being converted from.
    source: ClockDomainId,
    /// The domain being converted to.
    target: ClockDomainId,
    /// When the measurement was taken, in the source domain.
    measured_at: TimePoint,
    /// The image of `measured_at` in the target domain.
    offset: TimePoint,
    /// Dimensionless: target seconds per source second, minus 1. Refused when
    /// non-finite, because `serde_json` would otherwise write it as `null` and a
    /// NaN drift would share a Manifest hash with one that was never measured
    /// (OV-15, X9).
    drift: f64,
    /// Dimensionless, non-negative: the one-sided error bound on `drift`. Zero
    /// claims the drift is known exactly and is expected to be rare. Refused when
    /// non-finite (OV-15, X9).
    drift_uncertainty: f64,
    /// The bound at `measured_at`, in target ticks, before drift error accumulates.
    uncertainty: Duration,
    /// Namespaced name of the measurement method, for example `dev.pps_poll`.
    method: String,
    /// The window over which the relation may be applied.
    valid: Validity,
}

impl<'de> Deserialize<'de> for ClockRelation {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            source: ClockDomainId,
            target: ClockDomainId,
            measured_at: TimePoint,
            offset: TimePoint,
            drift: f64,
            drift_uncertainty: f64,
            uncertainty: Duration,
            method: String,
            valid: Validity,
        }
        let r = Raw::deserialize(d)?;
        ClockRelation::new(
            r.source,
            r.target,
            r.measured_at,
            r.offset,
            r.drift,
            r.drift_uncertainty,
            r.uncertainty,
            r.method,
            r.valid,
        )
        .map_err(|e| serde::de::Error::custom(format!("TM-14: {e}")))
    }
}

impl ClockRelation {
    /// Builds a relation, refusing one that breaks TM-14's shape, with the field named:
    /// `measured_at` and both `valid` bounds in `source`, `offset` and `uncertainty`
    /// in `target`, `uncertainty` ≥ 0, `drift` finite, `drift_uncertainty` finite and
    /// ≥ 0, and `valid.from` ≤ `valid.to` when `to` is present.
    ///
    /// Rule: TM-14.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source: ClockDomainId,
        target: ClockDomainId,
        measured_at: TimePoint,
        offset: TimePoint,
        drift: f64,
        drift_uncertainty: f64,
        uncertainty: Duration,
        method: String,
        valid: Validity,
    ) -> Result<ClockRelation, TimeError> {
        let bad = |field, why| Err(TimeError::Malformed { field, why });
        let not_source = "is not in the source domain";
        if measured_at.domain != source {
            return bad("measured_at", not_source);
        }
        if valid.from.domain != source {
            return bad("valid.from", not_source);
        }
        if valid.to.is_some_and(|to| to.domain != source) {
            return bad("valid.to", not_source);
        }
        if offset.domain != target {
            return bad("offset", "is not in the target domain");
        }
        if uncertainty.domain != target {
            return bad("uncertainty", "is not in the target domain");
        }
        if uncertainty.ticks < 0 {
            return bad("uncertainty", "is negative");
        }
        if !drift.is_finite() {
            return bad("drift", "is not finite");
        }
        if !drift_uncertainty.is_finite() {
            return bad("drift_uncertainty", "is not finite");
        }
        // `-0.0 < 0.0` is false: a negative zero is zero.
        if drift_uncertainty < 0.0 {
            return bad("drift_uncertainty", "is negative");
        }
        if valid.to.is_some_and(|to| to.ticks < valid.from.ticks) {
            return bad("valid.to", "precedes valid.from");
        }
        Ok(ClockRelation {
            source,
            target,
            measured_at,
            offset,
            drift,
            drift_uncertainty,
            uncertainty,
            method,
            valid,
        })
    }

    /// The domain being converted from (TM-14).
    pub fn source(&self) -> ClockDomainId {
        self.source
    }

    /// The domain being converted to (TM-14).
    pub fn target(&self) -> ClockDomainId {
        self.target
    }

    /// When the measurement was taken, in the source domain (TM-14).
    pub fn measured_at(&self) -> TimePoint {
        self.measured_at
    }

    /// The image of `measured_at` in the target domain (TM-14).
    pub fn offset(&self) -> TimePoint {
        self.offset
    }

    /// Target seconds per source second, minus 1; finite (TM-14).
    pub fn drift(&self) -> f64 {
        self.drift
    }

    /// The one-sided error bound on `drift`; finite and non-negative (TM-14).
    pub fn drift_uncertainty(&self) -> f64 {
        self.drift_uncertainty
    }

    /// The bound at `measured_at`, non-negative, in target ticks (TM-14).
    pub fn uncertainty(&self) -> Duration {
        self.uncertainty
    }

    /// The measurement method's namespaced name (TM-14).
    pub fn method(&self) -> &str {
        &self.method
    }

    /// The window over which the relation may be applied (TM-14).
    pub fn valid(&self) -> Validity {
        self.valid
    }

    /// Converts an instant from the source root to the target root.
    ///
    /// ```text
    /// delta       = t − measured_at                       (source ticks)
    /// delta_t     = delta rescaled to the target by TM-9
    /// drift_ticks = round(delta_t · drift)
    /// nominal     = offset + delta_t + drift_ticks
    /// uncertainty = relation.uncertainty
    ///             + |delta_t| · drift_uncertainty, rounded away from zero
    ///             + 1 tick + (1 tick if the rescale had a remainder)
    /// ```
    ///
    /// The uncertainty must grow with elapsed time, because `drift` is a measurement
    /// with an error: a relation used ten minutes later on an oscillator stable to
    /// 1e-8 carries a true bound near 6 µs, and reporting the measurement-time bound
    /// would overstate the timing claim of Vision §24 and §50 by two orders of
    /// magnitude. The Kernel does not chain relations (decision T13).
    ///
    /// Rule: TM-14.
    pub fn convert(
        &self,
        registry: &ClockRegistry,
        t: TimePoint,
    ) -> Result<UncertainTimePoint, TimeError> {
        // `new` fixed the relation's own shape; only `t` is checked here.
        t.ticks_in(self.source)?;
        if t.try_cmp(self.valid.from)? == std::cmp::Ordering::Less {
            return Err(TimeError::OutsideValidity { at: t });
        }
        if let Some(to) = self.valid.to {
            if t.try_cmp(to)? == std::cmp::Ordering::Greater {
                return Err(TimeError::OutsideValidity { at: t });
            }
        }
        let delta = t.checked_sub(self.measured_at)?;
        let rescaled = registry.rescale(delta, self.target)?;
        let delta_t = rescaled.floor();
        let had_remainder = matches!(rescaled, Rescaled::Inexact { .. });

        let drift_ticks = round_away(delta_t.ticks as f64 * self.drift)?;
        let nominal = self
            .offset
            .checked_add(delta_t)?
            .checked_add(Duration::new(self.target, drift_ticks))?;

        let drift_error = ceil_non_negative(delta_t.ticks.unsigned_abs() as f64 * self.drift_uncertainty)?;
        let ticks = self
            .uncertainty
            .ticks
            .checked_add(drift_error)
            .and_then(|v| v.checked_add(1))
            .and_then(|v| v.checked_add(i64::from(had_remainder)))
            .ok_or(TimeError::Overflow)?;
        Ok(UncertainTimePoint { nominal, uncertainty: Duration::new(self.target, ticks) })
    }
}

/// One past the largest `f64` that converts to an `i64`. `i64::MAX as f64` rounds
/// *up* to 2^63, so a guard written against it lets 2^63 through and the cast then
/// saturates to `i64::MAX` — a silently wrong instant where TM-2 and TM-8 require
/// an error (TM-8).
const I64_LIMIT: f64 = 9_223_372_036_854_775_808.0;

/// `round(x)` with halves away from zero, refusing a non-finite or out-of-range value (TM-14).
fn round_away(x: f64) -> Result<i64, TimeError> {
    let r = x.round();
    if !(r.is_finite() && r >= i64::MIN as f64 && r < I64_LIMIT) {
        return Err(TimeError::Overflow);
    }
    Ok(r as i64)
}

/// `ceil(x)` for a non-negative `x`, refusing a non-finite or out-of-range value (TM-14).
fn ceil_non_negative(x: f64) -> Result<i64, TimeError> {
    if !x.is_finite() || x < 0.0 {
        return Err(TimeError::Overflow);
    }
    let r = x.ceil();
    if r >= I64_LIMIT {
        return Err(TimeError::Overflow);
    }
    Ok(r as i64)
}
