use std::collections::{BTreeMap, BTreeSet, btree_map::Entry};
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex, OnceLock};

use crate::binding::{self, AdmissionResult, BindingProfile, Violation};
use crate::event::{
    Action, ActionId, ActionTemplate, Event, EventCollector, EventKind, EventSink, Severity,
};
use crate::hash::ContentHash;
use crate::id::{ClockDomainId, ResourceId, RunId};
use crate::manifest::{BindingSection, RunKind, SpecSection};
use crate::module_api::{
    ActionSubmitter, AttachedPort, Authority, Endpoint, ExecutionClass, Executor,
    ExecutorDescriptor, ModuleError, ModuleErrorKind, ModuleRef, PrepareContext, Provider, Sink,
};
use crate::plan::{CompileInputs, ExecutionPlan};
use crate::run::{Lease, RunState, RunStateMachine, Stage};
use crate::session::{ControlOp, Outcome, SessionAction, SessionLog};
use crate::spec::{ExperimentSpec, Ident, Namespace};
use crate::time::{
    AbsoluteDeadline, ClockDomainKind, Duration, RelativeBudget, TimeAuthority, TimePoint,
};

use super::state::{
    Context, ExecutorSlot, FailedTime, Inst, ProviderSlot, Queue, Routing, Shared, SinkSlot,
    TimeScheduleError, contain, contain_all, lock,
};
use super::{Assembly, DEFAULT_HOST_BUDGET_NS, EVENT_RING_DEPTH, KERNEL_SOURCE, RunHandle};

pub(super) fn assemble(
    kind: RunKind,
    spec: ExperimentSpec,
    spec_section: SpecSection,
    profile: BindingProfile,
    binding_section: BindingSection,
    mut assembly: Assembly,
    lease: Lease,
) -> RunHandle {
    let mut entry_failure = None;
    let mut fail_entry = |reason: String| {
        if entry_failure.is_none() {
            entry_failure = Some(reason);
        }
    };

    let mut groups: BTreeMap<(ModuleRef, String), Vec<Ident>> = BTreeMap::new();
    for name in spec.resources.keys() {
        if let Some(binding) = profile.bindings.get(name) {
            groups
                .entry(crate::plan::binding_description(binding))
                .or_default()
                .push(name.clone());
        }
    }
    let mut providers = Vec::new();
    for ((module, _), mut names) in groups {
        names.sort();
        let supplied: Vec<Ident> = names
            .iter()
            .filter(|name| assembly.providers.contains_key(*name))
            .cloned()
            .collect();
        match supplied.len() {
            0 => {}
            n if n > 1 => {
                let named = names
                    .iter()
                    .map(Ident::as_str)
                    .collect::<Vec<_>>()
                    .join(", ");
                fail_entry(format!(
                    "KC-4: resources {named} share one binding description and were handed {n} Provider objects"
                ));
                for name in supplied {
                    assembly.providers.remove(&name);
                }
            }
            _ => {
                let supplied_name = &supplied[0];
                let Some(object) = assembly.providers.remove(supplied_name) else {
                    continue;
                };
                let object: super::state::Slot<dyn Provider> = Arc::new(Mutex::new(object));
                let instance = {
                    let guard = lock(&object);
                    contain_all(|| {
                        let instance = guard.instance();
                        (
                            instance.id.clone(),
                            instance.module.clone(),
                            instance.driving.stepped,
                            instance.min_command_lead,
                        )
                    })
                };
                let (id, module_ref, stepped, lead) = match instance {
                    Ok(fields) => fields,
                    Err(()) => {
                        fail_entry(
                            "KC-30: a Module panicked during validate: instance()".to_owned(),
                        );
                        (
                            ResourceId::parse("kernel").expect("a valid reserved resource"),
                            module.clone(),
                            false,
                            None,
                        )
                    }
                };
                providers.push(ProviderSlot {
                    names,
                    id,
                    module: module_ref,
                    stepped,
                    lead,
                    object,
                    queue: Arc::new(Queue(Mutex::new(Default::default()))),
                });
            }
        }
    }
    for name in assembly.providers.keys() {
        fail_entry(format!(
            "KC-4: a Provider object under {name}, which is no bound Spec resource"
        ));
    }

    let outputs: BTreeSet<Ident> = spec.outputs.iter().map(|o| o.id.clone()).collect();
    let mut sinks = Vec::new();
    for (output, object) in std::mem::take(&mut assembly.sinks) {
        if !outputs.contains(&output) {
            fail_entry(format!(
                "KC-4: a Sink object under {output}, which is no output"
            ));
            continue;
        }
        sinks.push(SinkSlot {
            output,
            object: Arc::new(Mutex::new(object)),
            queue: Arc::new(Queue(Mutex::new(Default::default()))),
        });
    }

    let executor_names: BTreeSet<Ident> = profile
        .placements
        .islands
        .iter()
        .map(|island| island.executor.clone())
        .collect();
    let mut executors = Vec::new();
    for (name, object) in std::mem::take(&mut assembly.executors) {
        if !executor_names.contains(&name) {
            fail_entry(format!(
                "KC-4: an Executor object under {name}, which no Island names"
            ));
            continue;
        }
        let object: super::state::Slot<dyn Executor> = Arc::new(Mutex::new(object));
        let descriptor = {
            let guard = lock(&object);
            contain_all(|| guard.descriptor().clone())
        };
        let descriptor = match descriptor {
            Ok(descriptor) => descriptor,
            Err(()) => {
                fail_entry("KC-30: a Module panicked during validate: descriptor()".to_owned());
                continue;
            }
        };
        executors.push(ExecutorSlot {
            name,
            descriptor,
            object,
            queue: Arc::new(Queue(Mutex::new(Default::default()))),
        });
    }

    let fallback_root = fallback_root(&assembly);
    let authority: Arc<dyn Authority> = Arc::from(assembly.authority);
    let time = match contain_all(|| authority.time()) {
        Ok(time) => time,
        Err(()) => {
            fail_entry("KC-30: a Module panicked during validate: time()".to_owned());
            Arc::new(FailedTime {
                root: fallback_root,
            }) as Arc<dyn TimeAuthority>
        }
    };
    let primary = match contain_all(|| time.primary_root()) {
        Ok(root) => root,
        Err(()) => {
            fail_entry("KC-30: a Module panicked during validate: primary_root()".to_owned());
            fallback_root
        }
    };
    let machine = RunStateMachine::new(&*assembly.host_clock);
    let id = RunId::generate();
    let declared_classes = spec
        .graph
        .components
        .values()
        .flat_map(|component| component.params.iter())
        .map(|param| (param.key.clone(), param.update_class))
        .collect();
    let context = Context {
        kind,
        id,
        spec,
        spec_section,
        profile: profile.clone(),
        binding_section,
        environment: Arc::new(profile.environment.clone()),
        registry: assembly.registry,
        checks: assembly.checks,
        kinds: assembly.kinds,
        contracts: assembly.contracts,
        clocks: assembly.clocks,
        host_clock: assembly.host_clock,
        declared_classes,
        outputs,
    };
    let shared = Arc::new(Shared {
        ctx: context,
        providers,
        sinks,
        executors,
        authority,
        time,
        primary,
        last_now: std::sync::atomic::AtomicI64::new(0),
        routing: OnceLock::new(),
        machine: Mutex::new(machine),
        collector: OnceLock::new(),
        policy: OnceLock::new(),
        configuration: Mutex::new(BTreeMap::new()),
        next_action: AtomicU64::new(1),
        frozen: AtomicBool::new(false),
        delivered: Mutex::new(Vec::new()),
        marks: Mutex::new(Vec::new()),
        end: Mutex::new(None),
        also: Mutex::new(Vec::new()),
        failure: Mutex::new(None),
        artifacts: Mutex::new(Vec::new()),
        links: Mutex::new(Vec::new()),
        link_drops: Mutex::new(Vec::new()),
        counters: Mutex::new(None),
        prepared: Mutex::new(BTreeSet::new()),
        done: Mutex::new(BTreeSet::new()),
        drained: AtomicBool::new(false),
        closing: AtomicBool::new(false),
        scheduled: Mutex::new(Vec::new()),
        cleanup_failures: Mutex::new(Vec::new()),
    });
    RunHandle {
        shared,
        lease,
        log: SessionLog::new(),
        inputs: Vec::new(),
        store: assembly.inputs,
        agenda: Vec::<(i64, usize, Action)>::new(),
        t0: None,
        admission: AdmissionResult::default(),
        merged: None,
        links_by_ref: assembly.links,
        attached_links: BTreeMap::new(),
        entry_failure,
        last_wakeup: None,
        same_count: 0,
        manifest: None,
    }
}

fn fallback_root(assembly: &Assembly) -> ClockDomainId {
    let governed = contain_all(|| assembly.authority.descriptor().governs.clone()).ok();
    assembly
        .clocks
        .domains()
        .into_iter()
        .find(|d| {
            matches!(d.kind, ClockDomainKind::Root { .. })
                && d.id != ClockDomainId::HOST_MONOTONIC
                && governed.as_ref().is_none_or(|ids| ids.contains(&d.id))
        })
        .map(|d| d.id)
        .unwrap_or(ClockDomainId::HOST_MONOTONIC)
}

impl RunHandle {
    pub(super) fn pipeline(&mut self) {
        if let Some(reason) = self.entry_failure.clone() {
            self.fail(Stage::Validate, reason);
            return;
        }
        let admission = match contain_all(|| {
            self.with_inputs(|inputs| {
                crate::plan::validate(&self.shared.ctx.spec, &self.shared.ctx.profile, inputs)
            })
        }) {
            Ok(Ok(admission)) => admission,
            Ok(Err(error)) => {
                self.fail(Stage::Validate, error.to_string());
                return;
            }
            Err(()) => {
                self.fail(
                    Stage::Validate,
                    "KC-30: a Module panicked during validate".to_owned(),
                );
                return;
            }
        };
        self.admission = admission;
        if !self.admission.is_admitted() {
            self.fail(
                Stage::Validate,
                format!(
                    "KC-7: not admitted: rejected {:?}, violations {:?}",
                    self.admission.rejected, self.admission.violations
                ),
            );
            return;
        }
        let policy = match self
            .shared
            .ctx
            .kinds
            .compile(&self.shared.ctx.spec.policies.failure)
        {
            Ok(policy) => policy,
            Err(error) => {
                self.fail(Stage::Validate, format!("KC-8: {error}"));
                return;
            }
        };
        let _ = self.shared.policy.set(policy);
        self.shared.move_to(RunState::Validated {});
        if lock(&self.shared.end).is_some() {
            return;
        }

        let plan = match contain_all(|| {
            self.with_inputs(|inputs| {
                crate::plan::plan(
                    &self.shared.ctx.spec,
                    &self.shared.ctx.profile,
                    &self.admission,
                    inputs,
                    Vec::new(),
                )
            })
        }) {
            Ok(Ok(plan)) => plan,
            Ok(Err(error)) => {
                self.fail(Stage::Plan, error.to_string());
                return;
            }
            Err(()) => {
                self.fail(
                    Stage::Plan,
                    "KC-30: a Module panicked during plan".to_owned(),
                );
                return;
            }
        };
        let plan_class = plan.class;
        let routing = build_routing(plan, self.admission.matched.clone(), &self.shared);
        let _ = self.shared.routing.set(routing);
        if plan_class != ExecutionClass::Simulation {
            self.fail(
                Stage::Plan,
                format!(
                    "KC-2: Phase 2 runs the Simulation class only; this plan's class is {:?}",
                    plan_class
                ),
            );
            return;
        }
        self.shared.move_to(RunState::Planned {});
        if lock(&self.shared.end).is_some() {
            return;
        }

        if !self.check_inputs() || !self.create_links() || !self.create_collector() {
            return;
        }
        if !self.prepare_fragments()
            || lock(&self.shared.end).is_some()
            || !self.arm_instances()
            || lock(&self.shared.end).is_some()
            || !self.set_start_instant()
            || lock(&self.shared.end).is_some()
        {
            return;
        }
        let Some(batch) = self.resolve_schedule() else {
            return;
        };
        if lock(&self.shared.end).is_some() {
            return;
        }
        let Some(batch) = self.admit_schedule(batch) else {
            return;
        };
        if lock(&self.shared.end).is_some() {
            return;
        }
        if !self.start_instances() || lock(&self.shared.end).is_some() {
            return;
        }
        self.shared.move_to(RunState::Running {});
        if lock(&self.shared.end).is_some() {
            return;
        }
        for admitted in batch {
            super::admission::dispatch(&self.shared, admitted);
        }
        super::stepping::round(&self.shared, self.shared.now(), false);
    }

    fn check_inputs(&mut self) -> bool {
        for (i, entry) in self.shared.ctx.spec.schedule.iter().enumerate() {
            let crate::event::ActionTemplate::TxBurst { waveform, .. } = &entry.action else {
                continue;
            };
            let refusal =
                if !(waveform.uri.starts_with("mem:") || waveform.uri.starts_with("file://")) {
                    Some("uri must begin with mem: or file://".to_owned())
                } else if let Some(bytes) = self.store.get(&waveform.hash) {
                    if bytes.len() as u64 != waveform.size_bytes {
                        Some(format!(
                            "size is {}, but the ArtifactRef declares {}",
                            bytes.len(),
                            waveform.size_bytes
                        ))
                    } else if crate::hash::ContentHash::of_bytes(bytes) != waveform.hash {
                        Some("bytes do not match the ArtifactRef hash".to_owned())
                    } else {
                        None
                    }
                } else {
                    Some(format!("no bytes were supplied for {}", waveform.hash))
                };
            if let Some(reason) = refusal {
                self.fail(Stage::Plan, format!("KC-9: entry {i}: {reason}"));
                return false;
            }
            if !self.inputs.iter().any(|input| input.hash == waveform.hash) {
                self.inputs.push(waveform.clone());
            }
        }
        true
    }

    fn create_links(&mut self) -> bool {
        let declarations = self
            .shared
            .routing()
            .expect("plan was installed")
            .plan
            .links
            .clone();
        let mut attached: BTreeMap<Ident, Vec<AttachedPort>> = BTreeMap::new();
        for decl in declarations {
            let placement = self
                .shared
                .ctx
                .profile
                .placements
                .links
                .iter()
                .find(|p| p.from == decl.from && p.to == decl.to);
            let Some(placement) = placement else {
                self.fail(
                    Stage::Plan,
                    format!(
                        "KC-10: no LinkPlacement for {:?} -> {:?}",
                        decl.from, decl.to
                    ),
                );
                break;
            };
            let Some(link) = self.links_by_ref.get(&placement.link) else {
                self.fail(
                    Stage::Plan,
                    format!("KC-10: no Link object for {}", placement.link.id),
                );
                break;
            };
            let descriptor = match contain_all(|| link.descriptor().clone()) {
                Ok(descriptor) => descriptor,
                Err(()) => {
                    self.fail(
                        Stage::Plan,
                        "KC-30: a Module panicked during plan: Link descriptor()".to_owned(),
                    );
                    break;
                }
            };
            if self.shared.ctx.registry.link_descriptor(&placement.link) != Some(&descriptor) {
                self.fail(
                    Stage::Plan,
                    format!(
                        "KC-10: MA-27a: {}'s descriptor differs from the registered one",
                        placement.link.id
                    ),
                );
                break;
            }
            let link = match contain(|| link.create(&decl)) {
                Ok(link) => link,
                Err(error) => {
                    self.fail(Stage::Plan, format!("KC-10: {}", error.message));
                    break;
                }
            };
            let Some(from_owner) = fragment_owner(&self.shared, &decl.from.component) else {
                self.fail(
                    Stage::Plan,
                    format!("KC-10: no fragment owns {}", decl.from.component),
                );
                break;
            };
            let Some(to_owner) = fragment_owner(&self.shared, &decl.to.component) else {
                self.fail(
                    Stage::Plan,
                    format!("KC-10: no fragment owns {}", decl.to.component),
                );
                break;
            };
            attached.entry(from_owner).or_default().push(AttachedPort {
                component: decl.from.component.clone(),
                port: decl.from.port.clone(),
                endpoint: Endpoint::StreamOut(link.clone()),
            });
            attached.entry(to_owner).or_default().push(AttachedPort {
                component: decl.to.component.clone(),
                port: decl.to.port.clone(),
                endpoint: Endpoint::StreamIn(link.clone()),
            });
            lock(&self.shared.links).push((decl, link));
        }
        self.links_by_ref.clear();
        if lock(&self.shared.end).is_some() {
            return false;
        }
        self.attached_links = attached;
        true
    }

    fn create_collector(&self) -> bool {
        let mut pairs = BTreeSet::new();
        let kernel_kinds = [
            EventKind::parse(EventKind::EVENTS_DROPPED).expect("Kernel event kind"),
            EventKind::parse(EventKind::LINK_BACKPRESSURE).expect("Kernel event kind"),
            EventKind::parse(EventKind::PROCESSOR_DEADLINE_MISS).expect("Kernel event kind"),
            EventKind::parse(EventKind::DEVICE_LOST).expect("Kernel event kind"),
            EventKind::parse(EventKind::STEP_LIVELOCK).expect("Kernel event kind"),
        ];
        let device_lost = EventKind::parse(EventKind::DEVICE_LOST).expect("Kernel event kind");

        for slot in &self.shared.providers {
            let instance = match contain_all(|| {
                let guard = lock(&slot.object);
                guard.instance().clone()
            }) {
                Ok(instance) => instance,
                Err(()) => {
                    self.fail(
                        Stage::Prepare,
                        "KC-30: a Module panicked while building the event table".to_owned(),
                    );
                    return false;
                }
            };
            let kinds = module_event_kinds(&self.shared, &slot.module);
            for node in instance.tree.walk() {
                for kind in &kinds {
                    pairs.insert((node.id.clone(), kind.clone()));
                }
            }
            pairs.insert((instance.id, device_lost.clone()));
        }
        for slot in &self.shared.sinks {
            let module = self
                .shared
                .ctx
                .profile
                .bindings
                .get(&slot.output)
                .map(|b| &b.module);
            let Some(module) = module else {
                continue;
            };
            let source = ResourceId::parse(&format!("sink/{}", slot.output))
                .expect("validated Sink event source");
            for kind in module_event_kinds(&self.shared, module) {
                pairs.insert((source.clone(), kind));
            }
            pairs.insert((source, device_lost.clone()));
        }
        let routing = self.shared.routing().expect("plan was installed");
        for fragment in routing
            .plan
            .fragments
            .iter()
            .filter(|f| f.role == crate::module_api::Role::Executor)
        {
            let Some(Inst::Executor(i)) = routing.fragment_of.get(&fragment.id).copied() else {
                continue;
            };
            let module = self
                .shared
                .ctx
                .profile
                .bindings
                .get(&self.shared.executors[i].name)
                .map(|b| &b.module);
            let Some(module) = module else {
                continue;
            };
            let source =
                ResourceId::parse(fragment.id.as_str()).expect("validated Island event source");
            for kind in module_event_kinds(&self.shared, module) {
                pairs.insert((source.clone(), kind));
            }
        }
        for (i, _) in self.shared.executors.iter().enumerate() {
            let source = ResourceId::parse(self.shared.first_fragment(Inst::Executor(i)).as_str())
                .expect("validated Island event source");
            pairs.insert((source, device_lost.clone()));
        }
        for kind in kernel_kinds {
            pairs.insert((
                ResourceId::parse(KERNEL_SOURCE).expect("Kernel source"),
                kind,
            ));
        }
        let pairs: Vec<_> = pairs.into_iter().collect();
        let Some(policy) = self.shared.policy.get() else {
            self.fail(
                Stage::Prepare,
                "KC-8: the Run Policy was not compiled".to_owned(),
            );
            return false;
        };
        let collector = EventCollector::new(
            &pairs,
            &self.shared.ctx.kinds.kinds(),
            EVENT_RING_DEPTH,
            policy,
        );
        let _ = self.shared.collector.set(Arc::new(collector));
        true
    }

    fn prepare_fragments(&mut self) -> bool {
        let routing = self.shared.routing().expect("plan was installed");
        let fragments = routing.plan.fragments.clone();
        let plan_class = routing.plan.class;
        let Some(collector) = self.shared.collector.get().cloned() else {
            self.fail(
                Stage::Prepare,
                "KC-8: the event collector was not created".to_owned(),
            );
            return false;
        };
        let submitter: Arc<dyn ActionSubmitter> = Arc::new(Submitter {
            shared: Arc::downgrade(&self.shared),
        });
        let budget = RelativeBudget::new(Duration::new(
            ClockDomainId::HOST_MONOTONIC,
            DEFAULT_HOST_BUDGET_NS,
        ))
        .expect("positive host budget");
        let mut reports = Vec::new();
        let mut report_fragments = Vec::new();
        for fragment in &fragments {
            if fragment.role == crate::module_api::Role::Authority {
                continue;
            }
            let Some(instance) = self
                .shared
                .routing()
                .and_then(|r| r.fragment_of.get(&fragment.id))
                .copied()
            else {
                reports.push(Err(ModuleError::rejected(format!(
                    "KC-12: no instance owns fragment {}",
                    fragment.id
                ))));
                break;
            };
            let island = if fragment.role == crate::module_api::Role::Executor {
                match serde_json::from_value::<crate::module_api::IslandDecl>(
                    fragment.content.clone(),
                ) {
                    Ok(island) => Some(island),
                    Err(error) => {
                        reports.push(Err(ModuleError::rejected(format!(
                            "KC-12: invalid Island: {error}"
                        ))));
                        break;
                    }
                }
            } else {
                None
            };
            let components = island.as_ref().map_or_else(BTreeMap::new, |island| {
                island
                    .components
                    .iter()
                    .filter_map(|id| {
                        self.shared
                            .ctx
                            .spec
                            .graph
                            .components
                            .get(id)
                            .map(|c| (id.clone(), c.clone()))
                    })
                    .collect()
            });
            let context = PrepareContext {
                run: self.shared.ctx.id.clone(),
                class: plan_class,
                time: self.shared.time.clone(),
                clocks: self.shared.ctx.clocks.clone(),
                events: collector.clone(),
                actions: self.shared.queue(instance).clone(),
                actions_out: submitter.clone(),
                environment: self.shared.ctx.environment.clone(),
                links: self.attached_links.remove(&fragment.id).unwrap_or_default(),
                components,
                host_budget: budget,
            };
            lock(&self.shared.prepared).insert(fragment.id.clone());
            report_fragments.push(fragment.id.clone());
            let report = match instance {
                Inst::Provider(i) => {
                    let mut guard = lock(&self.shared.providers[i].object);
                    contain(|| guard.prepare(fragment, context))
                }
                Inst::Sink(i) => {
                    let mut guard = lock(&self.shared.sinks[i].object);
                    contain(|| guard.prepare(fragment, context))
                }
                Inst::Executor(i) => {
                    let mut guard = lock(&self.shared.executors[i].object);
                    contain(|| guard.prepare(island.as_ref().expect("Executor Island"), context))
                }
            };
            let failed = report.is_err();
            reports.push(report);
            if failed {
                break;
            }
            if lock(&self.shared.end).is_some() {
                return false;
            }
        }
        let collected = contain_all(|| {
            self.with_inputs(|inputs| {
                crate::plan::collect_prepare(
                    reports,
                    &self.shared.ctx.spec,
                    &self.shared.ctx.profile,
                    inputs,
                    &self.admission,
                )
            })
        });
        let merged = match collected {
            Err(()) => {
                self.fail(
                    Stage::Prepare,
                    "KC-30: a Module panicked during prepare".to_owned(),
                );
                return false;
            }
            Ok(Err(crate::plan::PrepareError::Fragment { index, error })) => {
                let id = report_fragments.get(index).cloned();
                self.fail(
                    Stage::Prepare,
                    format!(
                        "KC-12: fragment {}: {}",
                        id.map_or_else(|| "unknown".to_owned(), |id| id.to_string()),
                        error.message
                    ),
                );
                return false;
            }
            Ok(Err(crate::plan::PrepareError::Violations(violations))) => {
                self.fail(Stage::Prepare, format!("KC-12: {violations:?}"));
                return false;
            }
            Ok(Ok(merged)) => merged,
        };
        let configuration = merged
            .reports
            .iter()
            .map(|r| (r.fragment.clone(), r.effective.clone()))
            .collect();
        *lock(&self.shared.configuration) = configuration;
        self.merged = Some(merged);
        self.shared.move_to(RunState::Prepared {});
        true
    }

    fn arm_instances(&mut self) -> bool {
        let order = self
            .shared
            .routing()
            .expect("plan was installed")
            .order
            .clone();
        for instance in order {
            let result = match instance {
                Inst::Provider(i) => {
                    let mut g = lock(&self.shared.providers[i].object);
                    contain(|| g.arm())
                }
                Inst::Executor(i) => {
                    let mut g = lock(&self.shared.executors[i].object);
                    contain(|| g.arm())
                }
                Inst::Sink(i) => {
                    let mut g = lock(&self.shared.sinks[i].object);
                    contain(|| g.arm())
                }
            };
            if let Err(error) = result {
                emit_device_lost(&self.shared, instance, &error);
                let first = self.shared.first_fragment(instance);
                self.fail(Stage::Arm, format!("KC-30: {first}: {}", error.message));
                return false;
            }
            if lock(&self.shared.end).is_some() {
                return false;
            }
        }
        self.shared.move_to(RunState::Armed {});
        true
    }

    fn set_start_instant(&mut self) -> bool {
        let lead_ns = match binding::start_lead_ns(&self.shared.ctx.profile.environment) {
            Ok(lead) => lead,
            Err(error) => {
                self.fail(Stage::Arm, error.to_string());
                return false;
            }
        };
        let lead_ticks = match super::state::ceil_rescale(
            &self.shared.ctx.clocks,
            Duration::new(ClockDomainId::HOST_MONOTONIC, lead_ns as i64),
            self.shared.primary,
        ) {
            Ok(ticks) => ticks,
            Err(error) => {
                self.fail(Stage::Arm, format!("KC-15: {error}"));
                return false;
            }
        };
        let Some(ticks) = self.shared.now().ticks.checked_add(lead_ticks) else {
            self.fail(Stage::Arm, "KC-15: overflow".to_owned());
            return false;
        };
        self.t0 = Some(TimePoint::new(self.shared.primary, ticks));
        true
    }

    fn resolve_schedule(&mut self) -> Option<Vec<(usize, Inst, Ident, Action)>> {
        let t0 = self.t0.expect("T0 calculated after arm");
        let entries = self.shared.ctx.spec.schedule.clone();
        let mut resolved = Vec::with_capacity(entries.len());
        for (i, entry) in entries.into_iter().enumerate() {
            let template = entry.action;
            let rewritten = match rewrite_template(&self.shared, &template) {
                Ok(target) => target,
                Err(reason) => {
                    self.fail(Stage::Arm, format!("KC-16: entry {i}: {reason}"));
                    return None;
                }
            };
            let Some(node) = self
                .shared
                .routing()
                .and_then(|routing| routing.matched.get(&entry.at.clock))
            else {
                self.fail(
                    Stage::Arm,
                    format!(
                        "KC-16: entry {i}: clock {} is no Spec resource",
                        entry.at.clock
                    ),
                );
                return None;
            };
            let streams: Vec<_> = self
                .shared
                .ctx
                .clocks
                .declared_sample_clocks()
                .into_iter()
                .filter(|clock| clock.stream.is_within(node))
                .collect();
            let clock = if let Some(target) = rewritten.as_ref().map(|(target, _, _)| target) {
                streams
                    .iter()
                    .find(|clock| &clock.stream == target)
                    .cloned()
            } else {
                None
            };
            let clock = clock.or_else(|| {
                let first = streams.first()?;
                streams
                    .iter()
                    .all(|clock| clock.root_ticks_per_tick == first.root_ticks_per_tick)
                    .then_some(first.clone())
            });
            let Some(clock) = clock else {
                let reason = if streams.is_empty() {
                    format!("{} has no stream clock", entry.at.clock)
                } else {
                    format!(
                        "ambiguous: {}'s stream clocks differ in rate",
                        entry.at.clock
                    )
                };
                self.fail(Stage::Arm, format!("KC-16: entry {i}: {reason}"));
                return None;
            };
            if clock.root != self.shared.primary {
                self.fail(
                    Stage::Arm,
                    format!("KC-16: entry {i}: the stream clock is not on the primary root"),
                );
                return None;
            }
            if entry.at.offset_ticks < 0 {
                self.fail(Stage::Arm, format!("KC-16: entry {i}: negative offset"));
                return None;
            }
            let product = (entry.at.offset_ticks as i128)
                .checked_mul(clock.root_ticks_per_tick.num() as i128);
            let Some(product) = product else {
                self.fail(Stage::Arm, format!("KC-16: entry {i}: overflow"));
                return None;
            };
            let denominator = clock.root_ticks_per_tick.den() as i128;
            let Some(rounded) = product
                .checked_add(denominator - 1)
                .map(|v| v / denominator)
            else {
                self.fail(Stage::Arm, format!("KC-16: entry {i}: overflow"));
                return None;
            };
            let Some(instant) = (t0.ticks as i128)
                .checked_add(rounded)
                .filter(|v| *v >= i64::MIN as i128 && *v <= i64::MAX as i128)
                .map(|v| v as i64)
            else {
                self.fail(Stage::Arm, format!("KC-16: entry {i}: overflow"));
                return None;
            };
            resolved.push((instant, i, template, rewritten));
        }
        resolved.sort_by_key(|(instant, index, _, _)| (*instant, *index));
        let now = self.shared.now();
        let mut batch = Vec::new();
        for (instant, index, template, rewritten) in resolved {
            let timed = template.is_timed();
            let deadline = AbsoluteDeadline::new(TimePoint::new(self.shared.primary, instant));
            let action = template.resolve(deadline);
            if timed {
                let (instance, fragment) = match rewritten {
                    Some((_, instance, fragment)) => (instance, fragment),
                    None => {
                        self.fail(
                            Stage::Arm,
                            format!("KC-16: entry {index}: timed Action has no target"),
                        );
                        return None;
                    }
                };
                if let Action::TxBurst { late_policy, .. } = &action {
                    if *late_policy == crate::stream::LatePolicy::RejectAtPlan {
                        if let Inst::Provider(provider) = instance {
                            let lead = self.shared.providers[provider]
                                .lead
                                .unwrap_or(Duration::new(ClockDomainId::HOST_MONOTONIC, 0));
                            let ahead = Duration::new(
                                self.shared.primary,
                                instant.saturating_sub(now.ticks),
                            );
                            match self.shared.ctx.clocks.compare_durations(ahead, lead) {
                                Ok(std::cmp::Ordering::Less) => {
                                    self.fail(Stage::Arm, format!(
                                        "KC-19: SC-27: entry {index}'s burst is {} ticks ahead, shorter than its target's min_command_lead",
                                        instant.saturating_sub(now.ticks),
                                    ));
                                    return None;
                                }
                                Ok(_) => {}
                                Err(error) => {
                                    self.fail(Stage::Arm, format!(
                                        "KC-19: SC-27: entry {index}'s lead comparison failed: {error}"
                                    ));
                                    return None;
                                }
                            }
                        }
                    }
                }
                batch.push((index, instance, fragment, action));
            } else {
                self.agenda.push((instant, index, action));
                match self.shared.schedule(
                    TimePoint::new(self.shared.primary, instant),
                    Box::new(|_| {}),
                ) {
                    Ok(handle) => lock(&self.shared.scheduled).push(handle),
                    Err(TimeScheduleError::Refused(error)) => {
                        self.fail(Stage::Arm, format!("KC-17: entry {index}: {error}"));
                        return None;
                    }
                    Err(TimeScheduleError::Panicked) => {
                        self.fail(
                            Stage::Arm,
                            "KC-30: a Module panicked during arm: Authority schedule()".to_owned(),
                        );
                        return None;
                    }
                }
            }
        }
        Some(batch)
    }

    fn admit_schedule(
        &mut self,
        batch: Vec<(usize, Inst, Ident, Action)>,
    ) -> Option<Vec<super::admission::AdmittedAction>> {
        let mut configuration = lock(&self.shared.configuration).clone();
        let mut admitted = Vec::with_capacity(batch.len());
        for (index, _instance, _fragment, action) in batch {
            match super::admission::admit_with(
                &self.shared,
                action,
                super::state::Origin::Schedule,
                &[],
                &configuration,
            ) {
                Ok(action) => {
                    if let Action::UpdateParameter { key, value, .. } = &action.action {
                        configuration
                            .entry(action.fragment.clone())
                            .or_default()
                            .insert(key.clone(), value.clone());
                    }
                    admitted.push(action);
                }
                Err(violations) => {
                    self.fail(Stage::Arm, format!("KC-17: entry {index}: {violations:?}"));
                    return None;
                }
            }
        }
        Some(admitted)
    }

    fn start_instances(&mut self) -> bool {
        let order = self
            .shared
            .routing()
            .expect("plan was installed")
            .order
            .clone();
        let t0 = self.t0.expect("T0 calculated after arm");
        for instance in order {
            let result = match instance {
                Inst::Provider(i) => {
                    let mut g = lock(&self.shared.providers[i].object);
                    contain(|| g.start(Some(t0)))
                }
                Inst::Executor(i) => {
                    let mut g = lock(&self.shared.executors[i].object);
                    contain(|| g.start())
                }
                Inst::Sink(i) => {
                    let mut g = lock(&self.shared.sinks[i].object);
                    contain(|| g.start())
                }
            };
            if let Err(error) = result {
                emit_device_lost(&self.shared, instance, &error);
                let first = self.shared.first_fragment(instance);
                self.fail(Stage::Arm, format!("KC-30: {first}: {}", error.message));
                return false;
            }
            if lock(&self.shared.end).is_some() {
                return false;
            }
        }
        true
    }

    fn with_inputs<R>(&self, f: impl FnOnce(&CompileInputs<'_>) -> R) -> R {
        let provider_guards: Vec<_> = self
            .shared
            .providers
            .iter()
            .map(|slot| lock(&slot.object))
            .collect();
        let mut providers = BTreeMap::new();
        for (slot, guard) in self.shared.providers.iter().zip(&provider_guards) {
            for name in &slot.names {
                providers.insert(name.clone(), &***guard as &dyn Provider);
            }
        }
        let sink_guards: Vec<_> = self
            .shared
            .sinks
            .iter()
            .map(|slot| lock(&slot.object))
            .collect();
        let mut sinks = BTreeMap::new();
        for (slot, guard) in self.shared.sinks.iter().zip(&sink_guards) {
            sinks.insert(slot.output.clone(), &***guard as &dyn Sink);
        }
        let authorities = BTreeMap::from([(
            self.shared.ctx.profile.authority.clone(),
            self.shared.authority.descriptor().clone(),
        )]);
        let executors: BTreeMap<Ident, ExecutorDescriptor> = self
            .shared
            .executors
            .iter()
            .map(|slot| (slot.name.clone(), slot.descriptor.clone()))
            .collect();
        let inputs = CompileInputs {
            registry: &self.shared.ctx.registry,
            checks: &self.shared.ctx.checks,
            kinds: &self.shared.ctx.kinds,
            providers: &providers,
            authorities: &authorities,
            executors: &executors,
            contracts: &self.shared.ctx.contracts,
            sinks: &sinks,
            is_session: self.shared.ctx.kind == RunKind::Session,
        };
        f(&inputs)
    }

    fn fail(&self, stage: Stage, reason: String) {
        super::ending::request(
            &self.shared,
            crate::run::Termination::Failed { stage },
            crate::run::CleanupMode::Abort,
            Some(reason),
        );
    }
}

pub(super) fn submit(
    run: &mut RunHandle,
    action: SessionAction,
    waveform: Option<&[u8]>,
) -> Result<crate::session::LogEntry, super::RunHandleError> {
    let now = run.shared.now();
    run.log
        .check_entry(&now, &action)
        .map_err(|error| super::RunHandleError::Malformed { error })?;
    let log_action = action.clone();

    let waveform = if let Some(bytes) = waveform {
        let hash = ContentHash::of_bytes(bytes);
        let reference = if let Some(existing) = run.inputs.iter().find(|input| input.hash == hash) {
            existing.clone()
        } else {
            let id = Ident::parse(&format!("input_{}", run.inputs.len()))
                .expect("generated input id is valid");
            let reference = crate::manifest::ingest_input(
                id,
                Namespace::parse("ezsdr.input").expect("input namespace is valid"),
                format!("mem:{hash}"),
                bytes,
            );
            run.inputs.push(reference.clone());
            reference
        };
        run.store.entry(hash).or_insert_with(|| bytes.to_vec());
        Some(reference)
    } else {
        None
    };

    let earliest = match session_earliest(&run.shared, &action, now) {
        Ok(earliest) => earliest,
        Err(violation) => {
            run.log
                .append(
                    now,
                    action,
                    Outcome::Rejected {
                        violations: vec![violation],
                    },
                )
                .map_err(|error| super::RunHandleError::Malformed { error })?;
            return Ok(run.log.entries().last().expect("entry appended").clone());
        }
    };
    let mut compiled = match crate::session::compile(
        &action,
        &run.shared.ctx.registry,
        &run.shared.ctx.declared_classes,
        &run.shared.ctx.outputs,
        earliest,
        waveform,
    ) {
        Ok(compiled) => compiled,
        Err(violations) => {
            run.log
                .append(now, action, Outcome::Rejected { violations })
                .map_err(|error| super::RunHandleError::Malformed { error })?;
            return Ok(run.log.entries().last().expect("entry appended").clone());
        }
    };

    if matches!(compiled.control.as_ref(), Some(ControlOp::RunChild { .. })) {
        run.log
            .append(
                now,
                action,
                Outcome::Rejected {
                    violations: vec![violation(
                        "ezsdr.run_child",
                        "RS-25a: child Runs are Phase 6's",
                    )],
                },
            )
            .map_err(|error| super::RunHandleError::Malformed { error })?;
        return Ok(run.log.entries().last().expect("entry appended").clone());
    }

    let mut end_after_admission = None;
    match compiled.control.take() {
        Some(ControlOp::StopRun) => {
            end_after_admission = Some((
                crate::run::Termination::Stopped {
                    cause: crate::run::StopCause::Client {},
                },
                crate::run::CleanupMode::Orderly,
            ));
        }
        Some(ControlOp::Release) => {
            run.lease.released = true;
            end_after_admission = Some((
                crate::run::Termination::Stopped {
                    cause: crate::run::StopCause::Client {},
                },
                crate::run::CleanupMode::Orderly,
            ));
        }
        Some(ControlOp::Adopt { token }) => {
            if let Err(error) = run.lease.adopt(&token, "client") {
                run.log
                    .append(
                        now,
                        action,
                        Outcome::Rejected {
                            violations: vec![violation("ezsdr.lease", format!("RS-24: {error}"))],
                        },
                    )
                    .map_err(|error| super::RunHandleError::Malformed { error })?;
                return Ok(run.log.entries().last().expect("entry appended").clone());
            }
        }
        Some(ControlOp::Renew) => {
            if let Err(error) = run.lease.renew(&*run.shared.ctx.host_clock) {
                run.log
                    .append(
                        now,
                        action,
                        Outcome::Rejected {
                            violations: vec![violation("ezsdr.lease", format!("RS-24: {error}"))],
                        },
                    )
                    .map_err(|error| super::RunHandleError::Malformed { error })?;
                return Ok(run.log.entries().last().expect("entry appended").clone());
            }
        }
        Some(ControlOp::RunChild { .. }) => unreachable!("refused above"),
        None => {}
    }

    let mut configuration = lock(&run.shared.configuration).clone();
    let mut admitted = Vec::with_capacity(compiled.actions.len());
    let mut violations = Vec::new();
    let mut all_coercions = compiled.coercions.clone();
    let mut all_warnings = Vec::new();
    for (index, action) in compiled.actions.into_iter().enumerate() {
        let incoming = if index == 0 {
            compiled.coercions.as_slice()
        } else {
            &[]
        };
        match super::admission::admit_with(
            &run.shared,
            action,
            super::state::Origin::Session,
            incoming,
            &configuration,
        ) {
            Ok(action) => {
                if let Action::UpdateParameter { key, value, .. } = &action.action {
                    configuration
                        .entry(action.fragment.clone())
                        .or_default()
                        .insert(key.clone(), value.clone());
                }
                for coercion in &action.coercions {
                    if !all_coercions.contains(coercion) {
                        all_coercions.push(coercion.clone());
                    }
                }
                for warning in &action.warnings {
                    if !all_warnings.contains(warning) {
                        all_warnings.push(warning.clone());
                    }
                }
                admitted.push(action);
            }
            Err(mut action_violations) => {
                violations.append(&mut action_violations);
            }
        }
    }

    if !violations.is_empty() {
        run.log
            .append(now, log_action, Outcome::Rejected { violations })
            .map_err(|error| super::RunHandleError::Malformed { error })?;
        return Ok(run.log.entries().last().expect("entry appended").clone());
    }

    let dispatched = admitted
        .into_iter()
        .map(|action| super::admission::dispatch(&run.shared, action))
        .collect();
    run.log
        .append(
            now,
            action.clone(),
            Outcome::Admitted {
                coercions: all_coercions,
                warnings: all_warnings,
                dispatched,
            },
        )
        .map_err(|error| super::RunHandleError::Malformed { error })?;
    let entry = run.log.entries().last().expect("entry appended").clone();

    if let Some((termination, mode)) = end_after_admission {
        super::ending::request(&run.shared, termination, mode, None);
    } else {
        super::stepping::round(&run.shared, now, false);
        run.check_lease();
    }
    run.settle();
    Ok(entry)
}

fn session_earliest(
    shared: &Shared,
    action: &SessionAction,
    now: TimePoint,
) -> Result<TimePoint, Violation> {
    if let SessionAction::Vocabulary { ns, verb, .. } = action {
        if shared.ctx.registry.verb(ns, verb).is_some_and(|decl| {
            matches!(
                decl.compiles_to,
                crate::module_api::CompileRule::UpdateParameter { .. }
            )
        }) {
            return Ok(now);
        }
    }
    let target = match action {
        SessionAction::SetParameter { target, .. } | SessionAction::Vocabulary { target, .. } => {
            Some(target)
        }
        SessionAction::Stop { target } => target.as_ref(),
        SessionAction::Release {}
        | SessionAction::Adopt { .. }
        | SessionAction::Renew {}
        | SessionAction::RunChild { .. } => None,
    };
    let Some(target) = target else {
        return Ok(now);
    };
    let Ok((_, Inst::Provider(index), _)) = rewrite_spec_target(shared, target) else {
        return Ok(now);
    };
    let Some(lead) = shared.providers[index].lead else {
        return Ok(now);
    };
    let ticks =
        super::state::ceil_rescale(&shared.ctx.clocks, lead, shared.primary).map_err(|error| {
            violation(
                "ezsdr.time",
                format!("KC-28: cannot resolve Provider lead: {error}"),
            )
        })?;
    let ticks = now.ticks.checked_add(ticks).ok_or_else(|| {
        violation(
            "ezsdr.time",
            "KC-28: Provider lead overflows the primary root",
        )
    })?;
    Ok(TimePoint::new(shared.primary, ticks))
}

fn violation(check: &str, reason: impl Into<String>) -> Violation {
    Violation {
        check: Namespace::parse(check).expect("valid Session check"),
        key: None,
        requested: None,
        reason: reason.into(),
    }
}

struct Submitter {
    shared: std::sync::Weak<Shared>,
}

impl ActionSubmitter for Submitter {
    fn submit(&self, action: Action) -> Result<ActionId, Vec<Violation>> {
        let Some(shared) = self.shared.upgrade() else {
            return Err(vec![Violation {
                check: Namespace::parse("ezsdr.run_state").expect("a valid Kernel check"),
                key: None,
                requested: None,
                reason: "KC-24: the Run has ended".to_owned(),
            }]);
        };
        if let Action::Abort { cause } = action {
            super::ending::request(
                &shared,
                crate::run::Termination::Stopped { cause },
                crate::run::CleanupMode::Abort,
                None,
            );
            return Ok(ActionId(0));
        }
        super::admission::admit(&shared, action, super::state::Origin::Module)
            .map(|admitted| super::admission::dispatch(&shared, admitted))
    }
}

fn module_event_kinds(shared: &Shared, module: &ModuleRef) -> Vec<EventKind> {
    let Some(descriptor) = shared
        .ctx
        .registry
        .modules()
        .find(|d| d.id == module.id && d.version == module.version)
    else {
        return Vec::new();
    };
    descriptor
        .vocabularies
        .iter()
        .filter_map(|requirement| shared.ctx.registry.vocabulary(&requirement.id))
        .flat_map(|vocabulary| vocabulary.event_kinds.iter().map(|decl| decl.kind.clone()))
        .collect()
}

fn fragment_owner(shared: &Shared, component: &Ident) -> Option<Ident> {
    let routing = shared.routing()?;
    if shared.ctx.spec.resources.contains_key(component) || shared.ctx.outputs.contains(component) {
        return routing
            .fragment_of
            .contains_key(component)
            .then(|| component.clone());
    }
    routing.island_of.get(component).cloned()
}

fn rewrite_template(
    shared: &Shared,
    template: &ActionTemplate,
) -> Result<Option<(ResourceId, Inst, Ident)>, String> {
    let target = template.target().cloned();
    let Some(target) = target else {
        return Ok(None);
    };
    let (rewritten, instance, fragment) = rewrite_spec_target(shared, &target)?;
    Ok(Some((rewritten, instance, fragment)))
}

pub(super) fn rewrite_spec_target(
    shared: &Shared,
    target: &ResourceId,
) -> Result<(ResourceId, Inst, Ident), String> {
    let routing = shared.routing().expect("plan was installed");
    let segments: Vec<_> = target.segments().collect();
    let first = segments.first().copied().unwrap_or_default();
    if first == "sink" {
        if let Some(output) = segments.get(1).and_then(|s| Ident::parse(s).ok()) {
            if let Some(instance) = routing.fragment_of.get(&output).copied() {
                return Ok((target.clone(), instance, output));
            }
        }
    } else if let Ok(resource) = Ident::parse(first) {
        if let (Some(node), Some(instance)) = (
            routing.matched.get(&resource),
            routing.fragment_of.get(&resource).copied(),
        ) {
            let suffix = segments
                .iter()
                .skip(1)
                .copied()
                .collect::<Vec<_>>()
                .join("/");
            let path = if suffix.is_empty() {
                node.path.clone()
            } else {
                format!("{}/{suffix}", node.path)
            };
            let rewritten = ResourceId::parse(&path)
                .map_err(|_| format!("KC-23: {target} names no resource, output or component"))?;
            return Ok((rewritten, instance, resource));
        }
    }
    if let Ok(component) = Ident::parse(first) {
        if shared.ctx.spec.graph.components.contains_key(&component) {
            if let Some(fragment) = routing.island_of.get(&component) {
                if let Some(instance) = routing.fragment_of.get(fragment).copied() {
                    return Ok((target.clone(), instance, fragment.clone()));
                }
            }
        }
    }
    Err(format!(
        "KC-23: {target} names no resource, output or component"
    ))
}

pub(super) fn rewrite_action(action: &Action, target: ResourceId) -> Action {
    let mut action = action.clone();
    match &mut action {
        Action::TxBurst { target: t, .. }
        | Action::SetTimer { target: t, .. }
        | Action::UpdateParameter { target: t, .. }
        | Action::PeripheralCommand { target: t, .. }
        | Action::Emit { target: t, .. } => *t = target,
        Action::Stop { target: Some(t) } => *t = target,
        Action::Stop { target: None } | Action::Abort { .. } => {}
    }
    action
}

pub(super) fn emit_device_lost(shared: &Shared, instance: Inst, error: &ModuleError) {
    if error.kind != ModuleErrorKind::DeviceLost {
        return;
    }
    let Some(collector) = shared.collector.get() else {
        return;
    };
    let _ = collector.emit_control(Event {
        source: shared.source_root(instance),
        time: shared.now(),
        severity: Severity::Fatal,
        kind: EventKind::parse(EventKind::DEVICE_LOST).expect("Kernel event kind"),
        payload: serde_json::json!({ "message": error.message }),
    });
}

fn build_routing(
    plan: ExecutionPlan,
    matched: BTreeMap<Ident, ResourceId>,
    shared: &Shared,
) -> Routing {
    let mut fragment_of = BTreeMap::new();
    let mut first_fragment = BTreeMap::new();
    let mut order = Vec::new();
    let mut island_of = BTreeMap::new();
    for fragment in &plan.fragments {
        let instance = match fragment.role {
            crate::module_api::Role::Provider => shared
                .providers
                .iter()
                .position(|slot| slot.names.contains(&fragment.id))
                .map(Inst::Provider),
            crate::module_api::Role::Sink => shared
                .sinks
                .iter()
                .position(|slot| slot.output == fragment.id)
                .map(Inst::Sink),
            crate::module_api::Role::Executor => {
                serde_json::from_value::<crate::module_api::IslandDecl>(fragment.content.clone())
                    .ok()
                    .and_then(|island| {
                        for component in island.components {
                            island_of.insert(component, fragment.id.clone());
                        }
                        shared
                            .executors
                            .iter()
                            .position(|slot| slot.name == island.executor)
                            .map(Inst::Executor)
                    })
            }
            crate::module_api::Role::Authority | crate::module_api::Role::Link => None,
        };
        if let Some(instance) = instance {
            fragment_of.insert(fragment.id.clone(), instance);
            if let Entry::Vacant(entry) = first_fragment.entry(instance) {
                entry.insert(fragment.id.clone());
                order.push(instance);
            }
        }
    }
    let reverse = plan.fragments.iter().rev().map(|f| f.id.clone()).collect();
    Routing {
        plan,
        matched,
        fragment_of,
        first_fragment,
        order,
        island_of,
        reverse,
    }
}
