<!-- fix round 1: fix-report.md -->

STATUS: done

# 修正の報告：Phase 8 段階 1b-3c，orchestrator のレビュー O1〜O5

作業ツリー（HEAD `aeaf48f` からの未 commit の変更）に対して直しました．commit・push・`git add` はしていません．`cargo fmt`・`rustfmt` は使っていません．mutant は作業ツリーに当てていません（mutation は全部 `remote_mutate.py`）．3つのテストファイル（`mock_radio.rs`・`mock_channel.rs`・`timeline.rs`）の差分は空です．版は上げていません．

## 1. 指摘ごとの修正

### O1（MR-18）: 今のモードの床の文を戻した
- `design/09-mock-radio.md:153`（MR-18 の `cold` の段落）．HEAD の「never before the origin of a receive stream's line (RM-25)」を戻しました．
- 同じ段落で，「a past `at`, or an `e` so floored, emits `LATE_COMMAND`」の後の文を「including, under `keys`, when the receive-start floor sets it」にしました（今のモードでは T0 の床が LATE を出さないため）．
- `design/09-mock-radio.md:155`（同じ段落の `*Ceiling:*` の行の末尾）に，「*Ceiling:* without `keys`, a `cold` change floored at a receive stream's origin emits no `LATE_COMMAND`.」を足しました．
- コードは変えていません．`timeline::command_instant`（`crates/ezsdr-radio/src/timeline.rs:380`）は origin を `base` に入れるだけで，`LATE` の flag は `asked < now || e > base` なので，origin の床では立ちません．Ceiling はこれと一致します．

### O2（`start` の受信 loop）: `line()` の計算を2か所だけに戻した
- `crates/ezsdr-mock-radio/src/lib.rs:2529–2545`．`let line = self.line(...)` を，(a) link がない layout の分岐（2532）と (b) link がある経路（2539）の中に置きました．今のモードでは (a) に入らないので，HEAD と同じ順（`links.is_empty()` なら `continue`，そうでなければ `line()` → `first` → `insert` → `rx_begin`）に戻っています．
- mutation の行は足していません．差が出るのは `line()` が `Err` を返すとき（レートの変換か ns の溢れ）だけで，どのテストもその設定を作らないので，行が何も殺せないためです．

### O3（`start` の巻き戻しのコメント）: 届かないことを確かめ，規則番号をコメントに書いた
- `lib.rs:2555–2558` のコメントを書き直しました．コードは変えていません（`Lines` の丸ごと保存はしない）．
- 確かめた Kernel の規則と実装（結論：`arm` と `start` の間に `UpdateParameter` は届かない）：
  - KC-14（`design/06-kernel-coordinator.md:243`）：`arm` の後に Run は `Armed` になり，`start`（KC-18）が続く．
  - KC-17（`design/06…:250`）：timed の `UpdateParameter` は arm で admit されるが，dispatch は KC-18 のとおり start の後．
  - KC-18（`design/06…:252`）：`start` の後に Running へ移り，admit 済みの batch を dispatch する．
  - RS-18（`design/04-run-and-session.md:178`）：Running でない間に届いた Action は拒否．
  - 実装：`crates/ezsdr-kernel/src/coordinator/admission.rs:53`（Session と Module の origin は Running でないと refuse），`pipeline.rs:451`（`start_instances`）の後の `pipeline.rs:469`（batch の dispatch）．
  - MockRadio の `arm`（`lib.rs:2488–2505`）が入れるのは tx の line だけ（`insert`，`book` しない）．
- 注：指示書は「spec 03/04 の KC-*」と書いていましたが，KC-14・17・18 は spec 06 にあります．コメントは spec 06 と 04 の番号を書いています．

### O4（`mr_31_a_layout_instance_joins_simulation_channel`）: 送信の loopback で join を確かめる形にした
- `crates/ezsdr-mock-radio/tests/mock_layout.rs:808–823`．
  - coupling を1つ入れました（`radio` → `radio`，channel 0，`gain_db` 0）．fragment 名 `radio` は harness の fragment id と一致します．
  - rx と tx の channel 0 を stream 0 に置きました（制約を書かないと tx stream に clock が無く，`clocks_of` が空になるため）．
  - 100 サンプルの 0.5 の burst を tx sample 1000 で送り，10 ms まで step して，受信 block のうちゼロでない sample が 100 個であることを確かめます（`heard == 100`）．
  - fidelity の assertion は残しています．
- **指示書からの逸脱（理由）**：指示書の「`arm` が通る（`Medium::missing()` が空）ことを確かめる」では，join の省略を検出できません．`Medium::missing()` は `spec` が無いと空を返す（`crates/ezsdr-sim/src/channel.rs:296–299`）ので，join を省いても `arm` は通ります．これは一度目のセルフレビュー（`fix-self-review-1.log`）が major として指摘しました．そこで loopback の assertion にしました．
- mutation の行 P174 を足しました（`plan/maintenance/tools/mutations.json`，末尾）．join の呼び出し（`lib.rs:2433`）を `let _ = (&ctx.run, &f.id, &spec, seed, root_rate.num(), &tx);` に替える誤りです．name は「a channel-mode instance never joins its medium」（join は今のモードも含めた channel mode 全体の経路なので）．
- `design/09-mock-radio.md:197`（MR-31 の Test 行）を，テストの主張（prepare と arm が通り，fidelity が `impairment_model`，送った burst が自分の受信 stream に届く）に合わせました．`design/09-mock-radio.md:386`（§6 の行）は「a `keys` layout whose coupling names its own fragment arms, and a burst it transmits is received on its own channel (100 samples heard)」，規則は「MR-31, CH-6」です（CH-7 の拒否はテストが通らないので外しました）．

### O5（`mr_40_a_move_error…`）: `COMMAND_REJECTED` の event を確かめる
- assertion は作業ツリーに既にありました（`mock_layout.rs:1357–1358`，`COMMAND_REJECTED` の件数が 1）．mutation の行 P173（「a refused move does not emit COMMAND_REJECTED」，`mutations.json` の `P173`）も既にありました．私が着手する前にどちらも確かめ，内容が指示どおりであることを確かめた上で，二重には足していません．
  - 注：orchestrator のレビューは「assertion が無い」としていましたが，その時点のツリー（`mock_layout.rs` は 12:26 に更新済み）とは食い違っていました．
- `design/09-mock-radio.md:390`（§6 の行）の「whose new origin overflows」を「refused with a time error」に直しました．テストが確かめているのは `Time(...)` の refusal で，overflow ではないためです．

### P125（既存の行の当て直し）
- `mutations.json:5122–5127`．`old` が `let line = self.line(Direction::Rx, k, target, &self.config)?;` の1行だと，O2 の後に 12 桁の字下げのこの行が2か所に一致します（部分一致の検査のため）．そこで `old` を「その行 ＋ `let first = line.plan.first().copied();`」の2行にしました（一致は `lib.rs:2539–2540` の1か所）．`new` の規則（受信 stream が stream 0 の set と rate で始まる）は同じです．

## 2. 変えたファイル
- `crates/ezsdr-mock-radio/src/lib.rs`：O2（`start` の受信 loop，2529–2545）と O3（コメント，2555–2558）のみ．
- `crates/ezsdr-mock-radio/tests/mock_layout.rs`：O4 のテスト（808–823）のみ．
- `design/09-mock-radio.md`：O1（153，155）．O4（197，386）．O5（390）．
- `plan/maintenance/tools/mutations.json`：P125 の `old`／`new`（5122–5127），P174 を追加（末尾）．
- ほかは触っていません．`ezsdr_radio`・UHD・server・Python・fixture は変えていません．新しいファイルは作っていません（`git status` は変更前と同じ 12 件）．

## 3. 足した mutation の行と結果
- 足した行：**P174**（O4）．remote で killed．
- 当て直した行：**P125**（O2 の後に一意にするため）．remote で killed．
- 既存で，今回の確認の対象にした行：**P173**（O5，既存）．remote で killed．
- 「足した行と，コードかテストが変わった行」の remote 実行（`fix-remote-mutate.log`）：
  - 対象は 44 行：maintenance の P125・P128・P143 と P149〜P174（29 行），phase3 で変わった 15 行（M34・M36・M45・M46・M47・M48・M49・M50・M55・M57・M59・M62・M68・M69・M75）．
  - 結果：**44/44 killed**（maintenance 29/29，phase3 15/15）．ID は `fix-remote-ids.txt`．
  - P128・P143 と phase3 の行は，このステージの作業で既に変わっていたもので，私は書き換えていません．
- 生き残り：なし．

## 4. 確かめること 1〜6 の結果（最後の編集の後に，同じツリーで流した）
最後の編集は `design/09` と `mutations.json` が 12:55，`lib.rs` と `mock_layout.rs` が 12:46〜12:47 です．ゲートは 13:01 以降に流しました（`fix-gates.sh`）．

1. `cargo test --workspace --no-fail-fast`（`-j` なし）：**1248 passed，0 failed**（テスト結果 65 行）．ログ `fix-test-workspace.log`．
   - 参考：`mock_layout` 単体は `fix-mock-layout-3.log` で 34 passed（loopback 版）．
2. `cargo clippy --workspace --all-targets -- -D warnings`：**exit 0，warning・error 0**．ログ `fix-clippy-workspace.log`．
3. Docker の `--features uhd` clippy：**exit 0，warning・error 0**．ログ `fix-docker-clippy-uhd.log`．
4. `rules.py --base HEAD --renames test-renames.txt`：**0 problem**．ログ `fix-rules.log`．
5. remote の mutation：上の §3（44/44 killed，`fix-remote-mutate.log`）．全体の `all` は orchestrator の gate で流す分です（私は流していません）．
6. 3 つのテストファイルの `git diff HEAD`：**空**（0 バイト）．
- 追加の確認：`mutations.json` の全 564 行と phase3 の全 83 行で，`old` と `old2` がちょうど 1 回一致（`fix-old-unique.log`）．

## 5. セルフレビュー（AGENTS.md §9 の手順2）
指示書どおり，`opencode2 run --standalone -m "anthropic/claude-haiku-5-5#high" --agent plan --auto` を毎回新しく nohup で起動し，出力の伸びと `kill -0` で終わりを待ちました．

- **1回目**（`fix-self-review-1.log`）：
  - major：P174 が join の省略を殺さない（上の O4 の逸脱の理由）．→ loopback に直し，P174 を殺せることを remote で確かめた．
  - minor：`lib.rs` の「booked at `arm`」は実際は `insert`．→ 「inserted at `arm`」に直した．
  - 他：O1・O2・O3・O5 は指摘なし．P129 の削除（MR-41 の撤回に伴うもの）は，私が着手する前から作業ツリーにあったもの．
- **2回目**（`fix-self-review-2.log`）：
  - MR-18 の `cold` 段落の括弧「(without `keys`, …: *Ceiling*)」が Ceiling と重複し，文が切れていた．→ 括弧を削除．
  - §6 の mr_31 行の規則から CH-7 を外した（テストは CH-7 の拒否を通らない）．
  - §6 の mr_40 行の「overflows」を「refused with a time error」に直した．
  - P174 の name を広げた（「a channel-mode instance never joins its medium」）．
  - 残し：P149 以降の字下げ（3 スペース，HEAD の 2 スペースとは違う）．このステージで足した行の書式で，ファイル全体の書式替えは範囲外のため（下の未解決）．
- **3回目**（`fix-self-review-3.log`）：O1〜O5 と今のモードの挙動，mutation の一意性と P174 の `new` は指摘なし．残りは確かめの証拠（最後の編集の後のゲート：上の §4 で流した）と，範囲外の軽微な点だけ（下の未解決）．

3回やり，3回目で修正が必要な指摘は残りませんでした．

## 6. 未解決のこと（この回では範囲外として残したもの）
- `design/09-mock-radio.md` の MR-39 の「A move from it is still booked without a line lookup failure.」：何を規定するか曖昧（レビュー2・3）．O1〜O5 の範囲外のため直していません．
- `design/09-mock-radio.md` の MR-40 の文は「changes neither line, the configuration nor the planned segment order」と言うが，テストは configuration を直接は見ていない（`applied` が空であることで間接的に確かめる）．Test 行の名前もやや強く見える（レビュー3）．
- `design/09-mock-radio.md` の MR-27 の「design 33 D13」の参照は HEAD から同じ書き方が残っている．DA-1 からは決定記録の Changes 表へ移すのが望ましい（レビュー3）．
- `plan/maintenance/tools/mutations.json` の P149〜P174 は 3 スペースの字下げ（HEAD の行は 2 スペース）．書式の統一は別の作業．
- `design/vision/05-coherence-calibration-and-runtime-semantics.md` に，このステージで追加された規範に近い文が 2 つある（レビュー2・3）．AGENTS.md §2 は「Vision に規範を増やさない」なので，オーケストレーターの判断事項．
- phase7/G09 は既知の生き残り（`remote-mutate-all-final.log` の 12:02 の全体実行で SURVIVED のまま）．この回では `all` を流していないので，orchestrator の gate で確かめてください．
- 前のステージから残っていた作業ツリーの変更（私は触っていません）：`mutations.json` の P129 の削除（MR-41 の撤回），P128・P143 の `old`／`new` の変更，`mock_layout.rs` の O5 の assertion と P173 の追加．
- Python のテスト（ステージの指示書の確かめ 4）と，ステージの指示書にある `all` の remote 実行は，この修正の指示書の範囲外のため流していません．

---

<!-- fix round 2: fix2-report.md -->

STATUS: done

# 修正の回 2 の報告：Phase 8 段階 1b-3c，F1（stream に入った channel の untimed tune）・F2（MR-39）・F3（MR-40）

作業ツリー（HEAD `aeaf48f` からの未 commit の変更）に対して直しました．commit・push・`git add` はしていません．`cargo fmt`・`rustfmt` は使っていません．mutant は作業ツリーに当てていません（mutation はすべて `remote_mutate.py`）．3つのテストファイル（`mock_radio.rs`・`mock_channel.rs`・`timeline.rs`）の `git diff HEAD` は空です．版は上げていません．

確かめた結果，F1・F2・F3 は閉じました．ただし次の 2 点は範囲外として残しました（§6）．TX の move と rate change を直接確かめるテストがないこと，同じ instant での untimed tune と周波数更新の順序が規定されていないこと．

## 1. 指摘ごとの修正

### F1: stream に入った channel の untimed tune（`keys` のみ）

**採った形：** untimed tune は，stream の segment の origin で `advance` の中から `WorkKind::Tune` の候補として実行します．booking のときには実行しません．

- booking 時に実行すると，後から来た，より早い instant の tune が `channel::Timeline` の push 順を壊します（`channel::Timeline::at` は push 順の逆から探すため）．候補を `advance` の tick 順に並べると，push は常に instant の昇順になります．`timeline.rs` は変えていません．
- 候補は `self.profile.is_layout()` のときだけ作ります．今のモード（`keys` なし）では `tune` も `untimed_tune` も呼ばれず，出力は変わりません．`Tune` の腕は `continue` で抜けるので，`progressed` を立てません（`lib.rs` 2199–2212，2271–2275）．
- 同じ tick の順は，RxBegin（order 0）が先，その後に Tune．Tune は受信が先，stream 番号順，channel の並び順です（`tune_order`，`lib.rs` 2331 付近）．

**引く channel の規則：** segment の channel のうち，その stream が `cold` change の直前に持っていなかったものを引きます（`lib.rs` 1069–1107）．

- 「直前の segment が持っていたか」で判定する案は採りませんでした．stream を空にする cut の後，古い segment が plan に残るため，判定が狂います．
- 持っていた集合は `RxStream/TxStream.before: BTreeMap<u64, Channels>` に，`record_held`（`lib.rs` 1076）で booking のときに記録します．move は移動元（`from_held`）と移動先（`held`）の両方を記録します（`lib.rs` 1976–1977）．Streams の変更は 2004 で記録します．
- `start_rx` は記録しません（`lib.rs` 1594–1607）．記録がなければ既定の空集合になり，その stream の全 channel が引かれます．
- `enables` 集合は使わないため削除しました（既定の空集合で enable の retune は既に起きていたので，dead code でした）．

**`record_held` の保護（round 2 の指摘 1-a）：** `record_held` は `self.profile.is_layout()` のときだけ記録します（`lib.rs` 1077）．今のモードでは何も積みません．

**連結されていない RX stream（指摘 C）：** `tune()` は `links.is_empty()` の RX stream では早く戻ります（`lib.rs` 1093 付近）．その stream は `Stop` で始まり，segment を持たないため，untimed tune を取りません．これは MR-39 と MR-34 の文に書いてあります．

**規範の文（`design/09-mock-radio.md`）：**

- MR-34 の段落（204–206）：origin の untimed tune の規則，`random`・`random_unless_timed_tune` は新しい位相を引いて coherent でなくなる，`deterministic` は変わらない，引く順序（MR-30）．`phase_fixed` という実装名は規範の文から外しました（指摘 7）．
- `*Ceiling:*` 行（208）：「without `keys`, a stream's enable is not an untimed tune of its channels.」
- `*Test:*` 行（209）：新しい `mr_34_a_layout_*` テストを列挙し，行の区切りの `and` を補いました（指摘 6-b）．
- §6 表と改訂履歴の行を追加しました．

**テスト（`crates/ezsdr-mock-radio/tests/mock_layout.rs`）：** 次の `mr_34_a_layout_*` テストを追加しました（1712 行付近から，`mr_18_a_layout_receive_floor_at_t0_emits_late_command` の前まで）．

- `mr_34_a_layout_receive_entry_is_an_untimed_tune`，`mr_34_a_layout_transmit_entry_is_an_untimed_tune`
- `mr_34_a_layout_receive_move_is_an_untimed_tune_of_the_moved_channel`，`mr_34_a_layout_stream_enable_is_an_untimed_tune_of_its_channels`
- `mr_34_a_layout_deterministic_channel_keeps_its_phase_and_coherence_through_entry`，`mr_34_a_layout_deterministic_transmit_entry_keeps_phase_zero`
- `mr_34_a_layout_rate_change_keeps_a_channels_timed_tune`，`mr_34_a_layout_move_out_keeps_the_timed_tune_of_the_channels_left_behind`
- `mr_34_a_cbx_layout_entry_draws_a_fresh_phase`
- `mr_34_a_layout_unlinked_receive_stream_takes_no_untimed_tune`（今回の round 2 で書き直し，§2 参照）

各テストは `assert_near`（1e-5）で位相を確かめ，coherence の `basis` と `valid` も見ます．

**形の理由（指示書の確かめる 1〜3 について）：** timed tune で coherent（`timed_tune, true`），外して入れ直すと `none, false` で，位相は timed-tune 定数と違う（`mr_34_a_layout_receive_entry_is_an_untimed_tune`），もう一度 timed tune すると coherent に戻る．これらを満たします．deterministic は `deterministic, true` のままです．

### F2: MR-39 の文

`design/09-mock-radio.md:219` の「A move from it is still booked without a line lookup failure.」を，指示書の文に書き換えました：「A move into or out of it is booked on its line as on any other stream's (RM-34).」

Test 行（220）の `mr_40_a_move_to_and_from_a_linkless_receive_stream_never_publishes_there` は，今回次の assert を足しました（round 3 の指摘 3）：`segments` に stream 1 の行がないこと．`clocks_of("mock/rx/s1")` が空であることは既存の assert です．harness に `rx1` の port はないため，「block を出さない」は直接は見えません（SampleClock がなければ block は出ない，という前提で書いています）．

### F3: MR-40 の文と Test 行

テストは configuration を直接は確かめていなかったため，文の側を合わせました（指示書の 2 番目の選択肢）．

- 文（221）：「A move the lines cannot make is `radio.COMMAND_REJECTED`; it applies no update, and neither the Manifest's segments nor any clock changes.」
  - 「`MoveError`」は内部の型名なので，規範の文では使いません．
  - テストが確かめるのは，`COMMAND_REJECTED` が 1 件（`mock_layout.rs` 1357–1358），`applied` が空（1355），`segments` が不変（1356），clock の件数と終端（1359–1368）です．
- テスト名を `mr_40_a_move_error_changes_neither_line_configuration_nor_manifest` から `mr_40_a_refused_move_applies_no_update_and_changes_no_manifest_or_clock` に変えました（round 1 の nit F と，round 3 の指摘 4）．改名に伴い，`design/09` の Test 行（222）と §6 表（392），`mutations.json` の P150 と P173 の `filter` を直しました．

## 2. セルフレビュー（AGENTS.md §9 の手順 2）

`opencode2 run --standalone -m anthropic/claude-haiku-5-5#high --agent plan --auto`．各回とも読み取り専用で，新しい reviewer を起動しました．ログは `fix2-self-review-1.log`・`-2.log`・`-3.log`．

**round 1（6 件）：** A（move の移動元の保持を記録しない，major）は修正．B（今のモードでの progressed と候補）は修正．C（連結されていない RX の tune）は修正．D（足りない mutant，cbx テスト）は P181–P186 と cbx テストで対応．E（assert の不足）は rx/tx entry，enable，move のテストに足した．F（nit: テスト名）は今回の改名で閉じた．

**round 2（8 件）：**

- 2-a（major）：P186 が生き残った．原因は，linkless の RX stream が `start_rx` で有効にされても，元のテストでは untimed tune の経路に入らなかったこと．debug の複製（`/tmp/opencode/p186`，リポジトリの外）で確かめました．移動先の stream 1 は `Stop` で始まり，plan が空のままでした．そのため元のテストでは `tune()` の早期 return を通りませんでした．テストを書き直し，stream 1 の enable（`start_rx`）を stream 0 の stop・start の前に置きました．stream 0 の再開の retune が，retune stream の最初の draw になります．debug の複製で P186 を当てると，書き直したテストが失敗することを確かめました．リポジトリ本体では通ります．
- 1-a（minor）：`record_held` を layout のときだけ動くように囲みました．
- 1-b（minor）：同じ instant の untimed tune と周波数更新の順序は未規定．→ §6 へ．
- 5-a（minor）：MR-40 の文を，テストが確かめることに合わせました．
- 6-a（minor）：「has no segment」の文を「takes no untimed tune (MR-39)」に変えました（MR-39 は move の booking を言うため，矛盾していた）．
- 6-b（minor）：Test 行の `and` を補いました．
- 7-a（process）：最後の編集の後にゲートを流し直しました（§5）．

**round 3（7 件）：**

1. §6 表の `mr_34_a_layout_unlinked…` の説明が実テストと違った → 書き直しました（`design/09` 408）．
2. TX の move と rate change を確かめるテストがない → §6 へ（未解決）．
3. MR-39 の Test 行が「no segment」を確かめていない → 上の F2 のとおり，segments の assert を足しました．
4. MR-40 のテスト名 → 改名（F3 のとおり）．
5. `mutations.json` の P177 の `name` が，変異（timed 定数を使う）と違っていた → P181 と同じ言い方に直しました．
6. round 2 の変更範囲を示す材料がない（process）→ 本報告の §3 と §4 に，変えたファイル，変えた行，変えた行の mutant を書きました．
7. MR-34 の規範文に `phase_fixed` という実装名がある → 外しました．

観察（件数に入らない）：`advance` の中では，tick の順に push されます．同じ tick の Tune と Update は，同じ `advance` 呼び出しの中では Tune（rank 0）が先に push されます．ただし Update が前の呼び出しで既に処理されていて，Tune の origin がその tick と等しいときは，後から push された Tune が勝ちます．この順は仕様が決めていないため，§6 の 1-b と同じ問題です．

## 3. 変えたファイル（この回で変えたもの）

- `crates/ezsdr-mock-radio/src/lib.rs`：`tune_next`（1069）・`record_held`（1076，layout のみ）・`tune`（1089）・`untimed_tune`（1108）・`advance` の Tune 候補（2199–2212）・Tune の腕（2271–2275）・move と Streams の記録（1976–1977，2004）・`START_RX` は記録しない（1594–1607）．`channel.rs` と `profile.rs` は前の回からの変更で，この回では触っていません．
- `crates/ezsdr-mock-radio/tests/mock_layout.rs`：`mr_34_a_layout_*` テスト群（round 2 のテストの書き直しと改名を含む）．
- `design/09-mock-radio.md`：MR-34（204–209）・MR-39（219–220）・MR-40（221–222）・§6 表（392，408）・改訂履歴の行．
- `plan/maintenance/tools/mutations.json`：P175–P186 を追加，P150 と P173 の `filter` を改名，P177 の `name` を直した．

`git status` の変更は前の回からの 12 ファイルのままで，新しいファイルはありません．

## 4. 足した mutation の行と結果

| ID | 誤り | 殺すテスト | remote |
|---|---|---|---|
| P175 | RX の入った channel が `phase_fixed` を 0 にしない | `mr_34_a_layout_receive_entry_is_an_untimed_tune` | killed |
| P176 | TX の同じ誤り | `mr_34_a_layout_transmit_entry_is_an_untimed_tune` | killed |
| P177 | RX が新しい位相を引かず timed 定数を使う | 同上（`rx_entry ≠ rx_timed` の assert） | killed |
| P178 | RX の deterministic channel まで引く | `mr_34_a_layout_deterministic_channel_keeps_its_phase_and_coherence_through_entry` | killed |
| P179 | `start_rx` が channel を保持していたと見なし，retune しない | `mr_34_a_layout_stream_enable_is_an_untimed_tune_of_its_channels` | killed |
| P180 | rate change で，保持していた channel も retune する | `mr_34_a_layout_rate_change_keeps_a_channels_timed_tune` | killed |
| P181 | TX が新しい位相を引かず timed 定数を使う | `mr_34_a_layout_transmit_entry_is_an_untimed_tune` | killed |
| P182 | TX の deterministic まで引く | `mr_34_a_layout_deterministic_transmit_entry_keeps_phase_zero` | killed |
| P183 | CBX（`random`）の入れ直しが位相を引き直さない | `mr_34_a_cbx_layout_entry_draws_a_fresh_phase` | killed |
| P184 | 移動元の保持を記録しない（残る channel が retune される） | `mr_34_a_layout_move_out_keeps_the_timed_tune_of_the_channels_left_behind` | killed |
| P185 | 移動先の保持を記録しない（既存の channel が retune される） | `mr_34_a_layout_receive_move_is_an_untimed_tune_of_the_moved_channel` | killed |
| P186 | linkless の RX が enable で untimed tune を取る | `mr_34_a_layout_unlinked_receive_stream_takes_no_untimed_tune` | killed |

既存の行の変更：P150 と P173 は `filter` を改名，P177 は `name` を直し，どちらも remote で killed．既存の行 P151・P153・P158・P159・P160・P170 も同じ remote 実行に入れ，killed．

すべての行の `old` は作業ツリーで 1 回だけ一致します（576 行，`rules.py` と同じ検査）．

remote 実行の結果：`REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase3 sim02:8,sim03:8 P150 P173 P175 … P186 P151 P153 P158 P159 P160 P170` → **20/20 passed**（`fix2-remote-mutate-4.log`）．

## 5. 確かめること 1〜6 の結果（最後の編集の後に，同じツリーで流した）

1. `cargo test --workspace --no-fail-fast`：**1258 passed，0 failed**（`fix2-test-workspace.log`）．
2. `cargo clippy --workspace --all-targets -- -D warnings`：exit 0，警告 0（`fix2-clippy-workspace.log`）．
3. Docker の uhd clippy（`ezsdr-radio-uhd --features uhd --all-targets -- -D warnings`）：exit 0（`fix2-docker-clippy-uhd.log`）．
4. `rules.py --base HEAD --renames test-renames.txt`：**0 problem**（`fix2-rules.log`）．
5. remote mutation（追加・変更した行）：**20/20 killed**（§4）．全体の `all` の実行は orchestrator の作業です．
6. `mock_radio.rs`・`mock_channel.rs`・`timeline.rs` の `git diff HEAD`：**空**．

ゲートは `fix2-gates.sh` で流しました（`fix2-gates-status.txt`：exit-test 0，exit-clippy 0，exit-docker 0，exit-rules 0）．

## 6. 未解決のこと（この回では範囲外として残したもの）

1. **TX の move と rate change を確かめるテストがない（round 3 の指摘 2，minor）．** MR-34 の規則は両方向に当てはまりますが，テストは RX の move と RX の rate change だけです．P184・P185・P180 は RX の経路だけを殺します．ただし，move と rate change の経路の大部分は RX と TX で共通のコード（`record_held`・`tune`・`untimed_tune` の共通部分）で，TX の枝（`untimed_tune` の TX 側）は TX の entry テスト（P176・P181・P182）で確かめています．TX の move のテストには，2 channel の TX burst を作るヘルパーが必要ですが，今はありません．**推奨：** 次の回で，2 channel の TX burst のヘルパーと，TX の move・rate change のテストを足し，対応する mutant を足す．規則の文を RX に限るのは採りません（実装は両方向を扱うため）．
2. **同じ instant の untimed tune と周波数更新の順序が規定されていない（round 2 の 1-b，round 3 の観察）．** 今の実装では，同じ `advance` の中で Tune（rank 0）が Update（rank 1）より先に push されます．Update が前の呼び出しで処理済みの場合は，後から push された Tune が勝ちます．どちらの順が正しいかは仕様の判断です．**推奨：** MR-34 に一文を足し，「untimed tune は，その instant の command より前に効く」と決めてテストで固定する．これは owner（または orchestrator）の判断にしてください．
3. **push 順の前提がコメントにない（round 3 の観察 A）．** 「次の `now` が処理済みの tick より前にならない」という前提は，`advance` の中の注釈にありません．§6 の 2 と合わせて決めるのがよいと思います．
4. **round 3 の reviewer は，読み取り専用の作業で，抜粋を `/tmp/opencode/fix2-edits.txt`（リポジトリの外）に書いています．** リポジトリには影響ありません．
5. **phase3 の mutations.json は変えていません．** この回の変更は `plan/maintenance/tools/mutations.json`（maintenance）だけです．

---

<!-- fix round 3: fix3-report.md -->

STATUS: done

# Phase 8 段階 1b-3c 修正の回 3 の報告

作業ツリー（HEAD `aeaf48f` からの未 commit の変更）に対して直しました．commit・push・`git add` はしていません．`cargo fmt`・`rustfmt` は使っていません．mutant は作業ツリーに当てていません（mutation はすべて `remote_mutate.py`）．`mock_radio.rs`・`mock_channel.rs`・`timeline.rs` の `git diff HEAD` は空です（`fix3-tests-diff.log`，0 バイト）．版は上げていません．

採ったのは指示書 §6 の 1（G2），2 と 3（G1）です．ほかは変えていません．

## 指摘ごとの修正

### G1（§6 の 2・3）：同じ instant の untimed tune と command の順

- `crates/ezsdr-mock-radio/src/lib.rs` の `advance`（2199–2216）：`WorkKind::Tune` の候補を rank 1 から **rank 2** にしました．Update・HeldBurst・Fault（rank 1）の後に取られます．候補は `if self.started && self.profile.is_layout()` の内側です（`started` は RxBegin・Fault と揃えました）．
- 同じ箇所のコメント（2201–2204）に，push 順が instant 昇順になる前提（`channel::Timeline::at` が頼る）と，「この instant の Action は step がその instant を通る前に受け取られる（design 09 §4，MA-30）」という前提を書きました．
- `tune_order` のコメント（`lib.rs` 2333–2338）を，rank 2 として RxBegin・Fault・HeldBurst・Update の後に取る，と直しました．
- `design/09-mock-radio.md`：
  - MR-34（206）に1文を足しました．限定は `random_unless_timed_tune` だけです（`deterministic` は untimed tune で変わらないため）．
  - §4 の step の一覧（249）に「an untimed tune (MR-34) after the others at its instant」を足しました．
  - §6 に Test 行（401）を足し，改訂履歴に 2026-10-18 の行（450）を足しました．
- テスト `mr_34_a_layout_untimed_tune_follows_a_timed_tune_at_its_instant`（`crates/ezsdr-mock-radio/tests/mock_layout.rs` 1778）：`x3xx` の UBX loopback，seed 7，`random_unless_timed_tune` の channel．その segment の origin と同じ instant に，その channel の `hardware_timed` の周波数更新を置きます．走行は 2 段です（`run(None)` で origin を読み，`run(Some(origin))` で timed 更新を入れる）．次を assert します：coherence が `none, false`，位相が timed-tune 定数ではない，origin の更新が `applied` に入る．

### G2（§6 の 1）：送信側の move と rate change のテスト

- ヘルパー `two_channel_waveform`（`mock_layout.rs` 420）と `collect_blocks`（2333）を足しました．`collect_blocks` は受信 link（容量 64 の DropOldest）が溢れないよう，20 ms 刻みで step して drain します．
- `mr_34_a_layout_transmit_move_is_an_untimed_tune_of_the_moved_channel`（2270）：`x3xx`，stream 0 に送信 channel 2 本（`ubx` と `basic_tx`，both timed tuned）．channel 1 を空の stream 1 に移します．移った channel は `none, false` で新しい位相になり，移動元に残った channel 0 は timed 定数を保ちます．
- `mr_34_a_layout_transmit_rate_change_keeps_a_channels_timed_tune`（2343）：stream 0 の送信 rate を変えても，保持していた channel は timed tune を保ちます．
- 周波数の揃え：`ubx` 既定 1 GHz，`basic` 既定 0 に合わせ，TX の timed 更新は ubx 1.0e9・basic 10e6，`rx.c1.frequency_hz` は constraint の 10e6 にしました（不一致だと場の信号がゼロになったため）．
- `design/09-mock-radio.md`：MR-34 の Test 行（209）に，3 件の新テストを足しました．§6 に 3 行（401・404・408），改訂履歴に 1 行（450）を足しました．

## 足した mutation の行と結果

`plan/maintenance/tools/mutations.json` に 3 行を足しました（末尾，5732・5742・5752 付近）．全 579 行の `old` は，それぞれの `file` にちょうど 1 回一致します（python3 で数えました．書き換えていません）．

| ID | 誤った実装 | 殺すテスト | 結果 |
|---|---|---|---|
| P187 | Tune の rank を 0 に戻す（両 direction） | `mr_34_a_layout_untimed_tune_follows_a_timed_tune_at_its_instant` | killed（修正後）．修正前は SURVIVED（下記） |
| P188 | TX の move で，移動元・移動先の `record_held` を `if direction == Direction::Rx` で囲む | `mr_34_a_layout_transmit_move_is_an_untimed_tune_of_the_moved_channel` | killed |
| P189 | TX の rate change で `record_held` を `if direction == Direction::Rx` で囲む | `mr_34_a_layout_transmit_rate_change_keeps_a_channels_timed_tune` | killed |

P187 の最初の remote 実行は SURVIVED でした．原因は，origin の timed 更新が step(T0+100 ms) で受け取られて遅延扱いになり（applied が T0+102 ms），テストが順序を見ていなかったことです．テストに `harness.step(T0 + 20_000_000)` と，origin の更新が `applied` に `origin` の instant で入ることの assert を足して直しました．

remote 実行（最後の編集の後）：`remote_mutate.py maintenance,phase3 sim02:8,sim03:8 P170 P175 P176 … P189`．結果は `maintenance: 16/16 passed; not passed or missing: []`（16 行すべて killed）．ログ：`fix3-remote-mutate.log`．

F24 は `sort_by_key` の行で，コードもテストも変えていないため対象外です（grep で確認）．

## 確かめること 1〜6 の結果（最後の編集の後に流した）

1. `cargo test --workspace --no-fail-fast`：exit 0．合計 1261 pass，0 fail．`fix3-test-workspace.log`．（`mr_34_` の 16 件は，最後の編集の後の単体実行でも全部通りました．`fix-mr34.log`）
2. `cargo clippy --workspace --all-targets -- -D warnings`：exit 0，warning 0．`fix3-clippy-workspace.log`．
3. Docker の `--features uhd` clippy：exit 0．`fix3-docker-clippy-uhd.log`．
4. `rules.py --base HEAD --renames test-renames.txt`：`0 problem(s) against HEAD`．`fix3-rules.log`．
5. remote で全部 killed（上記，16/16）．
6. `git diff HEAD -- mock_radio.rs mock_channel.rs timeline.rs`：空（0 バイト）．`fix3-tests-diff.log`．

まとめは `fix3-gates.log`（exit-test 0，exit-clippy 0，exit-docker 0，exit-rules 0，exit-diff-bytes 0）．

## セルフレビュー（`anthropic/claude-haiku-5-5#high`，`opencode2 run --standalone --agent plan --auto`，毎回新しい reviewer，読み取り専用，3 回）

- **1 回目（`fix3-self-review-1.log`），3 件．**
  1. §6 の TX move の行が，テストの実際（2 channel，stream 1 は空）と合わない → 行を書き直した（直し済み）．
  2. MR-34 の追加文の "the channel" が `deterministic` にも当たる → 「a `random_unless_timed_tune` channel」に限定（直し済み，改訂履歴も）．
  3. 確かめることの結果が，最後の編集より前 → すべて流し直した（直し済み）．
- **2 回目（`fix3-self-review-2.log`），1 件（minor）．** advance のコメントの前提が不完全（後から届く timed 更新の経路が書かれていない）→ 前提を明記する形で直した．ただし，同じ `now` で後から Action が届く経路（ideal の lead 0 と `handle_update` の `effective = now` を含む）は，Kernel 側で確かめ切れていません．**未解決**（下記）．
- **3 回目（`fix3-self-review-3.log`），3 件．**
  1. 確かめることの結果が古い → 流し直した（直し済み）．
  2. Tune の候補が `if self.started` の外にある → `self.started &&` を足した（直し済み）．到達するかは読んだだけでは確かめ切れず，RxBegin・Fault と揃える防御として入れました．
  3. design/09 §4 の step の一覧に untimed tune がない → 足した（直し済み）．

3 回の上限に達したため，3 回目の指摘への修正（2 と 3）は**再レビューしていません**．

## 未解決のこと

1. **宛先側の保持記録（P188 の「to」側）のテストが入っていません．** TX の move の宛先 stream 1 は空の stream です．`x3xx` の TX は 2 channel（ubx A・basic_tx B）しか作れず，宛先に channel を持たせられません．P188 は移動元の側で殺せますが，宛先側だけを外す誤りは殺せません．2 TX channel の制約の下で入れる方法は，この回では見つけていません．
2. **advance のコメントの前提が，Kernel 側で未確認です（指示書どおり）．** 「この instant の Action は step がその instant を通る前に受け取られる」が成り立つかは，`crates/ezsdr-kernel` と design 09 §4 で確かめていません．成り立たない場合（同じ `now` の後着 timed 更新）は，rank 2 の untimed tune が先に効き，timed 定数が勝ちます．その経路のテストはありません．
3. **3 回目の指摘への修正（`started` 条件，design §4 の一覧）は再レビューしていません．**
4. 作業ツリーには，この回より前からの未 commit の変更があります（`src/channel.rs`，`src/profile.rs`，`design/07`，`design/08`，`design/11`，`Ez-SDR_v4_ARCHITECTURE_VISION.md`，`plan/phase3/tools/mutations.json`）．この回の編集記録には含まれず，触っていません．commit の前に，どの回の変更かを確かめてください．
5. 100 桁を超える行が残っています（`lib.rs` 2208，`mock_layout.rs` 2301 付近）．tree は rustfmt をかけない方針なので，そのままです．

## 変えたファイル

- `crates/ezsdr-mock-radio/src/lib.rs`（advance の Tune rank 2・started 条件・コメント，tune_order のコメント）
- `crates/ezsdr-mock-radio/tests/mock_layout.rs`（テスト 3 件，ヘルパー 2 件）
- `design/09-mock-radio.md`（MR-34 文・Test 行，§4 の一覧，§6 の 3 行，改訂履歴 1 行）
- `plan/maintenance/tools/mutations.json`（P187・P188・P189）

---

<!-- fix round 4: fix4-report.md -->

STATUS: done

# Phase 8 stage 1b-3c fix round 4 — report

brief: `fix4-brief.md`（前提: `fix-brief.md`, `fix2-report.md`, `fix3-report.md`, `sol-rereview.md`）

## 1. 実施内容

MR-34 の規則を変更しました。`keys` のもとで、stream が始める segment（arm / start が立てる最初の segment を除く）の origin では、その segment の全 channel に untimed tune をかけます。`random` / `random_unless_timed_tune` は phase を引き直し、`deterministic` は変えません。TX の clock が自分の origin で cut された場合（cut == 0）は tune を取りません。

- `crates/ezsdr-mock-radio/src/lib.rs`
  - `record_held`, `before`, `from_held`, `held` の記録を削除しました。
  - `tune` を書き直しました。segment の全 channel を `untimed_tune` に渡します（`by == None` と、TX で `cut == Some(0)` の segment は除外）。RX で link のない stream は従来どおり early return です。
  - `advance` のコメントを書き直しました。timed update の lead の前提（`earliest = now + min_timed_command_lead_ns`、`handle_update`）、MA-30 が instant 内の順序を保証しないこと、`cold` / `stop` の ceiling を書いています。
- `crates/ezsdr-mock-radio/tests/mock_layout.rs`
  - 三つの test を改名し、新しい規則に合わせました: `mr_34_a_layout_rate_change_is_an_untimed_tune`、`mr_34_a_layout_transmit_rate_change_is_an_untimed_tune`、`mr_34_a_layout_move_out_is_an_untimed_tune_of_the_channels_left_behind`。
  - 追加: `mr_34_a_layout_reentry_cut_before_its_start_leaves_the_rate_change_untimed`（再入の segment が始まる前に cut され、rate change の origin が untimed tune を取る）、`mr_34_a_layout_transmit_clock_cut_at_its_origin_takes_no_untimed_tune`（TX の cut at origin は tune を取らず、次の clock の origin が最初の 2 draw を取る）。
  - 指示書の「残す」一覧を超えて変えたもの（要報告）:
    - `mr_34_a_layout_receive_move_is_an_untimed_tune_of_the_moved_channel`: stream 1 の channel 1 も retune されるようになり、draw #2 を assert します。
    - `mr_34_a_layout_transmit_move_is_an_untimed_tune_of_the_moved_channel`: stream 0 の channel 0 も新しい origin で retune され、`none,false` を assert します。
    - どちらも新しい規則から直接導かれる変更です。
- `design/09-mock-radio.md`
  - MR-34 の規則文、Ceiling、Test 行、§6 の行（404–412 付近）、改訂履歴（454 行目）を更新しました。
  - 本ラウンド末尾の修正（self-review round 3 の指摘への対応）: 204 行の「The initial configuration is an untimed tune」を「draws at `prepare`」に直しました（`lib.rs` の `tune` の doc と矛盾していたため）。207 行の Ceiling の書式を `*Ceiling:*` に揃えました。
- `plan/maintenance/tools/mutations.json`
  - 削除: P179, P180, P184, P185, P188, P189（6 行）。
  - 追加: P190, P191, P193, P194, P195, P196（6 行）。P192 は使っていません（欠番）。
  - 合計 579 行。全 `old` は一致が一つだけです。

触っていないもの: `mock_radio.rs`, `mock_channel.rs`, `timeline.rs`（diff 0 バイト）。`channel.rs` と `profile.rs` は本ラウンド以前（13:55 と 09:34）から変更済みで、本ラウンドでは触っていません。`cargo fmt` / `rustfmt` は実行していません。`mutate.py` はローカルで実行していません。commit / push / `git add` はしていません。

## 2. 検証

| 項目 | 結果 | ログ |
|---|---|---|
| `cargo test --workspace` | 1263 passed, 0 failed（exit 0） | `fix4-test-workspace.log`（`fix4-gates-status.txt`） |
| `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 | `fix4-clippy-workspace.log` |
| docker `ezsdr-radio-uhd --features uhd` clippy | exit 0 | `fix4-docker-clippy-uhd.log` |
| `rules.py --base HEAD` | 0 problem(s)（設計文の修正後にも再実行して 0） | `fix4-rules.log` |
| 3 つの test ファイルの diff | 0 バイト | `fix4-tests-diff.log` |
| `ezsdr-mock-radio` の test（最終コードで再実行） | 2 + 31 + 49 + 60 + 1 passed, 0 failed | `fix4-crate-test-final.log` |
| `ezsdr-mock-radio` の clippy（最終コードで再実行） | exit 0 | `fix4-clippy-crate-final.log` |
| remote mutation（sim02/sim03、本ラウンドの 23 行） | 23/23 killed | `fix4-remote-mutate.log` |

workspace の gate は、最後の `lib.rs` の編集（16:41）と同時刻に始まっていたため、その直後に crate の test と clippy を最終コードで再実行しました。設計文の修正（204・207 行）は cargo の対象外で、`rules.py` で再確認しています。

remote mutation の 23 行: P190, P191, P193, P194, P195, P196（新規）、P170, P175–P178, P181–P183, P186, P187, P150, P151, P153, P158–P160, P173（既存で filter が本ラウンドの test に当たるもの）。`all` の全 gate は本ラウンドでは実行していません（orchestrator の作業）。

## 3. Self-review（`opencode2 run --standalone -m anthropic/claude-haiku-5-5#high --agent plan --auto`、上限 3 ラウンド）

- Round 1（`fix4-self-review-1.log`）: 指摘に対応しました。
- Round 2（`fix4-self-review-2.log`）: major なし、minor 3 件、nit 1 件。
  - 「P190–P196 の kill の根拠」は、コードを読んで判断したものです（remote 実行の前）。
- Round 3（`fix4-self-review-3.log`）: 新しい major はありません。
  - minor 1（TX rate change のテストに mutation 行がない）: P195 を追加して対応しました。
  - minor 2（segment の初期 `by.is_none()` の例外に mutation 行がない）: P196 を追加して対応しました。
  - minor 3（`advance` のコメント）: 書き直して対応しました（MA-30 の文、lead の前提、`cold` / `stop` の ceiling）。
  - minor 4（「untimed tune」の二つの意味）: round 3 の後に、設計文 204 行を修正して対応しました。round 3 の review には戻していません（上限 3 ラウンドのため）。
  - nit（Ceiling の書式）: 修正しました。nit（「so a timed tune … not coherent」の結果説明）: 削除しました。nit（§6 の「first clock draws none」が直接 assert されていない）: この文は現在の §6 の行にはありません。nit（P192 の欠番）: 意図的な欠番として残しました。

Round 3 の tail に「major 1: 完了条件の未実施（最後の編集の後に gate を流していない）」がありました。最後の編集の後に gate と remote mutation を流して閉じています（上の表）。

## 4. 未解決の項目（open）

1. **RX と TX の同一 origin の tie-break（MR-34 の規則文）**: 「then receive before transmit」に対応する test も mutant もありません。x3xx の origin が一致する scenario は見つかっていません（未確認）。決定が必要です: scenario を作るか、この節を削るか。
2. **mutation 行がない test**:
   - `mr_34_a_layout_stream_enable_is_an_untimed_tune_of_its_channels`（P179 の削除で enable の行がなくなった）。
   - `mr_34_a_layout_move_out_is_an_untimed_tune_of_the_channels_left_behind`。
   - `mr_34_a_layout_rate_change_is_an_untimed_tune`（RX 側）は P194 で殺されると推論していますが、実行していません（INFERRED）。
3. **Ceiling（test なし）**: `cold` の変更や `Provider::stop` が、loop がすでに通過した origin で clock を cut すると、draw がその clock に対して無駄になります。`advance` は `stop` で `advance(now)` を呼ぶことを読んで確かめました（INFERRED）。Ceiling として書いたので、test にはしていません。
4. **指示書の範囲外の変更**: `mr_34_a_layout_receive_move_…` と `mr_34_a_layout_transmit_move_…` の二つを変えました（§1 に記載）。
5. **Sol の minor 2（TX move の移動先の record）**: record を削除したので対象がありません（前提が消えています）。
6. **P192** は欠番のままです（意図的）。
7. **`all` の remote mutation gate** は未実行です（orchestrator の作業）。
8. **`fix4-self-review-3.log`**: 引用する前にログを確認しました。reviewer の出力の中に、この確認で流したコマンドの出力が混ざっていることがあります（上の tail の一部）。

---

