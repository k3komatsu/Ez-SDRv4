# #96 — review record and dispositions (2026-11-11)

Issue #96: five s15 paired parity tests failed together under host load at #89's first gate. Risk: not risky (tests only, no production code). Branch `work/n96` from `fe86629`, landing on `main` as one squashed commit.

## Process

- The implementer (OpenCode2 `anthropic/claude-sonnet-5-5#high`, [n96-brief.md](n96-brief.md)) could not reproduce the failures with busy loops (seven loaded runs, 0 failures). The test-timing design probe (Fable 5.2, `~/.cache/ezsdr-fixes/2026-11-11-test-timing/`) reproduced them deterministically by sleeping 20 ms in the test thread after `wait_until` in `submit`, and showed that the late party was **the test thread** (not a participant of the manual clock), not `uhd-tx` as the issue first said. The orchestrator redirected the implementer to that injection.
- Fix: the repeats are SendAsapAndFlag as `start_repeat` compiles them; switches are received from the repeat's observed first timed send (`o1_wait_playing`) in one receipt; final waits are relative to the last receipt; the untimed-queue test names its late booking by its target. Eight tests pass 5, 20, 50 and 200 ms injected test-thread delays.
- Sol's code review ([n96-sol-code.md](n96-sol-code.md)): FAIL, two majors (scheduling windows the fix left). Fixed ([n96-fix-briefs.md](n96-fix-briefs.md)); the Opus re-check closed both, with two minors, also fixed.
- Fourteen receipt-window tests (e.g. `s15_parity_b_first_accepted_booking_rule`, `s1_parity_stop_during_padded_device_burst`) cannot be made independent of the test thread by their sequence (VERIFIED that they fail under injection; INFERRED that no sequence fixes them). They are left to the test-timing design (the driving thread becomes a manual-clock participant), which closes them by construction in the design probe.

## Findings by source and severity

| Source | Major | Minor | Note |
|---|---|---|---|
| Sol, code | 2 | 1 (pre-existing) | — |
| Opus, re-check | 0 | 2 | 1 |

## Gate

PASS ([n96-gate.json](n96-gate.json)): 2267 tests, 532 UHD, Python 121 × 2, mutation 2114/2115 (`phase7/G09` known; run on sim02 only beside lane 2).

## Dispositions

**Sol, code (FAIL)**
- Major 1, `s15_parity_provider_stop_cancels_a_pending_switch`: a 5 s test-thread pause between the switch's receipt and `Provider::stop` read `applied`. Fixed: the UHD half judges only when the switch's boundary lies more than 1 ms past the Stop's bound (the stop row's `at` plus the in-flight window and the end margin, the oracle's `stop_bound`), else prints "not established"; the property itself is pinned on frozen rigs by `ur_23_provider_stop_and_loss_cancel_the_pending_switch` (UHD) and `mr_25_s11_loss_and_provider_stop_cancel_pending_switch` (Mock, which gained the "never sent" assertion).
- Major 2, two switches were pushed separately, not as one receipt: fixed, `o1_uhd_at` adds the batch to the rig's queue under one lock; uhd-control books a whole queue in one busy loop, so no device time passes between them.
- Minor (pre-existing), the wrong-policy test checked TIME_ERROR on a second drain: fixed, one drain; the UHD allows only `send_asap` at the repeat's own target.

**Opus re-check (PASS)**
- M1, the stop test's guard rested on the unnamed 100 ms shutdown window: fixed, it compares with the Stop's bound.
- M2, the Mock half was skipped with the UHD half: fixed, the Mock half runs unconditionally.
- Note: the UHD pins the pending-switch cancellation only for an orderly stop, not `Abort` (pre-existing; left).

Mechanism: `mech:wall-clock-test-oracle` (the seventh; plan/maintenance/failure-mechanisms.md). The class is answered by the test-timing design, not by this step.
