# Rows touched in note 47 stage 2 part (c1)

Decisions: DELETE 432, RETARGET 86 (85 Rust rows + P12), NEW 7, KEEP J32 (named in the brief; its Kernel killer `kc_28_an_untimed_burst_is_not_admitted_before_its_clock_s_origin` is a live test, not re-run). Probe logs: `~/.cache/ezsdr-fixes/2026-11-04-n46/work/c1-probes/` (`<list>-<id>.<mode>.<crate>.<bin>.log`; `full` = the row's own test binary, `wide` = the crate's other binaries plus acceptance parity and v58, `filter` = the final single-test check: all 91 retargeted/new Rust rows were killed). Probes ran in a throwaway copy `/tmp/c1work/copy`, never in the repository; helper scripts are in `work/c1/`.

Format: id | list | decision | reason | killing test and the assertion that fails on the mutant (path:line).

F28 | maintenance | DELETE | pending-update queue (`updates`) deleted; a loss has no update to reject | 
F29 | maintenance | RETARGET | re-anchored to today's code | `mock-radio|mock_radio::mr_20_a_loss_ends_its_receive_stream_at_its_instant` fails at ezsdr-mock-radio/tests/mock_radio.rs:1795:5: assertion `left == right` failed left: [(0, None)] right: [(0, Some(1100000))]
F36 | maintenance | DELETE → RE-ADDED | re-added in part (d): `ezsdr-mock-radio --test timeline rm_26_the_timeline_against_mockradio` (tests/timeline.rs:192) kills it (the reviewer found it in the Mock-vs-oracle differential that part (c1) never ran) | 
F37 | maintenance | DELETE | `Kind::Cold` booking in `handle_update` deleted (it only refuses now) | 
F39 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
F40 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|fake::ur_25_a_receive_cut_counts_its_own_stream_s_packet_size` fails at ezsdr-radio-uhd/tests/fake.rs:6363:5: 4000 samples at 200 ticks + 3 ms release window + 3 ms allowance + 50 ms lead, rounded to the lattice: 2000088 in {"rx":[{"channels":[1],"end":4208505
J06a | maintenance | DELETE | `sorted`'s `effective` floor deleted; `sorted` no longer rebuilds items | 
J06b | maintenance | RETARGET | re-anchored to today's code | `radio|timeline::rm_26_the_timeline_cases` fails at ezsdr-radio/tests/timeline.rs:124:9: assertion `left == right` failed: a loss before a transmit clock's origin ends it at its origin left: [(2000000000, None)] right: [(2000000000, Some(2
J11 | maintenance | DELETE | `settle` of held timed updates deleted; Stop books no cancellation of updates | 
J16 | maintenance | DELETE | `do_switch` deleted by part (a)/(b) (killer test gone too) | 
J185 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|fake::ur_21_txburst_refusals` fails at ezsdr-radio-uhd/tests/fake.rs:2220:5: [Event { source: Node { node: ResourceId { node: NodeId(0), path: "usrp" } }, time: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 2 }, t
J19 | maintenance | DELETE | `adopt_later` deleted by part (a)/(b) (killer test gone too) | 
J194 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
J20 | maintenance | DELETE | no surviving test kills the mutant (own bin, all bins of the crate, acceptance parity and v58; probe logs c1-probes/) | 
J21 | maintenance | DELETE | DEAD, region deleted in part (d): the tail loop of `Streams::reconcile` (the whole of `reconcile` is deleted; see the part (d) table) | 
J294 | maintenance | DELETE | TIMEOUT: the mutant hangs the uhd lib tests (go_idle removed); no named test fails | 
J32 | maintenance | KEEP | consolidates J33 and J34; Kernel test is live | `kernel|coordinator::kc_28_an_untimed_burst_is_not_admitted_before_its_clock_s_origin`
J33 | maintenance | DELETE | consolidated into J32 (Kernel kc_28 kills the same pipeline.rs mutant); probe: v58 survived | 
J34 | maintenance | DELETE | consolidated into J32; probe: radio-uhd fake survived (log maintenance-J34.full.ezsdr-radio-uhd.fake.log) | 
J50 | maintenance | DELETE | `prune` branch of `plan_section` and the switch deleted | 
J59 | maintenance | RETARGET | re-anchored to today's code | `radio-uhd|lib::ur_25_uhd_tx_reads_the_plan_again_when_it_changes` fails at ezsdr-radio-uhd/src/provider/tx.rs:1861:9: assertion `left == right` failed: the clock that has not begun ends at its origin left: None right: Some(200000000)
J61 | maintenance | DELETE | `sorted` has no requested-instant variant (`effective` deleted); the mutant equals the code | 
J62 | maintenance | DELETE | no surviving test kills the mutant (own bin, all bins of the crate, acceptance parity and v58; probe logs c1-probes/) | 
J65 | maintenance | RETARGET | filter changed (old killer gone) | `radio|timeline::rm_25_a_refused_segment_runs_for_planning_until_the_next_start_cuts_it` fails at ezsdr-radio/tests/timeline.rs:518:5: halted until S2 takes effect
J69 | maintenance | DELETE | `book_timed` deleted by part (a)/(b) (killer test gone too) | 
J70 | maintenance | RETARGET | filter changed (old killer gone) | `radio|timeline::rm_25_a_refused_segment_runs_for_planning_until_the_next_start_cuts_it` fails at ezsdr-radio/tests/timeline.rs:518:5: halted until S2 takes effect
P100 | maintenance | DELETE | `book_move` deleted by part (a)/(b) (killer test gone too) | 
P101 | maintenance | DELETE | `book_move` deleted by part (a)/(b) (killer test gone too) | 
P102 | maintenance | DELETE | move `settle` deleted | 
P1022 | maintenance | DELETE | `covering` deleted by part (a)/(b) (killer test gone too) | 
P1023 | maintenance | DELETE | `covering` deleted by part (a)/(b) (killer test gone too) | 
P1024 | maintenance | DELETE | `covering` deleted by part (a)/(b) (killer test gone too) | 
P1025 | maintenance | DELETE | `covering` deleted by part (a)/(b) (killer test gone too) | 
P1026 | maintenance | DELETE | `apply_layout_channel_update` deleted by part (a)/(b) (killer test gone too) | 
P1027 | maintenance | DELETE | `apply_layout_channel_update` deleted by part (a)/(b) (killer test gone too) | 
P1028 | maintenance | DELETE | `apply_layout_channel_update` deleted by part (a)/(b) (killer test gone too) | 
P1029 | maintenance | DELETE | `covering_rx` deleted by part (a)/(b) (killer test gone too) | 
P103 | maintenance | DELETE | `prune_requested` deleted by part (a)/(b) (killer test gone too) | 
P1030 | maintenance | DELETE | `covering_rx` deleted by part (a)/(b) (killer test gone too) | 
P1031 | maintenance | DELETE | `apply_layout_channel_update` deleted by part (a)/(b) (killer test gone too) | 
P1032 | maintenance | DELETE | `apply_layout_channel_update` deleted by part (a)/(b) (killer test gone too) | 
P1033 | maintenance | DELETE | `apply_layout_channel_update` deleted by part (a)/(b) (killer test gone too) | 
P1034 | maintenance | DELETE | `apply_layout_channel_update` deleted by part (a)/(b) (killer test gone too) | 
P1035 | maintenance | DELETE | `reach` deleted by part (a)/(b) (killer test gone too) | 
P1036 | maintenance | DELETE | `apply_layout_channel_update` deleted by part (a)/(b) (killer test gone too) | 
P1037 | maintenance | DELETE | `apply_layout_channel_update` deleted by part (a)/(b) (killer test gone too) | 
P104 | maintenance | DELETE | `effective` deleted by part (a)/(b) (killer test gone too) | 
P105 | maintenance | DELETE | `settle_once` deleted by part (a)/(b) (killer test gone too) | 
P1052 | maintenance | RETARGET | re-anchored to today's code, filter changed (old killer gone) | `radio|timeline::rm_25_a_refused_segment_runs_for_planning_until_the_next_start_cuts_it` fails at ezsdr-radio/tests/timeline.rs:511:9: assertion `left == right` failed: S2 inside S1's lead left: [(0, Some(500000000)), (1060000000, None)] right: [(0, Some(500000000)), (1100000000, None
P1053 | maintenance | DELETE | identical mutant of P1052 | 
P1054 | maintenance | DELETE | identical mutant of P1052 | 
P1055 | maintenance | DELETE | identical mutant of P1052 | 
P1057 | maintenance | DELETE | identical mutant of P1058 | 
P1058 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|lib::rm_26_the_timeline_against_uhd_rx` fails at ezsdr-radio-uhd/src/provider/rx/differential.rs:851:9: uhd-rx makes no progress
P106 | maintenance | DELETE | `set_release` deleted by part (a)/(b) (killer test gone too) | 
P1060 | maintenance | DELETE | `released` deleted by part (a)/(b) (killer test gone too) | 
P1061 | maintenance | DELETE | `released` deleted by part (a)/(b) (killer test gone too) | 
P1062 | maintenance | DELETE | `released` deleted by part (a)/(b) (killer test gone too) | 
P1065 | maintenance | DELETE | refused transmit (cold) change deleted | 
P1066 | maintenance | DELETE | refused transmit (cold) change deleted | 
P1067 | maintenance | DELETE | refused transmit (cold) change deleted | 
P107 | maintenance | DELETE | `item.release` deleted | 
P1075 | maintenance | DELETE | `reach` deleted by part (a)/(b) (killer test gone too) | 
P1076 | maintenance | DELETE | refused transmit (cold) change deleted | 
P1079 | maintenance | DELETE | moves and refused transmit change deleted | 
P108 | maintenance | DELETE | `prune_requested` deleted by part (a)/(b) (killer test gone too) | 
P1080 | maintenance | DELETE | moves and refused transmit change deleted | 
P1081 | maintenance | DELETE | moves and refused transmit change deleted | 
P109 | maintenance | DELETE | `book_move` deleted by part (a)/(b) (killer test gone too) | 
P110 | maintenance | DELETE | `settle_once` deleted by part (a)/(b) (killer test gone too) | 
P111 | maintenance | DELETE | `moves` deleted | 
P1112 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|lib::ur_23_m14_a_cut_is_not_waited_past_or_crossed` fails at ezsdr-radio-uhd/src/provider/tx.rs:4827:9: assertion `left == right` failed left: (2500, false, false) right: (2200, false, false)
P112 | maintenance | DELETE | `settle_once` deleted by part (a)/(b) (killer test gone too) | 
P113 | maintenance | DELETE | `raise_floor` deleted by part (a)/(b) (killer test gone too) | 
P1133 | maintenance | DELETE | `dispatch_delay` deleted by part (a)/(b) (killer test gone too) | 
P1139 | maintenance | DELETE | `h4_gain_action` deleted by part (a)/(b) (killer test gone too) | 
P114 | maintenance | DELETE | `book_move` deleted by part (a)/(b) (killer test gone too) | 
P1142 | maintenance | DELETE | DEAD, region deleted in part (d): the `host_entry_ns` field has no reader | 
P1143 | maintenance | DELETE | `measure_h4_dispatch` deleted by part (a)/(b) (killer test gone too) | 
P1144 | maintenance | DELETE | `measure_h4_dispatch` deleted by part (a)/(b) (killer test gone too) | 
P1148 | maintenance | DELETE | re-anchored mutant survives (H4 root-tick conversion is not observed by h4 tests) | 
P1149 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|fake::h4_a_tx_underflow_makes_the_load_record_partial_and_converts_its_time` fails at ezsdr-radio-uhd/tests/fake.rs:7441:5: {"clock_note":"host/device affine mapping is INFERRED; anchors must agree with the master clock within 1,000 ppm plus measured bracket uncertainty; br
P115 | maintenance | DELETE | `book_move` deleted by part (a)/(b) (killer test gone too) | 
P1150 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|fake::h4_a_tx_underflow_makes_the_load_record_partial_and_converts_its_time` fails at ezsdr-radio-uhd/tests/fake.rs:7444:5: assertion `left == right` failed: an underflow-terminated repeat cannot be complete left: String("complete") right: "partial"
P1151 | maintenance | DELETE | `tx_overlap` deleted by part (a)/(b) (killer test gone too) | 
P1154 | maintenance | DELETE | `matching_timed_apply` deleted by part (a)/(b) (killer test gone too) | 
P1155 | maintenance | DELETE | `measure_h4_dispatch` deleted by part (a)/(b) (killer test gone too) | 
P1156 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|fake::h4_a_tx_underflow_makes_the_load_record_partial_and_converts_its_time` fails at ezsdr-radio-uhd/tests/fake.rs:7441:5: {"clock_note":"host/device affine mapping is INFERRED; anchors must agree with the master clock within 1,000 ppm plus measured bracket uncertainty; br
P1157 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|fake::h4_a_tx_underflow_makes_the_load_record_partial_and_converts_its_time` fails at ezsdr-radio-uhd/tests/fake.rs:7441:5: {"clock_note":"host/device affine mapping is INFERRED; anchors must agree with the master clock within 1,000 ppm plus measured bracket uncertainty; br
P1158 | maintenance | DELETE | `measure_h4_dispatch` deleted by part (a)/(b) (killer test gone too) | 
P116 | maintenance | DELETE | `released` deleted by part (a)/(b) (killer test gone too) | 
P1169 | maintenance | DELETE | no surviving test kills the mutant (own bin, all bins of the crate, acceptance parity and v58; probe logs c1-probes/) | 
P117 | maintenance | DELETE | `released` deleted by part (a)/(b) (killer test gone too) | 
P1170 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|fake::ur_23_a_burst_at_the_first_safe_sample_after_a_held_tail_close_is_played` fails at ezsdr-radio-uhd/tests/fake.rs:3247:9: [Sent { n: 60, at: Some(403603800), sob: true, eob: false }, Sent { n: 2500, at: None, sob: false, eob: false }, Sent { n: 2500, at: None, sob: false,
P1176 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|lib::ur_22_arm_waits_for_t3_flush_ack_before_timed_tx_dsp` fails at ezsdr-radio-uhd/src/provider/core.rs:1400:5: the opener must hold the timed-DSP gate through its flush acknowledgement
P118 | maintenance | DELETE | `released` deleted by part (a)/(b) (killer test gone too) | 
P119 | maintenance | DELETE | `began_before` deleted by part (a)/(b) (killer test gone too) | 
P1193 | maintenance | RETARGET | re-anchored to today's code | `radio-uhd|lib::ur_22_uncertain_reopen_waits_for_owner_claim_and_stops_on_stop_or_loss` fails at ezsdr-radio-uhd/src/provider/tx.rs:1329:9: a held stream claim defers the reopen for an idle turn
P1199 | maintenance | RETARGET | re-anchored to today's code | `radio-uhd|lib::ur_22_tx_stop_preserves_recovery_for_later_bursts` fails at ezsdr-radio-uhd/src/provider/tx.rs:1674:13: ordinary Stop cleared the recovery requirement (failed close=true)
P120 | maintenance | DELETE | `released` deleted by part (a)/(b) (killer test gone too) | 
P1206 | maintenance | RETARGET | re-anchored to today's code | `radio-uhd|lib::ur_22_nonterminal_reopen_failures_cancel_held_initial_bursts` fails at ezsdr-radio-uhd/src/provider/tx.rs:1614:13: the held burst was cancelled (TxOpenFails(1))
P1207 | maintenance | DELETE | `plan_read` deleted | 
P121 | maintenance | DELETE | `released` deleted by part (a)/(b) (killer test gone too) | 
P1211 | maintenance | DELETE | `rx_drain_deadline` deleted by part (a)/(b) (killer test gone too) | 
P1212 | maintenance | DELETE | `of_record` deleted by part (a)/(b) (killer test gone too) | 
P1213 | maintenance | DELETE | Provider::stop frequency restore deleted | 
P1214 | maintenance | DELETE | `consume` deleted by part (a)/(b) (killer test gone too) | 
P1215 | maintenance | DELETE | `consume` deleted by part (a)/(b) (killer test gone too) | 
P1216 | maintenance | DELETE | `give` deleted by part (a)/(b) (killer test gone too) | 
P1217 | maintenance | DELETE | `hand` deleted by part (a)/(b) (killer test gone too) | 
P1219 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|lib::rm_26_the_timeline_against_uhd_control` fails at ezsdr-radio-uhd/src/provider/control/differential.rs:95:5: 291 sequences differ (EZSDR_TIMELINE_SEED=<seed> shows one), the first: [ "seed 0, Loss: [\"rx_clocks\"]", "seed 1, StartRx: [\"rx_clocks\"]", "seed 3
P1220 | maintenance | DELETE → RE-ADDED | re-added in part (d): same killer as F36 | 
P1222 | maintenance | DELETE | Provider::stop frequency restore deleted | 
P1223 | maintenance | DELETE | Provider::stop RX restore deleted | 
P1225 | maintenance | DELETE | `flush_rx` deleted by part (a)/(b) (killer test gone too) | 
P1226 | maintenance | DELETE | `report_debt` deleted by part (a)/(b) (killer test gone too) | 
P1227 | maintenance | DELETE | `flush_rx` deleted by part (a)/(b) (killer test gone too) | 
P1228 | maintenance | DELETE | `give` deleted by part (a)/(b) (killer test gone too) | 
P1229 | maintenance | DELETE | Provider::stop RX restore deleted | 
P1231 | maintenance | DELETE | no surviving test kills the mutant (own bin, all bins of the crate, acceptance parity and v58; probe logs c1-probes/) | 
P1232 | maintenance | DELETE | RX debt/drain at stop deleted | 
P1233 | maintenance | DELETE | `give` deleted by part (a)/(b) (killer test gone too) | 
P1234 | maintenance | DELETE | RX debt/drain at stop deleted | 
P1235 | maintenance | DELETE | `flush_rx` deleted by part (a)/(b) (killer test gone too) | 
P1236 | maintenance | DELETE | `bounded_read_len` deleted by part (a)/(b) (killer test gone too) | 
P1238 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|lib::ur_24_a_cut_receive_uses_one_short_packet_and_preserves_successor_lead` fails at ezsdr-radio-uhd/src/provider/rx.rs:900:9: assertion `left == right` failed: the one-packet receive returned after 500 samples and delivery latency left: 14500000 right: 10600000
P1241 | maintenance | DELETE | `report_debt` deleted by part (a)/(b) (killer test still exists but cannot fail) | 
P1243 | maintenance | DELETE | `give` deleted by part (a)/(b) (killer test gone too) | 
P1244 | maintenance | DELETE | no surviving test kills the mutant (own bin, all bins of the crate, acceptance parity and v58; probe logs c1-probes/) | 
P1245 | maintenance | DELETE | `eligible` deleted by part (a)/(b) (killer test gone too) | 
P1247 | maintenance | DELETE | `hand` deleted by part (a)/(b) (killer test gone too) | 
P1248 | maintenance | DELETE | `hand` deleted by part (a)/(b) (killer test gone too) | 
P1249 | maintenance | DELETE | `consume` deleted by part (a)/(b) (killer test gone too) | 
P1250 | maintenance | DELETE | `restore_rx` deleted by part (a)/(b) (killer test gone too) | 
P1251 | maintenance | DELETE | `restore_rx` deleted by part (a)/(b) (killer test gone too) | 
P1252 | maintenance | DELETE | Provider::stop RX restore deleted | 
P1254 | maintenance | DELETE | `flush_start_result` deleted by part (a)/(b) (killer test gone too) | 
P1255 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|lib::rx_chain_reuse_guard_rejects_open_and_close_of_an_active_chain` fails at ezsdr-radio-uhd/src/provider/core.rs:1863:9: an Armed chain cannot be reopened
P1257 | maintenance | DELETE | `release` deleted by part (a)/(b) (killer test gone too) | 
P1258 | maintenance | DELETE | `release` deleted by part (a)/(b) (killer test gone too) | 
P1259 | maintenance | DELETE | no surviving test kills the mutant (own bin, all bins of the crate, acceptance parity and v58; probe logs c1-probes/) | 
P1263 | maintenance | DELETE | `report_debt` deleted | 
P1265 | maintenance | DELETE | `give` deleted by part (a)/(b) (killer test gone too) | 
P1266 | maintenance | DELETE | `give` deleted by part (a)/(b) (killer test gone too) | 
P1267 | maintenance | DELETE | identical mutant of P1268 | 
P1268 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|lib::rx_chain_restart_transitions` fails at ezsdr-radio-uhd/src/provider/core.rs:1505:17: assertion `left == right` failed left: None right: Some(SegmentKey { by: Some(7), origin: 100 })
P1269 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|lib::ur_25_a_stream_silent_before_e1_switches_before_e2` fails at ezsdr-radio-uhd/src/provider/rx.rs:1981:9: a stream silent past its cut is ended
P1270 | maintenance | DELETE | `hand` deleted by part (a)/(b) (killer test gone too) | 
P1273 | maintenance | DELETE | `release` deleted by part (a)/(b) (killer test gone too) | 
P1274 | maintenance | DELETE | `release` deleted by part (a)/(b) (killer test gone too) | 
P1275 | maintenance | DELETE | `release` deleted by part (a)/(b) (killer test gone too) | 
P1276 | maintenance | DELETE | `release` deleted by part (a)/(b) (killer test gone too) | 
P1279 | maintenance | DELETE | `settle` deleted | 
P128 | maintenance | DELETE | `cold_streams` deleted by part (a)/(b) (killer test gone too) | 
P1280 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|lib::rx_chain_reuse_guard_rejects_open_and_close_of_an_active_chain` fails at ezsdr-radio-uhd/src/provider/core.rs:1315:9: L1: RX open requires an idle device chain, got Armed { at: 0 } thread 'provider::core::tests::rx_chain_reuse_guard_rejects_open_and_close_of_an_active
P1281 | maintenance | DELETE | identical mutant of P1280 | 
P1282 | maintenance | DELETE | identical mutant of P1280 | 
P1283 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|lib::rx_chain_prepare_open_requires_an_idle_chain` fails at ezsdr-radio-uhd/src/provider/mod.rs:826:9: arm cannot open a streamer on an active chain
P1284 | maintenance | DELETE | identical mutant of P1280 | 
P1285 | maintenance | DELETE | identical mutant of P1283 | 
P1286 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|lib::rx_chain_cleanup_close_requires_an_idle_chain` fails at ezsdr-radio-uhd/src/provider/mod.rs:837:9: cleanup cannot close streams while an RX chain is active
P1287 | maintenance | DELETE | `restore_rx` deleted by part (a)/(b) (killer test gone too) | 
P1289 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|lib::rx_chain_first_sample_transition` fails at ezsdr-radio-uhd/src/provider/core.rs:1520:9: assertion `left == right` failed left: None right: Some(SegmentKey { by: Some(7), origin: 100 })
P1290 | maintenance | DELETE → RE-ADDED | re-added in part (d), new unit `ur_24_a_missed_start_after_a_loss_makes_no_restart_call` (rx.rs) | 
P1293 | maintenance | DELETE | `release` deleted by part (a)/(b) (killer test gone too) | 
P1294 | maintenance | DELETE | `give` deleted by part (a)/(b) (killer test gone too) | 
P130 | maintenance | DELETE | `update_class` deleted by part (a)/(b) (killer test still exists but cannot fail) | 
P1300 | maintenance | DELETE | `do_switch` deleted by part (a)/(b) (killer test gone too) | 
P1301 | maintenance | DELETE | `do_switch` deleted by part (a)/(b) (killer test gone too) | 
P1308 | maintenance | DELETE | `next_segment_adopted` deleted | 
P131 | maintenance | DELETE | `cold_streams` deleted by part (a)/(b) (killer test gone too) | 
P1316 | maintenance | DELETE | `earliest_tx_end` deleted by part (a)/(b) (killer test gone too) | 
P1317 | maintenance | DELETE | `map_dispatch_row` deleted by part (a)/(b) (killer test gone too) | 
P1318 | maintenance | DELETE | `matching_timed_apply` deleted by part (a)/(b) (killer test gone too) | 
P135 | maintenance | RETARGET | filter changed (old killer gone) | `acceptance|v58::v58_16_a_rate_beyond_the_envelope_is_refused_at_connect_and_a_runtime_change_has_no_class` fails at ezsdr-acceptance/tests/v58.rs:558:5: Running
P1387 | maintenance | NEW | channel frequency is a classless key | `radio|radio_model::rm_04_the_key_table_is_exactly_the_declared_one` (its expected table lists every radio key with no class)
P1388 | maintenance | NEW | channel gain is a classless key | `radio|radio_model::rm_04_the_key_table_is_exactly_the_declared_one` (its expected table lists every radio key with no class)
P1389 | maintenance | NEW | channel antenna is a classless key | `radio|radio_model::rm_04_the_key_table_is_exactly_the_declared_one` (its expected table lists every radio key with no class)
P1390 | maintenance | NEW | channel stream membership is a classless key | `radio|radio_model::rm_04_the_key_table_is_exactly_the_declared_one` (its expected table lists every radio key with no class)
P1391 | maintenance | NEW | stream sample rate is a classless key | `radio|radio_model::rm_04_the_key_table_is_exactly_the_declared_one` (its expected table lists every radio key with no class)
P1392 | maintenance | NEW | the UHD Provider accepts a Run-time update of a radio key instead of refusing it | `radio-uhd|fake::ur_26_an_unnumbered_key_is_refused_at_runtime` (no COMMAND_REJECTED arrives, so its `until` fails)
P1393 | maintenance | NEW | a radio value setter submits SetParameter instead of refusing during a Run | `test_easy_api.EasyApi.test_v3_parameters_through_attributes` fails at python/tests/test_easy_api.py:156 (`connect(rx=ezsdr.Stream(` not in the error text; the Kernel `update_class` Rejected comes instead)
P141 | maintenance | DELETE | its code is already gone (deleted by part (b); the anchor matched nothing before part (c1) either), not "no surviving test kills it" | 
P142 | maintenance | DELETE | its code is already gone (deleted by part (b); the anchor matched nothing before part (c1) either), not "no surviving test kills it" | 
P143 | maintenance | DELETE | its code is already gone (deleted by part (b); the anchor matched nothing before part (c1) either), not "no surviving test kills it" | 
P146 | maintenance | RETARGET | re-anchored to today's code | `mock-radio|mock_layout::mr_16_each_transmit_stream_is_judged_by_its_own_channel_count` fails at ezsdr-mock-radio/tests/mock_layout.rs:884:5: assertion `left == right` failed: ["MR-16: target, clock, waveform size, channel count or metadata is invalid", "MR-16: target, clock, waveform size, 
P147 | maintenance | RETARGET | re-anchored to today's code | `mock-radio|mock_layout::mr_16_each_transmit_stream_is_judged_by_its_own_channel_count` fails at ezsdr-mock-radio/tests/mock_layout.rs:884:5: assertion `left == right` failed: [] left: 0 right: 1
P148 | maintenance | DELETE | its code is already gone (deleted by part (b); the anchor matched nothing before part (c1) either), not "no surviving test kills it" | 
P149 | maintenance | DELETE | its code is already gone (deleted by part (b); the anchor matched nothing before part (c1) either), not "no surviving test kills it" | 
P150 | maintenance | DELETE | its code is already gone (deleted by part (b); the anchor matched nothing before part (c1) either), not "no surviving test kills it" | 
P151 | maintenance | DELETE | its code is already gone (deleted by part (b); the anchor matched nothing before part (c1) either), not "no surviving test kills it" | 
P152 | maintenance | DELETE | its code is already gone (deleted by part (b); the anchor matched nothing before part (c1) either), not "no surviving test kills it" | 
P153 | maintenance | DELETE → RE-ADDED | re-added in part (d), assertions added to `mr_34_a_layout_unlinked_receive_stream_takes_no_untimed_tune` (mock_layout.rs) | 
P154 | maintenance | DELETE → RE-ADDED | re-added in part (d), new test `mr_27_a_segment_row_holds_its_channel_order_and_its_end` (mock_layout.rs) | 
P155 | maintenance | RETARGET | filter changed (old killer gone) | `mock-radio|mock_layout::mr_33_layout_gain_and_frequency_timelines_are_per_logical_channel` fails at ezsdr-mock-radio/src/channel.rs:97:39: index out of bounds: the len is 200 but the index is 200
P156 | maintenance | RETARGET | filter changed (old killer gone) | `mock-radio|mock_layout::mr_33_layout_gain_and_frequency_timelines_are_per_logical_channel` fails at ezsdr-mock-radio/tests/mock_layout.rs:978:5: 0+0i
P157 | maintenance | DELETE | `apply_layout_channel_update` deleted by part (a)/(b) (killer test still exists but cannot fail) | 
P158 | maintenance | RETARGET | re-anchored to today's code, filter changed (old killer gone) | `mock-radio|mock_layout::mr_34_a_layout_unlinked_receive_stream_takes_no_untimed_tune` fails at ezsdr-mock-radio/tests/mock_layout.rs:1112:5: (0.4735738, -0.16039905) != (0.19712968, 0.4594996)
P159 | maintenance | DELETE | `apply_layout_channel_update` deleted by part (a)/(b) (killer test gone too) | 
P160 | maintenance | RETARGET | re-anchored to today's code | `mock-radio|mock_layout::mr_34_ideal_layout_channels_are_deterministic_and_reproduce_with_the_seed` fails at ezsdr-mock-radio/tests/mock_layout.rs:1005:5: assertion `left == right` failed left: (-0.24740598, 0.035920527) right: (0.25, 0.0)
P161 | maintenance | RETARGET | filter changed (old killer gone) | `mock-radio|mock_layout::mr_33_layout_gain_and_frequency_timelines_are_per_logical_channel` fails at ezsdr-mock-radio/tests/mock_layout.rs:978:5: 0+0i
P163 | maintenance | DELETE | `layout_instant` deleted by part (a)/(b) (killer test gone too) | 
P164 | maintenance | RETARGET | filter changed (old killer gone) | `mock-radio|mock_layout::mr_33_layout_gain_and_frequency_timelines_are_per_logical_channel` fails at ezsdr-mock-radio/tests/mock_layout.rs:301:23: prepare the layout Mock: Rejected: MR-31: CH-6: radio has already joined
P165 | maintenance | DELETE | its code is already gone (deleted by part (b); the anchor matched nothing before part (c1) either), not "no surviving test kills it" | 
P166 | maintenance | DELETE | `apply_layout_channel_update` deleted by part (a)/(b) (killer test still exists but cannot fail) | 
P167 | maintenance | DELETE | `apply_layout_channel_update` deleted by part (a)/(b) (killer test still exists but cannot fail) | 
P168 | maintenance | DELETE | `apply_layout_channel_update` deleted by part (a)/(b) (killer test still exists but cannot fail) | 
P169 | maintenance | RETARGET | filter changed (old killer gone) | `mock-radio|mock_channel::mr_15_a_transmit_block_is_emitted_only_after_its_last_sample` fails at ezsdr-mock-radio/tests/mock_channel.rs:1253:17: assertion `left == right` failed: sample 999 left: (0.24414063, -0.12207031) right: (0.0, 0.5)
P170 | maintenance | RETARGET | filter changed (old killer gone) | `mock-radio|mock_layout::mr_34_a_layout_deterministic_channel_keeps_its_phase_and_coherence_through_entry` fails at ezsdr-mock-radio/tests/mock_layout.rs:1074:5: assertion `left == right` failed left: String("none") right: "deterministic"
P172 | maintenance | DELETE → RE-ADDED | re-added in part (d), same new test as P154 | 
P173 | maintenance | DELETE | its code is already gone (deleted by part (b); the anchor matched nothing before part (c1) either), not "no surviving test kills it" | 
P175 | maintenance | DELETE | no surviving test kills the mutant (own bin, all bins of the crate, acceptance parity and v58; probe logs c1-probes/) | 
P176 | maintenance | DELETE | no surviving test kills the mutant (own bin, all bins of the crate, acceptance parity and v58; probe logs c1-probes/) | 
P177 | maintenance | DELETE | mutated text names deleted `timed_phase` | 
P181 | maintenance | DELETE | mutated text names deleted `timed_phase` | 
P182 | maintenance | DELETE | DEAD, region deleted in part (d): MockRadio's transmit untimed tune | 
P183 | maintenance | DELETE → RE-ADDED | re-added in part (d), `old` re-anchored (the TX arm of the untimed tune is deleted), new test `mr_34_a_restart_redraws_each_random_channel_in_channel_order` (mock_layout.rs) | 
P187 | maintenance | DELETE | no surviving test kills the mutant (own bin, all bins of the crate, acceptance parity and v58; probe logs c1-probes/) | 
P190 | maintenance | DELETE → RE-ADDED | re-added in part (d), same new test as P183 | 
P191 | maintenance | DELETE → RE-ADDED | re-added in part (d), `old` re-anchored to `tune_order(k)` (now by stream number alone), new test `mr_34_two_streams_restarted_at_one_origin_draw_in_stream_order` (mock_layout.rs) | 
P192 | maintenance | DELETE | no surviving test kills the mutant (own bin, all bins of the crate, acceptance parity and v58; probe logs c1-probes/) | 
P193 | maintenance | DELETE | DEAD, region deleted in part (d): MockRadio's transmit untimed tune | 
P194 | maintenance | DELETE | identical mutant of P197 | 
P195 | maintenance | DELETE | DEAD, region deleted in part (d): MockRadio's transmit untimed tune | 
P196 | maintenance | RETARGET | filter changed (old killer gone) | `mock-radio|mock_layout::mr_34_a_layout_unlinked_receive_stream_takes_no_untimed_tune` fails at ezsdr-mock-radio/tests/mock_layout.rs:1112:5: (-0.17333674, -0.46899295) != (0.19712968, 0.4594996)
P197 | maintenance | RETARGET | filter changed (old killer gone) | `mock-radio|mock_layout::mr_34_a_layout_unlinked_receive_stream_takes_no_untimed_tune` fails at ezsdr-mock-radio/tests/mock_layout.rs:1112:5: (0.039304316, 0.49845278) != (0.19712968, 0.4594996)
P198 | maintenance | DELETE | identical mutant of P197 | 
P242 | maintenance | DELETE | no surviving test kills the mutant (own bin, all bins of the crate, acceptance parity and v58; probe logs c1-probes/) | 
P248 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|lib::ur_25_a_refused_receive_owner_books_and_gives_back_in_one_line_section` fails at ezsdr-radio-uhd/src/provider/rx.rs:2375:9: the refusal is booked
P249 | maintenance | DELETE | `do_switch` deleted by part (a)/(b) (killer test gone too) | 
P250 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|lib::ur_25_a_refused_segment_is_booked_by_uhd_rx` fails at ezsdr-radio-uhd/src/provider/rx.rs:2145:9: assertion failed: rx.begin()
P252 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P253 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P254 | maintenance | DELETE | `Switch` retarget deleted (`retarget` only reads the plan's cut) | 
P255 | maintenance | DELETE | `do_switch` deleted by part (a)/(b) (killer test gone too) | 
P260 | maintenance | DELETE | `Planned` carries no clock now | 
P261 | maintenance | DELETE | DEAD, region deleted in part (d): `reconcile` | 
P264 | maintenance | DELETE | projection at the origin deleted (`settings_on`) | 
P265 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P266 | maintenance | DELETE | `book_timed` deleted by part (a)/(b) (killer test gone too) | 
P267 | maintenance | DELETE | `book_timed` deleted by part (a)/(b) (killer test gone too) | 
P268 | maintenance | DELETE | `book_timed` deleted by part (a)/(b) (killer test gone too) | 
P271 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P272 | maintenance | DELETE | `do_switch` deleted by part (a)/(b) (killer test still exists but cannot fail) | 
P278 | maintenance | DELETE | `prune_stream` is `Line::prune` alone now (no move dependency) | 
P279 | maintenance | DELETE | `settings_on` deleted by part (a)/(b) (killer test gone too) | 
P312 | maintenance | DELETE | `release` deleted by part (a)/(b) (killer test gone too) | 
P313 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P314 | maintenance | DELETE | projection at the origin deleted (`settings_on`) | 
P315 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|fake::ur_12_a_layout_an_antenna_read_back_that_differs_is_refused` fails at ezsdr-radio-uhd/tests/fake.rs:5297:118: called `Result::unwrap_err()` on an `Ok` value: ()
P316 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|fake::ur_12_a_layout_s_stream_zero_applies_each_channel_s_own_settings` fails at ezsdr-radio-uhd/tests/fake.rs:5177:5: assertion `left == right` failed: ["set_sources internal internal", "set_time_zero now", "serial", "set_subdev_spec rx A:0 B:0", "set_subdev_spec tx A
P317 | maintenance | DELETE | no surviving test kills the mutant (own bin, all bins of the crate, acceptance parity and v58; probe logs c1-probes/) | 
P319 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P320 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P321 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P327 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P328 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P329 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|fake::ur_12_prepare_refusals` fails at ezsdr-radio-uhd/tests/fake.rs:1873:5: UR-12: the request was refused
P335 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P337 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P339 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P343 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P344 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|fake::ur_13_arm_refuses_a_rate_set_the_description_does_not_have` fails at ezsdr-radio-uhd/tests/fake.rs:5058:9: tx=false: Running
P345 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|lib::ur_36_a_successful_open_moves_the_generation_on_even_when_its_check_refuses` fails at ezsdr-radio-uhd/src/provider/mod.rs:995:9: assertion failed: core.open_streamer(Dir::Tx, 0, Channels::first(1)).is_err()
P349 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P351 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P352 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P354 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P355 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P356 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P357 | maintenance | RETARGET | re-anchored to today's code | `radio-uhd|lib::ur_25_a_start_rx_takes_the_line_lock_once_for_its_instant_and_its_booking` fails at ezsdr-radio-uhd/src/provider/control.rs:963:9: assertion `left == right` failed: an accepted start_rx took the line lock [125099110098904, 125099110098904] left: 2 right: 1
P358 | maintenance | DELETE | `do_switch` deleted by part (a)/(b) (killer test gone too) | 
P359 | maintenance | DELETE | no surviving test kills the mutant (own bin, all bins of the crate, acceptance parity and v58; probe logs c1-probes/) | 
P360 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|lib::ur_13_a_failed_open_by_the_owner_is_opened_again` fails at ezsdr-radio-uhd/src/provider/rx.rs:2068:9: assertion failed: matches!(lock(&core.streams).stream(stream_key(Dir::Rx, 0)).set, crate::provider::core::DeviceSet::Unknown)
P369 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|lib::ur_13_a_failed_open_by_the_owner_is_opened_again` fails at ezsdr-radio-uhd/src/provider/rx.rs:2067:9: assertion `left == right` failed: ["rx_start 0 0"] left: 0 right: 1
P370 | maintenance | DELETE | identical mutant of P369 | 
P371 | maintenance | DELETE | `do_switch` deleted by part (a)/(b) (killer test gone too) | 
P372 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|fake::ur_25_a_receive_session_follows_its_recorded_plan` fails at ezsdr-radio-uhd/tests/fake.rs:2584:5: assertion `left == right` failed: one streamer: ["set_sources internal internal", "set_time_zero now", "serial", "set_subdev_spec rx A:0", "set_subdev
P373 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|lib::ur_28_a_streamer_the_record_holds_keeps_its_correlations` fails at ezsdr-radio-uhd/src/provider/tx.rs:3273:9: a new streamer's queue starts empty
P377 | maintenance | DELETE | no surviving test kills the mutant (own bin, all bins of the crate, acceptance parity and v58; probe logs c1-probes/) | 
P379 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|lib::ur_28_a_later_stream_s_reopen_discards_only_its_correlations` fails at ezsdr-radio-uhd/src/provider/tx.rs:3421:9: stream 1's old correlations are dropped
P382 | maintenance | DELETE | no surviving test kills the mutant (own bin, all bins of the crate, acceptance parity and v58; probe logs c1-probes/) | 
P383 | maintenance | DELETE | `read` deleted by part (a)/(b) (killer test gone too) | 
P384 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P385 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P386 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P387 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P388 | maintenance | RETARGET | re-anchored to today's code, filter changed (old killer gone) | `radio-uhd|lib::ur_23_m6_a_waiting_owner_wakes_for_a_stop_within_an_idle_period` fails at ezsdr-radio-uhd/src/provider/tx.rs:5008:9: assertion `left == right` failed: [TxRec { what: "raise_thread_priority", host: 546.105µs, thread: "tx-stop", n: 0, eob: false }, TxRec { what: "tx_as
P389 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P390 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P391 | maintenance | DELETE → RE-ADDED | re-added in part (d), new unit `ur_29_a_loss_between_two_channels_of_a_configure_applies_no_more` (rx.rs) | 
P394 | maintenance | DELETE | `updates` lock in `configure` deleted | 
P395 | maintenance | DELETE | `await_reading` deleted by part (a)/(b) (killer test gone too) | 
P396 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P397 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P398 | maintenance | DELETE | no surviving test kills the mutant (own bin, all bins of the crate, acceptance parity and v58; probe logs c1-probes/) | 
P399 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P400 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P401 | maintenance | DELETE | `await_reading` deleted by part (a)/(b) (killer test gone too) | 
P404 | maintenance | DELETE | `read` deleted by part (a)/(b) (killer test gone too) | 
P405 | maintenance | DELETE | `targets` deleted by part (a)/(b) (killer test gone too) | 
P406 | maintenance | DELETE | `targets` deleted by part (a)/(b) (killer test gone too) | 
P407 | maintenance | DELETE | `targets` deleted by part (a)/(b) (killer test gone too) | 
P408 | maintenance | DELETE | `targets` deleted by part (a)/(b) (killer test gone too) | 
P409 | maintenance | DELETE | `give` deleted by part (a)/(b) (killer test gone too) | 
P410 | maintenance | DELETE | `give` deleted by part (a)/(b) (killer test gone too) | 
P414 | maintenance | DELETE | `record_given` deleted by part (a)/(b) (killer test gone too) | 
P415 | maintenance | DELETE | `record_given` deleted by part (a)/(b) (killer test gone too) | 
P416 | maintenance | DELETE | `record_given` deleted by part (a)/(b) (killer test gone too) | 
P418 | maintenance | DELETE | `give` deleted by part (a)/(b) (killer test gone too) | 
P419 | maintenance | DELETE | `give` deleted by part (a)/(b) (killer test gone too) | 
P420 | maintenance | DELETE → RE-ADDED | re-added in part (d), new test `ur_36_every_restart_is_configured_with_the_connect_values` (fake.rs) | 
P422 | maintenance | DELETE | `book_timed` deleted by part (a)/(b) (killer test gone too) | 
P424 | maintenance | DELETE | `give` deleted by part (a)/(b) (killer test gone too) | 
P426 | maintenance | DELETE | `book_timed` deleted by part (a)/(b) (killer test gone too) | 
P427 | maintenance | DELETE | `book_timed` deleted by part (a)/(b) (killer test gone too) | 
P428 | maintenance | DELETE | `book_timed` deleted by part (a)/(b) (killer test gone too) | 
P430 | maintenance | DELETE | `of_record` deleted by part (a)/(b) (killer test gone too) | 
P431 | maintenance | DELETE | `release` deleted by part (a)/(b) (killer test gone too) | 
P432 | maintenance | DELETE | `release` deleted by part (a)/(b) (killer test gone too) | 
P433 | maintenance | DELETE | `release` deleted by part (a)/(b) (killer test gone too) | 
P434 | maintenance | DELETE | `give` deleted by part (a)/(b) (killer test gone too) | 
P444 | maintenance | DELETE | DEAD, region deleted in part (d): `reconcile` | 
P447 | maintenance | DELETE | `channel_live` deleted by part (a)/(b) (killer test gone too) | 
P448 | maintenance | DELETE | `targets` deleted by part (a)/(b) (killer test gone too) | 
P449 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P455 | maintenance | DELETE | `await_reading` deleted by part (a)/(b) (killer test gone too) | 
P457 | maintenance | DELETE | `do_switch` deleted by part (a)/(b) (killer test gone too) | 
P459 | maintenance | DELETE | a cold change starting a stopped stream: deleted | 
P460 | maintenance | RETARGET | re-anchored to today's code, filter changed (old killer gone) | `radio-uhd|lib::ur_25_a_refused_segment_is_booked_by_uhd_rx` fails at ezsdr-radio-uhd/src/provider/rx.rs:2135:9: assertion `left == right` failed left: [(0, None), (25200000, Some(3))] right: [(0, None), (13600000, Some(2))]
P462 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P463 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P464 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|fake::ur_25_a_successful_start_clears_a_later_stream_s_halt` fails at ezsdr-radio-uhd/tests/fake.rs:5827:5: assertion `left == right` failed: the halt is cleared: ["set_sources internal internal", "set_time_zero now", "serial", "set_subdev_spec rx A:0 B:0", 
P465 | maintenance | DELETE | `book_timed` deleted by part (a)/(b) (killer test gone too) | 
P470 | maintenance | RETARGET | re-anchored to today's code | `radio-uhd|lib::ur_26_a_stream_s_delivered_is_cleared_alone` fails at ezsdr-radio-uhd/src/provider/rx.rs:1534:9: assertion `left == right` failed: stream 0's floor stands left: None right: Some((0, Config { channels: Channels(1), ratio: Rational { num: 200, den: 
P472 | maintenance | DELETE | identical mutant of P470 | 
P474 | maintenance | DELETE | `do_switch` deleted by part (a)/(b) (killer test gone too) | 
P478 | maintenance | RETARGET | re-anchored to today's code, filter changed (old killer gone) | `radio-uhd|lib::ur_23_txs_cut_and_stop_do_not_send_past_a_high_rate_frontier` fails at ezsdr-radio-uhd/src/provider/tx.rs:4474:9: assertion `left == right` failed left: (20000, false, false) right: (2, false, true)
P481 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|fake::ur_30_the_plan_names_every_stream_at_the_stop` fails at ezsdr-radio-uhd/tests/fake.rs:6629:9: assertion `left == right` failed: each stream holds its own channel: {"rx":[{"channels":1,"end":410721690,"origin":400515290,"sequence":null,"stream":
P482 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|fake::ur_30_the_plan_names_every_stream_at_the_stop` fails at ezsdr-radio-uhd/tests/fake.rs:6644:5: the restart's segment names its command: {"rx":[{"channels":[0],"end":410714089,"origin":400507989,"sequence":null,"stream":0,"ticks_per_sample":[100,
P484 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|lib::ur_25_a_refused_receive_owner_books_and_gives_back_in_one_line_section` fails at ezsdr-radio-uhd/src/provider/rx.rs:2375:9: the refusal is booked
P486 | maintenance | DELETE | `cold_streams` deleted by part (a)/(b) (killer test gone too) | 
P488 | maintenance | DELETE | `layout_instant` deleted by part (a)/(b) (killer test gone too) | 
P489 | maintenance | DELETE | `release_unneeded` deleted by part (a)/(b) (killer test gone too) | 
P490 | maintenance | DELETE | `release_first` of `owner_configure` deleted | 
P491 | maintenance | DELETE | `await_release` deleted by part (a)/(b) (killer test gone too) | 
P492 | maintenance | DELETE | `await_release` deleted by part (a)/(b) (killer test gone too) | 
P493 | maintenance | DELETE | `await_release` deleted by part (a)/(b) (killer test gone too) | 
P495 | maintenance | DELETE | `await_release` deleted by part (a)/(b) (killer test gone too) | 
P496 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P497 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P498 | maintenance | DELETE | destination reconciliation of a refused move deleted | 
P499 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P501 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P502 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P503 | maintenance | RETARGET | re-anchored to today's code | `radio-uhd|lib::ur_34_a_transmit_stop_books_the_wait_exit_and_queues_the_owner_stop` fails at ezsdr-radio-uhd/src/provider/control.rs:579:121: called `Option::unwrap()` on a `None` value
P504 | maintenance | DELETE | `prune_stream` is `Line::prune` alone now (no move dependency) | 
P505 | maintenance | DELETE | `await_release` deleted by part (a)/(b) (killer test gone too) | 
P506 | maintenance | DELETE | `do_switch` deleted by part (a)/(b) (killer test gone too) | 
P507 | maintenance | DELETE | `needed_before` deleted by part (a)/(b) (killer test gone too) | 
P508 | maintenance | DELETE | `release_unneeded` deleted by part (a)/(b) (killer test gone too) | 
P509 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|lib::ur_23_a_burst_at_the_first_safe_sample_after_a_held_tail_close_is_played` fails at ezsdr-radio-uhd/src/provider/tx.rs:3702:9: assertion `left == right` failed: ["tx_open 0 [0]", "apply tx 0 rate=1000000 freq=- gain=- antenna=- at=-", "tx_send 0 n=1000 at=8000000 sob=true eob=
P512 | maintenance | DELETE | `release_first` of `owner_configure` deleted | 
P513 | maintenance | DELETE → RE-ADDED | re-added in part (d), new unit `ur_29_an_owner_does_not_configure_a_segment_a_stop_dropped_from_the_plan` (rx.rs) | 
P515 | maintenance | DELETE | `release_unneeded` deleted by part (a)/(b) (killer test gone too) | 
P517 | maintenance | DELETE | mutated text names a deleted item (`switch`); fn gone | 
P518 | maintenance | DELETE | `adopt_later` deleted by part (a)/(b) (killer test gone too) | 
P519 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P520 | maintenance | DELETE | `await_release` deleted by part (a)/(b) (killer test gone too) | 
P522 | maintenance | DELETE | `do_switch` deleted by part (a)/(b) (killer test gone too) | 
P523 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P524 | maintenance | DELETE | `prune_requested` deleted by part (a)/(b) (killer test gone too) | 
P552 | maintenance | DELETE | `starting` deleted by part (a)/(b) (killer test gone too) | 
P553 | maintenance | DELETE | `search` deleted by part (a)/(b) (killer test gone too) | 
P554 | maintenance | RETARGET | re-anchored to today's code, filter changed (old killer gone) | `radio-uhd|lib::ur_26_a_receive_start_of_no_planned_segment` fails at ezsdr-radio-uhd/src/provider/rx.rs:1565:13: assertion `left == right` failed: HEAD keeps the tuple left: None right: Some((2000000, Config { channels: Channels(1), ratio: Rational { num: 200, de
P555 | maintenance | RETARGET | re-anchored to today's code, filter changed (old killer gone) | `radio-uhd|lib::ur_26_an_end_takes_delivered_from_each_segment_s_own_stream` fails at ezsdr-radio-uhd/src/provider/core.rs:1976:9: assertion `left == right` failed left: Some(999) right: Some(1000)
P556 | maintenance | DELETE | `Phase` enum deleted (`finished` only clears `delivered`) | 
P557 | maintenance | DELETE | `plan_read` deleted by part (a)/(b) (killer test gone too) | 
P558 | maintenance | RETARGET | re-anchored to today's code | `radio-uhd|lib::ur_26_a_finished_stream_clears_only_its_own_floor` fails at ezsdr-radio-uhd/src/provider/core.rs:2044:13: assertion `left == right` failed: stream 0's floor stands left: None right: Some((0, Config { channels: Channels(1), ratio: Rational { num: 200, den: 
P559 | maintenance | RETARGET | re-anchored to today's code | `radio-uhd|lib::ur_26_a_stream_s_delivered_is_cleared_alone` fails at ezsdr-radio-uhd/src/provider/rx.rs:1535:9: assertion `left == right` failed: stream 1's floor is cleared left: Some((0, Config { channels: Channels(1), ratio: Rational { num: 200, den: 1 } }, 0
P560 | maintenance | DELETE | `plan_read` deleted by part (a)/(b) (killer test gone too) | 
P564 | maintenance | DELETE | `cancel_later` deleted | 
P567 | maintenance | DELETE | `apply_update` deleted by part (a)/(b) (killer test gone too) | 
P568 | maintenance | DELETE | `tx_switch` deleted by part (a)/(b) (killer test gone too) | 
P571 | maintenance | DELETE | `do_switch` deleted by part (a)/(b) (killer test gone too) | 
P572 | maintenance | DELETE | `adopt_later` deleted by part (a)/(b) (killer test gone too) | 
P573 | maintenance | DELETE | `timed_instant` deleted by part (a)/(b) (killer test gone too) | 
P574 | maintenance | DELETE | `timed_instant` deleted by part (a)/(b) (killer test gone too) | 
P575 | maintenance | DELETE | `timed_instant` deleted by part (a)/(b) (killer test gone too) | 
P576 | maintenance | DELETE | `book_timed` deleted by part (a)/(b) (killer test gone too) | 
P577 | maintenance | DELETE | Mock timed queue deleted | 
P578 | maintenance | DELETE | Mock release window of the timed queue deleted | 
P579 | maintenance | DELETE | `book_timed` deleted by part (a)/(b) (killer test gone too) | 
P580 | maintenance | DELETE | `give` deleted by part (a)/(b) (killer test gone too) | 
P581 | maintenance | DELETE | `release` deleted by part (a)/(b) (killer test gone too) | 
P582 | maintenance | DELETE | `finish` only asserts `ended_at`; no instant left to mutate | 
P583 | maintenance | DELETE | `settle` deleted by part (a)/(b) (killer test gone too) | 
P584 | maintenance | DELETE | `settle` deleted by part (a)/(b) (killer test gone too) | 
P585 | maintenance | DELETE | `settle` deleted by part (a)/(b) (killer test gone too) | 
P586 | maintenance | DELETE | `settle` deleted by part (a)/(b) (killer test gone too) | 
P587 | maintenance | DELETE | `release` deleted by part (a)/(b) (killer test gone too) | 
P588 | maintenance | DELETE | `settle` deleted by part (a)/(b) (killer test gone too) | 
P589 | maintenance | DELETE | Mock `record_update` deleted | 
P590 | maintenance | DELETE | Mock LATE_COMMAND deleted | 
P591 | maintenance | DELETE | `ceilings` deleted by part (a)/(b) (killer test gone too) | 
P592 | maintenance | DELETE | `ceilings` deleted by part (a)/(b) (killer test gone too) | 
P593 | maintenance | DELETE | `settle` deleted by part (a)/(b) (killer test gone too) | 
P594 | maintenance | DELETE | `settle` deleted by part (a)/(b) (killer test still exists but cannot fail) | 
P595 | maintenance | DELETE | `release` deleted by part (a)/(b) (killer test gone too) | 
P596 | maintenance | DELETE | `release` deleted by part (a)/(b) (killer test gone too) | 
P598 | maintenance | DELETE | `give` deleted by part (a)/(b) (killer test gone too) | 
P599 | maintenance | DELETE | `settle` deleted by part (a)/(b) (killer test gone too) | 
P600 | maintenance | DELETE | `release` deleted by part (a)/(b) (killer test gone too) | 
P601 | maintenance | DELETE | `settle` deleted by part (a)/(b) (killer test gone too) | 
P602 | maintenance | DELETE | `settle` deleted by part (a)/(b) (killer test gone too) | 
P603 | maintenance | DELETE | `give` deleted by part (a)/(b) (killer test gone too) | 
P604 | maintenance | DELETE | `refusal` deleted by part (a)/(b) (killer test gone too) | 
P605 | maintenance | DELETE | `refusal` deleted by part (a)/(b) (killer test gone too) | 
P606 | maintenance | DELETE | `refusal` deleted by part (a)/(b) (killer test gone too) | 
P607 | maintenance | DELETE | `apply_update` deleted by part (a)/(b) (killer test gone too) | 
P608 | maintenance | DELETE | `apply_update` deleted by part (a)/(b) (killer test gone too) | 
P609 | maintenance | DELETE | `apply_update` deleted by part (a)/(b) (killer test gone too) | 
P610 | maintenance | DELETE | `apply_update` deleted by part (a)/(b) (killer test gone too) | 
P611 | maintenance | DELETE | `apply_update` deleted by part (a)/(b) (killer test gone too) | 
P612 | maintenance | DELETE | `cancel_booked` deleted by part (a)/(b) (killer test gone too) | 
P613 | maintenance | DELETE | halted-stream refusal in `handle_tx_burst` deleted | 
P614 | maintenance | DELETE | no surviving test kills the mutant (own bin, all bins of the crate, acceptance parity and v58; probe logs c1-probes/) | 
P615 | maintenance | DELETE | the intended code (a waiting cold_change_refused fault) is unreachable now; the lost-device tests kill the mutant only by accident | 
P616 | maintenance | DELETE | no surviving test kills the mutant (own bin, all bins of the crate, acceptance parity and v58; probe logs c1-probes/) | 
P618 | maintenance | DELETE | `refusal` deleted by part (a)/(b) (killer test gone too) | 
P619 | maintenance | DELETE | `apply_update` deleted by part (a)/(b) (killer test gone too) | 
P620 | maintenance | DELETE | `refusal` deleted by part (a)/(b) (killer test gone too) | 
P621 | maintenance | DELETE | `forget_refused_segment` deleted by part (a)/(b) (killer test gone too) | 
P622 | maintenance | DELETE | halted-stream refusal in `handle_tx_burst` deleted | 
P646 | maintenance | DELETE | no surviving test kills the mutant (own bin, all bins of the crate, acceptance parity and v58; probe logs c1-probes/) | 
P647 | maintenance | DELETE | no surviving test kills the mutant (own bin, all bins of the crate, acceptance parity and v58; probe logs c1-probes/) | 
P648 | maintenance | DELETE | `later` held list of moved bursts deleted | 
P65 | maintenance | DELETE | stream rate has no class now; replaced by NEW P1391 | 
P654 | maintenance | DELETE | no surviving test kills the mutant (own bin, all bins of the crate, acceptance parity and v58; probe logs c1-probes/) | 
P659 | maintenance | RETARGET | filter changed (old killer gone) | `mock-radio|lib::a_loss_does_not_remember_a_stop_front` fails at ezsdr-mock-radio/src/lib.rs:2443:9: a loss must cut at its instant
P711 | maintenance | DELETE | no surviving test kills the mutant (own bin, all bins of the crate, acceptance parity and v58; probe logs c1-probes/) | 
P713 | maintenance | DELETE | `book_timed` deleted by part (a)/(b) (killer test gone too) | 
P776 | maintenance | DELETE | `tx_start_trial_class` deleted by part (a)/(b) (killer test gone too) | 
P777 | maintenance | DELETE | `summarize_tx_start_lead` deleted by part (a)/(b) (killer test gone too) | 
P778 | maintenance | DELETE | `tx_start_value` deleted by part (a)/(b) (killer test gone too) | 
P786 | maintenance | DELETE | `tx_start_candidate_stage_json` deleted by part (a)/(b) (killer test gone too) | 
P790 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|fake::manifest_evidence_classifies_tx_ack_and_out_of_range_failure_evidence` fails at ezsdr-radio-uhd/tests/common/measurement.rs:4501:9: assertion `left == right` failed left: Incomplete right: AdmissionRejection
P80 | maintenance | DELETE | mutant (equal-size sets compare equal) survives radio, mock, uhd lib/fake, parity, v58: sets differ only after a membership change (deleted) | 
P81 | maintenance | DELETE | `book_move` deleted by part (a)/(b) (killer test gone too) | 
P82 | maintenance | DELETE | `settle_once` deleted by part (a)/(b) (killer test gone too) | 
P83 | maintenance | DELETE | `refuse_move` deleted by part (a)/(b) (killer test gone too) | 
P84 | maintenance | DELETE | `book_move` deleted by part (a)/(b) (killer test gone too) | 
P86 | maintenance | DELETE | move into a stopped stream: moves deleted | 
P87 | maintenance | DELETE | `book_move` deleted by part (a)/(b) (killer test gone too) | 
P88 | maintenance | DELETE | `book_move` deleted by part (a)/(b) (killer test gone too) | 
P89 | maintenance | DELETE | channel entering an empty stream: membership change deleted | 
P90 | maintenance | RETARGET | re-anchored to today's code, filter changed (old killer gone) | `radio|timeline::rm_25_a_refusal_does_not_move_a_later_origin` fails at ezsdr-radio/tests/timeline.rs:440:13: assertion `left == right` failed: seed 44: refusing 3 moves a later segment left: [Segment { origin: 2000000000, cut: Some(7), config: Config { channe
P91 | maintenance | DELETE | `config_at` deleted by part (a)/(b) (killer test gone too) | 
P92 | maintenance | DELETE | `book_move` deleted by part (a)/(b) (killer test gone too) | 
P93 | maintenance | RETARGET | re-anchored to today's code, filter changed (old killer gone) | `mock-radio|timeline::rm_26_the_timeline_against_mockradio` fails at ezsdr-mock-radio/tests/timeline.rs:224:5: 430 sequences differ (EZSDR_TIMELINE_SEED=<seed> shows one), the first: [ "seed 0, ideal, Loss: [\"rx_clocks\", \"blocks\", \"faults\", \"error\"]", "
P94 | maintenance | DELETE | `settle` of move dependence deleted | 
P945 | maintenance | DELETE | `rx_start_lead_group` deleted by part (a)/(b) (killer test gone too) | 
P946 | maintenance | RETARGET | filter changed (old killer gone) | `radio-uhd|fake::the_overflow_gap_adds_a_condition_with_the_other_direction_running` fails at ezsdr-radio-uhd/tests/fake.rs:5881:5: assertion failed: overflow.conditions.iter().any(|condition| Some(condition.id.as_str()) == extra.condition.as_deref())
P95 | maintenance | DELETE | `omitted` (move release) deleted | 
P957 | maintenance | DELETE | companion start-lead groups deleted | 
P96 | maintenance | DELETE | `settle` deleted by part (a)/(b) (killer test gone too) | 
P965 | maintenance | DELETE | `reach` deleted by part (a)/(b) (killer test gone too) | 
P966 | maintenance | DELETE | `give` deleted by part (a)/(b) (killer test gone too) | 
P967 | maintenance | DELETE | `covering` deleted by part (a)/(b) (killer test gone too) | 
P968 | maintenance | DELETE | `covering` deleted by part (a)/(b) (killer test gone too) | 
P969 | maintenance | DELETE | `covering` deleted by part (a)/(b) (killer test gone too) | 
P97 | maintenance | DELETE | `settle_once` deleted by part (a)/(b) (killer test gone too) | 
P970 | maintenance | DELETE | `give` deleted by part (a)/(b) (killer test gone too) | 
P971 | maintenance | DELETE | `give` deleted by part (a)/(b) (killer test gone too) | 
P972 | maintenance | DELETE | `reach` deleted by part (a)/(b) (killer test gone too) | 
P973 | maintenance | DELETE | `reach` deleted by part (a)/(b) (killer test gone too) | 
P974 | maintenance | DELETE | `configure` publishes no mark any more (projection lock deleted) | 
P975 | maintenance | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
P976 | maintenance | DELETE | `reach` deleted by part (a)/(b) (killer test gone too) | 
P98 | maintenance | DELETE | `book_move` deleted by part (a)/(b) (killer test gone too) | 
P989 | maintenance | DELETE | `covering` deleted by part (a)/(b) (killer test gone too) | 
P99 | maintenance | DELETE | `book_move` deleted by part (a)/(b) (killer test gone too) | 
P991 | maintenance | DELETE | `configure` publishes no mark any more (projection lock deleted) | 
P992 | maintenance | DELETE | `give` deleted by part (a)/(b) (killer test gone too) | 
P993 | maintenance | DELETE | `give` deleted by part (a)/(b) (killer test gone too) | 
T38 | maintenance | RETARGET | re-anchored to today's code | `mock-radio|mock_radio::mr_29_start_rx_takes_effect_at_its_at` fails at ezsdr-mock-radio/tests/mock_radio.rs:2039:5: assertion `left == right` failed left: [(0, Some(1000000)), (2000000, None)] right: [(0, Some(1000000)), (12000000, None)]
T39 | maintenance | RETARGET | re-anchored to today's code | `radio|timeline::mr_29_a_receive_link_floors_a_command_at_its_t0` fails at ezsdr-radio/tests/timeline.rs:353:5: assertion `left == right` failed left: 8000000 right: 10000000
T40 | maintenance | DELETE → RE-ADDED | re-added in part (d), `old` re-anchored to `timeline.rs` `command_instant`, `old2` kept: `mr_29_start_rx_is_never_late` kills it (c1 probed it without `old2`) | 
T41 | maintenance | DELETE | start_rx reports no late command now (`command_instant` returns the instant only) | 
M31 | phase3 | DELETE | `apply_layout_channel_update` deleted by part (a)/(b) (killer test gone too) | 
M32 | phase3 | DELETE | `apply_layout_channel_update` deleted by part (a)/(b) (killer test gone too) | 
M33 | phase3 | RETARGET | re-anchored to today's code, filter changed (old killer gone) | `mock-radio|mock_channel::mr_32_the_transmit_path_delay_shifts_a_loopback` fails at ezsdr-mock-radio/tests/mock_channel.rs:767:9: sample 2045
M46 | phase3 | DELETE | timed phases (`rx_timed`) deleted | 
M47 | phase3 | DELETE | `apply_layout_channel_update` deleted by part (a)/(b) (killer test still exists but cannot fail) | 
M51 | phase3 | DELETE | no surviving test kills the mutant (own bin, all bins of the crate, acceptance parity and v58; probe logs c1-probes/) | 
M55 | phase3 | RETARGET | filter changed (old killer gone) | `mock-radio|mock_channel::mr_32_the_transmit_path_delay_shifts_a_loopback` fails at ezsdr-mock-radio/tests/mock_channel.rs:767:9: sample 2045
M59 | phase3 | RETARGET | filter changed (old killer gone) | `mock-radio|mock_channel::mr_32_the_transmit_path_delay_shifts_a_loopback` fails at ezsdr-mock-radio/tests/mock_channel.rs:767:9: sample 2045
M63 | phase3 | DELETE | `apply_layout_channel_update` deleted by part (a)/(b) (killer test gone too) | 
M64 | phase3 | DELETE | `apply_layout_channel_update` deleted by part (a)/(b) (killer test still exists but cannot fail) | 
M68 | phase3 | DELETE → RE-ADDED | re-added in part (d), new test `mr_25_a_stream_stop_leaves_another_transmit_stream_radiating` (mock_layout.rs) | 
M69 | phase3 | DELETE → RE-ADDED | re-added in part (d), same new test as M68 | 
M70 | phase3 | DELETE | no surviving test kills the mutant (own bin, all bins of the crate, acceptance parity and v58; probe logs c1-probes/) | 
M72 | phase3 | RETARGET | re-anchored to today's code, filter changed (old killer gone) | `mock-radio|mock_channel::mr_16_a_burst_at_or_before_the_open_burst_s_next_sample_is_refused` fails at ezsdr-mock-radio/tests/mock_channel.rs:1128:9: assertion `left == right` failed left: Null right: "MR-16: burst begins at or before the open burst's next sample"
M80 | phase3 | RETARGET | re-anchored to today's code, filter changed (old killer gone) | `mock-radio|mock_channel::mr_16_a_burst_at_or_before_the_open_burst_s_next_sample_is_refused` fails at ezsdr-mock-radio/tests/mock_channel.rs:1186:5: assertion `left == right` failed left: Array [Object {"at": Object {"domain": Object {"local": Number(7), "node": Number(0)}, "ticks": Number(1000500)
M81 | phase3 | RETARGET | re-anchored to today's code, filter changed (old killer gone) | `mock-radio|mock_channel::mr_16_a_burst_at_or_before_the_open_burst_s_next_sample_is_refused` fails at ezsdr-mock-radio/tests/mock_channel.rs:1152:5: assertion `left == right` failed left: 2 right: 1
M82 | phase3 | RETARGET | re-anchored to today's code, filter changed (old killer gone) | `mock-radio|mock_channel::mr_16_a_burst_at_or_before_the_open_burst_s_next_sample_is_refused` fails at ezsdr-mock-radio/tests/mock_channel.rs:1128:9: assertion `left == right` failed left: Null right: "MR-16: burst begins at or before the open burst's next sample"
P12 | phase6 | RETARGET | python test renamed `..._across_a_rate_change_is_refused` -> `..._across_a_restart_is_refused`; probe: mutant killed | `test_easy_api.EasyApi.test_ea_17_a_capture_across_a_restart_is_refused` fails at test_easy_api.py:563 (`assertRaises(ezsdr.Error)`: Error not raised)
B06 | phase7 | DELETE → RE-ADDED | re-added in part (d), new test `ur_25_a_stopped_stream_ends_at_its_cut_and_its_late_tail_stays_off_the_new_clock` (fake.rs) | 
B07 | phase7 | DELETE → RE-ADDED | re-added in part (d), same new test as B06 | 
B13 | phase7 | DELETE | no surviving test kills the mutant (own bin, all bins of the crate, acceptance parity and v58; probe logs c1-probes/) | 
B15 | phase7 | RETARGET | filter changed (old killer gone) | `radio-uhd|fake::ur_25_a_start_booked_anywhere_in_a_long_receive_call_is_on_time` fails at ezsdr-radio-uhd/tests/fake.rs:5864:9: block 65536 packet 1996 phase 0: booked 440636024 {"at":468278679,"channels":1,"origin":452036608,"what":"rx_segment"}
B22 | phase7 | RETARGET | filter changed (old killer gone) | `radio-uhd|fake::ur_25_a_start_booked_anywhere_in_a_long_receive_call_is_on_time` fails at ezsdr-radio-uhd/tests/fake.rs:5864:9: block 100 packet 1996 phase 0: booked 440663460 {"at":441735017,"channels":1,"origin":452114944,"what":"rx_segment"}
C02 | phase7 | DELETE | `record_given` deleted by part (a)/(b) (killer test gone too) | 
R04 | phase7 | DELETE | same mutant as T38 (shared command_instant), T38's Mock test kills it | 
R05 | phase7 | DELETE | `held_updates` deleted by part (a)/(b) (killer test gone too) | 
R07 | phase7 | RETARGET | filter changed (old killer gone) | `radio-uhd|lib::ur_23_a_clock_ending_at_its_cut_cancels_a_held_burst_and_releases_its_reservation` fails at ezsdr-radio-uhd/src/provider/tx.rs:3487:9: assertion failed: events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().any(|event| event.kind.as_str() == ezsdr_rad
R21 | phase7 | DELETE | `book_timed` deleted by part (a)/(b) (killer test gone too) | 
U13 | phase7 | DELETE → RE-ADDED | re-added in part (d), `old` re-anchored to the loop in `Control::run`, new unit `ur_14_each_action_is_booked_before_the_next_recv` (control.rs) | 
U20 | phase7 | DELETE | `book_timed` deleted by part (a)/(b) (killer test gone too) | 
U21 | phase7 | DELETE | `book_timed` deleted by part (a)/(b) (killer test gone too) | 
U22 | phase7 | DELETE | no surviving test kills the mutant (own bin, all bins of the crate, acceptance parity and v58; probe logs c1-probes/) | 
U37 | phase7 | DELETE | `release` deleted by part (a)/(b) (killer test gone too) | 
U38 | phase7 | DELETE | `settle` deleted | 
U39 | phase7 | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
U41 | phase7 | DELETE | `book_cold` deleted by part (a)/(b) (killer test gone too) | 
V06 | phase7 | DELETE → RE-ADDED | re-added in part (d): `ezsdr-radio --test timeline rm_26_the_timeline_cases` kills it (c1 never ran that binary for this row) | 

## Gaps (no row, no test)

- MockRadio guard `handle_update` (`crates/ezsdr-mock-radio/src/lib.rs`, "MR-18: target or configuration key is not updateable"): no surviving test sends a classless UpdateParameter to the Mock (the Kernel refuses it before dispatch; `mock_radio.rs:1695-1747` only reach the loss reason). No row added; a harness test is needed first.
- 61 DELETE rows say "no surviving test kills the mutant": the code is still alive (the anchor matched) but no test pins it any more. Part (c2) or the reviewer may decide whether a test is owed: B06, B07, B13, F36, J20, J21, J62, M51, M68, M69, M70, P1142, P1169, P1220, P1231, P1244, P1259, P1290, P141, P142, P143, P148, P149, P150, P151, P152, P153, P154, P165, P172, P173, P175, P176, P182, P183, P187, P190, P191, P192, P193, P195, P242, P261, P317, P359, P377, P382, P391, P398, P420, P444, P513, P614, P616, P646, P647, P654, P711, U13, U22, V06

## Part (d): the rows the deletion lost, and the code it left dead

Re-check reviewer's classification: `opus-s2-c1-unkilled.md`. Probe logs and the scripts: `work/d-probes/` (`probes-d1.json`..`probes-d3.json`, `dprobe.py`, one log per row and target). Each row below was applied in a throwaway copy outside the repository and its named test run: all FAIL (killed), none COMPILE. The bullets above that name B06 .. V06 as "a test may be owed" and the Mock guard are answered by this part.

Re-added (HEAD text, appended at the end of its list; the kill is the test named in the `filter`): F36, P1220, T40 (maintenance), M68, M69 (phase3), V06, U13, B06, B07 (phase7), P1290, P391, P513, P420, P153, P154, P172, P183, P190, P191 (maintenance). Re-anchored because the code moved: T40 (`old` and `new`), U13 (`old` and `new`), P183 (`old`, `new`), P191 (`old`, `new`). The others' `old` matches HEAD text unchanged.

NEW rows (maintenance list, ids after P1393):

P1394 | maintenance | NEW | MockRadio accepts a classless update that reaches it directly | `mock-radio|mock_radio::mr_29_other_actions_are_command_rejected` (a classless `radio.rx.c0.frequency_hz` update is the third refusal, reason "MR-18: target or configuration key is not updateable")
P1395 | maintenance | NEW | a `start_rx` segment is configured with the profile defaults instead of the Run's start values | `radio-uhd|fake::ur_36_every_restart_is_configured_with_the_connect_values` (the device reads 2.0 GHz after each restart, the connect value)
P1396 | maintenance | NEW | uhd-rx never registers its SampleClock at the first block | `radio-uhd|lib::ur_17_a_finite_segment_delivers_through_its_cut` fails at rx.rs `the old clock does not end at its cut` (left `[]`); 13 other lib tests fail too
P1397 | maintenance | NEW | uhd-rx never ends its SampleClock at the cut | same test, left `[(0, None)]`, right `[(0, Some(1000000))]`

Rows re-anchored because part (d)'s deletions moved their code (each probed killed by its own filter): P170 (`layout_phase_basis(Direction::Rx, ..)`), P178 (`phase_behavior != "deterministic"` alone), P186 (`links.is_empty()` alone), P196 (`segment.by.is_none()` alone), P263 (`.clocks = ...map(|(clock, _)| clock)`), P450 (`stream_key(Dir::Tx, k)` in `book_burst`). P262 is unchanged: `Streams::changed` is kept, as a one-line method, so its `self.version += 1;` is still one text.

Deleted with their code (part (d)): P259 (`reconcile` registered a clock for a plan segment: the whole of `reconcile` is deleted). Before it, the reviewer's DEAD rows stay deleted with their regions: J21, P261, P444 (`reconcile`), P182, P193, P195 (MockRadio transmit untimed tune), P1142 (`host_entry_ns`).


## Gate fix round (17 rows the gate did not accept)

Gate: `gate-s2/gate.json` (4 COMPILE ERROR, 13 SURVIVED). Plan: `test-plan-s2.md` rev 9, sections 14-18. Probe logs and the script: `work/gate-fix-probes/` (`<row>.<run>.log`; `ws` = the whole workspace, `filter:<crate>:<bin>:<test>` = the row's own test, `py:` = Python). Every kept, re-anchored or new row was applied in a throwaway copy and its filter test FAILED; every deleted row survived the wide run.

P392 | maintenance | RE-ANCHOR | `Holder` is not in scope in `tx.rs`: `super::core::Holder::Free` | `radio-uhd|lib::ur_25_a_detached_control_does_not_hold_the_shutdown_s_read` fails at tx.rs:3446 "the shutdown's read waited for a detached control"
P445 | maintenance | RE-ANCHOR | `booked_stream(dir_of(..))` gone: `first_undelivered((stream.0, 0))` | `radio-uhd|lib::ur_26_an_end_takes_delivered_from_each_segment_s_own_stream`
P464 | maintenance | RE-ANCHOR | `stream_key(dir_of(..), 0)` gone: `refuse((stream.0, 0), seq)` | `radio-uhd|fake::ur_25_a_successful_start_clears_a_later_stream_s_halt`
P469 | maintenance | RE-ANCHOR | `config_at(..)` gone: `stream_channels(&self.config, true, k as u32)` | `radio-uhd|fake::ur_26_a_start_rx_of_a_channel_less_linked_stream_starts_nothing`
J01 | maintenance | OWED (test) | explicit cases; a Stop at the Run's start leaves no receive segment | `radio|timeline::rm_26_the_timeline_cases` fails at tests/timeline.rs:124 "a Stop at the Run's start leaves a receive stream no segment" left `[(0, Some(0))]` right `[]`
J02 | maintenance | EQUIVALENT | `begin` is receive-only and `ready >= cut` there, so `ready + lead >= cut + lead`; survived `ws` (1 697 passed) | 
J05 | maintenance | OWED (test) | a Stop before a restart begins, then a start | `radio|timeline::rm_26_the_timeline_cases` fails: "a Stop before a restart begins leaves it out, and the next start follows its origin" left `[.., (1080000000, None)]` right `[.., (1100000000, None)]`
J08 | maintenance | OWED (test fix) | the Stop is handled before the step that applies the fault | `mock-radio|mock_radio::mr_21_overrun_shape` fails at mock_radio.rs:1906 left `[(true, 19000)]` right `[(true, 50000)]`
J12 | maintenance | EQUIVALENT | `begin` has no transmit caller; survived `ws`; the dead `direction == Rx` guard around the call term is deleted | 
P136 | maintenance | OWED (test) | the `segments` rows of the transmit streams name their own channels | `mock-radio|mock_layout::mr_09_each_transmit_stream_has_its_own_clock_and_line` fails at mock_layout.rs:565 left `[(0,[0]),(1,[0])]` right `[(0,[0]),(1,[1])]`
P368 | maintenance | OWED (test) | UR-36's table in `owner_step` (pure decision test); retargeted to test_bin `lib` | `radio-uhd|lib::ur_36_the_owner_step_follows_the_record` fails at core.rs:1944 left `Configure` right `Nothing`
P539 | maintenance | OWED (test) | streams of different rates at connect | `python|test_easy_api.EasyApi.test_ea_16_radio_side_is_stream_zero` fails `(20000000.0, 2000000.0) != (2000000.0, 2000000.0)`
P540 | maintenance | OWED (test) | same test | same test fails `(2000000.0, 20000000.0)`
M30 | phase3 | RETARGET | test_bin `lib`, filter `mr_33_a_timeline_keeps_the_value_in_force_at_each_instant` | that test fails (value at `i64::MIN` is the last change, not the initial)
M53 | phase3 | OWED (test) | new unit test of `TxShared::radiated`; retargeted to test_bin `lib` | `mock-radio|lib::mr_32_a_radiated_sample_takes_the_values_in_force_at_its_own_instant` fails at channel.rs:178 left `(6.1e-16, 10.0, 200.0)` right `(1.0, 0.0, 100.0)`
M54 | phase3 | RETARGET | same unit test as M30 | fails (the first change is taken at tick 200)
M66 | phase3 | EQUIVALENT | no `frequency_hz` Timeline has a change; survived `ws`; replaced by P1400 | 
P1400 | maintenance | NEW | `model.phase.at(stamp)` of the receive path read at the start of time | `mock-radio|mock_layout::mr_34_a_restart_redraws_each_random_channel_in_channel_order` (3 `mr_34` tests fail)
