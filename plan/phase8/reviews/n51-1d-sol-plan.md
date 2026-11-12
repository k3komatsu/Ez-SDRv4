Independent scenarios: a release-before-registration gap; a driver retained during a channel wait/join; registering a helper rather than the owner thread; dropping the driver before re-freezing; W2 rewrites losing cancellation/report-read coverage; and mutations whose only failure is a timeout or whose unguarded version still passes. The inventory also needs `drop(_frozen)` and field releases: the current tree has additions beyond the original 15 sites.
## Verdict: **PASS WITH CHANGES**

No major finding. The register-before-release helper is sound; the **4 G + 3 W2** inventory matches the current tree. Removing W2’s sleeping helper and join preserves the shutdown work and existing assertions.

Paths below are relative to `/home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51-1d/` unless repository paths are specified.

### Required changes

1. **Minor — mark the new setup timeout as infrastructure failure.**  
   **VERIFIED:** `test-plan.md:126`; `probe/sketch-1d.diff:51` uses an unmarked `recv_timeout(10 s).expect(...)`. If the worker does not reach registration within that bound, the behavioral assertion never runs, but the mutation runner can report ordinary `killed` (`plan/maintenance/tools/mutate.py:178–182`).  
   Emit `test_support::infra` on **Timeout**, not Disconnected. Keep N51D-02’s intended kill as the clock-equality assertion; no `kill: bound` declaration is needed.

2. **Minor — distinguish the guard pin from the original regression pin.**  
   **VERIFIED:** N51D-03 fails at the later `device_at` equality, not the missing-apply assertion (`test-plan.md:150`; `plan.log:3204–3209`). Removing the re-freeze means that failure also detects an unheld clock during subsequent stepping; it does not independently establish that the deliberate pre-reopen stall exercised the original failure.  
   Retain the failing-first evidence **with the original re-freeze**, or add an exact clock-equality assertion immediately after the slow step and before `tx.pass()`. Describe N51D-03’s actual observation accurately.

3. **Minor — narrow negative diagnostic claims.**  
   **VERIFIED wording issue:** `test-plan.md:70–88,92` concludes “No dependence” from sampled injections. Say **“no dependence observed at these injection points/durations.”** Delays inside owner operations were not exhaustively tested. The proposed structural guard remains justified.

### Otherwise checked

- **VERIFIED by inspection:** registration belongs to the calling thread; `Driver` is `!Send`; the G guards remain live through owner work; deleting `ur_28`’s re-freeze introduces no hand-back gap.
- **VERIFIED:** N/W1/H classifications are appropriate. W1’s off-clock waits must remain unguarded.
- **VERIFIED from supplied raw output:** N51D-01/02 reach equality assertions; the unguarded M5/M6 variants pass. Omitting meaningless rows for those sites is correct.
- The 13 existing row filters match the named tests; **four apply only if C′ lands**.
- Text corrections are appropriate. `test-plan.md:179` must not silently replace §9’s per-sub-step review requirement; either implement this small change as one step or obtain an explicit exception.

### 1d-C′

**Defer to stage 4.** These sites have no `Frozen` hand-over, and the sketch drops the recovery guard before the subsequent dispatch (`probe/sketch-1d.diff:112–115`). It is reasonable cleanup, but not a complete timing closure and unnecessary for 1d.

No edits, builds, or probes performed; runtime evidence was inspected, not independently reproduced.
