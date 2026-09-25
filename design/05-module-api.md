# Phase 1 spec 05 — Module API

| Field | Value |
|---|---|
| Status | Accepted 2026-09-23 (Gate C; Phase 1 Step 5). Normative for `ezsdr-kernel::module_api`. Amended in Phase 2 by KA-1, KA-5, KA-7, KA-8, KA-9, KA-10, KA-11, KA-12, KA-13, KA-15, KA-19. Amended in Phase 3 by KB-1, KB-2. |
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
ModuleId          SB-1's ModuleId grammar (table SB-T0), e.g. ezsdr.radio.mock
Version           { major, minor, patch }                release-only; no pre-release tags (SB decision B7)
VersionReq        a caret requirement over a Version
Role              Provider | Executor | Sink | Link | Authority
Deployment        InProcess | Plugin { protocol: Version }

ModuleDescriptor  { id: ModuleId, version: Version, kernel_api: Version, roles: [Role],
                    vocabularies: [{ id: Namespace, req: VersionReq }],
                    deployment: Deployment, impl_hash: optional ContentHash }
VocabularyDescriptor { id: Namespace, version: Version, prefix: Namespace, keys: [KeyDecl],
                       event_kinds: [{ kind: EventKind, default: Reaction, severity: Severity }],
                       verbs: [{ verb: Ident, compiles_to: CompileRule }],
                       checks: [AdmissionCheck] }

ProviderInstance  { id: ResourceId, module: { id, version }, profile: optional { name, version },
                    tree: Resource, fidelity: Fidelity, driving: { stepped: bool },
                    arm_after: [ResourceId], sections: Map<Namespace, Value> }
Resource          { id: ResourceId, kind: Namespace, shareable: bool (default false),
                    capabilities: Map<Key, CapabilityValue>, children: [Resource],
                    ports: [Port] }   a stream node's Ports, which a Spec may link to (SB-15)
CoerceReport      { applied: Map<Key, Value>, coercions: [Coercion],
                    warnings: [Warning], rejected: [{ key, requested, reason }] }

ExecutorDescriptor { module: ModuleRef, kind: Namespace, memory_domains: [MemoryDomainId],
                     impl_kinds: [Namespace], capabilities: Map<Key, CapabilityValue> }
SinkDescriptor     { module: ModuleRef, kind: Namespace, memory_domains: [MemoryDomainId],
                     contracts: [DataContractId], artifact_kinds: [Namespace] }
LinkDescriptor     { module: ModuleRef, kind: Namespace, connects: [(MemoryDomainId, MemoryDomainId)],
                     policies: [BackPressure], cross_process: bool }
AuthorityDescriptor { module: ModuleRef, governs: [ClockDomainId],
                      pacing: FreeRunning | WallPaced | Device }

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
PrepareContext    { run: RunId, class: ExecutionClass,
                    time: shared TimeAuthority handle, clocks: shared ClockRegistry handle,
                    events: shared EventSink, actions: shared ActionReceiver,
                    actions_out: shared ActionSubmitter,
                    environment: shared read-only Map<Namespace, Value>,
                    inputs: shared read-only InputStore,
                    links: [{ component: Ident, port: Ident, endpoint: Endpoint }],
                    components: Map<Ident, ComponentDescriptor>, host_budget: RelativeBudget }
ActionSubmitter   submits a proposed Action into admit() (RS-16), never to a Module directly
InputStore        get(ContentHash) -> shared bytes or none: the Run's inputs, read-only (RS-44a, KB-1)
StepOutcome       { progressed: bool }
ModuleError       { kind: Rejected | Unsupported | Timeout | DeviceLost | Internal,
                    message: string, detail: Value }
```

## 5. Normative rules

### The axes and the roles

- **MA-1** The three axes of §3 are orthogonal. A Module declares its roles in its `ModuleDescriptor` and may hold several; `Plugin` names a deployment and never a role (Vision §7, audit Finding 22).
- **MA-2** There are exactly five roles and one trait each. There is no shared lifecycle supertrait: where two traits have methods with the same name it is by convention, and the coordinator dispatches per role. One trait forced onto radios and executors alike is what Finding 22 rejects, and Link and Authority have no lifecycle at all.
- **MA-3** A Module communicates only through Kernel-defined Resources, Ports, Events, Actions, Capabilities and DataContracts. In source terms, a Module crate depends on the Kernel crate and on Vocabulary crates, never on another Module crate. Binding to a sub-resource of another Module's device is resolved by the Kernel (SB-36) and is not a dependency (Vision §7, §39). *Checked in Phase 2 by `ma_03_no_module_crate_depends_on_another` (PO-8).*
- **MA-4** *Withdrawn.* It restated `00-overview.md` OV-23a, which OV-4 forbids. The ban stands there and `kernel_surface` enforces it; the `ma_04_kernel_surface_ban` test row now cites OV-23a.

### Rules common to every role trait

- **MA-5** Role traits are synchronous, object-safe and `Send`. No method is generic, none returns `Self` or an opaque type, and no trait is `async`. An async trait would not be object-safe without boxing and would pull an executor runtime into the Kernel, which every Provider would then inherit.
- **MA-5a** A Module may keep every handle its `PrepareContext` carries and use it from `prepare` through `cleanup`; the coordinator keeps each one valid until the Run is `CleanedUp`. A handle is shared (`Arc`), so a Module that keeps one keeps it alive; it must drop them in `cleanup` (MA-7). The `environment` is the BindingProfile's `environment` verbatim and read-only: a Module reads the sections its own Vocabularies define and no other (OV-21's rule, applied to Modules), and the Kernel still reads only SB-26's four. `inputs` is the Run's input store: the bytes of every input KC-9 verified and of every waveform KC-28 ingested, keyed by content hash and read-only. A Session's waveform is stored before the Action that names it is dispatched, so a Provider that transmits a `TxBurst`'s samples reads its waveform there (Phase 3, KB-1). *Checked by `ma_05a_a_module_keeps_its_handles_after_prepare`: a stepped test Provider stores `events` and `time` at `prepare` and emits an event and reads `now` in `step`; the event reaches the Manifest.*
- **MA-6** Every parameter and return type is one of three things: a document type with a schema, a Kernel handle (`PrepareContext`, the container that carries the others, `EventSink`, `ActionReceiver`, `ActionSubmitter`, `InputStore` (Phase 3, KB-1), `Endpoint`, the `DataLink` a Link creates and an `Endpoint` holds, a `TimeAuthority` handle), or `ModuleError` — or an `Option`, `Result`, `Vec`, `Box`, `Arc` or reference over those. Never a raw slice, a closure, an iterator or a generic parameter. This is what lets a Plugin host implement any role later by message passing without a Kernel change, and it is the rule v3's string-keyed `setParam` violated. *Checked by `ma_06_role_signatures_name_only_documents_and_handles`, which reads every role-trait signature in `src/` and refuses a name outside that list or a slice, `impl Trait` or bare `fn` type, and asserts at compile time that each listed document type is serialisable with a schema. It supplements `ma_06_signature_types_are_documents`, whose compile-time bound over a hand-written list no signature was checked against was MA-6's only carrier, and which now carries MA-46 (finding D100).*
- **MA-7** The coordinator calls lifecycle methods, one call at a time per instance, in the order `prepare → arm → start → (step)* → stop → (step)* → cleanup`. `step` follows `stop` only in an orderly cleanup of a Simulation Run, where cleanup step 3's drain steps every stepped instance until nothing is scheduled, so that a Provider delivers its declared tail (MA-13) and an Executor or Sink receives it before its own `stop`; after an abort no `step` follows `stop` (Phase 2, KA-12). `stop(reason)` is always attempted before `cleanup`, on an abort as well. `cleanup` is infallible and idempotent and is called for every instance that reached `prepare`, in reverse dependency order, after a failure at any phase (RS-6, RS-8). `prepare` is called once per **fragment**; `arm`, `start`, `step`, `stop` and `cleanup` once per **instance**. An instance's position in the arm and start order is the position of its first fragment in the plan; in a per-fragment cleanup step (RS-6) it is acted on at the first of its fragments the reverse pass reaches, and not again. A Provider may refuse a second fragment in `prepare` with `Rejected` when it serves one resource per instance, as MockRadio does (MR-7) (Phase 2, KA-9). *The single-instance order and idempotence are checked in Phase 1. Checked in Phase 2 by `kc_13_arm_and_start_follow_instance_order_cleanup_reverses_it` (KC-13) and `ma_07_an_instance_with_two_fragments_is_prepared_twice_and_armed_once` (KA-9).*
- **MA-8** `prepare` and `arm` are bounded by `PrepareContext.host_budget`; a Module that cannot finish returns `ModuleError { kind: Timeout }`. Enforcing the budget and returning `Timeout` is the Module's duty. *Checked in Phase 2 by `mr_07_prepare_cases` (MR-7) and `hd_09_prepare_cases` (HD-9); the tests supply a host budget and complete prepare, but do not assert timeout enforcement.* *Ceiling: an in-process Module that hangs hangs the transaction; spec 04's RS-8a gives cleanup its own deadline, and running fragment calls on a worker thread with a join timeout is the upgrade, with no trait change.*
- **MA-9** A fault crosses the boundary only as `ModuleError`. No panic crosses a trait boundary, and a foreign exception is translated to a status (Vision §35). `DeviceLost` is the kind that spec 04's Policy maps to `abort`. *The trait return type is checked in Phase 1 (MA-6). Checked in Phase 2 by `kc_30_a_panicking_module_fails_the_run_not_the_process` (KC-30) and `kc_30_device_lost_is_the_kernel_event_and_aborts` (KC-30); foreign exception translation remains Phase 9.*

### Provider

- **MA-10** `instance()` returns the `ProviderInstance`, exposing the composite resource tree with capabilities declared per node, so that the matcher can bind at any node and a sub-resource carries its own `ResourceId` (SB-33). A node that is a stream endpoint also declares its `ports`, so that a Spec's `graph.links` may name one and SC-3 has a contract to check against; Vision §7's own correct diagram is `PHY Processor -> SampleStream -> Radio Port`, and without declared Ports that shape is not expressible and a Provider has nowhere to deliver a block on a Spec Run (SB-15, SC-1, SC-3). Which memory domain a node's port delivers from is Vocabulary content and a Phase 2 value; Phase 1 fixes only the mechanism. Each node also declares whether more than one Spec resource may bind to it, as `shareable`, default `false` (SB-34). The Kernel enforces the flag and never decides it: which kinds are shareable is Radio Model knowledge, which OV-21 keeps out of the Kernel. `ProviderInstance.min_command_lead` is the least lead this instance needs between receiving a timed Action and that Action's instant, as a `Duration` in `host.monotonic`; absent means zero. It is the only envelope value the Kernel reads: RS-19's earliest instant and KC-19's plan-time lead check use it. A Vocabulary that defines a timing envelope defines how its own key relates to it (RM-6). `validate()` refuses one in another domain or negative (SB-22f) (Phase 2, KA-7). A Provider's `instance()` returns the same `id`, `tree`, `driving`, `arm_after` and `min_command_lead` for the whole Run; `fidelity` is final when `prepare` returns, because what a simulated Provider models can depend on the environment it reads there — a SimulationChannel makes MockRadio's `rf` aspect `impairment_model` (MR-31); after that only `sections` may change, as the Provider records what happened (Phase 3, KB-2). *Checked in Phase 2 by `mr_02_the_tree_has_the_radio_model_shape` (tree and Port declaration) and `mr_13_ramp_values` (block buffer's host memory domain) (MA-10).* *Checked in Phase 3 by `kb_02_the_manifest_records_the_fidelity_settled_in_prepare` (KB-2).*
- **MA-11** `coerce(request)` is pure, deterministic and requires no hardware. It is a function of the declared profile, which is what makes `validate()` a true dry run (Vision §52). Two calls with the same request return identical reports. `coerce` is called at every `validate()`, not only when a key needs coercion, so it must be cheap; and it is where a Provider refuses a combination its envelope does not admit (SB-7).
- **MA-12** `prepare(fragment, ctx)` returns a `PrepareReport` whose `coercions` equal what `coerce` returned for the same request (SB-44). `effective` may narrow a declared capability and must not widen one, and the coordinator re-runs constraint matching over `effective`, which is also Vision §52's re-check of applied values (SB-30). Both read each resource's **own** report, not the merged map, which SB-41 makes lossy for a key two fragments name; and a key the report declares a coercion for is SB-46's, not this rule's. Both are `prepare`-stage checks, run inside `collect_prepare` (SB-41).
- **MA-13** `arm` reserves and synchronises and radiates nothing. `start(at)` begins at that instant or as soon as possible. `stop(mode)` is graceful under an orderly stop, delivering the declared tail, and immediate under an abort; the mode is what reaches each Provider (RS-9), while the `StopCause` stays on `Action::Abort` and in the Termination record. `cleanup` releases and restores baseline. *The order and mode reaching `stop` are checked in Phase 1. Checked in Phase 2 by `mr_25_orderly_stop_delivers_the_tail_abort_does_not` (MR-25).*
- **MA-14** Actions reach a Provider only through `ctx.actions`, and only after Kernel admission (RS-16). A constraint that can only be checked at run time, such as the lead of a Reactor's burst, is enforced by the Provider, which emits the typed event; it never silently accepts.
- **MA-14a** A Module **emits** an Action through `ctx.actions_out`, which submits it to `admit()` (RS-16) rather than to another Module. Without a sender the inbound queue would have no producer: Vision §5 and §19 define the Action set as what a Reactor emits into the real-time path, and §19's worked example is a Reactor that receives a decoded packet and schedules a `TxBurst`, so the reactive half of the architecture would have no interface at all. Routing through `admit()` rather than to the target keeps invariant 42 true for a Reactor's Action exactly as for a Session's, and it is why the handle is a submitter rather than a queue into a peer.
- **MA-15** `step(until)` has a default no-op body on `Provider`. A Provider whose `driving.stepped` is true overrides it under MA-20's contract; a hardware Provider never receives a `step`.
- **MA-16** No role trait reaches another role trait, so the Kernel role API gives a Module no reference to a peer Module instance. *Checked by the `ma_16_role_trait_signatures_do_not_name_peer_roles` `syn` walk, which follows from each role trait the routes its documentation lists and fails on reaching another role trait, with one red route per case in `ma_16_the_gate_sees_a_peer_behind_a_context_field`. A walk over names cannot see through a rename, so `src/` contains no `use … as` and no `macro_rules!` other than the ones the gate lists (`ma_16_the_names_the_walk_reads_are_the_names_declared`): refusing the two mechanisms closes that class, where listing the spellings they produce did not (finding D100).*
- **MA-16a** A route the walk does not follow — a conversion defined on another type, a free function, a `static`, a blanket impl — is kept out by the allow-list review of `00-overview.md` OV-23 at each gate and at the v4.0 freeze. A route found later is recorded here, and the walk is extended only when such a route appears in `src/`, never for a demonstration: five review passes each demonstrated one, and the rule text had become a list of them. *Process obligation, the gate reviews and the freeze review (finding D100).*
- **MA-17** An instance-level capability such as a Peripheral's timing class is knowable at instantiation, from the selector and profile, and appears in `instance()`. Vision §38 said, before Step 5, that the class "is returned as a capability at `prepare` time", but the pipeline matches capabilities before `prepare` (SB-37), so a class first visible at `prepare` could not be matched. `PrepareReport.effective` may narrow it (MA-12), which is what §38's sentence is really about. *Producer obligation of the first Provider with a Peripheral (Phase 9); MockRadio 1.0.0 has none. The Kernel half — matching reads the bound instance's capabilities (SB-37) and `prepare` may only narrow them — is checked under MA-12 (D80).*

```rust
pub trait Provider: Send {
    fn instance(&self) -> &ProviderInstance;                                    // MA-10
    fn coerce(&self, request: &Requested) -> Result<CoerceReport, ModuleError>; // MA-11
    fn prepare(&mut self, f: &Fragment, ctx: PrepareContext)
        -> Result<PrepareReport, ModuleError>;                                  // MA-12
    fn arm(&mut self) -> Result<(), ModuleError>;
    fn start(&mut self, at: Option<TimePoint>) -> Result<(), ModuleError>;      // MA-13
    fn stop(&mut self, mode: StopMode) -> Result<(), ModuleError>;         // MA-13, RS-9
    fn cleanup(&mut self);                                                      // MA-7
    fn step(&mut self, _until: TimePoint) -> Result<StepOutcome, ModuleError> { // MA-15
        Ok(StepOutcome { progressed: false })
    }
}
```

### Executor

- **MA-18** `descriptor()` returns the `ExecutorDescriptor`. Admission reads `kind`, `memory_domains` (non-empty, D86) and `impl_kinds`, and compares `module` with the binding (SB-22f, D82); everything else is opaque.
- **MA-19** `PrepareContext.components` is the typed channel by which an Executor receives its Island's `ComponentDescriptor`s, keyed by component name; `prepare(island, ctx)` receives them typed, never as JSON. *Checked by `ma_19_executor_receives_the_descriptors_for_its_island`, which proves the field reaches `prepare` typed and readable; populating it with exactly the Island's descriptors is MA-19a (D71).*
- **MA-19a** The Phase 2 coordinator populates `PrepareContext.components` with only the descriptors belonging to the Island being prepared. *Checked in Phase 2 by `kc_11_an_island_gets_exactly_its_components` (KC-11).*
- **MA-19b** The Executor loads each component by its `impl` identity and returns a `PrepareReport` for the Island. *Producer obligation, tested by the Phase 5 Reactor Executor; Phase 2 has no Executor Module, only the coordinator's test doubles.*
- **MA-20** `step(until)` must consume every input at or before `until`, emit every output and event at or before `until`, emit nothing after it, never block on input, never call `wait_until` (TM-16d), and report `progressed` true exactly when it consumed an input or produced an output. It is required on `Executor`, because without it a deterministic Run is a wish (audit Finding 18).
- **MA-21** The execution ABI is the Executor's. The Kernel defines no `work` or `process` signature and never inspects `impl` beyond its identity (audit Finding 8).
- **MA-22** Plan admission rejects a cycle formed by `stream.*` links. A cycle is legal only when every edge that closes it is an Event or Action edge crossing an Island boundary. Adaptive feedback inside a component is state, not structure (Vision §19).
- **MA-23** Where a component maps input blocks to output blocks one to one, the Executor propagates block flags unchanged by default (SC-17). A mapping that is not one to one is the Processor author's documented convention, not a Kernel type. *Forward obligation, tested in Phase 10.*
- **MA-24** An `UpdateParameter` reaches an Executor through `ctx.actions` and is applied under the parameter's declared update class. An Action with an undeclared class was rejected at admission (RS-17) and never arrives; an admitted one is applied under the parameter's declared update class as UC-2…UC-6 define it. *The refusal is RS-17's and is checked there. Checked in Phase 2 by `kc_25_an_admitted_update_changes_the_configuration` (KC-25), `mr_18_hardware_timed_updates` (UC-6), `mr_18_a_cold_rate_change_starts_a_new_sample_clock` (UC-3), and `hd_10_session_requests_are_sequential` (UC-4).*

### Update classes

- **UC-1** An `UpdateParameter` carries its key's declared class (RS-52), and the coordinator delivers it unchanged to its target (KC-25). The **effective instant** of an update is its `at` converted to the target's domain when present, and otherwise the first instant its class permits. The Kernel checks only that the class was declared (RS-17); UC-2…UC-6 bind the target. *UC-1 is the Kernel's; its test is KC-25's.*
- **UC-2** An update never takes effect before its effective instant, and a target never applies two updates of one key out of their effective-instant order; two with one effective instant apply in delivery order. *Producer obligation of every Provider, Executor and Sink; MockRadio's test is MR-18's.*
- **UC-3** `cold`: the target stops the function the key belongs to at the effective instant, applies the value and restarts it. A stream so restarted ends its SampleClock at that instant (`ClockRegistry::end`, TM-13c) and continues on a new SampleClock whose origin is that instant, so no block spans the change and the samples in between are neither delivered nor a gap (SC-12). An open transmit burst on such a stream ends there (`BurstEnd::Stop`). Absent `at`, the effective instant is the current instant. A `cold` update changes a value, never the graph's structure (RS-4). *Producer obligation.*
- **UC-4** `block_boundary`: every block — or, for a consumer that slices blocks, every sample — whose first sample instant is at or after the effective instant sees the new value; no block mixes old and new. Absent `at`, the effective instant is the current instant. *Producer obligation.*
- **UC-5** `atomic_realtime`: the value applies to every sample processed after the Action is received and not before its effective instant, with no state in which part of the value is applied. *Producer obligation; no Phase 2 key uses it.*
- **UC-6** `hardware_timed`: the device applies the value at exactly the effective instant; a device that applies commands only on a sample grid advances it to the next sample instant (MockRadio 1.0.0 applies at the instant itself, MR-18). Absent `at`, the effective instant is the current instant plus the target's command lead (`min_command_lead`, KA-7). An `at` closer than that lead is late: the target applies the value at the current instant plus its lead and emits its Vocabulary's late-command event. A pending `hardware_timed` update occupies one slot of the device's command queue until it applies, and a full queue refuses it with the Vocabulary's queue event. *Producer obligation; MockRadio's tests are MR-18's.*

### Sink, Link and Authority

- **MA-25** `SinkDescriptor.memory_domains` declares the domains a Sink reads from, at least one — an empty list is refused at validate — and MA-39 checks a feed against it (D81, D86). `Sink` has `descriptor()`, `prepare(fragment, ctx)`, `arm`, `start`, `step`, `stop(reason) -> [ArtifactRef]` and `cleanup`. Every Sink is stepped in the Simulation class and runs on a thread in the other three (MA-30), so the decision is the class's and no descriptor flag carries it; a Sink that leaves `step` at the default and therefore never drains is a defect the stepping test catches. Every link that feeds a Sink is drop-class, which is SC-21's predicate; which Module fills an output's Sink slot, and that it holds the role, is SB-22's (SB-22b, SB-22c, SB-22e). A Sink is bound, never placed — it is a role, not a component an Executor loads — so it carries no `ComponentDescriptor` and appears in no Island, and its own fragment is the one `plan()` emits per bound output.
- **MA-26** `stop` returns its `ArtifactRef`s even on an abort, with `partial` set (RS-44). A Sink whose `stop` fails is recorded as a `CleanupFailure` naming its fragment (RS-6, RS-8), and the Manifest lists no `ArtifactRef` for it: that is what "unknown" means, since the Kernel records no artifact it was not handed. *`partial` on abort is checked in Phase 1. Checked in Phase 2 by `kc_41_a_failing_sink_stop_is_a_cleanup_failure` (KC-41).*
- **MA-27** `Link` has `descriptor()` and `create(&DataLinkDecl) -> DataLink`, returning an implementation of spec 02's link interface, whose two ends can be attached to producing and consuming ports as `Endpoint`s. *Checked by `ma_05_traits_are_object_safe_and_send`.* An attached end names the component and the port it is bound to; for a Provider the component is the Spec resource name and for a Sink its output id (Phase 2, KA-15).
- **MA-27a** The coordinator creates selected Links before preparing Providers, Executors and Sinks, refusing an instance whose `descriptor()` differs from the descriptor registered for its `ModuleRef` (MA-28, D79), attaches their two ends, and drops them at cleanup; Links have no lifecycle of their own. *Checked in Phase 2 by `kc_10_link_descriptor_must_equal_the_registered_one` (KC-10) and `kc_10_both_ends_are_attached` (KC-10).*
- **MA-28** A Link's descriptor carries the Link Module's exact `{id, version}` as its `module` and is registered under it, one per version, so it cannot be filed under another version (D82); `LinkPlacement` selects that same `ModuleRef`. The descriptor declares supported policies and memory-domain pairs in `connects`; admission checks the selected descriptor (MA-39) and never plans a transfer (SB-40). `cross_process: true` is rejected in v4.0, at registration and again at admission. Event and Action queues between Islands are Kernel handles, not Link products. *Checked by `ma_28_link_registration_refuses_cross_process_in_v4` and `ma_39_a_cross_domain_link_inside_one_island_needs_a_registered_link` (D74).*
- **MA-28a** A Link implements its declared policy exactly (SC-19, SC-20). *Checked in Phase 2 by `hd_05_policies` (HD-5).*
- **MA-29** There is one Authority instance per Run, named by the BindingProfile (SB-24), and its `AuthorityDescriptor` names its Module's exact `ModuleRef`, which admission compares with the binding (SB-22f, D98). It provides a `TimeAuthority` handle (TM-16a) and declares an `AuthorityDescriptor` whose `governs` is the set TM-16a requires: a primary root, that root's derived domains, `host.monotonic`, and for a simulation Authority every root it simulates. Its `pacing` is cross-checked against the derived ExecutionClass by MA-41. `next_wakeup` **advances** the Authority's governed clocks to the earliest instant at which a scheduled callback is due, fires the callbacks due at that instant in TM-16c order — including ones they schedule at that instant, up to TM-17b's cap per call — and returns the instant; with nothing scheduled it returns `None` and moves nothing. A step-driven Module that needs to run at an instant schedules a callback there, which may do nothing (Phase 2, KA-11). *The descriptor checks are carried in Phase 1 by spec 03's SB-22e, SB-22f and SB-24 tests. Checked in Phase 2 by `kc_20_virtual_time_advances_only_through_next_wakeup` (KC-3).*
- **MA-30** The coordinator owns the stepping loop, on one logical thread:

  ```text
  until = authority.next_wakeup()        None ends a Spec Run (KC-33); a Session waits for its client
  repeat: step(until) over every stepped instance in a fixed order
          role rank Provider < Executor < Sink, then instance id
  until no instance reports progressed; a cap of 1 000 iterations raises STEP_LIVELOCK and aborts
  ```

  | class | stepped Providers | Executors | Sinks |
  |---|---|---|---|
  | Simulation | step | step | step |
  | RealtimeEmulation | step, wall-paced inside `next_wakeup` | threads | threads |
  | HardwareInLoop, Hardware | none | threads | threads |

  The order is fixed by rule and not by registration, so determinism does not depend on the order in which a runtime happened to assemble its Modules (Vision §58 #3). *Ceiling: a zero-latency Event or Action cycle at one instant hits the cap, and the coordinator also counts consecutive `next_wakeup` results at one instant, more than `STEP_ROUND_CAP` of which is `STEP_LIVELOCK` (KC-22). Delivery latency on inter-Island Action edges, which would let such a cycle advance in time, is not added in Phase 2, which has no Reactor and so no such edge; it is Phase 5's (Phase 2, KA-11).*

### Descriptors and the registry

- **MA-31** Every Module ships a `ModuleDescriptor`, and its `roles` must match the factories it registers.
- **MA-32** Registration fails when: `kernel_api.major` differs from the Kernel's; a declared Vocabulary is absent or incompatible; a role has no factory or a factory no role; `(id, version)` is already registered; or `deployment` is `Plugin`, which is `Unsupported` in Phase 1. Registration is explicit in the runtime's assembly code; there is no link-time registration crate, whose order is opaque and whose failures are silent.
- **MA-33** Compatibility is the caret rule: a requirement `^a.b.c` is satisfied by a registered `x.y.z` when `a == x` and, for `a > 0`, `(y, z) >= (b, c)`, or for `a == 0`, `y == b` and `z >= c`. The Kernel compares ids and versions and never interprets a key.
- **MA-34** Every capability, constraint and parameter key is `<vocabulary prefix><path>` or `ext.<module-id>.<path>`. The Kernel refuses a key whose prefix belongs to no Vocabulary the declaring Module declares, naming the prefix, and validates the value's shape against the `KeyDecl`. It never interprets the meaning (SB-2, audit Finding 7).
- **MA-35** A `VocabularyDescriptor` carries what the Kernel is obliged to enforce on that Vocabulary's behalf but never interprets: its `KeyDecl`s, each with the shape, the coercion default and the optional update class of one key (SB-2, RS-17), its event kinds with their default reactions and severities (RS-27, RS-28), its Session verbs and how each compiles (RS-13a), and its admission checks (SB-29). Each of those four is a place where an earlier draft had put Vocabulary content in the Kernel.
- **MA-36** A `ComponentDescriptor` has the shape of §4. `requires` states requirements only; the placement lives in the BindingProfile (SB-25, re-review R21).
- **MA-37** Validation of a descriptor is structural: unique port names, registered contract ids, a finite budget. `validate()` runs it over every component of `graph.components` before anything reads a port, because a duplicate port name otherwise resolves a link to whichever came first — a link the author meant to carry one contract, checked and planned as another. Update classes from the closed set and a present `impl.hash` are enforced one layer out, at the JSON boundary by the schema, because in Rust an `UpdateClass` is a closed enum and a `ContentHash` exists only parsed — the checks can only fail for a non-Rust producer, which is where they are tested. A descriptor never carries an `AbsoluteDeadline` and a `TxBurst` never carries a `RelativeBudget`; they are distinct types (TM-15) and distinct schema definitions.

### Islands, class and fidelity

- **MA-38** An `IslandDecl` names an Executor instance, the components placed on it, and its optional affinity, real-time policy and preferred batch. The `executor` name is a binding that fills an Executor slot (SB-22b, SB-22e); the runtime supplies only that instance's `ExecutorDescriptor` (SB-22f). Naming the instance without naming the Module in a document would leave the plan's `fragments[].instance` and the Manifest's `modules` resting on assembly-time input that nothing records, which is not provenance Vision §50 can rest on. Where it lives and the requirement that every component be placed exactly once are SB-13 and SB-25.
- **MA-39** Island admission checks: every component placed exactly once; `requires.executor_kind` is `any` or the Executor's kind, `impl.kind` is among its `impl_kinds`, and the placement's memory domain is among its `memory_domains`; each data link's selected `LinkPlacement` descriptor — a graph link's or an output feed's (SB-25) — supports its declared policy, and if the producer's placement and the consumer's domains — a component's placement, or for an output feed the domains its bound `SinkDescriptor.memory_domains` declares — share no memory domain, in one Island or in two, that selected Link `connects` the pair (D77, D81; a resource-endpoint producer is skipped, D31); MA-22's cycle rule; and an Island with an `rt_policy` requires a declared budget on every component. *The arithmetic feasibility check, that the sum of the budgets fits the block period, needs declared port rates from the Radio Model; Phase 1 checks presence only. Forward obligation, Phase 10, when a Processor declares port rates.*
- **MA-40** Admission rejects; it never creates, merges or moves an Island (Vision §20, §63). A rejection names the rule it failed.
- **MA-41** The ExecutionClass is derived from the environment and cross-checked against the Authority's pacing, and it is what spec 03's plan records (SB-39). Where the environment's `ezsdr.time` section is **present**, its `class` must be one of the four names and the derived class is compared with it; an absent, non-string or unrecognised `class` is refused rather than ignored, for the reason the `ezsdr.rf_path` clause gives — defaulting or ignoring turns a misspelling into agreement, and the class a misspelling lands on is the one that may claim determinism (RS-42). A mismatch is refused: "a class that was merely declared could lie" is only a rule if the declaration is read, and SB-26 lists that section as one the Kernel reads:

  | Authority | pacing | RF path | class |
  |---|---|---|---|
  | Simulation Engine | free-running | simulated | Simulation |
  | Simulation Engine | wall-paced | simulated | RealtimeEmulation |
  | device timekeeper | — | cabled, or partly simulated | HardwareInLoop |
  | device timekeeper | — | over the air | Hardware |
  | Simulation Engine | any | over the air, or cabled | rejected |

  A simulated Authority cannot drive a real RF path, and a class that was merely declared could lie. The section's fields are `class` and `start_lead_ns` (KC-15) and no other; one it does not name is refused (Phase 2, KA-8).
- **MA-42** The value sets of `Fidelity` add `real` to those of Vision §14, which stop at `hardware_quirk` and leave a Hardware Run with nothing to record although §14 says every Run records the vector. A Run's value per aspect is the weakest over its bound Providers, in that aspect's own order: `none < envelope < hardware_quirk < real` for timing and continuity, `none < grid < real` for coercion, `none < impairment_model < real` for RF, and `none < model < real` for transport, so a Hardware Run with a best-effort MockPeripheral records `timing: envelope`. An `ideal` Mock profile declares `none` (Vision §13).
- **MA-43** *Withdrawn.* It restated spec 04's RS-42 verbatim, citation included.

### Test doubles and the Plugin boundary

- **MA-44** The Phase 1 test-double Provider lives in `tests/support/` and is never shipped. It declares a `test` Vocabulary at version 1.0.0 with the keys `test.count`, `test.grid` (coercible, snapping to an injectable grid, defaulting to `reject`) and `test.flag`; a two-level tree of a device with two `test.line` sub-resources; `fidelity` all `none`; `driving.stepped` false; a recorded call log; an injectable failure phase; the `test.custom` event **kind**, declared by the Vocabulary; and a drain of its action queue. The double generates no Events: Phase 1's event tests drive `EventCollector` directly, and this list used to claim "an event generator" that `tests/support/doubles.rs` does not have (exit-review finding). It uses no radio word, which is how the matcher is proven generic (OV-21). It has no time model, no blocks and no envelope: that boundary is exactly where Phase 2's MockRadio begins.
- **MA-45** Phase 1 also ships a recording `TestExecutor` of about twenty lines, which performs no processing. Without it MA-19, MA-20, MA-30 and MA-39 have no runtime test until Phase 2, and the stepping loop is the one mechanism on which Vision §58 #3 depends.
- **MA-46** What is fixed now so that a Plugin protocol later needs no Kernel change: the schemas of `ModuleDescriptor`, `VocabularyDescriptor`, `ProviderInstance` and `Resource`, the four other role descriptors, `ComponentDescriptor`, `IslandDecl`, `Fragment`, `Requested`, `PrepareReport`, `CoerceReport`, `ModuleError`, `StopMode`, `StepOutcome`, `ArtifactRef`, `ExecutionClass` and `Fidelity` — which includes every document type a role-trait signature carries (MA-6) — together with spec 04's `Event` and Action schemas (`StopCause` among them, on `Abort`); rule MA-6; and the reserved `Plugin` deployment variant. A Plugin host is then one Module implementing a role trait by proxy, with the handles becoming streams: an `EventSink` an event stream, an `ActionReceiver` an inbound action stream, an **`ActionSubmitter` an outbound request-and-response stream** carrying the admission result back, an `Endpoint` a shared-memory Link, the `TimeAuthority` a request and response, the `ClockRegistry` a request and response, the `InputStore` a request and response (Phase 3, KB-1), and the `environment` a document sent once at `prepare`. The submitter has to be in this list: adding a way to emit an Action after v4.0 would be a Kernel major, and a Plugin host built to this mapping without one could host a Provider but never a Reactor.

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
| `ma_06_signature_types_are_documents` | a compile-time bound requiring the Plugin-boundary document types to be serialisable with a schema | compiles | MA-46, MA-6 |
| `ma_06_role_signatures_name_only_documents_and_handles` (`kernel_surface.rs`) | every role-trait signature in `src/`; then synthetic signatures with a slice, a closure, `impl Trait` and an unlisted type | the first passes, each listed document type is a document at compile time and has a registered schema; the rest are refused | MA-6, MA-46 |
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
| `ma_39_island_admission` | each of the five checks violated in turn, and an Island naming an Executor with no descriptor | rejected, naming the failed rule | MA-39, MA-40, MA-38 |
| `ma_39_a_cross_domain_link_inside_one_island_needs_a_registered_link` | two components in two memory domains, in one Island and in two; Links that join, do not join, lack the policy, or are cross-process | refused unless the selected Link joins the pair and implements the policy | MA-39, MA-28, SB-25 |
| `ma_28_link_registration_refuses_cross_process_in_v4` | a Link descriptor for an unregistered Module; one with `cross_process: true`; a second for one version | each refused at registration; two versions keep their own descriptors | MA-28 |
| `ma_19_executor_receives_the_descriptors_for_its_island` | a test Executor prepared with `PrepareContext.components` set | the typed descriptors are readable in `prepare` | MA-19 |
| `ma_16_role_trait_signatures_do_not_name_peer_roles` (`kernel_surface.rs`) | every role trait's signatures and associated types, and the fields of every type they reach | no peer role trait | MA-16 |
| `ma_16_the_gate_sees_a_peer_behind_a_context_field` (`kernel_surface.rs`) | a `&dyn Sink` two structs behind a signature's context type, and one synthetic source per listed route | caught | MA-16 |
| `ma_16_the_names_the_walk_reads_are_the_names_declared` (`kernel_surface.rs`) | `src/`; then synthetic sources with a `use … as` in a module and in a fn body, and an unlisted `macro_rules!` | the first passes; the rest are refused | MA-16 |
| `ma_22_cycle_rules` | a `stream.*` cycle; an Event cycle across Islands; an Event cycle inside one Island | rejected, accepted, rejected | MA-22 |
| `ma_41_an_absent_or_non_string_class_is_refused` | `ezsdr.time` present with `clas`, with `{}`, with `class: 3`; then the section absent | the first three refused, the fourth planned — an absent section is not a declaration to disagree with | MA-41 |
| `ma_41_execution_class_table` | every row of MA-41 including the rejection | the derived class, or a rejection | MA-41 |
| `ma_42_fidelity_is_the_weakest` | two Providers declaring `hardware_quirk` and `envelope` | `envelope`; a Hardware Run records `real` where declared | MA-42, RS-41 |
| `ma_37_descriptor_structural_validation` | a duplicate port name; an unregistered contract; an update class outside the set; a missing `impl.hash` | refused in each case | MA-37 |
| `ma_30_stepping_order_and_quiescence` | three stepped instances registered in reverse order | stepped in role and id order, not registration order; the loop quiesces | MA-30, MA-45 |
| `ma_30_stepping_livelock_cap` | two components that reschedule each other at one instant | `STEP_LIVELOCK` and an abort, not a hang | MA-30 |
| `ma_25_sink_role_is_read_from_the_binding` | the Sink role read from the bound Module's descriptor, then a `Block` feed into it | a Provider is not a Sink; the `Block` feed is refused | MA-25, SC-21, SB-22 |
| `ma_10_sub_resource_binding` | the double's two-level tree, with a resource bound to a `test.line` | bound to that node's `ResourceId` | MA-10, SB-34 |
| `ma_35_vocabulary_carries_its_own_content` | a `test` Vocabulary declaring a key, an event kind with a default, a verb and a check | all four are enforced by the Kernel and none is interpreted by it | MA-35, RS-27, RS-13a, SB-29 |
| `ma_04_kernel_surface_ban` | the Kernel source | no banned token outside a comment citing the ban | OV-23a |
| `ma_14a_reactor_emits_through_admit` | a test Executor submitting a `TxBurst` through `actions_out`, once inside and once outside the `test.limits` envelope | admitted and dispatched; then rejected before reaching the Provider | MA-14a, RS-16 |
| `ma_05a_a_module_keeps_its_handles_after_prepare` | a stepped Provider keeps `events` and `time` from `PrepareContext` | an event emitted in `step` reaches the Manifest and `now` equals the step instant | MA-5a, KA-1 |
| `ma_07_an_instance_with_two_fragments_is_prepared_twice_and_armed_once` | one Provider instance bound to two Spec resources | calls are `prepare, prepare, arm, start, stop, cleanup` | MA-7, KA-9 |
| `ma_41_ezsdr_time_is_a_closed_set` | valid `class` with `start_lead_ns` at 5 and 2^62; misspelled field and invalid values | valid sections plan; unknown field and invalid values are refused; absent section reads as 0 | MA-41, KA-8 |
| `kc_25_an_admitted_update_changes_the_configuration` | Session sets `test.gain` to 3 | effective configuration holds 3 and the Provider receives the Action | MA-24, UC-2, KC-25, KA-10 |

## 8. Vision coverage

| Vision | This spec |
|---|---|
| §4 the microkernel picture and the Module registry | §3, MA-31…MA-33 |
| §5's Module API line; the Kernel stays small | MA-1…MA-3, MA-35 (MA-4 withdrawn; OV-23a carries the ban) |
| §7 the three axes, the five roles, the communication rule | MA-1…MA-3 |
| §14 ExecutionClass and the fidelity vector | MA-41, MA-42; spec 04's RS-41, RS-42 |
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
