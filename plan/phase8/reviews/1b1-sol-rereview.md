**判定：実装の修正は妥当。minor 2件が残っています。**

| 項目 | 判定 | 確認結果 |
|---|---|---|
| Sol 1 | 部分修正 | prefix 外の exact／numbered 宣言を拒否。指定の連続登録テストは未実装（下記2）。 |
| Sol 2 | 部分修正 | overlap の排他的定義と併存テストは正しい。ただし綴りの新しい限定句が偽（下記1）。 |
| Sol 3 | 修正済み | 固定 segment・segment 数の負例あり。P37 は固定 segment 比較を無効化する。 |
| O1 | 修正済み | `key_decl` は一回の探索。P23 削除、P22 更新済み。 |
| O2 | 部分修正 | MA-34 の拒否・診断・テスト・P38 は実装済み。連続登録の確認のみ不足。 |
| O3 | 修正済み | MA-32 は MA-34／SB-2a の引用のみ。追加文に経緯・理由の混入なし。 |
| O4 | 修正済み | P35 は exact 同士だけを対象とし、P36 は `(false, true)` 枝を対象とする。 |

### 残存・新規指摘

1. **minor — 「綴りを覆うのは exact 宣言だけ」は成立しない**
   - `design/03-spec-and-binding.md:148`
   - **反例：** A=`test.c.x0`／`Some(1)`、B=`test.c.x`／`Some(2)`。両者は重ならず登録可能だが、A 自身の綴り `test.c.x0` を numbered 宣言 B が覆う（`spec.rs:564–590`）。
   - **修正：** `which only an exact declaration of that spelling covers` を削除。「その numbered 宣言自身は覆わない」だけに留め、この組合せをテストする。

2. **minor — prefix 拒否後の registry 状態をテストしていない**
   - `crates/ezsdr-kernel/tests/module_api.rs:296–326`
   - **不足：** 拒否ごとに registry を破棄し、正当な登録は別の新品 registry で行っている。拒否された宣言を内部に残す誤実装でも、このテストは通り得る。また後続宣言は指定の exact `other.item0` ではない。
   - **修正：** 同じ registry で不正な `test` Vocabulary を拒否した後、prefix `other` の exact `other.item0` を登録・lookup する。

### 確認範囲

- P22・P35–P38 の `old` は各対象に**ちょうど1回一致**し、名前どおりの変更。保存ログでも全行 killed。mutation は再実行していません。
- radio／sink／sim／session 在庫／test doubles に新しい prefix 検査による誤拒否は見当たりません。
- `module_api` **42件**、`schema_freeze` **5件**を再実行し全成功。schema 再生成の整合も確認済み。
- ファイル変更なし。
