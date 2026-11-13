# Note 51 stage 2c — Sol's code review (2026-11-13)

My independent failure scenarios, before consulting the plan or reports:

- A trial is both late and wrong: repeated markers must end in mutation `ERROR` / a red gate, never a silent pass.
- A skipped trial precedes a plain failure: no timing marker may excuse that failure, including cross-trial assertions.
- A timed Action lacks a deadline, or becomes late inside `submit`: it may fail unmarked despite host delay.
- N7’s exclusion removes another Action’s evidence, or leaves C’s expected lateness behind.
- The wiring guard counts an unrelated `.trial()` / `.finish()` or misses an assertion executed outside classification.
- A timing-marked baseline is accepted without a real marker-free rerun.
- C’s shorter waveform, N2’s margin, or M4’s stricter verdict changes the hardware claim beyond what the rehearsal checks.
- Existing mutation filters now die only by timing, a bound, or a race.

I’m now comparing these against the plan, rows, and recorded checks.
Two issues surfaced:

- The new `one_sample_pair_returns_the_manifest_its_time_errors_were_read_from` test bypasses `Rehearsal`. In the external copy, it passes normally but fails unmarked with the existing 60 ms delay injection: its “two bursts” assertion sees zero or one.
- The accepted plan requires N2’s own late drops to use `late_arrival_verdict`. That path is absent; both Stop verdicts feed all trial TIME_ERRORs into a plain fault failure.

No repository files have been changed.
## Verdict: FAIL — two majors

The main classification and retry paths follow §5.3b, but a new test reintroduces unmarked timing failures, and N2’s accepted hardware lateness handling is missing.

### Major 1 — New p70 check bypasses classification
**VERIFIED** — `crates/ezsdr-radio-uhd/tests/fake.rs:8233–8240`

Sequence: create the pair → delay the test thread before a timed submit → the Provider legitimately drops a late burst → assert that the Manifest contains two bursts.

I reproduced this in the external copy using the existing delay injection:
- No injection: passes.
- `EZSDR_RIG_DELAY_MS=60`, `EZSDR_RIG_DELAY_AT=now`: fails, `Some(0)` versus `Some(2)`.
- Same delay at `submit`: fails, `Some(1)` versus `Some(2)`.

Neither failure carries `EZSDR-TIMING:`. The nine-test governance table does not cover this new test. Thus host delay still makes the gate fail plainly.

**Fix:** remove the timing-dependent burst-count assertion from this identity check, or classify its trial before asserting it. Include this new test in the delay sweep.

### Major 2 — N2’s own late drop becomes a hardware FAIL
**VERIFIED code path; hardware occurrence INFERRED** — `crates/ezsdr-radio-uhd/tests/common/n47s5.rs:719–743`, `crates/ezsdr-radio-uhd/tests/common/iii.rs:1174–1185`

Sequence: Stop → N1 stamped within the 5 ms margin → host stalls before N2 → N2 arrives late and emits `(late, drop)`.

Both Stop verdicts consume the whole trial’s `run_faults`; the TIME_ERROR therefore produces a plain **FAIL**. The N1-stamp guard does not help because N1 was on time. Fake rehearsals skip this trial, so their passing results cannot verify the hardware handling.

The accepted decision explicitly requires N2’s own late drops through `late_arrival_verdict` (`plan-decisions.md:42`; `test-plan-v5.md:14`). That path is absent.

**Fix:** implement the accepted late-drop handling, preserving failures for independent faults. Check both hardware verdict lines so N1 does not still fail solely because N2 was late.

### Minor — N47S5-192’s new killer remains timing-dependent
**INFERRED from assertions** — `plan/maintenance/tools/mutations.json:15556–15563`, `crates/ezsdr-radio-uhd/tests/fake.rs:8110–8120`

If A and C each produce one late drop, the mutant that reads *other* targets returns A’s single drop. The new test checks counts, not target identity, so its assertions can still pass.

**Fix:** pin target selection with synthetic events and use that deterministic test as the row’s killer.

### Maintainability and checks

No separate state/ownership blocker found. Reusing the shared classifier and runner logic is sound; the remaining weaknesses are test coverage and hardware-verdict fidelity.

Passed **61 focused tests** and **21 governance tests** in the external copy; reproduced Major 1. Copy deleted; repository unchanged.

Not checked: hardware, full suites, or mutation execution. No remote hosts or `mutate.py` were run.
