# Sol 6.1 — E2b code review, report section

# REVISE

This measures a useful implementation result, but not yet a fair, design-isolating E2b comparison. The recorded **1 MS/s runs pass numerically; the high-rate executable fails unequivocally**. Neither establishes the note-45 design’s final verdict.

Paths below are relative to `crates/ezsdr-radio-uhd/src/` unless stated.

## Failure scenarios considered first
Late ingress changes booking; pooling hides a bad repetition; receive-to-reader loss escapes accounting; saturation strands an update; growing test bookkeeping dominates receive work; a timer delivers early.

## Findings

1. **P1 — The FakeDevice configurations differ.** `brain/speed/newrt.rs:86–95` changes the master clock to the sample rate; `brain/speed/today.rs:26,71–73` retains 200 MHz. Thus decimation is **1 versus 200 at 1 MS/s**, and **1 versus 4 at 50 MS/s**.
   - Counterexample: the same 1 MS/s condition exercises unit-stride tick handling on new, but decimated handling on today. Equal samples/second does not satisfy “same FakeDevice configuration.”
   - This can change timing and loss verdicts. Make the runtime handle the common clock/decimation, rather than changing its device.

2. **P1 — The ingress schedule is neither the specified 50 ms schedule nor genuinely bounded by unapplied updates.** `brain/speed/runs.rs:191–200` gates on an earlier update’s **instant passing**, not its application.
   - Counterexample: after eight entries, spaced ingress waits until `e−40 ms`; bursty ingress is typically `e−20 ms`. Recorded median lateness is consequently **10/30 ms**, not scheduling noise. If earlier updates never apply, further ingress nevertheless continues.
   - The brief itself needs adjudication: 50 ms lookahead necessarily contains more than eight pending updates for these tables. Neither owner can satisfy both requirements literally.
   - Actual effective instants and per-update ingress lateness are not retained in the output; `moved_by_floor` is only a harness prediction. Schedule/queue pressure can change a verdict.

3. **P1 — The high-rate result cannot distinguish protocol cost from test-adapter cost.** The ranked costs below include sample-by-sample chain reconstruction and history-sized cloning, neither required by note 45.
   - Counterexample: receive execution falls behind while those adapters run; the fake skips samples, four obligations remain outstanding, and later updates never reach `apply`. This reproduces the observed failure without establishing that cell locking or one R per stream is intrinsically too slow.
   - Recommendation: retain this failure record, then authorize a narrowly scoped adapter cleanup with E2a equivalence checks before drawing a design conclusion.

4. **P1 — Saturation can strand held values even after capacity returns.** `brain/core.rs:518–530,644–653,724–739`.
   - Counterexample: fill four slots; release a fifth value; `reserve` returns with it held. Consume an earlier value and free a slot. `recheck` services deferred requests, **not held reservations**; the fifth receives no retry during continuous streaming.
   - This is not a terminal “refusal”; it is an unreserved held value. Counting its absent application as **NoApply/failure is correct**.
   - The bound is a strong **INFERRED** explanation for these runs, not a measured attribution: record bound hits, reservation rejection reasons and affected IDs. The report currently observes no-call outcomes, not their causes.

5. **P1 — The verdict is not fail-closed for the complete comparison.** `brain/speed/runs.rs:229–250,340–354`.
   - Counterexample: today refuses one update, new applies all, and latency/loss checks pass: the function returns PASS because it never requires `failures(today)==0`.
   - Unexpected applies, extra DSP entries and floor movement are likewise reported but excluded from the verdict. Gate these explicitly.
   - Pooled p99 and summed losses can also mask a bad repetition; retain repetition-level verdicts rather than relying solely on pooled checks. The supplied low-rate repetitions individually satisfy the numerical limits, so this does **not** reverse their observed result.

6. **P2 — Loss is not reconciled at a common receive-to-reader endpoint.** `brain/speed/runs.rs:85–93,126–142,203–209`; `brain/threads.rs:738–740,791–803`.
   - Snapshots read independently changing counters before stopping/draining. A final produced-but-undelivered block has no later block to expose its gap, and need not increment `link.drops()`.
   - New also publishes the complete raw payload **before** the cell accepts its publication; protocol-discarded samples are not reflected in that sink.
   - Reconcile produced, protocol-discarded, queued and reader-delivered work at the boundary. These defects can change close loss comparisons, though they cannot explain away the enormous recorded high-rate failure.

7. **P2 — Cell diagnostics are acquisition-level, not per-block measurements.** `brain/shared.rs:50–56,197–204`; `brain/speed/newrt.rs:187–188`.
   - Global histograms mix B/S/R acquisitions. Hold timing excludes histogram work performed while the mutex remains held; wait-histogram recording occurs before the hold timer starts.
   - Report these honestly as acquisition diagnostics, or collect the requested per-block R distributions. This affects interpretation, not the current verdict runs.

8. **P3 — Two diagnostic inaccuracies.**
   - `brain/speed/mod.rs:470–472`: when an update has no matching DSP entry, a later legitimate update at the alternating frequency can be labelled `AppliedAt`. That fallback does not preserve one-to-one identity; pass/fail remains failure.
   - `brain/speed/mod.rs:305–320`: bracket uncertainty omits tick quantization—up to roughly **1 µs** at the new 1 MHz clock. This understates uncertainty; it is too small to overturn the observed low-rate margin.

## Ranked hot-path costs
Estimates below are operation counts, **not measured nanoseconds**; source mechanisms are VERIFIED, dominance is INFERRED.

1. **Per-sample scratch reconstruction:** `brain/rules.rs:37–59,380–387`. A 500-sample block makes 500 scratch-chain constructions and about **1,000 tiny payload allocations**, before debt/order bookkeeping. At target rate, that is roughly **100 million allocation operations/s**.
2. **Growing consumed-history clones:** `brain/threads.rs:589–591,627,757,794`. Normally two `RxState` clones plus one consumed-set clone per cycle: **O(3D)** copied IDs/B-tree storage for D historical consumptions. IDs are never pruned here; cost grows throughout a run.
3. **FakeDevice history scanning, common but unequal in practice:** `device.rs:548–553,1793–1801`. `freq_constant` scans the accumulated DSP log under the fake mutex: up to **10,000 entries/block** on today, fewer on new because most updates never apply. The no-update trial does not exercise this growing cost.
4. **Tick materialization and repeated scans:** `brain/rules.rs:257,423–427,463–465`: roughly **two 500-element tick vectors (~8 KB)** plus consumption/filter/progress scans per block, even though measured runs discard the tick output.
5. **Test scheduler and fixture overhead:** `brain/threads.rs:589,761,771,805`; `brain/force.rs:133–140,224–242`. Predicate allocation, scheduler mutex/notification work, cell clones and hook-mutex reads remain per cycle. New’s sink additionally takes one pool mutex/block (`brain/speed/newrt.rs:117`).
   - I found **no per-sample mutex acquisition** in this adapter path. FakeDevice holds its state mutex across sample generation; that is common to both owners.

## Checks that hold
- Same requested receive length, `rx_packet=None`, cf32 conversion, `SampleBlock`, bounded link and reader: substantially equivalent payload work; publication ordering differs as noted above.
- Acceptance delay correctly uses successful `apply` return (`brain/speed/runs.rs:218–220`), not entry.
- Exact successful DSP matching uses `(tick, frequency, timed)` one-to-one; nearest-rank p99.9 correctly uses index **9,989/10,000**.
- ABBA then BAAB is implemented; per-run results and separate counter/timing diagnostics are retained.
- The condvar release timer rechecks the deadline after waking and never sends early on its mapped host timeline (`brain/speed/newrt.rs:48–67`).
- Rows E2-62…65 target the stated decisions and have assertions that should kill them; E2-64’s name is misleading.
- With bench/instrumentation off, the edits preserve E2a’s logical path; production excludes `brain` via `lib.rs:8–9`. Added dormant checks remain.

Not checked: no builds, tests, mutations or profiling rerun; no files changed, and holdout was not accessed. **Do not treat this executable’s high-rate failure as proof that note 45 must be rejected.**
