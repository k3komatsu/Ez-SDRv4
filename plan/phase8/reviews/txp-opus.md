# 段階 TXP：orchestrator のレビュー，裁定，数

## 1. 何をしたか

送信の 100/200 MS/s の失敗への答え（owner 2026-10-28：実機で優先度が効くと確かめてから入れる）．調査（repo の外：`~/.cache/ezsdr-fixes/2026-10-26-stage3/tx-throughput-report.md`）は Module と UHD の 1 回の send の手間は律速でなく，X300 の送信 buffer（65 536 sample，200 MS/s で 0.33 ms）と普通の優先度の uhd-tx の preempt を原因と見た（INFERRED）．実機 2026-10-28：UHD の `benchmark_rate` で送信 200 MS/s・60 s，普通の優先度 underrun 8，`--priority high` 0，`--tx_spb 1000` 8（[bench-results](../bench-results.md)）．

| 項目 | 中身 |
|---|---|
| 優先度 | `Device::raise_thread_priority`（default なし，24 の実装すべて）．`UhdDevice` は `uhd_set_thread_priority(0.5, true)`（UHD 4.10：呼んだ thread を SCHED_RR，Sol が source で確認）．`Tx::run` の最初の文で 1 回．拒否は `rejected` に 1 行，`lost: false` で致命にしない（UHD の EPERM は error 31 で，そのままだと DEVICE_LOST になっていた）．測定の包み役 `GuardedDevice` は拒否で latch を立てない．成功は `stats.tx_realtime`（dummy build でも成功を返すので「実時間になった」ではない） |
| 窓の端 | 1 つの大きさ／準備の判定を `send` に置き，2 つの入口がそこを通る．送れる量の全部が窓に入らなければ待つ（何も送らない）．除外は最初の timed SOB だけ（`min(wanted, avail)`）．wrap・burst の終わり・held・cold の cut の手前は小さくても送る．chunk は変えない |
| 実機の受け入れ | `bench.md` (f) の B0〜B4（低い rate，100/200 MS/s，同時運転と時刻つきの control と Stop，thread の policy の実値と RT の bandwidth）．B0 の send の大きさは `hw_txp_b0_send_sizes`（`ProbeDevice`）で読む．送信の前に att の確認 |

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| 試験計画 | Sonnet 5.5 | Sol 3 回：major 5 → 2 → 1（3 回目の 2 件は実装の最初に計画へ） |
| 実装 | Sonnet 5.5（worktree，43a と並行）．セルフレビュー 1 回（Go の DeepSeek：P1 0，P2 7，P3 7．13 件直し 1 件退け） | phase7 の U19 は重複した early return と等価になり，return と行を消した（性質は P637 と P1109） |
| Sol コード | | FAIL：major 2（M8b の絶対の host 時刻，B0 の Manifest の `blocks` は数），minor 1 |
| 修正 1・2 | M8b を経過で，B0 に診断の hardware 試験，M14e を新しい clock の送信まで | Sol 見直し 2 回で PASS |

## 3. 数

| 出所 | major | minor | 機構 |
|---|---|---|---|
| Sol（計画，3 回） | 8 | 8 | 測定の包み役が error を device の故障と取り違える（`failure-collapsed-into-value` の形，計画の段で防いだ），送る入口が 2 つ，試験の観測 |
| Sol（コード，3 回） | 3 | 1 | 試験の時刻の基準（設計メモ 37），実機の手順の観測元 |
| セルフレビュー | 0 | 14 | |

## 4. gate

- テスト 2020 全部通った．clippy，`rules.py` 0 件，docker の uhd（lib 464，`uhd_api` 5，`--features uhd` の clippy），Python 3.14・3.9 各 57．
- mutation の全リスト 1824/1825（G09 は既知）．X8〜X10（`uhd.rs`，feature `uhd`）は遠隔の image に libuhd が無いので延期．
