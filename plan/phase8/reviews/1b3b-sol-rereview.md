修正前の全指摘と F1〜F5 を照合します。特に instance-wide floor と `hardware_timed` の混在、Stop/loss 後の扱い、mutation が意図した assertion で落ちるかを確認します。変更・mutation 実行は行いません。
## 判定：accept

前回の **Sol 1〜6、Opus O1〜O3 はすべて修正済み**です。major／blocker の残存は確認しませんでした。

### 修正の確認

| 前回指摘 | 確認根拠 |
|---|---|
| Sol 1・2・3／O1：床と投影の不整合 | `crates/ezsdr-mock-radio/src/lib.rs:1684–1735`：全 pending `cold` の最大 instant → `projected(e, order)` → stream 選択 → `coerce` の順。方向・line の有無に依存しない |
| Sol 4／O2：旧モードの拒否文面 | 同ファイル`:1331–1333,1416–1421`：旧モードは clock 検査を先に通す経路へ復帰 |
| Sol 5：TX channel count | `crates/ezsdr-mock-radio/tests/mock_layout.rs:1131–1158`：2-channel／1-channel の解釈差と片方だけの拒否を固定 |
| Sol 6：selector／MR-4 | `design/09-mock-radio.md:39–44,84–85`：モード別の既定と RM-29 引用へ修正 |
| O3：全 stream の停止 | `crates/ezsdr-mock-radio/tests/mock_layout.rs:1072–1127`：RX 2本の終了、TX 2本の held burst 取消を確認 |

`hardware_timed` は床の適用先・床の集計対象ともに除外されています。frequency／gain は RM-31／RM-32 の判定項目を変えないため、この混在から予約済み membership／throughput を無効化する道も見つかりませんでした。Action の `Stop` は pending 更新を維持し、loss／`Provider::stop` は取消します（`lib.rs:1340–1365,1054–1056`）。

### minor — T0 に床付けした更新の `LATE_COMMAND`

**箇所:** `crates/ezsdr-mock-radio/src/lib.rs:1690–1695`、`design/09-mock-radio.md:153`

**シナリオ（INFERRED）:** `start(Some(T0))` 後、`now < at < T0` で RX の `cold` 更新を受信すると、`e=T0` になりますが、`base` にも T0 が入るため `LATE_COMMAND` が出ません。MR-18 の「床付けされた `e` は通知する」という文と一致しません。既存 `timeline::command_instant` にも同じ扱いがあります（`crates/ezsdr-radio/src/timeline.rs:380–388`）。

**修正案:** T0 の床を遅れに含めるか明記し、共通 helper と layout 経路を揃えてテストする。今回の順序保証修正の受理を妨げる問題ではありません。

### Mutation／テスト

- P141〜P148 の `old` は、現ツリーで**各1回一致**。
- 対応テストは、順に「床なし」「方向限定」「asked 時点で stream 選択」「一部だけ Stop／loss」「channel count 固定」「hardware_timed まで床付け」を検出する assertion を持っています。
- 保存ログの **8行すべて killed** を確認。mutation は再実行していないため、実際の失敗箇所の判定は **INFERRED**。
- 独立実行した `mock_layout` は **21 passed**。旧3テストファイルの差分、`git diff --check` は空でした。

指定の文書群に、この minor 以外の新たな虚偽や DA-1 上の問題は確認しませんでした。SE-6 の旧形式は今回新設されたものではありません。

### 1b-3c の必須整理

`RM-34` と `book_move` は二つの line だけを床にします（`design/07-radio-model.md:353`、`crates/ezsdr-radio/src/timeline.rs:554–564`）。一方、移動を起こす `UpdateParameter` にも RM-25 の instance-wide floor が掛かります。

したがって、**第三 stream／逆方向／line のない stream の pending `cold` も含めた最終 instant を先に確定し、その instant で設定の投影・検証・両 line の記帳を行う**必要があります。`book_move` 自体の床を広げることまでは必須ではなく、Provider が確定した instant を渡す形でも整合できます。仕様とテストでその責任分担を固定してください。

ファイル変更・mutation 実行は行っていません。
