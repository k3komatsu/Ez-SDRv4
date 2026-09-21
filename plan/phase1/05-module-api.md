# Phase 1 spec 05 — Module API

| Field | Value |
|---|---|
| Status | Draft for Gate C. Normative for `ezsdr-kernel::module_api` once accepted. |
| Scope | The three axes and the five role traits; the rules every role trait obeys; `ModuleDescriptor`, `VocabularyDescriptor` and the registry; `ComponentDescriptor`; the ExecutionIsland declaration and its admission; ExecutionClass and the fidelity vector; the Phase 1 test doubles; what is fixed now so that an out-of-process Plugin later needs no Kernel change. |
| Not in scope | Radio Model traits and keys (Phase 2); MockRadio and the Simulation Engine (Phase 2); the Peripheral and Endpoint vocabularies (Phase 9); the native, WASM and GPU execution ABIs (Phases 10 and 11); the Plugin wire protocol (Phase 9). |
| Vision § covered | §4; §5's Module API line; §7 in full; §14; §15's `step`; §19; §20; §32; §35's bridge requirements; §36; §37 and §38; §42's Executor rule; §62. |
| Audit §14.1 items | 11 (descriptor versus execution ABI, the cycle rule, the two deadline kinds), 15 (the three axes), and parts of 2 (step-driven simulation), 4 (the fidelity vector and the coerce contract), 6 (the resource tree), 8 (the coerce side), 14 (RFNoC is not an Executor kind). Findings 1, 7, 8, 18, 20, 22. |
| Re-review | R8 (the Authority role), R22 ("Kernel" means only the Core tier). |
| Depends on | Spec 01 for `TimePoint` and `TimeAuthority`; spec 02 for blocks, links and `DataLinkDecl`; spec 03 for `CapabilityValue`, `KeyDecl`, `PrepareReport` and the plan; spec 04 for the Action set, `Event`, `StopCause` and `ArtifactRef`. |
| Modal verbs | "must" and "must not" are normative (OV-4a). "Should" does not appear inside a rule. |

---

## 1. Purpose

Vision §7 separates three things that are easy to conflate: a Module is a unit of versioning and deployment, a role is a contract it implements, and in-process or out-of-process is how it is deployed. Audit Finding 22 asks for them as orthogonal axes, and the practical consequence is one trait per role rather than one `Provider` trait bent over radios and executors alike.

The other half of this document is a boundary. Audit Finding 8 says the Kernel owns a component's **descriptor** and the vocabulary it speaks, and not the way its code is invoked. GNU Radio carries `processOne`, `processBulk` and `work` in its core block API; a GPU executor batches and a WASM executor copies through linear memory, so a shared call signature would be a lowest common denominator that suits neither. Executors here share descriptors, DataContracts and DataLinks, and nothing else.

## 2. Evidence

- **Rust has no stable ABI** (audit Appendix B, verified). A dynamically loaded in-process Module would need a C-ABI layer or a crate such as `abi_stable`; the default is therefore compiled in, and out-of-process is the Plugin path (Vision §62).
- **v3's escape hatch became the main road.** `IDevice.setParam(key, value)` took a string key and a string value (`v3/source/device/package.d`), and synchronisation ran through it: `setParam("set_time_unknown_pps_to_zero", "[]")` in `v3/cpp/uhd_usrp/multiusrp.cpp:502-518`. Audit Finding 1 cites this as what happens when a typed contract has an untyped side door. MA-6's rule that every parameter and return is a document or a Kernel handle is the answer.
- **GNU Radio 4 keeps a block API in its core** (`processOne` / `processBulk` / `work`), and its scheduler has no deadline or budget primitive. Audit Finding 8 concludes that Ez-SDR can decline both, which MA-21 does.
- **GNU Radio 4 has an `externalStep` execution policy**, which is the shape audit Finding 18 asks for and which MA-20 adopts as a requirement rather than an option.
- **The same peripheral kind is not the same timing class on two devices.** Timed GPIO works through the radio's timed interface on an X3x0 and not on an X4x0, where only ATR is hardware-controlled. Vision §38 therefore makes the timing class a property of an instance, not of a Provider kind, and MA-17 places it where `validate()` can see it.

## 3. Model

```text
axis 1  unit         Module        a crate or a process; the unit of versioning and deployment
axis 2  role         Provider      implements a Resource Model
                     Executor      implements a processing engine
                     Sink          writes Artifacts
                     Link          implements a DataLink
                     Authority     implements the Time Authority
axis 3  deployment   InProcess     a Rust trait, compiled in
                     Plugin        out-of-process, a typed protocol; reserved, refused in Phase 1

One Module may hold several roles. The UHD Module is a Radio Provider, a GPIO Provider and an
Authority; the sim-engine Module is an Authority. "Plugin" names a deployment, never a role.
```

## 4. Types

```text
ModuleId          a Namespace, e.g. ezsdr.radio.mock
Version           { major, minor, patch }                release-only; no pre-release tags (SB decision B7)
VersionReq        a caret requirement over a Version
Role              Provider | Executor | Sink | Link | Authority
Deployment        InProcess | Plugin { protocol: Version }

ModuleDescriptor  { id: ModuleId, version: Version, kernel_api: Version, roles: [Role],
                    vocabularies: [{ id: Namespace, req: VersionReq }],
                    deployment: Deployment, impl_hash: optional ContentHash }
VocabularyDescriptor { id: Namespace, version: Version, prefix: Namespace, keys: [KeyDecl],
                       event_kinds: [{ kind: EventKind, default: Reaction, severity: Severity,
                                       hot_layout: optional HotLayout }],   HotLayout: RS-32a
                       verbs: [{ verb: Ident, compiles_to: CompileRule }],
                       checks: [AdmissionCheck] }

ProviderInstance  { id: ResourceId, module: { id, version }, profile: optional { name, version },
                    tree: Resource, fidelity: Fidelity, driving: { stepped: bool },
                    arm_after: [ResourceId], sections: Map<Namespace, Value> }
Resource          { id: ResourceId, kind: Namespace,
                    capabilities: Map<Key, CapabilityValue>, children: [Resource] }
CoerceReport      { applied: Map<Key, Value>, coercions: [Coercion],
                    warnings: [Warning], rejected: [{ key, requested, reason }] }

ExecutorDescriptor { kind: Namespace, memory_domains: [MemoryDomainId],
                     impl_kinds: [Namespace], capabilities: Map<Key, CapabilityValue> }
SinkDescriptor     { kind: Namespace, contracts: [DataContractId], artifact_kinds: [Namespace] }
LinkDescriptor     { kind: Namespace, connects: [(MemoryDomainId, MemoryDomainId)],
                     policies: [BackPressure], cross_process: bool }
AuthorityDescriptor { governs: [ClockDomainId], pacing: FreeRunning | WallPaced | Device }

ComponentDescriptor { id: Ident, kind: Processor | Reactor, ports: [Port],
                      params: [{ key: Key, schema: Value, update_class: UpdateClass, default: Value }],
                      timing: { budget: optional RelativeBudget, preferred_batch: optional u32,
                                stateful: bool, parallelism: optional u32 },
                      requires: { executor_kind: Namespace | "any", memory_bytes: optional u64 },
                      impl: { kind: Namespace, id: string, hash: ContentHash } }
UpdateClass       cold | block_boundary | atomic_realtime | hardware_timed
IslandDecl        { id: IslandId, executor: Ident, components: [Ident],
                    affinity: optional [u32], rt_policy: optional { sched, priority },
                    batch: optional u32 }

ExecutionClass    Simulation | RealtimeEmulation | HardwareInLoop | Hardware
Fidelity          { timing: none | envelope | hardware_quirk | real,
                    continuity: none | envelope | hardware_quirk | real,
                    coercion: none | grid | real,
                    rf: none | impairment_model | real,
                    transport: none | model | real }
Requested         { resource: ResourceId, constraints: Map<Key, Constraint> }
                  what the matcher offers a Provider's coerce (SB-7); prepare sees the same input
Endpoint          StreamIn(DataLink) | StreamOut(DataLink) | EventIn | EventOut
                  one attached end of a declared link (SC-19), handed over at prepare
PrepareContext    { run: RunId, class: ExecutionClass, time: TimeAuthority handle,
                    events: EventSink, actions: ActionReceiver, actions_out: ActionSubmitter,
                    links: [{ port: Ident, endpoint: Endpoint }], host_budget: RelativeBudget }
ActionSubmitter   submits a proposed Action into admit() (RS-16), never to a Module directly
StepOutcome       { progressed: bool }
ModuleError       { kind: Rejected | Unsupported | Timeout | DeviceLost | Internal,
                    message: string, detail: Value }
```

## 5. Normative rules

### The axes and the roles

- **MA-1** The three axes of §3 are orthogonal. A Module declares its roles in its `ModuleDescriptor` and may hold several; `Plugin` names a deployment and never a role (Vision §7, audit Finding 22).
- **MA-2** There are exactly five roles and one trait each. There is no shared lifecycle supertrait: where two traits have methods with the same name it is by convention, and the coordinator dispatches per role. One trait forced onto radios and executors alike is what Finding 22 rejects, and Link and Authority have no lifecycle at all.
- **MA-3** A Module communicates only through Kernel-defined Resources, Ports, Events, Actions, Capabilities and DataContracts. In source terms, a Module crate depends on the Kernel crate and on Vocabulary crates, never on another Module crate. Binding to a sub-resource of another Module's device is resolved by the Kernel (SB-36) and is not a dependency (Vision §7, §39).
- **MA-4** *Withdrawn.* It restated `00-overview.md` OV-23a, which OV-4 forbids. The ban stands there and `kernel_surface` enforces it; the `ma_04_kernel_surface_ban` test row now cites OV-23a.

### Rules common to every role trait

- **MA-5** Role traits are synchronous, object-safe and `Send`. No method is generic, none returns `Self` or an opaque type, and no trait is `async`. An async trait would not be object-safe without boxing and would pull an executor runtime into the Kernel, which every Provider would then inherit.
- **MA-6** Every parameter and return type is one of three things: a document type with a schema, a Kernel handle (`EventSink`, `ActionReceiver`, `ActionSubmitter`, `Endpoint`, a `TimeAuthority` handle), or `ModuleError`. Never a raw slice, a closure, an iterator or a generic parameter. This is what lets a Plugin host implement any role later by message passing without a Kernel change, and it is the rule v3's string-keyed `setParam` violated.
- **MA-7** The coordinator calls lifecycle methods, one call at a time per instance, in the order `prepare → arm → start → (step)* → stop → cleanup` (spec 04, RS-2). `stop(reason)` is always attempted before `cleanup`, on an abort as well. `cleanup` is infallible and idempotent and is called for every instance that reached `prepare`, in reverse dependency order, after a failure at any phase (RS-6, RS-8).
- **MA-8** `prepare` and `arm` are bounded by `PrepareContext.host_budget`; a Module that cannot finish returns `ModuleError { kind: Timeout }`. Enforcing it is the Module's duty in Phase 1. *Ceiling: an in-process Module that hangs hangs the transaction; spec 04's RS-8a gives cleanup its own deadline, and running fragment calls on a worker thread with a join timeout is the upgrade, with no trait change.*
- **MA-9** A fault crosses the boundary only as `ModuleError`. No panic crosses a trait boundary, and a foreign exception is translated to a status (Vision §35). `DeviceLost` is the kind that spec 04's Policy maps to `abort`.

### Provider

- **MA-10** `instance()` returns the `ProviderInstance`, exposing the composite resource tree with capabilities declared per node, so that the matcher can bind at any node and a sub-resource carries its own `ResourceId` (SB-33).
- **MA-11** `coerce(request)` is pure, deterministic and requires no hardware. It is a function of the declared profile, which is what makes `validate()` a true dry run (Vision §52). Two calls with the same request return identical reports.
- **MA-12** `prepare(fragment, ctx)` returns a `PrepareReport` whose `coercions` equal what `coerce` returned for the same request (SB-44). `effective` may narrow a declared capability and must not widen one, and the coordinator re-runs constraint matching over `effective`, which is also Vision §52's re-check of applied values (SB-30).
- **MA-13** `arm` reserves and synchronises and radiates nothing. `start(at)` begins at that instant or as soon as possible. `stop(reason)` is graceful under an orderly stop, delivering the declared tail, and immediate under an abort. `cleanup` releases and restores baseline.
- **MA-14** Actions reach a Provider only through `ctx.actions`, and only after Kernel admission (RS-16). A constraint that can only be checked at run time, such as the lead of a Reactor's burst, is enforced by the Provider, which emits the typed event; it never silently accepts.
- **MA-14a** A Module **emits** an Action through `ctx.actions_out`, which submits it to `admit()` (RS-16) rather than to another Module. Without a sender the inbound queue would have no producer: Vision §5 and §19 define the Action set as what a Reactor emits into the real-time path, and §19's worked example is a Reactor that receives a decoded packet and schedules a `TxBurst`, so the reactive half of the architecture would have no interface at all. Routing through `admit()` rather than to the target keeps invariant 42 true for a Reactor's Action exactly as for a Session's, and it is why the handle is a submitter rather than a queue into a peer.
- **MA-15** `step(until)` has a default no-op body on `Provider`. A Provider whose `driving.stepped` is true overrides it under MA-20's contract; a hardware Provider never receives a `step`.
- **MA-16** A Module never holds a reference to another Module's instance.
- **MA-17** An instance-level capability such as a Peripheral's timing class is knowable at instantiation, from the selector and profile, and appears in `instance()`. Vision §38 says the class "is returned as a capability at `prepare` time", but the pipeline matches capabilities before `prepare` (SB-37), so a class first visible at `prepare` could not be matched. `PrepareReport.effective` may narrow it (MA-12), which is what §38's sentence is really about.

```rust
pub trait Provider: Send {
    fn instance(&self) -> &ProviderInstance;                                    // MA-10
    fn coerce(&self, request: &Requested) -> Result<CoerceReport, ModuleError>; // MA-11
    fn prepare(&mut self, f: &Fragment, ctx: PrepareContext)
        -> Result<PrepareReport, ModuleError>;                                  // MA-12
    fn arm(&mut self) -> Result<(), ModuleError>;
    fn start(&mut self, at: Option<TimePoint>) -> Result<(), ModuleError>;      // MA-13
    fn stop(&mut self, reason: StopCause) -> Result<(), ModuleError>;
    fn cleanup(&mut self);                                                      // MA-7
    fn step(&mut self, _until: TimePoint) -> Result<StepOutcome, ModuleError> { // MA-15
        Ok(StepOutcome { progressed: false })
    }
}
```

### Executor

- **MA-18** `descriptor()` returns the `ExecutorDescriptor`. Admission reads `kind`, `memory_domains` and `impl_kinds`; everything else is opaque.
- **MA-19** `prepare(island, ctx)` loads each component by its `impl` identity and returns a `PrepareReport` for the island.
- **MA-20** `step(until)` must consume every input at or before `until`, emit every output and event at or before `until`, emit nothing after it, never block on input, never call `wait_until` (TM-16d), and report `progressed` true exactly when it consumed an input or produced an output. It is required on `Executor`, because without it a deterministic Run is a wish (audit Finding 18).
- **MA-21** The execution ABI is the Executor's. The Kernel defines no `work` or `process` signature and never inspects `impl` beyond its identity (audit Finding 8).
- **MA-22** Plan admission rejects a cycle formed by `stream.*` links. A cycle is legal only when every edge that closes it is an Event or Action edge crossing an Island boundary. Adaptive feedback inside a component is state, not structure (Vision §19).
- **MA-23** Where a component maps input blocks to output blocks one to one, the Executor propagates block flags unchanged by default (SC-17). A mapping that is not one to one is the Processor author's documented convention, not a Kernel type. *Forward obligation, tested in Phase 10.*
- **MA-24** An `UpdateParameter` reaches an Executor through `ctx.actions` and is applied under the parameter's declared update class. An Action with an undeclared class was rejected at admission (RS-17) and never arrives.

### Sink, Link and Authority

- **MA-25** `Sink` has `descriptor()`, `prepare(fragment, ctx)`, `arm`, `start`, `step`, `stop(reason) -> [ArtifactRef]` and `cleanup`. Every Sink is stepped in the Simulation class and runs on a thread in the other three (MA-30), so the decision is the class's and no descriptor flag carries it; a Sink that leaves `step` at the default and therefore never drains is a defect the stepping test catches. Every link whose consumer port belongs to a Sink-role Module is drop-class, which is SC-21's predicate: the Kernel reads the role from the `ModuleDescriptor` of the Module that supplies the component, which for a Session comes from the placement's `module` field (SB-25a).
- **MA-26** `stop` returns its `ArtifactRef`s even on an abort, with `partial` set (RS-44). A Sink whose `stop` fails leaves its artifacts recorded as unknown in the Manifest.
- **MA-27** `Link` has `descriptor()` and `create(&DataLinkDecl) -> DataLink`, returning an implementation of spec 02's link interface, whose two ends the coordinator attaches to the producing and consuming ports as `Endpoint`s. Links are created before the Providers, Executors and Sinks are prepared, and dropped at cleanup; they have no lifecycle of their own.
- **MA-28** A Link implements its declared policy exactly (SC-19, SC-20). `connects` and `cross_process` are the inputs to the memory-domain reachability check of MA-39; the Kernel never plans a transfer (SB-40). Event and Action queues between Islands are Kernel handles, not Link products.
- **MA-29** There is one Authority instance per Run, named by the BindingProfile (SB-24). It provides a `TimeAuthority` handle (TM-16a) and declares an `AuthorityDescriptor` whose `governs` is the set TM-16a requires: a primary root, that root's derived domains, `host.monotonic`, and for a simulation Authority every root it simulates. Its `pacing` is cross-checked against the derived ExecutionClass by MA-41.
- **MA-30** The coordinator owns the stepping loop, on one logical thread:

  ```text
  until = authority.next_wakeup()        None ends the Run
  repeat: step(until) over every stepped instance in a fixed order
          role rank Provider < Executor < Sink, then instance id
  until no instance reports progressed; a cap of 1 000 iterations raises STEP_LIVELOCK and aborts
  ```

  | class | stepped Providers | Executors | Sinks |
  |---|---|---|---|
  | Simulation | step | step | step |
  | RealtimeEmulation | step, wall-paced inside `next_wakeup` | threads | threads |
  | HardwareInLoop, Hardware | none | threads | threads |

  The order is fixed by rule and not by registration, so determinism does not depend on the order in which a runtime happened to assemble its Modules (Vision §58 #3). *Ceiling: a zero-latency Event or Action cycle at one instant hits the cap; the Simulation Engine of Phase 2 adds delivery latency on inter-Island Action edges, and TM-17b gives the Authority the same cap for the same reason.*

### Descriptors and the registry

- **MA-31** Every Module ships a `ModuleDescriptor`, and its `roles` must match the factories it registers.
- **MA-32** Registration fails when: `kernel_api.major` differs from the Kernel's; a declared Vocabulary is absent or incompatible; a role has no factory or a factory no role; `(id, version)` is already registered; or `deployment` is `Plugin`, which is `Unsupported` in Phase 1. Registration is explicit in the runtime's assembly code; there is no link-time registration crate, whose order is opaque and whose failures are silent.
- **MA-33** Compatibility is the caret rule: a requirement `^a.b.c` is satisfied by a registered `x.y.z` when `a == x` and, for `a > 0`, `(y, z) >= (b, c)`, or for `a == 0`, `y == b` and `z >= c`. The Kernel compares ids and versions and never interprets a key.
- **MA-34** Every capability, constraint and parameter key is `<vocabulary prefix><path>` or `ext.<module-id>.<path>`. The Kernel refuses a key whose prefix belongs to no Vocabulary the declaring Module declares, naming the prefix, and validates the value's shape against the `KeyDecl`. It never interprets the meaning (SB-2, audit Finding 7).
- **MA-35** A `VocabularyDescriptor` carries what the Kernel is obliged to enforce on that Vocabulary's behalf but never interprets: its `KeyDecl`s (SB-2), its event kinds with their default reactions and severities (RS-27, RS-28), its Session verbs and how each compiles (RS-13a), and its admission checks (SB-29). Each of those four is a place where an earlier draft had put Vocabulary content in the Kernel.
- **MA-36** A `ComponentDescriptor` has the shape of §4. `requires` states requirements only; the placement lives in the BindingProfile (SB-25, re-review R21).
- **MA-37** Validation of a descriptor is structural: unique port names, registered contract ids, update classes from the closed set, a finite budget, an `impl.hash` present. A descriptor never carries an `AbsoluteDeadline` and a `TxBurst` never carries a `RelativeBudget`; they are distinct types (TM-15) and distinct schema definitions.

### Islands, class and fidelity

- **MA-38** An `IslandDecl` names an Executor instance, the components placed on it, and its optional affinity, real-time policy and preferred batch. Where it lives and the requirement that every component be placed exactly once are SB-13 and SB-25.
- **MA-39** Island admission checks: every component placed exactly once; `requires.executor_kind` is `any` or the Executor's kind, `impl.kind` is among its `impl_kinds`, and the placement's memory domain is among its `memory_domains`; a link within an Island shares a memory domain or has a registered Link that `connects` the pair, and a link between Islands has a declared policy; MA-22's cycle rule; and an Island with an `rt_policy` requires a declared budget on every component. *The arithmetic feasibility check, that the sum of the budgets fits the block period, needs declared port rates from the Radio Model; Phase 1 checks presence only. Forward obligation, Phase 2.*
- **MA-40** Admission rejects; it never creates, merges or moves an Island (Vision §20, §63). A rejection names the rule it failed.
- **MA-41** The ExecutionClass is derived from the environment and cross-checked against the Authority's pacing, and it is what spec 03's plan records (SB-39):

  | Authority | pacing | RF path | class |
  |---|---|---|---|
  | Simulation Engine | free-running | simulated | Simulation |
  | Simulation Engine | wall-paced | simulated | RealtimeEmulation |
  | device timekeeper | — | cabled, or partly simulated | HardwareInLoop |
  | device timekeeper | — | over the air | Hardware |
  | Simulation Engine | any | over the air, or cabled | rejected |

  A simulated Authority cannot drive a real RF path, and a class that was merely declared could lie.
- **MA-42** The value sets of `Fidelity` add `real` to those of Vision §14, which stop at `hardware_quirk` and leave a Hardware Run with nothing to record although §14 says every Run records the vector. A Run's value per aspect is the weakest over its bound Providers, in that aspect's own order: `none < envelope < hardware_quirk < real` for timing and continuity, `none < grid < real` for coercion, `none < impairment_model < real` for RF, and `none < model < real` for transport, so a Hardware Run with a best-effort MockPeripheral records `timing: envelope`. An `ideal` Mock profile declares `none` (Vision §13).
- **MA-43** *Withdrawn.* It restated spec 04's RS-42 verbatim, citation included.

### Test doubles and the Plugin boundary

- **MA-44** The Phase 1 test-double Provider lives in `tests/support/` and is never shipped. It declares a `test` Vocabulary at version 1.0.0 with the keys `test.count`, `test.grid` (coercible, snapping to an injectable grid, defaulting to `reject`) and `test.flag`; a two-level tree of a device with two `test.line` sub-resources; `fidelity` all `none`; `driving.stepped` false; a recorded call log; an injectable failure phase; an event generator; and a drain of its action queue. It uses no radio word, which is how the matcher is proven generic (OV-21). It has no time model, no blocks and no envelope: that boundary is exactly where Phase 2's MockRadio begins.
- **MA-45** Phase 1 also ships a recording `TestExecutor` of about twenty lines, which performs no processing. Without it MA-19, MA-20, MA-30 and MA-39 have no runtime test until Phase 2, and the stepping loop is the one mechanism on which Vision §58 #3 depends.
- **MA-46** What is fixed now so that a Plugin protocol later needs no Kernel change: the schemas of `ModuleDescriptor`, `VocabularyDescriptor`, `ProviderInstance` and `Resource`, the four other role descriptors, `ComponentDescriptor`, `IslandDecl`, `Fragment`, `Requested`, `PrepareReport`, `CoerceReport`, `ModuleError`, `StopCause`, `ArtifactRef`, `ExecutionClass` and `Fidelity`, together with spec 04's `Event` and Action schemas; rule MA-6; and the reserved `Plugin` deployment variant. A Plugin host is then one Module implementing a role trait by proxy, with the handles becoming streams: an `EventSink` an event stream, an `ActionReceiver` an inbound action stream, an **`ActionSubmitter` an outbound request-and-response stream** carrying the admission result back, an `Endpoint` a shared-memory Link, and the `TimeAuthority` a request and response. The submitter has to be in this list: adding a way to emit an Action after v4.0 would be a Kernel major, and a Plugin host built to this mapping without one could host a Provider but never a Reactor.

## 6. Decisions

| # | Decision | Choice | Rejected (one line each) | Ceiling / upgrade path |
|---|---|---|---|---|
| M1 | Trait style | Synchronous, object-safe, `Send` (MA-5) | `async fn` in a trait (not object-safe without boxing, and it pulls a runtime into the Kernel that every Provider inherits); `async-trait` (an allocation per call, and a dependency) | A blocking `prepare`, and PPS synchronisation takes up to two seconds, so fragments serialise; running independent fragments on threads is a later change with no trait change |
| M2 | A shared lifecycle supertrait | None; one trait per role, shared method names by convention (MA-2) | One `Lifecycle` supertrait (Finding 22's exact complaint, and Link and Authority have no lifecycle) | A sixth role adds a trait, not a supertrait |
| M3 | How Actions reach a Module | A queue handle in `PrepareContext` (MA-14) | A `dispatch(&mut self, Action)` method (a coordinator lock on the real-time path) | Latency is unbounded only if the Module does not drain; the depth and policy are spec 04's |
| M4 | Stepping on a Provider | `step` with a default no-op plus `driving.stepped` (MA-15) | A separate `Steppable` trait: the coordinator holds each instance as a role trait object and could not ask whether it is also `Steppable` without downcasting, while `driving.stepped` is data it already reads; requiring `step` on every Provider (pointless for hardware) | A hardware Provider ignores it; MockRadio overrides it |
| M5 | Who owns the stepping loop | The Kernel coordinator, driven by `Authority::next_wakeup` (MA-30) | The Simulation Engine owning it through shared mutable handles (determinism rules would live in a Module, and the Kernel would hand out `&mut` to its own instances) | The livelock cap is 1 000; Phase 2's Engine adds edge latency |
| M6 | Registry population | Explicit at assembly (MA-32) | Link-time registration crates (a dependency, order-dependent, and silent when it fails) | none |
| M7 | Version type | Hand-written, release-only (MA-33) | The `semver` crate (pre-release and build metadata are unneeded for Module versions, and spec 03's B7 keeps document versions to a single integer) | No pre-release tags; adopt `semver` if a Vocabulary needs them |
| M8 | Does the Kernel check keys? | Prefix membership and value shape only (MA-34) | No check (a typo passes silently and binds nothing); a key registry in the Kernel (Finding 7) | Meaning stays in the Vocabulary |
| M9 | Fidelity for hardware Runs | Add the value `real` (MA-42) | Omitting the vector on a Hardware Run (Vision §14 says every Run records it); reusing `hardware_quirk` (it would read as a model) | Recorded as open question 1 for the owner |
| M10 | ExecutionClass | Derived from the environment, cross-checked against pacing (MA-41) | Declared in the profile (it could lie); derived without the cross-check (a mismatched Authority passes silently) | none |
| M11 | `ComponentDescriptor.requires` | Two typed fields (MA-36) | A generic constraint map under an `exec.*` prefix (it would need a Kernel-defined Vocabulary anyway) | A third requirement is an additive field |
| M12 | What a `VocabularyDescriptor` carries | Keys, event kinds, verbs and admission checks (MA-35) | Leaving each in the Kernel, which is where an earlier draft had all four | Each is data the Kernel enforces and never interprets |

## 7. Phase 1 tests

| test | input | expected | rules |
|---|---|---|---|
| `ma_05_traits_are_object_safe_and_send` | `Box<dyn Provider + Send>` and the other four, at compile time | compiles | MA-5 |
| `ma_06_signature_types_are_documents` | a compile-time bound requiring every parameter and return document type to be serialisable with a schema | compiles | MA-6, MA-46 |
| `ma_32_registry_refusals` | a missing Vocabulary; an incompatible one; a `kernel_api` major mismatch; `deployment: Plugin`; a role with no factory; a duplicate `(id, version)` | refused in each case, naming the cause | MA-31, MA-32 |
| `ma_33_caret_table` | requirements and registrations either side of 1.0 | the table of MA-33 exactly | MA-33 |
| `ma_34_key_prefix_refused` | a key whose prefix belongs to an undeclared Vocabulary | refused, naming the prefix | MA-34, SB-2 |
| `ma_11_coerce_is_pure` | the same request twice, with no hardware present | identical reports | MA-11 |
| `ma_12_prepare_matches_coerce` | a coercing request through `coerce` then `prepare` | identical coercions | MA-12, SB-44 |
| `ma_12_narrowing_triggers_readmission` | an `effective` that narrows below the Spec's constraint | re-admission fails; a widening `effective` is refused | MA-12, MA-17 |
| `ma_07_lifecycle_order_and_cleanup` | an injected failure at each of prepare, arm and start | the call log shows the order of MA-7 and a reverse-order cleanup | MA-7, MA-9, RS-6 |
| `ma_26_sink_returns_partial_on_abort` | an abort with a capture open | `stop` returns refs with `partial` set | MA-26, RS-44 |
| `ma_15_provider_step_default_is_idle` | a hardware-style double | `progressed` false, no override needed | MA-15 |
| `ma_14_actions_arrive_only_after_admission` | an Action rejected by an admission check | the double's queue stays empty | MA-14, RS-16 |
| `ma_39_island_admission` | each of the five checks violated in turn | rejected, naming the failed rule | MA-39, MA-40 |
| `ma_22_cycle_rules` | a `stream.*` cycle; an Event cycle across Islands; an Event cycle inside one Island | rejected, accepted, rejected | MA-22 |
| `ma_41_execution_class_table` | every row of MA-41 including the rejection | the derived class, or a rejection | MA-41 |
| `ma_42_fidelity_is_the_weakest` | two Providers declaring `hardware_quirk` and `envelope` | `envelope`; a Hardware Run records `real` where declared | MA-42, RS-41 |
| `ma_37_descriptor_structural_validation` | a duplicate port name; an unregistered contract; an update class outside the set; a missing `impl.hash` | refused in each case | MA-37 |
| `ma_30_stepping_order_and_quiescence` | three stepped instances registered in reverse order | stepped in role and id order, not registration order; the loop quiesces | MA-30, MA-45 |
| `ma_30_stepping_livelock_cap` | two components that reschedule each other at one instant | `STEP_LIVELOCK` and an abort, not a hang | MA-30 |
| `ma_25_sink_role_predicate` | a link into a Sink-role Module with `Block`, resolved through the placement's `module` | refused | MA-25, SC-21, SB-25a |
| `ma_10_sub_resource_binding` | the double's two-level tree, with a resource bound to a `test.line` | bound to that node's `ResourceId` | MA-10, SB-34 |
| `ma_35_vocabulary_carries_its_own_content` | a `test` Vocabulary declaring a key, an event kind with a default, a verb and a check | all four are enforced by the Kernel and none is interpreted by it | MA-35, RS-27, RS-13a, SB-29 |
| `ma_04_kernel_surface_ban` | the Kernel source | no banned token outside a comment citing the ban | OV-23a |
| `ma_14a_reactor_emits_through_admit` | a test Executor submitting a `TxBurst` through `actions_out`, once inside and once outside the `test.limits` envelope | admitted and dispatched; then rejected before reaching the Provider | MA-14a, RS-16 |

## 8. Vision coverage

| Vision | This spec |
|---|---|
| §4 the microkernel picture and the Module registry | §3, MA-31…MA-33 |
| §5's Module API line; the Kernel stays small | MA-1…MA-4, MA-35 |
| §7 the three axes, the five roles, the communication rule | MA-1…MA-3 |
| §14 ExecutionClass and the fidelity vector | MA-41…MA-43 |
| §15 `step(until)`, the Authority role | MA-20, MA-29, MA-30 |
| §19 descriptor versus ABI, cycles, the deadline kinds | MA-21, MA-22, MA-36, MA-37 |
| §20 placement validated, the executor-kind requirement | MA-36, MA-38…MA-40 |
| §27 update classes in the descriptor | MA-24, MA-36 |
| §32 Islands and the driving model per class | MA-30, MA-38, MA-39 |
| §35 the bridge: typed status, exceptions translated, `DEVICE_LOST` | MA-6, MA-9 |
| §36 the generic outputs a Provider offers | MA-10, MA-35 |
| §37, §38 the Peripheral shape and the timing class per instance | MA-17 |
| §39 GPIO as a sub-resource, no cross-module coupling | MA-3, MA-16 |
| §42 a WASM Executor consumes the same descriptor and schemas | MA-21, MA-46 |
| §62 process boundaries, compiled in by default | MA-6, MA-32, MA-46 |

## 9. Vision issues found

1. **§38 says a Peripheral's timing class "is returned as a capability at `prepare` time", while §10's pipeline matches capabilities before `prepare`.** A class first visible at `prepare` could not be matched. MA-17 puts it in `instance()`, where `validate()` can see it, and lets `prepare` narrow it.
2. **§14's fidelity value sets stop at `hardware_quirk`**, so a Hardware Run has no value to record although the same section says every Run records the vector. MA-42 adds `real`. This is open question 1 in `00-overview.md`.
3. **§7's role list and audit §13's `module-api` line disagree on one member.** Audit §13 names four roles, `Provider`, `Executor`, `Sink` and `Link`; the Vision names five, adding `Authority` from re-review R8. The Vision is later and is followed here.
4. **§32 says the Simulation Engine "step-drives every Island", which reads as the Engine owning the loop.** MA-30 puts the loop in the Kernel coordinator with the Authority deciding when to wake: the Engine still decides the instants, and the fixed step order is what keeps determinism independent of assembly order. This is open question 3 in `00-overview.md`.

## 10. Deferred

The Radio Model's traits and keys, and the contents of `TimingEnvelope` and `PerformanceEnvelope`, which are opaque `sections` in Phase 1 (Phase 2). MockRadio and the Simulation Engine (Phase 2). The Peripheral and Endpoint vocabularies (Phase 9). The native, WASM and GPU execution ABIs, which each Executor owns (Phases 10 and 11). The Plugin wire protocol (Phase 9); MA-46 is what makes it additive. Out-of-process Radio Providers (Vision §64). The budget-feasibility arithmetic of MA-39, which needs declared port rates (Phase 2).
