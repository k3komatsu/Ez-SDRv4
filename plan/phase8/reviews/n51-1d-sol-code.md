Independent failure scenarios, before reading the diff:

- Releasing the frozen participant before registration lets virtual time advance in the gap.
- Keeping a registered driver while the test waits outside the clock deadlocks progress (W1).
- Dropping the driver before owner work restores the race; retaining it across a later off-clock wait introduces a hang.
- Removing W2’s helper could weaken checks of shutdown ordering, owner completion, cancellation reasons, or thread cleanup.
- A mutation killed only by a timeout or scheduling race does not establish a reliable assertion-based kill.
- Documentation or mutation anchors could still describe the removed timing mechanism.
## Verdict: **PASS WITH CHANGES**

The hand-over implementation is sound. No major finding in `bfd59d3..fafb23c`.

### Minor — correct the bound-only mutation result

**VERIFIED; pre-existing, not introduced by 1d.**

`crates/ezsdr-radio-uhd/src/provider/tx/tests/e2a.rs:39–50` kills **E2A2A-10** only when its 10 s `recv_timeout` expires. The row lacks `"kill": "bound"` (`plan/maintenance/tools/mutations.json:16487–16495`).

Sequence: remove the re-entry check → worker locks the same mutex twice → worker deadlocks → test times out without an infrastructure marker → runner reports ordinary `killed`. `mutate.py:178–182` explains that classification: an unmarked internal timeout looks like an ordinary test failure.

Raw evidence confirms exactly this outcome: `~/.cache/ezsdr-fixes/2026-11-12-n51-1d/impl.log:1844–1850`.

**Required disposition:** do not count this as an assertion-based kill. Mark `Timeout` as infrastructure and declare this intentionally bound-killed row, then re-run it—or record an explicit deferral rather than claiming all results satisfy stage M.

### Otherwise checked

- **VERIFIED by inspection:** `test_support.rs:191–195` registers before releasing `Frozen`. All seven site guards cover the owner work; none extends across an off-clock owner wait. W1 sites remain unchanged.
- **VERIFIED:** W2 removes only the sleeping helper and its join. Shutdown ordering, cancellation reasons and existing assertions remain. The removed thread was not the owner thread; no replacement thread is detached.
- **VERIFIED from supplied raw output:** N51D-01…03 fail at their intended clock-equality assertions, not bounds (`impl.log:1738–1761`). The nine targeted existing behavioral rows retain assertion-based kills (`:1762–1842`). J58 explicitly declares equivalence.
- **VERIFIED by inspection:** the shared helper avoids seven copies of the ordering decision. Text reflects 4 G / 3 W2 sites and C′’s deferral. No introduced maintainability issue found.

Not checked independently: builds, runtime repetitions or mutation execution. Review was read-only; runtime conclusions above rely on inspected raw records.
