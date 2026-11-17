# #106 — Sol's plan review (openai/gpt-6.1-sol, read-only; the final answer, tool transcript left out)

**Changes needed before implementation.** The lifecycle coverage is substantial, but ACK exemption and final-record retention have gaps.

References below use:
- **Plan:** `/home/komatsu/.cache/ezsdr-fixes/2026-11-17-n106/test-plan.md`
- **Decisions:** the sibling `decisions.md`
- **TX:** `crates/ezsdr-radio-uhd/src/provider/tx.rs`

I wrote the independent scenarios before reading the plan. This review is static: no files changed, builds, tests, or mutation runs.

### Coverage assessment

- **L1–L9, E1–E7, C1–C2:** generally cover the removed ending rule and preserved endings; specific script and mutation problems are below.
- **B1–B9, K1–K5:** generally provide direct, deterministic observations of retention, classification, serialization, and cardinality. B6 needs its overflow-bucket oracle clarified.
- **W1–W8:** cover ordinary bookings and resumes, but not all mixed booking/resume correlations or final-record writers.
- **S1–S5, M1–M2:** the proposed assertions distinguish selector parsing, propagation, pre-open refusal, and Manifest shape. S5’s counted opener follows the existing server fixture (`crates/ezsdr-server/tests/protocol.rs:1656–1666`).
- **Timing:** L11/W7 use virtual time, not wall-clock verdicts; `fake()` forces the manual clock (`crates/ezsdr-radio-uhd/tests/common/mod.rs:468–475`). Their cycle floors remain inferred, as the plan acknowledges (`Plan:81–85,168`). Twenty successful repetitions would not establish a deterministic minimum.
- **Mutations:** the proposed pure-decision mutants generally have distinguishing assertions. Exceptions and unsafe retirement guidance follow. Actual anchors and kills cannot be verified before implementation.

### Findings

1. **Major — A booked switch joined onto a resumed device burst can lose its ACK.**

   Minimal sequence: exhaust the ordinary `BurstAck` budget → resume repeat into part P → book switch S → S joins P’s still-open device burst → close it → receive its ACK.

   The proposed exemption considers whether a matched correlation has `part == false` (`Plan:20–21`). But joining adds S’s target to the existing correlation without changing that correlation’s classification (`TX:365–368,787–790`). A correlation originating from P therefore remains a “part” even though its ACK now acknowledges booked S too.

   **Wrong outcome:** S’s ACK is bounded and omitted, violating the booked-ACK promise (`Decisions:11`). W2 tests separate device bursts; W4b tests a switch replacing a paused booking, not this joined case (`Plan:147–159`).

   Add a saturated-budget test for this sequence. ACK exemption must account for booked joiners, including each channel’s acknowledgement.

2. **Major — Saturated final-record retention is not tested at abandonment or failed reopen.**

   Minimal sequence: N = 1 → enough resumes to exhaust `resumed_part` → send another part → terminate through sequence loss/send failure, or failed recovery.

   W4a ends with an ordinary Stop, which reaches `end_open_at` through `stop_now` (`Plan:157`; `TX:1310–1312`). Abandonment and failed reopen write final records separately (`TX:973–975,1177–1179`).

   **Wrong outcome allowed:** either separate writer keys its final record as `resumed_part`; the exhausted budget silently removes the closing record. Existing E2/E3 tests use the default budget and do not exercise this saturation (`Plan:94–95`; `crates/ezsdr-radio-uhd/src/provider/tx/tests/mt3d.rs:568–610`).

   Extend W4’s saturated sequence to these two writers and mutate each independently. The compiler-enforced keyed push cannot establish that the supplied key is correct (`Plan:166`).

3. **Major — N106-27 cannot be killed by its designated test.**

   N106-27 changes the `BurstStep::Ended` arm, but its only named killer is W4a (`Plan:252`). W4a observes discontinuity records followed by `BurstTracker::stop()`, not the `Ended` arm (`Plan:157`; `TX:817–824,859–861`).

   **Counterexample:** incorrectly bound every finite burst’s `Ended` record → send more than N finite bookings → later booked records disappear. W4a still passes because it never executes the mutated arm.

   Point this row at W5’s eight finite bookings with N = 3, which already has the appropriate distinguishing assertion (`Plan:164`).

4. **Minor — Several exact scripts contain baseline contradictions.**

   - **L2:** after `resumed_part()`, the owner is at 24 ms; `go()` does not advance time. The second resume is beyond that turn’s window, so merely calling `go()` cannot produce the asserted second discontinuity (`Plan:39–40`; `crates/ezsdr-radio-uhd/src/provider/tx/tests/mt3d.rs:84–95`; `crates/ezsdr-radio-uhd/src/provider/tx/tests/mt3.rs:289–296`). Specify advancement to the new part’s send window.
   - **C3:** it calls `stop()` and then expects `open_next()` to remain `Some` (`Plan:110–111`). Move that liveness assertion before Stop; Stop ends the open booking (`TX:1310–1312`).
   - **L10:** recovery uses `R_K − 5 ms` after the final cycle has advanced to `R_K + 1 ms`, and its `transmitted(R_K, …)` passes samples where root ticks are required (`Plan:76–77`; `crates/ezsdr-radio-uhd/src/device.rs:1145–1158,1200–1207`). Recover at the *next* planned point and use `clock.instant(...)`.

5. **Minor — Do not retire MT3D-55 merely because the proposed killer survives.**

   The fallback says MT3D-54 already pins preservation of `paused` (`Plan:342`). It does not: MT3D-54 changes `first`, whereas MT3D-55 removes `paused` (`plan/maintenance/tools/mutations.json:25348–25366`).

   The behavior remains reachable and meaningful after removing the stall field. The existing staging test explicitly asserts `Some(Paused { … })` after `replace_paused` (`crates/ezsdr-radio/tests/staging.rs:3179–3190`). Retarget MT3D-55 there rather than treating survival of L6 as equivalence.

6. **Minor — B6’s oracle must aggregate overflow kinds.**

   B6 generates 100 kinds but asserts `kept + omitted == pushed` “for every kind” (`Plan:126`). After 64 kinds, the accepted design deliberately merges new kinds into `other` (`Decisions:12`; `Plan:18`).

   Assert conservation per **effective bucket**, using an independently computed first-64 mapping, plus global conservation. Otherwise the proposed baseline assertion is undefined or false for overflow kinds.

7. **Minor — The reader guard does not cover all audited readers.**

   OQ-6 is adopted (`Decisions:15`), but guarding `common::section()` does not make the reader audit “moot” (`Plan:338`). Throughput readers access `Manifest::section()` directly (`crates/ezsdr-radio-uhd/tests/common/measurement.rs:3218–3221`; `crates/ezsdr-radio-uhd/tests/common/measurement/multichannel.rs:874–878`).

   Keep those readers explicitly audited, or give them the same completeness check. Add a synthetic omitted-Manifest test for the guard and its opt-out. The present `section()` has no such check (`crates/ezsdr-radio-uhd/tests/common/mod.rs:613–615`).

8. **Minor — The text amendments retain an already-refuted promise and miss a stale table cell.**

   - T5 still says the gap is “told by the event” (`Plan:318`). An expired-check resume can have no event, intentionally tested today (`crates/ezsdr-radio-uhd/src/provider/tx/tests/mt3d.rs:451–466`). Use the accepted distinction: every resume is counted; a device-reported drop is told by an event (`Decisions:13`).
   - `design/18-uhd-radio.md:571` still promises that the removed stall part remains unchanged. T1 changes the rule and Test list, but does not list this §6 expectation cell (`Plan:314`). Include it in the amendments.

**Verdict: REQUEST CHANGES — close the three major coverage gaps and repair the listed plan inconsistencies; mutation kills and bench claims remain unexecuted/unverified.**
