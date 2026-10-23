# 1b-7b：Sol 6.1 のレビュー

## 1. テスト計画のレビュー

判定：FAIL

**major 1件、minor 2件。** 共通関数と9-byte payloadへの変更方針は支持しますが、変更しない挙動を固定するテストが1つ不足しています。以下の `test-plan.md` は指定の作業ディレクトリ内を指します。

### 指摘

1. **major：missed start 単独の飛びで、誤って `RX_OVERFLOW` を出す実装を検出できない**
   - **根拠：** `test-plan.md:22` の T17 は `LateCommand → Overrun → Samples` のみ。brief §1 は「missed start だけの飛びは event なし」を維持対象にしています。現在は `crates/ezsdr-radio-uhd/src/provider/rx.rs:574` が `MissedStart` を報告対象から除外していますが、既存の `crates/ezsdr-radio-uhd/tests/fake.rs:1213–1224` は `RX_OVERFLOW` の不在を確認していません。
   - **すり抜ける誤実装：** 共通関数へ渡す変換で `MissedStart` を `Sequence` にしてしまう。T17 は Overrun 優先で正常に見え、Alignment 単独や report なしの既存ケースでも検出できません。
   - **修正：** T17 の決定的 unit test に `LateCommand → Samples` 単独ケースを追加。実際に正の飛びを持つ block が届いたこと、`GAP_BEFORE` と正しい `lost`、`LATE_COMMAND` 1個、`RX_OVERFLOW` 0個を明示 assert してください。

2. **minor：mutation 検索結果から D33 が漏れている**
   - **根拠：** `test-plan.md:42–53` の一覧に、`plan/phase4/tools/mutations.json:323` の **D33** がありません。D33 の `old/new` はどちらも検索語 `to_hot` に一致します。
   - **シナリオ：** 共通 emit 箇所の変更で、この行だけ照合不能になる、または Warning の検証を弱める可能性があります。
   - **修正：** D33 を一覧へ追加。「共通 emit 箇所が不変なら維持、変われば当て直す」と記載し、`mr_37_the_overflow_travels_the_hot_path` の severity assertion（`mock_radio.rs:2130`）を維持してください。他の該当行と、無関係として除外した M19 は確認できました。

3. **minor：保持する既存 overlap テストの系列を誤記している**
   - **根拠：** `test-plan.md:32` は `mr_21_overrun_shape` を「別blockにまたがる2 fault」と記述しています。しかし `crates/ezsdr-mock-radio/tests/mock_radio.rs:1816–1836` は、1 ms と10 msの fault が**同じ再開 block**へ合流する系列です。fault ごとの損失は50,000と9,000、block 全体は59,000です。
   - **シナリオ：** T9 の50 ms間隔の系列へ置き換えると、再開待ちの途中の fault が残りの空白だけを延ばす、到達可能な系列を失います。
   - **修正：** この系列を明記して保持。event だけを `(time=1,000, cause=overrun, lost=59,000)` の1個に更新し、block と fault rows の既存 assertion は残してください。

### その他の確認

- **T18 の例外扱いは妥当です。** Mock は正の損失の場合だけ report を積みます（`lib.rs:761–768`）。現行の fault 経路では report を残したまま飛びなし block を作れません。UHD の通常の `Rx::receive` 経路と共通関数の unit test で固定する方針で十分です。実機での到達性は **INFERRED** のままです。
- T6–T12・T17–T20 は計画上すべて対応しています。event の存在確認を先に行うため、空集合の `all(...)` や空の合計による合格は避けられています。
- T8・T9 の2 ms／52 msという配置は、Mock と FakeDevice で同じ損失を作れます。FakeDevice は受信位置が fault 時刻へ達すると、Overrun で50 ms、SequenceError で要求 block 長だけ進めます（`device.rs:1312–1337`）。
- channel 全体への gap・valid、Alignment の独立した event、stream 固有の source は、保持予定の既存テストが観測しています。UHD の pending を変更するのは受信 owner 自身なので、recv 後から publish までに別 thread が report を直接差し込む経路はありません。
- 旧 object／旧17-byteの拒否、両 schema の再生成、Manifest reader・counter・policy の既存検証は計画にあります。server／Python に旧 payload の専用 decoder は検索上なく、今回の変更に伴う追加改修は見つかりませんでした。規範文書の更新対象にも追加漏れや scope creep は認めませんでした。
- UHD 4.10 source で、metadata に損失数がないこと、再開要求が現在時刻＋50 msであること、重複 overrun 通知を抑制することを確認しました。

ファイル変更、テスト・mutation 実行はしていません。新しい parity テストの実際の決定性と assertion の強さは、実装後のレビューで確認が必要です。

## 2. コードレビュー

判定：FAIL

### 指摘

1. **major — back-pressure 後の fault が、報告済み損失を再計上する**
   - **箇所:** `crates/ezsdr-mock-radio/src/lib.rs:698`、`:761`、`crates/ezsdr-mock-radio/tests/mock_radio.rs:1890`
   - **根拠・系列:** 追加された `mr_21_overrun_shape` の混在系列では、event の `lost` が `50,000 + 50,000 + 50,000 + 151,000 + 50,000 = 351,000`。実際に失う区間は sample `[1,000, 202,000)` の **201,000** で、150,000 を重複計上する。後続 summary の時刻も、既に報告した sample 1,000 に戻る。テストを実行して通過を確認したが、その期待値自体が重複を固定している。
   - **問題:** メモ36 T20 の「2 重に数えない」と衝突する。T12 が back-pressure を除外していても、T20 の要求は満たさない。`design/09-mock-radio.md:314` もこの挙動を固定している。
   - **修正:** まず whole-block 規則と T20 の衝突を owner が裁定すること。推奨は、block の `lost` は維持し、混在時の event は未報告分とその開始時刻だけを報告する例外を明文化して実装・テストを揃えること。「既知リスク」として残すだけでは、この gate は閉じられない。

2. **minor — production code のインデントが壊れている**
   - **箇所:** `crates/ezsdr-mock-radio/src/lib.rs:757–764`
   - **根拠:** `match` の終端、`let lost`、`if let Some(rx)` が一段深く、内側の `if lost > 0` と同じ字下げになっている。変更前には無かった崩れ。
   - **影響:** 損失計算と状態更新のスコープを誤読させる。brief が明示した整形要件にも違反する。
   - **修正:** この範囲だけ周囲に合わせて手作業で直す。formatter は不要。

3. **minor — Mock のテスト表で期待値・規則列が崩れる**
   - **箇所:** `design/09-mock-radio.md:312`、`:314–315`
   - **根拠:** 表は `test / expected / rules` の3列だが、追加行は操作と期待値を分けた4列になっている。315行には未エスケープの `|` もある。
   - **影響:** GitHub 表示では期待値が rules 列に入り、本来の規則参照が切り捨てられる。
   - **修正:** 操作と期待値を既存の expected 列にまとめ、flag の `|` を `\|` にする。

### 確認結果

- Radio Model 14本、Mock 62本、UHD unit 266本、新規 parity 5本はすべて通過。schema freeze も通過。
- 変更した mutation 行の置換対象は各1件。削除済みフィールド・テストへの参照は確認されなかった。
- `git diff --check` は通過するが、指摘2・3は検出しない。

未確認：実機、mutation の実行、workspace・Python 全体の gate。ファイル変更なし。

## 3. 修正の再レビュー

判定：PASS

新規指摘なし。前回の major 1件・minor 2件は解消しています。

1. **損失の二重計上・時刻を修正**
   `crates/ezsdr-mock-radio/src/lib.rs:695–701,759–769`  
   fault 自身の損失だけを集計し、時刻はその最初の `kf`。`Full` の追加損失は別途一度だけ報告されます。問題の系列は `50,000 + 50,000 + 50,000 + 1,000 + 50,000 = 201,000` となり、実損失と一致。report を運ぶ block 自体が `Full` でも再計上しません。

2. **UHD・仕様・表示を確認**
   `crates/ezsdr-radio-uhd/src/provider/rx.rs:562` は引き続き block の飛び全体を渡し、今回の修正による影響はありません。RM-17 は報告済み損失の除外を明記。残る「whole jump」は block の値、UHD、または back-pressure なしのテストを指し、矛盾なし。インデントと Mock のテスト表の3列・エスケープも修正済みです。

3. **テスト・mutation 行**
   Mock 62本、Radio Model 14本、UHD RX unit 39本が全通過。schema freeze も通過しました。F32・F43～F46 は置換対象が各1件で、指定テストの観測・期待値が変異を検出することをコード上で確認しました。

未確認：実機、mutation の実行、workspace・Python 全体の gate。ファイル変更なし。
