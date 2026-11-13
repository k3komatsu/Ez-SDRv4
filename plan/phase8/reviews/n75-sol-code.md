# #75 — Sol's code review (2026-11-13, openai/gpt-6.1-sol, read-only)

## Verdict: FAIL

The change correctly separates confirmed recovery from the legacy open flush, but two interruption/reporting gaps need fixing.

### Major — must fix

1. **Stop during a successful time read still permits a send — VERIFIED**
   
   `crates/ezsdr-radio-uhd/src/provider/core.rs:1272–1302`

   **Sequence:** recovery opens → pre-send `time_now()` blocks → `Provider::stop` publishes → read returns `Ok` → T3 sends four samples anyway. Stop is checked only on the read’s error branch, not after success. Stop during the retry sleep also permits another potentially blocking read.

   Check interruption before each read and again before sending. Add controlled tests for Stop during a successful read and during back-off; current tests interrupt failed reads or open boundaries.

2. **Retry stash recording silently drops required events — VERIFIED**
   
   `crates/ezsdr-radio-uhd/src/provider/core.rs:1467–1471`  
   Normal handling: `crates/ezsdr-radio-uhd/src/provider/tx.rs:1121–1140`

   **Sequence:** attempt 1 receives `SeqError` → stashes it → gets no ACK → retry records the stash directly as JSON → attempt 2 opens and confirms. The raw report survives, but its required `radio.TX_UNDERFLOW { cause: lost }` never appears. A non-flush underflow can similarly lose its event.

   Preserve normal event handling without correlating old-generation reports against the replacement streamer. Existing stash tests assert raw rows, not events; N75-11 therefore does not catch this.

### Minor — should fix

3. **The transient-read test still depends on scheduling — VERIFIED**
   
   `crates/ezsdr-radio-uhd/src/provider/core/t3.rs:39–43,354–364`

   The fault window is absolute device time, 100–103 ms. Device time can advance between construction and installing `Frozen`. A delay exceeding 3 ms means attempt 2 misses that window entirely, producing zero failed reads instead of the asserted three.

   Establish a known device-time origin after freezing, or place the fault relative to that origin. The 40 ms elapsed slack does not fix this exact-count assertion. This is a **lib** test, so it does not identify the unexplained **fake** failure.

4. **KL-44’s wait estimate omits streamer closes — VERIFIED omission; duration INFERRED**
   
   `KNOWN_LIMITATIONS.md:185`  
   `crates/ezsdr-radio-uhd/src/provider/core.rs:1401,1449,1463`

   Recovery also performs the initial close, inter-attempt closes and final exhaustion close. Native close takes the control mutex and destroys the streamer (`crates/ezsdr-radio-uhd/src/uhd.rs:1160–1164`). Include close/serialization time rather than presenting `3 × (open + 100 ms + read)` as the complete estimate.

### Coverage and maintainability

The tests cover partial/stale/tickless ACKs, exhaustion cleanup, late policies, pending switches and switch-off compatibility well. Keeping retries inside one reopen avoids additional lifecycle state. The new raw-recording path duplicates only part of normal report handling; finding 2 demonstrates the resulting inconsistency.

The plan’s acknowledged foreign-ACK residual remains: the previous successful front does not bound the burst whose EOB failed. Its delayed ACK can pass the floor. Hardware occurrence remains **INFERRED**, not ruled out by these tests.

**Not checked:** no test, mutation or hardware reruns; static review only. No files changed or probe copy created.
