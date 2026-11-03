# E3 Part 2: the holdout run

Setup. Each row in its own out-of-tree copy (`~/.cache/ezsdr-fixes/e3copy/w_<host>`, rsynced with `rdev.sync` and run in `ezsdr-v4-dev:uhd4.10` through `rdev.remote_argv`; rdev itself needs a git checkout, so `run_row.py` calls its two functions on the copy). One job per host at a time, sim02 and sim03 in parallel. Per row: (1) `cargo test -p ezsdr-radio-uhd --lib brain::` (debug, the default suite); (2) release, `EZSDR_SEEDS=1-10000`, `--ignored _walks _micro_full --skip s1c_whole_walks_wide` (13 walk sweeps + 11 micro searches; case 6 first-step micro is in the default suite). `s1c_whole_walks_wide` is a thread-conformance test that also matches `_walks`; it fails on the unmutated tree (baseline, first run) and counts for nothing, so it was skipped. Baseline (unmutated, `logs/BASE`): suite 362 passed; DST 25 passed, on sim03 (suite 23 s, DST 169 s; sim02 gave the same in the first run). Raw per-test output: `holdout-results-raw.md`; logs `logs/<row>/{suite,dst}.log`; rows and translations `holdout-rows.json`, `holdout-translation.md`. Failing walks print the shrunk trace; "trace" below is its length in events; micro "states" is the explored count at the violation.

Scoring (rulings.md): found = an oracle (I1–I16), a progress check (P1–P5) or a modelled-program failure fails in a walk sweep or a micro search, or in a default-suite case or harness test. Not counted: a unit test of a cell or brain procedure alone (listed as "unit only"), a thread or conformance test, an observer panic, a failure that is only an unreached witness. No witness-only failure and no thread-only failure occurred in any row.

| id | verdict | first oracle failure (test, invariant) | walk seed / trace | wall (suite+DST, host) |
|---|---|---|---|---|
| H01 (HO01) | **found** (I2); also 10 observer panics (`rules.rs:595` `reserved()` unreachable!) recorded separately; HO01b (observer made tolerant) found by I7/I2 in 11 tests | `case9_pending_move_partial_refused_walks`: I2 reserved outside its conditions (109 seeds); `seeded_config_walks` I2 (1 seed) | case 9 seed 34, 14 ev | 27+169 s, sim03 (HO01b: 75+236 s, sim02) |
| H02 | **found** | `case8_lifecycle_across_loss_walks` I2 (4,531 seeds); seeded I2 (122) | case 8 seed 1, 11 ev | 21+168 s, sim02 |
| H03 | **not found** (unit only: `cell::u02g_progress_at_cut_refuses`) | DST clean | - | 21+167 s, sim03 |
| H04 | **found** | micro case 4: P2 value 2 has 0 dispositions (65 ev, 1,291 states); walks P2 (cases 3, 4, 7b late, 7c) and I15 (7b, 9, 5) | case 4 seed 493, 139 ev | 17+196 s, sim02 |
| H05 | **not found** (suite and DST clean) | - | - | 21+168 s, sim03 |
| H06 | **found** | I5 (cut below published progress) in 9 walk sweeps; micro cases 1 and 6: I5 (120,159 and 364,916 states) | case 1 seed 32, 23 ev | 46+224 s, sim02 |
| H07 (R07) | **found** | micro case 3: I6 ack before the old epoch's recv published (32 states, 32 ev); walks case 3 (6,774 seeds), seeded | case 3 seed 2, 16 ev | 23+1,834 s, sim03 |
| H08 (R08) | **found** | `case3_plan_cancel_recv_stop_walks` I6 older epoch published past its retiring cut (14 seeds); seeded (113) | case 3 seed 726, 52 ev | 52+223 s, sim02 |
| H09 | **found** | I5 in 7 walk sweeps (cases 1, 3, 6, 7b, 7b late, 9, seeded) | case 1 seed 38, 40 ev | 43+222 s, sim03 |
| H10 | **not found: not modelled** (DST clean). The default suite fails `cases::tests::harness_loss_publish_releases_l_first_bounded` ("Settings marks the loss before its ungate"), a case test of the publication order, not an oracle | - | - | 69+237 s, sim03 |
| H11 | **found** | I1 ordinary call after a check that saw F set: micro cases 2, 7a, 8, 6 (stop); walks cases 2 (9,791 seeds), 7a (9,999), 6, 8, seeded | case 7a seed 1, 28 ev | 61+1,839 s, sim02 |
| H12 | **not found: not modelled** (suite and DST clean; no oracle reads `marked`) | - | - | 59+232 s, sim02 |
| H13 (R13) | **found** | I3 pinned-value issue: `case7b_flush_final_stop_restore_walks` (2 seeds), `case9_..._walks` (349), seeded (393); I15 case 5 (1) | case 9 seed 19, 9 ev | 77+236 s, sim02 |
| H14 | **found** | P2 0 dispositions in all 11 micro searches and all 13 walk sweeps | case 7c seed 1, 103 ev | 49+1,760 s, sim03 |
| H15 | **found** | micro cases 7b, 7b late, 5: I15 restore while debt (42, 45, 49 ev); walks case 4 (877 seeds) and seeded: I4 evidence without application | case 4 seed 15, 59 ev | 59+425 s, sim02 |
| H16 | **found** | P2 0 dispositions: 9 micro searches (cases 1, 2, 5, 7a-c, 8, 9, 4) and 9 walk sweeps | case 8 seed 9, 144 ev | 35+352 s, sim03 |
| H17 | **not found: no translation** | - | - | - |
| H18 | **not found** (unit only: `core::u14_*`); DST clean: no case holds more than the bound (4) values in flight, so a full cell is never met | - | - | 74+231 s, sim02 |
| H19 | **found** | I4 value received as evidence without the device application: 9 micro searches, 12 walk sweeps | case 6 micro 29 ev (1,096 states); walk case 8 seed 4021, 32 ev | 32+582 s, sim03 |
| H20 (R20) | **found** | `case4_equal_time_pair_stopping_walks` I4 (3,323 seeds); seeded (1,552); no micro search finds it | case 4 seed 4, 48 ev | 57+1,118 s, sim03 |

Diagnostics: HO13a (`pinned` only): suite unit tests and `oracle::inv_i3_restore_while_pinned_is_violation` fail, DST clean (the start-next and drain-stop decisions have no oracle). HO13b (`committed` only): found by I3 (walks 7b, 9, seeded) and I15 (case 5): the I3 evidence of H13 comes from the restore, move, close, open decisions. Not scored.

**Score: 14 of 20** (found: H01, H02, H04, H06, H07, H08, H09, H11, H13, H14, H15, H16, H19, H20). Criterion (at least 16): **not met**. Over the 17 modelled errors: 14 of 17 (H10, H12, H17 excluded by ruling).

Not found, and why the DST misses each one:
- H03: redundant guard in the model. `publish_planned` advances progress and, when it reaches the cut, calls `stop_accepting` in the same section (cell.rs), so a value is refused for the phase before `progress >= cut` is ever consulted; only the unit test pins the second guard. No case can separate the two.
- H05: missing oracle. The reopened segment accepts values that satisfy I2 (`e < cut`, same segment), and I7 reads the phase before the step; no invariant says a Stopping phase stays closed without an authorized start. A delayed first-sample after a stop decision is reachable in the walks.
- H18: missing case. Every case keeps at most the bound (4) values in flight; capacity plus one with a later free is not scripted (the holdout names it as needed).
- H10, H12, H17: not modelled (rulings).

Notes on side effects: H06's row loses the W1c witness at the Stop op; no case failed on that alone. H01: ten sweeps hit the observer's `unreachable!` (a stale insert into a Stopping cell) before any oracle spoke; only case 9 and the seeded sweep reach an I2 failure first. HO01b shows I7 and I2 fire in 11 tests when the observer tolerates the state.
