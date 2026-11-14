# bt1 — bench tools for maintenance table 3, cells C2–C6: review record and dispositions (2026-11-14)

Hardware tests, each with a FakeDevice rehearsal in the default suite and a pure verdict, for the cells of `plan/phase8/bench.md` "11-15 の後の手順": C2 (C1 at 100 and 200 MS/s, received at 10 MS/s), C3 (a 32-sample first send), C4 (the cascade boundary: B placed from A's first TimeError tick and compared with the fake's drain limit, UR-33), C5 (two channels, a late start: can one channel play alone, behind the slot-B gate) and C6 (the held tail starves: Underflow, then TimeError, no ACK). No production code changed. C7 (the Module-level repeat resume with an `m` sweep) needs a production hook and is the next step, bt2.

Measurement tooling, not risky (owner 2026-10-26): no test plan. Sol pre-checked the brief against the UHD 4.10 and FPGA sources before code (the hardware-brief rule). Branch `work/bt` from `e7459b8b`. Implementer: Claude Code Sonnet 5.5 subagent, effort high ([bt1-briefs.md](bt1-briefs.md)); self-review waived (Sonnet). A fresh Opus review of the sub-step, the full mutation lists (2940/2941, G09 the known survivor), Sol's code review, and Opus re-checks after each round of major fixes.

## Findings by source and severity

| Source | Major | Minor | Mechanism |
|---|---|---|---|
| Sol, brief pre-check ([bt1-sol-pre.md](bt1-sol-pre.md)) | 5 | 1 | — (brief defects before code: C4 could blame host starvation on the fake's pad and had no defined reference; C5 read the sought split as a tool failure; C6 sent the wrong close and could PASS without a TimeError) |
| Opus, sub-step review | 2 | 7 | — (C4: an unsampled band past the limit; C5: a leak could hide a split) |
| Sol, code ([bt1-sol-code.md](bt1-sol-code.md)) | 1 | 2 | — (C6: a head starved by a lost packet could PASS: sequence errors ignored) |
| Opus, re-check of fix round 1 | 1 | 3 | — (C5: a leak-hidden split outside the transition cells still read PASS) |
| Opus, re-check of fix round 2 | 0 | 4 notes | — |

No finding is a production-code defect; each was a test without power or with a host artefact in a measurement tool, so no `mech:` count changes ([failure-mechanisms.md](../../maintenance/failure-mechanisms.md)).

## Dispositions (decided by the orchestrator)

- **Scope:** C2–C6 in bt1; C7 in bt2 (needs a hook for `m` in production code).
- **Sol's pre-check, all adopted into the brief (v2):** every A send's handed time recorded and required ≥ 1 ms before the drain reaches it; trials with an Underflow or a sequence error are inconclusive; `t0` is the radio's `radio_time_d1` at `ST_TIME_CHECK` (VERIFIED in `radio_tx_core.v`) and δ a measurement coordinate with a stated uncertainty; a fixed grid with 1 µs steps from 0 to 30 µs (no adaptive search), the bracket and its mixed cells reported; a C5 split is positive evidence for the contract, and PASS needs the transition straddled; C6 sends the Module's two-sample close; C6 needs both Underflow and TimeError; C2/C3 observations print every tick and do not derive the packet length from the same gaps.
- **Opus review, all adopted:** C4's grid gains limit + uncertainty; C5 reads each channel from its reports first, and a tone heard with a late TimeError and no ACK is a suspected leak; C5's `hw` passes `EZSDR_SLOT_B_ATTENUATOR_CONFIRMED`; dedupe; rows killed by assertions.
- **Sol's code review:** M1 adopted (any sequence error makes a C6 trial inconclusive); BT1-53 retargeted. C1's reduced time reads (only after its last send) are accepted: C1 has never run on hardware, fewer control calls can only widen the last-chunk margin the verdict checks, and the shape and verdict are unchanged.
- **Opus re-check of round 1, all adopted:** a leak-hidden split in any cell makes the C5 reading at least inconclusive; C4 judges a dropped trial by its effective δ; C5's PASS headline counts decided trials; C6's Underflow filtered by tick. The round-2 re-check's notes need no change (n4's wording fixed in bench.md).
- **Rehearsal limits, accepted:** the fake loops transmit to receive only at equal rates, so C2's rehearsal cannot hear the control (its PASS path is pinned by the pure tests); C4's and C5's rehearsals run the grid's ends and the fake's boundary, not the whole bench grid (wall-time cost); the fake never splits, so C5's reading rules are pinned by hand-built outcomes.
- **Wiring:** #2's cells need a UBX in #2's slot A, which 11-15's board move empties; C5 needs both slots OBX/UBX. bench.md states the condition; the board move is the owner's.
- **`n47s5_front_rehearsal_on_the_fake`** failed once on a loaded PC and passes on sim02: the known host-lateness marker, already noted for the Phase 8-end review.
