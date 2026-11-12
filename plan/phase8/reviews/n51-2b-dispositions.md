# Note 51 stage 2b — review record and dispositions (2026-11-13)

The tests whose verdict changed under 2a's injected delays and whose move needs no Provider receipt instant are moved off the test thread's timing (plan/phase8/51-test-timing.md §7.2, the R table of plan/phase8/51-tables.md §3): `ur_21_a_refused_configure_refuses_its_burst_through_the_kernel` is split into an untimed Run half (the Kernel's Action id is the Provider's; the corrected value is taken before the next send) and a new direct-rig test of the window (a refused burst keeps its span to its booked end); `ur_21_s18`, `ur_23_a_burst_after_a_stop…` and `ur_29_a_loss_found…` each book one burst untimed or as a repeat; `ea_16_untimed_repeat_after_stop…` measures from the Stop's own log stamp; `ur_29_a_silent_stream…` and `ur_33_a_shutdown…` lose their time reads and their allow-list rows (24 → 22). The scan forbids `assemble_on` outside crates/ezsdr-acceptance/tests/uhd.rs (the `support/` Run path the 2a re-check noted); `bounded()` and the `entered.recv_timeout` handshake write the infra marker. Thirteen tests that need a receipt instant go to stage 2c; `ur_21_a_descheduled…` to stage 3. Every moved test passes with 200 ms injected at `now`, `advance` and `submit` together. The 2a record's count is corrected (14 tests, 13 `fake` + 1 `uhd`). Risk: not risky (test code only). Branch `work/n51-2b` from `829b7b0`, one squashed commit. Implementer: OpenCode Sonnet 5.5 #high ([n51-2b-briefs.md](n51-2b-briefs.md)); plan [n51-2b-test-plan.md](n51-2b-test-plan.md).

## Process

- Plan (read-only run) with probes in a copy; the orchestrator's decisions (briefs, "plan-decisions").
- Implementation in one run; it stopped waiting for a mutation-run notification after the full lists had finished (2382/2383, only `phase7/G09`); the orchestrator killed its client by PID and a continuation run re-verified and reported (no further edits).
- Sol's code review ([n51-2b-sol-code.md](n51-2b-sol-code.md)): FAIL, one major, two minors, fixed by the orchestrator; Opus narrow re-check ([n51-2b-recheck.md](n51-2b-recheck.md)): closed.

## Findings by source and severity

| Source | Major | Minor | Mechanism |
|---|---|---|---|
| Sol, code | 1 | 2 | — (a direct-rig test returning without finishing its rig, leaving uhd-control running; two text overstatements) |
| Opus, re-check | 0 | 0 | — (note: `Direct` has no `Drop`, pre-existing) |

## Dispositions

**Sol, code (FAIL)** — fixed by the orchestrator: the new direct-rig test calls `direct.finish()` after its observations and before its asserts (row N51-2B-06 re-run, killed); note 51 now says the `ea_16` bound from the Stop's log stamp does not show the interval from the actual receipt (a stall after the stamp is unbounded) and leaves the exact window to 2c's `receipts`; the deferred set's text says 9 of the 13 failed in the sweep and `n47s5_stop_end_rehearsal_on_the_fake` needs the Kernel log.

**Opus, re-check** — closed. Note (`Direct` without `Drop`; a panic before `finish` leaks in every `Direct` test): to the Phase 8-end review of code and test architecture.

## Decided by the orchestrator

- Scope, the deferrals to 2c and stage 3, the `bounded()` marker, not risky (no Sol plan review), one run and one commit.
- The stage-0 tables' "Planned move" column and §6.1 rows are not rewritten (they are the stage-0 record); note 51's "段 2b の後" paragraph holds the result.

## Gate

PASS ([n51-2b-gate.json](n51-2b-gate.json)) on sim02:8 and sim03:8: 2611 tests, 634 UHD, `hw-build`, Python 121 × 2, e2e 14/14, mutation 2414/2415 (the known survivor `phase7/G09`), `killed (bound)` A1-1, A1-2, E2A2A-10, J317, J318, N51-2A-21, N51A-14, N51A-15, N51B-08, N51C-06, P880, `phase7/G16`, `phase7/R15`, no `ERROR`, no timing re-runs.
