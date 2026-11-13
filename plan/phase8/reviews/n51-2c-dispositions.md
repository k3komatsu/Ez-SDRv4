# Note 51 stage 2c — review record and dispositions (2026-11-13)

FakeDevice rehearsals of hardware-facing trials no longer judge a trial that carries lateness evidence (§5.3b of [51-test-timing.md](../51-test-timing.md), owner 2026-11-13). The evidence is a `radio.TIME_ERROR` with cause `late` (not `late_at_device`), or a LogEntry stamped past the test's literal window. Such a trial writes the `EZSDR-TIMING:` marker, and the test is re-run serially (mutate.py `RERUNS`, gate.py `excuse_timing`). A test marked on all three runs turns the gate step red and makes a mutation row `ERROR`. A Kernel refusal wins over evidence (§5.3 priority 2). The mutation baseline goes through `judge`. The ten rehearsals run through `Rehearsal`/`trial`, and a governance guard (crates/ezsdr-acceptance/tests/governance.rs) counts per rehearsal its `.trial(` sites, `finish()`, assert macros and `_verdict(`/`_outcome(` calls outside trials. The shared trials changed: C is 1 000 samples; the hardware Session Stop gains N2, whose own late drop is INCONCLUSIVE (host latency) and is left out of N1's line; N2 is INCONCLUSIVE when N1's stamp is past its deadline; a missing long-close shape with clean reports is a FAIL (M4); N7 (held tail) is judged inside a trial whose lateness leaves out only C's own late drop and deadline. Risk: risky (the boundary between a fake and the hardware it stands for: what a verdict means). Branch `work/n51-2c` from `4327a66`, sub-steps §5.3b, A, B, C1, C2, C3, two fix rounds, squashed and rebased onto `79f55e1`. Implementer: OpenCode Sonnet 5.5 #high ([n51-2c-briefs.md](n51-2c-briefs.md)); test plan [n51-2c-test-plan.md](n51-2c-test-plan.md) (v5).

## Process

- The first plan (P7: everything after receipt is the Provider's, a masking rule with windows) went through Sol ([n51-2c-sol-plan.md](n51-2c-sol-plan.md), plan v1) and an Opus re-check. Both found that P7 conflicts with RM-15 (a dispatch delayed by host lateness is judged by its late policy there, design/07). The premise was brought back to the owner, who adopted §5.3b: a trial with lateness evidence is not judged. Fable 5.2 then reviewed §5.3b ([n51-2c-fable.md](n51-2c-fable.md)). The plan went from v3 through v5: Sol (plan v3), an Opus re-check (v4), then a narrow Opus re-check (v5, closed).
- Sub-steps, each committed locally:
  - §5.3b text;
  - A: the baseline goes through judge;
  - B: `late_evidence`, `Rehearsal`, admission first, the wiring guard, the shared verdict changes;
  - C1: the nine rehearsals through `Rehearsal`;
  - C2: rehearsals withhold judgment on evidence;
  - C3: N7 through `trial` with C's own drop excluded.
- Full lists at C3 (`6793e49`): 2569/2570, only the known survivor `phase7/G09`, 0 `ERROR`.
- Sol's code review ([n51-2c-sol-code.md](n51-2c-sol-code.md)): FAIL, two majors, one minor. Fix round 1, then the Opus re-check ([n51-2c-recheck.md](n51-2c-recheck.md)): closed, four minors. Fix round 2 (minor-only, no re-review).

## Findings by source and severity

| Source | Major | Minor | Mechanism |
|---|---|---|---|
| Sol, test plan v1 | 5 | 4 | `wall-clock-test-oracle` (M1 the window excused Provider lateness, M3 faults evaluated after lateness was excused, M4 a missing long close attributed to the owner's turn, M5 the rx restart window not derived; M2 the receipt witness) — plan defects of the masking design |
| Opus, re-check of plan v2 | 4 (N0–N3) | — | P7's premise refuted against RM-15 → owner decision §5.3b |
| Fable 5.2, §5.3b design | 4 | several | — (M1 "the same result alone" had no mechanism → owner: marked three times = red / `ERROR`; M2 the baseline's re-run; M3 serial runs not deterministic; M4 N7 vacuous) |
| Sol, test plan v3 | 2 | 4 | — (M1 the guard checked presence, not each site and `finish()`; M2 the Kernel-admission check inside the rehearsal path) |
| Opus, re-check of plan v4 | 2 (R1, R2) | 1 | — (R1 asserts outside trials; R2 N2 on hardware after a late N1) |
| Opus, narrow re-check of plan v5 | 0 | residuals | closed |
| Sol, code | 2 | 1 | `wall-clock-test-oracle` (M1 a new test failed unmarked under delay; M2 N2's own late drop a plain hardware FAIL) |
| Opus, re-check of fix round 1 | 0 | 4 | — (m1 an over-subtracting mutant unkilled, m2 the guard table missed the tenth rehearsal, m3/m4 note 51 text) |

The code findings are the mechanism note 51 answers (`wall-clock-test-oracle`). They were fixed inside the rule, not beside it.

## Dispositions

**Sol, plan v1 (FAIL)** — adopted. The 3 ms window that answered M1 was withdrawn after the Opus re-check. P7's premise went back to the owner, who adopted §5.3b on 2026-11-13 at about 12:10 (P7's masking rules and receipts as a verdict input dropped).

**Fable, §5.3b** — adopted. M1 went to the owner: a trial marked on the first run and on both serial re-runs makes the gate step red and the row `ERROR`, and lateness-producing mutants are killed by a direct or stepped-rig filter. The other findings were adopted into plan v3.

**Sol, plan v3 (FAIL)** — all adopted: the guard counts each trial site and `finish()`; the admission check runs first inside the rehearsal path; the minors.

**Opus, plan v4 (not closed)** — R1 and R2 adopted. The guard counts asserts outside trials, and `trial` is `#[must_use]`. N2 is INCONCLUSIVE when N1's stamp is past its deadline.

**Opus, plan v5** — closed. Residuals adopted: `_verdict(`/`_outcome(` calls counted outside spans; N2's own late drops through `late_arrival_verdict`.

**Sol, code (FAIL)** — all adopted, fix round 1:
- M1: the identity test books A late by construction and drops the timing-dependent burst count (N51-2C-58).
- M2: N2's own late drop goes through `late_arrival_verdict_of("N2", …)` on both hardware lines, and N1's line subtracts only N2's drops (N51-2C-59…66).
- Minor: a pure target test kills N47S5-192.

**Opus, re-check of fix round 1** — M1, M2 and the minor closed. The four new minors were fixed in round 2:
- m1: a test with N1 and N2 both late-dropped (N51-2C-67);
- m2: the tenth rehearsal added to the guard table;
- m3/m4: note 51 text corrected, and the "流していない・限界" record restored.

## Decided by the orchestrator

- `late_at_device` is not evidence. Clause 3 is folded into clause 2. Q15's "zero conclusive trials fails" rule is not built.
- M4 kept: a missing long-close shape with clean reports is a FAIL on hardware. The bench had 16/16 PASS. §5.3b's "実機は変えない" is narrowed to "lateness on hardware stays INCONCLUSIVE". This changes a hardware verdict; listed for the owner at landing.
- N2 is added to the hardware Session Stop scene; the next bench decides its 5 ms margin.
- N7 is wrapped in `trial` with a lateness that leaves out only C's own drop (C2 found the injected delay also made A late). This replaces the plan's "N7 never reaches the predicate".
- `Verdict` is `#[must_use]`. Four parallel front markers in C1 are the rule working (serial: 180 runs, 0 markers).
- The extra rows N51-2C-50…53 are kept.

## Not run

- The hardware lines changed by this stage (N2, M4, C = 1 000) run at the next bench.
- `rx_start_lead_group_runs_on_the_fake` is deferred to stage 3.

## Gate

PASS ([n51-2c-gate.json](n51-2c-gate.json)) on sim02:8 and sim03:8: 2754 tests, 667 UHD, `hw-build`, Python 121 × 2, e2e 16/16, mutation 2645/2646 (the known survivor `phase7/G09`), no `ERROR`, no timing re-runs.
