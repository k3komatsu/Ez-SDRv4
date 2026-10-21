# 1b-5c-1：Sol 6.1 のコードレビュー

## 1. コードレビュー

判定：FAIL

本番の layout 分岐を畳んだ処理に、明確な動作退行は見つかりませんでした。ただし、テスト専用の設定適用経路と、到達可能な系列の検証削除があるため、このままの受理は勧めません。

### 指摘

1. **major：設定適用がテスト時だけ別実装になっている**
   - **場所：** `crates/ezsdr-radio-uhd/src/provider/control.rs:129–136`、同 `:2115–2127`、`plan/maintenance/tools/mutations.json:8360`
   - **根拠：** `settings_on` は通常ビルドでは必ず description の番号付き設定を読みますが、テストでは `test_base` を優先します。また、新しい `settings` は本番から呼ばれない独立した投影計算です。P279 はこのテスト専用計算を変異させ、その計算だけを呼ぶテストで kill しています。本番の `settings_on` は変異しません。
   - **系列・影響：** channel ごとの設定読出しや origin 境界での投影に誤りが入っても、`ColdConfig::new` を使う owner テストや P279 の kill では保証できません。
   - **修正：** テストも番号付き設定 map を構築して同じ `settings_on` を通す形にし、独立した `settings` 計算を削除してください。P279 は実際の設定適用経路へ当て、device の適用値で検出してください。

2. **major：TX の未開始 segment を再 open する系列が失われている**
   - **場所：** `crates/ezsdr-radio-uhd/src/provider/tx.rs:1232`、同 `:2134–2164`、`plan/maintenance/tools/mutations.json:6894`
   - **根拠：** 削除された以下の2本に、同じ TX 系列の代替がありません。
     - `ur_13_a_transmit_from_none_segment_the_owner_never_started_is_opened_again`
     - `ur_25_a_transmit_owner_waiting_behind_a_from_none_open_starts_nothing`
     
     残った poll テストは最初の open 中の禁止を確認しますが、解放後に古い B の集合を再 open して採用するところまで進みません。RX テストは TX の代替ではありません。
   - **系列・影響：** control が B＝`{0,1}` を開き、owner が未開始の間に空集合を経て C＝`{0}` を開く。その後 TX owner が B を採用する系列は layout でも表現できます。C の streamer を B と誤認する退行の直接検証が消えています。
   - **修正：** 2本を membership key に移してください。P370 も TX テストへ戻してください。P369 と P370 は source/mutant は同じですが、filter が RX/TX で異なり、報告にある「同じ filter の完全重複」ではありません。

3. **major：control differential が2 channelへの変更を no-op に縮めている**
   - **場所：** `crates/ezsdr-radio-uhd/src/provider/control/differential.rs:128–132`、同 `:298`
   - **根拠：** generator は `Change::Channels(0|1|2)` を生成しますが、移行後は 1 と 2 をともに `c0.stream = 0` に変換します。期待側も同じ変換を行い、membership が同じなら `continue` します。
   - **系列・影響：** `{0}` → `{0,1}`、`{0,1}` → `{0}` が、実行側・期待側とも no-op になります。1,000系列が通っても、control の複数 channel membership 変更は以前と同じ範囲では検証されません。
   - **修正：** count の変更を `c0`・`c1` の membership 変更へ展開し、期待側にも対応する変更を持たせてください。単に正数を1へ丸めるのは代替になりません。

4. **minor：UR-9 に削除対象の description 型が残っている**
   - **場所：** `design/18-uhd-radio.md:180`
   - **根拠・影響：** 冒頭が依然として「`ezsdr_radio::x3xx` の `DeviceDescription`」です。一方、同じ規則の後段と実装は常に `channels::Description` です。規範文が矛盾しています。
   - **修正：** 冒頭も `channels::Description` に統一してください。今回変更した規範文に残る rationale・調査説明も、指定どおり Changes／判断記録へ移してください。

### 削除13テストの照合

| 削除テスト | 残る検証／判断 |
|---|---|
| `ur_05_the_current_keys_set_uhd_s_default_order` | `ur_05_the_layout_builds_the_subdev_spec`、`ur_05_the_layout_s_order_is_the_device_s_channel_order` |
| `ur_12_each_key_set_refuses_the_other_in_prepare` | `ur_12_prepare_refusals` の未知key拒否・apply不変。二モード相互拒否そのものは撤去対象 |
| `ur_13_a_transmit_from_none_segment_the_owner_never_started_is_opened_again` | **不足：指摘2** |
| `ur_15_the_current_keys_name_their_one_stream_s_threads` | `ur_15_every_declared_stream_starts_its_own_thread` |
| `ur_24_a_halted_direction_fills_no_queue_slot` | 旧会計は撤去対象。新会計は `ur_24_a_release_without_a_live_channel_takes_the_command_queue_slots` |
| `ur_24_the_current_keys_held_updates_reach_every_channel` | 方向全体への fan-out は撤去対象。`ur_24_a_timed_update_for_a_channel_out_of_its_stream_waits_for_its_segment` 等が個別channelを検証 |
| `ur_24_the_queue_counts_a_command_per_channel` | `ur_24_a_channel_keyed_command_counts_one_queue_slot` |
| `ur_25_a_transmit_owner_waiting_behind_a_from_none_open_starts_nothing` | busy中の一部は poll テストに残るが、**後続再openは不足：指摘2** |
| `ur_25_the_current_keys_floor_a_cold_change_only_at_its_own_stream` | 旧floorは撤去対象。`ur_25_a_layout_cold_change_is_floored_at_every_earlier_cold_change_of_the_instance` |
| `ur_26_a_current_keys_antenna_update_is_refused` | `ur_26_an_unnumbered_key_is_refused_at_runtime` |
| `ur_29_a_partly_applied_release_keeps_the_projection` | 1 command＝1 channelとなり部分fan-outは消滅。`ur_29_a_failed_released_update_is_a_device_failure`、`ur_24_a_refused_release_records_no_row_and_keeps_the_projection` |
| `ur_29_a_lost_device_between_a_release_s_channels_applies_no_more` | 複数channel releaseは消滅。`ur_29_a_lost_timed_release_publishes_device_loss` |
| `ur_29_a_loss_published_between_a_release_s_channels_stops_the_rest` | `ur_29_a_loss_published_before_a_release_stops_the_call`、`ur_29_a_loss_after_a_release_s_command_stops_the_one_behind_it` |

### 確認結果

- UHD unit **235本**、fake integration **265本**が成功。
- differential は双方 **1,000系列**成功。control の全class、RX の拒否38件・prune18件などは非ゼロ。既知の timed 比較除外は25系列。ただし指摘3の縮退があります。
- 残存 mutation 行は変更した2リストですべて source に1回一致。指定の削除行は **P370以外**すべて旧 source が消失。R21 は17件目を受理する非等価変異です。
- spec の `*Test:*` ブロック内434参照に、存在しないテスト名はありません。
- hardware 差分は key・stream path の移行で、既存のassert削除はありません。指定外のradio／MockRadio／server／Pythonにも変更はありません。

未実施：実機、UHD feature build、mutation実行、workspace全体の再実行。本番経路のmutation検出力と実機動作は未検証です。ファイルは変更していません。

## 2. 修正の回 1 の再レビュー

判定：PASS

前回の指摘1〜4は修正済みです。今回の修正と隣接系列に、新たな major／minor の指摘はありません。

### 確認内容

1. **TXのfrom-none系列と元のassertを復元**
   - `crates/ezsdr-radio-uhd/src/provider/tx.rs:1153`、`:2180`
   - controlが B＝`{0,1}` → 空 → C＝`{0}` を予約し、B／Cをfrom-noneでopenする系列になっています。元のopen順、clock採用、busy中のcursor・switch・idle mark・device callのassertは維持されています。
   - 追加の `tx.step()` は妥当です。同時刻の個別leaveで `{1}` の中間segmentができ、後続leaveによりゼロ長になります。timelineは送信のゼロ長segmentを保持し（`crates/ezsdr-radio/src/timeline.rs:962`）、TX ownerは採用せずスキップします（`tx.rs:731`）。誤ったplanを隠してはいません。

2. **control differentialの2-channel遷移とcold floorを維持**
   - `crates/ezsdr-radio-uhd/src/provider/control/differential.rs:111`、`:310`
   - 両側ともc0／c1の所属変更を扱います。no-op count変更も、期待側では `layout_schedule` がcold floorに算入し（`:97`）、実行側ではno-op key更新を予約します（`:328`）。segmentを増やさずfloorを残す点で一致しています。
   - 1,000 seedを再実行し成功。2-channel化 **495件**、2-channelからの縮小 **172件**で、双方の非ゼロassertもあります。

3. **ColdConfigの本番・テスト経路を統一**
   - `crates/ezsdr-radio-uhd/src/provider/control.rs:63–121`
   - test-only fieldと別投影計算は削除済みです。テスト用constructorは残りますが、設定適用は共通の `settings_on` → `configure` を通ります。
   - P279は本番のorigin境界を変異させます。`:2070–2095` のテストは実際にconfigureし、device read-backのgain／frequencyで検出します。

4. **規範文とmutation行**
   - `design/18-uhd-radio.md:180` のUR-9は `channels::Description` に統一済みです。
   - J69／P279／P355／P370／P465／U20／R21は、変異内容と検出テストの対応を維持しています。P370は復元したTXテストを指します。
   - 変更対象2リストの全 **1,053行**で、`old` がsourceにちょうど1回一致しました。既存の実行ログでも指定7行の **7/7 killed** を確認しました。mutationは再実行していません。

### 独立実行結果

- UHD unit：**237 passed**
- fake integration：**265 passed**
- control differential：**1,000 seed成功**
- `git diff --check HEAD`：問題なし

未実施：実機、UHD feature build、workspace全体、mutation再実行。実機での成立は本レビューでは未検証（INFERRED）です。ファイル変更はしていません。
