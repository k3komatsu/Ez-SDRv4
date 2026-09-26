use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::hash::ContentHash;
use crate::manifest::{
    ClocksSection, EventsSection, Manifest, ModuleEntry, PrepareSection, RunSection,
    TerminationSection,
};
use crate::module_api::{Fidelity, ModuleRef, ProfileRef, Provider, StopMode};
use crate::run::{
    CleanupFailure, CleanupMode, CleanupOps, CleanupStep, RunState, Stage, StopCause, Termination,
    run_cleanup,
};
use crate::spec::{Ident, Namespace};

use super::RunHandle;
use super::state::{Inst, Shared, contain, contain_all, lock, manifest_failure, try_slot};

impl RunHandle {
    pub(super) fn settle(&mut self) {
        if lock(&self.shared.end).is_some()
            && !matches!(
                lock(&self.shared.machine).state(),
                RunState::CleanedUp { .. }
            )
        {
            self.cleanup();
        }
    }

    pub(super) fn cleanup(&mut self) {
        let Some(request) = lock(&self.shared.end).clone() else {
            return;
        };
        let current = self.state();
        if matches!(current, RunState::CleanedUp { .. }) {
            return;
        }
        let at = Some(self.shared.now());
        let result = lock(&self.shared.machine).begin_stopping(
            request.mode,
            at,
            &*self.shared.ctx.host_clock,
        );
        debug_assert!(
            result.is_ok(),
            "coordinator failed to enter Stopping: {result:?}"
        );
        let order = self
            .shared
            .routing()
            .map(|routing| routing.reverse.clone())
            .unwrap_or_default();
        let outcome = run_cleanup(
            Arc::new(Ops {
                shared: self.shared.clone(),
            }),
            &order,
            request.mode,
            &|| {
                lock(&self.shared.end)
                    .as_ref()
                    .is_some_and(|end| end.mode == CleanupMode::Abort)
                    .then_some(CleanupMode::Abort)
            },
        );
        self.shared.closing.store(true, Ordering::Release);
        self.lease.released = true;
        let termination = lock(&self.shared.end)
            .as_ref()
            .map(|e| e.termination.clone())
            .unwrap_or(request.termination);
        let at = Some(self.shared.now());
        let result = lock(&self.shared.machine).finish(
            termination.clone(),
            at,
            &*self.shared.ctx.host_clock,
        );
        debug_assert!(
            result.is_ok(),
            "coordinator failed to enter CleanedUp: {result:?}"
        );
        self.manifest = Some(self.assemble_manifest(termination, outcome.failures));
        lock(&self.shared.links).clear();
        self.links_by_ref.clear();
    }

    fn assemble_manifest(
        &self,
        termination: Termination,
        mut cleanup_failures: Vec<CleanupFailure>,
    ) -> Manifest {
        let mut fidelities = Vec::new();
        let mut manifest_failures = Vec::new();
        let mut module_sections = Vec::new();
        for (i, slot) in self.shared.providers.iter().enumerate() {
            let first = self.shared.first_fragment(Inst::Provider(i));
            match try_slot(&slot.object) {
                None => manifest_failures.push(manifest_failure(
                    Some(first),
                    "KC-44: the instance is still held by an abandoned cleanup step",
                )),
                Some(guard) => match contain_all(|| {
                    let instance = guard.instance();
                    (instance.fidelity, instance.sections.clone())
                }) {
                    Err(()) => manifest_failures.push(manifest_failure(
                        Some(first),
                        "KC-44: a Module panicked while reading instance()",
                    )),
                    Ok((fidelity, sections)) => {
                        fidelities.push(fidelity);
                        let Some(owner) = Namespace::parse(slot.module.id.as_str()).ok() else {
                            if !sections.is_empty() {
                                manifest_failures.push(manifest_failure(
                                    Some(first),
                                    format!("KC-44: Module id {} is not a Namespace", slot.module.id),
                                ));
                            }
                            continue;
                        };
                        module_sections.push((owner, first, sections));
                    }
                },
            }
        }
        let fidelity = Fidelity::weakest(&fidelities);
        let run = RunSection {
            id: self.shared.ctx.id.clone(),
            kind: self.shared.ctx.kind,
            parent: None,
            execution_class: self
                .shared
                .routing()
                .map(|routing| routing.plan.class)
                .unwrap_or(crate::module_api::ExecutionClass::Simulation),
            fidelity,
            transitions: lock(&self.shared.machine).transitions().to_vec(),
            deterministic: self.shared.routing().is_some_and(|routing| {
                routing.plan.class == crate::module_api::ExecutionClass::Simulation
            }),
        };
        let plan = self.shared.routing().map(|routing| routing.plan.clone());
        let prepare = self
            .merged
            .as_ref()
            .map_or_else(PrepareSection::default, |merged| PrepareSection {
                reports: merged.reports.clone(),
                merged_effective: merged.effective.clone(),
            });
        let mut modules = Vec::new();
        let mut selected: Vec<(ModuleRef, Option<ProfileRef>)> = Vec::new();
        for binding in self.shared.ctx.profile.bindings.values() {
            let key = (binding.module.clone(), binding.profile.clone());
            if !selected.contains(&key) {
                selected.push(key);
            }
        }
        for placement in &self.shared.ctx.profile.placements.links {
            let key = (placement.link.clone(), None);
            if !selected.contains(&key) {
                selected.push(key);
            }
        }
        for (module, profile) in selected {
            let descriptor = self
                .shared
                .ctx
                .registry
                .modules()
                .find(|d| d.id == module.id && d.version == module.version);
            modules.push(ModuleEntry {
                module,
                impl_hash: descriptor.and_then(|d| d.impl_hash.clone()),
                profile,
            });
        }
        let mut vocabularies = BTreeMap::new();
        for module in &modules {
            if let Some(descriptor) = self
                .shared
                .ctx
                .registry
                .modules()
                .find(|d| d.id == module.module.id && d.version == module.module.version)
            {
                for requirement in &descriptor.vocabularies {
                    if let Some(vocabulary) = self.shared.ctx.registry.vocabulary(&requirement.id) {
                        vocabularies.insert(requirement.id.clone(), vocabulary.version);
                    }
                }
            }
        }
        let components: BTreeMap<Ident, ContentHash> = self
            .shared
            .ctx
            .spec
            .graph
            .components
            .iter()
            .map(|(name, component)| (name.clone(), component.implementation.hash.clone()))
            .collect();
        let mut sections = BTreeMap::new();
        let links = lock(&self.shared.links);
        let drops = lock(&self.shared.link_drops);
        let link_sections: Vec<_> = links
            .iter()
            .enumerate()
            .map(|(i, (decl, _))| {
                serde_json::json!({
                    "link": decl.id,
                    "from": decl.from,
                    "to": decl.to,
                    "drops": drops.get(i).copied(),
                })
            })
            .collect();
        sections.insert(
            Namespace::parse("ezsdr.links").expect("a valid section name"),
            serde_json::Value::Array(link_sections),
        );
        if let Termination::Failed { stage } = &termination {
            if let Some((failure_stage, reason)) = lock(&self.shared.failure).as_ref() {
                if failure_stage == stage {
                    sections.insert(
                        Namespace::parse("ezsdr.failure").expect("a valid section name"),
                        serde_json::json!({ "stage": stage_name(*stage), "reason": reason }),
                    );
                }
            }
        }
        cleanup_failures.extend(lock(&self.shared.cleanup_failures).clone());
        cleanup_failures.extend(manifest_failures);
        let termination_at = run.transitions.last().and_then(|t| t.at);
        let termination_host_utc = run.transitions.last().map_or(0, |t| t.host_utc_nanos);
        let mut manifest = Manifest {
            version: 1,
            run,
            policy: self.shared.policy.get().cloned(),
            spec: self.shared.ctx.spec_section.clone(),
            binding: self.shared.ctx.binding_section.clone(),
            plan,
            prepare,
            admission: self.admission.clone(),
            modules,
            vocabularies,
            components,
            inputs: self.inputs.clone(),
            clocks: ClocksSection {
                domains: self.shared.ctx.clocks.domains(),
                relations: Vec::new(),
                sample_clocks: self.shared.ctx.clocks.sample_clock_records(),
            },
            events: EventsSection {
                counters: lock(&self.shared.counters)
                    .clone()
                    .or_else(|| self.shared.collector.get().map(|c| c.counters()))
                    .unwrap_or_default(),
                delivered: lock(&self.shared.delivered).clone(),
            },
            lease: self.lease.clone(),
            action_log: self.log.entries().to_vec(),
            termination: TerminationSection {
                reason: termination,
                at: termination_at,
                host_utc_nanos: termination_host_utc,
                cleanup_failures,
                also: lock(&self.shared.also).clone(),
            },
            artifacts: lock(&self.shared.artifacts).clone(),
            sections,
            hash: None,
        };
        for (owner, fragment, entries) in module_sections {
            for (section, value) in entries {
                if let Err(error) = manifest.write_section(&owner, section, value) {
                    manifest
                        .termination
                        .cleanup_failures
                        .push(manifest_failure(Some(fragment.clone()), format!("KC-44: {error}")));
                }
            }
        }
        if let Err(error) = manifest.seal() {
            manifest.termination.cleanup_failures.push(manifest_failure(
                None,
                format!("KC-44: the Manifest could not be sealed: {error}"),
            ));
            manifest.hash = None;
        }
        manifest
    }
}

/// Requests orderly client finish when no other end was requested (KC-34).
pub(super) fn finish(run: &mut RunHandle) {
    if !matches!(run.state(), RunState::CleanedUp { .. }) {
        request(
            &run.shared,
            Termination::Stopped {
                cause: StopCause::Client {},
            },
            CleanupMode::Orderly,
            None,
        );
        run.cleanup();
    }
}

pub(super) fn request(
    shared: &Shared,
    termination: Termination,
    mode: CleanupMode,
    reason: Option<String>,
) {
    let mut end = lock(&shared.end);
    match end.as_mut() {
        None => {
            if let (Termination::Failed { stage }, Some(reason)) = (&termination, reason) {
                *lock(&shared.failure) = Some((*stage, reason));
            }
            *end = Some(super::state::EndRequest { termination, mode });
        }
        Some(current) if current.mode == CleanupMode::Orderly && mode == CleanupMode::Abort => {
            current.mode = CleanupMode::Abort;
            let additional = match termination {
                Termination::Stopped { cause } => Some(cause),
                Termination::Failed { .. } => Some(StopCause::Abort {
                    cause: reason.unwrap_or_else(|| "a later failure escalated cleanup".to_owned()),
                }),
                Termination::Completed {} => None,
            };
            if let Some(additional) = additional {
                lock(&shared.also).push(additional);
            }
        }
        Some(_) => {}
    }
}

struct Ops {
    shared: Arc<Shared>,
}

impl Ops {
    /// The instance that owns `fragment`, when that fragment reached `prepare` and the
    /// plan routed it to an instance. The three per-fragment steps of RS-6 all ask it,
    /// and `prepared` is a pure read, so folding the two checks together changes no
    /// ordering that matters.
    fn routed(&self, fragment: Option<&Ident>) -> Option<Inst> {
        let fragment = fragment?;
        if !lock(&self.shared.prepared).contains(fragment) {
            return None;
        }
        self.shared
            .routing()
            .and_then(|routing| routing.fragment_of.get(fragment).copied())
    }
}

impl CleanupOps for Ops {
    fn perform(
        &self,
        step: CleanupStep,
        fragment: Option<&Ident>,
        _mode: CleanupMode,
    ) -> Result<(), crate::module_api::ModuleError> {
        if (step as u8) > (CleanupStep::StopRx as u8)
            || (step == CleanupStep::StopRx && self.shared.drained.load(Ordering::Acquire))
        {
            self.shared.closing.store(true, Ordering::Release);
        }
        match step {
            CleanupStep::Children
            | CleanupStep::CancelPeripherals
            | CleanupStep::ReleaseAndWriteManifest => Ok(()),
            CleanupStep::FinaliseArtifacts => {
                let marks = lock(&self.shared.marks).clone();
                let mut artifacts = lock(&self.shared.artifacts);
                for artifact in artifacts.iter_mut() {
                    let span = artifact
                        .continuity
                        .first()
                        .zip(artifact.continuity.last())
                        .map(|(first, last)| {
                            let start = self
                                .shared
                                .ctx
                                .clocks
                                .convert(first.first, self.shared.primary)
                                .map(|point| point.floor());
                            let end = super::state::ceil_convert(
                                &self.shared.ctx.clocks,
                                last.end,
                                self.shared.primary,
                            );
                            start.and_then(|start| end.map(|end| (start, end)))
                        });
                    for (kind, time) in &marks {
                        let applies = match &span {
                            None => true,
                            Some(Err(_)) => true,
                            Some(Ok((start, end))) => {
                                match self.shared.ctx.clocks.convert(*time, self.shared.primary) {
                                    Ok(at) => {
                                        let at = at.floor();
                                        at.ticks >= start.ticks && at.ticks < end.ticks
                                    }
                                    Err(_) => true,
                                }
                            }
                        };
                        if applies {
                            crate::manifest::mark_open_artifacts(
                                std::slice::from_mut(artifact),
                                kind.clone(),
                                *time,
                            );
                        }
                    }
                }
                Ok(())
            }
            CleanupStep::FreezeDispatch => {
                self.shared.frozen.store(true, Ordering::Release);
                for slot in &self.shared.providers {
                    lock(&slot.queue.0).clear();
                }
                for slot in &self.shared.sinks {
                    lock(&slot.queue.0).clear();
                }
                for slot in &self.shared.executors {
                    lock(&slot.queue.0).clear();
                }
                for handle in lock(&self.shared.scheduled).drain(..) {
                    if self.shared.cancel(handle).is_err() {
                        return Err(crate::module_api::ModuleError::rejected(
                            "KC-30: a Module panicked during cleanup: Authority cancel()",
                        ));
                    }
                }
                Ok(())
            }
            CleanupStep::RestoreBaseline => {
                let Some(instance) = self.routed(fragment) else {
                    return Ok(());
                };
                let mut done = lock(&self.shared.done);
                if !done.insert((step as u8, instance)) {
                    return Ok(());
                }
                let stop_rx = (CleanupStep::StopRx as u8, instance);
                let needs_stop = matches!(instance, Inst::Sink(_) | Inst::Executor(_))
                    && !done.contains(&stop_rx);
                let Some(mut guard) = slot_lock(&self.shared, instance) else {
                    return Err(crate::module_api::ModuleError::rejected(
                        "KC-39: the instance is still held by an abandoned cleanup step",
                    ));
                };
                if needs_stop {
                    done.insert(stop_rx);
                }
                drop(done);
                let stop_error = if needs_stop {
                    match contain(|| guard.stop_rx(current_stop_mode(&self.shared))) {
                        Ok(artifacts) => {
                            lock(&self.shared.artifacts).extend(artifacts);
                            None
                        }
                        Err(error) => {
                            super::pipeline::emit_device_lost(&self.shared, instance, &error);
                            Some(error)
                        }
                    }
                } else {
                    None
                };
                let cleanup = contain(|| {
                    guard.cleanup();
                    Ok(())
                });
                match (cleanup, stop_error) {
                    (Err(error), _) | (Ok(()), Some(error)) => Err(error),
                    (Ok(()), None) => Ok(()),
                }
            }
            CleanupStep::StopTx => {
                let Some(instance) = self.routed(fragment) else {
                    return Ok(());
                };
                let Inst::Provider(i) = instance else {
                    return Ok(());
                };
                let mut done = lock(&self.shared.done);
                if done.contains(&(CleanupStep::RestoreBaseline as u8, instance))
                    || done.contains(&(step as u8, instance))
                {
                    return Ok(());
                }
                done.insert((step as u8, instance));
                drop(done);
                let mode = current_stop_mode(&self.shared);
                let result = {
                    let mut guard = lock(&self.shared.providers[i].object);
                    contain(|| guard.stop(mode))
                };
                if let Err(error) = &result {
                    super::pipeline::emit_device_lost(&self.shared, instance, error);
                }
                result
            }
            CleanupStep::StopRx => {
                let drain_owner = super::stepping::drain_cleanup(&self.shared);
                if drain_owner && self.shared.closing.load(Ordering::Acquire) {
                    return Ok(());
                }
                let Some(instance) = self.routed(fragment) else {
                    return Ok(());
                };
                if !matches!(instance, Inst::Executor(_) | Inst::Sink(_)) {
                    return Ok(());
                }
                let mode = current_stop_mode(&self.shared);
                let mut done = lock(&self.shared.done);
                if done.contains(&(CleanupStep::RestoreBaseline as u8, instance))
                    || done.contains(&(step as u8, instance))
                {
                    return Ok(());
                }
                let guard = match instance {
                    Inst::Executor(i) => {
                        try_slot(&self.shared.executors[i].object).map(CleanupGuard::Executor)
                    }
                    Inst::Sink(i) => try_slot(&self.shared.sinks[i].object).map(CleanupGuard::Sink),
                    Inst::Provider(_) => return Ok(()),
                };
                let Some(mut guard) = guard else {
                    return Err(crate::module_api::ModuleError::rejected(
                        "KC-39: the instance is still held by an abandoned cleanup step",
                    ));
                };
                done.insert((step as u8, instance));
                drop(done);
                let result = contain(|| guard.stop_rx(mode)).map(|artifacts| {
                    lock(&self.shared.artifacts).extend(artifacts);
                });
                if let Err(error) = &result {
                    super::pipeline::emit_device_lost(&self.shared, instance, error);
                }
                result
            }
            CleanupStep::FlushEvents => {
                super::stepping::drain_and_react(&self.shared);
                if let Some(collector) = self.shared.collector.get() {
                    *lock(&self.shared.counters) = Some(collector.counters());
                }
                *lock(&self.shared.link_drops) = lock(&self.shared.links)
                    .iter()
                    .map(|(_, link)| link.drops())
                    .collect();
                Ok(())
            }
        }
    }
}

fn current_stop_mode(shared: &Shared) -> StopMode {
    match lock(&shared.end).as_ref().map(|end| end.mode) {
        Some(CleanupMode::Abort) => StopMode::Abort,
        Some(CleanupMode::Orderly) | None => StopMode::Orderly,
    }
}

enum CleanupGuard<'a> {
    Provider(std::sync::MutexGuard<'a, Box<dyn Provider>>),
    Sink(std::sync::MutexGuard<'a, Box<dyn crate::module_api::Sink>>),
    Executor(std::sync::MutexGuard<'a, Box<dyn crate::module_api::Executor>>),
}

impl CleanupGuard<'_> {
    fn stop_rx(
        &mut self,
        mode: StopMode,
    ) -> Result<Vec<crate::manifest::ArtifactRef>, crate::module_api::ModuleError> {
        match self {
            Self::Sink(g) => g.stop(mode),
            Self::Executor(g) => g.stop(mode).map(|()| Vec::new()),
            Self::Provider(_) => Ok(Vec::new()),
        }
    }

    fn cleanup(&mut self) {
        match self {
            Self::Provider(g) => g.cleanup(),
            Self::Sink(g) => g.cleanup(),
            Self::Executor(g) => g.cleanup(),
        }
    }
}

fn slot_lock(shared: &Shared, instance: Inst) -> Option<CleanupGuard<'_>> {
    match instance {
        Inst::Provider(i) => try_slot(&shared.providers[i].object).map(CleanupGuard::Provider),
        Inst::Sink(i) => try_slot(&shared.sinks[i].object).map(CleanupGuard::Sink),
        Inst::Executor(i) => try_slot(&shared.executors[i].object).map(CleanupGuard::Executor),
    }
}

fn stage_name(stage: Stage) -> &'static str {
    match stage {
        Stage::Validate => "validate",
        Stage::Plan => "plan",
        Stage::Prepare => "prepare",
        Stage::Arm => "arm",
        Stage::Run => "run",
    }
}
