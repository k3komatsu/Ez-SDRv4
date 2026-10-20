# Opus のレビュー：Phase 8 段階 1b-4a（作業ツリー，HEAD `b8fdea0`）

読んだ範囲：`git diff HEAD` の provider・device.rs・uhd.rs の全差分，spec 18 の差分（word diff），実装役の報告．Basic/LF の front end の表と名前は image の UHD 4.10 のソース（`db_basic_and_lf.hpp:26-27`，`.cpp:104-107`）で確かめた．テストと mutation は実装役のログだけを見た．

## 判定：major 1件，minor 2件

### O1（major）：空の channel 列で fake が HEAD とも `UhdDevice` とも違う
- 箇所：`crates/ezsdr-radio-uhd/src/device.rs` の `rx_open`/`tx_open`（`RxStreamer::new(channels.to_vec())`），`tx_send` の buffer 数の検査．`provider/tx.rs:207,412,414,485`，`provider/rx.rs:281`，`provider/tx.rs:657`．
- 筋書き：channel 数を 0 にする cold change で，provider は `tx_open(0, &[])`（受信は `rx_open(0, &[])`）を呼ぶ．provider の送信は 0 channel を 1 channel として扱う（`self.channels.max(1)`：end-of-burst は buffer 1 つ）．
  - HEAD の fake：`tx_channels = 0` で buffer の数を見ず，受け取った．受信は `rx_channels.max(1)` で 1 本の vector を返した．
  - 今の fake：channel のない streamer を開くので，`tx_send` は「1 buffers for 0 channels」で失敗し，`device_failed` になる．受信は vector 0 本の `Samples` を返す．これは UR-2 の「1 本以上・同じ長さ」に反する．
  - 今の `UhdDevice`：UHD と同じく空を channel 0 と読み，1 channel の streamer を開くので受け取る．
  - つまり，この経路で `UhdRadio` の fake の上の振る舞いが HEAD から変わり，fake と `UhdDevice` も食い違う．「この段階では挙動を変えない」と「fake は UHD より緩くも違いもしない」の両方に反する．
- 直し方：fake も UHD と同じく，空の列を channel `[0]` と読む（`rx_open` と `tx_open` の両方．二重の open の検査も channel 0 として行う）．
  - UR-33 の「fake opens a streamer with none … whose sends are refused either way」を，「UHD と同じく空の列は channel 0」という 1 つの規則に直す．Changes 行も合わせる．
  - テスト：`rx_open(0, &[])` の後の `rx_recv` が 1 本の vector を返すこと，`tx_open(0, &[])` の後の 1 buffer の `tx_send` が受け取られること，channel 0 を別の stream が開けないこと．mutation の行を 1 つ（空の列を空のまま開く形）．

### O2（minor）：UR-33 の規範文に段階の話と意味の取れない句がある
- 箇所：`design/18-uhd-radio.md` の UR-33 の streamers の段落と，その次の段落．
  - 「the one caller of this stage that can name none is …」は段階の話（DA-1 違反）．O1 の直しで，この文ごと消える．
  - 「(the M-rules below need only the first)」は何を指すのか読み取れない．消すか，何の規則かを書く．
- Changes 行の「`UhdDevice` … holds one streamer per direction」は誤り．正しくは「stream の番号ごとに 1 つ」．

### O3（minor）：fake の振る舞いの変更で，報告にないもの
次の 2 つは `UhdDevice` に合わせた変更で，正しい向きです．ただし，報告の「指示書から外れたところ」に書かれていません．
- `rx_packet_samples` が，streamer がないとき 0 を返すようになった（HEAD の fake は `rx_packet` を返していた）．`control.rs:194` の受信の call 項に効く．
- 予定された `TxReport` の fault が，送信の streamer が開いているときだけ出るようになった．
- 直し方：報告に足すだけでよい．コードは今のままでよい．

## 確かめて問題がなかったこと
- provider の呼び出しの書き換えはすべて stream 0・`0..n` で，機械的だった．`set_subdev_spec` は呼ばれていない．
- `describe()` の channel 数は，既定の UBX 2 枚でも CBX＋空の slot B でも HEAD と同じ（2）．`front_end` の名前も同じ．
- FFI の宣言と header の対応（`uhd_usrp_get_rx_info`，`uhd_meta_range_*`，`uhd_usrp_get_*_antenna`，`uhd_subdev_spec_*`）．解放は成功と失敗の両方の経路で 1 回ずつ．
- `UhdDevice` のロックの順序は control → map → streamer で，新しい呼び出しも同じ順序．
- Basic/LF の表（受信 `0`・`1`，送信 `0`）と名前 `BasicRX (n)`・`LFRX (n)`・`BasicTX (n)`・`LFTX (n)` は UHD 4.10 と一致する．
- loopback を channel ごとにしたこと（受信 `c` ↔ 送信 `c`）は，指示書のとおり．

## 裁定（orchestrator）

Sol の独立レビュー（[1b4a-sol.md](1b4a-sol.md)：major 3，minor 3）と合わせて，全件を採った．O1 は Sol の 2 と同じ指摘．UHD 4.10 のソースで Sol の 1（`subdev_spec_c.cpp:31-36`：make は handle を確保してから解析する）と 4（`subdev_spec.cpp`：区切りは ASCII の空白だけ）を確かめた．指示書と報告は [1b4a-fix-briefs.md](1b4a-fix-briefs.md)・[1b4a-fix-reports.md](1b4a-fix-reports.md)．

- 修正の回 1：7 件．Sol の再レビュー 1（[1b4a-sol-rereviews.md](1b4a-sol-rereviews.md)）は major 1・minor 2．
  - major：外れていた front end に期限が来た予約が，spec に戻った front end に効く．
  - 3 件とも採った．
- 修正の回 2：3 件．Sol の再レビュー 2 は PASS．
- 持ち越し：
  - feature `uhd` のコードは mutation の gate の外（handoff §3）．
  - `UhdDevice` の 0 channel の経路は BP0．
  - 1b-4b・1b-4c への持ち越しは，修正の報告の §9 のとおり．

指摘の数え方（AGENTS.md §9）：

| 出どころ | major | minor | 仕組み（[failure-mechanisms.md](../../maintenance/failure-mechanisms.md)） |
|---|---|---|---|
| セルフレビュー（Haiku 5.5，9 回） | — | — | 空の列の食い違いは 1 回目と 2 回目で指摘したが，報告で持ち越された |
| Opus | 1 | 2 | 空の列：fake と device で同じ規則を別々に持つ |
| Sol | 3 | 3 | handle の解放：`failure-collapsed-into-value` に近い FFI の ownership．予約の宛先：`correlation-by-partial-key`（channel の番号で引く） |
| Sol 再レビュー 1 | 1 | 2 | 期限切れの予約の復活：`planned-state-read-as-actual`（spec 22 VH-2 の「その時刻に記録」に沿って直した） |
| Sol 再レビュー 2 | 0 | 0 | — |
