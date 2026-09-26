# Phase 4 — implementation notes

What was run, in order, and what it showed. The plan is [`00-overview.md`](00-overview.md) and spec 13 is [`13-amendments.md`](13-amendments.md).

## Baseline (2026-09-26, `56b2212`)

`cargo +stable test --workspace`: 605 passed, 0 failed (Phase 3's 604 plus the one test the post-Gate-X refactor `b46b137` added to `ezsdr-hostmem`).

## Commits

| Step | Commit | What | Tests (stable) |
|---|---|---|---:|
| 0 | `ac1ba9d` | `00-overview.md`, spec 13, `vision-issues.md` | 605 |
| 1 | `4cbdf46` | KD-1: `step_until_quiescent` finishes the round; MA-30, CH-9, MR-30 text | 608 |
| 2 | `b9059cc` | VC-1, VC-2: RM-24, MR-37; `radio` 1.2.0, `ezsdr.radio.mock` 1.2.0 | 611 |
| 3–4 | `311f2e9` | VC-3: HD-15, `ezsdr.sigmf-ext.md`; `ezsdr.sink.capture` 1.1.0; the Session `Stop(sink/rec)` carrier | 617 |

## Tests changed under GW-3

Each change is forced by the rule named; no expected value moved for any other reason.

| Test | Change | Forced by |
|---|---|---|
| `mr_01_descriptor_registers` | pins 1.2.0, both vocabulary requirements and the implementation hash (it checked only the requirement count) | MR-1 (VC-2) |
| `mr_19_backpressure_is_an_overrun` | asserts the byte form and decodes the cause | MR-37 |
| `mr_20_faults_fire_at_their_instants` | reads `lost` through `RxOverflowPayload::from_payload` | MR-37 |
| `v58_04_mock_events_reach_counters_policy_and_manifest` | asserts the byte form and decodes the cause | MR-37 |
| `v58_06_injected_overflow_is_a_uhd_overflow` | compares the decoded payload instead of the object | MR-37 |
| `rm_01_register_adds_the_descriptor_the_check_and_the_kinds` | `radio` 1.2.0 | RM-1 (VC-1) |
| `hd_07_descriptor` | 1.1.0 and its implementation hash | HD-7 (VC-3) |
| the Mock test harnesses (`mock_radio.rs`, `mock_channel.rs`) and the acceptance rig | bind `ezsdr.radio.mock` 1.2.0 and `ezsdr.sink.capture` 1.1.0 | MR-2, HD-8 (a binding names the exact version) |

`RxOverflowPayload` gained `Copy` (all its fields are `Copy`); its schema is unchanged, which `rm_20_schema_freeze` confirms.

## Verification

| Check | Result |
|---|---|
| `cargo +stable test --workspace` | 617 passed, 0 failed (626 after the review fixes) |
| `cargo +1.85.0 test --workspace` | 617 passed, 0 failed (626 after the review fixes) |
| `#[ignore]` in `crates/` | none |
| `cargo +stable clippy --workspace --all-targets -- -D warnings` | clean |
| `kernel_surface` `ov_23b` | 116 NEW / 292 public items (unchanged, GW-2) |
| `schema_freeze`, `rm_20_schema_freeze`, `se_12_schema_freeze`, `hd_14_schema_freeze` | pass; no Kernel schema changed |
| Kernel direct dependencies | `schemars`, `serde`, `serde_json`, `sha2` |
| `Cargo.lock` | unchanged |
| `check_links.py` | every link resolves |

Per crate before the reviews: `ezsdr-kernel` 449, `ezsdr-radio` 12, `ezsdr-sim` 17, `ezsdr-sim-engine` 7, `ezsdr-hostmem` 3, `ezsdr-link-host` 2, `ezsdr-sink` 2, `ezsdr-sink-capture` 19, `ezsdr-mock-radio` 66, `ezsdr-acceptance` 40. After: `ezsdr-kernel` 452, `ezsdr-sink-capture` 24, `ezsdr-acceptance` 41, the others unchanged — 626. One schema file was added after Review D (`schemas/radio/rx_overflow_hot_payload.v1.json`), none changed.

## Mutations (GW-4)

`python3 plan/phase4/tools/mutate.py plan/phase4/tools/mutations.json <scratch>` — Phase 3's tool, pointed at Phase 4's list (`tools/mutations.json`, 34 after the reviews), run in a scratch copy with its own target directory.

| Mutation | Result |
|---|---|
| D01 | `mutation: KD-1 return at the first error: killed` |
| D02 | `mutation: KD-1 return at the first error, end to end: killed` |
| D03 | `mutation: KD-1 a failed instance is stepped again: killed` |
| D04 | `mutation: KD-1 the last error wins: killed` |
| D05 | `mutation: RM-24 cause bytes swapped: killed` |
| D06 | `mutation: RM-24 no length check: killed` |
| D07 | `mutation: RM-24 the byte array refused: killed` |
| D08 | `mutation: RM-24 the byte array refused, end to end: killed` |
| D09 | `mutation: MR-37 the injected overflow on the control path: killed` |
| D10 | `mutation: MR-37 the back-pressure overrun on the control path: killed` |
| D11 | `mutation: MR-1 radio requirement left at 1.1.0: killed` |
| D12 | `mutation: HD-15 global index written as the file index: killed` |
| D13 | `mutation: HD-15 global index written as the file index, end to end: killed` |
| D14 | `mutation: HD-15 the file index ignores earlier gaps: killed` |
| D15 | `mutation: HD-15 metadata for a capture with two maps: killed` |
| D16 | `mutation: HD-15 sc16 as cf32_le: killed` |
| D17 | `mutation: HD-15 no channel annotations: killed` |
| D18 | `mutation: HD-15 the Dataset keeps the raw extension: killed` |
| D19 | `mutation: HD-15 the Dataset keeps the raw extension, end to end: killed` |
| D20 | `mutation: HD-7 implementation hash left at 1.0.0: killed` |
| D21 | `mutation: KD-1 the cap's STEP_LIVELOCK beats a failure: killed` |
| D22 | `mutation: KD-1 the cap's STEP_LIVELOCK beats a failure, in the coordinator: killed` |
| D23 | `mutation: KD-1 only the first failure of a round is reported: killed` |
| D24 | `mutation: KD-1 only the first failure of a round is reported, end to end: killed` |
| D25 | `mutation: KD-1 the last failure decides: killed` |
| D26 | `mutation: HD-15 annotations not sorted: killed` |
| D27 | `mutation: HD-15 an empty run gets a capture segment: killed` |
| D28 | `mutation: HD-15 the Sink reports every capture as complete: killed` |
| D29 | `mutation: HD-15 a gap's lost written as its len: killed` |
| D30 | `mutation: HD-15 a gap's link_dropped written as 0: killed` |
| D31 | `mutation: HD-15 a negative file index clamped instead of refused: killed` |
| D32 | `mutation: HD-15 the staged metadata never renamed into place: killed` |
| D33 | `mutation: MR-37 the overflow emitted at info severity: killed` |
| D34 | `mutation: MR-19 the back-pressure overrun reports no loss: killed` |

34 of 34 killed: D01–D20 before the reviews (20 of 20), and all 34 again after the fixes, in a scratch copy of the fixed tree. D28 was first written as a change inside the `json!` macro that did not compile; it was rewritten to change the Sink's call (`partial` → `false`), which the tool then reported killed.

## Reviews D and E (2026-09-26)

Two adversarial reviews ran in parallel over `ac1ba9d..118067b`, each in its own scratch copy, neither touching the repository:

- **Review D** — Claude Opus (AGENTS.md §8), dynamic: both toolchains, the 20 mutations, 16 mutations of its own, probes through the acceptance rig, SigMF validation with `jsonschema` against the v1.2.6 schema and with sigmf-python 1.13.0, and the UHD source for `ERROR_CODE_ALIGNMENT`. Verdict **CHANGES_REQUIRED**: one P0, three P1, thirteen P2.
- **Review E** — OpenCode's SpaceBunny (`opencode-go/space-bunny-free`), dispatched at the owner's request through Orca orchestration (Run `run_bbf2ebbd626a`, worker `opencode2`) as a second model family, with the same brief. It ran both toolchains, the 20 mutations, 30 probes and the same external SigMF validation. Verdict **CHANGES_REQUIRED**: two P1, eight P2. Its report is `tmp/review-spacebunny/REPORT.md` (git-ignored).

Both validated every Recording they produced against the official SigMF v1.2.6 schema and read it with sigmf-python; both reproduced 617 tests on both toolchains and 20 of 20 mutations killed. They agreed on the two most important findings independently.

### Triage

| Finding | Reviews | Verdict | Disposition |
|---|---|---|---|
| **KD-1 lets the cap's `STEP_LIVELOCK` replace a Module error**: a failure in a round that then cannot quiesce ran 1 000 rounds and returned the Kernel's own livelock, so the Run's cause became `STEP_LIVELOCK` and `DEVICE_LOST` was lost. A regression: before KD-1 the loop returned at the error | E P1-1, D P2-4 | **fixed** | `step_until_quiescent` returns the first failure at the cap and raises no `STEP_LIVELOCK`. Tests `ma_30_a_failure_beats_the_step_livelock_cap` and, from Review E's probe, the coordinator's `kd_01_a_device_lost_is_not_reported_as_a_step_livelock`; mutations D21, D22 |
| **A second failure in one round was found and dropped**: KD-1 steps a second failing instance, but `guard_step` kept one error | D P1-1 | **fixed** | `guard_step` records every failure in stepping order; `round` acts on the first as before and emits `DEVICE_LOST` for every later `DeviceLost`. A later failure of another kind is not recorded (MA-30's text says so). Tests `kd_01_every_lost_device_of_a_round_is_reported_and_the_first_failure_decides` (coordinator) and the both-lost half of `kd_01_a_faulted_round_…` (acceptance); mutations D23–D25 |
| MA-30's paragraph overclaimed ("returns the first error"; "as if it had produced nothing") | D P2-4 | **fixed** | the text now states the cap, the later failures and "sees only what the failed instance published before it failed" |
| **K3 is reachable in Simulation**: a `start_lead_ns` off the sample grid moves a burst one receive sample late, with only `requested_target` to show it; §3 said Simulation could not reach it | D P0-1 | **fixed (the claim); owner decision (the behaviour)** | §3's row corrected and split from K2/K5/K6/K8; the behaviour pinned as a ceiling by `k3_an_off_grid_start_lead_moves_a_burst_to_the_next_transmit_sample` (10 001 on the grid, 10 002 off it). Not fixed here: the fix depends on Phase 7's transmit model. Owner decision recorded in §11 |
| **SC-32 contradicts HD-15**: "each valid segment" read per channel, while HD-15 and the code make one segment per run between stream gaps, and the exit-review draft repeated the wrong reading | D P1-2, E P1-2 | **fixed** | SC-32's text amended (spec 13 VC-3, `design/02`); exit-review row rewritten |
| HD-15 clauses no test pinned: the annotation sort, the empty-run guard, the partial capture's metadata, a gap's `lost` and `link_dropped`, the `ChannelsChanged` route, sc16 end to end | D P1-3, E P2-1, P2-2, P2-6 | **fixed** | `hd_15_gap_fields_carry_the_continuity_causes`, `hd_15_annotations_are_sorted_by_index_then_channel`, `hd_15_a_partial_capture_has_metadata`, `hd_15_an_sc16_capture_is_ci16_le`, the `ChannelsChanged` half of `hd_15_a_capture_across_a_clock_change_has_no_metadata`, and the partial capture's Recording in `v58_13_a_session_stop_…`; mutations D26–D30, D32 |
| `sigmf_meta` wrote invalid SigMF for a hand-built map (negative `core:sample_start`, wrapped extents) | E P2-3, P2-7 | **fixed** | checked arithmetic; such a map is an error naming HD-15. `hd_15_a_map_it_cannot_describe_is_refused`; mutation D31 |
| A failed metadata write dropped the capture's `ArtifactRef`, and the write was not atomic | D P2-10, E P2-4 | **fixed** | the `ArtifactRef` is recorded first; the metadata is written to a staging file and renamed |
| A partial capture's Recording looked complete to a SigMF reader | E P2-5 | **fixed** | `ezsdr:partial` in the global object and in `ezsdr.sigmf-ext.md` |
| MR-37's severity and MR-19's `lost` were unpinned | D P2-5 | **fixed** | `mr_37_…` asserts `warning` and the source; `mr_19_…` the whole payload; mutations D33, D34 |
| Readers validating `RX_OVERFLOW` payloads had no schema for the array | D P2-3 | **fixed** | `RxOverflowHotPayload` and `schemas/radio/rx_overflow_hot_payload.v1.json`, pinned by `rm_24_a_delivered_payload_reads_in_either_form`; `SCHEMA_CHANGELOG.md` entry; Q3 says what "additive" covers |
| The drain delivers ring bodies before control bodies, so the delivered order is by path and, when two kinds of one drain both stop the Run, the hot one is the cause | D P2-1 | **ceiling; owner decision** | recorded on RM-24; no Kernel change |
| A hot body the full ring drops marks no artifact | D P2-2 | **ceiling** | recorded on RM-24 |
| `TIME_ERROR` "has no fixed-size form" is false (26 bytes fit) | D P2-6 | **fixed** | §3's row keeps only the control-path reason |
| RM-11 said every payload is a JSON object; RM-22 that every payload is built with `to_value`; MUST or MAY for the hot path | D P2-7 | **fixed** | RM-11 says "must" and "on the control path"; RM-22 and RM-20 amended |
| GW-2's "checked by `kernel_surface`": `ov_23b` prints the count but asserts less | D P2-8 | **fixed** | GW-2 names the allow-list and the completeness check |
| HD-10's file-name sentence garbled | D P2-9 | **fixed** | |
| §2's evidence for hole 1 (`grep '\.emit('` cannot show `emit_control`) | E P2-8 | **fixed** | the citation names `emit_event` |
| Doc nits: D20's name, GW-3's vacuous sentence, the `rm_24` row | D P2-11 | **fixed** | |
| UHD's `ERROR_CODE_ALIGNMENT` behaviour upgraded from INFERRED to VERIFIED; SC-31a's per-channel `ALIGNMENT` may have no UHD producer | D P2-12 | **recorded** | §3, as a Phase 7 input |
| P2-3 re-marked to Phase 10 though `handoff.md` listed it; K2/K5/K6/K8 deferred while the Kernel freezes at v4.0; Q7 leaves the metadata unreferenced by the Manifest | D P2-13 | **owner decisions** | §11 |

### Mutations of the reviews that survived, now killed

Review D: R01 (`guard_step` keeps the last error) → D25; R03 (severity `Info`) → D33; R05 (back-pressure `lost` 0) → D34; R07, R08 (annotation order) → D26; R09 (no metadata for a partial capture) → D28, D32; R11, R12 (`lost`, `link_dropped`) → D29, D30. Review E: the annotation sort → D26; the empty-run guard → D27.
