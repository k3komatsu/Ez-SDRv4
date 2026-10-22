# 1b-6b：Sol 6.1 のレビュー

## 1. テスト計画のレビュー

判定：FAIL

HEAD `2af9afd` に対する読み取り専用レビュー。実装前に、以下の計画修正が必要です。

## 指摘

1. **major — P6 の既存テストは「次が無い拒否」を通らず、追加予定の期待値も誤っている**
   - **根拠：** `test-plan.md:102` は既存テストの最後を `Idle` としています。しかし `crates/ezsdr-radio-uhd/src/provider/tx.rs:1550-1553` は変更を3個予約し、`:1578` の3回目の switch は3個目の segment を採ります。次が無い状態ではありません。
   - **系列：** 現在の系列へ予定どおり `Idle` の assert を足すと、正しい HEAD 相当の実装が失敗します。一方、最後の configure が拒否される系列は未固定です。
   - **修正：** このテストの最後は3個目の `Active` として検査する。別途、後続の無い configure 拒否を作り、旧 segment の完了、clock 不在、最終 `Idle` を検査する。

2. **major — P4 の復帰先 identity が誤り、loss 後の phase 観測も抜けている**
   - **根拠：** `test-plan.md:100` は再開後を `by: Some(1)` としています。しかし既存テストは command 1 の rate change を拒否し、command 2 の `Start` で再開します（`crates/ezsdr-radio-uhd/src/provider/rx.rs:1445-1456`）。
   - **系列：** 拒否された command の identity を `active` に残す誤実装を、予定の assert が正解として扱ってしまいます。また同じ計画行は「拒否／loss」を掲げますが、phase の追加観測は拒否だけです。
   - **修正：** 再開後は `Some(2)` を検査する。既存の configure 中 loss テスト（`rx.rs:935-940`）にも、戻った直後が `Pending`、active/floor 不在である assert を足す。

3. **major — P1b のテストが、tuple を落とす根拠となる実際の plan 消失を観測していない**
   - **根拠：** N3 は最初から line を入れない準備です（`test-plan.md:138-144`）。実際の P1b は、存在した T0 segment が別 owner の loss によって消えた状態です。さらに §5 の「その間はどの thread も動かない」は誤りです。`Provider::start` は RX を1本ずつ生成して直ちに spawn します（`crates/ezsdr-radio-uhd/src/provider/mod.rs:633-664`）。
   - **系列：** 後続 stream の `rx_start` 中に先行 owner が loss を publish し、その後 `Rx::new` が呼ばれる。HEAD は実時計の tuple を保持しますが、提案は `first_undelivered == None` にします。line 無しの N3 では、この差が End の結果を変えないことを確認できません。
   - **修正：** T0 の line を作り、T0 前の loss を実際に book して segment を消してから `Rx::new`・finish を通す。phase、完了記録なし、loss の End/cut が変わらないことを検査する。§5 の初期化順序の説明も訂正する。

4. **minor — N7 は決定的だが、指定された Abort 系列ではない**
   - **根拠：** `test-plan.md:188` の `end(..., false, false)` は Orderly です。指示書 P11 と設計メモ §2 R2 は Abort を要求しています。
   - **系列：** 現案は「delivered が End の instant より先」という Orderly の floor を検査します。Abort が instant より前の delivered で切ることは、このテストでは観測しません。
   - **修正：** Abort を使い、End の instant を stream 0 の delivered より後に置く。stream 0 が instant ではなく delivered で切れることを assert する。現在の Orderly ケースは残しても構いません。

5. **minor — N4・N5 が掲げる誤実装の一部を、記載された操作では落とせない**
   - **根拠：** N4 は一度も `Active` を作らないのに「前の Active を消さない」を殺すとしています（`test-plan.md:151-155`）。N5 は `Pending` 中に `delivered` を呼ばないのに「Pending にも書く」を殺すとしています（`:162-168`）。
   - **系列：** その不正な分岐だけを変えた実装は、記載された assert をすべて通ります。
   - **修正：** 各前提状態から該当操作を1回実行して不変性を確認するか、到達不能なら殺す誤実装の主張から外す。

## その他の確認結果

- **tuple の削減：** 通常の RX 成功経路では、origin・channels・整数比 `N/1` が一致することをコードで確認しました（`control.rs:852-853`、`rx.rs:76-84,340-346`）。この経路での不一致は見つかっていません。`segment: None` は tuple 自体が同値ではないため、指摘3のように **End の結果が同値**である根拠を固定してください。固定できなければ HEAD の tuple を保持するのが安全です。
- **四つの食い違い：** TX 初期化を Pending のままにする判断は妥当です。RX 二段初期化の採用も妥当ですが、thread に関する説明は訂正が必要です。clock あり・segment なしを Active とする判断も HEAD と整合しますが、N6 はその枝を検査していません。
- **54か所の置換：** 一覧に実コードとの明白な漏れは見つかりませんでした。到達不能な `idle && active` の組を、Pending と Active の拒否に分ける方針も妥当です。
- **mutation：** J46/J54 の維持、P258/P404/P445/P382 の当て直し、P470/P472 の `finished` への移動は方向として妥当です。P472 の全 stream 消去の観測は thread 無しで決定的になります。ただし移動後の行自体は `Rx::finish → finished` の呼び出し削除を変異しないため、配線を殺す行も明示してください。
- **範囲：** Claims、device record、`needed_before` 等への範囲拡大は見つかりませんでした。P10 の既存系列を残す方針も妥当です。

テスト・mutation・実機確認は未実行です。計画段階のため実際の kill は未検証（INFERRED）。ファイル変更はなく、作業ツリーは clean のままです。

## 2. コードレビュー

判定：FAIL

major 0 件、minor 1 件。本体の挙動変更・新たな競合は見つかりませんでしたが、P472 の当て直しが指定された検証意図を維持していません。

1. **minor：P472 が「他 stream の floor を消す」変異でなくなっている**
   - **場所：** `plan/maintenance/tools/mutations.json:7645-7652`
   - **証拠：** 現在の `new` は phase を変更せず、`None` を返します。「全受信 stream の delivered を消す」という名前・旧変異とは別の誤実装です。対象テストも `crates/ezsdr-radio-uhd/src/provider/core.rs:2103` の戻り値検査で失敗するため、Abort の cut に至る前に終了します。
   - **系列と影響：** stream 1 の終了が stream 0 の floor を消し、Abort が sample 3000 でなく 4000 で切る、という P472 固有の検証を、この行の killed 結果から確認できません。P470 は同じ機構を保護していますが、今回要求された決定的な Abort テストへの当て直しにはなっていません。
   - **最小修正：** P472 の `new` を P470 と同じ「全 RX stream の phase を消す」変異に戻し、現在の決定的な filter は維持してください。当該行をリモートで再実行します。

**確認済み**
- HEAD `2af9afd` と production の全変更を比較し、初期化、終了→再探索、switch・拒否、loss、release、from-none の経路を追跡。
- P1b・P4・P6 の追加観測と、P472 用テスト自体の floor／Abort cut の検査は実装済み。
- mutation 全900行の `old` が各対象にちょうど1回一致。対象 filter と UR-17・UR-25・UR-26 の Test 名は実在。
- 独立実行：lib **259 passed**、fake **264 passed**。`git diff --check` も成功。

未確認：mutation の実行、UHD feature・実機、workspace 全体の gate。ファイル変更はしていません。
