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

/// A scalar, or a list or map of scalars nested at most one level.
///
/// A Spec is data, not a document tree: one level is what a structured parameter
/// such as a capture request needs, and more invites the schema-inside-a-schema
/// that Vision §9 rejects.
///
/// Rule: SB-4.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
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
            Value::Num(x) if !x.is_finite() => Err(SpecError::KeyShape { key: path.to_owned(), expected: "finite number".into(), found: "non-finite".into(),
            }),
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
                // OV-15's canonicaliser refuses a non-ASCII object key, and a `Value`
                // reaches the sealed Manifest through paths that pass no `from_json`:
                // a Session `SetParameter`, an Action's `params` or `metadata`. Left
                // to hashing time it failed at cleanup step 7, after the Run had
                // transmitted — a Run with no Manifest, against RS-11.
                if let Some(bad) = items.keys().find(|k| !k.as_str().is_ascii()) {
                    return Err(SpecError::KeyShape {
                        key: format!("{path}.{bad}"),
                        expected: "an ASCII key, which OV-15 canonicalises".into(),
                        found: "a non-ASCII key".into(),
                    });
                }
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
            // Exactly, in 128-bit arithmetic: `as f64` is the cast SC-2 removed from
            // `Scalar` for the same reason, and SB-6's `Eq`, `Range`, `Min` and `Max`
            // all pass through here, so a capability match above 2^53 was inexact.
            (Value::Int(a), Value::Num(b)) => cmp_int_num(*a, *b),
            (Value::Num(a), Value::Int(b)) => cmp_int_num(*b, *a).map(|o| o.reverse()),
            (Value::Str(a), Value::Str(b)) => Some(a.cmp(b)),
            (Value::Bool(a), Value::Bool(b)) => Some(a.cmp(b)),
            _ => None,
        }
    }
}

/// Equality crosses `Int` and `Num` exactly, as `Scalar`'s does (SC-2): two values are
/// equal iff they share one canonical form under OV-15, which holds while the float is
/// integral and its magnitude is at most 2^53. OV-15 already gives this document family
/// one notion of "same value" — one canonical form, one hash — so a second notion here
/// would be a defect and not a choice, and SB-6's comparison would disagree with the
/// hash for exactly the values the derive got wrong.
///
/// Rule: SB-4, SB-6, OV-15, OV-15a.
impl PartialEq for Value {
    fn eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Num(a), Value::Num(b)) => a == b,
            (Value::Int(a), Value::Num(b)) | (Value::Num(b), Value::Int(a)) => {
                cmp_int_num(*a, *b) == Some(std::cmp::Ordering::Equal)
            }
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::List(a), Value::List(b)) => a == b,
            (Value::Map(a), Value::Map(b)) => a == b,
            _ => false,
        }
    }
}

/// Orders an integer against a float without rounding either. A non-finite float is
/// unordered, which is what `None` means to SB-6.
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
pub(crate) fn cmp_int_num(a: i64, b: f64) -> Option<std::cmp::Ordering> {
    if !b.is_finite() {
        return None;
    }
    let floor = b.floor();
    // `floor` is integral and finite; outside `i64` the comparison is decided by sign.
    if floor >= 9_223_372_036_854_775_808.0 {
        return Some(std::cmp::Ordering::Less);
    }
    if floor < -9_223_372_036_854_775_808.0 {
        return Some(std::cmp::Ordering::Greater);
    }
    let whole = floor as i64;
    Some(a.cmp(&whole).then(if b == floor {
        std::cmp::Ordering::Equal
    } else {
        // `b` sits strictly between `whole` and `whole + 1`, so any integer equal to
        // `whole` is below it.
        std::cmp::Ordering::Less
    }))
}

/// What a Spec requires of a key. Its value is a scalar: a constraint over a list
/// or a map is refused, because the matcher would then need the Vocabulary's
/// semantics to compare them.
///
/// Rule: SB-5, decision B1.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
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
    Present {},
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
    /// the shapes.
    ///
    /// v3's `CONSTANTS` and `!COMPUTE(...)` are refused here — by SB-9 as unknown
    /// fields and by SB-4 as values that are not scalars. There is no evaluation
    /// pass; parametrisation is the client-side builder of Vision §9 (SB-14).
    pub fn from_json(doc: &serde_json::Value) -> Result<ExperimentSpec, SpecError> {
        check_version(doc)?;
        check_top_level(doc, SPEC_TOP_LEVEL)?;
        check_no_placement(doc)?;
        check_ascii_keys(doc)?;
        let spec: ExperimentSpec = serde_json::from_value(doc.clone())
            .map_err(|e| SpecError::Structural { reason: format!("SB-9: {e}"),
            })?;
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
