**差し戻し推奨：major 2件、minor 1件。** 読み取り専用で確認しました。以下の新規シナリオは静的追跡による **INFERRED**（実行再現なし）です。

### 旧指摘の判定

| 指摘 | 判定 | 根拠 |
|---|---|---|
| Sol 1：開始待ち segment の cut | **修正済み** | `State::cut` が origin の cut も記録。回帰テストは `tests/timeline.rs:648`、P95。 |
| Sol 2：後の予約への追随 | **部分修正** | 移動 item 自身には追随する（同:664、P94）。別の開始 item は対象外で、下記①が残る。 |
| Sol 3：範囲外 channel | **修正済み** | 専用 error、全体不変の検査あり（同:767、P99）。 |
| Sol 4：layout の波形 channel 数 | **修正済み** | RM-13 が集合の要素数・昇順対応を規定。受信側には下記③。 |
| O1：不可能な移動 | **修正済み** | 4種類を予約前に拒否し、全体不変を assert（同:767、P98–P101）。 |
| O2：`config_at` 公開 | **修正済み** | 移動前後・prune 後を検査（同:786、P91）。 |

指定の **20 mutation 行はすべて `old` がちょうど1回一致**。P96以外は、指定テストの assertion が対象の誤実装を検出する構造です。最終ログも19 killed／P96 equivalent と一致します。mutation は再実行していません。

### 新規指摘

#### ① major — 後続の開始 item に移動依存が維持されない
`crates/ezsdr-radio/src/timeline.rs:562–575`  
`design/07-radio-model.md:354`

`settle` が直すのは移動 item の ready だけです。

例：両RXを停止 → 移動を予約 → destination の `start_rx` を予約。この時点では source の cut がなく、Provider の義務にも違反しません。その後、source に移動より前の `start_rx` を予約すると、移動 item の ready は更新されますが、destination の開始 item は更新されません。既存の3 MS/s→100 MS/sの条件なら、source の cut **1,000,000,334** より早い **1,000,000,010** に destination を開始できます。

**修正：** 各 item の元の ready を保持し、移動以後の destination の開始にも現在の source cut を `Lines` 内で適用する。Provider に一度計算させる方式は不十分です。prune 後にも必要な release の床を残すこと。生成テストの `check_move`（同テスト:854）は移動自身の segment しか検査せず、この反例を捕まえません。

#### ② major — release 用 ready の更新が、TXの「終了前に予約済み」を書き換える
`crates/ezsdr-radio/src/timeline.rs:573,681–682`

TX replay は `item.ready >= ended` を使って終了後の予約を除外します。そこへ source cut を上書きするため、**準備完了時刻と予約時刻が混同**されます。

例：両TXに800 msの将来の `End` を記録。500 msに、1,000 msでの移動を予約する。source の cut は1,000 ms。destination の ready が500→1,000 msへ更新されると、その item は `End` 後の予約扱いでスキップされ、移動で記帳すべきTX clockが消えます。VH-2の「終了前に予約した clock と cut は残す」に反します。

**修正：** 予約／終了との前後関係を判断する値を、release を含む準備時刻から分離する。将来の `End` を挟むTX移動の回帰テストと mutation を追加する。

#### ③ minor — RM-3 が block の count を集合として規定
`design/07-radio-model.md:57`

layout の block の `channels` を「集合」としていますが、実際は `u16` の**要素数**です（`crates/ezsdr-kernel/src/stream/block.rs:148–149`、SC-10）。集合 `{1,3}` なら `channels = 2` で、位置0・1が論理channel 1・3に対応します。

**修正：** RM-13と同じく「集合の要素数」に変更。SampleClock の stream ID の例にも `/rx/s<k>`・`/tx/s<k>` を加える。

### その他の確認

- **P96：** instance内で一意・到着順の `seq` を守る限り、移動の記帳順は依存の位相順となり、1巡で足りる説明は妥当。逆順の `seq` は追加反復で処理できても、過去の移動の effective instant が片側だけ変わり得るため、「規約違反も while が救う」とは言えません。
- **cut／prune：** 開始待ち・RXのゼロsample・TXの cut は記録され、pruned cut も `start.cuts` に保持されます。prune の契約を守る限り、後の予約が畳んだ移動の source cut を変更する経路は見当たりません。
