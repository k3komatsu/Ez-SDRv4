STATUS: done
(The subagent could not write files; the orchestrator saved its final message here, condensed only in layout.)

## Built
- src/provider/mod.rs: UhdRadio::with_measurement_performance(performance) — replaces the Description's performance and rebuilds surfaces via with_measurement_timing. Only src change; default construction untouched.
- tests/common/mod.rs: EZSDR_MEASUREMENT_SELECTOR — parse_selector accepts only rx, tx, image, links, link_gbps; a thread-local override is laid over bench_profile, which keeps explicit rx/tx; per-channel channel_hz and bench_envelope bands; no selector = unchanged output.
- tests/common/measurement.rs: Metadata.selector validated in process_metadata before open; MeasurementPlan gains duplex (EZSDR_MEASURE_DUPLEX=1) and nics (EZSDR_MEASUREMENT_NICS, fallback EZSDR_BENCH_NIC); multi-channel and duplex conditions in both throughput groups, capacity over the condition set; rx/tx start lead get -with-tx/-with-rx companion conditions at 10 MS/s, overflow gap one companion condition; check_control after each run.finish(); 1-channel tx Spec uses tx channel 0's frequency.
- tests/common/measurement/multichannel.rs (new, 775 lines): CountingDevice (per-stream counts, fault injection), throughput_stages, capacity_over_conditions, run_stage (per-stream samples / accepted sends / ack, NIC counters), companion verdict, wedge mark, 7 pure tests.
- tests/hardware.rs: record installs the selector; after a stage wedges the handle, later groups are skipped and recorded failed.
- run-record.sh: passes selector, NICs, duplex flag; att confirmation names every tx channel in the selector plus slot A (raw probes always drive slot A); --help; no skip.
- bench.md BP4b/BP4c; design/18 UR-34 text, test names, Changes row. rules.py 0 problems.

## Deviations
1. Stage order ascending in total bytes/s (brief: descending); matches the 1-channel sweep; value independent of order; only which stages ran after a wedge differs.
2. Duplex opt-in (EZSDR_MEASURE_DUPLEX=1); duplex stages run separately in the rx and tx groups (doubles that bench time).
3. Rx sample tolerance max(packet, DEFAULT_BLOCK_LEN): Provider stats are not per stream, CountingDevice counts overshoot the cut by up to one block.
4. A stream of m channels is acknowledged once per channel; exactly m acks = one acknowledgement, else Incomplete.
5. A streamer left open after a stage, or a control timeout, marks the control path wedged; remaining stages recorded not run; capacity stops below them.
6. Candidate ceiling = link_gbps[0] × 125 MB/s; applies to multi-channel and duplex stages, not the existing 1-channel sweep.
7. Raw probes still drive slot A device channel 0 at 1 GHz (slot A must be UBX or OBX).

## Tests
Selector (3); two-channel rehearsals (4); fault cases (5: starved stream, partial send, missing burst ack, one unacknowledged channel, duplex companion not flowing); admission (2); leads/overflow companion (1); pure tests in multichannel.rs (7).

## Bench time (computed)
1 channel no duplex unchanged (7 rx, 7 tx stages). 2 channels: ~27 stages per direction (~27–30 min each, ~1 h rx+tx). Duplex +7 stages per direction. XG 2 links: same count; stages above 1 250 MB/s rejected at admission.

## Mutation
Rows P913–P947 (35). First run P913–P946: 33 killed; P916 a compile error in the mutant, fixed, killed. After review fixes re-ran 13 rows (P915–P918, P937–P940, P943–P947): all killed. No existing row touched. No rows for: hardware.rs wedge skip, check_control call sites, tx_at_channel_zero_frequency, ceiling passed into the builder in run_stage.

## Checks
cargo test --workspace final 1 864 passed 0 failed. One earlier run: existing ur_25_an_enable_from_no_stream_configures_once failed once (burst 1 ms late); 3/3 after; full rerun clean. Workspace clippy -D warnings, docker --features uhd clippy, git diff --check, bash -n pass. New rehearsals 20/20 before and after review fixes.

## Self-review
Free model hit its limit; one round on opencode-go/deepseek-v4.1-flash#max: 3 P1, 8 P2, 6 P3.
- P1-1 att confirmation missing slot A: fixed.
- P1-2 companion evidence counted before the window: partly — CountingDevice is not attached to the lead Sessions, so the count is Run-wide incl. ~1 M samples before the window; a window under a few seconds can't tell stalled from flowing; bench window (100 × 20 ms ≈ 2 s) can (9/10 of 20 M samples must move). Earlier window start gave a false failure on the fake; reverted; shortcut: comment.
- P1-3 partial ack read as Success: rejected (code drops all acks); added a test and row P947.
- P2 fixed: selector-less envelope with two tx channels, tx channel 0 frequency, control-timeout detection on every path, selector shapes (one-element lists, test builds the Description), bench.md. Rejected: stage order, raw probes ignoring the selector, P942 worry.
- P3 fixed: builder's *Test:*, "each stage" wording, companion trials out of the main overflow observation, NIC fallback, help text. Left: duplex in both groups, topic splitting, max gap merge.

## Open
Stage order and duplex sharing; companion window limit; hardware.rs wedge skip and control-timeout Run not exercised on the fake; 3c-3b not started. Diff 968+/41− across 9 files.

# Fix round 1 (after Sol FAIL: 4 majors, 1 minor)
1. Companion lead/overflow conditions are observation-only ("observation_only": true, Run-wide samples, fault count); companion_verdict and the 90% arithmetic removed; P931–P933 re-pointed. Duplex throughput (run_stage, per-stream CountingDevice) stays in capacity. Open: a companion error still fails the whole lead group (losing solo values).
2. GuardedDevice + one shared Latch at the Device boundary replaces check_control / mark_wedged / thread-local mark. Latches on any apply error, a control timeout or device loss from any call, a streamer left open after a group; afterwards refuses every forwarded call except describe, fidelity, master_clock_rate, host, mark_lost. hw_measurements_record wraps the lease device and every probe device (usrp()/guard_if_recording); groups after the latch recorded not run. Not exercised: hardware.rs wiring, rx_recv latch path.
3. raw_probes_applicable / run_raw_probes: with a selector, probes run only if rx and tx c0 are slot A ubx/obx; else each recorded `inapplicable_to_this_layout`. run-record names slot A only when probes run, plus every selector tx.
4. Duplex by default whenever both directions exist (incl. 1-channel); EZSDR_MEASURE_DUPLEX removed. Companion lead/overflow observations always run (~2× lead time).
5. UR-34 rewritten as short topics with *Test:*; history in Changes ("UR-5, UR-34").
6. UR-5 table: link_gbps "1 or 10, [1] with hg, [10] with xg".
Mutation: P913–P951 (P952, P953 deleted: rx_recv paths unobservable on the fake); re-pointed P926, P929–P933, P937, P938, P945, P946; new P948–P951. Run: 38 killed, P949 compile error fixed then killed. rules.py: every old matches once.
Tests: nothing_reaches_the_device_after_{a_control_timeout, a_prepare_time_apply_failure, a_group_leaves_a_streamer_open} (inner FakeDevice call log), raw_probes_are_inapplicable_when_channel_zero_is_not_slot_a_on_a_ubx_or_obx, a_stage_between_the_profile_limit_and_the_candidate_ceiling_is_admitted (~28 s), the_one_channel_transmit_spec_carries_transmit_channel_zero_s_own_frequency, the_guard_latches_on_apply_errors_timeouts_and_losses_only; updated rehearsals. Timeout test uses a synthetic error via guard.note.
Checks: workspace 1 869 passed; clippy and docker clippy clean; rules.py 0; diff --check, bash -n ok; 20 changed rehearsals 20/20.
Diff 1 117+/49−; multichannel.rs 855 lines.

# Fix round 2 (after Sol re-review FAIL: 5 majors, 1 minor)
1. GuardedDevice caches describe/master clock/fidelity at wrap; latched drop leaks the inner handle (mem::forget); guard_open / open_record_guard check the latch before any opener. Record setup (open, description, serial) runs before the raw probes and closes cleanly; groups reopen after; a latched record opens nothing, all groups "not run".
2. Guard tracks (direction, stream) open streamers via its own forwarded open/close; re-open of a stream it still sees open latches and is refused before the device. After-group latch_if_streamers_open kept (one call; catches another stream index on the same front end).
3. Companion groups rx_start_lead_with_tx, tx_start_lead_with_rx, overflow_restart_gap_with_tx in provider_groups with no value keys, after the value groups; skipped with `skipped_no_opposite_direction` when needed.
4. run-record.sh names slot A when raw probes run or any preflight test is requested, plus every selector tx. Checked by hand: slot-B-only selector without preflight → only 0:B:cbx:0; with preflight → slot A and B.
5. transmit_waveform(n) scales pn by 0.875 and asserts |z| ≤ 0.5 before sending; used by duplex stages and companions; existing pn() unchanged.
6. Scripted ProbeDevice RxRecv::Failed timeout/loss → latch and later calls blocked; ordinary failure does not latch; rows P960, P961.
7. UR-34 latch/lifecycle, companion-group, amplitude topics with *Test:*.
Tests: a_latched_guard_reads_nothing_from_the_device_and_leaks_it_when_dropped, a_latched_record_calls_no_opener, an_open_of_a_stream_the_guard_still_sees_open_is_refused_before_the_device, a_forwarded_receive_timeout_or_loss_latches_and_blocks_later_calls, the_waveforms_of_the_added_transmissions_stay_within_half_a_unit, a_companion_that_fails_after_a_solo_measurement_keeps_the_solo_value, a_companion_group_is_skipped_with_an_observation_when_the_layout_lacks_the_opposite_direction.
Mutation: P913–P961 (47 rows), new P954–P961, re-pointed P945, P946; all 47 killed; rules.py every old once.
Checks: workspace 1 876 passed; clippy, docker clippy clean; rules.py 0; diff --check, bash -n ok; 28 rehearsals 20/20.
Open: hardware.rs flow, a latch from a probe's own handle, real open() not exercised on the fake; describe cache has no row; att prompt checked by hand.

# Fix round 3 (after Sol re-review 2 FAIL: 2 majors)
1. Guard records each open stream's channel list (empty = channel 0). An open whose channels intersect a DIFFERENT open stream's channels in the same direction latches and is refused; a same-(direction, stream) re-open replaces the entry and is forwarded (hw_b8_raw_restart_lead unchanged). After-group check kept. Tests: a_reopen_of_the_same_stream_and_a_disjoint_channel_set_are_allowed, an_open_of_channels_another_open_stream_holds_is_refused_before_the_device.
2. A selector with only one of rx/tx is refused before device access: parse_selector/process_metadata and run-record.sh (exit 2 before the prompt; checked by hand). UR-34, bench.md: "both or neither".
Mutation: all 50 rows P913–P964 killed; P954 re-pointed; new P962–P964. Checks: workspace 1 877 passed; clippy host and docker clean; rules.py 0; diff --check, bash -n ok; 14 rehearsals 20/20.
