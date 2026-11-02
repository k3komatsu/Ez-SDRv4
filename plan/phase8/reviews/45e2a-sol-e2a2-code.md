## Verdict: REVISE

Read-only review. Findings and mutation-kill assessments below are by inspection.

### P2 — Must fix

1. **S6/S7 remain load-sensitive** — `crates/ezsdr-radio-uhd/src/brain/device_tests.rs:25–31,89–94,124–141`.
   Deschedule the test after reading `origin` for over 20 ms: the timed starts become late. Deschedule S after choosing `e` for over 500 ms: S7’s admission/acceptance assertions fail on correct code. RX starvation beyond FakeDevice’s 100 ms overrun threshold is another failure path.
   Move these timing-sensitive sequences to controlled time. A real-clock smoke can remain if its operations/assertions tolerate scheduling delays; the 1 s watchdog is not a remedy for missed timing windows.

2. **S9 misses a real timeout/packet difference** — `crates/ezsdr-radio-uhd/src/brain/conformance.rs:174–183,349–352`.
   Minimal sequence: open, start at 0, immediately `recv(4, timeout=3 ticks, one_packet=true)`. FakeDevice returns `Timeout` at T=3 because its four-sample packet is not ready (`src/device.rs:1764–1767`). ModelDevice returns three samples at T=3 (`src/brain/model_device.rs:280–295`).
   All current scripts avoid this boundary. Add the case and resolve/report its E1 impact: micro cases use a one-tick timeout (`src/brain/cases.rs:36–38`), so “no E1 verdict is affected” is not established.

3. **The two-channel comparison fabricates channel coverage** — `crates/ezsdr-radio-uhd/src/brain/conformance.rs:105–108,162–169,310–314`.
   `recv` records only channel 0’s length; the probe independently synthesizes two channels. Returning only channel 0—or an empty channel 1—would leave g2 passing and still produce channel-1 consumption evidence.
   Assert returned channel count and each channel’s length, and derive probe coverage from the returned channel shape. Zero payload values may be waived; missing channels may not.

4. **g2 does not establish a reachable production Q1 defect** — `crates/ezsdr-radio-uhd/src/brain/conformance.rs:303–307,411–420`.
   The raw `Device` sequence is legal, but production cannot release instant 20 and then hand instant 8 as written. Booking floors the second instant behind the earlier in-window booking (`src/provider/control.rs:648–667`; `crates/ezsdr-radio/src/timeline.rs:423–427`), and release is ordered (`src/provider/control.rs:690–711`).
   Keep g2 as a raw-device/consumer mismatch, but classify production reachability as unproven. Supply a legal booking-to-release counterexample before recommending an effective-instant API change.

### P3 — Should fix

5. **The clock-read assertion cannot distinguish read time from send time** — `crates/ezsdr-radio-uhd/src/brain/thread_tests.rs:745–754`.
   No tick separates `ClockRead`, unlock and send. Reading the timestamp at send instead would pass. Advance time between read and send and assert the earlier timestamp.

6. **S6b duplicates an existing fixture** — `crates/ezsdr-radio-uhd/src/brain/device_tests.rs:160–170`.
   This repeats `src/brain/world.rs:2036–2047`. Both copies encode the same lifecycle/debt state and must change together; assertions against local literals do not detect drift between them. Extract only the shared setup, retaining each test’s sequence and assertions.

### Other checks
- S6b matches F2; S8 observes actual lock ownership and drain completion non-vacuously.
- S9’s manual participant and predicate-aware model ticking avoid functional scheduling windows. S8c likewise advances time only after observing its prerequisites; wall limits remain watchdogs.
- Dropped rows: E2-23→existing E2-20 and E2-24→E2-49 are sound. E2-25 is latency-equivalent under the current polling waits. FIFO Stop execution supports replacing E2-30 with E2-52.
- E2-49…54 have identifiable failure paths in their named tests. Execution is still required; S6/S7’s unrelated failures could falsely count as kills.

Not checked: test/mutation execution, full gate, hardware, or the prohibited holdout directory. No files, formatting, or git state changed.
