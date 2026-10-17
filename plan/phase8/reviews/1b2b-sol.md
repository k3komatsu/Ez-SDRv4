**差し戻し推奨：major 2件、minor 2件です。** 読み取り専用で確認しました。以下の数値シナリオは静的追跡による **INFERRED**（実行再現は未実施）です。

### 1. major — 開始待ち segment の cut を移動先が待たない
`crates/ezsdr-radio/src/timeline.rs:325–328,370–380`

`running_at` は `origin > e` の segment を除外しますが、replay はその segment を origin で cut します。

例：source に先行する cold change があり、`e=1 s`、次の origin が `1.05 s`。同じ effective instant に、空の destination への移動を `now=ready=0.5 s` で予約すると、source の cut は `1.05 s` なのに destination は `1 s` から始まれます。D3・RM-34 の release 順序に反します。

**修正案：** source の移動を仮 replay して得た実際の cut を destination の ready に使う。先行 cold と移動を組み合わせ、両 line 間の時刻関係を検査するテスト／mutation を追加する。

### 2. major — 後の予約で source の計画が変わっても destination の ready が古い
`crates/ezsdr-radio/src/timeline.rs:325–335`

source の cut は予約時に一度だけ destination の `Item.ready` に取り込まれます。その後の `Line::book` は片方しか再計画しません。

例：停止中の source（3 MS/s）から空の destination（100 MS/s）へ、`e=1.000000001 s` の移動を先に予約。その後、source に `at=0.9 s` の `start_rx` を予約すると、source は移動まで動作し、その cut は `1.000000334 s`。destination は古い ready のまま `1.000000010 s` に開始でき、channel の所有が重なります。`Start` は cold の床に拘束されないため、正当な到着順でも成立します。

**修正案：** 移動の依存関係を後続の再計画でも維持する。予約時の `running_at` 修正だけでは不足します。この予約順の回帰テストが必要です。

### 3. minor — 範囲外 channel が成功扱いで両 stream を再始動する
`crates/ezsdr-radio/src/timeline.rs:294–295,332–333`

`channel >= 64` は集合を変えませんが、両 line に cold item を追加して成功を返します。無効な移動が clock の置換や halt の解除を起こします。doc への追記だけでは、不正入力の検出にはなりません。

**修正案：** 予約前に拒否するか、検査済み channel 型／明示的な precondition と assertion を使う。拒否時に両 line が不変であるテストを追加する。

### 4. minor — layout 対応を追加した RM-13 が存在しない count key を参照する
`design/07-radio-model.md:161`

layout の TxBurst target を追加しましたが、波形の channel 数は依然として effective `radio.tx.channels` とされています。RM-27／RM-30 の configuration にはその key がありません。例えば stream が `{1,3}` を保持する場合、必要な count と interleaving の位置がこの規則から決まりません。

**修正案：** layout では対象 stream の membership 数を使い、波形内の位置と論理 channel 番号の対応を明記する。

### 確認して問題がなかった点

- 既存 Mock／UHD の `first(n)`・`.count()` への追随は、従来の有効範囲では挙動・Manifest 値を維持しています。
- `Channels::PartialEq` の bit 比較、`stream_target` の階層・番号文法は妥当です。
- clone 後に両方を確定する booking／refusal は、失敗時の原子性を保っています。
- P80–P93、F37、R04 の `old` は各対象で**ちょうど1回一致**しました。16/16 killed のログも確認しましたが、再実行はしていません。
- 既存テストの弱体化は見当たりません。ただし生成テストは line ごとの不変条件だけを検査し、上記の相互依存を検出しません。
