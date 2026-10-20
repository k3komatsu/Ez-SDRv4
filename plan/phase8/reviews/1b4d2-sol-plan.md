# 1b-4d-2：Sol の計画レビュー

# Phase 8・1b-4d-2 テスト計画レビュー

**推奨：実装前に Stop の保証とテストの観測方法を修正する。** per-stream 化の方向は妥当です。ただし、現計画では重要な誤実装を通すテストと、実装が保証できない規範文が残っています。

以下、`計画` は `/home/komatsu/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d2/test-plan.md`、`指示書` は同ディレクトリの `plan-brief.md` を指します。コードの問題は現行コードから確認し、未実装部分の帰結は必要に応じて **INFERRED** と記しました。

## 要修正

### 1. **major：Stop の再確認だけでは、掲げた保証を満たさない**

**根拠：** 指示書`:30-31`、計画`:389,478`、`crates/ezsdr-radio-uhd/src/provider/rx.rs:108-115,190-202,302-317`、`provider/mod.rs:406-420,695-704,718-722`。

計画自身が認めるとおり、再確認後に lock を離してから Stop が記帳されれば、その後でも `rx_start` を発行できます。したがって「origin が stop 以後なら始めない」という規範とは両立しません。

影響も「timed start が1回出るだけ」には限定できません。例えば origin が数十秒先なら、現行 RX は取り消された segment の停止を origin まで待ちます。join は1秒で detach し、cleanup は全 streamer を閉じません。**Stop が返った後まで owner と streamer が残り、将来の start を処理する経路があります。** 実機での開始から停止までの時間は **INFERRED** です。

**修正：**
- 「開始を許可済みとする瞬間」を定義し、Stop とその許可をどう順序づけるか裁定する。
- 再確認後→Stop→device call の系列を追加し、将来 origin、detach、最終停止まで観測する。
- 新しい状態を禁止したまま、強い保証だけを規範にする案は採らない。「UR-25 を守れば窓は閉じられない」という断定も根拠不足です。

### 2. **major：S16 TX は configure 後の再確認を検査しない**

**根拠：** 計画`:207-211`、`crates/ezsdr-radio-uhd/src/provider/tx.rs:715-728`。

S16 TX は Stop を記帳してから `do_switch()` を呼びます。この系列は、**入口でだけ Stop を確認し、configure 後には確認しない実装でも通ります**。今回塞ぐべき窓を再現していません。

また「新しい clock は開いたままになりうる」という説明は、この系列には不適切です。origin が stop 以後の TX segment は、`crates/ezsdr-radio/src/timeline.rs:892-895` が origin で終了させます。Stop より前に始まった TX clock とは区別が必要です。

**修正：**
- 実際に owner configure を通し、その戻りを gate で止めてから Stop を記帳する。
- gate 解放後に clock を採用しないことを確認する。
- booking 済みの新 clock の終了は origin であることも確認する。

unit で `tx.clock` を直接見る判断自体は適切です。e2e の `tx_send` 不在だけでは採用の有無を区別できません。

### 3. **major：S16 RX のテスト道具は、指定された場所から使えない**

**根拠：** 計画`:198-203,273-278`、`crates/ezsdr-radio-uhd/src/provider/test_support.rs:200,237-245,249-258`、`tests/fake.rs:671-674`。

`tests/fake.rs` から使う予定の `LockSpy` は private な unit-test 補助です。また内部に独自の `FakeDevice` を所有するため、`Direct::build_on(inner, tested, …)` の観測対象 `inner` とも、そのままでは一致しません。§3 の補助変更一覧に解決方法がありません。

**修正：** integration test 内に、同じ `Arc<FakeDevice>` へ転送する小さな wrapper を置く。対象を **RX・channel 1・untimed apply** に絞り、device から戻った位置で止めると明記してください。公開 API に unit-test 補助を露出させる必要はありません。

### 4. **major：S12 は `delivered` の書き先の混線を見逃す**

**根拠：** 計画`:163-167`、`crates/ezsdr-radio-uhd/src/provider/rx.rs:154,315,339`、`provider/core.rs:251-256`。

End が各 stream の `delivered` を読む unit は有用です。しかし値を手で設定するため、RX owner が stream 1 の進捗を stream 0 に書く誤実装でも通ります。

また「cold の cut に `delivered` を混ぜる誤実装を落とす」とありますが、S12 に cold 操作はありません。

**追加系列と期待結果：**
- 2本の RX owner に異なる長さの Samples を処理させ、実経路で異なる進捗を作る。
- Abort／Orderly の End を記帳し、それぞれ自 stream の進捗から cut が決まることを確認する。
- 一方を終了しても、他方の `delivered` が消えないことを確認する。

「cold ではなく End の床」という計画の訂正は正しいです。

### 5. **major：TX の隔離と per-stream 所有状態の系列が不足する**

**根拠：** 指示書`:24,29`、計画`:88-143,153-159,427-447`、`crates/ezsdr-radio-uhd/src/provider/tx.rs:162-176,749-764`。

S3・S5・S6・S8 は主に RX です。S11 の初期 burst だけでは、TX の cold change、halt、generation、report read の隔離を検査できません。

**追加すべき最小系列：**

| 系列 | 期待結果 |
|---|---|
| TX s0/s1 で送信中、s1 の rate を cold 変更 | s1 だけ clock が替わり、s0 の burst と clock は継続 |
| `Stop(tx/s1)` → 次の burst | s1 の burst だけ終了。clock は維持し、次の burst を送れる |
| s1 の reopen で generation 更新 | s1 の古い ACK 相関だけ破棄し、s0 の相関は維持 |
| s1 の `reading` 中に s1 を from-none enable | s1 は待つが、s0 の owner は device call を続けられる |

`busy`／`reading`／generation を方向全体で扱う誤実装を、少なくとも1本の交差系列で落としてください。複数 stream の VH-8 harness を 1b-4e へ送ることは支持しますが、この段階の TX 隔離まで送るべきではありません。

### 6. **major：S11 の操作と assert が矛盾する**

**根拠：** 計画`:156-157`。

進め方では s0 と s1 の両方に burst を送りますが、assert は「`tx_send 1` が出て `tx_send 0` は出ない」です。正しい実装でも成立しません。

**修正：** s1 だけ送る区間と s0 を送る区間を分けるか、異なる waveform を使って両方の送信内容・clock を対応づけて確認してください。

### 7. **major：`design/18` の amendment に、stream 全体を halt する文が残る**

**根拠：** 計画`:351-355,381-395,415-421`、`design/18-uhd-radio.md:216,241,264,279`。

変更予定にない次の文は、今回の隔離規則と合いません。

- UR-17：「configuration refusal **halts the direction**」
- UR-23：同じく「**halts the direction**」
- UR-25：同じく「**halts the direction**」
- UR-23：burst を止める宛先が `<id>/tx`／`<id>` のみで、layout の対象 stream が未記載

また UR-16／UR-26 の cleanup 文は、stream ごとに閉じられるようにも読めます。今回採る動作は **1本でも detach すれば device-wide `close_streams` を呼ばない**ことです。明記すべきです。

**DA-1 について：**
- stream 番号、port ごとの link、clock、event handle の修正方向は妥当。
- 新規の Stop 文は指摘1の裁定後に書く必要があります。
- 「configuration は settings だけを変える」という安全性の説明は、規範ではなく判断記録へ。
- move の暫定拒否は、拒否条件と結果を規範にし、導入段階・解除予定は Changes へ置いてください。

## テスト・mutation の補正

### 8. **minor：S13 は control-first の join を検査しない**

**根拠：** 計画`:171-175`、`crates/ezsdr-radio-uhd/src/provider/mod.rs:695-704`。

4 owner を gate で止め、detach 記録の順を見る方法は、TX 全体→RX 全体と stream 順を観測できます。ただし control は止めていないため、**control を最後に join しても同じ4行になりえます**。

control-first は unit の join 観測などで別途固定してください。gate は失敗時にも解放し、解放後の worker 終了まで待つ計画も必要です。

### 9. **minor：mutation の対応表に、そのテストでは殺せない案がある**

**根拠：** 計画`:148-151,429-446`。

- **S1 → `Core::register(dir,k)` を0固定：** S1 は初期 clock が中心です。`Core::register` を通る後続 TX cold change へ当てる必要があります。
- **S1 → 全 stream の rate を s0 から読む：** 異なる rate の設定が明記されていません。s0/s1 に異なる rate を与えてください。
- **S10 → released を数えない：** 全件 held の系列なので落とせません。released と held が混在する系列が必要です。
- **join 順の mutant「TX の後に RX」：** 正しい順序そのものです。RX-first 等へ訂正してください。
- **S16 TX の検査削除：** 指摘2の gate 系列に直してから対応させてください。

既存行の扱いは支持します。

| 行 | 判断 |
|---|---|
| P280・P308・P309 | **削除でよい。** 旧制限を固定する行で、対象の guard が消える |
| P313 | **equivalent を外して S6 へ再指定でよい。** s1 に line ができると channel 集合の誤選択が観測可能 |
| P344・P345 | **維持でよい。** owner open の rate 比較は引き続き必要 |

根拠は `plan/maintenance/tools/mutations.json:6564-6571,6844-6882,7175-7192`。正確な置換文字列と kill は、実装後の確認事項です。

## 判断を支持する点と残る系列

### 10. **minor：H2／H3 の VERIFIED の意味を訂正する**

- **H2 は VERIFIED に上げてよい。** UHD 4.10 `host/include/uhd/stream.hpp:242-247,391-396` が、別 streamer を別 thread から同時に呼べると明記しています。
- **H3 の「ハードウェア queue は1つ」は誤り。** `host/docs/driver_usage/timed_cmd.dox:94-115` は RFNoC block ごとの queue を明記しています。`multi_usrp_rfnoc.cpp:964-1015` の motherboard 指定は、各 chain の command time を設定・解除します。
- **それでも instance ごとの単一モデルを維持する判断は正しい。** `plan/phase8/33-channels-and-streams.md:172` が明示的に採った近似です。この段階で block ごとの会計へ広げる案は退けます。
- H1 の X3xx 実機での同時動作は **INFERRED** のままでよいです。streamer ごとの chain 構築は UHD source で確認できますが、bench の代用ではありません。

ほかの判断：

- **LATE_COMMAND の訂正は支持。** 更新／cold／早すぎる Provider start は device node、受信の missed start は stream nodeです（`design/07-radio-model.md:139`）。
- **S14 の close 抑止は支持。** `Device::close_streams` は全方向・全 streamer を閉じる APIです（`crates/ezsdr-radio-uhd/src/device.rs:216-218`）。
- **S18 の packet-size テストは成立する。** s0 未open、s1 の packet 4000、旧 decimation 200 なら、期待差 `11,400,000 ticks` は整合します。ただし両 stream の旧 rate が同じなので、「s0 の rate を誤用する」mutant までは落とせません。

独立シナリオから、さらに次を追加してください。

- **major：owner configure 中の device loss。** 最後の apply の戻りで他 stream が loss を公開し、その後に RX start／TX clock 採用を行わないこと。現在の検査位置は `provider/core.rs:734-737`、`provider/tx.rs:715-728` で、configure 後の境界を固定する必要があります。
- **major：部分 start 失敗。** s0 の開始後に s1 の `rx_start` または途中の spawn が失敗しても、cleanup が開始済み owner を回収し、残存 call がある間に close しないこと。現計画の正常起動・panic・detach 系列だけでは覆いません。
- **minor：off／空の on／halt の区別。** s1 を Stop 後に membership 更新しても再開せず、`start_rx` で再開すること。halt は成功した cold 変更で解除すること。設計33`:96` の3状態を後続 stream でも固定してください。

## 大きさ

1,600〜1,800行という見積もりだけでは分割を必須にしません。ただし「helper を同時に変えるので分割不能」という計画`:465` の説明は成立しません。

まず RX/TX 共通の試験 helper で重複を抑え、今回の追加系列を含めて再見積もりしてください。**機能変更と、それを保証するテストを別 commit に分ける案は推奨しません。**

未実施：build、テスト、mutation、実機試験。ファイル変更なし。UHD 4.10 の一次資料は container 内で確認しました。実機での残存 start の停止時間と queue 容量は未検証です。

判定：要修正
