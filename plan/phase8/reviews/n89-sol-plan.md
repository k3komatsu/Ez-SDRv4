Independent failure scenarios, before reading the supplied plan/design:

- EOF before signal registration, during connect/T0, or during child construction must not miss the newly created Run. EOF during finish or after completion must not overwrite the first termination.
- A disconnect must wake an **already waiting** control loop, including the check-to-wait race and an existing pending wake. A signal fired before `advance_to()` starts cannot prove this.
- A complete frame queued just before EOF must not start another operation after disconnection wins; partial headers/bodies and read errors must also stop the Session.
- Closing the reply pipe during a blocked request must not prevent cleanup/Manifest writing; BrokenPipe must produce the agreed successful exit.
- Signal delivery must not hold the registry lock while calling Authority code or cleanup. Concurrent wake scheduling, FreezeDispatch, and child registration/removal must not invert lock order or deadlock.
- Retaining a signal after Run destruction must neither retain the device nor cause its final drop on the input thread. `DeviceAuthority::drop` explicitly protects against device destruction racing UHD teardown.
- Python interruption during header/body reads or writes must close both pipes and never send `finish` into a pending reply. Valid server-error replies need explicit coverage under the stated “any exception” policy.
- Process-group SIGINT must reach Python but not its new-session server. `close()` must wait before loading the Manifest; timeout/kill, absent or invalid Manifest, repeated close, and interruption during close must have defined outcomes without hiding the original exception.
- Tests must hold horizons/clocks and establish ordering with handshakes. Neither a short elapsed-time assertion nor unrelated data-thread/clock notifications should let a no-wake implementation pass.
## Verdict: **FAIL**

K1 fixes F1 correctly: it observes the parked control loop before signalling, with no event-driven wake to mask **C89-1-02**. The remaining blockers concern the hardware rehearsal and lifetime race.

References below to `test-plan.md` mean `/home/komatsu/.cache/ezsdr-fixes/2026-11-11-n89/test-plan.md`.

### Major findings

1. **X1 does not establish that transmission started before disconnect. — VERIFIED ordering gap; INFERRED failing interleaving.**

   Minimal sequence:
   ```
   repeat booked → submitted reply
   uhd-tx has not sent yet
   EOF → disconnect → Provider::stop → cancel unsent repeat
   ```
   X1 requires an EOB after the repeat’s first `tx_send`, but never waits for that first send (`test-plan.md:87`). The submitted reply establishes **booking**, not device transmission (`crates/ezsdr-server/src/lib.rs:409–423`); `uhd-tx` executes independently and can process shutdown before its next pass (`crates/ezsdr-radio-uhd/src/provider/tx.rs:288–305,1305–1309`).

   **Observable outcome:** a correct implementation can fail X1 because no payload was sent; alternatively, relaxing that assertion would leave the claimed “repeat playing” bench rehearsal untested.

   **Fix:** observe a positive-payload `tx_send` before writing the advance/closing input. Retain the EOB → `close_streams` assertions.

2. **N1’s device-drop/thread-ownership protection has no concurrency test. — VERIFIED coverage gap; INFERRED unsafe interleaving.**

   Minimal sequence:
   ```
   input thread upgrades signal’s Weak and pauses
   control thread finishes/drops RunHandle without synchronizing signal removal
   input thread releases the last Shared → device destruction there
   ```
   The plan explicitly leaves this entirely to review (`test-plan.md:118–119`). W1 tests sequential counters, not overlapping `fire` and removal. K4 tests persistent retention, not a transient upgraded holder.

   **Observable wrong outcome:** final device destruction occurs on the input thread while the server exits—the teardown race `DeviceAuthority::drop` specifically guards against (`crates/ezsdr-radio-uhd/src/authority.rs:493–510`).

   **Fix:** use a blocking test closure to hold `fire` in progress and establish the removal barrier before permitting owner destruction. Cover the four removal sites listed at `test-plan.md:119,236–239`; use narrow hooks only where necessary.

### Minors / missing sequences

- **Connect-time coverage is not equivalent to W1.** W1 proves sticky `add`, but neither W1 nor X2 proves that `connect` registers before its T0 advance, or safely unregisters on its ended path (`crates/ezsdr-server/src/lib.rs:225–239`). Add a gated connect/register-after-fire integration case. This challenges the adjudication’s **coverage premise**, not the settled design choice.

- **First-end testing is incomplete.** K5 covers disconnect-first and Policy-first, but not `finish` requesting `client` first followed by a signal during cleanup (`test-plan.md:41`). Pin that order with a cleanup gate.

- **Read-error propagation is untested.** C89-2-04 combines EOF and read error, but X1 exercises EOF only (`test-plan.md:110`). Add a controlled reader returning an error during an active request. Existing byte-input framing tests do not exercise the watcher.

- **Python’s exception coverage stops at KeyboardInterrupt during write/readline.** Add interruption during reply-body reading and a mocked `TimeoutExpired → kill → wait` case. These pin complete-reply abandonment and the 30-second fallback without actually waiting 30 seconds.

### Mutation concerns

- **C89-3-06 is potentially equivalent.** Removing an `abandoned` check still leaves the existing `stdin.closed` refusal (`python/ezsdr/_client.py:79–81`). P1 can remain green. Specify the final anchor and demonstrate a distinct wrong outcome, or omit the row.
- **C89-2-04 tests EOF only**, despite claiming read-error coverage.
- **C89-1-02’s named killer is sound as planned.** No other definite killer/path mismatch found by inspection.

### Hardware, text, maintainability

- The “no changed hardware-test expectations” argument is reasonable, but **X1 must be repaired before counting it as the bench rehearsal**.
- The bench’s pre-call `wait-start` marker and hold margin establish remaining duration, not that the server entered `advance` (`test-plan.md:185–188`). Record request-entry evidence or classify an unestablished run as inconclusive.
- **EA-15:** promise exit 0 specifically for **BrokenPipe**, not every unwritable reply (`test-plan.md:125`; current errors propagate through `transport.rs:91–93`).
- **EA-16/KL-43:** incorporate the adjudicated distinction between an earlier abandoned call and failure of `close()`’s own `finish`; retain missing/partial Manifest, timeout kill, inherited pipes, and hung-server qualifications.
- Avoid production `abandoned is True` solely to accommodate truthy test Mocks (`test-plan.md:162`); initialize those test doubles explicitly.

No settled owner direction is refuted by the evidence above.

Read-only review; no builds, probes, mutations, or hardware runs. The quoted E5 measurement was not independently reverified against its raw archive.
