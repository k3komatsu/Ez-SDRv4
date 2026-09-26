# Phase 5 spec 15 — Amendments: a Reactor's inputs, its bursts, and the end of a Run

| Field | Value |
|---|---|
| Status | **Draft, implemented under the owner's delegation** ([`00-overview.md`](00-overview.md) §11). Its text is applied to `design/` in the commit that implements each amendment (GX-5); this file is the record. |
| Scope | Four Kernel amendments: KE-1 (a Spec lists inputs no schedule entry carries), KE-2 (admission checks a burst's waveform for every origin), KE-3 (a refusal because the Run is ending is not a Module's failure), KE-4 (the two Phase 5 forwards settled). |
| Amends | `design/03-spec-and-binding.md` (SB-9; new SB-20a; §4's `ExperimentSpec`), `design/04-run-and-session.md` (RS-17, RS-44a), `design/05-module-api.md` (MA-14a, MA-19b, MA-30's ceiling), `design/06-kernel-coordinator.md` (§3, KC-9, KC-24, K11, §13a), `schemas/experiment_spec.v1.json`. |
| Modal verbs | "must" and "must not" are normative (OV-4a). |

Each amendment gives the problem, the rule text as it reads after the amendment, the rejected alternatives and the tests. Appendix A lists the mutations that show each test guards its rule (GX-4).

---

## KE-1 — A Spec lists the inputs no schedule entry carries

**Problem.** A `TxBurst` carries its waveform by reference (RS-44), and the Provider reads the bytes from the Run's input store (MA-5a, KB-1). KC-9 fills the store with the waveforms of the Spec's schedule and drops every other `Assembly.inputs` entry, and a Spec has nowhere else to name an input. A Reactor decides its burst at run time, so its waveform is in no schedule entry: the prototype's responder found its PONG missing at `prepare` (`00-overview.md` §2, hole 1). The same gap would stop a Processor that reads a filter file or a calibration artifact, which spec 04 R10 already puts in the one `inputs` list of the Manifest.

**SB-9 amended**: the closed set is `version, requirements, resources, inputs, graph, schedule, outputs, policies, extensions`.

**SB-20a** (new):

> `inputs` lists artifacts the Run consumes that no schedule entry carries — the waveform a Reactor transmits, for example. Each is an `ArtifactRef`, which KC-9 verifies exactly as it verifies a scheduled waveform and whose bytes join the Run's input store, where a Module reads them by hash (MA-5a) and may name them in an Action (RS-44a). The Kernel reads nothing else about an input: which component uses it, and how, is that component's parameters' business (MA-36). Absent means empty (Phase 5, KE-1). *Checked by `ke_01_a_declared_input_is_stored_and_recorded`, `ke_01_a_declared_input_is_verified_as_a_scheduled_one_is` and `sb_09_inputs_is_a_top_level_field`.*

**KC-9 amended** (its first and last sentences):

> Every `ArtifactRef` the Spec's `inputs` lists and every one a Spec schedule entry carries is an input. … anything else is refused (`Failed { plan }`, reason beginning `"KC-9: input <i>: "` for the `i`-th listed input or `"KC-9: entry <i>: "` for a schedule entry). … Each distinct hash is recorded once in `Manifest.inputs`, as the `ArtifactRef` the Spec wrote, the listed inputs first in their order and then the schedule's in schedule order (Phase 5, KE-1).

The Spec schema (`schemas/experiment_spec.v1.json`) gains the optional property `inputs`, an array of `ArtifactRef`; `SCHEMA_CHANGELOG.md` records it. A document without it reads as before.

**Rejected.** A Module handle that adds bytes to the store at run time (a Run's inputs would no longer follow from its documents, and RS-45's comparability would fail). Keeping every `Assembly.inputs` entry whose hash is right (the runtime, not the Spec, would decide what the Run consumed; KB-1 dropped unreferenced entries for that reason). A per-component input list in `ComponentDescriptor` (a descriptor says what a component is; which waveform one experiment sends is the experiment's, and a second component or a calibration artifact would need its own list). Reusing a scheduled waveform (a Reactor could send only what the schedule sends).

**Code.** `crates/ezsdr-kernel/src/spec.rs`: `ExperimentSpec.inputs: Vec<ArtifactRef>` (`#[serde(default)]`) and `"inputs"` in `SPEC_TOP_LEVEL`. `coordinator/pipeline.rs` `check_inputs`: one loop over the listed inputs, then the scheduled waveforms. No public item is added (a field of an existing type).

**Tests.**

| test | input | expected | rules |
|---|---|---|---|
| `ke_01_a_declared_input_is_stored_and_recorded` (coordinator) | a Spec listing one input no schedule entry carries, its bytes in `Assembly.inputs`, a stepped test Provider that reads the store in `prepare` | the Provider reads the bytes; `Manifest.inputs` is exactly that `ArtifactRef` | SB-20a, KC-9, KE-1 |
| `ke_01_a_declared_input_is_verified_as_a_scheduled_one_is` (coordinator) | the listed input with its bytes absent; with one byte changed; with `size_bytes` one too large; with the uri `http://x`; then listed and also scheduled | `Failed { plan }` each time, reason beginning `"KC-9: input 0: "`; the last plans, and `Manifest.inputs` holds the hash once | KC-9, KE-1 |
| `sb_09_inputs_is_a_top_level_field` (`spec_binding.rs`) | a Spec with `inputs: [ArtifactRef]`; one with `inputs: [{ "uri": 1 }]`; one with no `inputs` | the first parses with the reference; the second is refused by the deserialiser (SB-9); the third reads as empty | SB-9, SB-20a |

`ov_22_schema_freeze` changes with the schema (GX-3).

---

## KE-2 — Admission checks a burst's waveform, whatever its origin

**Problem.** RS-44a: "An Action whose `ArtifactRef` resolves to nothing is rejected." KC-9 makes that true for the schedule and KC-28 for a Session before admission; nothing makes it true for a burst a Module submits (MA-14a). Such a burst reached its Provider naming bytes nobody holds: MockRadio in channel mode refused it at the device (MR-32), and MockRadio without a channel transmitted its headers and recorded the burst (`00-overview.md` §2, hole 2).

**RS-44a amended** (sentence added after "An Action whose `ArtifactRef` resolves to nothing is rejected."):

> For a `TxBurst` of any origin the check is admission's: a waveform whose hash names no bytes in the Run's input store, or bytes of another length than its `size_bytes`, is refused with the check `ezsdr.input` (KC-24) (Phase 5, KE-2).

**KC-24 amended**, step 4 ends: "…sets `requested_at` when it advanced; and the waveform's hash must name bytes in the Run's input store whose length is its `size_bytes`, else refused (`ezsdr.input`, RS-44a; Phase 5, KE-2)." Two rows join the refusal table:

| step | when | `check` | `reason` |
|---|---|---|---|
| 4 | the waveform is not in the input store | `ezsdr.input` | `"RS-44a: <hash> is not an input of this Run"` |
| 4 | its stored length is not `size_bytes` | `ezsdr.input` | `"RS-44a: input <hash> holds <n> bytes, and the burst declares <m>"` |

**Spec 06 §3 amended**: the input store is shared state, not the `RunHandle`'s: "State only the control path touches — the Session log, the agenda, the Lease and the documents' Manifest sections computed at entry — stays in the `RunHandle`; the input store is in `Shared`, because admission reads it for a Module's burst from inside a round (Phase 5, KE-2)."

**Rejected.** Leaving it to each Provider (a Provider that never reads the bytes transmits a burst of nothing and the Manifest records it; MockRadio without a channel is one). Checking only a Module's bursts (the schedule's and a Session's pass anyway, and one check for every origin is RS-16's "one admission function").

**Code.** `coordinator/state.rs`: `Shared.store`; `coordinator/mod.rs`: the `RunHandle` field removed; `coordinator/pipeline.rs`: every use reads `shared.store`; `coordinator/admission.rs`: the check at the end of step 4.

**Tests.**

| test | input | expected | rules |
|---|---|---|---|
| `ke_02_a_module_burst_must_name_a_run_input` (coordinator) | a test Executor submitting a `TxBurst` whose waveform is not an input; one whose waveform the Spec lists but with `size_bytes` one too large; one whose waveform the Spec lists | `Err` with check `ezsdr.input` and the two reasons above; then `Ok`, and the Provider receives the burst | RS-44a, KC-24, KE-2 |

---

## KE-3 — A refusal because the Run is ending is not a Module's failure

**Problem.** Cleanup step 3 steps every instance until nothing is scheduled (KA-12), so a Reactor sees the receive tail and may decide to answer it. Dispatch is frozen by then (RS-6 step 1) and `admit()` refuses with `ezsdr.dispatch`, or with `ezsdr.run_state` once the Run is not `Running` (KC-24). A Module that returns that refusal as its own step error makes the coordinator record the clean stop as an abort: the prototype's termination read `Stopped { client }` with `also: [Abort { "KC-30: island_0: responder: the PONG was refused: …RS-6: dispatch is frozen" }]` (`00-overview.md` §2, hole 3). Nothing told a Module how to read those two refusals.

**MA-14a amended** (sentence added at the end):

> A refusal whose every violation has the check `ezsdr.dispatch` or `ezsdr.run_state` (KC-24 steps 1 and 2) says that the Run is ending, not that the Action was wrong: the emitting Module must not turn it into an error of its own, which the coordinator would record as an abort of a Run that was stopping cleanly (KC-32). The Action simply does not happen, and the termination already records why (Phase 5, KE-3). *Carried by the native Executor, `design/14-native-executor.md` NX-6 (`nx_06_a_refusal_because_the_run_is_ending_is_not_a_failure`), and end to end by `ke_03_a_decision_after_the_stop_is_not_an_abort`.*

**Rejected.** Not stepping Executors in the drain (an Executor or Sink would lose the tail a Provider delivers, which is what KA-12's drain exists for). Admitting Module Actions during the drain (RS-7: a burst queued after dispatch froze would reopen the transmitter that step 2 stopped). A distinct error kind for "the Run is ending" (a Kernel type change for what the two existing checks already say).

**Tests.**

| test | input | expected | rules |
|---|---|---|---|
| `ke_03_a_decision_after_the_stop_is_not_an_abort` (acceptance) | the Mini Reactive Radio with the PING at 25 960 µs and the Run advanced to 25 990 µs and finished, so the PING's first sample reaches the responder only in the drain | the termination is `Stopped { client }` with `also` empty; the responder radio records no burst | MA-14a, KE-3, NX-6 |

---

## KE-4 — The two Phase 5 forwards settled

**RS-17 amended**: its sentence "*Forward: whether the reactive path coerces is Phase 5's to settle (Phase 2, KA-6).*" becomes:

> *Settled in Phase 5 (KE-4): it does not. The stepping loop holds every Module while one runs, so the Kernel cannot call the target's `coerce` for a Module's Action, and MA-14 already makes the target enforce its own envelope and report its Vocabulary's event; MockRadio re-runs its `coerce` when an update applies (MR-18). Checked by `kc_24_a_module_update_is_not_coerced_by_the_kernel`.*

**Spec 06 K11**'s ceiling cell "Phase 5" becomes "settled in Phase 5: not coerced (KE-4)", and §13a loses "The reactive path's coerce (Phase 5)."

**MA-30's ceiling** — its last sentence, "Delivery latency on inter-Island Action edges, which would let such a cycle advance in time, is not added in Phase 2, which has no Reactor and so no such edge; it is Phase 5's (Phase 2, KA-11)." — becomes:

> Delivery latency on inter-Island Action edges, which would let such a cycle advance in time, is not added: Phase 5's Reactor makes no zero-latency cycle reachable, because its Actions go to a radio, which answers with a later block (MR-14) or transmits at a later sample, and the native Executor applies no Action (`design/14-native-executor.md` NX-7), so no Action edge joins two Reactors. It is re-marked to the first Executor that applies Actions (Phase 10) (Phase 5, KE-4).

**MA-19b amended**: "*Producer obligation, tested by the Phase 5 Reactor Executor; Phase 2 has no Executor Module, only the coordinator's test doubles.*" becomes "*Producer obligation, carried by `ezsdr.exec.native` (`design/14-native-executor.md` NX-3, `nx_03_a_component_is_loaded_by_its_impl_identity`) (Phase 5).*"

**Tests.** None new: KE-4 changes text only; `kc_24_a_module_update_is_not_coerced_by_the_kernel` already carries RS-17's Module clause.

---

## Vision issues found

Applied at Step X with the owner's approval (OV-6, GX-5), recorded in [`vision-issues.md`](vision-issues.md):

1. **§9's conceptual shape of an ExperimentSpec** lists `version, requirements, resources, graph, schedule, outputs, policies, extensions`. It gains `inputs — artifacts the Run consumes that no schedule entry carries, such as a Reactor's waveform or a calibration artifact (§26)` (KE-1, SB-20a).
2. **§22 says "a Reactor that is too slow for the hardware fails in simulation."** What Simulation enforces is the device's side: the lead of a response, counted from when the device delivered the samples it reacts to, against the same envelope as on hardware. Simulation charges a component no processing time (`00-overview.md` R4). The sentence should say so, and name RealtimeEmulation and a component's declared budget as where processing time is exposed.
3. **§19** gains spec 14's `Normative:` line (spec 14 §8).

---

## Appendix A — Mutations (GX-4)

The list is [`tools/mutations.json`](tools/mutations.json), run with `python3 plan/phase5/tools/mutate.py plan/phase5/tools/mutations.json <scratch-dir>` (Phase 4's tool with its phase directory changed): each mutation is applied alone in a scratch copy, the named test is run, and the result is recorded in `implementation-notes.md` as `mutation: <name>: killed`, or the phase stops.

| Id | Change | Mutated crate | Killed by |
|---|---|---|---|
| E01 | KE-1 the listed inputs are not verified or stored | `ezsdr-kernel` | `ke_01_a_declared_input_is_stored_and_recorded` |
| E02 | KE-1 the listed inputs are not verified or stored, end to end | `ezsdr-kernel` | `v58_09_a_reactor_answers_a_ping_with_a_timed_pong` |
| E03 | KE-1 a listed input's size is not checked | `ezsdr-kernel` | `ke_01_a_declared_input_is_verified_as_a_scheduled_one_is` |
| E04 | KE-1 `inputs` missing from the top-level set | `ezsdr-kernel` | `sb_09_inputs_is_a_top_level_field` |
| E05 | KE-2 no waveform check at admission | `ezsdr-kernel` | `ke_02_a_module_burst_must_name_a_run_input` |
| E06 | KE-2 the stored length is not compared | `ezsdr-kernel` | `ke_02_a_module_burst_must_name_a_run_input` |
| E07 | NX-6 every refusal fails the step | `ezsdr-exec-native` | `nx_06_a_refusal_because_the_run_is_ending_is_not_a_failure` |
| E08 | NX-6 every refusal fails the step, end to end | `ezsdr-exec-native` | `ke_03_a_decision_after_the_stop_is_not_an_abort` |
| E09 | NX-6 any Run-ending check in a refusal drops it | `ezsdr-exec-native` | `nx_06_any_other_refusal_fails_the_step` |
| E10 | NX-6 every refusal is dropped | `ezsdr-exec-native` | `nx_06_any_other_refusal_fails_the_step` |
| E11 | NX-3 the implementation hash is not compared | `ezsdr-exec-native` | `nx_03_a_component_is_loaded_by_its_impl_identity` |
| E12 | NX-3 the implementation kind is not compared | `ezsdr-exec-native` | `nx_03_a_component_is_loaded_by_its_impl_identity` |
| E13 | NX-4 a component receives every link end of the Island | `ezsdr-exec-native` | `nx_04_prepare_cases` |
| E14 | NX-5 components stepped in reverse id order | `ezsdr-exec-native` | `nx_05_components_step_in_id_order_and_their_actions_follow_each_step` |
| E15 | NX-5 a pushed Action alone does not report progress | `ezsdr-exec-native` | `nx_05_components_step_in_id_order_and_their_actions_follow_each_step` |
| E16 | NX-7 an Action in the queue is ignored | `ezsdr-exec-native` | `nx_07_an_action_addressed_to_the_executor_fails_the_step` |
| E17 | NX-8 `stop` returns at the first error | `ezsdr-exec-native` | `nx_08_stop_reaches_every_component_and_cleanup_is_idempotent` |
| E18 | the responder answers at the block's first sample, not the PING's | `ezsdr-acceptance` | `v58_12_a_reactor_answers_whatever_the_block_lengths` |
| E19 | the responder answers every loud sample | `ezsdr-acceptance` | `v58_09_every_ping_gets_one_pong` |
| E20 | KE-1 the scheduled inputs are recorded before the listed ones | `ezsdr-kernel` | `v58_09_a_reactor_answers_a_ping_with_a_timed_pong` |
