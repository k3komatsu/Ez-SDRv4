//! The test-double Provider, the recording TestExecutor and the fake host clock
//! (OV-20, OV-21, MA-44, MA-45). Never compiled into `src/`.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use ezsdr_kernel::binding::{AdmissionCheck, CheckStage, Violation};
use ezsdr_kernel::contract::{DataContractId, Port, PortDirection};
use ezsdr_kernel::event::{Action, ActionId, EventKind};
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::{ClockDomainId, MemoryDomainId, ModuleId, ResourceId};
use ezsdr_kernel::manifest::ArtifactRef;
use ezsdr_kernel::module_api::{
    ActionReceiver, ActionSubmitter, CoerceReport, CompileRule, ComponentDescriptor,
    ComponentImpl, ComponentKind, ComponentRequires, ComponentTiming, Deployment, Driving,
    Executor, ExecutorDescriptor, Fidelity, IslandDecl, LinkDescriptor, ModuleDescriptor, ModuleError,
    ModuleErrorKind, ModuleRef, ParamDecl, PrepareContext, Provider, ProviderInstance, Requested,
    Resource, Role, Sink, SinkDescriptor, StepOutcome, StopMode, UpdateClass, VerbDecl, Version,
    VocabularyDescriptor,
};
use ezsdr_kernel::plan::{Fragment, PrepareReport};
use ezsdr_kernel::policy::{EventKindDecl, Reaction};
use ezsdr_kernel::run::{CleanupMode, CleanupOps, CleanupStep, HostClock};
use ezsdr_kernel::spec::{
    CapabilityValue, Coercion, CoercionPolicy, Constraint, Ident, Key, KeyDecl, Namespace, Value,
    ValueKind, Warning,
};
use ezsdr_kernel::time::TimePoint;

/// Parses an [`Ident`] from a literal known to be well formed.
pub fn id(s: &str) -> Ident {
    Ident::parse(s).expect("a well-formed Ident literal")
}

/// Parses a [`Namespace`] from a literal known to be well formed.
pub fn ns(s: &str) -> Namespace {
    Namespace::parse(s).expect("a well-formed Namespace literal")
}

/// Parses a [`Key`] from a literal known to be well formed.
pub fn key(s: &str) -> Key {
    Key::parse(s).expect("a well-formed Key literal")
}

/// Parses a [`ResourceId`] from a literal known to be well formed.
pub fn rid(s: &str) -> ResourceId {
    ResourceId::parse(s).expect("a well-formed resource path literal")
}

/// Parses a [`ModuleId`] from a literal known to be well formed.
pub fn mid(s: &str) -> ModuleId {
    ModuleId::parse(s).expect("a well-formed ModuleId literal")
}

/// The exact version every test Module registers, as a binding pins it (SB-22, D78).
pub fn mref(s: &str) -> ModuleRef {
    ModuleRef { id: mid(s), version: Version::new(1, 0, 0) }
}

/// A placeholder content hash for descriptors that need one (MA-37).
pub fn some_hash(seed: &str) -> ContentHash {
    ContentHash::of_bytes(seed.as_bytes())
}

// ---------------------------------------------------------------- the test Vocabulary

/// The `test` Vocabulary of MA-44 and OV-21: `test.count`, `test.grid` (coercible)
/// and `test.flag`. It uses no radio word, which is how the matcher is proven
/// generic.
pub fn test_vocabulary() -> VocabularyDescriptor {
    VocabularyDescriptor {
        id: ns("test"),
        version: Version::new(1, 0, 0),
        prefix: ns("test"),
        keys: vec![
            KeyDecl {
                key: key("test.count"),
                kind: ValueKind::Int,
                coercible: false,
                coercion_default: CoercionPolicy::Reject,
                update_class: None,
            },
            KeyDecl {
                key: key("test.grid"),
                kind: ValueKind::Num,
                coercible: true,
                coercion_default: CoercionPolicy::Reject,
                update_class: None,
            },
            KeyDecl {
                key: key("test.flag"),
                kind: ValueKind::Bool,
                coercible: false,
                coercion_default: CoercionPolicy::Warn,
                update_class: None,
            },
            // A Provider parameter that *is* changeable during a Run, which is what
            // SB-2's `update_class` exists for: the Vocabulary declares the class,
            // because a Provider's parameters are Vocabulary keys and never a
            // component's `params` (Vision §27's TX gain; finding D33).
            KeyDecl {
                key: key("test.gain"),
                kind: ValueKind::Num,
                coercible: false,
                coercion_default: CoercionPolicy::Reject,
                update_class: Some(ezsdr_kernel::module_api::UpdateClass::HardwareTimed),
            },
        ],
        event_kinds: vec![EventKindDecl {
            kind: EventKind::parse("test.custom").expect("a valid kind"),
            default: Reaction::Continue,
            severity: ezsdr_kernel::event::Severity::Info,
        }],
        verbs: vec![
            VerbDecl {
                verb: id("capture"),
                compiles_to: CompileRule::UpdateParameter {
                    key: key("test.capture"),
                    class: UpdateClass::BlockBoundary,
                },
            },
            VerbDecl {
                verb: id("start_repeat"),
                compiles_to: CompileRule::TxBurst {
                    repeat: true,
                    late_policy: ezsdr_kernel::stream::LatePolicy::SendAsapAndFlag,
                },
            },
            VerbDecl { verb: id("sweep"), compiles_to: CompileRule::PeripheralCommand {},
            },
        ],
        checks: vec![ns("test.limits")],
    }
}

/// The `test.limits` admission check: refuses a `test.grid` above its ceiling. It
/// is the generic stand-in for Vision §52's RF safety envelope (SB-29, decision B5).
pub struct TestLimitsCheck {
    section: Namespace,
    stages: Vec<CheckStage>,
}

impl Default for TestLimitsCheck {
    fn default() -> Self {
        TestLimitsCheck::new()
    }
}

impl TestLimitsCheck {
    /// A check that runs at all three points of SB-30.
    pub fn new() -> TestLimitsCheck {
        TestLimitsCheck {
            section: ns("test.limits"),
            stages: vec![CheckStage::Validate, CheckStage::Prepare, CheckStage::Runtime,
            ],
        }
    }
}

impl AdmissionCheck for TestLimitsCheck {
    fn section(&self) -> &Namespace {
        &self.section
    }

    fn stages(&self) -> &[CheckStage] {
        &self.stages
    }

    fn check(
        &self,
        section: &serde_json::Value,
        effective: &BTreeMap<Key, Value>,
        proposed: &BTreeMap<Key, Value>,
        _stage: CheckStage,
    ) -> Vec<Violation> {
        let ceiling = section.get("max_grid").and_then(|v| v.as_f64()).unwrap_or(f64::INFINITY);
        // SB-30: the effective configuration overlaid with what is proposed.
        let mut merged = effective.clone();
        merged.extend(proposed.iter().map(|(k, v)| (k.clone(), v.clone())));
        merged
            .iter()
            .filter(|(k, _)| k.as_str() == "test.grid")
            .filter_map(|(k, v)| {
                let n = match v {
                    Value::Num(n) => *n,
                    Value::Int(i) => *i as f64,
                    _ => return None,
                };
                (n > ceiling).then(|| Violation {
                    check: ns("test.limits"),
                    key: Some(k.clone()),
                    requested: Some(v.clone()),
                    reason: format!("test.grid {n} exceeds the declared ceiling {ceiling}"),
                })
            })
            .collect()
    }
}

// ---------------------------------------------------------------- the Provider double

/// Which lifecycle phase the double should fail at (MA-44).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FailAt {
    /// Never fail.
    Never,
    /// Fail in `prepare`.
    Prepare,
    /// Fail in `arm`.
    Arm,
    /// Fail in `start`.
    Start,
    /// Fail in `stop`.
    Stop,
}

/// The Phase 1 test-double Provider (MA-44, OV-21).
///
/// It uses no radio word. It has no time model, no blocks and no envelope: that
/// boundary is exactly where Phase 2's MockRadio begins.
pub struct TestProvider {
    instance: ProviderInstance,
    /// What the `test.grid` key snaps to; `None` means it does not coerce.
    pub grid: Option<f64>,
    /// Whether `coerce` reports a coercion of a key the request does not name (SB-44).
    pub stray_coercion: bool,
    /// Which phase to fail at (MA-44).
    pub fail_at: FailAt,
    /// The recorded call log (MA-44).
    pub log: Mutex<Vec<String>>,
    /// How many times `coerce` was called (MA-11).
    pub coerce_calls: AtomicU64,
    /// The last request `coerce` was handed, so a test can assert SB-7's "one call
    /// with the whole map" rather than only the call count (SB-7, SB-44).
    pub last_request: Mutex<Option<Requested>>,
    /// The Actions the coordinator dispatched to it (MA-14).
    pub actions: Mutex<Vec<Action>>,
    /// Makes `prepare` report something `coerce` did not, so that MA-12's equality
    /// has a negative case.
    pub prepare_disagrees: bool,
}

impl TestProvider {
    /// A device with two `test.line` sub-resources and the declared capabilities
    /// (MA-44, SB-33).
    pub fn new(path: &str, count: i64) -> TestProvider {
        let root = rid(path);
        let line = |root: &ResourceId, n: u32| Resource {
            id: root.child(&n.to_string()).expect("a valid child path"),
            kind: ns("test.line"),
            capabilities: [(key("test.count"), CapabilityValue::One { value: Value::Int(count),
                },
            )]
                .into_iter()
                .collect(),
            children: Vec::new(),
            // A line is one physical channel: exclusive, like SB-34's default.
            shareable: false,
            // A stream endpoint declares its Ports, so a Spec can link this node to
            // a component (MA-10, SB-15). A **line** carries `sc16` while the device
            // root below carries `cf32` under the same port name, so a test can tell
            // whether SB-15 resolved the port on the bound node or on some other.
            ports: vec![
                Port {
                    name: "rx".to_owned(),
                    direction: PortDirection::Out,
                    contract: DataContractId::parse("ezsdr.stream.sc16")
                        .expect("a valid literal"),
                },
                // The TX end of Vision §7's `PHY Processor -> Radio Port`: a link
                // whose **consumer** is a resource port, which is the case the plan's
                // `links` resolved to a default contract rather than to this one.
                Port {
                    name: "tx".to_owned(),
                    direction: PortDirection::In,
                    contract: DataContractId::parse("ezsdr.stream.sc16")
                        .expect("a valid literal"),
                },
            ],
        };
        TestProvider {
            instance: ProviderInstance {
                id: root.clone(),
                module: ModuleRef { id: mid("ezsdr.test.provider"), version: Version::new(1, 0, 0),
                },
                profile: None,
                tree: Resource {
                    id: root.clone(),
                    kind: ns("test.device"),
                    capabilities: [
                        (key("test.count"), CapabilityValue::One { value: Value::Int(count),
                            },
                        ),
                        // A grid is a discrete set: a declared continuous range
                        // could not express "multiples of 20", so SB-7's coercion
                        // path would never be reached.
                        (
                            key("test.grid"),
                            CapabilityValue::AnyOf {
                                values: vec![Value::Num(20.0), Value::Num(40.0), Value::Num(100.0)],
                            },
                        ),
                        (
                            key("test.flag"),
                            CapabilityValue::AnyOf {
                                values: vec![Value::Bool(true), Value::Bool(false)],
                            },
                        ),
                    ]
                    .into_iter()
                    .collect(),
                    children: vec![line(&root, 0), line(&root, 1)],
                    shareable: false,
                    // The device-level stream endpoint, which a Session's implicit
                    // Spec binds because RS-12 takes the instance's root kind.
                    ports: vec![Port {
                        name: "rx".to_owned(),
                        direction: PortDirection::Out,
                        contract: DataContractId::parse("ezsdr.stream.cf32")
                            .expect("a valid literal"),
                    }],
                },
                fidelity: Fidelity::NONE,
                driving: Driving { stepped: false },
                arm_after: Vec::new(),
                sections: BTreeMap::new(),
            },
            grid: None,
            stray_coercion: false,
            fail_at: FailAt::Never,
            log: Mutex::new(Vec::new()),
            coerce_calls: AtomicU64::new(0),
            last_request: Mutex::new(None),
            actions: Mutex::new(Vec::new()),
            prepare_disagrees: false,
        }
    }

    /// Declares both `test.line` sub-resources shareable, so that more than one
    /// Spec resource may bind to one of them (MA-10, SB-34).
    pub fn with_shareable_lines(mut self) -> TestProvider {
        for line in &mut self.instance.tree.children {
            line.shareable = true;
        }
        self
    }

    /// Gives the two `test.line` sub-resources different `test.count` values, so
    /// that a request only the second can satisfy has a second to find (SB-34).
    pub fn with_asymmetric_lines(mut self, first: i64, second: i64) -> TestProvider {
        for (i, child) in self.instance.tree.children.iter_mut().enumerate() {
            let n = if i == 0 { first } else { second };
            child
                .capabilities
                .insert(key("test.count"), CapabilityValue::One { value: Value::Int(n),
                },
            );
        }
        self
    }

    /// Makes `coerce` report a coercion of `test.flag`, which no fixture requests:
    /// SB-44's malformed report, from the `validate` side (MA-44, SB-44).
    pub fn with_stray_coercion(mut self) -> TestProvider {
        self.stray_coercion = true;
        self
    }

    /// Makes `test.grid` snap to a multiple of `step` (MA-44, SB-7).
    pub fn with_grid(mut self, step: f64) -> TestProvider {
        self.grid = Some(step);
        self
    }

    /// Declares a fidelity other than all-`none` (MA-42).
    pub fn with_fidelity(mut self, fidelity: Fidelity) -> TestProvider {
        self.instance.fidelity = fidelity;
        self
    }

    /// Reports another instance id than its root's (X7, D91).
    pub fn with_instance_id(mut self, id: ResourceId) -> TestProvider {
        self.instance.id = id;
        self
    }

    /// Reports itself as another Module version than the one registered (SB-22, D78).
    pub fn with_module(mut self, module: ModuleRef) -> TestProvider {
        self.instance.module = module;
        self
    }

    /// Fails at the named phase (MA-44).
    pub fn failing_at(mut self, phase: FailAt) -> TestProvider {
        self.fail_at = phase;
        self
    }

    /// Declares that this instance must be armed after `other` (SB-39).
    pub fn arm_after(mut self, other: ResourceId) -> TestProvider {
        self.instance.arm_after.push(other);
        self
    }

    fn record(&self, what: &str) {
        self.log.lock().expect("lock").push(what.to_owned());
    }

    fn fail_if(&self, phase: FailAt) -> Result<(), ModuleError> {
        if self.fail_at == phase {
            return Err(ModuleError {
                kind: ModuleErrorKind::Rejected,
                message: format!("injected failure at {phase:?}"),
                detail: serde_json::Value::Null,
            });
        }
        Ok(())
    }

    /// The recorded call log (MA-44).
    pub fn calls(&self) -> Vec<String> {
        self.log.lock().expect("lock").clone()
    }

    /// The Actions that reached it; they arrive only after Kernel admission (MA-14).
    pub fn drain_actions(&self) -> Vec<Action> {
        std::mem::take(&mut *self.actions.lock().expect("lock"))
    }

    /// Hands the double an admitted Action, as the coordinator would (MA-14).
    pub fn deliver(&self, action: Action) {
        self.actions.lock().expect("lock").push(action);
    }
}

impl Provider for TestProvider {
    fn instance(&self) -> &ProviderInstance {
        &self.instance
    }

    fn coerce(&self, request: &Requested) -> Result<CoerceReport, ModuleError> {
        self.coerce_calls.fetch_add(1, Ordering::Relaxed);
        *self.last_request.lock().unwrap_or_else(|e| e.into_inner()) = Some(request.clone());
        let mut report = CoerceReport::default();
        if self.stray_coercion {
            report.coercions.push(Coercion {
                key: key("test.flag"),
                requested: Value::Bool(false),
                applied: Value::Bool(true),
                reason: "a key the request does not name".to_owned(),
            });
        }
        for (k, c) in &request.constraints {
            let Constraint::Eq { value: v } = c else { continue;
            };
            if let (true, Some(step), Value::Num(x)) = (k.as_str() == "test.grid", self.grid, v) {
                let snapped = (x / step).round() * step;
                report.applied.insert(k.clone(), Value::Num(snapped));
                if (snapped - x).abs() > f64::EPSILON {
                    report.coercions.push(Coercion {
                        key: k.clone(),
                        requested: v.clone(),
                        applied: Value::Num(snapped),
                        reason: format!("snapped to a multiple of {step}"),
                    });
                }
                continue;
            }
            report.applied.insert(k.clone(), v.clone());
        }
        Ok(report)
    }

    fn prepare(&mut self, f: &Fragment, _ctx: PrepareContext<'_>,
    ) -> Result<PrepareReport, ModuleError> {
        self.record("prepare");
        self.fail_if(FailAt::Prepare)?;
        // MA-12: the report's coercions equal what `coerce` returned for the same
        // request, which a Provider honours by replaying it rather than by
        // recomputing something similar.
        // SB-39: a Provider fragment's content is `{ selector, requested }`. The
        // request is what the matcher resolved; a Provider that had only the
        // selector could not honour MA-12 at all.
        let report = match serde_json::from_value::<Requested>(f.content["requested"].clone()) {
            Ok(request) => self.coerce(&request)?,
            Err(_) => CoerceReport::default(),
        };
        let coercions = if self.prepare_disagrees { Vec::new() } else { report.coercions };
        Ok(PrepareReport {
            fragment: f.id.clone(),
            effective: report.applied,
            coercions,
            warnings: Vec::<Warning>::new(),
        })
    }

    fn arm(&mut self) -> Result<(), ModuleError> {
        self.record("arm");
        self.fail_if(FailAt::Arm)
    }

    fn start(&mut self, _at: Option<TimePoint>) -> Result<(), ModuleError> {
        self.record("start");
        self.fail_if(FailAt::Start)
    }

    fn stop(&mut self, mode: StopMode) -> Result<(), ModuleError> {
        self.record(&format!("stop:{mode:?}"));
        self.fail_if(FailAt::Stop)
    }

    fn cleanup(&mut self) {
        self.record("cleanup");
    }
}

/// The `ModuleDescriptor` the double ships (MA-31).
pub fn test_provider_descriptor() -> ModuleDescriptor {
    ModuleDescriptor {
        id: mid("ezsdr.test.provider"),
        version: Version::new(1, 0, 0),
        kernel_api: Version::new(4, 0, 0),
        // MA-1: a Module may hold several roles. This double is the Run's Authority in
        // every fixture that plans, and SB-24 reads the role off the descriptor of the
        // Module the profile binds, as MA-25 does for a Sink.
        roles: vec![Role::Provider, Role::Authority],
        vocabularies: vec![ezsdr_kernel::module_api::VocabularyRequirement {
            id: ns("test"),
            req: ezsdr_kernel::module_api::VersionReq(Version::new(1, 0, 0)),
        }],
        deployment: Deployment::InProcess {},
        impl_hash: Some(some_hash("ezsdr.test.provider")),
    }
}

// ---------------------------------------------------------------- the Executor double

/// The recording TestExecutor of MA-45: about twenty lines, performing no
/// processing. Without it MA-19, MA-20, MA-30 and MA-39 have no runtime test until
/// Phase 2.
pub struct TestExecutor {
    descriptor: ExecutorDescriptor,
    /// The per-Island descriptors received at prepare (MA-19).
    pub prepared_components: Mutex<Vec<BTreeMap<Ident, ComponentDescriptor>>>,
    /// How many `step` calls it has seen, and at which instants (MA-30).
    pub steps: Mutex<Vec<TimePoint>>,
    /// How many further steps report `progressed` (MA-20, MA-30).
    pub progress_budget: Mutex<usize>,
}

impl TestExecutor {
    /// An Executor over the given memory domain (MA-18).
    pub fn new(domain: MemoryDomainId) -> TestExecutor {
        TestExecutor {
            descriptor: ExecutorDescriptor {
                module: mref("ezsdr.test.executor"),
                kind: ns("test.executor"),
                memory_domains: vec![domain],
                impl_kinds: vec![ns("test.impl")],
                capabilities: BTreeMap::new(),
            },
            prepared_components: Mutex::new(Vec::new()),
            steps: Mutex::new(Vec::new()),
            progress_budget: Mutex::new(0),
        }
    }

    /// Reports `progressed` for the next `n` steps, then quiesces (MA-30).
    pub fn progressing_for(self, n: usize) -> TestExecutor {
        *self.progress_budget.lock().expect("lock") = n;
        self
    }
}

impl Executor for TestExecutor {
    fn descriptor(&self) -> &ExecutorDescriptor {
        &self.descriptor
    }

    fn prepare(&mut self, island: &IslandDecl,
        ctx: PrepareContext<'_>,
    ) -> Result<PrepareReport, ModuleError> {
        self.prepared_components
            .lock()
            .expect("lock")
            .push(ctx.components);
        Ok(PrepareReport {
            fragment: island.executor.clone(),
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
        self.steps.lock().expect("lock").push(until);
        let mut budget = self.progress_budget.lock().expect("lock");
        let progressed = *budget > 0;
        *budget = budget.saturating_sub(1);
        Ok(StepOutcome { progressed })
    }

    fn stop(&mut self, _mode: StopMode) -> Result<(), ModuleError> {
        Ok(())
    }

    fn cleanup(&mut self) {}
}

// ---------------------------------------------------------------- the Sink double

/// A Sink double that keeps one artifact open, so MA-26 and RS-44 have something to
/// mark partial.
pub struct TestSink {
    descriptor: SinkDescriptor,
    /// The artifact it is writing (RS-44).
    pub artifact: ArtifactRef,
    /// Whether it was stopped under an abort (MA-26).
    pub aborted: Mutex<bool>,
}

impl TestSink {
    /// Reports itself as another Module version than the one registered (SB-22, D82).
    pub fn with_module(mut self, module: ModuleRef) -> TestSink {
        self.descriptor.module = module;
        self
    }

    /// Reads blocks from `domains` only (MA-25, D81).
    pub fn reading(mut self, domains: Vec<MemoryDomainId>) -> TestSink {
        self.descriptor.memory_domains = domains;
        self
    }

    /// A recorder writing one artifact (MA-25).
    pub fn new(contract: DataContractId) -> TestSink {
        TestSink {
            descriptor: SinkDescriptor {
                module: mref("ezsdr.test.sink"),
                kind: ns("test.recorder"),
                memory_domains: vec![MemoryDomainId::local(0)],
                contracts: vec![contract],
                artifact_kinds: vec![ns("test.capture")],
            },
            artifact: ArtifactRef {
                id: id("capture0"),
                kind: ns("test.capture"),
                uri: "memory://capture0".to_owned(),
                hash: some_hash("capture0"),
                size_bytes: 0,
                partial: false,
                marks: Vec::new(),
                continuity: Vec::new(),
            },
            aborted: Mutex::new(false),
        }
    }
}

impl Sink for TestSink {
    fn descriptor(&self) -> &SinkDescriptor {
        &self.descriptor
    }

    fn prepare(&mut self, f: &Fragment, _ctx: PrepareContext<'_>,
    ) -> Result<PrepareReport, ModuleError> {
        Ok(PrepareReport {
            fragment: f.id.clone(),
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

    fn step(&mut self, _until: TimePoint) -> Result<StepOutcome, ModuleError> {
        Ok(StepOutcome { progressed: false })
    }

    fn stop(&mut self, mode: StopMode) -> Result<Vec<ArtifactRef>, ModuleError> {
        // MA-26: the refs come back even on an abort, with `partial` set.
        let mut a = self.artifact.clone();
        if mode == StopMode::Abort {
            *self.aborted.lock().expect("lock") = true;
            a.partial = true;
        }
        self.artifact = a.clone();
        Ok(vec![a])
    }

    fn cleanup(&mut self) {}
}

/// The Sink Module's descriptor, whose `Sink` role is what a Sink binding's `module`
/// resolves to when RS-12 builds a Session's implicit Spec (SB-22).
pub fn test_sink_descriptor() -> ModuleDescriptor {
    ModuleDescriptor {
        id: mid("ezsdr.test.sink"),
        version: Version::new(1, 0, 0),
        kernel_api: Version::new(4, 0, 0),
        roles: vec![Role::Sink],
        vocabularies: Vec::new(),
        deployment: Deployment::InProcess {},
        impl_hash: Some(some_hash("ezsdr.test.sink")),
    }
}

/// The Module that supplies the Executor instance an Island names (MA-38).
pub fn test_executor_descriptor() -> ModuleDescriptor {
    ModuleDescriptor {
        id: mid("ezsdr.test.executor"),
        version: Version::new(1, 0, 0),
        kernel_api: Version::new(4, 0, 0),
        roles: vec![Role::Executor],
        vocabularies: Vec::new(),
        deployment: Deployment::InProcess {},
        impl_hash: Some(some_hash("ezsdr.test.executor")),
    }
}

/// The Link Module descriptor used by plan fixtures (MA-28).
pub fn test_link_module_descriptor() -> ModuleDescriptor {
    ModuleDescriptor {
        id: mid("ezsdr.test.link"),
        version: Version::new(1, 0, 0),
        kernel_api: Version::new(4, 0, 0),
        roles: vec![Role::Link],
        vocabularies: Vec::new(),
        deployment: Deployment::InProcess {},
        impl_hash: Some(some_hash("ezsdr.test.link")),
    }
}

/// A Link descriptor supporting every Phase 1 back-pressure policy (MA-28).
pub fn test_link_descriptor() -> LinkDescriptor {
    LinkDescriptor {
        module: mref("ezsdr.test.link"),
        kind: ns("test.link"),
        connects: vec![
            (MemoryDomainId::local(0), MemoryDomainId::local(1)),
            (MemoryDomainId::local(1), MemoryDomainId::local(0)),
        ],
        policies: vec![
            ezsdr_kernel::stream::BackPressure::Block,
            ezsdr_kernel::stream::BackPressure::DropOldest,
            ezsdr_kernel::stream::BackPressure::DropNewest,
        ],
        cross_process: false,
    }
}

/// The `ComponentDescriptor` the Sink Module supplies for its recorder (RS-12).
pub fn recorder_component(contract: DataContractId) -> ComponentDescriptor {
    ComponentDescriptor {
        id: id("recorder"),
        kind: ComponentKind::Processor,
        ports: vec![Port {
            name: "in".to_owned(),
            direction: PortDirection::In,
            contract,
        }],
        params: vec![ParamDecl {
            key: key("test.capture"),
            schema: serde_json::json!({ "type": "boolean" }),
            update_class: UpdateClass::BlockBoundary,
            default: Value::Bool(false),
        }],
        timing: ComponentTiming::default(),
        requires: ComponentRequires { executor_kind: "any".to_owned(), memory_bytes: None,
        },
        implementation: ComponentImpl {
            kind: ns("test.impl"),
            id: "recorder".to_owned(),
            hash: some_hash("recorder"),
        },
    }
}

/// A component with one `out` port, for a link whose **consumer** is a resource port
/// — Vision §7's `PHY Processor -> Radio Port`, the TX direction.
pub fn source_component(contract: DataContractId) -> ComponentDescriptor {
    let mut c = recorder_component(contract.clone());
    c.id = id("source");
    c.ports = vec![Port { name: "out".to_owned(), direction: PortDirection::Out, contract,
    }];
    c.implementation.id = "source".to_owned();
    c
}

// ---------------------------------------------------------------- the host clock

/// A host clock the tests advance by hand (RS-22, decision R4).
#[derive(Default)]
pub struct FakeHostClock {
    millis: AtomicU64,
}

impl FakeHostClock {
    /// A clock at zero (RS-22).
    pub fn new() -> FakeHostClock {
        FakeHostClock::default()
    }

    /// Moves the clock forward (RS-22).
    pub fn advance(&self, millis: u64) {
        self.millis.fetch_add(millis, Ordering::Relaxed);
    }
}

impl HostClock for FakeHostClock {
    fn monotonic_millis(&self) -> u64 {
        self.millis.load(Ordering::Relaxed)
    }

    fn utc_nanos(&self) -> i64 {
        1_700_000_000_000_000_000i64 + self.millis.load(Ordering::Relaxed) as i64 * 1_000_000
    }
}

// ---------------------------------------------------------------- Action handles

/// An inbound Action queue for a Module (MA-14).
#[derive(Default)]
pub struct QueueReceiver {
    queue: Mutex<Vec<Action>>,
}

impl QueueReceiver {
    /// An empty queue (MA-14).
    pub fn new() -> QueueReceiver {
        QueueReceiver::default()
    }

    /// Delivers an admitted Action (MA-14).
    pub fn push(&self, a: Action) {
        self.queue.lock().expect("lock").push(a);
    }

    /// Everything queued so far (MA-14).
    pub fn drain(&self) -> Vec<Action> {
        std::mem::take(&mut *self.queue.lock().expect("lock"))
    }
}

impl ActionReceiver for QueueReceiver {
    fn recv(&self) -> Option<Action> {
        let mut q = self.queue.lock().expect("lock");
        if q.is_empty() { None } else { Some(q.remove(0)) }
    }
}

/// A submitter that runs an injected admission rule and, on success, delivers to a
/// queue (MA-14a, RS-16).
pub struct TestSubmitter {
    /// Where an admitted Action lands (MA-14).
    pub delivered: Mutex<Vec<Action>>,
    /// Actions this submitter refuses; the closure-free stand-in for `admit()`.
    pub ceiling: Option<f64>,
    next: AtomicU64,
}

impl TestSubmitter {
    /// A submitter admitting everything (MA-14a).
    pub fn new() -> TestSubmitter {
        TestSubmitter { delivered: Mutex::new(Vec::new()), ceiling: None, next: AtomicU64::new(0),
        }
    }

    /// A submitter refusing a `TxBurst` whose `test.limits` metadata exceeds the
    /// ceiling (MA-14a, RS-16).
    pub fn with_ceiling(mut self, ceiling: f64) -> TestSubmitter {
        self.ceiling = Some(ceiling);
        self
    }
}

impl Default for TestSubmitter {
    fn default() -> Self {
        TestSubmitter::new()
    }
}

impl ActionSubmitter for TestSubmitter {
    fn submit(&self, action: Action) -> Result<ActionId, Vec<Violation>> {
        if let (Some(ceiling), Action::TxBurst { metadata, .. }) = (self.ceiling, &action) {
            let asked = metadata.get(&key("test.grid")).and_then(|v| match v {
                Value::Num(n) => Some(*n),
                Value::Int(i) => Some(*i as f64),
                _ => None,
            });
            if asked.is_some_and(|n| n > ceiling) {
                return Err(vec![Violation {
                    check: ns("test.limits"),
                    key: Some(key("test.grid")),
                    requested: metadata.get(&key("test.grid")).cloned(),
                    reason: format!("RS-16: {asked:?} exceeds the ceiling {ceiling}"),
                }]);
            }
        }
        self.delivered.lock().expect("lock").push(action);
        Ok(ActionId(self.next.fetch_add(1, Ordering::Relaxed)))
    }
}

// ---------------------------------------------------------------- cleanup double

/// Records the cleanup sequence, with an injectable failure and an injectable wedge
/// (RS-6, RS-8a).
pub struct RecordingCleanup {
    /// Every `(step, fragment, mode)` in the order it was attempted (RS-6).
    pub log: Mutex<Vec<(CleanupStep, Option<Ident>, CleanupMode)>>,
    /// A step that returns an error (RS-6).
    pub fail: Option<CleanupStep>,
    /// A step that never returns (RS-8a).
    pub wedge: Option<CleanupStep>,
    /// The per-step deadline in milliseconds (RS-8a).
    pub deadline_ms: u64,
}

impl RecordingCleanup {
    /// A cleanup that records and always succeeds (RS-6).
    pub fn new() -> RecordingCleanup {
        RecordingCleanup {
            log: Mutex::new(Vec::new()),
            fail: None,
            wedge: None,
            deadline_ms: 200,
        }
    }

    /// Makes `step` return an error (RS-6).
    pub fn failing_at(mut self, step: CleanupStep) -> RecordingCleanup {
        self.fail = Some(step);
        self
    }

    /// Makes `step` never return (RS-8a).
    pub fn wedged_at(mut self, step: CleanupStep) -> RecordingCleanup {
        self.wedge = Some(step);
        self
    }

    /// The steps attempted, in order (RS-6).
    pub fn steps(&self) -> Vec<CleanupStep> {
        self.log.lock().expect("lock").iter().map(|(s, _, _)| *s).collect()
    }

    /// The `(step, fragment)` pairs attempted, in order (RS-8).
    pub fn entries(&self) -> Vec<(CleanupStep, Option<Ident>, CleanupMode)> {
        self.log.lock().expect("lock").clone()
    }
}

impl Default for RecordingCleanup {
    fn default() -> Self {
        RecordingCleanup::new()
    }
}

impl CleanupOps for RecordingCleanup {
    fn perform(
        &self,
        step: CleanupStep,
        fragment: Option<&Ident>,
        mode: CleanupMode,
    ) -> Result<(), ModuleError> {
        self.log.lock().expect("lock").push((step, fragment.cloned(), mode));
        if self.wedge == Some(step) {
            // RS-8a: a step that never returns must not stop the sequence.
            std::thread::sleep(std::time::Duration::from_secs(30));
        }
        if self.fail == Some(step) {
            return Err(ModuleError::rejected(format!("injected failure at {step:?}")));
        }
        Ok(())
    }

    fn deadline_millis(&self, _step: CleanupStep, _fragment: Option<&Ident>) -> u64 {
        self.deadline_ms
    }
}

/// The `host.monotonic` domain, which every Run governs (TM-16a).
pub const HOST: ClockDomainId = ClockDomainId::HOST_MONOTONIC;
