# Note 51 stage 1c-ii — Sol's plan review (2026-11-14)

## Verdict: PASS — one minor correction

Replacing the remaining holds with participant guards is supported by the code. No blocking flaw found in the proposed test plan. This approves the plan, not the reported probe results or eventual implementation.

### Findings

1. **Minor — add `P1302` to 4c’s mutation list. VERIFIED.**

   `table.md:328` additionally converts the sleeping participant at `crates/ezsdr-radio-uhd/src/provider/tx.rs:1852`. Its killing row is explicitly mapped to that test at `plan/maintenance/tools/mutations.json:10866–10873`, but Appendix B’s 4c list (`table.md:724–725`) omits `P1302`.

   **Fix:** include `P1302` in 4c’s row run and update the count. The final full gate covers it, but the planned sub-step check does not.

### Independent scenarios versus the plan

| Failure scenario identified before reading the plan | Assessment |
|---|---|
| Busy guard blocks an ordinary receive/join while the worker needs clock time | **VERIFIED:** the five W1 sites require the existing drops. The plan preserves them. For example, `rx.rs:891–910` waits for a clock-dependent receive. |
| Non-clock handshake must preserve frozen time | **VERIFIED:** keeping the guard works at `rx.rs:1113–1151` and `rx.rs:1475–1501`. Those workers are released by a channel or lock hand-off, not elapsed clock time. This supports correcting Q6’s keep-case premise. |
| Stray clock wait silently advances time and weakens an assertion | **VERIFIED semantic difference; occurrence INFERRED:** `host.rs:233–255` makes a waiting participant idle, unlike the separate busy `Frozen` participant. `table.md:340` identifies this loss, and `decisions.md:3` explicitly accepts it. No additional mechanism is required by this review. |
| Missing guard, lifetime gap, or duplicate registration | The proposed construction removes the hand-over gap. **VERIFIED:** duplicate registration is refused at `host.rs:415–420`. The rewritten helper test and retained `N51D-03` check the hold’s observable effect. |
| Inlined `released` leaves a thread-local hook armed | **VERIFIED risk:** `NEXT_LOCK` persists on the calling thread (`test_support.rs:237–265`). The plan requires checking `next_lock_armed()` after affected work (`table.md:72`). In particular, place that check before the post-operation `seam_line` at `tx/tests/e3b.rs:877`, which itself takes the targeted lock. |
| Mutation kills or intended interleavings change despite green tests | **INFERRED until execution.** The plan appropriately requires mutation reruns and preserves W1 worker choreography. Unchanged assertions alone do not establish equivalent test strength. |

### Evidence checked and limitations

**VERIFIED independently:**
- Clean tree at `17503736`.
- Static counts: 83 constructor matches including helper internals/tests, 39 `released` calls, 16 `handed_over` matches, and 20 matching drops.
- Appendix A contains 349 non-helper tests. Cross-checking their names against mutation filters reproduces Appendix B’s 248 listed rows plus deliberately excluded `J58`.
- No helper uses found outside the lib tests using the exact search `\bFrozen\b|\breleased\(|\bhanded_over\(` in the three named test/client areas.

**Not independently verified:** the reported 667-test shim pass, 24/24 parallel runs, delay-injection pass, negative-control results, dynamic site attribution, or zero waits in 287 hold-only tests. The probe copy was deleted; the supplied directory contains no raw probe outputs.

No files changed; no tests or mutation tools run. Remaining risk is the explicitly accepted loss of hang-on-stray-clock-wait detection.
