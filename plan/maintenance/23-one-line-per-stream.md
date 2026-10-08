# Maintenance note 23 — One timeline per stream on the UHD Module

| Field | Value |
|---|---|
| Status | **Accepted** by the owner on 2026-10-08 as recommended (A), with the receive-clock registration on both Providers at the first published block (below): "簡素化しましょう". A design note under AGENTS.md §6's three-strikes rule, written by the orchestrator (Claude Opus) after spec 22 step 3. Implemented on 2026-10-08 (*Implementation record*, at the end), except UR-17's bounded receive request, which stays: removing it makes a start late (the record's first judgment call). |
| Mechanism | The refusal round trip between uhd-control and uhd-rx: `planned-state-read-as-actual` and `correlation-by-partial-key` ([failure-mechanisms.md](failure-mechanisms.md)). |
| Scope | As proposed: `crates/ezsdr-radio-uhd/src/provider/{control,rx,tx,mod}.rs` and VH-8's layer 3. As accepted, with the two *Also decided* sections and #63: also `ezsdr_radio::timeline` (pruning), MockRadio (receive-clock registration), and RM-25, RM-26, MR-11, UR-17, UR-23, UR-25, UR-26 and UR-30 in `design/`. |

## The bugs

Five bugs of one mechanism in two days, all in how a receive plan reaches uhd-rx. All were found by review or by layer 3, not by users:

1. Spec 22 step 2, review B1: after the device refused a segment's rate, uhd-rx began the next segment where the old plan had placed it, and the stream went silent.
2. Step 3, fix 2: the wait for a replanned segment ended on a plan made before uhd-control knew of the refusal (seed 167).
3. Step 3, fix 3: a refused segment counted as begun, so an earlier replanned segment was skipped (seed 317).
4. Step 3 review, B1: uhd-control's half of the fix-2 protocol was tested by nothing.
5. Step 3 fix round: a loss ended the stream at a later cut from a plan that predated the loss (seed 118).

Step 3's fix 4, where an orderly stop cut nothing until the plan arrived, is the same shape on `Provider::stop`.

## The cause

Each receive stream's planned state exists twice:
- uhd-control holds the `Line`, with its items and its plan;
- uhd-rx holds the last plan uhd-control sent it.

Messages keep the two in step (`RxCmd::Plan`, `RxCmd::Cut`, `streams.refused`). Sometimes uhd-rx learns of an event first: a configuration the device refuses, a loss, or `Provider::stop`. From then until uhd-control's next plan arrives, uhd-rx acts on a copy it knows is out of date.

Each fix closed one such window by adding one more correlation field:
- the refused list in `RxCmd::Plan`;
- the `known` set;
- the `refused` wait;
- `stopping`;
- `lost_at`;
- `refusals()` at `finish`.

The next kind of event will open the next window.

## The question

Can each stream have one copy of its planned state? MockRadio already works that way: one `Line` per direction, booked on receipt, read by the step loop.

## Recommendation: A — one shared `Line` per direction, booked by whichever thread learns the event

- **Shared state.** Each direction's `Line` sits behind a mutex that uhd-control, uhd-rx and uhd-tx share.
- **Who books what.**
  - uhd-control books the commands, as now.
  - uhd-rx books what only it learns: it marks a refused segment's item `refused`, and it books a loss it detects as `End`, with `Item.delivered` set to the first sample not yet delivered. The field exists for exactly this (VH-3).
  - `Provider::stop` books `End` directly.
- **How uhd-rx reads it.** At each turn, uhd-rx takes two things under the lock: the next segment and the current segment's cut, both from the one plan. It holds the lock only to book and to read, never across a device call.
- **What goes.**
  - `RxCmd::Plan`, `RxCmd::Cut` and their transmit counterpart;
  - the refused list, the `known` set and the refusal wait;
  - `streams.refused` and uhd-control's `refusals()` poll;
  - the loss and stop fallbacks in uhd-rx.

  The delivered floor stays with uhd-rx, which alone knows what it delivered. It reaches the plan as `Item.delivered`.
- **What it gives.**
  - A whole class of windows goes away: a plan can no longer be older than an event its reader knows of.
  - The UHD Module takes MockRadio's structure, which serves parity (`radio-rule-implemented-twice`).
  - Code is deleted. INFERRED: 150–250 lines across `rx.rs` and `control.rs`.
- **Costs and risks.**
  - One lock acquisition per uhd-rx turn. INFERRED to cost microseconds against a receive call of milliseconds; the bench measures it.
  - Every booking replans from the start of the Run until #63 prunes the items. The lock makes that cost visible on the receive thread, so #63 should land first or alongside.
  - Layer 3 changes from handing plans to booking items into the shared `Line`. Its oracle, the timeline, does not change, and the plan latency stays as a turn delay before uhd-rx reads.
  - The mutation rows that pin the message protocol retire and name their replacements: J18, J24, J27 and J28, and possibly J22's successors.

## Rejected

- **B. uhd-rx owns the receive `Line`; uhd-control sends items, not plans.** uhd-control needs the receive plan when it books: `coerce` over the projected configuration, `LATE_COMMAND`, timed updates on the channels in force at their instant (#61), and the recorded plan row. It would need a copy again, or a query round trip.
- **C. Version numbers on plans.** uhd-rx would act only on a plan whose version covers every event it reported. This is fix 2 generalised: it keeps two copies and the waiting, and each new kind of event needs its own handshake.
- **D. Keep the protocol, now that it is pinned.** That means five bugs of one mechanism, found only because layer 3 models a plan latency of 0–2 ms. Production has other latencies: a 100 ms receive call, a slow control call. The rule asks for this note precisely to stop that.

## If accepted

A decision record and its implementation would form one step, with:
- an Opus implementer and a separate Opus reviewer;
- layers 1–3 as the gate;
- the touched mutation rows during the fix rounds and the full lists once at the end (AGENTS.md §7).

#63, pruning `Line.items`, goes with it.

## Also decided: one receive-clock registration rule

MockRadio registers a receive clock once its segment's first sample instant has passed; uhd-rx registers it at its first block. They disagree when a loss or an abort comes before the first block. The owner chose one rule (2026-10-08): both register a receive clock at the first block they publish. RM-25 states it once, and the Phase 8 parity test needs no special case. It lands with A.

## Also decided: UR-17's bounded receive request goes

uhd-rx asks the device for no more than up to a pending cut (`recv_len`). Since VH-4's ready term budgets one whole receive call plus 3 ms after every cut, the bound moves no instant, and samples past a cut are discarded either way. The owner removed it (2026-10-08); it goes with A, which rewrites uhd-rx, with `recv_len`, its test `ur_17_a_request_ends_at_the_pending_cut` and its mutation row B05.

## Implementation record (2026-10-08)

Implemented by a Claude Opus subagent, uncommitted, at `86e289a`. One step: code, `design/` text and mutation rows.

**The structure.** Each stream's planned state is one `ezsdr_radio::timeline::Line`, in `Core.streams` (`provider/core.rs`, `Streams`), shared by uhd-control, uhd-rx and uhd-tx under one mutex. Whichever thread learns an event books it:

| event | booked by | how |
|---|---|---|
| `cold` change, `Stop`, `start_rx` | uhd-control | `Streams::book`, its instant taken under the lock |
| a configuration the device refuses | uhd-rx (`begin`), uhd-tx (`do_switch`) | `Streams::refuse`, which marks the item `refused` (`Line::refuse`) and replans |
| a device loss | the thread that finds it: `Core::device_lost` | `Streams::end(lost)`, before the lost flag is set, so no thread sees the device lost before the plan has the loss |
| `Provider::stop` | `Provider::stop` itself, first | `Streams::end(at, abort)`, then `streams.stop` for uhd-control |

A loss and `Provider::stop` carry `Item.delivered`: uhd-rx keeps its segment and the first sample of it not yet delivered in `Streams.delivered`, written under the lock together with the cut it reads for each block, so a booking after a block cuts after it. It is used only when `Line::made` shows that segment running at the item, as `Item`'s doc requires. uhd-rx reads the plan at each turn (`Rx::follow`, and in `samples` for each block) and when it ends a stream (`Rx::finish`, since review round 2); uhd-tx reads it again when `Streams.version` has changed. Every booking that changes the transmit plan registers and ends transmit clocks there (`Streams::reconcile`, moved from uhd-control's `hand`), so uhd-control still registers a transmit clock before it takes the next Action (KC-21a, RS-19).

**Lock order.** A segment configuration's `updates` (`ColdConfig`), then `streams`, then `rec`; the clock registry's and the event sink's own locks are taken inside `streams`. `streams` is never held across a device call: uhd-control's enable from no stream drops it around its configuration, and `release` takes the projections from it before it locks their `updates` and calls the device; `device_lost`, which locks `streams`, is only called after a device call or a panic, never under it.

**Deleted.** `RxCmd::Plan`, `RxCmd::Cut`, `TxCmd::Plan`, the `Plan` type and `StopRequest`; uhd-control's `lines`, `planned`, `tx_clocks`, `ended`, `seq`, `to_rx`, `hand`, `book_item`, `refusals` and `end`, and the refusal poll and loss booking in `release`; uhd-rx's `plan`, `handed`, `stopping`, `refused` and `known`, the refusal wait, its own orderly and abort cuts from `RxCmd::Cut`, the shutdown's fallback cut, its loss cut, the `rx_stop` row in `timing` (the `plan` row records the cut now), and the second poll after `rx_recv` (`samples` reads the plan); `Streams.refused`; `Core::lost_at` and its atomic. `ColdConfig` has no origin of its own: one is made per command at booking, its rate and channel count the segment's at configuration, its updates filtered by the segment's origin, so a segment that appears only after a refusal (a `start_rx` that resumes) has its configuration.

**Lines** (non-test code, the changed files, `#[cfg(test)]` modules and the harnesses excluded; raw `diff --numstat`, and lines that are neither blank nor comments):

| crate | added | removed | code lines before → after |
|---|---|---|---|
| `ezsdr-radio-uhd` (`provider/{rx,control,tx,core,mod}.rs`) | 391 | 390 | 2 568 → 2 556 |
| `ezsdr-mock-radio` (`lib.rs`) | 10 | 13 | 1 621 → 1 619 |
| `ezsdr-radio` (`timeline.rs`) | 148 | 83 | 213 → 260 |

After review round 1, counted again from `86e289a` with one rule (each file cut at its `#[cfg(test)]` items, blank and `//` lines dropped): `ezsdr-radio-uhd` 2 563 → 2 560, `ezsdr-mock-radio` 1 621 → 1 619, `ezsdr-radio` 213 → 260; +42 in all. The reviewer's count, 2 572 → 2 560, differs only in the rule. After review round 2, by the same rule: `ezsdr-radio-uhd` 2 563 → 2 555 (−8), the others unchanged; +37 in all.

The INFERRED 150–250 lines deleted did not come: the protocol went (about 200 lines of `rx.rs` and `control.rs`), and the shared booking (`Streams`: booking, refusal, end, pruning, the plan as read, the transmit clocks, the recorded plan, about 160 lines with their comments, the clocks and the recorded plan moved from `control.rs`) and #63's pruning came. What went is what kept two copies in step.

**#63.** `Line` keeps the planner's state as a checkpoint (`State`, from `plan`'s loop, which is now `State::replay` and `State::finish`) and `Line::prune(before)` folds the items whose effective instant is before `before` into it; the plan, the pruned segments included, is unchanged. The contract: nothing booked later takes effect before `before`, and no pruned item is refused later. uhd-rx prunes when it ends a segment, at the command that began it (that segment has begun, so nothing before it can be refused, and everything booked later is at or after now); uhd-tx at each switch, at the command that began the segment it leaves. The segment configurations of pruned commands go with them. `Line::book` now restores the line when a plan cannot be represented. `rm_26_a_pruned_line_plans_as_one_that_keeps_every_item` books every generated sequence, both directions, in receipt order into a line that prunes before each booking at its receipt and one that does not, every fifth change or start refused once booked, and compares the plans after every booking and refusal (some item is pruned); `rm_26_a_pruned_line_keeps_few_items` books 10 000 changes and keeps at most 2 items, all 10 001 segments in the plan. MockRadio does not prune: a simulated Run is short, and nothing asked for it.

**One receive-clock registration rule.** MockRadio registers a receive clock with the first block it publishes, and ends it at its cut only if it has one (`rx_settle`); RM-25 states the rule once, MR-11 follows. Layer 1's oracle derives the receive clocks from the segments that publish a block. Three MockRadio tests and `v61_02_capture_starts_at_the_requested_sample_index` stepped 1 ns or 1 ms past T0 and read the clock; they now step past the first block (2 ms at `x310-like`). A Session sees its receive clock only from the first block on.

**VH-8.** Layer 3 books each command into the shared receive line `lag` (0, 1 or 2 ms) after its receipt, as a slow uhd-control would, which is the plan latency as a delay before uhd-rx reads; uhd-rx books its refusals, `Core::device_lost` the loss and the harness `Provider::stop`'s end, at once. The oracle is unchanged but for two lines: a loss and `Provider::stop` reach the plan at their instants, and a command whose booking would come at or after a loss is not booked. "plan during a wait" is now "booking during a wait", and "items pruned" is counted. Layers 1, 2 and 3 are gated on every field. Coverage: layer 3, of 1 000 sequences, 352 a cut floored at what was delivered, 343 at `block_len` 65 536, 506 a booking during a wait, 241 an abort, 206 an orderly stop, 184 a loss, 171 a stop before a queued start, 167 a fault, 38 a refusal, 18 with items pruned; layer 2, `timed` not compared in 25 sequences (#62), nothing else printed; layer 1, nothing printed.

**Judgment calls.**
1. **UR-17's bounded receive request stays.** Removing `recv_len` fails layer 3 on seed 718 (`block_len` 65 536, a sequence error 3.75 ms before a cut at T0 + 180 ms): RM-18 makes the device skip a whole block there, and the unbounded request then covers samples from `k_f + block_len` and returns two receive calls after the cut, past the next origin, so the next timed start is late ("`rx_start` … not ahead of its origin"). With the bound the request ends at the cut, and the call returns within one call of it, which is what VH-4's ready term budgets. An overrun shortly before a cut does the same. So the note's premise, that the bound moves no instant, holds only without a fault before a cut. `recv_len`, `ur_17_a_request_ends_at_the_pending_cut`, UR-17's sentence and row B05 stay, B05 now killed by that test and by layer 3. The owner decides whether VH-4's ready term should instead budget the call a fault can stretch.
2. **A `Stop` carries no `Item.delivered`; only a loss and `Provider::stop` do.** With it, a `Stop` floored at what uhd-rx had delivered also moved the next origin by the start rule (layer 3, seed 126), a change of RM-25's outcome the note did not ask for. A `Stop` is floored by uhd-rx as before, so the recorded plan keeps UR-30's floor ceiling for a `Stop` only.
3. **An abort of a segment already cut but still being delivered** is ended at the first sample not yet delivered as soon as uhd-rx reads the plan after `Provider::stop(abort)` has begun (`Stream::follow` reads `streams.stop`, under the lock it already takes), as HEAD's `RxCmd::Cut` did; the first implementation waited for uhd-rx's shutdown, after uhd-control and uhd-tx were joined (review round 1). The timeline applies an abort only to its running segment, so this stays a ceiling of the plan, and of layer 3's oracle. A running segment is cut there in the plan itself, and uhd-rx ends it as soon as it reads it (a cut at or before what it delivered, its origin passed, stops and ends the stream at once).
4. **`Core::device_lost` books the loss**, whichever thread finds it, with uhd-rx's delivered sample. That covers the note's "uhd-rx books a loss it detects" and the losses uhd-tx and uhd-control find, in one place.

**Mutations.** Run on the remote runner (sim02t, sim03t), the touched and new rows only.
- *Retired* (14): G59, G61 (the `rx_stop` row is gone; the `plan` row and `ur_26_a_stopped_stream_keeps_its_end` hold the cut); J10 (no stop flag: the plan has no segment after `Provider::stop`'s end; `ur_26_no_segment_begins_once_provider_stop_has_begun` and `rm_26_the_timeline_cases`); J18, J24 (no refusal wait: a plan cannot predate a refusal its reader booked; `ur_25_a_refused_segment_is_booked_by_uhd_rx`, layer 3); J26 (uhd-rx makes no orderly cut of its own; J45, R15, `ur_26_uhd_rx_follows_the_stop_it_reads`); J27 (no plan is handed); J28 (no refusal is reported for later booking); J29, J30, J31 (uhd-rx's loss cut is gone; J46, J47, `ur_29_a_loss_ends_the_stream_at_its_instant`); R19 (uhd-control no longer books the stop's end; `Provider::stop` books it at its own instant; `ur_26_an_orderly_stop_cuts_at_its_instant`); B11 (an abort ends the stream as it stops it, so nothing can stop it again); R16 (the poll after the wait is gone; J52).
- *Re-spelled* (24), each killed: G60, J01, J04, J06, J06b, J07, J12, J13, J14 (now `Line::refuse`), J15, J16, J21 (now `core.rs`), J23, J25; U22, U32, U38, U40, U41 (a booking that waits for the plan's last origin), R05, R08, R12, R13, R15 (`Provider::stop` books no end). G03 survives and is recorded `equivalent`: the plan's cut of a segment only moves earlier, the floor never passes a cut already taken, and an abort's or a loss's cut ends the stream at once (G04 and G58, killed, pin that an end moves only earlier).
- *Added* (8), each killed: J45 (a cut at or before what was delivered not ended at once), J46 (a loss and `Provider::stop` booked without the delivered sample), J47 (the finder does not book the loss), J48 (pruned items not folded into the checkpoint), J49 (uhd-rx does not prune), J50 (uhd-tx does not prune), J51 (pruning keeps the pruned commands' configurations), J52 (`samples` does not read the plan).
- *Run unchanged, killed*: G04, G58, J02, J03, J05, J06a, J09, J11, J17, J19, F21–F29, F39, F40; U12, U13, U20, U21, U23, U24, U25, U26, U37, U39, U46, V06, R03, R04, R07, R14, R21, C02, B01, B02, B05, B06, B07, B12, B13, B15, B22, R10.

**Review round 1** (an independent Opus review, FAILED on two blocking findings; fixed the same day).
- *A loss found after uhd-rx read the plan* (blocking, VERIFIED by the reviewer's probe): uhd-rx checked `is_lost` after its turn's read of the plan and finished at the cut it held, so a loss found in between by uhd-control, uhd-tx or a panic left the receive clock unended, or ended after the loss — historical bug 5, which `Core::lost_at` had covered. `Rx::lost` now reads the plan again before it finishes; `device_lost` books the end and sets the flag under the plan's lock, so uhd-rx, which reads under that lock after it sees the flag, always reads the loss. `ur_29_a_loss_found_after_uhd_rx_read_the_plan_ends_the_stream` (no cut, and a later cut) and row J53 pin it. The order of the booking and the flag inside the lock is then unobservable: row J58 records that mutation (the reviewer's X06) as `equivalent`.
- *The `Line::made` guard on `Item.delivered`* (blocking): unpinned (the reviewer's X05 survived). `ur_26_an_end_takes_delivered_only_from_the_segment_it_cuts` (a change at 10 ms, 9 000 delivered, an abort at 12 ms: one segment, not a second cut at the first one's index) and row J54 pin it.
- *An abort of a segment cut earlier* (non-blocking): judgment call 3 above is corrected; `Stream::follow` cuts at once when `streams.stop` is an abort (row J55). The shutdown's own stop then serves only a queued start whose origin has not passed and a dropped command channel, each now pinned: `ur_26_an_abort_stops_a_queued_start_at_shutdown` (G60 re-spelled to it) and `ur_26_uhd_rx_aborts_when_its_provider_is_gone` (R08 re-spelled to it). R15 survived its old test once an abort no longer needed the end, and is re-spelled to `ur_25_a_receive_session_follows_its_recorded_plan` (an orderly stop's end).
- *The prune boundary and the booking rollback* (non-blocking): `rm_26_a_line_pruned_at_an_item_s_instant_keeps_it` (rows J56, the reviewer's X01) and `rm_26_a_refused_booking_leaves_the_line` (J57, X03).
- *Wording*: the header's Scope row and `failure-mechanisms.md` ("proposed") are corrected.
- *Declined*: (a) more deletion: what remains is one copy of the plan and the code that books it; the net growth is #63's pruning in `timeline.rs`. (b) #63's per-booking cost: the item count is bounded, but `State.out` keeps every cut segment and is cloned per replan, so a booking costs time linear in the Run's segment count under the lock (a copy of a few dozen bytes per segment; INFERRED small). Keeping the frozen prefix outside the checkpoint is the upgrade when a Run's segment count makes it measurable. (c) A `Stop`'s delivered floor stays in uhd-rx (judgment call 2): in production uhd-control takes the `Stop`'s instant under the lock uhd-rx writes `delivered` under, and samples arrive after their instants, so the floor practically never applies; carrying it moves RM-25's next origin. (d) `ur_26_a_change_booked_after_the_stop_instant_ends_its_clock` stays as timeline coverage, though `Provider::stop` now books its end under the lock: only the enable-from-no-stream gap, where the lock is dropped, can reach it.
- *Rows run this round*, all on the remote runner, all killed unless stated: new J53, J54, J55, J56, J57, J58 (`equivalent`, survived as recorded); re-spelled G60, R08, R12 (the old text matched `Rx::lost` too), R15; touched and re-run G03 (`equivalent`, survived), G04, G58, J13, J45, J46, J47, J52, U13, U32, B02.

**Review round 2** (an independent Opus review, FAILED on one blocking finding; fixed the same day).
- *A cut booked while a receive call waits* (blocking, VERIFIED by the reviewer's probe): round 1 made only the loss path read the plan again. A stream that went silent was still ended, at the `Timeout` arm, with the cut uhd-rx's turn had read, so a loss (uhd-control's time read, uhd-tx's report read) or `Provider::stop`'s orderly end booked during the wait ended the receive clock at the old, later cut while the plan, the recorded UR-30 plan, had the earlier one — historical bug 5 by a second path, which HEAD had too. Fixed once where every ending goes: `Rx::finish` takes the plans' lock, reads the plan (`Stream::follow`), and only then ends the clock (the clock registry's lock inside `streams`, as the lock order says). `Rx::lost` is deleted: a turn that sees the device lost calls `finish`. `ur_29_a_silent_stream_ends_at_a_cut_booked_during_its_wait` (a loss, and an orderly end, booked at 5 ms with a change cut at 10 ms; the call times out at 50 ms: the clock and the plan end at sample 5 000) pins it with `ur_29_a_loss_found_after_uhd_rx_read_the_plan_ends_the_stream`, which now calls `finish`; row J53 is re-spelled to the read in `finish`, its filter `ur_29_a_`.
- *uhd-tx's re-read of the plan* (non-blocking; the reviewer's X6 survived every lib test): `ur_25_uhd_tx_reads_the_plan_again_when_it_changes` and row J59.
- *A loss at the Run's start in arrival order* (non-blocking; X4 survived): on the UHD Module the order of a loss and a change at its instant shows only through the delivered floor, since a segment cut at its origin is not planned on receive and is planned cut at 0 on transmit either way. `ur_29_a_loss_takes_effect_before_a_change_at_its_instant` (12 000 samples delivered, a change at 10 ms, the loss at 10 ms: one segment cut at 12 000; booked in arrival order the change would cut it at 10 000) and row J60. The fixture delivers past `now`, which a real device cannot; the test pins RM-25's rule, not a production path.
- *Pruning across a floored `cold` change* (non-blocking; X2 survived): `rm_26_pruning_folds_a_cold_change_floored_at_an_earlier_one` (changes requested for 500 ms and then, received later, 300 ms; pruned at 900 ms; a change at 1 000 ms; both directions) and row J61. The UHD Module books the effective instant as `Item.e`, so only `Line::prune`'s public contract could reach this. The round-1 timeline tests are now listed under RM-26 in `design/07-radio-model.md`.
- *Declined: uhd-control's `colds`* (non-blocking, INFERRED by the reviewer). `Control.colds` keeps a `cold` change after uhd-rx or uhd-tx books it refused, so `config_at` and `channels_at` go on using the refused value — for `coerce` of the next change, and for the channel count a timed command is released on. HEAD did the same, and note 23 did not ask for it. It is not a second copy of the plan but of the configuration intent, and fixing it needs a decision this note does not make: what configuration is in force after a refusal (RM-25 halts the stream, and says nothing of the values), and a refused item pruned later loses its mark, so filtering by the line's `refused` flag would be wrong after pruning. It belongs in an issue labelled `area:uhd`, `mech:planned-state-read-as-actual` (its design answer is spec 22 VH-2), for the owner; not filed here.
- *Declined: more deletion* — `Rx::lost`'s re-read went (above); the line counts and the per-booking cost stand as round 1 recorded them.
- *Not done, owner decisions*: UR-17's bounded receive request (judgment call 1); whether MockRadio's version goes from 2.0.0 for its receive clock now appearing with the first block, not at the first sample instant (spec 22 step 3 bumped nothing inside its series; before spec 22 each behaviour change bumped it).
- *Rows run this round*, remote runner: J53 (re-spelled), J59, J60, J61 (new), J46, J47 (re-run: the loss paths `finish` now serves), all killed; J58 (`equivalent`, its text now naming `finish`) survived as recorded. J60 first survived a test through the transmit plan, which cannot show the order, and was killed by the test above.

**Tests.** `cargo test --workspace`: 996 passed. `cargo clippy --workspace --all-targets -- -D warnings`: clean. The Python suite (`~/.cache/ezsdr-venv`, the tree's `ezsdr-server`): 42 passed. `tests/fake.rs` at `--test-threads=128`, twice: 22 and 29 of 127 failed, against 27 and 29 for `86e289a` on the same host the same hour, mostly transmit tests and bench rehearsals (#60). After review round 1: `cargo test --workspace` 1 002 passed; clippy clean; Python 42 passed; `tests/fake.rs` at `--test-threads=128` twice, 22 and 26 of 127 failed (#60), at `--test-threads=2` 127 passed. After review round 2: `cargo test --workspace` 1 006 passed; clippy clean; Python 42 passed; `tests/fake.rs` at `--test-threads=128` twice, 22 and 30 of 127 failed (#60), at `--test-threads=2` 127 passed; layer 3's coverage unchanged. Layer 3's coverage and layers 1 and 2 are as above.

After the final review the orchestrator pinned UR-23's refused transmit clock (`ur_23_a_refused_transmit_change_ends_its_clock_at_its_origin`, row J62, the review's Z03), stated in RM-25 that `Provider::stop` counts in arrival order while a loss counts with the faults (the review's Z08), and filed the `colds` question as #64.
