# 2026-11-15 bench probes, part 2 — review record and dispositions (2026-11-13)

Two bench tools for the 2026-11-15 X310 session: `hw_rxzeros_pn` (owner's idea, 2026-11-13: separate a receive restart's leading zeros from a group delay by PN cross-correlation at transmit gain 0 — a continuous capture's delay against a timed restart's delay and zeros, decision rule printed before the data: group delay / shifted / neither, and inseparable for z < 3) and the cleanup drain's busy loop seen in the 2026-11-13 record suite (`drain_without_clock`, moved to tests/common/raw.rs, ends only at an `RxRecv::Failed`). Risk: not risky (measurement tooling). Branch `work/bench1115b` from `4f47f4c`. Implementer: OpenCode Sonnet 5.5 #high ([bench1115b-briefs.md](bench1115b-briefs.md)).

## Findings by source and severity

| Source | Major | Minor | Mechanism |
|---|---|---|---|
| Sol, code | 2 | 0 | — (a residual against the ideal PN reads channel filtering as a shift; a failed receive read as INCONCLUSIVE) |
| Opus, re-check | 0 | 0 + 2 notes | — |

## Dispositions

**Sol, code (FAIL)** — both adopted in fix round 2, each first as a failing test ([bench1115b-sol-code.md](bench1115b-sol-code.md)): the residual predicts the restart from the continuous capture of the same channel at the same PN phase; the first failed receive call ends the window and the trial as an infrastructure FAIL.

**Opus, re-check** — both CLOSED; the two notes (a window across the continuous capture's start phase; filter start-up after the zeros) can only give MIXED or Neither, never a false GroupDelay: kept as notes ([bench1115b-recheck.md](bench1115b-recheck.md)).

## Decided by the orchestrator

- "inseparable" for z < 3 (both hypotheses fall inside ±1 there).
- The drain ends only at a failure; an overflow or a late command keeps draining (queued samples must not be left behind).
- A separate `#[ignore]` test rather than a phase of `hw_rxzeros_sweep` (the transmit rate follows the receive rate).
- Not risky; no test plan; no self-review (Sonnet).

**Gate 1 (FAIL)** — only the Run wrapper's scan (note 51 stage 2a): the two new FakeDevice tests read the wall clock and device time themselves. Fix round 3 moved those reads into helpers in tests/common/raw.rs (`timed`, `rx_start_ahead`, `rx_run_then_stop`); the assertions are unchanged.
