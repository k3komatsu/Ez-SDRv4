# 段階 3b-1：orchestrator のレビュー，裁定，数

## 1. 何をしたか

設計メモ [41](../41-measurement-records.md) §5 の前半：測定のための候補の timing と，記録を作る道．

- `UhdRadio::with_measurement_timing(timing)`：bench の Rust のコードだけが呼ぶ builder（`with_rx_stall` の前例）．Description の timing を候補にし，capability の tree，`ProviderInstance.min_command_lead`，envelope の section を作り直す（`from_binding` と共通の `description_surfaces`）．Module の内部の定数 `DEVICE_LEAD_NS`・`DELIVERY_ALLOWANCE_NS` は変えない．builder を呼ばない構築は今のまま．
- 実行時の読み手が候補を読むことを制御された時計で確かめた：start-up の境界（`arm`），方向ごとの start lead（`cold` の rate change の origin），RX の missed start，release window，queue depth，in-flight の境界と chunk．`overflow_restart_gap_ns` は UHD Module に読み手が無いので伝わり方だけ（意味は 3c）．
- `tests/common/measurement.rs`：metadata（`EZSDR_MEASUREMENT_*` 8 つ．無い・空・不正・既存の出力先・親の無い出力先は USRP を開く前に失敗），上書きしない書き出し，記録を組む関数（比べる key はすべて `unmeasured` と理由），raw の probe の observation，試行の 6 分類と帰属（span は一つの試行に収まるときだけ），読み方（最小の lead は成功の suffix，容量は成功の prefix，試行の不足は不合格，条件ごとに求めてから交わり）．
- raw の probe 9 本（`hw_b8_raw_*`，`hw_b6_delay_by_rate`，`hw_b8_stop_end`）が observation を残す．ignored の `hw_measurements_record` がそれを走らせて `source: "hardware"` の記録を書く．FakeDevice のリハーサルは Run の burst と capture から observation を作り，書いて読み戻し，`compare` の違反が `source` だけ．
- design/18 UR-34 に builder の 1 文と Test，UR-17 の Test．

Provider を通した測定の群（最小の command lead，in-flight，受信の start lead，throughput，full duplex）は 3b-2．

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| テスト計画 1 | Luna | Sol FAIL（major 7）→ 段を 3b-1 と 3b-2 に分けた |
| テスト計画 2（3b-1） | Luna | Sol FAIL（major 2，minor 1）→ orchestrator が指示書に裁定（系列 3 を読み手ごとに具体に，系列 4 は Kernel の KC-19 が見ているので消す，系列 11 を Run の証拠に結ぶ） |
| 実装 | Luna（14:05〜18:02） | セルフレビュー 1 回（free の DeepSeek，P1 0） |
| Opus | | minor 4（O1 builder が `from_binding` の導出を写す，O2 条件をまたぐ読み方，O3 出力先の確認が最後，O5 文言），note 1（O4 リハーサルの assert が作った値と比べるだけ） |
| Sol コード | | FAIL：major 3（O2 と同じ読み方，区間をはみ出す span で成功，O3 と同じ出力先），minor 1（P734 の変異が名前と合わない） |
| 修正 | Luna（18:07〜18:27） | 全部．セルフレビューなし |
| Sol 見直し | | **PASS** |

## 3. 数（AGENTS.md §9 step 8）

| 出所 | major | minor | 機構 |
|---|---|---|---|
| Sol（計画） | 9 | 1 | 計画の段階．測るものの切り分けと「何を観測すれば成功か」．`failure-mechanisms.md` の機構には当てない |
| Sol（コード） | 3 | 1 | 読み方の交わりの順，span の包含，bench の時間の無駄．どれも新しい測定の道具の中で，機構には当てない |
| Opus | 0 | 4 | O1 は `radio-rule-implemented-twice` の形（同じ導出を 2 か所）だが，同じ crate の中の写しで helper に寄せた．数に入れない |

計画の段階で major が多く出て，コードの段階の major 3 はどれも計画に無かった角（交わりの順，span の端）か bench の運用（出力先）だった．

## 4. 残したこと

- O4：リハーサルの observation は Run の証拠から作り，作った値と比べている．意味を持つのは読み戻しの後の assert と `compare`．3b-2 で `measure_*` が入るとき，リハーサルはそれを通す．
- 実機では何も流していない．FakeDevice の結果は実機の証拠ではない．
