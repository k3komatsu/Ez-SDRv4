## Verdict: REVISE

Four findings are closed; #1 and #2 remain partially open. **E2a is not ready for the full gate.** Assessment is by inspection.

### Original findings

| # | Status | Evidence |
|---|---|---|
| 1 — S6/S7 scheduling sensitivity | **Partial** | Manual-clock setup is correct, but wall-time nudging still permits scheduling-dependent overruns: `crates/ezsdr-radio-uhd/src/brain/device_tests.rs:39–54,75–102`. See N1. |
| 2 — Packet/timeout mismatch | **Partial** | Incomplete packets are withheld and S9i pins the original example: `crates/ezsdr-radio-uhd/src/brain/model.rs:246–265`; `crates/ezsdr-radio-uhd/src/brain/conformance.rs:388–389`. A delayed completion can still defeat the deadline; see N2. |
| 3 — Fabricated channel coverage | **Closed** | Equal channel lengths/counts are checked; probe coverage comes from returned shape, and g2’s expected evidence requires channel 1: `crates/ezsdr-radio-uhd/src/brain/conformance.rs:104–118,336,450–465`. |
| 4 — Unsupported production reachability | **Closed** | Classification is now explicitly raw-device-only; the booking-floor example is pinned: `crates/ezsdr-radio-uhd/src/brain/conformance.rs:438–482`. |
| 5 — Read-time versus send-time assertion | **Closed** | Three ticks separate read and send; the inbox must contain the earlier timestamp: `crates/ezsdr-radio-uhd/src/brain/thread_tests.rs:745–757`. |
| 6 — Duplicated fixture | **Closed** | Shared setup: `crates/ezsdr-radio-uhd/src/brain/mod.rs:85–99`; callers: `crates/ezsdr-radio-uhd/src/brain/world.rs:2039` and `crates/ezsdr-radio-uhd/src/brain/device_tests.rs:236`. |

### Remaining findings

**N1 — P2: The 20 ms nudge preserves the original load-sensitive failure mechanism.**
- Delay R before its next `rx_recv`, with progress just below S7’s `e`. Every 20 ms, `settle` advances another tick despite R not running (`device_tests.rs:79–94`).
- A roughly 2.2-second delay—inside the five-second watchdog—can advance over 100 ticks. FakeDevice then reports overflow and skips the sample at `e` (`crates/ezsdr-radio-uhd/src/device.rs:1716–1724`). Later consumption cannot satisfy S7’s exact evidence assertion (`device_tests.rs:218`).
- Thus wall time changes simulated history and can change the verdict. Replace the fallback with controlled clock/executor coordination.

**N2 — P2: Packet readiness is checked at completion, not against the receive deadline.**
- Minimal sequence: start at 0; begin `recv(4, timeout=3, one_packet=true)`; advance to 3; defer `REnd`; advance to 4; complete the receive.
- Both model paths return samples 0–3: `crates/ezsdr-radio-uhd/src/brain/model_device.rs:275–300`; `crates/ezsdr-radio-uhd/src/brain/world.rs:1016–1041`. FakeDevice committed to `Timeout` because packet readiness was beyond its patience (`crates/ezsdr-radio-uhd/src/device.rs:1763–1767`).
- Preserve the deadline’s result across delayed completion; add this sequence to conformance. Simply rejecting every completion after the deadline would wrongly reject packets ready in time.

### Other requested checks

- **HostClock:** The proposed lost-wake explanation is not supported. `ended` and `advancing` use the same `now` under the same mutex (`crates/ezsdr-radio-uhd/src/host.rs:201–218`). If the manual deadline has arrived, `ended` returns first.
- A concrete alternative explains the observation: R reads integer device tick 21 at host time `21 ms + ε`, then sleeps three milliseconds, giving deadline `24 ms + ε`. The participant stops at exactly 24 ms; R’s deadline has **not** arrived. See `device.rs:981–992,1135–1138` and `host.rs:316–318`. This is relative-wait/tick quantization, not a demonstrated HostClock defect.
- **Packet fidelity:** Whole-packet and stopped-tail behavior agree for the exercised `n = packet`, one-packet slice (`model.rs:246–265`; `device.rs:1739–1782`), except N2. Arbitrary lengths/multi-packet mode remain unmodelled: `ModelDevice` ignores `one_packet`.
- **E1 coverage:** `packet: 1` applies to all three `happy_world` callers, not only the exhaustive happy-path test (`crates/ezsdr-radio-uhd/src/brain/oracle.rs:946–958,966,994,1009`). The shutdown trace was shortened; micro case 4 lacks bounded-search W4b evidence but reportedly retains walk evidence. No longer epilogue was adopted. These are changed coverage, not unchanged E1 results.
- **Settings wrapper:** Outcome timing now agrees for Move, Restore and Timed: each uses its own call result (`crates/ezsdr-radio-uhd/src/brain/threads.rs:529–543`; `model_device.rs:163–215`). Full protocol equality still excludes the acknowledged Q1 acceptance-timestamp difference (`threads.rs:500–503`; `world.rs:929–931`).
- **Booking floor:** Sound for excluding g2 across channels: `booked` is global, and booking/release both include the window boundary (`crates/ezsdr-radio-uhd/src/provider/control.rs:652–673,690–711`; `crates/ezsdr-radio/src/timeline.rs:423–427`). At exactly 3 ms the floor applies; outside it, ordered release prevents inversion. The new test does not explicitly pin that boundary.

**Blockers:** N1 and N2. No new P1.

Not checked: test/mutation execution, full gate, hardware, or the prohibited holdout directory.
