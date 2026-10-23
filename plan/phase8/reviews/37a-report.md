# 設計メモ 37 の 37a：実装役（DeepSeek V4.1 Flash）の報告

# 設計メモ 37（#69）37a 実装 — 報告

## やったこと（ファイルごと）

- `crates/ezsdr-radio-uhd/tests/README.md`: メモ 37 §3 の規則（判定は試験のスレッドがいつ動くかに依らない，(1)〜(4)，「十分前」は (2) ではない）を，manual clock の段落の直後に足した．
- `crates/ezsdr-radio-uhd/src/provider/tx.rs`（`#[cfg(test)]` の中だけ）: 表 A の A-1〜A-5 と，表から unit test へ移した `ur_22_a_first_send_cut_short_keeps_the_reports_paired`・`ur_23_bursts_go_out_in_target_order` を足した．A-0 の 15 回の `step` は `at_a_held_tail` 1 つにまとめた（`manual_rig` は変えていない）．装置の送信は `transmitted` の値，`tx_send` の `n`/`sob`/`eob`，`unended_bursts`，`rejected`，`device_async` で見る．
- `crates/ezsdr-radio-uhd/src/provider/control.rs`（`#[cfg(test)]` の中だけ）: 表から unit test へ移した `ur_21_a_burst_before_start_up_is_refused` と `ur_21_a_burst_before_its_clock_s_origin_is_late` を足した．どちらも制御時計の booking サイトで，`TIME_ERROR`/`COMMAND_REJECTED` のイベントと uhd-tx への引き渡しを見る．
- `crates/ezsdr-radio-uhd/tests/fake.rs`: 表 A の fake 側を処分し（下記），B-1 の 18 系列を直した．新しい入力を作る小さな `second_waveform` と `hash_of`，Spec を T0 から進める `finish_spec` を足した．
- `crates/ezsdr-acceptance/tests/uhd.rs`: `uhd_61_02` の capture の開始を `first >= index` と長さ 100 に緩めた（厳密な開始は `uhd_59` が Spec で pin する，とコメント 1 行）．
- `crates/ezsdr-server/tests/protocol.rs`: `ea_17_a_capture_ahead_starts_at_its_instant` を `first >= at` と長さ 1 000 に緩めた（同じコメント）．
- `design/18-uhd-radio.md`: §6 の 7 行（`ur_21_late_policies_on_the_device_lead`，`ur_21_a_burst_before_start_up_is_refused`，`ur_21_a_burst_before_its_clock_s_origin_is_late`，`ur_23_a_burst_ends_at_the_next_bursts_start`，`ur_23_bursts_go_out_in_target_order`，`ur_23_a_burst_one_sample_after_a_burst_is_played`，`ur_28_the_device_reports_a_late_burst`）を今のテストの形に直し，unit test になった行（同じ 7 行のほか，T1/T2/T4/T5 の 4 行）に `(unit)` を付けた．Changes に 2026-10-23 の 1 行を足した．規則本文は変えていない（動作の変更なし）．
- `plan/phase7/tools/mutations.json`: 当て直し 12 行と新行 B41（下記）．形式は元のまま（手で行を差し替え，全体の再整形はしていない）．

本番のコード（`crates/*/src/` の `#[cfg(test)]` の外）は変えていない．`cargo fmt`/`rustfmt`，`mutate.py` のローカル実行，`git add`・commit・push・stash はしていない．作業 dir の orchestrator のファイルと watchdog は触っていない．

## 表 A（`ur_23` の 5 本と `ur_21_a_descheduled_…`）

| 表 | 処分 |
|---|---|
| A-1 T1 | fake から**削除**．同名の unit test（`tx.rs`）が代わり．fake 側の窓の要らない形は `ur_23_a_burst_ends_at_the_next_bursts_start` と `ur_23_two_bursts_back_to_back_loop_back_whole` が持つ |
| A-2 T2 | fake から**削除**．同名の unit test が代わり．窓の要らない `Stop` は `ur_23_a_stop_ends_the_burst_within_the_in_flight_window` |
| A-3 T3 | fake 側を Spec（A 10 000 / B 40 001，両方開始前）に**書き換え**，同名の unit test を `tx.rs` に置いた |
| A-4 T4 | fake から**削除**．同名の unit test が代わり．窓の要らない形は `ur_22_a_send_cut_short_ends_the_device_burst_after_what_it_took` |
| A-5 T5 | fake から**削除**．同名の unit test が代わり（B29 は `lib`） |
| A-6 T6 | fake に残し，`:3750` の `send` を `start_repeat` にした（J295 は `fake` のまま） |

表 A の assert は表の「assert / 観測」どおり: A-1 は `transmitted` が A ++ B の 31 000 サンプル（B は A と違う波形），handed 31 000，sob/eob 各 1，`unended_bursts` 0，TimeError/Underflow なし．A-2 は A の 30 000 と eob 1．A-3 は c の gap と B の 30 001 番目以降，sob/eob 各 2．A-4 は A の 29 999 の後の零 1 個と reason，A-5 は 30 000 の後の零 1 個．目的の状態（`open`/`continues_at`/`tail`）も各テストで見ている．

## 表 B-1（18 系列）— 表から変えた所とその理由

表の分類どおりに直したのは #1（同じ start を同じテスト内の Spec 2 本に），#3（20 ms を `start_repeat`，コメントも直した），#4（5 ms の 2 本だけを判定し，先頭は SendAsap か無しかを許す），#6/#7/#12/#14/#15（Spec），#9（`stop_at` を Stop の `LogEntry.time` に），#10（burst と cold change を同じ固定 offset に，burst を先に），#11（`start_repeat`，late の target は record の moved target），#13（Spec），#17/#18（`start_repeat`）．

**表から変えた 4 系列**（いずれも「Spec では性質が作れない」ことを実行して確かめた）:

- **#2 `ur_21_a_burst_before_start_up_is_refused`** → `control.rs` の unit test．Spec の offset は T0 からで，start-up は T0 以前なので，Spec では start-up より前の start を作れない．Session の固定 target（T0−1 ms）でも，試験のスレッドが 2 s 遅れれば late policy の Drop が先に立つ（セルフレビュー R1 の P1）．制御時計で `core.start_up` を置き，20 ms 先の booking が `COMMAND_REJECTED`（理由と action id）になり，uhd-tx に渡らないことを見る．
- **#5 `ur_21_a_burst_before_its_clock_s_origin_is_late`** → `control.rs` の unit test．Spec で cold change の後に burst を置くと，`resolve_schedule` が prepare で宣言した時計の domain に解決するため，booking が「UR-21: the start is not on the current transmit SampleClock」で拒否される（実行して確認）．制御時計で rate change を booking し，新しい時計の origin の 5 ms 前に start を置くと，SendAsap と「origin に移され，hand-over 前に予約される」が見える．
- **#8 `ur_23_bursts_go_out_in_target_order`** → `tx.rs` の unit test．KC-16 が schedule を instant 順に並べるので，Spec の 2 本は **target 順に到着**し，「到着順ではなく target 順」という RM-15 の性質を試せない（セルフレビュー R1 の P1）．制御時計で 200 ms → 100 ms の順に `Held` を入れ，100 ms が先に device へ渡り，200 ms が held のままであることを見る．U18 は `lib` に当て直した．
- **#16 `ur_22_a_first_send_cut_short_keeps_the_reports_paired`** → `tx.rs` の unit test．Spec で B を先に予約すると，B の held start が A の最初の send を切るため，「A の最初の send が短く終わり，A が放棄された後に B を A の device の終わりに予約する」という図が消える（最初の書き換えで実行して確認: A の最初の send が 999 ではなく 500 に切られた）．制御時計で A を 31 ms に 1 回 step して放棄させ，その後に B を 40 500 に予約し，device が B を落として 1 つの `late_at_device` が B を名指すことを見る．

#1 の duplicate 部分は，同じテストの中で Session（odd size，NaN）と Spec（同じ offset の 2 本）に分けた．Spec 側の拒否も `COMMAND_REJECTED` の action id を見る（J197 の kill を保つ）．#16 の移動に伴い B26/B27 は `lib`．

## 移した・消したテストと代わり

- 消した fake テスト（名前は `tx.rs` の unit test が引き継ぐ）: `ur_23_a_burst_booked_at_a_sent_burst_s_end_continues_it`（T1），`ur_23_a_stop_while_the_device_burst_waits_for_a_continuation_ends_it`（T2），`ur_23_a_continuation_whose_held_sample_is_not_taken_is_abandoned`（T4），`ur_23_a_continuation_s_first_send_that_takes_nothing_ends_the_device_burst`（T5）．fake 側の窓の要らない形は表のとおり `:1523`（`ur_23_a_burst_ends_at_the_next_bursts_start`，今は Spec），`:3269`（`ur_23_two_bursts_back_to_back_loop_back_whole`），`:1557`（`ur_23_a_stop_ends_the_burst_within_the_in_flight_window`），`:3326`（`ur_22_a_send_cut_short_ends_the_device_burst_after_what_it_took`，今は Spec）．
- fake から消して unit test に移したもの: `ur_21_a_burst_before_start_up_is_refused`，`ur_21_a_burst_before_its_clock_s_origin_is_late`，`ur_23_bursts_go_out_in_target_order`，`ur_22_a_first_send_cut_short_keeps_the_reports_paired`．
- 名前を変えたテストはない．spec 18 の `:248`・`:458-467` の参照はすべて解決する（§6 の行だけ今の形に直した）．

## mutation

当て直し（`test_bin: fake` → `lib`，`filter` は同名のまま）: B08，B10，B16，B20，B26，B27，B28，B29，B31，B32，U18，U46．J295・J317・J197 は `fake` のまま（J295 は移していない; J197 は #1 の Session+Spec の両方で action id を見る; J317 は #18 の `start_repeat` 後も join を見る）．**新行 B41**: `close_device_burst` が tail の代わりに空の end-of-burst を送る変異，`filter: ur_23_a_burst_one_sample_after_a_burst_is_played`，`test_bin: lib`．`old` は tree で 1 回だけ一致することを全 13 行で確認した．

`remote_mutate.py phase7,maintenance sim02:4` で 30 行（B03 B04 B08 B09 B10 B16 B18 B19 B20 B21 B25 B26 B27 B28 B29 B31 B32 B41 U17 U18 U19 U28 U36 U44 U46 R06 R07 J197 J295 J317）を流した結果: 初回 29/30 killed，**U17 だけ survived**．U17 は「repeat が次の burst の start で終わる」ことを record の `end` でも見るよう `BurstEnd::Eob` の assert を足して killed を確認（再流し）．U18 は unit 移設後に再流しして killed．その後 U46 と B41 を再流しして killed．最終的にこの 30 行はすべて killed．(J295 は `start_repeat` にしても kill された．)

## 確かめたこと

- 20 回連続（`timeout 300`，1 binary ずつ，`--test-threads` 既定，負荷なし）:
  - `tx.rs` unit: A-1〜A-5，`ur_22_a_first_send_cut_short_keeps_the_reports_paired`，`ur_23_bursts_go_out_in_target_order`，`control.rs` unit: `ur_21_a_burst_before_its_clock_s_origin_is_late`，`ur_21_a_burst_before_start_up_is_refused` — 各 20/20．
  - fake: `ur_21_txburst_refusals`，`ur_21_late_policies_on_the_device_lead`，`ur_21_a_burst_inside_the_in_flight_window_is_late`，`ur_22_a_single_burst_ends_with_end_of_burst`，`ur_23_a_burst_ends_at_the_next_bursts_start`，`ur_23_a_stop_ends_the_burst_within_the_in_flight_window`，`ur_25_a_cold_transmit_change_cancels_the_held_bursts`，`ur_28_the_device_reports_a_late_burst`，`ur_23_a_burst_alone_is_ended_before_it_runs_dry`，`ur_23_a_burst_one_sample_after_a_burst_is_played`，`ur_23_a_one_sample_burst_booked_ahead_starts_at_its_time`，`ur_22_a_send_cut_short_ends_the_device_burst_after_what_it_took`，`ur_21_a_descheduled_timed_send_is_on_time_on_the_manual_clock_and_late_on_the_real_one`，`ur_33_a_shutdown_with_a_report_read_that_fails_at_once_still_joins_uhd_tx` — 各 20/20（`ur_21_a_burst_before_start_up_is_refused` と `ur_23_bursts_go_out_in_target_order` は fake 版を移設前に，unit 版を移設後に 20/20）．
  - `uhd_61_02` と `ea_17_a_capture_ahead_starts_at_its_instant` — 各 20/20．
- `cargo test -p ezsdr-radio-uhd --test fake`: 256 passed，0 failed．`--lib`: 275 passed，0 failed．`cargo test -p ezsdr-acceptance --test uhd`: 9 passed．`cargo test -p ezsdr-server --test protocol`: 43 passed．
- `cargo clippy --workspace --all-targets -- -D warnings`: 0 warnings．`rules.py --base HEAD`: 0 problem．`git diff --check`: clean．
- mutation の実行: 上記のとおり `remote_mutate.py` のみ（ローカルで `mutate.py` は流していない）．負荷試験・再現（16 busy loop）・Docker gate・全 workspace のテストは orchestrator の gate なので流していない．

## セルフレビュー（`review-1.log`〜`review-3.log`，Luna max，読むだけ）

- **R1**: P1 2（#8 が逆順到着を試せない，#2 に実時間依存が残る），P2 1（scratch の出力が `ur_24_a_released_command_is_not_recalled_by_stop` に残っていた），P3 3．
  - #8 → `tx.rs` の unit test に移した（上記）．#2 → `control.rs` の unit test に移した（上記）．P2 → scratch の痕跡を消し，そのテストを元の fail-fast な形に戻した．
  - P3 の「manual clock の説明が誤り」は**退けた**: `tx_session` は `common::fake` を使い，`FakeConfig::default()` の `Real` を `ClockMode::Manual` で上書きする（`common/mod.rs` の `fake`）．P3 の helper は**退けた**: impl2-brief §3.2 が「tests の中の小さな関数 1 つにまとめてよい」とし，`at_a_held_tail` は `rig_with(FakeConfig { clock: Manual, command_queue: 64, faults, ..default() }, Vec::new())` を直接呼び，`manual_rig` を変えていない．P3 の `table.md` の U09 は，現行リストでその filter を持つ行が J197 である表の誤記（前段の表は書き換えず，ここに記録する）．
- **R2**: P1 1（A-1 が Underflow を見ていない），P2 2（#1 の拒否数が `>= 2`，#5 の `-5 000` が 2 MS/s で 5 ms でない），P3 3．
  - A-1 の `device_async` に Underflow を足した．#1 を `assert_eq!(rejected.len(), 2)` にした．#5 を `-10_000`（5 ms）に直した．P3 は R1 と同じ理由で退けた（helper，manual clock）／表の誤記として記録．
- **R3**: P1 0，P2 1（start-up の unit test がイベントを見ていない）．イベント収集器を残し，`COMMAND_REJECTED` の理由と action id を assert した．3 回で P1 以上は 0 になったので止めた（P2 は修正済み）．
- どの回も reviewer に mutation は流させていない．テストは checklist の `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review` で流れた．

## 残ったこと

- **37b は触っていない**: B-2（raw device の 13 本を `src/device.rs` へ），B-3（`Direct` 系）．
- 表 A-5 の「fake に残す代替案」は採らず，推奨どおり `tx.rs` へ移した（B29 は `lib`）．
- 負荷の下での再現（16 busy loop，`fake` 3 回），計画 30 §15 の負荷表，Docker gate，全 mutation リストは orchestrator の gate．
- `table.md` の誤記 1 件（`ur_21_txburst_refusals` の行を U09 としているが，現行リストでは J197）は前段の表の記録として残した．

## 修正の回（Sol）

Sol のコードレビュー（`sol-code.log`，判定 FAIL）の 2 件の major をどちらも採り，自己レビュー 2 回（`review-4.log`，`review-5.log`）の指摘を直した回．HEAD `814747a` のまま，tree は 37a の差分にこの回の変更を重ねたもの．本番のコード（`crates/*/src/` の `#[cfg(test)]` の外）は変えていない．

### 何を変えたか

- `crates/ezsdr-radio-uhd/src/provider/tx.rs`（`#[cfg(test)]` の中だけ）
  - **major 2 → `ur_21_a_burst_inside_the_in_flight_window_is_late`（unit）を新設**．`booking_control`（既存の `Control::new` に tx の start を渡すだけの小さな helper．新しい rig の種類ではない）で，制御した時計の 20 ms に A（30 000 sample）を 29 ms に予約し，2 000 sample ずつ 39 ms まで引き渡す．29 ms に B を 34 000（5 ms 先，device lead 2 ms を超えるので予約では on time）で予約し，`Control` が B を k=34 000 のまま渡すことと `TIME_ERROR` が予約で出ないことを assert．uhd-tx の最初の hand-over で `preempt` が再判定し，`send_asap` は 39 000 に動いてそこから A の device burst を継続して鳴り（`transmitted` の値，`sends` の `(1000,false,false)`，device の `TimeError` なし），`drop_and_flag` は device 呼び出しなしで落ちる．`TIME_ERROR` の target はどちらも 34 000．予約・hand-over・移動先の予約 (`core.held`) も見る．予約時点で on time だったことは，booking が k を動かさず verdict を出さないことで観測する．
  - **major 1 → `ur_23_a_stop_ends_the_burst_within_the_in_flight_window`（unit）を新設**．制御した時計の 12 ms に A（20 ms，30 000 sample）の最初の 2 000 sample を引き渡し（record の終わりは 22 ms = stop + 10 ms），`stop_now`．record の end が `stop` で，record の終わり ≤ stop + `IN_FLIGHT_WINDOW_NS` であること，device の BurstAck が record の終わり + empty end-of-burst の 1 zero sample であること，`TimeError`/`Underflow` がないことを見る．`Frozen`，`device_at`，`stop_now`，`rig_with` だけで作った（新しい rig はない）．
  - T2 `ur_23_a_stop_while_the_device_burst_waits_for_a_continuation_ends_it` を強化（review-5 の P2）: `transmitted` を 30 001 sample にして A の後に余分な zero sample がないこと，最後の send が `(1,false,true)`（tail が EOB の buffer そのもの）であることを assert．
- `crates/ezsdr-radio-uhd/tests/fake.rs`
  - **major 2 → `ur_21_a_burst_inside_the_in_flight_window_is_late` を削除**．代わりは `tx.rs` の同名 unit test（上）．fake 側には，予約の verdict と hand-over の再判定を区別する隙間の依らない形が作れないため（遅れた予約でも同じ `[SendAsap, Drop]` になる），消す方を採った．
  - **major 1 → `ur_23_a_stop_ends_the_burst_within_the_in_flight_window` を書き換え**．窓の上限は unit test に移し，fake 側は隙間に依らない性質だけにした: record の end が `Stop`，record の終わり ≥ Stop の ledger 時刻（RM-16: 引き渡した sample は recall しない），`tx_send` の eob が 1 本，`unended_bursts` 0，`async` に Underflow なし．`stop_at` は `LogEntry.time` のまま（上限の基準には使わない）．
  - `ur_21_late_policies_on_the_device_lead`（review-4 の P1）: 3 本目の `start_repeat` の target を `tx_at` で取り，`admitted`，record が 2 本，3 本目の `target` ≥ 要求，`requested_target` が要求と一致（動いた場合のみ）を assert．2 本目以降の `TIME_ERROR` は 2 本目の要求か 3 本目の要求の target で `SendAsap`（review-5 の P1: 2 本目の hand-over 再判定は 2 本目の target を持つ）．
  - `ur_21_late_policies_on_the_device_lead` の古いコメント（"the unit test below"）を `src/provider/tx.rs` の unit test を指すように直した．
- `crates/ezsdr-radio-uhd/tests/README.md`: 変更なし（37a で足した規則のまま）．
- `design/18-uhd-radio.md`: §6 の 2 本の行を今の形にし，`(unit)` を付けた（`ur_21_a_burst_inside_the_in_flight_window_is_late` は unit のみ，`ur_23_a_stop_ends_the_burst_within_the_in_flight_window` は unit + 同名の fake が end-of-burst の部分を持つ）．UR-23 の `*Test:*` の「fake の wider acceptance」の括弧を制御時計の説明に直した．§9 の U44 の「probably, not certainly」を，unit test が決定的に kill する説明に直した．Changes にこの回の 1 行を足した．規則本文は変えていない（動作の変更なし）．
- `plan/phase7/tools/mutations.json`: **U19，U36，U44，R06，B04 の `test_bin: "fake"` → `"lib"`**（`filter` は同名のまま）．`old` は 5 行とも tree で 1 回だけ一致することを確かめた（全 6 リストの全行も `old` が 1 回一致することを確認）．B10 は `lib` のままで，T2 の assert 変更に伴い再流しした．
- `table.md`（作業 dir，review-4 の P2）: `ur_21_txburst_refusals` の mutation 行を U09 → J197 に直した（現行リストでその filter を持つのは J197）．前段の「残ったこと」の表の誤記の記録はその時点のものとして残した．

### 移した・消したテストと代わり

- **削除**: fake の `ur_21_a_burst_inside_the_in_flight_window_is_late`（3 回の繰り返し）→ `tx.rs` の同名 unit test（上）が代わり．fake 側に残る形はない（予約と hand-over を区別できないため）．
- **新設（unit）**: `ur_21_a_burst_inside_the_in_flight_window_is_late`，`ur_23_a_stop_ends_the_burst_within_the_in_flight_window`．どちらも `tx.rs` の tests．
- **書き換え**: fake の `ur_23_a_stop_ends_the_burst_within_the_in_flight_window`（上限を unit へ，end-of-burst と no Underflow を残す），`ur_21_late_policies_on_the_device_lead`（3 本目の受付と記録，2 本目以降の target）．
- 名前は変えていない．spec 18 の参照はすべて解決する（§6 の 2 行を今の形にした）．

### mutation の結果

`chmod 644` の後 `REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py phase7 sim02:4 ...` で流した（ローカルで `mutate.py` は流していない）．

- 1 回目（5 行）: U19，U36，U44，R06，B04 → **5/5 killed**．
- 2 回目（T2 の assert 変更後，6 行）: U19，U36，U44，R06，B04，B10 → **6/6 killed**．
- どの行も `old` が tree で 1 回だけ一致し，`test_bin: "lib"` の filter は `tx.rs` の unit test に解決する．U19 は unit test の `open.next == 22 000` と record の窓の assert が kill する（変異で guard が消えると 8 回の step が 16 000 sample を送り，record の終わりが 36 ms になる）．U36/U44 は unit test の `TIME_ERROR` の outcome と移動先の assert，R06 は `sends.last() == (1 000,false,false)` と device の `TimeError` なし，B04 は B が A の device burst を継続して鳴ること，B10 は T2 の最後の send `(1,false,true)` が kill する．

### 確かめたこと（この回）

- 20 回連続（`timeout 300`，1 binary ずつ，`--test-threads` 既定，負荷なし）:
  - `tx.rs` unit `ur_21_a_burst_inside_the_in_flight_window_is_late` — 20/20．
  - `tx.rs` unit `ur_23_a_stop_ends_the_burst_within_the_in_flight_window` — 20/20．
  - fake `ur_23_a_stop_ends_the_burst_within_the_in_flight_window` — 20/20．
  - `tx.rs` unit T2（assert 強化後）— 20/20．
  - fake `ur_21_late_policies_on_the_device_lead`（P1 修正後）— 20/20．
- 全体: `cargo test -p ezsdr-radio-uhd --lib` 277 passed，0 failed．`--test fake` 255 passed，0 failed（削除した 1 本だけ減った）．
- `cargo clippy --workspace --all-targets -- -D warnings`: 0 warnings．`rules.py --base HEAD`: 0 problem．`git diff --check`: clean．
- 負荷試験・再現（16 busy loop），Docker gate，全 mutation リストは orchestrator の gate なので流していない．

### セルフレビュー（`review-4.log`，`review-5.log`，Luna max，読むだけ）

- **review-4**: P1 1（`ur_21_late_policies_on_the_device_lead` の 3 本目の予約の結果を捨てていて，`outcomes[2..]` が空でも通る），P2 1（`table.md` の U09 は誤りで，現行は J197）．
  - P1 → `third_at` を取り，`admitted`，record 2 本，3 本目の `target`/`requested_target` を assert する形に直した（上）．P2 → `table.md` の 2 箇所を J197 に直した．
- **review-5**: P1 1（2 本目の hand-over 再判定の `TIME_ERROR` は 2 本目の target を持つのに，`errors[2..]` がすべて 3 本目の target を要求していた），P2 1（T2 が EOB を最後の sample に付けたことを見ていない），P3 1（§6 の説明が repeat と書くが unit test の A は repeat でない）．
  - P1 → `second_at` を取り，`errors[2..]` は 2 本目か 3 本目の target の `SendAsap` を許す形に直した．P2 → T2 を 30 001 sample と最後の send `(1,false,true)` で強化した．P3 → §6 の行と test のコメントを「開いた burst」に直した（A は repeat にしていない）．
- どの回も reviewer に mutation は流させていない．テストは checklist の `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review` で流れた．review-5 の「未確認」（mutation，20 回，負荷）のうち mutation と 20 回はこの回に流した．
