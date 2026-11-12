# E2b test plan (note 49, stage E2b: `Configure` and `Reopen` become operations of the transmit book)

Implementer's test plan, written before any code (AGENTS §9 step 2). Base: `work/e2b` = `main` `9c84502`. No file in the repository was edited, nothing was built or run. Every claim comes from reading the code at `9c84502` and is cited as `path:line`. What I did not read or run is marked INFERRED.

## Changes after Sol's review

Sol's review (`sol-plan.md`, FAIL) and the orchestrator's adjudication (`plan-adjudication.md`: all three majors and all minors adopted). Each line says what changed and where.

**Majors**
- **M1. Operation identity.**
  - Every op is now issued with a token: `issue*` returns an `OpId`, and every `complete*` names it. A completion whose token is not the op in flight panics before it changes anything (§0.2).
  - Send gets the token too, because the three kinds share one `inflight` field. The adjudication's premise "as `Send` in E2a" does not hold: E2a's `Send` has no token (§8 item 7).
  - New pure test P7 covers the stale same-kind replay (Sol's sequence), a wrong token, a duplicate completion and a wrong kind, each for Configure, Reopen and Send. P2 and P3 now use tokens (§3.2).
  - New rows E2B-15 (the token check removed) and E2B-17 (tokens not unique). E2B-07 is restated (§4.2).
- **M2. The refused-configure boundary.**
  - Today's sections are kept exactly. The mark and `give_back` stay in one section. The book's result (`complete_configure`) is applied in its own later section, as today's `drop_piece` (`tx.rs:632`) and `hold` (`:630`) are.
  - The failed reopen's held cancellation also stays in its own section (`take_all_held`, `:977`). So `complete_reopen` no longer returns the held bookings.
  - (m1) and (m2) are withdrawn; only (m3) remains, and no section merges anywhere (§0.2).
  - New characterization **C8** pins Sol's interleaving deterministically on today's code. A receipt of Z overlapping the refused X runs between the two sections and is refused; the same Z after the turn is accepted (§2 Q11, §3.1).
  - New row E2B-16 merges the sections and is killed by C8 (§4.2). P1206 now stays unchanged (§4.1).
- **M3. The loss read in the result sections.**
  - New test-only seam `on_next_lock`: a one-shot hook that runs on the owner thread immediately before its next acquisition of `core.streams`, after any code between the device call and that lock (§3.3).
  - New **C9** publishes a loss there, for both the reopen and the configure result sections, and asserts the mark stays cleared (§3.1).
  - E2B-11's killer is now C9(a). New row E2B-14 hoists the configure's loss read and is killed by C9(b) (§4.2).

**Minors**
- **Full records and events.**
  - Every C-test asserts one ordered snapshot: `calls()`, `rec.applied`, `bursts`, `rejected`, `switches` and the events (kind, payload, order) (§3.1, "Snapshots"). Exclusions are listed with reasons.
  - The out-of-tree trace comparison now also dumps that snapshot for every existing test (§3.3).
- **Timing.** Every reopen C-test runs the owner's work inside `test_support::released` (`test_support.rs:170-185`). None uses a `Frozen` release with `pass` on the test thread (§3.1, "Timing").
- **R2.** It is scheduled for both the Configure and the Reopen paths, unconditionally: the lock-spy test at `tx.rs:3904-3928` reaches neither (§3.3).
- **Close rationale.** Corrected: `end_open` calls `book.end_open()` **after** the device close (`tx.rs:868-881`). The recommendation is unchanged: Close is not an op now; revisit at E3 or C6 (§1).
- **(d1).** Narrowed as Sol words it (§8 item 2).
- **Note 49's Configure-refusal wording.** It must distinguish a loss from a non-lost refusal, as UR-26 does. This is listed as a `plan/` amendment for the orchestrator; there is no `design/` change (§6, §8 item 5).

## Changes after the Opus re-check (`recheck-plan.md`, N1–N6; all adopted by the orchestrator)

- **N1 (major). `take` and `end_open` assert that no operation is in flight.**
  - With the in-flight op moved from `Open` to the book, today's `txbook_take_untake_and_end_open_keep_one_open_booking_and_change_no_record` (`crates/ezsdr-radio/tests/staging.rs:1748-1762`) would panic: it ends the open booking with S1 in flight, then takes and issues again. Production never ends an open booking with a Send in flight: every `end_open` caller runs after the send's completion (`tx.rs:879`, `:992`, `:1036`).
  - So `TxBook::take` and `TxBook::end_open` gain `assert!(self.inflight.is_none(), "… in flight")` (§0.2).
  - **Changed test:** `txbook_take_untake_and_end_open_keep_one_open_booking_and_change_no_record` completes S1 (`complete(id, Outcome::Accepted(10))`) before `end_open`. Reason: the old sequence (`end_open` with a Send in flight) is unreachable in production and is now refused by the new assertion; the test's subject (one open booking, no record change) is unchanged.
  - **New pure test** `txbook_end_open_with_an_operation_in_flight_panics` (`#[should_panic(expected = "in flight")]`): take, issue a Send, `end_open`.
  - **New row E2B-18**: `end_open`'s "nothing in flight" assert removed; killer the new test.
- **N2. One test function per `should_panic` case.** P2 and P3 are split, one `#[should_panic(expected = …)]` function each, with the exact message (§3.2 below):
  - P2a `txbook_issue_with_a_configure_in_flight_panics`, P2b `txbook_issue_configure_with_a_send_in_flight_panics`, P2c `txbook_issue_reopen_with_a_configure_in_flight_panics` (each `expected = "an operation is in flight"`);
  - P3a `txbook_complete_configure_twice_panics` (`"no operation in flight"`), P3b `txbook_complete_configure_with_a_send_in_flight_panics` (`"not a Configure in flight"`), P3c `txbook_complete_with_a_configure_in_flight_panics` (`"not a Send in flight"`).
  - So E2B-03 is killed by P2b alone and E2B-07 by P3b alone, and no case hides behind an earlier panic.
- **N3. C9(c): a failed reopen with a loss just before its result section.** `TxOpenFails(1)`; at `tx_reopen_done`, arm `on_next_lock(&core.streams, device_lost)`; `pass` inside `released`. Today (`tx.rs:958`, read inside the section): the outcome is "lost", so `chain.lose()`; no ClockEnded `COMMAND_REJECTED`, no "could not be reopened" note, `clock` stays, no `device_failed(None, "tx_reopen")` (`:982`), one `DEVICE_LOST`. Full snapshot (§3.1).
  - **New row E2B-19**: the failed reopen's loss read hoisted (`let lost = core.is_lost();` immediately before `lock(&core.streams)` at `:954`, and `:958`/`:962` read `lost`); killer C9(c) (the mutant takes the `Failed` branch on a lost device: B refused with ClockEnded, `clock = None`).
- **N4. Repeats.** C7, C9(a), C9(c) and R2b run the owner's work on a `released` participant thread, so each is repeated 20 times on sim03 before the report (AGENTS §9 step 8). The other C-tests run on the test thread with a frozen clock and are run twice.
- **N5. C8's hooks.** C8 asserts, before Z′ is booked, that both hooks ran (a counter the hooks increment equals 2) and that the slot is empty (`test_support::next_lock_armed() == false`). The test thread takes the `Control` mutex only outside `turns` (to book X, the value, and Z′), so a hook left armed under a mutant fails the assertion instead of blocking on that mutex.
- **N6. `issue_reopen` before the early returns.** **New row E2B-20**: in `reopen_streamer`, `issue_reopen` moved above the stop/loss/claim early returns (`tx.rs:930-940`), leaving the op in flight when the reopen does not run. Killers: `ur_22_uncertain_reopen_waits_for_owner_claim_and_stops_on_stop_or_loss` (`tx.rs:1481`: a refused claim, then the retry's `issue_reopen` panics "in flight"). Proof the killer runs the path: its first `pass` returns at the claim (`:940`) and its second `pass` reopens.

## Changes in E2b-1 (the characterization as built; base `9c84502`, run on sim03)

- **C5 / Q8 corrected (today's code contradicted the plan).** At a timely configure (R's start still ahead) a switch is refused `NoPlayingRepeat` ("UR-21: a repeat switch needs a playing repeat and no pending switch"), not `NotStarted`: `TxRecord::switch` checks for a chain that has begun (`staging.rs:821`) before `started` (`:827`). A C5 built that way could not kill E2B-02. C5 therefore runs the KL-41 case: R is a SendAsap repeat at 20 ms, configured at 12 ms, and the configure runs to 21 ms (the probe moves device time), past R's start. Then a switch at the probe and one after the turn are refused "UR-38: … not started"; R is moved and sent at 23 ms (TIME_ERROR send_asap, 3 ms); once device time passes R's moved start, a switch is accepted (row pending).
- **C2 as built.** Y is booked at the probe, right after the full Stop (so between the Stop and X's result, which is what P4(b) is about on the device path). Y carries X's generation, so the refused variant uses a one-shot refusal (`test_support::ApplyFails` on channel 0) instead of `TxApplyRefused(0, F2)`, which would refuse Y's own configure too. A new value before Y is refused today ("RM-21: the transmit stream is busy until …": X's value-rule end stays after the Stop), so Y cannot take one. The Stop's bound is a root tick (`staging.rs:1054`); the test books Y at its sample.
- **Rigs.** C0, C1, C4, C6, C7 and C9 use the existing `owner`/`owner_with` rig (no receipts needed); C2, C3, C5 and C8 use `control_rig` (uhd-control with `radio.tx.c0.stream = 0`). The owner rig's record has no clock (`core.register` alone), so C7's "full Stop" books the halt and a record stop that reaches nothing; B was held without a receipt either way.
- **C9 is three test functions:** `e2b_c9b_…` (configure), `e2b_c9a_…` (reopen), `e2b_c9c_…` (failed reopen).
- **Snapshot exclusions as built:** `stats.tx_lock_wait_max_ns`; an event's `source`, `severity` and `time` (kind and payload are compared, in order). Nothing else differed between runs.

**Owner points (pending the owner; the plan keeps today's behaviour for both, §7)**
- The silent drop of a booking whose configure fails because the device was lost: C4(ii) pins today's behaviour. The "one-line fix" claim is withdrawn.
- A failed reopen cancels the open booking whatever `first` is: P5 pins today's rule, and its reachability is INFERRED.

## Changes in E2b-2 (the production change as built; base `9c84502`, E2b-1 `2e53a76`; run on sim03)

- **`TxBook` shape.**
  - The in-flight operation is `ops: Ops` on the book, not `inflight` on `Open`. `Ops` is a small `pub` struct with private fields (the id counter and `Option<(OpId, TxOp)>`), `Default`-built in `TxBook::new`. A `pub` struct is needed because the existing tests build books with `TxBook { record, ..new_book() }`, and a base expression refuses a private field; `TxOp` stays private.
  - `start` (one place) and `finish` (one place: "no operation in flight", "not the operation in flight", "not a <Kind> in flight", all checked before any write) are shared by the six `issue*`/`complete*` functions.
  - `issue` and `complete` return and take the `OpId` (Send included, M1). `issue_configure(k, len)`, `complete_configure(id, bool) -> Option<P>`, `issue_reopen()`, `complete_reopen(id, Reopen) -> Option<P>` as §0.2. `TxBook::clock_ended` is folded into `complete_reopen` and deleted. `take` and `end_open` assert nothing is in flight (N1); `untake` and `cancel_all` read the book's field.
- **Panic messages keep today's words, so the two `expected` strings did not change.** `start` panics "an operation is in flight: a <kind> in flight is issued again", and `finish` "no operation in flight: no <kind> in flight to complete". `txbook_issue_twice_panics` (`"send in flight"`) and `txbook_complete_without_issue_panics` (`"no send in flight"`) are unedited. The plan said "may change"; none did.
- **uhd-tx.**
  - `step` decides `values_turn(clock, k)` on the held key before it takes the booking; only `Start` takes it. `Wait` and a refused claim leave `held` alone.
  - `configure_turn(clock, k, generation)` keeps its three sections. The claim section also reads the held booking's settings and record length and calls `issue_configure`. The mark section is unchanged. `complete_configure` has its own section, then `device_failed` for the booking it returns.
  - `reopen_streamer`: `issue_reopen` in the claim section. The result section decides `Reopened | Failed | Interrupted` from `result`, `core.is_lost()` and `streams.stop` under the lock, calls `complete_reopen`, then does the executor's parts (`halt_failed_reopen`: tracker stop, `finished`, `clock = None`). `take_all_held` keeps its own later section.
  - The precondition check (R1) is a `debug_assert!` in `send`'s plan section, next to `issue`.
- **Tests.** Pure, `crates/ezsdr-radio/tests/staging.rs` (14 new, one function per `should_panic` case): P1 `txbook_a_configure_keeps_its_booking_held`; P2a–c `txbook_issue_with_a_configure_in_flight_panics`, `…issue_configure_with_a_send_in_flight_panics`, `…issue_reopen_with_a_configure_in_flight_panics`; P3a–c `…complete_configure_twice_panics`, `…complete_configure_with_a_send_in_flight_panics`, `…complete_with_a_configure_in_flight_panics`, plus `…complete_reopen_with_a_configure_in_flight_panics`; P4 `…configured_changes_nothing_and_refused_takes_the_booking_and_its_region`; P5 `…a_failed_reopen_ends_the_open_booking_and_the_pending_switch`; P6 `…a_reopen_that_succeeds_or_is_interrupted_changes_nothing`; P7 `…a_completion_names_the_operation_in_flight`; P8 `…end_open_with_an_operation_in_flight_panics`; and `…take_with_an_operation_in_flight_panics` (the `take` assert N1 names, with row E2B-21). `txbook_clock_ended_cancels_the_pending_switch_and_configure_refused_drops_the_piece` is replaced by P4 and P5. Rig, `tx/tests/e2b.rs`: R1 `e2b_r1_…` (`should_panic`) and R1b `e2b_r1b_…` (a lost device: `send` called directly, because `step` returns before `send` on a loss).
- **Changed existing test (N1).** `txbook_take_untake_and_end_open_keep_one_open_booking_and_change_no_record` completes S1 before `end_open`. Spelling only elsewhere: `issue` returns an id and `complete` takes it (all in `staging.rs`; the Mock's pair).
- **Rows.**
  - New: E2B-01 … E2B-21. E2B-21 is the `take` assert's row, which the plan did not list. Each names one killer in `filter`: a row's `crate` and `test_bin` run one test target, so a row cannot name both the pure and the UHD killer. The book rows use the pure killer, the executor rows the rig test.
  - Re-anchored with the same mutation: P1193, P1209 (tracker part only), P1303, P1304, P1305, E2A2A-05, E2A2A-06, E2A2A-08, E2A2A-M1, E2A3-04.
  - Retargeted: N47S4-13b and N47S5-89 (into `complete_configure`'s `self.record.drop_piece(k, len)`, staging.rs; the killers stay the UHD tests), N47S5-44 (into `complete_reopen`'s `self.record.cancel_switch()`), N47S4-15b (the late policy before the configure, on the held key).
  - Not re-anchored but run, because their function changed: N47S4-12, -12b, -14, -56, -20, -51, -11c, -09a, N-11, P1194, P1206, P1208, P1298, P1312, N48S1-02, E1-U14, E2A2B-07, E2A2B-08.

Inputs read:
- the brief;
- AGENTS §3, §7, §9;
- note 49 §5.4 (all of it), §11.4, §12 and §12.1;
- `reviews/e2a-sketch.md` (the owner decisions, §1.1–§1.4, §2 item 5, (d1), §7);
- `reviews/e2a-test-plan.md` and `reviews/e2a-dispositions.md`;
- note 51 §3;
- the code: `staging.rs` (`TxBook`, `Bookings::{halt, drop_piece, cancel_switch}`), `tx.rs` (production 1–1327 and the configure, reopen and switch tests), `control.rs` (`book`, the Stop arm), `core.rs` (`StreamState`, `Streams::{claim_owner, give_back, end, record_*}`, `configure`, `reopen_tx_streamer`, `device_lost`) and `mod.rs` (`Provider::stop`);
- the Mock's `open_held` (`crates/ezsdr-mock-radio/src/lib.rs:955-967`).

Rig units match E2a's: 1 MS/s, 200 root ticks a sample, `MS` = 200 000 ticks, W 10 ms (`e1.rs:12-16`).

Contents:
- 0 what E2b changes, and the proposed shape;
- 1 Close as an op (the recommendation);
- 2 characterization: today's behaviour per sequence, and the test that pins it;
- 3 sequences → tests;
- 4 mutation rows;
- 5 the hardware audit;
- 6 `design/` text;
- 7 open questions;
- 8 where note 49 or the E2a records and the code disagree;
- 9 counts;
- 10 not checked.

---

## 0. What E2b changes, and the shape I propose

### 0.1 Today (VERIFIED)

**The configure turn.**
- `step` takes the first held booking out of `list.held` (`tx.rs:536`) before it decides its values (`tx.rs:542-549`).
- `Values::Configure(g)` → `configure_turn` (`tx.rs:610-637`) runs these steps in order:
  1. `claim_owner` in one section. With no claim, the booking goes back with `hold(held)` and the turn returns false (`:613-616`).
  2. The device call `configure_tx`, with no lock (`:618`), then the probe `tx_configure_before_mark` (`:619`).
  3. One section: write the mark (`Ok` and not lost → `Some(g)`; `Ok` while lost → unchanged; `Err` → `None`), then `give_back` (`:620-628`).
  4. On `Ok`: `hold(held)` in its own section (`:630`).
  5. On `Err`: `book.drop_piece(k, len)` in its own section (`:632`), then `device_failed(action, "tx_burst")` with no lock (`:633`).
- So for the whole device call the booking is in neither `held` nor `open`. That is the "leaves `held`" of e2a-sketch §7.

**The reopen turn.**
- `pass` calls `reopen_streamer` first whenever `chain.needs_reopen()` (`tx.rs:317-319`).
- `reopen_streamer` (`tx.rs:927-987`) runs these steps in order:
  1. One section: `streams.stop` (Provider::stop) or a loss ends the recovery, with no device call (`:930-939`). With no claim, the turn returns false (`:940`).
  2. `reopen_tx_streamer` with no lock (`:944`), between the probes `tx_reopen_claimed` (`:943`) and `tx_reopen_done` (`:945`).
  3. The chain's front from the flush's ACK (`:946-951`).
  4. One section (`:954-968`). On `Ok`: restore the mark, or `None` if lost. On failure with no loss and no `Provider::stop`: `halt_failed_reopen` (`:991-1002`), which takes `book.end_open()` (any `first`), stops the tracker, calls `book.clock_ended()` (`record.cancel_switch()`, `staging.rs:560-562`), `streams.finished`, and sets `clock = None`. Otherwise `chain.lose()` or `chain.provider_stop()`. Then `give_back`.
  5. `sync_streamer`; on `Ok`, `reopen_succeeded`.
  6. Outside the lock, in this order (`:973-985`): the open booking's `COMMAND_REJECTED` ("UR-22: cancelled because the TX streamer could not be reopened", `ClockEnded`), then `take_all_held()` in another section with one `COMMAND_REJECTED` each, then `device_failed(None, "tx_reopen")`.

**Who else touches what.**
- No other thread reads or writes `list.held` or `open`. `TxList::held` says "Only the executor writes it" (`staging.rs:273`).
- uhd-control's receipt reads `playing` and the record and appends to `unread` (`control.rs:521-604`).
- A Stop writes the record (`stop_at`) and appends a halt (`control.rs:317,322`).
- `Provider::stop` writes `streams.stop`, the record and a halt (`mod.rs:691-697`, `core.rs:645-650`).
- A loss writes the record and clears the mark, with no halt (`core.rs:636-650`).
- So the only booking-level facts another thread can change during the configure or reopen device call are the record, `unread` and `playing`. `held` and `open` stay as they were.

### 0.2 E2b (proposal; the orchestrator may override the names)

**`TxBook` (`staging.rs`).**
- Today's `Open::inflight` (`staging.rs:332`) is replaced by one `inflight: Option<(OpId, TxOp)>` on the book, with `enum TxOp { Send(TxSend), Configure { k: i64, len: Option<i64> }, Reopen }`.
  - One field holds the one fact "what is in flight on this stream, and which issue it was" (Maintainability 2). Two fields with an invariant between them were the alternative, and I reject it.
- **Tokens (M1).**
  - `OpId` is an opaque `Copy + Eq + Debug` newtype over a `u64` counter that the book owns (`next_op`). Each `issue*` increments the counter and returns the new id, so no two issues of one book share an id.
  - Every `complete*` takes the id. It asserts, **before any write**, three things:
    1. something is in flight ("no operation in flight");
    2. its id is the given one ("not the operation in flight");
    3. its kind is the function's kind ("not a Configure in flight", and so on).
  - A stale, duplicate or wrong-kind completion therefore panics and leaves the op in flight untouched.
  - These are `assert!`, not `debug_assert!`, as E2a's `issue` check is (`staging.rs:424`).
  - Send is included: `issue(TxSend) -> OpId` and `complete(OpId, Outcome) -> Settle`.
    - The UHD id stays a local of `send` from the plan section (`tx.rs:689`) to its two completions (`:717`, `:739`, through `complete_send`, `:839`); VERIFIED, all in `fn send` (`:642`).
    - The Mock's per-block pair (`ezsdr-mock-radio/src/lib.rs:1930-1931`) becomes `let id = …issue(..); …complete(id, ..)`.
    - The pure tests that call `issue`/`complete` (14 lines in `crates/ezsdr-radio/tests/staging.rs`, by `grep -rn '\.issue(TxSend\|\.complete(Outcome'`) change their spelling only.
  - Rejected alternative: tokens on Configure and Reopen only. One field would then have two completion shapes, and Sol's replay hole would stay open for Send.
- `issue_configure(k, len) -> OpId`:
  - asserts nothing is in flight and that `held` holds `k`;
  - does **not** take the booking out of `held` (it stays in the book);
  - does **not** touch `playing`.
- `complete_configure(id, configured: bool) -> Option<P>`:
  - checks the token as above and takes the op;
  - `true` changes nothing;
  - `false` removes `held[k]`, runs `record.drop_piece(k, len)` and returns the booking (today's `:632`).
- `issue_reopen() -> OpId`.
- `complete_reopen(id, Reopen) -> Option<P>`, with `enum Reopen { Reopened, Failed, Interrupted }`:
  - only `Failed` changes the book: `open.take()` (whatever `first`, as `halt_failed_reopen` does today, `tx.rs:992`) and `record.cancel_switch()` (today's `clock_ended`, `:996`);
  - it returns the open booking for the executor to report.
  - It does **not** take `held`. The executor's `take_all_held` keeps its own later section, as today (`:977`; M2).
- `TxBook::clock_ended` (`staging.rs:560`) is folded into `complete_reopen` and deleted (Maintainability 4). `TxBook::drop_piece` stays: the stale-held branch and dispatch's `Drop` still call it (`tx.rs:515,1120`).
- `cancel_all`'s and `untake`'s debug assertions read `inflight` (`staging.rs:412,467`).

**uhd-tx (`tx.rs`).**
- `step` decides `values_turn` on the first held booking **before** it takes it. Only `Start` takes it (today's order of checks is unchanged).
- `configure_turn` keeps today's three sections (M2):
  1. Claim, or return false with nothing issued; then `issue_configure`. Then the device call, then the probe `tx_configure_before_mark`.
  2. The mark, read against `core.is_lost()` inside this section as today (`:620-628`); then `give_back`. **Nothing else** happens in this section.
  3. In its own section, `complete_configure(id, ok)`: for `Ok`, the position of today's `hold` (`:630`); for `Err`, the position of today's `drop_piece` (`:632`).

  Outside the lock it then reports `device_failed(action)` for a returned booking (`:633`).
  - Between sections 2 and 3 the op stays in flight with its result known only to the executor. No other thread reads `inflight` or `held` (`staging.rs:273`; uhd-control reads the record and `playing`, `control.rs:521-604`).
- `reopen_streamer`: claim and `issue_reopen` in the claim's section.
  - In the result section, the outcome is decided from `result`, `core.is_lost()` and `streams.stop`, all read under that lock (today's `:956-966`).
  - Then, in that same section, `complete_reopen(id, …)`, the tracker stop, `finished` and `clock = None` (today's `halt_failed_reopen`, which already runs in that section).
  - The held bookings are taken in their own later section by `take_all_held`, as today (`:977`).
  - Outside the lock: the reports in today's order.
- One test-build check in `send`'s plan section, under the same lock as `issue`. A first send that opens a device burst (`device_sob`) for a booking of generation `g` has `stream.configured == Some(g)`, or the device is lost. This is note 49 §5.4.6's "configure is the precondition of payload issue" in an observable form. The precondition itself stays where it is today: `values_turn` and `device_holds` read the device record's mark (`tx.rs:588-604`). The mark is the device record of UR-36. Arm, a loss and an open also write it (`mod.rs:562`, `core.rs:636-639,697-724`), so moving it into the book would duplicate it.

**The Mock.** No change. It has no configure step with a result and no reopen: `open_held` sets the burst's values at its first sample (`lib.rs:955-967`, MR-33). Its per-block `issue`/`complete` pair only changes to pass the token (`lib.rs:1930-1931`; M1). Its behaviour does not change.

**Mechanism differences.** No section merges or splits; every record write stays in the section it has today (M2).
- (m1) and (m2) of the first draft are **withdrawn**. (m1) was observable: C8 pins it.
- (m3) remains. The booking under configure stays in `held` rather than in the executor's hand. Only the executor reads or writes `held` (`staging.rs:273`; a loss writes the record and the mark, and `Provider::stop` also appends a halt to `unread`, `core.rs:635-650`, `mod.rs:691-697`). So a receipt cannot see the difference. Shutdown reads `held` on the executor thread, after the turn has returned (`tx.rs:296,1305`).

No device call, event, record row or refusal changes. §2's characterization tests (C0–C9, full snapshots) and §3.3's trace comparison are the check.

**Sub-steps.** One step, one local commit. It is small: about 200–350 lines with tests, INFERRED. A split into Configure, then Reopen, buys little: they share one field.

## 1. Close as an op: not now (recommendation)

**Recommendation:** Close stays an executor device call that a book transition decides. It joins the op protocol in the stage whose rule first reads a Close result. Revisit at E3 (the device-state move) or C6; neither is claimed to be necessarily the first consumer (Sol).

**Evidence (VERIFIED).**
- The result of `close_device_burst` (`tx.rs:887-925`) changes only `TxChain`: `Ok(2)` → true, so the caller advances the front (`:873`, `:1027`); short or failed → `mark_reopen_needed` or `lose` (`:905-923`).
- No booking-level write depends on the close's result. (Corrected after Sol's review: the first draft said every such write comes before the close, which is false for `end_open`.)
  - `end_open` (`tx.rs:860-883`) calls the close (`:872`) and **then** `book.end_open()` (`:879`). It ends the open booking whatever the close returned: the close's `bool` only gates `chain.advance_front` (`:872-873`), and a lost device only gates `chain.lose()` (`:876-877`).
  - `abandon` calls `book.abandon(...)` (`:1017`) before the close (`:1025`).
  - `close_unjoined` closes the record (`book.close_unjoined`, `:575`) before `end_open(true)` (`:579`).
  - So the order differs by site, but at none of them does the book read the close's outcome.
- No receipt answer reads the close's outcome. The record predicts the front itself (P3) until E3.
- The consumers note 49 names do not need it:
  - C1 (re-segment after a refused configure) reads only which pieces were issued, which the book has once Configure is an op.
  - C2 (honest switch outcome) reads whether the `Send` carrying `b` was accepted (e2a-sketch §7).
  - C6 (shrink an abandoned region to "what was sent and its close") may read the close's success, because a failed close leaves the front unadvanced and recovery pending. E3's move of the device state into the book may need it earlier (INFERRED for both).

**Cost of doing it now.** One more lock section around each of the two close sites, a fourth `TxOp` variant, and test churn. No behaviour would change, and nothing would read the result. Rejected as scope creep (AGENTS §6).

## 2. Characterization: today's behaviour per sequence

How the observations are made:
- **calls**: `FakeDevice::calls()` (`apply tx …`, `tx_open …`, `tx_send … at= sob= eob=`);
- **rec**: `lock(&core.rec)`, i.e. `rejected` notes, `applied`, `bursts`, `switches`;
- **ev**: `COMMAND_REJECTED`, `TIME_ERROR`, `DEVICE_LOST`;
- **mark**: `stream(key).configured`;
- **book**: `held_keys`, `open_*` and `playing`.

"none" means no existing test pins the behaviour. Those get a **characterization test that passes on `9c84502` before any production change** (C-tests, file `crates/ezsdr-radio-uhd/src/provider/tx/tests/e2b.rs`, run red-first per §3.3).

| # | Sequence | Today (code) | Pinned by |
|---|---|---|---|
| Q1 | A booking with a new generation: configure succeeds, then the payload | calls: `apply tx` (per channel) before the burst's first `tx_send`, and after the previous device burst's front (`apply_due`, `tx.rs:254-256`). mark `Some(g)`. rec: one `applied` row per key and channel (`core.rs:1140-1153`). The late policy is decided after the turn (`:542-551` on the next turn) | `ur_25_a_value_after_a_burst_ends_is_applied_before_the_next_burst` (`tx.rs:6364`), `ur_25_a_slow_configure_makes_a_burst_late_by_its_policy` (`:6622`, no send before the apply; lateness read after the configure), `ur_23_a_value_waits_for_*` (`:6561,6583,6603`). The `applied` row of a **transmit burst's** configure: none (→ **C0**). "No send before the result": only by call order, not at the result instant (→ C0) |
| Q2 | Same generation back to back; different generations back to back | Same: no `apply`, one device burst (`device_holds`, `:528,601-604`). Different: the device burst closes, then `apply`, then the next payload | `ur_23_a_contiguous_burst_with_the_same_generation_makes_no_configure_call` (`:6397`), `ur_25_an_equal_value_after_a_new_one_configures_once` (`:6413`), `ur_23_a_burst_at_a_held_tail_with_a_new_generation_is_configured_before_it` (`:6382`), `ur_25_a_burst_whose_snapshot_equals_the_device_s_makes_no_apply` (`:6352`), `ur_23_apply_is_due_only_between_bursts_and_only_for_a_new_generation` (`:6339`, pure) |
| Q3 | Configure refused | ev: one `COMMAND_REJECTED{kind: tx_burst, action: X}`, nothing else. mark `None`. No `tx_send` for X. X's region leaves the record, its value-rule end stays (`drop_piece`). A later burst of that generation configures again | `ur_21_a_refused_configure_refuses_the_burst_and_the_fixed_value_plays_on_the_same_clock` (`:6428`), `ur_21_a_partial_configure_refusal_sends_nothing_and_the_retry_configures_both` (`:6454`), `ur_21_a_refused_burst_stays_counted_until_its_booked_end` (`:6473`), fake.rs `ur_21_a_refused_configure_refuses_its_burst_through_the_kernel` (`:1887`) |
| Q3b | Configure refused, later bookings behind it (KL-38) | The surviving short head is closed at its standalone front; pieces joined behind it are late at dispatch | `ur_21_s2_a_short_surviving_head_is_closed_and_a_piece_joined_behind_it_is_late` (`:3067`), `ur_21_s2_c_and_d_joined_behind_a_refused_head_are_late_at_dispatch` (`:3093`) |
| Q4a | Stop (list halt only) during a successful configure | The configure completes, mark `Some(g)`. The next turn reads the halt and X is cancelled with one note `UR-26: cancelled by Stop`. No `tx_send` | `ur_22_a_stop_during_the_configure_leaves_no_payload_after_it` (`:6515`) |
| Q4b | Stop (list halt only) during a refused configure | X gets `COMMAND_REJECTED` and **no** Stop note (it was "in the owner's hand"). No `tx_send` | `e1_q7_a_stop_received_while_a_configure_refuses_its_burst` (`e1.rs:262`) |
| Q4c | A **full** Stop (record `stop_at` + halt, as `control.rs:317,322`) during the configure, with a booking Y accepted after the Stop | As Q4a or Q4b for X. The record's halt removed X's chain at receipt (X.k ≥ t). The refused case's `drop_piece(X.k)` then finds no chain (`staging.rs:946-950`), and Y keeps its chain and plays | none (→ **C2**) |
| Q4d | Stop received **after** the configure's result and before the next turn | The next turn's guard reads the unread halt before anything else (`tx.rs:471-485`). X is cancelled with a note, no send | none (→ **C3**) |
| Q5a | `Provider::stop` during a successful configure (between two channels' applies) | Both applies complete; mark `Some(g)` (the configure does not check `streams.stop`); claim given back. `shutdown` → one note `UR-26: cancelled by Provider::stop`, no send | `ur_22_a_provider_stop_during_the_configure_keeps_the_mark_and_cancels_the_burst` (`:6648`) |
| Q5b | `Provider::stop` during a refused configure | `COMMAND_REJECTED` for X once, no ProviderStop note, no send | none (→ **C6**) |
| Q6a | Loss after the device call, before the mark (configure `Ok`) | mark stays `None` (`:623` guard; `end(lost)` cleared it, `core.rs:636-639`). X back in `held`. No send. `shutdown` notes X (`ProviderStop`) | `ur_22_a_loss_during_the_configure_clears_the_mark` (`:6534`, mark and no send only). The shutdown note: none (→ C4's first half) |
| Q6b | Loss **between two channels** (configure `Err{lost}`, `core.rs:1136-1137`) | `drop_piece`; `device_failed` → `device_lost`, a no-op once lost (`core.rs:1047-1049`). So X gets **no** `COMMAND_REJECTED` and, being out of `held`, **no** shutdown note. mark `None`. One `DEVICE_LOST` | none (→ **C4**'s second half; see Q2 in §7) |
| Q6c | Loss after the configure's result, before the next turn | `step` sees the loss: `chain.lose()`, false (`tx.rs:464-467`). `shutdown` notes X | generic: `ur_26_shutdown_after_loss_cancels_held_bursts_record_only` (named in UR-26). Folded into C4 as a third case |
| Q7 | Configure claim refused (another holder) | No device call; X stays held; the turn returns false. Once the claim is back, the configure runs | none (→ **C1**) |
| Q8 | A switch received during its repeat's configure (R a repeat with a new generation) | R has issued nothing, so `playing.started` is false. `receive_switch` refuses with `NotStarted` (`staging.rs:827-829`, `control.rs:567-575`); no row; R unaffected. After R's first send, a switch is accepted | KL-41 names "the stream's configure held its first send back", but its evidence tests are host-stall cases. Configure case: none (→ **C5**) |
| Q9a | Reopen succeeds | calls: the second `tx_open` → T3 `tx_send 0 n=4 at=- sob=true eob=true` → payload. mark restored. The front moves only on an acknowledged flush. Old correlations dropped | `ur_22_underflow_close_failure_reopens_before_the_next_payload` (`:1605`), `ur_22_tx_stop_preserves_recovery_for_later_bursts` (`:1833`), `ur_22_owner_keeps_clean_recovery_after_an_unacknowledged_t3` (`:1709`), `ur_22_a_reopen_keeps_the_mark_and_a_loss_clears_it` (`:6689`), `ur_28_a_value_during_an_underflow_recovery_is_applied_after_the_recovery` (`:6719`) |
| Q9b | Reopen fails | The open booking and each held one get `COMMAND_REJECTED` "UR-22: cancelled because the TX streamer could not be reopened". The pending switch is cancelled (row `cancelled`). `clock = None`. The tracker record ends `stop`. A later booking gets `COMMAND_REJECTED` "UR-25: its transmit clock ended before the burst could start" (`tx.rs:401-404`). Then `device_failed(None, "tx_reopen")` | `ur_22_underflow_close_failure_reopens_before_the_next_payload` (reopen-fault rows), `ur_22_nonterminal_reopen_failures_cancel_held_initial_bursts` (`:1771`), `ur_22_f8_recovery_that_ends_the_repeat_cancels_the_pending_switch` (`:7131`) |
| Q10a | `Provider::stop` or a loss during the reopen (at the claim, or at the T3 ACK return) | The claim is returned; the chain is `Clean`; the held burst is **not** refused; only T3 is sent; no device op after the loss | `ur_22_stop_and_loss_during_owner_recovery_release_the_claim` (`:1519`; uses barriers, so its verdict does not depend on scheduling), `ur_22_uncertain_reopen_waits_for_owner_claim_and_stops_on_stop_or_loss` (`:1481`), `ur_22_a_loss_during_the_reopen_clears_the_mark_the_reopen_read` (`:6704`), `ur_26_provider_stop_ends_recovery_after_stop_and_keeps_shutdown_report_reads` (`:1798`) |
| Q10b | A **Stop** (not `Provider::stop`) during the reopen | The Stop does not interrupt the reopen (`:932` reads only `streams.stop`). With the reopen succeeding: after it, the halt cancels held B with one Stop note and B is not sent. With the reopen failing: B is cancelled first with the ClockEnded `COMMAND_REJECTED`, and the later halt cancels nothing (note 49 §5.4.4 m1: the result first) | none (→ **C7**) |
| Q11 | A receipt between a refused configure's mark section and its `drop_piece` (Sol M2) | Section 1 (`tx.rs:620-628`) writes mark `None` and gives the claim back. X's chain is still in the record until `drop_piece` (`:632`) runs in its own section. A receipt in between of a burst Z overlapping X's span is refused "UR-21: the burst overlaps an accepted booking" (`control.rs:581-584`). After the turn, the same Z is accepted ("the refused burst's span is free", `tx.rs:6497-6498`) | after the turn: `ur_21_a_refused_burst_stays_counted_until_its_booked_end` (`:6473`). In the gap: none (→ **C8**) |
| Q12 | A loss published after the device call returned and immediately before the owner takes `core.streams` for the result (configure `:621`; reopen `:954`) (Sol M3) | The loss's `end(lost)` clears the mark under the lock (`core.rs:635-639`, from `device_lost_locked`, `:1046-1056`). The owner then reads `core.is_lost()` **inside** its section and writes nothing (configure: the `Ok(())` arm, `:624`) or `None` (reopen, `:957`). Mark `None` | at the earlier probes only (`ur_22_a_loss_during_the_configure_clears_the_mark` `:6534`, `ur_22_a_loss_during_the_reopen_clears_the_mark_the_reopen_read` `:6704`). These do not separate a read inside the lock from a read just before it. At the lock: none (→ **C9**) |

## 3. Sequences → tests

### 3.1 Characterization tests (new file `tx/tests/e2b.rs`; each green on `9c84502`)

The rig is `configure_rig(faults)`: `switch_rig_with` (`tx.rs:6810`) with uhd-control's config holding `radio.tx.c0.stream = 0`, as `ur_21_a_refused_burst_stays_counted_until_its_booked_end` builds it (`tx.rs:6487-6491`). It uses the manual clock and a `Frozen` device. A new generation is booked with `book_frequency(&mut rig.control, …, F2)` (`:6318`), then bursts with `book`, `book_repeat` and `book_switch`.

Probes run on the thread that runs the owner's code, with no lock held (`tx_configure_before_mark`, `tx_reopen_claimed`, `tx_reopen_done`). A probe that stops does the Stop's two writes as `control.rs:317,322` do: `stream.stop_at(now, core.tx_in_flight())` and `book.list.halt(Stop)`. Probes hold the Core weakly (`e1.rs:158`).

**Timing (Sol minor 2, note 51 P2).**
- Every C-test that runs a reopen (C7, C9(a), C9(c)) runs the owner's work, the `pass` calls and the turns after it, inside `test_support::released(frozen, &device.host(), move || { …; tx })` (`test_support.rs:170-185`, used today at `tx.rs:4512,4577` and `o1.rs:734`).
  - That work is a clock participant. The device's time therefore runs only while the work runs, and the clock is held again before `released` returns.
  - No C-test drops `Frozen` and calls `pass` on the unregistered test thread (today's pattern at `tx.rs:6708,6730`, which E2b leaves alone; it is note 51's to migrate).
  - If note 51 stage 1a's `drive()` has landed when E2b is implemented, it replaces `released` there.
- The configure C-tests need no device time to pass during the configure: `FakeDevice::apply` does not wait (INFERRED from the existing configure tests, which run on a held `Frozen`, `tx.rs:6534-6545`). They use the existing `turns`/`device_at` recipe.
- No case is excluded because its trace varies. A case whose snapshot differs between two base runs is a defect of the test, fixed before E2b's production edit.

**Snapshots (Sol minor 1).**
- Each C-test ends with `assert_eq!(snapshot(&rig), expected)`. `snapshot` is one helper in `tx/tests/e2b.rs` returning, in order:
  1. `device.calls()`;
  2. `rec.applied`;
  3. `rec.bursts`;
  4. `rec.rejected`;
  5. `rec.switches`;
  6. every event drained from the rig's sink, as `(kind, payload)` in emission order.
- `expected` is written as literal JSON in the test from the red-first base run, after a reader checks it against §2's "Today" column.
- Also in the snapshot: `rec.timing` and `rec.device_async`. Both are on the Core's manual clock in these rigs, so they are expected to be stable (INFERRED; checked by the two base runs).
- Excluded, with reasons:
  - from `rec.stats`, the lock-wait maxima (`tx_lock_wait_max_ns`, E2a). They measure lock contention, not behaviour, and the token bookkeeping legitimately changes what runs under the lock. The counter stats (`tx_bursts`, `tx_samples`, `tx_errors`, …) are kept.
  - Any other field that the two base runs show to differ is named in the test, with its reason. An unexplained exclusion is a finding.
- Intermediate asserts (at a probe or between turns) are kept where the sequence needs them (C0, C2, C5, C8).

| Test | Sequence | Asserts | Obs | Wrong behaviour that fails it |
|---|---|---|---|---|
| **C0** `e2b_c0_a_configure_records_its_applied_rows_before_the_burst_s_first_send` | value F2; burst X at 20 000; turns 1–30 ms; the probe `tx_configure_before_mark` records `calls()` and `rec.applied` | at the probe: no `tx_send` yet, and the `applied` row `radio.tx.c0.frequency_hz` with `claimed` F2 is present; afterwards the first `tx_send` carries `at=instant(20 000) sob=true`, and there is one `apply tx` | calls, rec | a Send issued before the Configure's result; a configure whose `applied` rows are lost |
| **C1** `e2b_c1_a_configure_waits_for_the_stream_claim` | the test takes `claim_owner`; `step` | `step()` is false; no `apply tx`; `held_keys == [X.k]`; mark unchanged. After `give_back`, turns: one `apply`, then X sent | calls, book, mark | a Configure issued, or X taken, without the claim; X lost when the claim is refused |
| **C2** `e2b_c2_a_full_stop_during_the_configure_settles_x_and_keeps_y` | success and refused (`TxApplyRefused(0, F2)`) variants; at the probe a full Stop at t ≤ X.k; then Y (finite, `DropAndFlag`, start ≥ the Stop's bound) via uhd-control; turns to Y's end | success: one note `UR-26: cancelled by Stop` for X, mark `Some(1)`. Refused: `COMMAND_REJECTED` [X], no note. Both: no sample of X (`transmitted`); Y plays whole; a booking that overlaps Y is refused at receipt (Y's chain survived X's `drop_piece`) | rec, ev, calls, rcpt | `complete_configure(false)` removing a chain other than X's (Y's); the refused X also noted by the halt (result after the Stop) |
| **C3** `e2b_c3_a_stop_after_the_configure_result_cancels_the_burst_at_the_next_turn` | `step()` once (the configure turn returns true); then a full Stop; then `step()` | one Stop note for X; no `tx_send`; the second `step` reads the halt first | rec, calls | the Configured result moving X out of `held` (no note), or into `open` (a send) |
| **C4** `e2b_c4_a_loss_around_the_configure` | (i) 1 channel, loss at `tx_configure_before_mark`; (ii) 2 channels, loss at the second `apply` probe (`Err{lost}`); (iii) loss after the turn returned, before the next `step`; then shutdown each | (i) and (iii): mark `None`, no `tx_send`, one note `UR-26: cancelled by Provider::stop` for X after `shutdown`. (ii): no `COMMAND_REJECTED` with X's id, **no** note for X, mark `None`, one `DEVICE_LOST` (today's; see §7 Q2) | rec, ev, mark | a lost `Err` turned into "interrupted, keep X held" (a new note in (ii)), or `Ok`-while-lost turned into a refusal (a missing note in (i)) |
| **C5** `e2b_c5_a_switch_during_its_repeat_s_configure_is_refused_until_the_repeat_sends` | value F2; R repeat at 2 000; at the probe: `book.receive_switch(..)` under the streams lock on the live book; after the turn: `book_switch` via uhd-control; then turns until R's first send; then `book_switch` again | at the probe: `Err(NotStarted)` and `playing.started == false`. After the turn: one refusal with the UR-38 "not started" text and no `switches` row. After R's first `tx_send`: the switch is accepted (row pending) | book, ev, rec | a Configure counted as the repeat's first issue (`issue_configure` setting `started`): the switch accepted while nothing of R was sent |
| **C6** `e2b_c6_provider_stop_during_a_refused_configure_reports_the_refusal_once` | `TxApplyRefused(0, F2)`; at the `apply` probe, `streams.stop` + `end` + `Shutdown` as `:6656-6664` does; turn; `shutdown` | ev `COMMAND_REJECTED` [X] once; `rec.rejected` has no `cancelled by Provider::stop` for X; no `tx_send`; claim free | ev, rec | X both refused and noted; the claim kept |
| **C7** `e2b_c7_a_stop_during_a_reopen_is_read_after_its_result` | recovery pending, B held (as `:1529-1530`); at `tx_reopen_claimed` a full Stop; `pass`, then `pass`, both inside `released`. Two variants: the reopen succeeds; it fails (`TxOpenFails(1)`) | succeeds: T3 sent, B gets one Stop note, no B payload. Fails: B gets the ClockEnded `COMMAND_REJECTED` and no Stop note; `clock` is none. Full snapshot | rec, ev, calls | the halt read before the result (B noted where it should be refused), or the reopen abandoned by a Stop (no T3) |
| **C8** `e2b_c8_a_receipt_between_a_refused_configure_s_mark_and_its_drop_is_refused` (Q11, Sol M2) | the rig of `ur_21_a_refused_burst_stays_counted_until_its_booked_end` (`tx.rs:6476-6494`: `TxApplyRefused(0, F2)`, value F2, X = 4 samples at 20 000, `SendAsapAndFlag`), with `control` in an `Arc<Mutex<Control>>`. `Control` is `Send`: it is moved into the uhd-control thread (`mod.rs:619,666`). At `tx_configure_before_mark`, arm `on_next_lock(&core.streams, …)`; that hook fires before the mark section's lock (`:621`) and arms a second `on_next_lock`. The second fires before the owner's next streams lock (today `drop_piece`'s section `:632`; after E2b `complete_configure`'s section 3) and books Z (1 sample at 20 002, `DropAndFlag`) through the real `Control` (`book_burst_on`). Then the turn ends; Z′ (the same at 20 002, a new id) is booked | inside the gap: mark already `None` and the claim free (read in the hook), and Z refused with "UR-21: the burst overlaps an accepted booking". After the turn: `COMMAND_REJECTED` for X (action 2) and Z′ accepted ("the refused burst's span is free"). Before Z′ is booked (N5): both hooks ran (count 2) and the slot is empty; the test thread holds no `Control` guard during `turns`. Full snapshot | rcpt, ev, mark | the book's result applied in the mark's section (E2B-16): Z accepted in the gap; `drop_piece` skipped: Z′ refused |
| **C9** `e2b_c9_a_loss_just_before_the_result_section_leaves_the_mark_cleared` (Q12, Sol M3) | (a) **reopen**: mark `Some(1)`, `mark_reopen_needed`, inside `released`. At `tx_reopen_done`, arm `on_next_lock(&core.streams, || core.device_lost("test"))`; `pass`. (b) **configure**: value F2, burst X at 20 000; at `tx_configure_before_mark`, arm the same hook; turns. Each hook sets a flag when it runs | (a) the hook ran once; `tx_open` was called; mark `None`; `is_lost`. (b) the hook ran once; mark `None`; no `tx_send`. After `shutdown`, X is noted `UR-26: cancelled by Provider::stop`, as C4(i). (c) (N3) **failed reopen**: `TxOpenFails(1)`, B held, the same hook at `tx_reopen_done`, inside `released`: the hook ran once; no "could not be reopened" note and no ClockEnded refusal; `clock` kept; one `DEVICE_LOST`; B still held. All: full snapshot | mark, calls, rec, ev | the loss read outside the result section (E2B-11 for reopen, E2B-14 for configure, E2B-19 for the failed reopen): mark `Some(1)` or `Some(g)` on a lost device (`core.rs:635-639` cleared it, and the owner wrote it back); (c) the clock halted and B refused on a lost device |

### 3.2 Pure book tests (`crates/ezsdr-radio/tests/staging.rs`; red until the transitions exist)

| Test | Asserts | Wrong behaviour |
|---|---|---|
| **P1** `txbook_a_configure_keeps_its_booking_held` | after `issue_configure(k, len)`: `held` still holds `k`; `record`, `playing` (with `started`), `unread` and `open` equal before (`PartialEq` on `Bookings`, field compares on the rest) | the booking taken out (today's hand-off); `playing.started` set |
| **P2a/b/c** (N2: one function each, `#[should_panic(expected = "an operation is in flight")]`): `txbook_issue_with_a_configure_in_flight_panics`; `txbook_issue_configure_with_a_send_in_flight_panics`; `txbook_issue_reopen_with_a_configure_in_flight_panics` | each panics | a Send issued before the Configure's result (note 49 §5.4.6) |
| **P3a/b/c** (N2: one function each): `txbook_complete_configure_twice_panics` (`"no operation in flight"`); `txbook_complete_configure_with_a_send_in_flight_panics` (`"not a Configure in flight"`); `txbook_complete_with_a_configure_in_flight_panics` (`"not a Send in flight"`) | each panics | a result applied twice or to the wrong kind |
| **P8** `txbook_end_open_with_an_operation_in_flight_panics` (N1; `should_panic(expected = "in flight")`): take, issue a Send, `end_open` | panics | an open booking ended under a Send in flight (its completion would find no booking) |
| **P7** `txbook_a_completion_names_the_operation_in_flight` (M1; one case per kind ∈ {Send, Configure, Reopen}, each run with `std::panic::catch_unwind`) | (i) **stale same-kind replay** (Sol's sequence): issue X → `a`, complete(`a`), issue Y → `b`, complete(`a`, the result that would change the book: Configure `false`, Reopen `Failed`, Send `Failed`) panics "not the operation in flight". The book is then equal to the book before the replay (`held` keeps Y; `open`, `record` and the pending switch are unchanged), and complete(`b`, …) then succeeds with Y's result. (ii) **wrong token**: issue → `a`; complete(an id never issued) panics, and complete(`a`) then succeeds. (iii) **duplicate**: complete(`a`) twice; the second panics "no operation in flight". (iv) **ids unique**: three issues on one book give three different ids | the replay consuming Y's op (Y removed, or the switch cancelled by X's stale `Failed`); ids reused across issues |
| **P4** `txbook_configured_changes_nothing_and_refused_takes_the_booking_and_its_region` | `true`: book equal to before, returns `None`. `false`: returns the booking; `held` without `k`; record equal to `record.drop_piece(k, len)` on a clone. Cells: (a) X's chain present; (b) after `stop((bound, Some((t ≤ k, s))))` with Y booked at ≥ s: Y's chain intact, X's gone already, `drop_piece` a no-op; (c) after a Stop with t > k: X's capped chain dropped | `drop_piece` skipped (region kept, KL-38 cells change); a chain other than X's removed |
| **P5** `txbook_a_failed_reopen_ends_the_open_booking_and_the_pending_switch` | book with `open` (`first` true, and a `first` false cell), two held, a pending switch, `playing` set: `Failed` returns `Some(open)`, leaves `open` none, settles the switch `Cancelled`, and leaves `record` otherwise equal to `record.cancel_switch()` on a clone. `held` and `playing` are unchanged: the executor takes `held` in its own later section (M2), and `halt_failed_reopen` does not touch `playing` today | the switch not cancelled (W7); an issued open kept (a change from today's `end_open`); held taken inside the result section (a merge of today's two sections) |
| **P6** `txbook_a_reopen_that_succeeds_or_is_interrupted_changes_nothing` | `Reopened` and `Interrupted` return `None`; book equal | an interrupted reopen cancelling held or the open booking (Q10a) |

`txbook_clock_ended_cancels_the_pending_switch_and_configure_refused_drops_the_piece` (`staging.rs:1679`, E2a's P5) is **replaced** by P4 and P5. Its W6 half becomes P4(a) and its W7 half P5. `txbook_issue_twice_panics` and `txbook_complete_without_issue_panics` keep their sequences, spelled with tokens. Their `expected` strings follow the unified field's messages ("in flight", "no operation in flight"). P2's cases need no token: an issue checks only that nothing is in flight.

### 3.3 Wiring, ordering and the rest

| Check | How | Expected |
|---|---|---|
| Device call order **and records** unchanged (Sol minor 1) | E2a's out-of-tree trace comparison (`e2a-test-plan.md` §2.2: entry-side attempt log + `calls()` per test thread, base run twice for the stable set) on `cargo test -p ezsdr-radio-uhd --lib`. **Extended:** the same throw-away patch also writes, at `Core`'s drop, `rec.applied`, `bursts`, `rejected`, `switches`, `timing` and the counter stats to `<thread>-<n>.rec.json`. It writes the events (kind, payload, in order) at the sink's drop. Same exclusions as the C-test snapshots. This removes e2a-test-plan §2.2 item 4's exclusion of records for E2b | **zero** differing files (calls, attempts, records, events) on the stable set |
| Every existing assertion | the workspace tests, O1 oracle, parity | green with no assertion edited. Exceptions: the two `expected` strings above and the plumbing of tests that read `open.inflight` (`grep -n 'inflight' crates` outside `staging.rs` returns nothing today, so I expect none) |
| The precondition check | **R1** `e2b_a_first_send_of_a_generation_the_device_does_not_hold_panics_in_the_test_build` (`#[should_panic]`): `poke_open` a `valued` booking of generation 1 with mark `Some(0)`, `step` → the plan section's check fires. **R1b**: the same with the device lost does not panic | panics / does not | a payload issued without a completed Configure |
| F7 (no emit under a lock) | existing `assert_line_free` in `emit`/`command_rejected` (E2a F7); every refused-configure and failed-reopen test runs it | green | `device_failed` or the ClockEnded reports moved inside the result section |
| UR-25 (no lock across a device call) | `ur_25_the_line_lock_is_free_inside_every_transmit_device_call` (`tx.rs:3904-3928`) reaches neither path (VERIFIED: it drives a held tail, a payload, an end of burst and `reports`; no value and no recovery). **R2** is therefore scheduled for both: **R2a** a `valued` burst with a new generation on the `LockSpy` rig, which must see `apply` (`spy.seen("apply") ≥ 1`, after recording the count left by the rig's setup; the spy wraps `Device::apply` as `"apply"`, `test_support.rs:492-500`); **R2b** `chain.mark_reopen_needed()` then `pass` on the same rig, which must see `tx_open` twice (the test's own and the reopen; `"tx_open"`, `:525-527`). Each runs inside `released` if it needs device time. They are new tests beside the existing one, which stays unedited | green; the spy proves each device call ran | the configure or reopen device call made inside the claim, issue or result section |
| The `on_next_lock` seam (M2, M3) | test build only: a thread-local one-shot slot `(mutex address, Box<dyn FnOnce()>)` in `test_support.rs`, armed by `on_next_lock(&mutex, f)`. `core::lock`'s existing `#[cfg(test)]` block (`core.rs:60-66`) checks the slot **once per call, before** `note_lock`/`entered_lock` and before acquiring. If the address matches, it takes `f` out of the slot and runs it, so `f`'s own locks and a re-arm inside `f` act on later calls. A test arms it from a probe running on the owner's thread, so the hook fires on that thread's next acquisition of that mutex, with no guard held. Production code is unchanged: `lock` compiles the block only under `cfg(test)`. A pure self-check `on_next_lock_runs_once_before_the_next_lock_of_its_mutex` (another mutex's lock does not fire it; it fires once; a re-arm inside the hook fires on the call after) | green | — (a seam; C8/C9 depend on it) |
| Mock | Mock behaviour unchanged (only the token spelling at `lib.rs:1930-1931`): `mr_33_a_transmit_value_takes_effect_at_the_next_burst_s_start` (`mock_channel.rs:811`), `mr_33_…_fractional_burst_start` (`:853`), `mr_25_a_stop_before_the_burst_with_a_new_value_cancels_it_and_keeps_the_value_of_record` (`:1040`); parity `s14_transmit_values_on_an_empty_stream_decide_the_same_on_both_providers` (`parity.rs:2597`), `s13_parity_b_refused_at_receipt_and_the_value_answers` (`:2064`) | green, unedited | the unified `inflight` breaking the Mock's per-block issue/complete |

No parity test is added. The Mock has no configure op, refusal or reopen. The user-visible answer of Q4 (a Stop note, no samples, the value kept) is pinned on each side by `mr_25_a_stop_before_the_burst_with_a_new_value…` and `ur_22_a_stop_during_the_configure…`.

**Red-first.**
- The first edit adds only test code: the `on_next_lock` seam (test-build only, inside `lock`'s `#[cfg(test)]` block), its self-check, `tx/tests/e2b.rs` with C0–C9, and R2a/R2b.
  - It runs on `9c84502` plus that edit, through `rdev.py --host sim03`, twice, before any production edit. C0–C9 and R2 must be **green** both times with identical snapshots (characterization, M2's "on today's code").
  - C8 and C9 green there show that today's code refuses Z in the gap and reads the loss inside the lock.
- P1–P7 and R1 are new API, red by construction (they do not compile on the base).
- The report gives the base runs' output for C0–C9 and R2.

## 4. Mutation rows

Anchors: Rust token sequences (`anchor.py`). A UHD-only behaviour that moves into `TxBook` gets **one** row in `staging.rs` with a pure killer and a UHD rig killer in `filter` (crate `ezsdr-radio-uhd`, as the E2a cross-crate rows do). That is not E2a-1's major: the Mock never calls these transitions, so there is no second Provider row to duplicate.

### 4.1 Existing rows on code E2b changes (`plan/maintenance/tools/mutations.json`, mapped to their enclosing `fn` by a script at `9c84502`)

| Row | Today's anchor (fn) | E2b action |
|---|---|---|
| N47S4-12 | `Err(_) => streams.record_configured(key, None),` (`configure_turn`) | re-anchor, same mutation (the mark stays in the executor's result section) |
| N47S4-12b | `Ok(()) if !core.is_lost() => …Some(generation))` | re-anchor, same; killer `ur_22_a_loss_during_the_configure_clears_the_mark` |
| N47S4-13b, N47S5-89 | `…book.drop_piece(held.k, held.record_len());` (`configure_turn`) | **retarget** into `complete_configure`'s `record.drop_piece(k, len)` (staging.rs). 13b's "reset the record" mutation becomes "drop the whole record", 89's "remove the call" stays. Killers: P4 + `ur_21_a_refused_burst_stays_counted_until_its_booked_end` |
| N47S4-14, N47S4-56 | `core.device_failed(held.settings.action(), "tx_burst", &error);` | re-anchor (the booking now comes back from `complete_configure`); same mutations and killers |
| N47S4-15b | `Values::Configure(generation) => return self.configure_turn(clock, held, generation),` (`step`) | retarget to the new call (configure on the held key, no `take_held` before it), same meaning: "judge the late policy before the configure"; killer `ur_25_a_slow_configure_makes_a_burst_late_by_its_policy` |
| N47S4-11c, -09a | `values_turn` | unchanged (the function stays; check with `rules.py`) |
| N47S4-09, -10, -11b, -15 | `apply_due` | unchanged |
| N47S4-50 | `Some(k) if k == c && self.device_holds(k) => {}` | unchanged |
| N47S5-44 | `streams.stream_mut(key).book.clock_ended();` (`halt_failed_reopen`) | **retarget** into `complete_reopen(Failed)`'s `record.cancel_switch()`; killers P5 + `ur_22_f8_recovery_that_ends_the_repeat_cancels_the_pending_switch` |
| P1209 | `let cancelled = …book.end_open().is_some(); if let Some(record) = self.tracker…` | re-anchor on the tracker part only (the mutation drops the tracker's `stop` record); killer unchanged |
| P1206 | `if halted { for _ in self.take_all_held().into_values() {` | **unchanged** (M2: the held cancellation keeps its own section and code; the first draft's retarget is withdrawn) |
| P1208 | `if open_cancelled { self.report_cancelled_burst(…) }` | re-anchor (the open booking now comes from `complete_reopen`) |
| N-11, P1193, P1194, P1298, P1303, P1304, P1305, P1312, N48S1-02, N47S4-20, N47S4-51 | `reopen_streamer` | re-anchor only if their tokens move; same mutations (checked by `rules.py` "exactly once") |
| P1172, P1198 | the `needs_reopen` gates in `pass` and `send` | unchanged |
| E1-U14 | `return self.cancel_unstarted();` (`receive`) | unchanged |
| E2A2A-05 | `assert!(open.inflight.is_none(), "a send in flight is issued again");` (`TxBook::issue`) | re-anchor on the book-level `inflight`; same mutation; killer `txbook_issue_twice_panics` |
| E2A2A-06 | `let send = self.open.as_mut().and_then(\|open\| open.inflight.take()).expect(…)` (`TxBook::complete`) | re-anchor on the shared take-and-check of the token (one helper used by the three `complete*`); same mutation ("no op in flight" not checked); killers `txbook_complete_without_issue_panics`, P7(iii) |
| Mock `issue`/`complete` call rows, if any (`grep -n 'book.issue\|book.complete' plan/maintenance/tools/mutations.json`) | `ezsdr-mock-radio/src/lib.rs:1930-1931` | re-anchor for the token spelling only |
| E2A2B-07, -08 | `self.open.take_if(\|open\| open.first)` (`cancel_all`) | unchanged tokens (the assert above it changes only) |

The exact set is whatever `rules.py` reports as "not found exactly once" after the edit. Each such row is re-anchored with the same mutation, and the report lists any row that changed meaning.

### 4.2 New rows

| Row (mutation) | File | Killer(s) | Proof the killer runs the path |
|---|---|---|---|
| E2B-01 `issue_configure` takes `held[k]` out of the book | staging.rs | P1, C3 | C3's second `step` finds no held X: no Stop note |
| E2B-02 `issue_configure` sets `playing.started` for the configured booking | staging.rs | P1, **C5** | C5's switch is accepted at the probe |
| E2B-03 `issue_configure` without its "nothing in flight" assert | staging.rs | P2 | `should_panic` |
| E2B-04 `issue` (Send) without the Configure/Reopen part of the assert (only `Send` in flight checked) | staging.rs | P2 | `should_panic` |
| E2B-05 `complete_configure(true)` removes `held[k]` | staging.rs | P4, `ur_25_a_value_after_a_burst_ends_is_applied_before_the_next_burst` | B never sent |
| E2B-06 `complete_configure(false)` leaves the booking held | staging.rs | P4, `ur_21_a_refused_configure_refuses_the_burst_and_the_fixed_value_plays_on_the_same_clock` | the next turn configures X again: a second `COMMAND_REJECTED` [1] |
| E2B-07 `complete_configure` without the kind check (a Send's or Reopen's op of the same id accepted as a Configure) | staging.rs | P3 | `should_panic` |
| E2B-08 `complete_reopen(Interrupted)` handled as `Failed` | staging.rs | P6, `ur_22_stop_and_loss_during_owner_recovery_release_the_claim` | "recovery interruption does not refuse the pending burst" |
| E2B-09 `complete_reopen(Failed)` keeps `open` | staging.rs | P5, `ur_22_underflow_close_failure_reopens_before_the_next_payload` | "B was cancelled from the send barrier" |
| E2B-10 executor: the configure's `issue_configure` before the claim check (the op left in flight when the claim is refused) | tx.rs | C1 | the second `step` panics on "in flight", or never configures |
| E2B-11 executor: the reopen's loss read hoisted out of the result section (`let lost = core.is_lost();` immediately before `lock(&core.streams)`, `:954`, and the `:957` read replaced by it) | tx.rs | **C9(a)** (M3; the first draft's killer, `ur_22_a_loss_during_the_reopen_clears_the_mark_the_reopen_read`, publishes the loss at `tx_reopen_done`, before the hoisted read, so it cannot kill this) | the hook fires after the hoisted read and before the lock: the mutant writes `Some(1)` on a lost device |
| E2B-12 executor: the precondition check removed from the plan section | tx.rs | R1 | `should_panic` |
| E2B-13 executor: `Err{lost}` from `configure_tx` mapped to "keep X held" (today's drop kept for non-lost only) | tx.rs | C4 (ii) | a ProviderStop note for X appears |
| E2B-14 executor: the configure's loss read hoisted out of the mark section (`let lost = core.is_lost();` immediately before `lock(&core.streams)`, `:621`; the guard `Ok(()) if !lost`) | tx.rs | **C9(b)** (M3; `ur_22_a_loss_during_the_configure_clears_the_mark` publishes at the probe, before the hoisted read) | the mutant writes `Some(g)` after the hook's loss |
| E2B-15 `complete*`'s token check removed (any in-flight op of the right kind accepted) | staging.rs | **P7(i), (ii)** | the stale replay is accepted and consumes Y's op: no panic, Y removed or the switch cancelled |
| E2B-16 executor: the refused configure's `complete_configure` moved into the mark section (before `give_back`'s section closes) | tx.rs | **C8** (M2) | Z accepted in the gap |
| E2B-17 `issue*` returns the same id every time (the counter not advanced) | staging.rs | **P7(i), (iv)** | the stale id equals Y's: the replay does not panic |
| E2B-18 (N1) `end_open` without its "nothing in flight" assert | staging.rs | **P8** | `should_panic` |
| E2B-19 (N3) executor: the failed reopen's loss read hoisted out of the result section (`:958`, `:962` read a `lost` taken before `lock(&core.streams)` at `:954`) | tx.rs | **C9(c)** | the hook's loss lands after the hoisted read: the mutant halts the clock and refuses B with ClockEnded |
| E2B-20 (N6) executor: `issue_reopen` before the stop/loss/claim early returns (`:930-940`) | tx.rs | `ur_22_uncertain_reopen_waits_for_owner_claim_and_stops_on_stop_or_loss` | the first `pass` returns at the refused claim with the op in flight; the retry's `issue_reopen` panics |

Each new row names its killer in `filter`. The implementer confirms each kill through `remote_mutate.py` on sim02/sim03, never locally (AGENTS §7, §9).

### 4.3 Removed leftovers

- `TxBook::clock_ended` (`staging.rs:559-562`) is folded into `complete_reopen`. Its only callers are `tx.rs:996` and the E2a pure test (`staging.rs` tests `:1685`).
- `Open::inflight` (`staging.rs:332`), with `Open::new`'s initializer.
- `halt_failed_reopen`'s booking parts (`tx.rs:992,996`). The function keeps the executor's parts (tracker, `finished`, `clock`, `cut`) or folds into `reopen_streamer`, whichever is smaller.
- The `self.hold(held)` put-backs in `configure_turn` (`tx.rs:614,630`) go, because the booking never left.

No mutation row is deleted: every row's behaviour stays reachable.

## 5. Hardware tests: the AGENTS §9 audit

E2b changes no rule that a hardware test computes its expectation from (no front, bound, region or rate grid). The table lists the hardware tests that pass through a transmit configure or reopen turn of the Provider.

| Test | Expectation that touches these turns | Verdict |
|---|---|---|
| `hw_b6_txrx_and_repeat` (rehearsal `rehearsal_b6_txrx_and_repeat`, fake.rs) | the repeat plays with the connect values: generation 0, no configure turn | still right; FakeDevice twin `ur_25_a_first_burst_with_the_connect_values_makes_no_apply` (fake.rs:1874) |
| `hw_b7_session_loopback` (`rehearse_session_loopback`, `common/mod.rs:887`) | the retune is refused by the Kernel's envelope (`radio.rf_envelope`, `:903-904`) before any configure turn | still right |
| `hw_n47s5_*` (front, long close, held tail and Stop, repeat switch, untimed) and their `n47s5_*_rehearsal_on_the_fake` (fake.rs:7831-7965) | no transmit value is changed during the Run (`grep -n 'radio.tx' crates/ezsdr-radio-uhd/tests/common/n47s5.rs` finds no frequency or gain update), so no configure turn. A reopen only after a failed close, which none provokes | still right |
| `hw_b9_unplug`, `hw_b9_unplug_transmit_only`, `hw_b9_transmit_only_session` | a loss mid-transmit: `reopen_streamer`'s lost branch runs only if a recovery was pending at the loss | still right (cannot tell whether a recovery is ever pending in that bench; nothing in the expectation depends on it) |
| `hw_n47_raw_tx_setting_after_a_burst_end`, `hw_n47_raw_tx_rate_on_an_open_streamer`, `hw_d9_p5_a_stale_timed_header_after_a_reopen`, `hw_h2_underflow_mid_burst` | raw device calls, not the Provider's turns | not affected |

No rehearsal is added: no rule boundary moves.

## 6. `design/` text

Rules that describe these turns, with the search `grep -n "configure turn\|reopen\|recovery\|F8" design/07-radio-model.md design/09-mock-radio.md design/18-uhd-radio.md`:
- **UR-26** (`design/18:305-306`): "Its owner makes a configure turn of its own before the burst … when all of these hold … The burst's late policy decides after that turn", and the mark's rules. Still true.
- **UR-22** (`:253-257`): "Once recovery is pending, no payload is sent on that streamer until it is replaced and T3 completes". Still true.
- **UR-36** (`:291-294`): the mark as the device record. Unchanged.
- **UR-38** (`:300-301`): "an F8 recovery that ends the repeat cancel it"; "uhd-tx has issued none of that repeat". Still true. C5 pins that a configure is not an issue.
- **RM-16**: `ClockEnded` after a failed reopen (Changes row 2026-11-06, `design/07:489`). Unchanged.
- **MR-33** (the Mock's values at the burst start). Unchanged.
- KNOWN_LIMITATIONS: KL-38 and KL-41 are unchanged. KL-41's "the stream's configure held its first send back" gains C5 as evidence. That is optional, and it is a test name added to an evidence line, not a behaviour change.

**No rule text changes.** No `*Test:*` line names the replaced pure test (`grep -rn txbook_clock_ended design/ KNOWN_LIMITATIONS.md` finds nothing). The step adds a Changes row to note 49 (orchestrator).

**`plan/` amendment, for the orchestrator (Sol; adjudication).** Note 49 §5.4.4 item 4 (the `Configure` bullet) and §5.4.6 say "`ConfigureRefused` はその予約を `COMMAND_REJECTED` で拒み". This should distinguish the two refusals, as UR-26 already does (`design/18-uhd-radio.md:306`):
- a **non-lost** refusal rejects X with `COMMAND_REJECTED` (`tx.rs:632-633`, `core.rs:1068`);
- a refusal **because the device was lost** emits no `COMMAND_REJECTED` and no note for X; the loss's `DEVICE_LOST` is the report (`core.rs:1065-1066`, `:1047-1049`; C4(ii)).

`design/` changes nothing.

## 7. Open questions for the orchestrator

- **Q1. The precondition check's home.** I propose a test-build check in `send`'s plan section, which reads the device record's mark under the same lock as `issue`, plus R1. The alternative, moving the transmit mark into `TxBook`, duplicates UR-36's record, which arm, an open and a loss also write. Recommendation: the check; no move.
- **Q2. A configure that fails because the device was lost (Q6b)** leaves the booking with no report at all. The same loss a moment later (Q6a, Q6c) gives it a `UR-26: cancelled by Provider::stop` note at shutdown. Today's code makes the difference (`tx.rs:631-634`, `core.rs:1047-1049`).
  - **Status: pending the owner.** E2b keeps today's behaviour, and C4(ii) pins it. The issue is filed only if the owner agrees. Mechanism label left to the orchestrator.
  - Withdrawn after Sol's review: the claim that the fix is one line. Keeping X held on a lost `Err` also keeps X's region in the record (its footprint for later receipts) and adds X to the shutdown report. So the fix is a behaviour change with its own receipt and report consequences, to be designed with the issue.
- **Q3. A failed reopen cancels the open booking whatever `first`** (`tx.rs:992`). The halt rule keeps an open booking that issued something (#85, `staging.rs:466-473`).
  - The `!first` case is not reachable in production (INFERRED, not shown). Every path that marks recovery ends the open booking first (`abandon` `:1036`; the Stop's `end_open`), and the booking taken during a pending recovery cannot send (`:643-645`). Only `ur_22_f8_…` builds it, by calling `mark_reopen_needed` directly (`:7139`).
  - **Status: pending the owner; E2b leaves it unchanged.** P5 pins today's rule as a synthetic characterization of the transition, not as a user-reachable failure. No test or record claims that a user can reach it.

## 8. Where note 49 or the E2a records and the code disagree

1. **Line references drifted** (the brief and e2a-sketch §1.1/§1.4, at `d9fddf3`). At `9c84502`:
   - the configure turn is `tx.rs:610-637`, and the take before it is `:536`;
   - the reopen is `:927-987`, and `halt_failed_reopen` is `:991-1002`;
   - `record_configured` is at `:623,:625,:957`;
   - `drop_piece` is at `:632`;
   - W7 is now `book.clock_ended()` at `:996` (E2a turned `busy.cancel_switch()` into a `TxBook` transition, `staging.rs:560`).

   The substance holds.
2. **e2a-sketch (d1) and §1.3 item 2:** "Configure and Reopen become ops in E2b, where a Stop can arrive between the claim and the result". Narrowed after Sol's review. Two things are true at once.
   - There is no demonstrated race on who owns the containers: control does not write `held` or `open` during these device calls. `held` and `open` have no other writer (`staging.rs:273`; uhd-control reads only `playing` and the record, `control.rs:521-604`).
   - A Stop, `Provider::stop` or a loss **can** arrive between the claim and the result. The record, `unread`, `playing` and the transmit mark change then.
   - Receipt-visible interleavings of the record and the mark with the executor's sections are real:
     - a receipt between a refused configure's mark section and its `drop_piece` (Q11, C8);
     - a loss just before a result section's lock (Q12, C9);
     - the record's halt before a refused configure's `drop_piece`, safe by position (`staging.rs:946-950`; C2, P4(b)).
   - E2b keeps each of these as today and pins it. Its added value is the op identity (M1) and one op in flight per stream, ready for C1.
3. **Note 49 §5.4.4 m1 (Reopen):** "`ReopenFailed` なら … 待っていた予約は `ClockEnded` で取り消される (`COMMAND_REJECTED`)". True for the open and held bookings ("UR-22: cancelled because the TX streamer could not be reopened"). A booking still **unread** at the failure is cancelled later with a different reason, "UR-25: its transmit clock ended before the burst could start" (`tx.rs:401-404`), also `ClockEnded`. The note does not distinguish the two.
4. **Note 49 §5.4.6:** "`Reopened{ack}` の後の front は ACK の tick". The front moves only when the flush was acknowledged (`tx.rs:946-950`). An unacknowledged T3 keeps the old front (`ur_22_owner_keeps_clean_recovery_after_an_unacknowledged_t3`). The note is imprecise, not wrong.
5. **Note 49 §5.4.6 and §5.4.4 m1 (Configure):** "`ConfigureRefused` はその予約を `COMMAND_REJECTED` で拒み". This does not hold when the refusal is a loss (`Err{lost}`). Then no `COMMAND_REJECTED` and no note are emitted (Q2). The amendment wording is in §6.
6. **Note 49 §5.4.6** says `next` returns `Configure` "before the new generation's first Send". In the code, values are decided before the late policy, and a booking whose configure succeeded is decided again from `held` at the next turn: the Stop drained first (`tx.rs:471-485`), then the late policy (`:550`). That matches the note's revised order. No disagreement; recorded because E2b must keep the booking in `held` (not `open`) to keep it.
7. **E2a's `Send` has no token** (found while closing M1). The adjudication says "issued with a token (as `Send` in E2a)", and e2a-sketch's owner decision (d1) says "tokens on Sends only in E2a" (`plan/phase8/reviews/e2a-sketch.md:20,349`; also "Ops are Sends only in E2a", `:135`). But at `9c84502` `TxBook::issue(TxSend)` returns nothing, and `complete(Outcome)` names no op (`staging.rs:422-435`). The only identity is "the one in flight" (`Open::inflight`, `:332`). Note 49 §5.4.4 item 1 ("op の id は装置を呼ぶ前に登録する") was therefore not implemented as an id in E2a. This plan adds the id for all three kinds (§0.2). The orchestrator should confirm that adding it to Send in E2b is wanted. The alternative, Configure and Reopen only, is rejected in §0.2.

## 9. Counts

- Tests added: 26 (after N1–N3).
  - 10 characterization (C0–C9; C9 with three cases);
  - 12 pure (P1, P2a–c, P3a–c, P4–P8);
  - 4 rig (R1, R1b, R2a, R2b);
  - 1 seam self-check (`on_next_lock_…`).
- Pure test replaced: 1 (`txbook_clock_ended_…`).
- Pure test changed (N1): 1 (`txbook_take_untake_and_end_open_…`: S1 completed before `end_open`).
- Tests whose `expected` text may change: 2.
- Tests respelled for the token only: the `issue`/`complete` callers in `crates/ezsdr-radio/tests/staging.rs` (14 lines). Existing tests are otherwise unedited.
- Mutation rows:
  - re-anchored: ≤ 21 (the Mock's token spelling added);
  - retargeted: 4 (N47S4-13b, N47S5-89, N47S4-15b, N47S5-44; P1206 is now unchanged);
  - new: 20 (E2B-01 … E2B-20);
  - deleted: 0.
- Estimates (no script run on the post-change tree): production about 150–220 lines changed (tokens add the Send and Mock spelling), tests about 600 lines, test support about 30 lines.

## 10. Not checked

- Nothing was built or run, and no C-test was run on the base, so "green on base" is INFERRED until the red-first run. That includes C8's refusal in the gap and C9's cleared mark on today's code.
- Whether the snapshot's `timing` and `device_async` rows are stable across two base runs (INFERRED stable; the run decides any exclusion).
- Whether a `pass` with a reopen needs device time, and so `released` (C7, C9(a), R2b use it either way). Whether the configure C-tests need it (INFERRED no).
- Whether `booking_control_on`'s defaults accept a transmit value without `radio.tx.c0.stream = 0` (the rig copies `ur_21_a_refused_burst_stays_counted…`'s config to be safe).
- Whether the T3 flush in C7 needs the deadline participant on the manual clock (I follow `ur_22_a_reopen_keeps_the_mark_and_a_loss_clears_it`'s release of the frozen participant).
- `Control` is `Send` (VERIFIED: it is moved into the uhd-control thread, `mod.rs:619,666`), so C8 shares it into the hook through `Arc<Mutex<Control>>`. C5 still calls `TxBook::receive_switch` directly at the probe; it could use the same sharing, and the implementer may switch it.
- The Python suites: E2b does not touch Python.
