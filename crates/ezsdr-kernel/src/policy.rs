//! The event-kind registry and the Policy table —
//! `04-run-and-session.md` RS-26…RS-29 (Vision §29, §53).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::event::{EventKind, Severity};
use crate::run::RunError;
use crate::spec::Namespace;

/// One of exactly four reactions. It is a table: there is no expression language
/// and no rules engine, and logic that needs conditions belongs in a Reactor or in
/// client orchestration.
///
/// Rule: RS-26.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
pub enum Reaction {
    /// Carry on.
    Continue,
    /// Record the kind and time against every artifact open at that moment (RS-30).
    MarkArtifact,
    /// Stop the Run, orderly (RS-9).
    Stop,
    /// End the Run immediately (RS-9).
    Abort,
}

/// What a Vocabulary or the Kernel declares when it registers an event kind.
///
/// A kind's default reaction is declared where the kind is registered: putting a
/// radio kind's default reaction in the Kernel would be a radio policy decision
/// inside a frozen tier.
///
/// Rule: RS-27, RS-28.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EventKindDecl {
    /// The kind (RS-27).
    pub kind: EventKind,
    /// What to do when no Policy entry overrides it (RS-28).
    pub default: Reaction,
    /// How bad it is; RS-29 falls back to this for an unregistered kind (RS-28).
    pub severity: Severity,
}

/// Which kinds exist, who owns them and what each defaults to.
///
/// The Kernel registers only the five it emits itself or owns the policy for; every
/// other kind is registered by the Vocabulary or Module that emits it, under that
/// owner's namespace.
///
/// Rule: RS-27, RS-28.
#[derive(Default)]
pub struct EventKindRegistry {
    decls: BTreeMap<EventKind, EventKindDecl>,
    owners: BTreeMap<EventKind, Option<Namespace>>,
}

impl EventKindRegistry {
    /// An empty registry (RS-27).
    pub fn new() -> EventKindRegistry {
        EventKindRegistry::default()
    }

    /// A registry holding the Kernel's five and nothing else. `EVENTS_DROPPED` and
    /// `LINK_BACKPRESSURE` continue and mark the artifact, `PROCESSOR_DEADLINE_MISS`
    /// marks the artifact, and `DEVICE_LOST` and `STEP_LIVELOCK` abort, both at
    /// `fatal` severity (RS-28).
    pub fn with_kernel_kinds() -> EventKindRegistry {
        let mut reg = EventKindRegistry::new();
        let kind = |s: &str| EventKind::parse(s).expect("a Kernel kind is well formed");
        for (k, default, severity) in [
            (EventKind::EVENTS_DROPPED, Reaction::MarkArtifact, Severity::Warning,
            ),
            (EventKind::LINK_BACKPRESSURE, Reaction::MarkArtifact, Severity::Warning,
            ),
            (EventKind::PROCESSOR_DEADLINE_MISS, Reaction::MarkArtifact, Severity::Warning,
            ),
            (EventKind::DEVICE_LOST, Reaction::Abort, Severity::Fatal),
            (EventKind::STEP_LIVELOCK, Reaction::Abort, Severity::Fatal),
        ] {
            reg.register(
                None,
                EventKindDecl { kind: kind(k), default, severity,
                },
            )
            .expect("the Kernel's own kinds do not collide");
        }
        reg
    }

    /// Registers a kind. `owner` is the namespace of the Vocabulary or Module that
    /// emits it, or `None` for the Kernel's own. A kind outside its owner's
    /// namespace is refused (RS-27, RS-39).
    pub fn register(
        &mut self,
        owner: Option<Namespace>,
        decl: EventKindDecl,
    ) -> Result<(), RunError> {
        if let Some(ns) = &owner {
            let owned = decl
                .kind
                .as_str()
                .strip_prefix(ns.as_str())
                .is_some_and(|suffix| suffix.starts_with('.'));
            if !owned {
                return Err(RunError::SectionNamespaceForbidden { ns: ns.to_string() });
            }
        }
        if self.decls.contains_key(&decl.kind) {
            // Not "unknown": RS-27 refuses a *second* declaration so that one
            // Vocabulary cannot overwrite another's default reaction and severity.
            return Err(RunError::EventKindAlreadyRegistered { kind: decl.kind.to_string(),
            });
        }
        self.owners.insert(decl.kind.clone(), owner);
        self.decls.insert(decl.kind.clone(), decl);
        Ok(())
    }

    /// The declaration, if the kind is registered (RS-27).
    pub fn get(&self, kind: &EventKind) -> Option<&EventKindDecl> {
        self.decls.get(kind)
    }

    /// Refuses an unregistered kind, so that a misspelling in `policies.failure` is
    /// an error rather than a silently ineffective entry (SB-18).
    pub fn require(&self, kind: &EventKind) -> Result<&EventKindDecl, RunError> {
        self.get(kind).ok_or_else(|| RunError::UnknownEventKind { kind: kind.to_string(),
        })
    }

    /// Every registered kind, in order (RS-33, RS-38).
    pub fn kinds(&self) -> Vec<EventKind> {
        self.decls.keys().cloned().collect()
    }

    /// Compiles the Run's Policy: the Spec's `policies.failure` overrides, then each
    /// kind's declared default (RS-26, RS-28, SB-18).
    pub fn compile(
        &self,
        overrides: &BTreeMap<EventKind, Reaction>) -> Result<Policy, RunError> {
        for kind in overrides.keys() {
            self.require(kind)?;
        }
        let mut table: BTreeMap<EventKind, Reaction> =
            self.decls.iter().map(|(k, d)| (k.clone(), d.default)).collect();
        let severities = self.decls.iter().map(|(k, d)| (k.clone(), d.severity)).collect();
        table.extend(overrides.iter().map(|(k, r)| (k.clone(), *r)));
        Ok(Policy { table, severities })
    }
}

/// A compiled reaction table for one Run (RS-26).
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    /// Kind to reaction (RS-26).
    pub table: BTreeMap<EventKind, Reaction>,
    /// Kind to severity, which RS-29 falls back to (RS-28).
    pub severities: BTreeMap<EventKind, Severity>,
}

impl Policy {
    /// What to do about a kind. A kind with no entry takes a default from its
    /// severity: `debug` and `info` continue, `warning` and `error` mark the
    /// artifact, `fatal` aborts. A flat default would be wrong in both directions —
    /// `continue` would ignore a Module's fatal event, and `mark_artifact` would
    /// mark a capture over an informational one.
    ///
    /// Rule: RS-26, RS-29.
    pub fn reaction_for(&self, kind: &EventKind) -> Reaction {
        if let Some(r) = self.table.get(kind) {
            return *r;
        }
        Policy::by_severity(self.severities.get(kind).copied().unwrap_or(Severity::Info))
    }

    /// The reaction to one event, falling back to the **event's** severity when the
    /// kind has no entry — which is the case RS-29 is about, since an unregistered
    /// kind has no declared severity for the Kernel to look up (RS-29).
    pub fn reaction_for_event(&self, kind: &EventKind, severity: Severity) -> Reaction {
        self.table.get(kind).copied().unwrap_or_else(|| Policy::by_severity(severity))
    }

    /// RS-29's severity-derived default, exposed so that a caller holding a
    /// severity but no registration can apply the same rule (RS-29).
    pub fn by_severity(severity: Severity) -> Reaction {
        match severity {
            Severity::Debug | Severity::Info => Reaction::Continue,
            Severity::Warning | Severity::Error => Reaction::MarkArtifact,
            Severity::Fatal => Reaction::Abort,
        }
    }
}
