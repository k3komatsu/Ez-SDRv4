//! Measurement-record construction and pure trial classification for the UHD bench.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration as Wall, Instant};

use ezsdr_kernel::binding::{Binding, BindingProfile};
use ezsdr_kernel::spec::{Ident, Key};
use ezsdr_kernel::coordinator::RunHandle;
use ezsdr_kernel::id::ClockDomainId;
use ezsdr_kernel::manifest::{ArtifactRef, Manifest};
use ezsdr_kernel::run::{Lease, RunState};
use ezsdr_kernel::stream::{BurstEnd, Direction};
use ezsdr_kernel::time::{SampleClockRecord, TimePoint};
use ezsdr_radio::channels;
use ezsdr_radio::device::Grid;
use ezsdr_radio::measurement::{
    self as model, Condition, Device as DeviceRecord, Host, Load, Observation, Record, Rule,
    Source, StreamCondition, StreamRef, ValueRecord,
};
use ezsdr_radio::x3xx::ProviderTerms;
use ezsdr_radio_uhd::{Device, Dir, RxRecv, Settings};
use serde_json::{json, Value as Json};

use super::SAFE_TX_HZ;

const REQUIRED_ENV: [&str; 8] = [
    "EZSDR_MEASUREMENT_OUTPUT",
    "EZSDR_MEASUREMENT_MBOARD",
    "EZSDR_MEASUREMENT_FPGA_IMAGE",
    "EZSDR_MEASUREMENT_HOST_NAME",
    "EZSDR_MEASUREMENT_SETTINGS_SCRIPT",
    "EZSDR_MEASUREMENT_SETTINGS_VERSION",
    "EZSDR_MEASUREMENT_LOAD",
    "EZSDR_MEASUREMENT_COMMIT",
];

/// Metadata required before a hardware measurement may run.
#[derive(Clone, Debug)]
pub struct Metadata {
    pub output: Option<PathBuf>,
    pub mboard: String,
    pub fpga_image: String,
    pub uhd_version: Option<String>,
    pub host: Host,
    pub load: Load,
    pub commit: String,
}

/// Validates all measurement environment values without reading process-global state.
pub fn metadata_from_env(values: &BTreeMap<String, String>) -> Result<Metadata, String> {
    let required = |name: &str| -> Result<String, String> {
        let value = values.get(name).ok_or_else(|| format!("{name} is required"))?;
        if value.trim().is_empty() {
            return Err(format!("{name} must not be empty"));
        }
        Ok(value.clone())
    };
    let output = PathBuf::from(required(REQUIRED_ENV[0])?);
    if !output.is_absolute() {
        return Err("EZSDR_MEASUREMENT_OUTPUT must be an absolute path".to_owned());
    }
    validate_output_path(&output)?;
    let load = match required(REQUIRED_ENV[6])?.as_str() {
        "none" => Load::None,
        "h4" => Load::H4,
        other => return Err(format!("EZSDR_MEASUREMENT_LOAD must be `none` or `h4`, got `{other}`")),
    };
    Ok(Metadata {
        output: Some(output),
        mboard: required(REQUIRED_ENV[1])?,
        fpga_image: required(REQUIRED_ENV[2])?,
        uhd_version: None,
        host: Host {
            name: required(REQUIRED_ENV[3])?,
            settings_script: required(REQUIRED_ENV[4])?,
            settings_version: required(REQUIRED_ENV[5])?,
        },
        load,
        commit: required(REQUIRED_ENV[7])?,
    })
}

fn validate_output_path(output: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(output) {
        Ok(_) => return Err(format!("measurement output already exists: {}", output.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("cannot inspect measurement output {}: {error}", output.display())),
    }
    let parent = output.parent().unwrap_or_else(|| Path::new("/"));
    if !parent.is_dir() {
        return Err(format!("measurement output parent is not a directory: {}", parent.display()));
    }
    Ok(())
}

/// Reads the required values from the process environment and applies the same validation.
pub fn process_metadata() -> Result<Metadata, String> {
    let values: BTreeMap<String, String> = REQUIRED_ENV
        .iter()
        .filter_map(|name| std::env::var(name).ok().map(|value| ((*name).to_owned(), value)))
        .collect();
    metadata_from_env(&values)
}

/// The explicit non-hardware metadata used by a FakeDevice rehearsal record.
pub fn rehearsal_metadata() -> Metadata {
    Metadata {
        output: None,
        mboard: "FakeDevice".to_owned(),
        fpga_image: "not_applicable".to_owned(),
        uhd_version: Some("not_applicable".to_owned()),
        host: Host {
            name: "test".to_owned(),
            settings_script: "test".to_owned(),
            settings_version: "test".to_owned(),
        },
        load: Load::None,
        commit: "test".to_owned(),
    }
}

/// Gets the radio binding from the same profile document used to assemble a Run.
pub fn radio_binding(document: &Json) -> Result<Binding, String> {
    let profile = BindingProfile::from_json(document).map_err(|error| error.to_string())?;
    profile
        .bindings
        .get(&Ident::parse("radio").map_err(|error| error.to_string())?)
        .cloned()
        .ok_or_else(|| "the profile has no radio binding".to_owned())
}

/// Builds the Radio Model description for the binding's selector and Provider terms.
pub fn description(binding: &Binding, block_len: u32, terms: &ProviderTerms) -> Result<channels::Description, String> {
    let layout = channels::from_selector(&binding.selector)?;
    Ok(channels::Description::new(&layout, block_len, terms))
}

/// Builds a complete record, replacing only the comparable rows supplied by a measurement group.
pub fn build_record(
    source: Source,
    binding: &Binding,
    device: &dyn Device,
    metadata: &Metadata,
    measurements: MeasurementGroup,
    description: &channels::Description,
) -> Result<Record, String> {
    let serial = device.serial().map_err(|error| error.message)?;
    build_record_with_identity(source, binding, device.describe(), serial, metadata, measurements, description)
}

/// `build_record` with the device's identity already read, so that a record can be written after a
/// measurement group left the device's control port unusable (stage 3c-2c).
pub fn build_record_with_identity(
    source: Source,
    binding: &Binding,
    device_description: Json,
    serial: String,
    metadata: &Metadata,
    measurements: MeasurementGroup,
    description: &channels::Description,
) -> Result<Record, String> {
    let device_uhd_version = device_description
        .get("uhd_version")
        .and_then(Json::as_str)
        .filter(|version| !version.trim().is_empty())
        .map(str::to_owned);
    let uhd_version = match (source, device_uhd_version) {
        (_, Some(version)) => version,
        (Source::Fake, None) => metadata.uhd_version.clone().ok_or_else(|| "FakeDevice rehearsal metadata must provide uhd_version".to_owned())?,
        (Source::Hardware, None) => return Err("device metadata does not include a nonempty uhd_version".to_owned()),
    };
    let profile = binding.profile.clone().ok_or_else(|| "radio binding has no profile".to_owned())?;
    Ok(Record {
        schema: model::Schema::V1,
        source,
        profile,
        selector: binding.selector.clone(),
        device: DeviceRecord {
            serial,
            mboard: metadata.mboard.clone(),
            fpga_image: metadata.fpga_image.clone(),
            uhd_version,
        },
        host: metadata.host.clone(),
        load: metadata.load,
        commit: metadata.commit.clone(),
        conditions: measurements.conditions,
        values: merge_values(source, description, measurements.values)?,
        observations: measurements.observations,
    })
}

fn merge_values(source: Source, description: &channels::Description, measured: Vec<ValueRecord>) -> Result<Vec<ValueRecord>, String> {
    let mut values: BTreeMap<Key, ValueRecord> = unmeasured_values(description)
        .into_iter()
        .map(|value| match &value {
            ValueRecord::Unmeasured { key, .. } | ValueRecord::Measured { key, .. } => (key.clone(), value),
        })
        .collect();
    let mut supplied = BTreeSet::new();
    for value in measured {
        let key = match &value {
            ValueRecord::Unmeasured { key, .. } | ValueRecord::Measured { key, .. } => key.clone(),
        };
        if !values.contains_key(&key) {
            return Err(format!("measurement supplied an unknown or non-comparable key `{}`", key.as_str()));
        }
        if !supplied.insert(key.clone()) {
            return Err(format!("measurement supplied `{}` more than once", key.as_str()));
        }
        let value = if source == Source::Fake {
            ValueRecord::Unmeasured { key: key.clone(), reason: "FakeDevice は実機の証拠ではない".to_owned() }
        } else {
            value
        };
        values.insert(key, value);
    }
    Ok(values.into_values().collect())
}

/// Emits one row for each comparable capability and none for `NotCompared` keys.
pub fn unmeasured_values(description: &channels::Description) -> Vec<ValueRecord> {
    model::profile_values(description)
        .keys()
        .filter(|key| model::rule(key.as_str()).is_some_and(|rule| rule != Rule::NotCompared))
        .map(|key| ValueRecord::Unmeasured {
            key: key.clone(),
            reason: unmeasured_reason(key.as_str()).to_owned(),
        })
        .collect()
}

fn unmeasured_reason(key: &str) -> &'static str {
    if key.starts_with("radio.rx.c") || key.starts_with("radio.tx.c") {
        if key.ends_with(".frequency_hz") || key.ends_with(".frequency_step_hz") {
            return "Not measured in this record; Phase 8 stage 3c-2c measures the profile frequency grid by untimed apply read-back.";
        }
        if key.ends_with(".gain_db") || key.ends_with(".gain_step_db") || key.ends_with(".antenna") {
            return "Not measured in this record; Phase 8 stage 3c-2a measures raw Device coercion read-back.";
        }
    }
    if key.starts_with("radio.rx.s") || key.starts_with("radio.tx.s") {
        return "Not measured in this record; Phase 8 stage 3c-2a measures raw Device sample-rate read-back.";
    }
    match key {
        "radio.timing.command_queue_depth" => "Not measured in this record; Phase 8 stage 3c-2a measures raw Device timed-apply queue back-pressure.",
        "radio.timing.min_timed_command_lead_ns"
        | "radio.timing.tx_in_flight_ns"
        | "radio.timing.rx_start_lead_ns"
        | "radio.perf.rx_bytes_per_s"
        | "radio.full_duplex"
        | "radio.hardware_time" => "Not measured in this record; the Phase 8 stage 3b-2 measurement group has not been run.",
        "radio.timing.tx_start_lead_ns" | "radio.perf.tx_bytes_per_s" => "Not measured in this record; the Phase 8 stage 3c-1 hardware measurement has not been run.",
        "radio.timing.overflow_restart_gap_ns" => "Phase 8 stage 3c-1 records full overflow gaps and inferred restart-after-resume estimates as observations; the comparable key stays unmeasured pending the first hardware bench reading and owner decision.",
        _ => "Not measured in Phase 8 stage 3b-1; independent measurement or channel-specific calibration is required.",
    }
}

/// Creates a validated observation from a JSON object.
pub fn observation(
    name: impl Into<String>,
    test: impl Into<String>,
    condition: Option<String>,
    trials: u32,
    result: Json,
) -> Result<Observation, String> {
    let Json::Object(result) = result else { return Err("an observation result must be a JSON object".to_owned()) };
    Ok(Observation {
        name: name.into(),
        test: test.into(),
        condition,
        trials,
        result: result.into_iter().collect(),
        note: None,
    })
}

/// Appends one result collected by a raw hardware probe to this test thread's record buffer.
pub fn push_raw_observation(name: &str, trials: u32, result: Json) {
    RAW_OBSERVATIONS.with(|observations| {
        observations.borrow_mut().push(
            observation(name, format!("tests/hardware.rs::{name}"), None, trials, result)
                .unwrap_or_else(|error| panic!("invalid raw observation {name}: {error}")),
        );
    });
}

/// Clears the current thread's raw observations before a measurement suite run.
pub fn clear_raw_observations() {
    RAW_OBSERVATIONS.with(|observations| observations.borrow_mut().clear());
}

/// Takes the current thread's raw observations after the probes finish.
pub fn take_raw_observations() -> Vec<Observation> {
    RAW_OBSERVATIONS.with(|observations| std::mem::take(&mut *observations.borrow_mut()))
}

thread_local! {
    static RAW_OBSERVATIONS: RefCell<Vec<Observation>> = const { RefCell::new(Vec::new()) };
}

/// Writes a record once and refuses to replace any existing path.
pub fn write_record(path: &Path, record: &Record) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(record).map_err(|error| error.to_string())?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("cannot create measurement output {}: {error}", path.display()))?;
    file.write_all(&bytes)
        .map_err(|error| format!("cannot write measurement output {}: {error}", path.display()))
}

/// Saves a structurally valid record before returning profile-value disagreements to the caller.
pub fn write_record_before_profile_assert(path: &Path, record: &Record, description: &channels::Description) -> Result<Vec<model::Violation>, String> {
    // A multiple's measured operand must be positive; `compare` reports that in the same words as a profile disagreement.
    for value in &record.values {
        if let ValueRecord::Measured { key, worst: model::Worst::Number(measured), .. } = value {
            if model::rule(key.as_str()) == Some(Rule::Multiple) && (measured.is_nan() || *measured <= 0.0) {
                return Err(format!("measurement record validation: {} is not a positive measured multiple", key.as_str()));
            }
        }
    }
    let violations = model::compare(record, description);
    let structural: Vec<_> = violations.iter().filter(|violation| !matches!(violation.reason.as_str(),
        reason if reason.starts_with("profile value ")
            || reason.starts_with("profile antenna set ")
            || reason.starts_with("profile grid is not a subset ")
            || reason == "profile claims true but the measured result is false"
            || reason == "profile phase guarantee is stronger than the measured guarantee"
    )).collect();
    if !structural.is_empty() { return Err(format!("measurement record validation: {structural:?}")); }
    write_record(path, record)?;
    Ok(violations)
}

/// One possible origin of failure evidence in a trial.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProviderTimeError {
    Drop,
    SendAsap,
    Refused,
    PlanViolation,
    LateAtDevice,
}

/// A signal retained from the Provider, device, or Run.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EvidenceKind {
    ProviderTimeError(ProviderTimeError),
    LateCommand,
    CommandRejected,
    CommandQueueFull,
    AsyncTimeError,
    OtherAsync,
    Underflow,
    UnderflowInPacket,
    SequenceError,
    SequenceErrorInBurst,
    BurstAck,
    ShortSendRejected,
    RxOverflow,
    UnknownTimeError,
}

/// Evidence with the device/Run tick used for conservative trial attribution.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TimedEvidence {
    pub tick: Option<i64>,
    pub span: Option<(i64, i64)>,
    pub kind: EvidenceKind,
}

/// The record-derived evidence expected from one booked trial.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TrialRecord {
    pub samples: u64,
    pub end_tick: i64,
}

/// Which observation completes a trial's expected record interval.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrialCompletion {
    ReceiveCapture,
    TransmitBurstAck,
}

/// All evidence attributed to one sequential trial.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TrialEvidence {
    pub start_tick: i64,
    pub end_tick: i64,
    pub open: bool,
    pub admitted: bool,
    pub expected_samples: u64,
    pub expected_end_tick: i64,
    pub completion: TrialCompletion,
    pub requires_preemption_evidence: bool,
    pub repeat_ended_at_target: bool,
    pub record: Option<TrialRecord>,
    pub burst_ack_ticks: Vec<i64>,
    pub evidence: Vec<EvidenceKind>,
    pub unattributable: bool,
}

impl TrialEvidence {
    pub fn new(
        start_tick: i64,
        end_tick: i64,
        expected_samples: u64,
        expected_end_tick: i64,
        completion: TrialCompletion,
    ) -> TrialEvidence {
        TrialEvidence {
            start_tick,
            end_tick,
            open: false,
            admitted: true,
            expected_samples,
            expected_end_tick,
            completion,
            requires_preemption_evidence: false,
            repeat_ended_at_target: false,
            record: None,
            burst_ack_ticks: Vec::new(),
            evidence: Vec::new(),
            unattributable: false,
        }
    }
}

/// The mutually exclusive result used by the measurement summarizer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrialClass {
    AdmissionRejection,
    ProviderFailure,
    DeviceFailure,
    Incomplete,
    Unattributable,
    Success,
}

/// Assigns one event only to a uniquely matching trial; ambiguous and out-of-range evidence fails a trial.
pub fn attribute_evidence(trials: &mut [TrialEvidence], evidence: TimedEvidence) {
    let (matches, can_attribute): (Vec<usize>, bool) = if let Some((start, end)) = evidence.span {
        let matches: Vec<usize> = trials
            .iter()
            .enumerate()
            .filter(|(_, trial)| start < trial.end_tick && end > trial.start_tick)
            .map(|(index, _)| index)
            .collect();
        let contained = matches.len() == 1
            && start >= trials[matches[0]].start_tick
            && end <= trials[matches[0]].end_tick;
        (matches, contained)
    } else if let Some(tick) = evidence.tick {
        (trials
            .iter()
            .enumerate()
            .filter(|(_, trial)| trial.start_tick <= tick && tick < trial.end_tick)
            .map(|(index, _)| index)
            .collect(), true)
    } else {
        (Vec::new(), false)
    };
    match matches.as_slice() {
        [index] if can_attribute => trials[*index].evidence.push(evidence.kind),
        [] => mark_unattributable(trials, evidence.tick),
        indexes => {
            for index in indexes { trials[*index].unattributable = true; }
        }
    }
}

fn mark_unattributable(trials: &mut [TrialEvidence], tick: Option<i64>) {
    let target = trials
        .iter()
        .rposition(|trial| trial.open)
        .or_else(|| match tick {
            Some(tick) if trials.first().is_some_and(|trial| tick < trial.start_tick) => Some(0),
            Some(tick) => trials.iter().rposition(|trial| trial.start_tick <= tick).or(Some(0)),
            None => trials.len().checked_sub(1),
        });
    if let Some(index) = target { trials[index].unattributable = true; }
}

/// Classifies a trial only after its record, samples, end, and device acknowledgement agree.
pub fn classify_trial(trial: &TrialEvidence) -> TrialClass {
    if !trial.admitted { return TrialClass::AdmissionRejection; }
    if trial.unattributable || trial.evidence.contains(&EvidenceKind::UnknownTimeError) { return TrialClass::Unattributable; }
    let provider = trial.evidence.iter().any(|evidence| matches!(
        evidence,
        EvidenceKind::ProviderTimeError(ProviderTimeError::Drop | ProviderTimeError::SendAsap | ProviderTimeError::Refused | ProviderTimeError::PlanViolation)
            | EvidenceKind::LateCommand
            | EvidenceKind::CommandRejected
            | EvidenceKind::CommandQueueFull
    ));
    let device = trial.evidence.iter().any(|evidence| matches!(
        evidence,
        EvidenceKind::ProviderTimeError(ProviderTimeError::LateAtDevice)
            | EvidenceKind::AsyncTimeError
            | EvidenceKind::Underflow
            | EvidenceKind::UnderflowInPacket
            | EvidenceKind::SequenceError
            | EvidenceKind::SequenceErrorInBurst
            | EvidenceKind::OtherAsync
            | EvidenceKind::RxOverflow
    ));
    if provider && device { return TrialClass::Unattributable; }
    if provider { return TrialClass::ProviderFailure; }
    if device { return TrialClass::DeviceFailure; }
    let Some(record) = trial.record else { return TrialClass::Incomplete };
    let missing_completion = match trial.completion {
        TrialCompletion::ReceiveCapture => false,
        TrialCompletion::TransmitBurstAck => !trial.evidence.contains(&EvidenceKind::BurstAck),
    };
    if record.samples != trial.expected_samples
        || (trial.completion == TrialCompletion::TransmitBurstAck && record.end_tick != trial.expected_end_tick)
        || missing_completion
        || (trial.requires_preemption_evidence && !trial.repeat_ended_at_target)
        || trial.evidence.contains(&EvidenceKind::ShortSendRejected)
    {
        return TrialClass::Incomplete;
    }
    TrialClass::Success
}

/// Which edge of a candidate sweep is being summarized.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SummaryMode {
    MinimumLead,
    Capacity,
}

/// Trial classifications recorded for one candidate value.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CandidateStage {
    pub value: u64,
    pub trials: Vec<TrialClass>,
}

/// The per-condition counts retained even when no comparable value can be selected.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ConditionCounts {
    pub candidates: usize,
    pub passing: usize,
    pub under_trialed: usize,
}

/// A conservative candidate summary; 3b-1 does not convert it into a measured value.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CandidateSummary {
    pub worst: Option<u64>,
    pub limiting_condition: Option<String>,
    pub unmeasured_reason: Option<String>,
    pub conditions: BTreeMap<String, ConditionCounts>,
}

/// Applies the successful-suffix lead rule or successful-prefix capacity rule across conditions.
pub fn summarize_candidates(
    mode: SummaryMode,
    candidates: &BTreeMap<String, Vec<CandidateStage>>,
    required_trials: usize,
) -> Result<CandidateSummary, String> {
    if candidates.is_empty() { return Err("at least one condition is required".to_owned()); }
    if required_trials == 0 { return Err("required_trials must be positive".to_owned()); }
    let mut common: Option<BTreeSet<u64>> = None;
    let mut eligible_by_condition = BTreeMap::new();
    let mut sorted = BTreeMap::new();
    for (condition, stages) in candidates {
        if condition.trim().is_empty() || stages.is_empty() {
            return Err("each condition needs an id and candidate stages".to_owned());
        }
        let mut stages = stages.clone();
        stages.sort_by_key(|stage| stage.value);
        if stages.windows(2).any(|pair| pair[0].value == pair[1].value) {
            return Err(format!("condition `{condition}` repeats a candidate value"));
        }
        let passing: Vec<bool> = stages
            .iter()
            .map(|stage| {
                stage.trials.len() >= required_trials && stage.trials.iter().all(|trial| *trial == TrialClass::Success)
            })
            .collect();
        let eligible: BTreeSet<u64> = match mode {
            SummaryMode::MinimumLead => (0..stages.len())
                .filter(|index| passing[*index..].iter().all(|passed| *passed))
                .map(|index| stages[index].value)
                .collect(),
            SummaryMode::Capacity => {
                let prefix = passing.iter().take_while(|passed| **passed).count();
                (0..prefix).map(|index| stages[index].value).collect()
            }
        };
        eligible_by_condition.insert(condition.clone(), eligible.clone());
        common = Some(match common { Some(common) => common.intersection(&eligible).copied().collect(), None => eligible });
        sorted.insert(condition.clone(), stages);
    }
    let common = common.unwrap_or_default();
    if common.is_empty() {
        return Ok(CandidateSummary {
            worst: None,
            limiting_condition: None,
            unmeasured_reason: Some("No candidate value satisfies every condition's successful edge rule with the required complete trials.".to_owned()),
            conditions: counts(&sorted, required_trials),
        });
    }
    let values: Vec<u64> = common.into_iter().collect();
    let worst = match mode {
        SummaryMode::MinimumLead => values.first().copied(),
        SummaryMode::Capacity => values.last().copied(),
    };
    let limiting_condition = worst.and_then(|worst| eligible_by_condition.iter().find(|(_, eligible)| match mode {
        SummaryMode::MinimumLead => eligible.range(..worst).next().is_none(),
        SummaryMode::Capacity => eligible.range((std::ops::Bound::Excluded(worst), std::ops::Bound::Unbounded)).next().is_none(),
    }).map(|(condition, _)| condition.clone()));
    Ok(CandidateSummary { worst, limiting_condition, unmeasured_reason: None, conditions: counts(&sorted, required_trials) })
}

fn counts(stages: &BTreeMap<String, Vec<CandidateStage>>, required: usize) -> BTreeMap<String, ConditionCounts> {
    stages
        .iter()
        .map(|(condition, stages)| {
            let passing = stages.iter().filter(|stage| stage.trials.len() >= required && stage.trials.iter().all(|trial| *trial == TrialClass::Success)).count();
            let under_trialed = stages.iter().filter(|stage| stage.trials.len() < required).count();
            (condition.clone(), ConditionCounts { candidates: stages.len(), passing, under_trialed })
        })
        .collect()
}

/// Constructs a valid stream condition for the selected layout.
pub fn stream_condition(direction: Direction, stream: u32, channels: Vec<u32>, sample_rate_hz: f64) -> StreamCondition {
    StreamCondition { direction, stream, channels, sample_rate_hz }
}

/// A reference to one declared stream in a condition.
pub fn stream_ref(direction: Direction, stream: u32) -> StreamRef {
    StreamRef { direction, stream }
}

/// Returns every comparable key as a set for exact-key assertions.
pub fn comparable_keys(description: &channels::Description) -> BTreeSet<Key> {
    model::profile_values(description)
        .keys()
        .filter(|key| model::rule(key.as_str()).is_some_and(|rule| rule != Rule::NotCompared))
        .cloned()
        .collect()
}

/// Candidate sweeps and trial counts for one hardware measurement suite.
#[derive(Clone, Debug)]
pub struct MeasurementPlan {
    pub timed_command_leads_ns: Vec<u64>,
    pub in_flight_windows_ns: Vec<u64>,
    pub rx_start_leads_ns: Vec<u64>,
    pub tx_start_leads_ns: Vec<u64>,
    pub rx_rates_hz: Vec<u64>,
    pub tx_throughput_rates_hz: Vec<u64>,
    pub overflow_rates_hz: Vec<u64>,
    pub overflow_stall_ms: Vec<u64>,
    pub overflow_after_ms: u64,
    pub step_trials: usize,
    pub overflow_trials: usize,
    pub throughput_seconds: u64,
    pub loopback_trials: usize,
    pub interval_ns: u64,
    pub queue_trials: usize,
    pub queue_ahead_ns: u64,
    pub queue_max_calls: usize,
    /// Wall-clock limit of the whole raw group (queue depth and coercion), in milliseconds.
    pub raw_group_limit_ms: u64,
    /// Points of each channel's frequency read-back sweep.
    pub frequency_points: usize,
}

impl Default for MeasurementPlan {
    fn default() -> Self {
        MeasurementPlan {
            timed_command_leads_ns: vec![500_000, 1_000_000, 1_500_000, 2_000_000, 3_000_000, 5_000_000, 10_000_000],
            in_flight_windows_ns: vec![250_000, 500_000, 1_000_000, 2_000_000, 3_000_000, 5_000_000, 10_000_000],
            rx_start_leads_ns: vec![1_000_000, 2_000_000, 5_000_000, 10_000_000, 25_000_000, 50_000_000, 100_000_000],
            tx_start_leads_ns: vec![1_000_000, 2_000_000, 5_000_000, 10_000_000, 25_000_000, 50_000_000, 100_000_000],
            rx_rates_hz: vec![10_000_000, 20_000_000, 25_000_000, 40_000_000, 50_000_000, 100_000_000, 200_000_000],
            tx_throughput_rates_hz: vec![10_000_000, 20_000_000, 25_000_000, 40_000_000, 50_000_000, 100_000_000, 200_000_000],
            overflow_rates_hz: vec![1_000_000, 10_000_000, 25_000_000, 50_000_000],
            // INFERRED from the 50 MB receive socket buffer: each stall exceeds its
            // approximate fill time at the configured wire rate (RM-17, bench B5).
            overflow_stall_ms: vec![20_000, 2_000, 1_000, 500],
            overflow_after_ms: 500,
            step_trials: 100,
            overflow_trials: 5,
            throughput_seconds: 60,
            loopback_trials: 20,
            interval_ns: 20_000_000,
            queue_trials: 5,
            queue_ahead_ns: 3_000_000_000,
            queue_max_calls: 64,
            raw_group_limit_ms: 300_000,
            frequency_points: 200,
        }
    }
}

impl MeasurementPlan {
    /// Small, deterministic-stage inputs for the FakeDevice record rehearsal.
    pub fn rehearsal() -> MeasurementPlan {
        MeasurementPlan {
            timed_command_leads_ns: vec![1_000_000, 2_000_000],
            in_flight_windows_ns: vec![1_000_000, 2_000_000],
            rx_start_leads_ns: vec![1_000_000, 2_000_000],
            tx_start_leads_ns: vec![1_000_000],
            rx_rates_hz: vec![390_625],
            tx_throughput_rates_hz: vec![390_625],
            overflow_rates_hz: vec![1_000_000],
            overflow_stall_ms: vec![300],
            overflow_after_ms: 200,
            step_trials: 1,
            overflow_trials: 1,
            throughput_seconds: 1,
            loopback_trials: 1,
            interval_ns: 20_000_000,
            queue_trials: 1,
            queue_ahead_ns: 20_000_000,
            queue_max_calls: 8,
            raw_group_limit_ms: 300_000,
            frequency_points: 20,
        }
    }

    /// Validates the values before a device is opened or any measurement Run starts.
    pub fn validate(&self) -> Result<(), String> {
        let candidates = [
            ("timed command lead", &self.timed_command_leads_ns),
            ("in-flight window", &self.in_flight_windows_ns),
            ("receive start lead", &self.rx_start_leads_ns),
            ("transmit start lead", &self.tx_start_leads_ns),
            ("receive rate", &self.rx_rates_hz),
            ("transmit throughput rate", &self.tx_throughput_rates_hz),
            ("overflow rate", &self.overflow_rates_hz),
        ];
        for (name, values) in candidates {
            if values.is_empty() || values.contains(&0) || values.windows(2).any(|pair| pair[0] >= pair[1]) {
                return Err(format!("{name} candidates must be nonempty, positive and strictly ascending"));
            }
        }
        if self.overflow_stall_ms.len() != self.overflow_rates_hz.len() || self.overflow_stall_ms.contains(&0) || self.overflow_after_ms == 0 {
            return Err("overflow rates need one positive stall duration each and a positive pre-stall interval".to_owned());
        }
        if self.step_trials == 0 || self.overflow_trials == 0 || self.throughput_seconds == 0 || self.loopback_trials == 0
            || self.interval_ns == 0 || self.queue_trials == 0 || self.queue_ahead_ns == 0 || self.queue_max_calls == 0
            || self.raw_group_limit_ms == 0 || self.frequency_points < 4
        {
            return Err("measurement counts, the raw group limit and the inter-trial interval must be positive, with at least 4 frequency points".to_owned());
        }
        if i64::try_from(self.queue_max_calls).is_err() {
            return Err("queue command limit exceeds i64".to_owned());
        }
        let step_observations = [
            self.timed_command_leads_ns.len(),
            self.in_flight_windows_ns.len(),
            self.rx_start_leads_ns.len().checked_mul(2).ok_or("receive-start stage count overflow")?,
            self.tx_start_leads_ns.len().checked_mul(2).ok_or("transmit-start stage count overflow")?,
        ];
        for stages in step_observations {
            if stages.checked_mul(self.step_trials).is_none_or(|trials| trials > u32::MAX as usize) {
                return Err("measurement observation exceeds the record's u32 trial limit".to_owned());
            }
        }
        if self.rx_rates_hz.len() > u32::MAX as usize {
            return Err("receive-throughput observation count exceeds the record's u32 trial limit".to_owned());
        }
        if self.tx_throughput_rates_hz.len() > u32::MAX as usize {
            return Err("transmit-throughput observation count exceeds the record's u32 trial limit".to_owned());
        }
        if self.overflow_rates_hz.len().checked_mul(self.overflow_trials).is_none_or(|trials| trials > u32::MAX as usize) {
            return Err("overflow observation exceeds the record's u32 trial limit".to_owned());
        }
        if self.loopback_trials > u32::MAX as usize {
            return Err("EZSDR_MEASURE_LOOPBACK_TRIALS exceeds the record's u32 trial limit".to_owned());
        }
        if self.queue_trials.checked_mul(3).is_none_or(|trials| trials > u32::MAX as usize) {
            return Err("EZSDR_MEASURE_QUEUE_TRIALS exceeds the record's u32 trial limit".to_owned());
        }
        Ok(())
    }

    /// Parses the supported count overrides without consulting global state.
    pub fn from_env(values: &BTreeMap<String, String>) -> Result<MeasurementPlan, String> {
        fn count(values: &BTreeMap<String, String>, name: &str, default: u64) -> Result<u64, String> {
            let Some(value) = values.get(name) else { return Ok(default) };
            let parsed = value.parse::<u64>().map_err(|_| format!("{name} must be a positive integer"))?;
            if parsed == 0 { return Err(format!("{name} must be positive")); }
            Ok(parsed)
        }
        let plan = MeasurementPlan {
            step_trials: usize::try_from(count(values, "EZSDR_MEASURE_STEP_TRIALS", 100)?).map_err(|_| "EZSDR_MEASURE_STEP_TRIALS is too large".to_owned())?,
            overflow_trials: usize::try_from(count(values, "EZSDR_MEASURE_OVERFLOW_TRIALS", 5)?).map_err(|_| "EZSDR_MEASURE_OVERFLOW_TRIALS is too large".to_owned())?,
            throughput_seconds: count(values, "EZSDR_MEASURE_THROUGHPUT_SECS", 60)?,
            loopback_trials: usize::try_from(count(values, "EZSDR_MEASURE_LOOPBACK_TRIALS", 20)?).map_err(|_| "EZSDR_MEASURE_LOOPBACK_TRIALS is too large".to_owned())?,
            queue_trials: usize::try_from(count(values, "EZSDR_MEASURE_QUEUE_TRIALS", 5)?).map_err(|_| "EZSDR_MEASURE_QUEUE_TRIALS is too large".to_owned())?,
            ..MeasurementPlan::default()
        };
        plan.validate()?;
        Ok(plan)
    }

    /// Reads only the documented environment variables and validates before opening a device.
    pub fn process_env() -> Result<MeasurementPlan, String> {
        let names = ["EZSDR_MEASURE_STEP_TRIALS", "EZSDR_MEASURE_OVERFLOW_TRIALS", "EZSDR_MEASURE_THROUGHPUT_SECS", "EZSDR_MEASURE_LOOPBACK_TRIALS", "EZSDR_MEASURE_QUEUE_TRIALS"];
        let values = names.iter().filter_map(|name| std::env::var(name).ok().map(|value| ((*name).to_owned(), value))).collect();
        MeasurementPlan::from_env(&values)
    }
}

const QUEUE_SPACING_NS: u64 = 1_000_000;
const QUEUE_CALL_TIME_LIMIT_NS: u64 = 100_000_000;
const QUEUE_DRAIN_MARGIN_NS: u64 = 10_000_000;
const QUEUE_RX_START_LEAD_NS: u64 = 50_000_000;
const QUEUE_RX_READY_LIMIT_MS: u64 = 1_000;
const QUEUE_TEST: &str = "tests/hardware.rs::hw_measurements_record";

#[derive(Clone, Debug, PartialEq, Eq)]
enum QueueTrialClass {
    Success { depth: usize },
    Incomplete { reason: &'static str },
    DeviceFailure { reason: String },
}

#[derive(Clone, Debug)]
struct QueueCall {
    duration_ns: u64,
    h_i_ns: Option<u64>,
    error: Option<String>,
}

#[derive(Clone, Copy, Debug)]
struct QueueClock {
    d0_tick: i64,
    h_before_ns: u64,
    h_after_ns: u64,
}

fn queue_device_time_bounds(clock: QueueClock, h_i_ns: u64, master_clock_hz: u64) -> Option<(i64, i64)> {
    let since_before = h_i_ns.checked_sub(clock.h_before_ns)?;
    let since_after = h_i_ns.checked_sub(clock.h_after_ns)?;
    let upper_ticks = ticks_for_ns(since_before, master_clock_hz).ok()?;
    let lower_ticks = i64::try_from((u128::from(since_after) * u128::from(master_clock_hz)) / 1_000_000_000).ok()?;
    Some((clock.d0_tick.checked_add(lower_ticks)?, clock.d0_tick.checked_add(upper_ticks)?))
}

fn classify_queue_trial(
    calls: &[QueueCall], clock: QueueClock, first_at: i64, stall_limit_ns: u64, return_limit_ticks: i64, master_clock_hz: u64,
) -> QueueTrialClass {
    let Some(read_duration_ns) = clock.h_after_ns.checked_sub(clock.h_before_ns) else {
        return QueueTrialClass::Incomplete { reason: "the device-time read host interval was invalid" };
    };
    if read_duration_ns > QUEUE_CALL_TIME_LIMIT_NS {
        return QueueTrialClass::Incomplete { reason: "the device-time read took too long to bound the host-clock interval" };
    }
    if let Some(reason) = calls.iter().find_map(|call| call.error.as_deref()) {
        return QueueTrialClass::DeviceFailure { reason: reason.to_owned() };
    }
    let stalled: Vec<_> = calls.iter().enumerate().filter(|(_, call)| call.duration_ns > stall_limit_ns).collect();
    let Some((depth, blocked)) = stalled.first().copied() else {
        return QueueTrialClass::Incomplete { reason: "no timed apply blocked before the call limit" };
    };
    if stalled.len() != 1 {
        return QueueTrialClass::Incomplete { reason: "more than one timed apply crossed the stall limit" };
    }
    if calls[..depth].iter().any(|call| call.duration_ns >= QUEUE_CALL_TIME_LIMIT_NS) {
        return QueueTrialClass::Incomplete { reason: "a call before the blocked apply was not short" };
    }
    if calls[..depth].iter().any(|call| call.h_i_ns.and_then(|h_i| queue_device_time_bounds(clock, h_i, master_clock_hz))
        .is_none_or(|(_, upper)| upper >= first_at)) {
        return QueueTrialClass::Incomplete { reason: "a call before the blocked apply has an upper device-time bound at or after the first command time" };
    }
    let Some(latest_return_tick) = first_at.checked_add(return_limit_ticks) else {
        return QueueTrialClass::Incomplete { reason: "the blocked apply return-time limit overflowed" };
    };
    let blocked_bounds = blocked.h_i_ns.and_then(|h_i| queue_device_time_bounds(clock, h_i, master_clock_hz));
    if blocked_bounds.is_none_or(|(lower, _)| lower < first_at) {
        return QueueTrialClass::Incomplete { reason: "the blocked apply has a lower device-time bound before the first command time" };
    }
    if blocked_bounds.is_some_and(|(_, upper)| upper > latest_return_tick) {
        return QueueTrialClass::Incomplete { reason: "the blocked apply has an upper device-time bound too long after the first command time" };
    }
    QueueTrialClass::Success { depth }
}

fn finish_queue_trial(queue_class: QueueTrialClass, operational_error: Option<String>, untimed_error: Option<String>) -> QueueTrialClass {
    match operational_error.or(untimed_error) {
        Some(reason) => QueueTrialClass::DeviceFailure { reason },
        None => queue_class,
    }
}

#[derive(Clone)]
struct QueueOperation {
    dir: Dir,
    settings: Settings,
}

fn queue_condition(id: &str, operations: Vec<QueueOperation>) -> (Condition, Vec<QueueOperation>) {
    (Condition { id: id.to_owned(), streams: vec![], concurrent: vec![], reopened: false }, operations)
}

fn queue_operations(dir: Dir, field: &str) -> Vec<QueueOperation> {
    let settings: Vec<Settings> = match field {
        "frequency" => [999_600_000.0, 1_000_400_000.0].into_iter().map(|value| Settings { freq: Some(value), ..Settings::default() }).collect(),
        "gain" => [0.0, 10.0].into_iter().map(|value| Settings { gain: Some(value), ..Settings::default() }).collect(),
        _ => unreachable!("known queue operation"),
    };
    settings.into_iter().map(|settings| QueueOperation { dir, settings }).collect()
}

/// Receive gain and transmit gain reach the radio alone and need no stream; receive frequency also
/// writes the DDC, whose timed commands leave only with samples, so its trials stream and drain.
/// Transmit frequency and the receive/transmit alternation are left out (stage 3c-2c): the DUC
/// needs transmitted samples, and a continuous transmission adds RF handling this stage does not take on.
fn queue_conditions() -> Vec<(Condition, Vec<QueueOperation>)> {
    vec![
        queue_condition("queue-rx-gain", queue_operations(Dir::Rx, "gain")),
        queue_condition("queue-tx-gain", queue_operations(Dir::Tx, "gain")),
        queue_condition("queue-rx-frequency", queue_operations(Dir::Rx, "frequency")),
    ]
}

/// Whether a condition's operations write the DDC, which needs a streaming, drained receiver.
fn queue_streams_rx(operations: &[QueueOperation]) -> bool {
    operations.iter().any(|operation| operation.dir == Dir::Rx && operation.settings.freq.is_some())
}

const QUEUE_EXCLUDED_CONDITIONS: [(&str, &str); 2] = [
    ("queue-tx-frequency", "the DUC takes a timed command only with transmitted samples; a continuous transmission adds RF handling this stage does not take on"),
    ("queue-rx-tx-frequency", "it alternates the transmit frequency, which is left out for the same reason"),
];

/// The wall-clock limit and the first-failure latch of the raw group (queue depth and coercion,
/// stage 3c-2c), shared by the command, drain and cleanup paths.
///
/// The bound it gives, exactly: each path checks the latch before each device call, so once one
/// thread has latched a failure (or the limit has passed) the other thread begins at most one more
/// call, the one whose check came before the latch and whose call came after it, and that call
/// ends by UHD's own 10 s `op_timeout`; a call already running is only waited for; nothing is
/// begun after that. The two threads' calls are not serialised by a lock, because that would change
/// how long an `apply` blocks, which is what the queue-depth measurement measures. A wedged
/// control port needs a power cycle.
pub struct RawGroup {
    started: Instant,
    limit: Wall,
    /// Elapsed time added by a test, so that a limit can pass at a chosen call.
    skew_ns: AtomicU64,
    failure: Mutex<Option<String>>,
    stopped: AtomicBool,
}

impl RawGroup {
    pub fn new(plan: &MeasurementPlan) -> RawGroup {
        RawGroup::with_limit(Wall::from_millis(plan.raw_group_limit_ms))
    }

    pub fn with_limit(limit: Wall) -> RawGroup {
        RawGroup { started: Instant::now(), limit, skew_ns: AtomicU64::new(0), failure: Mutex::new(None), stopped: AtomicBool::new(false) }
    }

    pub fn failure(&self) -> Option<String> {
        self.failure.lock().unwrap().clone()
    }

    /// Adds `by` to the elapsed time the limit is compared with (tests only).
    pub fn advance_elapsed(&self, by: Wall) {
        self.skew_ns.fetch_add(u64::try_from(by.as_nanos()).unwrap_or(u64::MAX), Ordering::AcqRel);
    }

    fn elapsed(&self) -> Wall {
        self.started.elapsed() + Wall::from_nanos(self.skew_ns.load(Ordering::Acquire))
    }

    /// Records the first failure only.
    pub fn fail(&self, reason: impl Into<String>) {
        let mut failure = self.failure.lock().unwrap();
        if failure.is_none() { *failure = Some(reason.into()); }
        self.stopped.store(true, Ordering::Release);
    }

    /// Whether the group has stopped; the limit is checked here, before each call.
    pub fn halted(&self) -> bool {
        if !self.stopped.load(Ordering::Acquire) && self.elapsed() >= self.limit {
            self.fail(format!("the raw group passed its {} ms wall-clock limit", self.limit.as_millis()));
        }
        self.stopped.load(Ordering::Acquire)
    }

    fn unmeasured(&self, key: Key, what: &str) -> Option<ValueRecord> {
        self.failure().map(|failure| ValueRecord::Unmeasured {
            key, reason: format!("Phase 8 stage 3c-2c stopped the raw group before {what} was complete: {failure}."),
        })
    }
}

/// The receive stream's progress as the drain thread sees it: the end of the samples that came
/// back, and what else `rx_recv` returned. A DDC setting leaves its FIFO only with a sample whose
/// time reaches it (INFERRED from UHD's `axi_tag_time.v:206-211`), so the end of the samples is
/// the evidence that the settings drained.
#[derive(Default)]
struct RxProgress {
    state: Mutex<RxState>,
}

#[derive(Default)]
struct RxState {
    first_tick: Option<i64>,
    end_tick: Option<i64>,
    counts: BTreeMap<&'static str, u64>,
}

impl RxProgress {
    fn note(&self, kind: &'static str) {
        *self.state.lock().unwrap().counts.entry(kind).or_default() += 1;
    }

    fn advance(&self, first_tick: i64, end_tick: i64) {
        let mut state = self.state.lock().unwrap();
        state.first_tick.get_or_insert(first_tick);
        if state.end_tick.is_none_or(|end| end_tick > end) { state.end_tick = Some(end_tick); }
        *state.counts.entry("samples").or_default() += 1;
    }

    fn end_tick(&self) -> Option<i64> {
        self.state.lock().unwrap().end_tick
    }

    fn snapshot(&self) -> Json {
        let state = self.state.lock().unwrap();
        json!({ "first_sample_tick": state.first_tick, "end_tick": state.end_tick, "counts": state.counts })
    }
}

/// Reads the receive stream until `stop` or the group stops, recording its progress (stage 3c-2c).
fn drain_receiver(device: &dyn Device, raw: &RawGroup, stop: &AtomicBool, progress: &RxProgress, ticks_per_sample: i64) {
    while !stop.load(Ordering::Acquire) && !raw.halted() {
        match device.rx_recv(0, 1_000, Wall::from_millis(100)) {
            RxRecv::Samples { first_tick, samples } => {
                let count = samples.first().map_or(0, Vec::len) as i64;
                progress.advance(first_tick, first_tick.saturating_add(count.saturating_mul(ticks_per_sample)));
            }
            RxRecv::Overflow { .. } => progress.note("overflow"),
            RxRecv::LateCommand => progress.note("late_command"),
            RxRecv::Alignment => progress.note("alignment"),
            RxRecv::BadPacket => progress.note("bad_packet"),
            RxRecv::Timeout => progress.note("timeout"),
            RxRecv::Failed(error) => {
                raw.fail(format!("rx_recv: {}", error.message));
                break;
            }
        }
    }
}

/// Sets `stop` when dropped, also while unwinding, so a panic on the command thread cannot leave the drain thread running.
struct StopOnDrop<'a>(&'a AtomicBool);

impl Drop for StopOnDrop<'_> {
    fn drop(&mut self) { self.0.store(true, Ordering::Release); }
}


fn wait_for_device_tick(device: &dyn Device, raw: &RawGroup, target: i64, timeout: Wall) -> Result<(), String> {
    let start = Instant::now();
    loop {
        if raw.halted() { return Err(raw.failure().unwrap_or_default()); }
        if device.time_now().map_err(|error| error.message)? >= target { return Ok(()); }
        if start.elapsed() >= timeout { return Err(format!("device time did not reach {target} before the wait limit")); }
        std::thread::sleep(Wall::from_millis(1));
    }
}

/// Waits for the end of the received samples to reach `target`: the DDC's settings are out of its
/// FIFO only once a sample's time has passed them.
fn wait_for_rx_end(raw: &RawGroup, progress: &RxProgress, target: i64, timeout: Wall) -> Result<(), String> {
    let start = Instant::now();
    loop {
        if raw.halted() { return Err(raw.failure().unwrap_or_default()); }
        let end = progress.end_tick();
        if end.is_some_and(|end| end >= target) { return Ok(()); }
        if start.elapsed() >= timeout {
            return Err(format!("the received samples ended at {end:?}, short of the last command's {target}, within the wait limit; DDC settings may remain queued"));
        }
        std::thread::sleep(Wall::from_millis(1));
    }
}

/// Waits for the first samples of a started receive stream, before any setting is submitted.
fn wait_for_rx_samples(raw: &RawGroup, progress: &RxProgress, timeout: Wall) -> Result<(), String> {
    let start = Instant::now();
    loop {
        if raw.halted() { return Err(raw.failure().unwrap_or_default()); }
        if progress.end_tick().is_some() { return Ok(()); }
        if start.elapsed() >= timeout { return Err(format!("the started receive stream returned no samples ({})", progress.snapshot()["counts"])); }
        std::thread::sleep(Wall::from_millis(1));
    }
}

/// The trial's result when the group has already stopped, before the next device call.
fn queue_stopped(trial: usize, raw: &RawGroup) -> Option<(QueueTrialClass, Json)> {
    if !raw.halted() { return None; }
    let reason = raw.failure().unwrap_or_default();
    Some((QueueTrialClass::DeviceFailure { reason: reason.clone() }, json!({ "trial": trial, "classification": "device_failure", "error": reason })))
}

/// A trial that failed on the device: the group stops with it.
fn queue_failed(trial: usize, raw: &RawGroup, label: &str, reason: String) -> (QueueTrialClass, Json) {
    raw.fail(format!("queue condition {label}: {reason}"));
    (QueueTrialClass::DeviceFailure { reason: reason.clone() }, json!({ "trial": trial, "classification": "device_failure", "error": reason }))
}

/// One trial: a streaming condition runs it with the receiver open and drained on another thread,
/// so DDC commands leave the FIFO as the sample time passes them (INFERRED from UHD's
/// `axi_tag_time.v`, which releases a timed setting only on a sample whose time reaches it). No
/// setting goes out before the receiver has returned samples, and the receiver is not stopped
/// before the samples have passed the last command.
fn queue_trial(
    device: &dyn Device,
    operations: &[QueueOperation],
    plan: &MeasurementPlan,
    trial: usize,
    label: &str,
    raw: &RawGroup,
) -> Result<(QueueTrialClass, Json), String> {
    if !queue_streams_rx(operations) { return queue_trial_calls(device, operations, plan, trial, label, raw, None); }
    let fail = |reason: String| Ok(queue_failed(trial, raw, label, reason));
    let master = device.master_clock_rate();
    let ticks_per_sample = i64::try_from(master / 1_000_000).map_err(|_| "ticks per sample overflow")?.max(1);
    if let Some(stopped) = queue_stopped(trial, raw) { return Ok(stopped); }
    if let Err(error) = device.apply(Dir::Rx, 0, &Settings { rate: Some(1e6), freq: Some(1e9), gain: Some(0.0), ..Settings::default() }, None) {
        return fail(error.message);
    }
    if let Some(stopped) = queue_stopped(trial, raw) { return Ok(stopped); }
    if let Err(error) = device.rx_open(0, &[0]) { return fail(error.message); }
    if let Some(stopped) = queue_stopped(trial, raw) { return Ok(stopped); }
    let start = match device.time_now() {
        Ok(now) => now.checked_add(ticks_for_ns(QUEUE_RX_START_LEAD_NS, master)?).ok_or("queue receive start overflow")?,
        Err(error) => return fail(error.message),
    };
    if let Some(stopped) = queue_stopped(trial, raw) { return Ok(stopped); }
    if let Err(error) = device.rx_start(0, start) { return fail(error.message); }
    let stop = AtomicBool::new(false);
    let progress = RxProgress::default();
    let outcome = std::thread::scope(|scope| {
        let _cancel = StopOnDrop(&stop);
        scope.spawn(|| drain_receiver(device, raw, &stop, &progress, ticks_per_sample));
        if let Err(reason) = wait_for_rx_samples(raw, &progress, Wall::from_millis(QUEUE_RX_READY_LIMIT_MS)) {
            return Ok(queue_failed(trial, raw, label, reason));
        }
        queue_trial_calls(device, operations, plan, trial, label, raw, Some(&progress))
    });
    let (mut class, mut row) = outcome?;
    // A failed trial sends nothing more; a good one stops the receiver and frees it, and a failed
    // stop is not followed by the close.
    if !raw.halted() {
        match device.rx_stop(0, None) {
            // The deadline can pass during the stop: the close is a new call and is looked at too.
            Ok(()) => if !raw.halted() { device.rx_close(0) },
            Err(error) => {
                let (failed, failed_row) = queue_failed(trial, raw, label, format!("rx_stop: {}", error.message));
                row["classification"] = failed_row["classification"].clone();
                row["reason"] = json!(format!("rx_stop: {}", error.message));
                class = failed;
            }
        }
    }
    row["rx_streaming"] = json!(true);
    row["rx_progress"] = progress.snapshot();
    Ok((class, row))
}

fn queue_trial_calls(
    device: &dyn Device,
    operations: &[QueueOperation],
    plan: &MeasurementPlan,
    trial: usize,
    label: &str,
    raw: &RawGroup,
    rx: Option<&RxProgress>,
) -> Result<(QueueTrialClass, Json), String> {
    let ahead_ticks = ticks_for_ns(plan.queue_ahead_ns, device.master_clock_rate())?;
    let spacing_ticks = ticks_for_ns(QUEUE_SPACING_NS, device.master_clock_rate())?;
    let margin_ticks = ticks_for_ns(QUEUE_DRAIN_MARGIN_NS, device.master_clock_rate())?;
    let return_limit_ticks = ticks_for_ns(QUEUE_CALL_TIME_LIMIT_NS, device.master_clock_rate())?;
    if let Some(stopped) = queue_stopped(trial, raw) { return Ok(stopped); }
    let h_before = Instant::now();
    let d0 = match device.time_now() {
        Ok(now) => now,
        Err(error) => return Ok(queue_failed(trial, raw, label, error.message)),
    };
    let h_after = Instant::now();
    let clock = QueueClock {
        d0_tick: d0,
        h_before_ns: 0,
        h_after_ns: u64::try_from(h_after.duration_since(h_before).as_nanos()).unwrap_or(u64::MAX),
    };
    let first_at = match d0.checked_add(ahead_ticks) {
        Some(at) => at,
        None => return Ok(queue_failed(trial, raw, label, "queue target tick overflow".to_owned())),
    };
    let stall_limit_ns = plan.queue_ahead_ns / 2;
    if device.channels(Dir::Tx) == 0 {
        return Ok(queue_failed(trial, raw, label, "queue probe requires transmit channel 0".to_owned()));
    }
    let master_clock_hz = device.master_clock_rate();
    let mut calls = Vec::new();
    let mut rows = Vec::new();
    let mut last_accepted_at = None;
    let mut operational_error: Option<String> = None;
    let read_duration_ns = clock.h_after_ns - clock.h_before_ns;
    if read_duration_ns <= QUEUE_CALL_TIME_LIMIT_NS {
        if let Some(stopped) = queue_stopped(trial, raw) { return Ok(stopped); }
        if let Err(error) = device.apply(Dir::Tx, 0, &Settings { freq: Some(1_000_000_000.0), ..Settings::default() }, None) {
            return Ok(queue_failed(trial, raw, label, error.message));
        }
        for index in 0..plan.queue_max_calls {
            // The deadline and the first failure are looked at before every call.
            if raw.halted() {
                operational_error = raw.failure();
                break;
            }
            let index = i64::try_from(index).map_err(|_| "queue command count exceeds i64")?;
            let at = first_at.checked_add(spacing_ticks.checked_mul(index).ok_or("queue command spacing overflow")?)
                .ok_or("queue command time overflow")?;
            let operation = &operations[index as usize % operations.len()];
            if operation.dir == Dir::Tx && operation.settings.freq.is_some_and(|frequency| !SAFE_TX_HZ.contains(&frequency)) {
                return Err("queue probe requested a TX frequency outside SAFE_TX_HZ".to_owned());
            }
            let started = Instant::now();
            let applied = device.apply(operation.dir, 0, &operation.settings, Some(at));
            let h_i = Instant::now();
            let duration_ns = u64::try_from(h_i.duration_since(started).as_nanos()).unwrap_or(u64::MAX);
            let h_i_ns = u64::try_from(h_i.duration_since(h_before).as_nanos()).unwrap_or(u64::MAX);
            let apply_error = applied.as_ref().err().map(|error| error.message.clone());
            let error = apply_error.clone();
            if applied.is_ok() { last_accepted_at = Some(at); }
            let bounds = queue_device_time_bounds(clock, h_i_ns, master_clock_hz);
            calls.push(QueueCall { duration_ns, h_i_ns: Some(h_i_ns), error: error.clone() });
            rows.push(json!({
                "number": index + 1,
                "direction": operation.dir.name(),
                "scheduled_tick": at,
                "duration_ns": duration_ns,
                "h_i_ns": h_i_ns,
                "device_time_lower_tick": bounds.map(|(lower, _)| lower),
                "device_time_upper_tick": bounds.map(|(_, upper)| upper),
                "apply_error": apply_error,
            }));
            if let Some(error) = error {
                raw.fail(format!("queue condition {label}: {error}"));
                operational_error = Some(error);
                break;
            }
            if duration_ns > stall_limit_ns { break; }
        }
    }
    let queue_class = classify_queue_trial(&calls, clock, first_at, stall_limit_ns, return_limit_ticks, master_clock_hz);
    // After an error nothing more goes to the handle: no clock read, no untimed reset.
    if let Some(last_at) = last_accepted_at.filter(|_| operational_error.is_none()) {
        let target = last_at.checked_add(margin_ticks).ok_or("queue drain time overflow")?;
        let timeout = Wall::from_nanos(plan.queue_ahead_ns.checked_add(QUEUE_DRAIN_MARGIN_NS + 2_000_000_000).ok_or("queue drain timeout overflow")?);
        let waited = match rx {
            // The received samples must pass the last command (their ticks are device ticks).
            Some(progress) => wait_for_rx_end(raw, progress, target, timeout),
            None => wait_for_device_tick(device, raw, target, timeout),
        };
        if let Err(error) = waited {
            raw.fail(format!("queue condition {label}: {error}"));
            operational_error = Some(error);
        }
    }
    let reset_dir = operations.first().map_or(Dir::Rx, |operation| operation.dir);
    if operational_error.is_none() && raw.halted() { operational_error = raw.failure(); }
    let untimed_error = if operational_error.is_some() { None } else {
        let error = device.apply(reset_dir, 0, &Settings { freq: Some(1_000_000_000.0), ..Settings::default() }, None).err().map(|error| error.message);
        if let Some(error) = &error { raw.fail(format!("queue condition {label}: {error}")); }
        error
    };
    let class = finish_queue_trial(queue_class, operational_error, untimed_error.clone());
    let depth = match &class { QueueTrialClass::Success { depth } => Some(*depth), _ => None };
    let class_name = match &class {
        QueueTrialClass::Success { .. } => "success",
        QueueTrialClass::Incomplete { .. } => "incomplete",
        QueueTrialClass::DeviceFailure { .. } => "device_failure",
    };
    let class_reason = match &class {
        QueueTrialClass::Incomplete { reason } => Some((*reason).to_owned()),
        QueueTrialClass::DeviceFailure { reason } => Some(reason.clone()),
        QueueTrialClass::Success { .. } => None,
    };
    Ok((class, json!({
        "trial": trial,
        "d0_tick": clock.d0_tick,
        "h_before_ns": clock.h_before_ns,
        "h_after_ns": clock.h_after_ns,
        "h_after_minus_h_before_ns": read_duration_ns,
        "first_command_tick": first_at,
        "classification": class_name,
        "reason": class_reason,
        "depth": depth,
        "calls": rows,
        "untimed_apply_error": untimed_error,
    })))
}

fn summarize_queue_depth(conditions: &[(String, Vec<QueueTrialClass>)], required_trials: usize) -> Option<(usize, String, usize)> {
    if conditions.iter().any(|(_, trials)| trials.iter().filter(|trial| matches!(trial, QueueTrialClass::Success { .. })).count() < required_trials) {
        return None;
    }
    conditions.iter().flat_map(|(condition, trials)| trials.iter().filter_map(move |trial| match trial {
        QueueTrialClass::Success { depth } => Some((*depth, condition.clone(), trials.iter().filter(|row| matches!(row, QueueTrialClass::Success { .. })).count())),
        QueueTrialClass::Incomplete { .. } | QueueTrialClass::DeviceFailure { .. } => None,
    })).min_by_key(|(depth, _, _)| *depth)
}

fn queue_depth_value(conditions: &[(String, Vec<QueueTrialClass>)], required_trials: usize) -> Result<ValueRecord, String> {
    let key = Key::parse("radio.timing.command_queue_depth").map_err(|error| error.to_string())?;
    Ok(match summarize_queue_depth(conditions, required_trials) {
        Some((depth, condition, trials)) => ValueRecord::Measured {
            key, worst: model::Worst::Number(depth as f64), trials: observation_trials(trials)?, condition,
            test: QUEUE_TEST.to_owned(),
        },
        None => ValueRecord::Unmeasured {
            key,
            reason: format!("Phase 8 stage 3c-2a did not obtain {required_trials} successful back-pressure trials in every queue condition."),
        },
    })
}

/// Measures per-condition timed-apply queue depth on the raw device interface. The value is the
/// least depth over the conditions measured. The first device error or the group's time limit
/// stops the group (`raw`): the value is `unmeasured` and the handle gets nothing more.
pub fn measure_command_queue_depth(device: Arc<dyn Device>, plan: &MeasurementPlan, raw: &RawGroup) -> Result<MeasurementGroup, String> {
    plan.validate()?;
    let mut group = MeasurementGroup::default();
    let mut condition_trials = Vec::new();
    let mut observed_conditions = Vec::new();
    let mut total_trials = 0usize;
    'conditions: for (condition, operations) in queue_conditions() {
        let mut classes = Vec::new();
        let mut trials = Vec::new();
        for trial in 0..plan.queue_trials {
            if raw.halted() { break; }
            let (class, row) = queue_trial(&*device, &operations, plan, trial + 1, &condition.id, raw)?;
            classes.push(class);
            trials.push(row);
            total_trials += 1;
        }
        if !trials.is_empty() || raw.failure().is_none() {
            observed_conditions.push(json!({ "id": condition.id.clone(), "trials": trials }));
            condition_trials.push((condition.id.clone(), classes));
            group.conditions.push(condition);
        }
        if raw.failure().is_some() { break 'conditions; }
    }
    group.values.push(match raw.unmeasured(Key::parse("radio.timing.command_queue_depth").map_err(|error| error.to_string())?, "the queue-depth trials") {
        Some(value) => value,
        None => queue_depth_value(&condition_trials, plan.queue_trials)?,
    });
    group.observations.push(observation("raw_command_queue_depth", QUEUE_TEST, None, observation_trials(total_trials.max(1))?, json!({
        "ahead_ns": plan.queue_ahead_ns,
        "spacing_ns": QUEUE_SPACING_NS,
        "stall_limit_ns": plan.queue_ahead_ns / 2,
        "call_time_limit_ns": QUEUE_CALL_TIME_LIMIT_NS,
        "host_time_origin": "h_before_ns; all host timestamps are offsets from h_before",
        "clock_rate_assumption": "device and host clocks advance at the same rate; drift is ignored (INFERRED)",
        "device_time_bounds": "lower=d0+(h_i-h_after); upper=d0+(h_i-h_before)",
        "depth_unit": "timed_apply_calls",
        "depth_definition": "the count of timed Device::apply calls (high-level operations), not device FIFO entries",
        "back_pressure_attribution": "INFERRED",
        "back_pressure_basis": "host apply duration and device time do not directly prove credit wait",
        "value_rule": "the least depth over the conditions measured; the condition is the one that gave it",
        "rx_frequency_streaming": "queue-rx-frequency opens the receiver, starts it, and drains it on another thread until after the last command's time; INFERRED that a DDC timed command leaves its FIFO only with a sample whose time reaches it (UHD axi_tag_time.v:206-211)",
        "excluded_conditions": QUEUE_EXCLUDED_CONDITIONS.iter().map(|(id, reason)| json!({ "id": id, "reason": reason })).collect::<Vec<_>>(),
        "after_a_failure": "once a failure or the limit latches, the other thread begins at most one more device call (its latch check came before the latch), which ends by UHD's 10 s timeout, and nothing after it; the two threads' calls are not serialised",
        "halted": raw.failure(),
        "power_cycle_note": raw.failure().map(|_| "a wedged DDC or DUC control port needs a power cycle of the device; the group sent nothing more after the first error"),
        "max_calls": plan.queue_max_calls,
        "conditions": observed_conditions,
    }))?);
    Ok(group)
}

#[derive(Clone, Copy, Debug)]
enum NumericSetting {
    Rate,
    Frequency,
    Gain,
}

#[derive(Clone, Debug)]
struct NumericReadback {
    requested: f64,
    returned: Option<f64>,
    error: Option<String>,
}

#[derive(Clone, Debug)]
struct AntennaReadback {
    requested: String,
    returned: Option<String>,
    error: Option<String>,
}

fn apply_numeric(
    device: &dyn Device,
    dir: Dir,
    channel: usize,
    setting: NumericSetting,
    requested: f64,
) -> NumericReadback {
    let settings = match setting {
        NumericSetting::Rate => Settings { rate: Some(requested), ..Settings::default() },
        NumericSetting::Frequency => Settings { freq: Some(requested), ..Settings::default() },
        NumericSetting::Gain => Settings { gain: Some(requested), ..Settings::default() },
    };
    match device.apply(dir, channel, &settings, None) {
        Ok(applied) => {
            let returned = match setting {
                NumericSetting::Rate => applied.rate,
                NumericSetting::Frequency => applied.freq,
                NumericSetting::Gain => applied.gain,
            };
            if returned.is_finite() {
                NumericReadback { requested, returned: Some(returned), error: None }
            } else {
                NumericReadback { requested, returned: None, error: Some(format!("non-finite {setting:?} read-back")) }
            }
        }
        Err(error) => NumericReadback { requested, returned: None, error: Some(error.message) },
    }
}

fn apply_antenna(device: &dyn Device, dir: Dir, channel: usize, requested: &str) -> AntennaReadback {
    match device.apply(dir, channel, &Settings { antenna: Some(requested.to_owned()), ..Settings::default() }, None) {
        Ok(applied) => AntennaReadback { requested: requested.to_owned(), returned: applied.antenna, error: None },
        Err(error) => AntennaReadback { requested: requested.to_owned(), returned: None, error: Some(error.message) },
    }
}

fn grid_points(grid: &Grid) -> Result<Vec<f64>, String> {
    match grid {
        Grid::Values(values) if !values.is_empty() && values.iter().all(|value| value.is_finite())
            && values.windows(2).all(|pair| pair[0] < pair[1]) => Ok(values.clone()),
        Grid::Values(_) => Err("profile Values grid is empty, non-finite or not ascending".to_owned()),
        Grid::Integer { lo, hi } if lo <= hi => Ok((*lo..=*hi).map(|value| value as f64).collect()),
        Grid::Integer { .. } => Err("profile Integer grid has lo > hi".to_owned()),
        Grid::Step { lo, hi, step } if lo.is_finite() && hi.is_finite() && step.is_finite() && lo <= hi && *step > 0.0 => {
            let mut points = Vec::new();
            let mut value = grid.at_or_above(*lo).ok_or("profile Step grid has no first point")?;
            while value <= *hi {
                points.push(value);
                let next = value + step;
                if next <= value { return Err("profile Step grid did not advance".to_owned()); }
                value = next;
            }
            if points.is_empty() { Err("profile Step grid has no points".to_owned()) } else { Ok(points) }
        }
        Grid::Step { .. } => Err("profile Step grid is not enumerable".to_owned()),
    }
}

fn fine_gain_points(grid: &Grid) -> Result<Vec<f64>, String> {
    let step = grid.step();
    if !step.is_finite() || step <= 0.0 || !grid.min().is_finite() || !grid.max().is_finite() {
        return Err("profile gain grid has no finite positive step".to_owned());
    }
    let lo = grid.min();
    let hi = grid.max();
    let spacing = step / 5.0;
    let intervals = ((hi - lo) / spacing).floor();
    let count = usize::try_from(intervals as u64).map_err(|_| "gain sweep has too many points")?;
    let mut points: Vec<_> = (0..=count).map(|index| {
        let value = lo + index as f64 * step / 5.0;
        (value * 1_000_000_000.0).round() / 1_000_000_000.0
    }).collect();
    if points.last().is_none_or(|last| *last < hi) { points.push((hi * 1_000_000_000.0).round() / 1_000_000_000.0); }
    Ok(points)
}

/// `count` frequency requests over the grid: both ends exactly, and between them positions
/// that alternate between a whole MHz and an off-MHz point (stage 3c-2c).
fn frequency_sweep_points(grid: &Grid, count: usize) -> Result<Vec<f64>, String> {
    let (lo, hi) = (grid.min(), grid.max());
    if count < 4 || !lo.is_finite() || !hi.is_finite() || lo >= hi {
        return Err("profile frequency grid cannot produce the requested sweep".to_owned());
    }
    let mut points = vec![lo];
    for index in 1..count - 1 {
        let target = lo + (hi - lo) * index as f64 / (count - 1) as f64;
        let wanted = if index % 2 == 0 { (target / 1e6).round() * 1e6 } else { (target / 1e6).floor() * 1e6 + 370_000.0 };
        let point = grid.nearest(wanted.clamp(lo, hi)).ok_or_else(|| format!("profile frequency grid has no value near {wanted}"))?;
        if point > points[points.len() - 1] && point < hi { points.push(point); }
    }
    points.push(hi);
    Ok(points)
}

const FREQUENCY_READBACK_TOLERANCE_HZ: f64 = 0.05;
const FREQUENCY_STEP_HZ: f64 = 1.0;

/// `frequency_hz` and `frequency_step_hz` from one sweep: a value only if every request was read
/// back within 0.05 Hz. The points between those sampled are INFERRED (stage 3c-2c).
fn frequency_values(
    keys: [&str; 2], points: &[NumericReadback], requests: &[f64], condition: &str, test: &str,
) -> Result<[ValueRecord; 2], String> {
    let parse = |key: &str| Key::parse(key).map_err(|error| error.to_string());
    let unmeasured = |reason: String| -> Result<[ValueRecord; 2], String> {
        Ok([
            ValueRecord::Unmeasured { key: parse(keys[0])?, reason: reason.clone() },
            ValueRecord::Unmeasured { key: parse(keys[1])?, reason },
        ])
    };
    if let Some(error) = numeric_readback_error(points) {
        return unmeasured(format!("Phase 8 stage 3c-2c frequency read-back failed: {error}."));
    }
    if points.len() != requests.len() {
        return unmeasured(format!("Phase 8 stage 3c-2c frequency sweep stopped after {} of {} requests.", points.len(), requests.len()));
    }
    let mut worst = 0.0_f64;
    for point in points {
        let Some(returned) = point.returned else { return unmeasured(format!("Phase 8 stage 3c-2c frequency request {} had no read-back.", point.requested)); };
        let difference = (returned - point.requested).abs();
        if difference > FREQUENCY_READBACK_TOLERANCE_HZ {
            return unmeasured(format!("Phase 8 stage 3c-2c frequency request {} read back as {returned}, {difference} Hz away (limit {FREQUENCY_READBACK_TOLERANCE_HZ} Hz).", point.requested));
        }
        worst = worst.max(difference);
    }
    let (lo, hi) = (requests[0], requests[requests.len() - 1]);
    let trials = observation_trials(points.len())?;
    Ok([
        ValueRecord::Measured {
            key: parse(keys[0])?, worst: model::Worst::Grid(model::GridValue::Step { lo, hi, step: FREQUENCY_STEP_HZ }),
            trials, condition: condition.to_owned(), test: test.to_owned(),
        },
        ValueRecord::Measured {
            key: parse(keys[1])?, worst: model::Worst::Number(worst * 2.0),
            trials, condition: condition.to_owned(), test: test.to_owned(),
        },
    ])
}

fn observation_number(value: f64) -> f64 {
    if value == 0.0 { return value; }
    let exponent = value.abs().log10().floor() as i32;
    let scale = 10.0_f64.powi(12 - exponent);
    (value * scale).round() / scale
}

fn numeric_points_json(points: &[NumericReadback]) -> Json {
    // Keep observed decimals stable through serde_json's f64 parse/serialize round trip.
    json!(points.iter().map(|point| json!({
        "requested": observation_number(point.requested),
        "readback": point.returned.map(observation_number),
        "error": point.error.as_deref(),
    })).collect::<Vec<_>>())
}

fn antenna_points_json(points: &[AntennaReadback]) -> Json {
    json!(points.iter().map(|point| json!({
        "requested": point.requested,
        "readback": point.returned.as_deref(),
        "error": point.error.as_deref(),
    })).collect::<Vec<_>>())
}

fn gain_readback_matches(requested: f64, returned: f64) -> bool {
    (requested - returned).abs() <= 1e-9
}

fn rate_readback_matches(requested: f64, returned: f64) -> bool {
    (requested - returned).abs() <= requested.abs().max(returned.abs()) * 1e-12
}

fn numeric_readback_error(points: &[NumericReadback]) -> Option<String> {
    points.iter().find_map(|point| point.error.as_ref().map(|error| format!("request {}: {error}", point.requested)))
}

fn coercion_grid_value(
    key: &str,
    points: &[NumericReadback],
    condition: &str,
    test: &str,
    matches: impl Fn(f64, f64) -> bool,
) -> Result<ValueRecord, String> {
    let key = Key::parse(key).map_err(|error| error.to_string())?;
    if let Some(error) = numeric_readback_error(points) {
        return Ok(ValueRecord::Unmeasured { key, reason: format!("Phase 8 stage 3c-2a read-back failed: {error}.") });
    }
    let values: Vec<_> = points.iter().filter_map(|point| point.returned.filter(|actual| matches(point.requested, *actual)).map(|_| point.requested)).collect();
    if values.is_empty() {
        return Ok(ValueRecord::Unmeasured { key, reason: "Phase 8 stage 3c-2a had no request read back at its requested value.".to_owned() });
    }
    Ok(ValueRecord::Measured {
        key, worst: model::Worst::Grid(model::GridValue::Values { values }),
        trials: observation_trials(points.len())?, condition: condition.to_owned(), test: test.to_owned(),
    })
}

fn gain_step_value(key: &str, points: &[NumericReadback], condition: &str, test: &str) -> Result<ValueRecord, String> {
    let key = Key::parse(key).map_err(|error| error.to_string())?;
    if let Some(error) = numeric_readback_error(points) {
        return Ok(ValueRecord::Unmeasured { key, reason: format!("Phase 8 stage 3c-2a read-back failed: {error}.") });
    }
    let mut values: Vec<_> = points.iter().filter_map(|point| point.returned).collect();
    values.sort_by(f64::total_cmp);
    values.dedup_by(|left, right| (*left - *right).abs() <= 1e-9);
    let Some(step) = values.windows(2).map(|pair| pair[1] - pair[0]).reduce(f64::max) else {
        return Ok(ValueRecord::Unmeasured { key, reason: "Phase 8 stage 3c-2a gain sweep returned fewer than two distinct values.".to_owned() });
    };
    Ok(ValueRecord::Measured {
        key, worst: model::Worst::Number(step), trials: observation_trials(points.len())?,
        condition: condition.to_owned(), test: test.to_owned(),
    })
}

fn antenna_value(key: &str, points: &[AntennaReadback], condition: &str, test: &str) -> Result<ValueRecord, String> {
    let key = Key::parse(key).map_err(|error| error.to_string())?;
    if let Some(error) = points.iter().find_map(|point| point.error.as_ref().map(|error| format!("request {}: {error}", point.requested))) {
        return Ok(ValueRecord::Unmeasured { key, reason: format!("Phase 8 stage 3c-2a read-back failed: {error}.") });
    }
    if points.iter().any(|point| point.returned.is_none()) {
        return Ok(ValueRecord::Unmeasured { key, reason: "Phase 8 stage 3c-2a returned no antenna read-back.".to_owned() });
    }
    let values = points.iter().filter(|point| point.returned.as_deref() == Some(point.requested.as_str())).map(|point| point.requested.clone()).collect();
    Ok(ValueRecord::Measured {
        key, worst: model::Worst::Texts(values), trials: observation_trials(points.len())?,
        condition: condition.to_owned(), test: test.to_owned(),
    })
}

/// The requests in turn, each one an untimed `apply`; the first error, or the group's limit, ends the sweep.
fn sweep_numeric(device: &dyn Device, raw: &RawGroup, dir: Dir, channel: usize, setting: NumericSetting, requests: &[f64]) -> Vec<NumericReadback> {
    let mut points = Vec::new();
    for requested in requests {
        if raw.halted() { break; }
        let point = apply_numeric(device, dir, channel, setting, *requested);
        if let Some(error) = &point.error { raw.fail(format!("{} channel {channel} {setting:?} request {requested}: {error}", dir.name())); }
        points.push(point);
    }
    points
}

fn sweep_antenna(device: &dyn Device, raw: &RawGroup, dir: Dir, channel: usize, requests: &[String]) -> Vec<AntennaReadback> {
    let mut points = Vec::new();
    for requested in requests {
        if raw.halted() { break; }
        let point = apply_antenna(device, dir, channel, requested);
        if let Some(error) = &point.error { raw.fail(format!("{} channel {channel} antenna request {requested}: {error}", dir.name())); }
        points.push(point);
    }
    points
}

fn raw_condition(id: &str) -> Condition {
    Condition { id: id.to_owned(), streams: vec![], concurrent: vec![], reopened: false }
}

fn coercion_observation(name: String, condition: &str, result: Json, trials: usize) -> Result<Observation, String> {
    observation(name, QUEUE_TEST, Some(condition.to_owned()), observation_trials(trials.max(1))?, result)
}

fn key_name(direction: Dir, channel: usize, field: &str) -> String {
    format!("radio.{}.c{channel}.{field}", direction.name())
}

/// Measures the profile's gain, antenna, frequency and sample-rate coercions by untimed `apply`
/// and read-back. No streamer is opened. The first device error or the group's limit (`raw`) ends
/// every later call; the keys that were not finished are `unmeasured`.
pub fn measure_coercion(device: Arc<dyn Device>, description: &channels::Description, plan: &MeasurementPlan, raw: &RawGroup) -> Result<MeasurementGroup, String> {
    let condition = "raw-device-coercion";
    let mut group = MeasurementGroup { conditions: vec![raw_condition(condition)], ..MeasurementGroup::default() };
    let test = QUEUE_TEST;
    for (dir, channels) in [(Dir::Rx, &description.rx), (Dir::Tx, &description.tx)] {
        for (channel_index, channel) in channels.iter().enumerate() {
            // The transmitter sits at its safe 1 GHz while the gain and antenna sweeps run.
            if dir == Dir::Tx { sweep_numeric(&*device, raw, dir, channel_index, NumericSetting::Frequency, &[1_000_000_000.0]); }

            let gain_key = key_name(dir, channel_index, "gain_db");
            match grid_points(&channel.gain) {
                Ok(requests) => {
                    let points = sweep_numeric(&*device, raw, dir, channel_index, NumericSetting::Gain, &requests);
                    group.values.push(match raw.unmeasured(Key::parse(&gain_key).map_err(|parse| parse.to_string())?, "the gain sweep") {
                        Some(value) => value,
                        None => coercion_grid_value(&gain_key, &points, condition, test, gain_readback_matches)?,
                    });
                    group.observations.push(coercion_observation(format!("raw_coercion_{}_c{}_gain_db", dir.name(), channel_index), condition, json!({ "points": numeric_points_json(&points) }), points.len())?);
                }
                Err(error) => {
                    let key = Key::parse(&gain_key).map_err(|parse| parse.to_string())?;
                    group.values.push(ValueRecord::Unmeasured { key, reason: format!("Phase 8 stage 3c-2a could not enumerate the gain grid: {error}.") });
                    group.observations.push(coercion_observation(format!("raw_coercion_{}_c{}_gain_db", dir.name(), channel_index), condition, json!({ "error": error, "points": [] }), 1)?);
                }
            }

            let gain_step_key = key_name(dir, channel_index, "gain_step_db");
            match fine_gain_points(&channel.gain) {
                Ok(requests) => {
                    let points = sweep_numeric(&*device, raw, dir, channel_index, NumericSetting::Gain, &requests);
                    group.values.push(match raw.unmeasured(Key::parse(&gain_step_key).map_err(|parse| parse.to_string())?, "the gain-step sweep") {
                        Some(value) => value,
                        None => gain_step_value(&gain_step_key, &points, condition, test)?,
                    });
                    group.observations.push(coercion_observation(format!("raw_coercion_{}_c{}_gain_step_db", dir.name(), channel_index), condition, json!({ "points": numeric_points_json(&points) }), points.len())?);
                }
                Err(error) => {
                    let key = Key::parse(&gain_step_key).map_err(|parse| parse.to_string())?;
                    group.values.push(ValueRecord::Unmeasured { key, reason: format!("Phase 8 stage 3c-2a could not sweep the gain grid: {error}.") });
                    group.observations.push(coercion_observation(format!("raw_coercion_{}_c{}_gain_step_db", dir.name(), channel_index), condition, json!({ "error": error, "points": [] }), 1)?);
                }
            }

            let antenna_key = key_name(dir, channel_index, "antenna");
            let antenna_points = sweep_antenna(&*device, raw, dir, channel_index, &channel.antennas);
            group.values.push(match raw.unmeasured(Key::parse(&antenna_key).map_err(|parse| parse.to_string())?, "the antenna sweep") {
                Some(value) => value,
                None => antenna_value(&antenna_key, &antenna_points, condition, test)?,
            });
            group.observations.push(coercion_observation(format!("raw_coercion_{}_c{}_antenna", dir.name(), channel_index), condition, json!({ "points": antenna_points_json(&antenna_points) }), antenna_points.len())?);

            // The whole profile grid, both ends and off-MHz points between them, by untimed apply. The
            // transmitter sends nothing (no streamer, gain 0) and is not held to SAFE_TX_HZ here: the
            // bench's attenuator and cables are rated to 18 GHz (owner, 2026-10-27; check them before running).
            let frequency_keys = [key_name(dir, channel_index, "frequency_hz"), key_name(dir, channel_index, "frequency_step_hz")];
            let frequency_name = format!("raw_coercion_{}_c{}_frequency_hz", dir.name(), channel_index);
            match frequency_sweep_points(&channel.frequency, plan.frequency_points) {
                Ok(requests) => {
                    if dir == Dir::Tx { sweep_numeric(&*device, raw, dir, channel_index, NumericSetting::Gain, &[0.0]); }
                    let points = sweep_numeric(&*device, raw, dir, channel_index, NumericSetting::Frequency, &requests);
                    if dir == Dir::Tx { sweep_numeric(&*device, raw, dir, channel_index, NumericSetting::Frequency, &[1_000_000_000.0]); }
                    let values = match raw.unmeasured(Key::parse(&frequency_keys[0]).map_err(|parse| parse.to_string())?, "the frequency sweep") {
                        Some(first) => [first, raw.unmeasured(Key::parse(&frequency_keys[1]).map_err(|parse| parse.to_string())?, "the frequency sweep").expect("the group has stopped")],
                        None => frequency_values([&frequency_keys[0], &frequency_keys[1]], &points, &requests, condition, test)?,
                    };
                    group.values.extend(values);
                    group.observations.push(coercion_observation(frequency_name, condition, json!({
                        "points": numeric_points_json(&points),
                        "tolerance_hz": FREQUENCY_READBACK_TOLERANCE_HZ,
                        "grid_between_points": "INFERRED: the points between those sampled are taken to read back as the first; the value's step is the profile's 1 Hz",
                    }), points.len())?);
                }
                Err(error) => {
                    for key in &frequency_keys {
                        group.values.push(ValueRecord::Unmeasured { key: Key::parse(key).map_err(|parse| parse.to_string())?, reason: format!("Phase 8 stage 3c-2c could not sweep the frequency grid: {error}.") });
                    }
                    group.observations.push(coercion_observation(frequency_name, condition, json!({ "error": error, "points": [] }), 1)?);
                }
            }

            // shortcut: Description has no stream membership, so rate probes use channel indices until a layout supplies it.
            let rate_key = format!("radio.{}.s{}.sample_rate_hz", dir.name(), channel_index);
            match grid_points(&description.rates) {
                Ok(requests) => {
                    let points = sweep_numeric(&*device, raw, dir, channel_index, NumericSetting::Rate, &requests);
                    group.values.push(match raw.unmeasured(Key::parse(&rate_key).map_err(|parse| parse.to_string())?, "the sample-rate sweep") {
                        Some(value) => value,
                        None => coercion_grid_value(&rate_key, &points, condition, test, rate_readback_matches)?,
                    });
                    group.observations.push(coercion_observation(format!("raw_coercion_{}_s{}_sample_rate_hz", dir.name(), channel_index), condition, json!({ "points": numeric_points_json(&points) }), points.len())?);
                }
                Err(error) => {
                    let key = Key::parse(&rate_key).map_err(|parse| parse.to_string())?;
                    group.values.push(ValueRecord::Unmeasured { key, reason: format!("Phase 8 stage 3c-2a could not enumerate the sample-rate grid: {error}.") });
                    group.observations.push(coercion_observation(format!("raw_coercion_{}_s{}_sample_rate_hz", dir.name(), channel_index), condition, json!({ "error": error, "points": [] }), 1)?);
                }
            }
        }
    }
    Ok(group)
}

/// Restores the unstreamed bench state after raw-device measurement: 1 Msps, 1 GHz, 0 dB,
/// and each channel's profile antenna. The first error stops it (`raw` latches it); a group that
/// has stopped restores nothing.
pub fn restore_measurement_defaults(device: &dyn Device, description: &channels::Description, raw: &RawGroup) -> Result<(), String> {
    for (dir, channel_list) in [(Dir::Rx, &description.rx), (Dir::Tx, &description.tx)] {
        for (channel_index, channel) in channel_list.iter().enumerate() {
            if raw.halted() { return Err(format!("the raw group stopped before restoring {dir:?} channel {channel_index}: {}", raw.failure().unwrap_or_default())); }
            if let Err(error) = device.apply(dir, channel_index, &Settings {
                rate: Some(channels::DEFAULT_SAMPLE_RATE_HZ), freq: Some(1_000_000_000.0), gain: Some(0.0),
                antenna: Some(channel.default_antenna.clone()),
            }, None) {
                let message = format!("could not restore {:?} channel {channel_index}: {}", dir, error.message);
                raw.fail(message.clone());
                return Err(message);
            }
        }
    }
    Ok(())
}

/// The raw group's state: the shared latch, and the restore that must run (a panic included)
/// unless the group stopped on a failure, after which the handle gets no more calls.
struct RawRun<'a> {
    device: &'a dyn Device,
    description: &'a channels::Description,
    raw: RawGroup,
    restored: bool,
}

impl Drop for RawRun<'_> {
    fn drop(&mut self) {
        if !self.restored { let _ = restore_measurement_defaults(self.device, self.description, &self.raw); }
    }
}

/// The raw group: queue depth, then coercion, then the restore of the unstreamed bench state.
/// Returns the group, why the group stopped if it did, and the restore's error: the last two are the
/// caller's to report once the record is written. After a stop the caller should leak the device
/// handle (`std::mem::forget`): its teardown would touch the control port.
pub fn measure_raw_group(device: Arc<dyn Device>, description: &channels::Description, plan: &MeasurementPlan) -> Result<(MeasurementGroup, Option<String>, Option<String>), String> {
    let mut run = RawRun { device: &*device, description, raw: RawGroup::new(plan), restored: false };
    let mut group = measure_command_queue_depth(device.clone(), plan, &run.raw)?;
    group.append(measure_coercion(device.clone(), description, plan, &run.raw)?)?;
    run.restored = true;
    let (status, error) = match run.raw.failure() {
        Some(failure) => ("skipped", Some(format!("the group stopped ({failure}); nothing was sent to the device afterwards, which may need a power cycle"))),
        None => match restore_measurement_defaults(&*device, description, &run.raw) {
            Ok(()) => ("restored", None),
            Err(error) => ("failed", Some(error)),
        },
    };
    // Read after the restore: a restore that failed latched the group too.
    let stopped = run.raw.failure();
    group.observations.push(observation("raw_restore", QUEUE_TEST, None, 1, json!({ "status": status, "detail": error }))?);
    Ok((group, stopped, error.filter(|_| status == "failed")))
}

/// The device handle of a record run. Once a group has failed, dropping it would run UHD's
/// teardown on a control port that failed, so a failed lease leaks the handle instead, also when
/// the run unwinds (a panic while the record is written).
pub struct DeviceLease {
    device: Option<Arc<dyn Device>>,
    failed: bool,
}

impl DeviceLease {
    pub fn new(device: Arc<dyn Device>) -> DeviceLease {
        // Leaked unless the run is marked clean: a panic anywhere before that point leaks it too.
        DeviceLease { device: Some(device), failed: true }
    }

    /// A clone for one use; do not keep it across the record's writing.
    pub fn device(&self) -> Arc<dyn Device> {
        self.device.clone().expect("the lease holds its device until dropped")
    }

    pub fn mark_clean(&mut self) {
        self.failed = false;
    }
}

impl Drop for DeviceLease {
    fn drop(&mut self) {
        // shortcut: the wedged handle is leaked so its teardown does not touch the control port; a power cycle is needed anyway.
        if self.failed { std::mem::forget(self.device.take()); }
    }
}

/// Runs the raw group as the record's last group, and marks the lease clean only when neither it,
/// the restore nor any earlier group failed. Returns why the group stopped (read after the restore) and the
/// restore's error.
pub fn run_raw_group_leased(lease: &mut DeviceLease, groups: &mut GroupRun, description: &channels::Description, plan: &MeasurementPlan) -> (Option<String>, Option<String>) {
    let (mut stopped, mut restored) = (None, None);
    let device = lease.device();
    groups.run("raw_queue_depth_and_coercion", &raw_group_keys(description), || {
        let (group, stop, restore_error) = measure_raw_group(device.clone(), description, plan)?;
        (stopped, restored) = (stop, restore_error);
        Ok(group)
    });
    drop(device);
    if stopped.is_none() && restored.is_none() && groups.failed().is_empty() { lease.mark_clean(); }
    (stopped, restored)
}

/// A measurement of one Provider group: the same shape for each, so a record run can name and key it.
pub type ProviderMeasure = fn(Arc<dyn Device>, &MeasurementPlan) -> Result<MeasurementGroup, String>;

/// The Provider groups in the order the record runs them, each with the value keys it supplies.
pub fn provider_groups() -> [(&'static str, ProviderMeasure, &'static [&'static str]); 8] {
    [
        ("min_timed_command_lead", measure_min_timed_command_lead, &["radio.timing.min_timed_command_lead_ns"]),
        ("tx_in_flight", measure_tx_in_flight, &["radio.timing.tx_in_flight_ns"]),
        ("rx_start_lead", measure_rx_start_lead, &["radio.timing.rx_start_lead_ns"]),
        ("tx_start_lead", measure_tx_start_lead, &["radio.timing.tx_start_lead_ns"]),
        ("rx_throughput", measure_rx_throughput, &["radio.perf.rx_bytes_per_s"]),
        ("overflow_restart_gap", measure_overflow_restart_gap, &[]),
        ("tx_throughput", measure_tx_throughput, &["radio.perf.tx_bytes_per_s"]),
        ("full_duplex", measure_full_duplex, &["radio.full_duplex", "radio.hardware_time"]),
    ]
}

/// The value keys the raw group supplies: queue depth, and each channel's gain, antenna and frequency keys and each stream's rate.
pub fn raw_group_keys(description: &channels::Description) -> Vec<String> {
    let mut keys = vec!["radio.timing.command_queue_depth".to_owned()];
    keys.extend(unmeasured_values(description).into_iter().map(|value| match value {
        ValueRecord::Measured { key, .. } | ValueRecord::Unmeasured { key, .. } => key.as_str().to_owned(),
    }).filter(|key| ["gain_db", "gain_step_db", "antenna", "frequency_hz", "frequency_step_hz", "sample_rate_hz"].iter().any(|leaf| key.ends_with(&format!(".{leaf}")))
        && ["radio.rx.", "radio.tx."].iter().any(|prefix| key.starts_with(prefix))));
    keys
}

/// A probe's arrays longer than this are summarised in the record and kept whole in a rows file.
const SUMMARISED_ARRAY_LEN: usize = 20;

/// Summarises every array of more than `SUMMARISED_ARRAY_LEN` elements in `value`: the count, the
/// minimum, median and maximum of numbers, the counts of a class-like field, the first and last rows
/// and the rows that carry an error. The whole of each such array goes to `sink` as
/// `(path, index, row)`, for a rows file outside the record (stage 3c-2c, record size).
fn summarise_value(value: &Json, path: &str, sink: &mut Vec<(String, usize, Json)>) -> Json {
    match value {
        Json::Array(rows) if rows.len() > SUMMARISED_ARRAY_LEN => {
            for (index, row) in rows.iter().enumerate() { sink.push((path.to_owned(), index, row.clone())); }
            let mut summary = serde_json::Map::new();
            summary.insert("summarised_rows".to_owned(), json!(rows.len()));
            summary.insert("rows_path".to_owned(), json!(path));
            let numbers: Vec<f64> = rows.iter().filter_map(Json::as_f64).collect();
            if numbers.len() == rows.len() {
                let mut sorted = numbers;
                sorted.sort_by(f64::total_cmp);
                summary.insert("min".to_owned(), json!(sorted[0]));
                summary.insert("median".to_owned(), json!(sorted[sorted.len() / 2]));
                summary.insert("max".to_owned(), json!(sorted[sorted.len() - 1]));
            }
            let mut counts = serde_json::Map::new();
            for field in ["class", "classification", "kind", "call", "code", "direction"] {
                let mut by_value: BTreeMap<String, u64> = BTreeMap::new();
                for row in rows { if let Some(text) = row.get(field).and_then(Json::as_str) { *by_value.entry(text.to_owned()).or_default() += 1; } }
                if !by_value.is_empty() { counts.insert(field.to_owned(), json!(by_value)); }
            }
            if !counts.is_empty() { summary.insert("counts".to_owned(), Json::Object(counts)); }
            let mut ignore = Vec::new();
            let mut small = |row: &Json, label: String| summarise_value(row, &label, &mut ignore);
            summary.insert("first".to_owned(), json!(rows.iter().take(3).enumerate().map(|(i, row)| small(row, format!("{path}[{i}]"))).collect::<Vec<_>>()));
            summary.insert("last".to_owned(), json!(rows.iter().enumerate().skip(rows.len() - 3).map(|(i, row)| small(row, format!("{path}[{i}]"))).collect::<Vec<_>>()));
            let outliers: Vec<Json> = rows.iter().enumerate()
                .filter(|(_, row)| ["error", "apply_error"].iter().any(|field| row.get(*field).is_some_and(|found| !found.is_null())))
                .take(5).map(|(i, row)| small(row, format!("{path}[{i}]"))).collect();
            if !outliers.is_empty() { summary.insert("outliers".to_owned(), json!(outliers)); }
            Json::Object(summary)
        }
        Json::Array(rows) => Json::Array(rows.iter().enumerate().map(|(index, row)| summarise_value(row, &format!("{path}[{index}]"), sink)).collect()),
        Json::Object(fields) => Json::Object(fields.iter().map(|(name, field)| (name.clone(), summarise_value(field, &if path.is_empty() { name.clone() } else { format!("{path}.{name}") }, sink))).collect()),
        other => other.clone(),
    }
}

/// A probe's observation with its long arrays summarised; the whole rows go to `<rows_dir>/<name>.jsonl`.
fn summarise_observation(mut row: Observation, rows_dir: Option<&Path>) -> Observation {
    let mut sink = Vec::new();
    let result = summarise_value(&Json::Object(row.result.clone().into_iter().collect()), "", &mut sink);
    if let Json::Object(fields) = result { row.result = fields.into_iter().collect(); }
    if let (Some(dir), false) = (rows_dir, sink.is_empty()) {
        let written = std::fs::create_dir_all(dir).and_then(|()| {
            let mut file = std::fs::File::create(dir.join(format!("{}.jsonl", row.name)))?;
            for (path, index, whole) in &sink { writeln!(file, "{}", json!({ "path": path, "index": index, "row": whole }))?; }
            Ok(())
        });
        row.note = Some(match written {
            Ok(()) => format!("the whole rows of each summarised array are in {}/{}.jsonl", dir.display(), row.name),
            Err(error) => format!("the summarised rows could not be written: {error}"),
        });
    }
    row
}

/// Runs the record's groups one by one. A group or raw probe that returns `Err` or panics is a
/// failed group: its keys are `unmeasured` with the failure, an observation keeps the failure, and
/// the next group runs. After each group the values and observations so far go to a `.partial.json`
/// beside the record, for diagnosis only.
pub struct GroupRun {
    measured: MeasurementGroup,
    completed: Vec<String>,
    failed: Vec<(String, String)>,
    probes: BTreeSet<String>,
    partial: Option<PathBuf>,
    rows_dir: Option<PathBuf>,
}

impl GroupRun {
    pub fn new(output: Option<&Path>) -> GroupRun {
        let partial = output.map(|path| {
            let mut name = path.as_os_str().to_owned();
            name.push(".partial.json");
            PathBuf::from(name)
        });
        let rows_dir = output.map(|path| {
            let mut name = path.as_os_str().to_owned();
            name.push(".rows");
            PathBuf::from(name)
        });
        GroupRun { measured: MeasurementGroup::default(), completed: Vec::new(), failed: Vec::new(), probes: BTreeSet::new(), partial, rows_dir }
    }

    pub fn partial_path(&self) -> Option<&Path> {
        self.partial.as_deref()
    }

    pub fn failed(&self) -> &[(String, String)] {
        &self.failed
    }

    /// The raw probes that ran (pushed an observation) or failed: the set the record expects whole.
    pub fn probes(&self) -> &BTreeSet<String> {
        &self.probes
    }

    pub fn panic_text(payload: Box<dyn std::any::Any + Send>) -> String {
        match payload.downcast::<String>() {
            Ok(text) => format!("panic: {text}"),
            Err(payload) => payload.downcast_ref::<&str>().map_or_else(|| "panic with a non-text payload".to_owned(), |text| format!("panic: {text}")),
        }
    }

    /// Records a failure that is no group's: the device could not be opened or described.
    pub fn record_failure(&mut self, name: &str, error: String) {
        self.fail(name, &[], error);
        self.write_partial();
    }

    fn fail(&mut self, name: &str, keys: &[String], error: String) {
        for key in keys {
            if let Ok(key) = Key::parse(key) {
                self.measured.values.push(ValueRecord::Unmeasured { key, reason: format!("Phase 8 stage 3c-2c: group `{name}` failed: {error}") });
            }
        }
        if let Ok(row) = observation(format!("group_failure_{name}"), QUEUE_TEST, None, 1, json!({ "group": name, "error": error })) {
            self.measured.observations.push(row);
        }
        self.failed.push((name.to_owned(), error));
    }

    /// Runs one group; a failure becomes `unmeasured` rows for `keys`.
    pub fn run(&mut self, name: &str, keys: &[String], measure: impl FnOnce() -> Result<MeasurementGroup, String>) {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(measure)) {
            Ok(Ok(group)) => match self.measured.append(group) {
                Ok(()) => self.completed.push(name.to_owned()),
                Err(error) => self.fail(name, keys, error),
            },
            Ok(Err(error)) => self.fail(name, keys, error),
            Err(payload) => self.fail(name, keys, Self::panic_text(payload)),
        }
        self.write_partial();
    }

    /// Runs one raw hardware probe, which pushes its observation on this thread; a panic is a failed probe.
    pub fn probe(&mut self, name: &str, run: impl FnOnce()) {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run));
        for row in take_raw_observations() {
            self.probes.insert(row.name.clone());
            self.measured.observations.push(summarise_observation(row, self.rows_dir.as_deref()));
        }
        match outcome {
            Ok(()) => self.completed.push(name.to_owned()),
            Err(payload) => {
                self.probes.insert(name.to_owned());
                self.fail(name, &[], Self::panic_text(payload));
            }
        }
        self.write_partial();
    }

    fn write_partial(&self) {
        let Some(path) = &self.partial else { return };
        let document = json!({
            "completed": self.completed,
            "failed": self.failed.iter().map(|(group, error)| json!({ "group": group, "error": error })).collect::<Vec<_>>(),
            "values": self.measured.values,
            "observations": self.measured.observations,
        });
        match serde_json::to_vec_pretty(&document) {
            Ok(bytes) => if let Err(error) = std::fs::write(path, bytes) { println!("partial record {}: {error}", path.display()); },
            Err(error) => println!("partial record {}: {error}", path.display()),
        }
    }

    /// The groups' values and observations, taken.
    pub fn take(&mut self) -> MeasurementGroup {
        std::mem::take(&mut self.measured)
    }

    pub fn remove_partial(&self) {
        if let Some(path) = &self.partial { let _ = std::fs::remove_file(path); }
    }
}

/// One group returns only comparable value rows and observations; fake records replace the
/// rows with `unmeasured` while preserving these observations.
#[derive(Clone, Debug, Default)]
pub struct MeasurementGroup {
    pub conditions: Vec<Condition>,
    pub values: Vec<ValueRecord>,
    pub observations: Vec<Observation>,
}

impl MeasurementGroup {
    pub fn append(&mut self, mut other: MeasurementGroup) -> Result<(), String> {
        for condition in other.conditions.drain(..) {
            if let Some(existing) = self.conditions.iter().find(|row| row.id == condition.id) {
                if existing != &condition { return Err(format!("measurement condition `{}` changed within one record", condition.id)); }
            } else {
                self.conditions.push(condition);
            }
        }
        self.values.append(&mut other.values);
        self.observations.append(&mut other.observations);
        Ok(())
    }
}

/// The booked interval and expected record for one sequential trial in a Run.
#[derive(Clone, Debug)]
pub struct TrialPlan {
    pub start_tick: i64,
    pub end_tick: i64,
    pub admitted: bool,
    pub expected_target: Option<TimePoint>,
    pub expected_samples: u64,
    pub expected_end_tick: i64,
    pub booked_tick: Option<i64>,
    pub completion: TrialCompletion,
    pub requires_preemption_evidence: bool,
    pub capture_index: Option<usize>,
    pub capture_target: Option<TimePoint>,
    pub cold_capture: bool,
}

fn default_timing() -> ezsdr_radio::TimingEnvelope {
    ezsdr_radio::x3xx::timing(&ezsdr_radio_uhd::profile::terms())
}

fn candidate_timing(field: &str, value: u64) -> Result<ezsdr_radio::TimingEnvelope, String> {
    let value = i64::try_from(value).map_err(|_| format!("{field} candidate exceeds i64"))?;
    let mut timing = default_timing();
    match field {
        "min_timed_command_lead_ns" => timing.min_timed_command_lead_ns = value,
        "tx_in_flight_ns" => timing.tx_in_flight_ns = value,
        "rx_start_lead_ns" => timing.rx_start_lead_ns = value,
        "tx_start_lead_ns" => timing.tx_start_lead_ns = value,
        _ => return Err(format!("unsupported candidate timing field `{field}`")),
    }
    Ok(timing)
}

fn candidate_session(device: Arc<dyn Device>, timing: ezsdr_radio::TimingEnvelope) -> Result<(RunHandle, super::TempDir), String> {
    let dir = super::TempDir::new();
    let doc = super::bench_profile(&*device, &dir, Json::Object(Default::default()), super::bench_envelope(&*device), true);
    let run = ezsdr_kernel::coordinator::connect(
        &doc,
        super::assembly(&doc, device, Default::default(), |radio| radio.with_measurement_timing(timing)),
        Lease::attached(),
    ).map_err(|error| error.to_string())?;
    Ok((run, dir))
}

fn ticks_for_ns(ns: u64, master_clock_hz: u64) -> Result<i64, String> {
    let ticks = (u128::from(ns) * u128::from(master_clock_hz)).div_ceil(1_000_000_000);
    i64::try_from(ticks).map_err(|_| "device-tick conversion overflow".to_owned())
}

fn root_now(run: &RunHandle) -> Result<(ClockDomainId, i64), String> {
    let now = run.now();
    let domain = now.domain();
    Ok((domain, now.ticks_in(domain).map_err(|error| error.to_string())?))
}

/// Why a Spec Run that validate or prepare refused has no T0: its termination. `None` for a Run that started.
fn rejected_run_reason(run: &RunHandle) -> Option<String> {
    if run.start_instant().is_some() { return None; }
    Some(match run.state() {
        RunState::CleanedUp { termination } => format!("{termination:?}"),
        other => format!("the Run has no T0 and is {other:?}"),
    })
}

fn stream_clock(run: &RunHandle, direction: Direction) -> Result<SampleClockRecord, String> {
    let suffix = match direction { Direction::Rx => "usrp/rx/s0", Direction::Tx => "usrp/tx/s0" };
    run.sample_clocks().into_iter().rev().find(|clock| clock.stream.to_string().ends_with(suffix) && clock.ended_at.is_none())
        .ok_or_else(|| format!("no active {direction:?} stream-0 SampleClock"))
}

fn sample_at_or_after(clock: &SampleClockRecord, root_tick: i64) -> Result<(TimePoint, i64), String> {
    let origin = clock.origin.ticks_in(clock.root).map_err(|error| error.to_string())?;
    let offset = i128::from(root_tick) - i128::from(origin);
    let num = i128::from(clock.root_ticks_per_tick.num());
    let den = i128::from(clock.root_ticks_per_tick.den());
    let sample = if offset <= 0 { 0 } else { (offset * den + num - 1) / num };
    let root = i128::from(origin) + (sample * num + den - 1) / den;
    let sample = i64::try_from(sample).map_err(|_| "sample target overflow".to_owned())?;
    let root = i64::try_from(root).map_err(|_| "root target overflow".to_owned())?;
    Ok((TimePoint::new(clock.domain, sample), root))
}

fn sample_root_tick(clock: &SampleClockRecord, sample_tick: i64) -> Result<i64, String> {
    let origin = clock.origin.ticks_in(clock.root).map_err(|error| error.to_string())?;
    let num = i128::from(clock.root_ticks_per_tick.num());
    let den = i128::from(clock.root_ticks_per_tick.den());
    let scaled = i128::from(sample_tick) * num;
    if scaled.rem_euclid(den) != 0 { return Err("sample instant is not an integral device tick".to_owned()); }
    i64::try_from(i128::from(origin) + scaled / den).map_err(|_| "sample root tick overflow".to_owned())
}

fn sample_target(run: &RunHandle, direction: Direction, lead_ns: u64, device: &dyn Device) -> Result<(TimePoint, i64), String> {
    let clock = stream_clock(run, direction)?;
    let (root, now) = root_now(run)?;
    if root != clock.root { return Err("Run now and stream clock have different roots".to_owned()); }
    sample_at_or_after(&clock, now.checked_add(ticks_for_ns(lead_ns, device.master_clock_rate())?).ok_or("target root tick overflow")?)
}

fn effective_lead_ns(target_root: i64, receipt_time: TimePoint, root: ClockDomainId, master_clock_hz: u64) -> Result<(i64, i64), String> {
    let receipt_tick = receipt_time.ticks_in(root).map_err(|error| error.to_string())?;
    let lead_ns = i64::try_from((i128::from(target_root) - i128::from(receipt_tick)) * 1_000_000_000 / i128::from(master_clock_hz))
        .map_err(|_| "effective lead overflow".to_owned())?;
    Ok((receipt_tick, lead_ns))
}

fn advance_root(run: &mut RunHandle, root: ClockDomainId, tick: i64) -> Result<(), String> {
    run.advance_to(TimePoint::new(root, tick)).map_err(|error| error.to_string())
}

fn enable_tx(run: &mut RunHandle) -> Result<(), String> {
    super::past_t0(run, super::ms(1));
    let entry = run.submit(super::set("radio.tx.c0.stream", ezsdr_kernel::spec::Value::from(0)), None).map_err(|error| error.to_string())?;
    if !super::admitted(&entry) { return Err(format!("TX stream was not admitted: {:?}", entry.outcome)); }
    super::past_tx_origin(run);
    Ok(())
}

fn disable_rx(run: &mut RunHandle) -> Result<(), String> {
    super::past_t0(run, super::ms(1));
    let entry = run.submit(super::set("radio.rx.c0.stream", ezsdr_kernel::spec::Value::from(-1)), None)
        .map_err(|error| error.to_string())?;
    if !super::admitted(&entry) { return Err(format!("RX stream disable was not admitted: {:?}", entry.outcome)); }
    super::wait(run, super::ms(100));
    if run.sample_clocks().iter().any(|clock| clock.stream.to_string().ends_with("usrp/rx/s0") && clock.ended_at.is_none()) {
        return Err("RX stream remained active during a TX-only measurement".to_owned());
    }
    Ok(())
}

fn start_repeat(run: &mut RunHandle, bytes: &[u8]) -> Result<(), String> {
    let entry = run.submit(super::verb("start_repeat", super::resource("radio/tx/s0"), None, &[]), Some(bytes)).map_err(|error| error.to_string())?;
    if !super::admitted(&entry) { return Err(format!("repeat was not admitted: {:?}", entry.outcome)); }
    Ok(())
}

fn tx_trial(
    run: &mut RunHandle,
    device: &dyn Device,
    bytes: &[u8],
    samples: u64,
    lead_ns: u64,
    interval_ns: u64,
) -> Result<(TrialPlan, i64), String> {
    let (target, target_root) = sample_target(run, Direction::Tx, lead_ns, device)?;
    let entry = run.submit(super::verb("send", super::resource("radio/tx/s0"), Some(target), &[]), Some(bytes)).map_err(|error| error.to_string())?;
    let clock = stream_clock(run, Direction::Tx)?;
    let (receipt_tick, effective_lead_ns) = effective_lead_ns(target_root, entry.time, clock.root, device.master_clock_rate())?;
    let root = clock.root;
    let end_sample = target.ticks_in(clock.domain).map_err(|error| error.to_string())?
        .checked_add(i64::try_from(samples).map_err(|_| "burst sample count exceeds i64".to_owned())?)
        .ok_or("burst end sample overflow")?;
    let expected_end_tick = sample_root_tick(&clock, end_sample)?;
    let end_tick = expected_end_tick.checked_add(ticks_for_ns(interval_ns, device.master_clock_rate())?).ok_or("trial end tick overflow")?;
    advance_root(run, root, end_tick)?;
    Ok((TrialPlan {
        start_tick: receipt_tick, end_tick, admitted: super::admitted(&entry),
        expected_target: Some(target), expected_samples: samples, expected_end_tick, booked_tick: Some(receipt_tick),
        completion: TrialCompletion::TransmitBurstAck, requires_preemption_evidence: false,
        capture_index: None, capture_target: None, cold_capture: false,
    }, effective_lead_ns))
}

fn duplex_condition(id: &str) -> Condition {
    Condition {
        id: id.to_owned(),
        streams: vec![stream_condition(Direction::Rx, 0, vec![0], 1e6), stream_condition(Direction::Tx, 0, vec![0], 1e6)],
        concurrent: vec![stream_ref(Direction::Rx, 0), stream_ref(Direction::Tx, 0)],
        reopened: false,
    }
}

fn tx_only_condition(id: &str) -> Condition {
    Condition { id: id.to_owned(), streams: vec![stream_condition(Direction::Tx, 0, vec![0], 1e6)], concurrent: vec![], reopened: false }
}

fn rx_condition(id: &str, rate: f64) -> Condition {
    Condition { id: id.to_owned(), streams: vec![stream_condition(Direction::Rx, 0, vec![0], rate)], concurrent: vec![], reopened: false }
}

/// Provider-path `radio.timing.min_timed_command_lead_ns` sweep (UR-21).
pub fn measure_min_timed_command_lead(device: Arc<dyn Device>, plan: &MeasurementPlan) -> Result<MeasurementGroup, String> {
    plan.validate()?;
    let condition = "tx-1msps";
    let (bytes, _) = super::waveform_of(&super::pn(100));
    let mut stages = Vec::new();
    let mut rows = Vec::new();
    let mut total = 0usize;
    for value in &plan.timed_command_leads_ns {
        let (mut run, _dir) = candidate_session(device.clone(), candidate_timing("min_timed_command_lead_ns", *value)?)?;
        disable_rx(&mut run)?;
        enable_tx(&mut run)?;
        let mut trials: Vec<TrialPlan> = Vec::new();
        let mut effective = Vec::new();
        for _ in 0..plan.step_trials {
            let (trial, lead) = tx_trial(&mut run, &*device, &bytes, 100, *value, plan.interval_ns)?;
            if let Some(previous) = trials.last_mut() { previous.end_tick = trial.start_tick; }
            trials.push(trial);
            effective.push(Some(lead));
        }
        let manifest = run.finish();
        let evidence = evidence_from_manifest(&manifest, &trials)?;
        let classes: Vec<_> = evidence.iter().map(classify_trial).collect();
        rows.push(tx_candidate_stage_json(condition, *value, &classes, &effective, &evidence));
        stages.push(CandidateStage { value: *value, trials: classes });
        total += plan.step_trials;
    }
    let candidates = BTreeMap::from([(condition.to_owned(), stages)]);
    let summary = summarize_candidates(SummaryMode::MinimumLead, &candidates, plan.step_trials)?;
    let measured = numeric_value("radio.timing.min_timed_command_lead_ns", &summary, &candidates, plan.step_trials, "tests/hardware.rs::hw_measurements_record")?;
    Ok(MeasurementGroup {
        conditions: vec![tx_only_condition(condition)], values: vec![measured],
        observations: vec![observation("provider_min_timed_command_lead", "tests/hardware.rs::hw_measurements_record", Some(condition.to_owned()), observation_trials(total)?,
            json!({ "interval_ns": plan.interval_ns, "candidates": rows }))?],
    })
}

/// Provider-path `radio.timing.tx_in_flight_ns` preemption sweep (UR-21, UR-23).
pub fn measure_tx_in_flight(device: Arc<dyn Device>, plan: &MeasurementPlan) -> Result<MeasurementGroup, String> {
    plan.validate()?;
    let condition = "tx-1msps";
    let (repeat, _) = super::waveform_of(&super::pn(1_000));
    let (burst, _) = super::waveform_of(&super::pn(100));
    let device_lead_ns = u64::try_from(ezsdr_radio_uhd::profile::DEVICE_LEAD_NS).map_err(|_| "negative device lead")?;
    let delivery_allowance_ns = u64::try_from(ezsdr_radio_uhd::profile::DELIVERY_ALLOWANCE_NS).map_err(|_| "negative delivery allowance")?;
    let mut stages = Vec::new();
    let mut rows = Vec::new();
    let mut total = 0usize;
    for value in &plan.in_flight_windows_ns {
        let (mut run, _dir) = candidate_session(device.clone(), candidate_timing("tx_in_flight_ns", *value)?)?;
        disable_rx(&mut run)?;
        enable_tx(&mut run)?;
        start_repeat(&mut run, &repeat)?;
        super::wait(&mut run, super::ms(100));
        let mut trials: Vec<TrialPlan> = Vec::new();
        let mut effective = Vec::new();
        for index in 0..plan.step_trials {
            if index > 0 { start_repeat(&mut run, &repeat)?; }
            let lead = value.checked_add(device_lead_ns).and_then(|lead| lead.checked_add(delivery_allowance_ns))
                .and_then(|lead| lead.checked_add(1_000_000)).ok_or("preemption lead overflow")?;
            let (mut trial, actual) = tx_trial(&mut run, &*device, &burst, 100, lead, plan.interval_ns)?;
            trial.requires_preemption_evidence = true;
            if let Some(previous) = trials.last_mut() { previous.end_tick = trial.start_tick; }
            trials.push(trial);
            effective.push(Some(actual));
        }
        let manifest = run.finish();
        let evidence = evidence_from_manifest(&manifest, &trials)?;
        let classes: Vec<_> = evidence.iter().map(classify_trial).collect();
        rows.push(tx_candidate_stage_json(condition, *value, &classes, &effective, &evidence));
        stages.push(CandidateStage { value: *value, trials: classes });
        total += plan.step_trials;
    }
    let candidates = BTreeMap::from([(condition.to_owned(), stages)]);
    let summary = summarize_candidates(SummaryMode::MinimumLead, &candidates, plan.step_trials)?;
    let measured = numeric_value("radio.timing.tx_in_flight_ns", &summary, &candidates, plan.step_trials, "tests/hardware.rs::hw_measurements_record")?;
    Ok(MeasurementGroup {
        conditions: vec![tx_only_condition(condition)], values: vec![measured],
        observations: vec![observation("provider_tx_in_flight", "tests/hardware.rs::hw_measurements_record", Some(condition.to_owned()), observation_trials(total)?,
            json!({ "interval_ns": plan.interval_ns, "device_lead_ns": device_lead_ns,
                "delivery_allowance_ns": delivery_allowance_ns,
                "lead_extra_ns": 1_000_000, "lead_is_inferred": true, "candidates": rows }))?],
    })
}

fn cold_specs() -> [(u64, u64, &'static str); 2] {
    [(1_000_000, 2_000_000, "cold-1msps-2msps"), (390_625, 400_000, "cold-390625-400000")]
}

fn capture_action(run: &mut RunHandle, target: TimePoint, samples: i64) -> Result<bool, String> {
    let entry = run.submit(super::verb("capture", super::output("rec"), Some(target), &[("sink.capture_samples", ezsdr_kernel::spec::Value::from(samples))]), None)
        .map_err(|error| error.to_string())?;
    Ok(super::admitted(&entry))
}

/// Provider-path receive cold-change lead sweep across both transition pairs (UR-25).
pub fn measure_rx_start_lead(device: Arc<dyn Device>, plan: &MeasurementPlan) -> Result<MeasurementGroup, String> {
    plan.validate()?;
    let mut condition_rows = Vec::new();
    let mut candidates: BTreeMap<String, Vec<CandidateStage>> = BTreeMap::new();
    let mut observation_stages = Vec::new();
    let mut total = 0usize;
    for (from, to, condition) in cold_specs() {
        let mut stages = Vec::new();
        for value in &plan.rx_start_leads_ns {
            let (mut run, _dir) = candidate_session(device.clone(), candidate_timing("rx_start_lead_ns", *value)?)?;
            super::past_t0(&mut run, super::ms(100));
            let (_, run_root) = root_now(&run)?;
            let clock = stream_clock(&run, Direction::Rx)?;
            let root = clock.root;
            if from != 1_000_000 {
                let entry = run.submit(super::set("radio.rx.s0.sample_rate_hz", ezsdr_kernel::spec::Value::num(from as f64).ok_or("invalid initial receive rate")?), None)
                    .map_err(|error| error.to_string())?;
                if !super::admitted(&entry) { return Err(format!("initial receive rate was not admitted: {:?}", entry.outcome)); }
                let settle = run_root.checked_add(ticks_for_ns(*value + 50_000_000, device.master_clock_rate())?).ok_or("settle target overflow")?;
                advance_root(&mut run, root, settle)?;
            }
            let count = ((from as u128 * 12 + to as u128 * 10) / 100).try_into().map_err(|_| "capture sample count overflow".to_owned())?;
            let mut trial_plans: Vec<TrialPlan> = Vec::new();
            for index in 0..plan.step_trials {
                let next_rate = if index % 2 == 0 { to } else { from };
                let active = stream_clock(&run, Direction::Rx)?;
                let (_, start_tick) = root_now(&run)?;
                let capture_at = sample_at_or_after(&active, start_tick.checked_add(ticks_for_ns(20_000_000, device.master_clock_rate())?).ok_or("capture target overflow")?)?.0;
                let target_index = capture_at.ticks_in(active.domain).map_err(|error| error.to_string())?;
                let before_events = run.events(0).len();
                let capture_admitted = capture_action(&mut run, capture_at, i64::try_from(count).map_err(|_| "capture sample count exceeds i64".to_owned())?)?;
                let start_capture = sample_root_tick(&active, target_index)?;
                advance_root(&mut run, root, start_capture.checked_add(ticks_for_ns(40_000_000, device.master_clock_rate())?).ok_or("capture settling overflow")?)?;
                let (_, booked_tick) = root_now(&run)?;
                let entry = run.submit(super::set("radio.rx.s0.sample_rate_hz", ezsdr_kernel::spec::Value::num(next_rate as f64).ok_or("invalid receive rate")?), None)
                    .map_err(|error| error.to_string())?;
                let admitted = capture_admitted && super::admitted(&entry);
                let horizon = TimePoint::new(root, booked_tick.checked_add(ticks_for_ns(3_000_000_000, device.master_clock_rate())?).ok_or("capture horizon overflow")?);
                let written = run.wait_for(&[super::kind("sink.CAPTURE_WRITTEN")], before_events, horizon).map_err(|error| error.to_string())?;
                let events = run.events(before_events);
                let written_at = written.and_then(|_| events.iter().find(|event| event.kind == super::kind("sink.CAPTURE_WRITTEN")).map(|event| event.time))
                    .and_then(|time| time.ticks_in(root).ok());
                let end_tick = written_at.unwrap_or(root_now(&run)?.1)
                    .checked_add(ticks_for_ns(plan.interval_ns, device.master_clock_rate())?).ok_or("cold trial end overflow")?;
                advance_root(&mut run, root, end_tick)?;
                if let Some(previous) = trial_plans.last_mut() { previous.end_tick = start_tick; }
                trial_plans.push(TrialPlan {
                    start_tick, end_tick, admitted, expected_target: None,
                    expected_samples: count, expected_end_tick: 0, booked_tick: Some(booked_tick), completion: TrialCompletion::ReceiveCapture,
                    requires_preemption_evidence: false,
                    capture_index: Some(index), capture_target: Some(capture_at), cold_capture: true,
                });
            }
            let manifest = run.finish();
            let evidence = evidence_from_manifest(&manifest, &trial_plans)?;
            let classes: Vec<_> = evidence.iter().map(classify_trial).collect();
            stages.push(CandidateStage { value: *value, trials: classes.clone() });
            let effective: Vec<_> = trial_plans.iter().map(|trial| effective_cold_lead_ns(&manifest, trial, device.master_clock_rate())).collect();
            observation_stages.push(cold_candidate_stage_json(condition, *value, &classes, &effective, from, to, plan.step_trials));
            total += plan.step_trials;
        }
        candidates.insert(condition.to_owned(), stages);
        condition_rows.push(rx_condition(condition, from as f64));
    }
    let summary = summarize_candidates(SummaryMode::MinimumLead, &candidates, plan.step_trials)?;
    let measured = numeric_value("radio.timing.rx_start_lead_ns", &summary, &candidates, plan.step_trials, "tests/hardware.rs::hw_measurements_record")?;
    Ok(MeasurementGroup {
        conditions: condition_rows, values: vec![measured],
        observations: vec![observation("provider_rx_start_lead", "tests/hardware.rs::hw_measurements_record", None, observation_trials(total)?,
            json!({ "interval_ns": plan.interval_ns, "candidates": observation_stages }))?],
    })
}

fn tx_start_specs() -> [(u64, u64, &'static str); 2] {
    [(1_000_000, 2_000_000, "tx-cold-1msps-2msps"), (390_625, 1_000_000, "tx-cold-390625-1msps")]
}

fn tx_rate_condition(id: &str, rate_hz: u64) -> Condition {
    Condition { id: id.to_owned(), streams: vec![stream_condition(Direction::Tx, 0, vec![0], rate_hz as f64)], concurrent: vec![], reopened: false }
}

fn clock_rate_matches(clock: &SampleClockRecord, rate_hz: u64) -> bool {
    u128::from(clock.nominal_rate.num()) == u128::from(rate_hz) * u128::from(clock.nominal_rate.den())
}

fn tx_switch_at(timing: &Json, origin: i64) -> Option<i64> {
    let mut rows = timing.as_array()?.iter().filter(|row| row["what"] == "tx_switch" && row["origin"].as_i64() == Some(origin));
    let at = rows.next()?.get("at")?.as_i64()?;
    rows.next().is_none().then_some(at)
}

fn tx_start_trial_class(trial: &TrialEvidence, timing: &Json, origin: Option<i64>) -> (TrialClass, Option<i64>) {
    let mut completion = trial.clone();
    completion.expected_samples = 0;
    completion.completion = TrialCompletion::ReceiveCapture;
    completion.record = Some(TrialRecord { samples: 0, end_tick: 0 });
    let class = classify_trial(&completion);
    if class != TrialClass::Success { return (class, origin.and_then(|origin| tx_switch_at(timing, origin))); }
    let Some(origin) = origin else { return (TrialClass::Incomplete, None) };
    let at = tx_switch_at(timing, origin);
    (if at.is_some_and(|at| at < origin) { TrialClass::Success } else { TrialClass::Incomplete }, at)
}

fn tx_start_candidate_stage_json(
    condition: &str,
    value: u64,
    rate_pair_hz: (u64, u64),
    master_clock_hz: u64,
    trials: &[TrialClass],
    transitions: &[[u64; 2]],
    segments: &[(Option<i64>, Option<i64>)],
) -> Json {
    let effective_leads_ns: Vec<_> = segments.iter().map(|(origin, at)| {
        origin.and_then(|origin| at.and_then(|at| i64::try_from((i128::from(origin) - i128::from(at)) * 1_000_000_000 / i128::from(master_clock_hz)).ok()))
    }).collect();
    let mut stage = candidate_stage_json(condition, value, trials, &effective_leads_ns);
    stage["tx_rate_pair_hz"] = json!([rate_pair_hz.0, rate_pair_hz.1]);
    stage["initial_tx_rate_hz"] = json!(transitions.first().map(|pair| pair[0]));
    stage["trial_transitions_hz"] = json!(transitions);
    stage["segment_starts"] = json!(segments.iter().enumerate().map(|(index, (origin, at))| json!({
        "trial": index, "origin_tick": origin, "at_tick": at,
        "start_lead_ns": origin.and_then(|origin| at.and_then(|at| i64::try_from((i128::from(origin) - i128::from(at)) * 1_000_000_000 / i128::from(master_clock_hz)).ok())),
        "at_before_origin": origin.zip(*at).map(|(origin, at)| at < origin),
    })).collect::<Vec<_>>());
    stage["burst_criterion"] = json!("omitted: the test thread must reserve the burst after the new SampleClock is registered, which can be too close to the origin at the 1 ms candidate; tx_switch.at records configuration completion and clock adoption, not that a transmitted burst started; the measurement uses tx_switch.at < origin and error-free evidence");
    stage
}

fn tx_start_value(
    summary: &CandidateSummary,
    candidates: &BTreeMap<String, Vec<CandidateStage>>,
    required_trials: usize,
    test: &str,
) -> Result<ValueRecord, String> {
    numeric_value("radio.timing.tx_start_lead_ns", summary, candidates, required_trials, test)
}

fn summarize_tx_start_lead(candidates: &BTreeMap<String, Vec<CandidateStage>>, required_trials: usize) -> Result<CandidateSummary, String> {
    summarize_candidates(SummaryMode::MinimumLead, candidates, required_trials)
}

/// Provider-path transmit cold-change start-lead sweep across both rate pairs (UR-25).
pub fn measure_tx_start_lead(device: Arc<dyn Device>, plan: &MeasurementPlan) -> Result<MeasurementGroup, String> {
    plan.validate()?;
    let mut conditions = Vec::new();
    let mut candidates = BTreeMap::new();
    let mut observed = Vec::new();
    let mut total = 0usize;
    for (from_rate, to_rate, condition) in tx_start_specs() {
        conditions.push(tx_rate_condition(condition, from_rate));
        let mut stages = Vec::new();
        for value in &plan.tx_start_leads_ns {
            let (mut run, _dir) = candidate_session(device.clone(), candidate_timing("tx_start_lead_ns", *value)?)?;
            disable_rx(&mut run)?;
            enable_tx(&mut run)?;
            let initial_clock = stream_clock(&run, Direction::Tx)?;
            if !clock_rate_matches(&initial_clock, 1_000_000) {
                return Err("TX start-lead measurement requires the X3xx bench profile's 1 MS/s initial TX rate".to_owned());
            }
            let mut current_rate = 1_000_000;
            let mut plans: Vec<TrialPlan> = Vec::new();
            let mut origins = Vec::new();
            let mut transitions = Vec::new();
            for _ in 0..plan.step_trials {
                let next_rate = if current_rate == from_rate { to_rate } else { from_rate };
                if current_rate != from_rate && current_rate != to_rate {
                    return Err(format!("TX start-lead condition {condition} is not at either candidate rate"));
                }
                let previous = stream_clock(&run, Direction::Tx)?;
                let entry = run.submit(super::set("radio.tx.s0.sample_rate_hz", ezsdr_kernel::spec::Value::num(next_rate as f64).ok_or("invalid transmit rate")?), None)
                    .map_err(|error| error.to_string())?;
                let admitted = super::admitted(&entry);
                let clock = if admitted {
                    run.sample_clocks().into_iter().rev().find(|clock| {
                        clock.stream == previous.stream && clock.domain != previous.domain && clock.ended_at.is_none()
                    })
                } else {
                    None
                };
                if let Some(clock) = &clock {
                    if clock.root != previous.root || !clock_rate_matches(clock, next_rate) {
                        return Err(format!("TX cold rate change registered an invalid SampleClock for {next_rate} Hz"));
                    }
                }
                let origin = clock.as_ref().map(|clock| clock.origin.ticks_in(clock.root).map_err(|error| error.to_string())).transpose()?;
                let (root, receipt) = root_now(&run)?;
                if root != previous.root { return Err("TX cold rate change changed the Run root clock".to_owned()); }
                let start_tick = entry.time.ticks_in(root).map_err(|error| error.to_string())?;
                let interval = ticks_for_ns(plan.interval_ns, device.master_clock_rate())?;
                let end_tick = origin.unwrap_or(receipt).max(receipt).checked_add(interval).ok_or("TX start trial end overflow")?;
                if let Some(previous) = plans.last_mut() { previous.end_tick = start_tick; }
                plans.push(TrialPlan {
                    start_tick, end_tick, admitted, expected_target: None, expected_samples: 0,
                    expected_end_tick: 0, booked_tick: Some(start_tick), completion: TrialCompletion::ReceiveCapture,
                    requires_preemption_evidence: false, capture_index: None, capture_target: None, cold_capture: false,
                });
                origins.push(origin);
                transitions.push([current_rate, next_rate]);
                advance_root(&mut run, root, end_tick)?;
                if origin.is_some() { current_rate = next_rate; }
            }
            let manifest = run.finish();
            let evidence = evidence_from_manifest(&manifest, &plans)?;
            let timing = manifest.section(&super::usrp(), "ezsdr.radio.uhd.timing").ok_or("TX start-lead Run has no timing section")?;
            let mut classes = Vec::new();
            let mut starts = Vec::new();
            for (trial, origin) in evidence.iter().zip(&origins) {
                let (class, at) = tx_start_trial_class(trial, timing, *origin);
                classes.push(class);
                starts.push(at);
            }
            let segments: Vec<_> = origins.iter().copied().zip(starts.iter().copied()).collect();
            observed.push(tx_start_candidate_stage_json(condition, *value, (from_rate, to_rate), device.master_clock_rate(), &classes, &transitions, &segments));
            stages.push(CandidateStage { value: *value, trials: classes });
            total += plan.step_trials;
        }
        candidates.insert(condition.to_owned(), stages);
    }
    let summary = summarize_tx_start_lead(&candidates, plan.step_trials)?;
    let measured = tx_start_value(&summary, &candidates, plan.step_trials, "tests/hardware.rs::hw_measurements_record")?;
    Ok(MeasurementGroup {
        conditions, values: vec![measured],
        observations: vec![observation("provider_tx_start_lead", "tests/hardware.rs::hw_measurements_record", None, observation_trials(total)?, json!({
            "interval_ns": plan.interval_ns,
            "rate_pairs_hz": tx_start_specs().iter().map(|(from, to, _)| [from, to]).collect::<Vec<_>>(),
            "candidates": observed,
        }))?],
    })
}

#[derive(Clone, Debug)]
struct OverflowGap {
    cause: String,
    start_sample: Option<i64>,
    len: u64,
    lost: Option<u64>,
    nanoseconds: Option<u64>,
}

#[derive(Clone, Debug)]
struct OverflowRestartEstimate {
    gap_end_sample: Option<i64>,
    reader_resume_sample: Option<i64>,
    nanoseconds: Option<u64>,
    reason: Option<String>,
}

#[derive(Clone, Debug)]
struct OverflowTrialEvidence {
    class: TrialClass,
    overflow_count: usize,
    overflow_causes: Vec<String>,
    payload_lost: Vec<Option<u64>>,
    gaps: Vec<OverflowGap>,
    gap_ns: Option<u64>,
    restart_after_resume: OverflowRestartEstimate,
    sample_clock_rate_matches: bool,
}

fn samples_to_ns_ceil(samples: u64, rate_hz: u64) -> Option<u64> {
    if rate_hz == 0 { return None; }
    u64::try_from((u128::from(samples) * 1_000_000_000).div_ceil(u128::from(rate_hz))).ok()
}

fn restart_after_resume_estimate(
    first_sample: i64,
    gap_start_sample: i64,
    gap_len: u64,
    rate_hz: u64,
    after_ms: u64,
    stall_ms: u64,
) -> OverflowRestartEstimate {
    let unavailable = |reason: String, gap_end_sample, reader_resume_sample| OverflowRestartEstimate {
        gap_end_sample, reader_resume_sample, nanoseconds: None, reason: Some(reason),
    };
    let Some(duration_ms) = after_ms.checked_add(stall_ms) else {
        return unavailable("reader-resume estimate duration overflow".to_owned(), None, None);
    };
    let Some(resume_offset) = u128::from(rate_hz).checked_mul(u128::from(duration_ms)).map(|samples| samples.div_ceil(1_000))
        .and_then(|samples| i64::try_from(samples).ok()) else {
        return unavailable("reader-resume estimate sample conversion overflow".to_owned(), None, None);
    };
    let Some(gap_end_sample) = i64::try_from(gap_len).ok().and_then(|len| gap_start_sample.checked_add(len)) else {
        return unavailable("overflow gap end sample overflow".to_owned(), None, None);
    };
    let Some(reader_resume_sample) = first_sample.checked_add(resume_offset) else {
        return unavailable("reader-resume estimate sample overflow".to_owned(), Some(gap_end_sample), None);
    };
    let Some(restart_samples) = gap_end_sample.checked_sub(reader_resume_sample) else {
        return unavailable("restart-after-resume sample difference overflow".to_owned(), Some(gap_end_sample), Some(reader_resume_sample));
    };
    if restart_samples < 0 {
        // The stall starts after the first block is received, so this early estimate overstates restart-after-resume.
        return unavailable(
            format!("gap ends {0} samples before the estimated reader resume", restart_samples.unsigned_abs()),
            Some(gap_end_sample), Some(reader_resume_sample),
        );
    }
    match samples_to_ns_ceil(restart_samples as u64, rate_hz) {
        Some(nanoseconds) => OverflowRestartEstimate {
            gap_end_sample: Some(gap_end_sample), reader_resume_sample: Some(reader_resume_sample),
            nanoseconds: Some(nanoseconds), reason: None,
        },
        None => unavailable("restart-after-resume time conversion overflow".to_owned(), Some(gap_end_sample), Some(reader_resume_sample)),
    }
}

fn extract_overflow_trial(
    events: &Json,
    artifacts: &Json,
    clocks: &[SampleClockRecord],
    rate_hz: u64,
    after_ms: u64,
    stall_ms: u64,
) -> OverflowTrialEvidence {
    let overflow_rows: Vec<_> = events.as_array().into_iter().flatten()
        .filter(|event| event["kind"] == "radio.RX_OVERFLOW").collect();
    let payloads: Vec<_> = overflow_rows.iter().map(|event| {
        ezsdr_radio::payloads::RxOverflowPayload::from_payload(&event["payload"]).ok()
    }).collect();
    let overflow_causes = payloads.iter().map(|payload| match payload.map(|payload| payload.cause) {
        Some(ezsdr_radio::payloads::RxOverflowCause::Overrun) => "overrun",
        Some(ezsdr_radio::payloads::RxOverflowCause::Sequence) => "sequence",
        None => "invalid",
    }.to_owned()).collect();
    let payload_lost: Vec<_> = payloads.iter().map(|payload| payload.map(|payload| payload.lost)).collect();
    let artifacts_for_capture = artifacts["rec"].as_array();
    let one_complete_capture = artifacts_for_capture.is_some_and(|rows| rows.len() == 1 && rows[0]["partial"] == false);
    let maps = artifacts_for_capture.and_then(|rows| rows.first()).and_then(|artifact| artifact["continuity"].as_array());
    let mut gaps = Vec::new();
    if let Some(maps) = maps {
        for map in maps {
            let map_domain = serde_json::from_value::<ClockDomainId>(map["domain"].clone()).ok();
            for gap in map["gaps"].as_array().into_iter().flatten() {
                let len = gap["len"].as_u64().unwrap_or_default();
                let cause = gap["cause"]["kind"].as_str().unwrap_or("invalid").to_owned();
                let start_sample = map_domain.and_then(|domain| {
                    serde_json::from_value::<TimePoint>(gap["start"].clone()).ok()
                        .filter(|start| start.domain() == domain)
                        .and_then(|start| start.ticks_in(domain).ok())
                });
                gaps.push(OverflowGap {
                    cause, start_sample,
                    len,
                    lost: gap["lost"].as_u64(),
                    nanoseconds: samples_to_ns_ceil(len, rate_hz),
                });
            }
        }
    }
    let rate_matches = clocks.iter().filter(|clock| clock.stream.to_string().ends_with("usrp/rx/s0"))
        .collect::<Vec<_>>().as_slice().first().is_some_and(|clock| {
            clocks.iter().filter(|candidate| candidate.stream.to_string().ends_with("usrp/rx/s0")).count() == 1
                && clock_rate_matches(clock, rate_hz)
        });
    let overflow_gaps: Vec<_> = gaps.iter().filter(|gap| gap.cause == "overflow_restart").collect();
    let capture_first_sample = maps.and_then(|maps| {
        if maps.len() != 1 { return None; }
        let map = &maps[0];
        let domain = serde_json::from_value::<ClockDomainId>(map["domain"].clone()).ok()?;
        let first = serde_json::from_value::<TimePoint>(map["first"].clone()).ok()?;
        (first.domain() == domain).then(|| first.ticks_in(domain).ok()).flatten()
    });
    let restart_after_resume = match (capture_first_sample, overflow_gaps.as_slice()) {
        (Some(first), [gap]) => gap.start_sample.map_or_else(
            || OverflowRestartEstimate {
                gap_end_sample: None, reader_resume_sample: None, nanoseconds: None,
                reason: Some("overflow gap has no sample-clock start".to_owned()),
            },
            |start| restart_after_resume_estimate(first, start, gap.len, rate_hz, after_ms, stall_ms),
        ),
        _ => OverflowRestartEstimate {
            gap_end_sample: None, reader_resume_sample: None, nanoseconds: None,
            reason: Some("restart estimate needs one capture map and one overflow_restart gap".to_owned()),
        },
    };
    let valid = one_complete_capture
        && maps.is_some_and(|maps| maps.len() == 1)
        && rate_matches
        && overflow_rows.len() == 1
        && payloads.first().is_some_and(|payload| payload.is_some_and(|payload| payload.cause == ezsdr_radio::payloads::RxOverflowCause::Overrun))
        && overflow_gaps.len() == 1
        && overflow_gaps[0].len > 0
        && overflow_gaps[0].lost == Some(overflow_gaps[0].len)
        && payload_lost.first().copied().flatten() == Some(overflow_gaps[0].len);
    OverflowTrialEvidence {
        class: if valid { TrialClass::Success } else { TrialClass::Incomplete },
        overflow_count: overflow_rows.len(), overflow_causes, payload_lost,
        gap_ns: valid.then(|| overflow_gaps[0].nanoseconds).flatten(), gaps,
        restart_after_resume,
        sample_clock_rate_matches: rate_matches,
    }
}

#[derive(Clone, Debug)]
struct OverflowSummary {
    worst_gap_ns: Option<u64>,
    limiting_rate_hz: Option<u64>,
    maximum_restart_after_resume_ns: Option<u64>,
    maximum_restart_rate_hz: Option<u64>,
    maximum_restart_by_rate: BTreeMap<u64, Option<u64>>,
    successful_by_rate: BTreeMap<u64, usize>,
    unmeasured_reason: Option<String>,
}

fn summarize_overflow_gaps(candidates: &BTreeMap<u64, Vec<OverflowTrialEvidence>>, required_trials: usize) -> Result<OverflowSummary, String> {
    if candidates.is_empty() { return Err("at least one overflow rate is required".to_owned()); }
    if required_trials == 0 { return Err("required overflow trials must be positive".to_owned()); }
    let mut successful_by_rate = BTreeMap::new();
    let mut maximum = None::<(u64, u64)>;
    let mut maximum_restart = None::<(u64, u64)>;
    let mut maximum_restart_by_rate = BTreeMap::new();
    let mut insufficient = Vec::new();
    for (rate, trials) in candidates {
        if *rate == 0 || trials.is_empty() { return Err("overflow rates and trial lists must be nonempty and positive".to_owned()); }
        let successful: Vec<_> = trials.iter().filter(|trial| trial.class == TrialClass::Success).collect();
        successful_by_rate.insert(*rate, successful.len());
        if successful.len() < required_trials {
            insufficient.push(format!("{rate} Hz: {} complete overflow trials, need {required_trials}", successful.len()));
        }
        let mut rate_restart_maximum = None;
        for trial in successful {
            if let Some(gap_ns) = trial.gap_ns {
                if maximum.is_none_or(|(_, current)| gap_ns > current) { maximum = Some((*rate, gap_ns)); }
            }
            if let Some(restart_ns) = trial.restart_after_resume.nanoseconds {
                if rate_restart_maximum.is_none_or(|current| restart_ns > current) { rate_restart_maximum = Some(restart_ns); }
                if maximum_restart.is_none_or(|(_, current)| restart_ns > current) { maximum_restart = Some((*rate, restart_ns)); }
            }
        }
        maximum_restart_by_rate.insert(*rate, rate_restart_maximum);
    }
    let (worst_gap_ns, limiting_rate_hz) = maximum.map_or((None, None), |(rate, gap)| (Some(gap), Some(rate)));
    let (maximum_restart_after_resume_ns, maximum_restart_rate_hz) =
        maximum_restart.map_or((None, None), |(rate, gap)| (Some(gap), Some(rate)));
    Ok(OverflowSummary {
        worst_gap_ns, limiting_rate_hz, maximum_restart_after_resume_ns, maximum_restart_rate_hz,
        maximum_restart_by_rate, successful_by_rate,
        unmeasured_reason: (!insufficient.is_empty()).then(|| format!("Insufficient complete overflow trials: {}.", insufficient.join("; "))),
    })
}

fn overflow_wait_ms(after_ms: u64, stall_ms: u64) -> Result<u64, String> {
    let stall_recovery_ms = stall_ms.checked_mul(2).ok_or("overflow Run horizon overflow")?;
    2_000u64.checked_add(after_ms).and_then(|ms| ms.checked_add(stall_recovery_ms)).and_then(|ms| ms.checked_add(500))
        .ok_or_else(|| "overflow Run horizon overflow".to_owned())
}

/// The Run's Manifest, and its termination if validate or prepare refused it (it then never ran).
fn overflow_manifest(device: Arc<dyn Device>, rate_hz: u64, after_ms: u64, stall_ms: u64) -> Result<(Manifest, Option<String>), String> {
    let capture_ms = after_ms.checked_add(stall_ms).and_then(|ms| ms.checked_add(250)).ok_or("overflow capture duration overflow")?;
    let samples = (u128::from(rate_hz) * u128::from(capture_ms)).div_ceil(1_000);
    let samples = i64::try_from(samples).map_err(|_| "overflow capture sample count exceeds i64".to_owned())?;
    let dir = super::TempDir::new();
    let doc = super::bench_profile(&*device, &dir, Json::Object(Default::default()), Json::Object(Default::default()), false);
    let spec = super::receive_spec(1, rate_hz as f64, super::bench_hz(&*device), Some(samples));
    let mut run = super::start_spec_run(&spec, &doc, super::assembly(&doc, device.clone(), BTreeMap::new(), |radio| {
        radio.with_rx_stall(Wall::from_millis(after_ms), Wall::from_millis(stall_ms))
    })).map_err(|error| error.to_string())?;
    if let Some(reason) = rejected_run_reason(&run) { return Ok((run.finish(), Some(reason))); }
    let wait_ms = overflow_wait_ms(after_ms, stall_ms)?;
    let wait_ns = wait_ms.checked_mul(1_000_000).ok_or("overflow Run horizon overflow")?;
    let (root, now) = root_now(&run)?;
    let horizon = now.checked_add(ticks_for_ns(wait_ns, device.master_clock_rate())?).ok_or("overflow Run horizon overflow")?;
    advance_root(&mut run, root, horizon)?;
    Ok((run.finish(), None))
}

/// Provider-path overflow restart-gap measurement, retaining non-overflowing trials as failures (RM-17).
pub fn measure_overflow_restart_gap(device: Arc<dyn Device>, plan: &MeasurementPlan) -> Result<MeasurementGroup, String> {
    plan.validate()?;
    let mut by_rate = BTreeMap::new();
    let mut rejection_by_rate = BTreeMap::new();
    let mut conditions = Vec::new();
    let mut total = 0usize;
    for (rate_hz, stall_ms) in plan.overflow_rates_hz.iter().zip(&plan.overflow_stall_ms) {
        let condition = rate_condition(*rate_hz);
        conditions.push(rx_condition(&condition, *rate_hz as f64));
        let mut trials = Vec::new();
        let mut rejections = Vec::new();
        for _ in 0..plan.overflow_trials {
            let (manifest, rejection) = overflow_manifest(device.clone(), *rate_hz, plan.overflow_after_ms, *stall_ms)?;
            let events = serde_json::to_value(&manifest.events.delivered).map_err(|error| error.to_string())?;
            let artifacts = serde_json::to_value(&manifest.artifacts).map_err(|error| error.to_string())?;
            let mut evidence = extract_overflow_trial(&events, &artifacts, &manifest.clocks.sample_clocks, *rate_hz,
                plan.overflow_after_ms, *stall_ms);
            if rejection.is_some() { evidence.class = TrialClass::AdmissionRejection; }
            rejections.push(rejection);
            trials.push(evidence);
        }
        total += plan.overflow_trials;
        by_rate.insert(*rate_hz, trials);
        rejection_by_rate.insert(*rate_hz, rejections);
    }
    let summary = summarize_overflow_gaps(&by_rate, plan.overflow_trials)?;
    let rate_observations: Vec<_> = by_rate.iter().map(|(rate, trials)| json!({
        "rate_hz": rate,
        "after_ms": plan.overflow_after_ms,
        "stall_ms": plan.overflow_stall_ms[plan.overflow_rates_hz.iter().position(|candidate| candidate == rate).unwrap()],
        "successful_trials": summary.successful_by_rate.get(rate),
            "required_successful_trials": plan.overflow_trials,
            "maximum_restart_after_resume_ns": summary.maximum_restart_by_rate.get(rate),
            "trials": trials.iter().enumerate().map(|(index, trial)| json!({
                "trial": index, "class": trial_class_name(trial.class),
                "run_termination": rejection_by_rate[rate][index],
                "overflow_count": trial.overflow_count, "overflow_causes": trial.overflow_causes,
                "payload_lost": trial.payload_lost, "sample_clock_rate_matches": trial.sample_clock_rate_matches,
                "restart_after_resume_ns": trial.restart_after_resume.nanoseconds,
                "restart_after_resume_reason": trial.restart_after_resume.reason,
                "gap_end_sample": trial.restart_after_resume.gap_end_sample,
                "estimated_reader_resume_sample": trial.restart_after_resume.reader_resume_sample,
                "gaps": trial.gaps.iter().map(|gap| json!({ "cause": gap.cause, "len_samples": gap.len, "lost": gap.lost, "gap_ns": gap.nanoseconds })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
    })).collect();
    Ok(MeasurementGroup {
        conditions, values: vec![],
        observations: vec![observation("provider_overflow_restart_gap", "tests/hardware.rs::hw_measurements_record", None,
            observation_trials(total)?, json!({
                "after_ms": plan.overflow_after_ms,
                "gap_rule": "gap_ns = ceil(lost_samples * 1e9 / sample_rate_hz)",
                "gap_scope": "the full RM-17 continuity gap includes loss during the configured reader stall and socket-backlog drain; it is not UHD recovery time alone",
                "restart_after_resume_rule": "restart_after_resume_ns = ceil((gap_end_sample - ceil(capture_first_sample + (after_ms + stall_ms) * rate_hz / 1000)) * 1e9 / rate_hz); negative results are null",
                "restart_after_resume_interpretation": "INFERRED: the stall begins after the host receives the first block, so the estimated resume sample is early and this value is high-side; it includes draining buffered samples",
                "maximum_gap_ns": summary.worst_gap_ns,
                "maximum_rate_hz": summary.limiting_rate_hz,
                "maximum_restart_after_resume_ns": summary.maximum_restart_after_resume_ns,
                "maximum_restart_rate_hz": summary.maximum_restart_rate_hz,
                "unmeasured_reason": summary.unmeasured_reason,
                "rate_measurements": rate_observations,
            }))?],
    })
}

fn rate_condition(rate_hz: u64) -> String {
    if rate_hz % 1_000_000 == 0 { format!("rx-{}msps", rate_hz / 1_000_000) } else { format!("rx-{rate_hz}hz") }
}

/// Provider-path receive capacity sweep over duration-sized Runs (UR-34).
pub fn measure_rx_throughput(device: Arc<dyn Device>, plan: &MeasurementPlan) -> Result<MeasurementGroup, String> {
    plan.validate()?;
    let mut stages = Vec::new();
    let mut observations = Vec::new();
    let mut conditions = Vec::new();
    let dir = super::TempDir::new();
    let doc = super::bench_profile(&*device, &dir, Json::Object(Default::default()), Json::Object(Default::default()), false);
    let binding = radio_binding(&doc)?;
    let description = description(&binding, ezsdr_radio_uhd::profile::DEFAULT_BLOCK_LEN, &ezsdr_radio_uhd::profile::terms())?;
    let wire_bytes = u64::try_from(description.performance.wire_bytes_per_sample).map_err(|_| "wire bytes per sample is negative".to_owned())?;
    for rate in &plan.rx_rates_hz {
        let id = rate_condition(*rate);
        let dir = super::TempDir::new();
        let doc = super::bench_profile(&*device, &dir, Json::Object(Default::default()), Json::Object(Default::default()), false);
        let spec = super::receive_spec(1, *rate as f64, super::bench_hz(&*device), None);
        let mut run = super::spec_run(&spec, &doc, device.clone(), Default::default());
        let bytes_per_second = rate.checked_mul(wire_bytes).ok_or("receive bytes per second overflow")?;
        // A Run that validate or prepare refused has no T0: the rate is an admission rejection, not a group failure.
        if let Some(reason) = rejected_run_reason(&run) {
            stages.push(CandidateStage { value: bytes_per_second, trials: vec![TrialClass::AdmissionRejection] });
            conditions.push(rx_condition(&id, *rate as f64));
            let stage = candidate_stage_json(&id, bytes_per_second, &[TrialClass::AdmissionRejection], &[None]);
            observations.push(observation(
                format!("provider_rx_throughput_{rate}"), "tests/hardware.rs::hw_measurements_record", Some(id), 1,
                json!({ "rate_hz": rate, "duration_secs": plan.throughput_seconds, "run_termination": reason, "stage": stage }),
            )?);
            continue;
        }
        let t0 = run.start_instant().ok_or("receive Run has no T0")?;
        let root = t0.domain();
        let start = t0.ticks_in(root).map_err(|error| error.to_string())?;
        let duration_ticks = device.master_clock_rate().checked_mul(plan.throughput_seconds).ok_or("throughput duration overflow")?;
        let horizon = start.checked_add(i64::try_from(duration_ticks).map_err(|_| "throughput horizon overflow".to_owned())?).ok_or("throughput horizon overflow")?;
        advance_root(&mut run, root, horizon)?;
        let packet = device.rx_packet_samples(0).max(1) as u64;
        let manifest = run.finish();
        let actual = super::section(&manifest, "stats")["rx_samples"].as_i64().unwrap_or_default().max(0) as u64;
        let mut rx_clocks = manifest.clocks.sample_clocks.iter()
            .filter(|clock| clock.stream.to_string().ends_with("usrp/rx/s0"));
        let interval = rx_clocks.next().and_then(|clock| {
            if rx_clocks.next().is_some() { None } else { expected_rx_samples(clock, *rate, device.master_clock_rate()) }
        });
        let (interval_ticks, expected) = interval.map_or((None, None), |(ticks, samples)| (Some(ticks), Some(samples)));
        let overflow = manifest.events.delivered.iter().any(|event| event.kind.as_str() == "radio.RX_OVERFLOW");
        let class = throughput_trial_class(actual, expected, packet, overflow);
        stages.push(CandidateStage { value: bytes_per_second, trials: vec![class] });
        conditions.push(rx_condition(&id, *rate as f64));
        let stage = candidate_stage_json(&id, bytes_per_second, &[class], &[None]);
        observations.push(observation(
            format!("provider_rx_throughput_{rate}"), "tests/hardware.rs::hw_measurements_record", Some(id), 1,
            json!({ "rate_hz": rate, "duration_secs": plan.throughput_seconds, "sample_clock_interval_ticks": interval_ticks,
                "expected_samples": expected, "actual_samples": actual, "packet_samples": packet,
                "rx_overflow": overflow, "wire_bytes_per_sample": wire_bytes, "stage": stage }),
        )?);
    }
    let one_condition = BTreeMap::from([("rx-throughput".to_owned(), stages.clone())]);
    let summary = summarize_candidates(SummaryMode::Capacity, &one_condition, 1)?;
    let row = capacity_value(&summary, &plan.rx_rates_hz, &stages, "tests/hardware.rs::hw_measurements_record")?;
    Ok(MeasurementGroup { conditions, values: vec![row], observations })
}

fn capacity_value(summary: &CandidateSummary, rates_hz: &[u64], stages: &[CandidateStage], test: &str) -> Result<ValueRecord, String> {
    let key = Key::parse("radio.perf.rx_bytes_per_s").map_err(|error| error.to_string())?;
    match summary.worst {
        Some(bytes) => {
            let (rate, stage) = rates_hz.iter().zip(stages).find(|(_, stage)| stage.value == bytes).ok_or("selected throughput candidate has no rate")?;
            Ok(ValueRecord::Measured {
                key, worst: model::Worst::Number(bytes as f64), trials: observation_trials(stage.trials.len())?,
                condition: rate_condition(*rate), test: test.to_owned(),
            })
        }
        None => Ok(ValueRecord::Unmeasured {
            key, reason: summary.unmeasured_reason.clone().unwrap_or_else(|| "No receive-throughput candidate completed a full successful trial.".to_owned()),
        }),
    }
}

fn tx_rate_id(rate_hz: u64) -> String {
    if rate_hz % 1_000_000 == 0 { format!("tx-{}msps", rate_hz / 1_000_000) } else { format!("tx-{rate_hz}hz") }
}

fn tx_bytes_per_second(rate_hz: u64, wire_bytes_per_sample: u64) -> Result<u64, String> {
    rate_hz.checked_mul(wire_bytes_per_sample).ok_or_else(|| "transmit bytes per second overflow".to_owned())
}

fn tx_capacity_value(summary: &CandidateSummary, rates_hz: &[u64], stages: &[CandidateStage], test: &str) -> Result<ValueRecord, String> {
    let key = Key::parse("radio.perf.tx_bytes_per_s").map_err(|error| error.to_string())?;
    match summary.worst {
        Some(bytes) => {
            let (rate, stage) = rates_hz.iter().zip(stages).find(|(_, stage)| stage.value == bytes).ok_or("selected transmit-throughput candidate has no rate")?;
            Ok(ValueRecord::Measured {
                key, worst: model::Worst::Number(bytes as f64), trials: observation_trials(stage.trials.len())?,
                condition: tx_rate_id(*rate), test: test.to_owned(),
            })
        }
        None => Ok(ValueRecord::Unmeasured {
            key, reason: summary.unmeasured_reason.clone().unwrap_or_else(|| "No transmit-throughput candidate completed a full successful trial.".to_owned()),
        }),
    }
}

fn summarize_tx_throughput(stages: &[CandidateStage]) -> Result<CandidateSummary, String> {
    summarize_candidates(SummaryMode::Capacity, &BTreeMap::from([("tx-throughput".to_owned(), stages.to_vec())]), 1)
}

#[derive(Clone, Debug)]
struct TxThroughputEvidence {
    class: TrialClass,
    actual_samples: Option<u64>,
    burst_count: usize,
    burst_target: Option<TimePoint>,
    burst_start_tick: Option<i64>,
    burst_target_matches_schedule: bool,
    burst_ack_count: usize,
    burst_ack_tick: Option<i64>,
    minimum_ack_tick: Option<i64>,
    ack_elapsed_ticks: Option<i64>,
    ack_elapsed_samples: Option<u64>,
    minimum_samples: Option<u64>,
    tx_samples_minus_ack_elapsed_samples: Option<i64>,
    underflow_events: usize,
    async_failures: Vec<String>,
    short_send_rejections: Vec<String>,
}

struct TxThroughputInput<'a> {
    stats: &'a Json,
    async_reports: &'a Json,
    events: &'a Json,
    bursts: &'a Json,
    rejected: &'a Json,
    clocks: &'a [SampleClockRecord],
    scheduled_target: TimePoint,
    rate_hz: u64,
    master_clock_hz: u64,
    duration_secs: u64,
}

fn samples_for_root_ticks(ticks: i64, rate_hz: u64, master_clock_hz: u64) -> Option<u64> {
    if ticks < 0 || master_clock_hz == 0 { return None; }
    let samples = u128::try_from(ticks).ok()?.checked_mul(u128::from(rate_hz))? / u128::from(master_clock_hz);
    u64::try_from(samples).ok()
}

fn tx_throughput_evidence(input: TxThroughputInput<'_>) -> TxThroughputEvidence {
    let TxThroughputInput { stats, async_reports, events, bursts, rejected, clocks, scheduled_target, rate_hz, master_clock_hz, duration_secs } = input;
    let actual_samples = stats.get("tx_samples").and_then(Json::as_u64)
        .or_else(|| stats.get("tx_samples").and_then(Json::as_i64).and_then(|samples| u64::try_from(samples).ok()));
    let burst_rows = bursts.as_array().cloned().unwrap_or_default();
    let burst_count = burst_rows.len();
    let burst = (burst_count == 1).then(|| &burst_rows[0]);
    let burst_target = burst.and_then(|row| serde_json::from_value::<TimePoint>(row["target"].clone()).ok());
    let tx_clocks: Vec<_> = clocks.iter().filter(|clock| clock.stream.to_string().ends_with("usrp/tx/s0")).collect();
    let tx_clock = (tx_clocks.len() == 1).then(|| tx_clocks[0]);
    let burst_start_tick = burst_target.and_then(|target| {
        tx_clock.filter(|clock| clock.domain == target.domain()).and_then(|_| point_root_tick(target, clocks).ok())
    });
    let burst_target_matches_schedule = burst_target == Some(scheduled_target);
    let burst_ends_on_stop = burst.is_some_and(|row| serde_json::from_value::<BurstEnd>(row["end"].clone()).ok() == Some(BurstEnd::Stop));
    let minimum_ack_tick = burst_start_tick.and_then(|start| {
        duration_secs.checked_mul(master_clock_hz).and_then(|ticks| i64::try_from(ticks).ok()).and_then(|ticks| start.checked_add(ticks))
    });
    let minimum_samples = duration_secs.checked_mul(rate_hz);
    let burst_acks: Vec<_> = async_reports.as_array().into_iter().flatten()
        .filter(|report| report["code"] == "BurstAck")
        .map(|report| report["tick"].as_i64()).collect();
    let burst_ack_count = burst_acks.len();
    let burst_ack_tick = (burst_ack_count == 1).then(|| burst_acks[0]).flatten();
    let ack_elapsed_ticks = burst_ack_tick.zip(burst_start_tick).and_then(|(ack, start)| ack.checked_sub(start));
    let ack_elapsed_samples = ack_elapsed_ticks.and_then(|ticks| samples_for_root_ticks(ticks, rate_hz, master_clock_hz));
    let tx_samples_minus_ack_elapsed_samples = actual_samples.zip(ack_elapsed_samples).and_then(|(actual, elapsed)| {
        i64::try_from(i128::from(actual) - i128::from(elapsed)).ok()
    });
    let underflow_events = events.as_array().into_iter().flatten().filter(|event| event["kind"] == "radio.TX_UNDERFLOW").count();
    let async_failures: Vec<_> = async_reports.as_array().into_iter().flatten()
        .filter_map(|report| report["code"].as_str())
        .filter(|code| matches!(*code, "Underflow" | "UnderflowInPacket" | "TimeError") || code.starts_with("SeqError"))
        .map(str::to_owned).collect();
    let short_send_rejections: Vec<_> = rejected.as_array().into_iter().flatten()
        .filter_map(|row| row["reason"].as_str())
        .filter(|reason| reason.contains("short send") || reason.contains("send did not complete"))
        .map(str::to_owned).collect();
    let complete_burst = burst_count == 1 && burst_ends_on_stop && burst_target_matches_schedule && burst_start_tick.is_some();
    let enough_samples = actual_samples.zip(minimum_samples).is_some_and(|(actual, minimum)| actual >= minimum);
    let ack_after_minimum = burst_ack_tick.zip(minimum_ack_tick).is_some_and(|(ack, minimum)| ack >= minimum);
    let class = if underflow_events > 0 || !async_failures.is_empty() || !short_send_rejections.is_empty() { TrialClass::DeviceFailure }
        else if burst_ack_count > 1 || burst_count > 1 { TrialClass::Unattributable }
        else if !complete_burst || !enough_samples || !ack_after_minimum { TrialClass::Incomplete }
        else { TrialClass::Success };
    TxThroughputEvidence {
        class, actual_samples, burst_count, burst_target, burst_start_tick, burst_target_matches_schedule,
        burst_ack_count, burst_ack_tick, minimum_ack_tick, ack_elapsed_ticks, ack_elapsed_samples,
        minimum_samples, tx_samples_minus_ack_elapsed_samples, underflow_events, async_failures, short_send_rejections,
    }
}

/// Provider-path transmit capacity sweep over duration-sized repeat Runs (UR-34).
pub fn measure_tx_throughput(device: Arc<dyn Device>, plan: &MeasurementPlan) -> Result<MeasurementGroup, String> {
    plan.validate()?;
    let dir = super::TempDir::new();
    let doc = super::bench_profile(&*device, &dir, Json::Object(Default::default()), Json::Object(Default::default()), false);
    let mut conditions = Vec::new();
    let mut stages = Vec::new();
    let mut observations = Vec::new();
    let wire_bytes = u64::try_from(description(
        &radio_binding(&doc)?, ezsdr_radio_uhd::profile::DEFAULT_BLOCK_LEN, &ezsdr_radio_uhd::profile::terms(),
    )?.performance.wire_bytes_per_sample).map_err(|_| "wire bytes per sample is negative".to_owned())?;
    for rate in &plan.tx_throughput_rates_hz {
        let id = tx_rate_id(*rate);
        let (bytes, waveform) = super::waveform_of(&super::pn(1_000));
        let run_dir = super::TempDir::new();
        let profile = super::bench_profile(&*device, &run_dir, Json::Object(Default::default()), Json::Object(Default::default()), false);
        let spec = super::with_burst(
            super::with_tx(super::receive_spec(0, *rate as f64, super::bench_hz(&*device), None), *rate as f64),
            &waveform, true, "send_asap_and_flag", 0,
        );
        let inputs = BTreeMap::from([(waveform.hash.clone(), bytes.clone())]);
        let mut run = super::spec_run(&spec, &profile, device.clone(), inputs);
        if let Some(reason) = rejected_run_reason(&run) {
            let value = tx_bytes_per_second(*rate, wire_bytes)?;
            stages.push(CandidateStage { value, trials: vec![TrialClass::AdmissionRejection] });
            conditions.push(tx_rate_condition(&id, *rate));
            let stage = candidate_stage_json(&id, value, &[TrialClass::AdmissionRejection], &[None]);
            observations.push(observation(
                format!("provider_tx_throughput_{rate}"), "tests/hardware.rs::hw_measurements_record", Some(id), 1,
                json!({ "rate_hz": rate, "duration_secs": plan.throughput_seconds, "run_termination": reason, "stage": stage }),
            )?);
            continue;
        }
        let clock = stream_clock(&run, Direction::Tx)?;
        if !clock_rate_matches(&clock, *rate) { return Err(format!("TX throughput Run declared the wrong SampleClock rate for {rate} Hz")); }
        let t0 = run.start_instant().ok_or("TX throughput Run has no T0")?;
        if t0.domain() != clock.root { return Err("TX throughput T0 and SampleClock have different root clocks".to_owned()); }
        let t0_tick = t0.ticks_in(clock.root).map_err(|error| error.to_string())?;
        let (scheduled_target, burst_start_root_tick) = sample_at_or_after(&clock, t0_tick)?;
        let duration_ticks = device.master_clock_rate().checked_mul(plan.throughput_seconds).ok_or("TX throughput duration overflow")?;
        let horizon = burst_start_root_tick.checked_add(i64::try_from(duration_ticks).map_err(|_| "TX throughput horizon overflow")?)
            .ok_or("TX throughput horizon overflow")?;
        advance_root(&mut run, clock.root, horizon)?;
        let manifest = run.finish();
        let stats = manifest.section(&super::usrp(), "ezsdr.radio.uhd.stats").ok_or("TX throughput Run has no stats section")?;
        let async_reports = manifest.section(&super::usrp(), "ezsdr.radio.uhd.async").ok_or("TX throughput Run has no async section")?;
        let bursts = manifest.section(&super::usrp(), "ezsdr.radio.uhd.bursts").ok_or("TX throughput Run has no burst section")?;
        let rejected = manifest.section(&super::usrp(), "ezsdr.radio.uhd.rejected").ok_or("TX throughput Run has no rejected section")?;
        let events = serde_json::to_value(&manifest.events.delivered).map_err(|error| error.to_string())?;
        let evidence = tx_throughput_evidence(TxThroughputInput {
            stats, async_reports, events: &events, bursts, rejected, clocks: &manifest.clocks.sample_clocks,
            scheduled_target, rate_hz: *rate, master_clock_hz: device.master_clock_rate(), duration_secs: plan.throughput_seconds,
        });
        let value = tx_bytes_per_second(*rate, wire_bytes)?;
        stages.push(CandidateStage { value, trials: vec![evidence.class] });
        conditions.push(tx_rate_condition(&id, *rate));
        let stage = candidate_stage_json(&id, value, &[evidence.class], &[None]);
        observations.push(observation(
            format!("provider_tx_throughput_{rate}"), "tests/hardware.rs::hw_measurements_record", Some(id), 1,
            json!({ "rate_hz": rate, "duration_secs": plan.throughput_seconds,
                "scheduled_burst_target": scheduled_target, "burst_target": evidence.burst_target,
                "burst_target_matches_schedule": evidence.burst_target_matches_schedule,
                "burst_start_tick": evidence.burst_start_tick, "burst_ack_count": evidence.burst_ack_count,
                "burst_ack_tick": evidence.burst_ack_tick, "minimum_ack_tick": evidence.minimum_ack_tick,
                "ack_elapsed_ticks": evidence.ack_elapsed_ticks, "ack_elapsed_samples": evidence.ack_elapsed_samples,
                "minimum_samples": evidence.minimum_samples, "actual_samples": evidence.actual_samples,
                "tx_samples_minus_ack_elapsed_samples": evidence.tx_samples_minus_ack_elapsed_samples,
                "burst_count": evidence.burst_count, "tx_underflow_events": evidence.underflow_events,
                "async_failures": evidence.async_failures, "short_send_rejections": evidence.short_send_rejections,
                "stage": stage }),
        )?);
    }
    let summary = summarize_tx_throughput(&stages)?;
    let row = tx_capacity_value(&summary, &plan.tx_throughput_rates_hz, &stages, "tests/hardware.rs::hw_measurements_record")?;
    Ok(MeasurementGroup { conditions, values: vec![row], observations })
}

fn expected_rx_samples(clock: &SampleClockRecord, rate_hz: u64, master_clock_hz: u64) -> Option<(i64, u64)> {
    let start = clock.origin.ticks_in(clock.root).ok()?;
    let end = clock.ended_at?.ticks_in(clock.root).ok()?;
    let interval = end.checked_sub(start)?;
    if interval <= 0 || master_clock_hz == 0 { return None; }
    let samples = (interval as u128).checked_mul(u128::from(rate_hz))? / u128::from(master_clock_hz);
    Some((interval, u64::try_from(samples).ok()?))
}

fn throughput_trial_class(actual: u64, expected: Option<u64>, packet: u64, overflow: bool) -> TrialClass {
    if overflow { TrialClass::DeviceFailure }
    else if expected.is_some_and(|expected| actual.abs_diff(expected) <= packet) { TrialClass::Success }
    else { TrialClass::Incomplete }
}

fn loopback_values(classes: &[TrialClass], exact_starts: &[bool], heard: &[bool]) -> (bool, bool) {
    (
        classes.iter().zip(heard).all(|(class, heard)| *class == TrialClass::Success && *heard),
        classes.iter().zip(exact_starts).all(|(class, exact)| *class == TrialClass::Success && *exact),
    )
}

/// Provider-path full-duplex and timed-capture rehearsal (UR-34).
pub fn measure_full_duplex(device: Arc<dyn Device>, plan: &MeasurementPlan) -> Result<MeasurementGroup, String> {
    plan.validate()?;
    let condition = "duplex-1msps";
    let wave = super::pn(1_000);
    let (bytes, _) = super::waveform_of(&wave);
    let mut target_rows = Vec::new();
    let mut classes = Vec::new();
    let mut first_matches = Vec::new();
    let mut heard = Vec::new();
    for index in 0..plan.loopback_trials {
        let opened = candidate_session(device.clone(), default_timing())?;
        // A Session that opened without T0 was refused by validate or prepare.
        if let Some(reason) = rejected_run_reason(&opened.0) {
            classes.push(TrialClass::AdmissionRejection);
            first_matches.push(false);
            heard.push(false);
            target_rows.push(json!({ "trial": index, "class": "AdmissionRejection", "run_termination": reason, "hardware_time": false, "waveform_found": false }));
            continue;
        }
        let (mut run, _dir) = opened;
        let t0 = run.start_instant().ok_or("loopback Run has no T0")?;
        let root = t0.domain();
        let trial_start = t0.ticks_in(root).map_err(|error| error.to_string())?;
        enable_tx(&mut run)?;
        start_repeat(&mut run, &bytes)?;
        let active = stream_clock(&run, Direction::Rx)?;
        let (_, now) = root_now(&run)?;
        let capture_target = sample_at_or_after(&active, now.checked_add(ticks_for_ns(50_000_000, device.master_clock_rate())?).ok_or("loopback capture target overflow")?)?.0;
        let first = capture_target.ticks_in(active.domain).map_err(|error| error.to_string())?;
        let from_events = run.events(0).len();
        let capture_admitted = capture_action(&mut run, capture_target, 5_000)?;
        let horizon = TimePoint::new(root, now.checked_add(ticks_for_ns(3_000_000_000, device.master_clock_rate())?).ok_or("loopback horizon overflow")?);
        let written = run.wait_for(&[super::kind("sink.CAPTURE_WRITTEN")], from_events, horizon).map_err(|error| error.to_string())?;
        let event_rows = run.events(from_events);
        let end_tick = written.and_then(|_| event_rows.iter().find(|event| event.kind == super::kind("sink.CAPTURE_WRITTEN")).map(|event| event.time))
            .and_then(|time| time.ticks_in(root).ok()).unwrap_or_else(|| root_now(&run).map_or(now, |(_, tick)| tick));
        let interval_end = end_tick.checked_add(ticks_for_ns(plan.interval_ns, device.master_clock_rate())?).ok_or("duplex trial end overflow")?;
        advance_root(&mut run, root, interval_end)?;
        let manifest = run.finish();
        let capture = serde_json::to_value(&manifest.artifacts).map_err(|error| error.to_string())?;
        let capture_ref = manifest.artifacts.get(&Ident::parse("rec").map_err(|error| error.to_string())?)
            .and_then(|artifacts| artifacts.first());
        let capture_first = capture_ref.and_then(|artifact| artifact.continuity.first()).map(|map| map.first);
        let exact_start = capture_first == Some(capture_target);
        let samples = capture_ref.and_then(read_capture_samples);
        let found = samples.as_ref().and_then(|samples| super::correlate(samples, &wave)).is_some();
        let mut trial = TrialPlan {
            start_tick: trial_start, end_tick: end_tick.checked_add(ticks_for_ns(plan.interval_ns, device.master_clock_rate())?).ok_or("duplex trial end overflow")?,
            admitted: capture_admitted, expected_target: None, expected_samples: 5_000,
            expected_end_tick: 0, booked_tick: Some(now), completion: TrialCompletion::ReceiveCapture,
            requires_preemption_evidence: false,
            capture_index: Some(0), capture_target: Some(capture_target), cold_capture: false,
        };
        let evidence = evidence_from_sections(
            super::section(&manifest, "bursts"), super::section(&manifest, "async"), super::section(&manifest, "rejected"),
            &serde_json::to_value(&manifest.events.delivered).map_err(|error| error.to_string())?,
            &capture, &manifest.clocks.sample_clocks, std::slice::from_mut(&mut trial),
        )?;
        let mut class = classify_trial(&evidence[0]);
        if !found && class == TrialClass::Success { class = TrialClass::Incomplete; }
        classes.push(class);
        first_matches.push(exact_start);
        heard.push(found);
        target_rows.push(json!({
            "trial": index, "class": format!("{class:?}"), "capture_first": capture_first.map(|point| point.ticks_in(point.domain()).unwrap_or_default()),
            "requested_capture_first": first, "hardware_time": exact_start, "waveform_found": found,
        }));
    }
    let (full_duplex, hardware_time) = loopback_values(&classes, &first_matches, &heard);
    let rejected = classes.contains(&TrialClass::AdmissionRejection);
    let value = |key: &str, worst: bool| -> Result<ValueRecord, String> {
        if rejected {
            return Ok(ValueRecord::Unmeasured {
                key: Key::parse(key).map_err(|error| error.to_string())?,
                reason: "Phase 8 stage 3c-2c: validate or prepare refused a loopback Run, so no trial measured it.".to_owned(),
            });
        }
        Ok(ValueRecord::Measured {
            key: Key::parse(key).map_err(|error| error.to_string())?, worst: model::Worst::Boolean(worst),
            trials: observation_trials(plan.loopback_trials)?, condition: condition.to_owned(), test: "tests/hardware.rs::hw_measurements_record".to_owned(),
        })
    };
    Ok(MeasurementGroup {
        conditions: vec![duplex_condition(condition)],
        values: vec![value("radio.full_duplex", full_duplex)?, value("radio.hardware_time", hardware_time)?],
        observations: vec![observation("provider_full_duplex", "tests/hardware.rs::hw_measurements_record", Some(condition.to_owned()), observation_trials(plan.loopback_trials)?,
            json!({ "interval_ns": plan.interval_ns, "trials": target_rows, "full_duplex": full_duplex, "hardware_time": hardware_time }))?],
    })
}

fn read_capture_samples(artifact: &ArtifactRef) -> Option<Vec<(f32, f32)>> {
    if artifact.partial { return None; }
    let path = artifact.uri.strip_prefix("file://")?;
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() % 8 != 0 { return None; }
    bytes.chunks_exact(8).map(|frame| Some((
        f32::from_le_bytes(frame[0..4].try_into().ok()?),
        f32::from_le_bytes(frame[4..8].try_into().ok()?),
    ))).collect()
}

/// The six Manifest-derived outcomes for one trial. This accepts only immutable JSON sections
/// and the pre-booked plan, so attribution and completion can be unit-tested without a Device.
pub fn extract_trial_evidence(
    bursts: &Json,
    async_reports: &Json,
    rejected: &Json,
    radio_events: &Json,
    artifacts: &Json,
    clocks: &[SampleClockRecord],
    plans: &[TrialPlan],
) -> Result<Vec<TrialEvidence>, String> {
    let mut trials: Vec<_> = plans.iter().map(|plan| {
        let mut trial = TrialEvidence::new(plan.start_tick, plan.end_tick, plan.expected_samples, plan.expected_end_tick, plan.completion);
        trial.admitted = plan.admitted;
        trial.requires_preemption_evidence = plan.requires_preemption_evidence;
        trial
    }).collect();
    if let Some(rows) = bursts.as_array() {
        for burst in rows {
            let Some(target) = serde_json::from_value::<TimePoint>(burst["target"].clone()).ok() else { continue };
            let matching: Vec<_> = plans.iter().enumerate().filter(|(_, plan)| plan.expected_target == Some(target)).map(|(i, _)| i).collect();
            if let [index] = matching.as_slice() {
                let plan = &plans[*index];
                let samples = burst["samples"].as_u64().unwrap_or_default();
                let end = serde_json::from_value::<BurstEnd>(burst["end"].clone()).ok();
                if trials[*index].record.is_some() {
                    trials[*index].unattributable = true;
                } else if end == Some(BurstEnd::Eob) {
                    let clock = clocks.iter().find(|clock| clock.domain == target.domain()).ok_or_else(|| "burst target has no SampleClock record".to_owned())?;
                    let end_sample = target.ticks_in(target.domain()).map_err(|error| error.to_string())?.checked_add(i64::try_from(samples).map_err(|_| "burst sample count exceeds i64".to_owned())?).ok_or("burst end sample overflow")?;
                    let end_tick = sample_root_tick(clock, end_sample)?;
                    trials[*index].record = Some(TrialRecord { samples, end_tick });
                } else {
                    trials[*index].record = Some(TrialRecord { samples, end_tick: plan.expected_end_tick.saturating_sub(1) });
                }
            }
        }
    }
    if let Some(rows) = bursts.as_array() {
        for (index, plan) in plans.iter().enumerate().filter(|(_, plan)| plan.requires_preemption_evidence) {
            let Some(target) = plan.expected_target.and_then(|target| point_root_tick(target, clocks).ok()) else { continue };
            let matching = rows.iter().filter(|burst| {
                if serde_json::from_value::<BurstEnd>(burst["end"].clone()).ok() != Some(BurstEnd::Eob) { return false; }
                let Some(repeat_target) = serde_json::from_value::<TimePoint>(burst["target"].clone()).ok() else { return false; };
                if plan.expected_target == Some(repeat_target) { return false; }
                let Some(samples) = burst["samples"].as_u64().and_then(|samples| i64::try_from(samples).ok()) else { return false; };
                let Some(clock) = clocks.iter().find(|clock| clock.domain == repeat_target.domain()) else { return false; };
                repeat_target.ticks_in(repeat_target.domain()).ok().and_then(|start| start.checked_add(samples))
                    .and_then(|end| sample_root_tick(clock, end).ok()) == Some(target)
            }).count();
            if matching == 1 { trials[index].repeat_ended_at_target = true; }
            else if matching > 1 { trials[index].unattributable = true; }
        }
    }
    for (index, plan) in plans.iter().enumerate() {
        if plan.completion == TrialCompletion::ReceiveCapture {
            trials[index].record = capture_record(artifacts, clocks, plan);
        }
    }
    if let Some(rows) = radio_events.as_array() {
        for event in rows {
            let name = event["kind"].as_str().unwrap_or_default();
            if name == "radio.TIME_ERROR" {
                let evidence = match event["payload"]["outcome"].as_str() {
                    Some("drop") => EvidenceKind::ProviderTimeError(ProviderTimeError::Drop),
                    Some("send_asap") => EvidenceKind::ProviderTimeError(ProviderTimeError::SendAsap),
                    Some("refused") => EvidenceKind::ProviderTimeError(ProviderTimeError::Refused),
                    Some("plan_violation") => EvidenceKind::ProviderTimeError(ProviderTimeError::PlanViolation),
                    Some("late_at_device") => EvidenceKind::ProviderTimeError(ProviderTimeError::LateAtDevice),
                    _ => EvidenceKind::UnknownTimeError,
                };
                let target = serde_json::from_value::<TimePoint>(event["payload"]["target"].clone()).ok();
                let matching: Vec<_> = plans.iter().enumerate().filter(|(_, plan)| target.is_some() && plan.expected_target == target).map(|(i, _)| i).collect();
                let tick = event_tick(event, clocks);
                if let [index] = matching.as_slice() {
                    if tick.is_some_and(|tick| plans[*index].start_tick <= tick && tick < plans[*index].end_tick) {
                        trials[*index].evidence.push(evidence);
                    } else {
                        trials[*index].unattributable = true;
                    }
                } else {
                    attribute_manifest_evidence(&mut trials, TimedEvidence { tick, span: None, kind: evidence });
                }
            } else {
                let kind = match name {
                    "radio.LATE_COMMAND" => EvidenceKind::LateCommand,
                    "radio.COMMAND_REJECTED" => EvidenceKind::CommandRejected,
                    "radio.COMMAND_QUEUE_FULL" => EvidenceKind::CommandQueueFull,
                    "radio.RX_OVERFLOW" => EvidenceKind::RxOverflow,
                    "radio.TX_UNDERFLOW" => EvidenceKind::Underflow,
                    _ => continue,
                };
                attribute_manifest_evidence(&mut trials, TimedEvidence { tick: event_tick(event, clocks), span: None, kind });
            }
        }
    }
    if let Some(rows) = async_reports.as_array() {
        for report in rows {
            let kind = match report["code"].as_str().unwrap_or_default() {
                "BurstAck" => {
                    let tick = report["tick"].as_i64();
                    let Some(tick) = tick else {
                        for (index, plan) in plans.iter().enumerate() {
                            if plan.completion == TrialCompletion::TransmitBurstAck { trials[index].unattributable = true; }
                        }
                        continue;
                    };
                    let matching: Vec<_> = plans.iter().enumerate().filter(|(_, plan)| {
                        plan.completion == TrialCompletion::TransmitBurstAck
                            && plan.start_tick <= tick && tick < plan.end_tick
                            && plan.expected_target.and_then(|target| point_root_tick(target, clocks).ok()).is_some_and(|target| tick >= target)
                    }).map(|(index, _)| index).collect();
                    if let [index] = matching.as_slice() {
                        trials[*index].burst_ack_ticks.push(tick);
                    } else {
                        for index in matching { trials[index].unattributable = true; }
                    }
                    continue;
                }
                "TimeError" => EvidenceKind::AsyncTimeError,
                "Underflow" => EvidenceKind::Underflow,
                "UnderflowInPacket" => EvidenceKind::UnderflowInPacket,
                "SeqError" => EvidenceKind::SequenceError,
                "SeqErrorInBurst" => EvidenceKind::SequenceErrorInBurst,
                _ => EvidenceKind::OtherAsync,
            };
            attribute_manifest_evidence(&mut trials, TimedEvidence { tick: report["tick"].as_i64(), span: None, kind });
        }
    }
    for trial in &mut trials {
        if trial.completion == TrialCompletion::TransmitBurstAck {
            match trial.burst_ack_ticks.len() {
                1 => trial.evidence.push(EvidenceKind::BurstAck),
                0 => {},
                _ => trial.unattributable = true,
            }
        }
    }
    if let Some(rows) = rejected.as_array() {
        if rows.iter().any(|row| row["reason"].as_str().is_some_and(|reason| reason.contains("send did not complete") || reason.contains("short send"))) {
            for (index, plan) in plans.iter().enumerate() {
                if plan.completion == TrialCompletion::TransmitBurstAck {
                    trials[index].evidence.push(EvidenceKind::ShortSendRejected);
                }
            }
        }
    }
    Ok(trials)
}

fn attribute_manifest_evidence(trials: &mut [TrialEvidence], evidence: TimedEvidence) {
    attribute_evidence(trials, evidence);
}

fn event_tick(event: &Json, clocks: &[SampleClockRecord]) -> Option<i64> {
    let point = serde_json::from_value::<TimePoint>(event["time"].clone()).ok()?;
    point_root_tick(point, clocks).ok()
}

fn point_root_tick(point: TimePoint, clocks: &[SampleClockRecord]) -> Result<i64, String> {
    if let Some(clock) = clocks.iter().find(|clock| clock.root == point.domain()) {
        return point.ticks_in(clock.root).map_err(|error| error.to_string());
    }
    let clock = clocks.iter().find(|clock| clock.domain == point.domain()).ok_or("event time has no known device root")?;
    sample_root_tick(clock, point.ticks_in(clock.domain).map_err(|error| error.to_string())?)
}

fn capture_record(artifacts: &Json, clocks: &[SampleClockRecord], plan: &TrialPlan) -> Option<TrialRecord> {
    let artifact = artifacts.get("rec")?.as_array()?.get(plan.capture_index?)?;
    if artifact.get("partial")?.as_bool()? { return None; }
    let maps = artifact.get("continuity")?.as_array()?;
    if maps.is_empty() { return None; }
    let mut samples = 0u64;
    for map in maps {
        if !map.get("gaps")?.as_array()?.is_empty() || !map.get("channel_gaps")?.as_array()?.is_empty() { return None; }
        let first: TimePoint = serde_json::from_value(map.get("first")?.clone()).ok()?;
        let end: TimePoint = serde_json::from_value(map.get("end")?.clone()).ok()?;
        let domain: ClockDomainId = serde_json::from_value(map.get("domain")?.clone()).ok()?;
        if first.domain() != domain || end.domain() != domain { return None; }
        let len = end.ticks_in(end.domain()).ok()?.checked_sub(first.ticks_in(first.domain()).ok()?)?;
        samples = samples.checked_add(u64::try_from(len).ok()?)?;
    }
    if samples != plan.expected_samples { return None; }
    if plan.cold_capture {
        if maps.len() != 2 { return None; }
        let capture_first: TimePoint = serde_json::from_value(maps[0].get("first")?.clone()).ok()?;
        if plan.capture_target.is_some_and(|target| target != capture_first) { return None; }
        let first_domain: ClockDomainId = serde_json::from_value(maps[0].get("domain")?.clone()).ok()?;
        let second_domain: ClockDomainId = serde_json::from_value(maps[1].get("domain")?.clone()).ok()?;
        if first_domain == second_domain { return None; }
        let old = clocks.iter().find(|clock| clock.domain == first_domain)?;
        let new = clocks.iter().find(|clock| clock.domain == second_domain)?;
        if old.stream != new.stream || old.root != new.root || !old.stream.to_string().ends_with("usrp/rx/s0") { return None; }
        let ended = old.ended_at?;
        let ended_root = ended.ticks_in(old.root).ok()?;
        if new.origin.ticks_in(new.root).ok()? <= ended_root { return None; }
        let origin_root = old.origin.ticks_in(old.root).ok()?;
        let numerator = i128::from(old.root_ticks_per_tick.num());
        let denominator = i128::from(old.root_ticks_per_tick.den());
        let elapsed = (i128::from(ended_root) - i128::from(origin_root)) * denominator;
        if elapsed.rem_euclid(numerator) != 0 { return None; }
        let e1 = elapsed / numerator;
        let recorded_end: TimePoint = serde_json::from_value(maps[0].get("end")?.clone()).ok()?;
        let new_first: TimePoint = serde_json::from_value(maps[1].get("first")?.clone()).ok()?;
        if i128::from(recorded_end.ticks_in(first_domain).ok()?) != e1
            || new_first.ticks_in(second_domain).ok()? != 0
        { return None; }
    }
    Some(TrialRecord { samples, end_tick: plan.expected_end_tick })
}

fn effective_cold_lead_ns(manifest: &Manifest, plan: &TrialPlan, master_clock_hz: u64) -> Option<i64> {
    let booked = plan.booked_tick?;
    let timing = super::section(manifest, "timing").as_array()?;
    let (row, booked_at) = timing.iter()
        .filter(|row| row["what"] == "cold_change" && row["key"] == "radio.rx.s0.sample_rate_hz")
        .filter_map(|row| row["booked_at"].as_i64().map(|tick| (row, tick)))
        .min_by_key(|(_, tick)| i128::from(*tick).abs_diff(i128::from(booked)))?;
    let e = row["e"].as_i64()?;
    let rx_clocks: Vec<_> = manifest.clocks.sample_clocks.iter()
        .filter(|clock| clock.stream.to_string().ends_with("usrp/rx/s0"))
        .collect();
    let old_index = rx_clocks.iter().position(|clock| clock.ended_at
        .and_then(|ended| ended.ticks_in(clock.root).ok()) == Some(e))?;
    let origin = rx_clocks.get(old_index + 1)?.origin.ticks_in(rx_clocks[old_index + 1].root).ok()?;
    i64::try_from((i128::from(origin) - i128::from(booked_at)) * 1_000_000_000 / i128::from(master_clock_hz)).ok()
}

fn observation_trials(trials: usize) -> Result<u32, String> {
    u32::try_from(trials).map_err(|_| "measurement observation exceeds the record's u32 trial limit".to_owned())
}

fn evidence_from_sections(
    bursts: &Json,
    async_reports: &Json,
    rejected: &Json,
    radio_events: &Json,
    artifacts: &Json,
    clocks: &[SampleClockRecord],
    plans: &[TrialPlan],
) -> Result<Vec<TrialEvidence>, String> {
    extract_trial_evidence(bursts, async_reports, rejected, radio_events, artifacts, clocks, plans)
}

fn evidence_from_manifest(manifest: &Manifest, plans: &[TrialPlan]) -> Result<Vec<TrialEvidence>, String> {
    let events = serde_json::to_value(&manifest.events.delivered).map_err(|error| error.to_string())?;
    let artifacts = serde_json::to_value(&manifest.artifacts).map_err(|error| error.to_string())?;
    evidence_from_sections(
        super::section(manifest, "bursts"), super::section(manifest, "async"), super::section(manifest, "rejected"),
        &events, &artifacts, &manifest.clocks.sample_clocks, plans,
    )
}

fn trial_class_name(class: TrialClass) -> &'static str {
    match class {
        TrialClass::AdmissionRejection => "AdmissionRejection",
        TrialClass::ProviderFailure => "ProviderFailure",
        TrialClass::DeviceFailure => "DeviceFailure",
        TrialClass::Incomplete => "Incomplete",
        TrialClass::Unattributable => "Unattributable",
        TrialClass::Success => "Success",
    }
}

fn candidate_stage_json(condition: &str, value: u64, trials: &[TrialClass], effective_leads_ns: &[Option<i64>]) -> Json {
    let mut counts = BTreeMap::from([
        ("AdmissionRejection", 0usize), ("ProviderFailure", 0), ("DeviceFailure", 0),
        ("Incomplete", 0), ("Unattributable", 0), ("Success", 0),
    ]);
    for trial in trials { *counts.get_mut(trial_class_name(*trial)).expect("all trial classes are listed") += 1; }
    json!({ "condition": condition, "candidate": value, "classes": counts, "effective_lead_ns": effective_leads_ns,
        "trials": trials.iter().enumerate().map(|(index, class)| json!({ "trial": index, "class": trial_class_name(*class), "effective_lead_ns": effective_leads_ns.get(index) })).collect::<Vec<_>>() })
}

fn tx_candidate_stage_json(
    condition: &str,
    value: u64,
    trials: &[TrialClass],
    effective_leads_ns: &[Option<i64>],
    evidence: &[TrialEvidence],
) -> Json {
    let mut stage = candidate_stage_json(condition, value, trials, effective_leads_ns);
    if let Some(rows) = stage["trials"].as_array_mut() {
        for (row, trial) in rows.iter_mut().zip(evidence) {
            row["expected_end_tick"] = json!(trial.expected_end_tick);
            row["burst_ack_ticks"] = json!(trial.burst_ack_ticks);
            row["burst_ack_offsets_from_expected_end_ticks"] = json!(trial.burst_ack_ticks.iter()
                .map(|tick| tick.checked_sub(trial.expected_end_tick)).collect::<Vec<_>>());
            row["repeat_ended_at_target"] = json!(trial.repeat_ended_at_target);
        }
    }
    stage
}

fn cold_candidate_stage_json(
    condition: &str,
    value: u64,
    trials: &[TrialClass],
    effective_leads_ns: &[Option<i64>],
    from_rate_hz: u64,
    to_rate_hz: u64,
    trial_count: usize,
) -> Json {
    let mut stage = candidate_stage_json(condition, value, trials, effective_leads_ns);
    stage["rate_pair_hz"] = json!([from_rate_hz, to_rate_hz]);
    stage["trial_transitions_hz"] = json!((0..trial_count).map(|index| {
        if index % 2 == 0 { [from_rate_hz, to_rate_hz] } else { [to_rate_hz, from_rate_hz] }
    }).collect::<Vec<_>>());
    stage
}

fn numeric_value(
    key: &str,
    summary: &CandidateSummary,
    candidates: &BTreeMap<String, Vec<CandidateStage>>,
    required_trials: usize,
    test: &str,
) -> Result<ValueRecord, String> {
    let key = Key::parse(key).map_err(|error| error.to_string())?;
    let Some(worst) = summary.worst else {
        return Ok(ValueRecord::Unmeasured { key, reason: summary.unmeasured_reason.clone().unwrap_or_else(|| "No candidate satisfies the successful-edge rule.".to_owned()) });
    };
    let condition = summary.limiting_condition.as_deref().ok_or("selected measurement has no limiting condition")?;
    let trials = candidates.get(condition).and_then(|stages| stages.iter().find(|stage| stage.value == worst)).map(|stage| stage.trials.len()).unwrap_or(required_trials);
    Ok(ValueRecord::Measured {
        key, worst: model::Worst::Number(worst as f64), trials: u32::try_from(trials).map_err(|_| "measured trial count exceeds u32".to_owned())?,
        condition: condition.to_owned(), test: test.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_queue_depth_requires_backpressure_timing_and_success_in_every_condition() {
        let call = |duration_ns, h_i_ns, error: Option<&str>| QueueCall {
            duration_ns, h_i_ns, error: error.map(str::to_owned),
        };
        let clock = QueueClock { d0_tick: 1_000, h_before_ns: 0, h_after_ns: 10 };
        let first_at = 1_100;
        let stall_limit = 1_500_000_000;
        let return_limit_ticks = 100;
        let classify = |calls: &[QueueCall], timing| classify_queue_trial(calls, timing, first_at, stall_limit, return_limit_ticks, 1_000_000_000);
        assert_eq!(classify(&[call(1_000_000, Some(99), None), call(stall_limit + 1, Some(110), None)], clock),
            QueueTrialClass::Success { depth: 1 });
        assert_eq!(classify(&[call(stall_limit + 1, Some(200), None)], QueueClock { h_after_ns: 0, ..clock }),
            QueueTrialClass::Success { depth: 0 });
        assert!(matches!(classify(&[call(stall_limit, Some(110), None)], clock), QueueTrialClass::Incomplete { .. }));
        assert!(matches!(classify(&vec![call(1_000_000, Some(99), None); 4], clock), QueueTrialClass::Incomplete { .. }));
        assert!(matches!(classify(&[call(100_000_001, Some(99), None)], clock), QueueTrialClass::Incomplete { .. }));
        assert!(matches!(classify(&[call(stall_limit + 1, Some(109), None)], clock), QueueTrialClass::Incomplete { .. }));
        assert!(matches!(classify(&[call(stall_limit + 1, Some(210), None)], clock), QueueTrialClass::Incomplete { .. }));
        assert!(matches!(classify(&[call(stall_limit + 1, None, None)], clock), QueueTrialClass::Incomplete { .. }));
        assert!(matches!(classify(&[call(1_000_000, Some(100), None), call(stall_limit + 1, Some(110), None)], clock), QueueTrialClass::Incomplete { .. }));
        assert!(matches!(classify(&[call(1_000_000, Some(101), None), call(stall_limit + 1, Some(111), None)], clock), QueueTrialClass::Incomplete { .. }));
        assert!(matches!(classify(&[call(1_000_000, None, None), call(stall_limit + 1, Some(110), None)], clock), QueueTrialClass::Incomplete { .. }));
        assert!(matches!(classify(&[call(QUEUE_CALL_TIME_LIMIT_NS, Some(99), None), call(stall_limit + 1, Some(110), None)], clock), QueueTrialClass::Incomplete { .. }));
        assert!(matches!(classify(&[call(stall_limit + 1, Some(110), None), call(stall_limit + 1, Some(110), None)], clock), QueueTrialClass::Incomplete { .. }));
        assert!(matches!(classify(&[], QueueClock { h_after_ns: QUEUE_CALL_TIME_LIMIT_NS + 1, ..clock }),
            QueueTrialClass::Incomplete { reason: "the device-time read took too long to bound the host-clock interval" }));
        assert!(matches!(classify(&[call(1, Some(110), Some("apply failed"))], clock), QueueTrialClass::DeviceFailure { .. }));
        assert_eq!(finish_queue_trial(QueueTrialClass::Success { depth: 1 }, None, Some("untimed reset failed".to_owned())),
            QueueTrialClass::DeviceFailure { reason: "untimed reset failed".to_owned() });
        let conditions = [
            ("a".to_owned(), vec![QueueTrialClass::Success { depth: 11 }, QueueTrialClass::Success { depth: 9 }]),
            ("b".to_owned(), vec![QueueTrialClass::Success { depth: 7 }, QueueTrialClass::Success { depth: 8 }]),
        ];
        assert_eq!(summarize_queue_depth(&conditions, 2), Some((7, "b".to_owned(), 2)));
        assert!(matches!(queue_depth_value(&conditions, 2).unwrap(), ValueRecord::Measured {
            key, worst: model::Worst::Number(value), trials: 2, condition, test,
        } if key.as_str() == "radio.timing.command_queue_depth" && value == 7.0 && condition == "b" && test == QUEUE_TEST));
        let incomplete = [
            ("a".to_owned(), vec![QueueTrialClass::Success { depth: 11 }, QueueTrialClass::Success { depth: 9 }]),
            ("b".to_owned(), vec![QueueTrialClass::Success { depth: 7 }, QueueTrialClass::Incomplete { reason: "no stall" }]),
        ];
        assert!(summarize_queue_depth(&incomplete, 2).is_none());
        assert!(matches!(queue_depth_value(&incomplete, 2).unwrap(), ValueRecord::Unmeasured { reason, .. } if reason.contains("2 successful back-pressure trials in every queue condition")));
    }

    #[test]
    fn coercion_readbacks_keep_only_exact_gain_rate_and_antenna_values() {
        let points = vec![
            NumericReadback { requested: 1.0, returned: Some(1.0 + 1e-13), error: None },
            NumericReadback { requested: 2.0, returned: Some(2.1), error: None },
        ];
        let gain = coercion_grid_value("radio.rx.c0.gain_db", &points, "coercion", "test", gain_readback_matches).unwrap();
        assert!(matches!(gain, ValueRecord::Measured { worst: model::Worst::Grid(model::GridValue::Values { values }), .. } if values == [1.0]));
        let rate = coercion_grid_value("radio.rx.s0.sample_rate_hz", &points, "coercion", "test", rate_readback_matches).unwrap();
        assert!(matches!(rate, ValueRecord::Measured { worst: model::Worst::Grid(model::GridValue::Values { values }), .. } if values == [1.0]));
        assert!(!gain_readback_matches(1.0, 1.0 + 1e-8));
        assert!(!rate_readback_matches(1.0, 1.0 + 1e-10));
        let failed = coercion_grid_value("radio.rx.c0.gain_db", &[NumericReadback {
            requested: 3.0, returned: None, error: Some("set_gain failed".to_owned()),
        }], "coercion", "test", |requested, actual| requested == actual).unwrap();
        assert!(matches!(failed, ValueRecord::Unmeasured { reason, .. } if reason.contains("set_gain failed")));

        let half_db = (0..=5).map(|index| NumericReadback {
            requested: index as f64 / 10.0,
            returned: Some(if index < 3 { 0.0 } else { 0.5 }), error: None,
        }).collect::<Vec<_>>();
        let one_db = (0..=5).map(|index| NumericReadback {
            requested: index as f64 / 5.0,
            returned: Some(if index < 3 { 0.0 } else { 1.0 }), error: None,
        }).collect::<Vec<_>>();
        let uneven = [0.0, 0.5, 1.5].into_iter().map(|value| NumericReadback {
            requested: value, returned: Some(value), error: None,
        }).collect::<Vec<_>>();
        assert!(matches!(gain_step_value("radio.rx.c0.gain_step_db", &half_db, "coercion", "test").unwrap(), ValueRecord::Measured { worst: model::Worst::Number(value), .. } if value == 0.5));
        assert!(matches!(gain_step_value("radio.rx.c0.gain_step_db", &one_db, "coercion", "test").unwrap(), ValueRecord::Measured { worst: model::Worst::Number(value), .. } if value == 1.0));
        assert!(matches!(gain_step_value("radio.rx.c0.gain_step_db", &uneven, "coercion", "test").unwrap(), ValueRecord::Measured { worst: model::Worst::Number(value), .. } if value == 1.0));
        let antennas = vec![
            AntennaReadback { requested: "RX2".to_owned(), returned: Some("RX2".to_owned()), error: None },
            AntennaReadback { requested: "TX/RX".to_owned(), returned: Some("RX2".to_owned()), error: None },
        ];
        assert!(matches!(antenna_value("radio.rx.c0.antenna", &antennas, "coercion", "test").unwrap(), ValueRecord::Measured { worst: model::Worst::Texts(values), .. } if values == ["RX2"]));
        assert!(matches!(antenna_value("radio.rx.c0.antenna", &[AntennaReadback {
            requested: "RX2".to_owned(), returned: None, error: None,
        }], "coercion", "test").unwrap(), ValueRecord::Unmeasured { .. }));
        assert_eq!(fine_gain_points(&Grid::Step { lo: 0.0, hi: 0.5, step: 0.5 }).unwrap(), [0.0, 0.1, 0.2, 0.3, 0.4, 0.5]);
    }

    #[test]
    fn gain_grid_comparison_rejects_a_profile_point_missing_from_readback() {
        let device = super::super::fake(super::super::one_obx());
        let dir = super::super::TempDir::new();
        let doc = super::super::bench_profile(&*device, &dir, Json::Object(Default::default()), super::super::bench_envelope(&*device), false);
        let binding = radio_binding(&doc).unwrap();
        let description = description(&binding, ezsdr_radio_uhd::profile::DEFAULT_BLOCK_LEN, &ezsdr_radio_uhd::profile::terms()).unwrap();
        let mut record = build_record(Source::Fake, &binding, &*device, &rehearsal_metadata(), MeasurementGroup::default(), &description).unwrap();
        record.source = Source::Hardware;
        record.conditions.push(raw_condition("coercion"));
        let key = Key::parse("radio.rx.c0.gain_db").unwrap();
        let all = grid_points(&description.rx[0].gain).unwrap();
        let set = |record: &mut Record, values: Vec<f64>| {
            let row = record.values.iter_mut().find(|row| match row {
                ValueRecord::Measured { key: found, .. } | ValueRecord::Unmeasured { key: found, .. } => found == &key,
            }).unwrap();
            *row = ValueRecord::Measured {
                key: key.clone(), worst: model::Worst::Grid(model::GridValue::Values { values }), trials: 64,
                condition: "coercion".to_owned(), test: "tests/hardware.rs::hw_measurements_record".to_owned(),
            };
        };
        set(&mut record, all.clone());
        assert!(model::compare(&record, &description).iter().all(|violation| violation.key != key.as_str()));
        let mut missing = all;
        missing.retain(|value| *value != 0.5);
        set(&mut record, missing);
        assert!(model::compare(&record, &description).iter().any(|violation| violation.key == key.as_str()
            && violation.reason.starts_with("profile grid is not a subset")));
    }

    fn synthetic_clock(domain: ClockDomainId, stream: &str, origin: i64, ended_at: Option<i64>) -> SampleClockRecord {
        let root = ClockDomainId::local(20);
        SampleClockRecord {
            stream: ezsdr_kernel::id::ResourceId::parse(stream).unwrap(),
            domain,
            root,
            root_ticks_per_tick: ezsdr_kernel::time::Rational::new(1, 1).unwrap(),
            origin: TimePoint::new(root, origin),
            ended_at: ended_at.map(|tick| TimePoint::new(root, tick)),
            nominal_rate: ezsdr_kernel::time::Rational::new(1, 1).unwrap(),
        }
    }

    fn synthetic_tx_plan(start_tick: i64, end_tick: i64, target_tick: i64) -> TrialPlan {
        let target = TimePoint::new(ClockDomainId::local(21), target_tick);
        TrialPlan {
            start_tick, end_tick, admitted: true, expected_target: Some(target), expected_samples: 4,
            expected_end_tick: target_tick + 4, booked_tick: Some(start_tick), completion: TrialCompletion::TransmitBurstAck,
            requires_preemption_evidence: false,
            capture_index: None, capture_target: None, cold_capture: false,
        }
    }

    fn synthetic_burst(target: TimePoint, samples: u64, end: &str) -> Json {
        json!([{ "target": target, "samples": samples, "end": end }])
    }

    fn synthetic_radio_event(kind: &str, tick: i64, payload: Json) -> Json {
        json!([{ "kind": kind, "time": TimePoint::new(ClockDomainId::local(20), tick), "payload": payload }])
    }

    #[test]
    fn effective_lead_uses_the_action_receipt_instead_of_a_later_clock_read() {
        let target = 300;
        let root = ClockDomainId::local(21);
        let receipt = 100;
        let later_post_submit_read = 150;
        assert_eq!(effective_lead_ns(target, TimePoint::new(root, receipt), root, 1_000_000_000).unwrap(), (receipt, 200));
        assert_eq!(effective_lead_ns(target, TimePoint::new(root, later_post_submit_read), root, 1_000_000_000).unwrap(),
            (later_post_submit_read, 150));
    }

    #[test]
    fn cold_observations_record_both_rates_and_each_transition_direction() {
        let stage = cold_candidate_stage_json("cold-1msps-2msps", 1, &[TrialClass::Success; 2], &[None, None], 1_000_000, 2_000_000, 2);
        assert_eq!(stage["rate_pair_hz"], json!([1_000_000, 2_000_000]));
        assert_eq!(stage["trial_transitions_hz"], json!([[1_000_000, 2_000_000], [2_000_000, 1_000_000]]));
    }

    fn complete(start: i64) -> TrialEvidence {
        let mut trial = TrialEvidence::new(start, start + 10, 4, start + 10, TrialCompletion::TransmitBurstAck);
        trial.record = Some(TrialRecord { samples: 4, end_tick: start + 10 });
        trial.evidence.push(EvidenceKind::BurstAck);
        trial
    }

    fn complete_receive(start: i64) -> TrialEvidence {
        let mut trial = TrialEvidence::new(start, start + 10, 4, start + 10, TrialCompletion::ReceiveCapture);
        trial.record = Some(TrialRecord { samples: 4, end_tick: start + 10 });
        trial
    }

    #[test]
    fn classify_trial_failure_origin_and_completion_evidence() {
        let mut trial = complete(10);
        assert_eq!(classify_trial(&trial), TrialClass::Success);
        assert_eq!(classify_trial(&complete_receive(10)), TrialClass::Success, "an RX capture ends from its complete record, not a TX BurstAck");
        trial.admitted = false;
        assert_eq!(classify_trial(&trial), TrialClass::AdmissionRejection);

        for error in [ProviderTimeError::Drop, ProviderTimeError::SendAsap, ProviderTimeError::Refused, ProviderTimeError::PlanViolation] {
            let mut trial = complete(10);
            trial.evidence.push(EvidenceKind::ProviderTimeError(error));
            assert_eq!(classify_trial(&trial), TrialClass::ProviderFailure, "{error:?}");
        }
        for error in [EvidenceKind::LateCommand, EvidenceKind::CommandRejected, EvidenceKind::CommandQueueFull] {
            let mut trial = complete(10);
            trial.evidence.push(error);
            assert_eq!(classify_trial(&trial), TrialClass::ProviderFailure, "{error:?}");
        }
        for error in [
            EvidenceKind::ProviderTimeError(ProviderTimeError::LateAtDevice),
            EvidenceKind::AsyncTimeError,
            EvidenceKind::Underflow,
            EvidenceKind::UnderflowInPacket,
            EvidenceKind::SequenceError,
            EvidenceKind::SequenceErrorInBurst,
        ] {
            let mut trial = complete(10);
            trial.evidence.push(error);
            assert_eq!(classify_trial(&trial), TrialClass::DeviceFailure, "{error:?}");
        }
        let mut ambiguous = complete(10);
        ambiguous.evidence.extend([
            EvidenceKind::ProviderTimeError(ProviderTimeError::Drop),
            EvidenceKind::AsyncTimeError,
        ]);
        assert_eq!(classify_trial(&ambiguous), TrialClass::Unattributable);
    }

    #[test]
    fn incomplete_or_unattributable_trials_never_pass() {
        let mut missing = complete(0);
        missing.record = None;
        assert_eq!(classify_trial(&missing), TrialClass::Incomplete);
        let mut no_ack = complete(0);
        no_ack.evidence.clear();
        assert_eq!(classify_trial(&no_ack), TrialClass::Incomplete);
        let mut wrong_samples = complete(0);
        wrong_samples.record.as_mut().unwrap().samples -= 1;
        assert_eq!(classify_trial(&wrong_samples), TrialClass::Incomplete);
        let mut wrong_end = complete(0);
        wrong_end.record.as_mut().unwrap().end_tick -= 1;
        assert_eq!(classify_trial(&wrong_end), TrialClass::Incomplete);
        let mut short_send = complete(0);
        short_send.evidence.push(EvidenceKind::ShortSendRejected);
        assert_eq!(classify_trial(&short_send), TrialClass::Incomplete);

        let mut trials = vec![complete(0), complete(20)];
        attribute_evidence(&mut trials, TimedEvidence { tick: Some(10), span: None, kind: EvidenceKind::BurstAck });
        assert!(trials[0].unattributable, "an interval-end report belongs to neither adjacent trial");
        assert_eq!(classify_trial(&trials[0]), TrialClass::Unattributable);
        trials[0].unattributable = false;
        trials[0].open = true;
        attribute_evidence(&mut trials, TimedEvidence { tick: None, span: None, kind: EvidenceKind::BurstAck });
        assert!(trials[0].unattributable, "tick-free report fails the open trial");
        trials[0].open = false;
        attribute_evidence(&mut trials, TimedEvidence { tick: Some(99), span: None, kind: EvidenceKind::BurstAck });
        assert!(trials[1].unattributable, "an out-of-range report fails the preceding trial");
        let mut spanning = vec![complete(0), complete(20)];
        attribute_evidence(&mut spanning, TimedEvidence { tick: None, span: Some((5, 25)), kind: EvidenceKind::BurstAck });
        assert!(spanning.iter().all(|trial| trial.unattributable), "an acknowledgement spanning trials is ambiguous");
        assert!(spanning.iter().all(|trial| classify_trial(trial) == TrialClass::Unattributable));

        for span in [(5, 15), (-5, 5)] {
            let mut crossing = vec![complete(0)];
            attribute_evidence(&mut crossing, TimedEvidence { tick: None, span: Some(span), kind: EvidenceKind::BurstAck });
            assert!(crossing[0].unattributable, "a span crossing one trial edge is not contained: {span:?}");
        }
        let mut contained = vec![complete(0)];
        attribute_evidence(&mut contained, TimedEvidence { tick: None, span: Some((5, 10)), kind: EvidenceKind::BurstAck });
        assert!(!contained[0].unattributable, "a span contained in its trial is attributable");

        let mut before_first = vec![complete(10)];
        attribute_evidence(&mut before_first, TimedEvidence { tick: Some(0), span: None, kind: EvidenceKind::LateCommand });
        assert!(before_first[0].unattributable, "failure evidence before the first trial invalidates it");

        let mut matched_async = vec![complete(0)];
        attribute_evidence(&mut matched_async, TimedEvidence { tick: Some(5), span: None, kind: EvidenceKind::AsyncTimeError });
        assert_eq!(classify_trial(&matched_async[0]), TrialClass::DeviceFailure, "a report inside the trial is device evidence");
        let mut unmatched_async = vec![complete(0)];
        attribute_evidence(&mut unmatched_async, TimedEvidence { tick: Some(99), span: None, kind: EvidenceKind::AsyncTimeError });
        assert_eq!(classify_trial(&unmatched_async[0]), TrialClass::Unattributable, "a report outside every trial cannot be credited to it");
    }

    #[test]
    fn manifest_evidence_classifies_tx_ack_and_out_of_range_failure_evidence() {
        let clock = synthetic_clock(ClockDomainId::local(21), "usrp/tx/s0", 0, None);
        let clocks = [clock];
        let plan = synthetic_tx_plan(90, 120, 100);
        let burst = synthetic_burst(plan.expected_target.unwrap(), 4, "eob");
        let ack = json!([{ "code": "BurstAck", "tick": 102 }]);
        let empty = json!([]);
        let no_artifacts = json!({});
        let evidence = |plan: &TrialPlan, bursts: &Json, async_rows: &Json, rejected: &Json, events: &Json| {
            extract_trial_evidence(bursts, async_rows, rejected, events, &no_artifacts, &clocks, std::slice::from_ref(plan)).unwrap()
        };

        let completed = evidence(&plan, &burst, &ack, &empty, &empty);
        assert_eq!(classify_trial(&completed[0]), TrialClass::Success, "the ACK need not equal the exclusive sample end");
        assert_eq!(completed[0].burst_ack_ticks, [102]);
        let tx_stage = tx_candidate_stage_json("tx-1msps", 1, &[TrialClass::Success], &[Some(3)], &completed);
        assert_eq!(tx_stage["trials"][0]["expected_end_tick"], 104);
        assert_eq!(tx_stage["trials"][0]["burst_ack_offsets_from_expected_end_ticks"], json!([-2]));
        assert_eq!(classify_trial(&evidence(&plan, &burst, &empty, &empty, &empty)[0]), TrialClass::Incomplete,
            "a missing ACK does not complete a recorded burst");
        let multiple_acks = json!([{ "code": "BurstAck", "tick": 101 }, { "code": "BurstAck", "tick": 103 }]);
        assert_eq!(classify_trial(&evidence(&plan, &burst, &multiple_acks, &empty, &empty)[0]), TrialClass::Unattributable,
            "multiple ACKs in one isolated trial cannot be assigned");
        let before_trial = synthetic_radio_event("radio.RX_OVERFLOW", 89, json!({}));
        assert_eq!(classify_trial(&evidence(&plan, &burst, &ack, &empty, &before_trial)[0]), TrialClass::Unattributable,
            "a known failure before the first trial invalidates it");
        let after_trial = synthetic_radio_event("radio.RX_OVERFLOW", 120, json!({}));
        assert_eq!(classify_trial(&evidence(&plan, &burst, &ack, &empty, &after_trial)[0]), TrialClass::Unattributable,
            "a known failure after the last trial invalidates it");

        let mut rejected_plan = plan.clone();
        rejected_plan.admitted = false;
        assert_eq!(classify_trial(&evidence(&rejected_plan, &empty, &empty, &empty, &empty)[0]), TrialClass::AdmissionRejection);

        let provider_error = synthetic_radio_event("radio.TIME_ERROR", 95, json!({ "outcome": "drop", "target": plan.expected_target }));
        assert_eq!(classify_trial(&evidence(&plan, &empty, &empty, &empty, &provider_error)[0]), TrialClass::ProviderFailure);
        let device_error = synthetic_radio_event("radio.RX_OVERFLOW", 95, json!({}));
        assert_eq!(classify_trial(&evidence(&plan, &empty, &empty, &empty, &device_error)[0]), TrialClass::DeviceFailure);
        assert_eq!(classify_trial(&evidence(&plan, &burst, &empty, &empty, &empty)[0]), TrialClass::Incomplete, "a burst record without its ACK is incomplete");
        let truncated = synthetic_burst(plan.expected_target.unwrap(), 4, "discontinuity");
        assert_eq!(classify_trial(&evidence(&plan, &truncated, &ack, &empty, &empty)[0]), TrialClass::Incomplete, "a started but non-EOB burst is incomplete");

        let tickless = json!([{ "code": "TimeError", "tick": null }]);
        assert_eq!(classify_trial(&evidence(&plan, &empty, &tickless, &empty, &empty)[0]), TrialClass::Unattributable);
        let unknown_time_error = synthetic_radio_event("radio.TIME_ERROR", 95, json!({ "outcome": "future_outcome", "target": plan.expected_target }));
        assert_eq!(classify_trial(&evidence(&plan, &burst, &ack, &empty, &unknown_time_error)[0]), TrialClass::Unattributable,
            "an unknown TIME_ERROR outcome cannot be ignored");
        let short_send = json!([{ "reason": "UR-22: a send did not complete within 1 s" }]);
        assert_eq!(classify_trial(&evidence(&plan, &burst, &ack, &short_send, &empty)[0]), TrialClass::Incomplete);

        let first = synthetic_tx_plan(90, 104, 100);
        let second = synthetic_tx_plan(104, 220, 200);
        let crossing_ack = json!([{ "code": "BurstAck", "tick": 104 }]);
        let crossed = extract_trial_evidence(
            &synthetic_burst(first.expected_target.unwrap(), 4, "eob"), &crossing_ack, &empty, &empty, &no_artifacts,
            &clocks, &[first, second],
        ).unwrap();
        assert!(crossed.iter().all(|trial| classify_trial(trial) == TrialClass::Incomplete), "a boundary ACK before the next target cannot complete either trial");
    }

    #[test]
    fn preemption_requires_the_repeat_record_to_reach_the_interrupt_target() {
        let clock = synthetic_clock(ClockDomainId::local(21), "usrp/tx/s0", 0, None);
        let clocks = [clock];
        let mut plan = synthetic_tx_plan(90, 120, 100);
        plan.requires_preemption_evidence = true;
        let interrupt = synthetic_burst(plan.expected_target.unwrap(), 4, "eob");
        let ack = json!([{ "code": "BurstAck", "tick": 102 }]);
        let empty = json!([]);
        let no_artifacts = json!({});
        let no_repeat = extract_trial_evidence(&interrupt, &ack, &empty, &empty, &no_artifacts, &clocks, std::slice::from_ref(&plan)).unwrap();
        assert_eq!(classify_trial(&no_repeat[0]), TrialClass::Incomplete,
            "a complete interrupting burst cannot prove that a repeat was running");

        let repeat = json!([
            { "target": TimePoint::new(ClockDomainId::local(21), 50), "samples": 50, "end": "eob" },
            { "target": plan.expected_target.unwrap(), "samples": 4, "end": "eob" }
        ]);
        let complete = extract_trial_evidence(&repeat, &ack, &empty, &empty, &no_artifacts, &clocks, std::slice::from_ref(&plan)).unwrap();
        assert!(complete[0].repeat_ended_at_target);
        assert_eq!(classify_trial(&complete[0]), TrialClass::Success);

        let early = json!([
            { "target": TimePoint::new(ClockDomainId::local(21), 50), "samples": 49, "end": "eob" },
            { "target": plan.expected_target.unwrap(), "samples": 4, "end": "eob" }
        ]);
        let ended_early = extract_trial_evidence(&early, &ack, &empty, &empty, &no_artifacts, &clocks, std::slice::from_ref(&plan)).unwrap();
        assert!(!ended_early[0].repeat_ended_at_target);
        assert_eq!(classify_trial(&ended_early[0]), TrialClass::Incomplete, "a repeat ending one sample before the interrupt proves no preemption");

        let before = synthetic_radio_event("radio.TX_UNDERFLOW", 89, json!({}));
        let failed_setup = extract_trial_evidence(&repeat, &ack, &empty, &before, &no_artifacts, &clocks, &[plan]).unwrap();
        assert_eq!(classify_trial(&failed_setup[0]), TrialClass::Unattributable,
            "an underflow before the first trial is retained against that trial");
    }

    #[test]
    fn cold_capture_evidence_requires_two_contiguous_clock_segments() {
        let old_domain = ClockDomainId::local(21);
        let new_domain = ClockDomainId::local(22);
        let old = synthetic_clock(old_domain, "usrp/rx/s0", 0, Some(200));
        let new = synthetic_clock(new_domain, "usrp/rx/s0", 250, None);
        let target = TimePoint::new(old_domain, 100);
        let plan = TrialPlan {
            start_tick: 90, end_tick: 400, admitted: true, expected_target: None, expected_samples: 200,
            expected_end_tick: 0, booked_tick: Some(150), completion: TrialCompletion::ReceiveCapture,
            requires_preemption_evidence: false,
            capture_index: Some(0), capture_target: Some(target), cold_capture: true,
        };
        let map = |domain, first, end| json!({
            "domain": domain, "gaps": [], "channel_gaps": [],
            "first": TimePoint::new(domain, first), "end": TimePoint::new(domain, end),
        });
        let artifacts = json!({ "rec": [{ "partial": false, "continuity": [map(old_domain, 100, 200), map(new_domain, 0, 100)] }] });
        let empty = json!([]);
        let evidence = extract_trial_evidence(&empty, &empty, &empty, &empty, &artifacts, &[old.clone(), new], std::slice::from_ref(&plan)).unwrap();
        assert_eq!(classify_trial(&evidence[0]), TrialClass::Success);

        let mut with_gap = artifacts.clone();
        with_gap["rec"][0]["continuity"][0]["gaps"] = json!([{ "len": 1 }]);
        let evidence = extract_trial_evidence(&empty, &empty, &empty, &empty, &with_gap, std::slice::from_ref(&old), std::slice::from_ref(&plan)).unwrap();
        assert_eq!(classify_trial(&evidence[0]), TrialClass::Incomplete);
        let wrong_origin = synthetic_clock(new_domain, "usrp/rx/s0", 200, None);
        let evidence = extract_trial_evidence(&empty, &empty, &empty, &empty, &artifacts, &[old, wrong_origin], &[plan]).unwrap();
        assert_eq!(classify_trial(&evidence[0]), TrialClass::Incomplete, "the new clock origin must follow the old clock's cut");
    }

    #[test]
    fn throughput_success_requires_sample_tolerance_and_no_overflow() {
        let clock = synthetic_clock(ClockDomainId::local(21), "usrp/rx/s0", 100, Some(1_200));
        let (interval, expected) = expected_rx_samples(&clock, 1_000, 1_000).unwrap();
        assert_eq!((interval, expected), (1_100, 1_100), "the recorded interval includes delayed shutdown");
        let missing_end = synthetic_clock(ClockDomainId::local(21), "usrp/rx/s0", 100, None);
        assert_eq!(expected_rx_samples(&missing_end, 1_000, 1_000), None,
            "without an ended SampleClock there is no measured interval");
        assert_eq!(throughput_trial_class(1_100, Some(expected), 100, false), TrialClass::Success);
        assert_eq!(throughput_trial_class(1_200, Some(expected), 100, false), TrialClass::Success,
            "a one-packet difference is within the permitted tolerance");
        assert_eq!(throughput_trial_class(900, Some(expected), 100, false), TrialClass::Incomplete,
            "a sample deficit beyond one packet remains incomplete");
        assert_eq!(throughput_trial_class(1_100, None, 100, false), TrialClass::Incomplete,
            "a SampleClock without an end cannot define an expected count");
        assert_eq!(throughput_trial_class(1_100, Some(expected), 100, true), TrialClass::DeviceFailure,
            "an overflow fails even with the expected sample count");
    }

    #[test]
    fn tx_start_lead_requires_an_early_switch_and_no_failure_evidence() {
        let trial = TrialEvidence::new(0, 200, 0, 0, TrialCompletion::ReceiveCapture);
        assert_eq!(tx_start_trial_class(&trial, &json!([{ "what": "tx_switch", "origin": 100, "at": 99 }]), Some(100)).0, TrialClass::Success,
            "the permitted start-lead observation is tx_switch.at < origin; no thread-timed burst is reserved");
        assert_eq!(tx_start_trial_class(&trial, &json!([{ "what": "tx_switch", "origin": 100, "at": 100 }]), Some(100)).0, TrialClass::Incomplete,
            "a switch at the origin is not early");
        assert_eq!(tx_start_trial_class(&trial, &json!([]), Some(100)).0, TrialClass::Incomplete,
            "a missing segment-start record is not success");
        assert_eq!(tx_start_trial_class(&trial, &json!([{ "what": "tx_switch", "origin": 100, "at": 99 }]), None).0, TrialClass::Incomplete,
            "without a newly registered clock there is no measured origin");
        let mut rejected = trial.clone();
        rejected.admitted = false;
        assert_eq!(tx_start_trial_class(&rejected, &json!([]), None).0, TrialClass::AdmissionRejection,
            "a rejected rate-change action is classified instead of aborting the measurement");
        let mut failed = trial;
        failed.evidence.push(EvidenceKind::LateCommand);
        assert_eq!(tx_start_trial_class(&failed, &json!([{ "what": "tx_switch", "origin": 100, "at": 99 }]), Some(100)).0, TrialClass::ProviderFailure);
        let stage = tx_start_candidate_stage_json("cold-390625-1msps", 1_000_000, (390_625, 1_000_000), 200_000_000,
            &[TrialClass::Success; 2], &[[1_000_000, 390_625], [390_625, 1_000_000]], &[(Some(200), Some(100)), (Some(400), Some(300))]);
        assert_eq!(stage["tx_rate_pair_hz"], json!([390_625, 1_000_000]));
        assert_eq!(stage["trial_transitions_hz"], json!([[1_000_000, 390_625], [390_625, 1_000_000]]));
        assert!(stage["burst_criterion"].as_str().unwrap().contains("omitted"));
    }

    #[test]
    fn tx_start_lead_value_uses_the_successful_suffix_intersection() {
        let stages = |failed: &[u64]| [1, 2, 5].into_iter().map(|value| CandidateStage {
            value, trials: vec![if failed.contains(&value) { TrialClass::Incomplete } else { TrialClass::Success }; 2],
        }).collect();
        let candidates = BTreeMap::from([
            ("condition-a".to_owned(), stages(&[1])),
            ("condition-b".to_owned(), stages(&[1, 2])),
        ]);
        let summary = summarize_tx_start_lead(&candidates, 2).unwrap();
        assert!(matches!(tx_start_value(&summary, &candidates, 2, "test").unwrap(), ValueRecord::Measured {
            key, worst: model::Worst::Number(5.0), condition, trials: 2, ..
        } if key.as_str() == "radio.timing.tx_start_lead_ns" && condition == "condition-b"));
    }

    #[test]
    fn overflow_evidence_requires_one_overrun_one_matching_gap_and_the_measured_rate() {
        let domain = ClockDomainId::local(21);
        let mut clock = synthetic_clock(domain, "usrp/rx/s0", 0, None);
        clock.nominal_rate = ezsdr_kernel::time::Rational::new(1_000_000, 1).unwrap();
        let clocks = [clock.clone()];
        let event = |cause: &str, lost: u64| json!([{ "kind": "radio.RX_OVERFLOW", "payload": { "cause": cause, "lost": lost } }]);
        let gap = |len: u64, lost: Json| json!({ "start": TimePoint::new(domain, 2_000), "cause": { "kind": "overflow_restart" }, "len": len, "lost": lost });
        let artifacts = |gaps: Json| json!({ "rec": [{ "partial": false, "continuity": [{
            "domain": domain, "first": TimePoint::new(domain, 0), "end": TimePoint::new(domain, 3_000), "gaps": gaps, "channel_gaps": []
        }] }] });
        let extract = |events: &Json, artifacts: &Json, rate| extract_overflow_trial(events, artifacts, &clocks, rate, 1, 1);
        let complete = extract(&event("overrun", 50), &artifacts(json!([gap(50, json!(50))])), 1_000_000);
        assert_eq!(complete.class, TrialClass::Success);
        assert_eq!(complete.gap_ns, Some(50_000));
        assert_eq!(complete.gaps[0].len, 50);
        assert_eq!(complete.restart_after_resume.nanoseconds, Some(50_000));
        let mut partial_capture = artifacts(json!([gap(50, json!(50))]));
        partial_capture["rec"][0]["partial"] = json!(true);
        assert_eq!(extract(&event("overrun", 50), &partial_capture, 1_000_000).class, TrialClass::Incomplete,
            "a partial capture cannot provide a complete overflow trial");
        assert_eq!(samples_to_ns_ceil(1, 3), Some(333_333_334), "fractional sample periods round upward conservatively");
        assert_eq!(extract(&json!([]), &artifacts(json!([gap(50, json!(50))])), 1_000_000).class, TrialClass::Incomplete,
            "a capture gap without exactly one RX_OVERFLOW is not evidence");
        assert_eq!(extract(&json!([event("overrun", 50)[0].clone(), event("overrun", 50)[0].clone()]),
            &artifacts(json!([gap(50, json!(50))])), 1_000_000).class, TrialClass::Incomplete,
            "duplicate overflow reports are not collapsed");
        assert_eq!(extract(&event("sequence", 50), &artifacts(json!([gap(50, json!(50))])), 1_000_000).class, TrialClass::Incomplete,
            "a sequence report cannot stand in for an overrun");
        assert_eq!(extract(&event("overrun", 50), &artifacts(json!([])), 1_000_000).class, TrialClass::Incomplete);
        assert_eq!(extract(&event("overrun", 50), &artifacts(json!([gap(50, json!(50)), gap(60, json!(60))])), 1_000_000).class, TrialClass::Incomplete);
        assert_eq!(extract(&event("overrun", 51), &artifacts(json!([gap(50, json!(50))])), 1_000_000).class, TrialClass::Incomplete,
            "the event's lost count must equal the continuity gap");
        assert_eq!(extract(&event("overrun", 50), &artifacts(json!([gap(50, json!(49))])), 1_000_000).class, TrialClass::Incomplete,
            "the gap must report losing its entire length");
        assert_eq!(extract(&event("overrun", 50), &artifacts(json!([gap(50, json!(50))])), 2_000_000).class, TrialClass::Incomplete,
            "the clock must prove the requested rate");
    }

    #[test]
    fn overflow_restart_after_resume_estimate_uses_gap_end_and_nulls_negative_results() {
        let estimate = restart_after_resume_estimate(100, 1_050, 250, 1_000, 500, 500);
        assert_eq!((estimate.gap_end_sample, estimate.reader_resume_sample, estimate.nanoseconds), (Some(1_300), Some(1_100), Some(200_000_000)));
        let zero = restart_after_resume_estimate(0, 1, 0, 3, 1, 0);
        assert_eq!((zero.reader_resume_sample, zero.nanoseconds), (Some(1), Some(0)), "fractional sample durations round up to the first sample at or after resume");
        let negative = restart_after_resume_estimate(100, 500, 100, 1_000, 500, 500);
        assert_eq!((negative.gap_end_sample, negative.reader_resume_sample, negative.nanoseconds), (Some(600), Some(1_100), None));
        assert!(negative.reason.as_deref().is_some_and(|reason| reason.contains("before the estimated reader resume")));
    }

    #[test]
    fn overflow_wait_horizon_allows_capture_to_finish_after_the_stall() {
        assert_eq!(overflow_wait_ms(500, 20_000).unwrap(), 43_000,
            "the horizon includes the TX/RX start lead, pre-stall interval, stall, one stall-length drain allowance, and margin");
        assert!(overflow_wait_ms(500, u64::MAX).is_err(), "horizon arithmetic must reject overflow");
    }

    #[test]
    fn overflow_summary_keeps_observations_and_the_key_is_not_a_record_value() {
        let complete = |gap_ns, restart_ns| OverflowTrialEvidence {
            class: TrialClass::Success, overflow_count: 1, overflow_causes: vec!["overrun".to_owned()],
            payload_lost: vec![Some(1)], gaps: vec![], gap_ns: Some(gap_ns),
            restart_after_resume: OverflowRestartEstimate { gap_end_sample: None, reader_resume_sample: None, nanoseconds: Some(restart_ns), reason: None },
            sample_clock_rate_matches: true,
        };
        let candidates = BTreeMap::from([
            (1_000_000, vec![complete(50_000, 5_000), complete(51_000, 6_000)]),
            (10_000_000, vec![complete(60_000, 7_000), complete(55_000, 4_000)]),
        ]);
        let summary = summarize_overflow_gaps(&candidates, 2).unwrap();
        assert_eq!((summary.worst_gap_ns, summary.limiting_rate_hz), (Some(60_000), Some(10_000_000)));
        assert_eq!((summary.maximum_restart_after_resume_ns, summary.maximum_restart_rate_hz), (Some(7_000), Some(10_000_000)));
        assert_eq!(summary.maximum_restart_by_rate, BTreeMap::from([(1_000_000, Some(6_000)), (10_000_000, Some(7_000))]));

        let mut incomplete = complete(55_000, 8_000);
        incomplete.class = TrialClass::Incomplete;
        incomplete.gap_ns = None;
        let under_trialed = BTreeMap::from([
            (1_000_000, vec![complete(50_000, 5_000), complete(51_000, 6_000)]),
            (10_000_000, vec![complete(60_000, 7_000), incomplete]),
        ]);
        let summary = summarize_overflow_gaps(&under_trialed, 2).unwrap();
        assert_eq!(summary.worst_gap_ns, Some(60_000), "observations retain the maximum even when the required trial count is short");
        assert_eq!(summary.maximum_restart_after_resume_ns, Some(7_000), "incomplete trial estimates are not included in the summary");
        assert!(summary.unmeasured_reason.as_deref().unwrap().contains("10000000 Hz"));
    }

    #[test]
    fn tx_throughput_uses_the_burst_target_ack_elapsed_and_minimum_samples() {
        let domain = ClockDomainId::local(21);
        let mut clock = synthetic_clock(domain, "usrp/tx/s0", 100, None);
        clock.nominal_rate = ezsdr_kernel::time::Rational::new(1_000, 1).unwrap();
        let target = TimePoint::new(domain, 2_000);
        let bursts = json!([{ "target": target, "samples": 1_000, "end": "stop" }]);
        let empty = json!([]);
        let ack = json!([{ "code": "BurstAck", "tick": 3_100 }]);
        let evaluate = |samples, async_reports: &Json, events: &Json, bursts: &Json, rejected: &Json, scheduled_target| {
            tx_throughput_evidence(TxThroughputInput {
                stats: &json!({ "tx_samples": samples }), async_reports, events, bursts, rejected,
                clocks: &[clock.clone()], scheduled_target, rate_hz: 1_000, master_clock_hz: 1_000, duration_secs: 1,
            })
        };
        let complete = evaluate(1_000, &ack, &empty, &bursts, &empty, target);
        assert_eq!(complete.class, TrialClass::Success, "an unended TX SampleClock does not prevent a completed repeat measurement");
        assert_eq!((complete.burst_start_tick, complete.burst_ack_tick, complete.ack_elapsed_samples,
            complete.minimum_samples, complete.tx_samples_minus_ack_elapsed_samples), (Some(2_100), Some(3_100), Some(1_000), Some(1_000), Some(0)));
        assert_eq!(evaluate(1_100, &ack, &empty, &bursts, &empty, target).class, TrialClass::Success,
            "already-handed-over in-flight samples are not rejected by an upper bound");
        assert_eq!(evaluate(999, &ack, &empty, &bursts, &empty, target).class, TrialClass::Incomplete,
            "fewer than D times rate samples cannot pass");
        assert_eq!(evaluate(1_000, &json!([]), &empty, &bursts, &empty, target).class, TrialClass::Incomplete,
            "no BurstAck leaves the repeat incomplete");
        assert_eq!(evaluate(1_000, &json!([{ "code": "BurstAck", "tick": 3_100 }, { "code": "BurstAck", "tick": 3_200 }]),
            &empty, &bursts, &empty, target).class, TrialClass::Unattributable, "multiple acknowledgements cannot be assigned to one interval");
        assert_eq!(evaluate(1_000, &json!([{ "code": "BurstAck", "tick": 1_500 }]), &empty, &bursts, &empty, target).class,
            TrialClass::Incomplete, "an ACK after clock arm but before burst start plus D is too early");
        assert_eq!(evaluate(1_000, &json!([{ "code": "BurstAck", "tick": 3_099 }]), &empty, &bursts, &empty, target).class,
            TrialClass::Incomplete, "an ACK after burst start but before the D-second boundary is too early");
        assert_eq!(evaluate(1_000, &ack, &empty, &bursts, &empty, TimePoint::new(domain, 1_999)).class,
            TrialClass::Incomplete, "the run must record the scheduled burst target");
        let underflow = json!([{ "kind": "radio.TX_UNDERFLOW" }]);
        assert_eq!(evaluate(1_000, &ack, &underflow, &bursts, &empty, target).class, TrialClass::DeviceFailure,
            "TX_UNDERFLOW fails an otherwise complete interval");
        for code in ["Underflow", "UnderflowInPacket", "SeqError", "SeqErrorInBurst", "TimeError"] {
            assert_eq!(evaluate(1_000, &json!([{ "code": code }]), &empty, &bursts, &empty, target).class,
                TrialClass::DeviceFailure, "{code} is device failure evidence");
        }
        assert_eq!(evaluate(1_000, &ack, &empty, &bursts, &json!([{ "reason": "short send" }]), target).class, TrialClass::DeviceFailure,
            "short-send rejections are device failures");
        assert_eq!(evaluate(1_000, &ack, &empty, &json!([bursts[0].clone(), bursts[0].clone()]), &empty, target).class,
            TrialClass::Unattributable, "multiple burst records cannot provide one start boundary");
        assert_eq!(tx_bytes_per_second(10_000_000, 4).unwrap(), 40_000_000);
        let stages = vec![
            CandidateStage { value: 40_000_000, trials: vec![TrialClass::Success] },
            CandidateStage { value: 80_000_000, trials: vec![TrialClass::DeviceFailure] },
        ];
        let summary = summarize_tx_throughput(&stages).unwrap();
        assert!(matches!(tx_capacity_value(&summary, &[10_000_000, 20_000_000], &stages, "test").unwrap(), ValueRecord::Measured {
            key, worst: model::Worst::Number(40_000_000.0), condition, ..
        } if key.as_str() == "radio.perf.tx_bytes_per_s" && condition == "tx-10msps"));
    }

    #[test]
    fn loopback_success_separates_waveform_from_requested_sample_time() {
        assert_eq!(loopback_values(&[TrialClass::Success], &[true], &[true]), (true, true));
        assert_eq!(loopback_values(&[TrialClass::Success], &[false], &[true]), (true, false));
        assert_eq!(loopback_values(&[TrialClass::Success], &[true], &[false]), (false, true));
        assert_eq!(loopback_values(&[TrialClass::DeviceFailure], &[true], &[true]), (false, false));
    }

    #[test]
    fn measured_rows_keep_the_selected_boundary_and_limiting_condition() {
        let stages = |failed: &[u64]| [1, 2, 3].into_iter().map(|value| CandidateStage {
            value, trials: vec![if failed.contains(&value) { TrialClass::ProviderFailure } else { TrialClass::Success }; 2],
        }).collect();
        let candidates = BTreeMap::from([
            ("cold-fast".to_owned(), stages(&[1])),
            ("cold-slow".to_owned(), stages(&[1, 2])),
        ]);
        let summary = summarize_candidates(SummaryMode::MinimumLead, &candidates, 2).unwrap();
        let measured = numeric_value("radio.timing.rx_start_lead_ns", &summary, &candidates, 2,
            "tests/hardware.rs::hw_measurements_record").unwrap();
        assert!(matches!(measured, ValueRecord::Measured {
            key, worst: model::Worst::Number(3.0), trials: 2, condition, test
        } if key.as_str() == "radio.timing.rx_start_lead_ns" && condition == "cold-slow"
            && test == "tests/hardware.rs::hw_measurements_record"));

        let rates = [10_000_000, 20_000_000, 40_000_000];
        let capacity = vec![
            CandidateStage { value: 20, trials: vec![TrialClass::Success] },
            CandidateStage { value: 40, trials: vec![TrialClass::Success] },
            CandidateStage { value: 80, trials: vec![TrialClass::DeviceFailure] },
        ];
        let candidates = BTreeMap::from([("rx-throughput".to_owned(), capacity.clone())]);
        let summary = summarize_candidates(SummaryMode::Capacity, &candidates, 1).unwrap();
        let measured = capacity_value(&summary, &rates, &capacity, "tests/hardware.rs::hw_measurements_record").unwrap();
        assert!(matches!(measured, ValueRecord::Measured {
            key, worst: model::Worst::Number(40.0), trials: 1, condition, ..
        } if key.as_str() == "radio.perf.rx_bytes_per_s" && condition == "rx-20msps"));
    }

    #[test]
    fn candidate_summaries_use_successful_suffix_prefix_trial_counts_and_condition_intersection() {
        let pass = || CandidateStage { value: 0, trials: vec![TrialClass::Success; 2] };
        let stages = |values: &[u64], pass_mask: &[bool]| values.iter().zip(pass_mask).map(|(value, passed)| {
            CandidateStage { value: *value, trials: vec![if *passed { TrialClass::Success } else { TrialClass::ProviderFailure }; 2] }
        }).collect();
        let mut conditions = BTreeMap::from([("load-none".to_owned(), stages(&[1, 2, 3, 4], &[true, false, true, true]))]);
        let lead = summarize_candidates(SummaryMode::MinimumLead, &conditions, 2).unwrap();
        assert_eq!(lead.worst, Some(3));
        conditions.insert("load-h4".to_owned(), vec![
            CandidateStage { value: 1, trials: vec![TrialClass::ProviderFailure; 2] },
            CandidateStage { value: 2, trials: vec![TrialClass::Success; 2] },
            CandidateStage { value: 3, trials: vec![TrialClass::Success, TrialClass::DeviceFailure] },
            CandidateStage { value: 4, trials: vec![TrialClass::Success; 2] },
        ]);
        let intersection = summarize_candidates(SummaryMode::MinimumLead, &conditions, 2).unwrap();
        assert_eq!(intersection.worst, Some(4), "every trial at a candidate must pass in every condition");
        assert_eq!(intersection.conditions["load-none"].candidates, 4);

        let asymmetric = BTreeMap::from([
            ("condition-a".to_owned(), stages(&[1, 2, 3], &[true, false, true])),
            ("condition-b".to_owned(), stages(&[1, 3], &[true, true])),
        ]);
        assert_eq!(summarize_candidates(SummaryMode::MinimumLead, &asymmetric, 2).unwrap().worst, Some(3));
        assert_eq!(summarize_candidates(SummaryMode::Capacity, &asymmetric, 2).unwrap().worst, Some(1));

        let under_trialed_asymmetric = BTreeMap::from([
            ("condition-a".to_owned(), vec![
                CandidateStage { value: 1, trials: vec![TrialClass::Success; 2] },
                CandidateStage { value: 2, trials: vec![TrialClass::Success] },
                CandidateStage { value: 3, trials: vec![TrialClass::Success; 2] },
            ]),
            ("condition-b".to_owned(), stages(&[1, 3], &[true, true])),
        ]);
        let lead = summarize_candidates(SummaryMode::MinimumLead, &under_trialed_asymmetric, 2).unwrap();
        assert_eq!(lead.worst, Some(3), "an under-trialled stage cannot be hidden by another condition omitting it");
        assert_eq!(lead.conditions["condition-a"].under_trialed, 1);
        assert_eq!(summarize_candidates(SummaryMode::Capacity, &under_trialed_asymmetric, 2).unwrap().worst, Some(1));

        let capacity = BTreeMap::from([("one".to_owned(), stages(&[1, 2, 3], &[true, false, true]))]);
        assert_eq!(summarize_candidates(SummaryMode::Capacity, &capacity, 2).unwrap().worst, Some(1));
        let none = BTreeMap::from([("one".to_owned(), stages(&[1, 2], &[false, false]))]);
        let summary = summarize_candidates(SummaryMode::MinimumLead, &none, 2).unwrap();
        assert_eq!(summary.worst, None);
        assert!(!summary.unmeasured_reason.unwrap().is_empty());
        let under_trialed = BTreeMap::from([("one".to_owned(), vec![
            CandidateStage { value: 1, trials: vec![TrialClass::Success] },
            CandidateStage { value: 2, trials: vec![TrialClass::Success; 2] },
        ])]);
        let summary = summarize_candidates(SummaryMode::MinimumLead, &under_trialed, 2).unwrap();
        assert_eq!(summary.worst, Some(2));
        assert_eq!(summary.conditions["one"].under_trialed, 1);
        let under_trialed_only = BTreeMap::from([("one".to_owned(), vec![CandidateStage { value: 1, trials: vec![TrialClass::Success] }])]);
        let summary = summarize_candidates(SummaryMode::Capacity, &under_trialed_only, 2).unwrap();
        assert_eq!(summary.worst, None);
        assert!(summary.unmeasured_reason.as_deref().is_some_and(|reason| !reason.is_empty()));
        let not_intersecting = BTreeMap::from([
            ("a".to_owned(), vec![pass()]),
            ("b".to_owned(), vec![CandidateStage { value: 2, trials: vec![TrialClass::Success; 2] }]),
        ]);
        let no_intersection = summarize_candidates(SummaryMode::MinimumLead, &not_intersecting, 2).unwrap();
        assert_eq!(no_intersection.worst, None);
        assert!(no_intersection.unmeasured_reason.as_deref().is_some_and(|reason| reason.contains("No candidate value satisfies")));
        assert!(summarize_candidates(SummaryMode::Capacity, &none, 2).unwrap().unmeasured_reason.is_some());
    }

    #[test]
    fn measurement_record_has_exact_unmeasured_keys_and_valid_conditions() {
        let dir = super::super::TempDir::new();
        let document = super::super::profile(&dir, Json::Object(Default::default()), Json::Object(Default::default()), false);
        let binding = radio_binding(&document).unwrap();
        let uhd = description(&binding, ezsdr_radio_uhd::profile::DEFAULT_BLOCK_LEN, &ezsdr_radio_uhd::profile::terms()).unwrap();
        let device = super::super::fake(ezsdr_radio_uhd::FakeConfig::default());
        let condition = Condition {
            id: "rx0".to_owned(),
            streams: vec![stream_condition(Direction::Rx, 0, vec![0], 1e6)],
            concurrent: vec![],
            reopened: false,
        };
        let observation = observation("raw_probe", "tests/hardware.rs::raw_probe", Some("rx0".to_owned()), 1, serde_json::json!({"lead_ticks": 10})).unwrap();
        let hardware_error = build_record(
            Source::Hardware,
            &binding,
            &*device,
            &rehearsal_metadata(),
            MeasurementGroup::default(),
            &uhd,
        ).unwrap_err();
        assert!(hardware_error.contains("uhd_version"), "{hardware_error}");
        let mut no_profile = binding.clone();
        no_profile.profile = None;
        assert!(build_record(
            Source::Fake,
            &no_profile,
            &*device,
            &rehearsal_metadata(),
            MeasurementGroup::default(),
            &uhd,
        ).is_err(), "profile must come from the binding");
        let mut no_fake_version = rehearsal_metadata();
        no_fake_version.uhd_version = None;
        assert!(build_record(
            Source::Fake,
            &binding,
            &*device,
            &no_fake_version,
            MeasurementGroup::default(),
            &uhd,
        ).is_err(), "FakeDevice version metadata must be explicit");
        let record = build_record(
            Source::Fake,
            &binding,
            &*device,
            &rehearsal_metadata(),
            MeasurementGroup { conditions: vec![condition], values: vec![], observations: vec![observation] },
            &uhd,
        ).unwrap();
        let keys: BTreeSet<_> = record.values.iter().map(|value| match value {
            ValueRecord::Unmeasured { key, reason } => { assert!(!reason.trim().is_empty()); key.clone() },
            ValueRecord::Measured { key, .. } => panic!("3b-1 must not measure {}", key.as_str()),
        }).collect();
        assert!(!keys.is_empty());
        assert!(keys.contains(&Key::parse("radio.timing.min_timed_command_lead_ns").unwrap()));
        assert_eq!(keys.len(), record.values.len(), "comparable values are unique");
        assert_eq!(keys, comparable_keys(&uhd));
        assert_eq!(record.device.serial, device.serial().unwrap());
        assert_eq!(record.profile, binding.profile.clone().unwrap());
        assert_eq!(record.selector, binding.selector);
        assert_eq!(record.device.uhd_version, "not_applicable");
        assert_eq!(record.observations[0].condition.as_deref(), Some("rx0"));
        assert_eq!(record.observations[0].result["lead_ticks"], 10);

        let reason = |key: &str| record.values.iter().find_map(|value| match value {
            ValueRecord::Unmeasured { key: found, reason } if found.as_str() == key => Some(reason.as_str()),
            _ => None,
        }).expect("the comparable key has an unmeasured reason");
        let queue_reason = reason("radio.timing.command_queue_depth");
        let gain_reason = reason("radio.rx.c0.gain_db");
        let antenna_reason = reason("radio.rx.c0.antenna");
        let sample_rate_reason = reason("radio.rx.s0.sample_rate_hz");
        let lead_reason = reason("radio.timing.min_timed_command_lead_ns");
        let startup_reason = reason("radio.timing.startup_latency_ns");
        let tx_start_reason = reason("radio.timing.tx_start_lead_ns");
        let overflow_reason = reason("radio.timing.overflow_restart_gap_ns");
        let tx_throughput_reason = reason("radio.perf.tx_bytes_per_s");
        assert!(queue_reason.contains("3c-2a") && queue_reason.contains("back-pressure"), "{queue_reason}");
        assert!(gain_reason.contains("3c-2a") && gain_reason.contains("coercion read-back"), "{gain_reason}");
        assert!(antenna_reason.contains("3c-2a") && antenna_reason.contains("coercion read-back"), "{antenna_reason}");
        assert!(sample_rate_reason.contains("3c-2a") && sample_rate_reason.contains("sample-rate read-back"), "{sample_rate_reason}");
        assert!(lead_reason.contains("has not been run"), "{lead_reason}");
        assert!(startup_reason.contains("3b-1"), "{startup_reason}");
        assert!(tx_start_reason.contains("3c-1") && overflow_reason.contains("3c-1") && tx_throughput_reason.contains("3c-1"));
        assert_ne!(queue_reason, lead_reason);
        assert_ne!(lead_reason, startup_reason);
        let no_pass = BTreeMap::from([("tx-1msps".to_owned(), vec![CandidateStage { value: 1, trials: vec![TrialClass::ProviderFailure] }])]);
        let summary = summarize_candidates(SummaryMode::MinimumLead, &no_pass, 1).unwrap();
        let failed_row = numeric_value("radio.timing.min_timed_command_lead_ns", &summary, &no_pass, 1, "test").unwrap();
        let failed_reason = match failed_row {
            ValueRecord::Unmeasured { reason, .. } => reason,
            ValueRecord::Measured { .. } => panic!("failed candidates cannot produce a measurement"),
        };
        assert_ne!(failed_reason, lead_reason, "not-run and measured-but-failed records need distinct reasons");

        let mut duplicate = record.clone();
        duplicate.values.push(duplicate.values[0].clone());
        let duplicate_key = match &duplicate.values[0] {
            ValueRecord::Unmeasured { key, .. } | ValueRecord::Measured { key, .. } => key.as_str(),
        };
        assert!(model::compare(&duplicate, &uhd).iter().any(|violation| {
            violation.key == duplicate_key && violation.reason.contains("more than once")
        }));

        let bytes = serde_json::to_vec(&record).unwrap();
        let round_trip: Record = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(round_trip, record);
        let mut fake_record = round_trip;
        fake_record.source = Source::Fake;
        let violations = model::compare(&fake_record, &uhd);
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert_eq!(violations[0].key, "source");

        let mut bad_condition = record;
        bad_condition.observations[0].condition = Some("missing".to_owned());
        assert!(model::compare(&bad_condition, &uhd).iter().any(|violation| violation.key == "observations"));
    }

    #[test]
    fn profile_disagreement_is_written_after_structural_validation() {
        let dir = super::super::TempDir::new();
        let document = super::super::profile(&dir, Json::Object(Default::default()), Json::Object(Default::default()), false);
        let binding = radio_binding(&document).unwrap();
        let description = description(&binding, ezsdr_radio_uhd::profile::DEFAULT_BLOCK_LEN, &ezsdr_radio_uhd::profile::terms()).unwrap();
        let device = super::super::fake(ezsdr_radio_uhd::FakeConfig::default());
        let mut record = build_record(Source::Fake, &binding, &*device, &rehearsal_metadata(), MeasurementGroup::default(), &description).unwrap();
        record.source = Source::Hardware;
        record.conditions.push(tx_only_condition("tx-1msps"));
        assert!(description.full_duplex, "the test profile advertises full duplex");
        record.conditions.push(duplex_condition("duplex-1msps"));
        let key = Key::parse("radio.timing.min_timed_command_lead_ns").unwrap();
        let profile_value = match model::profile_values(&description).get(&key).unwrap() {
            model::Worst::Number(value) => value + 1.0,
            other => panic!("expected a numeric lead profile, got {other:?}"),
        };
        let value = ValueRecord::Measured {
            key: key.clone(), worst: model::Worst::Number(profile_value), trials: 1,
            condition: "tx-1msps".to_owned(), test: "tests/hardware.rs::hw_measurements_record".to_owned(),
        };
        let row = record.values.iter_mut().find(|row| matches!(row,
            ValueRecord::Unmeasured { key: found, .. } if found == &key)).unwrap();
        *row = value;
        let duplex_key = Key::parse("radio.full_duplex").unwrap();
        let row = record.values.iter_mut().find(|row| matches!(row,
            ValueRecord::Unmeasured { key: found, .. } if found == &duplex_key)).unwrap();
        *row = ValueRecord::Measured {
            key: duplex_key.clone(), worst: model::Worst::Boolean(false), trials: 1,
            condition: "duplex-1msps".to_owned(), test: "tests/hardware.rs::hw_measurements_record".to_owned(),
        };

        let path = dir.0.join("measurement.json");
        let violations = write_record_before_profile_assert(&path, &record, &description).unwrap();
        assert!(violations.iter().any(|violation| violation.key == key.as_str() && violation.reason.starts_with("profile value ")));
        assert!(violations.iter().any(|violation| violation.key == duplex_key.as_str()
            && violation.reason == "profile claims true but the measured result is false"));
        let saved: Record = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(saved, record, "a valid but profile-disagreeing record remains available for diagnosis");
        let invalid_path = dir.0.join("invalid-measurement.json");
        let mut zero_alignment = record.clone();
        let align_key = Key::parse("radio.tx.repeat_align_samples").unwrap();
        let row = zero_alignment.values.iter_mut().find(|row| matches!(row,
            ValueRecord::Unmeasured { key: found, .. } if found == &align_key)).unwrap();
        *row = ValueRecord::Measured {
            key: align_key, worst: model::Worst::Number(0.0), trials: 1,
            condition: "tx-1msps".to_owned(), test: "tests/hardware.rs::hw_measurements_record".to_owned(),
        };
        assert!(write_record_before_profile_assert(&invalid_path, &zero_alignment, &description).is_err());
        assert!(!invalid_path.exists(), "a zero alignment is not a profile disagreement and is not written");
        let mut invalid = record;
        invalid.source = Source::Fake;
        assert!(write_record_before_profile_assert(&invalid_path, &invalid, &description).is_err());
        assert!(!invalid_path.exists(), "record-shape violations are rejected before writing");
    }

    #[test]
    fn raw_probe_results_are_observations_only() {
        clear_raw_observations();
        let names = [
            "hw_b8_raw_device_lead",
            "hw_b8_raw_restart_lead",
            "hw_b8_raw_queue_depth",
            "hw_b8_raw_in_flight_window",
            "hw_b8_raw_burst_gap",
            "hw_b8_raw_empty_eob_gap",
            "hw_b8_raw_timed_tune_phase",
            "hw_b6_delay_by_rate",
            "hw_b8_stop_end",
        ];
        for (index, name) in names.iter().enumerate() {
            push_raw_observation(name, 1, serde_json::json!({"raw_value": index + 10}));
        }
        let observations = take_raw_observations();
        assert_eq!(observations.len(), names.len());
        for (index, (row, name)) in observations.iter().zip(names).enumerate() {
            assert_eq!(row.name, name);
            assert_eq!(row.result["raw_value"], index + 10);
        }
        let dir = super::super::TempDir::new();
        let document = super::super::profile(&dir, Json::Object(Default::default()), Json::Object(Default::default()), false);
        let binding = radio_binding(&document).unwrap();
        let description = description(&binding, ezsdr_radio_uhd::profile::DEFAULT_BLOCK_LEN, &ezsdr_radio_uhd::profile::terms()).unwrap();
        let device = super::super::fake(ezsdr_radio_uhd::FakeConfig::default());
        let record = build_record(Source::Fake, &binding, &*device, &rehearsal_metadata(), MeasurementGroup { observations, ..MeasurementGroup::default() }, &description).unwrap();
        assert!(record.values.iter().all(|value| matches!(value, ValueRecord::Unmeasured { reason, .. } if !reason.trim().is_empty())));
        let violations = model::compare(&record, &description);
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert_eq!(violations[0].key, "source");
        assert!(record.observations.iter().all(|row| row.result.contains_key("raw_value")));
    }

    #[test]
    fn measurement_plan_from_env_uses_defaults_and_rejects_invalid_counts() {
        assert_eq!(MeasurementPlan::default().overflow_trials, 5);
        assert_eq!(MeasurementPlan::default().queue_trials, 5);
        let defaults = MeasurementPlan::from_env(&BTreeMap::new()).unwrap();
        assert_eq!((defaults.step_trials, defaults.overflow_trials, defaults.throughput_seconds, defaults.loopback_trials), (100, 5, 60, 20));
        assert_eq!((defaults.queue_trials, defaults.queue_ahead_ns, defaults.queue_max_calls), (5, 3_000_000_000, 64));
        assert_eq!(defaults.tx_start_leads_ns, defaults.rx_start_leads_ns);
        assert_eq!(defaults.overflow_rates_hz, [1_000_000, 10_000_000, 25_000_000, 50_000_000]);
        assert_eq!(defaults.overflow_stall_ms.len(), defaults.overflow_rates_hz.len());
        assert_eq!(defaults.tx_throughput_rates_hz, defaults.rx_rates_hz);
        let values = BTreeMap::from([
            ("EZSDR_MEASURE_STEP_TRIALS".to_owned(), "2".to_owned()),
            ("EZSDR_MEASURE_OVERFLOW_TRIALS".to_owned(), "3".to_owned()),
            ("EZSDR_MEASURE_THROUGHPUT_SECS".to_owned(), "3".to_owned()),
            ("EZSDR_MEASURE_LOOPBACK_TRIALS".to_owned(), "4".to_owned()),
            ("EZSDR_MEASURE_QUEUE_TRIALS".to_owned(), "2".to_owned()),
        ]);
        let custom = MeasurementPlan::from_env(&values).unwrap();
        assert_eq!((custom.step_trials, custom.overflow_trials, custom.throughput_seconds, custom.loopback_trials), (2, 3, 3, 4));
        assert_eq!(custom.queue_trials, 2);
        for name in ["EZSDR_MEASURE_STEP_TRIALS", "EZSDR_MEASURE_OVERFLOW_TRIALS", "EZSDR_MEASURE_THROUGHPUT_SECS", "EZSDR_MEASURE_LOOPBACK_TRIALS", "EZSDR_MEASURE_QUEUE_TRIALS"] {
            for invalid in ["0", "not-a-number"] {
                assert!(MeasurementPlan::from_env(&BTreeMap::from([(name.to_owned(), invalid.to_owned())])).is_err(), "{name}={invalid}");
            }
        }
        let mut no_overflow_trials = defaults;
        no_overflow_trials.overflow_trials = 0;
        assert!(no_overflow_trials.validate().is_err());
        let no_queue_calls = MeasurementPlan { queue_max_calls: 0, ..MeasurementPlan::default() };
        assert!(no_queue_calls.validate().is_err());
        let too_many_queue_trials = MeasurementPlan { queue_trials: (u32::MAX as usize / 3) + 1, ..MeasurementPlan::default() };
        assert!(too_many_queue_trials.validate().is_err());
        let mut missing_stall = MeasurementPlan::default();
        missing_stall.overflow_stall_ms.pop();
        assert!(missing_stall.validate().is_err());
    }

    #[test]
    fn measurement_group_merges_only_supplied_comparable_values() {
        let dir = super::super::TempDir::new();
        let document = super::super::profile(&dir, Json::Object(Default::default()), Json::Object(Default::default()), false);
        let binding = radio_binding(&document).unwrap();
        let description = description(&binding, ezsdr_radio_uhd::profile::DEFAULT_BLOCK_LEN, &ezsdr_radio_uhd::profile::terms()).unwrap();
        let key = Key::parse("radio.timing.min_timed_command_lead_ns").unwrap();
        let measured = ValueRecord::Measured {
            key: key.clone(), worst: model::Worst::Number(1_000_000.0), trials: 1,
            condition: "tx-1msps".to_owned(), test: "tests/hardware.rs::hw_measurements_record".to_owned(),
        };
        let hardware = merge_values(Source::Hardware, &description, vec![measured.clone()]).unwrap();
        assert_eq!(hardware.len(), comparable_keys(&description).len());
        assert_eq!(hardware.iter().filter(|row| matches!(row, ValueRecord::Measured { .. })).count(), 1);
        assert!(hardware.iter().any(|row| row == &measured), "the supplied hardware measurement is preserved exactly");
        let fake = merge_values(Source::Fake, &description, vec![measured]).unwrap();
        assert!(fake.iter().all(|row| matches!(row, ValueRecord::Unmeasured { .. })));
        assert!(fake.iter().any(|row| matches!(row, ValueRecord::Unmeasured { key: found, reason }
            if found == &key && reason == "FakeDevice は実機の証拠ではない")));
    }

    #[test]
    fn metadata_and_writer_fail_closed() {
        use ezsdr_kernel::module_api::ProfileRef;

        let temp = super::super::TempDir::new();
        let valid = BTreeMap::from([
            (REQUIRED_ENV[0].to_owned(), temp.0.join("metadata-record.json").to_string_lossy().into_owned()),
            (REQUIRED_ENV[1].to_owned(), "X310".to_owned()),
            (REQUIRED_ENV[2].to_owned(), "HG".to_owned()),
            (REQUIRED_ENV[3].to_owned(), "usrp-lnx03".to_owned()),
            (REQUIRED_ENV[4].to_owned(), "usrp-net.sh".to_owned()),
            (REQUIRED_ENV[5].to_owned(), "1".to_owned()),
            (REQUIRED_ENV[6].to_owned(), "none".to_owned()),
            (REQUIRED_ENV[7].to_owned(), "abc123".to_owned()),
        ]);
        for name in REQUIRED_ENV {
            let mut missing = valid.clone();
            missing.remove(name);
            assert!(metadata_from_env(&missing).is_err(), "missing {name}");
            let mut empty = valid.clone();
            empty.insert(name.to_owned(), " ".to_owned());
            assert!(metadata_from_env(&empty).is_err(), "empty {name}");
        }
        let mut invalid_load = valid.clone();
        invalid_load.insert(REQUIRED_ENV[6].to_owned(), "h3".to_owned());
        assert!(metadata_from_env(&invalid_load).is_err());
        let mut h4 = valid.clone();
        h4.insert(REQUIRED_ENV[6].to_owned(), "h4".to_owned());
        assert_eq!(metadata_from_env(&h4).unwrap().load, Load::H4);
        let mut relative = valid.clone();
        relative.insert(REQUIRED_ENV[0].to_owned(), "record.json".to_owned());
        assert!(metadata_from_env(&relative).is_err());

        let existing_output = temp.0.join("existing-record.json");
        std::fs::write(&existing_output, b"existing").unwrap();
        let mut existing = valid.clone();
        existing.insert(REQUIRED_ENV[0].to_owned(), existing_output.to_string_lossy().into_owned());
        assert!(metadata_from_env(&existing).is_err(), "an existing record is rejected before the hardware entry runs");
        let mut missing_parent = valid.clone();
        missing_parent.insert(
            REQUIRED_ENV[0].to_owned(),
            temp.0.join("missing-parent").join("record.json").to_string_lossy().into_owned(),
        );
        assert!(metadata_from_env(&missing_parent).is_err(), "a missing output parent is rejected before the hardware entry runs");

        let path = temp.0.join("record.json");
        std::fs::write(&path, b"existing").unwrap();
        let before = std::fs::read(&path).unwrap();
        let record = Record {
            schema: model::Schema::V1,
            source: Source::Fake,
            profile: ProfileRef { name: "x3xx".to_owned(), version: ezsdr_kernel::module_api::Version::new(0, 1, 0) },
            selector: BTreeMap::new(),
            device: DeviceRecord { serial: "test".to_owned(), mboard: "test".to_owned(), fpga_image: "test".to_owned(), uhd_version: "test".to_owned() },
            host: Host { name: "test".to_owned(), settings_script: "test".to_owned(), settings_version: "test".to_owned() },
            load: Load::None,
            commit: "test".to_owned(),
            conditions: vec![],
            values: vec![],
            observations: vec![],
        };
        assert!(write_record(&path, &record).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let new_path = temp.0.join("new-record.json");
        write_record(&new_path, &record).unwrap();
        let saved: Record = serde_json::from_slice(&std::fs::read(new_path).unwrap()).unwrap();
        assert_eq!(saved, record);
    }

    #[test]
    fn queue_conditions_stream_only_for_receive_frequency_and_the_value_is_the_least_depth() {
        let conditions = queue_conditions();
        let ids: Vec<_> = conditions.iter().map(|(condition, _)| condition.id.as_str()).collect();
        assert_eq!(ids, ["queue-rx-gain", "queue-tx-gain", "queue-rx-frequency"]);
        let streaming: Vec<_> = conditions.iter().map(|(_, operations)| queue_streams_rx(operations)).collect();
        assert_eq!(streaming, [false, false, true]);
        assert!(conditions.iter().all(|(_, operations)| operations.iter().all(|operation| operation.dir == Dir::Rx || operation.settings.freq.is_none())),
            "no transmit frequency operation is queued");
        // The value is the least depth over every condition measured, and names the condition that gave it.
        let measured = vec![
            ("queue-rx-gain".to_owned(), vec![QueueTrialClass::Success { depth: 9 }, QueueTrialClass::Success { depth: 12 }]),
            ("queue-tx-gain".to_owned(), vec![QueueTrialClass::Success { depth: 10 }, QueueTrialClass::Success { depth: 11 }]),
            ("queue-rx-frequency".to_owned(), vec![QueueTrialClass::Success { depth: 14 }, QueueTrialClass::Success { depth: 13 }]),
        ];
        assert_eq!(summarize_queue_depth(&measured, 2), Some((9, "queue-rx-gain".to_owned(), 2)));
        let mut slower = measured;
        slower[0].1 = vec![QueueTrialClass::Success { depth: 15 }, QueueTrialClass::Success { depth: 16 }];
        assert_eq!(summarize_queue_depth(&slower, 2), Some((10, "queue-tx-gain".to_owned(), 2)));
    }

    #[test]
    fn frequency_sweep_covers_both_ends_and_mixes_whole_and_off_mhz_points() {
        let grid = Grid::Step { lo: 10_000_000.0, hi: 8_400_000_000.0, step: 1.0 };
        let points = frequency_sweep_points(&grid, 200).unwrap();
        assert_eq!((points[0], *points.last().unwrap()), (10_000_000.0, 8_400_000_000.0));
        assert!(points.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(points.len() >= 190 && points.len() <= 200, "{}", points.len());
        let whole = points.iter().filter(|point| *point % 1e6 == 0.0).count();
        assert!(whole >= 90 && points.len() - whole >= 90, "{whole} whole-MHz of {}", points.len());
        assert!(frequency_sweep_points(&grid, 3).is_err());
        assert!(frequency_sweep_points(&Grid::Step { lo: 5.0, hi: 5.0, step: 1.0 }, 20).is_err());
    }

    #[test]
    fn frequency_values_need_every_point_within_a_twentieth_of_a_hertz() {
        let keys = ["radio.rx.c0.frequency_hz", "radio.rx.c0.frequency_step_hz"];
        let requests = [10_000_000.0, 1_500_000.5 + 2e6, 8_400_000_000.0];
        let read = |offsets: [f64; 3]| -> Vec<NumericReadback> {
            requests.iter().zip(offsets).map(|(requested, offset)| NumericReadback { requested: *requested, returned: Some(requested + offset), error: None }).collect()
        };
        let [frequency, step] = frequency_values(keys, &read([0.0, 0.04, -0.03]), &requests, "raw-device-coercion", "t").unwrap();
        assert!(matches!(&frequency, ValueRecord::Measured { worst: model::Worst::Grid(model::GridValue::Step { lo, hi, step }), trials: 3, .. }
            if (*lo, *hi, *step) == (10_000_000.0, 8_400_000_000.0, 1.0)), "{frequency:?}");
        assert!(matches!(&step, ValueRecord::Measured { worst: model::Worst::Number(value), .. } if (*value - 0.08).abs() < 1e-9), "{step:?}");
        for bad in [[0.0, 0.06, 0.0], [0.0, 0.0, -0.5]] {
            let values = frequency_values(keys, &read(bad), &requests, "raw-device-coercion", "t").unwrap();
            assert!(values.iter().all(|value| matches!(value, ValueRecord::Unmeasured { reason, .. } if reason.contains("away"))), "{values:?}");
        }
        let mut failed = read([0.0; 3]);
        failed[1] = NumericReadback { requested: requests[1], returned: None, error: Some("the apply failed".to_owned()) };
        assert!(frequency_values(keys, &failed, &requests, "raw-device-coercion", "t").unwrap().iter()
            .all(|value| matches!(value, ValueRecord::Unmeasured { reason, .. } if reason.contains("the apply failed"))));
        let short = read([0.0; 3]);
        assert!(frequency_values(keys, &short[..2], &requests, "raw-device-coercion", "t").unwrap().iter()
            .all(|value| matches!(value, ValueRecord::Unmeasured { reason, .. } if reason.contains("2 of 3"))));
    }
}
