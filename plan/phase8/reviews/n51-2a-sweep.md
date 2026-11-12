# Note 51 stage 2a — delay-injection sweep (sub-step 7, 2026-11-13)


## `fake`

baseline: 463 tests, 0 failed [], 143 s

| config | finished | seconds | tests | failed | changed vs baseline |
|---|---|---|---|---|---|
| now-5 | yes | 136 | 463 | 3 | 3 |
| now-20 | yes | 133 | 463 | 7 | 7 |
| now-60 | yes | 129 | 463 | 12 | 12 |
| now-200 | yes | 132 | 463 | 12 | 12 |
| advance-5 | yes | 143 | 463 | 1 | 1 |
| advance-20 | yes | 142 | 463 | 2 | 2 |
| advance-60 | yes | 144 | 463 | 2 | 2 |
| advance-200 | yes | 151 | 463 | 2 | 2 |
| submit-5 | yes | 140 | 463 | 2 | 2 |
| submit-20 | yes | 134 | 463 | 4 | 4 |
| submit-60 | yes | 133 | 463 | 6 | 6 |
| submit-200 | yes | 134 | 463 | 8 | 8 |

Tests whose verdict changed (the verdict at each config where it differs; first failure text):

| test | where | first failure text |
|---|---|---|
| `common::measurement::tests::rx_start_lead_group_runs_on_the_fake` | submit-20: FAILED, submit-200: FAILED, submit-60: FAILED | thread 'common::measurement::tests::rx_start_lead_group_runs_on_the_fake' (2288362) panicked at crates/ezsdr-radio-uhd/tests/common/measurement.rs:4141:9: / a SHORT candidate: [Object {"candidate": Number(3000000), "classes": Object {"AdmissionRejection": Number(0), "DeviceFailure": Number(0), "Inco |
| `n47s5_front_rehearsal_on_the_fake` | now-20: FAILED, now-200: FAILED, now-5: FAILED, now-60: FAILED, submit-20: FAILED, submit-200: FAILED, submit-5: FAILED, submit-60: FAILED | rehearsal T2 1 samples at 1000000 S/s: Inconclusive("admitted and no reported faults; the on-time part is INCONCLUSIVE: no tone onset in the capture (or a hole before it)") / rehearsal T2 2 samples at 1000000 S/s: Inconclusive("admitted and no reported faults; the on-time part is INCONCLUSIVE: no to |
| `n47s5_held_tail_and_stop_rehearsal_on_the_fake` | now-20: FAILED, now-200: FAILED, now-60: FAILED, submit-20: FAILED, submit-200: FAILED, submit-60: FAILED | T3 (f) C at e+0, after_join None: submitted -30.932 ms after J(e); refusals []; late drops [] ns / rehearsal T3 (f) C at e, booked before J(e): Inconclusive("1 device burst(s) as expected, each acknowledged once, no reported faults; admitted and no reported faults; the on-time part is INCONCLUSIVE:  |
| `n47s5_long_close_rehearsal_on_the_fake` | now-200: FAILED, now-60: FAILED, submit-200: FAILED, submit-60: FAILED | rehearsal long close at 1000000 S/s: Fail("1 radio.TIME_ERROR, 0 expected") / thread 'n47s5_long_close_rehearsal_on_the_fake' (791106) panicked at crates/ezsdr-radio-uhd/tests/fake.rs:7932:9: / 1000000 S/s: Fail("1 radio.TIME_ERROR, 0 expected") |
| `n47s5_one_sample_after_a_burst_refusal_rehearsal_on_the_fake` | advance-20: FAILED, advance-200: FAILED, advance-60: FAILED, now-20: FAILED, now-200: FAILED, now-5: FAILED, now-60: FAILED, submit-200: FAILED | B8 burst at end + 1 samples, booked late false: Admitted { coercions: [], warnings: [], dispatched: [ActionId(2)] }; refusals ["UR-21: the burst overlaps an accepted booking"]; TIME_ERROR []; async [{"channel":0,"code":"BurstAck","stream":0,"tick":429537,"tx_flush":true},{"channel":0,"code":"BurstAc |
| `n47s5_repeat_switch_rehearsal_on_the_fake` | now-20: FAILED, now-200: FAILED, now-60: FAILED | T4 switch: row Some("applied"), boundary Some(2062395) samples (before the loop delay), loop delay None ticks, seams Some(0), sends [(true, false), (false, false), (false, false), (false, false), (false, false), (false, false), (false, false), (false, false), (false, false), (false, false), (false,  |
| `n47s5_untimed_rehearsal_on_the_fake` | now-200: FAILED, now-60: FAILED | rehearsal T5 (1): Inconclusive("the first untimed repeat is admitted and not refused; admitted and no reported faults; no tone after it: the transmit is not shown") / T5 (2): the untimed burst placed at sample Some(2144154) after the timed one at 2090989; onset None; loop delay None samples / rehear |
| `p70_one_sample_burst_then_timed_burst_rehearsal_on_the_fake` | now-200: FAILED, now-60: FAILED, submit-200: FAILED, submit-60: FAILED | thread 'p70_one_sample_burst_then_timed_burst_rehearsal_on_the_fake' (797110) panicked at crates/ezsdr-radio-uhd/tests/fake.rs:8099:80: / 1 samples: no send at A's instant 409286600: [] |
| `s1_short_first_sends_rehearsal_on_the_fake` | now-200: FAILED, now-60: FAILED, submit-200: FAILED | S1 first send of 1 samples: TIME_ERROR [Object {"cause": String("late"), "late_by_ns": Number(3041000), "outcome": String("drop"), "target": Object {"domain": Object {"local": Number(4), "node": Number(0)}, "ticks": Number(2044294)}}]; TX_UNDERFLOW 0; async [{"channel":0,"code":"BurstAck","stream":0 |
| `ur_21_a_refused_configure_refuses_its_burst_through_the_kernel` | advance-20: FAILED, advance-200: FAILED, advance-5: FAILED, advance-60: FAILED, now-20: FAILED, now-200: FAILED, now-5: FAILED, now-60: FAILED, submit-20: FAILED, submit-200: FAILED, submit-5: FAILED, submit-60: FAILED | thread 'ur_21_a_refused_configure_refuses_its_burst_through_the_kernel' (476676) panicked at crates/ezsdr-radio-uhd/tests/fake.rs:1963:5: / assertion `left == right` failed: [] ["set_sources internal internal", "set_time_zero now", "serial", "set_subdev_spec rx A:0", "set_subdev_spec tx A:0", "apply |
| `ur_21_s18_an_untimed_repeat_on_a_playing_repeat_is_refused_without_time_error` | now-20: FAILED, now-200: FAILED, now-60: FAILED | thread 'ur_21_s18_an_untimed_repeat_on_a_playing_repeat_is_refused_without_time_error' (681239) panicked at crates/ezsdr-radio-uhd/tests/fake.rs:2090:5: / [TimeErrorPayload { cause: Late, outcome: SendAsap, late_by_ns: 6000000, target: TimePoint { domain: ClockDomainId { node: NodeId(0), local: 4 }, |
| `ur_23_a_burst_after_a_stop_targets_the_one_clock` | now-20: FAILED, now-200: FAILED, now-60: FAILED | thread 'ur_23_a_burst_after_a_stop_targets_the_one_clock' (685703) panicked at crates/ezsdr-radio-uhd/tests/fake.rs:2282:5: / ["UR-21: the late policy dropped the burst"] |
| `ur_29_a_loss_found_by_the_shutdown_s_report_read_ends_its_ack_window` | now-200: FAILED, now-60: FAILED | thread 'ur_29_a_loss_found_by_the_shutdown_s_report_read_ends_its_ack_window' (923654) panicked at crates/ezsdr-radio-uhd/tests/fake.rs:3814:5: / the shutdown never read a report with a timeout |


## `uhd`

baseline: 31 tests, 0 failed [], 6 s

| config | finished | seconds | tests | failed | changed vs baseline |
|---|---|---|---|---|---|
| now-5 | yes | 6 | 31 | 0 | 0 |
| now-20 | yes | 6 | 31 | 0 | 0 |
| now-60 | yes | 6 | 31 | 0 | 0 |
| now-200 | yes | 7 | 31 | 1 | 1 |
| advance-5 | yes | 6 | 31 | 0 | 0 |
| advance-20 | yes | 6 | 31 | 0 | 0 |
| advance-60 | yes | 6 | 31 | 0 | 0 |
| advance-200 | yes | 7 | 31 | 0 | 0 |
| submit-5 | yes | 6 | 31 | 1 | 1 |
| submit-20 | yes | 6 | 31 | 1 | 1 |
| submit-60 | yes | 6 | 31 | 1 | 1 |
| submit-200 | yes | 6 | 31 | 1 | 1 |

Tests whose verdict changed (the verdict at each config where it differs; first failure text):

| test | where | first failure text |
|---|---|---|
| `ea_16_untimed_repeat_after_stop_starts_at_the_stop_region_end` | now-200: FAILED, submit-20: FAILED, submit-200: FAILED, submit-5: FAILED, submit-60: FAILED | thread 'ea_16_untimed_repeat_after_stop_starts_at_the_stop_region_end' (2935557) panicked at crates/ezsdr-acceptance/tests/uhd.rs:500:5: / assertion `left == right` failed: the repeat and the one after the Stop: [Object {"actual_start": Null, "blocks": Number(49), "end": String("stop"), "late_by": N |


## `protocol`

baseline: 79 tests, 0 failed [], 8 s

| config | finished | seconds | tests | failed | changed vs baseline |
|---|---|---|---|---|---|
| server-5 | yes | 8 | 79 | 0 | 0 |
| server-20 | yes | 9 | 79 | 0 | 0 |
| server-60 | yes | 11 | 79 | 0 | 0 |
| server-200 | yes | 19 | 79 | 0 | 0 |

No test's verdict differs from the baseline at any point.


## Failing sets by config (what a new reader needs to compare with `plan/phase8/51-test-timing.md` §5.2)

| binary | config | failing tests |
|---|---|---|
| fake | now-5 | `n47s5_front_rehearsal_on_the_fake`, `n47s5_one_sample_after_a_burst_refusal_rehearsal_on_the_fake`, `ur_21_a_refused_configure_refuses_its_burst_through_the_kernel` |
| fake | now-20 | `n47s5_front_rehearsal_on_the_fake`, `n47s5_held_tail_and_stop_rehearsal_on_the_fake`, `n47s5_one_sample_after_a_burst_refusal_rehearsal_on_the_fake`, `n47s5_repeat_switch_rehearsal_on_the_fake`, `ur_21_a_refused_configure_refuses_its_burst_through_the_kernel`, `ur_21_s18_an_untimed_repeat_on_a_playing_repeat_is_refused_without_time_error`, `ur_23_a_burst_after_a_stop_targets_the_one_clock` |
| fake | now-60 | `n47s5_front_rehearsal_on_the_fake`, `n47s5_held_tail_and_stop_rehearsal_on_the_fake`, `n47s5_long_close_rehearsal_on_the_fake`, `n47s5_one_sample_after_a_burst_refusal_rehearsal_on_the_fake`, `n47s5_repeat_switch_rehearsal_on_the_fake`, `n47s5_untimed_rehearsal_on_the_fake`, `p70_one_sample_burst_then_timed_burst_rehearsal_on_the_fake`, `s1_short_first_sends_rehearsal_on_the_fake`, `ur_21_a_refused_configure_refuses_its_burst_through_the_kernel`, `ur_21_s18_an_untimed_repeat_on_a_playing_repeat_is_refused_without_time_error`, `ur_23_a_burst_after_a_stop_targets_the_one_clock`, `ur_29_a_loss_found_by_the_shutdown_s_report_read_ends_its_ack_window` |
| fake | now-200 | `n47s5_front_rehearsal_on_the_fake`, `n47s5_held_tail_and_stop_rehearsal_on_the_fake`, `n47s5_long_close_rehearsal_on_the_fake`, `n47s5_one_sample_after_a_burst_refusal_rehearsal_on_the_fake`, `n47s5_repeat_switch_rehearsal_on_the_fake`, `n47s5_untimed_rehearsal_on_the_fake`, `p70_one_sample_burst_then_timed_burst_rehearsal_on_the_fake`, `s1_short_first_sends_rehearsal_on_the_fake`, `ur_21_a_refused_configure_refuses_its_burst_through_the_kernel`, `ur_21_s18_an_untimed_repeat_on_a_playing_repeat_is_refused_without_time_error`, `ur_23_a_burst_after_a_stop_targets_the_one_clock`, `ur_29_a_loss_found_by_the_shutdown_s_report_read_ends_its_ack_window` |
| fake | advance-5 | `ur_21_a_refused_configure_refuses_its_burst_through_the_kernel` |
| fake | advance-20 | `n47s5_one_sample_after_a_burst_refusal_rehearsal_on_the_fake`, `ur_21_a_refused_configure_refuses_its_burst_through_the_kernel` |
| fake | advance-60 | `n47s5_one_sample_after_a_burst_refusal_rehearsal_on_the_fake`, `ur_21_a_refused_configure_refuses_its_burst_through_the_kernel` |
| fake | advance-200 | `n47s5_one_sample_after_a_burst_refusal_rehearsal_on_the_fake`, `ur_21_a_refused_configure_refuses_its_burst_through_the_kernel` |
| fake | submit-5 | `n47s5_front_rehearsal_on_the_fake`, `ur_21_a_refused_configure_refuses_its_burst_through_the_kernel` |
| fake | submit-20 | `rx_start_lead_group_runs_on_the_fake`, `n47s5_front_rehearsal_on_the_fake`, `n47s5_held_tail_and_stop_rehearsal_on_the_fake`, `ur_21_a_refused_configure_refuses_its_burst_through_the_kernel` |
| fake | submit-60 | `rx_start_lead_group_runs_on_the_fake`, `n47s5_front_rehearsal_on_the_fake`, `n47s5_held_tail_and_stop_rehearsal_on_the_fake`, `n47s5_long_close_rehearsal_on_the_fake`, `p70_one_sample_burst_then_timed_burst_rehearsal_on_the_fake`, `ur_21_a_refused_configure_refuses_its_burst_through_the_kernel` |
| fake | submit-200 | `rx_start_lead_group_runs_on_the_fake`, `n47s5_front_rehearsal_on_the_fake`, `n47s5_held_tail_and_stop_rehearsal_on_the_fake`, `n47s5_long_close_rehearsal_on_the_fake`, `n47s5_one_sample_after_a_burst_refusal_rehearsal_on_the_fake`, `p70_one_sample_burst_then_timed_burst_rehearsal_on_the_fake`, `s1_short_first_sends_rehearsal_on_the_fake`, `ur_21_a_refused_configure_refuses_its_burst_through_the_kernel` |
| uhd | now-5 | — |
| uhd | now-20 | — |
| uhd | now-60 | — |
| uhd | now-200 | `ea_16_untimed_repeat_after_stop_starts_at_the_stop_region_end` |
| uhd | advance-5 | — |
| uhd | advance-20 | — |
| uhd | advance-60 | — |
| uhd | advance-200 | — |
| uhd | submit-5 | `ea_16_untimed_repeat_after_stop_starts_at_the_stop_region_end` |
| uhd | submit-20 | `ea_16_untimed_repeat_after_stop_starts_at_the_stop_region_end` |
| uhd | submit-60 | `ea_16_untimed_repeat_after_stop_starts_at_the_stop_region_end` |
| uhd | submit-200 | `ea_16_untimed_repeat_after_stop_starts_at_the_stop_region_end` |
| protocol | server-5 | — |
| protocol | server-20 | — |
| protocol | server-60 | — |
| protocol | server-200 | — |
