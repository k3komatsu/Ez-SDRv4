# Phase 4 — Events, failure, continuity, artifacts: overview and plan

| Field | Value |
|---|---|
| Status | **Planned and implemented under the owner's delegation** (2026-09-26: "plan Phase 4, and implement it too if that is faster than handing it to another session"); there was no separate Gate P. Reviews D and E closed; **Accepted at Gate X** (owner, 2026-09-26, every decision as recommended; §11); **Step X done** (two Vision issues applied). Phase 4 is complete. |
| Phase | Vision §67 Phase 4. Predecessor: Phase 3 (SimulationChannel and deterministic Runs; accepted at Gate X 2026-09-26). Successor: Phase 5 (Mini Reactive Radio). |
| Scope | What the accepted specs and Phases 1–3 left to "Phase 4", after checking each item against what now exists: the **first hot-path event layout** (RS-32a's withdrawal left it to the Vocabulary, D51 left it to MockRadio's first one); the **error round** Phase 3 left open (Review C P2-1); **SigMF** captures (SC-32, the only Phase 1 rule still marked forward to Phase 4); and one missing carrier, a Kernel-routed Session `Stop(sink/rec)` (Phase 2's named Gate X risk). |
| Not in scope | §3 lists it. In one line: no new fault kinds, no calibration artifacts, no artifact store, no clock drift, no hardware-only questions from the UHD spike. |
| Language | English, like Phases 1–3. |
| Location | The amendments are spec 13, [`13-amendments.md`](13-amendments.md), which stays here as the record. Their text is applied to `design/02`, `05`, `07`, `08`, `09`, `10` and `11` in the implementation commits (GW-5); the Vision issues were applied at Step X. |
| Modal verbs | "must" and "must not" are normative (OV-4a). |

Documents:

```text
plan/phase4/
  00-overview.md          this file: scope, decisions, crates, governance, traceability, sequencing, exit criteria, decision log
  13-amendments.md        spec 13: Kernel amendment KD-1; Vocabulary and Module amendments VC-1…VC-3;
                          the new rules RM-24, MR-37 and HD-15
  implementation-notes.md what was run, what the review found, what was fixed
  exit-review/            per-rule dispositions (PO-10)
  vision-issues.md        the Vision edits spec 13 needs, applied at Step X with the owner's approval
```

---

## 1. Why Phase 4 is small

Vision §67 names Phase 4 "Events / failure / continuity / artifacts". Most of that exists already:

| Area | Done before Phase 4 | Where |
|---|---|---|
| Events | Envelope, never-dropping counters, bounded ring, `EVENTS_DROPPED`, escalation flags, the Policy table; §58 #4 | Phase 1 RS-27…RS-36; Phase 2 KC-*, `v58_04_*` |
| Failure | Transactional cleanup with deadlines, panics contained (KC-30), `DEVICE_LOST`, three injected faults; §58 #5 | Phase 1 RS-6…RS-8a; Phase 2 SE-3, MR-20…MR-22, `v58_05_*` |
| Continuity | Stream Contract flags, `ContinuityBuilder`, gap causes, per-channel breaks; §58 #6 | Phase 1 SC-14…SC-31d; Phase 2 `v58_06_*` |
| Artifacts | Hashed captures with continuity and marks in a sealed Manifest; §58 #7 | Phase 2 HD-10, RS-38…RS-47, `v58_07_*` |

Phase 2 pulled fault injection forward (KA-22), and Phase 3 added a channel. What is left is four specific holes, each with evidence (§2). A Phase 4 that filled the title rather than the holes would be scope creep (AGENTS.md §6).

## 2. What is left, with evidence

| # | Hole | Evidence | Amendment |
|---|---|---|---|
| 1 | **No Module has ever emitted on the hot path.** Every radio event goes through `emit_control` (RM-11: "RS-32's hot path is the Phase 7 Provider's concern"), so §58 #4's "RuntimeEvents flow through the same path used by hardware" holds only from the collector onward. The UHD spike's hardware Provider emitted from its receive thread with `emit_control` too, because no hot-path layout exists (spike finding K10, [`plan/spikes/2026-09-26-uhd.md`](../spikes/2026-09-26-uhd.md)) — an allocating, locking call on the sample path, which Vision §29 forbids | `crates/ezsdr-mock-radio/src/lib.rs`: `emit_event`, the one path every Mock event took, calls `emit_control`; `grep -rn '\.emit(' crates` finds hot-path emission only in Kernel tests | VC-1 (RM-24), VC-2 (MR-37) |
| 2 | **A Module error cuts a stepping round short**, so which blocks a receiver published before a fault depends on fragment names. Phase 3 measured it (receiver `rx_blocks` 0 or 1 by name) and accepted a ceiling on CH-9 and MR-30 that names "Phase 4's failure work" as the owner | `crates/ezsdr-kernel/src/module_api.rs` `step_until_quiescent` returns at the first `?`; `plan/phase3/implementation-notes.md`, "P2-1, measured after B1"; `design/11-simulation-channel.md` CH-9 | KD-1 |
| 3 | **No SigMF writer.** SC-32 is the only Phase 1 rule whose marker still says "tested in Phase 4"; spec 10 H3 deferred SigMF to "Phase 4's SigMF Sink"; Vision §51 calls SigMF interoperability "a standard feature" | `plan/phase1/exit-review/02-stream-contract.md` SC-32 row; `design/10-host-data-path.md` H3, §9 | VC-3 (HD-15) |
| 4 | **No Kernel-routed test of a Session `Stop(sink/rec)`.** Named at Phase 2 Gate X; a probe showed it works, but no test pins it | `plan/phase2/implementation-notes.md`, "Post-acceptance dynamic review" | a carrier only (§8) |

## 3. Scope

### In scope

1. **KD-1** — MA-30's round finishes after a Module error: the failed instance is not stepped again at that instant, every other instance steps to quiescence, and the coordinator then acts on the first error in stepping order. CH-9's and MR-30's ceilings are removed.
2. **VC-1** — `radio` 1.2.0: RM-24, the hot-path representation of `radio.RX_OVERFLOW` (17 bytes) and its decoder, owned by the Vocabulary as RS-32a's withdrawal requires.
3. **VC-2** — `ezsdr.radio.mock` 1.2.0: MR-37, `RX_OVERFLOW` emitted on the hot path with RM-24's bytes.
4. **VC-3** — `ezsdr.sink.capture` 1.1.0: every capture is a SigMF Recording (HD-15): the data file is `.sigmf-data`, and a capture with one ContinuityMap gets a `.sigmf-meta` mapping each valid segment to a capture segment and carrying gaps and per-channel validity in an `ezsdr` extension (SC-32).
5. **Carriers** for §8, including the Session `Stop(sink/rec)` test.

### Out of scope, with the phase that owns each

| Item | Why not now | Owner |
|---|---|---|
| New fault kinds. Vision §17 lists twelve; three exist (SE-3). The others need a mechanism that does not exist yet: TX underflow needs a transmit source that can starve (a TX port); alignment failure needs to know what the UHD Provider does with `ERROR_CODE_ALIGNMENT` — UHD returns no samples, or only those already aligned, and discards packets whose timestamps disagree, never a partial channel set (VERIFIED by Review D in UHD master `0d7ed3b`: `rx_streamer_zero_copy.hpp`, `rx_streamer_impl.hpp`, `get_aligned_buffs.hpp`) — so defining the Mock's version now would fix a contract with no hardware behind it; clock loss needs a reference-lock sensor; deadline miss needs a Processor; peripheral timeout and queue overflow need Peripherals and TUN/TAP; a plugin crash is already contained (KC-30) and tested with doubles. Phase 2's "more fault kinds (Phase 4)" note on §58 #5 was a note, not a requirement: #5 is satisfied | each kind with its mechanism: TX underflow Phase 10; alignment and clock loss Phase 7; deadline miss Phase 10; peripheral timeout and TUN/TAP Phase 9 |
| The producer of SC-31a's `ALIGNMENT` flag | The same reason as alignment injection; the Kernel side (`ContinuityBuilder`, `ChannelGap`) is tested, and HD-15 maps channel gaps whatever produces them. Since UHD never delivers a partial channel set, SC-31a's per-channel `ALIGNMENT` may have no UHD producer at all — a Phase 7 input | Phase 7 |
| A CalibrationArtifact overriding path delays (Vision §26; Phase 3 §3) | No experiment or test consumes one; it would add artifact typing with no reader | the first phase with a calibration experiment |
| An artifact store beyond `mem:` and `file://` (Phase 2 Y12) | Its consumers are Session replay and child Runs, both Phase 6 | Phase 6 |
| Per-device drifting roots and clock drift (Phase 3 Z2) | A ClockRelation with uncertainty and resampling, for no Phase 4 test | the first phase that models drift |
| Kernel decoding of hot-path payloads | Phase 1 withdrew it (D51, RS-32a): "the Kernel does not interpret a Vocabulary's hot-path byte layout". Reversing that is the owner's call, not Phase 4's (§4 Q1) | — |
| Hot-path emission of `TIME_ERROR`, `LATE_COMMAND` and the other radio kinds | They are emitted while handling an Action or a start, which is control-path work | none |
| MA-8 enforcement by the Kernel (a worker thread with a join timeout for `prepare` and `arm`) — Phase 2's named Gate X risk | In Simulation every Module call is in-process and instant; the first Module whose calls can block on I/O is the UHD Provider, and the spike's K9 (an abandoned cleanup step loses the Provider's Manifest sections) belongs with it | Phase 7 |
| The spike's K2, K5, K6, K8, K11 | Each needs a Provider that is not stepped, which only hardware has; Simulation does not reach them (INFERRED: Session admission and the Provider's receipt happen at one simulated instant) | Phase 7 |
| The spike's K3: the transmit SampleClock starts at arm (MR-9), the receive one at T0 (MR-11), so the grids are out of step whenever T0 − arm is not a whole number of samples | **Reachable in Simulation** (Review D, P0-1): a `start_lead_ns` of 2 000 000 500 moves a burst one receive sample late with only `requested_target` to show it, pinned as a ceiling by `k3_an_off_grid_start_lead_moves_a_burst_to_the_next_transmit_sample`. Not fixed here because the fix depends on Phase 7's transmit model: rounding T0 onto the transmit grids at `plan` (Review D's proposal) assumes a transmit grid exists, while UHD starts a burst at any master-clock tick; and a Session's `cold` change starts a new transmit clock off the receive grid, which KC-15 cannot reach. **Owner decision before the Kernel freezes**: Phase 7, or a Kernel amendment now | Phase 7 (owner may pull forward) |
| P2-3 (a cold `radio.tx.channels` change writes `config` before `stop_tx`) | Its ceiling names the first consumer of transmit block headers, a TX port | Phase 10 |
| SigMF for a capture spanning a SampleClock change, and `core:frequency` / `core:datetime` | SigMF has one `core:sample_rate` and one `core:num_channels` per Recording; the capture Sink knows neither the RF frequency nor a UTC relation | a later Sink version when asked (HD-15's ceiling) |

## 4. Cross-cutting decisions

Each row is open to reversal at Gate X; a reversal is recorded in §11.

| # | Decision | Choice | Rejected (one line each) | Ceiling / upgrade path |
|---|---|---|---|---|
| Q1 | Who decodes a hot-path payload | The Vocabulary that owns the kind (RM-24's `RxOverflowPayload::from_payload`); the Kernel keeps draining the bytes as a JSON array of integers, as it does now | The Kernel decoding a declared layout (D51 withdrew `HotLayout` for exactly this: the Kernel would read Vocabulary content, OV-21); a decoder callback registered with the collector (a Kernel API for one kind, and not language-neutral) | A Manifest reader decodes `RX_OVERFLOW` through the radio Vocabulary; Phase 6's Python client does the same |
| Q2 | Which events MockRadio moves to the hot path | `RX_OVERFLOW` only, both causes and both producers (MR-19's back-pressure overrun and MR-21/MR-22's injected faults) | Every radio kind (the others are control-path work, §3); none, leaving it to Phase 7 as RM-11 said (then the first layout would be defined with hardware in the loop, where D51 wanted MockRadio's to become the contract, and §58 #4 would stay true only from the collector onward) | none |
| Q3 | `radio` version | 1.2.0: RM-24 is additive for Providers — the control-path object stays valid — and a reader has `from_payload` for both forms and a committed schema for the array (`rx_overflow_hot_payload`, added after Review D); a reader that validated `RX_OVERFLOW` payloads against `rx_overflow_payload` alone must now accept either | 2.0.0 (nothing is removed); 1.1.0 unchanged (a Manifest would name one version for two payload forms of the Mock) | none |
| Q4 | The error round (KD-1) | Finish the round: skip the failed instance, step the rest to quiescence, return the first error in stepping order | Keep Phase 3's ceiling (every later determinism claim would carry the exception); step the failed instance again (it has reported that it cannot run); stop after the current pass only (an instance that needs a second pass to consume what a peer published would still depend on order) | none |
| Q5 | SigMF form | The capture Sink writes every capture as a SigMF Recording: `.sigmf-data` always, `.sigmf-meta` when the capture has one ContinuityMap | A separate SigMF Sink Module (it would duplicate the capture logic, and H6's reason for a `sink` Vocabulary was a *different* sink needing the verb, not a second copy of this one); a `format` selector key (a SigMF dataset is the same interleaved bytes, so raw loses nothing but the extension, and §51 calls SigMF standard); a post-Run export function (a user would have to call it; the Sink already has the continuity and the rate) | HD-15's ceiling (§3) |
| Q6 | Where the SigMF mapping lives | `ezsdr_sink_capture::sigmf_meta`, a public pure function of one `ContinuityMap`, the rate and the contract, tested without a Run | In the `sink` Vocabulary crate (one user today; moving it is cheap when a second Sink needs it) | Move to `ezsdr-sink` when a second Sink writes SigMF |
| Q7 | The meta file and the Manifest | The Manifest's `ArtifactRef` stays one per capture, naming the data file; the meta file is not hashed in the Manifest, because it is a function of that `ArtifactRef`'s continuity and partial flag and the SampleClock's recorded rate, and SigMF's naming rule finds it beside the data | A second `ArtifactRef` per capture (two records for one Recording, and `marks` would apply to only one); embedding the meta in the Manifest (large data by value, §50) | none |
| Q8 | How Phase 4 is delivered | Planned and implemented in one session under the owner's delegation, commit by commit on `main`, then one dynamic adversarial review (Opus, AGENTS.md §8) that runs the tests and the mutations | Phase 3's verified patches and five planning reviews before Gate P (sized for a phase with two Kernel amendments and a new spec; Phase 4 has one ten-line Kernel change) | The owner can still reverse any row at Gate X; a reversal becomes a follow-up commit |

## 5. Crate layout

No crate is added and no dependency changes (PO-4, PO-8).

| Crate | Tier | Change |
|---|---|---|
| `ezsdr-kernel` | Kernel | KD-1: `step_until_quiescent` and the coordinator's `round`; its tests. No public item added or removed |
| `ezsdr-radio` | Vocabulary | 1.2.0: RM-24's `to_hot`, `from_hot`, `from_payload` on `RxOverflowPayload`; `RxOverflowHotPayload` and its schema `schemas/radio/rx_overflow_hot_payload.v1.json` (after Review D) |
| `ezsdr-mock-radio` | Module | 1.2.0: MR-37 (the handle resolved in `prepare`, the hot-path emission); vocabulary requirement `radio ^1.2.0` |
| `ezsdr-sink-capture` | Module | 1.1.0: HD-15 (`sigmf_meta`, `.sigmf-data`, `.sigmf-meta`); `ezsdr.sigmf-ext.md`, the extension's definition file that SigMF requires |
| `ezsdr-acceptance` | tests | the rig's versions; §8's carriers |
| the others | — | unchanged |

## 6. Governance

OV-1…OV-23b, PO-1…PO-12 and Phase 3's lessons bind Phase 4. The rules below add what its form of delivery needs.

- **GW-1** Phase 4 rule ids are `KD-n` (Kernel amendments), `VC-n` (Vocabulary and Module amendments) and `GW-n`, protected by OV-1. New rules in existing series take the next number: `RM-24`, `MR-37`, `HD-15`. An amended rule keeps its id (PO-1). *Process obligation.*
- **GW-2** The Kernel changes only as KD-1 says. No Kernel public item is added or removed and no Kernel schema changes. *Checked by `kernel_surface` (the allow-list is unchanged and its completeness check passes; `ov_23b` prints 116 NEW / 292 public items) and `schema_freeze`.*
- **GW-3** A Phase 1–3 test changes only where spec 13 changes what it observes, and each such change is listed in `implementation-notes.md` with the rule that forces it. *Process obligation.*
- **GW-4** Every new or amended rule has a test that fails when the rule's code is disabled, shown by a recorded mutation (Appendix A of `13-amendments.md`), as PO-12 requires. *Process obligation; exit criterion 6.*
- **GW-5** Spec 13's text reaches `design/` in the same commit as the code it describes; the Vision is not edited before Gate X (OV-6). *Process obligation.*

## 7. Test strategy

- Kernel: KD-1 against test instances in `crates/ezsdr-kernel/tests/module_api.rs`.
- Radio: RM-24's round trip and both payload forms in `crates/ezsdr-radio/tests/radio_model.rs`.
- MockRadio: MR-37 proves the path, not only the payload: an `EventCollector` with a one-slot ring and two overflows between drains must count two, deliver one and report one `EVENTS_DROPPED` — the control path never drops, so a Mock that still used it would deliver both.
- Capture Sink: HD-15 against blocks built by hand, so channel gaps with and without `ALIGNMENT` can be produced without a producer that sets the flag.
- End to end in `ezsdr-acceptance` with the real Modules.

## 8. Traceability

### Vision §58 → Phase 4

| # | Test | Phase 4 carrier (acceptance crate unless named) | Remaining |
|---|---|---|---|
| 3 | Deterministic with a seed | `kd_01_a_faulted_round_does_not_depend_on_fragment_names` (the Phase 3 probe, now a test: the receiver's output is the same whether it sorts before or after the transmitter reporting `device_lost`, and two lost devices are both reported in either order) | Reactor decisions (Phase 5) |
| 4 | Events flow through the hardware path | `v58_04_mock_events_reach_counters_policy_and_manifest` (the delivered payload is RM-24's byte array and decodes to the overflow), `mr_37_the_overflow_travels_the_hot_path` (MockRadio crate) | — |
| 5 | Fault injection triggers cleanup and policy | `v58_05_*` unchanged; the coordinator's `kd_01_a_device_lost_is_not_reported_as_a_step_livelock` and `kd_01_every_lost_device_of_a_round_is_reported_and_the_first_failure_decides` (Kernel crate) | fault kinds with their mechanisms (§3) |
| 6 | Gaps equal a UHD overflow | `v58_06_injected_overflow_is_a_uhd_overflow` (the payload decoded through RM-24) | Phase 8 re-measures |
| 7 | Runs record Spec, Binding, plan, events, artifacts | `v51_an_overflowed_capture_is_a_sigmf_recording` (the capture's `.sigmf-meta` has two capture segments around the overflow's gap, with `core:global_index` jumping by the gap) | — |
| 13 | Sessions leave provenance | `v58_13_a_session_stop_of_the_recorder_keeps_a_partial_capture` (a Kernel-routed `Stop(sink/rec)` mid-capture gives a partial artifact, a Recording marked `ezsdr:partial`, and a later `capture` is served) | Python (Phase 6) |
| — | (a ceiling, not a §58 row) | `k3_an_off_grid_start_lead_moves_a_burst_to_the_next_transmit_sample` pins the spike's K3 as Simulation shows it (§3) | Phase 7, or the owner's decision |

### Earlier deferrals → Phase 4

| Item | Where it was deferred | Phase 4 disposition |
|---|---|---|
| SC-32, SigMF export | Phase 1 marker; spec 10 H3, §9 | HD-15 |
| MockRadio's first hot-path layout | Phase 1 D51; spec 07 RM-11; spec 09 §8 ("the hot-path event layout") | RM-24, MR-37 |
| The error round | Phase 3 Review C P2-1; CH-9's and MR-30's ceilings | KD-1; ceilings removed |
| Session `Stop(sink/rec)` through the Kernel | Phase 2 Gate X named risk | `v58_13_a_session_stop_…` |
| More fault kinds | Phase 2 §8 note; spec 08 S1 and §7 | re-marked per kind (§3) |
| `ALIGNMENT` injection | spec 09 §8 | Phase 7 (§3) |
| CalibrationArtifact | Phase 3 §3 | re-marked (§3) |
| Artifact store | Phase 2 Y12 | Phase 6 (§3) |
| Drift | Phase 3 Z2 | re-marked (§3) |
| MA-8 enforcement | Phase 2 Gate X named risk | Phase 7 (§3) |

## 9. Sequencing

| Step | What | Check |
|---|---|---|
| 0 | This file and spec 13 committed | links resolve |
| 1 | KD-1 | workspace tests on 1.85.0 and stable; Clippy; `kernel_surface` 116 / 292 |
| 2 | VC-1, VC-2 | the same |
| 3 | VC-3 | the same |
| 4 | §8's acceptance carriers | the same |
| 5 | Appendix A's mutations | each killed |
| — | **Review D**: one adversarial pass (Opus) over the diff and spec 13, running the tests and the mutations; **Review E**, in parallel at the owner's request, the same brief given to OpenCode's SpaceBunny through Orca orchestration (a second model family) | findings triaged in `implementation-notes.md`, fixed with tests, recorded in §11 (OV-5) |
| 6 | Exit tables, Vision issues collected, `handoff.md` | **Gate X** (owner) |
| X | Vision issues applied with the owner's approval | links and `v3/` paths recheck |

## 10. Exit criteria

1. Spec 13 accepted at Gate X, and every decision row — Q1–Q8 here, and the review's owner decisions — has a verdict in §11.
2. Every Phase 4 rule — KD-1, VC-1…VC-3, RM-24, MR-37, HD-15 and each amended rule — has an OV-3 disposition in `plan/phase4/exit-review/`, read from test bodies (PO-10), with no `GAP` and no `UNCERTAIN`.
3. `cargo test --workspace` passes on Rust 1.85.0 and on stable with no `#[ignore]`; `cargo +stable clippy --workspace --all-targets -- -D warnings` passes.
4. Every carrier of §8 exists and passes.
5. `kernel_surface` (116 NEW / 292 public items), `schema_freeze` and every Vocabulary freeze test pass.
6. Every mutation of spec 13's Appendix A is killed (GW-4).
7. The Kernel's direct dependencies are still exactly four; `Cargo.lock` gains no package (PO-4).
8. Every link in `design/` and `plan/` resolves, and the `v3/` path check of `handoff.md` §1 passes.

## 11. Decision log

| Decision | Gate | Verdict | Note |
|---|---|---|---|
| Plan and implement in one session (Q8) | — | **delegated** | owner, 2026-09-26: "pushしてphase4の計画を立ててください．…あなたが実装したほうがはやいなら計画を立てた後に実装まで進んでください" |
| Reviews D and E: every defect fixed with a test (`implementation-notes.md`, "Reviews D and E") | — | **closed** | 2026-09-26, before Gate X |
| K3 — Phase 7, or a Kernel amendment now (§3) | X | **Phase 7** | owner, 2026-09-26, as recommended: fixed with Phase 7's transmit model; the ceiling test stays until then |
| The drain order and dropped-body marks as RM-24 ceilings rather than a Kernel change (Review D P2-1, P2-2) | X | **ceilings** | owner, 2026-09-26, as recommended |
| P2-3 left with the first transmit-header consumer (Phase 10), although `handoff.md` listed it for Phase 4 | X | **Phase 10** | owner, 2026-09-26, as recommended |
| Q1–Q8, spec 13 (exit criterion 1) | X | **accepted** | owner, 2026-09-26, as recommended ("すべて推奨で受理します") |
| Step X: the two Vision issues | X | **done** | 2026-09-26: §§28, 29, 51 and a revision-history row ([vision-issues.md](vision-issues.md)) |
