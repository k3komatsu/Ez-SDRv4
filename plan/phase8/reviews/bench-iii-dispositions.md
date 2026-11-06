# Bench preparation (iii) after note 48 — dispositions (2026-11-06)

Not risky (bench-only test code and a runbook; production code unchanged): no test plan. Order: brief → Sol pre-check of the brief → implementer (Haiku 5.5 `anthropic/claude-haiku-5-5#high`, OpenCode2) → self-review (free DeepSeek refused at its limit; Go DeepSeek hung 25 min and was stopped; a narrower Go round ran) → orchestrator review → Sol code review → fix round 1 → Opus re-check (PASS) → gate. Records: `bench-iii-brief.md`, `bench-iii-brief-fix1.md`, `bench-iii-sol-pre.md`, `bench-iii-sol-code.md`, `bench-iii-gate.json`.

## Sol pre-check of the brief (12)

| # | Sev. | Finding | Disposition |
|---|---|---|---|
| 1 | FIX FIRST | TX200 adds slot-B (CBX) transmit trials when two TX channels exist | adopted: TX200 and the load test pinned to slot A (`iii::pin_to_slot_a`) |
| 2 | FIX FIRST | TX200 continues after `Err`/`left_open` | adopted; extended in fix round 1 to the guard latch (Sol code 2) |
| 3 | FIX FIRST | a blanket "zero TIME_ERROR" contradicts the negative controls (end + 1, preemption) | adopted: expected events per case |
| 4 | FIX FIRST | several #78 tests do not judge their faults (P70's control, P72, B0, preemption) | adopted: every block-(b) test prints a VERDICT from one fault judgement |
| 5 | SHOULD | T3's "different-rate control" hypothesis is wrong; the delta is +3 200 ticks | adopted (orchestrator's misreading of the log) |
| 6 | FIX FIRST | the fill probe could measure noise or a wrong rate | adopted: read-back, coverage and steady tone required |
| 7 | SHOULD | the probe reads fc32, not sc16; bound retained blocks | adopted |
| 8 | FIX FIRST | the load test can PASS while the sink's `drop_oldest` feed drops | adopted: Manifest link drops judged |
| 9 | SHOULD | NIC evidence empty by default | adopted: `EZSDR_BENCH_NIC`, byte and error/drop counters |
| 10 | SHOULD | the brief's comment correction in `hw_s1_short_first_sends` was backwards | adopted: the comment was right and stays (orchestrator's error) |
| 11 | SHOULD | `hw()` overwrites the first board's log | adopted: `<test>.<BOARD>.log`, no overwrite |
| 12 | SHOULD | manual row probes in a throwaway copy conflict with policy | rejected: AGENTS §9 forbids local `mutate.py` and mutants in the main tree; a debug copy outside the repository is what §9 allows |

## Implementer self-review (Go DeepSeek, narrow round)

Fixed: `faults_verdict` FAIL after INCONCLUSIVE; `stage_trial` empty/direction/underflow arms (tests added); a doc claim on `reach(0.9)`. Rejected with reasons in the report: `run_faults` has no unit test (a Manifest fixture needs every section); `tone_1msps` replacing `tone_wave` (would drop its magnitude assertion); `EZSDR_MEASUREMENT_NICS` (the brief names `EZSDR_BENCH_NIC`); merging `min_median_max` (different types).

## Orchestrator review (2)

| # | Sev. | Finding | Disposition |
|---|---|---|---|
| O1 | major | `hw_b8_burst_one_sample_after_a_burst` end + 1 changed to "on time"; the held-tail close is padded to two, so the first safe start is end + 2 (`tests/fake.rs` `ur_23_a_burst_at_the_first_safe_sample_after_a_held_tail_close_is_played`) | same as Sol code 1; fixed |
| O2 | minor | preemption at 10 ms is the window's boundary | subsumed by Sol code 4 |

## Sol code review (7)

| # | Sev. | Finding | Disposition |
|---|---|---|---|
| 1 | major | end + 1 after a held-tail close is late | fixed: one TIME_ERROR with cause `late` (`iii::late_control_verdict`) |
| 2 | major | a configuration timeout returns `AdmissionRejection` without stopping later trials | fixed: both tests open through `open_record_guard` and stop on its latch |
| 3 | major | restart-fill trial evidence stays in a thread-local buffer | fixed: one `III fill observation` line per rate |
| 4 | major | preemption judged against the requested lead, not UR-21's boundary | fixed: `iii::preemption_verdict` (FAIL only on faults, two errors or a non-late cause; count mismatch INCONCLUSIVE) |
| 5 | minor | summaries include incomplete trials | fixed: `complete_fills` |
| 6 | minor | the fill rehearsal was removed, not relocated | fixed: probe body in `common/iii.rs`, rehearsal `iii_restart_fill_probe_rehearses_on_the_fake` in `fake.rs` |
| 7 | minor | four row descriptions did not describe their mutants | fixed |

## Opus re-check (PASS; 4 minors)

Runbook preemption FAIL wording and a stop line missing the latch: fixed by the orchestrator in `bench.md`. No row for `preemption_verdict`'s async-fault arm: accepted as a note (unit-tested; the same arm in `fault_verdict` has its row). The fake rehearsals use the host clock: both ran 20/20 (`iii_restart_fill_probe_rehearses_on_the_fake`, `load200_duplex_stage_rehearses_on_the_fake`).

## Counts

Sol pre-check 12 (11 adopted, 1 rejected); Sol code 7 (4 major, 3 minor; all fixed); orchestrator 2 (1 major = Sol 1); re-check 4 minor. Mechanisms (`plan/maintenance/failure-mechanisms.md`): none counted. The majors are test-expectation and evidence defects in bench-only code (a misread rule, a missing stop, unsaved output), not a design mechanism.
