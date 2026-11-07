### Independent scenarios

- Overlapping, disjoint and adjacent bookings in both receipt orders, including a candidate that starts earlier but spans an existing booking.
- Standalone short bursts versus short bursts continuing an existing device burst.
- A contiguous booking received after another reservation has caused an early close.
- Booking-time and dispatch-time SendAsap moves into another reservation.
- Stop/rebook sequences, repeated Stops, sample-grid rounding and per-stream isolation.
- Repeat switches at the exclusive handed cursor, across tiled chunks and with different lengths.
- Second switches and cancellation before hand-over versus after samples have already been handed.

### P2 findings

1. **A future reservation can invalidate a later contiguous booking without host lateness.**  
   **TP §3.1 S4, §8 5b–5c; `crates/ezsdr-radio-uhd/src/provider/tx.rs:442–465`.**  
   Sequence: book long A and nonadjacent C early; let UHD finish handing A. Because C is next, `step` immediately closes A’s held tail. Then, still comfortably before A’s end minus device lead, book B exactly at A’s end. The planned receipt rule accepts B as a continuation, but UHD’s closed front is already `end+2`, so dispatch drops/moves B.  
   Add this three-booking test and specify how the owner preserves the continuation opportunity or how receipt admission knows it has ended.

2. **The standalone short-burst interval is unsafe for a short continuation.**  
   **TP §1.1–1.2, Q9; `tx.rs:601–610,773–785`; `crates/ezsdr-acceptance/tests/parity.rs:1651–1652`.**  
   Sequence: book A of 40 samples, B of 10 exactly after A, then C at B’s end+1. The table admits C using B’s standalone `end+1` front. But B continues A’s open device burst; its held-tail close requires `end+2`. C is therefore late at dispatch without any stall.  
   Resolve chain framing in the shared rule and retain this existing parity case with its actual assertions—not merely replace D5 numerically.

3. **Dispatch-time SendAsap leaves the allegedly unreachable preemption path reachable.**  
   **TP §1.4, §8 5b–5c; `tx.rs:968–996,433–440`.**  
   Sequence: accept A for `[20,40)` ms and B at 50 ms. Delay A’s first dispatch until 40 ms; SendAsap moves A to `[42,62)` ms. The moved start is not B’s identical start, so today’s dispatch check permits it. A then reaches B’s start and invokes preemption.  
   Define and test the full-interval collision outcome after a dispatch-time move before deleting preemption. The existing single-burst late-policy test does not cover this.

4. **The switch’s exact-cursor test reverses the cursor’s meaning.**  
   **TP:138–141,208; `tx.rs:2276–2280`.**  
   The cursor is the **exclusive first-unhanded sample**. If `cursor == boundary`, the boundary sample has not been handed, so installation can still succeed. TP instead requires refusal and plans a mutation enforcing that wrong result.  
   Test `cursor < boundary`, equality, and `cursor > boundary`; equality should preserve the exact switch.

5. **The two Providers give pending switches different lifetimes.**  
   **TP:178,304–305; §3.3 UHD table.**  
   Sequence: accept switch 1 at boundary b; advance the owner until it hands new-waveform samples at b while device time remains before b. Submit switch 2. TP clears UHD’s pending switch at hand-over, but Mock’s only when time passes b: UHD can accept while Mock refuses. Different lengths also make the predecessor and next boundary ambiguous.  
   Specify one shared logical pending/predecessor rule and test this ahead-of-device-time sequence on both Providers. Also fix the proposed Stop-at-b test: already handed new samples cannot be recalled. Add explicit UHD Stop/loss/shutdown/F8 cancellation tests before hand-over; the listed cancellation tests and rows currently target Mock only.

6. **The operative plan still implements two explicitly rejected designs.**  
   **TP:160–180,211–212,319; binding rulings Q6/Q8.**  
   TP adds radio-specific Kernel metadata admission and Python client-side playing-state checks. Kernel admission already checks ASCII keys, not emptiness (`crates/ezsdr-kernel/src/coordinator/admission.rs:85–87`); Python already raises from Provider refusals (`python/ezsdr/session.py:114–121`).  
   Remove these implementation/test/mutation proposals. Test metadata validity in both Providers and Python rejection through the submit reply. Include the Vocabulary verb declaration and the stale `event.rs:699–700` comment in the inventory.

7. **Root ticks and sample offsets are not reconciled in the shared representation.**  
   **TP:17–24,30–36,118.**  
   Intervals are declared to be in root ticks, but fronts add 1/2/3/5 **samples**, and the boundary helper combines sample periods with root-time receipt/window values without a specified conversion.  
   Counterexample: at 1 MS/s on UHD’s 200 MHz root, five samples are 1,000 root ticks, not five. Require explicit coordinates and rational conversions, with non-unit grids, nonzero origins, fractional root instants and exact-boundary tests.

8. **5a cannot both preserve staging behaviour and use `accept_from` as the busy end.**  
   **TP:24,150–152,278,314; `crates/ezsdr-radio/tests/staging.rs:158–163`.**  
   Sequence: accept a finite burst ending at e; submit a value at e. Stage 4 accepts it. TP’s `stageable = max(accept_from) <= receipt` refuses it until the close gap expires. Thus “same answer” and “no behaviour change” are false.  
   Keep payload ends and safe booking fronts distinguishable within the shared representation, or obtain an explicit semantic ruling and update all affected tests/specs. Likewise, existing #79 parity cannot become equal merely through a representation-only 5a.

9. **The removal inventory includes live, unrelated coverage.**  
   **TP:216,242,264.**  
   `N47S4-03…N47S4-53` is not a preemption family. Examples:
   - `N47S4-03`: staging while repeating (`plan/maintenance/tools/mutations.json:13803`).
   - `N47S4-39`: clock ending at its origin (`:13853`).
   - `N47S4-53`: Python channel targeting (`:14333`).
   
   Removing these with their tests loses reachable behaviour. Finish the per-row/per-assertion disposition before approving deletion; broad name/range searches are not sufficient evidence.

### P3 findings

10. **Several tests and mutations do not discriminate the claimed failure.**  
    **TP §3.4, §4, §8.**  
    - N47S5-01’s named existing killer only formats refusal reasons; it never calls `stageable` (`staging.rs` tests:227–231).
    - N47S5-07 changes `<` to `<=`: it catches rejection at the boundary, not acceptance of interior overlaps. S3 needs a mutation that removes/we akens overlap refusal.
    - S13 does not spell out #79’s actual one-sample A / refused B / later-value sequence; “inside the busy span” can pass while that divergence remains.
    - UHD pending-check/cancellation mutations are missing; Mock mutations cannot validate UHD wiring.
    - 5b changes user-visible limits but defers KL-36, contrary to the same-commit rule.
    
    Repair these mappings and give each sub-step its exact existing-test rewrites, especially the preemption tests that otherwise fail before 5c.

**No P1 finding. Approval requires the P2 sequences and their observable outcomes to be resolved first.**

Not checked: builds, tests, mutations or new bench measurements; none were run. No files were changed.
