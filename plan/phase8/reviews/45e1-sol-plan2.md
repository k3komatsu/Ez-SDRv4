> plan · gpt-6.1-sol
Most earlier fixes are present. I found remaining gaps in horizon-end timeouts, loss/command accounting, and how the harness observes deadline progress and gate contention. These affect whether a correct run can pass—and whether an incorrect one can escape detection.
**The revision substantially improves the plan, but event granularity and cases are not yet approved for implementation.** Five remaining issues can produce false passes or false failures.

References: **EP** = `45-experiment.md`; **TP** = `test-plan.md`, both in `~/.cache/ezsdr-fixes/2026-11-01-n45/`.

## Findings 1–11: closure

| Previous finding | Assessment |
|---|---|
| **1. Event granularity** | **Mostly closed.** Per-sample ticks, receive entry/return, separate results/delivery and brain ops expose the missing races. Clarify result-dependent `B.op` execution below. |
| **2. Lifecycle admission** | **Mostly closed.** `G`, its inner recheck and I14 are right. Loss publication’s lock scope and case-8 witness still need correction. |
| **3. Consumption evidence** | **Closed.** Device application, qualifying receive evidence—including unpublished draining—and publication are correctly separated. |
| **4. Once-only accounting** | **Partly closed.** Disposition/result/leftover-consumption separation fixes U11, but commands abandoned on cancellation, loss or panic remain undefined. |
| **5. Scheduler/deadlines/cycles** | **Partly closed.** Input windows and observation-relative P1 are appropriate. Horizon handling and P1’s observation remain defective. The explicitly stronger no-cycle property is acceptable once synthetic timeout loops are removed. |
| **6. Past-bug scripts** | **Closed in substance.** Cases 7(a–c) now describe reachable intended paths. For 7(c), “no streamer call for the refused segment” must mean **no further call after the failed begin**; the failed start itself necessarily calls the streamer. |
| **7. Missing concurrency cases** | **Closed as coverage.** Cases 3, 5, 8 and 9 add the competing requests and partial/refused outcomes. Restore eligibility still needs repair below. |
| **8. Test/mutation pairings** | **Closed at plan level.** Producer coverage, edge-once DFS, stopped-tail semantics and adapter-test qualification are corrected. Actual anchors/kills remain implementation evidence. |
| **9. Snapshot merge** | **Closed at plan level.** Revision validation plus identity-based consumption is suitable. Increment the revision on every obligation change, not only completion. |
| **10. Search identity** | **Mostly closed.** Exact keys and finite monitor summaries are appropriate, subject to the observation requirements below. |
| **11. Budget estimates** | **Closed as process.** Measurement-first and stop-and-report checkpoints replace unsupported feasibility assumptions. |

## Must fix

### R1. Horizon-end timers invent early deadlines and zero-time timeout loops

**Evidence:** EP:59,66,71.

**Sequence:** at horizon 48, a drain timer due at 56 is pending → the horizon rule delivers it immediately → the core ends draining at 48. This explores behaviour that cannot occur under the stated deadline.

Likewise: recv times out at the horizon → R retries recv → its new timeout is immediately enabled again. Correct receive-loop behaviour becomes a synthetic livelock, rejected by P5—or an endless sequence if timer identities keep changing.

**Change:** do not fire future deadlines early. Specify a bounded shutdown/draining epilogue with real time advancement. An unresolved bound is **incomplete exploration**, not successful shutdown or proof of a protocol bug. Add tests for both sequences.

### R2. Cancellation/loss/panic cannot satisfy the command-result oracle as written

**Evidence:** EP:48–54,100,122; TP:63–65.

**Sequence:** a settings command is queued → Stop cancels it → `S.take` produces `Done{Dropped}` without a call. P2 nevertheless requires a command result, while TP’s `CommandResult` has no `Dropped` outcome.

A second path is queued work discarded by `S.check` after loss, especially after brain panic: no completion reaches a functioning brain, but P2 still requires every command’s result.

**Observable outcome:** a correct cancellation/shutdown fails P2, or the implementer silently exempts commands and hides genuinely missing outcomes.

**Change:** distinguish **call results for calls actually entered** from **terminal command outcomes**, including cancellation and abandonment. Define who records abandoned commands after panic, without relying on the dead brain. Keep value dispositions separate.

### R3. Terminal-only checking cannot establish P1’s “without waiting for pins”

**Evidence:** EP:90,119–121; TP:423–424,528.

**Sequence:** deadline 20 is processed while S holds `L` and pins v → faulty R continues waiting for v → S finishes at 40 → R stops → shutdown completes.

**Observable outcome:** terminal checks pass, although R incorrectly waited for the pin. The native-lock wait masks the error.

**Change:** observe the logical **drain-ended/stop-pending transition while the pin still exists**, separately from native stop completion. Case 5 must witness and assert that transition after deadline processing; mutation-test waiting for pin clearance instead.

### R4. Restore eligibility omits outstanding device values

**Evidence:** EP:90,94,106,116; `plan/phase8/44-device-chain-state.md:128`.

**Sequence:** in case 5, Tick 40 frees the radio queue → S enqueues v → its call completes and clears the pin → R stops before the next sample consumes v → Restore executes because the chain is unpinned and `Off` → the next segment’s first sample consumes v and overwrites the restored setting.

**Observable outcome:** restore appears successful but a previously queued timed value subsequently defeats it.

**Change:** retain the existing channel-wide prohibition on restore while relevant debt/`left` remains, including across streams after a move. “Issued once after the pin clears” must also respect that eligibility. Extend case 5 or 9 through subsequent sample consumption; pin clearance alone is insufficient.

### R5. Loss publication’s lock ordering is not fully specified

**Evidence:** EP:62,67,93.

**Sequence permitted by the current wording:** S observes a lost result while holding `L` → R holds `G` waiting for `L` → S attempts `LossPublish`, waiting for `G`.

**Observable outcome:** deadlock, despite EP’s claim that no wait cycle exists.

**Change:** explicitly require native-call return and release of `L` **before acquiring `G` to publish loss**. Lifecycle callers already holding `G` need a defined non-recursive publication path after native return. Test both paths.

Also, atomic `LossPublish` acquires and releases `G` within one event, so case 8 cannot witness a persistent state where it holds `G` while R waits. Split gate acquisition/publication/release, or replace that witness with an explicitly observable contention/order witness.

## Other required clarifications

- **`B.op`:** cell-operation results must control subsequent ops. A failed reservation or changed pin query cannot leave an unconditional send precomputed by `B.step`.
- **Receive recomputation:** preserve the original recv’s handle/epoch/sample identity when refreshing cut and obligation snapshots. Recomputing must not relabel old samples as the new epoch.
- **Finite monitor:** retain enough identity—value, channel, segment/epoch and matching sample—to validate application/evidence correspondence. Validate event violations **before visited-state pruning**, or retain a sticky violation fact in the key.
- **Tick boundary:** define which sample the initial tick produces so origin sample 0 is neither omitted nor produced twice.

## Budget expectations

**Case 3 and the full case-6 panic sweep are the clearest likely overruns.** Cases **5 and 9** are also substantial risks; blocking one actor does not eliminate interleavings among time, receive, brain ops, timers and requests. These are estimates, not measurements.

The seeded configuration has its separate **30-minute/10,000-seed** budget, not the exhaustive 10-minute budget.

On an overrun, report the exact configuration, states, transitions, elapsed time, memory, reached/missing witnesses and incomplete frontier; for case 6, identify the affected `k`. Preserve any counterexample. Do **not** drop cases, narrow windows, merge events, weaken witnesses, add unreviewed reduction, or report partial exploration as passing.

No files changed; no execution or hardware checks performed. The prohibited holdout directory was not read.

**VERDICT: REVISE.**
