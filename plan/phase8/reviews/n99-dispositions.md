# #99 — review record and dispositions (2026-11-13)

After a finite TX booking was abandoned (UR-22: a short, failed or stalled send, an underflow or sequence-error report, an expired timeline), the record still treated its end as joinable: a booking at that end was accepted as a continuation and sent as its own closed burst, and a later booking accepted at the predicted front was dropped (or moved) at dispatch with `radio.TIME_ERROR`. Now the abandon of a finite booking is an owner's close for the record (`Bookings::end_finite`, crates/ezsdr-radio/src/staging.rs, called from `TxBook::abandon` and `Tx::abandon` in crates/ezsdr-radio-uhd/src/provider/tx.rs): the abandoned booking's chain takes no join and keeps its footprint (KL-39's rule), and pieces joined behind it before the abandon was known become a chain of their own by the head-leaves rule (`Chain::left_without_head`, extracted from `remove_head`). The write never touches what a `Stop`, `Provider::stop` or a device loss already ended (it is skipped while a halt is unread and when the device is lost), and it finds the chain by the abandoned piece's last sample, so a pruned join point cannot hide it. A booking at the abandoned end is refused with `UR-21: the burst overlaps an accepted booking`; a booking accepted before the abandon section and judged against the old prediction can still be dropped at dispatch (the residual race, KL-39). The Mock never abandons and is unchanged. RM-15, UR-23, KL-38, KL-39, note 49 §5.4.4. Owner's decision (2026-11-13): direction B. Risk: risky (the record's lifecycle across a Stop and a loss). Branch `work/n99` from `829b7b0`, one squashed commit. Implementer: OpenCode Sonnet 5.5 #high ([n99-briefs.md](n99-briefs.md)); test plan [n99-test-plan.md](n99-test-plan.md).

## Process

- Reproduction, the twelve paths where the record's prediction and the device chain can disagree, five options with a recommendation, and a reference sketch (read-only run).
- Owner's decision: B (the abandon closes the record; the footprint stays; a booking at the end is refused).
- Fable 5.2 design review of B ([n99-fable-design.md](n99-fable-design.md); claude-fable-5-2, about 175k tokens): sound with changes, F1 major and F2, plan and text corrections; all adopted.
- Plan amended; Sol's plan review ([n99-sol-plan.md](n99-sol-plan.md)): PASS WITH CHANGES, three minors, all adopted.
- Implementation in three sub-steps (refactor, record, Provider and text), one run; 39 rows run, all killed.
- Full mutation lists before Sol's code review: 2393/2394 (only `phase7/G09`).
- Sol's code review ([n99-sol-code.md](n99-sol-code.md)): PASS WITH CHANGES, one minor, fixed by the orchestrator.

## Findings by source and severity

| Source | Major | Minor | Mechanism |
|---|---|---|---|
| Fable, design | 1 | 1 + plan/text | `planned-state-read-as-actual` (F1: the abandon's record write read a chain a halt had already ended; F2: a pruned join point stood for the chain) — design defects of the sketch |
| Sol, test plan | 0 | 3 | — (a halt exception too broad, KL-39's front, a composition test) |
| Sol, code | 0 | 1 | — (UR-23 still stated the broad exception) |

#99 is `mech:planned-state-read-as-actual` (count 16 → 17 in failure-mechanisms.md): the record's join flag is a prediction, and the fix writes it at the event that makes it false, inside note 49's D′ record, not beside it.

## Dispositions

**Fable, design (sound with changes)** — all adopted: F1 the write is skipped while a halt is unread in the list and when the device is lost (`TxList::halted`, `core.is_lost()` read in the record's section); F2 `end_finite(e)` finds the chain holding `e − 1` below its front and splits at `e`; S9 corrected (a Stop after an abandon gives a re-framed standalone piece its region `[t, B)`, RM-16); the report-driven abandons are the primary rig cases (a short send returns only after the 1 s send timeout on hardware); the text corrections to RM-15, UR-23, KL-39 and KL-38. B+ (a grow-only front for a late abandon) and a dispatch stat not taken.

**Sol, test plan (PASS WITH CHANGES)** — all adopted: m1 the halt exception narrowed to "a finite abandon does not rewrite what a halt already ended" (a Stop, a new booking, its abandon: that booking's join ends); m2 KL-39's front is the retained RM-15 front (a short standalone head keeps padded end + g: one sample at 10 000 holds to 10 019); m3 the composition test `n99_p8`. `end_finite(e)` without `k` accepted (a waiting device burst's abandon has no open booking).

**Sol, code (PASS WITH CHANGES)** — fixed by the orchestrator: UR-23's sentence now carries the narrowed exception.

## Decided by the orchestrator

- The adoptions of Fable's and Sol's findings above; risky (Fable).
- S9 (a Stop after an abandon gives a re-framed standalone piece the Stop's region `[t, B)`): an application of B and RM-16, listed for the owner at landing.
- The changed expectations limited to `e3b_c1`'s F3 receipt column and `e3b_c6`'s F4 half (moved to U1, the test renamed `e3b_c6_f1_as_it_is`).

## Gate

PASS ([n99-gate.json](n99-gate.json)) on sim02:8 and sim03:8: 2597 tests, 634 UHD, `hw-build`, Python 121 × 2, e2e 14/14, mutation 2393/2394 (the known survivor `phase7/G09`), `killed (bound)` A1-1, A1-2, E2A2A-10, J317, J318, N51-2A-21, N51A-14, N51A-15, N51B-08, N51C-06, P880, `phase7/G16`, `phase7/R15`, no `ERROR`, no timing re-runs.
