# 1b-4e-1：Sol のコードレビュー（gpt-6.1-sol，read-only）

判定：PASS

major なし。minor 2件。段階 1b-4e-1 の受理を妨げる問題は見つかりませんでした。

## 指摘

1. **minor — 欠落・不正な `sequence` が `null` と区別されない**
   - **場所:** `crates/ezsdr-acceptance/tests/parity.rs:921,942`
   - **根拠・系列:** `row["sequence"].as_u64()` は、正しい `null` だけでなく、フィールド欠落や文字列も `None` にします。初期受信 segment の `sequence: null` が欠落・文字列に変わっても、`:1295` の期待値 `[None, None, Some(0), …]` を通ります。順位化ではなく、読み出し時の検査の穴です。
   - **修正:** `get("sequence").expect(...)` で存在を確認し、`null` 以外は `as_u64().expect(...)` で整数を要求する。

2. **minor — queue 待ちだけが実時間の約1秒で失敗する**
   - **場所:** `crates/ezsdr-acceptance/tests/parity.rs:1005–1009`
   - **根拠・系列:** この待ちは `std::thread::sleep` を使います。一方、manual host clock は Provider thread が実行待ちの間も時刻を止める設計です（`crates/ezsdr-radio-uhd/src/host.rs:8–20`）。**INFERRED:** control thread が約1秒以上実行されず、テスト thread だけが進む負荷条件では、Provider が正常でも assert が失敗します。mutation の偽 kill にもなり得ます。今回の実行では再現していません。
   - **修正:** queue 待ちも device の host clock に統一する。実時間での停止上限は既定の `timeout 300` に任せる。

## 確認結果

- 指示書 §3 の8比較は揃っています。同じ selector・Spec を両 Provider に通し、stream ごとの clock・床・拒否・Manifest 行を比較しています。
- T0 相対化は既知の T0 を引くだけなので、stream 間の origin 差を消しません。行のソートも node・stream・channel の取り違えを隠していません。
- **sequence の裁定は妥当です。** MR-27／UR-30 は固定の番号起点を要求していません。今回の系列では順位比較が command の順序・重複・segment との対応を検査します。
- 既存 helper の置換は同じ抽出処理を保っています。本体コードの変更はありません。冒頭コメントの比較範囲・除外範囲も実装と整合しています。
- P485–P488、および既存 parity 対象の P333・P352 は `old` が各1回一致。実装ログには6行とも killed の記録があります。

**独立実行:** `parity` を3回実行し、毎回 **18 passed / 0 failed / 0 ignored**。

未確認：mutation の独立再実行、workspace 全体、実機、負荷試験。ファイル変更はしていません。
