# 1b-5c-2：Sol 6.1 のコードレビュー

## 1. コードレビュー

判定：FAIL

本体の削除に明らかな挙動退行は見つかりませんでした。ただし、削除テストの未移植ケースと、仕様・Vision の更新漏れがあります。

## 指摘

1. **major — 削除した coercion テストの一部に代替がない**  
   `crates/ezsdr-radio/tests/channels.rs:482`、`crates/ezsdr-mock-radio/tests/mock_radio.rs:544`
   - **根拠：** HEAD の `rm_26_coerce_cases` は、gain `31.3 → 31.5` と、antenna の `Set(["LNA", "TX/RX"]) → "TX/RX"` を確認していました。残ったテストは gain の下向き丸め・同距離の丸めと、先頭から有効な antenna の集合しか確認していません。
   - **系列：** gain を常に切り下げる誤実装や、antenna の集合の先頭が無効なら直ちに拒否する誤実装を、移植先の assertion は区別できません。これらの誤実装による全テスト通過は **INFERRED**（mutation は未実行）。
   - **修正：** この2ケースを numbered key にして `rm_30_coerce_cases` へ移してください。到達可能な規則なので、旧 builder の廃止だけでは削除理由になりません。

2. **minor — Vision §8 が元の coherent-array 要求を保持していない**  
   `design/vision/02-spec-binding-and-compilation.md:27`、同ファイル `:60`
   - **根拠：** 4 RX channel の membership と `random_unless_timed_tune` は要求していますが、timed tune や coherence の有効性を要求していません。Vision §25（`design/vision/05-coherence-calibration-and-runtime-semantics.md:37`）自身が、untimed retune 後は coherent ではないと定義しています。
   - **系列：** 位相がランダムな初期状態でも、この例の capability 条件は満たせます。「4 channel を同一 stream に入れる」と「coherent な sensing array」は同義ではありません。
   - **修正：** 元の coherent-array の意図を明記し、timed tuning／有効性の前提を添えてください。現行キーだけではその要求を完全には表せないなら、その限界を明示してください。

3. **minor — 廃止したキーが現在の規範・説明に残る**  
   `design/07-radio-model.md:93`、`design/vision/05-coherence-calibration-and-runtime-semantics.md:110`
   - **根拠：** RM-8 は依然として `radio.d.frequency_step_hz`／`gain_step_db` を参照します。Vision §26 は `radio.tx.path_delay_samples`／`radio.rx.path_delay_samples` を現在の選択肢として説明しています。
   - **系列：** 説明どおり unnumbered key を使うと、layout description は `RM-4: not a radio key` で拒否します。
   - **修正：** step と path delay の説明を channel-numbered key のみに統一してください。Changes 表の歴史記述は残して構いません。

4. **minor — RM-26 の pruning 契約が移動せず消えている**  
   `design/07-radio-model.md:248`
   - **根拠：** HEAD の RM-26 は「将来の booking が先行せず、将来の refusal が到達しない境界より前だけを prune し、状態へ畳み込み、plan を変えない」と規定していました。新 RM-25 は使用する関数名だけで、RM-34 の move-specific pruning 規則もこの一般条件を補いません。
   - **系列：** まだ拒否され得る項目まで prune すると、その後の refusal に必要な履歴を失います。実装の退行ではなく、保持すべき契約の脱落です。
   - **修正：** この一般条件を RM-25 に移し、残っている `rm_26_a_pruned_line_plans_as_one_that_keeps_every_item` を引用してください。

## 確認できた点

- tree・defaults・envelope・decimation・RF envelope の主要な規則は、`rm_29_*`、`rm_30_coerce_cases`、`mr_03_*`、`mr_06_coerce_cases`、`ur_09_*`、`d3_*` に残っています。例外は指摘1です。
- 新 MR-30 テストは有効です。選んだ loopback sample の期待値は約 `(-0.454934, -0.207450)`、逆順なら虚部が `+0.207450` となり、同一 draw なら `(0.5, 0)` なので区別できます。P192 の kill は実装ログにもあります。
- 削除した P62／P122／P283／V02 の `old` は対象 source に0件。残存 mutation 行の一致検査、当て直し先の経路確認に問題はありません。P333 は effective configuration を空にする意味のある mutant です。
- `python/ezsdr` は無変更。維持キーの宣言・読み取り、変更箇所の本番／`cfg(test)` 分岐に退行は見つかりませんでした。
- Vision の §番号・navigation は不変。`rules.py` は **0 problems**。
- 独立実行した Radio／Mock、acceptance の x3xx／parity、UHD の UR-5、Kernel schema freeze は **計270テスト成功**。

未確認：実機、Python suite、workspace 全体、mutation の再実行。ファイル変更はしていません。

## 2. 修正の回 1 の再レビュー

判定：PASS

再レビューで追加指摘はありません。前回の major 1件・minor 3件は解消されています。

1. **coercion の網羅性**
   - `crates/ezsdr-radio/tests/channels.rs:555–575` に、19.5→20 Msps、gain 31.3→31.5 dB、無効 antenna を飛ばす Set の assertion があります。
   - `crates/ezsdr-mock-radio/tests/mock_radio.rs:544–589` に、下側への tie、7 GHz 拒否、RX/TX throughput 拒否、整数 Hz 以外の拒否が残っています。20.25 dB の tie は、旧30.25 dB と同じ共有規則を検証しています。
   - 固定 channel 数の上限拒否は、layout に存在しない RX/TX channel の拒否へ置き換わっています。

2. **mutation**
   - `plan/maintenance/tools/mutations.json:8821–8848` の P549–P551 は、最遠 rate の選択、gain の切り下げ、antenna Set の先頭だけを見る変更です。いずれも非等価で、対応 assertion が誤動作を区別します。
   - 各 `old` は source に正確に1件一致します。既存の remote 実行ログでも3件の killed を確認しました。今回の再実行はしていません。

3. **文書**
   - Vision §8 は coherent-array の意図、timed tune、達成した coherence と capability 要求の違いを明記しています。
   - RM-8 と Vision §26 は channel-numbered key に統一されています。
   - `design/07-radio-model.md:250–252` は pruning の境界条件、開始状態への畳み込み、plan 不変を復元しています。
   - 関連 spec 07・09・18 の `*Test:*` に列挙されたテスト名はすべて実在し、`rules.py` は **0 problems** でした。

**独立検証：** Radio／Mock の **221テスト成功**、`git diff --check` 成功。修正対象周辺に範囲外の変更は見つかりませんでした。

未確認：修正直前 tree の保存差分がないため、その時点との厳密な全差分比較。実機・workspace 全体・mutation 再実行は対象外です。ファイルは変更していません。
