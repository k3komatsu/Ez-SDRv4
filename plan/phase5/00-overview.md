# Phase 5 — Mini Reactive Radio: overview and plan

| Field | Value |
|---|---|
| Status | **Planned and implemented under the owner's delegation** (2026-09-26: "Phase4と同様にPhase 5の計画を立てて実装まで進んでください"); there is no separate Gate P. **Accepted at Gate X** (owner, 2026-09-26, every decision as recommended; §11); **Step X done** (spec 14 moved to `design/`, three Vision issues applied). Phase 5 is complete. Reviewed by an Opus review loop (§9): Review F found 2 P0, 4 P1 and 11 P2 and Review G, the re-review of those fixes, 4 P1 and 9 P2; all were fixed with tests or text (`implementation-notes.md`, "Review F", "Review G"), and the last fixes, being small and low-risk, were not reviewed again (the owner's rule). |
| Phase | Vision §67 Phase 5: "Mini Reactive Radio — PING → Reactor → timed PONG". Predecessor: Phase 4 (events, failure, continuity, artifacts; accepted at Gate X 2026-09-26). Successor: Phase 6 (Python Easy API). |
| Scope | The first Executor Module and the first Reactor, run end to end: a radio sends a PING, a Reactor on the other radio hears it in its receive samples and answers with a timed PONG through `admit()`, and the radio decides the PONG's lead as hardware would. Three Kernel holes the prototype hit (§2), and the Phase 5 items earlier phases left open (§8). |
| Not in scope | §3 lists it. In one line: no Event edges, no Reactor in a Session, no Actions applied to components, no model of a component's processing time, no dynamic waveform content. |
| Language | English, like Phases 1–4. |
| Location | Spec 14, the new Module's spec, was drafted here and moved to [`design/14-native-executor.md`](../../design/14-native-executor.md) at Step X, as spec 11 was. Spec 15, [`15-amendments.md`](15-amendments.md), holds the Kernel amendments; their text reaches `design/` in the commit that implements them (GX-5), and the file stays here as the record. |
| Modal verbs | "must" and "must not" are normative (OV-4a). |

Documents:

```text
plan/phase5/
  00-overview.md           this file: scope, holes, decisions, crates, governance, traceability, sequencing, exit criteria, decision log
  (spec 14, the Module ezsdr.exec.native 1.0.0 and its component ABI, is design/14-native-executor.md since Step X)
  15-amendments.md         spec 15: Kernel amendments KE-1…KE-5
  implementation-notes.md  what was run, what the reviews found, what was fixed
  exit-review/             per-rule dispositions (PO-10)
  vision-issues.md         the Vision edits specs 14 and 15 need, applied at Step X with the owner's approval
  tools/                   the mutation list (Appendix A of spec 15) and Phase 4's tool
```

---

## 1. What Phase 5 had to prove, and what already existed

§58 #9: "Reactive execution can receive an event and generate a dynamic TxBurst." §22: "A Processor or Reactor must be able to generate a burst dynamically during RUN … MockRadio applies the same policy with the same envelope, so a Reactor that is too slow for the hardware fails in simulation." §58 #3 adds "including the order of events and Reactor decisions".

Most of the machinery exists and is tested with doubles:

| Piece | Where | State before Phase 5 |
|---|---|---|
| A Module emits an Action through `admit()` | MA-14a, `ActionSubmitter`, the coordinator's `Submitter` | tested with `ProbeExecutor` (`kc_24_*`) |
| A runtime burst's lead decided by the Provider | MA-14, SC-27, MR-17 | tested with Session and scheduled bursts (`mr_17_*`, `v58_11_*`) |
| A target converted onto the transmit clock | SC-23a, SC-23b, KC-24 step 4 | tested (`kc_24_*`, `k3_*`) |
| Two radios hearing each other | spec 11, MR-31…MR-36 | tested (`v58_08_*`) |
| An Executor's `prepare`, `step`, `stop` in the coordinator | KC-11…KC-13, KC-23, KC-39 | tested with `ProbeExecutor` and `TestExecutor` |
| A component descriptor reaching its Executor | MA-19, MA-19a | tested with doubles |

What did **not** exist: an Executor Module that loads a component by its `impl` identity (MA-19b, a producer obligation "tested by the Phase 5 Reactor Executor"), a component that reacts to samples, and a Run that joins them. So Phase 5 began with a prototype: the Executor of spec 14, the responder of §7 and the Spec of `experiments::ping_pong`, run through the real coordinator with two MockRadios on a SimulationChannel. It hit three Kernel holes, listed in §2 with the evidence each left.

## 2. What the prototype hit, with evidence

| # | Hole | Evidence | Amendment |
|---|---|---|---|
| 1 | **A Reactor cannot transmit a waveform of its own.** A `TxBurst` carries its waveform by reference (RS-44), and a Provider reads the bytes from the Run's input store (MA-5a, MR-32). KC-9 fills the store only with the waveforms the Spec's schedule carries and drops every other `Assembly.inputs` entry (KB-1: "the store holds only verified bytes"). A Spec has no other place to name an input, so the PONG's bytes, handed to the Run in `Assembly.inputs`, are dropped before any Module is prepared | The prototype's Run failed `Failed { prepare }` with `KC-12: fragment island_0: responder: the PONG waveform sha256:c94f…a3787 is not an input of this Run`; `crates/ezsdr-kernel/src/coordinator/pipeline.rs` `check_inputs` iterates `spec.schedule` only | KE-1 |
| 2 | **Nothing checks that a Module's burst names a Run input.** RS-44a: "An Action whose `ArtifactRef` resolves to nothing is rejected." KC-9 makes that true for the schedule and KC-28 for a Session before admission; for a Module's burst nothing does. The burst reaches its Provider naming bytes nobody holds: MockRadio in channel mode refuses it at the device (MR-32), and MockRadio without a channel transmits its headers and records the burst | `crates/ezsdr-kernel/src/coordinator/admission.rs`: the `TxBurst` branch checks the target, the clock and `at`, and never the waveform | KE-2 |
| 3 | **A Reactor that answers during the orderly drain turns a clean stop into an abort.** Cleanup step 3 steps every instance until nothing is scheduled (KA-12), so a Reactor receives the receive tail and may answer it; dispatch is frozen by then (RS-6 step 1), `admit()` refuses with `ezsdr.dispatch`, and a component that reports the refusal as its own error makes the coordinator record `Stopped { client }` with `also: [Abort { "KC-30: island_0: responder: the PONG was refused: …RS-6: dispatch is frozen" }]` (KC-32). Nothing says a Module must read that refusal as "the Run is ending" | The prototype with a PING whose first sample reaches the responder in the drain (PING at 25 960 µs, stop at 25 990 µs): termination `also` held that abort | KE-3 |

Three Phase 5 items that the earlier phases left open need text, not code: RS-17's "whether the reactive path coerces is Phase 5's to settle" (decision K11 of spec 06), MA-30's "delivery latency on inter-Island Action edges … is Phase 5's", and MA-19b's producer obligation. KE-4 settles the first two; spec 14 carries the third (§8). Review F added one more: NX-7's refusal of Actions contradicted MA-24 and UC-2, which KE-5 amends.

## 3. Scope

### In scope

1. **KE-1** — a Spec may list `inputs`: artifacts the Run consumes that no schedule entry carries. KC-9 verifies and stores them as it does a scheduled waveform, and the Manifest records them (SB-9, SB-20a, KC-9).
2. **KE-2** — admission refuses a `TxBurst` whose waveform is not in the Run's input store with its declared size, for every origin (RS-44a, KC-24).
3. **KE-3** — a refusal with `ezsdr.dispatch` or `ezsdr.run_state` says the Run is ending; an emitting Module must not report it as its own failure (MA-14a).
4. **KE-4** — the two Phase 5 forwards settled in text (RS-17, K11; MA-30's ceiling).
5. **KE-5** — MA-24 and UC-2 bind an Executor that applies Actions; one that applies none refuses them (after Review F).
6. **Spec 14** — the Module `ezsdr.exec.native` 1.0.0, an Executor that loads compiled-in components by `impl` identity and steps them, with its component ABI (NX-1…NX-9).
7. **The Mini Reactive Radio** — the responder component in `ezsdr-acceptance` and the carriers of §8.

### Out of scope, with the phase that owns each

| Item | Why not now | Owner |
|---|---|---|
| Event edges between components (`Endpoint::EventIn` / `EventOut`, the `ezsdr.event.<schema>` contract) | §58's own "useful minimal reactive test" is "Radio A sends PING → Radio B receives PING → Reactor → dynamic timed PONG → Radio A receives PONG", and the responder is that Reactor with the detection folded in: it finds the PING in the receive samples. Vision §19's Reactor proper takes "events / messages", and §66's "reactive packet radio" is "continuous RX → packet detect → decode → Reactor → dynamic TxBurst", with detection and decoding before the Reactor; that chain needs an event edge between components. (The first draft cited `ComponentKind::Reactor`'s doc comment, "reacts to samples", which misquoted §19; the comment now follows §19. Review F, P1-2.) An event edge needs a queue the Kernel does not have: `EventIn` and `EventOut` are variants with no handle, and no coordinator code builds one. Its first real consumer is a PHY Processor handing decoded packets to a MAC Reactor (audit Appendix C, scenario C), which needs Phase 9's PDUs and Phase 10's Processors. **Owner decision before the Kernel freezes** (§11): the two variants' shape is undecided, and changing them after v4.0 would be a Kernel major | Phase 10 |
| A Reactor in a Session | A Session's implicit Spec has no graph components (SB-22c, `session::implicit_spec`), so an Island a Session profile places names nothing; a Reactor runs in a Spec Run | Phase 6 (`sdr.run(spec)` and child Runs) |
| Actions addressed to a component (`UpdateParameter` of a component parameter, `SetTimer`, `Stop` of a component) | The Kernel admits them (a component parameter declares its update class, RS-17), but no Phase 5 path sends one: a schedule entry cannot target a component (SB-16), a Session has none, and the responder addresses only its radio. The native Executor refuses them loudly (NX-7) rather than applying UC-2…UC-6 to parameters nobody updates. A component that needs a wakeup schedules one on `ctx.time` (MA-29, KA-11) | Phase 10 |
| A component's processing time in Simulation | Simulation charges a component no processing time, so a Reactor that computes for 5 ms and answers with a 4 ms turnaround passes here and is late on hardware. What §22 promises is enforced: the lead of a response, counted from when the device delivered the samples it reacts to (MR-14), is judged against the same envelope as on hardware (`v58_09_a_pong_short_of_the_lead_is_late_as_on_hardware`). Charging each component its declared budget (MA-36) in virtual time is the upgrade; RealtimeEmulation (TM-16a1) exposes the real time. Decision R4 | Phase 10 |
| Dynamic waveform content (a PONG whose samples depend on what was heard) | A `TxBurst` carries a waveform by reference (RS-44); content made at run time is link-fed transmission, a Processor feeding a transmit port, which has no transmit port yet | Phase 10 |
| MA-34 for a component parameter's key | `validate()` checks the owner of every resource constraint key (`check_keys`) and not of a component parameter key, whose grammar alone is checked. Found while writing the first component with parameters; no Phase 5 behaviour depends on it, and what "the declaring Module" means for a component (its Executor?) is a question for the first phase with components from several Executors | Phase 10 |
| A Manifest section written by an Executor | Only a Provider hands the coordinator sections (KA-13, `ProviderInstance.sections`). The responder's decisions are recorded where their effects are: the radio's burst records (SC-28), its `rejected` records and the `TIME_ERROR` events; a decision refused because the Run was ending is not recorded (NX-6) | when an Executor needs one |
| Executors on threads (RealtimeEmulation, hardware) | KC-2 runs the Simulation class only; the native Executor refuses another class (NX-4) | Phase 7 onwards |
| The spike's K3 | The responder's target is in its receive SampleClock, and admission converts it onto the transmit clock (SC-23a). With the rig's 2 s start lead the two grids agree and the conversion is exact; off the grid the burst moves to the next transmit sample with its `requested_target` recorded, which is K3's existing ceiling (`k3_an_off_grid_start_lead_moves_a_burst_to_the_next_transmit_sample`). Nothing new to decide | Phase 7 (Phase 4 Gate X) |

A request to add any of these during Phase 5 is scope creep and is refused (AGENTS.md §6).

## 4. Cross-cutting decisions

Each row is open to reversal at Gate X; a reversal is recorded in §11.

| # | Decision | Choice | Rejected (one line each) | Ceiling / upgrade path |
|---|---|---|---|---|
| R1 | What the Reactor receives | The responder radio's receive SampleStream, through a graph link; the component finds the PING's first sample (power above a threshold) and knows its device time as the block's first sample time plus `k` (Vision §23). This is §58's minimal reactive test with the packet detection inside the Reactor; §19's Reactor fed by events or messages from a detector is the upgrade (§3) | An Event edge from a detector Processor (the Kernel has no event queue, §3); a radio event announcing the packet (no UHD Provider could emit it, so the experiment could not run on hardware: §58 #10, §59); the Run's RuntimeEvents delivered to Modules (the event path is the Run's record, and routing Policy events into components would make the Policy a rules engine by the back door, RS-26) | Event edges, Phase 10 (§3) |
| R2 | Where the PONG's bytes come from | `ExperimentSpec.inputs` (KE-1): the Spec names the artifact, KC-9 verifies and stores it, the Manifest records it, and the component names it by hash in its parameters | A Module handle that adds bytes to the store at run time (a Run's inputs would no longer follow from its documents and Assembly, and RS-45's "equal Spec, binding, component and input hashes are comparable by construction" would fail); keeping every `Assembly.inputs` entry whose hash matches (the runtime, not the Spec, would decide what the Run consumed, and KB-1 drops unreferenced entries for exactly that reason); reusing the PING's waveform because the schedule happens to carry it (a Reactor could send only what the schedule sends) | A Reactor sends only waveforms declared before the Run; content made at run time is link-fed transmission (§3) |
| R3 | Who enforces RS-44a for a Module's burst | Admission, for every origin (KE-2) | Each Provider (MockRadio refuses only in channel mode, and a Provider that does not read the bytes would transmit a burst of nothing that the Manifest then records) | none |
| R4 | A component's processing time in Simulation | None: the component's Actions are submitted at the instant of the step that decided them. The lead the device sees already includes the delivery latency the device imposes (MR-14), which is what §22's check judges | Charging the declared budget as virtual latency in the Executor (worst-case, and §22's sentence is about the device's policy, not the host's speed; recorded as the upgrade); Kernel latency on Module Actions (MA-30's ceiling, KE-4: no Phase 5 cycle needs it) | §3; a Vision issue for §22's wording, which is the owner's to accept (§11): Review F judged §22's "too slow … fails in simulation" not met as written |
| R5 | The Executor | A new Module, `ezsdr.exec.native` 1.0.0, that loads each component by `impl.kind`, `impl.id` and `impl.hash` from the compiled-in implementations the runtime hands it (NX-3), steps them in component-id order and owns its component ABI (MA-21) | A Reactor written as an Executor with no component inside (MA-19b would stay uncarried, and the Spec's `impl` identity would name nothing that is checked); the Kernel's test `ProbeExecutor` (test code, and it loads nothing); a threaded Executor (KC-2: Simulation only) | Phase 10's execution engines may replace or extend the ABI; it is the crate's, not the Kernel's |
| R6 | How a component's Actions reach `admit()` | The component pushes them onto an output list in `step`; the Executor submits them in order after that component's step and handles every refusal one way: a Run-ending refusal is dropped (KE-3), any other fails the step (NX-6) | Each component calling `actions_out` itself (every component would have to know KC-24's refusal checks; the prototype's first responder did not, and recorded an abort) | A component that must learn an admission result gets it when an Executor needs one |
| R7 | Actions addressed to a component | Refused: the step fails, naming the Action (NX-7) | Applying UC-2…UC-6 to component parameters (no Phase 5 path sends one, §3); dropping them silently (MA-14: never silently accepted) | Phase 10 |
| R8 | RS-17's forward, spec 06's K11 | Settled: the Kernel does not coerce a Module's `UpdateParameter` (KE-4). The stepping loop holds every stepped Module while one runs, so the Kernel cannot call a stepped target's `coerce`; and for every target — a hardware Provider is not stepped (MA-15) — MA-14 already makes it enforce its envelope and report its Vocabulary's event, which keeps Mock and hardware alike (MockRadio re-runs `coerce` when an update applies, MR-18) | Coercing on the reactive path (the deadlock K11 names) | none |
| R9 | MA-30's inter-Island Action latency | Not added (KE-4). No Phase 5 path makes a zero-latency cycle: a Reactor's Actions go to a radio, which answers with a later block (MR-14) or transmits at a later sample, and the native Executor applies no Action (NX-7), so no Action edge joins two Reactors | Adding a Kernel latency now (it would model nothing a Phase 5 Run has) | Re-marked to the first Executor that applies Actions (Phase 10); the cap still turns such a cycle into `STEP_LIVELOCK`, not a hang |
| R10 | Where the responder lives | `ezsdr-acceptance/src/responder.rs`, as application logic: it imports only the Kernel, the Executor's ABI and the standard library, which `v58_10` checks (GX-6) | The Executor crate (an application inside an execution engine); a crate of its own (no second user) | Moves when a second experiment uses it |
| R11 | How Phase 5 is delivered | Planned from a prototype and implemented in one session under the owner's delegation, commit by commit on `main`; then an Opus review loop: review, fix with tests, review again unless the fixes are small and low-risk (owner's instruction) | Phase 3's verified patches and planning reviews before Gate P (sized for two new specs; Phase 5's Kernel change is small and its risk is in the new Module, which a review of the running code sees best) | The owner can reverse any row at Gate X; a reversal becomes a follow-up commit |

## 5. Crate layout

One crate is added. No external package is added (PO-4).

| Crate | Tier | Change |
|---|---|---|
| `ezsdr-kernel` | Kernel | KE-1: `ExperimentSpec.inputs`, KC-9's loop, the Spec schema; KE-2: the input store moves into the coordinator's shared state and admission reads it; tests. No public item added or removed |
| `ezsdr-exec-native` | Module (new) | `ezsdr.exec.native` 1.0.0 (spec 14). Depends on `ezsdr-kernel` and `serde_json` only (MA-3, PO-8) |
| `ezsdr-acceptance` | tests | the responder component, `experiments::ping_pong`, `rig::ping_pong_profile`, the Executor in `rig::assemble`, §8's carriers, the governance lists |
| the others | — | unchanged |

## 6. Governance

OV-1…OV-23b, PO-1…PO-12, GV-1…GV-6 and GW-1…GW-5 bind Phase 5. The rules below add what it needs.

- **GX-1** Phase 5 rule ids are `KE-n` (Kernel amendments), `NX-n` (spec 14) and `GX-n`, protected by OV-1. An amended rule keeps its id (PO-1); a new rule in an existing series takes the next number (`SB-20a`). *Process obligation.*
- **GX-2** The Kernel changes only as KE-1…KE-5 say. No Kernel public item is added or removed; the Kernel schema changes are KE-1's `inputs` on `ExperimentSpec` and, after Review F, the corrected description of `ComponentKind`'s `reactor` value, both recorded in `SCHEMA_CHANGELOG.md`. *Checked by `kernel_surface` (`ov_23b` prints 116 NEW / 292 public items, as after Phase 4) and `schema_freeze`.*
- **GX-3** A Phase 1–4 test changes only where spec 15 changes what it observes, and each such change is listed in `implementation-notes.md` with the rule that forces it. *Process obligation.*
- **GX-4** Every new or amended rule with code has a test that fails when the rule's code is disabled, shown by a recorded mutation (spec 15 Appendix A), as PO-12 requires. *Process obligation; exit criterion 6.*
- **GX-5** Spec 15's text reaches `design/` in the commit that implements it; spec 14 moves to `design/` at Step X; the Vision is not edited before Gate X (OV-6). *Process obligation.*
- **GX-6** The responder is application logic: its source names no Mock type and imports only `ezsdr_kernel`, `ezsdr_exec_native`, `serde_json` and `std` (§58 #10). *Checked by `v58_10_experiments_name_no_mock_type`, extended to `responder.rs`.*

## 7. Test strategy

- Kernel: KE-1 and KE-2 through the coordinator with the existing doubles (`crates/ezsdr-kernel/tests/coordinator.rs`); KE-1's parse in `spec_binding.rs`.
- The native Executor: `crates/ezsdr-exec-native/tests/native_executor.rs`, a harness in the style of MockRadio's (a `ManualTimeAuthority`, an `EventCollector`, a queue `ActionReceiver`, a recording `ActionSubmitter`) and small test components, so that loading, ordering, refusals and inbound Actions are each tested without a Run.
- End to end in `ezsdr-acceptance` with the real Modules: two MockRadios on a SimulationChannel with a coupling each way, the responder on an Island of `ezsdr.exec.native`, the recorder on the pinging radio. The responder hears a PING of constant amplitude through −6 dB and answers when a sample's power exceeds 0.02; with the channel's noise at −30 dBFS a noise sample exceeds it with probability e⁻²⁰.
- Timing arithmetic of the main carrier, which each test asserts exactly: at 1 Msps on `x310-like`, a PING at sample 10 000 of the pinger's transmit clock is radiated 45 samples late (MR-3's transmit path delay) and crosses a 1 µs path, so its first sample reaches the responder at receive sample 10 046; a 5 ms turnaround targets sample 15 046, which reaches the pinger at 15 092. The block holding sample 10 046 is published at 12 000 µs (MR-14), so with MR-3's 2 ms lead a target before sample 14 000 is late.
- With block-length jitter a block holds up to 4 000 samples (MR-12), so the responder may learn of a PING up to 4 ms after it began, and a 5 ms turnaround can then be late: the target does not move, the radio's verdict does, as the device's delivery latency would on hardware (found while writing `v58_03_reactor_decisions_reproduce_with_their_seed`, where one seed made the PONG late). The carriers that jitter therefore use a 7 ms turnaround, which no block length makes late (4 ms + 2 ms < 7 ms), and target sample 17 046.

## 8. Traceability

### Vision §58 → Phase 5

| # | Test | Phase 5 carrier (acceptance crate unless named) | Remaining |
|---|---|---|---|
| 9 | Reactive execution receives an event and generates a dynamic TxBurst | `v58_09_a_reactor_answers_a_ping_with_a_timed_pong` (the pinger hears the PONG from sample 15 092; the responder radio's burst record targets sample 15 046, on time, with no `requested_target`; the Spec carries no `sim.` key), `v58_09_every_ping_gets_one_pong` (two PINGs, two closed PONG bursts, SC-24) — §58's minimal reactive test, with the detection inside the Reactor | a Reactor fed by an event or message edge from a detector (Vision §19, §66's reactive packet radio): Phase 10, with the owner decision of §11 |
| 11 | Mock enforces the envelope | `v58_09_a_pong_short_of_the_lead_is_late_as_on_hardware` (a 1 ms turnaround: under `drop_and_flag` a `radio.TIME_ERROR { cause: late, outcome: drop }` and no PONG; under `send_asap_and_flag` the PONG at sample 14 000 with `late_by` and `requested_target` recorded; the boundary between on time and late at exactly the turnaround §7's arithmetic gives) | Phase 8 re-measures the lead |
| 3 | Deterministic with a seed, including Reactor decisions | `v58_03_reactor_decisions_reproduce_with_their_seed` (noise on both paths and jitter on the responder radio: one seed gives one Manifest projection, another seed another capture and the same decision; the same Run with the radios' names swapped gives the same decision) | — |
| 12 | Block-size independence | `v58_12_a_reactor_answers_whatever_the_block_lengths` (jitter on the responder radio: the same PONG target and the same capture at the pinger; the responder radio's block count differs, or the equality would prove nothing) | a Processor (Phase 10) |
| 10 | No Mock-specific API in application logic | `v58_10_experiments_name_no_mock_type`, extended to `responder.rs` (GX-6) | — |

### Earlier deferrals → Phase 5

| Item | Where it was deferred | Phase 5 disposition |
|---|---|---|
| §58 #9 | Phase 1 §8; Phase 2 §8; Phase 3 §3 | the carriers above |
| §58 #3's Reactor decisions | Phase 2, 3 and 4 §8 | `v58_03_reactor_decisions_reproduce_with_their_seed` |
| MA-19b: an Executor loads components by `impl` identity | spec 05 ("tested by the Phase 5 Reactor Executor"); Phase 2 KA-19 | NX-3 (`nx_03_*`) |
| RS-17's reactive coerce; spec 06 K11 | Phase 2 KA-6 | KE-4: settled, not coerced |
| MA-30's inter-Island Action latency | Phase 2 KA-11 | KE-4: re-marked to Phase 10 (R9) |

## 9. Sequencing

| Step | What | Check |
|---|---|---|
| 0 | This file, specs 14 and 15 committed | links resolve |
| 1 | KE-1…KE-4 (Kernel and `design/` text) | workspace tests on 1.85.0 and stable; Clippy; `kernel_surface` 116 / 292; `schema_freeze` |
| 2 | Spec 14: `ezsdr-exec-native` | the same |
| 3 | The responder and §8's carriers | the same |
| 4 | Appendix A's mutations | each killed |
| — | **Review F** (Opus, AGENTS.md §8): one adversarial pass over the diff and specs 14 and 15, running the tests and the mutations; fixes with tests; a further review unless the fixes are small and low-risk (the owner's instruction) | findings triaged in `implementation-notes.md`, recorded in §11 (OV-5) |
| 5 | Exit tables, Vision issues collected, `handoff.md` | **Gate X** (owner) |
| X | Spec 14 moved to `design/`, and the `plan/phase5/14-native-executor.md` references in `design/05` changed to it; Vision issues applied with the owner's approval | links and `v3/` paths recheck |

## 10. Exit criteria

1. Specs 14 and 15 accepted at Gate X, and every decision row — R1–R11 here, the owner decisions of §3, and the reviews' — has a verdict in §11.
2. Every Phase 5 rule — KE-1…KE-5, NX-1…NX-9, SB-20a, GX-1…GX-6 and each amended rule — has an OV-3 disposition in `plan/phase5/exit-review/`, read from test bodies (PO-10), with no `GAP` and no `UNCERTAIN`.
3. `cargo test --workspace` passes on Rust 1.85.0 and on stable with no `#[ignore]`; `cargo +stable clippy --workspace --all-targets -- -D warnings` passes.
4. Every carrier of §8 exists and passes.
5. `kernel_surface` (116 NEW / 292 public items), `schema_freeze` and every Vocabulary freeze test pass.
6. Every mutation of spec 15's Appendix A (29) is killed (GX-4).
7. The Kernel's direct dependencies are still exactly four; `Cargo.lock` gains no external package (PO-4).
8. Every link in `design/` and `plan/` resolves, and the `v3/` path check of `handoff.md` §1 passes.

## 11. Decision log

| Decision | Gate | Verdict | Note |
|---|---|---|---|
| Plan and implement in one session, then an Opus review loop (R11) | — | **delegated** | owner, 2026-09-26: "Phase4と同様にPhase 5の計画を立てて実装まで進んでください．実装が完了したらOpus 5.5にレビューさせて内容に従って修正し，再レビューです．もし修正内容が小規模で低リスクなら再レビューしなくていいです．これをループしてください．" |
| Event edges — Phase 10, or a Kernel decision before the freeze (§3) | X | **Phase 10** | owner, 2026-09-26, as recommended ("すべて推奨で受理します"): built with Phase 10's first Processor that hands a Reactor events; the two `Endpoint` variants may change shape until v4.0 because nothing builds them |
| §22's "a Reactor that is too slow for the hardware fails in simulation": accept Vision issue 2's rewording (the device's lead is enforced; a component's own processing time is not charged in Simulation), or charge each component's declared budget now (R4's rejected alternative) | X | **the rewording** | owner, 2026-09-26, as recommended ("すべて推奨で受理します"): Vision issue 2 applied at Step X; charging the budget belongs with Phase 10's budgets and `PROCESSOR_DEADLINE_MISS` |
| Review F: every P0, P1 and P2 fixed or answered (`implementation-notes.md`) | — | **closed** | 2026-09-26, before Gate X |
| Review G (re-review): every P1 and P2 fixed (`implementation-notes.md`); no further review, the fixes being text and two test cases | — | **closed** | 2026-09-26, the owner's rule for small, low-risk fixes |
| A component parameter that no Action may change (Vision §27's "a key with no class cannot change during a Run"): `ParamDecl.update_class` is mandatory, so every component parameter is admitted at run time and then refused by an Executor that applies none (Review G, P2-9) | X | **optional before the freeze** | owner, 2026-09-26, as recommended ("すべて推奨で受理します"): make the class optional before v4.0 freezes, with Phase 10's first Executor that applies parameters; until then the refusal (NX-7) keeps it loud |
| R1–R11, N1–N6, specs 14 and 15 (exit criterion 1) | X | **accepted** | owner, 2026-09-26, as recommended ("すべて推奨で受理します") |
| Step X: spec 14 moved to `design/14-native-executor.md`; the three Vision issues | X | **done** | 2026-09-26: §§9, 19, 22 and a revision-history row ([vision-issues.md](vision-issues.md)) |
