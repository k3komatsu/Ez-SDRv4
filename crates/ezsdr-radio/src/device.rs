//! Shared grids and coercion helpers used by the Radio Model layout description.

use ezsdr_kernel::module_api::RejectedRequest;
use ezsdr_kernel::spec::{Coercion, Constraint, Key, Scalar, Value};

/// A configuration key's admissible values (RM-8).
#[derive(Clone, PartialEq, Debug)]
pub enum Grid {
    /// An ascending list of values.
    Values(Vec<f64>),
    /// Every integer in `lo..=hi`.
    Integer {
        /// The least value.
        lo: i64,
        /// The greatest value.
        hi: i64,
    },
    /// Multiples of `step` in `lo..=hi`; a step of 0 means no grid.
    Step {
        /// The least value.
        lo: f64,
        /// The greatest value.
        hi: f64,
        /// The grid spacing, 0 for none.
        step: f64,
    },
}

impl Grid {
    /// The least value (RM-8).
    pub fn min(&self) -> f64 {
        match self {
            Grid::Values(values) => values[0],
            Grid::Integer { lo, .. } => *lo as f64,
            Grid::Step { lo, .. } => *lo,
        }
    }

    /// The greatest value (RM-8).
    pub fn max(&self) -> f64 {
        match self {
            Grid::Values(values) => *values.last().expect("nonempty grid"),
            Grid::Integer { hi, .. } => *hi as f64,
            Grid::Step { hi, .. } => *hi,
        }
    }

    /// The grid spacing of a `Step` grid, 0 otherwise (RM-4's step keys).
    pub fn step(&self) -> f64 {
        match self {
            Grid::Step { step, .. } => *step,
            Grid::Values(_) | Grid::Integer { .. } => 0.0,
        }
    }

    /// Whether `value` is on the grid (RM-8).
    pub fn contains(&self, value: f64) -> bool {
        if !value.is_finite() || value < self.min() || value > self.max() {
            return false;
        }
        match self {
            Grid::Values(values) => values.contains(&value),
            Grid::Integer { .. } => value.fract() == 0.0,
            Grid::Step { step: 0.0, .. } => true,
            Grid::Step { step, .. } => {
                let units = value / step;
                units.is_finite() && units.fract() == 0.0
            }
        }
    }

    /// The nearest grid value, ties to the lower (RM-8).
    pub fn nearest(&self, value: f64) -> Option<f64> {
        if !value.is_finite() || value < self.min() || value > self.max() {
            return None;
        }
        match self {
            Grid::Values(values) => values.iter().copied().min_by(|a, b| {
                (a - value)
                    .abs()
                    .total_cmp(&(b - value).abs())
                    .then_with(|| a.total_cmp(b))
            }),
            Grid::Integer { lo, hi } => {
                (value.fract() == 0.0 && value >= *lo as f64 && value <= *hi as f64)
                    .then_some(value)
            }
            Grid::Step { step: 0.0, .. } => Some(value),
            Grid::Step { lo, hi, step } => {
                let f = (value / step).floor();
                let r = value / step - f;
                let rounded = if r > 0.5 { f + 1.0 } else { f };
                let grid_value = rounded * step;
                (grid_value >= *lo && grid_value <= *hi).then_some(grid_value)
            }
        }
    }

    /// The least grid value at or above `bound` (RM-8).
    pub fn at_or_above(&self, bound: f64) -> Option<f64> {
        if !bound.is_finite() {
            return None;
        }
        match self {
            Grid::Values(values) => values.iter().copied().find(|v| *v >= bound),
            Grid::Integer { lo, hi } => {
                if bound > *hi as f64 {
                    None
                } else {
                    Some(bound.ceil().max(*lo as f64))
                }
            }
            Grid::Step { lo, hi, step: 0.0 } => {
                let value = bound.max(*lo);
                (value <= *hi).then_some(value)
            }
            Grid::Step { lo, hi, step } => {
                let value = (bound.max(*lo) / step).ceil() * step;
                (value <= *hi).then_some(value)
            }
        }
    }

    /// The greatest grid value at or below `bound` (RM-8).
    pub fn at_or_below(&self, bound: f64) -> Option<f64> {
        if !bound.is_finite() {
            return None;
        }
        match self {
            Grid::Values(values) => values.iter().rev().copied().find(|v| *v <= bound),
            Grid::Integer { lo, hi } => {
                if bound < *lo as f64 {
                    None
                } else {
                    Some(bound.floor().min(*hi as f64))
                }
            }
            Grid::Step { lo, hi, step: 0.0 } => {
                let value = bound.min(*hi);
                (value >= *lo).then_some(value)
            }
            Grid::Step { lo, hi, step } => {
                let value = (bound.min(*hi) / step).floor() * step;
                (value >= *lo).then_some(value)
            }
        }
    }
}

pub(crate) fn rejected(key: &Key, requested: &Constraint, reason: String) -> RejectedRequest {
    RejectedRequest {
        key: key.clone(),
        requested: requested.clone(),
        reason,
    }
}

fn number(value: &Scalar) -> Option<f64> {
    match value {
        Scalar::Int(value) => Some(*value as f64),
        Scalar::Num(value) => Some(value.get()),
        _ => None,
    }
}

/// A profile's number, which a JSON document cannot make non-finite.
pub(crate) fn num(x: f64) -> Scalar {
    Scalar::try_from(x).expect("a profile number is finite")
}

pub(crate) fn coerce_numeric(
    key: &Key,
    constraint: &Constraint,
    grid: &Grid,
    default: Option<&Value>,
    whole_hertz: bool,
) -> Result<(Value, Option<Coercion>), String> {
    let requested_eq = match constraint {
        Constraint::Eq { value } => Some(value),
        _ => None,
    };
    let value = match constraint {
        Constraint::Eq { value } => {
            let Some(value) = number(value) else {
                return Err("RM-4: numeric configuration needs a number".to_owned());
            };
            if whole_hertz && value.fract() != 0.0 {
                return Err("RM-8: this device represents a rate as whole hertz".to_owned());
            }
            grid.nearest(value)
                .ok_or_else(|| format!("RM-8: {value} is outside {}..{}", grid.min(), grid.max()))?
        }
        Constraint::Min { value } => {
            let value = number(value).ok_or_else(|| "RM-4: expected a number".to_owned())?;
            grid.at_or_above(value)
                .ok_or_else(|| format!("RM-8: no grid value satisfies {constraint:?}"))?
        }
        Constraint::Max { value } => {
            let value = number(value).ok_or_else(|| "RM-4: expected a number".to_owned())?;
            grid.at_or_below(value)
                .ok_or_else(|| format!("RM-8: no grid value satisfies {constraint:?}"))?
        }
        Constraint::Range { min, max } => {
            let min = bound(min.as_ref(), grid.min())?;
            let max = bound(max.as_ref(), grid.max())?;
            grid.at_or_above(min)
                .filter(|value| *value <= max)
                .ok_or_else(|| format!("RM-8: no grid value satisfies {constraint:?}"))?
        }
        Constraint::Set { values } => values
            .iter()
            .filter_map(number)
            .find(|value| grid.contains(*value))
            .ok_or_else(|| format!("RM-8: no grid value satisfies {constraint:?}"))?,
        Constraint::Present {} => default.ok_or_else(|| "RM-5: no default".to_owned())?
            .as_scalar().and_then(number)
            .ok_or_else(|| "RM-5: numeric default is invalid".to_owned())?,
    };
    let applied = Value::from(num(value));
    let coercion = requested_eq.and_then(|requested| {
        number(requested)
            .filter(|requested| *requested != value)
            .map(|_| Coercion {
                key: key.clone(),
                requested: Value::from(requested.clone()),
                applied: applied.clone(),
                reason: "RM-8: nearest grid value".to_owned(),
            })
    });
    Ok((applied, coercion))
}

pub(crate) fn coerce_antenna(
    constraint: &Constraint,
    allowed: &[String],
    default: Option<&Value>,
) -> Result<(Value, Option<Coercion>), String> {
    let known = |name: &str| allowed.iter().any(|allowed| allowed == name);
    match constraint {
        Constraint::Eq { value: Scalar::Str(value) } if known(value) => {
            Ok((Value::from(value.as_str()), None))
        }
        Constraint::Set { values } => values
            .iter()
            .find_map(|value| match value {
                Scalar::Str(value) if known(value) => Some(Value::from(value.as_str())),
                _ => None,
            })
            .map(|value| (value, None))
            .ok_or_else(|| "RM-8: no requested antenna is available".to_owned()),
        Constraint::Present {} => Ok((default.cloned().ok_or("RM-5: no default")?, None)),
        Constraint::Eq { value: Scalar::Str(value) } => {
            Err(format!("RM-8: {value} is not an antenna of this device"))
        }
        _ => Err("RM-4: antenna must be a supported string".to_owned()),
    }
}

/// UR-13 (design 33 D5): whether a device's `rates` — UHD's `(start, stop, step)` ranges at
/// one channel — are the `grid`'s rates, as a *set*: every value in exactly one of them,
/// whatever order the device reports. A `Step` grid expands like a range; a grid this
/// cannot enumerate (`Integer`, or a `Step` with step 0, an interval) is never equal to a
/// device's finite answer. The one place the comparison is read (BP0 measures the real
/// ranges; the fake reports singletons).
pub fn rate_set_matches(grid: &Grid, ranges: &[(f64, f64, f64)]) -> bool {
    let expand = |first: f64, last: f64, step: f64| -> Vec<f64> {
        let mut values = Vec::new();
        if !first.is_finite() || !last.is_finite() {
            return values;
        }
        if !step.is_finite() || step <= 0.0 || first >= last {
            values.push(first);
            return values;
        }
        let count = ((last - first) / step).floor();
        if count > 10_000.0 {
            return values;
        }
        for i in 0..=count as i64 {
            values.push(first + i as f64 * step);
        }
        values
    };
    let mut expected = match grid {
        Grid::Values(values) => values.clone(),
        Grid::Step { lo, hi, step } if *step > 0.0 => expand(*lo, *hi, *step),
        _ => return false,
    };
    let mut reported: Vec<f64> = ranges.iter().flat_map(|(start, stop, step)| expand(*start, *stop, *step)).collect();
    expected.retain(|value| value.is_finite());
    reported.retain(|value| value.is_finite());
    let order = |a: &f64, b: &f64| a.total_cmp(b);
    expected.sort_by(order);
    reported.sort_by(order);
    expected.dedup();
    reported.dedup();
    expected == reported
}

fn bound(value: Option<&Scalar>, default: f64) -> Result<f64, String> {
    match value {
        None => Ok(default),
        Some(value) => number(value).ok_or_else(|| "RM-4: numeric bound needs a number".to_owned()),
    }
}
