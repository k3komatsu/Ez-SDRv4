## 結論

**Sol 1–8、O1–O6 はすべて修正済みです。今回の修正による新規不具合は見つかりませんでした。**

| 項目 | 判定 | 確認内容 |
|---|---|---|
| Sol 1 / O1 | 修正済み | CBX は適用された retune ごとに位相を再抽選。テストが2回の周波数変更後の両 channel の回転・位相差と RNG の抽選順を照合する。 |
| Sol 2 / O2 | 修正済み | layout を `[]`・`[A]`・`[A,B]` に限定。B-only・重複・3要素を拒否し、理由の slot・位置も検査する。 |
| Sol 3 / O3 | 修正済み | 空 RX の既定は0。両 Provider の適用値、Mock の無受信、UHD の RX SampleClock・設定なしをテストで固定する。 |
| Sol 4 | 修正済み | CBX の coherent は false。両 Provider が `Eq(true)` を拒否し、UBX は受理する。 |
| Sol 5 / O4 | 修正済み | `rules.py` は HEAD と差分なし。3件の改名は `--renames` で扱い、存在検査を迂回しない。 |
| Sol 6 | 修正済み | R11 は `.fract() == 0.0` に修正。全639 rate の受理テストが、商の丸め誤差による誤拒否を検出する。 |
| Sol 7 / O6 | 修正済み | spec 07・Vision §25/§26 が CBX の例外と `random` に対応。 |
| Sol 8 | 修正済み | 指摘した rationale・段階の経緯を規則本文から除去。UR-5 のテスト表も実際の受理ケースに一致。 |
| O5 | 修正済み | `Profile::new` で description を一度構築し、各 accessor が保持値を読む。これはコード経路で確認。 |

再抽選は device ごとの独立 RNG を保持し、`advance` の effective instant・到着順で消費します。CBX の不要な timed 定数の抽選を省いても、別ストリームの retune・受信 jitter・noise の抽選はずれません。

### 検証

- 関連テスト **14件を独立実行し、全件成功**。
- 指定の `rules.py --base HEAD --renames …` は妥当で、**0 problems**。
- `git diff --check` 成功。
- mutation 17行の kill 記録と該当コード・テストを照合。mutation 自体は再実行していません。
- ファイル変更・実機操作なし。workspace 全体の再実行はしていません。
