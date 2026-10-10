//! `TimePoint`, `Duration`, the two deadline kinds, and exact conversion
//! (`01-time-model.md` TM-1, TM-4, TM-6…TM-9, TM-15).

use std::cmp::Ordering;
use std::fmt;

use serde::{Deserialize, Serialize};

use super::{Rational, TimeError};
use crate::id::ClockDomainId;

/// An instant: integer ticks in a named clock domain. Never floating-point seconds.
///
/// There is deliberately no `PartialOrd`: two points of different domains cannot be
/// compared at all, and `a < b` silently answering `false` is the bug class this
/// model exists to prevent (decision T3).
///
/// Rule: TM-1, TM-6.
// The fields are private: outside `time/` the integer is read only through
// `ticks_in`, which names the domain the reader expects (TM-6, T3).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TimePoint {
    /// The domain whose ticks `ticks` counts (TM-1).
    pub(super) domain: ClockDomainId,
    /// Tick count relative to the domain's origin; a sample index in a SampleClock (TM-10).
    pub(super) ticks: i64,
}

/// An elapsed time: integer ticks in a named clock domain (TM-1, TM-4 decision T4).
///
/// The domain tag is what stops 1 000 ticks at 20 Msps being added to a 200 MHz
/// point as a silent factor-of-ten error.
// Private fields, read like a `TimePoint`'s (TM-6).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Duration {
    /// The domain whose ticks `ticks` counts (TM-7).
    pub(super) domain: ClockDomainId,
    /// Tick count; may be negative (TM-7).
    pub(super) ticks: i64,
}

/// The result of converting a [`TimePoint`] between two exactly related domains (TM-4).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Converted {
    /// The instant is a tick of the target domain.
    Exact {
        /// The converted instant.
        point: TimePoint,
    },
    /// The instant lies strictly between two ticks of the target domain.
    Inexact {
        /// The largest target tick at or before the true instant.
        floor: TimePoint,
        /// The leftover fraction of one target tick; reduced, with `0 < num < den`.
        remainder: Rational,
    },
}

impl Converted {
    /// The converted point, discarding any remainder (TM-4).
    pub fn floor(self) -> TimePoint {
        match self {
            Converted::Exact { point } => point,
            Converted::Inexact { floor, .. } => floor,
        }
    }

    /// The remainder, or `None` when the conversion was exact (TM-4).
    pub fn remainder(self) -> Option<Rational> {
        match self {
            Converted::Exact { .. } => None,
            Converted::Inexact { remainder, .. } => Some(remainder),
        }
    }
}

/// The result of rescaling a [`Duration`] nominally between two domains (TM-9).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rescaled {
    /// The duration is a whole number of target ticks.
    Exact {
        /// The rescaled duration.
        duration: Duration,
    },
    /// The duration lies strictly between two whole numbers of target ticks.
    Inexact {
        /// The largest whole target duration at or before the true value.
        floor: Duration,
        /// The leftover fraction of one target tick; reduced, with `0 < num < den`.
        remainder: Rational,
    },
}

impl Rescaled {
    /// The rescaled duration, discarding any remainder (TM-9).
    pub fn floor(self) -> Duration {
        match self {
            Rescaled::Exact { duration } => duration,
            Rescaled::Inexact { floor, .. } => floor,
        }
    }

    /// The remainder, or `None` when the rescale was exact (TM-9).
    pub fn remainder(self) -> Option<Rational> {
        match self {
            Rescaled::Exact { .. } => None,
            Rescaled::Inexact { remainder, .. } => Some(remainder),
        }
    }
}

impl TimePoint {
    /// An instant in `domain` (TM-1).
    pub const fn new(domain: ClockDomainId, ticks: i64) -> TimePoint {
        TimePoint { domain, ticks }
    }

    /// The domain whose ticks this point counts (TM-1).
    pub const fn domain(self) -> ClockDomainId {
        self.domain
    }

    /// Orders two instants of one domain; `DomainMismatch` across domains (TM-6).
    pub fn try_cmp(self, other: TimePoint) -> Result<Ordering, TimeError> {
        self.same_domain(other)?;
        Ok(self.ticks.cmp(&other.ticks))
    }

    /// Adds a duration of the same domain, checked (TM-7, TM-8).
    pub fn checked_add(self, d: Duration) -> Result<TimePoint, TimeError> {
        if self.domain != d.domain {
            return Err(TimeError::DomainMismatch { expected: self.domain, found: d.domain });
        }
        self.ticks
            .checked_add(d.ticks)
            .map(|ticks| TimePoint { domain: self.domain, ticks })
            .ok_or(TimeError::Overflow)
    }

    /// Subtracts a duration of the same domain, checked (TM-7, TM-8).
    pub fn checked_sub_duration(self, d: Duration) -> Result<TimePoint, TimeError> {
        if self.domain != d.domain {
            return Err(TimeError::DomainMismatch { expected: self.domain, found: d.domain });
        }
        self.ticks
            .checked_sub(d.ticks)
            .map(|ticks| TimePoint { domain: self.domain, ticks })
            .ok_or(TimeError::Overflow)
    }

    /// The elapsed time from `other` to `self`; `DomainMismatch` across domains (TM-6, TM-7).
    pub fn checked_sub(self, other: TimePoint) -> Result<Duration, TimeError> {
        self.same_domain(other)?;
        self.ticks
            .checked_sub(other.ticks)
            .map(|ticks| Duration { domain: self.domain, ticks })
            .ok_or(TimeError::Overflow)
    }

    /// The raw tick count, after one domain check. The only access to the integer,
    /// so the check happens at ingress and the hot loop is raw `i64` (TM-6, T3).
    pub fn ticks_in(self, domain: ClockDomainId) -> Result<i64, TimeError> {
        if self.domain == domain {
            Ok(self.ticks)
        } else {
            Err(TimeError::DomainMismatch { expected: domain, found: self.domain })
        }
    }

    fn same_domain(self, other: TimePoint) -> Result<(), TimeError> {
        if self.domain == other.domain {
            Ok(())
        } else {
            Err(TimeError::DomainMismatch { expected: self.domain, found: other.domain })
        }
    }
}

impl Duration {
    /// An elapsed time in `domain` (TM-7).
    pub const fn new(domain: ClockDomainId, ticks: i64) -> Duration {
        Duration { domain, ticks }
    }

    /// The domain whose ticks this duration counts (TM-7).
    pub const fn domain(self) -> ClockDomainId {
        self.domain
    }

    /// Orders two durations of one domain; across domains use
    /// [`ClockRegistry::compare_durations`](crate::time::ClockRegistry::compare_durations),
    /// which is exact (TM-21).
    pub fn try_cmp(self, other: Duration) -> Result<Ordering, TimeError> {
        if self.domain != other.domain {
            return Err(TimeError::DomainMismatch { expected: self.domain, found: other.domain });
        }
        Ok(self.ticks.cmp(&other.ticks))
    }

    /// Sum of two durations of one domain, checked (TM-7, TM-8).
    pub fn checked_add(self, other: Duration) -> Result<Duration, TimeError> {
        if self.domain != other.domain {
            return Err(TimeError::DomainMismatch { expected: self.domain, found: other.domain });
        }
        self.ticks
            .checked_add(other.ticks)
            .map(|ticks| Duration { domain: self.domain, ticks })
            .ok_or(TimeError::Overflow)
    }

    /// The raw tick count, after one domain check (TM-6).
    pub fn ticks_in(self, domain: ClockDomainId) -> Result<i64, TimeError> {
        if self.domain == domain {
            Ok(self.ticks)
        } else {
            Err(TimeError::DomainMismatch { expected: domain, found: self.domain })
        }
    }
}

/// An instant known only to within a bound, the only result of crossing roots (TM-5, TM-14).
/// Built only by [`ClockRelation::convert`](super::ClockRelation::convert).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct UncertainTimePoint {
    pub(super) nominal: TimePoint,
    pub(super) uncertainty: Duration,
}

impl UncertainTimePoint {
    /// The best estimate of the instant (TM-14).
    pub fn nominal(self) -> TimePoint {
        self.nominal
    }

    /// The one-sided bound, non-negative and in the domain of `nominal` (TM-14).
    pub fn uncertainty(self) -> Duration {
        self.uncertainty
    }
}

/// A processing budget measured from a work item's arrival, in `host.monotonic`.
///
/// Distinct from `AbsoluteDeadline` with no common supertype: the checks that
/// consume them differ (Vision §19, decision T5).
///
/// Rule: TM-15.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RelativeBudget {
    duration: Duration,
}

impl<'de> Deserialize<'de> for RelativeBudget {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Parts {
            duration: Duration,
        }
        let parts = Parts::deserialize(deserializer)?;
        RelativeBudget::new(parts.duration).map_err(serde::de::Error::custom)
    }
}

impl RelativeBudget {
    /// Builds a budget; the duration must be positive and in `host.monotonic`, the
    /// domain in which an Executor measures elapsed processing time (TM-15).
    pub fn new(duration: Duration) -> Result<RelativeBudget, TimeError> {
        if duration.domain != ClockDomainId::HOST_MONOTONIC {
            return Err(TimeError::DomainMismatch {
                expected: ClockDomainId::HOST_MONOTONIC,
                found: duration.domain,
            });
        }
        if duration.ticks <= 0 {
            return Err(TimeError::Malformed { field: "duration", why: "is not positive" });
        }
        Ok(RelativeBudget { duration })
    }

    /// The budget itself (TM-15).
    pub fn duration(self) -> Duration {
        self.duration
    }

    /// Anchors the budget at a `host.monotonic` arrival instant (TM-15).
    pub fn deadline_from(self, arrival: TimePoint) -> Result<AbsoluteDeadline, TimeError> {
        if arrival.domain != ClockDomainId::HOST_MONOTONIC {
            return Err(TimeError::DomainMismatch {
                expected: ClockDomainId::HOST_MONOTONIC,
                found: arrival.domain,
            });
        }
        Ok(AbsoluteDeadline { time_point: arrival.checked_add(self.duration)? })
    }
}

/// A wall-time instant by which something must happen, in any domain (TM-15, Vision §19).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AbsoluteDeadline {
    /// The instant itself.
    pub time_point: TimePoint,
}

impl AbsoluteDeadline {
    /// A deadline at `time_point` (TM-15).
    pub const fn new(time_point: TimePoint) -> AbsoluteDeadline {
        AbsoluteDeadline { time_point }
    }

    /// Time left, as a same-domain checked subtraction; negative when already missed (TM-15).
    pub fn remaining(self, now: TimePoint) -> Result<Duration, TimeError> {
        self.time_point.checked_sub(now)
    }
}

/// A conversion between two exactly related domains, with its three integer terms
/// precomputed so that every later application is provably within 128 bits (TM-4, T7).
///
/// `t_to = (a + t_from · n) / d` where
/// `a = (o_from − o_to) · d_from · d_to`, `n = n_from · d_to`, `d = d_from · n_to`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ExactConversion {
    from: ClockDomainId,
    to: ClockDomainId,
    a: i128,
    n: u128,
    d: u128,
}

impl ExactConversion {
    /// Builds the conversion from the two domains' root ratios and origins.
    ///
    /// `(n_from, d_from, o_from)` describe the source: a `Root` has `n = d = 1`
    /// and `o = 0`. Registration (TM-3) is what bounds these terms.
    ///
    /// Rule: TM-4.
    pub(crate) fn build(
        from: ClockDomainId,
        to: ClockDomainId,
        (n_from, d_from, o_from): (u64, u64, i64),
        (n_to, d_to, o_to): (u64, u64, i64),
    ) -> Result<ExactConversion, TimeError> {
        let origin_delta = (o_from as i128).checked_sub(o_to as i128).ok_or(TimeError::Overflow)?;
        let a = origin_delta
            .checked_mul(d_from as i128)
            .and_then(|v| v.checked_mul(d_to as i128))
            .ok_or(TimeError::Overflow)?;
        let n = (n_from as u128).checked_mul(d_to as u128).ok_or(TimeError::Overflow)?;
        let d = (d_from as u128).checked_mul(n_to as u128).ok_or(TimeError::Overflow)?;
        Ok(ExactConversion { from, to, a, n, d })
    }

    /// The source domain (TM-4).
    pub fn from(&self) -> ClockDomainId {
        self.from
    }

    /// The target domain (TM-4).
    pub fn to(&self) -> ClockDomainId {
        self.to
    }

    /// Converts an instant, reporting exactness. Every step is checked; a result
    /// outside `i64` fails with `Overflow` (TM-4, TM-8).
    pub fn apply(&self, t: TimePoint) -> Result<Converted, TimeError> {
        let ticks = t.ticks_in(self.from)?;
        let numerator = (ticks as i128)
            .checked_mul(self.n as i128)
            .and_then(|v| v.checked_add(self.a))
            .ok_or(TimeError::Overflow)?;
        let d = self.d as i128;
        // Euclidean division: the floor is the largest tick at or before the true
        // instant for negative values too, and the remainder lands in [0, d).
        let floor_ticks = numerator.div_euclid(d);
        let rem = numerator.rem_euclid(d);
        let floor_ticks = i64::try_from(floor_ticks).map_err(|_| TimeError::Overflow)?;
        let floor = TimePoint { domain: self.to, ticks: floor_ticks };
        if rem == 0 {
            Ok(Converted::Exact { point: floor })
        } else {
            let remainder = super::rational_from_u128(rem as u128, self.d)?;
            Ok(Converted::Inexact { floor, remainder })
        }
    }

    /// Converts an instant, refusing an inexact result (TM-4).
    pub fn try_exact(&self, t: TimePoint) -> Result<TimePoint, TimeError> {
        match self.apply(t)? {
            Converted::Exact { point } => Ok(point),
            Converted::Inexact { floor, .. } => Err(TimeError::Inexact { floor }),
        }
    }
}

impl fmt::Display for TimePoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.ticks, self.domain)
    }
}

impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}t@{}", self.ticks, self.domain)
    }
}
