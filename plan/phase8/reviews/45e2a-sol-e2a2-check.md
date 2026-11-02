## Verdict: REVISE

**N1 remains open. N2 is closed for the exercised one-packet slice. Not ready for the full gate.**

### P2 — N1 counts expired waits as parked
- `device_tests.rs:77–80` accepts an aggregate wait count, not proof that each executor cannot run.
- `HostClock::blocked()` counts every registered wait (`crates/ezsdr-radio-uhd/src/host.rs:479–483`), including one whose deadline has arrived but whose thread has not resumed and removed it (`host.rs:204–211,245–246`).
- **Counterexample:** R0 waits for the first packet; advance beyond its deadline, then leave R0 unscheduled. Its wait remains counted, so `parked()` stays true. `settle()` repeatedly advances time and eventually fails its 16-extra-tick assertion instead of allowing R0 to publish (`crates/ezsdr-radio-uhd/src/brain/device_tests.rs:91–103`).
- Thus scheduling delay still changes simulated history. These executors are ordinary, non-participant threads (`crates/ezsdr-radio-uhd/src/brain/threads.rs:143–145`; `force.rs:317–320`).
- Between inbox acquisition and the device call, or after wait removal, the running status normally prevents advancement. The expired-but-still-registered wait is the remaining hole.
- **Required:** coordinate wake acknowledgement per executor. Add a controlled-pause test asserting that time cannot advance past a runnable executor. “No assertion kills the mutant” demonstrates missing coverage; it does not justify omitting N1’s test/row.

### P2 — E2-55 is not a valid behavioural mutant
- `plan/maintenance/tools/mutations.json:18608–18609` replaces the lookup with `streamer.buffered.first()`.
- `buffered` is a `VecDeque<i64>` (`crates/ezsdr-radio-uhd/src/brain/model.rs:37`), which uses `front()`, not `first()`. This mutant fails compilation rather than the intended assertion.
- Use `.get(0)` or `.front()`, then rerun E2-55 remotely.

### N2 closure
- One shared deadline decision serves both paths: `model.rs:248–270`, `world.rs:1031`, `model_device.rs:299` (all under `crates/ezsdr-radio-uhd/src/brain/`).
- For `n = packet`, ratio one, zero latency, full-packet readiness and stopped-tail readiness match FakeDevice’s inclusive deadline rule (`crates/ezsdr-radio-uhd/src/device.rs:1739–1767,1780–1782`).
- S9j pins late completion and its converse (`brain/device_tests.rs:448–465`); E2-59 targets that decision (`plan/maintenance/tools/mutations.json:18645–18652`). Arbitrary lengths/multi-packet mode remain outside this closure.

Not checked: builds, test/mutation execution, rerun claims, full gate, hardware, or the prohibited holdout directory.
