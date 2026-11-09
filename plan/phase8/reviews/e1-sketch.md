# E1 shape sketch: note 49 D′, stage E1 (receipt-ordered list)

Base: main `4151197`, read-only. Nothing was built or run. Claims about today's code come from reading it (path:line at HEAD). Claims about what E1 does are INFERRED.

## 0. The shape in two lines

Each transmit stream gets one generic `TxList<P>` in `ezsdr_radio::staging`. It sits beside `Bookings`, in the same per-stream state and under the same lock. It holds a FIFO of receipts (accepted bookings and halts, each appended in the critical section that decides it) and a start-keyed `held` map that only the executor writes. uhd-tx reads the FIFO one entry per critical section, exactly as `command` reads the mpsc today. MockRadio appends and reads at each receipt, which is its executor's turn. Only `TxCmd::Shutdown` stays on the mpsc.

## 1. Types and where they live

```rust
// crates/ezsdr-radio/src/staging.rs, beside `Bookings` (staging.rs:209-216)
pub struct TxList<P> {
    unread: VecDeque<Arrival<P>>,   // receipts the executor has not read, in receipt order
    pub held: BTreeMap<i64, P>,     // read and not yet taken, by first sample; its one writer is the executor
}
enum Arrival<P> { Booking(P), Halt(CancelReason) }
pub enum Read<P> {
    Booking(P),                                                 // the executor holds it in `held`, or refuses it
    Halt { reason: CancelReason, cancelled: BTreeMap<i64, P> }, // `held` taken whole (mem::take): what was read before the halt
}
impl<P> Default for TxList<P> { .. }              // by hand: StreamState derives Default (core.rs:358)
impl<P> TxList<P> {
    pub fn book(&mut self, booking: P);           // receipt; same critical section as Bookings::accept + book
    pub fn halt(&mut self, reason: CancelReason); // receipt: Stop -> Stop, Provider::stop's end -> ProviderStop, Mock loss -> Loss
    pub fn read(&mut self) -> Option<Read<P>>;    // executor: the oldest receipt, O(1)
    pub fn unread(&self) -> bool;
}
```
- **UHD:** `StreamState.list: TxList<Held>` sits beside `busy` and `playing` (core.rs:383-385). `Held` (tx.rs:29-44) stays the payload, built by uhd-control as today (control.rs:600-612).
- **Mock:** `TxStream.list: TxList<HeldBurst>` replaces `held` (lib.rs:208) and sits beside `busy` (:216). `HeldBurst.order` stays in the payload: it is the instance-wide arrival order that the step's same-tick sort reads (lib.rs:1956, 1959).
- **Sequence number:** the entry's position in `unread`, set by `push_back` under the lock (the UHD streams mutex; on the Mock, `&mut self`). There is no stored counter, because nothing in E1 would read it. A halt cancels exactly what was read before it, since nothing received after the halt can be in `held` yet. Add `seq: u64` only if E2a finds a reader for it (INFERRED none: §5.4.4 4 applies inputs at receipt).
- **`Bookings` is unchanged.** It keeps holding regions; `TxList` holds the payload. The fact "which bookings are alive" still has two copies, as today (note 49 §4, P1: the record versus uhd-tx's `held`), but both now change in one critical section. E2a merges them into the state machine.

## 2. Who calls what, under which lock, in which order

**UHD receipt, `book_burst` (uhd-control)**
1. Lock: the clock and channels of the uncut last segment (control.rs:415-426, unchanged). Unlock.
2. No lock: the domain, waveform, decode, finite, repeat and metadata checks (:427-454); `Arc::new(samples)`; `staging.tx_snapshot(k)` gives `settings`. The snapshot moves up from :596-597; `staging` belongs to uhd-control alone.
3. **One critical section:**
   - `receipt = core.now()` (moved from :457).
   - Check again that the last segment is uncut (misfit (d) in §5). If it is cut, refuse with :422's reason.
   - Decision instant and late policy (:456-503). A Drop, PlanViolation or Err drops the guard first, then emits and returns as today.
   - :509-573 unchanged.
   - On acceptance, `stream.list.book(Held { k: start, .. })`.
4. No lock: refusal events, the switch row and `TIME_ERROR{send_asap}` (:574-595). Nothing is sent to uhd-tx any more.

**UHD Stop (control.rs:309-323):** the same critical section, plus `streams.stream_mut(stream).list.halt(CancelReason::Stop)`, pushed whether or not the line booked the Stop (today's `TxCmd::Stop` went out either way, :320-322). The send is removed.

**`Provider::stop` (mod.rs:693-699):** the critical section is unchanged. `Streams::end` pushes `list.halt(CancelReason::ProviderStop)` on each transmit stream next to `stop_at` (core.rs:603-605). It does so only when `!lost`, and only on the first end (after the early return at :599-601).
- A loss pushes nothing, which matches today: the owner idles, and Shutdown cancels with ProviderStop (`vh_8_loss_cancels_held_bursts_without_rejection`, parity.rs:1386).
- The push must not go into `StreamState::stop_at` (core.rs:400-406), because abandon also calls it (tx.rs:999).
- After this, join uhd-control and send `TxCmd::Shutdown` (mod.rs:704-706), unchanged.

**uhd-tx (tx.rs:285-300)**
```text
loop { try_recv: Shutdown | Disconnected -> shutdown();  read_list();  if lost { sleep; continue }  if !pass() { sleep(IDLE) } }
read_list(): loop { let r = lock(streams).stream_mut(key).list.read();   // the guard is dropped here
                    match r { None => break, Booking(h) => receive(h), Halt{reason, cancelled} => stop_now(reason, cancelled) } }
```
- **`receive(held)`** is today's `command(TxCmd::Burst)` (tx.rs:329-340):
  - `tile_repeat`;
  - if `clock` is None, `cancel_unstarted` (ClockEnded);
  - a switch goes to `install`, which locks by itself (:353), or gets COMMAND_REJECTED "not taken";
  - otherwise lock and `list.held.insert(held.k, held)`.
- **`stop_now(reason, cancelled)`** is today's :1152-1157: `end_open(true)` (the device close, outside the lock), then one report per cancelled booking, with a fixed text per reason.
- **`step()` (:444-452):** one critical section reads `list.unread()`.
  - If something is unread, it releases the guard, calls `read_list()` and returns true.
  - Otherwise it reads `now = core.now()` and runs `busy.advance`. The `now` read moves into this critical section from outside the lock (:450).
  - Effect: every hand-over in a turn is bounded by an instant read while no halt was waiting. A later halt has `t ≥ now` (`fetch_max` at authority.rs:144, plus lock order), so its `B − 3 = at_or_after(t + W) ≥ window_cut`. This closes note 49 §4's second window.
- **Other `self.held` uses** become short critical sections on `list.held`: :465-531, 573, 584, 601, 936, 967-969, 991. There is one writer, so these are about placement only, not consistency.
- **`shutdown()` (:1292):** `read_list()` first, so a queued halt keeps its own reason; then `stop_now(ProviderStop, take(list.held))`; then the rest as today.
- **Outside the lock:** device calls, `tile_repeat`, events and `rec` writes (lock order streams → rec, core.rs:255-256).
- **Pitfall:** `while let Some(r) = lock(..).list.read() { .. }` keeps the guard alive for the whole loop body, in edition 2024 too (Cargo.toml:20). Since the body locks again, that deadlocks. Bind the result with `let` first.

**MockRadio (single thread; the receipt time is the step's `u`, lib.rs:2437-2438)**
- **`handle_tx_burst` (:1760-1761):** `busy.book(end); list.book(held); read_tx(k, now)`. `read_tx` puts a Booking into `list.held` by its start. A Halt becomes one `record_cancelled_burst` per entry, with the texts unchanged ("MR-25: cancelled by stop", DEVICE_LOST).
- **`stop_tx` (:1159-1209):** `list.halt(cancel_reason)` comes first. The MR-25 cut (`tx_handed_end`, :1393-1416) and `transmit_until_cut` (:957-970) read and take `list.held` as today. The final loop (:1200-1204) becomes `read_tx`.
- **`held` → `list.held`:** in `schedule_wakeup` (:852, 863), `emit_tx_until` (:1818) and `advance` (:1952, 1998). `advance` starts with `debug_assert!(!list.unread())` on each stream.
- The block output (MR-15) and `tx_handed_end` stay as they are until E3.

## 3. Removed, and what replaces each

| Removed | Replaced by |
|---|---|
| `TxCmd::Burst`, `TxCmd::Stop` (tx.rs:71-77) | `list.book` / `list.halt` at receipt; `Shutdown` stays |
| `Control.to_tx` and its `Control::new` parameter (control.rs:144-145, 175; mod.rs:622) | nothing: Control writes the list |
| `Tx.held` (tx.rs:221, 269); Mock `TxStream.held` (lib.rs:208, 230) | `list.held` |
| `Tx::command` (tx.rs:327-345) | `read_list` + `receive` + `stop_now(reason, cancelled)` |
| `end_clock`'s cancel loop (tx.rs:1164-1166) | nothing. It cannot be reached: the Run end's halt is queued in the critical section that cuts the clock (core.rs:603-610), the step reads it before the cut check (:455), and a receipt after the cut is refused |
| the domain-mismatch arm (tx.rs:331) | `clock` None only, plus `debug_assert` on the domain. Receipt already refuses another domain (control.rs:427-429), and one clock is registered per stream (UR-13) |
| test `ur_23_a_clock_end_cancels_every_held_burst` (tx.rs:4043) | deleted: the state it sets up cannot be reached (row above) |
| test `ur_25_a_burst_booked_on_an_ended_clock_is_cancelled_when_received` (tx.rs:4061) | deleted. The reachable case (no clock after a failed reopen) is pinned at tx.rs:1641-1651 |
| `ur_25_an_end_booked_after_the_first_read_cancels_the_held_burst_at_the_origin` (tx.rs:1965-1990) and its spec row (design/18-uhd-radio.md:543, "cancelled with `COMMAND_REJECTED`") | rewritten: "UR-26: cancelled by Provider::stop" with no event; a second pass ends the clock; assert `tx.clock.is_none()`, which keeps N47S4-37 killed |
| split-receipt tests (record now, hand-over later; this split no longer exists in E1): `ur_21_m1_held_tail_waits_for_a_join_received_before_its_txcmd_is_drained` (tx.rs:2953; named in design/07:176 and design/18:266), `ur_21_s2_a_short_head_is_not_held_for_a_piece_handed_after_it` (:3028), `ur_21_s2_c_and_d_joined_behind_a_refused_head_are_late_at_dispatch` (:3061, which hands over in non-receipt order), `ur_21_a_late_joined_burst_cannot_move_and_is_refused` (:3539) | book each atomically and keep the assertion where the outcome holds. Otherwise rebuild on a reachable path (D24 through an abandon) or delete with the argument. `txp_record`/`txp_give` (:4798-4811) → `txp_book` |
| test plumbing: `tx.held.insert` / `.is_empty()` / `.contains_key`; `tx.command(TxCmd::Burst/Stop)`; `to_tx.try_recv()` in control.rs (:697, 995, 1722 rigs) | `hold(&tx, h)` (held without a record: reachable between a Stop's receipt and its read), `held_keys`, `give` (book + `read_list`), `halt` (Stop + `read_list`), `next_booking(&core)`. `drain_tx` (:2099) keeps its name and signature with the body `tx.read_list()`, so the O1 tests need no change |
| `CancelReason::ClockEnded` doc (crates/ezsdr-radio/src/lib.rs:118-119) | "a failed streamer reopen ended the burst's transmit clock (UR-22)" |
| mutation rows P453, N47S4-41 (Stop send), P569 (the Stop's reason), P1199 (stop_now's loop), P1206, P1117, N47S5-151 (held reads), P635, N47S5-23 (Mock), and P570/P1302 if the text of shutdown's call changes | retargeted with the same mutation: P453 pushes into stream 0's list; N47S4-41 goes before the halt push; P569 goes on uhd-control's `list.halt(CancelReason::Stop)` |
| row N47S5-57 (`continuation_open`'s joined clause, staging.rs:284; its only killer is the m1 split test) | INFERRED unreachable in E1: when the step reaches `close_unjoined`, nothing is unread and a booking at `e` is already in `held`, so the step continues the burst at tx.rs:495-496. If that holds, remove the clause and the row; otherwise write a killer |
| (new rows) the `Streams::end` halt push; the step's unread guard; the receipt's cut re-check | killed by Q3, Q2 and a probe test. Add one pure `TxList` test in crates/ezsdr-radio/tests/staging.rs |

Not found in any spec: `TxCmd` and the mpsc. Search: `grep -n 'mpsc\|TxCmd' design/07-radio-model.md design/09-mock-radio.md design/18-uhd-radio.md` returns nothing.

Left to E3: `Bookings::close` (staging.rs:288-292). It becomes redundant once both instants are read under the lock (§5.4.10), but its return value still feeds the `tx_close_past_record_front` statistic (tx.rs:547-552), which E3 removes.

The 28 rows anchored inside `book_burst` stay valid only if the code moves into the critical section with its tokens unchanged. `rules.py` must still find each of them exactly once.

## 4. Operation sequences

| # | Sequence | After E1 | Today | Relies on |
|---|---|---|---|---|
| Q1 | A booked; Stop@t; R booked with start ≥ B; then the owner's turn (M3) | Read order A, Stop, R. A is cancelled (Stop note, no event); the open burst closes at its handed cursor; R plays from its start | same (mpsc FIFO, tx.rs:285-292) | front +1 and the Stop's c+3: VERIFIED (note 49 §8, 2026-11-06 / 11-09 (d)); W: INFERRED |
| Q2 | The owner reads the list; Stop@t arrives; device time moves to t+δ; the owner steps | The step finds the halt unread and reads it first. Nothing at or after B−3 is handed; a booking at B plays on time | That turn hands up to `at_or_after(t+δ+W) > B−3` (INFERRED, note 49 §4) | ordering only |
| Q3 | Stop@t1 before the origin; X booked with start ≥ B; Provider::stop@t2 ≤ origin; the origin is reached | Stop note for what was read before the Stop; X gets a "Provider::stop" note; no COMMAND_REJECTED; the clock ends at its origin (timeline.rs:460-463); nothing is sent | X becomes ClockEnded COMMAND_REJECTED (tx.rs:1164-1166) or ProviderStop (:1293), depending on which comes first | none |
| Q4 | A repeat R is playing; switch S arrives; the owner's turn | S is installed at its boundary from the handed cursor (tx.rs:350-374) and held at `S.k` | same | none |
| Q4b | Q4 with a Stop arriving after S, before the owner's turn | S is read first. The record's halt already took the pending switch (staging.rs:381), so install fails: COMMAND_REJECTED "not taken" (tx.rs:333) and the row is `cancelled`. Then the Stop closes R | same (E2a removes this event, §5.4.7) | none |
| Q5 | Underflow abandons R at cursor c while booking B (start ≥ c) sits in the list | `stop_at(c, 0)` drops B's chain from the record (staging.rs:386, tx.rs:999); B stays in the list and plays | same: E1 does not fix this (note 49 §4; E2a does) | none |
| Q6 | A, Stop, C, Provider::stop's end, all unread at the owner's turn | A gets a Stop note; C a Provider::stop note; Shutdown finds nothing left | same reasons (today C goes at Shutdown) | none |
| Q7 | X is taken for configure; a Stop arrives during it (probe) | Configured: X goes back into `held`, and the next read cancels it (note). Refused: COMMAND_REJECTED with X's id; the Stop cancels nothing of X (m1) | same (tx.rs:580-609, then the mpsc) | none |
| Q8 | R plays past the origin; Provider::stop@t with no Shutdown yet; time moves on | The owner's next turn reads the end: R closes at its handed cursor ≤ B−3, and held bookings get notes | R is handed until Shutdown, which comes after the uhd-control join (mod.rs:700-706, POLL 1 ms at control.rs:35). INFERRED past RM-16's "no later than B" (design/07:170) | none |
| Q9 | An F8 reopen fails (not lost, no stop); then B is accepted at receipt | Read with `clock` None: COMMAND_REJECTED "UR-25: its transmit clock ended…" (ClockEnded) | same (tx.rs:339; pinned at :1641-1651) | none |
| Q10 | Mock, one step `u`: A booked, Stop, R booked | A is cancelled when the Stop is read ("MR-25: cancelled by stop"); R is held and opened at its start | same (lib.rs:2437-2442) | none |
| Q11 | A join received at J(e), and one received a tick later (both instants read under the lock) | accepted / refused | same (`ur_21_m1_receipt_at_the_deadline_is_decided_by_the_record`, tx.rs:2979) | J(e): VERIFIED (§8, 11-09 (d)) |

## 5. Choices, risks, and where note 49 does not fit the code

**Rejected shapes**
1. **One start-keyed map written at receipt, plus a read watermark.** The step, `abandon` (tx.rs:991) and `cancel_held_switches` (:967) would see a switch before it is installed, which changes Q4b's outcome to a Stop note. Every executor query would also need a "read" filter: one lifecycle fact spread across queries.
2. **The list inside `Bookings`.** `Bookings` is cloned under the lock for the value rule (control.rs:287, lib.rs:1917), derives Eq, and is non-generic with 78 pure tests. A waveform payload does not belong in a region record.
3. **Halts left on the mpsc.** Ruled out by note 49 M3 (§5.4.4 6).
4. **A second lock for the list.** The record and the list must change in one critical section; that is S1's window.
5. **uhd-tx keeps a private `held`, and only the FIFO moves.** Smaller, but the decided E1 removes the private map, and moving it later would land in E2a, the riskiest stage.
6. **A stored u64 counter.** Nothing in E1 reads it.

**What this shape makes harder later.** Nothing has to be redone:
- E2a drops `Read::Booking` once switches install at receipt (§5.4.5) and halts become transitions applied at receipt. `held` then gains issued/sent state per booking, which #85, Q5 and C2 need. The receipt sites become `receive`, and `read_list` becomes the turn's `next`/`complete`.
- E2b removes the configure put-back (tx.rs:601).
- E3, C1 and C2 do not conflict: segmentation stays in uhd-tx's `open` and `chain`.
- core.rs now names `tx::Held`, as control.rs:29 already does.

**Where note 49 does not fit the code**
- **(a) `cancel_unstarted` stays reachable.** §12.1 says it becomes unreachable; Q9 is the counterexample. `halt_failed_reopen` clears only uhd-tx's own clock (tx.rs:959), while the stream's clock and the uncut line still admit bookings (control.rs:415-426). This is consistent with "ClockEnded only for a reopen failure". Keep `cancel_unstarted` for `clock == None`; only the `end_clock` loop and the domain-mismatch arm go.
- **(b) A third test pins ClockEnded at a Provider::stop cut.** §12.1 names two tests, but `ur_25_an_end_booked_after_the_first_read…` (tx.rs:1965) and its spec row (design/18:543) also do.
- **(c) Putting Provider::stop on the list makes its transmit end earlier (Q8).**
  - This is user-visible: the last burst is shorter, and a booking that would have started during the join window is cancelled.
  - §5.4.7 does not list it. Making Provider::stop a list input implies it; the only way to keep today's timing is to leave the Run end off the list, which reopens the ClockEnded race.
  - It is what RM-16 already requires of Provider::stop: "closes at its current handed cursor, no later than B" (design/07:170).
  - No test pins today's late end: the manual-clock rigs freeze device time during the join (INFERRED).
- **(d) A race at receipt.** The clock/cut check (control.rs:415-426) is a separate critical section, so a Provider::stop cut can slip in between it and the main section. The booking then queues behind the Run end, and if it is read after `end_clock` it gets ClockEnded again. Fix: E1 re-checks the cut inside the main section, so such a booking is refused at receipt, as any booking after a cut already is. This affects only that race.
- **(e) Split-receipt tests and N47S5-57.** See §3. Their scenario stops existing; in the m1 test's case that was the point of the test.
- **(f) The step's instant under the lock.** The §12 stage table puts "issue now under the lock" in E2a. E1 still needs the step's `now` inside the guard's critical section to close §4's second window, which §4 itself assigns to E1 together with §5.4.3. It is one line, and E1 should take it.

**#85 is not fixed by E1.** The burst that was dispatched but never sent lives in uhd-tx's `open` (tx.rs:530), outside the list, and `end_open` drops it with no record and no note (tx.rs:842-845). The pin `o1_uhd_t6b2_…_known_divergence_85` stays as it is; only its comment changes from "E1/E2a" to "E2a". E2a fixes it structurally: unissued bookings stay in the state machine, and the Stop cancels them (§5.4.8). A local two-line note in `stop_now` would be thrown away by E2a.

## 6. Owner decision

No owner decision is needed for the shape. Beyond the ProviderStop reason, two changes are user-visible:
- Q8: Provider::stop's transmit end.
- Q2: the §4 window.

Both make UHD follow RM-16's existing text (design/07:170). The E1 brief and record should name them so reviewers do not count them as regressions. If the orchestrator reads Q8 as a contract change, it needs one yes/no from the owner: should Provider::stop's transmit end come at uhd-tx's next turn (as RM-16 says) instead of after the uhd-control join? Recommendation: yes.
