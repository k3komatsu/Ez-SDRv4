# 2026-11-15 bench probes, part 2 — briefs (2026-11-13)

---

## Brief — bench tools for 2026-11-15: a PN cross-correlation cell for the receive-restart zeros, and the cleanup drain's busy loop

Worktree /home/komatsu/local_work/Ez-SDRv4-a1, branch `work/bench1115b` at main `4f47f4c`. You are the implementer (OpenCode Sonnet 5.5 #high). Measurement tooling: not a risky step (no test plan, no self-review; Sol reviews the code at the end). Run cargo tests and clippy locally; mutation rows only through `REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py <list> sim02:8,sim03:8 <ids...>`, in the foreground. Never edit /home/komatsu/local_work/Ez-SDRv4 or /home/komatsu/local_work/Ez-SDRv4-e3 (other implementers). Do not commit. `cargo fmt --all`; `python3 plan/maintenance/tools/rules.py --base 4f47f4c` 0 problems.

## 1. The question (owner, 2026-11-13)
After a receive restart the first samples are zeros: by DDC half-band class 0/4/11/15 (bench 2026-11-13: 14/10/14/11/13/9–10/3–4/0 at 1/2/5/10/25/50/100/200 MS/s; `hw_iii_raw_rx_restart_fill`, tests/common/bench1115.rs `zeros_*`). Are the zeros the filter's group delay (the pipeline flushed on stop, so the signal arrives that many samples late and zeros come first), or a separate fill with the signal on time? Separate them with a PN sequence at low power, by cross-correlation — not by power thresholds.

## 2. The cell (new, in tests/common/bench1115.rs + one `#[ignore]` hardware test or a new phase of `hw_rxzeros_sweep`; your choice, say why)
Per rate of the zeros sweep (the 8 rates, one board, slot A, gain 0 on TX — low power, ≥ 10 trials a rate):
- TX: a repeating PN (reuse `pn`/`safe_pn_iq`, `observations::correlation_offset` in tests/common/observations.rs and the H2 code in hardware.rs) started timed at a known tick T0, running through the trial.
- (a) continuous: a receive started well before (≥ 1 s of settled stream), capture a window across a known PN period; correlate → `d_cont` (samples from TX time to RX time; the H1/H2 runs saw 44 at 1 MS/s).
- (b) restart: stop the receive, start it timed at T1 inside the running PN; count the leading zeros `z`; correlate the samples after the zeros against the PN at the expected phase → `d_restart`; also compare the restart's post-zero samples with what (a)'s alignment predicts at the same ticks (sample-by-sample residual).
- Report per trial and per rate: z, d_cont, d_restart, d_restart − d_cont, the correlation status (Found/Ambiguous/Absent) and the residual. Print a decision line written before the data: "zeros = group delay" if d_restart == d_cont (±1) and the first non-zero samples match the prediction (the zeros replace samples that would have been there: no time shift); "shifted" if d_restart − d_cont ≈ z; otherwise "neither". A FAIL only for infrastructure (no correlation peak, wrong trial count); the outcome itself is a fact, not a defect.
- A FakeDevice rehearsal in the default suite for the pure parts (the decision function over synthetic z/d values; correlation on a synthetic shifted PN), with mutation rows that kill the decision boundaries.

## 3. The cleanup drain's busy loop (bench 2026-11-13 R1/R2)
`drain_without_clock` (tests/hardware.rs ~1684), called by `restore_raw_probe` after `rx_stop`, loops for its whole limit on `RxRecv::Failed(.. "UR-3: the receive streamer is not open")`: about 184 000 rows per `hw_56_*` trial. Stop draining at the first non-sample, non-timeout result (keep that row), and add a small test for it if the function is reachable without hardware; otherwise say so.

## 4. Records
plan/phase8/bench.md's "2026-11-15 の手順" gets the PN cell (command, time estimate, what it decides, the TX at gain 0 with the attenuator confirmation). Report: files, tests (commands + results), rows (ids + outcome), the estimate of the cell's bench time, anything INFERRED. Write it to /home/komatsu/.cache/ezsdr-fixes/2026-11-13-bench1115b/report.md and exit.

---

## Brief — bench1115b fix round (small)

Same worktree and rules as brief.md (local tests; rows via remote_mutate.py foreground; do not commit; never edit the other trees). Read decisions.md here: change `drain_without_clock` (tests/common/raw.rs) so it ends only on `RxRecv::Failed(..)` (row kept); `Overflow`, `LateCommand` and other results reset `idle` and keep draining (row kept). Update the FakeDevice test (add: an Overflow does not end the drain) and rows B1115-126/127 (+ one row for the new arm if needed); run them. Then run the full mutation lists once, foreground, through the sim queue: `~/.cache/ezsdr-fixes/tools/simq env REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py all sim02:8,sim03:8` (it may wait for another lane), and settle every row that fails. `cargo fmt --all`, `rules.py --base 4f47f4c`. Append the results to report.md (section "Fix round") and exit.

---

## Brief — bench1115b fix round 2 (Sol's code review)

Same worktree and rules as brief.md (local tests; rows via remote_mutate.py foreground, touched and new rows only; do not commit; never edit the other trees). Read sol-code.md (Sol: FAIL) and the last line of decisions.md (both majors adopted, with the tests required). Turn each major's sequence into a failing test first, then fix. `cargo fmt --all`, `rules.py --base 4f47f4c`. Append a "Fix round 2" section to report.md and exit.
