<!-- opus-s2a-plan2-final.md -->
**PLAN NEEDS CHANGES.** I worked read-only on HEAD 35afe16 plus the note-47 edit and built or ran nothing. I wrote my own failure scenarios first, then checked rev 2 (`~/.cache/ezsdr-fixes/2026-11-04-n46/test-plan-s2a.md`).

**Sol's 7 findings:** 1, 4, 5, 6 and 7 are closed except for the minor gaps below. Finding 2 is not closed (M1, M2). Finding 3 is closed for the fake and the Mock, but S6's in-source tests still depend on code 2a-1 removes (M3).
- 5: v58_11 checks out: origin T0+51 ms, 49 ms × 2 Msps = −98 000. kc_24 uses a refused TX change while the Run is Running, with a Kernel fixture as fallback.
- Leftovers: plan:163 still describes the dropped `ur_25_a_start_rx_counts_its_lead_…` with a recorded `ready` (4). control.rs:7327 reads `is_configured()`, which C5 removes (6).

**Major**

M1. Appendix A is incomplete; "no other item is open" (plan:207) is false. About 120 tests are missing from Appendix A and §3. Each one changes an RX value or a membership at run time, calls a helper the plan deletes, or depends on receive `Kind::Cold` behaviour that D4 removes. 2a-1 cannot be green at its review with these left undecided.
- Mock receive gain updates: mock_radio.rs:2919, 2934, 2949, 2964, 3011, 3124, 3137, 3168, 3186, 3279.
  - Sequence: push a `radio.rx.c0.gain_db` hardware-timed update; MK2 refuses it; `late_commands` and `applied_of_key` come back empty; the test fails.
  - Their subject (release window, queue slots, rows at stop) is kept for TX (D6).
- control.rs in-source tests that call `book_timed` with an RX gain or frequency (about 35): 3060–6583, 7661, 8238, 8290, 8485. U3 drops that RX arm. Their subjects (window floor, rows at stop/loss, queue slots) belong to the kept TX timed path.
- Mock tests built on `Ma1::tune(FREQ0="radio.rx.c0.frequency_hz")` (mock_layout.rs:3271): mock_layout.rs:3274–3878 (16 tests).
- Other Mock tests with RX updates: mock_layout.rs:1433, 1475, 3031; mock_channel.rs:966, 1018. Membership: mock_radio.rs:2810, 2842; rx.rs:3512; control.rs:4329, 4863.
- Callers of deleted helpers:
  - `covering` (D3): timeline.rs:1535.
  - Move-prune helpers: timeline.rs:883, 890, 929, 936.
  - `waiting_move` and debt helpers: core.rs:2406, 2422, 2728, 2795, 2884, 2910; mod.rs:839.
  - RX held-frequency helpers: control.rs:8475, 8480, 8617, 8622.
  - Move-recovery helpers: fake.rs:5846–5866.
- fake.rs:2738 schedules an RX gain, so the Run now fails at Validate. §3's "delete the entry or the test" leaves fake.rs:2708 and 4613 undecided; both should move to TX keys.
- Receive `Kind::Cold` unit tests whose behaviour D4 changes: timeline.rs:1665, 1712, 1754, 1782; rx.rs:4048, 4188, 4241.
- Python:
  - test_easy_api.py:118 expects `Rejected`, but the new client `Error` is not a `Rejected` (session.py:29), and the log loses its rejected entry. Use the TX setter.
  - test_easy_api.py:184 sets an RX rate.
  - test_easy_api.py:360 expects `RunEnded` from an RX setter after stop.
  - At test_easy_api.py:546, removing the setter breaks `len(continuity)==2`; use stop/start instead.
- Update-class tables that assert RX classes: radio_model.rs:106, channels.rs:118.
- Bench tests compiled with `--features uhd`: hardware.rs:698, 708, 914, 2574, 3195, 3231.
- Fix: rescan by behaviour, not by name:
  - update calls naming RX or stream keys, including keys bound with `let`, and `Ma1::tune`;
  - receive `rate()`/Cold items;
  - callers of every FIXTURE/CODE or deleted helper;
  - schedules and Python setters.
  Then decide each by subject and recount Appendix B. My 60 direct hits are in `/tmp/claude-1001/-home-komatsu-local-work-Ez-SDRv4/8aebdd6c-e346-4ee3-8da4-65b3b7a7fb57/scratchpad/miss3.txt`.

M2. Some DELETEs are wrong: their subject survives on TX or on Stop/start_rx. Each is the killer of a row in kept code that Appendix B marks "KEEP + NEW KILLER". All 153 such rows carry the same generic text and none names its killer, so the dispositions are not frozen.
- mock_radio.rs:3434, 3475, 3499, 3570 test `refusal()`'s candidate rules, which stay for TX (MK4). Rows P606, P614, P615, P618, P621.
  - Sequence: TX stream 1 Msps, `cold_change_refused` fault at 1.25 ms, TX rate change at 0.5 ms. Expected: nothing refused, fault unapplied.
  - Mutate `fault.tick <= e` and the 0.5 ms change is refused; no surviving test sees it.
  - Fix: KEEP-TX-KEY.
- mock_radio.rs:1971: its subject is misread. It tests rx_overflow/sequence faults in a gap between clocks, which Stop→start_rx still produces. Fix: KEEP-INITIAL.
- mock_layout.rs:2598 (a timed update during a refused halt is recorded) and control.rs:3246 (#64 no-channel settlement; `channel_live`'s halt check stays per U2; row J70). Fix: KEEP-TX-KEY.
- fake.rs:3889 (start lead over a long receive call, a kept part; rows B15, B22), fake.rs:8051 and fake.rs:2987 (plan following via Stop/start_rx/Run stop; row R15). Fix: KEEP-INITIAL. fake.rs:8051 is marked DELETE but its text says "KEEP-INITIAL if reachable". It is reachable: Stop s1, start_rx s1, hold the owner's configure, Provider::stop. Row P473.
- fake.rs:1425: the raw `queue-rx-frequency` condition (measurement.rs:938) is part of UR-34's raw record, which X3 keeps.
- v58.rs:567: a TX rate beyond the envelope at run time is still reachable, and this is the only acceptance test of the `ezsdr.coercion` refusal at admission.
- Fix: correct these and name the killer of every "KEEP + NEW KILLER" row before bulk deletion.

M3. Some KEEPs cannot pass as decided.
- control.rs:2734: on TX a refused segment stays in the plan, ended at its origin (timeline.rs:990–1000), and the test's assertions are receive-only. Fix: KEEP-INITIAL with a refused start_rx.
- timeline.rs:1951 asserts through `covering`, which D3 deletes. mock_layout.rs:3560 asserts an RX timed-versus-untimed draw. Fix: DELETE both.
- rx.rs:2115 and rx.rs:1484 assert only debt, left, restore and drain state (C1, X1). Fix: DELETE, or move the parts that survive to named tests: a refused stop leaves the device Uncertain; an abort publishes nothing after its cut.
- S6's kept tests rx.rs:3940, 3960, 3991, 4011, 4346 book receive `rate(…)` Cold items, which D4 removes. Fix: rewrite their setup as Stop then `Kind::Start`.

**Minor**

n1. 2a-1 ignores too few tests. These from-none tests are outside the 29 and fail in 2a-1: control.rs:1507, tx.rs:4234, tx.rs:4246, and the NEW-2a-2 test control.rs:1523.
- KEEP-TX-KEY rows that set up with `channels_from_none` need a named TX setup: control.rs:2062, 2195, 3428, 4221, 4576, 4657, 4780.
- For control.rs:2062, 4576 and 4780 the stated UNASSIGNED setup gives no segment or projection; use the pending segment of a booked TX rate change.

n2. Section 8 / S10. The UHD owner route is VERIFIED:
- an owner with no clock switches at once (tx.rs:382–389);
- `owner_configure` runs before the clock is adopted and before any send (tx.rs:1150–1186);
- a failed configure keeps the streamer open, so the corrected change takes `Owner::Configure` (core.rs:1782–1786).

What follows from that:
- (a) T3 is not on this route. Drop the T3 clause from S10 (plan:125), or add a variant where the open is refused.
- (b) "Rejected reasons equal on both" cannot hold: the Mock's fault reason and the UHD device reason differ. Normalise them as parity.rs:916–923 does.
- (c) The UHD Session test must wait for the refusal (its event, or `is_idle`) before submitting the corrected change. Otherwise the change books onto a plan that has not been refused yet, and the test exercises nothing new.
- (d) The two "new" Mock TX tests duplicate mock_layout.rs:2624, 2711, 2823. Name those as killers instead.
- (e) Until stage 4, `ready` is the booking time. Record that the configure can eat the start lead (INFERRED). Physical safety of the retry stays INFERRED, as the plan says.
- Rows P1404, P1405 and P1409 have no exact anchors yet.

n3. KNOWN_LIMITATIONS inventory: KL-24's workaround, a Session `set` sweep (KNOWN_LIMITATIONS.md:222–224), is now refused for receive values. KL-33 no longer applies.

**Item 3, READ entries:** rev 2 marks only one, control.rs:4958, repeated in the shared text of 11 rows. Decision: KEEP-TX-KEY for the projection half (TX frequency held at 320 ms, TX rate change at 300 ms; the switch's configure carries it). Drop the R1 "applied untimed at its release" half, which depends on RX `covering`.

**Item 5, S6 retirements:** the evidence is correct (VERIFIED). fake.rs:7105–7290 all start with `stream=-1` and reach their first open through a runtime enable; with membership fixed, the first open is arm's. The kept coverage is enough (fake.rs:5757, 5779; mod.rs:850; tx.rs:1666, 2211), once M3's rewrite is done.

**Sampled entries (37 DELETE, 21 KEEP):**
- DELETE right: mock_layout.rs:3476, 2006, 3421, 2519, 1035, 3069, 1071; mock_radio.rs:3541; control.rs:1816, 5998, 4034, 2023, 1991, 4377; tx.rs:4469, 1450; core.rs:2939; rx.rs:2507; fake.rs:3814, 8082 (S18 relocated to fake.rs:8195), 2808, 6837, 2842; hardware.rs:1613; parity.rs:676.
- DELETE wrong: mock_radio.rs:3434, 3475, 3499, 3570, 1971; mock_layout.rs:2598; control.rs:3246; fake.rs:3889, 8051, 2987, 1425; v58.rs:567.
- KEEP right: control.rs:3779, 2227, 4657; fake.rs:7596, 5872, 2098; mock_radio.rs:2737, 2247; mock_channel.rs:853; core.rs:3255; parity.rs:1515, 885 (needs a TX envelope refusal); v58.rs:296; control.rs:4958 (READ, decided above).
- KEEP wrong, or setup wrong: control.rs:2734, control.rs:2062 (setup); timeline.rs:1951; mock_layout.rs:3560; rx.rs:2115, 1484.

**Not checked:** Appendix B row by row (only the rows killed by sampled tests); the spec and Vision §58 text; the S8 generator; kc_24's body. I did not read Sol's partial log, to stay independent. My scans are regex heuristics, so expect some false positives; I read about 30 of the hits by hand.
<!-- opus-s2-a-recheck.md -->
**Verdict: NEEDS CHANGES.** I found 3 major and 6 minor problems. I worked read-only on the tree. All builds and test runs happened in copies in my scratchpad, using target `Ez-SDRv4-review` (5.7 GB). One copy was HEAD plus only the class flip, to see which tests the flip breaks. The other was the working tree, where I ran four probes. Before reading the table I wrote down the failures I expected to look for:
- a refusal at `start_rx` that spreads to another stream, or that is never cleared;
- the time a long receive call takes at the old rate eating the start lead;
- a fault in the gap between a Stop and the next start leaking into the next segment;
- uhd-control putting a Stop and a `start_rx` at the same instant in the wrong order;
- tests that now pass only because their stimulus is refused;
- bench probes recording refusals as if they were data.

**Major**
- **M1. H4 was deleted whole, but note 47 §4 says to keep part of it.** The note says 「H4 のその観測は退け，送信の余裕と負荷の測定だけを残して範囲を書き直す」 (drop the dispatch observation, keep the transmit headroom and load measurement, rewrite its scope). The brief's DELETE list names only "H4's control-dispatch proxy". Gone at HEAD: `tx_headroom` and the clock mapping (h4.rs:75–171), the CPU and thread parsers (:276–319), the 12 unit tests in h4.rs and `hw_h4_dispatch`. With only the flip applied, these 12 tests pass, and so do 2 `tx_start_lead_*` unit tests in measurement.rs. None of these 14 has a table row. The plan says the tests "measure Run-time rate transitions", but H4 sets its rates only as setup.
  - Fix: restore everything except the dispatch proxy. Rewrite the probe so it asks for receive and transmit c0 at 10 MS/s at connect, runs the repeat with no timed updates, and records headroom and CPU/thread load. Add the table rows.
- **M2. Several deletions take the only pin of a behaviour that survives, and each has a workable initial-value or Stop/`start_rx` stimulus.** Part (b) is unsafe after these.
  - (a) Refusal halt. Two fake.rs tests are gone: fake.rs@HEAD:7564 `ur_25_a_refused_change_of_a_later_stream_halts_that_stream_alone` and fake.rs@HEAD:8218 `ur_25_a_successful_change_clears_a_later_stream_s_halt`. Their behaviour is on the must-survive list.
    - Probe I ran on the working tree: `rx_two`, `WrongRate{claimed 1e6, applied 0.9e6, nth 1}`, then Stop rx/s1 and `start_rx` rx/s1. Result: `COMMAND_REJECTED "UR-12: the device applied 900000 S/s…"`, the `rx_start 1` count does not change, s1 has one clock and it is ended, s0's clock stays live and still delivers. A second `start_rx` gives `rx_start 1` +1 and a second clock.
    - The only pins left are in-source tests that reach the refusal through a Cold rate item (rx.rs:3462, 3639, 3692, 3797). Part (b) deletes those, so a refusal that halts s0 too, or a halt that never clears, would go unnoticed.
  - (b) fake.rs@HEAD:3889 `ur_25_a_cold_change_booked_anywhere_in_a_long_receive_call_is_on_time`. The brief's M2 said keep it, and the start lead at the old rate's call is must-survive.
    - Stimulus: 390 625 S/s, `block_len` 65 536, Stop then `start_rx` at the test's six phases. Assert origin ≥ booking + 50 ms + max(block, packet)×512 + 3 ms, the switch before the origin, and no `LATE_COMMAND`.
  - (c) mock_radio.rs@HEAD:1971 `mr_20_a_fault_between_cold_clocks_is_not_applied`. M2 said keep it.
    - Stimulus: Stop, then the fault in the gap, then `start_rx`. Assert the fault is unapplied with `lost` 0, no `RX_OVERFLOW`, and the new clock's first block is tick 0 with `lost` None and flags NONE.
  - (d) mock_layout.rs@HEAD:570 `mr_12_d14_the_actions_of_a_layout_instance_name_a_stream`. Only lines 576–577 and 582 are Run-time updates. The D14 target refusals (MR-16, MR-18, MR-29, and MR-25 for a Stop to `mock/rx/s9`, which no other Mock test pins) and the per-stream Stop/`start_rx` all survive.
    - Fix: rewrite all four as REWRITE-INITIAL.
- **M3. The uhd-control VH differential was deleted whole** (control/differential.rs; HEAD :111, :159–163 and :502–506 book Stop, StartRx and loss through `Control`). The Mock side kept its differential with Cold and Timed filtered out (timeline.rs `layout_sequence`). Layer 3 (rx/differential.rs:821) books generator items directly, not through uhd-control.
  - Nothing now cross-checks how uhd-control books generated Stop/`start_rx`/loss sequences (ties, instants out of order). Parity has one fixed sequence.
  - Example: a round of [Stop, StartRx] at one instant, then a loss. If uhd-control's same-instant order regressed, rx/s0 would stay stopped and both remaining differentials would still pass.
  - Fix: keep the module, filtered the way the Mock side is. Deleting `rm_26_the_ceiling_is_decided_from_the_sequence` is right.

**Minor**
- **m1 (your question ii). The stated reason for dropping transmit rows does not reproduce.** In `vh_8_the_same_manifest_plan_rows`/`_sequence` (parity.rs:1053), the table says UHD numbers initial transmit segments and Mock does not. I probed `planned()` with `rx_two`+`tx_two` asked at connect: both Providers give `sequence` None. Only the origin from T0 differs (Mock −400 200 000, UHD −400 251 600 ticks), and that is each rig's own arm instant. This is a parity weakening. Fix: keep the transmit rows and compare origins relative to each rig's arm, or check the lattice and compare (direction, stream, sequence, channels).
- **m2. The REFUSAL tests do not check why the change was refused.**
  - v58.rs:535 checks only `!violations.is_empty()`; v58.rs:556 and uhd.rs:192 check only `Rejected{..}`.
  - The connect refusals (v58.rs:528, uhd.rs:187) check only `Failed{..}`, not `radio.rf_envelope`.
  - v58.rs:1027 checks a Validate failure with no reason and no control case.
  - `v58_16` never checks that the rejection is logged.
  - Fix: assert `ezsdr.update_class`, the failure reason, and the log entry.
- **m3. Some tests that did not fail now pass without testing anything.** At mock_layout.rs:1001 and :1074, the Mock rejects both membership pushes ("MR-18: … not updateable"; my probe shows one clock and the test passes). fake.rs:3218 (its far cut is refused) and the first assertion of fake.rs:6282 are the same. Fix: rewrite the receive one with Stop→`start_rx` and assert `rejected()` is empty; hand the others to part (b).
- **m4 (your question i). Some bench tests would quietly record refused submissions as results.** These are `hw_b9_unplug_transmit_only` (hardware.rs:228), `hw_b9_transmit_only_session` (:1460; with `rx0_tx0` it is no longer transmit-only), `hw_56_stop_between_starts` (:2306) and the second half of `hw_ur17` (:2535, :2540).
  - Fix them with initial values: `tx_only`, an initial 50 MS/s with a Stop as the cut, and a pending segment made by `start_rx`.
  - These fail loudly: :725, :1628, :2132 (fixable with initial values), and `hw_ma1_phase_after_a_pre_start_tune` (:914, through `ma1_run`, common/mod.rs:958). The MA-1 test is missing from the 7 `?` items. Park them explicitly.
- **m5. Some surviving halves were lost.**
  - The overflow-companion half of `leads_and_the_overflow_gap_add_a_condition_with_the_other_direction_running` (fake.rs@HEAD:8826–8834) has no other pin.
  - The tail of `mr_21_overrun_shape` (mock_radio.rs@HEAD:2343) can use a Stop at 20 ms.
- **m6. Records and leftovers.**
  - recheck.tsv says v58.rs:567 was deleted, but it was rewritten. It says rx.rs:3512 and tx.rs:4234, 4246, 4469 were "not failing", but all four fail with the flip.
  - Stale text: the doc comment on `capture_action` (measurement.rs:2838); the unmeasured reasons at measurement.rs:220–224; plan fields at :707–708 and :740–741; the header at fake.rs:2534–2537; the comments at fake.rs:5324–5325.
  - The converters in `_PARAMS` (session.py:180–186) are no longer used.

**Notes**
- **Your question iv (start-lead groups).** Removing them matches note 47. But `rx_start_lead_ns` still governs every `start_rx` and has no measurement until stage 3. Record that in part (c).
- **Your question v (Mock differential).** Filtering is acceptable only as a stopgap. Part (b) should take Cold and Timed out of the generator and rebalance it: the Timed and TxCold classes now generate empty sequences.
- **Mislabelled refusal.** rx.rs:587 reports a `start_rx` refusal as `"update_parameter"` with no action id (my probe confirms it). This is now the only way a refusal happens; put it in the stage-3 brief.
- **Part (b) must keep:**
  - the Mock's per-channel value history, which no test exercises now;
  - rx/differential.rs layer 3, the remaining pin for cuts with 65 536-sample blocks and latency;
  - the in-source D14 twin at control.rs:5430.
- **For stage 3**, record the `ready` scenario of fake.rs@HEAD:2783.
- **Python (question 4) is right.** The setters always raise an `Error` naming `connect(<dir>=ezsdr.Stream(...))` and submit nothing (:156). The Kernel refusal stays pinned with `ezsdr.update_class` (:137, :176), and `RunEnded` through `sdr.set` (:372).

**Sampled DELETE rows (47; ≈ marks a deletion I accept with a caveat)**
- **mock_radio, right:** `mr_09_same_tick…`, `mr_17_a_burst_before…` (v58_11 covers it), `mr_25_a_loss_then_a_stop…`, `mr_25_a_stop_then_a_loss…`, `mr_20_a_loss_cancels…` (vh_8_loss… covers it), `mr_18_a_fractional_receive_cut…` (≈, the VH differential). **Wrong:** `mr_20_a_fault_between_cold_clocks…`.
- **mock_layout, right:** `mr_20_a_refused_transmit_burst…`, `rm_31_rm_32_…` (v58.rs:984–991 cover it), `mr_34_a_layout_receive_entry…` (the restart draw is pinned at :1036). **Wrong:** `mr_12_d14…`.
- **mock_channel, right:** `mr_34_a_random_phase…`, `mr_32_a_stop_after_a_cold_change…`, `mr_32_b_…_own_instant`.
- **fake.rs, right:** the three `ur_14_*`, `ur_25_a_long_block…` (≈), `ur_25_a_slow_link…` (≈), `ur_25_a_late_switch…`, `ur_25_the_first_segment_after_arm…` (:2584 covers it), `ur_25_a_receive_enable_counts…` (deferred by design), `ur_21_an_untimed_burst…`, `ur_24_stop_cancels_held_commands`, `ur_13_the_receive_owner_s_first_open…`, `ur_25_a_refused_antenna_read_back…` (:5306 covers it), `the_ma1_phase_probe…`. **Wrong:** `ur_25_a_refused_change…`, `ur_25_a_successful_change_clears…`, `ur_25_a_cold_change_booked_anywhere…`. **Partly wrong:** `leads_and_the_overflow_gap…`, `ur_26_d14_a_layout_instance_names_its_streams` (steps 4–6 survive).
- **In-source, right:** the three `ur_28` from-none tests, `ur_25_a_transmit_report_read…`, `ur_25_cold_change_does_not_inherit…`, the two lost-stream tests in control.rs, `rm_26_the_ceiling…`. **Wrong:** `rm_26_the_timeline_against_uhd_control`, the h4 module and `hw_h4_dispatch`.
- **Acceptance, right:** `kc_24_…` (coordinator.rs:953 covers it), `mr_34_…refused_change_span…`, `vh_8_cold_change_cancels_held_bursts`, `vh_8_tx_repeat_records…`, `vh_8_a_stream_one_refusal…`, `vh_8_tx_stop_cancels_a_burst_on_a_future_clock`.
- **REWRITE-INITIAL and REFUSAL:** all 47 rows still observe their subject, except the gaps in m1, m2 and m5.

**Not checked:** the other ~193 DELETE rows; mutation rows and `rules.py`; clippy and fmt; the `--features uhd` build; Python 3.9; specs and KNOWN_LIMITATIONS; a full test run of the working tree (it compiles; I relied on the implementer's green result).
<!-- opus-s2-a-fix-recheck.md -->
**Verdict: NEEDS CHANGES. All findings are minor.** M1, M2(a), M2(b), M2(d), M3 and m5 are closed. M2(c), m2, m3 and m4 each leave one gap. Since only minor fixes remain, no Sol re-review is needed. I worked in a scratchpad copy with target `Ez-SDRv4-review`: the workspace passed 1 988 tests with 0 failures. Every probe was reverted and the main tree is unchanged.

**Probes (each fails its test as intended)**
- **M2(a), halt never clears.** I changed the guard at `timeline.rs:929` to `!self.on`; `…successful_start_clears…` fails at `fake.rs:5842`.
- **M2(a), refusal halts both streams.** I made `refuse_segment` also book a Stop on every receive stream; `…refused_start…alone` fails at `fake.rs:5828`.
- **M2(b).** I dropped the receive-call term at `timeline.rs:1033`; the floor assertion fails at `fake.rs:5879`.
- **M3.** I added 1 000 000 to the receive Stop's arrival number at `control.rs:483`, so it sorts after a same-instant `start_rx`: 52 of the 142 StartRx-class sequences diverge in rx_clocks.

**Question 2 (the two `tx_start_lead_*` units): the deviation is right.** They tested `tx_start_trial_class` and `tx_start_candidate_stage_json`, which went with the TX group. They also exercised `summarize_candidates(MinimumLead)` and `numeric_value`, which `measured_rows_keep_the_selected_boundary_and_limiting_condition` (`measurement.rs:4775`) still pins the same way: two conditions, with the limiting one named. Side note: `tx_start_lead_ns` now governs nothing reachable, because a transmit segment only begins from a Cold item and no class allows one. Part (b) or (c) should decide whether the key stays.

**Question 3 (m1): only partly.** The check at `parity.rs:1095–1097` catches an extra or missing transmit segment, one after T0, and one off the 100-tick lattice. It misses an initial origin on the wrong lattice instant before T0 (rounded down, or before arm). `% 100` is also looser than stream 1's 200-tick lattice. Optional tightening: check `origin == lattice(arm, n) − T0` for each rig, using the UHD timing row `"arm".at` and arm = 0 for the Mock, as `vh_8_the_same_transmit_stream_clock` already does for the Mock.

**Minor findings**
1. **M2(c) is half vacuous.**
   - `mock_radio.rs:1928` reads `domain` from `last()` at 1.01 ms, when only the old clock exists. My probe shows one record there (origin 0, ended at 1 ms); the new clock (origin 1 010 000) registers later.
   - So the three first-block assertions check the old clock. Sequence: Stop at 1 ms, fault at 1.005 ms, `start_rx` at 1.01 ms. A Mock that marks the new clock's first block with GAP_BEFORE and `lost` but raises no event would pass this test; only the RX_OVERFLOW count and the Mock differential would catch it.
   - Fix: read the domain after `step(20_000_000)` and assert two clock records. I ran this version and it passes. Also, the comment says `start_rx` comes at 10 ms; it is at 1.01 ms.
2. **The m3 hand-off names the wrong test.** `test-plan-s2.md:417` lists `…unlinked_receive_stream…` as fine. The vacuous one is `mr_34_a_layout_deterministic_transmit_entry_keeps_phase_zero` (`mock_layout.rs:1123`): my probe shows both `radio.tx.c0.stream` pushes rejected (MR-18), one transmit clock, and the test still passes. It is in neither the table nor the part (b) list. Fix: list it, or delete it, since a transmit entry has no Run-time form.
3. **m2 missed one site (from part (a); my first review missed it too).** `common/mod.rs:873`, the B7 rehearsal that CI runs via `fake.rs:3103` and `:3131`, now accepts any `Rejected`. At HEAD it checked `radio.rf_envelope`. Fix: assert `ezsdr.update_class`.
4. **`hw_56` lost its subject.**
   - `hardware.rs:2194` now does Stop, then 120 ms, then `start_rx`: a plain restart. Issue #56 (`21-amendments.md:355`) is a Stop that lands between a start's e₁ and e₂.
   - The test is in `hw_measurements_record`'s raw probes, so a bench run would file a plain restart under #56.
   - Fix: Stop, `start_rx` (e₂ is its `rx_segment` origin), a second Stop about 10 ms later, then the current drain and recovery, keeping the e₁/e₂ placement field. Or park it with the `hw_b8` cold-change probes. The Provider side stays pinned at `rx.rs:3540`.
5. **Nit.** `fake.rs:5806` drains `link0` before the Stop, so the check at `:5829` ("still delivers after the refusal") can be met by blocks delivered before the refusal. The real pin is the clock check at `:5828`. Fix: drain just before `start_rx`.

**Question 4: no other new problem found.** `with_initial` and `tx_only` take explicit keys at the call site and hide no step. `tx_only`'s unused `_profile` matches the `fn(&Json)` shape that `candidate_session_asking` needs. No bench test submits a Run-time set any more.

**Notes**
- **Git index.** `h4.rs` is staged as deleted (`D`) while the restored file is untracked (`??`). `git add` it before the commit: `git commit -a` would commit the deletion while `measurement.rs:31` declares `mod h4;`. The gate rsyncs the working tree, so it will not show this.
- **For part (b):** `ProbeDevice::matching_timed_apply` (`probe_device.rs:135`) is now used only by its own test at `fake.rs:7466`.
- **For part (c):** `design/18-uhd-radio.md:431–432` (UR-34) still describes `hw_h4_dispatch` and its deleted tests.

**Not checked:** clippy and fmt, the `--features uhd` build, the Python suites, `rules.py` and mutation rows, bench runs (no USRP), the other DELETE rows, and the specs and KNOWN_LIMITATIONS.
<!-- opus-s2-c1-unkilled.md -->
# Re-check: note 47 stage 2 part (c1) — the 62 rows deleted as "no surviving test kills the mutant"

Re-check reviewer (Claude Opus, read-only), 2026-11-04. Tree: HEAD `45a4294` + parts (a), (b), (c1) uncommitted, snapshot rsynced at 21:33 to `/tmp/opus-c1-recheck/copy` (part (c2) was editing `design/`, `crates/ezsdr-sim`, `schemas/sim` meanwhile; line numbers below are the snapshot's, which still equal the tree's for every file cited). Probes: `~/.cache/ezsdr-fixes/2026-11-04-n46/work/opus-c1-probes/` (`probe.py`, `batch1..9.json` + `.result.json`, one log per probe and target; the probe tests I added in the copy are in `opus-probe-tests.diff`, 15 tests named `opus_probe_*`). Nothing was run in or written to the repository.

## My list first: surviving behaviours at risk of losing their pin (written before classifying)

1. RX restart timing on both Providers: cut + release window + call at the old rate + allowance + lead, the lattice, a past `at` taken at the receipt, never before T0 (RM-25, VH-4, MR-29).
2. uhd-rx after an untimed stop: the old stream's tail in flight is not published on the new clock (`not_before`, Review N B1); a Timeout after a cut waits the quiet span before ending (B2).
3. UR-29 silence check while a cut is pending.
4. Missed-start retry (LateCommand restart) and loss: no device call after loss.
5. Owner configure: one configure per segment command (UR-36), stale-plan guard, loss between two channels, refusal halt, and the `start_rx` configuration snapshot.
6. Clock registration and end order (TX clock per stream on its own node; RX clock at its first block, ended at its cut; orphan handling).
7. MA-14b / KC-21a: each Action booked before the next `recv()` (O2's premise).
8. MockRadio: a per-stream TX Stop cuts only its own stream in the shared channel plan.
9. MockRadio Manifest `segments` rows (channel order, `ended_at`) and a link-less receive stream running nothing.
10. MockRadio untimed tune at a `start_rx` origin: which channels redraw, in what order.
11. RM-15 `decided_at` and the TX front across clocks.
12. Fault kinds and their Run-end records (`cold_change_refused`).
13. RX chain lock discipline (L1); claim/read exclusion (UR-25).
14. The Providers' guard refusing a classless update reached directly.
15. Not in the 62 rows but on the must-survive list: MockRadio per-channel value history, F8, T3, UR-28 generation correlation, `ready` (parked for stage 3).

## Counts

RETARGET (wrongly deleted: an existing test kills it) 4 · OWED 15 · EQUIVALENT 24 · DEAD 19 = 62.

Method findings behind the 4 RETARGETs: the c1 "wide" probe never ran `ezsdr-mock-radio --test timeline` (the Mock-vs-oracle differential) for any Mock row except V06/T40, never ran `ezsdr-radio`'s own tests for V06, never ran acceptance `v61/x3xx/reactive/uhd/measurements`, `ezsdr-server --test protocol` or Python, and probed T40 without its `old2/new2` (`batchB.json`), so it tested a weaker mutant. I ran the missing binaries for all 52 live rows (`batch1-3`): only F36, P1220, V06 and T40 die there. Separately, P141–P152, P165 and P173 were hand-deleted in `work/c1/decisions.py` and never probed; their anchors match nothing today, so DELETE is right but rows-s2.md's reason ("no surviving test kills") is wrong for them.

## Table

Format: id | class | evidence (today's path:line; HEAD killer and its stimulus) | action.

B06 | OWED | rx.rs:530 `first_tick < stream.not_before`, live via Stop+`start_rx` (not_before = `last_stop`, rx.rs:291, 382). HEAD killer `fake.rs@HEAD:3802 ur_25_a_late_switch_drops_the_old_stream_s_tail` used a cold rate change on a 60 ms link. Probe `opus_probe_b06` kills it (baseline: origin 472 573 400 < untimed stop 473 243 639, first new-clock block at 495 243 800) | Sequence: `Direct::build(bench_link(60))`, settle 300 ms, `stop_at(rx/s0)`, `start_rx_at(rx/s0, None)`, settle 800 ms → the new clock's first block is not before the `rx_stop_untimed` row's `at`. Same config in stage 2, so the tail's values are valid samples; the rule is what stage 3 (new values per start) needs. Put it with B07 in one new fake.rs test beside `ur_25_a_start_booked_anywhere_in_a_long_receive_call_is_on_time` (fake.rs:5842).
B07 | OWED | rx.rs:510 `quiet && now >= cut + span`, live via Stop (any cut). HEAD killer was the same 60 ms link test. My 3.5 ms/390 625 S/s probe did not reproduce it (4 restarts); `opus_probe_b07_a_60_ms_link…` kills it | Sequence: as B06; assert the first (stopped) clock's last delivered sample index == its `ended_at` index. Same test as B06.
B13 | EQUIVALENT | rx.rs:511. `Action::Stop` has no `at`, so every receive cut is booked at its receipt and floored at the delivered sample (timeline.rs:484-488): when uhd-rx reads a cut, `now >= cut`, and `cut.is_none_or(now < cut)` ≡ `cut.is_none()`. The far cut was a timed cold change. Probe: survives `opus_probe_b07_a_60` | Delete (done). Optional: simplify to `cut.is_none()`; stage 3 adds no far receive cut.
F36 | RETARGET | lib.rs:1502 (MockRadio line lead). Killed by `ezsdr-mock-radio --test timeline rm_26_the_timeline_against_mockradio` (tests/timeline.rs:192; x3xx terms lead 50 ms built independently at :163-168; log `F36.ezsdr-mock-radio.timeline.log`) | Restore the row: crate ezsdr-mock-radio, test_bin `timeline`, filter `rm_26_the_timeline_against_mockradio`, anchor unchanged.
J20 | EQUIVALENT | lib.rs:1635. Mock TX clocks are registered once at `arm` with origin = lattice(arm) (lib.rs:2166-2172) and none later (tx_reconcile adds a clock only past the first plan segment), and bursts are handled only after start (≥ T0), so `decided_at(now, origin, lead)` = now. UHD keeps RM-15 pinned through a constructed future-origin clock (control.rs:750-772) | Delete (done). Stage 4 (configured bursts register clocks with future origins) needs a Mock row again.
J21 | DEAD | core.rs:697-703, `Streams::reconcile`'s loop ending clocks beyond the plan. Needs clocks > plan segments; a TX stream has ≤ 1 clock (control.rs:186) and its plan keeps its one segment (TX cuts are kept, timeline.rs:492) | Region core.rs:697-703 (no surviving caller reaches it: the only producer was a change booked after Provider::stop). Stage 4 revives it.
J62 | EQUIVALENT | core.rs:556 (`changed()` after `refuse`). `refuse` is called only for receive (rx.rs:408); for an RX key `changed` only bumps `version`, read only by uhd-tx for its own unchanged plan (tx.rs:331), and `reconcile` is TX-only (core.rs:677). c1 probed the re-anchored form | Delete (done).
M51 | EQUIVALENT | lib.rs:1186 `self.cut_segments(k, cut)`; the mutant's reason "MR-18: cancelled by a cold change" no longer occurs anywhere (grep), so its guard is always true | Delete (done).
M68 | OWED | lib.rs:914 `*stream == k && *g == generation`. The stream half is live: a per-stream TX Stop calls `stop_tx(k)` alone (lib.rs:1528-1545) and the plan is shared by all TX streams (lib.rs:2104-2108). HEAD killer `mr_32_a_stop_after_a_cold_change…` pinned only the generation half. Probe `opus_probe_m68_m69` kills it | Sequence: ideal, tx c0→s0, tx c1→s1, coupling tx c1→rx c0; burst on tx/s1 at sample 5 000; Stop tx/s0 at 1 ms; rx c0 hears 0.5 at 6 ms. New mock_layout test next to `mr_25_a_device_stop_stops_every_layout_stream` (mock_layout.rs:791).
M69 | OWED | lib.rs:912 retain by stream/generation; same reachability; same probe kills it (s1's held segment starting after s0's cut is withdrawn) | Same test as M68.
M70 | EQUIVALENT | lib.rs:1391 `tx_gen`: `cur` ∈ {None, Some(0)} (one TX clock per stream), so `index + 1` = 1 | Delete (done). Note: the generation key component is now constant state.
P1142 | DEAD | tests/common/probe_device.rs:295, the `host_entry_ns` field of the probe's timed-apply row: no reader after H4's dispatch observation was withdrawn (grep `host_entry_ns`: only probe_device.rs). The timed apply itself is still issued by the raw-group measurement tests (fake.rs:1003-1420) | Region: the `host_entry_ns` field (probe_device.rs:295) and its capture (:357).
P1169 | EQUIVALENT | tx.rs:257. The TX header's contract is read by nobody: the header goes only to `BurstTracker::on_block` (tx.rs:682), which never reads `contract` (kernel stream/burst.rs:296-330), and uhd-tx publishes no block | Delete (done). Maintainability note: the `contract` Option take/restore (tx.rs:225, 257, 661, 685) is dead weight.
P1220 | RETARGET | lib.rs:1504 (RX release window). Killed by `rm_26_the_timeline_against_mockradio` (log `P1220.ezsdr-mock-radio.timeline.log`) | Restore as F36 (test_bin `timeline`).
P1231 | EQUIVALENT | rx.rs:298-299. Since part (b) removed uhd-control's timed hand-over (HEAD control.rs: 115 `rx_chain` uses; today 0), `rx_chain[k]` is locked only by stream k's own uhd-rx thread, by mod.rs:650 after a failed spawn and by cfg(test) asserts; no other thread can see the gap | Delete (done). Note: the chain needs no shared lock any more (stage-6 or Phase 8-end cleanup).
P1244 | EQUIVALENT | rx.rs:365-370; same single-locker argument | Delete (done).
P1259 | EQUIVALENT | rx.rs:486-489; same; probe `opus_probe_p1290` survives it | Delete (done).
P1290 | OWED | rx.rs:488, the loss check before the missed-start retry (must-survive: 開始の逃しの再試行). HEAD killer was a timed RX hand-over. Probe `opus_probe_p1290` kills it | Sequence: rx.rs unit `rx(&core,1)` on a LockSpy; `core.device_lost(..)`; `rx.receive(RxRecv::LateCommand)` → no further `rx_start`. New rx.rs test beside `ur_24_a_loss_published_before_rx_recv_skips_the_device_call` (rx.rs:1026).
P141 | DEAD | anchor matches nothing; the `updates` floor of `handle_update` was deleted in part (b) (grep `update.class == UpdateClass`: 0) | Already deleted; fix rows-s2.md's reason.
P142 | DEAD | same as P141 | Already deleted; fix the reason.
P143 | DEAD | `cold_streams` deleted (grep 0) | Already deleted; fix the reason.
P148 | DEAD | the timed-update floor in `handle_update` deleted | Already deleted; fix the reason.
P149 | DEAD | move floor deleted (`book_move` grep 0) | Already deleted; fix the reason.
P150 | DEAD | `MoveError` arm deleted (grep 0) | Already deleted; fix the reason.
P151 | DEAD | `lines.book_move` deleted | Already deleted; fix the reason.
P152 | DEAD | `for k in [from, to]` move loop deleted (grep 0) | Already deleted; fix the reason.
P153 | OWED | lib.rs:1320. A link-less stream gets a line with a Stop at T0 (lib.rs:2221-2226); a `start_rx` on it (accepted, "carries out nothing", D14) puts a segment in its plan, and only this check keeps it from beginning. My first probe (no start_rx) missed it; with `start_rx(rx/s1)` it kills it | Sequence: ideal, rx c1→s1, ports `["rx0"]`; `start_rx(mock/rx/s1, 2 ms)`; step 10 ms → no SampleClock and no `segments` row for rx s1. Extend `mr_34_a_layout_unlinked_receive_stream_takes_no_untimed_tune` (mock_layout.rs:1079), which already sends that `start_rx`, with those two assertions.
P154 | OWED | lib.rs:1141, `segments` row `channels` order (MR-27; parity with UHD's plan rows). No test has a two-channel stream's row; parity's rows are one channel each (parity.rs:1101). Probe `opus_probe_p154_p172` kills it | Sequence: ideal, rx c0,c1→s0; step → the rx s0 row lists `[0, 1]`. New mock_layout test (with P172).
P165 | DEAD | the out-of-range move pre-check was deleted (grep `report.rejected.first()`: 0) | Already deleted; fix the reason.
P172 | OWED | lib.rs:1134 `ended_at` of a `segments` row, written at a Stop's cut (`rx_settle`, lib.rs:1281-1300). Probe kills it | Sequence: Stop rx/s0 at 5 ms → its row's `ended_at.ticks` == 5 000 000. Same test as P154.
P173 | DEAD | the RM-34 move refusal deleted (grep `RM-34: move of channel`: 0) | Already deleted; fix the reason.
P175 | EQUIVALENT | lib.rs:1010-1011. `phase_fixed` of a non-deterministic channel starts at 0.0 (lib.rs:2082, 2096) and is only ever set to 0.0 (lib.rs:1011, 1022); timed tunes, which set 1, are deleted. Probe `opus_probe_p183_p190` survives it | Delete (done). Note: `phase_fixed` and the `timed_tune` coherence basis (lib.rs:1061-1071) are now constant: dead state and MR-34/D13 text for part (c2)/stage 6.
P176 | EQUIVALENT | lib.rs:1022, same as P175 (TX branch) | Delete (done).
P182 | DEAD | lib.rs:1020, the TX branch of `untimed_tune`: `tune()` returns at `segment.by.is_none()` (lib.rs:993) and every TX segment's `by` is None (TX lines never begin a second segment: Start is receive-only, timeline.rs:447-456) | Region: lib.rs:1015-1027 (TX arm), the TX half of :993, the TX candidates at :1848-1852, `TxStream::tuned`. Stage 4 (TX values per burst) may revive it.
P183 | OWED | lib.rs:1009 `!= "deterministic"`: a CBX channel (`random`, x3xx.rs:163) must redraw at a restart's untimed tune. HEAD killer used a layout entry (move). Probe `opus_probe_p183_p190` kills it | Sequence: x3xx, rx two CBX channels in s0, tx CBX c0 coupled to both, seed 7; burst; Stop rx/s0; `start_rx(+8 ms)`; at origin+100 µs channel 0 hears `rotated(0.5, tx_lo − retune₁)`, channel 1 `rotated(0.5, tx_lo − retune₂)`. New test modelled on mock_layout.rs:1079.
P187 | EQUIVALENT | lib.rs:1845 rank 2→0 of the untimed tune: it only ordered the tune after same-instant timed updates (deleted); with faults or held bursts at that instant the phase is set at the same tick and emission up to the tick precedes every same-tick item (lib.rs:1866-1872). Probe survives | Delete (done).
P190 | OWED | lib.rs:996 channel order of a segment's draws. Probe `opus_probe_p183_p190` kills it | Same test as P183 (two channels in one stream).
P191 | OWED | lib.rs:1944 `tune_order` by stream number. Probe `opus_probe_p191` kills it | Sequence: as P183 but c0→s0, c1→s1, ports rx0, rx1; Stop both; `start_rx` both at one `at` (same origin) → s0's channel takes retune₁, s1's retune₂. New test.
P192 | EQUIVALENT | lib.rs:1944 RX-before-TX: TX untimed tunes never happen (P182), so only receive tunes compete. Probe `opus_probe_p191` survives it | Delete (done).
P193 | DEAD | lib.rs:993 the `Tx && cut == Some(0)` clause; TX tune path unreachable (P182) | Region as P182.
P195 | DEAD | lib.rs:987; the mutant only skips TX tunes, which never draw (P182) | Region as P182.
P242 | EQUIVALENT | control.rs:163. The mutant gives a link-less receive stream a line with an empty channel set; an empty set never plans a segment (timeline.rs:421, 452) and sets are fixed for the Run, so `start_rx`/Stop/end on it carry out nothing either way | Delete (done).
P261 | DEAD | core.rs:694-698, ending a TX clock at its segment's cut: a TX segment gets a cut only from `end_at_origin` (timeline.rs:466-472, 499-503) when a loss/Provider::stop precedes the clock's origin; the only clock is lattice(arm) (mod.rs:568-570), before T0 | Region core.rs:694-698. Stage 4 (a refused configured burst's clock C ends at its origin, RM-25) revives it.
P317 | EQUIVALENT | core.rs:624. The RX record is written only for its stream's own fixed set (mod.rs:560, core.rs:1356 via `current.set()`), so `open == set` ⟺ equal counts | Delete (done).
P359 | EQUIVALENT | core.rs:613 `Planned.refused` is always false: a refused Start's segment is never in `line.plan` (timeline.rs:469, 487-494) and only Start items are refused (rx.rs:408) | Delete (done). Dead code for cleanup: control.rs:80 `refused`, core.rs:613, core.rs:1346-1348.
P377 | EQUIVALENT | mod.rs:566. No surviving path reads a TX record's `set`: owner_step/owner_configure are receive-only (rx.rs:354), the TX reopen reads `configured`, which `record_open` sets None either way (core.rs:639-643), and `record_open(None)` leaves `generation` (core.rs:641) | Delete (done).
P382 | EQUIVALENT | rx.rs:343 (re-anchored). `Holder` is Free/Owner (core.rs:325-331) and only a stream's own owner claims it (core.rs:1353 from rx.rs:354; tx.rs:795), giving it back within the call: at `begin()` it is always Free | Delete (done). Note: the claim/read exclusion on the must-survive list has no second party left.
P391 | OWED | core.rs:1055, the loss check between channels of `Core::configure`, reached by every owner configure (a `start_rx` segment of a 2-channel stream). HEAD killer used a from-none enable. Probe `opus_probe_p391` kills it | Sequence: rx.rs unit, LockSpy, `rx_unopened(&core, 2)`, a planned 2-channel segment with a config, `probe(&core, "apply", …)` publishing `device_lost` at the second apply, `rx.begin()` → no `apply` after `mark_lost` in `spy.order()`. Extend `ur_29_a_loss_after_a_configure_starts_no_receive_segment` (rx.rs:1369).
P398 | EQUIVALENT | tx.rs:299 (re-anchored); same single-claimant argument as P382 | Delete (done).
P420 | OWED | core.rs:625 UR-36: configure again unless the mark is this segment's command. From the second restart on, the mutant skips the configure (first restart has `configured` None). Values are identical in stage 2, but the rule and the Manifest `applied` rows observe it, and stage 3's staged values depend on it. Probe `opus_probe_p420` kills it | Sequence: Stop, `start_rx`, settle; Stop, `start_rx`, settle → each restart makes `apply rx` calls. Combine with the snapshot gap below in one fake.rs test, extending `ur_26_a_stopped_later_stream_waits_for_start_rx` (fake.rs:6416).
P444 | DEAD | core.rs:690-693, `reconcile`'s registration of a TX clock for a plan segment beyond the arm clock: TX plans have ≤ 1 segment | Region core.rs:690-693. Stage 4 revives it (a configured burst registers C at booking).
P513 | OWED | core.rs:1345, the stale-plan guard in `owner_configure`: a Stop booked between uhd-rx's plan read (rx.rs:340-349) and its configure drops a future `start_rx` segment. HEAD killer used a move. Probe `opus_probe_p513` kills it | Sequence: rx.rs unit, `plan(&[(50 ms, None, Some(1))])`, `next = planned[0]`, `book(1 ms, 2, Kind::Stop)`, `owner_configure(Rx, 0, &next)` → `Ok(false)` and no device call. Extend `ur_29_a_begin_rechecks_loss_after_acquiring_the_chain_before_rx_start` (rx.rs:1391), which already calls `owner_configure` directly.
P614 | DEAD | lib.rs:780; its mutant names `FaultKind::ColdChangeRefused`, which part (c2) is withdrawing (`crates/ezsdr-sim/src/lib.rs` diff, `schemas/sim/fault_entry.v1.json`) | Nothing left once (c2) lands; check (c2) also drops the MR-20 text.
P616 | DEAD | `fires()` is already `!self.resolved` (lib.rs:256-258) after (c2); anchor gone | Nothing left.
P646 | EQUIVALENT | lib.rs:1426: one TX clock per stream, so the remembered front's domain always equals the clock's (lib.rs:1432-1435) | Delete (done); stage 4 makes it live again.
P647 | EQUIVALENT | tx.rs:909: `Tx.clock` is set once and only cleared (tx.rs:853), so the chain front's domain is the clock's | Delete (done); stage 4.
P654 | EQUIVALENT | lib.rs:1173: MockRadio never passes `CancelReason::ColdChange` (grep: only UHD uses it) | Delete (done).
P711 | EQUIVALENT | control.rs:149: a line's `lead` is used only by `State::begin` (timeline.rs:510-531), run only for a receive Start; for RX keys the mutant is the identity | Delete (done). Stage 4 decides `tx_start_lead_ns`.
U13 | OWED | control.rs:200-204 (`self.release()` is gone, so re-anchor): MA-14b/KC-21a, each Action booked before the next `recv()`, which is how a submit returns with its refusal (O2's premise). HEAD killer `fake.rs@HEAD:2159` used a Run-time TX enable with a 20 ms apply. Probe `opus_probe_u13` kills it | Sequence: control.rs unit; an `ActionReceiver` whose `recv()` records whether `core.rec.rejected` already holds the previous Action; two `SetTimer` Actions; `Control::run` in a thread, then set `streams.stop` → no early `recv`. New control.rs test (no existing test to extend). Row: old = the loop at control.rs:200-204, new = take all, then book.
U22 | EQUIVALENT | control.rs:354 `clocks.last()`: ≤ 1 TX clock per stream (control.rs:186, core.rs:690-693 unreachable) | Delete (done); stage 4.
V06 | RETARGET | timeline.rs:525. Killed by `ezsdr-radio --test timeline` (`rm_26_the_timeline_cases` tests/timeline.rs:120, `rm_26_generated_sequences_keep_the_timeline_invariants`, …), which c1 never ran for this row | Restore: crate ezsdr-radio, test_bin `timeline`, filter `rm_26_the_timeline_cases`. U23 mutates the same text (`+ 1`); V06 adds the "rounds up an off-lattice earliest" case.
T40 | RETARGET | old re-anchors to timeline.rs:270 `at.unwrap_or(now).max(now).max(floor)` (new `at.unwrap_or(now).max(floor)`), old2 timeline.rs:523 unchanged. With both halves `mr_29_start_rx_is_never_late` (mock_radio.rs:2065) fails (the clock restarts at 1 ms, not 3 ms), and 4 `ezsdr-radio` timeline tests too. c1 probed it without `old2` (`batchB.json`), and the first half alone is masked by `ready + lead` | Restore with both halves, filter unchanged. T38 anchors the same text with another mutant; that is fine.

## Mock guard `handle_update` (lib.rs:1813-1815)

A test is owed, and it is cheap. The brief requires that the Provider still refuse when a harness calls it directly. The Mock harness already bypasses the Kernel (`harness.actions.push`), but today it reaches `update_action` only after a loss (mock_radio.rs:1680-1750, LOST reason). Probe `opus_probe_mr_18_a_radio_update_dispatched_directly_is_refused` pushes `update_action("radio.rx.c0.frequency_hz", 2e9, Cold)` and asserts one `rejected` row ("update_parameter", "MR-18: target or configuration key is not updateable") and one COMMAND_REJECTED naming action 1. It kills the mutant `{ let _ = (id, now); Ok(()) }`. Cheapest form: add that Action to `mr_29_other_actions_are_command_rejected` (mock_radio.rs:2248) and add the row. UHD does the same in `ur_26_an_unnumbered_key_is_refused_at_runtime` (fake.rs:2480): the `Direct` harness pushes `UpdateParameter{radio.rx.antenna, Cold}` into its `Pushed` queue without a Kernel, then asserts the UR-26 reason and no `apply`. P1392 pins it. Its key is not even a layout key; a numbered classless key such as `radio.rx.c0.frequency_hz` would state the stage-2 intent better.

## Gaps against my list

- The 62 rows touch items 1–14: 1 (F36, P1220, V06, T40 retarget; P711 equivalent), 2 (B06, B07 owed), 3 (B13 equivalent), 4 (P1290 owed), 5 (P391, P420, P513 owed; P317, P359 equivalent), 6 (J21, P261, P444 dead; U22 equivalent), 7 (U13 owed), 8 (M68, M69 owed), 9 (P153, P154, P172 owed), 10 (P183, P190, P191 owed; P175, P176, P187, P192 equivalent; P182, P193, P195 dead), 11 (J20, P646, P647 equivalent), 12 (P614, P616 dead), 13 (P1231, P1244, P1259, P382, P398, P377 equivalent), 14 (Mock guard owed).
- **Not shown by any row (and no test): the `start_rx` configuration snapshot**, control.rs:296 `ColdConfig::new(self.config.clone())`. Mutating it to `self.core.description.defaults()` survives UHD lib, fake, acceptance uhd and parity, and server protocol (`batch8`). A restart would then silently retune to the profile defaults. Probe `opus_probe_gap_a_restart_applies_the_connect_time_values` kills it: connect with `radio.rx.c0.frequency_hz` = 2 GHz, Stop, `start_rx` → `device.freq_at(Rx, 0, now)` is still 2 GHz. One test with P420. Must-survive: owner の設定の snapshot.
- Row only (existing tests kill the mutant): uhd-rx registers its clock at the first block (rx.rs:574-579, `stream.handle.take()`) and ends it at the cut (rx.rs:322-326). No row anchors either. Both mutants die in lib, fake and acceptance uhd (`batch8`). Add rows; the killer could be `ur_17_a_finite_segment_delivers_through_its_cut` (rx.rs:1670).
- Item 15 is covered by existing rows (M30/M48 value history; P1172, P1193, P1194 F8/T3; P350, P373, P380 UR-28). `ready` stays parked for stage 3.

## Other notes for the orchestrator

- `crates/ezsdr-radio-uhd/tests/common/measurement/h4.rs` is staged as deleted (`D `) and present untracked (`??`, the rewritten module), and `tests/common/measurement.rs:31` still has `mod h4;`. The commit must `git add` it, or the committed tree does not build.
- PID 2934767 (`ezsdr_radio_uhd-f293f4671de25d25 --test-threads=8`, from the review target dir, running since about 20:07) looks like c1's J294 TIMEOUT probe (cargo was killed, the test binary was not). It is not mine, so I left it.
- The same narrow probe width applies to c1's other DELETEs whose reason is "killer gone": a row whose behaviour is pinned only by an unprobed binary (the Mock differential, `ezsdr-radio` tests, v61, x3xx, server protocol, Python) would look dead. That is outside these 62, so I did not check it.

<!-- opus-s2-fix-recheck.md -->
**Verdict: CLOSED.** Neither fix has a major problem. One P3 clause is left in the spec, plus two notes.

**Finding 1 (TX clock end), closed**
- `Streams::changed` (`crates/ezsdr-radio-uhd/src/provider/core.rs:667–677`) ends the one TX clock at `clock.instant(cut)` when the plan's first segment has a cut and the registry does not already show it ended. `book` and `refuse` both call it. So do `Provider::stop` (`mod.rs:~699`) and a loss, uhd-tx's included, through `Streams::end` → `book`, under the streams lock, before anything is joined or the manifest is written.
- My own failure scenarios all came out right. Only `Kind::End` can cut a TX segment, through `timeline.rs:465`'s `end_at_origin` when S ≤ O. A TX `Stop` books `Kind::Stop`, which does nothing on transmit, so the existing loss/`Provider::stop` callers are the only ones. A stop after O leaves the clock open, as at HEAD, and as MockRadio's `tx_reconcile` does (`mock-radio/src/lib.rs:1414`, which also ends only at a cut).
- The clock is ended once. `Streams::end` runs once, the registry check stops a second end, `tx.rs` `end_clock` no longer touches the registry, and nothing at cleanup ends TX clocks.
- RX streams have an empty `clocks` (only `control.rs:186` fills it, for TX), so dropping HEAD's `Direction::Tx` guard is harmless. Without `reconcile`'s registration and tail branches, a TX plan with a clock always has exactly one segment.
- A failed end is still visible, as `command_rejected("update_parameter", "UR-25: …")`, as at HEAD. It is practically unreachable: same root, `Stopped` filtered out.
- Lock order: streams lock, then the registry's RwLock, `rec` and emit. That is the same as HEAD's `reconcile`, and the registry never takes the streams lock.
- I re-probed the rows in a copy. P1398 fails at core.rs:2009 and P1399 at :2012, both against `ur_23_a_…`. The implementer's logs in `work/fix-probes/` predate the test's rename and name it `ur_25_a_…`, but the filter is correct. The unmutated test passes. `ur_25_uhd_tx_reads_the_plan_again…` survives P1398, so the new test is the only one that kills it.

**Finding 2 (spec text), closed apart from one clause**
- P3: `design/09-mock-radio.md:205`, MR-34, still says an untimed tune leaves the channel "not coherent until its next timed tune". No timed tune exists any more; that clause should go. The same idea is in the doc comment at `mock-radio/src/lib.rs:978` ("loses the coherence a timed tune gave it").
- Note: the label `"update_parameter"` on an internal clock-end failure (`core.rs:674`) and on the RM-25 book error is a leftover name from before stage 2. HEAD used it too.
- Rows I checked against their tests that are still accurate: 09:312–313 (`mr_20_*` still hand over a `HardwareTimed` update directly), 18:507 (a direct `Cold` update), 18:388 (`hw_h4_load`, which sends no timed update), and MR-15, MR-27, M12, UR-23 and the 18:524–525 rows. 11:113 (C3) mentions a "runtime `cold` change", but only as the reason a rejected alternative was rejected, which I read as history rather than a live description. My sweep found nothing else outside the Changes tables in 07, 09, 18, 16 or 11.

**Leftover: the `timed_tune` branch is dead**
`phase_fixed ≥ 0.5` cannot arise for a `random_unless_timed_tune` channel:
- At `prepare`, which reads the values asked at connect, `phase_fixed` starts at 1.0 only for deterministic channels (`lib.rs:2037`, `:2051`).
- Start at T0 takes no tune.
- The only writer, a `start_rx` restart in `untimed_tune` (`lib.rs:986`), sets it to 0.
- I confirmed it by probe: with `unreachable!` in the `timed_tune` return and an assert that such a channel is never fixed, the mock-radio, sim and acceptance tests all pass.

What to delete:
- The `phase_fixed` fields (`channel.rs:61`, `:117`), their initialisers (`lib.rs:2037`, `:2051`) and the `.set` at `:986`.
- In `layout_phase_basis` (`lib.rs:995–1037`): the `is_fixed` element, the `timed` variable, the `random_unless_timed_tune` arm (it folds into `_ => fixed = false`) and the `timed_tune` return. The basis becomes "deterministic if every channel is deterministic, else none", which is what MR-27 says.
- `update_layout_segment_coherence` (`lib.rs:1040–1085`) and its call at `:990`. Once the basis is constant per segment, it can only rewrite or append entries equal to the one already in force.
- Row P170, whose target goes with that function. P178 and P183 stay, because the phase redraw is still live.

I did not run the full workspace or Python suites, or the hardware or acceptance UHD binaries. Every build ran in a scratchpad copy; the copy's files are restored, no test processes are left, and the tree is untouched.
