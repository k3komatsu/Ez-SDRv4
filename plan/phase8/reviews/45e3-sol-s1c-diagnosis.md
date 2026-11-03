# Sol 6.1 — E3 s1c-diagnosis, report section

**Recommendation: diagnose the first differing payload/state, not the reported parking point. These errors establish replay disagreement, not protocol failure.**
Paths below are relative to `crates/ezsdr-radio-uhd/src/brain/`.

1. **`BStep` expected / `BOp` parked — seeds 2005, 2619, 6348, 7047**
   - **VERIFIED:** there is a receive-adapter ordering mismatch: World queues `LateCommand` at `REnd`, then appends publication boundaries behind it (`world.rs:1044–1045,1094`).
     Threads send publication boundaries **before** `LateCommand` (`threads.rs:911–921`).
   - **INFERRED, leading suspect:** different inbox order makes Brain process different inputs; those inputs can generate different operation counts (`core.rs:322–336,444–489`), eventually leaving threads at an extra `BOp`.
     The supplied log has no payload trace, so attribution to these four seeds remains unconfirmed.

2. **`RSend` expected / `RCheck` parked — seeds 4572, 8087**
   - **VERIFIED:** threads finished their sends and reached the receive loop’s next decision (`threads.rs:645,911–921`); the controller rejects rather than skips the missing send (`force.rs:415–424`).
   - **INFERRED:** first inspect the preceding `RPublish`: a missing `Consumed`, `FirstSample`, or `CutReached` boundary means its decision inputs differed (`rules.rs:453–470`).
     Acceptance visibility/timing is one concrete candidate: World accepts at `SEffect`; threads normally accept after `SUnlock` (`world.rs:929–932`; `threads.rs:581–584`; `model_device.rs:114–122`).

3. **`RCheck` expected / `RSend` parked — seed 4898**
   - **INFERRED:** the inverse: threads produced an extra boundary, or earlier send-payload ordering changed subsequent state. Inspect the pending `RSend` payload and preceding publication/call result (`threads.rs:753–769,911–921`).
   - **VERIFIED:** this shape also occurs in **classified Q1** runs (supplied log, lines 4–5). The parking shape alone does not identify its cause.

4. **Replay limitation versus real defect**
   - **VERIFIED:** `UNCLASSIFIED` means neither Q1 intervention restored **the entire replay and final equality**, not that Q1 contributed nothing (`thread_tests.rs:646–651`).
   - **INFERRED:** adapter sequencing is more plausible than a controller limitation; `settle()` cannot reconcile genuinely different next hooks (`force.rs:386–436`).
     Nevertheless, reordered protocol messages or different consumption decisions could affect retirement/draining: **none of these seven is yet safely dismissible as replay-only**.

5. **Why “shrunk to 0 events”**
   - **VERIFIED:** only the first **three total divergences**, including Q1, are shrunk; every later divergence receives a placeholder empty vector (`thread_tests.rs:643–644,653`).
     Thus `[]` need not have been replayed at all. This is misleading reporting, not evidence of a shrinker failure.

6. **Smallest confirming diagnostic**
   - Replay only seeded configuration **8087**, printing indexed events through event 51, World’s pending RX send payloads, thread send payloads, and both inboxes.
     At the preceding `RPublish`, print samples, snapshot/current cell, acceptance instants, and computed boundaries; repeat with A/B and report their **first errors**, not merely classification.
   - Then inspect one BStep seed for the first differing Brain input, specifically `LateCommand` versus publication-boundary order.

No edits, formatting, commits, or reruns performed; holdout was not accessed. Seed-specific causes remain **inferred** pending that diagnostic.
