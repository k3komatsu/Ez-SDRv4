
> plan · gpt-6.1-sol
What this change does: Removes the unused DSP gates and channel helper, and folds failed-reopen recovery into `Clean`. The report gate and clock-ending behavior remain.

### Failure scenarios checked
- Shutdown steals T3’s ACK during initial open or reopen.
- Payload overlaps the flush, or reports cross streamer generations.
- Failed reopen permits held or newly arriving payload, or changes cancellation counts.
- The RX replacement selects a different channel.
- Deleted mutation rows remove protection for retained behavior.

### Findings

1. **P2 — Reader-exclusion test lacks a reliable behavioral checkpoint**  
   `crates/ezsdr-radio-uhd/src/provider/core.rs:1539–1588`
   - N-09/N-10 die at `is_held`, not at the ACK-ownership assertions. The opener also discards `TxFlushResult`, so successful acknowledgement is never asserted.
   - Counterexample: remove the opener’s report lock; shutdown briefly increments `waiters()` while acquiring the now-free lock; the test releases the opener before shutdown reaches `tx_async`. Opener wins the race and the behavioral assertions pass.
   - The report’s explanation is incorrect: `test_support.rs:386–388` explicitly permits overlapping calls with the **same** name; it does not serialize two `tx_async` calls. `host.rs:535–539` explains the transient waiter count.
   - **Fix:** establish a deterministic checkpoint distinguishing genuinely queued lock acquisition from entry into the competing device read. On both open paths, return the flush result and assert:
     ```text
     flush.acknowledged == true
     flush.stopped == false
     shutdown_report == None
     exactly one channel-0 BurstAck recorded with tx_flush == true
     no missing-ACK rejection
     ```
     Demonstrate that removing either report gate fails a behavioral assertion, independently of `is_held`.

2. **P2 — Failed-reopen test never submits the promised new payload**  
   `crates/ezsdr-radio-uhd/src/provider/tx.rs:1529–1544`
   - The added second `pass()` checks only the already-cancelled B. It never calls `command(TxCmd::Burst(...))` after failure.
   - The mandatory A5′ sequence is: failed reopen → receive another burst on the ended clock → cancel it without sending. That last transition, reservation release, and cancellation count are not observed.
   - **Fix:** extend this existing branch with a distinct later burst and reservation, deliver it through `command`, then run another turn. Assert no payload send, released reservation, clock still absent, and exactly the expected additional cancellation report/event. No separate fixture is needed.

3. **P3 — Current bench instructions misdescribe the renamed measurement**  
   `crates/ezsdr-radio-uhd/tests/hardware.rs:3083–3095`; `plan/phase8/bench.md:549`
   - The output now correctly distinguishes `tune_auto` command-time latency from setter-plus-read-back latency, but the current bench instructions still describe all rows identically.
   - Update that paragraph with the same exception in this step. Preserve its hardware safety instructions.

### Other conclusions
- No production regression found in the retained report gates, generation handling, flush deadline/stash, or payload ordering.
- `Clean` does not revive transmission: failed reopen removes the clock; subsequent burst commands follow cancellation.
- RX still selects exactly `{1}`.
- P1175–P1177/P1307 removal is justified; P1178 is replaced by N-09. P1310 and N-04 are equivalent on reachable paths. N-11 protects clearing pending recovery.
- The UR-22 amendment is consistent. No S2b behavior was implemented early.
- Leave the broader plan-cursor cleanup to B4; it is not introduced by this diff.

**Verdict: CODE NEEDS CHANGES — fix 1 and 2 before acceptance.**

Not checked: I ran no builds, tests, mutations, or hardware operations; probe results above come from the supplied logs. No files changed.
