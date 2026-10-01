# Review U — the RS-36/KC-31 Kernel fix (`git diff dee838f b7a7271` on `main`)

Reviewer: Claude Opus, resuming the Review N–T session, 2026-10-01. I changed no repository files. The scratch copy was `/tmp/claude-1000/-home-komatsu-works-Ez-SDRv4/d82454b2-b1d7-4f3c-a624-72d8e684572b/scratchpad/review-u/copy`, made with `rsync -a --no-times` and touched before each build session. I compared every file under `crates/` against HEAD `f3e08d6`, at the start and after each probe: it matched. Every build ran in `ezsdr-v4-dev:uhd4.10` with `CARGO_TARGET_DIR=/cargo-target/review`. No USRP was used.

Brief: [`../prompts/review-u.txt`](../prompts/review-u.txt). The report as returned.

## VERDICT

**The fix is correct and minimal. No blockers.**

**The cause is right**, as design-notes §21 states it. The escalation flag was raised before the body was queued:
- on the hot path, before the ring check;
- in `emit_control`, before the push.

`escalation()` returns the lowest-index flagged kind. So a drain caught between flag and body could set the end ahead of the first stopping event in `delivered`.

**The fix removes the cause, and opens no other path to the same mismatch.**
- **Only the full-ring branch raises a flag.** `escalation()` therefore names only kinds that `delivered` will never hold.
- **One drain path.** Every drain goes through `drain_and_react` (`coordinator/stepping.rs:228`), under the `delivered` lock: `round`, the device-paced control round, the livelock paths, the paced data pass (`paced.rs:136`), and cleanup's `FlushEvents` (`ending.rs:606`). `EventCollector::drain` has no other caller in the Kernel, so no drain appends without reacting.
- **Nothing else reads the flags.** `escalation()` has no other reader. RS-10's mode escalation reads `shared.end` (`ending.rs:55–66`), not the collector, so it sees nothing less.

**RS-36's purpose is intact at the collector level.** Mutation E-1 removes the drop branch's flag: it is killed by `rs_36_abort_survives_a_drop`, `rs_29_an_unregistered_fatal_kind_escalates` and `rs_36_an_unforeseen_source_does_not_silence_the_abort`.

**No stopping event can now fail to end the Run.** A control-path body (STEP_LIVELOCK, a Provider thread's DEVICE_LOST, KC-30's own) is drained, by every path, through `drain_and_react`, which reacts to it.

**Test results.**

| Run | Result |
|---|---|
| Kernel suite, stable | All test binaries pass |
| Kernel suite, 1.85.0 | All test binaries pass |
| `device_paced`, full, fixed code | 40 runs, 0 failures |
| `kg_02` alone, fixed code | 300 runs, 0 failures |
| D01, D02 | Killed by `rs_36_a_delivered_body_raises_no_escalation_flag` |

**Two P2 test gaps.**
- No test shows, at the Run level, that a dropped stopping body ends the Run (TG-U1).
- KG-2's own test did not reproduce the old race here (TG-U2). The new collector test is the deterministic guard.

## BLOCKERS

None.

## NONBLOCKING

- **NB-U1 (P2) — two comments still describe the old flag.**
  - `event.rs:360–363`, the doc of `escalation()`: "True when a kind whose reaction is `stop` or `abort` was emitted, even if its body was dropped".
  - `module_api.rs:1326–1328`: "Emitting it is what puts it through RS-33's counters and RS-36's escalation flag". STEP_LIVELOCK goes through `emit_control`, which no longer raises one.

  Fix: "True when the hot path dropped the body of a kind whose reaction is `stop` or `abort`", and "through RS-33's counters, and is reacted to when drained (KC-31)".
- **NB-U2 (P2) — the restated texts are right and sufficient; three consequences worth knowing.**
  - **A dropped body loses the cause.** KC-31's new sentence ("its kind the cause unless a delivered stopping event already requested an end") makes a delivered stopping event the cause even when it was emitted later than a dropped one: `drain_and_react` reacts to the drained bodies before reading the flag. That is consistent with KC-31's rule that the cause is the first in `delivered`, which a dropped body never is. Stated; I would leave it.
  - **One drain of delay.** A drop is flagged after the ring lock is released (`drop(ring)`, then the drop count, then `escalate`). A drain falling between those steps delivers the drop's `EVENTS_DROPPED` but sees no flag yet, so the Run ends one drain later. Harmless.
  - **Flags are never cleared**, so each later drain repeats the request. Requests after the first are no-ops, or RS-10's orderly→abort escalation, which is the intended effect of an abort-class event during an orderly stop.
- **NB-U3 (P2) — the three restated tests are not weakened.**
  - **What changed.** Each now asserts the delivered body's reaction through `Policy::reaction_for_event`, the very call `drain_and_react` makes, and, where the test is about the flag, a dropped body's flag (`rs_36_an_unforeseen_source…` through the fallback row; `rs_29…` with a ring of 8 filled first).
  - **`ma_30_stepping_livelock_cap`** now stops at the collector and the Policy table. It no longer shows the Run aborting on STEP_LIVELOCK; that is a coordinator matter outside `ma_30`'s scope, and I did not look for a separate test of it.

## TEST_GAPS

- **TG-U1 (P2) — no test shows that a dropped stopping body ends a Run.**
  - Mutation E-2, `collector.escalation().filter(|_| false)` in `drain_and_react` (`stepping.rs:261`), survives the whole Kernel suite.
  - All of RS-36's tests (`rs_36_abort_survives_a_drop` and the two restated) stop at the collector. Before the fix this block also served delivered bodies; now it is the only way a dropped `DEVICE_LOST` ends a Run, so RS-36's purpose has no end-to-end guard.
  - design/04's table row for `rs_36_abort_survives_a_drop` ("the Run aborts") overstates what it checks.
  - Fix: a coordinator test (Simulation class is enough) with an event ring of capacity 1 or 2, a Module that fills it, then a stopping kind whose body is dropped. Assert `Stopped { policy { that kind } }` and an `EVENTS_DROPPED` for it in `delivered`.
- **TG-U2 (P2) — KG-2's own test did not catch the old race in this environment.**
  - With the old behaviour restored (D01 and D02 together), `device_paced` passed 40 of 40 full runs and `kg_02` 300 of 300 alone; §21 saw 2 failures in 36, under a concurrent load. So `kg_02` is a probabilistic check that depends on the machine's timing.
  - The deterministic guard is `rs_36_a_delivered_body_raises_no_escalation_flag`, which asserts the invariant whose absence allowed the race (no flag for a queued body, on either path); D01 and D02 kill it. That is cheap and sufficient, so no collector hook is needed.
  - Fix: say so in KC-31's "Checked by", naming `rs_36_a_delivered_body_raises_no_escalation_flag` as the guard and `kg_02` as the end-to-end check.
