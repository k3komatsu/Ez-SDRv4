# #77 — review record and dispositions (2026-11-14)

After a loss, a Stop or `cleanup`, MockRadio records the faults that never fired in instant order, document order breaking ties (SE-4), as the timeline oracle expects (MR-20, MR-27). It recorded them in document order. The test generator's Fault class now includes a loss half the time, so the Mock differential `rm_26_the_timeline_against_mockradio` reaches the case.

Not risky (a record order in MockRadio and the test generator): no test plan. Branch `work/n77` from `d0a0df2`, squashed. Implementer: Claude Code Sonnet 5.5 subagent, effort high ([n77-brief.md](n77-brief.md)).

## Reproduction (before the fix)

With a loss allowed in the Fault class, the differential failed on 6 of 1000 seeds. The smallest case needs no Action and no running stream: faults booked `Overrun @3 ms, Sequence @2 ms, Loss @1 ms`. The oracle expects `[device_lost, rx_sequence_error, rx_overflow]`; MockRadio wrote `[device_lost, rx_overflow, rx_sequence_error]`, on both profiles. `record_pending_faults` looped over the faults in document order. The spec was silent on this order (MR-20, MR-27; SE-4 orders the faults that fire).

## Findings by source and severity

| Source | Major | Minor | Mechanism |
|---|---|---|---|
| Orchestrator (diff read) | 0 | 0 | — |
| Sol, code ([n77-sol-code.md](n77-sol-code.md)) | 0 | 1 | — (the order was stated for a loss only, though a Stop and `cleanup` record pending faults too) |

## Dispositions (decided by the orchestrator)

- **The order:** instant order with document order breaking ties, the order the faults would have fired in (SE-4). The UHD Module keeps no list of pending faults (Sol, VERIFIED), so only MockRadio changes.
- **Sol's minor:** the sentence moved from MR-20 to MR-27, for every way a Run can end before a fault fires.
- **Generator:** the Fault class appends a loss half the time; the draw comes last in `sequence()`, so the other classes' seeds are unchanged and the Loss class keeps its meaning.
