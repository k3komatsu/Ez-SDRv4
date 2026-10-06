//! Spec 14's tests (design/14-native-executor.md §6): the Executor driven directly,
//! with a harness in the style of spec 09's and a scripted test component.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use ezsdr_exec_native::{
    Component, ComponentContext, Implementation, NativeExecutor, descriptor, executor_descriptor,
};
use ezsdr_kernel::binding::Violation;
use ezsdr_kernel::contract::{DataContractId, Port, PortDirection};
use ezsdr_kernel::event::{Action, ActionId, Event, EventHandle, EventKind, EventSink, Severity};
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::{ClockDomainId, IslandId, MemoryDomainId, ResourceId, RunId};
use ezsdr_kernel::module_api::{
    ActionReceiver, ActionSubmitter, AttachedPort, ComponentDescriptor, ComponentImpl,
    ComponentKind, ComponentPlacement, ComponentRequires, ComponentTiming, Endpoint, ExecutionClass, Executor,
    Factories, IslandDecl, ModuleError, ModuleRegistry, Pacing, PrepareContext, StepOutcome,
    StopMode, UpdateClass,
};
use ezsdr_kernel::run::{RunError, StopCause};
use ezsdr_kernel::spec::{Ident, Key, Namespace, Value};
use ezsdr_kernel::stream::{BackPressure, BlockRef, DataLink, DropCarry, PublishOutcome};
use ezsdr_kernel::time::{
    ClockDomain, ClockRegistry, Duration, EpochRef, ManualTimeAuthority, Rational, RelativeBudget,
    TimePoint,
};

const ROOT: ClockDomainId = ClockDomainId::local(7);
const SCRIPTED: &str = "test.scripted";

fn id(name: &str) -> Ident {
    Ident::parse(name).unwrap()
}

fn scripted_hash() -> ContentHash {
    ContentHash::of_bytes(b"test.scripted 1.0.0")
}

// ------------------------------------------------------------ the scripted component

#[derive(Default)]
struct Script {
    log: Vec<String>,
    /// Per component id: what each successive `step` pushes and reports.
    steps: BTreeMap<String, VecDeque<(Vec<Action>, bool)>>,
    failing_stop: Vec<String>,
    failing_prepare: Vec<String>,
}

thread_local! {
    static SCRIPT: RefCell<Script> = RefCell::new(Script::default());
}

fn log() -> Vec<String> {
    SCRIPT.with(|s| s.borrow().log.clone())
}

fn record(line: String) {
    SCRIPT.with(|s| s.borrow_mut().log.push(line));
}

fn script_step(component: &str, actions: Vec<Action>, progressed: bool) {
    SCRIPT.with(|s| s.borrow_mut().steps.entry(component.to_owned()).or_default().push_back((actions, progressed)));
}

#[derive(Default)]
struct Scripted {
    id: String,
}

impl Component for Scripted {
    fn prepare(&mut self, ctx: ComponentContext) -> Result<(), ModuleError> {
        self.id = ctx.descriptor.id.to_string();
        let ports: Vec<String> = ctx.links.iter().map(|l| format!("{}.{}", l.component, l.port)).collect();
        record(format!("prepare:{}:links={}:source={}", self.id, ports.join(","), ctx.source.path));
        if SCRIPT.with(|s| s.borrow().failing_prepare.contains(&self.id)) {
            return Err(ModuleError::rejected(format!("test: {} fails its prepare", self.id)));
        }
        Ok(())
    }

    fn step(&mut self, until: TimePoint, out: &mut Vec<Action>) -> Result<StepOutcome, ModuleError> {
        record(format!("step:{}@{}", self.id, until.ticks));
        let next = SCRIPT.with(|s| s.borrow_mut().steps.get_mut(&self.id).and_then(VecDeque::pop_front));
        let (actions, progressed) = next.unwrap_or_default();
        out.extend(actions);
        Ok(StepOutcome { progressed })
    }

    fn stop(&mut self, mode: StopMode) -> Result<(), ModuleError> {
        record(format!("stop:{}:{mode:?}", self.id));
        if SCRIPT.with(|s| s.borrow().failing_stop.contains(&self.id)) {
            return Err(ModuleError::rejected(format!("test: {} fails its stop", self.id)));
        }
        Ok(())
    }

    fn cleanup(&mut self) {
        record(format!("cleanup:{}", self.id));
    }
}

fn scripted() -> Implementation {
    Implementation { id: SCRIPTED.to_owned(), hash: scripted_hash(), make: || Box::new(Scripted::default()) }
}

// ------------------------------------------------------------ the harness

#[derive(Default)]
struct Queue(Mutex<VecDeque<Action>>);

impl ActionReceiver for Queue {
    fn recv(&self) -> Option<Action> {
        self.0.lock().unwrap().pop_front()
    }
}

/// Records every submitted Action and answers each with the next scripted result, `Ok`
/// once the script is empty.
#[derive(Default)]
struct Submitter {
    submitted: Mutex<Vec<Action>>,
    answers: Mutex<VecDeque<Result<ActionId, Vec<Violation>>>>,
}

impl ActionSubmitter for Submitter {
    fn submit(&self, action: Action) -> Result<ActionId, Vec<Violation>> {
        record(format!("submit:{}", action.target().map_or_else(String::new, |t| t.path.clone())));
        self.submitted.lock().unwrap().push(action);
        self.answers.lock().unwrap().pop_front().unwrap_or(Ok(ActionId(1)))
    }
}

struct NullEvents;

impl EventSink for NullEvents {
    fn resolve(&self, _source: &ResourceId, _kind: &EventKind) -> EventHandle {
        EventHandle { row: 0, kind: 0 }
    }
    fn emit(&self, _handle: EventHandle, _time: TimePoint, _severity: Severity, _payload: &[u8]) -> Result<(), RunError> {
        Ok(())
    }
    fn emit_control(&self, _event: Event) -> Result<(), RunError> {
        Ok(())
    }
}

struct NullLink;

impl DataLink for NullLink {
    fn publish(&self, _b: BlockRef) -> PublishOutcome {
        PublishOutcome::Accepted
    }
    fn receive(&self) -> Option<BlockRef> {
        None
    }
    fn drops(&self) -> u64 {
        0
    }
    fn take_drop_carry(&self) -> DropCarry {
        DropCarry::default()
    }
    fn policy(&self) -> BackPressure {
        BackPressure::DropOldest
    }
}

struct Harness {
    clocks: Arc<ClockRegistry>,
    time: Arc<ManualTimeAuthority>,
    actions: Arc<Queue>,
    out: Arc<Submitter>,
}

impl Harness {
    fn new() -> Harness {
        SCRIPT.with(|s| *s.borrow_mut() = Script::default());
        let clocks = Arc::new(ClockRegistry::new());
        clocks.register(ClockDomain::root(ROOT, Rational::new(1_000_000_000, 1).unwrap(), EpochRef::Arbitrary { set_by: "test".to_owned() })).unwrap();
        let time = Arc::new(ManualTimeAuthority::new(clocks.clone(), ROOT, &[], Pacing::FreeRunning).unwrap());
        Harness { clocks, time, actions: Arc::new(Queue::default()), out: Arc::new(Submitter::default()) }
    }

    fn ctx(&self, class: ExecutionClass, links: Vec<AttachedPort>, components: &[ComponentDescriptor]) -> PrepareContext {
        PrepareContext {
            run: RunId::from_string("test-run".to_owned()),
            class,
            time: self.time.clone(),
            clocks: self.clocks.clone(),
            events: Arc::new(NullEvents),
            actions: self.actions.clone(),
            actions_out: self.out.clone(),
            environment: Arc::new(BTreeMap::new()),
            inputs: Arc::new(BTreeMap::<ContentHash, Arc<[u8]>>::new()),
            links,
            components: components.iter().map(|c| (c.id.clone(), c.clone())).collect(),
            host_budget: RelativeBudget::new(Duration::new(ClockDomainId::HOST_MONOTONIC, 5_000_000_000)).unwrap(),
        }
    }

    fn answer(&self, answer: Result<ActionId, Vec<Violation>>) {
        self.out.answers.lock().unwrap().push_back(answer);
    }
}

fn component(name: &str) -> ComponentDescriptor {
    ComponentDescriptor {
        id: id(name),
        kind: ComponentKind::Reactor,
        ports: vec![Port { name: id("rx"), direction: PortDirection::In, contract: DataContractId::parse("ezsdr.stream.cf32").unwrap() }],
        params: Vec::new(),
        timing: ComponentTiming::default(),
        requires: ComponentRequires { executor_kind: Namespace::parse("ezsdr.exec.native").unwrap(), memory_bytes: None },
        implementation: ComponentImpl { kind: Namespace::parse("ezsdr.impl.native").unwrap(), id: SCRIPTED.to_owned(), hash: scripted_hash() },
    }
}

fn island(local: u32, components: &[&str]) -> IslandDecl {
    IslandDecl {
        id: IslandId::local(local),
        executor: id("exec"),
        components: components
            .iter()
            .map(|c| ComponentPlacement { component: id(c), memory_domain: MemoryDomainId::local(0) })
            .collect(),
        affinity: None,
        rt_policy: None,
        batch: None,
    }
}

fn link_end(component: &str) -> AttachedPort {
    AttachedPort { component: id(component), port: id("rx"), endpoint: Endpoint::StreamIn(Arc::new(NullLink)) }
}

fn at(ticks: i64) -> TimePoint {
    TimePoint::new(ROOT, ticks)
}

fn stop_action(target: &str) -> Action {
    Action::Stop { target: Some(ResourceId::parse(target).unwrap()) }
}

fn refusal(check: &str, reason: &str) -> Violation {
    Violation { check: Namespace::parse(check).unwrap(), key: None, requested: None, reason: reason.to_owned() }
}

fn prepared(h: &Harness, components: &[&str]) -> NativeExecutor {
    let mut executor = NativeExecutor::new(vec![scripted()]).unwrap();
    let descriptors: Vec<_> = components.iter().map(|c| component(c)).collect();
    executor
        .prepare(&island(0, components), h.ctx(ExecutionClass::Simulation, components.iter().map(|c| link_end(c)).collect(), &descriptors))
        .unwrap();
    executor
}

// ------------------------------------------------------------ the tests

#[test]
fn nx_01_descriptor_registers() {
    let mut registry = ModuleRegistry::new();
    registry.register(descriptor(), Factories { executor: true, ..Factories::default() }).unwrap();
    let module = descriptor();
    assert_eq!(module.id.as_str(), "ezsdr.exec.native");
    assert_eq!(module.version.to_string(), "1.0.0");
    assert!(module.vocabularies.is_empty());
    assert_eq!(module.impl_hash, Some(ContentHash::of_bytes(b"ezsdr.exec.native 1.0.0")));
    let declared = executor_descriptor();
    assert_eq!(declared.module.id, module.id);
    assert_eq!(declared.module.version, module.version);
    assert_eq!(declared.kind.as_str(), "ezsdr.exec.native");
    assert_eq!(declared.memory_domains, vec![MemoryDomainId::local(0)]);
    assert_eq!(declared.impl_kinds, vec![Namespace::parse("ezsdr.impl.native").unwrap()]);
    assert!(declared.capabilities.is_empty());
    assert_eq!(NativeExecutor::new(Vec::new()).unwrap().descriptor(), &declared);
}

#[test]
fn nx_03_a_component_is_loaded_by_its_impl_identity() {
    let h = Harness::new();
    let refused = |descriptor: ComponentDescriptor| {
        let mut executor = NativeExecutor::new(vec![scripted()]).unwrap();
        let error = executor
            .prepare(&island(0, &["c"]), h.ctx(ExecutionClass::Simulation, Vec::new(), &[descriptor]))
            .unwrap_err();
        assert!(error.message.starts_with("NX-3: component c "), "{}", error.message);
        error.message
    };
    let mut unknown = component("c");
    unknown.implementation.id = "test.other".to_owned();
    assert!(refused(unknown).contains("test.other"));
    let mut wrong_hash = component("c");
    wrong_hash.implementation.hash = ContentHash::of_bytes(b"test.scripted 2.0.0");
    assert!(refused(wrong_hash).contains("hash"));
    let mut wrong_kind = component("c");
    wrong_kind.implementation.kind = Namespace::parse("ezsdr.impl.wasm").unwrap();
    assert!(refused(wrong_kind).contains("ezsdr.impl.wasm"));
    assert!(log().is_empty(), "nothing is built for a refused identity: {:?}", log());

    let duplicate = NativeExecutor::new(vec![scripted(), scripted()]).err().expect("two implementations with one id");
    assert!(duplicate.message.starts_with("NX-3: two implementations"), "{}", duplicate.message);

    let _ = prepared(&h, &["c"]);
    assert_eq!(log(), vec!["prepare:c:links=c.rx:source=island_0"]);
}

#[test]
fn nx_04_prepare_cases() {
    let h = Harness::new();
    let mut executor = NativeExecutor::new(vec![scripted()]).unwrap();
    let other_class = executor
        .prepare(&island(0, &["a"]), h.ctx(ExecutionClass::RealtimeEmulation, Vec::new(), &[component("a")]))
        .unwrap_err();
    assert_eq!(other_class.kind, ezsdr_kernel::module_api::ModuleErrorKind::Unsupported);
    assert!(other_class.message.starts_with("NX-4:"), "{}", other_class.message);

    let foreign = executor
        .prepare(&island(0, &["a"]), h.ctx(ExecutionClass::Simulation, vec![link_end("z")], &[component("a")]))
        .unwrap_err();
    assert!(foreign.message.contains("z, which is not in this Island"), "{}", foreign.message);
    assert!(log().is_empty());

    // Two Islands on one Executor: each component sees only its own link ends and its
    // own Island's source, and each Island reports its own fragment.
    let mut executor = NativeExecutor::new(vec![scripted()]).unwrap();
    let first = executor
        .prepare(&island(3, &["a", "b"]), h.ctx(ExecutionClass::Simulation, vec![link_end("a"), link_end("b"), link_end("b")], &[component("a"), component("b")]))
        .unwrap();
    let second = executor
        .prepare(&island(4, &["c"]), h.ctx(ExecutionClass::Simulation, vec![link_end("c")], &[component("c")]))
        .unwrap();
    assert_eq!(first.fragment.as_str(), "island_3");
    assert_eq!(second.fragment.as_str(), "island_4");
    assert!(first.effective.is_empty() && first.coercions.is_empty() && first.warnings.is_empty());
    assert_eq!(log(), vec![
        "prepare:a:links=a.rx:source=island_3",
        "prepare:b:links=b.rx,b.rx:source=island_3",
        "prepare:c:links=c.rx:source=island_4",
    ]);

    let twice = executor
        .prepare(&island(5, &["a"]), h.ctx(ExecutionClass::Simulation, Vec::new(), &[component("a")]))
        .unwrap_err();
    assert!(twice.message.contains("component a is placed twice"), "{}", twice.message);

    // A component whose `prepare` fails fails the Island's, and is still cleaned up
    // (MA-7: every instance that reached `prepare`).
    let h = Harness::new();
    SCRIPT.with(|s| s.borrow_mut().failing_prepare.push("f".to_owned()));
    let mut executor = NativeExecutor::new(vec![scripted()]).unwrap();
    let failed = executor
        .prepare(&island(0, &["f"]), h.ctx(ExecutionClass::Simulation, Vec::new(), &[component("f")]))
        .unwrap_err();
    assert_eq!(failed.message, "test: f fails its prepare");
    executor.cleanup();
    assert_eq!(log(), vec!["prepare:f:links=:source=island_0", "cleanup:f"]);
}

#[test]
fn nx_05_components_step_in_id_order_and_their_actions_follow_each_step() {
    let h = Harness::new();
    // `b` first in the Island and in the scripts; `a` still steps first.
    let mut executor = prepared(&h, &["b", "a"]);
    script_step("a", vec![stop_action("radio_a/tx")], false);
    script_step("b", vec![stop_action("radio_b/tx")], false);
    let first = executor.step(at(10)).unwrap();
    assert!(first.progressed, "a pushed Action alone is progress (MA-20)");
    let steps: Vec<_> = log().into_iter().filter(|l| l.starts_with("step:") || l.starts_with("submit:")).collect();
    assert_eq!(steps, vec!["step:a@10", "submit:radio_a/tx", "step:b@10", "submit:radio_b/tx"]);

    let second = executor.step(at(20)).unwrap();
    assert!(!second.progressed, "nothing consumed, nothing pushed");
    script_step("a", Vec::new(), true);
    assert!(executor.step(at(30)).unwrap().progressed, "a component's own progress is reported");
}

#[test]
fn nx_06_a_refusal_because_the_run_is_ending_is_not_a_failure() {
    let h = Harness::new();
    let mut executor = prepared(&h, &["a"]);
    for check in ["ezsdr.dispatch", "ezsdr.run_state"] {
        h.answer(Err(vec![refusal(check, "the Run is ending")]));
        script_step("a", vec![stop_action("radio/tx")], false);
        assert!(executor.step(at(10)).is_ok(), "{check} says the Run is ending (KE-3)");
    }
    assert_eq!(h.out.submitted.lock().unwrap().len(), 2);
}

#[test]
fn nx_06_any_other_refusal_fails_the_step() {
    let h = Harness::new();
    let mut executor = prepared(&h, &["a"]);
    h.answer(Err(vec![refusal("ezsdr.target", "KC-23: nowhere names no resource")]));
    script_step("a", vec![stop_action("nowhere")], false);
    let error = executor.step(at(10)).unwrap_err();
    assert_eq!(error.message, "NX-6: component a's Stop was refused: ezsdr.target: KC-23: nowhere names no resource");

    h.answer(Err(vec![refusal("ezsdr.dispatch", "RS-6: dispatch is frozen"), refusal("ezsdr.input", "RS-44a: not an input")]));
    script_step("a", vec![stop_action("radio/tx")], false);
    let mixed = executor.step(at(20)).unwrap_err();
    assert!(mixed.message.contains("ezsdr.dispatch: RS-6") && mixed.message.contains("ezsdr.input: RS-44a"), "{}", mixed.message);

    h.answer(Err(Vec::new()));
    script_step("a", vec![stop_action("radio/tx")], false);
    assert!(executor.step(at(30)).is_err(), "a refusal that names no check says nothing about the Run ending");
}

#[test]
fn nx_07_an_action_addressed_to_the_executor_fails_the_step() {
    let h = Harness::new();
    let mut executor = prepared(&h, &["a"]);
    h.actions.0.lock().unwrap().push_back(Action::UpdateParameter {
        target: ResourceId::parse("a").unwrap(),
        key: Key::parse("ext.ezsdr.exec.native.x").unwrap(),
        value: Value::Int(1),
        class: UpdateClass::Cold,
        at: None,
    });
    let error = executor.step(at(10)).unwrap_err();
    assert_eq!(error.message, "NX-7: ezsdr.exec.native 1.0.0 applies no Action, and UpdateParameter for local:a reached it");
    assert!(!log().iter().any(|l| l.starts_with("step:")), "no component is stepped: {:?}", log());
}

#[test]
fn nx_08_stop_reaches_every_component_and_cleanup_is_idempotent() {
    let h = Harness::new();
    let mut executor = prepared(&h, &["a", "b"]);
    SCRIPT.with(|s| s.borrow_mut().failing_stop.push("a".to_owned()));
    let error = executor.stop(StopMode::Orderly).unwrap_err();
    assert_eq!(error.message, "test: a fails its stop");
    assert_eq!((Arc::strong_count(&h.out), Arc::strong_count(&h.actions)), (2, 2), "the Executor keeps both queues");
    executor.cleanup();
    executor.cleanup();
    // Every kept handle is dropped (MA-5a, MA-7): the queues are the harness's alone, and an
    // Action that arrives now reaches nothing (Review F, P2-7).
    assert_eq!((Arc::strong_count(&h.out), Arc::strong_count(&h.actions)), (1, 1));
    h.actions.0.lock().unwrap().push_back(stop_action("a"));
    assert!(executor.step(at(10)).unwrap() == StepOutcome { progressed: false });
    let ending: Vec<_> = log().into_iter().filter(|l| !l.starts_with("prepare:")).collect();
    assert_eq!(ending, vec!["stop:a:Orderly", "stop:b:Orderly", "cleanup:a", "cleanup:b"]);
    let _ = StopCause::Client {};
}
