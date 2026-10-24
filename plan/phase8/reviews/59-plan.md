STATUS: done

# #59 `cold_change_refused` — テスト計画（risky step）

| Field | Value |
|---|---|
| HEAD | `3109743` |
| 対象 | issue #59: UHD Module は `cold` change を適用して読み返し、値が違えば拒否する（spec 18 UR-12「Configuring a stream」, UR-13）。拒否された stream は halt し、次の `cold` change か `start_rx` まで続く（spec 22 VH-2、spec 07 RM-25・RM-34）。MockRadio は拒否しないので halt の道を Mock で再現できない（spec 07 RM-25 の Changes 2026-10-09 の行、`design/07-radio-model.md:457`） |
| この計画の範囲 | テスト計画だけ。コード、spec、schema、`mutations.json` は変えていない（案だけ）。ビルド、試験、mutation は走らせていない |
| 決定の出所 | `plan-brief.md` §1 の決定 1〜3（orchestrator）。§0 はその読みと、brief が決めていない点を分けて書く（一覧は §7） |
| 表記 | 経路は `path:line`（HEAD で読んだ行）。`T0` は Run の開始、`f = T0 + at_ns`、`e` は `cold` change の effective instant。値は k/M/G を付けずに書く（tick は root tick、1 GHz）。**INFERRED** はコードから導いた予測で、まだ走らせていない |

---

## 0. この計画が pin する振る舞い

### 0.1 決定 1 — 語彙と schema

- `FaultKind` に `ColdChangeRefused` を足し、serde 名を `cold_change_refused` にする（`crates/ezsdr-sim/src/lib.rs:33`）。既存 3 種の後ろに足し、doc comment を既存の形に合わせる。
- `schemas/sim/fault_entry.v1.json` の `$defs.FaultKind.oneOf` に 4 項目が入る。再生成は `EZSDR_UPDATE_SCHEMAS=1 cargo test -p ezsdr-sim --test sim_vocabulary se_12_schema_freeze`（`crates/ezsdr-sim/tests/sim_vocabulary.rs:168`）。
- version は上げない（brief 決定 1、AGENTS.md §6）。`sim` 1.1.0 のまま。
- `FaultsCheck`（SE-5、`crates/ezsdr-sim/src/lib.rs:141-190`）は target だけを見るので変更なし。`seed`、`channel` の check も変更なし。

### 0.2 決定 2 — 意味と、いつ効くか

brief の文面は「`at_ns` 以後に**始まる** segment のうち最初の、`cold` change が始めるもの」である。この計画は次で読む（**この読みが §7 の D1 で、orchestrator の確認が要る**）。

- 候補は、instance の各 line の `plan` の segment で、
  1. `by = Some(seq)`（Run の開始の segment は `by = None` なので入らない、決定 2）、
  2. `origin >= f`（`>=`。`origin == f` も候補）、
  3. 実際に始まるもの（`cut` が無いか `cut > 0`。`cut` は segment 内の sample index で、`cut == 0` は origin での cut なので始まらない）。
- 「最初」は `(origin, seq)` の最小。同じ instant なら `seq` の小さい方（決定 2）。move は `seq` が両 line で 1 つなので 1 回と数える（RM-34、`crates/ezsdr-radio/src/timeline.rs:508-521`）。
- 判定は、その時点の plan で行う（後に予約された change は候補に入らない。D1 の帰結）。
- **効く時**: その change が `e` に適用されるときで、かつ `f <= e` のとき（`apply_update` の中で、`self.updates` から取り出した後・`self.config` に入れる前）。`f` では何もしない。fault は **pending のまま待ち**、最初の候補の change を 1 度だけ拒否して spent になる。Run が先に終われば `stop`/loss の `record_pending_faults`（`crates/ezsdr-mock-radio/src/lib.rs:787-793`）が `applied: false` で書く。
  - 理由: 「効く segment が無いまま Run が終われば何も拒否しない」を pending と読む。resolve-at-`f` だと、fault の後に届いた change には効かないので、Session でこの fault を使えない。テスト 1 は change を `f` の後に予約して、この読みを pin する。
  - 遅れて効いた場合も fault の行の `at` は `f`（MR-27 の「`at` は fault の instant」）。`applied: true, lost: 0`。行を書くのは拒否した時点で、「fault が firing した時点で書く」の意味をこの kind だけ「効いた時点」と読む（§7 D8）。
- `e < f <= origin` の change（`e` で適用され、segment は `f` の後に始まる）は、`f <= e` の条件によりこの読みでは**拒否しない**。§7 D3 に挙げる。

### 0.3 決定 3 — 拒否のしかたと記録

- halt は語彙の `Lines::refuse(line, seq)`（`crates/ezsdr-radio/src/timeline.rs:508-521`。move は `refuse_move` に落ちて両 line、`:636-646`）。Mock に halt の規則を書かない。`replay` の `Kind::Cold`（`:931-947`）が「拒否された receive change は segment を持たず、tx の clock は origin で終わる」をすでに実装している。
- 拒否する change の値は **設定の記録**（RM-25 の Changes 2026-10-09 の行、#64）: `self.config` に入れ、`applied` にも 1 行書く（MR-18 の「At `e` the value applies and is recorded in `applied`」。UHD も拒否した configuration の read-back 行を `applied` に残す、`crates/ezsdr-radio-uhd/src/provider/core.rs:1089-1116`）。
- `radio.COMMAND_REJECTED` を 1 件（`action: null`、payload の `kind: "update_parameter"`）と、`ezsdr.radio.mock.rejected` に 1 行（`kind: "update_parameter"`、reason、`at`）。Mock の既存の `record_rejected_action`/`reject_action_at`（`crates/ezsdr-mock-radio/src/lib.rs:864-885`）を使う。
- reason の案（§7 D4）: 拒否された change は `"MR-18: the cold change was refused"`、拒否された change の clock に載っていた held burst は `"MR-18: cancelled by a refused cold change"`（UHD は `"UR-25: its transmit clock was never started"`、`crates/ezsdr-radio-uhd/src/provider/tx.rs:252`）。試験は prefix だけを見る。
- `rejected` 行と event の instant は `e`（§7 D2。UHD の rx は segment の origin、tx は switch の instant = `e`）。
- transmit: 拒否された change では `tx_switch` を呼ばない。`tx_reconcile(k)`（`crates/ezsdr-mock-radio/src/lib.rs:1311-1343`）を呼び、booking で登録された新しい clock をその origin で終わらせ（RM-25「a transmit clock registered for the refused change ends at its origin」）、その clock（plan の index が同じもの）の held burst だけを `rejected` に書く（後の change の clock に載った burst は残す）。旧 clock は計画の cut（`e`）で終わる。

### 0.4 実装が満たす制約（試験が前提にする。コードから導いた INFERRED）

- `advance` の候補（`crates/ezsdr-mock-radio/src/lib.rs:2011-2025`）にこの fault を入れると、resolved にならないまま同じ tick を拾い続けて無限ループになる。**候補から外す**（既存の `!fault.resolved` の filter に任せず、この kind を明示的に除く）。
- `emit_rx_until` の fault cut（`:587-590`）と `schedule_wakeup` の fault cut（`:810-819`）にも、この kind を入れない（受信 block を `f` で切ってはいけない。MR-20 の「every fault cuts the block」は受信を切る 3 種の話）。試験 8 が block の長さでこれを見る。
- 拒否の判定は `apply_update`（`:1948-1956`）の中で、cold の change にだけ行う。`hardware_timed` の update は segment を始めないので対象外（試験 4 が、halt 中でも適用・記録されることを見る）。
- `Lines::refuse` の後は、tx の stream で `tx_reconcile` を呼ぶ（plan が変わったため）。move は両 line。

---

## 1. 系列 1〜6

すべて `Harness::step(tick)` で `now` を固定し、block は各 step の後に link から読む。assert の「clock」は `ClockRegistry::sample_clock_records()`、「segments」は `ezsdr.radio.mock.segments`、「applied」は `ezsdr.radio.mock.applied`、「rejected」は `ezsdr.radio.mock.rejected`、「faults」は `ezsdr.radio.mock.faults`、「event」は `EventCollector::drain`。

37c の学び（`plan/phase8/reviews/37c-opus.md` §6）: 各試験は、拒否の効果を見る前に**場面に着いたこと**を assert する。たとえば試験 1 は、`f` を越えた step の後に fault の行がまだ無い（pending）こと、`step(e)` の後に旧 clock が `e` で終わっていること、そのあとで初めて「2 本目が無い」「block が無い」を見る。試験 6 は、拒否の前に新 clock が登録されている（`clocks_of` が 2 本）ことを先に見る。

新しい試験の名前は、HEAD の `crates/` を grep して既存の名前と衝突しないことを確かめた。

| # | どこで | 試験（新規／既存） | 場面 | assert | 観測 | 落とす誤り |
|---|---|---|---|---|---|---|
| 1 | MockRadio 統合、`crates/ezsdr-mock-radio/tests/mock_radio.rs` | 新規 `mr_20_a_cold_change_refused_fault_halts_the_stream` | `x3xx`、rate 1 Msps を `eq` で固定、`arm_start(2_000_000_000)`。`sim.faults` に `{at_ns: 1_000_000, fault: "cold_change_refused", target: "radio"}`。`step(T0+1 ms)`（= `f`、何も起きない）→ rate 1→2 Msps の `cold` を `at = T0+3 ms`（= `e`）で push（**`f` の後で予約**）→ `step(e)` → `step(T0+100 ms)` | s0 の clock は 1 本で `ended_at == T0+3 ms`、2 本目なし。block は `e` 以後なし（最後の block の `(first + len) · 1000 == 3_000`）。`rejected` は 1 行 `(update_parameter, "MR-18: the cold change was refused", at = e)`（reason は prefix 一致）。event は `COMMAND_REJECTED` 1 件、`payload.action == null`、`payload.kind == "update_parameter"`、`time == e`。`applied` は 1 行 `(radio.rx.s0.sample_rate_hz, 2e6, e)`。`faults` は 1 行 `{at: T0+1 ms, fault: "cold_change_refused", applied: true, lost: 0}`。`segments` は走っていた segment の `ended_at == e`、`e` 以後に始まる segment なし | clocks、link の block header、rejected／applied／faults／segments、event | 拒否しない。旧 segment を切らない。`COMMAND_REJECTED` を出さない／`action` を null にしない。`rejected` を書かない。`applied` を書かない。fault の行を `f` 以外の `at` で書く／`applied` を false にする |
| 2 | 同上（試験 1 の続きでよい） | 新規 `mr_20_the_next_cold_change_resumes_a_refused_stream` | 試験 1 の halt の後、同じ step（`now = e`）で `radio.rx.c1.stream = -1`（channel の exit）を `at = e+10 ms` で push → `step(e+10 ms)` → `step(e+100 ms)` | s0 の clock は 2 本目が `origin == e+55 ms`（= 2_058_000_000、x3xx の call 2 ms + allowance 3 ms + lead 50 ms。RM-25 の begin 規則、`crates/ezsdr-radio/src/timeline.rs:1000-1020`）、`root_ticks_per_tick == 500/1`（**拒否された 2 Msps が設定の記録**）、`channels == [0]`。その clock の block は index 0 から、`GAP_BEFORE` なし。`rejected` は 1 行のまま。`applied` は exit の行を `e+10 ms` で持つ | clocks、block header、segments、applied | `Lines::refuse` を呼ばず segment を飛ばすだけ（origin が `e+110 ms` になり、`e+55 ms` に clock が無い）。拒否された値を設定の記録に残さない（ratio 1000）。次の change も拒否する。halt が解けない |
| 3 | MockRadio 統合、`crates/ezsdr-mock-radio/tests/mock_layout.rs`（`start_rx` helper がある） | 新規 `mr_20_a_start_rx_resumes_a_refused_stream` | `ideal`、`rx: count(2)`（2 チャネルとも stream 0）、ports `["rx0"]`、rate 1 Msps、T0 = 0、fault `{at_ns: 1_000_000}`。`step(1 ms)`（= `f`）の後、rate 1→2 Msps を `at = e = 3 ms` で push → `step(e)`（試験 1 と同じ halt）。`start_rx("mock/rx/s0", Some(e+10 ms))` → `step(e+10 ms)` → `step(e+20 ms)` | s0 の clock は 2 本目が `origin == e+10 ms`（ideal の lead・call・allowance は 0）、ratio 500、`channels == [0, 1]`。block は index 0 から `GAP_BEFORE` なし。`rejected` は 1 行のまま。`applied` は拒否された rate の 1 行だけ。`faults` 1 行 `applied: true` | clocks、block header、segments、rejected、applied、faults | `start_rx` が halt を解かない。start の segment が拒否された set／rate でない。start を拒否する |
| 4 | 同上 | 新規 `mr_20_a_hardware_timed_update_in_a_refused_halt_is_recorded` | `x3xx`、`two()`、ports `["rx0"]`、fault `{at_ns: 1_000_000}`。`step(T0+1 ms)` の後、rate 1→2 Msps を `at = e = T0+3 ms` で push → `step(e)`（試験 1 と同じ halt）。`radio.rx.c0.gain_db = 6.0` を `HardwareTimed`、`at = h = e+10 ms` で push → `step(h)` | `applied` に `(radio.rx.c0.gain_db, 6.0, h)`。`rejected` は拒否の 1 行だけ。event は拒否の 1 件だけ。clock は 1 本のまま（halt のまま）。※ halt 中に値が channel の timeline に入ることは、medium 無しではサンプルに現れないので見ない | applied、rejected、event、clocks | halt 中に `hardware_timed` を拒否する。記録しない／`at` を `h` にしない。余計な `COMMAND_REJECTED` を出す |
| 5 | `mock_radio.rs` | 新規 `mr_20_only_the_first_cold_change_is_refused` | `x3xx`、1 Msps、fault `{at_ns: 1_000_000}`（`f = T0+1 ms`）。2 つ push（rate 2 Msps `at = e1 = T0+3 ms`、rate 4 Msps `at = e2 = T0+200 ms`）→ `step(T0+500_000)`（`f` の前）→ `step(e1)` → `step(e2)` → `step(e2+10 ms)` | 拒否は 1 回だけ。clock は [1 本目 `ended_at == e1`、2 本目 `origin == e2`（= 2_200_000_000）、ratio 250]。`e1+55 ms` に clock なし（拒否された segment は始まらない）。2 本目の clock で block が再開。`rejected` 1 行。`applied` 2 行（どちらも値は記録）。`faults` 1 行 `{at: f, applied: true}` | clocks、block header、rejected、applied、faults | fault が 2 度効く（2 つ目も拒否）。`(origin, seq)` の最小でなく最大を拒否。2 つ目の origin が `e2+55 ms` のまま（拒否後の replan が無い）。同じ instant の tie を `seq` でなく stream 番号で決める |
| 6 | `mock_layout.rs` | 新規 `mr_20_a_refused_transmit_change_ends_its_clock_and_cancels_its_bursts` | `x3xx`、`rx: elements(&["0:A:ubx:0"])`（link は付けない）、`tx: elements(&["0:A:ubx:0"])`、ports なし、`arm_start(T0)`、fault `{at_ns: 1_000_000}`。push → `step(T0+5 ms)` で rate 1→2 Msps の `cold` を booking（`at = e = T0+100 ms`。clock が登録される）。新 clock の domain を取り、その clock に burst（sample 100）、旧 clock に cut 後の burst（sample 150_000）を push → `step(T0+5 ms)` で booking → `step(T0+200 ms)` | 旧 clock `ended_at == e`。新 clock は `origin == e+50 ms == ended_at`（= 2_150_000_000、サンプル無し）。`segments` の tx 行も同じ origin／ended_at。burst は 2 本とも `rejected`（`kind == "tx_burst"`、prefix 一致）、`ezsdr.radio.mock.bursts` は空（BurstRecord は書かれない）。`rejected` に `update_parameter` 1 行。`applied` に拒否された rate の 1 行（`at = e`）。event は `update_parameter` 1 件 + `tx_burst` 2 件（どれも `action: null`）。`faults` 1 行 `applied: true` | clocks、segments、bursts、rejected、applied、event、faults | 拒否後も `tx_switch` する（新 clock が current になり block が出る）。新 clock を origin で終わらせない（`tx_reconcile` を呼ばない）。held burst を消さない。旧 clock を切らない |

## 2. 系列 7〜11

| # | どこで | 試験（新規／既存） | 場面 | assert | 観測 | 落とす誤り |
|---|---|---|---|---|---|---|
| 7 | `mock_layout.rs` | 新規 `mr_20_a_refused_move_halts_both_lines` | `x3xx`、`two()`、`radio.rx.c1.stream = 0`、ports `["rx0","rx1"]`、fault `{at_ns: 1_000_000}`、`f = T0+1 ms`。move `radio.rx.c1.stream: 0→1` を `at = e = T0+10 ms` で push → `step(T0+500_000)`（`f` の前）→ `step(e)` → `step(e+100 ms)`。そのあと `start_rx("mock/rx/s1", Some(e2 = e+10 ms))` → `step(e2+60 ms)` | s0 の clock 1 本 `ended_at == e`、s0 の block は `e` で終わる。s1 の clock は 0 本、s1 の block は無い。`segments` に s1 の segment なし。`rejected` 1 行（`update_parameter`）。`applied` に move の 1 行（`at = e`）。event 1 件。`faults` 1 行 `applied: true`。その後の start_rx で s1 の clock が 1 本 `origin == e2+50 ms`（= e+60 ms = 2_070_000_000）、`channels == [1]`（拒否された move の記録 set）、block は index 0 から | clocks、block header、segments、rejected、applied、faults | 片方の line だけ拒否（s1 が始まる／s0 が続く）。move を拒否しない。拒否された set で再開しない |
| 8 | `mock_radio.rs` | 新規 `mr_20_a_fault_after_every_change_refuses_nothing` | `ideal`、T0 = 0、fault `{at_ns: 1_250_000}`。rate 1→2 Msps の `cold` を `at = e = 500_000` で push → `step(e)`。`step(3_000_000)` → `stop(Orderly)`。`f = 1.25 ms` は change の後、block の途中（新 clock の sample 1 500） | `rejected` 空、`COMMAND_REJECTED` 0、`applied` 1 行。clock 2 本（[T0, ended e], [origin e]）。新 clock の block header は `[(0, 2000), (2000, 2000)]`（`f` の sample 1 500 で切られない）。`stop` の前は `faults` 空、`stop` の後は 1 行 `{at: f, applied: false, lost: 0}` | block header、rejected、event、faults | pending の fault が受信 block を `f` で切る（`emit_rx_until` の filter）。`f` で何かを拒否する。`stop` で pending を記録しない |
| 9 | `mock_layout.rs` | 新規 `mr_20_two_changes_at_one_instant_refuse_the_first_booked` | `ideal`、`rx: count(2)`、`radio.rx.c1.stream = 1`、ports `["rx0","rx1"]`、fault `{at_ns: 1_000_000}`（`f = 1 ms`）。2 つ push（s0 の rate 2 Msps と s1 の rate 4 Msps、同じ `at = e = 10_000_000`）→ `step(500_000)` → `step(e)` → `step(e+10 ms)`。push の順を入れ替えてもう一度 | 先に予約した stream（`seq` の小さい方）だけ halt: その clock は 1 本で `ended_at == e`、新しい segment なし、block は `e` で終わる。他の stream は 2 本目が `origin == e`、block は続く。`rejected` 1 行（`at = e`）。`faults` 1 行 `applied: true`。`applied` は 2 行（値は両方記録） | clocks、block header、segments、rejected、applied、faults | stream 番号の小さい方を拒否。`seq` の大きい方を拒否。両方拒否。同じ instant を `(origin, stream)` で並べる |
| 10 | `mock_radio.rs` | 新規 `mr_20_a_loss_before_the_refused_change_refuses_nothing` | `ideal`、T0 = 0。fault `{at_ns: 1_000_000}`、`device_lost` が 2 ms、rate 1→2 Msps の `cold` を `at = 3 ms`。3 つとも push → `step(1_000_000)`（成功）→ `step(2_000_000)` が `DeviceLost` → `stop(Abort)` | `faults` 1 行 `{at: 1 ms, applied: false, lost: 0}`。`rejected` は `update_parameter` の "MR-20: device lost" の行だけ（`e = 3 ms` の change が loss で拒否される既存の道）。`COMMAND_REJECTED` 0。block は `[0, 2_000)` だけ。clock は 1 本（loss の instant で終わる）。`e = 3 ms` の segment なし | block header、faults、rejected、event、clocks | loss の後に拒否する。loss で pending を記録しない。`device_lost` が失敗しない |
| 11 | `mock_radio.rs` | 新規 `mr_20_a_cold_change_refused_fault_acts_on_both_profiles` | `for profile in ["ideal", "x3xx"]`: 試験 1 と同じ halt。`ideal` は T0 = 0、fault `at_ns = 1_000_000`、`step(1 ms)` の後に change `at = e = 1_000_000`（**`e == f`**）を push → `step(e)`。`x3xx` は T0 = 2 s、`f = T0+1 ms`、change `at = T0+3 ms`。さらに `ideal`、`at_ns = 0`: change `at = 10 ms` を push → `step(10 ms)` | 両 profile で拒否（clock 1 本、`rejected` 1 行、`faults` `applied: true`、`applied` 1 行）。`at_ns = 0` の場合: change が拒否され、`0..e` の block は通常どおり（**Run の開始の segment は拒否されない**） | clocks、block header、rejected、applied、faults | profile 依存。`origin >= f` を `origin > f` にする（`e == f` の `ideal` が落ちる）。`by = None`（Run の開始）を候補に入れる（`at_ns = 0` で change が拒否されない） |

---

## 3. 決定 1〜3 の各項目

| 決定 | 項目 | 何を pin するか | 試験（新規／既存） | 観測 |
|---|---|---|---|---|
| 1 | variant と serde 名 | `cold_change_refused` が reader を通り、`FaultKind::ColdChangeRefused` になる | 既存 `se_03_faults_reader`（`crates/ezsdr-sim/tests/sim_vocabulary.rs:73`）を拡張: 4 番目の entry `{"at_ns": 20, "fault": "cold_change_refused", "target": "radio"}` を足し、`entries.len() == 4`、`entries[3].fault == FaultKind::ColdChangeRefused`、`at_ns == 20`。既存の "unknown" の case が綴り違いを拒否することをそのまま使う | `ezsdr_sim::faults` の戻り値 |
| 1 | schema | `fault_entry.v1.json` の `$defs.FaultKind.oneOf` に 4 項目 | 既存 `se_12_schema_freeze`（`sim_vocabulary.rs:168`）を再生成して green にする。**試験は足さない**（freeze が差分を見る） | `schemas/sim/fault_entry.v1.json` |
| 1 | version 据え置き | `sim 1.1.0` のまま | 既存 `se_01_register_adds_the_descriptor_and_the_three_checks` が descriptor を見る。追加の試験は不要（version を上げれば `se_01` が落ちる） | `ezsdr_sim::vocabulary()` |
| 2 | 候補の選び方 | `by = Some`、`origin >= f`、`cut > 0`（始まる segment）、`(origin, seq)` の最小、1 回だけ、move は 1 回 | 新規 1、5、7、9、11 | clocks、segments、faults |
| 2 | 効く時（待つ） | `f` の後に予約された change にも効く。Run が先に終われば何も拒否しない | 新規 1（`f` の後に予約）、8（`f` の後に change なし → `applied: false`）、10（loss が先） | clocks、faults |
| 3 | halt の規則を語彙から使う | 次の change の origin が「拒否された change の cut + lead」で決まる（Mock 独自の halt ではない） | 新規 2（`e+55 ms`）。既存の語彙の規則は `rm_34_a_refused_move_halts_both_lines_until_a_change_or_a_start`（`crates/ezsdr-radio/tests/timeline.rs:558`）と `rm_34_a_move_is_refused_on_both_lines_whichever_line_names_it`（`:943`） | clocks、segments |
| 3 | 記録 | `COMMAND_REJECTED`（`action: null`、kind `update_parameter`）、`rejected` 1 行、`applied` 1 行、fault の行 | 新規 1、6、7 | event、rejected、applied、faults |
| 3 | move | `Lines::refuse` が両 line を拒否 | 新規 7（両 line が halt、その後の start_rx が拒否された set で始まる） | clocks、segments |
| — | 語彙 | 新しい語彙 API は要らない（選び方が Mock の中にある限り）。既存 `Lines::refuse`/`refuse_move` で足りる | 追加なし。§7 D6 で、選び方を `timeline` に置くなら語彙の試験が要る、と書く | — |

---

## 4. spec の変更の一覧

「変更する」はこの計画では決めていない（orchestrator の裁可）。実装の commit が `design/` に書くべき差分。

| # | file:rule | 変更 | 試験 |
|---|---|---|---|
| 1 | `design/08-simulation.md` SE-3 | `fault: rx_overflow \| rx_sequence_error \| device_lost` に `\| cold_change_refused` を足す。`§5` の `se_03_faults_reader` の行の「three entries」を「four entries」に | `se_03_faults_reader`、`se_12_schema_freeze` |
| 2 | `design/08-simulation.md` SE-4 | この kind の意味を 1 文で: 最初の、`at_ns` 以後に始まる `cold` change の segment を拒否する（MockRadio の read-back 拒否の道）。`lost` は 0 | 系列 1、5、8〜11 |
| 3 | `design/08-simulation.md` 冒頭の Not in scope（`:7`） | 「Fault kinds beyond three」→「beyond these four」 | — |
| 4 | `design/08-simulation.md` §7（`:91`） | 「More fault kinds」はそのまま（将来の kind）。変更なしでよい。Changes に 1 行（2026-10-24、SE-3・SE-4、#59） | — |
| 5 | `design/09-mock-radio.md` MR-20 | 新しい段落: 候補の選び方（`by = Some`、`origin >= f`、始まるもの、`(origin, seq)` の最小、同じ instant は `seq`、move は 1 回）、効く時（change の `e`、1 回だけ、Run が先に終われば `applied: false`）、拒否の仕方（`Lines::refuse`、`COMMAND_REJECTED { action: null, kind: update_parameter }`、`rejected` 行、`lost: 0`）、受信 block は切らないこと、loss の後は効かないこと。`*Test:*` に新しい名前 | 系列 1〜11 |
| 6 | `design/09-mock-radio.md` MR-27 | `faults` の行: この kind は「効いた時点」に書き、`at` は `at_ns` の instant、`applied` は拒否したか、`lost` は 0。`*Test:*` | 系列 1、8、10 |
| 7 | `design/09-mock-radio.md` MR-18 | Ceiling の「the Mock never refuses a planned change at its instant … unless a fault is injected」はこの fault で真になる。`cold_change_refused` を指す 1 文と、拒否された change の値も `applied` に記録することを書く。`*Test:*` | 系列 1、2、4 |
| 8 | `design/09-mock-radio.md` §6 | 新しい 9 行（系列 1〜11 の表の内容を、既存の `mr_20_*` 行の形で） | — |
| 9 | `design/09-mock-radio.md` §6 の `mr_21_a_fault_cuts_the_lowest_running_receive_stream` の行 | 変更なし（新しい kind は受信を切らない） | — |
| 10 | `design/07-radio-model.md` RM-25（`:256`） | halt の段落に 1 文: MockRadio は spec 08 の `cold_change_refused` でこの拒否を起こす。`*Test:*` に新しい Mock の試験 | 系列 1、2、5、7 |
| 11 | `design/07-radio-model.md` RM-25 の Changes（`:457`） | 2026-10-09 の行の「only the UHD Module exercises the rule for now, since MockRadio cannot refuse yet (#59)」はこの変更で偽になる。Changes 表は日付ごとの記録なので、古い行は書き換えず、2026-10-24 の行を足して閉じる（「MockRadio も `cold_change_refused` で拒否でき、halt の道を両 Provider が踏む」）。orchestrator が古い行自体も直す判断なら、その行に「（MockRadio through `cold_change_refused`, #59, 2026-10-24）」を足す 1 行の編集になる | 系列 1、2、5、7 |
| 12 | `design/04-run-and-session.md`（`:7`） | 「fault injection beyond the three kinds of spec 08 SE-3 (`rx_overflow`, `rx_sequence_error`, `device_lost`)」→ four kinds、`cold_change_refused` を足す | — |
| 13 | `plan/maintenance/22-timing-simplification.md` | Follow-up 4（#59）を **Done 2026-10-24**（spec 09 MR-20）に。issue 表に `#59` を **Closed** の行として足す。VH-2 の「the read-back refusal is a MockRadio *Ceiling* and follow-up 4」はそのまま（決定の記録） | — |
| 14 | `schemas/sim/fault_entry.v1.json` | 再生成（`EZSDR_UPDATE_SCHEMAS=1 … se_12_schema_freeze`）。version は v1 のまま | `se_12_schema_freeze` |
| 15 | `handoff.md` | gate が閉じたときに 1 行（orchestrator が書く） | — |

---

## 5. mutation の行の案

`plan/maintenance/tools/mutations.json` に足す案。old/new の正確な文字列は実装後に書く（HEAD にコードが無いので、ここでは変える式と殺す試験だけ）。すべて、書いた後に「exactly once」を確かめる。

| id | 変える式 | 殺す試験 |
|---|---|---|
| M1 | 候補の選択を `(origin, seq)` の最小 → 最大（`min_by_key` → `max_by_key`） | 5 `mr_20_only_the_first_cold_change_is_refused` |
| M2 | 候補の条件 `segment.origin >= f` → `segment.origin > f` | 11 `mr_20_a_cold_change_refused_fault_acts_on_both_profiles`（`ideal` の `e == f`） |
| M3 | 拒否した後に fault を `resolved = true` にしない（2 度効く） | 5 |
| M4 | `emit_rx_until` の fault cut の filter から `ColdChangeRefused` を外す（pending の fault が `f` で受信 block を切る） | 8 `mr_20_a_fault_after_every_change_refuses_nothing` |
| M5 | 拒否を `Lines::refuse(line, seq)` でなく、その場で segment を飛ばす（`refuse` を呼ばない） | 2 `mr_20_the_next_cold_change_resumes_a_refused_stream`（origin `e+55 ms` → `e+110 ms`） |
| M6 | 拒否の `rejected` 行を書かない（`record_rejected_action` を落とす） | 1 `mr_20_a_cold_change_refused_fault_halts_the_stream` |
| M7 | 拒否の `COMMAND_REJECTED` を出さない（`reject_action_at` を `record_rejected_action` にする） | 1 |
| M8 | 拒否された change で `applied` の行を書かない | 1 |
| M9 | 拒否された transmit change でも `tx_switch` を呼ぶ | 6 `mr_20_a_refused_transmit_change_ends_its_clock_and_cancels_its_bursts` |
| M10 | 拒否の後に `tx_reconcile(k)` を呼ばない（新しい clock が origin で終わらない） | 6 |
| M11 | move の拒否で、名指された line だけ `refuse` する（`Lines::refuse` でなく `Line::refuse`） | 7 `mr_20_a_refused_move_halts_both_lines` |
| M12 | `stop_at` の `record_pending_faults` からこの kind を外す | 10 `mr_20_a_loss_before_the_refused_change_refuses_nothing` |
| M13 | 判定の条件 `f <= e` を落とす（fault の instant より前の change も拒否する） | **いまの系列には無い**（D3 の試験を足すまで記録しない） |
| M14 | 候補から `cut > 0` の条件を外す（始まらない segment を拒否する） | **いまの系列には無い**（`x3xx` で `e1` と `e2` が近く、1 つ目の segment が 2 つ目の change に origin 前で切られる場面を足すまで記録しない） |
| M15 | `runtime` の `faults` 行の `at` を `f` でなく拒否の instant にする | 1、8 |
| M16 | `FaultKind` の serde 名を `cold_change_refuse` にする（綴り違い） | `se_03_faults_reader`（4 番目の entry が拒否される） |

M13・M14 は殺す試験がまだ無いので、記録しない（足すなら場面も 1 つ足す）。

---

## 6. UHD Module と MockRadio で振る舞いが違う所（系列 4・6）

1. **`applied` の行の形（系列 4）.** UHD は channel ごとに `{key, channel, claimed, read_back, difference, at}` を書き（`crates/ezsdr-radio-uhd/src/provider/core.rs:1089-1116`）、halt 中に channel が無い `hardware_timed` は `{key, claimed, read_back: null, at, note: "no channel"}`（`crates/ezsdr-radio-uhd/src/provider/control.rs:676-690`）。Mock の行は `{key, value, at}` だけで、**`note` が無い**。したがって Mock の記録からは「channel が無かった」ことを読めない（値は channel の timeline に入り、次の segment で効く）。試験 4 はこの差を「Mock は普通の行を書く」として pin する。
2. **拒否された change の `applied`（系列 1）.** UHD は拒否した configuration でも apply した値の行を残す。Mock も 1 行残す（§0.3）。差は行の形（上）だけ。
3. **`rejected` 行と event の instant.** UHD の rx は segment の origin（`crates/ezsdr-radio-uhd/src/provider/rx.rs:306-321`）、tx は switch の instant（`crates/ezsdr-radio-uhd/src/provider/tx.rs:750-766`）。Mock は両方向 `e`（§0.3、D2）。
4. **reason の文.** UHD は device の read-back の文（`"UR-12: the device applied …"`）。Mock は規則の文（`"MR-18: the cold change was refused"`）。
5. **回数.** UHD は読み返しのたびに拒否できる。Mock は fault 1 つにつき 1 回だけ（決定 2）。
6. **系列 6 の burst の扱い.** UHD の `do_switch` は拒否の前に open burst を閉じ、旧 clock の held burst を「cancelled by a cold change」で `COMMAND_REJECTED` にし（`tx.rs:709-720`）、拒否の後は新しい clock を採用しない。booking で登録済みの clock は plan の拒否された segment が origin で終わるので、その origin で終わる。まだ current でない clock に載った burst は `"UR-25: its transmit clock was never started"` で取り消される（`tx.rs:242-255`、`CancelReason::ColdChange` なので `COMMAND_REJECTED` も出る、`crates/ezsdr-radio/src/lib.rs:139-142`）。**Mock も同じ**: 旧 clock の cut 後の burst は cold change の取り消し、拒否された clock の burst は「never started」に当たる取り消し。差は device 側だけ（UHD は in-flight の report を読み、`rx_stop` 行を持つ。Mock は無い）。試験 6 は Mock の 4 点（旧 clock の `ended_at`、新 clock の `origin == ended_at`、2 本の burst が `rejected`、block が出ない）を見る。

---

## 7. 設計の決定と合わない所・決めていない所

止めずに列挙する。D1〜D3 は実装の前に決めるべきもの、D4〜D12 は実装で決めてよいもの。

- **D1（要決定）: fault が効く時.** brief の「その instant に拒否する」は「segment が始まる instant」とも「fault の instant」とも読める。この計画は「change が `e` に適用されるとき（`f` では待つ）」を採り、テスト 1 がそれを pin する。resolve-at-`f` を選ぶなら、テスト 1 の場面は change を `f` の前に予約する形に変わり、テスト 8 の「fault の後に change が無い」の期待（`applied: false`）も「`f` で `applied: false`」に変わる。待つ読みでは、判定は decision の時点の plan で行うので、後に予約された change の segment が「最初」に変わることはない（その change は次の change として扱われる）。
- **D2（要決定）: `rejected` 行と event の instant.** この計画は `e`。UHD の rx は origin。どちらでも成立するが、`at` を試験で pin するので先に決める。origin を採ると、`e` と origin の間の `Stop` では拒否がまだ起きていない（fault は pending のまま `applied: false`）という違いが出る（系列には無い）。また tx は segment の開始を拾う場所が無いので、origin を採るなら D3 と同じ仕掛けが要る。
- **D3（要決定）: `e < f <= origin` の穴.** `e` で適用され、segment が `f` の後に始まる change は、この読み（`f <= e`）では拒否されない（brief の「`at_ns` 以後に始まる segment」には当たるのに）。直すなら「segment が始まる時点でもう一度見る」必要があり、tx は clock の origin を見る場所が要る（いまの Mock の `advance` に tx の segment 開始の候補は無い）。brief の系列はこの形を含まないので、この計画では試験を置かない。入れるなら新規 1 本（`x3xx` で change `at = e`、fault を `e < f < e+55 ms` に置く）。
- **D4: reason の文**（§0.3 の案）。試験は prefix だけを見るので、文を変えるなら 1 か所。
- **D5: `applied` に拒否された change の行を書くか.** この計画は書く（UHD と同じ、RM-25 #64 の「設定の記録」を直接読める）。書かないなら RM-27 の「per applied `UpdateParameter`」はそのままでよいが、テスト 1 の `applied` の assert を落とし、設定の記録はテスト 2 の ratio だけで見る。
- **D6: 選び方の置き場所.** Mock の中（この計画）か、`ezsdr_radio::timeline` の新しい helper か。語彙に置くと VH-8 の層 1 が独立に使える（§8）が、UHD はこの選び方を必要としない（device の読み返しが選ぶ）。Mock に置くなら語彙の API も試験も増えない。
- **D7: `lost` は 0.** RM-27 の「`lost` は fault が注入したもの」で、この kind はサンプルを失わない。試験 1・8 が pin する。
- **D8: MR-20/MR-27 の「fire した時点で書く」との関係.** この kind は `f` で何もせず、効いた時点で行を書く。MR-20 の 1 文をこの kind について読み替える必要がある（§4 の 5・6）。
- **D9: 停止中・未リンクの stream.** 停止中の stream への `cold` change は segment を始めない（`replay` の `on == false` の枝、`timeline.rs:931-947`）ので候補にならない。リンクの無い receive stream は plan に segment を持つが、Mock は `rx_next_segment` が `links.is_empty()` で `None` を返すので始めない（`crates/ezsdr-mock-radio/src/lib.rs:1247-1251`）。この計画の選び方は **plan 上の segment** を見るので、リンクの無い stream の change も拒否し得る（halt は観測できないが、`applied` の値と次の change の origin は変わる）。これは決めていない。推奨は plan を見る（device の読み返しは consumer に依らない）。
- **D10: `UR-12` の `prepare` の拒否（Rejected）は範囲外**（brief 決定 4）。この計画にも入れない。`prepare` の read-back 拒否は UHD Module の `prepare` の話で、Mock の `prepare` は read-back しない。
- **D11: 受信 block を切らないこと**を MR-20 に書くか、実装の注記だけにするか。spec 09 の `step` の疑似コード（`:242`）は「rx_overflow, rx_sequence_error or device_lost」を受信の cut として列挙している。この kind はその列に入らないので、疑似コードは変えなくてよいが、MR-20 の新しい段落に「この kind は受信 block を切らない」を 1 文入れるのが安全。
- **D12: 同じ instant の `seq` の意味.** 決定 2 の「予約の順（`seq`）」は Mock の `next_order`（`crates/ezsdr-mock-radio/src/lib.rs:1500-1505`、faults が先）と、`Lines` の `seq`（booking 順）のどちらか。この計画は change の `seq`（`Lines` の item の `seq`、`Kind::Cold` を booking したときの `order`）を採る。fault の `order` と混ざらないことをテスト 9 が pin する。

---

## 8. VH-8 の harness に拒否を入れるなら

`crates/ezsdr-mock-radio/tests/timeline.rs` の層 1（`rm_26_the_timeline_against_mockradio`、`:275`）は、generator（`crates/ezsdr-radio/tests/generator/mod.rs`）の `Fault::{Overrun, Sequence, Loss}`（`:84-91`）と `expect` の `Item.refused`（`:220` で常に `false`）で動いている。この fault を層 1 に入れるには、(1) generator に「拒否の fault」と、その fault が狙う change の index（`at_ns` だけでは、どの change を拒否するかを generator が決められない。`Fault` は kind しか持たない）を足し、(2) `expect` が Mock と同じ選び方（`(origin, seq)` の最小、`by = Some`、`origin >= f`、`cut > 0`）を実装して該当 `Item.refused = true` にし、(3) `layout_schedule` が `sim.faults` にその entry を入れ、(4) `Record` の比較に `refused` の plan と `faults` 行を足す——の 4 つが要る。問題は (2) で、選び方を `expect` と Mock の両方に書くと層 1 が「同じ規則を 2 回書いたものが一致するか」を見るだけになり、独立した検査でなくなる。入れるなら、まず選び方を `ezsdr_radio::timeline` の 1 関数（D6）にして両方がそれを呼び、generator が「どの change を狙うか」を持てるようにする。この item では入れない（UHD Module は `sim` を実装せず、拒否を fault で再現しないので、層 1 の価値より変更が大きい）。

---

## 9. 読んだもの（VERIFIED、HEAD = `3109743`）

- spec: `design/08-simulation.md` SE-1〜SE-5、§5・§7・Changes（`:7`, `:23-24`, `:87-91`）；`design/09-mock-radio.md` MR-18（`:149-155`）、MR-20（`:163`）、MR-21・MR-22（`:165-167`）、MR-27（`:182-183`）、`step` の疑似コード（`:230-250`）、§6（`:316-327`, `:415-431`）、Changes（`:438-450`）；`design/07-radio-model.md` RM-25（`:252-259`）、RM-34（`:319-325`）、Changes（`:457`）；`design/18-uhd-radio.md` UR-12（`:200-204`）、UR-13（`:205`）、UR-25（`:211` 以降）、§3 の値（`:62`）；`plan/maintenance/22-timing-simplification.md` VH-2（`:21`, `:376`）、VH-8（`:200`, `:424`）、issue 表（`:224-241`）、follow-up 4（`:246`）
- code: `crates/ezsdr-sim/src/lib.rs`（`FaultKind`:33、`faults`:109-125、`FaultsCheck`:141-190、`document_schemas`:211-230）；`crates/ezsdr-mock-radio/src/lib.rs`（`Fault {index}`:305、`section(faults)` の初期化:410、block cut:587-595、`record_fault`:777-786、`record_pending_faults`:787-793、`schedule_wakeup`:796-830、`advance`:2011-2100、`prepare` の faults:2255-2262、`start` の T0 加算:2414-2422、`stop_at`:1149-1180、`step` と `loss_due`:2514-2532、`handle_update`:1746-1946、`apply_update`:1948-1956、`tx_reconcile`:1311-1343、`handle_tx_burst`:1513-1625、`record_rejected_action`:864-875、`reject_action_at`:876-885）；`crates/ezsdr-radio/src/timeline.rs`（`Segment`:160-186、`Line::refuse`:248-259、`Line::halted`:349、`Lines::refuse`:508-521、`Lines::refuse_move`:636-646、`State::replay`:898-952、`begin`:1000-1020）；`crates/ezsdr-radio-uhd/src/provider/rx.rs:280-325`、`tx.rs:700-766`、`tx.rs:242-255`、`core.rs:522-548`、`core.rs:979-990`、`core.rs:1079-1116`、`control.rs:404-430`、`control.rs:599-700`、`control.rs:676-690`；`crates/ezsdr-radio/src/lib.rs:128-142`；`crates/ezsdr-radio/src/x3xx.rs:392-403`；`crates/ezsdr-mock-radio/src/profile.rs:130-150`
- tests: `crates/ezsdr-sim/tests/sim_vocabulary.rs`（`se_03_faults_reader`:73、`se_12_schema_freeze`:168）；`crates/ezsdr-mock-radio/tests/mock_radio.rs`（`mr_16_burst_refusals`:866、`mr_18_a_cold_rate_change_starts_a_new_sample_clock`:1106、`mr_18_a_cold_change_starts_its_clock_on_the_lattice`:1140、`mr_18_a_cold_transmit_change_replaces_the_tracker`:1213、`mr_20_faults_fire_at_their_instants`:1436、`mr_20_a_lost_device_delivers_every_sample_before_the_loss`:1677、`mr_20_a_lost_device_transmits_nothing_after_the_loss`:1693、`mr_20_a_loss_cancels_what_it_left_pending`:1725、`mr_20_an_action_after_the_losing_step_is_refused`:1766、`mr_20_a_fault_at_a_loss_s_instant_removes_nothing`:1788、helpers `fault_rows`:1623、`received`:1628、`rejected_rows`:1663、`rx_overflows`:1615）；`crates/ezsdr-mock-radio/tests/mock_layout.rs`（`mr_12_d14_the_actions_of_a_layout_instance_name_a_stream`:566、`mr_21_a_fault_cuts_the_lowest_running_receive_stream`:858、`mr_18_a_layout_hardware_timed_change_keeps_its_own_instant`:1097、`mr_20_a_lost_device_stops_every_layout_stream`:1150、`mr_40_a_receive_move_waits_for_release_and_lead_and_records_channel_order`:1209、`mr_40_a_refused_move_applies_no_update_and_changes_no_manifest_or_clock`:1314）；`crates/ezsdr-radio/tests/timeline.rs`（`rm_26_generated_sequences_keep_the_timeline_invariants`:309、`rm_34_a_refused_move_halts_both_lines_until_a_change_or_a_start`:558、`rm_34_a_move_is_refused_on_both_lines_whichever_line_names_it`:943）；`crates/ezsdr-mock-radio/tests/timeline.rs`（層 1: `:275`）と `crates/ezsdr-radio/tests/generator/mod.rs`（`Fault`:84-91、`Item.refused`:220）
- 表: `schemas/sim/fault_entry.v1.json`（`$defs.FaultKind.oneOf`）

---

## 10. 実装で足した行（impl-brief §2: 計画に無い状態・失敗の道）

| # | 足したもの | なぜ計画に無いか | 試験 |
|---|---|---|---|
| A1 | `ScheduledFault::fires()` — `cold_change_refused` を step の候補・受信 block の cut・wakeup から外す 1 か所 | 計画 §0.4 は 3 か所の filter を別々に書いていた。1 つの predicate にまとめた（同じ「この kind は瞬間を持たない」規則） | 1、8（`mr_20_a_cold_change_refused_fault_halts_the_stream`、`mr_20_a_fault_after_every_change_refuses_nothing`） |
| A2 | halt 中の送信 burst の拒否（`MR-25: the stream is halted by a refused change`） | 計画 S3(b) の「halt 中の burst は拒否」を実装する場所が無かった。`Line::halted(now)` を `handle_tx_burst` の入口で見る | 6（`mr_20_a_refused_transmit_change_ends_its_clock_and_cancels_its_bursts`）、mutation P613 |
| A3 | 拒否された change が同じ瞬間に始めた受信 segment の `segments` 行を消す（`forget_refused_segment`） | 自己レビュー 3 回目 P2。`advance` は同じ instant で `RxBegin`（rank 0）を update（rank 1）より先に処理するので、`ideal` のように origin == `e` のとき拒否された segment の行が `ended_at == origin` で残っていた。RM-25 が plan から除く segment に行だけ残るのは矛盾 | 11（`mr_20_a_cold_change_refused_fault_acts_on_both_profiles` の `segments` 行数）、mutation P621 |
| A4 | `PendingUpdate.cold` — change が名指す line | 選び方は plan の segment を見るので、change の line を知る必要がある | すべて（とくに 7 の move） |
| A5 | 拒否された送信 clock の burst だけを消す `cancel_booked` | 計画 §0.3 の「後の change の clock の burst は残す」を実装する部品 | 6a（`mr_20_only_the_refused_clock_s_bursts_are_cancelled`）、mutation P612 |

### 計画の期待値から変えた数（実装で確かめた値）

- 系列 2 の復帰 origin は `e + 60 ms`（計画は `e + 55 ms`）。復帰の change の受け取りが `e + 10 ms` で、RM-25 の ready 項は受け取りを含むため。
- 系列 2 の「次の `cold`」は `radio.rx.c1.stream = -1`（halt を本当に解く変更）。復帰後の set は `[0]`。
- 系列 9 の受け取りは `2 ms`（500 µs では最初の block がまだ出ておらず、場面の clock を assert できない）。
- 系列 6 の B の burst は clock #2 の sample 16 000（sample 0 では受け取り `later` との差が 2 ms lead に満たず `SendAsap` になり、記録の `late_by` が付くため）。
- 系列 6 の clock #2 の origin は `e + 100 ms`（2 Msps の segment の cut 50 ms + lead 50 ms）。計画の「e+50 ms」は単一 change の場合。
- S5(2) の 2 つの change は ideal ではなく x3xx の RX（origin `e+60 ms`）と TX（origin `e+50 ms`）の組で origin の順を逆にした（同じ profile の同方向どうしでは lead が同じで逆転しない）。

---

## 11. 直し 1 回目で足した行（fix1-brief §1）

| # | 足したもの | なぜ計画に無いか | 試験 |
|---|---|---|---|
| A6 | halt 中の burst の判定を burst の clock と，late policy が決めた start で行う：`halted` のとき，clock の lookup が選んだ clock の plan segment に `cut` があり，`start`（`SendAsap` の移動後）が `cut` 以上なら `HALTED_STREAM` | 計画 S3(b) と A2 は「halt 中は一律に拒否」だった（Sol のコードレビュー指摘 1）。UHD の `book_burst` は後の change の clock への予約を受けるので，Mock が厳しすぎた。後の change の clock の segment は `cut` が無いので，この判定を通り，clock の lookup が受ける。拒否された change の clock は cut 0 なので lookup が MR-16 の理由で断る（segment に sample が無い） | `mr_20_a_burst_on_a_clock_booked_after_a_refusal_plays`（新規），`mr_20_a_refused_transmit_change_ends_its_clock_and_cancels_its_bursts`（cut の後と，`SendAsap` が cut の後へ移す burst），mutation P613（一律に戻す）・P622（cut を見ない） |

系列（新しい試験，`mock_layout`）:

1. `x3xx`，channel mode（`radio.rx.c0.stream = 0`，`radio.tx.c0.stream = 0`，`sim.channel` の 1 本の coupling）。`cold_change_refused` を 1 ms に。
2. T0 で arm/start。A（2 Msps）を 100 ms に予約し，`step(0)` で clock 2 本（fault はまだ出ていない），`step(100 ms)` で拒否（`rejected == [REFUSED_CHANGE]`）。
3. B（4 Msps）を 300 ms に予約し，`step(290 ms)`：clock 3 本，burst はまだ無い（halt のまま）。
4. B の clock（`clocks[2].domain`）へ sample 16 000 の burst を push し，`step(290 ms)`：`rejected` は `[REFUSED_CHANGE]` のまま（**halt が B の burst を受けた**）。
5. `step(300 ms)`，`step(400 ms)`：B の segment で送信される — `BurstRecord` は 1 本，target `(B の clock, 16 000)`，`blocks == 1`，`samples == 100`，`end == Eob`；`stats.tx_blocks == 1`；受信 port の block に 0 でない sample がある（sample が media に出たこと）。

既存の `mr_20_a_refused_transmit_change_ends_its_clock_and_cancels_its_bursts` に足した 3 本目の burst: halt の間，旧の clock の sample 2 099 000（cut 2 100 000 より前）を `SendAsapAndFlag` で受ける．late なので policy が start を 2 102 000（cut の後）へ移す → `HALTED_STREAM`（`rejected()[2..]` の 3 番目）．移動を `cut` と比べないと受理され，cut で止まる open burst が残る．

期待値の根拠: B の clock の origin は `eb + 50 ms`（RM-25 の cut 50 ms + start lead 50 ms）なので，sample 16 000 は 4 ms 後＝2.354 s。受け取りは 2.29 s で，`decided = origin − lead`（2 ms）より前なので `OnTime`。B の burst は `later` に置かれ，`eb` の `tx_switch` で `held` へ移り，origin の後に送信される。
