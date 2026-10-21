# 1b-5a：Sol 6.1 のコードレビュー

## 1. コードレビュー

判定：FAIL

1. **major — channel／stream の一覧が binding の selector に依存する**
   - **箇所:** `python/ezsdr/session.py:206–211,243–246,399–404`、`design/16-easy-api.md:86`
   - **根拠（VERIFIED）:** `_channel_count()` が selector の `rx`／`tx` を解析し、stream 数も channel 数から決めています。EA-18 の binding 内容を名指ししない規則に反します。欠落キーの例外文（同ファイル203行）も selector の設定を指示しています。
   - **再現:** selector が空で、effective に RX c0・c1／s0・s1、TX c0／s0 がある mock 応答を与えると、一覧の長さは期待する `2,2,1,1` ではなく `1,1,0,0` となり、存在する TX s0 の別名も `Error` になります。現行の組込み profile で同じ不一致が起きることは未確認です。
   - **修正:** effective を一度取得し、方向別の `c<n>.stream` と `s<k>.sample_rate_hz` から、それぞれの一覧を作ってください。所属一覧も同じ応答から導出します。selector を案内する例外文と spec の記述を除き、selector に依存しないテストへ P545／P547 を更新してください。

**確認結果**
- server protocol：**43 passed**
- Python suite：同じ tree から review target に build した server で **55 passed**
- 全6 mutation リスト、計 **1,275行**の `old` が、それぞれ対象ファイルにちょうど1回一致。
- P528–P547 は指定テストの assertion と照合済み。ただし実際の kill は **INFERRED**。
- stream 別の宛先・recorder・時計、別名、所属の再読込、HD-10、aligned の共有時刻には、上記以外の不具合を確認しませんでした。

未実施：mutation 実行、実機試験、workspace 全体の gate。ファイルは変更していません。

## 2. 修正の回 1 の再レビュー

判定：FAIL

1. **minor — Vision §3 の sample rate が別名に戻っていない**
   - **箇所:** `design/vision/01-purpose-and-core-boundary.md:67`
   - **根拠（VERIFIED）:** `sdr.rx_streams[0].sample_rate = 20e6` が残っています。`fix1-brief.md:15` は、この例も `sdr.rx.sample_rate` のままにするよう明示しています。
   - **シナリオ:** §3 の第2例だけ、残すと決めた簡潔な別名を使っていません。実行上の不具合ではありません。
   - **修正:** この1行を `sdr.rx.sample_rate = 20e6` に戻してください。

**前回 major の修正：確認済み**
- 一覧と所属は effective のキーだけから導出。`python/ezsdr` に selector の読取りはありません。
- 再現結果は長さ **2,2,1,1**、`Radio.tx` は **TX s0**。
- 保持した `Radio` でも所属変更を再読込みします。一覧自体は構築時のものですが、所属変更では channel／stream のキー集合は変わらないため問題ありません。
- stream キー欠落時は channel 数から補完せず、直接その属性を読むと所定の `Error`。`Session.radios()` も effective から識別します。
- 欠番の合成応答では番号を保持しますが、リスト位置とは一致せず、s0 がなくても先頭が別名になります。ただし RM-28〜RM-30 と builder は連続番号・全キーを保証するため、適合する現行 Provider の不具合とはしません。
- stop／PPS の docstring、例、spec 16 の一覧の出所は、上記1行を除き修正済みです。

**検証結果**
- Python：**56 passed**
- server：**43 passed**
- 全6 mutation リスト **1,276行**の `old` が各対象にちょうど1回一致。
- P533／P544／P545／P547／P548 は assertion と照合し、remote ログでも **5/5 killed** を確認。独立した再実行での kill は未確認（**INFERRED**）。
- `git diff --check`：問題なし。

未実施：mutation 再実行、実機試験、workspace 全体の gate。ファイル変更なし。
