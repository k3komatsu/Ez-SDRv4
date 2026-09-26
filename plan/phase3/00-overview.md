# Phase 3 — SimulationChannel and deterministic Runs: overview and plan

| Field | Value |
|---|---|
| Status | **Accepted at Gate P** (owner, 2026-09-26; verdicts in §11). Steps 0–7 and Review C are done (`5eb61dd`, `857a1c2`, `3cbc103`); **Accepted at Gate X** (owner, 2026-09-26; §11), with Review C's P2-1 … P2-9 applied (VB-10, KC-9); **Step X done** (spec 11 in `design/`, eight Vision issues applied). Phase 3 is complete. |
| Phase | Vision §67 Phase 3. Predecessor: Phase 2 (Radio Model, Simulation Engine, MockRadio; accepted at Gate X 2026-09-25, fixes `612b9eb`). Successor: Phase 4 (Events, failure, continuity, artifacts). |
| Scope | The **SimulationChannel** of Vision §16 and §57 — loopback, gain, delay and AWGN over a coupling matrix declared in the BindingProfile's `environment`; **MockRadio on that channel** — transmit sample content, gain and frequency acting on samples, the LO phase behaviour of RM-9, clipping, a path-delay default; the **two Kernel amendments** it needs; and **deterministic Runs** with the channel in the loop (Z10). The Phase 3 half of Vision §58: #8, and #3, #12 and #14 extended to the channel. |
| Not in scope | §3 lists it. In one line: no richer channel model than §57's four, no drifting clocks, no Reactor, no RealtimeEmulation, no hardware, no Python. |
| Language | English, like Phases 1 and 2. |
| Location | Spec 11 was drafted here and moved to [`design/11-simulation-channel.md`](../../design/11-simulation-channel.md) at Step X (2026-09-26). Spec 12's amendments are applied to `design/04…09` by a patch at Step 5 and stay here as the record. |
| Modal verbs | "must" and "must not" are normative (OV-4a). |

Documents:

```text
plan/phase3/
  00-overview.md              this file: scope, decisions, crates, governance, traceability, gates, exit criteria, decision log
  (spec 11, the SimulationChannel — CH-1…CH-11 — moved to design/11-simulation-channel.md at Step X)
  12-amendments.md            spec 12: Kernel amendments KB-1, KB-2; Vocabulary and Module amendments VB-1…VB-10,
                              with the new rules RM-23 and MR-31…MR-36
  20-implementation-plan.md   the ordered steps for the implementer: patches, commands, mutation checks, done-lists,
                              Appendix A (every changed file), Appendix B (every rule → test), Appendix C (mutations)
  patches/                    01-kernel, 02-vocabularies, 03-mock-radio, 04-acceptance, 05-design-text
  prompts/                    implementation and review prompts
  reviews/                    planning reviews before Gate P; Review C and the open items for Gate X
```

---

## 1. Why Phase 3 exists

Phase 2 made an experiment run entirely in software against an envelope-enforcing MockRadio. But every MockRadio received a test pattern, and nothing it transmitted went anywhere: Phase 2's Y7 and Y8 said so and named Phase 3 as the owner. Vision §57's minimal stack — `sdr.tx.repeat(x); y = sdr.rx.capture(N)`, entirely in software — therefore could not yet capture what it transmitted, and §58 #8, two MockRadios communicating through a SimulationChannel declared only in the environment, had no carrier.

Phase 3 closes that. It also makes "deterministic" mean more than Phase 2 could show: a Run with random channel noise must reproduce from its seed, from its own Manifest, whatever block lengths the receiver uses and whatever order the stepping loop visits two radios in (Z10). The last property is not free — §2 shows that Phase 2's MockRadio would have broken it.

## 2. What Phase 3 found in Phase 2 before writing a line of code

Reading the Phase 2 crates against what a channel needs found five places where the accepted specs or the code cannot carry it, and the planning reviews found a sixth on the Session path Phase 3's carriers use. Each is an amendment in spec 12, with its evidence:

| # | What Phase 2 has | Why the channel cannot work with it |
|---|---|---|
| KB-1 | The Run's input bytes live in `RunHandle.store` (`crates/ezsdr-kernel/src/coordinator/mod.rs:106`), which only the control path reads; a `TxBurst` carries only its waveform's `ArtifactRef` | No Provider can read what it is asked to transmit, so nothing can reach a receiver |
| KB-2 | MA-10: a Provider's `fidelity` is the same for the whole Run | MockRadio's `rf` fidelity is `impairment_model` exactly when the environment, read in `prepare`, declares a channel |
| VB-4 | A receive block is published, and a transmit block emitted, in the round of its last sample's instant (`crates/ezsdr-mock-radio/src/lib.rs:336`, `:781`) | A receive block's channel content would depend on whether a transmitter stepped earlier in that round had already started a burst at that instant — the order of stepping would change the samples; and an Action handled in a later pass of that round would find a transmit sample at its own instant already recorded, so a stop would cut the radiation below the record |
| VB-5 | A burst's lateness compares its target with the current instant floored to a transmit sample (`lib.rs:701`) | With no lead, a target already in the past is on time, and would radiate before the round that admitted it |
| VB-6 | A stop closes the open burst at the end of the last whole block emitted (`lib.rs:616`) | The samples between that block and the stop instant were transmitted, but are neither recorded (SC-28) nor radiated |
| VB-8 | MR-16 admits a burst that starts exactly at the open burst's next sample (`lib.rs:736`) | The open burst cannot end there with `END_OF_BURST`: one that has transmitted nothing is overwritten unrecorded (`lib.rs:1256`), and one whose block ending there was emitted is recorded as a discontinuity while the second burst is cut short and, on a channel, radiates a sample no record holds — both reached by Session `start_repeat` calls |

VB-4…VB-6 and VB-8 are Phase 2 defects that the channel or its Session carriers make observable; each holds with or without a channel (Z11). The KB amendments change the accepted specs 04–06, which is allowed before the v4.0 freeze and recorded in each spec (PO-9).

## 3. Scope

### In scope

1. **Spec 11**, the SimulationChannel (`sim` 1.1.0): the `sim.channel` section and its check, exact instants, the shared medium, the field — couplings, path gain, delay, the frequency gate, AWGN — the transmitter contract, finality (CH-9), determinism (CH-11).
2. **KB-1, KB-2**: a Module reads the Run's inputs by hash; a Provider settles its fidelity in `prepare`.
3. **VB-1**: `radio` 1.1.0 — path-delay capabilities (RM-23), the late comparison (RM-14), the stop instant (RM-16).
4. **VB-2**: `sim` 1.1.0 — the channel section, check and schema.
5. **VB-3…VB-8**: `ezsdr.radio.mock` 1.1.0 and profiles 1.1.0 — strict publication and emission, the late comparison, the stop cut, MockRadio on the channel (MR-31…MR-36), and no burst at or before the open burst's next sample (MR-16).
6. **Acceptance tests** for §58 #8 and the channel's part of #3, #12 and #14, and for §57's software loopback Session (§8).

### Out of scope, with the phase that owns each

| Item | Why not now | Owner |
|---|---|---|
| Channel models beyond §57's four: CFO, Doppler, phase noise, PA nonlinearity, IQ imbalance, a MIMO channel beyond independent paths | Vision §57: "The initial SimulationChannel may support only loopback, gain, delay, AWGN" | a later `sim` minor version when an experiment needs one |
| Band-limited interpolation between sample grids; complex path gains | Spec 11 C3, C10: no Phase 3 test needs them | the same |
| Per-device drifting roots and clock drift (Phase 2 Y11: "Phase 3+") | A channel between two drifting roots needs a ClockRelation with uncertainty and resampling; clock drift is one of Vision §17's faults | Phase 4 (fault kinds) or the first phase that models drift (Z2) |
| A per-Run delay override by a CalibrationArtifact (Vision §26) | Needs artifact typing | Phase 4 |
| Reactive execution, PING → Reactor → PONG (§58 #9) | Vision §67 | Phase 5 |
| Python client | Vision §67 | Phase 6 |
| RealtimeEmulation, hardware | Phase 2 Y1 | first phase with real deadlines; Phase 7 |
| Measuring the X310's path delays | Phase 8's parity test | Phase 8 |
| Link-fed transmission, `TX_UNDERFLOW` | Needs a Processor | Phase 10 |
| Cross-platform bit-exact reproduction | CH-11's ceiling | when asked |
| Phase 2's named Gate X risks (MA-8 timeout enforcement untested, the static review) | Recorded as Phase 2 ceilings; nothing in Phase 3 depends on them | unchanged |

A request to add any of these during Phase 3 is scope creep and is refused (AGENTS.md §6).

## 4. Cross-cutting decisions

Each row is open to reversal at Gate P; a reversal is recorded in §11. Spec 11's C1–C11 and spec 12's M11–M15 (in VB-7) are decided the same way.

| # | Decision | Choice | Rejected (one line each) | Ceiling / upgrade path |
|---|---|---|---|---|
| Z1 | Execution classes | Simulation only, as Phase 2's Y1 | RealtimeEmulation now (a threaded driver for no Phase 3 test) | unchanged |
| Z2 | Virtual time | One virtual root, as Phase 2's Y11 | A root per device with ClockRelations (the channel would need uncertainty and resampling for no Phase 3 test; drift is a fault, Vision §17) | Phase 4 or later (§3) |
| Z3 | Where the channel lives | A shared `Medium` in the `sim` Vocabulary crate, created by the runtime once per Run and handed to each simulated radio (spec 11 C1) | A Module, a Kernel Link, MockRadio itself, a global registry (spec 11 C1) | In-process only |
| Z4 | What the plan hands the implementer | **Verified patches**: the code, the tests and the amended spec text, produced in a scratch copy of `612b9eb` and verified there (592 tests on 1.85.0 and stable, Clippy clean, 83 of 83 mutations killed, each patch applied in order to a fresh clone), then attacked by five adversarial planning reviews that ran them (`reviews/planning-reviews.md`). The implementer applies them in order, runs the checks, re-runs the mutations and records the results (§9, GV-3) | Pseudo-code for the implementer to retype (Phase 2's Review M found 3 P0, 6 P1 and 16 P2 in retyped code); test tables for the implementer to write (Review M's N5 and N6: carriers a no-op implementation would pass) | The plan fixes the implementation as well as the rules; a reviewer reviews code at Gate P rather than after Step 4 |
| Z5 | Versions | `radio` 1.1.0, `sim` 1.1.0, `ezsdr.radio.mock` 1.1.0, profiles `x310-like` and `ideal` 1.1.0; `ezsdr.sim-engine` stays 1.0.0 | Keeping 1.0.0 with new behaviour (a Manifest would name one version for two behaviours); 2.0.0 (no key, kind or contract is removed) | Bindings naming MockRadio 1.0.0 are refused, as MR-2 refuses any other version |
| Z6 | Where the Kernel changes | Only KB-1 (`PrepareContext.inputs`) and KB-2 (MA-10's text) | Bytes in the Action; inputs handed at construction; `file://` reads by the Provider (spec 12 KB-1) | none |
| Z7 | Crates | None added: the channel is `ezsdr_sim::channel`, MockRadio's side is `ezsdr_mock_radio`'s private `channel` module | An `ezsdr-sim-channel` crate (its behaviour would need its own recorded version, spec 11 C2) | Crates are added, never merged, later |
| Z8 | Determinism scope | Bit-identical for one build on one platform (CH-11) | A portable math library now (a dependency or several hundred lines, for no Phase 3 need) | CH-11's ceiling |
| Z9 | Receive publication | Strictly after the last sample, in every mode (VB-4) | Only in channel mode (two publication rules in one Provider version) | none |
| Z10 | What "deterministic Runs" means in Phase 3 | Four properties, each with a carrier: one seed and one set of documents give one Manifest projection, noise included (`v58_03_channel_noise_reproduces_with_its_seed`); a Run reproduces from its own Manifest's documents and inputs (`v58_03_a_run_reproduces_from_its_own_manifest`); the channel's output does not depend on block lengths (`v58_12_the_channel_output_does_not_depend_on_block_lengths`) nor on the order the stepping loop visits the radios (`ch_09_the_receive_output_does_not_depend_on_the_stepping_order`, `ch_09_a_burst_starting_between_rounds_survives_a_stop_in_either_order`, `v58_08_a_session_hears_a_burst_from_its_first_sample_in_either_instance_order`) | Session replay (needs the waveform store by hash beyond a Run, Phase 6); cross-platform bit-exactness (Z8); stepping-order independence in a round a Module error ends early (CH-9's ceiling, Review C P2-1, accepted at Gate X) | Reactor decisions join #3 in Phase 5 |
| Z11 | VB-4…VB-6 and VB-8 without a channel | They apply in both modes | Channel-only fixes (a pattern-mode Mock would keep a defect the channel-mode one does not have) | none |

## 5. Crate layout

No crate is added and no dependency changes (PO-4, PO-8). What changes, per crate:

| Crate | Tier | Change |
|---|---|---|
| `ezsdr-kernel` | Kernel | KB-1: `module_api::InputStore`, `PrepareContext.inputs`, the coordinator's shared store; test doubles for KB-1 and KB-2; three tests |
| `ezsdr-radio` | Vocabulary | `radio` 1.1.0: two capability keys |
| `ezsdr-sim` | Vocabulary | `sim` 1.1.0: module `channel` (spec 11), the third check, `schemas/sim/channel.v1.json` |
| `ezsdr-mock-radio` | Module | 1.1.0: private module `channel` (timelines, transmit plan, receive model, waveform decoding, LO phases); `src/lib.rs` (channel mode, VB-4…VB-6, VB-8); profiles; `tests/mock_channel.rs` |
| `ezsdr-acceptance` | tests | `rig::assemble` hands one `Medium` to every MockRadio; `rig::link_profile`; `experiments::link` and `waveform_of`; seven tests |
| `ezsdr-sim-engine`, `ezsdr-hostmem`, `ezsdr-link-host`, `ezsdr-sink`, `ezsdr-sink-capture` | — | unchanged, except a `PrepareContext` literal in `ezsdr-sink-capture`'s tests (KB-1) |

## 6. Governance

OV-1…OV-23b and PO-1…PO-12 bind Phase 3 as they bound Phase 2. The rules below add what Phase 3's form of plan needs.

- **GV-1** Phase 3 rule ids are `CH-n` (spec 11), `KB-n` and `VB-n` (spec 12) and `GV-n`, protected by OV-1 and split by OV-2. A new rule in an existing series takes that series' next number (`RM-23`, `MR-31`…`MR-36`); an amended rule keeps its id (PO-1). *Process obligation.*
- **GV-2** KB-1's Kernel items cite existing rule ids (`RS-44a`, `MA-5a`) in their doc comments, so `kernel_surface`'s rule prefixes stay the nine of KA-20. *Checked by `kernel_surface`.*
- **GV-3** The implementer applies each patch of `patches/` with `git apply`, never retyping it, in the order of `20-implementation-plan.md`, after `git apply --check` succeeds. A patch that does not apply is a stop (plan §0.3). *Process obligation.*
- **GV-4** The implementer changes no test, no expected value and no spec text beyond what the patches change. The Phase 2 tests whose code the patches change are exactly: the six test `PrepareContext` literals of KB-1 (patch 01); `rm_01_*` and `rm_04_*` for 31 keys and `se_01_*` renamed for three checks (patch 02); `mr_01`, `mr_03`, `mr_13`, `mr_14`, `mr_16_burst_refusals`, `mr_16_repeat_*`, `mr_17_late_policy_outcomes`, `mr_18_a_cold_rate_change_starts_a_new_sample_clock`, `mr_18_a_cold_receive_change_before_t0_applies_at_t0`, `mr_18_a_cold_transmit_change_replaces_the_tracker`, `mr_19`, `mr_21`, `mr_22` and `mr_25_orderly_*` (patch 03, VB-3 and VB-4). A Phase 2 test that fails after a patch is a stop. *Process obligation.*
- **GV-5** Spec 12's amended text reaches `design/04…09` only through `patches/05-design-text.patch`, at Step 5. The Vision is not edited during Phase 3 (OV-6); its issues are collected at Step 7 from specs 11 and 12 and applied at Step X with the owner's approval. *Process obligation.*
- **GV-6** After the patches are applied, the implementer runs every mutation of `20-implementation-plan.md` Appendix C, which carries PO-12's obligation for Phase 3's refusals and rules, and records each as `mutation: <name>: killed` or stops. *Process obligation; exit criterion 7.*

## 7. Test strategy

- Each crate tests itself in isolation, as in Phase 2. The channel is tested in `crates/ezsdr-sim/tests/sim_channel.rs` against stub transmitters, without MockRadio. MockRadio on the channel is tested in `crates/ezsdr-mock-radio/tests/mock_channel.rs` with a `World` harness that shares one `ClockRegistry`, `ManualTimeAuthority`, `Medium` and input store among several Mocks, each with its own collector, Action queue and link.
- End-to-end behaviour is proved in `ezsdr-acceptance` with the real Modules and the coordinator.
- A Manifest is compared only through Phase 2's `determinism_projection`, and two Runs compared that way share one capture directory, because the directory is in the recorded BindingProfile (Phase 2 Review M N8).
- Sample values are compared exactly wherever every operation is exact (unit gains, zero phases, the `ideal` profile), and within 10⁻⁶ where a rotation or a decibel scaling is involved.

## 8. Traceability

### Vision §58 → Phase 3

| # | Test | Phase 3 carrier (acceptance crate unless named) | Remaining |
|---|---|---|---|
| 3 | Deterministic with a seed | `v58_03_channel_noise_reproduces_with_its_seed`, `v58_03_a_run_reproduces_from_its_own_manifest` (Phase 2's two remain) | Reactor decisions (Phase 5) |
| 8 | Two MockRadios through a SimulationChannel | `v58_08_two_mock_radios_communicate_through_the_channel` (the Spec carries no `sim.` key; the capture equals the waveform at −6 dB and 1 µs), `v58_08_a_session_hears_a_burst_from_its_first_sample_in_either_instance_order` (a Session of two radios: a burst submitted at the instant of the receiver's last sample of a block, and a stop mid-block, heard exactly and identically whichever radio is stepped first) | — |
| 12 | Block-size independence | `v58_12_the_channel_output_does_not_depend_on_block_lengths` (Phase 2's remains) | a Processor (Phase 10) |
| 14 | Environment portability | `v58_08_without_a_channel_the_same_spec_hears_nothing`: the same Spec with and without `sim.channel` (Phase 2's remains) | — |

The other rows of Phase 2's table are unchanged, and their Phase 2 carriers still pass after every patch.

### Vision §57 → Phase 3

| Statement | Carrier |
|---|---|
| "`sdr.tx.repeat(x); y = sdr.rx.capture(N)` … entirely in software", through a SimulationChannel whose first models are loopback, gain, delay and AWGN | `v57_a_software_loopback_session_captures_what_it_transmits` (a Session: `start_repeat` of a ramp and `capture` of 5 000 samples through a loopback path; the capture equals the repeated ramp, sample for sample) |

### Phase 2's deferrals → Phase 3

| Phase 2 item | Where Phase 2 deferred it | Phase 3 carrier |
|---|---|---|
| SimulationChannel: loopback, gain, delay, AWGN | 00-overview §3; spec 08 §7 | spec 11 CH-1…CH-11 |
| Transmit sample content reaching a receiver | 00-overview §3, Y8; spec 09 M4 | KB-1, MR-32 |
| MockRadio clipping at full scale (Vision §23) | 00-overview §3 | MR-36 |
| Random LO phase after an untimed retune | 00-overview §3; RM-9 | MR-34 |
| The channel replaces the test pattern | Y7; spec 09 M8 | MR-31, MR-35 |
| Gain and frequency acting on samples | spec 09 M9 | MR-33 |
| A delay-calibration default per profile (Vision §26) | spec 07 §8 | RM-23, MR-3 |
| Per-device drifting roots | Y11 ("Phase 3+") | deferred again (Z2) |
| §58 #8 | 00-overview §8 | `v58_08_*` |

### Vision issues

Collected from spec 11 §7 and spec 12 §3 at Step 7, applied at Step X (GV-5): eight items.

## 9. Sequencing and gates

| Step | What | Gate / check |
|---|---|---|
| P | The owner reads this file's §4 and §11, spec 11, spec 12 and `20-implementation-plan.md`; the patches are part of what is accepted | **Gate P**. Nothing is applied before it |
| 1 | `patches/01-kernel.patch` (KB-1, KB-2) | 553 tests on both toolchains; Clippy clean; `kernel_surface` reports 116 NEW / 292 |
| 2 | `patches/02-vocabularies.patch` (VB-1's keys, VB-2, spec 11) | 561 tests |
| 3 | `patches/03-mock-radio.patch` (VB-3…VB-8, the rig's version bump and medium) | 585 tests |
| 4 | `patches/04-acceptance.patch` (§8's carriers) | 592 tests |
| 5 | `patches/05-design-text.patch` (spec 12's text into `design/04…09`) | every link resolves |
| 6 | Appendix C's mutations (GV-6) | 83 of 83 killed |
| — | **Review C**: one adversarial pass (Opus, AGENTS.md §8) over the applied diff and specs 11–12, *running* the tests and mutations rather than reading only (Phase 2's lesson, `plan/phase2/implementation-notes.md`, "Post-acceptance dynamic review") | findings recorded in §11 with verdicts (OV-5) |
| 7 | Exit tables (PO-10) from Appendix B, `handoff.md`, Vision issues collected | **Gate X** |
| X | Spec 11 moves to `design/11-simulation-channel.md`; the Vision issues are applied with owner approval (OV-6) | links and `v3/` paths recheck |

A review pass's findings are recorded in §11 with a verdict, never silently applied (OV-5). Before Gate P, a finding that changes code or tests changes the patches, re-verified in a scratch copy as `20-implementation-plan.md`'s header describes. After the patches are applied, an accepted finding is fixed in the repository with its own test and mutation check (`prompts/fix-findings.txt`) and recorded in `implementation-notes.md`; the patches stay as the record of what Gate P accepted.

## 10. Exit criteria

1. Specs 11 and 12 accepted at Gate X, and every decision row — Z1–Z11 here, C1–C11 in spec 11, M11–M15 in spec 12 — has a verdict in §11.
2. Every Phase 3 rule — CH-1…CH-11, KB-1, KB-2, VB-1…VB-10, RM-23, MR-31…MR-36 and each amended rule — has an OV-3 disposition in `plan/phase3/exit-review/`, read from test bodies (PO-10), with no `GAP` and no `UNCERTAIN`.
3. `cargo test --workspace` passes on Rust 1.85.0 and on stable with 592 tests (602 after Review C's fixes, 604 after Gate X's) and no `#[ignore]`; `cargo +stable clippy --workspace --all-targets -- -D warnings` passes.
4. Every carrier of §8 exists and passes.
5. `kernel_surface` (116 NEW / 292 public items), `schema_freeze` and every Vocabulary freeze test pass; `SCHEMA_CHANGELOG.md` has the Phase 3 entry.
6. The Kernel's direct dependencies are still exactly four; `Cargo.lock` gains no package (PO-4).
7. Every mutation of Appendix C is killed (GV-6).
8. Every link in `design/` and `plan/` resolves, and the `v3/` path check of `handoff.md` §1 passes.

## 11. Decision log

Filled in at Gate P and after each review. One row per decision the owner confirmed or reversed.

| Decision | Gate | Verdict | Note |
|---|---|---|---|
| Z1–Z11 (§4) | P | **accepted** | owner, 2026-09-26, as recommended |
| Spec 11 decisions C1–C11 | P | **accepted** | owner, 2026-09-26, as recommended |
| Spec 12: KB-1, KB-2, VB-1…VB-8, M11–M15 | P | **accepted** | owner, 2026-09-26, as recommended; this includes VB-8's consequence on `ideal` and M15's split of RF behaviour, whose Vision conflict is spec 12 §3 issue 4 |
| The patches `patches/01…05` and `tools/mutations.json` (83 mutations) | P | **accepted** | owner, 2026-09-26; the record of what Gate P accepted, not edited afterwards (§9) |
| Planning reviews, passes 1–5 (`reviews/planning-reviews.md`) | P | **accepted** | owner, 2026-09-26, as triaged there: every finding applied except pass 4's P2-5, rejected with its reason |
| VB-9: MR-27's per-instance section names and MR-2's `id` shape (Review C's B1) | Review C | **accepted** | owner, 2026-09-26, during Review C; applied in `857a1c2`, Kernel unchanged ([reviews/review-c.md](reviews/review-c.md) §2) |
| Review C: B1, P1-1, P1-2, the test gaps and P2 nits | Review C | **closed** | by tests and text in `857a1c2`; record in [reviews/review-c.md](reviews/review-c.md) §2 and `implementation-notes.md` |
| Review C P2-1 … P2-9 | X | **accepted** | owner, 2026-09-26, as recommended in [reviews/review-c.md](reviews/review-c.md) §3: ceilings for P2-1, P2-3, P2-5; P2-2 fixed in the Kernel (KB-1's KC-9 amendment, `kb_01_b_…`); P2-4, P2-7 amended (spec 12 VB-10, `mr_25_a_stream_stop_…`); P2-8 package descriptions; P2-6 and P2-9 no action. 604 tests |
| Specs 11 and 12, Z1–Z11, C1–C11, M11–M15 (exit criterion 1) | X | **accepted** | owner, 2026-09-26, as recommended; Review C found them supported, M13 after P2-7's correction |
| Step X: spec 11 to `design/`, the eight Vision issues | X | **done** | 2026-09-26: [`design/11-simulation-channel.md`](../../design/11-simulation-channel.md); Vision §§8, 13, 15, 16, 23, 25, 26, 57 and a revision-history row ([vision-issues.md](vision-issues.md)) |
