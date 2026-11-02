## Verdict: REVISE

Paths below are relative to `crates/ezsdr-radio-uhd/src/`, except mutation-row references.

### Earlier findings

| # | Status | Evidence |
|---|---|---|
| 1 — FIFO command loss | **Closed.** | `brain/threads.rs:423–430` atomically removes the batch; later arrivals remain for the discarder (`404–407`). Regression checks exactly one terminal and `Done` per command (`brain/thread_tests.rs:795–810`). |
| 2 — Panic coverage | **Partly closed.** | Step and operations are caught (`brain/threads.rs:361–365`). Scratch cells are restored before poisoned locks are recovered (`brain/shared.rs:62–66,186–197`). Guard-path unwinding still fails; see P1 below. |
| 3 — Actual executor exit | **Closed for the tested sequences.** | Discarder ends on `brain_gone` (`brain/threads.rs:404–406`); S5 checks scheduler completion after the thread body returns, before teardown (`brain/thread_tests.rs:555–585`; `brain/force.rs:322–329`). |
| 4 — Premature command issuance | **Test corrected; mutation obligation open.** | S10b reads routed commands before acknowledgement, after `Finished`, and after `Quiesced` (`brain/thread_tests.rs:817–834`). E2-14 still tests missing release, not early issuance. |
| 5 — Missing witness | **Closed.** | Declarations accumulate across seeds (`brain/thread_tests.rs:103–110`); the negative regression removes the first seed’s unique declaration from subsequent seeds (`159–168`). |
| 6 — Sample computation under lock | **Implementation closed; regression incomplete.** | Compute runs outside the cell; validation and write run inside (`brain/threads.rs:653–663`). `unchanged_since` covers the cell fields computation reads (`brain/cell.rs:340–346`). See P2-C. |

### New findings

**P1-A — A panic during the guard can strand its own lifecycle gate.**
- Minimal sequence: set `panic_in_op = Some(0)` → deliver `Input::Panic` → guard acquires `G` → injected operation panic.
- `brain_op` acquires `G` before the injection (`brain/threads.rs:295–303`). Recovery clears operations and restarts the guard at `Gate` (`369–376`), but never releases the already-owned gate.
- Observable outcome: the brain waits forever for its own `G`; `F` remains false, `mark_lost` never runs, and executor termination is never routed.
- A panic during recovery’s `run_ops` instead escapes the catch entirely (`376`). Make guard recovery ownership-safe and test unwinding during the guard, not only during reservation.

**P2-B — No mutation tests premature issuance.**
- The redundancy claim is correct here: before `Quiesced`, `retiring` remains set and `outstanding = Plan` makes `busy()` true (`brain/core.rs:473–477,584,833–837`; `brain/cell.rs:503–505`).
- That explains why removing either guard alone is ineffective; it does not replace testing the obligation.
- `plan/maintenance/tools/mutations.json:18205–18213` only removes acknowledgement release. Add a targeted composite row bypassing both barriers and require S10b’s **pre-acknowledgement** assertion to kill it.

**P2-C — The new compute/write race is not exercised through the runtime.**
- `publish_write_rechecks_what_compute_read` directly asserts the helper’s answer after `book_stop` (`brain/rules.rs:715–729`). E2-37 consequently tests the helper, not its use at `brain/threads.rs:663`.
- Concrete missed regression: compute before acceptance → S records `accepted_at` → write without `unchanged_since`. Acceptance does not increment revision (`brain/cell.rs:461–464`), so the snapshot check alone permits publication without the packet’s newly qualifying consumption evidence.
- Add one controlled runtime interleaving between compute and write that fails if the runtime bypasses validation.

### Adjacent sequences and rows

- No remaining threaded FIFO snapshot-then-clear path was found. R’s clears intentionally discard RX commands; settings batches and discarder pops have disjoint ownership.
- Ordinary `L`/`G` waits still depend on the current holder completing; brain death does not cancel acquisition. Note 45 §4.7 explicitly qualifies bounded shutdown when native locking blocks progress.
- No self-induced retry livelock was found: a Stop booking increments revision once per processed Stop, not per retry. Continuous competing writes can still cause starvation. Forced replay retains `RSnap` and `RPublish` points.
- All nine named changed/new row anchors match exactly once. Their expected kills follow by inspection; E2-14/E2-36 test acknowledgement progress, and E2-37 has the limitation above.

Not checked: test or mutation execution, full gate, hardware, E2a-2, or the prohibited holdout directory. No files or git state changed.
