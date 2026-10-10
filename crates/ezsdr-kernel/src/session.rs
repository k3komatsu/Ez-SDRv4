//! Sessions, the action log and `admit()` —
//! `04-run-and-session.md` RS-12…RS-20 (Vision §3).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::binding::{AdmissionCheckRegistry, BindingProfile, CheckStage, Violation};
use crate::event::{Action, ActionId, Target};
use crate::hash::ContentHash;
use crate::module_api::{CompileRule, ModuleRegistry, Role, UpdateClass};
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
    /// It passed every check and was dispatched (RS-16).
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

/// The one admission path, shared by `validate()`, `prepare()` and every Session
/// Action.
///
/// `admit(proposed, stage)` applies the same checks in the same order at every
/// stage: the registered admission checks of SB-30, the coercion policy of SB-45,
/// and the parameter's declared update class. `validate()` calls it with a Spec's
/// requested configuration, `prepare()` with the applied one, and a Session with
/// one Action's key and value. What is shared is the check set and its order; the
/// call sites differ only in what they propose.
///
/// Rule: RS-16, RS-17.
pub struct Admitter<'a> {
    /// The registered checks (SB-29).
    pub checks: &'a AdmissionCheckRegistry,
    /// The profile's `environment`, whose sections the checks read (SB-26).
    pub environment: &'a BTreeMap<Namespace, serde_json::Value>,
    /// The declared update class per component parameter. A key absent here is
    /// looked up in its Vocabulary's `KeyDecl`, which is where a Provider parameter
    /// declares its class; a key declared in neither is rejected (RS-52, SB-2).
    pub declared_classes: &'a BTreeMap<Key, UpdateClass>,
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
                self.registry.key_decl(&c.key).ok(),
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
        // 3. the parameter's declared update class. An update through an undeclared
        //    class is rejected here, so an Executor never receives one (RS-17, RS-52).
        if stage == CheckStage::Runtime {
            for (key, value) in proposed.values().flatten() {
                if declared_class(key, self.declared_classes, self.registry).is_none() {
                    return Err(vec![Violation {
                        check: Namespace::parse("ezsdr.update_class").expect("a valid literal"),
                        key: Some(key.clone()),
                        requested: Some(value.clone()),
                        reason: format!("RS-17: {key} has no declared update class"),
                    }]);
                }
            }
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------- compilation

/// What one Session Action compiled to (RS-14).
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Compiled {
    /// The Kernel Actions to dispatch (RS-14).
    pub actions: Vec<Action>,
    /// The Lease or Run operation it is instead (RS-14).
    pub control: Option<ControlOp>,
    /// RS-19's record of an "as soon as possible" that was resolved (RS-19).
    pub coercions: Vec<Coercion>,
}

/// A Session Action that is a Lease or Run operation rather than a Kernel Action
/// (RS-13, RS-14).
#[derive(Clone, PartialEq, Debug)]
pub enum ControlOp {
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
/// The Kernel's own compilations are: `SetParameter` to an `UpdateParameter` under
/// the parameter's declared update class; `Stop { target }` to a Kernel `Stop`
/// addressed to that resource and `Stop {}` to the Run's own stop; `Release`,
/// `Adopt` and `Renew` to Lease operations; `RunChild` to the creation of a child
/// Run. A `Vocabulary` action compiles as its registering Vocabulary declares. A
/// verb whose namespace no loaded Vocabulary claims is rejected and logged.
///
/// The update class declared for a key: from the caller's map, which carries a
/// component's `params`, or else from the key's `KeyDecl` in a registered
/// Vocabulary, which is where a Provider parameter declares it. `None` means no
/// declaration exists and RS-17 rejects the change (RS-17, RS-52, SB-2, MA-35).
fn declared_class(
    key: &Key,
    declared: &BTreeMap<Key, UpdateClass>,
    registry: &ModuleRegistry,
) -> Option<UpdateClass> {
    declared
        .get(key)
        .copied()
        .or_else(|| registry.key_decl(key).ok().and_then(|d| d.update_class))
}

/// `earliest` is the instant an untimed `Vocabulary` action is admitted at: the
/// applied time is recorded as a coercion of the requested "as soon as possible"
/// (RS-19).
///
/// Rule: RS-13a, RS-14, RS-19, RS-50.
pub fn compile(
    action: &SessionAction,
    registry: &ModuleRegistry,
    declared_classes: &BTreeMap<Key, UpdateClass>,
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
            let class = declared_class(key, declared_classes, registry).ok_or_else(|| {
                reject(
                    "ezsdr.update_class",
                    format!("RS-17: {key} has no declared update class"),
                )
            })?;
            // RS-19 / finding OQ2: a bare `SetParameter` carries no time, so the
            // Action carries none either and the class applies it at its first
            // permitted instant. No coercion is recorded: nothing was requested to
            // coerce, unlike the `Vocabulary` path below, whose `at` is a field the
            // caller left empty.
            out.actions.push(Action::UpdateParameter {
                target: target.clone(),
                key: key.clone(),
                value: value.clone(),
                class,
                at: None,
            });
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
                CompileRule::UpdateParameter { key, class } => {
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
                    let value = params.get(key).cloned().ok_or_else(|| {
                        reject(
                            "ezsdr.vocabulary",
                            format!("RS-14: verb {verb} sets {key} and the action carries no value for it"),
                        )
                    })?;
                    out.actions.push(Action::UpdateParameter {
                        target: target.clone(),
                        key: key.clone(),
                        value,
                        class: *class,
                        at: Some(AbsoluteDeadline::new(at)),
                    });
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
                    out.actions.push(Action::TxBurst {
                        target: target.clone(),
                        waveform,
                        repeat: *repeat,
                        at: AbsoluteDeadline::new(at),
                        requested_at: None,
                        late_policy: *late_policy,
                        metadata: params.clone(),
                    });
                }
                CompileRule::Command {} => {
                    out.actions.push(Action::Command {
                        target: target.clone(),
                        verb: verb.clone(),
                        params: params.clone(),
                        at: Some(AbsoluteDeadline::new(at)),
                    });
                }
                CompileRule::None {} => {}
            }
        }
        SessionAction::Stop { target: Some(t) } => {
            out.actions.push(Action::Stop {
                target: Some(t.clone()),
            });
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
