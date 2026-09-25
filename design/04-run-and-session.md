# Phase 1 spec 04 — Run, Session, events and the Manifest

| Field | Value |
|---|---|
| Status | Accepted 2026-09-23 (Gate B; Phase 1 Step 5). Normative for `ezsdr-kernel::{run, session, event, policy, manifest, hash}`. Amended in Phase 2 by KA-4, KA-6, KA-7, KA-12, KA-13, KA-18, KA-19, KA-21, KA-22. |
| Scope | The Run state machine and its cleanup algorithm; Session as a Run with an action log, and the admission every Action passes; the Lease; the closed Policy table and the registered event kinds; the event pipeline with its never-dropping counters; the Manifest envelope, its namespaced sections and the content hashing of both. |
| Not in scope | The Radio Model's event payloads (Vocabulary, Phase 2) (committed by the Radio Model in Phase 2, RM-22); fault injection beyond the three kinds of spec 08 SE-3 (`rx_overflow`, `rx_sequence_error`, `device_lost`), which Phase 2 carries with the acceptance tests §58 #5 and #6 (Phase 4); the Python client surface (Phase 6); GraphEpoch, which stays future work while its prohibition is in force. |
| Vision § covered | §3; §11's lifecycle; §14's ExecutionClass and fidelity vector as recorded values; §27's update classes and the ban on structural mutation; §29; §35's `DEVICE_LOST` as a Kernel policy; §50; §53; §54's mapping of the Easy API onto Sessions. |
| Audit §14.1 items | 5 (Session and Lease), 12 (counters, `EVENTS_DROPPED`, the closed Policy), 13 (the Manifest envelope, sections and hashes). Findings 5, 12, 16, 19. |
| Re-review | R3 (Session Actions are admitted), R5 (determinism is a Simulation-class property), R18 (a child Run inherits the Lease). |
| Depends on | Spec 01 for time; spec 02 for `ContinuityMap` and `BurstRecord`; spec 03 for the pipeline, `PrepareReport` and the admission checks; spec 05 for the role traits and `ArtifactRef`. |
| Modal verbs | "must" and "must not" are normative (OV-4a). "Should" does not appear inside a rule. |

---

## 1. Purpose

Vision §3 collapses two things v3 kept apart: the interactive session and the reproducible experiment. In v3 the controller protocol was a second command channel with no provenance, so an interactive capture left nothing behind. Here every interaction is a Run, every Run leaves a Manifest, and the Easy API is a Run whose Spec is implicit and whose history is a typed action log.

That only works if three mechanisms are exact. Cleanup must be an algorithm, not a list, because it runs when something has already gone wrong and it is what stops a transmitter. Admission must be one function, shared by `validate()` and by every runtime Action, or re-review R3's hole reopens and a Session becomes the unchecked channel again. And the event path must lose bodies without ever losing counts, because a ten-minute Run with thirty thousand overflows has to record thirty thousand.

## 2. Evidence

- **v3 did not stop on disconnect.** `v3/source/tcp_iface.d:227-312` closes the client socket and continues its accept loop on a disconnect or a protocol error, and nothing in that range stops a controller. That a repeating transmission therefore continued is an **inference**, not a quoted fact: the controllers are separate threads spawned once in `app.d` and are not tied to the connection, so nothing ends them. That behaviour is defensible for a teaching rig and indefensible as a default; audit Finding 12 is why the Lease has two modes and why the default is the other one.
- **v3's capture reply carried nothing but samples.** `v3/source/controller/cyclicrx.d:427-443` writes a result count, then per stream a length and the raw bytes: no timestamp, no sequence number, no validity, no channel identity. The Manifest envelope of §7 is the answer to that, and the continuity metadata it carries is derived rather than asserted (SC-30).
- **UHD's asynchronous message queue holds 1 000 entries**, and UHD's own documentation calls its `O`, `D`, `U` and `L` console prints "generally harmless". For publication-grade data they are not, which is why RS-43 counts what RS-46 may drop.
- **UHD coerces silently unless the caller reads back**, and v3 printed the actual rate to standard output. The PrepareReport of spec 03 lands in the Manifest here.

## 3. Model

```text
Run (kind: spec | session)
 ├── state      Created → Validated → Planned → Prepared → Armed → Running → Stopping{mode} → CleanedUp{why}
 ├── lease      Attached | Detached{ttl, renewable}          expiry on the host clock
 ├── policy     EventKind -> continue | mark_artifact | stop | abort
 ├── events     never-dropping counters + a bounded ring + EVENTS_DROPPED
 ├── log        Session only: typed Actions, each Admitted or Rejected
 └── manifest   written once, at the end, for every Run that terminates

admit() is one function. validate() runs it over a Spec; a Session runs it over each Action.
```

## 4. Types

```text
RunId            "<node>:<pid-hex>:<unix-nanos-hex>-<counter>"   opaque; see 00-overview X6
RunKind          spec | session
CleanupMode      orderly | abort
RunState         Created | Validated | Planned | Prepared | Armed | Running
                 | Stopping { mode: CleanupMode } | CleanedUp { termination: Termination }
Termination      Completed | Stopped { cause: StopCause } | Failed { stage: Stage }
StopCause        client | client_disconnect | lease_expiry | policy { kind: EventKind } | abort { cause }
Stage            validate | plan | prepare | arm | run

SessionAction    SetParameter { target: ResourceId, key: Key, value: Value }
                 | Vocabulary { ns: Namespace, verb: Ident, target: ResourceId,
                                at: optional TimePoint, params: Map<Key, Value> }
                   at is a Kernel field, not a param: SB-4 caps a Value at one level of nesting
                   and a TimePoint is two, so a time could not travel inside params at all
                 | Stop { target: optional ResourceId }
                 | Release | Adopt { token } | Renew | RunChild { spec_hash, binding_hash }
LogEntry         { seq: u32, time: TimePoint, action: SessionAction, outcome: Outcome }
Outcome          Admitted { coercions: [Coercion], warnings: [Warning], dispatched: [ActionId] }
                 | Rejected { violations: [Violation] }

Lease            { mode: Attached | Detached { ttl_ms: u64, renewable: bool },
                   token: optional, holder: optional, expires_at_host: optional, adoptions: u32 }

Severity         debug | info | warning | error | fatal
EventKind        dotted segments of [A-Za-z][A-Za-z0-9_]*; the Kernel registers the five of RS-27,
                 a Vocabulary the rest, under that owner's namespace
Reaction         continue | mark_artifact | stop | abort
Policy           { table: Map<EventKind, Reaction>, severities: Map<EventKind, Severity> }
Event            { source: ResourceId, time: TimePoint, severity: Severity,
                   kind: EventKind, payload: Value }                   the control-path form
EventRecord      a fixed-size hot-path record: pre-resolved source and kind indices, a TimePoint,
                 a severity, and an inline payload of at most 32 bytes
CounterRow       { source: ResourceId, kind: EventKind, count: u64 }
Manifest         see §7
ArtifactRef      { id: Ident, kind: Namespace, uri: string, hash: ContentHash,
                   size_bytes: u64, partial: bool, marks: [{ kind: EventKind, time: TimePoint }],
                   continuity: [ContinuityMap] }     in SampleClock order; see RS-40
ContentHash      "sha256:" followed by 64 lowercase hex digits
RunError         StructuralMutationForbidden | RunNotRunning | LeaseTtlRequired | LeaseNotRenewable
                 | AdoptRejected | UnknownEventKind { kind } | SectionNamespaceForbidden { ns }
                 | ReplayDivergence { field } | PayloadTooLarge
```

## 5. Normative rules

### The Run and its states

- **RS-1** Every execution is a Run, including an interactive Session (Vision §50). A Run has an id, a kind, an optional parent, a Lease, a compiled Policy and, for a Session, an action log. The Manifest records these as its `run`, `lease`, `policy` (absent when the Run failed before its Policy compiled, as `plan` is absent before `plan()`; D69) and `action_log` fields. *Checked by `rs_01_run_id_is_generated_and_unique_within_the_process` and `ov_22_schema_freeze`.*
- **RS-1a** The in-process Run object composes the same id, kind, parent, Lease, compiled Policy and optional Session action log. *Checked in Phase 2 by `kc_45_manifest_fields` (KC-5).*
- **RS-2** The states are those of §4, and the only legal transitions are forward through `Created → Validated → Planned → Prepared → Armed → Running`, from any of those into `Stopping`, and from `Stopping` into `CleanedUp`. There is no `Failed` state: a failure is a `Termination`, reached through `Stopping`, because a Run that failed and was not cleaned up is a transmitter nobody turned off.
- **RS-3** A failure at any stage moves the Run to `Stopping { abort }` with `Failed { stage }`, and cleanup runs. A cancellation before `Running` moves it to `Stopping { orderly }` with `Stopped { client }`, performing only the steps that apply to the state reached.
- **RS-4** The graph's structure does not change while a Run is `Running`. An attempt to add, remove or reconnect a component or a link is `StructuralMutationForbidden`. A structural change is `stop`, re-plan, `start`; a future GraphEpoch may make it atomic, and the prohibition holds meanwhile (Vision §27). *Checked in Phase 1 as the predicate `RunStateMachine::check_structural_mutation`; its call site is the first API that changes a plan (a re-plan or a GraphEpoch switch); Phase 2 has none, because a `cold` update changes a value and not a structure (UC-3).*
- **RS-5** Every transition is recorded with its runtime `TimePoint` and the host UTC time, and the sequence appears in the Manifest. *The recording is checked in Phase 1 by `rs_02_happy_path_transitions`. Checked in Phase 2 by `kc_01_a_validate_failure_records_its_reason` (KC-43).*

### Cleanup

- **RS-6** Cleanup is this ordered algorithm, not the unordered list Vision §53 gave before Step 5. Every step is attempted even when an earlier one failed; each failure is recorded and the sequence continues.

  ```text
  0. clean up child Runs, each by this same algorithm
  1. freeze dispatch and cancel every pending burst and timer
  2. stop TX on every Provider, in reverse dependency order
  3. stop RX on every Provider, in reverse dependency order
  4. cancel outstanding Peripheral operations
  5. restore baseline state, in reverse dependency order
  6. finalise artifacts, marking as partial anything still open
  7. flush the event path and collect the counters
  8. release the Lease and write the Manifest
  ```

The coordinator performs the steps as follows. **0** nothing (Phase 2 has no child Runs, KC-37). **1** refuse every further submission, empty every undelivered Action queue, cancel every callback the coordinator scheduled. **2** `Provider::stop(mode)` on each Provider instance. **3** in an `orderly` cleanup of a Simulation Run, first run the stepping loop until `next_wakeup` returns `None`, at most `DRAIN_WAKEUP_CAP` times, so that each Provider delivers its declared tail (MA-13) — counting consecutive results at one instant as KC-22 does, emitting `STEP_LIVELOCK` and stopping above `STEP_ROUND_CAP`, stopping at once when the end has escalated to `abort`, and stopping at its next wakeup once the `closing` flag is set, which every cleanup operation after the drain's own call sets on entry and the coordinator sets when `run_cleanup` returns, so that a drain RS-8a abandoned stops before the next step acts; a drain round also skips every instance whose step-5 `cleanup()` has been called; then `Executor::stop(mode)` on each Executor instance and `Sink::stop(mode)` on each Sink instance, keeping the artifacts each Sink returns. **4** nothing (no Peripherals). **5** `cleanup()` on every instance that reached `prepare`. **6** record the marks of RS-30 on the artifacts (KC-42). **7** drain the event path a last time, snapshot the counters, and read every created link's `drops()` (KA-18). **8** nothing inside `run_cleanup`: when it returns, the coordinator releases the Lease, assembles and seals the Manifest (KC-44), and only then drops the created links. In steps 3 and 5 a Module slot is taken with `try_lock`: a slot an abandoned earlier step still holds is not waited for — step 3's drain steps the other instances without it, and step 5 records a `CleanupFailure` for it ("KC-39: the instance is still held by an abandoned cleanup step") — so that one wedged Module costs one deadline and not one per later step. A radio Provider stops its transmit side before its receive side inside `stop` (RM-16), which is where RS-6's TX-before-RX order lives once the Kernel cannot tell them apart. Step 3's drain runs once, when the first fragment of step 3's reverse pass is performed, before that fragment's instance is acted on (Phase 2, KA-12).

- **RS-7** Step 1 precedes step 2. Vision §53 listed "stopping TX" before "cancelling pending bursts" before Step 5, but a burst still queued when TX stops will reopen it on a device that honours timed commands, so the dispatch freeze has to come first.
- **RS-8** Steps 2, 3 and 5 run in reverse dependency order, the inverse of the arm order of SB-39. The device that was armed first is released last.
- **RS-8a** Every cleanup step runs under a deadline, the Provider's declared one where it has one and a Kernel default otherwise. A step that exceeds it is abandoned, recorded in `termination.cleanup_failures` as a timeout, and the sequence continues. RS-6 covers a step that returns an error and says nothing about one that never returns; without a deadline a single wedged peripheral stops cleanup before step 8, so no Manifest is written for exactly the failure that most needs recording, and since a Lease expiry and a policy abort both terminate through this path, one hung Provider wedges every route out of a Run.
- **RS-9** An `abort` differs from an `orderly` stop only in the mode passed to each Provider: an orderly stop delivers the declared tail, an abort does not. Both run the same nine steps. Under `orderly` cleanup step 3 drains the stepped instances before the Executors and Sinks stop; under `abort` it does not (Phase 2, KA-12).
- **RS-10** An abort raised while an orderly stop is in progress escalates the remaining steps to abort mode and is recorded in the termination alongside the original cause. *Escalation is checked in Phase 1 by `rs_10_abort_during_orderly_escalates`. Checked in Phase 2 by `kc_32_an_abort_during_orderly_escalates_and_is_recorded` (KC-40).*
- **RS-11** `run_cleanup()` reaches `ReleaseAndWriteManifest` (step 8) regardless of mode, of how many fragments were prepared — including none, which is what a `validate` failure leaves — and of earlier steps failing or timing out. A failed Run's provenance is provenance. *Checked by `rs_11_cleanup_reaches_the_manifest_step_with_nothing_prepared`, `rs_06_cleanup_step_failure_continues` and `rs_8a_wedged_cleanup_step_times_out` (D70).*
- **RS-11a** The coordinator writes one Manifest at step 8 for every Run that reaches `CleanedUp`, including a failed Run, and seals it immutable afterwards. *Checked in Phase 2 by `kc_44_a_wedged_step_does_not_prevent_the_manifest` (KC-44) and `kc_01_a_validate_failure_still_writes_a_manifest` (KC-1).*

### Session

- **RS-12** A Session is a Run whose ExperimentSpec is implicit and hashed like any other (Vision §3). It is built from the BindingProfile by spec 03's derivation, SB-22c and its table SB-T2, which is the one place the derivation is written: an Executor per Island executor name, one **output** per other binding carrying `feed`, one resource with empty `requires` per remaining binding whose Module holds Provider, and a Module holding several roles bound once per role (MA-1, D87). *Checked by `rs_12_a_session_compiles_through_the_whole_pipeline` and `rs_12_a_multi_role_module_is_bound_once_per_role`.* `connect()` then runs the whole pipeline of SB-37 and leaves the Run `Running`, so the first Action has a runtime `TimePoint` to carry. The Sink clause is not a convenience: RS-4 forbids adding a recorder while the Run is `Running` and RS-14 refuses a capture with no recorder, so an implicit Spec with no outputs would reject every capture in Vision §3's own example. It is an **output** and not a component because a Sink is a Module role and not Executor-loaded code (MA-25), and because only an output carries the link that feeds it: the earlier component form had no field for that link at all, so a Session's capture was connected to nothing and recorded nothing. Taking the Sinks from the bindings keeps the rule generic, because the Kernel binds what the profile declared and needs no notion of which resources can be captured.
- **RS-13** The Session action envelope is the closed set of §4, and it is not the Kernel Action set: the Kernel's Actions are what a Reactor emits into the real-time path, while a lease operation or a recorder request is a control-path act with no real-time meaning. Each Session Action compiles to zero or more Kernel Actions, or to a Lease or Run operation. *Checked: the compilation table of RS-14 and its tests.*
- **RS-13a** Only the lifecycle verbs are the Kernel's: `SetParameter`, generic over a namespaced key and value, plus `Stop`, `Release`, `Adopt`, `Renew` and `RunChild`. A domain verb is `Vocabulary { ns, verb, target, params }`, and the Vocabulary that registers the verb declares how it compiles. Vision §3's log sketch named `StartRepeat` and `Capture` directly before Step 5; here they are `radio.start_repeat` and `sink.capture`, because `repeat` is a Radio Model capability in audit §13 and a recorder is a Sink, and a Kernel that enumerated them would need a new variant for the first peripheral sweep or calibration verb. Replay (RS-20) needs a closed envelope and a matching profile hash, not a closed list of verbs, so nothing is lost.
- **RS-14** The Kernel's own compilations are: `SetParameter` to an `UpdateParameter` under the parameter's declared update class; `Stop { target }` to a Kernel `Stop` addressed to that resource and `Stop {}` to the Run's own stop; `Release`, `Adopt` and `Renew` to Lease operations; `RunChild` to the creation of a child Run from the Spec and the profile it names by hash (RS-25a). A `Vocabulary` action compiles as its registering Vocabulary declares: `radio.start_repeat` to a `TxBurst` carrying the repeat attribute (SC-26), and `sink.capture` to a **timed** `UpdateParameter` addressed to a recorder the profile **bound** — carrying the `at` RS-19 resolved, which is what makes Vision §61's `capture(n, at:)` a capture at that instant rather than at the next one — refused when it bound none (RS-12, SB-17). The target is the **Sink**, addressed as `ResourceId { node: LOCAL, path: sink/<output id> }` (SB-22h), and not the resource the verb named: it is the recorder whose parameter changes. With more than one recorder bound, the action must name one of them. A verb whose namespace no loaded Vocabulary claims is rejected and logged. The **value** an `UpdateParameter` compilation carries is the action's own parameter under the rule's `key`; an action that carries none is refused (`ezsdr.vocabulary`). The Kernel supplies no default: a default is a Vocabulary meaning living in Core (OV-21), and it is also invisible to RS-20, which reproduces a Run from the logged `SessionAction` and would otherwise take the value from the Kernel's version rather than from the log. A `TxBurst` compilation carries the `late_policy` its `CompileRule` declares (SC-27); a rule declaring `RejectAtPlan` is refused at Vocabulary registration (MA-32), because a Session burst's target is resolved at `compile` and the Action never passes a plan stage. A Vocabulary wanting both behaviours declares two verbs (finding D46).
- **RS-15** Every Action is appended to the log with its sequence number, its runtime `TimePoint`, and its outcome. Sequence numbers are dense: a **rejected** Action occupies one too, because a log with holes cannot be reproduced or audited. An Action is a well-formed document (SB-4, SB-9a): one whose value the canonicaliser cannot hash was never an Action, and the log refuses it before it takes a number — a log the canonicaliser cannot hash would break RS-11 for the whole Run, which is worse than refusing one entry. For the same reason the log refuses an entry carrying an id the Manifest's deserialiser would refuse: an Action's target off the local node (X7) or with a path outside SB-1's grammar, and a time — the entry's or a Vocabulary Action's `at` — in a domain off the local node. An Action is built in Rust and never parsed, so this is D91's defect on the log's path (finding D106). `SessionLog::check_entry(time, action)` performs exactly the checks `append` performs, without appending; `append` calls it first. The coordinator calls it before compiling a Session Action (KC-28) (Phase 2, KA-21).
- **RS-16** Every Action passes `admit()` on the control path **before** anything is dispatched. A rejected Action is logged as rejected and never reaches the real-time path (Vision §3, re-review R3).
- **RS-17** Admission is **one check set in one order** at every stage: 0. For an `UpdateParameter` whose target resolves to a **Provider** fragment and which arrives on the **control path** — from a Session or from the Spec's schedule — the target Provider's `coerce` is called once, with a `Requested` whose `resource` is the fragment's matched node and whose `constraints` are `Eq` of every scalar value of the fragment's current configuration (KC-27), the proposed key's `Eq(value)` replacing its own. An `Err`, or a `rejected` entry for any key, is a violation (`ezsdr.coercion`) and the Action is refused; an `applied` value for the proposed key that differs from the proposed value is a `Coercion`, which step 2 judges under SB-45 and which replaces the Action's value; a missing `applied` value for the proposed key is a refusal. An Action a **Module** submits (MA-14a) is not coerced by the Kernel, because the stepping loop holds every Module while one runs; its target enforces its own envelope and emits its Vocabulary's event (MA-14). *Forward: whether the reactive path coerces is Phase 5's to settle (Phase 2, KA-6).* The registered admission checks of SB-30 (`AdmissionCheckRegistry::run`), the coercion policy of SB-45 (`coercion_policy` and `apply_coercion`), and the parameter's declared update class (RS-52, SB-2). Each step is one Kernel function, and each stage composes the three over what it proposes: `validate()` over a Spec's requested configuration, **reporting** every violation it finds (SB-38); `prepare()` over the applied configuration inside `collect_prepare`, **failing** the transaction (SB-41, SB-42); `Admitter::admit` over one Action's key and value, with the per-fragment configuration of SB-30, **refusing** before dispatch (SB-30's third point, RS-16, MA-14a). An update through an undeclared class is rejected at admission, so an Executor never receives one (Vision §27); that step applies at the runtime stage, which is the only one at which an update class exists to check. What is shared is the check set and its order, which is what re-review R3 requires. The sites differ in what they propose **and in whether they report, fail or refuse**, and each of those three is required by another rule — so "one function" would have to take three aggregation modes, and it is the step functions rather than the composition that are shared (finding D45).
- **RS-18** An Action that arrives while the Run is not `Running` is rejected and logged. *The predicate is checked in Phase 1 by `RunStateMachine::check_running`. Checked in Phase 2 by `kc_24_a_module_action_during_cleanup_is_refused` (KC-24).*
- **RS-19** A timed `Vocabulary` action carries its instant in the Kernel `at` field beside its `params`, because a time is generic — `SetTimer`, `PeripheralCommand` and `TxBurst` all carry one — while the verb is not, and because SB-4 caps a `Value` at one level of nesting so a `TimePoint` cannot travel inside `params`. An action with no `at` is admitted at the earliest instant its target allows: for a target that resolves to a Provider fragment, the coordinator's current instant in the Authority's primary root plus that instance's `min_command_lead` rescaled to the primary root with TM-9 and rounded up; for any other target, the current instant. The applied time is recorded in the entry as a coercion of the requested "as soon as possible". Either way the resolved instant reaches the compiled Action's own `at` (RS-49); a `SetParameter`, which has no `at` field to leave empty, compiles to an Action with none and records no coercion, because nothing was requested to coerce. This is what makes Vision §61's `capture(n, at: TimePoint | sample_index)` and v3's `onTime` expressible on the Session path, which TM-10 exists to support.
- **RS-20** Replaying a Session means re-applying its admitted entries against the same BindingProfile. A replay against a profile with a different hash reports `ReplayDivergence` rather than proceeding (Vision §3).

### Lease

- **RS-21** A Lease is `Attached` or `Detached { ttl_ms, renewable }`, and the default is `Attached`. A `Detached` Lease without a TTL is `LeaseTtlRequired` (Vision §53).
- **RS-22** The TTL runs on the host monotonic clock, injected so that tests can advance it, and the corresponding UTC time is recorded. Run time would be wrong in both directions: a paused simulation would keep a detached transmitter alive forever, and a simulation running faster than wall clock would expire it early. A Lease is about a client's absence, which is a wall-clock fact.
- **RS-23** An `Attached` Lease ends the Run when its client disconnects, with `Stopped { client_disconnect }` and an orderly cleanup. A `Detached` Lease survives the disconnect until its TTL expires, and expiry runs the same cleanup with `Stopped { lease_expiry }`. *The Lease half is checked in Phase 1. Checked in Phase 2 by `kc_36_attached_disconnect_ends_the_run` (KC-36) and `kc_36_detached_lease_expiry_ends_the_run` (KC-36).*
- **RS-24** A Detached Lease is granted with a token, and only `Adopt { token }` reclaims it. Adoption by run id alone would let any client seize a live transmitter. A wrong token is `AdoptRejected`; a `Renew` on a non-renewable Lease is `LeaseNotRenewable`. Both outcomes are logged.
- **RS-25** A child Run inherits its parent's Lease and may not declare its own. Ending the parent ends its children first (RS-6, step 0), and a Detached parent keeps its children until its TTL expires (Vision §53, re-review R18). *Step 0 is checked in Phase 1 by `rs_25_child_inherits_the_lease`; a child Run and its inherited Lease are the Phase 6 coordinator's, where `sdr.run(spec)` first exists; Phase 2 refuses `RunChild` (KC-37).*
- **RS-25a** A child Run is a Run like any other and has its own profile, which `RunChild` names by hash beside the Spec's (RS-14): the parent's cannot be reused, because a Session profile's recorder carries `feed`, which a Spec Run refuses (SB-22g), binds Providers the child's Spec may not name (SB-22d) and places no component of the child's graph. The child's profile binds each of its resources to an instance its parent binds, under an equal binding description (SB-3), and names a binding with the same description as its parent's `authority`, because the child holds no Lease of its own (RS-25) and so no device its parent does not hold. *Forward obligation, the Phase 6 coordinator's, where `sdr.run(spec)` first exists; Phase 2 refuses `RunChild` (KC-37).*

### Policy and events

- **RS-26** A Policy maps a registered `EventKind` to one of exactly four reactions: `continue`, `mark_artifact`, `stop`, `abort`. It is a table. There is no expression language and no rules engine; logic that needs conditions belongs in a Reactor or in client orchestration (Vision §53).
- **RS-27** The Kernel registers only the kinds it emits itself or owns the policy for: `EVENTS_DROPPED` from its own drain (RS-35), `LINK_BACKPRESSURE` from a DataLink policy (SC-20a), `PROCESSOR_DEADLINE_MISS` from a `RelativeBudget` it defines (TM-15), `DEVICE_LOST`, which Vision §35 states in as many words is a Kernel policy and which the Module boundary reports as a typed error, and `STEP_LIVELOCK` from its own stepping loop (MA-30). Every other kind is registered by the Vocabulary or Module that emits it, under that owner's namespace: the Radio Model registers `RX_OVERFLOW`, `TX_UNDERFLOW`, `TX_DISCONTINUITY`, `LATE_COMMAND`, `TIME_ERROR`, `ALIGNMENT_ERROR` and `CLOCK_LOST`; the Peripheral model registers `PERIPHERAL_TIMEOUT` and `PLUGIN_FAILURE`; Calibration registers `CALIBRATION_INVALID`; Host I/O registers `TUN_QUEUE_DROP`. Audit §13's Kernel line names the envelope, the counters, `EVENTS_DROPPED` and the Policy mechanism, not a registry of concrete kinds, and a Kernel holding a dozen radio kinds would make the thirteenth a Kernel change, which is the growth Vision §5's three tiers exist to prevent. Vision §29's list wrote `DEVICE_DISCONNECTED` where §35 writes `DEVICE_LOST` until Step 5; this document uses `DEVICE_LOST`. An `EventHandle` whose **row or kind** the tables do not have is refused with its own error rather than panicking — both fields, because checking only the row let a fabricated kind into the ring and moved the panic into the coordinator's drain, away from the Module that caused it. Its fields are public, so a Module can fabricate one, and RS-32's infallible hot path is a promise about the Kernel's own handles (MA-9). A **second** declaration of one kind is refused rather than overwriting the first, so one Vocabulary cannot replace another's default reaction and severity; the refusal is its own error and not "the kind is not registered", which said the opposite of what had happened.
- **RS-28** A kind's default reaction is declared where the kind is registered. The Kernel's five are: `EVENTS_DROPPED` and `LINK_BACKPRESSURE` continue and mark the artifact, `PROCESSOR_DEADLINE_MISS` marks the artifact, and `DEVICE_LOST` and `STEP_LIVELOCK` abort, both at `fatal` severity. Vision §53's two worked examples are then satisfied by `DEVICE_LOST` here and by the Radio Model's declaration for `RX_OVERFLOW`. Putting a radio kind's default reaction in the Kernel would be a radio policy decision inside a frozen tier.
- **RS-29** A kind with no entry in the Run's Policy takes a default from its severity: `debug` and `info` continue, `warning` and `error` mark the artifact, `fatal` aborts. A flat default would be wrong in both directions: `continue` would ignore a Module's fatal event, and `mark_artifact` would taint a capture over an informational one.
- **RS-30** `mark_artifact` records the kind and time against every artifact open at that moment, and those marks appear in the `ArtifactRef`.
- **RS-31** An Event carries a source, a `TimePoint` in a named domain, a severity, a kind and a schema-versioned payload (Vision §29). *Checked in Phase 1 by `schema_freeze` for the shape. Checked in Phase 2 by `rm_20_schema_freeze` and `rm_22_payloads_round_trip` (RM-22).*
- **RS-32a** *Withdrawn (D51).* The Kernel does not interpret a Vocabulary's hot-path byte layout; a Vocabulary that needs one defines and decodes its own representation.
- **RS-32** Emitting on the hot path allocates nothing. The hot-path record is fixed in size, carries pre-resolved indices for its source and kind rather than strings, and holds at most 32 bytes of payload inline. A larger payload is produced on the control path only; attempting one on the hot path is `PayloadTooLarge`.
- **RS-33** Every `(source, kind)` pair reachable in the plan has a never-dropping counter, and the table is sized at `prepare` from the plan, plus one fallback row for pairs that were not foreseen. The counter is incremented before the body is queued, so a count is never lost even when the body is.
- **RS-34** Event bodies travel through a bounded ring. When the ring is full the body is dropped and a per-kind drop count is incremented. Phase 1 drops; it does not subsample. Vision §29 said "sampled or dropped" before Step 5, which are different mechanisms, and only one of them is needed to keep the counts honest. *Subsampling is a forward obligation of a Sink Module, if it is ever wanted at all.*
- **RS-35** Each time the ring is drained, one `EVENTS_DROPPED { kind, count }` event is emitted per kind that dropped since the last drain, carrying the delta rather than a running total. These are produced on the control path at drain time and never enter the ring: an `EVENTS_DROPPED` that could itself be dropped would break, under load and non-deterministically, the very invariant it exists to preserve. The invariant the tests assert is that for every kind, the counter equals the delivered bodies plus the sum of the `EVENTS_DROPPED` counts.
- **RS-36** A kind whose Policy reaction is `stop` or `abort` sets a per-kind escalation flag on the hot path, which the control path acts on even when the body was dropped. Without it a `DEVICE_LOST` arriving during an event storm would be discarded and the Run would continue on a device that is gone.
- **RS-37** *Withdrawn.* It forbade a metrics registry that does not exist, which binds nobody. Vision §29's prohibition stands as prose, and `00-overview.md` OV-23a's banned-token list is what would catch an exporter in the Kernel.

### The Kernel Action set

- **RS-48** The Kernel Action set is closed: `TxBurst`, `SetTimer`, `UpdateParameter`, `PeripheralCommand`, `Emit`, `Stop`, `Abort` (Vision §5, §19). It is defined here because audit §13 pairs events and actions on one line and this document already owns the event envelope, the Policy table and `admit()`. Adding a member is a Kernel major. Without an owner the set would reach the exit review with no rule, no schema and no test, while spec 03's `schedule` and this document's RS-14 both depend on its shape.
- **RS-49** An Action is a document with a schema (`00-overview.md` OV-10). Each carries its own fields, and all but `Abort` name a target `ResourceId`, which `Stop` may leave unset (RS-50):

  ```text
  TxBurst           { target, waveform: ArtifactRef, repeat: bool,
                      at: AbsoluteDeadline, requested_at: optional AbsoluteDeadline,
                      late_policy: LatePolicy, metadata: Map<Key, Value> }
  SetTimer          { target, at: AbsoluteDeadline, token }
  UpdateParameter   { target, key: Key, value: Value, class: UpdateClass,
                      at: optional AbsoluteDeadline }
  PeripheralCommand { target, verb: Ident, params: Map<Key, Value>, at: optional AbsoluteDeadline }
  Emit              { target, event: Event }
  Stop              { target: optional ResourceId }
  Abort             { cause }
  ```

  `UpdateParameter.at` is the instant at or after which the update takes effect under its class; absent means the first instant the class permits. It is optional for **every** class, `hardware_timed` included: a class cannot make the field mandatory, because a Session's `SetParameter` carries no time to put in it and Vision §3's bare `sdr.rx.gain = 20` would otherwise be refused. Without the field at all, `hardware_timed`, which the device applies at a declared instant, is unimplementable, and RS-19's admitted instant is computed and then discarded, which is what took the time out of Vision §61's `capture(n, at:)` on the Session path.

  Every timed Action names its instant with an `AbsoluteDeadline`, which Vision §19 defines for exactly this — "a TxBurst or PeripheralCommand target in a device ClockDomain" — and which TM-15 made a distinct type so that envelope checks have one thing to compare. A bare `TimePoint` on one Action and a deadline on its siblings would be the same concept in two types.

  `TxBurst` carries no channel list. Spec 02's §10 issue 8 leaves the mapping of a burst's channels onto a stream's channels to the Radio Model, and deciding it here would put a radio field in a frozen Kernel document, which is the reasoning RS-27 and RS-13a apply everywhere else. It travels in `metadata` under the Radio Model's namespace until Phase 2 settles it.

- **RS-49a** An Action that a Spec schedules is written as an **ActionTemplate**: the Action without its time field. A Spec cannot name a `ClockDomainId`, because domains are allocated at `prepare` (SB-16, TM-13a), and every timed Action names one through its `AbsoluteDeadline`. `arm` resolves the entry's `SpecTime` and substitutes it, producing the Action. The `UpdateParameter` template is timed too: a Spec's schedule entry carries a mandatory `SpecTime` (SB-16), so a scheduled parameter change takes effect at the instant the Spec named rather than at `arm`. An Action emitted at run time by a Reactor carries its own time and is never a template. Without the split, `ExperimentSpec.schedule` could hold no timed Action at all, which is the one thing it exists for.
- **RS-50** `Stop` carries an optional target: with one it stops that resource, without one it stops the Run. Vision §3 uses the word for both, and RS-14 compiles a Session `Stop` to one or the other, so the ambiguity has to be resolved in the type.
- **RS-51** `TxBurst.at` names an instant in the transmit stream's SampleClock and is subject to SC-23a, which may advance its inner `TimePoint` to the next sample instant and record the original in `requested_at`. The same pair reaches the Manifest through the `BurstRecord` of SC-28, where it is spelt `target` and `requested_target`; they are one pair of times under two names, the Action's and the record's. Its `late_policy` is SC-27's, and `RejectAtPlan` is legal only on a burst whose target is statically known.
- **RS-52** `UpdateParameter.class` is the parameter's declared update class (Vision §27). An Action whose class was not declared is rejected at admission (RS-17) and never reaches a Module.

### The Manifest

- **RS-38** The Manifest is a Kernel envelope with namespaced Module sections. The Kernel writes the envelope; each Module writes its own section; no Provider-specific field ever requires a Kernel change (Vision §50, Finding 19). The envelope carries the mandatory `version` Vision §10 requires of it, as the Spec and the BindingProfile do: the Manifest outlives every Run, so a reader that cannot tell version 1 from version 2 leaves SB-47's migrate-or-refuse with nothing to test and 04 §10's deferred "migrations of the Manifest" with no field to migrate on.

  ```text
  Manifest
  ├── version        1                    mandatory; SB-47 and SB-48 apply
  ├── run            { id, kind, parent, execution_class, fidelity, transitions }
  ├── spec           { hash, body, original_version, original_hash }
  ├── binding        { hash, body including environment verbatim }
  ├── plan           { summary, placement as bound, transfer costs }
  ├── prepare        { reports per fragment, merged effective }
  ├── admission      { matched, rejected, violations, coercions }
  ├── modules        [{ id, version, impl_hash, profile }] and the Vocabulary versions in use
  ├── components     Map<Ident, impl_hash>
  ├── inputs         [ArtifactRef]        artifacts the Run consumed, by reference
  ├── clocks         { domains, relations including epoch to UTC with uncertainty, sample_clocks }
  ├── events         { counters (complete, including zero rows), delivered }
  ├── lease          { mode, ttl, renewable, adoptions, released }
  ├── action_log     Session only
  ├── termination    { reason, at, host_utc, cleanup_failures, also }
  ├── artifacts      [ArtifactRef] with content hashes and continuity
  └── sections       Map<Namespace, opaque>   including `ezsdr.capture` when the profile asks, `ezsdr.links` always, and `ezsdr.failure` when a stage failed (KC-7)
  ```

- **RS-39** A Module writes only under its own registered namespace; a write elsewhere is `SectionNamespaceForbidden`. The transmit burst records of SC-28 and a Provider's envelope go in that Module's section, not in the envelope. The content's object keys must be ASCII, because this is the path the Kernel *designs* for untrusted Module content and it passes no `from_json`: left to hashing time, a non-ASCII key made `seal()` fail at cleanup step 8 after the Run had already transmitted (SB-9a, RS-11, OV-15). A Module's own namespace is its `ModuleId` read as a `Namespace` (SB-T0); a Module whose id is not one writes no section, and a section it hands in is refused and recorded as a cleanup failure of step 8. A Provider hands its sections to the Manifest through `ProviderInstance.sections`: the coordinator reads `instance().sections` of every Provider instance after `stop` and writes each entry with `Manifest::write_section(owner, …)` (KC-44) (Phase 2, KA-13).
- **RS-40** A capture's continuity metadata is the sequence of `ContinuityMap`s derived by SC-30, in SampleClock order, hanging off its `ArtifactRef`. There is more than one whenever the capture spans a rate change, which TM-13c makes a `cold` update that RS-4 permits during a Run, or a channel-count change (SC-30a): each ends one map and starts the next. The Kernel never assembles a map by hand (Vision §28). *Checked jointly with spec 02.*
- **RS-41** The fidelity vector is the weakest value per aspect over the bound Providers' declarations (spec 05). A Run in which no Provider declares an aspect records `none` for it.
- **RS-42** The ExecutionClass is fixed at binding resolution and never changes during the Run. `deterministic` may be claimed only for the Simulation class with a recorded seed; RealtimeEmulation, HardwareInLoop and Hardware are never deterministic (Vision §14, re-review R5).
- **RS-43** Vision §50 listed "random seeds" and an optional environment capture among the envelope's contents before Step 5; neither is an envelope field here. The Kernel owns no random number generator, so a seed is something the environment declared, and the environment is already recorded verbatim under `binding.body` (RS-38); a second copy would only raise the question of which is authoritative. The environment capture has no Kernel-defined content at all, so it belongs under `sections` as `ezsdr.capture`, written by the Module that produces it.
- **RS-44** Large data is always by reference. An `ArtifactRef` carries a locator, a content hash and a size, never the bytes (Vision §50, §51).
- **RS-44a** A waveform supplied by a client is ingested as an input artifact before the Action that references it is admitted: its bytes are stored, hashed by RS-45 and listed in `inputs`, and the Action carries the resulting `ArtifactRef`. An Action whose `ArtifactRef` resolves to nothing is rejected. Vision §58 #13 requires the Easy API alone to produce a Manifest carrying the waveform hash, and without this rule nothing turns the client's array into something hashable.
- **RS-45** Everything hashable is content-addressed: the Spec body after migration, the BindingProfile body including its environment, each artifact's bytes, each component's declared implementation hash. Two Runs with equal Spec, binding, component and input hashes are comparable by construction.
- **RS-46** The Manifest's own hash is computed over the Manifest with that field removed and is stored beside it, never inside the hashed body. *Checked: `rs_46_manifest_hash_is_stored_beside_the_body`, which recomputes the hash from the serialised body with `hash` removed, confirms an absent hash is skipped rather than written as `null`, and confirms re-sealing is stable. This annotation used to name `rs_45_hash_equal_for_equal_inputs`, which asserts RS-45's equal-inputs-equal-hash property and never looks at where the hash sits (exit-review finding).*
- **RS-47** The canonical form and the digest are those of `00-overview.md` OV-14 to OV-17. This document adds no rule about them and restates none (OV-4). *Checked there.*

## 6. Decisions

| # | Decision | Choice | Rejected (one line each) | Ceiling / upgrade path |
|---|---|---|---|---|
| R1 | State machine | Eight states, no `Failed`; failure is a termination reached through `Stopping` (RS-2) | A `Failed` state (a failed-but-uncleaned Run is a live transmitter); separate `Stopping` and `CleaningUp` states (splits one algorithm across two) | Only two cleanup modes; per-step timeouts are Provider-declared, not Kernel parameters |
| R2 | Cleanup order | RS-6, with the dispatch freeze first and reverse dependency order for the device steps | Vision §53's bullet order verbatim (a queued burst reopens TX after it was stopped); best-effort with no order (Mock and hardware would diverge) | Steps are best-effort with no retry policy; a failing step is recorded and the rest still run |
| R3 | Session log model | Two levels with a closed *envelope* and an open verb list: the Kernel owns the lifecycle variants and `SetParameter`, while a domain verb is `Vocabulary { ns, verb, target, at, params }` whose compilation its Vocabulary declares (RS-13, RS-13a, RS-14) | One set shared with the Kernel Actions (a recorder request has no real-time meaning, and adding one bloats the Reactor's vocabulary); a closed verb list, which was the first draft and would have needed a Kernel variant for the first peripheral sweep; an open *envelope* (then nothing could be replayed — it is the envelope that must be closed, not the verbs) | A verb whose namespace no loaded Vocabulary claims is rejected; a capture still needs a recorder the profile placed, and is rejected rather than silently buffered on the host |
| R4 | Lease clock | Host monotonic, injectable; UTC recorded (RS-22) | Run time (a paused simulation never expires; a fast one expires early); wall clock for expiry (an NTP step would end a Run) | none |
| R5 | Adoption | A token issued at grant (RS-24) | Adoption by run id (any client could seize a detached transmitter); full authentication now (Vision §62 stages it later) | The token is a shared secret on the client channel; an authentication layer later needs no change to the Lease type |
| R6 | Unknown event kinds | A severity-derived default (RS-29) | A flat `continue` (a Module's fatal event would be ignored); a flat `mark_artifact` (an informational event would taint a capture); a closed Kernel enum of kinds (a Module kind would need a Kernel change) | Keyed by kind alone; a later `(source, kind)` key is additive, since kind-only entries stay valid |
| R7 | Escalation under drop | A per-kind atomic flag on the hot path (RS-36) | Relying on the ring (a dropped `DEVICE_LOST` would never abort) | none |
| R8 | Event record | Fixed size, pre-resolved indices, 32 bytes inline, larger payloads on the control path (RS-32) | Heap strings on the hot path (allocation); one unbounded channel (it blocks or grows without bound) | `EventRecord` is in-process by `00-overview.md` X8, so raising the inline limit is a recompile, not a schema change |
| R9 | Counter table | Sized at `prepare` from the plan, plus one fallback row (RS-33) | A hash map on the hot path (allocation and locking); a fallback row per source (unbounded when a Module mislabels its source) | Unforeseen pairs merge into one row, which is reported as such |
| R10 | Manifest shape | A closed Kernel envelope with namespaced Module sections and the mandatory `version` Vision §10 requires of it; one `inputs` list for waveforms and calibration artifacts (RS-38) (amended 2026-09-22, 00 §11) | Separate lists of the same shape; a free-form Manifest (two Runs would not be comparable) | Sections are opaque; a query across Modules is a Sink's job |

## 7. Phase 1 tests

Fixtures: the test-double Provider of `00-overview.md` OV-21 with a recording call log and an injectable failure phase; the in-memory DataLink; the recording TestExecutor; an injectable host clock; a test event kind `test.custom`.

| test | input | expected | rules |
|---|---|---|---|
| `rs_02_happy_path_transitions` | a valid Spec and profile | exactly the eight states in order, all recorded | RS-2, RS-5 |
| `rs_03_failure_at_each_stage_reaches_cleanup` | an injected failure at validate, plan, prepare and arm in turn | `Failed { stage }` each time and `CleanedUp`; a cancellation from each state before `Running` ends `Stopped { client }` after an orderly stop | RS-3 |
| `rs_11_cleanup_reaches_the_manifest_step_with_nothing_prepared` | `run_cleanup` with no fragments, in `Orderly` and `Abort` | step 8 reached, and last | RS-11 |
| `rs_12_a_session_compiles_through_the_whole_pipeline` (`spec_binding.rs`) | a Session profile with a Provider and a fed recorder | the implicit Spec validates and plans; the Sink is its own fragment; the feed is in the plan's links | RS-12, SB-25 |
| `rs_12_a_multi_role_module_is_bound_once_per_role` (`spec_binding.rs`) | a Provider+Sink Module bound as `radio` and, with `feed`, as `rec`; then once with `feed` | a resource and an output, planned; bound once, a Sink only | RS-12, MA-1 |
| `rs_01_run_id_is_generated_and_unique_within_the_process` | repeated `RunId::generate` in one process | the documented form, all distinct | RS-1 |
| `rs_01_manifest_records_the_compiled_policy` | a compiled Policy with one override written into a Manifest | read back typed and equal; the override answers | RS-1 |
| `rs_06_cleanup_order_orderly` | a Running Run stopped by its client | the call log is exactly the nine steps of RS-6 | RS-6, RS-8 |
| `rs_06_cleanup_order_abort` | a policy abort | the same sequence with the abort mode | RS-6, RS-9 |
| `rs_06_cleanup_step_failure_continues` | `stop_rx` returning an error | later steps still called; the failure is in `termination.cleanup_failures` | RS-6 |
| `rs_07_pending_burst_cancelled_before_tx_stop` | a burst scheduled in the future, then a stop | the cancel precedes `stop_tx` in the log | RS-7 |
| `rs_08_reverse_dependency_order` | three fragments with `b after a`, `c after b` | released `c`, `b`, `a` | RS-8, SB-39 |
| `rs_10_abort_during_orderly_escalates` | an abort raised while stopping | the remaining steps switch mode; both causes recorded | RS-10 |
| `rs_04_structural_mutation_forbidden` | adding a link while Running | `StructuralMutationForbidden` | RS-4 |
| `rs_12_session_implicit_spec_hashed` | `connect()` with one binding | an implicit Spec with empty `requires` and a hash | RS-12 |
| `rs_16_rejected_action_not_dispatched` | a `SetParameter` violating the `test.limits` check | logged `Rejected`; the Provider's action queue is empty | RS-16, RS-17, SB-30 |
| `rs_17_undeclared_update_class_rejected` | a `SetParameter` on a parameter with no declared class | rejected at admission | RS-17 |
| `rs_18_action_before_running_rejected` | an Action while Armed | rejected and logged | RS-18 |
| `rs_14_capture_without_recorder_rejected` | a `sink.capture` verb with no recorder placed by the profile | rejected | RS-14, SB-17, RS-12 |
| `rs_12_bare_connect_then_capture_succeeds` | `connect()` against a profile that places a recorder, then a capture, with nothing else | admitted and dispatched; the implicit Spec contains the recorder as a placed component | RS-12, RS-14 |
| `rs_14_capture_compiles_to_update_parameter` | a capture verb with a recorder placed | one `UpdateParameter` dispatched to the recorder; an artifact id in the entry | RS-14 |
| `rs_13a_unknown_vocabulary_verb_rejected` | a `Vocabulary` action whose namespace no loaded Vocabulary claims | rejected and logged | RS-13a, RS-14 |
| `rs_48_action_set_is_closed_and_schematised` | each of the seven Kernel Actions | each round-trips through its schema; the set has no eighth member | RS-48, RS-49 |
| `rs_14_the_kernel_supplies_no_vocabulary_value_or_late_policy` | a `capture` whose action carries no value; a `start_repeat`; a Vocabulary declaring `TxBurst { late_policy: RejectAtPlan }` | the first is refused, the second takes the policy the verb declares, the third is refused at registration | RS-14, OV-21, SC-27 |
| `rs_52_a_scheduled_update_states_the_declared_class` | a schedule entry updating `test.gain` with its declared class; with another class; a key that declares none; a component declaring the same key | the first validates, the rest are refused — the `KeyDecl` is the only source (SB-2) | RS-52, RS-17, SB-2 |
| `rs_50_stop_with_and_without_a_target` | `Stop { radio }` then `Stop {}` | the first leaves the Run Running; the second begins `Stopping` | RS-50, RS-14 |
| `rs_8a_wedged_cleanup_step_times_out` | a Provider double that never returns from `stop_rx` | the step is abandoned and recorded as a timeout; steps 4 to 8 still run and a Manifest is written | RS-8a, RS-11 |
| `rs_44a_waveform_ingested_before_admission` | a repeat verb carrying raw samples, then one carrying an unresolvable reference | the first appears in `inputs` with a hash; the second is rejected | RS-44a |
| `rs_40_capture_across_a_rate_change` | a capture spanning a `cold` rate change | two `ContinuityMap`s on the artifact, in SampleClock order | RS-40, SC-30, TM-13c |
| `rs_35_dropped_events_never_enter_the_ring` | a storm that fills the ring, drained repeatedly | every `EVENTS_DROPPED` is delivered; the RS-35 invariant holds each time | RS-35 |
| `rs_17_provider_parameter_class_comes_from_its_key_decl` | `test.gain`, declared only in the Vocabulary; then `test.flag`, declared nowhere | the class comes from the `KeyDecl` and both `compile` and `Admitter` accept it; the undeclared key is still refused | RS-17, SB-2, MA-35 |
| `rs_19_capture_asap_records_applied_time` | a `sink.capture` verb with no `at` | the entry carries a coercion from "as soon as possible" to the applied time | RS-19 |
| `rs_15_log_sequence_is_dense` | five Actions, two of them rejected | sequence numbers 0 to 4 with nothing missing | RS-15 |
| `rs_15_the_log_refuses_an_id_the_manifest_would_refuse` | targets on node 1 and with an unparsed `radio//0`, a Vocabulary `at` and an entry time in a node-1 domain | each refused before it takes a number; a local, parsed target is logged | RS-15, X7, SB-1 |
| `rs_20_replay_divergence` | a replay against a profile with a different hash | `ReplayDivergence` | RS-20 |
| `rs_21_lease_default_and_ttl_required` | a Run with no Lease; then `Detached` with no TTL | `Attached`; then `LeaseTtlRequired` | RS-21 |
| `rs_22_ttl_uses_the_host_clock` | virtual time advanced an hour, the host clock a millisecond | the Run is still Running | RS-22 |
| `rs_23_attached_stops_on_disconnect` | a disconnect while Running | `Stopped { client_disconnect }`, orderly cleanup | RS-23 |
| `rs_23_detached_expires` | a 5 000 ms TTL, a disconnect, the host clock advanced 4 999 then 1 | Running, then `Stopped { lease_expiry }` with full cleanup | RS-22, RS-23 |
| `rs_24_adopt_requires_the_token` | `Adopt` with the right token, then the wrong one | adopted and logged; then `AdoptRejected` | RS-24 |
| `rs_25_child_inherits_the_lease` | a Detached Session with a child | the child has no Lease of its own; the parent's expiry ends the child first | RS-25, RS-6 |
| `rs_27_a_second_declaration_of_one_kind_is_refused` | a kind the `test` Vocabulary registered, registered again | `EventKindAlreadyRegistered { kind }` | RS-27 |
| `rs_28_policy_defaults_table` | one event of each of the five Kernel-registered kinds, plus `test.*` kinds registered by the test Module with their own defaults | exactly the declared reactions; no radio kind is registered by the Kernel | RS-27, RS-28 |
| `rs_29_unknown_kind_by_severity` | `test.custom` at info, error and fatal | continue, mark, abort | RS-29 |
| `rs_26_policy_override_from_spec` | `policies.failure: { test.custom: stop }`, where the test Module registered it with a default of continue | the Run stops on the first such event, overriding the declared default | RS-26, SB-18 |
| `rs_30_mark_artifact_marks_open_artifacts` | a `test.custom` event whose reaction marks, with a capture open | the mark appears on that `ArtifactRef` | RS-30 |
| `rs_32_emit_allocates_nothing` | 100 000 emissions under a counting allocator | zero allocations; the record's size equals the documented constant | RS-32 |
| `rs_33_counters_exact_under_drop` | a ring of depth 8, thirty thousand overflows from one source, drained once | the counter reads 30 000, eight bodies delivered, one `EVENTS_DROPPED` of 29 992 | RS-33, RS-35 |
| `rs_35_dropped_counts_are_deltas` | drop ten, drain, drop five, drain | two `EVENTS_DROPPED` events of ten and five | RS-35 |
| `rs_33_unforeseen_pair_uses_the_fallback_row` | an event from a source not in the plan | the fallback row increments; no allocation | RS-33 |
| `rs_49a_scheduled_action_is_a_template` | a Spec scheduling a `TxBurst` | the Spec validates with no time field; `arm` substitutes the resolved deadline | RS-49a, SB-16 |
| `rs_49_update_parameter_carries_its_instant` | a capture with no `at`; a bare `SetParameter`; a scheduled template | the resolved `earliest` reaches the Action's `at`; the bare one compiles with `at: None` and no coercion, and is not refused; the template is timed and `resolve` substitutes the Spec's instant | RS-49, RS-49a, RS-19 |
| `rs_19_vocabulary_action_carries_a_time` | a capture verb with an `at`, then without | the first is admitted at that instant; the second records the coercion | RS-19 |
| `rs_36_abort_survives_a_drop` | the ring filled, then a `DEVICE_LOST` whose body is dropped | the Run aborts; the counter reads one | RS-36 |
| `rs_11_manifest_for_every_terminal_run` | a failed, a stopped and a completed Run | one Manifest each, with the matching termination | RS-11 |
| `rs_45_hash_equal_for_equal_inputs` | one Spec loaded from two differently ordered and differently spaced JSON texts | equal Spec, binding and Manifest hashes | RS-45, RS-47 |
| `rs_39_section_namespace_enforced` | a Module writing under another Module's namespace | `SectionNamespaceForbidden` | RS-39 |
| `rs_43_no_seeds_field_in_the_envelope` | an environment declaring a seed | it appears only in `binding.body`; the envelope has no `seeds` field | RS-43 |
| `rs_38_environment_recorded_verbatim` | an arbitrary environment section | byte-identical in the Manifest | RS-38, SB-27 |
| `rs_41_fidelity_is_the_weakest` | two Providers declaring `hardware_quirk` and `envelope` for timing | the Run records `envelope` | RS-41 |
| `rs_42_determinism_only_in_simulation` | a RealtimeEmulation Run with a seed | no determinism claim in the Manifest | RS-42 |
| `rs_38_counters_include_zero_rows` | a Run with no events | every registered row present with a count of zero | RS-38, RS-33 |
| `rs_38_manifest_carries_its_mandatory_version` | a sealed Manifest, then the same document with `version: 2` | `version` is 1 inside the hashed body; the future major is refused by name | RS-38, SB-47 |
| `rs_44_partial_artifact_on_abort` | an abort with a capture open | the `ArtifactRef` is marked partial | RS-6, RS-44 |
| `rs_17_a_session_rate_change_is_coerced_by_its_provider` | Session sets `test.grid` to 19.5 against a Provider grid of 20; the same update under Spec-Run `reject` | Session entry is admitted with value 20.0 and records the coercion; Spec Run refuses it | RS-17, KA-6 |
| `rs_17_a_session_change_beyond_a_joint_limit_is_refused` | `test.count = 2` in effective configuration and `test.grid = 40` against joint limit 50 | rejected with `ezsdr.coercion` on `test.grid` | RS-17, KA-6 |
| `rs_17_a_scheduled_rate_change_under_reject_is_refused` | the same rate update in a Spec schedule under coercion policy `reject` | `Failed { arm }`, reason contains SB-46 | RS-17, SB-46, KA-6 |
| `kc_28_a_malformed_action_takes_no_sequence_number` | a Session Action with a non-ASCII value key, followed by a valid Action | first returns `Malformed`; next entry has sequence 0 | RS-15, KC-28, KA-21 |

## 8. Vision coverage

| Vision | This spec |
|---|---|
| §3 Sessions, the action log, admission, child Runs, replay | RS-12…RS-20 |
| §11 prepare, arm, start, running, stop, cleanup as one transaction | RS-2, RS-3, RS-6 |
| §14 ExecutionClass, fidelity vector, the scope of determinism | RS-41, RS-42 |
| §27 update classes only, no structural mutation | RS-4, RS-17 |
| §29 the event envelope, counters, storms, `EVENTS_DROPPED`, no metrics framework | RS-27…RS-36; the metrics prohibition is Vision prose (RS-37 withdrawn) |
| §35 `DEVICE_LOST` maps to abort as a Kernel policy | RS-27, RS-28 |
| §50 the Manifest envelope, sections, content addressing | RS-38…RS-47 |
| §5, §19 the Kernel Action set | RS-48…RS-52 |
| §53 cleanup, Lease modes, the closed Policy table | RS-6…RS-11, RS-21…RS-30 |
| §54 the Easy API maps onto Sessions | RS-13, RS-14 |
| §58 #5, #7, #13, #16 | the `rs_28_*`, `rs_11_*`, `rs_12_*` and `rs_16_*` tests |

## 9. Vision issues found

1. **§29 writes `DEVICE_DISCONNECTED` and §35 writes `DEVICE_LOST`** for the same condition. RS-27 uses `DEVICE_LOST`, and §29's list should be corrected in the R13 pass.
2. **§22 says a late burst "emits LATE" where §13 and §29 treat lateness as an event.** Both exist and mean different things: `LATE` is a block flag (SC-16) and the event is `TIME_ERROR` for a burst target already passed or `LATE_COMMAND` for a control command issued too late. The Vision should distinguish them.
3. **§53 lists the cleanup actions as unordered bullets** while §11 orders only stop before cleanup. RS-6 fixes an order, and RS-7 gives the reason the list's own order would be wrong.
4. **§3's `Stop` is ambiguous**, meaning either "stop this transmitter" or "end the Session". RS-14 splits it with an optional target, which requires the Kernel `Stop` Action to carry one (spec 05).
5. **§29 says event bodies "may be sampled or dropped".** Sampling and dropping are different mechanisms with different guarantees. Phase 1 implements dropping only (RS-34); subsampling, if it is ever wanted, is a Sink's policy.
6. **§50 lists "random seeds" and an environment capture in the envelope** although the Kernel owns no generator and defines no capture content. RS-43 keeps both out of the envelope: the environment is recorded verbatim already, and the capture is a Module's section.
8. **Vision §3's action log names `StartRepeat` and `Capture` as if they were Kernel vocabulary.** Both are domain verbs — `repeat` is a Radio Model capability in audit §13 and a recorder is a Sink — so RS-13a makes them namespaced Vocabulary verbs. The Session log and its replay are unchanged; only the Kernel's list of variants shrinks.
9. **Vision §29's list of event kinds reads as a Kernel registry.** RS-27 keeps in the Kernel only the kinds it emits itself or owns the policy for — of §29's own list, `PROCESSOR_DEADLINE_MISS` and, under §35's name, `DEVICE_LOST` — and gives the rest to the Vocabulary that emits them, because §5's three tiers would otherwise make a new radio event kind a Kernel change.
7. **The Kernel Action set of §5 and §19 has no command to start a receive stream**, so a fixed Spec cannot say "start RX at time T" as an Action. It is expressed instead as `Provider::start(at)` plus a recorder parameter (RS-14). Either that reading is accepted, or the Action set gains a member, which would be a Kernel change; this is recorded as an open question in `00-overview.md`.

## 10. Deferred

The payload schemas of the Radio Model's event kinds (committed by the Radio Model in Phase 2, RM-22). Fault injection beyond the three kinds of spec 08 SE-3 (`rx_overflow`, `rx_sequence_error`, `device_lost`), which Phase 2 carries with the acceptance tests §58 #5 and #6 (Phase 4). Subsampling and any export of counters (Sink Modules). Authentication beyond the adoption token (Vision §62). GraphEpoch (future work; RS-4's prohibition is in force now). The Python client surface, whose mapping RS-14 fixes (Phase 6). Migrations of the Manifest, until a version 2 exists.
