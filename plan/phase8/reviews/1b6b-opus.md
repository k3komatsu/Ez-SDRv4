# 1b-6b：orchestrator のレビュー，裁定，数

## 1. 何をしたか

設計メモ [35](../35-uhd-stream-state.md) の R2．UHD Module の stream ごとの `idle`・`active`・`delivered` を，private な `Phase { Idle, Pending, Active { segment, delivered } }`（`crates/ezsdr-radio-uhd/src/provider/core.rs`）にし，`Streams` の遷移の関数（`starting`，`search`，`started`，`delivered`，`finished`，`plan_read`）だけが変える．読むのは `is_idle`・`active`・`first_undelivered`（と test の観測の `phase`）．

- `delivered` はメモの `Option<i64>` でなく HEAD の組 `(origin, config, k)` を残した（orchestrator の決定）．End の `cuts` の照合は HEAD のまま．
- 挙動は変えない．HEAD と書き方が違う所（`finished` の後は `Pending`，`search(false)` が `Active` を消す，`Rx::new` は stream が無いと phase に触らない，`Active` でない時の `delivered` は捨てる）は，production では届かないか同じ値になる．Opus が呼び出し元で確かめた：RX の stream を閉じる道は `Rx::finish` だけ（`self.stream.take()` は 1 か所），`Control::new` の `start.rx[k]` と `Rx::new` の `first` はどちらも `Provider::start` の `rx_first` から来る．
- spec 18 の UR-17・UR-25（phase の定義）・UR-26 の文と `*Test:*`，Changes．
- テスト：新しく 8 本（thread を使わない．`Streams` の unit 5，`Rx` 1，`Tx` 2）．既存テストで field を直接書いていた所は遷移の呼び出しに，読んでいた所は観測に置き換えた（削除・改名なし）．
- mutation：6 行を当て直し，9 行（P552〜P560）を足した．P472 は負荷で揺れた fake の thread テストから，決定的な unit test `ur_26_a_finished_stream_clears_only_its_own_floor` に移した（[1b5c1-opus](1b5c1-opus.md) §6 の持ち越し）．

本体とテストで約 +680／−150．実装は DeepSeek V4.1 Flash（`opencode/deepseek-v4.1-flash-free#max`），セルフレビューも同じモデル（1 回，P1 なし）．free の run は常に 1 本で，上限には当たらなかった．

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| テスト計画 | DeepSeek，14:04〜14:30 | 系列 P1〜P11，新しいテスト 7 本，食い違い 4 件（メモ R2 と HEAD） |
| Sol の計画レビュー | 自分の失敗の筋書きから | **FAIL**，major 3・minor 2 |
| 計画の修正と実装 | DeepSeek，14:34〜16:10 | セルフレビュー 1 回（P1 なし，P2 1・P3 3 を直した），触った 27 行 27/27 killed，lib を 20 回続けて流して 20/20 |
| Opus | 指示書への忠実さ，変えてはならない挙動 | major なし，minor 1 |
| Sol のコードレビュー | 自分の失敗の筋書きから | **FAIL**（major なし），minor 1 |
| 修正 | Opus が直した（mutation の 1 行，Changes の 1 行） | |
| gate | §4 | 1269/1270（G09 は既知） |

## 3. 指摘と扱い

計画レビュー（Sol，すべて採用）：
- **major 1（P6）**：既存の送信のテストは 3 個目の segment の `Active` で終わり，後続の無い拒否で `Idle` になる道を見ていなかった．N8 `ur_25_a_refused_last_transmit_change_ends_idle` を足し，既存テストの最後も `Active` の segment まで見る．
- **major 2（P4）**：拒否からの再開は `by: Some(2)` で，configure 中の loss の後の `Pending` を見ていなかった．両方の assert を足した．
- **major 3（P1b）**：T0 の segment が loss で消える道を「到達しにくい」としていた．`Provider::start` は RX を 1 本ずつ spawn するので届く．N3 で T0 の line に loss を実際に book してから `Rx::new` を通す．
- minor 2：N7 は Abort の `End` で見る．N4・N5 の「殺す」と書いた誤実装を実際に殺す形に．`Rx::finish` の `finished` の配線を殺す行（P559）．
- 計画の食い違い 4 件（TX の初期化は `Pending`，RX の初期化は 2 段，`delivered` の組，clock があり segment が無い `plan_read`）は HEAD を採った．

実装の後：
- **O1（minor）**：spec 18 の Changes の新しい行が前の行と形が違った（小文字始まり，規則の ID の繰り返し，"Versions are unchanged." が無い）．Opus が直した．
- **Sol（minor）**：P472 の当て直しが「他の stream の floor も消す」誤実装でなく，phase を変えずに `None` を返す変異になっていた．N7 の戻り値の assert で落ち，Abort の cut まで行かない．P470 と同じ「全 RX stream の phase を消す」変異に戻した（filter は N7 のまま）．
- **process**：今回は計画の修正節を実装の前に書いた（1b-6a では後だった）．

## 4. gate

テスト 1578，clippy，`rules.py` 0 件，docker の uhd（clippy，lib 267，`uhd_api` 5），Python 3.14・3.9 各 56，mutation の全リスト 1269/1270（G09 は既知．P472 と P552〜P560 は killed）．この段階で壊れた行は無かった．

## 5. 数（AGENTS.md §9 の 8）

| 出どころ | major | minor | 機構 |
|---|---|---|---|
| Sol（テスト計画） | 3 | 2 | テストの観測の穴．機構なし |
| セルフレビュー（DeepSeek，1 回） | P1 0 | P2 1・P3 3 | 文とテストの準備．機構なし |
| Opus | 0 | 1 | 文の形．機構なし |
| Sol（コード） | 0 | 1 | mutation の行の意図．機構なし |
| gate（mutation） | 0 | — | — |

計画を先に書く手順（AGENTS.md §9 の 2・3）は，1b-6a と同じく計画レビューで major 3 を拾い，コードレビューでは major 0 だった．設計メモ 35 の R1・R2 はこれで済み，`stream-lifecycle-scattered` の印のうち `StreamState` の分は遷移の関数の中に入った（device の記録と loss はメモの範囲外）．
