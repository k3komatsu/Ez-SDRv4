# #98 — Sol's test-plan review (2026-11-12, openai/gpt-6.1-sol, read-only)

Before reading the plan, my failure scenarios are:
- A lost error from `apply`, versus another thread publishing loss before configure completion: retain the booking, keep the mark cleared, emit one `DEVICE_LOST`, and cancel once.
- An ordinary refusal racing loss: decide which outcome wins at a defined point; never both refuse and cancel the booking.
- Stop/Provider shutdown queued during configure, including two held bookings: settle the operation first, preserve receipt-order cancellation, and never retry or send after loss.
- A pending switch: loss must settle it without configure completion removing another booking’s footprint or duplicating its accounting.

I’ll compare the tests and mutations against those cases, including whether their ordering is deterministic.
## PASS WITH CHANGES

No major defect found in the proposed fix. Using **this configure’s `error.lost`**, rather than `core.is_lost()`, consistently decides retention and reporting; S6 correctly preserves an ordinary refusal racing a separately published loss.

Paths below under `/home/komatsu/.cache/ezsdr-fixes/2026-11-12-n98/` unless specified.

### Minor findings

1. **Add Provider shutdown during the lost configure. — VERIFIED coverage gap**
   `test-plan.md:42,53` covers an ordinary Stop during configure, then calls `Tx::shutdown` afterward. It does not exercise `Provider::stop` booking the end **before** configure completion. Existing C6 covers this ordering only for an ordinary refusal (`crates/ezsdr-radio-uhd/src/provider/tx/tests/e2b.rs:388–414`).

   Add a deterministic hook that books `streams.stop`/`streams.end`, queues Shutdown, and then returns a lost configure result. Assert one cancellation, no refusal/send, cleared mark, and returned claim. This also exercises loss after `Streams::ended` is already true (`provider/core.rs:652–655`).

2. **Correct the mutation-killer claims. — VERIFIED by static tracing**
   `test-plan.md:72,76` incorrectly lists C4 as killing:
   - **N98-01:** C4(ii) already publishes loss before completion, so `result.is_ok() || core.is_lost()` retains it correctly; C4’s other cases also remain unchanged.
   - **N98-05:** C4(ii)’s loss has already removed X’s chain; the later `drop_piece` changes nothing, and C4 does not inspect `Bookings`.

   T4 and T2’s device-reported-loss mode are appropriate respective killers. Remove the C4 claims; do not count these as additional independently verified kills.

3. **Make the promised retry observation real. — VERIFIED**
   `test-plan.md:28,53` promises an assertion against another configure attempt. But `draft-e2b.rs:385–406` computes call information and discards it (`let _ = extra`). Moreover, `FakeDevice::calls()` does not record rejected wrapper attempts; `ApplyFails::apply` records those separately (`provider/test_support.rs:675–680`).

   Assert the wrapper’s attempt count, or count the configure probe invocations, across subsequent turns. Drain events after shutdown too when asserting the complete no-refusal/one-loss outcome.

### Other conclusions

- Frozen, synchronous hooks make the new ordering tests independent of thread scheduling, consistent with note 51 §3.
- The pending-switch exclusion is **INFERRED**, reasonably argued; optional T5 demonstrates book preservation, not Provider reachability.
- Narrow the proposed normative wording (`test-plan.md:87–88`) to **a configure returning `lost: true`**; “cut by a loss refuses nothing” would otherwise contradict S6.

Read-only review only: no edits, builds, probes, or mutation execution; no temporary copy created. Reported probe results were not independently reproduced.
