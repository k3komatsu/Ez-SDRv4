# A2：Sol のレビュー

## コードレビュー

判定：FAIL

1. **major — UR-17 の試験が sample index の誤りを検出しない**
   - **場所:** `crates/ezsdr-radio-uhd/src/provider/rx.rs:1078–1086`
   - **根拠: VERIFIED（静的確認）** 各 channel 内の3サンプルが同じ値なので、サンプル順序の反転や、全 index に `channel[0]` を書く誤実装でも全 assertion が通ります。T43 の channel/index 交換を殺しても、指示書が求めた「index もずれていない」は固定できていません。
   - **修正:** channel と index の両方で異なる値を与え、全位置を照合してください。先頭サンプルの複製などを壊す mutation 行も追加します。

2. **minor — T39 は T0 下限違反ではなく、受信が消えることで落ちる**
   - **場所:** `plan/maintenance/tools/mutations.json:9631–9640`、`crates/ezsdr-mock-radio/tests/mock_radio.rs:2070`
   - **根拠: VERIFIED（経路確認）** 複合変異では再開 segment が9 msになりますが、`rx_next_segment` の cursor 下限により選ばれません（`crates/ezsdr-mock-radio/src/lib.rs:1266–1269`）。結果は「T0 前の clock/block」ではなく「clock が無い」で、期待する1本との比較が落ちます。kill は有効ですが、T0 下限そのものを直接観測した証拠ではありません。
   - **修正:** planner の再開 `segment.origin ≥ T0` を直接確認する小テストを加え、T39 をその assertion で殺してください。既存の Provider 経由の試験は残します。

### その他の確認結果

- **T0 の三重下限は正しい layering** です。`command_instant` は command の効く時刻、`begin` の cut は前 segment との境界、ready は受信 call・allowance を含む準備完了を守ります。`ideal` で同じ値になることだけを理由に削るべきではありません。
- T40 は再開 origin が受理時刻より前になることを検出し、T41 は late event を別に検出します。
- RM-14、gap 0、未来の `at`、CLOCK_LOST の source、arm の無送信、missed-start lead の起点は、対象経路と assertion が対応しています。本番コードの変更や、新テストによる仕様との不一致は認めませんでした。
- Spec の追加参照名はすべて存在し、UR-17 の共有参照の `none named` は保持されています。DA-1 形式・Changes 行に追加指摘はありません。
- T34–T44 と U27 の全 anchor は各1件に一致。U27 は現行コードに適合しています。

**実行確認:** MockRadio 96件、UHD の対象 unit/fake 15件、計111件が成功。`git diff --check` も成功しました。

ファイル変更・mutation 実行・実機確認はしていません。全 workspace・rules・全 mutation gate は再実行していません。

## 直し 1 回目の再レビュー

判定：PASS

追加指摘なし（major 0、minor 0）。前回の2件は解消しています。

1. **前回 major：解消 — VERIFIED**
   - `crates/ezsdr-radio-uhd/src/provider/rx.rs:1068–1090`
   - channel と index の両方で異なる値を与え、全6位置を照合しています。channel/index 交換、index 固定、逆順、先頭サンプルの複製はいずれも失敗します。
   - T45 は出力 index を0に固定し、このテストを filter に指定しています。実装ログ `impl-fix1.log:1982` で killed を確認しました。
   - 追加修正不要。

2. **前回 minor：解消 — VERIFIED**
   - `crates/ezsdr-radio/tests/timeline.rs:1504–1516`
   - 受信の `command_instant` が直接 `e == T0` を返すことと、送信には同じ floor が掛からないことを確認しています。
   - `plan/maintenance/tools/mutations.json:9631` の T39 は floor だけを外す単一箇所の変異です。受信側の期待値10 msに対して8 msとなるため、この assertion で失敗します。実装ログ `impl-fix1.log:1981` でも killed を確認しました。
   - `design/09-mock-radio.md:195` の MR-29 Test 行に新テスト名があり、Provider 経由の既存テストも保持されています。
   - 追加修正不要。

隣接する経路・テスト・仕様参照に、新たな問題は認めませんでした。

**実行確認：** timeline 55件、UHD RX unit 47件、MockRadio MR-29 5件、計107件成功。

ファイル変更・mutation 実行はしていません。killed は既存ログの確認であり、全 workspace・全 gate・実機は再確認していません。
