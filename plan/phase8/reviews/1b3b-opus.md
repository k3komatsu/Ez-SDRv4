# Phase 8 段階 1b-3b — orchestrator（Opus）のレビュー

対象：作業ツリー（HEAD `9b36a91` からの diff）．報告 `report.md`．

## 判定：差し戻し（major 1，minor 2）

### O1 major — `keys` instance の `cold` 変更の床と，対象の stream の選び方が食い違う

`crates/ezsdr-mock-radio/src/lib.rs` の `handle_update`（`cold_streams` を `projected(requested_tick.unwrap_or(now))` で呼び，床は `cold_streams` の最初の stream の line だけで `timeline::command_instant` から取る）．報告 §11-1 の反例（`c1.stream` を `T+5 ms` に `1 → -1`，続けて `at` なしで `-1 → 1`）は，移動ではなく**この段階の範囲**（stream への出入り）で起きる．後から受けた `cold` が先に効き，設定と line が食い違う．line のない stream（link のない受信 stream）では床そのものが掛からないので，line の有無でも同じ逆転が起きる．spec 09 MR-18 の3つ目の *Ceiling* は既知の誤りを規範に書いたもので，残せない．

**直し方**：`keys` instance では，`cold` 変更は**同じ方向の，先に受けた `cold` 変更のどれよりも早く効かない**（床は `self.updates` に残っている同じ方向の `cold` の effective instant の最大と，受信の stream の開始．line の有無に依らない）．その床で決まった `tick` に投影した設定で `cold_streams` を選ぶ（先に受けた同じ方向の `cold` はすべて `tick` 以前なので投影に入る）．規則は spec 07 RM-25 に書く（今のモードは方向ごとに stream が1本なので同じ意味．今のモードの挙動は変えない）．1b-3c の移動の床（D3 の1：両方の line の先の `cold` の遅い方）もこれに含まれる．MR-18 の3つ目の *Ceiling* を消す．反例そのもの（line のある stream と，line のない stream の両方）をテストにし，床を掛けない mutation の行で kill を確かめる．

### O2 minor — 今のモードで，誤った宛先の `TxBurst` の拒否の文面が変わった

HEAD は `handle_tx_burst` が clock を先に調べ，送信 channel がないときは `MR-16: no transmit channel is configured` を返した．今は `handle_action` が宛先で先に振り分けるので，今のモードで「宛先が誤り，かつ送信 channel なし」のとき文面が `MR-16: target, clock, …` に変わる（テストは捕まえない）．今のモードは HEAD と同じ経路（`handle_tx_burst(0, …)` の中で宛先を調べる）に戻す．

### O3 minor — 「全 stream」の道のテスト（報告 §11-2 (a)(b)）

`keys` instance で，`<device>` 宛ての `Stop` が全 stream を止めること，`device_lost` が全 stream を止めることのテストと mutation の行を足す．(c) `start` 失敗時の巻き戻しは失敗を注入する手立てがないので記録のまま．

## 確かめて問題なしとしたもの

- 今のモードのテストの diff は空．`addressed`・`stream_set`・`prepare` の link の振り分け・`arm`・`start`・fault の選び方・event の source（`path`）は，方向ごとに stream 1本のとき HEAD と同じ経路．
- `Description::coerce` が `violations` を通すので，`prepare` と実行時の判定は1か所（`rm_31_rm_32_…` が両方を確かめる）．
- spec 09 の新規 MR-38〜MR-41 に経緯の文はない（O1 の *Ceiling* を除く）．spec 08 SE-6 の修正は妥当（設計 33 の「書き換える文書」に spec 08 を足すのは orchestrator が記録で行う）．
