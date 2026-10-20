# Phase 8 段階 1b-4d-2 テスト計画 v2 — layout の stream 1 以上を stream 0 と同じに動かす

対象：`/home/komatsu/local_work/Ez-SDRv4`，HEAD `ff2f61f`．v1：`test-plan.md`．裁定：`adjudication.md`（v1 と食い違う所は裁定が優先）．Sol の計画レビュー：`sol-plan-report.md`．

v2 は v1 に裁定 §2 の 1〜13 と §3 を適用したもの．v1 から変えたのは，S11・S12・S13・S16 の書き直し，TX の隔離 4 系列，configure 中の loss，start の途中の失敗，off・空の on・halt，mutation の対応表，H2・H3，見積もり．

## 0. 裁定の適用

| 裁定 | 適用 |
|---|---|
| §1 LATE_COMMAND は device node | v1 §0 の 3 のまま．`Control::late_command` は `<id>`． |
| §1 S12 は End の床 | S12 を「2 本の RX owner が実経路で違う進捗を作り，Abort・Orderly の End が line ごとの `delivered` で切る」に直す（§2 S12）． |
| §1 S3 の timed start は owner | v1 のまま（assert の順を `rx_open` → `apply` → `rx_start`）． |
| §1 S16 TX は unit | §2 S16 TX は unit． |
| §1 R19〜R22 | v1 のまま採る（R19・R20・R21・R22）． |
| §1 P280・P308・P309 削除，P313 当て直し，P344・P345 維持 | §5 のとおり． |
| §2 major 1 stop の再確認は取り下げ，S16 は今の挙動を固定 | §2 S16：owner は stop を見直さない．RX は `rx_start` を出し，segment は End で origin に cut，block なし，origin の後で untimed `rx_stop`．TX は clock を採り，segment は origin で終わり `tx_send` なし．design/18 に「owner が見直す」文は書かない． |
| §2 major 2 S16 TX は configure 後を検査 | S16 TX の unit が実際の `do_switch` を通り，switch の configure 後に gate して stop を記帳する（§2 S16 TX）． |
| §2 major 3 LockSpy は integration 内の wrapper | `tests/fake.rs` に test-local の `HoldAfterApply` を置く（RX・channel 1・untimed apply・device から戻った位置）．unit 補助は公開しない． |
| §2 major 4 S12 の書き先の混線 | §2 S12 の e2e で 2 本の RX owner に実経路で違う長さを処理させ，Abort と Orderly の End を別々に記帳する．手で `delivered` を置く unit も残す． |
| §2 major 5 TX の隔離と stream ごとの所有 | §2 の T1〜T4（4 系列）． |
| §2 major 6 S11 の操作と assert | §2 S11：s1 だけ送る区間と s0 を送る区間を分ける． |
| §2 major 7 design/18 | §4 のとおり．`halts the direction` を stream に，UR-23 の宛先に `<id>/tx/s<k>`，UR-16 の cleanup は「1 本でも detach すれば device-wide の `close_streams` を呼ばない」，move の拒否は条件と結果だけを規範にし段階は Changes へ，configure の説明は規範に書かない． |
| §2 minor 8 S13 の control-first | §2 C1（unit）で control を先に join することを固定する．e2e の S13 は 4 owner の順だけ．gate は失敗時にも離し，worker の終わりまで待つ． |
| §2 minor 9 mutation の対応表 | §5．`register(dir,k)` は TX の cold 変更の系列に当てる．S1 は s0・s1 に違う rate．S10 は released と held を混ぜる．join 順の mutant は RX-first． |
| §2 minor 10 H2・H3 | §7． |
| §2 追加 major configure 中の loss | §2 L1（RX）・L2（TX）．owner は configure の後，start／clock 採用の前に `is_lost()` を見る．loss の後に device を呼ばない． |
| §2 追加 major start の途中の失敗 | §2 P1．`start` は `Rejected`，cleanup は立ち上がった owner を join してから `close_streams`．spawn 自体の失敗は同じ 1 本の `?` を通る（§7 の穴 8）． |
| §2 minor off・空の on・halt | §2 O1・O2・O3．off は membership の更新で再開せず `start_rx` で再開する．空の on は R21．halt は成功した cold 変更で解ける． |
| §3 大きさ | 分けない．RX・TX 共通の helper を置く．見積もりは §6． |

## 1. 系列の一覧

| # | テスト名 | 場所 | 何を固定するか |
|---|---|---|---|
| S1 | `ur_12_a_layout_declares_and_starts_every_stream` | fake | prepare・arm・start が stream ごと（clock，streamer，`rx_start`，block，port） |
| S2 | `ur_13_a_linkless_receive_stream_has_no_streamer` | fake | link の無い受信 stream は open も start も持たない |
| S3 | `ur_25_a_channel_enabled_from_no_stream_in_a_later_stream_opens_and_starts_it` | fake | `-1 → s1` の join が s1 を open/configure/start し s0 を切らない |
| S4 | `ur_25_a_move_between_streams_is_refused_and_changes_nothing` | fake | `s0 → s1` の move は `UR-25:` で拒否，何も変えない |
| S5 | `ur_25_a_channel_leaving_a_later_stream_cuts_its_stream_alone` | fake | `s1 → -1` は s1 だけ cut，s0 不変 |
| S6 | `ur_25_a_rate_change_of_a_later_stream_cuts_that_stream_alone` | fake | s1 の rate 変更は s1 だけ cut/configure |
| S7 | `ur_26_a_stop_of_a_later_stream_stops_that_stream_alone` | fake | `<id>/rx/s1` の Stop と `start_rx` は s1 だけ．`<id>` は両方 |
| S8 | `ur_25_a_refused_change_of_a_later_stream_halts_that_stream_alone` | fake | s1 の configure 拒否は s1 だけ halt |
| S9 | `ur_24_a_timed_update_acts_on_the_stream_that_holds_its_channel` | fake | timed 更新の判断 line は channel を持つ stream |
| S10 | `ur_24_the_command_queue_is_counted_per_instance_across_streams` | control unit | queue depth は instance ごと（held と released を混ぜる） |
| S11 | `ur_21_a_burst_targets_its_own_transmit_stream` | fake | burst は自分の tx stream の clock/thread．channel の無い stream は UR-21 |
| S12a | `ur_26_two_receive_streams_end_at_their_own_stops` | fake | 2 本の RX owner がそれぞれの Stop で終わり，片方の Stop が他方を止めない |
| S12b | `ur_26_an_end_takes_delivered_from_each_segment_s_own_stream` | core unit | 手で置いた `delivered` で `Streams::book`/`end` が line ごとに読む |
| S12c | `ur_26_a_stream_s_delivered_is_cleared_alone` | rx unit | stream 1 の `finish` が stream 0 の `delivered` を消さない（実装で足した経路） |
| S13 | `ur_26_every_stream_s_threads_are_joined_in_order` | fake | 4 owner の join 順（tx s0, tx s1, rx s0, rx s1）と detach 時の close なし |
| C1 | `ur_26_the_control_thread_is_joined_before_every_stream` | mod unit | control を先に join する |
| S14 | `ur_16_a_wedged_later_stream_leaves_the_others_joined` | fake | 1 本 detach でも device-wide の close を呼ばない |
| S15 | `ur_16_a_panicking_later_stream_ends_the_run` | fake | panic の thread は `uhd-rx-s1`，DEVICE_LOST 1 回 |
| S16a | `ur_26_a_receive_start_queued_by_the_stop_is_issued_and_ends_at_its_origin` | fake | 今の挙動：stop 後でも出す timed start，segment は origin で cut，block なし，untimed stop |
| S16b | `ur_26_a_transmit_switch_at_a_stop_keeps_its_clock_and_sends_nothing` | tx unit | 今の挙動：clock は採る，segment は origin で終わり `tx_send` なし |
| U1 | `ur_15_every_declared_stream_starts_its_own_thread` | mod unit | 2×2 の thread 名と本数，stop で 5 本 join |
| U2 | `ur_15_the_current_keys_name_their_one_stream_s_threads` | mod unit | 今の key の thread 名 |
| S18 | `ur_25_a_receive_cut_counts_its_own_stream_s_packet_size` | fake | cut の ready はその stream の `rx_packet_samples(k)` |
| R19 | `ur_19_a_later_receive_stream_s_overflow_names_its_node` | rx unit | stream 1 の RX_OVERFLOW は `usrp/rx/s1` |
| R20 | `ur_24_a_timed_update_reaches_a_later_stream_s_pending_segment` | control unit | live でない channel の timed 更新が s1 の pending segment に届く |
| R21 | `ur_26_a_start_rx_of_a_channel_less_linked_stream_starts_nothing` | fake | 空の on は受け取って何もしない（後の join で始まる） |
| T1 | `ur_25_a_transmit_rate_change_of_a_later_stream_cuts_that_stream_alone` | fake | s1 の cold 変更は s1 の clock だけ替え，s0 の burst と clock は続く |
| T2 | `ur_26_a_stop_of_a_later_transmit_stream_ends_only_its_burst` | fake | `Stop(tx/s1)` は s1 の burst だけ終え，clock は残り次の burst を送れる |
| T3 | `ur_28_a_later_stream_s_reopen_discards_only_its_correlations` | tx unit | s1 の reopen の generation で s1 の古い相関だけ捨てる |
| T4 | `ur_25_a_transmit_report_read_of_a_later_stream_does_not_hold_the_earlier_owner` | tx unit | s1 の `reading` 中も s0 の owner は device を呼び続ける |
| L1 | `ur_29_a_loss_after_a_configure_starts_no_receive_segment` | rx unit | configure の後に loss が公開されたら `rx_start` を出さない |
| L2 | `ur_29_a_loss_after_a_configure_adopts_no_transmit_clock` | tx unit | configure の後に loss が公開されたら clock を採らない |
| P1 | `ur_15_a_failed_later_start_closes_after_the_owners_joined` | fake | s0 の `rx_start` の後に s1 が失敗 → `Rejected`，join してから `close_streams` |
| O1 | `ur_26_a_stopped_later_stream_waits_for_start_rx` | fake | off は membership の更新で再開せず `start_rx` で再開する |
| O2 | `ur_25_a_successful_change_clears_a_later_stream_s_halt` | fake | halt は成功した cold 変更で解ける |
| O3 | R21 が「空の on」を兼ねる | fake | 空の on は segment を作らず，後の join で始まる |

## 2. 各系列

共通：既存の `Direct`（`tests/fake.rs`），`device.calls()`，`clocks.sample_clock_records()`，`layout_selector()`，`attached_reading`．manual clock の rig では sleep で競合を作らず `Gate`／probe で同期する．テストは系列ごとに assert 1〜2 個．

### S1

- 進め方：`layout_selector()`（rx/tx とも 2 UBX），`rx.c0.stream=0`・`rx.c1.stream=1`・`tx.c0.stream=0`・`tx.c1.stream=1`，rates は s0=2 MS/s・s1=1 MS/s（違える）．links は `rx0`・`rx1`．`Direct::start`．
- assert：(1) `rx_open 0 [0]`・`rx_open 1 [1]`・`tx_open 0 [0]`・`tx_open 1 [1]` が各 1 回，`rx_start 0 T0` と `rx_start 1 T0` が同じ T0．(2) rx0 の block は 1 channel で domain が `usrp/rx/s0`，rx1 は `usrp/rx/s1`；`sample_clock_records` に `usrp/rx/s1` の origin があり，その ratio は 1 MS/s の decimation（200）．
- 落とす実装：prepare が stream 0 だけ，link を全部 s0 に付ける，`start` が `rx_start(0)` だけ，clock の node が s0 のまま，publish が全 link．

### S2

- links は `rx0` だけ．assert：(1) `rx_open 1`・`rx_start 1` が 0 回，`usrp/rx/s1` の clock record も 0．(2) s0 は S1 と同じで `COMMAND_REJECTED` 0．
- 落とす実装：link の有無を instance 全体で見る，全 stream を開く．

### S3

- c1 を既定（-1）で start，`settle(30 ms)` の後 `rx.c1.stream=1` を `now+40 ms` で記帳，`settle(300 ms)`．
- assert：(1) `rx_open 1 [1]` → `apply rx 1 … at=-` → `rx_start 1 <origin>` の順に各 1．(2) `rx_stop 0` も `apply rx 0` も 0；s0 の clock は終わらず，s1 の origin は変更の instant 以上．
- 落とす実装：move の拒否が `-1 → k` も拒む，from-none が s0 を open する，owner が start しない，s0 を cut．

### S4

- rx と tx の各方向で c0→s0・c1→s1 を走らせ，`c1.stream` を `1 → 0` と `0 → 1` に記帳，`settle(120 ms)`．
- assert：(1) `COMMAND_REJECTED` 1 件（各方向），reason が `UR-25:` で始まり `c1`・`s0`・`s1` を含む．(2) device の呼び出し数と clock record 数が変わらず，`rx_stop`・`tx_send` も増えない．
- 落とす実装：move を許す，`-1 → k` も拒む（S3 が落ちる），coerce の後に予約してから拒否する，move を s0 の line にだけ記帳する．

### S5

- S4 の 2 stream で `rx.c1.stream=-1` を記帳，`settle(300 ms)`．その後 `rx.c1.stream=1` で再 join，`settle(300 ms)`．
- assert：(1) `rx_stop 1 now` 1 回，`rx_stop 0` 0；s1 の clock は cut で終わり，s0 は終わらない；cut の後に `rx_start 1` は増えない．(2) 再 join で `rx_open 1 [1]` と新しい `rx_start 1`，s0 の呼び出しは増えない．
- 落とす実装：leave を s0 に記帳，両 stream を cut，`rx_open 1 []`，再 join で s0 を reopen．

### S6

- S1 の 2 stream で `rx.s1.sample_rate_hz=2e6` を記帳，`settle(300 ms)`．
- assert：(1) `rx_stop 1 now` 1 回，`apply rx 1 rate=2000000 … at=-` 1 回，`rx_open 1` は 0．(2) `apply rx 0`・`rx_stop 0` は 0，s0 の clock は終わらず，s1 は 2 つ目の clock を持つ（前は cut で終わる）．
- 落とす実装：P313（s0 の set で s1 を configure），両 stream を cut，`rx_open 1 []`．

### S7

- 2 stream を走らせ，`stop_at("usrp/rx/s1")` → `settle(120 ms)` → `start_rx_at("usrp/rx/s1", None)` → `settle(200 ms)` → `stop_at("usrp")` → `settle(120 ms)`．
- assert：(1) 最初の Stop で `rx_stop 1 now` 1 回，`rx_stop 0` 0；s1 の clock だけ終わる；`start_rx` の後に `rx_start 1` と 2 つ目の s1 clock．(2) `<id>` の Stop で `rx_stop 0` と `rx_stop 1` が各 1 回，両方の clock が終わる．
- 落とす実装：`k != 0` の guard，`Addressed::Instance` が s0 だけ，`start_rx` が channel の無い stream を開く．

### S8

- S1 の 2 stream で `FakeFault::WrongRate { claimed: 4e6, applied: 3.8e6, nth: 0 }`，`rx.s1.sample_rate_hz=4e6` を記帳，`settle(300 ms)`．
- assert：(1) `COMMAND_REJECTED` 1 件に `UR-12: the device applied 3800000 S/s`；`rx_stop 1` 1 回，新しい s1 clock は出ず，`rx_start 1` も増えない．(2) s0 の clock は終わらず，s0 の link には block が来続ける．
- 落とす実装：拒否で方向全体を halt，拒否された segment を始める，s0 を cut．

### S9

- 2 stream，c0→s0・c1→s1．s0 を S8 と同じ `WrongRate` の rate 変更で halt．その後に c1 の gain 3.0 を `now+20 ms`，node は `usrp/rx/s0` で記帳，`settle(100 ms)`．
- assert：(1) `apply rx 1 … gain=3 … at=<e>` 1 回，`apply rx 0 … gain=3` 0，`COMMAND_QUEUE_FULL` も `LATE_COMMAND` も 0．(2) `applied` の `radio.rx.c1.gain_db` の row が `issued: true` で `note` を持たない．
- 落とす実装：`channel_live` が s0 の line の halted を読む，宛先の node の stream に当てる，untimed で出す．

### S10

- control unit：2 stream の link 付き rig，depth 16（`command_queue: 64`），c0 と c1 の gain を交互に 8 個ずつ遠い instant で記帳し，17 個目を記帳する．その後 1 個を released にして（近い instant で）さらに記帳する．
- assert：(1) 最初の 16 個は held のままで device の `apply` は 0．(2) 17 個目で `COMMAND_QUEUE_FULL { depth: 16 }` が 1 件，`apply` は 0．released と held を混ぜた数え方（合計で数える）を固定する．
- 落とす実装：stream ごとに depth を数える，released を数えない，layout で 1 command を channel 数で数える．

### S11

- (a) tx c0→s0・c1→s1（両方 channel あり），rx は c0→s0 で link 1 本．start の後，s0 の clock でだけ burst を打ち，次に s1 の clock でだけ burst を打つ．assert：`tx_send 0` は最初の区間だけ，`tx_send 1` は次の区間だけ；2 つの tx clock の domain が異なる．遅れた burst（過去の at）の `TIME_ERROR` の source が `usrp/tx/s1`．
- (b) 別の `Direct`：tx c0→-1・c1→s1，`usrp/tx/s0` に burst．assert：`COMMAND_REJECTED` 1 件，kind `tx_burst`，reason が `UR-21:` で `no transmit channel` を含み，`tx_send` 0．
- 落とす実装：`book_burst` が s0 の clock を読む，`to_tx[0]` に渡す，`TIME_ERROR` を stream 0 から出す，channel の無い stream の burst を受ける．

### S12a（e2e）

- 2 rx stream（c0→s0，c1→s1，link 2 本）．T0 で s0 を start．s1 は `-1` のまま起動し，`settle(30 ms)` の後に `rx.c1.stream=1` を `now+50 ms` で記帳して s1 を遅れて始める．`settle(400 ms)`（s0 の delivered > s1 の delivered）．
- まず `stop_at("usrp/rx/s1")` → s1 の owner が終わり，s1 の `delivered` だけが消える．`settle(120 ms)`．
- 次に `Provider::stop(Orderly)`（`Direct::finish` は stop + cleanup）ではなく，`direct.radio.stop(Orderly)` を直接呼び，`timing` の plan を読む．
- assert：(1) plan の rx 2 行の end が異なり，s0 の end は s1 の end より大きい（s1 の Stop が s0 の delivered を消していない）．(2) s1 の前の clock は cut で終わり，s0 の clock は Orderly stop の instant で終わる．
- 落とす実装：`Streams::book`/`end` が s0 の `delivered` を全 line に使う，finish が `delivered` を stream 0 だけ消す・全 stream 消す．

### S12b（core unit）

- 2 stream の layout rig で，2 本の line に `delivered` を rx0 `(0,cfg0,1000)`・rx1 `(0,cfg1,500)` と置き，`streams.end(&core, at, abort=true, false)` と `lost=true` の経路を 1 回ずつ．
- assert：(1) `planned(rx0)` の cut 1000，`planned(rx1)` の cut 500（abort）．(2) orderly（at = sample 2000，rx0 delivered 3000，rx1 500）で rx0 3000・rx1 2000．(3) 4 line に End が 1 つずつ同じ `e`・`seq`．
- 追加：`ur_26_a_stream_s_delivered_is_cleared_alone`（rx unit）で，stream 1 の `Rx::finish` を実際に通り，stream 0 の `delivered` が残ることを固定する．mutation は `rx.rs` の clearing を全 stream に広げる行（P470）．
- 落とす実装：End が s0 の `delivered` を全 line に使う，cold の cut に混ぜる．

### S13

- 2 rx stream（link 2 本）・2 tx stream（channel 2 本）を走らせ，各 tx に repeat の burst．`Gated`（全 `rx_recv`/`tx_send` を止める）で `rx_recv` 2 本・`tx_send` 2 本が入るまで待ち，その後に gate を離し，worker の終わりまで待つ．
- assert：(1) `rejected` の「did not join within 1 s」の thread がこの順で 4 つ：`uhd-tx-s0`, `uhd-tx-s1`, `uhd-rx-s0`, `uhd-rx-s1`．(2) `close_streams` は呼ばれない（detach がある）．
- gate は `entered_n` の待ちの前に `Drop` guard で構え，失敗時にも離す．detach の後は sleep ではなく，`Gated::ends()` の各 worker の終了通知（thread の TLS destructor が名前を送る）を 4 本分待つ．control の順は C1 が見る．

### C1（mod unit）

- `prepared_with(2, 2)` に 5 本の test thread（`uhd-control`, `uhd-tx-s0`, `uhd-tx-s1`, `uhd-rx-s0`, `uhd-rx-s1`）を `host.spawn` で作り，各 thread は test の channel で待つ．`stop(Orderly)` を呼ぶと 5 本とも join せず，`rejected` に 5 行が join 順で入る：control が先頭．assert：`rejected` の thread 名の並びが `[uhd-control, uhd-tx-s0, uhd-tx-s1, uhd-rx-s0, uhd-rx-s1]`．最後に channel を drop して thread を終わらせる．

### S14

- 2 rx stream・1 tx stream を走らせ，`rx_recv` を stream 1 だけ止める test-local wrapper（`Gated { only: Some(1) }`）で `stop` する．
- assert：(1) `rejected` の「did not join」は `uhd-rx-s1` の 1 行だけ．(2) `close_streams` 0 回，`rejected` に `leaked: "streamers"` 1 行．
- gate は S13 と同じ `Drop` guard．detach の後は `Gated::ends()` で `uhd-rx-s1` の終了通知を待つ（sleep なし）．

### S15

- 2 rx stream を走らせ，`rx_recv` を stream 1 だけで panic する test-local wrapper で `run_until_end`．
- assert：(1) `DEVICE_LOST` 1 件，message `a Provider thread panicked`（Run の `Stopped` は既存の `ur_16_a_panicking_thread_ends_a_two_stream_run_with_its_message` が見る）．(2) `rejected` に `{ "thread": "uhd-rx-s1", "reason": "panicked" }`；`mark_lost` が `close_streams` より前；rx-s0 の segment の end は loss の instant．

### S16a（e2e，今の挙動．detach を許す統合確認として残す）

- 2 rx stream（link 2 本）．`Direct::build_on(inner, Arc<HoldAfterApply>, …)`；`HoldAfterApply` は同じ `Arc<FakeDevice>` へ転送し，RX・channel 1・untimed `apply` が device から戻った位置で 1 回だけ gate する（start の後に arm）．s1 の rate を `2e6` に `now+50 ms` で記帳し，gate を待つ．その間に `direct.radio.stop(Orderly)`（rx-s1 は gate の中なので join は 1 s で detach）．gate を離し，`settle(300 ms)`．
- assert：(1) `rx_start 1` が gate の前後で 1 回増え（今の挙動），rx1 の link に block が 1 つも来ない．(2) `rx_stop 1 now` は 2 回（変更が切った前の segment と，origin で終わる新しい segment），`usrp/rx/s1` の新しい SampleClock record は無い（block が無いので登録されない）．configure の `apply rx 1 rate=2000000 … at=-` は出ている．
- 落とす実装：stop を見直して `rx_start` を出さない（今の挙動と違う），segment を cut しない，block を配る．
- この e2e の stop は gate を保持したままなので必ず detach 経由になる．裁定の「join の 1 s の中で終わる」経路は，下の S16a unit が実際の `Rx::begin` を通して固定する（e2e は detach を*期待*にはしていない）．
- **S16a（rx unit，joined path）** `ur_26_a_receive_start_queued_by_the_stop_ends_its_owner_without_detaching`：`HoldApply`（RX・channel 0・untimed apply）で owner の `Rx::begin` を configure の戻りで止める．stop を `streams.stop = Some((now, Orderly))` と `streams.end(&core, now, false, false)` で記帳し，時刻を origin まで進めてから gate を離す．assert：(1) `rx_start 0` が出て，link に block が無く，plan の segment は origin で切れ（`rx_stop_untimed` の `cut` と `at` がともに origin），timed start は出る．(2) `begin` の戻りを 1 s の bound で受け（thread を join），その後 `RxCmd::Shutdown(Orderly)` で `turn()` が `Exit` を返す（detach を期待しない）．

### S16b（tx unit）

- 2 tx stream の rig で `Tx::new(core, 1, …)`．`tx.switch = Some(switch(cut, Some(clock), 1, …))`（origin は stop の instant 以後）．`Provider::stop` と同じ `streams.stop = Some((at, Orderly))` と `streams.end(&core, at, false, false)` を記帳してから `tx.do_switch()`．cold 変更の `ready` は**記帳の instant**（0）にする: これで segment は plan に残り，`finish` が origin で cut する（`timeline.rs:892-895`）．
- assert：(1) `tx.clock.is_some()`（今の挙動：採る），その clock の record の `ended_at` は origin．(2) 採った clock の上の burst（origin から 2 ms 以上の未来）を `TxCmd::Burst` で渡し，`tx.pass()` が false で `tx_send 1` 0，`COMMAND_REJECTED` 0．
- 落とす実装：stop を見直して clock を採らない，segment の end を見ずに burst を送る（P478）．

### U1/U2

- U1：rx 2・tx 2 の layout，channel は空の `Prepared` で `start(None)`，`running.threads` の名前が `["uhd-rx-s0", "uhd-rx-s1", "uhd-tx-s0", "uhd-tx-s1", "uhd-control"]`．`stop(Orderly)` で「did not join」0．
- U2：`prepared_with(1, 1)` で `["uhd-rx-s0", "uhd-tx-s0", "uhd-control"]`．

### S18

- 2 rx stream（c0→s0，c1→s1）で link は `rx1` だけ，`FakeConfig { rx_packet: Some(4_000) }`．start の後（`rx_packet_samples(0)==0`，`(1)==4000`）に `rx.s1.sample_rate_hz=2e6` を `now+300 ms` で記帳，`settle(600 ms)`．
- assert：(1) plan の rx 1 行（s0 は line が無い）の前の segment の end と次の origin の差が 11 400 000〜11 400 400 tick（cut と origin の格子丸めで 2 tick ずれる）．(2) 次の rate が 2e6，`applied` の rate の row が 2 つ．
- 落とす実装：`control.rs` が `rx_packet_samples(0)` を使う，`block_len` だけ，s0 の rate．

### R19

- 2 rx stream の layout rig（`rig_with_links_on(1, …)`）で `Rx::new(core, 1, …)`，`receive(Overflow{false})` の次の block．assert：flags `GAP_BEFORE|RESTARTED`，`RX_OVERFLOW` の source が `usrp/rx/s1`，payload が RM-24 の 17 bytes．k=0 は既存の `ur_19_a_layout_receive_report_names_its_stream` が s0 を固定．
- 落とす実装：`Core.overflow` が 1 つ，prepare が s1 の handle を resolve しない．

### R20

- control unit：2 rx stream の link 付き rig．c1 を `e_join=now+300 ms` で s1 に join．その origin より前に `c1.frequency_hz=2.1e9` を `e=now+200 ms` で timed．
- assert：(1) `apply rx 1 … freq=2100000000 … at=<e>` 1 回（`issued`，`note: "no channel"`）．(2) `apply rx 0` 0，`COMMAND_REJECTED` も `COMMAND_QUEUE_FULL` も 0．
- 落とす実装：`targets` が s0 の planned だけ，channel の無い segment を作る．

### R21（空の on を兼ねる）

- 2 rx stream で c0→s0，c1→-1，link は rx0・rx1．start の後，`start_rx_at("usrp/rx/s1", None)`，`settle(120 ms)`．その後に `rx.c1.stream=1`，`settle(300 ms)`．
- assert：(1) `start_rx` の後も `rx_start 1` 0，s1 の clock 0，`COMMAND_REJECTED` 0．(2) その後の join で `rx_open 1 [1]` と `rx_start 1`．
- 落とす実装：`start_rx` を s0 に記帳，空の channel 列で start，拒否する．

### T1（TX の cold 変更の隔離）

- 2 tx stream を走らせ，各 stream に repeat の burst（s0 と s1，それぞれ自分の clock で 50 ms 先）．`settle(30 ms)` の後 `tx.s1.sample_rate_hz=2e6` を `now+50 ms` で記帳，`settle(300 ms)`．
- assert：(1) `tx_open 1 [1]` は 1 回（同じ set は reopen しない），`apply tx 1 rate=2000000 … at=-` が 1 回，s1 の前の clock は cut で終わり，2 つ目の s1 clock が出る．(2) s0 の clock は終わらず，`tx_send 0` は変更の後も増える（repeat），`tx_open 0` は 0．
- 落とす実装：cold 変更を方向全体の clock に当てる，s0 の burst を止める．

### T2（`Stop(tx/s1)`）

- 2 tx stream に repeat burst（s0・s1）を打ち，`stop_at("usrp/tx/s1")`，`settle(120 ms)`．assert：(1) `tx_send 1` が止まり（Stop 後の増加 0），s1 の clock は record として終わっていない（終わらない）．(2) s0 は送り続け，その後に s1 に新しい burst を打つと `tx_send 1` が再び出る（clock が残っている）．
- 落とす実装：`Stop` が方向の全 tx を止める，s1 の clock を終える，次の burst を受けない．

### T3（reopen の generation）

- tx unit：2 つの `Tx`（k=0,1）を同じ core で作り，それぞれ `remember_burst(...)` で `unacked` を 1 つ持たせる．`core.open_streamer(Dir::Tx, 1, Channels::first(1))` で s1 の generation を進め，s1 の `reports(Wall::ZERO)` を呼ぶ．
- assert：s1 の `unacked` は空，s0 の `unacked` は 1 つのまま．`tx_async 0`/`1` の device 呼び出しは 1 回ずつ（s0 の相関は読まれない）．
- 落とす実装：generation を方向全体で 1 つ，reopen で全 stream の相関を捨てる．

### T4（`reading` の隔離）

- tx unit：2 tx stream の rig で s1 の `reading=true` にし，s1 の from-none enable を別 thread で走らせる（probe `wait_reading` と `reading_done` で同期）．assert：(1) enable は `reading_done` に達しない（wait する）．(2) その間に s0 の `pass()` は `tx_send 0` を出し，`reading` は stream ごと（`pass` 自体は `reading` を見ない）．
- `reading=true` は `Drop` guard で下ろす（assert の失敗でも残さない）．
- 落とす実装：`busy`/`reading` を方向全体で 1 つ．

### L1/L2（configure 中の loss）

- unit：`Rx`/`Tx` の rig に test-local の `HoldApply`（apply の device から戻った位置で gate）．owner の configure を別 thread で走らせ，gate の間に `core.device_lost("test")` を呼び，gate を離す．
- L1：`rx.begin()` の後に `rx_start` が 1 回も出ず，`usrp/rx/s0` の clock も出ない（`begin` は `from` を進める）．
- L2：`tx.do_switch()` の後に `tx.clock.is_none()`，`tx_send` 0．
- 落とす実装：configure の後に `is_lost` を見ない．

### P1（start の途中の失敗）

- **P1（spawn の失敗と cleanup）**：test-local の `OnceFails` wrapper（`rx_start` を stream 1 で 1 回だけ失敗させる）．2 rx stream（link 2 本）で `Direct::armed_on(inner, Arc::new(OnceFails), …)`，`radio.start(t0)` を直接呼ぶ．assert：(1) `Err` の kind が `Rejected` で message が `UR-15:` で始まる．(2) `radio.cleanup()` の後，device の log で `close_streams` が最後の `rx_recv` より後（owner を join してから閉じる），`rejected` に「did not join」0．s0 の `rx_recv` が 1 回以上ある（owner が立ち上がっていた）．
- **P1a（途中の `rx_start` の loss）**：`OnceFails { lost: true }` で s1 の `rx_start` を `lost: true` で失敗させる．assert：(1) `DEVICE_LOST` が 1 件（message は `rx_start` の `DeviceError.message`），`start` は `Rejected`．(2) `cleanup` の後に loss を公開した（`mark_lost` が 1 回）．`mark_lost` の後ろに `rx_stop` が無く，`close_streams` も無い（既存の `assert_marked_lost_before_closed` と同形）．
- **P1b（先行 owner の loss と後続 start の交差）**：mod unit で，layout 2 rx stream の `Prepared` を組み，stream 0 の owner の最初の `rx_recv` が `RxRecv::Failed { lost: true }` を返す wrapper を置く．probe `start_stream`（`start` が stream 0 の owner を spawn した直後）で stream 0 の loss 公開を待ち，次に進ませる．assert：`start` は `Rejected`，`rx_start 1` は呼ばれない，`mark_lost` 1 回，`close_streams` 0 回．
- 落とす実装：失敗時の `stop(Abort)` を落とす（P458）．lost を文字列にして捨てる（P476），`is_lost` の検査を落とす（P477）．

### O1（off）

- 2 rx stream を走らせ，`stop_at("usrp/rx/s1")`，`settle(120 ms)`．その後に `rx.s1.sample_rate_hz=2e6` を記帳，`settle(300 ms)`．assert：(1) 変更の後に `rx_start 1` も新しい s1 clock も無い（off は cold 変更で再開しない）．(2) `start_rx_at("usrp/rx/s1")` の後に `rx_start 1` と新しい clock が出る．
- 落とす実装：cold 変更が off の stream を始める．

### O2（halt）

- S8 の halt の後，`rx.s1.sample_rate_hz` を今度は成功する値（`1e6`）で記帳，`settle(300 ms)`．assert：(1) 新しい `rx_start 1` と新しい s1 clock が出る．(2) s0 は不変．
- 落とす実装：halt が成功した cold 変更で解けない．

## 3. 既存テストの変更と `renames.txt`

| テスト | 場所 | 変更 |
|---|---|---|
| `ur_16_a_panicking_thread_is_a_lost_device` | fake:1009 | `"uhd-rx"` → `"uhd-rx-s0"` |
| `ur_16_a_wedged_thread_does_not_wedge_stop` | fake:1035 | `"uhd-tx"` → `"uhd-tx-s0"` |
| `ur_16_a_panicking_thread_ends_a_two_stream_run_with_its_message` | fake:3784 | `"uhd-rx"` → `"uhd-rx-s0"` |
| `ur_26_the_sending_threads_join_before_the_receiving_ones` | fake:3909 | `["uhd-tx", "uhd-rx"]` → `["uhd-tx-s0", "uhd-rx-s0"]` |
| `ur_12_a_layout_stream_above_zero_is_refused` | fake:4847 | S1 に置き換え（rename） |
| `ur_25_a_membership_in_a_later_stream_is_refused_and_changes_nothing` | fake:5462 | S4 に置き換え（rename） |
| `ur_15_a_second_stream_is_refused_rather_than_half_started` | mod:843 | 削除（U1/U2 が代わり，renames は `-> -`） |
| `prepared_with` | mod:762 | 2×2 の `Prepared` を作れるようにする（helper） |
| `Control::new` を呼ぶ unit | control/tx | `Start { rx: Vec, tx: Vec }` |
| `rig_on_mode` / `Direct` の pairs | unit / fake | `usrp/rx/s2`・`usrp/tx/s2` まで足す（layout の 3 channel 用） |
| `rig_on_mode` の links | unit | stream 0 に置く形を残し，`rig_with_links_on(k, …)` を足す |
| `Gated` | fake | `only: Option<usize>` と `entered_n` を足す |
| `Direct::armed_on` | fake | link を port で per-stream に振り分ける（UR-12 の受理） |

`renames.txt`：

```text
ur_12_a_layout_stream_above_zero_is_refused -> ur_12_a_layout_declares_and_starts_every_stream
ur_25_a_membership_in_a_later_stream_is_refused_and_changes_nothing -> ur_25_a_move_between_streams_is_refused_and_changes_nothing
ur_15_a_second_stream_is_refused_rather_than_half_started -> -
```

## 4. `design/18-uhd-radio.md`（DA-1）

規範だけを書き，段階と解除の予定は Changes へ．行は 1 500 文字未満．

### §3

図の thread を `uhd-rx-s<k>`・`uhd-tx-s<k>` にし，「one per declared stream … idle while its stream holds no channel」．図の後に 1 文：layout は方向ごとに channel 数と同じ stream を宣言し，channel の有無にかかわらず thread を立てる．今の key は方向ごとに 1 本で `uhd-rx-s0`・`uhd-tx-s0`．

### UR-12

- channel の拒否の節を削る．
- link の受理：`rx`（今の key），layout では `rx<k>`（k は方向の stream 数未満）．1 つの port の複数 link はその stream の全 block を受ける．
- 「for each direction whose stream 0 has channels」→「for each stream of each direction whose channel set is not empty」．rate は `radio.<d>.s<k>.sample_rate_hz`．SampleClock は `<id>/rx/s<k>`・`<id>/tx/s<k>`．
- 見出し `Configuring a direction` → `Configuring a stream`．

### UR-13

各受信 stream のうち channel があり port に link のあるものを開き，各送信 stream のうち channel のあるものを開く．最初の open の後の rate 比較は方向ごとのまま．送信 stream ごとに SampleClock を登録する．

### UR-15

受信 stream の streamer が開いていればその stream の first segment を T0 に始め（`rx_start(k, T0)`），layout の宣言した stream ごとに `uhd-rx-s<k>`・`uhd-tx-s<k>` を，今の key では 1 本ずつ立てる．channel の無い stream の owner は idle．

### UR-16

panic はその stream の `uhd-rx-s<k>`／`uhd-tx-s<k>` のものとして `rejected` に記録する．join しない thread はその名前で `rejected` に記録し detach する．**1 本でも detach したら `close_streams`（device 全体）を呼ばず，leak を記録する．**

### UR-17

`rx_recv(k, block_len, 100 ms)`．source は `<id>/rx`／`<id>/rx/s<k>`．publish はその port の全 link．「halts the direction」→「halts the stream」．

### UR-20

layout では受信 stream `k` ごとに `(<id>/rx/s<k>, radio.RX_OVERFLOW)` の handle．

### UR-21

`at` はその宛先 stream の最後に登録された送信 clock の上にあること．

### UR-24

- live でない channel の timed 更新は「the stream holding it」の segments に届く（全 stream の planned を見る）．
- 「a refusal halting the direction」→「a refusal halting the stream that holds it」．
- queue depth は instance ごと（UR-24 のまま）．

### UR-25

- cut の ready は「that stream's `rx_packet_samples(k)`」．
- move の拒否は規範だけ：「A `cold` change whose `radio.<d>.c<n>.stream` names a stream number at or above 0 when the channel already names another at or above 0 is refused with `radio.COMMAND_REJECTED` and a reason beginning `UR-25:`; nothing changes.」段階（1b-4e まで）と解除は Changes へ．
- configure が値だけを変える説明は規範に書かない．
- stop の再確認の文は書かない．

### UR-26

thread の join は `uhd-control` → 各 `uhd-tx-s<k>`（k の順）→ 各 `uhd-rx-s<k>`（k の順），各 1 s．`Stop` の宛先と burst を止める宛先に `<id>/tx/s<k>` を書く．「halts the direction」→「halts the stream」．

### UR-28

`<id>/tx/s<k>`（報告する stream `k`）．

### §6

- `ur_12_a_layout_stream_above_zero_is_refused` の行を S1 の名前に．
- `ur_25_a_membership_in_a_later_stream_is_refused_and_changes_nothing` の行を S4 の名前に．
- 新しい行：S2・S3・S5・S6・S7・S8・S9・S10・S11・S12a/b・S13・C1・S14・S15・S16a/b・U1/U2・S18・R19・R20・R21・T1〜T4・L1/L2・P1・O1/O2．assert の要約を 1〜2 行に．

### Changes

1 行：layout の stream 1 以上を stream 0 と同じに動かす（thread 名，links，node，SampleClock，event handle，`arm`/`start`/`Stop`/`start_rx`/`TxBurst`，timed 更新の判断 line，queue depth は instance ごと，move の拒否は 1b-4e まで，design の UR-12/13/15/16/17/20/21/24/25/26/28/§3/§6）．記録はこの計画と 33 §5．

## 5. mutation の方針

- 新しい規則ごとに 1 行（P436 以降）．`crate` は `ezsdr-radio-uhd`，`test_bin` は unit なら `lib`，e2e なら `fake`．
- **P280・P308・P309 は削除**（guard と `channel_in_later_stream` の拒否が消える）．
- **P313 は `equivalent` を外して S6 に当て直す**．`old` は `book_cold` の channels の行（実装後の文字列）．S6 は s1 の rate 変更で，mutant が s0 の set を configure しようとし fake が reconnect で拒否する．
- **P344・P345 は維持**（owner open の rate 比較）．`old` が動いたら当て直す．
- 新しい行（対応，P436〜P475，修正の回 1 で P476〜P479）：

| 規則 | ファイル | 殺すテスト | mutant の形 |
|---|---|---|---|
| 全 stream を prepare する | mod.rs | S1 | stream の loop を `0..1` に |
| link は port ごと | mod.rs | S2 | `ctx.links` をすべて `links[0]` に |
| `start` は全 stream の `rx_start` | mod.rs | S1 | 最初の stream だけ start |
| thread の名前と本数 | mod.rs | U1/U2 | 名前から `-s{k}` を落とす，1 本だけ spawn |
| join の順（RX-first） | mod.rs | S13 | TX より RX を先に join |
| 受信 stream ごとの `RX_OVERFLOW` handle | core.rs | R19 | `overflow[0]` を全 stream に |
| `register(dir,k)` | core.rs | T1 | `stream_node(.., 0)` のまま |
| `booked_stream` を Stop/start_rx に使わない | control.rs | S7 | `stream_key(Rx,k)` を `booked_stream` に |
| `channel_live` の line | control.rs | S9 | `line(stream_key(dir,0))` に |
| `targets` の planned | control.rs | R20 | s0 の planned だけに |
| move の拒否 | control.rs | S4 | `was >= 0 && is >= 0 && was != is` を落とす |
| burst の clock/宛先 | control.rs | S11 | s0 の clock，`to_tx[0]` に |
| `TIME_ERROR` の source | control.rs | S11 | `stream_node(Tx, 0)` に |
| `rx_packet_samples(k)` | control.rs | S18 | `(0)` に |
| End の `delivered` の stream | core.rs | S12b | `stream_key(Rx, 0)` の delivered を全 line に |
| TX の cold 変更の隔離 | tx.rs | T1 | `do_switch` が自分の stream でなく方向の clock を替える（`register` の k） |
| `Stop(tx/s1)` の宛先 | control.rs | T2 | `to_tx[0]` に送る |
| generation は stream ごと | tx.rs | T3 | `sync_streamer` が方向の generation を見る |
| `reading` は stream ごと | tx.rs | T4 | `pass` が方向の `reading` を見る |
| configure 後の loss（RX） | rx.rs | L1 | `is_lost` の検査を落とす |
| configure 後の loss（TX） | tx.rs | L2 | 同じ |
| 途中の start 失敗の cleanup | mod.rs | P1 | 失敗時の `stop(Abort)` を落とす（cleanup の stop も P458 で落とす） |
| owner は stop を見直さない | rx.rs / tx.rs | S16a/S16b | configure の後に stop を見る（P473/P474） |
| publish は自 stream の link | rx.rs | S1 | `links(0)` に publish（P475） |
| 途中の loss の公開 | mod.rs | P1a | `if error.lost { core.device_lost(&error.message); }` を落とす（P476） |
| 公開済み loss で次の stream を始めない | mod.rs | P1b | `is_lost` の検査を落とす（P477） |
| 終わった clock の burst を送らない | tx.rs | S16b | `cut_k` を `i64::MAX` に（P478） |
| queued start の segment は origin で cut | rx.rs | S16a unit | `begin` の `self.follow()` を落とす（P479） |
| off は cold 変更で再開しない | timeline（既存） | O1 | 既存の `Kind::Cold` の `self.on` を落とす（`ezsdr-radio` の行は既存の gate 外なら追加しない） |
| halt を cold 変更で解く | timeline（既存） | O2 | 既存の `self.halted = item.refused` を `true` に固定 |

## 6. 大きさ

| 部分 | 見積もり |
|---|---|
| `provider/mod.rs` | +220 / −90 |
| `provider/core.rs` | +100 / −40 |
| `provider/control.rs` | +260 / −110 |
| `provider/rx.rs` / `tx.rs` | +90 / −20 |
| `provider/mode.rs` | +15 / −35 |
| `tests/fake.rs`（S1〜S18・R19〜R21・T1/T2・L1/P1/O1/O2） | +900 / −90 |
| unit（S10・S12b・S13 C1・S16b・U1/U2・R19・R20・T3・T4・L1/L2） | +500 |
| mutation の行（約 24） | +50 |
| **合計** | **約 2,000〜2,200 行** |

v1 の 1,600〜1,800 行から，裁定の TX 4 系列・S12 e2e・control-first・loss・部分 start・off/halt で +400 行程度．分けない：helper（`Direct` の pairs，`Gated`，`HoldAfterApply`）を同時に直さないと途中の tree が緑にならない．本体は ~650 行，残りはテストと mutation．

## 7. ハードウェアの前提と穴

### H2・H3

- **H2**：1 本の streamer の recv/send は 1 本の thread から，別の streamer は別の thread からでよい．**VERIFIED**（UHD 4.10 `host/include/uhd/stream.hpp:242-247,391-396` が，別 streamer を別 thread から同時に呼べると明記）．X3xx 実機で 2 本同時に動くことは **INFERRED**（bench BP0）．
- **H3**：command time は motherboard ごとに設定・解除し（`multi_usrp_rfnoc.cpp:964-1015`），queue は RFNoC block ごと（`timed_cmd.dox:94-115`）．UR-24 の「instance で 1 列，depth 16」は **instance ごとに 1 列に数える近似**（plan 33 D9）．S10 はそのモデルの規則を固定する．実機の queue の数と深さは BP0/B8 で測る．**INFERRED**（モデルの近似）．
- H1 は INFERRED のまま（bench BP0）．
- H4（untimed stream command は先の timed の後）は VERIFIED（1b-4d-1c）．S16a の「configure は値だけ」はこの上での INFERRED．

### 穴

1. **stop の見直しと start の間の窓**：裁定で再確認を足さないと決めた．stop の後に timed start が 1 回出る経路は残る（S16a が今の挙動として固定）．実害は #56 の bench 系列で測る（orchestrator が起票）．
2. **TX の clock を採らないときの held burst の行き先**：S16b は採る側を固定する．`Stop`（Provider でない）で同じことが起きたときの記録は 1b-4e．
3. **`Mode::settings` の stream 0 の rate**：v2 で k を取る形に直す（計画の任意項目，規則の追加ではない）．
4. **queue の数（H3）**：instance ごとの近似．bench まで INFERRED．
5. **link の無い受信 stream の `start_rx`**：`configs.insert` が先に走る小さな記録の漏れ（既存）．この段階では変えない．
6. **`prepare` の SampleClock の宣言と `arm` の open の間**：channel があり link が無い stream は clock を宣言するが登録しない（今の stream 0 と同じ）．
7. **`stop` の全体の時間**：join 1 s × thread 数．S13 の 4 本で約 4〜5 s．
8. **spawn 自体の失敗**：`host.spawn` の失敗を決定的に作る方法がない（thread の作成失敗だけ）．P1 は同じ 1 本の `?` の後の cleanup を通すが，`spawn` の `Err` 分岐そのものは未固定．テスト用の hook は裁定に無いので足さない．
9. **実装で確定した差**：S12a は s0 を `Abort` で止め，link0 の block 長の和と plan の end を比べる（`rx_latency` 50 ms で delivered を stop より遅らせる）．S5 の再 join は from-none なので `rx_open 1 [1]` が 2 回目に出る．S16a の untimed stop は前の segment の cut と新しい segment の origin で 2 回．S18 の gap は格子丸めで 11 400 002 tick になる（範囲で見る）．T1 の同じ set の rate 変更は reopen しない（`apply` だけ）．S12c を足した．
10. **S16a の clock record**：block が無いので SampleClock は登録されず，record は出ない．「origin で終わる」は plan の cut と untimed stop で観測する（record では観測できない）．
