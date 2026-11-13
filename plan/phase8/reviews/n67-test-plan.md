# #67 test plan: a cut at or before a queued start's origin stops the stream at once

Branch `work/n67` (from `79f55e1`). Written by the implementer (OpenCode Sonnet 5.5 #high). No tree edits. One probe in a copy outside the tree (`/tmp/n67-probe`, shared target dir, `CARGO_INCREMENTAL=0`), deleted. Nothing offloaded to DeepSeek.

## 0. What the probe showed (VERIFIED by running it)

In the copy I (A) removed `.rx_join_issue_67()` from the six places that carry it and ran the five exempted tests: **5/5 fail** with `rejected [("uhd-rx-s0", "did not join within 1 s; left detached")]` (red before the fix). (B) I then deleted one conjunct, `&& self.core.now() >= stream.clock.origin`, from `follow`'s `done` (`provider/rx.rs:183`) and ran `cargo test -p ezsdr-radio-uhd --lib` and `--test fake`: `fake` 546/546 pass, `lib` 646 pass and **exactly 3 fail**, which are the tests that pin the wait:

| failing test | why |
|---|---|
| `ur_26_a_stop_before_a_queued_start_stops_after_its_origin` (`rx.rs:2214`) | asserts the second stop is at or after the origin |
| `ur_26_an_abort_stops_a_queued_start_at_shutdown` (`rx.rs:1888`) | asserts "only the first stream's stop yet" after `rx.poll()` |
| `rm_26_the_timeline_against_uhd_rx` (`rx/differential.rs`) | 12 of 1 000 seeds: the compare flags `rx_stop … before the origin (VH-6)` (`differential.rs:346`) and `an rx_stop row issued … before its cut` (`:365`); and the scripted device's queued start survives the stop (`:724`), so the cancelled start's data shows up (seed 36: "segment 1: 61 blocks, the plan's 62", "the device streams at the end: true") |

So the production change is one conjunct. Everything else below is tests, the two device models, the one place that can still close a queued start, and text.

## 1. Decisions (all "decided by the orchestrator" unless marked owner)

**D1. The rule is one, for `Provider::stop` and for an ordinary `Stop`.** A segment whose plan cut is at or before its first undelivered sample (cut index 0 while nothing is delivered, or an abort) is stopped untimed and ended at once, whether or not its origin has passed. Reasons:
- uhd-rx cannot tell the two apart. Both reach it only as a plan cut through `refresh_intent` (`rx.rs:688`). Making `Provider::stop` special needs a new flag in `RxIntent`, a second rule, and keeps the VH-6 wait with its tests for the ordinary `Stop`.
- VH-6's wait was written "when the hardware answer to #56 was unknown" (spec 22 §VH-6). The bench (2026-11-13, `hw_67_stop_before_origin`, 30/30 per board) refutes the premise.
- RM-16 ("never before its origin") is about the *cut instant* of the segment, so the clock/segment record is unchanged. RM-25's start floor (next origin = old origin + ready term) is a plan rule and stays: after `Stop`, `start_rx` the new origin is still at least the old origin + ready, even though the device is told earlier. EA-16 ("`Stop` first, then `start_rx`") is unchanged.
- It also deletes `Abort`'s special case in effect: abort already stops a queued start at the shutdown (`command`, `rx.rs:253-262`); now it stops when uhd-rx reads the plan.

Rejected: (a) `Provider::stop` only (above). (b) Keep the wait and detach less (longer join, or a wake): the thread and the leak stay. (c) Close or reopen without a stop: refuted, 30/30 FAIL. (d) Leave the streamers open at cleanup: today's leak.

**D2. Nothing changes in MockRadio or `ezsdr_radio::timeline`.** The Mock has no device queue; its segment cut at its origin delivers nothing and registers no clock (`rm_26_the_timeline_cases`, "a Stop before a restart begins leaves it out", `crates/ezsdr-radio/tests/timeline.rs`; `mr_29_*`). The user-visible result of this case (no block, no clock, plan row, next origin floored) is identical on both Providers before and after. What changes is when UHD tells the device, which RM text never states. The shared rule stays one (RM-16/RM-25 plan); layer 1 and 2 of the differential are untouched.

**D3 (needs the orchestrator; I recommend yes). Cleanup stops an `Uncertain` receive chain before `close_streams`.** The brief says never close a streamer that holds a queued start. Today that is held by the chain: `assert_rx_chain_reusable` panics on `Armed`/`Streaming` at close in test builds (`core.rs:1571`). `Uncertain` is allowed, and two real paths reach cleanup with a queued start in it: a failed owner spawn (`mod.rs:652` `abandon_unowned`, pinned by `ur_15_a_failed_rx_owner_spawn_abandons_its_started_chain_before_cleanup`, `mod.rs:1041`, which asserts "no RX call follows the orphaned start") and a non-loss `rx_stop` failure (`ur_24_nonloss_rx_stop_failure_leaves_the_chain_uncertain`). Both close a streamer whose start may be queued, which the bench says does not cancel it. Fix: in `cleanup`, for each receive stream whose chain is `Uncertain` and not lost, with no thread detached, one best-effort untimed `rx_stop` (a failure goes to `rejected`) before `close_streams`; `stop_result` currently asserts `Armed|Streaming`, so the implementer either widens it to `Uncertain` or calls the device directly. If the orchestrator declines, the `mod.rs:1041` test stays as is, the gap is one line in the dispositions, and an owner decision on a KNOWN_LIMITATIONS entry is needed (not listed there today: `grep -n "detach\|join\|distant\|#67" KNOWN_LIMITATIONS.md` finds nothing for #67).

**D4 (note for the owner, low).** The `applied` row `{"key":"rx_stop","at": cut,"issued": now}` and the `timing` row `rx_stop_untimed` can now have `issued`/`at` earlier than the cut (a cut at an origin ahead). The fields mean what they meant. No document I found says `issued ≥ at` (UR-17 says "the cut and the instant it was issued"). If the owner counts the Manifest reading as user-visible, it is a one-line owner call.

## 2. Hardware evidence and what is INFERRED

VERIFIED (bench 2026-11-13, both boards, 1/10/100 MS/s × 10 trials, receive only, 1 GHz, origin ~2 s ahead, one streamer): an untimed `rx_stop` before the origin cancels the queued start (no block of it ever arrives; a fresh timed start 100 ms ahead delivers its first block at its own origin); `close_streams` + `rx_open` does not (the old start's blocks arrive on the reopened streamer at the old origin). UHD 4.10 source (Sol): STOP bypasses and clears the command FIFO.

INFERRED, and said so in the text: (1) the same for a restart start (`rx_restart` after a missed start; same command); (2) stop, `start_rx`, stop in quick succession while the previous stream's tail is in flight (the tail is not in the command FIFO); (3) a close/reopen that spans the origin, or a changed rate; (4) the fake's orphan keyed by channel (the bench reopened the same channel); (5) a stop that fails (non-loss) leaves the start queued. Join margin: uhd-rx reads the plan at its next turn, at most one 100 ms `rx_recv` timeout (`RECV_TIMEOUT`) plus the stop call (0.2–3 ms measured) after the cut is booked, against the 1 s join (`mod.rs` `JOIN_BOUND`); the bench's Stop lag max was 3 ms.

## 3. Production change the tests are written against (for the implementer; not made here)

1. `rx.rs:183`: drop `&& self.core.now() >= stream.clock.origin` from `done` (J45's anchor is a prefix of the new text and still matches once). Update the comments of the three tests above and the `ur_26_a_receive_start_queued_by_the_stop…` comment.
2. `device.rs` FakeDevice: orphaned queued start (section 5).
3. `differential.rs`: scripted device and `compare` (section 6).
4. `tests/common/run.rs`: delete `Declared::rx_join_issue_67`, its builder `rx_join_issue_67()`, the filter in `judge_end` (`:68`), the three `run_judge_end_*issue_67*` tests (`:411`, `:423`, `:459` keeps the rest of its case without `rx_join_issue_67`), `NONE`'s field; remove all six uses (`fake.rs:1687, 4917, 5000, 5017, 6853`, `common/mod.rs:493` and its two comment lines); `tests/README.md:113-114`.
5. D3 if adopted.

## 4. Sequences, tests, observations (E = expected; "red" = fails before the production change)

Unit tests use `test_support::rig_with_links` (FakeDevice on the manual clock, `time.advance_to`, `plan()`, `rx.poll()`/`rx.turn()`), as the three tests above do.

| # | Sequence | Test (new unless named) | Asserts and how observed | Wrong behaviour it fails |
|---|---|---|---|---|
| U1 | A segment's `rx_start` is queued for `origin`; a plan without it (ordinary `Stop`) or with its cut at the origin (`Provider::stop` End, orderly and abort) is read at `now`; table of `origin − now` = 300 ms, 900 ms, 1.1 s (both sides of the 1 s join bound), 5 s | `ur_26_a_cut_at_a_queued_origin_stops_the_stream_at_once` (unit; replaces `…_stops_after_its_origin`'s pre-origin half) | after one `rx.poll()`: `rx.stream` is `None`; the device log has exactly one `rx_stop 0 now` after `rx_start 0 <origin>`; the `rx_stop_untimed` timing row has `cut == origin` and `at < origin`; `chain.device() == Off`; link empty; no sample clock registered for it (`core.clocks.sample_clock_records()`); then time to origin + 500 ms and `device.rx_recv` returns `Timeout` only (the fake cancelled; observes the device, not the bookkeeping). red | waiting for the origin (R1); a timed stop; a second stop; data from the cancelled start reaching the link |
| U2 | Same, the origin has already passed and no block came (silent, and first-samples cases) | `ur_26_a_stop_before_a_queued_start_stops_after_its_origin` kept for these cases and renamed `ur_26_a_cut_after_the_origin_of_a_silent_start_stops_at_once` | the plan is dropped at 80 ms with origin 65 ms: stop `at ≥ origin`; `received(&link).len() == 1`; one clock; blocks arriving after the stop are not published (today's assertions) | the origin-passed path changing |
| U3 | A cut after samples are pending (index > 0) is still not stopped early | existing `ur_17_a_cut_is_never_before_what_was_delivered`, `ur_26_a_stopped_stream_keeps_its_end`, `ur_17_a_finite_segment_delivers_through_its_cut` unchanged; add one row to U1's table: cut index 5 000, origin ahead, stream not yet streaming: **no** stop at the poll (the cut is after the origin) | a stop issued for a segment the plan keeps | the conjunct removed too widely (e.g. `done` true whenever a cut exists) |
| U4 | `Abort` while a start is queued (the old G60 scenario) | `ur_26_an_abort_stops_a_queued_start_when_uhd_rx_reads_the_plan` (rename of `…_at_shutdown`) | after `rx.poll()` the stop count is 2 (first stream's, the queued one's); `rx.command(Shutdown(Abort))` adds none; clocks unchanged; no clock for the queued start | a second stop at shutdown; no stop |
| U5 | `Stop` (plan drops the queued segment) and `Provider::stop` End booked in the same turn; then `Shutdown(mode)` for orderly and abort | `ur_26_a_stop_and_provider_stop_in_one_turn_stop_a_queued_start_once` | exactly one `rx_stop` for the queued start (log), one `rx_stop_untimed` row and one `applied` row for it, chain `Off`, one plan | a double stop after `Off`; a stop lost when both fire |
| U6 | The owner's `begin` returns after `Provider::stop` booked (S16a), origin **ahead** (new variant) and passed (existing) | `ur_26_a_receive_start_queued_by_the_stop_ends_its_owner_without_detaching` run as a two-row table | ahead: begin returns within the join bound, `rx_start` issued, `rx_stop_untimed` `at < origin`, link empty; passed: the existing `(cut, at) == (origin, origin)`; then `Shutdown` and `turn() == Exit` | `begin` not calling `follow` (P479) |
| U7 | `Stop` cancels a queued B (stream A before it), then `start_rx` C | `ur_26_a_start_after_a_cancelled_queued_start_delivers_at_its_own_origin` (drives `rx.turn()` and the fake's `rx_recv`, manual time) | the stop of B before B's origin; C's `rx_start` at C's origin (≥ B's origin + ready, RM-25); the first block on C's clock has `first_sample_time` index 0 and its first tick is C's origin; two clocks (A, C), none for B; nothing from B on C | stale B data on C; C's first block off its origin; B left queued |
| U8 | A loss published while B is queued, then the cut is read | `ur_29_a_loss_found_before_the_stop_of_a_queued_start_makes_no_stop_call` | no `rx_stop` after the loss in the log; chain `Lost`; stream `None`; no clock; `finish` clean | a device call after the loss |
| U9 | The loss comes from uhd-rx's own stop (`FakeFault::Lost(at)` between the plan read and the call) | `ur_29_a_stop_of_a_queued_start_that_finds_the_loss_publishes_it_once` | one `DEVICE_LOST`; chain `Lost`; stream `None`; `device_failed` once | a stop error swallowed or published twice |

Provider level, `tests/fake.rs` with `Direct` (real threads, manual clock) and `Run`:

| # | Sequence | Test | Asserts and how observed | Wrong behaviour |
|---|---|---|---|---|
| P1 | The five tests that end a Run before T0 (T0 2 s ahead) | `ur_13_arm_cases`, `ur_13_arm_compares_the_rate_set_after_the_streamer_opens`, `ur_13_arm_takes_the_rate_set_in_any_order`, `ur_13_a_transmit_stream_is_compared_with_the_whole_device_grid`, `the_candidate_ceiling_admits_what_the_profiles_link_limit_refuses` and the `ur_15`/`measurement` creation at `fake.rs:6853`, unexempted | `judge_end` finds no `join` bound (no `did not join` row). Red: verified in the probe (5/5) | the thread outliving its join |
| P2 | `Provider::stop` with the start queued 5 s ahead, orderly and abort | `ur_26_provider_stop_before_a_distant_t0_stops_before_it_closes` (`Direct`, profile `start_lead_ns` 5 s) | joins; no `rejected` join row and no `leaked`; the fake's log has `rx_start 0 T0` < `rx_stop 0 now` < `close_streams` (UR-16 satisfied, the order is the new rule); no sample from the start reaches the link; after `cleanup`, `rx_open`, `rx_recv` through T0 + 500 ms: only `Timeout` (the bench's close-evidence boundary: a close without the stop would deliver). Red (join) | detach and leak; `close_streams` before the stop; a stop that does not cancel |
| P3 | Origin ahead < the join bound (300 ms) and origin passed, via `Direct` advanced to T0 − 300 ms and past T0 | same file, two more rows | stop `at < T0` in the first, `≥ T0` in the second; both join | the cliff at 1 s |
| P4 | Session mirror of `hw_56_stop_between_starts`: streaming, `Stop`, `start_rx`, a second `Stop` 10 ms later, a third `start_rx` | `ur_25_a_stop_between_start_rx_and_its_origin_leaves_no_trace` | the cancelled segment's `rx_stop` precedes its `rx_segment` origin (timing rows); no clock and no block for it; the third segment's first block at its origin | the old wait; stale data |
| P5 | Loss while the start is queued 5 s ahead, then `Provider::stop` | `ur_29_a_loss_while_the_start_is_queued_makes_stop_close_nothing` (`FakeFault::Lost`) | one `DEVICE_LOST`; no `rx_stop` after the loss; no `close_streams` (UR-29); join fine | a call on a lost device |

If D3 is adopted (mod.rs unit tests):

| # | Test | Asserts | Wrong |
|---|---|---|---|
| C1 | `ur_15_a_failed_rx_owner_spawn_abandons_its_started_chain_before_cleanup` updated | after `cleanup`: `rx_start 0 0` < `rx_stop 0 now` < `close_streams`, `close_streams` last; chain stays `Uncertain` or goes `Off` (the implementer states which); after reopen the fake delivers nothing at the old origin | the close of a queued start |
| C2 | `ur_16_cleanup_stops_an_uncertain_receive_chain_once_and_a_failing_stop_does_not_block_the_close` | a failing cleanup stop is in `rejected`, `close_streams` still runs; a `Lost` chain gets no stop | a retry loop; a call on a lost device |

Kept as they are (they pin things the change must not move): `ur_26_no_segment_begins_once_provider_stop_has_begun`, `ur_29_*`, `ur_24_*`, `rm_26_the_timeline_cases`.

## 5. FakeDevice must model the hardware

Today (`device.rs`): `rx_stop` at `:1500` sets `streamer.then = None` and, when `streamer.next` is not yet reached (`next >= now`), clears `next` and `end` (`:1517-1523`). So **an untimed stop before the origin already cancels a queued start**, as the bench shows. `rx_close` (`:2001`), `close_streams` (`:2015`), a replacing `rx_open` (`:1462`, `st.rx.remove`) and `set_subdev_spec` (`:1262`) drop the whole `RxStreamer` with its `next`, so **a close/reopen cancels it: the fake disagrees with the hardware (30/30 FAIL per board)**.

Change: when a streamer is removed by `rx_close`, `close_streams` or a replacing `rx_open` while `next = Some(at)` with `at >= now`, keep `Orphan { channels, at, ratio }` in `Fake`. A later `rx_open` whose channel list overlaps an orphan's channels adopts it (`next = Some(at)`, `origin = at`, `ratio`) and removes it; a later `rx_stop` on that streamer clears it as for any queued start. An orphan is dropped when its origin passes with no streamer open, when `set_time_zero` runs (it already clears `next`), and by `set_subdev_spec` (INFERRED: UHD disconnects the chains). Not modelled and marked INFERRED in UR-33: a close/reopen that spans the origin, and a changed rate.

Device unit tests (`device.rs`, the `manual` + `participant` pattern of `ur_33_a_stopped_stream_s_tail…`):
- D1 `ur_33_an_untimed_stop_before_a_queued_origin_cancels_it`: start 2 s ahead, stop, read to origin + 500 ms: no block; fresh start 100 ms ahead: first block at its origin. (Passes today; pins the fake to the bench.)
- D2 `ur_33_a_close_and_reopen_does_not_cancel_a_queued_start`: variants `close_streams`, `rx_close`, replacing `rx_open`: reading the reopened streamer, the first block's tick is the old origin. Red today.
- D3 stop before the close: no block after the reopen; stop on the reopened streamer: no block.
- D4 reopen on other channels: nothing inherited. D5 reopen after the origin and after `set_time_zero`: nothing (INFERRED limits pinned).

## 6. The scripted device and the compare in the differential (`rx/differential.rs`)

This is a third copy of the device's start/stop rule (FakeDevice, this scripted device, the hardware): a rule implemented twice. It is written apart on purpose, so both change by hand and each is pinned by its own test: the FakeDevice by D1/D2 and the rehearsal, the scripted device by `rm_26` itself (the probe shows 12 seeds fail if it does not cancel).
- Scripted `rx_stop` (`:724`): also drop every run with `origin > now` and no `end`. Doc comment at `:8` and `:722` ("a start still queued is not cancelled (#56)") rewritten.
- `compare`: delete the `before the origin (VH-6)` problem (`:346`); replace with: an `rx_stop` before its start's origin is legal only when the plan has that segment cut at sample 0 (or dropped) **and** not before the booking of the item that cut it; for a segment the plan keeps with samples, a stop before its origin is a problem. Relax the `issued < at` check (`:365`) the same way (a segment with no samples). The `dropped` coverage counter and its floor (`:124`, `:176`, `:991`) stay; only the "(VH-6)" label goes.
- Check the layer's coverage line after the change: "stop before a queued start" (317 seeds today) stays > 0.

## 7. Rehearsal that books the hardware's boundary (AGENTS §9)

- Move `n67_trial` (`hardware.rs:454`) to `tests/common/iii.rs` as `cancel_trial(device, cancel, rate)` (precedent: `restart_fill_probe`), with the constants it already uses (origin 2 s, observation to origin + 500 ms, `OBSERVE_TICKS`), and add `expected_cancel(cancel)` in `iii.rs`: `Control` PASS, `StopBefore` PASS, `CloseBefore` FAIL with "block(s) after the cancel".
- `hw_67_stop_before_origin` asserts per cell against `expected_cancel` (the present `assert_eq!(failed, 0)` would now fail every run on a close FAIL that is the finding). INCONCLUSIVE stays counted, not failed.
- New default-suite test `iii_n67_cancel_probe_rehearses_on_the_fake` (`fake.rs`, next to `iii_restart_fill_probe_rehearses_on_the_fake`): the same function on a manual-clock FakeDevice at 1/10/100 MS/s, one trial per cell, asserting the same table. Manual clock makes the 2.5 s per trial free; if `read_with` does not run on a manual clock, the fallback is a real clock with the ten-trial count reduced to one and a note in the test (about 25 s of wall for nine trials), which is the larger cost and not preferred. Red today: the fake's `CloseBefore` PASSes. Pure verdict tests (`iii.rs:1235+`) are unchanged.

## 8. Text to change

- `design/18-uhd-radio.md` UR-17, second paragraph: replace "When a cut cancels a segment whose timed start is already issued, it stops the stream untimed only after that origin has passed, discarding what arrives; a cut at or before the first sample not yet delivered, once the origin has passed, stops and ends the stream at once." with a rule that has no rationale: a cut at or before the first sample not yet delivered stops the stream untimed and ends it at once, whether or not the origin has passed; the stop cancels a queued start; a streamer holding a start is not closed before that stop (and, D3, cleanup stops an `Uncertain` chain first). Its `*Test:*` line (`:234`) gets U1, U4, P2.
- UR-33 `receive` bullet (`:374`): an `rx_stop` before a queued start's origin cancels it; a close or reopen does not (the old start's blocks arrive on the reopened streamer at the origin); VERIFIED 2026-11-13, INFERRED across an origin or a rate change. Its test line gets D1–D5 and the rehearsal.
- UR-16 (`:220`/`:322`): D3's sentence only.
- Test table rows: `:552` (`…stops_after_its_origin`), `:564` (`…_at_shutdown`), `:570` (`rm_26`: "its queued start surviving an untimed stop (#56)" → "cancelled by it"; "untimed stops … at or after its origin unless an abort stopped it" → the new rule), plus rows for U1, U5–U9, P2–P5, D1–D5, C1–C2; the *Changes* table gets a dated row.
- `plan/maintenance/22-timing-simplification.md` §VH-6 (decision text `:167`, table `:23`, `:378`) is a decision record: add one "Superseded in part (2026-11-xx, #67)" sentence under VH-6's decision and in the table row; do not rewrite history. `rules.py --base 79f55e1` must pass.
- `plan/maintenance/failure-mechanisms.md:41`: "waiting on #56's bench" → fixed in this step, with the outcome (the wait existed for an unknown hardware answer; one rule, one conjunct removed); no new count unless a review finds a bug of the mechanism.
- `handoff.md` §3 row #67: remove; the closing line in the current-order list; `plan/phase8/00-overview.md` record.
- `crates/ezsdr-radio-uhd/tests/README.md:113-114`: the `rx_join_issue_67()` sentence goes.
- KNOWN_LIMITATIONS: no entry for #67 exists, none added (unless D3 is declined; then the owner decides).
- Not touched: design/07 (RM-16, RM-21, RM-25), design/09, `design/` Vision; the phase 8 notes 35/44/45/46/51 that cite #67 are dated records.

## 9. Mutation rows (Rust token anchors; each must match once through `anchor.py`; run only through `remote_mutate.py`)

New:
| id | file | mutation | killed by |
|---|---|---|---|
| N67-01 | `rx.rs` | `done` waits for the origin again: `… <= stream.expected)` → `… <= stream.expected) && self.core.now() >= stream.clock.origin` | U1 (pure unit test, not the bound) |
| N67-02 | `rx.rs` | `done` true whenever a cut exists: `chain.cut().is_some()` | the U3 row (cut after the origin, not stopped) |
| N67-03 | `device.rs` | `rx_stop` leaves a queued start: the `_ => { streamer.next = None; streamer.end = None; }` arm emptied | D1, rehearsal (`StopBefore` cell) |
| N67-04 | `device.rs` | a close keeps no orphan (no stash in `rx_close`/`close_streams`/`rx_open`) | D2, rehearsal (`CloseBefore` cell) |
| N67-05 | `device.rs` | an orphan adopted by a non-overlapping channel set | D4 |
| N67-06 | `device.rs` | an adopted orphan survives an `rx_stop` | D3 |
| N67-07 | `device.rs` | an orphan survives `set_time_zero` / its origin | D5 |
| N67-08 | `differential.rs` | the scripted `rx_stop` leaves a queued run | `rm_26` (12 seeds fail in the probe) |
| N67-09 | `differential.rs` | the compare allows an early stop for a segment the plan keeps | a table row for it in the compare's own tests or `rm_26` seeds; if no seed produces it, add a pure test of the compare's predicate |
| N67-10 | `rx.rs` | `try_end` records the stop row twice / stops a chain already `Off` | U5 |
| N67-11 | `mod.rs` (D3) | cleanup skips the stop of an `Uncertain` chain; or stops a `Lost` one | C1, C2 |
| N67-12 | `iii.rs` | `expected_cancel(CloseBefore)` = PASS | a pure test of the table (add one) |

Existing rows to touch: **G60** (`filter ur_26_an_abort_stops_a_queued_start_at_shutdown`, mutates the abort branch in `command`): the test is renamed (U4) and the queued case is now ended by `follow` inside `command`, so G60 must be re-run and retargeted to a test that has a segment being delivered when the abort comes (`ur_26_a_stopped_stream_keeps_its_end` has the abort-with-300 case); if none kills it, add that assertion to U4's table. **J45** (`done` expression) and **P479** (`follow()` after begin) keep their anchors; re-run both. **N51-2A-02, -03, -04** are deleted with the declaration (their `old` text no longer exists; `-03`'s filter test `run_judge_end_a_join_is_a_bound_unless_it_is_the_subject` stays and keeps killing other rows). **P880** (`"kill": "bound"`, kill came only from #67's teardown): re-run; if it is now killed by a plain assertion, drop the `"kill"` key. Every row whose `old` is in a file the step touches goes through `rules.py` after `cargo fmt --all`.

## 10. Hardware audit table (AGENTS §9: tests whose expectation derives from the rule)

| test | expectation today | after | verdict |
|---|---|---|---|
| `hw_67_stop_before_origin` (`hardware.rs:520`) | `assert_eq!(failed, 0)`: every cell PASS | per-cell `expected_cancel`: control and stop PASS, close FAIL (the finding); same table in the rehearsal | **fixed** |
| `hw_56_stop_between_starts` (`:2087`) | records only; its `device_call_order.ordering` label compares the stop with the restart `rx_start` at e₂ | still records only. The label's meaning shifts: a second Stop that lands before the owner's `rx_start` now gives no restart at e₂ (`restart_rx_start_not_observed`), one after it gives `rx_stop_before_restart_rx_start` (the stop now precedes e₂) | still right (no assertion); a note in the README of the bench data |
| `hw_56_stop_lag` (`:2182`) | records the lag of a stop on a streaming stream | not affected | still right |
| `hw_ur17_receive_call_duration` (`:2260`), `hw_n47_rx_restart_with_new_values`, `hw_b9_*`, `hw_iii_load_200msps`, `hw_h5_held_last_sample`, `hw_n47s5_*`, `hw_txp_*`, `hw_s1_*`, `hw_b8_*` | Session tests that stop or finish a Run; read `rx_segment`/`rx_stop_untimed` rows at most for recording; none asserts a stop at or after an origin (`grep -n "rx_stop_untimed" tests/` finds only `:2170` and `:2343`, both recorded) | a Run that ends with a queued start now joins and closes its streamers (before: leaked) | still right; the implementer greps each for `rx_stop_untimed`/`ordering` before reporting and lists any that read it |
| raw tests (`hw_d9_*`, `hw_b8_raw_*`, bench1115, `common/d9.rs`) | call the device directly, no uhd-rx | not affected | still right |
| `hw_measurements_record` | records; its profile comparison not affected | not affected | still right |

Run on hardware only after the owner's attenuator check where a test transmits; `hw_67` is receive only.

## 11. Open questions for the orchestrator

1. **D3**: cleanup stops an `Uncertain` chain before closing. Recommend **yes** (about ten lines and two tests; it is the only remaining path that closes a queued start). If no: record the gap and ask the owner whether a KNOWN_LIMITATIONS entry is wanted.
2. **D1** is a judgment, not a measurement: the ordinary `Stop` shares the rule. Recommend as written; nothing user-visible changes except when the device is told.
3. **D4**: whether the owner counts `issued < at` in the `applied`/`timing` rows as a Manifest reading. Recommend: no.
4. Sol should look hardest at: U7 and P4 (the `Stop`, `start_rx`, `Stop` chain is INFERRED on hardware), the orphan model's channel keying, and the differential's early-stop predicate (U1's "not before the booking of the cutting item").
5. A risk for the fix round: the probe ran `lib` and `fake` only. The Python suites, `ezsdr-acceptance` (it includes `run.rs` by `#[path]`) and the full mutation lists are the gate's.

## 12. Changes after Sol's plan review (orchestrator decisions, plan-decisions.md; these bind over sections 1–11)

- **D3 is withdrawn; stop on open replaces it.** Cleanup adds no stop. Every receive streamer the Module opens gets one untimed `rx_stop` before its first `rx_start` (the one open path the implementer finds: arm/from-none, uhd-rx reopen, any other `rx_open`). A start orphaned by any earlier close (failed stop, failed owner spawn, cleanup, killed process) is cancelled before the streamer is used. `lost: true` paths stay as UR-29 (publish the loss, no further calls).
  - Tests (sub-step B): (SO1) open after an orphaned start, frozen clock, origin several seconds ahead: device log `rx_open` < `rx_stop` < next `rx_start`, silence through old origin + 500 ms; (SO2) a stop-on-open failing non-loss: a `rejected` row, the open is not used for a start (the existing rx_stop-failure path, UR-24; the implementer states which); (SO3) `lost: true`: loss published once, no further calls. C1/C2 are replaced by SO1–SO3; `ur_15_a_failed_rx_owner_spawn…` keeps its assertions except that its fixture is a future start (frozen clock, origin seconds ahead), and the reopen then observes silence through that origin.
  - Hardware: `hw_67_stop_before_origin` gains a cell `CloseThenStop` (start queued, close, reopen, untimed stop, observe silence through the old origin + 500 ms; expected PASS, INFERRED until run on 11-15). `expected_cancel`: Control PASS, StopBefore PASS, CloseBefore FAIL, CloseThenStop PASS. The fake models it (an adopted orphan is cleared by `rx_stop`).
- **FakeDevice orphans are per channel** (D1–D5). A reopen adopts only the pending starts of its own channels; the rest stay. Keyed by channel, independent of stream number and channel order (INFERRED; the bench reopened one channel). New device test: start on [0,1], close, open [0] adopts channel 0 only, open [1] still gets its start.
- **Rehearsal timing.** The manual clock does not make 2.5 s free (M5 advances no faster than real time, design/18 :378). The rehearsal budgets real time and takes part in the clock explicitly; no real-clock fallback with a timing-sensitive verdict.
- **Audit row `hw_56_stop_between_starts`.** Its `device_call_order.ordering` label compares host call timestamps (`hardware.rs:2138-2148`), not origins: an early second Stop can precede e₂ yet follow the owner's `rx_start` call. The earlier row's reading of the label is withdrawn; the test still records only.

## 13. Sub-step B, as built (implementer)

- **Stop on open — the one path.** Every receive open goes through `Core::open_device_locked(Dir::Rx, …)` (`open_streamer` for `arm` and the owner's `owner_configure`, `open_device` for the tests); the stop is made there, under the device gate, right after a successful `rx_open`: `rx_open(k).and_then(|()| rx_stop(k))`, an error prefixed `UR-16: the untimed stop on open:`. A failure is the open's failure, so it takes the existing open-refusal paths: at `arm` `rejected("UR-13: UR-16: …")` (`mod.rs` `open_refusal`); at the owner `refuse_segment(…)` → `device_failed(…, "command", …)` → a `radio.COMMAND_REJECTED` (UR-24/UR-25), the segment refused, no `rx_start`; `record_opened` books the streamer as not open (`DeviceSet::Unknown`) and the chain stays `Off`. `lost: true` keeps its flag, so the loss is published by the same caller (UR-29) and nothing but `mark_lost` follows. The streamer is not closed after a failed stop (a close would orphan the start again); the next open stops it again.
- Tests: SO1 `ur_16_an_open_stops_the_streamer_before_its_first_start_and_cancels_an_orphaned_start` (rx.rs; frozen clock, origin 5 s ahead; log order, silence through origin + 550 ms) and the updated `ur_15_a_failed_rx_owner_spawn…` (a start 5 s ahead through `radio.start(Some(..))`, cleanup, a reopen: `rx_open`, `rx_stop` directly after `close_streams`, silence through the origin); SO2 `ur_16_a_failed_stop_on_open_refuses_the_segment_and_starts_nothing`; SO3 `ur_29_a_loss_found_by_the_stop_on_open_is_published_once_and_nothing_follows`.
- **FakeDevice orphans**: `Fake::orphans: channel → (origin, ratio)`, stashed by `rx_close`, `close_streams` and a replacing `rx_open` for a streamer whose `next == origin >= now` (not yet delivering); adopted per own channel by `rx_open` (the earliest origin if several), dropped by a reopen after the origin, `set_time_zero`, a receive `set_subdev_spec`. Tests D1–D5 plus D6 (`ur_33_a_close_of_a_stream_that_has_begun_leaves_no_start_for_a_reopen`) and the per-channel case, and D3 extended: after a stop on the reopened streamer a further close/reopen finds no start.
- **Rehearsal**: `iii::cancel_trial` (moved from `hardware.rs`), `Cancel::CloseThenStop`, `iii::expected_cancel`/`cancel_unexpected` (pure test), `iii::driven_fake` (manual clock, the test thread a participant, real-speed waits: about 30 s); `iii_n67_cancel_probe_rehearses_on_the_fake` asserts each cell exactly (no INCONCLUSIVE). `hw_67` runs four cells × 3 rates × 10 trials, bound 20 min, and fails only on a verdict that contradicts `expected_cancel`.
- **Audit correction** (replaces §10's `hw_56_stop_between_starts` row): its `ordering` label compares host call timestamps, not origins (`hardware.rs` `stop_rx_call`: the first `rx_stop` call at or after the stop submit); an early second `Stop` can precede e₂ yet follow the owner's `rx_start` call. The test only records; unchanged. The stop on open adds an `rx_stop` call before every Session's first start; `hw_56_*` take their baselines after the open (`probe.count("rx_stop")` before the Stop), so they are unaffected.
- **Existing Provider tests that count `rx_stop`**: the open's stop is setup; `common::calls()` leaves it out (as `tx_experiment_sends` does for T3's four-zero send); a test of the open reads `device.calls()`.
