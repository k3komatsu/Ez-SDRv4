# #107 / #109 — report

Design (root causes and fix):


### #107 `mt3_c4_rehearsal_the_fake_drops_b_before_its_own_limit_and_plays_it_at_and_after`

**Root cause (VERIFIED by a trace, 12 runs with a temporary `eprintln!` of `t0 % per` and `d0 % per` in `cascade_trial`, reverted).**
- `FakeDevice::new` sets `zero = host.now()` (device.rs:1036). On the manual clock, `HostClock::manual()` runs at real speed while no participant is busy,
  so by the time `cascade_cell` calls `device.host().drive()` (the first line of the cell), real time has passed: the clock's value, and so the device's
  tick count, carries a random sub-microsecond phase `p`.
- From `drive()` on the clock is frozen while the owner is busy and jumps only by the exact deadlines it sleeps (200 us in `wait_until`, whole ms elsewhere),
  and 200 us = 40 000 ticks is a multiple of every grid (`per` = 20 or 200). So `d0 % per == p % per` in every trial of the device, and
  `t0 = A's first TimeError tick = now when the fake takes A's first send` has `t0 % per == d0 % per` (trace: `t0%per == d0%per` in every line,
  one value per device: e.g. 12 runs gave per=20 phases 11, 6, 15, 1, 4, 5, 11, 6, 0, 14, 3, 19, ...).
- B's time is `align_to(t0 + a_ticks + delta, per)` and `a_ticks = S x per`, so B is aligned up by `(-(p + delta)) mod per` ticks. Every delta the cell books is a multiple of
  `per` (-20 us, +30 us, `limit - per`; the limit is a whole number of samples), so B is aligned up iff `p % per != 0`. With `p` uniform, one run in 20 at 10 MS/s
  (and one in 200 at 1 MS/s) has `p % per == 0`: no effective delta differs from its nominal one and the coverage assertion `some trial's B was aligned up` fails.
  (INFERRED: the rate. 1 in 20 does not by itself explain 2 failures in 10 on sim03; in the 12 traced runs one had phase 0, and it was the failing kind: six trials,
  all on the grid. The fix does not depend on the rate: it covers every phase. VERIFIED with forced phases: a scratch copy sleeps the owner thread after `drive()` to put
  `d0 % per` at 0 or per/2; the old test fails at phase 0 (at 1 MS/s, per 200, first) and passes at per/2; the new one passes at both rates and both phases.)
- The verdicts at and past the limit never depend on `p`; only the coverage assertion does. So the phase is incidental host real time leaking in at the
  construction -> `drive()` gap, not a property under test.

**Fix (the issue's second direction, since the phase is incidental).** Book one delta off the grid on purpose, so whatever `p` is the alignment path runs:
with the three on-grid deltas plus `limit - per - per / 2`, a phase that puts the half-sample delta on the grid puts the others off it (`per/2` is not a
multiple of `per`), so at least one trial is aligned up for every `p`. Also assert the property the alignment exists for, on every trial: B's time is on
the sample grid (`b_first % per == 0`), which a "skip the alignment" or "round to nearest" change breaks for some trial for every `p`. The delta is below the limit by
`per + per/2` and aligned up by under `per`, so it stays a dropped trial (`effective < limit`, checked by the existing loop). Nothing at and past the limit
changes, the bench test (`hw_mt3_c4_cascade_boundary`) and its grid are untouched (the extra delta is the rehearsal's own).
Rejected: making `p` deterministic (a `drive()` before construction is impossible: the driver needs the device's host; zeroing the phase in the rig would hide
the same leak from other tests and test the grid only at one phase, the weaker coverage). Rejected: retrying the trial until a phase is off the grid (a loop on a random
input; the added delta is deterministic).

**Mechanism.** Closest: `mech:wall-clock-test-oracle` in a loose sense (host real time reaches a verdict through a wall-clock phase on a manual clock, not a deadline
or a bound); the contract was not regressed, the test's coverage precondition came from the host. It is not a bound or a deadline, so it does not fit the
ledger's wording ("asserts on host wall-clock timing"); recommend labelling it `mech:wall-clock-test-oracle` only with that note, or leaving it unlabelled
(orchestrator decides). The count in the ledger would be 9.

**Rows.** BT1-87 (cells_of takes the nominal delta) and BT1-24 select this test; BT1-87 is killed only by the coverage assertion, so before the fix it
survived in one run of 20. A new row N107-1 skips the alignment (`e.b_first = align_to(...)` -> `e.b_first = t0 + a_ticks + delta`) and is killed by the
new on-grid assertion / the coverage assertion, deterministically.

### #109 `t200_bare_uhd_script_judges_a_run_and_bounds_it`

**Root causes.**
1. The exit wait is wall-clock: `for _ in $(seq 100); do kill -0 "$pid" ... || break; sleep 0.1; done` judges "hung" after 10 s (30 s for the early-signal
   case), then `kill -KILL`s the script and records `hung`. A host stall longer than that is read as a hang. (VERIFIED by the issue's three failures
   passing alone; the budget is in the file; the script's normal exit is well under a second.)
2. The start wait has the same shape (100 x 0.1 s) but falls through silently: after 10 s with no stub started the script is signalled anyway, a different scenario
   from the one named. (A suspected second cause, a signal landing between the stub's two writes `docker-calls` and `docker-pid`, was tried and ruled out: with a 2 s
   delay injected between them in a scratch copy the old self-check still passes, because the stub logs the `kill` line before it reads the pid and the script TERMs its
   `timeout` job itself. So the start wait stays on `docker-calls`.)

**Fix.** One helper in the self-check: `await WHAT CMD...` polls `CMD` every 0.1 s until it succeeds, and gives up after `BACKSTOP` seconds (default 600,
a plain variable so the helper can be exercised with a short one) printing `t200-bare-uhd: EZSDR-INFRA: self-check: WHAT did not happen within N s` to stderr and returning 1.
- the start wait becomes `await 'the stub docker run starting' grep -q '^run ' "$tmp/docker-calls"`, and in the early-signal case `[[ -e $tmp/docker-started ]]`;
- the exit wait becomes `await "SIG$sig's exit" gone "$pid"` with `gone() { ! kill -0 "$1" 2>/dev/null; }`; on expiry: the script is `kill -KILL`ed and `ok ... hung stopped`
  records the failure, as now (a real hang still fails, with the marker line, so a runner reads an ERROR/infra and not a kill);
- a stub whose start never shows is a failure too (`ok ... never started`), not a silent fall-through.
Two new self-check cases pin the helper: an event that comes late is waited for (a background `touch` after 0.3 s, default backstop, returns 0 and no marker),
and an event that never comes returns 1 with the marker (`BACKSTOP=1 await 'x' false`). The script's behaviour on the bench (signals, `--init`, the bound, its INFRA line on
expiry) is untouched. Rejected: `wait "$pid"` with a watchdog subshell (a second background job to kill and a PID-reuse window, more code than a poll
that the backstop already bounds). Left as is (said in the report): the `BOUND=1` expiry cases (`run_one ... BOUND=1`, `full 1 expiry`) bound a stub that must
write two lines within 1 s; a stall that long still fails them. That is the cases' own subject (a bound expiring), out of this brief.

**Mechanism.** `mech:wall-clock-test-oracle` (#109 labelled; it is a shell test, judged by a wall-clock budget). Ledger count already includes #109.

**Added after the first review of the DeepSeek diff (advisor point 2, VERIFIED in a scratch copy).** Row S4B-49 (`kill $(jobs -p)` removed from `stop_on_signal`) had its only
kill in the early-signal case's wall-clock budget: without that kill the script waits for the stub docker client, which ends by itself after the stub's `exec sleep 60`,
~61 s later, inside the 150 s backstop, so the mutant passed. The late stub now sleeps 600 s (`s=600` only under `STUB_LATE`) and the case's bound is 600, so that mutant
hangs past the backstop (the INFRA line is printed, `hung` is recorded, the self-check fails after 150 s); the row declares `"kill": "bound"` because its kill is a real-time
bound by nature (the property is "the script ends its run within `timeout -k 10` of the signal"). The other signal cases do not need it: their stub's `docker kill` by name
ends the sleeper. Two rows pin the helper (N107-2: `await` returns failure after the event came; N107-3: the expiry line loses its marker).
**Deviation from the brief's "wait by `wait`":** the exit wait polls `kill -0` every 0.1 s until the long backstop. A `wait "$pid"` needs a watchdog subshell that must itself be killed
(its `sleep` child outlives it) and a PID-reuse window; the poll is bounded by the same backstop and is not a verdict that depends on host speed.
**Environment note (VERIFIED):** the self-check cannot pass when its parent shell was started with SIGINT/SIGHUP ignored (`setsid nohup ... &`, an async list in a non-interactive
shell): `HEAD`'s script fails the same signal cases that way (a pre-existing property, not changed here); run it from a foreground or a job-controlled shell.


Design: the design section above. Diff: crates/ezsdr-radio-uhd/tests/fake.rs (+8/−3), plan/maintenance/tools/mutations.json (+32/−1), plan/phase8/tools/t200-bare-uhd.sh (+24/−11).

### Root causes
- #107 (VERIFIED by a trace of 12 runs): `FakeDevice::new` takes `zero = host.now()` and the manual clock runs at real speed until `cascade_cell` calls `drive()`, so the device's ticks carry a random sub-µs phase p; after `drive()` the clock moves only by exact sleeps (200 µs = 40 000 ticks), so `t0 % per == p % per` for every trial; the cell booked only multiples of `per`, so no B is aligned up exactly when `p % per == 0`. INFERRED: the observed rate (2 in 10 on sim03) vs 1 in 20 expected at 10 MS/s; the fix covers every phase.
- #109: the signal cases judged "hung" from 10 s / 30 s polls (VERIFIED from the file); that this caused the three recorded flakes is INFERRED. A suspected race between the stub's two writes was ruled out by a 2 s injected delay.
- Outside the fix: the self-check cannot pass when its parent shell ignores SIGINT/SIGHUP (`setsid nohup … &`, any async list) — VERIFIED pre-existing on HEAD; the `BOUND=1` expiry cases still need the stub to write two lines within 1 s (INFERRED, never seen).

### Fix
- #107: the below cell books a fourth delta `limit - per - per/2` (half a sample off the grid), so at least one trial is aligned up at any phase; a per-trial assertion `b_first % per == 0`; coverage message and header comment updated; row N107-1 (skip `align_to`). Nothing at or past the limit changed; the bench test and mt3_cascade.rs untouched. Mechanism: loosely `wall-clock-test-oracle` (a clock phase leaking host real time) — orchestrator to label.
- #109: `await WHAT CMD…` polls every 0.1 s until the event, `BACKSTOP=150`; on expiry prints `t200-bare-uhd: EZSDR-INFRA: self-check: …` to stderr (to a file inside the self-check) and returns 1; waits for the stub's start and the script's exit (`kill -0` poll, not `wait`: a `wait` needs a watchdog subshell whose `sleep` outlives it and has a PID-reuse window). Three self-check cases pin the helper. The late stub sleeps 600 s, the early-signal bound is 600; S4B-49 now declares `"kill": "bound"`; new rows N107-2 (`await` returns 1 after the event), N107-3 (marker lost).

### Checks
- fmt clean; clippy -D warnings rc 0; cargo test --workspace rc 0 (fake 677/677), from a plain shell (a first run launched under `setsid nohup … &` failed the t200 signal cases — the ignored-signal finding).
- rules.py --base 0bd01fa4: 0 problems.
- Loaded host (sim03, 32 busy loops, load ≈ 27–32): #107 test 50/50, t200 test 30/30 (t200 loop in the foreground).
- Each fixed test still fails on its defect (scratch copies, removed): #107 old test fails at phase 0 on the coverage assertion and passes at per/2; new test passes at both phases and rates; N107-1 fails the new grid assertion at phase 0, per/2 and a random phase. #109: a script that never exits after a signal (BACKSTOP=3 copy) reports `hung`, status 137 and the EZSDR-INFRA line; S4B-49's mutant fails the early-signal case after 150 s with the marker; N107-2/3 fail their cases; the unmutated script prints no marker.
- Rows (rows.log, maintenance list, sim02:7,sim03:7, 217 s): 19/19 — S4B-38…51 killed (S4B-49 killed (bound), declared), N107-1/2/3, BT1-24, BT1-87 killed (BT1-87 was killed only by the coverage assertion, so it could survive before the fix). Selection: grep of all six lists for t200, mt3_c4, mt3_cascade, cascade.

### Adversarial review (one line per changed test)
- Rehearsal, new delta + coverage: fails on the old test at phase 0 and on N107-1 (grid assertion), not on a timeout; single-threaded.
- Rehearsal, `b_first % per == 0`: fails on any alignment-skipping or rounding change at every phase (residues 0 and per/2).
- t200 `await` late-event case: N107-2 fails it; the default backstop is long.
- t200 `await` expiry case: always expires, not timing-dependent; N107-3 fails its marker count.
- t200 signal and early-signal cases: event waits with a 150 s backstop; a real hang shown by the BACKSTOP=3 copy and S4B-49; a start wait that falls through is now a failure (`not started`).
- The `s=600` stub line: without it S4B-49 survived (the mutant passed in 65 s, inside the backstop).

### Trial record
- ds-1 (#107 edit + N107-1): 777 s, right first time. ds-2 (#109 `await`, case rewrites, S4B-49 bound): 151 s, followed the task exactly; review found S4B-49 surviving — Sonnet's own design error (bound raised to 600 but the stub's `sleep 60` ended the wait at 61 s). ds-3 (send-back 1: stub s=600, rows N107-2/3): 253 s; it added a `printf '` prefix to N107-3's `old` so the anchor matches once (Sonnet's text matched twice) — right.
- 3 review rounds, 1 send-back; ~66 min total, ~22 min DeepSeek. Sonnet wrote all the code into the tasks as exact lines.

### Not checked
- The self-check under ignored-signal launches still fails there (pre-existing); whether any runner launches the gate that way is unknown (INFERRED: the gate's container does not).
- Rows in the other five lists beyond the grep; the full lists run at the gate.

## Fix round 1

F1: the late stub publishes its pid first, installs `trap '' TERM` before `ready`, creates its container only after a non-`run` docker call is logged; the test asserts `kill, run, rm -f`; each case calls `reset` and reaps what it started.
F2: every stall-caused outcome is an `EZSDR-INFRA:` line on the self-check's stderr and ends the self-check at once (BOUND=1 cases check the stub wrote `run`; the early case counts a stall only when `kill` and `rm -f` are present and `run` is not; signal cases use a 600 s bound and the stub sleeps 600 s); real defects fail plainly.
F3: `await_exit` waits for the child's status with `wait`; a watchdog subshell reads an unwritten fifo with `read -t` and interrupts the wait with USR1; BACKSTOP 100 s on `SECONDS`; the first expired wait calls `abort` (reaps the script's process group, the stub and the `timeout` parent) and exits; `reap` guards pid > 1 and checks /proc/<stub>.
F4: the late-event probe is false once then true; poll count asserted 2.
Send-back 1 (Sonnet's own finding): S4B-48 and S4B-51 became ERROR because bash ignores an untrapped SIGQUIT, so the QUIT case waited for the backstop; `caught()` reads SigCgt from /proc/<pid>/status after readiness and reports "the script traps it" at once; both rows killed plainly again without `bound`.
Rows: N107-2's anchor moved to `return 0 # the event came`; new N107-4 (cleanup abandoned when the kill by name fails). rows-fix1.log 17/17: S4B-38…48, 50, 51, N107-2/3/4 killed; S4B-49 killed (bound), declared.
Checks: fmt, clippy clean; rules.py --base 0bd01fa4 0 problems; self-check `all cases pass` in 13–16 s; cargo test --workspace passed twice; one run of the whole fake binary failed `n47s5_one_sample_after_a_burst_refusal_rehearsal_on_the_fake` (passed 3/3 alone and in both workspace runs; output not captured; INFERRED load flake of another test). Loaded sim03 (load 36–46): t200 30/30.
Scratch checks: F1 a 3 s driver delay still passes in order; the N107-4 mutant fails plainly on the order assertion. F2 a 2 s stub stall in the 1 s bound case gives one INFRA line, no FAIL, exit within 2 s (the d585cf96 script gave two plain mismatches); a late stub held past `timeout -k 10` gives INFRA "did not get to create its container". F3 the S4B-49 mutant: one marker at 100 s, exit at 104 s; a 6 s backstop and a no-readiness variant: one marker, immediate exit, no process left.
Trial: ds-fix1-1 228 s (copied the code exactly); ds-fix1-2 (send-back) 244 s; Sonnet wrote all the code; ≈1.5 h elapsed, ≈8 min DeepSeek.
Residuals: the self-check still cannot pass under a parent that ignores SIGINT/SIGHUP (pre-existing); INFERRED that the stall rules cover Sol's windows (injected stalls only 2 s and 15 s); other five lists not run.

### Touch-ups after the Opus re-check (recheck-fix1.md: PASS with minors), by Sonnet, no DeepSeek run
1. Finding 1: the early case sets `sent=$SECONDS` before the INT; the stall condition also needs `((SECONDS - sent >= 9))` — a script that kills the client and exits sooner is a plain FAIL, not an INFRA line. Scratch mutant (`kill -KILL $(jobs -p)` in stop_on_signal): one plain FAIL in 3.8 s, no INFRA line; a real stall (late stub held 15 s) still only the INFRA line.
2. Note 1: the two `ok … hung stopped` calls after the aborts dropped (abort exits 1).
3. Note 3: the bound case accepts 124 or 137.
Checks: self-check passes (13.8 s); rules.py 0 problems; t200 test passes; rows-fix2.log 17/17 (S4B-49 killed (bound)).
