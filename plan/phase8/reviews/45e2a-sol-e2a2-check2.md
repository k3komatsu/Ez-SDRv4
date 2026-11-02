## Verdict: REVISE

N1 is only partly closed. Not ready for the full gate.

### P2 — The brain can still fall behind simulated time
- `crates/ezsdr-radio-uhd/src/brain/threads.rs:145–156` registers only `R_k`; the brain, S and K remain ordinary threads.
- **Counterexample:** R0 sends `Consumed`; suspend the brain before it processes that message. R0 keeps receiving while `advance_until` advances 400 ticks, then fails despite correct code (`crates/ezsdr-radio-uhd/src/brain/device_tests.rs:118–125,133–135`).
- Scheduling delay therefore still changes simulated history. Register the brain, S and K too, with clock-aware idle waits. The stated idle-wait objection no longer applies after introducing `Sched::idle`.
- Timer delivery also needs a clock deadline rather than wall-clock polling if deterministic deadline histories are claimed (`crates/ezsdr-radio-uhd/src/brain/threads.rs:795–805`).

### P2 — S6c can pass E2-60 under load
- `crates/ezsdr-radio-uhd/src/brain/device_tests.rs:257–266` releases R0 after 100 ms, then checks whether release preceded the test thread’s observation.
- **Counterexample with E2-60:** the sleep returns while R0 remains held; deschedule the test thread for over 100 ms; the releaser runs; the assertion now passes.
- S6c exercises the intended pause, but does not reliably distinguish blocked clock advancement from delayed observation. Replace the fixed-delay release with a controlled clock-state/wake handshake.

### P3 — The watchdog kills unrelated tests and can misdiagnose load
- `crates/ezsdr-radio-uhd/src/brain/device_tests.rs:49–65` exits the entire test binary after 20 seconds, including otherwise-correct concurrent tests.
- A correct participant descheduled beyond that bound triggers the same failure; elapsed wall time does not prove mutual waiting.
- Prefer a bounded result channel that fails this test, as already used in `crates/ezsdr-radio-uhd/src/brain/conformance.rs:145–154`. Process-wide termination belongs in the external runner.

### Checks that passed by inspection
- No lost-notify sequence found in `Sched::idle`: generation is sampled while holding the scheduler mutex, which `notify` must cross before notifying the host (`crates/ezsdr-radio-uhd/src/brain/force.rs:133–155`). Registered R executors retain the clock’s busy/deadline protection.
- E2-55’s `.front()` is type-correct and should fail S9i: an incomplete packet becomes readable before its full-packet deadline (`plan/maintenance/tools/mutations.json:18605–18612`; `crates/ezsdr-radio-uhd/src/brain/model.rs:265–270`).

Not checked: builds, test/mutation execution, rerun claims, full gate, hardware, or prohibited holdout contents.
