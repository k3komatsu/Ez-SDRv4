STATUS: done

# 設計メモ40 — preemption bound のテスト計画（改訂）

## 1. 範囲・読んだ事実

- 今回はこの計画の改訂だけ。repository code、設計メモ、brief は変更しない。実装・build・test・mutation・hardware run はしていない。
- brief の前提は `HEAD d6ee79a`、未 commit は `plan/phase8/40-preemption-bound.md` のみ。本ファイルは repository 外に置く。
- コードで確認した現在の差：Mock は Action 受信時に `timeline::decided_at` と `LatePolicy::decide` を使い、`open` / `held` の前線は見ない（`crates/ezsdr-mock-radio/src/lib.rs:1593-1644`）。UHD は open burst の `next` を preempt 境界に使うが、open が閉じた後の `dispatch_start` は現在時刻・clock origin・device lead だけで決める（`crates/ezsdr-radio-uhd/src/provider/tx.rs:303-323,579-625,627-684`）。
- UHD の現在の `send` は buffer の開始で W を確認し、chunk 全体を送るため、1 chunk 分 W を越えうる（`tx.rs:319-323,383-395`）。有限 burst の最後の sample は条件により `tail` に保持され、EOB は後で送られる（`tx.rs:405-413,497-502`）。このため Kernel の `BurstRecord` だけで hand-over や EOB の時点を証明しない。
- Hardware の根拠は `plan/phase8/bench-results.md:49-57` と `design/18-uhd-radio.md` UR-23。`hw_b8_preemption` では20 msが通り、10/5/3 msはdrop、提出から約10.4–11.3 ms（VERIFIED）。profile の W=10 ms は現行値に由来し、最終的な hardware envelope は段階4の測定待ち（INFERRED）。FakeDevice のテストを実機測定の代わりにしない。

## 2. 単位・共通の観測方法

すべての時刻は integer tick と明示した SampleClock / root domain で扱い、float seconds に変換しない。

| 試験対象 | rate / clock | in-flight W | lead・その他 | 1 sample |
|---|---|---:|---|---:|
| Mock x3xx | 1 Msps。比較用 tick は TX SampleClock の sample index。T0=2 s の後に action を送る | 10 ms = 10,000 samples | Mock の minimum timed-command lead L=2 ms = 2,000 samples | 1 µs。root は1 GHzなので1,000 root ticks |
| Mock ideal | 同じ1 Msps | 0 | ideal の既存 lead / origin 判定を維持 | 1 µs |
| UHD FakeDevice | 1 Msps、200 MHz root | 10 ms | device lead L=2 ms、delivery allowance=3 ms。`block_len=2,000` samples | 1 µs = 200 root ticks |

- Mock は `Harness` / `ManualTimeAuthority` の明示 `step(t)`、TX BlockHeader（sample times・SOB/EOB flags）・`bursts`・`rejected`・`TIME_ERROR` event を観測する。時間を進めた値は root ns、start/end/count は SampleClock samples と書き分ける。
- UHD は `provider::test_support` の controlled-clock rig、`FakeDevice`、`ManualTimeAuthority`、既存の `Frozen` clock-participant pattern を使う。参加者を登録してから手動時計を進め、必要なら `device_at` で Core と FakeDevice を同時刻に合わせる。新規 interleaving は実際の `Control::book` → `Tx::step` を通し、手で `Held` を作って「渡し済み」にしない。FakeDevice の send 記録から送信 sample 値・個数・SOB/EOB metadata・time spec を読む（`provider/test_support.rs:20-91,167-175`）。
- sleep、実時間、test thread の起床順を verdict に使わない。UHD tests README の controlled-clock pattern（`crates/ezsdr-radio-uhd/tests/README.md:18-71`）に従う。

## 3. D1 — key / schema / profile / runtime

| 項目 | テスト案 | assert / 観測 | 落とす誤り |
|---|---|---|---|
| key と TimingEnvelope | `rm_01_register_twice_is_refused`（`radio_model.rs:117-125`、Vocabulary key count 34）、`rm_04_the_key_table_is_exactly_the_declared_one`（同:140-196）、`rm_20_schema_freeze`（同:375-412）を拡張 | `radio.timing.tx_in_flight_ns` は `Int`、non-coercible、`Reject`、capability-only。TimingEnvelope / generated `schemas/radio/envelope.v1.json` に8つ目の timing member があり、unknown member は拒否、schema freeze は byte-equal | key と struct / schema の片側だけ追加、kind や拒否動作の誤り、追加 property の取り残し |
| profile 値の掲示 | `mr_03_profile_values_reach_the_capabilities_and_the_envelope_section`、`ur_09_profile_values_reach_the_capabilities_and_the_envelope_section`、`rm_29_the_ideal_layout_builds_its_tree_and_defaults` を更新 | Mock x3xx と UHD x3xx の capability + envelope = 10,000,000 ns。Mock ideal と ideal Description = 0。掲示値と Description が一致 | x3xx / ideal の値の取り違え、capability と envelope の不一致 |
| Mock runtime | 系列10の ideal case。profile 値を変えず、既存 lead/origin 判定の回帰も維持 | W=0 の追加 front は ideal の既存振る舞いを変えない | Mock が10 msを固定値で使う、またはW=0で別のlate判定を足す |
| UHD runtime / D3 | 新規 `ur_21_tx_reads_in_flight_window_from_description`。`rig_on_description` に `tx_in_flight_ns=4_000_000`、1 Msps、`block_len=2,000`、now=0、repeat A start=+3 msを設定。`Control::book` 後 `Tx::step`。別 rig では predecessor なしで B=+5 msをbook | W=4 msなら A の FakeDevice send は+3 msから+4 msまでの1,000 samplesで止まり、cursorは4 ms。別 rig の+5 ms BはW外なのでhand-overされない。custom WのDescriptionを実際に読んだことをFakeDeviceの記録で確認 | UHDのdispatchだけDescriptionを使いsendは旧constant、片方だけprofile値を読む、またはwindow外のheldを送る |
| 定数参照 | 実装時に `IN_FLIGHT_WINDOW_NS` の現行参照（`crates/ezsdr-radio-uhd/src/profile.rs:14`、`provider/tx.rs:27,293,385,3528`）を全て追う。Stop testのoracleもDescription値にする | runtimeとStop boundのどこにも旧constantを残さない。変更後に `grep -REn 'IN_FLIGHT_WINDOW_NS' crates/ezsdr-radio-uhd/src` が0件 | productionだけ、またはtest oracleだけ旧値のまま残る |
| spec / schema の伝播 | 実装時に `design/07` RM-4/6/15/20/29、`design/09` MR-3/17、`design/18` §3/UR-21/23 と schema を同じ段階で更新。38a（`plan/phase8/38-timed-release-order.md:29-34`、commit `30082bd`）の形に従う | `schema_freeze` green。version、`schemas/SCHEMA_CHANGELOG.md` は未凍結なので変更しない | vocabularyだけ、またはschemaだけの変更。不要なversion bump |

## 4. D2'' — 前線の式、共有境界、P1 の三つの順序

同一 TX stream・同一 SampleClock で、Action を受け取った instant を `t`、Wを `tx_in_flight_ns` とする。Brief §1 S1 の D2'' が本節を置き換える。Mock の `handed(t)` は、Bより前に受け取った「走行中またはheld」の burst のうち、`start < t+W` を満たすものについて `min(end, t+W)` の最大値。receipt order が先なら同じ instant の A も数える。repeat の end は無限、候補なしなら front なし。SampleClock が違う候補は含めず、tickを直接比較しない。Closed device burst は Q2 の safe-start boundary を使い、隣接 continuation はclosed frontを作らない。Stop の `handed(s)` は同じ clock で安全な開始を過ぎるまで front として残す（D4）。

新しい B は target `< max(ordinary bound, H)` なら late、等値は on-time。lateなら `send_asap` は有効境界へ動かして `TIME_ERROR {send_asap}`、`drop_and_flag` は送らず `TIME_ERROR {drop}`。S1 の三つの順：同じ Mock step に A(+4 ms)→B(+3 ms) は Mock が B を late にする一方 UHD は B を on-time で先に出す；Aを前のinstantに受け取った場合は両方late；時間を止めAを `Control::book` → `Tx::step` で手渡してから同 instant に B を受け取ると両方late。Mock の doc comment は D2'' と Vision §59 の保守的 parity を一行で示す。Closed finite EOB の safe start は exclusive end+1 sample、空EOBの safe start は H+2 samples（S3）。W=0のactive candidateは将来boundを作らないが、閉じたEOBのsafe-start ruleは残る。

### 共有するものだけ

共有は`crates/ezsdr-radio/src/timeline.rs`のTX SampleClock tick上での「現在のearliest-start boundとoptional Hのどちらが後か」というpure arithmeticまでにする。Provider側でroot instant / sample latticeから同じTX clockへ変換し、通常の遅延判定は`LatePolicy::decide`、計算済みのfront latenessのpolicy mappingは共有`LatePolicy::outcome`を使う。Mockはprior-receiptのopen / heldからHを作り、UHDは実 hand-over cursorからHを得る。candidate ledger、clock conversion、device stateは共有helperへ持ち込まない。`tx_preemption_front(window_end, spans)`案は採らない。helper testはno front、current/frontの大小、H−1 / H / H+1 sampleの境界だけを固定する。

Mock側のcandidate reductionは別のtable-driven unit test `mr_17_handed_front_reduction_cases`（`crates/ezsdr-mock-radio/src/lib.rs`のreducer近傍）で、none、repeat、finite endがwindow内 / 外、startがwindow endちょうどで除外、2 candidateのmax、同じinstantにreceiptしたburstの除外を固定する。これは式のtestであり、P1のAction receipt integration case (§4)を置き換えない。

### P1 — 同じ受け取りinstantと、Aを先に受け取った場合

| Module | Test / location | 同じinstant（A → B の順に投入） | Aを先のinstantに受け取った場合 | 観測 / 落とす誤り |
|---|---|---|---|---|
| MockRadio | 新規 `mr_17_x3xx_tx_preemption_front` (`crates/ezsdr-mock-radio/tests/mock_radio.rs`) | x3xx、1 Msps。A target=t+4 msの後、B target=t+3 msを同じstep・同じ時刻の別step・t+1 root tickの別stepで受け取る。 | — | TX BlockHeader・BurstRecord・`TIME_ERROR` / rejection eventsを観測。三順ともMock Bはlate、send_asapはHへmove、dropは送られない。same-instant Aを数えない変異とearlier receiptを数えない変異を殺す。|
| UHD / FakeDevice | 新規 `ur_21_booking_receipt_order_changes_preemption` (`crates/ezsdr-radio-uhd/src/provider/tx.rs`) | manual now固定。`Control::book(A,+4 ms)`、`Control::book(B,+3 ms)`を両方済ませてから最初の`Tx::step`。 | Aを`Control::book`し、`Tx::step`で手渡してから同じmanual instant、または次instantにBを`Control::book`して`Tx::step`。 | FakeDeviceのsend順・samples・time spec、events、reservationsを観測。同じstepに予約しただけならBが+3 msで先にon-time。手渡し済みならBはcursorへmoveし、`requested_target`を保持。内部`Held`を直接注入せず、bookedとhandedの混同を殺す。|

同じinstantの二つのbooking、同じinstantでAを実際に手渡してからBをbook、Aの手渡しとBのbookingの間にclockを進める、の三つを別case・別rigで実行する。Mockは三つとも`SendAsapAndFlag`と`DropAndFlag`を通す。UHDは同じinstantの予約だけならon-time、同じinstantに実際に手渡してからの`DropAndFlag`はdropになることも通す。単なる予約順のunit testではなく、MockのAction receipt / UHDの`Control::book`と`Tx::step`を通る回帰にする。

UHDの同一instant hand-over `DropAndFlag` case は、`TIME_ERROR {drop}` と呼び出し前後で変わらないFakeDevice call数を assert し、SendAsap の move だけでなく拒否の分岐も固定する。

Mockの閉じたfront：`mr_17_held_cut_continuation_does_not_remember_a_closed_front` は、隣接したheld burstの開始で現在のburstが切られる場合、UHDがdevice EOBなしでcontinuationする位置にMockがclosed-device frontを残さないことをassertする。

## 5. D2'' — 系列1〜12の動作テスト案

以下の `t` はBを受け取るinstant。特記のないM/Uでは、Aはreceipt orderでBより先に受け取られている。UではAを実際に`Tx::step`し、表に書いたfrontまでFakeDeviceへ渡してからBをbookする。時刻差は1 Msps SampleClock上で、1 ms=1,000 samples。

| 系列 | Test / location | MockRadio: setup | UHD: setup | Assert / observe / 落とす誤り |
|---:|---|---|---|---|
| 1 | `mr_17_x3xx_tx_preemption_front`（新規）; `ur_21_booking_receipt_order_changes_preemption`（新規） | x3xx repeat Aより前receiptのB target=t+3 ms、`DropAndFlag`; same step / 同時刻後のstep / t+1 root tick後のstep | Aを`Control::book`し`Tx::step`で+6 msまでhand-overした後にB target=t+3 msをbook | MockはB drop、UHDもhand-over済みならdrop。events / reservations / actual samplesを見て、front無視または順序誤認を殺す |
| 2 | `mr_17_x3xx_tx_preemption_front`（新規）; `ur_21_booking_receipt_order_changes_preemption`（新規） | 同じA、B target=t+3 ms、`SendAsapAndFlag` | Aを先にhand-overしてからBをbook | `TIME_ERROR`、`requested_target`、Mock headers / BurstRecord、FakeDevice samples / metadataを観測。Mock BはHへ、UHD Bは実cursor+leadのboundへmove。AはB位置まで続く。move先・requested target・continuation誤り |
| 3 | `mr_17_x3xx_tx_preemption_boundaries`; `ur_21_a_preemption_front_boundary_is_on_time` | repeat A、B target=HとH+1 sampleを別case | Aを実hand-overしcursor=Hまで。BをHとH+1 sampleにbook | Mock events / headers、FakeDevice send / time specを観測。両方on-time、TIME_ERRORなし。Hちょうどをlateとする比較、またはHを越えるまで拒否する変異 |
| 4 | `mr_17_a_finite_burst_front_stops_at_its_end`; `ur_21_a_preemption_front_stops_at_a_finite_end` | A start=t+2 ms、end=t+8 ms、B target=t+5 msを同stepに受け、Mockはfinite endへmove | A start=+3 ms/end=+5 msをEOB closeし、B target=+6 msをその後Control.book | Mock BlockHeader / BurstRecord、FakeDevice A end / B start sample-timeを観測。Mock H=end、UHDのclosed EOB safe front=end+1。finite frontをt+Wに固定する変異 |
| 5 | `mr_17_x3xx_tx_preemption_boundaries`; `ur_21_a_finite_burst_inside_the_window_is_late` | finite Aのend=t+30 ms、B=t+5 ms、Drop | finite Aをt+10 msまで実hand-overしてBをbook | events、Mock record、FakeDevice samplesを観測。H=t+10 ms、B late / drop。finite endのみを採る、またはW clampをしない変異 |
| 6 | `mr_17_x3xx_tx_preemption_boundaries`; `ur_21_a_new_burst_without_an_open_predecessor_is_on_time` | 同一streamにopen / held predecessorなし、B=t+5 ms | predecessorなし。`Control::book(B)`→`Tx::step` | Mock record / event、FakeDeviceのtarget time specを観測。B on-time。frontなしで偽のboundを作る変異 |
| 7 | `mr_17_x3xx_tx_preemption_boundaries`; `ur_21_a_handed_repeat_sets_the_preemption_front` | held repeat A start=t+4 msを先にreceipt、B=t+6 ms | Aを`Control::book`して`Tx::step`をWまで繰り返し、cursorより前のBを後からbook | Mock `TIME_ERROR` / record、UHD event / FakeDevice cursorを観測。B late。Mockはprior-receipt Aを数え、UHDは実cursorを観測する。held burstを落とす変異 |
| 8 | `mr_17_x3xx_tx_preemption_front`; `ur_21_booking_receipt_order_changes_preemption` | Aを先にreceipt/step、B=t+3 ms（A.startより前） | Aを実hand-overし、B=t+3 msを後からbook | Mock headers / records、FakeDevice sendを観測。BはlateでAより前に出ない（Drop caseはsendなし）。target順だけ見て、先のinstantに受け取ったAの前へ挿入する変異 |
| 9 | `mr_17_x3xx_tx_preemption_boundaries`; `ur_21_a_preemption_front_boundary_is_on_time` | repeat A、B target=Hちょうど | Aのcursor=HをFakeDevice記録で確認後、B=Hをbook | Mock events / headers、FakeDevice samples / metadataを観測。on-time。Hをinclusive lateにする変異。系列3の境界を別pathでも固定 |
| 10 | Mock `mr_16_burst_refusals` ideal `moved_duplicate`; UHD `ur_21_tx_reads_in_flight_window_from_description` | `ideal` W=0でlate moveがheld startと衝突。origin/lead bound回帰を維持 | UHDにideal bindingはないためW=0行動は試さない。D1 custom Description=4 ms testのみ | Mockのmoved-start refusal、UHD FakeDevice send boundaryを観測。UHDはcustom Wを読むことだけを確認。UHD W=0 scheduleは追加しない |
| 11a — 最後のsampleを保持中 | `mr_17_a_finite_burst_front_stops_at_its_end`; UHD `ur_21_a_finite_burst_with_a_held_tail_keeps_its_front` | A start=t+2 ms、len=6,000 samples（exclusive end=t+8 ms）。B=t+5 msを同stepに受け取る | 同じfinite A。Bは`Control::book`で受け、`Tx::step`がheld tail/cursorを処理 | Mock/UHDともBを+8 msへmoveし、Aのtailは一度だけB前に続く。FakeDevice per-sample values / metadataでdevice EOBとtimed SOBが挟まらないこと、`requested_target`保持、late_by=3 msを確認 |
| 11b — payload EOBの後 | Mock `mr_17_a_one_sample_burst_uses_the_safe_sample`; UHD `ur_23_a_one_sample_burst_has_a_one_sample_safe_restart` | x3xx、Aはsample 2,004,000の一sample burst。Bをexclusive end (2,004,001)と次sample (2,004,002)で別case | Aのone-sample payloadをEOBつきで実際に送った後、BをAのexclusive endと次sampleでbook | Mock/UHDともexact-endはlate、exclusive end+1 sampleがsafe。Mock records/medium、FakeDevice A payload/EOB、BのSOB/time spec/sampleを観測し、one-sample EOBを他のfinite pathと混同しない |
| 12 — Stopしたrepeatのempty EOB | `mr_17_stop_preserves_front_until_expiry`; `ur_23_stop_preserves_handed_cursor_after_stop` | Stop時のpayload cursor Hに対してBをH、H+1、H+2で別case | UHDは実際のcursorをStopでempty EOBし、zero paddingがHに出る | Mock BurstRecord/medium、FakeDevice padding, timed SOB, sample, `BurstAck`を観測。H/H+1 late、H+2 on-time。empty EOBをfinite EOBと同じ境界にする変異 |

系列11の根拠：現行UHDは`open=None`後の`dispatch_start`だけでは先のhand-overを参照しない（VERIFIED、`tx.rs:579-585`）。ただし有限Aでは最後のsampleがtailに保持される場合があり、recordのEOBだけではdevice EOB送信済みとは言えない。UR-23 / `hw_b8_raw_empty_eob_gap` はEOBの直後に同じinstantで再startするhardware gap-zero失敗を記録し、次のsampleでのstartは`ur_23_a_burst_one_sample_after_a_burst_is_played`（`tx.rs:3544`以降）が境界を示す。FakeDeviceでのassertはsoftware pathの確認で、実機結果を新たに証明するものではない。

## 6. P2 / D3 — Wを越えないUHD hand-over

`ur_21_send_does_not_cross_the_in_flight_window`（新規、`tx.rs` unit tests）を追加し、系列3のcursor境界もこの実sendで確認する。

| case | 段取り | assert / 落とす誤り |
|---|---|---|
| 整列した境界 | UHD: 1 Msps、root 200 MHz、block_len=2,000、W=10 ms、now=0。repeat A start=+9 msを`Control::book`し`Tx::step`。続いてB target=+10 msをbookしstep | 最初のdevice chunkは+9,000から+9,999まで1,000 samples、exclusive cursorは+10,000 sample (=+10 ms)。+10 ms以後のsampleをFakeDeviceに渡さず、B at+10 msはon-time。現行の+11 msまでのchunk overshootが復活する変異 |
| 非整列sample lattice | UHD 390,625 sps、同じ10 ms W。`Tx::step`でwindow boundaryをまたぐchunkを作る | `ceil((now+W)` onto TX sample lattice`)`をexclusive first-unhanded boundaryにする。sample 3,906はhanded、sample 3,907（10.00192 ms）は未handed。FakeDevice samplesとcursorで確認し、B at3,906はlate、B at3,907はon-time。floor / 単位混同の変異 |

D3は「chunkのstartがW以内ならchunk全部を送る」扱いを退け、Wをstrict upper boundにする判断。ここでいうchunk endはexclusiveの次sample境界であり、境界以上のpayload sample自体を先渡ししない。UR-23 Stop boundも同じcursorで測る。現行`ur_23_a_stop_ends_the_burst_within_the_in_flight_window`（`tx.rs:3501-3542`）はaligned setupでchunk overshootを隠すため、D3後はDescriptionのWをoracleにし、FakeDeviceのpayload samples / cursorを`s+W`と照合する。同testが別に確認するempty EOBのzero-sample paddingと、record end+1 sampleでのBurstAckは維持してpayload boundと混同しない。もしFakeDevice記録でpadding自体がW後のhand-overになるなら、Stop sequenceかD3/UR-23の境界を実装前に明示して決め、overshootを黙って許容しない。

## 7. P4 / D4 — frontの寿命・stream分離・Stop

四つともcontrolled-time caseとしてMockとUHD両方で行う。Mockは実際のAction / TX headers / SimulationChannel samples、UHDは`Control::book` / `Tx::step` / FakeDeviceを通す。異なるSampleClockのtickは比較せず、必要な時刻関係は両方をroot instantへ変換してassertする。

| sequence | Test / location | MockRadio | UHD Module | assert / 落とす誤り |
|---|---|---|---|---|
| Stop後、残りのhanded区間内にB | 新規 `mr_17_stop_preserves_front_until_expiry` (`mock_radio.rs`); `ur_23_stop_preserves_handed_cursor_after_stop` (`tx.rs`) | x3xx repeat Aを先にreceipt。instant sでTX Stop。MockはHまで出し、その後BをH、H+1、H+2で試す | AをFakeDeviceへhand-overし、sでStop。送信済みsampleはrecallされず、empty EOBはHにzero sampleを足す。Bを同じcursor境界で試す | Mockはrecord sample count=H−A.startとSimulationChannelの遅れた切れ目を観測。UHDはpadding、timed SOB、sample、`BurstAck`を観測。H/H+1 late、H+2 on-time。frontをStopで消す変異 |
| old-clock front → cold change → new-clock B | `mr_17_cold_change_does_not_cross_sample_clocks` (`mock_radio.rs`); `ur_25_cold_change_does_not_inherit_old_tx_front` (`tx.rs`) | old clockにA/frontを作ってcold switchし、新SampleClock上の予定時刻でB | 旧clockのhand-over後にclock participantをmanualで切替え、新clockをbookして`Tx::step` | Bは新clockのtargetで評価しon-time。old frontとtargetのinstantはrootへ変換して比較し、新SampleClockへのfront漏れを検出。SampleClock index同士は比較しない |
| stream 0 front、stream 1 B | 新規 `mr_17_tx_front_is_per_stream` (`mock_radio.rs`); `ur_21_tx_front_is_per_stream` (`tx.rs`) | stream 0だけAを先にreceipt / front化。stream 1でBをreceipt | stream 0のTx ownerだけAをhand-over。stream 1の`Control::book`とTx ownerでBをdispatch | stream 1のBは自身のleadでon-time、stream 0のfrontは不変。stream別headers / FakeDevice stream IDを確認。instance-wide frontにする変異 |
| front expiry後のB | `mr_17_a_target_after_a_stopped_burst_is_on_time` (`mock_radio.rs`); `ur_21_a_target_after_a_completed_burst_is_on_time` (`tx.rs`) | Stop後のfrontの安全な開始を越えたBをreceipt | device timeを完了frontの安全な開始より後へ進め、Bをbook | Bはcurrent lead/originだけでon-time。状態の物理削除は実装自由であり、このcaseはfront保持を殺すmutationを主張しない |

O2／cursor後の再評価：`ur_21_a_send_asap_move_is_reported_once_across_handovers` は、Bが一度lateで動き、後のhand-overでordinary boundへさらに動かされても`TIME_ERROR`は1回、`requested_target`は保持、FakeDevice上の開始はsample 11,000となることをassertする。`ur_21_a_late_move_past_the_cursor_closes_before_a_held_burst` は、cursorより先へ動いたBのためAがdevice EOBで閉じ、cursorにheldされたCがdrop、Bが新しいtimed SOBで始まることをassertする。

### D4 — 既存Mock Stop test inventory

実装前のMock unit / integration inventoryを再確認した。直接の`Action::Stop` testsでは、期待sample数を変える既存x3xx caseはない。一方、受理テストのRun StopはD4で変わるため、`ke_03`を別行に加えて期待値を更新する。

| 既存test | 現行条件 / 対応 |
|---|---|
| `mr_25_a_device_stop_stops_every_layout_stream` (`crates/ezsdr-mock-radio/tests/mock_layout.rs:1119-1147`) | x3xxでTX targetsは3,000,000 sample ticks、StopはT0+10 ms（既定1 Mspsならsample 10,000）。差は2,990,000 samplesでW=10,000より外。前線に入らず2件ともcancelのまま。現在のreject countを変えない |
| `mr_09_each_transmit_stream_has_its_own_clock_and_line` (`mock_layout.rs:698-755`) | x3xx。Stopするstream 1に残るheld burstはStop時点のin-flight precursorでなく、stream 0のrecordは別stream。期待countは維持し、新P4 isolation caseは別に足す |
| `mr_25_a_stop_cuts_at_its_instant` (`mock_radio.rs:1981-2008`)、`mr_25_orderly_stop_ends_at_its_instant_abort_at_once` (`mock_radio.rs:2129-2154`) | x3xxの箇所も受信streamだけ。TX window / sample数の変更対象でない |
| `mr_25_rx_stop_leaves_transmit_running_and_stop_records_an_unstarted_burst` (`mock_radio.rs:2195-2230`) | RX Stop後にTXの単独burstを送る。preceding TX frontなし。unstarted ideal Stop subcaseはW=0。期待値維持 |
| `mr_25_a_stopped_burst_transmits_every_sample_before_the_stop` (`mock_channel.rs:609-641`)、`mr_25_a_stop_at_a_held_burst_s_start_silences_the_transmitter` (`mock_channel.rs:1093-1150`)、`mr_25_a_stop_on_a_sample_instant_does_not_transmit_that_sample` (`mock_channel.rs:1272-1287`)、`mr_25_a_stop_transmits_every_held_burst_before_it` (`mock_channel.rs:1519-1554`) | いずれも`Options::default()` / `two()`のideal profileでW=0。D4でsample countは増えない。既存assertを維持 |
| `ke_03_a_decision_after_the_stop_is_not_an_abort` (`crates/ezsdr-acceptance/tests/reactive.rs:315-339`) | x3xx Run Stopは26.040 ms、PINGは25.960 ms開始で1,000 samples。W=10 msなのでBurstは26.960 msでEOBまで流れ、期待countを80から1,000へ更新。Responderはdrain中に1回決定し、dispatch拒否がabortにならないことは維持 |
| 新規 `mr_25_stop_hands_a_future_repeat_through_the_in_flight_window` (`mock_channel.rs`) | x3xxでAのstart=s+4 ms、Stop at s。BurstRecord 6,000 samplesとSimulationChannel sample at the delayed cut / silence after itをassert |
| `mr_16_repeat_is_contiguous_across_wraps` (`mock_radio.rs:954-966`)、`mr_18_a_cold_transmit_change_replaces_the_tracker` (`mock_radio.rs:1245-1268`)、`mr_18_a_fractional_transmit_cut_is_the_first_sample_at_or_after_e` (`mock_radio.rs:2356-2378`) | ideal profile / cold-change cutで、D4のx3xx Stopではない。既存sample countsは維持 |
| `mr_25_a_stream_stop_keeps_the_pending_commands`、`mr_25_a_stop_records_a_later_update_as_cancelled_at_the_stop` (`mock_radio.rs:1120-1136,2603-2618`) | Stop時のTX front/sampleをassertしていない。update cancellationのassertは変えない |

これはテスト実行結果ではなく、実装前のtest setup inventoryを実装後にreconcileしたもの。`ke_03`はD4で変更が必要になった受理側のcaseとして追加した。

Cold change と device loss は D4 の Stop hand-over とは別の終端で、どちらも instant で切る。新規 `mr_32_x3xx_cold_change_cuts_at_its_instant` は x3xx repeat A を +2 ms から +5 ms の cold cut まで 3,000 samples 放射し、その後を silence とする。新規 `mr_20_x3xx_loss_cuts_transmit_at_its_instant` は同じ境界の device loss で record が 3,000 samples に止まることを assert する。

## 8. S2 — `moved_duplicate` と x3xx の別case

`mr_16_burst_refusals` の`moved_duplicate`はideal (W=0)へ移し、Aのheld startとlate SendAsapの移動先を重ねて、held-start refusalの目的を保つ。x3xxでは同stepに受け取ったA(2,003,000, length 2)をD2''が数えるので、B(2,000,000)は`2,003,002`へ動き、重なり拒否にならない。新しいcaseで`TIME_ERROR {send_asap}`、`requested_target=2,000,000`、`target=2,003,002`をassertし、x3xx Mockが重なり拒否の道に届くかをreportに記録する。

`mr_17_x3xx_moved_start_overlap_still_refuses` でもx3xxの到達性を固定する：先に受け取った repeat A（+4 ms）がH=+10 msを作り、Hちょうどから始まる held burst は厳密な`start < t+W`でfront候補から除外される。B（+3 ms）はHへ動いてそのheld startと重なるため拒否される。したがって元の二-sample配置では拒否されないが、x3xxのmoved-start overlap refusal path自体は到達可能。

## 9. Mutation row 案

実装時に各rowの`old`はsourceの完全一致文字列で記録する。ここでは誤りと殺すtestだけを指定する。

| mutant | 殺すtest |
|---|---|
| `tx_in_flight_ns` key / envelope / schemaを落とす、型・値を誤る | D1の`rm_04` / `rm_20` / MR-03 / UR-09 / RM-29 |
| MockまたはUHDがDescriptionのWを無視し固定10 msを読む | Mock ideal系列10、UHD custom Description=4 msのFakeDevice send境界 |
| same-instant receiptのAをBのcandidateに含める | P1同じinstant。B+3 msがlateにならずA+4 msより前に出ること |
| earlier-receiptのAを候補から外す | P1 earlier receiptと系列7 / 8 |
| `start < t+W`を`<=`にする / finite endをclampしない / maxをminにする | §4のMock reducer tableのhorizon equality・finite end・複数候補ケース、および系列4 / 5 |
| Repeatをfinite waveform lengthで切る、またはheld候補を落とす | 系列1 / 2 / 7 / 8 |
| current lead/origin boundをfrontで置き換える | 系列6とMock ideal系列10 |
| UHD sendでWを越えるchunkを渡す、またはsample latticeをfloorする | D3 aligned (+9 ms, +10 ms) と390,625-sps FakeDevice cases |
| UHDの`open`終了でcursor/tail/EOB boundaryを捨てる | 系列11a / 11b。FakeDeviceの実sample、SOB/EOB、time specをassert |
| 最後のheld sampleを落とす / 重複させる / 連続するburst間にEOBを挿す | 系列11aのheld-tail continuationとsample-by-sample FakeDevice record |
| EOB直後のexact-end startをon-timeにする | 系列11bの`+8 ms` late / `+8.001 ms` send record |
| Stop/cold changeでfrontを消す、別clock/streamへ漏らす | P4のStop・cold-change・stream-isolation tests |
| cold changeまたはdevice lossにStopのW hand-overを適用する | 新規x3xx cold-change medium caseとdevice-loss record case。どちらもcut instant後のsampleを許さない |
| Mock Stopを`s`で即時終了する、またはfrontだけ持って該当samplesを記録しない | D4 Stop testのBurstRecord sample count + last BlockHeader / EOB + B late assertion |
| send_asap move後に`requested_target` / `late_by`を失う | 系列2、11a/bと既存`moved_duplicate` |
| UHD再評価で同一burstの`TIME_ERROR`を二重に出す、またはcursor後のmoveでdevice continuationを壊す | `ur_21_a_send_asap_move_is_reported_once_across_handovers`、`ur_21_a_late_move_past_the_cursor_closes_before_a_held_burst` |
| Mockのheld continuationで閉じていないdevice burstのrestart frontを記録する | `mr_17_held_cut_continuation_does_not_remember_a_closed_front` |

## 10. 改訂の対応表

| 指摘 / 判断 | 本計画で扱う箇所 |
|---|---|
| P1 / S1 | §4の三つのreceipt/hand-over順、§5系列1–3・7–8 |
| P2 | §6の1 Msps / block_len=2,000の実send境界とFakeDevice assertion |
| P3 | §5系列11a（tail保持）・11b（EOB後）の分割、device samples / metadata / time spec、Mock exact-end guard |
| P4 | §7のStop後・cold change・stream isolation・front expiry、両Moduleのcontrolled-time test |
| S2 | §8のideal collisionとx3xxの2,003,002 move |
| P6 | §3系列10 / UHD custom-W positive testのみ。UHD W=0 behavior testなし |
| D2'' | §4のprior-receipt-order candidate formulaと§5の系列1–12 |
| D3 | §3 custom Description、§6のstrict send bound / sample-lattice boundary / Stop oracle |
| D4 | §7 Stop lifecycleとinventory。既存のideal / receive-only / W外ケースは維持、新x3xx active-TX Stop caseを追加 |
