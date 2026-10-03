//! Cross-root conversion by measurement (`01-time-model.md` TM-5, TM-14, TM-18).

use serde::{Deserialize, Serialize};

use super::{ClockRegistry, Duration, Rescaled, TimeError, TimePoint, UncertainTimePoint};
use crate::id::ClockDomainId;

/// The window over which a [`ClockRelation`] may be applied (TM-14).
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
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClockRelation {
    /// The domain being converted from.
    pub source: ClockDomainId,
    /// The domain being converted to.
    pub target: ClockDomainId,
    /// When the measurement was taken, in the source domain.
    pub measured_at: TimePoint,
    /// The image of `measured_at` in the target domain.
    pub offset: TimePoint,
    /// Dimensionless: target seconds per source second, minus 1. Refused when
    /// non-finite, because `serde_json` would otherwise write it as `null` and a
    /// NaN drift would share a Manifest hash with one that was never measured
    /// (OV-15, X9).
    #[serde(serialize_with = "crate::hash::serialize_finite_f64")]
    pub drift: f64,
    /// Dimensionless, non-negative: the one-sided error bound on `drift`. Zero
    /// claims the drift is known exactly and is expected to be rare. Refused when
    /// non-finite (OV-15, X9).
    #[serde(serialize_with = "crate::hash::serialize_finite_f64")]
    pub drift_uncertainty: f64,
    /// The bound at `measured_at`, in target ticks, before drift error accumulates.
    pub uncertainty: Duration,
    /// Namespaced name of the measurement method, for example `dev.pps_poll`.
    pub method: String,
    /// The window over which the relation may be applied.
    pub valid: Validity,
}

impl ClockRelation {
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
        t.ticks_in(self.source)?;
        let measurement_bound = self.uncertainty.ticks_in(self.target)?;
        // Like a negative drift error, a negative measurement error cannot be a
        // one-sided bound. Reject before accumulated error could hide its sign.
        if measurement_bound < 0 {
            return Err(TimeError::Overflow);
        }
        // Validate before multiplication: a negative bound times zero produces
        // negative zero, which the accumulated-error check accepts as zero.
        if !self.drift_uncertainty.is_finite() || self.drift_uncertainty < 0.0 {
            return Err(TimeError::Overflow);
        }
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
        let ticks = measurement_bound
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
