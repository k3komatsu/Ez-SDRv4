# 設計メモ 37 の 37b：orchestrator のレビュー，裁定，数

## 1. 何をしたか

設計メモ [37](../37-test-thread-timing.md)（#69）の 37b．37a の対応表の B-2・B-3 を直した．本番のコードは変えていない．

- **raw device の 13 本**（`ur_33_…`）：`tests/fake.rs` から `src/device.rs` の tests（手動の時計の `manual`，参加者の `participant`）へ移した．名前は変えていない．移すときに assert を足した（空の channel list が channel 0 を取ったこと，空の end-of-burst と 1 sample 空けた burst の sample の値，止めた stream の tail の最後の sample の instant）．queue のテストは，17 件目の呼び出しの前と後の装置の時刻を参加者の中で読む形にした（Sol の major 1）．
- **`Direct` 系の 5 本**：`ur_24_hardware_timed_updates`，`ur_24_a_far_future_update_does_not_delay_a_nearer_one`，`ur_25_rx_channels_from_zero_starts_at_its_instant`，`ur_25_a_timed_update_at_the_cold_change_s_instant_reaches_the_switch` は `src/provider/control.rs` の制御された時計の unit test へ移した．`ur_25_a_stream_silent_before_e1_switches_before_e2` は rx の private な `recv_timeout`・`turn` を見るので `src/provider/rx.rs` へ移し，timeout を等値で固めた（cut で 5 ms，cut＋span の後は下限の 1 ms，装置に渡した値．Sol の major 2）．
- tests/README.md の raw の段落，spec 18 の UR-33 と §6 の参照，Changes の 1 行．mutation の 19 行を `fake` から `lib` に当て直した（P204，P205，P207，P213〜P217，P230〜P232，P264，U20，U21，U37，B01，B02，B17，B30）．
- `fake` の binary は 255 本から 237 本に，`--lib` は 277 本から 295 本になった．

実装は Go の DeepSeek（`opencode-go/deepseek-v4.1-flash#max`），セルフレビューは GPT-6 Luna（`openai/gpt-6-luna#max`）．

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| 実装 | Go の DeepSeek | セルフレビュー 3 回（P1 3 → 1 → 1），当て直した 19 行 19/19 killed（B01 は 1 回目に生き残り，テストを `turn()` を通す形に直して kill），移した 18 本を各 20 回 |
| Opus | 指示書への忠実さ，本番のコードの不変，mutation の行 | major なし．指示書の誤り 1 つと掃除の漏れを見つけた（§3） |
| Sol のコードレビュー | | **FAIL**，major 2 |
| 修正 | Go の DeepSeek | セルフレビュー 1 回（P1 0），B01・B02 killed |
| Sol の再レビュー | | PASS，指摘なし |
| gate | §4 | |

## 3. 指摘と扱い

Sol のコードレビュー（どちらも採用）：
- **major 1**：queue のテストは，17 件目の `apply` の前に送る合図と 200 ms の「終わらない」で判定していた．worker が合図の後で止まっただけでも通る．参加者の中で呼び出しの前（0）と後（20 ms 以後）の装置の時刻を読む形にした．Luna も 2 回同じ所を指摘したが，実装役が「参加者なので時計は止まる」として退けていた．時計が止まることは worker が動いたことを保証しない．
- **major 2**：silent RX の timeout の assert は「100 ms 未満」だけで，一律 99 ms でも通り，旧テストの「1 ms 未満を渡さない」も落ちていた．式から出した値の等値にした．

Opus：
- 指示書と設計メモ §7 は `Direct` を「実時間の時計」と書いたが，`common::fake` は `ClockMode::Manual` を強制する（`tests/common/mod.rs:305-309`）．試験のスレッドが参加者でないので窓は同じように危うく，結論は変わらない．設計メモ §7 の文を直した．
- **掃除の漏れ**：設計メモ §3 は「参加者でないスレッドが時刻を読んでから次の操作か assert までに時間が進み得る系列をすべて一覧にする」と決めたが，37a の対応表（free の DeepSeek）は `Direct` 系を 6 系列しか挙げなかった．`fake.rs` には `direct.now()` が 89 か所，`time_now()` が 16 か所ある．対応表は系列の全数を grep で確かめずに使った．残りは 37c で一覧にする．
- 実装役は，他の利用者の負荷の下で範囲外の `fake` の 3 本（`rehearsal_b5_overflow`，`ur_17_blocks_carry_the_device_timestamps`，`ur_25_a_receive_enable_counts_its_start_lead_from_the_end_of_its_configuration`）が落ちたと報告した．Sol の回でも別の 4 本が落ちた．このころ `/tmp`（tmpfs，ユーザーごとの上限）が別の job の出力で埋まっていて，capture が書けなかった（「artifact が無い」，sample の数が足りない）．gate では 4 本とも通った．`ur_25_a_receive_enable_…` は `ready - e` が 80 ms より 1 tick 短かった．Authority の推定を挟んだ 2 回の読みの差と見ている（INFERRED）．37c で確かめる．

セルフレビュー（Luna，4 回）：P1 は計 5 件（空の channel list，sample の値，queue の worker，rx の実時間，silent の late）．4 件は直した．queue の worker は 2 回退けられ，Sol の major 1 になった．退けた理由は [37b-report](37b-report.md)．

## 4. gate

テスト 1607（失敗 0），clippy，`rules.py` 0 件，docker の uhd（clippy，lib 303，`uhd_api` 5），Python 3.14 56/56．Python 3.9 は 55/56：`test_v54_sleep_is_run_time`（10 s の仮想時間に実時間 11.0 s）は，この host では負荷が低くても落ちる既知の失敗（plan 22，1b-7b の報告，handoff の「負荷に弱いテスト」）で，37b は Python も server も変えていない．

ローカルの workspace のテストは，owner の解析の job で load 15〜18 の下で流れ，`fake` は 237/237．mutation の全リスト 1290/1294．G09 は既知．G29（`kg_04`）と U35（`ur_07_now_tracks_the_device_while_no_call_runs`）は単独の流し直しで killed．P04 は変異なしの時点で落ちた（BASELINE FAILED．filter の Python の `test_v54_sleep_is_run_time` が実時間 10 s の上限を超えた．37a の gate では killed）．このテストも実時間に頼る判定で mutation の行の filter なので，37c で扱う．sim02・sim03 は解析の job で 32 コアのうち約 28 が埋まっていたので，空いている 4 コアに収まる 1 shard ずつで流した（負荷で落ちるテストが変異を「killed」と数えるのを避けるため）．

## 5. 数（AGENTS.md §9 の 8）

| 出どころ | major | minor | 機構 |
|---|---|---|---|
| セルフレビュー（Luna，4 回） | P1 5（4 件直した，1 件は退けて Sol の major 1） | P2・P3 | テストの観測．機構なし |
| Opus | 0 | 0 | 指示書の誤り 1（`Direct` の時計）．対応表の掃除の漏れは 37c へ |
| Sol（コード） | 2 | 0 | 2 件とも判定が worker の動く時機か緩い上限に頼った．`wall-clock-test-oracle` の型だが，#69 を直す段階の中の指摘なので新しい件に数えない（37a と同じ扱い） |
| Sol（修正） | 0 | 0 | |
| gate | 0 | — | なし（Python 3.9 の 1 本は既知で範囲外） |
