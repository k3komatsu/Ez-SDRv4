# A2：テストの無い条件 9 件にテストを足す — 実装の指示書（risky でない）

あなたは実装役です．HEAD は `6f6d6c4`，tree はきれい．作業 dir は `~/.cache/ezsdr-fixes/2026-10-25-a2/`．

**作業 dir の orchestrator のファイル（`*-brief.md`，`checklist*.md`）は読むだけ．書き換えない．見張りのプロセスは止めない．**

## 0. 背景

計画 32（`plan/maintenance/32-test-line-sweep.md`，仕分けは `plan/maintenance/32-triage.json`）が「Test 行に試験の無い条件」を仕分け，A2 の 14 件を Phase 8 に回した．orchestrator が今の tree で確かめた結果，5 件はその後のテストで済んでいる（下の §3，spec の Test 行を直すだけ），**9 件にテストが無い**（§1）．規則の足りない件は無い．

この段階は**テストを足すだけ**で，本番のコードを変えない．テストが今のコードで落ちたら（＝コードが spec と違う），**コードを直さず**，そのテストを `#[ignore = "A2: <理由>"]` にもしないで，その件だけ止めて `report.md` に「spec と違う」として，再現の手順と期待・実際を書く．他の件は続ける．

## 1. 足すテスト（9 件）

各件：spec の条件，どこに，場面，assert，落とす誤り．テスト名は spec の規則 id で始める（`rm_11_…`，`mr_29_…`，`ur_17_…`）．`32-triage.json` の `test_sketch` は 2026-10-16 の tree のもので，行番号と一部の名前（`restart_gap_ns` は 1b-7b で消えた）が古い．今の tree で場所を探す．

1. **RM-11：`radio.CLOCK_LOST` の source は device の節点**．UHD：`crates/ezsdr-radio-uhd/tests/fake.rs` の `ur_27_a_lost_reference_is_clock_lost`（payload と termination しか見ていない）に，`events_of(&manifest, "radio.CLOCK_LOST")[0].source` が `usrp` の節点であることを足す（同じファイルの `DEVICE_LOST` の source の assert の形）．発行は `crates/ezsdr-radio-uhd/src/provider/control.rs` の `kinds::CLOCK_LOST`．MockRadio は CLOCK_LOST を出さない（確かめたら report に書く）．
2. **RM-14：admission が target を進めたとき，`requested_target` は SC-23a の `requested_at`**．`requested_at: Some(..)` の `TxBurst` を Provider に渡す試験はまだ無い（Kernel の `run_session.rs:1462` は admission だけ）．MockRadio（`crates/ezsdr-mock-radio/src/lib.rs` の `requested_at` を読む所）と UHD（`control.rs` の `book_burst` の `requested_at`）の**両方**に 1 本ずつ：`at` より前の `requested_at` を持つ burst を，(a) `OnTime` で間に合う `at`（記録の `requested_target` が `requested_at`，`target` が `at`），(b) `SendAsap` で動かされる `at`（`requested_target` は `at` でなく `requested_at`）．Mock は `tests/mock_radio.rs` の `tx_action` ヘルパ（`requested_at: None` 固定）を引数で選べるようにしてよい（既存の呼び出しは `None` のまま）．UHD は `control.rs` か `tx.rs` の unit テストの既存の部品を使う．落とす誤り：`requested_at` を捨てて `at` を記録する．
3. **MR-19：restart gap が 0（`ideal`）でも落ちた block は失われる**．`mr_19_backpressure_is_an_overrun`（x3xx，gap 50 ms）の隣に `ideal`＋`BackPressure::Block` capacity 1：block を 1 つ Full で落とし，次の block が落ちた block の終わり `b` から始まり，`GAP_BEFORE | RESTARTED`，`lost` = 時間の跳び（= block_len），hot path の RX_OVERFLOW の `lost` も同じ．落とす誤り：`k_g = a + gap`（gap 0 で落ちた block を重複・逆戻りさせる）．
4. **MR-29：`start_rx` は `at` で効く（受け取りより後なら）**．`Harness::step(t)` は Authority を `t` に進めてから Action を処理する（受け取り = push の後の最初の step の時刻）．`ideal` で受信 stream を `Stop` した後，受け取り + 10 ms の `at` の `start_rx`：新しい受信 clock の origin がその格子の instant，LATE_COMMAND 0 件．`mock_layout.rs` の `start_rx("mock/rx/s1", Some(10_000_000))` は受け取りと `at` が同じ時刻なので見分けない．落とす誤り：`at` を無視して受け取りで再開する．
5. **MR-29：受信 link のある stream は T0 より前に始まらない**．`at` が T0 より前の `start_rx`（T0 の前に `Stop` と `start_rx` が届く場面，または受け取りが T0 の前の場面．今の harness で作れる形を探す）：新しい clock の origin が T0 以後，T0 より前の block が無い．作れなければこの件だけ止めて理由を報告．
6. **MR-29：`start_rx` は決して late でない**．`Stop` の後，過去の `at` の `start_rx`：drain した event に `radio.LATE_COMMAND` が無く，再開は受け取りの時刻．`mr_29_start_rx_is_the_one_command` を延ばすか隣に足す．落とす誤り：過去の `at` で LATE_COMMAND を出し lead だけ遅らせる．
7. **UR-13：`arm` は何も放射しない**．送信 stream のある rig で `arm` の直後（`start` の前）に `calls(&device, "tx_send")` が空で，送信の streamer の open は記録されている．`ur_13_arm_cases` に足すか隣に．落とす誤り：arm で空の end-of-burst や最初の buffer を送る．
8. **UR-17：受信 block の planar cf32 の配置（複数 channel）**．`crates/ezsdr-radio-uhd/src/provider/rx.rs` の unit テスト（`ur_19_reports_before_one_block_keep_each_kind` の近く）で 2 channel の stream に channel ごとに違う値の `RxRecv::Samples` を与えて公開し，block の bytes を `ezsdr_hostmem::read_cf32(bytes, n, c, i)` で読んで各 channel の値が入れ替わっていないこと（index もずれていないこと）．落とす誤り：`write_cf32` の channel と index の取り違え．
9. **UR-17：missed start の再開の lead は決定の時の `now` から測る**．`fake.rs` の `ur_17_a_missed_start_restarts_with_a_gap` は `applied − T0 ≥ 50 ms` しか見ない．late を決めた instant が T0 より十分後になる場面（manual clock で決定の時刻を決める）で，`applied` が「決定の instant + `rx_start_lead_ns` の後の最初の格子の instant」に等しく，`T0 + lead` より厳密に後．unit テスト（`rx.rs` の `RxRecv::LateCommand`）の方が時刻を決めやすければそちらでよい．落とす誤り：lead を T0 や origin から測る．

## 2. mutation の行

各件に 1 行以上：その試験が pin するコードの式を壊す行を `plan/maintenance/tools/mutations.json`（UHD の行が Phase 7 のリストにあるならそちら）に足し，新しい試験で killed を確かめる．id は各リストの既存の続き．A1 の行（T01〜T33）と同じ形．`mutations.json` は**行を足すだけにし，ファイル全体を書き直さない**（字下げは 1 個の空白のまま，前回は全体が 2 個に変わった）．`git diff --stat` が足した行の分だけであることを確かめる．

## 3. spec の Test 行

`design/07-radio-model.md`，`design/09-mock-radio.md`，`design/18-uhd-radio.md` の該当の `*Test:*` 行に新しいテスト名を入れ，その条件の「none named for …」を消す（他の条件の「none named」は残す）．加えて，既にテストがある 5 件の Test 行を直す：

- RM-11：TX_UNDERFLOW の source は `ur_28_a_layout_transmit_report_names_its_stream`（`fake.rs`）が見ている．RM-11 の Test 行に足す．
- RM-18：`mr_22_sequence_error_shape` が `cause: Sequence` を見ている（既に名がある．直さない）．
- MR-25：`mr_25_a_stop_records_a_later_update_as_cancelled_at_the_stop` が `stop(mode)` の pending の記録を見ている．Test 行の「none named for what `stop(mode)` records of pending commands」を消す（このテストは既に列にある）．
- UR-17：missed start の `GAP_BEFORE` と `lost` は既に名がある（直さない）．
- UR-18：`ur_19_reports_before_one_block_keep_each_kind` が cause を見ている（既に名がある）．

DA-1 の形（AGENTS.md §2）を守る．`rules.py --base HEAD` が 0 件．

## 4. 確かめること・セルフレビュー・報告・守ること

38a と同じ：`~/.cache/ezsdr-fixes/2026-10-24-note38/impl-brief.md` の §3（新しいテストを各 20 回，`cargo test --workspace`，clippy，rules，`git diff --check`，mutation は `remote_mutate.py <list> sim02:2 <ID> ...`，mode 644，負荷をかけない，TMPDIR を変えない），§4（セルフレビュー：reviewer は `openai/gpt-6-luna#max`，checklist は **この dir の `checklist-a2.md`**，log は **この dir の `review-a2-<n>.log`**），§5（`report.md` をこの dir に．件ごとにテスト名，`path:line`，mutation の行と結果，止めた件），§6（cargo fmt・ローカル mutate.py・commit 禁止など）．

本番のコード（`src/` のテストでない部分）を変えない．テストの部品（ヘルパの引数を足す等）は最小限．新しい rig や fake の新しい fault は足さない．

---

# 付録：直し 1 回目の指示書（fix1-brief.md）


あなたは実装役です．A2 の未 commit の変更（HEAD `6f6d6c4`）の上で，Sol のコードレビュー `sol-code-verdict.md` の 2 件を直す．守ることと確かめ方は `impl-brief.md` §4 と同じ（セルフレビューの checklist は `checklist-fix1.md`，log は `review-fix1-<n>.log`）．作業 dir の orchestrator のファイル（`*-brief.md`，`checklist*.md`，`sol-*`，`opus-review.md`，`.*.orig`）は読むだけ．本番のコードは変えない．`mutations.json` は**行を足す・直すだけ**（字下げは 1 個の空白のまま）．

## 1. orchestrator の裁定（2 件とも採る．1 は orchestrator の確認 O1 と同じ）

1. **major：`ur_17_a_block_s_channels_are_planar_cf32`（`crates/ezsdr-radio-uhd/src/provider/rx.rs`）が sample の index の誤りを見分けない**．channel ごとに一定の値なので，channel の中の並べ替えや，全 index に最初の sample を書く誤りでも通る．直し：channel と index の両方で違う値（例 channel `c`，index `i` に `[10c + i, -(10c + i)]`）を与え，全位置を照合する．mutation 行を 1 行足す：`write_cf32` に渡す sample の index を 0 に固定する（または channel の中で逆順にする）変異が，このテストで killed．
2. **minor：T39 は T0 の下限の違反でなく「clock が無い」で落ちる**．語彙の `command_instant`（`crates/ezsdr-radio/src/timeline.rs:380`）の受信の floor を直接見る小さなテストを `crates/ezsdr-radio/tests/timeline.rs` に足す（名は `rm_26_` か `mr_29_` で始める．例：受信の line（origin = T0）に，`at` と `now` がともに T0 より前の `start_rx` 相当の `command_instant(Some(&line), Some(t0 - x), t0 - y, seq, false)` → `e == t0`．送信の line では floor が掛からない（`e` は `max(at, now)`）ことも 1 行）．T39 は 1 箇所の変異（`base = asked.max(now)`，floor を外す）に戻し，この新しいテストで killed にする（filter をこのテストに）．Provider 経由の `mr_29_a_receive_link_never_begins_before_t0` は残す．`design/07` か `09` の該当の Test 行（MR-29 の T0 の floor）に新しいテストの名を足す．

O2（`mr_29_start_rx_is_never_late` が `ideal` の lead 0）は直さない．

## 2. 確かめること・報告

- 新しいテストと直したテストを各 20 回，`cargo test -p ezsdr-radio`・`-p ezsdr-radio-uhd --lib`，`cargo test --workspace`，clippy，`rules.py --base HEAD`，`git diff --check`．足した行と直した行（T39，新しい行）を `remote_mutate.py maintenance sim02:2 <ID> ...` で流して killed．`git diff --stat plan/maintenance/tools/mutations.json` が追加・修正の行の分だけ．変えたファイルは mode 644．
- セルフレビュー：checklist は `checklist-fix1.md`．P1 以上が無くなるか 3 回で止める．
- `report.md` の最後に「## 8. 直し 1 回目」を足す（前の節は消さない）．最初の行は `STATUS: done` か `STATUS: blocked`．**起きたことを書く．していないことは書かない．**
