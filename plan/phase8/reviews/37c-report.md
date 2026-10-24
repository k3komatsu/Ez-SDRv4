STATUS: done

# 設計メモ 37（#69）37c 実装 — 報告

HEAD `010fd26`（37b）から，37c の掃除を実装した。指示書 `37c-impl-brief.md` §1 の裁定（Sol の指摘をすべて採る）に従い，時刻の窓が判定そのものの系列を制御時計の unit test に移し，capture と device `Stop` は Spec で開始前に予約し，残りは時刻が動いても成り立つ assert にした。**本番のコード（`crates/*/src/` の `#[cfg(test)]` の外）は変えていない。** `cargo fmt`/`rustfmt`，ローカルの `mutate.py`，`git add`・commit・push・stash はしていない。作業 dir の orchestrator のファイルと見張りのプロセスは触っていない。

## 1. やったこと（ファイルごと）

- `crates/ezsdr-radio-uhd/src/provider/control.rs`（`#[cfg(test)]` の中だけ，+16 本）
  - **§2**：`ur_25_a_receive_enable_counts_its_start_lead_from_the_end_of_its_configuration`。`FakeConfig::apply_delay` 80 ms の制御時計で，from-none enable の構成の `apply` に probe を置き，参加者 thread がそこで止まっている間に時計を 200 ms へ進める。`cold_change` の行の `booked_at` 0，`e` 100 ms，`ready` 200 ms を別々に assert し（(a)），`origin` が `ceil_lattice(max(e, ready + lead))` に厳密に等しいこと（(b)）と，owner の `rx_start` がその origin であることを assert する。構成の終わり `c` は 200 ms で既知。
  - **(4)**：`ur_25_the_switch_applies_the_configuration_in_effect_at_e2`，`ur_25_an_enable_at_a_timed_update_s_instant_applies_it`，`ur_24_a_channel_keyed_timed_update_acts_on_its_own_channel`，`ur_24_a_channel_keyed_command_counts_one_queue_slot`，`ur_25_a_layout_cold_change_is_floored_at_every_earlier_cold_change_of_the_instance`，`ur_25_a_layout_cold_change_of_a_linkless_receive_side_is_floored_at_t0`，`ur_24_the_layout_destination_follows_the_effective_membership`，`ur_25_a_same_size_membership_swap_reopens_the_streamer`（record の `{0}`→`{1}` で同じ個数，`owner_configure` が `rx_open 0 [1]` を出し，TX 側も同じく `tx_open 0 [1]`），`ur_34_a_timed_update_crossing_the_move_reaches_each_streamer_channel_at_its_instant`（move の前後と destination の configure，`device.read_back` で 10→20 MHz），`ur_34_a_cold_change_before_the_move_s_release_is_floored_at_the_move`，`ur_34_two_moves_at_one_instant_swap_their_receive_channels_without_deadlock`（`control.rs` は plan の 2 つの cut が同じ instant・union segment を持たないこと，`rx.rs` は両 `Rx` owner が旧 segment を cut まで受信して自分で complete し，両方が `rx_open 0 [1]` / `rx_open 1 [0]` に着くこと），`ur_34_two_moves_at_one_instant_swap_their_transmit_channels_without_deadlock`（`control.rs` は plan と zero-length の union segment，`tx.rs` は両 owner が union を飛ばして両方の destination clock に着くこと），`ur_34_a_finite_middle_segment_releases_a_channel_along_a_move_chain`（`control.rs` の plan と `rx.rs` の A・B・C の real owner，B の受信と `rx_close 1 < rx_open 2 [1]`），`ur_25_a_floor_raised_layout_join_is_judged_at_its_instant`，`ur_26_d14_a_layout_instance_names_its_streams`（held burst と `Stop` の部分），`ur_25_a_receive_enable_counts_its_start_lead_from_the_end_of_its_configuration`（§2）。部品は既存の `manual_rig`・`manual_rig_events`・`hold`・`Frozen`・`device_at`・`probe`・`host.spawn` と，新しい helper（`timing_of`，`plan_of_stream`，`plan_rows`，`both_description`，`both_rig`，`both_control`，`layout_control`）だけ。新しい rig の種類・時計の hold・fake の fault は足していない。
- `crates/ezsdr-radio-uhd/src/provider/tx.rs`（`#[cfg(test)]` の中だけ，+1 本）：`ur_34_two_moves_at_one_instant_swap_their_transmit_channels_without_deadlock`。2 つの move を同じ instant に予約し，tx owner が zero-length の union segment を開かず（`tx_open` も `apply` も出さない，clock も取らない）次の segment を読むこと，union segment が reciprocal の destination を待たせないこと（`needed_before`）を assert する。
- `crates/ezsdr-radio-uhd/src/provider/rx.rs`（`#[cfg(test)]` の中だけ，+1 本）：`ur_17_a_finite_segment_delivers_through_its_cut`。有限の中間 segment が cut まで受信し（block が cut を越えない），次の segment が自分の origin から受信することを，stream 1 の rig（`rig_with_links_on`）で見る。
- `crates/ezsdr-radio-uhd/tests/fake.rs`
  - 14 本を削除し，それぞれの場所に移した先の unit test だという 1 行を残した（`ur_25_the_switch_applies_…`，`ur_25_an_enable_at_a_timed_…`，`ur_24_a_channel_keyed_timed_…`，`ur_24_a_channel_keyed_command_…`，`ur_25_a_layout_cold_change_is_floored_…`，`ur_25_a_same_size_membership_swap_…`，`ur_34_a_timed_update_crossing_…`，`ur_34_a_cold_change_before_…`，`ur_34_two_moves_…` の 2 本，`ur_34_a_finite_middle_…`，`ur_25_a_layout_cold_change_of_a_linkless_…`，`ur_25_a_floor_raised_…`，`ur_24_the_layout_destination_…`）。この回に 74・63・64 の 3 本も削除し，`control.rs`／`tx.rs` の unit を名指しした（§10）。
  - **(1)**：`captures_across_cold_changes`（3 本の呼び出し）を Spec にした。各 capture は output の `sink.capture_samples` の timed update（200 + 320·i ms），各 rate change は `radio.rx.s0.sample_rate_hz` の cold update（264 + 320·i ms）で，T0 からの offset（受信 stream の clock の tick）。`ur_25_a_layout_rate_change_cuts_its_stream_and_configures_every_channel` は Spec（timed 320 ms・cold 300 ms）にした。`ur_26_a_device_stop_stops_every_layout_stream` も Spec（burst 100 ms・instance の Stop 50 ms）にした。
  - **(2)・W**：`ur_07_now_tracks_the_device_while_no_call_runs`（装置の 2 つの読みの bracket，slack 1 ms），`ur_34_a_move_into_an_empty_stream_…`（move の `e` は `max(要求, 記録された booking)`），`ur_25_a_refused_enable_from_no_stream_…`（同じ式），`ur_21_a_burst_targets_its_own_transmit_stream`（意図した late burst の直前に TIME_ERROR の境界を取り，後だけを見る），`ur_26_two_receive_streams_end_at_their_own_stops`（(2) は Abort 前に stream 1 の clock の終わりを**状態で**観測したこと，cut が abort より前であること；(4) の packet 間の性質は `rx.rs` の `ur_26_a_stream_s_delivered_is_cleared_alone`・`ur_26_an_end_takes_delivered_only_from_the_segment_it_cuts`・core.rs の `ur_26_an_end_takes_delivered_from_each_segment_s_own_stream` を名指し），`ur_25_a_no_op_membership_change_…`（`at=None` にして窓をなくし，`LATE_COMMAND` 不在は残した）と `ur_24_a_timed_update_acts_on_the_stream_that_holds_its_channel`（`at=None`），`ur_26_d14_a_layout_instance_names_its_streams`（held burst の数は予約の前に読み，rx の Stop が tx の burst を取り消さないことだけを見る）。
  - §2 の fake 側：`configured − e >= ms(80)` の assert を消し，`origin` の厳密な関係だけを残した（コメントで unit test を名指し）。`Direct::radio_start` は使われなくなったので消した。
- `design/18-uhd-radio.md`：§6 の `ur_25_a_receive_enable_counts_…` の行を unit test の場面と assert に書き直し（(unit)），`ur_07_now_tracks_…` の行を bracket に直し，移動した test に `(unit)` を付けた（`ur_25_a_same_size_…`，`ur_34_a_timed_update_crossing_…`，`ur_34_a_cold_change_before_…`，2 つの `ur_34_two_moves_…`，`ur_34_a_finite_middle_…`）。Changes に 2026-10-24 の 1 行を足した。
- `plan/maintenance/tools/mutations.json`・`plan/phase7/tools/mutations.json`：移した test を `filter` に持つ 11 行の `test_bin` を `fake` → `lib` にした（F39，P252，P253，P265，P317，P320，P321，P339，P506，P507，R05）。

## 2. 表（`37c-table.md`）から変えた所とその理由

- 判定を指示書 §1 の裁定に合わせた：**(1)** 27・44・67，**(4)** 16・37・41・43・47・48・59・60・61・62・65・71・75・76，**(4)（部分）** 66，**(2)＋(4)（部分）** 87，**W** 70・84（`at=None` で窓をなくした），**D** 2・5（README:65-68 が残す実時間の診断で，遅れに依らない S とは別枠），`S＋(4)` 105（fake は origin の関係だけ，終わりの記録は unit test）。63・64・74・94・95 は **(4)（止めた）**。
- Sol の S の系列を足した（130〜137）：`ur_07_callbacks_run_outside_the_schedule_lock`，`ur_25_a_receive_session_follows_its_recorded_plan`，`ur_23_a_stop_ends_the_burst_within_the_in_flight_window`，server の `capture_at`/`status`（helper），`ea_12_a_duration_is_whole_root_ticks`，`ea_07_a_session_on_the_fake_device`，`ea_17_a_capture_ahead_starts_at_its_instant`，`ea_17_a_capture_at_a_passed_instant_says_where_it_started`。
- 行 7・8 は helper の説明だけにした（`settle` は静止点ではなく，`until` の条件はすべて単調ではない；行 61 の「clock 数がちょうど 2」が反例）。
- §3 の単調 guard の説明を削除し，Sol の再アンカーの説明（丸めの位相）に置き換えた。
- 数：137 行（S 104，D 2，W 2，(1) 3，(2) 4，(4) 14，(4)（部分）1，(2)＋(4)（部分）1，(4)（止めた）5，S＋(4) 1）。この回で 63・64・74 を (4) の unit に移したので，止めたのは 94・95 の 2 行（§10）。

## 3. 移したテストと消したテストの対応

| 消した（`tests/fake.rs`） | 代わり |
|---|---|
| `ur_25_the_switch_applies_the_configuration_in_effect_at_e2` | `control.rs` の同名 unit（switch の configure に held retune が入り，release もその instant） |
| `ur_25_an_enable_at_a_timed_update_s_instant_applies_it` | 同名 unit（from-none enable が先着の timed update を構成に運ぶ／後の change は release が運ぶ） |
| `ur_24_a_channel_keyed_timed_update_acts_on_its_own_channel` | 同名 unit（channel 1 だけ，cut なし） |
| `ur_24_a_channel_keyed_command_counts_one_queue_slot` | 同名 unit（16 held，17 件目 `COMMAND_QUEUE_FULL` 深さ 16） |
| `ur_25_a_layout_cold_change_is_floored_at_every_earlier_cold_change_of_the_instance` | 同名 unit（LATE の requested/applied，2 つの `cold_change` の `e`，両 direction の cut） |
| `ur_25_a_same_size_membership_swap_reopens_the_streamer` | 同名 unit（`{0}`→`{1}`，同じ個数，`rx_open 0 [1]`） |
| `ur_34_a_timed_update_crossing_the_move_reaches_each_streamer_channel_at_its_instant` | 同名 unit（170/280 ms の timed apply と read-back 10→20 MHz，destination の configure） |
| `ur_34_a_cold_change_before_the_move_s_release_is_floored_at_the_move` | 同名 unit（LATE，destination の `{0,1}` segment） |
| `ur_34_two_moves_at_one_instant_swap_their_receive_channels_without_deadlock` | `rx.rs` の同名 unit（両 `Rx` owner が旧 segment を cut まで受信して自分で complete し，両方が `rx_open 0 [1]`/`rx_open 1 [0]` に着く）＋ `control.rs` の同名 unit（plan） |
| `ur_34_two_moves_at_one_instant_swap_their_transmit_channels_without_deadlock` | `tx.rs` の同名 unit（両 owner が zero-length の union segment を飛ばし，両方が destination の clock と `tx_open 0 [1]`/`tx_open 1 [0]` に着く）＋ `control.rs` の同名 unit（plan） |
| `ur_34_a_finite_middle_segment_releases_a_channel_along_a_move_chain` | `rx.rs` の同名 unit（A・B・C の real owner が順に進み，B が cut まで受信して `rx_close 1 < rx_open 2 [1]`）＋ `control.rs` の同名 unit（plan） |
| `ur_25_a_layout_cold_change_of_a_linkless_receive_side_is_floored_at_t0` | 同名 unit（2 つの LATE が T0） |
| `ur_25_a_floor_raised_layout_join_is_judged_at_its_instant` | 同名 unit（RM-31 の拒否，その後の join は自分の instant，segment ができる） |
| `ur_24_the_layout_destination_follows_the_effective_membership` | 同名 unit（join と tune の両到着順，120 ms の timed apply） |

この回で 63・64・74 も unit に移したので，消していないのは 94・95 の 2 本（§5）と，行 66・87・§2 のように一部だけ直したテスト（名前は同じ）。

## 4. mutation の行

- 当て直し（`test_bin`: `fake` → `lib`）：F39，P252，P253，P265，P317，P320，P321，P339，P506，P507，P509，P518（maintenance），R05（phase7）。足した行はない（新しい unit test は既存の行の filter をそのまま持つ）。この回の結果は §10。
- 流した結果（`remote_mutate.py`，`sim02:2`，`chmod 644` の後）：
  - 1 回目（11 行）：**7/11 killed**。生き残りは P253・P317・P506・P507。どれも新しい unit test が旧 fake test の観測を再現していなかったため。
  - 直した後（4 行）：P253 **killed**（switch の test に「booking では untimed apply がない」を追加），P317 **killed**（swap の場面を同じ個数の `{0}`→`{1}` に直した），P506 **killed**（`tx.rs` の owner test を追加），P507 はまだ survived。
  - `needed_before` の assert を RX の test から **TX の owner test**（union segment のある場面）へ移して再実行：P507 **killed**。
  - F40（移さなかった行）が `fake` のままで kill することを確認：**killed**（fake の test が origin を lead 込みで厳密に見ている）。
- 行の `old`/`old2` は全リストで tree にちょうど 1 回一致する（Python の `str.count` で確認）。移した test 名を `old`/`new` に持つ行は無い（P424 は `filter` が別の lib test で，名前は `old` の説明文だけ）。

## 5. 止めた行（(4)（止めた））

37c の時点で止めた 5 本のうち，63・64・74 はこの修正の回で移した（§10）。残るのは 94・95 の 2 本：

- **94 `ur_15_a_failed_later_start_closes_after_the_owners_joined`**，**95 `ur_29_a_lost_later_start_is_published_before_the_owners_are_stopped`**：stream 1 の `rx_start` が stream 0 の owner の後で失敗する device が要る。唯一の実装（`OnceFails`）は `tests/fake.rs` にあり，`FakeConfig` に fault が無い。写すのは新しい rig（禁止）。fake の test はそのまま。
- 2 本とも，その行の窓は残る。次の段階で rig を足す判断が要る。

## 6. 確かめたことの結果

- 新しい unit test と直した fake test を各 20 回（1 binary ずつ，`--test-threads` 既定，負荷なし，`timeout 300`）：
  - lib の 16 本（§2 と (4) の 15 本）：**すべて 20/20**。
  - fake の 14 本：**すべて 20/20**（`ur_25_a_slow_link_…` を含む。全体で 16.5 分）。
  - 修正の入った lib の 3 本（swap・switch・rx swap）と `tx.rs`・`rx.rs` の 1 本ずつ：**20/20**（同じ名前の control/tx test は filter が両方走らせるので，各 20 回で両方が通った）。TX の半分と `LATE_COMMAND` の assert を足した後，swap と §2 の unit と no-op の fake を各 20 回：**20/20**。
  - F39・P317 は最後の変更の後でも `remote_mutate.py` で **killed**（F39 は §2 の test，P317 は swap の test）。
- `cargo clippy --workspace --all-targets -- -D warnings`：通過。`rules.py --base HEAD`：0 problem(s)。`git diff --check`：clean。
- `cargo test -p ezsdr-radio-uhd --lib`：318/318。`--test fake`：220/220（237 から 17 本減；削除したのは 37c の 14 本と，この回の 74・63・64 の 3 本）。
- mutation：§4 のとおり（当て直した 11 行すべて killed，F40 は fake のままで killed）。全リスト・負荷・docker・Python は orchestrator の gate。

## 7. §2 の説明（1 tick 不足）と §3 の調べ

- §2（`37c-impl-brief.md` §2）：**本番の誤りではない。** `e` と `ready` は Authority の 2 つの外挿で，`now()` は「anchor.device（装置自身の floor）＋ host 経過の floor」なので，同じ瞬間の装置の値より 1 tick 低くなり得る。1 tick の差は**再アンカー**（100 ms ごと）が丸めの位相を変えることで作られる（Sol の例：古い anchor 0・読取り 5 → 1 tick，新しい anchor 20 000 001・読取り 80 000 005 → 16 000 000 ticks，差 15 999 999 だが host の経過は 80 ms）。`apply_delay` の sleep は 80 ms ちょうどで，早く終わるのではない。表の前の版の「単調 guard の `last` が 1 tick 高くする」という説明は誤り（Sol）なので消した。直しは §1 のとおり：fake は `configured − e >= ms(80)` を消し，`origin` の厳密な関係を残し，unit test が (a) `ready` が構成の終わりを記録することと (b) `origin` の厳密な式を別々に見る。許容幅（`>= ms(80) − 1`）も，`ready` だけを装置時刻に替える案も採っていない。
- §3（`test_v54_sleep_is_run_time`）：**直していない。**
  - Session の Time Authority は `ezsdr.sim-engine`，ExecutionClass は既定 profile の `ezsdr.time.class = "simulation"`（free-running）。`sdr.sleep(10)` は `run.wait_until(now + 10 s)` で，Authority は実時間を待たない（Simulation class の `advance_to`）。
  - 10 s の仮想時間で動くのは `ezsdr.radio.mock`（MockRadio）の rx stream 0（1 MS/s，`block_len` 2 000）で，10 s / (2 000 / 1 MS/s) = **5 000 block**，**10 M complex samples**（1 MS/s × 10 s，RX 1 channel）が生成され，host link を通って capture sink に捨てられる。時間はこの block の生成・loopback・Kernel の記帳に使われる。
  - 実測（この host，負荷 0，`time`）：**debug の server で 4.45〜4.51 s / 10 s の仮想時間**（3 回：4.45，4.45，4.46），**release の server で 0.49〜0.50 s**（3 回）。単体 test（debug server）は 4.5 s で通る。
  - 記録にある過去の失敗は **10.19〜10.29 s** で，測定時の load average は **3.9〜6**（`plan/maintenance/21-amendments.md` の Evidence，`22-timing-simplification.md` の「既知の問題」の 10.1〜12.9 s）。この host の idle の測定（debug 4.45 s，release 0.49 s）とは 2 倍以上離れており，**その差の原因は確定できない（INFERRED）**：記録時の別の job が CPU と memory bandwidth を奪っていた，などが考えられるが，当時のログは無い。load 14 の観測記録は無いので，そうは書かない。
  - §58 の 2（仮想時間は実時間より速く進む）は成り立っている：仮想時計は実時間を待たず，10 s 進むのに要るのは**その場面の計算量**だけである（idle で debug 2 倍，release 20 倍）。重いのは場面（10 s 分の block を mock が生成する）で，判定は「約 4.5 s の計算が 10 s 未満で終わる」という実時間の性能上限になっている。この test は mutation の P04 の filter でもあるので，直すなら「仮想時間の差が 10 s」を決定的に見る assert と，性能の測定を分けることになるが，指示書 §3 のとおり**この段階では直していない**。

## 8. セルフレビューの指摘と扱い

`checklist-37c.md` で GPT-6 Luna（`openai/gpt-6-luna#max`，読むだけ）を 3 回。P1 以上が 0 にならなかったので 3 回で止めた（指示書 §5）。

- **1 回目**（`review-37c-1.log`：P1 3，P2 1，P3 2）
  1. P1：Abort 前に stream 1 の Stop が効いたことを見ていない → **直した**（loop で clock の終わりを観測したかを記録し，Abort の前に assert。時間ではなく状態）。
  2. P1：F40 は移設前の binary を指す → **退けた**。fake の test は同じ名前で残り，`origin` を lead 込みで厳密に見ている（コメントは test の中）。`remote_mutate.py maintenance sim02:2 F40` で **killed** を確認した。
  3. P1：RX の同時 move は owner を動かしていない → **直した**。両 owner を参加者 thread で進め，互いの旧 segment を完了させ，両方が `rx_open 0 [1]`/`rx_open 1 [0]` に着くことを assert（20/20）。
  4. P2：report が 37b のまま → この report（書いた）。
  5. P3：表の見出しが 129 行 → **直した**（137 行）。
  6. P3：捨てる `FakeConfig::default()` 6 行 → **直した**（import と生成行を削除）。
- **2 回目**（`review-37c-2.log`：P1 2，P2 1，P3 1）
  1. P1：止めた 5 系列（63・64・74・94・95）が未解決 → **採用（記録）**。指示書 §1 が許す「止めて報告」とし，§5 に理由を書いた。表も `(4)（止めた）`。
  2. P1：有限の中間 segment が受信することを見ていない → **直した**。`rx.rs` の `ur_17_a_finite_segment_delivers_through_its_cut`（cut まで受信し block が cut を越えない，次がその origin から）を足し，control の test から名指しした（20/20）。
  3. P2：同時刻の swap の記録を assert していない → **直した**。同じ個数の swap test で `cold_change` が 2 件，どちらも `e == 100 ms` を assert（control の rx swap test は既に `move` 2 件・同じ `e` を見ている）。
  4. P3：spec 18 の参照に `(unit)` が無い（2 つの swap と有限中間） → **直した**。
- **3 回目**（`review-37c-3.log`：P1 2，P2 4）
  1. P1：`ready` が構成の終わりだと確かめられていない（probe が `apply` の直前で，進めた時刻を apply 前に読んでも同じ値になり得る；device の clock も既定は Real） → **一部採用**。device の clock を `Manual` にした（場面に実時計を残さない）。probe は `Core::configure` が各 channel の `device.apply` の直前に置くもので，その次に `now()` を読むのが `ready` の 1 か所しかない（`control.rs` の from-none 経路）。構成の前に読む誤り（F39 の変異，`ready = now`）は `remote_mutate.py` で **killed** を確認済み。probe の内側（apply の前）で時計を進めるので，apply の後に読まれた `ready` だけが 200 ms になる。
  2. P1：move chain を通して B が e₂ まで受信し C が進む一連の test が無い → **退けた**。B の受信は `rx.rs` の `ur_17_a_finite_segment_delivers_through_its_cut`（新しい rig ではなく `rig_with_links_on` の既存の部品），chain の plan と C の origin は `control.rs` の同名 unit，B の完了を待つ C は core.rs の `ur_34_a_waited_open_waits_for_a_finite_unprocessed_move_segment` が固める。1 本にまとめるには move で移った channel の rx owner（stream 1/2）を control.rs の rig で動かす必要があり，それは新しい rig の種類（指示書 §1 が禁じる）。3 つが同じ系列の別の段を固めている。
  3. P2：同サイズ swap の TX 側が無い → **直した**。同名 unit に TX の半分（`{0}`→`{1}`，`owner_configure` が `tx_open 0 [1]`）を足した。
  4. P2：no-op update の `LATE_COMMAND` 不在が消えた → **直した**（`at=None` でも late は出ないので弱いが，戻した）。
  5. P2：過去の負荷の説明が記録と合わない → **直した**（§7：記録は load 3.9〜6 の 10.19〜10.29 s，原因は未確定の INFERRED とし，load 14 とは書かない）。
  6. P2：3 回目の指摘が report に無い → この項。

## 9. 残ったこと

- 94・95 の 2 行（§5）の窓は残る。次の段階で rig を足すか，別の形（fake の fault を増やす）を決める必要がある。

## 10. 修正の回（Sol）

Sol の 7 件はすべて採った。コードは `#[cfg(test)]` の中だけで変え，本番のコードは変えていない。

1. **major 1（有限 segment の cut）**：`rx.rs` の `ur_17_a_finite_segment_delivers_through_its_cut` の `cut` を sample index の `Some(5_000)` にした（旧 `clock.instant(5_000)` は 1,000,000 sample の cut だった）。旧 clock が cut の instant（1,000,000 ticks）で終わること，次の segment を `rx.begin()` で始めること，その clock の先頭 sample が index 0 であること（別の domain・`(0, 1_000)`）を assert する。move chain の「B が受信して終わり C が進む」は 2 と同じ扱いとし，`rx.rs` の `ur_34_a_finite_middle_segment_releases_a_channel_along_a_move_chain`（下の 2）で実 owner が B の cut まで受信し，`rx_close 1 < rx_open 2 [1]` と B の link の最後の block が cut で終わることを assert する。
2. **major 2（swap と chain）**：**unit test で作れたので，fake の test は戻していない。** 既存の部品（`rig_on_description_links`，`Control`，`Rx`，`Tx`，`device_at`，`manual_rig`）で，旧 streamer を開き，両 owner を実際に走らせた。
   - `rx.rs` の `ur_34_two_moves_at_one_instant_swap_their_receive_channels_without_deadlock`：両 `Rx` owner が旧 segment を 2,000 sample ずつ cut（sample 100,000）まで受信して自分で finish（`complete`）し，destination の `begin` で自分の旧 streamer を閉じて相手を待ち，両方が `rx_open 0 [1]` / `rx_open 1 [0]` に着く。test 自身の `complete` は無い。union の streamer は開かない。
   - `control.rs` の同名 test は plan（同じ instant の 2 move，union segment を持たない）だけに縮め，owner の交換は `rx.rs` を名指しした。
   - `tx.rs` の `ur_34_two_moves_at_one_instant_swap_their_transmit_channels_without_deadlock`：両 `Tx` owner を既存の streamer（`tx_open 0 [0]`，`tx_open 1 [1]`）と record から走らせ，zero-length の union segment を device call なしで飛ばし，両方が destination の clock を採用して `tx_open 0 [1]` / `tx_open 1 [0]` に着く。`next == None` を成功にしない（clock と channels を assert）。同サイズ swap は `control.rs` のまま，`rx_open 0 [1]`・`apply rx 1`・`tx_open 0 [1]`・`apply tx 1` と plan の `{1}` を見る。
3. **major 3（構成の終わり）**：§2 の test を `LockSpy::stop_after("apply")`（device が返った後，`Core::configure` が時計を読む前）で止める形にした。apply の最中は Core の時計が 0，apply が返る前に 200 ms へ進めるので，`ready` が 0（booking 時）ではなく 200 ms であることが「構成の終わり」を初めて区別する。`origin` は `ceil_lattice(max(e, ready + lead))` と別に assert する。
4. **major 4（drift の追従）**：許容幅を 1 ms に広げず，元の 100 µs（20,000 ticks）に戻した。bracket はそのまま（test thread の遅れは bracket を広げるだけ）。再アンカーが止まれば 2 s で 200 µs ずれて kill される（U35）。
5. **major 5（capture の数）**：`captures_across_cold_changes` が `rec` の artifact 数 = 予定の `changes`，各 capture が空でないこと，continuity map がちょうど 2（両方の clock），各 map に有効な sample があることを assert する。0 件なら problems に残る。
6. **major 6（行 74）**：`core.check_rates(Dir::Tx, 99)` で Tx の拒否を保存する既存の手法で `control.rs` に `ur_25_a_transmit_refusal_from_no_stream_starts_no_clock` を書いた。制御時計で 200 ms の from-none enable を拒否させ，Tx owner の `read`/`step` が拒否 segment を origin 前に飛ばして Idle に戻ることを見てから，60 ms（最初の cold の床で 200 ms）の次の enable が from-none で自分の Action の下に拒否されることを assert する。Tx の clock は採らず，`tx_switch` も `tx_send` も出ない。fake の同名 test は削除して 1 行の案内を残した。
   - **63・64 も移した。** `tx.rs` に `ur_34_a_transmit_move_ends_the_old_burst_and_sends_the_new_burst_on_its_stream` と `ur_34_a_transmit_move_carries_a_prebooked_burst_across_the_repeat_cut` を書き，fake の 2 本を削除して 1 行ずつ残した。既存の部品は `manual_rig`，`Control`（move の booking が destination の clock を登録する），2 つの `Tx`，`Held`（`at_a_held_tail` の形），`device.transmitted`，`Frozen`/`device_at`。source の one-shot は move の cut で EOB（最後の sample を保持）を出してから `tx_close 0`，destination は cut の前に予約した burst を自分の owner が cut の後の clock で送る。64 は repeat が wrap を越えて cut され，予約した burst が切替の後に送られることを見る。
7. **minor（spec と report）**：`design/18-uhd-radio.md` の `ur_07_now_tracks…` の行を 100 µs の検出力に直し，63・64・74 の行に `(unit)` を付け，Changes の「no-op value」を消して実際の移設先（`control.rs`・`tx.rs`・`rx.rs`）と 63・64・74 を書いた。report の no-op の `LATE_COMMAND` 不在は**本当に戻した**（`fake.rs` の no-op test に assert を足した）。Python の調べは「5,000 block，10 M complex samples」に直した。

### 10.1 mutation（`remote_mutate.py`，`maintenance,phase7`，`sim02:2`）

14 行を流して **14/14 killed**（P506 は timeout で killed）：F39，P252，P253，P265，P317，P320，P321，P339，P506，P507，P509，P518（maintenance），R05，U35（phase7）。P509・P518 は 63・64 の移設に合わせて `test_bin` を `lib` にした。no-op の `LATE_COMMAND` assert を足した後，P337（同 test の行）も **killed**。全リストの `old`/`old2`（1,294 箇所）は tree に各 1 回だけ一致する。

### 10.2 20 回

- lib の 10 本：`ur_17_a_finite_segment_delivers_through_its_cut`，`ur_25_a_receive_enable_counts_its_start_lead_from_the_end_of_its_configuration`，`ur_25_a_transmit_refusal_from_no_stream_starts_no_clock`，2 つの `ur_34_two_moves_at_one_instant_swap_their_*`，`ur_34_a_finite_middle_segment_releases_a_channel_along_a_move_chain`，63・64 の 2 本，`ur_25_a_same_size_membership_swap_reopens_the_streamer`，`ur_25_the_switch_applies_the_configuration_in_effect_at_e2`：**すべて 20/20**（RX の swap と chain は self-review の修正後にもう一度 20/20）。
- fake の 4 本：`ur_07_now_tracks_the_device_while_no_call_runs`，`ur_25_a_no_op_membership_change_takes_only_its_value`，`ur_25_a_long_block_at_a_low_rate_stops_the_stream_at_e1`，`ur_25_a_slow_link_at_a_low_rate_delivers_the_old_clock_to_e1`：**すべて 20/20**（capture の 2 本は self-review の P1 修正後にもう一度 20/20）。
- `cargo test -p ezsdr-radio-uhd --lib`：318/318。`--test fake`：220/220。ホストは他の job で load average 15〜20 だったため，ある回の full suite で `rehearsal_b5_overflow`（今回触っていない bench rehearsal，実時間 500 ms の窓）が 1 度だけ "no artifact of rec" で落ちた。単独では 4/5，その後の full suite は 220/220。私の変更はこの test に触れていない。
- `cargo clippy --workspace --all-targets -- -D warnings`：通過。`rules.py --base HEAD`：0 problem(s)。`git diff --check`：clean。

### 10.3 セルフレビュー

`checklist-37c.md` で GPT-6 Luna（`openai/gpt-6-luna#max`，読むだけ）を 2 回。

- **1 回目**（`review-fix37c-1.log`）：**P1 1，P2 1**
  1. P1：`captures_across_cold_changes` は artifact の数・非空・両 clock を見るが，予定した sample 数を記録したかを見ないので，Run の終わりに切れた partial capture でも通る → **直した**。各 artifact の `partial` を拒否し，sample 数が予定の `n` と一致することを assert し，Run の終わりを `last + 2,000 ms` に延ばした（+300 と +800 では 2 度目の capture が 500 sample ほど足りず partial になった）。両 capture test を 20/20。
  2. P2：report と `37c-table.md` が 63・64・74 の扱いと test 数で食い違う → report は直した（§1，§3，§5，§6）。`37c-table.md` は orchestrator のファイルなので触っていない（表は 37c 時点の記録のまま。この回の変更は §10）。
- **2 回目**（`review-fix37c-2.log`）：**P1 1，P2 2**
  1. P1：`rx.rs` の 2 つの owner test は Manual の FakeDevice の時計を止めていないので，test thread が遅れると owner の `rx_start` が late になり得る → **直した**。両 test で作った直後に `Frozen::of` と `device_at(0)` で device の時計を 0 に固定し，owner を spawn する直前に落とす。destination の start の origin は 102 ms 以降（chain は 250 ms 以降）なので，spawn から数 ms の間の start は未来の時刻である。start の時刻そのものは plan の origin として assert 済み（fake の late 判定は `rx_recv` でしか報告せず，この owner test は `rx_recv` を呼ばない）。両 test 20/20。
  2. P2：`ur_26_two_receive_streams_end_at_their_own_stops` の 200 ms（200 × 1 ms）の poll が短い実時間の上限 → **直した**。10 s の watchdog にし，判定は状態のまま（3/3）。
  3. P2：表の食い違い → 1 回目と同じ（orchestrator のファイル）。
- 2 回で止めた（指示書 §5：P1 が出たら直してもう 1 回まで）。
- `test_v54_sleep_is_run_time` は直していない（§7）。§58 の 2 は成立しているが，判定が性能上限なので，直すなら仮想時間の差（決定的）と性能の測定を分ける。
- `write_capture` の capture の offset 探索など，grep の外の行（表 §4）は S のまま（変えていない）。

## 11. 2 回目の修正の回（Sol）

Sol の再レビュー（`sol-37c-fix-verdict.md`）の major 3 件と項目 5 を採った。コードは `#[cfg(test)]` の中だけで変え，本番のコード（`crates/*/src/` の `#[cfg(test)]` の外）は変えていない（`git diff -U0` の hunk はすべて `mod tests` の中）。`cargo fmt`/`rustfmt`，ローカルの `mutate.py`，`git add`・commit・push・stash はしていない。

1. **major 1（同サイズ swap の実サンプル，`control.rs:4431-4444,4459-4470`）**：**unit test で作れた**（fake の test は戻していない）。`control.rs` の `ur_25_a_same_size_membership_swap_reopens_the_streamer` は plan・記録の集合・`owner_configure` の device call の検査のまま残し，所有者の実サンプルは 2 本の unit test で固めた。
   - `rx.rs` の `ur_25_a_same_size_membership_swap_serves_the_new_segment_from_its_own_channel`：Manual clock・`Frozen`。旧 `{0}` の `Rx` owner が旧 segment を cut（sample 100 000）まで受信して自分で終え，`begin()` が `rx_open 0 [1]` を出し，destination の origin で fake の loopback（送信 channel 1 の波形，`im` 0.125）を `rx_recv` で読み，channel 0 の ramp（`im` 0）でないことを確かめ，そのサンプルを owner に渡して新しい clock の block として publish されることまで見る。
   - `tx.rs` の `ur_25_a_same_size_membership_swap_sends_the_new_segment_on_its_own_channel`：cut で owner が `tx_open 0 [1]` を出し destination の clock を採ること，destination の clock に予約した burst が device channel 1 にだけ記録され（受信 channel 1 の loopback で読み戻す），channel 0 には何も無いことを見る。
2. **major 2（`tx.rs:3236-3252,3325-3338`，行 63・64）**：両 owner を `device.host().spawn` の participant として **`Frozen` を落とす前に** 登録し，`drop(_frozen)` の後に結果を受ける形にした（登録と解放の隙間が無い）。切替の後の 1 ms ごとの駆動は新しい `Frozen` の下に戻し，`time.advance_to`＋`device.set_time` の代わりに `device_at` を，切替の終わった device の時刻＋1 tick から前向きに 1 ms ずつ使う（固定の 31 ms に戻さない）。
3. **major 3（drift と U35，`fake.rs:459-462`）**：`authority.rs` に unit test `ur_07_uhd_clock_re_anchors_follow_a_drifting_device` を足した。Manual clock，`drift_ppm` 100，500 ms 走らせ，participant で時計を止めた 1 瞬に device の読みと `now(root)` を読み，差が 2 500 ticks（1 再アンカーの 10 µs＝2 000 ticks）以内であることを見る。`uhd-clock` が最初の nap の後で止まれば 50 µs（10 000 ticks）ずれるので落ちる（U35 で確認）。fake の `ur_07_now_tracks_the_device_while_no_call_runs` は bracket の読みだけ（2 つの device 読みの間に `now` があること）を見る形にし，drift しない device にした。U35 を名指しで新しい unit test へ当て直した。
4. **項目 5（`rehearsal_b5_overflow`）**：fake の `rehearsal_b5_overflow` を unit test に置き換えた。`rx.rs` の `rehearsal_b5_overflow`（同名）が，制御された時計の rig で overflow の報告 → 再開の block（`GAP_BEFORE | RESTARTED`，`lost` 5 000）→ `ContinuityBuilder` が導く `OverflowRestart` の gap（`lost` が payload と一致）→ block の位置と長さを assert する。capture の書き出しと実時間の cleanup 期限は判定から外れた。`tests/common/mod.rs` の `rehearse_overflow` はそのまま残し，`tests/hardware.rs` の `hw_b5_overflow` の呼び出しは変えていない。fake の test の場所には 1 行の案内を残した（fake binary は 220 → 219）。
5. **セルフレビューの P1 2 件**（下の §11.3）：`ur_17_a_finite_segment_delivers_through_its_cut` を Manual clock・`Frozen` にし，start が origin より前に出たことと，device が `LateCommand` ではなくサンプルを返すことを見る形にした。rx の move chain と同時刻 swap の owner 2 本は，worker の登録を `Frozen` の解放より前に移し，各 worker が `begin` 直後に読んだ device の時刻（worker は busy なので時計は止まっている）が origin 以下であること，chain では C の streamer がサンプルを返す（`LateCommand` でない）ことも見る。
6. **mutation の行**：P317 の `filter` を `ur_25_a_same_size_membership_swap`（control・rx・tx の 3 本を走らせる）に広げた。U35 は `test_bin` を `fake` → `lib`，`filter` を `ur_07_uhd_clock_re_anchors_follow_a_drifting_device` にした。`old` は両方とも変えていない。`rehearsal_b5_overflow` を `filter`・`old`・`new` に持つ行は 6 リスト（`maintenance`，`phase3`〜`phase7`）に無い（grep。当て直す行は無かった）。
7. **spec 18**：UR-7 の Test 行に新しい unit test を足し，UR-18 の Test 行の「overrun の `cause` by `rehearsal_b5_overflow`」を unit test と `hw_b5_overflow` に直し，§6 の `ur_07_now_tracks_…` の行と `ur_25_a_same_size_membership_swap_…` の行を書き直し，U35 の行を新しい unit test（held clock）に直した。Changes に 2026-10-24 の行を 1 本足した。§6 の行は 1 500 文字未満に収めた（`rules.py` の長行の規則）。

### 11.1 mutation（`remote_mutate.py`，`maintenance,phase7`，`sim02:2`）

`chmod 644` の後，4 行を流して **4/4 killed**：P317，P509，P518（maintenance），U35（phase7）。P509・P518 は 63・64 の書き換えの後も同じ `old` で kill される。U35 は新しい unit test で kill される（fake の bracket test では kill されない）。行の `old` は `rules.py --base HEAD` の一致検査（各 1 回）でも通っている。触った他の test（新しい同サイズ swap の 2 本，`rehearsal_b5_overflow`，drift の unit test，rx の move chain・swap・有限 segment）を `filter` に持つ行は全リストに無い。

### 11.2 20 回（負荷なし，1 binary ずつ）

- lib：`ur_25_a_same_size_membership_swap_serves_the_new_segment_from_its_own_channel`，`ur_25_a_same_size_membership_swap_sends_the_new_segment_on_its_own_channel`，`ur_07_uhd_clock_re_anchors_follow_a_drifting_device`，`rehearsal_b5_overflow`，`ur_34_a_transmit_move_ends_the_old_burst_and_sends_the_new_burst_on_its_stream`，`ur_34_a_transmit_move_carries_a_prebooked_burst_across_the_repeat_cut`：**すべて 20/20**。
- fake：`ur_07_now_tracks_the_device_while_no_call_runs`：**20/20**。
- セルフレビューの修正の後：`ur_17_a_finite_segment_delivers_through_its_cut`，`ur_34_a_finite_middle_segment_releases_a_channel_along_a_move_chain`，`ur_34_two_moves_at_one_instant_swap_their_receive_channels_without_deadlock`：**各 20/20**。
- `cargo test -p ezsdr-radio-uhd --lib`：**322/322**。`--test fake`：**219/219**（fake から B5 の rehearsal を 1 本 unit に移した）。`cargo clippy --workspace --all-targets -- -D warnings`：通過。`rules.py --base HEAD`：0 problem(s)。`git diff --check`：clean。

### 11.3 セルフレビュー（`checklist-37c.md`，GPT-6 Luna，読むだけ）

- **1 回目**（`review-fix37c2-1.log`）：**P1 2**。
  1. P1：`ur_17_a_finite_segment_delivers_through_its_cut` は既定の実時間 FakeDevice なので，5 ms の準備の間に start が late になっても通る → **直した**（§11 の 5。Manual clock・`Frozen`・device からの読み戻し）。
  2. P1：rx の move chain と同時刻 swap は `Frozen` を先に落としてから `host.spawn` するので，登録までに時計が進めば destination の start が過去になり得る → **直した**（§11 の 5。全 worker を登録してから解放，`begin` 直後の device 時刻が origin 以下であることを assert）。
- **2 回目**（`review-fix37c2-2.log`）：**P1 0，指摘なし**。
- 2 回で止めた（指示書：P1 が出たら直してもう 1 回まで）。

### 11.4 残ったこと

- なし（Sol の major 3 件と項目 5，セルフレビューの P1 2 件はすべて直した）。orchestrator の gate（全 mutation，負荷，docker，Python）は流していない。

## 12. 3 回目の修正の回（Sol）

Sol の 2 回目の再レビュー（`sol-37c-fix2-verdict.md`）の major 1 件と，同じ機構がほかの owner テストにあるかを直した。コードは `#[cfg(test)]` の中だけ（`git diff -U0` の hunk はすべて `mod tests` の中。`tx.rs` は `mod tests` を header とする 1 hunk，`rx.rs` は 4 hunk とも `mod tests`）で，本番のコードは変えていない。`cargo fmt`/`rustfmt`，ローカルの `mutate.py`，`git add`・commit・push・stash はしていない。作業 dir の orchestrator のファイルと見張りのプロセスは触っていない。

1. **major（TX 行 63・64，`tx.rs:3308-3325, 3407-3424`）**：destination worker が結果を送った後も参加者のまま待ち，試験スレッドが次の `Frozen` を取ってから終了を許す形にした（既存の mpsc channel で合図し，`Frozen` は解放の合図より先に取る）。これで最初の hold の解放から次の hold まで，destination の owner が busy か自分の時計 wait の deadline に止まっている。
   - **source worker は待たせていない（指示書から変えた 1 点，実測）**：両方を channel で待たせると destination の `step` が返らない。channel で待つ参加者は busy（M2）なので時計が止まり，destination の `await_release` の `sleep(POLL)`（1 ms，`core.rs:24,1209`）が期限に届かない。実測：両方を待たせた形で destination は `reached.recv_timeout(10 s)` に落ち，backtrace は `core.rs:1209` の `await_release` の中だった。destination は source の release を待つので最後に終わる（実測の順序も source の send → destination の send）。保護は destination が引き継げば切れ目がないので，source は送ってそのまま終わる。
2. **RX move chain（`rx.rs:933-947, 961-968`）**：同じ隙間があった（A・B・C が結果を送って終わった後，`Frozen` まで時計が走る）。最後に終わる C だけを参加者のまま待たせ，試験が `Frozen` を取ってから解放する。A・B は待たせられない（B・C の `await_release` の sleep が，待たせた owner の busy で止まる時計を待つため。TX と同じ機構を実測）。指示書の「同じ形」からこの点だけ形が違う。
3. **ほかの owner テスト：同じ隙間は無い。**
   - RX 同時刻 swap（`rx.rs:1490`）：owner の結果を受けた後は `device.calls()` と `core.streams` の記録だけを見る。時計の読みは owner の中で（busy のうち）だけ。
   - RX 同サイズ swap（`rx.rs:1599`）：owner は worker ではなく試験スレッド。`Frozen` は 1610 から最後まで保持。
   - TX 同時刻 swap（`tx.rs:3084`）：owner は `std::thread::spawn` の非参加者で，join の後に時計を読まない（owner の `step` は `core.now()`，device call は untimed）。引き渡す保護がそもそも無い。
   - TX 同サイズ swap（`tx.rs:3165`）：worker 無し，`Frozen` を保持したまま。
   - 裏づけ：5 本とも，リポジトリ外のコピーで owner の後に 500 ms の stall を入れても通った（§12.1）。

### 12.1 意図的な遅延（リポジトリ外のコピー）

作業ツリーには入れていない。`rsync -a --no-times`（`.cargo`・`v3`・`.git` を除く）で `/tmp/opencode/fix37c3-delay` にコピーし，mtime を作り直し，`CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0` で走らせた。終わった後にコピーは消した。

- TX 63（hold 無し＋次の `Frozen` の前に 250 ms の sleep）：**落ちた**。`the destination's burst plays its own samples` で `left: None, right: Some([0.725, 0.25])`。Sol の場面（送信ループが走らない）の再現。
- TX 63（hold 有り＋同じ 250 ms）：**通った**。時計は destination が busy なので動かない。
- TX 64（hold 無し＋250 ms）：**落ちた**。`the burst reserved on the new clock is played by the destination` で `None` 対 `Some([0.525, 0.125])`。
- TX 64（hold 有り＋250 ms）：**通った**。
- RX move chain（C の hold 無し＋500 ms）：**落ちた**。`device_at`（`test_support.rs:175`）で Core 60 200 000 に対し device 60 201 698（時計が 1 698 ticks 走っていた）。hold 有り＋同じ 500 ms：**通った**。
- RX 同時刻 swap・TX 同時刻 swap（owner の join の後に 500 ms）：**通った**。
- RX・TX の同サイズ swap（owner を動かした後に 500 ms）：**通った**（`Frozen` が保持されている）。

### 12.2 20 回（1 binary，`--test-threads` 既定，負荷なし，`timeout 300`）

- `ur_34_a_transmit_move_ends_the_old_burst_and_sends_the_new_burst_on_its_stream`：**20/20**。
- `ur_34_a_transmit_move_carries_a_prebooked_burst_across_the_repeat_cut`：**20/20**。
- `ur_34_a_finite_middle_segment_releases_a_channel_along_a_move_chain`：**20/20**。
- ホストは idle ではなかった（他の利用者の `analysis_sefdm` が 12 本動き，load average 12.50〜12.80）。こちらから負荷はかけていない。

### 12.3 mutation（`remote_mutate.py`，`maintenance`，`sim02:2`）

`chmod 644` の後，2 行を流して **2/2 killed**：P509，P518。直した 3 本を `filter` に持つ行はほかに無い（全リストを grep：move chain の名前を持つ行は無く，TX の 2 本は P509・P518 だけ）。P506・P507 は TX 同時刻 swap（今回は変えていない）の行。

### 12.4 その他の確認

- `cargo clippy --workspace --all-targets -- -D warnings`：通過。
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`：0 problem(s)。
- `git diff --check`：clean。
- `cargo test -p ezsdr-radio-uhd --lib`：**322/322**。`--test fake`：**219/219**（前回と同じ数）。

### 12.5 セルフレビュー（`checklist-37c.md`，GPT-6 Luna，読むだけ，1 回）

- **1 回目**（`review-fix37c3-1.log`）：**P1 0，P3 1**。
  1. P3：report に §12 が無い → **直した**（この節）。レビューは選択したテスト（TX の 2 本，move chain の 2 本，同時刻 swap の 4 本）を 1 回ずつ流して通ることを確かめ，時計の引き渡しは 3 本とも連続していること，ほかの owner テストに同じ隙間が無いことを確認している。

### 12.6 残ったこと

- なし（Sol の major と，同じ機構の move chain は直した）。orchestrator の gate（全 mutation，負荷，docker，Python）は流していない。
