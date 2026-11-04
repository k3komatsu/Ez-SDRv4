# Test plan: note 47 stage 2 — part (a) every radio key classless (revision 4, kept in `test-plan-s2.v4.md`), part (b) the Run-time change machinery deleted (revision 5, kept in `test-plan-s2.v5.md`), part (d) the pins the deletion lost and the last dead code (revision 6, at the end)

Replaces `test-plan-s2.v3.md` (rev 3; older copies `.v1`, `.v2`) and `test-plan-s2a.md` (the receive-only plan, background). Written by the implementer of part (a) after the class flip of all radio keys (`crates/ezsdr-radio/src/lib.rs`: every `radio.{rx,tx}.c<n>.{frequency_hz,gain_db,antenna,stream}` and `radio.{rx,tx}.s<k>.sample_rate_hz` has no update class). HEAD `45a4294`.

**Inventory:** `cargo test --workspace --no-fail-fast` after the flip failed 304 tests (the previous receive-only flip: 271). Decided by subject, one row each below. Python: 9 of 78 failed. `--features uhd` (via `rdev.py`): compile and clippy only, the bench tests are `#[ignore]` and need a USRP.

**Counts (rows, rev 3):** ? 7, DELETE 240, REFUSAL 4, REWRITE-INITIAL 43. 294 rows; rev 4 rewrites 20 of them (RESTORED, REWRITE-INITIAL, PARKED) and adds the rows after the table; several rows cover a rewrite of a test that was not failing (e.g. the companion-group skip test) or a helper-level change that fixes many tests at once (`tx_session`, `rx0_tx0`).

**Result:** `cargo test --workspace --no-fail-fast` green, `cargo clippy --workspace --all-targets -- -D warnings` green (also `-p ezsdr-radio-uhd --features uhd --all-targets` on sim02), Python 78/78 on 3.14 and on the 3.9 container. Rev 4: workspace 1 988 passed, `--features uhd` lib 561 + uhd_api 5, hardware tests build, clippy and fmt clean, `rules.py --base HEAD` 238 problems (219 cited tests, 19 mutation rows).

## Decisions that shaped many rows

- `fake.rs`: `tx_session`, `tx_session_on` and `common::rehearse_session_loopback` connect with initial `rx0_tx0` (new in `common/mod.rs`) instead of enabling transmit at run time; this alone turned about 20 tests from failing to passing.
- Measurement: `candidate_session_asking`, `tx_only` and `await_tx` replace `enable_tx`/`disable_rx`. The four start-lead groups (`rx_start_lead`, `tx_start_lead` and their `_with_tx`/`_with_rx` companions) and the H4 module (`common/measurement/h4.rs`) are deleted: they measure Run-time rate transitions, which stages 3 and 4 redesign. `radio.timing.rx_start_lead_ns` and `tx_start_lead_ns` therefore have no group (no measured value) until then.
- The shared VH generator (`crates/ezsdr-radio/tests/generator/mod.rs`) is unchanged. The Mock differential skips its Cold/Timed ops and the layout membership tail; the generator, the radio crate's timeline-oracle tests and the Cold/Timed arms go with the oracle in part (b). The uhd-control differential module was deleted whole (see its row).
- Dead test helpers the deletions left were removed (compiler-driven), so clippy `-D warnings` stays clean. No `#[ignore]` was added.

## Rows: path, test, subject, decision, new stimulus or reason

| path | test | subject | decision | new stimulus / reason |
|---|---|---|---|---|
| crates/ezsdr-acceptance/tests/v58.rs | `v58_11_short_lead_burst_is_a_time_error` | late burst is a time error (RM-15) | REWRITE-INITIAL | loopback_run (tx c0 stream 0 at connect); origin T0+3ms, late 1 ms |
| crates/ezsdr-acceptance/tests/v58.rs | `rs_19_an_untimed_repeat_after_an_enable_starts_at_the_new_clock_origin -> rs_19_an_untimed_repeat_in_a_session_starts_at_the_clock_origin` | RS-19 untimed repeat starts on time at clock origin | REWRITE-INITIAL | loopback_run; no enable |
| crates/ezsdr-acceptance/tests/v58.rs | `kc_24_a_burst_after_the_transmit_clock_ends_is_refused_at_admission` | burst after TX clock ended refused (SC-23) | DELETE | needs a runtime stream=-1 to end the clock; Kernel-side pin is coordinator.rs kc_24_a_burst_needs_a_transmit_sample_clock and fake.rs:4986 |
| crates/ezsdr-acceptance/tests/v58.rs | `v58_16_runtime_retune_outside_the_rf_envelope_is_rejected -> v58_16_a_value_outside_the_rf_envelope_is_refused_at_connect_and_a_runtime_change_has_no_class` | §58 #16 RF envelope | REFUSAL + REWRITE-INITIAL | initial frequency outside band -> failed Run at connect; runtime frequency change (inside and outside band) rejected, effective unchanged |
| crates/ezsdr-acceptance/tests/v58.rs | `v58_16_runtime_rate_beyond_the_envelope_is_rejected -> v58_16_a_rate_beyond_the_envelope_is_refused_at_connect_and_a_runtime_change_has_no_class` | §58 #16 rate envelope (RM-32) | REFUSAL + REWRITE-INITIAL | initial rate 200 MS/s -> failed Run at validate (RM-32); runtime rate change rejected |
| crates/ezsdr-acceptance/tests/v58.rs | `mr_34_a_scheduled_receive_retune_inside_a_running_segment_is_a_timed_tune -> mr_34_a_scheduled_radio_value_update_is_refused_at_validate` | scheduled timed tune | REFUSAL | Spec schedule update of radio.rx.c0.frequency_hz -> failed Run at Validate (RS-52) |
| crates/ezsdr-acceptance/tests/v58.rs | `mr_34_a_scheduled_receive_update_inside_a_refused_change_span_is_untimed` | timed update in refused change span | DELETE | Run-time change machinery |
| crates/ezsdr-acceptance/tests/uhd.rs | `uhd_59_a_session_retune_outside_the_rf_envelope_is_rejected -> uhd_59_a_value_outside_the_rf_envelope_is_refused_at_connect_and_a_runtime_change_has_no_class` | UHD RF envelope (§58 #16 on UHD) | REFUSAL + REWRITE-INITIAL | initial tx frequency outside band -> failed Run; runtime change rejected and logged |
| crates/ezsdr-acceptance/tests/parity.rs | `vh_8_the_same_transmit_stream_clock` | TX stream 0 clock at arm on lattice (RM-25); cut half gone | REWRITE-INITIAL | first half kept (initial tx c0 stream 0); the channel-leaves cut removed |
| crates/ezsdr-acceptance/tests/parity.rs | `vh_8_the_same_manifest_plan_rows` | plan/segments rows of both Providers | REWRITE-INITIAL (m1) | rx_two and tx_two asked at connect, Stop rx/s1 and start_rx rx/s1; receive rows compared with origins from T0; the two initial transmit rows (one per stream) are compared by (direction, stream, channels) and each origin must lie on the lattice at or before T0, because each Provider arms at its own instant (probe: Mock -400 200 000, UHD -400 251 600 ticks from T0; `sequence` is None on both, so the earlier reason did not hold) |
| crates/ezsdr-acceptance/tests/parity.rs | `vh_8_the_same_manifest_sequence` | sequence = arrival order of the command that began the segment | REWRITE-INITIAL (m1) | same planned() with the transmit streams: ranks [None, None, Some(0), None, None] |
| crates/ezsdr-acceptance/tests/parity.rs | `vh_8_cold_change_cancels_held_bursts` | burst held then cancelled by cold change | DELETE | Run-time change (cancel-by-cold-change); Stop/loss/Provider::stop cancellation siblings stay |
| crates/ezsdr-acceptance/tests/parity.rs | `vh_8_cold_change_cancels_a_burst_on_a_skipped_future_clock` | burst on never-started future clock after rate changes | DELETE | Run-time rate changes |
| crates/ezsdr-acceptance/tests/parity.rs | `vh_8_tx_stop_cancels_a_burst_on_a_future_clock` | Stop cancels burst on future clock after rate change | DELETE | future clock made by a Run-time rate change; Stop cancellation stays in vh_8_tx_stream_stop_cancels_held_bursts |
| crates/ezsdr-acceptance/tests/parity.rs | `vh_8_tx_repeat_records_match_at_a_common_explicit_cut` | repeat burst ended by an explicit-instant rate change | DELETE | the cut is a Run-time rate change; repeat replacement/cut is stage 5 |
| crates/ezsdr-acceptance/tests/parity.rs | `vh_8_a_stream_one_rate_change_cuts_that_stream_alone` | rx rate change | DELETE | Run-time change |
| crates/ezsdr-acceptance/tests/parity.rs | `vh_8_a_stream_one_join_leaves_the_same_ready_gap` | join | DELETE | Run-time membership change |
| crates/ezsdr-acceptance/tests/parity.rs | `vh_8_a_channel_less_stream_one_starts_the_same_first_segment` | from none | DELETE | Run-time membership change |
| crates/ezsdr-acceptance/tests/parity.rs | `vh_8_a_transmit_stream_one_has_the_same_clock` | tx enable | DELETE | Run-time membership change |
| crates/ezsdr-acceptance/tests/parity.rs | `vh_8_the_instance_floor_crosses_streams` | instance floor | DELETE | Run-time change floor |
| crates/ezsdr-acceptance/tests/parity.rs | `vh_8_a_stream_one_refusal_leaves_stream_zero_alone` | RM-31 antenna refusal of a Run-time change | DELETE | Run-time change; RM-31 at initial values stays in prepare tests |
| crates/ezsdr-acceptance/tests/parity.rs | `vh_8_a_move_between_streams_leaves_the_same_clocks_segments_and_update` | move | DELETE | Run-time membership change |
| crates/ezsdr-acceptance/tests/parity.rs | `vh_8_a_move_into_an_empty_stream_leaves_the_same_first_segment_and_update` | move into empty stream | DELETE | Run-time membership change |
| crates/ezsdr-acceptance/tests/parity.rs | `vh_8_the_same_floor_at_t0` | floor at T0 of a cold change | DELETE | Run-time change floor |
| crates/ezsdr-acceptance/tests/parity.rs | `vh_8_the_same_floor_after_a_transmit_change` | floor across directions | DELETE | Run-time change floor |
| crates/ezsdr-acceptance/tests/parity.rs | `vh_8_a_floored_refusal_is_reported_in_the_same_order` | floored refusal order | DELETE | Run-time change floor |
| crates/ezsdr-acceptance/tests/parity.rs | `vh_8_the_same_logical_updates` | logical applied rows of timed update and membership change | DELETE | Run-time change rows |
| crates/ezsdr-acceptance/tests/parity.rs | `vh_8_the_same_receive_membership_join_leaves_the_same_clocks` | join clocks | DELETE | Run-time membership change |
| crates/ezsdr-radio/tests/radio_model.rs | `rm_04_the_key_table_is_exactly_the_declared_one` | key table with classes | REWRITE-INITIAL | expected table: every update class none (RM-27) |
| crates/ezsdr-radio/tests/channels.rs | `rm_27_the_layout_builder_keys_are_declared` | layout key declarations | REWRITE-INITIAL | expected classes none |
| crates/ezsdr-server/tests/protocol.rs | `ea_10_a_named_selector_connects_as_the_unnamed_one_and_the_record_holds_numbers` | names stay out of key/log | REFUSAL | runtime gain set now rejected (class-less) and still logged by its numbered key |
| crates/ezsdr-server/tests/protocol.rs | `ea_11_submit_returns_the_logged_entry` | submit returns the logged entry | REWRITE-INITIAL | connect with loopback initial; admitted = repeat; rejected = rx rate over envelope (class-less refusal) |
| crates/ezsdr-server/tests/protocol.rs | `ea_07_a_session_on_the_fake_device` | Session on the fake device captures its transmit | REWRITE-INITIAL | fake_session_with(loopback()) instead of runtime tx enable |
| crates/ezsdr-mock-radio/tests/mock_channel.rs | `mr_32_b_the_transmitted_timelines_are_read_at_the_samples_own_instant` | timed tune read at sample instant (45-sample advance) | DELETE | timed tune |
| crates/ezsdr-mock-radio/tests/mock_channel.rs | `mr_32_c_the_transmitted_frequency_is_read_at_the_samples_own_instant` | timed freq retune window vs gate | DELETE | timed retune |
| crates/ezsdr-mock-radio/tests/mock_channel.rs | `mr_32_x3xx_cold_change_cuts_at_its_instant` | cold rate change cut | DELETE | Run-time rate change |
| crates/ezsdr-mock-radio/tests/mock_channel.rs | `mr_35_a_cold_receive_change_samples_the_field_on_the_new_clock` | cold rx rate change new clock | DELETE | Run-time rate change |
| crates/ezsdr-mock-radio/tests/mock_channel.rs | `mr_34_a_random_phase_is_redrawn_by_every_tune` | retune redraws phase | DELETE | Run-time retune |
| crates/ezsdr-mock-radio/tests/mock_channel.rs | `mr_32_a_stop_after_a_cold_change_keeps_the_old_clock_s_radiation` | stop after cold change | DELETE | Run-time rate change |
| crates/ezsdr-mock-radio/tests/mock_channel.rs | `mr_32_a_cold_transmit_change_ends_the_old_radiation` | cold tx change ends radiation | DELETE | Run-time rate change |
| crates/ezsdr-mock-radio/tests/mock_channel.rs | `mr_34_a_timed_tune_replaces_the_random_phase_with_its_constant` | timed tune constant phase | DELETE | timed tune |
| crates/ezsdr-mock-radio/tests/mock_channel.rs | `mr_34_a_a_timed_tune_reaches_every_transmit_channel` | timed tune reaches every tx channel | DELETE | timed tune |
| crates/ezsdr-mock-radio/tests/mock_channel.rs | `mr_30_untimed_receive_tune_precedes_transmit_at_one_origin` | untimed rx rate change precedes tx at one origin | DELETE | Run-time rate change |
| crates/ezsdr-mock-radio/tests/mock_channel.rs | `mr_36_clipping_at_full_scale_is_counted` | clipping counted (tx and rx) | REWRITE-INITIAL | initial rx gain 20 dB (Options.constraints) instead of a timed gain update; rx_clipped 1000 |
| crates/ezsdr-mock-radio/tests/mock_channel.rs | `mr_32_the_transmit_path_delay_shifts_a_loopback` | x3xx 45-sample tx advance shifts loopback | REWRITE-INITIAL | initial tx gain 6 dB; every heard sample scaled |
| crates/ezsdr-mock-radio/tests/mock_channel.rs | `mr_33_gain_scales_the_samples_from_its_instant -> mr_33_gain_scales_the_samples` | gain scales samples (CH/MR-33) | REWRITE-INITIAL | initial tx -6 dB, rx +12 dB |
| crates/ezsdr-mock-radio/tests/mock_channel.rs | `mr_33_a_receiver_hears_only_its_own_frequency` | CH-4 gate on exact frequency equality | REWRITE-INITIAL | two runs with initial tx/rx frequencies equal and unequal |
| crates/ezsdr-mock-radio/tests/mock_channel.rs | `mr_25_a_stop_at_a_held_burst_s_start_silences_the_transmitter` | Stop before a held burst start silences tx | REWRITE-INITIAL | ColdChange case dropped; StopBefore and StopInTheRound stay |
| crates/ezsdr-mock-radio/tests/timeline.rs | `rm_26_the_timeline_against_mockradio` | VH differential: MockRadio against the timeline oracle on generated sequences | REWRITE-INITIAL | the Mock side drops the generator's Cold/Timed ops and the layout membership tail (transition counts); Stop/start_rx/faults/loss sequences stay. The shared generator (ezsdr-radio/tests/generator/mod.rs) is unchanged: its Cold/Timed ops go with the oracle in part (b) |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_09_same_tick_work_keeps_insertion_order` | same-tick order of a fault and two cold updates | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_17_a_burst_before_its_clock_s_origin_is_late` | burst before a clock origin made by an enable/rate change is late (RM-15); the initial-clock form is v58_11 (acceptance) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_17_cold_change_does_not_cross_sample_clocks` | cold change and sample clocks | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_18_a_burst_on_a_later_clock_survives_an_earlier_switch` | MR-18 Run-time update machinery (timed/cold update, window, queue slots, floor, rows at stop/loss) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_18_a_channel_leaving_after_the_booking_does_not_move_the_floor` | MR-18 Run-time update machinery (timed/cold update, window, queue slots, floor, rows at stop/loss) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_18_a_cold_change_does_not_floor_a_timed_update` | MR-18 Run-time update machinery (timed/cold update, window, queue slots, floor, rows at stop/loss) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_18_a_cold_change_starts_its_clock_on_the_lattice` | MR-18 Run-time update machinery (timed/cold update, window, queue slots, floor, rows at stop/loss) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_18_a_cold_change_that_arrives_after_a_later_one_follows_it` | MR-18 Run-time update machinery (timed/cold update, window, queue slots, floor, rows at stop/loss) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_18_a_cold_rate_change_starts_a_new_sample_clock` | MR-18 Run-time update machinery (timed/cold update, window, queue slots, floor, rows at stop/loss) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_18_a_cold_receive_change_before_t0_applies_at_t0` | MR-18 Run-time update machinery (timed/cold update, window, queue slots, floor, rows at stop/loss) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_18_a_cold_transmit_change_replaces_the_tracker` | MR-18 Run-time update machinery (timed/cold update, window, queue slots, floor, rows at stop/loss) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_18_a_command_due_at_its_receipt_takes_no_slot` | MR-18 Run-time update machinery (timed/cold update, window, queue slots, floor, rows at stop/loss) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_18_a_command_due_before_the_receipt_takes_no_slot` | MR-18 Run-time update machinery (timed/cold update, window, queue slots, floor, rows at stop/loss) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_18_a_disable_that_arrives_after_a_later_change_follows_it` | MR-18 Run-time update machinery (timed/cold update, window, queue slots, floor, rows at stop/loss) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_18_a_fractional_receive_cut_is_the_first_sample_at_or_after_e` | MR-18 Run-time update machinery (timed/cold update, window, queue slots, floor, rows at stop/loss) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_18_a_fractional_transmit_cut_is_the_first_sample_at_or_after_e` | MR-18 Run-time update machinery (timed/cold update, window, queue slots, floor, rows at stop/loss) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_18_a_late_command_at_a_full_queue_reports_late_first` | MR-18 Run-time update machinery (timed/cold update, window, queue slots, floor, rows at stop/loss) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_18_a_loss_after_the_last_pass_keeps_a_row_due_before_it` | MR-18 Run-time update machinery (timed/cold update, window, queue slots, floor, rows at stop/loss) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_18_an_omitted_deadline_takes_the_floor` | MR-18 Run-time update machinery (timed/cold update, window, queue slots, floor, rows at stop/loss) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_18_an_update_applied_before_a_stop_keeps_its_row` | MR-18 Run-time update machinery (timed/cold update, window, queue slots, floor, rows at stop/loss) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_18_an_update_due_at_the_stop_instant_applies_and_at_the_loss_instant_does_not` | MR-18 Run-time update machinery (timed/cold update, window, queue slots, floor, rows at stop/loss) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_18_a_receive_enable_from_zero_waits_the_start_lead` | MR-18 Run-time update machinery (timed/cold update, window, queue slots, floor, rows at stop/loss) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_18_a_reentry_after_a_later_change_follows_it` | MR-18 Run-time update machinery (timed/cold update, window, queue slots, floor, rows at stop/loss) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_18_a_scheduled_pair_is_checked_when_it_applies` | MR-18 Run-time update machinery (timed/cold update, window, queue slots, floor, rows at stop/loss) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_18_a_stop_leaves_each_update_its_row_at_its_instant` | MR-18 Run-time update machinery (timed/cold update, window, queue slots, floor, rows at stop/loss) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_18_a_timed_update_behind_one_inside_the_window_is_late` | MR-18 Run-time update machinery (timed/cold update, window, queue slots, floor, rows at stop/loss) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_18_a_timed_update_behind_one_waiting_for_a_channel_is_late` | MR-18 Run-time update machinery (timed/cold update, window, queue slots, floor, rows at stop/loss) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_18_a_timed_update_beyond_the_window_keeps_its_instant` | MR-18 Run-time update machinery (timed/cold update, window, queue slots, floor, rows at stop/loss) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_18_hardware_timed_updates` | MR-18 Run-time update machinery (timed/cold update, window, queue slots, floor, rows at stop/loss) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_18_the_ideal_profile_has_no_release_window` | MR-18 Run-time update machinery (timed/cold update, window, queue slots, floor, rows at stop/loss) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_18_the_row_is_written_at_its_instant` | MR-18 Run-time update machinery (timed/cold update, window, queue slots, floor, rows at stop/loss) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_18_the_window_edge_excludes_one_tick` | MR-18 Run-time update machinery (timed/cold update, window, queue slots, floor, rows at stop/loss) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_18_two_updates_at_one_instant_keep_the_booking_order` | MR-18 Run-time update machinery (timed/cold update, window, queue slots, floor, rows at stop/loss) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_20_a_change_before_the_fault_is_no_candidate` | MR-20 cold_change_refused fault / loss with pending Run-time changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_20_a_change_the_plan_drops_is_no_candidate` | MR-20 cold_change_refused fault / loss with pending Run-time changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_20_a_cold_change_refused_fault_acts_on_both_profiles` | MR-20 cold_change_refused fault / loss with pending Run-time changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_20_a_cold_change_refused_fault_halts_the_stream` | MR-20 cold_change_refused fault / loss with pending Run-time changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_20_a_fault_after_every_change_refuses_nothing` | MR-20 cold_change_refused fault / loss with pending Run-time changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_20_a_fault_between_cold_clocks_is_not_applied -> mr_20_a_fault_between_receive_clocks_is_not_applied` | a fault in the gap between two receive clocks is not applied (MR-20) | REWRITE-INITIAL (M2c) | Stop at 1 ms, fault at 1.005 ms (rx_sequence_error and rx_overflow), start_rx at 10 ms; the fault is unapplied with lost 0, no RX_OVERFLOW, the new clock's first block is tick 0, lost None, flags NONE. The second half (a fault ordered before a refused cold change) is not restored: its stimulus is a refused Run-time change |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_20_a_loss_before_the_refused_change_refuses_nothing` | MR-20 cold_change_refused fault / loss with pending Run-time changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_20_a_loss_cancels_what_it_left_pending` | MR-20 cold_change_refused fault / loss with pending Run-time changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_20_a_loss_records_a_later_update_as_cancelled_at_the_loss` | MR-20 cold_change_refused fault / loss with pending Run-time changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_20_only_the_first_cold_change_is_refused` | MR-20 cold_change_refused fault / loss with pending Run-time changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_20_the_next_cold_change_resumes_a_refused_stream` | MR-20 cold_change_refused fault / loss with pending Run-time changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_25_a_loss_then_a_stop_keeps_the_loss_s_instant_and_its_rows` | Stop/loss keeps pending Run-time update rows | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_25_a_stop_records_a_later_update_as_cancelled_at_the_stop` | Stop/loss keeps pending Run-time update rows | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_25_a_stop_then_a_loss_keeps_the_stop_s_instant_and_its_rows` | Stop/loss keeps pending Run-time update rows | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_25_a_stream_stop_keeps_the_pending_commands` | Stop/loss keeps pending Run-time update rows | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_21_overrun_shape` | overrun row = injected k_g-k_f, never revised | REWRITE-INITIAL (m5) | tail restored with a Stop of mock/rx/s0 at 20 ms instead of the booked rate change: the row stays 50 000, no block carries the loss |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_12_d14_the_actions_of_a_layout_instance_name_a_stream` | D14 targets: burst names its transmit stream, wrong destinations refused (MR-16, MR-25, MR-29), per-stream Stop/start_rx | REWRITE-INITIAL (M2d) | the Run-time updates and the three MR-18 refusals of updates are dropped; the burst target refusals (device, rx stream), Stop to mock/rx/s9 and start_rx on the device remain (4 reasons), then the per-stream Stop and start_rx of rx/s1 unchanged |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_18_a_layout_a_move_after_a_refused_source_change_waits_the_refused_segments_end` | MR-18 layout Run-time change floor/queue/timed instant | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_18_a_layout_cold_change_is_floored_at_every_earlier_cold_change` | MR-18 layout Run-time change floor/queue/timed instant | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_18_a_layout_cold_floor_crosses_directions` | MR-18 layout Run-time change floor/queue/timed instant | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_18_a_layout_cold_floor_holds_without_a_line` | MR-18 layout Run-time change floor/queue/timed instant | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_18_a_layout_hardware_timed_change_keeps_its_own_instant` | MR-18 layout Run-time change floor/queue/timed instant | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_18_a_layout_instance_s_changes_belong_to_their_own_stream` | MR-18 layout Run-time change floor/queue/timed instant | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_18_a_layout_receive_floor_at_t0_emits_late_command` | MR-18 layout Run-time change floor/queue/timed instant | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_18_a_layout_rejects_an_out_of_range_stream_before_booking_a_move` | MR-18 layout Run-time change floor/queue/timed instant | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_18_a_layout_reservations_cannot_invalidate_an_earlier_one` | MR-18 layout Run-time change floor/queue/timed instant | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_20_a_burst_on_a_clock_booked_after_a_refusal_plays` | MR-20 refused cold change (halt, cancel, resume, retry) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_20_a_cold_change_to_a_linkless_stream_is_no_candidate` | MR-20 refused cold change (halt, cancel, resume, retry) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_20_a_hardware_timed_update_in_a_refused_halt_is_recorded` | MR-20 refused cold change (halt, cancel, resume, retry) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_20_a_refused_move_halts_both_lines` | MR-20 refused cold change (halt, cancel, resume, retry) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_20_a_refused_transmit_burst_leaves_the_channel_silent` | MR-20 refused cold change (halt, cancel, resume, retry) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_20_a_refused_transmit_change_ends_its_clock_and_cancels_its_bursts` | MR-20 refused cold change (halt, cancel, resume, retry) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_20_a_start_rx_resumes_a_refused_stream` | MR-20 refused cold change (halt, cancel, resume, retry) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_20_a_transmit_change_after_a_refusal_begins_as_if_not_refused` | MR-20 refused cold change (halt, cancel, resume, retry) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_20_a_transmit_change_after_repeated_refusals_begins_as_if_none_were_refused` | MR-20 refused cold change (halt, cancel, resume, retry) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_20_a_transmit_change_with_no_sample_is_a_candidate` | MR-20 refused cold change (halt, cancel, resume, retry) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_20_a_transmit_move_after_a_refused_source_change_begins_as_if_not_refused` | MR-20 refused cold change (halt, cancel, resume, retry) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_20_only_the_refused_clock_s_bursts_are_cancelled` | MR-20 refused cold change (halt, cancel, resume, retry) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_20_the_fault_follows_the_effective_instant_not_the_origin` | MR-20 refused cold change (halt, cancel, resume, retry) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_20_two_changes_at_one_instant_refuse_the_first_booked` | MR-20 refused cold change (halt, cancel, resume, retry) | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_30_a_layout_untimed_update_draws_are_reproducible_and_in_application_order` | untimed update draws in application order | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_32_a_logical_transmit_channel_follows_its_move_to_another_stream` | logical tx channel follows its move | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_cbx_layout_entry_draws_a_fresh_phase` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_cbx_layout_redraws_only_the_retuned_channel` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_layout_a_flip_flop_is_timed_by_the_segment_that_covers_e` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_layout_a_linkless_stream_does_not_cover_even_after_start_rx` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_layout_an_untimed_update_and_the_entry_tune_at_one_instant_draw_in_application_order` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_layout_an_update_at_a_cold_instant_is_timed_whatever_the_refusal_order` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_layout_an_update_at_a_cut_is_untimed` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_layout_an_update_in_the_span_a_refusal_once_advanced_is_untimed` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_layout_an_update_received_after_the_origin_a_refusal_once_gave_is_untimed` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_layout_an_update_received_before_t0_is_timed_when_its_segment_has_run_the_release_window` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_layout_an_update_received_while_samples_pass_draws_nothing` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_layout_an_update_received_while_stopped_is_timed_when_a_later_segment_covers_e` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_layout_a_refusal_does_not_move_the_next_segment_s_origin` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_layout_a_refused_segment_does_not_cover_an_update_received_after_its_planned_origin` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_layout_a_stop_after_the_update_makes_it_untimed_at_application` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_layout_a_transmit_origin_s_untimed_tune_follows_a_timed_update_at_its_instant` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_layout_a_transmit_update_before_t0_stays_a_timed_tune` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_layout_deterministic_and_cbx_receive_channels_under_the_new_rule` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_layout_deterministic_receive_update_before_t0_stays_zero` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_layout_move_out_is_an_untimed_tune_of_the_channels_left_behind` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_layout_rate_change_is_an_untimed_tune` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_layout_receive_entry_is_an_untimed_tune` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_layout_receive_move_is_an_untimed_tune_of_the_moved_channel` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_layout_receive_update_before_t0_is_an_untimed_tune` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_layout_reentry_cut_before_its_start_leaves_the_rate_change_untimed` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_layout_stream_enable_is_an_untimed_tune_of_its_channels` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_layout_the_other_streams_segment_covers` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_layout_transmit_clock_cut_at_its_origin_takes_no_untimed_tune` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_layout_transmit_entry_is_an_untimed_tune` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_layout_transmit_move_is_an_untimed_tune_of_the_moved_channel` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_layout_transmit_rate_change_is_an_untimed_tune` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_layout_update_before_a_planned_segment_is_untimed_and_the_segment_adds_its_draw` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_layout_update_to_a_stopped_receive_stream_is_untimed_and_the_restart_adds_its_draw` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_receive_update_is_timed_from_the_release_window_after_the_segment_origin` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_layout_mixed_boards_have_per_channel_timed_phase_behavior` | MR-34 phase/coherence basis of Run-time (timed/untimed) tunes, entries, moves, rate changes | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_40_a_move_to_and_from_a_linkless_receive_stream_never_publishes_there` | MR-40 channel move | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_40_a_move_uses_the_instance_floor_across_streams_directions_and_missing_links` | MR-40 channel move | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_40_a_receive_move_waits_for_release_and_lead_and_records_channel_order` | MR-40 channel move | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_40_a_refused_move_applies_no_update_and_changes_no_manifest_or_clock` | MR-40 channel move | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_40_a_transmit_move_cuts_both_clocks_and_cancels_both_held_bursts` | MR-40 channel move | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `rm_31_rm_32_a_layout_instance_s_violations_are_judged_at_runtime` | envelope judged on a layout instance at Run time | DELETE | Run-time change machinery |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_09_each_transmit_stream_has_its_own_clock_and_line` | per-stream tx clock/line, bursts held per stream, Stop of one stream | REWRITE-INITIAL | cold rate change step removed; s1 burst ticks moved to its arm clock |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_32_a_layout_transmit_path_delay_silences_samples_before_the_antenna` | 45-sample tx path delay silences samples before the antenna | REWRITE-INITIAL | initial tx rate constraint; burst at T0+20ms on the arm clock |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_33_layout_gain_and_frequency_timelines_are_per_logical_channel` | gain/frequency per logical channel | REWRITE-INITIAL | eight values as initial constraints instead of hardware_timed updates |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_12_every_advertised_rate_is_accepted` | every advertised rate accepted (UR-12) | REWRITE-INITIAL | the Session asks 200 MHz/7 at connect (rx0 initial + rx.s0.sample_rate_hz) instead of a runtime rate set |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_a_receive_session_follows_its_recorded_plan` | receive path follows the recorded plan | REWRITE-INITIAL | two Stop + start_rx cycles and the Run stop (three segments) instead of two cold rate changes |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_a_receive_cut_counts_its_own_stream_s_packet_size` | ready gap after a cut counts the cut stream's packet size | REWRITE-INITIAL | Stop rx/s1 then start_rx rx/s1 (Direct::stop_at/start_rx_at) instead of a rate change; gap range unchanged |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_26_a_receive_start_queued_by_the_stop_is_issued_and_ends_at_its_origin` | owner does not re-read Provider::stop after its configure | REWRITE-INITIAL | Stop s1 + start_rx s1 with the owner's configure held, then Provider::stop (the Opus re-check's KEEP-INITIAL) |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_26_a_start_rx_of_a_channel_less_linked_stream_starts_nothing` | start_rx of a channel-less stream carries out nothing | REWRITE-INITIAL | first half kept; the later join removed |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_26_two_receive_streams_end_at_their_own_stops` | Stop of one stream ends it at its own stop; abort cuts the other | REWRITE-INITIAL | both receive channels in streams 0 and 1 from the start; the runtime enable removed |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_30_the_plan_names_every_stream_at_the_stop` | plan rows name every stream, sequence and channels at the stop | REWRITE-INITIAL | rx s0 second segment by Stop + start_rx; tx streams keep one segment each |
| crates/ezsdr-radio-uhd/tests/fake.rs | `a_companion_that_fails_after_a_solo_measurement_keeps_the_solo_value` | GroupRun: a failed companion keeps the solo value | REWRITE-INITIAL | tx_in_flight solo + overflow_restart_gap_with_tx companion (the rx start-lead groups are gone) |
| crates/ezsdr-radio-uhd/tests/fake.rs | `provider_measurement_groups_fake_rehearsal_round_trips_without_claiming_measurements` | provider measurement groups round trip on the fake | REWRITE-INITIAL | provider_groups() lost rx_start_lead, tx_start_lead and their companions (Run-time rate-transition campaigns, redesigned in stages 3 and 4); min_timed_command_lead, tx_in_flight and full_duplex connect with initial tx (candidate_session_asking); expected keys and observations updated |
| crates/ezsdr-radio-uhd/tests/fake.rs | `leads_and_the_overflow_gap_add_a_condition_with_the_other_direction_running -> the_overflow_gap_adds_a_condition_with_the_other_direction_running` | overflow gap companion group with tx running | REWRITE-INITIAL (m5) | the two start-lead halves are gone with their groups; the overflow solo/companion half (fake.rs@HEAD:8826-8834) is restored unchanged |
| crates/ezsdr-radio-uhd/tests/fake.rs | `a_companion_group_is_skipped_with_an_observation_when_the_layout_lacks_the_opposite_direction` | companion group skipped without the opposite direction | REWRITE-INITIAL | only the overflow companion left (the two start-lead companions are gone); not failing before |
| crates/ezsdr-radio-uhd/tests/fake.rs | `group_run_keeps_the_other_groups_after_an_error_or_a_panic` | GroupRun keeps the other groups after an error or panic | REWRITE-INITIAL | passes after the group list shrank (it failed through the removed start-lead groups) |
| crates/ezsdr-radio-uhd/tests/fake.rs | `h4_counts_a_timed_tx_sample_clock_underflow_and_excludes_later_gain_updates` | h4 counts a timed tx sample clock underflow and excludes later gain updates | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `h4_fake_session_records_timed_rx_gain_and_tx_submission_proxies` | h4 fake session records timed rx gain and tx submission proxies | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `h4_not_submitted_updates_keep_their_direction_channel_setting_and_instant` | h4 not submitted updates keep their direction channel setting and instant | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `the_ma1_phase_probe_reads_a_phase_in_both_ways_on_the_fake` | the ma1 phase probe reads a phase in both ways on the fake | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_13_a_runtime_join_compares_the_rate_set` | ur 13 a runtime join compares the rate set | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_13_an_enable_from_no_stream_compares_the_rate_set` | ur 13 an enable from no stream compares the rate set | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_13_the_receive_owner_s_first_open_is_compared_there` | ur 13 the receive owner s first open is compared there | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_13_the_reopen_enforces_the_direction_s_rate_comparison` | ur 13 the reopen enforces the direction s rate comparison | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_14_a_device_error_while_booking_names_its_action` | ur 14 a device error while booking names its action | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_14_actions_are_finished_before_the_next_recv` | ur 14 actions are finished before the next recv | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_14_the_control_thread_never_waits_for_a_device_instant` | ur 14 the control thread never waits for a device instant | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_21_an_untimed_burst_after_a_cold_change_starts_no_earlier_than_its_clock_s_origin` | ur 21 an untimed burst after a cold change starts no earlier than its clock s origin | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_24_a_released_command_is_not_recalled_by_stop` | ur 24 a released command is not recalled by stop | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_24_a_restart_after_deferred_frequencies_starts_on_the_last_one` | ur 24 a restart after deferred frequencies starts on the last one | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_24_a_running_stream_takes_many_retunes_without_filling_the_ddc` | ur 24 a running stream takes many retunes without filling the ddc | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_24_a_timed_update_acts_on_the_stream_that_holds_its_channel` | ur 24 a timed update acts on the stream that holds its channel | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_24_a_timed_update_for_a_channel_out_of_its_stream_waits_for_its_segment` | ur 24 a timed update for a channel out of its stream waits for its segment | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_24_a_transmit_frequency_booked_between_bursts_takes_the_next_bursts_first_sample` | ur 24 a transmit frequency booked between bursts takes the next bursts first sample | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_24_a_transmit_frequency_inside_a_burst_is_taken_at_its_time` | ur 24 a transmit frequency inside a burst is taken at its time | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_24_an_untimed_retune_is_on_time_after_its_delivery` | ur 24 an untimed retune is on time after its delivery | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_24_stop_cancels_held_commands` | ur 24 stop cancels held commands | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_a_channel_enabled_from_no_stream_in_a_later_stream_opens_and_starts_it` | ur 25 a channel enabled from no stream in a later stream opens and starts it | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_a_channel_leaving_a_later_stream_cuts_its_stream_alone` | ur 25 a channel leaving a later stream cuts its stream alone | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_a_channel_s_antenna_change_cuts_its_stream` | ur 25 a channel s antenna change cuts its stream | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_a_cold_change_booked_anywhere_in_a_long_receive_call_is_on_time -> ur_25_a_start_booked_anywhere_in_a_long_receive_call_is_on_time` | start lead over a receive call in progress (UR-25) | REWRITE-INITIAL (M2b) | initial rate 390 625 S/s, block_len 65 536 / 100 (packet 1 996), six phases; Stop then start_rx pushed together at `direct.now()`; asserts origin - booked >= 50 ms + max(block, packet) x 512 + 3 ms, the new segment's `at` before its origin, no LATE_COMMAND (`bench_link` restored) |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_a_cold_change_the_envelope_refuses_changes_nothing` | ur 25 a cold change the envelope refuses changes nothing | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_a_cold_transmit_change_cancels_the_held_bursts` | ur 25 a cold transmit change cancels the held bursts | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_a_configure_overlapped_with_a_timed_update_to_the_same_projection_finishes` | ur 25 a configure overlapped with a timed update to the same projection finishes | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_a_late_switch_drops_the_old_stream_s_tail` | ur 25 a late switch drops the old stream s tail | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_a_layout_instance_s_full_duplex_rule_is_judged_at_runtime` | ur 25 a layout instance s full duplex rule is judged at runtime | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_a_layout_instance_s_throughput_rule_is_judged_at_runtime` | ur 25 a layout instance s throughput rule is judged at runtime | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_a_layout_rate_change_cuts_its_stream_and_configures_every_channel` | ur 25 a layout rate change cuts its stream and configures every channel | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_a_layout_rate_change_of_a_channel_less_stream_only_takes_its_value` | ur 25 a layout rate change of a channel less stream only takes its value | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_a_layout_transmit_rate_change_configures_the_new_segment` | ur 25 a layout transmit rate change configures the new segment | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_a_long_block_at_a_low_rate_stops_the_stream_at_e1` | ur 25 a long block at a low rate stops the stream at e1 | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_a_no_op_membership_change_takes_only_its_value` | ur 25 a no op membership change takes only its value | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_a_rate_change_admits_a_burst_on_the_new_clock` | ur 25 a rate change admits a burst on the new clock | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_a_rate_change_of_a_later_stream_cuts_that_stream_alone` | ur 25 a rate change of a later stream cuts that stream alone | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_a_receive_enable_counts_its_start_lead_from_the_end_of_its_configuration` | ur 25 a receive enable counts its start lead from the end of its configuration | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_a_receive_membership_join_opens_the_streamer_with_the_new_set` | ur 25 a receive membership join opens the streamer with the new set | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_a_receive_membership_leave_opens_the_streamer_for_the_remaining_set` | ur 25 a receive membership leave opens the streamer for the remaining set | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_a_refused_antenna_read_back_halts_the_stream` | ur 25 a refused antenna read back halts the stream | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_a_refused_change_of_a_later_stream_halts_that_stream_alone -> ur_25_a_refused_start_of_a_later_stream_halts_that_stream_alone` | a refused start of a later stream halts that stream alone (UR-25 S8) | REWRITE-INITIAL (M2a) | rx_two, WrongRate{claimed 1e6, applied 0.9e6, nth 1}, Stop rx/s1, start_rx rx/s1: one COMMAND_REJECTED "UR-12: the device applied 900000 S/s", the `rx_start 1` count stays 1, s1 has one ended clock, s0 stays live and still delivers |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_a_refused_enable_from_no_stream_halts_and_keeps_its_values` | ur 25 a refused enable from no stream halts and keeps its values | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_a_slow_link_at_a_low_rate_delivers_the_old_clock_to_e1` | ur 25 a slow link at a low rate delivers the old clock to e1 | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_a_successful_change_clears_a_later_stream_s_halt -> ur_25_a_successful_start_clears_a_later_stream_s_halt` | a later successful start clears the halt (UR-25 O2) | REWRITE-INITIAL (M2a) | same rig; a second start_rx on rx/s1 succeeds: `rx_start 1` +1, no second refusal, s1 has two clocks (one ended, one live), s0 delivers throughout |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_a_transmit_rate_change_of_a_later_stream_cuts_that_stream_alone` | ur 25 a transmit rate change of a later stream cuts that stream alone | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_an_antenna_change_of_a_channel_in_no_stream_only_records` | ur 25 an antenna change of a channel in no stream only records | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_enabling_tx_applies_the_configuration` | ur 25 enabling tx applies the configuration | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_the_first_segment_after_arm_does_not_reopen_the_streamer` | ur 25 the first segment after arm does not reopen the streamer | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_the_last_receive_channel_leaves_no_clock_and_no_block` | ur 25 the last receive channel leaves no clock and no block | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_tx_channels_to_zero_ends_the_clock_and_a_re_enable_starts_a_new_one` | ur 25 tx channels to zero ends the clock and a re enable starts a new one | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_26_d14_a_layout_instance_names_its_streams` | D14 stream nodes: burst, per-stream Stop, start_rx | REWRITE-INITIAL (M2d) | steps 1-3 (the Run-time updates) dropped; steps 4-6 kept as 1-3: a burst on tx/s0 plays, Stop of rx/s0 stops that stream alone and cancels no burst, start_rx starts a second clock |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_34_a_join_after_a_leave_closes_the_source_s_streamer_first` | ur 34 a join after a leave closes the source s streamer first | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_34_a_membership_change_to_its_own_stream_changes_nothing` | ur 34 a membership change to its own stream changes nothing | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_34_a_move_cuts_both_streams_and_the_source_reopens_before_the_destination_opens` | ur 34 a move cuts both streams and the source reopens before the destination opens | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_34_a_move_destination_configure_refusal_halts_both_lines_and_recovers` | ur 34 a move destination configure refusal halts both lines and recovers | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_34_a_move_destination_open_refusal_halts_both_lines_and_recovers` | ur 34 a move destination open refusal halts both lines and recovers | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_34_a_move_from_a_stopped_stream_closes_its_streamer_for_the_destination` | ur 34 a move from a stopped stream closes its streamer for the destination | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_34_a_move_into_an_empty_stream_opens_it_after_the_source_s_release` | ur 34 a move into an empty stream opens it after the source s release | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_34_a_move_of_the_source_s_only_channel_closes_its_streamer` | ur 34 a move of the source s only channel closes its streamer | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_34_a_move_source_configure_refusal_halts_both_lines_and_recovers` | ur 34 a move source configure refusal halts both lines and recovers | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_34_a_move_source_reopen_refusal_halts_both_lines_and_recovers` | ur 34 a move source reopen refusal halts both lines and recovers | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_34_a_rate_check_refusal_closes_the_streamer_before_a_later_move` | ur 34 a rate check refusal closes the streamer before a later move | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_34_a_transmit_join_after_a_leave_closes_the_source_s_streamer_first` | ur 34 a transmit join after a leave closes the source s streamer first | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_34_an_idle_destination_leaves_its_join_to_the_owner_when_another_streamer_holds_the_channel` | ur 34 an idle destination leaves its join to the owner when another streamer holds the channel | DELETE | Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs one |
| crates/ezsdr-radio-uhd/src/provider/control/differential.rs | `rm_26_the_timeline_against_uhd_control` | VH differential of uhd-control against the timeline | RESTORED (M3) | module restored, filtered like the Mock side: only the generator's Stop, StartRx and Loss are booked through `Control`, the Cold/Timed ops are skipped; Record is rx/tx clocks and rejected; 1 000 seeds pass. The Ceiling machinery (`ceilings`, `release_instants`, `drop_row`, the `timed`/`late`/`applied` fields) is gone with the timed commands |
| crates/ezsdr-radio-uhd/src/provider/control.rs | `ur_25_a_lost_stream_starts_no_rate_check_apply_or_booking_after_the_call` | ur 25 a lost stream starts no rate check apply or booking after the call | DELETE | from-none / move / membership / cold-change machinery |
| crates/ezsdr-radio-uhd/src/provider/control.rs | `ur_25_an_owner_waiting_behind_a_from_none_open_starts_nothing` | ur 25 an owner waiting behind a from none open starts nothing | DELETE | from-none / move / membership / cold-change machinery |
| crates/ezsdr-radio-uhd/src/provider/control.rs | `ur_26_an_enable_from_no_stream_a_stop_recorded_after_its_open_books_nothing` | ur 26 an enable from no stream a stop recorded after its open books nothing | DELETE | from-none / move / membership / cold-change machinery |
| crates/ezsdr-radio-uhd/src/provider/control.rs | `ur_29_a_lost_stream_between_the_rate_check_and_the_configure_applies_nothing` | ur 29 a lost stream between the rate check and the configure applies nothing | DELETE | from-none / move / membership / cold-change machinery |
| crates/ezsdr-radio-uhd/src/provider/rx.rs | `ur_24_a_left_stays_on_the_old_chain_after_channel_move` | ur 24 a left stays on the old chain after channel move | DELETE | from-none / move / membership / cold-change machinery |
| crates/ezsdr-radio-uhd/src/provider/rx.rs | `ur_25_a_same_size_membership_swap_serves_the_new_segment_from_its_own_channel` | ur 25 a same size membership swap serves the new segment from its own channel | DELETE | from-none / move / membership / cold-change machinery |
| crates/ezsdr-radio-uhd/src/provider/rx.rs | `ur_34_a_finite_middle_segment_releases_a_channel_along_a_move_chain` | ur 34 a finite middle segment releases a channel along a move chain | DELETE | from-none / move / membership / cold-change machinery |
| crates/ezsdr-radio-uhd/src/provider/rx.rs | `ur_34_two_moves_at_one_instant_swap_their_receive_channels_without_deadlock` | ur 34 two moves at one instant swap their receive channels without deadlock | DELETE | from-none / move / membership / cold-change machinery |
| crates/ezsdr-radio-uhd/src/provider/tx.rs | `ur_13_a_transmit_from_none_segment_the_owner_never_started_is_opened_again` | ur 13 a transmit from none segment the owner never started is opened again | DELETE | from-none / move / membership / cold-change machinery |
| crates/ezsdr-radio-uhd/src/provider/tx.rs | `ur_24_a_transmit_release_without_a_live_channel_reaches_the_pending_segment` | ur 24 a transmit release without a live channel reaches the pending segment | DELETE | from-none / move / membership / cold-change machinery |
| crates/ezsdr-radio-uhd/src/provider/tx.rs | `ur_25_a_from_none_enable_ends_its_wait_at_the_provider_stop` | ur 25 a from none enable ends its wait at the provider stop | DELETE | from-none / move / membership / cold-change machinery |
| crates/ezsdr-radio-uhd/src/provider/tx.rs | `ur_25_a_from_none_enable_waits_for_a_transmit_report_read` | ur 25 a from none enable waits for a transmit report read | DELETE | from-none / move / membership / cold-change machinery |
| crates/ezsdr-radio-uhd/src/provider/tx.rs | `ur_25_a_same_size_membership_swap_sends_the_new_segment_on_its_own_channel` | ur 25 a same size membership swap sends the new segment on its own channel | DELETE | from-none / move / membership / cold-change machinery |
| crates/ezsdr-radio-uhd/src/provider/tx.rs | `ur_25_a_shutdown_read_and_a_returning_from_none_resolve_with_the_control_first` | ur 25 a shutdown read and a returning from none resolve with the control first | DELETE | from-none / move / membership / cold-change machinery |
| crates/ezsdr-radio-uhd/src/provider/tx.rs | `ur_25_a_shutdown_read_and_a_returning_from_none_resolve_with_the_read_first` | ur 25 a shutdown read and a returning from none resolve with the read first | DELETE | from-none / move / membership / cold-change machinery |
| crates/ezsdr-radio-uhd/src/provider/tx.rs | `ur_25_a_transmit_owner_waiting_behind_a_from_none_open_starts_nothing` | ur 25 a transmit owner waiting behind a from none open starts nothing | DELETE | from-none / move / membership / cold-change machinery |
| crates/ezsdr-radio-uhd/src/provider/tx.rs | `ur_25_a_transmit_poll_during_a_control_open_starts_no_streamer_call` | ur 25 a transmit poll during a control open starts no streamer call | DELETE | from-none / move / membership / cold-change machinery |
| crates/ezsdr-radio-uhd/src/provider/tx.rs | `ur_25_a_transmit_report_read_of_a_later_stream_does_not_hold_the_earlier_owner` | ur 25 a transmit report read of a later stream does not hold the earlier owner | DELETE | from-none / move / membership / cold-change machinery |
| crates/ezsdr-radio-uhd/src/provider/tx.rs | `ur_25_cold_change_does_not_inherit_old_tx_front` | ur 25 cold change does not inherit old tx front | DELETE | from-none / move / membership / cold-change machinery |
| crates/ezsdr-radio-uhd/src/provider/tx.rs | `ur_28_a_from_none_open_replaces_the_report_queue` | ur 28 a from none open replaces the report queue | DELETE | from-none / move / membership / cold-change machinery |
| crates/ezsdr-radio-uhd/src/provider/tx.rs | `ur_28_a_poll_after_a_from_none_open_names_no_burst_of_the_replaced_queue` | ur 28 a poll after a from none open names no burst of the replaced queue | DELETE | from-none / move / membership / cold-change machinery |
| crates/ezsdr-radio-uhd/src/provider/tx.rs | `ur_28_a_shutdown_after_a_from_none_open_names_no_burst_of_the_replaced_queue` | ur 28 a shutdown after a from none open names no burst of the replaced queue | DELETE | from-none / move / membership / cold-change machinery |
| crates/ezsdr-radio-uhd/src/provider/tx.rs | `ur_29_a_lost_device_ends_a_from_none_enable_s_wait_for_a_report_read` | ur 29 a lost device ends a from none enable s wait for a report read | DELETE | from-none / move / membership / cold-change machinery |
| crates/ezsdr-radio-uhd/src/provider/tx.rs | `ur_34_a_transmit_move_carries_a_prebooked_burst_across_the_repeat_cut` | ur 34 a transmit move carries a prebooked burst across the repeat cut | DELETE | from-none / move / membership / cold-change machinery |
| crates/ezsdr-radio-uhd/src/provider/tx.rs | `ur_34_a_transmit_move_ends_the_old_burst_and_sends_the_new_burst_on_its_stream` | ur 34 a transmit move ends the old burst and sends the new burst on its stream | DELETE | from-none / move / membership / cold-change machinery |
| crates/ezsdr-radio-uhd/src/provider/tx.rs | `ur_34_two_moves_at_one_instant_swap_their_transmit_channels_without_deadlock` | ur 34 two moves at one instant swap their transmit channels without deadlock | DELETE | from-none / move / membership / cold-change machinery |
| crates/ezsdr-radio-uhd/src/provider/control/differential.rs | `rm_26_the_ceiling_is_decided_from_the_sequence` | timed command Ceiling at a loss | DELETE | its subject is the timed-command Ceiling, a deleted mechanism (the re-check agrees) |
| crates/ezsdr-radio-uhd/tests/hardware.rs | `hw_h4_dispatch -> hw_h4_load` | H4 transmit headroom and load on hardware | RESTORED (M1), dispatch proxy dropped | probe asks rx and tx c0 at 10 MS/s at connect (`h4_initial`), `await_tx`, `start_repeat`, waits `H4_WINDOW_MS`, no timed update; records `tx_headroom`, the CPU delta and the thread snapshot; wired into `hw_measurements_record` as `hw_h4_load` |
| crates/ezsdr-radio-uhd/tests/hardware.rs | `hw_measurements_record` | measurement record on hardware | REWRITE-INITIAL | without the rx/tx start-lead groups and their reopened forms; bench, not run |
| crates/ezsdr-radio-uhd/tests/hardware.rs | `hw_b8_leads / tx_session` | tx enable | REWRITE-INITIAL | connect with rx0_tx0; bench, not run |
| crates/ezsdr-radio-uhd/tests/hardware.rs | `hw_b9_unplug_transmit_only` | transmit-only Session (B9) | REWRITE-INITIAL (m4) | `tx_only_session` (initial tx c0 stream 0 only, via `common::tx_only`); the rx-disable submit is gone |
| crates/ezsdr-radio-uhd/tests/hardware.rs | `hw_b9_transmit_only_session` | transmit-only Session (B9) | REWRITE-INITIAL (m4) | as hw_b9_unplug_transmit_only |
| crates/ezsdr-radio-uhd/tests/hardware.rs | `cold_change_capture (helper of hw_b8_cold_change_capture*)` | capture across a Run-time rate change | PARKED | measured a Run-time rate change; the two tests and the helper are replaced by a comment naming stage 3's `start_rx` start-lead measurement |
| crates/ezsdr-radio-uhd/tests/hardware.rs | `hw_b8_cold_change_timing_at_the_extremes` | start lead at the rates' extremes after a Run-time rate change | PARKED | same reason; stage 3 re-measures the start lead of `start_rx` (the fake pin is `ur_25_a_start_booked_anywhere_in_a_long_receive_call_is_on_time`) |
| crates/ezsdr-radio-uhd/tests/hardware.rs | `hw_txp_b0_send_sizes` | transmit send sizes at 1 and 5 MS/s | REWRITE-INITIAL (m4) | gain 0 dB and the rate asked at connect (`with_initial` over `rx0_tx0`), `past_t0` then `past_tx_origin`; the two Run-time sets are gone |
| crates/ezsdr-radio-uhd/tests/hardware.rs | `hw_56_stop_between_starts` | Stop, pause, start_rx on the device | REWRITE-INITIAL (m4) | initial rx s0 at 2 MS/s with a Stop as the cut, then start_rx; the old cold change and its e1/e2 classification are gone, the device call order compares the first rx_stop after the Stop with the restart's rx_start |
| crates/ezsdr-radio-uhd/tests/hardware.rs | `hw_ur17_receive_call_duration` | receive-call duration, raw and through a Session with a cut | REWRITE-INITIAL (m4) | second half: initial 50 MS/s, reader paused, Stop as the cut and a pending segment made by start_rx; the record keys `stop_submit` and `start_rx_submit` replace `cold_change_submit` |
| python/tests/test_easy_api.py | `test_v3_parameters_through_attributes` | values through attributes | REWRITE-INITIAL | connect(rx={0: Stream([0], sample_rate=19.5e6, frequency, gain, antenna)}); setters raise Error naming connect(rx=ezsdr.Stream( |
| python/tests/test_easy_api.py | `test_ea_16_channel_keys_and_stream_membership` | channel keys and membership | REWRITE-INITIAL + REFUSAL | values and membership at connect; setters raise Error; sdr.set -> Rejected (ezsdr.update_class), logged |
| python/tests/test_easy_api.py | `test_ea_16_stream_channels_are_read_only` | stream.channels read only | REWRITE-INITIAL | join by setter now raises Error; channels unchanged |
| python/tests/test_easy_api.py | `test_ea_16_radio_side_is_stream_zero` | radio side is stream 0 | REWRITE-INITIAL | rate setters removed |
| python/tests/test_easy_api.py | `test_ea_16_errors` | errors incl. RunEnded after stop | REWRITE-INITIAL | RunEnded now pinned through sdr.set (the setter refuses in the client first) |
| python/tests/test_easy_api.py | `test_ea_16_rx_stop_then_start` | Stop then start_rx on new SampleClocks | REWRITE-INITIAL | rate changes removed; three clocks at 1 MS/s |
| python/tests/test_easy_api.py | `test_ea_17_a_capture_across_a_rate_change_is_refused -> ..._across_a_restart_is_refused` | capture across a gap | REWRITE-INITIAL | Stop + start_rx mid-capture instead of a rate change |
| python/tests/test_easy_api.py | `test_v58_13_the_session_manifest_is_written_and_complete` | manifest with a rejected call | REFUSAL | sdr.set(rx.s0.sample_rate_hz, 1e12) -> Rejected, logged |
| python/tests/test_easy_api.py | `test_v58_16_a_rejected_call_raises_and_is_logged` | rejected call raises and is logged | REFUSAL | sdr.set(tx.c0.frequency_hz) -> Rejected with check ezsdr.update_class; initial tx frequency in the band connects |
| python/examples/bench_loopback.py | `(example)` | sample-rate and retune setters | REWRITE-INITIAL | rate asked at connect; the retune demo uses sdr.set and expects Rejected |
| crates/ezsdr-radio-uhd/tests/hardware.rs | `hw_ma1_phase_after_a_pre_start_tune` | LO phase after an untimed receive tune (MA-1) | REWRITE-INITIAL (m4) | `ma1_run` asks the receive and transmit frequency at connect (`with_initial`); only the untimed way is kept, the timed way (a Spec schedule retune) has no Run-time form; one way, N Sessions |
| crates/ezsdr-radio-uhd/tests/common/measurement/h4.rs | `h4_mapping_rejects_a_monotonic_forward_clock_jump, h4_tx_headroom_uses_per_channel_cumulative_accepts_and_breaks_bad_segments, h4_tx_headroom_restarts_after_an_error_and_eob, h4_tx_headroom_discards_a_discontinuous_host_device_mapping, h4_zero_sample_send_breaks_the_headroom_segment, h4_proc_parsers_retain_cpu_and_thread_policy_fields` | H4 clock mapping, headroom and load parsers (6 units) | RESTORED (M1) | unchanged, pure |
| crates/ezsdr-radio-uhd/tests/common/measurement/h4.rs | `h4_dispatch_maps_signed_intervals_and_refuses_bad_anchors -> h4_mapping_gives_an_ordered_interval_and_refuses_bad_anchors` | clock mapping refuses bad anchors | REWRITE (M1) | the `dispatch_delay` half goes with the dispatch proxy; the `map_device_tick` half is kept with an ordered-interval check |
| crates/ezsdr-radio-uhd/tests/common/measurement/h4.rs | `h4_tx_overlap_classifies_an_apply_against_the_tx_interval, h4_the_measurement_finds_the_apply_recorded_at_its_instant, h4_the_gain_update_is_booked_at_its_instant, h4_a_successful_dispatch_maps_into_its_row_and_a_missing_anchor_does_not, h4_an_underflow_ends_tx_and_later_applied_gain_updates_are_excluded_from_the_distribution` | the dispatch proxy of timed gain updates (5 units) | DELETE | the observation note 47 drops |
| crates/ezsdr-radio-uhd/tests/fake.rs | `h4_fake_session_records_tx_headroom_and_load_with_no_run_time_update` | H4 probe rehearsal on the fake (replaces h4_fake_session_records_timed_rx_gain_and_tx_submission_proxies) | REWRITE-INITIAL (M1) | `measure_h4_load(probe, 200)` with the selector override: status complete or partial, 200 MHz master, 10 MS/s, no timed apply reached the device, tx payload sends and a non-empty `tx_headroom`, the proxy text, CPU and thread objects |
| crates/ezsdr-radio-uhd/tests/fake.rs | `h4_a_tx_underflow_makes_the_load_record_partial_and_converts_its_time` | H4 with a TX underflow (replaces h4_counts_a_timed_tx_sample_clock_underflow_and_excludes_later_gain_updates) | REWRITE-INITIAL (M1) | `underflow_after_tx_send`: underflow events > 0, converted > 0, unconvertible 0, status partial |
| crates/ezsdr-radio-uhd/tests/fake.rs | `h4_not_submitted_updates_keep_their_direction_channel_setting_and_instant` | H4 rows of updates not submitted | DELETE | no update rows exist any more |
| crates/ezsdr-radio-uhd/tests/common/measurement.rs | `tx_start_lead_requires_an_early_switch_and_no_failure_evidence, tx_start_lead_value_uses_the_successful_suffix_intersection` | the TX start-lead group's class and value rules | DELETE (deviation from M1) | M1 asked to restore them; they test `tx_start_trial_class`, `tx_start_candidate_stage_json` and `summarize_tx_start_lead`, which only the TX start-lead group (a Run-time TX rate change) used. Restoring them would restore dead production code, so they stay deleted and `tx_start_leads_ns` is removed from the plan; `radio.timing.tx_start_lead_ns` reads "no Run-time form (note 47 stage 2)". Raise it if the owner wants them back |
| crates/ezsdr-acceptance/tests/v58.rs | `v58_16_* (two tests), mr_34_a_scheduled_radio_value_update_is_refused_at_validate` | REFUSAL checks (m2) | REFUSAL, strengthened | connect refusals assert `radio.rf_envelope` in the Failed reason; the Session refusals assert a violation of check `ezsdr.update_class` ("declares no update class"); both v58_16 tests assert the rejection is in the action log; the scheduled update asserts the key in the Validate reason and has a control case (the same Spec with a scheduled Stop is not refused at Validate) |
| crates/ezsdr-acceptance/tests/uhd.rs | `uhd_59_a_value_outside_the_rf_envelope_is_refused_at_connect_and_a_runtime_change_has_no_class` | REFUSAL check (m2) | REFUSAL, strengthened | `radio.rf_envelope` in the connect reason; `ezsdr.update_class` violation on the Session set; the rejection is logged (already asserted) |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_34_a_layout_deterministic_channel_keeps_its_phase_and_coherence_through_entry` | deterministic entry keeps phase 0 and stays coherent (MR-34) | REWRITE-INITIAL (m3) | Stop and start_rx of mock/rx/s0 replace the two rejected membership pushes; asserts `rejected()` empty and two receive clocks, so the entry is real |
| fake.rs, measurement.rs, python/ezsdr/session.py | `stale text and leftovers (m6)` | comments and converters | CLEANUP | fake.rs: the `ur_25_a_receive_session_follows_its_recorded_plan` header names the two Stop/start_rx pairs and three segments; the two pointers to deleted tx.rs units (`ur_34_a_transmit_move_*`) are removed. measurement.rs: `capture_action`'s doc comment, the unmeasured reasons of rx/tx start lead (note 47), `tx_start_leads_ns` removed from the plan and its validation, `rx_start_leads_ns` kept for stage 3 with a doc line. session.py: `_PARAMS` holds only the key suffixes (the unused converters are gone). `work/recheck.tsv`: rx.rs:3512 and tx.rs:4234/4246/4469 now say they fail with the flip, v58.rs:567 says rewritten |

## The re-check's named tests (`opus-s2a-plan2-final.md`) and where each landed

The re-check named tests by `file:line` of HEAD; each is resolved to its function. "Kept as is" means the test did not fail after the flip; it tests internals of the removed machinery or a surviving subject, and part (b) judges it with the code it exercises.

| HEAD location | test | landed |
|---|---|---|
| control.rs:1507 | `ur_13_a_rate_refusal_from_no_stream_is_reapplied_and_starts_nothing` | not failing, kept as is (part (b) judges it with the code it exercises) |
| control.rs:1523 | `ur_25_a_transmit_refusal_from_no_stream_starts_no_clock` | not failing, kept as is (part (b) judges it with the code it exercises) |
| control.rs:1816 | `ur_34_a_control_books_the_next_action_while_an_owner_close_is_gated` | not failing, kept as is (part (b) judges it with the code it exercises) |
| control.rs:2062 | `ur_24_a_cancelled_held_update_leaves_the_projection` | not failing, kept as is (part (b) judges it with the code it exercises) |
| control.rs:2734 | `ur_30_the_recorded_plan_has_no_refused_segment` | not failing, kept as is (part (b) judges it with the code it exercises) |
| control.rs:3246 | `ur_24_a_late_update_halted_before_it_can_act_has_no_channel` | not failing, kept as is (part (b) judges it with the code it exercises) |
| control.rs:3779 | `ur_25_an_enable_behind_an_owner_s_open_is_a_cold_change` | not failing, kept as is (part (b) judges it with the code it exercises) |
| control.rs:4329 | `ur_24_a_layout_release_reaches_the_later_segment_s_channel` | not failing, kept as is (part (b) judges it with the code it exercises) |
| control.rs:4958 | `ur_25_the_switch_applies_the_configuration_in_effect_at_e2` | not failing, kept as is (part (b) judges it with the code it exercises) |
| control.rs:7327 | `ur_25_the_mark_is_published_while_the_projection_lock_is_held` | not failing, kept as is (part (b) judges it with the code it exercises) |
| control.rs:8475 | `ur_24_a_configured_successor_configure_refusal_keeps_frequency_held` | not failing, kept as is (part (b) judges it with the code it exercises) |
| fake.rs:1425 | `queue_depth_streams_and_drains_the_receiver_only_for_receive_frequency` | not failing, kept as is (part (b) judges it with the code it exercises) |
| fake.rs:2708 | `ur_24_a_released_command_is_not_recalled_by_stop` | DELETE: Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs  |
| fake.rs:2738 | `ur_24_stop_cancels_held_commands` | DELETE: Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs  |
| fake.rs:2987 | `ur_25_a_receive_session_follows_its_recorded_plan` | REWRITE-INITIAL: two Stop + start_rx cycles and the Run stop (three segments) instead of two cold rate changes |
| fake.rs:3814 | `ur_29_a_stream_silent_before_a_far_cut_is_a_lost_device` | not failing, kept as is (part (b) judges it with the code it exercises) |
| fake.rs:3889 | `ur_25_a_cold_change_booked_anywhere_in_a_long_receive_call_is_on_time` | REWRITE-INITIAL (rev 4, M2b): `ur_25_a_start_booked_anywhere_in_a_long_receive_call_is_on_time` |
| fake.rs:5757 | `ur_13_arm_compares_the_rate_set_after_the_streamer_opens` | not failing, kept as is (part (b) judges it with the code it exercises) |
| fake.rs:5846 | `ur_34_a_move_destination_open_refusal_halts_both_lines_and_recovers` | DELETE: Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs  |
| fake.rs:7105 | `ur_13_the_reopen_enforces_the_direction_s_rate_comparison` | DELETE: Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs  |
| fake.rs:7596 | `ur_24_a_timed_update_acts_on_the_stream_that_holds_its_channel` | DELETE: Run-time change (enable/join/leave/move/rate/antenna/timed update/release/cold change) or a measurement (H4) that needs  |
| fake.rs:8051 | `ur_26_a_receive_start_queued_by_the_stop_is_issued_and_ends_at_its_origin` | REWRITE-INITIAL: Stop s1 + start_rx s1 with the owner's configure held, then Provider::stop (the Opus re-check's KEEP-INITIAL) |
| fake.rs:8195 | `ur_26_a_stopped_later_stream_waits_for_start_rx` | not failing, kept as is (part (b) judges it with the code it exercises) |
| hardware.rs:698 | `hw_b8_cold_change_capture` | not failing, kept as is (part (b) judges it with the code it exercises) |
| hardware.rs:1613 | `hw_b8_cold_change_timing_at_the_extremes` | ?: still submits Run-time sets; ignored bench, not run, and will be refused on hardware. Decide with the bench plan (stages |
| mock_channel.rs:853 | `mr_33_a_receiver_hears_only_its_own_frequency` | REWRITE-INITIAL: two runs with initial tx/rx frequencies equal and unequal |
| mock_channel.rs:966 | `mr_30_untimed_receive_tune_precedes_transmit_at_one_origin` | DELETE: Run-time rate change |
| mock_layout.rs:1433 | `mr_33_layout_gain_and_frequency_timelines_are_per_logical_channel` | REWRITE-INITIAL: eight values as initial constraints instead of hardware_timed updates |
| mock_layout.rs:2598 | `mr_20_a_hardware_timed_update_in_a_refused_halt_is_recorded` | DELETE: Run-time change machinery |
| mock_layout.rs:2624 | `mr_20_a_refused_transmit_change_ends_its_clock_and_cancels_its_bursts` | DELETE: Run-time change machinery |
| mock_layout.rs:3271 | `lo_draws` | deleted (not failing before; removed with the module it tested) |
| mock_layout.rs:3274 | `mr_34_a_layout_receive_update_before_t0_is_an_untimed_tune` | DELETE: Run-time change machinery |
| mock_layout.rs:3476 | `mr_30_a_layout_untimed_update_draws_are_reproducible_and_in_application_order` | DELETE: Run-time change machinery |
| mock_layout.rs:3560 | `mr_34_a_layout_a_linkless_stream_does_not_cover_even_after_start_rx` | DELETE: Run-time change machinery |
| mock_radio.rs:1971 | `mr_20_a_fault_between_cold_clocks_is_not_applied` | REWRITE-INITIAL (rev 4, M2c): `mr_20_a_fault_between_receive_clocks_is_not_applied` |
| mock_radio.rs:2737 | `mr_18_a_fractional_receive_cut_is_the_first_sample_at_or_after_e` | DELETE: Run-time change machinery |
| mock_radio.rs:2810 | `mr_18_a_reentry_after_a_later_change_follows_it` | DELETE: Run-time change machinery |
| mock_radio.rs:2919 | `mr_18_a_timed_update_behind_one_inside_the_window_is_late` | DELETE: Run-time change machinery |
| mock_radio.rs:3434 | `mr_20_a_fault_after_every_change_refuses_nothing` | DELETE: Run-time change machinery |
| mock_radio.rs:3541 | `mr_20_a_change_before_the_fault_is_no_candidate` | DELETE: Run-time change machinery |
| parity.rs:676 | `vh_8_the_same_floor_at_t0` | DELETE: Run-time change floor |
| parity.rs:916 | `vh_8_a_floored_refusal_is_reported_in_the_same_order` | DELETE: Run-time change floor |
| parity.rs:1515 | `vh_8_the_instance_floor_crosses_streams` | DELETE: Run-time change floor |
| rx.rs:1484 | `ur_24_multichannel_abort_keeps_overflow_and_drain_samples_off_the_link` | not failing, kept as is (part (b) judges it with the code it exercises) |
| rx.rs:2115 | `ur_24_nonloss_rx_stop_failure_reports_pending_and_skips_restore` | not failing, kept as is (part (b) judges it with the code it exercises) |
| rx.rs:2507 | `ur_34_a_waited_receive_open_exits_when_stop_removes_its_segment` | not failing, kept as is (part (b) judges it with the code it exercises) |
| rx.rs:3512 | `ur_25_a_same_size_membership_swap_serves_the_new_segment_from_its_own_channel` | DELETE: from-none / move / membership / cold-change machinery |
| rx.rs:3940 | `ur_13_a_failed_open_by_the_owner_is_opened_again` | not failing, kept as is (part (b) judges it with the code it exercises) |
| rx.rs:4048 | `ur_17_an_ended_segment_prunes_the_items_before_it` | not failing, kept as is (part (b) judges it with the code it exercises) |
| timeline.rs:883 | `rm_34_a_source_first_prune_keeps_move_dependencies_until_both_lines_pass` | not failing, kept as is (part (b) judges it with the code it exercises) |
| timeline.rs:990 | `rm_34_a_move_is_refused_on_both_lines_whichever_line_names_it` | not failing, kept as is (part (b) judges it with the code it exercises) |
| timeline.rs:1535 | `rm_36_covering_boundaries` | not failing, kept as is (part (b) judges it with the code it exercises) |
| timeline.rs:1665 | `rm_25_a_refusal_does_not_move_a_later_origin` | not failing, kept as is (part (b) judges it with the code it exercises) |
| timeline.rs:1951 | `rm_25_a_booking_adds_no_coverage_before_its_start_lead` | not failing, kept as is (part (b) judges it with the code it exercises) |
| tx.rs:382 | `retarget` | not failing, kept as is (part (b) judges it with the code it exercises) |
| tx.rs:1150 | `do_switch` | not failing, kept as is (part (b) judges it with the code it exercises) |
| tx.rs:1666 | `ur_13_a_transmit_switch_is_compared_with_the_stored_outcome` | not failing, kept as is (part (b) judges it with the code it exercises) |
| tx.rs:4234 | `ur_28_a_poll_after_a_from_none_open_names_no_burst_of_the_replaced_queue` | DELETE: from-none / move / membership / cold-change machinery |
| tx.rs:4246 | `ur_28_a_shutdown_after_a_from_none_open_names_no_burst_of_the_replaced_queue` | DELETE: from-none / move / membership / cold-change machinery |
| tx.rs:4469 | `ur_24_a_transmit_release_without_a_live_channel_reaches_the_pending_segment` | DELETE: from-none / move / membership / cold-change machinery |
| v58.rs:296 | `v58_11_short_lead_burst_is_a_time_error` | REWRITE-INITIAL: loopback_run (tx c0 stream 0 at connect); origin T0+3ms, late 1 ms |
| v58.rs:567 | `v58_16_runtime_rate_beyond_the_envelope_is_rejected` | REFUSAL + REWRITE-INITIAL: initial rate 200 MS/s -> failed Run at validate (RM-32); runtime rate change rejected |

## `?` items for the reviewer (rev 4)

- The bench tests are `#[ignore]` and were not run (no USRP). Rewritten with initial values: `hw_b9_unplug_transmit_only`, `hw_b9_transmit_only_session`, `hw_txp_b0_send_sizes`, `hw_56_stop_between_starts`, `hw_ur17_receive_call_duration`, `hw_ma1_phase_after_a_pre_start_tune` (untimed way only), `hw_h4_load`. Parked: `hw_b8_cold_change_capture`, `hw_b8_cold_change_capture_low_rate`, `hw_b8_cold_change_timing_at_the_extremes` (a Run-time rate change; stage 3's `start_rx` start-lead measurement replaces them). `hw_b8_leads` and the other `tx_session` users connect with `rx0_tx0` and were already green.
- M1 deviation: the two `tx_start_lead_*` units are not restored (see the row): they would restore dead production code.
- `vh_8_the_same_manifest_plan_rows`: the transmit origins are checked on the lattice at or before T0, not compared across rigs.
- `rm_26_the_timeline_against_uhd_control` is restored in a reduced form (no Ceiling machinery); `mr_20_*` tests that needed a refused Run-time change (the second half of the fault-between-clocks test) are not restored.

## For part (b)

What part (b) must keep (the re-check's notes and the dispositions):

- The Mock's per-channel value history: no test exercises it after part (a). Give it a test in (b) before removing code around it, or record it as deleted with the machinery.
- `rx/differential.rs` layer 3 (`rm_26_the_timeline_against_uhd_rx`): the remaining pin for cuts with 65 536-sample blocks and latency.
- The in-source D14 twin at `control.rs:5430` (the control-side target resolution).
- The generator (`crates/ezsdr-radio/tests/generator/mod.rs`) still emits Cold and Timed ops; both differentials skip them. Part (b) takes Cold/Timed out of the generator and rebalances it, because the Timed and TxCold classes now generate empty sequences. When it does, the two `layout_*` functions in the differentials need no `Op::Cold | Op::Timed` arms.
- `rx.rs:587` labels a `start_rx` refusal `update_parameter` with no action id (a probe confirmed it): a note for the stage-3 brief.
- The `ready` scenario of fake.rs@HEAD:2783 goes into the stage-3 brief.
- `rx_start_lead_ns` governs every `start_rx` and has no measurement until stage 3 (part (c) records the gap). `rx_start_leads_ns` stays in `MeasurementPlan` for that group.

The tests m3 hands to part (b), each with its code: `fake.rs` `ur_29_a_stream_silent_before_a_far_cut_is_a_lost_device` (its far cut is a Cold rate change that is now refused, so it passes without testing the lost-device path it names) and the first assertion of `ur_26_a_stopped_later_stream_waits_for_start_rx` (its Cold change on rx/s1 is refused; the Stop then `start_rx` half is a real pin and stays); and `mock_layout.rs` `mr_34_a_layout_unlinked_receive_stream_takes_no_untimed_tune` already uses Stop and `start_rx` and is fine. The in-source tests (`rx.rs`, `tx.rs`, `control.rs`) that reach a refusal through a Cold rate item (`rx.rs:3462, 3639, 3692, 3797` at HEAD) are part (b)'s: the pins of the refusal halt that survive are now `ur_25_a_refused_start_of_a_later_stream_halts_that_stream_alone` and `ur_25_a_successful_start_clears_a_later_stream_s_halt` in `fake.rs`, so deleting them loses nothing.


---

# Part (b) — delete the unreachable Run-time change machinery (revision 5)

`test-plan-s2.v4.md` is revision 4. Part (b) deleted the Providers' change paths with their in-source tests; the rows below are the decisions. Line numbers are those of HEAD `45a4294` (a file part (a) had already changed has its production lines unchanged by part (a)). Mutation rows, specs and KNOWN_LIMITATIONS are part (c).

## 0. The part (a) re-check minors (`dispositions-s2-a-fix.md`, rows 1–5 and Q3)

| # | where | done |
|---|---|---|
| 1 | `mock_radio.rs` `mr_20_a_fault_between_receive_clocks_is_not_applied` | the domain is read after `step(20_000_000)`, two clock records are asserted, the first-block assertions are on the new clock; the comment says 1.01 ms |
| 2 | `mock_layout.rs` `mr_34_a_layout_deterministic_transmit_entry_keeps_phase_zero` | deleted (a transmit entry has no Run-time form until stage 4) |
| 3 | `common/mod.rs` B7 rehearsal | asserts `Outcome::Rejected` with an `ezsdr.update_class` violation |
| 4 | `hardware.rs` `hw_56_stop_between_starts` | Stop, `start_rx` (e1 = its request, e2 = its `rx_segment` origin), a second Stop 10 ms later, then the drain and recovery; the e1/e2 placement field is kept. Compile-only (no bench) |
| 5 | `fake.rs` `refused_second_start_of_stream_1` | the drain of `link0` is just before `start_rx` |
| Q3 | `parity.rs` `vh_8_the_same_manifest_plan_rows` | `planned_with_anchors` returns each rig's T0 and arm instant (Mock arm 0; UHD the `arm` timing row's `at`); each transmit origin is checked `== lattice(arm, n) - T0` |
| N2 | `probe_device.rs` `matching_timed_apply` | deleted with `h4_probe_records_a_timed_apply_under_its_instant_with_its_host_entry` (`fake.rs`) |

## 1. Deleted production regions (HEAD path:line, what it was)

Production regions are named by function; the transmit and receive threads' types are in the second table.

| path:line | name | what it was |
|---|---|---|
| crates/ezsdr-acceptance/tests/parity.rs:273 | `update` |  |
| crates/ezsdr-acceptance/tests/parity.rs:786 | `updates` | The logical updates a Manifest's `applied` section holds, as `(key, value, instant)` |
| crates/ezsdr-acceptance/tests/parity.rs:807 | `late_rows` | Every `LATE_COMMAND` as `(requested, applied)` relative to `t0` (RM-25, UC-2). |
| crates/ezsdr-acceptance/tests/parity.rs:1088 | `mock_rows` |  |
| crates/ezsdr-acceptance/tests/parity.rs:1092 | `uhd_rows` |  |
| crates/ezsdr-acceptance/tests/parity.rs:1394 | `cold_change` |  |
| crates/ezsdr-mock-radio/src/lib.rs:508 | `update_class_of` | MR-18's and RM-27's class an update of `key` must state; `None` when `key` is no |
| crates/ezsdr-mock-radio/src/lib.rs:1009 | `record_update` |  |
| crates/ezsdr-mock-radio/src/lib.rs:1024 | `covering_rx` | MR-34: the stream and origin of the segment of a receiving stream (one with a link, |
| crates/ezsdr-mock-radio/src/lib.rs:1547 | `tx_switch` | MR-18: transmit stream `k`'s clock `to` is current from a change's instant, with a |
| crates/ezsdr-mock-radio/src/lib.rs:1569 | `cancel_later` | Cancels the bursts booked on transmit stream `k`'s clocks not yet current (RM-16, |
| crates/ezsdr-mock-radio/src/lib.rs:1580 | `forget_later` | Withdraws a cancelled burst of transmit stream `k`'s clock `index` from the channel |
| crates/ezsdr-mock-radio/src/lib.rs:1589 | `projected` | The configuration in force at `e` for an update received with `order`: the applied |
| crates/ezsdr-mock-radio/src/lib.rs:1875 | `tx_later_clock` | Whether a transmit clock stream `k`'s booked change starts has been registered. |
| crates/ezsdr-mock-radio/src/lib.rs:2145 | `cold_streams` | RM-27, RM-34: the streams a `cold` change names in the configuration at its effective |
| crates/ezsdr-mock-radio/src/lib.rs:2180 | `channel_stream` | RM-30: the stream a layout configuration puts logical channel `number` of |
| crates/ezsdr-mock-radio/src/lib.rs:2190 | `apply_update` | MR-18: at `e` the value applies and is recorded; a transmit `cold` change ends the old |
| crates/ezsdr-mock-radio/src/lib.rs:2225 | `forget_refused_segment` | MR-27: drops the row of a receive segment the refused change began at its own instant. A |
| crates/ezsdr-mock-radio/src/lib.rs:2239 | `refusal` | MR-20: the `cold_change_refused` fault that refuses the change received with `seq` at |
| crates/ezsdr-mock-radio/src/lib.rs:2247 | `cancel_booked` | MR-18: withdraws the bursts held on transmit stream `k`'s clock `index` — the clock the |
| crates/ezsdr-mock-radio/src/lib.rs:2261 | `apply_layout_channel_update` | Updates only the named logical channel's SimulationChannel timelines (MR-33, MR-34). |
| crates/ezsdr-mock-radio/src/lib.rs:2442 | `int_config_opt` | The integer `configuration` holds for `name`, if it holds one. |
| crates/ezsdr-mock-radio/src/lib.rs:2451 | `direction_of` | RM-29: the direction an RM-27 key's `rx` flag names. |
| crates/ezsdr-mock-radio/src/lib.rs:2457 | `key_direction` | RM-27: the direction a layout configuration key belongs to, when it names a channel or a |
| crates/ezsdr-mock-radio/tests/mock_channel.rs:249 | `update` |  |
| crates/ezsdr-mock-radio/tests/mock_channel.rs:949 | `cbx_phases` |  |
| crates/ezsdr-mock-radio/tests/mock_channel.rs:950 | `phase` |  |
| crates/ezsdr-mock-radio/tests/mock_layout.rs:365 | `applied_of` | The `(value, at)` of every applied update of one key, in application order (MR-27). |
| crates/ezsdr-mock-radio/tests/mock_layout.rs:384 | `update` |  |
| crates/ezsdr-mock-radio/tests/mock_layout.rs:388 | `update_at` |  |
| crates/ezsdr-mock-radio/tests/mock_layout.rs:426 | `two_channel_waveform` | The waveform of a two-channel transmit burst of `frames` samples, both channels at `value`: |
| crates/ezsdr-mock-radio/tests/mock_layout.rs:2318 | `collect_blocks` | Steps `harness` from `from` to `to` in steps of 20 ms and collects the receive blocks of |
| crates/ezsdr-mock-radio/tests/mock_layout.rs:2543 | `cold_change_refused_at` |  |
| crates/ezsdr-mock-radio/tests/mock_layout.rs:2552 | `loop_burst` | A repeated burst on `domain`, so that it is still open at the change that cuts it (RM-15). |
| crates/ezsdr-mock-radio/tests/mock_layout.rs:2693 | `transmit_refusal_harness` | One transmit stream at 1 MS/s whose cold changes the first `refusals` fault lines refuse. |
| crates/ezsdr-mock-radio/tests/mock_layout.rs:2701 | `tx_rate` |  |
| crates/ezsdr-mock-radio/tests/mock_layout.rs:2706 | `tx_segment_origins` | The origin of the transmit clocks in the harness's `segments` section. |
| crates/ezsdr-mock-radio/tests/mock_layout.rs:3193 | `single` | One ubx receive channel `c0` in stream 0, looped back. |
| crates/ezsdr-mock-radio/tests/mock_layout.rs:3198 | `go` | Steps to `tick` and keeps the blocks that came out. |
| crates/ezsdr-mock-radio/tests/mock_layout.rs:3211 | `go_by` | As `go`, in steps of at most 10 ms, so a link of 64 blocks drops none of what it holds. |
| crates/ezsdr-mock-radio/tests/mock_layout.rs:3227 | `last_clock` | The last SampleClock of the stream at `path` and its origin. |
| crates/ezsdr-mock-radio/tests/mock_layout.rs:3234 | `ear` | What channel 0 hears of the clock at `tick`. |
| crates/ezsdr-mock-radio/tests/mock_layout.rs:3239 | `coherence` | The `coherence` rows of the receive segment of stream `stream` that begins at `origin`. |
| crates/ezsdr-mock-radio/tests/mock_layout.rs:3248 | `restart` | Stops stream `path` at `from`, then restarts it, and returns the restarted clock and |
| crates/ezsdr-mock-radio/tests/mock_layout.rs:3260 | `retune_draw` | The `n`th draw of the seed-7 retune stream. |
| crates/ezsdr-mock-radio/tests/mock_layout.rs:3731 | `refusal_run` | One run of the refusal scene: C1 (due `E_C1`) and, with `second`, C2 (due `E_C2`) are booked |
| crates/ezsdr-mock-radio/tests/mock_layout.rs:3754 | `origins` |  |
| crates/ezsdr-mock-radio/tests/mock_layout.rs:3821 | `refusal_origins` | The origins the refusal scene's segments have without the fault and with C1 refused. |
| crates/ezsdr-mock-radio/tests/mock_layout.rs:3838 | `assert_an_update_in_a_refused_span_is_untimed` | The refusal scene with an update `r` received and due at `e`, both before S3 begins at `o3`: |
| crates/ezsdr-mock-radio/tests/mock_radio.rs:2881 | `timed_harness` |  |
| crates/ezsdr-mock-radio/tests/mock_radio.rs:2888 | `applied_of_key` | The applied rows of one key as `(value, at)`, in application order (MR-27). |
| crates/ezsdr-mock-radio/tests/mock_radio.rs:2899 | `late_commands` | The `LATE_COMMAND`s since the last drain as `(key, requested, applied)`. |
| crates/ezsdr-mock-radio/tests/mock_radio.rs:2909 | `queue_full_events` |  |
| crates/ezsdr-mock-radio/tests/mock_radio.rs:3294 | `command_rejections` | The `radio.COMMAND_REJECTED` events a harness has emitted, as `(action, kind, at)`. |
| crates/ezsdr-mock-radio/tests/mock_radio.rs:3305 | `applied_rows` | The `applied` rows of one key, as `(value, at)` (MR-27). |
| crates/ezsdr-mock-radio/tests/mock_radio.rs:3310 | `cold_change_refused_at` |  |
| crates/ezsdr-mock-radio/tests/timeline.rs:85 | `add_membership` |  |
| crates/ezsdr-mock-radio/tests/timeline.rs:170 | `fractional_tx` |  |
| crates/ezsdr-mock-radio/tests/timeline.rs:564 | `side` |  |
| crates/ezsdr-radio-uhd/src/provider/control.rs:50 | `settings` |  |
| crates/ezsdr-radio-uhd/src/provider/control.rs:110 | `with_clock` | The numbered configuration at the segment's origin (RM-30, UR-12), with held timed |
| crates/ezsdr-radio-uhd/src/provider/control.rs:116 | `updates_held` | Test only: whether the projection's lock is held now. |
| crates/ezsdr-radio-uhd/src/provider/control.rs:122 | `updates_waiters` | Test only: the threads waiting for the projection's lock. |
| crates/ezsdr-radio-uhd/src/provider/control.rs:127 | `is_configured` | UR-25: whether the device was configured from this segment's projection (see `configured`). |
| crates/ezsdr-radio-uhd/src/provider/control.rs:138 | `settings_on` | The configuration in effect at `origin` on logical channel `chan` (UR-12, RM-30): |
| crates/ezsdr-radio-uhd/src/provider/control.rs:528 | `held_updates` | UC-2, UR-17: the timed commands held for `dir` as a segment configuration's own |
| crates/ezsdr-radio-uhd/src/provider/control.rs:539 | `projections` | The segment configurations of `dir`, shared with its owners (every stream's). |
| crates/ezsdr-radio-uhd/src/provider/control.rs:559 | `late_command` |  |
| crates/ezsdr-radio-uhd/src/provider/control.rs:566 | `config_at` | The configuration in force at `t`: the one at the Run's start with the timed values |
| crates/ezsdr-radio-uhd/src/provider/control.rs:581 | `channel_live` | RM-27, UR-24: whether logical channel `chan` of `dir` is in a running stream at `t` |
| crates/ezsdr-radio-uhd/src/provider/control.rs:609 | `targets` | UR-24: the channel a timed command is given on at `t` — in force there, or, with none |
| crates/ezsdr-radio-uhd/src/provider/control.rs:624 | `book_timed` |  |
| crates/ezsdr-radio-uhd/src/provider/control.rs:682 | `release` | Releases held commands within the release window, in instant order, each on the |
| crates/ezsdr-radio-uhd/src/provider/control.rs:763 | `record_given` | UR-24: the `applied` row of a command whose instant `e` has come: `issued`, with |
| crates/ezsdr-radio-uhd/src/provider/control.rs:778 | `of_record` | UR-24: `timed` becomes the configuration of record, which a segment beginning at or |
| crates/ezsdr-radio-uhd/src/provider/control.rs:795 | `reach` | UR-24: how a receive frequency due at `e` on `chan` reaches the device. The caller holds |
| crates/ezsdr-radio-uhd/src/provider/control.rs:822 | `give` | UR-24: gives the released command to the device at its instant, on `chans`; its row is |
| crates/ezsdr-radio-uhd/src/provider/control.rs:913 | `settle` | UR-26, UR-30, design 38 D2 (4): the end at its own instant `S` settles every command |
| crates/ezsdr-radio-uhd/src/provider/control.rs:954 | `cancel_held` | UR-26, UR-30: cancels the held commands at `at`, recording their rows. |
| crates/ezsdr-radio-uhd/src/provider/control.rs:964 | `await_reading` | UR-25: an enable from no stream whose owner is reading reports waits for the read to |
| crates/ezsdr-radio-uhd/src/provider/control.rs:989 | `book_cold` | UR-25: a `cold` update is booked when uhd-control takes it: its effective instant, |
| crates/ezsdr-radio-uhd/src/provider/control/differential.rs:40 | `divergent` |  |
| crates/ezsdr-radio-uhd/src/provider/control/differential.rs:90 | `layout_schedule` | The generated operations' effective instants with the layout's instance-wide cold floor |
| crates/ezsdr-radio-uhd/src/provider/control/differential.rs:132 | `layout_items` | A generated channel count is expanded as c0/c1 membership changes at the same instant. |
| crates/ezsdr-radio-uhd/src/provider/control/differential.rs:311 | `reaches` | Whether the expected plan gives `channel` a segment at or after `t`: one covering `t`, or |
| crates/ezsdr-radio-uhd/src/provider/control/differential.rs:390 | `by_instant` | The rows by instant and key, each key's in the order written: two directions' updates |
| crates/ezsdr-radio-uhd/src/provider/control/differential.rs:395 | `side` |  |
| crates/ezsdr-radio-uhd/src/provider/core.rs:163 | `debt` |  |
| crates/ezsdr-radio-uhd/src/provider/core.rs:166 | `left` |  |
| crates/ezsdr-radio-uhd/src/provider/core.rs:169 | `latest` |  |
| crates/ezsdr-radio-uhd/src/provider/core.rs:172 | `restore` |  |
| crates/ezsdr-radio-uhd/src/provider/core.rs:176 | `eligible` |  |
| crates/ezsdr-radio-uhd/src/provider/core.rs:240 | `hand` | Provisional debt is visible while `apply` runs; refusal removes it, loss is terminal. |
| crates/ezsdr-radio-uhd/src/provider/core.rs:277 | `flush_start_result` |  |
| crates/ezsdr-radio-uhd/src/provider/core.rs:293 | `flush_stop_result` |  |
| crates/ezsdr-radio-uhd/src/provider/core.rs:307 | `consume` |  |
| crates/ezsdr-radio-uhd/src/provider/core.rs:353 | `plan_restore` |  |
| crates/ezsdr-radio-uhd/src/provider/core.rs:357 | `take_restore` |  |
| crates/ezsdr-radio-uhd/src/provider/core.rs:367 | `report_debt` |  |
| crates/ezsdr-radio-uhd/src/provider/core.rs:530 | `is_idle` |  |
| crates/ezsdr-radio-uhd/src/provider/core.rs:533 | `active` |  |
| crates/ezsdr-radio-uhd/src/provider/core.rs:640 | `claim_release` | UR-25, RM-34: takes an idle, active-less `stream` for its owner's close of a |
| crates/ezsdr-radio-uhd/src/provider/core.rs:654 | `claim_from_none` | UR-25: takes `stream` for uhd-control's enable from no stream. A report read in flight |
| crates/ezsdr-radio-uhd/src/provider/core.rs:665 | `may_call_from_none` | UR-25: the from-none enable holds the stream and no report read is in flight, so its |
| crates/ezsdr-radio-uhd/src/provider/core.rs:725 | `starting` | UR-25: `start` gives the stream `held` (it has a segment from T0, or a clock) or |
| crates/ezsdr-radio-uhd/src/provider/core.rs:730 | `search` | UR-25: uhd-rx's search for the next planned segment found one or none. |
| crates/ezsdr-radio-uhd/src/provider/core.rs:758 | `plan_read` | UR-25: uhd-tx reads its plan: `clock` is its current clock, `switch` a change it has |
| crates/ezsdr-radio-uhd/src/provider/core.rs:769 | `phase` | The stream's phase, as the transitions left it (UR-25). |
| crates/ezsdr-radio-uhd/src/provider/core.rs:914 | `held_elsewhere` | Another stream's recorded streamer owns any channel in `set` (D2, D3). |
| crates/ezsdr-radio-uhd/src/provider/core.rs:925 | `needed_before` | An earlier, unprocessed segment owns the channel before `origin` (A2's finite chain). |
| crates/ezsdr-radio-uhd/src/provider/core.rs:943 | `move_source_busy` | A move destination waits only for its source's configuration of that same move. |
| crates/ezsdr-radio-uhd/src/provider/core.rs:949 | `complete` | Records a segment its owner finished carrying out. |
| crates/ezsdr-radio-uhd/src/provider/core.rs:1424 | `tx_dsp_waiters` |  |
| crates/ezsdr-radio-uhd/src/provider/core.rs:1663 | `release_unneeded` | Releases an idle stream's streamer only when another line still has an unprocessed |
| crates/ezsdr-radio-uhd/src/provider/core.rs:1697 | `await_release` | Waits for another stream's recorded streamer to release a target segment's channels. |
| crates/ezsdr-radio-uhd/src/provider/rx.rs:337 | `cursor` | The earliest origin of a planned segment not yet begun, as a test reads it (UR-25): an |
| crates/ezsdr-radio-uhd/src/provider/test_support.rs:118 | `rig_on_description_witness` | `rig_on_description` through a sink that witnesses which events are emitted while the line lock |
| crates/ezsdr-radio-uhd/src/provider/test_support.rs:128 | `rig_on_description_links_witness` | `rig_on_description_links` through a sink that witnesses control-path events. |
| crates/ezsdr-radio-uhd/src/provider/test_support.rs:389 | `rx_apply_pending` |  |
| crates/ezsdr-radio-uhd/src/provider/test_support.rs:398 | `dsp_pending` |  |
| crates/ezsdr-radio-uhd/src/provider/test_support.rs:402 | `dsp_log` |  |
| crates/ezsdr-radio-uhd/src/provider/test_support.rs:415 | `host_held` | Whether the line lock was held at each host clock read since `watch`, in order (UR-25). |
| crates/ezsdr-radio-uhd/src/provider/test_support.rs:437 | `refuse_rx_apply` |  |
| crates/ezsdr-radio-uhd/src/provider/test_support.rs:441 | `refuse_rx_start` |  |
| crates/ezsdr-radio-uhd/src/provider/test_support.rs:631 | `timed_lost` | The first timed `apply` of `dir` — of `chan`, when one is named — fails as a lost device. |
| crates/ezsdr-radio-uhd/src/provider/test_support.rs:641 | `attempts` | Every `apply` the wrapper was given, in order, as the test sees an attempt: a refusal |
| crates/ezsdr-radio-uhd/src/provider/test_support.rs:1000 | `events` | The events emitted since the last call, each with whether the line lock was held. |
| crates/ezsdr-radio-uhd/src/provider/test_support.rs:1005 | `observe` | Records the event's kind and whether the line lock is held now. |
| crates/ezsdr-radio-uhd/src/provider/test_support.rs:1012 | `resolve` |  |
| crates/ezsdr-radio-uhd/src/provider/test_support.rs:1016 | `emit` |  |
| crates/ezsdr-radio-uhd/src/provider/test_support.rs:1020 | `emit_control` |  |
| crates/ezsdr-radio-uhd/src/provider/tx.rs:193 | `next_segment_adopted` |  |
| crates/ezsdr-radio-uhd/src/provider/tx.rs:349 | `read` | Reads the transmit plan it shares with uhd-control again, when it has changed (UR-25), |
| crates/ezsdr-radio-uhd/src/provider/tx.rs:388 | `adopt_later` | UR-25: at a switch, the bursts booked on the new clock are held; those on a clock still |
| crates/ezsdr-radio-uhd/src/provider/tx.rs:1116 | `do_switch` | The transmit side of a `cold` change, at the old clock's cut (UR-25): the open burst |
| crates/ezsdr-radio-uhd/tests/common/probe_device.rs:135 | `matching_timed_apply` |  |
| crates/ezsdr-radio-uhd/tests/fake.rs:1703 | `build_on` | `build_with` on a device the test already has, which the Provider sees through |
| crates/ezsdr-radio-uhd/tests/fake.rs:1873 | `attempts` |  |
| crates/ezsdr-radio-uhd/tests/fake.rs:2400 | `hash_of` | A waveform's content hash, for the inputs map of a Spec that names it twice. |
| crates/ezsdr-radio-uhd/tests/fake.rs:2427 | `rejection_reasons` | Every `COMMAND_REJECTED` reason in `events`, in order (RM-22); one per event, so a count |
| crates/ezsdr-radio-uhd/tests/fake.rs:2686 | `late_commands` |  |
| crates/ezsdr-radio-uhd/tests/fake.rs:3676 | `captures_across_cold_changes` | Captures across `changes` `cold` receive rate changes in one Session on `config`, |
| crates/ezsdr-radio-uhd/tests/fake.rs:4209 | `applies` |  |
| crates/ezsdr-radio-uhd/tests/fake.rs:5934 | `mixed_selector` | One UBX and one BasicRX board: three receive channels, two streams, and two RX links. |
| crates/ezsdr-radio-uhd/tests/fake.rs:5938 | `fake_mixed` |  |
| crates/ezsdr-radio-uhd/tests/fake.rs:7401 | `rx_three` | Three receive channels: c0 and c1 in s0 at 2 Msps, c2 in s1 at 1 Msps. |
| crates/ezsdr-radio-uhd/tests/fake.rs:8800 | `leads_and_the_overflow_gap_add_a_condition_with_the_other_direction_running` |  |
| crates/ezsdr-radio-uhd/tests/fake.rs:9328 | `assert_h4_accounting` | The H4 result's update counts add up, whatever the scheduling did: every requested update is admitted, rejected, a submit error or not submitted, |
| crates/ezsdr-radio-uhd/tests/fake.rs:9368 | `h4_probe_records_a_timed_apply_under_its_instant_with_its_host_entry` |  |
| crates/ezsdr-radio-uhd/tests/fake.rs:9628 | `transmit_with_update` | A transmit Spec: a 60 000-sample burst at 40 ms, a transmit frequency update at `update_us`, |
| crates/ezsdr-radio/src/channels.rs:801 | `update_class` | RM-27's update class of the layout configuration key `key`, read from the one |
| crates/ezsdr-radio/src/timeline.rs:197 | `covering` | RM-36: the segment of `channel` whose samples pass at root tick `t`: it has begun |
| crates/ezsdr-radio/src/timeline.rs:288 | `released` | RM-34: the instant this line releases `channel` for a move of it whose item is the |
| crates/ezsdr-radio/src/timeline.rs:309 | `began_before` | RM-34: whether the segment a booking began takes effect at or before the instant and |
| crates/ezsdr-radio/src/timeline.rs:321 | `set_release` | RM-34: the booking `seq`'s release instant becomes `release` and the line is planned |
| crates/ezsdr-radio/src/timeline.rs:378 | `effective` | The instant a command takes effect at (RM-25, VH-2): a `cold` change never before a |
| crates/ezsdr-radio/src/timeline.rs:409 | `layout_instant` | RM-25 (design 33 D5): when a layout instance's `cold` change received at `now` with the |
| crates/ezsdr-radio/src/timeline.rs:423 | `timed_instant` | RM-21 (design 38 D2): when a `hardware_timed` update received at `now` with the requested |
| crates/ezsdr-radio/src/timeline.rs:570 | `book_move` | RM-34: books `channel` leaving `from`'s line for `to`'s, as one `Cold` item on both |
| crates/ezsdr-radio/src/timeline.rs:618 | `refuse_move` | RM-34: refuses the move booked as `seq` on both lines and plans them again (RM-25's |
| crates/ezsdr-radio/src/timeline.rs:630 | `move_lines` | The streams recorded by a move's `seq`, if it names one (RM-34). |
| crates/ezsdr-radio/src/timeline.rs:641 | `config_at` | RM-34: the configuration `stream`'s line leaves in effect at `e`, the state its |
| crates/ezsdr-radio/src/timeline.rs:652 | `floor` | RM-34: the release floor `stream`'s line leaves, from a pruning that folded away a move |
| crates/ezsdr-radio/src/timeline.rs:657 | `raise_floor` | RM-34: raises `stream`'s release floor to `cut`. |
| crates/ezsdr-radio/src/timeline.rs:665 | `request_prune` | RM-34: remembers the greatest prune instant requested for one stream. |
| crates/ezsdr-radio/src/timeline.rs:673 | `prune_before` | The latest per-stream prune request, if any. |
| crates/ezsdr-radio/src/timeline.rs:679 | `prune_requested` | RM-34: completes requested pruning without folding one side of an unfinished move. The |
| crates/ezsdr-radio/src/timeline.rs:735 | `settle` | RM-34: after any operation, every item of every destination line at or after a move |
| crates/ezsdr-radio/src/timeline.rs:747 | `settle_once` | RM-34: one round over every line's items; whether any item's release changed. Each |
| crates/ezsdr-radio/tests/timeline.rs:42 | `rx_bare` | A receive stream of `numbers` at `rate` from 0 with no lead: the source of the 1 Hz cases, |
| crates/ezsdr-radio/tests/timeline.rs:72 | `lines_of` | One instance's lines: `from` in stream `FROM` and `to` in `TO`, each planned. |
| crates/ezsdr-radio/tests/timeline.rs:80 | `planned_at` | One of an instance's streams, planned like [`planned`]. |
| crates/ezsdr-radio/tests/timeline.rs:85 | `lanes` | One of an instance's streams, planned (RM-25), for an assertion's message. |
| crates/ezsdr-radio/tests/timeline.rs:90 | `line_halted` | RM-25: whether one of an instance's streams is halted at `t`. |
| crates/ezsdr-radio/tests/timeline.rs:95 | `by` | The segment an instance's stream planned for the booking `seq` (RM-25's `by`). |
| crates/ezsdr-radio/tests/timeline.rs:101 | `released` | RM-34: the instant one of an instance's streams releases `channel` for its move `seq`. |
| crates/ezsdr-radio/tests/timeline.rs:442 | `rm_26_pruning_folds_a_cold_change_floored_at_an_earlier_one` | #63: pruning folds items in the order of their effective instants: a `cold` change received |
| crates/ezsdr-radio/tests/timeline.rs:487 | `rm_21_a_cold_change_to_another_set_of_the_same_size_starts_a_segment` | RM-21, RM-34: a stream's configuration is its set of channels, so a `cold` change to |
| crates/ezsdr-radio/tests/timeline.rs:506 | `rm_21_a_stream_that_loses_its_last_channel_starts_again_when_one_enters` | RM-21: the set, not the count, says whether a stream runs. Losing the last channel ends |
| crates/ezsdr-radio/tests/timeline.rs:525 | `rm_34_a_move_books_both_lines_at_one_instant` | RM-34: one channel moves from one stream to another, both lines booking a `cold` change |
| crates/ezsdr-radio/tests/timeline.rs:545 | `rm_34_a_move_is_floored_at_a_cold_change_of_either_line` | RM-34: a `cold` change either line received first floors the move's one instant, so the |
| crates/ezsdr-radio/tests/timeline.rs:568 | `rm_34_a_later_cold_change_is_floored_at_the_move` | RM-34: a `cold` change received after the move takes effect at or after the move's |
| crates/ezsdr-radio/tests/timeline.rs:591 | `rm_34_the_destination_waits_the_source_s_cut_and_its_own_lead` | RM-34, RM-16: the destination's new segment waits a lead after the instant of the |
| crates/ezsdr-radio/tests/timeline.rs:614 | `rm_34_a_refused_move_halts_both_lines_until_a_change_or_a_start` | RM-34, RM-25: a refused move halts both lines and leaves the moved channel's set as each |
| crates/ezsdr-radio/tests/timeline.rs:645 | `rm_34_a_move_into_a_stopped_stream_does_not_start_it` | RM-34, RM-21: a `Stop`ped receive stream does not resume because a channel moved into it; |
| crates/ezsdr-radio/tests/timeline.rs:658 | `rm_34_a_move_is_floored_at_a_receive_line_s_start` | RM-34, RM-25: a move asked for before a receive line's start takes effect at that start, |
| crates/ezsdr-radio/tests/timeline.rs:674 | `rm_34_a_move_into_a_stream_with_no_channel_starts_it` | RM-34, RM-25, VH-4: a stream with no channel has no segment, and the first channel |
| crates/ezsdr-radio/tests/timeline.rs:686 | `rm_34_a_move_whose_booking_fails_leaves_both_lines` | RM-34: when either line's booking fails neither line is changed, including the case |
| crates/ezsdr-radio/tests/timeline.rs:699 | `rm_34_a_move_reads_the_configuration_a_pruned_line_keeps` | RM-34, #63: a line that pruned its items keeps the configuration they left, and a move |
| crates/ezsdr-radio/tests/timeline.rs:716 | `rm_34_a_move_waits_a_source_segment_that_starts_after_its_instant` | RM-34, RM-16: the source's release is the cut the move makes on the plan as it stands, and |
| crates/ezsdr-radio/tests/timeline.rs:732 | `rm_34_a_later_booking_moves_the_destination_with_the_source` | RM-34: a move's record is not a snapshot of its booking: a later booking that changes the |
| crates/ezsdr-radio/tests/timeline.rs:756 | `rm_34_a_move_floors_a_later_start_of_the_destination` | RM-34, VH-4: the release floors every item of the destination at or after the move, not the |
| crates/ezsdr-radio/tests/timeline.rs:783 | `rm_34_a_move_keeps_a_transmit_clock_booked_before_an_end` | RM-34, VH-2: the release is held apart from the ready, so a transmit change booked before |
| crates/ezsdr-radio/tests/timeline.rs:809 | `rm_34_a_pruned_move_keeps_its_release_floor` | RM-34, #63: a pruning that folds a move's two items away keeps the source's cut as the |
| crates/ezsdr-radio/tests/timeline.rs:831 | `pruned_move_order` |  |
| crates/ezsdr-radio/tests/timeline.rs:883 | `rm_34_a_source_first_prune_keeps_move_dependencies_until_both_lines_pass` |  |
| crates/ezsdr-radio/tests/timeline.rs:890 | `rm_34_a_destination_first_prune_keeps_move_dependencies_until_both_lines_pass` |  |
| crates/ezsdr-radio/tests/timeline.rs:895 | `chained_move_prune_order` |  |
| crates/ezsdr-radio/tests/timeline.rs:929 | `rm_34_a_chain_prune_reaches_a_fixed_point_from_b_then_c` |  |
| crates/ezsdr-radio/tests/timeline.rs:936 | `rm_34_a_chain_prune_reaches_a_fixed_point_from_c_then_b` |  |
| crates/ezsdr-radio/tests/timeline.rs:943 | `rm_34_a_move_takes_the_greatest_cut_of_the_moves_into_a_line` | RM-34: the destination waits the greatest cut of the moves into it, not the last booked: a |
| crates/ezsdr-radio/tests/timeline.rs:971 | `rm_34_a_move_does_not_floor_an_item_received_before_it_at_its_instant` | RM-34, RM-25: at one instant the order is by arrival, so an item of the destination at the |
| crates/ezsdr-radio/tests/timeline.rs:990 | `rm_34_a_move_is_refused_on_both_lines_whichever_line_names_it` | RM-34, D3's4: refusing a move's `seq` on one of its lines refuses it on both — the call goes |
| crates/ezsdr-radio/tests/timeline.rs:1012 | `rm_34_a_move_without_a_timed_instant_takes_effect_at_its_receipt` | RM-34, RM-25, UC-2: `book_move` takes effect at `at`, or at its receipt when `at` is absent |
| crates/ezsdr-radio/tests/timeline.rs:1027 | `rm_34_a_pruned_move_takes_the_greatest_floor_of_its_line` | RM-34, #63: a pruning that folds several moves into one line keeps the greatest of their |
| crates/ezsdr-radio/tests/timeline.rs:1053 | `rm_34_a_moves_dependence_propagates_along_a_chain` | RM-34: the moves' dependence is held over the lines, not once at each booking: a booking |
| crates/ezsdr-radio/tests/timeline.rs:1082 | `rm_34_a_move_resumes_a_halted_stream` | RM-34, RM-21: a halted stream resumes on a successful move, as on any `cold` change, and |
| crates/ezsdr-radio/tests/timeline.rs:1100 | `rm_34_a_refused_source_change_does_not_move_the_destination` | RM-34, RM-25: the dependence is held after a refusal too: refusing the change the source's |
| crates/ezsdr-radio/tests/timeline.rs:1119 | `rm_34_the_destination_waits_a_stopped_source_s_cut` | RM-34: the source may release the moved channel before the move's item takes effect — a |
| crates/ezsdr-radio/tests/timeline.rs:1133 | `rm_34_the_destination_waits_an_ended_source_s_cut` | RM-34: the same on the transmit side, with a loss: its clock runs to the move's instant and |
| crates/ezsdr-radio/tests/timeline.rs:1147 | `rm_34_a_pruned_stopped_source_keeps_its_release` | RM-34, #63: a pruning between the move and the instant it releases the channel keeps that |
| crates/ezsdr-radio/tests/timeline.rs:1169 | `rm_34_a_move_does_not_wait_a_segment_of_another_channel` | RM-34: only the segments that hold the moved channel say when the source releases it. A |
| crates/ezsdr-radio/tests/timeline.rs:1186 | `rm_34_a_move_reads_the_source_s_plan_not_its_arrival_order` | RM-34: the release is the source's plan, not the arrival order of its items. A `start_rx` |
| crates/ezsdr-radio/tests/timeline.rs:1209 | `rm_34_a_move_reads_a_transmit_clock_the_loss_left_running` | RM-34, VH-2: a loss leaves a transmit clock running, so a move of its channel booked after |
| crates/ezsdr-radio/tests/timeline.rs:1223 | `rm_34_a_move_waits_a_segment_the_plan_leaves_out` |  |
| crates/ezsdr-radio/tests/timeline.rs:1241 | `rm_34_a_move_does_not_wait_a_channel_the_source_gains_after_it` | RM-34: a channel the source gains again after the move does not floor the destination: the |
| crates/ezsdr-radio/tests/timeline.rs:1258 | `rm_34_a_move_waits_the_greatest_end_not_the_last_one_read` | RM-34: the release is the greatest end of the segments the source ran before the move, not |
| crates/ezsdr-radio/tests/timeline.rs:1277 | `rm_34_a_move_does_not_wait_an_omitted_segment_the_source_gains_after_it` | RM-34: a segment RM-25 leaves out of the plan that began after the move is no release |
| crates/ezsdr-radio/tests/timeline.rs:1293 | `rm_34_a_move_books_both_transmit_clocks` | RM-34, RM-25: a move between transmit streams books both lines as a `cold` change does, |
| crates/ezsdr-radio/tests/timeline.rs:1311 | `rm_34_an_impossible_move_is_refused_without_a_booking` | RM-34: a move the lines cannot make is refused and neither line is changed: a number that |
| crates/ezsdr-radio/tests/timeline.rs:1335 | `rm_34_config_at_reads_the_move_and_the_pruned_state` | RM-34, #63: `config_at` is the configuration a line leaves in effect at an instant, a |
| crates/ezsdr-radio/tests/timeline.rs:1359 | `rm_34_generated_moves_keep_both_lines_invariants` | VH-8 and RM-34: with a move attempted among a generated sequence's items, both lines' |
| crates/ezsdr-radio/tests/timeline.rs:1414 | `check_move` | RM-34 across two lines of one instance: every segment of the destination at or after the |
| crates/ezsdr-radio/tests/timeline.rs:1476 | `rm_25_the_layout_instant_is_floored_at_every_earlier_cold_change` |  |
| crates/ezsdr-radio/tests/timeline.rs:1498 | `rm_25_the_timed_instant_is_floored_at_every_booked_command_inside_the_window` |  |
| crates/ezsdr-radio/tests/timeline.rs:1535 | `rm_36_covering_boundaries` |  |
| crates/ezsdr-radio/tests/timeline.rs:1580 | `rm_25_a_transmit_change_after_a_refusal_begins_as_if_not_refused` | RM-25 (design 43): a transmit change booked after a refusal at or after the refused change's |
| crates/ezsdr-radio/tests/timeline.rs:1754 | `rm_25_a_refused_segment_runs_for_planning_until_the_next_change_cuts_it` | RM-25 (design 43), the exact values: S1, C1 refused, C2, and the rest of the refusal's |
| crates/ezsdr-radio/tests/timeline.rs:1813 | `rm_34_a_refusal_does_not_change_a_move_release` | RM-25 (design 43), `Lines`: a refusal recorded after a move into the stream changes no |
| crates/ezsdr-radio/tests/timeline.rs:1867 | `rm_34_a_refusal_before_a_stop_or_in_a_chain_does_not_change_a_release` | RM-25, RM-34 (design 43), fixed cases: a later `Stop` between the refused segment and a booked move shortens the |
| crates/ezsdr-radio/tests/timeline.rs:2083 | `rm_34_a_refused_segment_is_a_release_only_for_a_channel_it_holds_and_that_began_before` | RM-34, RM-25 (design 43): a move after a refused move waits the end of the refused segments, |
| crates/ezsdr-radio/tests/timeline.rs:2115 | `rm_25_a_transmit_refusal_does_not_move_a_later_origin` | RM-25 (design 43), transmit, the same rule at the generated sequences: a change booked after a |
| crates/ezsdr-radio/tests/timeline.rs:2143 | `rm_25_a_transmit_refusal_followed_by_other_items_keeps_the_next_origin` | RM-25 (design 43), transmit: repeated refusals, and a refusal followed by an item that is no |
| crates/ezsdr-radio/tests/timeline.rs:2181 | `rm_34_a_transmit_move_after_a_refused_source_change_waits_the_cut_the_clock_would_have_had` | RM-25, RM-34 (design 43), transmit: a move of a channel of a refused clock's line waits the cut the |

Types, fields and enum variants removed with them (no row above): MockRadio `PendingUpdate`, `ColdChange`, `WorkKind::Update`, `MockRadio::updates`, `TxStream::later`, `REFUSED_CHANGE`, `REFUSED_BURST`, `HALTED_STREAM`, `channel::ChannelModel::timed_phase` and `RxChannelModel::timed_phase` (with their draws in `prepare`); the shared timeline's `Kind::Cold`, `Item::release`, `NO_RELEASE`, `MoveError`, the `Lines` moves, floors and per-stream prune requests, `Line::released`/`began_before`/`set_release`/`omitted`/`ended`, `State::refusal`/`refused_clock`/`omitted`, `timeline::covering`, `effective`, `layout_instant`, `timed_instant`, the `cold` and `seq` parameters of `command_instant`; UHD `Timed`, `Given`, `Reach`, `Handed`, `Control::{colds, held, released_at, rx_frequency, released, booked, t0, initial_config}`, `ColdConfig::{updates, configured, book, settings_on, with_clock, is_configured, updates_held, updates_waiters}`, `Planned::clock`, `Holder::{FromNone, Release}`, the `seq` of `Holder::Owner`, `RxEntry`, `Restore`, `RxDevice::Flushing`, `RxChain::{debt, left, latest, restore}`, `StreamState::completed`, `Phase` (replaced by `StreamState::delivered`), `Switch`, `Tx::{later, switch}` (replaced by `Tx::cut`), `TxChain::next_segment_adopted`, `channels::update_class`, `Description::stream_of`.

## 2. Rewritten items (a surviving subject, a new stimulus)

| path | test | what changed |
|---|---|---|
| ezsdr-mock-radio/src/lib.rs | `a_cancelled_held_burst_releases_the_shared_reservation (was cancelled_held_and_later_bursts_release_the_shared_reservations)` | Stop releases a held burst's shared reservation; the `later` burst and the second clock are gone |
| ezsdr-mock-radio/src/lib.rs | `a_loss_does_not_remember_a_stop_front (was cold_change_and_loss_do_not_remember_a_stop_front)` | a loss cuts at its instant; the ColdChange half is gone |
| ezsdr-mock-radio/src/channel.rs | `mr_33_a_timeline_keeps_the_value_in_force_at_each_instant (new)` | the per-channel value history M/src/channel.rs keeps for stages 3-4, which no test reached after part (a) |
| ezsdr-radio/tests/timeline.rs | `rm_26_each_stream_takes_its_own_directions_start_lead` | receive only: Stop then Start; a transmit clock has no restart |
| ezsdr-radio/tests/timeline.rs | `rm_25_a_receive_successor_adds_release_window_before_call_and_lead` | Stop + Start replace the Cold at the cut |
| ezsdr-radio/tests/timeline.rs | `rm_26_the_timeline_cases` | the cases without a Cold item are kept (Stop, Start, refused Start, fractional cuts, call term, delivered floor, abort, loss, loss before a transmit origin); the Cold, move, floor and refused-change cases are deleted |
| ezsdr-radio/tests/timeline.rs | `rm_26_a_pruned_line_keeps_few_items` | receive only; a Stop and a Start every 300 ms |
| ezsdr-radio/tests/timeline.rs | `rm_26_generated_sequences_keep_the_timeline_invariants` | runs both directions at 1 and 3 MS/s, so the fractional-ratio coverage the rate changes gave stays |
| ezsdr-radio/tests/timeline.rs | `rm_26_a_pruned_line_plans_as_one_that_keeps_every_item, rm_25_a_refusal_does_not_move_a_later_origin` | the refused item is a Start |
| ezsdr-radio/tests/timeline.rs | `rm_25_a_virtual_segment_ignores_a_stale_delivered_index, rm_25_a_refused_receive_segment_is_not_in_the_plan, rm_25_a_refused_segment_runs_for_planning_until_the_next_start_cuts_it, rm_25_a_start_after_a_refusal_begins_from_the_refused_segment_s_cut, rm_25_a_pruned_line_with_a_refused_segment_plans_as_one_that_keeps_every_item` | the refused item is a `start_rx` after a Stop; the numbers are re-derived (126 ms and 1 555 ms, a 1 MS/s call) |
| ezsdr-radio/tests/timeline.rs | `rm_25_a_booking_adds_no_coverage_before_its_start_lead` | one line; a test-local `covers` replaces `timeline::covering`; the move step is gone |
| ezsdr-radio/tests/timeline.rs | `rm_16_a_real_segment_keeps_its_delivered_floor` | a restart (Stop + Start) replaces the rate change; the delivered numbers are for 1 MS/s |
| ezsdr-radio/tests/timeline.rs | `mr_29_a_receive_link_floors_a_command_at_its_t0` | `command_instant` returns the instant |
| ezsdr-radio/tests/generator/mod.rs | `the VH generator` | Cold and Timed operations, `Change`, the rates, `Terms::timed_lead` and `release_window` removed; classes are Stop, StartRx, Fault, Loss in that order (a sequence holds its own class and any earlier one), so none generates an empty sequence. Loss keeps its old distribution (the Loss class only): a Fault class with a loss exposes an order difference of pending faults after a loss (MockRadio records them in booking order, the oracle by instant), which is outside this step |
| ezsdr-mock-radio/tests/timeline.rs | `rm_26_the_timeline_against_mockradio` | the Mock-side filter and the `Op::Cold|Timed` arm of `layout_sequence` removed |
| ezsdr-radio-uhd/src/provider/control/differential.rs | `rm_26_the_timeline_against_uhd_control` | uses `Sequence::items` directly (its own `layout_items` copy removed); no `release()` |
| ezsdr-radio-uhd/src/provider/rx/differential.rs | `rm_26_the_timeline_against_uhd_rx` | the refused rate is the stream's 1 MS/s whenever the sequence holds a `start_rx` (the first configure is refused once); `Kind::Cold` in the booking harness gone |
| ezsdr-radio-uhd/src/provider/control.rs | `ur_26_an_orderly_stop_cuts_at_its_instant` | `started` has no segment argument |
| ezsdr-radio-uhd/src/provider/control.rs | `ur_21_a_burst_before_its_clock_s_origin_is_late; ur_21_an_untimed_burst_before_the_clock_s_origin_is_on_time_at_the_origin (was ..._after_a_cold_change_...)` | a transmit clock whose origin is 50 ms ahead (`control_with_tx_origin`) replaces the clock a cold change registered |
| ezsdr-radio-uhd/src/provider/control.rs | `ur_25_an_open_is_recorded_only_after_the_device_returns` | the owner's open alone (no from-none open first) |
| ezsdr-radio-uhd/src/provider/core.rs | `rx_chain_adopt_start_transitions, rx_chain_begin_transitions, rx_chain_finish_transitions, rx_chain_lose_is_terminal, rx_chain_abandon_unowned_records_cleanup_state, rx_chain_abandon_unowned_preserves_loss` | without the debt, left, latest and restore asserts; `Flushing` is gone from the lose cases |
| ezsdr-radio-uhd/src/provider/core.rs | `rx_chain_reuse_guard_rejects_open_and_close_of_an_active_chain (was ..._active_open_and_debt_close)` | an Armed chain cannot be opened or closed; the debt half is gone |
| ezsdr-radio-uhd/src/provider/core.rs | `ur_25_a_claim_owner_takes_only_a_free_stream, ur_25_a_begin_read_needs_a_free_stream_and_no_read, ur_25_a_begin_shutdown_read_ignores_the_holder, ur_25_a_claim_dropped_without_give_back_panics_in_the_test_build, ur_26_a_finished_stream_clears_only_its_own_floor, ur_26_an_end_takes_delivered_from_each_segment_s_own_stream` | the from-none and release holders, `search` and the owner phase are gone; the claim and read exclusion, the delivered floor per stream stay |
| ezsdr-radio-uhd/src/provider/mod.rs | `rx_chain_prepare_open_requires_an_idle_chain, rx_chain_cleanup_close_requires_an_idle_chain, ur_15_a_failed_rx_owner_spawn_abandons_its_started_chain_before_cleanup` | the debt halves are gone |
| ezsdr-radio-uhd/src/provider/rx.rs | `ur_24_an_abort_keeps_overflow_and_later_samples_off_the_link (was ur_24_multichannel_abort_keeps_overflow_and_drain_samples_off_the_link)` | no handed DDC entries; abort ends the stream at once |
| ezsdr-radio-uhd/src/provider/rx.rs | `ur_24_a_cut_receive_uses_one_short_packet_and_preserves_successor_lead (was ur_24_a_deadline_receive_...)` | the S7 keep, trimmed: the drain deadline and the DDC entry are gone; the one short packet and the successor lead stay |
| ezsdr-radio-uhd/src/provider/rx.rs | `ur_24_nonloss_rx_stop_failure_leaves_the_chain_uncertain (was ..._reports_pending_and_skips_restore)` | a refused stop leaves the chain Uncertain and starts no recovery |
| ezsdr-radio-uhd/src/provider/rx.rs | `ur_24_a_loss_during_rx_receive_finishes_without_recovery_calls, ur_24_a_loss_published_before_rx_recv_skips_the_device_call, rx_chain_abandon_unowned_records_cleanup_state` | loss subjects: the DDC entry and the Control timed apply are gone; the loss is published directly |
| ezsdr-radio-uhd/src/provider/rx.rs | `ur_26_a_receive_start_queued_by_the_stop_ends_its_owner_without_detaching, ur_26_a_stream_s_delivered_is_cleared_alone, ur_26_a_receive_start_of_no_planned_segment, ur_29_a_loss_after_a_configure_starts_no_receive_segment, ur_29_a_begin_rechecks_loss_after_acquiring_the_chain_before_rx_start` | `first_undelivered` replaces the phase asserts; a Stop and a Start give the planned segment the Cold item gave |
| ezsdr-radio-uhd/src/provider/rx.rs | `ur_29_a_loss_ends_the_stream_at_its_instant, ur_29_a_loss_found_after_uhd_rx_read_the_plan_ends_the_stream, ur_29_a_silent_stream_ends_at_a_cut_booked_during_its_wait, ur_29_a_loss_takes_effect_before_a_change_at_its_instant, ur_26_an_end_takes_delivered_only_from_the_segment_it_cuts, ur_26_no_segment_begins_once_provider_stop_has_begun` | the "later cut" is a Stop at 10 ms (and a Start at 11 ms where a successor matters) |
| ezsdr-radio-uhd/src/provider/rx.rs | `ur_13_a_failed_open_by_the_owner_is_opened_again, ur_13_a_partly_applied_set_is_configured_whole_without_reopen, ur_13_a_failed_configure_is_configured_again_without_reopen, ur_25_a_refused_segment_is_booked_by_uhd_rx, ur_25_a_refused_receive_owner_books_and_gives_back_in_one_line_section, ur_17_an_ended_segment_prunes_the_items_before_it` | Stop, Start (refused), Start replace Cold, Start; the refused rate is 1 MS/s; numbers re-derived |
| ezsdr-radio-uhd/src/provider/tx.rs | `tx_chain_recovery_transitions` | without `next_segment_adopted` |
| ezsdr-radio-uhd/src/provider/tx.rs | `ur_22_uncertain_reopen_waits_for_owner_claim_and_stops_on_stop_or_loss, ur_25_a_detached_control_does_not_hold_the_shutdown_s_read` | another holder is an owner claim |
| ezsdr-radio-uhd/src/provider/tx.rs | `ur_22_nonterminal_reopen_failures_cancel_held_initial_bursts` | `plan_section` replaces `read`; no phase assert |
| ezsdr-radio-uhd/src/provider/tx.rs | `ur_29_eob_loss_at_the_cut_stops_device_calls (was ..._during_switch_...), ur_25_largest_aligned_cut_is_safe_in_tx_owner, ur_23_txs_cut_and_stop_do_not_send_past_a_high_rate_frontier, ur_23_m14_a_cut_is_not_waited_past_or_crossed, ur_25_the_line_lock_is_free_inside_every_transmit_device_call` | `tx.cut` and `end_clock` replace `tx.switch` and `do_switch` |
| ezsdr-radio-uhd/src/provider/tx.rs | `ur_28_a_streamer_the_record_holds_keeps_its_correlations, ur_28_delayed_time_error_uses_its_original_clock` | the owner's reopen (a new open record) and the clock's end replace the clock switch; generation/report correlation is pinned |
| ezsdr-radio-uhd/src/provider/tx.rs | `ur_26_stop_cancels_the_held_burst_record_only, ur_23_a_clock_ending_at_its_cut_cancels_a_held_burst_and_releases_its_reservation, ur_25_a_burst_on_a_clock_that_never_started_is_cancelled_and_releases_its_reservation, ur_26_shutdown_after_loss_cancels_held_bursts_record_only, ur_25_uhd_tx_reads_the_plan_again_when_it_changes` | no `later` bursts; the end booked before a clock's origin is the plan change |
| ezsdr-radio-uhd/tests/fake.rs | `ur_26_a_stopped_later_stream_waits_for_start_rx` | the cold-change half is gone (rev 4 m3); Stop then `start_rx` stays |

## 3. Deleted tests (name, subject)

The two tests rev 4 handed to part (b): `fake.rs` `ur_29_a_stream_silent_before_a_far_cut_is_a_lost_device` is deleted (its far cut had a Cold form only; `ur_29_a_silent_stream_is_a_lost_device` pins the silent stream); `ur_26_a_stopped_later_stream_waits_for_start_rx` is rewritten (section 2). The refusal-halt pins that survive are `ur_25_a_refused_start_of_a_later_stream_halts_that_stream_alone` and `ur_25_a_successful_start_clears_a_later_stream_s_halt` (`fake.rs`), so the in-source tests that reached a refusal through a Cold item are deleted with it (peer isolation, claim return in one section: `ur_25_a_refused_receive_owner_books_and_gives_back_in_one_line_section` stays).

| path:line (HEAD) | test | subject |
|---|---|---|
| crates/ezsdr-mock-radio/src/lib.rs:2997 | `a_cold_change_skipping_a_future_clock_releases_its_reservation` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:1446 | `ur_24_folded_rx_frequency_history_keeps_only_the_latest_per_channel` | timed receive frequency hand-over |
| crates/ezsdr-radio-uhd/src/provider/control.rs:1481 | `ur_13_a_failed_open_from_no_stream_is_opened_again_by_the_owner` | timed gain update |
| crates/ezsdr-radio-uhd/src/provider/control.rs:1506 | `ur_13_a_rate_refusal_from_no_stream_is_reapplied_and_starts_nothing` | refusal reached through a cold item |
| crates/ezsdr-radio-uhd/src/provider/control.rs:1522 | `ur_25_a_transmit_refusal_from_no_stream_starts_no_clock` | refusal reached through a cold item |
| crates/ezsdr-radio-uhd/src/provider/control.rs:1566 | `ur_22_from_none_open_waits_for_t3_flush_ack_before_timed_tx_dsp` | from-none claim, enable and join |
| crates/ezsdr-radio-uhd/src/provider/control.rs:1581 | `ur_22_timed_control_give_waits_for_t3_flush_ack` | hardware_timed booking, queue and release |
| crates/ezsdr-radio-uhd/src/provider/control.rs:1629 | `ur_24_a_timed_update_after_a_from_none_configure_is_sent_without_configuring_again` | from-none claim, enable and join |
| crates/ezsdr-radio-uhd/src/provider/control.rs:1699 | `ur_34_a_from_none_join_yields_to_an_earlier_unprocessed_segment` | from-none claim, enable and join |
| crates/ezsdr-radio-uhd/src/provider/control.rs:1815 | `ur_34_a_control_books_the_next_action_while_an_owner_close_is_gated` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/control.rs:1857 | `ur_34_a_move_books_one_item_on_both_lines_and_both_configurations` | RM-34 move / membership propagation |
| crates/ezsdr-radio-uhd/src/provider/control.rs:1891 | `ur_34_a_move_to_a_stream_without_a_line_uses_the_mock_reason` | RM-34 move / membership propagation |
| crates/ezsdr-radio-uhd/src/provider/control.rs:1909 | `ur_34_a_second_move_and_a_move_after_refusal_and_prune_keep_projection_aligned` | RM-34 move / membership propagation |
| crates/ezsdr-radio-uhd/src/provider/control.rs:1952 | `ur_24_the_command_queue_is_counted_per_instance_across_streams` | hardware_timed command queue |
| crates/ezsdr-radio-uhd/src/provider/control.rs:1990 | `ur_24_a_timed_update_reaches_a_later_stream_s_pending_segment` | hardware_timed booking, queue and release |
| crates/ezsdr-radio-uhd/src/provider/control.rs:2022 | `ur_24_a_release_without_a_live_channel_reaches_the_pending_segment` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:2061 | `ur_24_a_cancelled_held_update_leaves_the_projection` | segment configuration projection of updates |
| crates/ezsdr-radio-uhd/src/provider/control.rs:2083 | `ur_24_a_command_waiting_for_a_channel_is_given_before_a_later_one` | burst booked on a later clock |
| crates/ezsdr-radio-uhd/src/provider/control.rs:2122 | `ur_24_a_command_waiting_for_a_channel_keeps_a_later_booking_behind_it` | burst booked on a later clock |
| crates/ezsdr-radio-uhd/src/provider/control.rs:2158 | `ur_24_a_command_waiting_for_a_channel_holds_back_the_one_behind_it` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/control.rs:2194 | `ur_24_a_stop_cancels_a_command_waiting_for_a_channel` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/control.rs:2226 | `ur_17_a_prune_keeps_the_device_record_and_the_configure_mark` | move-aware prune |
| crates/ezsdr-radio-uhd/src/provider/control.rs:2304 | `ur_13_a_from_none_segment_the_owner_never_started_is_opened_again` | from-none claim, enable and join |
| crates/ezsdr-radio-uhd/src/provider/control.rs:2332 | `ur_25_an_owner_begins_a_from_none_enable_without_a_second_open` | from-none claim, enable and join |
| crates/ezsdr-radio-uhd/src/provider/control.rs:2347 | `rx_chain_control_open_requires_idle_empty_debt` | RX DDC debt |
| crates/ezsdr-radio-uhd/src/provider/control.rs:2358 | `rx_chain_rate_check_failure_close_requires_idle_empty_debt` | RX DDC debt |
| crates/ezsdr-radio-uhd/src/provider/control.rs:2375 | `ur_25_a_refused_receive_enable_is_booked_and_the_next_enable_starts` | enable / disable of a stream |
| crates/ezsdr-radio-uhd/src/provider/control.rs:2407 | `ur_14_booking_does_not_wait_for_a_future_cold_switch` | Run-time cold change |
| crates/ezsdr-radio-uhd/src/provider/control.rs:2692 | `ur_21_an_untimed_burst_after_a_cold_change_is_on_time_at_its_clock_s_origin` | hardware_timed booking, queue and release |
| crates/ezsdr-radio-uhd/src/provider/control.rs:2733 | `ur_30_the_recorded_plan_has_no_refused_segment` | refusal reached through a cold item |
| crates/ezsdr-radio-uhd/src/provider/control.rs:2763 | `ur_25_a_cold_change_that_arrives_after_a_later_one_follows_it` | Run-time cold change |
| crates/ezsdr-radio-uhd/src/provider/control.rs:2800 | `ur_26_a_change_booked_after_the_stop_instant_ends_its_clock` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/control.rs:2815 | `ur_25_later_timed_update_survives_cold_switch` | hardware_timed booking, queue and release |
| crates/ezsdr-radio-uhd/src/provider/control.rs:2840 | `ur_25_switch_projects_timed_updates_in_effective_order` | hardware_timed booking, queue and release |
| crates/ezsdr-radio-uhd/src/provider/control.rs:2864 | `ur_26_a_stop_cancels_no_update` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/control.rs:2889 | `ur_25_cold_deadline_overflow_is_rejected_without_panic` | timed hand-over deadline |
| crates/ezsdr-radio-uhd/src/provider/control.rs:2923 | `ur_24_invalid_explicit_deadline_is_not_asap` | timed hand-over deadline |
| crates/ezsdr-radio-uhd/src/provider/control.rs:2959 | `ur_24_valid_deadlines_and_omission_preserve_their_meaning` | timed hand-over deadline |
| crates/ezsdr-radio-uhd/src/provider/control.rs:2978 | `ur_24_a_timed_update_acts_on_the_configuration_in_force_at_its_instant` | hardware_timed booking, queue and release |
| crates/ezsdr-radio-uhd/src/provider/control.rs:3000 | `ur_24_a_direction_halted_by_a_refusal_has_no_channel` | refusal reached through a cold item |
| crates/ezsdr-radio-uhd/src/provider/control.rs:3059 | `ur_24_the_device_queue_order_is_the_effective_order` | hardware_timed command queue |
| crates/ezsdr-radio-uhd/src/provider/control.rs:3086 | `ur_24_hardware_timed_updates` | hardware_timed booking, queue and release |
| crates/ezsdr-radio-uhd/src/provider/control.rs:3130 | `ur_24_a_far_future_update_does_not_delay_a_nearer_one` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/control.rs:3169 | `ur_25_rx_channels_from_zero_starts_at_its_instant` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/control.rs:3197 | `ur_25_a_timed_update_at_the_cold_change_s_instant_reaches_the_switch` | hardware_timed booking, queue and release |
| crates/ezsdr-radio-uhd/src/provider/control.rs:3245 | `ur_24_a_late_update_halted_before_it_can_act_has_no_channel` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/control.rs:3273 | `ur_25_a_receive_enable_without_a_link_is_only_recorded` | enable / disable of a stream |
| crates/ezsdr-radio-uhd/src/provider/control.rs:3318 | `ur_24_a_refusal_halts_only_its_own_stream` | refusal reached through a cold item |
| crates/ezsdr-radio-uhd/src/provider/control.rs:3364 | `ur_25_a_cold_change_and_a_timed_update_at_one_instant_keep_their_own_instants` | hardware_timed booking, queue and release |
| crates/ezsdr-radio-uhd/src/provider/control.rs:3398 | `ur_24_the_device_queue_is_one_across_the_directions` | hardware_timed command queue |
| crates/ezsdr-radio-uhd/src/provider/control.rs:3427 | `ur_25_the_line_lock_is_free_inside_every_control_device_call` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/control.rs:3463 | `ur_25_the_cold_booking_books_in_the_section_that_takes_its_instant` | Run-time cold change |
| crates/ezsdr-radio-uhd/src/provider/control.rs:3498 | `ur_25_the_layout_cold_change_is_built_in_its_booking_section` | Run-time cold change |
| crates/ezsdr-radio-uhd/src/provider/control.rs:3525 | `ur_25_a_cold_change_takes_the_line_lock_once_for_its_instant_and_its_booking` | Run-time cold change |
| crates/ezsdr-radio-uhd/src/provider/control.rs:3586 | `ur_17_a_stream_s_prune_leaves_the_other_stream_alone` | move-aware prune |
| crates/ezsdr-radio-uhd/src/provider/control.rs:3688 | `ur_25_a_from_none_holds_the_streamer_from_its_decision_to_its_booking` | from-none claim, enable and join |
| crates/ezsdr-radio-uhd/src/provider/control.rs:3720 | `ur_25_an_enable_behind_an_owner_s_open_is_a_cold_change` | enable / disable of a stream |
| crates/ezsdr-radio-uhd/src/provider/control.rs:3754 | `ur_25_a_from_none_leaves_the_stream_unbusy_on_every_path` | from-none claim, enable and join |
| crates/ezsdr-radio-uhd/src/provider/control.rs:3791 | `ur_29_a_lost_device_stops_a_from_none_enable_between_channels` | from-none claim, enable and join |
| crates/ezsdr-radio-uhd/src/provider/control.rs:3823 | `ur_25_an_enable_from_no_stream_after_the_provider_s_stop_calls_no_device` | enable / disable of a stream |
| crates/ezsdr-radio-uhd/src/provider/control.rs:3867 | `ur_24_a_release_without_a_live_channel_precedes_the_owner_s_configure` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:3899 | `ur_24_a_release_during_a_configure_s_return_reaches_the_device` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:3948 | `ur_24_a_successor_frequency_after_its_origin_stays_timed_during_restart` | hardware_timed booking, queue and release |
| crates/ezsdr-radio-uhd/src/provider/control.rs:4010 | `ur_24_a_stopped_rx_segment_does_not_receive_a_late_timed_frequency` | hardware_timed booking, queue and release |
| crates/ezsdr-radio-uhd/src/provider/control.rs:4054 | `ur_24_a_release_without_a_live_channel_takes_the_command_queue_slots` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:4088 | `ur_24_a_release_after_its_instant_applies_on_arrival` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:4108 | `ur_24_a_release_without_a_live_channel_or_a_later_segment_applies_nothing` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:4128 | `ur_24_a_released_command_and_one_without_a_channel_settle_in_instant_order` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:4162 | `ur_24_a_layout_release_reaches_the_later_segment_s_channel` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:4185 | `ur_24_a_release_for_a_refused_transmit_segment_applies_nothing` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:4210 | `ur_24_a_release_for_a_refused_receive_change_has_no_candidate` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:4234 | `ur_24_a_release_stays_when_its_segment_is_refused_after_it` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:4260 | `ur_29_a_lost_device_applies_no_released_update` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:4277 | `ur_29_a_loss_published_before_a_release_stops_the_call` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:4294 | `ur_29_a_lost_timed_release_publishes_device_loss` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:4318 | `ur_29_a_failed_released_update_is_a_device_failure` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:4346 | `ur_24_refused_timed_rx_frequency_leaves_no_drain_obligation` | hardware_timed booking, queue and release |
| crates/ezsdr-radio-uhd/src/provider/control.rs:4365 | `ur_29_a_loss_after_a_release_s_command_stops_the_one_behind_it` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:4409 | `ur_24_a_refused_release_records_no_row_and_keeps_the_projection` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:4441 | `ur_24_a_release_behind_a_released_command_keeps_the_device_queue_order` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:4490 | `ur_24_a_command_waiting_for_a_channel_is_booked_at_the_instant_it_can_take_effect` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/control.rs:4538 | `ur_24_a_segment_booked_while_a_command_waits_for_a_channel_holds_it_at_its_instant` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/control.rs:4585 | `ur_24_a_segment_begun_before_a_late_command_s_instant_configures_the_old_value` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/control.rs:4613 | `ur_25_an_owner_configures_a_segment_the_record_s_command_is_not_for` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/control.rs:4641 | `ur_24_a_release_after_a_receive_channel_leaves_keeps_the_setting_until_its_instant` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:4668 | `ur_24_a_release_after_a_transmit_channel_leaves_keeps_the_setting_until_its_instant` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:4696 | `ur_24_a_layout_release_after_a_channel_leaves_keeps_the_setting_until_its_instant` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:4734 | `ur_25_a_receive_enable_counts_its_start_lead_from_the_end_of_its_configuration` | enable / disable of a stream |
| crates/ezsdr-radio-uhd/src/provider/control.rs:4791 | `ur_25_the_switch_applies_the_configuration_in_effect_at_e2` | transmit clock switch of a cold change |
| crates/ezsdr-radio-uhd/src/provider/control.rs:4840 | `ur_25_an_enable_at_a_timed_update_s_instant_applies_it` | enable / disable of a stream |
| crates/ezsdr-radio-uhd/src/provider/control.rs:4897 | `ur_24_a_channel_keyed_timed_update_acts_on_its_own_channel` | hardware_timed booking, queue and release |
| crates/ezsdr-radio-uhd/src/provider/control.rs:4935 | `ur_24_a_channel_keyed_command_counts_one_queue_slot` | hardware_timed command queue |
| crates/ezsdr-radio-uhd/src/provider/control.rs:4957 | `ur_25_a_layout_cold_change_is_floored_at_every_earlier_cold_change_of_the_instance` | Run-time cold change |
| crates/ezsdr-radio-uhd/src/provider/control.rs:5007 | `ur_25_a_layout_cold_change_of_a_linkless_receive_side_is_floored_at_t0` | Run-time cold change |
| crates/ezsdr-radio-uhd/src/provider/control.rs:5034 | `ur_24_the_layout_destination_follows_the_effective_membership` | membership change |
| crates/ezsdr-radio-uhd/src/provider/control.rs:5118 | `ur_25_a_same_size_membership_swap_reopens_the_streamer` | membership change |
| crates/ezsdr-radio-uhd/src/provider/control.rs:5205 | `ur_34_a_timed_update_crossing_the_move_reaches_each_streamer_channel_at_its_instant` | RM-34 move / membership propagation |
| crates/ezsdr-radio-uhd/src/provider/control.rs:5289 | `ur_34_a_cold_change_before_the_move_s_release_is_floored_at_the_move` | RM-34 move / membership propagation |
| crates/ezsdr-radio-uhd/src/provider/control.rs:5315 | `ur_34_two_moves_at_one_instant_swap_their_receive_channels_without_deadlock` | RM-34 move / membership propagation |
| crates/ezsdr-radio-uhd/src/provider/control.rs:5351 | `ur_34_two_moves_at_one_instant_swap_their_transmit_channels_without_deadlock` | RM-34 move / membership propagation |
| crates/ezsdr-radio-uhd/src/provider/control.rs:5381 | `ur_34_a_finite_middle_segment_releases_a_channel_along_a_move_chain` | RM-34 move / membership propagation |
| crates/ezsdr-radio-uhd/src/provider/control.rs:5485 | `ur_25_a_floor_raised_layout_join_is_judged_at_its_instant` | cold-change floor |
| crates/ezsdr-radio-uhd/src/provider/control.rs:5526 | `ur_25_a_configuration_taken_under_the_line_lock_is_refused` | refusal reached through a cold item |
| crates/ezsdr-radio-uhd/src/provider/control.rs:5572 | `ur_24_a_booked_command_floors_a_later_one_inside_the_window` | burst booked on a later clock |
| crates/ezsdr-radio-uhd/src/provider/control.rs:5604 | `ur_24_an_omitted_deadline_takes_the_floor` | timed hand-over deadline |
| crates/ezsdr-radio-uhd/src/provider/control.rs:5624 | `ur_24_the_window_edge_excludes_one_tick` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/control.rs:5645 | `ur_24_a_command_beyond_the_window_keeps_its_instant` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/control.rs:5669 | `ur_24_a_command_waiting_for_a_channel_floors_a_later_booking` | burst booked on a later clock |
| crates/ezsdr-radio-uhd/src/provider/control.rs:5699 | `ur_24_a_refused_release_floors_the_next_booking` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:5734 | `ur_24_a_refused_release_holds_its_queue_slot_until_its_instant` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:5774 | `ur_24_a_channel_leaving_after_the_release_does_not_move_the_floor` | RM-34 move / membership propagation |
| crates/ezsdr-radio-uhd/src/provider/control.rs:5807 | `ur_26_a_stop_leaves_each_update_its_row_at_its_instant` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/control.rs:5831 | `ur_26_provider_stop_restores_the_frequency_in_force_at_its_instant` | timed receive frequency hand-over |
| crates/ezsdr-radio-uhd/src/provider/control.rs:5862 | `ur_26_provider_stop_records_a_given_and_a_held_update_alike` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/control.rs:5891 | `ur_26_an_applied_update_keeps_its_row_after_a_later_stop` | burst booked on a later clock |
| crates/ezsdr-radio-uhd/src/provider/control.rs:5912 | `ur_26_a_loss_settles_what_the_last_release_gave` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:5935 | `ur_26_a_stop_settles_by_its_instant_not_by_the_finish` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/control.rs:5957 | `ur_26_a_release_after_the_stop_was_published_leaves_the_row_to_the_end` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:5990 | `ur_26_a_command_given_after_the_stop_s_instant_is_cancelled_at_it` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/control.rs:6013 | `ur_26_a_stop_between_two_hand_overs_judges_each_at_its_apply` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/control.rs:6053 | `ur_26_a_refusal_behind_a_handed_command_still_settles_it_at_the_stop` | timed hand-over |
| crates/ezsdr-radio-uhd/src/provider/control.rs:6082 | `ur_26_a_lost_apply_behind_a_handed_command_settles_both_at_the_loss` | timed hand-over |
| crates/ezsdr-radio-uhd/src/provider/control.rs:6109 | `ur_26_an_instant_at_the_stop_applies_and_one_at_the_loss_does_not` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/control.rs:6137 | `ur_24_a_due_command_behind_a_channel_less_head_is_cancelled_at_the_loss` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/control.rs:6177 | `ur_24_two_updates_at_one_instant_write_their_rows_in_booking_order` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/control.rs:6199 | `ur_24_a_late_command_at_a_full_queue_reports_late_first` | hardware_timed command queue |
| crates/ezsdr-radio-uhd/src/provider/control.rs:6227 | `ur_24_the_row_is_written_at_its_instant` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/control.rs:6248 | `ur_24_the_release_window_comes_from_the_description` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:6289 | `measurement_candidate_directional_start_leads_set_cold_change_origins` | Run-time cold change |
| crates/ezsdr-radio-uhd/src/provider/control.rs:6316 | `ur_26_a_loss_then_a_stop_keeps_the_loss_s_instant_and_rows` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/control.rs:6345 | `ur_26_a_stop_then_a_loss_keeps_the_stop_s_instant_and_rows` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/control.rs:6369 | `ur_24_a_command_the_loss_stops_from_being_given_settles_at_the_end` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/control.rs:6394 | `ur_24_a_command_due_before_the_receipt_takes_no_slot` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/control.rs:6416 | `ur_24_a_command_due_at_its_receipt_takes_no_slot` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/control.rs:6440 | `ur_29_a_loss_published_during_a_release_settles_the_channel_less_behind_it` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:6512 | `ur_24_a_stopped_receive_channel_is_given_no_timed_frequency` | hardware_timed booking, queue and release |
| crates/ezsdr-radio-uhd/src/provider/control.rs:6533 | `ur_24_a_stopped_receive_channel_still_takes_its_gain` | timed gain update |
| crates/ezsdr-radio-uhd/src/provider/control.rs:6545 | `ur_24_a_frequency_for_a_far_planned_segment_is_deferred_to_its_configure` | timed receive frequency hand-over |
| crates/ezsdr-radio-uhd/src/provider/control.rs:6578 | `ur_24_a_frequency_inside_a_running_segment_is_given_timed` | hardware_timed booking, queue and release |
| crates/ezsdr-radio-uhd/src/provider/control.rs:6594 | `ur_24_a_refusal_booked_after_a_release_leaves_the_pending_segments_value_to_its_configure` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:6631 | `ur_24_a_frequency_due_at_a_segment_origin_is_given_timed_and_the_configure_carries_it` | hardware_timed booking, queue and release |
| crates/ezsdr-radio-uhd/src/provider/control.rs:6678 | `ur_24_a_deferred_value_survives_a_start_booked_at_the_release_with_lead_equal_to_the_window` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:6717 | `ur_25_a_move_after_a_refused_source_change_configures_the_destination_at_the_unchanged_origin` | RM-34 move / membership propagation |
| crates/ezsdr-radio-uhd/src/provider/control.rs:6809 | `ur_25_a_transmit_change_after_a_refusal_begins_as_if_not_refused` | refusal reached through a cold item |
| crates/ezsdr-radio-uhd/src/provider/control.rs:6851 | `ur_25_a_transmit_change_after_repeated_refusals_begins_as_if_none_were_refused` | refusal reached through a cold item |
| crates/ezsdr-radio-uhd/src/provider/control.rs:6881 | `ur_24_a_frequency_waits_for_the_next_segment_that_holds_its_channel` | timed receive frequency hand-over |
| crates/ezsdr-radio-uhd/src/provider/control.rs:6923 | `ur_24_the_frequency_boundaries_at_the_origin_and_the_cut` | timed receive frequency hand-over |
| crates/ezsdr-radio-uhd/src/provider/control.rs:6970 | `ur_24_a_gap_value_for_a_move_destination_waits_for_the_source_to_release` | RM-34 move / membership propagation |
| crates/ezsdr-radio-uhd/src/provider/control.rs:7034 | `ur_24_a_gap_value_for_a_configured_segment_is_applied_untimed_at_its_release` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:7069 | `ur_24_a_frequency_before_t0_for_the_first_segment_is_applied_untimed` | hardware_timed booking, queue and release |
| crates/ezsdr-radio-uhd/src/provider/control.rs:7092 | `ur_24_a_later_cut_that_removes_a_configured_segment_leaves_no_stale_mark` | RM-34 move / membership propagation |
| crates/ezsdr-radio-uhd/src/provider/control.rs:7160 | `ur_25_the_mark_is_published_while_the_projection_lock_is_held` | segment configuration projection of updates |
| crates/ezsdr-radio-uhd/src/provider/control.rs:7177 | `ur_24_a_value_booked_just_after_the_owner_configures_is_not_lost` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/control.rs:7197 | `ur_24_a_booking_during_a_configure_waits_and_the_release_after_it_applies_untimed` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:7237 | `ur_24_a_release_waits_for_a_configure_that_carried_its_booking` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:7274 | `ur_24_a_move_round_trip_does_not_use_the_first_segment_s_mark` | RM-34 move / membership propagation |
| crates/ezsdr-radio-uhd/src/provider/control.rs:7304 | `ur_29_a_deferred_frequency_stops_at_a_loss_like_a_given_one` | timed receive frequency hand-over |
| crates/ezsdr-radio-uhd/src/provider/control.rs:7326 | `ur_24_deferred_frequency_rows_are_the_expected_ones` | timed receive frequency hand-over |
| crates/ezsdr-radio-uhd/src/provider/control.rs:7372 | `ur_25_a_transmit_channel_that_leaves_and_returns_keeps_its_old_timed_value` | hardware_timed booking, queue and release |
| crates/ezsdr-radio-uhd/src/provider/control.rs:7416 | `ur_24_a_frequency_just_after_an_origin_is_given_timed_and_follows_the_configure` | hardware_timed booking, queue and release |
| crates/ezsdr-radio-uhd/src/provider/control.rs:7464 | `ur_24_a_frequency_of_a_channel_the_running_segment_does_not_hold_is_deferred` | timed receive frequency hand-over |
| crates/ezsdr-radio-uhd/src/provider/control.rs:7494 | `ur_24_a_refused_untimed_release_is_a_refusal_like_a_timed_one` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:7554 | `ur_24_a_release_waits_until_begin_publishes_eligibility` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:7609 | `ur_24_a_first_rx_sample_preserves_eligibility_and_segment_identity` | hand-over eligibility |
| crates/ezsdr-radio-uhd/src/provider/control.rs:7650 | `ur_24_a_begin_and_timed_apply_serialize_in_both_chain_lock_orders` | hardware_timed booking, queue and release |
| crates/ezsdr-radio-uhd/src/provider/control.rs:7767 | `ur_24_a_held_frequency_is_handed_after_begin_inside_release_window` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:7788 | `ur_24_a_held_frequency_is_handed_after_its_instant` | timed receive frequency hand-over |
| crates/ezsdr-radio-uhd/src/provider/control.rs:7812 | `ur_24_a_unbegun_successor_does_not_extend_the_old_chain_drain` | RX DDC drain |
| crates/ezsdr-radio-uhd/src/provider/control.rs:7972 | `ur_24_a_blocked_rx_head_holds_later_rx_and_tx_updates` | burst booked on a later clock |
| crates/ezsdr-radio-uhd/src/provider/control.rs:8033 | `ur_24_a_blocked_rx_head_still_folds_earlier_handed_updates` | timed hand-over |
| crates/ezsdr-radio-uhd/src/provider/control.rs:8071 | `ur_24_a_refused_blocked_head_reconsiders_later_commands` | burst booked on a later clock |
| crates/ezsdr-radio-uhd/src/provider/control.rs:8096 | `ur_24_a_refused_covering_segment_retargets_held_frequency` | timed receive frequency hand-over |
| crates/ezsdr-radio-uhd/src/provider/control.rs:8123 | `ur_24_a_stop_retargets_held_frequency` | timed receive frequency hand-over |
| crates/ezsdr-radio-uhd/src/provider/control.rs:8154 | `ur_24_a_cold_change_before_e_retargets_held_frequency` | timed receive frequency hand-over |
| crates/ezsdr-radio-uhd/src/provider/control.rs:8189 | `ur_24_a_move_retargets_held_frequency_to_new_destination` | RM-34 move / membership propagation |
| crates/ezsdr-radio-uhd/src/provider/control.rs:8236 | `ur_24_a_removed_destination_retargets_held_frequency_to_no_channel` | RM-34 move / membership propagation |
| crates/ezsdr-radio-uhd/src/provider/control.rs:8308 | `ur_24_a_configured_successor_configure_refusal_keeps_frequency_held` | timed receive frequency hand-over |
| crates/ezsdr-radio-uhd/src/provider/control.rs:8313 | `ur_24_a_configured_successor_start_refusal_keeps_frequency_held` | timed receive frequency hand-over |
| crates/ezsdr-radio-uhd/src/provider/control.rs:8318 | `ur_24_a_stop_settles_blocked_rx_head_and_later_commands` | burst booked on a later clock |
| crates/ezsdr-radio-uhd/src/provider/control.rs:8343 | `ur_24_a_loss_during_timed_rx_handoff_finishes_without_recovery_calls` | hardware_timed booking, queue and release |
| crates/ezsdr-radio-uhd/src/provider/control.rs:8450 | `ur_24_a_late_command_restart_preserves_rx_eligibility_before_release` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:8455 | `ur_24_a_late_command_restart_preserves_rx_eligibility_after_release` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/control.rs:8460 | `ur_24_a_cut_bearing_late_command_finishes_without_restart` | cut by a cold change |
| crates/ezsdr-radio-uhd/src/provider/control.rs:8485 | `ur_24_a_delayed_rx_apply_refusal_rolls_back_debt_and_record` | RX DDC debt |
| crates/ezsdr-radio-uhd/src/provider/core.rs:1950 | `rx_chain_hand_transitions` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/core.rs:2009 | `rx_chain_flush_transitions` | RX DDC flush |
| crates/ezsdr-radio-uhd/src/provider/core.rs:2405 | `rx_chain_reuse_guard_rejects_active_open_and_debt_close` | RX DDC debt |
| crates/ezsdr-radio-uhd/src/provider/core.rs:2421 | `rx_chain_reconfigure_close_requires_idle_empty_debt` | RX DDC debt |
| crates/ezsdr-radio-uhd/src/provider/core.rs:2435 | `rx_chain_release_unneeded_close_requires_idle_empty_debt` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/core.rs:2446 | `rx_chain_open_failure_close_requires_idle_empty_debt` | RX DDC debt |
| crates/ezsdr-radio-uhd/src/provider/core.rs:2538 | `ur_34_a_waited_open_waits_for_a_finite_unprocessed_move_segment` | RM-34 move / membership propagation |
| crates/ezsdr-radio-uhd/src/provider/core.rs:2587 | `ur_34_a_waited_open_waits_for_same_move_source_configuration` | RM-34 move / membership propagation |
| crates/ezsdr-radio-uhd/src/provider/core.rs:2626 | `ur_34_a_move_destination_waits_for_source_busy_after_channel_release` | RM-34 move / membership propagation |
| crates/ezsdr-radio-uhd/src/provider/core.rs:2682 | `ur_34_a_release_skips_a_stream_held_busy_by_from_none` | from-none claim, enable and join |
| crates/ezsdr-radio-uhd/src/provider/core.rs:2702 | `ur_34_a_release_closes_an_idle_streamer_needed_by_a_move` | RM-34 move / membership propagation |
| crates/ezsdr-radio-uhd/src/provider/core.rs:2711 | `ur_34_a_release_keeps_an_idle_streamer_with_no_peer_need` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/core.rs:2727 | `ur_34_a_waited_open_reloads_a_move_refused_while_it_waits` | RM-34 move / membership propagation |
| crates/ezsdr-radio-uhd/src/provider/core.rs:2757 | `ur_34_a_owner_does_not_open_a_move_plan_removed_before_its_turn` | RM-34 move / membership propagation |
| crates/ezsdr-radio-uhd/src/provider/core.rs:2794 | `ur_34_a_waited_open_aborts_when_the_planned_segment_changes` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/core.rs:2883 | `ur_34_a_waited_open_ends_at_device_loss_without_closing_after_loss` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/core.rs:2909 | `ur_34_a_waited_open_ends_at_provider_stop_without_opening` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/core.rs:2938 | `ur_34_a_waited_transmit_open_observes_provider_stop_with_a_later_cold_reserved` | Run-time cold change |
| crates/ezsdr-radio-uhd/src/provider/core.rs:3004 | `ur_25_a_transmit_move_after_a_refused_source_change_begins_the_destination_as_if_not_refused` | RM-34 move / membership propagation |
| crates/ezsdr-radio-uhd/src/provider/core.rs:3065 | `ur_34_a_refused_move_reconciles_both_transmit_clocks` | RM-34 move / membership propagation |
| crates/ezsdr-radio-uhd/src/provider/core.rs:3174 | `ur_25_a_claim_release_needs_a_free_idle_stream_with_no_active_segment` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/core.rs:3205 | `ur_25_a_stream_starts_idle_or_waiting` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/core.rs:3234 | `ur_26_a_receive_start_without_a_segment_stays_active` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/core.rs:3254 | `ur_25_a_search_leaves_the_stream_pending_or_idle` | owner phase (idle/pending/active) |
| crates/ezsdr-radio-uhd/src/provider/core.rs:3279 | `ur_17_a_receive_segment_moves_from_pending_to_active_to_pending` | RM-34 move / membership propagation |
| crates/ezsdr-radio-uhd/src/provider/core.rs:3333 | `ur_25_a_claim_from_none_admits_a_read_in_flight` | from-none claim, enable and join |
| crates/ezsdr-radio-uhd/src/provider/core.rs:3458 | `ur_25_move_source_busy_reads_the_owner_of_the_move` | RM-34 move / membership propagation |
| crates/ezsdr-radio-uhd/src/provider/core.rs:3487 | `ur_34_a_release_gives_back_a_claim_when_loss_arrives_after_the_claim` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/rx.rs:921 | `ur_24_equal_time_rx_entries_need_distinct_samples_after_acceptance` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/rx.rs:943 | `ur_24_a_cut_after_timed_rx_release_drains_before_its_successor` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/rx.rs:1012 | `ur_24_a_stop_decision_waits_for_a_gated_timed_rx_handover` | hardware_timed booking, queue and release |
| crates/ezsdr-radio-uhd/src/provider/rx.rs:1102 | `ur_24_a_completed_rx_owner_serializes_a_late_frequency` | timed receive frequency hand-over |
| crates/ezsdr-radio-uhd/src/provider/rx.rs:1193 | `ur_24_provider_stop_drains_after_s_then_restores_the_frequency_in_force_at_s` | timed receive frequency hand-over |
| crates/ezsdr-radio-uhd/src/provider/rx.rs:1295 | `ur_24_multichannel_abort_keeps_overflow_and_drain_samples_off_the_link` | RX DDC drain |
| crates/ezsdr-radio-uhd/src/provider/rx.rs:1350 | `ur_24_drain_deadline_runs_one_bounded_flush_and_consumes_or_reports` | timed hand-over deadline |
| crates/ezsdr-radio-uhd/src/provider/rx.rs:1476 | `ur_24_a_left_and_latest_survive_close_open_and_chain_reuse` | RX leftover DDC value |
| crates/ezsdr-radio-uhd/src/provider/rx.rs:1570 | `ur_24_provider_stop_skips_restore_for_left_on_this_chain` | restore at Provider::stop |
| crates/ezsdr-radio-uhd/src/provider/rx.rs:1630 | `ur_24_a_local_leftover_blocks_restore` | restore at Provider::stop |
| crates/ezsdr-radio-uhd/src/provider/rx.rs:1658 | `ur_24_a_deadline_receive_uses_one_short_packet_and_preserves_successor_lead` | timed hand-over deadline |
| crates/ezsdr-radio-uhd/src/provider/rx.rs:1792 | `ur_24_nonloss_rx_stop_failure_reports_pending_and_skips_restore` | restore at Provider::stop |
| crates/ezsdr-radio-uhd/src/provider/rx.rs:1843 | `ur_24_provider_stop_skips_restore_when_flush_stop_fails` | restore at Provider::stop |
| crates/ezsdr-radio-uhd/src/provider/rx.rs:1910 | `ur_24_a_loss_during_rx_flush_finishes_without_recovery_calls` | RX DDC flush |
| crates/ezsdr-radio-uhd/src/provider/rx.rs:2067 | `ur_24_provider_stop_records_restore_failure_and_stops_after_loss` | restore at Provider::stop |
| crates/ezsdr-radio-uhd/src/provider/rx.rs:2115 | `ur_24_a_parallel_rx_restore_checks_do_not_nest_chain_locks` | restore at Provider::stop |
| crates/ezsdr-radio-uhd/src/provider/rx.rs:2184 | `ur_34_a_waited_receive_open_exits_when_stop_removes_its_segment` | RM-34 move / membership propagation |
| crates/ezsdr-radio-uhd/src/provider/rx.rs:3450 | `ur_25_a_refusal_leaves_the_transmit_stream_alone` | refusal reached through a cold item |
| crates/ezsdr-radio-uhd/src/provider/rx.rs:3502 | `ur_25_a_refusal_of_a_layout_stream_leaves_the_other_stream_alone` | refusal reached through a cold item |
| crates/ezsdr-radio-uhd/src/provider/tx.rs:1445 | `ur_34_a_transmit_stop_during_release_wait_cancels_later_bursts` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/tx.rs:1628 | `ur_13_a_transmit_switch_is_compared_with_the_stored_outcome` | transmit clock switch of a cold change |
| crates/ezsdr-radio-uhd/src/provider/tx.rs:1972 | `ur_22_halted_tx_chain_resets_only_when_the_next_segment_is_adopted` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/tx.rs:2091 | `ur_25_a_refused_enable_with_a_later_cold_has_no_tx_clock` | enable / disable of a stream |
| crates/ezsdr-radio-uhd/src/provider/tx.rs:2131 | `ur_13_a_refused_comparison_is_repeated_at_the_next_switch` | transmit clock switch of a cold change |
| crates/ezsdr-radio-uhd/src/provider/tx.rs:2152 | `ur_25_a_refused_transmit_switch_makes_no_device_call` | transmit clock switch of a cold change |
| crates/ezsdr-radio-uhd/src/provider/tx.rs:2176 | `ur_13_a_failed_configure_by_the_owner_is_configured_again_without_reopen` | timed gain update |
| crates/ezsdr-radio-uhd/src/provider/tx.rs:2196 | `ur_25_a_refused_transmit_clock_is_not_adopted` | refusal reached through a cold item |
| crates/ezsdr-radio-uhd/src/provider/tx.rs:2218 | `ur_25_a_refused_last_transmit_change_ends_idle` | refusal reached through a cold item |
| crates/ezsdr-radio-uhd/src/provider/tx.rs:2253 | `ur_13_a_switch_to_the_verified_set_opens_nothing` | transmit clock switch of a cold change |
| crates/ezsdr-radio-uhd/src/provider/tx.rs:2453 | `ur_25_a_transmit_read_sets_idle_pending_or_active` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/tx.rs:2496 | `ur_23_a_refused_transmit_change_ends_its_clock_at_its_origin` | refusal reached through a cold item |
| crates/ezsdr-radio-uhd/src/provider/tx.rs:2519 | `ur_25_a_burst_on_a_later_clock_survives_an_earlier_switch` | transmit clock switch of a cold change |
| crates/ezsdr-radio-uhd/src/provider/tx.rs:2601 | `ur_21_first_dispatch_cannot_move_past_a_cold_switch` | RM-34 move / membership propagation |
| crates/ezsdr-radio-uhd/src/provider/tx.rs:3795 | `ur_29_eob_loss_during_switch_stops_device_calls` | transmit clock switch of a cold change |
| crates/ezsdr-radio-uhd/src/provider/tx.rs:4083 | `ur_25_a_refusal_leaves_the_receive_stream_alone` | refusal reached through a cold item |
| crates/ezsdr-radio-uhd/src/provider/tx.rs:4267 | `ur_29_a_loss_after_a_configure_adopts_no_transmit_clock` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/tx.rs:4296 | `ur_29_a_lost_transmit_owner_configure_releases_stream_lock_before_reporting` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/tx.rs:4318 | `ur_26_a_transmit_switch_at_a_stop_keeps_its_clock_and_sends_nothing` | transmit clock switch of a cold change |
| crates/ezsdr-radio-uhd/src/provider/tx.rs:4407 | `ur_25_a_refused_transmit_owner_books_and_gives_back_in_one_line_section` | refusal reached through a cold item |
| crates/ezsdr-radio-uhd/src/provider/tx.rs:4464 | `ur_26_stop_cancels_current_and_future_bursts_record_only` | machinery of the Run-time change path |
| crates/ezsdr-radio-uhd/src/provider/tx.rs:4487 | `ur_23_a_cold_switch_cancels_a_current_burst_and_releases_its_reservation` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/tx.rs:4511 | `ur_25_a_skipped_future_clock_cancellation_releases_its_reservation` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio-uhd/src/provider/tx.rs:5494 | `ur_23_txs_switch_and_stop_do_not_send_past_a_high_rate_frontier` | transmit clock switch of a cold change |
| crates/ezsdr-radio-uhd/src/provider/tx.rs:5849 | `ur_23_m14_a_cold_cut_is_not_waited_past_or_crossed` | Run-time cold change |
| crates/ezsdr-radio-uhd/src/provider/tx.rs:5871 | `ur_23_m14e_a_waiting_owner_switches_to_the_new_clock_at_the_cut` | transmit clock switch of a cold change |
| crates/ezsdr-radio-uhd/tests/fake.rs:9367 | `h4_probe_records_a_timed_apply_under_its_instant_with_its_host_entry` | hardware_timed booking, queue and release |
| crates/ezsdr-radio/tests/timeline.rs:441 | `rm_26_pruning_folds_a_cold_change_floored_at_an_earlier_one` | Run-time cold change |
| crates/ezsdr-radio/tests/timeline.rs:486 | `rm_21_a_cold_change_to_another_set_of_the_same_size_starts_a_segment` | Run-time cold change |
| crates/ezsdr-radio/tests/timeline.rs:505 | `rm_21_a_stream_that_loses_its_last_channel_starts_again_when_one_enters` | timed gain update |
| crates/ezsdr-radio/tests/timeline.rs:524 | `rm_34_a_move_books_both_lines_at_one_instant` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:544 | `rm_34_a_move_is_floored_at_a_cold_change_of_either_line` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:567 | `rm_34_a_later_cold_change_is_floored_at_the_move` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:590 | `rm_34_the_destination_waits_the_source_s_cut_and_its_own_lead` | cut by a cold change |
| crates/ezsdr-radio/tests/timeline.rs:613 | `rm_34_a_refused_move_halts_both_lines_until_a_change_or_a_start` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:644 | `rm_34_a_move_into_a_stopped_stream_does_not_start_it` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:657 | `rm_34_a_move_is_floored_at_a_receive_line_s_start` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:673 | `rm_34_a_move_into_a_stream_with_no_channel_starts_it` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:685 | `rm_34_a_move_whose_booking_fails_leaves_both_lines` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:698 | `rm_34_a_move_reads_the_configuration_a_pruned_line_keeps` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:715 | `rm_34_a_move_waits_a_source_segment_that_starts_after_its_instant` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:731 | `rm_34_a_later_booking_moves_the_destination_with_the_source` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:755 | `rm_34_a_move_floors_a_later_start_of_the_destination` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:782 | `rm_34_a_move_keeps_a_transmit_clock_booked_before_an_end` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:808 | `rm_34_a_pruned_move_keeps_its_release_floor` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:882 | `rm_34_a_source_first_prune_keeps_move_dependencies_until_both_lines_pass` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:889 | `rm_34_a_destination_first_prune_keeps_move_dependencies_until_both_lines_pass` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:928 | `rm_34_a_chain_prune_reaches_a_fixed_point_from_b_then_c` | move-aware prune |
| crates/ezsdr-radio/tests/timeline.rs:935 | `rm_34_a_chain_prune_reaches_a_fixed_point_from_c_then_b` | move-aware prune |
| crates/ezsdr-radio/tests/timeline.rs:942 | `rm_34_a_move_takes_the_greatest_cut_of_the_moves_into_a_line` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:970 | `rm_34_a_move_does_not_floor_an_item_received_before_it_at_its_instant` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:989 | `rm_34_a_move_is_refused_on_both_lines_whichever_line_names_it` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:1011 | `rm_34_a_move_without_a_timed_instant_takes_effect_at_its_receipt` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:1026 | `rm_34_a_pruned_move_takes_the_greatest_floor_of_its_line` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:1052 | `rm_34_a_moves_dependence_propagates_along_a_chain` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:1081 | `rm_34_a_move_resumes_a_halted_stream` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:1099 | `rm_34_a_refused_source_change_does_not_move_the_destination` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:1118 | `rm_34_the_destination_waits_a_stopped_source_s_cut` | cut by a cold change |
| crates/ezsdr-radio/tests/timeline.rs:1132 | `rm_34_the_destination_waits_an_ended_source_s_cut` | cut by a cold change |
| crates/ezsdr-radio/tests/timeline.rs:1146 | `rm_34_a_pruned_stopped_source_keeps_its_release` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio/tests/timeline.rs:1168 | `rm_34_a_move_does_not_wait_a_segment_of_another_channel` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:1185 | `rm_34_a_move_reads_the_source_s_plan_not_its_arrival_order` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:1208 | `rm_34_a_move_reads_a_transmit_clock_the_loss_left_running` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:1222 | `rm_34_a_move_waits_a_segment_the_plan_leaves_out` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:1240 | `rm_34_a_move_does_not_wait_a_channel_the_source_gains_after_it` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:1257 | `rm_34_a_move_waits_the_greatest_end_not_the_last_one_read` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:1276 | `rm_34_a_move_does_not_wait_an_omitted_segment_the_source_gains_after_it` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:1292 | `rm_34_a_move_books_both_transmit_clocks` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:1310 | `rm_34_an_impossible_move_is_refused_without_a_booking` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:1334 | `rm_34_config_at_reads_the_move_and_the_pruned_state` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:1358 | `rm_34_generated_moves_keep_both_lines_invariants` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:1475 | `rm_25_the_layout_instant_is_floored_at_every_earlier_cold_change` | Run-time cold change |
| crates/ezsdr-radio/tests/timeline.rs:1497 | `rm_25_the_timed_instant_is_floored_at_every_booked_command_inside_the_window` | hardware_timed booking, queue and release |
| crates/ezsdr-radio/tests/timeline.rs:1534 | `rm_36_covering_boundaries` | RM-36 covering |
| crates/ezsdr-radio/tests/timeline.rs:1579 | `rm_25_a_transmit_change_after_a_refusal_begins_as_if_not_refused` | refusal reached through a cold item |
| crates/ezsdr-radio/tests/timeline.rs:1753 | `rm_25_a_refused_segment_runs_for_planning_until_the_next_change_cuts_it` | refusal reached through a cold item |
| crates/ezsdr-radio/tests/timeline.rs:1812 | `rm_34_a_refusal_does_not_change_a_move_release` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:1866 | `rm_34_a_refusal_before_a_stop_or_in_a_chain_does_not_change_a_release` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio/tests/timeline.rs:2082 | `rm_34_a_refused_segment_is_a_release_only_for_a_channel_it_holds_and_that_began_before` | hardware_timed release, channel release and waits |
| crates/ezsdr-radio/tests/timeline.rs:2114 | `rm_25_a_transmit_refusal_does_not_move_a_later_origin` | RM-34 move / membership propagation |
| crates/ezsdr-radio/tests/timeline.rs:2142 | `rm_25_a_transmit_refusal_followed_by_other_items_keeps_the_next_origin` | refusal reached through a cold item |
| crates/ezsdr-radio/tests/timeline.rs:2180 | `rm_34_a_transmit_move_after_a_refused_source_change_waits_the_cut_the_clock_would_have_had` | RM-34 move / membership propagation |

Deleted test helpers (fixtures only the deleted tests used) are removed with them; `deadsrc` found each by the compiler's never-used warning.

## 4. Kept although the brief expected them gone, and open items

- `Planned.refused`, `Planned.settings`, `ColdConfig` (now `base` only), `RxChain` device states and intent, `Streams::{book, refuse, end, prune, planned, record_*}`, `owner_configure` with `claim_owner`, `begin_read`/`end_read`/`begin_shutdown_read`: kept (owner configure snapshot, refusal halt, claim/read exclusion).
- `channels.rs` `Description::{stream_channels}` and `Channels::{with, without}`: `with` is production (`stream_channels`), `without` is a test helper only.
- `CancelReason::ColdChange` stays: the TX reopen-failure cancellation reports through it (`halt_failed_reopen`) and a burst on a clock that ended at its origin; part (c) renames it.
- `FaultKind::ColdChangeRefused` (ezsdr-sim) stays and MockRadio still accepts such a fault: it names a change that has no Run-time form, so it never fires (`MockRadio::fires` excludes it). A spec decision for part (c): refuse it at `prepare`, or withdraw the kind.
- The Mock `ezsdr.radio.mock.applied` section stays and is always empty (the section list is MR-spec text, part (c)).
- `tx_start_lead_ns` stays (stage 4 decides).
- `rx.rs:587` labels a `start_rx` refusal `update_parameter` with no action id (rev 4 note): unchanged, for the stage-3 brief. The `ready` scenario of `fake.rs@HEAD:2783` goes into the stage-3 brief.

# Part (d) — the pins the deletion lost, and the last dead code (revision 6)

Brief `brief-s2-d.md`; classification `opus-s2-c1-unkilled.md` (the re-check reviewer's read of the 62 rows part (c1) deleted as "no surviving test kills the mutant": RETARGET 4, OWED 15, EQUIVALENT 24, DEAD 19). Rows: `rows-s2.md`, section "Part (d)". Probes: `work/d-probes/`.

## 5. New tests

| path | test | what it asserts | the mutant that fails it (row) |
|---|---|---|---|
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_25_a_stopped_stream_ends_at_its_cut_and_its_late_tail_stays_off_the_new_clock` | 60 ms link, Stop then `start_rx` 300 ms in: the stopped clock's last delivered sample is its `ended_at` sample, and the new clock's first block is not before the `rx_stop_untimed` row's instant | the quiet-span wait after a cut dropped (B07, `rx.rs` `ended = cut.is_some_and(...)`); the old stream's tail published on the new clock (B06, `rx.rs` `first_tick < stream.not_before`) |
| crates/ezsdr-radio-uhd/tests/fake.rs | `ur_36_every_restart_is_configured_with_the_connect_values` | connect at 2 GHz; two Stop + `start_rx` rounds; each makes `apply rx` calls and the device reads 2 GHz after it | the second restart not configured (P420, `core.rs` `owner_step`); the restart snapshot taken from the profile defaults instead of the Run's start values (P1395, `control.rs` `ColdConfig::new(self.config.clone())`) |
| crates/ezsdr-radio-uhd/src/provider/rx.rs (unit) | `ur_24_a_missed_start_after_a_loss_makes_no_restart_call` | `rx_start` count unchanged after `LateCommand` on a lost device | the loss check before the missed-start retry (P1290) |
| rx.rs (unit) | `ur_29_a_loss_between_two_channels_of_a_configure_applies_no_more` | `apply` probe publishes `device_lost` at the second channel; no `apply` follows `mark_lost` | the loss check between a configure's channels (P391) |
| rx.rs (unit) | `ur_29_an_owner_does_not_configure_a_segment_a_stop_dropped_from_the_plan` | after a Stop dropped the planned segment, `owner_configure` returns `Ok(false)` and makes no device call | the stale-plan guard in `owner_configure` (P513) |
| crates/ezsdr-radio-uhd/src/provider/control.rs (unit) | `ur_14_each_action_is_booked_before_the_next_recv` | an `ActionReceiver` that notes each `recv` coming before the previous Action's refusal is on record sees none | `Control::run` taking every Action before booking any (U13) |
| crates/ezsdr-mock-radio/tests/mock_layout.rs | `mr_25_a_stream_stop_leaves_another_transmit_stream_radiating` | Stop of `tx/s0` cuts its own segments only: stream 1's held burst, starting after the cut, is heard | the stop ending or withdrawing the segments of every stream in the shared plan (M68, M69) |
| mock_layout.rs | `mr_27_a_segment_row_holds_its_channel_order_and_its_end` | the `segments` row of a two-channel receive stream lists `[0, 1]` and its `ended_at` is the Stop's tick | channel order reversed in the row (P154); `ended_at` not written (P172) |
| mock_layout.rs | `mr_34_a_restart_redraws_each_random_channel_in_channel_order` | two CBX channels in one stream: at the restart origin channel 0 hears the first retune draw, channel 1 the second | a random channel keeping its phase (P183); the segment's channels taken in reverse (P190) |
| mock_layout.rs | `mr_34_two_streams_restarted_at_one_origin_draw_in_stream_order` | two streams restarted at one origin: stream 0 takes the first retune draw, stream 1 the second | tunes at one instant taken in reverse stream order (P191) |
| mock_layout.rs | `mr_34_a_layout_unlinked_receive_stream_takes_no_untimed_tune` (extended) | the accepted `start_rx` on the stream with no link leaves no clock and no `segments` row | the link check before a segment begins (P153) |
| crates/ezsdr-mock-radio/tests/mock_radio.rs | `mr_29_other_actions_are_command_rejected` (extended) | a classless `radio.rx.c0.frequency_hz` update reaching the Mock directly is the third refusal, "MR-18: target or configuration key is not updateable", answered with its action id | the guard returning `Ok(())` (P1394) |

Rows with no new test, killed by an existing one (run to confirm): P1396, P1397 (`ur_17_a_finite_segment_delivers_through_its_cut`: the clock is registered at the first block and ended at the cut); F36, P1220 (`rm_26_the_timeline_against_mockradio`), V06 (`rm_26_the_timeline_cases`), T40 (`mr_29_start_rx_is_never_late`).

Timing: the two `fake.rs` tests (wall-clock settle) and the control and three rx units (threads, a spy) were run 20 times each: 20 of 20 passed. The Mock tests are pure.

## 6. Deleted regions (dead, each checked: instrumented in a copy and the suites run; nothing reached them with a real stimulus)

| path | region | why dead |
|---|---|---|
| crates/ezsdr-radio-uhd/src/provider/core.rs | `Streams::reconcile` whole (registration of a clock per plan segment beyond the arm clock; ending a clock at its segment's cut; ending clocks beyond the plan: P444, P261, J21) and its call in `changed`; `dir_of`; the `ended` flag of `StreamState.clocks` (now `Vec<Clock>`) | A transmit stream has at most one clock (registered at `arm`, seeded at `control.rs` `Control::new`) and its plan keeps its one segment. Instrumenting the three branches and running the `ezsdr-radio-uhd` lib and fake suites: the registration branch was reached 9 times and the cut branch once, all inside unit tests that build a transmit plan with no clock (`ur_26_a_finished_stream_clears_only_its_own_floor`, `ur_26_an_end_takes_delivered_from_each_segment_s_own_stream`, `ur_29_a_loss_ends_every_stream_once`, `ur_25_uhd_tx_reads_the_plan_again_when_it_changes`), none asserting on it; the tail branch never. With `reconcile` removed those four and the rest of both suites pass. `changed` stays as `self.version += 1` (uhd-tx's re-read, P262). |
| crates/ezsdr-mock-radio/src/lib.rs | the transmit arm of `untimed_tune`, the transmit half of `tune`'s guard, `TxStream::tuned`, the transmit loop that pushed `WorkKind::Tune`, `WorkKind::Tune.direction`, the `direction` parameters of `tune_next`, `tune`, `untimed_tune`, `tune_order` and `update_layout_segment_coherence` (P182, P193, P195) | `tune` returned at `segment.by.is_none()` and every transmit segment's `by` is none: a transmit line never begins a second segment (`Start` is receive-only). |
| crates/ezsdr-radio-uhd/tests/common/probe_device.rs | the `host_entry_ns` field of the timed-apply row and its capture (P1142) | no reader: `grep host_entry_ns` finds only the writer. |
| design/09-mock-radio.md | MR-34's "receive before transmit" in the untimed-tune order | no transmit untimed tune exists. |

Surviving callers to say plainly: `Streams::changed` (kept); `Tx::cancel_unstarted` (`tx.rs`) still cancels a burst booked on a clock the plan cut at its origin, a state reachable only when a loss or `Provider::stop` comes at or before the clock's origin (the arm instant's lattice): with `reconcile` gone that clock is no longer ended at its origin, the burst is still cancelled. Not changed here.

## 7. Deleted tests (part (d))

| path | test | why |
|---|---|---|
| crates/ezsdr-radio-uhd/src/provider/control.rs | `ur_24_a_cut_booked_after_the_release_leaves_the_entry_for_the_next_segment` (and `manual_rig_events`, the `Device` import) | a raw `FakeDevice` characterization (a timed entry survives a raw stop and restart) whose Provider subject, the timed release, is deleted; the fake's rule is pinned by `ur_33_a_ddc_fifo_outlives_its_streamer` and `ur_33_an_untimed_write_lands_at_once_and_a_waiting_entry_overwrites_it_later` in `device.rs` |

## 8. The four tests whose citations part (c2) dropped (item 4 of the brief)

| test | decision | why |
|---|---|---|
| `cancellation_reports_only_cold_change` (`ezsdr-radio/src/lib.rs`) | kept, cited in RM-16's Test line, out of the renames file | `cancellation_reports` is live: `tx.rs` `report_cancelled_burst` (UR-22 reopen refusal, UR-25 never-started clock) and MockRadio's `record_cancelled_burst` use it; it pins "Stop, `Provider::stop` and loss emit no `COMMAND_REJECTED`" (RM-16). Rows P561, P562. The reason is still named `ColdChange`: a rename is open for a later stage |
| `ur_26_a_finished_stream_clears_only_its_own_floor` (`core.rs`) | kept, cited in UR-26's Test line, out of the renames file | `finished` and the delivered floor an abort cuts at are the RM-16 machinery that survives (row P558) |
| `ur_24_a_cut_booked_after_the_release_leaves_the_entry_for_the_next_segment` (`control.rs`) | deleted (section 7), stays in the renames file | see section 7 |
| `the_ma1_phase_is_read_against_the_waveform_and_a_weak_or_unsteady_loop_is_inconclusive` (`fake.rs`) | kept, cited in UR-12's Test line (with `hw_ma1_phase_after_a_pre_start_tune`), out of the renames file | the bench probe of the loopback phase after a receive frequency applied untimed at connect survives (KL-32); this rehearses its `tone` readers (rows P1016, P1017, P1020). `the_ma1_phase_probe_reads_a_phase_in_both_ways_on_the_fake` has no test fn any more and stays `-` |

## 9. Counts and result

Rows: re-added 19 (F36, P1220, T40, M68, M69, V06, U13, B06, B07, P1290, P391, P513, P420, P153, P154, P172, P183, P190, P191), new 4 (P1394 to P1397), re-anchored after part (d)'s own deletions 6 (P170, P178, P186, P196, P263, P450), deleted with their code 1 (P259). Every one of these was applied in a throwaway copy and its named test failed (probe logs `work/d-probes/`). Suites: see the report.


# Rev 7 — fix round after Sol's code review (`sol-s2-code-final.md`, `dispositions-s2-code.md`)

Rev 6 is kept as `test-plan-s2.v6.md`.

## 10. Finding 1 (P2): a transmit clock the plan cuts at its origin ends there

Production: `Streams::changed` (`crates/ezsdr-radio-uhd/src/provider/core.rs`) takes `(core, stream)` and, after `version += 1`, ends the stream's one transmit clock at `clock.instant(cut)` when its plan's first segment has a cut and the clock registry does not already show it ended. One representation: the registry's `ended_at` (no flag in `StreamState`; `StreamState.clocks` stays `Vec<Clock>`). A failure to end is `command_rejected(None, "update_parameter", "UR-25: <error>")`, the path HEAD used for the cut case (`changed` called `reconcile`, whose `Err` it passed to that call). `changed` is called from `book` and `refuse`, so the loss path (`device_lost_locked` -> `end` -> `book`) is covered with no owner work. Single-clock case only; no registration branch.

What HEAD did after O: HEAD's `reconcile` ended a clock only for a segment with a cut. A transmit segment is cut only by `finish`'s `end_at_origin` (a loss or `Provider::stop` at S <= O; the cut is sample 0, so the end is O); after O the plan has no cut (`timeline.rs` `Kind::End` on transmit only sets `ended`), so the clock stays open. Confirmed by running the new test on a `git archive HEAD` copy (fresh timestamps, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review`, test adapted to HEAD's `Vec<(Clock, bool)>`): it passed with the same assertions.

| test | stimulus | asserts | mutant (row) |
|---|---|---|---|
| `ur_23_a_transmit_clock_the_plan_cuts_at_its_origin_ends_there` (`core.rs` unit, controlled clock, pure: no threads, manual time) | clock registered at O = 10 000 ticks, line planned from O; `Streams::end` (a `Provider::stop`) at O - 4 000, at O, at O + 400, each on a fresh rig; then `changed` again; and `device_lost` on a fresh rig | S < O and S = O: plan cut `Some(0)` and the registry record's `ended_at` = O; S > O: no cut and `ended_at` None; a second `changed` leaves `rec.rejected` empty (no second end); the loss ends the clock at O with no rejection | the end call replaced by `Ok(())` (P1398); the registry check dropped, so a second booking ends again and is rejected `Stopped` (P1399) |

Probes (copy outside the repository, fresh timestamps, `CARGO_INCREMENTAL=0`; logs `work/fix-probes/P1398.log`, `P1399.log`): P1398 fails at `S 6000` (`ended_at` None, expected O); P1399 fails at `S 6000` with `UR-25: the clock domain or authority has stopped` in `rejected`. The existing `ur_25_uhd_tx_reads_the_plan_again_when_it_changes` (a clock with origin 1 s ahead, `Provider::stop` at 500 ms) now reaches the end branch with a real clock and passes.

Rows appended at the end of `plan/maintenance/tools/mutations.json`: P1398, P1399 (filter `ur_23_a_transmit_clock_the_plan_cuts_at_its_origin_ends_there`, crate `ezsdr-radio-uhd`, test_bin `lib`). Rows match once each (`rules.py` 0).

## 11. Finding 2 (P3): spec text rewritten from today's assertions

Rules edited:

| spec | rule | change |
|---|---|---|
| 09 | MR-15 | tracker and device model created at `arm` only; no `cold` transmit change in the block-shortening and burst-closing clauses |
| 09 | MR-27 | `basis` is `unmodelled`, `none` or `deterministic`: no timed tune exists, so a `random_unless_timed_tune` channel is never coherent (the sentence "No timed tune exists during a Run today" is dropped as MR-34 says it) |
| 09 | MR-34 | no timed-tune constants in the draws |
| 09 | M12 (decision table) | random at `prepare` and after each untimed tune; no per-channel constant after a timed tune |
| 09 | `release_window_ns` row | cites RM-25 (the receive successor's ready term), not MR-18 |
| 11 | CH-9 | a transmitter's finality names the stop only; no cold change, no gain or frequency change at an instant |
| 18 | UR-23 | states the transmit clock's end at its origin and cites the new test |

Test rows rewritten (each read against its test):

| spec | row | now says |
|---|---|---|
| 09 | `mr_32_the_transmit_path_delay_shifts_a_loopback`, `mr_33_a_receiver_hears_only_its_own_frequency`, `mr_36_clipping_at_full_scale_is_counted`, `mr_33_layout_gain_and_frequency_timelines_are_per_logical_channel` | values asked at the start, not a change at an instant |
| 09 | `mr_34_a_layout_deterministic_channel_keeps_its_phase_and_coherence_through_entry`, `mr_34_a_layout_unlinked_receive_stream_takes_no_untimed_tune` | a Stop and `start_rx` restart; the linkless `start_rx` accepted with no clock and no row |
| 09 | `mr_12_d14_the_actions_of_a_layout_instance_name_a_stream` | no update to a node; the burst, `Stop` and `start_rx` targets and reasons (MR-16, MR-25, MR-29) |
| 09 | `rm_26_the_timeline_against_mockradio` and its paragraph | generated `Stop`, `start_rx`, faults, loss; no membership transitions |
| 09 | MR-9 Test line, `mr_33_gain_scales_the_samples`, MR-32 Test line, `mr_20_a_lost_device_transmits_nothing_after_the_loss` | no rate change, no gain "read at the sample's instant"; the update is handed to the Mock directly |
| 18 | `ur_25_tx_channels_from_zero_transmits_the_next_burst`, `ur_25_a_second_change_follows_the_first`, `ur_26_no_segment_begins_once_provider_stop_has_begun`, `ur_17_an_ended_segment_prunes_the_items_before_it`, `ur_29_a_loss_ends_the_stream_at_its_instant`, `ur_29_a_silent_stream_ends_at_a_cut_booked_during_its_wait`, `ur_29_a_loss_takes_effect_before_a_change_at_its_instant`, `ur_26_an_end_takes_delivered_only_from_the_segment_it_cuts`, `ur_25_uhd_tx_reads_the_plan_again_when_it_changes` | `Stop` and `start_rx` stimuli (the tests' own), a transmit clock cut by `Provider::stop` |
| 18 | `rm_26_the_timeline_against_uhd_rx`, `rm_26_the_timeline_against_uhd_control` | generated `Stop`, `start_rx`, faults, loss; the compared fields of today's code (control: clocks and refusal count only) |
| 18 | `ur_24_fake_ddc_distinguishes_host_silence_from_a_stalled_ddc` + the loss test; the channel-less `start_rx` row | the fake's DDC model on a direct apply; no "until a channel joins" |
| 16 | `test_ea_16_repeat_uses_stream_membership_without_setting_it`, `test_ea_16_channel_keys_and_stream_membership` / `stream_channels_are_read_only`, `test_ea_17_stream_one_capture_uses_its_recorder`, `test_ea_16_rx_stop_then_start` | asked at connect; setters raise `Error`, a raw `set` is `Rejected`; no rate change in the stop/start test |

Swept and left alone (read, subject survives): specs 01, 03, 04, 05, 08 (their value-change rows use the `test` Vocabulary or are Kernel contracts), 07's test rows and rules (already rewritten by (c2)), 18's FakeDevice rows for timed commands (device-level characterisation), 09's `mr_20_*` loss rows (refusal of an update after a loss still holds).

Open item for the orchestrator (not changed here, outside the brief): MockRadio still carries the `timed_tune` branch of `layout_phase_basis` and `update_layout_segment_coherence` in `crates/ezsdr-mock-radio/src/lib.rs`; with `phase_fixed` 0 for every non-deterministic channel and no timed tune, the `timed` path cannot produce `timed_tune` (a dead branch). Also still unfixed: the multi-pending-fault ordering mismatch (dispositions note).

## 12. Result

`cargo fmt --all`; clippy `-D warnings` workspace and `ezsdr-radio-uhd --features uhd --all-targets` (rdev) clean; `cargo test --workspace` all passed; Python 3.14 suite 78 passed; `rules.py --base HEAD --renames plan/phase8/reviews/47s2-renames.txt` 0 problems. The new unit test is pure (manual clock, no thread), so no 20-run repeat.

## 13. Fix round 2 (revision 8; rev 7 kept in `test-plan-s2.v7.md`): the dead `timed_tune` basis

The re-check (`opus-s2-fix-recheck.md`, "Leftover") found that no `random_unless_timed_tune` channel can be fixed any more: `phase_fixed` starts 1.0 only for `deterministic` channels, and its only writer, the untimed tune of a `start_rx` restart, set it to 0. This resolves section 4's open item. Deleted, with no surviving reader found (grep over `crates`, `design`, `plan/maintenance/tools`):

- `phase_fixed` in `ChannelModel` and `RxChannelModel` (`channel.rs`), its two initialisers in `prepare` and the `.set` in `untimed_tune`.
- `update_layout_segment_coherence` and its call: with a constant basis per segment it could only rewrite entries equal to the one in force.
- In `layout_phase_basis`: `is_fixed`, `timed`, the `random_unless_timed_tune` arm and the `timed_tune` return. The basis is `deterministic` if every channel is deterministic, else `none` (`unmodelled` as before). Its `at` parameter had no reader left and went too (one caller, `record_layout_segment`).
- Doc comment of `untimed_tune` corrected. Spec: MR-34 drops "not coherent until its next timed tune"; MR-27 already says `unmodelled`, `none`, `deterministic` and was left; one clause added to 09's stage 2 Changes row.

Mutation rows: P170 deleted (target gone). P178 and P183 stay and still match once (`rules.py` applies every row); probed in a copy outside the repository: P178 killed, P183 killed (`work/fix2-probes/P178.log`, `P183.log`; unmutated `mr_34` baseline passes, `baseline.log`). No new tests: nothing observable changed (the coherence of a segment was already `deterministic` or `none`, pinned by `mr_34_a_layout_deterministic_channel_keeps_its_phase_and_coherence_through_entry` and `mr_27_a_segment_row_holds_its_channel_order_and_its_end`).

Result: `cargo fmt --all`, clippy `-D warnings` (workspace, all targets), `cargo test --workspace` and the Python 3.14 suite (78) pass; `rules.py --base HEAD --renames plan/phase8/reviews/47s2-renames.txt` 0 problems.

# Rev 9 — gate fix round (17 mutation rows; rev 8 kept in `test-plan-s2.v8.md`)

The gate (`gate-s2/gate.json`) failed on 17 rows: 4 COMPILE ERROR (P392, P445, P464, P469) and 13 SURVIVED (J01, J02, J05, J08, J12, P136, P368, P539, P540, M30, M53, M54, M66). Each was decided after reading the code and the HEAD killer. Probes ran in a throwaway copy outside the repository (`/tmp/.../scratchpad/gfcopy`, built into `~/.cache/cargo-target/Ez-SDRv4-review`, `CARGO_INCREMENTAL=0`, one build at a time), by `work/gate-fix-probes/gfprobe.py`; one log per row and run in `work/gate-fix-probes/`. A compile failure counts as not killed. "Wide" is `cargo test --workspace --no-fail-fast` on the mutated copy: every test binary of every crate (the Mock and `ezsdr-radio` timeline differentials, acceptance `parity`, `v58`, `v61`, `x3xx`, `reactive`, `uhd`, server `protocol`, the unit tests), 1 697 tests; the Python suites matter only for the two Python rows, which a test kills.

## 14. Re-anchored (the code around the anchor moved; the behaviour stays)

| Row | Change | Killer (filter run, mutant FAILS) |
|---|---|---|
| P392 | `Holder::Free` is not imported in `tx.rs`: `super::core::Holder::Free` | `ur_25_a_detached_control_does_not_hold_the_shutdown_s_read` (lib): "the shutdown's read waited for a detached control" |
| P445 | `dir_of`/`booked_stream(..)` are gone: `self.first_undelivered((stream.0, 0))` | `ur_26_an_end_takes_delivered_from_each_segment_s_own_stream` (lib) |
| P464 | same: `self.lines.refuse((stream.0, 0), seq)` | `ur_25_a_successful_start_clears_a_later_stream_s_halt` (fake) |
| P469 | `config_at(..)` is gone: `stream_channels(&self.config, true, k as u32)` | `ur_26_a_start_rx_of_a_channel_less_linked_stream_starts_nothing` (fake) |

## 15. New or extended tests (OWED: the behaviour is reachable, or on the must-survive list, and lost its pin)

| Row | Behaviour | Test and assertion | Mutant result |
|---|---|---|---|
| J01 | a receive segment cut with no sample is left out of the plan (RM-25, VH-7) | `rm_26_the_timeline_cases` (`crates/ezsdr-radio/tests/timeline.rs`), new case "a Stop at the Run's start leaves a receive stream no segment": `[item(0, 0, 0, Stop)]` plans `[]` (the mutant plans `[(0, Some(0))]`). `Stop` at the Run's start is reachable. | FAIL |
| J05 | a cut is never before sample 0 (RM-16) | same test, new case "a Stop before a restart begins leaves it out, and the next start follows its origin": Stop 1 000 ms, `start_rx` 1 000 ms (origin 1 050 ms), Stop 1 020 ms (the restart has not begun: cut at its origin), `start_rx` 1 030 ms plans `[(0, Some(1 000 ms)), (1 100 ms, None)]`; without the clamp the previous cut is 1 020 ms and the origin 1 080 ms. A Stop reaching a pending restart is reachable. The same case also fails J01's mutant. | FAIL (`left: [.. (1080000000, None)]`) |
| J08 | the fault row records `k_g - k_f`, not what survives a cut (VH-5) | `mr_21_overrun_shape` (`mock_radio.rs`), last section: the `Stop` is now pushed before the step that applies the fault, so it is handled at 20 ms ahead of the fault at 1 ms (the earlier form stepped to 20 ms first, the fault was applied before the cut was booked, and the section tested nothing). The row stays 50 000 (mutant: 19 000). | FAIL (`left: 19000, right: 50000`) |
| P136 | each transmit stream books a line of its own channels (MR-9, MR-27) | `mr_09_each_transmit_stream_has_its_own_clock_and_line` (`mock_layout.rs`): the two `tx` rows of `segments` are `[(0, [0]), (1, [1])]` (the mutant gives stream 1 stream 0's line: `[0]`). The clock registration reads the stream's own set, which is why the clocks assertion never saw the mutant. | FAIL |
| M53 | a transmitted sample's gain, frequency and phase are read at the sample's own instant, not the antenna's (MR-32, MR-33) | new unit test `mr_32_a_radiated_sample_takes_the_values_in_force_at_its_own_instant` (`crates/ezsdr-mock-radio/src/channel.rs`): `TxShared::radiated` of a one-channel segment with `path_delay_samples` 2 and a change of all three timelines at tick 3 000; at antenna 4 900 the sample is at tick 2 000 (before the change: `(1.0, 0.0, 100.0)`), at 5 100 at tick 3 000 (at the change: im 10, frequency 200). No production path changes a transmit timeline now, so a direct unit test is the only form. | FAIL |
| P368 | the owner does not configure a segment again when its record holds that command (UR-36, spec 18: "is not configured again") | new pure unit test `ur_36_the_owner_step_follows_the_record` (`provider/core.rs`): open/not open, set differs, command equal (`Nothing`), command differs, no command (`Configure`). The arm is reached in production only by a repeated `owner_configure` of one command, which `begin` does not make (`self.from` moves on); only the direct call in `ur_29_a_begin_rechecks_loss_after_acquiring_the_chain_before_rx_start` meets it, and that test does not count device calls. See the open item below. | FAIL (`left: Configure, right: Nothing`) |
| P539, P540 | `Session.rx` / `Session.tx` are stream 0 (EA-16) | `test_ea_16_radio_side_is_stream_zero` (`python/tests/test_easy_api.py`): the streams are connected with different rates (stream 0 at 2 MS/s, stream 1 at 20 MS/s; the setters it used to use are refused now) and `sdr.rx.sample_rate`, `sdr.tx.sample_rate` are 2 MS/s. | FAIL (`(20000000.0, 2000000.0)`, and `(2000000.0, 20000000.0)`) |

## 16. Retargeted (a surviving test kills it)

M30 and M54 (`Timeline::at` ignores the tick; takes the first change, not the last): the unit test `mr_33_a_timeline_keeps_the_value_in_force_at_each_instant` (`channel.rs`, added in part (d)) fails both; rows set to test_bin `lib` with that filter (their old filter, `mr_33_gain` and the whole `mock_channel`, never read a changing history after the Run-time changes went). M53 and P368 above are retargeted to their new tests too.

## 17. EQUIVALENT (the mutant cannot change behaviour now; the row is deleted)

Every one survived the wide run (mutant applied in the copy, `cargo test --workspace --no-fail-fast`, 1 697 passed, 0 failed; logs `J02.ws.log`, `J12.ws.log`, `M66.ws.log`).

- **J02** `earliest = earliest.max(cut + lead)` removed. `begin` is called only for a receive `start_rx` (the `Kind::Start if receive` arm; search `\.begin(` and `fn begin` in `crates`: one caller), and there `ready` is raised to at least `cut` (release window, call and allowance are not negative), so `ready + lead >= cut + lead` already. Only a transmit restart, a deleted case, was ever bound by the cut term. The line stays: spec 07 RM-25 names "the previous cut plus the lead" as a term of the rule, and it is cheap.
- **J12** `ready + lead` taken by receive only: same search; `begin` has no transmit caller. The `if stream.direction == Direction::Rx` around the call-and-release-window term was dead for the same reason and is removed (`timeline.rs`, `begin`; doc comment says only a receive `start_rx` begins a segment).
- **M66** the receive frequency read at the start of time: no `frequency_hz` Timeline gets a change any more (search `frequency_hz.set` and `\.set(` in `crates/ezsdr-mock-radio/src`: the only `Timeline::set` call is the receive phase redraw of a restart's untimed tune), so `at(stamp)` is the initial value at every instant. A gain-at-start-of-time mutant also survives the Mock crate's tests for the same reason. The use of a changing history at the sample's instant is pinned through the one history that changes, the receive phase: the new row **P1400** (`model.phase.at(stamp)` read at the start of time) is killed by `mr_34_a_restart_redraws_each_random_channel_in_channel_order` (filter run; the crate's other `mr_34` tests fail too). A receive read at the antenna's instant instead of the sample's cannot show: `RX_PATH_DELAY` is 0 on every profile.

## 18. Result

`cargo fmt --all`; clippy `-D warnings` workspace (all targets) and `ezsdr-radio-uhd --features uhd --all-targets` (rdev) clean; `cargo test --workspace` 1 698 passed, 0 failed (gate: 1 696, plus the two new unit tests); uhd `--lib --test uhd_api` (rdev) 326 passed (gate: 325); Python 3.14 suite 78 passed; `rules.py --base HEAD --renames plan/phase8/reviews/47s2-renames.txt` 0 problems (every row, new and re-anchored, matches once). The new tests are pure (no threads, no clock), so no 20-run repeat. Rows: 15 kept or retargeted or re-anchored (`P392`, `P445`, `P464`, `P469`, `J01`, `J05`, `J08`, `P136`, `P368`, `P539`, `P540`, `M30`, `M53`, `M54`), 3 deleted (`J02`, `J12`, `M66`), 1 new (`P1400`). Spec: three `*Test:*` lines gained a citation (MR-32, MR-33, UR-36); no rule text changed.

Open items for the orchestrator, not changed here: (1) `Owner::Nothing` and the `configured == by` comparison are reached by no production sequence now (see P368 above); the record's `configured` mark is read only there. Removing the mark is a larger cleanup (UR-36's last sentence, `record_configured`, `tx.rs` restore, the tests of the mark) and the owner's must-survive list names the owner's configuration, so it stays until stage 3 shows whether staged values need it. (2) J02's `cut + lead` term is redundant with the ready term; kept because the rule text states it.
