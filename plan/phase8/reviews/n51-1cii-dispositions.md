# Note 51 stage 1c-ii — review record and dispositions (2026-11-14)

Every radio-uhd lib test now holds the manual test clock through the participant guard (`clock_guard`). The test thread itself registers as a clock participant: while it is busy the clock stops, and while it waits on the clock the clock runs. `Frozen` (a separate busy participant), `released` (work on a worker participant) and `handed_over` are deleted ([51-test-timing.md](../51-test-timing.md) §7.2 row 4, Q6).

The classification ([n51-1cii-test-plan.md](n51-1cii-test-plan.md)) covered 79 `Frozen` sites in 350 tests. It found no case for keeping `Frozen`: a guard holder blocked in a non-clock wait is busy, so it stops the clock exactly as `Frozen` did. A shim probe passed all 667 lib tests. The negative control (no hold) failed 326 tests, so the hold is real.

Risk: 4a was risky (the test clock's ownership semantics), so its plan was reviewed by Sol. Branch `work/n51-1cii` from `1750373`: sub-steps 4a, 4bc and 4d, squashed and rebased onto main. Implementer: Claude Code Sonnet 5.5 subagents, effort high ([n51-1cii-briefs.md](n51-1cii-briefs.md)). Sub-step reviews: fresh Opus subagents for 4a and 4bc. 4d (deletions and text only) was read by the orchestrator.

## Findings by source and severity

| Source | Major | Minor | Mechanism |
|---|---|---|---|
| Sol, plan ([n51-1cii-sol-plan.md](n51-1cii-sol-plan.md)) | 0 | 1 | — (P1302 missing from 4c's row run) |
| Opus, 4a | 0 | 1 | — (the e3b hook-leak check is live from 4a) |
| Opus, 4bc | 0 | 1 | — (stale texts) |
| Sol, code ([n51-1cii-sol-code.md](n51-1cii-sol-code.md)) | 0 | 1 | — (note 51's zero-wait claim stated for 350 tests; measured for 287) |

## Dispositions

All decided by the orchestrator; nothing is user-visible.

- **Table questions, all as recommended:**
  - Q1: stepped rigs no longer hang on a stray clock wait. This is accepted: 0 of 287 hold-only tests have one, and no mechanism is added.
  - Q2: the keep-case is struck from Q6, and W1 drops the guard where `Frozen` was dropped.
  - Q3: the helper is named `clock_guard`.
  - Q4: a guard line stays in the 23 tests that pass without a hold.
  - Q5: `SLACK_MS` is left untouched.
  - Q6: `c2.rs:581` holds the guard to the end.
  - Q7: all four sub-steps are done.
- **4b and 4c** were run as one sub-step, 4bc.
- **Findings:**
  - Sol plan m1: P1302 was run in 4bc and killed.
  - Opus 4a m1: `assert!(!next_lock_armed())` was added at `e3b.rs` before the post-operation `seam_line`.
  - Opus 4bc: the texts were fixed, `_driver` was renamed `_guard`, and N51D-03's anchor was lengthened, all in 4d.
  - Sol code m1: the claim was corrected.
- **Accepted cost (Q1):** a stray clock wait in a stepped stretch now lets time run instead of hanging. The stepped stretches of the 63 `released`/`handed_over` tests were not measured separately.
