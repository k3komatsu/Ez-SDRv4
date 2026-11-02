## Verdict: REVISE

`TP` = amended `test-plan-e2a.md`. Code paths below are relative to `crates/ezsdr-radio-uhd/src/`. Superseded text is not counted as a separate defect.

### Earlier findings

| Finding | Closed? | Evidence |
|---|---|---|
| #1 — observations and duplicates | Yes, at plan level | `TP:247,265` adds evidence identity, leftovers, discarded counts, field-isolation tests, and malformed-log rejection, including cross-list duplicates. |
| #2 — missing witnesses | Yes | `TP:251` requires complete coverage before closure; smoke cannot substitute. `brain/cases.rs:630–635` already rejects missing witnesses. |
| #3 — production clock/Q1 | Yes | `TP:250` specifies a no-`L` acceptance source and retains differences as findings. Matches `provider/core.rs:1233–1234`, `authority.rs:240–251`. |
| #4 — locks/timers | Partial | `TP:248–249,263,269` supplies permit/ack, actual lock logs, independent timer delivery and clock contention. The acquisition-order test still has an illegal sequence: F1. |
| #5 — legal S3/S4 contracts | Yes, at contract level | `TP:252` corrects abandonment, guard operations and Failed/Lost. `brain/world.rs:875–890,516–517` agrees. Runnable prefixes remain implementation-review work. |
| #6 — S7/S10 vacuity | Yes, at contract level | `TP:254,256` requires exact application/evidence ticks and a genuinely shortened retiring cut with exact publication/discard results. |
| #7 — conformance | No | `TP:273–282` supplies scripts, consumption observation and narrow waivers, but several expected results are incorrect or ambiguous: F3. |
| #8 — smoke and killers | Partial | `TP:253,258,269–271` improves smoke, clock contention, direct helper coverage and controller tests. Cleanup setup remains defective: F2. |
| N1 — acquisition authorization | Partial | The permit-before-acquisition/acknowledgment contract at `TP:263` addresses the original deadlock. Its named test and E2-31 killer need repair: F1. |
| N2 — duplicate-rejection self-tests | Yes, at plan level | `TP:265` explicitly requires rejection of identical records in each category and across consumption lists; rows target those checks. |
| N3 — executable killer sequences | Partial | `TP:269–271` makes clock contention, persisted-`seen` consumption and invalid-controller tests concrete. S6b is not yet a legal cleanup witness: F2. |
| N4 — conformance expectations | No | `TP:273` identifies `ConsumeProbe`; `TP:279–281` makes persistence and acceptance observations concrete. Other scripts still cannot serve as reliable conformance assertions: F3. |

### New or remaining findings

**F1 — P2: acquisition-order test A skips a required event.**  
`TP:263` starts R0 parked at `RCmd`, but order A proceeds directly from `SUnlock` to `RLock(0)`. `RCmd(0)` must first install the lock/call program (`brain/world.rs:510,515,977–990`).
The correct runtime therefore rejects A before testing acquisition order. Insert `RCmd(0)` in A, and state the corresponding executable setup for C. Keep E2-31’s killer on a legal baseline.

**F2 — P2: S6b does not establish its asserted cut or Restore.**  
`TP:268` requests Stop at 2, then expects cut 4 and publication of ticks 0..3 without establishing progress 4 before Stop. Publication uses the bound already booked; it does not retroactively raise it (`brain/rules.rs:22–29`, `brain/cell.rs:304–319`).
More decisively, no Restore request or preloaded deferred Restore appears in the setup. Stop does not invent one; the existing cleanup fixture explicitly inserts it (`brain/world.rs:2045–2054`).
Specify initial time/progress/buffering, book the intended cut, and enqueue Restore explicitly. Assert `Reported(Left)` plus separately observed leftover consumption, rather than allowing either disposition.

**F3 — P2: conformance scripts contain false baselines.**  
- `TP:274`: the second running recv precedes Stop and can wait for a complete packet, consuming the supposed tail. Remove the tentative recv and stop before requesting the tail.
- `TP:275`: `[5..?]` is not an expected result; at T=14 only ticks 12 and 13 have been buffered by the model. A four-sample return requires further advancement. Specify every recv’s timeout and resulting T (`brain/model.rs:135–149,258–260`).
- `TP:276`: preceding timeout calls advance time, so start-now cannot still be assumed to start at 10 or 11. Capture its actual invocation time.
- `TP:277`: after application at 5 and 6, the two older FIFO entries are gone. All 32 new writes fit; the **33rd** fails, not a write after only 30 acceptances (`brain/model.rs:109–129,170–180`; `device.rs:1437–1447`).
Correct these schedules before using their failures to diagnose model defects or count mutation kills.

Not checked: implementation or exact mutation anchors, builds, test/mutation execution, hardware, or the prohibited holdout directory.
