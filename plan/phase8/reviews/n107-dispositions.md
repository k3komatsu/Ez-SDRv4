# #107 / #109 — dispositions and counts

Two tests whose verdict depended on the host's real time: #107 (`mt3_c4_rehearsal_…`'s coverage depended on the manual clock's startup phase) and #109 (the t200 script's self-check judged "hung" from a wall-clock budget). Not a risky step. Brief: [n107-brief.md](n107-brief.md); report: [n107-report.md](n107-report.md); gate: `n107-gate.json` (beside this file).

## Counts (source, severity, mechanism)

| Source | Major | Minor | Note | Mechanism |
|---|---|---|---|---|
| Sol 6.2, code | 3 | 1 | 2 | none counted (fixture design of the step's own self-check) |
| Opus re-check, fix round 1 | 0 | 1 | 7 | none |

Both issues are `mech:wall-clock-test-oracle` (labelled with their reasons). The mechanism's answer stands (the manual clock, #60; events instead of budgets); #107's root cause — the FakeDevice's manual clock takes its zero from the host clock and runs at real speed until a cell drives it — is noted for the Phase 8-end review of the test architecture (a fake fault relative to the stream's start; 4b-2b's review n2 found the same phase in two parity helpers).

## Decisions (orchestrator)

- #107 labelled `mech:wall-clock-test-oracle` (the manual clock's zero comes from the host clock and runs at real speed until the cell drives it).
- The self-check failing when its parent shell ignores SIGINT/SIGHUP: pre-existing, noted; the gate's container launches it normally.

## Sol 6.2's code review (sol-code.md, 00:12–00:18): FAIL, 3 majors (all on the t200 self-check), 1 minor, 2 notes — dispositions
- #107's fix confirmed (note 5): kept as is.
- F1 major (the early-SIGINT case can pass after a late signal; the stub publishes readiness before installing its TERM disposition; a shared PID file across cases): adopted — readiness published after the trap is installed; the container's creation held until the kill attempt is observed; the order asserted; per-case PID state.
- F2 major (a host stall can still give an ordinary assertion failure in the 1 s expiry probes and in the signal cases' window): adopted with this rule — every self-check outcome that a host stall can cause is reported with an `EZSDR-INFRA:` line on the self-check's own stderr (so the Rust test's failure message carries it and the runner reads it as infrastructure), and never as a plain mismatch; a real defect (a script that does not stop, a lost marker on a real expiry, a wrong call order) still fails plainly.
- F3 major (a backstop expiry starts a second wait so the marker may come after the runner's own timeout; an exit-wait expiry leaves the `timeout` job and the stub alive): adopted — a case stops at its first expired wait and reaps the processes it owns before returning; the exit event is the child's completion status (`wait`), not `kill -0`; the backstop is short enough that the marker is printed well inside the mutation runner's per-test timeout (`plan/maintenance/tools/mutate.py`).
- F4 minor (the late-event probe can pass without polling twice): adopted — a predicate that is false once, then true.
- Proportion: this is a bench tool's self-check; fix what the findings name, no new framework.

## Opus re-check of fix round 1 (recheck-fix1.md): PASS with minors — dispositions
- Finding 1 (a cleanup defect reported as a host stall): adopted — the 9 s floor on the stall condition. Notes 1 and 3 adopted (redundant `hung` checks dropped; 124 or 137). The fifo watchdog stays (the implementer's choice of `wait`; the alternative is a note only). Other notes: no change.
- Not counted against a mechanism: the review findings are fixture-design gaps in the step's own change.
