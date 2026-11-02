# Brief for Sol: narrow check of N1 and N2 (E2a-2), read-only

You are the independent reviewer of the repository at ~/local_work/Ez-SDRv4. You are read-only: edit nothing, build nothing, and change no git state. **Never read `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`.**

## Inputs
- Your re-review `~/.cache/ezsdr-fixes/2026-11-01-n45/sol-e2a2-rereview.md`: N1 and N2.
- The dispositions: the "Re-review" section of `dispositions-sol-e2a2-code.md`.
- The implementer reports these changes, recorded in plan §17.3:
  - **N1:** `settle` and `shut_down` advance one tick only when every live receive executor is parked, either idle on its inbox or waiting on the fake's clock. Wall time is only a hang watchdog.
  - **N2:** `model.rs` `recv(stream, n, deadline)` returns a packet only if `ready_at <= deadline`. The world and `ModelDevice` share it. `s9j` covers your sequence and its converse.
  - **Rows:** E2-55 re-anchored; E2-59 new. There is no row for N1, because its mutant is killed by no assertion.
  - **Reruns:** unchanged. 2,000-seed sweeps clean, Q1 0 unclassified, `s1b_full` 34/34.

## Check
1. Are N1 and N2 closed? Give the evidence at `path:line`.
2. **N1:** can "parked" be true while an executor is about to take a step that a tick would race? For example, it is between its inbox and its device call, or between waking from the clock and recording. Is there any remaining path where wall time changes the simulated history?
3. **N2:** is `ready_at <= deadline` the same rule as FakeDevice's `rx_recv`, including one-packet mode and the stopped tail?
4. Is "no row for N1" acceptable, or should a test observe the parked condition?
5. Readiness for the full gate.

## Output
- Verdict: PASS or REVISE.
- Findings P1/P2/P3 with `path:line`.
- At most 40 lines.
- End with "Not checked:".
