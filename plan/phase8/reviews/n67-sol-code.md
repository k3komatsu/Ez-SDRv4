# #67 — Sol's code review (2026-11-13)

Before reading the plan or reports, my failure scenarios are:

- A future cut that keeps samples must not cancel the start; a cancelled start must not feed its successor.
- The open-time stop must run once, on RX only, before any start. Failure must leave device ownership truthful; loss must prevent subsequent calls.
- Close/reopen must preserve queued starts by channel, without implying unmeasured behavior is verified.
- The differential must reject premature stops, and call-log filtering must not hide experiment stops.
- Rehearsals must inspect received data and exact verdicts; mutation kills must not rely on an undeclared timeout.

I’m checking those against the implementation first, then the recorded decisions.
I found a blocking gap in the new loss path: the owner publishes a lost open-time stop, but `arm()` converts it to a rejection string and leaves `core.is_lost()` false. Cleanup can therefore close the lost device’s streamers. The new SO3 test covers only the owner path.

I also found that the fake’s orphan helper misses starts queued behind an unread stopped-stream tail.
## Verdict: FAIL

What this change does: cancels cut RX segments without waiting for their origin and adds an untimed stop on every receive open. The central fix is sound; one loss path remains unsafe.

Paths below are under `crates/ezsdr-radio-uhd/` unless stated otherwise.

### Major — must fix

1. **A lost stop during `arm()` does not publish device loss.** **VERIFIED by code tracing.**  
   `src/provider/core.rs:1524` preserves `lost: true`, but `src/provider/mod.rs:560` passes it through `open_refusal` (`:54–55`), which discards the flag.

   Minimal sequence:
   ```text
   arm → rx_open succeeds → initial rx_stop returns lost:true
   → arm returns Rejected → cleanup calls close_streams
   ```
   Observable wrong outcome: no `DEVICE_LOST` or `mark_lost`; cleanup frees streamers on the lost device (`mod.rs:741–744`). Hardware teardown failure is **INFERRED** here; the forbidden close is **VERIFIED**.

   **Fix:** publish loss before converting the error to an arm rejection. Add an arm-path regression asserting one loss event and no subsequent close. SO3 (`src/provider/rx.rs:1071–1086`) tests only the owner path.

### Minor — should fix

2. **The orphan model drops a queued start behind an unread tail.**  
   **VERIFIED model omission; hardware generalization INFERRED.**  
   `src/device.rs:542–548` examines only `old.next`. A start issued while a stopped stream has unread samples lives in `old.then` (`:1214–1215`).

   Sequence: run A → leave unread samples → stop A → queue future B → close/reopen before B. The fake silently cancels B, contrary to its new queued-start model.

   **Fix:** preserve the future `then` start too, with the appropriate rate; add this tail-plus-reopen case.

3. **`calls()` can hide an experiment stop instead of setup.** **VERIFIED.**  
   `tests/common/mod.rs:685–690` retains each open marker until *any* later matching stop, even across a start or failed open.

   For example, `open(fails), open, setup-stop, start, experiment-stop` suppresses both stops. A misplaced stop after `rx_start` is likewise treated as setup.

   **Fix:** filter only the immediate matching open/stop pair. Test failed-open and intervening-start logs.

4. **UR-17 retains the withdrawn “never close a queued start” promise.** **VERIFIED contradiction.**  
   `design/18-uhd-radio.md:230` forbids closing before cancellation, but the adopted spawn-failure path deliberately closes the queued start and relies on the next open (`src/provider/mod.rs:1088–1099`).

   **Fix:** qualify that sentence to ordinary owned-stream termination; retain the explicit uncertain-cleanup/open-time-cancellation rule.

### Maintainability and evidence

The shared open path and reused hardware trial are appropriately small. Findings 2 and 3 concern incomplete lifecycle handling, not style. No additional duplication finding.

I checked both raw bench logs: stop cancels and close/reopen preserves the start in all 30 trials per board. The full mutation log records all 26 `N67F` rows killed without bound outcomes.

No edits, builds, probes, mutations or remote operations performed. Fresh-streamer stop safety and `CloseThenStop` hardware behavior remain **INFERRED** pending the bench.
