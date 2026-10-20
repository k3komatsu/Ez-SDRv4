# 1b-4d-2：実装役の報告

## 実装


# Phase 8 段階 1b-4d-2 実装報告

対象：`/home/komatsu/local_work/Ez-SDRv4`，HEAD `273c120`（作業ツリーは未 commit）．読んだもの：`adjudication.md`，`sol-plan-report.md`，`test-plan.md`，`plan-brief.md`．

## 1. 計画 v2

`~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d2/test-plan-v2.md` を書いた．裁定 §2 の 1〜13 を反映し，S11・S12・S13・S16 の書き直し，TX の隔離 T1〜T4，configure 中の loss L1/L2，start の途中の失敗 P1，off/空の on/halt O1/O2，mutation の対応表，H2/H3，見積もりを入れた．実装中に確定した差（S5 の再 join の open，S16a の untimed stop 2 回，S18 の格子丸め，T1 の reopen なし，S12c/S12a の形）は v2 の §2 と §7 に追記した．

## 2. 本体（`crates/ezsdr-radio-uhd/src`）

- `provider/mod.rs`
  - `prepare` が方向の全 stream を回り，stream `k` の channel set・rate・SampleClock（`<id>/rx/s<k>`，`<id>/tx/s<k>`）を宣言する（`:467-584`）．
  - link を port `rx<k>` ごとに `links[k]` へ振り分ける（`:520-545`）．
  - `arm` が各受信 stream（channel があり link のあるもの）と各送信 stream の streamer を開き，送信 stream ごとに clock を lattice に登録する（`:586-625`）．
  - `start` が各受信 stream の `rx_start(k, T0)` を出し，`uhd-rx-s<k>`・`uhd-tx-s<k>`・`uhd-control` を立てる（`:627-722`）．失敗時は `stop(Abort)` で開始済み owner を join してから `Rejected` を返す．
  - `stop` は `uhd-control` → `uhd-tx-s<k>`（k 順）→ `uhd-rx-s<k>`（k 順）に join する（`:724-761`）．`detached` は `String`．
- `provider/core.rs`
  - `Streams::with_streams(rx, tx)` と stream ごとの `StreamState`（`:130-150`）．
  - `links: Vec<Vec<Arc<dyn DataLink>>>` と `links(k)`（`:570-582`）．
  - 受信 stream ごとの `RX_OVERFLOW` handle（`:575-590`）．
  - `register_in(dir, k, …)` と `Streams::reconcile` の stream ごとの clock 登録（`:400-420`, `:876-884`）．
- `provider/control.rs`
  - `Start { rx: Vec<Option<…>>, tx: Vec<Option<…>> }` と stream ごとの line・packet 長（`:225-300`）．
  - `Stop` を宛先 stream（`<id>/rx/s<k>`，`<id>/tx/s<k>`）へ，`<id>` は全 stream へ（`:385-425`）．
  - `start_rx` を宛先 stream へ（`:430-465`）．
  - `channel_live` がその channel を持つ stream の line を見る（`:531-546`）．
  - `targets` が方向の全 stream の planned を見る（`:547-570`）．
  - `book_cold` の move 拒否は **coerce の後**に判定し（`:834-866`），from-none は宛先 `k` の streamer を開く（`:942-1040`）．
  - `book_burst` は宛先 stream の clock・plan・`to_tx[k]`・`TIME_ERROR` の node を使う（`:1044-1185`）．
  - `rx_packet_samples(k)`（`:268`）．
  - `await_reading` は宛先 stream の `reading` を見る（`:766-785`）．
- `provider/mode.rs`：`settings(config, dir, k, chan)`，`channel_in_later_stream` を削除（`:229-240`, `:253`）．
- `provider/rx.rs`：自 stream の link に publish（`:573`），自 stream の overflow handle（`:545`），configure 後の loss 検査（`:303-307`）．
- `provider/tx.rs`：configure 後の loss 検査（`:721-723`）．`do_switch` の clock 採用を loss で止める．
- `provider/test_support.rs`：`rig_on_mode_links`・`rig_with_links_on`，`HoldApply`，`ReportFeed::reads`，pairs に s2/s3 を追加．

行数（`git diff --numstat`）：src +691 / −242（うち unit テストの追加を含む），e2e テスト `tests/fake.rs` +911 / −57．合計 +1,602 / −299．

## 3. テスト（`crates/ezsdr-radio-uhd`）

### 足したテスト

e2e（`tests/fake.rs`）：`ur_12_a_layout_declares_and_starts_every_stream`，`ur_13_a_linkless_receive_stream_has_no_streamer`，`ur_25_a_channel_enabled_from_no_stream_in_a_later_stream_opens_and_starts_it`，`ur_25_a_move_between_streams_is_refused_and_changes_nothing`，`ur_25_a_channel_leaving_a_later_stream_cuts_its_stream_alone`，`ur_25_a_rate_change_of_a_later_stream_cuts_that_stream_alone`，`ur_26_a_stop_of_a_later_stream_stops_that_stream_alone`，`ur_25_a_refused_change_of_a_later_stream_halts_that_stream_alone`，`ur_24_a_timed_update_acts_on_the_stream_that_holds_its_channel`，`ur_21_a_burst_targets_its_own_transmit_stream`，`ur_26_two_receive_streams_end_at_their_own_stops`，`ur_26_every_stream_s_threads_are_joined_in_order`，`ur_16_a_wedged_later_stream_leaves_the_others_joined`，`ur_16_a_panicking_later_stream_ends_the_run`，`ur_26_a_receive_start_queued_by_the_stop_is_issued_and_ends_at_its_origin`，`ur_25_a_receive_cut_counts_its_own_stream_s_packet_size`，`ur_26_a_start_rx_of_a_channel_less_linked_stream_starts_nothing`，`ur_25_a_transmit_rate_change_of_a_later_stream_cuts_that_stream_alone`，`ur_26_a_stop_of_a_later_transmit_stream_ends_only_its_burst`，`ur_26_a_stopped_later_stream_waits_for_start_rx`，`ur_25_a_successful_change_clears_a_later_stream_s_halt`，`ur_15_a_failed_later_start_closes_after_the_owners_joined`．

unit：`ur_24_the_command_queue_is_counted_per_instance_across_streams`，`ur_24_a_timed_update_reaches_a_later_stream_s_pending_segment`，`ur_26_an_end_takes_delivered_from_each_segment_s_own_stream`，`ur_26_a_stream_s_delivered_is_cleared_alone`，`ur_19_a_later_receive_stream_s_overflow_names_its_node`，`ur_29_a_loss_after_a_configure_starts_no_receive_segment`，`ur_28_a_later_stream_s_reopen_discards_only_its_correlations`，`ur_25_a_transmit_report_read_of_a_later_stream_does_not_hold_the_earlier_owner`，`ur_29_a_loss_after_a_configure_adopts_no_transmit_clock`，`ur_26_a_transmit_switch_at_a_stop_keeps_its_clock_and_sends_nothing`，`ur_15_every_declared_stream_starts_its_own_thread`，`ur_15_the_current_keys_name_their_one_stream_s_threads`，`ur_26_the_control_thread_is_joined_before_every_stream`．

### 直したテスト

- `ur_16_a_panicking_thread_is_a_lost_device`，`ur_16_a_wedged_thread_does_not_wedge_stop`，`ur_16_a_panicking_thread_ends_a_two_stream_run_with_its_message`，`ur_26_the_sending_threads_join_before_the_receiving_ones`：thread 名を `-s0` に（`tests/fake.rs:1017,1039,3800,3927`）．
- `ur_12_a_layout_stream_above_zero_is_refused` → S1 に，`ur_25_a_membership_in_a_later_stream_is_refused_and_changes_nothing` → S4 に置き換え．
- `ur_15_a_second_stream_is_refused_rather_than_half_started` を削除（`provider/mod.rs`）．
- `Gated` に `only`・`entered_n` を追加（S13/S14）．
- `Direct` の pairs に `usrp/rx/s2`・`usrp/tx/s2` などを追加．

### 足した・直した mutation の行

`plan/maintenance/tools/mutations.json`：P280・P308・P309 を削除．P313 の `equivalent` を外し，filter を `ur_25_a_rate_change_of_a_later_stream_cuts_that_stream_alone` に．P344・P345 は維持（`old` は未変更）．動いた既存の行を当て直した：J11, P242, P245, P263, P264, P273, P274, P275, P276, P303, P304, P305, P306, P314, P323, P325, P326, P336．新規 P436〜P475（40 行）．`plan/phase7/tools/mutations.json` の U26・B15・B22 も当て直した．

各行の `old` はツリー内でちょうど 1 回一致（`rules.py` と機械チェックの両方）．P280/P308/P309 は削除．P344/P345 は変更なしで一致．

`renames.txt`：

```text
ur_12_a_layout_stream_above_zero_is_refused -> ur_12_a_layout_declares_and_starts_every_stream
ur_25_a_membership_in_a_later_stream_is_refused_and_changes_nothing -> ur_25_a_move_between_streams_is_refused_and_changes_nothing
ur_15_a_second_stream_is_refused_rather_than_half_started -> -
```

## 4. design/18-uhd-radio.md（DA-1）

- §3 の thread 図と 1 文を `uhd-rx-s<k>`／`uhd-tx-s<k>` に．
- UR-12：later stream の拒否を削除，link は `rx<k>`，各 stream の configure と SampleClock，見出しを `Configuring a stream` に．
- UR-13：各受信 stream の streamer，各送信 stream の clock．
- UR-15：`rx_start(k, T0)`，declared stream ごとの thread 名，current keys は `s0`．
- UR-16：panic・join を stream 名で，detach が 1 本でもあれば device-wide の `close_streams` を呼ばない．
- UR-17：`rx_recv(k, …)`，source，その port の link，`halts its stream`．
- UR-20：受信 stream ごとの handle．
- UR-21：宛先 stream の最後の送信 clock．
- UR-24：判断 line は channel を持つ stream，全 stream の planned，halt はその stream．
- UR-25：cut の ready は `rx_packet_samples(k)`，move は条件と結果だけを規範に（段階は Changes へ），stop の見直しの文は書かない．
- UR-26：join は `uhd-control` → `uhd-tx-s<k>` → `uhd-rx-s<k>`，stream node の Stop はその stream だけ（受信は cut，送信は burst を終え clock は残す），detach 時の close．
- UR-28：`<id>/tx/s<k>`．
- §5 の U12・U14，§6 の行と Changes の 2026-10-20 行．`design/18:205` の「stream guard」を削除，`ur_16_a_wedged_thread…` の thread 名と with_rx_stall の文も直した．

`plan/phase8/reviews/1b4d1c-test-plan.md:384` の壊れた相対リンク（`../plan/phase8/33-…`）を `../33-…` に直した．`rules.py` がこれを指摘したため（計画外の 1 行，報告の「穴」参照）．

## 5. 確かめたこと

1. `cargo test --workspace --no-fail-fast`：**pass**（全 target，0 failed）．`ezsdr-radio-uhd` は lib 210・`--test fake` 248．
2. `cargo clippy --workspace --all-targets -- -D warnings`：**pass**．
3. `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD --renames …/renames.txt`：**0 problems**．
4. mutation：`remote_mutate.py maintenance,phase7 sim02:8,sim03:8 <59 IDs>`，**59/59 killed**（COMPILE ERROR なし）．
   - 新規：P436 P437 P438 P439 P440 P441 P442 P443 P444 P445 P446 P447 P448 P449 P450 P451 P452 P453 P454 P455 P456 P457 P458 P459 P460 P461 P462 P463 P464 P465 P466 P467 P468 P469 P470 P471 P472 P473 P474 P475 — すべて killed．
   - 当て直し：P313 J11 P242 P245 P263 P264 P273 P274 P275 P276 P303 P304 P305 P306 P314 P323 P325 P326 P336 — すべて killed．
5. `--features uhd` の clippy はこの機械では流していない（orchestrator が image で）．

## 6. セルフレビュー

reviewer は DeepSeek（`opencode-go/deepseek-v4.1-flash#max`），回ごとに新規，read-only．ログ：`review-1.log`，`review-2.log`，`review-3.log`．

- **回 1**：P1 1 件（S12 が `delivered` の書き先の混線と `finish` の clearing を観測しない）．P2：S16b が plan の系列でない，mutation の系列漏れ，UR-12 の test 要約の「stream guard」．P3 多数．
  - 直した：S12c を実際の `Rx::finish` に，S12a を Abort + link の block 和に，S16b を 2 tx stream・stream 1・origin 後・configure gate に，mutation を追加（P461〜P471），design の「stream guard」を削除．P3 の S4 両方向，S3 の各 1 回，S7 の start 数，O2 の 2 clock，U1 の join，S10 の depth，S13 の Drop gate，S1 の publish 行（P475），stale comment/design を直した．
  - 退けた：S15 の Run `Stopped` と s0 の loss cut は既存の `ur_16_a_panicking_thread_ends_a_two_stream_run_with_its_message` と `ur_29_a_loss_ends_every_stream_once` が見る（この e2e は thread 名と DEVICE_LOST を見る）．T4 の「`s1.pass()` が false / `tx_async 1` を呼ばない」は現行コードと違う（`pass` は `reading` を見ず，待つのは control の `await_reading`）ので計画を直した．`rx_stall` は B5 の current keys の stream 0 だけで，その旨を design に書いた．
- **回 2**：P1 なし．P2：S4 の `0→1` が最初の拒否の累積で通る，U14 が「direction in full」のまま．P3 多数．
  - 直した：S4 をケースごとに新しい `Direct` で `1→0`・`0→1` を実行，U14 を stream に，UR-17/UR-23 の「direction」を stream に，§6 の「halted direction」，`book_burst` の comment，mangled comment，indent，S12a の clock，S18 の packet 長，T3 の read 数，move 判定を coerce の後，Changes の §5・UR-29．P2 の S12a/S16a/S16b 用に P472〜P475 を追加．
  - 退けた：S12a の「Orderly の end が delivered で決まる」は，Orderly の cut が `instant.max(delivered)` で delivered ≤ instant なので観測できない．Abort の cut = delivered に変えて link の block 和と比べる形にした（P472 が kill）．
- **回 3**：P1 なし．P2：S12a の delivered（Abort 化で解決），S12a/S16a/S16b の行（P472〜P475 で解決），S15 の loss cut（既存 unit で退けた），S16a の detach 経路（下の穴）．P3：S16b の COMMAND_REJECTED 0（追加），S13 の 20 s bound（`bounded` を追加），S1 の publish 行（P475），design の `(unit)`・§5・UR-29・UR-26 の cuts・with_rx_stall（直した）．
  - 退けた：S16a を「join の 1 s の中で終わる」形にするには，stop の mark と join の間に gate を離す必要があり，e2e からその瞬間を probe できない．test 用 hook は裁定に無いので足さず，detach を許す形（stop の後に release）のままにした．この test は「owner が stop を見直さない」「timed start は出る」「block は出ない」「origin で終わる」を固定する．P473 で re-check を足すと kill する．

## 7. 穴・していないこと

1. **spawn 自体の失敗**：`host.spawn` の失敗を決定的に作れない（thread 作成失敗のみ）．P1 は `start` の失敗時の `stop(Abort)` と cleanup の join を通すが，`spawn` の `Err` 分岐そのものは未固定．テスト用 hook は足していない．
2. **S16a の joined path**：上記の理由で detach のまま．stop の mark 後に gate を離す test hook が要る．
3. **S15 の e2e の loss cut**：s0 の clock record がこの系列では出ない（s1 の panic が先）．unit の `ur_29_a_loss_ends_every_stream_once` と既存の 2 stream Run テストが見る．e2e では s0 の loss cut を assert していない．
4. **stop の窓**：裁定どおり再確認を足していない．S16a が今の挙動（stop 後に timed start が 1 回出る）を固定する．#56 の bench で測る．
5. **`rx_stall`**：B5 用に stream 0 だけ．layout の 2 stream では s0 しか stall しない（test-only，design に明記）．
6. **`plan/phase8/reviews/1b4d1c-test-plan.md` のリンク修正**：計画外の 1 行だが，`rules.py` が壊れたリンクを指摘したため直した．
7. **`--features uhd` の clippy・実機**：流していない．mutation の image も feature なし．

## 8. 起きたこと

- 最初の `cargo test --workspace` で `ezsdr-kernel --test time_model` の TM-6 が `tx.rs` の `ticks_in(at.domain())` を検出して失敗した．`ticks_in(record.root)` に直して pass（TM-6 は自分の domain を値から読むことを禁じる）．
- mutation の初回で P454・P455・P458・P460 が生存/COMPILE ERROR，P464・P472 も生存した．P454 は report が相関を retire していたため `Other` に，P455 は compile 不能な `dir_of` を `booked_stream(Dir::Tx)` に，P458 は start 側の stop を落とす形に，P460 は成功した変更後の timed 更新が live であることを追加，P464 は plan に拒否 segment が残らないことを追加，P472 は S12a を Abort + `rx_latency` に変えて kill した．
- S3 の `apply rx 0` は prepare の行を拾っていた，S5 の再 join は from-none で reopen する，S18 の gap は格子丸めで 2 tick ずれる，S16a の untimed stop は 2 回，P1 の `rx_recv` は fake が log しない，などでテストを直した．これらは計画 v2 §7 に記録した．

## 修正の回 1


# Phase 8 段階 1b-4d-2 修正の回 1（Sol の FAIL 4 件）

対象：`/home/komatsu/local_work/Ez-SDRv4`，HEAD `273c120`（1b-4d-2 は未 commit のまま）．読んだもの：`sol-report.md`，`adjudication.md`，`test-plan-v2.md`，`report.md`，`fix1-brief.md`．計画 `test-plan-v2.md` も今回の形に合わせて直した（S13・S14・S16a・S16b・T4・P1 と mutation の対応表）．commit・push・`git add` はしていない．

## 1. Sol major 1：start の途中の device loss

`crates/ezsdr-radio-uhd/src/provider/mod.rs:683-713`．

- `rx_start` が `lost: true` で失敗したら，`core.device_lost(&error.message)` で loss を公開してから `failed` を立てる（`:693-701`）．`DeviceError.lost` を文字列にして捨てない．
- rx stream の loop の先頭，次の stream の `rx_start` の前に `core.is_lost()` を見る（`:686-689`）．公開済みならその stream を始めず `"UR-15: the device was lost before stream {k} was started"` で拒否する．既存の `UR-15:` の形．
- 失敗経路は既存の `self.stop(StopMode::Abort)` のまま（`:729-734`）．loss 後は `stop_at_cut` が `is_lost` で `rx_stop` を出さず（`rx.rs:609-612`），`cleanup` も `close_streams` を呼ばない（`mod.rs:786-791`）．`mark_lost` が先．
- テスト (a)：`tests/fake.rs:7298` `ur_29_a_lost_later_start_is_published_before_the_owners_are_stopped`．`OnceFails { lost: true }`（`fake.rs:7219` の struct に `lost` を追加）で s0 の owner が最初の `rx_recv` をした後に s1 の `rx_start` を `lost: true` で失敗させる．assert：`DEVICE_LOST` 1 件（message は `rx_start` の `DeviceError.message`），`start` は `Rejected`（`UR-15:`），`mark_lost` は 1 回で `close_streams` より前，`mark_lost` の後ろに `rx_stop` なし，`close_streams` なし．
- テスト (b)：`mod.rs:1000` `ur_15_a_loss_published_by_an_owner_stops_the_next_stream_s_start`（mod unit）．layout 2 rx stream の `Prepared` と，stream 0 の `rx_recv` が `RxRecv::Failed { lost: true }` を返し `mark_lost` で channel を送る wrapper（`LostInReceive`，`:955`）．`start` が stream 0 の owner を spawn した直後の probe `start_stream`（`mod.rs:712`，test build だけ）で loss の公開を待つ．次に進むと `is_lost` の検査が s1 を始めない．assert：`start` は `Rejected`，`rx_start 1` は呼ばれない，`mark_lost` 1 回，`close_streams` 0 回．
- mutation：P476（loss の公開の 3 行を消す），P477（`is_lost` の検査を消す）．各 1 行．両方 killed（§5）．

## 2. Sol major 2：S16 の観測

裁定 §2.1 の期待は変えていない（stop の再確認は足さない）．観測だけを直した．

### RX（unit，joined path）

`crates/ezsdr-radio-uhd/src/provider/rx.rs:841` `ur_26_a_receive_start_queued_by_the_stop_ends_its_owner_without_detaching`．

- `HoldApply`（RX・channel 0・untimed apply）で `Rx::begin` を configure の戻りで止める．stream は channel 0 の line に cold 変更（`ready` は記帳の instant）を 1 つ予約し，plan の segment は origin 1 つ．
- gate の間に `streams.stop = Some((now, Orderly))` と `streams.end(&core, now, false, false)` を記帳し，時刻を origin まで進めてから gate を離す．
- assert：(1) `rx_start 0` が出る（今の挙動），link に block が無い，`rx_stop_untimed` の `cut` と `at` がともに origin（segment は origin で cut，untimed stop は origin の後）．(2) `begin` の戻りを 1 s の bound（`recv_timeout`）で受け，その後 `RxCmd::Shutdown(Orderly)` で `turn()` が `Exit` を返す．thread の join を待つ形で，detach を期待にしていない．
- e2e の `ur_26_a_receive_start_queued_by_the_stop_is_issued_and_ends_at_its_origin` は**残した**．gate を保持したまま `Provider::stop` するので必ず detach 経由になるが，detach や「did not join」を assert しておらず，統合経路（`Direct`，`Provider::stop`，plan，clock record，`rx_stop 1 now` 2 回）を見るテストとして有効と判断した．裁定の joined path は上の unit が固定する．テスト名・assert は変えていないので mutation P473 はそのまま．
- mutation：P479（`begin` の `self.follow()` を落とす．queued start の segment が origin で cut されず，untimed stop も block も変わる）．killed．

### TX（unit）

`crates/ezsdr-radio-uhd/src/provider/tx.rs:2320` の `ur_26_a_transmit_switch_at_a_stop_keeps_its_clock_and_sends_nothing` を直した．

- **起きたこと**：元の test の予約は `ready: 2_000` だった．stop が `e=500` なので `replay` の `ended` 検査（`ready >= end`）が cold 変更を落とし，plan は空になっていた．「segment が origin で終わる」（`timeline.rs:892-895` の `end_at_origin`）は起きておらず，`tx_send` が出ないのは plan が空だからで，guard は存在しなかった．予約の `ready` を実際の記帳 instant（`0`）に直すと，cold 変更は plan に残り，stop が `finish` でその segment を origin で cut する（`planned` の segment cut 0）．record の `ended_at` は origin．
- owner は gate の後に `tx` を返すようにし（`do_switch()` の後），採った clock の上の burst（origin の 2 ms 後，`k = 2_001`：遅延で落ちない位置）を `TxCmd::Burst` で渡し，`tx.pass()` まで進める．assert：`tx.clock` は採る，record の `ended_at` は origin，`pass()` は false，`tx_send 1` 0，`COMMAND_REJECTED` 0．
- mutation：P478（`cut_k` を `i64::MAX` にする＝segment の end を見ずに送る）．killed．

## 3. Sol minor 3：gate の解放と worker の終わり

`crates/ezsdr-radio-uhd/tests/fake.rs`．

- S13 `ur_26_every_stream_s_threads_are_joined_in_order`（`:6831`）：`Release` guard を `entered_n(...)` の**前**に移した（`:6851-6855`）．`stop` の後に sleep していた 100 ms を削除し，`Gated::ends()`（`:6871`）で 4 本の worker の終了通知を待つ（`:6858`）．
- S14 `ur_16_a_wedged_later_stream_leaves_the_others_joined`（`:6869`）：`Release` guard を `entered` の前に置き（`:6881-6884`），sleep を削除して `uhd-rx-s1` の終了通知を待つ．
- 終了通知の仕組み（test-local）：`Gated::watch_end`（`:3911`）が gated call を行った Provider thread の最初の呼び出しで TLS に `End`（`:3844`）を置き，thread の TLS destructor が名前を送る．`ended`（`:3940`）が名前ごとに待つ．sleep の代用ではない．
- T4 `ur_25_a_transmit_report_read_of_a_later_stream_does_not_hold_the_earlier_owner`（`tx.rs:2261`）：`reading=true` を下ろす `Reading` guard を assert の前に置き，明示的な `reading = false` を削除．assert が失敗しても Drop で下ろす．

## 4. Sol minor 4：UR-23 の Stop の宛先

`design/18-uhd-radio.md:245`：burst を終える宛先を「`<id>/tx`（current keys），`<id>/tx/s<k>`（layout），または `<id>`」に戻した．同 file の Changes の 2026-10-20 行にも 1 文で記録した（`:633`）．あわせて自己レビューの P3 に従い，新しい 3 テストを UR-15・UR-26・UR-29 の `*Test:*` 行と §6 の 1b-4d-2 行（`:529`・`:531`）に足した．

## 5. 確かめたこと

1. `cargo test --workspace --no-fail-fast`：**pass**（`ezsdr-radio-uhd` は lib 212・`--test fake` 249，0 failed）．
2. `cargo clippy --workspace --all-targets -- -D warnings`：**pass**．
3. `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD --renames …/renames.txt`：**0 problem(s)**．
4. mutation：`REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:8,sim03:8 P476 P477 P478 P479 P440 P245 P442 P467 P455 P474 P458` → **11/11 killed**（P2 の修正後に再実行した結果）．
   - 新規：P476 killed，P477 killed，P478 killed，P479 killed．
   - 触ったテストの既存行：P440（S1），P245・P442（S13 の join 順），P467（S14），P455（T4），P474（S16b），P458（P1）— すべて killed．
5. 全リストの mutation，`--features uhd` の clippy，実機は流していない（orchestrator の gate と image の仕事）．

## 6. セルフレビュー（1 回，DeepSeek，read-only）

`opencode2 run --standalone -m "opencode-go/deepseek-v4.1-flash#max" --agent plan --auto` を新しい thread で 1 回．チェックリストは `fix1-review-checklist.md`，ログは `fix1-review-1.log`．判定：**P1 なし**，P2 1 件，P3 1 件．両方直した（退けたものはない）．

- **P2（P1a の順序 gate が弱い）**：`OnceFails::rx_start` は `recv0` を最大 2 s 待つが，立たなくても lost エラーを返して続行していた．`fake.rs:7247` で待った後に `recv0` を `assert!` してから返すようにした．これで「s0 の owner が s1 の start より前に始まった」ことを固定する．
- **P3（新しいテストが design/18 の test index にない）**：UR-15・UR-26・UR-29 の `*Test:*` 行と §6 の 1b-4d-2 行に 3 本を足した（§4）．

## 7. 起きたこと

- P1b の最初の実行で `fake: ... Attempting to reconnect output port!`：`UhdRadio::from_binding` が `set_subdev_spec` を呼び，先に開いた streamer を外していた（spec 変更は方向の streamer を切る）．`rx_open` を `from_binding` の後に移し，stream 1 は device channel 1 で開いて直した．
- S16b の最初の実行で plan が空だった（§2 TX の「起きたこと」）．予約の `ready` を記帳 instant に直して，`end_at_origin` が実際に通る形にした．
- S13 は 4 本の join bound（約 4 s）と tx の shutdown の 100 ms で 6.3 s，S16a e2e は detach のため 8 s かかる（どちらも test の watchdog 120 s 内）．
- `--features uhd` はこの機械でビルドしていない．mutation の image も feature なし．

## 8. していないこと・穴

- 全 mutation リスト（maintenance 全体・phase7・phase3〜6），image build，Python の suite，`--features uhd` の clippy，実機・bench は流していない．orchestrator の gate に残す．
- S16a の e2e は detach を許す形のまま（§2 RX）．joined path の「1 s の join の中で終わる」は unit が固定する．
- `start` の途中の **spawn 自体の失敗**（thread 作成失敗）は引き続き決定的に作れず，未固定（前の報告の穴 1）．P1 が見るのは `rx_start` 失敗と cleanup の join．
- 行数（HEAD との差，test と mutation を含む）：`provider/mod.rs` +300/−90，`rx.rs` +147/−2，`tx.rs` +167/−4，`tests/fake.rs` +993/−57，`design/18` +50/−38，`mutations.json` +479/−70．今回の修正分のおよその内訳：mod.rs の `start` と P1b で +105，rx.rs unit で +50，tx.rs の S16b/T4 で +20，fake.rs の P1a・`Gated::ends`・S13/S14 で +105，design で +6，mutation 4 行．
