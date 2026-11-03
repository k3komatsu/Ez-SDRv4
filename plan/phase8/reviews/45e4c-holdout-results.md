# E4c Part 2: the holdout run

Setup. Rows `holdout-rows.json` (16 scored: N01-N04, N06, N08-N10, N12-N18, N20; N01, N08, N10, N18 are Sol's R01, R08, R10, R18), rulings `rulings.md`. Each row in its own out-of-tree copy (`~/.cache/ezsdr-fixes/e4ccopy/w_<host>`, fresh timestamps, rsynced with `rdev.sync`, run in `ezsdr-v4-dev:uhd4.10` through `rdev.remote_argv`; `run_row.py` calls those two functions because rdev itself wants a git checkout), one job per host at a time, sim02 and sim03 in parallel. Per row three commands, logs `logs/<row>/{suite,dst-walks,dst-micro}.log`: (1) `cargo test -p ezsdr-radio-uhd --lib brain::` (debug, default suite); (2) release, `EZSDR_SEEDS=1-10000`, `--ignored _walks --skip s1c_whole_walks_wide --test-threads=26` (25 sweeps); (3) release, `--ignored _micro_full --test-threads=10` (24 searches, each within its internal 2,000,000-state / wall budget; a budget overrun passes with a report). **Deviation from E3:** walks and micro run as two commands, because one command with 49 tests at 26 threads was killed by SIGKILL on the unmutated tree (first attempt, discarded and rerun; no result of it is used). Baseline (BASE, sim02): suite 456 pass; walks 25 pass; micro 24 pass.
Scoring (E3 rulings unchanged): found = an oracle, a progress check or an `Incomplete`/modelled-program failure fails in a walk sweep, a micro search, or a default-suite case or harness test. Not counted: a unit test of a cell/brain/bounds procedure alone, a thread or conformance test, an observer panic, a witness-only failure.

| id | verdict | first failure (test, invariant) | walk seed / trace | wall suite+walks+micro (s), host |
|---|---|---|---|---|
| N01 (R01) | **found (default suite only)**: P6a in `oracle::tests::oracle_happy_path_passes` (micro explorer on the happy world, 37 events, 594 states); the sweeps and `_micro_full` searches are clean; `cell::u01c` also fails (unit) | P6a | - | 22+25+268, sim02 |
| N02 | **not found** (unit only: `cell::u05a`); DST clean | - | - | 28+24+271, sim03 |
| N03 | **not found** (unit only: `core::u03`); DST clean | - | - | 28+25+270, sim02 |
| N04 | **not found** (units `u07a-c`, `publish_write_rechecks_what_compute_read`; the one DST failure is `case3_..._walks` Vacuous: declared witness W3b never reached, witness-only) | - | - | 27+24+271, sim03 |
| N05, N07, N11, N19 | **not found, no row** (ruling) | - | - | - |
| N06 | **found** | `case8_lifecycle_across_loss_walks` I14 lifecycle admission after F (12 seeds); also thread-test and harness failures | case 8 seed 1163, 75 ev | 28+25+265, sim02 |
| N08 (R08) | **found** | `seeded_config_walks` I3 (107 seeds); default suite `seeded_config_reduced` I3 | seeded seed 61, 90 ev | 28+49+262, sim03 |
| N09 | **found** | I4 second disposition: micro case 2 (40 ev, 50 states), 7a, 8, 12a, 13 loss; walks cases 2, 7a, 8, 12a, 13 loss, seeded (1,604-4,557 seeds each) | case 2 seed 1, 88 ev | 27+772+205, sim02 |
| N10 (R10) | **not found** (suite, walks, micro all clean) | - | - | 28+24+273, sim03 |
| N12 | **found** | I11 value reserved with K reached: micro case 11 (12 ev, 12 states), walks case 11 (3,711 seeds), seeded | case 11 seed 3, 7 ev | 28+76+264, sim02 |
| N13 | **found** | I11: walks cases 10, 11, 12a, seeded (112-2,418 seeds); micro clean | case 10 seed 2, 16 ev | 27+28+267, sim03 |
| N14 | **found** | I11: micro case 11 (29 ev, 13,133 states), walks cases 10, 11, seeded | case 11 seed 52, 34 ev (case 10 seed 67, 51 ev) | 28+37+265, sim02 |
| N15 | **not found** (unit only: `bounds_tests::b7`); DST clean | - | - | 28+24+271, sim03 |
| N16 | **found** | P2 released value with 0 dispositions: walks in 21 of 25 sweeps; micro cases 4 (59 ev, 59 states), 10, 11, 2, 7a, 14 | case 7a seed 163, 120 ev (case 11 seed 13, 12 ev) | 14+199+241, sim02 |
| N17 | **found** | B8 (capacity timer inert): micro case 11 (39 ev, 587,603 states); walks 12a (161 seeds), 11, seeded | case 12a seed 119, 35 ev | 28+549+251, sim03 |
| N18 (R18) | **found by one micro search** | `case2_loss_everywhere_micro_full` I4 value reported (Loss) although qualifying evidence exists (48 ev, 457,991 states); suite and all walks clean | - | 32+24+265, sim02 |
| N20 | **found** (explorer verdict `Incomplete`: waiting at the end of the epilogue, teardown; not a named invariant) | cases 15a-d: all four micro searches and all four walk sweeps (10,000 seeds fail); default suite 15a-d micro and `b14` tests | case 15a-c seed 1 | 28+124+257, sim03 |
| N07d (diag) | found by I19 (case 13 loss/panic, 15d: micro and walks); not scored | I19 | case 13 loss seed 1, 97 ev | 28+157+264, sim03 |

**Score: 11 of 20 strictly** (found: N01, N06, N08, N09, N12, N13, N14, N16, N17, N18, N20); **11 of 19** without N05. If N01 (default-suite oracle test only) is not accepted: 10 of 20 / 10 of 19. Criterion (at least 16 of 20): **not met**.

Not found, and why the DST misses each one:
- N02: no oracle or case sees the Stopping transition's returned obligation set: the brain never uses it (obligations are re-read from cell states), so only `u05a` pins it. Missing oracle (and a dead return value).
- N03: the equal-instant value `e = C` at a shortened cut: no case separates `e >= C` from `e > C` (a value queued exactly at the new cut); only `u03` pins it. Missing case.
- N04: the translation validates the epoch only, but the DST computes and writes a publish in one event from the live cut, so the compute-commit race does not exist in the world; case 3 only loses its W3b witness. Missing interleaving (Sol's check says so).
- N10: R10 renews the drain timer on empty receives, but the timer is replaced before it is delivered and P1 claims nothing for an undelivered deadline, and no case lets the stream wait for the cut long enough to see the lost stop (INFERRED from the clean run). Missing oracle/case.
- N15: a reported left value is the only residual D could count; no case reserves into a chain at D with a left value present (D = 64 and the cases hold few values), so only `b7` pins it. Missing case.
- N05, N07, N11, N19: not modelled or not a model-visible error (rulings).
