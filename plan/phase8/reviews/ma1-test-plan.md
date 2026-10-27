STATUS: plan v6

# MA1 試験計画 v6：MockRadio の LO の位相を D9a の覆う規則にそろえる

## 先に読むこと：MA1 の範囲外の欠陥（Sol が code の道として VERIFIED，別の段で直す）

D9a の UHD の値の消失。MA1 は UHD の振る舞いを変えず，UHD の新しい試験も足さない。直すのは orchestrator が別の段で決める。

- **何が起きるか**：release が更新を古い plan の宛先（旧 S3，origin o3 > e）に Deferred で回した後，拒否の replan で S3 の origin が o3' に早まり，**e > o3'** になると，その更新は S3 に届かない（e == o3' は configure が拾うので失われない）。値が e より前に当たる経路は無い。
- **path:line**：Deferred は device を何も呼ばない（`control.rs:788-792`）。configure は `effective ≤ origin` の更新だけを設定に入れる（`control.rs:146-175`）。release で更新は `held` から消え，以後の fold が更新するのは configuration of record だけで走行中の device ではない（`control.rs:669-733`）。start は start だけを出し周波数は出さない（`rx.rs:331-334,447-465`）。refusal で S3 が早まる理由は `timeline.rs:953-964,1022-1040`。
- **到達（INFERRED）**：owner の refusal の処理が control の release より遅れる順で起こる。start が間に合う普通の予定でも（遅れの記録が出なくても）起こりうる：1 つの owner turn は refusal を booking して return し（`rx.rs:309-322`），次の turn が plan を読み直して S3 を configure し start を出す。eager な owner（`rx.rs:227-245`）は普通 C1 の booking の直後に refusal を処理するので，頻度は code から分からない。

MA1 の Mock の規則はこの順に依らない（D2）。

読んだもの（すべて HEAD a9b0a26 を `git show` で）：AGENTS.md §3，§6，§9。brief。`plan/phase8/42-d9-production-fix.md`，`reviews/d9a-opus-review.md`。`control.rs`（`reach`，`give`，`targets`，`release`，`ColdConfig`，`settings_on`，`configure`，`projections`）。`rx.rs`（`turn`，`begin`，`receive`）。`timeline.rs`。`x3xx.rs`，`profile.rs`。Mock の `lib.rs`（`handle_update` 1877，`PendingUpdate` 333，`apply_update` 2088，`apply_layout_channel_update` 2153，`untimed_tune`，`tune`，`rx_next_segment` 1315）。`design/09` の MR-18，27，30，34，39。Sol の 4 回の計画レビュー。cargo も試験も流していない。確かめていない所は INFERRED。

## 0. 決め

### D1 共有の判定：「t で覆う segment を返す」自由関数（v4 のまま）

`ezsdr_radio::timeline::covering(segments, channel, t) -> Option<&Segment>`：その channel を含み，origin ≤ t で，cut が無いか `instant(cut) > t` の segment（`instant` が Err なら覆わない）。前提（doc）：stream の owner が実際に走らせる segment だけを渡す（拒否された segment は planner が作らない：`timeline.rs:945-964`。受信しない stream の segment は渡さない）。UHD の `reach` は `.is_some()`。宛先の探索は UHD に残す。

Mock の adapter（private fn 1 つ）：`!self.rx[k].links.is_empty()` の受信 stream それぞれの `self.lines.line(rx_key(k)).plan` に `covering` を呼び，最初に見つかった `(k, origin)` を返す（link 無しは `lib.rs:1315-1318`，`:1586-1596`，`:1030` の理由で除く）。

退けた案：`Lines::running_at`，`Segment::passes` だけの共有，`channels::Description`，Kernel，`Segment::runs()`。

### D2 Mock の規則（v4 のまま）

受けた時刻 `r`（`handle_update` の `now`），効く instant `e`。

- 受けた時（`lib.rs:2032` の insert）：`PendingUpdate.timed_from = adapter(channel, r)`（`Option<(stream, origin)>`）。
- 適用の時：`adapter(channel, e) == timed_from`（`Some` で同じ stream・同じ origin）なら timed tune，それ以外は untimed tune。
- timed tune の結果：`random_unless_timed_tune` はその channel の timed-tune の定数で固定（`phase.set(e, timed_phase)`，`phase_fixed` 1），`random` は `mode.lo_retune` から 1 つ引いて invalid（`phase_fixed` 0），`deterministic` は 0 で valid（引かない）。
- untimed tune の結果：`random_unless_timed_tune` と `random` は `mode.lo_retune` から 1 つ引いて `phase.set(e, draw)`，`phase_fixed.set(e, 0.0)`。`deterministic` は引かず 0，`phase_fixed` 1。`frequency_hz.set(e, value)` は常に今のまま。
- 送信は変えない（D3）。plan を変える所に hook は置かない。
- 2 回の引き：宛先が非 first の segment なら更新の引きと origin の untimed tune の引きの 2 回。Mock の模擬の規約で実機の retune の数ではない（D9a は宛先に configure か release の untimed の一方で当てる：`control.rs:752-760`）。
- MR-30：引きの順は「更新の適用の順（e，到着順）」の後に「その instant の origin の untimed tune（rank 2）」のまま。

#### 保証は「前提」つき

Mock が timed と言うのは，受信が流れている間（origin ≤ r の segment が走っている間）に受けた更新で，適用の時にも同じ segment が e を覆うとき。UHD でそれが「release の時にも覆っている」ことまで言えるのは，次の **前提（assumption，VERIFIED ではなく，イベントの有無から導かない）** が成り立つときだけ：

> UHD の受信の owner は refusal を処理し，segment の timed start を origin の 2 ms 前までに出す。Mock は host の遅れを模擬しない。

owner の start の発行に origin の 2 ms 前という締め切りを課す code は無い（`rx.rs:304-334`：configure が終わったら `rx_start(origin)`；FakeDevice は `at < now` のときだけ late：`device.rs:1395-1402`，native も margin を検査しない：`uhd.rs:592-607`）。`LATE_COMMAND` が出ないことは 2 ms の margin の証拠にならない。前提が破れると UHD は `radio.LATE_COMMAND` と MissedStart を記録しうる（`rx.rs:447-460`）。その segment では保証が無い（MR-34 の *Ceiling:*）。

前提の下での筋（INFERRED）：
1. 始まった segment の origin は後の plan の変化で動かない（VH-8 `rm_26_*` の不変条件）。
2. Mock で origin ≤ r なら，前提により owner は S の拒否を処理し S の start を t_s ≤ origin − 2 ms に出している。
3. Mock の規則で e ≥ r + L_mock ≥ origin + L_mock（L_mock は Mock の command lead）。release ≥ e − W ≥ origin + L_mock − W（W は release の窓）。**t_s < release となる十分条件は L_mock + 2 ms > W**（Mock の 2 ms，窓 3 ms：2 + 2 > 3）。この数の十分条件を試験 19 が守る。
4. S は UHD の plan にあり（`core.rs:582-590`），release の時も覆う。release の後の cut は UHD が timed で渡す向きで，Mock は適用の時に cut を見て untimed（保守的）。

使う term は別々のもの：Mock の command lead 2 ms（`ezsdr-mock-radio/src/profile.rs:49-53`，`lib.rs:1913-1917`），UHD の広告の command lead 5 ms（`ezsdr-radio-uhd/src/profile.rs:11-22`：device lead 2 ms + delivery allowance 3 ms），UHD の timed update の booking の `DEVICE_LEAD_NS` 2 ms（`control.rs:603-609`），release の窓 3 ms（`x3xx.rs:398-400`）。広告の lead が等しいとは主張しない。上の筋は Mock の lead と Mock の規則だけに依る。

#### 保守的な Ceiling

受けた時にまだ始まっていない segment に宛てた先の更新（T0 より前に受けた T0 後の更新，stop 中に受けた更新，planned segment に宛てた更新）は，UHD が release の時に覆われて timed で渡しても Mock は untimed と数える。保証は「受信が流れている間に受けた timed の tune」。

### D3 送信は変えない（VERIFIED）

D9a は UHD の送信を変えていない（R4 は D9b）。UHD 4.10 の source で，送信の LO の timed tune は sample の流れに依らない（VERIFIED，Sol）：`x300_radio_control.cpp:146-148`，`bus_int.v:541-556,918-940`，`ctrlport_to_settings_bus.v`，`obx/obx_expert.cpp:132-141,210-211`。VERIFIED の範囲は LO の経路まで。

### D4 UHD の振る舞いは変えない

`reach` は `covering(..).is_some()` を呼ぶだけ。D9a の既存の試験がそのまま通る。MA1 は UHD の新しい試験を足さない（UHD の欠陥は冒頭の節のとおり別の段）。

## 1. 試験

置き場：`TL` = `crates/ezsdr-radio/tests/timeline.rs`（INFERRED：存在。無ければ `timeline.rs` の unit test）。`ML` = `crates/ezsdr-mock-radio/tests/mock_layout.rs`（harness：`built_with_inputs`，`Layout`，`update`，`start_rx`，`stop`，`heard`，`rotated`，`phase`，`clocks_of`，`section`，`rejected`。未来の T0 と T0 前の更新を受け付ける：`mock_layout.rs:295-302`）。`UC` = `control.rs` の unit test（FakeDevice，manual clock）。

**時刻の書き方**：「受けた時刻 r（その `step` の now）」，「要求の instant」，「実効の instant e」，「segment の実際の origin（booking + 50 ms 以降）」を別に書く。Mock の x3xx の command lead は 2 ms（`profile.rs:51`，`lib.rs:1913-1917`）なので **e ≥ r + 2 ms**。segment の origin は calibration run で `clocks_of(...)` から読み，本番の run の r と e に使う。回復の booking は更新の適用の後。

観測：位相は loopback の `rotated(0.5, tx_phase - rx_phase)`，期待は `SimRng::new(7, "mock/lo")` と `SimRng::new(7, "mock/lo/retune")` の n 番目。引きの数と順は sentinel（更新の後に stop と `start_rx` で非 first の segment の origin の untimed tune を起こし，その位相が retune の何番目か）。coherence は `section("segments")`。assert は最後の sample の位相と coherence の `valid`/`basis`。

red：deterministic，送信，走行中に受けた更新の回帰は HEAD で既に通る（実装前に落ちなくてよい）。落ちるべきなのは 1，3，4，5b，7，10a，13，14，15，16，17a，18。

| # | 名前 | 置き場 | 組み立て（r / 要求 / 実効 e / 実際の origin） | assert | 落ちる間違った実装 |
|---|---|---|---|---|---|
| 1 | `mr_34_a_layout_receive_update_before_t0_is_an_untimed_tune` | ML | x3xx，ubx の rx c0，seed 7，loopback の tx。`arm_start(T0)`，`step(0)`。更新を r = 0 に受け，e = T0 − 1 ms（≥ r + 2 ms）。first segment の origin は T0 | 最初に聞こえる sample の位相が `tx_phase - retune[0]`。first の行の `coherence` の最後が `none`/false | MA-1 の元の実装 |
| 1b | `mr_34_a_layout_deterministic_receive_update_before_t0_stays_zero` | ML | deterministic の channel（別 stream）に同じ更新 | 位相 0，`deterministic`/valid，引かない（別 stream の `random_unless_timed_tune` の最初の更新が retune[0]）。回帰 | deterministic まで引く |
| 2 | `mr_34_a_layout_an_update_received_while_samples_pass_draws_nothing` | ML | 1 の run に，`step(T0 + 3 ms)` の後，r = T0 + 3 ms に更新を受け，e = T0 + 5 ms（first segment が r に走り e も覆う）。その後 stop，`start_rx(at T0 + 30 ms)`（実際の origin は booking + 50 ms 以降） | e 以降の位相 `tx_phase - rx_timed`，coherence の最後が `at = e`，`timed_tune`/valid。再開の origin の位相が `tx_phase - retune[1]`（T0 − 1 ms の更新が retune[0]，走行中の更新は引かない） | 走行中の更新も untimed にする |
| 3 | `mr_34_a_layout_update_to_a_stopped_receive_stream_is_untimed_and_the_restart_adds_its_draw` | ML | stop（T0 + 6 ms，cut ≈ その時），r = T0 + 6.5 ms に更新（stream は止まっている），**e = T0 + 8.5 ms 以降**（r + 2 ms の lead）。**回復の `start_rx` は更新の適用（`step` で e を過ぎる）の後**に booking（実際の origin は booking + 50 ms 以降） | 更新が retune[0]，再開の origin が retune[1]，再開後の位相 `tx_phase - retune[1]`，origin の coherence 項が `none`/false。2 回引く規約を固定 | 止まっている channel の更新を timed と見なす |
| 4 | `mr_34_a_layout_update_before_a_planned_segment_is_untimed_and_the_segment_adds_its_draw` | ML | rx c0 を stream 0 から `-1`（cold，要求 T0 + 6 ms），`0`（cold，要求 T0 + 8 ms）。両方 booking 済みの後，r = T0 + 6.5 ms に更新，e = T0 + 8.5 ms 以降で戻りの segment の実際の origin より前（calibration で読む） | 3 と同じ引きの順 | 先の planned segment を覆いに入れる |
| 5 | `rm_covering_boundaries`（unit） | TL | 手で作った `Segment`（ratio 1/1，3/2，1/3）に対し，t = origin − 1，origin，origin + 1，`instant(cut) − 1`，`instant(cut)`，`instant(cut) + 1`，cut = Some(0)，cut = None，別 channel，空，2 segment，`instant` が Err。戻りの segment の origin も assert | origin − 1 は None，origin は Some(その segment)，`instant(cut) − 1` は Some，`instant(cut)` は None，cut = Some(0) は None，cut = None は Some，別 channel は None，空は None，2 つ目だけが覆う t で Some(2 つ目)，Err は None | `<=`/`<`，`>`/`>=`，channel 絞りの欠落，最初の要素しか見ない，Err を覆うとする |
| 5b | `mr_34_a_layout_an_update_at_a_cut_is_untimed` | ML | stop を T0 + 3 ms に booking（実効 T0 + 6 ms，cut の instant = c）。stop の後に r = T0 + 3.5 ms で更新を受け（S は走っていて covering(r) = S），e = c ちょうど（c ≥ r + 2 ms） | 適用の時 covering(c) は None なので untimed：更新が retune[0]，再開の origin が retune[1] | `instant(cut) > t` を `>=` にする |
| 6 | `mr_34_a_layout_deterministic_and_cbx_receive_channels_under_the_new_rule` | ML | deterministic の channel と `random`（CBX）を **別の stream** に置く。T0 前の更新 | deterministic の stream：位相 0，`deterministic`/valid，引かない。CBX の stream：`none`/false，更新ごとに 1 つ引く（`random` は timed tune の後も invalid）。既存の `mr_34_a_cbx_layout_redraws_only_the_retuned_channel` は通る。回帰を含む | 条件が `random_unless_timed_tune` 以外の挙動を変える |
| 7 | `mr_30_a_layout_untimed_update_draws_are_reproducible_and_in_application_order` | ML | 同じ seed の 2 run。c0，c1 に同じ e（T0 前，r < T0）の更新を c1，c0 の順に | 2 run がビット一致。c1 が retune[0]，c0 が retune[1] | 引きの順が channel 番号順 |
| 8 | UHD：D9a の既存の試験全部 | UC | 変更なし | `reach` が `covering(..).is_some()` を呼ぶだけで通る | 移植ミス |
| 9 | VH-8 | 既存 | 変更なし | `mock-radio/tests/timeline.rs:672-673` は timed gain のみ。理由を *Changes* に 1 行 | なし |
| 10a | `mr_34_a_layout_a_refused_segment_does_not_cover_an_update_received_after_its_planned_origin` | ML | calibration（fault 無し）：cold C1（要求 e_c1）の S2 の実際の origin o2 を読み，r = o2 + 数 ms に更新を受けると timed になることを確かめる。本番（fault あり）：同じ組み立てで `cold_change_refused` の fault が C1 を拒む（S2 の row は残らない）。S1 は拒否された C1 に切られ（`timeline.rs:953-964`），r に走る segment は無い。e ≥ r + 2 ms。更新の適用の後に `start_rx` | 更新が retune[0]，再開の origin が retune[1]，再開後の位相 `tx_phase - retune[1]`。拒否された S2 の行が `section("segments")` に残らない。`rejected()` は C1 の拒否の 1 件。calibration では timed | refused な segment を覆いに数える（planner の結果なので mutation 行は無く，calibration との対で kill） |
| 10b | `mr_34_a_layout_an_update_at_a_cold_instant_is_timed_whatever_the_refusal_order` | ML | cold C1 の実効 e_c を sample の格子から外れた値（T0 + 6_000_500，1 Msps で 1000 tick 刻み）にし，更新を S1 が走る間に受け（r ≤ e_c − 2 ms），e = e_c。S1 の cut は e_c の後の最初の sample（拒否の後も C1 は S1 を切る：`timeline.rs:953-964`）。(a) 更新の到着が cold の前，(b) cold が先。どちらも C1 は拒否 | (a)(b) とも S1 が r に走り適用の時も同じ S1 が e を覆うので timed：定数，coherence valid，retune を消費しない（再開の sentinel が retune[0]）。判定は coherence と sentinel のみ | 到着の順で結果が変わる実装 |
| 11 | `mr_34_a_layout_a_transmit_update_before_t0_stays_a_timed_tune` | ML | tx の ubx channel に T0 − 1 ms の更新（burst は T0 + 2 ms から） | tx の位相 `tx_timed`，tx の segment の `coherence` が `timed_tune`/valid，retune を消費しない（受信の再開の sentinel が retune[0]）。回帰 | 受信と同じ条件を送信にも掛ける |
| 12 | `mr_34_a_layout_the_other_streams_segment_covers` | ML | 2 stream（`radio.rx.c0.stream = 1`，`radio.rx.c1.stream = 0`，ports `rx0`，`rx1`）。stream 1 が走行中に r で c0 に更新を受ける | timed：`tx_phase - rx_timed`，valid，sentinel が retune[0] | stream 0 の line だけを見る |
| 13 | `mr_34_a_layout_a_linkless_stream_does_not_cover_even_after_start_rx` | ML | `…unlinked_receive_stream_takes_no_untimed_tune` の形：stream 1（c1）は link 無し。`start_rx("mock/rx/s1", Some(T0 + 3 ms))`（実際の origin は booking + 50 ms 以降），その origin を越えて進め（T0 + 120 ms），r = T0 + 130 ms に rx c1 の更新を受ける（e ≥ r + 2 ms）。stream 0 の stop，`start_rx`（sentinel） | c1 の更新は untimed で retune[0]，c0 の再開の origin の位相 `tx_phase - retune[1]`。`clocks_of("mock/rx/s1")` は空，`section("segments")` に rx stream 1 の行が無く，`blocks("rx1")` が出ない | link 無しを覆いに数える |
| 14 | `mr_34_a_layout_a_stop_after_the_update_makes_it_untimed_at_application` | ML | r = T0 + 3 ms に更新を受け（first segment が覆う），e = T0 + 8 ms。T0 + 4 ms に stop（更新より後の到着，cut ≈ T0 + 4 ms），`start_rx(at T0 + 10 ms)`（実際の origin は booking + 50 ms 以降 = e より後） | 適用の時 covering(e) は None：更新が retune[0]，再開の origin が retune[1]，再開後の位相 `tx_phase - retune[1]`，coherence の origin の項が `none`/false | 適用の時の確認をしない |
| 15 | `mr_34_a_layout_an_update_received_while_stopped_is_untimed_even_if_a_later_segment_covers_e` | ML | stop（T0 + 1 ms），r = T0 + 2 ms に更新を受け（stream は止まっている），e = T0 + 200 ms。T0 + 20 ms に `start_rx`（実際の origin は T0 + 70 ms 以降 ≤ e：適用の時には新しい segment が e を覆う） | 受けた時に始まった segment が無いので untimed：再開の origin の untimed tune が retune[0]，更新が retune[1]，e 以降の位相 `tx_phase - retune[1]`，coherence の e の項が `none`/false。保守的な Ceiling の固定 | 受けた時の覆いを見ず適用の時の `covering(e)` だけで決める（**この試験は M5 を殺さない**：restart は受けた後の booking で `adapter(e)` も受けた時は None を返す。M5 は試験 18 が殺す） |
| 16 | `mr_34_a_layout_a_flip_flop_stays_untimed` | ML | r = T0 + 3 ms に更新を受け（first segment S が覆う），e = T0 + 200 ms。T0 + 10 ms に stop（S が切れる），T0 + 20 ms に `start_rx`（新しい segment S' の実際の origin は T0 + 70 ms 以降 ≤ e：適用の時には S' が e を覆う。S' の origin は S と違う） | 適用の時の `covering(e)` は S' で，`timed_from` の S と origin が違うので untimed：再開の origin の untimed tune が retune[0]，更新が retune[1]，e 以降の位相 `tx_phase - retune[1]`，coherence の e の項が `none`/false | 適用の時に origin を比べない（**M6 を殺す**） |
| 17a | `mr_34_a_layout_an_update_received_before_a_refusal_advanced_segment_began_is_untimed` | ML | **calibration 3 run**。構成：S1 が走る。cold C1（要求 e_c1）が S2 を作るはずで fault で拒否される（S2 は origin o2 の前に拒否が適用され row が残らない）。**C2（要求 e_c2 > o2，S2 に 1 sample 以上残す）** を C1 の後に booking。run 0（fault 無し）：o2，o3 を読む。run 1（fault あり）：S3 の origin o3' < o3 を読み，C1 が拒否されたコマンド，拒否の前に S2 が plan にあった（run 0 の o2 が存在，run 1 でその row が無い），r と e を決める：**e_c1 < r < o3'，e ≥ r + 2 ms，o3' < e < o3**。本番：その r に更新を受ける | untimed：S3 の origin の untimed tune が retune[0]，更新が retune[1]，e 以降の位相 `tx_phase - retune[1]`，coherence の e の項が `none`/false。`rejected()` は C1 の 1 件。構成が成り立たない（o3' ≥ o3，S2 が無い，C1 が拒否されない）場合は **試験を外さず**，構成を直す（e_c2，e_c1 を動かす）か事実を報告に書き orchestrator が裁定 | `covering(e)` だけで決め受けた時の覆いを見ない実装（M5） |
| 17b | `mr_34_a_layout_an_update_received_after_a_refusal_advanced_segment_began_is_timed` | ML | 17a と同じ構成で **o3' ≤ r**，**e ≥ r + 2 ms**，e < o3 | S3 が r に走り適用の時も S3 が e を覆うので timed：定数，valid，retune を消費しない（S3 の origin の untimed tune が retune[0]，その後の再開の origin が retune[1]）。この規則の前提（owner が締め切りを守る：D2）は assumption と comment に書く | 始まった segment まで untimed にする実装 |
| 18 | `mr_34_a_layout_an_update_received_before_its_segment_began_is_untimed` | ML | `arm_start(T0)`，`step(0)`，r = 0 に更新を受け e = T0 + 5 ms（first segment は origin T0 ≤ e だが r < T0）。その後 T0 を過ぎて聞く | untimed：位相 `tx_phase - retune[0]`（T0 + 5 ms 以降も），coherence `none`/false。保守的な Ceiling の固定 | 受けた時の `covering(r)` を `covering(e)` で置き換える（M5） |
| 19 | `mr_34_the_release_window_is_covered_by_the_command_lead_and_the_assumed_start_margin` | ML | 名前の付いた term を **別々に** 読む：Mock の command lead（`ezsdr-mock-radio/src/profile.rs:49-53` の `terms()`，`lib.rs:1913-1917` が床に使う値），release の窓（`ezsdr_radio::x3xx` の `release_window_ns`，`x3xx.rs:398-400`），前提の start の margin（試験内の定数 `START_ISSUE_MARGIN_NS = 2_000_000`：owner が start を origin の 2 ms 前までに出すという D2 の前提そのもの） | 各 term の値を個別に assert（Mock の lead = 2 ms，窓 = 3 ms，margin = 2 ms）。D2 の数の十分条件 `mock_lead + start_margin > release_window`（2 + 2 > 3）を assert。UHD の広告の lead 5 ms，UHD の `DEVICE_LEAD_NS` 2 ms は別の term で，この試験は読まない（等しいとも主張しない。UHD crate の試験は足さない）。owner の予定や実機を確かめる試験ではない | lead を縮める，窓を広げる，margin の定数を変えて十分条件を壊す変更 |

## 2. 壊れる既存の試験と扱い（予測。cargo は流していない）

- **変わる（期待を直す）**：`mr_34_a_layout_untimed_tune_follows_a_timed_tune_at_its_instant`（ML:1743）：origin 宛ての更新は `harness.push` の後の `step(T0 + 20_000_000)` の中で受けられる（`mock_layout.rs:1770-1774`）ので，r = T0 + 20 ms，re-entry の origin（booking + 50 ms 以降，T0 + 60 ms 付近）より前。新しい規則では untimed で，e == origin の更新が retune[0]，origin の untimed tune が retune[1] を引く。期待を「at_origin は `none`/false，位相は retune[1]」に直す。この試験の「更新が先，origin が後」の rank の assert は，covered な更新が origin と同じ instant に来られない（r ≥ origin かつ e ≥ r + 2 ms）ので意味が変わる：**理由を報告に書き，試験名と spec の Test の記述を「untimed な更新も origin の untimed tune も同じ instant に引き，origin の引きが後」に直す**。
- 変わらない（INFERRED）：更新が走行中の segment に受けた時から適用の時まで覆われる試験。`mr_34_layout_mixed_boards_have_per_channel_timed_phase_behavior`（ML:1490：r = T0 + 3 ms，e = T0 + 5 ms），`mr_34_a_layout_receive_entry_is_an_untimed_tune`（5 ms の更新は T0 + 3 ms に受け first が覆う，104 ms の更新は T0 + 100 ms に受け再入場の segment が覆う），`mr_34_a_cbx_layout_*`，`mr_34_ideal_layout_*`，MC の `mr_34_a_timed_tune_replaces_the_random_phase_with_its_constant`（更新は `step(t0)` の後に受ける：r = t0 = first の origin），`mr_33_*`（start(0)，r = 0 = origin），`mr_32_b_*`。
- 変わりうる：更新を受けた時にまだ始まっていない segment に宛てた更新を持つ試験（`stream_enable`，`receive_move`，`rate_change`，`move_out` の MR-34 の試験）。`rg -n "frequency_hz" crates/ezsdr-mock-radio/tests crates/ezsdr-acceptance/tests` で全部拾い，r < origin の更新を数える。Sol は検索した周波数更新の試験に他の固定の期待値の変更を見つけなかった。落ちたら期待値を新しい規則で計算し直し，理由を報告に書く。黙って書き換えない。

## 3. spec（DA-1 の形）

- `design/07`：新しい規則（次の空き ID は `rules.py` と grep で）に `timeline::covering` の定義と，渡してよい segment の前提。`*Test:* rm_covering_boundaries`。UR-24 の文は新しい規則を引用する。
- `design/09` MR-34 第 2 段落：「Every applied frequency update (MR-18) is a `hardware_timed` command, a timed tune」を直す。案：

  「A receive frequency update is a timed tune only if, at the device time `r` the Mock received it, a segment of a receiving stream that holds its channel had begun and was not cut (`timeline::covering`, RM-xx), and the segment `covering` finds for its effective instant `e` when it applies is that same segment (same stream, same origin). A timed tune sets a `random_unless_timed_tune` channel's phase to its timed-tune constant, fixed from `e` on; draws one fresh phase from `SimRng::new(sim.seed, "<device>/lo/retune")` for a `random` channel, which is never coherent; and leaves a `deterministic` channel at phase 0, coherent, with no draw. Every other receive update is an untimed tune: a `random_unless_timed_tune` or `random` channel draws one fresh phase from `<device>/lo/retune` in update application order; a `deterministic` channel is unchanged. A `random_unless_timed_tune` channel is not coherent until its next timed tune. A transmit update is a timed tune. The Mock counts only streams that receive (a stream with no link is none, MR-39).」

  *Ceiling:* 保証は「受信が流れている間に受けた timed の tune」。受けた時にまだ始まっていない segment に宛てた先の更新（T0 より前に受けた T0 後の更新，stop 中に受けた更新，planned segment に宛てた更新）は，UHD が release の時に覆われて timed で渡しても，Mock は untimed と数える（保守的）。
  *Ceiling:* owner が refusal を処理し timed start を origin の 2 ms 前までに出すことを前提とする。Mock は host の遅れを模擬しない。前提が破れた segment では保証が無い（UHD は `radio.LATE_COMMAND` と MissedStart を記録しうる：`rx.rs:447-460`）。
  *Ceiling:* 更新の引きと，その後の segment の origin の引きは 2 回（模擬の規約で実機の retune の数ではない）。
  送信の更新が timed tune のままなのは UHD の送信の規則が変わっていない（D9b）ため。これは Ceiling ではなく *Changes* に書く（段取りの記録）。release の後の stop の FIFO の結末が bench 未確認なことは証拠の記録（`plan/phase8` の該当記録）に書き，spec の規則には書かない。
- MR-34 の *Test:* 行に試験 1，2，3，4，5b，6，7，10a，10b，11〜13，14，15，16，17a，17b，18，19 の名前を足し，`mr_34_a_layout_untimed_tune_follows_a_timed_tune_at_its_instant` の記述を §2 のとおり直す。
- MR-27：`timed_tune` の定義を「a receive channel's timed tune is one MR-34 counts as timed; a transmit channel's is every applied update」（受信に限る）。記録の形は変えない。
- `design/18` UR-24：3e が同じ行を編集中。MA1 は触らず，3e の後に「覆う」の文を RM-xx の引用に替える。
- *Changes* の表：MA-1，VH-8 の確認，D9b の段取り。`plan/maintenance/failure-mechanisms.md` の `mech:radio-rule-implemented-twice` に 1 行。D9a の UHD の値の消失（冒頭，Sol VERIFIED）は MA1 の範囲外として orchestrator が別に記録する。

## 4. mutation の行の候補（各行は 1 つの試験の 1 つの assert が殺す。殺せない行は書かない。正確な text は実装後。remote_mutate.py，各行がちょうど 1 回 match）

| # | 変更 | 殺す試験と assert |
|---|---|---|
| M1 | `covering`：`segment.origin <= t` → `<` | 5：t = origin で `Some(その segment)` を期待 |
| M2 | `covering`：`instant(cut) > t` → `>=` | 5：t = `instant(cut)` で `None`。5b：e = c の更新が untimed（retune[0]） |
| M3 | `covering`：`contains(channel)` を `true` | 5：別 channel で `None` |
| M4 | `covering`：`is_ok_and` を誤り（Err を覆うとする） | 5：Err の行で `None` |
| M5 | Mock：受けた時の呼び出しを `adapter(channel, r)` から `adapter(channel, e)` に | **18**：r < T0 ≤ e の更新が untimed（位相 retune[0]，`none`/false）。17a も同じ（r < o3'） |
| M6 | Mock：適用の時の比較を `== timed_from` から `is_some() && timed_from.is_some()` に（origin を比べない） | **16**：S と S' の origin が違うので untimed（retune[0]/[1]） |
| M7 | Mock：適用の時の比較を落とし `timed_from.is_some()` だけで timed | 14：stop の後 covering(e) = None で untimed（retune[0]） |
| M8 | Mock：adapter の走査を `(0..self.rx.len())` から `(0..1)` | 12：stream 1 が覆う更新が timed |
| M9 | Mock：adapter の link の絞り `!self.rx[k].links.is_empty()` を落とす | 13：c1 の更新が untimed で retune[0]（c0 の再開が retune[1]） |
| M10 | Mock：untimed の枝の `lo_phase(&mut mode.lo_retune)` を `model.timed_phase` に | 1：位相が `tx_phase - retune[0]`。3：再開の位相 |
| M11 | Mock：untimed の枝の `phase_fixed.set(e, 0.0)` を `1.0` | 1：first の行の `coherence` が `none`/false |
| M12 | Mock：送信の枝にも同じ条件を入れる | 11：tx の位相 `tx_timed`，retune を消費しない |
| M13 | Mock：deterministic も引く（`_ => 0.0` を引きに） | 1b：位相 0，引かない。6 |
| M14 | UHD：`reach` の `covering(..).is_some()` を `true` | D9a の既存の試験（8） |
| M16 | `ezsdr-mock-radio/src/profile.rs` の `min_timed_command_lead_ns: 2_000_000` を `1_000_000` に（数の十分条件 `L_mock + 2 ms > W` を守る行。owner の予定や実機を確かめる行ではない） | 19：`mock_lead + start_margin > release_window` が 1 + 2 > 3 で落ちる（個別の lead = 2 ms の assert も落ちる） |

**M15（UHD：`!planned.refused` の絞りを落とす）は消した**：RX の plan は拒否された segment を含まず（`timeline.rs:945-964`），UHD の entry はその plan から作る（`core.rs:582-590`）ので，絞りを落としても振る舞いが同じで，既存の refusal の試験（`control.rs:3846-3866`，plan が空）も落ちない。到達できる違反する呼び出し側が無い。

落とした行：v3 の sticky の hook の行，rank の行（covered な更新が origin と同じ instant に来られず区別できない：§2）。受けた時の「e も覆う」確認は適用の時の一致が含むので書かず，その mutation も無い。

## 5. 疑わしい所

1. **D2 の前提（assumption）**：owner が refusal を処理し start を origin の 2 ms 前までに出すことは VERIFIED ではなく，「LATE_COMMAND が出ない」ことから導かない（`rx.rs:304-334`，`device.rs:1395-1402`，`uhd.rs:592-607` に 2 ms の margin の検査は無い）。Mock は host の遅れを模擬しない。MR-34 の *Ceiling:*。
2. **冒頭の UHD の欠陥（範囲外）**：VERIFIED の code の道，到達は INFERRED。MA1 は直さず，UHD の試験も足さない。
3. **D2 の筋の 1 と 4 は INFERRED**：始まった segment の origin の不変（VH-8 の不変条件），S が UHD の plan にある（`core.rs:582-590`）。
4. **r の定義**：`handle_update` の `now`。`step` の now と一致すること（INFERRED）。
5. **既存の試験 1743**：§2。orchestrator が確認する。
6. **17a/17b の構成**：成り立たないときは外さず直す（Sol）。o3' < o3 は `begin` の `previous` が S2 の cut から S1 の cut に変わるため（Sol が `timeline.rs:953-964,1022-1040` で支持）。
7. **試験 19 の term**：Mock の lead は Mock の `Profile::terms()`，release の窓は `ezsdr_radio::x3xx::timing()` の値。UHD の広告の lead（5 ms）と `DEVICE_LEAD_NS`（2 ms）は別の term で，この試験は読まない。margin 2 ms は前提そのもの。
8. **link 無しの除外**，**first segment の行**，**spec 07 の ID**，**3e との衝突**，**mutation list の text**，**試験名**（`rm_covering_boundaries` は ID 決定後に付け替え）。

## 6. 実装の順

1. `covering`（試験 5）と UHD の `reach` の切り替え（`.is_some()`）。D9a の既存の UHD の試験（8）が全部通ることを確かめる。UHD の新しい試験は足さない。
2. Mock の試験（1〜7，10a，10b，11〜13，14〜16，17a，17b，18，19）を書く。red になるべきものが落ちることを確かめる（回帰は通ってよい）。
3. adapter，`PendingUpdate.timed_from`，Mock の受信の枝（D2）。送信の枝は変えない。plan を変える所に hook は置かない。
4. §2 の試験を直し，「変わりうる」試験を走らせ，理由つきで直す。
5. spec（§3）と mutation の行（§4：M1〜M14，M16 を `remote_mutate.py` で）。

## v5 からの変更

| 指摘／裁定 | v6 での扱い |
|---|---|
| UHD の欠陥は MA1 から外す | 17c と 17d を消した。UHD の新しい試験（旧 8b，14b，17c，17d）は足さない。冒頭を「MA1 の範囲外の欠陥（Sol が code の道として VERIFIED，別の段で直す）」の短い節にした：境界 **e > o3'**（e == o3' は configure が拾う），path:line（`control.rs:788-792`，`:146-175`，`:669-733`，`rx.rs:331-334,447-465`，`timeline.rs:953-964,1022-1040`），到達の条件（INFERRED：owner の refusal の処理が release より遅れる順。start が間に合う普通の予定でも起こる。1 turn は refusal を booking して return し，次の turn が S3 を configure：`rx.rs:309-322`）。MA1 は UHD の振る舞いを変えない |
| Major 1：試験 19 の term | 名前の付いた term を別々に確かめる（Mock の lead 2 ms：`profile.rs:49-53`，`lib.rs:1913-1917`。release の窓 3 ms：`x3xx.rs:398-400`。前提の margin 2 ms は試験内の定数）。UHD の広告の lead 5 ms（`ezsdr-radio-uhd/src/profile.rs:11-22`）と `DEVICE_LEAD_NS` 2 ms（`control.rs:603-609`）は別の term で読まず，広告の lead が等しいという主張は消した。「start は origin の 2 ms 前までに出す」を前提として明記し，「LATE_COMMAND が出ない限り」から導く文は消した（`device.rs:1395-1402`，`uhd.rs:592-607` に margin の検査は無い）。十分条件は `L_mock + 2 ms > W`。M16 は数の十分条件を守る行で，owner の予定や実機を確かめる行ではない。MR-34 の Ceiling の前提の文を「owner が refusal を処理し timed start を origin の 2 ms 前までに出すことを前提とする。Mock は host の遅れを模擬しない」に直した |
| 試験の一覧・mutation の表・実装の順 | v6 に合わせて整えた（§1 の試験は 1〜7，10a，10b，11〜13，14〜16，17a，17b，18，19 と既存の 8，9。mutation は M1〜M14 と M16。実装の順は UHD の新しい試験を含まない） |
| 変更の無いもの | D1，D2 の規則，D3，試験 1〜7，10a，10b，11〜16，17a，17b，18，M1〜M14，spec 07/27 の方針，3e の順序 |

## 実装で足したこと（実装役，2026-10-28）

- **RM-ID は RM-36**（RM-35 は計測 record で使用済み）。`covering` の unit test は `rm_36_covering_boundaries`（TL）。`covering` の引数は `impl IntoIterator<Item = &Segment>`（UHD は `Planned` から `refused` を除いて `&planned.segment` を渡す。Mock は `&line.plan`）。
- **試験 1b の置き場**：deterministic と非 deterministic は 1 つの instance に同居できない（deterministic は `ideal` だけ）ので「別 stream」は作れない。`ideal` で 1 stream、更新を T0 より前に受け、位相が 0 のまま（`(0.25, 0.0)`）で coherence が `deterministic`/valid なことで「引かない」を見る（引くと位相が回る：M13/P1011 が殺す）。試験 6 は CBX の 2 stream で、c0 の更新を r=0（untimed）、c1 の更新を走行中（timed）に受け、どちらも `random` なので 1 回ずつ引くこと（retune 0, 1）と coherence none を見る。
- **試験 10a の観測**：計画は「更新が retune[0]，再開が retune[1]」を timed の calibration と対にしたが，timed の calibration でも S2 の origin の untimed tune が retune[0] を引くので，再開の引きだけでは両者を区別できない。calibration は S2 の coherence が `timed_tune` で位相が `rx_timed` なことで timed を示し，fault 側（S2 が無い）は再開が retune[1]（timed なら retune[0]）で untimed を示す。
- **試験 7（MR-30）**：c0 と c1 を別の tx から loopback（tx 2 stream）にした。tx が 1 stream だと c1 の受信が無音で観測できない。
- **試験 5b**：stop は受けた時に効く（`Stop` に instant は無い）ので，「更新を T0+1 ms に受け，stop を T0+3.5 ms に受けて cut c，e = c」の形にした。cut は 1 回目の run で読む。
- **既存の試験**：`mr_34_a_layout_untimed_tune_follows_a_timed_tune_at_its_instant` は `mr_34_a_layout_an_untimed_update_and_the_entry_tune_at_one_instant_draw_in_application_order` に改名し，期待を「更新が retune[0]，origin の untimed tune が retune[1] を引き，origin 以降の位相は retune[1]」に直した（更新は origin より前に受けるので untimed）。改名は `rules.py --renames` で渡す。他の既存試験は落ちなかった。
- **足した試験（計画に無い）**：`mr_34_a_layout_a_transmit_origin_s_untimed_tune_follows_a_timed_update_at_its_instant`（ML）。理由：既存の mutation P187（untimed tune の rank を commands の前にする）は，rx の既存試験だけが殺していた。MA1 で rx の更新は origin と同じ instant に timed で来られなくなり（e ≥ r+2 ms かつ origin は cut の後），その試験は P187 を殺さなくなる。MR-34 の「origin の untimed tune はその instant の commands の後」は送信側でなお成り立つので，送信の timed update を新しい送信 segment の origin に置く試験で P187 を殺す（P187 の filter をこの試験に付け替えた）。
- **mutation の既存の行の直し**：P168，P589，P965（`reach` の `passes` の行），M63（phase3）は `old` の text が変わったので書き直した。P967，P968，P969，P989 は判定が `timeline.rs` の `covering` に移ったので `file` と `old`/`new` を移した（killing test は UHD の既存の試験のまま）。P187 の filter は上記の送信の試験へ。
- **P989 の注意**：`reach` の `holds`（`next` の選び方だけに使う）の channel の絞りを外す mutation は，MA1 の後は UHD のどの既存の試験でも殺されない（以前は `passes` を通して殺していた）。P989 は `covering` の channel の絞りに移して UHD の試験で殺している（killed）。`next` の `holds` の channel の絞りを直接見る UHD の試験は無い。UHD の新しい試験は足さない約束なので足さず，orchestrator に報告する。
- **新しい mutation 行**：P999〜P1013（計画の M1〜M14，M16）。M1〜M4=P999〜P1002（covering の unit），M5=P1003，M6=P1004，M7=P1005，M8=P1006，M9=P1007，M10=P1008，M11=P1009，M12=P1010，M13=P1011，M14=P1012（UHD），M16=P1013。すべて killed。（合流の時に +23 して P1022〜P1036 に振り直した）
- **spec**：`design/07` に RM-36（`covering`，test `rm_36_covering_boundaries`），§6 の行，*Changes*。`design/09` MR-34 の段落と Ceiling 3 つ，Test 行，§6 の改名行，MR-27 の 1 行（受信の timed_tune の定義），*Changes*。`plan/maintenance/failure-mechanisms.md` に 1 段落。`design/18` と `KNOWN_LIMITATIONS.md` は触っていない。

## 規則の差し替え（2026-10-28）

見落とし：Spec Run の時刻つきの Action は T0 より前に Provider に渡る（KC-17/KC-18）。受け取りの時の規則だと Spec Run の周波数の更新は Mock で常に untimed になり，UHD（release の時に決める）は受信中の予約の retune を timed で渡す。Sol の判定（`sol-ma1-rule.log`）：ADOPT WITH CHANGES。

- **新しい規則**：受信の周波数の更新は，適用の時に `timeline::covering` が e で見つける segment（link の有る受信 stream だけ）の origin が `e − W` 以下なら timed，それ以外は untimed。W は description の `release_window_ns`（Mock の `self.profile.timing()` から root tick に直す。値は写さない）。`PendingUpdate.timed_from` と受け取りの配線は消した。保守的な違いは (e − W, e] に始まった segment。前提（owner が timed start を origin の 2 ms 前までに出す）の Ceiling は残す。
- **試験**：timed に変えた：`..._while_stopped_is_timed_when_a_later_segment_covers_e`，`..._a_flip_flop_is_timed_by_the_segment_that_covers_e`，`..._an_update_received_before_t0_is_timed_when_its_segment_has_run_the_release_window`，`..._an_update_due_in_a_refusal_advanced_segment_is_timed`（旧 17a）。lead と margin の算術の試験は境界の試験 `mr_34_a_receive_update_is_timed_from_the_release_window_after_the_segment_origin` に置換（e = o + W は timed，1 tick 早いと untimed，T0 前の受け取り，Session の 2 ms lead の更新 e = T0 + 2 ms は untimed，T0 + 5 ms の受け取りは timed）。Spec Run の予約の regression：`mr_34_a_scheduled_receive_retune_inside_a_running_segment_is_a_timed_tune`（v58.rs，scheduled `update_parameter`，捕捉した sample の位相と segment の coherence）。T0 前の適用，cut，拒否された segment，link 無し，deterministic/CBX，送信の試験は変えていない。`mock_channel.rs` の `mr_32_b_...` は更新の e が origin の 2.5 ms 後（窓の内側）で untimed になって落ちたので，更新を T0 + 3.5 ms に動かした（試験の目的は送受の読み出し instant で，更新の種類ではない）。
- **mutation**（ID は P1022〜P1037）：P1022〜P1025，P1029〜P1035 は残した。P1026〜P1028 を「窓の判定を落とす」「等号を拒む」「適用の時の覆いを省く（常に timed）」に，P1036 を「Mock が窓を無視（0）」に置き換え，境界の試験で殺す。P1037（受信の更新は常に untimed）を足し，Spec Run の regression が殺す。P589 の old は元に戻した。
- **spec**：design/09 MR-34 の規則の文と受け取りの Ceiling を窓の規則と (e − W, e] の例外に替えた。design/07 RM-36 と design/09 の Changes の record 欄は `plan/phase8/reviews/ma1-opus.md` への link。design/18 UR-24 に RM-36 の引用。KNOWN_LIMITATIONS.md KL-32 を MA1 の後の事実に直した。
