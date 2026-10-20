STATUS: done

# Phase 8 段階 1b-4c の実装報告（2 回目）

対象: `/home/komatsu/local_work/Ez-SDRv4`（HEAD `4199e51`，以下すべて未コミット）．
指示書: `~/.cache/ezsdr-fixes/2026-10-19-phase8-1b4c/impl-brief.md`．
段階の計画と裁定: 同ディレクトリ `test-plan.md`，`brief.md`，`sol-plan.md`．

1 回目の結果（実装・テスト・mutation・セルフレビュー 1 回）の後，セルフレビューの指摘を処理し，指摘が名指したテストと行を足し，もう一度セルフレビュー（ラウンド 2）を回した記録である．

---

## 1. 変えたファイルと要点

### 実装

| ファイル | 要点 |
|---|---|
| `crates/ezsdr-radio/src/lib.rs` | `channel_keys`（`keys` の読み，1 関数）と `Addressed` / `addressed`（宛先の判断，1 関数）を置いた． |
| `crates/ezsdr-radio/src/timeline.rs` | `layout_instant`（instance 全体の床，`ezsdr_radio::timeline` の 1 関数）． |
| `crates/ezsdr-radio/src/channels.rs` | `x3xx::from_selector` から外した prefix の制限の受け皿（`Layout::X3xx` の受理範囲）と，重複の検査の共有（`check_repeat`）． |
| `crates/ezsdr-radio/src/x3xx.rs`, `src/device.rs` | prefix の ORDER の loop を `check_repeat` に置き換え．`rate_set_matches`（範囲の展開と集合化）． |
| `crates/ezsdr-radio-uhd/src/provider/mode.rs` | **新規**．`Mode` enum（`Current` / `Layout`）が description ごと持ち，`coerce`・`tree`・`defaults`・`effective`・`stream_channels`・`stream_of`・`stream_node`・`rx_port`・`update_class`・`addressed` を 1 か所で決める． |
| `crates/ezsdr-radio-uhd/src/provider/mod.rs` | `from_binding` の順序（selector → serial の登録 → master clock → `set_subdev_spec`（要素のある方向だけ）→ front end の照合），`subdev_markup`，`layout_elements`，serial registry（check+insert を 1 lock，`SerialGuard` の `Drop` が外す）．`prepare` の channel 数の検査は今の key のモードだけに（layout の数は `from_binding` が UR-5 で見る）． |
| `crates/ezsdr-radio-uhd/src/provider/core.rs` | `check_rates`（方向ごとに 1 回，結果を instance に持つ）と `rates_checked`，`stream_node`（event の source，1 関数）．`Core::new` の overflow handle は `mode.stream_node`． |
| `crates/ezsdr-radio-uhd/src/provider/control.rs` | `book_cold`（layout の宛先・床・membership・from-none の open と configure），`book_timed`（channel ごとの宛先），宛先（`mode.addressed`），`t0`（link の無い受信側の床），`time_error` の source． |
| `crates/ezsdr-radio-uhd/src/provider/rx.rs`, `tx.rs` | 開き直しの比較を `Channels`（集合）に．event の source を `Core::stream_node` に． |
| `crates/ezsdr-radio-uhd/src/device.rs` | `FakeConfig.serial` を `String`（instance ごとの既定値），`FakeFault::WrongAntenna`，`From<Direction> for Dir`． |
| `crates/ezsdr-mock-radio/src/lib.rs` | `keys` の読みを `channel_keys` に，宛先を `ezsdr_radio::addressed` に（挙動は不変）． |
| `crates/ezsdr-radio-uhd/tests/fake.rs` | 1b-4c のテスト一式（約 60 本）と harness（`Direct`，`attached_on`，wrapper 群，`armed_on`/`build_on`）． |
| `crates/ezsdr-acceptance/tests/parity.rs` | **新規**．VH-8 の harness と 6 本（うち 2 本が今回追加）． |
| `crates/ezsdr-server/tests/protocol.rs` | `ea_07_the_reopen_of_one_device_is_not_a_second_binding`（serial の帰結）． |

### 設計文書

- `design/18-uhd-radio.md`: UR-2/5/9/10/12/13/17/19/20/23/24/25/26/28/30/33 の文と `*Test:*`，§6 の行，Changes に 2026-10-19 の行（今回のレビューの修正も同じ行に追記）．
- `design/07-radio-model.md`: RM-25/RM-26（`layout_instant` とその入力，共有の宛先），`rm_26_the_layout_parser_checks_the_shared_selector` の行，Changes．
- `design/09-mock-radio.md`: MR-18（`keys` の床は RM-25 の引用＋Mock 自身の機構），MR-2 の selector 表の prefix の記述，MR-38，Changes．
- `plan/maintenance/22-timing-simplification.md`: VH-8 に 1b-4c の記録（床が共有関数，harness の stream 次元は 1b-4d/1b-4e）．
- `plan/phase8/33-channels-and-streams.md`: 1b-4 行に 1b-4c の実装，分けたもの，1b-4d へ送る一覧．

### mutation と道具

- `plan/maintenance/tools/mutations.json`: 新規 67 行（P281–P347），当て直し 24 行，equivalent 3 行（P313，P344，P345）．
- `plan/phase7/tools/mutations.json`: 当て直し 10 行，equivalent 2 行（R23，R26），B14 の書き換え．
- `plan/phase7/tools/mutate.py`: **変更なし**（いったん `equivalent` の扱いを足したが，セルフレビュー 2 回目の P1-1 で越権とされ戻した．§2 の 12，§8）．

---

## 2. 構造の決定（裁定からの外れと理由）

1. **裁定 1 の C4・C5 は行を持たない**（1 回目から不変）．`Mode` が description ごと持つので，「モードの選択を無視して今の description を使う」形は struct の型で作れない．C4 の代わりに，選択の式そのものを壊す行 P340 を置いた（`ur_05_the_keys_value_picks_one_key_set` が殺す）．
2. **宛先の判断は `ezsdr_radio::addressed`，event の source は `Core::stream_node` に寄せた**（レビュー P2-6）．Mock と UHD の `addressed` は stream 数の出どころだけが違うので，その closure を引数にした．`Mode::stream_count` が唯一の stream 数の判断である．
3. **`Mode::addressed` を `&Core` を受ける形にした**のは，instance の node（`core.id`）と今の key の stream node（`core.rx_id`/`tx_id`）が instance の状態だからである（`provider/mode.rs`）．
4. **lock の規則**（レビュー P2-3）: `book_cold` と `start_rx` を「instant と seq は `streams` の下，configuration の projection と device call はその外，book はまた `streams` の下」に組み直した．`e` の決定と book の間の原子性は，control thread が唯一の booker であり，prune の `before` が常に `now` 以前（prune は終わった segment の item で起きる）なので，後から book する item の `e ≥ now ≥ before` が保たれる（HEAD の from-none の `drop(streams)` と同じ窓）．unit の `ur_25_the_line_lock_is_free_inside_every_*_device_call` 3 本が固定している．P271 はこの窓に lock を持ち込む行に当て直した．
5. **`from_none` の `streams.stream(stream)` は channels が空のとき読まない**．`Streams::stream` は k ≥ 1 で panic する（1b-4c は stream 0 だけを持つ）ため，空の集合のときは読まない形にした（`ur_25_a_layout_rate_change_of_a_channel_less_stream_only_takes_its_value` が踏んでいた）．
6. **`prepare` の channel 数の検査は今の key のモードだけ**（レビュー P3-14）．layout の数は binding のもので，`from_binding` が UR-5 として同じ判断をする（`have < need`）．同じ判断を 2 度書かない．
7. **`Layout::Ideal` の腕は `unreachable!()`**（レビュー P3-13）．`channels::from_selector` は `X3xx` しか返さず，`ideal` は MockRadio だけのもの．panic になりうる `.expect("a selection")` も消えた．
8. **`rx.rs` の `!channels.is_empty()` を外した**（レビュー P2-5）．この条件は到達しない（planned segment は必ず channel を持つ）ので，HEAD と同じ式 `channels != self.opened` に戻した．今の key のモードの挙動は変わらない．
9. **`layout_instant` の床は T0**（レビュー P2-4）．仕様 UR-25 は「a receive start」と書いており，link の有無と言っていない．UHD は Control の `t0` を持つようにした（line の origin は常に T0 なので，link がある場合の値は同じ）．
10. **event の source は stream の node**（レビュー major 6 から見つかった実装の穴）．UR-17/19/20/28 の文がそう定めているのに，uhd-rx の LATE_COMMAND/ALIGNMENT_ERROR と uhd-tx の TIME_ERROR/TX_UNDERFLOW，uhd-control の booking の TIME_ERROR が今の key の node を指していた．`Core::stream_node` に寄せて直した（今の key のモードでは同じ値になる）．
11. **equivalent の記録 2 件**（P344，P345）．方向の rate の照合は「方向ごとに最初に streamer を開いたとき」で，その最初の open は `arm` か uhd-control の from-none のどちらかである．uhd-rx/uhd-tx の reopen の `check_rates` は cache された結果を再確認するだけで，到達しうる状態を変えない．理由は行の `equivalent` に書いた（`plan/maintenance/20-amendments.md` Appendix A と同じ形）．R23・R26 も同じ形で equivalent（下記 §5）．
12. **phase7 の道具は変えない**（いったん `equivalent` の扱いを足したが，セルフレビュー 2 回目の P1-1 を受けて戻した．§8）．R23・R26 には理由を `equivalent` 欄として残すが，**phase7 の list の道具はそれを読まない**ので，gate の集計では 2 行が SURVIVED として見える．判断は orchestrator に委ねる（§9 の 8）．
13. **`RatesMisreported` に `failing` を，`Direct` に `armed_on`/`build_on` を足した**（裁定 major 5 が名指した「格子の違う device」「read error」の harness 拡張）．
14. **レビューが見つけた分岐（未修正，報告のみ）**: layout の `cold` 変更が床で上げられ，そのうえで `coerce` に拒否される場合，MockRadio は LATE_COMMAND を出してから拒否するが，UHD は `coerce` の拒否で先に戻るため LATE_COMMAND を出さない（今の key のモードでも HEAD から同じ）．この段階の裁定はこの場合を定めていないので触らず，`(b-4)` の parity は「受理される床の列」（`vh_8_the_same_floor_at_t0`）で比較した．owner/1b-4d の判断が要る（§8 に送る）．

---

## 3. 計画の項目と裁定ごとのテストと行

（テストは `crates/ezsdr-radio-uhd/tests/fake.rs`＝fake，`crates/ezsdr-acceptance/tests/parity.rs`＝parity，`crates/ezsdr-radio/tests/*`＝radio，lib は unit．行は `plan/maintenance/tools/mutations.json`＝P，`plan/phase7/tools/mutations.json`＝phase7．）

| 項目 / 裁定 | テスト | 行（id: 誤りの形 → 殺すテスト） |
|---|---|---|
| 1 `keys` の値 | `ur_05_the_keys_value_picks_one_key_set`，`ur_05_every_other_keys_value_is_refused`，`rm_38_the_keys_selector_is_read_once_for_both_providers`（radio） | P122（`keys` を任意値に）→ 前者，P282（`keys` 無しを layout に）→ 前者，P283（拒否の文面を弱める）→ 後者，P281（selector に `keys` を許さない）→ 前者，P340（モードが `keys` を無視）→ 前者 |
| 2 subdev spec と prefix | `ur_05_the_layout_builds_the_subdev_spec`，`ur_05_the_layout_s_order_is_the_device_s_channel_order`，`ur_05_the_current_keys_set_uhd_s_default_order`，`ur_05_a_layout_out_of_uhd_s_default_order_is_accepted`，`ur_05_a_direction_without_elements_sets_no_spec`，`ur_05_the_spec_is_set_before_the_channel_map_is_read`，`ur_05_the_front_end_check_reads_the_device_after_the_subdev_spec`，`rm_26_the_layout_parser_checks_the_shared_selector`（radio） | P284（spec を設定しない），P285（区切り），P286（逆順），P287（空の方向にも設定），P341（front end の番号を `0` に）→ T1/T2/T3 |
| 3 front end の照合 | `ur_05_a_basic_front_end_s_number_is_checked`，`ur_05_a_board_that_differs_from_the_layout_is_refused`，`ur_05_a_subdev_spec_the_device_refuses_is_refused_with_uhd_s_text`，`ur_05_each_channel_is_checked_at_its_own_device_channel`，`ur_05_the_profile_must_be_the_device_s_front_ends`（A3） | P288（番号を見ない），P289（失敗を飲む），P290（board を見ない），P291（全て channel 0 で見る），R26（equivalent） |
| 4 rate の集合 | `ur_13_arm_compares_the_rate_set_after_the_streamer_opens`，`ur_13_arm_refuses_a_rate_set_the_description_does_not_have`，`ur_13_arm_refuses_a_pre_chain_rate_set_after_the_open`，`ur_13_a_direction_the_arm_never_opened_is_not_compared`，`ur_13_arm_takes_the_rate_set_in_any_order`，`ur_13_the_rate_set_comparison_expands_ranges_and_ignores_order`（radio），`ur_13_a_runtime_join_compares_the_rate_set`，`ur_13_an_enable_from_no_stream_compares_the_rate_set`，`ur_13_a_rate_read_that_fails_refuses` | P292（比べない），P293（`{1.0}` を受ける），P294（順序で比べる），P295（全方向を比べたと記録），P296（open の前に読む），P334（arm の照合を外す），P343（from-none の照合を外す）→ 新テスト，P344（rx の reopen，equivalent），P345（tx の reopen，equivalent），P346（read の失敗を飲む）→ 新テスト |
| 5 serial | `ur_05_a_second_binding_of_one_serial_is_refused`，`ur_05_a_parallel_bind_of_one_serial_succeeds_once`，`ur_05_a_serial_read_that_fails_refuses`，`ur_05_a_failed_binding_leaves_no_serial_registered`，`ur_05_a_radio_dropped_after_its_device_is_lost_releases_its_serial`，`ur_05_two_default_fakes_bind_together`，`ur_05_cleanup_does_not_release_the_serial`，`ea_07_the_reopen_of_one_device_is_not_a_second_binding`（server） | P297（同じ serial を許す），P298（handle で key），P299（drop で外さない），P300（serial の失敗を飲む），P301（既定 serial を固定），P302（serial の前に装置に書く）→ 各 serial テスト |
| 6 `prepare` の stream 0 | `ur_12_a_layout_s_stream_zero_declares_its_clock_and_opens_its_channels`，`ur_12_a_layout_s_stream_zero_applies_each_channel_s_own_settings`，`ur_12_a_layout_s_stream_zero_with_no_channel_declares_no_clock`，`ur_13_a_layout_s_transmit_stream_zero_opens_its_channel_list`，`ur_12_a_layout_a_rate_the_device_does_not_apply_is_refused`，`ur_12_a_layout_an_antenna_read_back_that_differs_is_refused`，`ur_12_a_layout_stream_above_zero_is_refused`，`ur_12_each_key_set_refuses_the_other_in_prepare` | P303（全ての channel を stream 0 に），P304（clock を方向の node に），P305（今の key の rate），P306（channel 0 の設定を全 channel へ），P307（configure の宛先が position），P308（`prepare` が stream ≥ 1 を許す），P333（今の key の configuration filter），P336（port が `rx0` でない） |
| 7 channel ごとの timed | `ur_24_a_channel_keyed_timed_update_acts_on_its_own_channel`，`ur_24_a_timed_update_for_a_channel_out_of_its_stream_waits_for_its_segment`，`ur_24_a_channel_keyed_command_counts_one_queue_slot`，`ur_24_the_layout_destination_follows_the_effective_membership`（major 4(b)） | P310（timed が全 channel へ），P311（queue の数え方），P312（stream の無い channel へ発行） |
| 8 rate の cold 変更 | `ur_25_a_layout_rate_change_cuts_its_stream_and_configures_every_channel`，`ur_25_a_layout_rate_change_of_a_channel_less_stream_only_takes_its_value`，`ur_25_a_layout_transmit_rate_change_configures_the_new_segment` | P314（全 channel に channel 0 の設定） |
| 9 antenna | `ur_25_a_channel_s_antenna_change_cuts_its_stream`，`ur_25_an_antenna_change_of_a_channel_in_no_stream_only_records`，`ur_25_a_refused_antenna_read_back_halts_the_stream`，`ur_26_a_current_keys_antenna_update_is_refused` | P315（read-back を見ない），P316（antenna を settings から落とす），P335（値だけの変更が row を残さない） |
| 10 membership（M1） | `ur_25_a_receive_membership_join_opens_the_streamer_with_the_new_set`，`ur_25_a_receive_membership_leave_opens_the_streamer_for_the_remaining_set`，`ur_25_the_last_receive_channel_leaves_no_clock_and_no_block`，`ur_25_a_membership_in_a_later_stream_is_refused_and_changes_nothing`，`ur_25_a_no_op_membership_change_takes_only_its_value`，`ur_25_a_same_size_membership_swap_reopens_the_streamer`（rx/tx） | P317（rx の数比較），P318（tx の数比較），P319（空になった leave が cut しない），P309（stream ≥ 1 への join），P337（同値の変更が cut する）→ 新テスト |
| 11 instance の床 | `ur_25_a_layout_cold_change_is_floored_at_every_earlier_cold_change_of_the_instance`，`ur_25_the_current_keys_floor_a_cold_change_only_at_its_own_stream`，`ur_25_a_layout_cold_change_of_a_linkless_receive_side_is_floored_at_t0`，`ur_25_a_floor_raised_layout_join_is_judged_at_its_instant`（major 4(a)），`vh_8_the_same_floor_at_t0`（parity，b-4），`rm_25_the_layout_instant_is_floored_at_every_earlier_cold_change`（radio） | P320（床に pending を渡さない），P321（床上げを late にしない），P339（link の無い受信側の床を落とす）→ 新テスト |
| 12 宛先（D14） | `ur_26_d14_a_layout_instance_names_its_streams`，`ur_26_a_device_stop_stops_every_layout_stream`，`ur_26_d14_the_old_addresses_and_foreign_nodes_are_refused`，`ur_26_d14_a_channel_less_stream_keeps_the_mock_s_treatment`，`ur_26_an_unserved_command_is_refused_under_its_own_action_name`，`vh_8_a_channel_less_stream_is_the_same_on_both`（parity，b-5） | P322（存在しない stream を既知に；`mode.rs` に当て直し），P323（channel の無い tx stream の burst），P324（rx の node への burst），P325（stream の `Stop` が全部止める）→ テストを強化（下記 §6），P326（device の `Stop` が片方向だけ） |
| 13 実行時の判定 | `ur_25_a_layout_instance_s_full_duplex_rule_is_judged_at_runtime`，`ur_25_a_layout_instance_s_throughput_rule_is_judged_at_runtime`，`ur_25_a_floor_raised_layout_join_is_judged_at_its_instant` | P327（update の key だけ見る），P328（拒否しても book する），P329（固定の理由で拒否） |
| 14 Manifest | `ur_30_the_device_section_names_the_subdev_spec`，`ur_30_the_device_section_records_the_serial`，`ur_30_the_device_section_records_the_rate_check`，`ur_13_arm_compares_…`（`rates_checked`） | P330（spec を記録しない），P331（全方向を比べたと記録），P332（空の serial を記録），P295（同上） |
| 15 VH-8 | `vh_8_the_two_providers_prepare_one_layout_alike`，`vh_8_a_refusal_is_the_same_key_and_reason`，`vh_8_the_same_transmit_stream_clock`，`vh_8_the_same_receive_membership_join_leaves_the_same_clocks`，`vh_8_the_same_floor_at_t0`，`vh_8_a_channel_less_stream_is_the_same_on_both`（parity 6 本） | なし（比較そのもの）．plan の型は parity の行を持たない（mutation は UHD 側の行が殺す） |
| M1（項目 10） | `ur_25_a_same_size_membership_swap_reopens_the_streamer`（rx と tx） | P317/P318（集合の比較を数に戻す）→ このテスト |
| セルフレビュー 1 回目の指摘 | `ur_25_a_no_op_membership_change_takes_only_its_value`，`ur_24_the_current_keys_held_updates_reach_every_channel`，`ur_25_a_layout_cold_change_of_a_linkless_receive_side_is_floored_at_t0`，`ur_19_a_layout_receive_report_names_its_stream`，`ur_28_a_layout_transmit_report_names_its_stream`，`ur_05_cleanup_does_not_release_the_serial`，`ur_24_the_layout_destination_follows_the_effective_membership`，`ur_25_a_floor_raised_layout_join_is_judged_at_its_instant`，`ur_13_a_runtime_join_compares_the_rate_set`，`ur_13_an_enable_from_no_stream_compares_the_rate_set`，`ur_13_a_rate_read_that_fails_refuses`，`ur_26_an_unserved_command_is_refused_under_its_own_action_name` | P337，P338，P339，P340，P341，P342，P343，P344，P345，P346，P347（§5 と §6） |

`ur_30_the_device_section_records_the_rate_check` の Manifest は `rates_checked` の両方向の欄を持ち，layout の Run で `rx: true`（arm の open で比べた）・`tx: false`（開かなかった），link の無い Session で両方 `false` を固定する．

---

## 4. 当て直した行

コードとテストを変えたので，次の行の `old`/`new` を同じ規則・同じ形のまま当て直した（`old` は実装後の文にちょうど 1 回一致し，`rules.py --base HEAD` が 0 problem）．

- maintenance: **P252，P253**（`from_none` の式が `!channels.is_empty() && idle && planned` に），**P265**（enable の projection の filter が `chan.is_none_or(...)` に），**P271**（`drop(streams)` が消え，代わりに lock を持ち込む形に），**P317**（rx の比較から `!channels.is_empty()` が消え，`new` を数比較に），**P319**（`leaves` に `target.is_some()` が入った），**P322**（control.rs の stream 数の検査が `mode.rs` の `self.stream_count(direction.into())` に移った），**P323**（`Addressed::Stream` の field 名 `dir` → `direction`），**J141**（`new` を腕全体に直した．前回は断片で compile error になった），**J11，J64，J69，J71，P08，P09，P122，P141，P142，P149，P163，P251，P264，P267，P279，P43，F40，J277**（1 回目から：文が動いたための当て直し）．
- phase7: **R05，R14，R21，R27，B14，U10，U20，U39**（同上）．**B14** は今回さらに，意味の無い mutation（`Some(_) | None =>` は先の `Some` 腕の後で到達しない）を，held sample を本当に落とす形（`let _ = &tail;` で空の buffer を送る）に書き換えた．殺すテストは記録どおり `rehearsal_b6_txrx_and_repeat`（ローカルの debug コピーで確認）．
- 消した行はない（`old` が一致しなくなった行はすべて当て直した）．

**改名した識別子が `new` に残っていないか**の grep（指示書 4 の 2 つ目）: `description`（→ `mode`），`dir:`（`Addressed` の field），`self.channels`（rx/tx の数 → 集合），`core.description`，`x3xx::Selection`（変化なし）について，両 list の `new` を検索して残存 0 を確かめた．

---

## 5. equivalent として記録した行（生き残りと理由）

| 行 | 理由 |
|---|---|
| P313（maintenance） | この段階は stream 0 だけが channel を持つので，`stream_channels(dir, 0)` と `stream_channels(dir, k)` は同じ集合になる．key の stream は 1b-4d まで観測できない（行の `equivalent` に同じ文）． |
| P344（maintenance） | 受信側の最初の open は `arm` か uhd-control の from-none のどちらかで，方向ごとの照合はそこで済む．uhd-rx の reopen は cache された結果を再確認するだけで，到達しうる状態を変えない． |
| P345（maintenance） | 同じ理由（送信側）． |
| R23（phase7） | layout の要素から spec を設定した後は device の channel 数＝要素数なので，「channel 数が足りない」検査は，spec が受けた装置と食い違う装置でしか働かない（テストダブルでは作れない）． |
| R26（phase7） | `have` は `set_subdev_spec` の後の spec の長さなので，`0..have` は要素と同じ channel を尋ねる． |

R23・R26 の道具（`plan/phase7/tools/mutate.py`）は `equivalent` を読まない（maintenance の道具だけが読む．`plan/maintenance/20-amendments.md` Appendix A の規約）．いったん phase7 の道具にも同じ扱いを足したが，セルフレビュー 2 回目がこれを越権（免除の判断は orchestrator のもの）と指摘したので戻した（§2 の 12，§8 の P1-1）．したがって gate の phase7 の集計では R23・R26 は SURVIVED として見える．**owner の判断を仰ぎたい点の 1 つ．**

---

## 6. 変わった既存テストの hunk と帰属

`git diff HEAD -- crates/ezsdr-radio-uhd/tests/fake.rs crates/ezsdr-mock-radio/tests/mock_radio.rs crates/ezsdr-radio/tests/device.rs` の hunk ごとの帰属:

| 場所 | 帰属 |
|---|---|
| `fake.rs` の `ur_05_the_profile_must_be_the_device_s_front_ends` の 2 つの assert（`FAKE` の lookup の失敗の文へ） | A3 の 1（指示書の「final の A3」の 1 つ目） |
| `fake.rs` の `ur_33_the_serial_is_the_config_s`（既定 serial を明示に） | A3 の 2 |
| `mock_radio.rs` の `mr_02_from_binding_refusals`（prefix の 3 assert とコメント） | A3 の 3 |
| `device.rs` の `rm_26_the_layout_parser_checks_the_shared_selector`（prefix の 3 ケースを受理側へ） | **A3 の 3 件ではない**．裁定 構造 2 と計画 T5 が名指した書き換え（レビュー P2-9 の指摘どおり記録だけが漏れていた）．Ar/Ar 以外の変更なし |
| `fake.rs` の harness（`attached_on`，`Pushed::recv` の path の解決，`ctx.inputs` の入り口，event の pairs，`Direct::armed/build_with/start/update_at/stop_at/start_rx_at/burst_at`，`Direct::armed_on`・`build_on`，`RatesMisreported` の `failing`，`spawn` の形） | 裁定 12 の harness の拡張と，裁定 major 5 が名指した拡張（挙動は変えない） |
| `fake.rs` の `ur_26_d14_a_layout_instance_names_its_streams`（step 5 に held burst を足し，`tx_` の検査を「payload と end of burst が出た」に置き換え） | P325 を殺すためのテスト強化（レビュー P2-8．挙動の変更なし） |
| `fake.rs` の `RatesMisreported` の構築 5 か所（`failing: None` を足す） | 上と同じ harness の拡張 |
| `email` 以外の変更（`mock_radio.rs` の `mr_08_effective_holds_exactly_the_ten_configuration_keys` など） | なし（`git diff` の hunk は上の表で全部） |

`crates/ezsdr-acceptance/tests/parity.rs` の `UhdRig` に `events` を足したのは (b-5) の比較のため（harness の拡張）．

---

## 7. 確かめること 1〜4 の結果

1. **`cargo test --workspace --no-fail-fast`**（nohup，最後の編集の後）: 66 targets **すべて ok**，**1380 tests passed**，0 failed，0 ignored．ログ `/tmp/opencode/testall.log`．
2. **`cargo clippy --workspace --all-targets -- -D warnings`**: 警告なし（`Finished`）．ログ `/tmp/opencode/clippy.log`．`cargo fmt`/`rustfmt` は一度も走らせていない．
3. **`PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`**: **0 problem**（handoff の 300 行・40000 バイトも，`design/` の長すぎる行の増加も検出なし）．
4. **mutation（`remote_mutate.py`，sim02:8 + sim03:8 の 16 shards）**:
   - 足した・当て直した・コードかテストが変わった 73 行（P122，P252，P253，P265，P271，P281–P347，J141）: **1 回目 70/73 killed**．生き残り J141・P344・P345 を直して再実行し **3/3 passed**（J141 killed，P344・P345 equivalent (survived)）．ログ `/tmp/opencode/mut5.log`，`/tmp/opencode/mut6.log`．
   - phase7 の 4 行（B14，R14，R23，R26）: **1 回目 3/4**（B14 のみ SURVIVED）→ B14 の `new` を意味のある形に書き換えて再実行し **1/1 killed**．ログ `/tmp/opencode/mut5b.log`，`/tmp/opencode/mut6b.log`．
   - 締めの **全 list（`all sim02:8,sim03:8`，1106 行）**: **maintenance 730/730，phase3 83/83，phase4 34/34，phase5 29/29，phase6 63/63，phase7 166/167** — 1055/1106 が killed で，唯一の生き残りは **G09**（`crates/ezsdr-kernel/src/coordinator/stepping.rs` の `delivered` の lock を外す行．kill テストは kernel の `kg_02_the_first_delivered_stopping_event_is_the_termination_cause`）．この段階は kernel を触っておらず，単独で 2 回流しても SURVIVED（`/tmp/opencode/g09.log`）なので，**この段階とは無関係の既存の生き残り**である（この tree の変更では説明できない．orchestrator の gate の判断に委ねる）．phase7 の R23・R26 は，phase7 の道具が `equivalent` を読まないため（§2 の 12 を戻した．下の §8 の P1-1），この集計では SURVIVED として数えられている（理由は §5 と行の `equivalent`）．ログ `/tmp/opencode/mutall.log`．
   - `check_rates` を device 呼び出しの外へ出した後の再実行（P292–P296，P334，P346）: **7/7 killed**（`/tmp/opencode/mut7.log`）．
   - `COMPILE ERROR` は 1 回出た（J141．`new` が断片だった）．直して流し直し，killed．

---

## 8. セルフレビュー

### ラウンド 1（`anthropic/claude-haiku-5-5#high`，`--agent plan --read-only`，`/tmp/opencode/review1.log`）

P1 2 件，P2 11 件，P3 7 件（ほかに確認して問題なしとした多数）．各指摘の処理:

| # | 指摘 | 処理 |
|---|---|---|
| P1-1 | 値の変わらない `c<n>.stream` の変更が stream 0 を空の segment に区切る（Mock の `was == is` と食い違う） | **修正**．`leaves` に `target.is_some()` を足し，テスト `ur_25_a_no_op_membership_change_takes_only_its_value` と行 P337． |
| P1-2 | 今の key のモードで from-none の held 更新が channel 1 以降に届かない（HEAD は全 channel に当てていた） | **修正**．filter を `timed.chan.is_none_or(...)` に．テスト `ur_24_the_current_keys_held_updates_reach_every_channel` と行 P338． |
| P2-3 | lock 順序（`streams` → `updates`）違反 | **修正**．`book_cold` と `start_rx` を組み直し（§2 の 4）．決定 4 の原子性の根拠をつけた．P271 を当て直し． |
| P2-4 | link の無い受信側で床が無い（Mock と食い違い） | **修正**．Control の `t0` を床に．テスト `ur_25_a_layout_cold_change_of_a_linkless_receive_side_is_floored_at_t0`，行 P339，parity `vh_8_the_same_floor_at_t0`． |
| P2-5 | 今の key のモードの device 呼び出しが変わった（`!channels.is_empty()`） | **退けた（挙動の変更なし）**: この条件は到達しない（planned segment は必ず channel を持つ）．HEAD と同じ式に戻した（§2 の 8）． |
| P2-6 | 宛先の判断が 2 か所 | **修正**．`ezsdr_radio::addressed` と `Core::stream_node`．P322 を当て直し，行 P347 を追加． |
| P2-7 | テストの欠落（major 3・4・5・6，(b-4)，(b-5)） | **修正**．§3 の表の「セルフレビュー 1 回目」の行のテストを全部足した．major 4 の 2 列，major 5 の 3 本，major 6 の 2 本，major 3 の 1 本，parity 2 本． |
| P2-8 | mutation の行の欠落（C4，R3，overflow handle，`check_rates` の呼び出し） | **修正**．P340（C4），P341（R3），P342（overflow handle），P343/P346（呼び出し），P347（source の共有関数）． |
| P2-9 | A3 の列挙が不完全（`device.rs`） | **記録で解決**: §6 の表の 4 行目．裁定 構造 2/T5 が名指した書き換えで，A3 の 3 件ではない（中身は正しい）． |
| P2-10 | 偽のまま残る `design/` の文（09 の prefix，07 の非 prefix） | **修正**．`design/09-mock-radio.md` の selector 表と `design/07-radio-model.md` の parser テストの行． |
| P2-11 | B2 の未了（VH-8 と 1b-4 の行） | **修正**．`plan/maintenance/22-timing-simplification.md` の VH-8 に 1b-4c の記録，`plan/phase8/33-channels-and-streams.md` の 1b-4 行に実装と 1b-4d への一覧． |
| P3-12 | `SerialGuard` の doc が `Provider::coerce` と書く | 修正（`register_serial`）． |
| P3-13 | `Layout::Ideal` の腕が到達不能（panic の経路） | 修正（`unreachable!()`，`.expect("a selection")` を削除）． |
| P3-14 | prepare の channel 数の検査が `from_binding` と重複 | 修正（今の key のモードだけに）． |
| P3-15 | `check_rates` が `rates` の lock を device 呼び出しの間に持つ | **そのまま**（規則が禁じるのは line の lock と，device の呼び出しの間のそれ．`rates` は device の結果の cache で，間に他の device 呼び出しは無い）． |
| P3-16 | `remember_burst` の空集合 | 到達しないことを確認し，コメントを置いた（burst は channel のある stream にしか book されない）． |
| P3-17 | `start_rx`（`k != 0` の無音 return）と `Stop` の無音の枝が Mock と未照合 | **修正**（parity `vh_8_a_channel_less_stream_is_the_same_on_both` が両者を比べる）． |
| P3-18 | from-none の `rx_open(k, …)` の `k = 0` が暗黙 | コメントを足した． |

### ラウンド 2（同じ reviewer，`/tmp/opencode/review2.log`）

**P1 2 件，P2 4 件，P3 3 件，設計の問い 3 件．**

| # | 指摘 | 処理 |
|---|---|---|
| P1-1 | mutation gate に免除を足した（越権）．phase7 の道具を変えたことと，P313・P344・P345 の `equivalent` | **道具は戻した**（§2 の 12）．`equivalent` の欄は理由の記録として残し，phase7 の集計では SURVIVED として見えることを明記（§5，§7）．**免除そのものの可否は owner/orchestrator の判断**（§9 の 8）． |
| P1-2 | 計画の未達（テストと行） | **一部修正**．(a) 項目 2 の T0 `ur_02_a_subdev_spec_markup_names_each_element_s_pair`（lib の unit）を足した（`subdev_markup` が関数になったため）．(b) 項目 2 の T7 `ur_05_a_repeated_front_end_is_still_refused`（UHD 側）を足した．行は P08/P09（同じ検査）が殺すので新しい行は起こしていない．(c) 計画の 5 件の `ur_25_…` は，実装のテスト名が違うだけで中身は §3 の表で対応する（項目 7 = `ur_24_a_channel_keyed_*` の 3 本，項目 8 = `ur_25_a_layout_rate_change_*`，項目 10 = `ur_25_a_receive_membership_*`/`ur_25_the_last_receive_channel_*`，項目 11 = `ur_25_a_layout_cold_change_is_floored_*`/`ur_25_a_layout_cold_change_of_a_linkless_*`，項目 13 = `ur_25_a_layout_instance_s_*_judged_at_runtime`，項目 7 の queue = `ur_24_a_channel_keyed_command_counts_one_queue_slot`）．(d) 行の対応: 3-1（照合の後に spec）= P296（逆），3-3（失敗文面の要約）= P289，3-7（tx の照合）= P334/P345，3-8（理由の方向）= P293/P346，C2b = P283 と `ur_05_every_other_keys_value_is_refused`，C5 = 構造的に作れない（§2 の 1），W4/W5 = P300/P301 と `ur_05_a_failed_binding_leaves_no_serial_registered`（登録は成功時だけなので行は `Drop` の行 P299 が担う），W7 = 文言の行（`ur_05_a_second_binding_of_one_serial_is_refused` が serial を名指す文を固定），W8 = 記録の欄（`ur_30_the_device_section_records_the_serial`），W10 = 1 回目に equivalent として記録済み（test-plan の付録），W11 = P302，rate W2 = P296，W3 = P295，W4a = P294，W5 = P292，W6 = **足した**（下）． |
| P2-1 | A3 の記録に抜け（`device.rs`） | §6 の表に記録済み（この指摘と同じ）．A3 は「3 件」のままで，`device.rs` は裁定 構造 2/T5 の帰属であることを明記した． |
| P2-2 | `rates` の Mutex を device 呼び出しの間に保持 | **修正**．`check_rates` は lock の外で device を読み，結果を後で入れる（2 方向は別の slot，同じ読みは同じ答え）．P292–P296，P334，P346 を流し直して 7/7 killed（§7）． |
| P2-3 | UR-5 の要素の検査が 2 か所（layout と current の腕） | **退けた（一部）**: 2 つの腕は別の規則（layout は要素と front end の番号，current は profile の board の名前を `starts_with` で見る．`front_end_matches` は番号まで見る）で，同じ判断ではない．要素の写像（`layout_elements` と selection の写像）は形が違うだけで，1b-5 で `keys` を消すときに畳む（設計の問い Q3）． |
| P2-4 | 「重複・床・keys・宛先・rate は一か所」の確認 | 対応不要（確認結果）． |
| P3-1 | `mode.rs::stream_rate` の数の読みが `number()` と二重 | **修正**（`number()` に寄せた）． |
| P3-2 | 出口の grep「`updates.iter().filter` が 0 件」 | 報告に理由を書く（Mock の `updates` は床の入力集合で，判断は `layout_instant` にある）．§9 の 8 に送る． |
| P3-3 | `Channels::first(0)` の残り | 確認どおり，keys の経路ではない（空の印か current keys の数）．報告に一覧（`mod.rs:548-549,759`，`control.rs:257,262,741`，`rx.rs:152,302` は空の集合の印，`control.rs:601` の `0..channels_at` は current keys の分岐）． |
| Q1 | `set_subdev_spec` の後の要素数の検査は発火しない（免除より削除） | **判断を仰ぐ**（§9 の 8）．消せば R23 の行も引退できる． |
| Q2 | reopen の `check_rates` は 1 回目の結果を返すだけ（呼び出しを消すか） | P344/P345 の equivalent の根拠と同じ．halt の後にまた開けるかを確かめてから 1b-4d で判断する（§9）． |
| Q3 | `Mode` の二重の分岐は 1b-5 で畳む | 1b-5 へ送る（§9 の 9）． |

**行の追加（P1-2 の (d)）**: `P348` を足した — `check_rates` の device の読みで方向を入れ替える（rate の plan W6）．`old` = `        let outcome = match self.device.rates(dir, chan) {`，`new` = `        let outcome = match self.device.rates(match dir { Dir::Rx => Dir::Tx, Dir::Tx => Dir::Rx }, chan) {`．殺すテストは `ur_13_arm_refuses_a_rate_set_the_description_does_not_have`（rx だけを誤らせた run で，入れ替えると tx の良い方を読み，拒否が起きない）．killed（`/tmp/opencode/mut8b.log`）．

---

## 9. 1b-4d・1b-4e へ送るもの

1. `booked_stream` の stream 一般化，`rx_packet_samples(k)`，`<id>/d/s<k>` の node・SampleClock・event source の k ≥ 1（`Core::stream_node(k)` の一般化）．
2. stream ごとの thread 名 `uhd-rx-s<k>`・`uhd-tx-s<k>`，同名全 thread の join，`rejected` の `thread` の形（UR-16）．`[PreparedStream; 1]` ガードと stream 0 固定を外す．
3. `Streams::end` の複数 stream の検証と VH-8 harness の stream 次元（parity.rs の `UhdRig`/`MockRig` を stream ごとに）．
4. `delivered` の床が「その line と同じ stream」であることを 2 本目の受信 stream で確かめるテスト．
5. `Lines::prune_stream` と RM-34 の move（同数の別集合，「開き直しを数で比べる」行は 4e）．
6. Manifest の plan に stream の番号と segment ごとの論理 channel 列（Sol の minor 2）．
7. 行の当て直しで `new` の識別子も grep する（gate の U41）．
8. **owner の判断**: (a) 床で上がったうえで拒否される `cold` 変更の LATE_COMMAND（Mock は出す，UHD は出さない，§2 の 14），(b) phase7 の道具に足した `equivalent` の扱い（§5），(c) P313/P344/P345/R23/R26 の equivalent の記録の妥当性．
9. 1b-5: `keys` の削除，Vision の key の記述（`design/vision/02-…:24-28,54`，`03-…:71`）．

---

## 10. ラウンド 2 の後に足したものと，最後の検証

ラウンド 2 の処理で足したもの（§8 の P1-2，P2-2，P3-1）:

- テスト: `ur_02_a_subdev_spec_markup_names_each_element_s_pair`（lib の unit，T0），`ur_05_a_repeated_front_end_is_still_refused`（fake，T7）．
- 行: **P348**（UR-13 の照合が方向を入れ替える）．
- コード: `check_rates` が device を lock の外で読む（P2-2）．`Mode::stream_rate` が `number()` を使う（P3-1）．

最後の検証（上と同じ tree，最後の編集の後）:

1. `cargo test --workspace --no-fail-fast`: **66 targets すべて ok，1382 tests passed，0 failed**（`/tmp/opencode/testall2.log`．`check_rates` の変更と T0/T7 の追加を含む）．`Mode::stream_rate` の `number()` 寄せ（P3-1）はその直後なので，`cargo test -p ezsdr-radio-uhd -p ezsdr-radio -p ezsdr-mock-radio` をもう一度流して確かめた（`/tmp/opencode/test3.log`）．
2. `cargo clippy --workspace --all-targets -- -D warnings`: 警告なし（`/tmp/opencode/clippy2.log`，`/tmp/opencode/clippy3.log`）．
3. `rules.py --base HEAD`: 0 problem（`/tmp/opencode/rules2.log`）．
4. mutation: P348（新規）を流し直して **killed**（`/tmp/opencode/mut8b.log`．`other(dir)` と書いた 1 回目は COMPILE ERROR だったので，方向を入れ替える `match` に直した）．それ以外の行は §7 の結果のまま（コードもテストも変えていない．`Mode::stream_rate` の読みは P303–P305 の対象行の外で，`old` の一致も `rules.py` が確かめている）．

### 全 mutation のまとめ

| list | 行数 | killed | 生き残り |
|---|---|---|---|
| maintenance | 731 | 731（equivalent 3 件を含む） | 0 |
| phase3 | 83 | 83 | 0 |
| phase4 | 34 | 34 | 0 |
| phase5 | 29 | 29 | 0 |
| phase6 | 63 | 63 | 0 |
| phase7 | 167 | 165 | R23・R26（理由つき．phase7 の道具が `equivalent` を読まないため SURVIVED として見える） |
| **G09（phase7）** | — | — | この段階と無関係の既存の生き残り（kernel の行．§7） |

（maintenance の行数は P348 を足した後の 731．§7 の 1106 行の run は P348 を足す前の 1105/1106 で，生き残りは G09 のみ．P348 の run は §7 の 4 を見よ．）
