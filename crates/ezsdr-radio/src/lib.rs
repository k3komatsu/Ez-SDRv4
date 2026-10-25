//! Ez-SDR v4 Radio Model Vocabulary radio 2.0.0 (design/07-radio-model.md).
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
    CoercionPolicy, Ident, Key, KeyDecl, Namespace, Scalar, Value, ValueKind,
};
use ezsdr_kernel::stream::{Direction, LatePolicy};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub mod channels;
pub mod device;
pub mod timeline;
pub mod x3xx;

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
/// The verb that starts a receive stream again after a `Stop`: a `Command` on
/// `<device>/rx` with no params (RM-12, RM-21).
pub const START_RX: &str = "start_rx";

/// Radio Model keys declared by RM-4.
pub mod keys {
    /// Whether transmit and receive can run simultaneously (RM-4).
    pub const FULL_DUPLEX: &str = "radio.full_duplex";
    /// Whether timed commands execute on device time (RM-4).
    pub const HARDWARE_TIME: &str = "radio.hardware_time";
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
    /// Device timed-command queue depth (RM-4).
    pub const COMMAND_QUEUE_DEPTH: &str = "radio.timing.command_queue_depth";
    /// How far ahead of its instant a timed command is handed to the device (RM-4, RM-21).
    pub const RELEASE_WINDOW_NS: &str = "radio.timing.release_window_ns";
    /// Receive restart gap after overrun in nanoseconds (RM-4).
    pub const OVERFLOW_RESTART_GAP_NS: &str = "radio.timing.overflow_restart_gap_ns";
    /// Lead from a receive stream's previous cut, and from the instant its Provider is ready
    /// for it, to a new segment's first sample, in nanoseconds (RM-4, RM-25).
    pub const RX_START_LEAD_NS: &str = "radio.timing.rx_start_lead_ns";
    /// The same for a transmit stream (RM-4, RM-25).
    pub const TX_START_LEAD_NS: &str = "radio.timing.tx_start_lead_ns";
    /// Maximum time ahead of the device that transmit samples may be handed over (RM-4, RM-15).
    pub const TX_IN_FLIGHT_NS: &str = "radio.timing.tx_in_flight_ns";
    /// Receive throughput limit in bytes per second (RM-4).
    pub const RX_BYTES_PER_S: &str = "radio.perf.rx_bytes_per_s";
    /// Transmit throughput limit in bytes per second (RM-4).
    pub const TX_BYTES_PER_S: &str = "radio.perf.tx_bytes_per_s";
    /// Wire-format bytes per complex sample (RM-4).
    pub const WIRE_BYTES_PER_SAMPLE: &str = "radio.perf.wire_bytes_per_sample";
    /// The segment a channel key numbers: `c` followed by the channel number (RM-27).
    pub const CHANNEL: &str = "c";
    /// The segment a stream key numbers: `s` followed by the stream number (RM-27).
    pub const STREAM: &str = "s";
    /// The leaves of the channel keys (RM-27).
    pub const CHANNEL_LEAVES: [&str; 8] = [
        "frequency_hz",
        "gain_db",
        "antenna",
        "stream",
        "frequency_step_hz",
        "gain_step_db",
        "phase_behavior_on_retune",
        "path_delay_samples",
    ];
    /// The leaf of the stream keys (RM-27).
    pub const STREAM_LEAF: &str = "sample_rate_hz";

    /// The concrete channel key `radio.<direction>.c<n>.<leaf>` (RM-27).
    pub fn channel(direction: &str, n: u32, leaf: &str) -> String {
        format!("radio.{direction}.{CHANNEL}{n}.{leaf}")
    }

    /// The concrete stream key `radio.<direction>.s<k>.sample_rate_hz` (RM-27).
    pub fn stream(direction: &str, k: u32) -> String {
        format!("radio.{direction}.{STREAM}{k}.{STREAM_LEAF}")
    }
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

/// Why a Provider cancelled an accepted held transmit burst (RM-16).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelReason {
    /// A stream or device `Stop` cancelled the burst.
    Stop,
    /// `Provider::stop` cancelled the burst.
    ProviderStop,
    /// Device loss cancelled the burst.
    Loss,
    /// A `cold` change cancelled the burst on its old transmit clock.
    ColdChange,
}

/// Whether cancelling a held burst for `reason` emits `radio.COMMAND_REJECTED` (RM-16).
pub const fn cancellation_reports(reason: CancelReason) -> bool {
    matches!(reason, CancelReason::ColdChange)
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
        indexed: None,
        kind,
        coercible,
        coercion_default,
        update_class,
    }
}

/// A declaration whose segment at `NUMBERED_SEGMENT` is followed by a number (RM-27,
/// SB-2a).
fn indexed_key_decl(
    name: &str,
    kind: ValueKind,
    coercible: bool,
    coercion_default: CoercionPolicy,
    update_class: Option<UpdateClass>,
) -> KeyDecl {
    KeyDecl {
        key: Key::parse(name).expect("a declared Radio Model key is valid"),
        indexed: Some(NUMBERED_SEGMENT),
        kind,
        coercible,
        coercion_default,
        update_class,
    }
}

/// The position of the numbered segment in every RM-27 channel and stream key.
const NUMBERED_SEGMENT: u8 = 2;

/// RM-4's and RM-27's key declarations together, in RM-1's order: the one table the
/// Vocabulary advertises and, through [`channels::update_class`], a Provider's reading of a
/// layout configuration key's class comes from.
pub(crate) fn radio_keys() -> Vec<KeyDecl> {
    use CoercionPolicy::{Reject, Warn};
    use UpdateClass::{Cold, HardwareTimed};
    use ValueKind::{Bool, Int, Num, Str};

    let mut declarations = vec![
        key_decl(keys::FULL_DUPLEX, Bool, false, Reject, None),
        key_decl(keys::HARDWARE_TIME, Bool, false, Reject, None),
        key_decl(keys::TX_REPEAT_MAX_SAMPLES, Int, false, Reject, None),
        key_decl(keys::TX_REPEAT_ALIGN_SAMPLES, Int, false, Reject, None),
        key_decl(keys::RX_BLOCK_LEN, Int, false, Reject, None),
        key_decl(keys::MIN_TIMED_COMMAND_LEAD_NS, Int, false, Reject, None),
        key_decl(keys::STARTUP_LATENCY_NS, Int, false, Reject, None),
        key_decl(keys::COMMAND_QUEUE_DEPTH, Int, false, Reject, None),
        key_decl(keys::RELEASE_WINDOW_NS, Int, false, Reject, None),
        key_decl(keys::OVERFLOW_RESTART_GAP_NS, Int, false, Reject, None),
        key_decl(keys::RX_START_LEAD_NS, Int, false, Reject, None),
        key_decl(keys::TX_START_LEAD_NS, Int, false, Reject, None),
        key_decl(keys::TX_IN_FLIGHT_NS, Int, false, Reject, None),
        key_decl(keys::RX_BYTES_PER_S, Int, false, Reject, None),
        key_decl(keys::TX_BYTES_PER_S, Int, false, Reject, None),
        key_decl(keys::WIRE_BYTES_PER_SAMPLE, Int, false, Reject, None),
    ];
    for direction in ["rx", "tx"] {
        let declaration = |word: &str, leaf: &str| format!("radio.{direction}.{word}.{leaf}");
        declarations.push(indexed_key_decl(&declaration(keys::CHANNEL, "frequency_hz"), Num, true, Reject, Some(HardwareTimed)));
        declarations.push(indexed_key_decl(&declaration(keys::CHANNEL, "gain_db"), Num, true, Warn, Some(HardwareTimed)));
        declarations.push(indexed_key_decl(&declaration(keys::CHANNEL, "antenna"), Str, false, Reject, Some(Cold)));
        declarations.push(indexed_key_decl(&declaration(keys::CHANNEL, "stream"), Int, false, Reject, Some(Cold)));
        declarations.push(indexed_key_decl(&declaration(keys::CHANNEL, "frequency_step_hz"), Num, false, Reject, None));
        declarations.push(indexed_key_decl(&declaration(keys::CHANNEL, "gain_step_db"), Num, false, Reject, None));
        declarations.push(indexed_key_decl(&declaration(keys::CHANNEL, "phase_behavior_on_retune"), Str, false, Reject, None));
        declarations.push(indexed_key_decl(&declaration(keys::CHANNEL, "path_delay_samples"), Int, false, Reject, None));
        declarations.push(indexed_key_decl(&declaration(keys::STREAM, keys::STREAM_LEAF), Num, true, Reject, Some(Cold)));
    }
    declarations
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

/// Describes the Radio Model Vocabulary `radio` 2.0.0 (RM-1).
pub fn vocabulary() -> VocabularyDescriptor {
    VocabularyDescriptor {
        id: radio_namespace().clone(),
        version: Version::new(2, 0, 0),
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
            VerbDecl {
                verb: Ident::parse(START_RX).expect("a valid Session verb"),
                compiles_to: CompileRule::Command {},
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
            check_numbered_configuration(&configuration, &envelope, &fragment, self, &mut violations);
        }
        violations
    }
}

/// The concrete RM-27 key `key` spells: a channel's leaf or a stream's rate (RM-27).
/// `rx` is `true` for a receive channel or stream.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Concrete<'a> {
    /// `radio.<direction>.c<n>.<leaf>`.
    Channel {
        /// Whether it is a receive key.
        rx: bool,
        /// The channel number, without a leading zero (RM-27).
        number: u32,
        /// The key's leaf, one of [`keys::CHANNEL_LEAVES`].
        leaf: &'a str,
    },
    /// `radio.<direction>.s<k>.sample_rate_hz`.
    Stream {
        /// Whether it is a receive key.
        rx: bool,
        /// The stream number, without a leading zero (RM-27).
        number: u32,
    },
}

/// The one place the numbered spellings `c<n>` and `s<k>` are read (RM-27): the key `key`
/// names, if it names one. `coerce`, `is_configuration` and `RfEnvelopeCheck` all use it.
pub fn concrete(key: &Key) -> Option<Concrete<'_>> {
    let segments: Vec<&str> = key.as_str().split('.').collect();
    let [prefix, direction, segment, leaf] = <[&str; 4]>::try_from(segments.as_slice()).ok()?;
    if prefix != crate::VOCABULARY {
        return None;
    }
    let rx = match direction {
        "rx" => true,
        "tx" => false,
        _ => return None,
    };
    if let Some(number) = numbered_index(segment, keys::CHANNEL) {
        return keys::CHANNEL_LEAVES
            .contains(&leaf)
            .then_some(Concrete::Channel { rx, number, leaf });
    }
    if leaf == keys::STREAM_LEAF {
        let number = numbered_index(segment, keys::STREAM)?;
        return Some(Concrete::Stream { rx, number });
    }
    None
}

/// The number `segment` spells after `word` (SB-2a's own spelling of an index): one to
/// nine ASCII digits, and no leading zero.
pub(crate) fn numbered_index(segment: &str, word: &str) -> Option<u32> {
    let digits = segment.strip_prefix(word)?;
    if digits.is_empty()
        || digits.len() > 9
        || (digits.len() > 1 && digits.starts_with('0'))
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    digits.parse().ok()
}

/// RM-12, RM-13, RM-16: with a layout description (RM-29) an Action may name a stream's
/// node instead of the device: `target` is `<device>/rx/s<k>` or `<device>/tx/s<k>`, its
/// `k` written as SB-2a's index. Returns that stream's direction and number, or `None` for
/// another node. The one place such a target is read.
pub fn stream_target(
    device: &ezsdr_kernel::id::ResourceId,
    target: &ezsdr_kernel::id::ResourceId,
) -> Option<(Direction, u32)> {
    let stream = target.parent()?;
    if stream.parent().as_ref() != Some(device) {
        return None;
    }
    let direction = match stream.segments().last()? {
        "rx" => Direction::Rx,
        "tx" => Direction::Tx,
        _ => return None,
    };
    Some((direction, numbered_index(target.segments().last()?, keys::STREAM)?))
}

/// What an Action's resolved node names (KC-23, RM-12, RM-29): the instance's own node, one
/// of its layout streams, or something else.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Addressed {
    /// The instance's own node, `<device>`.
    Instance,
    /// One of the instance's streams.
    Stream {
        /// Its direction.
        direction: Direction,
        /// Its number.
        k: usize,
    },
    /// A node that is neither.
    Other,
}

/// RM-12, RM-29: what the resolved node `node` of an Action names for `device` — its own
/// node, one of its streams, or another node. `streams` is how many streams a direction has,
/// so that a number the instance does not have is another node.
pub fn addressed(
    device: &ezsdr_kernel::id::ResourceId,
    node: Option<&ezsdr_kernel::id::ResourceId>,
    streams: impl Fn(Direction) -> usize,
) -> Addressed {
    let Some(node) = node else { return Addressed::Other };
    if node == device {
        return Addressed::Instance;
    }
    if let Some((direction, number)) = stream_target(device, node) {
        let k = number as usize;
        if k < streams(direction) {
            return Addressed::Stream { direction, k };
        }
    }
    Addressed::Other
}

/// RM-33's per-channel judgement of a configuration that holds RM-27 channel keys.
fn check_numbered_configuration(
    configuration: &BTreeMap<Key, Value>,
    envelope: &RfEnvelope,
    fragment: &Ident,
    check: &RfEnvelopeCheck,
    violations: &mut Vec<Violation>,
) {
    let mut channels: BTreeMap<(bool, i64), BTreeMap<&str, &Value>> = BTreeMap::new();
    for (key, value) in configuration {
        if let Some(Concrete::Channel { rx: is_rx, number, leaf }) = concrete(key) {
            channels.entry((is_rx, i64::from(number))).or_default().insert(leaf, value);
        }
    }
    for ((is_rx, number), values) in &channels {
        let Some(stream) = values.get("stream").and_then(|value| value.as_scalar()).and_then(integer)
        else {
            continue;
        };
        if stream < 0 {
            continue;
        }
        if !*is_rx {
            if let Some(value) = values.get("frequency_hz") {
                let key = channel_key(false, *number, "frequency_hz");
                match numeric(value) {
                    Some(frequency) => {
                        if !envelope
                            .allowed_bands
                            .iter()
                            .any(|band| band.lo_hz <= frequency && frequency <= band.hi_hz)
                        {
                            push_numbered_violation(
                                violations,
                                check,
                                fragment,
                                &key,
                                Some((*value).clone()),
                                format!("{frequency} Hz is in no allowed band"),
                            );
                        }
                    }
                    None => push_numbered_violation(
                        violations,
                        check,
                        fragment,
                        &key,
                        Some((*value).clone()),
                        format!("radio.tx.c{number}.frequency_hz is not a number"),
                    ),
                }
            }
            if let Some(value) = values.get("gain_db") {
                let key = channel_key(false, *number, "gain_db");
                match numeric(value) {
                    Some(gain) => {
                        if let Some(maximum) = envelope.max_gain_db.filter(|maximum| gain > *maximum) {
                            push_numbered_violation(
                                violations,
                                check,
                                fragment,
                                &key,
                                Some((*value).clone()),
                                format!("gain {gain} dB exceeds {maximum}"),
                            );
                        }
                    }
                    None => push_numbered_violation(
                        violations,
                        check,
                        fragment,
                        &key,
                        Some((*value).clone()),
                        format!("radio.tx.c{number}.gain_db is not a number"),
                    ),
                }
            }
            let enabled = match &envelope.tx_enabled {
                None => true,
                Some(TxEnabled::All(enabled)) => *enabled,
                Some(TxEnabled::PerChannel(list)) => usize::try_from(*number)
                    .ok()
                    .and_then(|index| list.get(index))
                    .copied()
                    .unwrap_or(false),
            };
            if !enabled {
                push_numbered_violation(
                    violations,
                    check,
                    fragment,
                    &channel_key(false, *number, "stream"),
                    values.get("stream").map(|value| (*value).clone()),
                    format!("transmit channel c{number} is not enabled"),
                );
            }
        }
        check_channel_antenna(values, envelope, fragment, check, *is_rx, *number, violations);
    }
}

fn check_channel_antenna(
    values: &BTreeMap<&str, &Value>,
    envelope: &RfEnvelope,
    fragment: &Ident,
    check: &RfEnvelopeCheck,
    is_rx: bool,
    number: i64,
    violations: &mut Vec<Violation>,
) {
    let Some(value) = values.get("antenna") else {
        return;
    };
    let key = channel_key(is_rx, number, "antenna");
    let Some(antenna) = value.as_scalar().and_then(text) else {
        push_numbered_violation(
            violations,
            check,
            fragment,
            &key,
            Some((*value).clone()),
            format!("radio.{} antenna is not a string", if is_rx { "rx" } else { "tx" }),
        );
        return;
    };
    if envelope
        .antenna_ports
        .as_ref()
        .is_some_and(|ports| !ports.iter().any(|port| port == antenna))
    {
        push_numbered_violation(
            violations,
            check,
            fragment,
            &key,
            Some((*value).clone()),
            format!("antenna {antenna} is not allowed"),
        );
    }
}

/// The concrete key of a numbered channel (RM-33).
fn channel_key(is_rx: bool, number: i64, leaf: &str) -> Key {
    let direction = if is_rx { "rx" } else { "tx" };
    Key::parse(&keys::channel(
        direction,
        u32::try_from(number).expect("a parsed channel number"),
        leaf,
    ))
    .expect("a declared Radio Model key is valid")
}

fn push_numbered_violation(
    violations: &mut Vec<Violation>,
    check: &RfEnvelopeCheck,
    fragment: &Ident,
    key: &Key,
    requested: Option<Value>,
    detail: String,
) {
    violations.push(Violation {
        check: check.section().clone(),
        requested,
        key: Some(key.clone()),
        reason: format!("RM-19: {fragment}: {detail}"),
    });
}

fn numeric(value: &Value) -> Option<f64> {
    value.as_scalar().and_then(|scalar| match scalar {
        Scalar::Int(value) => Some(*value as f64),
        Scalar::Num(value) => Some(value.get()),
        _ => None,
    })
}

fn integer(scalar: &Scalar) -> Option<i64> {
    match scalar {
        Scalar::Int(value) => Some(*value),
        _ => None,
    }
}

fn text(scalar: &Scalar) -> Option<&str> {
    match scalar {
        Scalar::Str(value) => Some(value),
        _ => None,
    }
}

/// Timing values recorded in a radio Provider's Manifest (RM-20).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TimingEnvelope {
    /// Minimum lead for a timed command (RM-20).
    pub min_timed_command_lead_ns: i64,
    /// Time from arm to stream start (RM-20).
    pub startup_latency_ns: i64,
    /// Timed-command queue capacity (RM-20).
    pub command_queue_depth: i64,
    /// How far ahead of its instant a timed command is handed to the device (RM-20).
    pub release_window_ns: i64,
    /// Receive restart gap after overrun (RM-20).
    pub overflow_restart_gap_ns: i64,
    /// From a receive stream's previous cut, and from the instant its Provider is ready for
    /// it, to a new segment's first sample (RM-20, RM-25).
    pub rx_start_lead_ns: i64,
    /// The same for a transmit stream (RM-20, RM-25).
    pub tx_start_lead_ns: i64,
    /// Maximum time ahead of the device that transmit samples may be handed over (RM-20).
    pub tx_in_flight_ns: i64,
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
    }

    /// The length of [`RxOverflowPayload`]'s hot-path form (RM-24).
    pub const RX_OVERFLOW_HOT_BYTES: usize = 9;

    /// `RX_OVERFLOW`'s payload as a Manifest holds it when a Provider emitted it on the
    /// hot path: the Kernel's drain turns the record's bytes into this array (RS-34).
    /// Its schema gives a Manifest reader the shape; RM-24 gives the layout, which only
    /// the Radio Model's own reader interprets.
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
    #[serde(transparent)]
    pub struct RxOverflowHotPayload(pub [u8; RX_OVERFLOW_HOT_BYTES]);

    impl RxOverflowPayload {
        /// The hot-path form: the cause byte (0 overrun, 1 sequence), then `lost` as a
        /// little-endian u64 (RM-24). The Vocabulary owns this layout; the Kernel only queues
        /// the bytes (RS-32a, withdrawn).
        pub fn to_hot(&self) -> [u8; RX_OVERFLOW_HOT_BYTES] {
            let mut out = [0; RX_OVERFLOW_HOT_BYTES];
            out[0] = match self.cause {
                RxOverflowCause::Overrun => 0,
                RxOverflowCause::Sequence => 1,
            };
            out[1..9].copy_from_slice(&self.lost.to_le_bytes());
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
            Ok(RxOverflowPayload { cause, lost })
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

    /// The single `RX_OVERFLOW` for a block carrying one or more reported losses, `lost` the
    /// part of its jump no earlier event reported (RM-17).
    pub fn rx_overflow_payload(reports: &[RxOverflowCause], lost: Option<u64>) -> Option<RxOverflowPayload> {
        let lost = lost?;
        let cause = if reports.contains(&RxOverflowCause::Overrun) {
            RxOverflowCause::Overrun
        } else if reports.contains(&RxOverflowCause::Sequence) {
            RxOverflowCause::Sequence
        } else {
            return None;
        };
        Some(RxOverflowPayload { cause, lost })
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
        /// The changed key, or null for a stream start (RM-22).
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
        /// The id of the Action refused, when the Provider refuses it as it takes it
        /// from its queue; absent when it refuses later what it booked (RM-22, MA-14).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub action: Option<ezsdr_kernel::event::ActionId>,
        /// The Action's kind tag (RM-22).
        pub kind: String,
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

#[cfg(test)]
mod tests {
    use super::{CancelReason, cancellation_reports};

    #[test]
    fn cancellation_reports_only_cold_change() {
        for (reason, reports) in [
            (CancelReason::Stop, false),
            (CancelReason::ProviderStop, false),
            (CancelReason::Loss, false),
            (CancelReason::ColdChange, true),
        ] {
            assert_eq!(cancellation_reports(reason), reports, "{reason:?}");
        }
    }
}
