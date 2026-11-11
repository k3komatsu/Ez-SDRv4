Before reading the brief or plan, my failure scenarios are:

- Panic while idle or booking an Action: publish one fatal root-source `DEVICE_LOST`, retain the panic row, and release any submit wait without a KC-21a timeout.
- Panic with RX and repeat TX active: end every stream and stop further repeat feeding, even if Policy maps `DEVICE_LOST` to `continue`.
- Concurrent owner loss and control panic: record the panic but mark and emit loss only once.
- Panic after `Provider::stop` has begun, including plan recording: preserve bounded shutdown and distinguish a genuine panic from an ordinary stop.
- Panic while holding shared state, or during loss handling: recovery must not silently die before publishing the fatal event.

The ordinary control-panic omission is verified in `provider/mod.rs:373–381`; the recovery-path risks still need checking.
## Verdict: FAIL — revise the tests before implementation

The **basic direction is sound**: remove the control-thread exception and reuse `device_lost`. It does **not** guarantee an unconditional Run termination or instantaneous RF silence.

References below use `plan` for `/home/komatsu/.cache/ezsdr-fixes/2026-11-12-n92/test-plan.md`. **VERIFIED** denotes source evidence; unexecuted outcomes are **INFERRED**.

### Major findings

1. **T4 can deadlock its own manual clock.**  
   Sequence: uhd-tx reaches `Barrier::wait` before control’s next 500-ms check → TX remains a busy participant → device time stops → control never reaches the barrier. T6c has the same problem if control blocks in `recv` before the timed `FakeFault::Lost`.  
   **Evidence:** `plan:77–78,96`; `design/18-uhd-radio.md:365` (M2–M6). Mechanism **VERIFIED**; resulting hang **INFERRED**.  
   **Change:** stage both failures at already-reached, clock-independent gates, or use unit-level injected bodies. Do not wait for device time while a participant blocks on a standard synchronization primitive.

2. **T3’s advertised watchdog cannot detect the deadlock it targets.**  
   Sequence: handler deadlocks and remains busy → `Direct::until` enters `host.sleep(1 ms)` → clock cannot advance → its next wall-deadline check never executes.  
   **Evidence:** `plan:72–73`; `crates/ezsdr-radio-uhd/tests/fake.rs:1485–1490`. **VERIFIED** control flow; hang **INFERRED**.  
   **Change:** explicitly wrap the whole scenario, including teardown, in the external `bounded` watchdog (`fake.rs:1790–1802`). Apply this to every deliberately blocking seam.

3. **The plan omits the Policy-independent repeat-safety case.**  
   Sequence: repeat playing → Policy maps `DEVICE_LOST` to `continue` → control panics → no coordinator-requested shutdown. T1 only checks the default abort path, so cleanup can mask missing Provider-side quiescence.  
   **Evidence:** `plan:23,55–59`; `design/06-kernel-coordinator.md:383`; `provider/tx.rs:295–305,464–466`. **VERIFIED** coverage gap; escaped defect **INFERRED**.  
   **Change:** add a `continue` case proving that loss alone stops further repeat feeding and ends the stream clocks, **before** calling `finish`. The Run remaining Running is permitted.

4. **No test exercises KC-21a’s actual pending-submit failure sequence.**  
   Sequence: coordinator dispatches an Action → control panics while booking it → submit waits for completion. T2 substitutes `Pushed`; T1 panics during device polling and has no pending submit. Both can pass while submit still ends through the five-second failure.  
   **Evidence:** `plan:63–68`; `crates/ezsdr-kernel/src/coordinator/state.rs:62–80`; `design/06-kernel-coordinator.md:275`. **VERIFIED** gap.  
   **Change:** add a full-coordinator, gated booking-panic test: submit returns ended with the `DEVICE_LOST` policy cause, not KC-21a; assert undelivered IDs where applicable.

5. **The RF-safety oracle overclaims what the loss path guarantees.**  
   Sequence: TX prepares a send → control publishes loss → the already-in-flight send completes. `mark_lost` neither stops hardware nor serializes ordinary sends; a panic also leaves the physical device healthy. Therefore “no calls after `mark_lost`” is not automatically a valid concurrent oracle, and logical clock/burst records do not establish RF silence.  
   **Evidence:** `plan:41–48,142`; `provider/tx.rs:738`; `src/uhd.rs:1143–1145`; `src/device.rs:2016–2018`. **VERIFIED** implementation; interleaving **INFERRED**.  
   **Change:** distinguish already-admitted/in-flight work from newly initiated work, observe TX quiescence, and explicitly account for buffered samples. Do not amend UR-16 to promise immediate silence without hardware evidence.

### Recommendations on the four requested decisions

| Item | Recommendation |
|---|---|
| **A — panic inside `device_lost`** | **Reject the proposed second catch as a solution.** Another rejected row leaves the same missing-event/live-Run failure. T7’s always-panicking Authority demonstrates a synthetic second failure, not the alleged half-mutated-line failure. Establish a concrete reachable counterexample before expanding scope. `plan:17,98–101`; `core.rs:1058–1072`. |
| **B — panic during stop** | **Keep uniform fail-closed handling**, including during stop. Stop being booked does not mean TX/RX shutdown has finished (`mod.rs:702–715`). Do not assume cleanup is safe merely because control itself makes no more calls. Document the conservative retention cost; “leak bounded by process exit” understates repeated Sessions (`uhd.rs:685–688`). |
| **C — missing plan row** | **Adopt preservation of UR-30**, without making absence an accepted exception. Test exactly one complete plan row, including a panic during `Control::finish`; an unconditional handler append can duplicate a row already written. `control.rs:256–260`; `design/18-uhd-radio.md:336`. |
| **Policy `continue`** | **Preserve Policy semantics and add the safety test above.** “Fatal” does not override `continue`; wording must say the default Policy ends the Run. |

### Smaller corrections

- T2a’s Stop and loss are not necessarily at the same instant. Earlier fault arrival order does not make a later loss precede an earlier Stop. Control the clock or assert each actual instant (`plan:65`; `core.rs:631–654`).
- N92-3’s proposed T4 killer currently risks a harness deadlock, not an assertion-driven kill. N92-8 only tests extra logging, not restoration of #92’s safety property. Resolve these before adding rows (`plan:111,116`).
- S-time is reasonable for poison recovery, but label it **Authority-panic injection**, not evidence that UHD itself panics there.

No edits, builds, or probes performed. Hardware RF-stop timing and second-panic reachability remain unverified.
