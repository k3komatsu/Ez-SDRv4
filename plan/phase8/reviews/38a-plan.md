STATUS: done

# 設計メモ 38 の段階 38a：テスト計画（#62）

HEAD は `7a3b553`。tree は読むだけで，コード・spec・fixture は変えていない。テストも走らせていない（38a は計画だけ）。

前提は設計メモ `plan/phase8/38-timed-release-order.md` §2 の D1・D2 と §4 の系列である。device lead 2 ms，release window 3 ms，UHD の root は 200 MHz（1 tick 5 ns），Mock の root は 1 GHz（1 tick 1 ns），Mock の T0 は x3xx の start（2 s，`mock_layout.rs` の `T0`）である。時刻はすべて受け取り `r` を固定して書く。UHD は `ManualTimeAuthority` と `Frozen`（`crates/ezsdr-radio-uhd/src/provider/test_support.rs:143`）で試験のスレッドが時計を動かし，Mock は `Harness::step` で `now` を固定する。どちらも試験のスレッドの遅れは判定に入らない。

床（floor）の規則は新しい語彙の関数 1 つに置く。案：

```rust
// ezsdr_radio::timeline, layout_instant の隣
pub fn timed_instant(at: Option<i64>, now: i64, lead: i64, window: i64,
                     earlier: impl IntoIterator<Item = i64>) -> (i64, bool)
// floor = max(now + lead, max{ e | e は earlier にあり e <= now + window })
// e = max(at.unwrap_or(floor), floor)，late = at.is_some_and(|a| a < floor)
```

`earlier` は「それより前に予約された timed update の `e`」で，渡したか・channel 待ちか・装置が拒否したかは問わない。`lead` は各 Module の device lead（UHD `DEVICE_LEAD_NS` 2 ms，Mock `profile.timing().min_timed_command_lead_ns` 2 ms）。UHD の宣言値 5 ms（lead + delivery allowance）は床の lead ではない。

## 1. 系列の表

列の略号：`L` = `radio.LATE_COMMAND` の payload（`key`，`requested`，`applied`），`Q` = `radio.COMMAND_QUEUE_FULL` の payload（`key`，`depth`），`rows` = `applied` の行（key，claimed，at，issued/no channel/cancelled）。UHD の `rows` は `lock(&core.rec).applied`，Mock は `ezsdr.radio.mock.applied` を読む。

### 1.1 系列 1–6

| 系列 | どこで | テスト | 場面の作り方 | assert | 観測 | 落とす誤り |
|---|---|---|---|---|---|---|
| 1. A(e=10 ms) を 0 ms，B(9.5) を 7 ms。3 ms ちょうど | MockRadio `tests/mock_radio.rs`（x3xx。ideal は D1 の「今と同じ」で同じ test を 0 の期待値で） | `mr_18_a_timed_update_behind_one_inside_the_window_is_late`（新規） | `Harness::new("x3xx", …)`，`arm_start(T0)`。A `radio.rx.c0.gain_db=1.0`，`at=T0+10 ms` を push して `step(T0)`。B `=2.0`，`at=T0+9.5 ms` を push して `step(T0+7 ms)`。`step(T0+10 ms)` | `L` が 1 件で `key="radio.rx.c0.gain_db"`，`requested=T0+9.5 ms`，`applied=T0+10 ms`。`applied` section が A(1.0, T0+10)，B(2.0, T0+10) の順 | `harness.events.drain(TimePoint::new(HOST, 0))` の `L` と payload，`ezsdr.radio.mock.applied` | 床を `now+lead` だけにする（B が 9.5，L なし）。`e <= now+window` を `e < now+window` にする（3 ms ちょうどを外す）。window を読まない（0 にする） |
| 1. 同上 | UHD `control.rs` unit，`manual_rig_events()`＋`hold()` | `ur_24_a_booked_command_floors_a_later_one_inside_the_window`（新規） | `time.advance_to(core.at(0))`，`book_timed(A, at 10 ms)`。`advance_to(7 ms)`，`book_timed(B, at 9.5 ms)`，その後に `release()` | `L` 1 件で `key="radio.rx.c0.gain_db"`，requested 9.5，applied 10。`held` が A `(10,seqA)`，B `(10,seqB)`。`device.calls()` の `apply … at=10` が seq 順。`advance_to(10 ms)`＋`release()` で rows が claimed 1.0→2.0 の 2 行 | `events_of(&events, LATE_COMMAND)`，`control.held`，`device.calls()`，`rows_of` | 床を `released`（渡したもの）だけから取る（B の予約は release の前なので A はまだ `released` に無く，B=9.5，L なし）。`<` にする（L なし）。`at=None` だけ `now+lead` にする（下の 1b） |
| 1b. `at` なしが床を取る（D2 (1)） | MockRadio（x3xx） | `mr_18_an_omitted_deadline_takes_the_floor`（新規） | A `at=T0+10`。B は `at=None` を T0+7 に push。 | B の行が T0+10（`T0+7+2` でない）。`L` なし | applied | `at=None` を `now+lead` にする（B が T0+9） |
| 1b. 同上 | UHD（manual_rig_events） | `ur_24_an_omitted_deadline_takes_the_floor`（新規） | A `at=10 ms`，B は at なしを 7 ms に book，`release()` | B の行が 10 ms。`L` なし | held，rows | `requested.map_or(now+lead, …)` にする（B が 9 ms） |
| 2. 同じ A・B，B を 7 ms − 1 tick | MockRadio（x3xx。ideal は床が 0 なので B=9.5 の同じ結果） | `mr_18_the_window_edge_excludes_one_tick`（新規） | 1 と同じ。B の受け取りを `T0+6_999_999`（1 ns）にして `step` | `L` なし。`applied` section が B(2.0, T0+9.5)，A(1.0, T0+10) の順 | events，applied | 境界を `e <= now+window` のまま 1 tick 広げる（B を 10 にする）。window を 3 ms より 1 tick 大きく取る |
| 2. 同上 | UHD（manual_rig_events） | `ur_24_the_window_edge_excludes_one_tick`（新規） | B の受け取りを 6.999995 ms（200 MHz root の 1 tick = 5 ns） | `L` なし。rows が B(9.5) の次に A(10) | events，rows，device calls | 境界を `≤` のまま 1 tick 広げる。lead に profile の 5 ms を使う（B が 12 ms になり L が出る） |
| 3. B(at=11) を 7 ms | MockRadio（x3xx。ideal は L なしで B=11 の同じ結果） | `mr_18_a_timed_update_beyond_the_window_keeps_its_instant`（新規） | 1 と同じで B の `at=T0+11 ms`，最後の `step(T0+11)` | `L` なし。`applied` section が A(1.0, T0+10)，B(2.0, T0+11) | events，applied | 床を「後の command の instant」に丸める（B を 10 にする）。window を 3 ms より大きくする |
| 3. 同上 | UHD | `ur_24_a_command_beyond_the_window_keeps_its_instant`（新規） | 1 と同じで B `at=11 ms`。release を 7 ms の後 8 ms にも呼ぶ（B は 11−8=3 で渡る）。行は `advance_to(11)`＋`release()` の後 | `L` なし。rows が A(10) の次に B(11)。device calls もその順 | events，rows，device calls | 床を `released` の最大に丸める（この場面では A はまだ released でないので差は系列 1 が持つ）。window を 3 ms より大きくする |
| 4. A の channel が e に無く後ろの segment にも無い。B は別の channel で live，9.5 を 7 ms | MockRadio `tests/mock_layout.rs`（x3xx，`two()`，link `rx0`） | `mr_18_a_timed_update_behind_one_waiting_for_a_channel_is_late`（新規） | `built(Layout { profile:"x3xx", rx: two(), tx: elements(&["0:A:ubx:0"]), … })`，`arm_start(T0)`。A `radio.rx.c1.gain_db=1.0`（既定で c1 はどの stream にも無い），`at=T0+10` を push して `step(T0)`。B `radio.rx.c0.gain_db=2.0`（c0 は stream 0，live），`at=T0+9.5` を push して `step(T0+7)`。`step(T0+10)` | `L` 1 件（9.5→10）。`applied` は A(1.0,10)，B(2.0,10) の順。Mock の行に `no channel` は無い（MR-18 は channel の有無を記録しない） | events，`applied_of` | 床を「渡した command」だけから取る（A は渡せないので B=9.5）。床を channel の live なものだけにする |
| 4. 同上 | UHD（`control(&core, &[Dir::Tx])`：rx c0 を -1，tx c0 を live） | `ur_24_a_command_waiting_for_a_channel_floors_a_later_booking`（新規） | A `radio.rx.c0.gain_db`，`at=10 ms` を 0 ms。`advance_to(7 ms)`，`release()`（A は e=10，3 ms で窓の内，channel 無しの先頭なので止まる）。B `radio.tx.c0.gain_db`，`at=9.5 ms` を 7 ms に book。`release()`（A が先頭でまた止まる）。`advance_to(10 ms)`，`release()` | `L` 1 件（9.5→10）。10 ms の release で A の `rows` が `(10,false,true)`（`note="no channel"`），その後に B が `(10,true,false)`（issued）。`device.calls()` に `apply tx 0 … at=10` が 1 件，`apply rx 0` は無い | `events_of`，`rows_of`，`device.calls()` | 床を `released` だけから取る（A は渡していないので B=9.5）。A が channel 待ちのとき release を止めず B を先に渡す（B の行が A より先になり，device の順が予約順でない） |
| 5. 装置が A を拒否した後の B（UHD だけ） | UHD（`ApplyFails::timed(inner, Dir::Rx, None)`：最初の timed apply だけ拒否） | `ur_24_a_refused_release_floors_the_next_booking`（新規） | `rig_on_inputs(fails, …)`，`hold`。A `radio.rx.c0.gain_db`，`at=10 ms` を 0 ms。`advance_to(7 ms)`，`release()` で A を渡し，wrapper が拒否（`COMMAND_REJECTED`，行なし）。`advance_to(7.2 ms)`，B `at=9.5 ms` を book，`release()`（B は 10 ms で渡る） | `L` 1 件（9.5→10）。A の `rows` は e=10 でも空（拒否は行を書かない）。`fails.attempts()` の先頭が `(Rx,0,Some(10 ms))`。B の行は 10 ms | `fails.attempts()`（wrapper），`inner.calls()`，`events_of`，`rows_of` | 床を「装置が受け取った command」だけから取る（拒否された A を数えないので B=9.5）。拒否で A の枠を空ける（5b） |
| 5b. 深さ 16，A と遠い 15 件，A 拒否後の B（UHD だけ） | UHD（`ApplyFails::timed`） | `ur_24_a_refused_release_holds_its_queue_slot_until_its_instant`（新規） | A `at=10 ms` と遠い 15 件（`at=20 ms+i`）を 0 ms に予約。`advance_to(7 ms)`，`release()` で A を拒否。`advance_to(7.2 ms)`，B `at=9.5 ms` | `L` 1 件（9.5→10）の後に `Q` 1 件（`depth=16`）。B は予約されない。`held.len()==15`。A の枠は 10 ms まで。`advance_to(10 ms)`＋`release()` の後の C（`at=13 ms`）は予約でき，`held.len()==16` | `events_of`（L と Q の順），`control.held`，`fails.attempts()`，`inner.calls()` | 枠を `held + released` にする（拒否で A が消え，B が予約できる）。拒否で枠を空ける |
| 6. A を渡した後，7.2 ms に A の channel を cold change で外し，B を 7.3 ms | MockRadio（`tests/mock_layout.rs`，x3xx，`two()`） | `mr_18_a_channel_leaving_after_the_booking_does_not_move_the_floor`（新規） | A `radio.rx.c0.gain_db=1.0`，`at=T0+10` を `step(T0)` で予約。`step(T0+7.2)` で `radio.rx.c0.stream=-1`（Cold，`at=T0+7.2`）。B `=2.0`，`at=T0+9.5` を push して `step(T0+7.3)`。`step(T0+10)` | `L` 1 件（9.5→10）。applied が A(1.0,10)，B(2.0,10) | events，applied | 床を「今 channel が live な command」だけにする（A の channel が 7.2 に外れると B=9.5）。cold change の instant で床を切る |
| 6. 同上 | UHD（`control(&core, &[Dir::Rx])`） | `ur_24_a_channel_leaving_after_the_release_does_not_move_the_floor`（新規） | A `radio.rx.c0.gain_db`，`at=10 ms` を 0 ms，`advance_to(7 ms)`，`release()` で A を渡す。`advance_to(7.2 ms)`，`book_cold(radio.rx.c0.stream=-1, at 7.2 ms)`。`advance_to(7.3 ms)`，B `at=9.5 ms` | `L` 1 件（9.5→10）。B の行 `(10,false,true)`（9.5 は 7.3+3 以内で床は 10，channel は無い）。A の行 `(10,true,false)`（7 ms に渡した） | events，rows，device calls | 床を `channel_live` の結果で作る（A の channel が外れたので B=9.5）。`released` を release のたびに channel で消す |

### 1.2 系列 7–9d

| 系列 | どこで | テスト | 場面の作り方 | assert | 観測 | 落とす誤り |
|---|---|---|---|---|---|---|
| 7. A(10)・B(12) を予約し，8 ms に Action の `Stop` | MockRadio（x3xx，1 stream） | `mr_18_a_stop_leaves_each_update_its_row_at_its_instant`（新規） | A gain 1.0 `at=T0+10`，B 2.0 `at=T0+12` を T0 に push。`step(T0+7)` の後に `Action::Stop { target: mock }` を push して `step(T0+8)`。`step(T0+12)` | `L` なし。applied が A(1.0,10)，B(2.0,12)。`rejected` にこの 2 件は無い | events，applied，rejected | `Stop` で pending の update を取り消す（`stop_at` を Action の `Stop` にも使う）。行を instant でなく `Stop` の時点に書く |
| 7. 同上 | UHD（`control(&core, &[Dir::Rx])`） | `ur_26_a_stop_leaves_each_update_its_row_at_its_instant`（新規） | A `at=10`，B `at=12` を 0 ms に book。`advance_to(8 ms)`，`book(Action::Stop{target: usrp})`。`advance_to(10)`＋`release()`，`advance_to(12)`＋`release()` | A・B とも `rows` がそれぞれの instant に `issued:true`（`cancelled` でない）。`device.calls()` に `at=10`，`at=12`。cancelled 行なし | rows，device calls，events | `Stop` で `held`/`released_at` を消す。`channel_live` を `Stop` で halted にする（今の `halted` は refusal だけ。この計画は今のままを前提にする） |
| 8. 同じ A・B，8 ms に `Provider::stop`（喪失も同じ） | MockRadio（x3xx，1 stream） | `mr_25_a_stop_records_a_later_update_as_cancelled_at_the_stop`，loss は `mr_20_a_loss_records_a_later_update_as_cancelled_at_the_loss`（新規。既存 `mr_20_a_loss_cancels_what_it_left_pending`：1720 の形を 2 件に広げる） | A `at=T0+10`，B `at=T0+12` を T0 に push。`step(T0+8)` の後に `mock.stop(Orderly)`。loss は `sim.faults` の `device_lost` を `at_ns = 8 ms` にして `arm_start`，`step(T0+8)` の `Err(DeviceLost)` | `applied` は空。`rejected` は 2 件，`at=T0+8`，reason は stop が `"MR-25: cancelled by stop"`，loss が `"MR-20: device lost"` | `rejected`，`applied` | 停止/loss の S でなく step の時刻で記録する。loss を stop として記録する |
| 8. 同上 | UHD（`manual_rig_events`＋`hold`） | `ur_26_provider_stop_records_a_given_and_a_held_update_alike`（新規） | A `at=10` を 0 ms，`advance_to(7)`，`release()`（A は渡る）。B `at=12` を 7 ms に book。`advance_to(8)`，`lock(&core.streams).stop = Some((ms(8), Orderly)); end(&core, ms(8), false, false)`。`advance_to(10.6)`，`control.finish()`。loss は `core.device_lost("test")` を 8 ms に | A・B の `applied` 行が e=10/12 に 1 行ずつ，`cancelled` があり `cancelled_at=8 ms`。issued/no-channel の行は無い。A の device 呼び出しは 7 ms の 1 件だけで，8 ms 以降は無い | `rows_of`（`cancelled`/`cancelled_at` を含めて読む），`device.calls()` | `held` だけ消して `released_at` を残す（A の行が issued のまま）。`finish` の `now` で `cancelled_at` を書く（10.6 になる）。loss で `e ≤ S` を issued にする |
| 9. A が 10 ms に効いた後に停止 | MockRadio（x3xx） | `mr_18_an_update_applied_before_a_stop_keeps_its_row`（新規） | A `at=T0+10` を T0。`step(T0+10)`。`step(T0+12)` の後に `mock.stop(Orderly)` | A の行 `(10)` が残る。`rejected` に A は無い | applied，rejected | 停止時に `applied` を消す。停止で A を拒否として書き直す |
| 9. 同上 | UHD | `ur_26_an_applied_update_keeps_its_row_after_a_later_stop`（新規） | A `at=10` を 0 ms，`release()` を 10 ms（行が書かれる）。`advance_to(12)`，stop/end/finish | A の行 `(10,true,false)` が残る。cancelled 行は A に無い | rows | `finish` が `released_at` の既に折った行まで消す。`cancel_held` が A を再記録する |
| 9b. A を 7 ms に渡し，最後の release 9.6 ms，喪失 10.2 ms，制御のスレッドが次に 10.6 ms | MockRadio（x3xx） | `mr_18_a_loss_after_the_last_pass_keeps_a_row_due_before_it`（新規） | A `at=T0+10` を `step(T0+7)`。`step(T0+9.6)`。loss `at_ns=10.2 ms` で `step(T0+10.2)`（`Err(DeviceLost)`） | A の行 `(10)` がある。`rejected` に A は無い | applied，rejected | 喪失で e < S のものも拒否する。`step(10.2)` の `advance` で A を適用しない |
| 9b. 同上（前半） | UHD（manual_rig_events） | `ur_26_a_loss_settles_what_the_last_release_gave`（新規） | A `at=10` を 0 ms，`release()` を 7 ms（A は渡る）。`advance_to(9.6)`，`release()`（行はまだ無い）。`advance_to(10.2)`，`core.device_lost("test")`。`advance_to(10.6)`，`finish()` | A の行 `(10,true,false)`（issued）。9.6 ms 以降の device 呼び出しは無い | rows，device calls | 行を give の時点で書く（D2 (3) の試験が殺す）。loss の `finish` で行を書かない（A の行が無い）。`finish` の now（10.6）で e を判定する（A は e=10 ≤ 10.6 で issued になり，この前半だけでは差が出ない。後半で殺す） |
| 9b. 後半：停止 9.9 ms，終わりの処理 10.6 ms | UHD | `ur_26_a_stop_settles_by_its_instant_not_by_the_finish`（新規） | A `at=10` を 0 ms，`release()` を 7 ms。`advance_to(9.6)`，`release()`。`advance_to(9.9)`，stop/end(9.9)。`advance_to(10.6)`，`finish()` | A の行 `(10,false,false)` で `cancelled` があり `cancelled_at=9.9`。issued ではない | rows | `finish` の now を S に使う（A を issued にする）。S を 10.6 にする |
| 9c. 渡した A の e = S ちょうど | MockRadio（x3xx） | `mr_18_an_update_due_at_the_stop_instant_applies_and_at_the_loss_instant_does_not`（新規） | A `at=T0+10` を `step(T0+7)`。停止は `step(T0+10)` の後に `mock.stop(Orderly)`。loss は `at_ns=10 ms` で `step(T0+10)` | 停止：A の行 `(10)` がある。loss：A の行は無く，`rejected` 1 件（loss の reason，`at=10`） | applied，rejected | loss を `e ≤ S` で数える（A の行ができる）。停止を `e < S` で数える（A が拒否になる） |
| 9c. 同上 | UHD（manual_rig_events） | `ur_26_an_instant_at_the_stop_applies_and_one_at_the_loss_does_not`（新規） | A `at=10` を 0 ms，`release()` を 7 ms。停止：stop/end(10)，`advance_to(10.6)`，`finish()`。loss：`core.device_lost` を 10 ms，`advance_to(10.6)`，`finish()` | 停止：A の行 `(10,true,false)`。loss：A の行 `(10,false,false)` で `cancelled_at=10`。loss の 7 ms 以降の apply は無い | rows | 停止で `e < S`（A を cancelled にする）。loss で `e ≤ S`（A を issued にする） |
| 9d. 系列 4 の A（channel 無し）と B（live）が両方 e=10，最後の release 9.6，喪失 10.2，次の poll 10.6（UHD だけ，Ceiling） | UHD（`control(&core, &[Dir::Tx])`：rx c0 無し，tx c0 live） | `ur_24_a_due_command_behind_a_channel_less_head_is_cancelled_at_the_loss`（新規。停止の場合も同じ test の 2 ケース） | A `radio.rx.c0.gain_db`，B `radio.tx.c0.gain_db`，両方 `at=10` を A→B の順で 0/7 ms に book。`release()` を 7 ms と 9.6 ms（A が先頭で止まる）。`advance_to(10.2)`，`core.device_lost`。`advance_to(10.6)`，`finish()`。停止は stop/end(10.2) | `applied` の並びが A の行 `(10,false,true)`（`no channel`），B の行 `(10,false,false)`（cancelled，`cancelled_at=10.2`）の順。B の apply は無い。停止の場合は停止を S=10.2 にする（`e ≤ S` の Ceiling） | rows（並びまで），device calls | A の `no channel` を cancelled にする。B を release が渡していないのに issued にする。B の行を Ceiling として書かない（`finish` が両方消す）。B の行を A の前に置く（逆順） |

### 1.3 系列 10–12 と D2 (3)

| 系列 | どこで | テスト | 場面の作り方 | assert | 観測 | 落とす誤り |
|---|---|---|---|---|---|---|
| 10. 同じ e の 2 つの command：行の順は予約の順 | MockRadio（x3xx） | `mr_18_two_updates_at_one_instant_keep_the_booking_order`（新規。既存 `mr_18_hardware_timed_updates`：1042 の発展） | gain 1.0 を T0，gain 2.0 を T0+7，両方 `at=T0+10`。`step(T0+10)` | `applied` section が 1.0, 2.0 の順（同じ T0+10）。後の 2.0 が最後の行 | applied | `updates` を `(e, order)` でなく key 順にする。最後の値で前の行を上書きする |
| 10. 同上 | UHD | `ur_24_two_updates_at_one_instant_write_their_rows_in_booking_order`（新規） | 同じ 2 つを 0/7 ms に book。`release()` を 10 ms | `rows_of` が claimed 1.0→2.0 の 2 行。`config_at(10)` は 2.0。`device.calls()` の `at=10` も同じ順 | rows，device calls，`config_at` | `released_at` を key 順・逆順に回す。行を give の時点で書く（7 ms で 2 行になる） |
| 11. 満杯で requested が床より前：L の後に Q，予約されない。効いた command は instant で枠を外れる | MockRadio（x3xx） | `mr_18_a_late_command_at_a_full_queue_reports_late_first`（新規。既存 `mr_18_hardware_timed_updates`：1062-1078 の 17 件目も直す） | 16 件 `at=T0+5+4i ms`（5, 9, …, 65 ms。release が 1 件だけ渡る間隔）を T0。17 件目 `at=T0+1 ms` を T0。`step(T0+5)` で 5 ms の 1 件を適用した後，`at=T0+30 ms` を push して `step(T0+5)`（receipt 5，床 7，L なし） | events の順が `L`（1→2），`Q`（depth 16）。17 件目は予約されない。5 ms の 1 件が効いた後の 1 件は予約できる（15+1） | events，applied | 満杯を late の前に見る（L が出ない）。Q の後に L を出す。17 件目を予約する。e を過ぎた command を数え続ける |
| 11. 同上 | UHD（manual_rig_events） | `ur_24_a_late_command_at_a_full_queue_reports_late_first`（新規） | 同じ 16 件。17 件目 `at=1 ms`，受理されない。`advance_to(5)`＋`release()` の後に 1 件 `at=30 ms` | 同じ順。17 件目の拒否の後 `control.held` は 16（17 件目なし），5 ms の release の後は 15。次の 1 件は予約できる | events，held，device calls | 枠を `held + released` にする（16 で止まる）。late を full の後に見る（L が出ない） |
| D2 (3). 行は instant に書く（渡した時点ではない） | MockRadio（x3xx） | `mr_18_the_row_is_written_at_its_instant`（新規） | A `at=T0+10` を T0。`step(T0+9.999)`，`step(T0+10)` | 9.999 の applied は空。10 で A の行 `(10)` | applied | 予約時に書く（9.999 で行がある）。満杯/coerce の時点で書く |
| D2 (3). 同上 | UHD | `ur_24_the_row_is_written_at_its_instant`（新規） | A `at=10` を 0 ms，`release()` を 7 ms（device は `at=10`）。`advance_to(9.999)`，`release()`。`advance_to(10)`，`release()` | 9.999 の rows は空。10 の release で 1 行（at=10，issued）。7 ms の device 呼び出しは残る | rows，device calls | `give` で行を書く（9.999 で行がある）。`released_at` の fold でなく `released` の prune で行を書く（instant がずれる） |
| 12. differential harness | UHD `control/differential.rs`（既存） | `rm_26_the_timeline_against_uhd_control`（161） | 1 000 系列。`release_window_matters` を消し，全系列で `timed` を比べる。`Record` に `late` と `applied` を足す（§4） | 全系列で `timed`，`late` の中身，正規化した `applied`（Ceiling の command の行だけ除く）が一致。`rejected` 0 | `Record` の `diverged` と summary | `release_window_matters` を残す。`timed` を一部の系列で比べない。`late` を捨てる。Ceiling を系列から見分けず，行を全部比べる（9d の 1 件で落ちる） |

Mock の VH-8 layer 1（`crates/ezsdr-mock-radio/tests/timeline.rs:271`，`rm_26_the_timeline_against_mockradio`）も同じ床を expected に入れる（§4.2）。設計メモ §4b には無いが，入れないと 1 000 系列の `timed` が新しい床で食い違う。

## 2. D1・D2 の各項目

### D1. `radio.timing.release_window_ns`

- **値**：`TimingEnvelope` に `release_window_ns` を足す。`x3xx::timing` は 3 ms，`channels.rs:380` の ideal は 0。値の置き場は `x3xx::timing` の 1 か所（`ProviderTerms` に足して各 Module が 3 ms を書く実装でも，テストが見る値と読む場所は同じ）。
- **宣言**：`ezsdr_radio::keys::RELEASE_WINDOW_NS = "radio.timing.release_window_ns"`，RM-4 の key 表（`design/07-radio-model.md:61-72`）と `channels::Description::capabilities`（`channels.rs:488-506`）に足す。`TimingEnvelope` の literal は 3 か所（`x3xx.rs:393`，`channels.rs:380`，`crates/ezsdr-radio/tests/timeline.rs:111`）を直す。
- **schema**：`schemas/radio/envelope.v1.json` を再生成し，`rm_20_schema_freeze`（`crates/ezsdr-radio/tests/radio_model.rs:374`）を通す。`design/07` RM-20 の六 members を七に。version は上げない（AGENTS.md §6）。
- **fixture**：独立した JSON profile は無い。fixture は `mr_03_profile_values_reach_the_capabilities_and_the_envelope_section`（`crates/ezsdr-mock-radio/tests/mock_radio.rs:445`）と `ur_09_profile_values_reach_the_capabilities_and_the_envelope_section`（`crates/ezsdr-radio-uhd/tests/fake.rs:486`）の期待値，`rm_04_the_key_table_is_exactly_the_declared_one`（`radio_model.rs:140`）の行。Mock は x3xx 3 ms / ideal 0，UHD は 3 ms を capability と `envelope.timing` の両方で見る。
- **両 Module がその値を使うこと**：
  - Mock：`handle_update` が `self.profile.timing().release_window_ns` を床に渡す。x3xx の系列 1/2 が 3 ms を，ideal の同じ系列（window 0）が 0 を固定する。literal 3 ms を書くと ideal の系列 1 が B を 10 にして L を出し，落ちる。literal 0 を書くと x3xx の系列 1 が L を出さず落ちる。
  - UHD：`release` が `self.core.description.timing.release_window_ns` を読む。x3xx の系列 1/2 に加え，`ur_24_the_release_window_comes_from_the_description`（新規）：`default_description(2_000)` の `timing.release_window_ns` を 10 ms にした description で `rig_on_description_links` を組み，A `at=12`，B `at=9.5`（受け取り 7）を予約する。10 ms なら A（12 ≤ 17）が床に入り B は 12 ms で L。3 ms の定数なら B は 9.5 で L なし。これが「定数でなく description を読む」を殺す唯一の試験（x3xx は 3 ms 固定なので，境界の系列だけでは定数と区別できない）。
- **ideal（0）が今と同じ**：window 0 では床の第 2 項は `e <= now` の予約だけで，lead 0 の `now` を超えない（Mock は Action を `advance(now)` の前に処理するので e == now や e < now の予約が残っていても，床は `now` のまま）。よって ideal の振る舞いは変わらない。見る試験：ideal の系列 1/2/3（L なし，B=9.5/11），`rm_26_the_timeline_against_mockradio` の ideal 1 000 系列，既存の `mr_18_hardware_timed_updates`（x3xx）と `mr_18_a_layout_hardware_timed_change_keeps_its_own_instant`（x3xx）。release の段を Mock に足さない（D1 の「Mock は profile の値を使う」は床の window のこと）。

### D2 (1) floor

系列 1–6，1b，11 が床を固定する。語彙の unit test を 1 本足す：`crates/ezsdr-radio/tests/timeline.rs` の `rm_25_the_timed_instant_is_floored_at_every_booked_command_inside_the_window`（新規）。表：

| at | now | lead | window | earlier | e | late |
|---|---|---|---|---|---|---|
| Some(100) | 0 | 20 | 30 | [] | 100 | false |
| Some(45) | 0 | 20 | 30 | [50] | 50 | true |
| Some(50) | 0 | 20 | 30 | [50] | 50 | false（床ちょうどは late でない） |
| Some(50) | 0 | 20 | 30 | [51] | 51 | true |
| None | 0 | 20 | 30 | [50] | 50 | false（依頼なしは late でない） |
| Some(35) | 25 | 20 | 30 | [50] | 50 | true（50 ≤ 55） |
| Some(80) | 10 | 20 | 30 | [50] | 80 | false（50 > 40 で窓の外） |
| Some(10) | 40 | 20 | 30 | [] | 60 | true（過去の at） |
| Some(5) | 10 | 0 | 0 | [10] | 10 | true（window 0 は e ≤ now だけ） |
| Some(10) | 10 | 0 | 0 | [10] | 10 | false |

`layout_instant` の表（`rm_25_the_layout_instant_is_floored_at_every_earlier_cold_change`，`timeline.rs:1448`）はそのまま。両 Module はこの関数を呼び，各自の `lead` と `window` を渡す。UHD の `lead` は `DEVICE_LEAD_NS`，Mock は profile の `min_timed_command_lead_ns`。UHD の宣言 5 ms を渡す変異は系列 2 が殺す（B が 12 ms で L）。

### D2 (2) 枠

- 枠は「予約されて e がまだ確定していない timed update」の数。予約の履歴だけから決める。UHD は `held + released` をやめ，予約の台帳（booked，`e >= now`：同じ pass の release が確定する前の e == now も数える）を 1 つ持つ。拒否された release も e まで台帳に残す。Mock は `self.updates` の `HardwareTimed` がこれに当たる（booked のままで，`advance` が適用したら消す。拒否は予約時の full だけで，その command は入らない）。
- 系列 5b と 11。5b の「拒否で枠を空ける」を殺す。11 の「効いた command は instant で数から外れる」を殺す。
- 境界：`e == now` の予約が同じ step の Action の予約で枠を占めるか。設計メモの「e がまだ来ていない」は `e > now` とも読めるが，Mock の `step` は Action を `advance(now)` の前に処理するので e == now はまだ適用されておらず，UHD の loop も book の後に release が確定する。この計画は両方 `e >= now` とし，境界そのものは §7 の 9 に残す。系列 11 は境界を避け，release/step の後で数える。

### D2 (3) 行

D2 (3) の 2 行（Mock/UHD）と，系列 1/2/3/4/6/7/8/9/9b/10/11 の行の assert が，すべて「`at` は e」であることを見る。既存テストの書き換えは §3。

### D2 (4) 終わり

系列 8（S 基準，given/held 同じ），9（e < S は残る），9b（S の前の e を遅れて処理），9c（e = S の stop/loss の順），9d（Ceiling，UHD だけ）。`finish` は `streams.stop` の S か `lost_at` の S（最初に起きた方）を 1 度だけ読み，`cancel_held` の `cancelled_at` に使う。`e < S`（stop は `e <= S`）で，一度も渡されていない live な command は Ceiling として cancelled（9d）。渡されていない channel 無しの command は `no channel`（9d の A）。refused は行を書かない（D2 (3)）。同じ時刻の順は Mock の今のまま。

### spec の本文と Test 行

- `design/18-uhd-radio.md`：§3 の constants（62）から release window を外し profile の値に；UR-9 の note（188）の「held plus released」を「booked で instant がまだ来ていないもの，拒否された release を含む」に；UR-24（253-255）の床を「予約の履歴（渡したか，channel 待ちか，拒否かは問わない）」，深さ，行を instant に；UR-26（299）の `Provider::stop` を S で held/given を同じに確定する形に；UR-30（317）の `applied` の説明；§5 の U13（360）の「released §3's release window ahead」を profile の値に；*Test:* 行（256-259，300，321）に §1 と §3 の新しい名前。
- `design/09-mock-radio.md`：MR-18（149-150）の床，late の後に full，深さ；MR-25（174）の stop の S と pending の拒否，loss の同じ扱い；*Test:* 行（153-154，175）と §6 の該当行（305-306，326-329）に新しい名前。
- `design/07-radio-model.md`：RM-4（61-72）の key；RM-20（202-203）の七 members と schema；RM-21（230）の `hardware_timed` の床と深さ（`timed_instant` は RM-25 の `layout_instant` の隣），*Test:* 行に `rm_25_the_timed_instant_is_floored_at_every_booked_command_inside_the_window`。`rm_20_schema_freeze` の行は七 members。
- `schemas/radio/envelope.v1.json` は再生成。`SCHEMA_CHANGELOG` は書かない（freeze 前，AGENTS.md §6）。

## 3. 既存のテストの書き換え

### 3.1 release の直後に `issued` の行を見るもの（UHD）

「残す」は device 呼び出し・plan・projection・read-back，「移す」は行の assert を `e` の後（release か `finish` の後）へ。

| テスト（`control.rs`） | 行 | 残す | e の後へ移す |
|---|---|---|---|
| `ur_24_a_timed_update_reaches_a_later_stream_s_pending_segment` | 1643（row 1662） | `apply rx 1 at=200` の 1 件（197 ms の release） | `rows_of` を `advance_to(200)`＋`release()` の後へ |
| `ur_24_a_release_without_a_live_channel_reaches_the_pending_segment` | 1667（row 1691） | device 呼び出し，`control.released == [50]`（47 ms の時点），read-back | `row`（`note="no channel"`）を 50 ms＋`release()` の後へ |
| `ur_24_a_command_waiting_for_a_channel_is_given_before_a_later_one` | 1721（row 1751） | device の順（10，11） | `rows_of` を 11 ms＋`release()` の後へ |
| `ur_24_a_command_waiting_for_a_channel_keeps_a_later_booking_behind_it` | 1755（row 1780） | device の順 | 場面も変える（下の 3.2）。rows を各 e の後へ |
| `ur_24_a_command_waiting_for_a_channel_holds_back_the_one_behind_it` | 1784（row 1809） | device 呼び出し（tx at=101） | tx の行を 101 ms＋`release()` の後へ（rx の no-channel 行は 100.5 でよい） |
| `ur_14_booking_does_not_wait_for_a_future_cold_switch` | 1944（row 1965） | plan の cut/origin，`core.now()==0`（booking の後） | 行を `advance_to(2 ms)`＋`release()` の後へ |
| `ur_24_a_direction_halted_by_a_refusal_has_no_channel` | 2365（row 2400） | 199 ms の apply 1 件，`config_at(200)`，plan | 行を 200 ms＋`release()` の後へ |
| `ur_25_a_timed_update_at_the_cold_change_s_instant_reaches_the_switch` | 2549（row 2586） | switch の configure，298 ms の timed apply | 行を 300 ms＋`release()` の後へ |
| `ur_24_a_release_behind_a_released_command_keeps_the_device_queue_order` | 3769（row 3809） | device の順，projection，`released`，read-back | 2 行を 10 ms＋`release()` の後へ |
| `ur_24_a_segment_booked_while_a_command_waits_for_a_channel_holds_it_at_its_instant` | 3863（row 3899） | projection，device の 100.6 の read-back | `release()` を 100.6 の後にもう一度呼んでから行 |
| `ur_25_an_enable_at_a_timed_update_s_instant_applies_it` | 4141（row 4172） | configure と timed apply の数，L なし | 行を 300 ms＋`release()` の後へ |
| `ur_24_a_channel_keyed_timed_update_acts_on_its_own_channel` | 4189（row 4218） | apply の channel，L なし | 行を 40 ms＋`release()` の後へ |
| `ur_24_the_layout_destination_follows_the_effective_membership` | 4322（row 4349） | apply rx 1 at=120，L なし | 行を 120 ms＋`release()` の後へ |
| `ur_34_a_timed_update_crossing_the_move_reaches_each_streamer_channel_at_its_instant` | 4475（row 4518） | 170/280 の apply と read-back | `release()` を 280/300 の後にもう一度呼んでから 2 行 |

### 3.2 場面（instant）を変えるもの

- `ur_24_a_command_waiting_for_a_channel_keeps_a_later_booking_behind_it`（1755）：今の A(rx)=100.5，B(tx)=99.8，受け取り 97.6 は，A が 100.5 ≤ 97.6+3 なので D2 (1) で B が 100.5 に遅れる。この試験の目的（待っている command が後の予約の instant を動かさない）を残すため，A を 101 ms，channel を戻す cold change を 100.8 ms にする（A は 97.6+3 の外）。B は 99.8 のまま，L なし。B は 97.6 の release で 99.8 に渡り，A は 98 ms の release で 101（cold change で channel は live，issued）。rows は各 e の後。窓の内側の「B が A の e に遅れる」は系列 4 が持つ。
- `ur_29_a_loss_after_a_release_s_command_stops_the_one_behind_it`（3706, row 3736）：loss の後に `control.finish()` を呼ぶ。A（apply 済み，e=49，S=52）は issued の行 `(49,true,false)`。B（e=51，渡されていない。channel は 50 ms の cold change で既に外れている）は `no channel` の行 `(51,false,true)`。今の「A の行だけ」は「A issued，B no channel」に。device は A の 1 件だけ。B は live な Ceiling ではないので，9d の「live で渡されない」とは別。
- `mr_18_hardware_timed_updates`（`crates/ezsdr-mock-radio/tests/mock_radio.rs:1042`）の full の 17 件目（1062-1078）：今は `at=0`，16 件が 10 s なので D2 (1) で 17 件目は 2 ms に遅れ，L が先に出る。今の assert `LATE_COMMAND == 0`（1072）は成り立たない。この試験の目的（full だけ）を残すため 17 件目を `at=9 ms`（床 2 ms より後）にし，両方の場合は系列 11 の新規 test に回す。

### 3.3 変わらないと確かめたもの

- `ur_24_a_release_without_a_live_channel_or_a_later_segment_applies_nothing`（3471, row 3486），`ur_24_a_release_for_a_refused_transmit_segment_applies_nothing`（3548, 3568），`ur_24_a_release_for_a_refused_receive_change_has_no_candidate`（3573, 3592），`ur_24_a_release_stays_when_its_segment_is_refused_after_it`（3597, 3619），`ur_24_a_released_command_and_one_without_a_channel_settle_in_instant_order`（3491, 3515），`ur_24_a_timed_update_acts_on_the_configuration_in_force_at_its_instant`（2343, 2358），`ur_24_a_refusal_halts_only_its_own_stream`（2659, 2698），`ur_24_a_release_after_its_instant_applies_on_arrival`（3451, 3467），`ur_24_a_command_waiting_for_a_channel_is_booked_at_the_instant_it_can_take_effect`（3815, 3858）：行を読む時点が既に e 以降なので，そのまま。行を読まないもの（3342，3370，3525，3960，3987，4015）もそのまま。
- `rows_of` を読むが e 以降か空を確かめるものも，そのまま：`ur_24_a_late_update_halted_before_it_can_act_has_no_channel`（2592, row 2614），`ur_29_a_failed_released_update_is_a_device_failure`（3678, 3697/3702），`ur_24_a_refused_release_records_no_row_and_keeps_the_projection`（3741, 3764）。`ur_24_a_stop_cancels_a_command_waiting_for_a_channel`（1813, row 1838）は `cancel_held` を直接呼び，その時点で cancelled の行が書かれる。`cancel_held` が S を引数に取る形になっても，この test は呼び出しの時刻を渡すだけ。
- `ur_24_the_command_queue_is_counted_per_instance_across_streams`（1609），`ur_24_a_release_without_a_live_channel_takes_the_command_queue_slots`（3417）：`control.released.len()` を内部の数として使う。実装が `released` を残すならそのまま，台帳に置き換えるなら台帳の数に直す。
- `ur_29_a_lost_device_applies_no_released_update`（3623），`ur_29_a_loss_published_before_a_release_stops_the_call`（3640），`ur_29_a_lost_timed_release_publishes_device_loss`（3656）：`finish` を呼ばないので「release は行を書かない」のまま。end の行は `finish` の試験（9b/9c）が持つ。実装が `device_lost` の中で end を書くなら，この 3 本は「cancelled の行 1」に直す（§7 の 7）。
- `tests/fake.rs` の `ur_24_a_released_command_is_not_recalled_by_stop`（1627），`ur_24_stop_cancels_held_commands`（1655）：前者は Action の `Stop`（D2 (4) の対象外），後者は Run が 1 ms を過ぎてから `Provider::stop`。行は e の後に書かれ，そのまま通る。`fake.rs:6042`，`fake.rs:6554` は `settle` が e を過ぎてから Manifest を読むので，そのまま。

## 4. harness

### 4.1 UHD `src/provider/control/differential.rs`

- `release_window_matters`（214-222）と `RELEASE_WINDOW_NS` の import/使用（24，217）を消す。`uncompared` と `timed` の除外（184-186，203）も消し，`timed` を全系列で比べる。`DIVERGENT` は空のまま。
- `Record`（47-53）に足す：
  - `late: Vec<(Option<String>, i64, i64)>`（`key`，`requested`，`applied`）。observed は `events_of(&events, LATE_COMMAND)` のうち `key` が `radio.<d>.c<n>.{frequency_hz,gain_db}` のもの（cold と start の LATE は今の harness の expected が持たないので比べない）。expected は `layout_schedule` の予約順（`sequence.ops()` の順）で作る。
  - `applied: Vec<(String, f64, i64, bool)>`（key，claimed，at，cancelled）。observed は `rec.applied` の `read_back` が null の行。`note` と `issued` は落とし，`cancelled` だけ残す。並びは `(at, key)` の安定 sort（同じ key・同じ at は予約順）。expected は「loss の前に受け取り，e < S は applied，e ≥ S は cancelled」を作る。Ceiling の command の行は両方から除く。
- `terms()`（73-79）に window を足す。値は `super::super::test_support::default_description(2_000).timing.release_window_ns / 5`（定数を読まない）。`Terms` に `release_window` を足す（`crates/ezsdr-radio/tests/generator/mod.rs:126`）。`Sequence::schedule`（163-185）の doc と式も `timed_instant` にそろえる（`items()` は timed を捨てるので rx の挙動は変わらないが，規則が古いままにならないように）。
- `layout_schedule`（82-109）：`Op::Timed` を `ezsdr_radio::timeline::timed_instant(requested, receipt, terms.timed_lead, terms.release_window, それまでの timed e)` にする。timed e を予約順に集め，`late` を別に集める。cold の floor（99）はそのまま。`expect`（226-244）は `timed` に加えて `late` と `applied` を作る。loss の後の行は，`entry.receipt < loss` で予約されたものだけ。
- `run`（279-400）：`run` の戻りに Ceiling の集合を足す（`Vec<(String, f64, i64)>`）。各 `control.release()` の後（または device の `apply … at=<e>` の log）で「渡した e」を集める。loss の後，`core.device_lost` の直後に `control.finish()` を呼び（`Provider::stop` の end と同じ entry），行を確定する。Ceiling は「`e < S` かつ その e が渡した集合に無い」timed op。test は `expect` と observed の `applied`/`timed` から `(key, claimed, e)` が一致する行を 1 つずつ除く。同じ `(key, claimed, e)` の 2 件は記録上区別できないので，1 件ずつ除く（値は generator で重複しうる）。
- 系列 12 の test 本体（161-205）：`timed` を常に比べ，`late` と `applied` を `diverged` に足す。`summary` の「`timed` not compared」を消す。Ceiling の除外をした後も，`late`，`e`（`timed`），ほかの行は比べる。

### 4.2 Mock `tests/timeline.rs`

- `layout_schedule`（164-186）の `LayoutOp::Timed`（180）を `timed_instant` に。`earlier` はそれまでの timed e。`Record.timed`（198，424）はその `e` を読む。loss の後（417-419）の `entry.e >= loss` はそのままで，新しい e で判定する。
- これで `rm_26_the_timeline_against_mockradio`（271）が x3xx/ideal の 1 000 系列で新しい床を固定する。期待値を設計メモの系列 1–4 と同じ関数から作るので，Mock の実装との二重定義にはならない（関数の unit test は §2）。

### 4.3 harness の「観測」の整理

- `timed`：今の `(key, value, at)` を残す（既存の比較）。
- `late`：timed の LATE だけ。`key` は `Some`。
- `applied`：`(key, claimed, at, cancelled)`。`issued`，`note`，`read_back` は正規化で落とす。UHD の `no channel` と issued は同じ「applied」，Mock に行が無い refused は expected でも行なし。Ceiling の command は両方から 1 行除く。

## 5. mutation の行の案

行の `old` はコードと一緒に書く。ここでは変える式と殺す試験だけ。

| # | 変える式（HEAD の式，または新しい式） | 殺す試験 |
|---|---|---|
| M1 | UHD の床の候補を `released`（渡したもの）から台帳（予約したもの）へ変える前の式に戻す（`control.rs:540`） | 系列 1（B の予約は release の前なので A はまだ `released` に無い），系列 4，5（UHD） |
| M2 | `timed_instant` の `e <= now + window` を `e < now + window` にする | 系列 1（3 ms ちょうど），Mock/UHD |
| M3 | `timed_instant` の `window` を 0 にする／引数を無視する | 系列 1/4，Mock x3xx，UHD |
| M4 | `timed_instant` の `lead` を 0 にする | `ur_24_hardware_timed_updates`（+1 ms が +2 ms にならない），`mr_18_hardware_timed_updates`（rows[0] が now+2 ms にならない） |
| M5 | UHD の `pending = held.len() + released.len()`（`control.rs:548`）を残す | 5b（拒否で枠が空く） |
| M6 | 拒否された release を台帳から消す（`give` の失敗で `remove`） | 5，5b |
| M7 | `give`（`control.rs:656-662`）で行を書く（D2 (3) の前の形） | D2 (3) の UHD（9.999 で行がある），系列 10 |
| M8 | `released_at` の fold の `..=(now, u64::MAX)` を `..(now, …)` にする（e ちょうどを外す） | D2 (3) の UHD，系列 10（e=10 の行が無い） |
| M9 | `finish` の S を `core.now()` にする（`streams.stop.0`/`lost_at` でなく） | 9b 後半（cancelled_at が 10.6），9c loss（A が issued になる） |
| M10 | loss の終わりを `e <= S` で数える | 9c loss（A の行ができる） |
| M11 | stop の終わりを `e < S` で数える | 9c stop（A が cancelled になる） |
| M12 | Mock の full を late の前に見る（`lib.rs:1782` を `1788` の前に） | 系列 11 Mock（17 件目が 1 ms で 16 件が 4 ms 間隔。L が出ない） |
| M13 | Mock の床から window の項を消す（`requested.max(earliest)`） | 系列 1/4 Mock |
| M14 | Mock の `profile.timing().release_window_ns` を literal `3_000_000` にする | ideal の系列 1（window 0 で床が出る） |
| M15 | UHD の `description.timing.release_window_ns` を `RELEASE_WINDOW_NS`（3 ms の定数）にする | `ur_24_the_release_window_comes_from_the_description`（10 ms の description で L が出ない） |
| M16 | `timed_instant` の `late` を `asked < now`（または `e > asked`）にする | 系列 1（requested 9.5 ≥ now 7 でも late），11 |
| M17 | `timed_instant` の `earlier` を「e < now のものだけ」にする | 系列 1（A が 10 で床に入らない） |
| M18 | 終わりの行を逆順（または同じ instant を key の逆順）に回す（`cancel_held` の `mem::take` の後） | 系列 10（同一 instant の順），9d（A の no channel と B の cancelled の順） |
| M19 | Mock の `apply_update` の行を予約時に書く | D2 (3) の Mock（9.999 で行がある） |
| M20 | UHD の loss の end で `released_at` を無視する（`held` だけ） | 系列 8（A の行が issued のまま） |

## 6. INFERRED

1. release window 3 ms（`design/18-uhd-radio.md:29,62`，段階 4 の bench で決め直す）。x3xx の 3 ms は測定ではない。
2. stream command（UR-17）が timed command と同じ device queue を使うか（`design/18-uhd-radio.md:254`）。床は stream command を数えない。共有なら，uhd-rx が後から出した stream command が，先に渡した timed command の後ろで効きうる。
3. command queue の深さ 16（`design/18-uhd-radio.md:29,188`，bench）。
4. 装置が timed apply を拒否した後，その command が device の queue の枠を instant まで占めるか。D2 (2) の「拒否も e まで占める」はこちらの規則で，UHD の queue の実測ではない。
5. 喪失の時刻 S は，どの thread が喪失を見つけたかの Module の時計の読みで，hardware の「喪失の瞬間」ではない。D2 (4) の S はこの定義。
6. FakeDevice の timed queue は `effective = max(at, last_command, now)`（`device.rs:1085`）で，spec 18 §2 の「順番どおり・back-pressure・遅れたら到着時」（VERIFIED）を模す。16 より深い queue の実測は無い。
7. `e = S` の順（停止は `e ≤ S` が先，喪失は喪失が先）は Mock の今の順。UHD が同じにするのは設計の選択で，UHD の測定ではない。
8. 装置の refusal の後の投影と configure（UR-24 の「a refusal on release ... leaves the command in every projected configuration」）は今のまま。D2 (2) の枠だけが変わる。
9. 「late なら到着時」（`set_command_time`，`design/18-uhd-radio.md:22`，VERIFIED）の到着の粒度は，FakeDevice の `now` の粒度（1 ms の host poll と device の時間）で模す。

## 7. 設計メモと合わない所・決めていない所

1. **`RELEASE_WINDOW_NS` の使用は 2 か所**：`release`（`control.rs:574`）だけでなく `differential.rs:217`（`release_window_matters` の margin）にもある。D1 の「自分の定数を消す」は両方。
2. **床の lead**：メモは「device lead 2 ms」だが，UHD の profile の `min_timed_command_lead_ns` は 5 ms（lead + delivery allowance）。床の lead は UHD の `DEVICE_LEAD_NS`（2 ms）と Mock の `profile.timing().min_timed_command_lead_ns`（2 ms）で，宣言値 5 ms ではない。メモはこの区別を書いていない。系列 2 が 5 ms を殺す。
3. **`applied` の正規化**：Mock の行は `{key, value, at}`（`design/09-mock-radio.md:179`）で `issued`/`note` が無い。メモの「正規化した `applied` の行」の定義が無い。計画は `(key, claimed, at, cancelled)` に落とし，`issued`/`note`/`read_back` を捨てる。cross-module の系列 4 の A の `no channel` は Mock では「applied の行」になり，注記は UHD だけ。
4. **`Provider::stop`/loss の記録の形**：Mock は `rejected` の `{kind, reason, at: S}`（`design/09-mock-radio.md:179`），UHD は `applied` の `{…, at: e, cancelled, cancelled_at: S}`（UR-30）。メモの「同じ記録」は分類（適用しない，S を記録）で，JSON の形は同じでない。計画は各 Module の形を assert する。
5. **`e < S` で渡されていない command の行**：D2 (4) は「live なものは `e > S` と同じ（cancelled），channel 無しのものは今の結果（no channel）」だが，end の時に「channel が無かった」をどう知るかを決めていない。計画は `targets(dir, chan, e)` を device 呼び出しなしで読み直す案（9d の A）。end の後に plan が変わった場合（refusal が後から booked）は，e の時点の plan と違う。メモは決めていない。
6. **`e == S` で渡されていない command**：メモの「停止なら `e ≤ S`，喪失なら `e < S`」は Ceiling/harness の除外の定義。not-given の `e == S` を `no channel` にするか cancelled にするかは決めていない。9c は *渡した* command の `e == S` だけを固める。
7. **loss で release が lost を返した command**：`give` が `lost` で失敗した command は，held でも given でもない。台帳にはある（D2 (2)）が，end の行を書くかは D2 (4) の「持っているものも渡したものも」の外。既存 3623/3640/3656 は `finish` を呼ばないので今の「行なし」のまま。end を `device_lost` の中で書くなら，この 3 本は変わる。計画は `finish` で台帳の refused/lost を「行なし」にする案（D2 (3) の refused と同じ）。
8. **release で拒否された command が後の end でどうなるか**：D2 (3) は「行を書かない」，D2 (4) は「e が S より後のものは同じに記録する」。拒否は held/given でないので D2 (4) の外と読む（行なし，枠は e まで）。メモは明記していない。5/5b は e までの枠だけを見る。
9. **同じ step の `e == now`**：設計メモの「e がまだ来ていない」は `e > now` とも読めるが，Mock の `step` は Action を `advance(now)` の前に処理するので e == now はまだ適用されておらず，UHD の loop も book の後に release が確定する。この計画は両方 `e >= now` と決めた（§2 D2 (2)）。系列 11 はこの境界を避け，release/step の後で数える。
10. **Mock の VH-8 layer 1**（`crates/ezsdr-mock-radio/tests/timeline.rs`）が §4b に無い：expected の `layout_schedule` に床が無いと，1 000 系列で新しい Mock と食い違う。§4.2 で足す。
11. **RM-4 と capability**：D1 の `radio.timing.release_window_ns` は宣言 key と capability にもなる（`keys`，`channels::capabilities`，`rm_04_the_key_table`）。メモは schema と x3xx/ideal と fixture だけを挙げる。`rm_04` の行と `mr_03`/`ur_09` の capability を足す。
12. **`mr_18_hardware_timed_updates` の full**（`mock_radio.rs:1062-1078`）は D2 (1) の late が先に出るので，今の `LATE_COMMAND == 0` が成り立たない。§4b の書き換え一覧に無い。3.2 で直す。
13. **`ur_24_a_command_waiting_for_a_channel_keeps_a_later_booking_behind_it`**（1755）は D2 (1) で B が late になる。§4b に無い。3.2 で場面を変える。
14. **`Provider::stop` の後の loss の reason**：`finish` が stop の後に来ると `cancel_held` の `why` は "Provider::stop" のまま。loss が先なら S は loss の時刻で，reason を loss にするか stop にするかは決めていない。計画は `cancelled_at` と分類を assert し，reason の文字列は実装に任せる（系列 8 の loss）。
15. **`Sequence::schedule`**（`crates/ezsdr-radio/tests/generator/mod.rs:163-185`）の timed の式と doc は古い。`items()` は timed を捨てるので rx の挙動は変わらないが，共有の規則として残すなら `timed_instant` にそろえる。

## 8. 読んだもの

- 設計メモ：`plan/phase8/38-timed-release-order.md`（§1，§2 D1/D2，§3，§4，§4b，§5）。
- AGENTS.md §9（risky の手順），§7（mutation・時計），37c の学び：`plan/phase8/reviews/37c-opus.md` §6。
- UHD：`crates/ezsdr-radio-uhd/src/provider/control.rs`（`finish` 309-313，`book_timed` 511-561，`release` 569-621，`give` 632-664，`cancel_held` 666-684，`run` 285-305，Action `Stop` 353-384），`profile.rs`（`DEVICE_LEAD_NS` 10，`RELEASE_WINDOW_NS` 16），`provider/core.rs`（`end` 543-553，`record` 715-731，`device_lost_locked` 982-999），`provider/control/differential.rs`（`Record` 47-68，`terms` 73-79，`layout_schedule` 82-109，`release_window_matters` 214-222，`expect` 226-244，`run` 279-400），`provider/test_support.rs`（`Frozen` 143-165，`device_at` 170-176，`ApplyFails` 416-457），`tests/fake.rs`（1627，1655，486），`tests/README.md`。
- Mock：`crates/ezsdr-mock-radio/src/lib.rs`（`handle_update` 1746-1890，`stop_at` 1149-1192，`advance` 2004-2118，`stop` 2461-2480，`record_update` 955-962，`record_rejected_action` 864-871，`next_order` 2350），`src/profile.rs`（`terms` 49-55，`timing` 157-159），`tests/mock_radio.rs`（`Harness` 200-287，`mr_03` 445，`mr_18_hardware_timed_updates` 1042，`mr_25_a_stream_stop_keeps_the_pending_commands` 1082，`mr_20_a_loss_cancels_what_it_left_pending` 1720），`tests/mock_layout.rs`（`Harness` 294-346，`update` 365，`mr_18_a_layout_hardware_timed_change_keeps_its_own_instant` 1097），`tests/timeline.rs`（`layout_schedule` 164-186，`Record` 192-202，271）。
- 語彙：`crates/ezsdr-radio/src/lib.rs`（`keys` 43-72，`TimingEnvelope` 711-728），`src/x3xx.rs`（`timing` 393-402，`ProviderTerms` 414-424），`src/channels.rs`（`timing` 362，理想 380-392，`capabilities` 488-506，`default_stream` 955-957），`src/timeline.rs`（`layout_instant` 391-403），`tests/timeline.rs`（`rm_25_the_layout_instant...` 1448，`TimingEnvelope` literal 111），`tests/radio_model.rs`（`rm_04` 140，`rm_20_schema_freeze` 374），`tests/generator/mod.rs`（`Terms` 126-150，`schedule` 163-185）。
- spec：`design/18-uhd-radio.md`（§2 22,29,62; UR-9 の note 188; UR-14 208; UR-24 252-259; UR-26 297-300; UR-30 317-321），`design/09-mock-radio.md`（MR-18 148-154，MR-25 174-175，MR-27 179，§6 305-306,326-329），`design/07-radio-model.md`（RM-4 61-72，RM-20 202-203，RM-21 230），`schemas/radio/envelope.v1.json`。
- `crates/ezsdr-radio-uhd/tests/README.md`（手動の時計と参加者の規則）。

## まとめ

- 系列 1–12 を 33 行（UHD だけが 5・5b・9d，9b は UHD が 2 行）にし，D2 (3) と 1b（`at` なし）を足した。
- 新しい試験は語彙 1，Mock 15，UHD 19 前後。既存の書き換えは UHD の 14 本（行を e の後へ）と場面 3 本（1755，3706，`mr_18_hardware_timed_updates` の full），ほかに D1 の 4 本（`mr_03`，`ur_09`，`rm_04`，`rm_20`）。
- D1 は `radio.timing.release_window_ns` を profile の値にし，x3xx 3 ms / ideal 0。両 Module が読むことは，x3xx の 3 ms の境界（系列 1/2）と ideal の 0，UHD の description を 10 ms にした試験で見る。ideal の床は 0 で今と同じ。
- D2 は床（予約の履歴），枠（予約から e まで，拒否も），行（e に書く），終わり（S 基準，stop は e ≤ S，loss は e < S，Ceiling は UHD だけ）に分け，それぞれの系列と変異を決めた。
- harness は `release_window_matters` を消し，`Record` に `late` と `applied` を足し，`layout_schedule` を `timed_instant` にし，loss で `finish` を呼び，Ceiling の command を「e < S かつ渡していない」で見分けてその行だけ除く。Mock の `tests/timeline.rs` も同じ床にする（メモの §4b に無い）。
- 決めていないこと：`applied` の正規化（3），`Provider::stop`/loss の行の形（4），not-given の e < S の分類（5），e == S の not-given（6），lost/refused の end（7・8），同じ step の e == now（9），RM-4/capability（11），既存 test の 2 本（12・13）。
