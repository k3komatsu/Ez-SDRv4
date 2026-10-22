# 1b-6a：実装役（DeepSeek V4.1 Flash free）の報告

STATUS: done

# R1（UHD Module の stream ごとの holder と読み）テスト計画の報告

2026-10-22．指示書 `impl-brief.md` の段階 1b-6a，設計メモ `plan/phase8/35-uhd-stream-state.md` §2 R1 の
risky step 2（テスト計画）だけを行った．コード・テスト・mutation 行は書いていない．実行もしていない．

## 1. 読んだもの

- `impl-brief.md` 全部，設計メモ 35 全部（R1 の表，§4 の系列 S1–S8，R2 との境界）．
- HEAD `8cd89e4` の production：`provider/core.rs`（`Streams`/`StreamState`，`owner_configure`，
  `release_unneeded`，`move_source_busy`，claim の unit test），`provider/control.rs`（`book_cold`，
  `await_reading`，`rx_record`，所有のテスト），`provider/tx.rs`（`pass`，`read`，`end_reading`，
  `do_switch`，`read_reports_before_close`，`shutdown`），`provider/rx.rs`（`begin`，拒否と loss のテスト），
  `provider/test_support.rs`（`core.probe`，`LockSpy`，`record_locks`/`taken_locks`，manual clock）．
- `tests/fake.rs` の move 拒否の統合テスト（4880–4903）と，`ur_25_the_first_segment_after_arm...`（4906）．
- spec 18 UR-25（`design/18-uhd-radio.md:261–271`，`*Test:*` 279，Changes 645）．
- `plan/maintenance/tools/mutations.json` の該当行．

確認に使った grep（結果は計画に載せた）：

- `grep -rnE "\.(busy|busy_seq|reading)\b" crates/ezsdr-radio-uhd` → 71 行（production と test build の両方）．
  test build の直接読み書きは 23 か所．`tests/fake.rs:3677` の `plan.busy()` は別の型（watchdog の plan）なので除外．
  `host.rs` の `busy` も別の型なので除外．
- `mutations.json` で `old`/`new` に `.busy`・`.busy_seq`・`.reading` を持つ行 → 17 行．
  うち R1 で当て直すのは 14 行（P252，P253，P382–P386，P389，P392，P398，P399，P404，P455，P496）．
  J279・J295・J312 は `host.rs` の manual clock の `busy` で無関係．
  P505・P512 は文字列が変わらない（`move_source_busy` の呼び出しのままで，中身だけが `Owner { seq }` を読む）．

## 2. 計画の判断（`test-plan.md` §1，§7）

- R1 の範囲は `Claims { holder: Holder, reading: bool }`（field は private）と
  `Holder { Free, Owner { seq: Option<u64> }, Release, FromNone }`，`Claim`（`#[must_use]`，stream と役割），
  遷移の関数 8 つ（`claim_owner`，`claim_release`，`claim_from_none`，`may_call_from_none`，
  `begin_read`，`begin_shutdown_read`，`end_read`，`give_back`）だけ．`idle`・`active`・`delivered`（R2），
  device の記録，loss，#67 は変えない．`move_source_busy` は `Owner { seq }` を読む．
- `owner_configure` は失敗で `Err((error, claim))` を返し，呼び出し側が同じ line-lock の節で拒否を book して
  `give_back` する（`Rx::begin`，`Tx::do_switch`）．
- 観測のために read-only の accessor を 2 つ足す案を最小の追加として推奨：`Streams::holder(stream) -> Holder`
  と `Streams::reading(stream) -> bool`．`holder` は production の guard に必要．field は private のまま．
- `Claim` の Drop は test build でだけ panic．`give_back` は `mem::forget` か `returned` の印で正常終了時の
  Drop を止め，mismatch の panic の前にも `mem::forget` して二重 panic を避ける（計画 §1）．
- `end_read`/`give_back` の「前が違うとき」の production の扱いは設計メモが決めていない．推奨は状態を変えない
  （他の holder を壊さない）．test build の panic だけを N6/N8 で固定する（未決 A）．
- S2 の「book の前に他の thread が `Free` を見ない」は，line-lock の取得回数が 1 節であることで固定する（N10）．
  同じ節の中の文の順序は観測できないが，同じ節なら他の thread は入れないので規則は満たす（未決 E）．
- P385（booking の前に claim を返す）は `Option<Claim>::take()` で表現できるので，撤回せず当て直す．
  既存の `cold_booking` probe テストが殺す（未決 C）．前段の「consumes で表現できない」は `take` を見落としていた．

## 3. 新しいテスト（`test-plan.md` §3）

- thread を使わない `Streams` の unit test 10 個（`core.rs` の `mod tests`）：
  N1 `claim_owner`，N2 `claim_release`，N3 `claim_from_none` と `may_call_from_none`，
  N4 `begin_read`，N5 `begin_shutdown_read`，N6 `end_read` の mismatch（should_panic），
  N7 Drop の panic（返し忘れ，should_panic），N8 `give_back` の mismatch（should_panic），
  N9 `move_source_busy` が `Owner { seq }` だけを見る，N10 receive の拒否が 1 つの line-lock 節で
  refuse + give_back になること（`record_locks`/`taken_locks`）．
- thread と probe（S6，`spy_rig()` と manual clock，uhd-control は raw `std::thread::spawn`）：
  T1（control が先に返る順）と T2（shutdown の読みが先に終わる順）．どちらも `holder`/`reading` accessor と
  `spy.seen` で観測し，join で終わりを見る．
- 各テストに「何を assert し，どう観測し，どんな誤実装で落ちるか」を書いた．落とす誤実装は
  遷移の表の「前が違うとき」と production の各分岐（guard が `if false`，`give_back` の削除，
  `reading` を消す `give_back`，`begin_shutdown_read` が holder を見る，など）に対応させた．

## 4. 既存テストの扱い（`test-plan.md` §2，§4）

- S1–S8 のそれぞれに，HEAD で見ているテスト名と assert の内容を対応づけた（brief §4 の候補すべてを含む）．
- 直接読み書きしている 23 か所を置き換え表にした（`rx_record`/`tx_record` の 2 helper は
  `holder(k) != Free` と `reading(k)` に変え，返す tuple の形は変えない）．S4a/S5 の既存テストには
  `holder`/`reading` の assert を追加する（§4.1）．
- `ur_25_a_detached_control_does_not_hold_the_shutdown_s_read` は `busy = true` の代わりに
  `claim_from_none` + `mem::forget`（detach の模型）にする．`mem::forget` は test の中だけで，
  production の道は増やさない．

## 5. 未決（orchestrator / Sol の判断待ち）

- A. `end_read`/`give_back` の mismatch の production の扱い（推奨：状態を変えない）．
- B. accessor 2 つを足すこと．`reading` を `#[cfg(test)]` にするかは実装で決める（`holder` は production なので不可）．
- C. P385 を残して当て直すこと（推奨）．
- D. `release_unneeded` の claim 後の loss 分岐は既存の probe では決定的に踏めない．claim を返すコードは必要だが，
  テストは review-only か，`release_claimed` のような probe を足すか（今回は足さない）．
- E. S2 の観測を line-lock の節数でよいか．別の観測が要るなら `Streams::refuse` に probe を 1 つ足す案がある．
- F. `read_reports_before_close` は `begin_read` が false のとき読みを skip する．`pass` の `plan_section` の
  false 分岐は現状到達しないが，残すなら `end_read` を忘れない．
- G. `tx.pass` の `begin_read` 失敗は二重 read のときだけ．失敗したら device を呼ばず `false` を返す．

## 6. していないこと

- コード，テスト，mutation 行を書いていない．`cargo test`/`cargo build`，`cargo fmt`/`rustfmt`，
  `mutate.py`，`remote_mutate.py` を流していない．commit・push・`git add`・`git stash` をしていない．
- `design/18-uhd-radio.md` はまだ編集していない（実装段階で UR-25 の Ownership 段落と `*Test:*`，
  Changes 表を直す．計画 §6 に，偽になる行 265–266・269・271・279 と，645 は履歴なので触らないことを書いた）．
- 作業 dir の orchestrator のファイル（`*-brief.md`，`.*.orig`，`*.pid`，`*.log`）と見張りのプロセスに触れていない．

## 7. 次の段階（実装）への申し送り

1. `test-plan.md` §7 の未決 A–G を決めてからコードを書く．決めた内容はこの report に追記する．
2. 実装後は計画 §8 の順で確認する（`core` の unit test → control/tx/rx → `tests/fake.rs` →
   `rules.py --base 8cd89e4`）．mutation は gate で `remote_mutate.py` のみ．
3. 14 行の当て直しは，`old` がちょうど 1 回一致するように関数名・前後の行まで含めて一意にする
   （P384・P386・P389・P399 は `busy` の代入が `give_back` に変わる）．P402 は `end_reading()` を
   `end_read(stream)` に変えるので必ず当て直す．P505・P512 は変更不要．
4. 新しいテスト N1–N10・T1・T2 を UR-25 の `*Test:*` 行に足す（実装段階）．

## 実装

2026-10-22、HEAD `8cd89e4` の上に R1 を実装した。上の §1–§7 は計画段階の記録で、現在の状態はこの節にある。

### 1. 計画の修正

`test-plan.md` の末尾に「## 修正（Sol の計画レビュー）」を足した。Sol の計画レビューの 7 点はすべて採用：

1. S4a は `reading_wait` probe（`await_reading` のループ内で `may_call_from_none` が false の判定の直後）で同期し、読みを解放する前に `holder == FromNone`、`reading == true`、`tx_open == 0` を確かめる。
2. S2 は RX/TX の拒否の節に `refused_booking` probe を置き、`holder == Owner { seq: Some(1) }` と、その時点の `try_lock()` の失敗（同じ line-lock の節）を直接観測する。
3. `release_unneeded` は claim の後・loss の検査の前に `release_claimed` probe を置き、loss 後の `holder == Free`・close が呼ばれないこと・Drop の panic がないことを確かめる。
4. 遷移表の抜け（`may_call_from_none` の `Free`/`Owner`/`Release`、`FromNone` の二重取得、idle で active のない `Owner` への `claim_release` の拒否）を N2/N3 で固定する。
5. N8 は本物の `Claim` を外に持ち、誤った返しだけを `catch_unwind` で捕まえ、panic の後も holder が変わらないことを確かめる。
6. N9 は受信 state を 2 つ用意して source と destination の line を入れ、同じ source で Claim を返して取り直し、`Some(7)`・`Some(8)`・`None` を比べる。private な field は直接変えない。
7. mutation は P387・P388・P396・P402 も追跡し、P384 の狙いを保ち、P398 は新しい `begin_read` の guard を外しても元の誤りを再現できるか確かめた（§4）。

未決 A–G は brief の指示どおり：A 状態を変えない（test build は panic）、B `holder` と `reading` の accessor（`reading` は production の from-none 判定でも使う）、C P385 を `Option::take` で残す、D 3 の probe、E 2 の probe、F `begin_read` が false なら読みを skip し、成功した読みだけ `end_read` と対にする、G `tx.pass` は `begin_read` が false なら device を呼ばず state も動かさない。

**順序の逸脱**：この修正節は実装の後に書いた（「`test-plan.md` を直してからコードを書く」を守れなかった）。コードとテストはこの内容に従っていることを確認した。これは §8 の穴にも書く。

### 2. 変えた本体

`git diff --numstat`（テストは同じファイルに含む）：

| path | + | − |
|---|---:|---:|
| `crates/ezsdr-radio-uhd/src/provider/core.rs` | 418 | 76 |
| `crates/ezsdr-radio-uhd/src/provider/control.rs` | 19 | 22 |
| `crates/ezsdr-radio-uhd/src/provider/rx.rs` | 59 | 14 |
| `crates/ezsdr-radio-uhd/src/provider/tx.rs` | 174 | 40 |
| `design/18-uhd-radio.md` | 6 | 4 |
| `plan/maintenance/tools/mutations.json` | 38 | 38 |

- **core.rs**：`Claims { holder, reading }`（field は private）、`Holder { Free, Owner { seq }, Release, FromNone }`、`Claim`（`#[must_use]`、test build の Drop は panic）、遷移 8 つ（`claim_owner`、`claim_release`、`claim_from_none`、`may_call_from_none`、`begin_read`、`begin_shutdown_read`、`end_read`、`give_back`）、`holder`/`reading` accessor。`owner_configure` は失敗で `Err((DeviceError, Claim))` を返し、`release_unneeded` は claim を取ってから loss を検査する。`move_source_busy` は `Owner { seq }` を読む。
- **control.rs**：`await_reading` は `may_call_from_none` で待ち、false の判定の後に `reading_wait`。from-none の claim と、早期終了・成功・拒否の全経路の `give_back`。
- **rx.rs**：`begin` の holder guard、拒否の book を claim 保持中に同じ節で行い、probe の後に `give_back`。
- **tx.rs**：`pass` の `begin_read`/`end_read`（`plan_section` の false 分岐も `end_read`）、`read` の holder guard、`do_switch` の拒否の book と `give_back`、`read_reports_before_close` の `begin_read`/`end_read`、`shutdown` の `begin_shutdown_read`/`end_read`、`end_reading` の削除。
- **design/18-uhd-radio.md**：UR-25 の Ownership 段落、`*Test:*` 行、Changes 表（§5）。
- **mutations.json**：22 行の当て直し。新しい行は足していない（§4）。

### 3. テスト

新しく足した 14 個：

- `core.rs`（10）：N1 `ur_25_a_claim_owner_takes_only_a_free_stream`、N2 `ur_25_a_claim_release_needs_a_free_idle_stream_with_no_active_segment`、N3 `ur_25_a_claim_from_none_admits_a_read_in_flight`、N4 `ur_25_a_begin_read_needs_a_free_stream_and_no_read`、N5 `ur_25_a_begin_shutdown_read_ignores_the_holder`、N6 `ur_25_an_end_read_without_a_read_panics_in_the_test_build`、N7 `ur_25_a_claim_dropped_without_give_back_panics_in_the_test_build`、N8 `ur_25_a_give_back_of_another_holder_panics_in_the_test_build_and_changes_nothing`、N9 `ur_25_move_source_busy_reads_the_owner_of_the_move`、`ur_34_a_release_gives_back_a_claim_when_loss_arrives_after_the_claim`。
- `rx.rs`（1）：`ur_25_a_refused_receive_owner_books_and_gives_back_in_one_line_section`。
- `tx.rs`（3）：`ur_25_a_refused_transmit_owner_books_and_gives_back_in_one_line_section`、T1 `ur_25_a_shutdown_read_and_a_returning_from_none_resolve_with_the_control_first`、T2 `ur_25_a_shutdown_read_and_a_returning_from_none_resolve_with_the_read_first`。

変えた既存テスト（削除・改名はない）：

- S4a `ur_25_a_from_none_enable_waits_for_a_transmit_report_read`（`reading_wait` で待ち、待っている間の holder/reading と open 前を assert）。
- S5 `ur_25_a_from_none_enable_ends_its_wait_at_the_provider_stop` と `ur_29_a_lost_device_ends_a_from_none_enable_s_wait_for_a_report_read`（join 後の `holder == Free` と reading の終わり）。
- `ur_25_a_transmit_poll_during_a_control_open_starts_no_streamer_call`（自己レビューの P2：`holder == FromNone` を追加）。
- `ur_25_a_detached_control_does_not_hold_the_shutdown_s_read`（`busy = true` の代わりに `claim_from_none` + `mem::forget`）。
- `ur_25_a_transmit_report_read_of_a_later_stream_does_not_hold_the_earlier_owner`（`begin_read`/`end_read` と `reading` accessor）。
- 旧 mark を直接読み書きしていた既存テスト：core の move/release の 9 個（`ur_34_a_waited_open_waits_for_same_move_source_configuration`、`ur_34_a_move_destination_waits_for_source_busy_after_channel_release`、`ur_34_a_release_skips_a_stream_held_busy_by_from_none`、`ur_34_a_waited_open_reloads_a_move_refused_while_it_waits`、`ur_34_a_waited_open_aborts_when_the_planned_segment_changes`、`ur_34_a_waited_open_ends_at_device_loss_without_closing_after_loss`、`ur_34_a_waited_open_ends_at_provider_stop_without_opening`、`ur_34_a_waited_transmit_open_observes_provider_stop_with_a_later_cold_reserved`、`ur_26_an_end_takes_delivered_from_each_segment_s_own_stream`）、control の `ur_25_a_from_none_holds_the_streamer_from_its_decision_to_its_booking`、rx の `ur_29_a_lost_receive_owner_configure_releases_stream_lock_before_reporting`・`ur_34_a_waited_receive_open_exits_when_stop_removes_its_segment`・`ur_25_the_line_lock_is_free_inside_every_receive_device_call`、tx の `ur_34_a_transmit_stop_during_release_wait_cancels_later_bursts`・`ur_29_a_lost_transmit_owner_configure_releases_stream_lock_before_reporting`。assert の意味は変えず、`holder`/`reading` に置き換えた。
- helper `rx_record`/`tx_record` は tuple の形を変えず、`holder(k) != Free` と `reading(k)` を返す。

### 4. mutation

- **当て直し 22 行（新規 0 行）**：計画の 14 行（P252、P253、P382–P386、P389、P392、P398、P399、P404、P455、P496）に加え、書き換えで `old` が一致しなくなった 8 行（P388、P402 の `end_reading`、J15、J16、P248、P249、P484、P521 の拒否/lock の節）。計画の 14 行という数は実装前の見積もりで、実際には「全行がちょうど 1 回一致する」規則を満たすために 8 行も当て直す必要があった。各 row の狙い（誤った stream に book する、book しない、lock を持ったまま報告する、など）は変えていない。
- P387/P396 は `old` がそのまま一致（`if reading` の行は変わっていない）。P455 は `may_call_from_none(stream)` を `!streams.reading(stream_key(Dir::Tx, 0))` に当て直し、「別の stream の読みを見る」誤りを保った。
- **P398**：`begin_read` の guard を単に外すと `end_read` が対にならず test build の panic が artifact になるため、`begin_shutdown_read` を呼ぶ形に当て直した。これで元の「enable の open 中に report を読む」誤りを再現しつつ、`end_read` は対のまま。kill を確認した。
- **grep**：`new` に `state.busy`、`.busy =`、`end_reading` を持つ行は 0。`filter` のテスト名はすべて実在する（J53 の `ur_29_a_` は意図した前方一致）。
- **静的検査**：6 リストの全 1261 行の `old` がソースにちょうど 1 回一致（0 bad）。
- **remote**：`maintenance` の 22 行を `sim02:3,sim03:3` で実行 → **22/22 killed**。自己レビューの P2 修正で殺し手が変わった P383/P398/P404 を再実行 → **3/3 killed**。全リストは orchestrator の gate。

### 5. spec 18

`design/18-uhd-radio.md`：

- **Ownership 段落**：`busy`/`busy_seq` を削除し、`Holder`（`Free`/`Owner { seq }`/`Release`/`FromNone`）と `reading`、`Claim` と `give_back` の言葉に書き直した。uhd-control の from-none は `claim_from_none` で取り、booking または enable を終える節で返す。uhd-tx の report read は `begin_read`/`end_read`、shutdown は holder を見ない `begin_shutdown_read`。holder が `Free` でない owner は device を呼ばず cursor と idle を動かさない。from-none が読みを見つけたら host clock で待ち、stop/loss で終わる。
- **「`busy` marks ...」の文**：claim の役割（`Owner { seq }`、`FromNone`、`Release`）に直した。
- **「source owner keeps ... busy mark」の文**：`Owner { seq }` の claim を両 line の拒否まで保持し、その後 `give_back` する、に直した。
- **`*Test:*` 行**：新しい 14 テスト（`ur_34_a_release_gives_back_a_claim_when_loss_arrives_after_the_claim` を含む）を足した。1500 文字を超えたため 2 行に分けた。
- **Changes 表**：2026-10-22 の UR-25 の行を足した。version は上げていない。2026-10-19 の履歴行は触っていない。
- `rules.py --base HEAD` は 0 problems。

### 6. 確かめたこと

| コマンド | 結果 |
|---|---|
| `cargo test -p ezsdr-radio-uhd` | pass（最終の workspace 実行で lib 251、`tests/fake.rs` 264、hardware/uhd_api 0） |
| `cargo test --workspace --no-fail-fast`（最終） | exit 0、1570 passed、0 failed |
| `cargo clippy --workspace --all-targets -- -D warnings`（最終） | exit 0 |
| docker `ezsdr-v4-dev:uhd4.10`: `cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings` | exit 0 |
| docker: `cargo test -p ezsdr-radio-uhd --features uhd --lib --test uhd_api` | lib 259 passed、uhd_api 5 passed、0 failed |
| `python3 plan/maintenance/tools/rules.py --base HEAD` | 0 problems |
| 決定性：`ur_25_` 57 テストを同じ binary で `--test-threads=8` × 20 回 | 20/20 pass（P2 修正後に再実行） |
| 決定性：release の 1 テスト × 20 回 | 20/20 pass（P2 修正後に再実行） |
| remote mutation（当て直した 22 行） | 22/22 killed |
| remote mutation（P2 修正後の P383/P398/P404） | 3/3 killed |

### 7. セルフレビュー

`opencode2 run --standalone -m "opencode/deepseek-v4.1-flash-free#max" --agent plan --auto "$(cat checklist.md)"` を 1 回（`review-1.log`）。**P1 なし**。指摘は 2 件で、どちらも直した：

- **P2** `crates/ezsdr-radio-uhd/src/provider/tx.rs`：`ur_25_a_transmit_poll_during_a_control_open_starts_no_streamer_call` に、control が streamer を持つ間の `holder == FromNone` の assert がなかった。`gate.wait()` の直後に足した。テストは pass。
- **P3** `crates/ezsdr-radio-uhd/src/provider/core.rs`：N9 の `Streams::with_streams(1, 2)` は Rx 1 の `StreamState` を作らない（キーは Rx 0 と Rx 1）。`(2, 1)` に直した。テストは pass。

P1 が無かったので、brief §4 の「P1 が無くなったら次の回は流さない」に従い 2 回目は流していない。P2/P3 の修正はテストだけ（production は変えていない）。P2 の修正後に決定性 20 回と P383/P398/P404 の remote を再実行した。

### 8. 穴

- **順序の逸脱**：`test-plan.md` の修正節を実装の後に書いた（「直してからコードを書く」を守れなかった）。内容が実装に一致することは確認したが、順序は brief と違う。
- **全 mutation リスト・イメージ build・Python スイート**は orchestrator の gate。私は当て直した 22 行と、P2 修正で殺し手が変わった 3 行だけを remote で流した。
- **ハードウェア（X310）では実行していない**。UHD feature のテストは docker の fake/FFI まで。bench はこの段階の範囲外。
- `owner_configure(...).unwrap()` を直接呼ぶ既存テストは、予期しない `Err` で `Claim::drop` の test panic と `unwrap` の panic が重なり abort し得る。現在その経路を踏むテストはない（新しい拒否のテストは `Err` を明示的に扱う）。production では `Claim` の Drop は panic しない。
- commit・push・`git add`・`git stash`・`cargo fmt`/`rustfmt` はしていない（禁止）。作業ツリーの変更は上記 6 ファイルだけ。
