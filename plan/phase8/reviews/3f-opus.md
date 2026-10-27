# 段階 3f：orchestrator のレビュー，裁定，数

## 1. 何をしたか

owner の決め（2026-10-28）：queue の深さは条件ごとに成功が 5 回そろうまで，最大 15 回まで試行を続ける．15 回でそろわなければ今どおり `unmeasured`．要る成功の数と，目標をまたぐ試行（`Ambiguous`）を数えないことは変えない．3e の分類で 2026-10-27 の 12 回の読みは成功 6・あいまい 3・不完全 3 で，5 回続けての成功は見込めなかった．

| 項目 | 中身 |
|---|---|
| 計画 | `MeasurementPlan.queue_max_trials`（既定 15，env `EZSDR_MEASURE_QUEUE_MAX_TRIALS`，`run-record.sh` が渡す）．`queue_trials` は要る成功の数．validate は上限 < 要る数を拒み，record の u32 の確かめは上限の側 |
| loop | 成功が要る数に達するか上限で止める．装置の失敗・`raw.halted()`・交互の遅れは今どおり |
| 値と記録 | 値と条件は成功だけから．record の `trials` は値を決めた条件で実際に流した数．observation に条件ごとの `trial_count`・`success_count`，`required_successes`・`max_trials`．`unmeasured` の理由に条件ごとの数 |
| 文書 | UR-34 と *Changes*，`bench.md`（所要 2〜7 分は INFERRED） |

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| 実装 | Sonnet 5.5．セルフレビューなし（小さな段階） | fake は成功を出さないので，止まる側は純粋な試験だけで見る |
| Sol コード | | FAIL：major 1（Measured の `trials` が成功の数） |
| 修正 1 | 実際に流した数に．混ざる 8 試行の試験，P1044 | |
| Sol 見直し | | PASS |

## 3. 数

| 出所 | major | minor | 機構 |
|---|---|---|---|
| Sol（コード） | 1 | 0 | 数の取り違え（試験が誤った数を固めていた） |

## 4. gate

- テスト 1956 全部通った．clippy，`rules.py` 0 件，docker の uhd（lib 433，`uhd_api` 5，`--features uhd` の clippy），Python 3.14・3.9 各 57．
- mutation の全リスト 1767/1768（G09 は既知．修正 1 の前の tree）．修正 1 で触った P811，P1038，P1039 と新しい P1044 は個別に流して kill．
