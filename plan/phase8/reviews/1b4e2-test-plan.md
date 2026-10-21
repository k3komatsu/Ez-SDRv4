<!-- DeepSeek の初版（test-plan.md）を Luna が裁定 A1〜A10 に合わせて改訂した v2．初版は作業 dir に残した． -->

STATUS: implemented

# 1b-4e-2：改訂テスト計画 — UHD Module の channel move

対象は `plan-brief.md` の D1〜D6・S0〜S11。Sol の指摘はすべて採り、orchestrator の A1〜A10 を優先する。元の `test-plan.md` は記録として残す。本書は改訂計画と実施結果の対応記録であり、実装で増えた状態・失敗経路も追記した。

## 裁定の反映表

| 裁定 | 本計画への反映 |
|---|---|
| A1 | §2.1：片側だけ先に prune しても move の両 item と関係を保持する。source-first / destination-first、release の再計算、両側拒否、両側 prune 後の floor を unpruned `Lines` と比較する。 |
| A2 | §2.2・§2.3・§2.6：owner が旧 segment の処理を終えてからだけ streamer を close する。swap（RX/TX）、有限な B segment の A→B→C、cut 前 sample/burst/tail、拒否後の回復を検証する。未処理の segment は finite cut があっても必要とみなすが、cut が origin 以下の TX intermediary は処理対象から外し、streamer を開かず次の plan segment に進む。move の source configure が失敗したときは、両 line の拒否を記録するまで source の `busy` を保持し、destination が拒否前に open しない。 |
| A3 | §2.11・§2.12：open 後の UR-13 rate check が拒否されたら同じ thread が close し、記録も `Unknown` にする。from-none の失敗後に move しても fake の reconnect を起こさない。 |
| A4 | §2.4・§2.7：wait 解放後に plan の存在・拒否・origin・configuration・stop/loss を再読込する。中断は拒否と別で、RX `from` / TX `at` を進めず、古い segment を開始しない。待機中 TX Stop は held burst を取り消す。from-none の `busy` と owner close を競合させず、control の Action 予約継続も別テストにする。 |
| A5 | §2.6：`Lines::refuse` が両 line を拒否する既存動作を前提に、`changed(from)` と `changed(to)` の両時計を確認する。channel が残る S1 形で両 line の halted/refused を見る。configure 拒否を追加し、RX 回復と TX の旧 clock cut / 新 clock origin 終端を区別する。 |
| A6 | §2.4・§2.7：wait probe 到達後に stop/loss/refusal を投入する。owner close gate 中に次 Action の booking 完了を観測する。固定 sleep を同期根拠にせず、Provider stop の join と wait 単体の出口を分離し、#67 の遠い timed start は使わない。 |
| A7 | §2.5・§2.6・§2.9・§2.11：S1 の旧 call は 2,000 samples / 2 Msps = 1 ms。stream 間の stop/open の全順序は主張せず各 stream の stop→自分の open と source close→destination open を見る。S2 は `rx1` link。`tx_send n=` を channel 数と解釈せず、samples と `transmitted` で channel を識別し、held tail/repeat を確認。S8 は device channel・発行 instant・projection・effective value を確認。S10 は sequence と cut-at-origin を考慮。 |
| A8 | §2.12：UHD 4.10 ソース上は最後の streamer 参照の drop が graph edge を切る点を VERIFIED とするが、X300 の連続サンプルは未確認のまま。UhdDevice の close は open と同じ `control` mutex を使う。native-boundary test で Arc の遅延 drop と close/open 競合を確認する。 |
| A9 | §2.9・§2.12：四つの book_move 拒否は通常 key 経路では到達しない前提を二重 move・拒否後 move・prune 後 move で確認する。no-line の文言は Mock と完全一致の `... from stream s{from} to s{to}`。`effective != e` を防御する場合は Lines の clone 上で仮予約し、失敗時に元を変えない。UR-25 の拒否文と旧テスト名を置換する。 |
| A10 | §2.11・§2.12：UHD の move も `applied` に `{key, claimed, read_back: null, at}` を1行書き、VH-8 は `updates` も比較する。送信 close 前に owner の report read を終える。wait が origin を越すケースと、close で未読 report を失う場合は穴として記録し、範囲を広げない。 |

## 1. 共有前提と検証方式

- 共有時刻規則は `ezsdr_radio::timeline::Lines` が決める。UHD 側は close/open/configure の所有者と順序を決める。
- FakeDevice は同方向・別 stream の同一 channel open を `Attempting to reconnect ... port!` で拒否する。stream 自身の reopen は旧 streamer を置き換える。A8 の native-boundary 根拠は UHD 4.10 の RX/TX streamer destructor、disconnect callback、graph edge 削除（Sol-plan §8 の URL）で、実機の連続 sample は INFERRED / bench 未確認。
- テストは `tests/fake.rs` と `crates/ezsdr-radio/tests/timeline.rs` を中心に置き、probe/gate と channel、event、plan、`StreamState`、device calls で観測する。待ちの正しさは状態到達で同期し、wall-clock sleep を根拠にしない。timeout は watchdog のみ。
- 3 channel RX は UBX 1 + BasicRX 2、TX は UBX 2 を使う。`rx_three()` は c0→s0、c1→s1、c2→s2。`rx_two()` / `tx_two()` は既存 helper を使う。
- S0 test を先に main tree に追加して HEAD で失敗させ、その記録後に A1 timeline 実装を単独でテストし、成功してから UHD 実装に進む。

## 2. 系列とテスト

### 2.1 A1 — `Lines::prune_stream` と未完了 move

`timeline.rs` に per-stream prune watermark を保持する。片側の prune が move の instant `e` を過ぎても、peer の watermark が `e` を過ぎるまでは当該 line の boundary を `e` で止め、move item と record を保持する。両 watermark が `e` を過ぎた時だけ source release を destination floor にし、両 item と move record を prune する。

| テスト | 配置 | 操作と assert | 落とす誤実装 |
|---|---|---|---|
| `rm_34_a_source_first_prune_keeps_move_dependencies_until_both_lines_pass` | `crates/ezsdr-radio/tests/timeline.rs` | source Stop/Start と move を instant e に予約し、unpruned clone を oracle にする。source のみ `prune_stream(e+1)` 後に Start を refuse し、source release と destination move-segment origin の変化が unpruned と一致。move を refuse し両 line halt、destination prune 後に両 item/record が落ち、後続 booking waits the preserved floor. | source だけ prune して move dependency を落とす、Start refusal の release change が destination plan に伝わらない、refuse が片側だけ、両側 prune 後 floor を失う。 |
| `rm_34_a_destination_first_prune_keeps_move_dependencies_until_both_lines_pass` | 同上 | 同じ Stop/Start/move sequence で destination を先に prune し、source-first と同じ release change、destination origin、両側 refusal、最終 floor を確認。 | 片方向だけを保護する実装、`Line::released` が片側 prune 後に move item 不在で panic。 |

### 2.2 S0 — H3 回帰（実装前の失敗と実装後の通過）

| テスト | 配置 | 手順 / assert | 落とす誤実装 |
|---|---|---|---|
| `ur_34_a_join_after_a_leave_closes_the_source_s_streamer_first` | `tests/fake.rs` | RX c1:s1→none→s0。close 1 が `rx_open 0 [0,1]` より先、拒否/event なし、join clock が halt しない。 | leave で streamer を閉じない。 |
| `ur_34_a_transmit_join_after_a_leave_closes_the_source_s_streamer_first` | 同上 | TX の同じ系列。`tx_close 1` before `tx_open 0 [0,1]`、拒否なし、新 clock が origin で ended にならない。 | TX owner が streamer を閉じない。 |
| `ur_34_an_idle_destination_leaves_its_join_to_the_owner_when_another_streamer_holds_the_channel` | 同上 | Stop s0、c1:s1→none→s0、続いて `start_rx`。reservation 時点に s0 open なし、source close の後に owner が1回 open/start、拒否なし。 | from-none が別 stream の streamer release 前に開く。 |

実装前の使い捨て probe は `test-plan.md` §1 と `s0-rx.log`・`s0-tx.log`・`s0-idle.log` に記録した（3本とも `reconnect` 拒否で exit 101）。正式テストは実装後にそれぞれ通過した。idle-destination の正式テストには RM-21 の `start_rx` を加えたため、その最終形は未実装 HEAD では再実行していない。

### 2.3 S1/S2/S3/S4 — RX move、empty destination、stopped source

| テスト | 配置 | 操作と assert | 誤実装 |
|---|---|---|---|
| `ur_34_a_move_cuts_both_streams_and_the_source_reopens_before_the_destination_opens` | fake integration | c0,c1:s0、c2:s1，c1:s0→s1。両 lines 同じ `seq/e`、old segment cut e、source stop→source reopen `[0]`、source close/release→destination reopen `[1,2]`。2,000 samples / 2 Msps の旧 call は 1 ms。比較 clock origin は release + direction call + allowance + lead、plan channels と sequence も確認。stream 間の stop/open 全順序は要求しない。 | wait をせず destination open、片 line のみ予約、sequence が不一致。 |
| `ur_34_a_move_of_the_source_s_only_channel_closes_its_streamer` | 同上 | RX3 の c1:s0→s1、s0 link と s1 link（destination は `rx1`）。s0 に後続 segment なし。stop 0→close 0→open 1、s0 plan は新 segment なし。 | empty source の streamer を残す。 |
| `ur_34_a_move_into_an_empty_stream_opens_it_after_the_source_s_release` | 同上 | c1:s1→s2（s2 初期 channel 無しだが link あり）。reservation で control open なし。source close→s2 open。gate 中も s0 の別 Action booking は完了する。`ready=booking` と source release + lead を plan/clock で確認。 | D3違反、control が wait、または移動先を予約時に open。 |
| `ur_34_a_move_from_a_stopped_stream_closes_its_streamer_for_the_destination` | 同上 | c1 が s0 唯一、s0 を Stop 後に move。idle owner が必要を認識して close 0、dest open は release 後。 | idle を理由に streamer を残す。 |

### 2.4 A2/A4/A6 — owner release、swap/chain、wait の同期と再読込

| テスト | 配置 | 操作と assert | 誤実装 |
|---|---|---|---|
| `ur_34_two_moves_at_one_instant_swap_their_receive_channels_without_deadlock` | fake integration | RX s0{c0}, s1{c1} を同じ e で交換。両 owner が旧 segment を終えてから open。拒否/reconnect なし、plan は s0{c1}, s1{c0}。open 順はどちらでもよく、一方の `rx_close` と他方の同一 stream reopen が相手を解放してよい。 | 旧 streamer を解放せず双方が互いを待つ。 |
| `ur_34_two_moves_at_one_instant_swap_their_transmit_channels_without_deadlock` | 同上 | TX で同じ swap。EOB/report read は close より前、拒否/reconnect なし、両 clock は move plan の channel sets を持つ。各自の close が無くても、先行する相手の release と自分の same-stream reopen で正当に解ける。cut-at-origin の中間 channel set は open しない。 | TX が read を終えず close、循環待ち、または zero-duration set を開く。 |
| `ur_34_a_finite_middle_segment_releases_a_channel_along_a_move_chain` | 同上 | RX c1:s0→s1 at e1。B の `rx_start` 到達後に s1→s2 at e2 を予約し、s1 segment は有限 cut。s0 close→s1 open、s1 の最後の sample end は clock cut、stop→close→s2 open。 | `cut.is_none()` だけで必要判定し、finite B segment を省く／その最後の sample を失う。 |
| `ur_34_a_waited_open_waits_for_a_finite_unprocessed_move_segment` | `core.rs` unit + wait probe | B→C の destination segment を待つ間、B の finite segment はまだ未処理で、B の device set は Unknown。segment 完了を記録するまで wait は抜けず、記録後に destination が続行する。 | `cut.is_none()` だけで判断し、Unknown record を見て finite segment 完了前に destination を進める。 |
| `ur_34_a_waited_open_reloads_a_move_refused_while_it_waits` | control/core unit + gate | Destination owner が `await_release` probe に到達。source open/configure を gate し拒否を記録、gate 解放後 destination は最新 plan を読み直し、refused segment を open/configure/start しない。RX `from` / TX `at` が target を飛ばさない。 | stale `Planned` を実行、中断を拒否として誤記、cursor を進める。 |
| `ur_34_a_waited_open_aborts_when_the_planned_segment_changes` | `core.rs` unit + wait probe | destination waits while another stream records its channel open; then an accepted `Cold` replaces the destination segment at a later origin/rate without refusal. Releasing the source record lets the owner re-read, abort the stale `Planned`, make no open, and release its busy mark. | Segment equality/origin/configuration is not rechecked after wait. |
| `ur_34_a_owner_does_not_open_a_move_plan_removed_before_its_turn` | `core.rs` unit | destination `Planned` を読み取った後、source configure refusal で両 line を halt し、source record に moved channel がない状態で `owner_configure`。latest plan がないため no open。 | `latest` がないとき古い `next` に fallback して拒否済み move を実行。 |
| `ur_34_a_move_destination_waits_for_source_busy_after_channel_release` | `core.rs` unit + wait probe | source は move 後の残 channel だけで既に Open、destination は Unknown、source は move `seq` で busy。source busy だけを理由に destination が待ち、move refusal 後も open しない。 | source が moved channel を物理的に解放すると、configure/refusal 完了前に destination が open する。 |
| `ur_34_a_waited_open_waits_for_same_move_source_configuration` | `core.rs` unit + wait probe | source の同じ move の configure が `busy` の間、既に streamer を閉じた destination owner が待ち続ける。source move を拒否して busy を解除後、destination は中断し、open しない。 | device record がもう channel を持たないことだけを見て source configure 終了前に destination open。 |
| `ur_34_a_waited_open_ends_at_device_loss_without_closing_after_loss` / `ur_34_a_waited_open_ends_at_provider_stop_without_opening` | `core.rs` unit, separate | wait probe 後に loss または `Streams.stop` を記録。owner thread join 後 open なし、busy解除。loss後に closeなし。Provider の bounded join は #67 を避ける通常 origin で別 gate test。 | wait が出口を見ず thread を残す、または lost device を close。 |
| `ur_34_a_device_loss_waits_for_an_admitted_streamer_open_and_close` | `core.rs` unit + `LockSpy` gates | Hold open/close inside the device call; synchronize on loss reaching the contended gate; release the call, then assert it completed before `mark_lost` and no later open/close reaches the lost device. | `DEVICE_LOST` publishes during a streamer call or a later open/close reaches a lost device. |
| `ur_34_a_waited_receive_open_exits_when_stop_removes_its_segment` | `rx.rs` unit | wait probe 到達後、実際に Stop を line へ book。最新 plan に target segment なし、open/start なし、RX `from` 不変、move item は未拒否、`COMMAND_REJECTED` なし。 | 古い `Planned` を開始する、cursor を進める、Stop interruption を拒否と誤記する。 |
| `ur_34_a_transmit_stop_during_release_wait_cancels_later_bursts` | `tx.rs` unit | TX owner が wait probe に到達後、Stop item と queued `TxCmd::Stop` を送る。owner の switch が中断し `at` は target より進まず、command 処理が `later` held burst を取消す。`tx_async` report read は `tx_close` 前。 | wait 中の Stop を見失う／future burst を残す／report を close 前に読まない。 |
| `ur_34_a_transmit_stop_books_the_wait_exit_and_queues_the_owner_stop` | `control.rs` unit | `Stop` Action が TX line に現在時刻の Stop item を記録し、同じ owner queue に `TxCmd::Stop` を送る。 | queued command だけで line に記帳せず、wait が Stop を見落とす。 |
| `ur_34_a_release_closes_an_idle_streamer_needed_by_a_move` | `core.rs` unit | idle source streamer が destination の未処理 move segment に必要なとき、release が `rx_close` し、device record を Unknown にする。 | 必要な source streamer を閉じず destination が reconnect 拒否される。 |
| `ur_34_a_release_keeps_an_idle_streamer_with_no_peer_need` | `core.rs` unit | idle streamer の channel を他 line の未処理 segment が必要としない場合、release は false、close なし、record Open のまま。 | Stop後の不要な close が既存動作を変える。 |
| `ur_25_an_owner_waiting_behind_a_from_none_open_starts_nothing` + `ur_34_a_release_skips_a_stream_held_busy_by_from_none` | unit with `LockSpy` / Core | from-none busy gate 中 owner turn が device call なし。busy + 他 line の pending channel のとき idle release も close しない。 | busy 中に close/open が競合する。 |
| `ur_34_a_control_books_the_next_action_while_an_owner_close_is_gated` | `control.rs` unit + `LockSpy` | source close gate 中に2本目 cold Action を別 control thread で bookingし、gate解放前にdone channelと`colds` keyを確認する。 | uhd-control が owner close を同期 wait。 |

全ての wait test は `Core::probe` / device gate の reached 通知を使う。wall sleep は同期に使わない。timeout は test watchdog のみ。

### 2.5 S5/A7 — TX move、held burst と channel 識別

| テスト | 配置 | 操作と assert | 誤実装 |
|---|---|---|---|
| `ur_34_a_transmit_move_ends_the_old_burst_and_sends_the_new_burst_on_its_stream` | fake integration | c0:s0→s1。10,000-sample old one-shot waveform を e で終わらせる move。異なる waveform を送信し、`transmitted` channel 0 の pre/post value、cut 前の最後の sample が held tail として EOB call に `n=1` で乗ること、EOB-before-`tx_close 0` を assert。`tx_send n=` は sample count と解釈する。Continuous repeat wrap は既存 `ur_22_repeat_is_continuous_across_the_wrap` が固定する。 | burst を誤 stream に送る、move cut で held tail を落とす、channel count と n を混同。 |
| `ur_34_a_transmit_move_carries_a_prebooked_burst_across_the_repeat_cut` | fake integration | c0:s0 repeat burst を e の前に送り、waveform wrap を越える cut を予約する。move で登録された s1 clock に次の burst を cut 前に book し、old clock の cut sample・new clock の burst sample・source close 前の EOB を観測する。 | cut を越えて repeat を送り続ける、又は switch 時に new clock の held burst を落とす。 |

### 2.6 S6/A5 — device 拒否、両 line halt、回復

`Lines::refuse(stream, seq)` が move を両側で refused にする既存規則を使う。新規動作は両 stream の `changed`（特に TX clock reconciliation）。

| テスト | 配置 | assert |
|---|---|---|
| `ur_34_a_move_destination_open_refusal_halts_both_lines_and_recovers` | fake integration (S1 shape) | destination open refusal 1 event、両 plan の `halted(e)` と move item refused を unit で確認。source に channel が残るので次 cold update で source が回復。移動後の所属に沿った destination recovery も確認。 |
| `ur_34_a_move_destination_configure_refusal_halts_both_lines_and_recovers` | 同上 | destination open 成功後の configure 拒否。両側 refusal は core/timeline で確認し、destination の次 cold change でその実所属 membership が再開。 |
| `ur_34_a_move_source_reopen_refusal_halts_both_lines_and_recovers` | 同上 | source の残 channel を reopen する S1 形。source open 拒否時も両 line halted/refused、destination segment start なし。TX の拒否 clock は origin 終端。 |
| `ur_34_a_move_source_configure_refusal_halts_both_lines_and_recovers` | 同上 | source reopen 後の source apply 拒否。source の refusal が両 line に記帳されるまで `busy` が残り、destination は拒否された segment を開始しない。destination の次 cold change で実所属 membership が再開。 |
| `ur_34_a_refused_move_reconciles_both_transmit_clocks` | `core.rs` unit | 旧 clock の end は e、新しい refused clock は origin。両 clock record と `Streams::halted` を確認。 |

誤実装説明は「`refuse` は片側だけ halt」ではなく、「両側 `changed` の片方を呼び忘れると反対側 TX clock の cut/origin 登録が stale」とする。

### 2.7 S7 — loss と Provider stop

| テスト | 配置 | 操作と assert | 誤実装 |
|---|---|---|---|
| `ur_34_a_waited_open_ends_at_device_loss_without_closing_after_loss` | `core.rs` unit + wait probe | destination owner wait 到達後 `device_lost`。wait が戻って join し、`mark_lost` 後は close/open なし。 | loss 後に device を解放、または wait thread を残す。 |
| `ur_34_a_waited_open_ends_at_provider_stop_without_opening` | `core.rs` unit + wait probe | wait 到達後 `Streams.stop` と End を記帳。wait owner が戻って join し、新 open なし。Provider の実 join 順序は既存 orderly-stop tests。 | stop wait が終わらず owner を残す。 |
| `ur_25_a_lost_stream_starts_no_rate_check_apply_or_booking_after_the_call` | `control.rs` unit + Core `open_returned` probe | device open と gate 解放後、control の次の判定前に loss を記録。resume 後は rate-check probe/device call、apply、cold booking がなく、open result の set は保持し busy は解除。 | open 後の loss を見落として rates/configure/booking へ進む。 |
| `ur_29_a_lost_stream_between_the_rate_check_and_the_configure_applies_nothing` | `control.rs` unit + Core `rates_returned` probe | rates call と gate 解放後、control の configure 判定前に loss を記録。resume 後 apply/booking なし、stream record は open result のまま。 | rate check 後の loss を見落として configure へ進む。 |
| `ur_34_a_rate_read_waiting_for_device_gate_does_not_call_after_loss` | `core.rs` unit + Core `rates_before_gate` probe | rates の initial loss check 後、gate 取得前に wait。loss を gate 経由で記録してから resume し、`Device::rates` を呼ばず lost error になる。 | gate 後の second loss check がなく、lost device に rates を呼ぶ。 |
| `ur_29_a_lost_receive_owner_configure_releases_stream_lock_before_reporting` / `ur_29_a_lost_transmit_owner_configure_releases_stream_lock_before_reporting` | RX/TX owner unit + lost `ApplyFails` | untimed owner apply が `DeviceError { lost: true }` を返す。owner は stream record を更新し busy を解除した後で loss を報告し、Run lost になる。 | `device_failed` が同じ `streams` mutex を取り直して owner deadlock。 |

### 2.8 S8 — move を横切る timed update（A7）

`ur_34_a_timed_update_crossing_the_move_reaches_each_streamer_channel_at_its_instant`（fake integration）：同じ channel の update を e 前後に予約する。`apply` は streamer でなく device channel を名指すため、assert は `apply rx 1 ... at=<e>` の issued instant、`applied` の key/claimed/read_back/at と projection。e 前の更新は source projection、e 後は destination projection、queue rejection なし。誤実装では projection または issued/applied instant が逆側・欠落になる。

### 2.9 S9/S10 — 拒否と move 後の floor

| テスト | 配置 | assert | 誤実装 |
|---|---|---|---|
| `ur_34_a_move_to_a_stream_without_a_line_uses_the_mock_reason` | `control.rs` unit | line の無い RX destination s1 に move。`COMMAND_REJECTED` の reason は厳密に `RM-34: no line for move of channel c1 from stream s0 to s1`。device calls 不変。 | Mock と違う文、`book_move` の no-line panic。 |
| `ur_34_a_move_books_one_item_on_both_lines_and_both_configurations` | `control.rs` unit | move の同じ `seq/e` が両 line にあり、source/destination の projection と configs が異なる。`applied`/`timing` を1行ずつ記録し、booking は device を呼ばない。 | 片 line だけ記帳、同じ projection の共有、または logical update の記録漏れ。 |
| `ur_34_a_membership_change_to_its_own_stream_changes_nothing` | fake integration | 同じ membership は no-op、device/clock/event 不変。 | no-op を move refusal にする。 |
| `ur_34_a_second_move_and_a_move_after_refusal_and_prune_keep_projection_aligned` | control unit | s0→s1→s0 moves、first move refusal、両 line prune、さらに s0→s1。`config_at(e)` membership、both items/configs の seq が一致。 | `before` projection と folded `Lines` がずれる。 |
| `ur_34_a_cold_change_before_the_move_s_release_is_floored_at_the_move` | fake integration | move 後、release 前 instant の stream-rate cold を予約。late event requested/applied=e、s1 の新 clock は move floor 以後。既存 first segment を sequence で分け、cut-at-origin RX segment は no clock record も許容。 | `colds` に move を入れず後続 cold が e より前になる。 |

Same-stream、direction mismatch、NotInSource、AlreadyInDestination の `book_move` Err は `rm_34_an_impossible_move_is_refused_without_a_booking`（timeline）で検証。通常 key 経路での到達を主張せず、A9 の projection/Lines 一致を上記の二重・拒否後・prune 後系列で固定する。`effective != e` の防御は Lines clone 上で仮 booking して元の Lines を変更しない。

### 2.10 S11/VH-8（A10）

`crates/ezsdr-acceptance/tests/parity.rs` に次の2本を追加。

| テスト | 操作・assert | 誤実装 |
|---|---|---|
| `vh_8_a_move_between_streams_leaves_the_same_clocks_segments_and_update` | S1 の move。relative clock rows、plan rows、`sequence` の順位、`updates` の `(key,value,at)` を Mock/UHD で比較。 | source/destination clock、sequence order、applied move row の差。 |
| `vh_8_a_move_into_an_empty_stream_leaves_the_same_first_segment_and_update` | S3 の空 stream move。source cut、destination first origin、plan、sequence rank、updates を比較。 | uhd-control が予約時に開く、ready/release の差、applied row 欠落。 |

`sequence` は絶対値ではなく、各 Manifest の distinct non-null sequence を昇順 rank に写して比較する。Mock と差が出た場合は本体を合わせず該当 test に `#[ignore = "divergence: ..."]` を付け、レポートに実際の差を記録する。

### 2.11 A3/A8 — rate-check cleanup と native boundary

| テスト | 配置 | 操作と assert | 誤実装 |
|---|---|---|---|
| `ur_13_arm_refuses_a_pre_chain_rate_set_after_the_open` | `tests/fake.rs` | RX の owner/arm open 成功後に rate-set mismatch。`rx_close 0` が `close_streams` より前。 | open_streamer の rate check 後に streamer を物理的に閉じない。 |
| `ur_34_a_rate_check_refusal_closes_the_streamer_before_a_later_move` | `tests/fake.rs` | RX が arm 時に空で未比較の layout。from-none open 成功後に rates wrapper が rate set を拒否。`rx_close 0` が拒否 event/後続 move より前、record `Unknown`/configured none、後続 channel move で `reconnect` なし。 | from-none rate check 拒否後に物理 streamer を残し Unknown と不一致。 |
| `ur_34_a_rate_refusal_leaves_the_stream_record_unknown` | `core.rs` unit | successful open 後の UR-13 refusal を渡し、`StreamState.set == Unknown` / `configured == None` と close-after-open を assert。 | device close はしても StreamState を `Open` のまま残す。 |
| `uhd_close_drops_its_map_reference_but_an_in_flight_arc_keeps_the_streamer` | `uhd.rs` ffi unit | map に入れた streamer の Arc を call 側で保持して `rx_close`。map から消えるが strong ref が残る間 drop/free は行われず、call Arc drop 後にだけ stream destruct が実行される。 |
| `uhd_stream_close_and_open_compete_under_the_control_mutex` | `uhd.rs` ffi unit | native boundary の open と close/open の競合を並行実行し、stream map が1 entry以下に保たれ、その後の open→close が確実に追加/削除する。両 device call は同一 `control` mutex の直列領域。 |

UHD 4.10 の rate-set mismatch は既存 `RatesMisreported` wrapper を再利用する。lost の device は close しない。

### 2.12 D1〜D6/A1〜A10 の実装箇所・追加経路

| 決定 | 実装箇所 | 新しい状態/出口と対応テスト |
|---|---|---|
| D1/A2 | `provider/core.rs` Streams の held/needed/owner判定、`active`/`completed`、move 対応する busy owner の待ち、`release_unneeded`; `provider/rx.rs::begin/finish`; `provider/tx.rs::plan_section/pass/do_switch/end_open` | owner が処理中・処理済みの segment を記録。finite cut でも未完了 segment は必要だが、cut-at-origin の TX segment は開かず次の segment へ進む。physical set が move channel を含まなくなっても、source の同じ move configure が終わるまで destination を待つ。source refusal を両 line に記帳するまで move の `busy` を保持し、cut前のsample/burst/tailを終えてからclose。streamer open/close/rates read と loss publication を直列化。S0–S5、swap、chain。 |
| D2/A4/A6 | `provider/core.rs::owner_configure` と `await_release`; RX `begin`; TX `do_switch` | 待ち→loss/provider stop/RX segment消失/refusal/TX Stop の中断。wait後 latest plan照合。abort は refusal でない。device error 後は caller が refusal を記帳してから owner `busy` を下ろす。probe/gate unit と S7。 |
| D3 | `provider/control.rs::book_cold` from-none 条件 | other stream record が channel set を持つ場合は owner に委譲。S0c/S3。 |
| D4/A8 | `device.rs::Device`, FakeDevice; `uhd.rs::UhdDevice`; 全 trait impl | `rx_close`/`tx_close` は既定実装なし。close の呼び出し streamに streamer 無しも成功。Mutex直列化とArc遅延 drop。 |
| D5/A5 | `core.rs::Streams::refuse/changed/reconcile`; RX/TX open/configure failure callers | move `seq` は両 line refused、both TX clocks recalc。移動先/元 open・configure拒否。 |
| D6/A9/A10 | `control.rs::book_cold`; `core.rs::Streams`; `timeline.rs::Lines` | move 仮予約・両 configs/colds/timing/applied、拒否理由。S1/S9/S10/S11。 |
| A1 | `crates/ezsdr-radio/src/timeline.rs::Lines::prune_stream` | prune watermark、両 line 関係保持、同時 prune 完了時に destination floor。2 timeline tests。 |
| A3 | `core.rs::open_streamer`; `control.rs` from-none open/check | rate-check mismatch 後に同じ thread で close、record Unknown。rate check test。 |
| A7 | fake/parity assertions | corrected channel/sample/time/sequence observations in §§2.3, 2.5, 2.8, 2.9, 2.10. |

### 2.13 Design と mutation 更新

- `design/07-radio-model.md` RM-34：`Lines::prune_stream(stream,before)` を含め、片 line の prune は未完了 move item を残す、両 line の prune が move instant を過ぎた時に record と両 item を fold して source release を destination floor に残す。短い normative 文と `*Test:*` を追加し、理由は Changes に記録。
- `design/18-uhd-radio.md`：UR-25 に move booking、owner release / conflict close / wait / latest-plan read / busy / stop と report read 条件を記述。UR-25 の古い「move refused」文と旧 `ur_25_a_move_between_streams_is_refused_and_changes_nothing` 名を削除。UR-30 timing に move の instant/from/to/channel、UR-33 に per-stream close、UR-36 に rate-check refusal 後 close を加える。各規則の test names と Changes 行を更新。
- `plan/phase8/33-channels-and-streams.md`：§5 の 1b-4e-2 行を実装結果に更新し、§6 test list と計画/レビュー/裁定の参照を加える。
- `mutations.json` maintenance は P489 から追記。少なくとも close 判定、wait 条件、wait exit、latest-plan re-read/stale-plan abort、from-none exclusion、move dual booking、両 TX clock recalc、per-stream prune boundary、rate-check refusal close と record、source configure/refusal 中の peer wait、owner lost-error unlock、cut-at-origin TX segment skip、zero-duration intermediary の非必要判定、TX held-tail flush/report-before-close/prebooked-burst adoption、move `ready=booked_at`、loss/open-close/rate-read serialization、A6 owner-close 中 booking、`applied` move row に各1行以上。§3.7 の既存 P252/P253/P385/P386/P278 等、変更した `old` 行を検索して当て直し、更新済み行の `new` に旧識別子が残らないことも grep。`uhd_stream_close_and_open_compete_under_the_control_mutex` は `features=uhd` のみで動き、remote mutation image に libuhd がないため、feature test のみで確認する。ファイル全体は書き直さない。

## 3. 実装順と確認

1. この計画を確定。S0 正式 test 3本を先行追加し、現 HEAD でそれぞれ落ちることを確認してログ保存。
2. A1 の timeline 実装と2本のテストを UHD より先に完了し、focused test を通した。
3. Device trait close と全実装、UhdDevice mutex/Arc、FakeDevice call log を追加した。
4. move booking/applied/timing、owner release/wait/re-read、両側 refusal clock recalc、rate-check cleanup を実装し、増えた状態と出口を本計画に追記した。
5. S-series, A-series, VH-8, design, mutation rows を追加した。
6. `timeout 300 cargo test` を test binary ごとに使う。新規 `ur_34_*` と parity は連続3回。full workspace test/clippy、UHD feature container check、`rules.py --base HEAD` は最終 gate の結果を追記する。
7. mutation は `remote_mutate.py` のみ。新規・更新行の IDs を個別指定し、COMPILE ERROR は survivor 扱い、BASELINE FAILED は該当行を再実行して結果を追記する。

## 4. 範囲と穴

- moves の Python API / Session verb / 実機 bench は今回変更しない。
- move の `effective != e` は仮 Lines clone で拒否するが、到達する入力は確認できていない。
- `changed/reconcile` の i64 overflow は Fake で作れず、専用テストは追加しない。
- A10 の wait が origin を越えるケースと、TX close により未読 report が失われる場合は未確認として報告する。UHD 4.10 source で destructor disconnect は VERIFIED、X300 の別-stream move 後の連続 sample は bench まで INFERRED。
- `Device::rx_close/tx_close` は `Result` を持たないため close failure path は存在しない。lost device では close しない。
- 実装＋test は約1,500行を目安に記録する。超える場合は report に増分と理由を示す。`cargo fmt`/`rustfmt`、Python bytecode、git add/commit/push は行わない。
