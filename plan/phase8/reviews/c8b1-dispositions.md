# 8b first clean-up (c8b1) — dispositions

Note 52 §2d rows 12, 30 and 32 and the D1 trial's `ezsdr-radio` gaps (§4), done in the idle time before the 2026-11-27 bench (owner 2026-11-20). Risk: not risky. A Sonnet subagent implemented; GPT-6.1 Terra reviewed and re-checked ([c8b1-reviews.md](c8b1-reviews.md)). Briefs: [c8b1-brief.md](c8b1-brief.md). Gate: `c8b1-gate.json` (beside this file).

## Counts (source, severity, mechanism)

| Source | Major | Minor | Note | Mechanism |
|---|---|---|---|---|
| Terra, review | 2 | 0 | 0 | none |
| Terra, re-check | 0 | 0 | 1 | none |

## Dispositions
- Terra major 1 (adopted, orchestrator): row 30 is withdrawn. The take/restore of the TX header's `contract` is what keeps the TX path free of a `String` allocation per block, so it is not a no-op; `tx.rs` is back to its earlier code with one comment saying why.
- Terra major 2 (adopted): `Direct`'s `Drop` catches a cleanup panic only while the thread is already unwinding; on the normal path it fails the test. No normal-path test: making cleanup panic needs a production hook (decided by the orchestrator; Terra's re-check accepts it).

## Decisions (orchestrator)
- Row 12: `Direct` (`tests/fake.rs`) ends its Run on drop (stop and join, errors discarded), so a failing test leaks no threads into the binary's later tests.
- Row 32: `hw_m1`/`hw_m2` deleted — they moved a front end between streamers, the path of the withdrawn RM-34 (RM-21: channel membership has no runtime update class). Their results stay in `plan/phase8/bench-results.md`.
- D1 trial gaps: table tests in `crates/ezsdr-radio` (`Grid` edges, `decimation`, `FrontEnd::receives`, `uhd_name`, `addressed`, a one-frequency RF band) and rows C8B1-1..12. The sweep on those functions went from 27 missed of 138 to 2; both are `||` → `&&` in the early return of `decimation` and `clip_decimation` (`x3xx.rs`), equivalent: every input the early return refuses (NaN, ±∞, ≤ 0) gives the same result through the code after it (Terra checked `clip_decimation`, the orchestrator `decimation`).
- `tests/hardware.rs` is in D13's `DEFINES`, so the record suite on 2026-11-27 must be taken at a commit after this one.
- Gate run 1 (orchestrator): row C7-241 (bt2) survived here and was killed on `main`. Its test caught the mutant only through the resume rows' interval, whose width comes from host stamps, so whether the shifted lattice left the interval varied by run (INFERRED; survived twice here, killed on `main` and in the earlier gates). The test now also asserts the lattice's origin directly (`Trial::lattice` made public in `tests/common/c7.rs`); the row is killed.
