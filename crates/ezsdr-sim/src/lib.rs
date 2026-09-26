//! Ez-SDR v4 Simulation Vocabulary sim 1.1.0 (design/08-simulation.md; the channel is spec 11).
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod channel;

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use ezsdr_kernel::binding::{AdmissionCheck, AdmissionCheckRegistry, CheckStage, Violation};
use ezsdr_kernel::module_api::{
    ModuleError, ModuleRegistry, Version, VocabularyDescriptor,
};
use ezsdr_kernel::policy::EventKindRegistry;
use ezsdr_kernel::spec::{Ident, Key, Namespace, Value};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The Simulation root's ticks per second (SE-7).
pub const VIRTUAL_TICK_RATE_HZ: u64 = 1_000_000_000;
/// The Simulation root's arbitrary epoch name (SE-7).
pub const VIRTUAL_EPOCH: &str = "sim.run_start";
/// The Vocabulary id and key prefix (SE-1).
pub const VOCABULARY: &str = "sim";
/// The environment section carrying the Run seed (SE-1).
pub const SEED_SECTION: &str = "sim.seed";
/// The environment section carrying scheduled faults (SE-1).
pub const FAULTS_SECTION: &str = "sim.faults";

/// The kind of deterministic fault a Provider applies (SE-3).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FaultKind {
    /// Report an RX overrun (SE-3).
    RxOverflow,
    /// Report a receive sequence error (SE-3).
    RxSequenceError,
    /// Lose the device (SE-3).
    DeviceLost,
}

/// One fault scheduled relative to the Run's start instant (SE-3).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FaultEntry {
    /// Nanoseconds after T0, in the inclusive range `0..=2^62` (SE-3).
    pub at_ns: u64,
    /// The fault to apply (SE-3).
    pub fault: FaultKind,
    /// The Run fragment that owns the fault (SE-3).
    pub target: Ident,
}

/// The `sim.seed` admission check (SE-5).
pub struct SeedCheck;

/// The `sim.faults` admission check (SE-5).
pub struct FaultsCheck;

static SEED_NAMESPACE: OnceLock<Namespace> = OnceLock::new();
static FAULTS_NAMESPACE: OnceLock<Namespace> = OnceLock::new();
const VALIDATE: &[CheckStage] = &[CheckStage::Validate];

fn seed_namespace() -> &'static Namespace {
    SEED_NAMESPACE.get_or_init(|| Namespace::parse(SEED_SECTION).expect("a valid section name"))
}

fn faults_namespace() -> &'static Namespace {
    FAULTS_NAMESPACE.get_or_init(|| Namespace::parse(FAULTS_SECTION).expect("a valid section name"))
}

/// Describes Vocabulary `sim` 1.1.0 (SE-1).
pub fn vocabulary() -> VocabularyDescriptor {
    let id = Namespace::parse(VOCABULARY).expect("a valid Vocabulary namespace");
    VocabularyDescriptor {
        id: id.clone(),
        version: Version::new(1, 1, 0),
        prefix: id,
        keys: vec![],
        event_kinds: vec![],
        verbs: vec![],
        checks: vec![
            seed_namespace().clone(),
            faults_namespace().clone(),
            Namespace::parse(channel::CHANNEL_SECTION).expect("a valid section name"),
        ],
    }
}

/// Registers the descriptor, then the seed, fault and channel checks, in that order (SE-1, CH-2).
pub fn register(
    registry: &mut ModuleRegistry,
    checks: &mut AdmissionCheckRegistry,
    _kinds: &mut EventKindRegistry,
) -> Result<(), ModuleError> {
    registry.register_vocabulary(vocabulary())?;
    checks.register(Arc::new(SeedCheck));
    checks.register(Arc::new(FaultsCheck));
    checks.register(Arc::new(channel::ChannelCheck));
    Ok(())
}

/// Reads `sim.seed`, defaulting an absent section to zero (SE-2).
pub fn seed(environment: &BTreeMap<Namespace, serde_json::Value>) -> Result<u64, String> {
    match environment.get(seed_namespace()) {
        None => Ok(0),
        Some(value) => value.as_u64().ok_or_else(|| {
            format!("SE-2: sim.seed must be an integer in 0..=2^64-1, not {value}")
        }),
    }
}

/// Reads `sim.faults` in document order, refusing malformed or out-of-range entries (SE-3).
pub fn faults(
    environment: &BTreeMap<Namespace, serde_json::Value>,
) -> Result<Vec<FaultEntry>, String> {
    let Some(value) = environment.get(faults_namespace()) else {
        return Ok(vec![]);
    };
    let entries: Vec<FaultEntry> = serde_json::from_value(value.clone())
        .map_err(|error| format!("SE-3: {error}"))?;
    if let Some(entry) = entries.iter().find(|entry| entry.at_ns > (1_u64 << 62)) {
        return Err(format!("SE-3: at_ns {} exceeds 2^62", entry.at_ns));
    }
    Ok(entries)
}

fn one_section(section: &Namespace, value: &serde_json::Value) -> BTreeMap<Namespace, serde_json::Value> {
    BTreeMap::from([(section.clone(), value.clone())])
}

fn violation(check: &Namespace, reason: String) -> Violation {
    Violation {
        check: check.clone(),
        key: None,
        requested: None,
        reason,
    }
}

impl AdmissionCheck for SeedCheck {
    fn section(&self) -> &Namespace {
        seed_namespace()
    }

    fn stages(&self) -> &[CheckStage] {
        VALIDATE
    }

    fn check(
        &self,
        section: &serde_json::Value,
        _effective: &BTreeMap<Ident, BTreeMap<Key, Value>>,
        _proposed: &BTreeMap<Ident, BTreeMap<Key, Value>>,
        _stage: CheckStage,
    ) -> Vec<Violation> {
        match seed(&one_section(self.section(), section)) {
            Ok(_) => vec![],
            Err(reason) => vec![violation(self.section(), format!("SE-5: {reason}"))],
        }
    }
}

impl AdmissionCheck for FaultsCheck {
    fn section(&self) -> &Namespace {
        faults_namespace()
    }

    fn stages(&self) -> &[CheckStage] {
        VALIDATE
    }

    fn check(
        &self,
        section: &serde_json::Value,
        effective: &BTreeMap<Ident, BTreeMap<Key, Value>>,
        _proposed: &BTreeMap<Ident, BTreeMap<Key, Value>>,
        _stage: CheckStage,
    ) -> Vec<Violation> {
        match faults(&one_section(self.section(), section)) {
            Err(reason) => vec![violation(self.section(), format!("SE-5: {reason}"))],
            Ok(entries) => entries
                .iter()
                .filter(|entry| !effective.contains_key(&entry.target))
                .map(|entry| {
                    violation(
                        self.section(),
                        format!("SE-5: fault target {} names no fragment of this Run", entry.target),
                    )
                })
                .collect(),
        }
    }
}

/// SplitMix64 state seeded by the Run seed and a model-specific stream name (SE-6).
#[derive(Clone, Debug)]
pub struct SimRng {
    state: u64,
}

impl SimRng {
    /// Creates a named deterministic random stream (SE-6).
    pub fn new(seed: u64, stream: &str) -> SimRng {
        let mut hash = 0xcbf29ce484222325_u64;
        for byte in stream.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        SimRng { state: seed ^ hash }
    }

    /// Returns the next SplitMix64 value (SE-6).
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94D049BB133111EB);
        value ^ (value >> 31)
    }

    /// Returns a value in `0..n`, or zero when `n` is zero (SE-6).
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next_u64() % n }
    }
}

/// Generates the committed `channel`, `fault_entry` and `seed` schemas (SE-12, CH-10).
pub fn document_schemas() -> BTreeMap<&'static str, serde_json::Value> {
    fn of<T: JsonSchema>() -> serde_json::Value {
        serde_json::to_value(ezsdr_kernel::schema::generator().into_root_schema_for::<T>())
            .expect("a generated schema is JSON")
    }
    BTreeMap::from([
        ("channel", of::<channel::ChannelSpec>()),
        ("fault_entry", of::<FaultEntry>()),
        ("seed", of::<u64>()),
    ])
}
