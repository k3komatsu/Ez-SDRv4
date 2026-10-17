指定16行の mutation は、すべて対象コードにちょうど1回一致しています。全設定判定と複数 stream／TX のテスト追加も確認できました。残る差分は、RM-32 の拒否先の優先順位と、テスト・報告の記述の正確さです。
**major／blocker は確認できませんでした。minor の残件があります。**

### 各項目の判定

| 項目 | 判定 | 確認結果 |
|---|---|---|
| Sol 1 | **fixed** | 複数 stream の合算、異なる rate の境界、TX、membership-only の拒否を追加。P77–P79 も対応する誤実装を捕まえる。 |
| Sol 2 | **partially** | position・値の assert は追加。ただし重複要素では board 名だけを確認し、RM-28 が要求する frontend・slot の欠落を捕まえない。 |
| Sol 3 | **partially** | Session の既定 `warn` は訂正済み。ただし明示 `accept` も「警告付き」とする新しい説明は誤り。 |
| O1 | **fixed** | 公開 `violations` が全設定を要求なしで判定。違反 key、単一更新の重ね合わせ、帰属不能時の拒否をテストしている。 |
| O2 | **fixed** | parser は `lib.rs::concrete` に統合。旧 key 構成は従来の RF 判定経路を通り、テストもある。 |
| O3 | **fixed** | slot をまたぐ同一 timekeeper 上の異種 board 全対で表を参照。P72 が参照経路を固定する。 |

### 残件・新規指摘

1. **minor — RM-32 の拒否先優先順位が本文と異なる**  
   `crates/ezsdr-radio/src/channels.rs:717`、`design/07-radio-model.md:347`  
   **ケース:** `c0=-1、c1→s1` で s1 が上限超過し、要求が s0・s1 両方の rate を名乗ると、実装は違反自身の **s1** を優先する。本文は要求内の最小番号 **s0** を指定している。  
   **修正:** 決定どおり「違反自身の key を優先、なければ最小番号」と本文を合わせ、このケースを assert する。

2. **minor — RM-28 の診断テストがまだ記述より弱い（Sol 2 残件）**  
   `crates/ezsdr-radio/tests/channels.rs:249–256`  
   **ケース:** 重複要素の理由から frontend・slot を削ってもテストが通る。二種類衝突でも slot の欠落を検出しない。  
   **修正:** 各ケースで board・frontend・slot を明示的に assert する。

3. **minor — `accept` は警告を出さない（Sol 3 の訂正で混入）**  
   `crates/ezsdr-radio/tests/channels.rs:523–524`、`fix-report.md:25`  
   **ケース:** 明示 `accept` は値を適用するが警告はない（`crates/ezsdr-kernel/src/plan/coercion.rs:33`）。  
   **修正:** 「Session／`warn` は警告付き、`accept` は警告なし」と分ける。

帰属不能時の `Present` 拒否は、Kernel では通常の拒否ではなく **SB-7 の malformed-report violation** になることも確認しました。黙って受理される経路ではありません。二重拒否は同一 key で抑止されています。

指定16 mutation 行はすべて**ちょうど1回一致**し、対象・テストの対応も確認済み。既存ログは P78 再実行を含め16/16 killed。今回はファイル変更・ビルド・mutation 実行なしです。
