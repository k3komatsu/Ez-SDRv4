//! Ez-SDR v4 Module `ezsdr.exec.native` 1.0.0: the step-driven, in-process Executor
//! (design/14-native-executor.md, rules `NX-n`).
//!
//! It loads each component of its Islands by the `impl` identity the Spec's
//! `ComponentDescriptor` names, from the compiled-in implementations the runtime hands
//! it, and steps them in component-id order inside its own `step`. How a component is
//! called is this crate's [`Component`] trait: the execution ABI is the Executor's,
//! never the Kernel's (MA-21).
#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::collections::BTreeMap;
use std::sync::Arc;

use ezsdr_kernel::event::{Action, EventSink};
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::{MemoryDomainId, ModuleId, ResourceId};
use ezsdr_kernel::module_api::{
    ActionReceiver, ActionSubmitter, AttachedPort, ComponentDescriptor, Deployment,
    ExecutionClass, Executor, ExecutorDescriptor, InputStore, IslandDecl, KERNEL_API,
    ModuleDescriptor, ModuleError, ModuleErrorKind, ModuleRef, PrepareContext, Role,
    StepOutcome, StopMode, Version,
};
use ezsdr_kernel::plan::PrepareReport;
use ezsdr_kernel::spec::{Ident, Namespace};
use ezsdr_kernel::time::{ClockRegistry, TimeAuthority, TimePoint};

/// The Module id (NX-1).
pub const MODULE_ID: &str = "ezsdr.exec.native";
/// The Executor kind a component's `requires.executor_kind` names (NX-2).
pub const EXECUTOR_KIND: &str = "ezsdr.exec.native";
/// The one implementation kind this Executor loads (NX-2).
pub const IMPL_KIND: &str = "ezsdr.impl.native";

/// The admission checks that refuse an Action because the Run is ending, not because the
/// Action is wrong: dispatch frozen (RS-6 step 1) and not `Running` (RS-18) (KC-24).
const ENDING: [&str; 2] = ["ezsdr.dispatch", "ezsdr.run_state"];

fn module_ref() -> ModuleRef {
    ModuleRef {
        id: ModuleId::parse(MODULE_ID).expect("a valid Module id"),
        version: Version::new(1, 0, 0),
    }
}

/// The Module descriptor for `ezsdr.exec.native` 1.0.0 (NX-1).
pub fn descriptor() -> ModuleDescriptor {
    ModuleDescriptor {
        id: ModuleId::parse(MODULE_ID).expect("a valid Module id"),
        version: Version::new(1, 0, 0),
        kernel_api: KERNEL_API,
        roles: vec![Role::Executor],
        vocabularies: Vec::new(),
        deployment: Deployment::InProcess {},
        impl_hash: Some(ContentHash::of_bytes(b"ezsdr.exec.native 1.0.0")),
    }
}

/// What the Executor declares: its kind, host memory and the native implementation
/// kind (NX-2, MA-18).
pub fn executor_descriptor() -> ExecutorDescriptor {
    ExecutorDescriptor {
        module: module_ref(),
        kind: Namespace::parse(EXECUTOR_KIND).expect("a valid Executor kind"),
        // HD-1's HOST_MEMORY; a Module crate depends on no other Module crate (MA-3).
        memory_domains: vec![MemoryDomainId::local(0)],
        impl_kinds: vec![Namespace::parse(IMPL_KIND).expect("a valid implementation kind")],
        capabilities: BTreeMap::new(),
    }
}

/// One component this Executor runs: the execution ABI the Executor owns (MA-21, NX-4).
///
/// A component is stepped under MA-20's contract, as a whole Executor is: `step` consumes
/// every input at or before `until`, emits nothing after it, never blocks and never
/// calls `wait_until`, and reports `progressed` exactly when it consumed or produced
/// something. It does not submit Actions itself: it pushes the ones it decides onto
/// `out`, in order, and the Executor submits them (NX-6), so that every component's
/// refusals are handled one way.
pub trait Component: Send {
    /// Takes the component's descriptor, its ports' link ends and the Run's handles.
    fn prepare(&mut self, ctx: ComponentContext) -> Result<(), ModuleError>;
    /// Runs the component up to `until`, pushing the Actions it decides onto `out`
    /// (MA-20, NX-6).
    fn step(&mut self, until: TimePoint, out: &mut Vec<Action>) -> Result<StepOutcome, ModuleError>;
    /// Stops; the default does nothing.
    fn stop(&mut self, _mode: StopMode) -> Result<(), ModuleError> {
        Ok(())
    }
    /// Releases every handle; infallible and idempotent (MA-7). The default does nothing.
    fn cleanup(&mut self) {}
}

/// What one component is handed at `prepare`: its own descriptor, the link ends bound
/// to its own ports, and the Run's handles (NX-4).
pub struct ComponentContext {
    /// The descriptor the Spec's graph carries, parameters and all (MA-36).
    pub descriptor: ComponentDescriptor,
    /// The Run's class (MA-41).
    pub class: ExecutionClass,
    /// The Run's single source of "now" (TM-16a).
    pub time: Arc<dyn TimeAuthority>,
    /// The node's clock registry (TM-13a).
    pub clocks: Arc<ClockRegistry>,
    /// Where to emit events (RS-31); the source is [`ComponentContext::source`].
    pub events: Arc<dyn EventSink>,
    /// The Run's inputs by content hash (RS-44a, MA-5a).
    pub inputs: Arc<dyn InputStore>,
    /// The link ends bound to this component's ports (MA-27).
    pub links: Vec<AttachedPort>,
    /// The Island's event source root, `island_<n>` (KC-8, KC-30).
    pub source: ResourceId,
}

/// A compiled-in component implementation, found by the `impl.id` a descriptor names
/// and checked against its `impl.hash` (NX-3, MA-19b).
#[derive(Clone)]
pub struct Implementation {
    /// Its identity within [`IMPL_KIND`] (MA-19b).
    pub id: String,
    /// The hash a Spec must name for it (RS-45).
    pub hash: ContentHash,
    /// Builds a fresh instance.
    pub make: fn() -> Box<dyn Component>,
}

/// The `ezsdr.exec.native` Executor (NX-1…NX-7).
pub struct NativeExecutor {
    descriptor: ExecutorDescriptor,
    implementations: BTreeMap<String, Implementation>,
    components: BTreeMap<Ident, Box<dyn Component>>,
    actions: Option<Arc<dyn ActionReceiver>>,
    actions_out: Option<Arc<dyn ActionSubmitter>>,
}

impl NativeExecutor {
    /// An Executor that can load exactly `implementations`; a second implementation
    /// with one id is refused, because which one a Spec meant would be undecidable
    /// (NX-3).
    pub fn new(implementations: Vec<Implementation>) -> Result<NativeExecutor, ModuleError> {
        let mut table = BTreeMap::new();
        for implementation in implementations {
            let id = implementation.id.clone();
            if table.insert(id.clone(), implementation).is_some() {
                return Err(ModuleError::rejected(format!(
                    "NX-3: two implementations are registered as {id}"
                )));
            }
        }
        Ok(NativeExecutor {
            descriptor: executor_descriptor(),
            implementations: table,
            components: BTreeMap::new(),
            actions: None,
            actions_out: None,
        })
    }
}

impl Executor for NativeExecutor {
    fn descriptor(&self) -> &ExecutorDescriptor {
        &self.descriptor
    }

    fn prepare(
        &mut self,
        island: &IslandDecl,
        ctx: PrepareContext,
    ) -> Result<PrepareReport, ModuleError> {
        if ctx.class != ExecutionClass::Simulation {
            return Err(ModuleError {
                kind: ModuleErrorKind::Unsupported,
                message: format!(
                    "NX-4: ezsdr.exec.native 1.0.0 steps its components and runs the Simulation class only, not {:?}",
                    ctx.class
                ),
                detail: serde_json::Value::Null,
            });
        }
        let fragment = Ident::parse(&format!("island_{}", island.id.local))
            .map_err(|_| ModuleError::rejected("NX-4: the Island id makes no fragment name"))?;
        let source = ResourceId::parse(fragment.as_str())
            .map_err(|_| ModuleError::rejected("NX-4: the Island id makes no event source"))?;
        for link in &ctx.links {
            if !island.components.iter().any(|entry| entry.component == link.component) {
                return Err(ModuleError::rejected(format!(
                    "NX-4: a link end names {}, which is not in this Island",
                    link.component
                )));
            }
        }
        for id in island.components.iter().map(|entry| &entry.component) {
            if self.components.contains_key(id) {
                return Err(ModuleError::rejected(format!(
                    "NX-4: component {id} is placed twice on this Executor"
                )));
            }
            let descriptor = ctx.components.get(id).cloned().ok_or_else(|| {
                ModuleError::rejected(format!("NX-4: no descriptor for component {id}"))
            })?;
            let make = self.load(&descriptor)?.make;
            // Kept before its `prepare` runs, so that `stop` and `cleanup` reach a component
            // whose `prepare` failed as MA-7 has them reach every instance that reached it.
            let component = self.components.entry(id.clone()).or_insert_with(make);
            component.prepare(ComponentContext {
                descriptor,
                class: ctx.class,
                time: ctx.time.clone(),
                clocks: ctx.clocks.clone(),
                events: ctx.events.clone(),
                inputs: ctx.inputs.clone(),
                links: ctx.links.iter().filter(|l| &l.component == id).cloned().collect(),
                source: source.clone(),
            })?;
        }
        self.actions = Some(ctx.actions.clone());
        self.actions_out = Some(ctx.actions_out.clone());
        Ok(PrepareReport {
            fragment,
            effective: BTreeMap::new(),
            coercions: Vec::new(),
            warnings: Vec::new(),
        })
    }

    fn arm(&mut self) -> Result<(), ModuleError> {
        Ok(())
    }

    fn start(&mut self) -> Result<(), ModuleError> {
        Ok(())
    }

    fn step(&mut self, until: TimePoint) -> Result<StepOutcome, ModuleError> {
        if let Some(actions) = &self.actions {
            if let Some(action) = actions.recv() {
                return Err(ModuleError::rejected(format!(
                    "NX-7: ezsdr.exec.native 1.0.0 applies no Action, and {} for {} reached it",
                    action_name(&action),
                    action.target().map_or_else(|| "the Run".to_owned(), ToString::to_string),
                )));
            }
        }
        let mut progressed = false;
        let mut out = Vec::new();
        for (id, component) in &mut self.components {
            progressed |= component.step(until, &mut out)?.progressed;
            progressed |= !out.is_empty();
            for action in out.drain(..) {
                let submitter = self.actions_out.as_ref().expect("an Executor is prepared before it is stepped");
                match submitter.submit(action.clone()) {
                    Ok(_) => {}
                    // KC-24 steps 1 and 2: the Run is ending, so the decision came too late
                    // to act on, and the termination already says why. Turning it into a
                    // step error would record a clean stop as an abort (KC-32, KE-3).
                    Err(violations)
                        if !violations.is_empty()
                            && violations.iter().all(|v| ENDING.contains(&v.check.as_str())) => {}
                    Err(violations) => {
                        return Err(ModuleError::rejected(format!(
                            "NX-6: component {id}'s {} was refused: {}",
                            action_name(&action),
                            violations.iter().map(|v| format!("{}: {}", v.check, v.reason)).collect::<Vec<_>>().join("; ")
                        )));
                    }
                }
            }
        }
        Ok(StepOutcome { progressed })
    }

    fn stop(&mut self, mode: StopMode) -> Result<(), ModuleError> {
        let mut first = None;
        for component in self.components.values_mut() {
            if let Err(error) = component.stop(mode) {
                first.get_or_insert(error);
            }
        }
        first.map_or(Ok(()), Err)
    }

    fn cleanup(&mut self) {
        for component in self.components.values_mut() {
            component.cleanup();
        }
        self.components.clear();
        self.actions = None;
        self.actions_out = None;
    }
}

impl NativeExecutor {
    /// MA-19b: the implementation a descriptor's `impl` names — its kind, its id among
    /// the compiled-in ones, and the hash the Spec recorded for it (NX-3).
    fn load(&self, descriptor: &ComponentDescriptor) -> Result<&Implementation, ModuleError> {
        let named = &descriptor.implementation;
        if named.kind.as_str() != IMPL_KIND {
            return Err(ModuleError::rejected(format!(
                "NX-3: component {} is a {} implementation, and this Executor loads {IMPL_KIND}",
                descriptor.id, named.kind
            )));
        }
        let implementation = self.implementations.get(&named.id).ok_or_else(|| {
            ModuleError::rejected(format!(
                "NX-3: component {} names implementation {}, which this Executor does not have",
                descriptor.id, named.id
            ))
        })?;
        if implementation.hash != named.hash {
            return Err(ModuleError::rejected(format!(
                "NX-3: component {} names {} with hash {}, and this Executor's is {}",
                descriptor.id, named.id, named.hash, implementation.hash
            )));
        }
        Ok(implementation)
    }
}

fn action_name(action: &Action) -> &'static str {
    match action {
        Action::TxBurst { .. } => "TxBurst",
        Action::SetTimer { .. } => "SetTimer",
        Action::UpdateParameter { .. } => "UpdateParameter",
        Action::Command { .. } => "Command",
        Action::Emit { .. } => "Emit",
        Action::Stop { .. } => "Stop",
        Action::Abort { .. } => "Abort",
    }
}
