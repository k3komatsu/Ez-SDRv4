# 段階 3c-1：orchestrator のレビュー，裁定，数

## 1. 何をしたか

設計メモ [41](../41-measurement-records.md) §8 の 3c-1：Provider を通した測定の群を 3 つ，`crates/ezsdr-radio-uhd/tests/common/measurement.rs` に足した．

| key | 測り方 | 読み方 |
|---|---|---|
| `radio.timing.tx_start_lead_ns` | 候補 S ごと，条件（1 ⇄ 2 MS/s，390 625 S/s ⇄ 1 MS/s）ごとに Session を 1 つ．送信の rate を `cold` で交互に変える | 新しい segment の `tx_switch` の行で `at < origin`，試行の区間に失敗の証拠なし．成功の suffix，2 条件の交わり |
| `radio.timing.overflow_restart_gap_ns` | rate（1，10，25，50 MS/s）ごと，試行ごとに Spec の受信の Run を 1 つ．`with_rx_stall` で overrun を 1 回 | **値にしない（`unmeasured`）**．gap 全体と，読み直しから再開までの推定（INFERRED）を observation に |
| `radio.perf.tx_bytes_per_s` | rate（10〜200 MS/s）ごとに送信の repeat の Run を 1 つ．burst の始まりから D 秒 | underflow・SeqError・TimeError・short send なし，`BurstAck` がちょうど 1 つで始まり ＋ D 以後，`tx_samples` ≥ D × rate．容量 × `wire_bytes_per_sample` |

送信の start lead の成功に，新しい clock の origin に置いた burst が on time であることは入れていない．burst を予約できるのは試験のスレッドが新しい SampleClock を見た後で，1 ms の候補では origin に間に合わないことがある（指示書が許した省略）．`tx_switch.at` は構成の終わりと clock の採用で，送信した burst の証拠ではない（observation に明記）．

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| 実装 | Luna fast（23:35〜02:04） | risky でない扱い（テスト計画なし）．セルフレビュー 1 回（free の DeepSeek が上限で，Go の DeepSeek．P1 1，P2 4，P3 4．全部直すか理由つきで退けた） |
| Opus | | O1（major：overflow の値が stall を含む），O2（note：`tx_samples` の数え方），O3（note：bench の時間） |
| Sol コード | | FAIL：major 3（送信の SampleClock は Stop で終わらないので throughput が必ず不完全，送信の数を比べる区間がずれる〈clock の origin は arm，repeat は T0，in-flight の分〉，overflow の値が rig に依る＝O1） |
| 修正 | Luna fast（02:21〜03:51） | 全部．overflow の key は `unmeasured` にし，既定の試行数を 5 に．RX の packet の大きさを読む Session をやめた |
| Sol 見直し | 実装役が自分で起こした（下） | 元の major 3 は CLOSED．minor 1（ack が始まりと始まり ＋ D の間の場合のテストが無い）→ 実装役が足した（P806） |

Sol の見直しは，修正の指示書が「セルフレビューは回さない（Sol が修正を再レビューする）」と書いたのを，実装役が自分で Sol を起こせと読んで起こした．問いは読むだけで，元の指摘ごとに CLOSED／OPEN と近くの道を見るもので，orchestrator が書く見直しの指示と同じ形だったので，その結果を見直しとして使った．次の指示書では「Sol の見直しは orchestrator が起こす」と書く．

## 3. 裁定の要点

- **overflow の restart gap**：Sol が UHD 4.10 の source で確かめた（VERIFIED）．受信の再開は reader の `recv` が溜まりを読み切って overrun を見た後に始まる（`rx_streamer_zero_copy.hpp` の `get_recv_buffs()`，`rfnoc_rx_streamer.cpp` の `_handle_overrun()`）．だから stall で起こした overrun の gap 全体は stall と host の buffer の関数で，装置の性質でない．読み直しの時刻は記録に無い（`rx_stall` の timing の行に時刻が無い．`src/` は変えない）ので，capture の最初の sample ＋ `after` ＋ `stall` で推定した「読み直しから再開まで」を observation に残す．stall の起点は host が最初の block を受け取った時刻なので推定は早めに出て，値は大きめ（溜まりを読み切る時間を含む）．これを profile の `overflow_restart_gap_ns`（MR-21 が overrun ごとに失う長さ）とみなすかは，最初の bench の数を見て owner が決める．
- **送信の throughput**：underflow の無い連続の burst では装置が流した数は host が渡した数と等しいので，上側の許容は要らない．D 秒以上流れたことは `BurstAck` の tick で見る．

## 4. 数（AGENTS.md §9 step 8）

| 出所 | major | minor | 機構 |
|---|---|---|---|
| Sol | 3 | 1 | どれも新しい測定の道具の中（送信の clock の終わり，数を比べる区間，rig に依る値）．`failure-mechanisms.md` の機構には当てない |
| Opus | 1 | — | O1 は Sol 3 と同じ |
| セルフレビュー | 1（P1） | 8 | P1 は overflow の Run の長さが capture の終わりに足りない（直した） |

3b-2 と同じく，risky でない扱いの段階でコードのレビューに major が出た．どれも実機の Provider の振る舞い（Stop で終わらない送信の clock，arm と T0 の差，UHD の再開の起点）を読まないと見えない形で，FakeDevice の試験では出ない．3c-2 の指示書では，成功の条件が頼る Provider の記録（どの section のどの行が何時に書かれるか）を `path:line` で書く．

## 5. 残したこと

- bench の時間（3c-1 の 3 つ）：Run 41 本，装置の時間で約 13 分（INFERRED，Run の始まりと片付けを除く）．3b-2 の約 15 分と合わせて約 28 分．
- `BurstAck` の tick が実機で付くか（付かなければ送信の throughput は不完全）：最初の bench で読む．
- overflow：読み直しから再開までの推定を profile の値とするかは bench の後に owner が決める．
