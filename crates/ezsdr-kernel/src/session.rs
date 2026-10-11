//! Sessions, the action log and `admit()` —
//! `04-run-and-session.md` RS-12…RS-20 (Vision §3).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::binding::{AdmissionCheckRegistry, BindingProfile, CheckStage, Violation};
use crate::event::{Action, ActionId, Target};
use crate::hash::ContentHash;
use crate::module_api::{CompileRule, ModuleRegistry, Role};
use crate::plan::{apply_coercion, coercion_policy};
use crate::run::RunError;
use crate::spec::{
    Coercion, CoercionPolicy, ExperimentSpec, Ident, Key, Namespace, ResourceReq, SpecError, Value,
    Warning,
};
use crate::time::{AbsoluteDeadline, TimePoint};

/// The Session action envelope: a closed set, and **not** the Kernel Action set.
///
/// The Kernel's Actions are what a Reactor emits into the real-time path, while a
/// lease operation or a recorder request is a control-path act with no real-time
/// meaning. Each Session Action compiles to zero or more Kernel Actions, or to a
/// Lease or Run operation.
///
/// Only the lifecycle verbs are the Kernel's; a domain verb is
/// a `vocabulary` Session action, and the Vocabulary that registers the verb
/// declares how it compiles. The Vision's §3 log sketch named `StartRepeat` and
/// `Capture` directly before Step 5; here they are `radio.start_repeat` and `sink.capture`,
/// because a Kernel that enumerated them would need a new variant for the first
/// peripheral sweep or calibration verb.
///
/// Rule: RS-13, RS-13a, decision R3.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionAction {
    /// Generic over a namespaced key and value (RS-13a).
    SetParameter {
        /// Whose parameter.
        target: Target,
        /// Which parameter (SB-2).
        key: Key,
        /// Its new value (SB-4).
        value: Value,
    },
    /// A domain verb, whose compilation its Vocabulary declares (RS-13a, RS-14).
    Vocabulary {
        /// The Vocabulary that owns the verb.
        ns: Namespace,
        /// The verb.
        verb: Ident,
        /// What the verb addresses: a resource, or an output for a recorder's verb.
        target: Target,
        /// When. A Kernel field, not a param: SB-4 caps a `Value` at one level of
        /// nesting and a `TimePoint` is two, so a time could not travel inside
        /// `params` at all (RS-19).
        at: Option<TimePoint>,
        /// The verb's parameters (SB-4).
        #[serde(default)]
        params: BTreeMap<Key, Value>,
    },
    /// With a target it stops that resource; without one it stops the Run (RS-50).
    Stop {
        /// The resource, or the Run when absent.
        target: Option<Target>,
    },
    /// Give up the Lease (RS-14).
    Release {},
    /// Reclaim a Detached Lease. Adoption by run id alone would let any client
    /// seize a live transmitter (RS-24).
    Adopt {
        /// The token issued at grant.
        token: String,
    },
    /// Extend a renewable Lease (RS-24).
    Renew {},
    /// Create a child Run (RS-14, RS-25).
    RunChild {
        /// The child's Spec, by hash (RS-45).
        spec_hash: ContentHash,
        /// The child's own profile, by hash: a Session's cannot serve a Spec Run
        /// (RS-25a, D103).
        binding_hash: ContentHash,
    },
}

/// What `admit()` decided about one Session Action (RS-15, RS-16).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    /// It passed every check and was dispatched (RS-16); one the end discarded before
    /// its Module received it is also in the termination's `undelivered`.
    Admitted {
        /// What was coerced, including RS-19's "as soon as possible" (SB-44).
        #[serde(default)]
        coercions: Vec<Coercion>,
        /// Non-fatal notes (SB-46).
        #[serde(default)]
        warnings: Vec<Warning>,
        /// The Kernel Actions it compiled to (RS-14).
        #[serde(default)]
        dispatched: Vec<ActionId>,
    },
    /// It was refused and never reached the real-time path (RS-16).
    Rejected {
        /// Why (SB-29).
        violations: Vec<Violation>,
    },
}

/// One entry of the action log (RS-15).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LogEntry {
    /// Dense: a rejected Action occupies one too, because a log with holes cannot
    /// be replayed or audited (RS-15).
    pub seq: u32,
    /// The Run-time instant at which it arrived (RS-15).
    pub time: TimePoint,
    /// What was asked (RS-13).
    pub action: SessionAction,
    /// What happened (RS-15).
    pub outcome: Outcome,
}

impl SessionAction {
    /// OV-15's ASCII keys, over every `Value` this Action carries. A `Value` reaches
    /// the sealed Manifest through the action log and through an Action's `params`,
    /// neither of which passes a `from_json`, so a document that fails this is not an
    /// Action at all (SB-9a, RS-11, RS-15).
    pub fn check_values(&self) -> Result<(), SpecError> {
        let carried: Vec<(&Key, &Value)> = match self {
            SessionAction::SetParameter { key, value, .. } => vec![(key, value)],
            SessionAction::Vocabulary { params, .. } => params.iter().collect(),
            _ => Vec::new(),
        };
        carried
            .into_iter()
            .try_for_each(|(k, v)| v.check_ascii_keys(k.as_str()))
    }
}

/// Every Action, admitted or not, in order (RS-15).
#[derive(Clone, PartialEq, Debug, Default)]
pub struct SessionLog {
    entries: Vec<LogEntry>,
}

impl SessionLog {
    /// An empty log (RS-15).
    pub fn new() -> SessionLog {
        SessionLog::default()
    }

    /// Appends an entry with the next dense sequence number, refusing an Action that
    /// is not a well-formed document (RS-15, SB-4, SB-9a).
    ///
    /// RS-15 logs every Action, admitted or not: a **rejected** Action still takes a
    /// number. One whose value the canonicaliser cannot hash was never an Action —
    /// and a log it cannot hash breaks RS-11 for the whole Run, which is worse than
    /// refusing one entry. The check lives here rather than on the compile path
    /// because this is the door: `compile` returns a `Compiled` and the wiring that
    /// would log it is Phase 2's.
    pub fn append(
        &mut self,
        time: TimePoint,
        action: SessionAction,
        outcome: Outcome,
    ) -> Result<u32, SpecError> {
        self.check_entry(&action)?;
        let seq = self.entries.len() as u32;
        self.entries.push(LogEntry {
            seq,
            time,
            action,
            outcome,
        });
        Ok(seq)
    }

    /// What `append` refuses, checked without appending: an Action that is not a
    /// well-formed document. The coordinator calls it before anything is compiled, so
    /// that a refused Action takes no sequence number (RS-15, KC-28).
    pub fn check_entry(&self, action: &SessionAction) -> Result<(), SpecError> {
        action.check_values()
    }

    /// Every entry, in order (RS-15).
    pub fn entries(&self) -> &[LogEntry] {
        &self.entries
    }

    /// The entries RS-20 re-applies: those that were admitted (RS-20).
    pub fn admitted(&self) -> impl Iterator<Item = &LogEntry> {
        self.entries
            .iter()
            .filter(|e| matches!(e.outcome, Outcome::Admitted { .. }))
    }

    /// Replaying a Session means re-applying its admitted entries against the same
    /// BindingProfile. Re-applying against a profile with a different hash reports
    /// `ReplayDivergence` rather than proceeding (RS-20, Vision §3).
    pub fn check_replay_target( // OV-23a: RS-20's session replay, not §6's Replay block
        &self,
        recorded: &ContentHash,
        target: &ContentHash,
    ) -> Result<(), RunError> {
        if recorded == target {
            Ok(())
        } else {
            Err(RunError::ReplayDivergence {
                field: "binding.hash".to_owned(),
            })
        }
    }
}

// ---------------------------------------------------------------- admission

/// What `admit()` returns when it passes (RS-17).
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Admitted {
    /// What was coerced (SB-44).
    pub coercions: Vec<Coercion>,
    /// Non-fatal notes (SB-46).
    pub warnings: Vec<Warning>,
}

/// The admission stages' shared check set: the registered admission checks of
/// SB-30's points and the coercion policy of SB-45, in that order.
///
/// `validate()` calls it with a Spec's requested configuration, `prepare()` with the
/// applied one, and admission with one Action's key and value. The parameter's
/// declared update class is **not** here: it is `ExperimentSpec::declared_class`, and
/// the runtime refusal that reads it is KC-24's step 5, which runs before this check
/// set (pre-freeze item 3).
///
/// Rule: RS-16, RS-17.
pub struct Admitter<'a> {
    /// The registered checks (SB-29).
    pub checks: &'a AdmissionCheckRegistry,
    /// The profile's `environment`, whose sections the checks read (SB-26).
    pub environment: &'a BTreeMap<Namespace, serde_json::Value>,
    /// The Spec's `policies.coercion` overrides (SB-19).
    pub spec_coercion: &'a BTreeMap<Key, CoercionPolicy>,
    /// The registry, for each key's `KeyDecl` default (SB-45, MA-34).
    pub registry: &'a ModuleRegistry,
    /// Whether this is a Session, which warns rather than taking the Vocabulary's
    /// default (SB-45, re-review R16).
    pub is_session: bool,
}

impl Admitter<'_> {
    /// Runs the checks in RS-17's order. A non-empty violation list fails the
    /// stage, and nothing transmits until every check at every applicable stage has
    /// passed (SB-30, Vision invariant 42).
    ///
    /// Rule: RS-16, RS-17, SB-30, SB-45, SB-46.
    pub fn admit(
        &self,
        effective: &BTreeMap<Ident, BTreeMap<Key, Value>>,
        proposed: &BTreeMap<Ident, BTreeMap<Key, Value>>,
        coercions: &[Coercion],
        stage: CheckStage,
    ) -> Result<Admitted, Vec<Violation>> {
        // 1. the registered admission checks (SB-30)
        let violations = self
            .checks
            .run(self.environment, effective, proposed, stage);
        if !violations.is_empty() {
            return Err(violations);
        }
        // 2. the coercion policy (SB-45, SB-46)
        let mut out = Admitted {
            coercions: coercions.to_vec(),
            warnings: Vec::new(),
        };
        for c in coercions {
            let policy = coercion_policy(
                self.spec_coercion.get(&c.key).copied(),
                self.is_session,
                self.registry.key_decl(&c.key),
            );
            match apply_coercion(policy, c) {
                Ok(None) => {}
                Ok(Some(w)) => out.warnings.push(w),
                Err(SpecError::CoercionRejected(c)) => {
                    return Err(vec![Violation {
                        check: Namespace::parse("ezsdr.coercion").expect("a valid literal"),
                        key: Some(c.key.clone()),
                        requested: Some(c.requested.clone()),
                        reason: format!("SB-46: coercion to {:?} rejected", c.applied),
                    }]);
                }
                Err(e) => {
                    return Err(vec![Violation {
                        check: Namespace::parse("ezsdr.coercion").expect("a valid literal"),
                        key: Some(c.key.clone()),
                        requested: Some(c.requested.clone()),
                        reason: e.to_string(),
                    }]);
                }
            }
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------- compilation

/// What one Session Action compiled to (RS-14). Crate-internal: a caller outside
/// the Kernel drives Actions through a Run, never through the compiler
/// (pre-freeze item 3).
#[derive(Clone, PartialEq, Debug, Default)]
pub(crate) struct Compiled {
    /// What the Action compiled to (RS-14): finished Actions and class-pending
    /// parameter changes, the one `event::PendingAction` admission takes.
    pub(crate) items: Vec<crate::event::PendingAction>,
    /// The Lease or Run operation it is instead (RS-14).
    pub(crate) control: Option<ControlOp>,
    /// RS-19's record of an "as soon as possible" that was resolved (RS-19).
    pub(crate) coercions: Vec<Coercion>,
}

/// A Session Action that is a Lease or Run operation rather than a Kernel Action
/// (RS-13, RS-14).
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum ControlOp {
    /// Stop the Run (RS-50).
    StopRun,
    /// Give up the Lease (RS-14).
    Release,
    /// Reclaim a Detached Lease (RS-24).
    Adopt {
        /// The token offered.
        token: String,
    },
    /// Extend a renewable Lease (RS-24).
    Renew,
    /// Create a child Run (RS-25, RS-25a).
    RunChild {
        /// The child's Spec, by hash.
        spec_hash: ContentHash,
        /// The child's profile, by hash (RS-25a).
        binding_hash: ContentHash,
    },
}

/// Compiles one Session Action.
///
/// The Kernel's own compilations are: `SetParameter` to a `PendingUpdate` for the
/// key — a parameter change states its class at admission, where the declaration
/// is read (KC-24 step 5); `Stop { target }` to a Kernel `Stop` addressed to that
/// resource and `Stop {}` to the Run's own stop; `Release`, `Adopt` and `Renew` to
/// Lease operations; `RunChild` to the creation of a child Run. A `Vocabulary`
/// action compiles as its registering Vocabulary declares. A verb whose namespace
/// no loaded Vocabulary claims is rejected and logged.
///
/// The compiler keeps its own refusals — the ASCII and nesting rules of SB-9a and
/// SB-4, a verb no Vocabulary claims, a missing parameter value and a missing
/// waveform — and leaves the update class to admission: an update class that is
/// fixed or missing is refused there, never here (KC-24 step 5; pre-freeze item 3).
/// It therefore needs no `ExperimentSpec`: the class it used to read from one is
/// admission's own declaration reference.
///
/// `earliest` is the instant an untimed `Vocabulary` action is admitted at: the
/// applied time is recorded as a coercion of the requested "as soon as possible"
/// (RS-19).
///
/// Rule: RS-13a, RS-14, RS-19, RS-50.
pub(crate) fn compile(
    action: &SessionAction,
    registry: &ModuleRegistry,
    earliest: TimePoint,
    waveform: Option<crate::manifest::ArtifactRef>,
) -> Result<Compiled, Vec<Violation>> {
    let reject = |check: &str, reason: String| -> Vec<Violation> {
        vec![Violation {
            check: Namespace::parse(check).expect("a valid literal"),
            key: None,
            requested: None,
            reason,
        }]
    };
    let mut out = Compiled::default();
    // SB-9a: a `Value` reaches the sealed Manifest through the action log and through
    // an Action's `params`, neither of which passes a `from_json`, so OV-15's ASCII
    // key rule and SB-4's nesting rule are checked where the value enters. Left to
    // hashing time, `seal()` failed at cleanup step 7 after the Run had transmitted.
    if let Err(e) = action.check_values() {
        return Err(reject("ezsdr.value", e.to_string()));
    }
    match action {
        SessionAction::SetParameter { target, key, value } => {
            // RS-19 / finding OQ2: a bare `SetParameter` carries no time, so the
            // Action carries none either and the class applies it at its first
            // permitted instant. No coercion is recorded: nothing was requested to
            // coerce, unlike the `Vocabulary` path below, whose `at` is a field the
            // caller left empty.
            //
            // RS-52 (item 3): no class is read here. The declaration is read at
            // admission, so a target that is undeclared or fixed is refused by
            // KC-24's step 5 with the same violation shape as a Module's Action.
            out.items.push(crate::event::PendingAction::Update(crate::event::PendingUpdate {
                target: target.clone(),
                key: key.clone(),
                value: value.clone(),
                at: None,
            }));
        }
        SessionAction::Vocabulary {
            ns,
            verb,
            target,
            at,
            params,
        } => {
            let decl = registry.verb(ns, verb).ok_or_else(|| {
                reject(
                    "ezsdr.vocabulary",
                    format!("RS-13a: no loaded Vocabulary claims {ns}.{verb}"),
                )
            })?;
            let at = match at {
                Some(t) => *t,
                None => {
                    // RS-19: the applied time is recorded as a coercion of the
                    // requested "as soon as possible", in the caller's own domain.
                    let ticks = earliest.ticks_in(earliest.domain()).unwrap_or_default(); // own domain: RS-19 records the caller's instant
                    out.coercions.push(Coercion {
                        key: Key::parse("ezsdr.action.at").expect("a valid literal"),
                        requested: Value::from("asap"),
                        applied: Value::from(ticks),
                        reason: "RS-19: admitted at the earliest instant the envelope allows"
                            .to_owned(),
                    });
                    earliest
                }
            };
            match &decl.compiles_to {
                CompileRule::UpdateParameter { key } => {
                    // RS-14: the update goes to the target the verb names, like every
                    // other target; `sink.capture` names its recorder's output, and
                    // KC-23 refuses an output the Spec does not declare.
                    // RS-14, RS-19: `sink.capture` compiles to a **timed**
                    // UpdateParameter. `at` is either the instant the caller named
                    // or `earliest`, recorded above as a coercion; dropping it here
                    // is what made §61's `capture(n, at:)` lose its time.
                    // The value comes from the action, and the Kernel supplies none.
                    // A default was a Vocabulary meaning living in Core (OV-21), and
                    // it was also invisible to RS-20's reproduction of a Run, which
                    // reads the logged `SessionAction` and would have taken the value
                    // from the Kernel's version rather than from the log (D46).
                    // RS-52 (item 3): the class is admission's to read, from the
                    // target's own declaration.
                    let value = params.get(key).cloned().ok_or_else(|| {
                        reject(
                            "ezsdr.vocabulary",
                            format!("RS-14: verb {verb} sets {key} and the action carries no value for it"),
                        )
                    })?;
                    out.items.push(crate::event::PendingAction::Update(crate::event::PendingUpdate {
                        target: target.clone(),
                        key: key.clone(),
                        value,
                        at: Some(AbsoluteDeadline::new(at)),
                    }));
                }
                CompileRule::TxBurst {
                    repeat,
                    late_policy,
                } => {
                    let waveform = waveform.ok_or_else(|| {
                        reject(
                            "ezsdr.artifact",
                            "RS-44a: the burst's waveform resolves to nothing".to_owned(),
                        )
                    })?;
                    out.items.push(crate::event::PendingAction::Action(Action::TxBurst {
                        target: target.clone(),
                        waveform,
                        repeat: *repeat,
                        at: AbsoluteDeadline::new(at),
                        requested_at: None,
                        late_policy: *late_policy,
                        metadata: params.clone(),
                    }));
                }
                CompileRule::Command {} => {
                    out.items.push(crate::event::PendingAction::Action(Action::Command {
                        target: target.clone(),
                        verb: verb.clone(),
                        params: params.clone(),
                        at: Some(AbsoluteDeadline::new(at)),
                    }));
                }
                CompileRule::None {} => {}
            }
        }
        SessionAction::Stop { target: Some(t) } => {
            out.items.push(crate::event::PendingAction::Action(Action::Stop {
                target: Some(t.clone()),
            }));
        }
        SessionAction::Stop { target: None } => out.control = Some(ControlOp::StopRun),
        SessionAction::Release {} => out.control = Some(ControlOp::Release),
        SessionAction::Adopt { token } => {
            out.control = Some(ControlOp::Adopt {
                token: token.clone(),
            })
        }
        SessionAction::Renew {} => out.control = Some(ControlOp::Renew),
        SessionAction::RunChild {
            spec_hash,
            binding_hash,
        } => {
            out.control = Some(ControlOp::RunChild {
                spec_hash: spec_hash.clone(),
                binding_hash: binding_hash.clone(),
            })
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- the implicit Spec

/// Builds a Session's implicit ExperimentSpec from the BindingProfile, one binding at
/// a time, by the first row of spec 03's table SB-T2 that applies (SB-22c):
///
/// 1. named by an Island's `executor` — its Executor; no Spec entry;
/// 2. carries `feed` — a Sink: one **output**, taking its artifact kind from the bound
///    Sink's first declared `artifact_kinds` and its `feed` from the binding;
/// 3. its Module holds Provider — one resource with empty `requires`, of the bound
///    instance's root kind;
/// 4. named by `authority` — the Authority's own slot; no Spec entry;
/// 5. anything else — no slot, which `validate` refuses (SB-22d).
///
/// The derivation refuses a binding only when its row lacks what the row reads, in
/// the order registered roles, then role, then instance, with SB-22e's or SB-22f's
/// error; every other refusal is `validate`'s, which runs over the result unchanged.
///
/// The Sink clause is not a convenience: RS-4 forbids adding a recorder while the
/// Run is `Running` and RS-14 refuses a capture with no recorder, so an implicit
/// Spec with no outputs would reject every capture in Vision §3's own example. It is
/// an output and not a graph component because a Sink is a Module role and not
/// Executor-loaded code (MA-25), and because only an output carries the link that
/// feeds it — the placed-component form had no field for that link, so a Session's
/// capture was connected to nothing.
///
/// Rule: RS-12, SB-22c.
pub fn implicit_spec(
    profile: &BindingProfile,
    registry: &ModuleRegistry,
    providers: &BTreeMap<Ident, &dyn crate::module_api::Provider>,
    sinks: &BTreeMap<Ident, &dyn crate::module_api::Sink>,
) -> Result<ExperimentSpec, SpecError> {
    let mut spec = ExperimentSpec {
        version: 1,
        ..ExperimentSpec::default()
    };
    let island_executors: BTreeSet<&Ident> = profile
        .placements
        .islands
        .iter()
        .map(|i| &i.executor)
        .collect();
    let missing = |what: &str, name: &Ident| SpecError::Structural {
        reason: format!("SB-22f: no {what} for binding {name}"),
    };
    for (name, binding) in &profile.bindings {
        // Row 1: an Island names it. A `feed` on it is SB-22g's to refuse.
        if island_executors.contains(name) {
            continue;
        }
        // Row 2: a Sink, whose output takes its `feed` from the binding and its
        // artifact kind from the bound Sink, because a Session states no preference.
        if let Some(feed) = &binding.feed {
            crate::plan::require_role(registry, name, &binding.module, Role::Sink)?;
            let sink = sinks
                .get(name)
                .ok_or_else(|| missing("Sink instance", name))?;
            let kind = sink
                .descriptor()
                .artifact_kinds
                .first()
                .cloned()
                .ok_or_else(|| SpecError::Structural {
                    reason: format!("RS-12: Sink binding {name} declares no artifact kind"),
                })?;
            spec.outputs.push(crate::spec::OutputReq {
                id: name.clone(),
                kind,
                feed: feed.clone(),
                params: BTreeMap::new(),
            });
            continue;
        }
        // Rows 3 and 4 read the registered roles, which a version nobody registered
        // does not have: refused as SB-22e refuses it on a Spec Run.
        let Some(module) = registry
            .modules()
            .find(|m| crate::module_api::is_module(m, &binding.module))
        else {
            return Err(crate::plan::not_registered(name, &binding.module));
        };
        // Row 3: a resource, of the bound instance's own root kind. Inventing a
        // Kernel kind here would give the matcher (SB-34) nothing to bind to, and
        // would put a Kernel-owned vocabulary word where RS-12 asks only for "one
        // resource per binding with empty `requires`" (finding D21).
        if module.roles.contains(&Role::Provider) {
            let provider = providers
                .get(name)
                .ok_or_else(|| missing("Provider instance", name))?;
            spec.resources.insert(
                name.clone(),
                ResourceReq {
                    kind: provider.instance().tree.kind.clone(),
                    requires: BTreeMap::new(),
                    needs: BTreeMap::new(),
                    extensions: BTreeMap::new(),
                },
            );
        }
        // Rows 4 and 5 add nothing: the Authority's own slot comes from `authority`
        // (SB-22b), and a binding with no slot is `validate`'s to refuse (SB-22d).
    }
    Ok(spec)
}

// ---------------------------------------------------------------- tests

/// Unit tests of the compilation and its fixtures, which `pre-freeze item 3` moved
/// in-crate: `compile`, `Compiled`, `PendingAction` and `ControlOp` are no longer
/// public, so an integration test cannot call them and a Run is what an outside
/// caller drives (KC-28 step 3, RS-14).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::ClockDomainId;
    use crate::module_api::UpdateClass;
    use crate::policy::EventKindDecl;

    fn t(ticks: i64) -> TimePoint {
        TimePoint::new(ClockDomainId::HOST_MONOTONIC, ticks)
    }

    /// The `test` Vocabulary of the coordinator fixtures, in-crate: `test.count`,
    /// `test.grid` (coercible), `test.flag`, `test.gain` (a declared class), `test.list`
    /// and the three verbs `compile` reads (`capture`, `start_repeat`, `sweep`).
    fn vocabulary() -> crate::module_api::VocabularyDescriptor {
        crate::module_api::VocabularyDescriptor {
            id: ns("test"),
            version: crate::module_api::Version::new(1, 0, 0),
            prefix: ns("test"),
            keys: vec![
                crate::spec::KeyDecl {
                    key: key("test.count"),
                    kind: crate::spec::ValueKind::Int,
                    coercible: false,
                    coercion_default: crate::spec::CoercionPolicy::Reject,
                    update_class: None,
                },
                crate::spec::KeyDecl {
                    key: key("test.grid"),
                    kind: crate::spec::ValueKind::Num,
                    coercible: true,
                    coercion_default: crate::spec::CoercionPolicy::Reject,
                    update_class: None,
                },
                crate::spec::KeyDecl {
                    key: key("test.flag"),
                    kind: crate::spec::ValueKind::Bool,
                    coercible: false,
                    coercion_default: crate::spec::CoercionPolicy::Warn,
                    update_class: None,
                },
                crate::spec::KeyDecl {
                    key: key("test.gain"),
                    kind: crate::spec::ValueKind::Num,
                    coercible: false,
                    coercion_default: crate::spec::CoercionPolicy::Reject,
                    update_class: Some(UpdateClass::HardwareTimed),
                },
                crate::spec::KeyDecl {
                    key: key("test.list"),
                    kind: crate::spec::ValueKind::List,
                    coercible: false,
                    coercion_default: crate::spec::CoercionPolicy::Reject,
                    update_class: Some(UpdateClass::BlockBoundary),
                },
            ],
            event_kinds: vec![EventKindDecl {
                kind: crate::event::EventKind::parse("test.custom").expect("a valid kind"),
                default: crate::policy::Reaction::Continue,
                severity: crate::event::Severity::Info,
            }],
            verbs: vec![
                crate::module_api::VerbDecl {
                    verb: id("capture"),
                    compiles_to: CompileRule::UpdateParameter {
                        key: key("test.capture"),
                    },
                },
                crate::module_api::VerbDecl {
                    verb: id("start_repeat"),
                    compiles_to: CompileRule::TxBurst {
                        repeat: true,
                        late_policy: crate::stream::LatePolicy::SendAsapAndFlag,
                    },
                },
                crate::module_api::VerbDecl {
                    verb: id("sweep"),
                    compiles_to: CompileRule::Command {},
                },
            ],
            checks: Vec::new(),
        }
    }

    fn registry() -> ModuleRegistry {
        let mut reg = ModuleRegistry::new();
        reg.register_vocabulary(vocabulary()).expect("fresh");
        reg
    }

    fn id(s: &str) -> Ident {
        Ident::parse(s).expect("a well-formed Ident literal")
    }

    fn ns(s: &str) -> Namespace {
        Namespace::parse(s).expect("a well-formed Namespace literal")
    }

    fn key(s: &str) -> Key {
        Key::parse(s).expect("a well-formed Key literal")
    }

    fn target(resource: &str) -> Target {
        Target::Resource {
            resource: id(resource),
            path: String::new(),
        }
    }

    /// A Spec that declares `radio` and `test.capture`'s Vocabulary, which is what
    /// `capture`'s target and the `test.*` key positions need.
    fn minimal_spec() -> ExperimentSpec {
        let mut spec = ExperimentSpec {
            version: 1,
            ..ExperimentSpec::default()
        };
        spec.requirements.vocabularies = vec![crate::spec::VocabularyReq {
            id: ns("test"),
            major: 1,
        }];
        spec.resources.insert(
            id("radio"),
            ResourceReq {
                kind: ns("test.device"),
                requires: BTreeMap::new(),
                needs: BTreeMap::new(),
                extensions: BTreeMap::new(),
            },
        );
        spec
    }

    #[test]
    fn rs_13a_a_session_value_with_a_non_ascii_key_is_refused_at_compile() {
        // A `Value` reaches the sealed Manifest through the action log and an Action's
        // `params`, neither of which passes a `from_json`, so the check belongs where
        // the value enters the Kernel (SB-9a, RS-11). The Run-level carrier is
        // `kc_28_a_malformed_action_takes_no_sequence_number`.
        let reg = registry();
        let bad: Value =
            serde_json::from_value(serde_json::json!({ "\u{3c1}": 1 })).expect("deserialises");
        let action = SessionAction::SetParameter {
            target: target("radio"),
            key: key("test.flag"),
            value: bad.clone(),
        };
        let violations = compile(&action, &reg, t(0), None)
            .expect_err("refused before it can be logged");
        assert!(
            violations[0].reason.contains("non-ASCII"),
            "{:?}",
            violations[0]
        );
        // And through a Vocabulary action's params.
        let vocab = SessionAction::Vocabulary {
            ns: ns("test"),
            verb: id("capture"),
            target: target("radio"),
            at: Some(t(1)),
            params: [(key("test.capture"), bad)].into_iter().collect(),
        };
        assert!(compile(&vocab, &reg, t(0), None).is_err());
    }

    #[test]
    fn rs_14_the_kernel_supplies_no_vocabulary_value_or_late_policy() {
        // RS-14 with OV-21: the Kernel learns no Vocabulary word, and `compile` was
        // supplying two — the value a `capture` verb sets and the late policy of every
        // Session-compiled burst. The value is worse than a tier violation: RS-20
        // reproduces a Run from the logged `SessionAction`, so a value the Kernel
        // invented is a value the log does not carry (finding D46).
        let reg = registry();
        let without = SessionAction::Vocabulary {
            ns: ns("test"),
            verb: id("capture"),
            target: target("radio"),
            at: Some(t(100)),
            params: BTreeMap::new(),
        };
        let violations = compile(&without, &reg, t(0), None)
            .expect_err("no value, and the Kernel invents none");
        assert!(
            violations
                .iter()
                .any(|v| v.reason.contains("RS-14") && v.reason.contains("carries no value")),
            "{violations:?}"
        );

        // The late policy is the Vocabulary's declaration, not a constant in `compile`.
        let burst = SessionAction::Vocabulary {
            ns: ns("test"),
            verb: id("start_repeat"),
            target: target("radio"),
            at: Some(t(100)),
            params: BTreeMap::new(),
        };
        let compiled = compile(
            &burst,
            &reg,
            t(0),
            Some(crate::manifest::ingest_input(
                id("wave"),
                ns("test.waveform"),
                "memory://wave",
                &[0u8; 8],
            )),
        )
        .expect("compiles");
        match &compiled.items[0] {
            crate::event::PendingAction::Action(Action::TxBurst { late_policy, .. }) => {
                assert_eq!(
                    *late_policy,
                    crate::stream::LatePolicy::SendAsapAndFlag,
                    "what the verb declares"
                );
            }
            other => panic!("expected a TxBurst: {other:?}"),
        }
    }

    #[test]
    fn rs_12_capture_reaches_the_recorder_it_names() {
        // RS-12, RS-14: the recorder is a bound Sink, carried as an output, and a
        // capture compiles to a pending update whose target is that output. The class
        // is not read here (RS-52): admission fills it from the key's own declaration.
        let reg = registry();
        let mut spec = minimal_spec();
        spec.outputs.push(crate::spec::OutputReq {
            id: id("recorder"),
            kind: ns("test.capture"),
            feed: crate::spec::SinkFeed {
                port: crate::contract::PortRef {
                    component: id("radio"),
                    port: id("rx"),
                },
                policy: crate::stream::BackPressure::DropOldest,
                capacity: 4,
            },
            params: BTreeMap::new(),
        });
        let action = SessionAction::Vocabulary {
            ns: ns("test"),
            verb: id("capture"),
            target: Target::Output {
                output: id("recorder"),
            },
            at: Some(t(100)),
            params: [(key("test.capture"), Value::from(true))]
                .into_iter()
                .collect(),
        };
        let compiled = compile(&action, &reg, t(0), None).expect("compiles");
        assert_eq!(compiled.items.len(), 1);
        match &compiled.items[0] {
            crate::event::PendingAction::Update(pending) => {
                assert_eq!(
                    pending.target,
                    Target::Output {
                        output: id("recorder")
                    }
                );
                assert_eq!(pending.key, key("test.capture"));
                assert_eq!(pending.value, Value::from(true));
                assert_eq!(pending.at, Some(AbsoluteDeadline::new(t(100))));
            }
            other => panic!("expected a class-pending update, got {other:?}"),
        }
    }

    #[test]
    fn rs_14_capture_compiles_to_a_timed_pending_update() {
        // RS-14, RS-19: `sink.capture` compiles to a **timed** pending update. The
        // instant is what Vision §61's `capture(n, at:)` rests on; an untimed capture
        // takes `earliest`, recorded as a coercion.
        let reg = registry();
        let action = SessionAction::Vocabulary {
            ns: ns("test"),
            verb: id("capture"),
            target: target("radio"),
            at: Some(t(100)),
            params: [(key("test.capture"), Value::from(true))]
                .into_iter()
                .collect(),
        };
        let compiled = compile(&action, &reg, t(0), None).expect("compiles");
        match &compiled.items[0] {
            crate::event::PendingAction::Update(pending) => {
                assert_eq!(pending.at, Some(AbsoluteDeadline::new(t(100))));
            }
            other => panic!("expected a class-pending update, got {other:?}"),
        }
        assert!(
            compiled.coercions.is_empty(),
            "an explicit time is not a coercion"
        );
    }

    #[test]
    fn rs_14_a_verb_without_its_value_is_refused() {
        // RS-14: the value comes from the action and the Kernel supplies no default
        // (D46): a verb whose key the action does not carry is refused at compile.
        let reg = registry();
        let action = SessionAction::Vocabulary {
            ns: ns("test"),
            verb: id("capture"),
            target: target("radio"),
            at: Some(t(100)),
            params: BTreeMap::new(),
        };
        let violations = compile(&action, &reg, t(0), None).expect_err("no value for the key");
        assert!(
            violations[0].reason.contains("carries no value for it"),
            "{violations:?}"
        );
    }

    #[test]
    fn rs_13a_unknown_vocabulary_verb_rejected() {
        let reg = registry();
        let action = SessionAction::Vocabulary {
            ns: ns("radio"),
            verb: id("start_repeat"),
            target: target("radio"),
            at: None,
            params: BTreeMap::new(),
        };
        let violations = compile(&action, &reg, t(0), None).expect_err("no Vocabulary claims it");
        assert!(violations[0].reason.contains("radio.start_repeat"));
    }

    #[test]
    fn rs_17_a_set_parameter_compiles_to_a_class_pending_update() {
        // RS-52 (item 3): the compiler reads no class. `test.gain` declares
        // `hardware_timed`, and the pending update carries no class at all — admission
        // fills it from the same declaration at KC-24 step 5.
        let reg = registry();
        let gain = SessionAction::SetParameter {
            target: target("radio"),
            key: key("test.gain"),
            value: Value::num(20.0).unwrap(),
        };
        let compiled = compile(&gain, &reg, t(0), None).expect("compiles");
        assert!(matches!(
            &compiled.items[0],
            crate::event::PendingAction::Update(pending) if pending.at.is_none()
        ));
        assert!(
            compiled.coercions.is_empty(),
            "nothing was requested to coerce"
        );

        // A key whose `KeyDecl` declares no class compiles too: the refusal is
        // admission's own (KC-24 step 5), never compile's.
        let flag = SessionAction::SetParameter {
            target: target("radio"),
            key: key("test.flag"),
            value: Value::from(true),
        };
        let compiled = compile(&flag, &reg, t(0), None).expect("the class is admission's");
        assert!(matches!(
            &compiled.items[0],
            crate::event::PendingAction::Update(pending) if pending.key == key("test.flag")
        ));
    }

    // A component update compiles to the same class-pending form as any other; the
    // component's own `ParamDecl` is read at admission, and the coordinator tests
    // carry that rule (`kc_24_component_update_classes_belong_to_the_target`,
    // `kc_24_a_component_value_takes_no_vocabulary_kind_check`). The test that stood
    // here varied a Spec `compile` no longer takes (review P2-2).

    #[test]
    fn rs_19_capture_asap_records_applied_time() {
        let reg = registry();
        let action = SessionAction::Vocabulary {
            ns: ns("test"),
            verb: id("capture"),
            target: target("radio"),
            at: None,
            params: [(key("test.capture"), Value::from(true))]
                .into_iter()
                .collect(),
        };
        let compiled = compile(&action, &reg, t(4_242), None).expect("compiles");
        assert_eq!(compiled.coercions.len(), 1);
        assert_eq!(
            compiled.coercions[0].requested,
            Value::from("asap".to_owned())
        );
        assert_eq!(compiled.coercions[0].applied, Value::from(4_242));
        assert!(matches!(
            &compiled.items[0],
            crate::event::PendingAction::Update(pending) if pending.at == Some(AbsoluteDeadline::new(t(4_242)))
        ));
    }

    #[test]
    fn rs_19_an_untimed_command_carries_its_resolved_time() {
        let reg = registry();
        let action = SessionAction::Vocabulary {
            ns: ns("test"),
            verb: id("sweep"),
            target: target("radio"),
            at: None,
            params: BTreeMap::new(),
        };
        let compiled = compile(&action, &reg, t(900), None).expect("compiles");
        match &compiled.items[0] {
            crate::event::PendingAction::Action(Action::Command { at: Some(at), .. }) => {
                assert_eq!(*at, AbsoluteDeadline::new(t(900)));
            }
            other => panic!("expected a timed Command, got {other:?}"),
        }
        assert!(
            compiled.coercions.len() == 1,
            "the asap resolution is recorded (RS-19)"
        );
    }

    #[test]
    fn rs_19_vocabulary_action_carries_a_time() {
        // RS-19 (review P2-2): an explicit `at` is the caller's own time, not a
        // resolved "as soon as possible", so nothing is coerced and the Action carries
        // it. The contrast is `rs_19_an_untimed_command_carries_its_resolved_time`.
        let reg = registry();
        let timed = SessionAction::Vocabulary {
            ns: ns("test"),
            verb: id("sweep"),
            target: target("radio"),
            at: Some(t(900)),
            params: BTreeMap::new(),
        };
        let compiled = compile(&timed, &reg, t(0), None).expect("compiles");
        match &compiled.items[0] {
            crate::event::PendingAction::Action(Action::Command { at: Some(at), .. }) => {
                assert_eq!(*at, AbsoluteDeadline::new(t(900)));
            }
            other => panic!("expected a timed Command, got {other:?}"),
        }
        assert!(
            compiled.coercions.is_empty(),
            "an explicit time is not a coercion"
        );
    }

    #[test]
    fn rs_44a_a_burst_without_a_waveform_is_refused() {
        let reg = registry();
        let action = SessionAction::Vocabulary {
            ns: ns("test"),
            verb: id("start_repeat"),
            target: target("radio"),
            at: Some(t(10)),
            params: BTreeMap::new(),
        };
        let bytes: Vec<u8> = (0..64u8).collect();
        let waveform = crate::manifest::ingest_input(id("wave"), ns("test.waveform"), "memory://wave", &bytes);
        assert_eq!(waveform.size_bytes, 64);
        assert_eq!(waveform.hash, ContentHash::of_bytes(&bytes));
        let compiled = compile(&action, &reg, t(0), Some(waveform.clone()))
            .expect("compiles with a resolved waveform");
        match &compiled.items[0] {
            crate::event::PendingAction::Action(Action::TxBurst {
                waveform: w,
                repeat,
                at,
                ..
            }) => {
                assert_eq!(w.hash, waveform.hash);
                assert!(*repeat);
                assert_eq!(*at, AbsoluteDeadline::new(t(10)));
            }
            other => panic!("expected a TxBurst, got {other:?}"),
        }
        // An Action whose ArtifactRef resolves to nothing is rejected.
        let violations = compile(&action, &reg, t(0), None).expect_err("no waveform");
        assert_eq!(
            violations[0].check,
            Namespace::parse("ezsdr.artifact").expect("a valid literal")
        );
    }

    #[test]
    fn rs_50_stop_with_and_without_a_target() {
        let reg = registry();
        let with = compile(
            &SessionAction::Stop {
                target: Some(target("radio")),
            },
            &reg,
            t(0),
            None,
        )
        .expect("compiles");
        assert!(matches!(
            with.items[0],
            crate::event::PendingAction::Action(Action::Stop { target: Some(_) })
        ));
        assert!(with.control.is_none(), "the Run keeps Running");

        let without = compile(&SessionAction::Stop { target: None }, &reg, t(0), None)
            .expect("compiles");
        assert!(without.items.is_empty());
        assert_eq!(without.control, Some(ControlOp::StopRun));
    }

    #[test]
    fn rs_13_every_session_action_compiles_to_its_kernel_form() {
        // RS-13: the four actions that produce a **control operation** rather than an
        // Action, and `Stop {}`, which produces the Run's own stop.
        let reg = registry();
        let hash = ContentHash::of_bytes(b"child-spec");
        let profile_hash = ContentHash::of_bytes(b"child-profile");
        let cases: Vec<(SessionAction, ControlOp)> = vec![
            (SessionAction::Release {}, ControlOp::Release),
            (
                SessionAction::Adopt {
                    token: "tok".to_owned(),
                },
                ControlOp::Adopt {
                    token: "tok".to_owned(),
                },
            ),
            (SessionAction::Renew {}, ControlOp::Renew),
            (
                SessionAction::RunChild {
                    spec_hash: hash.clone(),
                    binding_hash: profile_hash.clone(),
                },
                ControlOp::RunChild {
                    spec_hash: hash.clone(),
                    binding_hash: profile_hash,
                },
            ),
            (SessionAction::Stop { target: None }, ControlOp::StopRun),
        ];
        for (action, want) in cases {
            let compiled = compile(&action, &reg, t(0), None)
                .unwrap_or_else(|v| panic!("{action:?} compiles: {v:?}"));
            assert_eq!(compiled.control.as_ref(), Some(&want), "{action:?}");
            assert!(
                compiled.items.is_empty(),
                "{action:?} dispatches no Action"
            );
            assert!(compiled.coercions.is_empty(), "{action:?} coerces nothing");
        }
        // The contrast RS-13 draws: `Stop { target }` addresses a resource, so it is
        // an Action and not a control op.
        let compiled = compile(
            &SessionAction::Stop {
                target: Some(target("radio")),
            },
            &reg,
            t(0),
            None,
        )
        .expect("compiles");
        assert!(compiled.control.is_none());
        assert!(matches!(
            compiled.items[0],
            crate::event::PendingAction::Action(Action::Stop { target: Some(_) })
        ));
    }
}
