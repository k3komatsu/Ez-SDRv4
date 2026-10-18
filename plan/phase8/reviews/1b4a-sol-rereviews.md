# 1b-4a の Sol 6.1 の再レビュー

## 再レビュー 1（修正の回 1 の後）

縮小・並べ替えの修正は確認できました。ただし、期限を過ぎてから front end を spec に戻すと、破棄されるべき予約が復活する経路が残っています。handle の解放経路は妥当ですが、新テストは leak を検出できません。
**判定：修正が必要です。major 1件、minor 2件。現段階の gate 前に閉じるべきです。**

### 指摘

1. **major — spec に戻った front end に、期限切れの予約が復活する**  
   `crates/ezsdr-radio-uhd/src/device.rs:831–856`、`:714–719`  
   B に時刻 T の変更を予約 → `"A:0"` に縮小 → T を過ぎるまで待つ → `"A:0 B:0"` に戻す → timed `apply` で settle、という順序では、B が不在だった T の予約が復帰後の B に適用されます。`set_subdev_spec` が期限到来済みの queue を処理せず、`settle` が**処理時点**の spec を使うためです。UR-33 の「その instant に不在なら何もしない」に反します。既存テストは復帰を扱いません。  
   **修正案：** 成功する spec 変更の直前に旧 spec で settle し、期限の前／後に復帰する両ケースをテスト・mutation で固定する。**INFERRED：再現実行はしていません。**

2. **minor — handle leak を戻しても新テストが通る**  
   `crates/ezsdr-radio-uhd/tests/uhd_api.rs:43–54`、`design/18-uhd-radio.md:116`  
   `made_subdev_spec` の失敗時の free を削除しても、テストが読む error は同じです。解放経路自体はコード上妥当ですが、採用 finding 1 の誤実装を検出できません。また UHD の `uhd_subdev_spec_free` は解放後に null を代入するため、同じ変数への二度目の free が必ず abort するという説明も正確ではありません。  
   **修正案：** 既存 FFI テストと同様の置換で make/free を観測し、成功・解析失敗・set 失敗時の解放回数を確認する。Test 行も観測範囲に合わせる。valgrind 自体は必須ではありません。

3. **minor — 新たな fake の境界規則にテスト・mutation がない**  
   `crates/ezsdr-radio-uhd/src/device.rs:1110–1114`、`:1154–1159`、`:1588–1594`  
   streamer 不在時の `rx_packet_samples == 0`、`rx_recv` の clock wait、`tx_async` の timeout wait は、報告どおり未固定です。即時 return に戻すと manual-clock の利用者が時間を進められない危険があります。  
   **修正案：** packet 数を非ゼロに設定した不在ケースと、participant 上での待ち時間を直接確認し、それぞれ mutation を追加する。今回導入した規則なので持ち越さない。

### fix-report §8 の扱い

- **gate 前：** 上記の packet-size／wait coverage、free の実行検証。
- **1b-4b まで可：** stream 0 の呼び出し、`Scripted` の単一 stream、0-channel cold change の整理。
- **1b-4c まで可：** antenna read-back の記録・失敗経路、spec 設定順序、`opened` の実機との対応確認。
- **後続／BP0 まで可：** `close_streams` 後の接続履歴、RX のない装置の serial、pre-chain rate・reconnect 文面・front end 名・antenna の実機確認。実機挙動は **INFERRED**。段落長は任意の整理です。

### 確認して問題を見つけなかった点

- 空リストは両方向とも検査前に `[0]` に正規化され、範囲・二重 open 検査を通ります。
- booking の宛先、縮小・並べ替え、queue の順序、`set_time_zero` の queue／順序値の clearing は妥当です。
- subdev handle はコード上、成功・make 失敗・set 失敗の各経路で一度解放されます。
- stop 後の継続／停止と TX 側切断のテストは強化済みです。
- **994 mutation 行すべての `old`／`old2` が各対象に一度だけ一致**。P209、P230–P235 の誤りと assertion の対応も確認しました。
- provider 差分は stream 0・`0..n` への機械的変更。旧 call signature、今回の規範文の理由・段階説明は除去されています。
- 保存ログは workspace・feature UHD・Python 成功、mutation **993/994（既知 G09 のみ生存）**と一致します。ただしテスト再実行、HEAD との Manifest／event の実測比較はしておらず、完全な挙動不変は **INFERRED** です。変更・ビルド・mutation 実行はしていません。

## 再レビュー 2（修正の回 2 の後）

**判定：PASS。前回の3件は修正済み。今回の gate を止める新規指摘はありません。**

### 指摘・持ち越し
- **gate 前に必要：** feature `uhd` の mutation 未対応を handoff に記録すること。対応実装自体は後続で可。`UhdDevice` の空 channel 列の検証は BP0 待ちで可。報告 §9 のその他の項目も、記載された 1b-4b／4c／BP0 への持ち越しで妥当です。

### 妥当と確認した点
- **期限切れ予約：** `crates/ezsdr-radio-uhd/src/device.rs:835–858` は、解析成功後、旧 spec のまま settle してから置換します。`:1792–1825` は期限後の復帰で破棄、期限前の復帰で適用を検査し、P236 が前者の誤実装を捕捉します。
- **隣接する系列：** 静的追跡では以下も妥当です。
  - 拒否 spec は settle・切断・置換をせず、呼出ログだけ残す。**拒否時の settle は無観測ではありません**。後続の untimed `apply` が返す設定値や queue の内部状態を変え得ます。
  - 変更瞬間ちょうどの予約は `<= now` により旧 spec で処理。
  - 連続変更でも各回の旧 spec で処理。期限前に離脱・復帰した予約は保持。
  - 他方向の予約も settle されますが、その方向の spec で処理され、未来の予約は保持。
  - `set_time_zero` は queue と `last_command` を消去（`:903–906`）。
  
  これらの追加系列は再現実行しておらず、実行結果は **INFERRED**。専用テストで固定されているとは評価していません。
- **境界のテスト：** `device.rs:1829–1865` の非ゼロ packet 設定と manual-clock の実時間差 assertion は、P237–P241 の誤実装を検出します。P236–P241 はすべて default build の `--lib` 対象です。
- **handle：** `crates/ezsdr-radio-uhd/src/uhd.rs:1373–1397` は成功・make 失敗・set 失敗それぞれの解放を観測し、free 削除を検出します。feature `uhd` の mutation 行がない点は明示された例外です。
- **FFI double：** `uhd.rs:1095` の `#[cfg(test)]` 内に限定。既存 FFI テストが今回の置換関数を実 libuhd の代わりに呼ぶ経路は見当たりません。`tests/uhd_api.rs:46–57` は別 binary で実 libuhd の経路を維持しています。
- mutation 全1000行の一致を再確認済み。保存ログも P236–P241 kill、999/1000（既知 G09 のみ生存）、feature UHD の111＋5 tests成功と一致します。

変更・ビルド・テスト再実行・mutation 実行はしていません。実機での parity は **INFERRED** のままです。
