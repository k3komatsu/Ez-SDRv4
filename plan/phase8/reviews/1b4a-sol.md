指定の brief に従い、変更・ビルド・mutation 実行はせず、HEAD 差分と UHD 4.10 の一次ソースを照合します。結果は日本語で報告します。
UHD ソースとの照合で、異常な subdev markup のエラー経路に handle leak を確認しました。また、空の channel list は FakeDevice と UhdDevice で意味が異なり、二重 open の検査をすり抜けるケースがあります。引き続き、spec 変更後の状態とテストの検出力を確認します。
**判定：修正が必要です。major 3件、minor 3件。** 変更・ビルド・mutation 実行はしていません。

### 指摘

1. **major — 不正な markup で native handle が漏れる**  
   `crates/ezsdr-radio-uhd/src/uhd.rs:688`  
   `uhd_subdev_spec_make(...)?` が失敗すると、そのまま戻ります。UHD 4.10 の `host/lib/usrp/subdev_spec_c.cpp:33–35` は **handle を確保してから解析**するため、`"A:0:1"` で確保済み handle が解放されません。  
   **修正案：** make の成否にかかわらず、非 null の handle を解放する所有管理にし、解析失敗経路のテストを追加する。

2. **major — 空の channel list で二重 open の検査をすり抜ける**  
   `crates/ezsdr-radio-uhd/src/device.rs:1038`, `:1335`  
   `rx_open(0, &[0])` の後の `rx_open(1, &[])` を fake は受理します。UhdDevice は空を `[0]` に変換するため、実機では既存 channel 0 と衝突します。TX も同じです。「fake が UHD より緩くない」に反します。  
   **修正案：** fake も検査前に空を `[0]` に正規化する。channel 数ゼロへの切り替えは空 streamer と混同せず扱い、両方向のテスト・mutation を追加する。

3. **major — subdev spec 変更後、予約済み command が別の front end に適用される／panic する**  
   `crates/ezsdr-radio-uhd/src/device.rs:707`, `:837`  
   command は論理 channel 番号を保持したまま、spec と settings だけが並べ替え・縮小されます。channel 1 の timed change を予約して `"A:0"` に縮めると、期限後の `settle()` が範囲外参照します。並べ替えなら別の front end を変更します。  
   **修正案：** 予約済み command の宛先を安定した `(slot, front end)` にするなど、map 変更で宛先が変わらないようにし、縮小・並べ替えの回帰テストを追加する。

4. **minor — markup の区切り文字が UHD と異なる**  
   `crates/ezsdr-radio-uhd/src/device.rs:600`  
   fake の `split_whitespace()` は `"A:0\tB:0"` を受理します。UHD の `host/lib/usrp/subdev_spec.cpp:19–20` は ASCII space だけで分割するため、これは解析エラーです。また、不正 markup の native 側の失敗元は `uhd_subdev_spec_make` ですが、fake は `uhd_usrp_set_rx_subdev_spec` と報告します。  
   **修正案：** tokenization とエラーの呼び出し名を実際の境界に合わせ、tab/newline の拒否を固定する。

5. **minor — stop のテストと mutation coverage が不足**  
   `crates/ezsdr-radio-uhd/tests/fake.rs:2141–2143`  
   stream 1 を停止した後、stream 0 の残り300 samples だけを確認しています。P216 が誤って stream 0 を停止しても、その300 samples が停止時点ですでに生成済みなら tail として届き、誤実装を見逃せます（**INFERRED、負荷下で未実行**）。stream 1 が実際に止まったことも確認していません。加えて、P226 は RX 側だけで、TX の spec 変更による切断を固定していません。  
   **修正案：** manual clock で停止後より先まで継続／停止を確認し、TX 側の対称テスト・mutation も追加する。

6. **minor — DA-1 違反と古い署名が残る**  
   `design/18-uhd-radio.md:276–277`, `:198`, `:205`, `:244`  
   新しい UR-33 に理由・実装ソースの説明・`this stage` が入り、UR-15/17/25 は旧署名のままです。Changes に持ち越しを書いても、規範本文の不整合は解消しません。  
   **修正案：** 理由と段階の説明を decision record／Changes へ移し、現在の stream 0 を明記した呼び出し形に更新する。

### 確認して問題を見つけなかった点

- provider の書き換えは stream 0・channels `0..n` への機械的変更で、`set_subdev_spec` の追加呼び出しはありません。
- 新規 FFI 宣言と2構造体の field 順・型は UHD 4.10 headers と一致。antenna の文字列 buffer は末尾 NUL を確保しています。
- `serial()` の null 初期化と成功／失敗時の free、`rates()` の読み取り後の解放経路は妥当です。
- **全988 mutation 行の `old` が各対象にちょうど1回一致**することを独立に確認。指定された再ターゲット行も、元と同じ誤りを作っています。

テスト・mutation の成功件数と実機挙動は再検証していません。
