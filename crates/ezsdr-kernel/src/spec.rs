//! Identifiers, values, constraints and the ExperimentSpec envelope —
//! `03-spec-and-binding.md` SB-1…SB-20, SB-47…SB-49 (Vision §8, §9, §10).

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::contract::PortRef;
use crate::event::EventKind;
use crate::module_api::ComponentDescriptor;
use crate::policy::Reaction;
use crate::stream::BackPressure;

/// A name inside one document, matching `^[a-z][a-z0-9_]*$`.
///
/// Names are compared as bytes; there is no case folding and no Unicode
/// normalisation, so the canonical form of OV-15 stays byte-comparable.
///
/// Rule: SB-1.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(transparent)]
pub struct Ident(String);

/// A dotted sequence of [`Ident`]s owned by a Vocabulary or a Module,
/// matching `^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$`.
///
/// Rule: SB-1.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(transparent)]
pub struct Namespace(String);

/// A `<vocabulary prefix><path>` or `ext.<module-id>.<path>` key.
///
/// The Kernel checks the prefix and the value's shape against the [`KeyDecl`] and
/// never interprets the meaning (audit Finding 7).
///
/// Rule: SB-2, MA-34.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(transparent)]
pub struct Key(String);

fn parse_segment(seg: &str) -> bool {
    let mut chars = seg.chars();
    matches!(chars.next(), Some('a'..='z')) && chars.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_'))
}

impl Ident {
    /// Parses an identifier (SB-1).
    pub fn parse(s: &str) -> Result<Ident, SpecError> {
        if parse_segment(s) {
            Ok(Ident(s.to_owned()))
        } else {
            Err(SpecError::UnknownField { path: s.to_owned() })
        }
    }

    /// The name as written (SB-1).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Namespace {
    /// Parses a dotted namespace (SB-1).
    pub fn parse(s: &str) -> Result<Namespace, SpecError> {
        if !s.is_empty() && s.split('.').all(parse_segment) {
            Ok(Namespace(s.to_owned()))
        } else {
            Err(SpecError::UnknownField { path: s.to_owned() })
        }
    }

    /// The namespace as written (SB-1).
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// True when `self` is `other` or lies under it, on segment boundaries (SB-2).
    pub fn is_under(&self, other: &Namespace) -> bool {
        self.0 == other.0
            || (self.0.starts_with(&other.0) && self.0.as_bytes().get(other.0.len()) == Some(&b'.'))
    }
}

impl Key {
    /// Parses a key; at least two segments, since a bare prefix names no path (SB-2).
    pub fn parse(s: &str) -> Result<Key, SpecError> {
        let segments = s.split('.').count();
        if segments >= 2 && s.split('.').all(parse_segment) {
            Ok(Key(s.to_owned()))
        } else {
            Err(SpecError::UnknownKeyPrefix { key: s.to_owned() })
        }
    }

    /// The key as written (SB-2).
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// True when the key sits under `prefix` (SB-2, MA-34).
    pub fn has_prefix(&self, prefix: &Namespace) -> bool {
        self.0.starts_with(prefix.as_str())
            && self.0.as_bytes().get(prefix.as_str().len()) == Some(&b'.')
    }

    /// The `ext.<module-id>` escape prefix (MA-34).
    pub fn is_extension(&self) -> bool {
        self.0.starts_with("ext.")
    }
}

macro_rules! display_newtype {
    ($($t:ty),*) => {$(
        impl fmt::Display for $t {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.0) }
        }
    )*};
}
display_newtype!(Ident, Namespace, Key);

/// A scalar, or a list or map of scalars nested at most one level.
///
/// A Spec is data, not a document tree: one level is what a structured parameter
/// such as a capture request needs, and more invites the schema-inside-a-schema
/// that Vision §9 rejects.
///
/// Rule: SB-4.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum Value {
    /// A boolean.
    Bool(bool),
    /// A signed 64-bit integer, written as an exact decimal by the canonicaliser (OV-15).
    Int(i64),
    /// A finite float; a non-finite one is refused rather than written as `null`,
    /// because a `Value` reaches the sealed Manifest through `prepare.effective`,
    /// the coercion records and the action log (OV-15, X9).
    Num(#[serde(serialize_with = "crate::hash::serialize_finite_f64")] f64),
    /// A string.
    Str(String),
    /// A list of scalars; nesting deeper is refused (SB-4).
    List(Vec<Value>),
    /// A map of scalars; nesting deeper is refused (SB-4).
    Map(BTreeMap<String, Value>),
}

/// The declared shape of a key's value (SB-2, MA-34).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ValueKind {
    /// [`Value::Bool`].
    Bool,
    /// [`Value::Int`].
    Int,
    /// [`Value::Num`].
    Num,
    /// [`Value::Str`].
    Str,
    /// [`Value::List`].
    List,
    /// [`Value::Map`].
    Map,
}

impl Value {
    /// True for the four scalar variants; a constraint's value must be one (SB-5).
    pub fn is_scalar(&self) -> bool {
        matches!(self, Value::Bool(_) | Value::Int(_) | Value::Num(_) | Value::Str(_))
    }

    /// The declared shape this value has (SB-2).
    pub fn kind(&self) -> ValueKind {
        match self {
            Value::Bool(_) => ValueKind::Bool,
            Value::Int(_) => ValueKind::Int,
            Value::Num(_) => ValueKind::Num,
            Value::Str(_) => ValueKind::Str,
            Value::List(_) => ValueKind::List,
            Value::Map(_) => ValueKind::Map,
        }
    }

    /// Refuses nesting beyond one level, and any non-finite float (SB-4, OV-15).
    pub fn check_nesting(&self, path: &str) -> Result<(), SpecError> {
        match self {
            Value::Num(x) if !x.is_finite() => {
                Err(SpecError::KeyShape { key: path.to_owned(), expected: "finite number".into(), found: "non-finite".into() })
            }
            Value::List(items) => {
                if !items.iter().all(Value::is_scalar) {
                    return Err(SpecError::KeyShape {
                        key: path.to_owned(),
                        expected: "a list of scalars".into(),
                        found: "nested list or map".into(),
                    });
                }
                // Recursed, so that a non-finite float inside a list is refused too.
                items.iter().try_for_each(|v| v.check_nesting(path))
            }
            Value::Map(items) => {
                if !items.values().all(Value::is_scalar) {
                    return Err(SpecError::KeyShape {
                        key: path.to_owned(),
                        expected: "a map of scalars".into(),
                        found: "nested list or map".into(),
                    });
                }
                items.values().try_for_each(|v| v.check_nesting(path))
            }
            _ => Ok(()),
        }
    }

    /// Orders two scalars of one kind; `None` across kinds, which is `KeyShape` (SB-6).
    pub fn partial_cmp_scalar(&self, other: &Value) -> Option<std::cmp::Ordering> {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => Some(a.cmp(b)),
            (Value::Num(a), Value::Num(b)) => a.partial_cmp(b),
            (Value::Int(a), Value::Num(b)) => (*a as f64).partial_cmp(b),
            (Value::Num(a), Value::Int(b)) => a.partial_cmp(&(*b as f64)),
            (Value::Str(a), Value::Str(b)) => Some(a.cmp(b)),
            (Value::Bool(a), Value::Bool(b)) => Some(a.cmp(b)),
            _ => None,
        }
    }
}

/// What a Spec requires of a key. Its value is a scalar: a constraint over a list
/// or a map is refused, because the matcher would then need the Vocabulary's
/// semantics to compare them.
///
/// Rule: SB-5, decision B1.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Constraint {
    /// Exactly this value.
    Eq {
        /// The value.
        value: Value,
    },
    /// Within these bounds, either of which may be absent.
    Range {
        /// Lower bound, inclusive.
        min: Option<Value>,
        /// Upper bound, inclusive.
        max: Option<Value>,
    },
    /// Any of these values.
    Set {
        /// The permitted values.
        values: Vec<Value>,
    },
    /// At least this value.
    Min {
        /// The lower bound.
        value: Value,
    },
    /// At most this value.
    Max {
        /// The upper bound.
        value: Value,
    },
    /// The key is declared at all.
    Present,
}

impl Constraint {
    /// Every scalar this constraint mentions; SB-5 requires them all to be scalars.
    pub fn values(&self) -> Vec<&Value> {
        match self {
            Constraint::Eq { value } | Constraint::Min { value } | Constraint::Max { value } => {
                vec![value]
            }
            Constraint::Range { min, max } => min.iter().chain(max.iter()).collect(),
            Constraint::Set { values } => values.iter().collect(),
            Constraint::Present => Vec::new(),
        }
    }
}

/// What a Provider declares it can do for a key (spec 05 `ProviderInstance`).
///
/// Rule: SB-6.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CapabilityValue {
    /// Exactly this value.
    One {
        /// The value.
        value: Value,
    },
    /// A continuous range, inclusive.
    Range {
        /// Lower bound.
        min: Value,
        /// Upper bound.
        max: Value,
    },
    /// A discrete set.
    AnyOf {
        /// The declared values.
        values: Vec<Value>,
    },
}

/// What a Vocabulary declares about one key (spec 05, MA-35).
///
/// Rule: SB-2, SB-7, SB-45.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct KeyDecl {
    /// The key this declares (SB-2).
    pub key: Key,
    /// The shape its value takes (SB-2).
    pub kind: ValueKind,
    /// Whether a constraint the capability does not satisfy directly may be
    /// offered to the Provider's `coerce` (SB-7).
    pub coercible: bool,
    /// The Vocabulary's default coercion policy for this key (SB-45).
    pub coercion_default: CoercionPolicy,
}

/// What to do when a Provider coerces a requested value (SB-45, SB-46).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CoercionPolicy {
    /// Apply and record.
    Accept,
    /// Apply, record and warn.
    Warn,
    /// Fail the stage with `CoercionRejected`.
    Reject,
}

/// A value the Provider changed, with its reason (SB-44).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Coercion {
    /// The key that was coerced.
    pub key: Key,
    /// What the document asked for.
    pub requested: Value,
    /// What the Provider will apply.
    pub applied: Value,
    /// The Provider's reason, uninterpreted by the Kernel.
    pub reason: String,
}

/// A non-fatal note from a stage (SB-38, SB-41).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Warning {
    /// Where it came from.
    pub source: Namespace,
    /// What it says; uninterpreted by the Kernel.
    pub message: String,
}

/// A constraint the bound instance could not satisfy (SB-38).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RejectedConstraint {
    /// The Spec resource whose constraint failed.
    pub resource: Ident,
    /// The key.
    pub key: Key,
    /// What was asked.
    pub constraint: Constraint,
    /// Why it failed, uninterpreted.
    pub reason: String,
}

// ---------------------------------------------------------------- the envelope

/// A capability this resource needs from somewhere, possibly another instance (SB-36).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SubResourceReq {
    /// The kind of node that can satisfy it.
    pub kind: Namespace,
    /// What it must declare.
    #[serde(default)]
    pub requires: BTreeMap<Key, Constraint>,
}

/// What a Spec requires of one resource, and nothing about how it is met.
///
/// Direction-asymmetric requests are expressed as distinct keys under the
/// Vocabulary's prefix — `radio.rx.channels`, `radio.tx.channels` — not as a single
/// count. The Kernel does not know that `rx` means anything; the flattening is what
/// keeps it generic while still expressing the asymmetry re-review R1 required.
///
/// Rule: SB-12, decision B2.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ResourceReq {
    /// The kind of resource wanted (SB-12).
    pub kind: Namespace,
    /// Per-key constraints (SB-12).
    #[serde(default)]
    pub requires: BTreeMap<Key, Constraint>,
    /// Capabilities this resource needs from elsewhere (SB-36).
    #[serde(default)]
    pub needs: BTreeMap<Ident, SubResourceReq>,
    /// Namespaced opaque content (SB-20).
    #[serde(default)]
    pub extensions: BTreeMap<Namespace, serde_json::Value>,
}

/// A link the Spec's graph declares. Both `policy` and `capacity` are mandatory:
/// SC-19 forbids a default, and SC-21's Sink rule is a statement about the policy.
///
/// Rule: SB-15.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct LinkReq {
    /// The producing port.
    pub from: PortRef,
    /// The consuming port.
    pub to: PortRef,
    /// What happens at capacity (SC-19).
    pub policy: BackPressure,
    /// Queue depth in blocks (SC-19).
    pub capacity: u32,
}

/// A resource name plus an offset from the Run's start, in that resource's stream
/// clock. A Spec cannot name a `ClockDomainId`, because domains are allocated at
/// `prepare` (TM-13a) and fixed at `arm` for a transmit stream (TM-13e).
///
/// Rule: SB-16, decision B4.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SpecTime {
    /// The Spec resource whose stream clock the offset counts in.
    pub clock: Ident,
    /// Ticks from the Run's start.
    pub offset_ticks: i64,
}

/// Where an output's samples come from (SB-17).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OutputSource {
    /// A component's port.
    Port {
        /// Which port.
        port: PortRef,
    },
    /// A Spec resource.
    Resource {
        /// Which resource.
        resource: Ident,
    },
}

/// An artifact the Run must produce. A capture whose source has no placed Sink is
/// refused at `validate()`.
///
/// Rule: SB-17.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct OutputReq {
    /// The artifact's name inside this document.
    pub id: Ident,
    /// The artifact kind; a Sink Module's namespace.
    pub kind: Namespace,
    /// Where the samples come from.
    pub source: OutputSource,
    /// Sink parameters, uninterpreted by the Kernel.
    #[serde(default)]
    pub params: BTreeMap<Key, Value>,
}

/// The Vocabulary majors this Spec's keys belong to. It is the only content of
/// `requirements`; per-resource requirements live in `resources[].requires`, which
/// is the division that makes SB-2's prefix check possible.
///
/// Rule: SB-11.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Requirements {
    /// One entry per Vocabulary the Spec's keys draw on (SB-11).
    #[serde(default)]
    pub vocabularies: Vec<VocabularyReq>,
}

/// One Vocabulary this Spec requires, by namespace and major version (SB-11).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct VocabularyReq {
    /// The Vocabulary's namespace, which is also its key prefix (SB-2).
    pub id: Namespace,
    /// The major version required (SB-11).
    pub major: u32,
}

/// The Spec's component graph (SB-15).
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SpecGraph {
    /// Components by name (SB-15).
    #[serde(default)]
    pub components: BTreeMap<Ident, ComponentDescriptor>,
    /// Links between their ports (SB-15).
    #[serde(default)]
    pub links: Vec<LinkReq>,
}

/// The Spec's failure and coercion tables (SB-18, SB-19).
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SpecPolicies {
    /// Registered event kinds mapped to reactions; an unregistered kind is refused
    /// at `validate()`, so a misspelling is an error rather than a silently
    /// ineffective entry (SB-18).
    #[serde(default)]
    pub failure: BTreeMap<EventKind, Reaction>,
    /// Per-key coercion policy; SB-45 gives the resolution order (SB-19).
    #[serde(default)]
    pub coercion: BTreeMap<Key, CoercionPolicy>,
}

/// One scheduled Action, held as a template because the Spec cannot name a
/// `ClockDomainId` (RS-49a, SB-16).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ScheduleEntry {
    /// When, relative to the Run's start (SB-16).
    pub at: SpecTime,
    /// The Action without its time field (RS-49a).
    pub action: crate::event::ActionTemplate,
}

/// What an experiment requires: intent, portable, with nothing about placement or
/// environment in it.
///
/// Rule: SB-9…SB-20. Vision §8, §9, §10.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ExperimentSpec {
    /// Mandatory positive integer; Phase 1 supports exactly `{1}` (SB-10, SB-47).
    pub version: u32,
    /// The Vocabulary majors the keys belong to (SB-11).
    #[serde(default)]
    pub requirements: Requirements,
    /// Per-resource requirements (SB-12).
    #[serde(default)]
    pub resources: BTreeMap<Ident, ResourceReq>,
    /// Components and links (SB-15).
    #[serde(default)]
    pub graph: SpecGraph,
    /// Actions at relative times (SB-16).
    #[serde(default)]
    pub schedule: Vec<ScheduleEntry>,
    /// The artifacts the Run must produce (SB-17).
    #[serde(default)]
    pub outputs: Vec<OutputReq>,
    /// Failure and coercion tables (SB-18, SB-19).
    #[serde(default)]
    pub policies: SpecPolicies,
    /// Namespaced opaque content, copied into the Manifest and never interpreted (SB-20).
    #[serde(default)]
    pub extensions: BTreeMap<Namespace, serde_json::Value>,
}

/// The document versions Phase 1 supports (SB-10, SB-47).
pub const SUPPORTED_VERSIONS: &[u32] = &[1];

/// Top-level fields an [`ExperimentSpec`] may carry. An unknown one is
/// `UnknownField`, not a warning: v3's configuration accreted keys nobody removed
/// (SB-9).
pub const SPEC_TOP_LEVEL: &[&str] = &[
    "version",
    "requirements",
    "resources",
    "graph",
    "schedule",
    "outputs",
    "policies",
    "extensions",
];

/// Field names a Spec may not carry at any depth outside `extensions`. A Spec
/// naming an Executor cannot be promoted to a host that lacks one, which is the
/// regression re-review R21 caught (SB-13, decision B3).
pub const PLACEMENT_FIELDS: &[&str] =
    &["placements", "placement", "executor", "memory_domain", "island"];

/// The one field name SB-13 reports as `EnvironmentInSpec` rather than
/// `PlacementInSpec` (SB-13).
pub const ENVIRONMENT_FIELD: &str = "environment";

/// Why a document was refused (SB-9…SB-13, SB-47).
#[derive(Clone, PartialEq, Debug)]
pub enum SpecError {
    /// The version is not one Phase 1 supports; it is never interpreted under a
    /// newer version's defaults (SB-47).
    UnsupportedVersion {
        /// What the document declared.
        found: u32,
        /// What this build supports.
        supported: Vec<u32>,
    },
    /// A field outside the closed envelope, or a malformed identifier (SB-9, SB-1).
    UnknownField {
        /// Dotted path to the offending field.
        path: String,
    },
    /// A placement field appeared in a Spec (SB-13).
    PlacementInSpec {
        /// Dotted path to it.
        path: String,
    },
    /// An environment field appeared in a Spec (SB-13).
    EnvironmentInSpec {
        /// Dotted path to it.
        path: String,
    },
    /// The key's prefix belongs to no Vocabulary the document declares (SB-2).
    UnknownKeyPrefix {
        /// The key.
        key: String,
    },
    /// The value's shape does not match the `KeyDecl`, or two scalars of different
    /// kinds were compared (SB-6).
    KeyShape {
        /// The key or path.
        key: String,
        /// What was wanted.
        expected: String,
        /// What was there.
        found: String,
    },
    /// A Spec resource has no binding (SB-22).
    UnboundResource {
        /// The resource name.
        name: Ident,
    },
    /// No single instance satisfies the requirement; the Core never assembles a
    /// capability across instances (SB-35).
    NoSingleInstance {
        /// The resource name.
        name: Ident,
        /// The constraint that could not be met by one instance.
        constraint: String,
    },
    /// The arm-order edges form a cycle (SB-39).
    ArmCycle {
        /// The cycle, as a dotted path.
        path: String,
    },
    /// An admission check refused the stage (SB-30).
    Violation(crate::binding::Violation),
    /// A coercion was refused under the `reject` policy (SB-46).
    CoercionRejected(Coercion),
    /// A structural rule of a neighbouring spec failed; the message names it.
    Structural {
        /// What was wrong, naming the rule.
        reason: String,
    },
}

impl fmt::Display for SpecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SpecError::UnsupportedVersion { found, supported } => {
                write!(f, "document version {found} is not supported (supported: {supported:?})")
            }
            SpecError::UnknownField { path } => write!(f, "unknown field {path:?}"),
            SpecError::PlacementInSpec { path } => {
                write!(f, "placement field {path:?} belongs in the BindingProfile, not a Spec")
            }
            SpecError::EnvironmentInSpec { path } => {
                write!(f, "environment field {path:?} belongs in the BindingProfile, not a Spec")
            }
            SpecError::UnknownKeyPrefix { key } => {
                write!(f, "key {key:?} has a prefix no declared Vocabulary owns")
            }
            SpecError::KeyShape { key, expected, found } => {
                write!(f, "key {key:?} wanted {expected} and found {found}")
            }
            SpecError::UnboundResource { name } => write!(f, "resource {name} has no binding"),
            SpecError::NoSingleInstance { name, constraint } => {
                write!(f, "no single instance satisfies {constraint} for {name}")
            }
            SpecError::ArmCycle { path } => write!(f, "arm order has a cycle: {path}"),
            SpecError::Violation(v) => write!(f, "admission violation: {}", v.reason),
            SpecError::CoercionRejected(c) => {
                write!(f, "coercion of {} to {:?} was rejected", c.key, c.applied)
            }
            SpecError::Structural { reason } => f.write_str(reason),
        }
    }
}

impl std::error::Error for SpecError {}

/// Checks a document's `version` against [`SUPPORTED_VERSIONS`] (SB-10, SB-47).
pub fn check_version(doc: &serde_json::Value) -> Result<u32, SpecError> {
    let found = doc
        .get("version")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| SpecError::UnknownField { path: "version".to_owned() })?;
    let found = u32::try_from(found).unwrap_or(u32::MAX);
    if found == 0 || !SUPPORTED_VERSIONS.contains(&found) {
        return Err(SpecError::UnsupportedVersion {
            found,
            supported: SUPPORTED_VERSIONS.to_vec(),
        });
    }
    Ok(found)
}

/// Refuses a top-level field outside `allowed` (SB-9, SB-21).
pub fn check_top_level(doc: &serde_json::Value, allowed: &[&str]) -> Result<(), SpecError> {
    let map = doc.as_object().ok_or_else(|| SpecError::UnknownField { path: "$".to_owned() })?;
    match map.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(k) => Err(SpecError::UnknownField { path: k.clone() }),
        None => Ok(()),
    }
}

/// Scans a Spec for placement and environment field names at any depth outside
/// `extensions`. A field-name scan rather than a schema check: a Spec could
/// otherwise carry placement inside a Vocabulary section and still validate
/// (SB-13, decision B3).
pub fn check_no_placement(doc: &serde_json::Value) -> Result<(), SpecError> {
    fn walk(v: &serde_json::Value, path: &str) -> Result<(), SpecError> {
        match v {
            serde_json::Value::Object(map) => {
                for (k, child) in map {
                    let here = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
                    if k == "extensions" || k == "schema" {
                        // SB-20: `extensions` is opaque, and B3 accepts the residual
                        // risk. A parameter's `schema` is opaque Vocabulary content
                        // too (MA-36): scanning it refuses a component whose
                        // parameter object merely *describes* a property called
                        // `island`, which B3 never weighed. Raised as finding D19.
                        continue;
                    }
                    if k == ENVIRONMENT_FIELD {
                        return Err(SpecError::EnvironmentInSpec { path: here });
                    }
                    if PLACEMENT_FIELDS.contains(&k.as_str()) {
                        return Err(SpecError::PlacementInSpec { path: here });
                    }
                    walk(child, &here)?;
                }
                Ok(())
            }
            serde_json::Value::Array(items) => items
                .iter()
                .enumerate()
                .try_for_each(|(i, child)| walk(child, &format!("{path}[{i}]"))),
            _ => Ok(()),
        }
    }
    walk(doc, "")
}

impl ExperimentSpec {
    /// Parses and validates a Spec envelope: the version (SB-10, SB-47), the closed
    /// top-level set (SB-9), the placement and environment scan (SB-13), and then
    /// the shapes.
    ///
    /// v3's `CONSTANTS` and `!COMPUTE(...)` are refused here — by SB-9 as unknown
    /// fields and by SB-4 as values that are not scalars. There is no evaluation
    /// pass; parametrisation is the client-side builder of Vision §9 (SB-14).
    pub fn from_json(doc: &serde_json::Value) -> Result<ExperimentSpec, SpecError> {
        check_version(doc)?;
        check_top_level(doc, SPEC_TOP_LEVEL)?;
        check_no_placement(doc)?;
        let spec: ExperimentSpec = serde_json::from_value(doc.clone())
            .map_err(|e| SpecError::Structural { reason: format!("SB-9: {e}") })?;
        spec.check_shapes()?;
        Ok(spec)
    }

    /// The value and constraint shape rules (SB-4, SB-5).
    fn check_shapes(&self) -> Result<(), SpecError> {
        for (name, r) in &self.resources {
            // `needs[].requires` is a constraint map like any other (SB-36).
            let constraints = r.requires.iter().chain(r.needs.values().flat_map(|n| n.requires.iter()));
            for (key, c) in constraints {
                for v in c.values() {
                    if !v.is_scalar() {
                        return Err(SpecError::KeyShape {
                            key: key.to_string(),
                            expected: "a scalar".into(),
                            found: "a list or map".into(),
                        });
                    }
                    v.check_nesting(key.as_str())?;
                }
            }
            let _ = name;
        }
        for o in &self.outputs {
            for (key, v) in &o.params {
                v.check_nesting(key.as_str())?;
            }
        }
        Ok(())
    }

    /// Refuses a key whose prefix belongs to no Vocabulary this Spec declares in
    /// `requirements`, with `UnknownKeyPrefix` (SB-2, MA-34).
    pub fn check_key_prefixes(&self) -> Result<(), SpecError> {
        let declared: Vec<&Namespace> =
            self.requirements.vocabularies.iter().map(|v| &v.id).collect();
        let ok = |k: &Key| k.is_extension() || declared.iter().any(|ns| k.has_prefix(ns));
        for r in self.resources.values() {
            for k in r.requires.keys().chain(r.needs.values().flat_map(|n| n.requires.keys())) {
                if !ok(k) {
                    return Err(SpecError::UnknownKeyPrefix { key: k.to_string() });
                }
            }
        }
        for o in &self.outputs {
            for k in o.params.keys() {
                if !ok(k) {
                    return Err(SpecError::UnknownKeyPrefix { key: k.to_string() });
                }
            }
        }
        for k in self.policies.coercion.keys() {
            if !ok(k) {
                return Err(SpecError::UnknownKeyPrefix { key: k.to_string() });
            }
        }
        Ok(())
    }
}

/// A migration from one major version to the next, registered per document type.
///
/// Phase 1 registers none, because only version 1 exists; the refusal path and the
/// registration point both exist and are tested, so that adding version 2 is a
/// migration rather than a redesign.
///
/// Rule: SB-48.
pub type Migration = fn(serde_json::Value) -> Result<serde_json::Value, SpecError>;

/// The migrations registered for one document type (SB-48, SB-49).
#[derive(Default)]
pub struct MigrationRegistry {
    steps: BTreeMap<u32, Migration>,
}

impl MigrationRegistry {
    /// An empty registry, which is what Phase 1 ships (SB-48).
    pub fn new() -> MigrationRegistry {
        MigrationRegistry::default()
    }

    /// Registers the step from `from` to `from + 1` (SB-48).
    pub fn register(&mut self, from: u32, step: Migration) {
        self.steps.insert(from, step);
    }

    /// Migrates a document up to the newest supported version, or refuses it.
    /// Returns the migrated body and the original version, which the Manifest
    /// records beside the compiled one (SB-48, SB-49).
    pub fn migrate(
        &self,
        doc: serde_json::Value,
    ) -> Result<(serde_json::Value, u32), SpecError> {
        let newest = *SUPPORTED_VERSIONS.iter().max().expect("at least one supported version");
        let original = doc
            .get("version")
            .and_then(|v| v.as_u64())
            .map(|v| u32::try_from(v).unwrap_or(u32::MAX))
            .ok_or_else(|| SpecError::UnknownField { path: "version".to_owned() })?;
        let mut at = original;
        let mut doc = doc;
        while at < newest {
            let step = self.steps.get(&at).ok_or(SpecError::UnsupportedVersion {
                found: original,
                supported: SUPPORTED_VERSIONS.to_vec(),
            })?;
            doc = step(doc)?;
            at += 1;
        }
        if at != newest {
            return Err(SpecError::UnsupportedVersion {
                found: original,
                supported: SUPPORTED_VERSIONS.to_vec(),
            });
        }
        Ok((doc, original))
    }
}
