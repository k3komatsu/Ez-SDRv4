# Test plan rev 5: design note 48, S2 (delete the timed-setting paths and the mechanisms whose reason is gone)

Base: `main` at `c58cb32`, tree clean. This document changes no tree file. Rev 3 applies the orchestrator's rulings R1–R10 (rev 2) and S1–S12 (Round 2) in `rulings-plan.md`, and records the Round 3 rulings T1–T3 (§0.13–0.15). The rev 3 text as first written is kept as `test-plan.v3.md`. Nothing here was run: every claim is from reading the code and from grep at `c58cb32` (VERIFIED) or is marked INFERRED. Sizes are estimates from reading, not counts of a diff. Rev 2 is kept as `test-plan.v2.md`; rev 1 as `test-plan.v1.md`. The scripts used for the R3 ledger and the anchor checks are `s2plan/ledger.py` and `s2plan/anchors.py` in this directory; their outputs are `s2plan/ledger.json` and `s2plan/anchors.out`.

## Ruling map (rulings to the sections that changed)

| Ruling | Sections changed in rev 3 |
|---|---|
| R1 frequency history, front end | §1.1 (N10 and N11 defined), §4 G1, §5 G1 |
| R2 halt rule as a public plan | §1.9 (N6, the replacement at `tests/timeline.rs:540`), §5 G5 |
| R3 mutation ledger by location | §2.1 (S3 decisions added), §2.2 (N-09, N-10 killers) |
| R4 report reader, fake.rs D9 test | §1.2 (D9 table), §1.6 (split of `core.rs:1570`), §4 G2, §5 G2 |
| R5 G6 per boundary | §0.9 (N7b dropped, S5), §1.8, §3.7, §5 G6 |
| R6 spec ordering, §3.3, UC-6, bench | §3.3 (S6 lines added), §3.9 (new: S6 rows), §3.8 (probes) |
| R7 `owner_step` trace | §1.12 (N8 optional, S7), §5 G7 |
| R8 cancellation rename | §1.11 (one `ClockEnded`, S8), §7 Q1 |
| R9 receive origin | §1.8 (N9 values, S9), §0.12 |
| R10 four-way split | §6 (S2a-i conditions, S10 probes and `tune_dsp`), §3.8, §7 Q3 |
| S1 N10 through `rx_recv`; N11 mutation at `device.rs:1306–1310` | §1.1, §2.2 (N-13) |
| S2 `tests/timeline.rs:540`, `:469`, `:677` through the public plan | §1.9 |
| S3 sweep: decided rows, missing tests in the map | §2.1, §1.2 (added entries) |
| S4 delete the P7 assertion at `fake.rs:7693–7695` | §1.2 (D9 table) |
| S5 drop N7b; `tests/measurement.rs:76–82, 155` retargeted in S2b-i | §0.9, §1.8, §5 G6, §6 |
| S6 spec lines added with their sub-step | §3.9 (new) |
| S7 N8 optional | §1.12, §5 G7, §7 Q2 |
| S8 no `FirstSendShort`; one `ClockEnded` | §1.11, §7 Q1 |
| S9 N9 values: ceil and floor differ | §1.8 |
| S10 keep `hw_b8_raw_timed_tune_phase`; keep `tune_dsp`/`tune_auto`; keep `StreamState.configured` | §3.8, §7 Q3, Q5 |
| S11 delete P863; keep P985 retargeted; N-10 killer; N-11 killer; N-12 at S2a-i; drop N4 | §2.1, §2.2, §1.6, §7 Q6–Q8 |
| S12 fake.rs 748–1423 not deleted as a block; flows keep `Dsp.log`, `tx_header`, `tx_header_stale` | §1.2, §6 S2a-i |
| T1 split S2a-i into α (queue measurement group) and β (interface change, FIFO, D9, N1/N2/N10/N11, A1′, A5); α first | §0.13, §6 (S2a-i-α, S2a-i-β), §1.2 (α/β marks), §2.1 (α/β marks on rows), §2.2 (N-rows by sub-step) |
| T2 `hw_b8_raw_timed_tune_phase` on `UhdDevice::tune_auto` with `at`; `hw_b8_raw_timed_retune` deleted | §0.14, §3.8, §7 Q10, Q11 (closed) |
| T3 dropping N4 accepted | §0.15, §1.6, §5 G2, §7 Q9 (closed) |

**Deviations and corrections found while applying the rulings (for the orchestrator to rule):**
- **S8 applied (no `FirstSendShort`).** The short first send (`tx.rs:765`) is cancelled as `ClockEnded`. Rev 2's deviation is withdrawn.
- **R7 (Nothing arm):** the arm has no production path (§5 G7). It is deleted in S2b-ii after N8 (optional, S7).
- **Correction, new in rev 3:** rev 2 §1.7 said `Channels::without` has no caller in `crates`. `crates/ezsdr-radio-uhd/src/provider/rx.rs:1707` (`Channels::first(2).without(0)`) is a caller (VERIFIED by grep at `c58cb32`). Its test is retargeted in S2a-ii before the method is deleted (§1.7).
- **Correction, new in rev 3:** rev 2 §1.1 and §6 put `fake.rs:1361` (`raw_group_limit_stops_every_later_call`) in the S12 "keep, strip queue assertions" list. It calls `measure_command_queue_depth` directly (`fake.rs:1363`), so it is retargeted to the raw group's own measurement, and its queue assertions go (§1.2).
- **Interpretation of S11 "drop N4" (flagged):** the test N4 is dropped as a separate test. The order claim it held (flush, then ACK read, then the timed send) keeps its first two links in the renamed reader test (§1.6, assertion `flush < ack`). The third link (timed send after the ACK) is no longer pinned by a test, because the gate that enforced it is removed by A4. This is open question 9 (§7).
- **Interpretation of S10 for `hw_b8_raw_timed_retune` (flagged):** S10 keeps only `hw_b8_raw_timed_tune_phase`. The retune probe calls the trait's timed `apply` (`hardware.rs:839`), so it is deleted with the trait change (rev 2 position). Open question 10 (§7).

## 0. Findings that change the brief's plan

1. **Forced order: A2's measurement group leaves before A1.** The queue-depth measurement drives the FakeDevice's timed queue through `Device::apply(..., Some(at))`: `crates/ezsdr-radio-uhd/tests/common/measurement.rs:1278` (VERIFIED). A1 removes `at` from `apply`, so the queue group goes in S2a-i with A1. The rest of A2 (key, envelope member, payload, `COMMAND_QUEUE_FULL`, schema, measurement key list) stays in S2b-i.

2. **`owner_step` cannot reach `Nothing` from `by: None`.** `core.rs:618` reads `state.configured.is_some() && state.configured == by`. With `by == None` the guard is false, so a T0 segment always gets `Owner::Configure`. The test `core.rs:2093` pins this (VERIFIED).

3. **`Owner::Nothing` has no production path (R7 trace, VERIFIED by reading, §5 G7).** The only production caller of `owner_configure` is `provider/rx.rs:354` (grep: the other hits are tests at `rx.rs:1419, 2461, 2626–2634`). `Nothing` returns `Ok(true)` at `core.rs:1325–1327`. `from` moves past a segment only after `Ok(true)` (`rx.rs:362`), and `Rx::new` is called once per stream when a Run spawns its receive thread (`provider/mod.rs:646`, read; INFERRED: one spawn per Run). So the same segment is seen again only with a new `Rx` whose `from` is reset (`rx.rs:145`, `i64::MIN` when there is no first stream), and no production path does that. The unit test `rx.rs:2629` is the only caller that repeats the call.

4. **`Owner::Open` is reachable (VERIFIED).** A failed open at `core.rs:1331–1335` records `DeviceSet::Unknown` through `record_open(None)` (`core.rs:644`, set at `:635`). A later segment's `owner_step` then falls through to `_ => Owner::Open` (`core.rs:624`). A refused start is cleared by a Stop and a new start (`fake.rs:7966`, the test named `ur_25_a_refused_start_is_fixed_by_a_stop_a_new_value_and_a_start`; its body is not read here). The `DeviceSet::Open(other)` case is reached only if the channel set changes within a Run, which RM-21 refuses (VERIFIED in the spec, not traced in code).

5. **`COMMAND_QUEUE_FULL` has no production emitter.** Its uses: the constant and table row (`crates/ezsdr-radio/src/lib.rs:110, 215`), the payload type (`lib.rs:802`, registered `lib.rs:843`), `radio_model.rs:15, 173, 380`, and the measurement evidence kinds (`tests/common/measurement.rs:318, 449, 4102, 4643`) (VERIFIED by grep).

6. **`Recovery::Halted` has no production reader.** `needs_reopen` (`tx.rs:149`) tests only `ReopenNeeded`; `recovery()` is `#[cfg(test)]` (`tx.rs:144–147`). The readers are tests at `tx.rs:1320` and `:1546` (VERIFIED).

7. **`Line::halted` has test callers only.** `timeline.rs` tests at 469, 513, 540, 677 (VERIFIED). Its body reads `State.halted` (`timeline.rs:428, 470`). `refused_running` (`timeline.rs:443, 473`) is the second flag. A3 keeps the rule as a public plan (§1.9, R2).

8. **`apply_delay` has one user: `tests/common/measurement.rs:5054`** (VERIFIED). It is read at `device.rs:1488` on every FakeDevice `apply`. Removing the queue measurement leaves it without a test, so N1 pins it.

9. **G6 answer per boundary (R5, VERIFIED):**
   - *Envelope:* `TimingEnvelope` has `#[serde(deny_unknown_fields)]` (`crates/ezsdr-radio/src/lib.rs:536`), and `schemas/radio/envelope.v1.json` has `additionalProperties: false`. A document with `command_queue_depth` or `release_window_ns` is refused, not migrated.
   - *Binding selectors:* the removed keys are not in `x3xx::SELECTOR_KEYS` (`crates/ezsdr-radio/src/x3xx.rs:20`) or the UHD test list (`crates/ezsdr-radio-uhd/tests/common/mod.rs:111`). The generic refusal refuses them: UHD `provider/mod.rs:213–214`, Mock `lib.rs:353–355`. No selector carries a removed key, so no selector test is added (S5: N7b dropped).
   - *Measurement comparison:* `ezsdr-radio/src/measurement.rs:405–406` rejects a key with no rule (`rule(name)` is `None`) as "unknown Radio Model capability key".
   - *Fixtures (found by this revision):* `crates/ezsdr-radio/tests/fixtures/measurement/invalid-comparisons.json` and `records/{valid,uhd-only-rejects,mock-only-rejects}.json` each carry two `measured` entries with the removed keys (lines 28–29). `valid.json` would stop being valid once the keys are refused.
   - *Schema text:* `measurement.v1.json` does not name the removed keys (grep, VERIFIED).

10. **`design/06:335` has no queue or release term (VERIFIED).** It is KC-27, the coordinator's configuration record. Rev 1 cited it; that citation is withdrawn (§3.4).

11. **`design/09` MR-27 documents the `applied` section as "always empty"** (`design/09-mock-radio.md:179`, VERIFIED), which matches the code (`ezsdr-mock-radio/src/lib.rs:396` inserts `[]`). Removing it is a deletion of the section and its count (seven to six), not a correction.

12. **Receive origin (R9, formula read at `timeline.rs:538` before B3; VERIFIED).** The origin formula after B3 is `ceil_to_grid(max(effective, cut + lead, max(item.ready, cut + ceil(call × old_ratio) + allowance) + lead))`, with the profile lead unchanged at 50 ms. The lattice step (`ceil_div(earliest, num) * num`, `timeline.rs` `begin`) is read in §1.8; N9's literal is computed from it.

13. **Round 3, T1: S2a-i is split into α and β (orchestrator ruling, 2026-11-05).**
    - **α (the queue measurement group, first):** `measure_command_queue_depth` and everything only it uses (`crates/ezsdr-radio-uhd/tests/common/measurement.rs`), the `MeasurementPlan` queue fields (`queue_trials`, `queue_max_trials`, `queue_ahead_ns`, `queue_max_calls`) with their env parsing (`EZSDR_MEASURE_QUEUE_TRIALS`, `EZSDR_MEASURE_QUEUE_MAX_TRIALS`) and validation, the queue tests of `tests/fake.rs` and `measurement.rs` (§1.2 S12 block, §1.2 retargets), the hardware probe `hw_b8_raw_queue_depth` and its bench step (`bench.md:143–147`), the mutation rows of §2.1 for the queue group, and the UR-34 raw-queue sentences. The `Device` trait keeps `at`; `apply` still takes `Some(at)` in the kept tests, so the tree is green without β.
    - **β (the interface change):** `at` leaves `Device::apply` and `rx_stop`, the DSP FIFO and flows (A1), `rx_start_now` (A5), A1′ (`rx_stop`'s `at`), N1, N2, N10, N11 and N-13, the D9 test, `hw_b8_raw_timed_tune_phase`, `hw_b8_raw_timed_retune` and `hw_b8_raw_rx_timed_stop`. A4 and the rest of A5 are S2a-ii. Their tests are written in β, not in α.
    - **Why α first (T1):** R10 said the queue measurement removal comes with the timed `apply` removal. α removes the queue group's timed `apply` calls before `apply`'s `at` changes, and leaves `at` in place, so the step is green on its own. R10's condition was "not after", which α satisfies.
    - **Consequence for α's tests:** α adds no new test. N1, N2, N10 and N11 belong to β (T1 names them there). §6's "Tests added" list for S2a-i is therefore β's list.
    - **Consequence for the key:** after α, `radio.timing.command_queue_depth` stays a comparable key that no group measures, so `unmeasured_values` still emits it with a reason. Its reason text (`measurement.rs:224`) names the removed measurement and the step that removes the key (S2b-i). The key itself leaves in S2b-i (§1.8, fixtures, schema).
14. **Round 3, T2: `hw_b8_raw_timed_tune_phase` uses `UhdDevice::tune_auto` with `at` (`uhd.rs:559`); `hw_b8_raw_timed_retune` is deleted.** Both `UhdDevice` bench methods (`tune_dsp`, `tune_auto`) stay (Q3). The probe's timed step moves to `tune_auto` in β, because the probe's timed call is the trait's `apply`, which β removes. `hw_b8_raw_timed_retune` is deleted with the trait change in β (its claim is the timed receive retune's start; the kept phase probe covers the timed RF retune; §7 Q10 closed by T2).
15. **Round 3, T3: N4 is dropped (accepted).** The flush → ACK → payload order runs in sequence inside the opener, so no separate test pins its third link. Open question 9 is closed by T3: no test is added (§5 G2, §7 Q9).

## 1. Retained / moved / deleted map, grouped by what is checked

Notation: **keep** = unchanged; **move** = the assertion goes to a named test; **delete** = removed, with the covering test named; **retarget** = the same test asserts the new name or the new observable.

### 1.1 Untimed settings, frequency history and read-back (G1; R1)

Stays: `ur_33_the_antenna_is_read_back_after_an_untimed_change` (`device.rs:2675`); the untimed half of `ur_33_an_untimed_write_lands_at_once_and_a_waiting_entry_overwrites_it_later` (`device.rs:3258`; the assertion at `:3268` "the untimed write is at once" stays, `:3270–3271` go with the DDC FIFO); the `freq_at` history checks of ur_33 tests that use no timed command. `freq_at` (`device.rs:1129`) stays.

Moved, R1 (S2a-i, written before the old tests go):
- **N1** `untimed_apply_sleeps_apply_delay_on_the_host_clock` (G1, S2a-i): a FakeConfig with `apply_delay` set and a manual host clock; one untimed `apply`; the host clock advances by exactly `apply_delay`. Failure it catches: the sleep moves or disappears. (Manual clock: no wall-clock verdict.) Keeps `apply_delay` tested after the queue measurement that used it goes (§0.8).
- **N2** `start_rx_staged_values_are_read_back_and_refused_as_today` (G1, S2a-i): stopped receive stream, untimed `apply` of frequency, gain and antenna, then `start_rx` with staged values; asserts coercion, read-back and refusal (`UR-12`) as before. This pins G1's coercion and refusal part, which no current test isolates from the timed queue.
- **N10** `untimed_write_in_a_loopback_burst_splits_the_received_samples` (S1, S2a-i, through `rx_recv`, not `freq_at`): a loopback transmit burst is running and the receive stream is reading it; in the middle of the burst, an untimed `apply` of frequency f2 is written. The samples `rx_recv` returns before the write carry the burst's frequency f1; the samples after it do not carry f1. Replaces the assertions of `ur_33_a_later_write_does_not_change_samples_already_produced` (`device.rs` ~3194–3210), which is deleted with the DDC FIFO (its claim moves here, R1). Failure it catches: a later write rewrites produced samples. Killing row: P987 (`device.rs:543`, `freq_of`), retargeted to N10 (§2.1).
- **N11** `untimed_front_end_value_follows_the_spec` (S2a-i): a spec sets a front end; an untimed `apply` of a frequency gives a value that follows the spec's front end. This is the one untimed claim of `ur_33_a_timed_command_follows_its_front_end_across_a_spec` (`device.rs:2641`), which is deleted. Killing rows: P232 (`device.rs:1478`, `apply`), retargeted to N11 only if its old text survives A1, otherwise deleted (§2.1); and N-13 (`set_subdev_spec`'s settings remap, `device.rs:1306–1310`, §2.2), which N11 must also kill.
- R1's front-end preservation assertion has its named destination: N11.

Deleted, with covering test named: `ur_33_a_timed_command_follows_its_front_end_across_a_spec` (2641; its claim moves to N11).

### 1.2 Timed DSP command FIFO and DUC/DDC flow (A1)

Deleted, with the covering test named where one exists:
- The `device.rs` tests that assert a timed frequency waits in the DDC or DUC FIFO: `ur_33_a_timed_ddc_value_waits_for_an_input_sample_at_or_after_its_time` (3086), `ur_33_a_ddc_takes_one_entry_per_radio_rate_tick` (3104), `ur_33_decimation_does_not_change_the_consumption_rate` (3149), `ur_33_a_flowing_ddc_consumes_while_the_host_does_not_read` (3173), `ur_33_a_later_write_does_not_change_samples_already_produced` (3194; claim moves to N10), `ur_33_a_stopped_ddc_takes_nothing` (3215), `ur_33_a_full_ddc_refuses_the_next_timed_write` (3236), `ur_33_a_ddc_fifo_outlives_its_streamer` (3276), `ur_33_a_timed_duc_value_is_taken_only_while_has_time_holds` (3363), `ur_33_an_untimed_burst_takes_no_duc_entry` (3399), `ur_33_a_full_duc_is_a_model_that_breaks_the_time_value_pairing` (3414), `ur_33_each_chain_has_its_own_fifo` (3445), `ur_33_a_fresh_device_has_empty_fifos` (3438; body not read), `ur_33_a_stream_that_starts_now_takes_a_due_command_at_its_first_sample` (3460), `ur_33_a_stale_tx_header_advances_the_dsp_fifo_on_logical_ticks` (3119).
- `ur_33_a_command_due_while_its_front_end_was_out_never_applies` (2354).
- `ur_33_the_fake_device_keeps_the_devices_queues` (2690): **split**. The untimed queue shape is kept if its assertions are untimed; the timed-queue half is deleted. The body is read in S2a-i.
- The covering test for the reachable part of the DUC is the data path: samples, underflow, EOB and SOB headers (below). The DDC FIFO has no production caller left, so its failure cases are unreachable (INFERRED from A1: the only writer is the timed `apply`, which has no production caller; VERIFIED for `at: Some` in production: none, `core.rs:1031`, `rx.rs:287, 405`).

Kept (sample and header behaviour without the FIFO): `ur_33_a_timed_start_at_the_previous_burst_s_end_is_late` (2782), `ur_33_a_one_sample_eob_loses_the_next_timed_header` (2808), `ur_33_idle_checks_each_tx_packet_and_accepts_a_future_continuation` (2835), `ur_33_a_burst_that_runs_dry_underflows` (2865), `ur_33_a_late_continuation_without_eob_advances_the_duc_before_radio_drops_the_next_packet` (2885), `ur_33_a_late_eob_ends_the_duc_before_a_future_timed_burst` (2916), `ur_33_a_duc_timeline_survives_radio_underflow` (2947), `ur_33_an_active_timed_sob_keeps_the_duc_sample_timeline` (2976), `ur_33_an_untimed_sob_clears_duc_time_until_a_fresh_burst` (3301), `ur_33_an_untimed_two_zero_eob_clears_the_stale_duc_header` (3326), `ur_33_a_one_sample_eob_has_no_burst_ack` (3350). Each body is checked in S2a-i for `apply(..., Some(..))` or `rx_stop(..., Some(..))`; a hit moves that assertion to an untimed call.

**`fake.rs` queue block (S12; the block is not deleted as a whole).** `fake.rs:748–1423` holds the timed-queue tests and also the lease, restore, record assembly, `ur_7_retries…`, the probe-array, `coercion_*` and `raw_group_*` tests. Each test in the block is named in S2a-i and is one of:
- **delete**: a queue test whose only claim is the timed queue (its name and body are queue-only);
- **strip**: a test with queue assertions beside a kept claim; only the queue assertions go. `raw_group_sends_nothing_after_a_failed_clock_read` (`fake.rs:1352`) is stripped. Its queue assertions, if any, are named in S2a-i;
- **retarget**: `raw_group_limit_stops_every_later_call` (`fake.rs:1361`) calls `measure_command_queue_depth` (`fake.rs:1363`), which is deleted. Its claim (a limit stops every later call) is kept on the raw group's own measurement, and the call changes to it (correction, §0 deviations);
- **keep**: the lease, restore, record assembly, `ur_7_retries…`, probe-array, `coercion_*` and `raw_group_*` tests not listed above.

**Flows (S12).** "Delete the flows" (the DDC/DUC flow parts of A1) keeps `Dsp.log` (`device.rs:507–527`, read by `dsp_log`), and keeps `tx_header` and `tx_header_stale` with their update inside the flows block (`device.rs` ~2004–2012, `chain.tx_header = Some(…)` and `chain.tx_header_stale = false`). These are the stale-header state of G2 and P5/D70.

**D9 test `ur_33_d9_raw_probes_match_the_x3xx_tx_model` (`fake.rs:7654–7707`), disposition by assertion (R4).** The timed-tune assertions go; packet-state, starvation and D70 assertions stay. The test's `tune` closure (`fake.rs:7656`) passes `at` to `apply`, so it becomes untimed in S2a-i, and the helpers in `tests/common/d9.rs` change with it (§7 Q4).

| fake.rs line | Assertion | Decision |
|---|---|---|
| 7661–7663 | P5 stale header: `old_header >= e`, control `None`, flush ack | keep (packet state) |
| 7667–7668 | P5 timed `dsp_log` ticks equal start, e, e | **delete** (timed tune, DSP log) |
| 7672 | `before_calls` | keep |
| 7673 | `before_log = dsp_log(..).len()` | **delete** (the DSP log) |
| 7676–7677 | P6 timed `dsp_log` ticks at e | **delete** (timed tune) |
| 7678 | P6 every `tx_send` carries its first sample's time | keep (packet header time, not the FIFO) |
| 7679–7680 | starvation reports `Underflow` and `TimeError` | keep |
| 7683 | late continuation absent from loopback | keep |
| 7687 | `before` = `dsp_log` length | **delete** |
| 7690–7692 | `p7_log` (the `dsp_log`); P7 `dsp_log` untimed flags; `dsp_pending == 1` | **delete** (DSP log and FIFO count) |
| 7693–7695 | P7 tone sample unchanged "through the timed command" (its message; the sample is read at `p7.start + 25 ms`) | **delete** (S4, the ruling; the message names a timed command, and the sample-history claim that survives is N10's, through `rx_recv`). Its body is read again in S2a-i to confirm nothing else in it is kept |
| 7699–7707 | D70 one-sample EOB ack, early start, two-zero control honours the start | keep |

**Report-reader exclusion (R4)** is split out of `ur_22_t3_flush_ack_is_a_timed_dsp_barrier` in §1.6.

Also: `ur_33_a_timed_command_follows...` and `ur_33_d9...` above are the only tests whose name changes; the other kept tests keep their names.

### 1.3 Timed receive stop (A1′)

- `ur_33_a_timed_receive_stop_stops_the_stream_at_once` (`device.rs:2261`): delete. Its claim (the stream stops at once) moves to `ur_33_a_stopped_stream_s_tail_comes_before_the_next_stream` (2280), retargeted to an untimed `rx_stop` (body read in S2a-i).
- The `rx_stop(at)` callers in production: none (`rx.rs:287, 405` pass `None`, VERIFIED). Tests that pass `Some` are in `tests/fake.rs` and `provider/test_support.rs` (grep count 12, not each read; S2a-i lists them).
- `ur_24_nonloss_rx_stop_failure_leaves_the_chain_uncertain` (`rx.rs:991`): its `Some` argument goes, its assertion stays (G5).

### 1.4 Receive restart rate and the `rx_start_now` path (A5)

- `ur_24_rx_recovery_start_uses_uhd_stream_now` (`crates/ezsdr-radio-uhd/src/uhd.rs:1461`): delete with `rx_start_now`. `rx_start_now` has only the `start_rx(None)` branch (`device.rs:1193–1239`) and no production caller (VERIFIED).
- `rx_start_now` wrappers in `tests/common/measurement/multichannel.rs:237, 419`, `tests/common/probe_device.rs:429`, and `tests/fake.rs` (3437, 3686, 3914, 4244, 4496, 4660, 4861, and later; grep count) are deleted with the trait method.

### 1.5 Transmit reopen failure (A5′, G4)

Current: `reopen_failed` (`tx.rs:180–183`) sets `DeviceBurst::Closed` and `Recovery::Halted`. The change sets `Recovery::Clean` and keeps `Closed`. `Recovery::Halted` is deleted.

- `tx.rs:1310–1330` (`tx_chain_recovery_transitions`, the assertion at `:1320`): **retarget** to `Clean`; the burst-closed assertion stays.
- `tx.rs:1546` ("a failed reopen enters Halted"): **retarget**. The observable replaces the state name: after a failed reopen no held or new payload is sent, and the clock is `None` (G4).
- New test **N3** `failed_reopen_sends_no_held_or_new_payload` (G4, S2a-ii, written first): drives `reopen_streamer` to fail with a held burst and a new send queued; asserts the device sees no `tx_send` for either, `clock() == None`, and the cancellation reports as today under its new name (§1.11). Failure it catches: the held payload is resent, or a new send is accepted on the failed chain. Killing rows: N-04, N-11 (§2.2).
- `ur_22_nonterminal_reopen_failures_cancel_held_initial_bursts` (P1206, `tx.rs:855`): keep. Its `halted` is a local variable (`tx.rs:835`), not `Recovery::Halted`.

### 1.6 Transmit DSP gates (A4) and the report-reader exclusion (R4)

- `Core.tx_dsp_gates` (`core.rs:735, 819`) and `lock_tx_dsp` (`core.rs:1065–1069`) are deleted. Callers: `core.rs:1097, 1190, 1200, 1599`; test uses at `1377, 1382, 1387, 1390`.
- Kept per the inventory: `tx_report_gates`, `lock_tx_reports` (`core.rs:1070` onwards), the generation split, the flush's ACK and deadline, and the report store (`core.rs:1072–1181`).
- **Split of `ur_22_t3_flush_ack_is_a_timed_dsp_barrier` (`core.rs:1570`, R4):**
  - (a) *Report-reader exclusion (kept, renamed `ur_22_t3_flush_ack_is_not_taken_by_shutdown`):* a competing shutdown reader cannot take the flush ACK (`core.rs:1605–1647`). It does not depend on a DSP gate. Removed from it: the timed-DSP thread (`core.rs:1593–1600`, `lock_tx_dsp` at `:1599`, `apply(..., Some(10_000))` at `:1600`), and its assertion at `:1630–1631` ("timed TX DSP crossed the open-time flush wait", "no timed write reached the device before the flush ack"). The `apply` count goes with the thread; the order assertion at `:1645` is kept in the reduced form `flush < ack` (the first two links of the order: flush, then ACK read).
  - (b) *The third link (timed send after the ACK) is not pinned by a test (S11: N4 dropped).* The order `flush < ack < apply` at `:1642–1645` loses its `apply` link with the thread. Open question 9 (§7).
- The assertion at `:1377` ("the opener must hold the timed-DSP gate through its flush acknowledgement") is deleted with the gate.
- Killing rows for `lock_tx_reports` in the reader and the reopen (N-09, N-10, §2.2) are the renamed reader test run over both `open_tx_device` and `reopen_tx_streamer` (S11). The test body calls `open_tx_device` (`core.rs:1585`); a second run calls the reopen path with the same shutdown reader. The reopen run is written in S2a-ii.
- `ur_22_t3_stop_interrupts_an_open_flush_wait` (`core.rs:1884`), `ur_22_t3_flush_attribution_is_per_channel_and_code_bounded` (`core.rs:1774`): kept unchanged.

### 1.7 `Channels::without` (A5)

- `timeline.rs:52` `pub fn without(self, channel)` (`crates/ezsdr-radio/src/timeline.rs:52`): no production caller (VERIFIED: `ezsdr-server`'s `without_feeds` and the kernel's `without_relations` are unrelated).
- **Correction (rev 3):** one test caller exists: `crates/ezsdr-radio-uhd/src/provider/rx.rs:1707` (`let config = Config { channels: Channels::first(2).without(0), … }`, VERIFIED by grep). Rev 2's "no caller" was wrong. S2a-ii replaces the call with the channel set the test needs (the body is read in S2a-ii; the test's claim is kept), then deletes `without`.

### 1.8 Timeline release window, receive restart origin and fixtures (B3, G3, R5, R9)

Deleted: the release term at `timeline.rs:538` (`cut + i128::from(stream.release_window)`), `Stream.release_window` (`timeline.rs:104`), the `release_window` argument at every `Stream` literal (about 20 sites: `rx.rs:1166, 1476, 1535, 2272`, `core.rs:1949`, `control.rs:164`, `differential.rs:64`, `ezsdr-mock-radio/src/lib.rs:1462`, `ezsdr-mock-radio/tests/timeline.rs:193, 197`, `ezsdr-radio/tests/timeline.rs:32, 37, 81, 99`), `check_start_lead` (`ezsdr-radio/src/lib.rs:561–566`), `TimingEnvelope.release_window_ns` and its key (`lib.rs:57, 545`), `keys::RELEASE_WINDOW_NS`.

Retained with the new origin (G3): the ready term is `cut + ceil_div(call, den) + allowance`, rounded to the grid by the existing `timed_instant`/`lead` path. The configure-complete warning (`timeline::lead_left`, `START_LEAD_SHORT`) and the missed-start path are unchanged. The profile start lead is **not** lowered (owner decision; x3xx `rx_start_lead_ns` stays 50 ms, `x3xx.rs:384–390`; R9).

Tests:
- `rm_25_a_receive_successor_adds_release_window_before_call_and_lead` (`ezsdr-radio/tests/timeline.rs:96`): **retarget** and rename to `rm_25_a_receive_successor_adds_one_call_and_allowance_before_lead`. The old input `release_window: 3 * MS` is removed.
- `ur_24_a_cut_receive_uses_one_short_packet_and_preserves_successor_lead` (`provider/rx.rs:868`, sets `rx_start_lead_ns = release_window_ns` at ~`:880`): **retarget** to the new terms.
- `rm_25_a_virtual_segment_ignores_a_stale_delivered_index` (378), `rm_25_a_refusal_does_not_move_a_later_origin` (433), the rest of the `rm_25_*` refusal tests (480–719): kept; check each body for a `release_window` argument while editing.
- Delete (the start-lead floor is gone, so there is no failure case left): `rm_6_the_start_lead_check_cases` (`lib.rs`, P1070, P1071), `rm_6_every_published_description_keeps_the_start_lead_at_or_above_the_release_window` (`channels.rs:863`), `rm_6_building_a_description_refuses_a_lead_below_the_window_x3xx` (`channels.rs`, P1072), `rm_6_a_measurement_timing_with_a_start_lead_below_the_window_is_refused` (`mod.rs`, P1073), the RM-6 part of `measurement_plan_from_env_uses_defaults_and_rejects_invalid_counts` (P1074), `channels.rs:848–923` (RM-6 with-lead tests), `fake.rs:533–591` (`RM-6` tests). The `rx_start_lead_ns` value stays checked by the `mr_03`/`ur_09` tests.
- Mock: `mock_radio.rs:540, 567` (`release_window_ns` capability and `timing["rx_start_lead_ns"] >= release_window_ns`): delete the release assertion; keep the lead assertion.
- Measurement fixtures and their tests (R5, S2b-i): delete the two `measured` entries for `radio.timing.command_queue_depth` and `radio.timing.release_window_ns` (lines 28–29) in `invalid-comparisons.json`, `records/valid.json`, `records/uhd-only-rejects.json` and `records/mock-only-rejects.json`. Each file's expected result is checked after the edit: `valid.json` must stay valid. `crates/ezsdr-radio/tests/measurement.rs:501–502` (the LowerBound and UpperBound rules for the two keys) is deleted in S2b-i. `crates/ezsdr-radio/tests/measurement.rs:76–82` (the `command_queue_depth` value-rule cases in `rm_35_each_value_rule_…`) and `:155` (the `command_queue_depth` entry in `rm_35_invalid_comparison_fixture_is_rejected_for_each_rule`) are **retargeted** to `radio.tx.repeat_max_samples` in S2b-i, not deleted (S5). They are not in S2a-i: that crate is the Radio Model, and its key stays until S2b-i. (Rev 2 §6 listed them in S2a-i; corrected.)
- `crates/ezsdr-radio/tests/channels.rs:438` (the ideal description's `command_queue_depth` equals `u32::MAX`), `:848` (the lead and window helper), `:883` (lead at least window) and `:888` (ideal equals zero): edited in S2b-i. Line 438 and the queue-depth part go with A2; `:848–888` go with B3.

New tests:
- **N5** `receive_successor_origin_is_old_rate_call_plus_allowance_plus_lead` (G3, S2b-i): the origin the UHD Provider and MockRadio compute for the same cut agree, through the differential harness `differential.rs` (VH). Failure it catches: the Providers disagree.
- **N9** `a_receive_origin_with_a_fractional_call_rounds_once_to_the_grid` (R9, S9, S2b-i, in the shared timeline test `crates/ezsdr-radio/tests/timeline.rs`, next to `rm_25_*`). Values (S9: ceil and floor of the origin differ): old rate 3 MS/s (one sample is 1000/3 ns, `ratio` 1000/3), call 1 sample, allowance 3 ms, lead 3 ms, new rate 200 MS/s (lattice 5 ns), a Stop at 10 000 003 ns (`cut`) and a Start at the same instant.
  - Hand computation: the call term is ceil(1 × 1000/3) = 334 ns; ready = cut + 334 + 3 000 000 = 13 000 337; earliest = max(e, cut + lead, ready + lead) = 16 000 337. Rounded up to the 5 ns lattice: **16 000 340**. Rounded down it would be 16 000 335, so the two differ.
  - The literal is 16 000 340. The test asserts the literal, not the production formula.
  - Kills: a dropped call term (ready 13 000 003 → origin 16 000 003, which rounds to 16 000 005), a dropped allowance, a dropped lead, and a rounding that goes to the floor.
  - **Read before the literal is written:** the lattice step is `ratio.num()` of the new config (`timeline.rs` `begin`, `ceil_div(earliest, num) * num`, VERIFIED). The name `ceil_to_grid` in rev 2 does not exist at `c58cb32`; the function is that expression. The 5 ns value assumes `ratio` of 200 MS/s is 5/1 (INFERRED; checked by the test's first run).
  - N9 is in the shared timeline, so both Providers and MockRadio are covered (R9: Provider agreement alone cannot see a shared bug).

### 1.9 Halt rule for receive streams (A3, G5, R2)

- `Line::halted` (`src/timeline.rs:249–256`) and `State.halted` (`src/timeline.rs:428, 443, 470`): deleted.
- Tests that assert `halted` are retargeted to the public plan (`planned`, `Line::plan`, and `made()` at `src/timeline.rs:243`) and to `streaming()` (admission) separately. No test reads `refused_running` or `halted` after the change. Refusal exclusion is observed through the plan of the items with `e <= t`, i.e. a line booked with only those items (S2), never through private state.
  - `tests/timeline.rs:469` (`assert_eq!(after.halted(item.e), direct.halted(item.e))`, seeded replay): **retarget** to `after.plan` and `direct.plan` of the items with `e <= item.e`, and to `streaming()` on both, seeded.
  - `tests/timeline.rs:513` (`booked.halted(item.e) == after.halted(item.e)`): **retarget** to the plan of `booked` and `after` booked with the items `e <= item.e`.
  - `tests/timeline.rs:535–540` (the 2.050 s origin; the prefix plan): **retarget** (S2). At 1 999 ms the plan of the line booked with the items `e <= 1 999 ms` is `[(0, Some(500 ms))]` and `!streaming()`; at 2 000 ms the plan of the line booked with the items `e <= 2 000 ms` is `[(0, Some(500 ms)), (2 050 ms, None)]` and `streaming()`. The 2.050 s origin stays at its time. (The assertion `line.halted(1_999 ms) && !line.halted(2_000 ms)` is replaced by these two plan checks.)
  - `tests/timeline.rs:677` (prune equivalence, `pruned.halted(t) == full.halted(t)`): **retarget** to `pruned.plan` against `full.plan` at each `t` with the items `e <= t`, and to `streaming()` on both.
- **N6** `halt_rule_is_streaming_and_refusal_exclusion` (G5, S2b-ii, written first): over the seeded sequences of the existing test, asserts (a) admission: a refused `start_rx` leaves `streaming()` true, a later `Stop` and `start_rx` make it false and then true (checked separately); (b) exclusion: a refusal in effect with no start is absent from the public plan's segments. This is the explicit replacement that lets `halted` go. No private state is read.

### 1.10 Mock `applied` section (A7) and stale text (A6)

- `ezsdr-mock-radio/src/lib.rs:396` (the `applied` section) and its assertion `mock_radio.rs:552`: the section is deleted. The `instance().sections` count drops from seven to six (MR-27, `design/09-mock-radio.md:177`).
- The `advance` comment at `ezsdr-mock-radio/src/lib.rs:1779–1798` (A6): **reword** to the current behaviour; no test.
- Rows naming `applied`: decided in §2.3. F21 and F30 are the fault flag `self.faults[index].applied` (`lib.rs:2302` and `:755`), not the section: kept.

### 1.11 Cancellation reason rename (B5, R8)

- `CancelReason::ColdChange` (`ezsdr-radio/src/lib.rs:127`) is renamed to `ClockEnded` for the two causes that end a clock: an orphaned clock (`tx.rs:348`, "its transmit clock was never started") and a clock ended by a reopen failure (`tx.rs:858` and `:874`, "cancelled because the TX streamer could not be reopened").
- **One name for all three constructors (S8; no `FirstSendShort`).** `tx.rs:765` (`refuse_first_send`: "the first send needs 4 samples per channel before the clock's cut") also cancels a held clock: the short first send is cancelled because the clock's cut comes before it can start. It is `ClockEnded`, like the other two constructors. Rev 2's deviation is withdrawn (§7 Q1 closed).
- Mapping kept, counts unchanged: `cancellation_reports` (`lib.rs:132`) becomes `matches!(reason, CancelReason::ClockEnded)`, which is what `ColdChange` alone gave today. The event counts (`lib.rs:857`, `(CancelReason::ColdChange, true)`) become `(CancelReason::ClockEnded, true)`, with the same count.
- Sites renamed in S2b-ii: `lib.rs:127, 132, 857`; `tx.rs:348, 765, 858, 874`; mutation rows P561, P562 (old and new text), P563, P565, P566 (Mock `vh_8_*`, new text), P569, P570, P1209 (`tx.rs`, the new or old text). Test names: `cancellation_reports_only_cold_change` (`lib.rs`), `vh_8_*`, `ur_26_shutdown_after_loss_cancels_held_bursts_record_only` (P570), `ur_22_underflow_close_failure_reopens_before_the_next_payload` (P1208, P1209).
- Tests: retarget the names; the assertions stay. `ur_26_stop_cancels_the_held_burst_record_only` (`tx.rs:3490`) is kept unchanged (G5).

### 1.12 `owner_step` (B7, R7)

- The `Nothing` branch is deleted (§5 G7). `owner_step` returns `Configure` for a set that is open and `Open` otherwise. Its `by` parameter then has no reader, so it goes too (`core.rs:614`). `owner_configure`'s `Ok(true)` branch for `Nothing` (`core.rs:1325–1327`) goes with it.
- `StreamState.configured` (`core.rs:368`) keeps its other reader: the transmit reopen round trip (`tx.rs:826`, `:839`) and the record in `record_configured` (`core.rs:663–664`). Whether that round trip still needs the mark is open (§7 Q5).
- Test `ur_36_the_owner_step_follows_the_record` (`core.rs:2081`): the five assertions. Keep the `Open` for nothing open, the `Configure` for open-not-configured, `Open` for another set, `Configure` for another command and for `None`. Delete the `Nothing` assertion ("this command's configuration", no longer reachable).
- `owner_configure`'s second-call test (`rx.rs:2629`, "the second call finds the segment configured"): **delete the second call and its assertion** (the one that pins `Nothing` through the caller); the first call's `Ok(true)` assertion stays. (Body read in S2b-ii.)
- Mutation rows P368 (`core.rs:619`, `Nothing` to `Configure`) and N-03 (the guard at `core.rs:618`): deleted with the arm (§2.1, §2.2).
- If N8 finds a repeat `configured` probe in a production-path run, the arm stays: P368 and N-03 are kept, and this section is reversed.
- **S7:** the caller trace (§5 G7) is the decision. N8 is an optional cheap pin: if it is run, a repeat `configured` in a production-path run is a counterexample to the trace and the arm stays; no run of N8 is needed to delete the arm.

### 1.13 Spec-only and schema-only items

The measurement and envelope tests are in §1.8 and §0.9. Schemas are §3.7.

## 2. Mutation rows (R3)

### 2.0 Method and commands (run again by the orchestrator)

Row IDs are not unique. In the six `mutations.json` files there are 1 776 rows with 1 712 distinct IDs; 62 IDs appear in two or more files (for example `F21` at `plan/maintenance/tools/mutations.json:73`, the Mock row, and `plan/phase6/tools/mutations.json:205`, the server row). Rows are therefore keyed by **(file, id)**. A row's decision is taken from where its `old` and `old2` text sits, and its `filter` names the killing test. The earlier rev 1 method (by id or name match) is withdrawn.

Commands, run from the repository root (`~/local_work/Ez-SDRv4`), read-only:

```
PYTHONDONTWRITEBYTECODE=1 python3 ~/.cache/ezsdr-fixes/2026-11-05-n48s2/s2plan/ledger.py > ~/.cache/ezsdr-fixes/2026-11-05-n48s2/s2plan/ledger.json
PYTHONDONTWRITEBYTECODE=1 python3 ~/.cache/ezsdr-fixes/2026-11-05-n48s2/s2plan/anchors.py | tee ~/.cache/ezsdr-fixes/2026-11-05-n48s2/s2plan/anchors.out
grep -rn --include=mutations.json -E '"id": ?"(F21|F30|P1307|P1310|P1209)"' plan
```

`ledger.py` lists 482 rows whose `old`, `old2`, `new`, `new2` or `filter` text matches a removed mechanism's name, or whose containing function has a queue, DSP, halt, release, cancel, reopen, owner or measurement name. The function-name part is over-inclusive (common names such as `prepare`, `new`, `check`), so the list is a search aid and not a disposition. The decisions are in §2.1. Any other row whose `old` text is removed fails `anchor.py` (each row must match exactly once, AGENTS §7) at its sub-step, and is then deleted or retargeted there, so no row is lost silently.

Anchor results at `c58cb32` (`anchors.out`): N-01 1 match (`timeline.rs:538`); N-03 1 (`core.rs:618`); N-04 1 (`tx.rs:881`); N-05 1 (`timeline.rs:473`); N-06 1 (`lib.rs:536`); N-09 1 (`core.rs:1191`); N-10 1 (`core.rs:1201`); N-11 1 (`tx.rs:842`); N-12 1 (`device.rs:594`); ColdChange `lib.rs` 2 matches at `:132, :857` (the variant at `:127` is written without `CancelReason::`); `tx.rs` 4 at `:348, 765, 858, 874`.

### 2.1 Dispositions by location (R3)

**Delete (the row's old text is in code the step removes, or its killer test is removed):**

| Row(s) | File:line (old text) | Why | Step |
|---|---|---|---|
| P832; P844–P847 | `measurement.rs:1292` (`queue_trial_calls`); `:1477–1480` (`measure_command_queue_depth`) | queue group, timed `apply` | S2a-i |
| P864 | `measurement.rs:1328` (`untimed_error` in the queue trial) | queue-only code | S2a-i |
| P862, P863, P895, P911 | `measurement.rs:1311` (queue-only `last_accepted_at`), `:1082` (shared raw latch) and the rows named in S3 | P863's old text includes the queue-only `last_accepted_at` (S11: delete P863; its shared-latch part goes with it, §7 Q6). P862, P895 and P911 are deleted by S3 (their old text or killer is in the queue group) | S2a-i |
| P871; P1044 | `measurement.rs:1398`, `:1394` (`summarize_queue_depth`) | queue group | S2a-i |
| P894 | `measurement.rs:1316` (`queue_trial_calls`) | queue group | S2a-i |
| P1001–P1002 | `measurement.rs:1407` (`queue_index_agreement`); `:1229` (`queue_trial_calls`) | queue group | S2a-i |
| P1038–P1040 | `measurement.rs:1383` (`queue_condition_done`); `:1445` (`measure_command_queue_depth`) | queue group | S2a-i |
| P807–P813; P835–P837; P840–P843; P868–P869; P999–P1000 | `measurement.rs` (`classify_queue_trial`, `finish_queue_trial`, `queue_streams_rx`, `drain_receiver`, `summarize_queue_depth`, `queue_depth_value`) | queue group | S2a-i |
| P826–P830; P1041–P1043 | `measurement.rs:656–782` (queue-trial plan fields in `default`, `count`, `validate`) | queue plan fields (A2) | S2a-i |
| P1175–P1178 | `core.rs:1189`, `:1199` (`let _dsp = self.lock_tx_dsp(...)`) | A4 gates. Replaced by N-09 and N-10 | S2a-ii |
| P978, P980, P982, P977, P979, P981, P983, P984, P988 | `device.rs:501, 1497, 1091, 1059, 1462, 1078, 1615, 1229, 1468` | the DDC/DUC FIFO and its drain (A1) | S2a-i |
| P1224 | `d9.rs:225` (`p4_dsp_only`) | pre-stop DDC entry (A1) | S2a-i |
| P1183 | `device.rs:594` (`chain.tx_header = None;`, new text adds `chain.fifo.clear()`) | the FIFO clear is A1. Replaced by N-12 | S2a-i |
| P1307 | `tx.rs:180` (`reopen_failed`, `Recovery::Halted`) | `Halted` is removed (A5′) | S2a-ii |
| P1310 | `tx.rs:841` (`reopen_failed` to `reopen_succeeded`) | after A5′, `reopen_failed` sets `Clean`, so this mutant is equivalent (R3). Replaced by N-11 | S2a-ii |
| P90, P460 | `timeline.rs:469` (`self.halted = item.refused`) | `State.halted` is removed (A3). Replaced by N-05 | S2b-ii |
| J65, J70 | `timeline.rs:253` (`Line::halted` body) | A3 | S2b-ii |
| P368 | `core.rs:619` (`Nothing` to `Configure`) | the `Nothing` arm goes (R7, §1.12). Replaced by nothing: the arm has no production path | S2b-ii |
| N-03 (new, below) | `core.rs:618` | the same arm | S2b-ii |
| P68 | `channels.rs:359` (`command_queue` in the ideal description) | A2 remainder | S2b-i |
| P1070, P1071; P1072; P1073; P1074 | `lib.rs:562` (`check_start_lead`); `channels.rs:373`; `mod.rs:346`; `measurement.rs:2578` (`candidate_timing`) | the start-lead floor (B3) | S2b-i |
| P1218 | `timeline.rs:538` (pre-B3 release term) | the B3 change itself. Replaced by the retargeted J06 | S2b-i |
| P1219, P1220 | `control.rs:164`; `lib.rs:1462` (`release_window` in a `Stream` literal) | B3 | S2b-i |

**Retarget (the same killing test, the new text):**

| Row | Old text (new form) | New killer or name | Step |
|---|---|---|---|
| J06 | post-B3 `ready.max(cut + ceil_div(call, den) + i128::from(stream.allowance))`; new `ready.max(cut + i128::from(stream.allowance))` (`timeline.rs:538`). Pre-B3 it reads `cut + i128::from(stream.release_window) + ceil_div(...) + ...` | `rm_26_the_timeline_cases`, unchanged | S2b-i (rewrite the old text in the B3 commit) |
| P561 | `matches!(reason, CancelReason::ClockEnded)` (`lib.rs:132`; the old text is renamed too; S8) | `cancellation_reports_only_cold_change` (renamed) | S2b-ii |
| P562 | old text as P561; new `matches!(reason, CancelReason::Stop)` | same | S2b-ii |
| P563, P565, P566 | the `new` text uses `CancelReason::ClockEnded` (Mock, `lib.rs:1499, 2262, 1865`) | `vh_8_*` | S2b-ii |
| P569, P570 | `new` text `ClockEnded` (`tx.rs:323, 1205`) | `vh_8_*`, `ur_26_shutdown_after_loss…` | S2b-ii |
| P1209 | old and new text `ClockEnded` (`tx.rs:874`) | `ur_22_underflow_close_failure_reopens_before_the_next_payload` | S2b-ii |
| P987 | `freq_of`, `device.rs:543` | N10 (§1.1) | S2a-i |
| P232 | `device.rs:1478` (`apply`) | N11 if its old survives A1, else deleted (§1.1) | S2a-i |
| P1206 | `tx.rs:855` (`halted && false`, local) | unchanged: the local is not `Recovery::Halted` | — |

**Keep (verified by location; the old text is not in removed code):**
- P1341 (`mod.rs:512`): `applied` appears only in the test name `uhd_59_initial_values_are_applied_before_the_streamers_open`. Keep.
- F21 (`crates/ezsdr-mock-radio/src/lib.rs:2302`) and F30 (`:755`): the fault flag `self.faults[index].applied`, not the `applied` section (`:396`). Keep.
- Rows whose `old` is in the DSP-independent packet and header code (the EOB, SOB and underflow tests kept in §1.2), for example P1182, P1185, P1187, P1192, P1200, P1204 (`device.rs:614–748`): keep.
- Rows whose old text is in `raw.halted()` shared raw-group code (the raw latch): keep unless the row's old1 is queue-only (P863 above).

**S3 decisions (fixed in the plan, before code; rows read by their `file` and `old` at `c58cb32`, VERIFIED with a script that prints each row's id, file, `filter`, `old` and `new`):**
- *Retarget (old text stays in kept code; its killer changes):* **P215** (`device.rs`, `streamer.next = Some(at)` in the `None` arm, filter `ur_33_two_receive_streamers_keep_their_own_channels`) and **P219** (`device.rs`, the UR-25 reopen comment and its code, filter `ur_33_one_channel_is_opened_by_one_streamer`). Killing test: the kept test of the same receive path, named in S2a-i when its body is read (the body changes with the `apply` or `rx_stop` signature). **P985** (`device.rs:639`, `if at.is_none() {` in the DUC packet path, filter `ur_33_a_timed_duc_value_is_taken_only_while_has_time_holds`): retargeted to the kept packet-state test that covers the packet's own time, not the FIFO entry (Q7 keep; the row is kept only if its `if` stays a packet decision after S2a-i reads the body).
- *Rows with a kept killer, nothing to change:* **P865, P866, P867** (`measurement.rs`, filter `raw_group_limit_stops_every_later_call`; the killer is retargeted by §1.2, the row stays); **P872, P873, P890, P900, P901, P902, P903, P904, P905, P906, P909, P912** (raw-group and lease rows, filters are kept tests).
- *Rows with a deleted killer or a deleted anchor: deleted:* **P862** (`queue condition` text, queue group), **P895** (filter `a_receiver_whose_samples_pass_the_last_command_completes_the_trial`, the trial's advance of queue time), **P911** (filter `the_command_thread_begins_no_call_after_the_drain_latches`, the drain latch of the queue group), **P863** (S11, §2.1 table).
- *Row P990* (`device.rs`, `self.end_rx_flow(&mut st, stream)` in the subdev-spec replacement, filter `ur_33_replacing_the_subdev_spec_stops_the_receive_flow`): the flow call goes with the flows, so the row's `old` is deleted; its killer test is a kept one only if it asserts the receive streamers are cleared, which is decided when the body is read in S2a-i. The row is deleted or retargeted there; no other choice.
- *Row P369* (`core.rs`, `_ => Owner::Open,` to `Owner::Nothing`, filter `ur_13_a_failed_open_by_the_owner_is_opened_again`): **retarget** its `new` to `Owner::Configure` (S3; the `Nothing` arm goes in S2b-ii, the row's `new` must name a kept variant).
- *Missing tests added to the map (S3):* `rx.rs:850` `ur_24_fake_ddc_distinguishes_host_silence_from_a_stalled_ddc` (uses `RxFlowStall`, the DDC flow: **delete** its `RxFlowStall` branch and its `pending` assertion, keep the `Silence` branch); `provider/mod.rs:871` `ur_22_arm_waits_for_t3_flush_ack_before_timed_tx_dsp` (**delete**: a timed DSP wait; its claim is the reader exclusion, kept in §1.6); `provider/tx.rs:1344` `ur_22_tx_owner_reopen_waits_for_t3_flush_ack_before_timed_tx_dsp` (**delete** the timed-DSP wait; keep the owner-reopen ACK ordering `flush < ack` as in §1.6; the reopen run is in S2a-ii); `tests/fake.rs:7631` `ur_24_d9_p4_a_and_c_keep_a_pre_stop_ddc_entry_until_restart_samples` (**delete**: a pre-stop DDC entry, A1); `tests/fake.rs:6514` (a `dsp_log` assertion in a restart loop, `ur_25`/`ur_33` family; **strip** the `dsp_log` message and keep the `freq_at` assertion, which is the frequency history of G1).
  These five rows are read in S2a-i before their edits; a body that differs from this summary is reported.

**Decide at the sub-step by the rule above (not yet read):** P985 (`device.rs:639`, `if at.is_none()` in the DUC packet path, filter `ur_33_a_timed_duc_value…`): keep if `at` is the TX packet's time, delete if it is the FIFO entry (open, §7 Q7); P1052 (`timeline.rs:472`, `replay`); P870, P893, P897, P898, P907, P908, P910 (raw-group rows with `queue_trial`, `halted`, `deadline` names: decided by location at S2a-i); P216, P1341-style rows where an `at` or `rx_stop(... Some)` may sit in a retained test's body.

### 2.2 New rows (Rust token anchors; each must match exactly once through `anchor.py`)

| Row | Anchor (old → new) | Killing test | Status | Step |
|---|---|---|---|---|
| N-01 | `ceil_div(call, den) + i128::from(stream.allowance)` → `ceil_div(call, den)` (`timeline.rs:538`) | N5, N9, retargeted `rm_25_a_receive_successor…` | 1 match now; kill power unverified | S2b-i |
| N-03 | `if state.configured.is_some() && state.configured == by {` (`core.rs:618`) | `ur_36_the_owner_step_follows_the_record` | 1 match; **deleted with the arm** unless N8 says otherwise | S2b-ii |
| N-04 | `self.at = self.plan.len();` (`tx.rs:881`) → `self.at = 0;` | N3 | 1 match | S2a-ii |
| N-05 | `self.refused_running = item.refused;` (`timeline.rs:473`) → `self.refused_running = false;` | N6, the retargeted `timeline.rs:535–540` test | 1 match | S2b-ii |
| N-06 | `#[serde(deny_unknown_fields)]` removed from `TimingEnvelope` (`lib.rs:536`) | N7a | 1 match | S2b-i |
| N-09 | `let _reports = self.lock_tx_reports(k);` (`core.rs:1191`, open) → `let _reports = ();` | the renamed reader test `ur_22_t3_flush_ack_is_not_taken_by_shutdown`, over `open_tx_device` (N4 dropped, S11) | 1 match; kill power unverified | S2a-ii |
| N-10 | the same gate in the reopen (`core.rs:1201`) | the same reader test, over `reopen_tx_streamer` (S11: the reader test is run on both paths; N-10 needs this killer) | 1 match; kill power unverified | S2a-ii |
| N-11 | `self.chain.reopen_failed();` removed (`tx.rs:842`, before `halted = true;`) | N3, and the assertion at `tx.rs:1545` (S11: kept for N-11) | 1 match; kill power unverified | S2a-ii |
| N-12 | `chain.tx_header = None;` removed (`device.rs:594`) | retargeted `ur_22_t3_flush_clears_the_header_but_does_not_drain_timed_dsp_fifo` (header assertion only) | 1 match at c58cb32; re-check after S2a-i edits `device.rs` (S11: confirm at S2a-i) | S2a-i |
| N-13 | `set_subdev_spec`'s settings remap (`device.rs:1306–1310`): `map_or_else(|| Channel::default_of(dir), |i| old_settings[i].clone())` → `map_or_else(|| Channel::default_of(dir), |_| Channel::default_of(dir))` (a kept front end loses its settings) | N11 (S1); N11 must fail under it | 1 match at `c58cb32` (grep, VERIFIED); kill power unverified | S2a-i |

Dropped from rev 1: N-02 (the same mutation as J06), N-07 (`i128::from(stream.call)` does not exist), N-08 (P1206 covers the held-burst loop). N-01 and N-02 in rev 1 used pre-B3 text and could not match.

### 2.3 Rows checked, not yet decided

The list in §2.1 "decide at the sub-step" is the only open part of the ledger. Each row must be dispositioned in the step review record of its sub-step, with the anchor result.

## 3. Spec and documentation changes

Each row needs a `Changes` row (the table at the end of each spec). The `design/` text is one topic in a few lines with a `*Test:*` line (AGENTS §2); `rules.py` must pass. Withdrawn rules keep a one-line stub. Every row below names the sub-step that changes its contract (R6).

### 3.1 `design/07-radio-model.md` (S2b-i)

| Rule / line | Change | Sentence |
|---|---|---|
| RM-6 (line 89) | **reword** | "the eight `radio.timing.*` capabilities" → "the six"; delete "the queue depth," and "the release window,"; delete the sentence "A Description's `radio.timing.rx_start_lead_ns` is at least its `radio.timing.release_window_ns`; a Provider builds no Description, and a measurement instance takes no candidate timing, that breaks this." |
| RM-6 Test line (line 91) | **reword** | remove the RM-6 start-lead test names (`rm_6_…`, `channels.rs:863`, `fake.rs:533`) |
| table row 72 (`radio.timing.command_queue_depth`) | **delete** | |
| table row 73 (`radio.timing.release_window_ns`) | **delete** | |
| table row 70, `min_timed_command_lead_ns` | keep | |
| RM-20 (line 208) | **reword** | delete `command_queue_depth` and `release_window_ns` from the listed timing members |
| RM-25 (line 257) | **reword** | "After a cut, readiness includes the release window, one receive call at the old rate and the delivery allowance; the start lead follows those terms." → "After a cut, readiness is one receive call at the old rate and the delivery allowance; the start lead follows those terms." |
| RM-25 Test line (line 267) | **reword** | `rm_25_a_receive_successor_adds_release_window_before_call_and_lead` → the renamed test (§1.8) |
| RM-21 (line 237) | keep | `Line::streaming` is the name in its 2026-11-05 row; `Line::halted` is not in the rule |
| RM-10 event table (line 119, `radio.COMMAND_QUEUE_FULL` row) | **delete** the table row | |
| RM-10 paragraph (line 123, "No Provider emits `radio.COMMAND_QUEUE_FULL`") | **reword**: delete the clause | |
| RM-11 payload table (line 133, `radio.COMMAND_QUEUE_FULL` row) | **delete** | |
| line 232 (schema list) | **reword**: delete `command_queue_full_payload` from the list | |
| RM-34 (line 330) | keep (already a withdrawal stub) | |
| table lines 337–338 (`LowerBound` and `UpperBound` keys) | **reword**: remove `release_window_ns` from the `LowerBound` list and `command_queue_depth` from the `UpperBound` list | |
| Changes table | **add** a row, 2026-11-05 | "note 48 S2: the queue depth and the release window leave the TimingEnvelope; COMMAND_QUEUE_FULL is removed; the receive restart origin has no release term" → `plan/phase8/48-simplification-inventory.md` |

### 3.2 `design/09-mock-radio.md` (S2b-ii for MR-27 and the `applied` section; S2b-i for the timing rows)

| Rule / line | Change | Sentence |
|---|---|---|
| line 22 (INFERRED note: "the command-queue depth of 16") | **reword** | delete the queue-depth clause (S2b-i) |
| line 53 (`release_window_ns` = 3 000 000) | **delete** the row (S2b-i) | |
| line 74 (`command_queue_depth` = 4 294 967 295) | **delete** the row (S2b-i) | |
| line 73 (the zero list) | **reword**: delete `release_window_ns` from the list (S2b-i) | |
| MR-27 (lines 177–179) | **reword** (S2b-ii) | "holds seven entries" → "holds six entries"; delete the `applied` entry (`ezsdr.radio.mock.applied`, line 179, "always empty") from the list |
| MR-17 (line 145), MR-18 (line 150), MR-25 (line 172), MR-33 (line 200), MR-40 (line 219) | check each for a queue or release-window sentence when editing (S2b-i); not matched by the grep pattern | |
| MR-3 (lines 52–74) | **reword**: the capability list (S2b-i) | |
| Changes table | **add** a row, 2026-11-05 (S2b-ii) | "note 48 S2: the Mock's release window and queue depth leave; the `applied` section is removed (six sections)" |

### 3.3 `design/18-uhd-radio.md` (S2a-i for UR-2 and UR-33; S2b-i for UR-9, UR-25, line 63; S2b-ii for the rest)

| Rule / line | Change | Sentence | Step |
|---|---|---|---|
| UR-2 trait (lines 86–90) | **reword** the signatures: `fn apply(&self, dir: Dir, chan: usize, s: &Settings) -> …` (no `at`); `fn rx_stop(&self, stream: usize) -> …`; delete the `rx_start_now` line if present | | S2a-i |
| UR-9 (line 184, the timing values table; line 190 `min_timed_command_lead_ns` keep) | **reword**: delete the queue-depth and release-window capability rows and their `Test:` mention (line 196) | | S2b-i |
| line 63 (§3 timing paragraph, INFERRED values) | **reword**: delete the sentence "The **release window** is the profile's `radio.timing.release_window_ns` (UR-9, UR-25), counted after a receive cut by the next start's readiness, 3 ms on `x3xx` (INFERRED)." The device lead, the allowance and the start leads stay. (Corrected in rev 2: this is §3 text, not UR-3.) | | S2b-i |
| UR-17 (lines 223–229) | check line 224 for a release term; line 229's "the first lattice instant at or after now plus UR-9's `radio.timing.rx_start_lead_ns`" stays | | S2b-i |
| UR-22 (line 250, `Test:` line) | keep (T3 flush names stay; G2). The `ur_22_t3_flush_ack_is_a_timed_dsp_barrier` name is removed from the `Test:` line; the split test is named there | | S2a-ii |
| UR-24 (line 267, withdrawal stub) | keep: the list "the held list, release window, queue depth and floor, the DDC debt and drain" is history | | — |
| UR-25 (line 271) | **reword**: delete "the stream's release window," from "…the cut plus the stream's release window, one receive call at the…" | | S2b-i |
| UR-33 DSP paragraph (line 334, "**DSP** — each front end has a DDC or DUC FIFO of timed frequency commands, which the radio's queue only paces …") | **reword**: delete the timed DDC/DUC frequency FIFO description and its capacity sentence (line 335, "A DDC holds 32 commands …", and the DUC model). **Keep** the receive-command FIFO (starts; STOP bypasses and clears it) and the transmit data path (the DUC consumes each packet; EOB and SOB headers). (Rev 1 wrongly described the receive FIFO as the DDC and DUC; corrected here.) | | S2a-i |
| UR-33 transmit bullet (line 336, "**transmit** — a transmit queue in order …") | **reword**: delete "timed DSP values take effect at the samples that pass it". The in-order packet queue and the DUC's packet handling stay. | | S2a-i |
| UR-33 receive-command FIFO sentence (in the receive bullet; not read in this revision) | **keep** as written; S2a-i checks that it names no DDC entry or timed frequency | | S2a-i |
| UR-33 configuration bullet (line 333) | **reword**: delete "a timed `apply` joins an in-order command queue …" and the queue-refusal clause; keep `FakeConfig::apply_delay` (G1) | | S2a-i |
| UR-33 Test line (lines 343–346) | **reword**: remove the deleted `ur_33_*` test names (§1.2) | | S2a-i |
| UR-34 (lines 352–358) | **reword**: the `measurement` group names at line 356–357 (`measurement_plan_from_env_uses_defaults_and_rejects_invalid_counts`, `measurement_defaults_restoration_stops_at_the_first_apply_error`) are kept or retargeted per §2.1 (P1074, P899) | | S2a-i / S2b-i |
| UR-34, line 358 ("A raw queue trial reads the device's time once, bracketed by two host timestamps …") | **delete** the raw queue trial sentence | | S2a-i |
| UR-36 (`owner_step`, `design/18` §UR-36 text) | **reword** if the text names the `Nothing` outcome as reachable (read in S2b-ii) | | S2b-ii |
| Changes table (lines 682–721) | **add** a row, 2026-11-05 | "note 48 S2: `apply` and `rx_stop` lose `at`; `rx_start_now` leaves the trait; the DSP FIFO and the release window leave the Module" | S2a-i and S2b-i |

### 3.4 `design/05-module-api.md:236` (UC-6) and `design/06-kernel-coordinator.md:335` (KC-27)

- **UC-6** `hardware_timed` (`design/05-module-api.md:236`): **decided: keep UC-6 as a generic Kernel update class.** It describes a Kernel-level update class; no radio key has one (`design/07` row 2026-11-04, line 474: "no radio key has an update class", VERIFIED), and the Kernel text does not name a radio concept. Remove only the sentence "A pending `hardware_timed` update occupies one slot of the device's command queue until it applies, and a full queue refuses it with the Vocabulary's queue event" (VERIFIED in the line). The rest of UC-6 stays. Changes row in `design/05`, S2b-ii. The rule is then checked by `rules.py`.
- **KC-27** (`design/06-kernel-coordinator.md:335`): **no change.** It has no queue or release term (VERIFIED: grep count 0). Rev 1's citation is withdrawn.

### 3.5 `design/vision/03-simulation-mock-and-time.md:65, 91` (S2b-ii)

Vision text mentioning the queue depth. The Vision is the design source (AGENTS §2). Edit only the mention, keep the section and its `Normative:` line, and add a revision-history row in the index (`Ez-SDR_v4_ARCHITECTURE_VISION.md`). Not read in full in this revision.

### 3.6 `KNOWN_LIMITATIONS.md` (S2b-i for the timing rows, S2b-ii for the rest)

- Table rows 153–156 (`rx_start_lead`, `release_window_ns`, `command_queue_depth`): **delete** the two rows whose subject is removed (`release_window_ns`, `command_queue_depth`); reword the `rx_start_lead` row to state that the 50 ms floor is now the only term.
- KL-16 status line (line 167): **reword** to the new status.
- Entries "Gone in note 47" stay for numbering (entry numbers are never reused).
- AGENTS §2: a commit that changes a user-visible limit updates its entry in the same commit; this one does.

### 3.7 `schemas/` (S2b-i)

- `schemas/radio/envelope.v1.json`: regenerated with `EZSDR_UPDATE_SCHEMAS=1` through the radio schema test `rm_20_schema_freeze` (`crates/ezsdr-radio/tests/radio_model.rs:302`; the switch is read at `:304`). The two members leave `properties` and `required`. The kernel's `schema_freeze.rs` uses the same switch for kernel schemas and is not run for this change.
- `schemas/radio/command_queue_full_payload.v1.json`: **delete** the file. Its registration (`lib.rs:843`) is removed with it.
- `schemas/radio/measurement.v1.json`: does not name the removed keys (VERIFIED by grep); no change.
- No version bump, no `SCHEMA_CHANGELOG` entry (AGENTS §6).

### 3.8 Bench instructions and probe registrations (R6, R10)

Current instructions and probes that change (historical records stay):
- `plan/phase8/bench.md:143–147` (the queue-depth bench step): **delete** the step (S2a-i).
- `crates/ezsdr-radio-uhd/tests/hardware.rs` probes (all `#[ignore]`, compile under `hw-build`, not run here):
  - `hw_b8_raw_queue_depth` (line 561): **delete** (S2a-i; its measurement is gone with A2).
  - `hw_b8_raw_rx_timed_stop` (line 427): **delete** (S2a-i; the timed stop is removed with A1′).
  - `hw_b8_raw_timed_tune_phase` (line 592): **keep** (S10: it is the evidence for `phase_behavior_on_retune`, note 48 A1). Its timed step at `hardware.rs:616` is `device.apply(dir, 0, &Settings { freq: Some(f0) }, timed.then_some(tune))`, a timed **RF** retune, and it moves to the timed path that `UhdDevice` keeps (`tune_auto` with `at`, `uhd.rs:559`) (VERIFIED by reading: `tune_dsp` (`uhd.rs:526`) sets the DSP shift, not the RF LO, so S10's wording "on `tune_dsp`" would change what the probe measures). **Deviation, open question 11 (§7):** the recommendation is `tune_auto`, which needs the orchestrator's yes.
  - `hw_b8_raw_timed_retune` (line 816): **delete** (S2a-i; its timed step is `device.apply(..., (lead_us > 0).then_some(at))` at `hardware.rs:839`, the trait's timed `apply`, which A1 removes, and S10 keeps only the phase probe). Open question 10 (§7).
  - `hw_b8_raw_restart_lead` (line 499): **keep**, and re-measure after B3 (S2b-i) before `rx_start_lead_ns` is trusted.
- Other probes (`hw_b8_leads` line 180, `hw_b8_raw_device_lead` 456, `hw_b8_raw_in_flight_window` 653, `hw_b8_stop_end` 745, `hw_b8_preemption` 795, the D9 probes 1001–1300): check each body for `apply(..., Some(..))` or `rx_stop(..., Some(..))` in S2a-i; a hit is changed or deleted with the same rule.
- Bench run lists (`bench.md:332–333`, `:488–489`) name `hw_b8_burst_at_a_sent_burst_s_end`, `hw_b8_preemption` and others; none of the four deleted probes is in the printed lines. Check with `grep -n "hw_b8_raw_queue_depth\|hw_b8_raw_rx_timed_stop\|hw_b8_raw_timed" plan/phase8/bench.md` before S2a-i; any hit is removed.

### 3.9 Lines added by S6 (each in the sub-step that changes its contract)

| Location | What it says | Change | Step |
|---|---|---|---|
| `design/18-uhd-radio.md:249` | UR-22 sentences on timed TX DSP and the DUC FIFO | reword: delete the timed DSP sentence; the in-order packet and its EOB/SOB headers stay | S2a-ii (with the `ur_22_t3_flush_ack_is_a_timed_dsp_barrier` rename) |
| `design/18-uhd-radio.md:30` | content not read in rev 3 (line cited by S6) | read in S2a-i; reword the timed DSP or DSP FIFO clause if present | S2a-i |
| `design/18-uhd-radio.md:193` | UR-9 timing values (the queue-depth and release-window rows; §3.3) | reword | S2b-i |
| `design/18-uhd-radio.md:362–363` | UR-34 raw queue trial sentences (§3.3) | delete the raw queue trial sentence | S2a-i |
| `design/18-uhd-radio.md:368` | content not read in rev 3 (line cited by S6) | read in S2a-i; reword if it names the queue or the DSP FIFO | S2a-i |
| `design/18-uhd-radio.md:451, 502, 545` | §6 implementation-table rows (content not read in rev 3) | read in the step that removes the mechanism the row names; reword to its status | the step that removes the mechanism |
| `design/07-radio-model.md:378, 384` | counts and member lists of the timing capabilities (content not read in rev 3) | count and member list | S2b-i |
| `KNOWN_LIMITATIONS.md:67` | an entry line (content not read in rev 3) | update in the same commit (AGENTS §2) | the step that changes the limit |

Lines cited by S6 and not read in rev 3 are read in their sub-step before the edit (this table is a summary from grep of `c58cb32`). A body or line that differs from this table is reported in the step record.

## 4. Tests that stay as they are (G1, G2, G5 especially)

- **G1 (untimed settings):** `ur_33_the_antenna_is_read_back_after_an_untimed_change` (`device.rs:2675`); `ur_33_an_untimed_write_lands_at_once…` (first assertion, `device.rs:3268`); the `freq_at` history checks; `mr_*` coercion tests in `ezsdr-mock-radio/tests/mock_radio.rs`; `ur_12_a_rate_the_device_does_not_apply_is_refused` (`mutation U10`); `ur_12_*` refusal tests; N1, N2, N10, N11 (new).
- **G2 (transmit flush and packet state):** `ur_22_t3_stop_interrupts_an_open_flush_wait` (`core.rs:1884`), `ur_22_t3_flush_attribution_is_per_channel_and_code_bounded` (`core.rs:1774`), `ur_22_a_flush_is_four_zeros_and_draws_no_underflow` (`core.rs:1755`), `ur_22_t3_loss_during_flush_records_its_reports` (`core.rs:1830`), `ur_22_t3_ack_read_returned_after_deadline_is_not_a_flush_success` (`core.rs:1851`), `ur_22_t3_flush_reports_after_the_deadline_stay_the_runs` (`core.rs:1796`), `ur_22_t3_flush_ack_is_not_taken_by_shutdown` (the renamed reader part of `core.rs:1570`, §1.6); the packet-state tests `ur_33_an_untimed_sob_clears_duc_time_until_a_fresh_burst` (3301) and `ur_33_an_untimed_two_zero_eob_clears_the_stale_duc_header` (3326); the hardware tests `hw_d9_p5_a_stale_timed_header_after_a_reopen` (`tests/hardware.rs:1231`) and `hw_d70_one_sample_burst_then_timed_burst` (`:1300`), which are `#[ignore]`d. N4 is new.
- **G5 (loss, Stop, Provider::stop):** `ur_26_stop_cancels_the_held_burst_record_only` (`tx.rs:3490`); `ur_26_shutdown_after_loss_cancels_held_bursts_record_only` (`tx.rs:4231`, renamed reason only); `ur_26_stop_actions` (`tests/fake.rs:2458`); `ur_25_a_refused_start_of_a_later_stream_halts_that_stream_alone` (`tests/fake.rs:5831`); `ur_25_a_refused_start_is_fixed_by_a_stop_a_new_value_and_a_start` (`tests/fake.rs:7966`); `ur_24_a_loss_during_rx_receive_finishes_without_recovery_calls` (`rx.rs:1024`); `ur_24_nonloss_rx_stop_failure_leaves_the_chain_uncertain` (`rx.rs:991`, its `Some` argument removed, its assertion stays); `ur_29_a_begin_rechecks_loss_after_acquiring_the_chain_before_rx_start` (`rx.rs:1410`); `timeline.rs` `mr_25_*` stop tests; N6 (new).
- **G3 (receive restart):** `ur_17_a_missed_start_restarts_with_a_gap` (in the `ur_17_*` list, `design/18` line 230); `ur_17_a_missed_start_without_another_report_is_not_an_overflow`; `rm_25_lead_left_reports_a_configure_that_left_less_than_the_declared_lead` (`timeline.rs:719`); the configure-complete warning and `START_LEAD_SHORT` tests named in `design/07` RM-25; `ur_24_a_missed_start_after_a_loss_makes_no_restart_call` (`rx.rs:2405`).

## 5. Sequences G1–G7 and own failure scenarios

**G1** (untimed setting on a stopped receive stream, then `start_rx` with staged values). Stays: §4 G1, N1, N2, N10, N11. Expected: coercion, read-back, refusal and `apply_delay` as today. Frequency history unchanged (`freq_at`, `device.rs:1129`). Failure caught by N10: a later write rewrites produced samples; by N11: the front end does not follow the spec.

**G2** (open transmit, flush, timed burst, close). Stays: §4 G2, the reader exclusion over both paths (N-09, N-10), and the `flush < ack` order in the reader test (§1.6). N4 is dropped (S11): the timed send after the ACK is not pinned by a new test (open question 9). Expected: the four-zero T3 flush and its S1 attribution unchanged; the packet-state tests unchanged.

**G3** (receive restart origin). Stays and moves: N5 (origin agreement, differential VH), N9 (exact origin, shared timeline), the retargeted `rm_25_*` test (exact remaining terms), the retargeted `ur_24_a_cut_receive…`. Expected: ready term is old-rate call + allowance + lead, rounded to the grid; the configure-complete warning, missed start and `START_LEAD_SHORT` unchanged. Failure caught: the release term survives in one Provider only (N5); a shared error in both Providers (N9).

**G4** (failed transmit reopen). N3 (new, S2a-ii): no held or new payload is sent; clock `None`; the cancellation reports as today under its new name (after S2b-ii the report carries `ClockEnded` or `FirstSendShort`; before it, `ColdChange`). Replaces the `Halted` assertions at `tx.rs:1320` and `:1546`. Killing rows: N-04, N-11.

**G5** (loss, Stop, Provider::stop, receiving and transmitting). §4 G5 and N6: order and outcomes unchanged after `rx_stop`'s `at`, the gates and `Line::halted` are gone. Halt rule: public plan, `streaming()` (admission) and refusal exclusion through the plan (R2). N6 is written first.

**G6** (a profile or Binding document that still carries `command_queue_depth` or `release_window_ns`). Answer per boundary (§0.9): **refused** at each boundary. New tests, all S2b-i:
- **N7a** `a_timing_document_with_a_removed_key_is_refused`: a `TimingEnvelope` JSON with either key fails to deserialise, and the envelope schema rejects it. Killed by N-06.
- **N7b: dropped (S5).** No Binding selector carries a removed key (§0.9), so there is no selector refusal to pin beyond the generic refusal that exists already.
- **N7c** `a_measurement_comparison_on_a_removed_key_is_refused`: the comparison refuses the key as "unknown Radio Model capability key" (`measurement.rs:405–406`); the four fixtures no longer carry it (§1.8).
- The existing `schema_freeze`-style test `rm_20_schema_freeze` stays green after `EZSDR_UPDATE_SCHEMAS=1` regenerates the envelope schema.
- No migration, no version bump, no changelog (AGENTS §6). No shipped profile file carries either key (grep of `*.json`, `*.toml`, `*.md`, excluding `v3/` and `target/`; other extensions not checked).

**G7** (`owner_step` branches, R7). Trace (VERIFIED by reading, §0.3–0.4):
- `Open`: **reachable.** `core.rs:1331–1335` (open failure) → `record_open(None)` at `core.rs:644` → `DeviceSet::Unknown` (`core.rs:635`) → a later segment's `owner_step` falls through to `_ => Owner::Open` (`core.rs:624`), after a Stop and a new start (`fake.rs:7966`, by name). The arm stays.
- `Nothing`: **no production path found.** The only production caller of `owner_configure` is `rx.rs:354`. `Nothing` returns `Ok(true)` (`core.rs:1325–1327`), and `from` moves past the segment (`rx.rs:362`), so the same segment is seen again only through a new `Rx` with `from` reset (`rx.rs:145`, `i64::MIN` without a first stream), which the Run does not create (`provider/mod.rs:646`, one spawn per stream per Run; INFERRED). The unit test `rx.rs:2629` repeats the call without a production caller. **Decision (R7): delete the arm** in S2b-ii (§1.12, P368, N-03). N8 is the check before deletion.
- `Open(other set)`: reachable only if the channel set changes within a Run; RM-21 refuses it (VERIFIED in the spec, not traced in code). The `DeviceSet::Open(other)` branch is the `_` arm and is not deleted.

**N8** `a_begun_segment_is_not_configured_twice` (S2b-ii, before deletion): over the differential harness's seeded production-path runs (`rm_26_the_timeline_against_uhd_rx`), count `probe("configured")` per `by` (`core.rs:1339`) on each receive stream. Any run with a second `configured` for the same `by` in one stream's lifetime means `Nothing` is reachable, and the arm stays (P368 and N-03 kept). No run means the deletion goes ahead. A synthetic second `owner_configure` does not decide reachability (R7).

**Own failure scenarios (beyond G1–G7):**
- S-a: a timed `apply` survives in one Provider (a missed `at` removal): the signature change breaks the build, so every implementer must compile `--features uhd --tests` (`hw-build`), not only the default build.
- S-b: `rx_stop` loses `at` in the trait, but a UHD wrapper keeps it: caught by the trait-object test `ur_02_device_is_object_safe_and_shared` and `cargo check --features uhd --tests`.
- S-c: the measurement group is deleted in one place and kept in another (`run` at `measurement.rs:1968` and `:2090`): the key list at `:2090` loses the key in the same commit (S2a-i).
- S-d: `COMMAND_QUEUE_FULL` removed from the `radio.rs` tables but the payload kept: `radio_model.rs:173, 380` fail; the test is deleted with it (S2b-i).
- S-e: a `Stream` literal keeps `release_window: 3 * MS` after the field is removed: a compile error, caught by `cargo test` in each sub-step.
- S-f (new): the measurement fixtures keep the two removed keys after the envelope refusal lands: `valid.json` stops being valid, and the fixture tests fail. Caught by the fixture tests in S2b-i; the edit is in the same commit.

## 6. Sub-step plan and sizes (R10, S5, S10, S12)

The four-way split of rev 1 is accepted with the conditions the ruling sets. Each sub-step leaves the tree green on its own.

**Round 3 split (T1, §0.13).** S2a-i is two gated steps: **S2a-i-α** (the queue measurement group, its plan fields, env parsing, probe, bench step, rows and UR-34 sentences; no new tests) lands first, and **S2a-i-β** (the interface change, FIFO, D9, N1/N2/N10/N11, N-13, A1′, A5, the two probes of §3.8 and their spec text) follows. The list below is the rev 3 S2a-i scope; the α part is marked α, the rest is β.

**S2a-i** (A2 queue measurement group, A1, A1′, A5 `rx_start_now`, the interface change; rev 3 scope):
- Conditions (R10): the queue measurement removal comes with the timed `apply` removal; every interface-dependent test adaptation (the `apply` and `rx_stop` signatures, the ~25 and ~24 implementations, the `tune` closure in `fake.rs:7656`, the `d9.rs` helpers) is in this sub-step; UR-2 and UR-33 spec text changes here (R6, S6).
- Tests added: N1, N2, N10 (through `rx_recv`), N11, and the anchor for N-13.
- Tests deleted: the queue group's tests; the DDC/DUC FIFO tests (§1.2); `ur_33_a_timed_receive_stop_stops_the_stream_at_once` (§1.3); `ur_24_rx_recovery_start_uses_uhd_stream_now` (§1.4); the five missing-map tests (§2.1 S3) where their body is only a timed claim.
- Tests stripped: `fake.rs:1352` and the `dsp_log` message at `fake.rs:6514`; the queue assertions of the `fake.rs` queue block (S12: each test named, §1.2).
- Tests retargeted: `raw_group_limit_stops_every_later_call` (`fake.rs:1361`, §1.2); D9 (`ur_33_d9_…`, §1.2 table; S4 deletion at `fake.rs:7693–7695`); `hw_b8_raw_timed_tune_phase` (§3.8, tune_auto); `ur_33_the_fake_device_keeps_the_devices_queues` (split, body read in S2a-i).
- Order inside the step: (1) delete the queue measurement group and its tests (`measurement.rs:1436–1470`, `:1968`, `:2090`, tests `:4352–4390`, `:5216`; `fake.rs` queue tests named in §1.2); (2) add N1, N2, N10, N11; (3) delete `at` from `apply` and `rx_stop`, delete the DSP FIFO and flows (keeping `Dsp.log`, `tx_header`, `tx_header_stale`, S12), delete `rx_start_now`; (4) rewrite the `dsp_log` assertions; (5) the D9 test disposition (§1.2 table); (6) delete the four probes (`hw_b8_raw_queue_depth`, `hw_b8_raw_rx_timed_stop`, `hw_b8_raw_timed_retune`, and the timed step of `hw_b8_raw_timed_tune_phase`, §3.8); (7) re-check N-12 after `device.rs` changes; (8) the mutation rows of §2.1 (queue group, FIFO, S3 decisions) and §2.2 (N-12, N-13).
- Production: about 350 lines changed (mostly deletions) in `device.rs` (timed queue, DSP FIFO, `dsp_pending`, `end_rx_flow`, `due`, `rx_start_now`), `uhd.rs`, `provider/*.rs` (about 30 call sites, one line each).
- Tests: about 1 100–1 300 lines (the `apply` and `rx_stop` wrappers, `device.rs` ur_33 FIFO tests about 800, the queue measurement group about 300, the D9 and probe edits about 100).
- Total about 1 550–1 750 lines. This is above the brief's target of about 1 500; the excess is the interface change itself, which cannot be split further without leaving `apply` with two signatures.

**S2a-ii** (A4, A5 `Channels::without`, A5′):
- Production: about 50 lines (`tx_dsp_gates`, `lock_tx_dsp` and its 4 callers, `Channels::without`, `Recovery::Halted`, `reopen_failed`).
- Tests: about 150 lines: N3; the reader test `ur_22_t3_flush_ack_is_not_taken_by_shutdown` run over `open_tx_device` and `reopen_tx_streamer` (S11; N4 is dropped); the split of `core.rs:1570`; the `core.rs:1377–1390` and `:1599` edits; the `provider/mod.rs:871` and `provider/tx.rs:1344` deletions (§2.1 S3); `tx.rs:1320` and `:1546` retargets; `provider/rx.rs:1707` (`without`, §1.7).
- Mutation rows: N-04, N-09, N-10, N-11 added; P1175–P1178 deleted; P1307, P1310 deleted.
- Total about 200–250 lines.

**S2b-i** (A2 remainder, B3, G6 boundaries, the receive origin):
- Production: about 120 lines (`TimingEnvelope` fields, `keys`, `check_start_lead`, `timeline.rs` release term, `Stream.release_window`, `channels.rs` 359–360, 470–471, `x3xx.rs` 384–385, `mock lib.rs:1462`, `uhd control.rs:164`, `differential.rs:64`, the `COMMAND_QUEUE_FULL` payload and table rows).
- Tests: about 800–900 lines (`Stream` literals at about 20 sites, `rm_25_*` retargets, the `rm_6_*` deletes, the measurement fixtures (four files, two entries each), N5, N7a, N7c, N9 (values of S9), `radio_model.rs:124–125, 173, 380` edits, schema regeneration); `ezsdr-radio/tests/measurement.rs:76–82, 155` retargeted to `radio.tx.repeat_max_samples` (S5); `:501–502` deleted.
- Schemas: `envelope.v1.json` regenerated; `command_queue_full_payload.v1.json` deleted. Specs §3.1, §3.2 timing rows, §3.3 (UR-9, UR-25, line 63), §3.6 timing rows, §3.9 rows for UR-9 (`design/18:193`) and `design/07:378, 384`.
- Mutation rows: J06 retargeted, N-01, N-06 added; P68, P1070–P1074, P1218, P1219, P1220 deleted.
- Re-measure `hw_b8_raw_restart_lead` after B3 (§3.8); the measured value is not trusted before.
- Total about 900–950 lines.

**S2b-ii** (A3, A6, A7, B5, B7, R7's `Nothing` arm, spec text):
- Production: about 110 lines (`Line::halted`, `State.halted`, the `ColdChange` rename across about 12 sites, the `applied` section, the `advance` comment, the `owner_step` `Nothing` arm and its `by` parameter, the `owner_configure` `Ok(true)` branch for `Nothing`).
- Tests: about 350 lines (`tests/timeline.rs` `halted` retargets at :469, :513, :535–540, :677 (§1.9), N6, N8 (optional, S7), the `ur_36` retarget, `rx.rs:2629`, cancellation renames).
- Spec and docs: about 150 lines (§3.1 Changes row, §3.2 MR-27, §3.4 UC-6, §3.5, §3.6 KL-16).
- Mutation rows: N-03 deleted, N-05 added, P90, P460, J65, J70, P368 deleted; P561, P562, P563, P565, P566, P569, P570, P1209 renamed.
- Total about 600–650 lines.

Overall (estimate): about 3 300–3 500 changed lines across the four sub-steps.

## 7. Open questions (each with my recommendation)

1. **The third cancellation constructor (S8, closed).** `tx.rs:765` is cancelled as `ClockEnded`; no `FirstSendShort`. Recorded as applied.
2. **The `Nothing` arm (R7, S7).** Recommendation: delete it in S2b-ii on the caller trace (§5 G7). N8 is optional; a repeat `configured` in a production-path run would keep the arm.
3. **`UhdDevice::tune_dsp` and `tune_auto` with `at` (S10, yes).** Kept on `UhdDevice` only; the trait loses `at`.
4. **D9 helpers' timed `tune` closure** (`tests/common/d9.rs`). With untimed `apply`, the packet-state assertions of D9 (§1.2 table) may depend on how the stale header was set up. Recommendation: S2a-i keeps the packet-state assertions and changes the helpers to untimed tunes; if one fails, it is retargeted to the header state it actually tests, and the timed-tune claim is deleted.
5. **`StreamState.configured` (S10, yes).** Kept; it is read by the transmit reopen round trip (`tx.rs:826`, `:839`).
6. **P863 (S11: delete).** Its shared raw-latch part (`measurement.rs:1082`) goes with it. Recommendation: no further row; S2a-i reports whether any raw-group latch mutation survives in the remote run.
7. **P985 (`device.rs:639`, S3, S11: keep, retargeted).** Kept if its `if` is the packet's time decision after S2a-i reads the body; deleted if it is the FIFO entry.
8. **Kill power of N-09, N-10, N-11, N-12, N-13.** Each is an anchor-verified mutation with a named killer; its kill is unverified until its sub-step's probe (§2.2). A surviving row is a finding, not a deletion. N-10's killer runs the reader test on the reopen path (S11).
9. **CLOSED by T3 (N4 dropped; no test added).** The order `flush < ack < timed send` keeps its first two links in the reader test; the third link is not pinned.
10. **CLOSED by T2 (`hw_b8_raw_timed_retune` deleted in β).** Its claim, a timed receive retune's start, is not a kept B8 claim; the kept phase probe (§3.8) covers the timed RF retune on `tune_auto`.
11. **CLOSED by T2 (`hw_b8_raw_timed_tune_phase` on `tune_auto` with `at`, in β).** The probe's timed step is an RF retune (`hardware.rs:616`); `tune_dsp` sets only the DSP shift. Measured values are not compared with a bench run in S2a (the bench is not run here).

## 8. Claims I could not verify

- No test was run at `c58cb32` for this plan. The green baseline is assumed, not shown.
- The per-test classification of `device.rs` ur_33 is by name and by grep of `at` and DSP terms. Each body is read in S2a-i; a body that uses an untimed call moves to keep.
- `ur_33_a_fresh_device_has_empty_fifos` (3438) and `ur_33_the_fake_device_keeps_the_devices_queues` (2690): bodies not read.
- The count of `apply`/`rx_stop` impls (25 and 24) is from an earlier grep at this commit.
- `ur_24_nonloss_rx_stop_failure_leaves_the_chain_uncertain` and the other `rx.rs` tests that pass `rx_stop` a `Some`: the count is from grep, not each read.
- `timeline.rs:52` `without`: its one test caller is `provider/rx.rs:1707` (found in rev 3). Not checked in `plan/` or `tools/` (grep of `plan/*/tools` found no call).
- No shipped profile document carries the two removed keys: checked only for `*.json`, `*.toml`, `*.md` outside `v3/` and `target/`.
- `ceil_to_grid` (rev 2's name) does not exist at `c58cb32`; the lattice step is `ceil_div(earliest, num) * num` in `timeline.rs` `begin` (VERIFIED by reading). The N9 literal assumes the 200 MS/s ratio is 5/1 (INFERRED).
- `design/18:63` (the release-window sentence) is quoted from the line; the rest of the §3 timing paragraph and its surroundings were not read in full.
- `design/18` UR-36 (`owner_step`) text: not read; the reword in §3.3 is conditional on it.
- `FakeConfig::apply_delay` (`device.rs:1488`): its host-clock sleep is read from the call site only.

## 9. Implementation record: S2a-i-α (rev 3, implemented 2026-11-05)

- **Removed (α):** the raw queue measurement group (`measure_command_queue_depth`, its trial, drain and command-thread helpers, the queue plan fields, the env parsing and validation of `EZSDR_MEASURE_QUEUE_TRIALS` and `EZSDR_MEASURE_QUEUE_MAX_TRIALS`, the `hw_b8_raw_queue_depth` probe), and the queue wording in `plan/phase8/bench.md`, `run-record.sh`, UR-34. `radio.timing.command_queue_depth` stays a comparable key (no group measures it; `unmeasured_values` reports it). It leaves in S2b-i.
- **Kept (α):** `Device::apply`'s `at`; the coercion group, renamed `raw_coercion`, now `measure_raw_group` calling only `measure_coercion`; `RAW_TEST` (was `QUEUE_TEST`).
- **Tests:** added 0 (none planned for α by rev 3 §0.13). Deleted 18 (fake.rs 13, measurement.rs 5). Rewritten or retargeted 5 (fake.rs: `raw_group_sends_nothing_after_its_first_failed_apply` rewritten to fail the first receive gain request; `raw_group_restores_after_a_panic_in_it` retargeted to a panic in the second untimed apply; the deadline test retargeted to `measure_coercion` through a new untimed-apply hook; the rehearsal's expected key list; measurement.rs: `measurement_record_has_exact_unmeasured_keys_and_valid_conditions`'s reason assertion).
- **Deviations from rev 3 (each is a deletion or retarget the plan does not name):** `raw_group_sends_nothing_after_a_failed_clock_read` deleted (the group makes no clock read after α, so the test has nothing to assert); `the_deadline_passing_during_the_receiver_s_stop…` deleted (its claim is covered by the retargeted deadline test). Rows P891, P896, P907 deleted with their scope (`fn queue_trial`, `StopOnDrop`, queue exclusions); P732 retargeted, not named in rev 3; P908 kept with its killer retargeted, not named in rev 3.
- **Mutation rows** (`plan/maintenance/tools/mutations.json`): 52 deleted, 1 retargeted (P732), 0 added. Anchor sweep after the edits: 1724 rows, all anchored once. Probes in a throwaway copy: the 21 re-run or retargeted rows each killed by their named test; baseline passes unmutated (logs in `probes-a1a/`). The later `TimeReadFails` removal and import edits touch no row.
- **Probe hooks** (`probe_device.rs`): `before_timed_apply` → `before_untimed_apply`; `fail_timed_apply`, `fail_rx_stop`, `after_rx_stop` and their branches removed. `timed_applies` and `timed_apply_count` kept (H4 test).
- **Spec** (`design/18-uhd-radio.md` UR-34, Changes row dated 2026-11-05): raw queue trial and receive-condition paragraphs, the two-thread bound and command-thread drain removed; the 13 queue test names removed from Test lines; the latch and `measurement_plan_from_env` wording reworded. UR-34's device-queue facts (lines 22, 196) stay true until S2b-i.
- **Self-review** (one round, `opencode-go/deepseek-v4.1-flash#max`; the free model refused): no major findings. Adjudicated:
  1. bench.md lines 87, 89, 234 still described the removed group: **adopted**, reworded.
  2. `design/18:30` (INFERRED clause "checked on the bench" for the queue depth): **adopted**, the queue clause is moved to a sentence saying it is unmeasured.
  3. Stray `,,` in UR-34's Test line: **adopted**, fixed.
  4. Unused imports: `SAFE_TX_HZ` **adopted** (removed); `RxRecv` **rejected**: `measurement/multichannel.rs` uses it through `use super::*`, and removing it broke the build.
  5. `FakeFault::TimeReadFails` lost its only test: **adopted**, the variant and its arm deleted from `src/device.rs` (no row anchors them; the `TimeReadFailsFor` variant and its test stay).
- **Checks at the end:** `cargo fmt --all --check` passes; `rdev.py -- cargo test --workspace --no-fail-fast`: 1799 passed, 0 failed (schema_freeze, rm_20_schema_freeze green); `--features uhd` unit 373 and API 5 passed; hardware compiles; clippy workspace and `--features uhd` `-D warnings` clean; `rules.py --base HEAD --renames plan/phase8/reviews/48s2a1a-renames.txt`: 0 problems. Python suite not rerun: no Python, server or schema input changed.
- **Pre-existing, not α:** `design/18` UR-34's Test line names `manifest_evidence_extracts_six_classes_and_rejects_boundary_acknowledgements`, absent at HEAD `c58cb32`. Open; to fix in its own step.

## 10. Implementation record: S2a-i-β (rev 4, implemented 2026-11-05)

- **Removed (β):** `Device::apply` and `rx_stop` lose `at`; `rx_start_now` leaves the trait and every impl; the FakeDevice's in-order command queue, its DDC/DUC FIFOs, `settle`, the receive-flow state, `FakeFault::RxFlowStall` and `FakeConfig::command_queue` leave; `Dsp.log` stays, its accessor `dsp_log` (no caller) goes. `start_rx(stream, at)` stays timed. `UhdDevice::tune_dsp` and `tune_auto` keep `at`.
- **Kept (β, per rev 3 rulings):** `Dsp.log`, `tx_header`, `tx_header_stale`; the DUC header state; the untimed write. `Channels::without`, `Recovery::Halted`, the A4 gates and the `core.rs` thread are S2a-ii, untouched.
- **Tests:** 26 names left HEAD: 7 renamed (`ur_33_the_fake_device_keeps_the_devices_queues` → `ur_33_a_timed_sob_met_while_a_burst_plays_does_not_restart_it`, and five more; list in `plan/phase8/reviews/48s2a1b-renames.txt`), 19 dropped. Added: N1 `untimed_apply_sleeps_apply_delay_on_the_host_clock`, N2 `start_rx_staged_values_are_read_back` (renamed from a name that promised a refusal its body does not check), N10 `untimed_write_in_a_loopback_burst_splits_the_received_samples`, N11 `untimed_front_end_value_follows_the_spec`, and `ur_33_an_untimed_continuation_clears_the_duc_timeline` (pins P985). `ur_24_fake_ddc_distinguishes_host_silence_from_a_stalled_ddc` became `ur_24_fake_host_silence_delivers_no_block` (the Silence branch kept, as the plan says).
- **Deviation from the brief (not in rev 3):** `ur_33_an_active_timed_sob_keeps_the_duc_sample_timeline`, `ur_33_an_untimed_sob_clears_duc_time_until_a_fresh_burst` and `ur_33_an_untimed_burst_takes_no_duc_entry` are kept. They assert only the DUC `tx_header`, which β keeps; the earlier deletions were a misreading of their observable. Rows P986 and P1201 depend on them.
- **Mutation rows** (`plan/maintenance/tools/mutations.json`): 17 deleted, 2 added (N-12, N-13), 10 changed (filter or anchor: J327, P214, P215, P219, P226, P270, P824, P825, P985, P987). `plan/phase7/tools/mutations.json`: R12 deleted, R13 anchor moved to the untimed `rx_stop(self.k)`. Sweep: 3061 rows, every anchor matches exactly once (`sweep-a1b.txt`).
- **Probes** (throwaway copy, full lib and fake binaries): each row listed above is killed by its named test or another kept test; P985 is killed by its new test. Rows P986 and P1201 are killed by their kept DUC tests. P1210 was a duplicate of P985 (same edit at the same line) and is deleted.
- **Self-review** (one round, `--agent plan`; the free model refused, so `opencode-go/deepseek-v4.1-flash#max` was used): adopted: N2 rename; `dsp_log` accessor and spec mention removed; the §6 row for the new test rewritten to its actual body; P1210 deleted as duplicate. Rejected: the D9 `tune` closure dropping `at` (plan §3.8 intends it; the narrowing is recorded here); the pre-existing write-only `attempts` in `provider/test_support.rs`.
- **Tree change during review:** while the review ran, `device.rs` and `design/18` changed without an edit of mine: the three DUC tests above came back, `ur_33_an_untimed_continuation_clears_the_duc_timeline` was rewritten to a `tx_header` form with a `tx_header` helper, two comments changed, and the spec's UR-33 Test line and its §6 row were rewritten. I did not make those edits. I kept them because they match the plan and pass the probes. Whoever made them should confirm.
- **Checks at the end:** `cargo fmt --all --check` passes; remote clippy `-D warnings` clean for the workspace and for `--features uhd --all-targets`; remote `cargo test --workspace --no-fail-fast`: 1784 passed, 0 failed; local `--lib` 345 passed, `--test fake` 313 passed; Python 3.14 suite 92 OK; `rules.py --base HEAD --renames plan/phase8/reviews/48s2a1b-renames.txt`: 0 problems.
- **Open question:** the DUC rules (P985, P986, P1201) are pinned only through the internal `tx_header`. Recommendation: keep them; a fake observable for the header would be a later step.

## 10. Implementation record: S2a-i-β (rev 4, implemented 2026-11-05)

Rev 3 (§0–§9) is kept as `test-plan.v3a.md`'s text; this section is the record of what β did against it.

- **Removed (β):** `Device::apply` and `Device::rx_stop` lose `at` (A1, A1′); `Device::rx_start_now` leaves the trait and its implementations and wrappers (A5); the FakeDevice's in-order timed command queue, `settle`, `FakeConfig::command_queue`, the DDC and DUC FIFOs, the flows (`open_flow`, `close_flow`, `end_rx_flow`, `dsp_pending`) and the timed receive stop; the probes `hw_b8_raw_rx_timed_stop` and `hw_b8_raw_timed_retune`, and the timed-command helpers of `probe_device.rs` (`record_timed_apply`, `timed_apply_count`).
- **Kept:** `Dsp.log`, `tx_header`, `tx_header_stale` (S12); `UhdDevice::tune_dsp` and `tune_auto` keep `at` (S10, T2). `hw_b8_raw_timed_tune_phase` and the timed row of `hw_n47_raw_control_call_durations` measure `tune_auto` with its command time (an adaptation the removed timed `apply` forces; see deviations). `Channels::without`, the A4 gates, `Recovery::Halted`, the `tx.rs` reopen work and the ur_22 timed-DSP-wait tests are S2a-ii and untouched.
- **Tests (rev 3 §1.1–§1.3):** deleted 19, renamed or retargeted 8, added 2 (`untimed_apply_sleeps_apply_delay_on_the_host_clock`, N1; `start_rx_staged_values_are_read_back`, N2). The 8 renames and the deletions are listed in `plan/phase8/reviews/48s2a1b-renames.txt` and pass `rules.py --renames`. Helper `tx_header` added (the DUC header read by the DUC tests).
- **Deviations from rev 3** (each is a retained, added or deleted item the plan does not name, or a planned item changed):
  1. `ur_33_an_active_timed_sob_keeps_the_duc_sample_timeline`, `ur_33_an_untimed_burst_takes_no_duc_entry` and `ur_33_an_untimed_sob_clears_duc_time_until_a_fresh_burst` are kept, not deleted: their assertions now read the header (`tx_header`), not the FIFO, and they kill P1201, P986 and P985 respectively (the last also kills P985's mutant, probed).
  2. `ur_33_an_untimed_continuation_clears_the_duc_timeline` is the rename of `ur_33_a_timed_duc_value_is_taken_only_while_has_time_holds` (its FIFO claim removed; the header claim kept); it kills P985.
  3. N2 asserts coercion, read-back and the staged start, not the refusal rev 3 named: the fake does not refuse a rate at `start_rx`. The refusal is pinned at the Provider by the existing `ur_12_a_layout_a_rate_the_device_does_not_apply_is_refused`.
  4. `ur_24_fake_ddc_distinguishes_host_silence_from_a_stalled_ddc` is reduced to its Silence branch and renamed `ur_24_fake_host_silence_delivers_no_block` (rev 3 ruling); its `design/18` §6 row is rewritten.
  5. `ur_33_a_late_continuation_without_eob_advances_the_duc_before_radio_drops_the_next_packet` (P1203's killer) asserts the DUC header after the dropped continuation; its FIFO assertion went with the FIFO. Without that assertion P1203 survived (the probe was run and the survivor fixed).
  6. P1224 (`d9.rs`, `if case == P4Case::DspOnlyReopen`) was deleted in this run, because its only killer, `ur_24_d9_p4_a_and_c_keep_a_pre_stop_ddc_entry_until_restart_samples`, tested the fake's FIFO. Fix round 1 (below) restores it, retargeted to a test that does not use the FIFO. The mutant's behaviour is still in the hardware probe `hw_d9_p4_ddc_entry_after_an_early_stop`, which no runnable test drives.
  7. P1183 is deleted: its `new` text used the removed `chain.fifo`. Its old text (`chain.tx_header = None;`) is covered by N-12 (`ur_33_an_untimed_two_zero_eob_clears_the_stale_duc_header`).
  8. P1210 is deleted: it is the same mutant as P985 (`if at.is_none() {` → `if false && at.is_none() {`), probed under both filters.
  9. Rows N-14 and N-15 are added after the self-review found that the apply delay and the half-step gain coercion had no row (`untimed_apply_sleeps_apply_delay_on_the_host_clock`, `start_rx_staged_values_are_read_back`).
  10. `hw_n47_raw_control_call_durations`'s timed row now times `tune_auto` with its command time, not the timed `apply` (setter plus three read-backs). Its printed summary still says "setter + read-back"; the owner decides whether the row keeps its name and whether the bench comparison is rebuilt.
- **Conflict:** rev 3 §3.9 gives `design/18:249` (UR-22's T3 flush sentence about the timed TX DSP value) to S2a-ii; the brief lists UR-22 at 249 as β. I left the sentence: it describes the flush's order before the timed TX DSP value, which the A4 gates, S2a-ii's, enforce, so it is not false at β.
- **Mutation rows** (`plan/maintenance/tools/mutations.json`, text edits only): 1354 rows in the maintenance file (was 1368). Deleted: J326, P213, P232, P236, P977–P984, P988, P990, P1183, P1210, P1221, P1224; added: N-12, N-13, N-14, N-15; retargeted: P986, P1201 (new filters), P985 (filter `ur_33_an_untimed_continuation_clears_the_duc_timeline`), P987 (filter `untimed_write_in_a_loopback_burst_splits_the_received_samples`). `phase7`: R12 deleted (its FIFO-based old text is gone; phase7 now 151 rows). Anchor sweep after all edits (`sweep-a1b.txt`): 3063 entries (the maintenance file is listed twice by the glob, 1709 distinct rows), 0 not decidable by anchor; every non-empty filter names an existing test.
- **Probes** (throwaway copy outside the repository, `probes-a1b2/`, summary `probes-a1b2-summary.txt`): the 44 existing rows whose old text lies in a changed function, plus the new and retargeted rows (N-12, N-13, N-14, N-15, P985, P986, P987, P1201, P1203). All killed by their named test; P1203 survived once and was fixed (deviation 5). No compile error counted as a kill.
- **Self-review** (one round; `opencode2 run --standalone -m opencode/deepseek-v4.1-flash-free#max` refused for quota, so `opencode-go/deepseek-v4.1-flash#max` ran). Findings and disposition:
  1. Spec Test line named `start_rx_staged_values_are_read_back_and_refused_as_today`, the code `start_rx_staged_values_are_read_back`: **adopted**, the spec name corrected.
  2. N1 and N2 had no mutation row: **adopted**, N-14 and N-15 added (probed).
  3. P1210 duplicated P985: **adopted**, deleted (deviation 8).
  4. `STREAM_CMD` in `uhd.rs` written and never read after the deleted `ur_24_rx_recovery_start_uses_uhd_stream_now`: **adopted**, removed; the shim takes `_cmd`.
  5. Comments naming the removed P4 FakeDevice check (`d9.rs:207`, `hardware.rs:1043`): **adopted**, reworded. `d9.rs:1` kept: the TX differential `ur_33_d9_raw_probes_match_the_x3xx_tx_model` still exists.
  6. `uhd.rs` `tune_dsp`/`tune_auto` comments said "as `apply` does": **adopted**, now name each other.
  7. A4 threads still named `timed-tx-dsp` while calling the untimed `apply`: **rejected for β**; S2a-ii deletes those threads and their names with them. Renaming them now would edit A4 code outside β.
  8. The `hw_n47` timed row now measures `tune_auto`: **recorded** (deviation 10), not changed.
  9. `design/18` `Applied` text still said "or for a timed change", and the Changes row omitted the trait change: **adopted**, both fixed.
- **Checks at the end:** `cargo fmt --all --check` clean; `rdev.py -- cargo clippy --workspace --all-targets -- -D warnings` and `-p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings` clean; `rdev.py -- cargo test --workspace --no-fail-fast`: 1784 passed, 0 failed; `--features uhd`: 714 passed, 0 failed; Python 3.14.4 `unittest discover`: 92 OK (`EZSDR_SERVER` set to the `ezsdr-server` built from the tree); `rules.py --base HEAD --renames plan/phase8/reviews/48s2a1b-renames.txt`: 0 problems; `chmod go+r` applied to the changed and new files.
- **Fix round 1** (`brief-fix-a1b.md`; Sol's P2 on the code review, adopted). The retained P4 check is restored as its own test, not from the FIFO: `ur_24_d9_p4_c_closes_and_reopens_the_streamer_and_a_does_not` (`tests/fake.rs`, D9a). It runs `p4_dsp_only` for case A and case C with an untimed tune closure (`apply` without `at`), and reads the calls `FakeDevice::calls()` logs from that run: case C must log `rx_close 0` then `rx_open 0 [0]`, case A must log neither. The test is new; the 1785 workspace tests now include it (was 1784).
- **Row P1224 is retargeted, not deleted:** `plan/maintenance/tools/mutations.json` gets P1224 back (text edit, same `old` and `new`), with filter `ur_24_d9_p4_c_closes_and_reopens_the_streamer_and_a_does_not`. The "Deleted" list in the bullet above is as of the continuation run; P1224 is not in it any more. Probe in a throwaway copy (`/tmp/opencode/p1224-copy`, logs in `probes-a1b3/`): the unmutated copy passes the new test (`P1224-baseline.log`); with `if case == P4Case::DspOnlyReopen {` → `if false {` the test fails at its assertion, `left: []`, `right: ["rx_close 0", "rx_open 0 [0]"]` (`P1224.log`). A compile error would not have counted; this one compiled. The anchor sweep after the edit: 1710 rows across 6 files, 0 not decidable (`sweep-a1b3.txt`).
- **Renames:** `plan/phase8/reviews/48s2a1b-renames.txt` is unchanged. The old name stays `-` (dropped); the new test is a new test, not a rename of the old one.
- **Carried to S2a-ii** (owner's note 2, not done here): `design/18:249` (UR-22's T3 sentence on the timed TX DSP value; S2a-ii's A4 gates) and the `hw_n47_raw_control_call_durations` rename (its timed row now times `tune_auto`; deviation 10 above).
- **Stale citation, not changed here:** `KNOWN_LIMITATIONS.md:80` cites the deleted test `ur_24_d9_p4_a_and_c_keep_a_pre_stop_ddc_entry_until_restart_samples` as evidence for the FakeDevice P4 A/C. The new test checks only the close and reopen calls, so that evidence line no longer holds for the FIFO claim. Recommendation: the orchestrator edits that entry in the gate-closing commit, and cites the new test there.
- **Checks after the fix:** `cargo fmt --all --check` clean; remote `cargo clippy --workspace --all-targets -- -D warnings` and `-p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings` clean (`clippy-ws-a1b3.log`, `clippy-uhd-a1b3.log`); remote `cargo test --workspace --no-fail-fast`: 1785 passed, 0 failed (`ws-tests-a1b3.log`); Python 3.14.4 `unittest discover`: 92 OK, with `ezsdr-server` built from this tree (`python-suite-a1b3.log`); `chmod go+r` applied; `rules.py --base HEAD --renames plan/phase8/reviews/48s2a1b-renames.txt`: 0 problems.
- **Fix round 2** (`brief-fix-a1b2.md`; P1187 survived the orchestrator's gate, `gate-a1b/mutation.log`). **Cause:** the killer `ur_33_a_one_sample_eob_loses_the_next_timed_header` (`device.rs:2454`) had one assertion on the lost header's effect, `dsp_pending(Dir::Tx, 0) == 1` (HEAD `device.rs:2825`), which read the removed FIFO. β dropped it, and the kept assertions do not separate the two models: `start` and `transmitted(start, 1)` are equal under the header-lost model and under P1187's mutant (the packet starts at `first+200` either way), so the test passed with the header lost. The difference is the DUC header: with the model, the one-sample EOB's header stays `first+200` (1000200 in the probe); with P1187 the next timed packet takes a DUC entry and the header becomes `first+20200` (1020200). **Fix:** one assertion, `tx_header(&device) == Some(first + 200)`, after the existing `transmitted` checks (`device.rs:2475`); the test's name and body are otherwise unchanged. No row changed: P1187 is the only row naming this test, and no row's `old` lies in its body (checked by line). **Probes** (throwaway copy `/tmp/opencode/p1187-copy`, logs in `probes-a1b4/`): the fixed test passes unmutated (`P1187-fixed-unmutated.log`); with `self.eob_lost = true` → `false` it fails at the new assertion, `left: Some(1020200)`, `right: Some(1000200)` (`P1187-fixed-mutant.log`); before the fix the mutant passed (`P1187-mutant-old-test.log`). The unmutated copy with the old test passed too (`P1187-baseline.log`). A probe-only `eprintln!` in the copy printed the header under both builds (1020200 with the mutant, 1000200 without); its output went to the terminal, not a log. Those probe edits were in the copy only.
- **Carried to S2a-ii, not changed here:** `design/18:249` and the `hw_n47_raw_control_call_durations` rename (owner's note 2). Also noticed, not changed (outside the fix; for S2a-ii's review): the doc comment above `fn tx_header` in `device.rs` (~line 2819) is the one for `send` (`/// A timed transmit send of n samples at at ...`), so `tx_header` has no doc comment of its own and `send` loses its comment placement; a one-line move would fix it.
- **Checks after fix round 2:** `cargo fmt --all` run (no other change); remote `cargo clippy --workspace --all-targets -- -D warnings` and `-p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings` clean (`clippy-ws-a1b4.log`, `clippy-uhd-a1b4.log`); remote `cargo test --workspace --no-fail-fast`: 1785 passed, 0 failed (`ws-tests-a1b4.log`); Python 3.14 `unittest discover -s python/tests` with `EZSDR_SERVER` = the `ezsdr-server` built from this tree: 92 OK (`python-suite-a1b4.log`); `chmod go+r` applied; `rules.py --base HEAD --renames plan/phase8/reviews/48s2a1b-renames.txt`: 0 problems (`plain --base HEAD` lists the 23 renamed or dropped tests, as in β, and needs `--renames`).

## 11. Implementation record: S2a-ii (rev 5, implemented 2026-11-05)

Scope: rev 3 §6 S2a-ii (A4, A5, A5′, the two ur_22 timed-DSP-wait tests, the rows of §2.1 and §2.2 for those, and the two spec items carried from β, §10). S2b was not started. The base is `aed8111` (β). Rev 4 (§0–§10) is kept as `test-plan.v4.md`; the earlier rev 4 snapshot (before fix rounds 1 and 2) is kept as `test-plan.v4-pre-fix.md`.

- **Removed (A4):** `Core.tx_dsp_gates` (its field, its initialiser, and the `tx_channel_count` local that only sized it), `lock_tx_dsp` (its doc comment, which described `open_device`, moved to `open_device`), and its four callers: `tx_flush` (test), `open_tx_device`, `reopen_tx_streamer` and the `tx_flush` path. The report gate, the generation split, the flush ACK and deadline, and the report stash are unchanged.
- **Removed (A5):** `Channels::without` (`ezsdr-radio/src/timeline.rs`). Its one caller, `provider/rx.rs` `ur_24`-test at the `Line::new` of stream 1 (formerly `Channels::first(2).without(0)`), is retargeted first to `Channels::first(0).with(1)`: the same set `{1}` (both are the bit 2), so the test's claim is unchanged.
- **Removed (A5′):** `Recovery::Halted` (`tx.rs`). `reopen_failed` now sets `Recovery::Clean` and keeps `burst = Closed`. `needs_reopen` never tested `Halted`; the `recovery()` reader is `#[cfg(test)]`.
- **Tests:**
  - Renamed and rewritten: `ur_22_t3_flush_ack_is_a_timed_dsp_barrier` → `ur_22_t3_flush_ack_is_not_taken_by_shutdown` (`provider/core.rs`). It runs over `open_tx_device` and over `reopen_tx_streamer` in one loop (`for reopen in [false, true]`). The timed-DSP thread and its `apply` assertions are gone; the held report gate, `flush < ack`, one `tx_send` and two reads are kept per path. The checkpoint and the assertions are rewritten in fix round 1 (below).
  - Deleted: `ur_22_arm_waits_for_t3_flush_ack_before_timed_tx_dsp` (`provider/mod.rs`) and `ur_22_tx_owner_reopen_waits_for_t3_flush_ack_before_timed_tx_dsp` (`provider/tx.rs`), and the helper `assert_tx_open_flush_barrier` they used. Their claim (a timed DSP write waits for the flush ACK) has no production writer left: `UhdDevice::tune_dsp` and `tune_auto` keep `at` but have no production caller (`uhd.rs:526, 559`, grep). The owner-reopen ACK order (`flush < ack`) is pinned by the renamed test's reopen path.
  - Retargeted: `tx_chain_recovery_transitions` (`tx.rs`, the `Halted` assertion → `Clean`); `ur_22_underflow_close_failure_reopens_before_the_next_payload` (`tx.rs`): the `Halted` assertion → `tx.clock.is_none()`, plus `tx.chain.next().is_none() && tx.chain.tail().is_none()` (the device burst closed), plus one `tx.pass()` after the failure with no send of B on the failed chain. The assertion at `:1545` (`!needs_reopen`) is kept for N-11.
  - Stale message fixed (self-review finding 1): `"Stop/loss must not mark the chain Halted"` → `"Stop/loss must leave the chain Clean"`.
  - Counts: added 0 test functions; deleted 2; renamed 1; retargeted 3 (one in `rx.rs`, two in `tx.rs`, plus the message fix). Workspace tests 1785 → 1783 (the two deletions).
- **Deviation 1 (N3, rev 3 §1.5):** N3 `failed_reopen_sends_no_held_or_new_payload` is not a separate test. Its setup (an underflow with a failed EOB close, then a failed reopen with a held burst B) is the `reopen_fault` branch of `ur_22_underflow_close_failure_reopens_before_the_next_payload`, which already runs `TxOpenFails(1)` and asserts that B is not sent, its cancellation is recorded, and its record is kept. The claims rev 3 gave N3 (clock `None`, the device burst closed, no later send) are now in that branch. A second copy of the same setup would add about 60 lines and no failure path. Self-review finding 2 (adopted as this record entry).
- **Deviation 2 (the DUC FIFO sentence, brief item 2):** `design/18:249` at `aed8111` has no sentence on a DUC FIFO. The clause the step changes is "before any timed TX DSP value is issued to that chain", which is the only timed-DSP sentence on that line. "This flush clears stale DUC header state" is the header (kept).
- **Spec (`design/18-uhd-radio.md`):** UR-22 line 249: the clause above is removed; the rest of the sentence (the per-channel BurstAck read, the 100 ms deadline, the `rejected` record, `tx_flush`) stays. UR-22 Test line: `ur_22_t3_flush_ack_is_a_timed_dsp_barrier`, `ur_22_arm_waits_for_t3_flush_ack_before_timed_tx_dsp` and `ur_22_tx_owner_reopen_waits_for_t3_flush_ack_before_timed_tx_dsp` → `ur_22_t3_flush_ack_is_not_taken_by_shutdown`. Changes row dated 2026-11-05 (the S2a-ii row sits after β's row, which is last).
- **Carried from β, done (owner's note 2):** `hw_n47_raw_control_call_durations` (`crates/ezsdr-radio-uhd/tests/hardware.rs`): the timed row is renamed `rx_tune_auto_timed_with_samples` → `rx_tune_auto_with_command_time`; the printed summary and the raw observation's note say that this row times `UhdDevice::tune_auto` with its command time and that the others are setter plus read-back/enqueue. The probe's name and the raw observation's name are unchanged. The bench paragraph T6 in `plan/phase8/bench.md:549` still describes the old row (see open question 2).
- **Mutation rows** (`plan/maintenance/tools/mutations.json`, text edits only; the file is not re-serialised):
  - Deleted, NOT FOUND (the mechanical sweep, `sweep-a2.txt`, run on HEAD's rows against the edited tree before any deletion): P1175, P1176, P1177, P1178 (the four `let _dsp = self.lock_tx_dsp(...)` lines) and P1307 (`reopen_failed`'s `Recovery::Halted`).
  - Deleted, equivalent: **P1310** (`tx.rs` owner call site, `reopen_failed` → `reopen_succeeded`). Its anchor still matches, and the sweep does not catch it. The test's `Halted` assertion used to kill it; after the change the mutant survives (probe `probes-a2/P1310.log`). It is equivalent on every reachable state: each `mark_reopen_needed` call (`tx.rs:642, 660, 794, 804`) is preceded by a `close_device_burst` (`tx.rs:740` or in `abandon`, `tx.rs:894`), so `reopen_failed`'s `burst = Closed` is a no-op when the reopen runs. (Rev 3 gave the reason "reopen_failed sets Clean"; that is not the reason, the burst closure is.) Open question 4 asks whether to keep the closure.
  - **N-04** (`self.at = self.plan.len();` → `self.at = 0;`, added in this step, then deleted): probe `probes-a2/N-04.log` survived the planned killer. Equivalent: `self.at` is read only by `retarget` (`tx.rs:341`, needs `Some(clock)`) and written by `end_clock` (`tx.rs:1078`, needs a clock); `halt_failed_reopen` sets `clock = None` and nothing assigns a new clock to the Tx, so the cursor is unobservable after a failed reopen. Open question 3.
  - Added: **N-09** (`open_tx_device`: `let _reports = self.lock_tx_reports(k);` → `let _reports = ();`) and **N-10** (`reopen_tx_streamer`: the same line before `close_device`), both killed by `ur_22_t3_flush_ack_is_not_taken_by_shutdown` (see the note on the kill below); **N-11** (`halt_failed_reopen`'s caller: `self.chain.reopen_failed();` removed), killed by `ur_22_underflow_close_failure_reopens_before_the_next_payload` at its `!needs_reopen` assertion (`probes-a2/N-11.log`).
  - Retargeted: none. Rows whose killer changed and that were probed in the throwaway copy, all still killed: P1198, P1205, P1208, P1209 (`probes-a2/`).
  - Rows total: 1707 (`sweep-a2.txt`, all six files: every `old` and `old2` matches once; every non-empty filter names an existing test).
- **Probes** (throwaway copy `/tmp/opencode/a2-copy`, outside the repository, rsync without `.git`, `.cargo`, `target`, `v3`; `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`; `UHD_LIB_DIR=/tmp/opencode/uhdlib`, a symlink to the installed `libuhd.so.4.9.0`, because the host has no `libuhd.so` name; a copy is touched before each build; one copy at a time): baseline passes (`BASE.log`); N-09, N-10, N-11 killed; N-04 and P1310 survived and are deleted as equivalent; P1198, P1205, P1208, P1209 killed. Compile errors were not counted as kills; none occurred. Logs in `probes-a2/`.
- **The N-09/N-10 kill (open question 1):** resolved in fix round 1 (below). At the S2a-ii step, N-09 and N-10 died at the `is_held` precondition, and with that precondition disabled the test passed (`probes-a2/N-09-without-precondition.log`).
- **Self-review** (one round; `opencode2 run --standalone -m opencode/deepseek-v4.1-flash-free#max --agent plan` refused on the free quota, so `opencode-go/deepseek-v4.1-flash#max` ran; output `selfreview-a2.out`). Findings and disposition:
  1. P3, the stale `Halted` message at `tx.rs` (`Stop/loss must not mark the chain Halted`): **adopted**, reworded.
  2. P3, N3 not a separate test: **adopted** as deviation 1 (recorded here).
  3. P3, the equivalence reasons of P1310 and N-04 lived only in the transcript: **adopted**, recorded in this section.
  The reviewer found no P1 or P2. Its check of the `N-11` kill and of the callers (`Channels::without`, `Recovery::Halted`, `lock_tx_dsp`, `tx_dsp_gates`, `assert_tx_open_flush_barrier`) found no other caller.
- **Checks at the end:** `cargo fmt --all --check` clean; remote `cargo clippy --workspace --all-targets -- -D warnings` and `-p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings` clean (`clippy-ws-a2.log`, `clippy-uhd-a2.log`); remote `cargo test --workspace --no-fail-fast`: 1783 passed, 0 failed, exit 0 (`ws-tests-a2.log`; `schema_freeze` and `rm_20_schema_freeze` included and green); remote `cargo test -p ezsdr-radio-uhd --features uhd --no-fail-fast`: lib 356, fake 296, hardware 56 (59 ignored), uhd_api 5, all passed (`uhd-tests-a2.log`); Python 3.14.4 `unittest discover -s python/tests` with `EZSDR_SERVER` set to the `ezsdr-server` built from this tree: 92 OK (`python-suite-a2.log`); `rules.py --base HEAD --renames plan/phase8/reviews/48s2a2-renames.txt`: 0 problems (renames file: `ur_22_t3_flush_ack_is_a_timed_dsp_barrier -> ur_22_t3_flush_ack_is_not_taken_by_shutdown`; `ur_22_arm_waits_for_t3_flush_ack_before_timed_tx_dsp -> -`; `ur_22_tx_owner_reopen_waits_for_t3_flush_ack_before_timed_tx_dsp -> -`).
- **Open questions (each with my recommendation):**
  1. ~~The reader exclusion's kill of N-09/N-10~~ Closed by fix round 1 (below): N-09 and N-10 are killed by the flush-acknowledgement assertion, with `is_held` removed.
  2. `plan/phase8/bench.md:549` (bench T6, Japanese) still says the timed row is setter plus read-back and mentions the device FIFO. Recommendation: the owner rewrites that paragraph with the bench step; it was not changed here.
  3. `TxChain`'s plan cursor `self.at` is dead once a Tx's clock is `None` (N-04). Recommendation: remove the `self.at = self.plan.len();` line and the cursor on the failed-reopen path in a later step, with a test pinning that a failed reopen leaves no clock.
  4. `reopen_failed`'s `burst = Closed` is a no-op on every reachable path (P1310). Recommendation: keep it as the contract (the reopen function states it), since removing it would leave a state the function does not own. Owner decides.
  5. KNOWN_LIMITATIONS.md:80 still cites the deleted test `ur_24_d9_p4_a_and_c_keep_a_pre_stop_ddc_entry_until_restart_samples` (the β record's stale citation). Not in S2a-ii; the orchestrator edits it in the gate-closing commit.
- **Not verified:** the workspace run covers the uhd feature only through `rdev.py` (the remote image); the local probes used the throwaway copy. No mutation run on sim02/sim03 (`remote_mutate.py`) was made; the gate's full mutation run is the orchestrator's. Hardware probes are `#[ignore]`d and were compiled (`--features uhd`), not run.

### Fix round 1 (Sol's code review, `sol-code-a2.log`; brief `brief-fix-a2.md`)

- **P2, reader exclusion** (`ur_22_t3_flush_ack_is_not_taken_by_shutdown`, `provider/core.rs`):
  - Checkpoint: the test waits until the shutdown reader is **queued** on the report clock lock (`ClockMutex::queued()`, new in `host.rs`: the clock's own FIFO of waiters, which holds a thread from its queueing until the holder's hand-off; `waiters()` is transient and stays for `rx.rs`), or until the reader has entered `tx_async`. At release it asserts `(queued, reads) == (1, 1)`.
  - Device time: once the opener is parked at its gate, `spy.set_time(+1_000_000)` makes the flush's acknowledgement due (the fake releases an ACK only at its tick). A reader that got past the gate would take it at once; the host clock, which the flush deadline uses, does not move.
  - `is_held` is removed from the test and deleted from `host.rs` (it had no other caller).
  - Both open paths: the opener's `TxFlushResult` comes back over a channel and is asserted (`acknowledged && !stopped`); shutdown got `None`; exactly one channel-0 `BurstAck` with `tx_flush == true` is in `rec.device_async`; no `tx_flush` rejection row.
  - The first version of the checkpoint (queue-only, no device time) killed N-09 and N-10 only at the checkpoint (`(0, 2)`: the reader had read, but the ACK was not due). The device-time step fixed that.
  - Probes (throwaway copy `/tmp/opencode/a2b-copy`, logs in `probes-a2b/`): N-09 and N-10 are **KILLED** by the flush-acknowledgement assertion ("the opener's flush was not acknowledged", the shutdown reader took the ACK), with `is_held` absent. The rows keep their anchors and filter; no `mutations.json` change.
  - Stability: the threaded test passed 20 of 20 runs on the baseline.
- **P2, failed reopen** (`ur_22_underflow_close_failure_reopens_before_the_next_payload`, reopen-fault branch): a later burst (`target + 1_000`) with its reservation is delivered through `command(TxCmd::Burst(...))`, then one `tx.pass()`. Asserts: the reservation is released; `tx.clock.is_none()`; no send of B or of the later burst; exactly one `UR-25: its transmit clock was never started` rejection row; exactly one `COMMAND_REJECTED` event (drained before the command, so B's event is not counted). No new fixture: `recovery_rig` now also returns its event collector (five call sites).
  - Probes: N-11 KILLED (at `!needs_reopen`, "the failed reopen is no longer pending"); P1198, P1205, P1208, P1209 KILLED at their existing assertions. No row was added for the new block, so no probe shows that block kills a mutant on its own (open question 2 below).
- **P3, bench** (`plan/phase8/bench.md`, T6): the paragraph describes `rx_tune_auto_with_command_time` apart from the setter-plus-read-back rows, as `hardware.rs` prints them; the safety sentences are unchanged.
- **Counts:** no test added, deleted or renamed in this round; `48s2a2-renames.txt` is unchanged. Files changed: `host.rs`, `provider/core.rs`, `provider/tx.rs`, `plan/phase8/bench.md`, this record.
- **Checks:** `cargo fmt --all --check` clean; `rdev.py -- cargo clippy --workspace --all-targets -- -D warnings` and `-p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings` exit 0; `rdev.py -- cargo test --workspace --no-fail-fast`: 1783 passed, 0 failed (`ws-tests-a2b.log`); `rdev.py -- cargo test -p ezsdr-radio-uhd --features uhd --no-fail-fast`: lib 356, fake 296, hardware 56 (59 ignored), uhd_api 5, all passed (`uhd-tests-a2b.log`); Python 3.14.4 in the image, `EZSDR_SERVER` set to the `ezsdr-server` built from this tree: 92 OK (`python-suite-a2b.log`).
- **Open question 2 (new):** the later-burst block of the failed-reopen test has no probe of its own. Recommendation: add a row for "a burst after a failed reopen keeps its reservation" in the S2b step, where the cancel path is next touched, so that the block is shown to kill something.
- No self-review round was run (the brief does not ask for one).
