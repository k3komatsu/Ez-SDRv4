//! Ez-SDR v4 Radio Model Vocabulary radio 1.4.0 (design/07-radio-model.md).
#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, OnceLock};

use ezsdr_kernel::binding::{
    AdmissionCheck, AdmissionCheckRegistry, CheckStage, Violation,
};
use ezsdr_kernel::event::{EventKind, Severity};
use ezsdr_kernel::module_api::{
    CompileRule, ModuleError, ModuleRegistry, UpdateClass, VerbDecl, Version, VocabularyDescriptor,
};
use ezsdr_kernel::policy::{EventKindDecl, EventKindRegistry, Reaction};
use ezsdr_kernel::spec::{
    CoercionPolicy, Ident, Key, KeyDecl, Namespace, Value, ValueKind,
};
use ezsdr_kernel::stream::LatePolicy;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub mod device;

/// The Radio Model Vocabulary id and key prefix (RM-1).
pub const VOCABULARY: &str = "radio";
/// The radio device resource kind (RM-2).
pub const DEVICE_KIND: &str = "radio.device";
/// The receive stream resource kind (RM-2).
pub const RX_STREAM_KIND: &str = "radio.rx_stream";
/// The transmit stream resource kind (RM-2).
pub const TX_STREAM_KIND: &str = "radio.tx_stream";
/// The RF safety envelope section (RM-19).
pub const RF_ENVELOPE_SECTION: &str = "radio.rf_envelope";

/// Radio Model keys declared by RM-4.
pub mod keys {
    /// Receive channel count (RM-4).
    pub const RX_CHANNELS: &str = "radio.rx.channels";
    /// Transmit channel count (RM-4).
    pub const TX_CHANNELS: &str = "radio.tx.channels";
    /// Receive sample rate in samples per second (RM-4).
    pub const RX_SAMPLE_RATE_HZ: &str = "radio.rx.sample_rate_hz";
    /// Transmit sample rate in samples per second (RM-4).
    pub const TX_SAMPLE_RATE_HZ: &str = "radio.tx.sample_rate_hz";
    /// Receive RF centre frequency in hertz (RM-4).
    pub const RX_FREQUENCY_HZ: &str = "radio.rx.frequency_hz";
    /// Transmit RF centre frequency in hertz (RM-4).
    pub const TX_FREQUENCY_HZ: &str = "radio.tx.frequency_hz";
    /// Receive gain in decibels (RM-4).
    pub const RX_GAIN_DB: &str = "radio.rx.gain_db";
    /// Transmit gain in decibels (RM-4).
    pub const TX_GAIN_DB: &str = "radio.tx.gain_db";
    /// Receive antenna port name (RM-4).
    pub const RX_ANTENNA: &str = "radio.rx.antenna";
    /// Transmit antenna port name (RM-4).
    pub const TX_ANTENNA: &str = "radio.tx.antenna";
    /// Receive frequency tuning step in hertz (RM-4).
    pub const RX_FREQUENCY_STEP_HZ: &str = "radio.rx.frequency_step_hz";
    /// Transmit frequency tuning step in hertz (RM-4).
    pub const TX_FREQUENCY_STEP_HZ: &str = "radio.tx.frequency_step_hz";
    /// Receive gain step in decibels (RM-4).
    pub const RX_GAIN_STEP_DB: &str = "radio.rx.gain_step_db";
    /// Transmit gain step in decibels (RM-4).
    pub const TX_GAIN_STEP_DB: &str = "radio.tx.gain_step_db";
    /// Whether receive channels form a coherent group (RM-4).
    pub const RX_COHERENT: &str = "radio.rx.coherent";
    /// Whether transmit and receive can run simultaneously (RM-4).
    pub const FULL_DUPLEX: &str = "radio.full_duplex";
    /// Whether timed commands execute on device time (RM-4).
    pub const HARDWARE_TIME: &str = "radio.hardware_time";
    /// Phase behaviour after an untimed retune (RM-4).
    pub const PHASE_BEHAVIOR_ON_RETUNE: &str = "radio.phase_behavior_on_retune";
    /// Maximum samples in a repeated waveform (RM-4).
    pub const TX_REPEAT_MAX_SAMPLES: &str = "radio.tx.repeat_max_samples";
    /// Required sample-count alignment for repeated waveforms (RM-4).
    pub const TX_REPEAT_ALIGN_SAMPLES: &str = "radio.tx.repeat_align_samples";
    /// Nominal samples per receive block (RM-4).
    pub const RX_BLOCK_LEN: &str = "radio.rx.block_len";
    /// Minimum timed-command lead in nanoseconds (RM-4).
    pub const MIN_TIMED_COMMAND_LEAD_NS: &str = "radio.timing.min_timed_command_lead_ns";
    /// Startup latency in nanoseconds (RM-4).
    pub const STARTUP_LATENCY_NS: &str = "radio.timing.startup_latency_ns";
    /// Receive tail after orderly stop in nanoseconds (RM-4).
    pub const STOP_TAIL_NS: &str = "radio.timing.stop_tail_ns";
    /// Device timed-command queue depth (RM-4).
    pub const COMMAND_QUEUE_DEPTH: &str = "radio.timing.command_queue_depth";
    /// Receive restart gap after overrun in nanoseconds (RM-4).
    pub const OVERFLOW_RESTART_GAP_NS: &str = "radio.timing.overflow_restart_gap_ns";
    /// Receive throughput limit in bytes per second (RM-4).
    pub const RX_BYTES_PER_S: &str = "radio.perf.rx_bytes_per_s";
    /// Transmit throughput limit in bytes per second (RM-4).
    pub const TX_BYTES_PER_S: &str = "radio.perf.tx_bytes_per_s";
    /// Wire-format bytes per complex sample (RM-4).
    pub const WIRE_BYTES_PER_SAMPLE: &str = "radio.perf.wire_bytes_per_sample";
    /// Transmit path delay, in transmit samples, from a sample's timestamp to the antenna (RM-23).
    pub const TX_PATH_DELAY_SAMPLES: &str = "radio.tx.path_delay_samples";
    /// Receive path delay, in receive samples, from the antenna to a sample's timestamp (RM-23).
    pub const RX_PATH_DELAY_SAMPLES: &str = "radio.rx.path_delay_samples";
    /// The ten configuration keys in RM-5 order (RM-5).
    pub const CONFIGURATION: [&str; 10] = [
        RX_CHANNELS,
        TX_CHANNELS,
        RX_SAMPLE_RATE_HZ,
        TX_SAMPLE_RATE_HZ,
        RX_FREQUENCY_HZ,
        TX_FREQUENCY_HZ,
        RX_GAIN_DB,
        TX_GAIN_DB,
        RX_ANTENNA,
        TX_ANTENNA,
    ];
}

/// Radio Model event kinds declared by RM-10.
pub mod kinds {
    /// Receive overrun or sequence loss (RM-10).
    pub const RX_OVERFLOW: &str = "radio.RX_OVERFLOW";
    /// Transmit stream underflow (RM-10).
    pub const TX_UNDERFLOW: &str = "radio.TX_UNDERFLOW";
    /// Transmit burst discontinuity (RM-10).
    pub const TX_DISCONTINUITY: &str = "radio.TX_DISCONTINUITY";
    /// Burst target was too close or past (RM-10).
    pub const TIME_ERROR: &str = "radio.TIME_ERROR";
    /// Timed control command arrived with too little lead (RM-10).
    pub const LATE_COMMAND: &str = "radio.LATE_COMMAND";
    /// Receive channels lost alignment (RM-10).
    pub const ALIGNMENT_ERROR: &str = "radio.ALIGNMENT_ERROR";
    /// Device lost its reference clock (RM-10).
    pub const CLOCK_LOST: &str = "radio.CLOCK_LOST";
    /// Timed-command queue is full (RM-10).
    pub const COMMAND_QUEUE_FULL: &str = "radio.COMMAND_QUEUE_FULL";
    /// Provider could not carry out an admitted Action (RM-10).
    pub const COMMAND_REJECTED: &str = "radio.COMMAND_REJECTED";
}

static RADIO_NAMESPACE: OnceLock<Namespace> = OnceLock::new();
static RF_ENVELOPE_NAMESPACE: OnceLock<Namespace> = OnceLock::new();
const CHECK_STAGES: &[CheckStage] = &[
    CheckStage::Validate,
    CheckStage::Prepare,
    CheckStage::Runtime,
];

fn radio_namespace() -> &'static Namespace {
    RADIO_NAMESPACE.get_or_init(|| Namespace::parse(VOCABULARY).expect("a valid Vocabulary namespace"))
}

fn rf_envelope_namespace() -> &'static Namespace {
    RF_ENVELOPE_NAMESPACE.get_or_init(|| {
        Namespace::parse(RF_ENVELOPE_SECTION).expect("a valid section name")
    })
}

fn key_decl(
    name: &str,
    kind: ValueKind,
    coercible: bool,
    coercion_default: CoercionPolicy,
    update_class: Option<UpdateClass>,
) -> KeyDecl {
    KeyDecl {
        key: Key::parse(name).expect("a declared Radio Model key is valid"),
        kind,
        coercible,
        coercion_default,
        update_class,
    }
}

fn radio_keys() -> Vec<KeyDecl> {
    use CoercionPolicy::{Reject, Warn};
    use UpdateClass::{Cold, HardwareTimed};
    use ValueKind::{Bool, Int, Num, Str};

    vec![
        key_decl(keys::RX_CHANNELS, Int, false, Reject, Some(Cold)),
        key_decl(keys::TX_CHANNELS, Int, false, Reject, Some(Cold)),
        key_decl(keys::RX_SAMPLE_RATE_HZ, Num, true, Reject, Some(Cold)),
        key_decl(keys::TX_SAMPLE_RATE_HZ, Num, true, Reject, Some(Cold)),
        key_decl(keys::RX_FREQUENCY_HZ, Num, true, Reject, Some(HardwareTimed)),
        key_decl(keys::TX_FREQUENCY_HZ, Num, true, Reject, Some(HardwareTimed)),
        key_decl(keys::RX_GAIN_DB, Num, true, Warn, Some(HardwareTimed)),
        key_decl(keys::TX_GAIN_DB, Num, true, Warn, Some(HardwareTimed)),
        key_decl(keys::RX_ANTENNA, Str, false, Reject, None),
        key_decl(keys::TX_ANTENNA, Str, false, Reject, None),
        key_decl(keys::RX_FREQUENCY_STEP_HZ, Num, false, Reject, None),
        key_decl(keys::TX_FREQUENCY_STEP_HZ, Num, false, Reject, None),
        key_decl(keys::RX_GAIN_STEP_DB, Num, false, Reject, None),
        key_decl(keys::TX_GAIN_STEP_DB, Num, false, Reject, None),
        key_decl(keys::RX_COHERENT, Bool, false, Reject, None),
        key_decl(keys::FULL_DUPLEX, Bool, false, Reject, None),
        key_decl(keys::HARDWARE_TIME, Bool, false, Reject, None),
        key_decl(keys::PHASE_BEHAVIOR_ON_RETUNE, Str, false, Reject, None),
        key_decl(keys::TX_REPEAT_MAX_SAMPLES, Int, false, Reject, None),
        key_decl(keys::TX_REPEAT_ALIGN_SAMPLES, Int, false, Reject, None),
        key_decl(keys::RX_BLOCK_LEN, Int, false, Reject, None),
        key_decl(keys::MIN_TIMED_COMMAND_LEAD_NS, Int, false, Reject, None),
        key_decl(keys::STARTUP_LATENCY_NS, Int, false, Reject, None),
        key_decl(keys::STOP_TAIL_NS, Int, false, Reject, None),
        key_decl(keys::COMMAND_QUEUE_DEPTH, Int, false, Reject, None),
        key_decl(keys::OVERFLOW_RESTART_GAP_NS, Int, false, Reject, None),
        key_decl(keys::RX_BYTES_PER_S, Int, false, Reject, None),
        key_decl(keys::TX_BYTES_PER_S, Int, false, Reject, None),
        key_decl(keys::WIRE_BYTES_PER_SAMPLE, Int, false, Reject, None),
        key_decl(keys::TX_PATH_DELAY_SAMPLES, Int, false, Reject, None),
        key_decl(keys::RX_PATH_DELAY_SAMPLES, Int, false, Reject, None),
    ]
}

fn radio_event_kinds() -> Vec<EventKindDecl> {
    use kinds::*;

    [
        (RX_OVERFLOW, Severity::Warning, Reaction::MarkArtifact),
        (TX_UNDERFLOW, Severity::Warning, Reaction::MarkArtifact),
        (TX_DISCONTINUITY, Severity::Warning, Reaction::MarkArtifact),
        (TIME_ERROR, Severity::Error, Reaction::MarkArtifact),
        (LATE_COMMAND, Severity::Warning, Reaction::MarkArtifact),
        (ALIGNMENT_ERROR, Severity::Error, Reaction::MarkArtifact),
        (CLOCK_LOST, Severity::Fatal, Reaction::Abort),
        (COMMAND_QUEUE_FULL, Severity::Error, Reaction::Abort),
        (COMMAND_REJECTED, Severity::Error, Reaction::MarkArtifact),
    ]
    .into_iter()
    .map(|(name, severity, default)| EventKindDecl {
        kind: EventKind::parse(name).expect("a declared radio event kind is valid"),
        severity,
        default,
    })
    .collect()
}

/// Describes the Radio Model Vocabulary `radio` 1.4.0 (RM-1).
pub fn vocabulary() -> VocabularyDescriptor {
    VocabularyDescriptor {
        id: radio_namespace().clone(),
        version: Version::new(1, 4, 0),
        prefix: radio_namespace().clone(),
        keys: radio_keys(),
        event_kinds: radio_event_kinds(),
        verbs: vec![
            VerbDecl {
                verb: Ident::parse("start_repeat").expect("a valid Session verb"),
                compiles_to: CompileRule::TxBurst {
                    repeat: true,
                    late_policy: LatePolicy::SendAsapAndFlag,
                },
            },
            VerbDecl {
                verb: Ident::parse("send").expect("a valid Session verb"),
                compiles_to: CompileRule::TxBurst {
                    repeat: false,
                    late_policy: LatePolicy::DropAndFlag,
                },
            },
        ],
        checks: vec![rf_envelope_namespace().clone()],
    }
}

/// Registers the descriptor, RF-envelope check and event kinds in RM-1 order (RM-1).
pub fn register(
    registry: &mut ModuleRegistry,
    checks: &mut AdmissionCheckRegistry,
    kinds: &mut EventKindRegistry,
) -> Result<(), ModuleError> {
    registry.register_vocabulary(vocabulary())?;
    checks.register(Arc::new(RfEnvelopeCheck));
    for kind in radio_event_kinds() {
        kinds
            .register(Some(radio_namespace().clone()), kind)
            .map_err(|error| ModuleError::rejected(format!("RM-1: {error}")))?;
    }
    Ok(())
}

/// RF transmit constraints supplied by a BindingProfile (RM-19).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RfEnvelope {
    /// Inclusive RF bands available at the site (RM-19).
    pub allowed_bands: Vec<Band>,
    /// Optional maximum transmit gain in decibels (RM-19).
    #[serde(default)]
    pub max_gain_db: Option<f64>,
    /// Whether all channels or each channel may transmit (RM-19).
    #[serde(default)]
    pub tx_enabled: Option<TxEnabled>,
    /// Optional allowed antenna names for receive and transmit (RM-19).
    #[serde(default)]
    pub antenna_ports: Option<Vec<String>>,
}

/// One inclusive RF band in hertz (RM-19).
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Band {
    /// Lowest allowed frequency (RM-19).
    pub lo_hz: f64,
    /// Highest allowed frequency (RM-19).
    pub hi_hz: f64,
}

/// Global or per-channel transmit enable state (RM-19).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum TxEnabled {
    /// One setting for every transmit channel (RM-19).
    All(bool),
    /// One setting per channel (RM-19).
    PerChannel(Vec<bool>),
}

/// Enforces the BindingProfile's RF safety envelope (RM-19).
pub struct RfEnvelopeCheck;

impl AdmissionCheck for RfEnvelopeCheck {
    fn section(&self) -> &Namespace {
        rf_envelope_namespace()
    }

    fn stages(&self) -> &[CheckStage] {
        CHECK_STAGES
    }

    fn check(
        &self,
        section: &serde_json::Value,
        effective: &BTreeMap<Ident, BTreeMap<Key, Value>>,
        proposed: &BTreeMap<Ident, BTreeMap<Key, Value>>,
        _stage: CheckStage,
    ) -> Vec<Violation> {
        let envelope: RfEnvelope = match serde_json::from_value(section.clone()) {
            Ok(envelope) => envelope,
            Err(error) => {
                return vec![Violation {
                    check: self.section().clone(),
                    key: None,
                    requested: None,
                    reason: format!("RM-19: the section does not parse: {error}"),
                }];
            }
        };
        if let Some(band) = envelope.allowed_bands.iter().find(|band| band.lo_hz > band.hi_hz) {
            return vec![Violation {
                check: self.section().clone(),
                key: None,
                requested: None,
                reason: format!("RM-19: band {}..{} has lo_hz > hi_hz", band.lo_hz, band.hi_hz),
            }];
        }

        let fragments: BTreeSet<Ident> = effective
            .keys()
            .chain(proposed.keys())
            .cloned()
            .collect();
        let mut violations = Vec::new();
        for fragment in fragments {
            let mut configuration = effective.get(&fragment).cloned().unwrap_or_default();
            if let Some(updates) = proposed.get(&fragment) {
                configuration.extend(updates.clone());
            }
            let tx_channels = number(&configuration, keys::TX_CHANNELS, &fragment, self, &mut violations)
                .unwrap_or(0.0) as i64;
            if tx_channels > 0 {
                if let Some(frequency) = number(
                    &configuration,
                    keys::TX_FREQUENCY_HZ,
                    &fragment,
                    self,
                    &mut violations,
                ) {
                    if !envelope
                        .allowed_bands
                        .iter()
                        .any(|band| band.lo_hz <= frequency && frequency <= band.hi_hz)
                    {
                        push_fragment_violation(
                            &mut violations,
                            self,
                            &fragment,
                            keys::TX_FREQUENCY_HZ,
                            &configuration,
                            format!("{frequency} Hz is in no allowed band"),
                        );
                    }
                }
                if let Some(gain) = number(
                    &configuration,
                    keys::TX_GAIN_DB,
                    &fragment,
                    self,
                    &mut violations,
                ) {
                    if envelope.max_gain_db.is_some_and(|maximum| gain > maximum) {
                        let maximum = envelope.max_gain_db.unwrap_or_default();
                        push_fragment_violation(
                            &mut violations,
                            self,
                            &fragment,
                            keys::TX_GAIN_DB,
                            &configuration,
                            format!("gain {gain} dB exceeds {maximum}"),
                        );
                    }
                }
                let disabled_channel = match &envelope.tx_enabled {
                    Some(TxEnabled::All(false)) => Some(None),
                    Some(TxEnabled::PerChannel(enabled)) => {
                        let count = tx_channels as usize;
                        enabled
                            .iter()
                            .take(count)
                            .position(|is_enabled| !is_enabled)
                            .or_else(|| (enabled.len() < count).then_some(enabled.len()))
                            .map(Some)
                    }
                    _ => None,
                };
                if let Some(channel) = disabled_channel {
                    push_fragment_violation(
                        &mut violations,
                        self,
                        &fragment,
                        keys::TX_CHANNELS,
                        &configuration,
                        channel.map_or_else(
                            || "transmission is disabled".to_owned(),
                            |index| format!("channel {index} is not enabled"),
                        ),
                    );
                }
                if let Some(antenna) = string(
                    &configuration,
                    keys::TX_ANTENNA,
                    &fragment,
                    self,
                    &mut violations,
                ) {
                    if envelope
                        .antenna_ports
                        .as_ref()
                        .is_some_and(|ports| !ports.iter().any(|port| port == antenna))
                    {
                        push_fragment_violation(
                            &mut violations,
                            self,
                            &fragment,
                            keys::TX_ANTENNA,
                            &configuration,
                            format!("antenna {antenna} is not allowed"),
                        );
                    }
                }
            }
            if let Some(antenna) = string(
                &configuration,
                keys::RX_ANTENNA,
                &fragment,
                self,
                &mut violations,
            ) {
                if envelope
                    .antenna_ports
                    .as_ref()
                    .is_some_and(|ports| !ports.iter().any(|port| port == antenna))
                {
                    push_fragment_violation(
                        &mut violations,
                        self,
                        &fragment,
                        keys::RX_ANTENNA,
                        &configuration,
                        format!("antenna {antenna} is not allowed"),
                    );
                }
            }
        }
        violations
    }
}

fn number(
    configuration: &BTreeMap<Key, Value>,
    name: &str,
    fragment: &Ident,
    check: &RfEnvelopeCheck,
    violations: &mut Vec<Violation>,
) -> Option<f64> {
    let key = Key::parse(name).expect("a declared Radio Model key is valid");
    match configuration.get(&key) {
        None => None,
        Some(Value::Int(value)) => Some(*value as f64),
        Some(Value::Num(value)) => Some(*value),
        Some(_) => {
            push_fragment_violation(
                violations,
                check,
                fragment,
                name,
                configuration,
                format!("{name} is not a number"),
            );
            None
        }
    }
}

fn string<'a>(
    configuration: &'a BTreeMap<Key, Value>,
    name: &str,
    fragment: &Ident,
    check: &RfEnvelopeCheck,
    violations: &mut Vec<Violation>,
) -> Option<&'a str> {
    let key = Key::parse(name).expect("a declared Radio Model key is valid");
    match configuration.get(&key) {
        None => None,
        Some(Value::Str(value)) => Some(value),
        Some(_) => {
            push_fragment_violation(
                violations,
                check,
                fragment,
                name,
                configuration,
                format!("{name} is not a string"),
            );
            None
        }
    }
}

fn push_fragment_violation(
    violations: &mut Vec<Violation>,
    check: &RfEnvelopeCheck,
    fragment: &Ident,
    key_name: &str,
    configuration: &BTreeMap<Key, Value>,
    detail: String,
) {
    let key = Key::parse(key_name).expect("a declared Radio Model key is valid");
    violations.push(Violation {
        check: check.section().clone(),
        requested: configuration.get(&key).cloned(),
        key: Some(key),
        reason: format!("RM-19: {fragment}: {detail}"),
    });
}

/// Timing values recorded in a radio Provider's Manifest (RM-20).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TimingEnvelope {
    /// Minimum lead for a timed command (RM-20).
    pub min_timed_command_lead_ns: i64,
    /// Time from arm to stream start (RM-20).
    pub startup_latency_ns: i64,
    /// Receive tail after orderly stop (RM-20).
    pub stop_tail_ns: i64,
    /// Timed-command queue capacity (RM-20).
    pub command_queue_depth: i64,
    /// Receive restart gap after overrun (RM-20).
    pub overflow_restart_gap_ns: i64,
}

/// Throughput values recorded in a radio Provider's Manifest (RM-20).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PerformanceEnvelope {
    /// Receive throughput limit in bytes per second (RM-20).
    pub rx_bytes_per_s: i64,
    /// Transmit throughput limit in bytes per second (RM-20).
    pub tx_bytes_per_s: i64,
    /// Wire-format bytes per complex sample (RM-20).
    pub wire_bytes_per_sample: i64,
}

/// The profile and envelopes a radio Provider records in its Manifest (RM-20).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RadioEnvelope {
    /// The exact Provider profile applied (RM-20).
    pub profile: ezsdr_kernel::module_api::ProfileRef,
    /// The timing limits declared by the Provider (RM-20).
    pub timing: TimingEnvelope,
    /// The throughput limits declared by the Provider (RM-20).
    pub performance: PerformanceEnvelope,
}

/// Payload documents for Radio Model events (RM-22).
pub mod payloads {
    use ezsdr_kernel::spec::Key;
    use ezsdr_kernel::time::TimePoint;
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    /// Whether sample loss came from an overrun or a sequence error (RM-22).
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum RxOverflowCause {
        /// A receive overrun (RM-22).
        Overrun,
        /// A receive sequence error (RM-22).
        Sequence,
    }

    /// Receive-loss event data (RM-22).
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct RxOverflowPayload {
        /// The kind of receive loss (RM-22).
        pub cause: RxOverflowCause,
        /// Number of lost samples (RM-22).
        pub lost: u64,
        /// Restart gap in nanoseconds, or zero for a sequence error (RM-22).
        pub restart_gap_ns: i64,
    }

    /// The length of [`RxOverflowPayload`]'s hot-path form (RM-24).
    pub const RX_OVERFLOW_HOT_BYTES: usize = 17;

    /// `RX_OVERFLOW`'s payload as a Manifest holds it when a Provider emitted it on the
    /// hot path: the Kernel's drain turns the record's bytes into this array (RS-34).
    /// Its schema gives a Manifest reader the shape; RM-24 gives the layout, which only
    /// [`RxOverflowPayload::from_payload`] interprets.
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
    #[serde(transparent)]
    pub struct RxOverflowHotPayload(pub [u8; RX_OVERFLOW_HOT_BYTES]);

    impl RxOverflowPayload {
        /// The hot-path form: the cause byte (0 overrun, 1 sequence), then `lost` as a
        /// little-endian u64 and `restart_gap_ns` as a little-endian i64 (RM-24). The
        /// Vocabulary owns this layout; the Kernel only queues the bytes (RS-32a,
        /// withdrawn).
        pub fn to_hot(&self) -> [u8; RX_OVERFLOW_HOT_BYTES] {
            let mut out = [0; RX_OVERFLOW_HOT_BYTES];
            out[0] = match self.cause {
                RxOverflowCause::Overrun => 0,
                RxOverflowCause::Sequence => 1,
            };
            out[1..9].copy_from_slice(&self.lost.to_le_bytes());
            out[9..17].copy_from_slice(&self.restart_gap_ns.to_le_bytes());
            out
        }

        /// Reads the hot-path form, refusing another length or an unknown cause (RM-24).
        pub fn from_hot(bytes: &[u8]) -> Result<RxOverflowPayload, String> {
            let bytes: &[u8; RX_OVERFLOW_HOT_BYTES] = bytes.try_into().map_err(|_| {
                format!("RM-24: the hot-path form is {RX_OVERFLOW_HOT_BYTES} bytes, not {}", bytes.len())
            })?;
            let cause = match bytes[0] {
                0 => RxOverflowCause::Overrun,
                1 => RxOverflowCause::Sequence,
                other => return Err(format!("RM-24: {other} is not a cause")),
            };
            let lost = u64::from_le_bytes(bytes[1..9].try_into().expect("eight bytes"));
            let restart_gap_ns = i64::from_le_bytes(bytes[9..17].try_into().expect("eight bytes"));
            Ok(RxOverflowPayload { cause, lost, restart_gap_ns })
        }

        /// Reads a delivered event's payload in either form: RM-11's object from the
        /// control path, or the array of byte values the Kernel's drain makes of a
        /// hot-path record (RM-24, RS-34).
        pub fn from_payload(payload: &serde_json::Value) -> Result<RxOverflowPayload, String> {
            match payload {
                serde_json::Value::Array(values) => {
                    let bytes = values
                        .iter()
                        .map(|value| value.as_u64().and_then(|byte| u8::try_from(byte).ok()))
                        .collect::<Option<Vec<u8>>>()
                        .ok_or_else(|| "RM-24: the hot-path form holds byte values only".to_owned())?;
                    RxOverflowPayload::from_hot(&bytes)
                }
                serde_json::Value::Object(_) => serde_json::from_value(payload.clone())
                    .map_err(|error| format!("RM-24: {error}")),
                _ => Err("RM-24: a payload is an object or an array of bytes".to_owned()),
            }
        }
    }

    /// Why a radio burst reported a time error (RM-22).
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum TimeErrorCause {
        /// The target did not meet the required lead (RM-22).
        Late,
        /// A new timed block followed an unclosed burst (RM-22).
        UnclosedBurst,
    }

    /// What the Provider did about a late burst (RM-22).
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum TimeErrorOutcome {
        /// Transmit as soon as possible (RM-22).
        SendAsap,
        /// Drop the burst (RM-22).
        Drop,
        /// Reject a plan-time violation at runtime (RM-22).
        PlanViolation,
        /// Refuse the burst (RM-22).
        Refused,
        /// The Provider handed the burst over in time by its own clock and the device
        /// reported it late, so nothing was transmitted (RM-11, VE-3).
        LateAtDevice,
    }

    /// Why a transmit stream underflowed (RM-11, VE-3).
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum TxUnderflowCause {
        /// The device ran out of samples inside a burst because the host was late.
        Starved,
        /// Samples of a burst were lost between host and device.
        Lost,
    }

    /// `radio.TX_UNDERFLOW` data (RM-11, VE-3).
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct TxUnderflowPayload {
        /// Why (RM-11).
        pub cause: TxUnderflowCause,
    }

    /// `radio.ALIGNMENT_ERROR` data (RM-11, VE-3).
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct AlignmentErrorPayload {
        /// The samples, on every channel of the stream, the misalignment removed (RM-11).
        pub lost: u64,
    }

    /// Which reference a device lost (RM-11, VE-3).
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum ClockReference {
        /// The frequency reference (10 MHz).
        Frequency,
    }

    /// `radio.CLOCK_LOST` data (RM-11, VE-3).
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct ClockLostPayload {
        /// The reference lost (RM-11).
        pub reference: ClockReference,
    }

    /// Timed burst error data (RM-22).
    #[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct TimeErrorPayload {
        /// Why the time error occurred (RM-22).
        pub cause: TimeErrorCause,
        /// How the Provider handled it (RM-22).
        pub outcome: TimeErrorOutcome,
        /// How late the target was, in nanoseconds (RM-22).
        pub late_by_ns: i64,
        /// The requested burst target (RM-22).
        pub target: TimePoint,
    }

    /// Timed control-command lateness data (RM-22).
    #[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct LateCommandPayload {
        /// The changed key, or `None` for a stream start (RM-22).
        pub key: Option<Key>,
        /// The requested instant (RM-22).
        pub requested: TimePoint,
        /// The actual instant applied (RM-22).
        pub applied: TimePoint,
    }

    /// Timed-command queue exhaustion data (RM-22).
    #[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct CommandQueueFullPayload {
        /// The key of the timed command (RM-22).
        pub key: Key,
        /// Queue depth at refusal (RM-22).
        pub depth: i64,
    }

    /// Data for an admitted Action the Provider could not perform (RM-22).
    #[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct CommandRejectedPayload {
        /// The Action's kind tag (RM-22).
        pub action: String,
        /// Why the Provider refused the Action (RM-22).
        pub reason: String,
    }
}

/// Generates the eleven committed Radio Model schemas (RM-20, RM-22, RM-24).
pub fn document_schemas() -> BTreeMap<&'static str, serde_json::Value> {
    use payloads::{
        AlignmentErrorPayload, ClockLostPayload, CommandQueueFullPayload,
        CommandRejectedPayload, LateCommandPayload, RxOverflowHotPayload, RxOverflowPayload,
        TimeErrorPayload, TxUnderflowPayload,
    };

    fn of<T: JsonSchema>() -> serde_json::Value {
        serde_json::to_value(ezsdr_kernel::schema::generator().into_root_schema_for::<T>())
            .expect("a generated schema is JSON")
    }
    BTreeMap::from([
        ("rf_envelope", of::<RfEnvelope>()),
        ("envelope", of::<RadioEnvelope>()),
        ("rx_overflow_payload", of::<RxOverflowPayload>()),
        ("rx_overflow_hot_payload", of::<RxOverflowHotPayload>()),
        ("time_error_payload", of::<TimeErrorPayload>()),
        ("late_command_payload", of::<LateCommandPayload>()),
        ("command_queue_full_payload", of::<CommandQueueFullPayload>()),
        ("command_rejected_payload", of::<CommandRejectedPayload>()),
        ("tx_underflow_payload", of::<TxUnderflowPayload>()),
        ("alignment_error_payload", of::<AlignmentErrorPayload>()),
        ("clock_lost_payload", of::<ClockLostPayload>()),
    ])
}
