# 1b-4a の修正の指示書

## 修正の回 1


あなたは実装役（修正の回）です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `9816d78`，作業ツリーに 1b-4a の未 commit の変更がある）．

最初に読むもの：
- `AGENTS.md`
- `~/.cache/ezsdr-fixes/2026-10-18-phase8-1b4a/brief.md`．「守ること」「確かめること」「セルフレビュー」「報告」の形は，この回も同じ．
- 自分の報告 `report.md`
- Opus のレビュー `opus-review.md`
- Sol のレビュー `sol-review.md`

## 裁定（orchestrator）：直すもの

すべて採用する．番号は下の「直すこと」の番号．

| 番号 | 元の指摘 | 重さ |
|---|---|---|
| 1 | Sol 1 | major |
| 2 | Opus O1＝Sol 2 | major |
| 3 | Sol 3 | major |
| 4 | Sol 4 | minor |
| 5 | Sol 5 | minor |
| 6 | Sol 6＋Opus O2 | minor |
| 7 | Opus O3 | minor |

## 直すこと

### 1. `uhd_subdev_spec_make` の失敗で handle が漏れる（`uhd.rs` の `set_subdev_spec`）

UHD 4.10 の `subdev_spec_c.cpp:31-36` は，次の順に動く．
1. handle を `new` する．
2. その後に markup を解析する．

そのため，不正な markup では handle が確保されたまま error が返る．

直し方：make の成否にかかわらず，handle が非 null なら `uhd_subdev_spec_free` で解放する．テストは feature `uhd` の中で書けるなら書く．書けなければ，Test 行に「none named」と残す．

### 2. 空の channel 列（fake）

- fake の `rx_open` と `tx_open` は，検査の前に，空の列を `[0]` に読み替える．これは UHD の `sanitize_stream_args` と，今の `UhdDevice` と同じ扱い．
- そのうえで，次の検査を channel 0 として行う．
  - 二重の open
  - 範囲
- 理由：provider は 0 channel の送信を 1 channel として扱う（`provider/tx.rs:207,412,414,485` の `channels.max(1)`）．HEAD の fake も 1 本の vector を返し，buffer 1 つの send を受け取っていた．
- テスト：
  - `rx_open(0, &[])` の後の `rx_recv` が vector を 1 本返す．
  - `tx_open(0, &[])` の後の buffer 1 つの `tx_send` が受け取られる．
  - `rx_open(0, &[0])` の後の `rx_open(1, &[])` が拒否される．送信も同じ．
- mutation の行：空の列を空のまま開く形．`rx` と `tx` で 1 行ずつ．
- UR-33 の該当の文と Changes 行は，「空の列は，UHD と同じく channel 0」という 1 つの規則に直す．

### 3. subdev spec を変えたとき，予約済みの timed command の宛先が変わる（fake）

今の作りでは，次の問題がある．
- `commands` は channel の番号を持ったままで，spec と settings だけが並べ替わったり縮んだりする．
- そのため，縮めると `settle()` が範囲外を参照して panic する．
- 並べ替えると，別の front end に効いてしまう．

UHD の timed command は radio block（front end）の command FIFO に入るので，宛先は front end．直し方：
- 予約の宛先を `(slot, front end)` で持つ．
- 効く時刻に，その front end が今の spec のどの channel かを引いて適用する．
- その front end が spec から外れていたら，何もしない．
- spec に戻ったとき既定値から始まる，という今の規則（UR-33）は変えない．

テスト：timed change を channel 1 に予約してから spec を `"A:0"` に縮める．時刻が過ぎても panic せず，channel 0 は変わらない．並べ替え（`"B:0 A:0"`）では，予約した front end に効く．mutation の行を 1 つ足す（channel の番号で引く形）．

### 4. markup の区切りと，失敗の呼び出し名（fake の `parse_spec`）

UHD の `subdev_spec.cpp` は，次のように動く．
- `boost::char_separator<char>(" ")` で，ASCII の空白だけで区切る．空の token は飛ばす．
- token を `:` で分ける．3 つ以上に分かれると `value_error("invalid subdev-spec markup string: " + markup)` になる．
- したがって `"A:0\tB:0"` は，`"A"`・`"0\tB"`・`"0"` の 3 つに分かれて失敗する．

直し方：
- `split_whitespace()` をやめ，`' '` だけで区切る．空の token は飛ばす．
- 解析の失敗（`invalid subdev-spec markup string`）の呼び出し名は `uhd_subdev_spec_make` にする．
- 表にない組の失敗は，今のまま `uhd_usrp_set_*_subdev_spec`．
- テストに `"A:0\tB:0"` の拒否を足す．

### 5. テストの力（`tests/fake.rs` の `ur_33_two_receive_streamers_keep_their_own_channels` ほか）

- stream 1 を止めた後のテストを強める．manual clock で，止めた時刻より十分先まで次を確かめる．
  - stream 0 が，止めた時刻より後の sample を届け続ける．
  - stream 1 は，止めた後に生成された sample を届けない（tail の後は Timeout）．
- P216（`rx_stop` が stream 0 を止める）が，負荷に関係なく kill されることを確かめる．
- spec の変更で streamer が切れることの送信側を足す．
  - テスト：`set_subdev_spec(Tx, …)` で送信の streamer が切れ，受信は残る．
  - mutation の行を 1 つ足す．

### 6. spec 18 の文（DA-1）

- UR-33 の今回の文から，次を取り除く．Changes 行と決定記録（plan 33）に移す．
  - 理由
  - 「this stage」などの段階の話
  - 「(the M-rules below need only the first)」のような意味の取れない句
- 規範の文は「fake が何をするか」と，短い出典（周りの UR-33 と同じ程度）だけにする．
- UR-15・UR-17・UR-25 の呼び出しの形を，新しい署名にする（`rx_start(0, T0)`，`rx_recv(0, …)`，`rx_packet_samples(0)` など．今の `UhdRadio` は stream 0 を使う）．他にも古い署名が残っていないか，`design/` を grep する．
- Changes 行の「`UhdDevice` … holds one streamer per direction」を「stream の番号ごとに 1 つ」に直す．

### 7. 報告に足すこと

「指示書から外れたところ」に，fake の次の変更を書く（コードは今のままでよい）．
- `rx_packet_samples` は，streamer がないとき 0 を返す（`UhdDevice` と同じ）．
- 予定された `TxReport` の fault は，送信の streamer が開いているときだけ出る．

## 確かめること

指示書 `brief.md` の 1〜6 を，最後の編集の後に，同じツリーで流す．6 の remote 実行の流し方：
- まず，次の行だけを流す．
  - 足した行
  - コードかテストが変わった行（P202・P208〜P210・P216・P226 と，`parse_spec`・`settle`・`rx_open`・`tx_open` に当たる行など）
- 最後に `all` を流す．

報告は `~/.cache/ezsdr-fixes/2026-10-18-phase8-1b4a/fix-report.md` に書く．セルフレビューは `anthropic/claude-haiku-5-5#high` で，最大 3 回．チェックリストは `brief.md` のものに，この指示書の 1〜7 を足す．

## 修正の回 2


あなたは実装役（修正の回）です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `9816d78`，作業ツリーに 1b-4a と修正の回 1 の未 commit の変更がある）．

最初に読むもの：
- `AGENTS.md`
- `~/.cache/ezsdr-fixes/2026-10-18-phase8-1b4a/brief.md`．「守ること」「確かめること」「セルフレビュー」「報告」の形は，この回も同じ．
- 修正の回 1 の指示書 `fix-brief.md` と報告 `fix-report.md`
- Sol の再レビュー `sol-rereview.md`

## 裁定（orchestrator）：Sol の再レビューの 3 件をすべて採用する

### 1.（major）期限を過ぎた予約が，spec に戻った front end に効く（fake）

**起きること：**
1. front end B に，時刻 T の timed change を予約する．
2. spec を `"A:0"` に縮める．
3. T を過ぎるまで待つ．
4. `"A:0 B:0"` に戻す．
5. その後の呼び出しの `settle` が，B が不在だった T の予約を，戻った B に効かせてしまう．

**原因：** `set_subdev_spec` が，期限の来た queue を処理しない．`settle` は，処理するときの spec を見る．

**直し方：** `set_subdev_spec` が成功して spec を置き換える直前に，古い spec のまま `settle` する．こうすれば，期限の来た予約は，その時点で効くか捨てられるかが決まる．

**テスト：** 次の 2 つの場合を確かめる．
- T より後に B を戻す：予約は捨てられ，B は既定値から始まる．
- T より前に B を戻す：予約は戻った B に T で効く．

**mutation の行：** `set_subdev_spec` の中の `settle` を外す形を，1 行足す．

### 2.（minor）handle の解放を外してもテストが通る

**直し方：**
- `src/uhd.rs` の `ffi_tests`（feature `uhd` の lib の unit test．すでに `#[unsafe(no_mangle)]` で UHD の関数を置き換えている）と同じやり方で，`uhd_subdev_spec_make` と `uhd_subdev_spec_free` を置き換える．
- 置き換えた関数で解放の回数を数え，次の 3 つの経路でそれぞれ 1 回だけ解放されることを確かめる．
  - 成功
  - 解析の失敗（make が handle を確保してから error を返す）
  - set の失敗
- 置き換えは，その test binary の中だけに効くようにする．他の ffi テストや `tests/uhd_api.rs` を壊さないこと．
- 置き換えられない事情があれば，報告にその理由を書く．
- `tests/uhd_api.rs` のテストと UR-3 の `*Test:*` 行の説明は，実際に見ているものに合わせる．
  - UHD の `uhd_subdev_spec_free` は，解放の後に null を入れる．
  - したがって「二度目の free は必ず abort する」という説明は誤り．
- 公開した `subdev_spec_lifecycle` が不要になれば消す．

### 3.（minor）fake の新しい境界の規則に，テストと mutation の行がない

いずれも，streamer のない stream を呼んだときの規則．
- `rx_packet_samples` は 0 を返す．`rx_packet` を 0 でない値に設定した fake で確かめる．
- `rx_recv` は，host clock で最大 10 ms 待ってから `Failed` を返す．
- `tx_async` は，timeout まで待ってから `Ok(None)` を返す．

テスト：manual clock の participant の上で，待った時間を直接確かめる（既存の `ur_33_a_stopped_stream_s_recv_waits_its_whole_timeout` の形）．
mutation の行：規則ごとに 1 行（0 を返さない形，待たずに返す形）．

## 確かめること

`brief.md` の 1〜6 を，最後の編集の後に，同じツリーで流す．
- 3 の docker では，feature `uhd` の `--lib` も流す．
- 6 は，次の行だけを流す．
  - 足した行
  - コードかテストが変わった行
- 最後に `all` を流す．

報告は `~/.cache/ezsdr-fixes/2026-10-18-phase8-1b4a/fix2-report.md` に書く．

セルフレビュー：
- 役：`anthropic/claude-haiku-5-5#high`
- 回数：最大 3 回
- チェックリスト：`brief.md` のものに，この指示書の 1〜3 を足す．
