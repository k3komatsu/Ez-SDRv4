# 段階 3f：Sol のレビュー

## コードレビュー

> plan · gpt-6.1-sol
VERDICT: FAIL

### Major — Measured record still counts successes, not trials run

`crates/ezsdr-radio-uhd/tests/common/measurement.rs:1371` returns the successful-trial count, which becomes `ValueRecord::Measured.trials` at line 1398.

If a condition needs eight attempts to obtain five successes, its measured record reports `trials: 5`, not `8`. This contradicts the brief’s requirement that record `trials` reflect actual attempts. The observation’s separate counts are correct.

**Fix:** Return the limiting condition’s `trials.len()` from `summarize_queue_depth`, while continuing to select depth and establish eligibility using successes only. Add a mixed-success test asserting the resulting measured record’s trial count. The existing assertion at line 3986 currently pins the incorrect count.

### Other checks

- The loop correctly stops at the required successes or trial limit, preserves alternating delays, and retains the failure/deadline latch checks.
- Validation checks the limit against required successes and applies the three-condition u32 bound to the limit.
- Environment parsing and `run-record.sh` forwarding are connected.
- Static inspection identifies killing assertions for P1038–P1043 and P811, P826–P828, P1002.
- UR-34 and its Changes entry describe the intended behavior; the bench estimate is marked INFERRED.
- No `src/` changes. The fake-rehearsal deviation is disclosed; success stopping is covered only by the pure helper test.

Not checked: No tests, mutation runs, Cargo or hardware commands were executed, as instructed. Reported execution results were not independently reproduced.

## 見直し

> plan · gpt-6.1-sol
VERDICT: PASS

No findings.

- Eligibility, depth and limiting condition still come exclusively from successful trials.
- `Measured.trials` now reports all attempts run for the limiting condition; insufficient successes and latched failures remain unmeasured.
- The mixed eight-attempt assertion checks `trials == 8`, condition `"a"` and depth `8`.
- P1044 matches exactly once and changes that count to `4`, causing the assertion to fail.

Not checked: Tests and mutations were not executed; this was a read-only static review, as instructed.
