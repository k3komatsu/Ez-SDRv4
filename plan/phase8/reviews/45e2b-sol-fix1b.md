# Sol 6.1 — E2b fix1b review, report section

# REVISE — Part B

What this change does: it repairs much of the measurement accounting, but **B1–B6 are not fully satisfied**. The recorded high-rate failure is real for this executable, not yet a verdict against note 45.

Paths below are relative to `crates/ezsdr-radio-uhd/src/`. `R` denotes `~/.cache/ezsdr-fixes/2026-11-01-n45/e2b-results-2/`.

## Independent failure scenarios
I first considered: expiry followed by a delayed return; pooling hiding one bad repetition; a final missing block; publication before acceptance; duplicate DSP matching; and per-block instrumentation waking unrelated threads or accumulating history.

## Findings
1. **P1 — B3 no longer measures device-side missing samples.** `brain/speed/runs.rs:295–313`.
   - Sequence: FakeDevice skips samples on overflow, then every returned block reaches the reader. Produced-minus-delivered is zero, despite a timestamp gap.
   - `Reader::missing` records that gap (`:84–94`) but is excluded from the verdict. High runs consequently report hundreds of overflows with zero missing samples.
   - Restore timestamp-gap accounting, reconcile publication loss separately, and include leading/trailing loss without double-counting. Today’s derived discard residual also needs an independently justified boundary.

2. **P1 — The high-rate comparison still includes unbounded, new-only per-block tracing.** `brain/threads.rs:125–126`; `brain/model_device.rs:336–348,417–418`.
   - Every receive appends to `CheckingDevice.log`, even in counters-off runs. At 100k blocks/s: **5 million records/50 s**, approximately **120 MB live entries** on this platform, plus capacity growth.
   - This is test-adapter work, not the cell protocol. Disable that history for measurement runs while retaining the no-cell-held assertion; otherwise throughput cannot isolate the design.

3. **P2 — B1 expiry can be missed, and booking evidence remains inferred.** `brain/speed/runs.rs:206–227,284–291,361–374`.
   - Sequence: an update queues successfully, but its owner returns after `e+10 ms`; the final outstanding entries are never revisited. A later `has_returned=true` also suppresses the expiry check.
   - If DSP application was at `e`, this update can remain “clean.” Check each recorded return against its deadline directly.
   - “Observed effective” pairs orphan requests with arbitrary stray apply instants; it is not an observed booking. Persist actual booking identity/effective instant and the requested/ingress timestamps.

4. **P2 — B5 attribution is incomplete and misclassifies failures as bound hits.** `brain/speed/newrt.rs:200–210`; `brain/speed/mod.rs:143–154`.
   - A reservation rejected because the cell is lost is called a bound hit: the fault run reports **357** such hits (`R/fault.txt:7`).
   - High spaced repetitions drop **2.7–5.3 million reservation records** (`R/verdict-high-spaced.txt:10,12,16,22`), losing later successes and affected IDs.
   - Record the actual rejection reason and bounded **per-update aggregates**, not every attempt.

5. **P2 — B2 does not close the complete observation.** `brain/speed/runs.rs:244–254,314–329,527–540`.
   - Apply/DSP snapshots precede shutdown; unexpected calls during shutdown escape accounting. Failed joins, panics and failed orderly exits are not cleanliness conditions.
   - Per-repetition `"pass"` also ignores cleanliness: an unapplied update can coexist with `"pass":true`.
   - Snapshot after successful stop/drain; gate lifecycle success and include cleanliness in each repetition’s verdict.

6. **P3 — B6’s forbidden frequency fallback remains.** `brain/speed/mod.rs:529–551`.
   - Exact matches and diagnostic labels now consume entries one-to-one, but a stray same-frequency entry can still label the wrong failed update `AppliedAt`.
   - Remove that fallback. Tick quantization and E2-64’s renamed description are corrected.

## Ranked diagnosis (a): 1 MS/s bursty regression
Mechanisms below are **VERIFIED**; causal ranking is **INFERRED**, not a measured attribution.

1. **Timer-to-brain-to-S scheduling is the leading suspect.**
   - New p99 entry is **303–380 µs**, acceptance **316–403 µs**; release p99 is only **95–99 µs** (`R/verdict-1mhz-bursty.txt:9,11,15,21`).
   - First-round release p99 was approximately **64–74 µs**. Timer degradation explains part, not all, of the increase; most delay exists **before apply entry**.
   - Each update entails multiple broadcasts and scheduler acquisitions, beyond the necessary inbox/FIFO handoffs (`brain/threads.rs:378–415,467–487,507–582`).

2. **S’s serial processing amplifies clustered releases.** `brain/threads.rs:507–591`.
   - Four releases are 100 µs apart. Earlier S bookkeeping/calls delay later entries; measured later burst positions generally have worse tails.
   - One S is design cost; scheduler bookkeeping, observation vectors and reservation stamps around it are adapter cost.

3. **FakeDevice mutex contention contributes, but does not explain the whole increase.**
   - Sample generation holds its state mutex across 2,000 samples (`device.rs:1785–1803`); apply needs that mutex (`:1423`).
   - Its DSP history scan is **O(H) per block** (`device.rs:548–553`): up to 10k comparisons/block. This affects both owners.
   - Decimation 200 does **not** imply 200× work: tick mapping is constant-time; payload count remains 2,000.

4. **Other candidates are weaker.**
   - B1 holds unreleased values in **RelTimer**, not the brain (`brain/speed/newrt.rs:190–192,63–66`); low-rate bursty runs have **zero bound hits**.
   - Moving the sink after acceptance delays consumption messages by one conversion/publication’s work (`brain/threads.rs:835–858`); plausible, unmeasured.
   - B4 timestamping is off in verdicts (`:754–765`). B5 adds one clock read, mutex and append per normal reservation—not per block.

## Ranked diagnosis (b): intermittent high-rate dips
1. **Scheduler wake-up amplification is the strongest adapter suspect.**
   - Each ordinary block broadcasts from R acquisition and publication (`brain/force.rs:233–242`; `brain/threads.rs:855`): roughly **200k broadcasts/s**.
   - Brain, S, clock and timer share that condvar; potentially **~800k unnecessary predicate rechecks/s**, serialized by the scheduler mutex (`brain/force.rs:175–192,224–249`).
   - Predicates additionally lock queues/timers. Hooks themselves return immediately outside forcing (`:200–203`).
   - This can create scheduler-dependent convoys and recovery, matching intermittent dips; it does **not prove** their cause.

2. **Remaining receive-adapter work consumes the 10 µs/block budget.**
   - R evaluates `loop_next` twice, then snapshots/views/writes: ordinarily **five cell acquisitions/block**, versus the design’s two publication sections (`brain/threads.rs:609–610,643–654,772,798,812`).
   - `consume_samples` still allocates two tiny one-channel payload vectors even with no pending values (`brain/rules.rs:383`), about **200k allocations/s**.
   - Add the unbounded checking log above. These costs also existed in the short run; duration-dependent allocator/scheduling effects remain unverified.

3. **History costs explain update-run deterioration, not the no-update trial.**
   - FakeDevice scans growing DSP history under its mutex; at H=10k and 100k blocks/s, up to **1 billion comparisons/s**.
   - Brain result/disposition searches also grow with completed updates (`brain/core.rs:380–393,602–605`).
   - Saturation retries every held value (`:399–401,643–647`); millions of attempts add recording overhead and amplify backlog. None explains an update-free trial by itself.

## Minimal confirming diagnostics
- **Throughput:** repeat only the 50 s, 50 MS/s × 500 **no-update** trial with per-second context switches, migrations, page faults and sampled CPU stacks. Compare good/dip seconds and per-thread scheduler/futex activity; this separates wake convoys from log growth and device generation.
- **Latency:** one bursty diagnostic with bounded per-update stamps at release-send, brain dequeue, S dequeue, apply entry/return. That identifies which stage contributes the missing ~200 µs; marginal percentiles cannot.
- B3 sink ordering is repaired; global B2 gates are substantially improved; B4 acquisition labels are honest, but they are **not per-block distributions**. The high-rate FAIL remains unequivocal despite the accounting defects.

Not checked: no tests, mutations or profiling rerun; no files changed or holdout accessed. **Do not reject note 45 from these adapter-contaminated results.**
