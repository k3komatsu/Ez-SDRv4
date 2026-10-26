//! Measurement-record construction and pure trial classification for the UHD bench.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use ezsdr_kernel::binding::{Binding, BindingProfile};
use ezsdr_kernel::spec::{Ident, Key};
use ezsdr_kernel::stream::Direction;
use ezsdr_radio::channels;
use ezsdr_radio::measurement::{
    self as model, Condition, Device as DeviceRecord, Host, Load, Observation, Record, Rule,
    Source, StreamCondition, StreamRef, ValueRecord,
};
use ezsdr_radio::x3xx::ProviderTerms;
use ezsdr_radio_uhd::Device;
use serde_json::Value as Json;

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

/// Builds a record whose comparable keys are all explicitly unmeasured.
pub fn build_record(
    source: Source,
    binding: &Binding,
    device: &dyn Device,
    metadata: &Metadata,
    conditions: Vec<Condition>,
    observations: Vec<Observation>,
    description: &channels::Description,
) -> Result<Record, String> {
    let device_description = device.describe();
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
            serial: device.serial().map_err(|error| error.message)?,
            mboard: metadata.mboard.clone(),
            fpga_image: metadata.fpga_image.clone(),
            uhd_version,
        },
        host: metadata.host.clone(),
        load: metadata.load,
        commit: metadata.commit.clone(),
        conditions,
        values: unmeasured_values(description),
        observations,
    })
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
    match key {
        "radio.timing.command_queue_depth" => "Applied instants are reservations, not device execution observations; raw queue probes remain observations, and the comparison method is deferred to Phase 8 stage 3c.",
        "radio.timing.min_timed_command_lead_ns"
        | "radio.timing.tx_in_flight_ns"
        | "radio.timing.rx_start_lead_ns"
        | "radio.perf.rx_bytes_per_s"
        | "radio.full_duplex"
        | "radio.hardware_time" => "The comparative Provider measurement group is deferred to Phase 8 stage 3b-2.",
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
    Underflow,
    UnderflowInPacket,
    SequenceError,
    SequenceErrorInBurst,
    BurstAck,
    ShortSendRejected,
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
    pub record: Option<TrialRecord>,
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
            record: None,
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
            Some(tick) => trials.iter().rposition(|trial| trial.start_tick <= tick),
            None => trials.len().checked_sub(1),
        });
    if let Some(index) = target { trials[index].unattributable = true; }
}

/// Classifies a trial only after its record, samples, end, and device acknowledgement agree.
pub fn classify_trial(trial: &TrialEvidence) -> TrialClass {
    if !trial.admitted { return TrialClass::AdmissionRejection; }
    if trial.unattributable { return TrialClass::Unattributable; }
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
        || record.end_tick != trial.expected_end_tick
        || missing_completion
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
        common = Some(match common { Some(common) => common.intersection(&eligible).copied().collect(), None => eligible });
        sorted.insert(condition.clone(), stages);
    }
    let common = common.unwrap_or_default();
    if common.is_empty() {
        return Ok(CandidateSummary {
            worst: None,
            unmeasured_reason: Some("No candidate value satisfies every condition's successful edge rule with the required complete trials.".to_owned()),
            conditions: counts(&sorted, required_trials),
        });
    }
    let values: Vec<u64> = common.into_iter().collect();
    let worst = match mode {
        SummaryMode::MinimumLead => values.first().copied(),
        SummaryMode::Capacity => values.last().copied(),
    };
    Ok(CandidateSummary { worst, unmeasured_reason: None, conditions: counts(&sorted, required_trials) })
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(!before_first[0].unattributable, "an event before every trial has no preceding trial to fail");

        let mut matched_async = vec![complete(0)];
        attribute_evidence(&mut matched_async, TimedEvidence { tick: Some(5), span: None, kind: EvidenceKind::AsyncTimeError });
        assert_eq!(classify_trial(&matched_async[0]), TrialClass::DeviceFailure, "a report inside the trial is device evidence");
        let mut unmatched_async = vec![complete(0)];
        attribute_evidence(&mut unmatched_async, TimedEvidence { tick: Some(99), span: None, kind: EvidenceKind::AsyncTimeError });
        assert_eq!(classify_trial(&unmatched_async[0]), TrialClass::Unattributable, "a report outside every trial cannot be credited to it");
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
            vec![],
            vec![],
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
            vec![],
            vec![],
            &uhd,
        ).is_err(), "profile must come from the binding");
        let mut no_fake_version = rehearsal_metadata();
        no_fake_version.uhd_version = None;
        assert!(build_record(
            Source::Fake,
            &binding,
            &*device,
            &no_fake_version,
            vec![],
            vec![],
            &uhd,
        ).is_err(), "FakeDevice version metadata must be explicit");
        let record = build_record(
            Source::Fake,
            &binding,
            &*device,
            &rehearsal_metadata(),
            vec![condition],
            vec![observation],
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
        let lead_reason = reason("radio.timing.min_timed_command_lead_ns");
        let startup_reason = reason("radio.timing.startup_latency_ns");
        assert!(queue_reason.contains("reservations"), "{queue_reason}");
        assert!(lead_reason.contains("3b-2"), "{lead_reason}");
        assert!(startup_reason.contains("3b-1"), "{startup_reason}");
        assert_ne!(queue_reason, lead_reason);
        assert_ne!(lead_reason, startup_reason);

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
        let record = build_record(Source::Fake, &binding, &*device, &rehearsal_metadata(), vec![], observations, &description).unwrap();
        assert!(record.values.iter().all(|value| matches!(value, ValueRecord::Unmeasured { reason, .. } if !reason.trim().is_empty())));
        let violations = model::compare(&record, &description);
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert_eq!(violations[0].key, "source");
        assert!(record.observations.iter().all(|row| row.result.contains_key("raw_value")));
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
}
