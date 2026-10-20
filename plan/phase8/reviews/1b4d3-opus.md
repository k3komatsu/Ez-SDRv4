# 1b-4d-3：orchestrator のレビュー，裁定，数

## 1. 何をしたか

event と Manifest の記録に stream の番号を持たせた（plan 33 の「1b-4c で 1b-4d へ送るもの」(4)，(8)，(9)，設計 33 D3 の 6）．risky ではない段階（記録の形とテスト）なので，テスト計画と Sol の計画レビューは省いた．

- `Streams::record` の `timing.plan` の行を `{ stream, sequence, origin, end, channels, ticks_per_sample }` にした．`channels` は論理 channel の列（block の順）で，数を置き換える．`sequence` は segment を始めた command の arrival order（無ければ `null`）．名前は Mock の MR-27 の `segments` に合わせた．別の `segments` section を足す案は退けた（UHD では segment が 2 か所に書かれる）．
- `device_async` の行に `stream`（`channel` は streamer の中の位置なので，2 本の送信 stream では足りない）．
- UR-25 の拒否の隔離のテストを，予約された変更を持つ line で作り直し，layout の 2 本の受信 stream の版を足した．
- 2×2 の layout の `Provider::stop` の後の plan の行を 1 本の e2e で見る．
- `tests/fake.rs` の繰り返し（section を引く式 46 か所，拒否の理由 12 か所）を helper にした．

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| 実装 | DeepSeek，14:13〜16:20 ごろ（約 2 時間，セルフレビュー 3 回を含む） | セルフレビュー：3 回とも P1 なし |
| Opus | 指示書への忠実さ，変えてはいけない挙動，design の文 | minor 1（下）．その場で直した |
| Sol のコードレビュー | 自分の失敗の筋書きから | **PASS**，minor 1（下）．その場で直した |
| gate | テスト 1507，clippy，docker の uhd，Python 3.14・3.9，`rules.py`，mutation の全リスト | mutation 1213/1214（G09 だけ）．P442 の kill 役 `ur_26_every_stream_s_threads_are_joined_in_order`（4 本の thread がそれぞれ join の 1 s を待つ）は，sim02・sim03 が他の計算で埋まっていた間，4 CPU の shard で素の tree のまま落ちた．8 CPU で流し直して kill．負荷に弱いテストの候補として残す |

## 3. 指摘と扱い

- **Opus minor 1**：design/18 の Changes の引用が「RM-27's `segments`」．`segments` は Mock の MR-27（spec 07 の RM-27 は layout の key の宣言）．MR-27 に直した．
- **Sol minor 1**：helper `rejection_reasons` が `filter_map` で `reason` の無い拒否を落とすので，件数の検査が弱くなる（`ur_13_...` などで以前は拒否の event 自体を数えていた）．helper の側で `reason` の無い拒否を panic にして，9 か所の呼び出しすべてで件数 = 拒否の数に戻した．
- **TX の `end`**：`Provider::stop` の後，TX の最後の segment の `end` は `null` のまま（stop は burst を終え，clock を終えない）．DeepSeek は報告の穴に書き，Sol に判定を頼んだ．Sol：今の規則（RM-16，`timeline.rs:849–855`）と合っており，この段階で閉じる穴ではない．採る．
- 範囲外として残したもの：`rx_segment` の timing 行の `channels` はまだ数で stream 番号を持たない（`rx.rs:326`．決定は plan の行だけ）．e2e は Provider の実 thread と `cleanup` を通るが，Session の Manifest ファイルまでは通さない（VH-8 の harness は 1b-4e）．

## 4. 数（AGENTS.md §9 の 8）

| 出どころ | major | minor | 機構 |
|---|---|---|---|
| セルフレビュー（DeepSeek，3 回） | P1 0 | P2 2，P3 14 | テストの観測と design の文．機構なし |
| Opus | 0 | 1 | 引用の誤り（機構なし） |
| Sol | 0 | 1 | テストの検出力（機構なし） |

`failure-mechanisms.md` に足すものはない．

## 5. DeepSeek V4.1 Flash（3 段階目の観察）

- 約 2 時間でセルフレビュー 3 回まで．小さな段階では速い．
- 指示書の「触ったコードに当て先がある既存の行も流す」を守り，新しい 5 行と既存の 7 行を流した（12/12 killed）．gate の全リストで当て先を失った行は出なかった．
- 指示書に無い仕組みは足さなかった．判断が要る点（TX の `end`）は足さずに穴として書いた．
- セルフレビューは今回も P1 が無いまま 3 回目まで回した（上限内）．
