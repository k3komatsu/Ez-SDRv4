# 段階 3e：orchestrator のレビュー，裁定，数

## 1. 何をしたか

実機の準備の小さな段階（risky でない：試験と bench の道具．本番の `src/` は `uhd.rs` の 2 か所だけ）．入力は [flaky-tests-1027](flaky-tests-1027.md)，[bench-dry-run-1027](bench-dry-run-1027.md)，D9a の MA-1（owner の決め 2026-10-28）と同時運転の扱い（owner の決め 2026-10-28），送信の throughput の調査．Sol の事前確認（判定 CHANGE）を指示書に入れてから実装．

| 項目 | 中身 |
|---|---|
| 揺れるテスト | `ur_25` の 3 本は遅れた burst の `time_errors` の assert を外す．`ur_22`（短く切られた send）と `ur_21` の 2 本（untimed の send，cold change の後の untimed の burst）の時刻どおりの性質は，時刻を制御した unit test（`provider/tx.rs`，`provider/control.rs` の `mod tests`）へ．fake 側はテストの thread の時機に判定が依らない形だけ残す．server の loopback は MockRadio の模擬の時計なので変えない |
| queue の深さ | 目標をまたぐ区間は `Ambiguous`（成功に数えない）．値は成功だけから．呼び出しの上限 512．試行ごとに遅れを交互に変え，止まった呼び出しの番号が一致するかを observation に．条件ごとに成功 5 回の要件は残す（実機で `unmeasured` になりうる：owner に） |
| 同時運転 | 値の段から外す．両方の値が決まったら，それぞれの値を持ちこたえた stream・channel・rate で 1 組だけ流し，値は決めない（observation `provider_duplex_at_the_measured_limits`）．片方が `unmeasured` なら流さない |
| 容量の行 | T で成功した条件の実際の ID（止めた条件の rate を除いた系列から）．止めた条件は別に記録 |
| PLL・段の失敗 | UR-7 の再試行を使い切ると記録の latch を立てる．raw の `set_sources` も同じ再試行と `REFERENCE_DID_NOT_LOCK` への変換．段の `Err`・panic は前の段を残す |
| `uhd.rs` | streamer を開く 2 つの呼び出しのエラーの文を streamer の handle から読む（UHD 4.10 は `UHD_SAFE_C_SAVE_ERROR(h_s, ...)`）．`lost` の判定は不変 |
| MA-1 | UR-24 の Ceiling，[KL-32](../../../KNOWN_LIMITATIONS.md)，probe `hw_ma1_phase_after_a_pre_start_tune`（送信は 0 dB・振幅 ≤ 0.5，att の確認） |
| bench | `run-record.sh` の `--tmpfs /tmp`・`CARGO_HOME`・`bench-host.txt`．`bench-host.sh` は governor・C-state・NIC の IRQ・coalescing を読むだけ．`bench.md` に机上の通しの F11 の直し，実機の日の手順，`benchmark_rate` の 200 MS/s の 3 通り，送信の `burst_blocks` の読み方 |

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| Sol 事前確認 | 指示書 | CHANGE（queue の判定の向き，同時運転の 1 組の rate，位相の probe の比べ方，PLL のエラー文の形） |
| 実装 | Sonnet 5.5．セルフレビュー 1 回（free は上限，Go の DeepSeek：P1 1，P2 3，P3 6） | P1（Session には `at` が無い）ほかを直した．揺れるテスト 3 本は退けた（後で Sol が覆した） |
| Sol コード | | FAIL：major 4（送信 buffer の容量は DEBUG では出ない，`blocks` が記録に無い，`ur_21` の揺れ，容量の行の条件の照合が rate つきの ID），minor 3 |
| 修正 1 | 7 件 | |
| Sol 見直し 1 | | FAIL：major 1（image の libuhd は `UHD_LOG_MIN_LEVEL=1` で TRACE が build に無い） |
| 修正 2 | orchestrator が `bench.md` の段落を直した（image で確認） | |

## 3. 数（AGENTS.md §9 step 8）

| 出所 | major | minor | 機構 |
|---|---|---|---|
| Sol（事前確認） | — | — | 指示書の段で 4 件 |
| Sol（コード，2 回） | 5 | 3 | UHD の事実と手順の食い違い 2（log の level，build の level），記録に無い証拠 1，`wall-clock-test-oracle` 1（`ur_21`，既知），試験の fixture が実際の ID と違う 1 |
| セルフレビュー | 1 | 9 | Session と Spec Run の違い 1 |

`wall-clock-test-oracle` の `ur_21` は 2026-10-27 の調査で既に数えた 4 本のうちの 2 本で，新しい数ではない．設計メモ 37 の規則（判定がテストの thread の時機に依らない）どおりに直した．

## 4. gate

- テスト 1932 全部通った．clippy，`rules.py` 0 件，docker の uhd（lib 433，`uhd_api` 5，`--features uhd` の clippy），Python 3.14・3.9 各 57．
- mutation の全リスト 1745/1746（G09 は既知）．
- 時刻に依る試験は 20 回で失敗 0（実装役）．

## 5. 残したこと

- **owner に**：queue の深さの成功 5 回の要件（実機で `unmeasured` になりうる）．
- 送信の 100/200 MS/s：調査（`~/.cache/ezsdr-fixes/2026-10-26-stage3/tx-throughput-report.md`）の推奨は uhd-tx の実時間の優先度と窓の端の最小の send．owner の判断待ち．
- `tests/README.md` に規則の 1 行を足すこと（揺れるテストの調査の提案）．
