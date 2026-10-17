# Phase 1 spec 05 — Module API

| Field | Value |
|---|---|
| Status | Accepted 2026-09-23 (Gate C; Phase 1 Step 5). Normative for `ezsdr-kernel::module_api`. Its amendments are listed in the closing Changes table. |
| Scope | The three axes and the five role traits; the rules every role trait obeys; `ModuleDescriptor`, `VocabularyDescriptor` and the registry; `ComponentDescriptor`; the ExecutionIsland declaration and its admission; ExecutionClass and the fidelity vector; the test doubles; what is fixed now so that an out-of-process Plugin later needs no Kernel change. |
| Not in scope | Radio Model traits and keys (the Radio Model, spec 07); the Simulation Engine (spec 08) and MockRadio (spec 09); the Peripheral and Endpoint vocabularies; the native, WASM and GPU execution ABIs; the Plugin wire protocol. |
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
axis 3  deployment   in-process    a Rust trait, compiled in; the only deployment so far, so a
                                   descriptor declares none

One Module may hold several roles. The UHD Module is a Radio Provider, a GPIO Provider and an
Authority; the sim-engine Module is an Authority. A deployment is never a role: an out-of-process
Plugin will be one, added with its protocol and the descriptor field that names it (MA-46).
```

## 4. Types

```text
ModuleId          SB-1's ModuleId grammar (table SB-T0), e.g. ezsdr.radio.mock
Version           { major, minor, patch }                release-only; no pre-release tags (SB decision B7)
VersionReq        a caret requirement over a Version
Role              Provider | Executor | Sink | Link | Authority

ModuleDescriptor  { id: ModuleId, version: Version, kernel_api: Version, roles: [Role],
                    vocabularies: [{ id: Namespace, req: VersionReq }],
                    impl_hash: optional ContentHash }
VocabularyDescriptor { id: Namespace, version: Version, prefix: Namespace, keys: [KeyDecl],
                       event_kinds: [{ kind: EventKind, default: Reaction, severity: Severity }],
                       verbs: [{ verb: Ident, compiles_to: CompileRule }],
                       checks: [AdmissionCheck] }

ProviderInstance  { id: ResourceId, module: { id, version }, profile: optional { name, version },
                    tree: Resource, fidelity: Fidelity, driving: { stepped: bool },
                    arm_after: [ResourceId], min_command_lead: optional Duration (host.monotonic),
                    sections: Map<Namespace, Value> }
Resource          { id: ResourceId, kind: Namespace, shareable: bool (default false),
                    capabilities: Map<Key, CapabilityValue>, children: [Resource],
                    ports: [Port],                    a stream node's Ports, which a Spec may link to (SB-15)
                    memory_domains: [MemoryDomainId] }   absent means empty; non-empty on a node with ports (MA-10)
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
                      params: [{ key: Key, schema: Value, update_class: optional UpdateClass, default: Value }],
                        absent update_class: fixed for the Run (RS-52); each key once (MA-37)
                      timing: { budget: optional RelativeBudget },
                      requires: { executor_kind: Namespace | "any" },
                      impl: { kind: Namespace, id: string, hash: ContentHash } }
UpdateClass       cold | block_boundary | atomic_realtime | hardware_timed
IslandDecl        { id: IslandId, executor: Ident,
                    components: [{ component: Ident, memory_domain: MemoryDomainId }],
                    affinity: optional [u32], rt_policy: optional { sched, priority } }

ExecutionClass    Simulation | RealtimeEmulation | HardwareInLoop | Hardware
Fidelity          { timing: none | envelope | hardware_quirk | real,
                    continuity: none | envelope | hardware_quirk | real,
                    coercion: none | grid | real,
                    rf: none | impairment_model | real,
                    transport: none | model | real }
Requested         { resource: ResourceId, constraints: Map<Key, Constraint> }
                  what the matcher offers a Provider's coerce (SB-7); prepare sees the same input
Endpoint          StreamIn(DataLink) | StreamOut(DataLink), non-exhaustive
                  one attached end of a declared link (SC-19), handed over at prepare; an event
                  queue's end joins it with its handle when it is designed (MA-28)
PrepareContext    { run: RunId, source: EventSource, class: ExecutionClass,
                    time: shared TimeAuthority handle, clocks: shared ClockRegistry handle,
                    events: shared EventSink, a view of the Run's collector bound to `source` (MA-5),
                    actions: shared ActionReceiver,
                    actions_out: shared ActionSubmitter,
                    environment: shared read-only Map<Namespace, Value>,
                    inputs: shared read-only InputStore,
                    links: [{ component: Ident, port: Ident, endpoint: Endpoint }],
                    components: Map<Ident, ComponentDescriptor>, host_budget: RelativeBudget }
                  non-exhaustive, as are `Endpoint`'s `AttachedPort` and spec 06's `Assembly`: a field
                  joins them without breaking a Module; a Module's own tests build one with
                  `PrepareContext::testing` (feature `testing`)
ActionReceiver    recv() -> optional Dispatched
Dispatched        { id: ActionId, action: Action, node: optional ResourceId }: the id KC-24 gave the
                  Action at dispatch, the Action with its authored target, and the node KC-23 resolved
                  a `{resource}` target to, which a Provider matches on; non-exhaustive, built in a
                  Module's tests with `Dispatched::new` (feature `testing`)
ActionSubmitter   submits a proposed Action into admit() (RS-16), never to a Module directly
InputStore        get(ContentHash) -> shared bytes or none: the Run's inputs, read-only (RS-44a, KB-1)
StepOutcome       { progressed: bool }
ModuleError       { kind: Rejected | Unsupported | Timeout | DeviceLost | Internal,
                    message: string, detail: Value }
```

## 5. Normative rules

### The axes and the roles

- **MA-1** The three axes of §3 are orthogonal. A Module declares its roles in its `ModuleDescriptor` and may hold several; a deployment is never a role (Vision §7).
  *Test:* `rs_12_a_multi_role_module_is_bound_once_per_role` (a Module holding several roles is bound once per role) and `ma_32_registry_refusals` (a descriptor naming a deployment does not parse).
- **MA-2** There are exactly five roles and one trait each. There is no shared lifecycle supertrait: where two traits have methods with the same name it is by convention, and the coordinator dispatches per role. Link and Authority have no lifecycle at all.
  *Test:* the five traits by `ma_05_traits_are_object_safe_and_send`; the absence of a supertrait by `ma_06_role_signatures_name_only_documents_and_handles` (it refuses a supertrait other than `Send` or `Sync`).
- **MA-3** A Module communicates only through Kernel-defined Resources, Ports, Events, Actions, Capabilities and DataContracts. In source terms, a Module crate depends on the Kernel crate and on Vocabulary crates, never on another Module crate. Binding to a sub-resource of another Module's device is resolved by the Kernel (SB-36) and is not a dependency (Vision §7, §39).
  *Test:* `ma_03_no_module_crate_depends_on_another` (PO-8).
- **MA-4** *Withdrawn.* It restated `00-overview.md` OV-23a, which OV-4 forbids; the ban stands there and `kernel_surface` enforces it, and the `ma_04_kernel_surface_ban` test row cites OV-23a.

### Rules common to every role trait

- **MA-5** Role traits are synchronous, object-safe and `Send`. No method is generic, none returns `Self` or an opaque type, and no trait is `async`. `PrepareContext.source` is the source root the Kernel gives the instance (spec 06 KC-8): a Provider its root node, a Sink its output, an Executor its Island. Every event the instance emits carries it or, for a Provider, a node below it; no Module formats a source of its own, and a Module's id is never part of one. The rule is enforced, not trusted: `PrepareContext.events` is a view of the Run's collector bound to that root (spec 06 KC-8, which also says which Island an Executor's root is), and the view refuses any other source (spec 04 RS-32); `resolve` is therefore fallible, returning `Result<EventHandle, RunError>`.
  *Test:* spec 06's `kc_08_each_instance_is_given_its_source_root`, `kc_08_each_instance_s_event_view_is_rooted_through_the_coordinator` and `kc_08_an_executor_event_from_a_second_island_has_its_own_counter`, MockRadio's `mr_10_events_carry_the_source_the_context_gave`, the Sink's `sink_capture.rs` and the native Executor's `native_executor.rs`.
- **MA-5a** A Module may keep every handle its `PrepareContext` carries and use it from `prepare` through `cleanup`; the coordinator keeps each one valid until the Run is `CleanedUp`. A handle is shared (`Arc`), so a Module that keeps one keeps it alive; it must drop them in `cleanup` (MA-7). The `environment` is the BindingProfile's `environment` verbatim and read-only: a Module reads the sections its own Vocabularies define and no other (OV-21's rule, applied to Modules), and the Kernel still reads only SB-26's four. `inputs` is the Run's input store: the bytes of every input KC-9 verified and of every waveform KC-28 ingested, keyed by content hash and read-only. A Session's waveform is stored before the Action that names it is dispatched, so a Provider that transmits a `TxBurst`'s samples reads its waveform there.
  *Test:* `ma_05a_a_module_keeps_its_handles_after_prepare`: a stepped test Provider stores `events` and `time` at `prepare` and emits an event and reads `now` in `step`; the event reaches the Manifest.
- **MA-6** Every parameter and return type is one of three things: a document type with a schema, a Kernel handle (`PrepareContext`, the container that carries the others, `EventSink` — the root-bound view MA-5 describes —, `ActionReceiver`, `ActionSubmitter`, `InputStore`, `Endpoint`, the `DataLink` a Link creates and an `Endpoint` holds, a `TimeAuthority` handle), or `ModuleError` — or an `Option`, `Result`, `Vec`, `Box`, `Arc` or reference over those. Never a raw slice, a closure, an iterator or a generic parameter.
  *Test:* `ma_06_role_signatures_name_only_documents_and_handles`, which reads every role-trait signature in `src/` and refuses a name outside that list or a slice, `impl Trait` or bare `fn` type, and asserts at compile time that each listed document type is serialisable with a schema; it supplements `ma_06_signature_types_are_documents`, which carries MA-46.
- **MA-7** The coordinator calls lifecycle methods, one call at a time per instance, in the order `prepare → arm → start → (step)* → stop → (step)* → cleanup`. `step` follows `stop` only in an orderly cleanup of a Simulation Run, where cleanup step 3's drain steps every stepped instance until nothing is scheduled, so that a Provider delivers what its orderly `stop` publishes (MA-13) and an Executor or Sink receives it before its own `stop`; after an abort no `step` follows `stop`. `stop(reason)` is always attempted before `cleanup`, on an abort as well. `cleanup` is infallible and idempotent and is called for every instance that reached `prepare`, in reverse dependency order, after a failure at any phase (RS-6, RS-8). `prepare` is called once per **fragment**; `arm`, `start`, `step`, `stop` and `cleanup` once per **instance**. An instance's position in the arm and start order is the position of its first fragment in the plan; in a per-fragment cleanup step (RS-6) it is acted on at the first of its fragments the reverse pass reaches, and not again. A Provider may refuse a second fragment in `prepare` with `Rejected` when it serves one resource per instance, as MockRadio does (MR-7).
  *Test:* `ma_07_lifecycle_order_and_cleanup` (the single-instance order and idempotence), spec 06's `kc_13_arm_and_start_follow_instance_order_cleanup_reverses_it` and `ma_07_an_instance_with_two_fragments_is_prepared_twice_and_armed_once`.
- **MA-8** `prepare` and `arm` are bounded by `PrepareContext.host_budget`; a Module that cannot finish returns `ModuleError { kind: Timeout }`. Enforcing the budget and returning `Timeout` is the Module's duty. In a device-paced class (HardwareInLoop or Hardware, spec 06 KC-2) the coordinator enforces it as well (KC-12a).
  *Test:* `mr_07_prepare_cases` (MR-7) and `hd_09_prepare_cases` (HD-9), which supply a host budget and complete prepare but do not assert timeout enforcement; the coordinator's enforcement by `kg_06_a_prepare_that_hangs_fails_the_run_within_its_budget` and `kg_06_an_arm_that_hangs_fails_the_run_within_its_budget` (KC-12a); the Ceiling's in-process prepare on the caller's thread by `kg_06_the_simulation_class_prepares_on_the_callers_thread` (which asserts neither a budget nor a timeout).
  *Ceiling:* in the Simulation class an in-process Module that hangs hangs the transaction; there every call is in-process and instant, and a wall-clock timeout would make the outcome of a deterministic Run depend on the machine (PO-11).
- **MA-9** A fault crosses the boundary only as `ModuleError`. No panic crosses a trait boundary, and a foreign exception is translated to a status (Vision §35). `DeviceLost` is the kind that spec 04's Policy maps to `abort`.
  *Test:* the trait return type by `ma_06_role_signatures_name_only_documents_and_handles` (MA-6); spec 06's `kc_30_a_panicking_module_fails_the_run_not_the_process` and `kc_30_device_lost_is_the_kernel_event_and_aborts`; none named for foreign exception translation.
- **MA-9a** A Provider that is not stepped and finds on one of its own threads that its device is gone reports it by emitting `DEVICE_LOST` itself, through `emit_control`, with source its context's `source` (MA-5), severity `fatal`, time the current instant on the primary root and payload `{ "message": <text> }` — KC-30's event, whose pair KC-8 registers for every instance's source root. It then stops using the device; the Policy applies (KC-31, KC-46b), and its `stop` and `cleanup` must still succeed so that cleanup reaches its end (RS-11).
  *Producer obligation*; the Kernel's half by `kg_10_a_device_lost_from_a_provider_thread_aborts_the_run`.

### Provider

- **MA-10** `instance()` returns the `ProviderInstance`, exposing the composite resource tree with capabilities declared per node, so that the matcher can bind at any node and a sub-resource carries its own `ResourceId` (SB-33). A node that is a stream endpoint also declares its `ports`, so that a Spec's `graph.links` may name one and SC-3 has a contract to check against (SB-15, SC-1, SC-3). Each node declares the memory domains its ports produce or consume, as `memory_domains`, absent meaning empty; `validate()` refuses a node that has ports and declares none, on every node of each bound Provider tree, matched by a Spec resource or not (SB-T3), and MA-39 reads the matched node's list for a resource endpoint. A Provider that builds its tree from a shared builder takes the domain from its caller (spec 07 RM-2). Each node also declares whether more than one Spec resource may bind to it, as `shareable`, default `false` (SB-34). The Kernel enforces the flag and never decides it: which kinds are shareable is Radio Model knowledge, which OV-21 keeps out of the Kernel. `ProviderInstance.min_command_lead` is the least lead this instance needs between the coordinator **dispatching** a timed Action to it and that Action's instant — the time the instance takes to receive the Action included — as a `Duration` in `host.monotonic`; absent means zero. A stepped Provider receives an Action in the round that follows its dispatch, at the same instant (KC-21), so its **delivery allowance** is zero; a Provider that is not stepped receives it when its own thread next calls `recv()`, which KC-21a bounds, and states in its own spec the delivery allowance its declared lead includes. What remains, the lead less the allowance, is the lead the instance needs from receipt, which UC-6 and a Vocabulary's lateness rule (RM-14) use. It is the only envelope value the Kernel reads: RS-19's earliest instant and KC-19's plan-time lead check use it. A Vocabulary that defines a timing envelope defines how its own key relates to it (RM-6). `validate()` refuses one in another domain or negative (SB-22f). A Provider's `instance()` returns the same `id`, `tree`, `driving`, `arm_after` and `min_command_lead` for the whole Run; `fidelity` is final when `prepare` returns; after that only `sections` may change, as the Provider records what happened.
  *Test:* the default empty list by `ma_10_resource_memory_domains_default_to_empty` and the check on every node of every bound tree by `ma_10_memory_domains_are_validated_for_every_provider_tree_node`; a sub-resource's own `ResourceId` by `ma_10_sub_resource_binding`; the tree and Port declaration by `mr_02_the_tree_has_the_radio_model_shape`, the block buffer's host memory domain by `mr_13_ramp_values`, and the settled fidelity in the Manifest by `kb_02_the_manifest_records_the_fidelity_settled_in_prepare`. The delivery allowance has no Kernel test; it is carried by MockRadio's `mr_17_*` and `mr_18_*` (a zero allowance) and by spec 18's `ur_21_an_untimed_send_is_on_time_after_its_delivery` and `ur_24_an_untimed_retune_is_on_time_after_its_delivery`. None named for the values that are the same for the whole Run (`id`, `tree`, `driving`, `arm_after`, `min_command_lead`) or for the limit that after `prepare` only `sections` may change.
- **MA-11** `coerce(request)` is pure, deterministic and requires no hardware. It is a function of the declared profile (Vision §52). Two calls with the same request return identical reports. `coerce` is called at every `validate()`, not only when a key needs coercion, so it must be cheap; and it is where a Provider refuses a combination its envelope does not admit (SB-7).
  *Test:* `ma_11_coerce_is_pure`.
- **MA-12** `prepare(fragment, ctx)` returns a `PrepareReport` whose `coercions` equal what `coerce` returned for the same request (SB-44). `effective` may narrow a declared capability and must not widen one, and the coordinator re-runs constraint matching over `effective`, which is also Vision §52's re-check of applied values (SB-30). Both read each resource's **own** report (SB-41). The re-match skips exactly the keys SB-46 exempts. Both are `prepare`-stage checks, run inside `collect_prepare` (SB-41).
  *Test:* `ma_12_prepare_matches_coerce`, `ma_12_a_widening_effective_is_refused_at_prepare`, `ma_12_two_resources_naming_one_key_do_not_refuse_each_other`, `sb_44_a_provider_may_not_exempt_a_key_by_declaring_a_coercion` (a key the report alone declares is still re-matched) and `sb_46_an_accepted_coercion_survives_prepare` (a key the Kernel's preview recorded is not).
- **MA-13** `arm` reserves and synchronises and radiates nothing. `start(at)` begins at that instant or as soon as possible. `stop(mode)` under an orderly stop delivers each stream up to the instant it is handled, and under an abort up to the first sample not yet delivered (RM-16); the mode is what reaches each Provider (RS-9), while the `StopCause` stays on `Action::Abort` and in the Termination record. `cleanup` releases and restores baseline.
  *Test:* `mr_25_orderly_stop_ends_at_its_instant_abort_at_once` (MR-25); the mode reaching a Provider's `stop` through the coordinator by spec 06's `kc_14_an_arm_failure_is_failed_arm` (its provider's probe line `p:stop:Abort` after an arm failure); the `Abort` mode on every cleanup step by spec 04's `rs_06_cleanup_order_abort` (RS-9; every recorded mode is `Abort`, and no order is asserted there); the cleanup step order by spec 04's `rs_06_cleanup_order_orderly`.
- **MA-14** Actions reach a Provider only through `ctx.actions`, and only after Kernel admission (RS-16). A constraint that can only be checked at run time, such as the lead of a Reactor's burst, is enforced by the Provider, which emits the typed event; it never silently accepts. An event that answers one Action — a refusal at receipt, or a Sink's answer to a request — names it by `Dispatched.id` in its payload's `action` field, so a requester waits for its own answer and never counts others' (RM-22, HD-16).
  *Test:* `ma_14_actions_arrive_only_after_admission` (a `TestSubmitter` stand-in for `admit()` refuses an Action and nothing is delivered, the admission-gate half); an Action reaching a Provider through `ctx.actions` by spec 06's `kc_25_an_admitted_update_changes_the_configuration`; the run-time refusal by MockRadio's `mr_16_burst_refusals`; an answer naming the Action it answers in its payload's `action` field by the Sink's `hd_16_every_answer_names_its_action` and `hd_16_each_requester_gets_its_own_answer`.
- **MA-14a** A Module **emits** an Action through `ctx.actions_out`, which submits it to `admit()` (RS-16) rather than to another Module. For an Action submitted from `step` — where a round runs only while the Run is `Running` or in cleanup's drain, which freezes dispatch first (RS-6 step 1) — a refusal naming at least one violation, every one of which has the check `ezsdr.dispatch` or `ezsdr.run_state` (KC-24 steps 1 and 2), says that the Run is ending, not that the Action was wrong: the emitting Module must not turn it into an error of its own, which the coordinator would record as an abort of a Run that was stopping cleanly (KC-32). The Action simply does not happen, and the termination already records why. An `Abort` ends the Run (KC-32); it is neither admitted nor logged, and `submit` returns its real `ActionId` (spec 04 RS-15).
  *Test:* `nx_06_a_refusal_because_the_run_is_ending_is_not_a_failure`, carried by the native Executor (spec 14 NX-6), and, end to end, `ke_03_a_decision_after_the_stop_is_not_an_abort`.
- **MA-14b** An instance that the control thread does not step — a Provider whose `driving.stepped` is false, and in a device-paced class every Executor and Sink (KC-46) — has **finished** with every Action it took from its `ActionReceiver` when it next calls `recv()`. It must therefore carry out each Action it takes, or emit the event that refuses it (MA-14), before its next `recv()`, and having taken an Action it must call `recv()` again without waiting for another to be dispatched, until `recv()` returns `None`. The loop `while let Some(dispatched) = actions.recv() { handle(dispatched) }` does both. "Carrying out" an Action means doing what it asks at receipt — booking it, or applying it — not waiting for its effective instant: a Provider books a timed or future change and returns to its queue.
  *Producer obligation*; the Kernel half is KC-21a's.
- **MA-15** `step(until)` has a default no-op body on `Provider`. A Provider whose `driving.stepped` is true overrides it under MA-20's contract; a hardware Provider never receives a `step`.
  *Test:* `ma_15_provider_step_default_is_idle`; none named for the coordinator never stepping a Provider whose `driving.stepped` is false (spec 06's `kg_01_a_stepped_provider_is_refused_in_a_device_paced_class`, KC-2a, pins only that a stepped Provider is refused in a device-paced class).
- **MA-16** No role trait reaches another role trait, so the Kernel role API gives a Module no reference to a peer Module instance. A walk over names cannot see through a rename, so `src/` contains no `use … as` and no `macro_rules!` other than the ones the gate lists.
  *Test:* `ma_16_role_trait_signatures_do_not_name_peer_roles` (a `syn` walk, which follows from each role trait the routes its documentation lists and fails on reaching another role trait), `ma_16_the_gate_sees_a_peer_behind_a_context_field` (one red route per case) and `ma_16_the_names_the_walk_reads_are_the_names_declared`.
- **MA-16a** A route the walk does not follow — a conversion defined on another type, a free function, a `static`, a blanket impl — is kept out by the allow-list review of `00-overview.md` OV-23 at each gate and at the v4.0 freeze. A route found later is recorded here, and the walk is extended only when such a route appears in `src/`, never for a demonstration.
  *Producer obligation* of the gate reviews at each gate and at the v4.0 freeze; none named.
- **MA-17** An instance-level capability such as a Peripheral's timing class is knowable at instantiation, from the selector and profile, and appears in `instance()`. `PrepareReport.effective` may narrow it (MA-12).
  *Producer obligation* of the first Provider with a Peripheral; MockRadio has none.
  *Test:* `prepare` may only narrow them by `ma_12_narrowing_triggers_readmission`; matching's reading of the bound instance's capabilities (SB-37) by `sb_37_matching_follows_binding`.

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

- **MA-18** `descriptor()` returns the `ExecutorDescriptor`. Admission reads `kind`, `memory_domains` (non-empty) and `impl_kinds`, and compares `module` with the binding (SB-22f); everything else is opaque.
  *Test:* `sb_22f_the_runtime_supplies_each_slot_at_its_version` (an Executor that declares no memory domain, and a descriptor naming another version, are refused).
- **MA-19** `PrepareContext.components` is the typed channel by which an Executor receives its Island's `ComponentDescriptor`s, keyed by component name; `prepare(island, ctx)` receives them typed, never as JSON.
  *Test:* `ma_19_executor_receives_the_descriptors_for_its_island`, which proves the field reaches `prepare` typed and readable; populating it with exactly the Island's descriptors is MA-19a.
- **MA-19a** The coordinator populates `PrepareContext.components` with only the descriptors belonging to the Island being prepared.
  *Test:* spec 06's `kc_11_an_island_gets_exactly_its_components` (KC-11).
- **MA-19b** The Executor loads each component by its `impl` identity and returns a `PrepareReport` for the Island.
  *Producer obligation*, carried by `ezsdr.exec.native` (spec 14 NX-3, `nx_03_a_component_is_loaded_by_its_impl_identity`).
- **MA-20** `step(until)` must consume every input at or before `until`, emit every output and event at or before `until`, emit nothing after it, never block on input, never call `wait_until` (TM-16d), and report `progressed` true exactly when it consumed an input or produced an output. It is required on `Executor`. In a device-paced class `until` is the instant the data thread's pass took, and an event a step stamps with the current instant may lie after it, because the device's clock does not stop for a pass, as it does not for a callback (TM-16c).
  *Producer obligation* of every Executor.
  *Test:* the `progressed` contract partly by spec 14 NX-5's `nx_05_components_step_in_id_order_and_their_actions_follow_each_step` (an Action a component pushed counts as progress, and a component's own progress is reported); none named for the time bound, the input and output contract, emitting nothing after `until`, and never blocking.
- **MA-21** The execution ABI is the Executor's. The Kernel defines no `work` or `process` signature and never inspects `impl` beyond its identity.
  *Test:* none named.
- **MA-22** Plan admission rejects a cycle formed by `stream.*` links. A cycle is legal only when every edge that closes it is an Event or Action edge crossing an Island boundary. Adaptive feedback inside a component is state, not structure (Vision §19).
  *Test:* `ma_22_cycle_rules`; none named for an Action edge (no test builds an `EdgeKind::Action` edge, so only the Event half of the closing-edge condition is pinned).
- **MA-23** Where a component maps input blocks to output blocks one to one, the Executor propagates block flags unchanged by default (SC-17). A mapping that is not one to one is the Processor author's documented convention, not a Kernel type.
  *Producer obligation* of every Executor; none named.
- **MA-24** An `UpdateParameter` reaches an Executor through `ctx.actions`, and the Executor applies it under the parameter's declared update class or, if it applies no Action, refuses it. An Action with an undeclared class was rejected at admission (RS-52, spec 06 KC-24 step 5) and never arrives; an admitted one is applied under the parameter's declared update class as UC-2…UC-6 define it by an Executor that applies Actions. An Executor that applies none refuses one that reaches it, failing its `step` (MA-14: never silently accepted), and its spec says so; `ezsdr.exec.native` 1.0.0 is one (spec 14 NX-7).
  *Test:* the admission refusal is RS-52's; the Executor's refusal by `nx_07_an_action_addressed_to_the_executor_fails_the_step` (spec 14 NX-7); the update classes by their own rules — KC-25 by `kc_25_an_admitted_update_changes_the_configuration`, UC-6 and UC-3 by MockRadio's `mr_18_hardware_timed_updates` and `mr_18_a_cold_rate_change_starts_a_new_sample_clock`, UC-4 by the capture Sink's `hd_10_session_requests_are_sequential`; none named for an Executor that applies an Action under the declared class.

### Update classes

- **UC-1** An `UpdateParameter` carries its key's declared class (RS-52), and the coordinator delivers it unchanged to its target (KC-25). The **effective instant** of an update is its `at` converted to the target's domain when present, and otherwise the first instant its class permits. The Kernel checks only that the class was declared (RS-17); UC-2…UC-6 bind the target, except an Executor that applies no Action, which refuses the update (MA-24).
  *Test:* spec 06's `kc_25_an_admitted_update_changes_the_configuration` (KC-25).
- **UC-2** An update never takes effect before its effective instant, and a target never applies two updates of one key out of their effective-instant order; two with one effective instant apply in delivery order.
  *Producer obligation* of every Provider and Sink, and of every Executor that applies Actions; an Executor that applies none refuses the update instead (MA-24). `kc_25_an_admitted_update_changes_the_configuration` covers an admitted update; MockRadio's tests are MR-18's.
- **UC-3** `cold`: at the effective instant the target stops the function the key belongs to, applies the value and, if the function was running, starts it again; a function that is not running only takes the value. A stream so restarted ends its SampleClock and continues on a new one; between the two it delivers nothing and reports no gap (SC-12). Where the end and the new origin fall is the Vocabulary's (radio: RM-16, RM-25). Absent `at`, the effective instant is the current instant. A `cold` update changes a value, never the graph's structure (RS-4).

  *Producer obligation; tested by each Vocabulary's Providers.*
- **UC-4** `block_boundary`: every block — or, for a consumer that slices blocks, every sample — whose first sample instant is at or after the effective instant sees the new value; no block mixes old and new. Absent `at`, the effective instant is the current instant.
  *Producer obligation.*
- **UC-5** `atomic_realtime`: the value applies to every sample processed after the Action is received and not before its effective instant, with no state in which part of the value is applied.
  *Producer obligation.*
- **UC-6** `hardware_timed`: the device applies the value at exactly the effective instant; a device that applies commands only on a sample grid advances it to the next sample instant (MockRadio applies at the instant itself, MR-18). Absent `at`, the effective instant is the current instant at receipt plus the target's lead from receipt (`min_command_lead` less its delivery allowance, MA-10). An `at` closer than that lead is late: the target applies the value at the current instant plus that lead and emits its Vocabulary's late-command event. A pending `hardware_timed` update occupies one slot of the device's command queue until it applies, and a full queue refuses it with the Vocabulary's queue event.
  *Producer obligation*; MockRadio's tests are MR-18's.

### Sink, Link and Authority

- **MA-25** `SinkDescriptor.memory_domains` declares the domains a Sink reads from, at least one — an empty list is refused at validate — and MA-39 checks a feed against it. `Sink` has `descriptor()`, `prepare(fragment, ctx)`, `arm`, `start`, `step`, `stop(reason) -> [ArtifactRef]` and `cleanup`. Every Sink is stepped in the Simulation class and runs on a thread in the other three (MA-30), so the decision is the class's and no descriptor flag carries it; a Sink that leaves `step` at the default and therefore never drains is a defect the stepping test catches. Every link that feeds a Sink is drop-class, which is SC-21's predicate; which Module fills an output's Sink slot, and that it holds the role, is SB-22's (SB-22b, SB-22c, SB-22e). A Sink is bound, never placed — it is a role, not a component an Executor loads — so it carries no `ComponentDescriptor` and appears in no Island, and its own fragment is the one `plan()` emits per bound output.
  *Test:* `ma_25_sink_role_is_read_from_the_binding` (the Sink role is read from the bound Module, a Provider is not a Sink, and a `Block` feed is refused by `check_sink_link`, SC-21); the empty-`memory_domains` refusal by `ma_39_plan_admits_a_real_graph_and_refuses_a_misplaced_component` and spec 03's `sb_22f_the_runtime_supplies_each_slot_at_its_version` (its Sink case).
- **MA-26** `stop` returns its `ArtifactRef`s even on an abort, with `partial` set (RS-44). A Sink whose `stop` fails is recorded as a `CleanupFailure` naming its fragment (RS-6, RS-8), and the Manifest lists no `ArtifactRef` for it: that is what "unknown" means, since the Kernel records no artifact it was not handed.
  *Test:* `ma_26_sink_returns_partial_on_abort` (the `partial` flag) and spec 06's `kc_41_a_failing_sink_stop_is_a_cleanup_failure` (KC-41).
- **MA-27** `Link` has `descriptor()` and `create(&DataLinkDecl) -> DataLink`, returning an implementation of spec 02's link interface, whose two ends can be attached to producing and consuming ports as `Endpoint`s. An attached end names the component and the port it is bound to; for a Provider the component is the Spec resource name and for a Sink its output id.
  *Test:* `ma_05_traits_are_object_safe_and_send`; the attached end's component naming by spec 06's `kc_10_both_ends_are_attached` (the attached ends are logged as `radio.rx:out` and `rec.in:in`).
- **MA-27a** The coordinator creates selected Links before preparing Providers, Executors and Sinks, refusing an instance whose `descriptor()` differs from the descriptor registered for its `ModuleRef` (MA-28), attaches their two ends, and drops them at cleanup; Links have no lifecycle of their own.
  *Test:* spec 06's `kc_10_link_descriptor_must_equal_the_registered_one` and `kc_10_both_ends_are_attached` (KC-10).
- **MA-28** A Link's descriptor carries the Link Module's exact `{id, version}` as its `module` and is registered under it, one per version, so it cannot be filed under another version; `LinkPlacement` selects that same `ModuleRef`. The descriptor declares supported policies and memory-domain pairs in `connects`; admission checks the selected descriptor (MA-39) and never plans a transfer (SB-40). `cross_process: true` is rejected in v4.0, at registration and again at admission. Event and Action queues between Islands are Kernel handles, not Link products.
  *Test:* `ma_28_link_registration_refuses_cross_process_in_v4` and `ma_39_a_cross_domain_link_inside_one_island_needs_a_registered_link`.
- **MA-28a** A Link implements its declared policy exactly (SC-19, SC-20).
  *Test:* `hd_05_policies` (HD-5).
- **MA-29** There is one Authority instance per Run, named by the BindingProfile (SB-24), and its `AuthorityDescriptor` names its Module's exact `ModuleRef`, which admission compares with the binding (SB-22f). It provides a `TimeAuthority` handle (TM-16a) and declares an `AuthorityDescriptor` whose `governs` is the set TM-16a requires: a primary root, that root's derived domains, `host.monotonic`, and for a simulation Authority every root it simulates. Its `pacing` is cross-checked against the derived ExecutionClass by MA-41. `next_wakeup` **advances** the Authority's governed clocks to the earliest instant at which a scheduled callback is due, fires the callbacks due at that instant in TM-16c order — including ones they schedule at that instant, up to TM-17b's cap per call — and returns the instant; with nothing scheduled it returns `None` and moves nothing. A step-driven Module that needs to run at an instant schedules a callback there, which may do nothing. A `WallPaced` or `Device` Authority's `cancel` wakes a `next_wakeup` that is waiting, which then re-evaluates: it goes on waiting for the earliest instant still scheduled, or returns `None` when none is. It runs its callbacks without holding a lock that `schedule` or `cancel` takes, so a callback may schedule and another thread may schedule while one runs. An Authority whose root is a timekeeper's measures its primary root's relations and returns them from `relations()`, a provided method whose default returns none: the relation to `host.monotonic` and the relation to `utc` (TM-18, KC-45). The Kernel has no paced Authority of its own.
  *Test:* the descriptor checks by spec 03's SB-22e, SB-22f and SB-24 tests; `kc_20_virtual_time_advances_only_through_next_wakeup` (KC-3); the `cancel` sentence by spec 18's `ur_07_cancel_wakes_a_waiting_next_wakeup` and the `WallAuthority` test double, which follows it (`kg_03_a_cancelled_horizon_wakes_the_control_loop`); `relations()` by the `kg_11_*` tests (KC-45).

  ```rust
  pub trait Authority: Send + Sync {
      // … the three existing methods …
      /// TM-18: the measured relations of this Authority's primary root — to `host.monotonic`
      /// and to `utc` — or none when the root is virtual (a Simulation Authority).
      fn relations(&self) -> Vec<ClockRelation> { Vec::new() }
  }
  ```
- **MA-30** The coordinator owns the stepping loop, on one logical thread:

  ```text
  until = authority.next_wakeup()        None ends a Spec Run of the Simulation class (KC-33); a Session waits for its client
  repeat: step(until) over every stepped instance not failed at this until, in a fixed order
          role rank Provider < Executor < Sink, then instance id
  until no instance reports progressed; a cap of 1 000 iterations raises STEP_LIVELOCK and aborts
  ```

  | class | stepped Providers | Executors | Sinks |
  |---|---|---|---|
  | Simulation | step | step | step |
  | RealtimeEmulation | step, wall-paced inside `next_wakeup` | threads | threads |
  | HardwareInLoop, Hardware | none | the data thread (KC-46) | the data thread (KC-46) |

  In the device-paced classes the coordinator's data thread steps every Executor and Sink in the fixed order above, one pass at a time and without the cap (KC-46); an Island's declared affinity and real-time policy are spec 06 KC-2a's.

  The order is fixed by rule and not by registration, so determinism does not depend on the order in which a runtime happened to assemble its Modules (Vision §58 #3).

  A `step` that returns an error — a panic included, which the coordinator contains as one (KC-30) — marks its instance failed for the rest of the round: it is not stepped again at that `until` (a later instant may step it again, as a cleanup drain does), and the round goes on over the other instances until none reports `progressed`. `step_until_quiescent` then returns the **first** error in stepping order (pass, then role rank, then instance id); a round that reaches the cap after a failure returns that failure too, and raises no `STEP_LIVELOCK`, because the failure is what ends the Run. The coordinator handles every failure of the round on its own terms, in stepping order (KC-30) — `DEVICE_LOST` for each `DeviceLost`, a request for a failed Run for any other, the first end request fixing the end (KC-32) — so a second lost device is reported, and a failure after a lost device still fails the Run. The instances a failure did not touch therefore see only what the failed instance published before it failed, wherever it sorts, and no instance's output depends on a failed peer's name.
  *Test:* `ma_30_a_step_error_finishes_the_round`, `ma_30_a_second_failure_does_not_replace_the_first`, `ma_30_a_failure_beats_the_step_livelock_cap`, the coordinator's `kd_01_a_device_lost_is_not_reported_as_a_step_livelock` and `kd_01_every_lost_device_of_a_round_is_reported_and_the_first_failure_decides`, and, end to end, `kd_01_a_faulted_round_does_not_depend_on_fragment_names`.

  *Ceiling:* a zero-latency Event or Action cycle at one instant hits the cap, and the coordinator also counts consecutive `next_wakeup` results at one instant, more than `STEP_ROUND_CAP` of which is `STEP_LIVELOCK` (KC-22). Delivery latency on inter-Island Action edges, which would let such a cycle advance in time, is not added: the Reactor makes no zero-latency cycle reachable, because its Actions go to a radio, which answers with a later block (MR-14) or transmits at a later sample, and the native Executor applies no Action (spec 14 NX-7), so no Action edge joins two Reactors. It is re-marked to the first Executor that applies Actions.

### Descriptors and the registry

- **MA-31** Every Module ships a `ModuleDescriptor`, and its `roles` must match the factories it registers.
  *Test:* `ma_32_registry_refusals`.
- **MA-32** Registration fails when: `kernel_api.major` differs from the Kernel's; a declared Vocabulary is absent or incompatible; a role has no factory or a factory no role; or `(id, version)` is already registered. A `VocabularyDescriptor` whose declarations MA-34 or SB-2a refuses is refused. A descriptor declares no deployment, so one naming any does not parse. Registration is explicit in the runtime's assembly code; there is no link-time registration crate, whose order is opaque and whose failures are silent.
  *Test:* `ma_32_registry_refusals`; the `VocabularyDescriptor` refusals by `ma_34_registration_refuses_a_declaration_outside_its_prefix`, `sb_02a_registration_refuses_a_numbered_segment_out_of_range_or_outside_the_path` and `sb_02a_registration_refuses_two_declarations_that_cover_one_key`, their admitted side by `sb_02a_registration_accepts_declarations_that_cover_no_common_key`.
- **MA-33** Compatibility is the caret rule: a requirement `^a.b.c` is satisfied by a registered `x.y.z` when `a == x` and, for `a > 0`, `(y, z) >= (b, c)`, or for `a == 0`, `y == b` and `z >= c`. The Kernel compares ids and versions and never interprets a key.
  *Test:* `ma_33_caret_table`.
- **MA-34** Every capability and constraint key a document uses is `<vocabulary prefix><path>` or `ext.<module-id>.<path>`; a key a Vocabulary declares is `<vocabulary prefix><path>` alone, and registration refuses a `KeyDecl` whose key is not under the declaring Vocabulary's own `prefix`, naming the key and the prefix. The Kernel refuses a key whose prefix belongs to no Vocabulary the declaring document declares, naming the prefix, and validates the value's shape against the `KeyDecl`, at SB-2's five key positions; an `ext.` key's owner is a registered Module. It never interprets the meaning (SB-2). A **component's** parameter key is owned by the component that declares it in its `params` — SB-2's one exception — and its update class is that `ParamDecl`'s alone (RS-52).
  *Test:* that the registry holds the declaration (`Some`) or none (`None`) by `ma_34_key_prefix_refused` (SB-2); a declaration outside its Vocabulary's prefix by `ma_34_registration_refuses_a_declaration_outside_its_prefix`; the refusal by `sb_02_unknown_key_prefix_refused`; the diagnostic naming the offending key, and so its prefix, by `sb_39_plan_reruns_the_structural_checks_and_refuses_a_stale_admission` and `sb_16_a_schedule_entry_may_target_a_component`.
- **MA-35** A `VocabularyDescriptor` carries what the Kernel is obliged to enforce on that Vocabulary's behalf but never interprets: its `KeyDecl`s, each with the shape, the coercion default, the optional update class and the optional numbered segment of one key (SB-2, SB-2a, RS-52), its event kinds with their default reactions and severities (RS-27, RS-28), its Session verbs and how each compiles (RS-13a), and its admission checks (SB-29).
  *Test:* `ma_35_vocabulary_carries_its_own_content`; a numbered declaration in the descriptor by `sb_02a_a_numbered_declaration_declares_the_indexed_keys` and `sb_02a_a_declaration_without_index_is_an_exact_key`.
- **MA-36** A `ComponentDescriptor` has the shape of §4. `requires` states requirements only; the placement lives in the BindingProfile (SB-25).
  *Test:* none named.
- **MA-37** Validation of a descriptor is structural: unique port names, registered contract ids, and each parameter key declared once within one component — two `ParamDecl`s with one key in one component are refused, while two components may declare the same key. A budget is positive by `RelativeBudget`'s own rule (TM-15). `validate()` runs it over every component of `graph.components` before anything reads a port or a `ParamDecl`. Update classes from the closed set and a present `impl.hash` are enforced one layer out, at the JSON boundary by the schema. A descriptor never carries an `AbsoluteDeadline` and a `TxBurst` never carries a `RelativeBudget`; they are distinct types (TM-15) and distinct schema definitions.
  *Test:* a duplicate port name, an unregistered contract and a non-positive budget at the document boundary by `ma_37_descriptor_structural_validation`; the parameter-key half by `ma_37_a_component_declares_each_parameter_key_once`; none named for the update class outside the closed set and a missing `impl.hash`, which the JSON boundary refuses.

### Islands, class and fidelity

- **MA-38** An `IslandDecl` names an Executor instance, the components placed on it, each with its memory domain, and its optional affinity and real-time policy. The `executor` name is a binding that fills an Executor slot (SB-22b, SB-22e); the runtime supplies only that instance's `ExecutorDescriptor` (SB-22f). Naming the instance without naming the Module in a document would leave the plan's `fragments[].instance` and the Manifest's `modules` resting on assembly-time input that nothing records. Where it lives and the requirement that every component be placed exactly once are SB-13 and SB-25.
  *Test:* `ma_39_island_admission`.
- **MA-39** Island admission checks: every component placed exactly once; `requires.executor_kind` is `any` or the Executor's kind, `impl.kind` is among its `impl_kinds`, and the memory domain its Island states for it is among its `memory_domains`; each data link's selected `LinkPlacement` descriptor — a graph link's or an output feed's (SB-25) — supports its declared policy; every data link, graph link or output feed, in one Island or in two, meets one reachability rule: for its producer's domain set `P` and its consumer's domain set `C`, every `p ∈ P` has a `c ∈ C` with `p = c` or with the selected Link's `connects` holding `(p, c)` or `(c, p)`. A component's set is the one domain its Island states for it, a Sink's the domains its bound `SinkDescriptor.memory_domains` declares, and a resource endpoint's the `memory_domains` of the node the matcher actually bound (MA-10); MA-22's cycle rule; and an Island with an `rt_policy` requires a declared budget on every component. With an `rt_policy`, only the presence of each component's declared budget is checked; the arithmetic feasibility check — that the sum of the budgets fits the block period — needs declared port rates and waits for the first Processor that declares them (§10).
  *Test:* `ma_39_island_admission`; the reachability rule by `ma_39_resource_endpoint_domain_sets_use_one_rule`, `ma_39_resource_domains_follow_bound_nodes_and_cover_resource_endpoints` and `ma_39_sink_feed_rejects_a_resource_domain_set_only_partly_reached`.
- **MA-40** Admission rejects; it never creates, merges or moves an Island (Vision §20, §63). A rejection names the rule it failed.
  *Test:* `ma_39_island_admission`.
- **MA-41** The ExecutionClass is derived from the environment and cross-checked against the Authority's pacing, and it is what spec 03's plan records (SB-39). Where the environment's `ezsdr.time` section is **present**, its `class` must be one of the four names and the derived class is compared with it; an absent, non-string or unrecognised `class` is refused rather than ignored. A mismatch is refused, and SB-26 lists that section as one the Kernel reads:

  | Authority | pacing | RF path | class |
  |---|---|---|---|
  | Simulation Engine | free-running | simulated | Simulation |
  | Simulation Engine | wall-paced | simulated | RealtimeEmulation |
  | device timekeeper | — | cabled, or partly simulated | HardwareInLoop |
  | device timekeeper | — | over the air | Hardware |
  | Simulation Engine | any | over the air, or cabled | rejected |

  The section's fields are `class` and `start_lead_ns` (KC-15) and no other; one it does not name is refused.
  *Test:* `ma_41_execution_class_table`, `ma_41_an_absent_or_non_string_class_is_refused` and `ma_41_ezsdr_time_is_a_closed_set`; a declared `class` compared with the derivation by `ma_41_a_declared_time_class_that_disagrees_is_refused`.
- **MA-42** The value sets of `Fidelity` add `real` to those of Vision §14. A Run's value per aspect is the weakest over its bound Providers, in that aspect's own order: `none < envelope < hardware_quirk < real` for timing and continuity, `none < grid < real` for coercion, `none < impairment_model < real` for RF, and `none < model < real` for transport, so a Hardware Run with a best-effort MockPeripheral records `timing: envelope`. An `ideal` Mock profile declares `none` (Vision §13).
  *Test:* `ma_42_fidelity_is_the_weakest`; the `ideal` Mock profile's all-`none` declaration by MockRadio's `mr_03_profile_values_reach_the_capabilities_and_the_envelope_section`.
- **MA-43** *Withdrawn.* It restated spec 04's RS-42 verbatim, citation included.

### Test doubles and the Plugin boundary

- **MA-44** The test-double Provider lives in `tests/support/` and is never shipped. It declares a `test` Vocabulary at version 1.0.0 with the keys `test.count`, `test.grid` (coercible, snapping to an injectable grid, defaulting to `reject`), `test.flag`, `test.gain`, `test.tx_clock`, `test.list` and the numbered `test.item` (SB-2a); a two-level tree of a device with two `test.line` sub-resources; `fidelity` all `none`; `driving.stepped` false; a recorded call log; an injectable failure phase; the `test.custom` event **kind**, declared by the Vocabulary; and a drain of its action queue. The double generates no Events. It uses no radio word, which is how the matcher is proven generic (OV-21). It has no time model, no blocks and no envelope: that boundary is exactly where MockRadio begins.
  *Test:* `ma_44_the_run_doubles_record_their_calls`; the two-level tree by `ma_10_sub_resource_binding`, `driving.stepped` and the default `step` by `ma_15_provider_step_default_is_idle`, a `test` Vocabulary key and the `test.custom` kind by `ma_35_vocabulary_carries_its_own_content`, the key set by `sb_08_the_test_vocabulary_names_no_radio_word`, and the call log and injectable failure phase by `ma_07_lifecycle_order_and_cleanup`; none named for its `fidelity` vector, its action-queue drain, or its generating no Events.
- **MA-45** A recording `TestExecutor` of about twenty lines also ships with the tests and performs no processing.
  *Test:* none named for its `step`; no test steps the double.
- **MA-46** What is fixed now so that a Plugin protocol later needs no Kernel change: the schemas of `ModuleDescriptor`, `VocabularyDescriptor`, `ProviderInstance` and `Resource`, the four other role descriptors, `ComponentDescriptor`, `IslandDecl`, `Fragment`, `Requested`, `PrepareReport`, `Dispatched`, `CoerceReport`, `ModuleError`, `StopMode`, `StepOutcome`, `ArtifactRef`, `ExecutionClass`, `Fidelity` and `ClockRelation` — the last since a role-trait signature carries it (`Authority::relations`, MA-29; `schemas/clock_relation.v1.json`) — which includes every document type a role-trait signature carries (MA-6) — together with spec 04's `Event` and Action schemas (`StopCause` among them, on `Abort`); and rule MA-6. The Plugin deployment itself is not reserved: it is added with its protocol. A Plugin host is then one Module implementing a role trait by proxy, with the handles becoming streams: an `EventSink` an event stream, an `ActionReceiver` an inbound action stream, an **`ActionSubmitter` an outbound request-and-response stream** carrying the admission result back, an `Endpoint` a shared-memory Link, the `TimeAuthority` a request and response, the `ClockRegistry` a request and response, the `InputStore` a request and response, and the `environment` a document sent once at `prepare`. The submitter has to be in this list.
  *Test:* `ma_06_signature_types_are_documents` and `ma_06_role_signatures_name_only_documents_and_handles` (each document a role signature carries has a schema); the committed schemas of `Resource`, `ModuleDescriptor` and `VocabularyDescriptor` by `ov_22_schema_freeze` (every registered document's committed file must match its rendered schema, and a committed file no registration generates is refused), `Resource`'s presence among the registered schemas also by `ov_10_every_document_has_a_schema`; none named for a compile-time document bound (`Serialize`, `DeserializeOwned`, `JsonSchema`) naming `ModuleDescriptor` and `VocabularyDescriptor` (their committed schemas are frozen by `ov_22_schema_freeze`, but no bound test names them).

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

## 7. Tests

| test | input | expected | rules |
|---|---|---|---|
| `ma_05_traits_are_object_safe_and_send` | `Box<dyn Provider + Send>` and the other four, at compile time | compiles | MA-5 |
| `ma_06_signature_types_are_documents` | a compile-time bound on the document types this test binds | compiles | MA-46, MA-6 |
| `ma_06_role_signatures_name_only_documents_and_handles` (`kernel_surface.rs`) | every role-trait signature in `src/`; then synthetic signatures with a slice, a closure, `impl Trait` and an unlisted type | the first passes, each listed document type is a document at compile time and has a registered schema; the rest are refused | MA-6, MA-46 |
| `ma_32_registry_refusals` | a missing Vocabulary; an incompatible one; a `kernel_api` major mismatch; a role with no factory; a duplicate `(id, version)`; a descriptor naming a `deployment` | refused in each case, naming the cause; a descriptor naming a deployment does not parse | MA-31, MA-32 |
| `ma_33_caret_table` | requirements and registrations either side of 1.0 | the table of MA-33 exactly | MA-33 |
| `ma_34_key_prefix_refused` | the declared key `test.count`, then a key whose prefix belongs to an undeclared Vocabulary | the registry holds the first declaration (`Some`) and answers `None` for the second; the refusal itself is `sb_02_unknown_key_prefix_refused` | MA-34, SB-2 |
| `ma_11_coerce_is_pure` | the same request twice, with no hardware present | identical reports | MA-11 |
| `ma_12_prepare_matches_coerce` | a coercing request through `coerce` then `prepare` | identical coercions | MA-12, SB-44 |
| `ma_12_narrowing_triggers_readmission` | an `effective` that narrows below the Spec's constraint | re-admission fails; a widening `effective` is refused | MA-12, MA-17 |
| `ma_07_lifecycle_order_and_cleanup` | an injected failure at each of prepare, arm and start | the call log shows `stop` before `cleanup`, `cleanup` last and `cleanup` idempotent | MA-7, RS-6 |
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
| `ma_37_descriptor_structural_validation` | a duplicate port name; an unregistered contract; a zero budget in a document | refused in each case; the update class and a missing `impl.hash` are refused only at the JSON boundary, where no test names them | MA-37 |
| `ma_30_stepping_order_and_quiescence` | three stepped instances registered in reverse order | stepped in role and id order, not registration order; the loop quiesces | MA-30 |
| `ma_30_stepping_livelock_cap` | two components that reschedule each other at one instant | `STEP_LIVELOCK` and an abort, not a hang | MA-30 |
| `ma_25_sink_role_is_read_from_the_binding` | the Sink role read from the bound Module's descriptor, then a `Block` feed into it | a Provider is not a Sink; the `Block` feed is refused | MA-25, SC-21, SB-22 |
| `ma_10_sub_resource_binding` | the double's two-level tree, with a resource bound to a `test.line` | bound to that node's `ResourceId` | MA-10, SB-34 |
| `ma_35_vocabulary_carries_its_own_content` | a `test` Vocabulary declaring a key, an event kind with a default, a verb and a check | the descriptor carries all four (`v.keys`, `v.event_kinds`, `reg.verb`, `v.checks`) and the registry answers for each; a kind outside its owner's namespace is refused | MA-35, RS-27, RS-13a, SB-29 |
| `ma_04_kernel_surface_ban` | the Kernel source | no banned token outside a comment citing the ban | OV-23a |
| `ma_14a_reactor_emits_through_admit` | a `TestSubmitter` double standing in for `admit()`, called directly with a `TxBurst` once inside and once outside the `test.limits` envelope | admitted into the double's `delivered` list; then refused, naming `test.limits`, with nothing further delivered | MA-14a, RS-16 |
| `ma_05a_a_module_keeps_its_handles_after_prepare` | a stepped Provider keeps `events` and `time` from `PrepareContext` | an event emitted in `step` reaches the Manifest and `now` equals the step instant | MA-5a, KA-1 |
| `ma_07_an_instance_with_two_fragments_is_prepared_twice_and_armed_once` | one Provider instance bound to two Spec resources | calls are `prepare, prepare, arm, start, stop, cleanup` | MA-7, KA-9 |
| `ma_41_ezsdr_time_is_a_closed_set` | valid `class` with `start_lead_ns` at 5 and 2^62; misspelled field and invalid values | valid sections plan; unknown field and invalid values are refused; absent section reads as 0 | MA-41, KA-8 |
| `ma_41_a_declared_time_class_that_disagrees_is_refused` | a declared `class` that disagrees with the derivation; then one that agrees; then none declared | refused, naming MA-41 and `ezsdr.time`; the agreeing and the absent section plan | MA-41 |
| `kc_25_an_admitted_update_changes_the_configuration` | Session sets `test.gain` to 3 | effective configuration holds 3 and the Provider receives the Action | KC-25, KA-10 |

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

1. **Resolved:** Vision §38 now says a Peripheral's timing class is known when the instance is created and appears among its declared capabilities, so `validate()` can match it and `prepare` may only narrow it (MA-17).
2. **Resolved:** Vision §14 now says `real` is at the top of every aspect, so a Hardware Run has something to record (MA-42).
3. **§7's role list and audit §13's `module-api` line disagree on one member.** Audit §13 names four roles, `Provider`, `Executor`, `Sink` and `Link`; the Vision names five, adding `Authority` from re-review R8. The Vision is later and is followed here.
4. **Resolved:** Vision §32 now says the Kernel coordinator owns the loop, the Authority's `next_wakeup` sets the instant, and the fixed order keeps determinism independent of assembly order (MA-30).

## 10. Deferred

The Peripheral and Endpoint vocabularies. The WASM and GPU execution ABIs, which each Executor owns. The Plugin wire protocol; MA-46 is what makes it additive. Out-of-process Radio Providers (Vision §64). The budget-feasibility arithmetic of MA-39, which needs declared port rates.

## Changes

| date | rules | change | record |
|---|---|---|---|
| 2026-10-07 | UC-3, MA-13 | a `cold` update restarts a function only if it was running; the Vocabulary places the end and the new origin; an orderly `stop` has no tail | [spec 22](../plan/maintenance/22-timing-simplification.md) |
| 2026-10-09 | MA-30 | the coordinator handles every failure of a round on its own terms (KC-30), so a failure after a `DeviceLost` in the same round is no longer dropped (checked by spec 06's `kc_30_a_panic_after_a_device_lost_in_one_round_fails_the_run`, and by `kd_01_every_lost_device_of_a_round_is_reported_and_the_first_failure_decides`, which now also has two ordinary errors in one round) | [note 25](../plan/maintenance/25-fault-and-mark-notes.md), #65 |
| 2026-10-10 | MA-1, MA-10, MA-32, MA-36, MA-38, MA-46, §3, §4 | deleted the fields no rule gave a meaning and nothing read: `ComponentDescriptor.timing`'s `preferred_batch`, `stateful` and `parallelism`, `requires.memory_bytes` and `IslandDecl.batch`; deleted the reserved `Deployment::Plugin` (a Plugin arrives with its protocol); deleted `Endpoint::EventIn` and `EventOut`, which carried no handle, and made `Endpoint` non-exhaustive; the Action `PeripheralCommand` and `CompileRule::PeripheralCommand` are `Command` (#55); `min_command_lead`'s doc comment and schema text say "from dispatching", and §4's `ProviderInstance` lists the field | [audit item 7](../plan/maintenance/24-prefreeze-audit.md), owner decision 2026-10-09 |
| 2026-10-10 | MA-37 | the budget check is deleted: a `RelativeBudget` is positive by construction and by deserialisation (TM-15) | [spec 26](../plan/maintenance/26-invariants-in-types.md) §2, audit item 1 |
| 2026-10-10 | MA-1, MA-32, MA-46, §3, §4 | `ModuleDescriptor.deployment` and `Deployment` are deleted: with in-process the only deployment, the field carried nothing; a Plugin deployment adds it with its protocol | [audit item 7](../plan/maintenance/24-prefreeze-audit.md) (F47's alternative), owner decision 2026-10-10 |
| 2026-10-10 | MA-5, MA-9a, MA-14b, §4 | `PrepareContext` gains `source`, the instance's source root, and no Module formats a source; `ActionReceiver::recv` returns the Action with its authored target and the node a `{resource}` target resolved to; `PrepareContext` and `AttachedPort` are non-exhaustive, with testing constructors | [spec 27](../plan/maintenance/27-role-typed-identities.md) §2, [audit 28](../plan/maintenance/28-forward-audit.md) decision 5 |
| 2026-10-11 | MA-14, MA-14a, MA-14b, MA-46, §4 | `recv` returns `Dispatched { id, action, node }`, a document with a schema; an event answering one Action names its id as `action`; a Module's `submit(Abort)` returns a real id (KC-24), neither admitted nor logged and recorded nowhere else (owner, 2026-10-11); `PrepareReport` names no fragment (SB-41) | [spec 27](../plan/maintenance/27-role-typed-identities.md) §3 |
| 2026-10-12 | MA-5, MA-6, MA-10, MA-24, MA-34, MA-35, MA-37, MA-39, §4 | `PrepareContext.events` is a view of the collector bound to the instance's source root, whose fallible `resolve` and `emit_control` refuse a source outside it, and an Executor's root is the Island of the fragment prepared; a `Resource` node declares `memory_domains`, non-empty when it has ports, checked on every node of each bound tree; MA-39 applies one rule, every producer domain reaches some consumer domain directly or through the selected Link in either orientation, to every graph link and feed, resource endpoints included (D31 closed); a component owns its parameter keys, declares each once, and its `ParamDecl.update_class` is optional, absent meaning fixed for the Run | [spec 29](../plan/maintenance/29-amendments.md), pre-freeze item 3 |
| 2026-10-13 | all; MA-12, UC-5, MA-10, MA-14a, MA-30, MA-39, MA-41, MA-42, MA-46, §7, §9, §10, the header | rewritten to spec 22's DA-1 format: rationale, history, phase labels, review ids and finding ids leave the rules, each rule's Test line moves to its own line under it, and the full text as it last stood is at commit `c347fbb`; MA-12's coercion exception is a reference to SB-46 — the re-match skips exactly the keys SB-46 exempts, whose exemption is keyed on the Kernel's own preview (`crates/ezsdr-kernel/src/plan/prepare.rs`) and not on a key the report declares — and its five tests are named (plan 31 §8 E3); UC-5's Producer obligation drops the remark on how many keys declare the class, which stated the implementation's state and was not a rule; MA-30 points at spec 06 KC-2a for an Island's declared affinity and real-time policy instead of restating its refusal; MA-39 states that with an `rt_policy` only each component's declared budget is checked, and that the arithmetic feasibility check waits for the first Processor that declares port rates (§10); MA-14a points at spec 04 RS-15 for a Module's `Abort` instead of restating it, and MA-10's, MA-41's, MA-42's and MA-46's rules lose the rationale sentences that explained them; §7's heading drops its phase; §9's resolved Vision issues 1, 2 and 4 become one-line stubs naming the section that now says it; §10 keeps only what is still deferred (the Peripheral and Endpoint vocabularies, the WASM and GPU ABIs, the Plugin wire protocol, out-of-process Radio Providers and MA-39's arithmetic), its implemented items now held by specs 07, 08, 09 and 14; the header's Scope and Not-in-scope rows name the specs instead of phases, in spec order (the Simulation Engine, spec 08, before MockRadio, spec 09); no other change of behaviour | [plan 31](../plan/maintenance/31-item8-kernel-specs.md) §4.2, §8 E3, this stage's brief |
| 2026-10-14 | MA-17, UC-6 | MA-17's Producer obligation and UC-6 name MockRadio without a version | documentation fixes D7, 2026-10-14 (handoff.md §3) |
| 2026-10-16 | MA-8, MA-13, MA-14, MA-15, MA-22, MA-24, MA-25, MA-27, MA-34, MA-41, MA-42, MA-44, MA-46, §7 rows `ma_06_signature_types_are_documents`, `ma_07_lifecycle_order_and_cleanup`, `ma_14a_reactor_emits_through_admit`, `ma_35_vocabulary_carries_its_own_content`, `ma_41_a_declared_time_class_that_disagrees_is_refused` (new), `kc_25_an_admitted_update_changes_the_configuration` | Test lines and §7 rows state only what their tests assert (the Test-line sweep): the reverse-order cleanup is named to spec 06's `kc_13_...` and the cleanup step order to `rs_06_cleanup_order_orderly`, not to `ma_07`'s row or `rs_06_cleanup_order_abort`; the Simulation-class prepare is named to the Ceiling's test, not to the coordinator's enforcement; the `ctx.actions` path, an Action edge, the empty-`memory_domains` refusal, the attached-end naming, the offending-key diagnostic, the declared `class` mismatch, the `ideal` Mock's `none` and `Resource`'s schema are named to the tests that assert them; an Action edge, an Executor applying an Action under its class, the coordinator never stepping a non-stepped Provider, and the double's `fidelity`, action-queue drain and generating no Events are added as `none named`; `kc_25`'s row drops MA-24 and UC-2, and `ma_35`'s and `ma_14a`'s input and expected columns now describe what their tests observe; fix round 1 (Sol 6.1's review) narrows MA-13's mode-reaching-`stop` citation to spec 06's `kc_14_an_arm_failure_is_failed_arm` and to `rs_06_cleanup_order_abort`'s cleanup-step modes alone, and names MA-46's committed schemas — `Resource`, `ModuleDescriptor` and `VocabularyDescriptor` — to `ov_22_schema_freeze`, leaving under `none named` only the absent compile-time document bound on `ModuleDescriptor` and `VocabularyDescriptor`; no change of behaviour | Test-line sweep, 2026-10-16 |
| 2026-10-17 | MA-32, MA-34, MA-35, MA-44 | a `VocabularyDescriptor`'s `KeyDecl`s may carry a numbered segment, and registration refuses a declaration outside its own prefix (MA-34) and a bad position, and two declarations that cover one concrete key (SB-2a); MA-35's list of what a `KeyDecl` carries and MA-44's keys name it; MA-32 cites MA-34 and SB-2a rather than restating them | [Phase 8 design 33](../plan/phase8/33-channels-and-streams.md) D2, §5 stage 1b-1 |
