# C2 test plan (note 49, stage C2: a switch's outcome follows whether the sample at its boundary was sent)

Implementer's test plan, written before any code (AGENTS §9 step 2). Base: `work/c2` = `main` `fbde789` (after E3b). Nothing in the repository was edited and nothing was committed. Every claim is read from the code at `fbde789` and cited as `path:line`; what I did not read or run is marked INFERRED; hardware behaviour is marked VERIFIED or INFERRED.

**Probes.** A prototype of the rule ran in a copy outside the repository (`/tmp/opencode/c2-probe`, `rsync -a --no-times --exclude .cargo`, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`, local, not on sim02/sim03). It is a sketch (about 90 changed lines in `staging.rs`, a 6-line reorder in the Mock's `stop_tx`), not the implementation. What it ran: `-p ezsdr-radio` (all), `-p ezsdr-mock-radio` (all), `-p ezsdr-radio-uhd --lib` (581), `-p ezsdr-radio-uhd --test fake` (416) and `-p ezsdr-acceptance` (all, parity included). Result (§3.4): **6 existing tests change, nothing else**. Two probe tests (`c2_probe_stop_in_flight`, `c2_probe_mock_stop_edges`) answered the two sequences the sources did not settle (§2).

Units: samples are indices on the stream's SampleClock; the UHD frozen rigs run 1 MS/s with 200 root ticks a sample, window `W` = 10 000 samples, chunk 2 500, device lead 2 000 samples. `b` is a switch's boundary, `B` a Stop's bound (`stop_sample`, `staging.rs:1403`), `t_k` the Stop's sample.

## Changes after Sol's review and the decisions (2026-11-12; sol-plan.md, plan-decisions.md; the sections below are the original plan, and where they differ this section wins)

Sol's plan review (FAIL: one major, three minors) is adopted in full, and the owner's answers to Q1, Q2, Q4 and Q5 and the orchestrator's to Q3, Q6 and Q7 are folded in. Read §0 to §8 with these amendments.

### A. Decisions applied

- **Q1 (owner): a loss settles by device time at the loss**: `applied` iff the boundary send was taken and the sample at `b` was played (`b < t_k`, the loss sample; the same edge as the Mock's `b < cut` with `cut = t_k`), `uncertain` if the boundary send failed, else `cancelled`; not as a Stop. The loss-sample half (UHD `Streams::end` passes `t_k` as `played`; §5 C2-3) **moves into C2-2** (see D): M1 below needs the deferred settlement to hold the loss sample, so the two cannot be split. The Mock's half needs nothing new (`cut = t_k` already).
- **Q2 (owner): `uncertain` shows in the Manifest only**, readable from Python through `sdr.manifest`; nothing new in the Python client; `iii::switch_verdict` gives `Inconclusive` for `uncertain` (§6 stands). `Session.switch()`'s docstring changes: see C.
- **Q3 (orchestrator):** a Stop while the boundary send is in flight settles by that send's result; the row stays `pending` until the completion (`Switch.halted`; extended in M1).
- **Q4 (owner):** the new refusal text `UR-21: … no pending switch` for a second switch on an unissued one past its boundary is accepted; one clause in KL-41.
- **Q5 (owner):** the Mock/UHD difference near the window edge is accepted; MR-42 states it as MR-25 does for the cut.
- **Q6 (orchestrator):** KL-42 becomes "Gone in C2" with its symptom corrected (the Mock had the same defect for `b` between its payload cut and `B`); the entry keeps its number.
- **Q7 (orchestrator):** `Bookings::switch(.., carried)` is explicit.

### B. Sol's findings

**M1 (major): a loss during the unresolved boundary send.** Sequence: issue the send carrying `b` → a loss at `t < b` → the send completes successfully → the row must read `cancelled` (Q1), not `applied`. The §0 sketch's `halted` keeps only the Stop's bound and `complete` then settles with `played = i64::MAX`, which gives `applied` for a device that was lost before `b`. The loss path is independent of the send's completion (`core.rs:1047-1056`, `tx.rs:579-642`), so it is reachable.

*Design.* `Switch.halted` is `Option<Halted { stop, played }>`, not a bound alone. The first halt sets it; a later halt in flight keeps the first `stop` (S18: the apply geometry) and takes `played = min(played)` (an earlier loss is never un-lost, and a Stop's `played` is `i64::MAX`). `TxBook::complete` settles the deferred halt with the same function `Bookings::halt` uses for a switch whose `carried` is known, fed the stored `stop` and `played`: whole or `Accepted(n ≥ 1)` → `applied` iff `b < stop` and `b < played`, else `cancelled`; `Accepted(0)` or `TailShort` → `cancelled`; `Failed` → `uncertain` unless `b ≥ stop` (a Stop's bound at or before `b` cancels whatever the send did, the oracle's order). For a loss `played` is the loss sample `t_k`, for a Stop `i64::MAX`. So one `Settled` path serves every halt, and C2-2's deferred settlement and C2-3's loss sample are the same mechanism: **`halt(t, stop, played)` is called by a Stop with `played = i64::MAX` and by a loss with `played = t_k`, in C2-2.**

*New sequences* (§2; oracle `switch_outcome_after_loss`, `…_after_stop_and_loss`):

| # | Sequence | Today UHD | C2 | Evidence |
|---|---|---|---|---|
| **S19** | boundary send issued; **loss at `t < b`** while in flight; the send then completes whole / short ≥ 1 / short 0 / exception | applied (a loss settles by `B`) | row `pending` between loss and completion; then **cancelled** / cancelled / cancelled / **uncertain** | new (M1); Q1 |
| S19b | the same with the loss at `t > b` (device time passed `b` while the send was unresolved: a late send the device may have dropped, H2) and the send whole | applied | **applied** (the host cannot tell, S12); exception → uncertain; zero → cancelled | new |
| **S20** | **loss after the completion, before device time reaches `b`** (boundary send `Sent`; the book has not run `advance`) | applied | `t ≤ b` → **cancelled**; `t > b` → applied (what `advance` would have done); a failed send → uncertain | new |
| S21 | **a Stop and a loss both during one boundary send**, either order, then the result | applied | by the result: whole / short ≥ 1 → `applied` iff `b < B` (first Stop's bound) and `b < t_k`; so cancelled when the loss is before `b`; exception → uncertain unless `B ≤ b`; the apply geometry (when applied) uses the first halt's `stop`; the row stays `pending` until completion | new; extends S18 |

*Tests* (new rows of §3.2):

- Record (`crates/ezsdr-radio/tests/staging.rs`, pure): `c2_record_a_loss_during_the_boundary_send_is_kept_until_the_result` (S19, S19b: `halt(.., InFlight, played = t_k)` leaves `take_settled()` empty and `switch()` refused `PendingSwitch`; then `carried(Sent)` → `Cancelled` for `t_k ≤ b`, `Applied` (chains equal the applied ones) for `t_k > b`; `carried(Unknown)` → `Uncertain` for both; `carried(Unissued)` (zero) → `Cancelled`); `c2_record_a_loss_after_the_boundary_send_settles_at_once` (S20: `Sent` and `halt(.., played = t_k)`: `t_k ≤ b` `Cancelled`, `t_k > b` `Applied`; `Unknown` `Uncertain`); `c2_record_a_stop_and_a_loss_in_flight_keep_the_first_bound_and_the_earliest_loss` (S21: Stop then loss and loss then Stop, with `b < B`: after `Sent`, `Cancelled` when the loss is before `b`, `Applied` when after; a second Stop with a later bound leaves the apply geometry of the first). `c2_record_a_second_halt_keeps_the_first_bound` (S18) stays and is extended to `played`.
- Book: `txbook_complete_applies_the_deferred_halt_by_its_loss_sample` (a `TxBook::stop_played`/loss in flight, then `complete(Whole)`: `Cancelled`; `complete(Failed)`: `Uncertain`; `complete(Accepted(0))`: `Cancelled`).
- UHD frozen rig (`tx/tests/c2.rs`, the hook at `tx_send_planned` as U3): **U13** `c2_s19_a_loss_while_the_boundary_send_is_in_flight_is_decided_by_the_result_and_the_loss_sample`: the loss through `Streams::end(.., lost = true)` at a sample before `b`, then no fault / `ShortSend` / `StalledSend` / `FailSend`; asserts the row `pending` between the hook and the completion (the intermediate row, read with `switch_row(0)`), then in the Manifest `cancelled`, `cancelled`, `cancelled`, `uncertain`; no `NEW` sample after the loss on the device; a note only where the plan says. **U14** `c2_s20_a_loss_after_the_boundary_send_before_device_time_reaches_b`: boundary send whole, `end_streams(.., lost = true)` at `t_k ≤ b` → `cancelled`; at `t_k > b` → `applied`; the failed send → `uncertain`. **U15** `c2_s21_a_stop_and_a_loss_in_flight_either_order`: Stop at the hook, then the loss (and the reverse), then the result. S19b is reached only if the rig can move device time past `b` at the hook (`device_at` as in U7); otherwise it is covered at the record and book levels, and the plan says so in the sub-step's report.
- **Mock**: the Mock has no unresolved send (its boundary emission is atomic at the step: no send faults, M5), so S19, S19b and S21 are not reachable there; its analogue of S20 is **M2**, extended with a case where the loss and `b` are in the same step, and a case where a Stop and a loss follow each other at different samples (the loss settles the record; a later Stop leaves a settled row alone). Stated in MR-42.
- Parity: no new row (the Providers differ at the window edge, Q5).

*Mutation rows* (new, all killers fail on an assertion):

| Row | Mutates | Killer |
|---|---|---|
| **C2-16** | `TxBook::complete`: the deferred halt settles with `played = i64::MAX` instead of the stored one | `c2_record_a_loss_during_…`, `txbook_complete_applies_the_deferred_halt_by_its_loss_sample`, U13 |
| C2-17 | the second halt in flight overwrites `played` instead of taking the minimum (or keeps the later) | `c2_record_a_stop_and_a_loss_in_flight_…` |
| C2-18 | the second halt in flight overwrites `stop` (resets to the later bound) | `c2_record_a_second_halt_keeps_the_first_bound` |
| C2-19 | the loss path (`Streams::end(lost)`) passes `i64::MAX` as `played` (C2-15, moved with the loss sample to C2-2) | U13, U14, M2 |

C2-15 is C2-19 and is not counted twice; the table of §3.3 keeps "C2-15" for it.

**m2 (minor): two accepted switches settle each on its own boundary send.** Test (UHD frozen rig, next to `e2a`'s positive twin at `e2a.rs:464-472`): `c2_s17b_two_accepted_switches_settle_each_on_its_own_boundary_send` — S₁'s boundary send is taken and device time passes `b₁` → S₁ `applied`; accept S₂; S₁'s next send (its repeat's continuation) completes while S₂ is pending; S₂'s boundary send fails (`FailSend`) → rows `[applied, uncertain]`, each with its own action id and S₂'s predecessor = S₁; S₂'s `carried` was not set by S₁'s completions. A record-level twin (`c2_record_two_switches_keep_their_own_carry`): `Bookings` `switch` S₁, `carried(Sent)`, `advance` → S₁ `Applied`, `switch` S₂, `carried(Unknown)`, halt → S₂ `Uncertain`. New row **C2-20**: `TxBook::issue`/`complete` set `carried` on the pending switch without the `Playing.id == switch.id` test (a continuation of the first switch's booking marks the second); killers: the two tests above.

**m3 (minor): H4 corrected.** H4 of §2 read "an underflow's tick is the end of what was sent (VERIFIED for the ACK, `bench-results.md:311,331`; …)". The cited bench evidence establishes BurstAck timing only. **The claim that an underflow's tick identifies the transmitted end is INFERRED**, not VERIFIED. C2's rule does not use an underflow's tick for `b`: S10 rests on RM-37's "before handing the sample at `b`" and the abandon cursor `c`, which the book keeps from the sends, not from the report; U6 pins that software behaviour (an underflow after a whole boundary send does not unsend it). No hardware claim is made for S10; if the owner wants one, a bench cell would measure it (not proposed).

**m4 (minor): `Session.switch()`'s docstring** (`python/ezsdr/session.py:498-504`) is added to §4 and to the sub-step that changes RM-37 and UR-38 (C2-2, same commit, AGENTS §2). Its text gains one sentence: *"The outcome of a switch is one of `applied`, `cancelled` or `uncertain` in the Manifest's `switches` section (read it from `sdr.manifest`): `uncertain` means the send that carried the new waveform's first sample ended in an exception, so whether any of it was sent is unknown."* No code and no new accessor (Q2). It is a docstring only, so the Python suites run at the gate, not as a sub-step check.

### C. §4.2 additions

- `python/ezsdr/session.py` (`Session.switch()` docstring, C2-2): the sentence of m4.
- RM-37 and UR-38: the loss sentence follows Q1 (a loss settles by device time at the loss: `applied` when the sample at `b` was played and the boundary send was taken, `uncertain` when it failed, else `cancelled`), and the deferred-settlement sentence says "a Stop or a loss that comes while the boundary send is unresolved settles by its result".
- MR-42: as §4.2 plus the Mock's loss analogue (a loss at the sample `t_k` is `applied` iff `b < t_k`; no unresolved send exists).

### D. §5 sub-steps, amended

| # | Content | Rows |
|---|---|---|
| C2-1 | Oracle functions and `o1_oracle.rs` self-tests; no production change. **Done in this round**; names: `Outcome::{Uncertain, Pending}`, `Sent`, `switch_outcome_at_device_time`, `switch_outcome_after_{stop, loss, stop_and_loss, abandon, mock_stop}`. The old two-argument `outcome_after_stop` and `outcome_after_abandon` stay until C2-2 moves their callers to the new functions and deletes them | none |
| C2-2 | As §5, **plus** `Halted { stop, played }` (M1), `halt(t, stop, played)` called by a Stop with `i64::MAX` and by a loss with `t_k` (UHD `Streams::end(lost)`, moved from C2-3), the loss's UHD tests U10, U13 to U15 and the Mock's M2, the record and book tests of B, m2's tests, the `Session.switch()` docstring | C2-01 to C2-14, C2-16 to C2-20 (C2-15 = C2-19 here), the retargets |
| C2-3 | **Only** what remains of Q1 after C2-2: the Mock loss derivation check (no code expected: `cut = t_k` is already the Mock's) and `schema_freeze` unchanged. If C2-2 leaves nothing, C2-3 is empty and is merged into C2-4 | none |
| C2-4 | Hardware verdict (`uncertain`, Q2) and its `iii.rs` unit test | none |
| C2-5 | Delete the unreachable rewrite; unchanged | retire six rows |

The loss-sample rule being in C2-2 keeps one meaning of `played` per sub-step: no commit has a `played` argument that every caller passes as `i64::MAX`.

## Changes in C2-2 (implementer's record, 2026-11-12; where it differs from the text above, this section wins)

C2-2 as amended (§D): the record and book (`Carried`, `Halted { stop, played }`, `halt_played`, `issued`, `carried`), `SwitchOutcome::Uncertain`, the Mock's `stop_tx` reorder, UHD `stop_at(.., lost)`, the specs and KL text, `Session.switch()`'s docstring, the tests and the rows. Tree: `work/c2` over `f3b04bb` (C2-1); nothing committed.

**Production.** `staging.rs`: `Carried { Unissued, InFlight, Sent, Unknown }`; `Switch { id, boundary, carried, halted }`; `Bookings::switch(.., carried)` (Q7, 16 test sites); `Bookings::issued(id)` and `Bookings::carried(id, c)` (called by `TxBook::issue` and `TxBook::complete`); `Bookings::halt_played(t, stop, played)` with `halt(t, stop)` as its `played = i64::MAX` form; `settle_halted` is the one function that turns (boundary, carry, bound, played) into an outcome and is used by a halt, by the deferred result and by an abandon (through `halt`); `advance` applies only a `Sent` switch; `TxBook::receive_switch` starts the carry by the Provider's rule (`AtFirstIssue` → `Unissued`, `AtAcceptance` → `Sent`); `TxBook::stop_played(reach, played)`. `core.rs`: `StreamState::stop_at(at, in_flight, lost)` passes the loss sample as `played` for a loss (M1, moved here from C2-3). Mock `lib.rs`: `stop_tx` computes its payload cut before it books the Stop and passes it as `played`.

**Decided while implementing (decided by the implementer; each is the smaller change that keeps the plan's behaviour).**
1. `halt(t, stop)` stays and calls `halt_played(t, stop, i64::MAX)`: 70 record tests call it for a Stop. The loss and the Mock's Stop call `halt_played`, so no caller passes `MAX` through the three-argument form (§D's remark).
2. No `first`/`Playing.id` test in `TxBook::issue` and `complete`: `Bookings::issued` acts only on a pending switch of the same id that is `Unissued`, and `carried` only on one `InFlight`, so the first send of the switch's own booking is the only one that can change the carry. One state, one place (AGENTS §9 Maintainability 2). Rows C2-07, C2-08, C2-20 pin it (§3.3 listed C2-07/C2-08 for a `first` filter and C2-20 for the `Playing.id` test).
3. A halt whose bound is at or before `b` settles at once even while the boundary send is in flight (§A S21 said "pending until completion"): no result can change a cancellation by `B ≤ b`, and the Manifest row stays accurate. A halt with `B > b` waits for the result. Test `c2_record_a_halt_during_the_boundary_send_waits_for_the_result` pins both.
4. `c2_record_a_halt_during_…` cannot assert `switch()` refused as `PendingSwitch` after the halt: the halt caps the old repeat's front, so the record answers `NoPlayingRepeat` first. It asserts that the row is not settled, that `advance` at any device time does not settle it, and that the chains after the deferred settlement equal those of a Stop that came after the result (a whole-record equality).

**Tests.** Record (`crates/ezsdr-radio/tests/staging.rs`, 11 new, pure): `c2_record_a_halt_settles_a_pending_switch_by_what_became_of_its_boundary_send`, `…_a_halt_during_the_boundary_send_waits_for_the_result`, `…_a_second_halt_keeps_the_first_bound`, `…_advance_applies_only_a_sent_switch`, `…_an_abandon_settles_by_the_cursor_and_the_carry`, `…_the_footprint_does_not_depend_on_the_outcome`, `…_a_loss_during_the_boundary_send_is_kept_until_the_result`, `…_a_loss_after_the_boundary_send_settles_at_once`, `…_a_stop_and_a_loss_in_flight_keep_the_first_bound_and_the_earliest_loss`, `…_two_switches_keep_their_own_carry`. Book (5 new): `txbook_issue_marks_only_the_switch_s_own_first_send_in_flight`, `txbook_complete_sets_carried_by_the_outcome`, `txbook_receive_switch_starts_the_carry_by_the_provider_s_rule`, `txbook_stop_passes_the_played_cut_to_the_record`, `txbook_complete_applies_the_deferred_halt_by_its_loss_sample`. The two changed tests of §3.4 (`txbook_receive_switch_takes_the_boundary_at_receipt`, `txbook_issue_starts_the_playing_repeat_and_a_switch_s_own_payload`) changed as the table says. UHD frozen rig (`tx/tests/c2.rs`, 14 new; the hook is `tx_send_planned`, detected as "the open booking is the switch's and has not completed a send"; the run stops at the turn the hook fired in so a hook that moves device time is not set back): U1 `c2_s4_…`, U2 `c2_s3_…`, U3 `c2_s5_…` (no fault, `ShortSend`, `StalledSend`, `FailSend`; a Control Stop and `Provider::stop`; the row is `pending` between hook and completion), U5 `c2_s8_…`, U6 `c2_s10_…`, U7 `c2_s11_…`, U9 `c2_s14_…`, U10 `c2_s13_…`, U11 `c2_no_row_is_left_pending_when_the_run_ends`, U12 `c2_s15_…`, U13 `c2_s19_…`, U14 `c2_s20_…`, U15 `c2_s21_…`, m2 `c2_s17b_…`. U4 and U8 are the changed `o1_uhd_t7c_…`/`e2a_84_…`. Mock: M1 `c2_mr_25_a_stop_settles_a_switch_by_the_payload_cut` (`x3xx` and `ideal`, Stop positions around `b` and its cut; the row follows the oracle and agrees with the successor record), M2 `c2_mr_25_a_loss_settles_a_switch_by_the_loss_sample` (loss at `b − 1`, `b`, `b + 1`; `mock_radio.rs`). Parity (3 new): `c2_parity_a_switch_whose_boundary_precedes_the_stops_bound_is_cancelled_alike`, `c2_parity_a_stop_right_after_a_switch_cancels_it_alike` (the KL-42 cell on both), `c2_parity_a_switch_sent_before_the_stop_is_applied_alike`. The oracle's old two-argument `outcome_after_stop`/`outcome_after_abandon` and their self-test lines are deleted; the callers (UHD `o1.rs` T4c, T4d, T4e, T7a, T7a3, T7c, Mock `o1.rs` T4c, T4d, T4e) call the C2-1 functions, and their `C2` comments are gone.

**Changed existing tests (the six of §3.4, and only they).** `e2a_s11_…` (renamed `…_cancels_it_and_its_payload`, `cancelled`), `o1_uhd_t4d_…` (renamed `…_cancels_a_switch_whose_first_sample_was_not_handed`, row `cancelled`, successor count and note unchanged), `o1_uhd_t7c_…` (renamed `…_leaves_the_switch_uncertain`, `uncertain`), `e2a_84_…` (`UR-21: … no pending switch`), the two `txbook_*` tests. The Mock's `o1_mock_t4d/t4e` keep their rows (`applied`) and now compute them from `outcome_after_mock_stop(b, cut)`; their helper `stopped_after_switch` no longer returns `bound`. Nothing else failed.

**Deviations from §3.2.** (a) The plan's third parity test `c2_parity_the_row_follows_each_provider_s_own_emission` (a Stop with `b` near the window edge) is not written: its UHD half needs a Stop placed by device time inside a window of a few samples with the rig's threads, and the Mock half is M1. `c2_parity_a_stop_right_after_a_switch_cancels_it_alike` takes its place and covers KL-42 on both. (b) U13's S19b (loss with device time past `b` while the send is in flight) has the whole-send and `StalledSend` cells only: the fake drops a late continuation whatever fault is set, so an exception cannot be arranged there; the record test `c2_record_a_loss_during_the_boundary_send_is_kept_until_the_result` covers `Unknown` with `t_k > b`. (c) The loss tests on UHD (U10, U13, U14, U15) assert rows only: `end_streams(.., lost = true)` books the loss on the line but does not make the fake device lost, so "no `NEW` sample after the loss on the device" is not checkable. (d) A Stop booked after `Streams::end` (U15, loss first) may be refused by the line; the cell then checks the loss alone.

**Mutation rows.** New: C2-01 to C2-14, C2-16 to C2-22 (C2-15 is C2-19; C2-21 and C2-22 are new, below). Retargeted (anchor only, same program): N47S4-18c, N47S5-29 (`.stop_played((…, …),`), N47S4-43, N47S5-24 (`stop_at(…, lost/false)`), N47S5-35 (`s.boundary <= now_k`), N47S5-36 (now `_ if switch.boundary >= stop => return,` in `settle_halted`), P670, O1-M4, N95-21 (the Mock's stop cut/horizon in the tuple), E2A3-06, E2A3-10 (the `carried` argument). Not changed: E2A3-03, E2A3-04, N47S5-43, N47S5-44, N47S5-94, N47S5-132, N47S5-147, N47S5-155 (they anchor as before; run). The retirement rows of C2-5 (N47S5-136, E2A2B-01, -05, -12, N47S5-152, E2A2B-06) are untouched and were run as a check.

| Row | Mutates | Killer |
|---|---|---|
| C2-01 | `settle_halted`: no cancel at `B ≤ b` | `c2_record_a_halt_settles_…` |
| C2-02 | `played` ignored for a sent switch | same |
| C2-03 | exception → `Applied` | same |
| C2-04 | an in-flight halt settles at once | `c2_record_a_halt_during_…` |
| C2-05 | the result does not settle a waiting halt | same |
| C2-06 | `advance` applies an unsent switch | `c2_record_advance_applies_only_a_sent_switch` |
| C2-07 | `issued` ignores the booking's id | `c2_record_two_switches_keep_their_own_carry` |
| C2-08 | `TxBook::issue` never marks | `txbook_issue_marks_only_…` |
| C2-09 | `Accepted(0)` counts as taken | `txbook_complete_sets_carried_by_the_outcome` |
| C2-10 | `Failed` counts as taken | same |
| C2-11 | a UHD switch starts `Sent` | `txbook_receive_switch_starts_the_carry_by_the_provider_s_rule` |
| C2-12 | a Mock switch starts `Unissued` | `mr_16_s8_applied_switch_settles_at_device_time_without_a_stop` |
| C2-13 | the Mock passes `i64::MAX`, not its cut | `c2_mr_25_a_stop_settles_a_switch_by_the_payload_cut` |
| C2-14 | the `uncertain` label reads `applied` | `o1_uhd_t7c_…` |
| C2-16 | the deferred halt forgets `played` | `c2_record_a_loss_during_the_boundary_send_is_kept_until_the_result` |
| C2-17 | a second halt overwrites the earliest `played` | `c2_record_a_stop_and_a_loss_in_flight_…` |
| C2-18 | a second halt overwrites the first bound | `c2_record_a_second_halt_keeps_the_first_bound` |
| C2-19 | UHD's loss passes `MAX` as `played` | `c2_s13_a_loss_before_the_boundary_is_decided_by_device_time` |
| C2-20 | `carried` ignores the booking's id | `c2_record_two_switches_keep_their_own_carry` |
| C2-21 | a halt at `B ≤ b` waits for the result | `c2_record_a_halt_during_…` |
| C2-22 | the book passes no `played` to the record | `txbook_stop_passes_the_played_cut_to_the_record` |

**Text.** RM-37 (second paragraph and Test line), UR-38 (the pending-switch paragraph, the refusal clause and Test line), MR-42, one Changes row each in 07, 09 and 18; KL-42 ("Gone in note 49 stage C2", symptom corrected), KL-41 (one clause); note 49 §5.4.5 (the in-flight row); `python/ezsdr/session.py` (`Session.switch()` docstring, m4). The Test lines were split so that no spec line passes 1 500 characters (`rules.py`); `rules.py` needs `--renames ~/.cache/ezsdr-fixes/2026-11-12-c2/renames.txt` for the renamed `e2a_s11_…` (the gate must pass it). No schema, version or SCHEMA_CHANGELOG change.

**AGENTS §9 audit table (hardware tests and bench values).** C2-2 changes no hardware test and no bench value. `hw_n47s5_repeat_switch` (`applied`) and its FakeDevice rehearsal `n47s5_repeat_switch_rehearsal_on_the_fake` stay right and passed. `iii::switch_verdict` and its `uncertain` case are C2-4.

**Mutation outcome (`remote_mutate.py maintenance sim03:4`, 51 rows; then `sim03:2` for the two re-targeted).** All 21 new C2 rows, the 11 re-anchored rows and the adjacent rows are `killed`, except four that were not killed at first: N47S5-152 and E2A2B-06 (the abandon's rewrite of settled `applied` rows) SURVIVE as predicted in §0 item 6: the code is unreachable, and both go with it in C2-5 (they will need `known-survivors.txt` or retirement at the gate). N47S5-43 and N47S5-155 survived because their old killers use a switch whose boundary send was not issued (equivalent under C2); re-targeted to `c2_s3_a_stop_after_the_boundary_send_leaves_it_applied` and `rm_37_a_switch_taken_at_the_abandoned_cursor_is_cancelled` (a sent switch) and both are `killed`.

**Found, not fixed.** A switch received on the `ideal` Mock at the very sample the repeat starts (`ideal`, repeat from 2 000, switch at 2 000) panics in `TxBook::take` (`a booking is already open`, `staging.rs`); reproduced at `fbde789` in a copy outside the repository. Not part of C2; my M1 test uses a later receipt.

## Changes in C2-3 + C2-4 (implementer's record, 2026-11-12; where it differs from the text above, this section wins)

Merged as plan §D allows: C2-2 already moved the loss sample (`StreamState::stop_at(.., lost)` passes `t_k` as `played`) and wrote M2, so C2-3 reduces to two checks, and C2-4 is the hardware verdict. Tree: `work/c2` over `ccbeba7` (C2-2); nothing committed.

**C2-3 checks (no code).**
- The Mock's loss derivation: `stop_tx` (`crates/ezsdr-mock-radio/src/lib.rs:1153-1165`) takes the non-handover branch for a loss (`cancel_reason` is neither `Stop` nor `ProviderStop`), so `planned = (now, at_stop)`, the cut is `at_stop = t_k`, and `stop_played` receives it. Nothing to change. M2 `c2_mr_25_a_loss_settles_a_switch_by_the_loss_sample` (`mock_radio.rs:1969`; loss at `b − 1`, `b`, `b + 1` samples → `cancelled`, `cancelled`, `applied`, and the new waveform's sample radiated iff `applied`) passes.
- `schema_freeze` (`cargo test -p ezsdr-kernel --test schema_freeze`: `ov_22_schema_freeze`, `ov_10_…`, `x8_…`, 5 tests) passes and `git status` shows no change under `schemas/`: the value set lives only in RM-37, UR-38 and MR-42 (§0 item 1).

**C2-4 (Q2).** `iii::switch_verdict` (`crates/ezsdr-radio-uhd/tests/common/iii.rs`): a row `"uncertain"` returns `Verdict::Inconclusive` ("the send that carried the boundary ended in an exception, so whether the new waveform was sent is unknown"). Its place is after the refusal, missing-row, fault and seam checks and before the `row != "applied"` FAIL: a refusal, a missing row, a transmit fault and a seam still FAIL whatever the row says; only the outcome text changes from FAIL to INCONCLUSIVE. New unit test `an_uncertain_switch_is_inconclusive_and_only_a_seam_or_a_refusal_still_fails_it` (next to `a_switch_not_applied_fails_and_no_tone_is_inconclusive`): `uncertain` with a clean trial is `Inconclusive` (text contains "uncertain"), with `seams = Some(1)` a `Fail` ("restarted"), with a refusal a `Fail`. It fails on the old code (`uncertain` → `Fail`), and on a verdict that returns `Inconclusive` before the seam and refusal checks. No mutation row (plan §D: none for C2-4; the test is in the hardware tests' shared harness).

**AGENTS §9 audit (hardware tests and bench values).** `hw_n47s5_repeat_switch` and `n47s5_repeat_switch_rehearsal_on_the_fake` read `applied` through `switch_verdict` and are unchanged; the new `uncertain` branch is reached by neither (the 60 ms Stop follows a whole boundary send). No bench value is fixed.

The rule (note 49 §11.10, §5.4.5) as a function of facts. The *boundary send* is the send (UHD) or emission (Mock) that carries the sample `b`, the first sample of the new waveform. It is always the first send of the switch's own booking, because the old repeat is cut at `b` (`tx.rs:416`, `held_cut`).

| Outcome | When |
|---|---|
| `applied` | the device took the sample at `b` (boundary send whole, or short with at least one sample), and device time reached `b`, or a Stop or `Provider::stop` with `B > b` was received after it (the device plays what it holds up to the Stop's cut) |
| `uncertain` (new) | the boundary send ended in an exception (`Outcome::Failed`): whether the device took any of it is unknown (`tx.rs:660`) |
| `cancelled` | a Stop, `Provider::stop`, a loss or an abandon came before the sample at `b` was taken (boundary send not issued, or issued and the device took none of it), or `B ≤ b` |
| `pending` | not settled yet |

**Recommended implementation** (probed): a pending switch carries what became of its boundary send (`Carried`: `Unissued`, `InFlight`, `Sent`, `Unknown`); `TxBook::issue` and `TxBook::complete` set it (they already see "the open booking is the switch's and has not sent"); `Bookings::advance` applies only a `Sent` switch; `Bookings::halt` settles by `Carried` (`InFlight` waits for the result, with one extra field `halted`); the Mock's Stop passes its payload cut so that `Sent` means `b < cut`.

**What the code, the specs or the schema contradict or do not have** (the final message lists them again):

1. **There is no schema to change.** The brief and note 49 §11.10 name "the schema". The `switches` rows are a Provider section: `SwitchRecord.outcome` is a `&'static str` (`staging.rs:242`), the Manifest's `sections` is `additionalProperties: true` (`schemas/provider_instance.v1.json:551`), and `grep -rn switch schemas` finds only burst and `time_error_payload` descriptions. The value set exists only in RM-37, UR-38 and MR-42. No regeneration, no `schema_freeze` change, no `SCHEMA_CHANGELOG` entry (AGENTS §6).
2. **KL-42 describes the Mock wrongly.** It says MockRadio sends the new waveform from `b` up to its payload cut "which ends before `B`", as if the Mock were right. The Mock has the same defect in a window: `Bookings::halt` applies a switch when `b < B` (`staging.rs:1169`); the Mock's cut is `max(t_k, handed payload, previous cut)` (`lib.rs:1161`) and the handed payload excludes a repeat whose first send (32 samples, MR-25) does not fit the window; so for `b` in `[cut, B)` the Mock reads `applied` and radiates none of it (read from the code; after the prototype the row agrees with the emitted records at five Stop times, §2 S4). KL-42 must be corrected, not only closed.
3. **"Device time passed the boundary" cannot be applied to a Stop.** After a Stop with `b < B` device time may never reach `b` before the clock ends, yet `ur_23_stop_after_handover_before_the_boundary_takes_effect` (applied at 8 ms, `b` at 15 ms) is right: the sample was handed and the device plays it. I read the decision as "or will play it up to the Stop's cut". Only a loss breaks the equivalence (Q1).
4. **Note 49 §5.4.5's table has no row for a Stop that arrives while the boundary send is in flight** (issued, not completed). The lock is released during the device call, so it is reachable (the probe hits it at the `tx_send_planned` hook). The three values do not say; the rule says "the result decides" (Q3).
5. **A pending switch now outlives its boundary**: `advance` no longer applies an unsent switch at device time `b`. A second switch received after `b` while the first is unissued is refused as `PendingSwitch` (`UR-21: a repeat switch needs a playing repeat and no pending switch`) instead of `NotStarted` (`UR-38: the repeat the switch replaces has not started on the device`). User-visible refusal text (Q4); it broke `e2a_84_…` in the probe.
6. **The abandon's retroactive rewrite becomes dead code.** `Bookings::abandon` rewrites already settled `Applied` rows to `Cancelled` when `boundary >= cursor` (`staging.rs:1230-1234`) and drops that switch's chain (`never_started`, `:1229,1241`). With C2 a row is `applied` only after its boundary send, so `cursor > b` always holds. Remove it with its tests and rows (AGENTS §9 Maintainability 4), after a reachability probe (§5, C2-5).
7. **The `boundary < stop` guard stays.** The first probe without it failed `rm_16_advance_runs_inside_every_call` (`ezsdr-radio/tests/staging.rs:1010`): a record-level `Sent` switch with `b ≥ B` must be cancelled (RM-37: "`B` at or before `b` cancels"). Unreachable on UHD (a `Sent` switch has `b < c_I ≤ B − 3`), but the record is tested alone.
8. **Hardware is untouched by the rule** (§6): the one bench test that reads a switch outcome (`hw_n47s5_repeat_switch`) stops 60 ms after the switch and expects `applied`, which stays right. The new cells are FakeDevice-only (INFERRED on hardware, on two VERIFIED facts, §2).

## 1. Inventory

### 1.1 Where a switch is settled today

`Bookings` is the only writer of an outcome (`Settled`, `staging.rs:156`; written into the row by `write_settled`, `:1425`, which both Providers call: UHD through `Core::settle_switches`, `core.rs:1119`; the Mock through `settle_switches`, `lib.rs:1813`). Rows are drained lazily by `take_settled`, so a settle can be rewritten before the drain (item 4).

| # | Event | Code | Today | UHD | Mock | Under C2 |
|---|---|---|---|---|---|---|
| 1 | device time reaches `b` | `Bookings::advance`, `staging.rs:1256` (called by `prune` `:1131`, `switch` `:1186`, `TxBook::advance` `:921`) | `applied` | `Tx::step` before it reads, `tx.rs:374`; every receipt, `control.rs:523` | `settle_switches_to`, `lib.rs:1822` | `applied` only when `Sent`; else stays `pending` |
| 2 | Stop / `Provider::stop` / loss receipt | `Bookings::halt`, `:1153`, via `TxBook::stop` `:882` | `applied` if `b < B` (then `apply`: old repeat's front `b`, new chain `[b, B)`), else `cancelled` | `StreamState::stop_at`, `core.rs:436`: Ctl `control.rs:317`; `Streams::end` `core.rs:647` (Run end and loss) | `stop_tx`, `lib.rs:1152`, then `settle_switches` `:1154` | by `Carried` (§2); the Mock passes its payload cut |
| 3 | abandon of the repeat at cursor `c` | `Bookings::abandon`, `:1226`, via `TxBook::abandon` `:893`; callers `Tx::abandon`, `tx.rs:867` (short, failed, tail-short send `:608,652,666`; timeline expiry `:400,422`; underflow `:1136`) | pending, `b ≥ c`: `cancelled` (`:1237`); pending, `b < c`: `halt(c, c+5)` → `applied` | yes | never (no send faults, M5) | `b ≥ c`: `cancelled` (unchanged); `b < c`: `Sent` → `applied`, `Unknown` → `uncertain` |
| 4 | rewrite of a settled `applied` row | `staging.rs:1230-1234`, `never_started` `:1229,1241` | `applied → cancelled` for `b ≥ c` | yes (T4d/F3 state) | no | unreachable, deleted |
| 5 | repeat ended by a failed reopen (F8) | `Bookings::cancel_switch`, `:1201`, from `TxBook::complete_reopen(Failed)` `:817` | `cancelled` | yes | no | unchanged |
| 6 | payload leaves the lists with the switch | `TxBook::cancel_switch`, `:909`, only through `abandon` | (payload, not outcome) | yes | no | unchanged |
| 7 | a Stop reads the list: held / unread / unissued-open payloads cancelled with a note | `TxBook::read` → `cancel_all`, `:825,837`; `Tx::stop_now`, `tx.rs:1020` | (payload; note `UR-26: cancelled by Stop`) | yes | `read_tx`, `lib.rs:1129` | unchanged; the row now agrees with the note |

Readers of the outcome: the Manifest `switches` section on both Providers (`mod.rs:421`, `lib.rs:400`) and nothing else. No Python reader (`grep -rn "switches\|applied" python/` finds only `coercion.applied`); no server reader; no schema (§0 item 1). Tests and tools that read it: §3.4 and §6.

### 1.2 What the book already knows

`TxBook::issue` (`staging.rs:735`) sets `playing.started` on the first issue of the playing repeat's booking; for a switch that is the issue of its boundary send (`Playing.id` is the switch's id from receipt, `:855,876`). `TxBook::complete` (`:747`) sees the outcome and `open.first`. `ops.inflight` says whether an operation is in flight. So every fact C2 needs is in the book, in sections that already exist: no new lock section and no new emit.

## 2. Sequences and expected outcomes

Hardware facts used. **(H1)** Samples handed to the device are not recalled by a Stop: the transmitter keeps sending 9.3 to 10.5 ms after the Stop's submit (VERIFIED, `plan/phase8/bench-results.md:229-245`). **(H2)** A continuation whose time has passed is dropped by the device (VERIFIED, P6, `bench-results.md:223`). **(H3)** Whether UHD sent packets before an exception is unknown (INFERRED; the code's comment, `tx.rs:660`; FakeDevice's `FailSend` takes half and fails). **(H4)** An underflow's tick is the end of what was sent (**INFERRED**: the BurstAck timing is VERIFIED, `bench-results.md:311,331`, an underflow's tick is not measured there; the rule does not use it for `b`, see m3).

| # | Sequence | Today UHD | Today Mock | C2 | Evidence |
|---|---|---|---|---|---|
| S1 | boundary send whole, device time passes `b` | applied | applied | applied (UHD at the first `advance` once `Sent` and `now ≥ b`) | `ur_21_s8_…`, `o1_uhd_t4a_…`, `mr_16_s8_…` |
| S2 | Stop with `B ≤ b` | cancelled | cancelled | cancelled, whatever the send state | `ur_23_stop_before_the_boundary_…`, `mr_25_s11_stop_cancels_pending_switch`, `o1_uhd_t4c_…` |
| S3 | Stop with `b < B`, boundary send already whole | applied | applied (`b < cut`) | applied (H1) | `ur_23_stop_after_handover_…`, `o1_uhd_t4e_…`, `mr_25_s11_stop_before_the_boundary_applies_…` |
| **S4** | **Stop with `b < B`, boundary send not issued** (KL-42: host stall, a Stop right after the switch, or the first send waits for window space) | **applied**, note, no sample | applied when `b < cut`; **applied with no sample when `b ∈ [cut, B)`** | **cancelled**; Mock: applied iff `b < cut` | `e2a_s11_…`, `o1_uhd_t4d_…`; probe `c2_probe_mock_stop_edges`: Stop at 2 008 000 000 ns (`B = b + 3`, `b = we`) → `cancelled`, one record; 2 008 002 000 ns → `applied`, two records |
| **S5** | **Stop while the boundary send is in flight** | applied | (no in-flight on the Mock) | the result decides: whole or short ≥ 1 → applied, short 0 → cancelled, exception → uncertain; the row stays `pending` until then | probe `c2_probe_stop_in_flight` (Stop booked at the `tx_send_planned` of the boundary send; `fired=true` each time): no fault → `applied`; `FailSend` → `uncertain`; `StalledSend` → `cancelled` |
| S6 | send short before `b` (`c < b`) | cancelled | n/a | cancelled | `ur_21_s4_a_switch_past_the_abandoned_cursor_…`, `o1_uhd_t7a_…` |
| S7 | the old repeat's last send ends at `b` and fails or is short (`c = b`) | cancelled | n/a | cancelled (nothing of the new waveform issued) | `o1_uhd_t7a3_…`, `ur_21_s4_a_switch_at_the_abandoned_cursor_…` |
| S8 | boundary send short, `n ≥ 1` taken (`ShortSend`) | applied | n/a | applied; `n = 0` (`StalledSend`) cancelled | new |
| **S9** | **boundary send fails** (`FailSend`) | **applied** | n/a | **uncertain** | `o1_uhd_t7c_…` (its own comment says so) |
| S10 | underflow or sequence error after a whole boundary send (tick ≥ `b`) | applied | n/a | applied (H4) | new |
| S11 | host late: device time ≥ `b`, boundary send unissued, then the owner's turn | applied at the first `advance`; the new waveform's send is late | n/a | pending until the turn; the turn's timeline-expiry check abandons at `c = b` (`tx.rs:398,420`) → cancelled | new, U7 |
| S12 | host late inside the window: boundary send issued, device drops it (H2) | applied | n/a | applied (the host cannot tell; a stated limit, §8) | unchanged |
| **S13** | **loss** with `b ∈ [t_k, B)`, device time `< b` | applied (same `halt`) | applied (`b < B`); the Mock's cut for a loss is `t_k` (`lib.rs:1164`) | Q1: recommended **cancelled**; applied only if device time had passed `b` | no test today (`ur_23_provider_stop_and_loss_cancel_…`, `mr_25_s11_loss_…` have `b ≥ B`) |
| S14 | `Provider::stop` (`Streams::end`; Mock `stop(mode)`, `handover`, `lib.rs:1156`) | as Stop | as Stop | as S2 to S5 | `s15_parity_provider_stop_cancels_a_pending_switch` |
| S15 | failed reopen ends the repeat (F8) | cancelled | n/a | cancelled even if `Sent` (the clock ended; if device time had passed `b`, `advance` already applied it) | `ur_22_f8_recovery_…` |
| S16 | switch moved to the next boundary (SendAsap late) | not reachable since E2a | | n/a | note 49 §5.4.5 |
| S17 | two switches: S₁ unissued past `b₁` when S₂ arrives | S₁ applied by `advance`; S₂ `NotStarted` | n/a | S₁ pending; S₂ `PendingSwitch` (Q4) | `e2a_84_…` (4th failing test of the probe) |
| S18 | two Stops while the boundary send is in flight | applied | | the first bound is kept for the apply geometry; outcome by the result | new |

The oracle (`ezsdr-acceptance/tests/oracle/mod.rs:308-319`) says `outcome_after_stop(b, bound)` and `outcome_after_abandon(b, cursor)`, and its own text names C2. It is rewritten from this table (§3.1), not from the code.

## 3. Tests

### 3.1 The oracle (sub-step C2-1, no production change)

Spec-text functions, naming no Provider type:

- `Outcome::Uncertain` and its label.
- `outcome_after_stop(b, bound, sent)` with `enum Sent { Whole, Short(n), Failed, NotIssued }`: `Cancelled` if `bound <= b`; else applied for `Whole` and `Short(n ≥ 1)`, `Uncertain` for `Failed`, `Cancelled` otherwise. `outcome_after_abandon(b, cursor, sent)` likewise. A Mock form `outcome_after_mock_stop(b, cut)` = applied iff `b < cut` (`cut` from the existing `mock_cut`).
- `o1_oracle.rs` self-tests next to lines 166-171: one assert per cell of §2 S1 to S10 and S13, written from the text, so an oracle typo is caught without a Provider.

The tree stays green: existing callers keep the old two-argument forms until C2-2 swaps them.

### 3.2 Per sequence: test, assertion, observation, the wrong behaviour that fails it

"Changed" marks an existing test whose expectation changes (§3.4).

**Record level** (`crates/ezsdr-radio/tests/staging.rs`; pure):

| Test | Asserts / how | Fails on |
|---|---|---|
| `c2_record_a_halt_settles_a_pending_switch_by_what_became_of_its_boundary_send` | table `Carried` {Unissued, InFlight, Sent, Unknown} × (`b < stop`, `b ≥ stop`) × `played` {MAX, `b`, `b + 1`}; reads `take_settled()` | the old `b < stop` rule; `played` ignored; `Unknown` applied |
| `c2_record_a_halt_during_the_boundary_send_waits_for_the_result` | `halt` with `InFlight`: `take_settled()` empty, `switch()` refused `PendingSwitch`; then `carried(Sent)` → `Applied` and the chains equal the applied ones; `carried(Unknown)` → `Uncertain`; `carried(Unissued)` → `Cancelled` | settling at the halt; losing the switch |
| `c2_record_a_second_halt_keeps_the_first_bound` | two halts in flight (S18); apply geometry uses the first | resetting `halted` to the later bound |
| `c2_record_advance_applies_only_a_sent_switch` | at `now ≥ b`: `Unissued`, `InFlight`, `Unknown` leave `settled` empty; `Sent` → `Applied`; a halted switch is not applied by `advance` | the old unconditional apply |
| `c2_record_an_abandon_settles_by_the_cursor_and_the_carry` | cursor ≤ `b` → `Cancelled`; cursor > `b`: `Sent` → `Applied`, `Unknown` → `Uncertain` (S6 to S9) | cursor test flipped; `Unknown` applied |
| `c2_record_the_footprint_does_not_depend_on_the_outcome` | `accept` answers after `halt` are equal for `Sent` and `Unknown`, and for the cancelled one (the union of chains is the same) | a footprint change turning a refusal into an acceptance |

**Book level** (same file):

| Test | Asserts | Fails on |
|---|---|---|
| `txbook_issue_marks_only_the_switch_s_own_first_send_in_flight` | the old repeat's send, the switch's second send and a finite first send leave `carried` alone | marking every issue |
| `txbook_complete_sets_carried_by_the_outcome` | whole and `Accepted(n ≥ 1)` → `Sent`; `Accepted(0)` and `TailShort` → `Unissued`; `Failed` → `Unknown`; later sends change nothing | `Accepted(0)` counted as sent |
| `txbook_receive_switch_starts_the_carry_by_the_provider_s_rule` | `AtFirstIssue` → `Unissued`; `AtAcceptance` → `Sent` | the Mock waiting for a send that never comes |
| `txbook_stop_passes_the_played_cut_to_the_record` | `stop_played((bound, region), cut)`: `Sent` with `b ≥ cut` cancelled, `b < cut` applied | `cut` ignored |

**UHD, frozen rig** (new `crates/ezsdr-radio-uhd/src/provider/tx/tests/c2.rs`, `mod c2;` beside `e3b`; probes at `tx_send_planned` as in `e2a_a_stop_received_between_the_plan_and_the_device_call_changes_nothing_in_the_send`; every device-clock wait through the participant helpers, plan 51):

| # | Test | Asserts | Fails on |
|---|---|---|---|
| U1 | `c2_s4_a_stop_before_the_boundary_send_cancels_the_switch` (the changed `e2a_s11_…`, plus a twin through `stop_receipt`) | row `cancelled`, one note `UR-26: cancelled by Stop`, no `NEW` sample on the device, burst records = 1 | applied row with no sample (KL-42) |
| U2 | `c2_s3_a_stop_after_the_boundary_send_leaves_it_applied` (`o1_uhd_t4e`, `ur_23_stop_after_handover_…` stay; one new cell, the Stop right after the first `NEW` send) | `applied`, `NEW` heard to the cut | cancelling a sent switch |
| U3 | `c2_s5_a_stop_in_flight_is_decided_by_the_result` (the probe: no fault, `FailSend`, `StalledSend`; Stop at the boundary send's `tx_send_planned`, both via `Streams::end` and via a `Control` Stop) | `applied` / `uncertain` / `cancelled`; the row is `pending` between hook and completion; notes only on `cancelled`; the device's `transmitted` agrees | settling at the halt; `uncertain` as applied |
| U4 | `o1_uhd_t7c_…` (changed) | row `uncertain`; half of `NEW` sounds; Kernel records unchanged | old `applied` |
| U5 | `c2_s8_a_short_boundary_send_is_applied_when_it_took_a_sample` (`ShortSend`) and `…_none_is_cancelled` (`StalledSend`) | `applied`; `cancelled` | `Accepted(0)` counted sent |
| U6 | `c2_s10_an_underflow_after_the_boundary_send_does_not_unsend_it` | report tick ≥ `b` after a whole boundary send: `applied`, no note | the abandon cancelling a `Sent` switch |
| U7 | `c2_s11_a_host_late_switch_is_pending_until_the_turn_then_cancelled` | `device_at` past `b`, no step: row `pending`; next step abandons at `c = b`: `cancelled`, no `NEW` | old `advance`; a row stuck pending |
| U8 | `c2_s17_a_second_switch_after_the_unissued_boundary_is_refused_as_pending` (the changed `e2a_84_…`) | `PendingSwitch` text (Q4) | a second switch accepted on an unissued one |
| U9 | `c2_s14_provider_stop_is_decided_like_a_stop` | `end_streams(.., lost = false)`: S4, S3, S5 cells | a Run end settling differently |
| U10 | `c2_s13_a_loss_before_the_boundary_is_decided_by_device_time` (after Q1) | `cancelled` for `b ≥ t_k` sent or not; `applied` for `b < t_k` | a loss settled by `B` |
| U11 | `c2_no_row_is_left_pending_when_the_run_ends` | each cell above followed by the shutdown: no `pending` row | rows stuck pending |
| U12 | `c2_s15_a_failed_reopen_cancels_even_a_sent_switch` | `cancelled` | applying on reopen failure |

**Mock** (`ezsdr-mock-radio/tests/mock_radio.rs`, `mock_channel/o1.rs`):

| # | Test | Asserts | Fails on |
|---|---|---|---|
| M1 | `c2_mr_25_a_stop_settles_a_switch_by_the_payload_cut` | Stop times around `b` (`B ≤ b`; `b ∈ [cut, B)`; `b < cut`), `x3xx` and `ideal`: row = `outcome_after_mock_stop`, **and** the row agrees with `bursts().len()` and what is heard (the new waveform exists iff `applied`) | the old `b < B` rule; `cut` ignored |
| M2 | `c2_mr_25_a_loss_settles_a_switch_by_the_loss_sample` (Q1) | a step that jumps both the loss and `b`, window 10 ms: `cancelled` for `b ≥ t_k`, `applied` for `b < t_k` | a loss settled by `B` |
| M3 | `mr_25_s11_loss_and_provider_stop_cancel_pending_switch`, `mr_25_s11_stop_before_the_boundary_applies_…`, `mr_16_s8_*`, `mr_16_pending_switch_ends_at_its_boundary` (stay) | unchanged rows | the Mock stuck pending |
| M4 | `mock_channel/o1.rs:396,435,448` (changed to the oracle's new signature) | `outcome_after_mock_stop` | |

**Parity** (`ezsdr-acceptance/tests/parity.rs`; no row equality across Providers where the hand rules differ, as `o1_parity_a_switched_repeat_…` states at :2916):

| Test | Asserts | Fails on |
|---|---|---|
| `c2_parity_a_switch_whose_boundary_precedes_the_stops_bound_is_cancelled_alike` (the existing `s15_parity_provider_stop_…` extended to a Stop action) | both `cancelled`; the old repeat is the only burst | one Provider applying |
| `c2_parity_a_switch_sent_before_the_stop_is_applied_alike` | repeat running 40 ms, Stop 20 ms after the switch: both `applied`, successor record on both | one Provider cancelling a sent switch |
| `c2_parity_the_row_follows_each_provider_s_own_emission` | Stop with `b` near the window edge: each row equals its own oracle answer and its own record count | rows chosen by `B` |

**Oracle rows** (`o1.rs`): T4c, T4d, T4e, T7a, T7a3, T7c call the new oracle functions with the observed send state; their `C2` comments (`o1.rs:469,475,861,866`; Mock `o1.rs:430,435`) are removed. T4d's emission assertion is untouched, its row is the oracle's answer for `NotIssued` (`cancelled`); the successor record count stays 1 and `r.cancellations()` stays 1.

### 3.3 Mutation rows (Rust token anchors; only through `remote_mutate.py`; each killer fails on an assertion)

| Row | Mutates | Killer |
|---|---|---|
| C2-01 | `halt`: drop `switch.boundary < stop` from the `Sent` arm | `c2_record_a_halt_settles_…` |
| C2-02 | `halt`: `played` ignored (`stop.min(played)` → `stop`) | `c2_record_a_halt_settles_…`, `txbook_stop_passes_the_played_cut_…` |
| C2-03 | `settle_halted`: `Unknown` treated as `Sent` | `c2_record_…`, `o1_uhd_t7c_…` |
| C2-04 | `settle_halted`: `InFlight` settled at the halt | `c2_record_a_halt_during_…`, U3 |
| C2-05 | `carried()`: deferred settle skipped | `c2_record_a_halt_during_…`, U3 |
| C2-06 | `advance`: drop `carried == Sent` | `c2_record_advance_applies_only_a_sent_switch`, U7 |
| C2-07 | `TxBook::issue`: mark in flight for every booking | `txbook_issue_marks_only_…` |
| C2-08 | `TxBook::issue`: never mark | `txbook_issue_marks_only_…`, U3 |
| C2-09 | `TxBook::complete`: `Accepted(n) if n >= 1` → `Accepted(_)` | `txbook_complete_sets_carried_…`, U5 |
| C2-10 | `TxBook::complete`: `Failed` → `Sent` | `txbook_complete_…`, `o1_uhd_t7c_…` |
| C2-11 | `receive_switch`: the `AtFirstIssue` arm dropped (UHD starts `Sent`) | `txbook_receive_switch_starts_…`, U1 |
| C2-12 | `receive_switch`: the Mock starts `Unissued` | M3 (rows stuck pending), `txbook_receive_switch_starts_…` |
| C2-13 | Mock `stop_tx`: pass `i64::MAX` instead of `cut` | M1 |
| C2-14 | `SwitchOutcome::Uncertain` label → `"applied"` | `o1_uhd_t7c_…` |
| C2-15 | UHD loss path: `played` ignored (after Q1) | U10, M2 |

Existing rows to retarget (anchors move; killers stay): **N47S5-35** (`advance` filter), **N47S5-36** (`halt`'s cancelled arm), **N47S5-147, N47S5-155** (abandon `boundary >= cursor_k`), **N47S5-44** (reopen failure), **N47S5-94, N47S5-132** (Mock `advance`), **N47S5-43** (UHD `book.advance`), **E2A3-03, E2A3-04** (`playing.started = true`, the line that now also marks the carry). **Retire with the dead code (C2-5):** N47S5-136, E2A2B-01, E2A2B-05, E2A2B-12 (`never_started`), N47S5-152, E2A2B-06 (the rewrite); the killers they name are re-pointed at a surviving row before anything is deleted (`rules.py` checks each row anchors exactly once). The full lists run at the sub-step commits and before Sol's code review (AGENTS §7).

### 3.4 Existing tests whose expectation changes (probe-verified: the only ones that failed)

| Test | Old | New | Why |
|---|---|---|---|
| `e2a_s11_a_stop_after_a_switch_whose_boundary_is_before_the_bound_applies_it_and_cancels_the_payload` (`tx/tests/e2a.rs:559`) | `applied`, note, no `NEW` | `cancelled` (rename) | S4, KL-42 |
| `o1_uhd_t4d_…_leaves_the_row_applied_with_no_new_sample_handed` (`o1.rs:466`) | `applied` (literal at :477) | `cancelled` (rename); :476 follows the oracle | S4 |
| `o1_uhd_t7c_a_failed_send_that_carries_the_boundary_leaves_the_switch_applied_today` (`o1.rs:859`) | `applied` | `uncertain` (rename) | S9 |
| `e2a_84_a_second_switch_on_a_switch_the_host_has_not_issued_is_refused` (`e2a.rs:447`) | `UR-38: … has not started on the device` | `UR-21: a repeat switch needs a playing repeat and no pending switch` (Q4) | S17 |
| `txbook_receive_switch_takes_the_boundary_at_receipt` (`staging.rs`, :2016) | `advance(20)` applies | applies after `carried(Sent)`; with `AtFirstIssue` it does not before | `advance` needs `Sent` |
| `txbook_issue_starts_the_playing_repeat_and_a_switch_s_own_payload` (:2148) | second switch after `advance`: `NotStarted` | `PendingSwitch` until `Sent`, then `NotStarted` | S17 at the book |

Not changed (probe green): all Mock tests; the other 141 record tests; the 416 `fake.rs` tests; every acceptance test including `s15_parity_*` and `o1_parity_a_switched_repeat_…`; the e2b snapshot tests (their `switches` rows are `pending` or empty). The record-level `rm_16_*` tests build records with `Bookings::switch` directly and never see `Unissued`. **Decision for C2-2:** `Bookings::switch` takes `carried` explicitly (16 call sites, all tests) rather than defaulting to `Sent` as the probe did, so the Provider's rule is visible.

Deleted with the dead rewrite (C2-5), each after showing its failure case is unreachable (§0 item 6): cells (b) and (c) of `txbook_abandon_equals_a_stop_at_the_cursor_except_the_named_differences` (`staging.rs:1536-1552`), the two `applied` cases of `txbook_abandon_at_a_switch_boundary_extends_the_old_repeat_to_its_footprint` (:1597), `switch_applied()` if unused; `e2a_f3_a_booking_at_b_plus_one_after_an_abandon_at_a_switch_boundary_is_refused_at_receipt` (`e2a.rs:291`) builds T4d's state with a Stop, whose switch is now `cancelled`: it becomes `t4d_state(false)` and `e2a_f3_the_pending_twin_answers_the_same` already covers that cell, so it is merged, not lost.

## 4. Schema and text

### 4.1 Schema
None (§0 item 1). `SwitchOutcome::label` gains `"uncertain"`; `SwitchRecord.outcome` stays `&'static str`. `schema_freeze` runs at the gate and must stay green. No `EZSDR_UPDATE_SCHEMAS`, no `SCHEMA_CHANGELOG.md`, no version bump (AGENTS §6).

### 4.2 `design/` (spec 22 DA-1: normative text only, no rationale, `rules.py --base fbde789` must pass)

- **RM-37**, second paragraph: replace "A Stop, a loss or `Provider::stop` whose bound `B` is at or before `b` cancels the switch, … so does a Provider that ends the repeat on its own … before handing the sample at `b`" with the rule of §0: `applied` when the Provider took the sample at `b` and device time has reached it or a Stop or `Provider::stop` with `B > b` follows (RM-16: what is handed is not recalled); `cancelled` when a Stop, `Provider::stop`, a loss or an abandon comes before the sample at `b` was taken, or `B ≤ b`; `uncertain` when the send that carries the sample at `b` ended in an exception. Test line: new names, minus retired ones.
- **UR-38**, "The pending switch is applied when device time reaches its boundary": applied once the owner has sent the sample at the boundary and device time reaches it; a Stop whose bound is at or before the boundary cancels it; a later one settles it by the boundary send (sent: applied; not issued or none taken: cancelled; exception: uncertain). The abandon sentence follows. Test line.
- **MR-42**, "`applied` or `cancelled` once the record settles it": the Mock's boundary send is an ideal executor's, so a switch is `applied` at device time `b`; at a Stop, `Provider::stop` or loss it is `applied` iff `b` is below the payload cut (MR-25; for a loss, the loss sample); never `uncertain` (no send faults, M5). Test line.
- **MR-25, RM-16**: no change (the cut is unchanged).
- **Changes tables**: one row each in specs 07, 09, 18 pointing at note 49 stage C2.
- **Note 49**: §5.4.5's table gets the in-flight row (S5); the Changes table and §12's C2 row get the C2 line at the gate. §11.10 (a decision record) is not edited. `plan/maintenance/failure-mechanisms.md`: the finding is `planned-state-read-as-actual` (the row recorded a plan as an actual); counted in the review record.
- **KNOWN_LIMITATIONS.md**: **KL-42** updated in the same commit as the fix: "Gone in note 49 stage C2; the entry stays for its number" (KL-12's form), with the corrected symptom (§0 item 2). **KL-41** gets one clause if Q4 keeps `PendingSwitch`. No new entry unless Q1 is answered `applied`.
- **handoff.md**: one line, at the gate.

## 5. Sub-steps (each leaves the tree green)

| # | Content | Rows |
|---|---|---|
| C2-1 | Oracle functions and `o1_oracle.rs` self-tests (§3.1); no production change | none |
| C2-2 | Record and book: `Carried`, `Uncertain`, `Bookings::switch(.., carried)`, `halt`/`carried`/`advance`/`abandon`, `TxBook::issue`/`complete`/`receive_switch`/`stop_played`; the Mock's `stop_tx` passes `cut` (the 6-line reorder). `tx.rs` needs no change: the hooks are in the book. The 6 tests of §3.4 and the oracle's call sites change in this commit; new record, book, U1 to U9, U11, U12, M1, M3, parity tests; RM-37, UR-38, MR-42, KL-42 (and KL-41) in the same commit (AGENTS §2) | C2-01 to C2-14, the retargets |
| C2-3 | Loss (Q1): `Streams::end(lost)` passes the loss sample as `played`; Mock loss as derived (`cut = t_k`); U10, M2 | C2-15 |
| C2-4 | Hardware verdict (`uncertain`, Q2) and its `iii.rs` unit test | none |
| C2-5 | Delete the unreachable rewrite (§0 item 6): first a `debug_assert!(cursor > boundary)` run of the full suite in a copy; then the code, `never_started`, their rows and tests | retire six rows |

C2-2 cannot be split without a red tree: once `TxBook` marks the carry the UHD's behaviour changes. The risky part (S5) lives in `Bookings` and `TxBook`, so Sol's code review reads one diff of about 150 production lines plus tests. Self-review (§9 step 5) runs once per sub-step. New record and book tests are pure and not repeated; U3, U7, U11 run on the frozen rig (plan 51 §3: the rig's thread is a participant of the manual clock) and do not depend on when a thread runs; the unfrozen acceptance parity cells already carry the stage 1b participant.

## 6. Hardware tests and bench values (AGENTS §9 audit)

| Test | Expectation computed from this rule | Verdict | Default-suite rehearsal |
|---|---|---|---|
| `hw_n47s5_repeat_switch` (`hardware.rs:3843`; trial `n47s5::repeat_switch_trial`: repeat from 20 ms, switch 40 ms ahead, Stop 60 ms after) | `row == "applied"`; `iii::switch_verdict` fails on any other text | **still right**: the boundary is about 12 ms after the receipt, the Stop 60 ms after it | `n47s5_repeat_switch_rehearsal_on_the_fake` (`fake.rs:7871`), the same functions and boundary |
| `iii::switch_verdict` unit tests (`iii.rs:1153-1390`, `"applied"`/`"cancelled"`) | row text | still right; **add** an `"uncertain"` case, verdict `Inconclusive` (Q2) | the unit tests |
| other readers of `switches` (`fake.rs:2067`, `7882`; `repeat_switch.rs:38`) | | still right (probe green) | default suite |

There is **no bench test of "a Stop inside the window before the boundary send"** and I propose none: it exercises H1, already VERIFIED twice. If the owner wants it, it is a T4b of `hw_n47s5_repeat_switch` with the Stop 4 ms after the switch (needs the attenuator confirmation, handoff §2); expected row `cancelled`, no new tone; its FakeDevice rehearsal is U1 with the same times. C2 fixes no value from bench data, so the §9 "value fixed from bench data" conditions do not apply.

## 7. Questions for the orchestrator (none decided by me)

- **Q1 (owner: a user-visible outcome). A loss while the switch is not applied by device time.** The decision's three values do not name a loss. Today both Providers settle a loss by the Stop bound (`b < B` → `applied`), which says `applied` for a sample the lost device never played when `b ∈ [t_k, B)`. Recommendation: **a loss settles by device time at the loss**: `applied` iff `b < t_k` (what `advance` would already have done), `uncertain` if the boundary send failed, else `cancelled`. A loss ends the transmitter at once, unlike a Stop. Cost: UHD `Streams::end` passes the loss sample as `played` (the probe's `halt_played`); the Mock's loss already uses `cut = t_k`. Alternative: leave a loss as a Stop: smaller, but the row can say `applied` for a sample never emitted, the defect C2 fixes.
- **Q2 (owner: how `uncertain` shows).** Python never reads the `switches` rows and no server code does; `uncertain` is visible only in the Manifest. Recommendation: nothing in the Python client (no raise, no accessor); `iii::switch_verdict` gives `Inconclusive` for `uncertain`, since the bench cannot tell either.
- **Q3 (orchestrator). A Stop while the boundary send is in flight settles by the result.** The three values do not say; "whether the sample at its boundary was sent" does. The row reads `pending` until the completion (up to the 1 s `SEND_TIMEOUT`). It needs one field, `Switch.halted`. The alternative (settle at the Stop and correct later) fails because rows are drained lazily.
- **Q4 (owner: refusal text). A second switch on a switch whose boundary has passed unissued** now reads `UR-21: a repeat switch needs a playing repeat and no pending switch` instead of `UR-38: the repeat the switch replaces has not started on the device` (S17). Both refuse the same case; KL-41 quotes the second. Recommendation: accept the new text (a switch is pending), add one clause to KL-41, and keep `Bookings::switch`'s refusal order (reordering changes the pre-boundary case that `txbook_a_refused_switch_changes_nothing` pins).
- **Q5 (owner: a Manifest difference between the Providers).** The Mock's boundary send is "the first 32 samples fit the window"; UHD's is a whole 2 500-sample chunk (the continuation rule). For a Stop with `b` near the window edge the Mock can read `applied` where UHD reads `cancelled`, each agreeing with what it emitted (the existing allowance: no row outcome is compared in `o1_parity_a_switched_repeat_…`). Recommendation: accept, state it in MR-42 as MR-25 does for the cut. Alternative: give the Mock a continuation-sized first send for a switch (a change of its Stop cut that C2 does not need).
- **Q6 (orchestrator). KL-42's form after the fix:** "Gone in C2, entry stays for its number" (KL-12), with the corrected symptom. The brief says "resolved".
- **Q7 (orchestrator). `Bookings::switch(.., carried)` explicit rather than defaulted** (§3.4): recommended; 16 test call-site edits.

## 8. What I did not read or run, and limits

- Not run: `-p ezsdr-kernel` (`schema_freeze`), `-p ezsdr-server`, the Python suites, the `uhd` feature (libuhd absent), the hardware tests, `rules.py`, `gate.py`, any mutation row. The probe covered the five crates that read a switch outcome.
- The probe left the UHD loss path unchanged (`played = i64::MAX`), so Q1's UHD half is not probed; the Mock's half follows from `cut = at_stop` for a loss (`lib.rs:1164`), read, not run.
- The Mock window's behaviour *before* the change (§0 item 2) is read from `halt` (`staging.rs:1169`) and the cut formula (`lib.rs:1161`), not run on `main`; the probe shows the new behaviour only.
- The reorder of the Mock's `stop_tx` assumes `tx_handed_end` and `list.halt` do not depend on `book.stop` (`lib.rs:1391-1397` reads `book.open` and `list.held` only); all Mock and acceptance tests passed with it.
- Stated limit (S12): a send the host issues too late, which the device drops (H2), still reads `applied`; the host cannot tell. Same as today.
- A `halted` second Stop and `Reopen::Failed` with a `Sent` switch are tested (S18, S15) but were not probed.

## Changes in C2-5 (implementer's record, 2026-11-12; where it differs from the text above, this section wins)

C2-5 as amended (§D): delete the abandon's rewrite of settled `applied` rows, which C2 made unreachable. Tree: `work/c2` over `8c5422f` (C2-3, C2-4); nothing committed.

**Reachability probe (before any deletion).** A copy outside the repository (`/tmp/opencode/c2-5-probe`, `rsync -a --no-times --exclude .cargo --exclude .git --exclude v3`, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`, local) with an unconditional `panic!("C2-5 PROBE: …")` as the first statement of the loop `for s in self.settled.iter_mut().filter(|s| s.outcome == Applied && s.boundary >= cursor_k)`; the plan asked for a `debug_assert!(cursor > boundary)`, and a panic in the loop body is the same assertion on the rows that matter. `cargo test --workspace --no-fail-fast` in the copy: every crate green except two tests of `-p ezsdr-radio --test staging`, which hit the panic (`boundary 10, cursor 10` and `boundary 10, cursor 5`): the record-level cells that build an applied switch by hand with `advance` and abandon it at or before its boundary, exactly the cells this row deletes. The UHD `lib` and `fake` suites, the Mock, the acceptance parity and every other test never reach the rewrite (no panic), so it is unreachable through any Provider path.

**Production.** `staging.rs`, `Bookings::abandon`: the loop that rewrote an `applied` row with `boundary >= cursor_k` to `cancelled`, `never_started` and its test in the partition go (`.partition(|c| c.start >= stop)`); the doc comments of `Bookings::abandon` and `TxBook::cancel_switch` no longer speak of an applied switch the abandon cancels. Why it is unreachable: a row is `applied` only once its boundary send was taken (`Carried::Sent`: `advance`, `settle_halted`), and a send that took the sample at `b` leaves the handed cursor past `b`, so every abandon (`cursor_k`) of a repeat with an applied row has `cursor_k > b`; an abandon's own settlement of a pending switch is the `halt` that follows.

**Tests deleted (§3.4's last paragraph).** `txbook_abandon_equals_a_stop_at_the_cursor_except_the_named_differences`: cells (c) and (b) (the loop over the table stays; its `applied S, abandoned past its boundary` row at cursor 15 is reachable and stays). `txbook_abandon_at_a_switch_boundary_extends_the_old_repeat_to_its_footprint`: the two `applied` cases go; the one `pending, c == b` case stays (a one-element `for` became straight code: clippy's `single_element_loop`). `e2a_f3_a_booking_at_b_plus_one_after_an_abandon_at_a_switch_boundary_is_refused_at_receipt` and its helper `t4d_state` are deleted rather than turned into `t4d_state(false)`: with no Stop the state is the pending twin's, which `e2a_f3_the_pending_twin_answers_the_same` already runs (its comment now stands alone); RM-37's Test line (the spec text that named the deleted test) names the twin. `switch_applied()` stays (used by `abandon_rows`).

**Mutation rows.** Retired with the code: N47S5-152 and E2A2B-06 (the rewrite's `boundary >= cursor_k`) and E2A2B-12 (`never_started`). **Retargeted, not retired:** N47S5-136, E2A2B-01 and E2A2B-05 mutate the partition's `c.start >= stop` rule, which stays and whose killers (`ur_21_s4_an_underflow_ends_…`, `e2a_80_a_failed_send_after_a_stop_…`, `e2a_80_an_abandon_at_the_repeat_s_own_start_…`) still apply; only their anchors lose `&& !never_started.contains(&c.start)`. This differs from "retire six rows" of §3.3: three of the six name behaviour that stays reachable, as the brief allows. No `known-survivors.txt` entry.

**Not changed.** No behaviour, no spec rule, no schema, no version.

**Added in C2-5: a killer for E2A2B-04.** E2A2B-04 (`repeat.front = stop` → `repeat.front.min(stop)` in `abandon`) had only the deleted `applied` cases of `txbook_abandon_at_a_switch_boundary_extends_the_old_repeat_to_its_footprint` as killer (an applied switch had lowered R's front to `b`); it SURVIVED after the deletion. The rule stays reachable (a Stop capped R below the cursor the device had been handed to), so a new pure record test `txbook_abandon_sets_the_repeat_s_front_to_its_footprint_even_when_a_stop_cut_it_lower` (R from 0, `halt(2, 7)`, abandon at 10: `accept(14)` refused, `accept(15)` free) and the row's filter is re-pointed at it; `killed`.

**Rows run** (`remote_mutate.py maintenance sim03:4`, 8 rows, then E2A2B-04): N47S5-136, E2A2B-01, E2A2B-05 (retargeted), E2A2B-04 (re-pointed), E2A2B-13, E3B-12, N47S5-153, N47S5-154 (neighbours): all `killed`. Retired: N47S5-152, E2A2B-06, E2A2B-12. `renames.txt` gained `e2a_f3_a_booking_at_b_plus_one_… -> e2a_f3_the_pending_twin_answers_the_same`. Checks: `cargo fmt --all --check`, `cargo test --workspace` (74 result lines ok; the pre-existing `n47s5_front_rehearsal_on_the_fake` passed), `cargo clippy --workspace --all-targets -- -D warnings`, `rules.py --base fbde789 --renames …` = 0. The Python suites are not run (no `python/` change).

## Changes in fix round 1 (implementer's record, 2026-11-12, after Sol's code review `sol-code.md`; where it differs from the text above, this section wins)

Tree: `work/c2` over `e2b1810`; nothing committed. All three findings are adopted (decided by the orchestrator). Each major's test was written first and seen to fail on `e2b1810`.

**Major 1: a loss after `Provider::stop` keeps the playback cutoff.**
- *Failing test.* UHD frozen rig, `tx/tests/c2.rs`: `c2_s22_a_loss_after_provider_stop_in_flight_keeps_the_loss_sample`: at the boundary send's `tx_send_planned` hook `Provider::stop` (`end_streams(.., lost = false)`, bound past b: the row waits) and then a loss at the same instant (a sample before b); cells: whole send → `cancelled`, `FailSend` → `uncertain`, whole send with device time past b at the loss → `applied`; each in both orders (the loss first, then `Provider::stop`). On `e2b1810` the first cell reads `applied` (`Streams::end` returned at `ended` before `stop_at`, so `played` stayed `i64::MAX`).
- *Fix.* `Bookings::lose(played)` (`staging.rs`): a loss after the streams ended lowers `played` of the halt a switch is waiting on (`halted.played = min`); no halt, no pending switch, or a settled switch: nothing. `StreamState::lose_at(at)` (`core.rs`) passes the loss sample (`clock.try_at_or_after(at)`, the one `stop_at` uses); `Streams::end` calls it for every transmit stream when `ended` was already set and `lost` is true, before it returns: no end is booked again, no stream is re-ended. The reverse order (the loss first) was already right (the first `end` books the loss; the later `Provider::stop` returns at `ended`) and the test pins it.
- *Record test.* `c2_record_a_loss_after_a_halt_in_flight_keeps_the_earliest_sample` (`staging.rs`, pure): losses `[90]`, `[120]`, `[120, 90]`, `[90, 120]` after a Stop with bound 150 in flight; a loss with no halt before it and a loss after the switch settled change nothing.
- *Rows.* C2-23 (`Streams::end` does not call `lose_at`; killer U `c2_s22_…`), C2-24 (`halted.played = played`, a later loss overwrites; killer `c2_record_a_loss_after_a_halt_in_flight_keeps_the_earliest_sample`).

**Major 2: a boundary send whose result never comes.**
- *Rule and place (decided by the orchestrator, one place).* At the Provider's Manifest write, `UhdRadio::write_sections` (`provider/mod.rs`), a switch still `InFlight` is settled as a send whose result is unknown: `uncertain`. `Core::settle_unresolved_switches` (new, `core.rs`) calls `Bookings::settle_unresolved` on every transmit stream's record, just before `settle_switches`. It is `write_sections` and not the end of the owner's thread, and not `settle_switches`, because both the panic path and the detached one (UR-16) pass `write_sections` exactly once (from `cleanup`), a worker's thread end is not reached by a detached one, and `settle_switches` is also what the tests' `switch_row` calls mid-Run (a settlement there would hide the pending state). A halt whose bound is at or before b has settled the row `cancelled` at the halt already (the oracle's order), so only `uncertain` is written here.
- *Deviation from "the same settlement as `Outcome::Failed`".* `settle_unresolved` writes `uncertain` itself and does not go through `Bookings::carried(.., Unknown)`: a first version did, and mutating the call away survived: after the Manifest write the record's chains are never read, and the geometry `apply` leaves (the old repeat's front at b, the new chain `[b, B)`) is the same union of regions as the halt left, so the only observable result of the two forms is the outcome. Direct is smaller and the outcome rule is the same one (`Unknown` → `uncertain` when `B > b`, and a pending switch has `B > b`).
- *Failing tests.* (1) `c2_a_boundary_send_whose_worker_panicked_is_uncertain_in_the_manifest` (`provider/mod.rs`, `UhdRadio` with a real thread that panics: `running_radio`, `panics_when_released`, then `stop` and `cleanup`): the record holds a switch issued and in flight, and the Manifest's `switches` row reads `pending` on `e2b1810` (the panic handler books a loss and nobody completes the send); cases: no clock (no halt), a clock with `B > b` (the loss's halt waits), a clock with `B ≤ b` (cancelled at the halt, written `cancelled`). The row is `uncertain`, `uncertain`, `cancelled`. It does not depend on when a thread runs (the panic is the test's own, `all_finished` waits for it): 20 runs, all green. (2) UHD frozen rig `c2_s23_a_worker_that_panics_in_the_boundary_send_leaves_the_row_uncertain_at_the_close`: the owner panics in the hook at the boundary send's `tx_send_planned` (a scripted panic through the probe, caught by `catch_unwind` around the rig's run, then `device_lost` as the spawn handler does): the row is `pending` until `core.settle_unresolved_switches()`, then equals `oracle::switch_outcome_at_close(b, stop)`; a Stop with a bound at or before b first (the 3 ms one) gives `cancelled`. (It uses the new method, so it cannot run on `e2b1810`; row C2-28 is its failure mode.) (3) Record: `c2_record_an_unresolved_boundary_send_is_uncertain_when_the_manifest_is_written`: in flight with and without a halt (Stop, loss) → `uncertain` and nothing pending after; a halt with `B ≤ b` → `cancelled` once; `Unissued`, `Sent`, `Unknown` untouched. (4) Oracle: `switch_outcome_at_close(b, stop)` = `halted(b, stop, None, Sent::Failed)` and its self-test `o1_oracle_c2_an_unresolved_boundary_send_is_uncertain_when_the_manifest_is_written` (equal to `switch_outcome_after_stop(.., Sent::Failed)`; `B` at or before `b` cancels).
- *Not changed: the Mock.* Its boundary emission is atomic (M5): no send is ever in flight on it, so no row of it is unresolved at its Manifest write.
- *Rows.* C2-25 (the Manifest write settles a pending switch whatever its carry: `take_if(|_| true)`; killer the record test), C2-26 (`Uncertain` → `Applied` in the row it writes; killer the record test), C2-27 (`write_sections` does not call `settle_unresolved_switches`; killer `c2_a_boundary_send_whose_worker_panicked_…`), C2-28 (`settle_unresolved_switches` settles nothing; killer `c2_s23_…`).

**Minor 3: the specs.** RM-37 (second paragraph and Test line), UR-38 (the pending-switch paragraph and Test line) and note 49 §5.4.5 (the in-flight row) say, in one clause each: a Stop or loss whose bound is at or before b cancels at once, even while the boundary send is in flight (the oracle's order; the record already does it, `c2_record_a_halt_during_the_boundary_send_waits_for_the_result`); and a boundary send that has no result when the Manifest is written settles the switch as an exception does: `uncertain`. §5.4.5 also says the loss after `Provider::stop` keeps its sample. RM-37's paragraph was split so that no line passes 1 500 characters (`rules.py`). No other spec text, no schema, no version.

**Row retarget.** P243 (`Streams::end`'s early return) anchors on `if std::mem::replace(&mut self.ended, true) {` now (the `return` is no longer the block's only token after it); same mutant.

**Checks and outcomes.** `remote_mutate.py maintenance sim03:4` on C2-23 to C2-28 (and, run before the C2-25 retarget, C2-25's first form, which SURVIVED: see the deviation), P243, C2-16, C2-17, C2-19, N47S5-24, N47S4-43, C2-04, C2-05: all 14 `killed` in the final run (`fix1-mut2.log`; `fix1-mut.log` is the run before the C2-25 retarget). `cargo fmt --all`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `rules.py --base fbde789 --renames …` = 0: `cargo fmt --all --check` clean, `cargo test --workspace` 74 result lines ok (2 478 tests, no failure), clippy clean, `rules.py` = 0. The Python suites were not run (no `python/` change).

## Changes in fix round 2 (implementer's record, 2026-11-12, after the Opus re-check `recheck1.md`; where it differs from the text above, this section wins)

Tree: `work/c2` over `9f6b853`; nothing committed. All items adopted (decided by the orchestrator).

**Item 1: a Stop's `applied` is overturned by a later loss before `b` (Q1: a loss settles by device time).**
- *Rule (one function, `Bookings::settle_halt`).* Every halt (`halt_played`), every boundary-send result (`carried`) and every later loss (`lose`) records the halt on the pending switch (`Halted { at, stop, played }`: `at` the sample the first halt took effect at, i.e. device time then, the later halt's `at` taking the larger) and calls `settle_halt`. The switch keeps waiting when `b < stop` and either its boundary send is in flight, or it is `Sent`, device time at the halt is short of `b` (`at < b`) and the sample at `b` was played (`b < played`). Otherwise `settle_halted` decides as before (one settlement function). The wait ends in `advance` (device time reaches `b`: applied, with the first halt's `stop` as the new repeat's front), in `lose` (a loss at a sample at or before `b` lowers `played` and `settle_halt` cancels), and in `settle_unresolved` (the Manifest write: a `Sent` switch with a halt is applied; one in flight is uncertain as before).
- *Wider than the brief's text, one consequence.* The in-flight form takes the same path: a Stop in flight whose result is whole, with `at < b`, also stays pending (before, `carried` settled it applied at once, so a loss after the result was a no-op: the same overturn). Existing record and rig tests that expected `applied` at once for a Stop at a device time below `b` were changed to the new row (`pending`, then applied by device time or the Manifest write) or moved their halt to device time `b`; their intent (the carry decides) is kept. `carry_of` probes at device time 30 (b = 20).
- *An abandon (the owner's own end of the repeat, UR-22) is not changed:* it halts at its cursor, which is past `b` when the send was taken (`c2_s5`'s `ShortSend` cell stays `applied` at once). A loss after that abandon and before device time reaches `b` is the same overturn, not covered by this round (reported).
- *The Mock.* `MockRadio::stop_tx` calls `settle_unresolved` right after `stop_played`: its payload cut is known at the Stop and no loss can follow, so it does not hold a sent switch back for device time (MR-42). Without it a Stop below `b` that hands `b` (x3xx, window edge `b + 600`) left the row `pending` at the Manifest.
- *Failing tests (on `9f6b853`).* Record, `c2_record_a_stop_before_device_time_reaches_b_leaves_the_switch_to_a_later_loss` (both forms: Sent then the halt, and halt in flight then result Sent): the row reads `applied` at once, expected `pending`; then advance, loss at 90, 100 (cancelled) and 101 (stays), a loss through `halt_played`, a second Stop at device time 110 (applied, first bound), the Manifest write, and the device time `b`, 120 cells. UHD frozen rig: `c2_s24_a_loss_after_a_stop_below_b_overturns_a_whole_boundary_send_s_applied` (a `Provider::stop` and uhd-control's Stop, each with: a loss at the Stop's instant, a loss past b, device time reaching b, the Manifest write; every cell compared with `o::switch_outcome_after_loss` / `switch_outcome_after_stop`; run on `9f6b853`'s `staging.rs` it fails at "pending", the row says `applied`) and `c2_s25_a_loss_after_the_result_of_a_send_in_flight_at_a_stop_overturns_applied` (the Stop in flight, the result whole, then a loss at the same device time: `cancelled`). Mock: `c2_mr_25_a_stop_below_b_settles_the_row_at_the_stop` (pending on the tree with the record change and without the Mock call).
- *Rows.* New: C2-29 (a sent switch is settled at once instead of waiting), C2-30 (waits at `at == b`), C2-31 (waits though the loss did not play `b`), C2-32 (`lose` does not settle), C2-33 (advance applies with the open geometry instead of the Stop's bound), C2-34 (the Manifest write leaves it pending), C2-35 (the Manifest write gives `uncertain`), C2-36 (a second halt does not take the later `at`), C2-37 (the Mock's `settle_unresolved`), C2-38 (a loss does not reach the waiting switch). Retargeted to the new code, same mutants: C2-04 (`Carried::InFlight => false`), C2-05 (`carried` does not call `settle_halt`), C2-16 (`settle_halted(.., i64::MAX)` forgets the loss), C2-21 (a Stop at or before `b` waits for the result: the `b < stop` guard removed), C2-25, C2-26.

**Item 2: RM-37 and UR-38.** Said once each: the exception wins over a loss at or before `b` (code: `Unknown` before `played`, `settle_halted`); a Stop with `B > b` after the boundary send is `applied` unless a loss at a sample at or before `b` follows before device time reaches `b`, and the Manifest write applies a switch a Stop left pending. Test lines name the new tests. `rules.py` line limit: the RM-37 and UR-38 paragraphs were split.

**Item 3: note 49 §5.4.5.** The in-flight row now says: Stop or loss with `B` at or before `b` cancels at once; a loss whose sample is at or before `b` cancels (`Failed`: `uncertain`); the Stop's `applied` is not final before device time reaches `b`.

**Item 4: the oracle.** `switch_outcome_after_loss(b, bound, loss, sent)` takes the first halt's bound (the cancel-at-once case is expressible); `switch_outcome_after_stop_and_loss`, now identical, is removed and its callers use it. `o1_oracle_c2_a_loss_settles_a_switch_by_device_time_at_the_loss` gives the loss's bound past `b`, and a cell for a bound before `b` (`Failed` → `cancelled`). `o1_oracle_c2_a_stop_and_a_loss_together_…` also pins the Stop alone (`applied`). The `provider/mod.rs` panic test (`c2_a_boundary_send_whose_worker_panicked_…`) reads its three expected rows from the oracle (`switch_outcome_at_close`, `switch_outcome_after_loss`); `tx.rs`'s `oracle` module is `pub(super)` for it. 20 runs of that test, all green.

**Item 5: recorded, not changed.** The detached-join path (UR-16) reaches `settle_unresolved` with no test of its own; C2-24 is equivalent in production (one loss is published) and is a staging-level contract row.

**Fix round 2: checks and outcomes.** Before the mutation run: `cargo fmt --all --check` clean, `cargo test --workspace --no-fail-fast` no failure, `cargo clippy --workspace --all-targets -- -D warnings` clean, `rules.py --base fbde789 --renames …` = 0 (all on my tree). `remote_mutate.py maintenance sim03:4` on 59 rows (all C2-*, the staging.rs/Mock rows that touch a pending switch, the rows whose killing test I changed; log `fix2-mut.log`): 56 `killed`, 3 `NOT FOUND: old occurs 0 times` (C2-26, C2-35, C2-36). The run waited for the sim lock until 20:50 and copied the tree while another implementer (`brief-fix3.md`, started 20:46) was editing `staging.rs` and `mutations.json`, so the copy mixed its edits with mine (`halt_at`, `pending_switch`, a changed `settle_unresolved`); the three rows are not a verdict on my tree and must be run again once fix 3 has landed. The three rows' anchors match my tree's text before that edit.

## Changes in fix round 3 (implementer's record, 2026-11-12, after the Opus re-check `recheck2.md`; where it differs from the text above, this section wins)

Tree: `work/c2` over `aed9f0d` (fix round 2); nothing committed. All items adopted (decided by the orchestrator). Round 2's rows (fix2-ids.txt) run with this round's in one run (`fix3-mut.log`).

**Item 1: an abandon halts at device time, not at its cursor.**
- *Fix.* `Bookings::abandon(cursor_k, now_k, bound)` and `TxBook::abandon(repeat, id, cursor_k, now_k, bound)` take device time (uhd-tx passes `clock.at_or_after(core.now())`, as a Stop's `at`). A private `Bookings::halt_at(t, at, stop, played)` cuts the chains at `t` (the cursor) and records `Halted.at = at`; `halt_played(t, ..)` is `halt_at(t, t, ..)`, so no other halt changes. `settle_halt` is untouched: a sent switch whose `b` device time has not reached waits like one a Stop holds (a loss at a sample ≤ b cancels; `advance` or the Manifest write apply). `TxBook::abandon` still clears `playing` when the switch stays pending (`Bookings::pending_switch`, new, one line): the repeat is over either way.
- *Failing tests (on the round-2 `staging.rs`, signature adapted).* Record: `c2_record_an_abandon_halts_at_device_time_and_cuts_the_chains_at_the_cursor` (Sent, abandon at cursor 150 with device time 50: the abandon returned `Applied` at once, expected nothing; then advance 99/100, loss at 90/100 cancelled and 101 left to device time, the Manifest write, device time `b` applied by the abandon itself, a Stop that left the row waiting followed by an abandon at device time 110 (applied) and at 60 (still waiting), the chains cut at the cursor's front) and `c2_txbook_an_abandon_that_leaves_the_switch_waiting_ends_the_playing_repeat` (`playing` cleared for `id` Some and None). UHD frozen rig: `c2_s26_an_abandon_after_a_whole_boundary_send_halts_at_device_time` (the boundary send whole, the seventh send fails after half its samples at 9 ms (`FailSend(6)`, not a loss), so the abandon is at a cursor past b with device time short of it; cells: loss at the abandon, loss past b, device time reaching b, the Manifest write, each against `o::switch_outcome_after_abandon_and_loss` / `switch_outcome_after_abandon`; on the round-2 tree it fails at "pending", the row says `applied`). Oracle: `switch_outcome_after_abandon_and_loss(b, cursor, loss, sent)` and its self-check cells in `o1_oracle.rs`.
- *Existing tests changed (consequence of the rule).* `c2_s5` (`ShortSend(5)` cell), `c2_s8` (`ShortSend(5)`) and `c2_s10` (a whole boundary send, an underflow, the owner's abandon) said `applied` at the abandon at device time 8 ms; they now say `pending` there and `applied` once device time reaches b (`run(.., 30)`) or at the Manifest write; `StalledSend` and `FailSend` cells are unchanged (nothing waits). `c2_s5`'s and `c2_s10`'s comments say so. The Stop-then-abandon history is pinned at record level only: a Stop that holds the row also cancels the unissued remainder, so no abandon can follow it on the rig.

**Item 2: F8 on a row a Stop is holding.** No code change (`complete_reopen`'s `Failed` branch cancels the pending switch whatever its halt). Pinned: `c2_txbook_a_failed_reopen_cancels_a_switch_a_stop_is_holding` (record, with and without the Stop) and `c2_s15_…` now loops over both histories on the rig (`Stop {stopped}`: pending before the reopen, `cancelled` after). RM-37 and UR-38 say it.

**Item 3: `settle_unresolved` applies through `apply()`.** Its Sent branch calls `apply(switch, halted.stop, Applied)`; the in-flight branch is the push of `Uncertain` as before. Test: `c2_record_a_stop_before_device_time_reaches_b_leaves_the_switch_to_a_later_loss` gained `assert_eq!(manifest, by_device_time)`: the record after the Manifest write equals the record after `advance(b)` (it kept one chain at front 150 before; now `[0, 100)` and `[100, 150)`).

**Item 4: text.** RM-37 and UR-38: a Stop or loss in flight leaves the switch pending until its result, which then settles it, except that a whole result under a Stop that came before device time reached `b` keeps it pending (also in the Changes rows); RM-37's "(the Stop leaves the switch pending until then)" is "UHD leaves …; MockRadio settles it at the Stop, MR-42"; the abandon's halt at device time and the F8 case are said in both. MR-42 (own paragraph line, for `rules.py`'s 1 500-character limit): the Mock settles at the Stop by its payload cut, so its row never waits. Test lines name the three new Rust tests and `c2_s26`.

**Item 5: recorded, no change.** The tie at exactly device time `b` (a Stop or `advance` at sample `b`, then a loss at sample `b`: `applied`; loss first: `cancelled`; from `advance`'s `b <= now`, predates C2); C2-32 and C2-38 near-duplicates kept; the detached-join path has no test.

**Rows.** New: C2-39 (the abandon halts at its cursor), C2-40 (uhd-tx passes the cursor as device time), C2-41 (the abandon leaves `playing` when its switch waits), C2-42 (the Manifest write applies a held switch without `apply()`), C2-43 (a failed reopen leaves a held switch pending). Retargeted to the new code, same mutants: E1-U13 and E2A2B-03 (the abandon's new argument and the waiting clause), C2-26, C2-35, C2-36. The run is fix2-ids.txt plus these and every row whose code or filter mentions an abandon, `halt_played`, `settle_unresolved` or `c2_s15` (92 rows, one run).

**Round 3 mutation outcome (`fix3-mut.log`).** 92/92 rows killed (round 2's fix2-ids.txt rows, C2-39 to C2-43, and the abandon-related rows); no survivor, no ERROR; nothing missing. C2-27 ran on a real clock with 0 reruns.
