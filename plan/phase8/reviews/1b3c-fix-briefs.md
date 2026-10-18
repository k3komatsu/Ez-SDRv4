<!-- fix round 1: fix-brief.md -->

# 修正の指示書：Phase 8 段階 1b-3c 修正の回 1

あなたは実装役（修正の回）です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `e4ca550`，作業ツリーに段階 1b-3c の未 commit の変更がある）．

最初に次を読んでください．
- `AGENTS.md` の全部（特に §2 の DA-1，§7，§9）
- この段階の指示書 `~/.cache/ezsdr-fixes/2026-10-18-phase8-1b3c/brief.md`
- orchestrator のレビュー `~/.cache/ezsdr-fixes/2026-10-18-phase8-1b3c/opus-review.md`

レビューの **O1〜O5 を全部直してください**．「持ち越し」の節は扱いません．

## 守ること（違反は作業のやり直しになる）

- **`cargo fmt`・`rustfmt` を一切使わない**．新しいコードは周りの書き方に手で合わせる（長い行もそのまま）．
- **mutation は `plan/maintenance/tools/remote/remote_mutate.py` だけ**（sim02/sim03）．手元の `mutate.py` は使わない．mutant を作業ツリーに当てない．
- **commit・push・`git add` をしない**．
- `crates/ezsdr-mock-radio/tests/mock_radio.rs`・`mock_channel.rs`・`timeline.rs` は1行も変えない．今のモード（`keys` なし）の挙動は変えない．
- 直す対象以外のファイルや行は変えない．版を上げない．Python は `PYTHONDONTWRITEBYTECODE=1`．
- 2分を超えるコマンドは背景で流す：`nohup <cmd> > ~/.cache/ezsdr-fixes/2026-10-18-phase8-1b3c/fix-<名前>.log 2>&1 &`．終わりは `kill -0 <pid>` で待つ（`pgrep -f`・`pkill -f` は使わない）．
- `cargo test` は `-j` なしで流し，重い処理を同時に走らせない．
- mutation の行の `old` は，編集の後も実際のコードとちょうど1回一致すること．行の位置が動いた既存の行も確かめる．
- `design/` は DA-1 の形を守る：規範の文だけを書き，経緯・段階・issue 番号は書かない．`*Test:*` 行は，テストが実際に確かめていることだけを言う．

## 各指摘の要点

- **O1**：MR-18（`design/09-mock-radio.md`）に，今のモードの床の文を戻す．今のモードでは `cold` 変更が受信 line の origin より前に効かない（`timeline::command_instant`）．あわせて，T0 まで床付けしたとき今のモードは `LATE_COMMAND` を出さないことを `*Ceiling:*` として書く．コードは変えない．
- **O2**：`start` の受信の loop で，`self.line(...)` の計算を次の2か所だけに置く．今のモードの経路は HEAD（`git show HEAD:crates/ezsdr-mock-radio/src/lib.rs`）と同じ順に戻す．
  - link がない layout の stream
  - link がある stream
- **O3**：`start` が失敗したときの巻き戻しのコメントを直す．まず `arm` と `start` の間に `UpdateParameter` が届くかを Kernel の規則（spec 03/04 の KC-*，`crates/ezsdr-kernel`）で確かめる．
  - 届かない場合：その根拠の規則番号をコメントに書く．
  - 届く場合：`Lines` を丸ごと保存して戻す実装にし，テストを足す．
- **O4**：`mr_31_a_layout_instance_joins_simulation_channel` を変える．この fragment を tx と rx に持つ coupling を1つ入れ，`prepare` で join し，`arm` が通る（`Medium::missing()` が空）ことを確かめる．
  - 必要なら mutation の行（join を省く誤り）を足す．
  - spec 09 の MR-31 の Test 行と §6 の表の行は，テストが確かめていることに合わせる．
- **O5**：`mr_40_a_move_error_changes_neither_line_configuration_nor_manifest` に，`radio.COMMAND_REJECTED` の event が出ることの assertion を足す．あわせて mutation の行を足す（`MoveError` の分岐の `reject_action_at` を，event を出さない `record_rejected_action` に替える誤り）．
  - id は `plan/maintenance/tools/mutations.json` の今の最大の P 番号の次から．
  - `test_bin` と `filter` は，その行を kill するテスト．

## 確かめること（最後の編集の後に，同じツリーで全部流す）

1. `cargo test --workspace --no-fail-fast`（背景で）
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. Docker の `--features uhd` clippy：`docker run --rm --user "$(id -u):$(id -g)" -v "$PWD:/work" -v ezsdr-v4-cargo-target:/cargo-target -w /work ezsdr-v4-dev:uhd4.10 cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings`
4. `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD --renames ~/.cache/ezsdr-fixes/2026-10-18-phase8-1b3c/test-renames.txt`（0 problem）
5. 足した行と，コードかテストが変わった行（O2・O4・O5 の周りの行を含む）を remote で流す（背景で）：`REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase3 sim02:8,sim03:8 <IDs>`．全部 killed にすること．全体（`all`）は orchestrator が gate で流す．
6. `git diff HEAD -- crates/ezsdr-mock-radio/tests/mock_radio.rs crates/ezsdr-mock-radio/tests/mock_channel.rs crates/ezsdr-mock-radio/tests/timeline.rs` が空

## セルフレビュー（AGENTS.md §9 の手順2）

報告の前に，読み取り専用の新しいレビュー役を自分のシェルから起動します．指摘を直し，指摘が無くなるか3回やるまで繰り返します．

起動のしかた：
- nohup で流し，`kill -0 <pid>` で終わりを待つ．
- 起動したら最初の出力が出ることを確かめる．
- 30分ログが伸びなければ止まったとみなし，起動し直す．

コマンド：`opencode2 run --standalone -m "anthropic/claude-haiku-5-5#high" --agent plan --auto "<チェックリストと，この指示書のパス>"`

チェックリスト：
- O1〜O5 がそれぞれ本当に直っているか．抜けと余分はないか．
- 今のモードの挙動が変わっていないか（O2 の経路が HEAD と同じか）．
- 足した assertion と mutation の行は，それぞれどの誤った実装を捕まえるか．`old` はちょうど1回一致するか．
- `design/` の文は DA-1 の形か．Test 行の主張はテストと一致するか．

## 報告

`~/.cache/ezsdr-fixes/2026-10-18-phase8-1b3c/fix-report.md` に書く．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．中身：
- 指摘ごとに何をどう直したか（ファイルと行）
- O3 で確かめた Kernel の規則と，その結論
- 足した mutation の行と，流した行の結果
- 確かめること1〜6の結果（数とログのパス）
- セルフレビューの各回
- 未解決のこと

---

<!-- fix round 2: fix2-brief.md -->

# 修正の指示書：Phase 8 段階 1b-3c 修正の回 2

あなたは実装役（修正の回）です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `e4ca550`，作業ツリーに段階 1b-3c と修正の回 1 の未 commit の変更がある）．

最初に次を読んでください．
- `AGENTS.md` の全部
- `~/.cache/ezsdr-fixes/2026-10-18-phase8-1b3c/fix-brief.md`：守ること・確かめること・セルフレビュー・報告の形は，この回も同じ
- 修正の回 1 の報告 `fix-report.md`
- Sol のレビュー `sol-review.md`

採る指摘は下の F1〜F3 です．それ以外は変えません．

## F1（major，Sol の S2）：stream に入った channel は untimed tune として扱う

**問題：** `keys` の channel mode で，`random_unless_timed_tune` の channel を timed tune したあと stream から外し（`stream = -1`），もう一度入れると，次のことが起きる（`lib.rs` の `phase_fixed`，`layout_phase_basis`，`apply_layout_channel_update`）．
- 新しい segment が，すぐ `timed_tune, valid: true` を名乗る．
- 位相は timed tune の定数のまま．

実機では，stream を有効にするとき UHD Module が周波数を含む設定を untimed で設定し直す（`design/18-uhd-radio.md:185` の UR-25）．design 33 D13（`plan/phase8/33-channels-and-streams.md:250`）では，coherent（timed）は次の untimed retune で失われる．つまり Mock が実機より強い保証を名乗っている（Vision §59）．

**決めた規則（spec 09 MR-34 に書く）：** `keys` の channel mode では，あるストリームの segment の origin を，そのストリームの直前の segment が持っていなかった channel にとっての **untimed tune** とする．stream への出入りと，別の stream からの移動の両方が当てはまる．その origin から次のようになる．
- `random_unless_timed_tune` と `random` の channel：`<device>/lo/retune` から新しい位相を1つ引き，coherent でなくなる（`phase_fixed` を 0 に）．次に timed tune したら，また timed-tune 定数で coherent になる．
- `deterministic` の channel：変わらない（位相 0，coherent）．
- 引く順序：決定的にし（MR-30），spec に書く．たとえば「segment の origin の順，同じ origin なら受信が先，stream 番号の順，segment の中は channel の並び順」．
  - 受信の segment は step の中で始まり，送信の clock は記帳のときに登録される．このため，引く時点と timeline に `set` する instant が食い違わないようにする．
  - `channel::Timeline::set` が instant の順を前提にしているなら，それを壊さない形を選ぶ．
  - 選んだ形と理由を報告に書く．
- Manifest の `segments` の行の `coherence` は，この規則と一致すること．
- **今のモード（`keys` なし）は変えない．** そのかわり MR-34 に1行足す：「*Ceiling:* without `keys`, a stream's enable is not an untimed tune of its channels.」

**テスト（`mock_layout.rs`）と mutation の行を足す：**
- UBX の受信と送信のそれぞれで次を確かめる．
  1. timed tune で coherent（`timed_tune, true`）になる．
  2. stream から外し，もう一度入れる．新しい segment は `none, false` になり，受信した位相が timed-tune 定数と違う．
  3. もう一度 timed tune すると coherent に戻る．
- 別の stream への移動でも同じく `none, false` になる．
- `deterministic`（`ideal`）の channel は，出し入れしても `deterministic, true` のまま．
- mutation の行を足す（id は今の最大の P 番号の次から）．
  - 入った channel の `phase_fixed` を落とさない誤り
  - 新しい位相を引かない誤り
  - deterministic の channel まで引いてしまう誤り

## F2（minor，修正の回 1 の報告 §6）：MR-39 のあいまいな文

`design/09-mock-radio.md` の MR-39 の「A move from it is still booked without a line lookup failure.」を，何を決めているかが分かる規範の文に書き直す．例：「A move into or out of it is booked on its line as on any other stream's (RM-34).」

そのとおりのことをテストが確かめているかを確認し，Test 行と合わせる．

## F3（minor，修正の回 1 の報告 §6）：MR-40 の Test 行の主張

MR-40 は「changes neither line, the configuration nor the planned segment order」と言う．しかし `mr_40_a_move_error_changes_neither_line_configuration_nor_manifest` は configuration を直接確かめていない．

どちらかにする．
- 拒否の後の configuration（そのキーの値）が変わっていないことを直接 assert する．
- 文と Test 行を，テストが確かめていることに合わせる（DA-1）．

## 確かめること

`fix-brief.md` の 1〜6 と同じものを，最後の編集の後に流す．5 の remote 実行に入れる行は次のとおり．
- 足した行
- コードかテストが変わった行（P158〜P160・P170 など，位相と coherence の行を含む）

報告は `~/.cache/ezsdr-fixes/2026-10-18-phase8-1b3c/fix2-report.md` に書く．セルフレビューは `anthropic/claude-haiku-5-5#high` で，最大3回．

---

<!-- fix round 3: fix3-brief.md -->

# 修正の指示書：Phase 8 段階 1b-3c 修正の回 3

あなたは実装役（修正の回）です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `e4ca550`，作業ツリーに 1b-3c と修正の回 1・2 の未 commit の変更がある）．

最初に読むもの：
- `AGENTS.md`
- `~/.cache/ezsdr-fixes/2026-10-18-phase8-1b3c/fix-brief.md`（守ること・確かめること・セルフレビュー・報告の形は，この回も同じ）
- 修正の回 2 の報告 `fix2-report.md`

採るのは，その §6 の 1・2・3 です．ほかは変えません．

## G1（§6 の 2 と 3）：同じ instant での untimed tune と command の順

**決めた規則：** 1つの instant では，segment の origin の untimed tune を，その instant のほかの command・fault・held burst の**後**に効かせる．

**理由：**
- UHD Module は，segment を始めるときに direction を設定し直す（`design/18-uhd-radio.md:200`）．その設定には，その origin で効いている設定（UC-2 の投影．保留中の timed command を含む）を **untimed** で入れる．
- だから，その instant の timed の周波数更新も untimed の設定にまとめられ，coherent にはならない．
- Mock がこれより強い保証を名乗らないようにする（Vision §59）．

**変えること：**
- `lib.rs` の `advance` で，`WorkKind::Tune` の候補の rank を，Update・HeldBurst・Fault（rank 1）より後にする（たとえば rank 2）．
  - 前の `advance` の呼び出しで処理済みの Update より後に Tune が push される場合も，結果は同じ（Tune が勝つ）になる．
  - `channel::Timeline` の push が instant の昇順になる前提も，`advance` の該当箇所にコメントで書く．
- `tune_order` のコメントを直す．
- spec 09 の MR-34 に1文足す：「At one instant, a segment origin's untimed tune takes effect after that instant's commands, so a timed tune at that instant leaves the channel not coherent.」（言い回しは DA-1 の形で整えてよい）
- テストを1つ足す．`random_unless_timed_tune` の channel について，stream への入り直しの origin と同じ instant に，その channel の `hardware_timed` の周波数更新を置く．そして次を確かめる．
  - その segment の coherence が `none, false` であること
  - 位相が timed-tune 定数ではないこと
- mutation の行を足す（Tune の rank を Update より前に戻す誤り）．

## G2（§6 の 1）：送信側の move と rate change のテスト

- 2 channel の送信 burst を作るヘルパーを `mock_layout.rs` に足す．既存の `mr_32_a_logical_transmit_channel_follows_its_move_to_another_stream` が多 channel の送信 block を使っているので，その書き方を流用してよい．
- 次の2つのテストと，それぞれに対応する mutation の行を足す．
  - **送信の move**：timed tune した送信 channel を別の stream に移す．移った channel は `none, false` になり，新しい位相になる．移動元に残った channel は timed tune を保つ．
    - mutation の行：送信側で移動元・移動先の保持を記録しない誤り．
  - **送信の rate change**：送信 stream の rate を変えても，その stream が持っていた channel は timed tune を保つ．
    - mutation の行：rate change で保持していた送信 channel も retune する誤り．
- 規則の文は RX に限らない（両方向に当てはまる）．MR-34 の Test 行と §6 の表に，新しいテストを足す．

## 確かめること

`fix-brief.md` の 1〜6 を，最後の編集の後に流す．5 の remote 実行に入れる行は次のとおり．
- 足した行
- コードかテストが変わった行（P175〜P186，P170 など）

報告は `~/.cache/ezsdr-fixes/2026-10-18-phase8-1b3c/fix3-report.md` に書く．セルフレビューは `anthropic/claude-haiku-5-5#high` で，最大3回．

---

<!-- fix round 4: fix4-brief.md -->

# 修正の指示書：Phase 8 段階 1b-3c 修正の回 4

あなたは実装役（修正の回）です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `e4ca550`，作業ツリーに 1b-3c と修正の回 1〜3 の未 commit の変更がある）．

最初に読むもの：
- `AGENTS.md`
- `~/.cache/ezsdr-fixes/2026-10-18-phase8-1b3c/fix-brief.md`（守ること・確かめること・セルフレビュー・報告の形は，この回も同じ）
- `fix2-report.md`・`fix3-report.md`
- Sol の再レビュー `sol-rereview.md`

## 決めたこと（orchestrator）：規則を単純にする

Sol の再レビューの major 1 の中身：
- 修正の回 2 の「stream が `cold` 変更の直前に持っていなかった channel だけを retune する」は，記帳のときの membership（`record_held`・`before`）から決めている．
- 記帳された segment が開始前に切られて plan から消えると，その segment がするはずだった retune が失われる．
- `Stop`→`start_rx`→開始前の `cold` 変更でも同じことが起きる．

記帳のときの membership と，実際に走った segment が食い違う，という仕組みそのものが原因です．局所的に直さず，規則を単純にします．

**新しい規則（MR-34）：** `keys` の channel mode では，次のように扱う．
- **ストリームの segment が始まるたびに，その origin は segment のすべての channel の untimed tune になる．** ただし最初の segment（`arm`・`start` の初期設定．位相は `prepare` で引く）は除く．
- `random_unless_timed_tune` と `random` の channel は，新しい位相を引き，coherent でなくなる．
- `deterministic` の channel は変わらない．
- 同じ instant では，ほかの command の後に効く．これは修正の回 3 のとおりで，変えない．

**理由：**
- UHD Module は次の segment ごとに「その origin で効いている設定で direction を設定し直し」，それは untimed に入る（`design/18-uhd-radio.md:185` と `:200`）．
- untimed の周波数設定は UBX の LO の位相を変えるので，coherent（timed）はそこで失われる（design 33 D13）．
- rate の変更や，channel が出て行った側の stream も新しい segment になるので，同じく retune する．Mock を実機より強くしない（Vision §59）．
- 開始前に切られて plan から消えた segment は始まらないので，retune もしない．実際に始まった segment だけが retune する．

## 変えること

- `lib.rs`：
  - `record_held`，`RxStream`/`TxStream` の `before`，`from_held`・`held` の記録を消す．
  - `tune` は，始まる segment（`by` が `Some`）のすべての channel を `untimed_tune` に渡す．
  - 送信は，clock の origin に，その clock が実際に走る（origin より後に cut がある，または cut がない）ときだけ retune する．origin で終わる clock では何もしない．
  - link のない受信 stream（segment を作らない）は retune しない．これは今のまま．
  - `advance` のコメントにある Kernel の前提（同じ instant に後から届く Action）を直す．MA-30 はそれを保証しない．非 deterministic な profile では，timed の更新は `min_timed_command_lead_ns`（x3xx は 2 ms）より後に来るので，すでに処理した instant には届かない．lead が 0 の `ideal` は deterministic で，untimed tune の影響を受けない．この根拠をコメントに書く．
- テスト：
  - 変える：規則が変わったテスト．`mr_34_a_layout_rate_change_keeps_a_channels_timed_tune`，`mr_34_a_layout_transmit_rate_change_keeps_a_channels_timed_tune`，`mr_34_a_layout_move_out_keeps_the_timed_tune_of_the_channels_left_behind` は，「rate の変更や出て行った側の新しい segment でも retune され，`none, false` になる」に変える．テスト名も中身に合わせる．
  - 消す：意味がなくなるテストと mutation の行（P179・P180・P184・P185・P188・P189 など，`before` の記録を壊す行）．mutation の行は消してよい．spec 09 の §6 の表と Test 行も合わせる．
  - 足す（Sol の major 1 の筋書き）：timed tune → stream から外す → 同じ instant に「再加入」と「rate の変更」を記帳する．再加入の segment は plan から消える．残った segment が retune され，`none, false` になり，位相が timed-tune 定数でないことを確かめる．
  - 足す（Sol の minor 3）：同じ origin で複数の channel が retune されるとき，`<device>/lo/retune` からの引き順（spec に書いた順）をテストで固定する．各 channel が受け取る位相を，同じ seed で引いた値と比べる．引き順を逆にする mutation の行を足す．
  - 残す：入る・移動・enable・CBX・deterministic・同じ instant の順（P187）のテスト．
- spec 09：
  - MR-34 の規則の文を，上の新しい規則に書き換える．
  - `*Ceiling:*` 行を「without `keys`, a segment's origin is not an untimed tune of its channels.」にする．
  - Test 行・§4・§6・改訂履歴を，実際のテストに合わせる．
- Sol の minor 2（送信の move の宛先の記録）は，記録をなくすので対象がなくなる．報告にそう書く．

## 確かめること

`fix-brief.md` の 1〜6 を，最後の編集の後に流す．5 の remote 実行に入れる行は次のとおり．
- 足した行
- コードかテストが変わった行（P170・P175〜P178・P181〜P183・P186・P187 など．消した行は除く）

報告は `~/.cache/ezsdr-fixes/2026-10-18-phase8-1b3c/fix4-report.md` に書く．セルフレビューは `anthropic/claude-haiku-5-5#high` で，最大3回．

---

