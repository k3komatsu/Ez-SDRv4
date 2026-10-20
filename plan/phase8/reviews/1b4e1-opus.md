# 1b-4e-1：orchestrator のレビュー，裁定，数

## 1. 何をしたか

計画の 1b-4e（moves と複数 stream の VH-8 harness）を 2 つに分けた（orchestrator，2026-10-20）．

- **1b-4e-1**（これ）：VH-8 の harness（`crates/ezsdr-acceptance/tests/parity.rs`）に stream の次元を足す．テストだけで本体を変えないので risky ではなく，計画と計画レビューは省いた．
- **1b-4e-2**（次）：UHD Module の moves（RM-34）．streamer の持ち主が stream から stream へ移るので risky．harness が先にあれば，moves の差を Mock との比較で拾える．

足した比較（指示書 §3 の 8 つと `sequence`）：2 本ずつの layout の prepare，stream 1 だけの rate 変更，stream 1 への join，channel の無い stream 1 の最初の segment，送信 stream 1 の clock，instance 全体の床，stream 1 の拒否，Manifest の plan の行．時刻はそれぞれの T0 からの相対．

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| 実装 | DeepSeek，18:41〜19:32（約 50 分，セルフレビュー 1 回を含む） | セルフレビュー：P1 なし（P2 1，P3 6．すべて直した） |
| Opus | 指示書への忠実さ，divergence の裁定 | 裁定 1（下）．その場で直した |
| Sol のコードレビュー | 自分の失敗の筋書きから | **PASS**，minor 2（下）．その場で直した |
| gate | テスト，clippy，`rules.py`，mutation の全リスト | 下の §4 |

## 3. 指摘と扱い

- **Opus 裁定 1（`sequence`）**：DeepSeek は Manifest の `sequence` を「Mock は 0 から，uhd-control は 1 から」の divergence として `#[ignore]` にした．MR-27・UR-30 は「segment を始めた command の arrival order」とだけ言い，番号の起点を定めていない．しかも起点はそれぞれの Provider の数え方による．Mock の `next_order` は fault の数から始まる（`ezsdr-mock-radio/src/lib.rs:2527`），uhd-control の `seq` は 0 を loss に使う（`provider/core.rs:298`）．だから spec の違いではない．比べるのは番号の値ではなく順位にした（`vh_8_the_same_manifest_sequence`，ignore を外した）．design の文は変えない．Sol も「MR-27／UR-30 は固定の番号起点を要求していない」として裁定を妥当とした．
- **Sol minor 1**：`row["sequence"].as_u64()` は，欠けた field も文字列も `None` にするので，最初の segment の `null` が壊れても通る．`sequence_of` で field の存在を要求し，`null` 以外は整数を要求するようにした．
- **Sol minor 2**：`Started::cold` の queue の待ちだけが `std::thread::sleep` で，実時間 1 秒で失敗しうる．device の host clock（control thread が走っている間は止まる，UR-33）で待つようにした．
- **DeepSeek の穴（採る）**：送信 stream 1 の arm 時の clock は比べていない（Mock は arm を root 0 に置き，UHD は arm の実時間ぶん後になる．arm の瞬間を rig から取る仕組みが要る）．stream は 0 と 1 の 2 本だけ．

## 4. gate

テスト 1516，clippy，`rules.py` 0 件，mutation 1217/1218（G09 だけ，既知）．Sol の修正の後，`parity` を kill 役に持つ 6 行（P485〜P488，P333，P352）を流し直して 6/6 killed．docker の uhd と Python は省いた（変えたのはテストの crate だけ）．

## 5. 数（AGENTS.md §9 の 8）

| 出どころ | major | minor | 機構 |
|---|---|---|---|
| セルフレビュー（DeepSeek，1 回） | P1 0 | P2 1，P3 6 | テストの観測と重複．機構なし |
| Opus | 0 | 1 | テストの比べ方（番号の起点）．機構なし |
| Sol | 0 | 2 | テストの検出力と負荷への弱さ．機構なし |

`failure-mechanisms.md` に足すものはない．

## 6. DeepSeek V4.1 Flash（4 段階目の観察）

- 実装は約 25 分，セルフレビュー込みで約 50 分．指示書の「P1 が無くなったら次の回は流さない」を守った（前の段階までは P1 なしでも 3 回まで回していた）．
- divergence を見つけたとき，本体を直さずに ignore と報告に回した（指示書どおり）．ただし「どちらが spec に合うか決められない」とし，起点が Provider ごとの数え方であることまでは踏み込まなかった．
- 指示書に無い仕組み（arm の瞬間を取る仕組み）は足さずに穴として書いた．

## 7. 1b-4e-2 へ送るもの

- 受信の owner も送信の owner も，次の segment が無いと `idle` を立てるだけで streamer を閉じない（`rx.rs:287`，`tx.rs:220`）．だから channel を `-1` にしてから別の stream に入れる今の経路でも，残った streamer と FakeDevice の `Attempting to reconnect ... port!`（UR-33）がぶつかるはず（INFERRED，未テスト）．1b-4e-2 のテスト計画の最初の系列にする．
