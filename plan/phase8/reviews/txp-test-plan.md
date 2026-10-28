# (decision record: plan v3 and Sol's third review's two changes, applied in the implementation; the section at the end lists what the implementation added)

# 段階 TXP 試験計画 v2：uhd-tx の実時間優先度と窓の端の最小の send

対象 HEAD `d0a0e64`。code は書かない。読んだもの：`provider/tx.rs`（`run` 117-，`pass`，`step` 270-，`send` 396-，`shutdown`），`device.rs`（trait 156-，FakeDevice `tx_send`），`uhd.rs`（`check_with` 260-，`tx_send` 1100-，C shim 1190-），`host.rs`（manual clock，`blocked()` 475，`hooks::passes()` 617，`pause_next_wake`），`provider/mod.rs`（`spawn` 389-，tx 起動 695-），`test_support.rs`，`tests/common/measurement/multichannel.rs`（`GuardedDevice` 100-162），`design/18` UR-2/UR-3/UR-16/UR-22/UR-23/UR-24，`tx-throughput-report.md`，UHD 4.10 source（image `ezsdr-v4-dev:uhd4.10`，`/root/tmp/uhd-4.10.0.0/host`），Sol の計画レビュー（FAIL，major 5 / minor 4：全部採用）。v1 からの変更は末尾の表。

記法：manual clock の FakeDevice，MCR 200 MHz，rate 1 MS/s（`clock.n = 200` root tick/sample），W = 10 ms = 10 000 sample，`block_len` 2 000，`chunk = min(W/n/4 = 2 500, 2 000) = 2 000`，device lead 2 ms = 2 000 sample。「advance a」は device 時刻を a sample（200 a tick）進める（`device_at`，戻せない）。`sends` は tx.rs の既存 helper（`(n, sob, eob)` の列）。観測は `step()` の返り値，`tx.open`／`tail`／`continues_at`，`device.transmitted`，`rec.rejected`／`stats`。

## 1. 推奨（決め）

### 1.1 優先度

1. **trait に `fn raise_thread_priority(&self) -> Result<(), DeviceError>`**（呼んだ thread を実時間にする）。default 実装なし（必須）。必須にすると転送を**書き忘れる**のは compile error になるが，`Ok(())` を返して転送しない実装は防げない：包み役ごとに「内側に届く」試験を置く（G2）。
2. **呼ぶ場所は `Tx::run` の最初の文**（loop の前，`Tx::new` は Provider の thread）。owner（stream）ごとに 1 回。uhd-control・uhd-rx には付けない。
3. **UhdDevice**：`uhd_set_thread_priority(0.5, true)` を 1 回。Linux は SCHED_RR 50（`thread.cpp:47-66`）。`check("uhd_set_thread_priority", code, false)` で `DeviceError`。**UHD の数値 code を正，text は参考**（text は process 全体の `uhd_get_last_error` で他 thread に上書きされうる）。
4. **拒否は致命にしない**：Provider は `lost` を見ず，どんな `Err` も記録して run を続ける（`device_failed` を使わない）。理由（VERIFIED）：EPERM は `os_error` → `UHD_ERROR_OS` (31)，`check_with` は 31 を `lost = true` にする（`uhd.rs:264`）。**この罠は Provider だけでなく測定の包み役にもある**（1.3）。
5. **記録**：拒否 → `rejected` に `{ "call": "set_thread_priority", "stream": k, "error": <text> }` を owner ごとに 1 行。成功 → `stats.tx_realtime += 1`。**注意**：UHD は `HAVE_THREAD_PRIO_DUMMY` の build で `set_thread_priority` が何もせず成功する（`thread.cpp:162-168`，INFERRED：Linux の bench image は pthread 版）。よって `tx_realtime` は「呼びが成功した」であって「実時間になった」ではない。実際の policy は bench で `/proc/<pid>/task/<tid>/stat` を読んで確かめる（§3 B1）。event／`timing` 行にしない。

### 1.2 最小の send

```
wanted = min(chunk, len − offset, cut − next)      // cut は held_cut と switch_cut の小さい方；窓以外の上限だけで決まる大きさ
avail  = window_cut − next
if device_sob（burst の最初の timed send）:  count = min(wanted, avail)           // 除外：待たない。avail < wanted でも min を送る
else if avail < wanted:                       待つ（何も送らず，tail も取らず，false を返す）
else:                                         count = wanted
```

- 新しい定数なし。除外は **`device_sob` だけ**（`open.first` ではない）：continuation の最初の buffer は Kernel の最初だが device の SOB ではなく，待つ。
- **判断は 1 つの関数にし，2 つの入口の両方で，tail や payload の device 呼び出しの前に使う**：(a) `step` の open の分岐（`tx.rs:312-333`），(b) held から新しく burst を開いて直ちに `send` する入口（`tx.rs:364-396`）。(b) で `device_sob` でない（continuation）なら，待つ場合は：held から外し，`open = Some(Open { next = k, first: true })` にして，`continues_at`・`tail` を触らず，`false` を返す（何も渡していない）。次の pass は (a) で同じ判断を受ける。除外の `device_sob` は `first && continues_at != Some(start)` で，`send` の中の定義と同じ式を 1 箇所で使う。
- 窓が送れる量を入れられない時だけ待つ。小さい最後の部分（`len − offset`），wrap の残り，held／cold の cut の手前は `wanted` 自体が小さいので待たず送る。
- **限定**：待ちは `IDLE`（200 µs）の polling で，起床の遅れ（scheduling），窓の丸め（`at_or_after`），`chunk = 1`（`block_len = 1` など）を含むので「必ず `wanted` に届く」「mid-burst の先読みは常に 0.75 W 以上」は主張しない。`wanted ≤ chunk ≤ ceil(W/n)` なので `next < window_cut` の間に `avail` はいずれ `wanted` に届く（待つだけで止まらない）ことだけが規則の前提で，M11 で `chunk = 1` の場合を縛る。W ≤ lead の既存の dispatch の問題は TXP の対象外で，直すとは書かない。
- 却下：W/4 や固定 sample 数での待ち，眠りを `(wanted − avail)/rate` に伸ばす（Stop／preempt が 1 IDLE で見えなくなる），最初の send も待つ（SOB の lead），chunk の変更。

### 1.3 測定の包み役

`GuardedDevice`（`multichannel.rs:140-159`）の `refuse_or!` は `note` を通し，`error.lost` で latch して以後の呼びを拒み，drop で内側の handle を leak する。**優先度の拒否で latch を作らない**：`GuardedDevice::raise_thread_priority` は既に latch していれば従来の拒否（`refusal()`）を返し（保つ），そうでなければ `self.inner().raise_thread_priority()` の結果を **`note` を通さず**返す。`CountingDevice`／`ProbeDevice` は転送し，呼びを数える。

## 2. UHD／OS の振る舞い

| 事柄 | 状態 | 根拠 |
|---|---|---|
| C API `uhd_error uhd_set_thread_priority(float, bool)`，`UHD_SAFE_C` | VERIFIED | `include/uhd/utils/thread_priority.h:33`，`thread_priority_c.cpp` |
| `(0.5, true)` は `pthread_self()` を SCHED_RR，priority `int(0.5×98)+1 = 50` にする。負は 0，範囲外は `value_error` | VERIFIED | `thread.cpp:35-66` |
| 失敗（`pthread_setschedparam` ≠ 0）は `os_error` → code 31，log なし | VERIFIED | `thread.cpp:67-69` |
| dummy build は何もせず成功，debug log のみ | VERIFIED（source），Linux bench image での選択は INFERRED | `thread.cpp:162-168` |
| 未対応の platform の code は全部 ENOSYS とは限らない（dummy は成功）。code 20（`UHD_ERROR_NOT_IMPLEMENTED`）などの扱いは試験で数値のまま通す | VERIFIED（header），挙動は INFERRED | `error.h:33` |
| `check_with` は 31 を `lost = true` | VERIFIED | `uhd.rs:264` |
| SCHED_RR 50 は CAP_SYS_NICE か `RLIMIT_RTPRIO ≥ 50` で成功。bench container の `--cap-add=SYS_NICE --ulimit rtprio=99` で足りる | INFERRED（`chrt -f -p 50` は同 container で成功：調査 §1.3） | man 7 sched |
| container の外の通常 user は `ulimit -r` が 0 なら EPERM | INFERRED | bench B3 |
| 実時間 thread の 100 % 回転は同じ core の通常 thread を飢えさせ，kernel の RT throttling（`sched_rt_runtime_us`）は帯域の上限であって遅延の保証ではない | INFERRED | man 7 sched；bench B1〜B4 で測る |
| 窓の端の spin は rate が約 6.5 MS/s 未満（W = 10 ms が X300 の 65 536 sample より長い）で起きる | VERIFIED（FakeDevice，hardware 1 MS/s で 2.9–3.1 sample/send） | 調査 §1.2 |
| host clock：manual clock は busy な participant の間止まる。つまり spin は失敗でなく hang になる | VERIFIED | `host.rs` M2–M5 |

## 3. 試験

### 3.1 優先度（P，G，F）

道具：`TxSpy`（`test_support.rs`，v2 の `PrioritySpy` を広げたもの）：`Arc<FakeDevice>` を包み全 method を転送し，次を **共通の呼び出し番号と，その呼びの時点の `host.now()` と `thread::current().name()` つきで記録**する：`raise_thread_priority`（設定した結果 `Ok`／`Err { lost, message }` を返す），`tx_open`，`tx_send`（`n`，`sob`，`eob`），`tx_async`。FakeDevice 自身は何もしない（`Ok(())`）。**P の試験は全部 manual clock で決定的**：`Tx::run` を `core.device.host().spawn(tx_name(k), …)` で動かし，Shutdown を送って join する。優先度の呼びは run の先頭なので，Shutdown が処理される時点で必ず済んでいる。**host clock と device 時刻**：FakeDevice の時刻は `HostClock` とともに進む（`device.rs:845-856`）。`Frozen` で止めると owner の時刻つきの眠りも終わらないので，thread の試験（P，M6，M8b）では使わず，device の時刻を固定する主張はしない。manual clock は owner が busy の間は止まり，owner が眠っている間だけ real speed で進む（`host.rs` M2–M5）：owner が payload を送っている pass は host 時刻 T0 に凍結されたまま起き，その後の idle の眠りの間だけ時刻が進む。

| ID | 系列 | assert | 落とす誤り |
|---|---|---|---|
| P1 | tx stream 2 本（`uhd-tx-s0`，`uhd-tx-s1`）の `Tx::run` を spawn，idle の待ちを待ち，Shutdown | thread 名の集合 = その 2 つが 1 回ずつ，他 thread（test，uhd-rx，uhd-control）から 0 回 | 呼ばない；`Tx::new` で呼ぶ；毎 pass 呼ぶ（回数）；uhd-rx／control にも付ける |
| P1b | Provider を manual clock の FakeDevice で `start`→`stop`（`mod.rs:695` の経路。`stop` が thread を join するので呼びは済んでいる） | P1 と同じ集合。名前は `tx_name(k)` と一致 | spawn が `run` を通らない／別名 |
| P2 | P1 の `TxSpy` の記録の番号を **owner（thread 名）ごとに**見る（burst を 1 本送る版：各 owner に payload を booking する） | 各 owner について，その owner が最初の `tx_send` を **持つ**（無ければ失敗）ことと，`優先度の呼びの番号 < その owner の最初の `tx_send` の番号` | send の後に呼ぶ；呼びが無い／send が無いのに通る |
| P3 | `start`→`stop`→`start`→`stop` | 各 owner から 2 回（毎回 新しい thread） | process で 1 回（static） |
| P4 | spy が `Err { lost: true }`（UhdDevice の EPERM の形）。burst を送る | `core.is_lost()` 偽，DEVICE_LOST／COMMAND_REJECTED の event 0，burst の payload が device に届く（`transmitted`），`rejected` に `set_thread_priority`・`stream`・error の行が owner 数，`tx_realtime` 無し，Shutdown が 1 s の join に収まる | `device_failed`；`return`／`?`；`expect`（panic → UR-16 の `device_lost`） |
| P5 | spy が `Ok` | `tx_realtime` = owner 数，priority の `rejected` 行無し | 成功を数えない／拒否でも数える |
| P6 | stream 0 だけ `Err`，stream 1 は `Ok` | 行は stream 0 だけ，`tx_realtime` = 1 | 結果の使い回し |
| P7 | 既存の manual clock の `ur_26_*`／`ur_34_*` の `Tx::run` の試験 | 緑のまま（優先度の呼びが participant の busy／idle の順序を壊さない） | 呼びが host の外で blocking |
| G1 | **測定の包み役の鎖**：`GuardedDevice`（`recording`）→ spy（`Err { lost: true }`）で Provider を動かす | `guard.latched()` が None のまま，burst の payload が内側の `tx_send` に届く，`rejected` は 1 回（owner 数），後続の device 呼びが拒まれない | `refuse_or!`／`note` を通して latch；`Ok` を返して転送しない |
| G2 | `GuardedDevice`／`CountingDevice`／`ProbeDevice` のそれぞれで，spy が `Ok` | 内側の spy が ちょうど 1 回の呼びを見る（thread 名 = uhd-tx） | 転送を忘れ `Ok(())` を返す |
| G3 | 既に latch した `GuardedDevice`（`latch.set(..)`）に優先度の呼び | 従来の拒否（`latched (…)`）を返し，spy は呼ばれない。Provider は記録して続ける（latch は device-failure の追加無し） | latch 後も内側へ通す |
| F1 | `uhd.rs` の C shim に `#[unsafe(no_mangle)] extern "C" fn uhd_set_thread_priority(priority: f32, realtime: bool) -> uhd_error`（引数と回数を static に記録，返りは static の code）。`raise_thread_priority()` を 1 回 | 引数が `(0.5f32, true)`，呼び 1 回，`Ok` | 値を変える；`realtime` を false（SCHED_OTHER で黙って no-op）；2 回呼ぶ |
| F2 | shim が 31（`UHD_ERROR_OS`），20（`UHD_ERROR_NOT_IMPLEMENTED`），44（`UHD_ERROR_RUNTIME`）を返す | いずれも `Err`，message に `uhd_set_thread_priority` と **数値の code** を含む（text は assert しない）。`lost` は assert しない（Provider 側が無視する：P4） | `Err` を `Ok` に潰す（`let _ =`） |
| F3 | 既存の shim の試験 | symbol の追加で link でき緑 | symbol 未定義 |

### 3.2 最小の send（M）

step 直接の unit（`tx.rs` の `tests`，既存の組み立てを使う）は manual clock で時間は advance のときだけ動く。thread を使う M6／M8b だけ `host.blocked()`／`hooks::passes()` の handshake を使う（§5）。

| ID | 系列（now = 0） | 期待 | 観測 | 落とす誤り |
|---|---|---|---|---|
| M1 | repeat（len 30 000）を start 3 000。`while step()` | sends = [2000, 2000, 2000]，next 9 000（avail 1 000 < 2 000）。次の `step()` は false で `tx_send` が増えない（現行は 1 000 を足す） | `sends`，`open.next`，`step()`，`calls().len()` | 規則が無い；待つのに true を返す |
| M1b | 続けて advance 1 000（avail ちょうど 2 000） | 1 回 2 000，next 11 000（`<` の境）。advance 500 なら送らない | 同上 | `<=` で待つ |
| M1c | 各 step の後 | `transmitted(window_cut 以後)` が None，next ≤ window_cut | `transmitted` | 窓を超える |
| M2 | 非 repeat len 6 800，start 3 500。`while step()`，次に advance 300 | [2000, 2000, 2000]，next 9 500（avail 500 < 残り 800 < chunk：割らない）。advance 300 → n = 799，sob 偽，eob 偽，`tail` = Some，`continues_at` = Some(10 300) | `sends`，`tail` | 最後の部分を窓で割る；`wanted` が `len − offset` を使わない；`wanted` を `sending`（count − 1）で測る |
| M3 | repeat の waveform 1 500，start 9 000（最初の send は SOB で除外，avail 1 000 の 1 000 を送り offset 1 000）。advance 500 | 1 回 500（`len − offset`，avail ちょうど），next 10 500。advance 400 では wanted = 1 500 > 400 で待つ | `sends`，loopback の連続（wrap の隙間なし，restart 0） | wrap の残りを chunk に切り上げる；wrap の手前を待つ |
| M4 | M1 の終わり（A = repeat，next 9 000）で B2（非 repeat len 4 000）を start 9 600 に book。step，step | step 1：A の 600（< chunk だが cut までなので送る），device eob にならない。step 2：B2 を continuation として開き，avail 400 < 2 000 なので **待つ**：`false`，`open.next` = 9 600，`open.first` 真，`continues_at` = Some(9 600) のまま，`tail` 不変，send 無し。advance 1 600 → B2 の 2 000（sob 偽，`at` 無し）。`unended_bursts` 0，`restarts` 0 | `sends`，`continues_at`，`step()` の返り | cut を無視する；**入口 (b) を迂回して continuation の小さい payload を送る** |
| M4b | 同じ状態で B2 を start 10 500（cut − next 1 500 > avail 1 000） | 待つ。advance 500 → 1 回 1 500（eob = ends_at_held） | 同上 | cut の手前を割る |
| M5 | 候補の窓 W = 2.2 ms（`description.timing.tx_in_flight_ns = 2_200_000`，chunk 550；unit は description を直接設定する既存の candidate 試験の型で，production の admission の可否は主張しない）で repeat を start = now + lead = 2 000（avail 200 < 550） | 最初の send が**即座に**出る：n = 200（`min(wanted, avail)`），sob 真，`at` = `clock.instant(2000)` | `sends`，`calls` の `at=` | 除外を外す／除外の send に `wanted` を送る（窓を越える） |
| M6 | **M8b の行と同じ設定**（100 kS/s，W = 50 ms，avail = 0，burst を preload，`Frozen` なし）の `Tx::run` の thread を，owner の clock の sleep／wake を制御して使う：owner の thread の先頭で `pause_next_wake(woken, release)` を仕掛け，最初の idle の起床で止める。(1) owner が 3 つの payload を送り idle の待ちに入り，その最初の起床で止まる（`woken` を real 10 s の上限で待つ）。(2) test が `TxSpy` から T_in（idle に入る直前の最後の `tx_async` の host 時刻 = T0，owner は busy で時刻は凍結）を読み，`woken` を受けた直後に `host.now()` = T_wake を読む。(3) **release の前に Stop を積む**（owner の channel の sender は test が持つ：`ur_34_a_transmit_stop_during_release_wait_cancels_later_bursts` が自前の channel を作る型）。(4) `release` を放す | **T_wake − T_in ≥ IDLE かつ < 3·IDLE**（IDLE = 200 µs。眠りが `(wanted − avail)/rate` = 12.5 ms の実装 X19 は落ちる；上限は起床の scheduling の遅れの余裕）。放した後：次の `tx_send` が **EOB（n = 1，eob 真，zero sample）で，その前に payload の send が 1 つも増えない**（Stop が次の payload の send より前に EOB を出す）。**cancel の `rejected` 行は期待しない**（`stop_now` は held の burst だけ）。実時間の timeout は有限時間の失敗のためだけで，応答性の oracle ではない | `TxSpy` の記録（時刻と順），`woken` | 眠りを伸ばす（X19）；待ちが無く spin（`woken` が来ない：10 s で有限時間の失敗）；Stop を payload の後に処理する |
| M7 | M1 の待ち中に `h ≤ next` の held burst（start 8 800，`SendAsapAndFlag`）を book | 次の step で preempt（`time_error` が 1 つ，`held.k` = bound） | events | 待ちの判断を preempt の前に置く |
| M8 | step 直接：repeat start 3 000，advance を 1 000 ずつ 10 回，各回 `while step()` | send は最初の 3 つ + 2 ms ごとに 1 つ（t = 1, 3, 5, 7, 9 ms）= **合計 8**，どれも 2 000，期限切れ無し（payload の大きさ・数の oracle：待ちを示さない） | `sends` | 規則が無い（約 50 回）；`wanted` が chunk 未満 |
| M8b | **thread の設定**：rate 100 kS/s（`clock.n` = 2 000 root tick/sample），候補の窓 `tx_in_flight_ns` = 50 ms（W = 5 000 sample，chunk = min(1 250, 2 000) = 1 250；unit が description を直接設定する既存の candidate 試験の型），repeat を start 1 250（SOB は除外で 1 250 を送る，次に 1 250 が 2 回）で **avail = 0**（next = 5 000）。burst を channel に preload してから owner を spawn（`Frozen` は使わない）。payload が準備できる境目は avail が 1 250 に届く時で，host 時刻で 12.5 ms（= IDLE 62 周期）後。test は real 時間で polling（1 ms 以内の間隔）し，**`TxSpy` の idle の `tx_async` の記録（payload の send の後の記録）が 5 件になった時点で snapshot を取る（Shutdown の前）**。その後に Shutdown，join | snapshot で：(a) idle の記録が 5 件以上，互いに **別の周期**（host 時刻が真に増える）で，隣り合う間隔が **IDLE 以上，5·IDLE 以下**（sleep している：spin なら時刻が凍結して間隔 0 で落ちる）；(b) その間の `tx_send` の記録が 3 つのまま（payload の呼びが区間で変わらない）；(c) snapshot の時刻は境目（12.5 ms）より手前（5·IDLE = 1 ms 付近）。Shutdown の report の待ち（`tx.rs:939-955`）は snapshot の後なので数えない。`hooks::passes()` は待ちに入る・condvar の起床も数え Shutdown の待ちも入るので **使わない** | `TxSpy` の host 時刻つきの記録 | 送らずに回り続ける spin（時刻が凍結して間隔 0，有限時間の失敗）；待ちの判断のかわりに payload を送る；idle の周期が無い |
| M9 | A（非 repeat len 6 000，start 3 000）：[2000, 2000, 1999]，`tail` = Some（1 sample），`continues_at` = Some(9 000)。B を start 9 000 に book（avail 1 000 < 2 000） | 待つ：`tail` が Some のまま，`calls` が増えない。advance 1 000 → tail の 1 sample（n=1，sob/eob 偽，`at` 無し）の後に B の 2 000。A と B が device で連続 | `tail`，`calls`，`transmitted` | 判断が tail の送信の後；tail が 2 重／失われる；入口 (b) の迂回 |
| M10 | M1 の待ちから device 時刻が `next` を超える（advance 9 001） | 次の step が「UR-22: the transmit timeline expired」を `rejected` に記録して abandon（true），EOB の zero sample 1 つ | `rejected`，`sends` | 待ちの判断が期限切れの検査の前 |
| M11 | `block_len = 1`（selector が認める）：`chunk = 1` | `wanted` は 1 以下で，`avail ≥ 1` なら送る（現行と同じ，待ち続けない） | `sends` | `wanted` を chunk 固定にして届かない |
| M12 | 2 channel の stream で M1 | 同じ sends（count は sample 数） | `sends` | channel を掛けた count |
| M13 | 窓の外の burst（start ≥ window_cut）は開かれない | `ur_21_a_burst_inside_the_in_flight_window_is_late` 等の既存が緑 | 既存 | `step` の最後の分岐を壊す |
| **M14** | **cold の cut**：repeat A（start 3 000）が M1 の待ち（next 9 000，avail 1 000），`tx.switch = switch(&core, cut, None, 0, …)` で cut = sample 10 500（root 2 100 000）。(a) 次の step は待つ（cut − next 1 500 > avail 1 000；現行は 1 000 と 500）。(b) advance 500 → 1 回 1 500 で next = 10 500（排他の cut ちょうど）。(c) 次の step：`next >= cut_k` で device burst を閉じる（空の EOB の zero sample のみ，cut 以後の payload は無い：`transmitted(clock.instant(10_500), 1)` は EOB の zero sample 以外 None）。(d) advance して `now ≥ cut` → step で `do_switch`：`tx.clock` = None，`tx.at` = 1。(e) 同じ列を `switch(&core, cut, Some(new), 2, Settings { rate: Some(2e6), .. })`：`tx.clock == Some(new)`，`tx.channels` = 2，新しい clock の最初の send が出る | `sends`，`tx.clock`／`at`／`channels`，`transmitted` | `wanted` から `switch_cut` を外す（(a) で 1 000 を送る／cut を越える） |

**時刻つきの命令との順**：待ちは device の呼びを作らず，uhd-control の thread と状態を共有しない。timed 命令の queue 順と DUC の対は次の既存の試験を回帰として緑のまま保つ（新規の試験は足さない）：UR-24 の queue 順 `ur_24_the_device_queue_order_is_the_effective_order` と `ur_24_a_release_behind_a_released_command_keeps_the_device_queue_order`（`provider/control.rs`），queue の数 `ur_24_the_command_queue_is_counted_per_instance_across_streams`，DUC の対 `ur_33_a_timed_duc_value_is_taken_only_while_has_time_holds`，`ur_33_a_full_duc_is_a_model_that_breaks_the_time_value_pairing`，`ur_33_a_timed_command_follows_its_front_end_across_a_spec`（`device.rs`）。`ur_25_a_transmit_poll_during_a_control_open_starts_no_streamer_call` は所有の確認で，order の根拠にはしない。D9 F3′（KL-10）の「最初の send に入る instant」は最初の send を除外するので幅が変わらない。

### 3.3 実機の受け入れ（owner が流す。**送信の前に減衰器 ≥ 30 dB の確認**）

結論は測るまで **INFERRED**。各回に記録：Manifest（`stats`：`tx_realtime`，`tx_samples`，`tx_errors`；`rejected`；`events` の `TX_UNDERFLOW`／`TIME_ERROR`；`tx` の bursts の `blocks`），uhd-tx／uhd-rx／uhd-control の各 thread の `/proc/<pid>/task/<tid>/stat` の policy（第 41 欄，RR = 2）と priority（第 18 欄），`/proc/sys/kernel/sched_rt_runtime_us`・`sched_rt_period_us`（container の中と host），uhd-rx の進み（`rx_blocks`，overflow，`rx_samples`），uhd-control の timed 命令の issue 時刻（`timing`／`applied`）と release 窓の遅れ，Stop の完了までの時間。

- **B0 低い rate（窓が律速）**：1 MS/s と 5 MS/s，repeat の waveform **31 000 sample**（chunk 2 000 で割り切れない：wrap ごとに 1 000 の境目の send が出る），60 s。Manifest の `bursts[].blocks` を累積 offset で分類する：(i) 最初の block（SOB）は ≤ chunk，(ii) wrap（累積が 31 000 の倍数）で終わる block は許された小さい大きさ（≤ 1 000），(iii) **それ以外の普通の continuation の block は chunk（2 000）に等しい**。(iii) に違反があれば失敗。基準の run（`d0a0e64` の build，同じ設定）と比べて，uhd-tx thread の `blocks`／s が `rate / chunk × 1.1` 以下（基準は約 330 k/s の send の規模）に減り，uhd-tx の CPU 時間（`/proc/<tid>/stat` の utime + stime を 60 s で割る）と自発的でない context switch の数（`/proc/<tid>/status`）が基準より減ることを**記録**する（減る割合の閾値は INFERRED で，gate にしない）。underrun 0。
- **B1 100 MS/s**・**B2 200 MS/s**：60 s，cap あり（`--cap-add=SYS_NICE --ulimit rtprio=99`）。`tx_realtime` = owner 数，policy = 2。underrun を `benchmark_rate --priority high`（0 件，記録済）と比べる。
- **B3 同時運転**：送信（100 MS/s）+ 受信 + timed の control 更新（frequency／gain）+ burst 途中の Stop。uhd-rx の overflow 0，control の issue の遅れが release 窓内，Stop が 1 s 以内に完了。starvation の兆候（uhd-rx／uhd-control の runnable 待ち）を `schedstat` で記録。
- **B4 cap 無し**（`--ulimit rtprio=0`，cap なし）と host の通常 user：`rejected` に priority の行が出て run が完走，DEVICE_LOST にならない。

## 4. 壊れる既存の試験

- 予測：`ur_21_send_does_not_cross_the_in_flight_window` の case 2（rate 390 625：chunk 976，start 782，window_cut 3 907）。新規則では next が 782 + 976 × 3 = 3 710 で止まり，`expected_cut` = 3 907 に届かない。期待値と後続の `device_at(…, 385)`・booking の位置を書き直す。case 1（start 9 000）は最初の send の除外（`min(wanted, avail)` = 1 000）で現行の期待のまま通る。
- 要確認：`ur_25_cold_change_does_not_inherit_old_tx_front`（`while step()`），`ur_21_a_burst_inside_the_in_flight_window_is_late`，`ur_23_stop_*`（window_cut までの handed を assert するもの），`ur_21_a_finite_burst_with_a_held_tail_keeps_its_front`，`ur_21_a_post_eob_restart_uses_the_safe_sample`，`ur_23_a_continuation_tail_is_counted_at_recovery`。`tests/fake.rs` の `burst_blocks` の `blocks.len() == 1` は影響しないはず。
- 実装者は lib と fake を通し，落ちたものを「規則の更新」か「回帰」に分けて報告する。

## 5. 時刻に依る試験（20 回繰り返し）

manual clock でも thread と real 上限つきの handshake を使うので，次を 20 回繰り返す：**P1，P1b，P2，P3，P4，P5，P6，P7，G1，G2，G3，M6，M8b**。step 直接の M1〜M5，M7，M8〜M14 は純粋（繰り返し不要）。F1〜F3 は C shim の static を使うので `TEST_LOCK` 内（既存の型）。

## 6. spec 18 の変える規則（DA-1：規則だけ）

- **UR-2**：trait の一覧に `fn raise_thread_priority(&self) -> Result<(), DeviceError>; // the calling thread to real-time priority (UR-23)`。*Test:* P1，G2，F1。
- **UR-3**：関数の一覧に `uhd_set_thread_priority`。*Test:* F1，F3。
- **UR-23**：(1)「uhd-tx は thread の開始時に 1 回，`Device::raise_thread_priority` を呼ぶ（`UhdDevice` は `uhd_set_thread_priority(0.5, true)`）。どんな `Err` も `rejected` の `call = set_thread_priority` の行として記録され，stream は動き続け，device の loss とはされない。成功は `stats.tx_realtime` に数えられる。」*Test:* P1，P1b，P2，P3，P4，P5，P6，G1〜G3，F1，F2。(2)「uhd-tx は，buffer の大きさを定める他の上限（chunk，waveform の終わりか wrap までの残り，次の held burst の start か cold の cut までの残り）が許すすべての sample が in-flight window に収まるまで，burst の最初の timed buffer を除いて，その buffer を送らず（tail も送らず）待つ。最初の timed buffer は window に収まる分を送る。」*Test:* M1，M1b，M2，M3，M4，M4b，M5，M6，M7，M8b，M9，M10，M11，M14。
- Changes 表に 1 行（日付，規則，決定 record `plan/phase8/` の本段階の file）。偽になる `design/` の文：無し（UR-23 の「hands only payload samples before …」は真のまま）。KL-10 は最初の send を除外するので変わらない。**KL-5**（50 MS/s を超える送信の失敗）は bench で効きが確かめられるまで書き換えない（コード段階の commit では更新せず，bench 後に status を更新）。`rules.py --base d0a0e64` を通す。

## 7. mutation 行の候補（`old` は実装後の正確な文字列。P714 は chunk の行を触らないなら不変）

| ID（仮） | 変異 | 殺す試験 | 実行の場 |
|---|---|---|---|
| X1 | `run` の先頭の優先度の呼びを消す | P1，P5 | lib |
| X2 | 呼びを `Tx::new` へ | P1 | lib |
| X3 | 呼びを loop 内へ | P1 | lib |
| X4 | 失敗を `core.device_failed(…)` に | P4 | lib |
| X5 | 失敗で `return`／`?` | P4 | lib |
| X6 | 失敗の `rejected` 行を落とす／成功でも書く | P4，P5 | lib |
| X7 | `stats.tx_realtime` を落とす | P5，P6 | lib |
| X8 | `0.5` → `1.0`／`0.0` | F1 | **`--features uhd`** |
| X9 | `true` → `false` | F1 | **`--features uhd`** |
| X10 | `UhdDevice` が `Err` を `Ok` に | F2 | **`--features uhd`** |
| X11 | 規則を外す（待ちを消す） | M1，M8，M8b | lib |
| X12 | `<` → `<=` | M1b | lib |
| X13 | `wanted` から `len − offset` を外す | M2，M3 | lib |
| X14 | `wanted` から `held_cut` を外す | M4，M4b | lib |
| **X14b** | `wanted` から `switch_cut` だけを外す | M14 | lib |
| X15 | `device_sob` の除外を外す | M5，`ur_21_send_does_not_cross_the_in_flight_window` case 1 | lib |
| X15b | 除外の send が `min(wanted, avail)` でなく `wanted` | M5 | lib |
| X15c | 除外を `open.first`（continuation も除外）に | M4，M9 | lib |
| X16 | 待ちの判断を期限切れの検査の前へ | M10 | lib |
| X17 | 待ちの判断を `send` の中の tail の送信の後へ | M9 | lib |
| **X17b** | **入口 (b)（held から開いて直ちに send）の待ちを迂回** | M4，M9 | lib |
| X18 | `wanted` を `sending` で測る | M2 | lib |
| X19 | 待ちの眠りを IDLE より長くする（`(wanted − avail)/rate`） | M6（T_wake − T_in < 3·IDLE） | lib |
| **X21** | `GuardedDevice` が優先度を `refuse_or!`（note）に通す | G1 | tests（fake bin） |
| **X22** | `GuardedDevice`／`CountingDevice`／`ProbeDevice` が転送せず `Ok(())` | G2 | tests（fake bin） |
| **X23** | latch 後の `GuardedDevice` が内側へ通す | G3 | tests（fake bin） |

**X8〜X10 は `--features uhd` でしか build されない**（`lib.rs:13`），遠隔の mutation image に libuhd が無いので sim02／sim03 の `remote_mutate.py all` では走らない。この段階では **延期と明記**：F1〜F3 は `cargo test -p ezsdr-radio-uhd --features uhd --lib uhd::`（libuhd のある image／host）で緑を確認し，X8〜X10 は feature つきで流せる gate（`ezsdr-v4-dev:uhd4.10` の image を遠隔に載せた run）をオーナーが用意するまで保留する，と段階の記録に書く。

## 8. 疑わしい所

1. **小さい窓**：待ちは IDLE 200 µs の polling なので，起床の遅れ・丸め・`chunk = 1` で `wanted` に届くまでの遅れがあり，mid-burst の先読みの下限は保証しない（1.2）。W ≤ lead の既存の dispatch の問題は直さない。
2. **低い rate の起床**：100 kS/s では `wanted` = W/4 = 250 sample = 2.5 ms で眠りは 200 µs：起床は 12 回に 1 回送る程度。spin より軽い（INFERRED）。
3. **error text** は他 thread に上書きされうる：numeric code が正（F2）。
4. **実時間の飢え**：SCHED_RR 50 の uhd-tx と通常の uhd-rx／uhd-control が同じ core に載る場合，flow-control 下（高い rate）の UHD 内部の挙動は bench（B1〜B3）でしか分からない。`sched_rt_runtime_us` は帯域の上限で遅延の保証ではない。
5. **dummy build**：成功が実時間を意味しない（1.1 の 5）。bench で policy を読む（B1）。
6. **#2 の 10 MS/s の停止**（範囲の外）：候補は (a) 1 s の send timeout（`rejected`），(b) UR-22 の期限切れ，(c) `TX_UNDERFLOW` 後の abandon，(d) 別の host／NIC。本段階の `tx_realtime` と優先度の拒否の行が #2 の host で効いたかの手掛かりになる。
7. **`raise_thread_priority` が `lost` を返す実装**：Provider は無視する。別の Device 実装が本物の loss を返しても見逃すが，loss は他の呼びで見つかる（UR-29）。
8. **panic**：呼びは `Result` で panic しない。`catch_unwind` の中（UR-16）。
9. **MockRadio**：優先度は device の振る舞いで，窓は Module の内部。Mock に対応する規則は無いと判断（INFERRED：Mock の `PerformanceEnvelope` は send の大きさを検査しない）。
10. **continuation が待つ間の状態**：held から外れ `open` が Some，`first` 真，`continues_at` と `tail` が残る。その間に Stop／cold の cut／preempt が来たときの `end_open` は既存の経路（`device_open` が `continuing == Some(open.held.k)` を見る）で，M4 の続きで確認する（Stop を積んで空の EOB が 1 回，tail が送られるか破棄されるかは `end_open` の既存の規則どおり）。

## 9. 実装者への制約（brief に入れる）

- `cargo fmt` 禁止。mutation の行は正確な文字列。**1 つの関数が count と待ちを決め，2 つの入口の両方で使う**。
- `Device` の全実装を直す：`device.rs`（FakeDevice），`uhd.rs`，`mod.rs:1014`，`rx/differential.rs:545`，`test_support.rs`（LockSpy，ApplyFails，HoldApply，ReportFeed），`tests/common/measurement/multichannel.rs`（GuardedDevice は 1.3 の規則，CountingDevice），`tests/common/probe_device.rs`，`tests/fake.rs` の各 wrapper（Recording，Watched，Descheduled，LostAtShutdown，Gated ほか）。`git grep -n "impl Device for"` で全数を確認。
- 既存の試験の期待値を変える場合は §4 の分類を報告に書く。

## v1 からの変更

| 項目 | v1 | v2 |
|---|---|---|
| Sol major 1：GuardedDevice | 必須 method の転送で足りるとした | §1.3：優先度の拒否で latch を作らない（`note` を通さない），既 latch の拒否は保つ。G1（鎖の試験：inner が `lost: true`，latch 空・payload 到達・拒否 1 回），G2（成功の転送が内側に届く，3 つの包み役），G3，X21〜X23。「`Ok` を返して転送しない」実装は必須 method では防げないと明記 |
| major 2：continuation の入口 | 判断は `step` の open の分岐だけ | §1.2：2 つの入口の両方に 1 つの判断，tail／payload の device 呼びの前。除外は `device_sob` だけ（`open.first` でない）。待つ continuation の状態（held から外れ open，first 真，`continues_at`／`tail` 不変）と false を明記。擬似 code は除外の send が `min(wanted, avail)`。M4／M9 を残し X15b／X15c／X17b を追加 |
| major 3：oracle | M8 は「高々 6」，Stop の試験が cancel の拒否を期待，P1 は実時計の待ち | M8 の数は 8（3 + 5）で payload の数の oracle と明記。M8b／M6：`host.blocked()` で実際の idle の待ちに入ったことを観測，`hooks::passes()` で起床の繰り返し，Stop を積んで次の起床で EOB を観測，失敗は real 10 s の有限時間（spin は clock が止まり hang になる点を明記）。M6 から cancel の拒否の期待を削除（`stop_now` は held のみ）。P1 は manual clock・決定的（P1b は Provider 経由）。§5 に P1〜P7，G1〜G3 を追加 |
| major 4：cold の cut | M4 の held だけ | M14：待ち中に cut までの残り < chunk，待ち → 排他の cut ちょうど → payload を越えず閉じる → `do_switch`（to no stream と to new clock：`tx.clock`／`at`／`channels`）。X14b（`switch_cut` だけ） |
| major 5：実機 | B1：100 MS/s の TX のみ | §3.3：B0 低い rate（窓が律速），B1／B2（100／200 MS/s），B3 同時運転（timed の control 更新と Stop），B4 cap 無し。policy／priority の実値，underrun，RX の進み，control の issue 時刻，Stop の完了，`sched_rt_runtime_us` ほかを記録。結論は測るまで INFERRED |
| minor 1：feature | X8〜X10 を通常の mutation に含めた | `--features uhd` でのみ build，遠隔 image に libuhd 無し：延期と明記，F1〜F3 は feature つきの `cargo test` で緑を確認する gate を書いた |
| minor 2：小さい窓 | 「必ず wanted に届く」，0.75 W の先読み，M11 は極小の W | §1.2／§8.1 で限定（polling・scheduling・丸め・chunk = 1）。M11 は `block_len = 1`（selector が認める設定）。M5 は unit の description 直接設定で production の admission は主張しない。W ≤ lead の既存の問題を直すとは書かない |
| minor 3：error | text を参考とした | numeric code を正，text は assert しない。F2 に 31／20／44 の case。未対応は全部 ENOSYS とは書かず，UHD の dummy（成功）を §1.1 の 5／§2 に追記（`tx_realtime` は「呼びが成功」であって RT の証拠でない） |
| minor 4：timed 命令の順 | 所有の試験を挙げた | UR-24 の queue 順と DUC の既存の試験を名指しで回帰（§3.2），所有の試験は order の根拠にしない |

## v2 からの変更

| 項目 | v2 | v3 |
|---|---|---|
| Sol 2 回目 major 1：M6 | `host.blocked()` と「次の起床」で EOB を見るだけ（眠りが長い X19 が生き残る） | owner の起床を `pause_next_wake` で止め，**release の前に Stop を積み**，idle に入る直前の `tx_async` の host 時刻（T_in，owner が busy で凍結）から起床の `host.now()`（T_wake）までが IDLE 以上 3·IDLE 未満であることを assert（X19 は 12.5 ms で落ちる）。放した後，次の `tx_send` が EOB（n = 1，eob 真）で，その前に payload の send が増えないことを assert。実時間の timeout は有限時間の失敗のためだけ。cancel の `rejected` の期待は無し |
| major 2：M8b | device 時刻を固定する主張，`hooks::passes()` の合計（Shutdown の report の待ちを含む）の threshold 3 | device の時刻は `HostClock` とともに進む（`device.rs:845-856`）ので固定の主張を外し，`Frozen` を使わないと明記。`hooks::passes()` は使わない。`TxSpy` が各呼びの host 時刻を記録し，**Shutdown の前の snapshot**で，payload の send の後の idle の `tx_async` を 5 件，互いに別の周期（host 時刻が真に増える）で隣り合う間隔が IDLE 以上 5·IDLE 以下，その間 `tx_send` が 3 つのまま。設定は 100 kS/s・W = 50 ms・chunk 1 250・avail 0（境目は 12.5 ms 後で snapshot は約 1 ms）。report／shutdown の待ちは snapshot の後なので数えない。spin は時刻が凍結して間隔 0 で有限時間に落ちる |
| minor 3：P2 | 優先度の呼び > 最初の send（向きが逆） | owner ごとに `優先度の呼びの番号 < 最初の tx_send の番号`，かつ最初の send が在ること |
| minor 4：B0 | 「各 send ≥ chunk」 | repeat の waveform を 31 000 sample に決め，累積 offset で分類：SOB は ≤ chunk，wrap で終わる境目の block は許された小さい大きさ（≤ 1 000），それ以外の普通の continuation は chunk（2 000）に等しい。CPU は基準 run（`d0a0e64`）との比較（blocks/s ≤ rate/chunk × 1.1，uhd-tx の utime+stime と自発的でない context switch の数）を記録（閾値は INFERRED で gate にしない） |
| minor 5：UR-24 の回帰 | 「`ur_24_*` の queue 順の試験」 | `ur_24_the_device_queue_order_is_the_effective_order`，`ur_24_a_release_behind_a_released_command_keeps_the_device_queue_order`（`provider/control.rs`）を名指し。queue の数と DUC の回帰はそのまま |
| 道具 | `PrioritySpy`（優先度の呼びだけ記録） | `TxSpy`：`raise_thread_priority`／`tx_open`／`tx_send`／`tx_async` を共通の番号・host 時刻・thread 名つきで記録（P2，M6，M8b が共有） |

## Sol 3 回目の 2 つの変更（実装前に計画へ入れた）

1. **M8b**：`TxSpy` が，payload の send が 3 つ済んだ後の 5 つ目の idle の `tx_async` で，owner の thread の上で snapshot を取り，owner を channel の `recv` で止める（manual clock は止まったまま）。test の thread は snapshot を受けて Shutdown を積み，owner を放す。snapshot の受け取りは有限時間（10 s）で失敗させるためだけ。実装：`test_support::TxSpy::trap_idle`，`ur_23_m8b_…`。
2. **B0**：wrap で終わる block は `min(chunk, 31 000 − 直前までの offset)` で確かめる（`plan/phase8/bench.md` (f)）。

## 実装で足したこと

- `send` は bool を返す（待ったら false）。`abandon` も bool（常に true）を返し，`return self.abandon(…)` の行を既存の mutation の行のまま残した。待つ判断は `send` の中の 1 箇所で，`step` の 2 つの入口は `return self.send(…)`。
- `Device::raise_thread_priority` の FakeDevice は `Ok(())` だけ（`calls` に記録しない：既存の `calls` の列を変えないため）。LockSpy は転送だけ。CountingDevice は転送だけで呼びは数えない（G2 は内側の `Recording` が見る）。
- 試験の道具：`test_support::TxSpy`（呼び出し番号の代わりに記録の順，host 時刻，thread 名，`n`，`eob`；`refuse_priority(error, only)`；`trap_idle`），`tests/fake.rs` の `Recording` に `raise_thread_priority` の記録と答えの設定。lib の P/M6/M8b は `Tx::run` の thread，P1b・G1〜G3 は `tests/fake.rs`（Provider の session と包み役の鎖）。P3 は lib で同じ Core に `Tx::run` を 2 回。
- `TxSpy::tx_async` は lock を持ったまま FakeDevice の眠りに入らない（最初の版は trap の lock を持ったまま眠り，2 owner の shutdown で manual clock が止まって hang した。直した）。
- M12 は `Held` を直接作って 2 channel の stream に入れた（`booking_control` は 1 channel）。M14 は (a)〜(d)（to no stream）まで。(e) の新しい clock への switch は書いていない（既存の `ur_25_*switch*` の試験が `do_switch` の側を持つ）。
- 空の end-of-burst は `tx_send` の呼びとしては n = 0（FakeDevice の記録は zero sample 1 つ）：M6 は `(n, eob) = (0, true)`，M10・M14 は FakeDevice の記録の `(1, false, true)`。
- 窓の規則で期待値を更新した既存の試験（§4 の分類：すべて**規則の更新**で，回帰はない）：`ur_21_a_finite_burst_inside_the_window_is_late`，`ur_21_a_preemption_front_boundary_is_on_time`（start を 4 000 に：窓の端 10 000 に 2 000 の倍数で届く；後者は device の時刻 401 tick で B の 2 sample が窓に入る），`ur_21_a_preempted_send_asap_move_keeps_one_error_and_its_requested_target`（start 7 000：13 000 に届く），`ur_21_a_handed_repeat_sets_the_preemption_front`（continuation の B の 2 000 が窓に入る時刻まで進める），`ur_21_a_burst_inside_the_in_flight_window_is_late`（A の cursor は 38 000 で止まり，continuation は 38 000），`ur_21_send_does_not_cross_the_in_flight_window`（case 2 の start 979：976 × 3 で窓の端 3 907 に届く；B の continuation が窓に入るよう device の時刻 1 000 tick），`ur_22_a_send_cut_short_ends_the_device_burst_after_what_it_took`（send が 2 000 sample ごとになるので，6 000 sample の case の最後の buffer は 4 つ目の send：`ShortSend(3)`）。
- 既存の mutation の行：P637，U17，R06 の `old`／`new` を新しい本文に合わせた（P637・U17 は行の形，R06 は `device_sob` の定義の位置）。他の既存の行は本文が変わらず一致する。
- 新しい行は P1100〜P1123（maintenance list）。計画の対応：X1=P1100，X3=P1101，X2=P1102，X4=P1103，X5=P1104，X6=P1105（行を落とす。「成功でも書く」の行は作っていない：成功の側は P5・P6 が `rejected` の行が無いことを見る），X7=P1106・P1107（拒否も数える／成功を数えない），X11=P1108，X12=P1109，X13=P1110，X14=P1111，X14b=P1112，X15=P1113，X15b=P1114，X15c=P1115，X17=P1116，X17b(b)=P1117，X19=P1118，X21=P1119，X22=P1120〜P1122，X23=P1123。
- 行を作らなかったもの：X8〜X10（`--features uhd`：遠隔の mutation image に libuhd が無い。**延期**：F1・F2 は feature つきの image で緑を確認した），X16・X18（待つ判断は `send` の中で，期限切れの検査は `step` の前段，`wanted` は hold の前に決まるので，その形の変異は作れない），入口 (a) の変異（`self.send(…); return true;` にすると M1 の `while tx.step() {}` が終わらず hang するので，有限時間で落ちる行にならない）。
- M8b の上限 5·IDLE は計画のまま（実行 20 回で落ちなかった）。
- `step` の `if next >= window_cut { return false; }` を消した（待つ判断が `send` の 1 箇所なので重複で，`next == window_cut` は `avail < wanted` で待つ。open は `step` の入口 (b) だけが作り，作った直後に `send` するので，first で device の SOB の open が次の step に残ることは無い）。それで phase7 の行 U19（その行の `>=` → `>`）は変異が等価になり，phase7 の `mutate.py` は `equivalent` を扱わないので**行を消した**。端の排他は M1（`<` と `<=`：P1109）と P637 が見る。B31 の `new` は `return;` → `return true;`（`abandon` が bool を返すため）。
- 実機の受け入れ手順は `plan/phase8/bench.md` (f)。X8〜X10 は延期のまま。

## セルフレビュー（1 回，`opencode-go/deepseek-v4.1-flash#max`；free は上限で止まった）への対応

P1 は 0 件。指摘 14 件（P2 7，P3 7）：

- 直した：1（`UhdDevice::raise_thread_priority` の拒否は `lost: false` にして返す。F2 が固定），2（M14e を足した），3（M4c：待つ continuation への Stop，tail の有無の 2 通り），4（P4 に COMMAND_REJECTED／DEVICE_LOST の event が無いこと，owner の join は 10 s の上限つき `txp_join`），5（新しい試験の `while step()` を 100 000 回上限の `txp_drain` に），6（この節に U19 の削除を記録），7（UR-2 の *Test* の文を狭めた），8（M10 に zero sample の位置），9（M1c を各 step 後に，M8b に 12.5 ms 未満。M4 の `unended_bursts`／`restarts` は FakeDevice の公開 API に無いので足さず，一つの device 開始と `tx_bursts` で見る），11（X6／X7 の対応），12（UR-23 の *Test* の位置，1 500 文字の上限のため 2 行に），13（P4 の重複を spy の thread 名の確認に），14（UR-3 の *Test* に F3 の文）。
- 退けた：10（CountingDevice に呼びの数を足さない：G2 は内側の `Recording` が呼びを見る。数えても読む試験が無く，計画の「数える」は上の「実装で足したこと」のとおり落とした）。
