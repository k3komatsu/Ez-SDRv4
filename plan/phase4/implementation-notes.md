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
| `cargo +stable test --workspace` | 617 passed, 0 failed |
| `cargo +1.85.0 test --workspace` | 617 passed, 0 failed |
| `#[ignore]` in `crates/` | none |
| `cargo +stable clippy --workspace --all-targets -- -D warnings` | clean |
| `kernel_surface` `ov_23b` | 116 NEW / 292 public items (unchanged, GW-2) |
| `schema_freeze`, `rm_20_schema_freeze`, `se_12_schema_freeze`, `hd_14_schema_freeze` | pass, no schema file changed |
| Kernel direct dependencies | `schemars`, `serde`, `serde_json`, `sha2` |
| `Cargo.lock` | unchanged |
| `check_links.py` | every link resolves |

Per crate: `ezsdr-kernel` 449, `ezsdr-radio` 12, `ezsdr-sim` 17, `ezsdr-sim-engine` 7, `ezsdr-hostmem` 3, `ezsdr-link-host` 2, `ezsdr-sink` 2, `ezsdr-sink-capture` 19, `ezsdr-mock-radio` 66, `ezsdr-acceptance` 40.

## Mutations (GW-4)

`python3 plan/phase4/tools/mutate.py plan/phase4/tools/mutations.json <scratch>` — Phase 3's tool, pointed at Phase 4's list of 20 (`tools/mutations.json`), run in a scratch copy with its own target directory.

| Mutation | Result |
|---|---|
| D01 KD-1 return at the first error | `mutation: KD-1 return at the first error: killed` |
| D02 KD-1 return at the first error, end to end | `mutation: KD-1 return at the first error, end to end: killed` |
| D03 KD-1 a failed instance is stepped again | `mutation: KD-1 a failed instance is stepped again: killed` |
| D04 KD-1 the last error wins | `mutation: KD-1 the last error wins: killed` |
| D05 RM-24 cause bytes swapped | `mutation: RM-24 cause bytes swapped: killed` |
| D06 RM-24 no length check | `mutation: RM-24 no length check: killed` |
| D07 RM-24 the byte array refused | `mutation: RM-24 the byte array refused: killed` |
| D08 RM-24 the byte array refused, end to end | `mutation: RM-24 the byte array refused, end to end: killed` |
| D09 MR-37 the injected overflow on the control path | `mutation: MR-37 the injected overflow on the control path: killed` |
| D10 MR-37 the back-pressure overrun on the control path | `mutation: MR-37 the back-pressure overrun on the control path: killed` |
| D11 MR-1 radio requirement left at 1.1.0 | `mutation: MR-1 radio requirement left at 1.1.0: killed` |
| D12 HD-15 global index written as the file index | `mutation: HD-15 global index written as the file index: killed` |
| D13 HD-15 global index written as the file index, end to end | `mutation: HD-15 global index written as the file index, end to end: killed` |
| D14 HD-15 the file index ignores earlier gaps | `mutation: HD-15 the file index ignores earlier gaps: killed` |
| D15 HD-15 metadata for a capture with two maps | `mutation: HD-15 metadata for a capture with two maps: killed` |
| D16 HD-15 sc16 as cf32_le | `mutation: HD-15 sc16 as cf32_le: killed` |
| D17 HD-15 no channel annotations | `mutation: HD-15 no channel annotations: killed` |
| D18 HD-15 the Dataset keeps the raw extension | `mutation: HD-15 the Dataset keeps the raw extension: killed` |
| D19 HD-15 the Dataset keeps the raw extension, end to end | `mutation: HD-15 the Dataset keeps the raw extension, end to end: killed` |
| D20 HD-7 version left at 1.0.0 | `mutation: HD-7 version left at 1.0.0: killed` |

20 of 20 killed. Each test named in spec 13's Appendix A fails with its rule disabled; the end-to-end duplicates (D02, D08, D13, D19) show the acceptance carriers guard the same code as the unit tests.
