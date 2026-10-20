# Opus のレビュー：Phase 8 段階 1b-3c（作業ツリー，HEAD `aeaf48f`）

レビューした範囲：`git diff HEAD`（12ファイル）のうち `crates/ezsdr-mock-radio/src/{lib,channel}.rs` の全差分，design 07・08・09・11・Vision の差分，セルフレビュー3回目のログ．mutation は流していない．テストは実装役のログ（11:52 の最終確認）だけを見た．

## 判定：major なし，minor 5件

最初に確かめた点はどれも問題ありませんでした．

- **M50 と同じ形の `radiated`**（`channel.rs:94`）：channel を持たない segment は `return None` せずに `continue` するようになった．今のモードでも結果は同じ．理由は次のとおり．
  - 同じ clock の世代の segment はすべて同じ channel の集合（`0..c`）を持つ．そのため一番新しい segment が `c` を持たなければ，その世代のどれも持たない．
  - 古い世代の segment は切り替えのところで `end` を切られている（`cut_segments`）ので，`j >= end` で `None` になる．
  - M50 は `unwrap_or(0)` の形に当て直されている．「channel を持たない burst が位置0の sample を出す」という同じ規則を壊す形のままなので，問題ない．
- **今のモードの3つのテストファイル**：差分は空（実装役のログとセルフレビュー3回目）．
- **今のモードの計算順**：受信は `else` の分岐にそのまま残り，乱数の引き順（`<device>/lo` の方向全体の順）も今のモード用の分岐に残っている．`path_delay` は `channel_model_terms(Tx, 0).1` に変わったが，値は `tx_path_delay_samples` と同じ．`phase.get(position)` は今のモードでは `position == channel` になる．
- **移動**：Provider が instance 全体の床で `e` を決め，`projected(e)` から `ColdChange::Move` を判定し，`coerce` を通してから `Lines` の複製に `book_move(Some(e))` を記帳する．成功したときだけ複製を採用する．`MoveError` のときは何も変えない．送信では両方の stream に `PendingUpdate.tx` を2件持たせ，適用時に両方で `stop_tx` と `tx_switch` を行う．
- **link のない受信 stream**：T0 で Stop した line を持ち，`rx_next_segment` が `None` を返すので，segment も block も作らない．panic の経路はない．

## O1（minor）MR-18 から，今のモードの「受信 line の origin を床にする」文が消えた

`design/09-mock-radio.md:153` の旧文には「never before the origin of a receive stream's line (RM-25)」があった．新しい文では `keys` の場合（受信 stream の start）にしか床が書かれていない．

一方，今のモードのコード（`lib.rs` の `timeline::command_instant`）は今も line の origin を床にしており，そのとき `LATE_COMMAND` は出さない（実装役が「今のモードは変えない」を選んだため）．

**直し方：** MR-18 に今のモード側の文を戻す．あわせて，今のモードでは T0 まで床付けしても `LATE_COMMAND` を出さないことを、`*Ceiling:*` か明記の文として書く．

## O2（minor）`start` が今のモードでも，link のない受信 stream の line を計算するようになった

`lib.rs`（`start` の受信の loop）で，`let line = self.line(Direction::Rx, k, target, &self.config)?;` が `links.is_empty()` の判定より前に移った．今のモードで受信の link がない instance（送信だけ）でも，これまで呼ばなかった `line()` を呼び，その `?` で `start` が失敗しうる．

**直し方：** `line()` の計算を `if self.profile.is_layout()` の中と，link がある経路だけに置く．今のモードの経路は HEAD と同じにする．

## O3（minor）`start` の巻き戻しのコメントが古い

`lib.rs`（`start` の `schedule_wakeup` 失敗時）のコメントに「this stage books no move, so the rebuild drops nothing else (RM-34)」とある．しかし移動はこの段階で入った．また `Lines::new()` で作り直すと，送信 line を戻しても `Lines` の move の記録は捨てられる．

`arm` と `start` の間に `UpdateParameter` が届かないのなら，実際には起きない．

**直し方：** 届かないことの根拠（KC-* の規則）をコメントに書く．届きうるのなら，`Lines` を丸ごと保存して戻す．

## O4（minor）`mr_31_a_layout_instance_joins_simulation_channel` の主張がテストより強い

spec 09 の MR-31 の Test 行と §6 の表は，このテストが「join と arm」を確かめると言っている．しかしテスト（`mock_layout.rs` の該当関数）が確かめているのは，空の coupling と fidelity だけで，join を省いても通る（セルフレビュー3回目の指摘が残っている）．

**直し方：** coupling を1つ持つ環境にして `Medium::missing()` が空であることを確かめるか，Test 行を実際に確かめていることに合わせる（DA-1）．

## O5（minor）`MoveError` の event の種類が固定されていない

`mr_40_a_move_error_changes_neither_line_configuration_nor_manifest` は，拒否の記録と状態が変わらないことを確かめるが，`radio.COMMAND_REJECTED` の event は確かめていない．`reject_action_at` を `record_rejected_action` に替えても通る（セルフレビュー3回目の指摘が残っている）．

**直し方：** event の種類を確かめる assertion と，それに対応する mutation の行を足す．

## 持ち越し（この段階では扱わない）

- `start` が失敗したときの segment の行の巻き戻し（`rows.truncate`）を確かめるテストがない．失敗を注入する手段がないためで，前の段階から持ち越している．
- MR-10 と MR-29 の専用の mutation の行がない．
- Vision §2 の「one key per direction」の古い記述．1b-5 で扱う．

---

## 裁定（orchestrator，2026-10-18）

- **O1〜O5：** 修正の回 1（Haiku 5.5 #high）で全部閉じた．Sol も「修正済み」と判定した．O5 は私の誤りで，assertion と P173 は実装役がすでに足していた．
- **Sol S1（今のモードの `radiated`）：** Sol 自身が取り下げた．上の M50 の判断と同じ．
- **Sol S2（major，coherence の過大な宣言）：** 採った．
  - 修正の回 2 は，記帳のときの membership から retune する channel を決めた．再レビュー 1 は，それが「開始前に切られた segment の retune を失う」と指摘した（major）．
  - 修正の回 4 で規則を単純にした．**`keys` の channel mode で始まった segment は，最初の segment を除き，その全 channel の untimed tune**（`random`・`random_unless_timed_tune` は引き直して coherent でなくなる）．根拠は次の2つ．
    - design/18 UR-25 と segment の開始：UHD Module は次の segment ごとに direction を untimed で設定し直す．
    - design 33 D13：untimed retune で coherent（timed）は失われる．
  - 記帳のときの記録は消した．
- **同じ instant の順：** 修正の回 3 で決めた．untimed tune は，その instant のほかの command の後に効く（G1）．
- **再レビュー 2：** major なし，minor 3 件．
  - minor 2（enable と move-out の経路を落とす mutation の行がない）：採った．P197・P198 を足した（既存のテストで kill する．orchestrator が追加）．
  - minor 1（同じ origin の RX と TX の引き順）：持ち越し．順は `tune_order` で決まっていて決定的（MR-30）．RX 先・TX 後を固定するテストは，mutation の行を整理する 1b-5 で足す．
  - minor 3（処理済みの origin を後から cut したときの Ceiling）：採らない．Ceiling は保証しないことを書いた文で，引き順の副作用（引いた位相が使われずに捨てられること）をテストで固定する理由がない．
