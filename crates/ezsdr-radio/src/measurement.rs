//! Versioned hardware measurement records and profile comparisons (RM-35).

use std::collections::{BTreeMap, BTreeSet};

use ezsdr_kernel::module_api::ProfileRef;
use ezsdr_kernel::spec::{Ident, Key, Value as SelectorValue};
use ezsdr_kernel::stream::Direction;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::device::Grid;
use crate::{Concrete, channels, keys};

/// The one measurement-record schema this version reads and writes.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
pub enum Schema {
    /// `ezsdr.radio.measurement.v1`.
    #[serde(rename = "ezsdr.radio.measurement.v1")]
    V1,
}

/// Whether a record came from hardware or a fake device.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// A physical device.
    Hardware,
    /// A fake device; never evidence for a hardware profile.
    Fake,
}

/// The recorded load condition.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Load {
    /// No added load.
    None,
    /// The H4 load condition.
    H4,
}

/// One device's identifying details in a measurement record.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Device {
    /// The device serial.
    pub serial: String,
    /// The motherboard model.
    pub mboard: String,
    /// The loaded FPGA image.
    pub fpga_image: String,
    /// The UHD version.
    pub uhd_version: String,
}

/// The host and network-configuration script used for a measurement.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Host {
    /// The host name.
    pub name: String,
    /// The settings script name or path.
    pub settings_script: String,
    /// The settings script version.
    pub settings_version: String,
}

/// One declared stream reference in a condition.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StreamRef {
    /// The stream direction.
    pub direction: Direction,
    /// The stream number.
    pub stream: u32,
}

/// The channels and sample rate used by one stream in a condition.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StreamCondition {
    /// The stream direction.
    pub direction: Direction,
    /// The stream number.
    pub stream: u32,
    /// The logical channels assigned to this stream.
    pub channels: Vec<u32>,
    /// The configured sample rate in hertz.
    pub sample_rate_hz: f64,
}

/// The active stream layout and concurrency for one measurement condition.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    /// The condition's unique identifier within the record.
    pub id: String,
    /// The streams and their channel/rate configuration.
    pub streams: Vec<StreamCondition>,
    /// The declared streams exercised concurrently.
    pub concurrent: Vec<StreamRef>,
    /// Whether this condition was measured after reopening the device.
    pub reopened: bool,
}

/// A finite or stepped set of values measured for a containment key.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GridValue {
    /// An explicit, ascending set.
    Values {
        /// Its values.
        values: Vec<f64>,
    },
    /// Every integer in the inclusive range.
    Integer {
        /// The least integer.
        lo: i64,
        /// The greatest integer.
        hi: i64,
    },
    /// Multiples of `step` in the inclusive range; zero is continuous.
    Step {
        /// The lower bound.
        lo: f64,
        /// The upper bound.
        hi: f64,
        /// The grid spacing, or zero for a continuous interval.
        step: f64,
    },
}

impl GridValue {
    fn from_grid(grid: &Grid) -> GridValue {
        match grid {
            Grid::Values(values) => GridValue::Values { values: values.clone() },
            Grid::Integer { lo, hi } => GridValue::Integer { lo: *lo, hi: *hi },
            Grid::Step { lo, hi, step } => GridValue::Step { lo: *lo, hi: *hi, step: *step },
        }
    }

    fn to_grid(&self) -> Result<Grid, String> {
        let grid = match self {
            GridValue::Values { values } if !values.is_empty() && values.iter().all(|value| value.is_finite()) && values.windows(2).all(|pair| pair[0] < pair[1]) => Grid::Values(values.clone()),
            GridValue::Values { .. } => return Err("a values grid must be finite, nonempty and ascending".to_owned()),
            GridValue::Integer { lo, hi } if lo <= hi => Grid::Integer { lo: *lo, hi: *hi },
            GridValue::Integer { .. } => return Err("an integer grid has lo > hi".to_owned()),
            GridValue::Step { lo, hi, step } if lo.is_finite() && hi.is_finite() && step.is_finite() && lo <= hi && *step >= 0.0 => {
                let grid = Grid::Step { lo: *lo, hi: *hi, step: *step };
                if *step > 0.0 {
                    let first = grid.at_or_above(*lo);
                    let last = grid.at_or_below(*hi);
                    if !matches!((first, last), (Some(first), Some(last)) if first <= last) {
                        return Err("a stepped grid has no values".to_owned());
                    }
                }
                grid
            }
            GridValue::Step { .. } => return Err("a stepped grid needs finite bounds, lo <= hi and step >= 0".to_owned()),
        };
        Ok(grid)
    }
}

/// The JSON shape of a measured worst value.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Worst {
    /// A finite numeric result.
    Number(f64),
    /// A boolean result.
    Boolean(bool),
    /// A text result.
    Text(String),
    /// A set of supported names, such as antenna ports.
    Texts(Vec<String>),
    /// A supported numeric grid.
    Grid(GridValue),
}

/// A measured value or an explicit statement that it was not measured.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ValueRecord {
    /// The worst observed value for one capability key.
    Measured {
        /// The capability key.
        key: Key,
        /// The worst observed result, with a shape selected by the key's rule.
        worst: Worst,
        /// The number of trials.
        trials: u32,
        /// The condition under which it was measured.
        condition: String,
        /// The test that produced it.
        test: String,
    },
    /// A capability key deliberately left without a measured value.
    Unmeasured {
        /// The capability key.
        key: Key,
        /// Why it was not measured.
        reason: String,
    },
}

impl ValueRecord {
    fn key(&self) -> &Key {
        match self {
            ValueRecord::Measured { key, .. } | ValueRecord::Unmeasured { key, .. } => key,
        }
    }
}

/// A non-capability observation kept with its test result.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    /// The observation name.
    pub name: String,
    /// The test that produced it.
    pub test: String,
    /// The condition used, when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    /// The number of trials.
    pub trials: u32,
    /// The structured observation result.
    pub result: BTreeMap<String, serde_json::Value>,
    /// Optional explanatory note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// One hardware measurement run for one device and selector layout.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Record {
    /// The exact schema identifier, rejected on deserialization if unsupported.
    pub schema: Schema,
    /// Whether the result came from hardware or a fake device.
    pub source: Source,
    /// The profile whose values this run measures.
    pub profile: ProfileRef,
    /// The Provider binding's flat selector.
    pub selector: BTreeMap<Ident, SelectorValue>,
    /// Device identity and software details.
    pub device: Device,
    /// Host identity and settings-script details.
    pub host: Host,
    /// The load condition for the run.
    pub load: Load,
    /// The source commit identifier.
    pub commit: String,
    /// Operating conditions used by the measurements and observations.
    pub conditions: Vec<Condition>,
    /// One row for every comparable capability key.
    pub values: Vec<ValueRecord>,
    /// Results which are recorded but not compared as capabilities.
    pub observations: Vec<Observation>,
}

/// How a profile capability is compared with its measured value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rule {
    /// The profile value must be at least the measured value.
    LowerBound,
    /// The profile value must be no greater than the measured value.
    UpperBound,
    /// The profile's supported set must be a subset of the measured set.
    Subset,
    /// The profile value must be a multiple of the measured value.
    Multiple,
    /// The profile's guarantee must be no stronger than the measured guarantee.
    Guarantee,
    /// A true profile claim requires a true measured result.
    TrueImplies,
    /// The profile and measured values must be equal.
    Equal,
    /// The capability is intentionally absent from measurement values.
    NotCompared,
}

/// One comparison or record-validation failure.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Violation {
    /// The key or record field that failed.
    pub key: String,
    /// Why the record does not satisfy the profile comparison.
    pub reason: String,
}

/// Returns the comparison rule for one concrete Radio Model capability key.
pub fn rule(key: &str) -> Option<Rule> {
    use Rule::*;

    match key {
        keys::FULL_DUPLEX | keys::HARDWARE_TIME => return Some(TrueImplies),
        keys::TX_REPEAT_MAX_SAMPLES | keys::COMMAND_QUEUE_DEPTH | keys::RX_BYTES_PER_S | keys::TX_BYTES_PER_S => return Some(UpperBound),
        keys::TX_REPEAT_ALIGN_SAMPLES => return Some(Multiple),
        keys::MIN_TIMED_COMMAND_LEAD_NS
        | keys::STARTUP_LATENCY_NS
        | keys::OVERFLOW_RESTART_GAP_NS
        | keys::RX_START_LEAD_NS
        | keys::TX_START_LEAD_NS
        | keys::TX_IN_FLIGHT_NS
        | keys::RELEASE_WINDOW_NS => return Some(LowerBound),
        keys::WIRE_BYTES_PER_SAMPLE => return Some(Equal),
        keys::RX_BLOCK_LEN => return Some(NotCompared),
        _ => {}
    }

    match crate::concrete(&Key::parse(key).ok()?)? {
        Concrete::Channel { leaf: "frequency_hz" | "gain_db" | "antenna", .. } | Concrete::Stream { .. } => Some(Subset),
        Concrete::Channel { leaf: "frequency_step_hz" | "gain_step_db", .. } => Some(LowerBound),
        Concrete::Channel { leaf: "phase_behavior_on_retune", .. } => Some(Guarantee),
        Concrete::Channel { leaf: "path_delay_samples", .. } => Some(Equal),
        Concrete::Channel { leaf: "stream", .. } => Some(NotCompared),
        _ => None,
    }
}

/// Returns every profile capability's value, including keys that have no comparison rule.
pub fn profile_values(description: &channels::Description) -> BTreeMap<Key, Worst> {
    description
        .capabilities()
        .keys()
        .map(|key| {
            let value = profile_value(description, key).unwrap_or_else(|| panic!("no profile value for capability {}", key.as_str()));
            (key.clone(), value)
        })
        .collect()
}

fn profile_value(description: &channels::Description, key: &Key) -> Option<Worst> {
    if let Some(concrete) = crate::concrete(key) {
        return match concrete {
            Concrete::Channel { rx, number, leaf } => {
                let channel = if rx { description.rx.get(number as usize)? } else { description.tx.get(number as usize)? };
                Some(match leaf {
                    "frequency_hz" => Worst::Grid(GridValue::from_grid(&channel.frequency)),
                    "gain_db" => Worst::Grid(GridValue::from_grid(&channel.gain)),
                    "antenna" => Worst::Texts(channel.antennas.clone()),
                    "stream" => {
                        let channels = if rx { &description.rx } else { &description.tx };
                        Worst::Grid(GridValue::Integer { lo: -1, hi: channels.len() as i64 - 1 })
                    }
                    "frequency_step_hz" => Worst::Number(channel.frequency.step()),
                    "gain_step_db" => Worst::Number(channel.gain.step()),
                    "phase_behavior_on_retune" => Worst::Text(channel.phase_behavior_on_retune.clone()),
                    "path_delay_samples" => Worst::Number(channel.path_delay_samples as f64),
                    _ => return None,
                })
            }
            Concrete::Stream { .. } => Some(Worst::Grid(GridValue::from_grid(&description.rates))),
        };
    }

    Some(match key.as_str() {
        keys::FULL_DUPLEX => Worst::Boolean(description.full_duplex),
        keys::HARDWARE_TIME => Worst::Boolean(description.hardware_time),
        keys::TX_REPEAT_MAX_SAMPLES => Worst::Number(description.repeat_max_samples as f64),
        keys::TX_REPEAT_ALIGN_SAMPLES => Worst::Number(description.repeat_align_samples as f64),
        keys::RX_BLOCK_LEN => Worst::Number(description.block_len as f64),
        keys::MIN_TIMED_COMMAND_LEAD_NS => Worst::Number(description.timing.min_timed_command_lead_ns as f64),
        keys::STARTUP_LATENCY_NS => Worst::Number(description.timing.startup_latency_ns as f64),
        keys::COMMAND_QUEUE_DEPTH => Worst::Number(description.timing.command_queue_depth as f64),
        keys::RELEASE_WINDOW_NS => Worst::Number(description.timing.release_window_ns as f64),
        keys::OVERFLOW_RESTART_GAP_NS => Worst::Number(description.timing.overflow_restart_gap_ns as f64),
        keys::RX_START_LEAD_NS => Worst::Number(description.timing.rx_start_lead_ns as f64),
        keys::TX_START_LEAD_NS => Worst::Number(description.timing.tx_start_lead_ns as f64),
        keys::TX_IN_FLIGHT_NS => Worst::Number(description.timing.tx_in_flight_ns as f64),
        keys::RX_BYTES_PER_S => Worst::Number(description.performance.rx_bytes_per_s as f64),
        keys::TX_BYTES_PER_S => Worst::Number(description.performance.tx_bytes_per_s as f64),
        keys::WIRE_BYTES_PER_SAMPLE => Worst::Number(description.performance.wire_bytes_per_sample as f64),
        _ => return None,
    })
}

/// Compares a hardware record with a Provider description and validates its conditions.
pub fn compare(record: &Record, description: &channels::Description) -> Vec<Violation> {
    let mut violations = Vec::new();
    if record.source != Source::Hardware {
        reject(&mut violations, "source", "hardware comparisons reject fake-device records");
    }
    if record.profile != description.profile {
        reject(&mut violations, "profile", "record profile does not match the Provider description");
    }

    let profile = profile_values(description);
    let mut condition_ids = BTreeSet::new();
    for condition in &record.conditions {
        if !condition_ids.insert(condition.id.as_str()) {
            reject(&mut violations, "conditions", format!("duplicate condition id `{}`", condition.id));
        }
        validate_condition(condition, description, &mut violations);
    }

    let mut seen = BTreeSet::new();
    for value in &record.values {
        let name = value.key().as_str();
        if !seen.insert(value.key().clone()) {
            reject(&mut violations, name, "value key appears more than once");
        }
        let Some(rule) = rule(name) else {
            reject(&mut violations, name, "unknown Radio Model capability key");
            continue;
        };
        if rule == Rule::NotCompared {
            reject(&mut violations, name, "this capability is not recorded in values");
            continue;
        }
        let Some(profile_value) = profile.get(value.key()) else {
            reject(&mut violations, name, "key is not a capability of this description");
            continue;
        };
        match value {
            ValueRecord::Unmeasured { reason, .. } => {
                if reason.trim().is_empty() {
                    reject(&mut violations, name, "an unmeasured value needs a reason");
                }
            }
            ValueRecord::Measured { worst, trials, condition, test, .. } => {
                if *trials == 0 {
                    reject(&mut violations, name, "a measured value needs at least one trial");
                }
                if test.trim().is_empty() {
                    reject(&mut violations, name, "a measured value needs its test name");
                }
                if !condition_ids.contains(condition.as_str()) {
                    reject(&mut violations, name, format!("condition `{condition}` is not declared"));
                }
                if let Err(reason) = compare_value(name, rule, profile_value, worst) {
                    reject(&mut violations, name, reason);
                }
            }
        }
    }
    for key in profile.keys() {
        if rule(key.as_str()).is_some_and(|rule| rule != Rule::NotCompared) && !seen.contains(key) {
            reject(&mut violations, key.as_str(), "comparable capability key is missing from values");
        }
    }

    for observation in &record.observations {
        if observation.name.trim().is_empty() || observation.test.trim().is_empty() || observation.trials == 0 {
            reject(&mut violations, "observations", "an observation needs a name, test and at least one trial");
        }
        if let Some(condition) = &observation.condition {
            if !condition_ids.contains(condition.as_str()) {
                reject(&mut violations, "observations", format!("condition `{condition}` is not declared"));
            }
        }
    }
    violations
}

fn validate_condition(condition: &Condition, description: &channels::Description, violations: &mut Vec<Violation>) {
    let mut declared = BTreeSet::new();
    let mut channels = BTreeSet::new();
    for stream in &condition.streams {
        let (rx, count) = match stream.direction {
            Direction::Rx => (true, description.rx.len()),
            Direction::Tx => (false, description.tx.len()),
        };
        let name = format!("conditions.{}.streams.{}.{:?}", condition.id, stream.stream, stream.direction);
        if !declared.insert((rx, stream.stream)) {
            reject(violations, &name, "direction and stream are declared more than once");
        }
        if stream.stream as usize >= count {
            reject(violations, &name, "stream number is outside this direction's channel count");
        }
        if !stream.sample_rate_hz.is_finite() || !description.rates.contains(stream.sample_rate_hz) {
            reject(violations, &name, "sample rate is not on the description's rate grid");
        }
        let mut local = BTreeSet::new();
        for channel in &stream.channels {
            if *channel as usize >= count {
                reject(violations, &name, format!("channel {channel} is outside this direction's layout"));
                continue;
            }
            if !local.insert(*channel) {
                reject(violations, &name, format!("channel {channel} is repeated in the stream"));
                continue;
            }
            if !channels.insert((rx, *channel)) {
                reject(violations, &name, format!("channel {channel} is assigned to more than one stream"));
            }
        }
    }
    let mut concurrent = BTreeSet::new();
    for stream in &condition.concurrent {
        let rx = stream.direction == Direction::Rx;
        let reference = (rx, stream.stream);
        if !concurrent.insert(reference) {
            reject(violations, "conditions.concurrent", "stream is listed more than once");
        }
        if !declared.contains(&reference) {
            reject(violations, "conditions.concurrent", "concurrent names a stream not declared in this condition");
        }
    }
}

fn compare_value(key: &str, rule: Rule, profile: &Worst, measured: &Worst) -> Result<(), String> {
    let number = |value: &Worst| match value {
        Worst::Number(value) if exact_f64(*value) => Some(*value),
        _ => None,
    };
    let numbers = || number(profile).zip(number(measured)).ok_or_else(|| "comparison requires finite numeric values below the exact f64 limit".to_owned());
    match rule {
        Rule::LowerBound => {
            let (profile, measured) = numbers()?;
            if profile >= measured { Ok(()) } else { Err(format!("profile value {profile} is below measured value {measured}")) }
        }
        Rule::UpperBound => {
            let (profile, measured) = numbers()?;
            if profile <= measured { Ok(()) } else { Err(format!("profile value {profile} exceeds measured value {measured}")) }
        }
        Rule::Subset if key.ends_with(".antenna") => match (profile, measured) {
            (Worst::Texts(profile), Worst::Texts(measured)) if profile.iter().all(|value| measured.contains(value)) => Ok(()),
            (Worst::Texts(_), Worst::Texts(_)) => Err("profile antenna set is not a subset of the measured set".to_owned()),
            _ => Err("antenna comparison requires name sets".to_owned()),
        },
        Rule::Subset => {
            let (Worst::Grid(profile), Worst::Grid(measured)) = (profile, measured) else {
                return Err("grid comparison requires profile and measured grids".to_owned());
            };
            let profile = profile.to_grid()?;
            let measured = measured.to_grid()?;
            if grid_subset(&profile, &measured) { Ok(()) } else { Err("profile grid is not a subset of the measured grid".to_owned()) }
        }
        Rule::Multiple => {
            let (profile, measured) = numbers()?;
            let ratio = profile / measured;
            if profile > 0.0 && measured > 0.0 && ratio.is_finite() && ratio.fract() == 0.0 {
                Ok(())
            } else {
                Err(format!("profile value {profile} is not a positive multiple of measured value {measured}"))
            }
        }
        Rule::Guarantee => {
            let (Worst::Text(profile), Worst::Text(measured)) = (profile, measured) else {
                return Err("guarantee comparison requires phase-behavior names".to_owned());
            };
            let rank = |value: &str| match value {
                "random" => Some(0),
                "random_unless_timed_tune" => Some(1),
                "deterministic" => Some(2),
                _ => None,
            };
            match (rank(profile), rank(measured)) {
                (Some(profile), Some(measured)) if profile <= measured => Ok(()),
                (Some(_), Some(_)) => Err("profile phase guarantee is stronger than the measured guarantee".to_owned()),
                _ => Err("unknown phase-behavior guarantee".to_owned()),
            }
        }
        Rule::TrueImplies => match (profile, measured) {
            (Worst::Boolean(true), Worst::Boolean(false)) => Err("profile claims true but the measured result is false".to_owned()),
            (Worst::Boolean(_), Worst::Boolean(_)) => Ok(()),
            _ => Err("truth comparison requires boolean values".to_owned()),
        },
        Rule::Equal => {
            let (profile, measured) = numbers()?;
            if profile == measured { Ok(()) } else { Err(format!("profile value {profile} differs from measured value {measured}")) }
        }
        Rule::NotCompared => Err("this capability is not compared".to_owned()),
    }
}

fn grid_subset(profile: &Grid, measured: &Grid) -> bool {
    if !grid_numbers_are_exact(profile) || !grid_numbers_are_exact(measured) {
        return false;
    }
    match profile {
        Grid::Values(values) => values.iter().all(|value| measured.contains(*value)),
        Grid::Integer { lo, hi } => match measured {
            Grid::Integer { lo: measured_lo, hi: measured_hi } => lo >= measured_lo && hi <= measured_hi,
            Grid::Values(values) => integer_values(*lo, *hi, values.len()).is_some_and(|values| values.iter().all(|value| measured.contains(*value))),
            Grid::Step { lo: measured_lo, hi: measured_hi, step } => {
                let lo = *lo as f64;
                let hi = *hi as f64;
                if lo < *measured_lo || hi > *measured_hi {
                    return false;
                }
                if lo == hi || *step == 0.0 {
                    return measured.contains(lo) && (lo == hi || *step == 0.0);
                }
                measured.contains(lo) && measured.contains(lo + 1.0) && measured.contains(hi)
            }
        },
        Grid::Step { lo, hi, step } => match measured {
            Grid::Values(values) => stepped_values(*lo, *hi, *step, values.len()).is_some_and(|values| values.iter().all(|value| measured.contains(*value))),
            Grid::Integer { lo: measured_lo, hi: measured_hi } => {
                if *step == 0.0 {
                    return *lo == *hi && lo.fract() == 0.0 && *lo >= *measured_lo as f64 && *hi <= *measured_hi as f64;
                }
                let Some((first, last)) = step_endpoints(*lo, *hi, *step) else { return false };
                if first < *measured_lo as f64 || last > *measured_hi as f64 {
                    return false;
                }
                first.fract() == 0.0 && last.fract() == 0.0 && (first == last || step.fract() == 0.0)
            }
            Grid::Step { lo: measured_lo, hi: measured_hi, step: measured_step } => {
                let Some((first, last)) = step_endpoints(*lo, *hi, *step) else { return false };
                if first < *measured_lo || last > *measured_hi {
                    return false;
                }
                if *measured_step == 0.0 {
                    return true;
                }
                if first == last {
                    return measured.contains(first);
                }
                if *step == 0.0 {
                    return false;
                }
                let ratio = *step / *measured_step;
                measured.contains(first) && measured.contains(last) && ratio.is_finite() && ratio.fract() == 0.0
            }
        },
    }
}

fn exact_f64(value: f64) -> bool {
    value.is_finite() && value.abs() < 9_007_199_254_740_992.0
}

fn grid_numbers_are_exact(grid: &Grid) -> bool {
    match grid {
        Grid::Values(values) => values.iter().all(|value| exact_f64(*value)),
        Grid::Integer { lo, hi } => exact_f64(*lo as f64) && exact_f64(*hi as f64),
        Grid::Step { lo, hi, step } => exact_f64(*lo) && exact_f64(*hi) && exact_f64(*step),
    }
}

fn integer_values(lo: i64, hi: i64, maximum: usize) -> Option<Vec<f64>> {
    let count = i128::from(hi) - i128::from(lo) + 1;
    if count < 0 || count > maximum as i128 {
        return None;
    }
    Some((lo..=hi).map(|value| value as f64).collect())
}

fn stepped_values(lo: f64, hi: f64, step: f64, maximum: usize) -> Option<Vec<f64>> {
    if step == 0.0 {
        return (lo == hi && maximum > 0).then_some(vec![lo]);
    }
    let grid = Grid::Step { lo, hi, step };
    let first = grid.at_or_above(lo)?;
    let last = grid.at_or_below(hi)?;
    if last < first {
        return None;
    }
    let count = ((last - first) / step).floor() + 1.0;
    if !count.is_finite() || count > maximum as f64 {
        return None;
    }
    Some((0..count as usize).map(|index| first + index as f64 * step).collect())
}

fn step_endpoints(lo: f64, hi: f64, step: f64) -> Option<(f64, f64)> {
    if step == 0.0 {
        return Some((lo, hi));
    }
    let grid = Grid::Step { lo, hi, step };
    Some((grid.at_or_above(lo)?, grid.at_or_below(hi)?))
}

fn reject(violations: &mut Vec<Violation>, key: impl Into<String>, reason: impl Into<String>) {
    violations.push(Violation { key: key.into(), reason: reason.into() });
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ezsdr_kernel::spec::{Ident, Key, Scalar, Value};

    use super::{GridValue, Rule, Worst, profile_values, rule};
    use crate::channels::{self, Description};
    use crate::x3xx::ProviderTerms;

    #[test]
    fn rm_35_every_profile_capability_has_one_rule_and_one_value() {
        let selector = BTreeMap::from([(Ident::parse("rx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0")])), (Ident::parse("tx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0")]))]);
        let layout = channels::from_selector(&selector).unwrap();
        let terms = ProviderTerms { min_timed_command_lead_ns: 2_000_000, repeat_max_samples: 268_435_456, repeat_align_samples: 2 };
        let description = Description::new(&layout, 2_000, &terms);
        let capabilities = description.capabilities();
        profile_values(&description);

        for key in capabilities.keys() {
            assert!(rule(key.as_str()).is_some(), "{}", key.as_str());
        }
        assert_eq!(rule("radio.rx.c0.stream"), Some(Rule::NotCompared));
        assert_eq!(rule("radio.rx.block_len"), Some(Rule::NotCompared));
        assert_eq!(rule("radio.rx.c0.unknown"), None);
    }

    #[test]
    fn rm_35_numbered_channel_and_stream_keys_reach_the_second_layout_entry() {
        let selector = BTreeMap::from([
            (Ident::parse("rx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:B:obx:0")])),
            (Ident::parse("tx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:B:obx:0")])),
        ]);
        let layout = channels::from_selector(&selector).unwrap();
        let description = Description::new(&layout, 2_000, &ProviderTerms { min_timed_command_lead_ns: 2_000_000, repeat_max_samples: 268_435_456, repeat_align_samples: 2 });
        let values = profile_values(&description);
        let frequency = values.get(&Key::parse("radio.rx.c1.frequency_hz").unwrap()).unwrap();
        assert!(matches!(frequency, Worst::Grid(GridValue::Step { hi, .. }) if *hi == 8_400_000_000.0));
        assert!(values.contains_key(&Key::parse("radio.rx.s1.sample_rate_hz").unwrap()));
        assert!(values.contains_key(&Key::parse("radio.tx.s1.sample_rate_hz").unwrap()));
    }
}
