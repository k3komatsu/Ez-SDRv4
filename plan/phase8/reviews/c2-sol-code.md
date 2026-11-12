My independent failure scenarios, before reading the plan/review records:

- Boundary send blocked; Stop/loss arrives; time advances. The row must not apply before the send returns, and successive halts must preserve the first Stop bound and earliest loss time.
- Stop with `B ≤ b` cancels immediately while send is blocked. Later completion, another Stop/loss, or a replacement repeat must neither revive that row nor alter the replacement’s identity.
- Short-0, exception, failed reopen, shutdown, or abandon must leave no permanently pending row; `advance` must never apply an unissued/failed boundary send.
- Two successive switches must use the right predecessor and grid, including an earlier switch settling during receipt of the next.
- Mock payload cuts must match UHD’s contract except the explicitly accepted window edge; `uncertain` must remain Manifest-only.
- New Mock Stop paths must not broaden #100’s panic exposure, and concurrency tests must observe ordered events rather than depend on thread scheduling.
I found a gap outside the new `Halted` logic: `Streams::end` returns early after `Provider::stop` has ended the streams. A loss during the still-pending boundary send therefore never updates `played`. The new Stop/loss test uses a Control Stop, which does not exercise that guard.

I’m also checking the worker-panic path: it can leave an issued boundary send with no completion to settle the row.
## Verdict: FAIL

C2 makes ordinary switch outcomes follow the boundary send correctly, but two terminal paths remain incomplete.

### Major

1. **Loss after `Provider::stop` loses the playback cutoff.**  
   **VERIFIED by inspection; sequence not executed.**

   ```text
   issue boundary send at b
   Provider::stop with B > b → pending, played = MAX
   device loss at t < b
   boundary send completes whole
   ```

   `Streams::end` returns at `crates/ezsdr-radio-uhd/src/provider/core.rs:644–645` because Provider Stop already set `ended`. Consequently, the loss never reaches `stop_at` at line 649. Completion uses the stored `played = MAX` and writes **`applied` instead of `cancelled`** (`crates/ezsdr-radio/src/staging.rs:1268,1282`).

   Preserve the loss cutoff for an unresolved switch even when stream termination was already booked. Add a deterministic Provider-Stop-then-loss cell: the existing S21 test uses **Control Stop**, bypassing this guard (`crates/ezsdr-radio-uhd/src/provider/tx/tests/c2.rs:335–341`).

2. **A TX-worker panic can leave the final Manifest permanently `pending`.**  
   **VERIFIED by inspection; sequence not executed.**

   ```text
   issue boundary send
   TX worker panics before complete_send
   panic handler publishes loss with B > b
   join and cleanup write Manifest
   ```

   The panic handler reports loss but does not finish the abandoned operation (`crates/ezsdr-radio-uhd/src/provider/mod.rs:375–378`). C2 retains the switch as `InFlight` awaiting a result (`crates/ezsdr-radio/src/staging.rs:1219–1221`), but that worker has exited. Cleanup merely drains settled rows (`provider/mod.rs:410–421,745`), leaving **`pending` in the completed Run**.

   Add terminal settlement for an operation whose owner exited, with a deterministic panic-path test. Do not treat ordinary loss as proof that a still-running send cannot complete.

### Minor

3. **The accepted specs omit the immediate-cancellation exception.**  
   **VERIFIED.**

   `design/07-radio-model.md:357`, `design/18-uhd-radio.md:305`, and `plan/phase8/49-time-and-booking-inventory.md:256` promise that an unresolved boundary send remains pending until its result. Implementation deliberately cancels immediately when `B ≤ b` (`crates/ezsdr-radio/src/staging.rs:1219–1223`), explicitly tested at `crates/ezsdr-radio/tests/staging.rs:2472–2476`.

   State that exception consistently, assuming the orchestrator adopts the deviation.

### Other checks

The adopted carry-isolation tests, oracle, mutation retargets/retirements, Manifest-only `uncertain`, hardware verdict ordering, and C2-5 removal look consistent by inspection. No additional scheduling-dependent verdict or C2-expanded #100 trigger identified.

**Not checked:** executable probes, builds, mutation runs, or hardware; no files changed.
