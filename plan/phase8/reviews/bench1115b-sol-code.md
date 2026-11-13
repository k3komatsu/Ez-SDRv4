# 2026-11-15 bench probes, part 2 — Sol's code review (2026-11-13, openai/gpt-6.1-sol, read-only)

**Verdict: FAIL — two majors before bench use.**

What this change does: Adds the PN restart measurement and FakeDevice rehearsals, and fixes the cleanup drain’s failure loop.

### Must fix

1. **The residual confuses normal filtering with a restart difference — VERIFIED counterexample; hardware occurrence INFERRED.**  
   `crates/ezsdr-radio-uhd/tests/common/bench1115.rs:1686–1705,1726–1731`
   - Sequence: receive PN through the unchanged filter `y[n] = 0.6·PN[n−44] + 0.4·PN[n−45]`; restart with eight zeros replacing samples, without shifting the remaining signal.
   - Numerical probe: both correlations find delay **44**, with valid peaks, but `res_gd = 0.549`. The outcome becomes **Neither**, although the post-zero signal remains on time.
   - Cause: the residual fits ideal transmitted PN, not the filtered continuous received waveform. The current rehearsals use only ideal PN times a complex gain.
   - **Fix:** use a continuous received template covering the needed PN phases, or explicitly qualify the residual against continuous-channel distortion before drawing a conclusion. Add this filtered counterexample and a killing row.

2. **Receive-call failures become INCONCLUSIVE and busy-loop — VERIFIED by control flow.**  
   `crates/ezsdr-radio-uhd/tests/common/bench1115.rs:1931–1932,2011–2025`
   - Sequence: continuous capture succeeds; restart’s `rx_recv` repeatedly returns `RxRecv::Failed`.
   - Wrong outcome: `read_window` accumulates failures until its deadline; missing coverage sets `incomplete`, while `error` remains `None`. The cell reports **INCONCLUSIVE**, and the hardware test can finish successfully instead of reporting the promised infrastructure **FAIL**.
   - **Fix:** stop immediately on `Failed` and propagate it through the trial’s error path. Rehearse this actual receive path and add a mutation row.

### Review coverage

- Decision tolerance regions are disjoint for `z ≥ 3`; zero counting uses exact complex zeros.
- The cleanup drain matches the corrected decision: failures stop it; overflow does not.
- **Maintainability:** no additional concrete finding. Existing helpers are reused; the missing filtering and failed-read cases are the important rehearsal/mutation gaps.

**Checks:** 63 bench1115 tests passed in the external copy; `git diff --check` passed. Copy removed; worktree unchanged.

**Not checked:** hardware/UHD execution or mutation runs. Actual high-rate filtering and throughput remain unverified.
