//! Sessions, the action log and `admit()` —
//! `04-run-and-session.md` RS-12…RS-20 (Vision §3).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::binding::{AdmissionCheckRegistry, BindingProfile, CheckStage, Violation};
use crate::event::{Action, ActionId, ActionTemplate};
use crate::hash::ContentHash;
use crate::id::{ModuleId, ResourceId};
use crate::module_api::{
    CompileRule, ComponentDescriptor, ModuleRegistry, Role, UpdateClass,
};
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
/// [`SessionAction::Vocabulary`], and the Vocabulary that registers the verb
/// declares how it compiles. Vision §3's log sketch names `StartRepeat` and
/// `Capture` directly; here they are `radio.start_repeat` and `sink.capture`,
/// because a Kernel that enumerated them would need a new variant for the first
/// peripheral sweep or calibration verb.
///
/// Rule: RS-13, RS-13a, decision R3.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionAction {
    /// Generic over a namespaced key and value (RS-13a).
    SetParameter {
        /// Whose parameter.
        target: ResourceId,
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
        /// Whose resource.
        target: ResourceId,
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
        target: Option<ResourceId>,
    },
    /// Give up the Lease (RS-14).
    Release,
    /// Reclaim a Detached Lease. Adoption by run id alone would let any client
    /// seize a live transmitter (RS-24).
    Adopt {
        /// The token issued at grant.
        token: String,
    },
    /// Extend a renewable Lease (RS-24).
    Renew,
    /// Create a child Run (RS-14, RS-25).
    RunChild {
        /// The child's Spec, by hash (RS-45).
        spec_hash: ContentHash,
    },
}

/// What `admit()` decided about one Session Action (RS-15, RS-16).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
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

    /// Appends an entry with the next dense sequence number (RS-15).
    pub fn append(&mut self, time: TimePoint, action: SessionAction, outcome: Outcome) -> u32 {
        let seq = self.entries.len() as u32;
        self.entries.push(LogEntry { seq, time, action, outcome });
        seq
    }

    /// Every entry, in order (RS-15).
    pub fn entries(&self) -> &[LogEntry] {
        &self.entries
    }

    /// The entries RS-20 re-applies: those that were admitted (RS-20).
    pub fn admitted(&self) -> impl Iterator<Item = &LogEntry> {
        self.entries.iter().filter(|e| matches!(e.outcome, Outcome::Admitted { .. }))
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
            Err(RunError::ReplayDivergence { field: "binding.hash".to_owned() })
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
    /// The declared update class per parameter; a key with no entry is rejected
    /// (RS-52).
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
        effective: &BTreeMap<Key, Value>,
        proposed: &BTreeMap<Key, Value>,
        coercions: &[Coercion],
        stage: CheckStage,
    ) -> Result<Admitted, Vec<Violation>> {
        // 1. the registered admission checks (SB-30)
        let violations = self.checks.run(self.environment, effective, proposed, stage);
        if !violations.is_empty() {
            return Err(violations);
        }
        // 2. the coercion policy (SB-45, SB-46)
        let mut out = Admitted { coercions: coercions.to_vec(), warnings: Vec::new() };
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
            for key in proposed.keys() {
                if !self.declared_classes.contains_key(key) {
                    return Err(vec![Violation {
                        check: Namespace::parse("ezsdr.update_class").expect("a valid literal"),
                        key: Some(key.clone()),
                        requested: proposed.get(key).cloned(),
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
    /// Create a child Run (RS-25).
    RunChild {
        /// The child's Spec, by hash.
        spec_hash: ContentHash,
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
/// `earliest` is the instant an untimed `Vocabulary` action is admitted at: the
/// applied time is recorded as a coercion of the requested "as soon as possible"
/// (RS-19).
///
/// Rule: RS-13a, RS-14, RS-19, RS-50.
pub fn compile(
    action: &SessionAction,
    registry: &ModuleRegistry,
    declared_classes: &BTreeMap<Key, UpdateClass>,
    placed: &BTreeSet<Ident>,
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
    match action {
        SessionAction::SetParameter { target, key, value } => {
            let class = *declared_classes.get(key).ok_or_else(|| {
                reject("ezsdr.update_class", format!("RS-17: {key} has no declared update class"))
            })?;
            out.actions.push(Action::UpdateParameter {
                target: target.clone(),
                key: key.clone(),
                value: value.clone(),
                class,
            });
        }
        SessionAction::Vocabulary { ns, verb, target, at, params } => {
            let decl = registry.verb(ns, verb).ok_or_else(|| {
                reject("ezsdr.vocabulary", format!("RS-13a: no loaded Vocabulary claims {ns}.{verb}"))
            })?;
            let at = match at {
                Some(t) => *t,
                None => {
                    // RS-19: the applied time is recorded as a coercion of the
                    // requested "as soon as possible".
                    out.coercions.push(Coercion {
                        key: Key::parse("ezsdr.action.at").expect("a valid literal"),
                        requested: Value::Str("asap".to_owned()),
                        applied: Value::Int(earliest.ticks),
                        reason: "RS-19: admitted at the earliest instant the envelope allows"
                            .to_owned(),
                    });
                    earliest
                }
            };
            match &decl.compiles_to {
                CompileRule::UpdateParameter { key, class } => {
                    // RS-14: "refused when it placed none". A capture has nowhere to
                    // go unless the profile placed a recorder, and it is rejected
                    // rather than silently buffered on the host.
                    if placed.is_empty() {
                        return Err(reject(
                            "ezsdr.placement",
                            format!(
                                "RS-14: {ns}.{verb} needs a component the profile placed, and it placed none"
                            ),
                        ));
                    }
                    out.actions.push(Action::UpdateParameter {
                        target: target.clone(),
                        key: key.clone(),
                        value: params.get(key).cloned().unwrap_or(Value::Bool(true)),
                        class: *class,
                    });
                }
                CompileRule::TxBurst { repeat } => {
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
                        late_policy: crate::stream::LatePolicy::SendAsapAndFlag,
                        metadata: params.clone(),
                    });
                }
                CompileRule::PeripheralCommand => {
                    out.actions.push(Action::PeripheralCommand {
                        target: target.clone(),
                        verb: verb.clone(),
                        params: params.clone(),
                        at: Some(AbsoluteDeadline::new(at)),
                    });
                }
                CompileRule::None => {}
            }
        }
        SessionAction::Stop { target: Some(t) } => {
            out.actions.push(Action::Stop { target: Some(t.clone()) });
        }
        SessionAction::Stop { target: None } => out.control = Some(ControlOp::StopRun),
        SessionAction::Release => out.control = Some(ControlOp::Release),
        SessionAction::Adopt { token } => {
            out.control = Some(ControlOp::Adopt { token: token.clone() })
        }
        SessionAction::Renew => out.control = Some(ControlOp::Renew),
        SessionAction::RunChild { spec_hash } => {
            out.control = Some(ControlOp::RunChild { spec_hash: spec_hash.clone() })
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- the implicit Spec

/// Builds a Session's implicit ExperimentSpec from the BindingProfile: one resource
/// per binding with empty `requires`, and one graph component for every Sink the
/// profile's `placements` declares, placed as the profile places it.
///
/// The Sink clause is not a convenience: SB-25 requires every component to be
/// placed, RS-4 forbids adding one while the Run is `Running`, and RS-14 refuses a
/// capture with no recorder, so an implicit Spec with no components would reject
/// every capture in Vision §3's own example. Taking the Sinks from the profile
/// keeps the rule generic, because the Kernel places what the profile declared and
/// needs no notion of which resources can be captured.
///
/// `sink_descriptors` supplies the `ComponentDescriptor` each Sink Module declares,
/// which is what SB-25a's `module` field resolves to.
///
/// Rule: RS-12, SB-25a.
pub fn implicit_spec(
    profile: &BindingProfile,
    registry: &ModuleRegistry,
    providers: &BTreeMap<Ident, &dyn crate::module_api::Provider>,
    sink_descriptors: &BTreeMap<ModuleId, ComponentDescriptor>,
) -> Result<ExperimentSpec, SpecError> {
    let mut spec = ExperimentSpec { version: 1, ..ExperimentSpec::default() };
    for name in profile.bindings.keys() {
        // The resource's `kind` is the bound instance's own root kind. Inventing a
        // Kernel kind here would give the matcher (SB-34) nothing to bind to, and
        // would put a Kernel-owned vocabulary word where RS-12 asks only for "one
        // resource per binding with empty `requires`".
        let provider = providers.get(name).ok_or_else(|| SpecError::Structural {
            reason: format!("RS-12: no Provider instance for binding {name}"),
        })?;
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
    for (name, placement) in &profile.placements.components {
        let Some(module) = &placement.module else { continue };
        let is_sink = registry
            .modules()
            .find(|m| m.id == *module)
            .is_some_and(|m| m.roles.contains(&Role::Sink));
        if !is_sink {
            continue;
        }
        let descriptor = sink_descriptors.get(module).ok_or_else(|| SpecError::Structural {
            reason: format!("RS-12: no ComponentDescriptor for Sink module {module}"),
        })?;
        spec.graph.components.insert(name.clone(), descriptor.clone());
    }
    Ok(spec)
}

/// A placement entry that names a `module` for a component the Spec also declares
/// is refused, since the two would disagree (SB-25a).
pub fn check_placement_modules(
    spec: &ExperimentSpec,
    profile: &BindingProfile,
) -> Result<(), SpecError> {
    for (name, placement) in &profile.placements.components {
        if placement.module.is_some() && spec.graph.components.contains_key(name) {
            return Err(SpecError::Structural {
                reason: format!(
                    "SB-25a: placement of {name} names a module although the Spec declares it"
                ),
            });
        }
    }
    Ok(())
}

/// An Action that a Spec scheduled, resolved at `arm`: the template's `SpecTime`
/// becomes an `AbsoluteDeadline` in the stream's SampleClock (SB-43, RS-49a).
pub fn resolve_scheduled(template: ActionTemplate, at: AbsoluteDeadline) -> Action {
    template.resolve(at)
}
