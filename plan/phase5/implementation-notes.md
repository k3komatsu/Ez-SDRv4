# Phase 5 — implementation notes

What was run, in order, and what it showed. The plan is [`00-overview.md`](00-overview.md); spec 14 is [`14-native-executor.md`](14-native-executor.md) and spec 15 is [`15-amendments.md`](15-amendments.md).

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

`python3 plan/phase5/tools/mutate.py plan/phase5/tools/mutations.json <scratch>` — Phase 4's tool, pointed at Phase 5's list (20 mutations), run in a scratch copy with the shared review target directory.

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

## Reviews

Review F (Opus, AGENTS.md §8) — pending.
