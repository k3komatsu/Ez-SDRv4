## 結論

**段階1aは修正後に再レビューを推奨します。** major 4件、minor 4件です。特に、CBX の位相と layout の物理 channel 対応は、現状のテストが通っても hardware parity を満たしません。

### 1. major — CBX が retune をまたいで位相を保持する

**箇所:** `crates/ezsdr-mock-radio/src/lib.rs:1399–1408`、`crates/ezsdr-mock-radio/tests/mock_channel.rs:738–787`、`design/09-mock-radio.md:197`

- **入力 → 誤動作:** CBX で周波数を f₁ → f₂ → f₁ と timed retune しても、Mock は初期の LO phase を保持します。`random` の場合は phase timeline を更新しないためです。
- これは設計33 D13 の「timed tune でも位相をランダムにする」と異なり、retune 前の較正を保持できるという強すぎる保証になります。新テストは、むしろ「retune 後も初期位相と同じ」を assert して、この誤動作を固定しています。
- UHD 4.10 の [sync.dox:197–205](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/docs/driver_usage/sync.dox#L197-L205) も、その他の frontend は tune ごとに再 alignment が必要としています。

**修正案:** `random` は適用された retune ごとに seeded RNG から新しい位相を描き、その effective instant に記帳する。テスト・P10・MR-34 も、複数の実周波数変更を通してこの性質を確認する形に直してください。

### 2. major — B-only layout が slot A を操作する

**箇所:** `crates/ezsdr-radio/src/x3xx.rs:375–392`、`crates/ezsdr-radio-uhd/src/provider/mod.rs:167–169`

- **入力 → 誤動作:** slot A/B に同種の UBX がある装置で、`tx: ["0:B:ubx:0"]` を指定します。parser は受理しますが、UHD 側は要素の位置をそのまま channel 0 として照合・操作するので、既定 mapping の **slot A** から送信します。board 名が同じなので拒否されません。
- 実際の設定も `provider/core.rs:445–446`、streamer も `uhd.rs:743` が `0..channels` を使います。slot 情報はその後使われません。
- 実機での送信は未確認ですが、channel 0 を操作するコード経路と UHD の既定 subdev mapping は確認しました。

**修正案:** subdev spec 対応を予定どおり1b-4に残すなら、1aでは各方向を既定順の prefix、すなわち `[]`・`[A]`・`[A,B]` に限定してください。B-only の拒否テストと mutation row が必要です。

### 3. major — RX layout が空でも既定で RX 1 channel が有効になる

**箇所:** `crates/ezsdr-radio/src/x3xx.rs:431–438`、`crates/ezsdr-radio/src/device.rs:563`

- **入力 → 誤動作:** `rx: []`, `tx: ["0:A:ubx:0"]` で、RX channel 数を要求しません。description は `rx_max_channels = 0` を宣言する一方、既定値は `x310_defaults()` の **RX 1** のままです。
- `coerce` の空要求はこれを拒否せず、`Present` でも最大値を検査せず 1 を返します。両 Provider の `prepare` はこの既定値から RX SampleClock を作り、UHD は layout が除外した RX channel を設定します。
- 既存の非対称 layout テストは、RX が空のケースと default の整合性を見ていません。

**修正案:** RX layout が空なら既定 RX channel 数も 0 にする。空要求・`Present`・`prepare` を通し、layout 外の channel、SampleClock、受信処理が生じないことを固定してください。

### 4. major — CBX が依然として coherent を宣言する

**箇所:** `crates/ezsdr-radio/src/x3xx.rs:451`

- **入力 → 誤動作:** CBX 2 channel の layout に `radio.rx.coherent: Eq(true)` を要求すると、両 Provider が受理します。description が board にかかわらず `coherent: true` だからです。
- RM-9 はこれを「sample-aligned **かつ known phase relation**」と定義しています。CBX は設計33 D13のとおり aligned ですが coherent ではありません。`phase_behavior_on_retune = random` を足すだけでは、この誤った matching は止まりません。

**修正案:** 現行 key が残る1aでは、少なくとも CBX の `coherent` を false にする。CBX の coherent 要求を両 Provider が拒否するテストを追加してください。key 自体の削除は予定どおり1bで構いません。

### 5. minor — `KNOWN_MISSING` が rename の検査を恒久的に迂回する

**箇所:** `plan/maintenance/tools/rules.py:52–60`

- **入力 → 誤動作:** 追加された旧テスト名を、現行規則の `*Test:*` 行に誤って再使用しても、対応するテストが存在しないことを検出できません。
- `check()` をメモリ上の小さな文書に対して呼び、追加3名称すべてでこの見逃しを確認しました。単なる history の許可ではなく、本文も含む global exemption です。

**修正案:** 3件を `KNOWN_MISSING` から外し、既存の `--renames` を使う。旧名を closing Changes table に残す必要があるなら、その履歴引用だけを区別して扱い、現行本文の存在検査は維持してください。

### 6. minor — R11 の移植で mutant の条件が反転している

**箇所:** `plan/phase7/tools/mutations.json:1017–1024`

- **入力 → 誤動作:** R11 は「逆算した商の小数部が非ゼロの rate を誤って拒否する」変異のはずですが、新しい `new` は `.fract() != 0.0` を**受理条件**にしています。
- 例えば N=200 の通常の rate が拒否されるため、狙った floating-point hazard を検査できなくても kill できます。元の行が表していた failure mechanism とは逆です。

**修正案:** 追加する受理条件を `.fract() == 0.0` に直す。kill が、商の丸め誤差を持つ合法 rate によって成立することを確認してください。

### 7. minor — Vision に CBX の誤った保証が残っている

**箇所:** `design/vision/05-coherence-calibration-and-runtime-semantics.md:114–120`

- **入力 → 誤った説明:** CBX の利用者が §26 を読むと、依然として「timed tuning keeps the offset constant」と
読めます。phase capability の値も2種類のままで、追加された `random` がありません。
- これは設計33の「書き換える文書」に明示された箇所であり、今回変更しないよう指定された §8 の例とは別です。

**修正案:** CBX を timed phase synchronization の説明から分離し、`random` を追記してください。併せて `design/07-radio-model.md:23` の冒頭「UBX/CBX/SBX is random unless … timed」も、後半の CBX 例外と矛盾しない表現に直してください。

### 8. minor — 新しい規則本文に rationale・段階の経緯が混入している

**箇所:** `design/09-mock-radio.md:48`、`design/18-uhd-radio.md:163–177`

- **入力 → 誤った文書構造:** accepted rule を読むと、規範に加えて「so that … cannot describe one selector differently」や「not changed now」といった共通化の理由・段階的実装の経緯が本文に含まれています。DA-1 が要求する normative text と decision record の分離を満たしません。
- また `design/18-uhd-radio.md:330` は、二つの UBX を受理する実際のテストに対して「CBX と one-OBX 以外は拒否」と記載しており、テスト結果の説明も不正確です。

**修正案:** rationale・段階の説明を closing Changes table と設計33へ移し、規則には現行の契約だけを残す。§6 のテスト表には two-UBX の受理も記載してください。

## 確認して問題がなかった点

- X3xx の board 値と selector parser を語彙へ集約し、両 Provider が同じ description を使う構造。
- decimation 集合の生成、639要素、合法 rate の厳密な判定。UHD 4.10 の DDC/DUC source と照合しました。
- FakeDevice の nearest-decimation clipping と、同距離なら小さい N を選ぶ処理。UHD の `meta_range_t::clip` と整合します。
- start lead の方向別分離、両 Provider の読み出し箇所、schema、および異なる lead に対する timeline テスト。
- 指示された command lead・repeat 制約の Provider 間差を、この段階で保持している点。
- server の既定 profile、fixture、Python の移行。既存テストの変更は概ね設計上の意味変更に対応していました。
- 変更された mutation 32行について、`old` と適用後の `old2` が各1回一致することを確認しました。ただし R11 と CBX のテストの意味には上記の問題があります。
- `rules.py --base 0f0c46b` は **0 problems**、`git diff --check` も問題なしでした。ただし前者は追加された whitelist の妥当性を保証しません。

**検証範囲:** ファイル変更、build、テスト suite、mutation、実機操作は行っていません。実装役の実行結果は独立再確認していません。実機での再現と、FPGA の halfband/CIC 実値に基づく639要素の適用可否は **INFERRED／bench 未確認**です。
