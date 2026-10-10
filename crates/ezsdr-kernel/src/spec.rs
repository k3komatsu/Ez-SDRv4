//! Identifiers, values, constraints and the ExperimentSpec envelope —
//! `03-spec-and-binding.md` SB-1…SB-20, SB-47 (Vision §8, §9, §10).

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
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, schemars::JsonSchema,
)]
#[serde(transparent)]
pub struct Ident(String);

/// A dotted sequence of `Ident`s owned by a Vocabulary or a Module,
/// matching `^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$`.
///
/// Rule: SB-1.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, schemars::JsonSchema,
)]
#[serde(transparent)]
pub struct Namespace(String);

/// A `<vocabulary prefix><path>` or `ext.<module-id>.<path>` key: a Vocabulary key
/// matches `^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+$` and an Extension key
/// `^ext(\.[A-Za-z0-9_-]+){2,}$`; a key beginning `ext.` is read as an Extension key only.
///
/// The Kernel checks the prefix and the value's shape against the `KeyDecl` and
/// never interprets the meaning (audit Finding 7).
///
/// Rule: SB-1, SB-2, MA-34.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, schemars::JsonSchema,
)]
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
        if s.split('.').all(parse_segment) {
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
            || self
                .0
                .strip_prefix(&other.0)
                .is_some_and(|suffix| suffix.starts_with('.'))
    }
}

impl Key {
    /// Parses a key; at least two segments, since a bare prefix names no path (SB-2).
    pub fn parse(s: &str) -> Result<Key, SpecError> {
        let segments = s.split('.').count();
        // An `ext.` key carries a **Module id**, whose grammar (SB-1's table SB-T0) is laxer than a
        // Vocabulary prefix's: it admits `A-Z`, `-` and a leading digit. Holding an
        // `ext.` key to `Ident`'s grammar made MA-34's escape hatch unusable for every
        // Module whose id contains a dash or a capital — refused at `parse`, before
        // any rule could run (SB-1, SB-2, MA-34).
        let ok = if let Some(rest) = s.strip_prefix("ext.") {
            rest.split('.').count() >= 2 && rest.split('.').all(crate::id::is_module_segment)
        } else {
            segments >= 2 && s.split('.').all(parse_segment)
        };
        if ok {
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
        self.0
            .strip_prefix(prefix.as_str())
            .is_some_and(|suffix| suffix.starts_with('.'))
    }

    /// The `ext.<module-id>` escape prefix (MA-34).
    pub fn is_extension(&self) -> bool {
        self.0.starts_with("ext.")
    }
}

// SB-1 at the document boundary (D95): each name type deserialises through its own
// `parse`, so a document cannot carry a name a Rust caller could not have built.
impl<'de> Deserialize<'de> for Ident {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        crate::id::parsed(d, "an Ident", Ident::parse)
    }
}

impl<'de> Deserialize<'de> for Namespace {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        crate::id::parsed(d, "a Namespace", Namespace::parse)
    }
}

impl<'de> Deserialize<'de> for Key {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        crate::id::parsed(d, "a Key", Key::parse)
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

/// A finite 64-bit float: NaN and the infinities are refused, so OV-15's
/// canonicaliser never meets one.
///
/// Rule: SB-4, OV-15.
// The field is private and `new` is the only way in, deserialising included.
#[derive(Clone, Copy, PartialEq, PartialOrd, Debug, Serialize, schemars::JsonSchema)]
#[serde(transparent)]
pub struct Finite(f64);

impl Finite {
    /// `x`, unless it is NaN or an infinity (SB-4).
    pub fn new(x: f64) -> Option<Finite> {
        x.is_finite().then_some(Finite(x))
    }

    /// The float (SB-4).
    pub fn get(self) -> f64 {
        self.0
    }
}

impl<'de> Deserialize<'de> for Finite {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Finite::new(f64::deserialize(d)?)
            .ok_or_else(|| serde::de::Error::custom("SB-4: a number must be finite"))
    }
}

/// The one scalar type: a `Value`'s scalar, a constraint's and a capability's value,
/// and a `DataContract` attribute (SB-4, SB-5, SC-2).
///
/// Carries no tag (OV-13's carve-out): a scalar is the value an author wrote, not a
/// two-field object wrapping it.
///
/// Rule: SB-4, SB-6, SC-2.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum Scalar {
    /// A boolean.
    Bool(bool),
    /// A signed 64-bit integer, written as an exact decimal by the canonicaliser (OV-15).
    Int(i64),
    /// A finite float (OV-15).
    Num(Finite),
    /// A string.
    Str(String),
}

/// A scalar, or a list or map of scalars. One level is the type: nothing deeper can
/// be built or parsed.
///
/// A Spec is data, not a document tree: one level is what a structured parameter
/// such as a capture request needs, and more invites the schema-inside-a-schema
/// that Vision §9 rejects.
///
/// Rule: SB-4.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum Value {
    /// A scalar.
    Scalar(Scalar),
    /// A list of scalars.
    List(Vec<Scalar>),
    /// A map of scalars; its keys are ASCII (SB-9a).
    Map(BTreeMap<String, Scalar>),
}

/// The declared shape of a key's value (SB-2, MA-34).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
pub enum ValueKind {
    /// A boolean.
    Bool,
    /// An integer.
    Int,
    /// A float.
    Num,
    /// A string.
    Str,
    /// A list of scalars.
    List,
    /// A map of scalars.
    Map,
}

impl Scalar {
    /// The declared shape this scalar has (SB-2).
    pub fn kind(&self) -> ValueKind {
        match self {
            Scalar::Bool(_) => ValueKind::Bool,
            Scalar::Int(_) => ValueKind::Int,
            Scalar::Num(_) => ValueKind::Num,
            Scalar::Str(_) => ValueKind::Str,
        }
    }
}

impl Value {
    /// The declared shape this value has (SB-2).
    pub fn kind(&self) -> ValueKind {
        match self {
            Value::Scalar(s) => s.kind(),
            Value::List(_) => ValueKind::List,
            Value::Map(_) => ValueKind::Map,
        }
    }

    /// The scalar, if this value is one (SB-4).
    pub fn as_scalar(&self) -> Option<&Scalar> {
        match self {
            Value::Scalar(s) => Some(s),
            _ => None,
        }
    }

    /// A float value, unless `x` is NaN or an infinity (SB-4).
    pub fn num(x: f64) -> Option<Value> {
        Finite::new(x).map(|x| Value::Scalar(Scalar::Num(x)))
    }

    /// Refuses a map key that is not ASCII (SB-9a): a `Value` reaches the sealed
    /// Manifest through paths that pass no `from_json` — a Session `SetParameter`, an
    /// Action's `params` or `metadata` — and OV-15's canonicaliser refuses such a key,
    /// at cleanup step 7, after the Run had transmitted (RS-11).
    pub fn check_ascii_keys(&self, path: &str) -> Result<(), SpecError> {
        match self {
            Value::Map(items) => match items.keys().find(|k| !k.is_ascii()) {
                Some(bad) => Err(SpecError::KeyShape {
                    key: format!("{path}.{bad}"),
                    expected: "an ASCII key, which OV-15 canonicalises".into(),
                    found: "a non-ASCII key".into(),
                }),
                None => Ok(()),
            },
            _ => Ok(()),
        }
    }
}

impl From<i64> for Scalar {
    fn from(x: i64) -> Scalar {
        Scalar::Int(x)
    }
}

impl From<bool> for Scalar {
    fn from(x: bool) -> Scalar {
        Scalar::Bool(x)
    }
}

impl From<&str> for Scalar {
    fn from(x: &str) -> Scalar {
        Scalar::Str(x.to_owned())
    }
}

impl From<String> for Scalar {
    fn from(x: String) -> Scalar {
        Scalar::Str(x)
    }
}

impl<T: Into<Scalar>> From<T> for Value {
    fn from(x: T) -> Value {
        Value::Scalar(x.into())
    }
}

impl TryFrom<f64> for Scalar {
    type Error = ();
    fn try_from(x: f64) -> Result<Scalar, ()> {
        Finite::new(x).map(Scalar::Num).ok_or(())
    }
}

impl TryFrom<f64> for Value {
    type Error = ();
    fn try_from(x: f64) -> Result<Value, ()> {
        Value::num(x).ok_or(())
    }
}

/// Equality crosses `Int` and `Num` exactly: two scalars are one value iff they are
/// numerically equal, the `Equal` case of the one order [`cmp_int_num`] defines, so
/// `PartialEq`, SB-6's `Eq`, `Set`, `Range`, `Min`, `Max` and SC-2's "identical" all
/// decide by one relation and cannot disagree (finding D24, D50). `1` and `1.0` are
/// one value and share one canonical form and one hash under OV-15a.
///
/// Rule: SB-6, SC-2, OV-15, OV-15a.
impl PartialEq for Scalar {
    fn eq(&self, other: &Scalar) -> bool {
        self.partial_cmp(other) == Some(std::cmp::Ordering::Equal)
    }
}

/// Orders two scalars of one kind, and an `Int` against a `Num` exactly; `None` across
/// other kinds, which SB-6 reports as `KeyShape`.
///
/// Rule: SB-6.
impl PartialOrd for Scalar {
    fn partial_cmp(&self, other: &Scalar) -> Option<std::cmp::Ordering> {
        match (self, other) {
            (Scalar::Int(a), Scalar::Int(b)) => Some(a.cmp(b)),
            (Scalar::Num(a), Scalar::Num(b)) => a.partial_cmp(b),
            // Exactly, in 128-bit arithmetic, never through `as f64`: above 2^53 that
            // cast made two different numbers equal and equality non-transitive, so
            // `ContractRegistry::register` took a different definition for an
            // identical re-registration (SC-2) and a capability match was inexact.
            (Scalar::Int(a), Scalar::Num(b)) => Some(cmp_int_num(*a, *b)),
            (Scalar::Num(a), Scalar::Int(b)) => Some(cmp_int_num(*b, *a).reverse()),
            (Scalar::Str(a), Scalar::Str(b)) => Some(a.cmp(b)),
            (Scalar::Bool(a), Scalar::Bool(b)) => Some(a.cmp(b)),
            _ => None,
        }
    }
}

/// Orders an integer against a float without rounding either.
///
/// This is the **one** relation SB-6 names: `Eq`, `Set`, `Range`, `Min`, `Max` and
/// SC-2's "identical" all decide by it, and `PartialEq` agrees with it by
/// construction. Deciding equality by the canonical form instead was wrong in the
/// direction that matters most: above 2^53 a float's canonical text is the shortest
/// decimal that *names the `f64`*, not the number's exact decimal, so
/// `Int(1152921504606847000)` and `Num(2^60)` share one form and are two different
/// numbers — and `ContractRegistry::register` took one for an identical
/// re-registration of the other, which is SC-2's own named harm (finding D50).
///
/// Rule: SB-6, OV-15.
fn cmp_int_num(a: i64, b: Finite) -> std::cmp::Ordering {
    let b = b.get();
    let floor = b.floor();
    // `floor` is integral and finite; outside `i64` the comparison is decided by sign.
    if floor >= 9_223_372_036_854_775_808.0 {
        return std::cmp::Ordering::Less;
    }
    if floor < -9_223_372_036_854_775_808.0 {
        return std::cmp::Ordering::Greater;
    }
    let whole = floor as i64;
    a.cmp(&whole).then(if b == floor {
        std::cmp::Ordering::Equal
    } else {
        // `b` sits strictly between `whole` and `whole + 1`, so any integer equal to
        // `whole` is below it.
        std::cmp::Ordering::Less
    })
}

/// What a Spec requires of a key. Its value is a [`Scalar`]: a constraint over a
/// list or a map cannot be written, because the matcher would then need the
/// Vocabulary's semantics to compare them.
///
/// Rule: SB-5, decision B1.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Constraint {
    /// Exactly this value.
    Eq {
        /// The value.
        value: Scalar,
    },
    /// Within these bounds, either of which may be absent.
    Range {
        /// Lower bound, inclusive.
        min: Option<Scalar>,
        /// Upper bound, inclusive.
        max: Option<Scalar>,
    },
    /// Any of these values.
    Set {
        /// The permitted values.
        values: Vec<Scalar>,
    },
    /// At least this value.
    Min {
        /// The lower bound.
        value: Scalar,
    },
    /// At most this value.
    Max {
        /// The upper bound.
        value: Scalar,
    },
    /// The key is declared at all.
    Present {},
}

impl Constraint {
    /// Every scalar this constraint mentions (SB-5).
    pub fn values(&self) -> Vec<&Scalar> {
        match self {
            Constraint::Eq { value } | Constraint::Min { value } | Constraint::Max { value } => {
                vec![value]
            }
            Constraint::Range { min, max } => min.iter().chain(max.iter()).collect(),
            Constraint::Set { values } => values.iter().collect(),
            Constraint::Present {} => Vec::new(),
        }
    }
}

/// What a Provider declares it can do for a key (spec 05 `ProviderInstance`).
///
/// Rule: SB-6.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CapabilityValue {
    /// Exactly this value.
    One {
        /// The value.
        value: Scalar,
    },
    /// A continuous range, inclusive.
    Range {
        /// Lower bound.
        min: Scalar,
        /// Upper bound.
        max: Scalar,
    },
    /// A discrete set.
    AnyOf {
        /// The declared values.
        values: Vec<Scalar>,
    },
}

/// What a Vocabulary declares about one key (spec 05, MA-35).
///
/// Rule: SB-2, SB-7, SB-45.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
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
    /// The update class this key may be changed under during a Run; absent means it
    /// is not changeable during a Run. This is where a **Provider** parameter's
    /// class is declared: §27's own examples of runtime mutation — TX gain, antenna
    /// beam, MCS — and Vision §3's `sdr.rx.gain = 20` all target a Provider, whose
    /// parameters are Vocabulary keys and not `ComponentDescriptor.params`, so
    /// without this RS-17 has nothing to consult for them (SB-2, RS-17, MA-35).
    #[serde(default)]
    pub update_class: Option<crate::module_api::UpdateClass>,
}

/// What to do when a Provider coerces a requested value (SB-45, SB-46).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct Warning {
    /// Where it came from.
    pub source: Namespace,
    /// What it says; uninterpreted by the Kernel.
    pub message: String,
}

/// A constraint the bound instance could not satisfy (SB-38).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct SpecTime {
    /// The Spec resource whose stream clock the offset counts in.
    pub clock: Ident,
    /// Ticks from the Run's start.
    pub offset_ticks: i64,
}

/// The link that feeds a Sink: the port the samples leave from and the drop-class
/// policy and capacity of the link itself. One shape serves both a Spec `outputs[]`
/// entry and a Session profile's Sink binding, because an output *is* the
/// declaration of that link (SB-17, SB-22, SC-19, SC-21).
///
/// Rule: SB-17, SB-22.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SinkFeed {
    /// The port the samples leave from: a component's port, or a port a bound
    /// resource declares (SB-15, MA-10).
    pub port: PortRef,
    /// What happens at capacity. SC-19 forbids a default, and SC-21 restricts a
    /// link into a Sink to the drop class, so `Block` is refused here.
    pub policy: crate::stream::BackPressure,
    /// Queue depth in blocks; mandatory and at least 1 (SC-19).
    pub capacity: u32,
}

/// An artifact the Run must produce. A capture whose source has no placed Sink is
/// refused at `validate()`.
///
/// Rule: SB-17.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OutputReq {
    /// The artifact's name inside this document.
    pub id: Ident,
    /// The artifact kind; a Sink Module's namespace.
    pub kind: Namespace,
    /// The link that carries the samples to the bound Sink. The output is the
    /// declaration of that link, so SC-19's mandatory policy and capacity live
    /// here rather than in `graph.links` (SB-17).
    pub feed: SinkFeed,
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
#[serde(deny_unknown_fields)]
pub struct Requirements {
    /// One entry per Vocabulary the Spec's keys draw on (SB-11).
    #[serde(default)]
    pub vocabularies: Vec<VocabularyReq>,
}

/// One Vocabulary this Spec requires, by namespace and major version (SB-11).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VocabularyReq {
    /// The Vocabulary's namespace, which is also its key prefix (SB-2).
    pub id: Namespace,
    /// The major version required (SB-11).
    pub major: u32,
}

/// The Spec's component graph (SB-15).
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct ExperimentSpec {
    /// Mandatory positive integer; this build supports exactly `{1}` (SB-10, SB-47).
    pub version: u32,
    /// The Vocabulary majors the keys belong to (SB-11).
    #[serde(default)]
    pub requirements: Requirements,
    /// Per-resource requirements (SB-12).
    #[serde(default)]
    pub resources: BTreeMap<Ident, ResourceReq>,
    /// Artifacts the Run consumes that no schedule entry carries, such as the waveform a
    /// Reactor transmits: each is verified and stored like a scheduled waveform, so a
    /// Module can read it and name it in an Action (SB-20a, KC-9; Phase 5, KE-1).
    #[serde(default)]
    pub inputs: Vec<crate::manifest::ArtifactRef>,
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
    "inputs",
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
    &["placements", "placement", "executor", "memory_domain", "island",
];

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
    /// A Spec resource has no binding (SB-22d).
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
    /// Two names of SB-22a's one namespace collide — resource names, graph component
    /// names, output ids, Island executor names, Island fragment ids and the `authority`
    /// name — or a name is the reserved `sink` (SB-22a, D83, D96).
    DuplicateBindingName {
        /// The colliding name.
        name: Ident,
        /// Which two sets it is in.
        sets: String,
    },
    /// A binding names a Module that does not hold the role its slot requires
    /// (SB-22e, MA-1).
    WrongBindingRole {
        /// The bound name.
        name: Ident,
        /// The role its name requires.
        expected: String,
        /// The Module bound to it.
        module: String,
    },
    /// Two Spec resources bound to one node the Provider did not declare shareable
    /// (SB-34, MA-10).
    NodeAlreadyBound {
        /// The node both wanted.
        node: String,
        /// The resource that took it.
        first: Ident,
        /// The resource that then asked for it.
        second: Ident,
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
            SpecError::KeyShape { key, expected, found,
            } => {
                write!(f, "key {key:?} wanted {expected} and found {found}")
            }
            SpecError::UnboundResource { name } => write!(f, "resource {name} has no binding"),
            SpecError::DuplicateBindingName { name, sets } => {
                write!(
                    f,
                    "SB-22a: {name} collides as {sets}; resource, component, output, Island executor, Island fragment and `authority` names form one namespace"
                )
            }
            SpecError::WrongBindingRole { name, expected, module,
            } => {
                write!(f, "SB-22e: {name} needs a Module holding the {expected} role; {module} does not")
            }
            SpecError::NodeAlreadyBound { node, first, second,
            } => {
                write!(
                    f,
                    "SB-34: {second} wants node {node}, which {first} already binds and which \
                     the Provider does not declare shareable"
                )
            }
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

/// Refuses a non-ASCII object key anywhere in a document.
///
/// OV-15's canonicaliser refuses one, because RFC 8785 orders keys by their UTF-16
/// code units and this profile does not implement that ordering. Nothing refused it
/// at **ingestion**, so a Spec carrying `{"ρ": 1}` in an opaque section validated,
/// planned, armed and transmitted, and `Manifest::seal()` then failed at cleanup
/// step 7 — a Run that radiated and produced no Manifest, against RS-11. Refusing it
/// here is what keeps that impossible.
///
/// Rule: OV-15, OV-16, RS-11, SB-9.
pub(crate) fn check_ascii_keys(doc: &serde_json::Value) -> Result<(), SpecError> {
    match doc {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                if !k.is_ascii() {
                    return Err(SpecError::UnknownField {
                        path: format!("{k}: OV-15 canonicalises only ASCII object keys"),
                    });
                }
                check_ascii_keys(v)?;
            }
            Ok(())
        }
        serde_json::Value::Array(items) => items.iter().try_for_each(check_ascii_keys),
        _ => Ok(()),
    }
}

/// Checks a document's `version` against [`SUPPORTED_VERSIONS`] (SB-10, SB-47).
pub fn check_version(doc: &serde_json::Value) -> Result<u32, SpecError> {
    let found = doc
        .get("version")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| SpecError::UnknownField { path: "version".to_owned(),
            })?;
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
    let map = doc.as_object().ok_or_else(|| SpecError::UnknownField { path: "$".to_owned(),
    })?;
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
    /// the shapes, which the types hold (SB-4, SB-5).
    ///
    /// v3's `CONSTANTS` and `!COMPUTE(...)` are refused here — by SB-9 as unknown
    /// fields and by SB-4 as values that are not scalars. There is no evaluation
    /// pass; parametrisation is the client-side builder of Vision §9 (SB-14).
    pub fn from_json(doc: &serde_json::Value) -> Result<ExperimentSpec, SpecError> {
        check_version(doc)?;
        check_top_level(doc, SPEC_TOP_LEVEL)?;
        check_no_placement(doc)?;
        check_ascii_keys(doc)?;
        serde_json::from_value(doc.clone())
            .map_err(|e| SpecError::Structural { reason: format!("SB-9: {e}"),
            })
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
