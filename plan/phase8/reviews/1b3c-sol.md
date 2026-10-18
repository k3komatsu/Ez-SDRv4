先に現ツリーを独立にレビューし、所見をこの会話に記します。その後で指定の2報告を読み、O1–O5の修正を照合します。ファイル変更やmutation実行は行いません。
### 独立所見（指定の2報告を読む前）

- **S1 — minor：旧モードの `radiated` の選択規則が変わっています。**  
  `crates/ezsdr-mock-radio/src/channel.rs:93–104`：HEADは時刻を満たす最新burstに要求channelがなければ `None`、現コードはそのburstを飛ばして古いburstを探します。「古いburstにはc1、新しいburstにはc0だけ」という状態では結果が異なります。公開Actionからこの差を観測できる到達経路は未確認（**INFERRED**）ですが、旧モード不変という要件はコード上保存されていません。旧モードだけ従来の選択規則を残し、複数世代を含む回帰テストを追加するのが安全です。M50の現テストは単一burstの欠落channelを検査するため、この差を固定しません。

- **S2 — major：segmentのcoherenceがuntimed再設定をまたいで有効のままになります。**  
  `crates/ezsdr-mock-radio/src/lib.rs:1055–1082,1295–1298,2049–2092`：`phase_fixed`はtimed tuneで立った後、membership変更／再enable時に失効しません。UBXの全channelをtimed tune → channelをstreamから外す → 再加入すると、新segmentも直ちに `timed_tune, true` を名乗ります。一方、D13は次のuntimed retuneで失効とし、hardware側の再設定は周波数を含めuntimedです（`plan/phase8/33-channels-and-streams.md:250`、`design/18-uhd-radio.md:185,200`）。Mockが強すぎる保証を出す経路と判断します（実機挙動は未実測、**INFERRED**）。再設定時の失効と、必要なら位相の再描画を明文化してテストしてください。

確認できた点：instance-wide floor → 投影・coerce → `book_move(Some(e))` の順序、MoveError時のlineの非変更、両TX streamの切替、linkless RXの出版抑止、論理channelとblock位置の対応は整合しています。変更されたmutation 44行の`old`は各1回一致し、旧3テストファイルに差分はありません。build・テスト・mutationの再実行はしていません。

この所見を固定したうえで、指定の2報告を照合します。
## 判定：要修正 — major 1件

### S2 — major：再enable後もcoherenceを過大に宣言する

**箇所：** `crates/ezsdr-mock-radio/src/lib.rs:1055–1082,1295–1298,2073,2089`

UBXの全channelをtimed tune → streamから外す → 再加入すると、`phase_fixed`が残り、新segmentも直ちに`timed_tune, true`になります。

D13はuntimed retuneでの失効を要求し、hardwareの再設定は周波数を含めuntimedです（`plan/phase8/33-channels-and-streams.md:250`、`design/18-uhd-radio.md:185,200`）。

**修正案：** untimed再設定に対応するcoherenceの失効を実装・明文化し、再enable／move後はtimed tuneの再成功まで無効になるテストとmutationを追加する。コード上の状態保持は確認済みですが、実機での位相変化は未実測（**INFERRED**）です。

### S1の扱い

旧モードの`radiated`の分岐差は、同世代のchannel集合不変と旧世代のcutを踏まえると、到達可能な出力差を確認できませんでした。**不具合指摘としては撤回します。**

## O1–O5の照合

| 指摘 | 判定・根拠 |
|---|---|
| O1 | **修正済み**。旧モードのRX origin床と通知の例外を明記（`design/09-mock-radio.md:153–155`）。 |
| O2 | **修正済み**。旧モードのlinkless RXは`line()`計算前にcontinue（`lib.rs:2529–2539`）。 |
| O3 | **修正済み**。コメントの根拠を実装でも確認：Running前のAction拒否、全start後のschedule dispatch（`admission.rs:53`、`pipeline.rs:451–469`）。 |
| O4 | **修正済み**。loopbackの100非零sampleを検査し、join省略を検出（`mock_layout.rs:808–822`、P174）。 |
| O5 | **修正済み**。`COMMAND_REJECTED`件数を検査（`mock_layout.rs:1357–1358`、P173）。 |

## 確認して問題なかった点

instance-wide floor、投影後のmove判定、MoveErrorの非変更性、両TX streamの切替、linkless RX、論理channel対応は整合しています。変更mutation 44行は各1回一致し、保存ログも44/44 killedです。旧3テストファイルは変更なし。build・テスト・mutationは再実行していません。
