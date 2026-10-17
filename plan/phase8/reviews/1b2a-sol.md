N=257 の coercion は RM-8 と整合しています。ただし「後段で必ず拒否」は不正確で、SB-45 により Session は既定 `warn` です。主な懸念は、throughput テストが RX・stream 0 に偏り、複数 stream の合算と TX 側の制約を固定できていない点です。
**blocker はありません。major 1件、minor 2件です。実装上の誤動作は確認できませんでしたが、throughput のテスト補強を推奨します。**

### 1. major — 複数 stream・TX の throughput 制約をテストが固定していない
`crates/ezsdr-radio/tests/channels.rs:617–678`

全ケースが RX・stream 0 です。例えば `c0→s0、c1→s1、両方200 Msps` は合計1.6 GB/sとして拒否すべきですが、「stream 0だけ計算する」「stream ごとに別々に上限判定する」「TX を判定しない」という誤実装を現行テストと P56–P58 は捕まえません。

**修正案:** 異なる rate の複数 stream の合算、TX 側、rate を要求せず membership だけで上限を超えるケースを追加し、それぞれを壊す mutation row を置く。現コードの合算処理自体は正しいと読めます。

### 2. minor — spec のテスト記録が実際の assert より広い
`design/07-radio-model.md:396`  
`crates/ezsdr-radio/tests/channels.rs:217–259`

RM-28 のテスト表は「every refusal names its position and value」としますが、大半は理由の部分文字列しか検査していません。例えば未知 board のエラーから position・入力値を削っても、そのケースは通ります。

**修正案:** 各要素の拒否について position・入力値を assert するか、テスト表を検証済みの範囲へ狭めてください。

### 3. minor — N=257 が後段で拒否されるという報告に Session の例外がない
`/home/komatsu/.cache/ezsdr-fixes/2026-10-17-phase8-1b2a/report.md:23`

N=257 を `coerce` が格子値へ丸め、coercion を返す判断は RM-8 と整合します。ただし **Session は SB-45 により既定 `warn`** なので、後段で必ず拒否されるわけではありません。Spec の明示的な `accept` も同様です。

**修正案:** 「通常の Spec Run の既定 policy では拒否、Session では警告付き適用」と訂正してください。実装変更は不要です。

### 確認して問題なかった点
- 混在 layout、番号の存在確認、同一 board の全二重制約、link 1本分の合算処理。
- UBX/OBX/CBX の値は `Board` に委譲され、旧 builder の変更は同値の共通化です。Mock・UHD・server・Python の変更はありません。
- P39–P71 は全33行の `old` が対象コードに**ちょうど1回一致**し、指定テストも存在します。既存ログの33/33 killed を確認しました。
- HG の省略時1 GbEは保守的な既定として妥当です。ただし HG 全体が1 GbEという意味ではありません。

ビルド・mutation の再実行、ファイル変更はしていません。Basic/LF の実機 parity、周波数上端・phase は引き続き **INFERRED** です。
