**REVISE — one P2 remains; no P1.** Paths below are relative to `crates/ezsdr-radio-uhd/src/brain/`.

- **P2-1 closed:** `threads.rs:186–203` registers all senders before spawning the brain. Clock registration is unconditional; timer registration matches its spawn; unspawned R streams are filtered once. Guards move into closures without cloning; there is no late or double registration (`shared.rs:285–288`). OS spawn errors panic rather than recover (`force.rs:485–488`).

- **P2-2’s world exit window closed:** `world.rs:562–564,582,688` now requires S’s terminate, empty queue/program and no hand. `oracle.rs:508` agrees. `stop_tests.rs:638–653` checks that a pending discard prevents exit, and its queued Done still prevents exit afterward.

1. **P2 — Threaded S’s join still reports completion before its discarder finishes.** `threads.rs:643–645,613–617,318–322`
   - Sequence: SCheck sees failure and marks Settings exited → brain routes B → SDiscard remains parked → call `join_within(Settings, 0)`.
   - Observable wrong outcome: the join returns `true` although B has not been abandoned and its Done has not been sent. The world now correctly rejects that same join (`world.rs:562–564,582`).
   - Thus `s_ended` models the sender’s actual completion, but **not the threaded join’s completion criterion**. Use actual thread completion for joining, separately from the processing-loop exit observation; add the corresponding threaded regression.

No further normal-termination brain-exit defect found in this narrow inspection. Rows E4-115/E4-116 target the two repaired predicates; E4-81 follows the registration filter.

Not checked: builds, test/mutation reruns, hardware, or either forbidden holdout directory. No files or git state changed.
