> plan · gpt-6.1-sol
## NEEDS CHANGES

The classless declarations, shared admission checks, Python refusals and Provider guards appear sound. Two findings remain.

### 1. P2 — UHD no longer ends a TX clock cut at its origin
**Locations:** `crates/ezsdr-radio-uhd/src/provider/core.rs:665–668`; `crates/ezsdr-radio-uhd/src/provider/tx.rs:1034–1045`

- **Sequence:** register the initial TX clock C with origin O → publish loss or `Provider::stop` at S ≤ O → finish.
- **Wrong outcome:** the timeline retains C’s segment with cut zero, but C’s registry record still has `ended_at: None`. HEAD’s `Streams::reconcile` ended it at O. Its deletion removed the only production TX `clocks.end` call; `end_clock` now clears only owner state.
- This boundary is acknowledged in test-plan rev 6 §6, but its clock-end behaviour is not dead merely because ordinary suite runs miss it.
- **Fix:** retain the small, single-clock reconciliation that ends C when its plan acquires a cut. Do this at booking, including loss paths that bypass owner work. Add controlled-clock assertions for S < O and S = O, alongside a normal post-origin stop that keeps the TX clock open.

### 2. P3 — Current specs still promise deleted machinery and obsolete test coverage
**Locations:** `design/09-mock-radio.md:135,204,325,347,364`; `design/18-uhd-radio.md:530,567`

- **Sequence:** follow MR-15’s documented `cold` transmit change, or inspect the cited differential tests expecting membership changes, timed updates and release observations.
- **Wrong outcome:** admission refuses that change; those generated operations and observations no longer exist. MR-34 also still requires drawing timed-tune constants, although their fields and draws were deleted.
- These are current rules/test descriptions, not historical Changes rows. `rules.py` passes because the surviving test names resolve; it cannot verify their claimed subjects.
- **Fix:** remove obsolete clauses from MR-15/MR-34 and rewrite the affected test-table rows from today’s assertions. Keep historical descriptions in the Changes tables.

### Checks and limits

- Spot-checked DELETE decisions against HEAD, rewritten initial-value tests, part-(d) tests and requested mutation-row definitions.
- `rules.py --base HEAD --renames plan/phase8/reviews/47s2-renames.txt`: **0 problems**.
- `gate.py --self-test`: **PASS**.
- The known multi-pending-fault ordering mismatch remains outside generated coverage: Loss sequences contain at most one non-loss fault. I did not classify that pre-existing gap as a new regression.

No files changed. No builds, suite runs, mutation execution or hardware checks performed; mutation-kill claims were inspected, not independently rerun.
