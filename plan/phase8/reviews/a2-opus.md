# A2：orchestrator の仕分け，レビュー，裁定，数

## 1. 何をしたか

計画 32 の A2（Test 行に試験の無い条件のうち radio の Provider の 14 件，`plan/maintenance/32-triage.json`）を Phase 8 段階 2 で片づけた．本番のコードは変えていない．

**仕分け**（2026-10-25，Opus が今の tree で assert を読んで確かめた）：

| 条件 | 判定 | テスト |
|---|---|---|
| RM-11 `TX_UNDERFLOW` の source | 済み（Test 行に名が無かった） | `ur_28_a_layout_transmit_report_names_its_stream` |
| RM-18 sequence error の `cause` | 済み（1b-7b で `restart_gap_ns` は消えた） | `mr_22_sequence_error_shape` |
| MR-25 `stop(mode)` の pending の記録 | 済み（38a．Test 行に「none named」が残っていた） | `mr_25_a_stop_records_a_later_update_as_cancelled_at_the_stop` |
| UR-17 missed start の `GAP_BEFORE`・`lost` | 済み | `ur_17_a_missed_start_without_another_report_is_not_an_overflow` |
| UR-18 overrun・sequence の `cause` | 済み（`restart_gap_ns` は消えた） | `ur_19_reports_before_one_block_keep_each_kind` |
| RM-11 `CLOCK_LOST` の source | 足した | `ur_27_a_lost_reference_is_clock_lost`（1 行） |
| RM-14 `requested_at` → `requested_target` | 足した（Mock と UHD） | `rm_14_the_recorded_requested_target_is_the_requested_at`，`rm_14_the_handed_burst_keeps_its_requested_at` |
| MR-19 restart gap 0 | 足した | `mr_19_a_zero_gap_still_loses_the_dropped_block` |
| MR-29 `start_rx` の `at` | 足した | `mr_29_start_rx_takes_effect_at_its_at` |
| MR-29 受信 link の T0 の下限 | 足した | `mr_29_a_receive_link_never_begins_before_t0`，`mr_29_a_receive_link_floors_a_command_at_its_t0`（語彙） |
| MR-29 決して late でない | 足した | `mr_29_start_rx_is_never_late` |
| UR-13 `arm` は放射しない | 足した | `ur_13_arm_radiates_nothing` |
| UR-17 複数 channel の planar cf32 | 足した | `ur_17_a_block_s_channels_are_planar_cf32` |
| UR-17 再開の lead は決定の `now` から | 足した | `ur_17_the_missed_start_lead_is_measured_from_the_decision_s_now` |

規則の足りない件は無く，新しいテストで spec と違うコードも見つからなかった．spec 07・09・18 の Test 行と Changes を直した（UR-17 の「すべての link が共有する参照」の none named は C のまま残した）．mutation の行は T34〜T45（maintenance），当て直し U27（phase7）．

テストを足すだけなので risky でない段階として扱い，テスト計画と Sol の計画レビューを省いた（AGENTS.md §9 の 1）．実装は Go の DeepSeek，セルフレビューは GPT-6 Luna．

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| 実装 | 9 件，テスト 10 本，mutation 11 行 | セルフレビュー 2 回（P1 0，P2 1 と P3 1 は直した） |
| Opus | 差分を読む | O1：2 channel のテストが channel ごとに一定の値で，index の誤りを見分けない．O2：never late のテストが `ideal`（lead 0）．O3：`arm` のテストは `Prepared` を手で組み立てる（記録） |
| Sol のコードレビュー | | **FAIL**，major 1（O1 と同じ），minor 1（T39 が T0 の下限でなく「clock が無い」で落ちる） |
| 直し 1 | 2 channel の全位置を違う値で照合，T45．語彙の `command_instant` の受信の floor を直接見るテストを足し，T39 を 1 箇所の変異に | セルフレビュー 1 回（P1 0） |
| Sol の再レビュー | | **PASS** |
| gate | §4 | |

## 3. 指摘と扱い

Sol の 2 件は採った．O2 は直さない（T41 が LATE_COMMAND の数で落ち，x3xx の再開は `begin` の lead が別に掛かるので時刻の比較が別の規則の試験になる）．

MR-29 の T0 の下限は 3 か所が守る（`command_instant` の受信の floor，`begin` の直前の segment の cut，`ready`）．実装役は重複かと記録したが，Sol は正しい層分けと判断した（command の効く時刻，前の segment との境界，受信の準備）．Opus も同じ判断で，設計メモは要らない．

## 4. gate

テスト 1698（失敗 0），clippy，`rules.py` 0 件，docker の uhd（clippy，lib 365，`uhd_api` 5），Python 3.14・3.9 各 57/57，mutation の全リスト 1353/1354（G09 は既知．sim02・sim03 で 16 shard）．

## 5. 数（AGENTS.md §9 の 8）

| 出どころ | major | minor | 機構 |
|---|---|---|---|
| セルフレビュー（Luna，3 回） | — | P2 1・P3 1（直した） | Test 行の残り，コメントの誤り．機構なし |
| Opus（コード） | 1 | 1 | テストの力．機構なし |
| Sol（コード） | 1 | 1 | テストの力（O1 と同じ），mutation の行の殺し方．機構なし |
| Sol（再レビュー） | 0 | 0 | — |

テストだけの段階で，機構に数える指摘は無い．
