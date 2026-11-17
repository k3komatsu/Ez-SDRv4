# #106 — test plan v2 (AGENTS §9 step 2)

**v2 changes (implementer, 2026-11-17, sub-step A):** Sol's plan review adopted in full (majors 1-3: booked-joiner ACK W9, final-record writers W4d/W4e, N106-27's killer is W5; minors 4-8: L2, C3, L10 scripts, MT3D-55 retargeted to `crates/ezsdr-radio/tests/staging.rs`, B6 per effective bucket, reader guard + throughput readers + synthetic guard test, T5 wording and `design/18:571`) and the orchestrator's decisions on OQ-1...OQ-11 (`decisions.md`, last entry). Sub-step A implements §1 (lifecycle parts), L, E, C, §9 rows touched by A, §10.1, T1, T2, T7, T9, T10(KL-5, KL-39, KL-13), T12, T13; sub-step B implements the bound, keys, wiring, selector, Manifest (B, K, W, S, M, §10.2, the rest of §11).

Author: Claude Code Sonnet subagent (effort high), 2026-11-17, read-only against `work/n106` (main 5b8059c7; only `handoff.md` is modified, by the orchestrator). Nothing was built or run: every "expected" below is read from the code and the tests, and is **INFERRED** unless it is a figure the cited test already pins. Where a number depends on `busy`, `r`, `x` of a scripted send I give the relation, not the value; the implementer works the value out in the test's comment, as `mt3d.rs` does, before the test is written. Paths are relative to `/home/komatsu/local_work/Ez-SDRv4`.

Inputs read: `decisions.md` (wins over the draft), `design-draft.md`, `fable-review.md`, issue #106, AGENTS §6/§7/§9, `design/18` UR-5/22/23/28/30, `design/02` SC-24/25/28, `design/07` RM-37, `provider/{mod,core,tx,rx,control}.rs`, `crates/ezsdr-radio/src/staging.rs`, `tx/tests/mt3d.rs`, `tests/fake.rs`, `tests/common/*`, both mutation lists.

## 0. How to read this plan

- Sequence ids: **L** lifecycle, **E** endings that stay, **C** counters, **B** bounded type (pure), **K** key functions (pure), **W** wiring, **S** selector, **M** Manifest. Mutation ids are `N106-nn` (new), others are existing ids. Texts are **T1…T13**.
- Names of new types and functions are **working names** (`Bounded`, `Key`, `Records::written`, `is_resume_part`, `PendingReport.part`, `Tx.open_part`). The implementer may rename; the assertions stay. Kind strings that appear in `record_bound.omitted` are user-visible: they are pinned in §11.1 and flagged in OQ-11.
- "Rig" names: **mt3d rig** = `src/provider/tx/tests/mt3d.rs` (`SwitchRig`, `late_repeat()`, `resumed_part()`, `gap_rig()`, `report()`, `at()`, `go()`, manual clock, FakeDevice); **control rig** = `tx_rig()` of `control.rs:807`; **fake.rs** = `tests/fake.rs` Session on the manual-clock fake (`tx_session`, which goes through `fake()` and already forces `ClockMode::Manual`).
- Verdicts never depend on wall-clock time. The one place a Session test depends on how much virtual time fits (L11, W7) is marked and has a stated fallback.

## 1. The production shape the tests assume

| # | Shape | Where | Replaces / adds |
|---|---|---|---|
| P1 | `Bounded`: push-only; `push(key, row)`; per-kind kept count; `omitted() -> &BTreeMap<String, u64>`; at most 64 keyed kinds, later kinds under `"other"`; `Key::Exempt` is never tracked; `Deref<Target=[Json]>`, `Clone`, `Debug`, `Serialize` (as the array). `Records.{bursts, device_async, rejected, timing}` become `Bounded`; `applied` and `switches` stay `Vec<Json>`. `Records::with_limit(n)`, `Records::written(&self)` builds every written section **and** `record_bound` from one `&self`. | new `provider/core/records.rs` (or in `core.rs:786`) | `core.rs:786-803` |
| P2 | Key functions `timing_key(&Json)`, `rejected_key(&Json)`, `async_key(&Json, booked_ack: bool)`; `TIMING_EXEMPT = [tx_stop_applied, rx_segment, first_rx_block, rx_stop_untimed, rx_stop_late]` | same | `Core::timing`/`reject_note` keep their signatures (anchors of MT3D-21/22, U43, R13, B40, P277, N92-2 survive) |
| P3 | `is_resume_part(first: bool, paused: bool) -> bool` = `paused && !first`; **one** decision used for (a) `PendingReport.part: bool` (replaces `resumed: Option<i64>`) and (b) `Tx.open_part: bool` (new; assigned **after** `record(step)` at every start-of-burst send, read only in `record`'s `Discontinuity` arm for `closed`) | `tx.rs:91-122,355-363,759-790,816-828` | `stall` goes; `remember_burst` stays as the test shorthand `part = false` (43 call sites; Fable's merge does not apply because `part` is needed, OQ-5) |
| P4 | `retire(..)` also tells whether any entry an `Ack` matched had `part == false` (`booked`); `Tx::report` pushes the async row **after** `retire` so the key is known (row order is still read order) | `tx.rs:373-405,1397-1401` | new state: `PendingReport.part` only |
| P5 | `stat("tx_resumes", 1)` and `stat("tx_gap_samples", planned - handed)` beside the `tx_resume` row; both keys in `Records::default()`'s fixed list (OQ-2) | `tx.rs:1115`, `core.rs:796-803` | |
| P6 | `UhdRadio.record_rows: usize` read by a pure `record_rows_per_kind(&Binding) -> Result<usize, ModuleError>`; `Core::new(.., rows)` → `Records::with_limit(rows)`; key added to the allow-list | `mod.rs:219-245,507`, `core.rs:918-933`, `test_support.rs:127` | OQ-1 (seam) |
| P7 | Removed: `Paused.{stall, source}`, `TxBook::resume_at`'s `stall` argument, `PendingReport.resumed`, `Reached.resumed`, the stall block of `dropped` and its `newest` parameter (`late` stays: it names the cause), the stall in `resume_repeat` | `staging.rs:383-398,778-792`, `tx.rs:101-121,1103-1135` | `DeviceChain::first()` stays (debug asserts, `tx.rs:611,722`); `rx_stall` is unrelated and untouched |

Small N: unit rigs replace the records before the first row, `*lock(&rig.core.rec) = Records::with_limit(n)` (the fields are already `pub(crate)` and read directly by dozens of tests); Session tests set the selector key through `profile(&dir, json!({ "record_rows_per_kind": n }), …)`. **Deviation from the brief:** no `UhdRadio` builder is added; the selector key is the public path and exercises the refusal; a builder would add surface for no test (OQ-9). No test calls `std::env::set_var`.

## 2. Lifecycle (L) — mt3d rig, FakeDevice, manual clock

Common observations: `emitted(&events)` (event kind, `resumes_at`), `resumes(&core)` (the `tx_resume` rows), `ended_notes(&core)`, `rig.tx.open_next()`, `tx_sends(&device)`, `closing_sends(&device)`, `rows(&rig)` (burst records), `report(&mut rig.tx, code, &clock, tick, channel)`. New helper `assert_counters(&core)`: while the Run is below N, `stats.tx_resumes == resumes(&core).len()` and `stats.tx_gap_samples == Σ(row.resumes_at − row.from)` (C1).

**L1 — an underflow on a part before it played the old stall length resumes again** (rewrites `mt3d_a_part_that_plays_less_than_the_stall_part_ends_the_repeat`; rename `mt3d_an_underflow_on_a_part_resumes_the_repeat_again_whatever_it_played`).
- Ops: `resumed_part()` (first drop resumed at R1 = 32 000, part handed); read `h2 = open_next()`; `report(Underflow, tick = 32_000 + played)` for `played ∈ {0, 1, 10_001, 10_002, 10_003}` (the old P−1, P, P+1 and the two below it), a fresh rig each.
- Expect: `ended_notes == 0`; `resumes` +1 with `cause == "underflow"`, `from == h2`, `resumes_at == R2`; `open_next() == Some(R2)`; `emitted == [("underflow", Some(R2))]`; `closing_sends` +1; `unacked` empty; `R2` on the lattice `20 000 + 1 000 j` and `R2 − R1 ≥ 10 032` (m + 32 samples; R2's exact value is INFERRED, worked out from `busy` in the test comment); `assert_counters`.
- Wrong behaviour that fails it: b′ left in (the `played < stall` cases end the repeat); a rule that ends on `played == 0`; a resume that writes no row / no close / an event without `resumes_at`.
- Rows: N106-01 (a part's drop ends the repeat, restored through `open_part`); MT3D-10, -11, -12, -13, -28 retire (§4).

**L2 — a late report that reaches a part (nothing of it played) resumes it again.**
- Ops: `resumed_part()`; `report(TimeError, tick = 33_000)` (≥ the part's first sample 32 000); then advance the owner to the new part's send window (the resume point R2's `send_asap` window: owner to `R2/1 000 − 9` ms, as L10 does, `go()` is not enough: it advances no time, Sol minor 4) and `go()` the next part.
- Expect: `emitted == [("late", Some(R2))]` with the event's `target == 32 000` (the part's own start, UR-28); `resumes` +1 with `cause == "late_at_device"`; `open_next() == Some(R2)`; no note; the part's record ends `discontinuity` once part 2 is handed (`rows(&rig)`: 2 `discontinuity` rows, targets `[20 000, 32 000]`); `assert_counters`.
- Wrong: the b′ zero case (ends the repeat); the part not remembered (no correlation → no event; kills the re-anchored MT3D-26).
- Rows: N106-01; MT3D-26 (re-anchored, killer retargeted here).

**L3 — one delayed report that retires two entries keeps both resumes** (rewrites `mt3d_a_delayed_underflow_that_retires_two_entries_applies_the_stutter_rule`; new name `…_keeps_both_resumes`).
- Ops: as the old test up to the second paused state: `late_repeat()`; expired check at 22 ms (R1, cause `expired`); part handed (24, 25 ms); expired check again at `h2 + 1_500` (R2, cause `expired`); drain the fake's own reports; then (a) `report(Underflow, tick = 33_000 + played)` with `played ∈ {1, 11_001}`; (b) a fresh rig, `report(TimeError, tick = 33_001)`.
- Expect (a): `emitted == [("underflow", Some(R2))]` (UR-28: the newest entry's plan; R1 is taken and not carried); `resumes == 2` (no third); `open_next() == Some(R2)`; `ended_notes == 0`; `tx_sends` unchanged (no close, no start at R2); `unacked` empty. Expect (b): `emitted == [("late", Some(R1)), ("late", Some(R2))]` — each Kernel burst keeps its own plan — and the same state. Both `played` values give the same result (the rule is gone).
- Wrong: any ending rule (old `ends` cases); the event built from `reached.first()`; the older entry told with no plan; a second resume made by the delayed report.
- Rows: N106-03 (`reached.last()` → `reached.first()` in the Underflow arm), N106-04 (`_ => reached.resumes_at` → `_ => None` in the TimeError arm); MT3D-27 and MT3D-80 retire; MT3D-37 retargets (E1).

**L4 — two channels, a late drop, either order: one resume, one event; the part too** (rewrites `mt3d_two_channels_late_resume_once_in_either_order_and_the_part_ends_on_one_channel`; new name `…_and_the_part_resumes_on_one_channel`).
- Ops: the first block as today (the dropped booked burst, `first_channel ∈ {0,1}`, then the other channel). Last block: `resumed_part()`, `unacked.back_mut().channels = {0,1}`; for `first_channel ∈ {0,1}`: `report(TimeError, 33_000, first_channel)` then `report(TimeError, 33_000, 1 − first_channel)`.
- Expect: after the first report, one event `("late", Some(R2))`, `resumes` +1, `open_next() == Some(R2)`; after the second, **no** event, **no** second resume, `ended_notes == 0` (the device burst is closed, so `!hit` → `Nothing`).
- Wrong: the second channel resumes again (breaks the 100/s bound: two closes, two rows per drop); the second channel tells again; the part still ends on one channel.
- Row: N106-02 (the `if !hit { return Dropped::Nothing; }` early return removed: the paused booking is not resumable, so the second channel would abandon it: note + abandon). Also kills L7's mutant.

**L5 — two channels, Underflow** keeps `mt3d_two_channels_resume_once_and_tell_each_underflow` unchanged (first Underflow resumes with `resumes_at`; the second's tick is before R: its event has none; one row). Add `assert_counters`.

**L6 — a switch part resumes like any part** (rewrites `mt3d_a_switch_part_keeps_the_stutter_bound_of_the_gap_it_replaced`; new name `mt3d_a_switch_part_resumes_like_any_part`).
- Ops: `gap_rig(o::ramp(1, 1_000))`; switch at 4 000 replaces the paused repeat at R = 20 000 (as the old test); `report(Underflow, 20_000 + played)` for `played ∈ {1, 10_001, 10_002}`.
- Expect: each case `ended_notes == 0`, `open_next().is_some()`, `resumes` +1.
- Wrong: the former P−1 case ends the repeat; `replace_paused` losing `paused` (MT3D-54 stays its killer: `mt3d_a_halt_in_the_gap_after_a_switch_replaced_the_repeat_sends_nothing_and_cancels_the_switch`).
- Rows: MT3D-55 retargeted to the staging test (§9.2); L6 is the second killer (with `paused: None` the part is not `paused`, so the part is not recognised as a resume part).

**L7 — a stopped repeat's delayed report touches no newer repeat** (`mt3d_a_stopped_repeat_s_delayed_report_does_not_end_a_newer_repeat_that_waits_to_resume`; assertions unchanged, only its comment — "sol-code major 2"/the `Paused.source` guard — is reworded). Now it pins `!hit → Nothing` alone (B's device burst is closed by its pause). Rows: N106-02; MT3D-79 retires.

**L8 — a second drop found in the gap resumes again at the plan** (rewrites `mt3d_a_second_drop_found_in_the_gap_abandons_at_the_handed_cursor`; new name `…_a_part_the_expired_check_resumed_is_told_by_its_report_and_resumes_nothing_more`).
- Ops: `resumed_part()`; owner to `h2/1 000 ms + 1 ms`; `step()` (expired check → H2, R2); then `report(TimeError, 33_000)` (the part's own correlation).
- Expect: `resumes == 2` before and after the report; `emitted == [("late", Some(R2))]` (the plan the expired check made, once); `open_next() == Some(R2)`; `ended_notes == 0`; `tx_sends` unchanged by the report; then `sweep` to R2: the third part starts at R2 (`part_start(&rig) == Some(R2)`).
- Wrong: the report resumes a second time; ends the repeat; carries no plan.
- The "abandon at H, no stray close at R" property is pinned where it still happens: E1.
- Rows: MT3D-15 and MT3D-38 retarget (§4).

**L9 — the expired check, then its report** keeps `mt3d_the_expired_timeline_resumes_and_the_report_that_comes_later_carries_the_event` and `…_and_a_later_underflow_carries_the_plan` (MT3D-33, MT3D-39 stay). Add: `stats.tx_resumes == 1` before and after the report (C2).

**L10 — every part dropped late: the repeat keeps resuming, transmits nothing, one `TIME_ERROR { late_at_device }` per cycle** (m7; new `mt3d_a_repeat_whose_every_part_is_dropped_late_keeps_resuming_and_transmits_nothing`).
- Ops (manual clock, exact): `late_repeat()` (device at 21 ms, first send dropped); then K = 6 times: `rig.tx.reports(Wall::ZERO)` (resume to R_i); owner to `R_i/1 000 − 9` ms; `rig.device.set_time((R_i/1 000 + 1) ms)` (device already past the part's start, the way `late_repeat` sets 21 ms past 20 ms); `go()` (part handed and dropped late in the fake); owner to `R_i/1 000 + 1` ms. Then one last cycle: recover at the **next** planned point R_{K+1} (the owner and the device are already past R_K + 1 ms, so a recovery at `R_K − 5 ms` would go backwards): `rig.tx.reports(Wall::ZERO)`, `set_time` to R_{K+1} − 5 ms, owner to R_{K+1}/1 000 − 9 ms, `go()` so the part plays. Root ticks are built with `clock.instant(sample)`, never with raw sample counts.
- Expect: per cycle one event `("late", Some(R_{i+1}))` and one `tx_resume` row `cause == "late_at_device"`; `R_{i+1} − R_i ≥ 10 032` and on the lattice; `ended_notes == 0` through all K; `open_next() == Some(R_K)`; `rig.device.transmitted(clock.instant(20_000), (R_K − 20_000 + 1_000) as usize)` all `None` (nothing radiated); `assert_counters` with `tx_resumes == K`; the recovery cycle: `transmitted(clock.instant(R_{K+1}), 1_000)` all `Some`, no further resume. If the fake cannot move its time forward like this, drop the recovery step (INFERRED).
- Wrong: the repeat ends at the second drop (the old zero case); a resume inside the drain (`R_{i+1} − R_i < m + 32`, a missing `m` is also killed by MT3D-02); two events or two rows per cycle; counters off.
- Rows: N106-01, plus MT3D-02/MT3D-24 already kill neighbours; no new row.

**L11 — Session level, no cycle count** (rewrites `ur_28_the_device_reports_a_late_burst`, `fake.rs:2577`; **keeps its name**: phase7 row U28 filters on it and `design/18:775` lists it among the tests whose verdict must not depend on the test thread).
- Ops: `tx_session(FakeConfig { faults: vec![SlowSend(20 ms)] })`; `start_repeat` at 8 ms with `tone(100)`; `wait(400 ms)`; `finish`.
- Expect (no count asserted except the floor below): `late` = the `late_at_device` events; `late[0].late_by_ns > 0`; `late[0].target == records[0].target`; for every consecutive pair `late[i].resumes_at == Some(records[i+1].target)` while both exist (the part's record opens at the plan); every event except possibly the last carries `resumes_at`; `rows(tx_resume, cause late_at_device) == events with resumes_at`; consecutive `resumes_at` differ by ≥ 10 032 samples (1 MS/s, m = 10 ms); `rejected` has no "not resumed" note; `late.len() >= 3` — **the floor is a virtual-time count** (threads are manual-clock participants; the old run fitted two cycles in 100 ms, so 400 ms is a margin of about four); the implementer repeats it 20 times (§9 step 8) and, if any run gives fewer, lowers the floor to `>= 1` and says so (OQ-7). The exact count is pinned by L10.
- Wrong: the repeat ends at the second drop (floor); a part record not opening at the plan; the first event without `resumes_at`.
- Row: U28 keeps killing `Refused` for `LateAtDevice`.

## 3. Endings that stay (E) — reuse, none rewritten

Each ending must still end the repeat. The existing test pins it; the table says which and which rows depend on it.

| E | Ending | Pinned by (file) | Rows |
|---|---|---|---|
| E1 | sequence loss, running and in the gap (`SeqError`, `SeqErrorInBurst`): note, abandon at H, no send at or after R, no stray close, region footprint H + 5 | `mt3d_what_still_ends_a_repeat` (first block), `mt3d_a_lost_sequence_while_the_repeat_is_paused_ends_it` (`mt3d.rs`) | MT3D-08, -09 (re-anchored), -78 keep; MT3D-15 and MT3D-37 retarget here |
| E2 | failed or short send (`FailSend`, `ShortSend`, `StalledSend`) | `mt3d_more_that_ends_a_repeat_a_resume_point_past_the_clock_and_a_failed_continuation` (second loop), `mt3d_a_stalled_start_of_the_part_ends_the_repeat_with_no_stray_close` | MT3D-32 keep |
| E3 | failed reopen / three unconfirmed T3 | `mt3d_a_recovery_that_fails_ends_the_repeat_and_the_clock` | — |
| E4 | lost device, `Provider::stop` in the gap | `mt3d_provider_stop_and_a_loss_in_the_gap_send_nothing_and_tell_nothing` | — |
| E5 | `R` or a switch boundary past the clock | `mt3d_more_that_ends_…` (first block), `mt3d_an_expired_timeline_whose_resume_point_is_past_the_clock_ends_the_repeat_with_a_note`, `mt3d_a_resume_point_or_a_switch_boundary_the_clock_cannot_hold_ends_the_repeat_with_a_note` | MT3D-36, -70, -71 keep; **MT3D-38 retargets** here (see below) |
| E6 | a `Stop` in the gap | `mt3d_a_stop_in_the_gap_sends_nothing_and_ends_the_record` | — |
| E7 | a burst that is not a repeat ends at its first Underflow / late drop | `mt3d_what_still_ends_a_repeat` (second block), `mt3_a_finite_burst_handed_late_transmits_nothing_and_is_told_once` | MT3-15 keeps |

**E5 addition (for MT3D-38).** Add to the first block of `mt3d_more_that_ends_…` (late report, `busy = i64::MAX`): `assert_eq!(closing_sends(&rig.device).len(), 1, "the resume's close only")`. Reason (INFERRED from the code): the booking is paused when `resume_repeat` fails, so the abandon's `device_open` argument is `burst_is_open() == false`; the mutant `abandon(true, …)` sends a second close. Fable's retarget to the one-shot late drop (`mt3d.rs:279-296`) would **not** kill it: a one-shot's device burst is open there, so `burst_is_open()` is already true and the mutant is equivalent.

## 4. Counters (C)

**C1 — exact counters** (helper `assert_counters`, used by L1, L2, L3, L8, L9, L10, and by the existing resume tests that read `resumes(&core)`): `stats.tx_resumes` = number of `tx_resume` rows + `record_bound.omitted.timing.tx_resume`; `stats.tx_gap_samples` = Σ over the **planned** spans `resumes_at − from`. Plus one case where `R` moves: `mt3d_a_missed_resume_point_moves_to_the_next_on_time_boundary_and_is_told_once` gains `tx_gap_samples == planned − handed` while the part really starts at the moved `R'` (the counter is a lower bound; the `send_asap` event's `late_by_ns` says the rest). Rows: N106-05 (`tx_resumes` not counted), N106-06 (`tx_gap_samples` adds `planned` instead of `planned − handed`). MT3D-21/22 keep (`self.core.timing(json!({ "what": "tx_resume", … }))` keeps its text).

**C2 — an expired-check resume is counted once and makes no event**: in L9, `stats.tx_resumes == 1` before the report that comes later and after it; `emitted(&events)` empty before the report (as today). Wrong: a second count by the late report (the report does not resume: `Nothing`).

**C3 — many resumes past a small N: the Run goes on** (new `mt3d_many_resumes_past_the_bound_keep_going_and_are_counted_exactly`, mt3d rig).
- Ops: `Records::with_limit(3)` set on a `late_repeat_n(3)` rig; K = 8 cycles of "scripted Underflow": hand the part at R_i (owner and device on time), `report(Underflow, R_i + 1)`; record `(H_i, R_i)` per cycle from `open_next()` before the report and the event after; then, **after** the liveness assertions below, `stop(&mut rig)` (Stop ends the open booking, so `open_next()` is `None` afterwards; Sol minor 4).
- Expect: every cycle resumes (`ended_notes == 0`, `open_next() == Some(R_i)` at the end, asserted before Stop); `stats.tx_resumes == 8`; kept `tx_resume` rows `== 3` and `written().record_bound.omitted.timing.tx_resume == 5`; `stats.tx_gap_samples == Σ(R_i − H_i)` over all 8; `async`: `Underflow` kept 3, omitted 5; `bursts`: the booked first record, `resumed_part` kept 3 omitted 4 (the K−1 = 7 parts closed by a later resume), and the closing record `end == stop` kept (after the Stop); the Run's `open_next()` is `Some` before the Stop (it went on).
- Wrong: the repeat ends when rows run out; counters taken from kept rows; the closing record bounded; a gap recorded twice.
- Rows: N106-05, -06, -07…-09, N106-25…-28 (as the wiring rows below).

## 5. The bound, pure (B) — `Bounded` and `Records::written`, no rig

Tests in `provider/core/records.rs` (`mod tests`). N is passed explicitly except B1, which uses the production default 1 000.

| B | Test | Asserts | Wrong behaviour | Row |
|---|---|---|---|---|
| B1 | `bound_a_kind_keeps_its_first_n_rows_and_counts_the_rest` | 1 001 rows of one kind (N = 1 000): `len == 1 000`, the kept rows are `0..999` in order, `omitted == {"k": 1}`; 1 000 rows: `omitted == {}` | omitted never counted; off by one (`>`/`>=`) | N106-07 (count after keep: the omitted counter is not incremented), N106-08 (`>=` ↔ `>`) |
| B2 | `bound_two_kinds_keep_their_own_budgets` | `a` × 1 500 and `b` × 10 interleaved: kept `1 000 + 10`; omitted only `a: 500` | one budget per section | N106-09 |
| B3 | `bound_a_one_off_after_a_flood_is_kept` | `a` × 1 500 then one `b`: `b` kept, `omitted == {"a": 500}` | the section-wide first-N (rejected by the draft) | N106-09 |
| B4 | `bound_an_exempt_key_is_never_bounded` | N = 2: 10 `Key::Exempt` rows → all 10 kept, `omitted == {}`; an exempt row arriving **after 64 keyed kinds** and after `other` exists is still kept; exempt rows use no kind slot (the 65th *keyed* kind goes to `other`, not displaced by exempt ones) | exempt tracked; exempt cut by the kind cap | N106-10 |
| B5 | `bound_the_65th_kind_goes_to_other` | N = 1: kinds `k0…k63` once (64 kept), then `k64` (kept under `other`), `k65` (`other`, omitted): `omitted == {"other": 1}`; a later `k0` row is omitted under **`k0`**, not `other` (its own budget); a kind that was keyed before the cap keeps its budget | cap absent (unbounded key count); rows past the cap neither kept nor counted | N106-11 (cap removed), N106-12 (over-cap rows dropped) |
| B6 | `bound_counts_before_it_keeps` | a deterministic pseudo-random sequence of 5 000 pushes over 100 kinds, N = 7: for every **effective bucket** (an independently computed first-64 mapping: the first 64 distinct kinds in push order are their own buckets, later kinds land in `other`) `kept + omitted == pushed`, and globally Σ kept + Σ omitted == 5 000 and Σ kept == `len()` | a writer that skips the count | N106-07 |
| B7 | `bound_derefs_to_the_rows_in_write_order` | rows of three kinds interleaved appear in push order; `len`, `iter`, `[i]`, `to_vec`, `Serialize == Json::Array(rows)` | reordering by kind; a reader that sees a different array than the Manifest | — |
| B8 | `bound_a_limit_of_one` | N = 1: the first row of each kind kept, the rest counted | an off-by-one that hides at 1 000 | N106-08 |
| B9 | `records_written_builds_every_section_and_record_bound_from_one_borrow` | floods across sections (N = 3: `timing tx_resume` × 5, `async Underflow` × 4, `rejected tx_burst:UR-22` × 5, `bursts resumed_part` × 4, `applied` × 10, `switches` × 10): `written()` returns `applied, bursts, async, rejected, switches, timing, stats, record_bound`; each bounded section equals its first 3 rows; `record_bound == {"rows_per_kind": 3, "omitted": {"timing": {"tx_resume": 2}, "async": {"Underflow": 1}, "rejected": {"tx_burst:UR-22": 2}, "bursts": {"resumed_part": 1}}}`; `applied` and `switches` keep all 10 and appear **nowhere** in `omitted`; a clean `Records` gives `omitted == {}` and still writes `record_bound` (always written) | two sections' omitted swapped; `rows_per_kind` taken from a constant; `record_bound` missing when clean; `applied` or `switches` bounded | N106-13, N106-14, N106-15 |

**Same guard (m5).** `Records::written(&self)` builds the sections and `record_bound` from one borrow, and `write_sections` is `let rec = lock(&core.rec); for (name, value) in rec.written() { put }`: by construction a row cannot land between a clone and the count. The race itself (a UR-16 detached thread still writing during `cleanup`) is **not** tested: its detection depends on thread timing, so a verdict would be a coin-flip rather than a pin; B9 is the deterministic form (count, rows and `omitted` agree for one `&Records`). No mutation row for the guard (the construction removes the seam an anchor could mutate).

## 6. Key functions, pure (K)

| K | Test | Asserts | Wrong | Row |
|---|---|---|---|---|
| K1 | `key_async_by_code_with_booked_acks_exempt` | `async_key({code: Underflow…}, false) == Kind("Underflow")`; `TimeError`, `SeqError`, `Other(7)` → `Kind("Other(7)")`; `BurstAck` with `booked_ack = true` → `Exempt`; `BurstAck`, `booked_ack = false` → `Kind("BurstAck")`; `BurstAck` with `tx_flush: true` → `Kind("BurstAck:flush")` (also when `booked_ack = true`: a flush ACK matches no entry); a flagged `Underflow` keeps its code (OQ-4) | flush ACK in the `BurstAck` budget (so an ordinary flood hides the recovery's evidence, `flush_evidence`, `tests/common/e3.rs:109-114`); a booked ACK keyed | N106-21 |
| K2 | `key_timing_by_what_and_the_five_exempt_names` | `timing_key({what: "tx_resume"}) == Kind("tx_resume")`; **each** of `tx_stop_applied`, `rx_segment`, `first_rx_block`, `rx_stop_untimed`, `rx_stop_late` → `Exempt`; a new name (`"rx_error"`, `"ref_locked_failed"`, `"time_read_failed"`, `"rx_restart"`, `"rx_block_refused"`, `"device_lost"`, `"plan"`) → keyed; no `what` → `Kind("other")` | an exempt name missing from the list (the test iterates all five) | N106-17 |
| K3 | `key_rejected_by_call_then_kind_and_rule_then_field` | `{call: "tx_flush", attempt: 2}` → `tx_flush`; `{call: "tx_async"}`, `tx_close`, `set_thread_priority` → themselves; `{kind: "tx_burst", reason: "UR-22: …; burst not resumed"}` → `tx_burst:UR-22`; `{kind: "stop", reason: "UR-26: the Stop target…"}` → `stop:UR-26`; `{kind: "tx_burst", reason: "UR-21: …"}` ≠ the UR-22 key; reasons carrying device text — `"EOB close accepted 1 of 2 samples"` (no rule id) → `tx_burst`; `"device said UR-22 inside"` (id not at the start) → `tx_burst`; 200 distinct reasons sharing the prefix `UR-26: …device text n…` → **one** key; the named-field rows `thread`, `leaked`, `event_not_emitted`, `burst_tracker`, `clock_not_ended`, `clock_not_registered` → the field's name; anything else → `other` | the whole reason as the key (unbounded cardinality from device text); the rule id dropped (so a flood of refusals hides "the resume point does not fit the clock", Fable m1); `call` ignored | N106-18, N106-19, N106-20 |
| K4 | `a_part_is_a_start_of_burst_after_the_first_send_while_paused` | `is_resume_part(first, paused)`: only `(false, true)` is true; `(true, true)` (a switch that replaced a paused repeat) and `(true, false)`, `(false, false)` are false | a switch part counted as a resume part (its ACK and its record then bounded) | N106-24 |
| K5 | `cardinality_a_flood_of_new_kinds_cannot_grow_a_section_without_bound` | 10 000 rows with 10 000 distinct `what` values, N = 5: `len ≤ 65 × 5`, `omitted` has ≤ 65 keys; a `TxCode::Other(i)` flood for 1 000 distinct `i` the same | an unbounded key space (Fable m4) | N106-11 |

## 7. Wiring (W)

**W1 — Action-paced timing rows are never bounded** (control rig, `control.rs`, beside `ur_30_a_transmit_stop_records_the_bound_its_bookings_end_at`; new `ur_30_a_stop_per_action_is_never_bounded_past_n`). `Records::with_limit(2)`; 5 timed bookings each followed by a TX `Stop` (the helper of the existing test); expect 5 `tx_stop_applied` rows, `written().record_bound.omitted == {}`. Wrong: the key function bounds `tx_stop_applied` (a clean Run of 1 001 Stops loses the Stops' applied instants, which exist nowhere else; Fable M2). Row: N106-16 (the name missing from the list). The other four exempt names are pinned by K2; their writers (`rx.rs:287,415,468,678`) all go through `Core::timing`, so a missing wiring would be a missing `timing_key` call, which W5 and K2 together catch; one receive writer is not given a rig of its own (an INFERRED saving; the audit in §10 lists them).

**W2 — a booked burst's ACK is never bounded; a resume's close ACK is** (mt3d rig, `mt3d_a_booked_bursts_ack_is_kept_and_a_resume_closes_are_bounded`).
- Ops: `repeat_rig()`, `Records::with_limit(2)`, `book_repeat(1, 20_000, 0)`, owner 17/18 ms, `hand(2)` (two healthy sends, the device on time); owner to 26 ms; `step()` (**expired check**: the close goes to the fake, which acknowledges a closed device burst at `cursor + 1`, `device.rs:866-873`); `rig.tx.reports(Wall::ZERO)` (reads that ACK: it matches the booked entry, `part == false`). Then K = 4 more cycles: hand the part at R_i, owner to `h + 1 500`, `step()` (expired again), `reports(Wall::ZERO)`.
- Expect: `async` has 1 + 2 = 3 `BurstAck` rows (the booked first device burst's ACK exempt, then the first 2 of the 4 part ACKs); `record_bound.omitted.async == {"BurstAck": 2}`; no `Underflow` row exists.
- Wrong: every `BurstAck` bounded (2 rows, omitted 3); every `BurstAck` exempt (5 rows); the booked ACK keyed because `part` is set for the booking's first device burst. The flush ACK of the open (UR-22's T3, not run by this rig) stays K1's.
- Observation: the test reads `rig.tx.unacked.len()` before and after each `reports(Wall::ZERO)`, so an ACK that matched is seen as a removed entry and an unmatched one as an unchanged queue.
- Rows: N106-22, N106-23, N106-24. Note: the first device burst of a repeat is `part == false`, so its close ACK is exempt even though it is a resume's close; that is one row per booking and keeps the bound (documented in UR-30's text, T3).

**W3 — the report-driven resume leaves the close ACK unmatched, hence bounded** (same rig, K = 4 scripted Underflow cycles as C3, then `reports(Wall::ZERO)` once at the end): the fake's close ACKs are read after their entries were retired by the Underflow reports, so they match nothing and all four are `BurstAck`-keyed (kept up to N). INFERRED from `retire`'s tick rule (`start < tick` on a missing entry). This is the "unmatched" case of the decision; it also pins that an unmatched ACK is never exempt.

**W4 — the part predicate, three chains** (mt3d rig, N = 1, `bursts` section).
- **W4a (repeat only).** Chain: booked A, then four Underflow resumes (parts P1…P4), then `stop`. Expect `bursts` targets `[A, P1, P4]`, `end == [discontinuity, discontinuity, stop]`, `record_bound.omitted.bursts == {"resumed_part": 2}` (P2, P3 closed by a later resume are bounded; A is a booked record; P1 is the first `resumed_part`; P4 is the closing record). Wrong: `open_part` assigned **before** `record(step)` (then A is counted as `resumed_part`: `[A, P4]`); the predicate ignoring `open_part` (A bounded: with N = 1, P1 is then omitted); the closing record keyed as a part (P4 missing); the `end_open_at`/`abandon` stop sites (`tx.rs:860,974,1178`) keyed. Rows: N106-25 (assignment order), N106-26 (the `Discontinuity` arm bounds `closed` unconditionally), N106-28 (the Stop's `end_open_at` site keyed). N106-27 (`Ended` arm keyed `resumed_part`) is killed by W5, not here (Sol major 3).
- **W4b (a switch that replaced a paused repeat is a booked record).** `gap_rig`: A paused; switch S replaces it at R (`first && paused`); S dropped (Underflow) → part P of S resumes; P dropped again → P2; `stop`. Expect `bursts` targets `[A, S, P, P2]`, ends `[discontinuity, discontinuity, discontinuity, stop]`, `omitted == {}` (A and S are booked; P is the first `resumed_part`, kept at N = 1). Wrong: `is_resume_part` without `!first` (S counted as a part: P is then omitted). Row: N106-24.
- **W4c (the pinned open case: a part closed by a switch).** A → P1 → P2 (two Underflow resumes), paused; a switch replaces the repeat at R3; the switch part is sent. Decision of this plan (OQ-3, decided by the orchestrator, may be reversed): P2's record, opened by a resume and closed by the switch's start of burst, **is** bounded (`resumed_part`), because the tracker cannot tell what closed it and one bool is the whole rule. With N = 1: `bursts` targets `[A, P1, S, …]`, `omitted.bursts == {"resumed_part": 1}` (P2). If the orchestrator rules "kept", W4c flips to `omitted == {}` and the predicate gains a second bool; nothing else changes.

- **W4d (the closing record of an abandon, saturated; Sol major 2).** Chain as W4a with N = 1 (exhaust `resumed_part`: booked A, three Underflow resumes), then the repeat ends by sequence loss in a part (`SeqError`) or by a failed send (`FailSend`), the two writers at `tx.rs` ~:1177 (`abandon`). Expect: the last record in `bursts` is the part's closing record with `end` an `abandon`-kind end (not dropped), `omitted.bursts == {"resumed_part": 1}` and nothing more; one test per trigger. Wrong: the abandon's `tracker.stop()` push keyed `resumed_part` (the closing record vanishes because the budget is spent). Row: N106-37 (the abandon writer keyed `resumed_part`).
- **W4e (the closing record of a failed reopen, saturated).** Same chain, the repeat's recovery fails (the `mt3d_a_recovery_that_fails_ends_the_repeat_and_the_clock` rig) after the budget is exhausted: the last `bursts` record is kept. Row: N106-38 (the failed-reopen writer at `tx.rs` ~:973 keyed `resumed_part`).

**W9 — a booked switch joined onto a resumed part's open device burst keeps its ACK (Sol major 1; sub-step B implements the exemption, A plans it).** mt3d rig, N = 1. Ops: booked A, then enough Underflow resumes to exhaust the ordinary `BurstAck` budget (the part ACKs are unmatched, bounded); resume into part P; book switch S so that it joins P's open device burst (S's target is added to the same correlation, `tx.rs` ~:365-368,787-790); close (Stop at or after S's start); read the ACK for each channel. Expect: S's `BurstAck` is kept for every channel and not counted in `omitted.async.BurstAck`; the correlation is booked because **any** of its targets is booked, not because of how it was opened. Wrong: the correlation keeps `part == true` from P (S's ACK bounded); the exemption looks only at how the correlation was opened. Row: N106-39 (the joiner's classification not updated: a correlation that gained a booked target keeps `part`).

**W5 — Session: the selector reaches the Records; `record_bound` is written** (`fake.rs`).
- `ur_30_the_sections_are_written` gains `record_bound`: present on a receive-only default Run (`receive_run`) with `rows_per_kind == 1000` and `omitted == {}`; `stats` carries `tx_resumes` and `tx_gap_samples` as `i64` equal to 0 (M1).
- New `ur_30_a_run_past_the_bound_says_what_it_left_out`: `tx_session_n(FakeConfig { faults: vec![TxReport(2_100 ms, TxCode::Other(7)); 6] }, 3)` (selector `record_rows_per_kind: 3`); `wait`; `finish`. Expect: `async` has exactly 3 rows with `code == "Other(7)"` (plus any other kinds' rows); `record_bound == {"rows_per_kind": 3, "omitted": {"async": {"Other(7)": 3}}}`; `rows_per_kind` differs from the default so a constant fails; the arrays keep their shape (`bursts(&manifest)` still deserialises as `Vec<BurstRecord>`). Wrong: N not read from the binding (stays 1 000: nothing omitted); `Core::new` given the default; `rows_per_kind` hard-coded. Rows: N106-15, -32, -36.
- New `ur_30_a_clean_run_with_more_than_n_booked_bursts_keeps_every_booked_row`: `tx_session_n(default, 3)`; 8 timed one-shot `send`s 10 ms apart; `wait`; `finish`. Expect 8 `BurstAck` rows with `tx_flush != true` (exempt), 8 burst records, `omitted == {}` (the open's T3 flush ACK is `BurstAck:flush`, one per channel, under N). Wrong: booked ACKs bounded (3 rows); booked records bounded (finite bursts' `Ended` records: N106-27). Rows: N106-22, N106-27.

**W6 — wiring of the writers.** Every writer pushes through the one type; this is checked by the compiler once `Records.{bursts, device_async, rejected, timing}` expose no `push` of their own (`Bounded::push` takes a `Key`), so the 12 production writers (`core.rs:1038,1042,1239`; `tx.rs:819-827,860,974,1178,1401`) cannot bypass the count. No behaviour test beyond B/K/W.

**W7 — Session, a repeat every part of which is dropped late, N = 2** (`fake.rs`, `ur_30_a_repeat_the_device_drops_late_every_time_keeps_its_first_and_closing_records`; the same SlowSend fake as L11). Ops: `tx_session_n(SlowSend(20 ms), 2)`, `start_repeat` at 8 ms, `wait(400 ms)`, `finish`. Expect (no cycle count): `stats.tx_resumes == (kept tx_resume rows) + omitted.timing.tx_resume`; `bursts.len() + omitted.bursts.resumed_part ∈ {K, K + 1}` with `K = stats.tx_resumes` (the last pause may have no part yet); the first record's target is the booked start and the last record's `end == stop`; the kept `TimeError` rows are exactly 2 (N = 2) and `omitted.async.TimeError ≥ 0` (a per-report count is asserted only if the fake exposes one; INFERRED that it does not). The virtual-time floor of OQ-7 applies (`K ≥ 4`, so that `omitted.bursts.resumed_part ≥ 1`).

**W8 — one Action never loses its outcome**: covered by W1 (Stops), W5 (booked ACKs and records), B9 (`applied`, `switches` complete). A Session sweep of > N `UpdateParameter`s is **not** added: `applied` and `switches` are `Vec<Json>` and the type system, not a test, keeps them out of the bound; B9 asserts the shape.

## 8. Selector (S) and Manifest (M)

**S1 — default and accepted values.** `ur_05_the_record_rows_per_kind_default_is_1000` (via `ur_30` of M1: `rows_per_kind == 1000` on a Run that sets nothing); `from_binding` accepts `1` and `100000` (`ur_05_from_binding_refusals` gains the ok cases); a Run with `7` writes `record_bound.rows_per_kind == 7` (W5 uses 3). Rows: N106-31 (default constant), N106-34 (key missing from the allow-list: `unsupported selector key`).

**S2 — refusals, before any device call.** In `ur_05_from_binding_refusals` (`fake.rs:114`) the selector list gains `record_rows_per_kind` = `0`, `-1`, `100001`, `1.5` (Float), `"1000"` (String), `[1000]` (List), `true` (Bool): each `Err(Rejected)` with the exact text ``"UR-5: selector `record_rows_per_kind` must be an Int in 1..=100000"``, and, new, `device.calls()` is empty after each refusal (the fake logs `serial`, `device.rs:1366`, so the key is checked before `serial()`: the same order as `block_len`). Rows: N106-29 (`..=100_000` → `..=100_001`), N106-30 (`1..` → `0..`), N106-33 (`let _ = device.serial();` inserted before the key's parse).

**S3 — the pure parse.** `record_rows_per_kind(&binding)` unit tests next to it (`mod.rs`): missing → `Ok(1000)`; the seven refusals above; `Int(1)`, `Int(100000)` → ok. (Same cases as S2 without a device: the function is what `from_binding` and the catalogue share.)

**S4 — the server surfaces it as `EA-7: <name>: …`** (`crates/ezsdr-server/tests/protocol.rs`, beside `ea_07_the_reopen_can_be_switched_off`; new `ea_07_a_bad_record_rows_per_kind_is_refused_with_the_ur_5_text`): profile from `uhd_profile(&temp.0)` with `["bindings"]["radio"]["selector"]["record_rows_per_kind"] = json!(0)`; `Connect` → `ErrorKind::Refused`, `message == "EA-7: radio: UR-5: selector `record_rows_per_kind` must be an Int in 1..=100000"`. EA-16 check (brief): no new Python surface and no new Session error class; the refusal is the same `connect` → `ServerError("refused", …)` as every UR-5 selector refusal, so EA-16 needs no change. EA-7's sentence that the server "opens the device … and builds the Provider from the binding and that device" is true only for the **order**: see OQ-1.

**S5 — before any device open (OQ-1, recommended seam B).** The same test asserts `opened.load() == 0` (the closure counts opens, as `ea_07_the_reopen_can_be_switched_off` does with `opened == 1` today). This needs the catalogue to call `ezsdr_radio_uhd::record_rows_per_kind(binding)` before the open loop (`catalogue.rs:134`), with `built(&e)`. Row: N106-35 (the pre-check removed: `opened == 1`). If the orchestrator takes seam A (the same order as `block_len`: open, then refuse), S5 is dropped, `opened == 1` is asserted instead, and EA-7 is unchanged.

**M1 — Manifest.** `ur_30_the_sections_are_written` (`fake.rs:2862`): the suffix list gains `record_bound`; `record_bound["rows_per_kind"] == 1000`, `["omitted"] == {}`; the stats loop gains `tx_resumes`, `tx_gap_samples` (`is_i64`). **Schema:** `schemas/manifest.v1.json`'s `ModuleSection.content` is free (`"description": "Its content, which the Module defines"`, no `properties`), and `binding_profile.v1.json`'s `selector` is `additionalProperties: Value`; so **no schema is regenerated** and `schema_freeze` stays as it is (run at the gate; no `SCHEMA_CHANGELOG` entry before the v4.0 freeze, AGENTS §6). Python reads no Provider section (`grep -rn "radio.uhd\|\.section" python/` finds only a PPS string, Fable §5).

**M2 — MockRadio is unchanged.** Confirmed by the search the draft names, re-run for this plan: `grep -n -i "resum\|stall\|stutter\|underflow" crates/ezsdr-mock-radio/src/*.rs` finds only `resumes_at: None` (`lib.rs:980,1905`); `grep -rn -i "resum\|stall\|stutter\|paused" crates/ezsdr-acceptance/tests/oracle/` finds nothing; `grep -rn "\.pause(\|resume_at(\|Paused" crates --include=*.rs` outside `staging.rs` and `ezsdr-radio-uhd` finds nothing. No Mock test or row is added.

## 9. Mutation rows

`plan/maintenance/tools/mutations.json` (maintenance) and `plan/phase7/tools/mutations.json` (p7). Rows run only through `remote_mutate.py`. **Counts (v2): new 39 (38 firm + N106-35 which depends on OQ-1; N106-37, -38, -39 added by Sol's majors 1-2), retired 10, retargeted 5 (MT3D-55 now to the staging test), re-anchored 8 (MT3D-26 is in both lists, so 22 distinct existing rows change). Sub-step A: the L and C rows (N106-01…06), retirements, retargets, re-anchors of the lifecycle code; sub-step B: the rest.**

### 9.1 Retire (10): they mutate removed code
| Row | Why |
|---|---|
| MT3D-10, -11, -12, -13 | b′ (`played`, `played < stall`, the stall's measure, always-ends); anchors and killer vanish |
| MT3D-27, -28 | the newest-entry stall rule and the "ours" test of a running part |
| MT3D-53 | its `new` text uses `.stall` (would not compile) and the behaviour it pins ("a move leaves the stall unchanged") is gone |
| MT3D-79, -80 | `Paused.source` ("ours") |
| MT3D-81 | `let source = self.device.first();` (`pause`'s source) |

### 9.2 Retarget the killer (5)
| Row | Mutant | New killer | Reason |
|---|---|---|---|
| MT3D-15 | `TxBook::cursor` of a paused booking = its resume point | `mt3d_a_lost_sequence_while_the_repeat_is_paused_ends_it` (the `book(&mut rig, 7, 1, 21_000 + 5_000)` footprint-H+5 assertion) | its old killer no longer ends the repeat (INFERRED) |
| MT3D-37 | a lost sequence's abandon of a paused repeat closes a device burst there is none of | the same test (`tx_sends` unchanged after the report) | its old killer no longer reaches `Ended` |
| MT3D-38 | the late report's abandon of a paused repeat closes a device burst there is none of | `mt3d_more_that_ends_…` first block with the added `closing_sends == 1` (E5) | **not** Fable's one-shot test (equivalent mutant there) |
| MT3D-55 | `replace_paused` drops `paused` | `txbook_a_switch_replaces_a_paused_repeat_and_its_boundary_moves_with_the_resume_point` (`crates/ezsdr-radio/tests/staging.rs` ~:3179-3190, which asserts `Some(Paused { … })` after `replace_paused`); L6 as a second killer; **never retired for a survivor** (MT3D-54 mutates `first`, not `paused`: it does not pin this; Sol minor 5) | the old killer is rewritten; the behaviour stays reachable |
| MT3D-26 | the part's device burst is not remembered (re-anchored too) | L2 (no correlation: no event) | |

### 9.3 Re-anchor (8)
| Row | New anchor text |
|---|---|
| P350 | the `PendingReport { … resumed: stall }` literal → `part: bool` |
| MT3D-26, B25, B26 | `remember_part(…, paused.map(|paused| paused.stall))` → `remember_part(…, is_resume_part(first, paused.is_some()))` (three call sites `tx.rs:730,746,787`) |
| MT3-16, MT3D-09 | `self.dropped(reached.last(), …)` loses its first argument |
| P281 | the selector allow-list chain gains `"record_rows_per_kind"` |
| P1209 | `lock(&self.core.rec).bursts.push(serde_json::to_value(record)…)` → the keyed push |
After rustfmt, `rules.py` must still find every `old`/`old2` exactly once (§9 closing rule, `--base 5b8059c7`).

### 9.4 Keep (checked against the planned text; no edit)
MT3D-08, -19, -20, -21, -22, -24, -32, -33, -34, -36, -39, -42, -51, -52, -54, -56, -58, -59 (their staging test only changes the `Paused` literal, `source`/`stall` removed), -64, -69, -70, -71, -72, -73, -77, -78, MT3-15, U28, U43, R13, B40, N48S1-05, N92-2, N92-7b, P277, P1103-P1106, P1191, R07. MT3D-58/59's killer `txbook_a_switch_replaces_a_paused_repeat_and_its_boundary_moves_with_the_resume_point` stays; MT3D-69's mutant (`paused.planned = 0`) is unaffected.

### 9.5 New rows (anchor = the smallest unique token text once the code exists; the implementer submits them with the code, §9 step 2)
| Row | Mutant | Killer |
|---|---|---|
| N106-01 | `match self.clock.filter(|_| resumes && self.resumable())` → `… && !self.open_part` (a part's drop ends the repeat) | L1, L2, L10 |
| N106-02 | `if !hit { return Dropped::Nothing; }` → `if false {…}` | L4, L7 |
| N106-03 | `Dropped::Nothing => reached.last().and_then(|reached| reached.resumes_at)` → `reached.first()` (same old as MT3D-39, different `new`) | L3 |
| N106-04 | `_ => reached.resumes_at,` (TimeError arm) → `_ => None,` | L3 |
| N106-05 | `stat("tx_resumes", 1)` removed | C1, C3 |
| N106-06 | `stat("tx_gap_samples", planned - handed)` → `planned` | C1, C3 |
| N106-07 | `Bounded::push`: omitted counter not incremented | B1, B6 |
| N106-08 | `>=` ↔ `>` at the keep decision | B1, B8 |
| N106-09 | one kept counter per section instead of per kind | B2, B3 |
| N106-10 | `Key::Exempt` rows tracked | B4 |
| N106-11 | the 64-kind cap removed | B5, K5 |
| N106-12 | over-cap rows neither kept nor counted | B5 |
| N106-13 | `written()` puts `timing`'s omitted under `async` | B9 |
| N106-14 | `record_bound` written only when `omitted` is non-empty | B9, M1 |
| N106-15 | `rows_per_kind` = the default constant | B9, W5 |
| N106-16 | `tx_stop_applied` removed from `TIMING_EXEMPT` | W1 |
| N106-17 | `rx_segment` (or another of the four) removed | K2 |
| N106-18 | `rejected_key` uses the whole `reason` | K3 |
| N106-19 | `rejected_key` drops the rule id | K3 |
| N106-20 | `rejected_key` ignores `call` | K3 |
| N106-21 | the flush ACK shares the `BurstAck` key | K1 |
| N106-22 | `booked` forced false (a booked ACK bounded) | W2, W5 |
| N106-23 | `booked` forced true (a resume's ACK exempt) | W2 |
| N106-24 | `is_resume_part`: `paused && !first` → `paused` | K4, W4b |
| N106-25 | `open_part` assigned before `record(step)` | W4a |
| N106-26 | the `Discontinuity` arm bounds `closed` whatever `open_part` | W4a |
| N106-27 | the `Ended` arm keys its record `resumed_part` | W5 (eight finite bookings, N = 3: the booked records of a finite burst would be bounded; W4a never executes the `Ended` arm: Sol major 3) |
| N106-28 | a `tracker.stop()` push (`end_open_at`) keyed `resumed_part` | W4a |
| N106-29 | `1..=100_000` → `1..=100_001` | S2 |
| N106-30 | `1..=` → `0..=` | S2 |
| N106-31 | default `1_000` → another value | M1 |
| N106-32 | the selector's value ignored (default used) | W5 |
| N106-33 | `let _ = device.serial();` before the key's parse | S2 (`calls()` empty) |
| N106-34 | `"record_rows_per_kind"` removed from the allow-list | S1, W5 |
| N106-35 | the catalogue's pre-open check removed (seam B only) | S5 |
| N106-36 | `Core::new(…, rows)` given the default | W5 |
| N106-37 | the abandon writer's closing record keyed `resumed_part` (saturated, W4d) | W4d |
| N106-38 | the failed-reopen writer's closing record keyed `resumed_part` (saturated, W4e) | W4e |
| N106-39 | a correlation that a booked switch joined keeps `part` (its ACK bounded) | W9 |

## 10. Tests the change makes false, and the audit

### 10.1 Tests of the lifecycle (draft §4.3, Fable §4)
| Test (file) | What changes | Disposition |
|---|---|---|
| `mt3d_a_part_that_plays_less_than_the_stall_part_ends_the_repeat` (`mt3d.rs:226`) | all four cases resume | **rewrite** → L1 |
| `mt3d_a_delayed_underflow_that_retires_two_entries_applies_the_stutter_rule` (`:469`) | no `ends`; late sub-case keeps R2 | **rewrite** → L3 (the two-geometry loop goes: it separated P1 from P2, which no longer exists) |
| `mt3d_a_second_drop_found_in_the_gap_abandons_at_the_handed_cursor` (`:258`) | the TimeError no longer ends | **rewrite** → L8; "abandon at H, no stray close" lives in E1 |
| `mt3d_two_channels_late_resume_once_in_either_order_and_the_part_ends_on_one_channel` (`:370`) | last block flips | **rewrite** → L4 |
| `mt3d_a_switch_part_keeps_the_stutter_bound_of_the_gap_it_replaced` (`:1091`) | P − 1 resumes | **rewrite** → L6; `design/07` RM-37 and `design/18:280` test lists rename it |
| `mt3d_a_stopped_repeat_s_delayed_report_does_not_end_a_newer_repeat_that_waits_to_resume` (`:1263`) | the guard it pinned is removed; assertions still hold | **keep**, comment reworded (L7) |
| `mt3d_a_switch_the_gap_moved_and_a_report_then_resumed_is_told_from_its_receipt_boundary` (`:1199`) | comment "played at least the stall part" is stale; assertions hold | **keep**, comment reworded |
| `txbook_a_pause_remembers_the_first_instant_of_the_device_burst_it_closed` (`staging.rs:3226`) | `Paused.source` gone | **retire** |
| `txbook_a_paused_repeat_keeps_its_handed_cursor_and_its_plan_when_its_resume_point_moves`, `txbook_a_switch_replaces_a_paused_repeat_and_its_boundary_moves_with_the_resume_point` (`staging.rs:3165,3179`; fixture `:3154-3160`) | `Paused { handed, planned, lattice }` literal, `resume_at(50)` | **retarget** (shape only; assertions on `handed`, `planned`, boundary unchanged; MT3D-58/59 keep killing) |
| `ur_28_the_device_reports_a_late_burst` (`fake.rs:2577`) | b′'s zero case (`late.len() == 2`, `late[1].resumes_at == None`) | **rewrite** → L11, name kept (U28) |
| `ur_22_a_repeat_the_device_reports_late_resumes_on_its_lattice`, `ur_22_a_late_send_asap_repeat_resumes` | one scripted/first-send drop only | **keep** |
| `mt3d_what_still_ends_a_repeat`, `mt3d_a_lost_sequence_while_the_repeat_is_paused_ends_it`, `mt3d_more_that_ends_…` | none (E1, E2, E5) | **keep**; one assertion added (E5) |
| `ur_30_the_sections_are_written` (`fake.rs:2862`) | gains `record_bound`, two stats keys | **fixed** (M1) |
| `remember_burst` callers (43 sites, `tx.rs` tests and `mt3.rs`) | unchanged: the shorthand stays | **keep** |
| the e2b/e3 golden snapshots (`e2b.rs` 16 `"stats": {…}` lines, `e3.rs` 41 `stats …` lines) | `stats` always gains `tx_gap_samples`, `tx_resumes` (OQ-2) | **fixed** mechanically (`tx_errors … tx_gap_samples=0 tx_resumes=0 tx_samples`, key order is the `BTreeMap`'s); `snapshot()` (`e2b.rs:42`) and `e3.rs:54` need `Bounded: Serialize/Debug/Clone`; `c2.rs:234` (`lock(&rig.core.rec).timing.clone()`) compiles through `Clone` |

### 10.2 Readers that count rows (AGENTS §9 step 1 audit table)
Columns: test/reader — expectation — **verdict**. "Below N": every named Run today holds far fewer than 1 000 rows of one kind (the measured T200 trials: ≤ 14 underflows × ≤ 48 `TimeError` ≈ 670 rows of one kind, Fable M3); a reader that must stay right past N reads `kept + omitted` or asserts `omitted == {}`.

| Reader | Expectation | Verdict |
|---|---|---|
| `tests/common/iii.rs:33-38` `run_faults` (`async_faults`) | count of fault rows | still right below N; **cannot tell** for a stutter Run past N (a count of kept rows) → OQ-6 guard |
| `common/measurement/multichannel.rs:874-912` | `BurstAck` per channel, `async_failures`, `burst_count` | still right: booked ACKs exempt; failures only need ≥ 1 |
| `common/measurement.rs:3085-3165` `tx_throughput_evidence` (+ call sites `3219-3221,3372-3376,3686`) | `burst_count == 1`, `burst_ack_count == 1`, failures any | still right: clean Run keeps one booked ACK and one record; a resuming Run is `DeviceFailure` on `underflow_events > 0` whatever the row count |
| `common/n47s5.rs:177,798-805` | `BurstAck` counts (`acks != 1`) | still right: booked ACKs exempt, flush ACKs not counted (`tx_flush != true`) |
| `common/e3.rs:55,111` (`flush_evidence`) | flush ACKs and `tx_flush` rejections | still right: `BurstAck:flush` and `tx_flush` have their own budgets (K1, K3) |
| `common/mod.rs:600-606` | any `tx_resume` row / `timeline expired` note | still right: the first row of a kind is kept |
| `common/measurement/h4.rs:283-328` `repeat_resumed` | any `tx_resume` row or `late_at_device` event | still right |
| `common/s4bench.rs:280-300` `t200_run` | `async_failures` list, `underflow_events` | still right (class and counts from events; the list length is informational past N) |
| `hardware.rs:2090-2121` `hw_h5_held_last_sample` | `burst_record_count`, `count_report_codes(async)` | still right: 200 booked bursts, all kept; rehearsal: the H5 shape test runs on the fake in the default suite (no rule is computed from the row counts) |
| `hardware.rs:1382,2226,2401` (print the `async` section) | printed | still right |
| `crates/ezsdr-acceptance/tests/parity.rs:1222-1478` (`manifest_rows(..).len()`, `kind == "tx_burst"`) | empty or equal counts for Mock and UHD | still right (single-digit counts) |
| `crates/ezsdr-acceptance/tests/uhd.rs:175,501` | typed `Vec<BurstRecord>`, `rows.len() == 1 / 2` | still right (the array keeps its shape; booked records kept) |
| `crates/ezsdr-server/tests/protocol.rs:1627,1651` | one-off `timing` rows | still right |
| `tests/fake.rs` (`ur_22_a_repeat_…`, `ur_28_*`, `time_errors`, `bursts`) | counts of one resume | still right below N; L11 rewritten |
| In-crate tests that read `lock(&core.rec).{rejected, device_async, timing, bursts}` (`core.rs:1906-2484`, `tx.rs:2283-2285,5893-5996`, `mt3d.rs`, `e1.rs` `rows()`, `rx.rs:1725`) | `.iter()`, `.len()`, indexing | compile through `Deref<[Json]>`; verdict unchanged |
| the hardware tests `hw_h1…h4`, `hw_tx200_hot_path_probe`, `hw_iii_load_200msps` and their fake rehearsals (`measurement_fake_rehearsal_*`, `h4.rs:358`) | none computes an expectation from a row count of a resuming repeat | still right; **no `#[ignore]` hardware test's expected value comes from the changed rule**, so no new rehearsal is required (AGENTS §9, bench values) |

Every row verdict was read from the code, not run. The raw-evidence caveat of AGENTS §8 applies: "still right below N" rests on the T200 log counts in the draft §5 (VERIFIED there), not on a new measurement.

## 11. Texts (T) — substance only; the wording is the implementing diff's

`rules.py --base 5b8059c7` must pass; each edited rule keeps its `*Test:*` line and gets a Changes row; no rule id is renumbered; versions untouched.

| T | File | Change |
|---|---|---|
| T1 | `design/18` UR-22 (`:266-267`) | Replace the b′ paragraph: "An underflow or a late report that reaches a repeat's part resumes it again, as it did the first device burst; a report read while the repeat waits to resume changes nothing but its event; a sequence-loss report (also in the gap), a failed or short send, a failed reopen, a lost device and an `R` or a switch boundary that does not fit the clock end it; a burst that is not a repeat is never resumed." Delete the sentence on the stall part under "A resume point that is missed moves." Add the rate bound with its step: consecutive resume points are ≥ `m` + 32 samples apart (`R(n+1) ≥ busy + m ≥ R(n) + 32 + m`, the part's first send of ≥ 32 samples moving `busy`), so a stream resumes at most 100 times a second. `*Test:*`: the renamed/new names of L1–L11, E5. Also `design/18-uhd-radio.md:571` (the §6 test-table cell that promises the stall part remains unchanged) and the `design/18` test-table rows that name removed or renamed tests (sub-step A, Sol minor 8). |
| T2 | `design/18` UR-28 (`:352,354-355`) | "records every asynchronous report in the `async` section" → "…within UR-30's bound"; drop "and the newest decides the repeat by UR-22's stutter rule"; "`resumes_at` is carried once per resume (the newest burst's, by the report that made the resume or first reaches a burst whose resume the expired check made); its absence does not mean the repeat ended (a second channel's event, a stale report)". `*Test:*`: L3, L4. |
| T3 | `design/18` UR-30 (`:370-374`) | The bound paragraph: first N rows of each kind per Run (kinds as §11.1/K1–K3; exempt names; ≤ 64 kinds per section, then `other`; count before keep; `applied` and `switches` complete; `bursts`: only intermediate parts, kind `resumed_part`; booked bursts' records, every repeat's closing record and a booked burst's `BurstAck` always kept). `record_bound = {rows_per_kind, omitted}` always written. `stats` gains `tx_resumes`, `tx_gap_samples` (Σ planned R − H, a lower bound when R moves; exact count of resumes). Fix the two clauses that become false: "`rejected` (every `COMMAND_REJECTED` reason, …)" and "the `bursts` section then holds one record per part" gain "within the bound". `*Test:*`: B1–B9, K1–K5, W1–W5. |
| T4 | `design/18` UR-5 | Selector table row `record_rows_per_kind` — int, 1…100 000, default 1 000 — "rows of each kind the Module keeps per section (UR-30)"; `*Test:*`: S1–S3. If seam B is taken, one clause that the key is checked before the device opens (and EA-7, T11). |
| T5 | `design/02` SC-25 | "…and what the hardware radiated during the gap is recorded in the Provider's Manifest section" → the accepted distinction: every resume is counted (`stats.tx_resumes`); a drop the device reports is told by an event; the gap is recorded within the Provider's stated bound in its Manifest section (spec 18 UR-30). Never "told by the event" alone (an expired-check resume has no event). Changes row. |
| T6 | `design/02` SC-28 | one clause: "a Provider that resumes a repeat may leave out its intermediate parts' records past a bound its spec states, counting them (spec 18 UR-30)". Changes row. |
| T7 | `design/07` RM-37 test list (`:368`) | `mt3d_a_switch_part_keeps_the_stutter_bound_of_the_gap_it_replaced` → `mt3d_a_switch_part_resumes_like_any_part`; Changes row. RM-11/RM-22 comment "absent unless a repeat resumes" stays true. |
| T8 | `design/18` Changes table | One new row (UR-5, UR-22, UR-28, UR-30; SC-25, SC-28): "#106: b′ goes (a repeat resumes after every underflow or late report); the Module's per-Run records are bounded per kind; `record_bound`, `tx_resumes`, `tx_gap_samples`, `record_rows_per_kind`". The 2026-11-14 row ("a part that plays less … ends the repeat") is a history row and stays. Names the owner decisions it supersedes (draft §4.1: M4, the b′ items of "Triggers/endings" and REPEAT RESUME's one cap, O5, "Missed R", "KL and bench"). |
| T9 | `KNOWN_LIMITATIONS.md` KL-13 (`:127-132`) | Symptom/Workaround/Status: "the repeat keeps dropping and resuming until it is stopped" — **never** the word "stutter"; stop the stream when the first `TX_UNDERFLOW` arrives; replace "a host that cannot keep up ends it" (`:130`) and "A part that plays less than the stall part … ends the repeat" (`:132`) by the list of what still ends it; each resume a gap ≥ 10 ms, at most 100 a second per stream; **m7**: a repeat whose every part the device drops late transmits nothing and keeps resuming, each cycle one `TIME_ERROR { late_at_device }` (INFERRED on hardware until bench cell C7's late-start cell); **m8**: more resumes are more two-zero closes, so more chances of the three-attempt T3 end that kills the stream's clock (INFERRED, unmeasured); `resumes_at` once per resume, absence ≠ ended; every resume is counted in `stats.tx_resumes`. |
| T10 | `KNOWN_LIMITATIONS.md` KL-39 note (`:158`), KL-5 Workaround (`:48`), KL-45 | KL-39: drop "a part that played less than the stall part of the gap before it" from the list of ends. KL-5: replace "(two close together can still end it: KL-13, issue #106)" by "(it keeps resuming after every underflow, each a recorded gap: KL-13)"; Status gains one sentence that a repeat at 200 MS/s on this host resumes after each underflow (14 per 300 s on #1). KL-45: one line — its `TimeError` rows are counted by the record bound (KL-46). |
| T11 | `KNOWN_LIMITATIONS.md` new **KL-46** | Symptom: the Manifest keeps the first N rows of each kind of `async`, `timing`, `rejected` and of a repeat's intermediate parts' records; the rest are counted in `record_bound.omitted`. When: a Run with more than N rows of one kind (N = 1 000 by default): a repeat that resumes thousands of times, a long flood of reports. The `TimeError` figure (m3): a drop sends 1–48 `TimeError` rows (mean ≈ 13), so a drop-heavy Run exhausts that kind after about 21–75 drops (≈ 27 minutes at the measured 14 resumes per 300 s). Per **Run**, not per stream (m6): one stream's flood uses the rows a later, unrelated episode would have had. Workaround: raise `record_rows_per_kind` (1…100 000) in the binding's selector before a long Run; read exact counts from `stats.tx_resumes`/`tx_gap_samples`, `events.counters` and `record_bound.omitted`. **Not bounded:** the Kernel's delivered events and marks keep growing by about one per resume per channel (Q1; the event list's own bound is on the freeze-review list); long Runs stop at the first `TX_UNDERFLOW`. Evidence: UR-30, the tests of §5–§8, `plan/phase8/` bench. Status: Ceiling (owner, 2026-11-17). Entry number 46 is unused (highest 45). |
| T12 | `plan/phase8/bench.md:1583` (C7) | "cap に達しない" → the repeat resumes after every drop (no cap); C7's late-start cell is also the check for m7. (Not `handoff.md`: the orchestrator adds the freeze-review item.) |
| T13 | doc comments | `staging.rs:383-384,389,786`; `tx.rs:102,115,1103,1119-1121`; `crates/ezsdr-radio/tests/staging.rs:3152` lose "stall"/"stutter". |

### 11.1 Kind strings the Manifest shows (pinned by K1–K3, W-tests)
`async`: the code (`"Underflow"`, `"TimeError"`, `"SeqError"`, `"UnderflowInPacket"`, `"SeqErrorInBurst"`, `"BurstAck"`, `"Other(<n>)"`); a flush's acknowledgement `"BurstAck:flush"`. `timing`: `what`. `rejected`: `call`, else `<kind>:<rule id>` (`"tx_burst:UR-22"`) or `<kind>`, else the identifying field's name, else `"other"`. `bursts`: `"resumed_part"`. Overflow beyond 64 kinds: `"other"`. Exempt (never tracked): `tx_stop_applied`, `rx_segment`, `first_rx_block`, `rx_stop_untimed`, `rx_stop_late`; a booked burst's ACK; every record that is not an intermediate part.

## 12. Open questions, each with a recommendation

1. **OQ-1 "before the device opens" (decision F-M3).** `catalogue.rs:134-146` opens the device before `UhdRadio::from_binding`, and `block_len` is refused only after that open today; so "through the same UR-5 selector path before any device open" cannot hold as written. **Recommend seam B (narrow):** the pure `record_rows_per_kind(&Binding)` is shared by `from_binding` and called by the catalogue before the open loop; S5 asserts `opened == 0`; EA-7 gains one clause (T4/T11 note). Alternative A: refuse after the open, like `block_len` (cost: one wasted several-second open on a typo; EA-7 unchanged; S5 becomes `opened == 1`). If the owner's sentence meant "before the Run starts", A is enough.
2. **OQ-2 the two stats keys: always present or written at the first resume.** "Fixed keys" read as always present (readers never handle absence). Cost: ~57 golden lines in `e2b.rs`/`e3.rs` (mechanical). Alternative: write at first use, as `tx_lock_wait_max_ns` is (no golden change; absent means 0). **Recommend always present.**
3. **OQ-3 a part closed by a switch that replaced the paused repeat** (Fable's open case): bounded or kept? **Decided here (orchestrator may reverse): bounded** — one bool, the tracker cannot tell what closed a record; pinned by W4c. Switches are Actions, so the extra loss per switch is one record of an already-bounded kind.
4. **OQ-4 flagged non-ACK async rows** (`tx_flush: true` on an `Underflow`/`TimeError` of an unacknowledged flush): the decision names only the flush's `BurstAck`; planned literally (those keep their code's key). Fable m2 would key every flagged row `<code>:flush`. At most one row per flush channel per attempt, so **recommend the literal reading**.
5. **OQ-5 `PendingReport.part` and `remember_burst`.** The decision's booked-ACK key needs one bit per correlation; it replaces `resumed`. Therefore `remember_burst`/`remember_part` are **not** merged (draft §4.5/Fable §4 assumed `stall` was the only difference). `retire` returns whether an `Ack` matched a booked entry. No other new state.
6. **OQ-6 guard against silent truncation in tests.** Fable's cheaper-than-the-audit idea: `tests/common/mod.rs::section()` fails when `record_bound.omitted` is not `{}`, with an opt-out for the bound's own tests; and the hardware profile builder for `hw_tx200_hot_path_probe`/throughput stages sets `record_rows_per_kind` to 100 000 (≤ about 0.1 GB per flooded kind). Adopted. It does **not** make the audit moot: the throughput readers (`common/measurement.rs` ~:3218 and `common/measurement/multichannel.rs` ~:874) read `Manifest::section()` directly and either stay in the audit table or get the same completeness check; a synthetic test builds a Manifest whose `record_bound.omitted` is non-empty and asserts the guard fails, and that the opt-out passes (sub-step B, Sol minor 7).
7. **OQ-7 virtual-time floor in L11/W7** (`late.len() >= 3`, `K >= 4`): INFERRED stable; the 20-run check decides. **Recommend keeping the floor if 20/20 pass.**
8. **OQ-8 counters and row under one lock.** `stat("tx_resumes")`, `stat("tx_gap_samples")` and `core.timing(row)` are three adjacent `rec` locks, so a UR-16 detached writer could show an off-by-one at cleanup. **Recommend leaving them adjacent** (anchors of MT3D-21/22 survive; threads are joined before `write_sections` except a detached one, which `rejected` already names).
9. **OQ-9 builder.** The brief asks for a `with_measurement_timing`-style builder; the selector key is the public path and a builder would be unused API. **Recommend no builder.**
10. **OQ-10 MT3D-55's new killer** is the staging test (decided: never retired for a survivor; Sol minor 5).
11. **OQ-11 compound kind strings are user-visible** (`BurstAck:flush`, `tx_burst:UR-22`, `resumed_part`, `other`). Spelling is mine; KL-46 shows them. **Recommend as written;** the owner may rename before the diff.

## 13. Report

- Plan: this file. Sequences: **L 11, E 7, C 3, B 9, K 5, W 8, S 5, M 2 = 50** (W6 and W8 are by-construction notes with no test of their own, so 48 carry a test). Mutation rows: **new 36** (35 firm + N106-35), **retired 10**, **retargeted 5**, **re-anchored 8** (22 distinct existing rows touched).
- INFERRED: every expected number and every mutant→killer pairing (nothing was run), the fake's ACK-after-close behaviour used by W2/W3 (`device.rs:866-873`), the X300's behaviour after a late-dropped part (draft §9, unmeasured), the cycle floors of L11/W7, MT3D-15/-37/-38/-55's new killers, and the cost figures of OQ-2/OQ-6.
- Not checked: whether `FakeDevice::set_time` may be set forward repeatedly across L10's cycles; whether W7's fake exposes a per-report count; the mutation lists' `rules.py` anchoring after rustfmt.

## 14. Deviations found while implementing sub-step B (implementer, 2026-11-17)

- **Where the code is.** The bound lives in `provider/core/records.rs` (`Bounded`, `RowKey`, `Records`, `timing_key`, `rejected_key`, `async_key`, `burst_key`); `Records.written()` builds every section and `record_bound` from one borrow; `Core::new` takes `record_rows`; `UhdRadio.record_rows` comes from the pure `ezsdr_radio_uhd::record_rows_per_kind(&Binding)` (OQ-1 seam B: `from_binding` and the catalogue share it; `from_binding` calls it before `serial()`). `remember_part(target, part)` is production, `remember_burst` is again the `#[cfg(test)]` shorthand; `Tx.open_part` is assigned after `record(step)`; `join_burst` clears `part`; `retire` returns `(Vec<Reached>, booked)`; `Tx::report` pushes every non-`BurstAck` row before the `match` (a tickless `TimeError` returns early) and a `BurstAck` row after `retire` (no row is written between).
- **W4c** as planned could not happen: a switch that joins a resumed part's open device burst arrives after the part's Kernel burst ended with an `END_OF_BURST` (the repeat's last send before the switch boundary), so the part's record is closed by `Ended` (kept), not by `Discontinuity`. A part closed by a switch's start of burst is the **gap** case: the part is dropped, the repeat waits for R2, and a switch booked in that gap replaces it there. W4c is that scenario (`mt3d_a_resumed_part_that_a_switch_replaces_is_bounded_like_any_part_a_resume_closes`): A and S kept, P1 counted. OQ-3 stands as decided.
- **W4d/W4e (fix round).** Both are built on `resumed_part()`, so the open record is a part's (`open_part == true`) and the `resumed_part` budget is spent synthetically (`Records::with_limit(1)` and one `resumed_part` row pushed by the test, which `real_rows` filters out). W4d: a `SeqError` while P is the open device burst ends the repeat by the abandon, whose closing record (P) must be kept. W4e: the same chain with the fake's `ShortSend(<ordinal of the close after the part>)` and `TxOpenFails(1)` (the ordinal is read from an identical rig): P is dropped, the close is short, the reopen is refused, and `halt_failed_reopen` writes P's record, which must be kept. N106-37 and N106-38 now mutate to `burst_key(self.open_part)`, which both tests kill. (Before the fix both tests closed only the booked record A, so such a mutant survived.)
- **W9 / W2 shortcuts.** W9 drives `join_burst` directly (the call the owner makes on a booked burst's first send) on a correlation with two channel positions, not a switch run through the fake; W2 and W3 feed `BurstAck` reports by hand where the plan used the fake's own acknowledgement flow (W3 does use the fake's close ACKs). W7 uses N = 1 so that the floor of 4 resumes makes `omitted.bursts.resumed_part >= 1` certain (with N = 2 it needs 5).
- **W3** reads four close acknowledgements for five resumes (the fifth lies ahead of the device's time): N = 2 keeps two and counts two. **C3 past the bound**: `bursts` keeps A, three intermediate parts and the closing record, and counts 3 `resumed_part` (7 parts handed, 6 closed by a later resume, the seventh is the closing record), not 4.
- **S4**: the server's text is `EA-7: radio: Rejected: UR-5: selector ...` (the `ModuleError` display, as every UR-5 selector refusal); design/16's EA-7 clause and Changes row say so. N106-35 anchors on the catalogue's pre-check, killer `ea_07_a_bad_record_rows_per_kind_is_refused_with_the_ur_5_text_before_the_device_opens`.
- **OQ-6**: `section()` of `tests/common/mod.rs` panics when `record_bound.omitted` is not empty; `section_unchecked` is the opt-out (W5, W7); `omitted_rows` is the check the two direct throughput readers (`measurement.rs` TX throughput, `multichannel.rs` `run_stage`) call, which return `Err`; the synthetic test of the guard and its opt-out is inside `ur_30_a_run_past_the_bound_says_what_it_left_out` (a real bounded Manifest) plus the pure `omitted_in` cases. The throughput selector (`throughput_selector()`, 100 000) goes to the two throughput profile builders (not the shared `bench_profile`, whose other callers keep 1 000).
- **Rows.** New: N106-07…N106-40 (34). Retargeted: N106-01 (`&& !self.open_part`). Re-anchored: P350, P281, P1209, MT3-07, MT3-12, MT3D-26, B25, B26 (and N106-36's `old` carries rustfmt's trailing comma). N106-17 removes a name from the other four of the exempt list (`rx_stop_late`); N106-16 `tx_stop_applied`. The plan's N106-14 anchor is the `vec!`'s tail (the mutant chains `record_bound` behind a filter). K4 is `mt3d_a_part_is_a_start_of_burst_after_the_first_send_while_paused`.
- **Texts.** UR-28's "within the bound" sits at "`Other` is recorded only." (the rule's first line was within 30 characters of the 1 500 limit); UR-5's and UR-30's long lines are split.
- **Fix round 2 (Sol major 1, owner decision).** A joined booked burst B on a dropped device burst gets its own `TIME_ERROR` with the same `resumes_at` as A's: the code stays, and the texts (UR-28, the Changes row, KL-13) say that an event that tells a drop carries the plan of the resume it caused, the Kernel bursts of one dropped device burst share it, a report retiring two entries carries only the newest plan, and resumes are counted by `stats.tx_resumes`. `mt3d_a_switch_that_joins_a_resumed_part_is_named_with_it` asserts both events' `resumes_at` equal the one planned R and that `tx_resumes` rose by one; new row N106-40 (the second target's event carries no plan) is killed by it.
