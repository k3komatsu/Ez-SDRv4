STATUS: done

# Phase 8 1b-5b 実装報告

## 結果と変更範囲

MockRadio は常に layout description を使うようになりました。selector の `keys` は absent と `"channel"` のどちらでも layout を選び、`rx`・`tx` は常に受け付けます。それ以外の `keys` 値は拒否します。current-key の `Mode::Current` / `DeviceDescription` 経路と `is_layout()` 分岐を削除し、残った layout 経路に畳みました。既存の layout 挙動は維持しています。commit・push・stage・stash はしていません。

保護対象の `crates/ezsdr-radio/src`、`crates/ezsdr-radio-uhd/src`、`python/`、`crates/ezsdr-server/src` に変更はありません。

## 削除した実装と行数

| 範囲 | 追加行 | 削除行 | 差引 |
|---|---:|---:|---:|
| MockRadio 実装（`channel.rs`、`lib.rs`、`profile.rs`） | 151 | 497 | −346 |
| Acceptance の support（新規 `src/layout.rs` を含む） | 124 | 62 | +62 |
| 変更した test files | 491 | 448 | +43 |
| **実装とテスト** | **766** | **1,007** | **−241** |
| `design/09-mock-radio.md` と mutation tables | 163 | 203 | −40 |
| **untracked の 62 行 `layout.rs` を含む総計** | **929** | **1,210** | **−281** |

削除した current-key の description literal、`Mode::Current` 型と profile 分岐は `crates/ezsdr-mock-radio/src/profile.rs`、dispatch/selector 分岐は `src/lib.rs`、方向全体の channel model 処理は `src/channel.rs` にあります。Mock が常に layout 経路になったため、`is_layout()` の条件分岐も畳みました。

## テストの扱い

対象 binary のテスト数は変更前後とも 199 件です。93 件のテスト本体を layout key/port に移すか書き換えました。最終 tree で削除されたテストはありません。VH-8 テストは削除のままにせず、layout-key differential として戻しました。テスト名の変更は 1 件です：`mr_18_a_disable_that_arrives_after_a_later_change_follows_it` → `mr_18_a_reentry_after_a_later_change_follows_it`。`renames.txt` と `design/09` の表を更新しています。

以下は変更したテスト名の全件です。`mock_layout.rs` の 2 件は元から layout テストで、移植ではなく更新です。

| ファイル | 移行・再構成したテスト |
|---|---|
| `crates/ezsdr-mock-radio/tests/mock_radio.rs`（44 件＋改名 1 件） | `mr_02_from_binding_refusals`; `mr_02_the_tree_has_the_radio_model_shape`; `mr_03_profile_values_reach_the_capabilities_and_the_envelope_section`; `mr_03_a_cbx_layer_starts_at_its_board_s_default_frequency`; `mr_06_coerce_cases`; `mr_06_coerce_is_pure`; `mr_07_prepare_cases`; `mr_07_prepare_reports_the_coercions_coerce_reported`; `mr_08_effective_holds_exactly_the_ten_configuration_keys`; `mr_11_start_cases`; `mr_05_a_layout_without_receive_channels_opens_no_receive_stream`; `mr_12_jitter_draws_belong_to_blocks_after_a_receive_cut`; `mr_13_ramp_values`; `mr_16_burst_refusals`; `mr_17_a_burst_before_its_clock_s_origin_is_late`; `mr_17_late_policy_outcomes`; `mr_18_hardware_timed_updates`; `mr_25_a_stream_stop_keeps_the_pending_commands`; `mr_18_a_cold_rate_change_starts_a_new_sample_clock`; `mr_09_the_transmit_clock_starts_on_its_lattice`; `mr_18_a_cold_change_starts_its_clock_on_the_lattice`; `mr_18_a_cold_receive_change_before_t0_applies_at_t0`; `mr_18_a_receive_enable_from_zero_waits_the_start_lead`; `mr_18_a_cold_transmit_change_replaces_the_tracker`; `mr_18_a_burst_on_a_later_clock_survives_an_earlier_switch`; `mr_18_a_scheduled_pair_is_checked_when_it_applies`; `mr_19_backpressure_is_an_overrun`; `mr_09_same_tick_work_keeps_insertion_order`; `mr_20_faults_fire_at_their_instants`; `mr_20_a_fault_between_cold_clocks_is_not_applied`; `mr_20a_the_overflow_comes_with_the_block_that_carries_it`; `mr_20_a_lost_device_transmits_nothing_after_the_loss`; `mr_20_a_loss_cancels_what_it_left_pending`; `mr_20_a_stop_received_in_the_losing_step_does_not_cancel_the_loss`; `mr_20_an_action_after_the_losing_step_is_refused`; `mr_21_overrun_shape`; `mr_25_a_stop_cuts_at_its_instant`; `mr_29_start_rx_is_the_one_command`; `mr_25_a_stopped_stream_keeps_its_end`; `mr_25_rx_stop_leaves_transmit_running_and_stop_records_an_unstarted_burst`; `mr_37_the_overflow_travels_the_hot_path`; `mr_18_a_fractional_receive_cut_is_the_first_sample_at_or_after_e`; `mr_18_a_fractional_transmit_cut_is_the_first_sample_at_or_after_e`; `mr_18_a_cold_change_that_arrives_after_a_later_one_follows_it`; `mr_18_a_reentry_after_a_later_change_follows_it`（改名）。 |
| `crates/ezsdr-mock-radio/tests/mock_channel.rs`（23 件） | `mr_32_b_the_transmitted_timelines_are_read_at_the_samples_own_instant`; `mr_32_c_the_transmitted_frequency_is_read_at_the_samples_own_instant`; `mr_34_a_a_timed_tune_reaches_every_transmit_channel`; `mr_25_a_stopped_burst_transmits_every_sample_before_the_stop`; `mr_33_gain_scales_the_samples_from_its_instant`; `mr_33_a_receiver_hears_only_its_own_frequency`; `mr_34_a_timed_tune_replaces_the_random_phase_with_its_constant`; `mr_34_a_random_phase_is_redrawn_by_every_tune`; `mr_34_the_deterministic_profile_keeps_every_phase_zero`; `mr_32_the_transmit_path_delay_shifts_a_loopback`; `mr_36_clipping_at_full_scale_is_counted`; `ch_09_the_receive_output_does_not_depend_on_the_stepping_order`; `mr_32_a_cold_transmit_change_ends_the_old_radiation`; `mr_35_a_cold_receive_change_samples_the_field_on_the_new_clock`; `mr_25_a_stop_at_a_held_burst_s_start_silences_the_transmitter`; `ch_09_a_burst_starting_between_rounds_survives_a_stop_in_either_order`; `mr_32_a_two_channel_waveform_is_channel_interleaved`; `mr_25_a_stop_on_a_sample_instant_does_not_transmit_that_sample`; `mr_35_the_receive_gain_scales_the_noise_with_the_signal`; `mr_32_a_stop_after_a_cold_change_keeps_the_old_clock_s_radiation`; `mr_16_a_burst_at_or_before_the_open_burst_s_next_sample_is_refused`; `mr_15_a_transmit_block_is_emitted_only_after_its_last_sample`; `mr_25_a_stop_transmits_every_held_burst_before_it`. |
| `crates/ezsdr-mock-radio/tests/mock_layout.rs`（既存 layout test 2 件を更新） | `mr_38_a_layout_instance_uses_one_key_set`; `mr_12_d14_the_actions_of_a_layout_instance_name_a_stream`. |
| `crates/ezsdr-mock-radio/tests/timeline.rs`（1 件） | `rm_26_the_timeline_against_mockradio` を layout selector、per-channel keys、stream nodes に移行。1,000 seed の比較では rate、timed update、fault、stop、start を維持。generator の direction-wide channel-count 操作には一対一の layout key がないため differential から除外し、その membership 規則は spec 09 に記載した layout move tests で直接確認します。expected 側は layout の instance-wide cold floor を使います。 |
| `crates/ezsdr-acceptance/tests/reactive.rs`（1 件） | `v58_09_a_reactor_runs_in_a_child_run_of_a_session`. |
| `crates/ezsdr-acceptance/tests/uhd.rs`（3 件） | `uhd_59_a_session_retune_outside_the_rf_envelope_is_rejected`; `uhd_57_the_session_loopback_captures_what_it_transmits`; `uhd_61_02_capture_starts_at_the_requested_sample_index`. |
| `crates/ezsdr-acceptance/tests/v58.rs`（11 件） | `v58_04_mock_events_reach_counters_policy_and_manifest`; `v58_07_manifest_records_every_input_and_output`; `v58_11_short_lead_burst_is_a_time_error`; `v58_11_19_5_msps_is_coerced_to_20`; `v58_11_beyond_the_performance_envelope_is_rejected_at_validate`; `v58_13_session_manifest_has_log_waveform_and_capture`; `v58_16_runtime_retune_outside_the_rf_envelope_is_rejected`; `kc_24_a_burst_after_the_transmit_clock_ends_is_refused_at_admission`; `v58_16_runtime_rate_beyond_the_envelope_is_rejected`; `v58_08_a_session_hears_a_burst_from_its_first_sample_in_either_instance_order`; `v57_a_software_loopback_session_captures_what_it_transmits`. |
| `crates/ezsdr-acceptance/tests/v61.rs`（2 件） | `v61_02_capture_starts_at_the_requested_sample_index`; `v61_04_pps_source_is_armed_first_and_streams_align`. |
| `crates/ezsdr-acceptance/tests/x3xx.rs`（5 件） | `d3_the_two_providers_describe_one_layout_alike`; `d3_the_layout_decides_the_channels_on_both_providers`; `d3_a_layout_without_receive_channels_requests_none_on_both_providers`; `d3_a_cbx_layout_is_not_coherent_on_either_provider`; `d3_the_layout_is_a_node_id_of_neither_provider`. |

`mr_08_effective_holds_exactly_the_ten_configuration_keys` という名前は意図して残しました。spec 07 がこの関数名を引用しており、1b-5c までは同 spec を変更しないためです。assertion は現在、layout configuration key 5 個（c0 の 4 keys と s0 rate key）を検証します。

## Mutation rows

削除した rows（参照していたコードが削除済み）：

| ID | 理由 |
|---|---|
| P10 | `Mode::Current` の random-phase branch を削除。 |
| P12 | current-key X3xx `DeviceDescription` の gain override を削除。 |
| P123 | `(Ideal, true)` の current-key selection branch を削除。 |
| P162 | 2-mode 経路とともに、withdrawn MR-41 用 `is_layout()` guard を削除。 |

filter の当て直し：

| IDs | 新しい target |
|---|---|
| P04–P09 | `ezsdr-radio` / `device` / `rm_26_the_layout_parser_checks_the_shared_selector`. |
| P17, P18 | `ezsdr-radio` / `device` / `rm_26_the_description_is_the_one_profile_both_providers_name`. |
| M31, M32, M33, M46, M55, M63 | `mock_channel` / `mr_34_a_timed_tune_replaces_the_random_phase_with_its_constant`. |
| M57 | `mock_channel` / `mr_36_clipping_at_full_scale_is_counted`. |
| M59 | `mock_channel` / `mr_34_a_random_phase_is_redrawn_by_every_tune`. |
| M64, M66 | `mock_channel` / `mr_33_a_receiver_hears_only_its_own_frequency`. |

新規 mutation row はありません。全体の選択実行は初回 110/116 killed、P04–P09 は共有 selector test で再実行し 6/6 killed。P17/P18 も retarget 後に再実行（2/2）。source text の調整後は P158/P171/M46 が 3/3、binding-refusal assertion の強化後は P16 が 1/1 killed です。**選択した 116 rows は全て killed**。`rules.py --base HEAD --renames .../renames.txt` は 0 problems で、mutation の `old` は全てソースに正確に 1 回一致しました。

## 確認結果

| 確認 | 結果 |
|---|---|
| focused MockRadio binaries | `mock_radio` 60/60、`mock_channel` 31/31、`mock_layout` 49/49、`timeline` 1/1。 |
| 修正後の focused acceptance binaries | `v58` 33/33、`x3xx` 5/5。 |
| `cargo test --workspace --no-fail-fast`（最終に 1 回実行） | 66 test/doc-test reports、1,569 passed、0 failed、0 ignored。 |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass。 |
| `cargo build -p ezsdr-server` | pass。 |
| Python 3.14 suite | 56/56 passed。 |
| `rules.py --base HEAD --renames ...` | 0 problems。 |
| `git diff --check` | pass。 |

## DeepSeek セルフレビューと残る穴

1 回目の DeepSeek review では P1 が 2 件でした。VH-8 differential の削除は layout 対応として復帰し、`kc_24` の target は旧 `radio/tx` から実在する layout stream node `radio/tx/s0` に変更しました。あわせて spec の古い target、retune 数、無効な P162 row を修正・削除しました。

2 回目の verdict は **P1 なし**。次の P2 指摘は意図的な deferred item / limitation として残します。

1. `design/07-radio-model.md:119` は current-key の `radio.rx.coherent: One(false)` / `Eq(true)` をまだ記述しています。layout keys では `radio.rx.coherent` は radio key ではありません。x3xx acceptance test はその拒否と per-channel phase behavior を確認します。spec 07 は依頼範囲上 1b-5c まで変更せず、同段階で current-key の文を削除します。
2. `design/07-radio-model.md:103` は旧名 `mr_08_effective_holds_exactly_the_ten_configuration_keys` を producer test として引用します。既存 citation を維持するため関数名を残しましたが、assertion は layout の 5 keys を検証します。spec 07 の古い citation は 1b-5c で更新が必要です。
3. VH-8 differential は generator の current-key channel-count 操作を除外しています。layout move tests は membership 規則を直接固定しますが、`timeline::plan` との differential では比較しません。必要なら、layout membership operation を出せる generator に拡張する作業が残ります。

2 回目に出たその他の指摘は修正済みです。MR-38 から stage 計画の文言を除去し、Changes 行に MR-14/MR-16/MR-19 を追加、まだ有効な cold-transmit ceiling を復元、mixed-board refusal assertion を具体化し、comments/indentation を修正しました。残る P1 はありません。変更は未 commit・未 stage です。

---

STATUS: done

# 1b-5b 修正の回 1

## 1. VH-8 layout differential（major）

修正箇所：`crates/ezsdr-mock-radio/tests/timeline.rs:47-156,271-313,321-457,583-678`、spec 09 は `design/09-mock-radio.md:334,337`。

- 旧 generator の `Change::Channels(0/1/2)` を捨てず、`radio.<d>.c<n>.stream` の個別変更へ展開しました。実行側は同じ更新を Action として Mock に渡します。
- x3xx/ideal の双方を各方向 2 channel・2 stream にし、期待側は `Lines` と各 stream の `Line` を使って channel set、RM-34 move、各 line の plan を追います。block 比較には stream、clock、sample start/length、channel count、flags、lost を含めました。fault の instant は、fault が cut しない別 stream の block 境界にも反映しています。
- 各 seed の末尾に、direction を seed parity で選んだ membership sequence を生成し、leave、occupied stream への return、stream 間 move、empty stream への join を発生させます。全1,000 seedでの生成数は leave **2,634**、return **1,495**、move **2,148**、empty-stream join **2,161**。各数が 0 より大きいことを test 内で assert します。
- spec 09 の「手書きケースで代替」は削除し、seeded differential が membership transitions を含むことに更新しました。

Differential は連続3回とも pass し、遷移数も同一でした。

| 実行 | 結果 | 生成数（leave / return / move / empty join） |
|---|---|---|
| 1 | 1/1 pass | 2634 / 1495 / 2148 / 2161 |
| 2 | 1/1 pass | 2634 / 1495 / 2148 / 2161 |
| 3 | 1/1 pass | 2634 / 1495 / 2148 / 2161 |

## 2. disable の回帰テスト（minor）

修正箇所：`crates/ezsdr-mock-radio/tests/mock_radio.rs:2220-2281`、spec 09 `design/09-mock-radio.md:332`。

reentry test `mr_18_a_reentry_after_a_later_change_follows_it` は残しました。別に元の名前 `mr_18_a_disable_that_arrives_after_a_later_change_follows_it` を戻し、TX の `radio.tx.s0.sample_rate_hz` を 1,000,500 に予約した後、1,000,001 に届いた `radio.tx.c0.stream = -1` が同じ effective instant 1,000,500 に繰り下がることを検証します。TX clocks は初期 clock がその cut で終わり、新しい clock が自身の origin 1,001,000 で終了します。両 `applied` rows の instant と1件の `LATE_COMMAND` も assert します。旧名の rename は `renames.txt` から除きました。

確認：`timeout 300 cargo test -p ezsdr-mock-radio --test mock_radio` — 61/61 pass。

## 3. P162（minor）

修正箇所：`plan/maintenance/tools/mutations.json:5432-5440`、対象 source `crates/ezsdr-mock-radio/src/lib.rs:2258`。

P162 を復元しました。`old` は現行 source の `Some(spec) => {`、mutant は layout の `sim.channel` を withdraw された MR-41 で拒否する arm を挿入します。target は `mock_layout` の `mr_31_a_layout_instance_joins_simulation_channel` です。

Mutation 結果：P162 **killed**（`maintenance` 1/1 passed）。

## 4. その他の確認

- `cargo clippy --workspace --all-targets -- -D warnings` — pass。
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD --renames /home/komatsu/.cache/ezsdr-fixes/2026-10-21-phase8-1b5b/renames.txt` — 0 problems。全 mutation `old` は source に正確に1回一致。
- VH-8 / disable / reentry の filter を持つ mutation row は mutation lists 全体で追加確認しましたが、該当 row はありません。したがって remote mutation は P162 のみを実行しました。
- 変更した repository files は mode 644。`git diff --check` pass。
- 指示どおりセルフレビューと workspace 全体の test は実行していません。この修正回では runtime source を変更していません（未 commit の先行 1b-5b diff はそのままです）。commit・push・stage はしていません。

## 結果

Mock と timeline の differential に未解決 divergence はありませんでした。3回の differential、`mock_radio`、clippy、rules、P162 mutation はすべて pass。
