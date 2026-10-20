# 1b-4b orchestrator review（AGENTS.md §9 の 6：指示書への忠実さ・変えてはならない振る舞い・設計の意図）

対象: HEAD `fdfdf3b` からの未コミットの diff（13 ファイル，+1626/−284），`report.md`．

## 変えてはならない振る舞い — production の各書き換え

- `core.rs`
  - `Streams::book`・`refuse` は `Line::book`・`refuse` から `Lines::book`・`refuse` に移った．
    - `Lines` は複製した上で `settle` を通す（`timeline.rs:466-488`）．move と floor が無ければ `settle_once` はすべての item の release を `NO_RELEASE` と比べる（`timeline.rs:654-656, 688-713`）．
    - UHD の記帳はすべて `release: NO_RELEASE`（grep `release:` provider/*.rs）なので，何も変わらない．
    - 失敗したときに line を元のままにする点も同じ（`Line::book` は pop，`Lines` は複製を捨てる）．
  - `end`
    - 記帳は rx → tx の順で，各方向 `k = 0` の 1 本ずつ．
    - rx の line が無い（link が無い）ときに `book` が false を返す点も同じ．
  - `prune`
    - `prune_stream` は 1 本の `Line::prune` だけ．
    - 旧コードも自分の方向の line だけを畳んでいたので，同じ．
  - `reconcile`
    - 旧コードは常に tx の line と `tx_clocks` を使っていた．新しいコードは `changed` から `stream.0 == Tx` のときだけ呼ばれ，同じ `(Tx, 0)` を見る．
    - `core.register(dir_of(stream), …)` は `Dir::Tx` と同じ．
  - `record`：各方向 1 本ずつなので，行の数と順序は同じ．
- `mod.rs`
  - `prepare`・`arm`・`start` の各経路は旧コードと一対一に対応する．`channels == 0` のときは handle を作らずに飛ばし，link の無い rx は open しない．
  - `rx_first` は旧コードの `first` と同じで，`Tx::new` の `channels` は旧 `tx_channels` と同じ．
  - `stop` の join の順（control → tx → rx）も変わらない．
  - `Core` は `prepare` ごとに新しく作られる（`mod.rs:371`）．したがって `Lines::insert` が古い rx line を残す心配は無い（旧コードは `lines = [rx, Some(tx)]` で上書きしていた）．
- `control.rs`・`rx.rs`・`tx.rs`
  - `Dir::*` から `booked_stream(dir)`／`stream_key(dir, self.k)` への置き換え，device 呼び出しの `0` から `self.k`／`k` への置き換えは，いずれも機械的な同値変換．
  - lock の範囲も変わらない．from-none の `drop(streams)` から再 lock までの位置は同じ．
  - `rx_packet_samples(0)` は裁定どおり残り，理由のコメントが付いている．

**結論：** 振る舞いの変化は見つからなかった．pre-existing の全テストが通ることは gate で確かめる．

## 指示書への忠実さ・設計の意図

- 裁定の「構造」（`StreamState` 1 型，instance に残す `seq`・`ended`・`stop`・`version`，`Lines` の key，per-line prune）はすべて反映されている．
- `start` の列の長さの guard：今は到達できない防御（`prepare` が常に長さ 1 を作る）．それでも，1b-4c で列が 2 本になったときに黙って先頭だけを使う事故を防ぐ．テスト（P280）が付いているので採る．
- spec 18
  - UR-26 の join の順の記述は，旧文「joins uhd-tx, uhd-rx and uhd-control」がコード（control が先）と違っていた．今回コードに合わせて直っている．
  - Changes 行が挙げる規則は UR-3・UR-26・U2・§3．UR-16 と UR-25 は Test 行だけの変更なので，挙げなくてよい．
- 1b-4c へ送る一覧（report §7）は `plan-final.md` (c) と整合する．

## 所見

- **minor**（後回しでよい）：`ur_25_a_refusal_leaves_the_transmit_stream_alone` は，空の tx line（item も clock も無い）を手で入れて snapshot を比べる．そのため「変わらない」の大半は自明に成り立ち，実質の検査は P248（拒否を tx line に載せる）を殺せるかどうかだけ．tx 側に item を 1 つ記帳してから比べれば強くなる．
  - 1b-4c で 2 本目の stream を入れるときに同じ形のテストを作り直すので，ここでは直さない．

## 判定

指示書・裁定に忠実で，変えてはならない振る舞いは保たれている．私の側からの必須修正は無い．Sol のレビューを待って裁定する．

## 裁定（Opus と Sol のコードレビュー）

| 出所 | 重さ | 指摘 | 裁定 |
|---|---|---|---|
| Opus | minor | `ur_25_a_refusal_leaves_the_transmit_stream_alone` が空の tx line と比べる | 1b-4c へ送る：2 本目の stream を入れるとき，同じ形のテストを item のある line で作り直す |
| Sol | minor 1 | `ur_25_a_cold_change_and_a_timed_update_at_one_instant_keep_their_own_instants` の `cold_first = false` が，予約の順を変えていない（`bookings` の push の位置だけが変わる） | 採用．orchestrator が直した（テストだけの約 10 行．実装役の回を 1 つ足すより安い）：逆順では timed の受信の update を cold より先に予約し，held の `(e, seq)` を両順で期待値と比べる．両順で通る．行 P266・P267 は gate の `all` で走る |
| Sol | minor 2 | `record()` は同じ方向の全 stream の segment を平らに並べ，stream の番号を持たない．この持ち越し（final (c) の 11）が report §7 から抜けている | 採用．1b-4c へ送る一覧に足した（設計 33 §5 の 1b-4 行）．今回の Manifest は変えない |

Sol の判定は PASS with minors（blocker 0，major 0）．major が無いので，再レビューは要らない．

## 指摘の数え方（出所・重さ・機構）

| 段階 | 出所 | 件数 | 機構（`plan/maintenance/failure-mechanisms.md`） |
|---|---|---|---|
| テスト計画（コードの前） | Sol | major 4・minor 2，全件採用 | 正しい挙動で満たせない期待（1），名指す誤りを壊さない mutation の組（1），stream ごとの prune のテストの欠け（1），`apply` を見ない spy（1），minor（gate の待ち方，clock の前提）．いずれもテストの不足か検出力の不足で，設計の機構には数えない |
| セルフレビュー | Haiku 5.5 #high × 3 回 | P1 0．P2 1（`start` が列の先頭だけを使い，残りを黙って捨てる）→ 直して，テストと行（P280）を足した．P3 9 → 7 件を直し，2 件は理由を付けて退けた | P2 は新しい経路の潜在的な失敗で，実装の時点で塞いだ．設計の機構には数えない |
| コードレビュー | Opus | minor 1 | テストの検出力 |
| コードレビュー | Sol | minor 2 | テストの検出力（1），持ち越しの書き漏れ（1） |

- コードの後に出た major は 0．修正の回は 0（Sol の minor 1 は orchestrator が直した）．再レビューも 0．
- 1b-4a は major 4（Opus 1・Sol 3），修正の回 2，再レビュー 2 だった．
- ただし 1b-4b は「挙動を変えない」組み直しで，新しい振る舞いを足した 1b-4a より性質が軽い．テスト計画を先に書く手順が割に合うかは，振る舞いを足す 1b-4c と 1b-4d と合わせて判断する（AGENTS.md §9）．
- 計画の段階で Sol が見つけた 4 件は，コードの前に直っている．以前の流れなら，これらはコードのレビューで major として出て，修正の回になっていたはず．

## gate（orchestrator，2026-10-19）

- 通ったもの：
  - `cargo test --workspace`：1299 passed
  - clippy：手元と image 内の `--features uhd`
  - image 内の `--features uhd`：`--lib` 124 passed，`--test uhd_api` 5 passed
  - Python：3.14 と 3.9 でそれぞれ 47 OK
  - `rules.py`：0 problem
- mutation の全リスト（`all`）：1037/1039 で，残ったのは既知の phase7/G09 と phase7/U41．
- U41 の原因：
  - `new` が消えた field `streams.lines[dir as usize]` を使っていて，COMPILE ERROR になった．
  - `old` は一致したままだったので，実装役の「全行の `old` がちょうど 1 回一致する」の確認では見えなかった．
- U41 の処置：`new` を `streams.line(stream)` に当て直し，単独で再実行して killed．これで 1038/1039（G09 のみ）．
- 次の段階の指示書に足すこと：`old` の一致だけでなく，名前を変えた識別子が各行の `new` に残っていないかも grep する．
