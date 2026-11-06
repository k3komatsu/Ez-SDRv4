# Design note 48 S3 — Sol's code review (2026-11-06, verbatim; verdict FAIL)

## Major

1. **A second Stop loses the closed-short-burst frontier** — `crates/ezsdr-mock-radio/src/lib.rs:1128–1142`.
   - Minimal sequence, inside the window: book A of 10 samples at `s`; Stop; Stop again before A plays; book B with `DropAndFlag` at `s+11`.
   - The first Stop retains payload cut `s+10` and safe front `s+11`. It removes A. The second Stop finds no `floor`, reuses the retained cut, and advances the front to `s+12`.
   - **Observed:** a review probe drops B with `TIME_ERROR`, `late_by_ns=1000`. The Module preserves its closed front across the second Stop and admits B.
   - Preserve the closed front when Stop hands nothing new; do not reconstruct its closing margin from the payload cut alone. Extend the Stop parity test to repeated Stops. This is an S3 lifecycle regression, not the deferred receipt-order #79 cases.

2. **FakeDevice emits an extra TimeError for the bench’s untimed-continuation shape** — `crates/ezsdr-radio-uhd/src/device.rs:564–591,748–759`.
   - Minimal sequence at 1 MS/s: timed SOB of two samples, open; advance 1 ms; untimed middle packet; untimed EOB.
   - The first untimed continuation inherits `Duc::time` in `packet.radio_time`, although its incoming metadata is untimed. `faulted()` therefore treats it as timed and retains `Burst::Timed`; the next untimed packet emits another TimeError.
   - **Observed:** `[Underflow, TimeError, TimeError]`. The bench records **one** TimeError for untimed continuations (`plan/phase8/bench-results.md:329`; reproduction at `plan/phase8/tools/uhd-repro/tx_repro.cpp:355–365`).
   - Base this report-mode transition on the incoming packet’s timing metadata, not the residual DUC timestamp. Add the mixed timed-SOB/untimed-continuations test. The existing untimed test starts with an **untimed SOB**, so it misses this case.

## Minor

3. **The amended specs still contain contradictory rules** — `design/18-uhd-radio.md:255–256`; `design/09-mock-radio.md:143`.
   - UR-23 still unconditionally says an adjacent burst continues the device burst and shares its ACK. S3 closes a short predecessor and decides that adjacent booking late.
   - MR-17 retains the blanket “two samples” frontier, then describes the short frontier as “at least” padded-end+1. That does not clearly specify the implemented **replacement** of +2.
   - State the short-burst exceptions explicitly. Also move the new bench history and unmeasured-boundary explanation at `design/18-uhd-radio.md:332` into the Changes/decision record, as AGENTS §2 requires.

4. **KL-36’s workaround fails for lengths 1–3** — `KNOWN_LIMITATIONS.md:135–137`.
   - A one-sample A at `s` is padded through exclusive device end `s+4`; its safe successor starts at `s+5`.
   - Moving B from A’s payload end `s+1` “one sample later” gives `s+2`, which is still late.
   - Specify **padded device end plus one**, with end+1 sufficient only for unpadded lengths.

5. **Two renames are missing from the rename record** — `plan/phase8/reviews/48s3-renames.txt:1–3`.
   - `ur_23_a_four_sample_window_takes_a_first_send_of_four` → `ur_23_a_32_sample_window_takes_a_first_send_of_32` (`crates/ezsdr-radio-uhd/src/provider/tx.rs:4954`).
   - `s1_parity_short_burst_then_adjacent_burst_both_orders` → `s1_parity_short_burst_then_adjacent_burst_before_and_after_dispatch` (`crates/ezsdr-acceptance/tests/parity.rs:1569`).

## Note

6. **The checked-in full-Provider test is not the discriminating HEAD reproduction** — `crates/ezsdr-radio-uhd/tests/fake.rs:2860–2876`.
   - The implementation log records that it passed with the HEAD Module because its first send contained 55 samples (`~/.cache/ezsdr-fixes/2026-11-06-n48s3/impl.log:2945–2971`).
   - A temporary deterministic Module test did produce a 28-sample first send and missing continuation payload, but was subsequently deleted as a duplicate.
   - Retain that distinct deterministic device-path regression, asserting the reports as well as payload, rather than treating the surviving test as equivalent coverage.

**Checks:** external-copy UHD unit tests: 363 passed; Mock suites: 136 passed; S3 parity and full-Provider tests passed. All 1,540 primary mutation anchors match uniquely. No additional normal Module path leaving an undersized open first send was found.

**Verdict: FAIL.**

Not checked: hardware, full gate, or fresh mutation executions; mutation results were inspected from the supplied records. Repository files were unchanged; all review processes ended.
