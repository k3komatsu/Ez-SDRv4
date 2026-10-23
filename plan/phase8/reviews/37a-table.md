# 設計メモ 37 対応表（#69）

> orchestrator の注（2026-10-23）：37a は表 A・B-1・B-5・B-6，37b は B-2・B-3．37a の実装は 4 系列（#2，#5，#8，#16）を表の Spec の案から unit test に変え，Sol のコードレビューの後に #4 と #9 も unit test に移した（[37a-opus](37a-opus.md)）．

- 対象: `plan/phase8/37-test-thread-timing.md`（owner 決定 2026-10-23，§5）と指示書 `impl-brief.md` §3。
- HEAD: `814747a`（`docs: design note 37 …`）。作業ツリーはこの表を書く時点できれい。
- 方法: `fake.rs`，`uhd.rs`，`protocol.rs`，`tx.rs`，`device.rs`，`test_support.rs`，`core.rs`，`host.rs`，`design/18-uhd-radio.md`，両 `mutations.json` を読んだ。下に引用した grep を流した。**cargo のテストも mutation も流していない**（表は読んだ内容から作った）。
- 分類はメモ §3 の規則 (1)〜(4):
  - (1) Spec で開始前に予約する（予約の瞬間を文書に固定する）。
  - (2) 時刻が動いても成り立つ assert にする（読んだ時刻を判定の基準にしない）。
  - (3) 遅れが判定に関わらない予約は late policy `send_asap_and_flag` にする（Session の verb では `start_repeat`）。
  - (4) 時刻の窓が判定そのものなら，制御された時計の unit test にする。
- 「十分前に予約する」は確率を下げるだけで (2) ではない（メモ §3）。表 B で「margin だけ」と書いた行は，規則上は未分類のままである。

## 表 A: `ur_23` の 5 本と `ur_21_a_descheduled_…`

### A-0 共通の rig と駆動（T1，T2，T3，T4，T5）

`tx.rs` の tests に置く。使う部品は既存のものだけ（指示書 §2.2）。

```rust
let (core, device, time) = manual_rig();                 // tx.rs:2253-2257（Manual, command_queue 64）
let _frozen = Frozen::of(&device);                       // test_support.rs:143-155
device_at(&core, &time, &device, 0);                     // test_support.rs:170-176
device.tx_open(0, &[0]).unwrap();
device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
let clock = core.register(Dir::Tx, 200, 0).unwrap();     // origin 0, 200 root ticks/sample
let (_to_tx, cmds) = std::sync::mpsc::channel();
let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
let ms = |k: i64| core.ticks(k * 1_000_000);             // 200 000 root ticks/ms
```

- `Frozen` は manual clock の参加者で，テストの間ずっと busy（M3）。`device_at` は Core の時計を進め，device の時計を同じ tick に置き，両者が等しいことを assert する。呼び出しは単調増加のみ。
- `device_at` と `Frozen` の同時使用の既存例: `ur_24_a_transmit_release_without_a_live_channel_reaches_the_pending_segment`（`tx.rs:2259-2266`）。
- `tx_open` → `apply(rate 1e6)` → `register` の既存例: `ur_21_first_dispatch_rechecks_all_late_policies`（`tx.rs:1496-1502`）。
- `held` は `tx.rs:929-939`。**`held()` の波形は全 sample `[0.25, 0.0]` で一様**なので，B を `held()` で作ると B20（tail と payload の順）が値の比較で生き残る。B は必ず A と違う値にする。
- 駆動の算術: A は 40 ms 開始，30 000 samples（1 MS/s）→ 70 ms 終わり。`IN_FLIGHT_WINDOW_NS = 10 ms`（`profile.rs:14`），`DEVICE_LEAD_NS = 2 ms`（`:10`），chunk = `min(window / n / 4, block_len)` = 2 000（`window/n/4 = 2 500`，`block_len` 2 000 で clamp，`tx.rs:385`）。
- `for k in (31..=59).step_by(2) { device_at(..., ms(k)); assert!(tx.step()); }` の 15 回で送信 #0〜#14。`ms(30)` では窓がちょうど 10 ms（`<` ではない）なので dispatch しないため 31 から。15 回目（#14，68 000〜70 000）は `open_ended` が真で eob，最後の 1 sample を hold する（`tx.rs:410-418`）→ `tx.open == None`，`tx.continues_at == Some(70_000)`，`tx.tail.is_some()`。
- 送信番号は 0 起点で，`n >= 1` の `tx_send` を数える（空の eob も n=1）。#15 が tail，#16 が B payload，#17 が eob（T1/T3 の分割による）。

### A-1 `ur_23_a_burst_booked_at_a_sent_burst_s_end_continues_it`（`fake.rs:3040`，B08/B16/B20）

| 項目 | 内容 |
|---|---|
| 今の性質 | A の最後の buffer が出た後（in-flight window の中，5 ms lead の外）に A の終わりへ予約した B が捨てられず，A の device burst を続ける。31 000 samples が一度だけ渡り，device の送信は A ++ B（A の保持した最後の sample が B の先頭の前）。TimeError なし，Underflow なし，unended 0（`fake.rs:3040-3076`，`design/18:458`）。 |
| unit test 名（案） | 同じ名前 `ur_23_a_burst_booked_at_a_sent_burst_s_end_continues_it`（`tx.rs` の tests）。 |
| 系列 | A-0 の rig。A = `held(&clock, 40_000, 30_000, DropAndFlag)`，`lock(&core.held).insert((clock.domain, 40_000))`，`tx.command(TxCmd::Burst(a))`（既存例 `tx.rs:1385-1388`）。A-0 の 15 回で #0〜#14。状態を assert。B は `Held { samples: Arc::new(vec![second]), ..held(&clock, 70_000, 1_000, DropAndFlag) }`，`second[i] = [-0.25 - i as f32 / 4_000.0, 0.125]`。`insert((clock.domain, 70_000))`，`command(B)`，`device_at(ms(62))`，`step()` → tail #15（n=1，at なし，sob/eob なし）+ payload #16（n=999）。`device_at(ms(69))`，`step()` → eob #17（B の保持した最後の sample，n=1）。 |
| 作れる根拠 | `manual_rig` `tx.rs:2253`，`Frozen`/`device_at` `test_support.rs:143/170`，`held` `tx.rs:929`，`tx.command(TxCmd::Burst(...))` `tx.rs:1385-1388`，`Tx::step` の分岐 `tx.rs:332-366`（`continues_at` と held が同じ k → 継続）。`transmitted`/`unended_bursts` は `FakeDevice` の pub fn（`device.rs:722/740`）で，unit test から呼べる。`Frozen` + `device_at` の同時例は `tx.rs:2265` だが，**「faults なしの manual rig で 15 回駆動し，tail を保持した状態で B を予約する」系列そのものの既存例はない**（部品は全部既存，組み合わせは INFERRED）。 |
| assert / 観測 | `tx.open == None && tx.continues_at == None && tx.tail == None`。`device.transmitted(clock.instant(40_000), 31_000) == A ++ B`（各 `Some`）。`device.calls()` の `eob=true` が 1，`sob=true` が 1。`device.unended_bursts() == 0`。`lock(&core.rec).rejected` に `UR-22` / `UR-21` がない。`tx.reports(Wall::ZERO)` の後 `core.rec.device_async` に `TimeError` がない。handed の合計 31 000（`n=` の和）。 |
| 誤った実装と本番コード | B08 `hold = false` → A が 70 000 で device eob，B の timed sob が `t <= cursor`（`device.rs:1589`）で drop。B は device に入らず，`transmitted[30_000]` が None になり，device が TimeError を報告する（eob は 2）。B16 tail を捨てる → handed 30 999，`transmitted[29_999]` が B[0] ≠ A[29 999]。B20 payload を tail の前に送る → B が 69 999 から始まり `transmitted[29_999]` = B[0]。**B20 は B が A と違う値のときだけ kill する**。本番: `tx.rs:404-418`（`continuing`/`device_sob`/`hold`/`device_eob`），`:415-433`（tail の送信），`:463`（tail の保存），`:497-503`（eob と `continues_at`），`device.rs:1589`。 |
| fake 側 | **消す**。代わり: 新しい unit test（この表 A-1），および fake に残る窓の要らない形として `ur_23_a_burst_ends_at_the_next_bursts_start`（`fake.rs:1523`，予約済みの継続で 1 device burst）と `ur_23_two_bursts_back_to_back_loop_back_whole`（`fake.rs:3269`，両方先に予約した end-to-end loopback）。spec 18 の `:248`，`:458` は名前が残るので変更不要。 |
| mutation 行 | B08/B16/B20（`plan/phase7/tools/mutations.json`）。`filter` は同じ名前のまま，`test_bin: fake` → `lib`。`old`/`new` は本番コードなので変更なし。kill: B08 は `transmitted == A ++ B`（30 000 番目）と TimeError なし，B16 は handed 31 000 と `transmitted[29_999]`，B20 は `transmitted == A ++ B`（B を A と違う波形にする）。 |

### A-2 `ur_23_a_stop_while_the_device_burst_waits_for_a_continuation_ends_it`（`fake.rs:3078`，B10）

| 項目 | 内容 |
|---|---|
| 今の性質 | device burst が継続を待って開いているとき（A の最後の buffer の後，終わりの 7 ms 前）に `Stop` すると，1 つの eob で閉じ，走り切る前に終わる。Underflow なし（`fake.rs:3078-3101`，`design/18:459`）。 |
| unit test 名（案） | 同じ名前。 |
| 系列 | A-0 の rig と駆動（A 40 000，30 000；15 回で `continues_at = Some(70_000)`，tail あり）。`device_at(ms(63))`，`tx.stop_now("test stop", CancelReason::Stop)`（既存例 `tx.rs:1399`，`:1487`）。`stop_now` → `end_open(true)` → `close_device_burst(tail)` が eob n=1 を 69 999 に送る。 |
| 作れる根拠 | `stop_now` は既存（`tx.rs:695`，test から `tx.rs:1399/1487`）。`end_open`/`close_device_burst` は `tx.rs:523-552`。A-0 と同じ rig。**「A の 15 回駆動後に Stop」の既存例はない**（部品は既存，組み合わせは INFERRED）。 |
| assert / 観測 | `device.transmitted(clock.instant(40_000), 30_000) == A`（B なし）。`eob=true` が 1（A の最後の sample，n=1）。`tx.open == None && tx.continues_at == None && tx.tail == None`。`unended_bursts() == 0`。`core.rec.device_async` に Underflow がない（`tx.reports(Wall::ZERO)` 後）。 |
| 誤った実装と本番コード | B10 `device_open = self.open.as_ref().is_some_and(...)` → open が None で continuing が Some のとき false → eob を送らず tail を捨てる。`eob` が 0，`transmitted[29_999]` が None（A の最後の sample がない）。本番: `tx.rs:523-538`（`end_open` の `device_open`），`:542-552`（`close_device_burst`）。 |
| fake 側 | **消す**。代わり: 新しい unit test，および fake に残る Stop の窓の要らない形 `ur_23_a_stop_ends_the_burst_within_the_in_flight_window`（`fake.rs:1557`，repeat を in-flight window 内で Stop し，記録の終わりを窓で見る）。spec 18 `:248`，`:459` は名前が残る。 |
| mutation 行 | B10。`filter` 同じ，`test_bin: fake` → `lib`。kill: eob が 0 / `transmitted[29_999]` が None。 |

### A-3 `ur_23_a_burst_one_sample_after_a_burst_is_played`（`fake.rs:3145`，mutation 行なし）

| 項目 | 内容 |
|---|---|
| 今の性質 | A の device burst は c で終わり，c+1 に空の eob の 0 sample を足さない。だから c+1 に予約した B が drop されずに再生される（A の最後の buffer の前と後の両方の予約で）。A の device burst はちょうど 30 000 samples（`fake.rs:3145-3224`，`design/18:461`）。 |
| unit test 名（案） | 同じ名前（fake 側の書き換えと同じ名前になる。別 binary なので Rust では共存できる。spec 18 の参照は 1 つに見えるので minor）。 |
| 系列 | A-0 の rig と駆動。B を 70 001 に `device_at(ms(62))` で予約（A と違う波形 1 000 samples）。`step()` 1 回目: `continues_at = Some(70_000)` と held 70 001 が違う → `end_open(true)` が A の tail eob を 69 999 に送る（cursor 70 000）。`step()` 2 回目: B を dispatch し，timed sob 70 001，n=999 + tail。`device_at(ms(69) + 20_000)`（69.1 ms，`continues_at = Some(71_001)` まで 1.901 ms ≤ 2 ms），`step()` → B の tail eob を 71 000 に送る。 |
| 作れる根拠 | A-0 と同じ。`step()` を同じ `device_at` の瞬間に 2 回呼ぶのは既存の `tx.rs` tests でも普通（例 `tx.rs:1385-1393` の連続 `step`）。**この c+1 の窓そのものの unit test は既存にない**（新規，INFERRED）。 |
| assert / 観測 | `device.transmitted(clock.instant(40_000), 31_001)`: `[..30_000] == A`，`[30_000] == None`（c の 1 sample の gap），`[30_001..] == B`。`eob=true` が 2（A の tail と B の tail），`sob=true` が 2（A と B）。`unended_bursts() == 0`。`tx.reports(Wall::ZERO)` の後 `device_async` に TimeError/Underflow がない。`tx.open/continues_at/tail` は None。 |
| 誤った実装と本番コード | A の終わりを空の eob で閉じる（`close_device_burst` が tail でなく empty を送る）→ 70 000 に 0 sample が入り cursor 70 001。B の timed sob 70 001 は `t <= cursor`（`device.rs:1589`）で drop。`transmitted[30_001]` が None，TimeError。本番: `tx.rs:523-552`（tail 優先），`device.rs:1589`。**この性質を殺す mutation 行は今ない**。行を足すなら `close_device_burst` の `Some(tail) => ...` を `None => ...` にする形だが，他の多くのテストも殺すので，この段階では行を足さない（orchestrator の判断）。 |
| fake 側 | **その場で書き換える**（消さない）: Spec schedule で A を radio offset 10 000，30 000 samples，B を 40 001 に置く（両方開始前に文書で予約）。`late_booking` の 2 分岐と「drop されたらやり直す」loop を消す。A の最後の buffer の後に予約する分岐は unit test（A-1，A-2，A-4，A-5）が持つ。assert は既存 T3 の形で `tx_sends` を eob ごとに 2 つの device burst に分ける: burst 0 の先頭が `sob && at == Some(a_root)`（A は 10 000）で合計 30 000，burst 1 の先頭が `sob && at == Some(a_end + n)`（B は 40 001）で合計 1 000。A が 40 000 に 0 sample を足せば B の timed sob が drop されて 2 burst にならない。`unended_bursts() == 0`，TimeError/Underflow なし。Spec schedule の既存例: `ur_23_two_bursts_back_to_back_loop_back_whole`（`fake.rs:3289-3298`）。 |
| mutation 行 | なし（T3 を filter にする行は両リストにない）。 |

### A-4 `ur_23_a_continuation_whose_held_sample_is_not_taken_is_abandoned`（`fake.rs:3402`，B28/B31/B32）

| 項目 | 内容 |
|---|---|
| 今の性質 | B が A の device burst を続けるとき，A の保持した sample の送信が何も取らない（`StalledSend(15)`）か失敗する（`FailSend(15)`）と，B は再生されず，device burst は A の 29 999 samples の後に空の eob（0 sample 1 つ）で終わる。`rejected` が原因を名指す（`fake.rs:3402-3436`，`design/18:466`）。 |
| unit test 名（案） | 同じ名前。 |
| 系列 | rig は A-0 と同じだが faults 付き: `rig_with(FakeConfig { clock: ClockMode::Manual, command_queue: 64, faults: vec![FakeFault::StalledSend(15)], ..FakeConfig::default() }, Vec::new())`（`FakeFault::FailSend(15)` も別 case）。`Frozen::of`，`device_at(0)`，`tx_open`/`apply`/`register`/`Tx::new`。A 40 000，30 000，15 回駆動。B 70 000，1 000 を `ms(62)` で予約。`device_at(ms(62))`，`step()` → tail #15 が `Ok(0)`（stalled）または `Err`（failed）→ `reject_note`/`device_failed` → `abandon(true)` → 空 eob #16 を 69 999 に送る。 |
| 作れる根拠 | `StalledSend`/`FailSend` は既存の `FakeFault`（`device.rs:324-331`，T4 が同じ番号を使う）。`rig_with(FakeConfig{ faults, .. })` の既存例は `tx.rs:1643`，`:2127`（ただし default の実時間 rig）。**manual clock + faults + `Frozen` + `device_at` の組み合わせの既存例はない**（`manual_rig` は faults を取らない）。`manual_rig` と同じ config に faults を足す形（`clock: Manual, command_queue: 64`）で作れる見込みだが，**INFERRED**。orchestrator の判断が要る（新しい helper は作らない）。 |
| assert / 観測 | `device.transmitted(clock.instant(40_000), 30_001)`: `[..29_999] == A[..29_999]`，`[29_999] == Some([0.0, 0.0])`，`[30_000] == None`。`eob=true` が 1。`unended_bursts() == 0`。`lock(&core.rec).rejected` に `UR-22: a send did not complete within 1 s`（Stalled）または `fake: the send failed`（Fail）。`device.calls()` に `stalled`/`failed` があり，case に到達したことを観測する。 |
| 誤った実装と本番コード | B28 tail の結果を `.map(|_| 1)` → Ok(0)/Err が Ok(1) になり，B が 69 999 から再生。`[29_999]` が B[0] ≠ `[0,0]`，`UR-22` の reason もない。B31 Err 分岐から `return self.abandon(true)` を落とす → 同じく B が 69 999 から。B32 `if false` で `FailSend` を消す → fault が起きず，B が正しい位置 70 000 から再生され，`rejected` に `fake: the send failed` がない。本番: `tx.rs:415-433`（tail の `match`），`:443-462`（`Ok(sent)`/`Err` の abandon），`:554-569`（`abandon`/`close_device_burst`），`device.rs:1545-1560`（fault の `Ok(0)`/`Err`）。 |
| fake 側 | **消す**。代わり: 新しい unit test，および `ur_22_a_send_cut_short_ends_the_device_burst_after_what_it_took`（`fake.rs:3326`，窓の要らない Spec 形に直す。cut/fail した send の後の空 eob を見る）。spec 18 `:248`，`:466` は名前が残る。 |
| mutation 行 | B28/B31/B32。`filter` 同じ，`test_bin: fake` → `lib`。kill: B28 は `transmitted[29_999] == [0,0]` と reason，B31 は `transmitted[29_999] == [0,0]`，B32 は reason と `failed` の call。 |

### A-5 `ur_23_a_continuation_s_first_send_that_takes_nothing_ends_the_device_burst`（`fake.rs:3438`，B29）

| 項目 | 内容 |
|---|---|
| 今の性質 | B が A の device burst を続け，tail は取られたが B の最初の payload send が何も取らない（`StalledSend(16)`）。device burst はまだ開いているので，A の最後の sample の後に空の eob で終わる。timed sob が burst の中にない。Underflow なし（`fake.rs:3438-3470`，`design/18:467`）。 |
| unit test 名（案） | 同じ名前。**推奨: tx.rs へ移す**（下の代替案も可）。 |
| 系列 | A-4 と同じ faults rig（`StalledSend(16)`）。A 40 000，30 000，15 回。B 70 000，1 000 を `ms(62)` で予約。`device_at(ms(62))`，`step()` → tail #15 は Ok(1)（69 999 に A の最後の sample），payload #16 は Ok(0)（stalled）→ `Ok(sent)` の `sent < sending` 分岐 → `reject_note("UR-22…")` → `abandon(was_open = true)` → 空 eob #17 を 70 000 に送る。 |
| 作れる根拠 | A-4 と同じ（faults rig は INFERRED）。`StalledSend(16)` の番号は T5 の fake test が同じ番号を使っており，A-0 の駆動（#0〜#14，tail #15）と一致する。 |
| assert / 観測 | `device.transmitted(clock.instant(40_000), 30_002)`: `[..30_000] == A`，`[30_000] == Some([0.0, 0.0])`，`[30_001] == None`。`eob=true` が 1。`unended_bursts() == 0`。`core.rec.device_async` に Underflow なし。`rejected` に `UR-22…`。`device.calls()` に `stalled`。 |
| 誤った実装と本番コード | B29 `let was_open = !first;` → B の payload の `Ok(0)` で `abandon(false)`，eob を送らない。`eob` 0，`transmitted[30_000]` が None。本番: `tx.rs:464`（`was_open`），`:443-462`（`Ok(sent)` の分岐），`:554-569`（`abandon`）。 |
| fake 側 | **推奨: 消して tx.rs の unit test に移す**。fake の側に残る「継続の最初の send が 0」の窓の要らない形はないので，消すなら新しい unit test が代わり。**代替案**: fake.rs に残し，Spec schedule で A と B を両方先に予約する形に書き換える（A の最後の chunk は `ends_at_held` で tail なし，B の payload が #15 になるので `StalledSend(15)`）。この場合 B29 は `test_bin: fake` のまま。どちらも B29 を kill する。**orchestrator の判断が要る**（B29 の `test_bin` が決まる）。 |
| mutation 行 | B29。推奨（unit 移設）: `filter` 同じ，`test_bin: fake` → `lib`。代替（fake に残す）: `test_bin: fake` のまま。kill: eob が 1 / `transmitted[30_000] == [0,0]`。 |

### A-6 `ur_21_a_descheduled_timed_send_is_on_time_on_the_manual_clock_and_late_on_the_real_one`（`fake.rs:3744`，J295，**移さない**）

| 項目 | 内容 |
|---|---|
| 今の性質 | `tx_send` の中で 200 ms だけ uhd-tx を deschedule する。manual clock は uhd-tx が busy の間止まるので burst は on time。実時間ではデバイスが late を報告する。`tx_send` に入ったことと，manual で `LateAtDevice` なし・実時間であり，manual の burst 記録が 1 つであることを見る（`fake.rs:3743-3766`，`design/18:384`）。 |
| 直し方 | `:3750` の `send_at(&mut run, "send", Some(at), &tone(10_000))` を `send_at(&mut run, "start_repeat", Some(at), &tone(10_000))` に変える。`start_repeat` は `send_asap_and_flag`（`crates/ezsdr-radio/src/lib.rs:270-276`）なので，試験のスレッドが遅れて予約が遅れても drop されず，動かされるだけ。`entered.recv_timeout`（`:3752`）と `assert_eq!(errors.any(LateAtDevice), late)`（`:3759`），manual の `bursts == 1`（`:3761`）は残す。 |
| 作れる根拠 | `start_repeat` は既存の verb で，隣の `ur_26_the_shutdown_s_last_burst_ack_waits_on_the_device_s_clock`（`fake.rs:3776`）が同じ形で使う。manual clock の deschedule は既存の `Descheduled`（`fake.rs` の `Deschedule::TxSend`）。**移設しない**ので `manual_rig` は不要。 |
| assert / 観測 | 既存のまま: `entered` が 5 s 以内に返る（`tx_send` に入った），manual の `LateAtDevice` が `false`，real が `true`，manual の `bursts(&manifest).len() == 1`。 |
| 誤った実装と本番コード | J295（`host.rs` の `if false { return st.vnow; }` と deadline の arm）→ manual clock が busy の間に実時間で進み，manual で `LateAtDevice` が `true` になる。`late == false` の assert が落ちる。本番: `host.rs` の manual clock（M3/M5），`tx.rs:384-504`，`device.rs` の late 規則。 |
| fake 側 | その場で直す（上記）。テスト名は変えない。spec 18 `:384` の参照はそのまま。 |
| mutation 行 | J295（`plan/maintenance/tools/mutations.json`）。`filter` 同じ，**`test_bin: fake` のまま**（メモ §3 の再レビュー minor 1）。kill: manual の `LateAtDevice` の assert。 |

### 表 A のまとめ（mutation の当て直し）

| 行 | 今の filter | 今の test_bin | 後 | kill する assert |
|---|---|---|---|---|
| B08 | `ur_23_a_burst_booked_at_a_sent_burst_s_end_continues_it` | fake | lib | `transmitted == A ++ B`（30 000 番目），TimeError なし |
| B16 | 同上 | fake | lib | handed 31 000，`transmitted[29_999]` |
| B20 | 同上 | fake | lib | `transmitted == A ++ B`（B を A と違う波形に） |
| B10 | `ur_23_a_stop_while_the_device_burst_waits_for_a_continuation_ends_it` | fake | lib | eob 1，`transmitted[29_999]` |
| B28 | `ur_23_a_continuation_whose_held_sample_is_not_taken_is_abandoned` | fake | lib | `transmitted[29_999] == [0,0]`，reason |
| B31 | 同上 | fake | lib | `transmitted[29_999] == [0,0]` |
| B32 | 同上 | fake | lib | reason `fake: the send failed`，`failed` の call |
| B29 | `ur_23_a_continuation_s_first_send_that_takes_nothing_ends_the_device_burst` | fake | lib（推奨）／fake（代替） | eob 1，`transmitted[30_000] == [0,0]` |
| J295 | `ur_21_a_descheduled_timed_send_is_on_time_on_the_manual_clock_and_late_on_the_real_one` | fake | fake | manual の `LateAtDevice` の assert |
| （T3） | — | — | — | 行なし |

- 行の `old`/`new` は本番コードの文字列なので，この段階（本番コードを変えない）では変更しない。移動したテストは `filter` が同じ名前で `lib` に当たる。移動後に各行がちょうど 1 回一致することを確かめる。
- 名前を変える・消す場合は `filter` だけでなく `old`/`new` と `design/18-uhd-radio.md` の `:248`，`:458-467` も追う（メモ §3 の再レビュー minor 1）。この表の案は名前を変えないので，spec 18 の変更は T3 の同名 2 本の注記だけ（下記）。

## 表 B: 掃除の一覧

### B-0 流した grep（そのまま）

```sh
grep -nE "run\.now\(\)|device\.time_now\(\)|device\.host\(\)\.now\(\)" crates/ezsdr-radio-uhd/tests/fake.rs
# 40 hits: 258,263,307,308,455,1301,1561,2137,2141,2254,2276,2286,2394,2422,2465,2468,2474,2477,
# 2553,2600,2605,2612,2661,2671,2860,2877,2897,2918,2940,2943,2947,3052,3089,3168,3178,3179,
# 3364,3387,3418,3450
# （1301 は helper tx_at，1561 は Stop の read，2860 は helper reports_until，3052 以降は 6 本のテスト。）

grep -nE "\.now\(\)" crates/ezsdr-acceptance/tests/uhd.rs
# 71,194,229,234

grep -nE "Instant::now|elapsed\(\)|time_now|host\(\)\.now|sleep" crates/ezsdr-server/tests/protocol.rs
# 0 hits（protocol.rs は時刻を status の応答としてしか読まない）

grep -n "status(&mut server)\|status(server)\|capture_at(&mut server" crates/ezsdr-server/tests/protocol.rs
# 347,419,775,805,806,835,837,992,1068,1092,1096,1107,1110
```

補助（call site を追うため）:

```sh
grep -nE "\b(send|send_at|tx_at)\(&mut run|\b(send|send_at|tx_at)\(&run" crates/ezsdr-radio-uhd/tests/fake.rs
grep -nE "fn (ur_23|ur_21|ur_22|ur_28|ur_33)_" crates/ezsdr-radio-uhd/tests/fake.rs
```

### B-1 `fake.rs` の掃除（raw と Direct を除く。表 A の T3/T6 は分類のため再掲）

| # | ファイル:行 | テスト | 読む → 次の操作/assert | 時間が進むと | 分類 | 理由 | mutation |
|---|---|---|---|---|---|---|---|
| 1 | `fake.rs:1334` | `ur_21_txburst_refusals` | `tx_at(ms(500))`（1342）→ 同じ target に 2 回 `send_at`（1343-1344）→ `rejections` を見る（1350-1356） | ~490 ms 遅れると 2 回目が booking で late/drop になり，「held burst already has this start」が出ない | (1)（分割） | 壊れた波形・非有限の拒否は receipt 時で時刻に依らない（無害）。同じ start の部分だけ Spec で 2 本を固定 offset に置く。**要確認** | J197 |
| 2 | `fake.rs:1358` | `ur_21_a_burst_before_start_up_is_refused` | `send(ms(60))`（1364）→ 拒否を見る（1366） | ~58 ms 遅れると device の start-up の後になり，拒否されない | (1) | burst を start-up より前の固定 offset に置く Spec にする。目標が `now+60 ms` なので，遅れで start-up を越える | なし |
| 3 | `fake.rs:1370` | `ur_21_late_policies_on_the_device_lead` | `send(ms(1))`，`start_repeat(ms(0.5))`，`send(ms(20))`（1381-1388）→ `outcomes` | 20 ms の 3 本目が ~18 ms 遅れると late → Drop になり，`outcomes[2..]` が SendAsap でなくなる | (3) | `:1388` の 20 ms を `start_repeat` にすると，遅れても SendAsap に動くだけで assert が成り立つ（20 ms の `send` が on time であることを主張し続けるなら (1) の Spec が元の意図を保つ。上のコメントも直す） | なし |
| 4 | `fake.rs:1414` | `ur_21_a_burst_inside_the_in_flight_window_is_late` | `start_repeat(ms(20))`，`start_repeat(ms(5))`，`send(ms(5))`（1417-1421）→ `outcomes == [SendAsap, Drop]` | 20 ms の 1 本目が ~18 ms 遅れると SendAsap/Drop になり，先頭の期待が崩れる | (2) | 5 ms の 2 本（SendAsap と Drop）だけを判定し，1 本目は on time でも late でも許す。**要確認** | U36,U44,R06,B04 |
| 5 | `fake.rs:1432` | `ur_21_a_burst_before_its_clock_s_origin_is_late` | `start_repeat(ms(5))`（1437）→ `[SendAsap]` と target 0 | ~5 ms 遅れると new clock の origin の後になり，on time（SendAsap なし） | (1) | cold change と burst を Spec の固定 offset に置く（burst は新 clock の origin の前） | U46 |
| 6 | `fake.rs:1494` | `ur_22_a_single_burst_ends_with_end_of_burst` | `send(ms(20))`（1496）→ eob と record | ~18 ms 遅れると booking で drop，eob/record なし | (1) | Spec で 20 ms に置く | なし |
| 7 | `fake.rs:1523` | `ur_23_a_burst_ends_at_the_next_bursts_start` | `start_repeat(ms(10))`，`send(ms(20))`（1525-1527） | どちらかが遅れると drop/移動で 2 burst の前提が崩れる | (1) | Spec で 10 ms と 20 ms に置く | U17,B03 |
| 8 | `fake.rs:1545` | `ur_23_bursts_go_out_in_target_order` | `send(ms(80))`，`send(ms(40))`（1547-1548） | 40 ms が ~38 ms 遅れると drop，starts が 1 つ | (1) | Spec で 80 ms と 40 ms に置く | U18 |
| 9 | `fake.rs:1557` | `ur_23_a_stop_ends_the_burst_within_the_in_flight_window` | `run.now()`（1561）→ `Stop`（1562）→ `end <= stop_at + 30 ms`（1569） | Stop の提出が遅れると record の終わりが `stop_at` より後に伸び，30 ms を越える | (2) | `stop_at` を Stop の `LogEntry.time`（提出の receipt）から取る。送信は `start_repeat` なので booking の遅れは移動で吸収される | U19 |
| 10 | `fake.rs:1902` | `ur_25_a_cold_transmit_change_cancels_the_held_bursts` | `send(ms(300))`（1905）→ cold change → held の取消 | ~298 ms 遅れると burst が drop，held がない | (1) | Spec で burst を 300 ms，cold change を固定 offset に置く（または `start_repeat`）。**要確認** | R07 |
| 11 | `fake.rs:2102` | `ur_28_the_device_reports_a_late_burst` | `tx_at(ms(8))`（2106）→ `send` → 1 つの `LateAtDevice`，`target == at` | ~6 ms 遅れると booking で drop，late report なし | (3) | `start_repeat` にし，`late[0].target == bursts(&manifest)[0].target` を見る（動いた target）。`SlowSend` は毎 send 20 ms なので repeat の後続も遅いが，timed sob は最初の 1 つだけ | U28 |
| 12 | `fake.rs:3103` | `ur_23_a_burst_alone_is_ended_before_it_runs_dry` | `send(ms(20))`（3107）→ BurstAck/Underflow | ~18 ms 遅れると drop，BurstAck なし | (1) | Spec で 20 ms に置く | B09 |
| 13 | `fake.rs:3145` | `ur_23_a_burst_one_sample_after_a_burst_is_played` | `run.now()`（3168）→ B の予約 → `device.host().now()` の 2 s watchdog（3178-3179） | A の最後の buffer の後 8 ms の窓を外す。retry は確率を下げるだけ | (4) | unit test（A-3）に移し，fake 側は Spec の A 10 000 / B 40 001 に書き換える（A-3 の fake 側） | なし |
| 14 | `fake.rs:3309` | `ur_23_a_one_sample_burst_booked_ahead_starts_at_its_time` | `tx_at(ms(40))`（3314）→ 1 sample burst | ~38 ms 遅れると drop | (1) | Spec で 40 ms に置く | B21 |
| 15 | `fake.rs:3326` | `ur_22_a_send_cut_short_ends_the_device_burst_after_what_it_took` | `tx_at(ms(40))`（3337）→ send cut/fail | ~38 ms 遅れると A が drop，cut する send がない | (1) | Spec で 40 ms に置く（3 つの fault case はそのまま） | B18,B19,B25 |
| 16 | `fake.rs:3374` | `ur_22_a_first_send_cut_short_keeps_the_reports_paired` | `tx_at(ms(40))`（3382）→ `a_root - 8 ms` まで待って B（3387-3391） | A が drop，または B の予約が遅れて late の対象が変わる | (1) | Spec で A を 40 ms，B を A+500 に置く（両方先に予約） | B26,B27 |
| 17 | `fake.rs:3744` | `ur_21_a_descheduled_timed_send_is_on_time_on_the_manual_clock_and_late_on_the_real_one` | `tx_at(ms(30))`（3749）→ `send`（3750） | ~28 ms 遅れると `send` が drop，`tx_send` に入らない | (3) | `start_repeat`（A-6）。manual/real の判定はそのまま | J295 |
| 18 | `fake.rs:3796` | `ur_33_a_shutdown_with_a_report_read_that_fails_at_once_still_joins_uhd_tx` | `tx_at(ms(20))`（3798）→ `send`（3799）→ shutdown | ~18 ms 遅れると burst が drop，shutdown の ACK 経路が変わらないとしても前提が崩れる | (3) | `start_repeat`。test の目的（read が即失敗しても join する）は burst の有無に依らないが，予約を落とさない | J317 |

### B-2 `fake.rs` の raw device テスト（13 本，`src/device.rs` へ移す）

`fake.rs` の Review N の節（`:2848` の注記）は「raw receive の case は `src/device.rs` の unit test で，全スレッドが参加者」と既に書いている。TX の raw も同じ形に移す。移す先の部品は既存: `manual(config)`（`device.rs:1787`），`participant(&device, f)`（`device.rs:1799`），`ms`（`:1782`），`set_time`（`:779`），`transmitted`/`unended_bursts`（`:722/740`）。テスト名は変えない。分類は (4)（device の時刻の窓が判定そのもの）。

| ファイル:行 | テスト | 読む → 次 | 時間が進むと | 行 |
|---|---|---|---|---|
| `fake.rs:2249` | `ur_33_the_fake_device_loops_back_what_it_transmits` | `time_now + ms(20)`（2254）→ timed send | start が過去になり drop | なし |
| `fake.rs:2269` | `ur_33_scripted_faults_fire` | `rx_start(now + ms(1))`（2276），`time_now` の lost（2286） | start が過去，fault の順序が変わる | なし |
| `fake.rs:2383` | `ur_33_two_receive_streamers_keep_their_own_channels` | `now + ms(20)`（2394），`stopped = now`（2422） | start が過去，report の読みが遅れる | P204,P214,P215,P216 |
| `fake.rs:2457` | `ur_33_two_transmit_streamers_keep_their_own_reports` | `t0/t1 = (now/200 + 20 000)*200`（2465/2474）→ 100 samples 待つ | 窓を外す | P205,P217 |
| `fake.rs:2545` | `ur_33_an_empty_channel_list_is_channel_zero` | `rx_start(now + ms(1))`（2553） | start が過去 | P230,P231 |
| `fake.rs:2591` | `ur_33_a_timed_command_follows_its_front_end_across_a_spec` | `apply(at=now+ms(50))`，`at=now+ms(5)`（2600/2605/2612） | at が過去 | P232 |
| `fake.rs:2653` | `ur_33_the_antenna_is_read_back_after_an_untimed_change` | `at = now + ms(5)`（2661） | at が過去 | P207,P213 |
| `fake.rs:2666` | `ur_33_the_fake_device_keeps_the_devices_queues` | `now = time_now`（2671）→ queue の操作 | queue の順序が実時間に依る | なし |
| `fake.rs:2868` | `ur_33_an_empty_end_of_burst_is_one_zero_sample` | `t0 = (now/200 + 20 000)*200`（2877） | 窓を外す | B17 |
| `fake.rs:2889` | `ur_33_a_timed_start_at_the_previous_burst_s_end_is_late` | `t0`（2897） | 窓を外す | なし |
| `fake.rs:2911` | `ur_33_a_burst_that_runs_dry_underflows` | `t0 = (now/200 + 5 000)*200`（2918） | 窓を外す | なし |
| `fake.rs:2929` | `ur_33_a_burst_resumed_after_an_underflow_plays_late` | `t0`（2940），`dry + 5 ms` 待ち（2943），`resumed = now`（2947） | 窓を外す | なし |
| `fake.rs:3357` | `ur_33_a_burst_s_acknowledgement_comes_once_its_end_has_played` | `t0 = (now/200 + 20 000)*200`（3364），`reports_until` | 窓を外す | B30 |

- helper `reports_until`（`fake.rs:2858`）は `time_now <= tick + 200_000` の poll。テストと一緒に `device.rs` へ移すか，`participant` の中で読む形にする。
- 移すと `test_bin: fake` → `lib`。行の `filter`（テスト名）と `old`/`new` は変えない。行のない 6 本（loops_back，scripted_faults，keeps_devices_queues，timed_start_at_previous，runs_dry，resumed_after）はそのまま。
- `fake.rs:2703` の `ur_33_a_real_fake_s_time_starts_at_its_own_creation` は **移さない**: 前後の読みで bracket する設計で，テストのスレッドの遅れに依らない（R12）。zero は `device.rs:1762` の `..._counts_from_its_own_creation` が pin する。無害。

### B-3 `fake.rs` の `Direct` 系（**要確認: 掃除の範囲に含めるか**）

`Direct::armed_with` も `fake(config)` を作るので manual clock だが，`Direct::new`/`with_links` は `FakeConfig::default()`（Real clock）を使う。テストのスレッドは参加者でなく，時刻を読んでから `update`/`settle` する。窓の判定は `control.rs` の unit test（例: `fake.rs:1648-1650` の注記が指す `ur_24_the_device_queue_order_is_the_effective_order`）へ移す前例がある。行はテストの関数頭を指し，読む行は「読む」列に書く。

| ファイル:行 | テスト | 読む → 次 | 時間が進むと | 分類 | 理由 | 行 |
|---|---|---|---|---|---|---|
| `fake.rs:1591` | `ur_24_hardware_timed_updates` | `direct.now()`（1594）→ `now+1 ms`（late）, `now+40 ms`（timed）, `settle(80 ms)` | ~39 ms 遅れると 40 ms が late，`LATE_COMMAND` が 2 / `at` がずれる | (4) | 1 ms の device lead の境界が判定。制御された時計の unit test に移す（または 1 ms の境界だけ unit，40 ms は log の at で (2)） | U20,U21 |
| `fake.rs:1623` | `ur_24_a_far_future_update_does_not_delay_a_nearer_one` | `direct.now()`（1629）→ far 500 ms, near 20 ms | ~18 ms 遅れると near が late，`applied` が空 | (1) or (4) | Spec で far 500 ms / near 20 ms に置くか，順序と effective を制御された時計で見る | U37 |
| `fake.rs:1726` | `ur_25_a_stream_silent_before_e1_switches_before_e2` | `direct.now() < silent - 150 ms` の guard（1740），`until = ... - direct.now()`（1742） | 起動が 150 ms より遅いと guard が落ちる | (4) | 130 ms の silence 窓が判定。制御された時計へ | B01,B02 |
| `fake.rs:1780` | `ur_25_rx_channels_from_zero_starts_at_its_instant` | `at = now + 300 ms`（1784）→ cold change | 300 ms 遅れると cold change が late，`origin < at+200` が崩れ得る | (1) or (4) | start の instant を固定（Spec/start_instant）にするか制御された時計へ | なし |
| `fake.rs:3238` | `ur_25_a_cold_change_booked_anywhere_in_a_long_receive_call_is_on_time` | 読まない。`settle(200 + phase*step)` → cold change `None` | 遅れても booking が遅れるだけ。assert は log の `booked_at` 基準 | 無害（(2) 相当） | 判定が `booked_at` と switch の origin の関係で，テストのスレッドの時刻を基準にしない | B15,B22 |
| `fake.rs:4241` | `ur_25_a_timed_update_at_the_cold_change_s_instant_reaches_the_switch` | `at = now + 300 ms`（4248）→ timed/cold を同じ at に | 300 ms 遅れると両方 late，`LATE_COMMAND` が非空 | (4) | 「同じ瞬間」が判定。制御された時計へ（`control.rs` の前例） | P264 |

### B-4 `fake.rs` の無害な隙間（判定に関わらない）

| ファイル:行 | テスト/helper | 読む → 次 | なぜ無害か | 行 |
|---|---|---|---|---|
| `fake.rs:258/263` | `ur_07_the_authority_paces_to_the_device` | `host.now()` の前後 → 30 ms の schedule | 実時間の Authority の診断。schedule は `time.now + 30 ms` で，assert は device の時計が 29 ms 以上進んだこと（下限）。テストのスレッドの遅れは下限を緩めるだけ | なし |
| `fake.rs:269` | `ur_07_schedule_wakes_a_waiting_next_wakeup` | `time.now + 1 ms` の schedule → 20 回の median lateness | 実時間の wake の遅れそのものが測定対象（plan 30 §4.4 の例外）。(1)〜(4) にすると測れない | なし |
| `fake.rs:295/307` | `ur_07_now_is_monotonic_across_a_reanchor` | `host.now + 400 ms` の loop，`time.now` の単調性 | 実時間の re-anchor の診断。遅れは loop の回数を減らすだけで単調性を破らない | なし |
| `fake.rs:455` | `ur_07_now_tracks_the_device_while_no_call_runs` | `time.now` と `device.time_now` を 2 s 後に比較 | 前後を同じ瞬間に読む bracket。遅れは両方に同じに効く | なし |
| `fake.rs:1467` | `ur_22_repeat_is_continuous_across_the_wrap` | Spec の offset 1 000 | 予約が文書で固定。テストのスレッドは時刻を読まない | なし |
| `fake.rs:1509` | `timing_precondition_failure_reports_evidence_and_still_fails` | `send(ms(1))`（1512）→ Drop を期待 | 1 ms は device lead 2 ms の中。遅れても late のまま | なし |
| `fake.rs:1857` | `ur_25_enabling_tx_applies_the_configuration` | `start_repeat(ms(20))` → capture +5 ms | `start_repeat` は SendAsap。遅れれば動くだけで，capture は repeat のどの offset でもよい | なし |
| `fake.rs:1920` | `ur_26_stop_actions` | `start_repeat(ms(10))` → Stop | SendAsap。遅れれば動くだけ。Stop は booking の後 | なし |
| `fake.rs:2130/2137` | `ur_29_a_silent_stream_is_a_lost_device` | `host.now()` の前後 → `elapsed < 7 s` | 装置の沈黙の wall watchdog。判定は DEVICE_LOST で，7 s は上限 | なし |
| `fake.rs:2703` | `ur_33_a_real_fake_s_time_starts_at_its_own_creation` | `before/after` の bracket，`time_now` | bracket の設計（R12）。zero は `device.rs:1762` が pin | なし |
| `fake.rs:2858` | `reports_until` helper | `time_now <= tick + 200 000` の poll | report は queue に残る。遅れは loop の回数を減らすだけ。raw 移設のときに `participant` 側へ | なし |
| `fake.rs:3269` | `ur_23_two_bursts_back_to_back_loop_back_whole` | Spec の offset 10 000/11 000 | 文書で固定 | なし |

### B-5 `crates/ezsdr-acceptance/tests/uhd.rs`

| ファイル:行 | テスト | 読む → 次 | 時間が進むと | 分類 | 理由 |
|---|---|---|---|---|---|
| `uhd.rs:71` | helper `wait_capture` | `run.now()` → `wait_for(..., now + 3 s)` | 待ちの残りが短くなる。capture は 100 ms 以内に書かれるので，3 s の stall がなければ見つかる | 無害（watchdog） | 3 s は判定でなく上限。`from` の event index は submit の前 |
| `uhd.rs:194` | `uhd_57_the_session_loopback_captures_what_it_transmits` | `now + 100 ms` の capture | capture が遅れても，repeat の content を offset で探すので判定は変わらない | 無害 | 連続 repeat の任意 offset |
| `uhd.rs:229` | `uhd_61_02_capture_starts_at_the_requested_sample_index` | `now + 10 ms` の first block，`index = (now-origin)/n + 100 000` | `index` の計算後に遅れると capture が過去になり，`first` が `index` より後 | (2) | `:240` を `first >= index && end-first == 100` に緩める。厳密な「要求した index で始まる」は `uhd_59_receive_capture_starts_at_the_requested_index`（`:136-145`，Spec）が pin する |
| `uhd.rs:136-145` | `uhd_59_receive_capture_starts_at_the_requested_index` | Spec の timed capture 50 000 | 文書で固定 | 無害 | — |

### B-6 `crates/ezsdr-server/tests/protocol.rs`

直接の時刻読みはない（grep 0 hits）。`status` の応答を読んでから capture/wait する系列だけを扱う。

| ファイル:行 | テスト/helper | 読む → 次 | 時間が進むと | 分類 | 理由 |
|---|---|---|---|---|---|
| `protocol.rs:347` | `ea_09_the_default_profile_loops_back` | `status` → `Advance(60 ms)` → capture | capture が遅れても repeat の content を offset で探す | 無害 | 判定は content |
| `protocol.rs:419` | `ea_12_time_and_events` | `status` → `Advance(1 ms)` → `Status` | served time の比較のみ | 無害 | 時刻の値そのもの |
| `protocol.rs:775` | helper `capture_at` | `status` → submit capture → `wait_for(3 s)` | 待ちの残りが短くなる | 無害（watchdog） | 3 s は上限。event index は submit の前 |
| `protocol.rs:805` | `ea_12_a_duration_is_whole_root_ticks` | `status` → capture +20 000 000 ticks → `WaitFor` | 待ちの horizon が変わるが，判定は horizon >= 1e9 | 無害 | duration の判定 |
| `protocol.rs:835` | `ea_07_a_session_on_the_fake_device` | `status` → capture +20 ms → content | capture が遅れても repeat の offset 探索 | 無害 | 判定は content |
| `protocol.rs:992` | `ea_07_a_lost_device_ends_the_session_with_its_manifest` | `status` → `WaitFor(5 s)` | 待ちの残りが短くなる | 無害 | DEVICE_LOST の判定。5 s は上限 |
| `protocol.rs:1068` | `ea_12_status_carries_the_root_rate` | `status` → `WaitFor(3 ms)` | 待ちの残りが短くなる | 無害 | 判定は root_rate |
| `protocol.rs:1089-1101` | `ea_17_a_capture_ahead_starts_at_its_instant` | `status`（1092）→ `at = now + 50 ms` → `capture_at`（1096）→ `first >= at && first < at+n`（1100） | 50 ms 遅れると capture が `at` より後に始まり，`first < at+n` が落ちる | (2) | `:1100` を `first >= at` にし，長さ 1 000 を assert する。厳密な開始は `uhd_59`（Spec）が pin する |
| `protocol.rs:1104-1116` | `ea_17_a_capture_at_a_passed_instant_says_where_it_started` | `at = now - 20 ms` → `first > at` + 長さ 1 000 | すでに tolerant | 無害（(2) 済み） | 遅れても `first > at` と長さ 1 000 が成り立つ |

### B-7 mutation の当て直し（表 B）

| 行 | 今の filter | 今の test_bin | 後 | どこで kill するか |
|---|---|---|---|---|
| P204/P214/P215/P216 | `ur_33_two_receive_streamers_keep_their_own_channels` | fake | lib | device.rs の移設先の assert（reports/streams） |
| P205/P217 | `ur_33_two_transmit_streamers_keep_their_own_reports` | fake | lib | 同上 |
| P207/P213 | `ur_33_the_antenna_is_read_back_after_an_untimed_change` | fake | lib | read_back の assert |
| P230/P231 | `ur_33_an_empty_channel_list_is_channel_zero` | fake | lib | channel 0 の assert |
| P232 | `ur_33_a_timed_command_follows_its_front_end_across_a_spec` | fake | lib | apply の front-end の assert |
| B17 | `ur_33_an_empty_end_of_burst_is_one_zero_sample` | fake | lib | 0 sample と late の assert |
| B30 | `ur_33_a_burst_s_acknowledgement_comes_once_its_end_has_played` | fake | lib | report の順の assert |
| J197 | `ur_21_txburst_refusals` | fake | fake（その場） | 分割後も同じ name なら filter そのまま。old/new は本番コードで不変 |
| U17/B03 | `ur_23_a_burst_ends_at_the_next_bursts_start` | fake | fake（Spec 書き換え） | 1 device burst / 1 sob / 1 eob |
| U18 | `ur_23_bursts_go_out_in_target_order` | fake | fake（Spec） | starts の順 |
| U19 | `ur_23_a_stop_ends_the_burst_within_the_in_flight_window` | fake | fake（(2)） | record.end と bound |
| U36/U44/R06/B04 | `ur_21_a_burst_inside_the_in_flight_window_is_late` | fake | fake（(2)） | outcomes の 5 ms の 2 本 |
| U46 | `ur_21_a_burst_before_its_clock_s_origin_is_late` | fake | fake（Spec） | SendAsap と target 0 |
| R07 | `ur_25_a_cold_transmit_change_cancels_the_held_bursts` | fake | fake（Spec） | rejected の理由と bursts 空 |
| U28 | `ur_28_the_device_reports_a_late_burst` | fake | fake（start_repeat） | late 1 つと target |
| B09 | `ur_23_a_burst_alone_is_ended_before_it_runs_dry` | fake | fake（Spec） | BurstAck/Underflow なし |
| B21 | `ur_23_a_one_sample_burst_booked_ahead_starts_at_its_time` | fake | fake（Spec） | 1 sample の at |
| B18/B19/B25 | `ur_22_a_send_cut_short_ends_the_device_burst_after_what_it_took` | fake | fake（Spec） | took と空 eob |
| B26/B27 | `ur_22_a_first_send_cut_short_keeps_the_reports_paired` | fake | fake（Spec） | late の target |
| J295 | `ur_21_a_descheduled_…` | fake | fake | manual の LateAtDevice |
| J317 | `ur_33_a_shutdown_with_a_report_read_that_fails_at_once_still_joins_uhd_tx` | fake | fake（start_repeat） | join と tx_errors |
| U20/U21 | `ur_24_hardware_timed_updates` | fake | fake（その場）／lib（control.rs へ移す場合） | late 1 つと at |
| U37 | `ur_24_a_far_future_update_does_not_delay_a_nearer_one` | fake | fake（(1)）／lib（(4) control.rs） | applied の 1 つ |
| B01/B02 | `ur_25_a_stream_silent_before_e1_switches_before_e2` | fake | lib（control.rs へ移す場合） | switch と late なし |
| P264 | `ur_25_a_timed_update_at_the_cold_change_s_instant_reaches_the_switch` | fake | lib（control.rs へ移す場合） | switched/timed の 1 つ |
| B15/B22 | `ur_25_a_cold_change_booked_anywhere_in_a_long_receive_call_is_on_time` | fake | fake（無害） | booked_at の floor |

- 掃除でテストの中身だけを変える行は `test_bin` も `filter` も変えない（本番コードの `old`/`new` は不変）。移動する行だけ `lib` にする。
- どの行も、編集後に `old`/`new` がちょうど 1 回一致することを確かめる（AGENTS.md §7）。

### B-8 spec 18 の参照

- `design/18-uhd-radio.md:248`（`*Test:*` の列挙）と `:458-467`（表の行）は，T1/T2/T3/T4/T5/T6 の**名前**を参照している。表 A の案は名前を変えないので，原則として変更不要。
- 唯一の注記: T3 は fake 側を同じ名前で書き換え，tx.rs にも同じ名前の unit test を置くので，`design/18:461` / `:248` の名前が 2 つのテストを指す。名前を分けるなら（例: unit 側に `_on_the_manual_clock` を付ける），`:248` と `:461` を直す。**orchestrator の判断**。
- テストを消す T1/T2/T4/T5 は，同じ名前の unit test が残るので参照は解決する。T5 を fake に残す代替案なら，なおさら変更不要。

### B-9 表 B のまとめ（分類ごとの件数）

- (1) Spec へ: 2,5,6,7,8,10,12,14,15,16（10 系列）＋ Direct の 1623（案）＋ 1334 の duplicate 部分（案）。
- (2) tolerant: 4,9，uhd `:229`，protocol `:1089`。
- (3) `start_repeat`: 3,11,17,18。
- (4) unit test: 13（T3），raw 13 本，Direct 1591/1623/1726/1780/4241（要確認）。
- 無害: B-4 の 12 系列，uhd `:71`/`:194`/`:136-145`，protocol の 6 系列＋ `:1104-1116`，mutation 行のない raw 6 本（移設はする）。

## 未確定・orchestrator の判断が要る所

1. **T4/T5 の faults rig**: `manual_rig` は faults を取らない。`rig_with(FakeConfig { clock: ClockMode::Manual, command_queue: 64, faults, ..FakeConfig::default() }, Vec::new())` で作れる見込みだが，faults + `Frozen` + `device_at` の既存例はない（INFERRED）。`manual_rig` を拡張するか，直接 `rig_with` を呼ぶか。
2. **T5 の移設 vs fake に残す**: 推奨は tx.rs へ移す（B29 は `lib`）。代替は fake の Spec 書き換え（payload #15，`StalledSend(15)`，B29 は `fake`）。B29 の `test_bin` がこれで決まる。
3. **T3 の mutation 行**: 今ない。`close_device_burst` の tail/empty を殺す行を足すかは，他のテストへの影響が大きいので今回は足さない案。T3 の unit と fake が同名になる件（spec 18:461）。
4. **`:1334` / `:1414` / `:1902`**: それぞれ分割 (1)，tolerant (2)，Spec (1) の案。`ur_21_txburst_refusals` の duplicate 部分を別テストにするか，同じテストで Spec の 2 本にするか。
5. **`uhd_61_02` (`uhd.rs:240`) と `ea_17_a_capture_ahead` (`protocol.rs:1100`)**: (2) の緩和でよいか（`first >= at` + 長さ）。厳密な開始は `uhd_59`（Spec）が pin する。
6. **Direct 系を掃除の範囲に入れるか**: `Direct::new`/`with_links` は Real clock。`1591`/`1623`/`1726`/`1780`/`4241` は (4)（または (1)）が要る。`3238` は無害。範囲に入れるなら `control.rs` の unit test への移設が前例（`fake.rs:1648-1650` の注記）。
7. **raw 13 本の移設**: `device.rs` の `manual`/`participant` へ。`reports_until` helper も一緒に。行は `fake` → `lib`。テスト名は変えない。
8. **B08 の kill**: handed の合計は B08 でも 31 000 のまま（drop された B の `tx_send` も n を記録する）。kill は `transmitted` の 30 000 番目と TimeError。B20 は B を A と違う波形にしないと生き残る（`held()` の一様な波形は不可）。
9. **Authority の実時間テスト**（`ur_07_schedule_wakes…`，`ur_07_now_is_monotonic…` など）: メモ §4 のとおり実時間の測定そのもの。掃除の対象外としてよいか。
