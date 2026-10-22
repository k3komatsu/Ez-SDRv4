# 1b-7a：Sol 6.1 のレビュー

## 1. テスト計画のレビュー

判定：FAIL

**major 2件、minor 2件。** 方針と対象範囲は妥当ですが、取消し後の予約残留と TX stream 間の独立性を固定する観測が不足しています。

### 独立に挙げた故障系列

計画を読む前に、現行コード・設計メモ・spec から次を確認対象としました。

- 呼び出し元が取消し原因を取り違える。
- 現在 clock／将来 clock の取消しが enum を迂回する。
- 一つの stream の Stop が別 stream の予約を消す。
- 取消しを記録しても、送信または予約が残る。
- loss／`Provider::stop` が誤って報告する。
- cold change、特に未開始 clock の取消しが報告しなくなる。

### 指摘

1. **major — cold change の予約残留を検出できない**
   - **根拠：** `test-plan.md:40,43,47`。T4・T14a はイベント、取消し行、無送信を確認しますが、内部の予約解除確認は Stop／ProviderStop に限っています。現行 UHD は `crates/ezsdr-radio-uhd/src/provider/tx.rs:251,258,709` の `forget` で共有予約を除去します。
   - **故障例：** cold change の取消しで `forget` だけを落とすと、イベントと行は正しく、送信もされませんが、`core.held` に予約が残ります。計画された T4・T14a の観測では検出できません。
   - **修正：** 現在 clock と skipped future clock の cold-change unit test に、対象の `held`／`later` と共有予約が除去された assert を追加してください。loss 後の shutdown も同じ予約解除観測の対象にしてください。

2. **major — TX stream の Stop が他の TX stream を取消す系列がない**
   - **根拠：** `test-plan.md:37–38,45–47`。T1 は s0 の取消し、T2 は全 stream の取消し、T15a は RX→TX の独立性です。TX→別 TX の独立性は計画されていません。実際の振り分けは `crates/ezsdr-radio-uhd/src/provider/control.rs:354–372` にあります。
   - **故障例：** TX-stream Stop を全 TX stream に送る実装でも、これらの系列は期待どおりになります。
   - **修正：** 両 Provider で s0・s1 に未来 burst を保持し、s1 だけ Stop。s1 の取消し行が1件、s0 は取消されず実際に送信されることを比較してください。既存 Mock の `crates/ezsdr-mock-radio/tests/mock_layout.rs:744–754` は補助になりますが、UHD 側の代わりにはなりません。

3. **minor — skipped future clock の現行挙動の説明が誤り**
   - **根拠：** `report.md:5`、`test-plan.md:43` は「報告も行もない」としています。しかし `crates/ezsdr-radio-uhd/src/provider/tx.rs:251–252` は `forget` の直後に `command_rejected` を呼んでいます。
   - **結論：** この点は orchestrator の読みが正しいです。T14a は既存挙動の回帰テストであり、欠落イベントを新設する修正ではありません。
   - **修正：** baseline gap と「実装前には必ず失敗する」という説明を削除し、既存の `"UR-25: its transmit clock was never started"` を保持する旨に直してください。

4. **minor — enum を迂回する取消し経路の検査がない**
   - **根拠：** `test-plan.md:41` の表テストと各 parity test は出力を確認しますが、共通判断への集約そのものは確認しません。現行の直接報告箇所は Mock `lib.rs:1347`、UHD `tx.rs:252,710` にあります。
   - **故障例：** bool 引数だけ enum に置換し、これらの直接報告を残しても、期待出力は変わらずテストが通ります。今回の主目的である規則の一元化が未達になります。
   - **修正：** 実装レビュー項目に、全取消し箇所の列挙と「原因の選択以外に報告可否を決める分岐・直接報告が残っていない」ことのソース検査を追加してください。受信時拒否など範囲外の経路は除外します。

### その他の確認結果

- mutation の `old`／`new` を指定語で検索し、**24行の一覧は一致**しました。F27・R07 の当て直し対象と、他行を原則維持する判断は妥当です。
- UHD release-wait test の予約・行・順序確認を残し、イベントを「なし」に変える計画は適切です。`fake.rs:6260` の `|| true` を実観測に置き換える方針も支持します。
- `fake.rs:6225` は「RX Stop が TX を取消さない」という否定の確認であり、旧報告仕様を前提とする肯定の assert ではありません。既存の送信確認とともに維持できます。
- 到達可能な系列の削除、D2・Kernel・版管理への範囲拡大は計画にありません。
- parity harness が両 Provider を駆動する構造は確認しました。ただし、新規テストの同期方法・実際の観測能力はコード未作成のため **INFERRED**。固定 sleep を成立判定にせず、booking 完了と取消し完了の観測を明確にしてください。

ファイル変更・テスト・mutation 実行はしていません。実装開始前に上記の計画修正が必要です。

## 2. コードレビュー

判定：FAIL

**major 0件、minor 1件。** 本体の報告規則は設計メモ36に沿っていますが、追加テストの barrier に修正が必要です。

### 指摘

1. **minor — 2回目の barrier が前回のイベントで成立する**
   - **箇所：** `crates/ezsdr-acceptance/tests/parity.rs:1231–1235`（`uhd_barrier`）、同ファイル `:1843–1847`。
   - **根拠：** `uhd_barrier(token)` は token に対応する Action を投入しますが、待つ条件は単に `kind == "set_timer"` の拒否イベントです。`wait_for_events` は過去のイベントを `seen_events` に保持するため、2回目は前回の拒否だけで戻れます。
   - **系列：** T14a の barrier 107、T14b の barrier 109 は、それぞれ106・108のイベントで成立し、直前の burst／cold change／Stop の booking 完了を確認せず時刻を進めます。実際の不安定化は **INFERRED**（今回の実行では再現なし）ですが、計画で要求した順序の観測にはなっていません。
   - **修正：** 今回の Action ID を照合するか、barrier ごとの期待累積件数まで待ってください。前回のイベントだけでは次の barrier が成立しないことも固定してください。

### 確認結果

- Mock の `stop_tx`・`cancel_later`・`tx_switch`、UHD の `stop_now`・`do_switch`・`adopt_later` は共通の `cancellation_reports` を使用しています。対象の取消しに独自の報告判断は残っていません。
- 本体の意図的な挙動変更は UHD Stop の取消しイベント抑制です。Provider ごとの理由文字列、受付拒否、late policy、underflow の処理は維持されています。
- UHD loss が shutdown で `ProviderStop` を渡すのは妥当です。既存の終了経路と理由を維持し、別途 `DEVICE_LOST` が通知されます。現状 `Loss` の production 呼出しは Mock のみです。
- `shutdown(&mut self)` 後に処理を続ける production 呼出しはありません。`run` は直ちに return します。今回の変更に test／通常 build 間の本体分岐追加もありません。
- 現在／将来 clock の予約解放、cold change・loss、s1 Stop 後の s0 送信を観測するテストが追加されています。
- mutation 全リストの `old` は各1箇所に一致しました。F27・R07・P561–P572 の filter は存在し、変異内容と観測の対応も確認しました。
- RM-16・UR-23・UR-26 の変更内容と Test 名を確認しました。MR-18・MR-25 に今回の変更で不正確になる記述は見つかりませんでした。

### 実行確認

指定の review target と `timeout 300` を使用し、次を実行しました。

```text
cargo test -p ezsdr-radio -p ezsdr-mock-radio \
  -p ezsdr-radio-uhd -p ezsdr-acceptance
```

**844 passed、0 failed**。parity 30本、UHD unit 263本、fake 264本を含みます。

未確認：mutation の実行、実機、UHD feature build、Python、全 workspace gate。ファイル変更はありません。
