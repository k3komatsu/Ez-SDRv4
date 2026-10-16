# Phase 1 spec 04 — Run, Session, events and the Manifest

| Field | Value |
|---|---|
| Status | Accepted 2026-09-23 (Gate B; Phase 1 Step 5). Normative for `ezsdr-kernel::{run, session, event, policy, manifest, hash}`. Its amendments are listed in the closing Changes table. |
| Scope | The Run state machine and its cleanup algorithm; Session as a Run with an action log, and the admission every Action passes; the Lease; the closed Policy table and the registered event kinds; the event pipeline with its never-dropping counters; the Manifest envelope, its namespaced sections and the content hashing of both. |
| Not in scope | The Radio Model's event payloads (the Radio Model, spec 07; RM-22); fault injection beyond the three kinds of spec 08 SE-3 (`rx_overflow`, `rx_sequence_error`, `device_lost`), which spec 08 carries with the acceptance tests §58 #5 and #6; the Python client surface; GraphEpoch, which stays future work while its prohibition is in force. |
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
Termination      Completed | Stopped { cause: StopCause } | Failed { stage: Stage, reason: string }
StopCause        client | client_disconnect | lease_expiry | policy { event: EventKind } | abort { message }
                 every data-carrying enum here is tagged `kind` (00-overview OV-13)
Stage            validate | plan | prepare | arm | run

Target           Resource { resource: Ident, path } | Output { output: Ident } | Component { component: Ident }
                   tagged `kind`; resolved per role by spec 06 KC-23; `path` is a sub-path below
                   the resource's matched node in SB-1's segment grammar, empty for the node itself
SessionAction    SetParameter { target: Target, key: Key, value: Value }
                 | Vocabulary { ns: Namespace, verb: Ident, target: Target,
                                at: optional TimePoint, params: Map<Key, Value> }
                   at is a Kernel field, not a param: SB-4 caps a Value at one level of nesting
                   and a TimePoint is two, so a time could not travel inside params at all
                 | Stop { target: optional Target }
                 | Release | Adopt { token } | Renew | RunChild { spec_hash, binding_hash }
LogEntry         { seq: u32, time: TimePoint, action: SessionAction, outcome: Outcome }
Outcome          Admitted { coercions: [Coercion], warnings: [Warning], dispatched: [ActionId] }
                 | Rejected { violations: [Violation] }

Lease            { mode: Attached | Detached { ttl_ms: u64, renewable: bool },
                   token: optional, holder: optional, expires_at_host: optional, adoptions: u32 }
                   runtime state; the Manifest records only LeaseRecord
LeaseRecord      { mode, released: bool }
CleanupStep      freeze_dispatch | stop_providers | drain_and_stop_consumers | restore_baseline
                 | finalise_artifacts | flush_events | release_and_write_manifest     (RS-6)

Severity         debug | info | warning | error | fatal
EventKind        two or more dotted segments of [A-Za-z][A-Za-z0-9_]*; the Kernel registers the five
                 of RS-27 under `ezsdr.`, a Vocabulary the rest, under that owner's namespace
Reaction         continue | mark_artifact | stop | abort
Policy           { table: Map<EventKind, Reaction>, severities: Map<EventKind, Severity> }
EventSource      Node { node: ResourceId } | Output { output: Ident } | Island { island: IslandId }
                 | Kernel | Unforeseen            tagged `kind`; the instance a source root names is
                   given it by the Kernel (spec 05 PrepareContext.source, spec 06 KC-8)
Event            { source: EventSource, time: TimePoint, severity: Severity,
                   kind: EventKind, payload: Value }                   the control-path form
EventRecord      a fixed-size hot-path record: pre-resolved source and kind indices, a TimePoint,
                 a severity, and an inline payload of at most 32 bytes
CounterRow       { source: EventSource, kind: EventKind, count: u64 }
Manifest         see §7
ArtifactRef      { id: Ident, kind: Namespace, uri: string, hash: ContentHash,
                   size_bytes: u64, partial: bool, marks: [{ kind: EventKind, time: TimePoint }],
                   continuity: [ContinuityMap] }     in SampleClock order; see RS-40
ContentHash      "sha256:" followed by 64 lowercase hex digits
RunError         StructuralMutationForbidden | RunNotRunning | LeaseTtlRequired | LeaseNotRenewable
                 | AdoptRejected | UnknownEventKind { kind } | SectionNamespaceForbidden { ns }
                 | ReplayDivergence { field } | PayloadTooLarge
                 | EventSourceOutsideRoot { root, source } | ForeignEventHandle
                 | PayloadKeyNotAscii { position }                      RS-27, RS-32, RS-33
```

## 5. Normative rules

### The Run and its states

- **RS-1** Every execution is a Run, including an interactive Session (Vision §50). A Run has an id, a kind, an optional parent, a Lease, a compiled Policy and, for a Session, an action log. The Manifest records these as its `run`, `lease`, `policy` (absent when the Run failed before its Policy compiled, as `plan` is absent before `plan()`) and `action_log` fields.
  *Test:* `rs_01_run_id_is_generated_and_unique_within_the_process`, `rs_01_manifest_records_the_compiled_policy`, `ov_22_schema_freeze`.
- **RS-1a** The in-process Run object composes the same id, kind, parent, Lease, compiled Policy and optional Session action log.
  *Test:* `kc_45_manifest_fields` (spec 06 KC-5).
- **RS-2** The states are those of §4, and the only legal transitions are forward through `Created → Validated → Planned → Prepared → Armed → Running`, from any of those into `Stopping`, and from `Stopping` into `CleanedUp`. There is no `Failed` state: a failure is a `Termination`, reached through `Stopping`.
  *Test:* `rs_02_happy_path_transitions`; no `Failed` state (the eight states are frozen by `ov_22_schema_freeze`, which pins `$defs/RunState` in `manifest.v1.json` — a drift guard, not a behaviour assert).
- **RS-3** A failure at any stage moves the Run to `Stopping { abort }` with `Failed { stage, reason }`, and cleanup runs; `reason` is the failure's text, beginning with the rule that refused (spec 06 KC-7). A cancellation before `Running` moves it to `Stopping { orderly }` with `Stopped { client }`, performing only the steps that apply to the state reached.
  *Test:* `rs_03_failure_at_each_stage_reaches_cleanup`.
- **RS-4** The graph's structure does not change while a Run is `Running`. An attempt to add, remove or reconnect a component or a link is `StructuralMutationForbidden`. A structural change is `stop`, re-plan, `start`.
  *Test:* `rs_04_structural_mutation_forbidden` (the predicate `RunStateMachine::check_structural_mutation`).
- **RS-5** Every transition is recorded with its runtime `TimePoint` and the host UTC time, and the sequence appears in the Manifest.
  *Test:* `rs_02_happy_path_transitions`; spec 06's `kc_01_a_validate_failure_records_its_reason` (KC-43).

### Cleanup

- **RS-6** Cleanup is this ordered algorithm. Every step is attempted even when an earlier one failed; each failure is recorded and the sequence continues.

  ```text
  1. freeze_dispatch             freeze dispatch and cancel every pending burst and timer
  2. stop_providers              stop every Provider, in reverse dependency order
  3. drain_and_stop_consumers    drain the stepped instances, then stop every Executor and Sink, in reverse dependency order
  4. restore_baseline            clean up every instance, in reverse dependency order
  5. finalise_artifacts          drain the event path, then finalise the artifacts, marking as partial anything still open
  6. flush_events                drain the event path again and collect the counters
  7. release_and_write_manifest  release the Lease and write the Manifest
  ```

  Each step is named for what the coordinator does in it, and the Manifest records these names in `termination.cleanup_failures`. No step cleans up child Runs: a child is cleaned up inside `run_child` before it returns, so none is live at its parent's cleanup (spec 06 KC-37a). No step cancels Peripheral operations: there are no Peripherals, and a Peripheral model that needs a step adds one, which is a Kernel change either way.

  The coordinator performs the steps as follows. **1** refuse every further submission, empty every undelivered Action queue, recording the ids of the Actions it discards in `termination.undelivered` (RS-16), cancel every callback the coordinator scheduled. **2** `Provider::stop(mode)` on each Provider instance.

  **3** in an `orderly` cleanup of a Simulation Run, first run the stepping loop until `next_wakeup` returns `None`, at most `DRAIN_WAKEUP_CAP` times, so that each Provider delivers what its orderly `stop` publishes (MA-13) — ending the drain on a panic in `next_wakeup`, which ends the Run as spec 06 KC-30 states, counting consecutive results at one instant as KC-22 does, emitting `STEP_LIVELOCK` and stopping above `STEP_ROUND_CAP`, stopping at once when the end has escalated to `abort`, and stopping at its next wakeup once the `closing` flag is set, which every cleanup operation after the drain's own call sets on entry and the coordinator sets when `run_cleanup` returns, so that a drain RS-8a abandoned stops before the next step acts; a drain round also skips every instance whose step-4 `cleanup()` has been called; then `Executor::stop(mode)` on each Executor instance and `Sink::stop(mode)` on each Sink instance, keeping the artifacts each Sink returns, filed under that Sink's output (RS-38).

  **4** `cleanup()` on every instance that reached `prepare`. **5** drain the event path a last time, then record the marks of RS-30 on the artifacts (KC-42), so that a mark delivered by that drain — an event emitted after the last round's drain or while the instances stopped — reaches them too. **6** drain the event path again, so that the counters are taken next to a drain (RS-35), then snapshot them, and read every created link's `drops()`. **7** nothing inside `run_cleanup`: when it returns, the coordinator releases the Lease, assembles and seals the Manifest (KC-44), and only then drops the created links.

  In steps 3 and 4 a Module slot is taken with `try_lock`: a slot an abandoned earlier step still holds is not waited for — step 3's drain steps the other instances without it, and step 4 records a `CleanupFailure` for it ("KC-39: the instance is still held by an abandoned cleanup step") — so that one wedged Module costs one deadline and not one per later step. A radio Provider stops its transmit side before its receive side inside `stop` (RM-16), which is where RS-6's TX-before-RX order lives once the Kernel cannot tell them apart. Step 3's drain runs once, when the first fragment of step 3's reverse pass is performed, before that fragment's instance is acted on.

  In a device-paced class (HardwareInLoop or Hardware, spec 06 KC-2) step 3's drain is instead: stop and join the data thread (KC-46), if it exists; then, under `orderly`, one `step_until_quiescent` at the current instant over every Executor and Sink whose step-4 `cleanup()` has not been called — the Providers stopped in step 2, so what they published is in the links and the round is finite — and under `abort` none; then `Executor::stop` and `Sink::stop` as before.

  *Test:* `kg_03_step_3_runs_a_final_round_over_the_sinks`, `rs_06_cleanup_order_orderly` (the step names) and spec 06's `kc_30_a_next_wakeup_panic_in_the_cleanup_drain_is_recorded` (the drain's panic and its `abort` mode).

- **RS-7** Step 1 precedes step 2.
  *Test:* `rs_07_pending_burst_cancelled_before_tx_stop`.
- **RS-8** Steps 2, 3 and 4 run in reverse dependency order, the inverse of the arm order of SB-39. The device that was armed first is released last.
  *Test:* `rs_08_reverse_dependency_order`.
- **RS-8a** Every cleanup step runs under a deadline, the Provider's declared one where it has one and a Kernel default otherwise. A step that exceeds it, including one that never returns, is abandoned, recorded in `termination.cleanup_failures` as a timeout, and the sequence continues.
  *Test:* `rs_8a_wedged_cleanup_step_times_out` (a step that never returns is abandoned and recorded as a timeout, and the later steps still run; which deadline applied is not asserted); none named for a real Provider declaring a deadline (`CleanupOps::deadline_millis` is the Kernel's own hook, and the double returns a fixed 200 ms); the Kernel default is pinned indirectly by spec 06's `kc_44_a_wedged_provider_stop_does_not_block_other_cleanup`, whose wall-clock bound is twice `DEFAULT_CLEANUP_DEADLINE_MS`.
- **RS-9** An `abort` differs from an `orderly` stop only in the mode passed to each Provider: an orderly stop delivers each stream up to the stop instant, an abort only what it has delivered (MA-13). Both run the same seven steps. Under `orderly` cleanup step 3 drains the stepped instances before the Executors and Sinks stop; under `abort` it does not.
  *Test:* `rs_06_cleanup_order_abort` (every recorded mode is `Abort`; that no step is missing is not asserted there — the seven steps are `rs_06_cleanup_order_orderly`'s, which runs `Orderly`); the mode's effect on step 3's drain by `kg_03_step_3_runs_a_final_round_over_the_sinks` (an orderly final round over the Sinks) and `kg_03_an_abort_stops_the_data_thread` (no such round under an abort).
- **RS-10** An abort raised while an orderly stop is in progress escalates the remaining steps to abort mode and is recorded in the termination's `also` alongside the original cause, as are the later failures and Policy reactions the first end overrode (spec 06 KC-32).
  *Test:* `rs_10_abort_during_orderly_escalates`; spec 06's `kc_32_an_abort_during_orderly_escalates_and_is_recorded` (KC-40).
- **RS-11** `run_cleanup()` reaches `release_and_write_manifest` (step 7) regardless of mode, of how many fragments were prepared — including none, which is what a `validate` failure leaves — and of earlier steps failing or timing out.
  *Test:* `rs_11_cleanup_reaches_the_manifest_step_with_nothing_prepared`, `rs_06_cleanup_step_failure_continues`, `rs_8a_wedged_cleanup_step_times_out`.
- **RS-11a** The coordinator writes one Manifest at step 7 for every Run that reaches `CleanedUp`, including a failed Run, and seals it immutable afterwards.
  *Test:* `kc_44_a_wedged_step_does_not_prevent_the_manifest` (KC-44), `kc_01_a_validate_failure_still_writes_a_manifest` (KC-1).

### Session

- **RS-12** A Session is a Run whose ExperimentSpec is implicit and hashed like any other (Vision §3). It is built from the BindingProfile by spec 03's derivation, SB-22c and its table SB-T2: an Executor per Island executor name, one **output** per other binding carrying `feed`, one resource with empty `requires` per remaining binding whose Module holds Provider, and a Module holding several roles bound once per role (MA-1). `connect()` then runs the whole pipeline of SB-37 and leaves the Run `Running`, so the first Action has a runtime `TimePoint` to carry.
  *Test:* `rs_12_a_session_compiles_through_the_whole_pipeline`, `rs_12_a_multi_role_module_is_bound_once_per_role`, `rs_12_session_implicit_spec_hashed`; `connect()` leaving the Run `Running` by `rs_14_a_capture_reaches_the_output_it_names_with_two_sinks_bound`.
- **RS-13** The Session action envelope is the closed set of §4, and it is not the Kernel Action set. Each Session Action compiles to zero or more Kernel Actions, or to a Lease or Run operation.
  *Test:* RS-14's compilation tests, `rs_12_capture_reaches_the_recorder_it_names` and `rs_14_capture_compiles_to_a_timed_pending_update`; the Lease and Run operations by `rs_13_every_session_action_compiles_to_its_kernel_form`.
- **RS-13a** Only the lifecycle verbs are the Kernel's: `SetParameter`, generic over a namespaced key and value, plus `Stop`, `Release`, `Adopt`, `Renew` and `RunChild`. A domain verb is `Vocabulary { ns, verb, target, params }`, and the Vocabulary that registers the verb declares how it compiles. The envelope is closed; the verb list is not.
  *Test:* `rs_13a_unknown_vocabulary_verb_rejected`.
- **RS-14** The Kernel's own compilations are: `SetParameter` to a **class-pending update request** — the target, key, value and time of an `UpdateParameter`, without its class — which admission completes from the target's own declaration (RS-52; spec 06 KC-24 step 5), so compiling reads no class and refuses none; `Stop { target }` to a Kernel `Stop` addressed to that target and `Stop {}` to the Run's own stop; `Release`, `Adopt` and `Renew` to Lease operations; `RunChild` to the creation of a child Run from the Spec and the profile it names by hash (RS-25a). A `Vocabulary` action compiles as its registering Vocabulary declares: `radio.start_repeat` to a `TxBurst` carrying the repeat attribute (SC-26), and `sink.capture` to a **timed** class-pending update request addressed to the target the verb names, an `{ output }` — carrying the `at` RS-19 resolved, which is what makes Vision §61's `capture(n, at:)` a capture at that instant rather than at the next one. Every verb compiles to the target it names, and the Kernel picks no recorder of its own; a target its Spec does not declare in that role is refused by spec 06 KC-23. A verb whose namespace no loaded Vocabulary claims is rejected and logged. The **value** an `UpdateParameter` compilation carries is the action's own parameter under the rule's `key`; an action that carries none is refused (`ezsdr.vocabulary`). The Kernel supplies no default. An `UpdateParameter` `CompileRule` names the key and states no class, which is its target's declaration's alone; a rule that still carries `class` is refused when its Vocabulary descriptor is parsed, never reinterpreted (invariant 39). A `TxBurst` compilation carries the `late_policy` its `CompileRule` declares (SC-27); a rule declaring `RejectAtPlan` is refused at Vocabulary registration (MA-32), because a Session burst's target is resolved at `compile` and the Action never passes a plan stage. A Vocabulary wanting both behaviours declares two verbs.
  *Test:* `sb_49a_an_old_class_carrying_template_or_rule_is_refused`; the `RejectAtPlan` registration refusal by `rs_14_a_reject_at_plan_session_verb_is_refused_at_registration`; the no-value and no-default rules by `rs_14_the_kernel_supplies_no_vocabulary_value_or_late_policy`; the compilations by `rs_12_capture_reaches_the_recorder_it_names` and `rs_14_capture_compiles_to_a_timed_pending_update`; the Lease and Run-child compilations by `rs_13_every_session_action_compiles_to_its_kernel_form` and `kf_03_a_child_run_is_admitted_logged_run_and_recorded`.
- **RS-15** Every Session Action is appended to the log with its sequence number, its runtime `TimePoint`, and its outcome. Sequence numbers are dense: a **rejected** Action occupies one too. An Action is a well-formed document (SB-4, SB-9a): one whose value the canonicaliser cannot hash was never an Action, and the log refuses it before it takes a number. `SessionLog::check_entry(action)` performs exactly the check `append` performs, without appending; `append` calls it first. The coordinator calls it before compiling a Session Action (KC-28). A Module's `Abort` is neither admitted nor logged; it takes a real `ActionId` from the Run's counter, which `submit` returns and nothing else records (spec 06 KC-24).
  *Test:* `rs_15_log_sequence_is_dense`, `kc_28_a_malformed_action_takes_no_sequence_number`.
- **RS-16** Every Action passes `admit()` on the control path **before** anything is dispatched. A rejected Action is logged as rejected and never reaches the real-time path (Vision §3). An admitted Action is dispatched with its `ActionId` into its target's queue; one that RS-6 step 1 discards before its Module received it is recorded by id in `termination.undelivered`, so a reader tells an Action that reached its Module from one dropped at the end. The id of an undelivered Action a Module submitted is in `termination.undelivered` too, but in no action-log entry, since the log records Session Actions only.
  *Test:* `rs_16_an_action_the_freeze_discards_is_undelivered` (a Provider's queue), `rs_16_a_sink_action_the_freeze_discards_is_undelivered` (a Sink's), `rs_16_an_executor_action_the_freeze_discards_is_undelivered` (an Executor's, a Module's submission) and `rs_16_rejected_action_not_dispatched`; none named for a Module's id being absent from the log.
- **RS-17** Admission is **one check set** at every stage: the registered admission checks of SB-30 (`AdmissionCheckRegistry::run`) and the coercion policy of SB-45 (`coercion_policy` and `apply_coercion`). Each is one Kernel function, and each stage composes the two over what it proposes: `validate()` over a Spec's requested configuration, **reporting** every violation it finds (SB-38); `prepare()` over the applied configuration inside `collect_prepare`, **failing** the transaction (SB-41, SB-42); admission, through `Admitter::admit`, over one Action's key and value with the per-fragment configuration of SB-30, **refusing** before dispatch (SB-30's third point, RS-16, MA-14a). What is shared is the check set. The sites differ in what they propose **and in whether they report, fail or refuse**, and each of those three is required by another rule. At the runtime stage the order, the update class's place in it and the coerce of a control-path Provider update are spec 06's KC-24 (steps 5 and 8–13), and the coerce call is KC-26's. The Kernel does not coerce an Action a Module submits (MA-14a): the stepping loop holds every stepped Module while one runs, so the Kernel cannot call a stepped target's `coerce` for a Module's Action; MA-14 makes every target — including a hardware Provider, which is not stepped and not held (MA-15) — enforce its own envelope and report its Vocabulary's event, which keeps Mock and hardware alike (§59); MockRadio re-runs its `coerce` when an update applies (MR-18).
  *Test:* `rs_16_rejected_action_not_dispatched`, `rs_17_a_session_rate_change_is_coerced_by_its_provider`, `rs_17_a_session_change_beyond_a_joint_limit_is_refused`, `rs_17_a_scheduled_rate_change_under_reject_is_refused` and `rs_17_a_set_parameter_compiles_to_a_class_pending_update`; a Module's update by spec 06's `kc_24_a_module_update_is_not_coerced_by_the_kernel`.
- **RS-18** An Action that arrives while the Run is not `Running` is rejected and logged.
  *Test:* `rs_18_action_before_running_rejected` (the predicate `RunStateMachine::check_running`); spec 06's `kc_24_a_module_action_during_cleanup_is_refused`.
- **RS-19** A timed `Vocabulary` action carries its instant in the Kernel `at` field beside its `params`. An action with no `at` is admitted at the earliest instant its target allows: for a target that resolves to a Provider fragment, the coordinator's current instant in the Authority's primary root plus that instance's `min_command_lead` rescaled to the primary root with TM-9 and rounded up, or, when the target is a stream with a registered SampleClock that has not ended (TM-13a), the origin of its latest such clock — the one an Action admitted next lands in (SC-23) — converted to the primary root and rounded up, whichever is later (a Provider registers the clock a change starts when it takes the change, spec 07 RM-25, and the call that dispatched it returns after that, spec 06 KC-21 and KC-21a, so the next action sees it); for any other target, the current instant. The applied time is recorded in the entry as a coercion of the requested "as soon as possible". Either way the resolved instant reaches the compiled Action's own `at` (RS-49), or a class-pending request's (RS-14); a `SetParameter`, which has no `at` field to leave empty, compiles to a request with none and records no coercion, because nothing was requested to coerce.
  *Test:* `rs_19_capture_asap_records_applied_time`, `rs_19_vocabulary_action_carries_a_time` (a timed verb, so no coercion) and `rs_19_an_untimed_command_carries_its_resolved_time` (the untimed case records the resolution); the `SetParameter` case (no `at` field, so nothing was requested to coerce) by `rs_17_a_set_parameter_compiles_to_a_class_pending_update`; spec 06's `kc_28_an_untimed_burst_is_not_admitted_before_its_clock_s_origin`.
- **RS-20** Replaying a Session means re-applying its admitted entries against the same BindingProfile. A replay against a profile with a different hash reports `ReplayDivergence` rather than proceeding (Vision §3).
  *Test:* `rs_20_replay_divergence`.

### Lease

- **RS-21** A Lease is `Attached` or `Detached { ttl_ms, renewable }`, and the default is `Attached`. A `Detached` Lease without a TTL is `LeaseTtlRequired` (Vision §53).
  *Test:* `rs_21_lease_default_and_ttl_required`.
- **RS-22** The TTL runs on the host monotonic clock, injected so that tests can advance it, and the corresponding UTC time is recorded.
  *Test:* `rs_22_ttl_uses_the_host_clock`, `rs_23_detached_expires`; none named for the recorded UTC time.
- **RS-23** An `Attached` Lease ends the Run when its client disconnects, with `Stopped { client_disconnect }` and an orderly cleanup. A `Detached` Lease survives the disconnect until its TTL expires, and expiry runs the same cleanup with `Stopped { lease_expiry }`.
  *Test:* `rs_23_attached_stops_on_disconnect`, `rs_23_detached_expires`; spec 06's `kc_36_attached_disconnect_ends_the_run`, `kc_36_detached_lease_expiry_ends_the_run`.
- **RS-24** A Detached Lease is granted with a token, and only `Adopt { token }` reclaims it. A wrong token is `AdoptRejected`; a `Renew` on a non-renewable Lease is `LeaseNotRenewable`. Both outcomes are logged.
  *Test:* `rs_24_adopt_requires_the_token`; none named for the two outcomes being logged.
- **RS-25** A child Run inherits its parent's Lease and may not declare its own. A child is cleaned up inside `run_child` before it returns (spec 06 KC-37a), so no child outlives its parent's cleanup, and a Detached parent keeps its children until its TTL expires (Vision §53).
  *Test:* `kf_03_a_child_inherits_the_lease` (KC-37a).
- **RS-25a** A child Run is a Run like any other and has its own profile, which `RunChild` names by hash beside the Spec's (RS-14). The child's profile binds each of its resources to an instance its parent binds, under an equal binding description (SB-3), and names a binding with the same description as its parent's `authority` (RS-25). The child's profile also carries, with an equal value, every `environment` section of its parent's that an admission check registered for the parent reads at the runtime stage (SB-29, SB-30): a child may add a section and may not drop or change one that judges what the Session may do (Vision §52); a section only validate- or prepare-stage checks read — the simulated channel, seed and faults — may differ.
  *Test:* `kf_03_rs_25a_refusals` (KC-37a).

### Policy and events

- **RS-26** A Policy maps a registered `EventKind` to one of exactly four reactions: `continue`, `mark_artifact`, `stop`, `abort`. It is a table. There is no expression language and no rules engine; logic that needs conditions belongs in a Reactor or in client orchestration (Vision §53).
  *Test:* `rs_26_policy_override_from_spec`, `rs_28_policy_defaults_table`.
- **RS-27** Every event kind is namespaced: a kind is two or more `.`-separated segments (SB-1), and the Kernel's own kinds are under `ezsdr.` — `ezsdr.EVENTS_DROPPED`, `ezsdr.LINK_BACKPRESSURE`, `ezsdr.PROCESSOR_DEADLINE_MISS`, `ezsdr.DEVICE_LOST`, `ezsdr.STEP_LIVELOCK`, and RS-33's fallback kind `ezsdr.unforeseen`, so no consumer tells a Kernel kind by the absence of a dot. Prose in these specifications writes a Kernel kind by its last segment. The Kernel registers only the kinds it emits itself or owns the policy for: `EVENTS_DROPPED` from its own drain (RS-35), `LINK_BACKPRESSURE` from a DataLink policy (SC-20a), `PROCESSOR_DEADLINE_MISS` from a `RelativeBudget` it defines (TM-15), `DEVICE_LOST`, which Vision §35 states in as many words is a Kernel policy and which the Module boundary reports as a typed error, and `STEP_LIVELOCK` from its own stepping loop (MA-30). Every other kind is registered by the Vocabulary or Module that emits it, under that owner's namespace: the Radio Model registers `RX_OVERFLOW`, `TX_UNDERFLOW`, `TX_DISCONTINUITY`, `LATE_COMMAND`, `TIME_ERROR`, `ALIGNMENT_ERROR` and `CLOCK_LOST`; the Peripheral model registers `PERIPHERAL_TIMEOUT` and `PLUGIN_FAILURE`; Calibration registers `CALIBRATION_INVALID`; Host I/O registers `TUN_QUEUE_DROP`. An `EventHandle`'s fields are private and `resolve` is its only constructor, so no Module fabricates one. What is still refused, with its own error rather than a panic, is a handle **another Run or another view** minted (`ForeignEventHandle`): every view and every Run's collector mints handles under a scope id of its own, unique in the process and never reused, the Kernel's own sink keeping one too, and `emit` checks the handle's scope beside its index in constant time (RS-32, MA-9). A **second** declaration of one kind is refused rather than overwriting the first, so one Vocabulary cannot replace another's default reaction and severity; the refusal is its own error and not "the kind is not registered".
  *Test:* `rs_27_a_second_declaration_of_one_kind_is_refused`, `rs_28_policy_defaults_table`, `rs_32_another_view_s_event_handle_is_refused`, `rs_32_another_run_s_event_handle_is_refused`.
- **RS-28** A kind's default reaction is declared where the kind is registered. The Kernel's five are: `EVENTS_DROPPED` and `LINK_BACKPRESSURE` continue and mark the artifact, `PROCESSOR_DEADLINE_MISS` marks the artifact, and `DEVICE_LOST` and `STEP_LIVELOCK` abort, both at `fatal` severity.
  *Test:* `rs_28_policy_defaults_table`; none named for the declared `fatal` severity of `DEVICE_LOST` and `STEP_LIVELOCK` (the test reads the reactions and the five registered names only).
- **RS-29** A kind with no entry in the Run's Policy takes a default from its severity: `debug` and `info` continue, `warning` and `error` mark the artifact, `fatal` aborts.
  *Test:* `rs_29_unknown_kind_by_severity`.
- **RS-30** `mark_artifact` records the kind and time against every artifact open at that moment, and those marks appear in the `ArtifactRef`.
  *Test:* `rs_30_mark_artifact_marks_open_artifacts` (one artifact), `rs_30_marks_every_open_artifact_including_a_partial_one` (two open at once, one of them partial) and spec 06's `kc_42_a_mark_delivered_during_an_orderly_cleanup_reaches_the_artifact`, `kc_42_a_mark_delivered_during_an_abort_cleanup_reaches_the_artifact` (the event path).
- **RS-31** An Event carries a source, an `EventSource` naming the instance by its role (§4), a `TimePoint` in a named domain, a severity, a kind and a schema-versioned payload (Vision §29).
  *Test:* `ov_22_schema_freeze` for the shape; spec 07's `rm_20_schema_freeze`, `rm_22_payloads_round_trip` (RM-22).
- **RS-32a** *Withdrawn.* The Kernel does not interpret a Vocabulary's hot-path byte layout; a Vocabulary that needs one defines and decodes its own representation.
- **RS-32** Emitting on the hot path allocates nothing. The hot-path record is fixed in size, carries pre-resolved indices for its source and kind rather than strings, and holds at most 32 bytes of payload inline. A larger payload is produced on the control path only; attempting one on the hot path is `PayloadTooLarge`. A Module emits through the root-bound view of the collector its context carries (spec 05 MA-5, spec 06 KC-8). The view's `resolve` is fallible: it refuses, with `EventSourceOutsideRoot { root, source }`, a source outside the instance's source root and `kernel` or `unforeseen` named by a Module — a Provider's root admits its node and every node below it on a segment boundary (SB-33), a Sink's exactly its output, an Executor's exactly its Island. `emit_control` is bound to the same root, and refuses a payload carrying a non-ASCII map key at any depth, before the event is counted or delivered, with `PayloadKeyNotAscii { position }`, which names the position and never the key (SB-9a).
  *Test:* `rs_32_emit_allocates_nothing`, `rs_32_a_view_refuses_a_source_outside_its_root`, `rs_32_a_view_refuses_another_instance_s_source`, `rs_32_a_control_event_from_outside_the_root_is_refused`, `rs_32_a_control_event_with_a_non_ascii_key_is_refused`.
- **RS-33** Every `(source, kind)` pair reachable in the plan has a never-dropping counter, and the table is sized at `prepare` from the plan, plus one fallback row, `(unforeseen, ezsdr.unforeseen)`, for pairs that were not foreseen, and the row `(kernel, EVENTS_DROPPED)` of RS-35's meta-event. The counter is incremented before the body is queued, so a count is never lost even when the body is. The fallback row is for an unforeseen pair **inside** the instance's own tree: a view refuses a source outside its root before the fallback is consulted (RS-32), so a Module naming `unforeseen` is refused and not folded into the row.
  *Test:* `rs_33_unforeseen_pair_uses_the_fallback_row`, `rs_33_counters_exact_under_drop`, `rs_33_drain_uses_the_planned_kernel_source_for_meta_events`, `rs_33_an_unforeseen_pair_inside_the_root_uses_the_fallback_row`, `rs_33_a_view_refuses_kernel_and_unforeseen`.
- **RS-34** Event bodies travel through a bounded ring. When the ring is full the body is dropped and a per-kind drop count is incremented, never subsampled.
  *Test:* `rs_35_dropped_events_never_enter_the_ring`, `rs_33_counters_exact_under_drop` (the ring's drop and its counters); none named for subsampling.
- **RS-35** Each time the ring is drained, one `EVENTS_DROPPED { kind, count }` event is emitted per kind that dropped since the last drain, carrying the delta rather than a running total, with source `kernel` and the drain's instant as its time (`EventCollector::drain(at)`; the coordinator passes its current instant in the primary root). These are produced on the control path at drain time and never enter the ring. For every kind, the counter equals the delivered bodies plus the sum of the `EVENTS_DROPPED` counts.
  *Test:* `rs_35_dropped_counts_are_deltas`, `rs_35_dropped_events_never_enter_the_ring`, `rs_33_counters_exact_under_drop`.
- **RS-36** A kind whose Policy reaction is `stop` or `abort` sets a per-kind escalation flag when the hot path drops its body (the ring full), which the control path acts on although the body never arrives. A body that is queued — on the hot path, or on the control path, which never drops one — raises no flag: it is reacted to when it is delivered (KC-31), so that a flag raised before its body arrives cannot name the Run's cause ahead of an event delivered first.
  *Test:* `rs_36_abort_survives_a_drop`, `rs_36_a_dropped_stopping_body_ends_the_run`, `rs_36_a_delivered_body_raises_no_escalation_flag`; none named for the escalation flag of a `stop`-reacting kind.
- **RS-37** *Withdrawn.* Vision §29's prohibition on a metrics framework stands as prose, and `00-overview.md` OV-23a's banned-token list is what would catch an exporter in the Kernel.

### The Kernel Action set

- **RS-48** The Kernel Action set is closed: `TxBurst`, `SetTimer`, `UpdateParameter`, `Command`, `Emit`, `Stop`, `Abort` (Vision §5, §19). Adding a member is a Kernel major.
  *Test:* `rs_48_action_set_is_closed_and_schematised`.
- **RS-49** An Action is a document with a schema (`00-overview.md` OV-10). Each carries its own fields, and all but `Abort` name a `Target` (§4), resolved per role by spec 06 KC-23, which `Stop` may leave unset (RS-50):

  ```text
  TxBurst           { target, waveform: ArtifactRef, repeat: bool,
                      at: AbsoluteDeadline, requested_at: optional AbsoluteDeadline,
                      late_policy: LatePolicy, metadata: Map<Key, Value> }
  SetTimer          { target, at: AbsoluteDeadline, token }
  UpdateParameter   { target, key: Key, value: Value, class: UpdateClass,
                      at: optional AbsoluteDeadline }
  Command           { target, verb: Ident, params: Map<Key, Value>, at: optional AbsoluteDeadline }
  Emit              { target, event: Event }
  Stop              { target: optional Target }
  Abort             { cause }
  ```

  `Command` carries a Vocabulary verb to any target: `radio.start_rx` on a receive stream is one (RM-12), an operation on a Peripheral Port another.

  `UpdateParameter.at` is the instant at or after which the update takes effect under its class; absent means the first instant the class permits. It is optional for **every** class, `hardware_timed` included: a class cannot make the field mandatory.

  Every timed Action names its instant with an `AbsoluteDeadline`, which Vision §19 defines for this: "a TxBurst or Command target in a device ClockDomain".

  `TxBurst` carries no channel list; which channels a burst uses is the Radio Model's (spec 07 RM-13).

  *Test:* `rs_48_action_set_is_closed_and_schematised`, `rs_49a_a_scheduled_update_resolves_to_a_class_pending_request`, `kc_24_component_update_classes_belong_to_the_target` (an `UpdateParameter` with no `at` whose class is `hardware_timed`).
- **RS-49a** An Action that a Spec schedules is written as an **ActionTemplate**: the Action without its time field. A Spec cannot name a `ClockDomainId`, because domains are allocated at `prepare` (SB-16, TM-13a), and every timed Action names one through its `AbsoluteDeadline`. `arm` resolves the entry's `SpecTime` and substitutes it, producing the Action. The `UpdateParameter` template states no class either, since the class is its target's declaration's alone (RS-52): `arm` resolves it to a **class-pending update request**, which spec 06 KC-24 step 5 completes, and a template that still carries `class` is refused at parse, never reinterpreted (invariant 39). The `UpdateParameter` template is timed too: a Spec's schedule entry carries a mandatory `SpecTime` (SB-16), so a scheduled parameter change takes effect at the instant the Spec named rather than at `arm`. An Action emitted at run time by a Reactor carries its own time and is never a template.
  *Test:* `rs_49a_a_scheduled_update_resolves_to_a_class_pending_request`, `rs_49a_a_timed_template_resolves_with_its_deadline`, `sb_49a_an_old_class_carrying_template_or_rule_is_refused`.
- **RS-50** `Stop` carries an optional target: with one it stops that resource, without one it stops the Run. RS-14 compiles a Session `Stop` to one or the other.
  *Test:* `rs_50_stop_with_and_without_a_target`.
- **RS-51** `TxBurst.at` names an instant in the transmit stream's SampleClock and is subject to SC-23a, which may advance its inner `TimePoint` to the next sample instant and record the original in `requested_at`. The same pair reaches the Manifest through the `BurstRecord` of SC-28, where it is spelt `target` and `requested_target`. Its `late_policy` is SC-27's, and `RejectAtPlan` is legal only on a burst whose target is statically known.
  *Test:* none named here; the rules it cites are SC-23a's, SC-27's and SC-28's, in spec 02.
- **RS-52** `UpdateParameter.class` is the parameter's declared update class (Vision §27), and the target's own declaration is its only source: a `{resource}` or `{output}` target's key's `KeyDecl.update_class`, a `{component}` target's `ParamDecl.update_class`, which is optional, absent meaning the parameter is fixed for the Run. A component's missing or fixed declaration is never resolved through a Vocabulary's (`ExperimentSpec::declared_class`). A Session's and the schedule's updates state no class and take it at admission (RS-14, RS-49a); a Module's finished Action states one, which must equal the declaration's. Either way an update whose target declares no class is refused at admission (spec 06 KC-24 step 5) and never reaches a Module; a scheduled one is refused earlier, by `validate()` (SB-16).
  *Test:* `kc_24_component_update_classes_belong_to_the_target`, `kc_24_a_module_update_must_state_its_providers_declared_class`, `rs_17_a_set_parameter_compiles_to_a_class_pending_update` and `rs_52_a_scheduled_update_states_no_class`.

### The Manifest

- **RS-38** The Manifest is a Kernel envelope with namespaced Module sections. The Kernel writes the envelope; each Module writes its own section; no Provider-specific field ever requires a Kernel change (Vision §50). The envelope carries the mandatory `version` Vision §10 requires of it, as the Spec and the BindingProfile do.

  ```text
  Manifest
  ├── version        1                    mandatory; SB-47 applies
  ├── run            { id, kind, parent, execution_class (absent without a plan), fidelity, transitions, deterministic, children }
  ├── policy         Policy, absent when the Run failed before its Policy compiled (RS-1)
  ├── spec           { hash, body, source }   hash and body as parsed (RS-45); source as supplied
  ├── binding        { hash, body including environment }
  ├── plan           { summary, placement as bound }
  ├── prepare        { reports: { <fragment>: PrepareReport } }   filed under the fragment invoked; plan order is `plan`'s
  ├── admission      { matched, needs, rejected, violations, coercions_preview, warnings }
  ├── modules        [{ id, version, impl_hash, profile }] and the Vocabulary versions in use
  ├── components     Map<Ident, impl_hash>
  ├── inputs         [ArtifactRef]        artifacts the Run consumed, by reference
  ├── clocks         { domains, relations including epoch to UTC with uncertainty, sample_clocks }
  ├── links          [{ link, drops }]    one per created DataLink, its endpoints in `plan.links`
  ├── events         { counters (complete, including zero rows), delivered }
  ├── lease          { mode, released }   no token and no host monotonic deadline
  ├── action_log     Session only
  ├── termination    { reason, at, host_utc_nanos, cleanup_failures, also: [Termination],
  │                    undelivered: [ActionId] }   RS-16
  ├── artifacts      { <output>: [ArtifactRef] }   with content hashes and continuity, under the output
  │                    whose Sink returned them; an id is the Sink's choice, unique by no rule
  └── sections       [{ source: EventSource, name: Namespace, content: opaque }]   Module sections, each filed under
                     the source root of the instance that wrote it, including `ezsdr.capture` when the profile asks; the Kernel writes none
  ```

  `spec.source` is the hash of the code that generated the Spec — Vision §9's client-side builder — supplied by the caller that starts the Run as `Assembly.spec_source` (spec 06 §4) and recorded as given; absent when the caller has none. A Session records the caller's value the same way, as opaque provenance, although its Spec is the implicit one (RS-12). The Kernel never reads it, and it is outside `spec.hash`, so two builders that produce one Spec give one Spec hash (RS-45).

  *Test:* `rs_38_spec_source_is_recorded_outside_the_spec_hash`, `rs_38_environment_recorded_verbatim`, `rs_38_counters_include_zero_rows`, `rs_38_manifest_carries_its_mandatory_version`, `kc_45_manifest_fields`, `sb_41_a_report_is_filed_under_the_fragment_invoked`.
- **RS-39** A Module writes only under its own registered namespace; a write elsewhere is `SectionNamespaceForbidden`. The transmit burst records of SC-28 and a Provider's envelope go in that Module's section, not in the envelope. The content's object keys must be ASCII. A Module's own namespace is its `ModuleId` read as a `Namespace` (SB-T0); a Module whose id is not one writes no section, and a section it hands in is refused and recorded as a cleanup failure of step 7. A Provider hands its sections to the Manifest through `ProviderInstance.sections`: the coordinator reads `instance().sections` of every Provider instance after `stop` and writes each entry with `Manifest::write_section(owner, source, …)` (KC-44). Each section is filed under the source root the Kernel gave the writing instance (spec 06 KC-8), so two instances of one Module that write one section name both keep it, and no Module puts its own id in a section name.
  *Test:* `rs_39_section_namespace_enforced`, `rs_39_a_module_section_with_a_non_ascii_key_is_refused`; an instance whose `ModuleId` is not a `Namespace` by `ka_13_an_invalid_provider_namespace_without_sections_is_not_a_failure` (an instance with no sections is no failure); none named for such an instance handing in a section (refused, and recorded as a step-7 cleanup failure — the other branch, a section outside the writer's namespace, is spec 06's `kc_45_a_provider_section_outside_its_namespace_is_a_cleanup_failure`); spec 06's `kc_45_two_instances_writing_one_section_name_both_keep_it`.
- **RS-40** A capture's continuity metadata is the sequence of `ContinuityMap`s derived by SC-30, in SampleClock order, hanging off its `ArtifactRef`. There is more than one whenever the capture spans a rate change or a channel-count change (SC-30a): each ends one map and starts the next. The Kernel never assembles a map by hand (Vision §28).
  *Test:* `rs_40_capture_across_a_rate_change` (two maps assigned by hand); a capture actually spanning a change by spec 10's `hd_10_a_capture_spans_a_gap_and_a_clock_change` and spec 16's `test_ea_17_a_capture_across_a_rate_change_is_refused`; the channel-count change of SC-30a by `hd_15_a_capture_across_a_clock_change_has_no_metadata` (its 1-to-2-channel case gives two maps on one clock); spec 02's SC-30 tests.
- **RS-41** The fidelity vector is the weakest value per aspect over the bound Providers' declarations (spec 05). A Run in which no Provider declares an aspect records `none` for it.
  *Test:* `rs_41_fidelity_is_the_weakest`.
- **RS-42** The ExecutionClass is fixed at binding resolution and never changes during the Run. `deterministic` may be claimed only for the Simulation class with a recorded seed; RealtimeEmulation, HardwareInLoop and Hardware are never deterministic (Vision §14).
  *Test:* `rs_42_determinism_only_in_simulation`; none named for the recorded seed the Simulation class's determinism claim requires.
- **RS-43** Neither random seeds nor an environment capture is an envelope field. The Kernel owns no random number generator. A seed is something the environment declared, and the environment is already recorded under `binding.body` (RS-38). The environment capture belongs under `sections` as `ezsdr.capture`, written by the Module that produces it.
  *Test:* `rs_43_no_seeds_field_in_the_envelope`; the envelope carrying no capture field by `ov_22_schema_freeze` (the frozen `manifest.v1.json` forbids additional properties and declares no `capture` — a drift guard, not a behaviour assert); none named for the environment capture (`sections` under `ezsdr.capture`).
- **RS-44** Large data is always by reference. An `ArtifactRef` carries a locator, a content hash and a size, never the bytes (Vision §50, §51).
  *Test:* none named.
- **RS-44a** A waveform supplied by a client is ingested as an input artifact before the Action that references it is admitted: its bytes are stored, hashed by RS-45 and listed in `inputs`, and the Action carries the resulting `ArtifactRef`. An Action whose `ArtifactRef` resolves to nothing is rejected. For a `TxBurst` of any origin the check is admission's: a waveform whose hash names no bytes in the Run's input store, or bytes of another length than its `size_bytes`, is refused with the check `ezsdr.input` (KC-24). The stored bytes are what a Module reads through `PrepareContext.inputs` (spec 05 MA-5a).
  *Test:* `ke_02_a_module_burst_must_name_a_run_input`, `rs_44a_a_burst_without_a_waveform_is_refused`.
- **RS-45** Everything hashable is content-addressed: the Spec, the BindingProfile including its environment, each artifact's bytes, each component's declared implementation hash. A document is hashed, and recorded as the Manifest's `body`, as parsed and re-serialised canonically from the parsed document, never as submitted, so one document has one hash whatever optional members its author spelled; an opaque value keeps its content, not its spelling. A `RunChild` entry names its documents by these hashes, and a document that does not parse by the hash of its text as sent (KC-37a). Two Runs with equal Spec, binding, component and input hashes are comparable by construction.
  *Test:* `rs_45_hash_equal_for_equal_inputs` (a Spec Run and a Session), `kf_03_a_child_run_is_admitted_logged_run_and_recorded`, `kf_03_rs_25a_refusals` (a document that does not parse).
- **RS-46** The Manifest's own hash is computed over the Manifest with that field removed and is stored beside it, never inside the hashed body.
  *Test:* `rs_46_manifest_hash_is_stored_beside_the_body`, which recomputes the hash from the serialised body with `hash` removed, confirms an absent hash is skipped rather than written as `null`, and confirms re-sealing is stable.
- **RS-47** The canonical form and the digest are those of `00-overview.md` OV-14 to OV-17. This document adds no rule about them and restates none (OV-4).
  *Test:* none named here; the canonical form and the digest are `00-overview.md`'s (OV-14…OV-17, checked there).

## 6. Decisions

| # | Decision | Choice | Rejected (one line each) | Ceiling / upgrade path |
|---|---|---|---|---|
| R1 | State machine | Eight states, no `Failed`; failure is a termination reached through `Stopping` (RS-2) | A `Failed` state (a failed-but-uncleaned Run is a live transmitter); separate `Stopping` and `CleaningUp` states (splits one algorithm across two) | Only two cleanup modes; per-step timeouts are Provider-declared, not Kernel parameters |
| R2 | Cleanup order | RS-6, with the dispatch freeze first and reverse dependency order for the device steps | Vision §53's bullet order verbatim (a queued burst reopens TX after it was stopped); best-effort with no order (Mock and hardware would diverge) | Steps are best-effort with no retry policy; a failing step is recorded and the rest still run |
| R3 | Session log model | Two levels with a closed *envelope* and an open verb list: the Kernel owns the lifecycle variants and `SetParameter`, while a domain verb is `Vocabulary { ns, verb, target, at, params }` whose compilation its Vocabulary declares (RS-13, RS-13a, RS-14) | One set shared with the Kernel Actions (a recorder request has no real-time meaning, and adding one bloats the Reactor's vocabulary); a closed verb list, which was the first draft and would have needed a Kernel variant for the first peripheral sweep; an open *envelope* (then nothing could be replayed — it is the envelope that must be closed, not the verbs) | A verb whose namespace no loaded Vocabulary claims is rejected; a capture names the output it records to, and a target the Spec does not declare is rejected rather than silently buffered on the host |
| R4 | Lease clock | Host monotonic, injectable; UTC recorded (RS-22) | Run time (a paused simulation never expires; a fast one expires early); wall clock for expiry (an NTP step would end a Run) | none |
| R5 | Adoption | A token issued at grant (RS-24) | Adoption by run id (any client could seize a detached transmitter); full authentication now (Vision §62 stages it later) | The token is a shared secret on the client channel; an authentication layer later needs no change to the Lease type |
| R6 | Unknown event kinds | A severity-derived default (RS-29) | A flat `continue` (a Module's fatal event would be ignored); a flat `mark_artifact` (an informational event would taint a capture); a closed Kernel enum of kinds (a Module kind would need a Kernel change) | Keyed by kind alone; a later `(source, kind)` key is additive, since kind-only entries stay valid |
| R7 | Escalation under drop | A per-kind atomic flag on the hot path (RS-36) | Relying on the ring (a dropped `DEVICE_LOST` would never abort) | none |
| R8 | Event record | Fixed size, pre-resolved indices, 32 bytes inline, larger payloads on the control path (RS-32) | Heap strings on the hot path (allocation); one unbounded channel (it blocks or grows without bound) | `EventRecord` is in-process by `00-overview.md` X8, so raising the inline limit is a recompile, not a schema change |
| R9 | Counter table | Sized at `prepare` from the plan, plus one fallback row (RS-33) | A hash map on the hot path (allocation and locking); a fallback row per source (unbounded when a Module mislabels its source) | Unforeseen pairs merge into one row, which is reported as such |
| R10 | Manifest shape | A closed Kernel envelope with namespaced Module sections and the mandatory `version` Vision §10 requires of it; one `inputs` list for waveforms and calibration artifacts (RS-38) | Separate lists of the same shape; a free-form Manifest (two Runs would not be comparable) | Sections are opaque; a query across Modules is a Sink's job |

## 7. Tests

Fixtures: the test-double Provider of `00-overview.md` OV-21 with a recording call log and an injectable failure phase; the in-memory DataLink; the recording TestExecutor; an injectable host clock; a test event kind `test.custom`.

| test | input | expected | rules |
|---|---|---|---|
| `rs_02_happy_path_transitions` | a valid Spec and profile | exactly the eight states in order, all recorded | RS-2, RS-5 |
| `rs_03_failure_at_each_stage_reaches_cleanup` | an injected failure at validate, plan, prepare and arm in turn | `Failed { stage, reason }` each time and `CleanedUp`; a cancellation from each state before `Running` ends `Stopped { client }` after an orderly stop | RS-3 |
| `rs_11_cleanup_reaches_the_manifest_step_with_nothing_prepared` | `run_cleanup` with no fragments, in `Orderly` and `Abort` | step 7 reached, and last | RS-11 |
| `rs_12_a_session_compiles_through_the_whole_pipeline` (`spec_binding.rs`) | a Session profile with a Provider and a fed recorder | the implicit Spec validates and plans; the Sink is its own fragment; the feed is in the plan's links | RS-12, SB-25 |
| `rs_12_a_multi_role_module_is_bound_once_per_role` (`spec_binding.rs`) | a Provider+Sink Module bound as `radio` and, with `feed`, as `rec`; then once with `feed` | a resource and an output, planned; bound once, a Sink only | RS-12, MA-1 |
| `rs_01_run_id_is_generated_and_unique_within_the_process` | repeated `RunId::generate` in one process | the documented form, all distinct | RS-1 |
| `rs_01_manifest_records_the_compiled_policy` | a compiled Policy with one override written into a Manifest | read back typed and equal; the override answers | RS-1 |
| `rs_06_cleanup_order_orderly` | a Running Run stopped by its client | the call log is exactly the seven steps of RS-6, which serialise as `freeze_dispatch` … `release_and_write_manifest` | RS-6, RS-8 |
| `rs_06_cleanup_order_abort` | `run_cleanup` in `Abort`, with the recording double | every recorded mode is `Abort`; that no step is missing is not asserted here (the seven steps are `rs_06_cleanup_order_orderly`'s, which runs `Orderly`) | RS-6, RS-9 |
| `rs_06_cleanup_step_failure_continues` | `drain_and_stop_consumers` returning an error | later steps still called; the failure is in `termination.cleanup_failures` | RS-6 |
| `rs_07_pending_burst_cancelled_before_tx_stop` | `run_cleanup` in `Orderly`, with the recording double | the dispatch-freeze entry precedes the stop-providers entry (no burst is scheduled, so none is cancelled: none named for a burst's cancellation) | RS-7 |
| `rs_08_reverse_dependency_order` | three fragments with `b after a`, `c after b` | released `c`, `b`, `a` | RS-8, SB-39 |
| `rs_10_abort_during_orderly_escalates` | an abort raised while stopping | the remaining steps switch mode, `Orderly` first and `Abort` last (both causes recorded is spec 06's `kc_32_an_abort_during_orderly_escalates_and_is_recorded`) | RS-10 |
| `rs_04_structural_mutation_forbidden` | a Run in `Running` | the predicate `RunStateMachine::check_structural_mutation` returns `StructuralMutationForbidden` (no link is added; the refusal of a real link change is the coordinator's, in Phase 2) | RS-4 |
| `rs_12_session_implicit_spec_hashed` | `connect()` with one binding | an implicit Spec with empty `requires` and a hash | RS-12 |
| `rs_16_rejected_action_not_dispatched` | a `SetParameter` violating the `test.limits` check | the `test.limits` check refuses it (`admit` returns `Err`) and the entry is logged `Rejected`; none named for the Provider queue staying empty | RS-16, RS-17, SB-30 |
| `rs_16_an_action_the_freeze_discards_is_undelivered` (`device_paced.rs`) | a device-paced Session whose Provider never calls `recv()` and loses its device 100 ms after the start, one `SetParameter` | logged `Admitted` with one id; `termination.undelivered` holds that id | RS-16, RS-6, KC-21a |
| `rs_16_a_sink_action_the_freeze_discards_is_undelivered` (`device_paced.rs`) | the same Session with a recorder that never calls `recv()`, one recorder verb (`Command`) to its output | logged `Admitted` with one id; `termination.undelivered` holds that id; the recorder took nothing | RS-16, RS-6 |
| `rs_16_an_executor_action_the_freeze_discards_is_undelivered` (`coordinator.rs`) | a Spec Run whose Executor never calls `recv()` submits a `Command` to its component `c1`, then an `Abort` | the `Command`'s id, the first `submit` returned, is in `termination.undelivered`; the Executor took nothing | RS-16, RS-6 |
| `sb_41_a_report_is_filed_under_the_fragment_invoked` (`coordinator.rs`) | two Sinks `rec` and `rec2` returning one and two artifacts | `artifacts` is `{ rec: [rec_0], rec2: [rec2_0, rec2_1] }` | RS-38, RS-6 |
| `kc_24_component_update_classes_belong_to_the_target` (`coordinator.rs`) | Module updates to two components declaring one key under different classes; to a component whose `ParamDecl` is fixed; to one that declares none | each component's own class is the one admitted; the fixed and the missing declarations are refused at admission, never falling back to `test.gain`'s `KeyDecl` | RS-17, RS-52, KC-24 |
| `rs_18_action_before_running_rejected` | the state machine in `Armed`, no Action submitted | `RunStateMachine::check_running` returns `RunNotRunning`; a rejected entry is still logged (the refusal is the predicate's, not an Action's) | RS-18 |
| `rs_12_capture_reaches_the_recorder_it_names` (`src/session.rs`) | a Spec with a recorder output, then a capture naming that output | it compiles to exactly one class-pending update, addressed to the output, carrying the verb's key, value and `at`; the class is not read | RS-12, RS-14 |
| `rs_14_capture_compiles_to_a_timed_pending_update` (`src/session.rs`) | a capture verb with an `at` | its first compiled item is a class-pending update timed at that instant, with no coercion | RS-14, RS-19 |
| `rs_14_a_capture_reaches_the_output_it_names_with_two_sinks_bound` (`coordinator.rs`) | a Session with two recorders bound; a capture to the second | only the second Sink receives it | RS-14, KC-23 |
| `rs_13a_unknown_vocabulary_verb_rejected` | a `Vocabulary` action whose namespace no loaded Vocabulary claims | refused at compile, the reason containing `radio.start_repeat` (the check name is not asserted); none named for the log entry | RS-13a, RS-14 |
| `rs_48_action_set_is_closed_and_schematised` | each of the seven Kernel Actions | each round-trips through its schema; the set has no eighth member | RS-48, RS-49 |
| `rs_14_the_kernel_supplies_no_vocabulary_value_or_late_policy` (`src/session.rs`) | a `capture` whose action carries no value; a `start_repeat` | the first is refused, the second takes the policy the verb declares | RS-14, OV-21, SC-27 |
| `rs_14_a_reject_at_plan_session_verb_is_refused_at_registration` (`run_session.rs`) | a Vocabulary registering a Session `start_repeat` with `TxBurst { late_policy: RejectAtPlan }` | refused at registration, naming RS-14 | RS-14, SC-27 |
| `rs_52_a_scheduled_update_states_no_class` (`spec_binding.rs`) | schedule entries updating `test.gain`; `test.count`, whose `KeyDecl` declares no class; `test.gain` with a `Bool` | the first validates; the second is refused by `validate()` with RS-52's reason; the third is `KeyShape` (the component cases are `sb_16_a_schedule_entry_may_target_a_component`'s) | RS-52, SB-2, SB-16 |
| `rs_50_stop_with_and_without_a_target` | `Stop { radio }` then `Stop {}` | the first compiles to an `Action::Stop` with a target and no control operation (the Run keeps `Running`: `test_ea_16_rx_stop_then_start`); the second compiles to `ControlOp::StopRun` (which ends the Run: `kc_28_stop_run_ends_the_session`) | RS-50, RS-14 |
| `rs_8a_wedged_cleanup_step_times_out` | a Provider double that never returns from `drain_and_stop_consumers` | the step is abandoned and recorded as a timeout; steps 4 to 7 still run and a Manifest is written | RS-8a, RS-11 |
| `rs_44a_a_burst_without_a_waveform_is_refused` (`src/session.rs`) | a repeat verb with ingested bytes, then one with no waveform | the first compiles to a `TxBurst` carrying the ingested hash; the second is refused | RS-44a |
| `rs_40_capture_across_a_rate_change` | two `ContinuityMap`s built by `ContinuityBuilder` for two domains, assigned to one artifact in SampleClock order | the artifact carries both, in that order (a capture actually spanning a change is the Test line's `hd_10_a_capture_spans_a_gap_and_a_clock_change` and `test_ea_17_a_capture_across_a_rate_change_is_refused`) | RS-40, SC-30, TM-13c |
| `rs_35_dropped_events_never_enter_the_ring` | a storm that fills the ring, drained repeatedly | every `EVENTS_DROPPED` is delivered; the RS-35 invariant holds each time | RS-35 |
| `rs_17_a_set_parameter_compiles_to_a_class_pending_update` (`src/session.rs`) | `SetParameter` on `test.gain`, then on `test.flag`, whose `KeyDecl` declares no class | both compile to class-pending updates with no class and no coercion: the class is admission's, read at KC-24 step 5 | RS-14, RS-17, RS-52 |
| `rs_19_capture_asap_records_applied_time` | a `sink.capture` verb with no `at` | the entry carries a coercion from "as soon as possible" to the applied time | RS-19 |
| `rs_15_log_sequence_is_dense` | five Actions, two of them rejected | sequence numbers 0 to 4 with nothing missing | RS-15 |
| `rs_20_replay_divergence` | a replay against a profile with a different hash | `ReplayDivergence` | RS-20 |
| `rs_21_lease_default_and_ttl_required` | a Run with no Lease; then `Detached` with no TTL | `Attached`; then `LeaseTtlRequired` | RS-21 |
| `rs_22_ttl_uses_the_host_clock` | the host clock advanced one millisecond on a Detached Lease whose client disconnected | the Lease has not expired | RS-22 |
| `rs_23_attached_stops_on_disconnect` | a disconnect on an `Attached` Lease | `Lease::on_disconnect` returns `StopCause::ClientDisconnect {}` (the Run's end is spec 06's `kc_36_attached_disconnect_ends_the_run`) | RS-23 |
| `rs_23_detached_expires` | a 5 000 ms TTL, a disconnect, the host clock advanced 4 999 then 1 | the Lease has not expired, then has (the Run's end is spec 06's `kc_36_detached_lease_expiry_ends_the_run`) | RS-22, RS-23 |
| `rs_24_adopt_requires_the_token` | `Lease::adopt` with the right token, then the wrong one | `adoptions` is 1; then `AdoptRejected`; a `Renew` on a non-renewable Lease is `LeaseNotRenewable` | RS-24 |
| `rs_27_a_second_declaration_of_one_kind_is_refused` | a kind the `test` Vocabulary registered, registered again | `EventKindAlreadyRegistered { kind }` | RS-27 |
| `rs_28_policy_defaults_table` | one event of each of the five Kernel-registered kinds, plus `test.*` kinds registered by the test Module with their own defaults | exactly the declared reactions; no radio kind is registered by the Kernel | RS-27, RS-28 |
| `rs_29_unknown_kind_by_severity` | `vendor.unregistered` at info, error and fatal | continue, mark, abort | RS-29 |
| `rs_26_policy_override_from_spec` | `policies.failure: { test.custom: stop }`, where the test Module registered it with a default of continue | the compiled Policy answers `stop` for `test.custom`, overriding the declared default; an unregistered override is refused (`UnknownEventKind`) (the Run stopping on it is spec 06's `kc_32_a_reaction_equal_to_the_first_end_is_not_recorded`) | RS-26, SB-18 |
| `rs_30_mark_artifact_marks_open_artifacts` | one open `ArtifactRef` and a direct `mark_open_artifacts` call | the mark appears on that `ArtifactRef` (the event path is the `kc_42_*` rows') | RS-30 |
| `rs_30_marks_every_open_artifact_including_a_partial_one` | two open `ArtifactRef`s, one of them partial, and two direct `mark_open_artifacts` calls | two marks on each artifact, the partial one included (the marks' kinds and times are not asserted) | RS-30 |
| `rs_32_emit_allocates_nothing` | 100 000 emissions under a counting allocator | zero allocations; the inline payload limit is the documented 32 bytes and an `EventRecord` stays within 64 bytes | RS-32 |
| `rs_33_counters_exact_under_drop` | a ring of depth 8, thirty thousand overflows from one source, drained once at an instant | the counter reads 30 000, eight bodies delivered, one `EVENTS_DROPPED` of 29 992, stamped at that instant with source `kernel` | RS-33, RS-35 |
| `rs_35_dropped_counts_are_deltas` | drop ten, drain, drop five, drain | two `EVENTS_DROPPED` events of ten and five | RS-35 |
| `rs_33_unforeseen_pair_uses_the_fallback_row` | an event from a source not in the plan | the fallback row, kind `ezsdr.unforeseen`, increments | RS-33 |
| `rs_33_drain_uses_the_planned_kernel_source_for_meta_events` | a drop under a planned `kernel` source | the drain's `EVENTS_DROPPED` is counted in the `(kernel, EVENTS_DROPPED)` row (that this row is the collector's own, not a planned pair, is `rs_33_planned_kernel_source_shares_its_meta_counter_row`) | RS-33, RS-35 |
| `rs_49a_a_timed_template_resolves_with_its_deadline` (`src/event.rs`) | a `TxBurst` template and a deadline | the resolved Action carries that deadline | RS-49a, SB-16 |
| `rs_49a_a_scheduled_update_resolves_to_a_class_pending_request` (`src/event.rs`) | an `UpdateParameter` template and a deadline | a class-pending request at that instant, never a finished `UpdateParameter` | RS-49, RS-49a, RS-52 |
| `rs_19_vocabulary_action_carries_a_time` | a `test.sweep` verb with an `at` | it compiles to a `Command` timed at that instant, with no coercion (the untimed case is `rs_19_an_untimed_command_carries_its_resolved_time`) | RS-19 |
| `rs_36_abort_survives_a_drop` | the ring filled, then a `DEVICE_LOST` whose body is dropped | the collector's escalation names it with `abort`; the counter reads one | RS-36 |
| `rs_36_a_dropped_stopping_body_ends_the_run` | a Simulation Run whose Provider fills the ring in one step, then emits a hot-path `DEVICE_LOST` whose body is dropped | the Run ends `Stopped { policy { DEVICE_LOST } }` in `abort` mode; `delivered` holds no `DEVICE_LOST` but its `EVENTS_DROPPED` (count 1), stamped at the round's instant in the primary root with source `kernel` | RS-35, RS-36, KC-31 |
| `rs_11_manifest_for_every_terminal_run` | three Manifests whose terminations are failed, stopped and completed | each seals with the matching `termination.reason` and a hash (a real Run's Manifest is spec 06's `kc_01_a_validate_failure_still_writes_a_manifest` and `kc_44_a_wedged_step_does_not_prevent_the_manifest`) | RS-11a |
| `rs_45_hash_equal_for_equal_inputs` | one Spec and one profile through `start_spec_run`, once with their defaults omitted and once spelled out in another order and spacing | equal `spec` and `binding` hashes and bodies in the two Manifests | RS-45 |
| `rs_38_spec_source_is_recorded_outside_the_spec_hash` | one Spec Run with `Assembly.spec_source` set and one without; a Session with it set | `spec.source` is the supplied hash, or absent; `spec.hash` and `spec.body` are equal in the two Manifests; the Session records the supplied hash | RS-38, RS-45 |
| `rs_39_section_namespace_enforced` | a Module writing under another Module's namespace | `SectionNamespaceForbidden` | RS-39 |
| `rs_43_no_seeds_field_in_the_envelope` | an environment declaring a seed | it appears only in `binding.body`; the envelope has no `seeds` field | RS-43 |
| `rs_38_environment_recorded_verbatim` | an arbitrary environment section | its content in the Manifest | RS-38, SB-27 |
| `rs_41_fidelity_is_the_weakest` | a pair of `Fidelity` values declaring `hardware_quirk` and `envelope` for timing, and an empty list | `Fidelity::weakest` answers `envelope`, and `none` for the empty list; none named for a Run whose Manifest `fidelity` is the weakest over its bound Providers' declarations (a single Provider's settled vector is spec 06's `kb_02_the_manifest_records_the_fidelity_settled_in_prepare`) | RS-41 |
| `rs_42_determinism_only_in_simulation` | a sealed Manifest whose `execution_class` is `RealtimeEmulation` and whose `deterministic` is true, and one whose class is `Simulation` | the `RealtimeEmulation` claim is cleared; the `Simulation` claim stands | RS-42 |
| `rs_38_counters_include_zero_rows` | an `EventCollector` with two registered pairs and no events | at least three rows, every row's count zero (which rows they are is not asserted) | RS-38, RS-33 |
| `rs_38_manifest_carries_its_mandatory_version` | a sealed Manifest, then the same document with `version: 2` | `version` is 1 inside the hashed body; the future major is refused by name | RS-38, SB-47 |
| `rs_44_partial_artifact_on_abort` | an abort with a capture open | the `ArtifactRef` is marked partial | RS-6, MA-26 |
| `rs_17_a_session_rate_change_is_coerced_by_its_provider` | Session sets `test.grid` to 19.5 against a Provider grid of 20 | the entry is admitted with value 20.0 and records the coercion | RS-17, KC-26 |
| `rs_17_a_session_change_beyond_a_joint_limit_is_refused` | `test.count = 2` in effective configuration and `test.grid = 40` against joint limit 50 | rejected with `ezsdr.coercion` on `test.grid` | RS-17, KC-26 |
| `rs_17_a_scheduled_rate_change_under_reject_is_refused` | the same rate update in a Spec schedule under coercion policy `reject` | `Failed { arm }`, reason contains SB-46 | RS-17, SB-46, KC-26 |
| `kc_28_a_malformed_action_takes_no_sequence_number` | a Session Action with a non-ASCII value key, followed by a valid Action | first returns `Malformed`; next entry has sequence 0 | RS-15, KC-28 |

## 8. Vision coverage

| Vision | This spec |
|---|---|
| §3 Sessions, the action log, admission, child Runs, replay | RS-12…RS-20 |
| §11 prepare, arm, start, running, stop, cleanup as one transaction | RS-2, RS-3, RS-6 |
| §14 ExecutionClass, fidelity vector, the scope of determinism | RS-41, RS-42 |
| §27 update classes only, no structural mutation | RS-4, RS-17, RS-52 |
| §29 the event envelope, counters, storms, `EVENTS_DROPPED`, no metrics framework | RS-27…RS-36; the metrics prohibition is Vision prose (RS-37 withdrawn) |
| §35 `DEVICE_LOST` maps to abort as a Kernel policy | RS-27, RS-28 |
| §50 the Manifest envelope, sections, content addressing | RS-38…RS-47 |
| §5, §19 the Kernel Action set | RS-48…RS-52 |
| §53 cleanup, Lease modes, the closed Policy table | RS-6…RS-11, RS-21…RS-30 |
| §54 the Easy API maps onto Sessions | RS-13, RS-14 |
| §58 #5, #7, #13, #16 | the `rs_28_*`, `rs_11_*`, `rs_12_*` and `rs_16_*` tests |

## 9. Vision issues found

1. **§29 writes `DEVICE_DISCONNECTED` and §35 writes `DEVICE_LOST`** for the same condition. RS-27 uses `DEVICE_LOST`, and §29's list should be corrected in the R13 pass.
2. **§22 says a late burst "emits LATE" where §13 and §29 treat lateness as an event.** Both exist and mean different things: `LATE` is a block flag (SC-16a) and the event is `TIME_ERROR` for a burst target already passed or `LATE_COMMAND` for a control command issued too late. The Vision should distinguish them.
3. **§53 lists the cleanup actions as unordered bullets** while §11 orders only stop before cleanup. RS-6 fixes an order, and RS-7 gives the reason the list's own order would be wrong. §53 then copied RS-6's first list, whose radio and Peripheral steps — stop TX, stop RX, cancel Peripheral operations, end child Runs — misdescribed what the coordinator does; RS-6's steps are now named for it, without the two that never act, and §53 follows (pre-freeze audit item 4, 2026-10-09).
4. **§3's `Stop` is ambiguous**, meaning either "stop this transmitter" or "end the Session". RS-14 splits it with an optional target, which requires the Kernel `Stop` Action to carry one (spec 05).
5. **§29 says event bodies "may be sampled or dropped".** Sampling and dropping are different mechanisms with different guarantees. Phase 1 implements dropping only (RS-34); subsampling, if it is ever wanted, is a Sink's policy.
6. **§50 lists "random seeds" and an environment capture in the envelope** although the Kernel owns no generator and defines no capture content. RS-43 keeps both out of the envelope: the environment is recorded already, and the capture is a Module's section.
8. **Vision §3's action log names `StartRepeat` and `Capture` as if they were Kernel vocabulary.** Both are domain verbs — `repeat` is a Radio Model capability in audit §13 and a recorder is a Sink — so RS-13a makes them namespaced Vocabulary verbs. The Session log and its replay are unchanged; only the Kernel's list of variants shrinks.
9. **Vision §29's list of event kinds reads as a Kernel registry.** RS-27 keeps in the Kernel only the kinds it emits itself or owns the policy for — of §29's own list, `PROCESSOR_DEADLINE_MISS` and, under §35's name, `DEVICE_LOST` — and gives the rest to the Vocabulary that emits them, because §5's three tiers would otherwise make a new radio event kind a Kernel change.

## 10. Deferred

Subsampling and any export of counters (Sink Modules). Authentication beyond the adoption token (Vision §62). GraphEpoch (future work; RS-4's prohibition is in force now). Migrations of the Manifest, until a version 2 exists.

## Changes

| date | rules | change | record |
|---|---|---|---|
| 2026-10-08 | RS-19 | an action with no `at` whose target is a stream is admitted no earlier than the origin of the stream's latest registered clock that has not ended, so a first `start_repeat` after a transmit enable is not late (checked by spec 06's `kc_28_an_untimed_burst_is_not_admitted_before_its_clock_s_origin`) | owner decision, 2026-10-08 |
| 2026-10-09 | RS-6 | the event path's last drain moves from step 7 into step 6, before the marks are recorded on the artifacts, under `orderly` and `abort` alike, so a mark delivered during cleanup reaches the artifact it overlaps (checked by spec 06's `kc_42_a_mark_delivered_during_an_orderly_cleanup_reaches_the_artifact` and `kc_42_a_mark_delivered_during_an_abort_cleanup_reaches_the_artifact`); the `CleanupStep` names are unchanged | [note 25](../plan/maintenance/25-fault-and-mark-notes.md), #66 |
| 2026-10-09 | RS-38, RS-45 | a document is hashed and recorded as parsed and re-serialised canonically from the parsed document, not as submitted, and a `RunChild` entry names the hashes its child's Manifest records; the Manifest's `spec` section loses `original_version` and `original_hash` (SB-49 withdrawn) | [audit item 5](../plan/maintenance/24-prefreeze-audit.md), owner decision 2026-10-09 |
| 2026-10-09 | RS-3, RS-6…RS-11a, RS-25, RS-27, RS-33, RS-35, RS-38 | `Failed` carries its `reason`; RS-6's step 3 drain records a `next_wakeup` panic as `Failed { run }` (F25 (a)); RS-6's steps are named for what they do (`freeze_dispatch`, `stop_providers`, `drain_and_stop_consumers`, `restore_baseline`, `finalise_artifacts`, `flush_events`, `release_and_write_manifest`) and numbered 1–7, the two that never acted (child Runs, Peripheral operations) deleted, so RS-25 no longer cites a step; every event kind has a namespace and the Kernel's are under `ezsdr.`, the fallback kind `ezsdr.unforeseen`; `EVENTS_DROPPED` is stamped at the drain's instant with source `kernel`; the Manifest records `links` and `run.children` as typed records, `execution_class` absent without a plan, `also` as Terminations, and the Lease as `{ mode, released }`, and the Kernel writes no section (checked by `rs_06_cleanup_order_orderly`, `rs_33_counters_exact_under_drop`, `rs_33_unforeseen_pair_uses_the_fallback_row` and spec 06's `kc_45_manifest_fields`, `kc_32_a_later_failure_is_recorded_in_also` and `kc_30_a_next_wakeup_panic_in_the_cleanup_drain_is_recorded`) | [audit item 4](../plan/maintenance/24-prefreeze-audit.md), owner decision 2026-10-09 |
| 2026-10-10 | RS-19, RS-38, RS-48, RS-49 | the Action `PeripheralCommand` is `Command`, a Vocabulary verb sent to any target (#55); the Manifest's `spec` section gains `source`, the caller's builder hash, outside `spec.hash` (SB-14, KF-4), recorded for a Session too as opaque provenance; `plan` has no transfer costs (SB-40) | [audit item 7](../plan/maintenance/24-prefreeze-audit.md), owner decision 2026-10-09 |
| 2026-10-10 | RS-15 | the log's id checks are deleted (X7 and SB-1 hold by construction: `NodeId` and `ResourceId` have private fields), and `check_entry` takes the action only | [spec 26](../plan/maintenance/26-invariants-in-types.md) §2, audit item 1 |
| 2026-10-10 | RS-12, RS-14, RS-31, RS-33, RS-35, RS-39, RS-49 | an Action and a Session action name a typed `Target` (`{resource, path}`, `{output}`, `{component}`), and an Event and a counter row a typed `EventSource` (`{node}`, `{output}`, `{island}`, `kernel`, `unforeseen`); `sink.capture` compiles to the output it names and the Kernel chooses no recorder (#54; `rs_14_capture_without_recorder_rejected` is deleted, the refusal is KC-23's); Manifest `sections` is a list of `{source, name, content}` filed under the writer's source root (decision 2) | [spec 27](../plan/maintenance/27-role-typed-identities.md) §2, [audit 28](../plan/maintenance/28-forward-audit.md) decision 2 |
| 2026-10-10 | RS-17 | a Provider parameter update whose value is a list or a map is refused before `coerce`, since `Constraint` holds `Scalar` (SB-5) | [spec 26](../plan/maintenance/26-invariants-in-types.md) §3, audit item 1 |
| 2026-10-11 | RS-6, RS-15, RS-16, RS-38 | an admitted Action is queued with its id, and RS-6 step 1 records the ids it discards in `termination.undelivered`; a Module's `Abort` takes a real id that `submit` returns, neither admitted nor logged (owner, 2026-10-11), and no Manifest field records it; `artifacts` is a map from the producing output to its `ArtifactRef`s, with no uniqueness rule on ids; `prepare.reports` is a map filed under the fragment invoked | [spec 27](../plan/maintenance/27-role-typed-identities.md) §3 |
| 2026-10-12 | RS-14, RS-17, RS-19, RS-27, RS-32, RS-33, RS-49a, RS-52, §4 `RunError`, the test table | a Session's and the schedule's parameter changes compile to class-pending update requests, and a template or `CompileRule` that still states `class` is refused at parse; the class comes from the target's own declaration alone — a `KeyDecl`, or a component's optional `ParamDecl.update_class` with no Vocabulary fallback — and is checked at spec 06 KC-24 step 5, before the coerce and the registered checks, which RS-17 now points to instead of restating; a Module emits through a root-bound view whose `resolve` is fallible and whose `emit_control` refuses a source outside the root and a non-ASCII payload key; an `EventHandle` is opaque, and one minted by another view or Run is `ForeignEventHandle`; the fallback row is for an unforeseen pair inside the instance's own tree | [spec 29](../plan/maintenance/29-amendments.md), pre-freeze item 3 |
| 2026-10-13 | all; RS-6, RS-14, RS-16, RS-38, RS-43, RS-49, §7, §9, §10 | rewritten to spec 22's DA-1 format: rationale, history, phase labels, review ids and finding ids leave the rules, each rule's test line moves to its own line under it, and the full text as it last stood is at commit `7396bb0`; RS-16 gains the sentence that a Module-submitted undelivered Action's id is in `termination.undelivered` but in no log entry; RS-6's step 3 cites spec 06 KC-30, which states the `next_wakeup` panic's `Failed { run }` reason, instead of KC-20; RS-38's diagram spells `host_utc_nanos` as `manifest.rs` and KC-45 do, and gains `policy` and admission's `needs` and `warnings` as `manifest.rs` and `binding.rs` have them; RS-49's channel sentence points to spec 07 RM-13, which settled that a burst transmits on every channel of its stream and leaves `metadata` empty; RS-43 keeps that the Kernel owns no random number generator; §7's heading drops its phase, its `rs_14_the_kernel_supplies_no_vocabulary_value_or_late_policy` row drops the third case it does not assert, and the row for `rs_14_a_reject_at_plan_session_verb_is_refused_at_registration` is added; §9's item 7 is deleted (the Action set has `Command`, and `radio.start_rx` is a `Command` to a receive stream, RM-12) and item 2 cites SC-16a; §10 drops the items the header's Not-in-scope row or specs 07 and 16 already hold; no other change of behaviour | [plan 31](../plan/maintenance/31-item8-kernel-specs.md) §4.2, this stage's brief |
| 2026-10-16 | RS-2, RS-4, RS-7, RS-8a, RS-9, RS-10, RS-11a, RS-12, RS-13, RS-13a, RS-14, RS-16, RS-18, RS-19, RS-22, RS-23, RS-24, RS-26, RS-28, RS-29, RS-30, RS-31, RS-32, RS-33, RS-36, RS-38, RS-39, RS-40, RS-41, RS-42, RS-43, RS-44, §7 | Test lines and §7 rows state only what their tests assert (the Test-line sweep): a wrong name corrected (`schema_freeze` → `ov_22_schema_freeze`), a stale `none named` replaced by the frozen `$defs/RunState`, the tests that do pin a claim named (`rs_13_every_session_action_compiles_to_its_kernel_form`, `kf_03_a_child_run_is_admitted_logged_run_and_recorded`, `rs_19_an_untimed_command_carries_its_resolved_time`, `rs_17_a_set_parameter_compiles_to_a_class_pending_update`, `rs_30_marks_every_open_artifact_including_a_partial_one`, `ka_13_an_invalid_provider_namespace_without_sections_is_not_a_failure`, `hd_10_a_capture_spans_a_gap_and_a_clock_change`, `test_ea_17_a_capture_across_a_rate_change_is_refused`, `kg_03_step_3_runs_a_final_round_over_the_sinks`, `kg_03_an_abort_stops_the_data_thread`, `test_ea_16_rx_stop_then_start`, `kc_28_stop_run_ends_the_session`), over-claimed inputs and expectations narrowed to the predicate or compilation the named test actually exercises, and `none named` clauses added where no test asserts the condition — a real Provider's declared cleanup deadline, the log entries of an unknown-namespace verb and of `Adopt`, the declared `fatal` severity of the two aborting kinds, a `stop`-reacting kind's escalation flag, a Provider queue staying empty, a burst's cancellation, the recorded seed, the environment capture and the weakest fidelity over bound Providers; one §7 row added for `rs_30_marks_every_open_artifact_including_a_partial_one`; fix round 1 narrows four §7 rows and two Test lines further (`rs_38_counters_include_zero_rows` states only three-or-more rows all counting zero, `rs_13a_unknown_vocabulary_verb_rejected` stops at the compile refusal whose reason names `radio.start_repeat` rather than the check's name, `rs_06_cleanup_order_abort` records only its mode, `rs_30_marks_every_open_artifact_including_a_partial_one` states two marks on each artifact without their kinds and times), and the Test lines cite `ov_22_schema_freeze` for the envelope carrying no capture field and `hd_15_a_capture_across_a_clock_change_has_no_metadata` for the channel-count change (three Test lines: RS-9, RS-40, RS-43); no change of behaviour | Test-line sweep, 2026-10-16 |
