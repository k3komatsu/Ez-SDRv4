//! Kernel time model — `01-time-model.md` (rules `TM-n`).
//!
//! Every Module contract carries a timestamp, and Vision invariant 32 fixes its
//! representation: integer ticks at a rational rate, in a named ClockDomain, never
//! floating-point seconds (TM-1). Two timestamps that cannot be compared exactly
//! cannot be compared at all, by construction rather than by convention (TM-6).
//!
//! Same root → integer arithmetic ([`ExactConversion`], TM-4).
//! Different roots → measurement ([`ClockRelation`], TM-5, TM-14).
//! Anything else → refused.

mod authority;
mod domain;
mod point;
mod rational;
mod relation;

use std::fmt;

pub use authority::{ScheduleHandle, TimeAuthority};
#[cfg(feature = "testing")]
pub use authority::ManualTimeAuthority;
pub use domain::{
    ClockDomain, ClockDomainKind, ClockRegistry, EpochRef, RATIO_TERM_CAP, SampleClockHandle,
    SampleClockRecord,
};
pub use point::{
    AbsoluteDeadline, Converted, Duration, ExactConversion, RelativeBudget, Rescaled, TimePoint,
    UncertainTimePoint,
};
pub use rational::Rational;
pub use relation::{ClockRelation, Validity};

use crate::id::ClockDomainId;

/// Why a time operation was refused. Nothing in this module wraps, saturates or
/// panics: every failure is one of these (TM-2, TM-8).
///
/// Rule: TM-2, TM-4…TM-8, TM-12, TM-14, TM-16.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TimeError {
    /// Two operands named different domains, and the operation is same-domain only (TM-6, TM-7).
    DomainMismatch {
        /// The domain the operation required.
        expected: ClockDomainId,
        /// The domain that was supplied.
        found: ClockDomainId,
    },
    /// The two domains have different roots, so only a `ClockRelation` connects them (TM-5).
    Unrelated {
        /// One domain.
        a: ClockDomainId,
        /// The other.
        b: ClockDomainId,
    },
    /// An exact conversion was demanded and the instant is not a tick of the target (TM-4).
    Inexact {
        /// The largest target tick at or before the true instant.
        floor: TimePoint,
    },
    /// A checked result left the representable range (TM-8).
    Overflow,
    /// The domain is not registered (TM-12).
    UnknownDomain {
        /// The id that was not found.
        id: ClockDomainId,
    },
    /// The id is already registered; domains are immutable (TM-12).
    DuplicateDomain {
        /// The id that collided.
        id: ClockDomainId,
    },
    /// No timekeeper in this Run advances that domain. Not a claim about
    /// relatedness: governance is declared, not inferred (TM-16b).
    NotGoverned {
        /// The domain no Authority advances.
        id: ClockDomainId,
    },
    /// The instant lies outside the relation's validity window (TM-14).
    OutsideValidity {
        /// The instant that was asked for.
        at: TimePoint,
    },
    /// A rational had a zero component (TM-2).
    InvalidRational,
    /// A declared value exceeded a cap the model depends on, such as
    /// [`RATIO_TERM_CAP`] or TM-17b's callback cap (TM-3, TM-17b).
    LimitExceeded,
    /// The instant has already passed (TM-16c, TM-17a).
    InPast {
        /// The Authority's current instant.
        now: TimePoint,
        /// The instant that was asked for.
        requested: TimePoint,
    },
    /// The domain or Authority has already stopped (TM-12, TM-13c).
    Stopped,
}

impl fmt::Display for TimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TimeError::DomainMismatch { expected, found } => {
                write!(f, "time domain mismatch: expected {expected}, found {found}")
            }
            TimeError::Unrelated { a, b } => write!(f, "{a} and {b} have different roots"),
            TimeError::Inexact { floor } => write!(f, "conversion is inexact; floor is {floor}"),
            TimeError::Overflow => f.write_str("tick arithmetic overflowed"),
            TimeError::UnknownDomain { id } => write!(f, "clock domain {id} is not registered"),
            TimeError::DuplicateDomain { id } => write!(f, "clock domain {id} is already registered"),
            TimeError::NotGoverned { id } => write!(f, "no timekeeper advances {id}"),
            TimeError::OutsideValidity { at } => write!(f, "{at} is outside the relation's validity"),
            TimeError::InvalidRational => f.write_str("a rational had a zero component"),
            TimeError::LimitExceeded => f.write_str("a declared limit was exceeded"),
            TimeError::InPast { now, requested } => write!(f, "{requested} precedes now ({now})"),
            TimeError::Stopped => f.write_str("the clock domain or authority has stopped"),
        }
    }
}

impl std::error::Error for TimeError {}

/// Builds a reduced [`Rational`] from two 128-bit terms, for remainders (TM-2, TM-4).
fn rational_from_u128(num: u128, den: u128) -> Result<Rational, TimeError> {
    let g = {
        let (mut a, mut b) = (num, den);
        while b != 0 {
            let t = a % b;
            a = b;
            b = t;
        }
        a
    };
    match (u64::try_from(num / g), u64::try_from(den / g)) {
        (Ok(n), Ok(d)) => Rational::new(n, d),
        _ => Err(TimeError::Overflow),
    }
}
