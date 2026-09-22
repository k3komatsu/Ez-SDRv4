//! Exact rational tick rates (`01-time-model.md` TM-2, decision T1).

use std::cmp::Ordering;
use std::fmt;

use serde::{Deserialize, Serialize};

use super::TimeError;

/// A positive rational, stored reduced by the greatest common divisor.
///
/// `num > 0`, `den > 0`, `gcd(num, den) = 1`, so equality is structural. Every
/// operation evaluates with 128-bit intermediates and returns an error rather than
/// wrapping, saturating or panicking.
///
/// Rule: TM-2.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Rational {
    num: u64,
    den: u64,
}

/// Greatest common divisor, binary-free Euclid; `gcd(x, 0) = x`.
fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

/// Reduces a 128-bit fraction and narrows it to two `u64`s, or fails with `Overflow` (TM-2).
fn reduce(num: u128, den: u128) -> Result<Rational, TimeError> {
    if num == 0 || den == 0 {
        return Err(TimeError::InvalidRational);
    }
    let g = gcd(num, den);
    let (num, den) = (num / g, den / g);
    match (u64::try_from(num), u64::try_from(den)) {
        (Ok(num), Ok(den)) => Ok(Rational { num, den }),
        _ => Err(TimeError::Overflow),
    }
}

impl Rational {
    /// One.
    pub const ONE: Rational = Rational { num: 1, den: 1 };

    /// Builds a reduced rational. A zero component fails with `InvalidRational` (TM-2).
    pub fn new(num: u64, den: u64) -> Result<Rational, TimeError> {
        reduce(num as u128, den as u128)
    }

    /// The whole number `n` as `n/1` (TM-2).
    pub fn integer(n: u64) -> Result<Rational, TimeError> {
        Rational::new(n, 1)
    }

    /// The reduced numerator (TM-2).
    pub fn num(self) -> u64 {
        self.num
    }

    /// The reduced denominator (TM-2).
    pub fn den(self) -> u64 {
        self.den
    }

    /// `1/self`; never fails, because both components are positive (TM-2).
    pub fn recip(self) -> Rational {
        Rational { num: self.den, den: self.num }
    }

    /// Product, reduced; `Overflow` when the reduced result exceeds 64 bits (TM-2).
    pub fn checked_mul(self, other: Rational) -> Result<Rational, TimeError> {
        reduce(self.num as u128 * other.num as u128, self.den as u128 * other.den as u128)
    }

    /// Quotient, reduced; `Overflow` when the reduced result exceeds 64 bits (TM-2).
    pub fn checked_div(self, other: Rational) -> Result<Rational, TimeError> {
        self.checked_mul(other.recip())
    }

    /// True when either component exceeds `cap`; TM-3 uses it with 2^31.
    pub fn exceeds(self, cap: u64) -> bool {
        self.num > cap || self.den > cap
    }
}

/// Ordering by cross-multiplication in 128 bits, which cannot overflow for two
/// `u64` pairs and so never rounds (TM-2).
impl Ord for Rational {
    fn cmp(&self, other: &Rational) -> Ordering {
        (self.num as u128 * other.den as u128).cmp(&(other.num as u128 * self.den as u128))
    }
}

impl PartialOrd for Rational {
    fn partial_cmp(&self, other: &Rational) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Rational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.num, self.den)
    }
}
