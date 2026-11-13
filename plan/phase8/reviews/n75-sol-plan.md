# #75 — Sol's test-plan review (2026-11-13, openai/gpt-6.1-sol, read-only)

Before reading the plan, my failure scenarios are:

- A stale, duplicate, wrong-channel or late ACK confirms the wrong attempt; partial ACKs from separate attempts combine into success.
- Stop arrives during an attempt: ordinary Stop must preserve recovery and cancel only earlier bookings; Provider shutdown/loss must prevent another open or T3, including after a blocked call returns.
- Retries consume time while held bookings expire or a switch boundary passes: no premature switch application, lost reservation, continuation across an abandoned finite burst, or payload before confirmation.
- A retry clears reports stashed by the preceding attempt, or changes their generation/correlation incorrectly.
- Intermediate send failures emit rejections even though a later attempt succeeds; exhaustion emits duplicate rejections or permits another recovery next turn.
- The arm switch, when off, changes today’s single-flush acceptance/cleanup behaviour.
- A concurrency test waits outside the manual clock, or asserts “nothing happened” before the competing reader actually ran.

I’ll now compare these against the proposed tests and mutation rows.
## Verdict: FAIL

The retry direction is sound, but the plan is not implementation-ready. `plan-decisions.md` resolves several questions without updating the tests that must enforce those answers.

Paths below use **N75** = `/home/komatsu/.cache/ezsdr-fixes/2026-11-13-n75/`.

### Major findings

1. **Exhaustion explicitly expects more than the owner’s one rejection. — VERIFIED**  
   **Evidence:** `N75/test-plan.md:120,136`; owner decision `N75/options.md:215`.  
   **Sequence:** one held burst → three unconfirmed attempts.  
   **Wrong outcome:** P1b requires two `COMMAND_REJECTED` events: one `tx_reopen`, one cancellation. Additional held bursts produce additional events.  
   **Change:** assert exactly one recovery-failure event overall; retain cancellation records without extra events, or return this interpretation to the owner.

2. **The switch-off mapping can report success with its streamer already closed. — VERIFIED plan inconsistency**  
   **Evidence:** `N75/test-plan.md:62,68,109`; `N75/plan-decisions.md:4`.  
   **Sequence:** switch off → single arm flush unconfirmed → loop closes its final streamer → `open_streamer` maps `Unconfirmed` to success.  
   **Wrong outcome:** successful arm records an open set, but subsequent sends find no streamer. C11 checks the record, not actual usability.  
   **Change:** explicitly preserve today’s live streamer, events and acceptance with the switch off. Test missing ACK, zero/short send and failed send, including a subsequent usable send and the mid-run caller.

3. **The new ACK-attribution decision has no committed test sequence. — VERIFIED**  
   **Evidence:** `N75/plan-decisions.md:5–6`; incompatible expectations at `N75/test-plan.md:89–95,235–236`.  
   **Sequence:** whole T3 → stale ACK arrives first → genuine ACK absent or later.  
   **Wrong outcome:** accepting the stale ACK still passes the proposed whole-send tests; tickless-only confirmation is also untested.  
   **Change:** incorporate C5d: older ACK rejected, equal/newer timestamp boundary checked, tickless ACK unable to confirm, and stale ACK retained raw. Cover mixed channel positions and failure/loss of the newly added pre-send time read.

4. **The between-attempt loss tests deadlock at their chosen gate. — VERIFIED by lock ordering**  
   **Evidence:** `N75/test-plan.md:97–100,130`; `provider/core.rs:1055,1398–1404`; `provider/test_support.rs:471–475`.  
   **Sequence:** recovery holds `device_gate` inside `close_device` → `stop_before("tx_close")` blocks → test calls `device_lost` → loss waits for that same gate → test never releases the call.  
   **Wrong outcome:** timeout/deadlock, not an observation of retry interruption.  
   **Change:** gate between attempts **outside** `device_gate`; publish loss there before permitting another attempt.

5. **Stop coverage leaves the check-to-open/send gaps untested. — VERIFIED coverage gap; regression INFERRED**  
   **Evidence:** `N75/test-plan.md:97–100,130,132`; `provider/core.rs:1335–1365`.  
   **Sequence:** retry passes its stop check → Stop arrives before `tx_open`, or open returns after Stop → T3 starts.  
   **Wrong outcome:** another streamer/send after interruption despite passing C7’s earlier gate. Ordinary TX Stop **during** retries is also missing; P6 places it beforehand.  
   **Change:** gate before open and after open/before T3; assert interruption, claim release and no later attempt. Add ordinary Stop during recovery, with bookings on both sides of its receipt, checking cancellation order and preserved recovery.

### Minor findings

- **P1a’s successful payload assertion conflicts with its timing. — VERIFIED**  
  `N75/test-plan.md:114–115` books at +60 ms but spends 100–200 ms retrying; the confirmed flush front overtakes that target. Use a later target for P1a and keep expired-target behaviour in P3. Explicitly distinguish the rig’s independent `ManualTimeAuthority` from device-host time (`provider/test_support.rs:87,200–204`).

- **Two mutation claims are invalid as written. — VERIFIED**  
  `N75/test-plan.md:179`: N75-08 is equivalent when every non-whole send already returns before confirmation.  
  `:183`: a report gate held separately across each complete attempt can let shutdown read **between** attempts without stealing either ACK; C10 need not kill it. Specify an observable blocked-reader checkpoint across the retry boundary, rather than assuming ACK theft.

- **Record-boundary coverage remains too weak. — VERIFIED**  
  `N75/test-plan.md:128` permits control-equality instead of proving the switch boundary was crossed. Pin pending → applied/cancelled explicitly during the longer recovery. Also add receipt-side reservation checks around an abandoned finite chain and Stop, using `tx/tests/n99.rs:108–135,210` as the existing pattern; held-key checks alone do not establish those record rules.

No edits, builds, mutation runs or probes performed. Hardware/source claims quoted from Fable were not independently reverified.
