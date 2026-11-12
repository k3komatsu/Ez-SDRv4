# #100 — review record and dispositions (2026-11-12)

On the `ideal` MockRadio profile (no window, no lead) a repeat switch whose boundary is the open repeat's next sample — received at the repeat's first sample, or where its last block ended — made `open_held` (crates/ezsdr-mock-radio/src/lib.rs) take the switch's booking while the repeat's was still open: a debug build panicked in `TxBook::take` ("a booking is already open"); a release build overwrote the booking and rejected the new burst's start (MR-15 "not contiguous", MR-24 "unclosed burst"). Now `open_held` ends an open booking at that sample in one path: the tracker's burst closes by `BurstTracker::stop` (a record if it sent a block), the device model closes, the booking ends (`DeviceEnd::Keep`), then the switch's booking is taken; the switch row settles `applied` as before. The cause was the Mock's use of `TxBook`, not `TxBook`: `x3xx` cannot reach it (its window and lead put the boundary after the repeat's first sample) and the UHD Provider ends an open booking through `join_open` before `take`. MR-15 and MR-42 amended. Risk: not risky (the Mock only; the UHD path does not reach it). Branch `work/n100` from `e449397`, one squashed commit. Implementer: OpenCode Sonnet 5.5 #high ([n100-briefs.md](n100-briefs.md)).

## Process

- First run: failing test, fix for the unsent repeat only, rows N100-1, N100-2.
- Sol's code review ([n100-sol-code.md](n100-sol-code.md)): FAIL, one major.
- Fix round 1: one path for an open repeat whether or not it sent a block; rows N100-3, N100-4; every row dies on an observable assertion (no rejections, records), checked with debug assertions off.
- Opus re-check ([n100-recheck.md](n100-recheck.md)): major closed, three minors.

## Findings by source and severity

| Source | Major | Minor | Mechanism |
|---|---|---|---|
| Sol, code | 1 | 0 | — (an incomplete fix of a lifecycle transition: the open booking ended only when it had sent nothing; not one of the counted mechanisms) |
| Opus, re-check | 0 | 3 | — |

## Dispositions

**Sol, code (FAIL)** — adopted, fixed in fix round 1: after the repeat's first block `[2000, 4000)`, a switch received at 4 000 still reached `take` with `open.first == false`; the `first` guard is gone, the tracker and the device model close with the booking, and Sol's sequence is a case of `mr_16_a_switch_at_the_open_repeat_s_next_sample_replaces_it`.

**Opus, re-check** — closed. Minors:
- F1 adopted (by the orchestrator): MR-15's END_OF_BURST exception names a switch at the burst's next sample; the test asserts every record ends `Stop`. The `end: Stop` of a burst replaced by a switch is what UHD's join already records (crates/ezsdr-radio-uhd/src/provider/tx.rs, `end_open_at`); not changed here.
- F2 rejected: the fix does not touch the channel segments the radiated samples come from (channel.rs), and the existing switch tests pin records, not samples; a sample-level switch test is no part of #100.
- F3 adopted (by the orchestrator): `debug_assert_eq!(open.next, next)` keeps the tripwire; the comment says UHD keeps the device burst where the Mock closes it. N100-1 re-anchored to the new block (same mutant: the block removed).

## Decided by the orchestrator

- Not risky (no test plan); the full mutation lists ran at the gate only, not before Sol's review (four rows, one crate).
- The minors F1 and F3 fixed by the orchestrator; F2 rejected as above.

## Gate

PASS ([n100-gate.json](n100-gate.json)) on sim02:8 and sim03:8: 2512 tests, 619 UHD, Python 121 × 2, e2e 9/9, mutation 2330/2331 (the known survivor `phase7/G09`), `killed (bound)` A1-1, A1-2, E2A2A-10, N51A-14, N51A-15, N51B-08, N51C-06, `phase7/G16`, no `ERROR`, no timing re-runs.
