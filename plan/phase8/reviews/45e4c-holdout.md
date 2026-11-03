1. **N01 — Procedure 1: origin equality rejected**
   - **Wrong behaviour:** Require the value’s instant to be strictly after the segment origin, rather than at or after it, while leaving all other reservation conditions unchanged.
   - **Minimal sequence:** Arm a segment with origin `O` and cut `C > O`; present a released value at `e = O`, with matching channel and generation and progress below `C`. Repeated reservation attempts reject it despite available capacity.
   - **Observable wrong outcome:** An eligible origin-time update remains held and blocks later updates. **Progress for eligible reservations** is broken.

2. **N02 — Stopping transition: maybe-applied work omitted**
   - **Wrong behaviour:** When atomically entering Stopping, record remaining drain obligations only for commands in Taken, omitting commands in Maybe-applied.
   - **Minimal sequence:** Reserve and take a value; begin its device call, entering Maybe-applied; delay its return. RX enters Stopping before its drain deadline. Because its captured obligation set is empty, RX stops without draining or reporting that value.
   - **Observable wrong outcome:** A command already executing is absent from the stop procedure’s accounting. **Every non-cancellable handover remains a drain obligation** is broken.

3. **N03 — Procedure 2: equality omitted from cut cancellation**
   - **Wrong behaviour:** On shortening a cut to `C`, cancel queued values only when `e > C`, rather than when `e ≥ C`.
   - **Minimal sequence:** Queue a value at `e = 60` under cut 100. Stop shortens the cut to 60 while progress remains below 60. Before RX enters Stopping, settings takes the still-queued value and applies it.
   - **Observable wrong outcome:** A queued value outside the shortened half-open segment reaches the device. **Cut changes revoke all queued values no longer covered** is broken.

4. **N04 — Procedure 3: cut-only changes escape commit validation**
   - **Wrong behaviour:** Validate only the epoch, not the cut, when committing publication calculated outside the cell.
   - **Minimal sequence:** Under epoch `v`, copy cut 100 and calculate publication of `[50,90)`. Stop shortens the cut to 60 without changing `v`. Commit the calculated publication because the epoch still matches.
   - **Observable wrong outcome:** Samples `[60,90)` escape after the cut was shortened. **Publication respects the cut current at commitment** is broken.

5. **N05 — Procedure 4: native loss publication follows recording**
   - **Wrong behaviour:** Set the instance failure flag, then publish the loss record or event, and only afterward call native `mark_lost`.
   - **Minimal sequence:** Detect loss; set the flag; publish `DEVICE_LOST`; pause before native marking. The event recipient initiates stop and cleanup; executors have already exited, so native destruction runs before `mark_lost`.
   - **Observable wrong outcome:** Teardown can free an unreachable device despite the published loss. **Native retention is established before observable loss publication** is broken.

6. **N06 — Procedure 4: lifecycle admission checked before the gate**
   - **Wrong behaviour:** For a rate read, check the failure flag before waiting for the lifecycle gate, but do not check it again after acquiring that gate.
   - **Minimal sequence:** A rate reader sees a clear flag and waits for the gate. Another participant acquires the gate, publishes loss and calls `mark_lost`, then releases it. The reader acquires the gate and calls the device.
   - **Observable wrong outcome:** A lifecycle operation is admitted after loss, outside the permitted check-to-call window protected by that gate. **Lifecycle calls serialize with loss admission** is broken.

7. **N07 — Procedure 4: retain streamers but destroy their parent**
   - **Wrong behaviour:** After native loss has been marked, retain streamer handles but still free the parent native device handle during destruction.
   - **Minimal sequence:** Open a streamer; detect and fully publish loss; let all executors terminate; destroy the provider’s native ownership.
   - **Observable wrong outcome:** Parent-device teardown runs over a dead link and can abort the process before the Manifest is returned. **Loss-safe retention includes the native device and its streamers** is broken.

8. **N08 — Pinning: one completion releases another command’s pin**
   - **Wrong behaviour:** Treat any Finished command on a chain as sufficient to clear that chain’s pin, without considering its other Taken or Maybe-applied commands.
   - **Minimal sequence:** Command A finishes, but its completion input is delayed. Settings takes command B on the same chain and pauses before calling. Deliver A’s completion; request channel move or next-segment start.
   - **Observable wrong outcome:** Conflicting lifecycle work is issued while B remains non-cancellable. **Pinning lasts until every outstanding pinning command finishes** is broken.

9. **N09 — Settlement: late completion records an already-settled command again**
   - **Wrong behaviour:** On receiving a late completion, append the command’s ordinary result record even if stop has already finalized that command’s externally visible disposition.
   - **Minimal sequence:** Take a value due after stop instant `S`; delay completion. Provider stop records its cancellation at `S`. The device call later succeeds and its completion reaches the brain.
   - **Observable wrong outcome:** The same command receives both its finalized end disposition and a second ordinary result row. **Exactly-once record settlement, independent of residual obligations** is broken.

10. **N10 — Drain deadline: empty receives renew the allowance**
    - **Wrong behaviour:** After each empty or timed-out drain receive, recompute the hard deadline using the current time plus the delivery allowance instead of retaining the original deadline.
    - **Minimal sequence:** Reach the cut with an unconsumed value; keep the native lock available; let successive bounded receive calls return no qualifying samples. Each return pushes the deadline forward.
    - **Observable wrong outcome:** RX never finishes or reports the residual despite continued scheduling. **Drain has a fixed, bounded deadline when native operations can proceed** is broken.

11. **N11 — Native lock: command-time bracket split into separate sections**
    - **Wrong behaviour:** Release the native control lock after `set_command_time`, then reacquire it separately for the setting and for `clear_command_time`.
    - **Minimal sequence:** Settings installs a future command time and releases the lock. RX acquires it and issues the intended untimed stop. Settings subsequently applies its value and clears command time.
    - **Observable wrong outcome:** The stop can inherit the setting command’s future time rather than being untimed. **The command-time bracket is indivisible with respect to other control calls** is broken.

12. **N12 — Bound K: capacity counted per chain**
    - **Wrong behaviour:** Apply K independently to each chain instead of counting reservations across the entire device.
    - **Minimal sequence:** With device bound `K = 1`, reserve one value on chain A and leave it queued or taken. Reserve another value on chain B before A releases device capacity.
    - **Observable wrong outcome:** Two reservations occupy a device whose declared reservation bound is one. **Device-wide capacity conservation** is broken.

13. **N13 — Bound K: partial failure releases future capacity**
    - **Wrong behaviour:** Count finished successful commands until their instant, but immediately exclude finished partial failures even when a device effect remains possible.
    - **Minimal sequence:** With `K = 1`, apply a future timed value; its setting is accepted, but command-time cleanup fails. Before the value’s instant, process that partial failure and reserve another value.
    - **Observable wrong outcome:** The second reservation is admitted while the first may still occupy device capacity. **Potentially effective partial failures retain K until their instant** is broken.

14. **N14 — Bound K: time expiry releases unfinished work**
    - **Wrong behaviour:** Stop counting every command once device time reaches its instant, including Queued, Taken and Maybe-applied commands.
    - **Minimal sequence:** With `K = 1`, take a value and delay its call’s completion beyond its instant. Deliver a device-time update past that instant, then present another eligible value.
    - **Observable wrong outcome:** A second reservation enters while the first unfinished command still requires a capacity reservation. **Unfinished commands retain K regardless of elapsed device time** is broken.

15. **N15 — Bound D: reported residuals excluded**
    - **Wrong behaviour:** Exclude reported `left` values from the chain’s D occupancy while retaining their residual obligations.
    - **Minimal sequence:** With `D = 1`, finish a chain with one reported, unconsumed `left` value whose completion has arrived. Begin a subsequent segment on that chain and reserve another value without consuming the old one.
    - **Observable wrong outcome:** The cell contains two retained values under a bound of one. **D bounds all retained cell values, including reported residuals** is broken.

16. **N16 — Bound D: cancellation removes the entry before Dropped**
    - **Wrong behaviour:** Remove a cancelled queued value from the cell immediately, rather than retaining it until the brain receives the executor’s Dropped input.
    - **Minimal sequence:** Reserve a queued value; shorten the cut so it is cancelled; delay the settings executor’s observation of cancellation. The brain removes the entry immediately, then later receives Dropped for that reservation.
    - **Observable wrong outcome:** The reservation disappears before its terminal input, leaving that input without its retained command identity and state. **Cell retirement requires both terminal-input receipt and absence of residual obligations** is broken.

17. **N17 — Held queue: capacity timer executes drain-expiry behaviour**
    - **Wrong behaviour:** Handle a capacity timer through the drain-deadline transition, rather than merely reevaluating current capacity and held work.
    - **Minimal sequence:** Fill K with a finished future value and hold another update. Schedule the capacity timer for the first value’s instant. While the segment is still streaming well before its cut, deliver that timer.
    - **Observable wrong outcome:** RX enters stopping or emits a stop command solely because capacity became available. **Capacity wakeups cannot terminate or restart a stream** is broken.

18. **N18 — Consumption: pre-return samples credited retrospectively**
    - **Wrong behaviour:** Retain samples observed before a setting call returns and later use them as consumption evidence once its completion becomes known, provided their timestamps satisfy the thresholds.
    - **Minimal sequence:** The device accepts a value at time `a`, but the setting call remains outstanding. RX observes a same-channel sample timestamped at or after both `a` and `e`. The call then returns; provide no further samples and stop.
    - **Observable wrong outcome:** The value is declared consumed without any qualifying sample observation after return, so its residual is omitted. **Consumption requires post-return observation, not timestamp eligibility alone** is broken.

19. **N19 — Lifecycle: join wakeups renew the timeout**
    - **Wrong behaviour:** Restart a thread’s one-second join allowance after each notification or unsuccessful join poll instead of measuring it from the beginning of that thread’s join.
    - **Minimal sequence:** Request provider stop with one executor permanently blocked. Other participants continue sending notifications more frequently than once per second. Repeatedly retry that executor’s join.
    - **Observable wrong outcome:** The provider never reaches timeout, detach and handle-retention handling. **Each join has a fixed upper bound independent of unrelated wakeups** is broken.

20. **N20 — Lifecycle: spawn failure waits for an absent executor**
    - **Wrong behaviour:** Include an executor whose creation failed in the set required to acknowledge the new epoch or complete shutdown commands.
    - **Minimal sequence:** Successfully issue a receive stream’s initial start; fail creation of its executor. Enter Abort stop, abandon that unowned chain, and wait for every declared executor’s quiescence acknowledgement, including the failed one.
    - **Observable wrong outcome:** Start-failure rollback cannot complete because the missing executor can never acknowledge. **Rollback waits only for executors actually created** is broken.

Count by area: Procedure 1—1; Stopping—1; Procedure 2—1; Procedure 3—1; Procedure 4—3; pinning—1; settlement—1; drain deadline—1; native lock—1; K—3; D—2; held queue/timers—1; consumption—1; lifecycle—2.
