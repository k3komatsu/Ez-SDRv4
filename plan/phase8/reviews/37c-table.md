STATUS: done

# 設計メモ 37 の 37c：掃除の一覧

- 対象：`plan/phase8/37-test-thread-timing.md` §3 の「掃除」，指示書 `37c-table-brief.md`，Sol の表のレビュー `sol-37c-table-verdict.md`，実装の指示書 `37c-impl-brief.md` §1。
- HEAD：`010fd26`（37b の commit）。この版は 37c の実装のあとの一覧：判定は指示書 §1 の裁定（Sol の指摘をすべて採る）と，(4) に移した先（`src/provider/control.rs` の unit test か，止めた行）を反映する。§3 の説明は Sol の訂正による。
- 読んだもの：`crates/ezsdr-radio-uhd/tests/fake.rs`（全部），`tests/common/mod.rs`（`after`，`after_ticks`，`captured`，`rehearse_overflow`，`rehearse_session_loopback`，`fake`/`fake_real`），`crates/ezsdr-acceptance/tests/uhd.rs`（`wait_capture`，`uhd_57`，`uhd_61_02`），`tests/README.md`，`plan/maintenance/30-fakedevice-manual-time.md` §1・§4.4〜§5，`plan/phase8/reviews/37a-opus.md`・`37b-opus.md`，`src/provider/control.rs`（`book_cold`，`book_burst`），`src/provider/core.rs`（`now`），`src/authority.rs`（`read`，`reanchor`，`extrapolate`，`uhd-clock`），`src/device.rs`（`base`，`now`，`apply`），`src/host.rs`（`now`，`sleep`），`crates/ezsdr-radio/src/timeline.rs`（`layout_instant`，`decided_at`）。
- 出発点：`37c-sites.tsv` の 164 行と `37c-settle.tsv` の 216 行（関数名でまとめて 129 行。`time_now` は 12 hit を 1 行にした）。表の `path:line` は**関数の宣言行**で，grep が拾った行は `根拠` の範囲に含まれる。grep の外で見つけたものは §4 に足した。
- 判定の付け方：**S** は，試験のスレッドがどれだけ待たされても判定が変わらないもの。そうでないものは，規則 (1)〜(4) のうち，その系列を直すのに使うもの。**固定の瞬間を「余裕をもって先」に置いただけの行は S にしない**（README:34-35，37a の対応表と同じ扱い）。S の理由に「秒の上限」と書いた行は，README:68 が認める「終わることだけを見る上限」である。
- **D** は，README:65-68 が残す実時間の診断（遅れに依らない S とは別枠）。**W** は，未来の時刻を `at=None` にして窓そのものをなくした行（未来の時刻が性質でなかったもの）。**(4)（止めた）** は，この段階で unit test にできず，そのまま残した行（理由は report）．**(4)（部分）** は，行の一部だけを (4) に移したもの。
- 行の読み方：`読み → 次` は，試験のスレッドが何を読んでから次に何をするか。`理由` の「N ms 遅れると…」は，その遅れで落ちる assert。

## 1. 一覧（137 行）

| # | 関数（`path:line`） | 時計 | 読み → 次 | 判定 | 理由 | 根拠 |
|---|---|---|---|---|---|---|
| 1 | `ur_07_the_authority_paces_to_the_device`（fake.rs:253） | fake | `device.host().now()` と `time.now(root)`+30 ms → `schedule(at)`，進み ≥29 ms と `now ≥ at` | S | 「29 ms 以後」と「at に着いた」だけで，遅れは進みを強めるだけ（at を過ぎて予約しても `schedule` は受け付け，`next_wakeup` は at のまま） | fake.rs:253-266 |
| 2 | `ur_07_schedule_wakes_a_waiting_next_wakeup`（fake.rs:269） | fake_real | `now`+1 ms を 7 ms の sleep の後に予約し，起床の遅れの中央値 <4 ms | D | 実時間の期限そのものが性質（README:65-68 が real-clock の診断として残すと明記．wake 自体は `authority.rs` の unit test が止めた時計で固める） | fake.rs:269-292 |
| 3 | `ur_07_now_is_monotonic_across_a_reanchor`（fake.rs:295） | fake_real | 400 ms の実時間ループで `now` を読み単調性を assert | S | 単調性は遅れに依らない（遅れはループを延ばすだけ）；StepBack の診断 | fake.rs:295-314 |
| 4 | `ur_07_schedule_accepts_an_instant_already_passed`（fake.rs:350） | fake | `fired=now` を予約，6 ms 後に `passed=now−1 ms` を予約 | S | 判定は「返る瞬間が passed」で，過ぎた瞬間を受け付けるのが性質（速さは判定でない，R21） | fake.rs:350-365 |
| 5 | `ur_07_cancel_wakes_a_waiting_next_wakeup`（fake.rs:427） | fake_real | `now`+1 s を予約，7 ms 後に cancel，起床の遅れの中央値 <3 ms | D | #2 と同じ（実時間の診断） | fake.rs:427-446 |
| 6 | `ur_07_now_tracks_the_device_while_no_call_runs`（fake.rs:449） | fake | `time.now(root)` と `device.time_now()` を続けて読み差 <20 000 ticks（100 µs） | (2) | 2 つの読みの間に 100 µs 遅れると差がその分になり落ちる（参加者が idle なら時計は実時間で進む）．装置の読みで前後を挟み，範囲で assert する | fake.rs:449-457 |
| 7 | `settle`（fake.rs:797） | fake | `host.sleep(wall)` → events を drain | S | helper（待ちと drain）だけの説明．判定は呼び側の行．静止点ではない | fake.rs:797-803 |
| 8 | `until`（fake.rs:807） | fake | 1 ms ずつ sleep しながら条件を観測（実時間 10 s は見張り） | S | helper（観測の待ち）だけの説明．判定は呼び側の行．条件はすべて単調ではない（行 61 の「clock 数がちょうど 2」が反例）．10 s は「終わること」の見張り | fake.rs:807-815 |
| 9 | `time_now`（12 hit，11 の関数：862，2872-2881，3078，3222，3513，3774，3890，4034，4116，6901，7154） | なし | `Device` の転送のみ（`Watched::time_now` の 60 ms 読みは uhd-clock のため） | S | 試験のスレッドは読まない．読みは Provider の参加者（uhd-clock 等） | fake.rs:862, 2872-2881, 3078, 3222, 3513, 3774, 3890, 4034, 4116, 6901, 7154 |
| 10 | `move_open_refusal_recovery`（fake.rs:886） | fake | `now`+100/50 ms の cold → `until` で拒否と復帰を観測 | S | 判定は「拒否が 1 回」「宛先が開かない」「復帰する」で，時刻は場面だけ（遅れて床に落ちても同じ系列） | fake.rs:886-912 |
| 11 | `move_config_refusal_recovery`（fake.rs:914） | fake | ('`now`+100/50 ms の cold → `until` で設定拒否と復帰を観測', '判定は「拒否 1 件」「宛先が開始しない」「後で復帰する」で，時刻は場面だけ') | S | 判定は「拒否 1 件」「宛先が開始しない」「後で復帰する」で，時刻は場面だけ | fake.rs:914-935 |
| 12 | `ur_16_a_panicking_thread_is_a_lost_device`（fake.rs:1147） | Spec（手動） | horizon 5 s で `run_until_end` | S | 判定は termination と mark_lost の順；horizon は秒 | fake.rs:1147-1157 |
| 13 | `wedged`（fake.rs:1159） | fake_real | `Instant::now()` から `finish` まで <3 s | S | 秒の上限は「終わること」だけを見る（README:68） | fake.rs:1159-1170 |
| 14 | `ur_17_blocks_reach_every_link`（fake.rs:1228） | Spec（手動） | `after(&run, 10 s)` の horizon で 2 つの CAPTURE_WRITTEN を待つ | S | horizon は秒；待つのは event | fake.rs:1228-1247 |
| 15 | `tx_at`（fake.rs:1306） | Spec（手動） | `run.now()` から lead を足した TimePoint | S | helper．判定は呼び側の行 | fake.rs:1306-1310 |
| 16 | `ur_25_the_switch_applies_the_configuration_in_effect_at_e2`（fake.rs:1625） | fake | `now`+320（timed）と +300（cold）→ settle 450 → `apply … freq=2100000000 … at=-` | (4) | 300 ms 遅れると両方が late になり，切替の構成が retune を運ばない．20 ms の関係が性質 | fake.rs:1625-1637 |
| 17 | `ur_25_enabling_tx_applies_the_configuration`（fake.rs:1772） | Spec（手動） | `after(&run, 5 ms)` の capture，3 s horizon | S | 判定は loopback に波形が戻ること；capture が遅れても次のブロックから始まり，繰り返しの中にある | fake.rs:1772-1800 |
| 18 | `ur_27_a_lost_reference_is_clock_lost`（fake.rs:1979） | Spec（手動） | 5 s horizon | S | 判定は CLOCK_LOST の termination | fake.rs:1979-1988 |
| 19 | `ur_28_a_failed_report_read_is_not_an_empty_one`（fake.rs:2006） | Spec（手動） | 200 ms 待ちと 5 s horizon | S | 判定は stats・rejected・DEVICE_LOST | fake.rs:2006-2023 |
| 20 | `ur_29_a_lost_device_aborts_the_run`（fake.rs:2043） | Spec（手動） | 5 s horizon | S | 判定は DEVICE_LOST の termination | fake.rs:2043-2053 |
| 21 | `ur_29_a_silent_stream_is_a_lost_device`（fake.rs:2056） | Spec（手動） | `device.host().now()` と `Instant::now()`，8 s horizon，装置時計 <7 s，実時間 <30 s | S | 判定は DEVICE_LOST の時刻−2.1 s が [1, 2] s（event の時刻）；7 s の上限は「喪失はずっと手前」を見るもので，負荷の class（ms）に対して閉じている | fake.rs:2056-2076 |
| 22 | `ur_29_a_dead_link_that_fails_without_lost_is_a_lost_device`（fake.rs:2079） | Spec（手動） | 6 s horizon | S | 判定は DEVICE_LOST の時刻（event） | fake.rs:2079-2101 |
| 23 | `ur_29_time_reads_that_fail_apart_are_not_a_lost_device`（fake.rs:2104） | Spec（手動） | 5 s horizon | S | 判定は DEVICE_LOST がないことと `time_read_failed` の回数 | fake.rs:2104-2118 |
| 24 | `ur_29_a_failing_reference_sensor_alone_is_not_a_lost_device`（fake.rs:2121） | Spec（手動） | 5 s horizon | S | 判定は DEVICE_LOST がないこと | fake.rs:2121-2131 |
| 25 | `ur_29_a_lost_device_is_found_while_idle`（fake.rs:2134） | Spec（手動） | (None, '判定は DEVICE_LOST 1 件と mark_lost の順') | S | 判定は DEVICE_LOST 1 件と mark_lost の順 | fake.rs:2134-2148 |
| 26 | `ur_33_a_real_fake_s_time_starts_at_its_own_creation`（fake.rs:2380） | fake_real | 生成の前後と読みの前後を実時間で挟み，age ≤ since | S | 挟んだ範囲の assert なので遅れに依らない（R12）；0 は `device.rs` の unit test | fake.rs:2380-2405 |
| 27 | `captures_across_cold_changes`（fake.rs:2537） | Spec（手動） | `after(&run, 10 ms)` の capture → `wait(60)` → レート変更 → 3 s horizon | (1) | capture の提出後に 130 ms 以上遅れると 200 ms の capture が変更の前に終わり「2 つの clock」が崩れる．capture と変更を Spec の固定 offset で予約する | fake.rs:2537-2594 |
| 28 | `ur_29_a_stream_silent_before_a_far_cut_is_a_lost_device`（fake.rs:2643） | fake | `now`+5 s の cold → settle 2 s → DEVICE_LOST 1 件 | S | 変更が遅れて近くなっても，無音の検査は切替の後に続き喪失は見つかる（判定は「喪失が 1 件」） | fake.rs:2643-2658 |
| 29 | `ur_07_the_last_device_handle_is_not_dropped_on_uhd_clock`（fake.rs:2924） | fake_real | 130/200 ms の実時間 sleep の後に drop 順を assert | S | 判定は「どのスレッドが drop するか」で，時刻でない | fake.rs:2924-2942 |
| 30 | `ur_07_no_device_read_follows_the_authority_s_drop`（fake.rs:2945） | fake_real | 50 ms 後に authority を drop，350 ms 後に read 数 0 | S | drop が uhd-clock を止めた後に read はない（sleep は広げるだけ） | fake.rs:2945-2963 |
| 31 | `busy`（fake.rs:2998） | なし | Plan の hold 時間を返す | S | 判定しない | fake.rs:2998-3002 |
| 32 | `hold`（fake.rs:3041） | fake | 参加者の busy sleep と entered の合図 | S | 試験の待ちは合図（handshake）で，時計の読みでない | fake.rs:3041-3067 |
| 33 | `ur_33_a_shutdown_with_a_report_read_that_fails_at_once_still_joins_uhd_tx`（fake.rs:3177） | fake | `Instant::now()` から 10 s 未満 | S | 秒の上限は「終わること」だけ | fake.rs:3177-3194 |
| 34 | `ur_25_a_configure_overlapped_with_a_timed_update_to_the_same_projection_finishes`（fake.rs:3274） | fake | `after(&run, 2 s+800)` へ advance，記録した apply の前後関係 | S | 判定は wrapper が記録した装置の時刻（apply の入口と戻り）；遅れは horizon を延ばすだけ | fake.rs:3274-3304 |
| 35 | `ur_34_the_receive_stall_fires_after_its_device_clock_interval`（fake.rs:3311） | fake | `after_ticks(&run, 3.8 s)` へ advance，overflow−first ≥ 450 ms | S | 「以後」の下限；遅れは強める | fake.rs:3311-3334 |
| 36 | `ur_16_a_panicking_thread_ends_a_two_stream_run_with_its_message`（fake.rs:3339） | Spec（手動） | 5 s horizon | S | 判定は termination と rejected | fake.rs:3339-3357 |
| 37 | `ur_25_an_enable_at_a_timed_update_s_instant_applies_it`（fake.rs:3624） | fake | `at=now`+300 の enable と timed の順序 → settle 500 | (4) | 300 ms 遅れると enable が late になり，順序（構成が timed を運ぶか）が崩れる．窓が性質 | fake.rs:3624-3653 |
| 38 | `ur_05_a_radio_dropped_after_its_device_is_lost_releases_its_serial`（fake.rs:4067） | fake | `host.sleep(20 ms)` の後に serial が err | S | Lost(10 ms) の後なので，遅れは強めるだけ | fake.rs:4067-4078 |
| 39 | `ur_34_a_rate_check_refusal_closes_the_streamer_before_a_later_move`（fake.rs:4216） | fake | `now`+40 の 2 つの cold → `until` | S | 判定は close<open と reconnect なし；遅れて床に落ちても同じ | fake.rs:4216-4239 |
| 40 | `ur_25_the_first_segment_after_arm_does_not_reopen_the_streamer`（fake.rs:4268） | fake | `now`+50 の cold → settle 300 → open 回数 | S | 判定は open が 1 回・次のセグメントが同じ streamer で構成されること | fake.rs:4268-4290 |
| 41 | `ur_24_a_channel_keyed_timed_update_acts_on_its_own_channel`（fake.rs:4540） | fake | `at=now`+40 と +41 → settle 80 → `LATE_COMMAND` 空と `at=` の一致 | (4) | 40 ms 遅れると timed が late になり `LATE_COMMAND` が 1 件になる．1 ms 差の窓が性質 | fake.rs:4540-4569 |
| 42 | `ur_24_a_timed_update_for_a_channel_out_of_its_stream_waits_for_its_segment`（fake.rs:4572） | fake | `at=now`+40 → settle 80/200 | S | 判定は「hand over しない」「後で untimed で 1 回」；遅れても記録は残る（LATE は assert しない） | fake.rs:4572-4599 |
| 43 | `ur_24_a_channel_keyed_command_counts_one_queue_slot`（fake.rs:4602） | fake | `far=now`+5 s に 16 件 → settle 30 → 17 件目で QUEUE_FULL | (4) | 5 s 遅れると far が過去になり 16 件が保持されず，17 件目が拒否されない．台数が性質 | fake.rs:4602-4625 |
| 44 | `ur_25_a_layout_rate_change_cuts_its_stream_and_configures_every_channel`（fake.rs:4706） | fake | `now`+320（timed）と +300（cold）→ settle 500 | (1) | 300 ms 遅れると両方が late になり，切替の構成に 30 MHz が入らない．Spec の固定 offset で予約する | fake.rs:4706-4762 |
| 45 | `ur_25_a_layout_rate_change_of_a_channel_less_stream_only_takes_its_value`（fake.rs:4765） | fake | settle 30 → `now`+100 → settle 200 | S | 判定は装置の呼び出し数と segment がないこと | fake.rs:4765-4789 |
| 46 | `ur_25_a_layout_transmit_rate_change_configures_the_new_segment`（fake.rs:4792） | fake | settle 30 → `now`+100 → settle 300 | S | 判定は新セグメントの apply と 2 つの clock | fake.rs:4792-4824 |
| 47 | `ur_25_a_layout_cold_change_is_floored_at_every_earlier_cold_change_of_the_instance`（fake.rs:4827） | fake | `now`+50 と +10 → settle 200；`applied == now+50`，`e == now+50` | (4) | 50 ms 遅れると最初の変更の e が booking になり assert が落ちる．記録した e 同士の関係で assert する | fake.rs:4827-4865 |
| 48 | `ur_25_a_same_size_membership_swap_reopens_the_streamer`（fake.rs:4868） | fake | settle 30 → `at=now`+300，`now`+10 の burst → settle 700 | (4) | 判定は 2 セグメントの block の値と open の順 | fake.rs:4868-4931 |
| 49 | `ur_25_a_receive_membership_join_opens_the_streamer_with_the_new_set`（fake.rs:4934） | fake | settle 30 → `now`+300 → settle 700 | S | 判定は cut/open/start の順と `origin ≥ end+55 ms`（下限） | fake.rs:4934-4967 |
| 50 | `ur_25_a_receive_membership_leave_opens_the_streamer_for_the_remaining_set`（fake.rs:4970） | fake | settle 30 → `now`+300 → settle 700 | S | 判定は open が `[1]` で `[0]` がないこと | fake.rs:4970-4991 |
| 51 | `ur_34_a_join_after_a_leave_closes_the_source_s_streamer_first`（fake.rs:5019） | fake | `now`+40 の 2 つを `until` で gate | S | 各操作が前の状態を `until` で待つので，遅れは系列をずらすだけ；判定は close<open | fake.rs:5019-5043 |
| 52 | `ur_34_a_transmit_join_after_a_leave_closes_the_source_s_streamer_first`（fake.rs:5046） | fake | ('`now`+40 の 2 つを `until` で gate', '各操作が前の状態を `until` で待ち，判定は tx_close<tx_open') | S | 各操作が前の状態を `until` で待ち，判定は tx_close<tx_open | fake.rs:5046-5071 |
| 53 | `ur_34_an_idle_destination_leaves_its_join_to_the_owner_when_another_streamer_holds_the_channel`（fake.rs:5074） | fake | (None, '各段階を `until` で待ち，判定は close<open と拒否なし') | S | 各段階を `until` で待ち，判定は close<open と拒否なし | fake.rs:5074-5098 |
| 54 | `ur_34_a_membership_change_to_its_own_stream_changes_nothing`（fake.rs:5101） | fake | `now`+40 の timed | S | 判定は clock 数・open 数が変わらないこと | fake.rs:5101-5123 |
| 55 | `ur_34_a_move_cuts_both_streams_and_the_source_reopens_before_the_destination_opens`（fake.rs:5126） | fake | `now`+100 → `until` ×2 | S | 判定は stop/open の順と plan | fake.rs:5126-5161 |
| 56 | `ur_34_a_move_of_the_source_s_only_channel_closes_its_streamer`（fake.rs:5164） | fake | `now`+100 → `until` | S | 判定は stop<close<open | fake.rs:5164-5185 |
| 57 | `ur_34_a_move_into_an_empty_stream_opens_it_after_the_source_s_release`（fake.rs:5188） | fake | `e=now`+100 → `until`；`move_row["e"]==e` | (2) | 100 ms 遅れると move の e が booking になり assert が落ちる．記録した e を基準にする | fake.rs:5188-5215 |
| 58 | `ur_34_a_move_from_a_stopped_stream_closes_its_streamer_for_the_destination`（fake.rs:5218） | fake | `now`+100 → `until` | S | 判定は close<open | fake.rs:5218-5238 |
| 59 | `ur_34_a_timed_update_crossing_the_move_reaches_each_streamer_channel_at_its_instant`（fake.rs:5241） | fake | `e=now`+200，timed を e−30/e+80 → `at=` の一致 | (4) | 200 ms 遅れると timed が late になり at が変わる．窓が性質 | fake.rs:5241-5285 |
| 60 | `ur_34_a_cold_change_before_the_move_s_release_is_floored_at_the_move`（fake.rs:5288） | fake | `e=now`+100，e−50 の cold → late の requested/applied | (4) | 50 ms 遅れると `applied == e` が崩れる．記録した requested/applied の関係で assert する | fake.rs:5288-5313 |
| 61 | `ur_34_two_moves_at_one_instant_swap_their_receive_channels_without_deadlock`（fake.rs:5316） | fake | `e=now`+100 の 2 つ → `until` | (4) | 判定は最終の channels と deadlock なし；両方が同じ e を使う | fake.rs:5316-5338 |
| 62 | `ur_34_two_moves_at_one_instant_swap_their_transmit_channels_without_deadlock`（fake.rs:5341） | fake | ('`e=now`+100 の 2 つ → `until`', '判定は両方向の最終 channels と deadlock なし') | (4) | 判定は両方向の最終 channels と deadlock なし | fake.rs:5341-5364 |
| 63 | `ur_34_a_transmit_move_ends_the_old_burst_and_sends_the_new_burst_on_its_stream`（fake.rs:5367） | fake | `old_target=now`+20，e を clock から，burst は asap，sample を固定 instant で読む | (4)（止めた） | 20 ms 以上遅れると burst が asap で動き，`transmitted(old_sample_at)` が None のまま（`until` の見張りで落ちる）か，動いた TIME_ERROR が `TIME_ERROR 空` を破る．記録した burst の開始を基準にする | fake.rs:5367-5426 |
| 64 | `ur_34_a_transmit_move_carries_a_prebooked_burst_across_the_repeat_cut`（fake.rs:5429） | fake | `now`+100 の burst，e を clock から，sample を固定 instant で読む | (4)（止めた） | 100 ms 以上遅れると同様（`wrapped_at`・`last_old_at` が None，または TIME_ERROR） | fake.rs:5429-5489 |
| 65 | `ur_34_a_finite_middle_segment_releases_a_channel_along_a_move_chain`（fake.rs:5492） | fake | `e1=now`+100，`e2=e1`+150 → `until` | (4) | 判定は close_a<open_b<stop_b<close_b<open_c の順と B の最後の block | fake.rs:5492-5533 |
| 66 | `ur_26_d14_a_layout_instance_names_its_streams`（fake.rs:5538） | fake | settle 30 → `now`+40 の 3 つ → settle 80；`now`+30 の burst ×2，Stop | (4)（部分） | 判定は apply の回数，burst が送られること，rx の clock だけ切れること | fake.rs:5538-5601 |
| 67 | `ur_26_a_device_stop_stops_every_layout_stream`（fake.rs:5604） | fake | `start=now`+100 の burst（保持），Stop，`now > start+20` を `until` | (1) | 100 ms 以上遅れると burst が asap で送られてしまい「装置に届いていない」が落ちる．Spec で予約する | fake.rs:5604-5642 |
| 68 | `ur_26_d14_the_old_addresses_and_foreign_nodes_are_refused`（fake.rs:5645） | fake | settle 30 → `now`+40 の update，settle 10 ×3 | S | 拒否は target で決まり，時刻に依らない | fake.rs:5645-5695 |
| 69 | `ur_26_d14_a_channel_less_stream_keeps_the_mock_s_treatment`（fake.rs:5698） | fake | settle 30/80/120，`now`+30 の burst | S | 判定は「clock なし」「拒否 1 件」「送信なし」 | fake.rs:5698-5732 |
| 70 | `ur_25_a_no_op_membership_change_takes_only_its_value`（fake.rs:5904） | fake | settle 30 → `at=now`+100 の no-op → settle 300；`applied … at == at` | W | 100 ms 遅れると row の at が booking になり落ちる．記録した e を基準にする | fake.rs:5904-5927 |
| 71 | `ur_25_a_layout_cold_change_of_a_linkless_receive_side_is_floored_at_t0`（fake.rs:5930） | fake | `t0=now`+2 s+1 で start → update(None) → settle | (4) | 2 s 遅れると T0 floor の場面が崩れる（late にならない）．止めた時計で固める | fake.rs:5930-5954 |
| 72 | `ur_13_the_reopen_enforces_the_direction_s_rate_comparison`（fake.rs:5993） | fake | `now`+40，settle 200；`now`+60，settle 300 | S | 判定は拒否 2 件・open +1・apply 不変 | fake.rs:5993-6045 |
| 73 | `ur_13_the_receive_owner_s_first_open_is_compared_there`（fake.rs:6048） | fake | (None, '判定は拒否 2 件・open 2・apply 不変') | S | 判定は拒否 2 件・open 2・apply 不変 | fake.rs:6048-6084 |
| 74 | `ur_25_a_transmit_refusal_from_no_stream_starts_no_clock`（fake.rs:6087） | fake | `now`+200 と +60 | (4)（止めた） | 判定は拒否 2 件・tx_switch なし | fake.rs:6087-6122 |
| 75 | `ur_25_a_floor_raised_layout_join_is_judged_at_its_instant`（fake.rs:6138） | fake | `now`+50/+10，settle 200；`later`+60，settle 400；`rows[1]["e"] == later+60` | (4) | 60 ms 遅れると 2 回目の e が booking になり落ちる．記録した e を基準にする | fake.rs:6138-6177 |
| 76 | `ur_24_the_layout_destination_follows_the_effective_membership`（fake.rs:6180） | fake | `at=now`+120，join を `now`+40 | (4) | 40 ms 遅れると join が late になり，tune の「effective membership」が崩れる | fake.rs:6180-6212 |
| 77 | `ur_13_a_runtime_join_compares_the_rate_set`（fake.rs:6215） | fake | `now`+60，settle 400 | S | 判定は拒否 1 件・open 1・rates 1 | fake.rs:6215-6244 |
| 78 | `ur_13_an_enable_from_no_stream_compares_the_rate_set`（fake.rs:6265） | fake | (None, '判定は拒否 1 件・rx_open 1・構成なし') | S | 判定は拒否 1 件・rx_open 1・構成なし | fake.rs:6265-6292 |
| 79 | `ur_25_a_refused_enable_from_no_stream_halts_and_keeps_its_values`（fake.rs:6318） | fake | `join_at=now`+50，`gain_at=now`+50，`now`+50；`changes[0]["e"] == join_at` | (2) | 50 ms 遅れると join_at が booking になり落ちる．記録した e を基準にする | fake.rs:6318-6379 |
| 80 | `ur_25_a_channel_enabled_from_no_stream_in_a_later_stream_opens_and_starts_it`（fake.rs:6452） | fake | settle 30 → `join=now`+40 → settle 300；`origin >= join` | S | 「以後」の下限 | fake.rs:6452-6480 |
| 81 | `ur_25_a_channel_leaving_a_later_stream_cuts_its_stream_alone`（fake.rs:6483） | fake | settle 30 → `now`+40 → settle 300 | S | 判定は cut の数と s0 不変 | fake.rs:6483-6512 |
| 82 | `ur_25_a_rate_change_of_a_later_stream_cuts_that_stream_alone`（fake.rs:6515） | fake | (None, '判定は s1 の cut・s0 の clock 不変') | S | 判定は s1 の cut・s0 の clock 不変 | fake.rs:6515-6538 |
| 83 | `ur_25_a_refused_change_of_a_later_stream_halts_that_stream_alone`（fake.rs:6573） | fake | settle 30 → `now`+40 → settle 300 | S | 判定は拒否・s1 停止・s0 継続 | fake.rs:6573-6605 |
| 84 | `ur_24_a_timed_update_acts_on_the_stream_that_holds_its_channel`（fake.rs:6608） | fake | settle 30 → `now`+40 cold → settle 200 → `at=now`+20 timed → settle 100；`LATE_COMMAND` 空 | W | 20 ms 遅れると timed が late で `LATE_COMMAND` が 1 件 | fake.rs:6608-6636 |
| 85 | `burst_in`（fake.rs:6639） | fake | `direct.now()`+lead から sample k | S | helper．判定は呼び側の行 | fake.rs:6639-6643 |
| 86 | `ur_21_a_burst_targets_its_own_transmit_stream`（fake.rs:6650） | fake | settle 50/80，`burst_in` ms(40)，`now`−20 の late burst | (2) | 判定は「送った stream」と late の source；asap で動いても同じ | fake.rs:6650-6700 |
| 87 | `ur_26_two_receive_streams_end_at_their_own_stops`（fake.rs:6703） | fake | settle 30 → `now`+50 → settle 400 → `stopped_at=now` → Stop → 装置時計を 1 ms ずつ待つ → Abort | (2)＋(4)（部分） | `s1 − stopped_at < 50 ms` は，Stop の提出前に 50 ms 遅れると落ちる．記録した stop の at を基準にする | fake.rs:6703-6743 |
| 88 | `ur_16_a_panicking_later_stream_ends_the_run`（fake.rs:6815） | fake | settle 500，DEVICE_LOST 1 件 | S | 判定は喪失 1 件と rejected | fake.rs:6815-6864 |
| 89 | `ur_26_a_receive_start_queued_by_the_stop_is_issued_and_ends_at_its_origin`（fake.rs:6933） | fake | settle 50，`now`+50 の cold，合図を待ち，Stop → settle 300 | S | 判定は start 2 回・stop 2 回・clock なし | fake.rs:6933-6961 |
| 90 | `ur_25_a_receive_cut_counts_its_own_stream_s_packet_size`（fake.rs:6964） | fake | settle 100 → `now`+300 → settle 700；plan の gap | S | gap は plan の origin/end から計算する | fake.rs:6964-6984 |
| 91 | `ur_25_a_transmit_rate_change_of_a_later_stream_cuts_that_stream_alone`（fake.rs:7011） | fake | settle 50/80，`burst_in` ms(30)，`now`+50 cold → settle 300 | S | 判定は s0 の clock が 1 つ・s1 が 2 つ・s0 は送り続ける | fake.rs:7011-7043 |
| 92 | `ur_25_a_successful_change_clears_a_later_stream_s_halt`（fake.rs:7100） | fake | settle 30，cold 2 つ（None），settle 300；`now`+20 timed → settle 60 | S | 判定は start +1 と issued row（遅れても issued） | fake.rs:7100-7131 |
| 93 | `rx_start`（fake.rs:7158） | fake | `OnceFails` の待ち（`host.sleep` 1 ms ×2000） | S | 装置の wrapper の待ち；合図 | fake.rs:7158-7169 |
| 94 | `ur_15_a_failed_later_start_closes_after_the_owners_joined`（fake.rs:7190） | fake | `t0=now`+2 s+1 で start | (4)（止めた） | 2 s 遅れると start が late で別の拒否になり落ちる．止めた時計で固める | fake.rs:7190-7215 |
| 95 | `ur_29_a_lost_later_start_is_published_before_the_owners_are_stopped`（fake.rs:7218） | fake | ('`t0=now`+2 s+1 で start', '2 s 遅れると start が late になり，喪失の公開の場面が崩れる．止めた時計で固める') | (4)（止めた） | 2 s 遅れると start が late になり，喪失の公開の場面が崩れる．止めた時計で固める | fake.rs:7218-7240 |
| 96 | `ur_30_the_plan_names_every_stream_at_the_stop`（fake.rs:7245） | fake | settle 50 → `now`+20 ×2 → settle 250 | S | 判定は plan の行と stop の cut（stop 行から計算） | fake.rs:7245-7300 |
| 97 | `wait_capture`（uhd.rs:70） | Spec（手動） | `run.now()`+3 s の horizon で CAPTURE_WRITTEN | S | horizon は秒；待つのは event | uhd.rs:70-75 |
| 98 | `uhd_57_the_session_loopback_captures_what_it_transmits`（uhd.rs:179） | Spec（手動） | `now`+100 ms の capture，3 s horizon，offset を探す | S | 判定は sampler が波形のどれかの offset で連続すること；遅れても capture は次のブロックから | uhd.rs:179-205 |
| 99 | `uhd_61_02_capture_starts_at_the_requested_sample_index`（uhd.rs:225） | Spec（手動） | `now`+10 ms の first block，`index = now + 100 000 samples`；`first >= index` | S | 「以後」の下限；厳密な開始は `uhd_59` が Spec で固める | uhd.rs:225-244 |
| 100 | `after`（mod.rs:332） | Spec（手動） | `run.now()`+ticks | S | helper | mod.rs:332-336 |
| 101 | `captured`（mod.rs:408） | Spec（手動） | 10 s horizon | S | 秒の horizon；待つのは event | mod.rs:408-412 |
| 102 | `rehearse_overflow`（mod.rs:594） | Spec（手動） | 2 s+stall+500 ms の horizon | S | 秒の horizon | mod.rs:594-620 |
| 103 | `rehearse_session_loopback`（mod.rs:674） | Spec（手動） | `after_ticks(50 ms)` の capture，3 s horizon | S | 判定は loopback の相関と TIME_ERROR；遅れは capture を後にするだけ | mod.rs:674-723 |
| 104 | `after_ticks`（mod.rs:722） | Spec（手動） | `run.now()`+ticks | S | helper | mod.rs:722-725 |
| 105 | `ur_25_a_receive_enable_counts_its_start_lead_from_the_end_of_its_configuration`（fake.rs:1714） | fake | update(None) → settle 300 → 記録した e/ready と origin | S＋(4) | e と ready は control が記録した値で，試験のスレッドの遅れは関係しない．`configured − e >= 80 ms` の 1 tick は §3（丸め）で，スレッドの遅れの型ではない | fake.rs:1714-1734 |
| 106 | `ur_25_a_cold_change_the_envelope_refuses_changes_nothing`（fake.rs:1738） | fake | update(None) → settle 30 | S | 拒否は booking で決まる | fake.rs:1738-1752 |
| 107 | `ur_14_a_device_error_while_booking_names_its_action`（fake.rs:1755） | fake | update(None) → settle 100 | S | 拒否は booking | fake.rs:1755-1769 |
| 108 | `ur_26_an_unnumbered_key_is_refused_at_runtime`（fake.rs:1867） | fake | update(None) → `until` | S | 拒否は key で決まる | fake.rs:1867-1877 |
| 109 | `ur_26_an_abort_publishes_nothing_after_the_stop_instant`（fake.rs:1884） | fake | settle 50 → Abort → link の block | S | 判定は published の終わり < stop（記録した stop の at） | fake.rs:1884-1913 |
| 110 | `ur_25_a_cold_change_booked_anywhere_in_a_long_receive_call_is_on_time`（fake.rs:2724） | fake | settle(200+phase·step) → update(None) → settle 600；floor を `booked_at` から計算 | S | 判定は `origin − booked_at ≥ floor` と `at < origin`（どちらも記録から） | fake.rs:2724-2752 |
| 111 | `ur_26_a_second_stop_does_nothing`（fake.rs:3360） | fake | settle 30 → stop ×2 | S | 判定は stop row 1 件 | fake.rs:3360-3375 |
| 112 | `ur_12_a_layout_declares_and_starts_every_stream`（fake.rs:4434） | fake | settle 100，link から block | S | 判定は open/start/clock と block の channel | fake.rs:4434-4477 |
| 113 | `ur_25_a_channel_s_antenna_change_cuts_its_stream`（fake.rs:4628） | fake | update(None) → settle 150 | S | 判定は stop<apply と `origin ≥ end+55 ms`（下限） | fake.rs:4628-4654 |
| 114 | `ur_25_an_antenna_change_of_a_channel_in_no_stream_only_records`（fake.rs:4657） | fake | update(None) → settle 120 | S | 判定は apply なし | fake.rs:4657-4673 |
| 115 | `ur_25_a_refused_antenna_read_back_halts_the_stream`（fake.rs:4676） | fake | update(None) → settle 150 | S | 判定は拒否 1 件 | fake.rs:4676-4701 |
| 116 | `ur_25_the_last_receive_channel_leaves_no_clock_and_no_block`（fake.rs:4994） | fake | settle 30 → update(None) → settle 300 | S | 判定は open/start 数不変 | fake.rs:4994-5016 |
| 117 | `ur_25_a_layout_instance_s_full_duplex_rule_is_judged_at_runtime`（fake.rs:5735） | fake | settle 30 → update(None) → settle 200 | S | 判定は拒否と clock 2 つ不変 | fake.rs:5735-5769 |
| 118 | `ur_25_a_layout_instance_s_throughput_rule_is_judged_at_runtime`（fake.rs:5772） | fake | (None, '判定は拒否（RM-32）と clock 2 つ不変') | S | 判定は拒否（RM-32）と clock 2 つ不変 | fake.rs:5772-5801 |
| 119 | `ur_30_every_applied_key_is_declared_by_the_instance`（fake.rs:5854） | fake | settle 30 → finish | S | 判定は applied の key が宣言済み | fake.rs:5854-5876 |
| 120 | `ur_19_a_layout_receive_report_names_its_stream`（fake.rs:5957） | fake | settle 250 | S | report の source | fake.rs:5957-5972 |
| 121 | `ur_28_a_layout_transmit_report_names_its_stream`（fake.rs:5975） | fake | (None, 'TX_UNDERFLOW の source が `usrp/tx/s0`') | S | TX_UNDERFLOW の source が `usrp/tx/s0` | fake.rs:5975-5990 |
| 122 | `ur_26_an_unserved_command_is_refused_under_its_own_action_name`（fake.rs:6295） | fake | settle 30 → push → settle 30 | S | 拒否は command で決まる | fake.rs:6295-6315 |
| 123 | `ur_13_a_linkless_receive_stream_has_no_streamer`（fake.rs:6436） | fake | settle 100 | S | 判定は open/start の数 | fake.rs:6436-6449 |
| 124 | `ur_26_a_stop_of_a_later_stream_stops_that_stream_alone`（fake.rs:6541） | fake | settle 30 → stop → settle 120 → start_rx → settle 200 → stop instance → settle 120 | S | 判定は各 cut の数 | fake.rs:6541-6570 |
| 125 | `ur_26_every_stream_s_threads_are_joined_in_order`（fake.rs:6746） | fake（Gated） | settle 50/100，hold，`entered_n`，stop | S | 判定は join の順（rejected の順）；合図で gate | fake.rs:6746-6781 |
| 126 | `ur_16_a_wedged_later_stream_leaves_the_others_joined`（fake.rs:6784） | fake（Gated） | (None, '判定は detached が `uhd-rx-s1` だけであること') | S | 判定は detached が `uhd-rx-s1` だけであること | fake.rs:6784-6812 |
| 127 | `ur_26_a_start_rx_of_a_channel_less_linked_stream_starts_nothing`（fake.rs:6987） | fake | settle 30 → start_rx → settle 120 → cold(None) → settle 300 | S | 判定は start の数 | fake.rs:6987-7008 |
| 128 | `ur_26_a_stop_of_a_later_transmit_stream_ends_only_its_burst`（fake.rs:7046） | fake | settle 50/80 → stop → settle 120 → 読んだ数 → settle 100 | S | stop が効いていれば後の待ちで送信は増えない（増えるのは stop の失敗だけ） | fake.rs:7046-7074 |
| 129 | `ur_26_a_stopped_later_stream_waits_for_start_rx`（fake.rs:7077） | fake | settle 30 → stop → settle 120 → cold(None) → settle 300 | S | 判定は start の数 | fake.rs:7077-7097 |
| 130 | `ur_07_callbacks_run_outside_the_schedule_lock`（fake.rs:368） | fake | 時計を読み，別 thread から追加予約 → 過去の予約の規則と handshake | S | 判定は過去の予約を受け付ける規則と handshake で，短い窓に依らない | fake.rs:368-402 |
| 131 | `ur_25_a_receive_session_follows_its_recorded_plan`（fake.rs:1917） | Spec（手動） | `wait` が `after` → `run.now()` を読む | S | 判定は記録済み plan と clock の対応（clock の付かない segment を許す）で，遅れは次の block を選ぶだけ | fake.rs:1917-1966 |
| 132 | `ur_23_a_stop_ends_the_burst_within_the_in_flight_window`（fake.rs:1574） | fake | `send` → `tx_at`，`wait` → `after`（37a で修正済み） | S | Stop の ledger 時刻と end-of-burst だけを見る．上限の窓は `tx.rs` の unit test が固める | fake.rs:1574-1599 |
| 133 | `capture_at`・`status`（protocol.rs:763） | Spec（手動） | helper（呼び側の行で判定） | S | helper | protocol.rs:763-781 |
| 134 | `ea_12_a_duration_is_whole_root_ticks`（protocol.rs:797） | Spec（手動） | 返った horizon と now の関係 | S | 関係は遅れに依らない | protocol.rs:797-809 |
| 135 | `ea_07_a_session_on_the_fake_device`（protocol.rs:828） | Spec（手動） | 繰り返し波形の offset を探索 | S | offset の探索は遅れに依らない | protocol.rs:828-844 |
| 136 | `ea_17_a_capture_ahead_starts_at_its_instant`（protocol.rs:1089） | Spec（手動） | `first >= requested`（37a で修正済み） | S | 「以後」の下限 | protocol.rs:1089-1104 |
| 137 | `ea_17_a_capture_at_a_passed_instant_says_where_it_started`（protocol.rs:1108） | Spec（手動） | 遅れは「過去」という条件を強める | S | 同上 | protocol.rs:1108-1119 |

## 2. 数

| 判定 | 行数 | 行 |
|---|---|---|
| S | 104 | 1，3，4，7，8，9，10，11，12，13，14，15，17，18，19，20，21，22，23，24，25，26，28，29，30，31，32，33，34，35，36，38，39，40，42，45，46，49，50，51，52，53，54，55，56，58，68，69，72，73，77，78，80，81，82，83，85，88，89，90，91，92，93，96，97，98，99，100，101，102，103，104，106，107，108，109，110，111，112，113，114，115，116，117，118，119，120，121，122，123，124，125，126，127，128，129，130，131，132，133，134，135，136，137 |
| D | 2 | 2，5 |
| W | 2 | 70，84 |
| (1) | 3 | 27，44，67 |
| (2) | 4 | 6，57，79，86 |
| (4) | 14 | 16，37，41，43，47，48，59，60，61，62，65，71，75，76 |
| (2)＋(4)（部分） | 1 | 87 |
| (4)（部分） | 1 | 66 |
| (4)（止めた） | 5 | 63，64，74，94，95 |
| S＋(4) | 1 | 105 |
| 合計 | 137 | |

## 3. `ur_25_a_receive_enable_counts_its_start_lead_from_the_end_of_its_configuration` が 1 tick 短く落ちた件

- 観測（指示書）：`e=404266100`，`ready=420266099`，差 15 999 999 ticks。MCR 200 MHz で 1 tick = 5 ns，80 ms = 16 000 000 ticks。つまり **80 ms よりちょうど 1 tick 短い**。
- `e` は `book_cold` が booking の瞬間に読む `now`（`control.rs:731`）を `layout_instant`（`:736`，`timeline.rs:398-403`）に通した値。`at=None` なので `e = max(now, T0)` = now。`ready` は 863 で `now` に初期化され，`from_none`（0 チャネルからの enable）の経路だけ 917 で構成（`open`，`check`，`configure`）の後に `self.core.now()` で読み直される。このテストは from_none なので 917 の値が使われる。両方が `cold_change` の行に載る（`:948`）。
- `Core::now` は Authority の `now(root)`（`core.rs:911-913`）で，`extrapolate`（`authority.rs:142-148`）= `anchor.device + (host.now() − anchor.host) × 200 MHz / 1e9` の整数除算。
- `anchor.device` は装置自身の `time_now()`，すなわち `base`（`device.rs:762-771`）= `(host − zero) × 200 MHz / 1e9` の整数除算。`anchor.host` は手動の時計では read の幅が 0 なので read の瞬間そのもの（`authority.rs:102-120`，`123-140`）。
- したがって `now()` は「2 つの floor の和」で，**同じ瞬間の装置の値（1 つの floor）以下になる**（`floor(a)+floor(b) ≤ floor(a+b)`）。古いアンカーの和が真値より 1 tick 高くなることはない（Sol の指摘；表の前の版の「単調 guard の `last` が 1 tick 高くする」という説明は誤りなので削除した。`last` は `fetch_max` で，この前提では真値を越えない）。
- 1 tick のずれは**再アンカー**で作られる：`uhd-clock` は 100 ms ごとに再アンカーする（`authority.rs:33-34`，`465-497`）。再アンカーは丸めの位相を変える。Sol の例（200 MHz，host 時刻を ns で表す）：古い anchor が 0，`e` の読取りが 5 → `e = 1 tick`；新しい anchor が 20 000 001，`ready` の読取りが 80 000 005 → 外挿は 16 000 000 ticks。差は 15 999 999 ticks だが，host 上の経過は正確に 80 ms。
- `apply_delay` の sleep 自体は 80 ms ちょうど（M7，`host.rs:307-315`：期限は現在の clock + d，clock は ns の Duration）。**sleep が早く終わるのではない**。実際の失敗がこの再アンカーだったかは，ログが無いので **INFERRED**。
- 結論：**本番の誤りではない。** 装置は 80 ms の構成を行い，`origin` の規則（`origin = (max(e, e_c) + 199)/200*200`）は厳密に成り立つ。落ちたのは「2 つの外挿の差 ≥ 80 ms」という**テスト側の 1 tick の主張**で，Authority の推定は tick の量子化を持つ（UR-7 の bracket の規則は 1 ms 幅で，2 つの `now` の差の厳密性は約束していない）。
- 37c で直した：fake の側は `configured − e >= ms(80)` を消し，`origin` の厳密な関係（今の 1733 行）を残した．構成の終わり `c` を既知にした `src/provider/control.rs` の制御時計の unit test（`ur_25_a_receive_enable_counts_its_start_lead_from_the_end_of_its_configuration`）が，(a) `cold_change` の行の `ready` が booking の時刻ではなく `c` を記録することと，(b) `origin = ceil_lattice(max(e, c + lead))` が厳密に成り立つことを別々に assert する（§2 の指示）。

## 4. grep の外で見つけた行

sites の 8 つの pattern（`direct.now()`，`run.now()`，`time_now()`，`Instant::now()`，`thread::sleep`，`host().sleep`，`after(&run`，`after_ticks(`）にも settle の pattern にも当たらないが，試験のスレッドが時刻を読む（または helper の中で読む）系列。**どれも S**：

- `ur_21_a_descheduled_timed_send_is_on_time_on_the_manual_clock_and_late_on_the_real_one`（`fake.rs:3123`）：`tx_at(&run, ms(30))` + `send_at(start_repeat)` + `bounded`。`tx_at` の呼び出し行は sites の pattern に当たらない（definition の `fake.rs:1306` だけが載っている）。37a で `send_asap_and_flag` に直済み。S（判定は deschedule が `tx_send` に入ったことと，実時間側だけ LateAtDevice）。
- `ur_26_the_shutdown_s_last_burst_ack_waits_on_the_device_s_clock`（`fake.rs:3151`）：`tx_at(&run, ms(20))` + `send_at(start_repeat)`。S（asap，判定は `BurstAck` の code；窓は装置の時計で，hold は参加者）。
- `ur_21_late_policies_on_the_device_lead`（`fake.rs:1406`）：`send`/`send_at` 経由で `tx_at`。37a で `start_repeat` に直済み。S（判定は Drop と SendAsap の並び）。
- `ur_21_txburst_refusals`（`fake.rs:1360`）：`send("send", Some(ms(20)))`（NaN の波形）。拒否は波形で決まり，時刻に依らない。S。
- `timing_precondition_failure_reports_evidence_and_still_fails`（`fake.rs:1524`）：`send("send", Some(ms(1)))` は Drop を期待する。遅れは Drop を強める。S。
- `ur_21_an_untimed_send_is_on_time_after_its_delivery`（`fake.rs:1450`），`ur_21_an_untimed_burst_after_a_cold_change_is_on_time_at_its_clock_s_origin`（`:1463`），`ur_26_stop_actions`（`:1844`），`ur_28_the_device_reports_a_late_burst`（`:2026`）：`wait`/`send` だけで，読んだ時刻は判定の基準でない。S。
- `ur_25_tx_channels_from_zero_transmits_the_next_burst_on_time`（`fake.rs:1698`），`ur_25_tx_channels_to_zero_ends_the_clock_…`（`:3566`），`ur_25_an_enable_from_no_stream_configures_once`（`:3607`），`ur_25_a_rate_change_admits_a_burst_on_the_new_clock`（`:1803`）：untimed の `start_repeat` と `wait` だけ。S（INFERRED：untimed の予約の瞬間は Kernel が刻むので，試験のスレッドの読みではない）。
- `ur_23_a_burst_alone_is_ended_before_it_runs_dry`（`fake.rs:2617`），`ur_22_a_single_burst_ends_with_end_of_burst`（`:1507`），`ur_23_a_burst_ends_at_the_next_bursts_start`（`:1538`），`ur_25_a_cold_transmit_change_cancels_the_held_bursts`（`:1817`），`ur_23_a_burst_one_sample_after_a_burst_is_played`（`:2661`），`ur_23_a_one_sample_burst_booked_ahead_starts_at_its_time`（`:2795`），`ur_22_a_send_cut_short_ends_the_device_burst_after_what_it_took`（`:2813`），`ur_23_two_bursts_back_to_back_loop_back_whole`（`:2755`）：`finish_spec`/`start_instant`+`advance_to` で T0 から計算する（`run.now()` を直に読まない）。`ur_23_two_bursts…` は `captured` の horizon（行 101）を，ほかの `finish_spec` 組も同じ形の horizon を使う。S。
- `ur_26_the_sending_threads_join_before_the_receiving_ones`（`fake.rs:3543`），`ur_29_a_loss_found_by_the_shutdown_s_report_read_ends_its_ack_window`（`:3254`），`ur_07_a_reference_that_does_not_lock_is_named`（`:2914`）：`wait`/hold/合図だけで，読んだ時刻を使わない。S。
- `ur_34_a_move_destination_open_refusal_halts_both_lines_and_recovers`（`:4242`），`…source_reopen…`（`:4249`），`…destination_configure…`（`:4255`），`…source_configure…`（`:4262`）：行 10/11 の helper を呼ぶ。S（helper の行で判定）。

**grep が拾わない形そのもの**（2 つ）：
- **helper の中で時刻を読む**：`tx_at`（`:1306-1310`，`run.now()`）と `burst_in`（`:6639-6643`，`direct.now()`）は，どちらも definition の行だけが sites に載り，**呼び出し側の行**（`tx_at(&run, …)`，`burst_in(…, ms(40))`）はどの pattern にも当たらない。呼び出し側は上の 1〜6 の項目で拾った。`clock_of`（`:6645-6647`）は記録を読むだけで，時計は読まない。
- **`run.advance_to` の後に記録を読む**：`ur_34_the_receive_stall…`（`:3320`）と `ur_25_a_configure_overlapped…`（`:3292`）は，advance の後に `timing`/wrapper の記録を読む（時計は読まない）。どちらも S（行 34・35）。

## 5. していないこと

- この一覧の版は，コードとテストの変更（37c の実装）のあとの状態を記す。§3 の原因の説明はコードと指示書の観測値からで，どの read が 1 tick を落としたかは run のログが無いので **INFERRED**。
- (4)（止めた）の 5 行（63，64，74，94，95）は，この段階では unit test にしていない：それぞれの理由は `report.md`。表の判定は「(4) にすべきだが止めた」，fake のテストはそのまま残る（消していない）。
- 負荷の下の確かめ（設計メモ §3 の (c) と計画 30 §15）は orchestrator の gate。§4 の行のうち grep の外で見つけたものは，指示書の「S のまま」に従って一覧に足しただけで，個別のテストは変えていない。
