**REVISE — two P2 findings; no P1.** Paths below are relative to `crates/ezsdr-radio-uhd/src/brain/`.

1. **P2 — Zero senders can mean “not registered yet.”** `threads.rs:184–202`
   - Sequence: start a terminated, failed world → brain runs before the builder registers S/R/clock/timer → empty inbox/count 0 permits return → builder registers R → R sends its final Lost.
   - Wrong outcome: Lost remains unserved; the cell never becomes Lost. This affects targeted and polling runs, including the new hand-built regressions. Forced replay avoids this particular return because automatic exit is disabled.
   - Register **all** senders before starting the brain, or retain a construction registration until registration finishes.

2. **P2 — The world counts S’s failed loop as completed, despite pending discards.** `world.rs:556–557,573–575,646,677–681,1226–1230`
   - Sequence: S holds command A; brain decides independent command B → failure → SCheck abandons A and sets `exited` → brain routes B → R’s final Lost inputs are served → Shutdown terminates → choose BExit before SDiscard.
   - Wrong outcome: SDiscard subsequently queues B’s Abandoned Done into an already-returned brain. `pending[B]` remains uncleared; forced replay reproduces the stranded input. S’s join can also succeed before its discarder finishes.
   - Model S completion separately from its failed processing loop: termination received, queue empty, and no remaining work/send.

**Previous findings:** the targeted wake, joint empty/count read, clock/timer registrations, and requested controlled regressions address the original mechanisms/P3. Exit correctness is nevertheless not closed because of the two windows above.

**Declared deviation is sound.** If the waiter sees the old count, it holds the scheduler lock until its condition wait releases it; the subsequent wake cannot pass that boundary unnoticed. If it sees the decrement, its predicate succeeds. Polling is also safe; forced waits use the same lock discipline.

**S’s threaded discarder is safe** at terminate plus empty queue: queued commands are processed/abandoned before return, and registration survives through their sends. The world does not yet represent that completion correctly.

**Listed world/thread differences:** atomic timer delivery matches forced `Event::Timer`; clock serves after Terminated remain replayable because forcing does not halt the clock. Neither is itself a replay hazard, though forced replay does not establish free-running clock shutdown.

Not checked: builds, test/mutation reruns, hardware, or either forbidden holdout directory. No files or git state changed.
