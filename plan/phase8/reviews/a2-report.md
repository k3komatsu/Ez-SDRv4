STATUS: done

# A2：テストの無い条件 9 件にテストを足す — 報告

HEAD `6f6d6c4`，tree はきれいな所から開始．差分は 9 ファイル（+359 / −9）．**本番コード（`src/` のテストでない部分）は変えていない**（`src/` の差分はすべて `#[cfg(test)] mod tests` の中）．`git add`・commit・push・`cargo fmt` はしていない．`mutations.json` の差分は **114 行の追加のみ（削除 0）**，字下げは 1 個の空白のまま．

## 1. 件ごとのテスト（9 件 → テスト 10 本）

| 件 | テスト | 置き場 | assert（落とす誤り） |
|---|---|---|---|
| 1 RM-11 `CLOCK_LOST` の source | `ur_27_a_lost_reference_is_clock_lost`（既存に 1 行追加） | `crates/ezsdr-radio-uhd/tests/fake.rs:1971` | `events_of(..., "radio.CLOCK_LOST")[0].source == usrp()`（device の節点）．DEVICE_LOST の source の assert と同じ形．誤り：stream の節点から発行する |
| 2 RM-14 `requested_at`（Mock） | `rm_14_the_recorded_requested_target_is_the_requested_at` | `crates/ezsdr-mock-radio/tests/mock_radio.rs:1056` | `BurstRecord.target`（OnTime は `at`，SendAsap は 2 012 000＝device lead 後の格子点）と `BurstRecord.requested_target == Some(requested_at)`．誤り：`requested_at` を捨てて `at` を記録する |
| 2 RM-14 `requested_at`（UHD） | `rm_14_the_handed_burst_keeps_its_requested_at` | `crates/ezsdr-radio-uhd/src/provider/control.rs:2132` | `TxCmd::Burst` の `held.k`（OnTime 20 000，SendAsap 12 000）と `held.open.requested_target == Some(requested_at)`．同じ誤りを UHD 側で落とす |
| 3 MR-19 gap 0 | `mr_19_a_zero_gap_still_loses_the_dropped_block` | `crates/ezsdr-mock-radio/tests/mock_radio.rs:1384` | `ideal`＋`Block` capacity 1：block 0 を溜め block 1 を `Full` で落とし，次は `b = 4 000` から `GAP_BEFORE｜RESTARTED`，`lost == Some(2 000)`（= block_len），hot path の `RX_OVERFLOW { overrun, 2 000 }`．誤り：`k_g = a + gap`（gap 0 で落ちた block を重複させる） |
| 4 MR-29 `at` | `mr_29_start_rx_takes_effect_at_its_at` | `crates/ezsdr-mock-radio/tests/mock_radio.rs:2040` | 1 ms に `Stop`，2 ms に受理した `at = 12 ms` の `start_rx`：clocks は `[(0, Some(1 ms)), (12 ms, None)]`，LATE_COMMAND 0 件．誤り：受理時刻で再開する |
| 5 MR-29 T0 下限 | `mr_29_a_receive_link_never_begins_before_t0` | `crates/ezsdr-mock-radio/tests/mock_radio.rs:2058` | T0 = 10 ms の link 付き stream：8 ms の `Stop` と，T0 − 1 ms を求める 8.5 ms 受理の `start_rx`：clock は `[(10 ms, None)]` の 1 本だけで，最初の block は index 0．誤り：`at` をそのまま採り T0 前に始める |
| 6 MR-29 never late | `mr_29_start_rx_is_never_late` | `crates/ezsdr-mock-radio/tests/mock_radio.rs:2077` | 1 ms の `Stop` の後，過去の `at = 1 ms` を 3 ms に受理：clocks は `[(0, Some(1 ms)), (3 ms, None)]`（再開は受理時刻），LATE_COMMAND 0 件．誤り：過去の `at` で LATE_COMMAND を出し lead だけ遅らせる |
| 7 UR-13 `arm` は放射しない | `ur_13_arm_radiates_nothing` | `crates/ezsdr-radio-uhd/src/provider/mod.rs:806` | 送信 stream のある rig で `arm()` の直後（`start` の前）：`tx_send` の call が無く，`tx_open` が 1 回記録されている．誤り：arm で空の end-of-burst や最初の buffer を送る |
| 8 UR-17 planar cf32 | `ur_17_a_block_s_channels_are_planar_cf32` | `crates/ezsdr-radio-uhd/src/provider/rx.rs:1068` | 2 channel・len 3 の block の bytes を `ezsdr_hostmem::read_cf32(bytes, 3, c, i)` で読み，channel 0 は `(1, 2)`，channel 1 は `(3, 4)`（入れ替え・index ずれが無い）．誤り：`write_cf32` の channel と index の取り違え |
| 9 UR-17 再開 lead の起点 | `ur_17_the_missed_start_lead_is_measured_from_the_decision_s_now` | `crates/ezsdr-radio-uhd/src/provider/rx.rs:1089` | manual clock を 40 ms に進めて `RxRecv::LateCommand`：`rx_restart` の `at` と LATE_COMMAND の `applied` が `lattice(40 ms + rx_start_lead_ns)` に等しく，`T0 + lead` より厳密に後．誤り：T0（origin）から lead を測る |

テスト名はすべて規則 id で始まる（`rm_14_…`，`mr_19_…`，`mr_29_…`，`ur_13_…`，`ur_17_…`）．追加したテスト部品は `mock_radio.rs:63` の `start_rx(target, at)` 1 個だけ．**`tx_action` の署名は変えていない**（27 か所の呼び出しを触らずに済ませるため，件 2 のテストは `Action::TxBurst` を組み立てて `requested_at` を差し替える）．新しい rig・fake の新しい fault は足していない．

MockRadio は `radio.CLOCK_LOST` を発行しない（`crates/ezsdr-radio/src/lib.rs:119` が種を宣言し，`CLOCK_LOST` は種の表にだけある．`crates/ezsdr-mock-radio/` に発行箇所は無い）ので，件 1 は UHD の 1 本だけ．

## 2. 計画からの変更と理由

- **件 2 の `tx_action` 拡張はしなかった**（指示書は「引数で選べるようにしてよい」）．呼び出しが 27 か所あり，署名変更は差分と mutation 行の `old` を無駄に危険にするので，テストの中で Action の `requested_at` を書き換える形にした．挙動の差は無い．
- **件 5・6 の mutation は 2 箇所（`old`/`old2`）の行にした**（T39，T40）．理由は §4 の「最初に SURVIVED した行」に書く．`old2` は T21 と同じ既存の仕組みで，両方とも同じファイル `crates/ezsdr-radio/src/timeline.rs` の中にある．
- **件 5 のテストが pin するのは「結果」**（T0 前に clock も block も無い）．`at` の下限は `command_instant` の floor だけでなく，`timeline::begin` の「直前 segment の cut」と「cut ＋ 1 受信 call（`ideal` は 0）」の 2 つの下限でも守られており，floor を 1 つ外すだけの変異は等価（後述）．
- **design/09 MR-17 の Test 行も直した**（セルフレビュー P2）．RM-14 へ移った条件の `none named` が MR-17 の行に残っていた．

止めた件は無い．

## 3. spec の Test 行（DA-1）

`design/07-radio-model.md`
- RM-11：`ur_27_a_lost_reference_is_clock_lost`（device 節点が `CLOCK_LOST` の source）と `ur_28_a_layout_transmit_report_names_its_stream`（stream 節点が `TX_UNDERFLOW` の source）を追加．
- RM-12：`none named for start_rx's never-late instant` を消し，3 本の新テストを条件つきで名指し．
- RM-14：`rm_14_the_recorded_requested_target_is_the_requested_at` と `rm_14_the_handed_burst_keeps_its_requested_at` を追加（SC-23a の `requested_at` の引き継ぎ）．

`design/09-mock-radio.md`
- MR-17：`none named for requested_target and SC-23a's requested_at` を消し，`rm_14_the_recorded_requested_target_is_the_requested_at` を名指し（レビュー P2 の修正）．
- MR-19：`none named for a gap of 0` を消し，`mr_19_a_zero_gap_still_loses_the_dropped_block` を追加．
- MR-25：`none named for what stop(mode) records of pending commands` を削除（`mr_25_a_stop_records_a_later_update_as_cancelled_at_the_stop` は既に列にある）．
- MR-29：`none named for start_rx's at, for a receive link before T0, or for its never being late` を消し，3 本を名指し．

`design/18-uhd-radio.md`
- UR-13：`none named for radiating nothing at arm` を消し，`ur_13_arm_radiates_nothing` を追加．
- UR-17：`none named for the byte layout, the reference every link shares, or the restart's lead from the decision's now` のうち前 2 つを新しい unit テスト 2 本で名指しし，**`the reference every link shares` の `none named` は残した**．

各 spec の Changes に 1 行ずつ（2026-10-25，`[plan 32](../plan/maintenance/32-test-line-sweep.md) A2`）．
指示書 §3 の「既にテストがある 5 件」のうち，RM-18（`cause: Sequence` は `mr_22_sequence_error_shape` が既に名指し）・UR-17 の missed start の `GAP_BEFORE`/`lost`・UR-18（`ur_19_reports_before_one_block_keep_each_kind`）は既に名があったので**触っていない**．RM-11 と MR-25 は上で直した．

## 4. mutation の行

`plan/maintenance/tools/mutations.json` に **T34〜T44 の 11 行を追加**（`git diff --numstat` は `114 0`，字下げは 1 空白）．UHD の行は Phase 7 のリストではなく maintenance に置いた（T31〜T33 と同じ場所，A1 の続き）．

| 行 | 壊す式 | リスト／bin／filter | 結果 |
|---|---|---|---|
| T34 | `CLOCK_LOST` の source を `core.id` → `stream_node(Dir::Rx, 0)` | maintenance / fake / `ur_27_a_lost_reference_is_clock_lost` | killed |
| T35 | Mock：`requested = requested_at…` → `Some(at.time_point)` | maintenance / mock_radio / `rm_14_the_recorded_…` | killed |
| T36 | UHD：同上（`control.rs`） | maintenance / lib / `rm_14_the_handed_burst_…` | killed |
| T37 | `next_sample = stop.max(after_gap)` → `next + gap` | maintenance / mock_radio / `mr_19_a_zero_gap_…` | killed |
| T38 | `command_instant`：`base = asked.max(now).max(floor)` → `now.max(floor)`（`at` を無視） | maintenance / mock_radio / `mr_29_start_rx_takes_effect_at_its_at` | killed |
| T39 | T38 の floor 以外（`base = asked.max(now)`）＋ `begin` の直前 segment の下限（`cut + lead` と `ready` の cut 項）を外す（`old2`） | maintenance / mock_radio / `mr_29_a_receive_link_never_begins_before_t0` | killed |
| T40 | `base = asked.max(floor)`（過去の `at` を採る）＋ `begin` の `ready + lead` を `item.e + lead` に | maintenance / mock_radio / `mr_29_start_rx_is_never_late` | killed |
| T41 | `start_rx` の `command_instant` の late を見て LATE_COMMAND を出す | maintenance / mock_radio / `mr_29_start_rx_is_never_late` | killed |
| T42 | `arm` の transmit streamer open の後に `tx_send(&[&[]], sob, eob)` | maintenance / lib / `ur_13_arm_radiates_nothing` | killed |
| T43 | `write_cf32(buffer, n, c, i, …)` → `(buffer, n, i, c, …)` | maintenance / lib / `ur_17_a_block_s_channels_are_planar_cf32` | killed |
| T44 | 再開を `now + lead` → `stream.start + lead`（origin から測る） | maintenance / lib / `ur_17_the_missed_start_lead_is_…` | killed |
| U27（既存，phase7） | 参照監視を切る（killing test に assert を足したため当て直した） | phase7 / fake / `ur_27_a_lost_reference_is_clock_lost` | killed |

流したコマンド：`REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase7 sim02:2 T34 … T44 U27` → **12/12 killed**．ローカルで `mutate.py` は流していない（1 行も当てていない）．

### 最初に SURVIVED した 3 行（と直し方）

- **T42**：`samples` を空 slice（`&[]`）で送る変異は，fake が「buffer 数 ≠ streamer の channel 数」で**記録前に** `Err` を返すため等価だった（`device.rs:1521`）．1 channel 分の空 buffer（`&[&[]]`）に直したら killed．
- **T40**：Mock の plan は `begin` の `ready`（受理時刻）でも下限を取るので，`command_instant` の `now` を外すだけでは等価だった．`ready` の下限も「item の instant」に落とす 2 箇所の行にしたら killed（過去の `at` の 1 ms に再開し，clock の assert が落ちる）．
- **T39**：受信 stream の再開は `timeline::begin` が (a) 直前 segment の cut，(b) その cut ＋ 1 受信 call ＋ allowance（`ideal` は cut だけ）でも T0 に床打ちするので，`command_instant` の floor を外すだけでは等価だった．3 つの下限のうち 2 つを外す行にしたら killed（再開が stream の起点より前になり `rx_next_segment` が拾わず，clock が 1 本も出ない＝`clocks` の assert が落ちる）．つまり MR-29 の「T0 より前に始まらない」は **3 重に守られている**（`command_instant` の Rx の floor，`begin` の cut，`begin` の `ready`）．テストは結果（T0 前の clock/block が無いこと）を pin している．この重複自体は設計の話なので（AGENTS.md §6 の三振りに数えるかは orchestrator の判断），ここでは記録だけする．

## 5. 確かめたこと

- **新しいテストと直したテストを各 20 回**（`timeout 300`，1 binary ずつ，`--test-threads` 既定，`CARGO_TARGET_DIR` は main tree の既定）：10 本すべて **0/20 failed**．内訳：`ur_27_a_lost_reference_is_clock_lost`（fake），`rm_14_the_recorded_requested_target_is_the_requested_at`・`mr_19_a_zero_gap_still_loses_the_dropped_block`・`mr_29_start_rx_takes_effect_at_its_at`・`mr_29_a_receive_link_never_begins_before_t0`・`mr_29_start_rx_is_never_late`（mock_radio），`rm_14_the_handed_burst_keeps_its_requested_at`・`ur_13_arm_radiates_nothing`・`ur_17_a_block_s_channels_are_planar_cf32`・`ur_17_the_missed_start_lead_is_measured_from_the_decision_s_now`（lib）．
- **`cargo test --workspace`**：exit 0（66 個の `test result: ok`）．途中 1 回，`tm_06_no_crate_reads_ticks_in_their_own_domain` が新しい rx.rs の unit テストの `payload.applied.ticks_in(payload.applied.domain())` を検出して落ちた（TM-6：自分の domain を値から読む）．`ticks_in(core.root)` に直してから全 suite が通った．
- **`cargo clippy --workspace --all-targets -- -D warnings`**：警告なし（最終 tree）．
- **`python3 plan/maintenance/tools/rules.py --base HEAD`**：0 件．`--self-test`：ok．`git diff --check`：ok．
- **`rm_20_schema_freeze`**（`crates/ezsdr-radio/tests/radio_model.rs:375`）：1 passed（schema は変えていない）．
- 変更した 9 ファイルはすべて mode 644．`TMPDIR` は変えていない．負荷をかける操作（busy loop，並列 cargo，`--test-threads` の引き上げ）はしていない．テストの一時ファイルは /tmp のまま．

## 6. セルフレビュー

reviewer は `openai/gpt-6-luna#max`（`--agent plan`，読み取りのみ），1 回ごとに新しく起こし，ログはこの dir の `review-a2-1.log`，`review-a2-2.log`．

- **1 回目**：`P1 の数：0`．P2-1：design/09 MR-17 の Test 行に「`requested_target` と SC-23a の `requested_at` は none named」が残っている（新しいテストで偽になった）→ **直した**：その節を消して `rm_14_the_recorded_requested_target_is_the_requested_at` を名指し，design/09 の Changes 行にも MR-17 を足した．
- **2 回目**：`P1 の数：0`．P3-1：`mr_19_a_zero_gap_still_loses_the_dropped_block` のコメントの `b = 2 000` は誤り（落ちた block は `[2 000, 4 000)` なので `b = 4 000`）→ **直した**（コメントのみ，テストを 1 回再実行して green）．P3 なので追加の回は回していない（`P1 以上` の停止条件は満たしている）．
- 退けた指摘は無い．

## 7. 残ったこと

- MR-29 の T0 下限は 3 重の守り（§4）．設計を単純にするかは orchestrator／owner の判断．`mutation` の T39 はその重複をまたぐ 2 箇所の行である．
- `plan/maintenance/32-test-line-sweep.md` の Status は「A2 は Phase 8 の計画へ」のままにしてある（この段階の記録は orchestrator が close する時に書くものと読んだ）．`handoff.md` も触っていない．
- 見張りのプロセス・orchestrator のファイル（`*-brief.md`，`checklist*.md`，`.*.orig`）は読むだけで，書き換えていない．

## 8. 直し 1 回目

STATUS: done

`sol-code-verdict.md` の 2 件を，裁定どおり両方採って直した．本番コード（`src/` のテストでない部分）は変えていない．O2（`mr_29_start_rx_is_never_late` の `ideal` の lead 0）は指示どおり直していない．

### 8-1. major：UR-17 の試験が sample index の誤りを見分けなかった

- `crates/ezsdr-radio-uhd/src/provider/rx.rs:1068` の `ur_17_a_block_s_channels_are_planar_cf32` を作り直した．sample は channel `c`・index `i` に `[10c + i + 1, −(10c + i + 1)]` を持ち，2 channel × 3 index の全 6 位置を `ezsdr_hostmem::read_cf32(bytes, 3, c, i)` で照合する．channel 0 は `(1,−1) (2,−2) (3,−3)`，channel 1 は `(11,−11) (12,−12) (13,−13)`．channel ごとに一定だった前の値と違い，channel/index の入れ替え，index を 0 に固定する誤り，channel の中の逆順がどれも assertion を落とす．
- mutation 行 **T45** を足した（`plan/maintenance/tools/mutations.json:9693`）：`write_cf32(buffer, n, c, i, …)` → `write_cf32(buffer, n, c, 0, …)`（全 index を 0 に書く）．filter はこのテスト．T43（channel と index の入れ替え）はそのまま残した．
- 結果：T45 は **killed**（§8-4）．

### 8-2. minor：T39 が T0 の下限でなく「clock が無い」で落ちていた

- `crates/ezsdr-radio/tests/timeline.rs:1504` に `mr_29_a_receive_link_floors_a_command_at_its_t0` を足した．受信の line（origin = T0 = 10 ms）に，`at = T0 − 2 ms`・`now = T0 − 3 ms` の `command_instant(Some(&line), Some(…), …, 0, false)` が `(T0, false)` を返すこと（`e == T0`）を直接見る．送信の line（origin = T0）では floor が掛からず `(T0 − 2 ms, false)`，つまり `e = max(at, now)` であることも 1 行で見る．
- **T39 を 1 箇所の変異に戻した**（`plan/maintenance/tools/mutations.json:9631`）：`let base = asked.max(now).max(floor);` → `let base = asked.max(now);`．`old2`／`new2`（`begin` の直前 segment の下限を外す半分）は消した．`crate` は `ezsdr-radio`，`test_bin` は `timeline`，filter は新しいテストに変えた（前は `mr_29_a_receive_link_never_begins_before_t0`）．name も「link の T0 floor を無視する」に直した．
- Provider 経由の `mr_29_a_receive_link_never_begins_before_t0` は残した（変えていない）．T0 の下限は `command_instant` の floor，`timeline::begin` の cut の下限，`ready` の下限の 3 つが重ねて守っている（§4 の記録）．新しいテストはそのうち語彙の `command_instant` の受信 floor を 1 つで見る．§2 と §4 の「T39 は 2 箇所の行」「`old2`」はこの回で置き換わった（前の節はその時の記録として残す）．
- spec：`design/09-mock-radio.md:195` の MR-29 の Test 行の「a receive link's T0 floor」に新しいテストの名を足した．Changes に 1 行（`design/09-mock-radio.md:488`，2026-10-25，fix round 1）．`design/07` の RM-12 の Test 行は変えていない（指示は 07 か 09 のどちらか）．
- 結果：T39 は新しいテストで **killed**（§8-4）．

### 8-3. 確かめたこと

- 直したテストと新しいテストを各 **20 回**：`ur_17_a_block_s_channels_are_planar_cf32` 0/20 failed，`mr_29_a_receive_link_floors_a_command_at_its_t0` 0/20 failed（`timeout 300`，1 binary ずつ，`--test-threads` 既定，main tree の既定の `CARGO_TARGET_DIR`）．
- `cargo test -p ezsdr-radio`：55 passed，0 failed．`cargo test -p ezsdr-radio-uhd --lib`：357 passed，0 failed．`cargo test --workspace`：exit 0，`test result: ok` 66 個，`FAILED`／`failures:` 無し．
- `cargo clippy --workspace --all-targets -- -D warnings`：警告なし（exit 0）．
- `python3 plan/maintenance/tools/rules.py --base HEAD`：0 件．`--self-test`：ok．`git diff --check`：ok．
- この回に変えた 4 ファイル（`crates/ezsdr-radio-uhd/src/provider/rx.rs`，`crates/ezsdr-radio/tests/timeline.rs`，`design/09-mock-radio.md`，`plan/maintenance/tools/mutations.json`）は mode 644（差分全体の 10 ファイルも 644 を確認）．`TMPDIR` は変えていない．負荷をかける操作はしていない．`cargo fmt`／`rustfmt`，ローカルの `mutate.py`，`git add`・commit・push・`git stash` はしていない．

### 8-4. mutation（この回に足した行と直した行）

`REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:2 T39 T45` → **2/2 killed**（`maintenance: 2/2 passed`）．

| 行 | 壊す式 | リスト／bin／filter | 結果 |
|---|---|---|---|
| T39（直した） | `base = asked.max(now).max(floor)` → `base = asked.max(now)`（受信 floor を外す） | maintenance / timeline / `mr_29_a_receive_link_floors_a_command_at_its_t0` | killed |
| T45（足した） | `write_cf32(buffer, n, c, i, …)` → `write_cf32(buffer, n, c, 0, …)`（全 index を 0 に書く） | maintenance / lib / `ur_17_a_block_s_channels_are_planar_cf32` | killed |

`git diff --stat plan/maintenance/tools/mutations.json`：`122 +++…`，`1 file changed, 122 insertions(+)`（HEAD `6f6d6c4` からの差分全体．前の段階の 114 行に T45 の行が加わり，T39 は同じ位置で書き換わった）．字下げは 1 個の空白のまま．

### 8-5. セルフレビュー

reviewer は `openai/gpt-6-luna#max`（`--agent plan`，読み取りのみ），log はこの dir の `review-fix1-1.log`．

- **1 回目**：`P1 の数：0`．指摘なし（新しい 2 つのテストがそれぞれ channel/index の全位置と受信 floor の `e == T0` を直接見ていること，T39 が 1 箇所で filter が新しいテストであること，本番コードが変わっていないこと，`mutations.json` の各 `old` が 1 回一致することを確認）．`P1 以上` が無いので 1 回で止めた（2・3 回目は回していない）．
- 退けた指摘は無い．

### 8-6. 残ったこと

- T0 の下限の 3 重の守り（§4）はそのまま．この回は語彙側の 1 つを直接 pin するテストを足しただけで，重なりを減らすかは orchestrator／owner の判断．
- 新しいテスト名は `mr_29_` で始まるが，置き場は `ezsdr-radio` の `tests/timeline.rs`（指示が許した 2 つの接頭辞のうち）．同ファイルの他のテストは `rm_25_`／`rm_26_`／`rm_34_` で始まる．
- `plan/maintenance/32-test-line-sweep.md` の Status と `handoff.md` は前の節のまま触っていない．
