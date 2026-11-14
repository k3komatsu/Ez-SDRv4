I’m tracing the report and switch paths across pauses, especially delayed reports after a resume or Stop. I’ll also check that the C1 verdict needs positive RX evidence, rather than passing on silence alone.
I found two gaps in the lifecycle handling: sequence errors are ignored while a repeat is paused, and the paused-state stutter check can let a delayed report from a stopped repeat end a newer repeat.

The 742 UHD library tests and 71 MT3-filtered tests pass. The full workspace run reached the 120-second timeout, so it has no complete verdict.
## Verdict: FAIL

Reviewed `5121cdf..6b7f677c`, read-only.

### Major

1. **Sequence loss does not end a paused repeat** — `crates/ezsdr-radio-uhd/src/provider/tx.rs:1121–1134`

   **Sequence:** start repeat → underflow/expired timeline → paused until R → receive `SeqError` or `SeqErrorInBurst` before R.

   No device burst is open, so `hit` is false. Sequence errors retire nothing, leaving `newest` empty; `dropped()` returns `Nothing`. The user receives `TX_UNDERFLOW { lost }`, but the repeat subsequently resumes, contrary to the decided sequence-loss ending.

   **Fix:** handle sequence loss against an already-started paused booking too. Add the gap case beside the current running-repeat test.

   **VERIFIED:** control flow. Countersequence not executed.

2. **A stopped repeat’s delayed report can abandon a newer repeat** — `crates/ezsdr-radio-uhd/src/provider/tx.rs:1122–1126`

   **Sequence:**
   ```text
   A resumes; its resumed part underflows after P−1 samples.
   Hold that report; Stop A (its correlation remains pending).
   Start B after the Stop region.
   B expires and pauses until RB.
   Read A’s delayed Underflow, whose tick precedes B’s start.
   ```

   Retirement reaches A only, but `paused` describes B. `(hit || paused) && played < stall` therefore returns `Ended`, and the caller abandons **B**. B never resumes; its record ends and its region is capped by an unrelated report.

   **Fix:** associate the paused-state stutter decision with the device part that produced that pause, rather than any paused booking. Preserve the intended delayed-two-entry case.

   **VERIFIED:** retained correlations and abandonment path. **INFERRED:** hardware report-delay occurrence; scripted countersequence not executed.

### Minor

3. **KL-45 overstates the measured boards** — `KNOWN_LIMITATIONS.md:196`

   It says H1 ran on “both boards”. Both raw measurement records identify serial `347D545` (#1 OBX), with loads `none` and `h4`. The session README also explicitly leaves board #2 unmeasured (`bench/2026-11-13-n67-records/README.md:13,33`).

   **Fix:** say “board #1, unloaded and loaded”. **VERIFIED** against both raw records.

### Checks

- UHD library: **742 passed**.
- MT3-filtered tests: **71 passed**, including C1 rehearsals.
- Full workspace: **incomplete**, timed out after 120 seconds.

No files changed, formatting or mutation tools run. Hardware C1–C7 and the new countersequences were not executed.
