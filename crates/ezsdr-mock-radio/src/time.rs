//! Integer arithmetic between the Mock's virtual root and SampleClocks (MR-7).

use ezsdr_kernel::id::ClockDomainId;
use ezsdr_kernel::time::{
    ClockRegistry, Converted, Duration, Rescaled, TimeError, TimePoint,
};

fn ceil_div(a: i128, b: i128) -> Option<i128> {
    if b <= 0 {
        return None;
    }
    let floor = a.div_euclid(b);
    floor.checked_add(i128::from(a.rem_euclid(b) != 0))
}

/// Returns the virtual-root tick for sample `k`, rounded up (MR-7, TM-20).
pub(crate) fn v_of(o: i64, q: ezsdr_kernel::time::Rational, k: i64) -> Option<i64> {
    let scaled = i128::from(k).checked_mul(i128::from(q.num()))?;
    let offset = ceil_div(scaled, i128::from(q.den()))?;
    i64::try_from(i128::from(o).checked_add(offset)?).ok()
}

/// Returns the first virtual-root tick strictly after sample `k`'s instant:
/// `floor(o + k · q) + 1`, the earliest round at which sample `k` is in the past (MR-14).
pub(crate) fn v_after(o: i64, q: ezsdr_kernel::time::Rational, k: i64) -> Option<i64> {
    let scaled = i128::from(k).checked_mul(i128::from(q.num()))?;
    let offset = scaled.div_euclid(i128::from(q.den()));
    i64::try_from(i128::from(o).checked_add(offset)?.checked_add(1)?).ok()
}

/// Returns the first nonnegative sample index at or after virtual tick `t` (MR-7).
pub(crate) fn k_at_or_after(
    o: i64,
    q: ezsdr_kernel::time::Rational,
    t: i64,
) -> Option<i64> {
    let delta = i128::from(t).checked_sub(i128::from(o))?;
    let scaled = delta.checked_mul(i128::from(q.den()))?;
    let k = ceil_div(scaled, i128::from(q.num()))?.max(0);
    i64::try_from(k).ok()
}

/// Converts nanoseconds in `host.monotonic` to virtual ticks, rounding up (MR-7).
pub(crate) fn ns_to_v(
    clocks: &ClockRegistry,
    v: ClockDomainId,
    ns: i64,
) -> Result<i64, TimeError> {
    match clocks.rescale(Duration::new(ClockDomainId::HOST_MONOTONIC, ns), v)? {
        Rescaled::Exact { duration } => Ok(duration.ticks),
        Rescaled::Inexact { floor, .. } => floor.ticks.checked_add(1).ok_or(TimeError::Overflow),
    }
}

/// Converts an instant to virtual-root ticks, rounding an inexact result up (MR-7).
pub(crate) fn to_v(
    clocks: &ClockRegistry,
    v: ClockDomainId,
    time: TimePoint,
) -> Result<i64, TimeError> {
    if time.domain == v {
        return Ok(time.ticks);
    }
    match clocks.convert(time, v)? {
        Converted::Exact { point } => Ok(point.ticks),
        Converted::Inexact { floor, .. } => floor.ticks.checked_add(1).ok_or(TimeError::Overflow),
    }
}

/// Converts a duration to host nanoseconds, rounding an inexact result up (MR-17).
pub(crate) fn to_ns(clocks: &ClockRegistry, duration: Duration) -> Result<i64, TimeError> {
    match clocks.rescale(duration, ClockDomainId::HOST_MONOTONIC)? {
        Rescaled::Exact { duration } => Ok(duration.ticks),
        Rescaled::Inexact { floor, .. } => floor.ticks.checked_add(1).ok_or(TimeError::Overflow),
    }
}

#[cfg(test)]
mod tests {
    use super::{k_at_or_after, v_of};
    use ezsdr_kernel::time::Rational;

    #[test]
    fn mr_14_a_block_is_published_strictly_after_its_last_sample() {
        use super::v_after;
        assert_eq!(v_after(0, Rational::new(5, 1).unwrap(), 3), Some(16));
        assert_eq!(v_after(10, Rational::new(1_000, 3).unwrap(), 1), Some(344));
        assert_eq!(v_after(10, Rational::new(1_000, 3).unwrap(), 3), Some(1_011));
    }

    #[test]
    fn mr_14_tick_arithmetic_rounds_up() {
        assert_eq!(v_of(0, Rational::new(5, 1).unwrap(), 3), Some(15));
        assert_eq!(v_of(10, Rational::new(1_000, 3).unwrap(), 1), Some(344));
        assert_eq!(
            k_at_or_after(0, Rational::new(1_000, 3).unwrap(), 334),
            Some(2)
        );
        assert_eq!(
            k_at_or_after(100, Rational::new(5, 1).unwrap(), 0),
            Some(0)
        );
    }
}
