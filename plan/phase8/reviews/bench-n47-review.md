# Bench preparation after note 47 stage 3 — review record (2026-11-05)

Step: hardware tests and the day's runbook for the bench between note 47 stages 3 and 4 (handoff §2 (7) ③). Not risky (bench-only test code, no `src/` change): no test plan. Implementer `opencode-go/deepseek-v4.1-flash#max` (owner 2026-11-05), one self-review round (free DeepSeek), Sol pre-check of the brief and Sol code review, Opus re-checks (effort high) after the fix rounds. Brief: `bench-n47-brief.md`; dispositions: `bench-n47-dispositions.md`; gate: `bench-n47-gate.json`; dropped tests: `bench-n47-renames.txt`.

## What the step adds

- Deleted `hw_p4_rx_timed_tune_then_early_stop` and `hw_l1_held_rx_tune_applies_when_its_segment_streams` (they submitted a receive change during a Run, removed in stage 2) and their helpers.
- `hw_n47_rx_restart_with_new_values` (T2, the Module path of stage 3), the restored receive start-lead group with `hw_n47_rx_start_lead_group` (T3), `hw_n47_raw_rx_rate_on_an_open_streamer` and `hw_n47_raw_tx_rate_on_an_open_streamer` (T4, note 47 §1's acceptance condition), `hw_n47_raw_tx_setting_after_a_burst_end` (T5, stage 4's INFERRED margin; the −5 ms disruptive control behind `EZSDR_N47_T5_CONTROL=1`), `hw_n47_raw_control_call_durations` and `hw_n47_raw_rx_buffer_capacity` (T6, inputs of the Module-form note). Pure verdicts in `tests/common/n47.rs` with unit tests; rows N47B-01…41, P994 re-anchored.
- `plan/phase8/bench.md` §「2026-11-05 の手順（note 47 段階 3 の後）」 (blocks (a)–(h)); the 2026-11-01 section marked superseded.

## Findings by source

| Source | P1 | P2 | P3 | Notes |
|---|---|---|---|---|
| Sol pre-check of the brief | — | — | — | BRIEF NEEDS CHANGES: T2, T4–T7 changed (stream-scoped Stop, drain to quiescence, ACK-confirmed burst end, calibrated loop delay, direction-specific coverage for timed commands, censored buffer outcomes). One item rejected (throwaway-copy probes are allowed by AGENTS §9) |
| Self-review (free DeepSeek) | — | 8 | 6 | 13 fixed, 1 rejected with reason (implementer's report) |
| Sol code review | 1 | 12 | 1 | all adopted; P1 (timed transmit campaign could leave DUC commands queued) closed by removing the trial, not hardening it; the ≤ 0 dB gain rule was the orchestrator's over-restriction (owner allows transmit gain to max with the attenuator confirmed), T5 uses 0 → +6 dB |
| Opus re-check 1 | — | 2 | 6 | N1 ACK window ±1 tick unverified → `[e − 2 samples, e + 1]`, outside INCONCLUSIVE; N2 a lost timed receive apply ends the probe with a power-cycle message, no further device command |
| Opus re-check 2 | — | — | 3 | PASS; the three P3 (missing B ACK is INCONCLUSIVE, A's duplicate counted after the last drain, move tick in the transmit frame) fixed by the orchestrator, row N47B-41 |

Mechanisms (plan/maintenance/failure-mechanisms.md): none counted. The pre-check and code-review findings are hardware assumptions of a measurement brief (the lesson recorded in memory "Sol pre-check for hardware briefs"), not a design mechanism of the Module.

## Gate

`gate.py --mutation all --base d27718a --renames plan/phase8/reviews/bench-n47-renames.txt`: see `bench-n47-gate.json`. A first gate (before the orchestrator's three P3 fixes) passed with mutation 1751/1752 (the known survivor phase7/G09).

## Not verified

No hardware run. RF settling inside the start lead, the burst-ACK tick against the burst end, the `e + 5 ms` margin, buffer capacity and the 50 MS/s baseline are what the bench measures.
