//! One implementation of RM-2's tree, RM-5's defaults, RM-7's refusal and RM-8's
//! coercion over a [`DeviceDescription`], shared by every radio Provider (RM-26).
//! Moved from MockRadio 1.2.0 with its logic unchanged (Phase 7, VE-1).

use std::collections::BTreeMap;

use ezsdr_kernel::binding::satisfies;
use ezsdr_kernel::contract::{DataContractId, Port, PortDirection};
use ezsdr_kernel::id::ResourceId;
use ezsdr_kernel::module_api::{
    CoerceReport, ModuleError, ProfileRef, RejectedRequest, Requested, Resource,
};
use ezsdr_kernel::spec::{CapabilityValue, Coercion, Constraint, Ident, Key, Namespace, Value};

use crate::{PerformanceEnvelope, RadioEnvelope, TimingEnvelope, keys};

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

/// What a radio Provider's profile says about its device: enough to build RM-2's tree
/// and to coerce a request against it (RM-26).
#[derive(Clone, PartialEq, Debug)]
pub struct DeviceDescription {
    /// The profile this describes (RM-20).
    pub profile: ProfileRef,
    /// Both directions' sample rates; `Values` becomes an `AnyOf` capability (in
    /// descending order), any other grid a `Range`.
    pub rates: Grid,
    /// An `Eq` rate that is not a whole number of hertz is refused.
    pub whole_hertz_rates: bool,
    /// The tuning range, a `Step` grid; step 0 means no grid.
    pub frequency: Grid,
    /// The gain range, a `Step` grid.
    pub gain: Grid,
    /// Channels per direction.
    pub max_channels: i64,
    /// Receive antenna names.
    pub rx_antennas: Vec<String>,
    /// Transmit antenna names.
    pub tx_antennas: Vec<String>,
    /// `radio.rx.coherent`.
    pub coherent: bool,
    /// `radio.full_duplex`.
    pub full_duplex: bool,
    /// `radio.hardware_time`.
    pub hardware_time: bool,
    /// `radio.phase_behavior_on_retune`.
    pub phase_behavior_on_retune: String,
    /// `radio.tx.repeat_max_samples`.
    pub repeat_max_samples: u64,
    /// `radio.tx.repeat_align_samples`.
    pub repeat_align_samples: u64,
    /// `radio.rx.block_len`.
    pub block_len: u32,
    /// `radio.tx.path_delay_samples` (RM-23).
    pub tx_path_delay_samples: i64,
    /// `radio.rx.path_delay_samples` (RM-23).
    pub rx_path_delay_samples: i64,
    /// The TimingEnvelope (RM-6).
    pub timing: TimingEnvelope,
    /// The PerformanceEnvelope (RM-7).
    pub performance: PerformanceEnvelope,
    /// RM-5's defaults: exactly the ten configuration keys.
    pub defaults: BTreeMap<Key, Value>,
}

fn key(name: &str) -> Key {
    Key::parse(name).expect("a declared radio key")
}

impl DeviceDescription {
    /// RM-5's defaults for MockRadio 1.2.0 and the X310 profiles that keep its values:
    /// 1 receive channel, no transmit channel, 1 Msps, 1 GHz, 0 dB, `RX2` and `TX/RX`.
    pub fn x310_defaults() -> BTreeMap<Key, Value> {
        use keys::*;
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

    /// The envelope a radio Provider records (RM-20).
    pub fn envelope(&self) -> RadioEnvelope {
        RadioEnvelope {
            profile: self.profile.clone(),
            timing: self.timing,
            performance: self.performance,
        }
    }

    /// RM-2's tree for the device at `device`, with the capabilities in MR-4's form.
    pub fn tree(&self, device: &ResourceId) -> Resource {
        use keys::*;
        let mut capabilities = BTreeMap::new();
        let range = |min, max| CapabilityValue::Range { min, max };
        let one = |value| CapabilityValue::One { value };
        let names = |names: &[String]| CapabilityValue::AnyOf {
            values: names.iter().map(|name| Value::Str(name.clone())).collect(),
        };
        for direction in ["rx", "tx"] {
            capabilities.insert(
                key(&format!("radio.{direction}.channels")),
                range(Value::Int(0), Value::Int(self.max_channels)),
            );
            let rates = match &self.rates {
                Grid::Values(values) => CapabilityValue::AnyOf {
                    values: values.iter().rev().map(|rate| Value::Num(*rate)).collect(),
                },
                grid => range(Value::Num(grid.min()), Value::Num(grid.max())),
            };
            capabilities.insert(key(&format!("radio.{direction}.sample_rate_hz")), rates);
            capabilities.insert(
                key(&format!("radio.{direction}.frequency_hz")),
                range(Value::Num(self.frequency.min()), Value::Num(self.frequency.max())),
            );
            capabilities.insert(
                key(&format!("radio.{direction}.gain_db")),
                range(Value::Num(self.gain.min()), Value::Num(self.gain.max())),
            );
            capabilities.insert(
                key(&format!("radio.{direction}.frequency_step_hz")),
                one(Value::Num(self.frequency.step())),
            );
            capabilities.insert(
                key(&format!("radio.{direction}.gain_step_db")),
                one(Value::Num(self.gain.step())),
            );
        }
        capabilities.insert(key(RX_ANTENNA), names(&self.rx_antennas));
        capabilities.insert(key(TX_ANTENNA), names(&self.tx_antennas));
        let int = |n: i64| Value::Int(n);
        for (name, value) in [
            (RX_COHERENT, Value::Bool(self.coherent)),
            (FULL_DUPLEX, Value::Bool(self.full_duplex)),
            (HARDWARE_TIME, Value::Bool(self.hardware_time)),
            (PHASE_BEHAVIOR_ON_RETUNE, Value::Str(self.phase_behavior_on_retune.clone())),
            (
                TX_REPEAT_MAX_SAMPLES,
                int(i64::try_from(self.repeat_max_samples).expect("a profile limit fits i64")),
            ),
            (TX_REPEAT_ALIGN_SAMPLES, int(self.repeat_align_samples as i64)),
            (RX_BLOCK_LEN, int(i64::from(self.block_len))),
            (MIN_TIMED_COMMAND_LEAD_NS, int(self.timing.min_timed_command_lead_ns)),
            (STARTUP_LATENCY_NS, int(self.timing.startup_latency_ns)),
            (STOP_TAIL_NS, int(self.timing.stop_tail_ns)),
            (COMMAND_QUEUE_DEPTH, int(self.timing.command_queue_depth)),
            (OVERFLOW_RESTART_GAP_NS, int(self.timing.overflow_restart_gap_ns)),
            (RESTART_LEAD_NS, int(self.timing.restart_lead_ns)),
            (START_LEAD_NS, int(self.timing.start_lead_ns)),
            (RX_BYTES_PER_S, int(self.performance.rx_bytes_per_s)),
            (TX_BYTES_PER_S, int(self.performance.tx_bytes_per_s)),
            (WIRE_BYTES_PER_SAMPLE, int(self.performance.wire_bytes_per_sample)),
            (TX_PATH_DELAY_SAMPLES, int(self.tx_path_delay_samples)),
            (RX_PATH_DELAY_SAMPLES, int(self.rx_path_delay_samples)),
        ] {
            capabilities.insert(key(name), one(value));
        }
        let stream = |direction: &str, kind: &str| Resource {
            id: device.child(direction).expect("a valid stream id"),
            kind: Namespace::parse(kind).expect("a radio stream kind"),
            capabilities: BTreeMap::new(),
            children: Vec::new(),
            ports: Vec::new(),
            shareable: false,
        };
        Resource {
            id: device.clone(),
            kind: Namespace::parse(crate::DEVICE_KIND).expect("the radio device kind"),
            capabilities,
            children: vec![
                stream("rx", crate::RX_STREAM_KIND),
                stream("tx", crate::TX_STREAM_KIND),
            ],
            ports: vec![Port {
                name: Ident::parse("rx").expect("the rx port name"),
                direction: PortDirection::Out,
                contract: DataContractId::parse("ezsdr.stream.cf32").expect("cf32"),
            }],
            shareable: false,
        }
    }

    /// RM-5's defaults, RM-7's refusal and RM-8's coercion; pure (MA-11).
    pub fn coerce(
        &self,
        device: &ResourceId,
        request: &Requested,
    ) -> Result<CoerceReport, ModuleError> {
        if request.resource != *device {
            return Err(ModuleError::rejected(format!(
                "RM-26: the request names {}, not this device",
                request.resource
            )));
        }
        let tree = self.tree(device);
        let configuration_keys = keys::CONFIGURATION;
        let defaults = &self.defaults;
        let mut report = CoerceReport::default();

        for (key, constraint) in &request.constraints {
            let Some(capability) = tree.capabilities.get(key) else {
                report
                    .rejected
                    .push(rejected(key, constraint, "RM-4: not a radio key".to_owned()));
                continue;
            };
            if !configuration_keys.contains(&key.as_str()) {
                match satisfies(constraint, capability) {
                    Ok(true) => {
                        if let CapabilityValue::One { value } = capability {
                            report.applied.insert(key.clone(), value.clone());
                        }
                    }
                    _ => report.rejected.push(rejected(
                        key,
                        constraint,
                        format!("RM-8: the device declares {capability:?}"),
                    )),
                }
                continue;
            }

            let result = if key.as_str() == keys::RX_CHANNELS || key.as_str() == keys::TX_CHANNELS
            {
                coerce_channels(constraint, self.max_channels, defaults.get(key))
            } else if key.as_str() == keys::RX_ANTENNA {
                coerce_antenna(constraint, &self.rx_antennas, defaults.get(key))
            } else if key.as_str() == keys::TX_ANTENNA {
                coerce_antenna(constraint, &self.tx_antennas, defaults.get(key))
            } else {
                let (grid, whole) = if key.as_str().ends_with("sample_rate_hz") {
                    (&self.rates, self.whole_hertz_rates)
                } else if key.as_str().ends_with("frequency_hz") {
                    (&self.frequency, false)
                } else {
                    (&self.gain, false)
                };
                coerce_numeric(key, constraint, grid, defaults.get(key), whole)
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

        let mut effective = defaults.clone();
        for (key, value) in &report.applied {
            if configuration_keys.contains(&key.as_str()) {
                effective.insert(key.clone(), value.clone());
            }
        }
        for (direction, bytes_per_second) in [
            ("rx", self.performance.rx_bytes_per_s),
            ("tx", self.performance.tx_bytes_per_s),
        ] {
            let channels_key = Key::parse(&format!("radio.{direction}.channels")).expect("radio key");
            let rate_key = Key::parse(&format!("radio.{direction}.sample_rate_hz")).expect("radio key");
            let channels = match effective.get(&channels_key) {
                Some(Value::Int(value)) => *value as f64,
                _ => 0.0,
            };
            let rate = effective.get(&rate_key).and_then(number).unwrap_or(0.0);
            let need = channels * rate * self.performance.wire_bytes_per_sample as f64;
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
                    reason: format!("RM-7: {direction} needs {need} B/s, the transport carries {cap}"),
                });
            }
        }
        report.warnings.clear();
        Ok(report)
    }
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

fn coerce_numeric(
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
        Constraint::Present {} => number(default.ok_or_else(|| "RM-5: no default".to_owned())?)
            .ok_or_else(|| "RM-5: numeric default is invalid".to_owned())?,
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
        Constraint::Range { min, max: upper_bound } => {
            let lower = min.as_ref().map(int_value).transpose()?.unwrap_or(0) as f64;
            let upper = upper_bound.as_ref().map(int_value).transpose()?.unwrap_or(max) as f64;
            let Some(value) = grid.at_or_above(lower).filter(|value| *value <= upper) else {
                return Err(format!("RM-8: no channel count satisfies {constraint:?}"));
            };
            return Ok((Value::Int(value as i64), None));
        }
        Constraint::Set { values } => {
            let Some(value) = values
                .iter()
                .filter_map(|v| int_value(v).ok())
                .find(|value| *value >= 0 && *value <= max)
            else {
                return Err(format!("RM-8: no channel count satisfies {constraint:?}"));
            };
            return Ok((Value::Int(value), None));
        }
        Constraint::Present {} => return Ok((default.cloned().ok_or("RM-5: no default")?, None)),
        _ => return Err("RM-4: channel count must be an integer".to_owned()),
    };
    let selected = match constraint {
        Constraint::Eq { .. } => grid.nearest(number),
        Constraint::Min { .. } => grid.at_or_above(number),
        Constraint::Max { .. } => grid.at_or_below(number),
        _ => None,
    }
    .filter(|value| value.fract() == 0.0)
    .ok_or_else(|| format!("RM-8: channel count is outside 0..={max}"))?;
    Ok((Value::Int(selected as i64), None))
}

fn int_value(value: &Value) -> Result<i64, String> {
    match value {
        Value::Int(value) => Ok(*value),
        _ => Err("RM-4: channel count must be an integer".to_owned()),
    }
}

fn coerce_antenna(
    constraint: &Constraint,
    allowed: &[String],
    default: Option<&Value>,
) -> Result<(Value, Option<Coercion>), String> {
    let known = |name: &str| allowed.iter().any(|allowed| allowed == name);
    match constraint {
        Constraint::Eq { value: Value::Str(value) } if known(value) => {
            Ok((Value::Str(value.clone()), None))
        }
        Constraint::Set { values } => values
            .iter()
            .find_map(|value| match value {
                Value::Str(value) if known(value) => Some(Value::Str(value.clone())),
                _ => None,
            })
            .map(|value| (value, None))
            .ok_or_else(|| "RM-8: no requested antenna is available".to_owned()),
        Constraint::Present {} => Ok((default.cloned().ok_or("RM-5: no default")?, None)),
        Constraint::Eq { value: Value::Str(value) } => {
            Err(format!("RM-8: {value} is not an antenna of this device"))
        }
        _ => Err("RM-4: antenna must be a supported string".to_owned()),
    }
}

fn bound(value: Option<&Value>, default: f64) -> Result<f64, String> {
    match value {
        None => Ok(default),
        Some(value) => number(value).ok_or_else(|| "RM-4: numeric bound needs a number".to_owned()),
    }
}
