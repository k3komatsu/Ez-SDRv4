# E1 test plan (note 49 stage E1: bookings and Stops in one receipt-ordered list)

Status: plan only, no code, nothing built or run. Base `4151197`. Every fact is VERIFIED (read at the `path:line` named, HEAD) or INFERRED (settled by the first run; the implementer reports each one it settles, and a result that contradicts the plan is reported, never tuned away). Sketch = `sketch.md`; Qn = its sequences.

## 0. Spec/code disagreements found while planning (first, as the brief asks)

1. **Mutation row R07 (`plan/phase7/tools/mutations.json`) has the deleted `ur_23_a_clock_end_cancels_every_held_burst` as its only `filter`.** The sketch §3 table does not list it. It mutates `report_cancelled_burst`'s `cancellation_reports` branch, which stays. Retarget its `filter` to `ur_22_underflow_close_failure_reopens_before_the_next_payload` (tx.rs:1564; its :1647-1651 asserts exactly one COMMAND_REJECTED for `ClockEnded`, so `if false` dies there). The note-only half is pinned by `ur_26_stop_cancels_the_held_burst_record_only` (tx.rs:4025).
2. **Rows N47S4-37, N47S5-57, N47S5-61 name tests the brief deletes or rewrites** (grep §6). N47S4-37 keeps its filter (the test is rewritten under the same name); N47S5-57 is kept and retargeted to Q14 (§5); N47S5-61 moves to the rebuilt test (§4).
3. **Sketch §3 says `end_clock`'s cancel loop is unreachable. I agree for a Provider::stop cut, with two limits.** `Streams::end` returns early on the second call and on `lost` (core.rs:599-601, :586-591), and a loss pushes no halt. After a loss, `end_clock` still cannot run: the run loop idles on `is_lost` (tx.rs:297) and `step` returns early (:445). So the loop stays dead; no limit changes the removal.
4. **`Streams::book` can return false for a Stop** (`lines.book` Err or no line: core.rs:555, :560-563). Today's `TxCmd::Stop` goes out regardless (control.rs:320-322), and the sketch keeps that. The plan has a row for it only if a refused Stop is reachable on the rig (§7, U8); the implementer greps `lines.book` errors for the Stop kind and reports.
5. **Rows P245/P442 (mod.rs `to_tx.len()`)** name `Running.to_tx`, which stays (it carries `Shutdown`). Only `Control.to_tx` goes. They need no change, and the grep in §6 proves it by showing `Running` is untouched.
6. Q8 may not differ from today in the handed cursor: `read_cut`/`stop_at` already bound the stream from the Run end (core.rs:603-605, tx.rs:`read_cut`). What differs today is that the burst row, the EOB close and the held bookings' notes wait for Shutdown. The Q8 test asserts those (INFERRED; if the closing is already done before Shutdown, the brief's point 2 is only a pin, and the report says so).

## 1. Test homes and rigs (no wall-clock waits, design note 37)

- **UHD, new file `crates/ezsdr-radio-uhd/src/provider/tx/tests/e1.rs`**, mounted `mod e1;` beside `mod o1;` (tx.rs:1345). It uses `switch_rig_with`, `book_with`, `book_repeat`, `book_switch`, `stop`, `run`, `device_at`, `owner_with`, `turns`, `valued`, `txp_*` through `super::*`, and the oracle `crate::provider::tx::oracle` (stop_bound, Slots). Rig: `FakeDevice` manual clock; time moves only through `device_at`; receipt and owner turn are called from the test thread in the order written, so no verdict depends on when a thread runs. The owner's list read is `tx.read_list()` or `tx.pass()`/`tx.step()` (the new reads), never `drain_tx`'s mpsc.
- **Mock, appended (no existing line edited) to `crates/ezsdr-mock-radio/tests/mock_channel/o1.rs`**, which owns the private harness `M` (`book`, `stop`, `go`, `check`, `cancellations`). Stepped run: actions queued with `push` are all taken in one `step(u)`.
- **Pure, `crates/ezsdr-radio/tests/staging.rs`** (public API of `ezsdr_radio::staging`).
- **Probes** (test-only no-op in production, `core.rs:882-892`): two new names, each one `self.core.probe(name, None)` line, called with no lock held, so a hook may take the streams lock and call `Streams::end` and `device_at` on the test thread:
  - `tx_book_cut_checked`: in `book_burst` between the first (cut) critical section and the main one.
  - `tx_step_guarded`: in `step`, right after the critical section that read `now`.
- O1 tests drive uhd through the `Rig` of `o1.rs` and call `drain_tx` (its body becomes `tx.read_list()`) and `stop(&mut rig)` (book + `read_list`), so they need no edit and stay green. The `o1.rs:675` comment "E1/E2a" becomes "E2a" and nothing else in the file changes.

## 2. Sequences: test, rig, assertion, the wrong behaviour that fails it

Names: `e1_*` are new (in e1.rs unless stated); others exist.

| Q | Test | Rig and observation | Asserts | Wrong behaviour that fails it |
|---|---|---|---|---|
| Q1 | new `e1_q1_a_stop_is_read_between_the_booking_before_it_and_the_one_after_it` | `switch_rig_with`; A (12 000 samples at 10 000) and H3 (start 40 000) booked, turns to 10 ms, `at(11 ms)`, one turn; then **no drain**: `control.book(Stop)`, then book R (start 30 000, after B) atomically; then `tx.read_list()` and turns to 60 ms. Samples via `FakeDevice::transmitted` + `o::Slots`, `rejected` rows, `bursts` rows | read order A, Stop, R: H3 has one note "UR-26: cancelled by Stop" and no event; A is cut in `[t_k, B]`; R plays exactly from 30 000 (Slots), is not refused; burst rows: A `end == "stop"`, R 200 samples at 30 000 | Stop read after R (receipt order broken): R is cancelled with the Stop note and 30 000 is silent; Stop read before A's booking: A never cut. `o1_uhd_t6c` already covers the same outcome but drains between the Stop and R, so it cannot see order |
| Q1 (Mock side) | Q10 | | | |
| Q2 | new `e1_q2_the_step_reads_a_stop_it_has_not_read_before_it_hands` (the guard is on an unread halt, amendment R2) | repeat R playing (as ur_23_a_provider_stop…); owner turn done (list empty); then `Streams::end(core, t, false, false)` (adds halt and `stop_at`), `device_at(t + 2 ms)`; then **`tx.step()` directly** (not `read_list`) | after that one step: handed cursor ≤ `B-3` (`o::stop_bound`); the closing send has EOB; R's row `end == "stop"` after the next step | a step that skips the unread-halt check hands up to `at_or_after(t + 2 ms + W)`, past `B-3` (Q2's wrong outcome); its sends show payload beyond `B-3` |
| Q2b | new `e1_q2b_now_is_read_under_the_lock_that_checked_the_list` | R playing; probe `tx_step_guarded` hook: `Streams::end(core, t, ..)` then `device_at(t + 2 ms)`, once (an `AtomicBool`); one `tx.step()` | the turn that raced the Stop hands nothing at or after `B-3`; the next `read_list` then closes R at its handed cursor | `now` read after the section: the hook's time move is seen, `window_cut = at_or_after(t + 2 ms + W)`, the turn hands past `B-3` |
| Q3 | new `e1_q3_a_pre_origin_provider_stop_cancels_a_booking_with_its_own_note`, three arms (`for order in [..]`) | `owner_with`/`switch_rig` with a clock origin 1 s after arm (the rig of `ur_25_an_end_booked…`, tx.rs:1965). Stop@t1 (control), X booked start ≥ B, `Streams::end(…, t2 ≤ origin, false, false)`, time to origin. Arms: (a) nothing read before the origin; (b) owner reads Stop and X, then the end, then the origin; (c) as (a) but `tx.shutdown()` runs before the origin turn | `rejected`: X has exactly one note "UR-26: cancelled by Provider::stop"; zero COMMAND_REJECTED events; `tx.clock.is_none()` and `tx.cut.is_none()` after the origin; no `tx_send`; arm (c) leaves the same single note | today X is "UR-25: its transmit clock ended…" with COMMAND_REJECTED in (b) (end_clock runs first), or the note depends on Shutdown order; a missing `Streams::end` halt push gives no note at all; a halt pushed with `CancelReason::Stop` gives the wrong text |
| Q3 (the existing) | rewritten `ur_25_an_end_booked_after_the_first_read_cancels_the_held_burst_at_the_origin` (same name, N47S4-37 filter) | same rig: A held after the first `pass`; end at 500 ms; origin reached; first `tx.pass()` reads the halt and returns (step returns true after reading); second `tx.pass()` runs the clock end | after the first pass: the single note (no EOB/close expectation: nothing is open before the origin); after the second: `tx.cut.is_none()` and `tx.clock.is_none()` (keeps N47S4-37 killed). The single note is "UR-26: cancelled by Provider::stop"; **zero** COMMAND_REJECTED events; no `tx_send` | the old expectation (ClockEnded text and one event) must now fail |
| Q4 | existing `ur_21_s8_switch_installed_before_the_boundary_sample`, `ur_21_s12_…`, `o1_uhd_t4a`, `o1_uhd_t4b` | unchanged rigs | S installed at its boundary from the handed cursor; held at `S.k` | unchanged. `receive` must still install a switch through `install` before holding it |
| Q4b | new `e1_q4b_a_switch_read_after_the_stop_that_took_it_is_not_taken` | repeat R playing at 3 ms (as `ur_23_stop_before_the_boundary_cancels_the_pending_switch`); `book_switch` S (atomic); `control.book(Stop)` with no read; `read_list` | S: COMMAND_REJECTED "UR-21: the repeat switch was not taken…" with S's action id; the `switches` row is `cancelled`; R closes at its handed cursor (`end == "stop"`); no S samples | S read after the Stop (order broken): S is cancelled by the Stop with a note and no event, so the event count is 0 |
| Q5 | new `e1_q5_a_booking_waiting_when_the_repeat_is_abandoned_still_plays` (existing :6884 and `o1_uhd_t7a/b` stay as regression) | repeat R playing, FailSend underflow abandon scheduled; order: Stop receipt, then B receipt (start ≥ the abandon cursor), then the abandon through the report path: `tx.report(Underflow)` then `read_list`, as the test at tx.rs:~6884 does (`report` abandons at tx.rs:~1259-1263); not FailSend inside `step`, where the pass's guard reads the Stop and B first and no abandon runs | B is unread when the abandon runs; it is read afterwards (after the Stop) and plays (Slots); today's E1 behaviour, without E2a's #80 fix (B's chain dropped from the record is not asserted either way) | halt pushed inside `StreamState::stop_at` (abandon calls it, tx.rs:999): B is cancelled by the abandon. Row U14 |
| Q6 | new `e1_q6_a_stop_a_booking_and_the_run_end_all_unread` | A booked, `control.book(Stop)`, C booked, `Streams::end`, then one `read_list`; then `tx.shutdown()` | notes in order: A "cancelled by Stop", C "cancelled by Provider::stop"; `list.held` empty; the shutdown adds no note; zero COMMAND_REJECTED | C is cancelled by the Stop (reading order) or by nothing; Shutdown re-reporting |
| Q6b | new `e1_q6b_shutdown_reads_the_list_before_it_cancels` | A booked, Stop unread, `tx.shutdown()` straight away | A's note is "UR-26: cancelled by Stop" (its own reason) | shutdown that does not read first labels it "Provider::stop". Row U11 |
| Q7 | new refusal arm `e1_q7_a_stop_received_while_a_configure_refuses_its_burst` plus existing `ur_22_a_stop_during_the_configure_leaves_no_payload_after_it` (:6426), `ur_22_a_provider_stop_during_the_configure_keeps_the_mark_and_cancels_the_burst` (:6557), `ur_21_a_refused_configure_refuses_the_burst_…` | the probe's `TxCmd::Stop` send becomes `list.halt(CancelReason::Stop)` under the streams lock (the probe runs with no lock held); the Shutdown send stays | configured: X is put back and the next read cancels it; **new refusal arm** (`TxApplyRefused`, probe `tx_configure_before_mark` pushes `halt(Stop)`): X in flight, Stop received, configure refuses X, then the Stop is read: exactly one COMMAND_REJECTED naming X, no "cancelled by Stop" note for X, no payload | a configure put-back after the Stop's read that the new `held` loses; a halt that clears `held` while X is in the owner's hand |
| Q8 | new `e1_q8_provider_stop_closes_the_open_burst_at_the_next_turn` | repeat R past its origin with a held X (start after B); `device_at(t0)`; `Streams::end(…, t, false, false)`; `device_at(t + 1 ms)`; **`tx.pass()` only, never `shutdown`** | after that pass: R's `bursts` row exists with `end == "stop"` and handed cursor ≤ `B-3`; the last send has EOB; X has the "Provider::stop" note | a Run end handled only at Shutdown: none of the three exists before the shutdown. The rig's manual device time cannot hide this because the test never shuts down |
| Q9 | existing `ur_22_underflow_close_failure_reopens_before_the_next_payload` (:1564; its `tx.command(TxCmd::Burst(..))` becomes `give`) | | with `clock` None after the failed reopen, the later burst is not held and gets one `ClockEnded` note and one COMMAND_REJECTED | a `receive` that holds it, or one that drops the note |
| Q10 | new Mock `e1_mock_q10_a_booking_a_stop_and_a_booking_in_one_step`, appended to `mock_channel/o1.rs` | the three Actions are queued directly with `radio.push` (not `m.book`/`m.stop`, which step at once, o1.rs:84-122), then one `step(u)` | A: one cancellation note "MR-25: cancelled by stop"; R plays from its start (loopback Slots) with a burst record; no refusal event | halt applied after R's read: R cancelled; R read before the Stop: ditto |
| Q11 | existing `ur_21_m1_receipt_at_the_deadline_is_decided_by_the_record` (:2979), `ur_21_m1_148us_booked_early_at_the_chain_front_plays_on_time` (:2932), `o1_uhd_t2*` | `txp_book` replaces `txp_record`+`give` | J(e) accepted, J(e)+1 tick refused | unchanged. This is the instants-under-the-lock check |
| 4 | new `e1_q12_a_booking_racing_a_provider_stop_cut_is_refused_at_receipt` (the brief's decision 4) | `switch_rig`; probe `tx_book_cut_checked` hook runs `Streams::end(core, now, false, false)` once; then `book_with` | the booking is refused at receipt: exactly one COMMAND_REJECTED with the booking's id and the uncut-segment reason of control.rs:422; `next_booking` is None; no note, no sample | the cut re-check absent: the booking is accepted, queues behind the Run end, and gets a ClockEnded/ProviderStop note instead of a refusal |
| 5 | new `e1_q13_a_refused_booking_is_not_on_the_list` | each refusal arm of existing `control.rs` tests (overlap, drop at booking, untimed refusal) | `lock(streams).stream(k).list` has nothing unread | the push before the decision |
| 6 | existing O1 tests (§8) | | | |

The two Q1 and Q10 rows share one Mock check: Q10 above. The O1 Mock tests `o1_mock_t6c`, `t4c`, `t6b` keep covering a Stop that follows a booking in an earlier step.

**Guard pitfall (sketch §2).** `while let Some(r) = lock(..).list.read() { .. }` deadlocks on the single-threaded rig the moment a body locks again. Killers: Q6 (three unread entries, two of which lock inside `receive`/`stop_now`) and the Q4b test. A hang, not a failure: the gate's per-test timeout reports it, and it is the one verdict that is a hang by design.

## 3. Pure `TxList` tests (`crates/ezsdr-radio/tests/staging.rs`, payload `u32`)

| Test | Asserts | Row it kills |
|---|---|---|
| `txlist_reads_in_receipt_order` | `book(1)`, `halt(Stop)`, `book(2)`: `read` yields `Booking(1)`, `Halt{Stop,..}`, `Booking(2)`, `None` | `pop_front` → `pop_back` (U6) |
| `txlist_a_halt_cancels_exactly_what_was_read_before_it` | book 1, 2 (unread), `read`+`held.insert` for each, `halt`, `book(3)`; `read` returns `Halt` with `cancelled == {1,2}` and `held` empty; next `read` is `Booking(3)`, held still empty | `mem::take` → an empty map (U9); a halt that also takes unread bookings |
| `txlist_a_booking_read_after_a_halt_survives_it` | `halt`, `book(9)`, read both, insert 9: `held == {9}` | halt that clears later bookings |
| `txlist_unread_tells_what_is_waiting` | `unread()` false on a new list, true after `book` and after `halt`, false after the last `read` | the step's guard source (U3) |
| `txlist_carries_the_reason` | `halt(Stop)`, `halt(ProviderStop)`, `halt(Loss)` read back as those reasons in order | a reason fixed in `halt` |
| `txlist_default_needs_no_payload_default` | `TxList::<NoDefault>::default()` compiles and is empty | a derived `Default` (compile-time) |
| `txlist_held_is_by_first_sample` | insert starts 30, 10, 20: keys ascend | `held` as a different map type |

## 4. Deletions, rewrites and rebuilds (every test touched)

**Deleted (unreachable state, with the argument).**
- `ur_23_a_clock_end_cancels_every_held_burst` (tx.rs:4043): sets `held` and calls `end_clock`. In E1 the Run end's halt is queued in the critical section that cuts the clock (core.rs:603-610) and the step reads it before the cut check (tx.rs:455). A held burst reaching `end_clock` would need a booking received after the cut, which receipt refuses (control.rs:415-426, plus the re-check of decision 4). Q3 and the rewritten UR-25 test pin the reachable outcome.
- `ur_25_a_burst_booked_on_an_ended_clock_is_cancelled_when_received` (tx.rs:4061): a burst whose domain differs from the owner's clock. Receipt refuses another domain (control.rs:427-429), and one clock is registered per stream (UR-13). `receive` keeps a `debug_assert` on the domain; the `clock == None` arm is Q9's, pinned.
- `ur_21_s2_a_short_head_is_not_held_for_a_piece_handed_after_it` (tx.rs:3028): its only difference from `ur_21_s2_a_short_surviving_head_is_closed_and_a_piece_joined_behind_it_is_late` (:3001) is that C's hand-over comes after B's first send. After E1 the hand-over is the receipt; booked first, C is read with the others and the owner closes B at its end, and :3001 asserts exactly that (B `(10,true,true)`, C not played, one BurstAck). Covered there, same assertions, atomic.
- `ur_21_m1_held_tail_waits_for_a_join_received_before_its_txcmd_is_drained` (:2953): its scenario hands the pieces in a later turn than their record; the reachable form of "joined, recorded, unread at the close point" is Q14 (§5), which replaces it as N47S5-57's killer. Its bottom half, B and C joined early continue A with one EOB, is kept as the new `ur_21_m1_joins_booked_early_continue_the_device_burst` (the same bottom assertions, atomic `txp_book` for A, B, C). `design/07:176` and `design/18:266` get the new name.

**Rewritten.**
- `ur_25_an_end_booked_after_the_first_read_…` (:1963): Q3 row above. `design/18-uhd-radio.md:543` is edited (see §9).
- `ur_21_s2_c_and_d_joined_behind_a_refused_head_are_late_at_dispatch` (:3061): books A, B, C, D in receipt order with `txp_book`; the E probe uses `held_keys`; assertions kept (E refused at receipt with `Overlap::Booking`; B plays; C and D late, DropAndFlag). It no longer hands B, C, D before A.
- `ur_21_a_late_joined_burst_cannot_move_and_is_refused` (:3539): the old path (A ends before B is read) is split-only, and a refused-configure rebuild is wrong: refusing A promotes B to head, and a late head can move (staging.rs:534-548, :500-516). Rebuilt now on the **finite abandon**: A (finite, long) and B joined at A's end, `SendAsapAndFlag`, booked atomically; a `FakeFault::FailSend` ends A mid-send (abandon at cursor c < A's end) so B is dispatched late while still a non-head joined piece. Move device time past instant(e) minus the lead that makes B late at dispatch (`dispatch_start`, tx.rs:~1052-1064). Before dispatch either try an `accept` inside A's span after the cursor, or drop that check and rely on the "moved start overlaps" refusal (staging.rs:~513-516). Before dispatch assert B's joined record state (the head chain's `joined` still holds B's start: observed through `busy.first_free`/an `accept` at B's start returning `Overlap::Booking`, not through private fields). Then assert one `TIME_ERROR{refused}`, one COMMAND_REJECTED with B's action id and "UR-21: the moved start overlaps an accepted booking", no B sample. N47S5-61 keeps its test name.

**Plumbing, mechanical (no assertion changes).**
- `txp_record`/`txp_give` → `txp_book`; `tx.held.insert` → `hold(&tx, h)` (held without a record: reachable between a Stop's receipt and its read); `tx.held.is_empty()/contains_key` → `held_keys`; `tx.command(TxCmd::Burst)` → `give` (book + `read_list`); `tx.command(TxCmd::Stop)` (13 sites, :1558 … :6147) → `halt` (Stop + `read_list`); `to_tx.send(TxCmd::Stop)` (:6147, :6432) → the list's `halt`; `to_tx.send(TxCmd::Shutdown)` stays (the mpsc stays); `drain_tx` keeps name and signature, body `tx.read_list()`; `txp_drain` is unchanged (it loops `step`).
- `control.rs` tests (`to_tx.try_recv()` at :751, :790-:1393, :1773-:1867): `Receiver<TxCmd>` return values go; `next_booking(&core)` pops the list for the stream (None when refused) and `stop_halted(&core)` replaces :751/:1401. `control()`/`control_with_tx_origin`/`tx_rig` stop returning the receiver; the callers named `_to_tx` change shape. `differential.rs:172-177`, `rx.rs:1043`, `mod.rs:1217` lose their `to_tx` argument.

## 5. N47S5-57 (`continuation_open`'s joined clause, staging.rs:280-284): decision (amended)

**Keep the clause and the row.** The earlier removal argument was wrong: an empty list at the step's first critical section does not protect the rest of the turn. A continuation can be accepted at J(e) after that check and before `close_unjoined` locks the streams (tx.rs:541-549); it is then recorded and unread, not in `held`, and only the joined clause keeps the device burst open. New killer: `e1_q14_a_continuation_accepted_after_the_unread_check_keeps_the_burst_open` (e1.rs). Probe: **`tx_step_guarded`** (right after the step's guard section; the step then reaches `close_unjoined` in the same call, so no new probe is needed). Rig: A (40 samples at 20 000) sent, the device burst holding its tail at cursor e = 20 040, owner idle (nothing held, `chain.next() == Some(e)`). The hook, once: `device_at(J(e))`, book B@e (10 samples, atomic: record + list), then `device_at(J(e) + 1 tick)`, so the third disjunct (`now <= join_by(e)`) is false and only the joined clause holds the burst open. Assert after that step and the following turns: no EOB before B (`sends(&device)` has no `eob` at e), B's samples continue A (`device.transmitted(clock.instant(20_000), 50)` equals A then B), one device burst, no TIME_ERROR, `past_front == 0`. Wrong behaviour: without the clause, `close_unjoined` ends A at e, B starts a new burst or is refused. Row N47S5-57's `filter` becomes this test. Deterministic: the hook runs on the test thread with no lock held.

Correction to earlier claims: record plus a queued payload can still precede the executor's read (receipt is atomic, the owner's consumption is not). "Every split-receipt state disappears" is false; only the states that need a payload handed in non-receipt order, or a record with no queued payload, disappear.

## 6. Grep commands (listed for the report; results from planning)

```
python3 - <<'E'   # rows whose old/old2/new name a removed or renamed item
import json,re
rows=[r for f in ['plan/maintenance/tools/mutations.json','plan/phase7/tools/mutations.json'] for r in json.load(open(f))]
pat=re.compile(r'self\.held\b|tx\.held|\bcommand\(|TxCmd|to_tx|stop_now|end_clock|cancel_unstarted|self\.cmds')
print([r['id'] for r in rows if pat.search(r['old']+r.get('old2','')+r['new'])])
E
# result (14): P245 P442 P453 P569 P570 P635 P1117 P1199 P1206 P1302 N47S4-41 N47S5-23 N47S5-151 U18
# rows whose filter names a deleted or rewritten test: R07 (ur_23_a_clock_end…), N47S4-37 (ur_25_an_end_booked…), N47S5-57 (ur_21_m1_held_tail…), N47S5-61 (ur_21_a_late_joined…)
# none: ur_25_a_burst_booked_on_an_ended_clock, ur_21_s2_a_short_head_is_not_held, ur_21_s2_c_and_d_joined
grep -rn "ur_23_a_clock_end_cancels\|ur_25_a_burst_booked_on_an_ended\|ur_25_an_end_booked_after\|ur_21_m1_held_tail\|ur_21_s2_a_short_head_is_not\|ur_21_s2_c_and_d\|ur_21_a_late_joined" design crates KNOWN_LIMITATIONS.md plan/maintenance/tools plan/phase7/tools
```
Design hits: `design/07:176` (m1, late_joined), `design/18:266` (m1, late_joined), `design/18:543` (UR-25 row). Nothing in KNOWN_LIMITATIONS.md.

**Row table (existing rows touched).** P453, N47S4-41, P569 (control.rs / tx.rs); the `held` rows move to `list.held` with the same mutation; none changes behaviour.

| Row | Today | After |
|---|---|---|
| P453 (control.rs, `if let Some(to_tx) = self.to_tx.get(k)`) | Stop send for a later stream | pushes the halt into stream 0's list instead of stream `k`; filter `ur_26_a_stop_of_a_later_transmit_stream_ends_only_its_burst` |
| N47S4-41 (`let _ = to_tx.send(TxCmd::Stop);`) | clears the stream's clock after the send (it is a clock-clearing mutation, not a halt-order one) | stays a clock-clearing mutation: the new text is a statement after `drop(streams)` in the Stop arm, `lock(&self.core.streams).stream_mut(stream).clock = None;`, taken after the section's guard is dropped (no nested streams lock); filter `ur_23_a_burst_after_a_stop_targets_the_one_clock` (runs `control.book(Stop)` then a booking through `book_burst`) |
| P569 (tx.rs `TxCmd::Stop => self.stop_now("UR-26: cancelled by Stop", …Stop)`) | Stop's reason | two rows on uhd-control's `list.halt(CancelReason::Stop)`: P569 Stop → ProviderStop and new P569b Stop → ClockEnded (a ClockEnded reason would emit COMMAND_REJECTED, Stop and ProviderStop are note-only); filter `vh_8_tx_stream_stop_cancels_held_bursts` (parity.rs:1281; real Provider path: `Provider` Stop action → control → list → owner thread; it asserts the exact note text at :1303-1309 and no rejection event) |
| P570, P1302 (shutdown's `stop_now` call), P1199, P1206, P1117, N47S5-151, U18 (`self.held…`) | anchors in `Tx` | anchors in `Tx`, with `self.held` → `self.list.held` access; same mutation, same filters |
| P635, N47S5-23 (Mock `self.tx[k].held`) | | `self.tx[k].list.held`; same mutation |
| R07 | filter deleted test | filter `ur_22_underflow_close_failure_reopens_before_the_next_payload` |
| N47S5-57 | | kept; filter becomes Q14 `e1_q14_a_continuation_accepted_after_the_unread_check_keeps_the_burst_open` (§5) |
| N47S5-61 | | filter unchanged, test rebuilt (§4) |
| N47S4-37 | | filter unchanged, test rewritten |
| P245, P442 | | unchanged (`Running.to_tx`) |
| the 28 `book_burst` rows (N47S4-21, N47S5-19, N47S5-21, J315, C83-2, …) | | anchors unchanged: tokens move into the critical section; checked by `rules.py` |

After each sub-step `rules.py --base 4151197` must find every anchor once; a re-anchored row yields the same program.

## 7. New mutation rows (token anchors; each names its killer)

Anchor text is fixed at implementation; the mutation and killer are fixed here. `crate`/`test_bin` as the killer's.

| Id | Where | Mutation | Killer (`filter`) |
|---|---|---|---|
| E1-U1 | `Streams::end` | the `list.halt(ProviderStop)` push removed | `e1_q3_a_pre_origin_provider_stop_cancels_a_booking_with_its_own_note` |
| E1-U2 | `Streams::end` | halt reason `ProviderStop` → `Stop` | same (the note's text) |
| E1-U3 | `step` | the guard `!stream.list.halt_unread()` → `true` (the guard ignores halts) | `e1_q2_the_step_reads_a_stop_it_has_not_read_before_it_hands` |
| E1-U3b | `TxList::halt_unread` (ezsdr-radio) | the `any(..)` predicate → `false` | `txlist_halt_unread_ignores_bookings`, `e1_q2_…` |
| E1-U4 | `step` | `now` read again after the critical section (shadowing) | `e1_q2b_now_is_read_under_the_lock_that_checked_the_list` |
| E1-U5 | `book_burst` main section | the re-check of the uncut segment → `if false` | `e1_q12_a_booking_racing_a_provider_stop_cut_is_refused_at_receipt` |
| E1-U6 | `TxList::read` (ezsdr-radio) | `pop_front` → `pop_back` | `txlist_reads_in_receipt_order`, `e1_q1_…` |
| E1-U7 | control's Stop | `list.halt(CancelReason::Stop)` removed | `e1_q1_a_stop_is_read_between_the_booking_before_it_and_the_one_after_it` |
| E1-U8 | control's Stop | the halt push moved inside `if streams.book(..)` | a Stop the line refuses (only if reachable, §0.4; otherwise no row, reported) |
| E1-U9 | `TxList::read` Halt arm | `mem::take(&mut self.held)` → `BTreeMap::new()` | `txlist_a_halt_cancels_exactly_what_was_read_before_it` |
| E1-U10 | `book_burst` | `list.book(..)` moved before the Drop/refusal decision | `e1_q13_a_refused_booking_is_not_on_the_list` |
| E1-U11 | `shutdown` | the leading `read_list()` removed | `e1_q6b_shutdown_reads_the_list_before_it_cancels` |
| ~~E1-U12~~ | dropped | | Deleting the run loop's `read_list` call cannot be killed by a test that calls `pass()` (a different entry than `run`, tx.rs:279-324), and with the step's unread guard the deletion is behaviourally redundant: the next `step` reads the list. No row. |
| E1-U13 | `Streams::end` | the halt pushed inside `StreamState::stop_at` | `e1_q5_a_booking_waiting_when_the_repeat_is_abandoned_still_plays` |
| E1-U14 | `receive` | the `clock.is_none()` arm → holds the burst | `ur_22_underflow_close_failure_reopens_before_the_next_payload` |
| E1-K1 | Mock `stop_tx` | `self.read_tx(k, now)?` right after `list.halt(..)` (halt read before the cut) | `mr_25_a_stop_transmits_every_held_burst_before_it`, `mr_25_stop_hands_a_future_repeat_through_the_in_flight_window` |
| E1-K2 | Mock `stop_tx` | the `list.halt(cancel_reason)` push removed | `mr_25_a_stop_before_the_burst_with_a_new_value_cancels_it_and_keeps_the_value_of_record` |
| E1-K3 | Mock `handle_tx_burst` | `list.book(held)` after the `read_tx` call (booking read before it is queued) | `e1_mock_q10_a_booking_a_stop_and_a_booking_in_one_step` |

No row, with the reason: a halt pushed on a loss (the effect is the same note as Shutdown's), the Mock `advance` `debug_assert!(!list.unread())` (an assertion, cannot be broken by a token without breaking the build).

## 8. Which O1 tests already cover which sequence (all stay green unchanged)

| O1 test | Sequence |
|---|---|
| `o1_uhd_t6c`, `o1_uhd_t6d` | Q1 after a Stop (Stop read before R, one booking after a failing close) |
| `o1_uhd_t4c`, `t4d`, `t4e`, `t4a`, `t4b` | Q4, Q4b's Stop-before-boundary cases |
| `o1_uhd_t6a`, `t6b` | a booking held at a Stop |
| `o1_uhd_t6b2_…_known_divergence_85` | #85 pin; only its comment changes |
| `o1_uhd_t7a*`, `t7b`, `t7c` | Q5 (abandon) |
| `o1_uhd_t2*` | Q11 |
| `o1_uhd_t8_…_known_divergence_84` | #84 pin, unchanged |
| `o1_mock_t6c`, `t6a`, `t6b`, `t4c`, `t9*` | Mock Stop and later booking (Q10 partly) |
| `o1_oracle_*` | the oracle itself |

## 9. `design/` edits this step is allowed

- `design/18-uhd-radio.md:543`: the row of `ur_25_an_end_booked_after_the_first_read_…` — outcome cell text becomes "the clock that has not begun ends at its origin: the held burst is cancelled by `Provider::stop` with the note \"UR-26: cancelled by Provider::stop\", with no `COMMAND_REJECTED`, leaves…" (the implementer reports the exact cell after reading its tail).
- `*Test:*` lines naming deleted tests: `design/07:176` and `design/18:266` replace `ur_21_m1_held_tail_waits_for_a_join_received_before_its_txcmd_is_drained` by `ur_21_m1_joins_booked_early_continue_the_device_burst`; `ur_21_a_late_joined_burst_cannot_move_and_is_refused` keeps its name (rebuilt).
- `CancelReason::ClockEnded` doc (ezsdr-radio lib.rs:118-119): "a failed streamer reopen ended the burst's transmit clock (UR-22)".
- Sentences the change may make false, to be proposed in the report, not edited: UR-25's "ended at its origin … cancelled" wording where it names `COMMAND_REJECTED` for a Provider::stop cut (design/18:543 region), RM-16 text on "the cut waits for uhd-control's join" if any (grep `join` near design/07:170 at implementation), and any `design/` sentence that says uhd-tx "receives" a burst by message.

## 10. Order of work, and the gate between sub-steps

E1a: `TxList` + pure tests (green by itself), then UHD: list in `StreamState`, control/Stop/`Streams::end` pushes, uhd-tx `read_list`/`receive`/`stop_now`, `step` guard, the two probes, test plumbing, §4, rows U*. E1b: the Mock and its rows K*. Each ends: `cargo fmt --all`; `rdev.py -- cargo test --workspace`; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 4151197`.

## 11. Not covered, and why

- A refused Stop (U8) only if reachable (§0.4).
- Multi-stream interleaving beyond the existing `ur_26_a_stop_of_a_later_transmit_stream_ends_only_its_burst` (row P453): a Stop of stream 1 must not touch stream 0's list.
- The `now`-under-the-lock rule has one deterministic test (Q2b) through a probe that lies outside the section; the section itself cannot be probed.
- No test on a real thread race: the receipt/Stop interleavings are made by calling in order, and the one place a race matters (decision 4) is a probe hook on the test thread.

## 12. Amendments after Sol's review (`sol-plan.md`, FAIL; every finding adopted)

| # | Finding | Change |
|---|---|---|
| 1 | Major: the joined clause stays reachable | §5 rewritten: clause and row N47S5-57 kept; new killer Q14 via the `tx_step_guarded` probe (B@e booked at J(e) after the unread check, device time then J(e)+1 tick, so only the joined clause holds); the "remove it after a green copy run" idea and the claim that every split-receipt state disappears are withdrawn. The m1 deletion text in §4 points to Q14 |
| 2 | Q10 harness steps per Action | Q10 queues the three Actions with `radio.push` and steps once |
| 3 | Q5 killer ordering | new `e1_q5_a_booking_waiting_when_the_repeat_is_abandoned_still_plays`: Stop, B, abandon, then inputs; row E1-U13 now runs it |
| 4 | D24 rebuild promoted B to head | §4: rebuilt on a finite abandon (FailSend mid-send), B's joined state asserted before dispatch, then the refusal and no B samples; the refused-configure path is dropped |
| 5 | Q7 refusal with a Stop pending | `e1_q7_a_stop_received_while_a_configure_refuses_its_burst` added to the Q7 row |
| 6 | Mutation mapping | N47S4-41 stays a clock-clearing mutation placed after `drop(streams)`; P569 gets a sibling P569b Stop → ClockEnded; E1-U12 dropped (reason in the table); the table below was rechecked row by row: |
| 7 | Rewritten origin-end test | first pass: cancellation and EOB; second pass: clock and cut cleanup (Q3 existing row) |
| m | `now` before the lock | no row (orchestrator decision); the limit stays in §11 only |
| R1 | Re-check minors a-f | applied: Q5/E1-U13 via `report`; D24 rebuild time move and pre-dispatch check; origin-end test drops the first-pass EOB expectation; E1-U4b removed; stray text fixed; Q2b: the `tx_step_guarded` probe call is the first statement after the guard is dropped, so E1-U4's mutant (reads `now` after the probe) is killed by Q2b; §5's J(e)+1 time move is redundant but harmless |
| R2 | Orchestrator: guard on an unread halt | `step`'s guard reads `list.halt_unread()`, not `unread()`; `read_list()` is restored before `pass()` in the run loop (today's mpsc order). New pure test `txlist_halt_unread_ignores_bookings`; row E1-U3b. A booking-only or any-entry guard would change switch timing (a switch received but not yet read would be installed before its cursor passes the boundary): `o1_uhd_t4b_…`, `ur_21_s12_send_asap_switch_handed_past_is_installed_at_the_next_boundary` and `ur_23_stop_before_the_moved_boundary_cancels_the_switch` fail with it, so it is not used. Q2b's probe books nothing but the Run end; its killer for E1-U3 and E1-U4 stands |

Row recheck (behaviour changed, call path the filter runs): E1-U1/U2 Provider::stop → `Streams::end` → owner `read_list` via Q3 arms (a)/(b) (`pass`); arm (c) runs `shutdown`; E1-U3 `step` direct call (Q2); E1-U4 the Q2b probe in `step`; E1-U5 the Q12 probe in `book_burst`, `control.book`; E1-U6/U9 pure `TxList`, and Q1 through `read_list`; E1-U7 `control.book(Stop)` then `read_list` (Q1); E1-U8 only with a Stop the line refuses; E1-U10 `control.book` refusal arms, then `next_booking`; E1-U11 `shutdown()` called directly (Q6b); E1-U13 abandon through `tx.report(Underflow)` with B unread (Q5); E1-U14 `receive` via `give` after the failed reopen (:1564 test); E1-K1/K2/K3 Mock `step(u)` paths through `stop_tx`/`handle_tx_burst`; N47S5-57 Q14 (`step` → `close_unjoined`).
