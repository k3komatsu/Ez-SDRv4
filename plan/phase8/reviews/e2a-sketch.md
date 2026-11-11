# E2a design sketch: note 49 D′, stage E2a (operations and results as one protocol)

AGENTS §9 step 0 (design). Draft by an Opus subagent, read-only. Nothing was built, run or edited in the tree.

**Base.** `main` = `d9fddf3`. Step F (the time-based front, `CLOSE_GAP`, `STOP_FRONT`, `closed_short_gap`) landed on `main` while this draft was being written (`git log main`: `d9fddf3` on top of `698df19`). Every `path:line` below is at `d9fddf3`.

**Marks.**
- VERIFIED: I read the code or the record at the cited line.
- INFERRED: my reasoning, not run.
- Line counts are estimates unless they come from a script I ran (searches in the appendix).

**Step F matters in one place.** The Stop bound is `B = at_or_after(t + W) + STOP_FRONT` (`crates/ezsdr-radio/src/staging.rs:734-736`, `STOP_FRONT = 5` at `:90`), and an abandoned repeat keeps cursor + `STOP_FRONT` (RM-15, `design/07-radio-model.md:163`). Note 49's literal "B − 3" is now "B − STOP_FRONT". The brief must use the constants, not the numbers.

## Owner decisions (2026-11-10, orchestrator's record; these override the sketch where they differ)

1. **Direction**: the recommended shape — one `TxBook<P>` per transmit stream in `ezsdr_radio::staging` owning bookings, halts, the switch, `playing`, the open booking and the one issued send; uhd-tx keeps device facts; sub-steps E2a-1 (one owner, no behaviour change), E2a-2 (results, #80, #85), E2a-3 (switches, #84, #86, stale grid). The lean and full shapes are rejected (§2).
2. **N2 = (b1)**: a switch whose repeat's first sample uhd-tx has not yet issued is refused at receipt (`COMMAND_REJECTED` in the submit reply, `switch()` raises `Rejected`, no row). The #84 oracle answer changes to the receipt refusal in the same commit as RM-37's text. (The owner checked: `switch()` does not wait for the hardware; it learns a refusal at once; an accepted switch can still be cancelled later by RM-37's rules.)
3. **#86 — changed from the sketch's "None"**: a switch whose boundary `b` is later than the boundary `at` alone gives (the first boundary at or after `at`), because the receipt instant plus W plus the device lead pushed it (RM-37's second term), records `requested_target = at`, on both Providers. A switch at the rule's own boundary records none. Reason (owner's question): under RM-37 a switch whose `at` is exactly a boundary is pushed to a later one when received less than W + lead before `at`; when that is not late by RM-14 it has no TIME_ERROR today (INFERRED), so "None" would hide the deviation. Whether that push should also emit TIME_ERROR is a separate question, not in E2a.
4. **`moved_to`, `Switch.installed`, `Bookings::refuse`, `SwitchOutcome::Refused` are deleted** (text: MR-42, UR-38, UR-30's row, note 49 §7's RM-37 line).
5. **(d1)–(d5) as recommended**: tokens on Sends only in E2a; the stale grid gets an issue and is fixed in E2a-3 as a named behaviour change; #80 includes the value rule (busy end capped only when no other booking remains); the lock-wait stat `tx_lock_wait_max_ns`; the new receipt refusal is added to KL-13.
6. #84 was reopened by the orchestrator (it had been auto-closed by the O1 commit's message).
7. **After the Fable 5.2 design review** (`fable-design.md`, verdict "direction holds with changes", 2026-11-10/11): all findings F1–F9 and alternatives A1–A5 are adopted. The owner changed two decisions:
   - **#86** is limited to **timed** switches: a timed switch whose boundary the receipt instant (+ W + lead) pushed past the boundary `at` alone gives records `requested_target = at`; an untimed switch (RS-19; Python's `switch(x)` outside `aligned`) takes the same boundary as a timed one at the same instant and records none (RM-37's last sentence says so).
   - **(d5)**: the new receipt refusal gets its **own KNOWN_LIMITATIONS entry** (AGENTS §2, one entry each), not a KL-13 line.
   - A1: one switch-cancellation transition in the book (row settled, `playing` cleared, payload removed from `held` **and** `unread`, one note), fixing F1 (a cancelled switch's unread payload would sound). A2: E2a-2 splits into E2a-2a (the `open` move and the send split, no behaviour or trace change) and E2a-2b (#80, #85, the late-underflow rule). A3: no `OpId`/`ops`/`issued`; one `inflight: Option<Send>`. A4: the scoped abandon as a pure function, table-tested against today's `halt(c, c+STOP_FRONT)` + `stop` when no other chain follows. A5: `broken` with no open device burst changes nothing (named change).


## 0. The recommendation in brief

1. **One owner type.** One `TxBook<P>` per transmit stream in `ezsdr_radio::staging`. It replaces, on UHD, `StreamState.busy`/`.list`/`.playing` and `Tx.open`; on the Mock, `TxStream.busy`/`.list`/`.playing`/`.open`.
   - It owns the bookings, the halts, the switch, `playing`, the open booking and the one issued send.
   - uhd-tx keeps only device facts: `TxChain` (device burst, held tail, front, recovery), the report correlation, the `BurstTracker`, the clock and the streamer.
2. **Results come back to the book.** uhd-tx plans each send under the streams lock and registers it (`issue`), makes the device call(s) outside the lock, and hands the outcome back (`complete`). The book alone turns a short or failed outcome, or a broken report, into an abandon. That abandon ends only the abandoned booking and the switch that replaces it.
3. **Switches are decided at receipt.** A switch is taken at receipt, or refused there. Nothing moves its boundary later. The UR-38 move, `moved_to`, `installed`, `Bookings::refuse` and `refused_past_boundary` go.

**Finding that changes the brief.** The note's N2 rule and the #84 oracle answer cannot both hold.
- The N2 rule refuses a switch whose repeat has no issued cursor (note 49 §5.4.5 table, §12.1).
- The #84 oracle answer is "cancelled" (`crates/ezsdr-acceptance/tests/oracle/mod.rs:314-316`), yet #84's own sequence is exactly an N2 case: the repeat has not started on the device when the switch arrives.
- So question (b) below also decides what the #84 pin flips to.

**Three side findings for the orchestrator.**
- **#84 is closed on GitHub, but its pin is still live.** It was closed on 2026-11-09T17:09Z, `stateReason COMPLETED`, with no comment. That timestamp is the O1 commit `4151197`, whose message says "pinned so that they go red when fixed:\n#84", which GitHub reads as a closing keyword (INFERRED from the timestamp and the text). It should be reopened.
- **A repeat moved at dispatch leaves `playing` on its old grid.**
  - `dispatch_start` moves the record's chain (`tx.rs:1124`) but never touches `StreamState.playing`. `playing` is set at receipt with the receipt start (`control.rs:577`; its only writers are `control.rs:577` and `tx.rs:382,1033`). VERIFIED.
  - So a later switch takes its boundary from the stale origin (`control.rs:554`) and cuts the moved repeat mid-period. INFERRED: no test covers it.
  - The mechanism is `planned-state-read-as-actual`, the P5 copy. E2a removes it (§1.5 E); it needs an issue.
- **The #86 comment's reading is wrong.** It says the dispatch-move twin "would give the chosen boundary b0". But `held.target` is the switch's `at` (`control.rs:604`, `target: at.time_point`), and the twin writes `.or(Some(held.target))` (`tx.rs:1137`). So the twin would give `at`, which is what the oracle also says (`oracle/mod.rs:320-322`). VERIFIED. Under E2a the question is moot anyway (question (a)).

## 1. Shape

### 1.1 Types and home

Home: `crates/ezsdr-radio/src/staging.rs`, beside `Bookings` (`:291` ff.) and `TxList` (`:243-285`). The pure tests go in `crates/ezsdr-radio/tests/staging.rs`. No new crate, no new module file: the switch, region and list types already live there.

```rust
/// One transmit stream's bookings, halts, repeat switch and issue state (RM-15, RM-16, RM-37, UR-21–UR-23): their one owner, under the
/// Provider's stream lock (UHD) or `&mut self` (Mock). The executor never writes `record` itself; every executor write is a transition here.
pub struct TxBook<P> {
    pub record: Bookings,          // unchanged type; receipt keeps reading and writing it as today (placement, accept, book, prune)
    list: TxList<P>,               // E1's receipt FIFO and `held` map, unchanged
    open: Option<Open<P>>,         // moved from uhd-tx `Tx::open` (tx.rs:219) and the Mock's `TxStream::open` (lib.rs:210)
    playing: Option<Playing>,      // moved from `StreamState::playing` (core.rs:387) and the Mock's (lib.rs:218)
    inflight: Option<(OpId, Send)>,// the one issued send whose outcome is not yet applied (UHD; the Mock's is applied at once)
    issued: Option<i64>,           // c_I: the exclusive end of the last issued payload on this clock
    ops: u64,
}
pub struct Open<P> { pub booking: P, pub next: i64, pub first: bool }   // today's `tx::Open` (tx.rs:77-81) and the Mock's `OpenBurst`
/// The repeat a switch replaces (RM-37). `started`: its first payload sample has been issued (UHD: set by the first `issue` of the
/// booking at `origin`); a stepped Provider passes `true` at acceptance, since it never moves or drops a booking after receipt (MR-17).
pub struct Playing { pub id: ActionId, pub origin: i64, pub period: i64, pub started: bool }
pub struct OpId(u64);
/// One planned send of the open booking (UR-22, UR-23): `[k0, k0 + count)` issued (a held tail included), `device_count` what the device
/// is given (less a held tail, plus padding), whether the booking's held tail before it goes first, and whether it ends the booking.
pub struct Send { pub k0: i64, pub count: i64, pub device_count: usize, pub tail_first: bool, pub ends: bool }
pub enum Outcome { Accepted(usize), Failed, TailShort }        // per op; the executor folds its one or two device calls into one
pub enum Settle { Sent, Abandon { cursor: i64 } }               // the device-level close the executor makes next
pub trait Booked { fn start(&self) -> i64; fn len(&self) -> Option<i64>; fn switch(&self) -> bool; fn id(&self) -> Option<ActionId>; }
```

`Booked` is implemented by UHD's `Held` (`tx.rs:29-44`) and the Mock's `HeldBurst`. A trait costs four one-line methods, which is less churn than wrapping E1's `TxList<P>` payload in a header struct. The book needs only these four facts about a payload.

**Kept, not moved, in E2a.**
- `TxChain` (`tx.rs:119-202`): the device burst, its next sample, the held tail, the front and recovery. Nothing at receipt reads it in E2a, because the record still predicts the front (P3) until E3 shares the segmentation. Moving it now would give the Mock fields it never uses.
- Segmentation (`send`'s count, padding, hold and first-send rules, `tx.rs:645-855`): E3's job.
- Configure and reopen turns (`tx.rs:614-641`, `:923-995`): E2b's job. Their record writes become book transitions (`configure_refused`, `clock_ended`) with today's semantics.
- The Mock's block output (MR-15) and Stop-cut model (`tx_handed_end`, `lib.rs:1406`): E3's job.

**Transitions**, each a method under the lock. The executor-side ones replace the write-backs listed in §1.4.

| Transition | Caller | Replaces |
|---|---|---|
| `book(booking, end_root, repeat_playing)` | receipt, both | `busy.book` + `list.book` + `playing = Some` (`control.rs:575-605`, `lib.rs:1773-1781`) |
| `receive_switch(id, booking, b, now_k, cursor, started) -> Result<(), SwitchRefusal>` | receipt, both | `busy.switch` + install + row data + `list.book` + `playing` (`control.rs:552-605`, `lib.rs:1712-1781`, `tx.rs:371-395`) |
| `stop(t_k, stop_k, bound, reason)` | receipt, both | `StreamState::stop_at` + `list.halt` (`core.rs:432-440`, `control.rs:318,323`, `core.rs:636-640`; `lib.rs:1177-1186`) |
| `advance(now_k)` | executor, both | `busy.advance` (`tx.rs:478`, `lib.rs:1817`) |
| `read() -> Option<Read<P>>` | executor, both | `list.read()`; a `Halt` also takes an open booking with nothing issued (#85) |
| `take(k)` / `untake()` | executor, both | `self.open = Some(..)` / `self.hold(open.held)` (`tx.rs:564`, `:499`; `lib.rs:974-986`) |
| `dispatched(verdict) -> Result<(), Refused>` | executor, UHD | `move_untimed` / `move_start` / `drop_piece` / `cancel_switch` (`tx.rs:1106,1124,1126,1145,1146`); `playing` follows a moved repeat |
| `close_unjoined(e, now, join_by) -> Option<Option<i64>>` | executor, UHD | `continuation_open` + `close` (`tx.rs:578,581`) |
| `issue(send) -> OpId` | executor, both | new: one in flight; sets `issued`; sets `playing.started` at a booking's first issue |
| `complete(op, outcome) -> Settle` | executor, both | `open.next += count; first = false` (`tx.rs:811-813`); short or failed outcomes give the abandon (`tx.rs:749-785` → `abandon`) |
| `broken(device_next) -> Option<i64>` | executor, UHD | the underflow, sequence and expired-timeline abandons (`tx.rs:1300-1303`, `:502-506`, `:524-527`) |
| `configure_refused(k, len)`, `clock_ended()` | executor, UHD | `drop_piece` (`tx.rs:636`), `cancel_switch` (`tx.rs:990`); E2b folds them into `complete` |

### 1.2 Who calls what, under which lock, in which order

**UHD.** The streams mutex is never held across a device call (UR-25, `core.rs:256`; `ur_25_the_line_lock_is_free_inside_every_transmit_device_call`). Lock order stays streams → rec. Events and `rec` rows are written after the streams guard is released, as E1 already does in `book_burst` (`control.rs:473-616`).

| Section | Thread | Under the streams lock | Then, outside it |
|---|---|---|---|
| receipt (`book_burst`, Stop, `Provider::stop`'s end) | uhd-control | E1's one section (`control.rs:473-611`), with the `book.*` receipt transitions in place of `busy.*`/`list.*`/`playing` | refusal and `TIME_ERROR` events, the `switches` row |
| `read_list` | uhd-tx | one `book.read()` per entry (E1, `tx.rs:342-352`) | `receive` (with `tile_repeat`), `stop_now`'s close and notes |
| `step` plan | uhd-tx | `now` read (E1), then the guard (any unread entry → `read_list` first; widened from halts only, §1.5 D), `advance`, the cut check, expiry → `broken`, `late_start` → `untake`, join, `close_unjoined`, `dispatched`, and the send plan → `issue` (clone the samples' `Arc`, copy the slice bounds and the `BurstOpen`) | — |
| device calls | uhd-tx | — | the held-tail send (when `tail_first`), then the payload send (`tx.rs:695-734`) |
| `complete` | uhd-tx | `book.complete(op, outcome)` | `TxChain` updates (`tx.rs:814-835`), `tracker.on_block` and its record, stats, notes, the EOB close after an abandon |
| close (`end_open`, abandon close, `close_unjoined`'s close) | uhd-tx | the book decided it in the plan or `complete` section | `close_device_burst` (`tx.rs:883-921`) |
| reports | uhd-tx | `begin_read`/`end_read` (unchanged); an underflow calls `book.broken(chain.next())` in its own section | `tx_async` |
| configure, reopen | uhd-tx | unchanged (E2b) | `configure_tx`, `reopen_tx_streamer` |

The plan section uses its own `now` for `open_ended` (`tx.rs:689`) and for `dispatch_start`'s decision instant (`tx.rs:1092`). Today both read `core.now()` again outside the lock. This is identical on a manual clock and differs by microseconds in real time (INFERRED: no verdict depends on it). It also gives note 49 §5.4.3 m2: an issue is bounded by an instant read under the lock.

**Lock-wait record** (note 49 §12: "lock の最長の待ちの記録は E2a で入れ"). uhd-tx measures its wait to take the streams lock in the plan and `complete` sections, on the host clock, and keeps the maximum in `stats.tx_lock_wait_max_ns`. It is read on the bench in B1. Nothing decides on it.

**MockRadio** runs on one thread, under `&mut self`.
- Receipt calls `book.book` / `book.receive_switch(.., cursor = now_k, started = true)`. `now_k` is today's install cursor (`lib.rs:1723`, MR-42).
- `stop_tx` calls `book.stop` first (today `list.halt` comes first, `lib.rs:1177`, E1-K1). The MR-25 cut and `transmit_until_cut` follow unchanged; then `book.read()`.
- `open_held` calls `book.take`.
- `emit_tx_until` calls `book.issue` then `book.complete(.., Accepted(len))` for each block, an immediate full result (note 49 §5.4.3).
- `settle_switches_to` calls `book.advance`.
- The Mock's own rule for an opened burst with no block (`open_burst_without_record`, `lib.rs:1209-1216`) becomes the book's halt rule: the same answer (VERIFIED by reading both), one copy.

### 1.3 The op protocol

1. **One op in flight.** `issue` debug-asserts `inflight.is_none()`. `complete` debug-asserts the id and takes it. A second `complete` cannot apply: the id is gone. This is "exactly once" by construction, checked by a pure test.
2. **Ops are Sends only in E2a.** A Close (`close_device_burst`), a configure and a reopen are device calls of the executor, decided by a book transition. Close gets no `OpId`, because its result (front, or a reopen mark) is device-level only (`tx.rs:883-921`) and no booking-level fact reads it in E2a. Configure and Reopen become ops in E2b, where a Stop can arrive between the claim and the result. This narrows note 49's "Send and Close with tokens": see owner question (d1).
3. **The book interprets outcomes, with today's cursors** (`tx.rs:749-785`, `:695-714`):

   | Outcome | Cursor | Device close |
   |---|---|---|
   | `Accepted(device_count)` | — | — (`Sent`) |
   | `Accepted(m < device_count)` | `k0 + m` | today's `abandon(was_open \|\| m > 0, m, !device_eob)`, plus the EOB-failure reopen mark |
   | `Failed` | `k0 + device_count` | today's `abandon(true, device_count, ..)` |
   | `TailShort` (the tail send failed or was short) | `k0` | today's `abandon(true, 0, true)` |

   The device-level parts (close, front, reopen mark, `tracker.stop`) stay in uhd-tx.
4. **Reports.** UHD's reports carry no booking id (UR-28; `async_metadata_t`, VERIFIED in note 49 §8). An underflow or sequence report keeps today's rule: it applies to the device burst open when it is read (`tx.rs:1300-1303`), through `book.broken(chain.next())`.
   - `TimeError` and `BurstAck` reach no booking state (event and correlation only).
   - INFERRED residual risk, not changed by E2a: a late underflow from an earlier device burst would abandon the current one (no tick check, `tx.rs:1296-1303`).

### 1.4 Every write-back and `playing` site, and what each becomes

The executor's 15 record writes and 2 `playing` writes at `d9fddf3` (VERIFIED by `grep -n 'busy\.\|stop_at(\|\.playing'` on `tx.rs` production lines 1-1355). They match note 49 §4's 15 + 2 at `134b273` one for one.

| # | Site | Today | In E2a |
|---|---|---|---|
| W1 | `tx.rs:377` `busy.install(id, cursor)` | uhd-tx takes a switch at its handed cursor, when it reads it | gone: taken at receipt inside `receive_switch` (E2a-3) |
| W2 | `tx.rs:378` `busy.refuse(id)` | no cursor → `refused_past_boundary` | gone: under (b1) a receipt refusal, no row (E2a-3) |
| P1 | `tx.rs:382` `playing = S` | set when uhd-tx takes the switch | `receive_switch` sets it at receipt, as the Mock already does (`lib.rs:1781`) |
| W3 | `tx.rs:478` `busy.advance(now_k)` | the step applies due switches | `book.advance(now_k)`, same section, same rule |
| W4 | `tx.rs:517` `busy.drop_piece` (stale held) | unreachable (`debug_assert!` at `:513`) | `book.dispatched(Refused)`, or delete with the branch (minor, not required) |
| W5 | `tx.rs:578,581` `continuation_open` + `close` | uhd-tx's close of an unjoined device burst | `book.close_unjoined(..)`, one transition; the stat `tx_close_past_record_front` stays until E3 |
| W6 | `tx.rs:636` `busy.drop_piece` | refused configure | `book.configure_refused(k, len)` (E2b: `complete(Configure)`) |
| W7 | `tx.rs:990` `busy.cancel_switch()` | a failed reopen ends the clock | `book.clock_ended()` (E2b: `complete(Reopen)`) |
| W8 | `tx.rs:1004` `busy.cancel_applied(id)` | a held switch cancelled with its repeat | inside the book's abandon |
| W9 | `tx.rs:1029` `busy.cancel_switch_from(cursor)` | abandon of a repeat | inside the abandon |
| W10 | `tx.rs:1032` `stop_at(instant(cursor), 0)` | abandon ends the repeat, and removes every chain at or after the cursor (#80) | inside the abandon, scoped to the abandoned repeat (§1.5 A) |
| P2 | `tx.rs:1033` `playing = None` | abandon | inside the abandon, only when `playing` is that repeat or its switch |
| W11 | `tx.rs:1106` `busy.move_untimed` | untimed head moved at dispatch | `book.dispatched(MoveUntimed{from})`; `playing.origin` follows when it is the playing repeat |
| W12 | `tx.rs:1124` `busy.move_start` | SendAsap move at dispatch | `book.dispatched(Move{to})`; `playing.origin` follows (§1.5 E) |
| W13 | `tx.rs:1126` `busy.drop_piece` | a refused move | inside `dispatched(Move)` |
| W14 | `tx.rs:1145` `busy.drop_piece` | Drop at dispatch | `book.dispatched(Drop)`; `playing = None` when it was this repeat |
| W15 | `tx.rs:1146` `busy.cancel_switch()` | a dropped repeat cancels its switch | under (b1) unreachable, so deleted (a switch is accepted only on a started repeat, and a started repeat is never judged at dispatch again: `late_start` needs `open.first`, `tx.rs:495-496`). Under (b2) inside `dispatched(Drop)` |

Also at the executor, not the record: `record_configured` (`tx.rs:627,629,952`, E2b) and `finished` (`tx.rs:991,1208`, the receive line), both unchanged.

The receipt-side writers become `book.*` calls with unchanged logic: Ctl `control.rs:518-605`, Stop `:318,323`; `Streams::end` `core.rs:636-640`; the Mock `lib.rs:1657-1781`, `:1177-1186`, `:1817`.

**Checkable rule for the review:** `git grep -n 'record\.' crates/ezsdr-radio-uhd/src/provider/tx.rs` returns nothing in production code. Today's spelling of the same rule: `busy\.` must disappear from `tx.rs`.

### 1.5 How #80, #84, #85, #86 (and the stale grid) disappear

**A. #80** (abandon after a Stop removes a booking accepted after that Stop).
- Today:
  1. R plays; R's send is in flight.
  2. Stop@t: the record halts and `B = at_or_after(t+W) + STOP_FRONT`.
  3. Booking B is accepted (start ≥ B).
  4. R's send fails, or an underflow is read before the Halt.
  5. `abandon` → `stop_at(instant(c), 0)` → `halt(c, c+STOP_FRONT)` keeps only chains with `start < c` (`staging.rs:475-501`, `tx.rs:1032`). B's chain is gone; `playing = None` (`tx.rs:1033`); `until` is capped at c + STOP_FRONT, below B's end.
  6. B still plays (`e1_q5_a_booking_waiting_when_the_repeat_is_abandoned_still_plays` asserts only that).
  7. So a booking over B is accepted, and a switch to B is refused (INFERRED, issue #80).
- E2a: `complete(op_R, Failed)` (or `broken`) runs the abandon for R only:
  - R's region is capped at c + STOP_FRONT;
  - R's pending switch with b ≥ c is cancelled, or its applied-but-unsent chain is removed and settled cancelled;
  - its switch payloads at or after c leave `held`, one note each;
  - `playing` is cleared only if it is R or that switch;
  - `until` is capped only when no other booking remains.

  B keeps its region, its `playing` and its value-rule busy end. Nothing in RM-15, RM-16 or RM-21 changes: today's code breaks them (RM-21 "busy end is the exclusive end of its last booked burst", `design/07-radio-model.md:238`).

**B. #85** (a dispatched, unsent booking vanishes at a Stop).
- Today: H is dispatched into `Tx::open` and its first send waits for 32 sealed samples. The halt's `stop_now` → `end_open` takes `open` and writes a record only if the tracker has one (`tx.rs:875-877`): no note.
- E2a: `open` is in the book, and the book's `read()` turns a `Halt` into `cancelled = held ∪ {open if open.first}`. H gets one note with the halt's reason; the device-level close is unchanged.

  This covers every halt: Stop, `Provider::stop` (shutdown calls `stop_now`, `tx.rs:1333-1336`), and a joined continuation taken but not yet sent. It is the Mock's existing rule (`lib.rs:1209-1216`).
- The pin `o1_uhd_t6b2_…_known_divergence_85` (`o1.rs:663-680`) flips: `notes == 1`.

**C. #84** (a repeat dropped at dispatch leaves its switch `refused_past_boundary`).

| Step | Today | E2a with (b1) | E2a with (b2) |
|---|---|---|---|
| R (DropAndFlag) is booked at 2 000 | | | |
| device time reaches 3 ms; R is not yet issued | | | |
| switch S at 3 000 | accepted, row pending | `receive_switch` sees `playing.started == false` (R's first send not issued) → refused at receipt, `COMMAND_REJECTED` with S's id, in the submit reply (EA-11), no row | accepted, pending, not taken |
| uhd-tx reads R and S | reads R into `held`; reads S → no cursor → `refuse` → `refused_past_boundary` + "not taken" (`tx.rs:361-362`) | — | — |
| R is dispatched late | dropped (`o1.rs:908-930`) | dropped: `TIME_ERROR{drop}` | dropped, which cancels S → row `cancelled` |

- (b1): the pin flips, with the oracle's #84 answer changed in the same commit as RM-37's text.
- (b2): the pin flips to the oracle's current answer.

**D. #86** (a moved switch's record has no `requested_target`).
- Under E2a no switch is ever moved, so no `late_by` is set on a switch. The pin's block (`o1.rs:435-440`) goes with the move.
- Why no switch moves (INFERRED from the code, VERIFIED lines):
  - The receipt reads its instant inside the streams section (`control.rs:481`) and chooses `b ≥ at_or_after(receipt + W + lead)` (`control.rs:553`).
  - uhd-tx's step reads `now` under the same lock (`tx.rs:472-481`), and no send passes `window_cut = at_or_after(now + W)` (`tx.rs:488`, `:664-668`).
  - Both instants come from the monotonic authority (`fetch_max`, note 49 §5.4.10).
  - So everything issued before the receipt ends at or before `b − lead`.
  - Everything issued after it stops at `b`, provided uhd-tx reads the switch before it hands past `b`.
- That proviso is why E2a widens the step's guard from "a halt is unread" (`tx.rs:475`) to "anything is unread". E1's implementer found that the three move-path tests went red with that guard, and the record names no other (`plan/phase8/reviews/e1-dispositions.md`, "Implementer, E1a" row). VERIFIED from the record; that nothing else changes is INFERRED.

**E. Stale grid** (new; INFERRED consequence, VERIFIED lines).
- Today: R (SendAsap) is booked at 2 000; a configure or a host stall makes it late; `dispatch_start` moves it to m (`tx.rs:1124`). `playing.origin` stays 2 000 (`control.rs:577`; no other writer). A later switch S gets `b = repeat_boundary(2000, N, ..)` (`control.rs:554`) while R's samples repeat from m, so the waveform switches mid-period.
- E2a: `dispatched(Move{to})` updates `playing.origin` when `playing.id` is R. Under (b1) no switch can be pending then (R had not started), so nothing else needs to follow.

## 2. Alternatives, and why each is rejected

1. **Lean shape: `open` and result interpretation stay in uhd-tx; the book holds only bookings, halts, the switch and `playing`; no ops.**
   - Smallest change: about 40 % of E2a-2.
   - It fixes #80, #84 and the stale grid as well. #85 would become a 3-line local note in `stop_now`.
   - Rejected because:
     - the cursor at abandon and the abandon's scope stay executor logic, which is exactly how #80 arose ("the abandon writes its own view back over bookings it does not own", #80's mechanism line);
     - #85 would be the fourth local patch of `mech:planned-state-read-as-actual` (AGENTS §6, three strikes; note 49 is the design note that rule asks for);
     - the protocol would land in E3 together with the segmentation move, so E3 would become the riskiest stage, against note 49 §12's split;
     - C2 and C6 need results applied by the book (§7).
2. **Full shape: the book also owns `TxChain` (device burst, tail, front, recovery).**
   - Rejected for E2a:
     - nothing at receipt reads device-burst state until E3 replaces the record's front prediction with the shared segmentation;
     - the Mock has no device bursts (its `DeviceModel` sees only SOB/EOB, `crates/ezsdr-mock-radio/src/device.rs`), so it would carry dead fields;
     - E3 moves the state together with the decision function that reads it, so nothing is moved twice.
3. **One transition per write-back** (wrap each `busy.x()` as `book.x()`). It moves code, not decisions: the executor would still choose the cursor and what an abandon reaches. The table in §1.4 groups by cause instead (abandon, dispatched, close), so each rule is written once in the book.
4. **Tokens per device burst instead of per send.** A device burst spans bookings (joins, a switch at its boundary). A short or failed result is per call, and the cursor needs the call's range. Reports carry no id to match a burst token against (UR-28). It adds an identity nothing can read.
5. **Close as an op too** (note 49's "Send and Close"). It costs a lock section around four close sites (`end_open`, abandon, `close_unjoined`, `end_clock`), but no booking-level fact reads a Close result in E2a. Keep it out until a reader exists (E2b or C2): owner question (d1).

## 3. Principles that decide cases, and the check against the promises

**Principles** (note 49 §6, applied to E2a):
1. **One writer per fact.** The book writes bookings, halts, switch, `playing`, `open`, the issued send and `issued`. uhd-tx writes device facts: `TxChain`, `unacked`, `late_reported`, the tracker, the streamer generation.
2. **A result reaches only its own op's booking and the switch that replaces that booking.** Never a booking received later, and never another booking's value-rule end.
3. **Receipt answers from the book's actual state at its instant, under the lock**: the record, and the fact that a repeat has started. E2a adds no prediction. The existing predictions (`front`, `may_be_handed`) stay until E3.
4. **A switch is decided at receipt:** taken at its boundary or refused. Afterwards only Stop, loss, `Provider::stop`, an F8 failure or an abandon settle it, by RM-37's existing rules.
5. **Nothing issued is recalled; a halt cancels everything not issued**: held bookings, those read before it, and an open booking with nothing issued.
6. **Extraction changes no behaviour except the named items.** Each sub-step lists its determined differences, and every other existing assertion stays, FakeDevice `calls()` traces included.
7. **The Mock is a stepped executor.** Its results are immediate full accepts, and its repeats are started at acceptance.

**Check against the existing promises.** "Text" = what E2a changes in `design/` or `python/`, in the commit that changes the behaviour.

| Promise | Effect of E2a | Text |
|---|---|---|
| RM-13 (`design/07:144`) | none | none |
| RM-14 (`:157`) | unchanged: lateness is still decided at dispatch, now inside `dispatched` | none (note 49 §7's "issue" wording optional) |
| RM-15 (`:161-165`) | unchanged; #80 makes UHD keep the footprint of a booking accepted after a Stop, as the text says | none |
| RM-16 (`:169-176`) | #85: UHD now cancels an unsent open booking ("cancel held bursts not handed"), as the text says | `*Test:*` line names the flipped pin |
| RM-21 (`:238`) | #80: the busy end is no longer capped below a live booking | none |
| RM-37 (`:350`) | outcome rules unchanged; the UR-38 move goes | delete "the owner moves it when its cursor has passed it (UR-38; a Mock never does…)"; (b1): "playing" = its first sample has been issued to the device, and delete "and one that drops the repeat at dispatch under its late policy" (unreachable); (b2): add "or moves it" to that clause |
| UR-21 (`design/18:243-250`) | the "not taken" refusal (`tx.rs:362`) disappears | (b1): a switch's receipt refusal is listed in UR-38, not here |
| UR-22 (`:253-257`) | #80: "future held bursts keep their reservations" is now true for the record too | none |
| UR-23 (`:260-275`) | the step reads every unread receipt before it hands | one sentence: "uhd-tx reads every receipt waiting on its list before it hands further" |
| UR-25, UR-26 (`:278-313`) | unchanged | none |
| UR-28 (`:317-321`) | unchanged | none |
| UR-30 (`:333`, the `switches` row) | `moved_to` goes | drop `moved_to` from the row's columns |
| UR-36 (`:291-297`) | its last sentence (pending switch and playing repeat under the same lock) stays true | none |
| UR-38 (`:300-302`) | taken at receipt; no move | rewrite: the second paragraph's take and move sentences, `moved_to`, "The moved switch's burst record reads the same amount as its `late_by`", the abandon/drop sentence per (b1)/(b2); (b1) adds the refusal "the repeat it replaces has not yet started on the device" |
| MR-25 (`design/09:171`) | unchanged (the halt rule is the Mock's own) | none |
| MR-42 (`:221`) | unchanged behaviour | drop `moved_to` from the row and the sentence about it |
| EA-16 (`design/16-easy-api.md:85-98`; `switch` row at `:97`) | (b1): `switch()` raises `Rejected` in one more case (`python/ezsdr/session.py:114-121` raises on any refusal in the submit reply, EA-11) | (b1): the row's and the docstring's (`session.py:500-502`) list of refusals add "or one whose start the radio has not yet sent (a host or configure delay)" |
| KL-13, KL-36, KL-37, KL-38 | unchanged | none |
| KL-39 | unchanged (the finite and repeat footprints after an abandon stay; C6 later) | none |
| Vision §58, §61; AGENTS §3 | unchanged; the book is Vocabulary (`ezsdr_radio`), no Kernel change | none |

## 4. Owner questions, one at a time

### (a) #86: which `requested_target` a moved switch's burst record carries

**What changes first.** Under E2a no switch moves (§1.5 D), so no switch record ever has `late` set. The question becomes "what does any switch's record carry".

**Today** (VERIFIED):
- An unmoved switch records `late: None` and `requested_target` = SC-23a's `requested_at`, which is `None` for an exact `at` (`control.rs:499`, `:593`).
- The Mock does the same (`lib.rs:1660`, `:1769`).
- Only UHD's moved switch differs: `late_by` set, `requested_target: None` (#86).

| Candidate | What it means to a user | Spec reading |
|---|---|---|
| **None** (today for every unmoved switch, both Providers) | "the switch started where the switch rule put it; nothing deviated" | SC-29a says requested target is "what the caller asked for when the applied target differs" (`design/02-stream-contract.md:260`). A switch's `at` is "not before", and its boundary is the rule's answer to that request, like an untimed placement, which has no requested instant (RS-19). |
| **the switch's `at`** | "you asked for `at`, it started at `b`" | SC-29a read literally; the oracle's answer for a moved switch (`oracle/mod.rs:320-322`); what the dispatch-move twin would write (`tx.rs:1137` with `held.target` = `at`, `control.rs:604`). Every timed switch would then carry it, on both Providers. |
| **b0, the boundary chosen at receipt** | "it was due at b0" | UR-38's `late_by` is measured from b0 (`tx.rs:386`). Redundant once nothing moves: b0 is the record's own `target`. |

RM-37, UR-38 and SC-23a say nothing beyond this.

**Recommendation: None**, which needs no decision beyond deleting the move. The record stays the same on both Providers.
- If the owner wants SC-29a's literal reading (every timed switch records its `at`), make it a separate small contract change on both Providers with its text and schema-free fixture updates, not part of E2a.
- #86 closes when E2a-3 lands ("scenario removed"), with the comment corrected (the twin would give `at`, not b0).

### (b) N2: a switch on a repeat that plays by the record but has not started on the device

**When it happens.** Only on UHD, and only when uhd-tx has not issued the repeat's first sample by the time device time reaches its start: a configure or a host stall of at least W. VERIFIED from the code: a normal owner issues a booking about W ahead (`tx.rs:536-537`).

| Option | Behaviour | Merits | Demerits |
|---|---|---|---|
| **(b1) refuse at receipt** | `COMMAND_REJECTED` with the switch's id in the submit reply; `switch()` raises `Rejected`; no row | the user learns at once, with a reason, and can retry once the repeat sounds; matches EA-16's "raises `Rejected` when the Provider refuses it at booking (no repeat plays on the stream…)" (`design/16-easy-api.md:97`) in its plain meaning; deletes the most code (`installed`, `refuse`, `Refused`, W15, the deferred take); the Mock is unchanged (its repeats are started at acceptance) | the receipt answer depends on whether uhd-tx has issued (an actual fact the book holds, not a prediction, but host-timed); the #84 oracle answer and RM-37's "playing" change; one more refusal reason in UR-38 and EA-16 |
| **(b2) accept; settle when the repeat is decided** | accepted, row pending, not taken; the repeat's first issue takes it; a drop or move of the repeat at dispatch cancels it (row `cancelled`, a note) | the receipt stays a function of the record alone; the #84 oracle answer stands as written; RM-37 needs only "or moves" | the switch is lost with no event naming it (cancellations are notes only, `cancellation_reports`, `crates/ezsdr-radio/src/lib.rs:122-124`), which is less than today's async `COMMAND_REJECTED` "not taken"; needs a deferred take (`installed` kept) and an `advance` guard, or a receipt or step that reaches `b` before the late repeat is decided applies the switch and then plays it as a moved repeat (INFERRED from `tx.rs:478`, `:1145-1146`) |
| (b3) today | accepted, refused later by uhd-tx with "not taken" | none new | keeps the two-stage take E2a removes; #84 stays |

**Recommendation: (b1)**, the orchestrator's recommendation in note 49 §12.1, and it holds against EA-16.
- Python: `Session.switch` → `_admitted(*self._submit(...))` raises on the first refusal in `refused` (`session.py:114-121`, `:496-512`).
- EA-11 puts a booking-time refusal in the submit reply on both kinds of Run (`design/16-easy-api.md`, EA-11).
- Note 49 §5.3 rejected receipt answers that depend on the owner's progress because Mock and UHD would differ in the normal case. Here they differ only when UHD's owner is already late, and they differ in every option.

**Evidence that (b1) is total.**
- A switch is accepted only when `playing.started` is true.
- A started repeat is never judged at dispatch again: `late_start` requires `open.first`, `tx.rs:495-496`, VERIFIED.
- So RM-37's "drops the repeat at dispatch" clause and W15 become unreachable.

### (c) `install`'s `moves` argument and the `moved_to` field

**Facts.**
- `moves` is already gone. N0 removed it ("Bookings::install's `moves` parameter are gone", commit `25b9b15`), and `install(&mut self, id, cursor_k)` has two parameters (`staging.rs:522`). VERIFIED.
- `moved_to` would be always null on both Providers after E2a-3.
- It has no schema and no reader outside the Rust tests: `git grep moved_to -- schemas python plan/phase8/tools` returns nothing. Its only reader is a bench verdict, `crates/ezsdr-radio-uhd/tests/common/n47s5.rs:289` (`moved_to.or(boundary)`).

**Options.**
- **Delete** `moved_to`, `Switch.installed` (under (b1)), `Bookings::install` (folded into `switch`), `Bookings::refuse` and `SwitchOutcome::Refused` together. Pre-release, no schema to regenerate, AGENTS §9 Maintainability 4.
- **Keep** a null column for a future Provider that moves switches (8c Replay, another SDR). YAGNI: that Provider would re-add the move rule and the column with its own spec text.

**Recommendation: delete.** Text: MR-42, UR-38, UR-30's row, note 49 §7's RM-37 line.

### (d) Other owner-level points

| Point | Recommendation |
|---|---|
| (d1) Tokens on Sends only in E2a; Close, Configure and Reopen join the protocol in E2b (or when C2/C6 need a Close result) | Yes: nothing reads a Close result at booking level in E2a (§2 item 5) |
| (d2) The stale grid (§1.5 E) is fixed in E2a-3 as a named behaviour change (UHD only), and an issue is filed before the fix | Yes |
| (d3) #80 includes the value rule: after a repeat's abandon, the busy end is capped only when no other booking remains | Yes: RM-21's text already says so |
| (d4) The lock-wait record is the stat `tx_lock_wait_max_ns` (wall time on the host clock, never a verdict) | Yes |
| (d5) Under (b1), the new synchronous refusal is user-visible. Add it to KL-13 (burst timing: "a `switch` right after a repeat whose start was delayed is refused; retry once it sounds"), or not at all (today's async refusal is unlisted) | Add to KL-13 |
| (d6) Reopen #84 (orchestrator action, not an owner decision) | — |

## 5. Test and mutation-row migration (the table note 49 §12 asks for)

All counts at `d9fddf3`. Counts from my scripts are marked as such; the rest are estimates.

**Tests that disappear** (their state is unreachable once switches are taken at receipt):
- `ur_21_s12_send_asap_switch_handed_past_is_installed_at_the_next_boundary` (tx.rs).
- `ur_23_stop_before_the_moved_boundary_cancels_the_switch` (tx.rs). Its unmoved twin `ur_23_stop_before_the_boundary_cancels_the_pending_switch` stays.
- `rm_37_a_late_switch_moves_to_the_next_boundary_of_its_grid_and_a_stop_reads_the_moved_one`, `rm_37_install_takes_only_the_pending_switch`, `rm_37_install_at_the_boundary_is_not_a_move` (`crates/ezsdr-radio/tests/staging.rs`). Their "only the pending switch, at its boundary" assertions move to the `receive_switch` pure test.
- `txlist_halt_unread_ignores_bookings` and `TxList::halt_unread`, if the widened guard uses `unread()` alone.
- The `cursor > b` case of `ur_21_s8_switch_at_the_exact_handed_cursor_installs`. The `≤ b` cases become one pure receipt test.
- In each, also remove their `*Test:*` names in RM-37, UR-36 and UR-38.

**Tests rewritten** (the pins flip or the outcome changes):

| Test | Change |
|---|---|
| `o1_uhd_t4b_…` | "an owner that has not read a switch never hands past its boundary": the row is applied at b, no `TIME_ERROR`, the old record ends at b; the #86 block goes |
| `o1_uhd_t8_…_84` | (b1) refused at receipt, no row; (b2) `cancelled` |
| `o1_uhd_t6b2_…_85` | `notes == 1` |
| `ur_21_s4_a_switch_whose_repeat_the_host_dispatched_late_is_refused_without_a_cursor` | (b1) refused at receipt with id 2 |
| `e1_q4b_a_switch_read_after_the_stop_that_took_it_is_not_taken` | no "not taken" event; one Stop note; row still `cancelled` (B = 13 005 ≤ b = 15 000 at 1 MS/s, INFERRED from the rig) |
| `e1_q5_…_still_plays` | adds #80's asserts: a booking over B is refused; B is the playing repeat |
| `ur_21_s8_switch_installed_before_the_boundary_sample` | taken at receipt |
| `o1_parity_a_switched_repeat_changes_waveform_at_its_recorded_boundary_on_its_grid` (`parity.rs:2708`) | drop the `moved_to` fallback (`:2737,2757`) |
| `moved_to` assertions in `tx.rs` (2), `mock_radio.rs` (1), `mock_channel/o1.rs` (2), `staging` tests (estimate 6), and the bench verdict `n47s5.rs:289` | removed |
| `iii.rs`'s `a_switch_not_applied_fails_and_no_tone_is_inconclusive` | uses `"cancelled"` instead of `"refused_past_boundary"` |

**Tests whose plumbing changes, not their assertions:**
- `tx.rs` tests touching `tx.open`: 78 references (script count), through the book.
- `stream.busy`/`.playing`/`.list`: about 20 helpers (`give`, `hold`, `held_keys`, `book_burst`, `book_switch`, `booking_control`, …).
- Mock tests that poke `list.held`.
- Estimate: 300–500 lines of test diff over E2a-1 and E2a-2.

**Assertions that move to pure book tests** (`crates/ezsdr-radio/tests/staging.rs`; estimate 12–18 tests):
- the switch receipt: playing, pending, started, boundary, taken at receipt, the debug invariant `cursor ≤ b`;
- the abandon's scope (#80): another booking's region, `playing` and busy end survive;
- a halt with an unsent `open` (#85);
- `dispatched` (moves, drop, a refused move, `playing` following a move);
- `issue`/`complete` exactly once, and the outcome→cursor table of §1.3.

The device-call sides stay as `tx.rs` rig tests (Maintainability 3).

**New UHD or Mock rig tests** (estimate 10–15):
- #80 on the frozen rig;
- the stale grid;
- N2 (b1) on UHD; and on the Mock, a switch received in a step that jumped over the repeat's start is accepted;
- the widened guard (the `t4b` rewrite);
- #85 under `Provider::stop` and for a taken, unsent continuation;
- `stats.tx_lock_wait_max_ns` present.

**Mutation rows** (lists in `plan/maintenance/tools/mutations.json`; family totals by script: N47S5 184, N48S1 19, N48S3 19, N47S4 63, E1 16, O1 10):

| Action | Rows |
|---|---|
| **Delete** (estimate 15–17) | the move and `moved_to`: N47S5-41, -42, -74, -75, -77, -78, -79, -80, -96, -97, -98, -99, -100, -101 (script: anchored in `install`/`write_settled`/`apply`/`tx.rs install`); under (b1) N47S5-157, -158 (dispatch drop cancels the switch); E1-U3b if `halt_unread` goes |
| **Retarget with a changed mutation** (the behaviour moves) | N47S5-144 (no-cursor refuse) → (b1)'s started check; N47S5-46 (owner never takes) → the receipt take; N47S5-91 (`playing` at install) → `playing` at receipt; N47S5-35 and -36 (`installed` in `advance`/`halt`) → the boundary alone; N47S5-136 and -139 (abandon's `stop_at`) → the scoped abandon; E1-U3 and -U4 (guard) → any unread; N47S5-23, E1-K1 and -K2 (the Mock's halt order) → `book.stop`/`book.read` |
| **Retarget with the same mutation** (re-anchored) | 74 rows whose `old` names `busy.`, `playing`, `list.`, `self.open`, `install(`, `refuse(`, `stop_at(`, `installed` or the held helpers in the five files (script count; they overlap with the rows above); plus rows in code that moves into the book: `dispatch_start` 12, `abandon` 9, `close_unjoined` 3, `cancel_held_switches` 1, `core.rs` `stop_at`/`end` 10, the Mock's `stop_tx` record part 4, `handle_tx_burst`'s switch part about 6. Net estimate: 60–80 rows re-anchored, plus the `file`, `crate` and `filter` updates for those that move into `staging.rs`. Cross-crate rows already exist: 15 rows mutate `staging.rs` with `ezsdr-radio-uhd` tests (script) |
| **O1** | O1-M3a and -M3b (the switch boundary on each Provider) become two rows on the one shared `receive_switch` anchor, one killer per Provider; O1-M6 → the book's abandon; O1-M8 → `receive_switch`; O1-M1, -M2, -M4, -M5, -M7 unchanged |
| **N48S1, N48S3** | in `send` (13 rows) and `step` (2): re-anchor where the send split changes tokens (estimate 5–10); FakeDevice and the Mock's `tx_handed_end` rows unchanged (E3) |
| **N47S4** | about 10 anchored on `busy.` (value rule): re-anchor only; configure rows unchanged (E2b) |
| **New** (estimate 15–20) | the abandon scope (removes later chains; caps `until` despite another booking; clears `playing` regardless of id); the halt skips an unsent `open`; the started check skipped or started set at acceptance on UHD; `dispatched` leaves `playing.origin`; the guard reads halts only; `complete` ignores the op id, or applies twice; the short cursor `k0 + count` instead of `k0 + m`; the failed cursor without padding; the Mock skips its per-block `complete` |

Each new or retargeted row names its killer test in `filter`.

**O1 stays green throughout.**
- Under (b1), E2a-3 changes the oracle's #84 answer (`outcome_after_repeat_dropped`, `oracle/mod.rs:314-316`) to a receipt refusal, in the same commit as RM-37's text.
- `moved_boundary` and `requested_target_of_a_moved_switch` (`:177`, `:320`) go with the move.
- `o1_oracle.rs`'s self-tests follow (estimate 3).
- All three known-divergence pins flip in E2a: #85 in E2a-2; #84 and #86 in E2a-3.

## 6. Risk: sub-steps, and the sequences the brief must list

**Risky** in AGENTS §9's sense: ownership of results, a lifecycle (open → issued → ended), and the order of device calls against lock sections. All three sub-steps end green (workspace `cargo test`, O1 included, `rules.py` = 0). Each is committed on a local `work/e2a` branch (AGENTS §9).

| Sub-step | Contents | Behaviour | Size (estimate) |
|---|---|---|---|
| **E2a-1: one owner** | `TxBook<P>` holding `record`, `list`, `playing`; the write-backs W3–W15 become book transitions with today's semantics (abandon still today's `stop_at` reach); receipts call `book.*`; `Booked` for `Held` and `HeldBurst` | none | 600–900 lines with tests and about 50 re-anchored rows |
| **E2a-2: results to the book** | `open` into the book; `send` split into plan (lock) / device calls / `complete` (lock); `issue`/`complete`/`broken`; the abandon scoped to its booking (#80); the halt takes an unsent `open` (#85); the Mock's `open` and per-block `issue`/`complete`; `stats.tx_lock_wait_max_ns` | #80, #85 | 1 100–1 400 lines |
| **E2a-3: switches at receipt** | `receive_switch` shared by both Providers; (b1) N2; the guard on any unread entry; `dispatched` moves `playing.origin`; delete `install`, `refuse`, `installed`, `moved_to`, `Refused`, the UR-38 move, W15; spec text (§3), oracle (#84, #86) | #84, #86, the stale grid, the four determined differences of note 49 §5.4.7 (+ (b1)'s receipt refusal) | 1 000–1 300 lines |

**Sequences for the brief** (AGENTS §9 step 1). Each has its expected result after the sub-step and what it relies on.

Hardware facts:
- VERIFIED: the gap-0 drop and the safe start after a close (2026-11-05/06, 2026-11-10 sweep, `plan/phase8/bench-results.md`); P6, late continuations are not sent (bench-results:223); reports carry no booking id (`metadata.hpp`, note 49 §8).
- INFERRED: W = 10 ms, lead 2 ms, allowance 3 ms (KL-15).

| # | Sub-step | Sequence | Expected | Relies on |
|---|---|---|---|---|
| S1 | E2a-1 | every existing test, the O1 oracle and `calls()` traces | unchanged | — |
| S2 | E2a-2 | R plays; Stop@t; B (repeat, start ≥ B_stop) accepted; R's in-flight send fails, then the Halt is read | R's region ends at c + STOP_FRONT; B keeps its region (a booking over B is refused), is `playing` and keeps the stream busy; B plays; one close at c (two zeros) | #80; RM-15/16/21 text; close front VERIFIED |
| S3 | E2a-2 | the same with an underflow report instead of a failure, and with B finite | as S2; with B finite, `until` stays B's end | UR-22; underflow attribution unchanged (INFERRED risk §1.3) |
| S4 | E2a-2 | H booked at `window_end − 16`, dispatched and waiting for 32 sealed samples; Stop | one note "UR-26: cancelled by Stop"; no sample, no record; same on the Mock | #85; MR-25 |
| S5 | E2a-2 | as S4 with `Provider::stop`, and with a taken, unsent continuation behind a held tail | one note each; the tail closes the device burst (tail + 0) | UR-26; held-tail close VERIFIED |
| S6 | E2a-2 | the send in flight; Stop received; send returns short (m) | close at k0 + m once; held bookings then cancelled with notes; no second close | m1 order (note 49 §5.4.4 4) |
| S7 | E2a-2 | `complete` twice for one op; `complete` with another op's id | debug panic (pure test) | — |
| S8 | E2a-2 | `Failed` on a padded closing first send; `TailShort` on a continuation | cursor k0 + device_count / k0; fronts as today | today's `tx.rs:749-785`, `:695-714` |
| S9 | E2a-2 | lock-wait stat | present, positive, never in a verdict | — |
| S10 | E2a-3 | R started; S received; uhd-tx steps without having read S at 4–8 ms | S taken at b at receipt; uhd-tx reads S before handing; old repeat to b, new from b, one device burst, no EOB at b, no `TIME_ERROR` | §1.5 D; E1 implementer record |
| S11 | E2a-3 | S received; Stop received before uhd-tx reads S, with b < B; and with B ≤ b | `applied` (the Mock's answer; today UHD `cancelled`); S's payload cancelled with a Stop note; no new sample when the close is before b. With B ≤ b: `cancelled`, one Stop note, no "not taken" | note 49 §5.4.7 differences 1, 3 |
| S12 | E2a-3 | #84's sequence | (b1) refused at receipt with S's id, no row; R dropped with `TIME_ERROR{drop}` | §4 (b) |
| S13 | E2a-3 | R moved at dispatch to m, started; S received | b on the grid from m | stale grid (§1.5 E) |
| S14 | E2a-3 | R abandoned at c < b with S pending; and at the join point c = b with S taken there | S `cancelled`; its payload removed with a note; `playing` none; no new sample | RM-37; O1 t7a/t7a3 |
| S15 | E2a-3 | Mock: a switch received in a step that jumped over R's start; a switch at R.start == now_k | accepted, as today | MR-42 unchanged; `rm_16_switch_refused_unless_a_repeat_is_playing` pins the second |
| S16 | E2a-3 | a switch's burst record | `late: None`, `requested_target` per SC-23a; no `moved_to` | §4 (a) |
| S17 | all | `ur_25_the_line_lock_is_free_inside_every_transmit_device_call` | green | UR-25 |

**Main risk.** E2a-2's send split: plan under the lock, then up to two device calls, then `complete`.
- The held tail, the padding and the "device burst open" flags are now computed in one section and applied in another. A mistake there keeps the tests' samples right but changes the FakeDevice framing (an EOB placement, a tail sent twice), and on hardware the framing is what drops a burst.
- Mitigations:
  - E2a-1 first, so E2a-2's diff is only the split;
  - compare `calls()` traces before and after on the existing frozen rigs: their assertions already pin SOB, EOB, `at` and counts;
  - keep the device-level code in uhd-tx unchanged;
  - run the full mutation lists before Sol's review (AGENTS §9 step 6).
- Second risk: the abandon's new scope must remove the switch's applied-but-unsent chain (S14) and nothing else. Today's `stop_at` removed too much; a narrower rule can remove too little.

## 7. What this choice makes impossible or harder later

- **C1** (re-segment unissued pieces after a refused configure): not blocked.
  - The pieces are book state (`held`, `open` with `first`), so "unissued" is a book query.
  - E2b must first bring the configuring booking into the book (today it leaves `held` during the turn, `tx.rs:538`, `:614-641`).
- **C2** (honest switch outcome): needs "the op carrying b was sent", which `complete` sees (op range ∋ b). Not blocked.
  - (b1) removes one C2 case (the dispatch drop), so C2 only has the Stop and the failed-send rows.
- **C6** (shrink an abandoned region to what was sent): the book's `open.next` after `complete` is the sent cursor. Not blocked; it is cheap after E2a, as note 49 §12 expects.
- **C4 / C5** (Stop (a), joins to short heads): segmentation (E3). Not blocked.
- **E3** must move `TxChain` and the segmentation into the shared function together. E2a leaves P2 and P3 as two copies (record join points and front versus `TxChain`) until then. That is deliberate (§2 item 2), not a dead end.
- **8b / 8c / other SDRs.** The book is per stream; an op maps to a Replay play command (note 49 §9).
  - No Close op until E2b: a future executor that needs a Close's result at booking level adds it then.
- **Impossible after (b1):** accepting a switch on a repeat that has not started on the device, so there is no "queued switch that follows a late repeat". A user must retry.
- **Impossible after deleting the move:** a switch that follows its repeat past a missed boundary. A Provider that needs it (a slower executor than UHD's real-time thread) re-adds a move rule, its `TIME_ERROR` and the column, with spec text.
- **Impossible after deleting `refused_past_boundary`:** a third outcome value until C2 adds `uncertain`.

## Appendix: searches run, and what was not checked

- Write-backs: `grep -n -E 'busy\.|stop_at\(|\.playing|record_configured|\.finished\('` on `tx.rs` lines < 1356 at `d9fddf3`.
- Callers: `git grep -n '\.refuse(id)\|busy\.install(\|\.install(id' d9fddf3 -- 'crates/*.rs'` (only `tx.rs:377-378`, `lib.rs:1723`). `git grep -n '\.playing\b' d9fddf3 -- 'crates/*.rs'` (production writers `control.rs:577`, `tx.rs:382,1033`, `lib.rs:1781`; readers `control.rs:519`, `lib.rs:1712`). `git grep moved_to d9fddf3 -- schemas python plan/phase8/tools` (nothing).
- Mutation rows: a script mapped each row's `old` (whitespace-free) to the enclosing `fn` in the five files at `d9fddf3`; every row was found (0 unfound). Token counts: `busy.` 29, `playing` 8, `list.` 12, `self.open` 3, `install(` 1, `refuse(` 5, `moved_to` 6, `installed` 3, `stop_at(` 6, held helpers 5; 74 distinct rows.
- Issues: `gh issue view 80|84|85|86` (#84 closed 2026-11-09T17:09:24Z, COMPLETED, no comment).
- **Not checked:**
  - nothing was built or run, and no sequence was executed;
  - line counts and row counts marked as estimates were not computed;
  - the Mock's per-block `complete` cost on the long channel tests (INFERRED small);
  - whether any test advances time at a probe between the step's guard and `send` (searched only `tx_step_guarded`, used by E1's Q2b);
  - the Python suites (no Python change in behaviour except (b1)'s new refusal path on UHD, which no Python test reaches: the Python carriers run on the Mock, INFERRED).
