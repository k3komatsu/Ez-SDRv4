STATUS: done

# Phase 8 段階 1b-4d-1b テスト計画 v2（裁定 §0 の分け方で書き直したもの）

対象：`/home/komatsu/local_work/Ez-SDRv4`，HEAD `a52863c`．根拠：`plan-brief.md` §2・§3，`adjudication.md`（優先），`test-plan-1b.md`（裁定で直すところ以外），`sol-plan.log` の末尾．

この段階は **B1（busy）・B2（reading）・B3（D2 の取り下げ）・B5（lock の順序の検査）と，loss・stop の出口**だけ．**B4（D1 の A，`release` の記録だけの枝，S11d，S28 の重ね合わせ）は 1c に送り，この段階では触らない**．したがって `release` の枝・`applied` の行・parity・UR-24・UR-36 は変えない．S11d の 2 本，S28 の重ね合わせ，Sol major 1〜3 は 1c へ（§7）．

## 0. 前の計画からの差分（この段階の分だけ）

| 系列 | 前の計画 | この段階 | 理由 |
|---|---|---|---|
| S9 RX・TX | 残す（形を変える） | **残す**（テスト 2 本） | B1 規則 2．`idle` と cursor の不変を assert に入れる（裁定 §1.6） |
| S11d（正・逆） | 実装する | **1c へ**（1 行） | B4 は 1c（裁定 §0） |
| S13 | 残す（広げる） | **残す**（既存 3 本＋対照 1 本） | `LockSpy` の判定を thread-local の数えに替える（裁定 §1.4）．対照のテストを 1 本足す |
| S13b | 残す | **残す**（1 本） | `Open` を返ってから書く．P376 とは別 |
| S15・S17・S18・S19 | 消す／1a で済み | **変えない**（消したまま） | B1 の busy は from-none だけ．1a のテストが固定している |
| S22 | 残す | **残す**（1 本，`pass()` を切り出す） | 裁定 §1.6：run loop の 1 回分を test から呼ぶ |
| S23 | 形を変える | **形を変える**（1 本，hook で同期） | 裁定 §1.2：50 ms の実時間観測をやめる |
| S24 | 形を変える | **2 本**：booking までの busy の probe（旧 S24）と，brief の逆向きの競合（裁定 §1.3） | idle を強制 true にする helper は使わない |
| S26 | 残す | **残す**（1 本） | open の Ok の後に loss |
| S27 | 形を変える | **2 本**：(a)〜(d) の表，(e) busy を立てた後の loss（多 channel の apply の間） | 裁定 §1.1：先に lost にしてから記帳する形はやめる |
| S28（lock の順） | 残す | **残す**（1 本，should_panic） | `assert_line_free` の hook |
| S28（重ね合わせ） | 残す | **1c へ**（1 行） | B4 に依存（裁定 §0） |
| loss・stop の出口 | S26 の 1 形だけ | **足す**：`reading` 待ちの loss，`reading` 待ちの stop，join が切り離した control と shutdown の読み，configure の channel の間の loss | 裁定 §1.1（Sol 5） |

## 1. 本体の変更（この段階）

1. **`StreamState` に `busy: bool`・`reading: bool`**（`core.rs`）．doc は「busy は uhd-control の from-none が，その stream の device を操作している間」，「reading は uhd-tx の report の読みの間」．
2. **`core::lock` は透明な `Locked<'_, T>` を返す**（`MutexGuard` を包み `Deref`／`DerefMut`／`Drop`）．`#[cfg(test)]` で，この thread が持っている line の guard の数を thread-local に数える（`Core::lock` は `streams`・`rec`・`held`・`rates` のどれにも使うが，B5 が見るのは「それを持ったまま `updates` を取る」ことだけ．`rec → streams` の逆は見ない）．`core::assert_line_free(what)` は深さ 0 を assert する（production では no-op）．
3. **`updates` を取る 5 か所**（`ColdConfig::book`・`changed`・`configure`，`release` の batch，`cancel_held` の batch）の直前に `assert_line_free` を 1 行ずつ．
4. **`Core::configure` は各 channel の `apply` の前に `is_lost` を見る**（裁定 §1.1）．lost なら `DeviceError { lost: true, .. }` を返し，その channel もその後の channel も呼ばない（owner の configure も同じ関数）．
5. **`Core::probe(name, Option<&Streams>)`**（`#[cfg(test)]` の probe の表，production は no-op）．`open_device` の頭（`"open"`），`check_rates` の頭（`"rates"`），`configure` の各 channel の `apply` の前（`"apply"`），`book_cold` の booking の直前（`"cold_booking"`，`Some(&streams)`），from-none の待ちの枝の入口（`"wait_reading"`）．テストの同期と観測にだけ使う．
6. **`Control::book_cold` の from-none**：
   - 判断の section（`idle && planned` を読む section）で `busy = true` にし，`reading` を控える（TX だけ立ちうる）．
   - lock を離し，`reading` なら `await_reading`（`host.sleep(POLL)` の回し．`stop` の印と loss で終える）．
   - その後の device の呼び出しは，`is_lost` と（判断の section で見た）`stop` の印が無いときだけ．open の後・照合の後・configure の前にも `is_lost` を見る．device 呼び出しの**間**に立った stop は次の section（booking の前）で見て，記帳せずに返る（design の Changes もそう書く）．
   - `record_opened` は呼び出しの結果から書く（open が Ok で返った後の loss でも `Open` は残る）．`may_call` が偽，または呼び出しの後で lost／stop なら，`busy` を下ろして**何も記帳せずに**返る（stop の記帳は既存の stop の経路）．
   - 通常の経路は今どおり `record_configured`・`configs.insert`・`book`・`cold_change` を書き，`book` の後に `busy` を下ろす（booking の section の中）．
   - `wait_reading` で待つ間は lock を持たない（`streams` は待ちの前に drop 済み）．
7. **`rx.rs begin`**：plan を読む section の頭で `busy` を見て false を返す（`idle`・`from` を書かない）．
8. **`tx.rs read`**：`bool` を返す．`busy` なら false（plan・`version`・`retarget`・`idle` を書かない）．`passed == None` の early return も true（今の turn は device を呼んでよい）．
9. **`tx.rs begin_reading`／`end_reading`**：`streams` の section で `busy` が下りていれば `reading = true`，`reports` の後に false（from-none が二重に読まないための，`read` の直後の section）．`pass()` に run loop の 1 回分を切り出す：`read` → `begin_reading` → `reports` → `end_reading` → `step`．busy の turn は `sleep(IDLE)` だけ．
10. **`tx.rs shutdown`**：読みごとに `reading` を立てて下ろす．**`busy` は見ない**（join が 1 s で切り離した uhd-control が後で busy を下ろさなくても，shutdown の読みは止まらない）．期待はこれに決める（裁定 §1.1 の最後）．
11. **`test_support` の `LockSpy`**：
    - `enter` の line の検査を `try_lock` から thread-local の深さ（`core::assert_line_free`）に替える．別 thread が正当に line を持っていても，呼んだ thread が持っていなければ通る．
    - `stop_before(name)`／`stop_after(name)` の gate（`reached`／`release` の channel，10 s の番人）．gate 中に同じ方向の別の streamer 呼び出しが入ったら panic．
    - `rates` を数える（`enter("rates")`）．`log()`（中の fake の `calls()`）．
12. **`test_support::probe(core, name, f)`**：`Core::probes` に closure を入れる helper．

## 2. 系列ごと（テスト名・assert・観測・落とす誤り）

観測の書き方は前の計画と同じ．assert は系列ごとに 1〜2 個．gate の thread は `core.device.host().spawn`（participant），終わりは `mpsc` の `recv_timeout(10 s)` で見てから join．sleep で競合を作らない（S23 の待ちは clock の待ちで，gate と probe で同期する）．gate を使う rig は `FakeConfig { clock: ClockMode::Manual, .. }`．

### S9a `ur_25_an_owner_waiting_behind_a_from_none_open_starts_nothing`（RX，`control.rs` の unit）

- 準備：`LockSpy`（Manual）＋`NoLink` の rig．`control` の `radio.rx.channels=0`．`channels_from_none` で B={0,1}（100 ms），0（200 ms）を通す．`stop_before("rx_open")` を入れ，C={0}（300 ms）を `host.spawn` の thread で記帳（`idle=true` は helper が置く）．
- 手順：`reached` を待ち，owner（`owner_rx`）を 1 回 `begin`．`spy.log()` の数と `rx.from` を控える．gate を離して thread を join し，owner をもう 1 回 `begin`．
- assert：(1) 止めている間，`rx.begin()` は false，`spy.log()` は不変，`rx.from` は不変，記録の `idle` は true．(2) gate を離した後，owner の最初の `rx_open`（gate の後を mark してその先だけを数える）は `rx_open 0 [0, 1]`（記録は C の `{0}` なので B を開き直す）．
- 落とす誤り：`begin` から busy の確認を消す（B の `rx_open`／`rx_start` を出し，`from` が進む）．

### S9b 同じ（TX，`tx.rs` の unit）

- 準備：同じ rig（link なし）．`tx_channels_from_none` で B={0,1}（100 ms），0（200 ms）．`stop_before("tx_open")`，C={0}（300 ms）を thread で記帳．
- assert：(1) 止めている間，`tx.read(None)` は false（ここが busy の検査），`tx.at == 0`，`tx.switch.is_none()`，`tx.step()` は false，`spy.log()` は不変．(2) 離した後，gate の後を mark して `tx.read(None)` と `tx.step()` を動かすと，その先の最初の `tx_open` は `tx_open 0 [0, 1]` で，clock を採る．
- 落とす誤り：`read` の busy の確認を消す（switch が B を開き，`at` が進む）．

### S13 `ur_25_a_device_call_passes_while_another_thread_holds_the_line`（`control.rs`，新しい対照）

- 別の thread が `lock(&core.streams)` を持っている間，test の thread から `core.device.rx_open(0, &[0])` を呼ぶ．`LockSpy` は panic しない．assert：`spy.seen("rx_open") == 1`．落とす誤り：`try_lock` の判定のまま（別 thread の line を自分のものと誤る）．
- 既存の `ur_25_the_line_lock_is_free_inside_every_{control,receive,transmit}_device_call` はそのまま通る（assert は変えない）．

### S13b `ur_25_an_open_is_recorded_only_after_the_device_returns`（`control.rs`）

- `probe("open", …)` で，呼び出しの頭の記録 `set` を集める．from-none（`channels_from_none` 1 channel）と `core.open_streamer(Dir::Rx, 0, {0})`（記録を先に `record_open(None)` にする）の 2 回．
- assert：(1) 2 回とも呼び出しの間に `Unknown`．(2) 返った後は `Open({0})`．
- 落とす誤り：device の呼び出しの前に `record_open(Some(set))` を書く実装（mutation 行で作る）．P376（失敗した open が集合を書く行）は呼び出しの後なので，これとは別．

### S22 `ur_25_a_transmit_poll_during_a_control_open_starts_no_streamer_call`（`tx.rs`）

- 準備：`LockSpy` Manual．`control_of`．`stop_before("tx_open")`，1 channel の enable（100 ms）を `host.spawn` の thread で記帳．`reached` を待つ．
- 手順：`tx.read(None)` を 1 回，`tx.pass()` を 1 回（run loop の 1 回分）．
- assert：(1) `read` は false，`pass` の後に `spy.seen("tx_async") == 0`・`spy.log()` 不変・`reading` false．(2) gate を離して join すると `tx_open` 1 回・`apply` あり・`busy` false．
- 落とす誤り：`read` から busy を消す（`reports` が `tx_async` を呼ぶ），`pass` から busy の分岐を消す（`step` が switch を始める）．

### S23 `ur_25_a_from_none_enable_waits_for_a_transmit_report_read`（`tx.rs`）

- 準備：`LockSpy` Manual，link なし．`Tx`（clock なし・0 channel）を `host.spawn("tx", run)`．`stop_before("tx_async")` で最初の読みを止め，`reached` を待つ（この時点で `reading` は true）．
- 同期：`probe("wait_reading", …)` は「待ちの枝に入った」ことを送り，test の release まで block する．
- 手順：control を `host.spawn` で 1 channel の enable（100 ms）．`wait_reading` の reached を待ち，そこで `spy.seen("tx_open") == 0` と `busy` を見る．probe を離し，tx の gate を離し，`TxCmd::Shutdown` を送る．control の終わりを `recv_timeout` で見る．
- assert：(1) 待ちの枝に入った時点で `tx_open` は 0 回，`reading` は true，`busy` は true．(2) 終わった後 `tx_open` は 1 回，`apply` が 1 回以上，`busy`・`reading` は false，記録の `configured` が `(segment.by, segment.version())`．
- 「普通の cold 変更へ落ちなかった」ことは `ready` の時刻では見ない（rig の時計は別）．open・configure と記録の `(seq, 版)` で見る（裁定 §1.2）．
- 落とす誤り：`reading` を待たない（`tx_open` が 1 回出て，`wait_reading` に届かない），`end_reading` を消す（待ちが終わらず control が終わらない），`reading` を見て普通の cold 変更に落とす（`configured` が書かれない）．

### S24a `ur_25_a_from_none_holds_the_streamer_from_its_decision_to_its_booking`（`control.rs`）

- `probe("open"/"rates"/"apply", …)` で呼び出しの頭の `busy` を，`probe("cold_booking", …)` で booking の直前の `busy` を集める．`channels_from_none` 1 channel．
- assert：(1) 4 か所とも `busy` は true．(2) 戻った後 `busy` は false，記録の `configured` は `(seq, 版)`．
- 落とす誤り：configure の後・booking の前に busy を下ろす実装（booking の probe が false），busy を立てない実装（S9 も落とす）．

### S24b `ur_25_an_enable_behind_an_owner_s_open_is_a_cold_change`（`control.rs`，brief の逆向き）

- 準備：`LockSpy` Manual＋`NoLink`，`control(&core, &[Dir::Rx])`（RX line は T0 から）．`stop_before("rx_open")`．owner（`Rx::new(…, None, None)`）を `host.spawn` で `begin` 1 回．`reached` を待つ（この時点で `idle` は false，owner は open の中）．
- 手順：deliberately idle をいじらない．test の thread で `control.book_cold(radio.rx.channels=1, 100 ms)`．
- assert：(1) control の前後で `spy.log()` は不変（open も configure もしない），記録の `idle` は booking の後も false．(2) plan の segment が 1 つ増え（普通の cold 変更として記帳），gate を離すと owner が自分の open をする（`rx_open 0 [0]` とその記録）．owner の最初の segment は設定を持たない（`by` が None で `owner_configure` が configure できない）ので，`rx_start` は観測しない．
- 落とす誤り：from-none の条件から `idle` を外す（control が device を呼ぶ）．

### S26 `ur_25_a_lost_stream_starts_no_rate_check_apply_or_booking_after_the_call`（`control.rs`）

- 準備：Manual rig，`stop_after("rx_open")`（Ok が返った後に止める）．control を `host.spawn` で 1 channel の enable（100 ms）．`reached` を待ち，test の thread で `core.device_lost("test")`．gate を離して join．
- assert：(1) `spy.seen("rates") == 0`・`spy.seen("apply") == 0`・`plan_of(Rx)` は空．(2) `busy` は false，記録の `set` は `Open({0})`，`configured` は None．
- 落とす誤り：open の後に `is_lost` を見ない（`rates` が出る），早く戻る経路で busy を下ろさない．

### S27 `ur_25_a_from_none_leaves_the_stream_unbusy_on_every_path`（`control.rs`，表のテスト）

- (a) 成功（素の fake），(b) open の失敗（`FakeFault::RxOpenFails`），(c) 照合の拒否（先に `core.check_rates(Dir::Rx, 99)` を拒否させる），(d) configure の失敗（`FakeFault::RxApplyFails`）．各場合で `channels_from_none` を 1 channel で 1 回通す．
- assert：各場合で (1) `busy` false・`reading` false．(2) 記録は (a) `Open`+`configured`，(b)(c) `Unknown`，(d) `Open`+`configured None`．
- 落とす誤り：失敗の枝で busy を下ろさない（`busy` が残る）．

### S27e `ur_29_a_lost_device_stops_a_from_none_enable_between_channels`（`control.rs`）

- 準備：Manual rig，2 channel の enable．`stop_after("apply")`（1 channel 目の apply が返った後）．control を `host.spawn` で記帳．`reached` を待ち，`device_lost` を記録．gate を離して join．
- assert：(1) `spy.seen("apply") == 1`（2 channel 目を呼ばない）．(2) `busy` は false，記録の `configured` は None，`plan_of(Rx)` は空．
- 落とす誤り：`Core::configure` の各 channel の前の `is_lost` を消す（fake は mark_lost の後も apply に成功するので 2 回目が出る）．

### S-stop `ur_25_a_from_none_enable_ends_its_wait_at_the_provider_stop`（`tx.rs`）

- 準備：Manual `LockSpy`，link なし．tx を `host.spawn("tx", run)`，`stop_before("tx_async")` で `reading` を立てる．`wait_reading` の probe で wait の枝を捕まえる．
- 手順：control を thread で enable（100 ms）．probe の reached を待ち，`Provider::stop` と同じく `streams.stop = Some(..)` と `streams.end(..)` を記帳してから probe を離す．control の終わりを待つ．tx の gate を離し，Shutdown を送って join．
- assert：(1) `spy.seen("tx_open") == 0`・`spy.seen("apply") == 0`・`busy` false．(2) control の thread が 10 s 以内に終わる（待ちが stop で終わる）．
- 落とす誤り：待ちの loop から `stop` を見ない（終わらない），早い戻りで busy を下ろさない．

### S-loss-wait `ur_29_a_lost_device_ends_a_from_none_enable_s_wait_for_a_report_read`（`tx.rs`）

- 準備・手順は S-stop と同じ（`stop_before("tx_async")`，`wait_reading` の probe，control は `host.spawn`）．stop の代わりに `core.device_lost("test")` を記録し，probe を離す．**tx の gate は離さない**（`reading` は true のまま）．
- assert：(1) control が 10 s 以内に終わる（loss が待ちを終える），`spy.seen("tx_open") == 0`・`apply` 0・`busy` false．(2) その後 gate を離し，Shutdown で tx を止める．
- 落とす誤り：`await_reading` の loop から `is_lost` を消す（読みが終わらず control が終わらない）．裁定 §1.1 の「`reading` を待っている間の loss」．

### S-detach `ur_25_a_detached_control_does_not_hold_the_shutdown_s_read`（`tx.rs`）

- 準備：Manual `LockSpy`，link なし．`Tx` の `busy` を手で true にする．`tx.shutdown()` を呼ぶ．
- assert：`spy.seen("tx_async") >= 1`（読みは busy を見ない），`reading` は false．
- 落とす誤り：shutdown の読みに busy の確認を入れる（1 回も読まない）．

### S28 `ur_25_a_configuration_taken_under_the_line_lock_is_refused`（`control.rs`）

- `lock(&core.streams)` を持ったまま `plan[1].configure(&core, Dir::Rx)` を呼ぶ．`#[should_panic(expected = "under the line lock")]`．
- 落とす誤り：`assert_line_free` を消す．

## 3. 既存のテストで変わるもの

1. `ur_25_the_line_lock_is_free_inside_every_*_device_call`（3 本）：判定だけが thread-local になる．assert は変えない．
2. `ur_25_a_cold_change_takes_the_line_lock_once_for_its_instant_and_its_booking`：from-none の `streams` の取得は 2 回のまま（busy を立てる section と booking の section）．assert は変えない．実装後に再確認する．
3. `ur_14_booking_does_not_wait_for_a_future_cold_switch`：`read` の戻り値は無視される．`reading` は test の thread では立たない（`pass` を通らない）．
4. `ur_28_a_poll_after_a_from_none_open_names_no_burst_of_the_replaced_queue`・`ur_28_a_from_none_open_replaces_the_report_queue`：`tx.read(None)` の戻り値は無視．from-none の待ちは `reading` が false なので入らない．
5. `tests/fake.rs` の Direct の系列：`reading` は run loop の `pass` だけが立てる．`tx.read` を直接呼ぶ test は待たない．ワークスペース全体を流して確かめる．
6. `P343`・`P351` の `old` は from-none の書き換えで動くので当て直す．`P374` は `release` を触らないので**変えない**．

## 4. 大きさ（この段階の分だけ数え直し）

| ファイル | 足すもの | 行数（概算） |
|---|---|---|
| `core.rs` | `Locked`・thread-local の数え・`assert_line_free`・`probe`・2 つの欄・`configure` の lost の検査 | 100 |
| `control.rs` | from-none の組み直し（busy・待ち・loss・stop・早い戻り・booking の probe）と `assert_line_free` の 5 か所 | 95 |
| `rx.rs` | `begin` の busy | 6 |
| `tx.rs` | `read` の bool と busy，`begin_reading`／`end_reading`，`pass`，`shutdown` | 55 |
| `test_support.rs` | `LockSpy` の gate・`log`・`rates`・thread-local の検査，`probe` helper | 90 |
| テスト | S9 RX 75，S9 TX 75，S13 対照 30，S13b 55，S22 65，S23 120，S24a 55，S24b 70，S26 65，S27 110，S27e 55，S-stop 80，S-detach 35，S28 15 | 約 905 |
| `design/18` | UR-3・UR-17・UR-23・UR-25・Changes | 25 |
| 合計 | | **約 1,270 行**（1,500 行以内） |

不確かさ：S23・S-stop の thread の組み方（+60），`LockSpy` の gate（+30）．

## 5. mutation の行（P382 以降）

| 規則 | 変える場所 | 殺すテスト |
|---|---|---|
| RX の owner は busy を見る | `rx.rs begin` | S9a |
| TX の owner は busy を見る | `tx.rs read` | S9b・S22 |
| from-none は busy を立てる | `control.rs book_cold` | S9a・S9b・S24a |
| from-none は booking の後に busy を下ろす | 同じ | S24a（booking の probe） |
| from-none は `reading` を待つ | 同じ | S23 |
| TX は読みの後に `reading` を下ろす | `tx.rs pass` | S23 |
| `reading` 待ちの loss（`is_lost` を見る） | `control.rs await_reading` | S-loss-wait |
| 早い戻り（stop・loss，device の前）で busy を下ろす | `control.rs book_cold` | S-stop・S23（loss 側は S27e の後半） |
| 早い戻り（open・照合の後の lost／stop）で busy を下ろす | 同じ | S26 |
| open の後の lost を見る | 同じ | S26（`rates` が 0） |
| configure の各 channel の前の lost | `core.rs configure` | S27e |
| shutdown の読みは busy を見ない | `tx.rs shutdown` | S-detach |
| lock の順（line を持ったまま `updates`） | `control.rs owner_configure`（mutation で guard を configure まで生かす） | `ur_24_a_record_only_release_after_a_from_none_configure_makes_the_owner_configure_again` |
| line を持ったまま `configurations` を configure | `assert_line_free` を消す | S28 |

`P343`・`P351` は書き換えた行に当て直す．`P374`・`P375`〜`P381` は変えない．

## 6. `design/18-uhd-radio.md`（裁定 §1.7，DA-1）

1. UR-3：streamer の担当の文に「enable from no stream の間だけ uhd-control がその stream を持ち，booking，または booking せずに終える section で返す（UR-25）」を足す．
2. UR-25：短い別の段落「Ownership.」— busy の持ち主と返す section（booking と，booking しない出口），owner が見つけたら device を呼ばない（shutdown の読みは UR-16 の例外），`reading` と clock の待ち（busy loop にしない；loss と stop も終える），lock の順序（`updates` → `core::lock` の state → …）と，検査の範囲（`updates` を state の下で取ることだけを見る；`rec → streams` の逆は見ない），configure の各 channel の前の loss（UR-29）．
3. UR-17・UR-23：UR-25 を引く 1 文ずつ（規則を 2 度書かない）．
4. Changes の行（UR-3・UR-17・UR-23・UR-25）．UR-24・UR-36・UR-17 の他の文は変えない（B4 は 1c）．
5. 偽になる文：UR-3 の「each streamer has one owning thread at a time」（from-none の間の担当）．UR-25 の「one lock that no thread holds across a device call」は，待ちと section の追加で補う．

## 7. 1c へ送るもの（この段階では実装しない）

- S11d の正・逆（D1 の A，`release` の記録だけの枝，`released`，`applied` の行）．
- S28 の重ね合わせ `ur_25_a_release_overlapping_a_configure_neither_deadlocks_nor_mismatches`．
- Sol major 1（owner が始めた後の系列），major 2（configure の成功記録の公開の窓），major 3（queue の会計と過ぎた origin）．
- UR-24・UR-36 の文（計画 §9.9 の宿題も含む）．

## 8. 手順

1. `core.rs` → `control.rs` → `rx.rs` → `tx.rs` → `test_support.rs` の順．テストと mutation の行を一緒に書く．
2. `cargo test --workspace --no-fail-fast`，`cargo clippy`（2 つ），`rules.py --base HEAD`．
3. mutation は `remote_mutate.py` で，足した行・変えた行と，コードかテストが変わった既存の行（P343・P351）を流す．
4. `design/18` を同じ変更に入れる．

## 9. セルフレビュー 1 回目（DeepSeek）の扱い

- **P2（`reading` 待ちの loss のテストが無い）**：採る．S-loss-wait を足し，mutation の行 P395 で `await_reading` の `is_lost` を検査する．計画 §2 に追記済み．
- **P2（design の「booking で返す」の全称）**：採る．UR-3・UR-25・Changes を「booking，または booking せずに enable を終える section」に直した．
- **P2（design の待ちの出口）**：採る．loss と `Provider::stop` も待ちを終えることを UR-25 の段落に書いた．
- **P2（shutdown の読みの例外）**：採る．UR-25 の段落に UR-16 の例外として書いた．
- **P3（S24b の assert）**：採る．plan の segment 数と booking 後の `idle` を assert に足した．owner の最初の segment は設定を持たないので `rx_start` は観測しない（計画を直した）．
- **P3（S9 の「最初」と `last()`）**：採る．gate の後を mark し，owner の最初の open を assert する形に直した（計画も直した）．
- **P3（S26 のテストが UR-25 の Test 行に無い）**：採る．UR-25 の Test 行に足した．
- **P3（`assert_line_free` は `core::lock` 全体を数える）**：採る．実装の doc を「`streams`・`rec`・`held`・`rates` の guard」に直し，design も「`core::lock` の state」に揃えた（B5 の範囲は裁定 §1.5 のとおり）．
- **P3（「One stream's device calls have one owner at a time」）**：採る．「uhd-control の最初の device call は read の後」と限定する文に直した．
- **P3（configure の各 channel の loss が design に無い）**：採る．UR-25 の段落に UR-29 として書いた．
- **P3（`rx.rs:176` の整形）**：採る．直した．

## 10. セルフレビュー 2 回目の扱い

- **P2-1（stop が device call の間で再検査されない）**：実装は裁定 §1.1 の stop の項（待ちの間の stop）と，判断の section の stop を扱う．**呼び出しの間**に立った stop は再検査しない——その時点で in-flight の呼び出しは止められず，次の section で見て記帳せずに返る（`busy` は下りる）．design の Changes 行と計画 §1 をそのとおりに狭めた（裁定の要求を超える規則を design に書かない）．
- **P2-2（「何も記帳せずに返る」が未観測）**：採る．RM-25 は End の後の item を plan から落とすので `plan_of` では見えない．`rx_colds`／`tx_colds`（line の `Kind::Cold` の数）を足し，S26・S27e・S-stop・S-loss-wait・S-stop-before-calls・S-loss-after-rates で 0 を assert する．
- **P3-1（`reading` を待たない stop の分岐）**：採る．`ur_25_an_enable_from_no_stream_after_the_provider_s_stop_calls_no_device`（判断の section で stop を見る）と mutation の行 P396．
- **P3-2（照合後の loss）**：採る．`ur_29_a_lost_stream_between_the_rate_check_and_the_configure_applies_nothing`（`probe("apply")` の回数 0）と mutation の行 P397．`Core::configure` の per-channel の検査（S27e）が本体で，from-none の検査はその前段であることをテストで固定する．
- **P3-3（UR-3 の shutdown の例外）**：採る．UR-3 の文に「shutdown の report の読み（UR-25，UR-16）」を引いた．
- **P3-4（UR-25 の `reading` が `reports` 全体より狭い）**：採る．「`reports` の呼び出し（report が起こす end-of-burst の send を含む）」に直した．

## 11. セルフレビュー 3 回目の扱い

- **P2-1（`read` と `begin_reading` の間の窓）**：窓そのものを消す方を採る．`pass` が plan の読みと `reading` の設定を**同じ `streams` の section** で行う（`plan_section` に切り出す）．`read` は test と `do_switch` のための経路として残し，busy を見る．「窓を通すテスト」は要らなくなり，`ur_25_a_transmit_turn_that_meets_the_enable_s_mark_…` は消した．mutation の P398 は `pass` の busy の検査を消す行に置き換え（殺すテストは S22），`J50`・`J59` は `plan_section` の行に当て直した．
- **P3-1（呼び出しの間に立った stop）**：テストと design を足した．`ur_26_an_enable_from_no_stream_a_stop_recorded_after_its_open_books_nothing`（`stopped` を確認し，`rx_colds == 0`）と mutation の P400，UR-25 の段落と Changes の文（「その呼び出しの間に立った stop は，booking する section で enable を終え，何も記帳しない」）．
- **P3-2（早期 return の「記帳しない」の gate の行）**：mutation の P399（条件を `if false` にする）．
- **P3-3（`await_reading` の stop，shutdown の `end_reading` の行）**：P401・P402．
- 新しいテストの名は UR-25 の Test 行に足した（`ur_26_an_enable_from_no_stream_a_stop_recorded_after_its_open_books_nothing`，`ur_29_a_lost_stream_between_the_rate_check_and_the_configure_applies_nothing`，`ur_25_an_enable_from_no_stream_after_the_provider_s_stop_calls_no_device`）．
