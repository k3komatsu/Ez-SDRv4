## Verdict: REVISE

### P2 — Both pause tests can miss their mutants under load
- `crates/ezsdr-radio-uhd/src/brain/device_tests.rs:237–240,262–265,288–291,312–314`
- These waits end on **a notification or 50 ms**, whichever comes first (`crates/ezsdr-radio-uhd/src/host.rs:177–178,336–341`). They do not guarantee a 50 ms observation window.
- **E2-60 counterexample:** confirm R0 held; another runtime thread notifies during the test’s idle wait. The test resumes before the sleeper runs or measurable clock advancement, passes both assertions, and releases R0.
- **E2-61 counterexample:** confirm the unregistered brain held; a different registered participant remains runnable but descheduled throughout the observation window. That participant freezes the clock, so the assertion passes despite the mutant.
- Increasing the delay cannot make either test deterministic.
- **Fix:** replace elapsed-time inference with an atomic test-only clock-state probe/handshake. Establish that all other participants are idle, no deadline is reached, and the held actor itself is registered and busy—the reason advancement is blocked. Exclude the observing test participant from that decision.

### Closure and unchanged behaviour
- **Participants:** closed by inspection: brain, S, R, K and timer register on the manual fake clock; scheduler idle waits use that clock (`crates/ezsdr-radio-uhd/src/brain/threads.rs:91–94,156–167`; `force.rs:154–163`).
- **S6c:** the timed-release race is closed, but reliable mutant detection is not.
- **Watchdog:** process-wide termination is removed; the result-channel timeout fails its own test (`device_tests.rs:66–73`).
- Correct code should not fail the exact clock-equality assertions: the held participant freezes time regardless of scheduling delay. Wall-time watchdogs can still fail under sufficiently severe load.
- **ModelDevice/forced replay:** no N1 behaviour change found by inspection. They receive no fake host, retain condition-variable waits, and forced replay still omits the timer (`threads.rs:61–67,91,165`; `force.rs:191,364–366`).

**Full-gate blocker:** deterministic negative coverage for E2-60/E2-61 remains missing; a green mutation run would not resolve this.

Not checked: builds, execution, mutation results, full gate, hardware, or prohibited holdout contents.
