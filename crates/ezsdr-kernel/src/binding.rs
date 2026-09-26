//! The BindingProfile, the generic matcher and the admission-check hook —
//! `03-spec-and-binding.md` SB-6…SB-8, SB-21…SB-36 (Vision §8, §31, §52).

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::contract::PortRef;
use crate::id::{MemoryDomainId, ResourceId};
use crate::module_api::{IslandDecl, ModuleRef, ProfileRef};
use crate::spec::{
    CapabilityValue, Coercion, Constraint, Ident, Key, KeyDecl, Namespace, RejectedConstraint,
    SpecError, Value, ValueKind, Warning, check_top_level, check_version,
};

/// Tests every candidate so a later kind mismatch is not hidden by an earlier hit (SB-6).
fn any_evaluating_all<'a>(
    mut values: impl Iterator<Item = &'a Value>,
    mut predicate: impl FnMut(&Value) -> Result<bool, SpecError>,
) -> Result<bool, SpecError> {
    values.try_fold(false, |found, value| Ok(predicate(value)? | found))
}

/// Which Provider serves a Spec resource, and how it is selected.
///
/// A `selector` is Provider content. Vision §8's simulation example writes
/// `instances: 2` under a Mock binding; that is selector content meaning one
/// Provider instance emulating two motherboards, exactly as the laboratory
/// example's `addrs: [a, b]` is one instance spanning two. It is not two Provider
/// instances, which SB-35 would then have to reconcile with the single-instance
/// rule.
///
/// Rule: SB-22, SB-23.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    /// The Module bound here: a Provider for a Spec resource, a Sink for an output
    /// id, an Executor for an Island's `executor` name, and the Authority for the name
    /// `authority` gives (SB-22b). Named `module` rather than `provider` because every
    /// role is bound through this one map; the Vision's §8 examples wrote `provider`
    /// until Step 5 corrected them (03 §9). It names the exact `{id, version}`, as
    /// `LinkPlacement` does, and every bound instance declares the same `ModuleRef`
    /// (a Provider's `instance()`, a Sink's `descriptor()`, an Executor's or the
    /// Authority's supplied descriptor), which admission compares, so one profile hash
    /// cannot run two Module versions (SB-22f, D78, D82, D98).
    pub module: ModuleRef,
    /// Namespaced content the Provider interprets (SB-23).
    #[serde(default)]
    pub selector: BTreeMap<Ident, Value>,
    /// The declared profile, when it has one (SB-22, Vision §33).
    pub profile: Option<ProfileRef>,
    /// For a **Sink** binding on a Session profile: the port it records and the
    /// drop-class policy and capacity of the link that feeds it. SB-22c makes it the
    /// implicit Spec's output. A Spec Run's `outputs[]` already declare their feeds,
    /// so a binding that carries one there is refused (SB-22g).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feed: Option<crate::spec::SinkFeed>,
}

/// Where a component runs (SB-25).
// SB-25a, which put the Sink's Module in this struct, is withdrawn: an output is bound
// rather than placed (D17, D18). Kept out of the doc comment because that text is the
// schema's `description` and OV-10 freezes it.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComponentPlacement {
    /// Which Island (SB-25).
    pub island: Ident,
    /// Which memory domain (MA-39).
    pub memory_domain: MemoryDomainId,
}

/// Which versioned Link Module carries one data link, named by its two ends: a
/// graph link's `from` and `to`, or an output's feed port and `{output id, "in"}`.
/// Naming the ends rather than a position means reordering `graph.links` cannot
/// rebind a Link Module (SB-25, D76).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LinkPlacement {
    /// The Link Module (MA-27, MA-28).
    pub link: ModuleRef,
    /// The producer end (SB-25).
    pub from: PortRef,
    /// The consumer end (SB-25).
    pub to: PortRef,
}

/// The Island declarations, the component assignment and the Link Module for each
/// data link — graph link or output feed. Every component of the Spec's graph
/// appears in exactly one Island.
///
/// Rule: SB-25.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Placements {
    /// The Islands (MA-38).
    #[serde(default)]
    pub islands: Vec<IslandDecl>,
    /// Component name to placement (SB-25).
    #[serde(default)]
    pub components: BTreeMap<Ident, ComponentPlacement>,
    /// One placement per data link — every graph link and every output feed —
    /// each naming the link by its endpoints (SB-25, D76).
    #[serde(default)]
    pub links: Vec<LinkPlacement>,
}

/// How this requirement is met, at this site, for this run.
///
/// `environment` is the only place a channel model, a fault schedule, a virtual-time
/// setting, a clock distribution or a site limit may appear (SB-26).
///
/// Rule: SB-21…SB-27.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BindingProfile {
    /// Mandatory positive integer; Phase 1 supports exactly `{1}` (SB-21, SB-47).
    pub version: u32,
    /// Binding name to Binding: exactly one entry per role slot the two documents
    /// name, and no other (SB-22b, SB-22d, D89).
    #[serde(default)]
    pub bindings: BTreeMap<Ident, Binding>,
    /// Which binding keeps time: a resource the Authority rides on, or a binding of
    /// its own. Mandatory; nothing is inferred (SB-24, D97).
    pub authority: Ident,
    /// Islands, components and links (SB-25).
    #[serde(default)]
    pub placements: Placements,
    /// Namespaced opaque content, recorded verbatim in the Manifest (SB-26, SB-27).
    #[serde(default)]
    pub environment: BTreeMap<Namespace, serde_json::Value>,
}

/// Top-level fields a [`BindingProfile`] may carry (SB-21).
pub const BINDING_TOP_LEVEL: &[&str] = &[
    "version",
    "bindings",
    "authority",
    "placements",
    "environment",
];

/// The four `environment` sections the Kernel reads by name; every other section is
/// opaque to it (SB-26).
pub const KERNEL_SECTIONS: &[&str] = &[
    "ezsdr.time",
    "ezsdr.rf_path",
    "ezsdr.capture",
    "ezsdr.arm_order",
];

impl BindingProfile {
    /// Parses and validates the envelope: the version (SB-47) and the closed
    /// top-level set (SB-21). Unlike a Spec, a profile is where placement and
    /// environment belong, so no field scan runs (SB-13).
    pub fn from_json(doc: &serde_json::Value) -> Result<BindingProfile, SpecError> {
        check_version(doc)?;
        check_top_level(doc, BINDING_TOP_LEVEL)?;
        crate::spec::check_ascii_keys(doc)?;
        serde_json::from_value(doc.clone()).map_err(|e| SpecError::Structural {
            reason: format!("SB-21: {e}"),
        })
    }

    /// One `environment` section, or `None`. The Kernel reads the four of
    /// [`KERNEL_SECTIONS`] and nothing else (SB-26).
    pub fn section(&self, ns: &str) -> Option<&serde_json::Value> {
        // SB-26 fixes the sections the Kernel reads, and this is the only reader, so
        // the list is enforced here rather than being a constant nothing consults: a
        // Kernel read of an unlisted section is a Kernel that learned a Vocabulary
        // word (OV-21).
        if !KERNEL_SECTIONS.contains(&ns) {
            return None;
        }
        Namespace::parse(ns)
            .ok()
            .and_then(|n| self.environment.get(&n))
    }
}

/// `ezsdr.time.start_lead_ns`: the lead between the end of `arm` and the Run's start
/// instant, in nanoseconds; absent means 0. The one reader of the field, used by
/// `plan()` and by the coordinator (MA-41, KC-15, KA-8).
pub fn start_lead_ns(
    environment: &BTreeMap<Namespace, serde_json::Value>,
) -> Result<u64, SpecError> {
    let Some(section) = Namespace::parse("ezsdr.time")
        .ok()
        .and_then(|n| environment.get(&n))
    else {
        return Ok(0);
    };
    match section.get("start_lead_ns") {
        None => Ok(0),
        Some(v) => v
            .as_u64()
            .filter(|n| *n <= 1u64 << 62)
            .ok_or_else(|| SpecError::Structural {
                reason: format!(
                    "MA-41: ezsdr.time.start_lead_ns must be an integer in 0..=2^62, not {v}"
                ),
            }),
    }
}

// ---------------------------------------------------------------- the matcher

/// Whether a constraint is satisfied by a declared capability.
///
/// `Eq(v)` by `One(v)`, by a `Range` containing `v`, or by an `AnyOf` containing
/// `v`. `Range{min,max}` by a `One` inside it, by an overlapping `Range`, or by an
/// `AnyOf` with a member inside it. `Set(s)` by any capability value whose possible
/// values intersect `s`. `Min` and `Max` by the corresponding bound. `Present` by
/// the key being declared at all.
///
/// The matcher is generic: no Kernel item names a radio, a channel, a sample rate,
/// a gain or a frequency (SB-8).
///
/// Rule: SB-6.
pub fn satisfies(c: &Constraint, cap: &CapabilityValue) -> Result<bool, SpecError> {
    use std::cmp::Ordering::*;
    let cmp = |a: &Value, b: &Value| -> Result<std::cmp::Ordering, SpecError> {
        a.partial_cmp_scalar(b).ok_or_else(|| SpecError::KeyShape {
            key: String::new(),
            expected: format!("{:?}", a.kind()),
            found: format!("{:?}", b.kind()),
        })
    };
    // `Eq` and set membership ask whether two values are **one value**, and SB-6 knows
    // one relation for that: exact numeric comparison, the `Equal` case of the ordering
    // below. `PartialEq` decides the same way, so the two never disagree. Deciding it
    // by the canonical form instead equated two *different* numbers above 2^53, because
    // a float's canonical text is the shortest decimal that names the `f64` and not the
    // number's exact decimal (finding D50). The kind check still runs through `cmp`,
    // because a capability declared in the wrong kind is malformed rather than unequal.
    let same = |a: &Value, b: &Value| -> Result<bool, SpecError> { Ok(cmp(a, b)? == Equal) };
    let within = |v: &Value, lo: Option<&Value>, hi: Option<&Value>| -> Result<bool, SpecError> {
        if let Some(lo) = lo {
            if cmp(v, lo)? == Less {
                return Ok(false);
            }
        }
        if let Some(hi) = hi {
            if cmp(v, hi)? == Greater {
                return Ok(false);
            }
        }
        Ok(true)
    };
    Ok(match (c, cap) {
        (Constraint::Present {}, _) => true,

        (Constraint::Eq { value: v }, CapabilityValue::One { value: x }) => same(v, x)?,
        (Constraint::Eq { value: v }, CapabilityValue::Range { min, max }) => {
            within(v, Some(min), Some(max))?
        }
        (Constraint::Eq { value: v }, CapabilityValue::AnyOf { values: xs }) => {
            any_evaluating_all(xs.iter(), |x| same(v, x))?
        }

        (Constraint::Range { min, max }, CapabilityValue::One { value: x }) => {
            within(x, min.as_ref(), max.as_ref())?
        }
        (
            Constraint::Range { min, max },
            CapabilityValue::Range {
                min: cmin,
                max: cmax,
            },
        ) => {
            // Propagated, not swallowed: a capability declared in the wrong kind is
            // malformed, not merely unsatisfiable (SB-6).
            let above = match min {
                Some(lo) => cmp(cmax, lo)? != Less,
                None => true,
            };
            let below = match max {
                Some(hi) => cmp(cmin, hi)? != Greater,
                None => true,
            };
            above && below
        }
        (Constraint::Range { min, max }, CapabilityValue::AnyOf { values: xs }) => {
            any_evaluating_all(xs.iter(), |x| within(x, min.as_ref(), max.as_ref()))?
        }

        (Constraint::Set { values: s }, CapabilityValue::One { value: x }) => {
            any_evaluating_all(s.iter(), |v| same(v, x))?
        }
        (Constraint::Set { values: s }, CapabilityValue::Range { min, max }) => {
            any_evaluating_all(s.iter(), |v| within(v, Some(min), Some(max)))?
        }
        (Constraint::Set { values: s }, CapabilityValue::AnyOf { values: xs }) => {
            any_evaluating_all(s.iter(), |v| any_evaluating_all(xs.iter(), |x| same(v, x)))?
        }

        // Min and Max are satisfied by the corresponding bound of the capability.
        (Constraint::Min { value: v }, CapabilityValue::One { value: x }) => cmp(x, v)? != Less,
        (Constraint::Min { value: v }, CapabilityValue::Range { max, .. }) => cmp(max, v)? != Less,
        (Constraint::Min { value: v }, CapabilityValue::AnyOf { values: xs }) => {
            any_evaluating_all(xs.iter(), |x| Ok(cmp(x, v)? != Less))?
        }
        (Constraint::Max { value: v }, CapabilityValue::One { value: x }) => cmp(x, v)? != Greater,
        (Constraint::Max { value: v }, CapabilityValue::Range { min, .. }) => {
            cmp(min, v)? != Greater
        }
        (Constraint::Max { value: v }, CapabilityValue::AnyOf { values: xs }) => {
            any_evaluating_all(xs.iter(), |x| Ok(cmp(x, v)? != Greater))?
        }
    })
}

/// Refuses a constraint whose scalars are not of the `KeyDecl`'s kind (SB-6).
pub fn check_constraint_kind(decl: &KeyDecl, c: &Constraint) -> Result<(), SpecError> {
    for v in c.values() {
        let ok = matches!(
            (decl.kind, v.kind()),
            (ValueKind::Int, ValueKind::Int)
                | (ValueKind::Num, ValueKind::Num | ValueKind::Int)
                | (ValueKind::Str, ValueKind::Str)
                | (ValueKind::Bool, ValueKind::Bool)
                | (ValueKind::List, ValueKind::List)
                | (ValueKind::Map, ValueKind::Map)
        );
        if !ok {
            return Err(SpecError::KeyShape {
                key: decl.key.to_string(),
                expected: format!("{:?}", decl.kind),
                found: format!("{:?}", v.kind()),
            });
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- admission checks

/// When an admission check runs (SB-29, SB-30).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
pub enum CheckStage {
    /// Against the requested configuration (SB-30).
    Validate,
    /// Against the applied configuration each Provider returned. Not redundant: a
    /// coercion can move an applied value outside a limit that the requested value
    /// respected (SB-30, Vision §52).
    Prepare,
    /// Against every Session Action before dispatch, with the Action's key and
    /// value as `proposed` (SB-30, RS-16).
    Runtime,
}

/// What an admission check refused (SB-29).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Violation {
    /// Which check (SB-29).
    pub check: Namespace,
    /// Which key, when the violation names one.
    pub key: Option<Key>,
    /// What was asked.
    pub requested: Option<Value>,
    /// Why, uninterpreted by the Kernel.
    pub reason: String,
}

/// A Vocabulary's own enforcement, registered against an `environment` section.
///
/// This is how Vision §52's demand that the Kernel enforce the RF safety envelope
/// is reconciled with audit Finding 17's ruling that the envelope's schema belongs
/// to the Radio Model. The Kernel guarantees that the check runs; the Vocabulary
/// owns what it means. No radio word enters the Kernel, and no Vocabulary can be
/// bypassed.
///
/// A registered check must be pure and must not require hardware, so that
/// `validate()` is a true dry run.
///
/// Rule: SB-29, SB-31, decision B5.
pub trait AdmissionCheck: Send + Sync {
    /// The `environment` section this check reads (SB-29).
    fn section(&self) -> &Namespace;
    /// The stages at which it runs (SB-29).
    fn stages(&self) -> &[CheckStage];
    /// Evaluates the effective configuration overlaid with `proposed`. `proposed`
    /// is empty at `validate` and `prepare`; at the runtime stage it carries the
    /// Action's key and value, because "before dispatch" means the value has not
    /// been applied and a check given only the current effective configuration
    /// could not see what it is being asked to admit (SB-30).
    fn check(
        &self,
        section: &serde_json::Value,
        effective: &BTreeMap<Ident, BTreeMap<Key, Value>>,
        proposed: &BTreeMap<Ident, BTreeMap<Key, Value>>,
        stage: CheckStage,
    ) -> Vec<Violation>;
}

/// The checks a runtime registered. The Kernel runs every registered check whose
/// section is present, at the three points of SB-30; a non-empty violation list
/// fails the stage, and nothing transmits until every check at every applicable
/// stage has passed (Vision invariant 42).
///
/// Rule: SB-29, SB-30.
#[derive(Default, Clone)]
pub struct AdmissionCheckRegistry {
    checks: Vec<Arc<dyn AdmissionCheck>>,
}

impl AdmissionCheckRegistry {
    /// An empty registry (SB-29).
    pub fn new() -> AdmissionCheckRegistry {
        AdmissionCheckRegistry::default()
    }

    /// Registers a check (SB-29).
    pub fn register(&mut self, check: Arc<dyn AdmissionCheck>) {
        self.checks.push(check);
    }

    /// The sections the registered checks read (SB-29; KC-37a).
    pub(crate) fn sections(&self) -> impl Iterator<Item = &Namespace> {
        self.checks.iter().map(|check| check.section())
    }

    /// Runs every check whose section is present in `environment` and whose stages
    /// include `stage`. A section with no registered check is informational and is
    /// still recorded verbatim (SB-30, SB-31).
    pub fn run(
        &self,
        environment: &BTreeMap<Namespace, serde_json::Value>,
        effective: &BTreeMap<Ident, BTreeMap<Key, Value>>,
        proposed: &BTreeMap<Ident, BTreeMap<Key, Value>>,
        stage: CheckStage,
    ) -> Vec<Violation> {
        let mut out = Vec::new();
        for check in &self.checks {
            if !check.stages().contains(&stage) {
                continue;
            }
            if let Some(section) = environment.get(check.section()) {
                out.extend(check.check(section, effective, proposed, stage));
            }
        }
        out
    }
}

/// A coercion the Kernel previewed by calling one Provider's `coerce`, together with
/// the Spec resource whose request produced it.
///
/// A `Coercion` names a key and two values and nothing else, because a Provider
/// answers about the request it was handed. The Kernel's own record has to say more:
/// two resources may constrain one key on two different devices, and one of them
/// coercing says nothing about the other. Keyed on the key alone, `prepare`'s SB-44
/// check charged one resource's coercion to every resource naming that key and
/// refused a value `coerce` was never asked about — so the resource name is part of
/// the record, as it already is in [`RejectedConstraint`].
///
/// Rule: SB-7, SB-38, SB-44.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreviewedCoercion {
    /// The Spec resource whose `requires` map was coerced (SB-44).
    pub resource: Ident,
    /// What that resource's Provider said it would change (SB-7).
    pub coercion: Coercion,
}

/// What `validate(spec, binding, registry)` returns. It touches no hardware.
///
/// Rule: SB-38.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AdmissionResult {
    /// Spec resource name — and `needs` name — to the resolved `ResourceId` (SB-36).
    #[serde(default)]
    pub matched: BTreeMap<Ident, ResourceId>,
    /// Constraints the bound instance could not satisfy (SB-38).
    #[serde(default)]
    pub rejected: Vec<RejectedConstraint>,
    /// What the registered checks refused (SB-30).
    #[serde(default)]
    pub violations: Vec<Violation>,
    /// What the Providers said they would change (SB-7, SB-44).
    #[serde(default)]
    pub coercions_preview: Vec<PreviewedCoercion>,
    /// Non-fatal notes (SB-38).
    #[serde(default)]
    pub warnings: Vec<Warning>,
}

impl AdmissionResult {
    /// True when nothing was rejected and no check was violated (SB-38).
    pub fn is_admitted(&self) -> bool {
        self.rejected.is_empty() && self.violations.is_empty()
    }

    /// Turns a result that was not admitted into the error naming why: a rejected
    /// constraint is `NoSingleInstance`, because the Core never assembles a
    /// capability across instances (SB-35), and a check refusal is `Violation`.
    ///
    /// Rule: SB-30, SB-35, SB-38.
    pub fn into_result(self) -> Result<AdmissionResult, SpecError> {
        if let Some(r) = self.rejected.first() {
            return Err(SpecError::NoSingleInstance {
                name: r.resource.clone(),
                constraint: format!("{:?} on {}", r.constraint, r.key),
            });
        }
        if let Some(v) = self.violations.first() {
            return Err(SpecError::Violation(v.clone()));
        }
        Ok(self)
    }
}
