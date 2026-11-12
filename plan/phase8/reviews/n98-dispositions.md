# #98 — review record and dispositions (2026-11-12)

A TX booking whose configure returned `lost: true` (the device lost between two channels' applies) was refused: `complete_configure(id, false)` took it out of the held bookings and dropped its region, and the loss emitted `DEVICE_LOST` but nothing named the booking, while a loss just before or after the configure left it held and `Provider::stop` cancelled it with `UR-26: cancelled by Provider::stop`. Now `configure_turn` (crates/ezsdr-radio-uhd/src/provider/tx.rs) decides by the configure's own `error.lost`: a lost configure keeps its booking and footprint (RM-15) for the loss path, which gives it the note a held booking gets (`cancelled by Stop` when a Stop was queued); the loss is still published once (`device_failed(None, …)` when the device reported it first). An ordinary refusal — also one racing a loss published by another thread — stays a refusal (R5). With two held bookings and a loss in the first one's configure, both now get the note (before, the first vanished). UR-26 and RM-15 amended; note 49 §5.4.4 and §5.4.6; the #98 row of failure-mechanisms.md. Owner's decision (2026-11-12, in #98): the booking stays in the book through its configure and the loss path accounts for it. Risk: risky (a lifecycle). Branch `work/n98` from `13364eb`, one squashed commit. Implementer: OpenCode Sonnet 5.5 #high ([n98-briefs.md](n98-briefs.md)); test plan [n98-test-plan.md](n98-test-plan.md).

## Process

- Test plan with out-of-tree probes; it corrected the brief: since E2b the booking is already held through its configure, and the fix is the result's handling in one place.
- Sol's plan review ([n98-sol-plan.md](n98-sol-plan.md)): PASS WITH CHANGES, three minors, all adopted.
- Implementation in one run; rows N98-01, -03, -04, -05, E2B-13 and E2B-16 retargeted, N47S4-14 and N47S4-56 re-anchored (same mutant), touched rows run: 12/12 killed.
- Full mutation lists before Sol's code review: 2334/2335 (only `phase7/G09`).
- Sol's code review ([n98-sol-code.md](n98-sol-code.md)): PASS, no findings. (Its first launch hung at start-up with no output for 30 minutes and was killed by PID and relaunched.)

## Findings by source and severity

| Source | Major | Minor | Mechanism |
|---|---|---|---|
| Sol, test plan | 0 | 3 | — (test coverage and row-killer claims) |
| Sol, code | 0 | 0 | — |

#98 is labelled `mech:stream-lifecycle-scattered`; the fix keeps the decision in one place (the configure's own `lost` bit decides both the book and the events), recorded in failure-mechanisms.md.

## Dispositions

**Sol, test plan (PASS WITH CHANGES)** — all adopted: m1 a `Provider::stop` that books the end before a lost configure completes (`n98_a_provider_stop_before_a_lost_configure_completes_cancels_the_booking_once`); m2 N98-01 and N98-05 name their real killers (T4, T2's device-reported-loss mode), not C4; m3 the no-retry assertion counts the configure's `apply` attempts, and events are read after shutdown too. The normative sentence says "a configure that returns `lost: true`".

## Decided by the orchestrator

- The implementer's correction of the brief accepted; T5 (a book-level pending-switch case) dropped: `complete_configure(id, true)` is already pinned and a pending switch cannot coexist with a configure turn (INFERRED, Sol agreed).
- No N98-02 (it would duplicate E2B-13).

## Gate

PASS ([n98-gate.json](n98-gate.json)) on sim02:8 and sim03:8: 2516 tests, 623 UHD, Python 121 × 2, e2e 9/9, mutation 2334/2335 (the known survivor `phase7/G09`), `killed (bound)` A1-1, A1-2, E2A2A-10, N51A-14, N51A-15, N51B-08, N51C-06, `phase7/G16`, no `ERROR`, no timing re-runs.
