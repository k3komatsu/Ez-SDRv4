# Note 47 stage 5 — sub-step self-reviews (DeepSeek) and their rulings

# Self-review round (sub-step reviews 5a–5b-1, 5b-2a, 5b-3, 5c, 5d-U): the orchestrator's rulings
Reports: `selfrev/<name>.md` (the review of that commit range, file:line at the range's end commit; check each against HEAD).
All adopted unless marked. "Row" means a mutation row with its named killer, probed.

## Fix round 1 (behaviour and tests)
- F3 (5b-2a P2) `first_free`'s `joins &&` conjunct has no test: add the ideal-profile repeat sequence from the report as a pure test; it becomes N47S5-68's killer (or a new row), probed.
- F4 (5b-2a P2) `ordinary_k` has no test and no row: add `rm_16_ordinary_k_is_decide_s_threshold` (pure; the 390 625 S/s case, n = 512, where the old Mock form differs by one sample) and a row.
- F5 (5b-2a P3) `Bookings::prune` has no test and no row: a pure test (a chain at or past the prune point is kept; over-pruning would accept a booking meeting it) and a row.
- F10 (5b-3 P2) `move_start` returns Ok when no chain starts at `old` (a chain pruned while a stalled owner still holds its burst), so the moved burst plays untracked. Fix so the record always reflects the moved burst: when no chain starts at `old`, the moved region is clash-tested against every chain and the Stop region like any other, refused on a clash (the burst is dropped with TIME_ERROR refused, as 5b-3), otherwise inserted. Test with the report's sequence (A pruned while held, B booked on time, A dispatched late into B: A refused, B plays on time).
- F11 (5b-3 P2) the configure refusal's `drop_piece` (D27) is pinned only one way: add a booking inside a configure-refused burst's span (accepted after the refusal) and a row that deletes the `drop_piece` line.
- F12 (5b-3 P3) `ur_23_s3_refused_move_frees_its_front` lacks C6's value half: add the report's geometry (A [20 000, 60 000), B at 45 000 len 100, dispatch at 40 000 refused; a value at 45 500 refused by A's booked end).
- F13 + F14 (5b-3 P3) `move_start` holds a second copy of `accept`'s region decision, with an unreachable Stop-region clause: put the decision "does `[start, front)` meet any chain except these, or the Stop region" in one private helper that `accept` and `move_start` both call; keep the Stop-region term in the helper (no separate dead clause); state in one comment why the hand-over horizon (5h) does not apply to a move (the owner hands in start order, so no chain after the moved head is handed before it).
- F15 (5c P2) D24's refusal is coded twice and the only test drives the unreachable copy: write the reachable sequence from the report (A(1 000) at `s`; B(10) joined at `s+1 000` before `J(s+1 000)`; the owner stalls so B is dispatched as a late head) as a test of `dispatch_start` → `move_start`'s refusal; if `join_open`'s copy is unreachable from receipt, delete it with the evidence; retarget N47S5-61 to the reachable path.
- F20 (5c P3) the `debug_assert!` at the old `k < next` / `h < next` site: no panic in a release build and no negative count. If the state is unreachable, keep a `debug_assert!` and in release refuse the head (TIME_ERROR refused, its id) instead of computing a count; add a unit test that drives the guard directly if it can be reached on the test rig, else say why not.
- F21 (5d-U P2) `stream.playing` is set at a switch's receipt and not restored when the switch is refused, so the next switch uses the refused waveform's grid and predecessor. Fix: `playing` changes when the switch is installed (applied), not at receipt — on UHD, and check the Mock (it installs at receipt, so likely unaffected; say so). Test the report's sequence (repeat A; switch B refused under DropAndFlag; switch C: its boundary from A's grid and predecessor A).
- F24 (5d-U P3) the mark on a non-repeat burst is refused but untested: add the case to `ur_21_metadata_is_empty_or_the_mark_only` and to the Mock's twin.

## Fix round 2 (texts and rows)
- F1 (P3) RM-16: say that `B` is that sample's instant on the transmit SampleClock where it is compared with instants (RM-16's `[t, B)`, RM-21's `min(busy end, B)`).
- F2 (P3) design/07, design/09, design/18 Changes tables: one row each for stage 5's normative changes not yet listed (RM-12/13/14/15/16/21/37; MR-16/17/25/27/32/42; UR-21/22/23/30/36/38), citing note 47 stage 5.
- F6 (P3) UR-21's Test line cites `ur_21_a_burst_inside_the_in_flight_window_is_late` and its superseded behaviour: correct to the current tests.
- F7 (P3) MR-17's horizon sentence describes no booking rule now: rewrite to what `handed_end_before` still does (the Stop payload cut).
- F8 (P3) KL-36 quotes only the UHD reason: name both Providers' reasons.
- F9 (P3) `mr_16_s2_closed_burst_end_refused_end_plus_one_accepted` and `mr_16_s1_overlap_refused_at_receipt` names do not match what they test: rename by subject (renames.txt) or rewrite to S2/S1's shapes.
- F16 (P3) UR-21's "a held burst at the open burst's next sample joins it; uhd-tx decides its lateness" does not match the path: describe the code (`chain.continues_at`, no late policy at the cursor; `join_open` only while a Kernel burst is open).
- F17 (P3) `core.rs:28-29, 64-65` doc comments still name `held`.
- F18 (P3) `measurement.rs:222-223`: the unmeasured reason for `radio.timing.tx_in_flight_ns` says the group "has not been run", but the group is deleted: say the record no longer measures it (its value rests on the bench rows in `plan/phase8/bench-results.md`).
- F19 (P3) design/18 §5's U7 row still names a preemption bound.
- F22 (5d-U P2) UR-30 lists eight sections; UHD now writes `switches`: amend UR-30 and `tests/fake.rs:2498`'s section check.
- F23 (P3) row N47S5-52 mutates every held cut: narrow it to the switch's continuation (`ends_at_held` for a switch), probed.

## Added from 5b-2b's review (fix round 2)
- F25 (P3) `ur_21_s4b_booking_exactly_at_the_deadline_continues` was planned and never written: add it at the owner level (a join received exactly at `J(e)` continues the device burst: no EOB at `s+40`, plays at `s+40`), manual clock.
- F26 (P3) design/18: the `*Test (held-tail close):*` line sits inside UR-23; merge it into UR-23's one closing Test line.
- F27 (P3) `Bookings::continuation_open` and `close` have no pure tests: add them in `crates/ezsdr-radio/tests/staging.rs` (the owner-rig tests stay for wiring).
- F28 (P3) the S4b parity test submits B at 12 ms against `J = 15.04 ms` on the wall clock (3 ms margin): book B with a margin that a loaded host cannot eat (or move the timing-critical part to the manual clock), then run it 20 times (AGENTS §9 step 8) and record the count.
- F29 (P3) `Core::join_by` and `book_burst` each spell the device lead: one source for the lead that both `join_by` and `decided_at`/`ordinary_k` read.

## Added from 5d-M's review
- F30 (P2, fix round 2 — behaviour) MockRadio: a switch that has applied stays `pending` in the record and the `switches` row, because the Mock advances the record only at a receipt or a Stop, never at device time. Fix: the Mock settles the record as its device time passes the boundary (in `step`/`advance`, the same `advance` call UHD makes at device time), so the row reads `applied` once the new waveform plays. Test the report's sequence (x3xx: repeat at 2 004 000, switch at 2 005 000, boundary 2 018 000, step to 2 030 000: `outcome == "applied"`), and a row.
- F31 (P3) `accept` does not run `advance` (plan m3, row 61): make every record call that reads the pending switch run `advance` first, `accept` included, or record why `accept` cannot see a stale switch; extend `rm_16_advance_runs_inside_every_call` with `accept` at `b + 1`, and the row.
- F32 (P3) design/09 lines ~177 and ~289 still say "six sections": seven.
- F33 (P3) RM-13 and MR-42: the mark is accepted only on a repeat (a non-repeat burst with the mark is refused), and a refused switch writes no row ("each accepted switch is one row").

## Added from 5e–5f's review
- F34 (P2) `Tx.switch`'s `Rejected` on a Provider refusal is untested and has no row: a Python test where the submit reply carries `refused` for a switch (no repeat playing; one already pending) raises `Rejected`; a row on `switch`'s use of `_admitted`.
- F35 (P3) EA-16's `aligned` row omits `switch`, and no test runs `switch` inside `aligned`: amend the row and add the test.
- F36 (P3, check first) the reviewer infers that `tx.repeat(x); tx.stop(); tx.repeat(y)` now raises `Rejected` (the second repeat's start falls inside the Stop region), while EA-16 (design/16 ~line 99) promises that after `stop()` the next `repeat` transmits; `test_ea_16_aligned_gives_its_calls_one_instant` hides it with `sleep(0.02)`. **First** write the real-Session Python test (no sleep) and see what happens. If it raises: the untimed `repeat` compiles with `send_asap_and_flag`, so its start should be placed by `first_free` past the Stop region and transmit with `TIME_ERROR{send_asap}` — find why it is refused instead and fix it at the Provider booking (both Providers, the same record call), with the parity of that sequence; if the cause is something else, stop and report it with evidence. Then remove the `sleep` from the rewritten test.
- F37 (P3) row N47S5-50's `old` is a five-line block: narrow it to `self._check_channels(channels)` plus the minimum that keeps it unique.
- F38 (P3) `switch` and `repeat` spell the stream target two ways because of row P542: one spelling in both, P542 re-anchored.

## Fix round 3 — added from 5g's review
- F39 (P2) a switch moved by its late policy reports `TIME_ERROR{send_asap, action}` but its burst's `BurstRecord` says on time (`held.open.late` not set; RM-15/RM-22: the outcome reaches the burst's record through `BurstOpen.late`). Fix and test the report's S12 sequence: the switch's `bursts` row has `late_by` equal to the move.
- F40 (P2) the moved switch's `late_by_ns` (UR-38: by how much the cursor passed the chosen boundary) has no assertion and no row: assert the exact value in the S12 test and add a row.
- F41 (P3) UR-38 lists the `switches` row's fields without `boundary` and `moved_to`: one list.
- F42 (P3) the `switches` row has two writers that each re-find it by `action_id` (`settle_switches` writes `outcome`, `install` writes `moved_to`): carry `moved_to` in the settlement so one writer fills the row (AGENTS §9 rule 2).

## Fix round 3 — added from 5h's review
- F43 (P3) the horizon refusal reuses the overlap text: give it its own reason on both Providers ("starts before a booking that may already be handed"), update the parity helper's classification and the UHD test's wording.
- F44 (P3) MR-17 does not say which bound the receipt refusal uses (the Mock's `tx_handed_end` also models a first send that must fit): MR-17 states that the receipt rule uses the conservative horizon `at_or_after(now + W)` (whatever the first send), and why (the UHD owner may hand from there).
- F45 (P3) `handover_horizon` has no row: add one (its `now + in_flight` or its `at_or_after`), killed by `ur_21_s5_b_handed_before_an_earlier_a_is_refused_at_receipt`.
- F46 (P3) the new refusal is a user-visible Ceiling: a new KNOWN_LIMITATIONS entry (symptom: a burst that starts before an already-accepted burst less than the in-flight window ahead is refused although nothing overlaps; workaround: submit bursts in start order, or more than the window ahead; evidence: the rule and its test; status: by design).

## Fix round 3 — F36 (owner decision 2026-11-07)
Principle (the orchestrator's, from the owner's question): a booking that cannot start at its requested sample **because of device state** — a Stop's region (the device draining a stopped burst), and as 5g already does, a switch boundary the owner has handed past — follows its **late policy**: SendAsap moves it to the first sample the device can take (`first_free`) and reports `TIME_ERROR{send_asap}` with its id; DropAndFlag refuses it. Only a conflict with **another accepted booking** is refused outright (R1).
- F36: implement this for the Stop region at both Providers' booking (the same record call): a SendAsap booking that meets the Stop region and no accepted chain is placed by `first_free` past the region, with `TIME_ERROR{send_asap}`; a DropAndFlag one is refused as today. Turn `test_ea_16_a_repeat_right_after_stop_transmits` into a passing test (no `expectedFailure`), remove the `sleep(0.02)` from the aligned test, add the parity of `repeat; Stop; repeat` (both Providers: the second repeat starts at the region's end with one TIME_ERROR{send_asap}), a pure test of the decision, and rows. RM-15/RM-16 (and MR/UR restatements, EA-16) say it.
- Check every other receipt-time refusal the stage added against the principle and list each in §8 with its verdict (device state → late policy, or a conflict with an accepted booking → refusal): the overlap refusal, the hand-over horizon (5h), the joined-piece move (D24), the dispatch collision (5b-3), the switch's S12 (5g), the second pending switch.

## Fix round 4 — from the 5j-1/2 review
- F47 (P2) SC-23a (design/02 ~242), KC-28 step 7 (design/06 ~293) and RS-51 (design/04 ~260) still say `requested_at` is always recorded when the target advanced: say "for a timed action" (an untimed one has none, RS-19).
- F48 (P3) MA-14 (design/05 ~169) says a Provider places every untimed Action; only the radio's TxBurst gives it a meaning (RS-19: each Vocabulary decides): MA-14 states the mark and that its meaning is the Vocabulary's; RM-15 (or the radio rule that does) states the TxBurst placement. No behaviour change for `start_rx`/`capture`.
- F49 (P3) `move_untimed` (staging.rs ~304) duplicates `move_start`'s chain rewrite: one private helper for the rewrite, the two differ only in the placement decision.
- F50 (P3) `place` computes `front(k, len, join)` with plain arithmetic before the Providers' overflow guards: checked arithmetic in `place`/`front` (an overflow is a refusal, no panic, no wrap).

## Fix round 4 — from the 5j-3–5 review
- F51 (P2) RM-15's untimed text omits the hand-over horizon: say the placement also skips past a booking the Provider may already have handed (KL-37's horizon), as `place` does.
- F52 (P2) same as F48 (MA-14 obliges every untimed Action): fold into F48.
- F53 (P3) UR-21's Test line cites `ur_21_an_untimed_send_is_placed_after_its_delivery` twice: once.
- F54 (P3) parity.rs: the doc comment of `tx_decisions` is now above the new parity tests: move it back to its function.
- F55 (P3) `mr_16_untimed_drop_and_flag_is_placed_at_the_stop_region_end` builds no Stop and asserts no target: build the Stop and assert the placed target exactly (or rename by what it tests).
- F56 (P3) missing tests: the Mock off-grid untimed test (sequence 8: an untimed finite burst's record has `requested_target == None`), and sequence 6 (`aligned` SendAsap inside a Stop region moves with TIME_ERROR) on the Mock.
- F57 (P3) `ur_21_s19_untimed_switch_takes_its_timed_twin_boundary` tolerates one whole period: make both twins use the same lead and grid phase so the test asserts the same boundary exactly (or the exact relation between them).

## Review 5a-5b-1 (report tail)
> plan · deepseek-v4.1-flash-free
Read-only review of `c41530f..0ab7948` (5a shared booking record, 5a fixes, major A, 5b-1), findings checked against `7df8066`.

- **P3** `design/07-radio-model.md:166,232` (VERIFIED) — RM-16 defines the Stop bound `B` as "the first sample at or after `s + radio.timing.tx_in_flight_ns`, plus three samples" (a sample position), but then uses `B` in `[t, B)` where `t` is the Stop's root-tick instant, and RM-21 compares `min(busy end, B)` where the busy end is held in root ticks (`crates/ezsdr-radio/src/staging.rs:349-355` separates `stop_sample` from `stop_bound`; Mock passes `time::v_of`, UHD `Clock::instant`). The spec never says whether `B` is the sample or its instant, so the two uses are not unambiguously comparable; say that `B` is that sample's instant in the transmit SampleClock.
- **P3** `design/07-radio-model.md:427` (VERIFIED) — the closing Changes table has no row for this range's normative design/07 edits: RM-16's Stop bound `B` and RM-21's `min(busy end, B)`, introduced by `0ab7948` (5b-1). At `0ab7948` the last row is 2026-11-06 (b5); at `7df8066` it is 2026-11-07 (5d-M), still without a 5a/5b-1 row. Spec 22 DA-1 and AGENTS.md §2 require the change in this table with its decision record.

Dropped 1 candidate that no longer holds at `7df8066`: `first_free` lacking the `joins` flag / returning `i64` (now `Option` and takes `joins`).

Checks not done: mutation rows were not run locally (they run only through `remote_mutate.py`); no full gate, workspace test run or Python suites; the documented S13 parity deviation (D3) was taken as accepted.


## Review 5b-2a (report tail)
> plan · deepseek-v4.1-flash
[0m$ [0mcd /home/komatsu/.cache/ezsdr-fixes/2026-11-07-n47s5/review-tree && grep -n "delivery_allowance_ns" -A 6 crates/ezsdr-mock-radio/src/profile.rs | head -30; grep -n "DELIVERY_ALLOWANCE_NS" -A 2 crates/ezsdr-radio-uhd/src/profile.rs
143:    pub(crate) fn delivery_allowance_ns(&self) -> i64 {
144-        match self.kind {
145-            ProfileKind::X3xx => 3_000_000,
146-            ProfileKind::Ideal => 0,
147-        }
148-    }
149-
13:pub const DELIVERY_ALLOWANCE_NS: i64 = 3_000_000;
14-/// The default samples per receive block (UR-5, UR-9).
15-pub const DEFAULT_BLOCK_LEN: u32 = 2_000;
--
21:    ProviderTerms { min_timed_command_lead_ns: DEVICE_LEAD_NS + DELIVERY_ALLOWANCE_NS, repeat_max_samples: u64::from(u32::MAX), repeat_align_samples: 1 }
22-}
23-
> plan · deepseek-v4.1-flash
Read the diff, the plan's 5b-2a entry (D5–D23, rev 7), the connected code, and checked each item against `work/n47s5`.

- **P2** `crates/ezsdr-radio/src/staging.rs:196` — the `joins &&` conjunct added for **[R5-m4]** is unguarded: no test calls `first_free(…, false, …)` (all calls pass `true`, `crates/ezsdr-radio/tests/staging.rs:440-517`) and row N47S5-68 (`!(joins && c.join && c.end == k)` → drops only `now <= join_by(k)`) survives a mutant that deletes `joins &&`. Sequence (Mock `ideal`, lead+allowance 0): book 40 samples at 0, then a repeat targeted at 39 received at root tick 40 (SendAsap, `ordinary` = 40). Correct: placed at 42, plays. With the conjunct gone: `first_free` returns the join point 40, `accept(40, None)` refuses it ("MR-16: burst overlaps an accepted booking") and the repeat never plays. VERIFIED (code read); reachability INFERRED from `profile.rs:143-147` (Ideal = 0).
- **P2** `crates/ezsdr-radio/src/staging.rs:217` (+ Mock call `crates/ezsdr-mock-radio/src/lib.rs:1629`) — `ordinary_k` has no test and no row; the plan's assigned `rm_16_ordinary_k_is_decide_s_threshold` (pure, `n = 3`, §8 m1) does not exist at the tip. Reverting the Mock to its old `at_or_after(decided + lead)` breaks no test and no row: at 390 625 kS/s (`n` = 512, 2 ms = 3 906.25 samples) the two forms differ by one sample, so a SendAsap booking is placed one sample early/behind `decide`'s own threshold. VERIFIED (grep: no `ordinary_k` row in `mutations.json` at the tip).
- **P3** `crates/ezsdr-radio/src/staging.rs:163` — `Bookings::prune` is new here (Q-5) with no unit test and no mutation row, though the plan's "Rows 18–30" range covers 5b-2a; only over-pruning is observable (it would accept a booking that meets an accepted region). VERIFIED.
- **P3** `design/18-uhd-radio.md:247` — UR-21's *Test* line still cites `ur_21_a_burst_inside_the_in_flight_window_is_late` (the test is `…_is_refused` since this commit) and states its superseded behaviour ("uhd-tx moves `send_asap` or drops `drop_and_flag` at the first hand-over"). It was un-backticked, so `rules.py` cannot catch it. Holds at the tip. VERIFIED.
- **P3** `design/09-mock-radio.md:143` — MR-17 now says "with no in-flight front", then keeps "At horizon `now + W`, an earlier-received burst contributes…"; at the tip the only consumer of `handed_end_before` is the Stop payload cut (`lib.rs:1173`), so the sentence no longer describes any booking rule. VERIFIED.
- **P3** `KNOWN_LIMITATIONS.md:135` — KL-36's symptom quotes only `UR-21: the burst overlaps an accepted booking`; the Mock's reason is `MR-16: burst overlaps an accepted booking` (`lib.rs` refusal string), so the user-visible text is wrong for MockRadio. VERIFIED.
- **P3** `crates/ezsdr-mock-radio/tests/mock_radio.rs:1081` — `mr_16_s2_closed_burst_end_refused_end_plus_one_accepted` uses a 2 000-sample head and asserts `end + 1` **refused** / `end + 2` accepted, the opposite of the plan's S2 ("4, 10, 32 samples … `end+1` accepted"); `mr_16_s1_overlap_refused_at_receipt` (`:1057`) books a repeat at `s` plus a burst *before* `s`, not S1. The names/killers mislead; the real S2/S4 shape is only in the parity test. VERIFIED.

Dropped after the tip check: 1 — the 5b-2a parity list missing at `90af78e` (D21) is written as `s15_parity_booking_rule_both_receipt_orders` in 5f; the UHD Stop-region half has no `ur_23_s7_stop_tail_refuses_until_bound` but is pinned by the extended `ur_23_stop_preserves_handed_cursor_after_stop` (`tx.rs:2664-2670`).

Not done: no build or test run (static review only), and I could not read the out-of-tree probe logs (`probes-b2a/summary-final.txt`), so the record's "all 18 rows KILLED" claim is unverified.


## Review 5b-2b (report tail)
> plan · deepseek-v4.1-flash
[0m$ [0mcd /home/komatsu/.cache/ezsdr-fixes/2026-11-07-n47s5/review-tree && git show work/n47s5:plan/maintenance/tools/mutations.json | grep -n "ur_21_s4b_booking_exactly" | head
(no output)
[0m$ [0mcd /home/komatsu/.cache/ezsdr-fixes/2026-11-07-n47s5/review-tree && git show work/n47s5:crates/ezsdr-radio-uhd/src/provider/tx.rs | grep -n "fn ur_21_.*deadline\|fn ur_21_s4b" | head
2642:    fn ur_21_s4b_three_bookings_b_continues_a_and_c_plays_on_time() {
2711:    fn ur_21_m1_receipt_at_the_deadline_is_decided_by_the_record() {
> plan · deepseek-v4.1-flash
## Review — n47s5 5b-2b (90af78e..410e3ac, still present at `work/n47s5`)

- **P3** `test-plan.md:923` vs `crates/ezsdr-radio-uhd/src/provider/tx.rs:2642` — planned test `ur_21_s4b_booking_exactly_at_the_deadline_continues` does not exist at 410e3ac **or at the tip** (`git show work/n47s5:…` has only `…s4b_three_bookings…` and `…receipt_at_the_deadline…`), and no deviation (D15–D23) records the drop. The end-to-end case "join received exactly at J(e) still continues the device burst (no EOB at `s+40`, plays at `s+40`)" is pinned only at record level (`tx.rs:2711` calls `Bookings::accept` directly, never the owner's close), so the owner's `now <= J(e)` is only exercised through S4b's 12 ms receipt. VERIFIED.
- **P3** `design/18-uhd-radio.md:258` — the `*Test (held-tail close):*` line is inserted **inside** the UR-23 rule; AGENTS §2 requires a rule to end in its indented *Test* line, and UR-23 already ends with one, so the rule now has two and the prose after 258 ("A last buffer sent when its end is already…") reads as a new topic behind a test list. Still present at the tip. VERIFIED.
- **P3** `crates/ezsdr-radio/src/staging.rs:154`/`:163` — `continuation_open` and `close` are pure predicates with no pure test; their only coverage is the whole UHD owner rig (manual clock + `FakeDevice` + `Tx`), and §4.1 of the plan has no row for them either (AGENTS §9 maintainability rule 3: test pure decisions at that level). VERIFIED (ran `cargo test -p ezsdr-radio-uhd --lib`: 399 pass, no `staging` test touches them).
- **P3** `crates/ezsdr-acceptance/tests/parity.rs:1788` — B is submitted at 12 ms while its join deadline is `J(20_040) = 15.04 ms` (2 ms lead + 3 ms allowance), and `submit` waits on the **wall clock** (`wait_until`, `:1055`), so it carries only 3.04 ms of margin; on a loaded host (the remote `sim02/sim03` shards, AGENTS §7) B is refused at receipt and the test fails on `overlap_refusals` (`:1795`). The new test is timing-dependent and §9 asks for 20 repeats; the step's record names only the five mutation probes. Arithmetic VERIFIED, failure INFERRED (ran it once, passed).
- **P3** `crates/ezsdr-radio-uhd/src/provider/core.rs:903` vs `.../control.rs:449` — `join_by` hard-codes `DEVICE_LEAD_NS + DELIVERY_ALLOWANCE_NS` while `book_burst` keeps its own `lead = ticks(DEVICE_LEAD_NS)` for `decided_at`/`ordinary_k`; the description's published command lead (`radio.timing.min_timed_command_lead_ns`, 5 ms) is read by neither in this path, so a profile-driven lead would silently split J(e) from the late/ordinary bound (maintainability rule 1, duplicated decision). INFERRED.

Checked and not reported: the `Some(_)|None` merge, the `last == e` case (a booking at a chain end can only be a join — every chain's `front > end` refuses a standalone head there, and a continuation skips `dispatch_start`, so no refused piece can strand `last == e`); `close`'s join clearing and the stale-`now` receipt are R4-4's recorded decision; row 58's `aclosed`-vs-`<` mutant of `continuation_open` is still killed by `ur_21_m1_receipt_at_the_deadline…`.
Dropped for later commits: 0.

Could not do: run rows N47S5-25/-56/-57/-58/-60 and phase7/B09 through `remote_mutate.py` (anchors resolve uniquely in the tree, kill not observed); the 20× repeat of the new wall-clock parity test; anything on real UHD hardware.


## Review 5b-3 (report tail)
> plan · deepseek-v4.1-flash
[0m$ [0mcd /home/komatsu/.cache/ezsdr-fixes/2026-11-07-n47s5/review-tree && grep -n "busy" crates/ezsdr-radio-uhd/src/provider/core.rs | head -20
378:    pub busy: Bookings,
386:            self.busy.stop(stop_bound(at, in_flight, |t| clock.at_or_after(t), |k| clock.instant(k)));
387:            self.busy.halt(clock.at_or_after(at), stop_sample(at, in_flight, |t| clock.at_or_after(t)));
> plan · deepseek-v4.1-flash
[0m$ [0mcd /home/komatsu/.cache/ezsdr-fixes/2026-11-07-n47s5/review-tree && grep -n "busy.prune\|fn prune" crates/ezsdr-radio-uhd/src/provider/control.rs crates/ezsdr-radio/src/staging.rs; grep -n "fn stop_now" crates/ezsdr-radio-uhd/src/provider/tx.rs
crates/ezsdr-radio-uhd/src/provider/control.rs:492:            busy.prune(clock.at_or_after(receipt));
crates/ezsdr-radio/src/staging.rs:178:    pub fn prune(&mut self, now_k: i64) {
1110:    fn stop_now(&mut self, reason: &str, cancel_reason: ezsdr_radio::CancelReason) {
> plan · deepseek-v4.1-flash
Review of `410e3ac..c53d4a3`, checked against `work/n47s5`.

1. **P2** `crates/ezsdr-radio/src/staging.rs:210-211` — `move_start` returns `Ok(())` when no chain starts at `old`, so a head whose chain was pruned is neither clash-tested nor re-added to the record: the moved burst plays untracked. `prune` (`staging.rs:178-183`) runs on every receipt (`control.rs:492`) and drops any chain whose front ≤ the receipt sample, including one whose burst a stalled owner still holds. Sequence: A booked on time at `k=20 000`, len 40 000 (front 60 002); the owner does not step past 60 002; B is received at 61 000 and booked on time at 70 000 (prunes A); the next step dispatches A late → `move_start(20 000, 63 000, 40 000)` finds no chain → A plays `[63 000, 103 000)` with nothing in the record. B, accepted on time, is then dropped or moved by the Module's late path, and a booking C received in that span is accepted and then dropped/moved although the host is not late. So `design/07-radio-model.md:158` and `design/18-uhd-radio.md:245` ("judged … against every accepted booking") are false in this state, and no test prunes a UHD `Bookings` (`grep prune crates/`: no UHD test). VERIFIED (code, arithmetic); sequence INFERRED.
2. **P2** `crates/ezsdr-radio-uhd/src/provider/tx.rs:569` — the new configure-refusal `drop_piece` (D27, row N47S4-13b) is pinned only in the "too much" direction: the row's mutant is `busy = Default::default()`, killed by the *value* assertions of `ur_21_a_refused_burst_stays_counted_until_its_booked_end`; nothing books inside a configure-refused burst's span (`tests/fake.rs:1879` books after its end), so a mutant that deletes this line (the pre-commit behaviour, span kept) survives the suite — the region-freed half of the rule has no test. VERIFIED (read rows and tests).
3. **P3** `crates/ezsdr-radio-uhd/src/provider/tx.rs:4597-4605` — `ur_23_s3_refused_move_frees_its_front` omits the plan's C6 half ("the refused burst's end still counts for a value", §4.2; `design/18:245`), and D29's "could not discriminate" is wrong: A `[20 000, 60 000)`, B at 45 000 len 100, dispatch at 40 000 → refused (moved front 82 002); a value at 45 500 is refused only by A's booked end (60 000 > B's 45 100), so a mutant that also reset `until` on the dispatch refusal would be caught by that geometry. VERIFIED (read).
4. **P3** `crates/ezsdr-radio/src/staging.rs:215` — the Stop-region clause is unreachable: `stop_now` (`tx.rs:1110-1116`) empties `held` on every `TxCmd::Stop`, and a booking received after the Stop cannot start in `[t, stop)` (`accept`), so a moved start never enters it; no test and no row (31/32/59 cover other chains and the head's own pieces). Dead branch to delete, or a case whose test is missing. VERIFIED (read).
5. **P3** `crates/ezsdr-radio/src/staging.rs:214-216` — second copy of `accept`'s region decision (`staging.rs:135-136`): RM-15's overlap/region rule must now change in two places, and 5h's hand-over-horizon rule was added to `accept` only (safe today only because queue order keeps a moved head first — nothing states that). VERIFIED (read).

Dropped (fixed later on `work/n47s5`): the joined-piece unchecked move (5c/rev 10 `move_start` clause), `preempt`'s unchecked `SendAsap` (5c), the `core.held` leftover and N47S5-31's `forget`-sized anchor (5c-2/rev 11).
Could not do: build or run the two tests or their mutation rows (read-only tree, no probe copy), so `cargo fmt`, clippy and the gate remain unchecked.


## Review 5c (report tail)
> plan · deepseek-v4.1-flash
[0m$ [0mcd /home/komatsu/.cache/ezsdr-fixes/2026-11-07-n47s5/review-tree && git show work/n47s5:crates/ezsdr-radio-uhd/src/provider/tx.rs | grep -n "fn ur_21_a_late_joined_burst_cannot_move_and_is_refused\|continues_at(k)" ; git show work/n47s5:crates/ezsdr-radio-uhd/src/provider/core.rs | grep -c "held\` and \`rates\|rec\`, \`held\`"
489:                let held = if self.chain.continues_at(k) {
3087:    fn ur_21_a_late_joined_burst_cannot_move_and_is_refused() {
2
> plan · deepseek-v4.1-flash
[0m$ [0mcd /home/komatsu/.cache/ezsdr-fixes/2026-11-07-n47s5/review-tree && git show work/n47s5:plan/maintenance/tools/mutations.json | grep -c '"id": "N47S5-62"'; git show work/n47s5:plan/maintenance/tools/mutations.json | grep -c '"id": "N47S5-72"'; git show work/n47s5:crates/ezsdr-radio-uhd/tests/common/measurement.rs | grep -n "tx_in_flight_ns" | head -3
1
1
222:        "radio.timing.min_timed_command_lead_ns" | "radio.timing.tx_in_flight_ns" | "radio.perf.rx_bytes_per_s" | "radio.full_duplex" | "radio.hardware_time" => {
> plan · deepseek-v4.1-flash
[0m$ [0mcd /home/komatsu/.cache/ezsdr-fixes/2026-11-07-n47s5/review-tree && grep -n "a held burst starts before" crates/ezsdr-radio-uhd/src/provider/tx.rs; grep -n "UR-21: a late joined burst cannot be moved" crates/ezsdr-radio-uhd/src/provider/tx.rs; grep -n "self.open.take().is_some()" crates/ezsdr-radio-uhd/src/provider/tx.rs; grep -n "^| U7" design/18-uhd-radio.md | cut -c1-60
437:            debug_assert!(self.held.keys().next().is_none_or(|h| *h > next), "a held burst starts before the open burst's next sample (RM-15)");
1032:                    return self.core.command_rejected(held.settings.action(), "tx_burst", "UR-21: a late joined burst cannot be moved; it is refused");
900:        if self.open.take().is_some() {
410:| U7 | A transmit in-flight window | §3's in-flight wind
> plan · deepseek-v4.1-flash
Report:

**P2** `crates/ezsdr-radio-uhd/src/provider/tx.rs:2997` (test) / `crates/ezsdr-radio/src/staging.rs:211-213` / `tx.rs:980` — D24's refusal is coded twice; the only test drives the unreachable copy. The reachable refusal is `dispatch_start` → `move_start` (a joined piece at a chain's end is no chain start, so the new `Err(Overlap)` branch fires), and no test reaches it: `ur_21_a_late_joined_burst_cannot_move_and_is_refused` books C with `TxCmd::Burst` at A's mid-payload sample, which `Bookings::accept` refuses as an overlap, so it only exercises `join_open` (`tx.rs:1032`). Sequence: A(1 000) at `s`; B(10) at `s+1 000` booked before `J(s+1 000)` (joined, accepted); the tx thread does not run until after `instant(s+1 000) − 2 ms`; B is late at dispatch → `send_asap` → `move_start` refuses. Reverting `staging.rs:213` to `Ok(())` (its pre-commit behaviour) fails no test. VERIFIED.

**P3** `design/18-uhd-radio.md:247` — "A held burst at the open burst's next sample joins it. uhd-tx decides its lateness…" is not what the reachable path does: a held burst at the device cursor is dispatched through `chain.continues_at` (`tx.rs:459`) and skips `dispatch_start`, so no `LatePolicy` runs; only a burst held while a Kernel burst is still open (`tx.rs:433`) reaches `join_open`, and that state cannot come from receipt. A late joined hand-over is therefore handed to the device (late report/underflow), not dropped or refused as the rule says. VERIFIED code path, INFERRED consequence.

**P3** `crates/ezsdr-radio-uhd/src/provider/core.rs:28-29, 64-65` — doc comments still name `held` among the instance's state guards ("`streams`, `rec`, `held` and `rates`") after D28 removed the field; still there at the tip.

**P3** `crates/ezsdr-radio-uhd/tests/common/measurement.rs:222-223` — the unmeasured reason for `radio.timing.tx_in_flight_ns` still says "the Phase 8 stage 3b-2 measurement group has not been run", but this commit deletes that group (D37): the record now reports a reason that can never become false, and no test pins the sentence.

**P3** `design/18-uhd-radio.md:410` — §5's U7 row still says the choice states "the preemption bound it implies (UR-21, UR-23)"; after this commit neither UR-21 nor UR-23 states a preemption bound, so the decision row describes a rule set that no longer exists.

**P3** `tx.rs:437` — `debug_assert!` replaces the previous `k < next`/`h < next` handling without a test. If that state is reachable (the plan's unreachability argument is INFERRED; receipt only guarantees the record's regions, and the plan records no probe for it), a debug build panics and a release build feeds `send` a negative `count` (index panic) instead of applying the burst's policy. No test drives it.

Dropped: one finding (duplicate mutation-row id `N47S5-62`, `mutations.json` twice) — a later commit renumbered the 5c copy to `N47S5-72`.

Not checked: I did not build or run the tests (no `cargo`, and the commit's green claims are taken from the plan's end checks); I did not re-derive the plan's `k < c`/`h < next` unreachability from a run, and I did not read the untouched bodies of `design/18`'s §6 table rows or `plan/phase8` documents for further stale preemption text.


## Review 5d-M (report tail)
> plan · deepseek-v4.1-flash
[0m$ [0mcd /home/komatsu/.cache/ezsdr-fixes/2026-11-07-n47s5/review-tree && grep -n "is neither empty nor the switch mark\|is empty, or exactly the switch mark" design/07-radio-model.md && grep -n "Each switch is one row" design/09-mock-radio.md
144:- **RM-13** A `TxBurst` targets `<device>/tx/s<k>` and its `ArtifactRef` holds complex samples in that stream's `ezsdr.stream.cf32` contract, channel-interleaved (sample `n` of channel `c` at byte offset `(n · channels + c) · 8`), little-endian; `channels` is the number of channels the stream holds and channel `i` is that set's `i`-th smallest logical channel (RM-3, RM-30). Its length is `size_bytes / (8 · channels)`, a whole number ≥ 1. The burst transmits on every channel of the stream. A repeated waveform's length must be at most `radio.tx.repeat_max_samples` and a multiple of `radio.tx.repeat_align_samples`. `TxBurst.metadata` is empty, or exactly the switch mark `radio.repeat_operation = "switch"` (RM-37). A burst whose region meets an accepted burst's is refused by RM-15.
151:  - `metadata` is neither empty nor the switch mark (RM-37); or
220:- **MR-42** MockRadio decides a switch (RM-37) on receipt with the profile's device lead and in-flight window: the boundary is the first sample of RM-37 at or after `now + radio.timing.tx_in_flight_ns + min_timed_command_lead_ns`, and the stream's booking record holds it. Each switch is one row of `ezsdr.radio.mock.switches`, `{ action_id, predecessor, selected_at, old_period, new_period, outcome }`: written at receipt with `outcome: "pending"`, and `applied` or `cancelled` once the record settles it. `action_id` is the id the switch was dispatched with, `predecessor` the id of the repeat it replaces, and `selected_at` the receipt's instant.
> plan · deepseek-v4.1-flash
Findings (read-only; nothing edited, no git state changed):

1. **P2** `crates/ezsdr-mock-radio/src/lib.rs:2382` (`fn step`; settle only at `:1696`/`:1177`) — **A switch that has applied is still recorded `pending`.** The record is advanced only by `prune`/`switch` (`staging.rs:257,300`), and the Mock calls them only at a receipt or a Stop, never at device time; `instance()` is `&self` (`:2025`) so it cannot settle either. Sequence (x3xx): repeat at 2 004 000, switch at 2 005 000 (boundary 2 018 000), `step` to 2 030 000, read `ezsdr.radio.mock.switches[0].outcome` → `pending`, while `bursts` shows the new repeat playing from 2 018 000. Wrong outcome: MR-42 says the row is `applied` once settled and RM-37 says it is pending only "until the device reaches b"; UHD settles it at record write (`write_sections`), so the two Providers record the same sequence differently. Fix: `advance(now_k)` + `settle_switches` in `step`. INFERRED (read; not run).

2. **P3** `crates/ezsdr-radio/src/staging.rs:154` (`accept`), test `crates/ezsdr-radio/tests/staging.rs:584` — **The plan's m3 sequence and row 61 are missing from `accept`.** Plan §4.4 [m3] requires "`halt`, `accept` and `switch` at `now = b + 1` see the switch applied" and "row 61: the call of `advance` removed from `accept`"; the test covers only `prune` and `switch`, `accept` never calls `advance`, and D44/D49 record the drop for `halt` only. Today no caller reaches `accept` without a prior `prune` (Mock `lib.rs:1650`, UHD `control.rs:492`), so a direct `Bookings::accept(b+1, …)` refusing against the old repeat is a latent, untested rule. VERIFIED.

3. **P3** `design/09-mock-radio.md:177` and `:289` — **"All six sections exist from construction" / "all six MR-27 sections" are now false** (MR-27 at line 175 and the code say seven, `lib.rs:433`). Leftover from this commit's MR-27 edit; still false at `work/n47s5`. VERIFIED.

4. **P3** `crates/ezsdr-mock-radio/src/lib.rs:1577` vs `design/07-radio-model.md:144,151` — **The mark's scope is wider in the text than in the code.** RM-13 says metadata "is empty, or exactly the switch mark" for every `TxBurst` and its refusal list only refuses "neither empty nor the mark", yet a `repeat: false` burst carrying the mark is refused (`switch = repeat && is_switch`). Likewise MR-42 (`design/09:220`) says "Each switch is one row", but a refused switch writes no row (`mr_16_s10_switch_on_stopped_…` asserts an empty `switches`). Both should say "on a repeat" / "each accepted switch". VERIFIED.

Dropped against `work/n47s5`: 0 (the later 5d-U re-anchors rows 33–40, 51, 70, 71 and keeps every mutant live; MR-27/RM-13 texts are unchanged at the tip).

Not checked: I ran no tests and no mutation rows, so the tests' passing and each row's kill are taken from the plan's probe records; I verified the emitted-sample cut only by reading `emit_tx_until` (`lib.rs:1772-1798`) and the `bursts` assertions, not by running a harness; I did not review 5d-U/5e/5f beyond the spot checks needed to keep these findings alive.


## Review 5d-U (report tail)
> plan · deepseek-v4.1-flash
**P2 — `stream.playing` is not the repeat that is actually playing after a refused switch.** `crates/ezsdr-radio-uhd/src/provider/control.rs:500,511-514,528` sets `playing = {id, origin: start, period: len}` at receipt and never restores it; `crates/ezsdr-radio/src/staging.rs:298-300` (`install`) drops the pending switch as `Refused` and the old repeat plays on. The next switch is then computed from the *refused* switch's grid and names it as its predecessor. Sequence (DropAndFlag, 1 MS/s rig of the S8 test): repeat A (1 000 samples, origin 2 000); switch B (500 samples) at 3 ms → boundary 15 000; stall the hand-over past 15 000 → B `refused_past_boundary`, A plays on; then switch C (`at` 20 100). Expected (RM-37, S8): boundary 21 000 on A's grid (2 000+1000n), row `predecessor 1, old_period 1000`. Actual: boundary 20 500 (the refused 500-grid from 15 000), row `predecessor 2, old_period 500`, and A's waveform is cut mid-period (its last period is 20 000-20 999). The Mock cannot reach the state (it installs at receipt, staging.rs:279-287), so a parity test would diverge. Same mechanism at the tip after a SendAsap move (5g): the repeat starts at `moved_to`, `playing.origin` stays the chosen boundary. VERIFIED by reading; reachability INFERRED (no test books a switch after a refusal: `ur_21_s12_drop_and_flag_switch_handed_past_is_refused`, tx.rs:6317-6338).

**P2 — `design/18-uhd-radio.md:325-328` (UR-30) is made false and not amended.** UR-30 enumerates the Provider's sections as the eight `device, envelope, applied, bursts, async, rejected, timing, stats`; `mod.rs:419` now writes a ninth, `switches`, and UR-38 (`design/18:295`) cites "Its `switches` row (UR-30)". 5d-M updated the Mock's MR-27 to seven sections for the same change; the UHD list and the 2026-11-07 Changes rows (UR-36, UR-38 only) were not. `crates/ezsdr-radio-uhd/tests/fake.rs:2498` also still checks only the eight. VERIFIED.

**P3 — row N47S5-52 mutates the held cut in general, not the switch's continuation.** `plan/maintenance/tools/mutations.json:14824-14825` forces `device_eob` true for every held cut, so the row is not "the switch's continuation sends an EOB at `b`" (plan §5) and would be killed by unrelated cut tests; a mutant on `ends_at_held` for a switch would pin m9. The kill is real; the mutant is broader than the rule. VERIFIED (probe KILLED).

**P3 — `ur_21_metadata_is_empty_or_the_mark_only` leaves the mark on a non-repeat untested** (tx.rs:6308-6318). RM-13 (`design/07:144`) allows the mark as metadata; the code refuses it because `switch = repeat && is_switch(metadata)` (control.rs:404,448). The test only marks repeats. VERIFIED.

Dropped by the tip check: none — both P2s and both P3s still hold at `work/n47s5` (`control.rs:503-535` keeps the un-restored `playing`; `design/18:326-329` still lists eight sections).

Could not do: no build or test run (no compilation, no mutation run) — reachability and the wrong boundary are read from the code, not executed; I did not read the whole UHD test module or the 5g move's tests beyond their install path.


## Review 5e-5f (report tail)
> plan · deepseek-v4.1-flash
[0m$ [0mcd /home/komatsu/.cache/ezsdr-fixes/2026-11-07-n47s5/review-tree && grep -n "pub fn ordinary_k" -A 12 crates/ezsdr-radio/src/staging.rs
401:pub fn ordinary_k(decided: i64, lead: i64, at_or_after: impl Fn(i64) -> i64, instant: impl Fn(i64) -> i64) -> i64 {
402-    at_or_after(instant(at_or_after(decided)).saturating_add(lead))
403-}
404-
405-/// RM-16: the first sample a booking may start at after a `Stop` at `at` (note 47 stage 5, §0):
406-/// the first sample at or after `at + in_flight`, plus three. The window `in_flight` is the
407-/// profile's; `at_or_after` is the stream's clock, which each Provider passes in.
408-pub fn stop_sample(at: i64, in_flight: i64, at_or_after: impl Fn(i64) -> i64) -> i64 {
409-    at_or_after(at.saturating_add(in_flight)).saturating_add(3)
410-}
411-
412-/// RM-16: the root tick a `Stop` at `at` bounds a transmit stream by: [`stop_sample`] in root ticks.
413-pub fn stop_bound(at: i64, in_flight: i64, at_or_after: impl Fn(i64) -> i64, instant: impl Fn(i64) -> i64) -> i64 {
> plan · deepseek-v4.1-flash
[0m$ [0mcd /home/komatsu/.cache/ezsdr-fixes/2026-11-07-n47s5/review-tree && git log --oneline 4c18c6c..work/n47s5 | cat
906d45f wip(n47s5): 5i — a lone burst plays every sample (D74 was a misreading; regression test)
dc57cb9 wip(n47s5): 5h — a booking before one the Provider may already have handed is refused at receipt (owner); #79's B-first sequences pass
54421e9 wip(n47s5): 5g — a late switch follows its late policy: SendAsap moves it to the next boundary (owner)
[0m$ [0mcd /home/komatsu/.cache/ezsdr-fixes/2026-11-07-n47s5/review-tree && git show work/n47s5:python/tests/test_easy_api.py | grep -n "\.switch(\|switch_repeat" | head -20; echo ---; git show work/n47s5:design/16-easy-api.md | grep -c "tx_streams\[k\].switch"
735:            sdr.tx.switch(ramp(500))
1479:        ezsdr.session.Tx(sdr, "radio", 0).switch(np.ones(100, dtype=np.complex64))
1494:                    ezsdr.session.Tx(sdr, "radio", 0).switch(x)
---
1
> plan · deepseek-v4.1-flash
Findings (my three sub-step checks plus the later commits at `work/n47s5`):

- **P2** `python/ezsdr/session.py:502-512` / `design/16-easy-api.md:97` — the switch's `Rejected`-on-Provider-refusal path is untested and has no row. All three `Tx.switch` call sites at the tip pass an accepted booking (`python/tests/test_easy_api.py:735,1479,1494`); the only killer of `_admitted`'s `refused` branch (row 48) goes through `repeat`. Sequence: make `switch` return `self._session._submit(action, data)` raw, or give it `if refused: pass`; a submit reply carrying `refused` for a switch (no repeat playing, or one already pending — exactly the cases EA-16:97 promises) then returns an entry instead of raising, and the whole suite stays green. VERIFIED (all call sites read; the three mocked tests run OK in `~/.cache/ezsdr-venv`).

- **P3** `design/16-easy-api.md:98` (`Session.aligned`) — `switch` takes the block's instant (`session.py:509`), but the row still names only `tx.repeat`, `rx.start`, `rx.request`, and no test at the tip exercises `switch` inside `aligned` (the only three call sites are outside a block), so the two-line `"at": self._session._aligned` path is unpinned. VERIFIED.

- **P3** `design/16-easy-api.md:96` + `session.py:479-480` — the new "a second `repeat` on a playing stream is refused" names the wrong condition: the refusal is the stream's busy end being after the receipt, so `tx.repeat(x); tx.stop(); tx.repeat(x)` is also `Rejected` while `design/16:99` says "after a transmit stream's `stop()` its next `repeat` transmits" (`ordinary_k` starts at `at + lead`, `stop_sample` bounds at `at + in_flight + 3`, `crates/ezsdr-radio/src/staging.rs:401-410`). The rewritten `test_ea_16_aligned_gives_its_calls_one_instant` (`python/tests/test_easy_api.py:610-613`) papers over it with `sleep(0.02)` rather than pinning the window. INFERRED (no server binary to run the real-Session sequence).

- **P3** `plan/maintenance/tools/mutations.json` N47S5-50 — the `old` is a five-line block that includes `"verb": "start_repeat"`, which the mutant does not touch, so any edit of `switch`'s action header breaks a row about the channel check; `self._check_channels(channels)\n        action = {` would be unique and minimal (§9 rule 5). VERIFIED (anchor counts once today).

- **P3** `python/ezsdr/session.py:507` (D62) — `switch` writes `f"{self._direction}/s{n}"` while `repeat` writes `f"tx/s{n}"`: one decision, two spellings, chosen to keep row P542 unique, i.e. production text shaped by a row anchor (§9 rule 5). VERIFIED.

Dropped against `work/n47s5`: none of the above is fixed there (5g/5h/5i touch other code). The `#[ignore]` R5 test noted in the commit message no longer holds: 5h (`dc57cb9`) un-ignores `s15_parity_b_first_accepted_booking_rule`, so no finding from it.

Could not do: no cargo build and no real-Session run (`EZSDR_SERVER` binary) in this read-only copy, so the Providers' switch/stop sequences and the `stop(); repeat()` refusal of finding 3 are read from code, not executed; and I did not run `rules.py`.


## Review 5g (report tail)
> plan · deepseek-v4.1-flash
**Findings (5g, 4c18c6c..54421e9; all still present at `work/n47s5` unless noted)**

- **P2** `crates/ezsdr-radio-uhd/src/provider/tx.rs:352-361` — a moved switch emits `TIME_ERROR{send_asap, action}` but never sets `held.open.late`, so its `BurstRecord` (the Manifest `bursts` section, fed from `BurstOpen.late` at `crates/ezsdr-kernel/src/stream/burst.rs:356`) records the switch's burst as on time, contradicting `design/07-radio-model.md:158` ("the outcome reaches an admitted burst's `BurstRecord` through `BurstOpen.late`"). Sequence (S12): repeat at 2 000, `SendAsapAndFlag` switch at 3 ms, owner stalled to 8 ms → switch moved to its next grid boundary; observable: one `TIME_ERROR{send_asap, action:2}` exists while the switch's `bursts` row has `late_by: null`. The same gap leaves `report_send_asap` (`:974`, guard `open.late.is_none()`) free to emit a second, action-less `send_asap` if the switch later reaches `dispatch_start` (device burst closed between install and the moved boundary). No test reads the burst record. VERIFIED (read).
- **P2** `crates/ezsdr-radio-uhd/src/provider/tx.rs:354` — `late_by_ns` for the moved switch (UR-38's "the amount by which the cursor passed the chosen boundary", `design/18-uhd-radio.md:296`) is asserted by no test and pinned by no row: the S12 killer checks only `outcome` and `action`, and rows 74–80 touch the move, the policy branch, `held.k`, `moved_to` and `action`, not this value. `cursor - held.k` → `boundary - held.k`, `cursor` or `0` all keep the suite green. VERIFIED (read).
- **P3** `design/18-uhd-radio.md:296` — UR-38 still enumerates the `switches` row as "the action, the predecessor, the receipt's instant, the old and new periods, and the outcome", two sentences after the same paragraph adds `boundary` and `moved_to` to that row. VERIFIED.
- **P3** `crates/ezsdr-radio-uhd/src/provider/tx.rs:360-362` with `crates/ezsdr-radio-uhd/src/provider/core.rs:1055` — one row, two writers that each re-find it by `action_id` with `.rev()`: `settle_switches` writes `outcome`, `install` writes `moved_to` outside the settlement (the deviation D67 documents). A miss of that lookup leaves `moved_to: null` with no error while the Module did move, so the record silently contradicts the sends; rule 2 (a lifecycle fact kept outside its owning state). VERIFIED.
- Also checked, no finding: Mock/UHD divergence (Mock never moves) is what RM-37/MR-42 now state; `switch()`/`install` callers and every `TimeErrorPayload` site are updated; the schema keeps `action` optional (`required` unchanged); the pure test's three grid cases and rows 74/75/76/77/80 each break the right assertion (row 75 is killed by the on-boundary case, not the off-grid ones).

Dropped because a later commit fixed it: 1 — at 54421e9 `crates/ezsdr-radio-uhd/src/provider/control.rs:534` sets `stream.playing` for a switch at booking with the *chosen* boundary, so a second switch after a moved one is booked on the wrong grid (a partial repeat); 8c3ba79 sets `playing` at install with the moved boundary.

Could not do: no `cargo` build or test run (read-only checkout), so the S12/`ur_23` cursor arithmetic and the "one TIME_ERROR" count are from reading the rig, not execution; no UHD/FakeDevice behaviour observed.


## Review 5h (report tail)
> plan · deepseek-v4.1-flash
[0m$ [0mcd /home/komatsu/.cache/ezsdr-fixes/2026-11-07-n47s5/review-tree && grep -n "overlaps an accepted booking" crates/ezsdr-radio-uhd/src/provider/control.rs crates/ezsdr-mock-radio/src/lib.rs; grep -n "pub fn handover_horizon" crates/ezsdr-radio/src/staging.rs; grep -n "start.saturating_add(first) <= window_end" crates/ezsdr-mock-radio/src/lib.rs; grep -n "reason.*overlaps an accepted booking" crates/ezsdr-radio-uhd/src/provider/tx.rs; grep -n "fn overlap_refusals" crates/ezsdr-acceptance/tests/parity.rs
crates/ezsdr-radio-uhd/src/provider/control.rs:528:                Some("UR-21: the burst overlaps an accepted booking")
crates/ezsdr-mock-radio/src/lib.rs:1684:            Some("MR-16: burst overlaps an accepted booking")
442:pub fn handover_horizon(now: i64, in_flight: i64, at_or_after: impl Fn(i64) -> i64) -> i64 {
1402:            let candidate = handed_end_before(order, before, start, end, window_end).filter(|_| start.saturating_add(first) <= window_end);
2154:            assert!(refused[0].payload["reason"].as_str().unwrap().contains("overlaps an accepted booking"), "{case}");
2188:        assert!(refused[0].payload["reason"].as_str().unwrap().contains("overlaps an accepted booking"));
2345:            assert!(refused[0].payload["reason"].as_str().unwrap().contains("overlaps an accepted booking"), "{case}");
2514:        assert!(rejected[0].payload["reason"].as_str().unwrap().contains("overlaps an accepted booking"));
2550:        assert!(refused[0].payload["reason"].as_str().unwrap().contains("overlaps an accepted booking"));
2587:        assert!(rejected[0].payload["reason"].as_str().unwrap().contains("overlaps an accepted booking"));
2625:                assert!(rejected[0].payload["reason"].as_str().unwrap().contains("overlaps an accepted booking"), "{case}");
2793:                assert!(refused[0].payload["reason"].as_str().unwrap().contains("overlaps an accepted booking"), "B at {target}");
2938:                assert!(rejected[0].payload["reason"].as_str().unwrap().contains("overlaps an accepted booking"), "B at {target}");
3078:            assert!(refused[0].payload["reason"].as_str().unwrap().contains("overlaps an accepted booking"), "{policy:?}");
4196:            assert!(rejected.iter().any(|row| row["reason"].as_str().unwrap_or("").starts_with("UR-21: the burst overlaps an accepted booking")), "{case}: {rejected:?}");
1092:fn overlap_refusals(rows: &[serde_json::Value]) -> usize {
> plan · deepseek-v4.1-flash
[0m$ [0mcd /home/komatsu/.cache/ezsdr-fixes/2026-11-07-n47s5/review-tree && git show work/n47s5:design/09-mock-radio.md | grep -n "no in-flight front" | cut -c1-60; git show work/n47s5:design/07-radio-model.md | grep -n "hand-over horizon" | cut -c1-40
143:- **MR-17** On receipt at `now`, the Mock decides a `TxB
161:- **RM-15** Bursts on one transmit s
478:| 2026-11-07 | RM-15 | note 47 stage
> plan · deepseek-v4.1-flash
**Findings (all hold at `work/n47s5`; none dropped).**

1. **P3** `crates/ezsdr-radio-uhd/src/provider/control.rs:528`, `crates/ezsdr-mock-radio/src/lib.rs:1684` — the horizon refusal reuses the *overlap* text although the two regions are disjoint. Book B(40) at 5 000 (inside the 10 ms horizon), then A(40) at 2 000: `COMMAND_REJECTED` says "overlaps an accepted booking". The parity helper classifies refusals by that text (`crates/ezsdr-acceptance/tests/parity.rs:1092`), so the new horizon refusals are counted as overlap refusals; the new UHD test pins the wording (`crates/ezsdr-radio-uhd/src/provider/tx.rs:2188`); KL-36's quoted symptom now also matches a case KL-36 does not describe. VERIFIED (read).

2. **P3** `design/09-mock-radio.md:143` (MR-17) vs `crates/ezsdr-mock-radio/src/lib.rs:1402` — the commit gives the Mock two hand-over bounds: the refusal uses `c.start < at_or_after(now+W)` alone, while the Mock's own front (`tx_handed_end`) includes a held burst only when its first send fits (`start+first <= window_end`). `accept(9_995, Some(40))`, horizon 10 000, then `accept(9_000, Some(1))` is refused although the Mock's front model says nothing at 9 995 was handed (40-sample first send needs 32 → 10 027 > 10 000). Over-conservative, no invariant break, but MR-17's amended sentence no longer says which bound the refusal uses. INFERRED (read, not run).

3. **P3** `crates/ezsdr-radio/src/staging.rs:442` — no row mutates `handover_horizon`. Rows 81–84 pass explicit horizons into `accept`/`first_free`, so `now.saturating_add(in_flight)` → `now`, or dropping `at_or_after`, survives all four; only `ur_21_s5_b_handed_before_an_earlier_a_is_refused_at_receipt` and the parity test would kill it, and the gate never applies it. `anchor.find` on all 81 `N47S5` rows: each `old` matches exactly once at the tip. VERIFIED.

4. **P3** `KNOWN_LIMITATIONS.md` (no entry; KL-36 nearest, `:134-139`) — the new refusal is a user-visible Ceiling: a burst starting before an already-accepted burst inside the 10 ms window is refused although it overlaps nothing (workaround: submit in target order, or more than `W` ahead). AGENTS §2 asks for the entry in the same commit; the plan checked only KL-36, whose text is about a booking *inside* a short burst's region, a different limit. INFERRED.

Dropped as already fixed on `work/n47s5`: 0 (the rev-19 `meets` refactor keeps `early`/`may_be_handed`/`handover_horizon`, the reason strings, `tx_handed_end` and KL unchanged; `git diff dc57cb9 work/n47s5` and `git show work/n47s5:<path>` checked). Rules/sequences, the joinable-blocker (S4b), `first_free`'s skip, and the S6 re-geometry read sound: no P1/P2 found.

Not done: no build or test run (read-only, budget) — every verdict above is from reading code, tests and rows, not execution.


## Review 5j-1-2 (report tail)
> plan · deepseek-v4.1-flash
[0m$ [0mcd /home/komatsu/.cache/ezsdr-fixes/2026-11-07-n47s5/review-tree && grep -n "if untimed { None }\|pub fn place(\|pub fn book_untimed\|pub fn move_untimed\|pub fn region_move\|pub fn move_start" crates/ezsdr-kernel/src/coordinator/admission.rs crates/ezsdr-radio/src/staging.rs && grep -n "out.untimed = true" crates/ezsdr-kernel/src/session.rs && git show work/n47s5:crates/ezsdr-radio/src/staging.rs | grep -n "pub fn place_untimed\|pub fn move_untimed\|pub fn move_start"
crates/ezsdr-kernel/src/coordinator/admission.rs:161:                *requested_at = if untimed { None } else { admitted.requested_target.map(AbsoluteDeadline::new) };
crates/ezsdr-radio/src/staging.rs:267:    pub fn place(&self, mut from: i64, len: Option<i64>, now: i64, join_by: impl Fn(i64) -> i64, horizon: i64) -> Option<i64> {
crates/ezsdr-radio/src/staging.rs:282:    pub fn book_untimed(&mut self, at: i64, ordinary: i64, len: Option<i64>, now: i64, join_by: impl Fn(i64) -> i64, horizon: i64) -> Result<i64, Overlap> {
crates/ezsdr-radio/src/staging.rs:291:    pub fn region_move(&self, at: i64, len: Option<i64>, now: i64, join_by: impl Fn(i64) -> i64, horizon: i64) -> Result<i64, Overlap> {
crates/ezsdr-radio/src/staging.rs:304:    pub fn move_untimed(&mut self, old: i64, from: i64, len: Option<i64>) -> Option<i64> {
crates/ezsdr-radio/src/staging.rs:437:    pub fn move_start(&mut self, old: i64, new: i64, len: Option<i64>) -> Result<(), Overlap> {
370:                    out.untimed = true;
283:    pub fn place_untimed(&self, at: i64, ordinary: i64, len: Option<i64>, now: i64, join_by: impl Fn(i64) -> i64, horizon: i64) -> Option<i64> {
303:    pub fn move_untimed(&mut self, old: i64, from: i64, len: Option<i64>) -> Option<i64> {
436:    pub fn move_start(&mut self, old: i64, new: i64, len: Option<i64>) -> Result<(), Overlap> {
> plan · deepseek-v4.1-flash
Findings (5j-1/2, commit 85dc4ac; each re-checked against `work/n47s5`):

**P2 — `crates/ezsdr-kernel/src/coordinator/admission.rs:161` makes three accepted design texts false, and the tip never fixes them.** `design/06-kernel-coordinator.md:293` (KC-28 step 7: `admit_burst_target` … "sets `requested_at` when it advanced"), `design/02-stream-contract.md:242` (SC-23a: "both times are recorded") and `design/04-run-and-session.md:260` (RS-51: "record the original in `requested_at`") still state the unconditional rule; 5j-5b (tip, `1bc67d3`) updated RS-19, MA-14 and KC-28 step 6 only. Sequence: submit `start_repeat` with no `at` whose earliest instant is off the TX grid → the dispatched burst has `requested_at: null` (pinned by `rs_19_an_untimed_session_action_has_no_requested_instant`), while KC-28 step 7 says step 7 sets it and SC-23a says both times are recorded. An implementer following spec 06 would write the opposite of the shipped behaviour. VERIFIED (both texts and the code read; the rule is the intended M2, only the texts are stale).

**P3 — `crates/ezsdr-kernel/src/session.rs:370` marks every `Vocabulary` with no `at`, but `design/05-module-api.md:169` (MA-14, tip) obliges every such Action to be placed.** `start_rx` (`CompileRule::Command`) and an untimed `capture` (`UpdateParameter`) also get `untimed = true`; both Providers pass the flag only into the `TxBurst` arm (`crates/ezsdr-mock-radio/src/lib.rs:1494`), so MA-14's "A Provider places an `untimed` Action at the first instant its target can take it … and does not judge it late" is false for them — and no test covers an untimed non-burst Action. Contradicts RS-19's "each Vocabulary decides what the mark means" (design/04:181). VERIFIED (mark breadth read at 85dc4ac, MA-14 read at the tip).

**P3 — `crates/ezsdr-radio/src/staging.rs:304` duplicates `move_start` (`staging.rs:437`).** Both find the head, refuse a joined piece that is not a head, compute `join` from the head, call `remove_head`, and push `Chain { start, end: len.map_or(i64::MAX, |l| start + l), last: start, front: front(start, len, false), join }`; they differ only in the overlap decision (`meets`/refuse vs `place`). Two copies of the same chain rewrite must change together, or the two dispatch moves diverge (e.g. the `join`/`end` rebooking rule). Extract the tail (`replace_head`) or have `move_start` take a placement closure. VERIFIED.

**P3 — `staging.rs:267` `place` does `front(k, len, join)` (`start + len + 2`, plain arithmetic) with no precondition, and both Providers now call it before their overflow guards:** UHD `control.rs:515` `place_untimed` precedes the `try_instant` refusal (~`:554`); Mock `lib.rs:1657` precedes its `start_v`/`end` checks (~`:1705`), so rev 27's ruling 2 ("overflow before booking") does not cover the placement's own arithmetic. Sequence: a Session burst at an instant near `i64::MAX` with a 40-sample waveform → `front` overflows; release builds wrap to a negative front (region test false) and the refusal comes later, debug builds panic on the control thread before any refusal. No test reaches it (the plan records the residual, but only for the order in `accept`). INFERRED.

Dropped (fixed later, not counted): 5j-1's third test `rs_19_an_untimed_session_action_has_no_requested_instant` and row 111 are absent here (5j-5a, `35f570a`); the plan's §5 table still names `run_session.rs` though rows 107/108/111 correctly use `test_bin: coordinator` — 2 dropped.

Could not do: no build/test run (so the twelve pure tests' passing and rows 112–115 KILLED are read, not re-probed; row 112's `k + 1` and row 114's `if let Some(_)` were checked with `anchor.py` only for uniqueness, all 23 rows ≥107 unique at the tip), and no Mock/UHD-behaviour comparison beyond reading.


## Review 5j-3-5 (report tail)
> plan · deepseek-v4.1-flash
[0m$ [0mcd /home/komatsu/.cache/ezsdr-fixes/2026-11-07-n47s5/review-tree && grep -n "placed at the first free sample\|An untimed booking starts" design/07-radio-model.md | cut -c1-120; grep -n "rm_15_an_untimed_booking_meeting_a_booking_beyond_the_horizon" crates/ezsdr-radio/tests/staging.rs
162:  A booking that cannot start at its requested sample because of device state follows its late policy; one that meet
482:| 2026-11-07 | RM-14, RM-15, RM-16, RM-37 | [note 47](../47-no-change-while-streaming.md) stage 5, sub-s
799:fn rm_15_an_untimed_booking_meeting_a_booking_beyond_the_horizon_is_placed_after_it() {
> plan · deepseek-v4.1-flash
**Findings (5j-3..5b, `85dc4ac..1bc67d3`; tip of `work/n47s5` is this same commit, so nothing was dropped)**

1. **P2** `design/07-radio-model.md:162` — new RM-15 text says an untimed booking "starts at the first sample from which its region meets no accepted region and no Stop region, at or after `max(at, ordinary)`". `Bookings::place` also skips a chain the owner may already have handed (`first_free`/`may_be_handed`), which no region meets. Sequence (the commit's own test `rm_15_an_untimed_booking_meeting_a_booking_beyond_the_horizon_is_placed_after_it`, `crates/ezsdr-radio/tests/staging.rs:799`): B(10) at 120, horizon 110, untimed A(30) at 100 → code returns 131; RM-15 as written returns 100. Wrong outcome from reading the spec: a 30-sample burst started inside a chain already handed. VERIFIED (text and test read).
2. **P2** `design/05-module-api.md:169` — MA-14 gained "A Provider places an `untimed` Action at the first instant its target can take it (RM-15) and does not judge it late". That makes a Radio-Model burst rule a Kernel-level obligation for *every* untimed Action, and contradicts RS-19's sentence added in the same commit (`design/04-run-and-session.md:181`, "Each Vocabulary decides what the mark means; for a radio burst, RM-15 says"): an untimed `capture` (`rs_19_capture_asap_records_applied_time`) is now told to be placed by RM-15. Also extra against plan §6, which asked for the `untimed` field only. VERIFIED.
3. **P3** `design/18-uhd-radio.md:249-250` — `ur_21_an_untimed_send_is_placed_after_its_delivery` is cited twice in the same UR-21 `Test:` line (with two different glosses); the second citation adds nothing. VERIFIED.
4. **P3** `crates/ezsdr-acceptance/tests/parity.rs:2131` — the two new parity tests were inserted between the doc comment "What each Provider refused among transmit values and bursts…" and its function `tx_decisions` (now `:2205`), so the comment documents `s15_parity_untimed_booking_after_stop` and `tx_decisions` has none. VERIFIED.
5. **P3** `crates/ezsdr-mock-radio/tests/mock_radio.rs:1282` — `mr_16_untimed_drop_and_flag_is_placed_at_the_stop_region_end` builds no Stop and asserts only "one record, no `TIME_ERROR`"; it never asserts the placed `target`, so a placement at the wrong sample (e.g. `ordinary` instead of a Stop front) still passes, and the name claims a case the rig does not build. VERIFIED.
6. **P3** missing tests vs plan §3: sequence 8's Mock test (`mr_16_untimed_off_grid_earliest_has_no_requested_target`) does not exist, and no test asserts an untimed finite burst's *record* `requested_target == None` (only the repeat case and the Kernel's dispatched Action do); sequence 6's Mock test `mr_16_aligned_send_asap_inside_the_stop_region_moves` is also absent (this one is recorded in rev 27, the first is not). VERIFIED (grep, no hits).
7. **P3** `crates/ezsdr-radio-uhd/tests/fake.rs:2036` — `ur_21_s19_untimed_switch_takes_its_timed_twin_boundary` allows one whole repeat period (1 000 samples); a boundary one period off passes, and the timed twin uses `DEVICE_LEAD_NS` rather than the Kernel's lead (rev 27's recorded deviation), so the test cannot distinguish "same boundary" from "one period later". VERIFIED.

No P1 found: I traced the receipt→dispatch invariant (untimed placed at `max(at, ordinary)`, `Held.untimed` overriding `SendAsap`/`Drop`/`PlanViolation` at `tx.rs:1015-1035`, silent `move_untimed`, `Held.policy` reuse at `tx.rs:457`/`:353` harmless because `start_repeat` is `SendAsapAndFlag`) and found no accepted-at-receipt burst that is refused or `TIME_ERROR`ed at dispatch, and no Mock/UHD answer that differs (`late_by` is a ceiling on both: `time::to_ns` vs `Core::late_by`).

Checks I could not do: no build, test, clippy, Python suite or mutation row was run, so the parity/Session/UHD tests are judged by reading; bench/loopback behaviour unverified.

