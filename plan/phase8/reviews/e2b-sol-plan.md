Before reading the test plan, my failure scenarios are:

- Configure finishes after Stop: success must become one cancellation note; refusal must remain one rejection naming the booking, without a second Stop note. Bookings received after Stop must survive.
- Configure stalls: dispatch must recheck time, placement and queued receipts; configuration alone must not make a repeat “started” (KL-41).
- Loss or `Provider::stop` overlaps Configure/Reopen: preserve today’s configured-mark, cancellation and event rules, including their differences.
- Reopen succeeds: retain the ACK-derived front, restore the configured generation, and discard old report correlations before payload resumes. Failure must preserve today’s clock-ending and cancellation order.
- Protocol mistakes: stale, duplicate or wrong-kind completion; payload issued before Configure completes; halt removes an in-flight booking.
- Future stages: keep configure refusal tied to its booking for C1, send outcomes distinguishable for C2, and actual close/front facts available for C6.

I’ll now compare these against the brief and plan, including the claimed absence of the `(d1)` race.
## Verdict: **FAIL**

The characterization inventory is useful, but the proposed protocol omits the requested tokens and explicitly changes a receipt interleaving.

References below: **TP** = `/home/komatsu/.cache/ezsdr-fixes/2026-11-12-e2b/test-plan.md`; **TX** = `crates/ezsdr-radio-uhd/src/provider/tx.rs`.

### Major findings

**1. No operation identity: “applied once” is checked only until another operation starts.**  
**VERIFIED shape; INFERRED counterexample.** TP:67–80 preserves tokenless `issue`/`complete` signatures. P3 checks duplicate completion with nothing—or another *kind*—in flight, not another operation of the same kind (TP:181).

Minimal sequence:
```text
issue Configure(X)
complete_configure(true)
issue Configure(Y)
replay X's complete_configure(false)
```
The replay consumes Y’s operation and removes Y’s booking. Reopen has the same identity hole. This contradicts the brief’s “issued with a token” requirement and note 49 §5.4.4 (`plan/phase8/49-time-and-booking-inventory.md:216–223`).

**Required:** completion must identify its issued operation; test stale same-kind completion, wrong token and duplicate completion. Include Reopen, not only Configure/Send.

**2. Merging the refused-configure sections changes a caller-visible refusal.**  
**VERIFIED lock boundaries; INFERRED execution.** TP:97–102 acknowledges the merge but calls it unobservable because no probe exists there. Production threads do not need probes.

Minimal sequence:
```text
X's configure returns a non-lost refusal
TX publishes configured=None and releases streams
control receives on-time Z overlapping X's still-recorded span
TX removes X's span
```
Today Z is refused (`TX:620–633`, `control.rs:581–584`). With TP:89’s merged section, a receipt following publication sees X already removed and can accept Z. Thus “every refusal stays as today” is not established.

**Required:** preserve this boundary, or obtain explicit owner approval for the changed linearization. Characterize the sequence deterministically with a barrier/probe; do not dismiss it as unobservable.

**3. E2B-11’s proposed killer does not exercise its mutation.**  
**VERIFIED ordering; INFERRED survivor—not executed.** TP:249 moves the loss read outside the result lock, but its killer publishes loss at `tx_reopen_done` (`TX:6704–6715`). That probe runs **before** the proposed out-of-lock read (`TX:945–955`). Both correct and mutated code therefore see the loss.

Minimal missed sequence:
```text
reopen succeeds
TX snapshots lost=false outside streams
another thread publishes loss and clears configured
TX acquires streams and restores the old configured generation
```
Wrong outcome: a lost device again has `configured=Some(g)` (`core.rs:635–639`, `TX:956–957`).

**Required:** force contention between snapshot and lock acquisition, then verify the mark stays cleared. Add corresponding coverage for Configure’s result publication.

### Minor findings / required plan corrections

- **Records are not compared fully.** TP:192 imports a trace procedure that expressly excludes records (`plan/phase8/reviews/e2a-test-plan.md:69`). C0 checks one applied-row field; C7 checks cancellation presence, not the complete ordered records/events. Add deterministic snapshots of calls **and** relevant `applied`, `bursts`, `rejected`, `switches`, and event payloads/order. Explain exclusions; unchanged existing assertions alone do not prove unchanged unasserted fields.
- **Resolve the timing recipe before implementation.** TP:162 releases `Frozen` and executes `pass` on the unregistered test thread. Reading owner records does not itself satisfy note 51 P2. Use the existing participant-based `released` helper (`provider/test_support.rs:170–185`), or `drive()` once available, for deadline-sensitive reopen work. Do not silently exclude a critical characterization case merely because its traces vary.
- **R2 is necessary, not unresolved.** The existing lock-spy test covers sends/reports, not an owner Configure or Reopen (`TX:3904–3928`). Explicitly schedule both paths.
- **Close rationale contains a factual error.** TP:112 says every booking write precedes Close; `end_open` calls `book.end_open()` **after** the device close (`TX:868–881`).

### `(d1)`, owner points, and Close

- **`(d1)`: narrowly right, broadly overstated.** Control does not mutate `held`/`open` during these calls, so there is no demonstrated concurrent-container ownership race. But Stop can arrive between claim and result, and receipt-visible record/mark interleavings are real. TP:307’s broader dismissal should be narrowed.
- **Loss mid-configure:** preserve today’s silent removal in E2b and file an issue. The discrepancy is supported by `TX:629–634` and `core.rs:1064–1068`. Do not promise a one-line fix: retaining X also changes its footprint and later shutdown reporting.
- **Failed reopen cancelling an already-sent open booking:** leave unchanged. The claimed production unreachability is plausible, but remains **INFERRED**; preserve the synthetic characterization without presenting it as a reachable user failure.
- **Close:** agree—**not an op now**. C1 needs configure-result ownership; C2 needs send outcomes; neither requires Close results. Revisit during E3’s device-state move or C6, rather than asserting C6 is necessarily the earliest consumer.
- **Design text:** no accepted-rule change is inherently needed if behavior truly remains unchanged; retain KL-38/41/42. Note 49’s Configure-refusal wording should distinguish loss from a non-lost refusal, as UR-26 already does (`design/18-uhd-radio.md:306`).

No edits, formatting, builds or probes performed. Mutation survival and counterexample executions above are inferred from the cited code, not measured.
