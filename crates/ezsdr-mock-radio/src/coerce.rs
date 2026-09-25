//! Mock-specific grid matching and RM-7 performance enforcement (MR-6, RM-8).

use std::collections::BTreeMap;

use ezsdr_kernel::binding::satisfies;
use ezsdr_kernel::module_api::{CoerceReport, ModuleError, RejectedRequest, Requested};
use ezsdr_kernel::spec::{Coercion, Constraint, Key, Value};

use crate::profile::{Profile, ProfileKind};

pub(crate) enum Grid {
    Values(Vec<f64>),
    Integer { lo: i64, hi: i64 },
    Step { lo: f64, hi: f64, step: f64 },
}

impl Grid {
    pub(crate) fn min(&self) -> f64 {
        match self {
            Grid::Values(values) => values[0],
            Grid::Integer { lo, .. } => *lo as f64,
            Grid::Step { lo, .. } => *lo,
        }
    }

    pub(crate) fn max(&self) -> f64 {
        match self {
            Grid::Values(values) => *values.last().expect("nonempty grid"),
            Grid::Integer { hi, .. } => *hi as f64,
            Grid::Step { hi, .. } => *hi,
        }
    }

    pub(crate) fn contains(&self, value: f64) -> bool {
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

    pub(crate) fn nearest(&self, value: f64) -> Option<f64> {
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

    pub(crate) fn at_or_above(&self, bound: f64) -> Option<f64> {
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

    pub(crate) fn at_or_below(&self, bound: f64) -> Option<f64> {
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

/// Computes a pure coercion report for one Mock resource (MR-6, MA-11).
pub(crate) fn coerce(
    profile: &Profile,
    device: &ezsdr_kernel::id::ResourceId,
    request: &Requested,
) -> Result<CoerceReport, ModuleError> {
    if request.resource != *device {
        return Err(ModuleError::rejected(format!(
            "MR-6: the request names {}, not this device",
            request.resource
        )));
    }
    let tree = profile.tree(device);
    let configuration_keys = ezsdr_radio::keys::CONFIGURATION;
    let defaults = defaults();
    let mut report = CoerceReport::default();

    for (key, constraint) in &request.constraints {
        let Some(capability) = tree.capabilities.get(key) else {
            report.rejected.push(rejected(
                key,
                constraint,
                "MR-6: not a radio key".to_owned(),
            ));
            continue;
        };
        if !configuration_keys.contains(&key.as_str()) {
            match satisfies(constraint, capability) {
                Ok(true) => {
                    if let ezsdr_kernel::spec::CapabilityValue::One { value } = capability {
                        report.applied.insert(key.clone(), value.clone());
                    }
                }
                _ => report.rejected.push(rejected(
                    key,
                    constraint,
                    format!("MR-6: the device declares {capability:?}"),
                )),
            }
            continue;
        }

        let result = if key.as_str() == ezsdr_radio::keys::RX_CHANNELS
            || key.as_str() == ezsdr_radio::keys::TX_CHANNELS
        {
            coerce_channels(constraint, profile.max_channels(), defaults.get(key))
        } else if key.as_str() == ezsdr_radio::keys::RX_ANTENNA {
            coerce_antenna(constraint, profile.rx_antennas(), defaults.get(key))
        } else if key.as_str() == ezsdr_radio::keys::TX_ANTENNA {
            coerce_antenna(constraint, profile.tx_antennas(), defaults.get(key))
        } else {
            let grid = if key.as_str().ends_with("sample_rate_hz") {
                profile.rate_grid()
            } else if key.as_str().ends_with("frequency_hz") {
                profile.freq_grid()
            } else {
                profile.gain_grid()
            };
            coerce_numeric(key, constraint, &grid, defaults.get(key), profile.kind)
        };
        match result {
            Ok((value, coercion)) => {
                report.applied.insert(key.clone(), value);
                if let Some(coercion) = coercion {
                    report.coercions.push(coercion);
                }
            }
            Err(reason) => report.rejected.push(rejected(key, constraint, reason)),
        }
    }

    let mut effective = defaults;
    for (key, value) in &report.applied {
        if configuration_keys.contains(&key.as_str()) {
            effective.insert(key.clone(), value.clone());
        }
    }
    let performance = profile.performance();
    for (direction, bytes_per_second) in [
        ("rx", performance.rx_bytes_per_s),
        ("tx", performance.tx_bytes_per_s),
    ] {
        let channels_key = Key::parse(&format!("radio.{direction}.channels")).expect("radio key");
        let rate_key = Key::parse(&format!("radio.{direction}.sample_rate_hz")).expect("radio key");
        let channels = match effective.get(&channels_key) {
            Some(Value::Int(value)) => *value as f64,
            _ => 0.0,
        };
        let rate = numeric_value(effective.get(&rate_key)).unwrap_or(0.0);
        let need = channels * rate * performance.wire_bytes_per_sample as f64;
        let cap = bytes_per_second as f64;
        if need > cap {
            let key = if request.constraints.contains_key(&rate_key) {
                rate_key
            } else {
                channels_key
            };
            let requested = request
                .constraints
                .get(&key)
                .cloned()
                .unwrap_or(Constraint::Present {});
            report.applied.remove(&key);
            report.rejected.push(RejectedRequest {
                key,
                requested,
                reason: format!(
                    "RM-7: {direction} needs {need} B/s, the transport carries {cap}"
                ),
            });
        }
    }
    report.warnings.clear();
    Ok(report)
}

pub(crate) fn defaults() -> BTreeMap<Key, Value> {
    use ezsdr_radio::keys::*;
    BTreeMap::from([
        (key(RX_CHANNELS), Value::Int(1)),
        (key(TX_CHANNELS), Value::Int(0)),
        (key(RX_SAMPLE_RATE_HZ), Value::Num(1_000_000.0)),
        (key(TX_SAMPLE_RATE_HZ), Value::Num(1_000_000.0)),
        (key(RX_FREQUENCY_HZ), Value::Num(1_000_000_000.0)),
        (key(TX_FREQUENCY_HZ), Value::Num(1_000_000_000.0)),
        (key(RX_GAIN_DB), Value::Num(0.0)),
        (key(TX_GAIN_DB), Value::Num(0.0)),
        (key(RX_ANTENNA), Value::Str("RX2".to_owned())),
        (key(TX_ANTENNA), Value::Str("TX/RX".to_owned())),
    ])
}

fn key(name: &str) -> Key {
    Key::parse(name).expect("radio key")
}

fn rejected(key: &Key, requested: &Constraint, reason: String) -> RejectedRequest {
    RejectedRequest {
        key: key.clone(),
        requested: requested.clone(),
        reason,
    }
}

fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Int(value) => Some(*value as f64),
        Value::Num(value) if value.is_finite() => Some(*value),
        _ => None,
    }
}

fn numeric_value(value: Option<&Value>) -> Option<f64> {
    value.and_then(number)
}

fn coerce_numeric(
    key: &Key,
    constraint: &Constraint,
    grid: &Grid,
    default: Option<&Value>,
    kind: ProfileKind,
) -> Result<(Value, Option<Coercion>), String> {
    let requested_eq = match constraint {
        Constraint::Eq { value } => Some(value),
        _ => None,
    };
    let value = match constraint {
        Constraint::Eq { value } => {
            let Some(value) = number(value) else {
                return Err("MR-6: numeric configuration needs a number".to_owned());
            };
            if key.as_str().ends_with("sample_rate_hz")
                && kind == ProfileKind::Ideal
                && value.fract() != 0.0
            {
                return Err("MR-6: the ideal profile represents a rate as whole hertz".to_owned());
            }
            grid.nearest(value).ok_or_else(|| {
                format!("RM-8: {value} is outside {}..{}", grid.min(), grid.max())
            })?
        }
        Constraint::Min { value } => {
            let value = number(value).ok_or_else(|| "MR-6: expected a number".to_owned())?;
            grid.at_or_above(value)
                .ok_or_else(|| format!("RM-8: no grid value satisfies {constraint:?}"))?
        }
        Constraint::Max { value } => {
            let value = number(value).ok_or_else(|| "MR-6: expected a number".to_owned())?;
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
        Constraint::Present {} => number(default.ok_or_else(|| "MR-5: no default".to_owned())?)
            .ok_or_else(|| "MR-5: numeric default is invalid".to_owned())?,
    };
    let applied = Value::Num(value);
    let coercion = requested_eq.and_then(|requested| {
        number(requested)
            .filter(|requested| *requested != value)
            .map(|_| Coercion {
                key: key.clone(),
                requested: requested.clone(),
                applied: applied.clone(),
                reason: "RM-8: nearest grid value".to_owned(),
            })
    });
    Ok((applied, coercion))
}

fn coerce_channels(
    constraint: &Constraint,
    max: i64,
    default: Option<&Value>,
) -> Result<(Value, Option<Coercion>), String> {
    let grid = Grid::Integer { lo: 0, hi: max };
    let number = match constraint {
        Constraint::Eq { value: Value::Int(value) }
        | Constraint::Min { value: Value::Int(value) }
        | Constraint::Max { value: Value::Int(value) } => *value as f64,
        Constraint::Range {
            min,
            max: upper_bound,
        } => {
            let lower = min.as_ref().map(int_value).transpose()?.unwrap_or(0) as f64;
            let upper = upper_bound
                .as_ref()
                .map(int_value)
                .transpose()?
                .unwrap_or(max) as f64;
            let Some(value) = grid.at_or_above(lower).filter(|value| *value <= upper) else {
                return Err(format!("MR-6: no channel count satisfies {constraint:?}"));
            };
            return Ok((Value::Int(value as i64), None));
        }
        Constraint::Set { values } => {
            let Some(value) = values.iter().filter_map(|v| int_value(v).ok()).find(|value| {
                *value >= 0 && *value <= max
            }) else {
                return Err(format!("MR-6: no channel count satisfies {constraint:?}"));
            };
            return Ok((Value::Int(value), None));
        }
        Constraint::Present {} => return Ok((default.cloned().ok_or("MR-5: no default")?, None)),
        _ => return Err("MR-6: channel count must be an integer".to_owned()),
    };
    let selected = match constraint {
        Constraint::Eq { .. } => grid.nearest(number),
        Constraint::Min { .. } => grid.at_or_above(number),
        Constraint::Max { .. } => grid.at_or_below(number),
        _ => None,
    }
    .filter(|value| value.fract() == 0.0)
    .ok_or_else(|| format!("MR-6: channel count is outside 0..={max}"))?;
    Ok((Value::Int(selected as i64), None))
}

fn int_value(value: &Value) -> Result<i64, String> {
    match value {
        Value::Int(value) => Ok(*value),
        _ => Err("MR-6: channel count must be an integer".to_owned()),
    }
}

fn coerce_antenna(
    constraint: &Constraint,
    allowed: &[&str],
    default: Option<&Value>,
) -> Result<(Value, Option<Coercion>), String> {
    match constraint {
        Constraint::Eq { value: Value::Str(value) } if allowed.contains(&value.as_str()) => {
            Ok((Value::Str(value.clone()), None))
        }
        Constraint::Set { values } => values
            .iter()
            .find_map(|value| match value {
                Value::Str(value) if allowed.contains(&value.as_str()) => {
                    Some(Value::Str(value.clone()))
                }
                _ => None,
            })
            .map(|value| (value, None))
            .ok_or_else(|| "MR-6: no requested antenna is available".to_owned()),
        Constraint::Present {} => Ok((default.cloned().ok_or("MR-5: no default")?, None)),
        Constraint::Eq { value: Value::Str(value) } => {
            Err(format!("MR-6: {value} is not an antenna of this device"))
        }
        _ => Err("MR-6: antenna must be a supported string".to_owned()),
    }
}

fn bound(value: Option<&Value>, default: f64) -> Result<f64, String> {
    match value {
        None => Ok(default),
        Some(value) => number(value).ok_or_else(|| "MR-6: numeric bound needs a number".to_owned()),
    }
}
