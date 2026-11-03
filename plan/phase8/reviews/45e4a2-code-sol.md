# REVISE

No P1 findings. Three P2 findings; paths below are relative to `crates/ezsdr-radio-uhd/src/`.

## Findings

1. **P2 — The DST joins a brain that the thread runtime has not ended.**  
   `brain/world.rs:545–550`, `brain/oracle.rs:509–514`, `brain/threads.rs:506–544,288–298`  
   Sequence:
   ```text
   Shutdown → brain becomes Terminated → executors exit → join brain → cleanup
   ```
   The world considers `Terminated` plus empty queues an ended thread. But `run_brain` exits only on `dead` or the separate stopping path; normal termination leaves it waiting for another input. The DST therefore permits cleanup while the corresponding real brain remains alive—the exact lifetime the new ruling must exclude.
   Conversely, `join_within(Actor::Brain, …)` checks `sink.exited`, which `run_brain` never records, even after panic. It reports a timeout for an ended brain.
   **Fix:** model actual brain-thread exit and drive the same shutdown/join sequence on threads. I20’s copied `ended` calculation currently repeats the model’s mistake.

2. **P2 — Native retention still depends on cleanup suppressing destructor entry.**  
   `brain/world.rs:638–640,861–887`, `brain/stop_tests.rs:87–109,193–215`  
   Sequence:
   ```text
   brain panic → F=true → omit native mark
   → cleanup Retain → release the remaining native owner
   ```
   In the DST, `Retain` prevents `CleanupClose`, and only `CleanupClose` creates a pending destructor. Releasing the remaining native owner is absent, so this path never tests the missing mark’s consequence.
   Today `UhdDevice::drop` independently decides retention and otherwise closes streamers (`uhd.rs:667–692`); `RxStream::drop` reads only the native mark (`:345–356`).
   The new test manually changes F/mark after cleanup’s check, after all publishers were joined. That is useful destructor-unit coverage, **not a reachable lifecycle sequence**. E4-60 likewise kills this injected path, not DST coverage of owner release.
   **Fix:** represent final native-owner release separately from cleanup eligibility, including the native device destructor’s decision. Add a reachable missing-mark regression rather than relying on F-gated cleanup.

3. **P2 — Spawn failure does not initiate the required failed-start shutdown.**  
   `brain/world.rs:533–535,1087–1102`, `brain/cases.rs:549–552`  
   Sequence:
   ```text
   spawn R0 succeeds → spawn R1 fails
   → Release(A→R0) → apply A → much later Shutdown
   ```
   Failure sets `startup.failed`, which immediately enables environment Actions, but queues only `SpawnFailed` notifications. Case 15 deliberately delivers releases before its later horizon-based shutdown. Thus a failed start can continue accepting work and still pass I21/P2/P3.
   Today failure immediately executes `stop(Abort)` and returns rejection; the control thread is not subsequently spawned (`provider/mod.rs:659–678`).
   **Fix:** make the startup failure itself trigger shutdown and rejection, suppress subsequent Actions, and assert the already-spawned executor’s stop sequence. Omitting `Abort` is not merely a harmless abstraction while this ordering differs.

## Remaining assessments

- Joining the brain before cleanup is **sound in principle**: joined publishers cannot introduce a new loss; detached publishers require retention. Finding 1 prevents the implementation from demonstrating that argument.
- Case 13’s smaller loss window and tick-based join limits are acceptable abstractions for ordering, not evidence of production timeout fidelity.
- Excluding cases 13–15 from replay is not adequately compensated: current thread cleanup tests call cleanup before joining.
- `RxExec.unspawned` versus `Brain.unspawned` is legitimate runtime fact versus delivered knowledge, not inherently scattered ownership. Preserve that distinction; do not demand instantaneous synchronization.
- Oracle self-test rows are separately labelled. The destructor-unit kill must not be reported as reachable DST lifecycle coverage.

Not checked: builds, test execution, mutation-run results, hardware, or either forbidden holdout directory. No files or git state changed.
