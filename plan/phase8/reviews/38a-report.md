STATUS: done

# 設計メモ 38 段階 38a：実装の報告（#62）

HEAD `7a3b553` の tree に未 commit の変更を置いた（commit・push・add・stash はしていない）．`impl-brief.md` §1 の S1〜S9・F2 をすべて実装した．

## 1. やったこと（ファイルごと）

### 語彙 `ezsdr-radio`
- `src/lib.rs`：`TimingEnvelope.release_window_ns`，`keys::RELEASE_WINDOW_NS`，`radio_keys` の宣言（key 33 個）．
- `src/x3xx.rs`：`x3xx::timing` に 3 ms．
- `src/channels.rs`：ideal は 0，`capabilities` に key．
- `src/timeline.rs`：`timed_instant(at, now, lead, window, earlier) -> (e, late)`（`layout_instant` の隣）．floor = max(`now+lead`, `e <= now+window` の earlier の最大)，`e = max(at.unwrap_or(floor), floor)`，`late = at < floor`．
- `schemas/radio/envelope.v1.json`：再生成（+6 行）．version は上げず，`SCHEMA_CHANGELOG` は書いていない（AGENTS.md §6）．
- `tests/timeline.rs`：`TimingEnvelope` の literal と，`rm_25_the_timed_instant_is_floored_at_every_booked_command_inside_the_window`（11 行：S1 の訂正 4 行，窓の外の行，window 0，過去の at，窓の端ちょうど）。
- `tests/radio_model.rs`：RM-4 の key 行，key 数 33，schema freeze．
- `tests/generator/mod.rs`：`Terms.release_window` と `Sequence::schedule` の `timed_instant` 化（O11）．

### MockRadio
- `src/lib.rs`：`hardware_timed` の floor を `timeline::timed_instant` に（lead は profile の `min_timed_command_lead_ns`，window は `release_window_ns`），late → 満杯の順，深さは「予約の履歴のうち instant が receipt より後」．停止・喪失の記録は既に S 基準だったので変更なし．
- `tests/mock_radio.rs`：下の表の 18 本＋`mr_03` の release window 期待値，`mr_18_hardware_timed_updates` の 17 件目を `at=9 ms` に．
- `tests/timeline.rs`：layer 1 の `layout_schedule` に floor，`Terms.release_window`（O12）．

### UHD Module
- `src/provider/core.rs`：`Streams::lost_at` と `Streams::ended_at()`（stop と loss の S を 1 か所で，loss が同時刻なら loss）．
- `src/provider/control.rs`：
  - `book_timed`：floor を `timed_instant`，深さを `booked`（`e > now`，拒否された release も `e` まで），late → 満杯．
  - `release` / `of_record` / `record_given`：行は instant に書く（`released_at` の fold）．`no channel` は `issued` なしの行，後の segment へ渡したものは `issued: true` + note．fold は streams の lock の下で loss を再確認（S3 の競合）．
  - `give`：行を書かない．`Handed::{Given, Refused, Lost}`；拒否は予約から外して行なし，**apply 自体が lost を返した場合も `Lost`** として `held` に残す（S3）．
  - `settle(at, why, lost)`：held と given を (e, seq) 順に確定．stop は `e <= S`，loss は `e < S`；due で channel 無しは `no channel`，live は Ceiling として cancelled，due でないものは cancelled．`cancelled_at = S`．
  - `finish`：S を `streams.ended_at()` から 1 度だけ取り，無ければ `expect`（本番の呼び出し元は stop 時だけであることを grep で確認した．harness は loss 後に呼ぶ）．`cancel_held` は test 専用の `settle` 呼び出しに．
  - `RELEASE_WINDOW_NS` の import を削除（`profile.rs` の定数も削除）．release の条件（いつ・どの channel か・channel 待ちで止まる）は変えていない．`released` / `released_at` は残した．
  - 新しい unit test 26 本と，下の「既存テストの書き換え」．
- `src/provider/control/differential.rs`：`Record` に `late`（`LATE_COMMAND` の payload）と `applied`（`(key, claimed, at, cancelled)`），`timed` を全系列で比較，`terms()` に window，`layout_schedule` を `timed_instant` に，`expect` が late/applied を作り，`run` が loss 後に `finish()` を呼び，`releases()` と `ceilings()` を**系列だけから**決める（S2）．`release_window_matters` は削除．1 000 系列に加えて 9d 形の明示系列を同じ test で回し，Ceiling の除外と先頭の比較を exercize する．分類そのものは `rm_26_the_ceiling_is_decided_from_the_sequence`．
- `src/provider/rx/differential.rs`：`Terms.release_window`（0）．
- `tests/fake.rs`：`ur_09_...` に `release_window_ns` の capability と envelope．

### spec（DA-1 の形，各 Changes に 1 行）
- `design/07-radio-model.md`：RM-4（key），RM-6（7 個．release window），RM-16（S の確定と Ceiling），RM-20（7 members），RM-21（floor・深さ・late → 満杯．`timed_instant`）．RM-29 の「six」→「seven」．
- `design/09-mock-radio.md`：MR-3 の x3xx/ideal の表，MR-18（floor・window・late → 満杯，`timed_instant`），MR-25（S 基準と同時刻の順，Ceiling の 1 文），Test 行．
- `design/18-uhd-radio.md`：§3 の定数から release window を外し profile の値に，UR-9 の note（予約の履歴），UR-24（floor，深さ，行は instant，lost で `held` に残す），UR-26（S の確定と Ceiling），UR-30（行の説明），U13，VH-8 の `rm_26_the_timeline_against_uhd_control` 行，Test 行．

## 2. 計画から変えた所とその理由

- **`at=None` の fallback**：`at.unwrap_or(floor)` のまま．`unwrap_or(now + lead)` との差は `e = max(asked, floor)` に吸収され（lead <= floor），同値の変異にしかならない（S9 の M16 と同じ扱い）．なのでその mutation 行（P597）は足さず，1b の試験は**結果**（B が floor の instant）を固定する．
- **11b/11c の `e`**：lead が 2 ms なので `e = 1 ms` は作れない（1 ms の at は 2 ms に上がる）．両 Module とも `at = 2…17 ms`（有効 e も 2…17 ms）にそろえた（review 3 の 3）．受取り（20 ms，2 ms，その 1 tick 前）は指示書どおり．
- **stop/loss の同時刻の mutation**：確定は held と given の両方を回すので，9c の given の e == S を殺す行は **given 側**の `due` を変える（P583 loss，P584 stop）．held 側は P593（`due = false`）が 9d で殺す．
- **`release` の loop 先頭の `is_lost` 確認を削除**：`give` の確認と fold の（streams lock 下の）確認が同じことをし，loop 先頭の行を殺す P435 は同値になったため行ごと削除した．fold の確認は review 2 の P1 の競合を閉じる（P596 が killed）．
- **`cancel_held`**：`finish` からは `settle` を呼び，`cancel_held(why, at)` は test 専用（`#[cfg(test)]`）の wrapper にした．
- **harness の Ceiling**：1 000 系列では Ceiling が 0 行だったので，`rm_26_the_timeline_against_uhd_control` の中に 9d 形の明示系列を足して除外の経路を通した（P594 がその test で killed）．`ceilings()` の分類は `rm_26_the_ceiling_is_decided_from_the_sequence` が固定する（P591・P592 が killed）．
- **Mock の 11c の「step(e) を先に済ませた」場合**：時刻を戻せないので，`step(2 ms)` を先に済ませた harness と済ませない harness の両方で receipt = 2 ms の結果が同じ（予約できる）ことを見る形にした．

## 3. 系列ごとのテスト名

| 系列 | MockRadio | UHD Module |
|---|---|---|
| 1 | `mr_18_a_timed_update_behind_one_inside_the_window_is_late` | `ur_24_a_booked_command_floors_a_later_one_inside_the_window` |
| 1（ideal） | `mr_18_the_ideal_profile_has_no_release_window` | —（window は description の試験） |
| 1b | `mr_18_an_omitted_deadline_takes_the_floor` | `ur_24_an_omitted_deadline_takes_the_floor` |
| 2 | `mr_18_the_window_edge_excludes_one_tick` | `ur_24_the_window_edge_excludes_one_tick` |
| 3 | `mr_18_a_timed_update_beyond_the_window_keeps_its_instant` | `ur_24_a_command_beyond_the_window_keeps_its_instant` |
| 4 | `mr_18_a_timed_update_behind_one_waiting_for_a_channel_is_late` | `ur_24_a_command_waiting_for_a_channel_floors_a_later_booking` |
| 5 | — | `ur_24_a_refused_release_floors_the_next_booking` |
| 5b | — | `ur_24_a_refused_release_holds_its_queue_slot_until_its_instant` |
| 6 | `mr_18_a_channel_leaving_after_the_booking_does_not_move_the_floor` | `ur_24_a_channel_leaving_after_the_release_does_not_move_the_floor` |
| 7 | `mr_18_a_stop_leaves_each_update_its_row_at_its_instant` | `ur_26_a_stop_leaves_each_update_its_row_at_its_instant` |
| 8 | `mr_25_a_stop_records_a_later_update_as_cancelled_at_the_stop`，`mr_20_a_loss_records_a_later_update_as_cancelled_at_the_loss` | `ur_26_provider_stop_records_a_given_and_a_held_update_alike` |
| 8b | — | `ur_24_a_command_the_loss_stops_from_being_given_settles_at_the_end`（e > S，e = S，e < S の 3 つ） |
| 9 | `mr_18_an_update_applied_before_a_stop_keeps_its_row` | `ur_26_an_applied_update_keeps_its_row_after_a_later_stop` |
| 9b | `mr_18_a_loss_after_the_last_pass_keeps_a_row_due_before_it` | `ur_26_a_loss_settles_what_the_last_release_gave`，`ur_26_a_stop_settles_by_its_instant_not_by_the_finish` |
| 9c | `mr_18_an_update_due_at_the_stop_instant_applies_and_at_the_loss_instant_does_not` | `ur_26_an_instant_at_the_stop_applies_and_one_at_the_loss_does_not` |
| 9d | — | `ur_24_a_due_command_behind_a_channel_less_head_is_cancelled_at_the_loss` |
| 10 | `mr_18_two_updates_at_one_instant_keep_the_booking_order` | `ur_24_two_updates_at_one_instant_write_their_rows_in_booking_order` |
| 11 | `mr_18_a_late_command_at_a_full_queue_reports_late_first` | `ur_24_a_late_command_at_a_full_queue_reports_late_first` |
| 11b | `mr_18_a_command_due_before_the_receipt_takes_no_slot` | `ur_24_a_command_due_before_the_receipt_takes_no_slot` |
| 11c | `mr_18_a_command_due_at_its_receipt_takes_no_slot` | `ur_24_a_command_due_at_its_receipt_takes_no_slot` |
| D2 (3) | `mr_18_the_row_is_written_at_its_instant` | `ur_24_the_row_is_written_at_its_instant` |
| D2 (1) cold は床に入らない | `mr_18_a_cold_change_does_not_floor_a_timed_update` | （語彙関数は 1/4 が固定） |
| S5（window は description） | — | `ur_24_the_release_window_comes_from_the_description`（床と release の両側） |
| S6（loss→stop，stop→loss，再通知） | `mr_25_a_loss_then_a_stop_keeps_the_loss_s_instant_and_its_rows`，`mr_25_a_stop_then_a_loss_keeps_the_stop_s_instant_and_its_rows` | `ur_26_a_loss_then_a_stop_keeps_the_loss_s_instant_and_rows`，`ur_26_a_stop_then_a_loss_keeps_the_stop_s_instant_and_rows` |
| S3 競合（release 中の loss） | — | `ur_29_a_loss_published_during_a_release_settles_the_channel_less_behind_it` |
| lost の apply | — | `ur_29_a_lost_timed_release_publishes_device_loss` に `finish` 後の行を追加 |
| 12 | `rm_26_the_timeline_against_mockradio`（layer 1，1 000 系列） | `rm_26_the_timeline_against_uhd_control`（1 000 系列＋9d 形 1 系列），`rm_26_the_ceiling_is_decided_from_the_sequence` |
| 語彙 | `rm_25_the_timed_instant_is_floored_at_every_booked_command_inside_the_window` | |

新しい test 関数は 48 本（UHD 25，Mock 21，語彙 1，harness の分類 1）．

## 4. 既存テストの書き換え

- **行を instant の後へ移した 14 本**（§3.1 の表どおり）：`ur_24_a_timed_update_reaches_a_later_stream_s_pending_segment`，`ur_24_a_release_without_a_live_channel_reaches_the_pending_segment`，`ur_24_a_command_waiting_for_a_channel_is_given_before_a_later_one`，`ur_24_a_command_waiting_for_a_channel_holds_back_the_one_behind_it`，`ur_14_booking_does_not_wait_for_a_future_cold_switch`，`ur_24_a_direction_halted_by_a_refusal_has_no_channel`，`ur_25_a_timed_update_at_the_cold_change_s_instant_reaches_the_switch`，`ur_24_a_release_behind_a_released_command_keeps_the_device_queue_order`，`ur_24_a_segment_booked_while_a_command_waits_for_a_channel_holds_it_at_its_instant`，`ur_25_an_enable_at_a_timed_update_s_instant_applies_it`，`ur_24_a_channel_keyed_timed_update_acts_on_its_own_channel`，`ur_24_the_layout_destination_follows_the_effective_membership`，`ur_34_a_timed_update_crossing_the_move_reaches_each_streamer_channel_at_its_instant`，`ur_24_a_command_waiting_for_a_channel_is_booked_at_the_instant_it_can_take_effect`．
- **場面を変えた 3 本**：`ur_24_a_command_waiting_for_a_channel_keeps_a_later_booking_behind_it`（A を 101 ms，cold を 100.8 ms に．遅れの試験は系列 4 が持つ），`ur_29_a_loss_after_a_release_s_command_stops_the_one_behind_it`（`finish` 後に A issued・B `no channel`），`mr_18_hardware_timed_updates` の full の 17 件目を `at=9 ms` に．
- `ur_30_the_recorded_plan_has_no_refused_segment`：試験の側で `streams.stop` を明示的に置いた（S8）．
- D1 の期待値：`mr_03`（capability と envelope），`ur_09`（capability と envelope），`rm_04`（key の行と数），`rm_01`（key 数）．
- `mr_25_a_*`（`mr_25_a_stream_stop_keeps_the_pending_commands` など）と `fake.rs` の `ur_24_a_released_command_is_not_recalled_by_stop`，`ur_24_stop_cancels_held_commands` は変更なしで通る（後者は `Provider::stop` が 1 ms を過ぎてからで，行の時点は e のまま）．
- `ur_29_a_lost_timed_release_publishes_device_loss`：apply が lost を返す場合も予約を残すようになったので，`finish` 後の cancelled 行を足した（review 3 の 1）．

## 5. mutation の行

新規 25 行（`plan/maintenance/tools/mutations.json`）：P573（窓の端 `<=`），P574（window 無視），P575（lead 無視），P576（UHD の深さが instant を見ない），P577（Mock の深さ），P578（Mock の 3 ms 定数），P579（UHD の floor の 3 ms 定数），P580（give で行を書く），P581（fold が e == now を外す），P582（S を finish の now に），P583（loss が e == S を確定），P584（stop が e == S を確定しない），P585（確定の順を逆に），P586（given を忘れる），P587（lost で止まった command を捨てる），P588（channel 無しの先頭を cancelled に），P589（Mock が予約時に行を書く），P590（Mock が late を出さない），P591（Ceiling が先頭を除外），P592（Ceiling が release の有無を見ない），P593（held の due を false に），P594（P588 を harness でも），P595（UHD の release の 3 ms 定数），P596（release 中の loss を無視），P598（lost の apply を拒否扱い）．P577 は Mock の 11b/11c 用．

当て直した既存 16 行：J47，J58（`self.now()` → `at`），P148（cold が timed を床付けする），P268（window 無視），P414（issued を消す），P416（no channel に issued を足す），P419（拒否を成功扱い），P422（given だけ数える），P426（floor を使わない），P427（late を出さない），P432（no channel を of-record にしない），P433（fold 逆順），P434（give の loss 確認），U21（lead 0），U38（stop が確定しない），C02（issued false）．削除：P435（loop 先頭の loss 確認が `give`・fold と重複して同値になった），P597（`at=None` の同値変異．P598 は別）．

結果（`remote_mutate.py maintenance,phase7 sim02:2`，40 行）：**40/40 killed**（J58 と P424 は記録済みの equivalent として survived が正しい）．他に P595，P596，P598，P148/P594，P573/P583/P591/P581/P584/P593，P419 を個別に流して killed を確認した．各 `old` は tree に 1 回だけ一致（`rules.py` と，reviewer の全 1,316 行チェック）．

## 6. 確かめたこと

- 新しい test と直した test を **各 20 回**，1 binary ずつ，`timeout 300`，`--test-threads` 既定で流して全部通った（48 本の新規＋書き換えた 20 本＋`rm_26_the_timeline_against_uhd_control`・`rm_26_the_timeline_against_mockradio`・`rm_04`・`rm_01`・`mr_03`・`ur_09`・schema freeze）．
- `cargo test --workspace`：全部通る（最後に 1 回，変更のたびに再実行）．
- `cargo clippy --workspace --all-targets -- -D warnings`：警告なし．
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`：0 problems．
- `git diff --check`：なし．
- `rm_20_schema_freeze`：通る（`envelope.v1.json` を再生成）．
- `RELEASE_WINDOW_NS` は tree に残っていない（`profile.rs` から削除．設計メモの歴史記述のみ）．
- 負荷：busy loop，`--test-threads` の変更，並列 cargo なし．`TMPDIR` は変えていない．

## 7. セルフレビュー（reviewer: `openai/gpt-6-luna#max`，3 回）

- **1 回目**（P1 1・P2 2・P3 1）：
  1. S6 の stop→loss で，Mock が stop 時に fault を解決済みにすることを確かめておらず，戻り値も捨てていた／UHD は理由を見ていなかった → **直した**（fault 行 `applied:false` と step の Ok を assert，UHD は `cancelled` の理由と再通知の event 数を assert）．
  2. release 側の定数化 mutation が無い → **直した**（P595）．
  3. RM-29 の「six」→ **直した**（seven）．
  4. MR の Test 行に一部の test が無い → **直した**（MR-18，MR-25）．
- **2 回目**（P1 1・P2 3・P3 1）：
  1. release が入口でだけ loss を見るため，release 中に公表された S == e の loss を行が追い越せる → **直した**（fold を streams の lock の下で行い，lock 取得後に loss を再確認．`ur_29_a_loss_published_during_a_release_settles_the_channel_less_behind_it` と P596）．
  2. Mock の `at=None`＋先行予約の統合 test が無い → **直した**（`mr_18_an_omitted_deadline_takes_the_floor`）．
  3. Ceiling が spec で曖昧 → **直した**（RM-16 と UR-26 に 1 文）．
  4. UHD の loss 再通知 test が無い → **直した**（`ur_26_a_loss_then_a_stop_...` に 2 回目の `device_lost`）．
  5. RM-16/UR-9 の Test 行 → **直した**．
- **3 回目**（P1 2・P2 1・P3 1）：
  1. apply 自体が lost を返すと予約が消える → **直した**（`Handed::Lost`，`ur_29_a_lost_timed_release_publishes_device_loss` の `finish` 行，P598）．
  2. 同じ tick の loss が stop の確定より後に worker から届く競合 → **退けた**．`Provider::stop` は S を記録して uhd-control を先に join し，その中で確定する（UR-26 の join 順）．worker が stop の確定後に同じ tick の loss を公表しても，確定をやり直す場所が無い．直すには「worker が止まるまで終端理由を保留する」か join 順の変更が要り，それは 38a の範囲（D2 (4) の確定を `finish` で行う）を超える lifecycle の変更なので，設計メモへの差し戻しとして orchestrator に渡す．**未解決として下に残す**．
  3. UHD の 11b の時刻が Mock と違う → **直した**（両方 `at = 2…17 ms`）．
  4. 一部の event を件数だけで見ていた → **直した**（5・5b・6・11・S5 で `LATE_COMMAND` の key/requested/applied と `COMMAND_QUEUE_FULL` の depth，L→Q の順）．
  5. MR-25 の marker が 1 行に混在 → **直した**（`*Producer obligation (MA-13).*` と別行の `*Test:*`）．

## 8. 残ったこと

- **stop と loss が同じ tick の lifecycle（review 3 の P1 #2，退けた）**：uhd-control の確定が先で，worker の loss が後なら stop の理由・S が残る．D2 (4) の「同じ時刻は loss が先」をこの順序でも守るには，終端理由の確定を worker の停止後まで遅らせる（または join 順を変える）設計判断が要る．orchestrator の判断を仰ぐ．
- **`at=None` の fallback は同値変異**（`unwrap_or(floor)` と `unwrap_or(now+lead)` は `max` に吸収される）．1b の試験は結果を固定するが，この式自体は mutation で区別できない．
- **harness の 1 000 系列には Ceiling が 0 行**（生成器の at と loss の位置の関係で珍しい）．9d 形の明示系列を harness の test に足して経路を通し，分類は専用 test が固定する．生成器そのものに Ceiling を強制する変更はしていない（Mock layer 1 と共有のため）．
- orchestrator の gate（全 mutation リスト，`mutate.py` の全行，負荷，docker，Python suite）は流していない．`sim02:2` だけで流した（`sim03` は別 job のため使っていない）．
- **release の同期 `apply` が `S` をまたぐときの受理時刻**（セルフレビュー 3 回目の P1，退けた）：`Device::apply` は受理の時刻を返さないので，uhd-control は pass の `now` でしか `S` と比べられない．受理時刻で判定するには装置の API か「`S` までに渡した」の定義を変える設計判断が要る（§9.5 の 3 回目）．

## 9. 直し 1 回目

`sol-code-verdict.md` の 3 件（major 2・minor 1）を `fix1-brief.md` §1 のとおりに直した．HEAD `7a3b553` の未 commit の変更の上に置いた（commit・push・add・stash はしていない）．

### 9.1 直したこと

1. **停止の公開の後の release が行を確定する**（`crates/ezsdr-radio-uhd/src/provider/control.rs:649-671`）: fold の前の確認を `self.core.is_lost()` から，streams の lock の下の `streams.ended_at()`（喪失も停止も）に広げた．終わりが予約されていれば fold をせずに戻り，行は `settle` が `S` で確定する（`e ≤ S` は issued，`e > S` は cancelled，`S` が now より後でも同じ）．release が装置へ渡す command（いつ・どの channel に渡すか），Action の `Stop`，`released`／`released_at` は変えていない．lock の変数名を `_streams` から `streams` にした（使うため）．
   - **セルフレビュー 2 回目の P1（release が `S` を過ぎてから渡した command）も直した**: 渡した後に fold で終わりを見たとき，その pass が `now > S` で走っていたなら，渡した command は `S` の時点で渡されていない（RM-16 の Ceiling）ので，`released_at` から `held` へ戻して `settle` に cancelled で確定させる（`given_here` に pass で渡した `(e, seq)` を集める）．この分岐が戻すのはその pass で渡したものだけなので，`S` より前の pass で渡した command の行は issued のまま．装置の呼び出しは変えていない（渡した後で記録だけが cancelled になる；D2 (4) の「渡したものも `S` より後なら同じに記録する」と同じ）．
   - 試験: `ur_26_a_release_after_the_stop_was_published_leaves_the_row_to_the_end`（`control.rs:5270`）．A（`e = 10 ms`）を 7 ms に渡し，9.9 ms に `stop_end`（run の loop の停止の確認を通った後の形），10.6 ms に `release()`: release の後の行は無く，`finish` の後に A は cancelled 1 行（`cancelled_at = 9.9 ms`），issued の行は無い．同じ形で S = 10.2 ms（A は `e ≤ S`，7 ms の pass で渡し済み）: `release()` の後の行は無く，`finish` の後に issued が 1 行だけ，cancelled は無い．`ur_26_a_command_given_after_the_stop_s_instant_is_cancelled_at_it`: A（`e = 10 ms`）を持ったまま 10.2 ms に `stop_end`，10.6 ms の `release()` が A を装置へ渡す（`timed_gains` で確認）→ 行は無く，`finish` の後に cancelled（`cancelled_at = 10.2 ms`，理由 `Provider::stop`），issued は無い．
   - mutation: P599（`now > at` を `false` に）が killed，P600（`ended_at` を損失だけに）が killed，P596（fold の確認ごと消す）は当て直して killed．P581（fold の範囲）も killed．
2. **harness の Ceiling が channel 無しの後続も外す**（`crates/ezsdr-radio-uhd/src/provider/control/differential.rs:297-334`）: 除外の条件に「その command 自身の channel が期待する layout の `e` に在る（`!channel_less`）」を足した．channel 無しの command は先頭でも後続でも外さない．明示系列を 3 command にした（RX の channel を外し，A・B を RX，C を live な TX へ，どれも `e = 10.4 ms`，最後の release 10 ms，喪失 10.9 ms）: `rm_26_the_timeline_against_uhd_control` は除外が 1 行（C）であることを見て，A・B は `no channel` の行として比べる．`rm_26_the_ceiling_is_decided_from_the_sequence` が除外の集合を `[C]` と assert する．`timed`・`late` は全 command で比べるまま．
   - mutation: P591（除外の条件から `!channel_less` を落とす）を当て直して killed．P592（release の有無を見ない）も killed（同じ test が持つ）．
3. **spec の文**:
   - `design/09-mock-radio.md` MR-18: 床の式を消し，`e` と late の判定を RM-21 に委ねた（`at` が床より後なら `e = at`）．window が profile の値であることと，late → 満杯の順は残した．
   - `design/07-radio-model.md` RM-16: Ceiling の定義をここに 1 度だけ置いた（release の遅れ全般 — poll，予約の途中の同期的な装置の呼び出し，channel の無い先頭，apply の直前の喪失 — で `S` の前に渡されなかった live な command）．
   - `design/18-uhd-radio.md` UR-26: それを引き，Module の仕組み（終わりが予約された後は release が行を書かない）だけを足した．RM-16 と UR-26 の Test 行に新しい試験を足した（セルフレビュー 3 回目の P2）．設計メモ 38 は触っていない．

### 9.2 確かめたこと

- 新しい試験と直した試験を各 20 回（`timeout 300`，1 binary ずつ，`--test-threads` 既定）: `ur_26_a_release_after_the_stop_was_published_leaves_the_row_to_the_end` 20/20，`ur_26_a_command_given_after_the_stop_s_instant_is_cancelled_at_it` 20/20，`rm_26_the_ceiling_is_decided_from_the_sequence` 20/20，`rm_26_the_timeline_against_uhd_control` 20/20．
- `cargo test -p ezsdr-radio-uhd`: lib 350 passed・`tests/fake.rs` 219 passed（0 の binary を含め全部通る）．`cargo test -p ezsdr-mock-radio`: 169 passed（4+32+49+83+1）．`cargo test -p ezsdr-radio`: 79 passed（1+8+2+14+54）．
- `cargo clippy --workspace --all-targets -- -D warnings`: 警告なし（最後の変更の後で 1 回）．
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`: 0 problem(s)．
- `git diff --check`: なし．変えたファイルは mode 644．
- mutation: `REMOTE_SSH="ssh -o BatchMode=yes" ... remote_mutate.py maintenance sim02:2 P581 P591 P592 P596 P599` で **5/5 killed**，その後 P581 P596 P599 P600 で **4/4 killed**（P599・P600 は 2 回目の直しの行）．sim02・sim03 の `ezsdr-mut-multi` が 0 のときに流した（`docker ps` で確認）．ローカルの `mutate.py` は流していない．
- 全 mutation リスト（942＋165 行）の `old` が tree に 1 回だけ一致することを確かめた．
- 負荷: busy loop，`--test-threads` の変更，並列 cargo なし．`TMPDIR` は変えていない．

### 9.5 セルフレビュー

reviewer は `openai/gpt-6-luna#max`（`--agent plan`，読むだけ），1 回ごとに新しく起こし，log は `review-fix1-<n>.log`．

**1 回目**（P1 1）:
1. P1「停止公開後の release が未送信の command を装置へ渡す」（`control.rs:598-655`）: **退けた**．例（A・B とも `e = 10.4 ms`，停止 `S = 10.9 ms`，release が 10.6 ms に再開）では，release は `now = 10.6 ms` で B を装置へ渡す．渡した時刻 10.6 ms は `S = 10.9 ms` より前なので，D2 (4) の「`S` の前に渡したものは issued」に当たり，Mock も（10.4 ms を過ぎた step の後なら）B を 10.4 ms に適用して issued にする．Ceiling は「`S` までに渡されなかった」command の話で，公開の瞬間にまだ渡していないことではない．release の条件を変えるなという `fix1-brief.md` §1 の指示と，設計メモ 38 §2（release は今のまま）にも反する．ただし release の `now` がすでに `S` を過ぎている場合は渡した時刻が `S` の後になり，これは 2 回目の P1 で直した（下）．

**2 回目**（P1 1）:
1. P1「終端後に渡した command が `issued` と記録される」（`control.rs:598-643, 652-672, 733-761`）: **直した**．停止 `S = 10.2 ms` が公開された後に release が 10.6 ms に走り，`e = 10 ms` の live な command を渡す経路．渡した時刻 10.6 ms は `S` より後なので `S` の時点では渡されておらず，RM-16 の Ceiling として cancelled になる．fold が終わりを見たとき `now > S` なら，その pass で渡した `(e, seq)`（`given_here`）を `released_at` から `held` へ戻し，`settle` が `cancelled_at = S` で確定する．装置の呼び出し（いつ・どの channel に渡すか）は変えていない．試験 `ur_26_a_command_given_after_the_stop_s_instant_is_cancelled_at_it`，mutation P599・P600．

**3 回目**（P1 1・P2 1，3 回で終了）:
1. P1「同期 `apply` が停止時刻をまたぐと command が誤って `issued` になる」（`control.rs:602, 629-635, 655-667`; `device.rs:62-72`）: **退けた**．`Device::apply` は `Applied`（rate・freq・gain・antenna）を返し，受理の時刻を持たない．uhd-control が装置の時計で読めるのは pass の `now` だけで，`S` をまたぐ同期呼び出しのなかで「`S` の前に受理されたか」は区別できない（streams の lock を装置の呼び出しの下に置かない UR-25 の制約で，呼び出しと終わりの予約を同じ lock で順序づけることもできない）．設計メモ 38 の Ceiling は「予約の途中の同期的な装置の呼び出し」を，その後ろの command の release の遅れの原因として挙げるだけで，呼び出し自身の受理時刻を `S` と比べる規則は置いていない．いまの規則は pass の `now` で判定する（`now > S` の pass が渡したものは 2 回目で cancelled にした）．受理時刻で判定するには装置の API か「`S` までに渡した」の定義を変える設計判断が要るので orchestrator に渡す（§8 に残す）．
2. P2「RM-16／UR-26 の Test 行に新しい試験が無い」: **直した**．RM-16 に `ur_26_a_release_after_the_stop_was_published_leaves_the_row_to_the_end`・`ur_26_a_command_given_after_the_stop_s_instant_is_cancelled_at_it`・`rm_26_the_ceiling_is_decided_from_the_sequence` を，UR-26 に前の 2 つを足した．

## 10. 直し 2 回目

STATUS: done

`sol-fix1-verdict.md` の major 2 件を `fix2-brief.md` §1 のとおりに直した（orchestrator の裁定: `given_here` の仕組みをやめ，「`S` までに渡した」を command ごとの時刻で決める）．HEAD `7a3b553` の未 commit の変更の上に置いた（commit・push・add・stash はしていない）．

### 10.1 変えた所

- **`crates/ezsdr-radio-uhd/src/provider/control.rs`**
  - `Given` に `handed_at: Option<i64>`，`Handed::Given(i64)`．`give` が各 `apply` の直前（probe と loss の確認の後）に `self.core.now()` を読み，最後のものを返す（`targets` は常に 1 channel なので「最後」はその 1 つ）．channel 無しの `Given` は `handed_at: None`．
  - `release`：`given_here` と，fold で `now > at` なら渡したものを `held` へ戻す分岐を消した．fold は streams の lock の下で `ended_at()` が `Some` なら何もせずに戻る（直し 1 回目のまま）．release の条件（いつ・どの channel に渡すか）と装置の呼び出しは変えていない．
  - `settle`：`released_at` の command ごとに `due`（停止 `e ≤ S`，喪失 `e < S`）と `handed = handed_at.is_none_or(|h| 停止なら h ≤ S，喪失なら h < S)` を別々に見る．両方なら `record_given`（issued / `no channel`），`due` で handed でなければ Ceiling として cancelled（`cancelled_at = S`），`due` でなければ cancelled．channel 無し（`handed_at: None`）は今のまま（`due` なら `no channel`，でなければ cancelled）．
  - コメントを新しい規則に合わせた（fold，`give`，`settle`）．
- **試験**（`control.rs` の test module）
  - 新規 3 本：`ur_26_a_stop_between_two_hand_overs_judges_each_at_its_apply`（試験 1．停止を 2 つ目の apply の直前の probe で公開し，時計を `S` ちょうど／`S` + 0.1 ms に進める 2 形），`ur_26_a_refusal_behind_a_handed_command_still_settles_it_at_the_stop`（試験 2），`ur_26_a_lost_apply_behind_a_handed_command_settles_both_at_the_loss`（試験 3）．
  - 直し 1 回目の 3 本（`ur_26_a_release_after_the_stop_was_published_leaves_the_row_to_the_end` の 2 形，`ur_26_a_command_given_after_the_stop_s_instant_is_cancelled_at_it`）はそのまま通る（変えていない）．
  - 既存 2 本の期待値を直した（下の 10.2）．
- **spec**（DA-1 の形）
  - `design/07-radio-model.md` RM-16：「`S` までに渡した」の定義を 1 度だけ置いた — その command の装置の呼び出しを始める直前に読んだ装置の時刻 `h`（channel が複数なら最後の channel の `apply`）が，停止なら `h ≤ S`，喪失なら `h < S`；`S` をまたぐ呼び出しは始めた時刻で決める（装置は受理の時刻を返さない）．前の節にあった「`S` の前に instant が来た given は issued」の 1 文は，この定義と食い違うので消した（「due なら その結果を保つ — issued か `no channel`」が同じことを言っている）．
  - `design/18-uhd-radio.md` UR-26：RM-16 を引いたまま，Module の仕組み（各 channel の `apply` の直前に装置の時刻を読み，最後を hand-over として `S` と比べる）を 1 文だけ足した．
  - RM-16・UR-26 の Test 行に新しい 3 本を足した．両 spec の Changes の 2026-10-24 の行に hand-over の比較を足した（同じ日の同じ段階なので行は増やしていない）．
- **mutation**（`plan/maintenance/tools/mutations.json`）：新規 3 行 P601（喪失側 `h < at` → `h <= at`），P602（停止側 `h <= at` → `h < at`），P603（`handed_at` が command の instant `e` になる）．当て直し 8 行：P419（`Handed::Given(0)`），P432（`Given` の literal），P580（`Handed::Given(...)` の形），P583・P584（destructuring に `handed_at`），P596（fold の `ended_at` の塊），P599（`handed` を比べない），P600（`ended_at` → `lost_at`）．`given_here` に結びついた P599 は「`handed_at` を比べない」に当て直した（消していない）．

### 10.2 既存テストの書き換え（2 本）

どちらも「release の pass が `S` と同じ時刻に走り，その apply の途中で喪失が公開される」形で，旧規則（pass の `now > S` で戻す）は issued，新規則（呼び出しを始めた `h` が `h < S`）は cancelled になる．orchestrator の定義（喪失は `<`）どおり，**cancelled に直した**：

- `ur_29_a_loss_after_a_release_s_command_stops_the_one_behind_it`：A（`e = 49 ms`）の apply が 52 ms に始まり，その途中で喪失（`S = 52 ms`）が公開される．A は cancelled（`cancelled_at = 52 ms`），B は `no channel` のまま．理由と `cancelled_at` の assert を足した．
- `ur_29_a_loss_published_during_a_release_settles_the_channel_less_behind_it`：live な command（`e = 9 ms`）の apply が 10 ms に始まり，その途中で喪失（`S = 10 ms`）．live は cancelled（10 ms），channel 無しの command も cancelled（10 ms）．`cancelled_at` は 2 行とも 10 ms．

### 10.3 確かめたこと

- 新しい 3 本と，直し 1 回目の 2 本・期待値を直した既存 2 本の計 7 本は各 **20 回**（`timeout 300`，1 binary ずつ，`--test-threads` 既定）で全部通った：7 本すべて 20/20．
- `cargo test --workspace`：全部通る（`ezsdr-radio-uhd` lib 353・`tests/fake.rs` 219 を含む）．
- `cargo clippy --workspace --all-targets -- -D warnings`：警告なし．
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`：0 problem(s)．
- `git diff --check`：なし．変えたファイルは mode 644．
- 全 mutation リスト（945＋378 行）の `old` が tree に 1 回だけ一致することを確かめた（`old2` も）．
- `REMOTE_SSH="ssh -o BatchMode=yes" ... remote_mutate.py maintenance sim02:2 P419 P432 P434 P580 P581 P583 P584 P585 P586 P587 P593 P596 P598 P599 P600 P601 P602 P603`：**18/18 killed**．sim02・sim03 の docker が空のときに流した．ローカルの `mutate.py` は流していない．
- 「最初の channel の時刻」の mutation 行は足していない：`targets` が常に 1 channel を返すので，複数 channel の試験が作れない（`give` の `chans` は長さ 1）．
- 負荷：busy loop，`--test-threads` の変更，並列 cargo なし．`TMPDIR` は変えていない．

### 10.4 セルフレビュー

reviewer は `openai/gpt-6-luna#max`（`--agent plan`，読むだけ），log は `review-fix2-1.log`・`review-fix2-2.log`．`impl-brief.md` §4 のコマンドで checklist は `checklist-fix2.md`．

**1 回目**（P1 0）：指摘なし．新しい 3 本と既存 2 本の assert，`handed_at` の記録と比較（停止 `≤`／喪失 `<`），release の全出口で `released_at` に残ること，RM-16 の定義が 1 度だけで UR-26 が引いていること，release の条件と装置の呼び出しが変わっていないことを見て **Ship**．reviewer は mutation を実行せず読んで判定し，workspace gate と実機は未実施（どちらも orchestrator の gate）．ファイルの変更なし．

1 回目の後，RM-16 の定義の文を Ceiling の文の前に移した（同じ 2 文の並べ替え，意味は変えていない）．reviewer に最終の tree を見せるため，もう 1 回起こした．

**2 回目**（P1 0・P2 1）:
1. P2「喪失を返す `apply` では `handed_at` が判定に使われない」（`control.rs:717, 724-726, 645, 745-761`; `design/07-radio-model.md:170`）: **退けた**．裁定は `fix2-brief.md` §1 の試験 3 と `impl-brief.md` §1 の S3 にある: 喪失で `give` が失敗した command は `held` に残し，「渡していないので issued にはならない」— 試験 3 も B を「持っていたものとして cancelled（喪失の `S` で）」と期待する．`Handed::Lost` は装置がその command を取らなかった結果（`apply` がエラーを返した）で，`handed_at` を持つのは `Handed::Given` だけ，というのが裁定の形．reviewer の例（呼び出しが 10.1 ms に始まり，処理中に `S = 10.2 ms` の喪失が公開される）は，装置が取らなかった command を issued にすることになり，S3 と逆になる．よってコードは変えない．この形の試験（試験 3）が B を cancelled で固定している．

### 10.5 残ったこと

- orchestrator の gate（全 mutation リスト，`mutate.py` の全行，負荷，docker，Python suite）は流していない．`sim02:2` の 18 行だけ流した（sim03 は使っていない）．
- 複数 channel の command は `targets` が常に 1 channel を返すため，この Module では作れない（RM-16 の定義は「最後の channel」を一般則として書き，mutation 行はその分を足していない）．
- §8 に残した stop と loss が同じ tick の lifecycle，`at=None` の同値変異は前の節のまま（この直しでは触っていない）．
