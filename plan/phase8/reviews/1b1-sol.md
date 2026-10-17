**判定：major 1件、minor 2件。prefix 外の宣言は、この段階で拒否することを推奨します。**

### 指摘

1. **major — prefix 外の宣言が、無関係な Vocabulary の登録まで妨げる**
   - `crates/ezsdr-kernel/src/module_api/registry.rs:199,237`
   - 入力：Vocabulary A（prefix `test`）が `other.item`／`indexed: Some(1)` を登録 → 受理されるが、lookup の prefix filter により `other.item0` は引けない。その後、Vocabulary B（prefix `other`）が正当な `other.item0` を登録 → overlap として拒否される。
   - 単なる既存の「死んだ宣言」に、新しい cross-Vocabulary 検査による誤拒否が加わっています。
   - **修正案：** exact／numbered 共通で、宣言が自身の Vocabulary prefix 配下にあることを登録時に検査。両登録順のテストと mutation を追加する。

2. **minor — SB-2a の重なりの定義が実装より広い**
   - `design/03-spec-and-binding.md:148`
   - 入力：exact `test.item` と numbered `test.item`／`Some(1)`。共通する具体 key はなく、実装は正しく受理します。しかし本文の「各位置で segments が等しい」という条件では overlap になります。また「宣言自身の spelling の要求は拒否」も、別の exact 宣言がある場合には成立しません。
   - **修正案：** equality の枝を「両方とも非 numbered の位置」に限定。spelling は「その numbered 宣言では覆わない」と記述し、この exact＋numbered の併存をテストする。

3. **minor — lookup が非 numbered segment を保存する条件の負例がない**
   - `crates/ezsdr-kernel/tests/module_api.rs:318,470`
   - 例：`test.c.frequency`／`Some(1)` に対する `test.c0.gain` は拒否すべきですが、この種の suffix 違いを拒否する assertion がありません。`declares` が numbered 以外の segment 比較を省いても、新規テストでは捕まらないと判断します（実行未確認：**INFERRED**）。
   - **修正案：** middle segment の numbered 宣言に対し、固定 segment・segment 数の異なる key の拒否と mutation を追加する。

### 報告 §6 の判断

1. prefix 外：上記の理由で延期非推奨。
2. 二段検査の順序：D2 の独立した要求ではなく、mutation 不在は blocker ではありません。ただし tool は複数行置換・`old2/new2` を扱えるため、「表現できない」は不正確です。
3. **P23 equivalent は妥当。** 登録後は一致する宣言が高々一つで、探索順による返値の差がありません。
4. HD-6：optional field の省略なので変更不要。
5. 未追跡研究ファイル：レビュー対象外。
6. 公開関数の署名変更：凍結前方針に適合。

### 確認して問題なかった点

- 有効な宣言について、番号文法と overlap 実装は整合。異なる numbered 位置の交差も扱っています。
- 全8か所の registry lookup 呼び出し、具体 key の診断・capability／effective／update-class lookup、coercion policy の完全一致を確認。
- P19–P35 と J206/J226/J242 の `old` はすべて対象に**ちょうど1回一致**。既存テストの弱体化はなく、double の既存 `test.grid` 挙動も保持されています。
- schema の `indexed` は非必須・default null。省略された旧文書は `None` のままです。
- 保存ログで workspace 1163 passed、mutation は16 killed＋1 equivalent を確認。**ビルド・テスト・mutation の再実行、ファイル変更はしていません。**
