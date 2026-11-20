# D13 — dispositions and counts

D13 (owner 2026-11-13): only current-code, `load: none`, hardware records, one per (board, layout), taken at the stage-4 closing commit, are placed in `crates/ezsdr-radio/measurements/`; the rest stays in `bench/`. This step builds the placement tool `plan/phase8/tools/place_measurement.py` (used after the 2026-11-27 bench) and fixes what a dry run of the 2026-11-13 R1 record exposed. Risk: not risky. A Sonnet subagent implemented; GPT-6.1 Terra reviewed and re-checked ([d13-reviews.md](d13-reviews.md)). Briefs: [d13-brief.md](d13-brief.md). Gate: `d13-gate.json` (beside this file).

## Counts (source, severity, mechanism)

| Source | Major | Minor | Note | Mechanism |
|---|---|---|---|---|
| Terra, review | 3 | 0 | 0 | none |
| Terra, re-check 1 | 4 | 1 | 0 | none |
| Terra, re-check 2 | 3 | 0 | 0 | none — fixed in a last round without further review (orchestrator) |

## Decisions
- Owner 2026-11-20 「スループットの件はこれで OK」: `radio.perf.tx_bytes_per_s` is `NotCompared` (RM-35): the profile's 8e8 is the admission bound, sustained host-fed TX is KL-5's Ceiling, the measured value stays in the bench record as an observation. Found by the dry run: the R1 TX ladder stops at 100 MS/s (200 MS/s underflows host-fed), so every record would fail `profile ≤ measured`.
- `serde_json`'s `float_roundtrip` on `ezsdr-kernel` (all crates share it): some 17-digit floats were read 1 ulp off. User-visible, verified before the fix: Python `sample_rate=200e6/14` reached the server as `14285714.285714284`, was coerced to the grid point and recorded as a coercion with a `requested` the experimenter never wrote (N=14 of 21 points checked). Fixed with a Rust round-trip test over the X3xx grids and a Python test; no fixture, hash or schema changed. A rate the experimenter's own arithmetic puts 1 ulp off a grid point is #124 (8b).
- The placement tool's checks (orchestrator, after Terra's rounds): exact key set from `lib.rs` keys and `rule()`; current code = the files that shape a record (`DEFINES`, including the record-suite entry point in `tests/hardware.rs`) unchanged and committed between the record's commit and HEAD, replacing word and pattern matches of condition ids; raw `measurement.json` only (bench copies are marked and refused); every observation printed, any other than `not_compared_*` needs `--reviewed "<note>"`; numbers kept as written; a refused `--replace` leaves the old file intact. Bench copies keep their observations (the TX result stays in the bench record).
- Gate run 2 (orchestrator): row P823 survived — the test helper `observation_number` re-parsed numbers to keep them exact, which `float_roundtrip` already guarantees, so its mutant was equivalent. The helper was removed (`numeric_points_json` writes `requested`/`readback` directly), P823 deleted, and P833 retargeted to the `readback` field it pinned; gate run 3 passed.
