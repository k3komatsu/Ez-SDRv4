**差し戻し推奨：新規 major 1件。** 読み取り専用で確認しました。

### 旧指摘の判定

| 項目 | 判定 |
|---|---|
| ① 後続 item の release・prune 後の床 | **修正済み**。全 item への再計算と床の保持あり。 |
| ② release と ready の混同 | **修正済み**。ready の上書きなし。End 前に予約した TX clock の回帰テストあり。 |
| ③ RM-3 の channels | **修正済み**。要素数・昇順対応・stream ID の例を訂正。 |

追加の方向間移動の拒否、`refuse` の両側への委譲も妥当です。

### 新規指摘：major — Stop 済みの移動元の cut を待たない

**箇所：** `crates/ezsdr-radio/src/timeline.rs:809`、`:816–819`、`:631–633`

**反例（静的追跡による INFERRED、実行再現なし）：**

- RX、origin=0、lead=0。移動元は channel 0・1 Hz、移動先は空・1 MHz。
- 100 ms に移動元を `Stop`：元の segment の cut は **1 s**。
- 200 ms に channel 0 を移動先へ移動。
- 移動元には running segment がないため、move の `cut_by(seq)` は `None`。移動先は **200 ms** に開始し、元の segment の終了 **1 s** より早く channel を持ちます。

`Stop` の cut は残っていますが、release は「move 自身が作った cut」しか参照しません。`End` や、その後の prune でも同じ欠落を検討する必要があります。生成テストも `tests/timeline.rs:1064` で `None` を検査対象外にしています。

**修正：** move 自身が cut を作らなくても、移動元がその channel を持っていた直前の segment の終了を release に反映し、prune 後も保持する。上記の Stop→move と、その prune 版をテスト・mutation で固定してください。

### 検証

- 既存ビルドの timeline テスト：**38/38 passed**。
- 指定19 mutation 行：`old` は全件ちょうど1回一致。mutation は再実行していません。
- P96 は現実装では非 equivalent。P103 は **plan に関して equivalent**：畳んだ cut と release floor が同じ制約を残します。
