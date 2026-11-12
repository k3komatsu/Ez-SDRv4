# #67 bench probe — review record and dispositions (2026-11-13)

A raw bench probe `hw_67_stop_before_origin` (crates/ezsdr-radio-uhd/tests/hardware.rs; its verdict `cancel_trial_verdict` in tests/common/iii.rs with pure tests in the default suite) for issue #67, to run on 2026-11-15: on one receive channel at 1 GHz and three rates, ten trials per cell, a timed `rx_start` for an origin about 2 s away is cancelled before the origin by an untimed `rx_stop` (`stop_before_origin`) or by closing and reopening the streamer (`close_before_origin`), with the present VH-6 path (stop after the origin) as the control; a trial PASSes when no block of the cancelled start arrives, the cancel returned before the origin, observation reached origin + 500 ms and the next start's first block is at its new origin. Receive only, no transmit; one 15-minute bound covers the device's whole life. Sol's reading of UHD v4.10 (VERIFIED): STOP bypasses and clears the radio's acquisition-command FIFO, so `stop` is expected to PASS; a close sends no STOP, so `close` is the open question. The bench entry is in plan/phase8/bench.md. Risk: not risky (measurement tooling, owner 2026-10-26); Sol's code review kept. Branch `work/n67probe` from `a9123cb`, one squashed commit. Implementer: OpenCode Sonnet 5.5 #high ([n67probe-briefs.md](n67probe-briefs.md)).

## Findings by source and severity

| Source | Major | Minor | Mechanism |
|---|---|---|---|
| Sol, code | 2 | 2 | — (a probe verdict that could pass a cancel returning after the origin; a hang bound not covering the device's creation and drop) |
| Opus, re-check | 0 | 4 + 1 cosmetic | — |

## Dispositions

**Sol, code (FAIL)** — all adopted, fix round 1: the cancel's entry and return instants recorded, a distant cancel returning at or after the origin FAILs, no tail allowance for a cancelled start, observation must reach origin + 500 ms (else INCONCLUSIVE), a `time_now` failure is not silent (Sol's counterexample is a pure test, failing before the fix); one bound around the device's whole life; bench.md separates a trial FAIL from evidence that cancelling is unsafe; the `uhd_kept_count` leak claim dropped (it cannot see a streamer leak while the device lives).

**Opus, re-check** — closed. Minors fixed by the orchestrator in bench.md (TRACE field names, the `EZSDR-INFRA` grep, an all-INCONCLUSIVE run exits 0, what a close PASS shows). Cosmetic message not changed; `RECORD_LATCH` noted for if the probe ever joins the record suite.

## Decided by the orchestrator

- The probe is built now for the 2026-11-15 session (the 2026-11-13 draft found no test that stops before the origin); not in the record suite.
- Not risky; no test plan; the full mutation lists run at the gate.

## Gate

PASS ([n67probe-gate.json](n67probe-gate.json)) on sim02:8 and sim03:8: 2609 tests, 634 UHD, `hw-build` (the probe compiled with `--features uhd`), Python 121 × 2, e2e 14/14, mutation 2408/2409 (the known survivor `phase7/G09`), `killed (bound)` A1-1, A1-2, E2A2A-10, J317, J318, N51-2A-21, N51A-14, N51A-15, N51B-08, N51C-06, P880, `phase7/G16`, `phase7/R15`, no `ERROR`, no timing re-runs.
