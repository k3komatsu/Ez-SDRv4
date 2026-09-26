# Phase 5 — implementation notes

What was run, in order, and what it showed. The plan is [`00-overview.md`](00-overview.md); spec 14 is [`design/14-native-executor.md`](../../design/14-native-executor.md) and spec 15 is [`15-amendments.md`](15-amendments.md).

## Baseline (2026-09-26, `0627839`)

`cargo +stable test --workspace`: 626 passed, 0 failed (Phase 4's count).

## The prototype

Before the plan, the Executor of spec 14 (then submitting through `actions_out` from each component), the responder and `experiments::ping_pong` were written in the working tree and run through the real coordinator with two MockRadios on a SimulationChannel. What it showed is the evidence column of `00-overview.md` §2:

| Run | Result |
|---|---|
| The PONG's bytes in `Assembly.inputs`, the Spec naming them nowhere but in the component's parameter | `Failed { prepare }`: `KC-12: fragment island_0: responder: the PONG waveform sha256:c94f…a3787 is not an input of this Run` (hole 1) |
| After KE-1: turnaround 5 ms, `x310-like`, 1 Msps | the responder radio's burst targets transmit tick 2 015 046 (receive sample 15 046); the pinger hears the PONG from sample 15 092 |
| turnaround 1 ms, `drop_and_flag` / `send_asap_and_flag` | `TIME_ERROR { late, drop }`, late by 2 954 000 ns, no PONG / PONG at 14 000 with `requested_target` 11 046 |
| turnaround 3 955 µs and 3 953 µs | targets 14 001 on time and 13 999 late by 1 000 ns: the lead is counted from the block's publication at 12 000 µs (MR-14) |
| a PING whose first sample reaches the responder in the orderly drain (PING at 25 960 µs, `advance_to` 25 990 µs, `finish`), the component returning the refusal as its error | `Stopped { client }` with `also: [Abort { "KC-30: island_0: responder: the PONG was refused: [… ezsdr.dispatch … RS-6: dispatch is frozen …]" }]` (hole 3). After the ABI change (components push Actions, the Executor submits, NX-6): `also` empty |

Hole 2 (KE-2) is from reading `coordinator/admission.rs`: its `TxBurst` branch never looked at the waveform. No Phase 1–4 test broke when the check was added.

## Commits

| Step | Commit | What | Tests (stable) |
|---|---|---|---:|
| 0 | `e13348c` | `00-overview.md`, specs 14 and 15, `vision-issues.md`, the mutation tool | 626 |
| 1 | `0140755` | KE-1…KE-4: `ExperimentSpec.inputs`, KC-9's loop, the store in `Shared`, admission's waveform check; `design/03`–`06` text; the Spec schema | 630 |
| 2 | `7c5f0dd` | spec 14: `ezsdr-exec-native`; the governance lists | 638 |
| 3 | `ec46527` | the responder, `experiments::ping_pong`, `rig::ping_pong_profile`, `tests/reactive.rs`, `v58_10` extended | 644 |

Each step was verified before its commit on 1.85.0 and on stable; Steps 1 and 2 in a scratch worktree holding only that step's files, because the working tree already held the next steps.

## Tests changed under GX-3

| Test | Change | Forced by |
|---|---|---|
| `ov_22_schema_freeze` (`schemas/experiment_spec.v1.json` regenerated) | the optional `inputs` property | SB-20a (KE-1) |
| `ma_03_no_module_crate_depends_on_another`, `po_11_no_hashmap_and_no_wall_clock_in_simulation_code` | the new crate in their lists; `ezsdr-acceptance` depends on it | spec 14 §5 (a new Module crate) |
| `v58_10_experiments_name_no_mock_type` | also scans `responder.rs`, with the Executor's crate an allowed import | GX-6 |

No expected value of a Phase 1–4 test moved.

## A finding while writing the carriers

`v58_03_reactor_decisions_reproduce_with_their_seed` first used the 5 ms turnaround with jitter on the responder radio, and with the radios' names swapped (another `SimRng` stream, so other block lengths) the PONG was dropped late by 317 µs: the block holding the PING's first sample ended at 13 362 µs, so the lead left was 1 684 µs. That is MR-12 and MR-17 working as specified — a jittered block holds up to 4 000 samples, and the device's delivery latency is part of what the lead is counted from, as on hardware — not a defect. The target did not move. The carriers that jitter use a 7 ms turnaround, which no block length makes late, and `00-overview.md` §7 records why.

## Verification

| Check | Result |
|---|---|
| `cargo +stable test --workspace` | 644 passed, 0 failed |
| `cargo +1.85.0 test --workspace` | 644 passed, 0 failed |
| `#[ignore]` in `crates/` | none |
| `cargo +stable clippy --workspace --all-targets -- -D warnings` | clean |
| `kernel_surface` `ov_23b` | 116 NEW / 292 public items (unchanged, GX-2) |
| `schema_freeze` | passes; `experiment_spec` gained `inputs` (SCHEMA_CHANGELOG.md, Phase 5 entry); no other schema changed |
| Kernel direct dependencies | `schemars`, `serde`, `serde_json`, `sha2` |
| `Cargo.lock` | one workspace member added (`ezsdr-exec-native 1.0.0`); no external package |
| `check_links.py` | every link resolves |

Per crate: `ezsdr-kernel` 456, `ezsdr-radio` 12, `ezsdr-sim` 17, `ezsdr-sim-engine` 7, `ezsdr-hostmem` 3, `ezsdr-link-host` 2, `ezsdr-sink` 2, `ezsdr-sink-capture` 24, `ezsdr-mock-radio` 66, `ezsdr-exec-native` 8, `ezsdr-acceptance` 47 — 644.

## Mutations (GX-4)

`python3 plan/phase5/tools/mutate.py plan/phase5/tools/mutations.json <scratch>` — Phase 4's tool, pointed at Phase 5's list (20 mutations; 27 after Review F, 29 after Review G), run in a scratch copy with the shared review target directory.

| Mutation | Result |
|---|---|
| E01 | `mutation: KE-1 the listed inputs are not verified or stored: killed` |
| E02 | `mutation: KE-1 the listed inputs are not verified or stored, end to end: killed` |
| E03 | `mutation: KE-1 a listed input's size is not checked: killed` |
| E04 | ``mutation: KE-1 `inputs` missing from the top-level set: killed`` |
| E05 | `mutation: KE-2 no waveform check at admission: killed` |
| E06 | `mutation: KE-2 the stored length is not compared: killed` |
| E07 | `mutation: NX-6 every refusal fails the step: killed` |
| E08 | `mutation: NX-6 every refusal fails the step, end to end: killed` |
| E09 | `mutation: NX-6 any Run-ending check in a refusal drops it: killed` |
| E10 | `mutation: NX-6 every refusal is dropped: killed` |
| E11 | `mutation: NX-3 the implementation hash is not compared: killed` |
| E12 | `mutation: NX-3 the implementation kind is not compared: killed` |
| E13 | `mutation: NX-4 a component receives every link end of the Island: killed` |
| E14 | `mutation: NX-5 components stepped in reverse id order: killed` |
| E15 | `mutation: NX-5 a pushed Action alone does not report progress: killed` |
| E16 | `mutation: NX-7 an Action in the queue is ignored: killed` |
| E17 | ``mutation: NX-8 `stop` returns at the first error: killed`` |
| E18 | `mutation: the responder answers at the block's first sample, not the PING's: killed` |
| E19 | `mutation: the responder answers every loud sample: killed` |
| E20 | `mutation: KE-1 the scheduled inputs are recorded before the listed ones: killed` |
| E21 | `mutation: the responder forgets its rearm state at each block: killed` (after Review F) |
| E22 | `mutation: KC-9 a partial or marked input is accepted: killed` (after Review F) |
| E23 | `mutation: KC-9 two inputs may share an id: killed` (after Review F) |
| E24 | `mutation: the drain never steps an Executor: killed` (after Review F) |
| E25 | `mutation: NX-8 cleanup keeps the queues: killed` (after Review F) |
| E26 | `mutation: the responder rounds a turnaround down: killed` (after Review F) |
| E27 | `mutation: KE-2 the stored length compared one way only: killed` (after Review F) |
| E28 | `mutation: KC-9 an input's continuity is not checked: killed` (after Review G) |
| E29 | `mutation: KC-9 the id rule compares listed inputs only: killed` (after Review G) |

After Review F the whole list (27) was run again with the timestamp fix of `mutate.py`, and after Review G the whole list (29): all killed.

## Review F

Opus, adversarial and dynamic (AGENTS.md §8), brief [`prompts/review-f.txt`](prompts/review-f.txt), report [`reviews/review-f.md`](reviews/review-f.md). Verdict **CHANGES_REQUIRED**: 2 P0, 4 P1, 11 P2, all three holes of `00-overview.md` §2 reproduced at `0627839`, the code's behaviour correct (644 on both toolchains, the 20 mutations killed). Every finding was taken; none was rejected.

| Finding | What was done | Where |
|---|---|---|
| P0-1 KC-9 kept "an entry no schedule entry references is not kept" | the store's sentence now names the listed inputs too | `design/06` KC-9; spec 15 KE-1 |
| P0-2 NX-7 against MA-24 and UC-2 | **KE-5** (text): MA-24 and UC-2 bind an Executor that applies Actions; one that applies none refuses them, failing its step; spec 14 §2's "only obligation with no producer" corrected | `design/05` MA-24, UC-2; `design/06` UC-2; spec 15 KE-5; spec 14 §2, NX-7 |
| P1-1 MA-14a's sentence too broad | restricted to Actions submitted from `step` | `design/05` MA-14a; spec 15 KE-3; spec 14 NX-6 |
| P1-2 §58 #9 on the wrong evidence | #9 rests on §58's minimal reactive test; its Remaining is a Reactor fed by an event or message edge (Phase 10, §11's owner decision); `ComponentKind::Reactor`'s doc comment follows §19, which regenerates two schema descriptions | `00-overview.md` §3, R1, §8; `module_api.rs`; `schemas/component_descriptor.v1.json`, `experiment_spec.v1.json`; `SCHEMA_CHANGELOG.md` |
| P1-3 state across a block boundary untested | `v58_12` gains the PING at 11 500 that straddles the boundary at 12 000 (one burst at 18 546); mutation E21 | `tests/reactive.rs`; Appendix A |
| P1-4 stale builds in the shared target directory | `mutate.py` copies with fresh timestamps; the brief uses `rsync -a --no-times`; AGENTS.md §7 says why | `tools/mutate.py`; `prompts/review-f.txt`; `AGENTS.md` |
| P2-1 R8's reason | "every stepped Module", and MA-14 for a hardware Provider | `design/04` RS-17; spec 15 KE-4; `00-overview.md` R8 |
| P2-2 NX-3's "nothing is built" | "that component is not built; those before it were, and stop and cleanup reach them" | spec 14 NX-3 |
| P2-3 Run-owned provenance in `Manifest.inputs` | KC-9 refuses an input with `partial`, marks or continuity, and two inputs of one id and two hashes; tests; mutations E22, E23 | `pipeline.rs`; `design/06` KC-9; `ke_01_a_declared_input_is_verified_as_a_scheduled_one_is` |
| P2-4 KE-2 one way | `ke_02` checks a declared size one too small as well; mutation E27 | `coordinator.rs` |
| P2-5 `v58_10` reads only `use` lines | every `ezsdr_…` crate path is checked, `pub use` is forbidden, and the misspelt `ezsdr_radio_mock` is `ezsdr_mock_radio` | `tests/v58.rs` |
| P2-6 `ke_03` made no decision | the test wraps the responder to count its decisions and asserts exactly one; mutation E24 (the drain never steps an Executor) | `tests/reactive.rs` |
| P2-7 NX-8's handles | `nx_08` asserts the queue and the submitter are the harness's alone after `cleanup`, and that an Action then reaches nothing; mutation E25 | `native_executor.rs` |
| P2-8 the turnaround's rounding | a 3 953.5 µs turnaround is rounded up to 14 000 and is on time; mutation E26 | `tests/reactive.rs` |
| P2-9 test tables | spec 14's and spec 15's tables now say what the tests do | specs 14 and 15 |
| P2-10 `plan/phase5/14-…` in `design/05` | the path change is part of Step X | `00-overview.md` §9; spec 15, "Vision issues found" |
| P2-11 the responder's limits | its doc comment says it reads channel 0 only, ignores validity, and counts its rearm in samples across a gap | `responder.rs` |

The reviewer's judgement of R4 — §22's "a Reactor that is too slow for the hardware fails in simulation" is not met as written — is now an owner decision in `00-overview.md` §11, with Vision issue 2 as the recommendation.

After the fixes: stable 644 passed, 1.85.0 644 passed, Clippy clean, `kernel_surface` 116 NEW / 292, `check_links.py` every link. The mutation list has 27 entries (E21–E27 added for the findings).

## Review G

The re-review of Review F's fixes (Opus), brief [`prompts/review-g.txt`](prompts/review-g.txt), report [`reviews/review-g.md`](reviews/review-g.md). Verdict **CHANGES_REQUIRED**: no P0, 4 P1, 9 P2 — two text defects the fixes introduced (G-1, G-2), two refusals the fixes left untested (G-3), and the build-hygiene fix incomplete (G-4). The code was correct (644 on both toolchains, 27/27 mutations). Every finding was taken.

| Finding | What was done | Where |
|---|---|---|
| G-1 UC-2's qualifier freed Providers and Sinks too; UC-1 and UC-3…UC-6 still bound the native Executor | UC-2's marker names every Provider and Sink and the Executors that apply Actions; UC-1's last clause carries the exception | `design/05`, `design/06` UC-1, UC-2; spec 15 KE-5 |
| G-2 KC-9's spliced sentence inverted "anything else is refused" | the new conditions are part of the list of requirements, citing KE-1 | `design/06` KC-9; spec 15 KE-1 |
| G-3 the `continuity` refusal and the id rule across listed and scheduled inputs untested | `ke_01_a_declared_input_is_verified_as_a_scheduled_one_is` adds an input with a `ContinuityMap`, and a listed input and a scheduled waveform of one id and two hashes; mutations E28, E29 | `coordinator.rs`; Appendix A |
| G-4 a later build in the shared directory is still reused | AGENTS.md §7: refresh the copy's timestamps before each build session, `--exclude .cargo`, never interleave | `AGENTS.md`; `prompts/review-g.txt` (a note) |
| P2-1, P2-2 | the table row's separator restored; §66 cited | `00-overview.md` |
| P2-3 | "lengthens, in time", and a PING wholly inside a gap is not heard | `responder.rs` |
| P2-4 | `v58_10` forbids `crate::` and `super::` in both files | `tests/v58.rs` |
| P2-5 | MA-24's first sentence says "applies it … or, if it applies no Action, refuses it"; its marker names `nx_07_…` | `design/05` MA-24; spec 15 KE-5 |
| P2-6 | spec 15's `Amends` row lists UC-1, UC-2 in both specs and both schema descriptions | spec 15 |
| P2-7 | `--exclude .cargo` | `AGENTS.md` §7 |
| P2-8 | KC-9's ceiling: a produced artifact is listed as an input without `partial`, `marks` and `continuity`, its hash linking it to the Manifest that produced it | `design/06` KC-9; spec 15 KE-1 |
| P2-9 | a component parameter that no Action may change is an owner decision before the freeze | `00-overview.md` §11; spec 15 KE-5 |

**No further review.** The owner's instruction was to re-review unless the fixes are small and low-risk. These are: text in five rules, two test cases, one test-of-the-test line and documentation, with the whole list of mutations run again. Stable 644 passed, 1.85.0 644 passed, Clippy clean, `check_links.py` every link, and the 29 mutations all killed (the table above).

