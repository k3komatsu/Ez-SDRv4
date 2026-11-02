## Verdict: REVISE

All paths below are relative to `crates/ezsdr-radio-uhd/src/`, unless stated otherwise.

### Findings

1. **P1 — Failure cleanup can silently lose a settings command** (`brain/threads.rs:392–394`).
   - Minimal sequence: S sees `F`, snapshots its FIFO, and pauses during abandonment; before processing the loss, B routes another settings command; S resumes and clears the live FIFO.
   - The newly routed command was not in the snapshot. It disappears without `Abandoned`, a terminal record, or `Done`. Forced replay cannot expose this interleaving because it treats `SCheck` as one event.
   - Atomically remove the commands being abandoned, then process that removed batch. Leave later arrivals for the discarder. Add a controlled concurrent regression.

2. **P2 — The panic guard does not cover the brain’s actual operations** (`brain/threads.rs:332–348`; `brain/force.rs:317–324`).
   - Only `Brain::step` is inside `catch_unwind`. A panic during `brain_op`, including a cell section, escapes to `spawn`, which merely records it and marks the thread exited.
   - Sequence: RX running → unwind during a brain cell operation → brain exits while `F` remains false and `mark_lost` is never called. Scratch restoration does not provide the required loss guard.
   - Cover the complete brain execution loop and test an unwind inside an operation, not only before `step`.

3. **P2 — S5 substitutes exit flags and forced shutdown for actual executor exit** (`brain/threads.rs:371–377`; `brain/thread_tests.rs:546–555`).
   - S sets its sink exit flag, then remains alive waiting in `SDiscard`. The test accepts that flag and calls `Runtime::stop`, whose `stopping` flag finally releases this wait.
   - Thus the test passes even though S did not exit following the brain’s death. Arrange sender closure/discarder termination, and observe actual thread completion before teardown assistance.

4. **P2 — S10 does not establish its command-issuance claim** (`brain/thread_tests.rs:414–420`).
   - `Stop` appearing before `Start` in device calls does not prove that B waited for the quiesce acknowledgement before **issuing** Start. R’s FIFO can preserve that call order even when Start was queued too early.
   - The publication/quiesce sink ordering checks a different obligation. E2-14 removes the acknowledgement rather than testing premature issuance.
   - Stop the forced sequence before acknowledgement processing and assert that no new-epoch Start/Open/Close command has been routed.

5. **P2 — “Every declared witness” is not reliably enforced for case 6** (`brain/thread_tests.rs:103–105,135–138`; `brain/cases.rs:225–229`).
   - `declared` is overwritten on every seed, although case 6’s declarations depend on its panic step.
   - If W6a is never reached and the final seed selects a different step, the full test need not report W6a missing.
   - Preserve the union of required declarations, or use separate builders for the required panic positions. Add a deliberately missing-W6a regression.

6. **P3 — Sample-sized work runs under the cell lock** (`brain/threads.rs:614–627`; `brain/rules.rs:412–434`).
   - `publish_round` scans samples and constructs scratch chains, vectors and boundary lists while `with_cell` holds the mutex.
   - This does not implement note 45 §4.5’s compute-outside/validate-and-write-inside split. It makes cell hold time depend on packet work and blocks settings acceptance and Stop booking behind that work.
   - Reuse the existing snapshot/recheck mechanism to move computation outside the critical section; do not count this implementation as demonstrating the proposed short-cell architecture.

### Q1

**The classification is useful and operationally sound, but its labels need qualification** (`brain/thread_tests.rs:586–610`):

- “Value” means changing only the post-return acceptance timestamp to `effective` restores equality.
- “Visibility” means that change is insufficient, but recording acceptance inside the device effect restores equality. This bucket can involve **both** timestamp and visibility differences.
- These interventions explain the replay differences; they do not independently prove production equivalence.

**The delayed-timestamp effect is real in production.** `provider/control.rs:882` passes a post-apply clock read to `RxChain::hand`; `provider/core.rs:245–248,316–322` uses that timestamp to qualify consumption.

Minimal sequence:
1. A running chain accepts v, instant 8, effectively at 8.
2. The settings thread is delayed before returning/recording acceptance until 12.
3. RX subsequently receives buffered samples 8–11.
4. DST consumes v at 8; production rejects those samples as consumption evidence.

The direction is **conservative under the modeled clock/device semantics**: production can retain debt despite actual application and reception. It can cause extra drain/flush work and, without later qualifying evidence, an `rx_drain` rejection or skipped restore (`provider/rx.rs:321–324,425–430,453–455`).

The six visibility cases are **not automatically production defects**: production holds `rx_chain` across `hand`, excluding concurrent chain consumption (`provider/control.rs:845–900`). Also, reversed-instant model scripts alone do not establish unsafe production consumption: production applies a booking floor and releases in instant order (`provider/control.rs:648–652,690–711`).

### Fidelity and checks

- No E1 behavior regression or incorrect retarget was found in the moved decisions and 14 E1 mutation rows by inspection.
- F1’s acquisition tests are executable; F2/F3 concern the explicitly deferred E2a-2 coverage.
- Focused tests: **284 passed, 48 ignored**. Both S5 tests also passed in **20 repeated rounds**; finding 3 explains their limitation.

Not checked: wide replay counts independently, mutation execution, full gate, E2a-2, hardware, or the prohibited holdout directory.
