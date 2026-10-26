# 段階 3a：orchestrator のレビュー，裁定，数

## 1. 何をしたか

設計メモ [41](../41-measurement-records.md) §2〜§4：測定記録の型 `ezsdr_radio::measurement::Record`（schema `schemas/radio/measurement.v1.json`，型から生成し `schema_freeze` に入れた），key ごとの扱い `rule(key)`（下限，上限，包含，倍数，保証の強さ，真なら真，一致，比べない），description から key ごとの profile の値を出す関数（周波数・利得は格子全体），純粋な比較 `compare(record, description)`．比較テスト `crates/ezsdr-acceptance/tests/measurements.rs` は `crates/ezsdr-radio/measurements/` の記録を，MockRadio と UHD Module それぞれの terms と block_len で作った Description と比べる（記録はまだ無い）．MockRadio の `Profile::terms()` と `BLOCK_LEN` を公開した（値は変えない）．spec 07 に RM-35，RM-20 の schema の数．risky でない段階として，テスト計画を省いた（AGENTS.md §9）．

ついでに直したもの：設計メモ 40 の後，壁時計で一度落ちた `ur_21_a_moved_start_on_a_held_burst_s_is_refused` を手動の時計に移した（設計メモ 40 で手渡しが窓の端まで細かく進み，読んだ cursor と予約の間に進むと落ちる）．

実装は GPT-6 Luna（OpenAI），セルフレビューは free の DeepSeek（4 回）．

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| 設計メモ 41 | Opus | Sol FAIL（major 7，minor 1．全部採って書き直し）→ PASS |
| 実装 | Luna | セルフレビュー 3 回（3 回目に P1 1：格子の内側の値の欠けを見ないテスト） |
| Opus | | O1（minor：比べない `block_len` を UHD の selector から読み直す），O2（note：倍数の浮動小数の比較） |
| Sol コード | | FAIL：major 2（2^53 を超える整数の丸め，ファイルの道で片方の Provider の判定を消しても落ちない），minor 2（1 点の格子の包含，計画に無い「測った値が 1 つも無い記録は拒否」） |
| 修正 1 | Luna | セルフレビュー 1 回（P1 0） |
| Sol 見直し 1 | | FAIL：major 1（gate の assert を消しても落ちない），minor 1（RX と TX の数の取り違え） |
| 修正 2 | Luna | 上の 2 つと，壁時計の UR-21 のテスト |
| Sol 見直し 2 | | **PASS** |

## 3. 数（AGENTS.md §9 step 8）

| 出所 | major | minor | 機構 |
|---|---|---|---|
| Sol | 3 | 3 | どれもテストの力（判定を消しても落ちない）か比較の角（丸め，1 点の格子）で，`failure-mechanisms.md` の機構には当てない．2^53 の丸めは今の x3xx の値では起きない |
| Opus | 0 | 1 | O1：同じ selector の読みを 2 か所に書いた（`radio-rule-implemented-twice` の形だが比較の結果に効かない値で，数に入れない） |
| 壁時計のテスト | — | 1 | `wall-clock-test-oracle`（設計メモ 37 の掃除の漏れ．設計メモ 40 で起きやすくなった） |

## 4. gate

テスト 1768（失敗 0），clippy，`rules.py` 0 件，docker の uhd（clippy，lib 388，`uhd_api` 5），Python 3.14・3.9 各 57，mutation の全リスト 1434/1435（G09 は既知）．
