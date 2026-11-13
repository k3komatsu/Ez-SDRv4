# Note 51 stage 2c — Opus re-check of fix round 1 (2026-11-13)

Read-only re-check (Claude Opus subagent, effort high) of commit `21b7683` against Sol's code review ([n51-2c-sol-code.md](n51-2c-sol-code.md)). The plan's earlier Opus re-checks (plan v2: not closed, N0–N3; plan v4: not closed, R1/R2; plan v5: closed) are summarised in [n51-2c-dispositions.md](n51-2c-dispositions.md).

## Verdict per Sol finding

- **M1 — closed (VERIFIED).** `one_sample_pair_returns_the_manifest_its_time_errors_were_read_from` books A 1 ms ahead (past its 2 ms deadline) and asserts a `(late, drop)` exists, `time_errors == time_error_pairs(manifest)` and the booked names `["A","B"]`; the burst-count assertion is gone. Host delay cannot remove A's drop: the late decision is taken against the Provider's receipt clock (`control.rs:481-490`). The identity check now compares a non-empty list (N51-2C-58 kills it). Copy run: passes with no injection and at now, submit and advance, 60 and 200 ms.
- **M2 — closed on both hardware lines (VERIFIED code path; hardware not run).** `hardware.rs:3844-3845` → `stop_next_verdict` / `stop_after_margin_verdict` (`n47s5.rs:737-772`), Kernel refusal first; N2's own late drop → `late_arrival_verdict_of("N2", …)` INCONCLUSIVE; N1's line subtracts only N2's own late drops (`without_late_drops`). `n2_late` matches target ticks equal to `next2_k`, which holds for a drop at receipt (`control.rs:463`) and at dispatch (`tx.rs:988`, `:1326`).

  | Sequence | Required | Killer |
  |---|---|---|
  | N2 late drop alone | N2 INCONCLUSIVE, N1 not FAIL | fake test + rows -59, -60, -63 (die only when the trial is judged; under load ERROR, not a false kill) |
  | N2 late + another TIME_ERROR (incl. N1's own late drop) | FAIL | pure test (`time_errors: 2`) + existing rows |
  | N2 late + underflow or device fault | both lines FAIL | guard rows, pure test (`async_faults`) |
  | `n2_late` read at the wrong target | — | `assert_eq!(n2_late.len(), 1)` in both fake tests |
  | N1's line subtracts more than N2's drops | N1's own late drop still FAILs | **nothing** → m1 |

- **Minor (N47S5-192) — closed.** Pure test `late_arrivals_read_only_the_late_drops_at_the_booking_s_target` (targets 100 and 200); deterministic.

## Rows
Deterministic killers: -58, -61, -62, -65, -66, N47S5-192, and -64 (direct `n2_late.len() == 1`). Die only when judged: -59, -60, -63 (10 quiet serial runs, 0 markers). -64's new mutant submits N2 at about T − 5.5 ms, ahead of the drop point (about T − 3 ms), so it tests that the trial really makes N2 late.

## The 0.5 ms lag
Cannot flip into a plain failure: the drop is decided by the receipt rule from the Provider's receipt clock; host delay only makes it more certain. The T − 3 ms threshold stays INFERRED (fake only). Sweep: now/submit at 60/200 ms → the N2 rehearsal fails with the marker only; advance passes; quiet serial runs 30/30.

## New findings (all minor)
- **m1** (test power): nothing kills an over-subtracting mutant in `stop_next_verdict` (`n47s5.rs:747`): N2 and N1 both late-dropped → count 2 → with `+ 1` → 0 → N1's line no longer FAILs on its own drop.
- **m2** (guard): the new tenth rehearsal `n47s5_stop_n2_late_drop_is_host_latency_on_both_lines` is missing from `REHEARSALS` (`governance.rs:820-830`).
- **m3** (note 51 false): `51-test-timing.md:231` "2c は N1 の行を変えず".
- **m4** (note 51 record lost): the fix-1 bullet at `:576` replaced the "流していない・限界" bullet.
- Incomplete text: lines 20, 205, 572 on `late_arrival_verdict` / N2.

Observation (pre-existing, INFERRED): a host stall of about 5–7 ms before N1's submit on hardware late-drops N1 and FAILs both lines plainly; the brief keeps N1's own late drop a FAIL.
