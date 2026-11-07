
<!-- opus-plan-recheck.md -->
# Note 47 stage 5: re-check of test plan rev 2 (Opus, read-only, HEAD c41530f)

**Verdict: GO WITH LISTED CHANGES.** The record design holds, but four majors need to land first. M1 and M2 change the pure rule itself, so they must be in rev 3 before 5a starts. M3 and M4 change the sub-steps. A narrow re-check of rev 3 (M1–M4 only) is enough; it does not need another Sol pass. I built nothing and ran nothing; all line numbers below are from my own reading at c41530f.

## Sol 1–10

| Sol | Status | Note |
|---|---|---|
| 1 held-tail close | **not closed** | M1: the close has no margin, `continuation_open` is mis-defined, and receipt races the close |
| 2 chain front for a short continuation | closed | the §1.1 join gives `end+2`; S4c and parity 1607/1651 keep their real assertions |
| 3 dispatch-time SendAsap | **not closed** | M4: `move_start` tests only "the other chains", so the head's own joined pieces are skipped |
| 4 exclusive cursor | closed in the tests; **broken by 5c** | M3: the `=` case runs through the `h <= next` path that 5c deletes |
| 5 one pending rule | closed, with m3 and m4 | |
| 6 no Kernel or client state | closed | `admission.rs:85-87` checks only ASCII keys; `session.py:114-121` raises from `refused` (VERIFIED) |
| 7 coordinates | closed, with m1 | |
| 8 payload ends kept apart from fronts | closed, with m2 | |
| 9 per-row inventory | closed | the deferred reads (tx.rs:4884, :5258, iii.rs) are named with their sub-step; acceptable |
| 10 P3 items | closed except S13 | m5 |

## Majors

**M1 — Sol 1's close instant has zero margin; a booking at the chain front is late on UHD (VERIFIED in the code; the outcome is INFERRED, not run).**
The ruling says a later held burst "stays at least a device lead after the close, because R1 refuses it inside the chain's front". That is false: the front is `e+2` samples (§1.1), not `e + lead`. Under the plan (§3), the owner closes only once `e < earliest(now)` (`ordinary_k`, about `instant(e) − 2 ms`). A step that has nothing to do sleeps `IDLE` = 200 µs (`tx.rs:286-288`, `:218`), so the close lands somewhere in the 200 µs after the deadline. C's start is then judged by `dispatch_start` with a 2 ms lead (`tx.rs:968-976`, `burst.rs:110-111`).
Sequence, x3xx at 1 MS/s:
1. Book A (40 samples) at `s`, early.
2. Book C (40 samples) at `s+42`, early. The record accepts it, since A's front is `s+42`.
3. A is fully handed and its tail is held. The owner waits, because C is not a continuation.
4. The step at `now = instant(s+40) − 2 ms + 150 µs` closes the tail.
5. The next step runs `dispatch_start(C)`. Here `target − now_k` is 1 852 samples, under the 2 000-sample lead, so C is late by 148 µs: a `TIME_ERROR` and a move to `s+190` (or a Drop).

The Mock plays C at `s+42`, on time. Today's code (`tx.rs:457-460`: close as soon as C is held) never produces this; the plan introduces it. Two further defects:
- (b) `continuation_open(end, earliest)` is false once B has joined: "a chain ends at `end`" no longer holds, because the chain now ends at B's end. In the window after control has written B's join to the record but before the owner drains B's `TxCmd` (`tx.rs:276-287`), the owner reaches the `None` branch and closes. B then goes late at `e` against the front `e+2`.
- (c) "Receipt exactly at the continuation deadline" decides the same instant in two threads, so the answer depends on the race, not the rule.

Fix, one rule in the record:
- A join at `e` is accepted only when the receipt is at or before `J(e) = instant(e) − (lead + delivery allowance)`. That is 5 ms on x3xx. Both Providers already hold the value: UHD has `DEVICE_LEAD_NS + DELIVERY_ALLOWANCE_NS` (`profile.rs:11-21`), and the Mock's x3xx has 2 ms + 3 ms (`mock profile.rs:47,143-146`).
- The owner closes the held tail at the first step at or after `J(e)`, and caps its idle sleep at `J(e) − now`.
- The owner reads the record under the `streams` lock it already takes. If a piece is joined at `e`, it never closes there; it waits for that piece's `Held`.
- `continuation_open` becomes "a piece is joined at `e`, or the chain is joinable and `now ≤ J(e)`". `first_free`'s join exception uses the same test.

C at `e+2` then has 3 ms of slack, and a stall beyond that is host lateness by the profile's own declaration. Tests:
- the sequence above with the manual clock stepped 150 µs past the deadline;
- B's record join before its `TxCmd` is drained;
- receipt at `J(e)` and one tick after it.

**M2 — Stop's `cancel_from(t)` drops pieces the device has already been handed (VERIFIED: the plan text plus the code).**
§1.2 drops every piece that starts at or after `t`, and `book` (§1.3) never consults `self.stop`. But a piece starting in `[t, at_or_after(t+W))` is already dispatched and handed when the Stop arrives:
- UHD: `step` dispatches when `k < window_cut` (`tx.rs:471`); the Stop then reaches `stop_now` → `end_open(true)` (`tx.rs:1087`, `:773-790`), which closes at the cursor with front `cursor+2` or `+3`.
- Mock: `stop_tx` plays every held burst that starts before the horizon (`lib.rs:1162-1172`, `handed_end`, `:300`).

Sequence, 1 MS/s, W = 10 ms:
1. Book D (1000 samples) at `s`, early.
2. Stop received at `t = s − 5 ms`. D is fully handed and its tail held, so the Module's front is `s+1002`. The record drops D.
3. Book E (100 samples) at `s+500`, received at `t + 1 ms`. The record accepts it.
4. UHD: `later_bound` with front `s+1002` makes E late (`TIME_ERROR`, moved or dropped). The Mock plays E on top of D's played samples.

The same defect shows when the dropped piece is a joined B that was held at A's end: A then has no tail, the Module closes at `e` with front `e+3` (`tx.rs:784-786`), and a recomputed chain front is `e+2`.
Fix: a Stop that cuts or cancels anything adds one region `[at_or_after(t), stop_k)` to the record, a chain with `join = false`. Remaining fronts are never recomputed downward after a drop. §0 already calls `stop_k` "the first sample a booking may start at after a Stop", so this only makes §1.3 agree with §0. Test: the sequence above, on both Providers.

**M3 — 5c deletes the continuation path that 5d-U's `cursor = boundary` install needs (VERIFIED).**
At `cursor == b`, the switch's `Held` at `b` has `h == next`. Today that goes to `preempt` (`tx.rs:433-436`), whose `held.k == next` branch continues the device burst (`tx.rs:1064-1066`). C2 deletes the branch. `send` then computes `wanted = min(…, b − next) = 0`, which trips `debug_assert!(count > 0)` (`tx.rs:579-582`). So `ur_21_s8_switch_at_the_exact_handed_cursor_installs` fails, and 5d-U is red.
Fix: 5c keeps a reduced `h == next` branch (`end_open(false)` and continue). It deletes only the `h < next` re-decision and the `k < c` preempt.

**M4 — `move_start` skips the moved head's own chain (VERIFIED in the plan text; the code effect is VERIFIED).**
Sequence:
1. A (40 samples) at `s`; B (10 samples) joins at `s+40`.
2. A host stall makes A's dispatch late with SendAsap, and it moves to `s+5`. The test runs against "the other chains" only, so the move is accepted.
3. B is held at `s+40`, inside A's moved span. After 5c there is no `h <= next` branch. In `send`, `cut − next` is negative, `count` is negative, and the slice index panics (`tx.rs:579-590`).

Fix: test the moved region against every piece except the head itself, and refuse on a hit (Sol 3's `TIME_ERROR{refused}`). Add this sequence to `ur_23_s3_…`.

## Minors

- **m1 (VERIFIED).** `ordinary_k = at_or_after(decided + lead)` (Mock, `lib.rs:1642`) differs from the on-time test `decide`, which uses `start ≥ at_or_after(instant(at_or_after(decided)) + lead)` (`burst.rs:110-111`, `control.rs:452-453,473`). They differ by one sample when `lead/n` is not an integer. Define `ordinary_k` as `decide`'s threshold, and test at `n = 3`.
- **m2.** The value rule reads only the maximum end, so `dropped_ends: Vec` is today's `until` scalar; keep a scalar. Q-5's pruning belongs in 5b-2, not 5f: drop chains whose front is at or before `ordinary_k(now)`. Without pruning, `book` is O(bookings) on the control path.
- **m3.** `advance` must run inside every record call, at the receipt's `now_k` (`switch`, `stop`, `book`). Otherwise UHD control can still refuse a second switch after `b` until the owner's next step, while the Mock accepts it.
- **m4 (VERIFIED).** The UHD cursor can lag `window_cut` by up to one chunk, W/4 (`tx.rs:567,580-581`). After a Stop with `b < stop_k` but `cursor < b`, `stop_now` cancels the held new repeat, so nothing of it plays. The outcome must not claim new samples. `ur_23_stop_after_handover_…` must put the cursor at or past `b`.
- **m5.** S13 should be #79's own sequence: B of 100 samples, `DropAndFlag`, at `s+1`; the value at `s+5`; accepted on both. State that a booking refused at receipt adds no value end. Move `s13_parity_…` from 5a to 5b-2: today the Providers disagree, so 5a cannot pin one answer.
- **m6.** A Stop's `t` is control's `now` (`control.rs:311-315`), but the owner may hand one more step before it drains `TxCmd::Stop`. Its front can then pass `stop_k` by one step's duration. Name this as host timing in RM-16/UR-23. Parity tests at `stop_k` use the manual clock.
- **m7 (VERIFIED).** `ideal` has lead 0 and W 0 (`channels.rs:352`). Say whether the device fronts (+5/+1/+2/+3) apply on `ideal`. Today `short_burst_floor` applies on every profile (`lib.rs:318`).
- **m8.** After a host-late move or drop of a head, its joined pieces become heads on the Module. A short one is then closed (`tx.rs:594`), and a later join on it goes late. Either re-frame the chain on `drop_piece`, or record this as a host-lateness consequence.
- **m9 (VERIFIED).** The switch at `b` is a continuation: `device_sob` is false, so `short_first` is false (`tx.rs:574,594`). `ur_23_switch_cut_reuses_held_cut_path` should assert that there is no device EOB at `b`, not a legal close. Row 52 (an unpadded close) has no target.
- **m10.** Add one 2-channel stream to `s15_parity_booking_rule…`, so the held tail and close run with `channels > 1` (`tx.rs:802-813`). I found no multi-channel defect: the lengths are per channel on both Providers (`control.rs:431`, `lib.rs:1607`).

## Rulings on Q-1 to Q-7

- **Q-1, two-sided refusal: adopt.** It is half-open interval overlap. The one-sided form accepted a candidate that runs into a booking.
- **Q-2: adopt `b ≥ stop_k(s)`, with m4.** "Applied" means only that the record's switch took effect. Whether its samples reached the device is RM-16's uncertainty about handed samples.
- **Q-3: delete in 5c.** Remove `hw_b8_preemption` (`hardware.rs:650`), the `measurement.rs` sweep and the `iii.rs` verdict, with the grep counts in the report. The bench rows stay as evidence.
- **Q-4: list them, and add two items.** First, M1's case on hardware: C at the front `e+2` after a held tail closed at `J(e)`. Second, a switch at `b`. None is run in this stage.
- **Q-5: prune in 5b-2** (m2).
- **Q-6: keep `chain.front` as the device fact** (the configure guard, dispatch lateness). The record's fronts are booking facts. Add a stat that counts each owner close whose front exceeds the record's front, and assert it is zero in the on-time and parity tests. That is the invariant in checkable form.
- **Q-7: one lead, passed in: adopt.** It is 2 ms on both Providers today (`profile.rs:11`, the Mock's `profile.rs:47`; VERIFIED). The join deadline (M1) uses lead plus delivery allowance, 5 ms on both, which is also the UHD profile's declared value.

## Sub-steps

- **5a** is green and fits one run if S13 moves to 5b-2 (m5). M1's `J(e)` and M2's Stop region go into the pure rule here.
- **5b-1** is fine.
- **5b-2 is too large for one run** (both Providers, about 20 test rewrites, about 15 rows, 11 spec rules, parity). Split it:
  - **5b-2a:** `book`, `first_free` and the Stop region on both Providers; the Mock's fronts deleted; parity and KL-36. Green with today's immediate close, because S4b is not yet tested.
  - **5b-2b:** the UHD owner's close (M1: `J(e)`, the lock, the capped sleep), with S4b and rows 25 and 26.
- **5b-3:** add M4.
- **5c:** keep the `h == next` branch (M3).
- **5d-M, 5d-U, 5e:** fine.
- **5f:** fine, once its "rest" is spelled out as R5's list.

Not checked: no build, test or mutation run; the full bodies of tx.rs:4884, :5258 and `iii.rs`; the `old` texts of the N47S4 rows; whether the configure turn can make a booking late beyond the lead. The configure delay is stage 4's T5, and I did not re-check it here.

<!-- opus-rev3-recheck.md -->
# Note 47 stage 5: narrow re-check of plan rev 3 and the 5a code (Opus, read-only, HEAD c41530f + tree)

**Verdict: GO for 5b-1. NO-GO for 5b-2a until a 5a fix round corrects `halt` (majors 1–3). NO-GO for 5b-2b until rev 4 settles M1's receipt race and removes the second deadline (majors 4–5).**
5b-1 touches only the value number (`until`), so it does not depend on these fixes. 5a changes no behaviour (details below). Probe (VERIFIED, run in a throwaway copy that has since been deleted): majors 1 and 2 reproduce on the 5a code as written. A mutant that deletes the region survives all 38 staging tests.

## M1–M4

- **M1 — partly closed.** The 148 µs case is gone: `J(e)` leaves 3 ms of slack over the 2 ms lead, so a close up to one `IDLE` (200 µs) after `J(e)` still leaves C on time. `continuation_open` holds after a join through `last == e` (§1.2). **Not closed:** "the receipt at `J(e)` is decided by the record alone" (major 4), and the plan still states the old deadline in several places (major 5).
- **M2 — closed for the original sequence** (a first Stop drops D, and `[t, stop)` covers it). The 5a rule overshoots in one direction and undershoots in another (majors 1–2), and nothing pins the region (major 3).
- **M3 — closed.** §3 and §8 5c keep the reduced `h == next` branch, and 5c deletes only `h < next` and `k < c`.
- **M4 — closed** in the text and in `ur_23_s3_…` (M4's sequence, row 59). The data model gap is minor m1.

## Majors

1. **"A Stop never lowers a finite front" refuses valid bookings after stopping a long burst** (`crates/ezsdr-radio/src/staging.rs:163-169`; pinned as correct by `tests/staging.rs:434` (second half) and `:463`, and by row N47S5-62). VERIFIED by probe.
   Sequence at 1 MS/s, W = 10 ms:
   1. `accept(0, 1_000_000)`.
   2. Stop at `t = 100_000`: `halt(t, 110_003)`.
   3. `accept(110_003, 5)` → `Overlap`. The record refuses everything until `1_000_002`.
   The Module closes at its cursor, which is at most `stop_k` (§0; RM-16 "closes at its current handed cursor, no later than s + W"). Today the same booking is accepted and plays on time on both Providers. After 5b-2a both would refuse it for about 900 ms. That is a regression against RM-16's "a later burst is transmitted as usual" and against the plan's own §0 meaning of `stop_k`.
   A kept chain's front can exceed `stop_k` only when the chain was not fully handed (its end lies past `window_cut`), so `min(front, stop)` never drops below the device's front (m6 host timing aside).
   **Fix:** for every kept chain, set `end` and `front` to `min(…, stop)`. That is row 62's "mutant", made the rule. Invert the second half of `:434`, replace `:463`, and delete row 62. My "never recomputed downward" in M2 meant only "never recomputed from the remaining pieces after a drop". Capping at `stop_k` is not that.
2. **A second Stop that cuts a burst booked after the first Stop leaves its handed span unrefused** (`staging.rs:170-173`). VERIFIED by probe.
   Sequence:
   1. A repeat plays; `halt(100, 110)`.
   2. `accept(110, 1000)` (D, at the Stop bound, booked right after the first Stop). D is handed at once, because its start is within W.
   3. `halt(105, 200)` drops D, but the merge keeps the region at `(100, min(110, 200)) = (100, 110)`.
   4. `accept(111, 1)` → `Ok`.
   On UHD the second Stop closes at its cursor, inside D (front ≤ 200), so the accepted booking goes late. The Mock's `stop_tx` plays D's handed part, so the booking overlaps samples already played. This is M2's defect again, reached through the "second Stop never extends" merge.
   **Fix:** extend the region to the hull of the old region and `[t, stop)` exactly when `halt` drops a chain, and leave it unchanged otherwise. `rm_16_second_stop_keeps_the_earlier_bound` (`tests/staging.rs:391`) stays green, because its second halt drops nothing. With major 1's `min`, the `cut` test reduces to "a chain is dropped".
3. **M2's region is unpinned in 5a** (VERIFIED by probe). Replacing `self.region = Some(…)` (`staging.rs:170`) with a no-op leaves all 38 tests green. Every region refusal the tests make is also explained by a chain's own front, and `cancelled` at `:422-425` asserts only acceptance at 115. M2's own sequence has no pure test.
   **Fix:** add `accept(100, 40); halt(100, 110); accept(105, 1)` → `Overlap`, plus major 2's sequence. Add a row that removes the region, and one for the region term in `held_until` (`:183`). `first_free` is never tested after a Stop.
4. **M1(c), the receipt race, is not closed** (VERIFIED in the code; the outcome is INFERRED). Control reads its receipt `now` at `control.rs:451`, before it takes the streams lock (`:498`, where 5b-2a's `accept` goes). The plan locks only the owner's read (§3).
   Sequence:
   1. Control reads `now = J(e)`.
   2. Before control takes the lock, the owner steps at `J(e)+δ`: under the lock, `continuation_open(e, J(e)+δ)` is false (no join recorded yet), so the owner closes at `e` with front `e+2`.
   3. Control takes the lock: `accept(e, len, J(e))` joins.
   4. B is dispatched against front `e+2` and goes late (`TIME_ERROR`, moved or dropped). The Mock plays it on time.
   Test 58 cannot show this on the manual clock.
   **Fix (one rule, the record decides):** the owner's close clears that chain's `join` in the record under the same lock (a pure `close(e)`), so a stale `now` cannot join any more. The alternative is to read `now` under the streams lock in `book_burst`. Either belongs in rev 4 §1.2 and §3, before 5b-2b.
5. **Two join deadlines in the plan.** §3's "Equality" bullet (test-plan.md:233) says a booking at exactly `earliest = ordinary_k(now)` finds the chain open. So do S4b's "in time (`start ≥ ordinary_k(now)`)" (:201), row 26 (:420) and `ur_21_s4b_booking_exactly_at_the_deadline_continues` (:287).
   Under rev 3, a join whose start equals `ordinary_k(now)` has `now ≈ instant(e) − 2 ms > J(e)`, so `accept` refuses it (:163). Written as planned, the S4b tests assert a continuation that the 5a rule refuses.
   **Fix:** S4b's B is received at or before `J(e)`. Delete row 26 and its test (row 58 owns the equality), and delete the `≥`/`<=` bullet in §3.

## Minors

- m1 (INFERRED). `move_start(old, new)` (§1.2, :143) has no head length, and `Chain` keeps no head end. With a joined piece, any forward move overlaps the chain's own joined region. State it as "a head with a joined piece is never moved; refused", or pass `len`.
- m2. The idle-sleep cap at `J(e)` (§3, :231) is not needed, given the 3 ms slack. Row 57's killer (the held-tail-waits test) does not observe it, and row 56's killer (the 148 µs test) involves no join, so it cannot see `last == e`. Swap the killers of 56 and 57 (or drop the cap and row 57). Row 56's own mutation should be "close at the lead deadline instead of `J(e)`".
- m3. `first_free`'s join exception (`staging.rs:181`) ignores `J(e)`, while `accept` applies it. A SendAsap bound that lands on a chain end after `J(e)` is moved there and then refused. Give `first_free` the same `now` and `join_by` test.
- m4. The stale rev 2 bullets in §8 remain, so 5b-1 and 5b-2 are listed twice (:596-597). §8 5b-2a and §4.6 (:379) put S4b into 5b-2a's parity test while saying S4b waits for 5b-2b.
- m5. §1.3's last paragraph (:168) calls "a join received after `J(e)` is refused" the 148 µs case. The 148 µs case is C booked early at `e+2`.
- m6. `design/07-radio-model.md:164` (RM-15's new paragraph) and `design/18-uhd-radio.md:250` (UR-22's added sentence) state a receipt refusal that no Provider makes until 5b-2a. UR-22 also carries "(note 47 stage 5)" inside the rule text. Land both with 5b-2a, the same commit as the behaviour (as KL-36 does).

## 5a code against rev 3: no behaviour change (VERIFIED)

- The Providers change only the type name and `.clone()` (`lib.rs:1843`, `control.rs:287`, `core.rs:377`, `differential.rs:585`).
- `book`, `stop` and `stageable` are unchanged in body.
- `accept`, `halt`, `first_free` and `repeat_boundary` have no caller outside `staging.rs` and its tests (grep for `.accept(`, `.halt(`, `.first_free(` and `repeat_boundary(` over `crates`).
- No rename changed a decision. The re-anchored N47S4-08 and -48 produce the same mutant: `Bookings::default()` replaces the stream's record.

## Rows 01–14, 54, 55, 62

- Each mutant is observed by its named killer, as the probes report. I re-derived 05, 06, 08, 10, 12, 14 and 54 against the tests.
- Row 01 duplicates N47S4-01 (as noted in the plan).
- **Row 62 pins the wrong side** (major 1). Its "mutant" is the fix.
- No row covers the region (major 3), `first_free` after a Stop, or `held_until`'s region term.

Not checked: rows 15–61 against code that does not exist yet; the Mock's `stop_tx` horizon in detail (major 2's Mock outcome is INFERRED from `lib.rs:1162-1172` as read in the rev 2 re-check); end checks, which I did not re-run.

<!-- opus-a2-recheck.md -->
# Note 47 stage 5: re-check of the 5a fix round and plan rev 4 (Opus, read-only, HEAD c41530f + tree)

**Verdict: GO for 5b-1. NO-GO for 5b-2a until major A is fixed in `staging.rs`.** The fix is one line plus a test and a row. The orchestrator can check it without another re-review.
Probes (VERIFIED) ran in a throwaway copy, now deleted: pure sequences P1–P5; the fix for A (all 42 staging tests stay green); mutants that remove `cut` and the `end` cap.

## (1) `halt`

- **My three majors are gone (VERIFIED).** After `halt(100_000, 110_003)` on a 1 000 000-sample burst, `accept(110_003, 5)` returns `Ok` (`tests/staging.rs:425`). Major 2's sequence is refused through the hull (`:433-439`). Removing the region fails three tests (rows 63–65).
- **The fix's own choices hold.** Every kept chain is capped at `stop` (`staging.rs:168`). The region is added or hulled only `if dropped` (`:171`). The `cut` early return is kept (`:161-164`).
- **No sequence accepts a booking at receipt that overlaps samples already handed** (VERIFIED for the record). I tried repeated Stops (`:391`, and repeat → Stop → booking at the bound → second Stop), an idle stream, a head starting in `[t, stop)` and a repeat. Each leaves `[t, stop)` covered by a capped kept chain or by the region. The one exception is m5 (the close shape, INFERRED).

### Major A: a Stop that cancels a booking not yet handed refuses on-time bookings for up to W

`staging.rs:165` (VERIFIED by probe; the Provider outcome is INFERRED).
`dropped` counts every chain with `start >= t`, including one that starts at or after `stop`. Such a chain lies past the window, so the Module has handed none of it (§0: the handed cursor is at most `stop_k − 3`). The region `[t, stop)` then refuses bookings that nothing on the device occupies.
Sequence at 1 MS/s, W = 10 ms:
1. `accept(1_000_000, 100)`: D is booked 1 s ahead.
2. Stop: `halt(100, 10_103)`.
3. `accept(5_000, 10)` returns `Overlap` (P1).
P2 shows the same with a head booked at 50 and `halt(10, 20)`: `accept(15, 1)` returns `Overlap`.
The state is physically the same as `rm_16_a_stop_that_cuts_nothing_adds_no_region`'s "ended" case (`tests/staging.rs:506-508`, which accepts), so whether an unrelated far booking existed decides the answer.
Today the Mock's `stop_tx` sets its front from the handed payload at about `at_stop + 2` (`crates/ezsdr-mock-radio/src/lib.rs:1167-1178`). UHD cancels held bursts that were not handed. Both therefore play this booking on time: RM-16 says "a later burst is transmitted as usual" (`design/07-radio-model.md:166`). After 5b-2a both would answer `COMMAND_REJECTED` for up to W+3 samples. `vh_8_tx_stop_allows_a_later_burst` does not see this, because it books 150 ms later.
**Fix:** `let dropped = self.chains.iter().any(|c| c.start >= t && c.start < stop);`. With it, all 42 tests pass and P1 and P2 return `Ok` (VERIFIED). Add P1 as a test, and a row that drops `&& c.start < stop` with that test as its killer. In §1.2 and §1.3, say "a Stop that cancels a booking starting before `stop_k`".

## (2) Rev 4 M1: `close(e)` under the streams lock; `now` still read before the lock

- **The race I found is removed** (INFERRED: 5b-2b is not written yet). Control reads `now` at `control.rs:451`. Its `accept` comes after the owner's `close(e)`, so it finds `join == false` and returns `Overlap` (§1.3 step 4). It no longer joins a chain that was already closed.
  - What remains is a host-timing parity gap: with `now ≈ J(e)`, the Mock accepts and UHD refuses. The refusal is visible at receipt, not a silent late, and §3's last line already counts it as host timing.
  - A stale owner `now` only delays the close by one turn. `continuation_open`, `close` and `accept` decide under one lock, so no join lands between the owner's read and its close.
- **The cap can go** (VERIFIED from the code).
  - The owner closes after `J(e)` by one `IDLE` plus the time it takes to be scheduled. The waiting turn returns `false` (`tx.rs:467`), the run loop then sleeps `IDLE` (`tx.rs:286-288`), and `pass` reads reports with `Wall::ZERO` (`tx.rs:312`), so it does not block.
  - A booking that joined before the close prevents the close (`last == e`). C, booked early at `e+2`, is dispatched after the close and keeps `3 ms − IDLE` of lead. Only a host stall of about 3 ms makes anything late, and that is host lateness.
- **One `J(e)`, `move_start(.., len)` and `first_free(now, join_by)` agree.**
  - `first_free` and `accept` share `join_by` and `now`. `held_until` returns `c.front`, never the join point, unless `k == e` exactly, so a SendAsap start that `first_free` returns is one `accept` takes.
  - Pruning (`front ≤ ordinary_k(now)`) never removes a chain that can still be joined. `move_start` with `len` makes any forward move of a joined head collide with its own pieces (m8 stays consistent). One gap remains; see m4.

## (3) Rows 09, 10, 12, 14, 55, 62–68

- **All 12 are observed by their named killers** (VERIFIED: I re-derived each mutant against its test, and the logs in `probes-a2/` fail at the lines I predicted: 405, 457, 521 and 445).
- **Row 10 is right now.** `> t` leaves `dropped` false while `retain` still removes the chain at `t`, so no region is added. Line 405 sees it.
- **Row 55 is right.** The mutant's region `(10, 20)` from the first Stop hulls into `(10, 40)`, and line 457 sees it.
- Rows 63–65 pin the region's three parts, row 68 pins m3, and row 62 now pins the superseded rule from the right side. Rows 09 and 66 are near-duplicates (minor m3).

## Minors

- m1 (VERIFIED): `c.end.min(stop)` (`staging.rs:168`) has no observable effect. The `end` cap mutant passes all 42 tests: after `halt`, `join` is false, and `end` is read only together with `join`. Drop the cap, or pin it when 5b-2b's `close` and `continuation_open` read `end`. §9 rule 4.
- m2 (VERIFIED): the `cut` early return (`:161-164`) is redundant. Removing it keeps all 42 tests green. When `cut` is false, nothing is dropped and the caps change nothing; it only skips clearing `join` on chains that end at or before `t`, and no on-time receipt can reach those joins (`now > J(e)`). Delete it, or keep it with a comment saying it is an optimisation.
- m3: rows 09 (`min(stop)`) and 66 (`front`) mutate the same anchor and are killed by the same assertion (`tests/staging.rs:446`). Both give the first Stop's end. Keep one (§9 rule 5).
- m4 (VERIFIED pure, INFERRED reachability): `first_free` excuses a join point without knowing whether the candidate is a repeat. `accept(0, 40)` then `first_free(40) == 40`, and a repeat there gets `Overlap` (P4). On the x3xx profile this is unreachable for a SendAsap start, because `k = ordinary_k(now)` means `now > J(k)` while the allowance is above 0. It is reachable where lead plus allowance is 0. In 5b-2a, pass `len.is_some()` or do not call `first_free` for repeats.
- m5 (INFERRED, the plan's §11 `+3`): a 1–3 sample head at `c−1` (`c = stop − 3`) is padded to `c+3` and its device front is `c+4`, yet the record accepts `c+3` (P5). With the cap on `ideal` (W = 0), a kept 1-sample head at `t−1` drops from `t+4` to `t+3` (P3). Add both to the bench and §11 list.

Not checked: 5b-1 code (not written); rev 4 §4–§8 beyond the sections above; Mock or UHD Provider wiring of `halt` (5b-2a).

<!-- opus-j-recheck.md -->
# Opus re-check of the 5j plan (rev 22), 2026-11-07 — GO WITH CHANGES (summary saved by the orchestrator)
M1 (VERIFIED): `first_free` checks one sample, `accept` checks the full region `[start, front)` with `meets`, so "accept then cannot refuse" is false. Counterexamples (s = earliest free sample):
 (a) finite B (10 samples) at s+20 beyond the horizon; untimed finite A of 30 samples: first_free(s)=s, accept([s, s+32)) meets B → COMMAND_REJECTED although s+31.. is free; A should go after B's front.
 (b) timed finite B far ahead; untimed start_repeat: first_free returns `ordinary`, a repeat's front is MAX so it meets B → refused; it should start after B.
 (c) a timed Stop at t cancels a future booking (region [t, stop)); untimed finite A with from < t whose region reaches t → refused.
 Fix: one pure `Bookings::place(from, len, now, join_by, horizon) -> Option<i64>` looping first_free → meets on the full front → jump past what it met; both Providers at receipt, UHD at dispatch; the timed SendAsap move should use it too (orchestrator's call).
M2 (VERIFIED path): admission sets `requested_at` (SC-23a) for untimed TxBursts too (`admission.rs:155-158`); Providers copy it into the record (`lib.rs:1646`, `control.rs:466`): an untimed burst whose now+lead is off the sample grid records a requested_target. Kernel sets requested_at = None when untimed.
M3 (VERIFIED): dispatch-time move of an untimed head with joined pieces: remove only the head's span, no hand-over horizon at dispatch, place after the pieces' front. Sequence: H 40 at 100, P joined at 140, bound 120 → placement 120, move_start fails, H silently refused.
Minors: untimed required field (no serde default, invariant 39); carry Compiled.untimed to every compiled item, not by index; one staging function for the whole untimed decision; untimed overrides every non-OnTime dispatch outcome, Err stays a refusal; parity inputs: choose `at` so it binds (Mock lead = min_timed_command_lead_ns, UHD = DEVICE_LEAD_NS); F36 timed move late_by = B − at with one shared ceiling rescale.
Q1 agree; Q2 None (with M2); Q3 required; Q4 untimed = the Kernel chose `at` (RS-19); Q5 untimed record reports no lateness; Q6, Q7, Q9, Q10 agree; Q8 keep text.
Wall clock: Python repeat-after-stop test asserts target ≥ B, transmitted, no TIME_ERROR; Rust Session tests on simulated/manual clocks may assert = B; check `ur_21_untimed_sends_are_each_played`'s clock mode.

<!-- opus-sol-fix-recheck.md -->
# Opus re-check of the Sol fix round (1bc67d3..09e6195), 2026-11-07 — FAIL (saved by the orchestrator)
S1 closed (VERIFIED); minor: `Chain.joined` only grows (push only); a long back-to-back chain grows for the whole run, `contains` O(n) under the lock — bound it (drop join points device time has passed, keeping remove_head's first-piece rule).
S2 MAJOR (VERIFIED): the `send()` change (`joined_end`, tx.rs:661-666) leaves a 2–32-sample first send open, against UR-22 (open first send ≥ MIN_OPEN_FIRST_SEND = 32; bench: ≤ 28 underflows at interpolation ≥ 80, device.rs:657-663, design/18:741) and UR-23:260. Sequence: A(40), B(10), C(4) contiguous; A's configure refused, removed; B goes out as a 10-sample SOB packet without EOB; C in a later tx_send → underflow, C lost silently (before: C flagged late). Tests pass only because the fake clears an open short packet when the next one arrives at the same tick (device.rs:641-642) and the held-form test never reads reports. Neighbour: A(40)+B(10) without C: after remove_head B keeps A's `join=true` and end+2 front, so continuation_open(B_end) holds B open short with nothing joined, and a D received before J(B_end) would be accepted as a join then dropped late. Recommendation: (1) revert the send() change; (2) in remove_head, when the surviving head is the only piece and ≤ 32 samples, clear `join` and give it its standalone front; (3) when pieces are already joined, C is decided by its late policy against B's closed front (UR-23:260) — record as a consequence of a refused configure (spec line or KL entry), S2's test rewritten to expect it. Rejected: packing B and C into one packet; refusing the whole chain.
S3 closed, no neighbour (VERIFIED).
S4 MAJOR (VERIFIED): `stop_at(clock.instant(cursor), tx_in_flight)` (tx.rs:976) gives stop = at_or_after(instant(cursor)+W)+3, one window past the device's real front cursor+3 (tx.rs:983). (1) after an underflow abandon at cursor c, an on-time DropAndFlag burst at s in [max(c+3, now+W+3), c+W+3) is refused "overlaps", though a user Stop at the same instant would accept it; (2) a switch installed with b ≥ c is settled `applied` by halt though nothing of it played. Fix: `stream.stop_at(clock.instant(cursor), 0)` (stop = cursor+3, as advance_front). `playing = None` is unobservable (an equivalent mutant; keep, document, no row). The no-cursor `refuse` in install stays reachable via host lateness (repeat dropped late by DropAndFlag after a switch was accepted): keep it; a test would be good (INFERRED).

<!-- opus-fix5-recheck.md -->
# Opus re-check of fix round 5 (75d770b..b89740d), 2026-11-07 — FAIL (saved by the orchestrator)
(1) S2 MAJOR: staging.rs:491-496 shrinks the front to the first retained join point even when joined pieces remain. Sequence (1 MS/s, on time): A(40)@100000 configure-refused; B(10)@100040, C(4)@100050, D(10)@100054 joined; A leaves → {start 100040, joined [100050,100054], join false, front 100051}; E(10, DropAndFlag)@100056 received before D is dispatched is accepted (75d770b refused it, front 100066); owner closes B at 100050; C late; D plays [100054,100064) front 100065; E dropped late. Fix: shrink only when `joined` is empty after the retain; with pieces remaining clear `join` and keep the front, at least each retained piece's own front (a 1–3-sample last piece needs end+4); test with three pieces. Deviation (d) only over-strict (host lateness only).
(2) S4 MAJOR: halt's `installed && boundary < stop` with stop = c+3 settles a switch with b in [c, c+3) `applied`; c == b is the natural case (the owner cuts at the switch's held key b and joins it at b). Sequence: switch installed at b; old samples sent up to b; the switch's first send fails or the timeline-expired path (tx.rs:467-470) runs; abandon with cursor = b → row `applied`, transmitted(b) None. Fix: in `abandon`, cancel a pending installed switch whose boundary ≥ cursor before `stop_at`; test cursor == boundary. Neighbour (INFERRED, pre-existing): a report between `join_open(b)` and the switch's Open finds open == None, abandon skips stop_at, the installed switch stays pending and `advance` later settles it applied — test it.
(3) S1 prune: no loss (VERIFIED). (4) F57 correct (VERIFIED).
Minor: RM-15 should say pieces retained behind a closed short head keep their regions.

<!-- opus-fix6-recheck.md -->
# Opus re-check of fix round 6 (b89740d..d654646), 2026-11-07 — FAIL, one major (saved by the orchestrator)
(1) remove_head with retained pieces: PASS. Minor (INFERRED, narrow race): clearing `join` disables continuation_open's wait at an accepted join point (staging.rs:253); a piece accepted before the head leaves but whose TxCmd is not yet drained when the owner reaches its join point is closed out by close_unjoined (tx.rs:542-548) and falls to its late policy — new for a head > 32 samples; reported loss, no RF or parity risk.
(2) MAJOR neighbour: between `join_open(b)` and the switch's dispatch `open` is None, so `abandon`'s stop (tx.rs:976, gated on `self.open` being a repeat) does not run: S is cancelled but R's chain keeps front i64::MAX and `playing` points at S. Sequence: book R@2000 period 1000; switch S b=15000; owner hands R to 15000, `join_open(15000)`; Underflow → abandon at cursor 15000; S cancelled; no stop_at; book finite X on time at 20000 → refused (Overlap::Booking) until a Stop (one step earlier the same X is accepted). Fix: also run the stop_at / playing = None block when a switch was cancelled (`switched`); extend `ur_21_s4_a_report_between_the_join_and_the_switch_s_open_cancels_it` (tx.rs:6774) to assert X accepted on time.
(3) PASS. (4) RM-37 sentence matches; minors: its Test line does not name the new rm_37/ur_21_s4 tests; the parenthesis names only an underflow though the same path runs on timeline expiry and failed sends (tx.rs:471, 493, 675, 679).

<!-- opus-fix7-recheck.md -->
# Opus re-check of fix round 7 (d654646..1031f90), 2026-11-07 — PASS WITH MINORS (saved by the orchestrator)
(1) closed (VERIFIED). (2) every abandon of a playing repeat ends it once at the cursor; a repeat not started is never stopped (VERIFIED).
Minor A (new, INFERRED, host-late): join_open(b), host late; `advance(now)` (tx.rs:452) applies S first; the timeline-expiry abandon then runs with switched=false, repeat=true; stop_at(b) removes S's chain, playing=None; S stays held (removal gated on `switched`) and is dispatched late: DropAndFlag → S dropped, row still `applied`; SendAsapAndFlag → move_start puts S back as an endless repeat but playing stays None, so a later switch is refused. Fix: when `repeat` comes from the held switch, remove held switches at or after the cursor whatever `switched` says.
Minor B (reachable, INFERRED, host-late): F ends 1500, R booked at 2000 as its own head; host late: at 2100 S accepted (b=4000) while the owner still has F open; timeline expiry abandons F; cancel_switch_from cancels S; R not stopped; R dispatched late SendAsap → plays the old waveform forever while S reads `cancelled` (RM-37; Mock takes S at b). Options: cancel S only when the replaced repeat ends or is dropped at dispatch, or a KL entry.
Unverified: the DropAndFlag drop path at tx.rs:1087 calls no `drop_piece`, so a dropped late repeat may leave its endless chain in the record.
