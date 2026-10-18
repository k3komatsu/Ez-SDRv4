# 1b-4b のテスト計画（DeepSeek V4.1 Flash が項目ごとに 12 本で書き，orchestrator がまとめた）

指示書：`brief.md`．各項目の本文は下の「項目」に，書かれたまま載せた（冒頭の作業メモの行だけ除いた）．

## orchestrator の裁定（実装の前に決めること）

### 構造

- `Prepared` の受信・送信と `Running` の sender は，stream ごとの列にする（今は 1 本ずつ）．thread は列の各 stream に 1 本．名前は `uhd-rx`・`uhd-tx` のまま（k = 0）．
- 受信の link がない方向の line がない形，thread は stream がなくても起きる形（UR-15）は変えない．
- **prune**：UHD の prune は今，その方向の line にだけ効く（`core.rs:139-145`）．`Lines::prune` は全 line を畳む（`timeline.rs:495-519`）ので，そのまま使わない．stream ごとの prune が要るなら，`Lines` に足すだけにする．既存の `Lines` のコードと mutation の行は変えない（final (b) の注意）．
- `seq`・`ended`・`stop`・`version` は instance 全体のまま．`delivered`・`configs`・`idle`・`tx_clocks` は stream ごとに移す．
- `Streams::end` は，存在する全 line に End を 1 回ずつ記帳する．今の「各方向 1 本」を全 line に広げる．
- `control.rs:194` の `rx_packet_samples(0)` は，`Control::new` が stream ごとに line を作る形にするなら `k` にする（final (a)）．

### 足すテスト（既存のテストは変えない．足すだけ）

| 項目 | 採る | 送る・採らない |
|---|---|---|
| 1 | link のない受信方向の enable が値の記録だけになる（unit）＋行 | — |
| 2 | T1 全 line に End 1 回（unit），T2 2 回目の stop は何もしない（integration），T3 join の順序（integration，約 2 s）＋各行 | T4（任意）は採らない |
| 3 | 両方向の Spec Run での panic が送信 stream も終わらせる＋`DEVICE_LOST` の message（新しいテストとして）＋行 | `TxPanic`・control の panic の fault は 1b-4c |
| 4 | `ur_29_a_loss_ends_every_stream_once`（unit）＋N1・N2 | e2e は採らない |
| 5 | T1（control），T2（rx），T2b（tx）＋M1〜M4 | — |
| 6 | T1 0 への disable と再 enable（integration），T2 enable の configure は 1 回＋R1〜R5 | — |
| 7 | 行 B・C・D | — |
| 8 | 登録・添字・cut での終わり・`version`・seed の行 | (c-7) の rate 変更後の比は 1b-4c |
| 9 | T1，T1(c)，T2，T3（方向をまたぐ device queue．UR-24）＋行 | — |
| 10 | lock の spy の unit 1〜3＋行 A〜D | 4（任意）と E は採らない |
| 11 | 行 a・b・c・d・f | e・g（U42 が裏），control の panic と reason の文面は 1b-4c |
| final | (a) の置き換え，(b) の当て直し，(c) を 1b-4c の指示書へ | — |

mutation の行の `old` は，実装の後の文で書く．行が今の文で書けるものも，実装で文が動けば当て直す．

### Sol のテスト計画レビュー（`sol-plan.md`）の裁定：全件採用

計画の本文より，ここが優先する．

- **major 1（項目 3）**：panic の integration で「送信の SampleClock の `ended_at`／plan の `end` が Some」を求めない．
  - 今の送信の `End` は走っている segment を切らない（`timeline.rs:820-830,867-873`，`core.rs:181-185`）．期待に合わせて送信の意味を変えてはいけない．
  - 送信 line に loss の End が入ることは，項目 4 の unit で観測する．
  - panic の integration は，message・termination・`mark_lost` を見る．
- **major 2（項目 7・9 の行とテスト）**：
  - 項目 9 T1 の行（`..=(origin, u64::MAX)` → `..(origin, u64::MAX)`）は採らない．origin と同じ instant は，純 `ColdConfig` の unit で検査する．mutation は，本当に等号の instant を落とす形（例：`..(origin, 0)`）にする．
  - 床の検査には，cold＝300 ms／timed＝100 ms の対照を足す．
  - 項目 7 の行 D（shutdown の `expected + 1`）は，その枝に実際に届く専用の sequence のテストに結び直す．届かせられなければ行を採らない．
- **major 3（構造の prune）**：stream ごとの prune を守る unit を足す．
  - 2 本の line（受信と送信）を使う．A の prune の後も，B の item と configuration が残り，B の予約を拒否できることを確かめる．A の不要な configuration が回収されることも assert する．
  - 新しい API が channel の移動に対応しないなら，その制約を doc comment に書く．1b-4d で見直す．
- **major 4（項目 10）**：owner の lock の spy は `apply` も見る．
  - 各テストに，通す sequence と，spy が実際に呼ばれたことの確認を入れる．spy が一度も呼ばれずに緑になる形を防ぐ．
  - 区別する経路：同じ channel の rate の switch，channel 数の変更，cut の stop，送信の tail と EOB，report の読み，control の受信・送信の enable，timed の release．
- **minor 1（項目 2 T3）**：wrapper から受信と送信の入口に届いたことを通知し，両方を確かめてから finish する．30 s は watchdog にとどめる．thread は，検査の後に解放できる gate で止める（永久に止めない）．
- **minor 2**：`Direct::build` は manual clock（`tests/fake.rs:620`，`tests/common/mod.rs:298-303`）．計画の「real clock」の記述は誤り．既存テストの lock の検出力は，この前提で見直す．
- **全 line への End**：実装は 4b で行い，複数 stream での検証は 4c で行う．final (c) の「4c で決める」は，この裁定で置き換える．

## 項目

## 1. start：line は thread を起こす前に入る．受信の link がないときは line がない．uhd-rx と uhd-tx は stream がなくても起きる（UR-15）

対象は `crates/ezsdr-radio-uhd/src/provider/mod.rs:421-479`（`Provider::start`：link のガードは `:457`，`rx_start(0, t0)` は `:458-460`，line を置く `Control::new` は `:471`，spawn は `:474-477`），line を置く `crates/ezsdr-radio-uhd/src/provider/control.rs:181-235`（rx の line の link ガードは `:208`，tx の line は `:212-215`，`streams` への代入は `:216-221`），型は `crates/ezsdr-radio-uhd/src/provider/core.rs:57-81` と `crates/ezsdr-radio/src/timeline.rs:409-463`．`start` 自身を叩く unit test は無い（`grep -n "mod tests" crates/ezsdr-radio-uhd/src/provider/mod.rs` の出力なし，exit 1）．

### (a) 既存のテスト

- T1 `crates/ezsdr-radio-uhd/tests/fake.rs:859` `ur_15_start_cases`
- T2 `crates/ezsdr-radio-uhd/tests/fake.rs:873` `ur_15_every_clock_starts_on_its_lattice`
- T3 `crates/ezsdr-radio-uhd/tests/fake.rs:1516` `ur_25_rx_channels_from_zero_starts_at_its_instant`
- T4 `crates/ezsdr-radio-uhd/tests/fake.rs:1535` `ur_25_a_receive_enable_counts_its_start_lead_from_the_end_of_its_configuration`
- T5 `crates/ezsdr-radio-uhd/tests/fake.rs:1500` `ur_25_tx_channels_from_zero_transmits_the_next_burst_on_time`
- T6 `crates/ezsdr-radio-uhd/tests/fake.rs:1576` `ur_14_a_device_error_while_booking_names_its_action`
- T7 `crates/ezsdr-radio-uhd/tests/fake.rs:1941` `ur_29_a_lost_device_is_found_while_idle`
- T8 `crates/ezsdr-radio-uhd/src/provider/control.rs:795` `ur_14_booking_does_not_wait_for_a_future_cold_switch`（lib の unit test）
- T9 `crates/ezsdr-radio-uhd/src/provider/control.rs:822` `ur_21_the_receipt_site_reserves_the_moved_start_before_any_hand_over`（lib の unit test）
- T10 `crates/ezsdr-radio-uhd/tests/fake.rs:2955` `ur_25_a_cold_change_booked_anywhere_in_a_long_receive_call_is_on_time`

### (b) assert と観測

- T1: 前半は start_lead 1 s の profile で Run が `Failed { stage: Stage::Arm }` に終わること，failure 文が `"UR-15: the start at"` で始まること，`radio.LATE_COMMAND` が 1 件（`events_of`，`:866`）— これは本順序の対象外（拒否の形は `mod.rs:446-454`）．後半は capture 1 000 の Spec Run（`captured`，`tests/common/mod.rs:401`）の Manifest `timing` セクションの `first_rx_block` 行の `index == 0`（`:868-869`）．manifest セクションは cleanup が書く（`mod.rs:280`），行は最初の block でだけ出る（`rx.rs:569-577`）ので，「uhd-rx の最初の turn に line がある」を決定的に固定する：line が無ければ uhd-rx は plan なしを cut 0 と読み（`rx.rs:110`），サンプル 0 で終わってこの行は出ない．
- T2: tx と rx を 1 Msps，start lead を奇数（2 000 000 003 ns）にした Spec Run で，Manifest `clocks.sample_clocks` が 2 本，各 origin が 200 の倍数（`:878-881`）．registered clock だけが記録される（`crates/ezsdr-kernel/src/time/domain.rs:400-427`）ので，rx の clock は最初の block でしか登録されない（`rx.rs:487-492`）— rx の line が無いと 1 本になり落ちる．
- T3: `Direct::with_links(..., &[("radio.rx.channels", 0)], vec![attached(..)])`（`:1519`）で start し（`Direct::build` が prepare/arm/start を回す，`:656-661`），T0+1 ms 後に `cold` `rx.channels = 1` を `at` 付きで入れる．device の `rx_start 0 <tick>` が出ること（`.unwrap_or_else(panic)`，`:1524`），origin が `at` 以上 `at+200` 未満（`:1526`），registered rx clock がその origin 1 本（`:1528-1529`），`radio.COMMAND_REJECTED` が空（`:1530`）．観測は device calls，`sample_clock_records`，event．「0 channel の line は空の plan」でなければ uhd-rx が `begin()` で segment を見つけ，`settings: None` の `configure` 失敗で COMMAND_REJECTED が出る（`rx.rs:278-292`）ことを，この空 assert が押さえる．
- T4: apply 80 ms の fake で同じ 0 → 1 の enable．envelope の `rx_start_lead_ns` と，Manifest `timing` の `cold_change`（`e`, `ready`）と `rx_segment`（`origin`）で，`ready − e ≥ 80 ms` と `origin == ceil200(max(e, ready + lead))`（`:1551-1554`）．`ready` は `from_none` の枝でしか設定されない（`control.rs:550-568`）ので，「stream のない方向は uhd-control が configure する」を，start が置く `idle[Rx]`（`control.rs:220`）と空 plan（`control.rs:548`）ごと観測する．
- T5: `tx_session`（`:1024-1032`）は T0 後に `tx.channels = 1` にする（start の時点で tx の stream は無い）．device calls で `apply tx 0 rate=1000000…` が `tx_send … sob=true` より先にあること，TIME_ERROR が空（`:1507-1511`）．apply は uhd-control の `from_none`，`tx_send` は uhd-tx だけが出す（`control.rs:697-705` が `TxCmd::Burst` を渡す）ので，uhd-tx が起きていないと落ちる．
- T6: 0 channel + link から `cold` enable し，configure が別 rate で失敗する fake で，`radio.COMMAND_REJECTED` が 1 件・`action == 1`・理由が `"UR-12: the device applied"` で始まる（`:1584-1588`）．「その stream の line があり，booking の中で configure する」を event で観測する（line が無ければ `control.rs:539-542` の枝で何も出ない）．
- T7: `rec` と `placements` を消した（link 0 本の）Session（`:1944-1947`）を start し，2.5 s 後に fake が Lost になる．`DEVICE_LOST` が 1 件，device の calls で `mark_lost` が `close_streams` より前（`:1953-1954`，`fake.rs:886-891`）．uhd-control の 500 ms time read が損失を見つける（`control.rs:728-737`）ので，「rx の line が無い Run でも start/stop と三本の thread が回る」を通す唯一の既存テスト．
- T8: `control(&core, &[Dir::Rx])`（`:775-787`，core は rig = NoLink 1 本）で `Control::new` の直後に `book_cold` する．`plan_of(&core, Dir::Rx)` の cut が 1 000 000，次の segment の origin が `core.ticks(1_055_000_000)`，`rec.applied` の gain 行が issued，`rec.rejected` が空（`:810-818`）．観測は `core.streams` の plan（直接）．line が無ければ plan が空で `plan[0]`/`plan[1]` が落ちる．`Control::new` が booking より先を固定する．
- T9: `control(&core, &[Dir::Tx])`（tx clock は test が `core.register(Dir::Tx, 200, 0)` で先に作る，`:783`）で，`plan_of(&core, Dir::Tx)[0].clock` が `Some`（`:838`），`to_tx.try_recv()` に `TxCmd::Burst`（`:855`），`core.held` の予約（`:859`）．観測は plan（start の tx clock）と送信 channel．`control.rs:218` の `tx_clocks` 初期化と tx line が直後に要る．
- T10: block 65 536 と 100（packet 1 996）の 2 ケースで `cold` rate change を booking し，Manifest `timing` の `cold_change.booked_at` に対し `rx_segment.origin` が `50 ms + max(block_len, packet)·N + 3 ms` 以上（`:2977-2979`）．packet 項は `Control::new` の `core.device.rx_packet_samples(0)`（`control.rs:194`）から来る（B15/B22 の行がこの規則を殺している）．

### (c) 壊れうる形

- C1 line の key の取り違え・挿入漏れ：rx の line を `(Direction::Tx, 0)` に入れる／両方同じ key に入れる（`Lines::insert` は同じ key を置き換える：`timeline.rs:453-458`）／挿入そのものを落とす．rx の line が無いと uhd-rx は plan なしを cut 0 と読んで最初の block を出さず（`rx.rs:110`），tx の line が無いと `book_burst` に現在 clock が無い（`control.rs:597-601`）．
- C2 挿入の時期：`Control::new`（`mod.rs:471`）が三本の spawn（`:474-477`）より後になると，uhd-rx/uhd-tx の最初の turn が line を見ないことがある（`mod.rs:470` のコメントが順を要求している）．全部を後ろへ移す書き換えは決定的に落ちるが，spawn の直後に挿入する部分的な並べ替えは race で flaky に転ぶ．
- C3 link ガード：`control.rs:208` の `!core.links.is_empty()` を落とす／`start.rx.is_some()` に置き換えると，link が無くても rx の line ができる（channel 数は `start.rx.unwrap_or((0, ..))` なので 0，plan は空）．すると `book_cold` の「link なしは値だけ」の枝（`control.rs:539-542`）に入らず，0 → n の enable が `from_none`（`control.rs:548-568`）から `rx_open` と `apply` を行う．逆に link があり 0 channel のとき line を作らないと，enable の segment が誰にも始められない．
- C4 spawn の条件：spawn を `start.tx.is_some()`／`start.rx.is_some()`／line の有無で囲うと，「stream がなくても起きる」が壊れる（tx は T0 後に enable される `tx_session` 型，rx は 0 channel から enable される型で，所有者がいなくなる）．
- C5 start が置く stream ごとの値の取り違え：`streams.tx_clocks = start.tx.map(..)`（`control.rs:218`）と `streams.idle = [start.rx.is_none(), start.tx.is_none()]`（`:220`）を別の stream・別方向へ入れる，`idle` の 2 要素を入れ替える，`tx_clocks` を instance 全体のまま残す（逆に，`seq`・`ended`・`stop`・`version` を stream ごとへ移す）．`idle` の取り違えは「enable の configure を uhd-control がやるか owner がやるか」（UR-25 の `ready` の意味）を変える．
- C6 device 呼び出しの stream 番号：`start` の `rx_start(0, t0)`（`mod.rs:459`），`Control::new` の `rx_packet_samples(0)`（`control.rs:194`），直前の `arm` の `rx_open(0, ..)`／`tx_open(0, ..)`（`mod.rs:405, 409`）で `k` 以外を渡す．0 でない値を渡すと「その streamer は開いていない」応答になり観測できる（fake も `UhdDevice` も即失敗，`design/18-uhd-radio.md:276`）が，値が 0 のままの取り違え（rx の 0 に tx の 0）は観測できない．
- C7 テスト側の書き換えの取り違え：line を直接書く rig（`rx.rs:646, 669, 1116, 1142`；`tx.rs:992, 1023, 1050, 1070, 1080`；`control/differential.rs:242`；`rx/differential.rs:596, 756, 795, 821, 843`）の key を書き込み側と読み出し側で食い違わせると，コンパイルは通ったままテストが黙って弱くなる．

### (d) 捕まえるか・足す案

- C1：捕まえる．rx 側は T1（`first_rx_block` が無く `.unwrap()` が落ちる），T2（clock が 1 本），T8（plan が空），T3・T4・T6（from 0 の enable が始まらない）；tx 側は T9（`plan_of(Tx)[0].clock` の `expect` が落ちる），T5，`wedged()`（`fake.rs:906-917`）を使う `ur_16_a_wedged_thread_does_not_wedge_stop`（`:920`）．行案（新規）：rx の `insert` の key を `(Direction::Tx, 0)` にした 1 行（killer: T1）．既存行の当て直しの見込みは項目 12 が扱う（この範囲では `control.rs:196-203` の `lead:` を使う F40 と `:194` の B15/B22 が形の変化に応じて対象）．
- C2：全面的な並べ替えは T1（+T8・T9）が決定的に捕まえる．部分的な並べ替えは race で赤の保証が無いので行は作らず，`mod.rs:470-471` の順（`Control::new` が spawn より先）をレビューで確認する．
- C3：link があり 0 channel の側（line を作りすぎ／作らなすぎ）は T3・T4・T6 が捕まえる（T3 の `COMMAND_REJECTED` 空が特に効く）．**link が無い側は今日どのテストも捕まえない**．根拠の検索：`grep -n "Direct::new(\|Direct::with_links(" crates/ezsdr-radio-uhd/tests/fake.rs` のうち link を渡さない rig は `:1327` `ur_24_hardware_timed_updates`・`:1359` `ur_24_a_far_future_update_does_not_delay_a_nearer_one`・`:1764` `ur_24_the_queue_counts_a_command_per_channel`（いずれも `HardwareTimed` だけで `book_timed` を通り line の key を見ない）と `:1561` `ur_25_a_cold_change_the_envelope_refuses_changes_nothing`（`coerce` が先に拒否し `control.rs:539` に届かない）；`grep -rn "NoLink\|rig_with_links\|fn rig()" crates/ezsdr-radio-uhd/src/provider/control.rs crates/ezsdr-radio-uhd/src/provider/rx.rs crates/ezsdr-radio-uhd/src/provider/control/differential.rs crates/ezsdr-radio-uhd/src/provider/rx/differential.rs` は unit 側が全部 link（`NoLink` 1 本か実 link）であることを示す．足すテストの案（`control.rs` の tests に 1 本，`test_support::rig()`（`test_support.rs:13-15`，link ゼロ）を使う）：`Control::new` を `Start { t0: 0, rx: None, tx: Some(..) }` で作り，`book_cold(radio.rx.channels = 1, None)` の後で，`plan_of(&core, Dir::Rx)` が空，`control.config_at(i64::MAX)` に新しい値が入っている，`rec.timing` に `cold_change` が無い，`rec.rejected` が空，`device.calls()` が空（`rx_open` も `apply rx` も無い）を assert する．行案（新規）：`control.rs:208` の `(!core.links.is_empty()).then(||` を `Some((||` に相当する形へ（killer: この新テスト）．
- C4：捕まえる．tx 側は T5（`tx_send` が出ない）と `ur_16_a_wedged_thread_does_not_wedge_stop`（`rejected` に `"uhd-tx"` が出ない，`:924`）；rx 側は T3（`rx_start` が出ない），T4（`rx_segment` が出ない），T6．行案（新規）：tx の spawn を `if tx_clock.is_some() { .. }` で囲う（killer: T5）／rx の `Rx::new` と spawn を `if first.is_some() { .. }` で囲う（killer: T3）．
- C5：捕まえる（検出は隣の順序のテスト）．`idle` の入れ替え・初期値の誤りは `tests/fake.rs:822` `ur_14_actions_are_finished_before_the_next_recv`（tx enable の `apply tx` が submit の帰りまでに済んでいること，`:833`）と T4（`ready − e ≥ 80 ms`）；`tx_clocks` の初期登録を落とす・ずらすと T9（`:838`）と U22 のテスト `ur_25_a_rate_change_admits_a_burst_on_the_new_clock`（行 U22 の `old` は `control.rs:600` の `streams.tx_clocks.last()`）が殺す．行案（新規）：`streams.tx_clocks = start.tx.map(|(clock, _)| (clock, false)).into_iter().collect();` を `Vec::new()` にする 1 行（killer: T9）．`seq`・`ended`・`stop`・`version` を stream ごとへ移す逆方向の誤りはこの順序では観測しない（順序 2・4・9 のテスト）。
- C6：0 でない `k` の取り違えは捕まえる（`rx_start` 等はその streamer なしで即失敗し，T1 が Run の失敗として見る）．`Control::new` の `rx_packet_samples(k)` の誤りは T10 が殺す（B15/B22 の行が同じ規則）．0 と 0 の取り違えは値が同じなのでこの段階では観測できない → 項目 12 の「多数 stream でしか出ない誤り」の一覧へ送る（1b-4c）．
- C7：書き換えたテストを使う既存の lib 行が同じ規則を壊して生き残るかで観測できる：J49（`lib#ur_17_an_ended_segment_prunes_the_items_before_it`，`rx.rs` の `streams.prune(Dir::Rx, by)`），J50（`lib#ur_25_a_burst_on_a_later_clock_survives_an_earlier_switch`，`tx.rs` の `streams.prune(Dir::Tx, by)`），J55（`lib#ur_26_a_stopped_stream_keeps_its_end`，`streams.stop` の読み）．行の追加は要らないが，書き換えたテストを含む行が mutation で死ぬことを実装の報告に書く．
- 0 の置き換えの一覧，mutation 行の当て直しの見込み，複数 stream でしか見えない誤りは，指示どおり項目 12（計画の最後）が扱う．

### 関わる他の順序

- 2（stop）：start が起こした三本を stop が join する（`mod.rs:501-506`）．thread を起こさない誤りは join を no-op にするだけで，End の記帳と `to_tx.send(Shutdown)` は残るため stop のテストだけでは見えない（T3・T5 が見る）．
- 3（panic）：名指しの thread が無ければ panic を記録する主体もいない（`ur_16_a_panicking_thread_is_a_lost_device`，`fake.rs:894` は `rejected` に `"uhd-rx"` を要求，`:902`）．
- 4（device loss）：損失を最初に見つけるのは thread（`rx.rs:428`，`control.rs:728-737`）．uhd-rx の spawn を条件づけると，T0 後に enable した stream の損失検出（silence 監視は streaming 中だけ，`rx.rs:381-386`）が薄くなる．
- 6（stream のない方向の enable）：`Control::new` の 0 channel の line と `idle` の初期値が前提で，configure を誰がやるか（UR-25 の `ready`）がここで決まる．T4・T6 はこの境界のテストでもある．
- 7（受信の cut と delivered の床）：`Rx::new` が start で `streams.delivered` を置く（`rx.rs:153`）．組み直しでこれを落とすと最初の block 前の Stop/abort の床が変わる（順序 7 の担当）．
- 8（tx clock と version）：start の tx clock を `tx_clocks` に置くのは `Control::new`（`control.rs:218`），uhd-tx が最初に plan を読むのは version が `None` のとき（`tx.rs:160-171`）．C5 の誤りは U22 のテストにも現れる．
- 9（同じ instant の cold と timed）：start 直後の instance 全体の `seq` と `version` の初期値が前提で，`seq` を stream ごとへ移すと同 instant の取り合いの順序が方向ごとに別になる（順序 9 の範囲；本順序では観測しない）．
- 11（thread の名前と `rejected.thread`）：名前は start が付ける（`mod.rs:474-477`）．取り違えは `fake.rs:902`・`:924` が見る．

## 2. stop（Drain と Abort）：全 line に End が 1 回だけ記帳される．join の順序は control → 送信 → 受信．

ここでいう「Drain」は `StopMode::Orderly`（`crates/ezsdr-kernel/src/module_api.rs:1183-1188`）．対象は `Provider::stop`（`crates/ezsdr-radio-uhd/src/provider/mod.rs:481-509`）と，それが呼ぶ `Streams::end`（`crates/ezsdr-radio-uhd/src/provider/core.rs:124-134`）．Kernel の `Stop` action は (a) の `ur_26_stop_actions` でだけ触れる．

### (a) 既存のテスト

**End が line に記帳され，line を終わらせること**

- `crates/ezsdr-radio-uhd/src/provider/control.rs:900` `ur_26_an_orderly_stop_cuts_at_its_instant` — `Streams::end` を直接呼び，5 ms の End が Rx の segment を sample 5 000（`delivered` が 6 000 なら 6 000）で切ることを見る．
- `crates/ezsdr-radio-uhd/src/provider/control.rs:916` `ur_26_a_change_booked_after_the_stop_instant_ends_its_clock` — 5 ms の End の後，10 ms に予約した Tx の変更の clock が origin で終わること（Tx line にも End が入ること）を見る．
- `crates/ezsdr-radio-uhd/tests/fake.rs:1711` `ur_25_a_receive_session_follows_its_recorded_plan` — Session の `finish()`（Orderly）で，最後の Rx segment が stop で切られ，各 Rx clock の終わりが plan の cut 以降・次の origin 以前で，その instant の `applied` の `rx_stop` 行があることを見る．
- `crates/ezsdr-radio-uhd/tests/fake.rs:1678` `ur_26_an_abort_publishes_nothing_after_the_stop_instant` — `radio.stop(Abort)` で，publish された最後の sample が stop の instant より前であること，`rx_stop` の device call と `applied` 行が各1回であることを見る．
- `crates/ezsdr-radio-uhd/src/provider/rx.rs:800` `ur_26_a_stopped_stream_keeps_its_end` — 1 ms の `Stop` の後に 5 ms の End（Orderly/Abort，delivered 1 000/300）を置く4通りで，end が 1 000・1 000・300・300 になり，clock もそこで終わり，device の `rx_stop` と `applied` が各1回であることを見る．
- `crates/ezsdr-radio-uhd/src/provider/rx.rs:870` `ur_26_uhd_rx_follows_the_stop_it_reads` — 5 ms の End の後に 7 ms の End を置いても，publish が 5 000 sample で止まり clock も 5 ms で終わること（後の End が end を動かさないこと）を見る．
- `crates/ezsdr-radio-uhd/src/provider/rx.rs:767` `ur_26_a_late_start_after_abort_does_not_stop_again` と `:781` `ur_26_a_late_start_during_orderly_stop_is_recorded_once` — shutdown 中に `LateCommand` が来ても `rx_stop` が1回だけで `rx_start` が無いことを見る．
- `crates/ezsdr-radio-uhd/src/provider/rx.rs:1149` `ur_26_no_segment_begins_once_provider_stop_has_begun` — 5 ms の End の後に置いた 10 ms の変更の segment が始まらないことを見る．
- `crates/ezsdr-radio-uhd/src/provider/rx.rs:1001` `ur_26_an_abort_stops_a_queued_start_at_shutdown`，`:1028` `ur_26_uhd_rx_aborts_when_its_provider_is_gone` — queued start を持つ abort と，command channel が消えた abort の stop 1回を見る．

**join と thread**

- `crates/ezsdr-radio-uhd/tests/fake.rs:920` `ur_16_a_wedged_thread_does_not_wedge_stop` — Tx を `tx_send` で永久に止め，`stop` が 3 s 以内に返り，`rejected` が `uhd-tx` を名指すことを見る．
- `crates/ezsdr-radio-uhd/tests/fake.rs:928` `ur_16_cleanup_keeps_the_streamer_of_a_detached_thread` — 同じく wedged の後，`close_streams` が呼ばれず leak の行が残ることを見る．
- `crates/ezsdr-radio-uhd/tests/fake.rs:894` `ur_16_a_panicking_thread_is_a_lost_device` — panic が `rejected.thread == "uhd-rx"` で記録され，`DEVICE_LOST` になることを見る．
- `crates/ezsdr-radio-uhd/tests/fake.rs:3483` `ur_26_the_shutdown_s_last_burst_ack_waits_on_the_device_s_clock` — stop 後の Tx shutdown の 100 ms ACK window が device の host clock で測られ，最後の `BurstAck` が失われないことを見る．
- `crates/ezsdr-radio-uhd/tests/fake.rs:3509` `ur_33_a_shutdown_with_a_report_read_that_fails_at_once_still_joins_uhd_tx`，`:3582` `ur_29_a_loss_found_by_the_shutdown_s_report_read_ends_its_ack_window` — shutdown の report read の失敗・損失で `uhd-tx` が join し，"did not join" が残らないことを見る．

**記録・冪等・差分**

- `crates/ezsdr-radio-uhd/tests/fake.rs:1651` `ur_26_stop_actions` — Tx/Rx/device/別 target の `Stop` と最後の `finish()` で，burst が `BurstEnd::Stop` で終わり，`applied` に `rx_stop` があり，別 target が拒否されることを見る．
- `crates/ezsdr-radio-uhd/tests/fake.rs:1779` `ur_26_cleanup_is_idempotent` — 未 prepare の `cleanup()` を2回呼んでも壊れないことだけを見る．
- `crates/ezsdr-radio-uhd/src/provider/control.rs:971` `ur_26_a_stop_cancels_no_update` — `Stop` action は held の timed update を消さず，`Provider::stop` の `cancel_held` だけが消すことを見る．
- `crates/ezsdr-radio-uhd/src/provider/rx/differential.rs:111` `rm_26_the_timeline_against_uhd_rx` — 1 000 の seeded sequence を，`Provider::stop` の End item を足した理想の `timeline::Line`（同ファイル `:189`）と比べる．`crates/ezsdr-radio-uhd/src/provider/control/differential.rs:85` `rm_26_the_timeline_against_uhd_control` は control 側の clock と timed update を比べる．

**「End の数」と「join の順序」は固定されていない（検索結果）**

- `grep -rn "Kind::End" crates/ezsdr-radio-uhd/tests crates/ezsdr-radio-uhd/src/provider/*.rs` → 一致は `crates/ezsdr-radio-uhd/src/provider/core.rs:95` と `:132` の実装2行だけ．テストは0件．
- `grep -rn "Transmit before receive\|transmit before receive\|uhd-rx.*uhd-tx\|uhd-tx.*uhd-rx" crates/ezsdr-radio-uhd/tests crates/ezsdr-radio-uhd/src/provider/mod.rs` → `crates/ezsdr-radio-uhd/tests/README.md:21`（participant の列）と `crates/ezsdr-radio-uhd/src/provider/mod.rs:470`・`:472`・`:502` のコメントだけで，テストは0件．
- `grep -rn "\.stop(" crates/ezsdr-radio-uhd/tests/fake.rs` → `:697`（`Direct::finish` の Orderly）と `:1689`（abort のテスト）の2件だけ．`Provider::stop` を2回呼ぶテストは無い．

### (b) assert と観測

仕組み:

- `Streams::end` は `ended` を一度だけ立て（`crates/ezsdr-radio-uhd/src/provider/core.rs:127`），`next_seq()` を一度だけ取り（`:130`），`[Dir::Rx, Dir::Tx]` の各 line に同じ `e`・`seq`・`abort` の `Kind::End` を記帳する（`:131-133`）．`Kind::End` は Manifest に出ない．観測できるのは (i) その line の plan の segment の `cut`（`core.rs:147-159` の `planned` と `crates/ezsdr-radio/src/timeline.rs:826-830` の `Kind::End`），(ii) Tx の clock の `ended_at`（`core.rs:174-193` の `reconcile`），(iii) Rx の `applied` の `rx_stop` 行（`crates/ezsdr-radio-uhd/src/provider/rx.rs:595-613`）．
- `stop` は End を `streams` の lock 1回の中で記帳し（`crates/ezsdr-radio-uhd/src/provider/mod.rs:494-500`），control → Tx → Rx の順に join する（`:501-506`）．`join` は 1 s で打ち切り，`rejected` に `{ "thread": name, "reason": "did not join within 1 s; left detached" }` を積み，`detached` に入れる（`:253-267`）．`cleanup` は detached があれば `close_streams` を呼ばず leak を記録する（`:520-525`）．
- `Provider::stop` の instant は `timing` の `{ "what": "stop", "at": ... }` 行に残る（`mod.rs:507`）．plan は control の `finish()` が `Streams::record` で記録する（`crates/ezsdr-radio-uhd/src/provider/control.rs:259-265`）．
- `Stop` の End は Rx では `timeline::State::cut` を通り（`timeline.rs:826-830`），Tx では `State::ended` を立てるだけ（`:830`）．2つ目の End は Rx では `over`（`:813`），Tx では `ended`（`:820`）で無視されるため，**End を2回記帳しても plan と clock は変わらない**．

各テストの assert と観測:

| テスト | assert | 観測 |
|---|---|---|
| `ur_26_an_orderly_stop_cuts_at_its_instant`（control.rs:900） | 5 ms の End で Rx の cut が 5 000／`delivered` 6 000 なら 6 000 | `planned(Dir::Rx)[0].segment.cut` |
| `ur_26_a_change_booked_after_the_stop_instant_ends_its_clock`（control.rs:916） | 5 ms の End の後の 10 ms の Tx 変更の clock が origin で終わる（2本の clock） | `ClockRegistry::sample_clock_records()` の `origin`／`ended_at` |
| `ur_25_a_receive_session_follows_its_recorded_plan`（fake.rs:1711） | 最後の Rx segment が stop で切れ，各 clock の終わりが plan の cut 以降・次の origin 以前で，その instant の `rx_stop` 行がある．streamer は1本，`rx_stop` は untimed | Manifest の `timing.plan`・`clocks.sample_clocks`・`applied`・`device.calls()` |
| `ur_26_an_abort_publishes_nothing_after_the_stop_instant`（fake.rs:1678） | publish の終わり < stop instant．`rx_stop` の call と `applied` 行が各1回 | link から drain した block，`device.calls()`，`applied` |
| `ur_26_a_stopped_stream_keeps_its_end`（rx.rs:800） | 4通りで end が 1 000／1 000／300／300，clock もそこで終わり，stop は1回 | `rx.stream`，publish した sample 数，`clocks(&core)`，`device.calls()`，`applied` |
| `ur_26_uhd_rx_follows_the_stop_it_reads`（rx.rs:870） | 5 ms の後に 7 ms の End を置いても end は 5 000 | publish した sample 数，clock の `ended_at` |
| `ur_26_a_late_start_after_abort...`（rx.rs:767） | End 2回＋`LateCommand`＋shutdown でも `rx_stop` 1回・`applied` 1行・`rx_start` 無し | `device.calls()`，`applied` |
| `ur_26_no_segment_begins_once_provider_stop_has_begun`（rx.rs:1149） | End の後の変更の segment が始まらない | `rx.begin()` の戻り値，`rx_start` の有無 |
| `ur_26_stop_actions`（fake.rs:1651） | burst の end が `Stop`，`applied` に `rx_stop`，別 target は拒否 | Manifest の `bursts`・`applied`・`rejected` |
| `ur_16_a_wedged_thread_does_not_wedge_stop`（fake.rs:920） | `stop` が 3 s 以内，`rejected` が `uhd-tx` を名指す | 壁時計，`rejected` |
| `ur_16_cleanup_keeps_the_streamer_of_a_detached_thread`（fake.rs:928） | `close_streams` が無く leak 行がある | `device.calls()`，`rejected` |
| `ur_26_the_shutdown_s_last_burst_ack_waits_on_the_device_s_clock`（fake.rs:3483）／`ur_33_a_shutdown...`（fake.rs:3509） | ACK window が device の host clock で閉じ，`uhd-tx` が join する | Manifest の `async`・`rejected` |
| `ur_26_a_stop_cancels_no_update`（control.rs:971） | `Stop` は held を消さず，`Provider::stop` だけが消す | 設定の projection と `applied` の `cancelled` |
| `rm_26_the_timeline_against_uhd_rx`（rx/differential.rs:111） | 1 000 sequence が End item 込みの理想 line と一致 | clock，block，device call，`applied` の stop 行 |

### (c) 壊れうる形

1. **固定 key への記帳**：`Streams::end` が `Lines` の存在する key ではなく `(dir, 0)` を決め打ちで引く．Rx line の無い `Direct`（`crates/ezsdr-radio-uhd/tests/fake.rs:610` の `Direct::new` は link 無し）や Tx line の無い Rx 単体の rig（`crates/ezsdr-radio-uhd/src/provider/rx.rs:646` は Rx line だけを作る）で `Lines::line_mut` が panic する（`crates/ezsdr-radio/src/timeline.rs:633-639`）．
2. **片方向だけへの記帳**：`end` が Rx だけ／Tx だけに書く．Tx 側は `control.rs:916`，Rx 側は `fake.rs:1742`（最後の segment の cut が `Some`）が落ちる．
3. **`ended` を stream ごと・line ごとにする**：2回目の `end`・stop・loss が2つ目の End を記帳する．`timeline.rs:813` の `over` と `:820` の `ended` により plan は同じままなので，既存テストはすべて沈黙する．
4. **`seq` を line ごとに取る／`at` を2回読む**：2本の End の `seq`・`e` がずれる．同一 instant の command との順序（`timeline.rs:367-373`）が変わりうる．manual clock では `now` が同じなので `e` のずれは見えず，`seq` のずれも同一 instant の command が無ければ見えない．低リスクだが，1つの `Item` を作って両 line に渡す形を保つ．
5. **`stop` と `end` を別の lock にする**：control や Rx が End の前に `streams.stop` を見る．control の `finish()` が End 前の plan を記録しうる（`control.rs:259-265`）．`ur_25_a_receive_session_follows_its_recorded_plan` の `segments[3].1.is_some()`（`fake.rs:1742`）が落ちうるが，1 ms poll の race なので確実ではない．
6. **End を control の join より後に置く**：control が先に plan を記録し，stop の cut が plan に無い．`fake.rs:1742` が落ちる．
7. **join の順序を変える／両方向の Shutdown を先に送る**：Tx→Rx の順を Rx→Tx にしても，Rx は Abort なら `streams.stop`（`crates/ezsdr-radio-uhd/src/provider/rx.rs:113`）で，Orderly なら plan の cut で先に止まるため，device の call 順・record 順に差が出ない．既存テストは沈黙する．
8. **`stopped` の早期 return を外す**：2回目の `Provider::stop` が2つ目の `timing` の `"stop"` 行を書き（`mod.rs:507`），2度目の Shutdown を送る．既存テストは stop を2回呼ばない．
9. **`cleanup` の stop 経路を落とす**：`mod.rs:515-517` の `running.is_some() && !self.stopped` の stop が無くなる．既存テストに「running のまま cleanup だけ」が無い（`ur_26_cleanup_is_idempotent` は未 prepare）．
10. **`Running.threads` を stream ごとにし，名前を共有のまま二重登録する**：`join(name)` は最初の1本しか join しない（`mod.rs:255`）．今は各方向1本なので見えない．1b-4c の宿題（thread 名，item 11）．
11. **`k` を使うべき device call・line 参照を `0` のままにする**：stop の経路では `rx_stop(0, None)`（`rx.rs:601`），`tx_send(0, ...)`（`crates/ezsdr-radio-uhd/src/provider/tx.rs:487`），`lines[Rx]`（`rx.rs:109`）．今は `k = 0` なので挙動は同じで，2本目で初めて出る．1b-4c へ．
12. **`Streams::record`／`planned` が旧 array のまま**：`core.rs:148-159`・`:197-207` が `(dir, 0)` だけを読むと Tx の plan が `timing.plan` から落ちる．既存テストは `plan["tx"]` を読まない（`grep -rn '"tx"' crates/ezsdr-radio-uhd/tests/fake.rs` の一致は selector と別の用途だけ）．

### (d) 捕まえるか・足す案

| (c) | 既存のテストが捕まえるか | 足す案 |
|---|---|---|
| 1 | 捕まえる（Rx-only の `streams.end` と `Direct` の stop が panic） | 不要 |
| 2 | 捕まえる（`control.rs:916` と `fake.rs:1742`） | 不要．mutation は既存 R15 の近く |
| 3 | 捕まえない（plan が同じ） | 下の T1 と新しい mutation 行 |
| 4 | 主に捕まえない（manual clock で `now` が同じ） | T1 で `e`・`seq` の同値を assert．`at` の二重読みは観測不能として記録 |
| 5 | 確実には捕まえない（race） | lock 1回を item 10 の review で確認 |
| 6 | 捕まえる（`fake.rs:1742`） | mutation: `streams.end` を `join("uhd-control")` の下へ移す |
| 7 | 捕まえない（Rx は plan で先に止まる） | 下の T3 と mutation（Rx→Tx に入れ替え） |
| 8 | 捕まえない | 下の T2 と mutation（`stopped` の early return を外す） |
| 9 | 捕まえない | 任意の T4（cleanup だけの停止） |
| 10・11 | 今は見えない | 1b-4c へ．stop 経路の `0` は final の一覧にも送る |
| 12 | 捕まえない | 任意の T5（`record()` の両方向） |

**T1: `ur_26_a_stop_ends_every_line_once`（unit，`control.rs` の tests）**

- 狙い：`end` が存在する全 line に，同じ `e`・`seq`・`abort` で End を1つだけ記帳し，2回目は何も記帳しないこと．(c)3・4 を固定する．
- 形：`rig()`（`control.rs:769`，link あり）で `control(&core, &[Dir::Rx, Dir::Tx])` を作り，`streams.end(&core, ms(5), false, false)` の後に `streams.seq` を控え，`streams.end(&core, ms(7), true, false)` を呼ぶ．次に `(Direction::Rx, 0)` と `(Direction::Tx, 0)` の `Lines::line` を引き，各 `line.items` の `Kind::End` が1つで，`e == ms(5)`・`seq == 先の seq`・`abort == false` であること，2回目の後も `streams.seq` が変わらないことを assert する．
- 観測：`Streams` の line の `items`（既存の unit test も `rx.rs:1116`・`:1142`，`tx.rs:1070`・`:1080` で `items` を読んでいる）．
- 既存で捕まらない理由：`Kind::End` を数えるテストが無く，2つ目の End は plan を変えない（(b) の `timeline.rs:813`・`:820`）．
- mutation 行の案：`Streams::end` の `if std::mem::replace(&mut self.ended, true) { return; }` を「常に続行」に変える（正確な `old` は実装後に取る）．T1 が2つ目の End と `seq` の増加で殺す．

**T2: `ur_26_a_second_stop_does_nothing`（integration，`fake.rs`）**

- 狙い：`Provider::stop` の2回目が何もしないこと（UR-26）．(c)8 を固定する．
- 形：`Direct::with_links(FakeConfig::default(), &[], vec![attached(DropOldest)])` で Rx の stream を持つ provider を作り，`direct.radio.stop(StopMode::Orderly)` を2回呼んでから `direct.finish()`．`instance.sections` の `ezsdr.radio.uhd.timing` で `what == "stop"` の行が1つ，`rejected` に "did not join" が無く，`calls(&direct.device, "rx_stop")` が1つであることを assert する．
- 既存で捕まらない理由：`grep -rn "\.stop(" crates/ezsdr-radio-uhd/tests/fake.rs` の2件はどちらも1回だけ．
- mutation 行の案：`stop` の `if std::mem::replace(&mut self.stopped, true) { return Ok(()); }` を「常に続行」に変える．2つ目の `timing` 行が T2 を殺す．

**T3: `ur_26_the_sending_threads_join_before_the_receiving_ones`（integration，`fake.rs`）**

- 狙い：join の順序（control → 全 Tx → 全 Rx）を，(c)7 の「device call では差が出ない」状況でも観測できる形で固定する．`join` の打ち切りが `rejected` に積む順序（`mod.rs:263-267`）を使う．
- 形：`Device` の薄い wrapper を作り，`rx_recv` だけを永久に `std::thread::sleep` させる（他の call は `FakeDevice` へ転送する．`LostAtShutdown`（`fake.rs:3528-3578`）と同じ形）．中身は `FakeConfig { faults: vec![FakeFault::TxBlocks], .. }` の real-clock fake にし，`tx_session_on` で repeat burst を送って `tx_send` で Tx を止める．`run.finish()` を `bounded(30s)` で流し，`rejected` の "did not join" 行の `thread` が `["uhd-tx", "uhd-rx"]` の順であること（両方あること）を assert する．
- 既存で捕まらない理由：(a) の grep のとおり join の順序を固定するテストが無く，Rx は plan の End で先に止まるため device の call 順では区別できない．
- mutation 行の案：`mod.rs` の `to_tx.send(TxCmd::Shutdown); join("uhd-tx"); to_rx.send(RxCmd::Shutdown(mode)); join("uhd-rx");` の Tx と Rx の組を入れ替える（実装後の正確な `old` で）．T3 の `rejected` の順序が `["uhd-rx", "uhd-tx"]` になって殺される．
- 費用：join の打ち切り2回で約2 s．detached の thread は `ur_16_a_wedged...` と同じくテスト終了まで残る．

**T4（任意）: `ur_26_the_control_thread_joins_first`**

- 狙い：control を先に join することを固定する．(c)7 の control 版．
- 形：`time_now` を `std::thread::current().name() == Some("uhd-control")` のときだけ永久に止める wrapper と，T3 の Rx wedge を組み合わせ，device の 500 ms の check（`control.rs:710-738`）が来るまで待ってから `finish()`．`rejected` の "did not join" が `["uhd-control", "uhd-rx"]` の順であることを assert する．
- 注意：wrapper が thread 名に依存するので，T3 より侵襲的．control の join 位置は device call にも record にも出ないため，これか test-only の join log 以外に固定する方法が無い．採るかは orchestrator の判断．

**T5（任意）: `ur_30_the_recorded_plan_has_both_directions`**

- 狙い：`Streams::record` が `Lines` の全 line を記録すること（(c)12）．`control(&core, &[Dir::Rx, Dir::Tx])` の後に `streams.record()` を呼び，`rx` と `tx` の両方に segment があることを assert する．
- 既存で捕まらない理由：`fake.rs:1742` は `plan["rx"]` だけ，`ur_30_the_recorded_plan_has_no_refused_segment`（`control.rs:863`）も `rx` だけ．

**既存 mutation 行で当て直しが要りそうなもの（この項目の近く）**

- `plan/phase7/tools/mutations.json:1057` の R15（`streams.end(&core, at, mode == StopMode::Abort, false);`）— `stop` の呼び出し文が変われば当て直す．T1・T2 は R15 と同じ規則の別方向を固定する．
- `plan/maintenance/tools/mutations.json:788` の J46，`:868` の J54 — `delivered` を stream ごとに移すと `item.delivered` の式が変わる．
- 同 `:798` の J47，`:911` の J58 — `device_lost` の `streams.end(...)` の署名が変われば当て直す（loss は item 4 だが `ended` を共有する）．
- 同 `:529` の J09 など Rx の `lines[Rx]` を読む行 — `Lines::line((Rx, 0))` に変わる．
- 同 `:1060` 付近の R21 など，`tx_clocks` を読む行 — stream ごとの型に移すと当て直す（item 8）．
- 新しい行は `plan/maintenance/tools/mutations.json` に，T1・T2・T3（と T4 を採るなら T4）の4行を足す．`old` は実装後に exact text を取る（この段階では実装が無い）．

### 関わる他の順序

- **1（start）**：`start` 前・`running` 無しの `stop` は早期に返る（`mod.rs:488-490`）．`stopped` を先に立てる順序を変えると，start に失敗した後の2回目の stop が `timing` 行を書く可能性がある（T2 は running の場合だけを固定する）．
- **3（thread の panic）**：panic の `catch_unwind` が `core.device_lost` を呼び（`mod.rs:238-244`），その `Streams::end` と stop の End が同じ `ended` を奪い合う．先に勝った方の instant・abort が両 line の End になり，`rejected.thread` は panic した thread 名のまま（`ur_16_a_panicking_thread_is_a_lost_device`，`fake.rs:894`）．
- **4（device loss）**：loss は `lost=true` で `seq=0`（`core.rs:130`）．stop と同時なら loss が先に勝ち，stop の End は何も記帳しない．`ur_26_a_stopped_stream_keeps_its_end` は Stop と stop の組だけなので，loss と stop の競合は別に見る必要がある（item 4）．
- **5（設定の拒否）**：拒否で halt した line にも End は記帳され，End は拒否を解除しない（`timeline.rs:820-830`）．同じ instant では `seq` の順で拒否と End が並ぶため，`seq` を line ごとにすると順序が変わる（(c)4）．
- **6（channel 数 0 からの enable／0 への disable）**：disable の `tx_open(0, &[])`（1b-4a の境界）と enable の `configure` の途中で stop が来ると，control の join がその device call を待つ（最大 1 s，`mod.rs:257-267`）．End は enable の後か前に置かれ，`item.ready` と `e` の順で segment が始まるかが決まる．
- **7（受信の cut と `delivered` の床）**：Abort の End は Rx の `delivered` を床にする（`core.rs:95-98`）．`delivered` を stream ごとの型に移すとき，stop の End がその stream の値を読むこと（別 stream の値や instance 全体の古いコピーを読まないこと）が要点．`ur_26_a_stopped_stream_keeps_its_end`（`rx.rs:800`）が 1 000/300 の差で固定する．
- **8（送信 clock と `version`）**：End の記帳が `changed` → `reconcile` を呼び（`core.rs:161-194`），stop の instant より後に予約された変更の clock を origin で終わらせる（`control.rs:916`）．`version` を stream ごとにすると，Tx が stop の plan を読み直さない可能性がある（今は instance 全体）．
- **9（同一 instant の cold 変更と timed 変更）**：End の `seq` は instance の `next_seq`（`core.rs:130`）．同じ instant で先に受けた `cold` 変更が先に効き，End がそれを切る．`seq` を line ごとに取るとこの順序が逆になりうる．
- **10（lock の順序と device call の間の line lock）**：`stop` は `streams` の lock を End の記帳の間だけ持ち（`mod.rs:494-500`），join や device call の間は持たない．lock の順序は `updates` → `streams` → `rec`（`core.rs:55-56`）．`end` を2つの lock に分けると (c)5 の race が生じ，`end` を join の後に置くと (c)6 の stale plan が生じる．
- **11（thread の名前と `rejected.thread`）**：今の名前は `uhd-rx`・`uhd-tx`・`uhd-control`（`mod.rs:474-477`）で，`join` と `detached` は名前で引く（`:253-267`）．1b-4c で stream ごとの名前にするとき，同じ名前の thread を2本登録すると `join` が1本しか待たない（(c)10）．`ur_16_a_panicking_thread_is_a_lost_device` は `uhd-rx` を期待している．

## 3. stream の thread の panic：その thread の名前で `rejected` に記録され，device の loss として全 stream が終わる

### (a) 既存のテスト

- `crates/ezsdr-radio-uhd/tests/fake.rs:894` `ur_16_a_panicking_thread_is_a_lost_device` — **唯一，thread の panic そのものを起こすテスト**．`FakeFault::RxPanic(2.2s)`（fake.rs:896）を rx だけの Spec Run（fake.rs:897 の `receive_spec(1, 1e6, 1e9, None)`）で走らせる．
- `crates/ezsdr-radio-uhd/tests/fake.rs:886` `assert_marked_lost_before_closed`（同テストが fake.rs:903 で呼ぶ）— F4：`mark_lost` が 1 回，`close_streams` より前．
- loss の「全 stream の終わり」側を **panic 以外の経路**で固定しているもの（unit，rx のみの rig）：
  - `crates/ezsdr-radio-uhd/src/provider/rx.rs:889` `ur_29_a_loss_ends_the_stream_at_its_instant`
  - `crates/ezsdr-radio-uhd/src/provider/rx.rs:913` `ur_29_a_loss_found_after_uhd_rx_read_the_plan_ends_the_stream`
  - `crates/ezsdr-radio-uhd/src/provider/rx.rs:935` `ur_29_a_silent_stream_ends_at_a_cut_booked_during_its_wait`
  - `crates/ezsdr-radio-uhd/src/provider/rx.rs:966` `ur_29_a_loss_takes_effect_before_a_change_at_its_instant`（loss の seq 0）
  - `crates/ezsdr-radio-uhd/tests/fake.rs:1850` `ur_29_a_lost_device_aborts_the_run`（rx のみ），`crates/ezsdr-radio-uhd/tests/fake.rs:1814` `ur_28_a_failed_report_read_is_not_an_empty_one` の後半（fake.rs:1826-1832，tx のみ）
- 隣接（panic ではなく join 側）：`crates/ezsdr-radio-uhd/tests/fake.rs:920` `ur_16_a_wedged_thread_does_not_wedge_stop` — `rejected` が thread `"uhd-tx"` の行を持つこと（fake.rs:924）を固定している．

### (b) assert と観測

`ur_16_a_panicking_thread_is_a_lost_device`（fake.rs:894）:

- `FakeFault::RxPanic(2.2s)`（fake.rs:896）が device time 2.2 s の `rx_recv` で panic する（device.rs:1212，panic そのものは device.rs:1245）．
- panic は uhd-rx の `turn`（rx.rs:241 の `rx_recv`）から `run`（rx.rs:205）を巻き戻り，`spawn` の `catch_unwind`（`crates/ezsdr-radio-uhd/src/provider/mod.rs:238`）が捕まえる．
- mod.rs:240 が `{"thread": name, "reason": "panicked"}` を `Records.rejected`（`crates/ezsdr-radio-uhd/src/provider/core.rs:215`）に記録 → Manifest の `rejected` section（mod.rs:279．読みは `section`，`crates/ezsdr-radio-uhd/tests/common/mod.rs:376`）．
- mod.rs:241-243 の `if name != "uhd-control"` で `device_lost("a Provider thread panicked")`（core.rs:383）: `Streams::end(..., lost=true)`（core.rs:388）が値 `End`（seq 0，core.rs:130）を**両方向の line** に記帳，`lost` flag（core.rs:389），`device.mark_lost()`（core.rs:392，fake では device.rs:1609-1610 が calls に残す），`DEVICE_LOST` を `usrp` から emit（core.rs:394-399）．
- 観測：テストは (i) 実行結果が `Stopped { cause: Policy { event: DEVICE_LOST } }`（fake.rs:901），(ii) `rejected` に `thread == "uhd-rx"` の行（fake.rs:902），(iii) fake の `calls()` の順（fake.rs:903 → 886-891）だけを見る．`run_until_end` は終わらなければ `Ok` を返す（`crates/ezsdr-kernel/src/coordinator/stepping.rs:559-580`）ので，loss が起きなければ fake.rs:898-899 が落ちる．
- line への反映：rx line の End は uhd-rx が `is_lost` を読んで `finish`（rx.rs:222-226，rx.rs:321-333）が clock を終わらせる．tx line は HEAD では常にあり（`crates/ezsdr-radio-uhd/src/provider/control.rs:214`。rx のみの Spec では 0 channel），End は `changed`→`reconcile`（core.rs:162-194）が tx clock を終わらせる．**panic テストはこの tx 側を読んでいない**．

unit（rx.rs:889/913/935/966）は `core.device_lost("test")` を直接呼び，`rx.turn()`/`rx.finish()` と `clocks(&core)`（各 clock の `(origin, ended)`），`lock(&core.streams).planned(Dir::Rx)` の cut，`device.calls()` の `rx_stop` 回数で観測する．

### (c) 壊れうる形

1. **thread 名**：spawn が stream ごとになっても名前は `"uhd-rx"`/`"uhd-tx"` のまま（指示）．stream 番号入り（1b-4c の形）や別名にすると mod.rs:240 の note の `thread` が変わる．`name != "uhd-control"`（mod.rs:241）を stream ごとの条件に置き換えると，control の panic が loss になる／ならないが変わる（既存テストでは見えない，下記 (d)）．
2. **全 stream の終わり**：`Streams.lines` が `timeline::Lines`（key `StreamKey = (Direction, u32)`，`crates/ezsdr-radio/src/timeline.rs:411,419`）になると，`end`（core.rs:126-134）の「両方向に End」は「全 key に End」の書き換えになる．(i) panicking thread の stream だけに記帳する，(ii) 方向や k を取り違えて別 key に記帳する，(iii) 存在しない key に `Lines::book` を呼ぶ（`line_mut` は panic する，timeline.rs:633-639）——いずれもありうる．
3. **delivered の床の取り違え**：`delivered`（core.rs:76）が stream ごとになると，rx の End の床（core.rs:95-98）を別 stream の値から取ると終わりが動く．
4. **一度きりの崩れ**：`ended`（core.rs:71，core.rs:127-129）と `lost`（core.rs:271，core.rs:385-389）が instance 全体であること．stream ごと・line ごとに割ると End や DEVICE_LOST が 2 回になりうる．
5. **note の生存**：`cleanup` の `write_sections`（mod.rs:270-282，mod.rs:526）が loss 経路で呼ばれなくなると note が Manifest に出ない．
6. **rx のみのテストの盲点**：fake.rs:902-903 は rx の行と `mark_lost` しか見ないので，**tx line の End だけを落とす書き換えは緑のまま**通る．
7. **文面の未固定**：`rejected` の行の `reason`（"panicked"）と `DEVICE_LOST` の message（"a Provider thread panicked"）はどのテストも見ていない（fake.rs:902 は `thread` だけ，fake.rs:901 は kind だけ．`Provider thread panicked` を検索しても実装 mod.rs:242 以外に出ない：`grep -rn "Provider thread panicked" crates/ plan/ design/`）．
8. 今の key では各方向 1 本（k=0 のみ）なので，k 同士の取り違えの多くはこの段階では観測できない（1b-4c 送りの一覧は「計画の最後」の担当）．

### (d) 捕まえるか・足す案

**捕まえる：**

- panic → note（thread 名）→ loss：fake.rs:898-903 が捕まえる．note を消す mutant は fake.rs:902，`device_lost` を呼ばない mutant は horizon を過ぎても Run が終わらず fake.rs:899，`catch_unwind` の除去も同様（note も loss も出ない）．
- `mark_lost` 1 回・`close_streams` より前：fake.rs:903 → 886-891．
- loss の rx line の終わり（sample と plan の cut）：rx.rs:889/913/935（ただし stream 1 本）．
- loss の seq 0（RM-25 の床）を単位とする既存 mutation：J60（`plan/maintenance/tools/mutations.json:929-937`，filter `ur_29_a_loss_takes_effect_before_a_change_at_its_instant`）．
- この経路の既存の行：J47（`plan/maintenance/tools/mutations.json:798-806`，`device_lost` の `streams.end` を消す），J46（`:788-796`，delivered の床），J58（`:907-917`，end と flag の順），B33（`plan/phase7/tools/mutations.json:1469-1477`，`mark_lost`）．**panic 側の行は存在しない**：`grep -rn '"panicked"\|Provider thread panicked\|catch_unwind(AssertUnwindSafe' plan/*/tools/mutations.json plan/maintenance/tools/mutations.json` → 一致なし．

**捕まえない（足す案）：**

1. **全 stream の終わり（tx 側）**：両方向の Spec Run（`with_tx(receive_spec(1, 1e6, 1e9, None), 1e6)`，`crates/ezsdr-radio-uhd/tests/common/mod.rs:201-207`）で `FakeFault::RxPanic` を起こし，`usrp/tx` の SampleClock が `ended_at` を持つこと，または `timing.plan.tx[...]["end"]` が Some であることを assert する案．mutation の行は**実装後の text** で `Streams::end` の「全 key」を壊す形（例：key の列から tx を落とす；この mutant は rx のみの既存テストでは死なない）．
2. **message**：fake.rs:894 の会話に `events_of(&manifest, EventKind::DEVICE_LOST)[0].payload["message"] == "a Provider thread panicked"` を足す案（文面は design/18-uhd-radio.md:200）．mutation の行：mod.rs:242 の文字列を変える行——今は survivor．
3. **tx thread の panic**：fake に tx 側の panic fault が無い（`grep -rn "TxPanic" crates/` → 一致なし）．`FakeFault::TxPanic(Duration)` を device.rs:275-276 の隣に足して同じテストをする案（1b-4c 送りでもよい）．
4. **uhd-control の panic は note だけで loss にしない**（mod.rs:241）：これを固定するテストは無い（`grep -rn "uhd-control" crates/ezsdr-radio-uhd/tests/` はコメントのみ；`grep -rn "panicked" crates/ezsdr-radio-uhd/tests/` → 一致なし）．control の panic を起こす仕掛けが今の fake に無いので，**この段階では gate を変えないこと**の確認に留めるのが妥当．

（注：この段階は「テストは変えない」なので，1〜3 は採否を orchestrator が決める提案として書いている．実装のままの text で mutation の `old` を取る．）

### 関わる他の順序

- **1（start）**：thread を起こす前に line が入る前提（control.rs:216-221 → mod.rs:474-477）．line の生成を thread 側へ動かすと「line が無い間に panic」で loss が記帳されない．
- **2（stop）**：panic の `device_lost` と `Provider::stop` の `end`（mod.rs:494-500）が `streams` の lock で競合する．`ended`（core.rs:71,127-129）が instance 全体でないと End が 2 回・後ろの instant で記帳されうる．
- **4（別 thread が loss を発見）**：互いに `device_lost` を呼ぶ（core.rs:385-389 の flag と lock）．`lost` を stream ごとにすると DEVICE_LOST が 2 回出る（fake.rs:886-891 の 1 回 assert が一部を支える）．
- **5（stream の device が設定を拒否，RM-25）**：拒否は 1 本の line だけ halt させる（core.rs:110-122）．panic の loss は halt した line も終わらせるので，key の取り違えが loss の記帳に紛れうる．
- **6（0 からの enable / 0 への disable）**：rx line は link が無いと存在せず（control.rs:208-211），tx line は 0 channel でも常にある（:212-214）．存在しない key へ End を記帳すると `Lines::book` が panic する（timeline.rs:633-639）——tx のみの loss（fake.rs:1826-1832）がこの位置にある．
- **7（rx の cut と delivered の床）**：loss の End の床（core.rs:95-98）が stream ごとの `delivered` になる．別 stream の値を使うと rx の終わりが動く（rx.rs:889 は 1 本では捕まらない）．
- **8（tx の clock の登録と終わり）**：loss の End が tx segment を切り `reconcile`（core.rs:174-194）が clock を終わらせる．`tx_clocks` を stream ごとにすると，他の stream の clock が開いたまま残りうる（今は 1 本で観測不能）．
- **9（同じ instant の cold と timed，UC-2/RM-25）**：loss は seq 0（「fault と同時，Run の始め」）で記帳される（core.rs:130）．seq を stream ごとにすると同 instant の順序が変わる（rx.rs:966 が rx 側を支える）．
- **10（lock の順序，device 呼び出しの間に line の lock を持たない）**：panic は unwind で guard を落としてから note と `device_lost` を走らせる（mod.rs:238-244，poison は core.rs:22-24 が回復）．`streams` の中を stream ごとの lock に割ると `end` が複数 lock を取り，順序・deadlock の新しい面になる．
- **11（thread の名前と `rejected` の `thread`）**：同じ note の行（mod.rs:240）を両方が見る．名前を変える変更は 3 と 11 の両方を落とすので，3 は「panic → loss の意味」，11 は「名前の一覧」を持つ．

## 4. ある stream の thread が device の loss を見つける：全 stream が 1 回だけ終わる（`ended` は instance 全体）

### (a) 既存のテスト

loss を見つけた thread が呼ぶのは `Core::device_lost`（`crates/ezsdr-radio-uhd/src/provider/core.rs:383`）で，呼び元は uhd-rx（`crates/ezsdr-radio-uhd/src/provider/rx.rs:385`，`rx.rs:428`，`rx.rs:610` の `device_failed`），uhd-tx（`crates/ezsdr-radio-uhd/src/provider/tx.rs:692`，`tx.rs:366,394,488,675` の `device_failed`），uhd-control（`crates/ezsdr-radio-uhd/src/provider/control.rs:719,730,735`），panic の捕捉部（`crates/ezsdr-radio-uhd/src/provider/mod.rs:240-243`）．どの thread も同じ 1 本を通り，「見つけたのは誰か」は渡らない．

- `ur_29_a_loss_ends_the_stream_at_its_instant` — `crates/ezsdr-radio-uhd/src/provider/rx.rs:889`
- `ur_29_a_loss_found_after_uhd_rx_read_the_plan_ends_the_stream` — `crates/ezsdr-radio-uhd/src/provider/rx.rs:913`
- `ur_29_a_silent_stream_ends_at_a_cut_booked_during_its_wait` — `crates/ezsdr-radio-uhd/src/provider/rx.rs:935`
- `ur_29_a_loss_takes_effect_before_a_change_at_its_instant` — `crates/ezsdr-radio-uhd/src/provider/rx.rs:966`
- `ur_29_eob_device_errors_are_escalated` — `crates/ezsdr-radio-uhd/src/provider/tx.rs:1200`
- `ur_29_eob_loss_during_switch_stops_device_calls` — `crates/ezsdr-radio-uhd/src/provider/tx.rs:1226`
- `ur_29_eob_loss_from_report_prevents_next_burst_send` — `crates/ezsdr-radio-uhd/src/provider/tx.rs:1249`
- `ur_16_a_panicking_thread_is_a_lost_device` — `crates/ezsdr-radio-uhd/tests/fake.rs:894`（panic 経由で同じ `device_lost` に来る．順序 3 と共有）
- `ur_29_a_lost_device_aborts_the_run` — `crates/ezsdr-radio-uhd/tests/fake.rs:1850`
- `ur_29_a_silent_stream_is_a_lost_device` — `crates/ezsdr-radio-uhd/tests/fake.rs:1863`
- `ur_29_a_dead_link_that_fails_without_lost_is_a_lost_device` — `crates/ezsdr-radio-uhd/tests/fake.rs:1886`
- `ur_29_a_lost_device_is_found_while_idle` — `crates/ezsdr-radio-uhd/tests/fake.rs:1941`
- `ur_29_a_stream_silent_before_a_far_cut_is_a_lost_device` — `crates/ezsdr-radio-uhd/tests/fake.rs:2844`
- `ur_29_a_loss_found_by_the_shutdown_s_report_read_ends_its_ack_window` — `crates/ezsdr-radio-uhd/tests/fake.rs:3582`
- `rm_26_the_timeline_against_uhd_control` — `crates/ezsdr-radio-uhd/src/provider/control/differential.rs:85`（loss を含み，両方向の line を持つ唯一の比較テスト）
- `rm_26_the_timeline_against_uhd_rx` — `crates/ezsdr-radio-uhd/src/provider/rx/differential.rs:111`
- 規則側（Provider ではない）: `rm_26_the_timeline_cases` — `crates/ezsdr-radio/tests/timeline.rs:141`（loss の line ごとの意味は `crates/ezsdr-radio/tests/timeline.rs:266-290`）
- `Provider::stop` が同じ `end` で受信 side を終えること: `ur_25_a_receive_session_follows_its_recorded_plan` — `crates/ezsdr-radio-uhd/tests/fake.rs:1711`

### (b) assert と観測

- `rx.rs:889` は loss を `core.device_lost("test")`（`rx.rs:903`）で記帳し，受信 SampleClock が loss の sample で終わること（`clocks(&core)` が `[(0, Some(clock.instant(expected)))]`，`rx.rs:907`），plan の cut が同じ sample（`rx.rs:908`），stream が消えること（`rx.rs:906`）を見る．`expected` は loss の instant を配送済みの床で持ち上げた値で，5 ms で 3 000 配送済みなら 5 000，2 ms の loss なら 3 000（`rx.rs:895-901`）．観測は `core.clocks.sample_clock_records()` と線の plan だけ．
- `rx.rs:913` は uhd-rx が plan を読んだ後に他の thread が見つけた loss でも，`finish` が plan を読み直して同じ cut で終わること（`rx.rs:928-930`）を，やはり clock と plan で見る．
- `rx.rs:935` は受信呼び出しの待ちの間に他の thread が記帳した loss でも，plan の cut（5 000）で stream と clock が終わること（`rx.rs:958-961`）を見る．`Device` の calls は見ない．
- `rx.rs:966` は loss が `seq = 0`（Run の最初に受けた fault）で，同じ instant の `cold` 変更より先に効くこと（plan は 1 segment，cut 12 000，`rx.rs:978-979`）を plan で見る．mutation は J60（`plan/maintenance/tools/mutations.json:928-937`）．
- `tx.rs:1200` は `tx_send` の end-of-burst が `lost` を返すと `core.is_lost()` が立ち（`tx.rs:1212`），`DEVICE_LOST` が 1 回で（`tx.rs:1215`），2 度目の close が何も出さず（`tx.rs:1218`），`mark_lost` が 1 回（`tx.rs:1219`）であることを event と device の calls で見る．ただし rig には line がない（`rig()` は links なしで `Streams` は `[None, None]` のまま，`core.rs:58-60`）．
- `tx.rs:1226` は switch 中の loss で loss 以降の device 呼び出しが `["mark_lost"]` だけ（`tx.rs:1244`），`tx.clock` が none（`tx.rs:1245`）であることを calls で見る．`tx.rs:1249` も report 中の loss で次の burst を送らないこと（`tx.rs:1262-1265`）を calls で見る．
- `fake.rs:1850` は e2e で，fake の `Lost` が uhd-rx の `rx_recv`（`rx.rs:428`）で見つかる経路を回し，`termination` が `Stopped { policy { DEVICE_LOST } }`（`fake.rs:1857`），event の source が `usrp`（`fake.rs:1858`），`mark_lost` が `close_streams` より前で 1 回（`fake.rs:1859` → `fake.rs:886-891`）を見る．plan と tx 側は見ない．
- `fake.rs:894` は uhd-rx の panic で `rejected` に `{"thread": "uhd-rx"}`（`fake.rs:902`），DEVICE_LOST での停止（`fake.rs:901`），`mark_lost` の位置（`fake.rs:903`）を見る．
- `fake.rs:1886`，`fake.rs:1941`，`fake.rs:2844` は `DEVICE_LOST` がちょうど 1 回（`fake.rs:1902`，`fake.rs:1953`，`fake.rs:2858`）を見る．これは `Core::lost` の once（`core.rs:385-389`）であり，`ended` の once ではない．
- `fake.rs:3582` は shutdown の最初の timeout 付き `tx_async` が `lost` を返すと ACK window が終わり，uhd-tx が join し（`did not join` がない，`fake.rs:3591-3592`），loss 後の device 呼び出しがない（`fake.rs:3593`）ことを見る．loss を見つけるのは uhd-tx の thread．
- `control/differential.rs:85` は loss を `core.device_lost` で記帳し（`control/differential.rs:211-213`），両方向の line を持ち（`Control::new` の `control/differential.rs:201` → `control.rs:216-221`），`tx_clocks` を timeline の account と比較する（`control/differential.rs:59`，`:264-269`）．期待側は loss を `Kind::End` として両方向の items に入れる（`crates/ezsdr-radio/tests/generator/mod.rs:196-211`，`control/differential.rs:137-140`）．ただし比較するのは clock の (origin, ended) で，`End` の item 自体は読まない．
- `rx/differential.rs:111` は VH-8 の 1 000 seed の loss をどの thread でもよいとして `core.device_lost` で記帳し（`rx/differential.rs:788-790`），受信 clock/plan を model と比べる（`rx/differential.rs:822-846`）．rx の line だけ（`rx/differential.rs:754-756`）．
- `fake.rs:1711` は `Provider::stop` の `end` が受信 plan の最後の segment に cut を入れ（`fake.rs:1742`），各受信 clock がその cut で終わる（`fake.rs:1743-1755`）ことを見る．loss と stop が同じ `end` を通ることの，stop 側の証拠．

### (c) 壊れうる形

1. `end` の宛先が「見つけた stream」になる．`device_lost` が `(Direction, k)` を受け取り（または `end` が `StreamKey` を受け取り），その line にだけ `End` を記帳する．見つけた側の stream だけが終わり，もう一方（今は tx）の line が終わらない．
2. `end` の一括ループが `timeline::Lines`（`crates/ezsdr-radio/src/timeline.rs:409-427`）の反復に直るときに line を取りこぼす．例：`for dir in [Dir::Rx, Dir::Tx]`（`core.rs:131`）を `self.configs` や `self.idle` の key で回す（`configs` は Run の始め空で，`control.rs:216-221` が入れるのは lines・tx_clocks・idle だけ），`if k == 0` を残す，`Lines::insert` した順の 1 本だけ回す．落とした方向の line に `End` がない．
3. `ended` を per stream（または line ごと）に移す．`end` の 1 回目で line ごとの flag が立ち，`Provider::stop`（`mod.rs:494-499`）が loss の後に 2 つ目の `End` を記帳する．planner は `over` で後続を無視する（`crates/ezsdr-radio/src/timeline.rs:813-815`）ので plan と clock は変わらず，`items` にだけ 2 つ残る．
4. `seq` を loop の中で採る．`let seq = if lost { 0 } else { self.next_seq() };`（`core.rs:130`）が line ごとの `next_seq()` になり，tx line の loss が `seq > 0` になる．RM-25（`design/07-radio-model.md:258`）の「loss は Run の最初に受けた fault」が tx で崩れ，同じ instant の tx の `cold` 変更との順序が変わる．
5. `delivered` の床を「見つけた stream」から引く．`book` の床（`core.rs:93-98`）が per stream になった後，終わる line 自身でなく `device_lost` に渡された `k` の `delivered` を使う．未配送の受信サンプルを cut の前に捨てる（受信が 1 本の今は見えない）．
6. `book` の key の取り違え．`book(core, dir, …)`（`core.rs:92`）が `book(core, stream, …)` になったとき，方向を間違える，または `k` を 0 決め打ちにする（`Lines::line_mut` は key がないと panic，`timeline.rs:633-639`）．今は両方向とも k=0 なので，方向の取り違え（`(Tx,0)` と `(Rx,0)`）だけが観測できる．
7. `device_lost` の順序．`lost` の判定を `streams` の lock の外へ出す，`mark_lost` を lock の中へ入れる，`end` の後に `lost.store` をやめる（`core.rs:384-392`）．J58（`plan/maintenance/tools/mutations.json:908-917`）が equivalent と記録している箇所で，lock の外に出すと 2 つの thread が同時に `end` に来られる．

### (d) 捕まえるか・足す案

- 1: 見つけた側が Rx の形（Rx は終わり tx が残る）は捕まえない．見つけた側が Tx の形は rx.rs:889 が捕まえる（Rx の cut と clock が動かない）．ただし `device_lost` を直接呼ぶ unit テストは見つけた側を渡さないので，実装が渡す形になると `(Rx,0)` を渡して通りうる．→ 1 本足す（下）．
- 2: Rx を落とす形は rx.rs:889（J47 の filter）が捕まえる．Tx を落とす形は捕まえない．→ 1 本足す．
- 3: 捕まえない．`end` を 2 回呼ぶテストは既にある（`rx.rs:770,773`，`rx.rs:879,880`）が，見るのは `rx_stop` の回数や plan の cut で，`items` の `End` の数ではない（planner は 2 つ目を無視する）．e2e の loss テストも `run.finish()` の後始末で stop を通るが，見るのは termination と `mark_lost` だけ（`fake.rs:1856-1859`）．→ 1 本足す．
- 4: Rx 側は J60 の `ur_29_a_loss_takes_effect_before_a_change_at_its_instant`（`rx.rs:966`）が捕まえる．Tx 側は捕まえない．→ 足すテストで両 line の `seq == 0` を見る．
- 5: 今の `device_lost` は見つけた stream を渡さないので，直接呼ぶテストでは区別できない．実装が渡す形にするなら，足すテストで見つけた側を Tx にして Rx の床を assert する．しないなら 1b-4c 送り（受信 2 本以上）．
- 6: 方向の取り違え（Rx が終わらない）は rx.rs:889 が捕まえる．k の取り違えは今は見えない → 1b-4c 送り．
- 7: J58 の行を同じ規則・同じ形で当て直す（挙動は変えない）．

足すテストの案（1 本，追加で，既存の assert は変えない．採否は orchestrator）:

- 名前（案）`ur_29_a_loss_ends_every_stream_once`．置き場は `crates/ezsdr-radio-uhd/src/provider/rx.rs` の tests（`ur_29_a_loss_ends_the_stream_at_its_instant` の隣）．
- 作り: `rig_with_links(vec![link()])`（`rx.rs:769-772` と同形）に `rx(&core, 1)` で rx line（`rx.rs:641-659`），tx line を `Control::new` と同じ形（`control.rs:196-215`）で `(Direction::Tx, 0)` に挿し，tx clock は `control.rs:783`／`tx.rs:992` と同形で登録する．
- 手順: rx に 3 000 サンプル配送（`rx.rs:901` と同形），`time.advance_to` で 2 ms へ，`core.device_lost("test")`；その後 `streams.stop = Some((5 ms, Abort)); streams.end(&core, 5 ms, true, false)`（`mod.rs:494-499` と同形）．
- assert:
  1. 両 line に `Kind::End` がちょうど 1 つ，`e` が loss の instant，`seq == 0`，`abort == false`（`items` を読む．`rx.rs:1116,1142`，`tx.rs:1070,1080` と同じ読み方）．
  2. rx の plan の cut が 3 000（loss の instant より配送済みの床が優先．`rx.rs:895-908` と同じ観測）．
  3. stop の後も両 line の `End` は 1 つのまま，rx の cut も 3 000 のまま（instance 全体の `ended`．`brief.md:25`）．
  4. 実装が `device_lost` に見つけた stream を渡す形にするなら，見つけた側を `(Tx, 0)` にして 2 が成り立つこと（床は終わる line 自身の `delivered`）．
- mutation の行（実装後の文面で．新規は `plan/maintenance/tools/mutations.json` に P 番号で置く（1b-4a の P234…P241 が `:6134-6204`），`test_bin: "lib"`，`filter` は新テスト名）:
  - N1: `end` の `ended` guard を外す — `old` は guard の 3 行（今の `core.rs:127-129` 相当），`new` は空 → 3 が落ちる（stop が 2 つ目の `End` を記帳）．
  - N2: `end` が tx の line に記帳しない — 今の形なら `old` `for dir in [Dir::Rx, Dir::Tx] {`（`core.rs:131`），`new` `for dir in [Dir::Rx] {` → 1 が落ちる．
  - N3（任意）: `lost` の `seq`（`core.rs:130`）を `next_seq()` にする行は J60 と同じ規則なので，行を増やさず，新テストの 1 が J60 の行でも落ちることを記録する（tx line を押さえるのは新テストだけ）．
- e2e の追加は任意で 1 本．今の loss の e2e はすべて片方向（`fake.rs:1853` rx のみ，`fake.rs:1892-1895` tx のみ，`fake.rs:1944-1947` tx のみ，`fake.rs:2848-2852` rx のみ，`fake.rs:3588` tx のみ）なので，「tx の thread が loss を見つけ，rx に未配送がある」を実 thread で 1 本足すならここ．観測は `DEVICE_LOST` 1 回，`mark_lost` の位置，loss 後の `tx_send`/`rx_recv` がないこと（`device.calls()`）．loss 後は uhd-control が booking しない（`control.rs:245-248`）．

「Tx line の `End` を読むテストはない」の根拠（実行した検索そのまま）:

- `grep -rn "Kind::End" crates/ezsdr-radio-uhd` → `core.rs:95,132`（実装）と `rx/differential.rs:179,184,185,189,200,754,768`（model 側）だけ．
- `grep -n "items" crates/ezsdr-radio-uhd/src/provider/tx.rs` → `1070,1080,1081`（refused と seq を読むだけ．`End` は読まない）．
- `grep -n "FakeFault::Lost\|FakeFault::Silence\|RxPanic" crates/ezsdr-radio-uhd/tests/fake.rs` → `896,1472,1852,1865,1943,2004,2431,2849`（Provider 経由の loss は `1852`・`1865`・`1943`・`2849` で，どれも片方向の spec）．
- `grep -n "with_tx(" crates/ezsdr-radio-uhd/tests/fake.rs` → `876,910,1208,3006`（loss を伴うものはない）．

### 関わる他の順序

- 2（`stop` の Drain と Abort）: loss と `Provider::stop` は同じ `end` を通る（`core.rs:388`，`mod.rs:498`）．`ended` を per stream にすると stop が loss の後に 2 つ目の `End` を記帳し，stop の join 順（control → 送信 → 受信，`mod.rs:501-506`）の間に tx がどう終わるかが変わる．
- 3（stream の thread の panic）: panic も同じ `device_lost` を呼ぶ（`mod.rs:240-243`）．panic した thread の stream だけを終える実装だと他が残り，`rejected` の `thread` 名（順序 11）と同時に崩れる．
- 5（ある stream の device が設定を拒否）: 拒否で halt した line にも loss の `End` が 1 つ要る．`end` が halted や channel 0 の line を飛ばすと，その stream の `End` が欠ける．
- 7（受信の cut と delivered の床）: loss の `End` は各受信 line 自身の `delivered` で床を作る（`core.rs:93-98`）．per stream 化の取り違えは，受信が 1 本の今は見えない．
- 9（同じ instant の `cold` と timed の順序）: loss の `seq = 0` が同じ instant の `cold` 変更との順序を決める（J60，`rx.rs:966`）．per line の `seq` にすると tx 側だけ順序が変わる．
- 10（lock の順序と device 呼び出し）: `device_lost` は `streams` の lock の下で `end` と `lost` を済ませ，`mark_lost` は lock を離してから（`core.rs:384-392`）．`end` の中で device を呼ぶ形にすると UR-25（`design/18-uhd-radio.md:244`）の「device の呼び出しの間は line の lock を持たない」に反する．
- 11（thread の名前と `rejected` の `thread`）: panic の記録は今 `uhd-rx`／`uhd-tx`（`mod.rs:240-243`，`mod.rs:474-477`）で，stream ごとの thread でも名前は今のまま（`brief.md:31`）．loss 自体は thread 名を記録しない（変えない）．

## 5. ある stream の device が設定を拒否する（RM-25）：その stream の line だけが halt する．他の方向の stream はそのまま．

規範は `design/07-radio-model.md:260`（拒否された変更はその stream を halt し，次の `cold` 変更か `start_rx` まで segment を持たない．拒否された値は構成の記録として残り，halt 中の stream は `hardware_timed` の channel を持たない）と `design/18-uhd-radio.md:207`（UR-17：device が拒否したら uhd-rx が refusal を book する），`:230`（UR-23：uhd-tx が book し，その clock は origin で終わる），`:242`（UR-24：halt した direction は channel を数えない），`:244`（UR-25：各 stream の計画は 1 本の line，学んだ thread が book する）．今の実装の関門は `crates/ezsdr-radio-uhd/src/provider/rx.rs:286`（uhd-rx の book），`crates/ezsdr-radio-uhd/src/provider/tx.rs:672`（uhd-tx の book），`crates/ezsdr-radio-uhd/src/provider/core.rs:110`（`Streams::refuse` → `:162` `changed` → `:174` `reconcile`），`crates/ezsdr-radio-uhd/src/provider/control.rs:382`（`channels_at` が halt を読む）．この段階はここを `(Direction, k)` の key と stream ごとの型に組み直すだけをする．

### (a) 既存のテスト

- 受信の book と halt：`crates/ezsdr-radio-uhd/src/provider/rx.rs:1100` `ur_25_a_refused_segment_is_booked_by_uhd_rx`（FakeFault::WrongRate が 2 MS/s の apply を 1 回拒否する本番経路．:1105-1106）。
- 受信の差分（VH-8）：`crates/ezsdr-radio-uhd/src/provider/rx/differential.rs:111` `rm_26_the_timeline_against_uhd_rx`（1 000 列のうち refuse する列で，rx line の refused と rejected の数が「device が拒否した回数」と一致する．:356-359）。
- 送信の book と clock の終わり：`crates/ezsdr-radio-uhd/src/provider/tx.rs:1012` `ur_23_a_refused_transmit_change_ends_its_clock_at_its_origin`（:1027，:1029-1030）。
- 送信の book と burst の扱い：`crates/ezsdr-radio-uhd/src/provider/tx.rs:1034` `ur_25_a_burst_on_a_later_clock_survives_an_earlier_switch`（:1066-1071）。
- 制御側の halt の帰結：`crates/ezsdr-radio-uhd/src/provider/control.rs:1105` `ur_24_a_direction_halted_by_a_refusal_has_no_channel`，`:1153` `ur_24_a_halted_direction_fills_no_queue_slot`，`:1202` `ur_24_a_late_update_halted_before_it_can_act_has_no_channel`，`:863` `ur_30_the_recorded_plan_has_no_refused_segment`．
- timeline（`Lines`/`Line` の層）：`crates/ezsdr-radio/tests/timeline.rs:141` `rm_26_the_timeline_cases` の拒否の case（:229-252，:291-294），`:308` `rm_26_generated_sequences_keep_the_timeline_invariants`（5 回に 1 回を拒否．:317-319，:353-355），`:557` `rm_34_a_refused_move_halts_both_lines_until_a_change_or_a_start`，`:833` `rm_34_a_move_is_refused_on_both_lines_whichever_line_names_it`（触れない stream は何も拒否しない．:841-842），`:402` `rm_26_a_refused_booking_leaves_the_line`．
- 近いがこの順序ではない：`crates/ezsdr-radio-uhd/tests/fake.rs:1576` `ur_14_a_device_error_while_booking_names_its_action`（channel 数 0 からの enable の booking で拒否＝順序 6），`crates/ezsdr-radio-uhd/tests/fake.rs:751` `ur_12_a_rate_the_device_does_not_apply_is_refused`（prepare の configure；この段階の対象外）．

### (b) assert と観測

- `rx.rs:1100`：device の `apply rx 0 rate=…` が拒否されると `begin`（:269-293）が `refuse(&core, Dir::Rx, by)`（:286）を呼ぶ．観測は (i) **line の item の `refused` フラグ**（:1116 `lines[Dir::Rx]…items.any(|item| item.seq == 1 && item.refused)`），(ii) **owner の state**（:1114 `rx.stream.is_none()`，:1120 `rx.stream` の ratio が拒否された 2 MS/s = 100），(iii) **device の calls**（:1119 `rx_start 0 <ms(65)>` がちょうど 1 回；拒否された segment は begun と数えず，同じ origin で再開する）．event は assert していない．
- `differential.rs:111`：clocks・blocks・calls・stops を plan と比較（:311-354）に加え，**rx line の refused の数と rejected の数**が `refusable > 0` から 1 と決まる期待値に一致すること（:155，:356-359；refused の収集は :794-797）．
- `tx.rs:1012`：`streams.refuse(&core, Dir::Tx, 1)` の後，**`ClockRegistry` の sample clock 記録**（`ended_at == c1.origin`）を観測する（:1029-1030）．
- `tx.rs:1034`：`do_switch` の拒否で **`tx.clock == None`，`tx.held` が空，`rec.rejected` の tx_burst 行が 1，Tx line の refused が `[1]`**（:1066-1071）；さらに後の switch で後続 clock の burst が残ること（:1072-1076）．
- `control.rs:1105`（:1138-1148）：**device の calls**（`apply <dir> 0 … gain=3` の本数）と **`rec.applied` の行**（`note == "no channel"`，`issued`），`control.config` / `config_at(t)`，`planned()` の segment の settings を観測する．:1153 は queue の深さ（`held.len()`），:1202 は遅れた更新が「それが act する instant」で判定されること（:1219-1222）．:863 は `finish()` が書く **timing の "plan" 行**の rx の segments（(0,200),(ms(300),200)，:877-880＝拒否された segment は plan に無い）．
- timeline（`tests/timeline.rs:141` ほか）：`plan`/`planned()`（:65）の segments と `halted(t)`（:92 → `timeline.rs:349`），`refuse` の戻り値，触れない line が不変であること（:841-842）を観測する．`Lines::refuse` は指名された line だけを触る（`timeline.rs:477-488`）ので，この層の scope はもう固定されている．

### (c) 壊れうる形

- **c1 拒否が別の line に載る**：`rx.rs:286` の `Dir::Rx` と `tx.rs:672` の `Dir::Tx` を `(dir, k)` の key に書き換えるとき，方向か `k` を取り違える（例：rx の拒否を `(Direction::Tx, k)` に）．今は各方向に stream は 1 本だけなので**方向の取り違えだけが観測できる**；`k ≥ 1` の取り違えは 1b-4c（brief 本文）．`Streams.refuse`（`core.rs:110-122`）の `self.lines[dir as usize]` を `Lines::line(key)` に置き換える際も同じ．
- **c2 halt が instance 全体に及ぶ**：`Streams::refuse` が指名された line だけでなく両 line を拒否する，または `end`（`core.rs:126-134`）を通る．`end` は `ended`（`core.rs:69-71`）を立て両 line に `Kind::End` を book し，以後の booking を無効化するので，**他の方向の segment が切り詰められ，その clock が終わり，再開も効かなくなる**（RM-16，`design/07-radio-model.md:260` の「cuts made and clocks ended stand」と逆向きの副作用）．
- **c3 halt の読みが別の line になる**：`channels_at`（`control.rs:382-387`）の `lines[dir]` の halt 判定を他方の line から読む．UR-24 の queue の深さ（`:415-416`）と release（`:449-455`）が同じ読みに依存する．
- **c4 stream ごとの型に移した状態の取り違え**：`configs`（`core.rs:63`），`tx_clocks`（`:66`），`delivered`（`:76`），`idle`（`:78`）を stream ごとの型へ移すとき，拒否の副作用（`changed` `:162-169` → `reconcile` `:174-194`）が**別の stream の clock を終わらせる／終わらせ損ねる**．`version`（`:80`）は instance 全体のまま（brief）．また拒否を book したあと `device_failed(None, …)`（`rx.rs:291`，`tx.rs:675`）が thread・id を変える形もここ．
- **c5 同じ line 側の副作用が消える**：`changed` を呼ばず tx clock を origin で終わらせない（J62 の規則），または `refuse` の `Err` 経路（`core.rs:117-120`）の `RM-26:` の記録が変わる．
- **c6 line の無い stream への book/refuse**：今は `Option` の `None` で false（`core.rs:94`，`:111`）．`timeline::Lines` は無い line で **panic** する（`timeline.rs:633-639`）．拒否の経路は planned segment が在るので line はあるが，`book` 側（`Stop` と `start_rx`；rx link 無しの受信）は false を保つ必要がある（順序 2・6 の担当）．

### (d) 捕まえるか・足す案

- **c1 は捕まる**．`rx.rs:1116` が Rx line を名指しし，`tx.rs:1070-1071` が Tx line を名指しする（どちらの rig も他方の line を持たない：`rx.rs:1106`＋`rx()` `:641-646`，`tx.rs:1041-1050`）．条件：型名の書き換えで**観測点が同じ key（`(Direction::Rx, 0)`／`(Direction::Tx, 0)`）を指し続ける**こと（入れ替えない）．
- **c2 は捕まらない**．`grep -rn "halted\|\.refuse(" crates/ezsdr-radio-uhd/tests/` → 一致 0 件（統合テストに拒否の経路が無い）；provider の rig も 1 line ずつ（`rx.rs:641-646`，`tx.rs:1050`，`differential.rs:756`）；`grep -n "refuse(&core" crates/ezsdr-radio-uhd/src/provider/control.rs crates/ezsdr-radio-uhd/src/provider/rx.rs crates/ezsdr-radio-uhd/src/provider/tx.rs` → control.rs:872,1122,1164,1216（各 rig は 1 方向）と tx.rs:1027．**→ T1/T2/T2b を足す**．
- **c3 は概ね捕まる**．`control.rs:1110` の 4 例のうち「halt していない側」（`:1124` の `(Dir::Rx, false, "start_rx")`＝`Stop` された方向）は apply を要求するので，他方の line を読む実装は落ちる（rig の既定 channel 数は `crates/ezsdr-radio/src/device.rs:217-231`）．ただし「**他方の line が halt していて自分はしていない**」形は無い（4 例はどれも片方だけ live）→ T1 がその形を作る．
- **c4 は捕まらない**．送信側の拒否は同じ stream の clock だけを見ており（`tx.rs:1012`），rx の拒否が tx の clock/plan に及ぶ形を観測するテストが無い（上の 2 つの grep と同じ理由）．→ T1/T2/T2b．
- **c5 は捕まる**（J62＝`mutations.json:948-957`＋`tx.rs:1012`）．**c6 の refuse 側**は c1 の副作用として捕まる；book 側は順序 2・6 の担当．

**足すテスト案（新規関数；既存テストは weak 化しない）**

- **T1（control.rs unit）`ur_24_a_refusal_halts_only_its_own_stream`**：`rig()`（NoLink。rx line は links があるので存在）＋`control(&core, &[Dir::Rx, Dir::Tx])`；`control.rs:1113` と同じく device を open；拒否される側に rate change を `book_cold`（`:1120`）し `refuse`（`:1122`）；**両方向**に `book_timed` gain（rx 3.0, tx 4.0）を ms(200) で（`:1126`）；ms(199) まで進め `release()`（`:1136-1137`）．assert：拒否側は `rec.applied` に "no channel" があり `device.calls()` にその方向の gain apply が無い（`:1138-1140` と同形）；**他方の gain は `issued == true` で `apply <他方> 0 … gain=… at=<effective>` が在る**；**他方の `planned(dir)`・line の items の `refused`・`tx_clocks` の `(origin, ended)` が拒否前と同じ**．rx 側／tx 側の 2 通りを `:1110` の形で回す．
- **T2（rx.rs unit）`ur_25_a_refusal_leaves_the_transmit_stream_alone`**：`rx()`（`:641-658`）に加え `Control::new` と同様に Tx line を 1 本入れる（`tx.rs:1050` の形）；`FakeFault::WrongRate`（`:1105`）で rate change を book し `rx.poll(); rx.receive(…)`（`:1112-1113`）；assert：Rx line の item refused（`:1116` と同じ）**かつ Tx line の items の refused が全 false・`planned(Tx)`・`tx_clocks` が前後同じ**．
- **T2b（tx.rs unit）**：対称（Rx line を 1 本入れて `do_switch` の拒否を起こし，Rx line が変わらないことを assert）．key は最初から `(dir, k)` の形で書いておき，1b-4c で k=1 の列を足せるようにする．
- 統合（fake.rs）に runtime の拒否で両方向を見るテストは今 1 本も無い（`grep -rn "WrongRate" crates/ezsdr-radio-uhd/` → fake.rs:753（prepare），fake.rs:1581（enable の booking），tx.rs:1040，rx.rs:1105，rx/differential.rs:723，device.rs:294/1026；`grep -rn "the device applied" crates/ezsdr-radio-uhd/tests/` → fake.rs:757，:1587 のみ）．規則は T1〜T2b で固定できるので優先は unit；Session で tx の burst が続くことを 1 本見たければ 1b-4b の後でよい．

**mutation の行の案**（`old`/`new` の literal は実装時にコードと同時に書く；AGENTS.md §9 step 2）

- M1（rx.rs）：拒否を book する key を送信 stream の line へ（`(Direction::Rx, k)` → `(Direction::Tx, k)`）．filter `ur_25_a_refused_segment_is_booked_by_uhd_rx`（今の rig に tx line が無いので false か panic になり :1116 が落ちる）と T2．
- M2（tx.rs）：対称．filter `ur_25_a_burst_on_a_later_clock_survives_an_earlier_switch` と T2b．
- M3（core.rs `Streams::refuse`）：指名された line だけでなく instance の全 line を拒否する（または `end` を通る）．filter T2 と T2b（今のテストでは他の line が無いので素通り）．
- M4（control.rs `channels_at`）：line の参照を他の方向へ．filter `ur_24_a_direction_halted_by_a_refusal_has_no_channel` と T1．
- 既存行でこの項目を固定しているもの（`old` が変わるなら**同じ規則・同じ形のまま当て直す**；filter を変えない・弱めない）：J14（`mutations.json:567-576`，timeline なので動かない），J15（`:577-586`，rx.rs），J16（`:587-596`，tx.rs），J25（`:647-656`，rx.rs），J62（`:948-957`，core.rs の `changed`），J64（`:968-977`，control.rs），J65（`:978-987`，timeline），J68（`:1008-1017`），J69（`:1018-1027`），J70（`:1028-1037`，timeline），J71（`:1038-1047`）．
- **1b-4c に送る（今は観測できない）**：同じ方向に stream が 1 本しか無いので，(i) stream `k ≥ 1` の拒否が stream 0 の line に載る（逆も），(ii) halt の読みが別の `k` を見る，(iii) 同じ方向の他の stream が halt する／続く，(iv) `configs`・`delivered`・`idle`・`tx_clocks` の `k` 取り違え．T1/T2/T2b を `(dir, k)` で書いておけば拡張だけで済む．

### 関わる他の順序

- 1（`start`）：line は thread を起こす前に挿入されるので，拒否の book は line の存在を前提にできる；逆に line の無い stream への `book`/`refuse` は false のままでなければならず，`Lines` 直呼びの panic（`timeline.rs:633-639`）に化けると thread が死んで順序 3 の device loss になる．
- 2（`stop`: Drain と Abort）：`Provider::stop` は両 line に `End` を 1 回だけ book してから control→tx→rx の順で join する（`mod.rs:494-506`）；拒否の book と同時でも，両者が同じ `Lines`・同じ lock の下で行われないと End と refused が食い違い，join 済みの thread が後から書く形になる．
- 3（thread の panic）：panic は**instance 全体**を loss として終わらせ，`rejected` にその thread の名前を残す（UR-16，`design/18-uhd-radio.md:200`）；拒否は**1 本の line**だけを止める．組み直しでどちらかが他方の scope を借りると `ur_16_a_panicking_thread_is_a_lost_device`（`tests/fake.rs`）と本項目のテストが食い違う．
- 4（device loss）：`ended` は instance 全体で損失は全 stream を 1 回だけ終わらせる（`core.rs:126-134`）；拒否が `ended` を立てる／損失が 1 本だけ終わらせる取り違えは，「全 stream」と「その line だけ」の対で最も危険（順序 4 の `ur_29_*` と本項目の T1/T2 が両側から押さえる）．
- 6（stream のない方向の enable と 0 への disable）：enable の構成が拒否される場合は uhd-control が booking 時に `device_failed(Some(id), …)` で拒否し何も halt しない（`control.rs:548-567`，`tests/fake.rs:1576`）；`idle`/`configs` を stream ごとに移すとき direction を取り違えると，他方の enable が構成を飛ばす／二重に構成される．
- 7（受信の cut と `delivered` の床）：拒否は走っている segment をその instant で切る（RM-25）；`delivered` を stream ごとの型にする際，拒否や `End` の床（`core.rs:95-98`）が別 stream の `delivered` を読むと RM-16 の床が別 segment に付く（この項目自身の拒否経路は `delivered` を使わない）．
- 8（送信の clock の登録と終わり，`version` による再読み込み）：拒否された送信変更は自分の clock を origin で終わらせ（J62），`version` は instance 全体のまま；`changed`/`reconcile` が stream ごとの `tx_clocks` を取り違えると，別 stream の clock を終わらせる／終わらせ損ねる（T2b と `tx.rs:1012` が見る）．
- 9（同じ instant の `cold` と timed，UC-2）：halt は instant `e` から効き，その instant の timed 更新が channel を数えるかどうかを決める（`control.rs:382-387`，`:415-416`）；床の順序は instance 全体の `seq`（brief）なので，組み直しで `seq` を stream ごとにすると同じ instant の `cold` と timed の順序が変わる．
- 10（lock の順序，device の呼び出しの間に line の lock を持たない）：拒否は device の呼び出し（`configure`）が返った後に `streams` の下で book される（`rx.rs:286`，`tx.rs:672`）；stream ごとの型に移す際に新しい lock を足したり，`device_failed`（event と `rec`）を line の lock 保持中に呼ぶと，`updates`→`streams`→`rec` の順序（`core.rs:52-56`）が崩れ uhd-control/uhd-tx と交差する．
- 11（thread の名前と Manifest の `rejected` の `thread`）：拒否の記録は thread の名前を持たず（`device_failed(None, …)` → `command_rejected(None, …)`，`core.rs:371-378`），thread 名は `uhd-rx`・`uhd-tx` のまま（brief）；`rejected` の行や `COMMAND_REJECTED` の payload を組み直しで変えると `ur_16_*` と差分の counts（`differential.rs:356-359`）が落ちる．

## 6. stream のない方向の enable（channel 数 0 → n．`configs`・`idle`・UR-25）と，0 への disable（`tx_open(0, &[])` の経路）

### (a) 既存のテスト

**enable（0 → n）**
- `crates/ezsdr-radio-uhd/tests/fake.rs:1516` `ur_25_rx_channels_from_zero_starts_at_its_instant`（受信を 0 → 1）
- `crates/ezsdr-radio-uhd/tests/fake.rs:1535` `ur_25_a_receive_enable_counts_its_start_lead_from_the_end_of_its_configuration`（受信，apply が 80 ms かかる場合）
- `crates/ezsdr-radio-uhd/tests/fake.rs:1576` `ur_14_a_device_error_while_booking_names_its_action`（enable の configure が device に拒否される経路）
- `crates/ezsdr-radio-uhd/tests/fake.rs:822` `ur_14_actions_are_finished_before_the_next_recv`（送信の enable を booking 側で configure する，apply 20 ms）
- `crates/ezsdr-radio-uhd/tests/fake.rs:1500` `ur_25_tx_channels_from_zero_transmits_the_next_burst_on_time`（送信を 0 → 1 してから burst）
- `crates/ezsdr-radio-uhd/tests/fake.rs:1593` `ur_25_enabling_tx_applies_the_configuration`（enable が設定全体を untimed に適用する）
- 前提の helper：`crates/ezsdr-radio-uhd/tests/fake.rs:1024` `tx_session`・`fake.rs:1035` `tx_session_on`（送信の smoke テストはほぼ全部，この enable を最初に通る）
- 反対側の対照（enable ではない cold 変更は owner が cut で configure する）：`crates/ezsdr-radio-uhd/tests/fake.rs:1389` `ur_25_the_switch_applies_the_configuration_in_effect_at_e2`

**disable（n → 0）と空の channel 列**
- booking 側：`crates/ezsdr-radio-uhd/src/provider/control.rs:1083` `ur_24_a_timed_update_acts_on_the_configuration_in_force_at_its_instant`（`control.rs:1090` で `radio.tx.channels` → 0 を booking している），`control.rs:996` `ur_25_cold_deadline_overflow_is_rejected_without_panic`（`control.rs:1023-1028` が disable で，clock が cut で終わることと plan の segment 数を見る），`control.rs:916` `ur_26_a_change_booked_after_the_stop_instant_ends_its_clock`
- 差分（生成列に `Change::Channels(0)` が入る：`crates/ezsdr-radio/tests/generator/mod.rs:256`）：`crates/ezsdr-radio-uhd/src/provider/control/differential.rs:84` `rm_26_the_timeline_against_uhd_control`，`crates/ezsdr-radio-uhd/src/provider/rx/differential.rs:110` `rm_26_the_timeline_against_uhd_rx`
- device 境界（空の列 = channel 0）：`crates/ezsdr-radio-uhd/tests/fake.rs:2266` `ur_33_an_empty_channel_list_is_channel_zero`（fake），`crates/ezsdr-radio-uhd/src/uhd.rs:1005-1017`（`UhdDevice`。feature `uhd` のテストは実機以外に無い）
- 実機（`#[ignore]`）：`crates/ezsdr-radio-uhd/tests/hardware.rs:216`・`tests/hardware.rs:912`（送信の Session で `radio.rx.channels` → 0）
- Kernel 側の相当（MockRadio）：`crates/ezsdr-acceptance/tests/v58.rs:477` `kc_24_a_burst_after_the_transmit_clock_ends_is_refused_at_admission`（clock が終わった後の burst は admission で拒否）

`grep -rn 'Value::from(0)' crates/ezsdr-radio-uhd/tests/ crates/ezsdr-radio-uhd/src/` の結果は `hardware.rs:216`・`hardware.rs:912`（実機，ignore）と `control.rs:1025`・`control.rs:1090`（unit の booking）だけで，fake の smoke に 0 への disable は無い（`grep -rn 'Value::from(0)' crates/ezsdr-radio-uhd/tests/fake.rs` は 0 件）．

### (b) assert と観測

- `ur_25_rx_channels_from_zero_starts_at_its_instant`（`fake.rs:1516-1532`）：`Direct` で受信側に link だけあり channel 0 の状態から `radio.rx.channels` を 1 に上げる。device の call 列から最後の `rx_start …` を拾い，`"rx_start 0 "` の長さで origin を切り出して（`fake.rs:1524-1526`）`origin >= at && origin < at + 200`（1 MS/s の 1 sample）を assert。受信の SampleClock record が origin 1 本だけであること（`fake.rs:1528-1529`）と `radio.COMMAND_REJECTED` が 0 であること（`fake.rs:1530`）。観測は device の calls，`ClockRegistry`，event collector。
- `ur_25_a_receive_enable_counts_its_start_lead_from_the_end_of_its_configuration`（`fake.rs:1535-1555`）：Manifest の `timing` section の `cold_change` row の `e`・`ready`（book 側が `control.rs:575` で記帳）と `rx_segment` row の `origin` を読み，`ready - e >= 80 ms`（`fake.rs:1552`，enable の準備完了 = 設定の終わり，F39 の規則）と `origin == lattice(max(e, ready + lead))`（`fake.rs:1554`，`lead` は `envelope` section の `rx_start_lead_ns`）を assert。観測は Manifest の `timing`・`envelope`。
- `ur_14_a_device_error_while_booking_names_its_action`（`fake.rs:1576-1590`）：enable の configure に `WrongRate` fault を仕込み，`radio.COMMAND_REJECTED` がちょうど 1 つ，reason が `"UR-12: the device applied"` で始まり `payload["action"] == 1` であることを assert（`fake.rs:1586-1588`）。観測は events。booking 中（`control.rs:565-567` の `device_failed(Some(id), …)`）の失敗が segment を作らないこと。
- `ur_14_actions_are_finished_before_the_next_recv`（`fake.rs:822-838`）：送信の enable を `apply_delay` 20 ms で行い，`submit` が返った時点で `apply tx` の call が既にあること（`fake.rs:833`）と，`ended_at` の無い `usrp/tx` の SampleClock があること（`fake.rs:835`）を assert。つまり enable の configure は uhd-control の booking 内で行われた（owner の switch 待ちではない）ことを観測する。
- `ur_25_tx_channels_from_zero_transmits_the_next_burst_on_time`（`fake.rs:1500-1513`）：5 回繰り返し，`tx_session`（0 → 1 の enable）後に `start_repeat` の burst を admit し，`TIME_ERROR` が 0（`fake.rs:1507`）で，`apply tx 0 rate=1000000` の位置が `tx_send … sob=true` より前（`fake.rs:1509-1511`）であることを assert。観測は device の calls と Manifest の events。
- `ur_25_enabling_tx_applies_the_configuration`（`fake.rs:1593-1621`）：enable の前に設定した tx の freq・gain と rx の freq が，enable の untimed な 1 回の `apply tx 0 rate=1000000 freq=2400000000 gain=5 antenna=TX/RX at=-` に載ること（`fake.rs:1606`）を assert。続けて repeat を送り，loopback の capture が波形と一致する（`fake.rs:1617-1620`）。
- `ur_25_the_switch_applies_the_configuration_in_effect_at_e2`（`fake.rs:1389-1401`）：走っている受信 stream への rate 変更（enable ではない）で，保持された timed 値（2.1 GHz）を含む `apply rx 0 rate=2000000 freq=2100000000 … at=-` が存在することを assert。`from_none` が「stream のない方向」だけに立つことの対照。
- `ur_24_a_timed_update_acts_on_the_configuration_in_force_at_its_instant`（`control.rs:1083-1102`）：100 ms で `radio.tx.channels` → 0 を booking し，50/50/150 ms の timed gain を release して，`applied` row が（issued, issued, no channel）となり `apply tx` が 2 回であることを assert（`control.rs:1098-1101`）。disable 中（`channels_at == 0`）の timed 値は記録だけになる，という規則。
- `ur_25_cold_deadline_overflow_is_rejected_without_panic`（`control.rs:996-1029`）：`control.rs:1025` の disable で，`sample_clock_records()[0].ended_at == Some(aligned_max)`（`control.rs:1026`）と plan が 1 segment（`control.rs:1028`）を assert。disable は新しい clock を登録せず，既存の tx clock を cut で終えること。
- `rm_26_the_timeline_against_uhd_control`（`control/differential.rs:84-116`）：1000 seed の生成列（両方向，`Change::Channels(0..2)`）を実 `Control` に booking し，理想化した owner で `rx_clocks`・`tx_clocks`（origin と ended）・`timed`・`rejected` を `ezsdr_radio::timeline::plan` と比較。disable の clock の終わりと設定の記帳はこの比較に入っている。
- `rm_26_the_timeline_against_uhd_rx`（`rx/differential.rs:110`）：同じ列を実 uhd-rx で運び，block・stop・clock・拒否を plan と比較。受信の 0 channel 化（disable）と，その後の enable は seed 次第で必ず出る。
- `ur_33_an_empty_channel_list_is_channel_zero`（`fake.rs:2266-2287`）：`rx_open(0, &[])`・`tx_open(0, &[])` が channel 0 の streamer として 1 channel を読み書きし，channel 0 を他の streamer が取れないことを assert。観測は device の振る舞いそのもの（`src/device.rs:1047-1051`・`1349-1356` が検査の前に空列を `[0]` に読み替える）。
- `crates/ezsdr-acceptance/tests/v58.rs:477`（MockRadio の Kernel 経路）：`radio.tx.channels` を 1 → 0 にした後，burst の submit が `SC-23: … has no running transmit SampleClock` で Rejected になること（`v58.rs:486-491`）。UHD 用の同じテストは無い（下の (d) の T1）。

### (c) 壊れうる形

- **c1 `configs` の stream 取り違え**：記帳（`control.rs:572` の `streams.configs[dir as usize].insert(seq, (segment_config, from_none))`）と読み出し（`core.rs:151` の `self.configs[dir as usize].get(&by)`）が，per-stream 化のあと別の stream・別の方向を指す。`Planned.settings` が `None` になり owner の `configure` が `"UR-25: the segment has no configuration"`（`control.rs:131-136`）で拒否されるか，逆に別 command の設定を拾う。`configs` の prune（`core.rs:144`）が stream をまたぐと，終わった segment の設定が残る。
- **c2 `idle` の取り違え**：`from_none`（`control.rs:548`）が `streams.idle[(dir,k)]` と `lines[(dir,k)].plan` を突き合わせる判定。初期化（`control.rs:220`）・owner の更新（`rx.rs:273`，`tx.rs:171`）のどれか1つでも別キーに落ちると，(i) stream のない方向の enable で uhd-control が open/configure しなくなる（owner が switch でやる），(ii) 逆に走っている stream の変更を booking で configure してしまう。
- **c3 `from_none` の `ready = self.core.now();`（`control.rs:568`）の書き換え**：enable の準備完了が設定の終わりでなく booking になる（F39）。
- **c4 disable の clock の終わり**：`reconcile`（`core.rs:176-186`，`core.rs:182` の `segment.cut` で `clocks.end`）と `tx_clocks` の per-stream 化。disable は segment を作らないので clock を登録せず既存を切る，という規則。
- **c5 device 呼び出しの番号**：`device の呼び出しは 0 ではなく k`（`control.rs:554-557`，`rx.rs:281`，`tx.rs:657`，`mod.rs:405-409`）。今の key は方向ごとに stream 0 だけなので，*一貫した* k≠0 は観測できない（tx は call 列の stream 番号を見るテストが無い）。rx は `rx_start 0 `（`fake.rs:1491-1492`・`fake.rs:1525`）と `rx_stop 0 now`（`fake.rs:1759`）が stream 番号を文字列で見ている。決め打ちの `0` を残す誤りはこの段階では観測不能（1b-4c の複数 stream で出る）。
- **c6 disable の owner 側（`do_switch` の `to = None` 経路）**：0 channel の cold 変更は plan に segment を作らない（`crates/ezsdr-radio/src/timeline.rs:848` の `if self.on && !next.channels.is_empty()`，初期状態は `timeline.rs:805`，不変条件は `crates/ezsdr-radio/tests/timeline.rs:1317` の `assert!(!segment.config.channels.is_empty(), …)`）。uhd-tx の reopen は `to` = plan の次 segment（`tx.rs:179`）なので disable は `Switch { cut, to: None }` で来て reopen ごと飛ばされる（`tx.rs:655`）。この経路（clock の終わり，open burst の終了，held burst の `cancelled by a cold change`：`tx.rs:645-649`，その後 `read` の prune と `idle`）を通す *provider の* テストが fake の smoke に 1 本も無い。
- **c7 `tx_open(0, &[])` が実際に呼ばれるように変わる**：現在の provider に空列を渡す呼び出しは無い。根拠：`grep -rn 'to.channels()' crates/ezsdr-radio-uhd/src/` は `tx.rs:656`・`657`・`663` のみで，`grep -rn 'tx_open(0, &\[\])' crates/ezsdr-radio-uhd/src/` は 0 件（exit 1）。空列を渡す caller は plan に 0 channel segment が要るがそれは作られない（上記 c6 の根拠）。`design/18-uhd-radio.md:569` と `plan/phase8/33-channels-and-streams.md:266` は「0 channel に切り替える caller が空の列を開く」と書くが，HEAD の code は開かない（食い違いは report で orchestrator に渡す。この段階は挙動を変えない）。

### (d) 捕まえるか・足す案

| (c) | 既存テストが捕まえるか | 足すもの |
|---|---|---|
| c1 | 捕まえる：enable の 2 本（`fake.rs:1516`・`1535`）と tx の 2 本（`fake.rs:1500`・`1593`），`tx_session` 系全部，差分 layer 3。逆方向（古い設定を拾う）は `fake.rs:1389`・`1535` が捕まえる | 不要。`configs` の prune は J51 の当て直しで足りる（下記） |
| c2 | 捕まえる：enable を取り逃す側は `fake.rs:833`（tx, apply が submit 前に無い）と `fake.rs:1552`（rx, `ready` の床）。逆（booking で configure しすぎる）は `fake.rs:1399`（timed 値が届かない） | 行の案：`control.rs:548` の `from_none` を壊す 2 行。(R1) `let from_none = false;`（kill: `ur_25_a_receive_enable_counts_its_start_lead_from_the_end_of_its_configuration`，`ur_14_actions_are_finished_before_the_next_recv` も）。(R2) `let from_none = channels > 0;`（kill: `ur_25_the_switch_applies_the_configuration_in_effect_at_e2`） |
| c3 | 捕まえる：`fake.rs:1535`（F39 の行そのもの） | 行の当て直しの見込み：**F39**（`old` = `ready = self.core.now();`）と **F40**（`control.rs:200` の lead）。どちらも文面が同じならそのまま，per-stream 化で行が変われば同じ規則・同じ形に当て直す |
| c4 | 捕まえる：`control.rs:1026`（unit）と差分 layer 2。行の当て直しの見込み：**J21**（`core.rs:187` の `tx_clocks.iter_mut().skip(plan.len())`）は `reconcile` の書き換えで必ず当て直し。`core.rs:182` の cut 終了の行は今は行が無い（足すなら c4 の形） | 追加不要（unit＋差分で足りる） |
| c5 | 半分：rx は call 文字列（`fake.rs:1491-1492`・`1525`・`1759`）が捕まえる。tx は捕まえない。決め打ちの `0` を残す誤りはこの段階では観測不能 | 1b-4c へ送る（key で k が決まる段階で，tx の call 文字列 assert を足す） |
| c6 | **捕まえない**（provider の disable を通すテストが無い。`grep -rn 'channels", Value::from(0)' crates/ezsdr-radio-uhd/` は 0 件） | T1（下）＋行の案（下） |
| c7 | 捕まえるのは device 境界だけ（`fake.rs:2266`）。provider 経路は存在しないので，behavior を変えない限り観測点も無い | T1 に「空列の `tx_open` が無いこと」の assert を 1 つ入れる（今の挙動の pin。`tx_open` の行は mutation のリストに無い：`grep -rn 'tx_open' plan/maintenance/tools/mutations.json plan/phase7/tools/mutations.json` は 0 件） |

**T1（足すテストの案）** `crates/ezsdr-radio-uhd/tests/fake.rs` に `ur_25_tx_channels_to_zero_ends_the_clock_and_a_re_enable_starts_a_new_one`：
1. `tx_session(FakeConfig::default())`（0 → 1 の enable）で repeat の burst を送り，少し待つ。
2. `set("radio.tx.channels", Value::from(0))` を submit（admitted）→ 少し待つ → その間に burst を submit すると admission で拒否されること（`SC-23: … has no running transmit SampleClock`。MockRadio の `v58.rs:477` と同じ規則が Kernel 側で効く）。
3. `set("radio.tx.channels", Value::from(1))` を submit（admitted）→ 待つ → burst を送って finish。
4. assert：(i) `usrp/tx` の SampleClock が 2 本で，1 本目が Some(ended) かつその end が Manifest の `timing` plan の tx 1 本目の `end` と一致（UR-30，`core.rs:198-206`），(ii) 2 本目の origin が 1 本目の end + `tx_start_lead_ns` 以後（RM-25 の `previous` の床，`timeline.rs:919-921`），(iii) device の calls に `tx_open 0 []` が無い，(iv) 2 本目の burst が記録され `TIME_ERROR`/`COMMAND_REJECTED` が期待分以外に無い。
- 行の案 (R3)：`tx.rs:180` の `None => self.plan.get(self.at).cloned()…` → `None => self.plan.get(self.at + 1).cloned()…`（「clock のない owner が，自分の位置の segment でなく次を取る」）。kill: 上の (iii)/(iv) は `ur_25_tx_channels_from_zero_transmits_the_next_burst_on_time`（`tx_send` が来ず `unwrap` が panic）で十分。
- 行の案 (R4)：`tx.rs:651-653` の `if passed.is_some() { self.at += 1; }` を `if false { self.at += 1; }`（「switch で抜けた segment の次へ進まない」= disable の後も uhd-tx が古い segment に残る）。kill: T1 の (i)/(ii)（再 enable が新しい clock で始まらない）。`self.at += 1;` は `tx.rs:674` にもあるので，`old` は前後 2 行を含める。
**T2（足すテストの案）** `ur_25_an_enable_from_no_stream_configures_once`：enable（0 → 1）の後，`tx_open 0 [0]` が 1 回，`apply tx 0 rate=…` が 1 回であることを数え，続けて burst が on time であること。行の案 (R5)：`control.rs:572` の `insert(seq, (segment_config, from_none))` → `insert(seq, (segment_config, false))`（`configured` を反転 → owner が二度 configure する）。kill: T2（今の test では確定で捕まらない）。

**この順序の行の当て直し候補（item 12 の一覧にも挙げる）**：F39・F40（`control.rs:568`・`200`），**U22**（`control.rs:600` の `streams.tx_clocks.last()…zip(`，per-stream 化で必ず変わる），**J64**（`control.rs:383` の `lines[dir as usize]…halted(t)`），**J51**（`core.rs:144` の `configs[…].retain`），J21（`core.rs:187`），U41（`control.rs:574-575` の book と timing row）。

### 関わる他の順序

- **1（start）**：line・`tx_clocks`・`idle` の初期値は `Control::new`（`control.rs:216-221`）が作る。ここが per-stream 化で狂うと最初の enable が `from_none` を外し，T1 以前に `fake.rs:833`・`1535` が落ちる。受信の link が無いとき line が無いこと（`control.rs:208-211`）と，line の無い受信 stream への変更が値の記録だけになること（`control.rs:539-543`）が重なる。
- **2（stop）**：enable/disable の booking と `Stop`/`Provider::stop` が同 instant に来ると，`End` の item が後続命令を無効化する（`timeline.rs:824-830`）。disable の held burst の cancel（`tx.rs:646-649`）と `Provider::stop` の cancel（`tx.rs:761-772`）は同じ held を持つので，join の順（control → 送信 → 受信，`mod.rs:501-506`）を変えると cancel が間に合わない。
- **3（panic）・4（loss）**：enable の open/configure の最中に loss を検出すると，`device_lost` が両 stream の End を記帳してから `mark_lost` する（`core.rs:383-400`）。per-stream 化で streamer を握る thread と `cleanup` の detached 判定（`mod.rs:520-525`）の対応が崩れると streamer を二重解放/leak する。
- **5（RM-25 の拒否）**：`from_none` の configure 拒否は Action 名つきで booking 時に拒否（`control.rs:565-567`，`fake.rs:1576`・J194）。owner 側の拒否は owner が refuse を記帳して halt（`rx.rs:286`，`tx.rs:672`）。per-stream 化で `refuse` の seq 対応を誤ると halt が別 stream に付く（今は 1 本なので観測不能，1b-4c へ）。
- **7（delivered の床）**：disable の cut は `delivered`（`rx.rs:153`・`307`・`330` と `core.rs:92-98`）を通じて RM-16 の床になる。`delivered` を stream ごとにしたとき，disable 直後の `finish` と `Streams::book` の End が同じ stream を指すこと。
- **8（送信 clock と version）**：enable は booking で clock を登録し（`core.rs:177-180`），disable は登録せず既存を cut で終える（`core.rs:182-185`）。`version` は instance 全体のまま（`core.rs:80`・`tx.rs:165-171`）で，ここを stream ごとにすると uhd-tx の読み直しの条件が変わる。
- **9（同 instant の cold と timed）**：enable/disable の effective instant と held の床は instance 全体の `seq`（`core.rs:68`，`control.rs:422`・`512`）を使う。seq を stream ごとにすると同 instant の順序と RM-25 の床が変わる（今は 1 本で差は出ないが，書き換えないこと）。
- **10（lock の順序）**：`from_none` は `streams` を drop してから device を呼び，戻って再 lock する（`control.rs:552`・`569`）。UR-25 の「device の呼び出しの間は line の lock を持たない」が，per-stream の型を触る間の device 呼び出しで崩れると manual clock が止まり watchdog が落とす。順序は `updates` → `streams` → `rec`（`core.rs:55-56`）。
- **11（thread の名前）**：enable/disable は thread を増やさない（D15 の per-stream thread は今の数と同じ 1+1）。名前を変えると `rejected` の `thread` を見るテスト（`fake.rs:902`・`924`）が落ちるので，1b-4c まで `uhd-rx`・`uhd-tx` のままにする。

## 7. 受信の cut と delivered の床（RM-16．`delivered` が stream ごとになる）

### (a) 既存のテスト

床は2か所にある．(1) 共有の `Streams.delivered`（`crates/ezsdr-radio-uhd/src/provider/core.rs:74-76`）が `book` で `Item.delivered` になり，plan の `Segment.cut` を押し上げる経路（`core.rs:92-98`，`crates/ezsdr-radio/src/timeline.rs:879-884`）．(2) uhd-rx の局所 `Stream::expected` が plan の cut を下限で押し上げる経路（`crates/ezsdr-radio-uhd/src/provider/rx.rs:108-116`）．規則は RM-16（`design/07-radio-model.md:181`）と UR-17 の床の段落（`design/18-uhd-radio.md:208`）．

- `ur_17_a_cut_is_never_before_what_was_delivered` — `rx.rs:848`．3 000 配達のあと，plan の cut 1 000／plan から消えた segment／空の plan のいずれでも，stream と clock が 3 000 で終わる．(2) の `.max(self.expected)`（`rx.rs:111`）と「plan にない segment は 0 で切る」（`rx.rs:110`）を固定する（J13，J23）．
- `ur_26_a_stopped_stream_keeps_its_end` — `rx.rs:800`．4ケース．(Abort, 300, 300, true) は 1 ms の `Stop` が plan を 1 000 で切った後でも abort が 300 で終えること，(Abort, 300, 300, false) は `Stop` なしで abort が 300 で即終わることを固定する．後者は abort の局所床（`rx.rs:246-261` の `cut_at(clock.instant(expected))`）を，早期終了（`rx.rs:190-203` の `done`）と併せて見る．
- `ur_29_a_loss_ends_the_stream_at_its_instant` — `rx.rs:889`．損失が 5 ms（3 000 配達）なら 5 000，損失が 2 ms でも 3 000．共有の床が `Core::device_lost`（`core.rs:383-389`）→ `Streams::end`（`core.rs:126-134`）→ `book` を通ること，値が block の最後の次（`k + len`）であることを，clock（`rx.rs:907`）と plan の cut（`rx.rs:908`）の両方で見る．
- `ur_29_a_loss_takes_effect_before_a_change_at_its_instant` — `rx.rs:966`．12 000 配達，10 ms の `cold` 変更，同じ instant の損失（seq 0）．plan は 1 segment，cut 12 000．`book` の guard が「その item の瞬間に効いている item」で segment を選ぶこと（`core.rs:96` の `line.made(item.e, item.seq)`）を固定する．standing plan で選ぶと変更の cut 10 000 が見えて床が落ちる．
- `ur_26_an_end_takes_delivered_only_from_the_segment_it_cuts` — `rx.rs:983`．9 000 配達，10 ms の変更，12 ms の abort．plan は 1 segment，cut 10 000．`(origin, config)` の guard（`core.rs:97`）が，切っていない次の segment に床を付けないことを固定する（J54）．
- `ur_26_an_orderly_stop_cuts_at_its_instant` — `provider/control.rs:900`．`delivered` を直接 6 000 に置き（`control.rs:909`），5 ms の orderly な End の plan cut が 6 000 になること．
- `ur_26_uhd_rx_follows_the_stop_it_reads` — `rx.rs:870`．300 配達，5 ms と 7 ms の End．公開は 5 000，clock は 5 000（早い方だけ）．
- `ur_29_a_loss_found_after_uhd_rx_read_the_plan_ends_the_stream` — `rx.rs:913`，`ur_29_a_silent_stream_ends_at_a_cut_booked_during_its_wait` — `rx.rs:935`．待ちの間に別スレッドが book した床が，`finish` の plan 読み直し（`rx.rs:321-333`）で効くこと．
- `ur_26_uhd_rx_aborts_when_its_provider_is_gone` — `rx.rs:1028`．300 配達で abort，clock 300，untimed stop 1 回．
- `ur_26_an_abort_stops_a_queued_start_at_shutdown` — `rx.rs:1001`．開始前の segment はサンプルを持たず床は 0，cut は origin のまま；stop は shutdown で1回（G60）．
- `rm_26_the_timeline_against_uhd_rx` — `provider/rx/differential.rs:111`．期待側が床を独立に計算し（`differential.rs:226-260`），clock（`differential.rs:288-313`），block，呼び出しを比べる．1 000 seed で最も強い．
- `ur_26_an_abort_publishes_nothing_after_the_stop_instant` — `tests/fake.rs:1678`．abort の cut が stop の instant ではなく「まだ配達していない最初のサンプル」であること（`end < stop`，`fake.rs:1701`；stop 1 回，`fake.rs:1704`）．
- `ur_25_a_receive_session_follows_its_recorded_plan` — `tests/fake.rs:1711`．各 clock の終わりが plan の cut 以上・次の origin 未満（`fake.rs:1752-1756`）．床で cut が押し上がった場合を含む唯一の end-to-end．

見つからないもの（実行した検索そのまま）:

- 受信 stream 2 本で床が混ざることを見るテストは無い: `grep -rn "rx/s1\|rx_open(1\|rx_open(k" crates/ezsdr-radio-uhd/` → 当たるのは device レベルの `tests/fake.rs:2126,2240,2279` だけで，`UhdRadio` を通るものは無い．
- `delivered` を Provider の外から直接置くテストは `control.rs:909` だけ: `grep -rn "\.delivered" crates/ezsdr-radio-uhd/src` → `core.rs:75,93,97`，`rx.rs:153,307,330,454,462`，`control.rs:909`．
- `Stop` に床を付けないこと（plan の Stop cut が配達数で押し上がらないこと）を主張するテストは無い: `grep -rn "Kind::Stop" crates/ezsdr-radio-uhd/src crates/ezsdr-radio-uhd/tests` → `provider/control.rs:305`，`rx.rs:818,1133` だけ．

### (b) assert と観測

- **clock**（最も多い）: `core.clocks.sample_clock_records()` を `clocks()` ヘルパ（`rx.rs:696-698`）で `(origin, ended_at)` にして比べる（`rx.rs:841`，`rx.rs:865`，`rx.rs:885`，`rx.rs:907`，`rx.rs:961`，`rx.rs:1024`，`rx.rs:1039`）．サンプル番号はその clock の `instant()` で root tick に直してある．
- **plan**: `streams.planned(Dir::Rx)` の `segment.cut`（`rx.rs:908`，`rx.rs:978`，`rx.rs:996`，`control.rs:911`）．`Streams::record()`（`core.rs:197-207`）が Manifest の `timing` の `plan` 行になるので，plan の cut は Manifest でも観測できる（`tests/fake.rs:1736-1742`）．
- **block**: ホスト link の `receive()` を `received()`（`rx.rs:687-692`）で集め，長さの和（`rx.rs:838`）や `(first, len)` を比べる（`rx.rs:762`，`rx.rs:1094`）．
- **device の calls**: `device.calls()` の `rx_stop `／`rx_start ` の前置きと回数（`rx.rs:775`，`rx.rs:791`，`rx.rs:842`，`rx.rs:1019`，`rx.rs:1118`）．
- **applied**: `rec.applied` の `key == "rx_stop"` の行と `at`（`rx.rs:776`，`rx.rs:793-795`，`rx.rs:843`；Manifest では `tests/fake.rs:1745`）．
- **イベント**: 床そのものは event を出さない．`COMMAND_REJECTED`／`DEVICE_LOST` は別の規則（順序3・4）．
- **差分**: `rm_26_the_timeline_against_uhd_rx` は，各 block の「instant + 実測 latency が booking より前」の最後を配達数とし（`differential.rs:231-236`），`floored = cut.max(delivered)`（`differential.rs:257`）で期待 plan を作り，clock と block を比べる．uhd-rx の内部は見ない．

### (c) 壊れうる形

- **c1. 床だけ instance 全体（方向ごと）に残す**（`lines` は per stream，`delivered` は `[Option<…>; 2]` のまま）: 受信 stream が2本になると片方の配達数が他方の End を切る．今は各方向 k=0 だけなので**差が出ない**．
- **c2. `delivered` を uhd-rx の局所状態（`Rx`／`Stream`）へ移し，共有の `Streams` から消す**: `Provider::stop`（`mod.rs:491-500`）と `Core::device_lost`（`core.rs:383-389`）は別スレッドで End を book するので床が消える．`Stream` の `expected` だけでは `book` の瞬間の値にならない（UR-17 は「a booking reads it」と書く，`design/18-uhd-radio.md:208`）．
- **c3. 床の値の作り方を取り違える**: block の先頭 `k` を書く／stream 番号 `k` を書く．後者は k=0 で無害（c1 と同じく 1b-4c）．
- **c4. `(origin, config)` の guard を落とす，または stream 番号だけで済ませる**: 切っていない次の segment に前の segment の配達数が付く（J54 の形）．
- **c5. guard を standing plan で評価する**（`line.made(e, seq)` をやめ，`line.plan` の segment を見る）: 同じ instant に先に届いた `cold` 変更の cut が見えて床が落ちる．
- **c6. 床の `config` を「line の最新設定」にする**（実行中の segment の `config` でなく）: 将来 instant の変更が line にあると guard が外れる．
- **c7. 床を `Kind::End` 以外（`Stop`・`Cold`）にも付ける**: `Stop` は `now` のサンプルが床以上なので通常は不変，`Cold` は将来 instant なので不変．差が出るのは uhd-rx が `now` より先まで配達した unit rig の場合だけで，規則ではない（`Item.delivered` の doc は `Stop` も許す，`timeline.rs:146-150`）．
- **c8. `delivered` の書き込みを plan の読みと同じ lock から出す**: `samples` は cut と配達数を1つの `lock` で読み書きする（`rx.rs:452-464`）．分割すると，その間に届いた `Stop` が block の手前で切る（J52 が plan 側だけを固定している）．
- **c9. `Stream::follow` の `.max(self.expected)` を落とす**（J13），**plan にない segment を 0 で切る規則を落とす**（J23）．
- **c10. abort の局所床を落とす**（`rx.rs:254` の `cut_at(clock.instant(expected))`）: Stop が先に plan を切っている場合（`rx.rs:800` の (Abort, 300, 300, true)）に，clock が配達数でなく Stop の cut で終わる．
- **c11. `finish` の `delivered = None`（`rx.rs:330`）を落とす**: 次の segment は `begin`（`rx.rs:307`）が自分の値で上書きし，guard が古い `(origin, config)` を拒むので不変．テストは要らない．

### (d) 捕まえるか・足す案

- c2 → `ur_26_a_stopped_stream_keeps_its_end`（`rx.rs:800` の 4ケース目が「abort が即終わる」ことを要求する）と `ur_29_a_loss_ends_the_stream_at_its_instant`（`rx.rs:889`）が捕まえる．J46（`plan/maintenance/tools/mutations.json:788-796`）が `item.delivered = …` を消す形で同じ規則を壊す．**形を変えないこと**: `delivered` は `Streams` に残し，stream ごとの器に入れる．
- c3（`k`）→ `ur_29_a_loss_ends_the_stream_at_its_instant` の2ケース目（2 ms の損失で 3 000）が捕まえる．**新しい行 C** を足す: `rx.rs`，`old` `streams.delivered = Some((clock.origin, stream.config, k + len));` → `new` `…, k));`，`filter` `ur_29_a_loss_ends_the_stream_at_its_instant`．当て直し後は代入先（例 `streams.rx[k].delivered`）に合わせる．
- c4 → `ur_26_an_end_takes_delivered_only_from_the_segment_it_cuts`（`rx.rs:983`）が J54 で捕まえる．
- c5 → `ur_29_a_loss_takes_effect_before_a_change_at_its_instant`（`rx.rs:966`）が捕まえる．**新しい行 B** を足す: `core.rs`，`old` `let cuts = |origin, config| line.made(origin, config, item.e, item.seq).ok().flatten().is_some_and(|s| s.cut.is_none());` → `new` `let cuts = |origin, config| line.plan.iter().find(|s| s.origin == origin && s.config == config).copied().is_some_and(|s| s.cut.is_none());`，`filter` `ur_29_a_loss_takes_effect_before_a_change_at_its_instant`．
- c6 → `ur_29_a_loss_ends_the_stream_at_its_instant` の2ケース目（10 ms の変更が line にある状態で 2 ms の損失，床 3 000）が捕まえる．行は c4 と同じ guard の内側なので追加不要．
- c7 → 既存テストは捕まえない（差が出るのは rig の配達が `now` より先行する場合だけ）．規則ではないのでテストも行も足さない．実装は `core.rs:95` の `dir == Dir::Rx && matches!(item.kind, Kind::End { .. })` をそのまま保つ．
- c8 → 決定的には観測できない（同じ `lock` を2回に分けた競合は作れない）．J52（`plan/maintenance/tools/mutations.json:848-856`）は plan の読みだけを守る．実装は `rx.rs:455-463` の1つの `lock` の中で `follow` と `delivered` の書き込みを続け，lock の順序は順序10に従う．
- c9 → J13（`mutations.json:558-566`）と J23（`mutations.json:638-646`）が捕まえる．`old` の式は当て直し後も同じ形に保つ（`stream.follow` の引数だけが変わる見込み）．
- c10 → `ur_26_a_stopped_stream_keeps_its_end` の (Abort, 300, 300, true) が捕まえる．行は無いので**新しい行 D** を足す: `rx.rs`，`old` `stream.cut_at(stream.clock.instant(stream.expected));` → `new` `stream.cut_at(stream.clock.instant(stream.expected + 1));`，`filter` `ur_26_a_stopped_stream_keeps_its_end`（1 MS/s で 301 サンプルになり，300 の assert が落ちる）．
- c1 と c3 の stream 番号版 → **この段階では観測できない**．1b-4c に送る（2 本目を受信する `rx_open(k, …)` と `book`／`planned`／`prune` の key を同時に動かすテストが要る）．
- c11 → 不変なのでテストも行も足さない．

**足すテストは無し**．c2〜c6，c9，c10 は既存のテストが捕まえ，c1・c8 と k=0 の取り違えはこの段階では観測できない．新しい行 B・C・D は「既存テストが固定しているが行が無い規則」を，この段階で文言が動く箇所に付けるためのもの．

**テストの機械的な書き換え**（弱めない）: `lines` が `timeline::Lines` になると，`rx()` の `lines[Dir::Rx] = Some(Line::new(…))`（`rx.rs:646`），`plan()` の `.plan = …`（`rx.rs:669`），`tx.rs:992,1023,1050`，`rx/differential.rs:756` は `Lines::insert((Direction::Rx, 0), line)` の形に直る．`streams.book(core, Dir::Rx, item)` の呼び出し（`core.rs:132`，`control.rs:305,324,573`，`rx/differential.rs:509`，テストの `book()` ヘルパ `rx.rs:673-677`）は key を取る形になり，`control.rs:909` の `delivered` の直接代入も stream ごとの器の名前に直る．assert と入力は変えない．**当て直しが要りそうな既存の行**（`old` が一致しなくなったものだけ）: J49（`rx.rs:332` の `streams.prune(Dir::Rx, by);`），J51（`core.rs:144` の `self.configs[dir as usize].retain(…)`），J52・J53（`stream.follow(&streams)` の引数が変わる場合），J54（tuple の形を変えた場合），J46（代入先を変えた場合）．J13・J23・J45・J55・G03・G04・G58・G60・J60 は文言がそのまま残る見込み．`MockRadio` の 1b-3a（`b8ec8c3`）は床を stop 時に stream ごとに計算する（`crates/ezsdr-mock-radio/src/lib.rs:1254-1261`）が，UHD は `Provider::stop` と `device_lost` が別スレッドなので，その形は採れない（c2）．

### 関わる他の順序

- **1 (`start`)**: `Rx::new` が最初の segment の床を置く（`rx.rs:153`）．最初の segment は `by: None` なので，床を segment の command（`by`）で key すると最初の segment で失われる．`ur_26_uhd_rx_aborts_when_its_provider_is_gone` が見る．
- **2 (`stop`)**: `Provider::stop` は両方向の全 stream に End を book する（`core.rs:131`）．床は受信 stream のものでなければならず，Tx の End に受信の床が混ざってはならない（今は Tx の `Item.delivered` は未使用なので無害だが，共有 slot を `k` で読む形は危険）．abort の局所床（`rx.rs:254`）と plan の床が同じ値を指すこと．
- **3 (thread の panic)**: panic の記録と `device_lost`（`mod.rs:238-243`）は uhd-rx 以外のスレッドで走る．床が `Streams` に無いと c2 になる．
- **4 (stream の thread が loss を見つける)**: 同じ `device_lost` 経路．損失の `seq = 0`（`core.rs:130`）と `made(e, 0)` が床の guard の前提なので，per-stream の seq にしてはならない（順序9）．
- **5 (device が設定を拒否)**: 拒否された変更は `Cold` item で，床を付けてはならない（`core.rs:95`）．拒否で halt した stream は実行中の segment を持たないので guard は床を返さない．`ur_25_a_refused_segment_is_booked_by_uhd_rx`（`rx.rs:1100`）が plan を見る．
- **6 (enable／disable)**: `begin` が床を `(新しい origin, config, 0)` に置き（`rx.rs:307`），`finish` が消す（`rx.rs:330`）．disable（`tx_open(0, &[])`）が Rx の床に触れてはならない．
- **8 (送信 clock と `version`)**: `Streams::changed`／`reconcile`（`core.rs:162-194`）は同じ mutex の下で走る．床を送信側の器へ移したり，`book` の床を `version` に依存させたりしない．
- **9 (同じ instant の `cold` と timed)**: guard は `(e, seq)` で，seq は instance 全体（`core.rs:67-68`，`next_seq` は `core.rs:84-87`）．損失が seq 0 で「Run の開始時に届いた」ものとして扱われる（`core.rs:130`）ことが `ur_29_a_loss_takes_effect_before_a_change_at_its_instant` の前提．
- **10 (lock の順序)**: 床の書き込みは plan の読みと同じ `streams` の lock の中で行う（`rx.rs:452-464`）．`book` も同じ lock（`core.rs:92-105`）．分割すると，待ちの間に届いた `Stop` が block を切れない／切れすぎる．
- **11 (thread の名前と `rejected`)**: 名前の変更は床に影響しないが，block を受けてから `delivered` を書くまでの間に panic すると床が失われる．この段階では観測しない（1b-4c の stream ごとの thread で再検討）．

## 8. 送信の clock の登録と終わり（`tx_clocks`），`version` による再読み込み

この順序が固定する規則は3つ．(i) transmit の plan を変える booking が新しい clock を登録し，前の clock を plan の cut で終える（`design/18-uhd-radio.md:245`，`crates/ezsdr-radio-uhd/src/provider/core.rs:171-194`）．(ii) plan から外れた clock はその origin で終える（`core.rs:187-192`）．(iii) uhd-tx は `version` が変わったときだけ plan を読み直す（`crates/ezsdr-radio-uhd/src/provider/tx.rs:160-172`）．1b-4b では `tx_clocks` を stream ごとの型へ移し，`vec` の添字はその stream の plan の segment の添字のまま，`version` は instance 全体に残す（brief 22-25 行）．

### (a) 既存のテスト

- `ur_14_actions_are_finished_before_the_next_recv` — `crates/ezsdr-radio-uhd/tests/fake.rs:822`．0 channel から transmit を enable すると，`submit` が返るまでに untimed の `apply` と clock の登録が終わる．`fake.rs:833` が `apply tx`，`fake.rs:834-835` が `usrp/tx` の開いた clock を要求．
- `ur_15_every_clock_starts_on_its_lattice` — `fake.rs:873`．`with_tx` の spec で arm が登録する最初の transmit clock の origin が格子の上にあり，clock がちょうど2本（rx と tx）であること（`fake.rs:878-881`）．
- `ur_22_repeat_is_continuous_across_the_wrap` — `fake.rs:1203`．`with_tx` の spec の burst が `Control::new` の seed（`control.rs:218`）経由の clock で送れること（seed が無ければ `book_burst` が `UR-21: no transmit channel is configured` で拒む，`control.rs:597-604`）．
- `ur_25_a_rate_change_admits_a_burst_on_the_new_clock` — `fake.rs:1624`．rate 変更の booking が2本目の `usrp/tx` clock を登録し（`fake.rs:1633`），its domain の burst が送られる（`fake.rs:1634`）．`book_burst` の「現在の clock」が新しい clock であること（`control.rs:600`）も同時に固定．
- `ur_25_uhd_tx_reads_the_plan_again_when_it_changes` — `tx.rs:980`．booking の後に uhd-tx が plan を読み直し，`switch.cut` が plan の cut，`switch.to.clock` が登録された `c1` になること（`tx.rs:1004-1008`）．
- `ur_25_a_burst_on_a_later_clock_survives_an_earlier_switch` — `tx.rs:1034`．3 segment で clock が2本（`tx.rs:1056`），switch が later clock の burst を消さず（`tx.rs:1066-1076`），`adopt_later` が `Planned.clock` で「まだ来る clock」を判定すること（`tx.rs:193`）．
- `ur_25_a_cold_transmit_change_cancels_the_held_bursts` — `fake.rs:1638`．cut での switch が起き，古い clock の held burst が `cancelled by a cold change` で拒否される（`fake.rs:1645-1646`）．
- `ur_23_a_refused_transmit_change_ends_its_clock_at_its_origin` — `tx.rs:1012`．refuse が `changed` を呼び，拒否された segment の clock が origin で終わること（`tx.rs:1027-1030`）．
- `ur_26_a_change_booked_after_the_stop_instant_ends_its_clock` — `control.rs:916`．`Provider::stop` の後に booking された変更の clock が plan から外れ，origin で終わること（`control.rs:925-927`）．
- `ur_25_cold_deadline_overflow_is_rejected_without_panic` — `control.rs:996`．拒否された booking は clock の record と `version` を変えず（`control.rs:1012-1014`），0 channel への disable は最初の transmit clock をその cut で終える（`control.rs:1026`）．
- `ur_24_invalid_explicit_deadline_is_not_asap` — `control.rs:1032`．変換できない deadline でも clock の record と `version` が変わらない（`control.rs:1053-1054`）．
- `rm_26_the_timeline_against_uhd_control` — `crates/ezsdr-radio-uhd/src/provider/control/differential.rs:85`．1000 系列で，`usrp/tx` の全 clock の `(origin, ended_at)` が timeline の plan と一致すること（`differential.rs:48-53, 139-141, 264-269`）．TxCold と RxCold の両方の系列を含む（`crates/ezsdr-radio/tests/generator/mod.rs:16-35, 251-264`）．

### (b) assert と観測

- **Manifest の clock record**：`manifest.clocks.sample_clocks`（`crates/ezsdr-kernel/src/coordinator/ending.rs:234`）の `stream`・`origin`・`root_ticks_per_tick`・`ended_at` を見る．unit では `core.clocks.sample_clock_records()`（`control.rs:925`，`tx.rs:1029`）．登録の有無・origin・cut での終了・origin での終了・二重登録はすべてここに出る．
- **plan と clock の対応**：unit は `streams.planned(Dir::Tx)` の `segment` と `clock` を直接見る（`tx.rs:1004-1008`，`tx.rs:1056`）．end-to-end は burst の `target.domain` と `bursts` の record（`fake.rs:1634`），`sample_clocks()` の開いている clock（`fake.rs:834-835`，`tests/common/mod.rs:339`）．
- **`timing` section**：`arm`（`provider/mod.rs:417`），`start`（`mod.rs:466`），`cold_change`（`control.rs:575`），`tx_switch`（`tx.rs:678`），`plan`（`core.rs:196-207`）．switch の cut と origin，clock を登録した変更の実効時刻が読める．
- **event**：`radio.COMMAND_REJECTED`（`core.rs:373-378`），`radio.LATE_COMMAND`，`radio.TIME_ERROR`．`rejected` section の `clock_not_ended`（`core.rs:189`）は終了の失敗，`kind: tx_burst` の理由は burst の取り消しを見る．
- **device の calls**：FakeDevice の `apply tx …`，`tx_open 0 …`，`tx_send … at=… sob=true`（`fake.rs:833`，`tx_at` が使う `root_ticks_per_tick`，`fake.rs:1046-1049`）．clock の `n` が誤っていれば `at` の root tick がずれる．
- **unit の内部**：`streams.tx_clocks`（`tx.rs:990, 1056`），`tx.plan`・`tx.switch`・`tx.clock`・`tx.later`（`tx.rs:1005-1008, 1066-1074`），`core.clocks.sample_clock_records()`（`tx.rs:1029`），`streams.version`（`control.rs:1008, 1048`）．

### (c) 壊れうる形

1. **登録の欠落・遅れ**：`Streams::book` → `changed`（`core.rs:103, 163-168`）→ `reconcile`（`core.rs:174-194`）のどこかが TX で呼ばれない，または `reconcile` が plan の新しい segment を見ない（`core.rs:177` の `index == self.tx_clocks.len()` の判定を，per-stream の別の長さ（例：全 stream の合計）と比べる）．`planned().clock` が `None` になり，`switch.to.clock` が `None`（`tx.rs:179`），`book_burst` が「no transmit channel」か「not on the current transmit SampleClock」で拒む（`control.rs:602-609`）．
2. **取り違え（方向）**：`changed(core, dir)` の `dir` を `reconcile` に渡さず，常に TX（または常に変更の方向と違う line）を plan として読む．RX の変更で TX の clock が RX の origin に登録されたり RX の cut で終わったりする．`planned(Dir::Tx)` の clock が RX の plan の添字で引かれる．
3. **取り違え（stream, `k`）**：`tx_clocks` を stream ごとに持つのに，`reconcile` が stream 0 の list に登録し，`planned((Dir::Tx, k))` が別の stream の list を読む．今は `k=0` だけなので観測できない（`mod.rs:474-477` は1本ずつ）．
4. **添字のずれ**：`planned` が plan の全 segment（prune で畳んだ過去の segment を含む，`crates/ezsdr-radio/src/timeline.rs:202-203, 865-874`）ではなく未来の segment だけを返し，`tx_clocks[index]` と `self.at`（`tx.rs:176-182`）が食い違う．または prune のときに `tx_clocks` を切り詰めて，plan の segment と clock の対応が1つずれる（`core.rs:139-145`，`tx.rs:682`）．
5. **終了の欠落・時刻違い**：cut で終えない（`core.rs:182-184`），cut ではなく `clock.origin` で終える，orphan を終えない（`core.rs:187-192`），二重に終えて `clock_not_ended` を残す（`core.rs:189`）．`ended_at` が `None` のまま，または origin になる．
6. **`version` の取り違え**：`changed` が `version` を進めない（`core.rs:163`），TX が RX の stream の version を読む，または per-stream の version にして TX の変更だけ別の値になる．uhd-tx が plan を読み直さず，古い clock のまま burst を待ち続ける（`tx.rs:165-169`，`tx.rs:149-151`）．
7. **`n` の取り違え**：`core.register(Dir::Tx, segment.config.ratio.num() as i64, segment.origin)`（`core.rs:178`）の `n` を別の stream・別の segment の値にする．今の differential は `(origin, cut)` だけを比べ（`differential.rs:139-141`），`n` は比べない．TX の変更後の clock の `root_ticks_per_tick` を直接 assert するテストは無い．
8. **最初の clock の seed**：`Control::new` の `streams.tx_clocks = start.tx…`（`control.rs:218`）を落とす，または `arm` の登録（`mod.rs:408-416`）と別の stream に入れる．0 channel からの enable 以外の経路（arm 時点で channel がある spec）で `book_burst` が現在の clock を見つけられない．
9. **lock の外での登録**：`reconcile` を `streams` の lock の外で呼び，plan が先に見えて clock が後から入る．`Tx::read` は `version` と `planned` を同じ lock で読む（`tx.rs:161-169`）ので，その間に入ると `Planned.clock` が `None` の瞬間ができる．race なので決定的なテストにはならない（順序10の規則）．

### (d) 捕まえるか・足す案

- (c-1) 登録の欠落：**既存が捕まえる**．`ur_25_uhd_tx_reads_the_plan_again_when_it_changes`（`tx.rs:1008` の `switch.to.clock == Some(c1)`）と `ur_25_a_rate_change_admits_a_burst_on_the_new_clock`（`fake.rs:1634`）．ただし「登録そのもの」を壊す mutation 行は今は無い（`grep -n "tx_clocks\|reconcile\|register(" plan/*/tools/mutations.json` で `U22`・`J21` のみ）．**行の案**：`core.rs:177` の `if index == self.tx_clocks.len() {` → `if false {`，`test_bin: lib`，`filter: ur_25_uhd_tx_reads_the_plan_again_when_it_changes`（`planned().clock` が `None` になり `tx.rs:1008` が落ちる）．
- (c-2) 方向の取り違え：**differential が捕まえる**．RxCold の系列で `expect()` の tx plan（`differential.rs:139-141`）と観測の `usrp/tx` clock（`differential.rs:264-269`）が食い違う．単体の TX テストは1方向しか立てないので，方向の取り違え専用の追加テストは不要．
- (c-3) stream の取り違え：**今は観測できない**．1b-4c に送る（下記）．
- (c-4) 添字のずれ：**既存が捕まえる**．`ur_25_uhd_tx_reads_the_plan_again_when_it_changes`（`tx.rs:1004-1008`，2 segment）と `ur_25_a_burst_on_a_later_clock_survives_an_earlier_switch`（`tx.rs:1056, 1066-1076`，prune 後の switch と `adopt_later`），differential．**行の案**：`core.rs:156` の `clock: (dir == Dir::Tx).then(|| self.tx_clocks.get(index).map(|(clock, _)| *clock)).flatten(),` → `clock: None,`，`filter: ur_25_uhd_tx_reads_the_plan_again_when_it_changes`．
- (c-5) 終了の欠落・時刻違い：**既存が捕まえる**．cut での終了は `ur_25_cold_deadline_overflow_is_rejected_without_panic`（`control.rs:1026`）と differential（loss の End が tx の plan を cut する，`timeline.rs:826-830`）．orphan は `ur_26_a_change_booked_after_the_stop_instant_ends_its_clock`（`control.rs:925-927`）と `ur_23_a_refused_transmit_change_ends_its_clock_at_its_origin`（`tx.rs:1029-1030`）．**行の案**：`core.rs:182` の `if let Some(cut) = segment.cut.filter(|_| !*ended) {` → `if let Some(cut) = segment.cut.filter(|_| false) {`，`filter: ur_25_cold_deadline_overflow_is_rejected_without_panic`（`control.rs:1026` が落ちる）．
- (c-6) `version`：**既存が捕まえる**．`ur_25_uhd_tx_reads_the_plan_again_when_it_changes`（`tx.rs:980`）が「TX の変更を読み直す」を，`control.rs:1014`・`control.rs:1053` が「拒否では進めない」を固定．**行の案**：`core.rs:163` の `self.version += 1;` → `self.version += 0;`，`filter: ur_25_uhd_tx_reads_the_plan_again_when_it_changes`．既存の `J59`（`self.version.is_some()`，`plan/maintenance/tools/mutations.json:922-923`）はそのまま使えるが，`Streams` の分割で `streams.version` の綴りが変わるなら当て直す．
- (c-7) `n`：**既存は捕まえない**（differential は origin と cut のみ，`differential.rs:139-141`；TX の変更後の `root_ticks_per_tick` を assert するテストは無い）．1b-4b はテストを変えないので（brief 37-39 行），**1b-4c に送る**：rate 変更の後に `manifest.clocks.sample_clocks` の新しい `usrp/tx` clock の `root_ticks_per_tick` が新しい decimation（例：2 MS/s なら 100）であることと，root tick で指定した burst が `device.transmitted(root_tick, n)` の位置に載ることを assert するテスト．mutation 行は `core.rs:178` の `segment.config.ratio.num() as i64` → `200`．
- (c-8) seed：**既存が捕まえる**．`ur_15_every_clock_starts_on_its_lattice` の「2本ちょうど」（`fake.rs:878`）が再登録を，`ur_22_repeat_is_continuous_across_the_wrap`（`fake.rs:1203`）が seed の欠落を捕まえる．**行の案**：`control.rs:218` の `streams.tx_clocks = start.tx.map(|(clock, _)| (clock, false)).into_iter().collect();` → `streams.tx_clocks = Vec::new();`，`test_bin: fake`，`filter: ur_22_repeat_is_continuous_across_the_wrap`．
- (c-9) lock：**決定的なテストは無い**．順序10 の規則として扱い，`reconcile` を `streams` の lock の中で終える（`core.rs:174-194` の現状を保つ）．レビューで確認する．
- 当て直しの見込み（この順序の分）：`U22`（`plan/phase7/tools/mutations.json:678-679`，`streams.tx_clocks.last()` の綴りが変わる），`J21`（`plan/maintenance/tools/mutations.json:631-632`，`self.tx_clocks` が stream の field になる），`J62`（`plan/maintenance/tools/mutations.json`，`self.changed(core, dir)` の署名が `k` を取るなら），`J59`（`streams.version` の綴りが変わるなら）．`U11`（`mod.rs:412` の lattice）は arm の行がそのままなら一致し続ける見込み．
- 1b-4c に送る（2本以上の stream でしか出ない誤り）：(i) `k≠0` の `tx_clocks` の登録・終了・orphan と，`planned((Dir::Tx, k))` の引き先，(ii) `reconcile` が変更の stream だけを触り，他の stream の clock を終えないこと，(iii) `book_burst` の「現在の clock」が宛先 stream の最後の clock であること，(iv) `version` を per-stream にするなら，各 stream の uhd-tx が自分の version で読み直すこと，(v) `Core::register` がまだ `id.child(dir.name())`（`core.rs:473-477`）で stream を名指ししないので，clock の stream id（今は `usrp/tx`）を `usrp/tx/s<k>` にするのは 4c．

### 関わる他の順序

- **1（start）**：`arm` が最初の transmit clock を登録し（`mod.rs:408-416`），`Control::new` がその clock を list に seed する（`control.rs:218`）．ここで別の stream に入れると以後の登録の添字が全部ずれる．`ur_15_every_clock_starts_on_its_lattice` の「2本ちょうど」が再登録を捕まえる．
- **2（stop）**：`Streams::end` が両方向に `End` を記帳し（`core.rs:126-134`），TX の最後の clock を stop の cut で終える（`core.rs:182-184`）．join の順（control → 送信 → 受信，`mod.rs:501-506`）より前に終わる．stop の後に booking された変更の orphan は `ur_26_a_change_booked_after_the_stop_instant_ends_its_clock` が見る．
- **3（panic）**：`spawn` の `catch_unwind` が `device_lost` を呼び（`mod.rs:239-243`），`device_lost` → `streams.end`（`core.rs:383-400`）で TX の clock が loss の cut で終わる．`rejected` の `thread` は `uhd-tx` のまま（`mod.rs:476`）．
- **4（loss）**：3 と同じ経路．`end` の `seq` を per-stream にすると loss の End の順序が変わり，clock の終了時刻が動く（brief 25 行は `seq` を instance 全体に残す）．differential の loss 系列が `tx_clocks` を比較する（`differential.rs:139-141, 264-269`）．
- **5（device の設定拒否，RM-25）**：`refuse` → `changed` → clock が origin で終わる（`core.rs:110-121, 187-192`）．同じ instant の後続の成功した変更があるときは，その segment の cut で終わる（`timeline.rs:758-765, 850-857`）．`J62` がこの規則を固定．
- **6（enable/disable）**：0 channel からの enable は `book_cold` の `from_none` が streamer を開いて `streams.book` し（`control.rs:548-579`），`reconcile` が clock を登録する（`core.rs:177`）．0 への disable は cut で clock を終える（`control.rs:1026`）．`idle`・`configs` を stream ごとに移すとき，この2つと同じ stream を指すこと．
- **7（受信の cut と delivered）**：`delivered` は RX の `End` の cut にだけ効く（`core.rs:93-98`）．TX の clock の終了に混ざると，differential の Stop/End 系列が落ちる（`timeline.rs:879-895`）．
- **9（同じ instant の cold と timed，UC-2）**：`seq` は instance 全体のまま（brief 25 行）．同じ instant の2つの cold は plan の2 segment と2 clock を作り，clock の添字は plan の segment 順（`core.rs:176-186`）．`ur_25_a_burst_on_a_later_clock_survives_an_earlier_switch`（`tx.rs:1051-1056`）と differential が固定．
- **10（lock の順序）**：`reconcile` は `streams` の lock の中で `core.register`/`core.clocks.end` を呼び，device の呼び出しはしない（`core.rs:174-194`）．uhd-tx は `version` と `planned` を同じ lock で読む（`tx.rs:161-169`）．登録を lock の外に出すと，plan だけが見える瞬間ができる（(c-9)）．
- **11（thread の名前と `rejected`）**：panic の記録は 3・4 と同じ経路で，clock の終了時刻に効く．名前を stream ごとにしても `uhd-rx`・`uhd-tx` のまま（brief 29-31 行，`mod.rs:474-477`）．

## 9. 同じ instant の `cold` 変更と timed 変更の順序（UC-2，RM-25 の床は instance 全体の `seq`）

この順序で固定すべきなのは，(i) 同じ instant に `cold` 変更（channels / sample_rate）と `hardware_timed` 変更（frequency / gain）が来たとき，互いの effective instant を動かさないこと（`cold` は RM-25 の床，timed は device lead と release の床だけ），(ii) その instant の timed 値が `cold` の始める新しい segment の設定に載ること，(iii) その順序の根拠になる到着順 `seq` が **instance 全体の1つの counter**（`Streams.seq`）であること，の3つ．HEAD（`05e8d2f`）の仕組みは5か所に分かれている:

- **到着順の counter は instance に1つ**: `Streams.seq` は instance の field（`crates/ezsdr-radio-uhd/src/provider/core.rs:68`）で，`next_seq()` が1つずつ進める（`core.rs:84-87`）．booking はすべてここから取る — `cold` は `control.rs:512`，timed は `control.rs:422`（`lock(&self.core.streams).next_seq()`），`Stop` は `control.rs:304`，`start_rx` は `control.rs:321`，`Streams::end` は `core.rs:130`（loss だけ 0）．
- **RM-25 の床**: `timeline::command_instant`（`crates/ezsdr-radio/src/timeline.rs:380-389`）が `effective`（`timeline.rs:367-373`）で「同じ line の，先に受けた（`earlier.seq < item.seq`）`cold` item の最大 `e`」まで押し上げる．timed 変更は line の item ではないので床にも床の対象にもならない．比較に使う `seq` が instance の `next_seq` の値．
- **UC-2 の順序キー**: `Control.colds: BTreeMap<(i64, u64), …>`（`control.rs:148`，`config_at` が `(e, seq)` の昇順で `..=(t, u64::MAX)`，`control.rs:373-379`），`Control.held: BTreeMap<(i64, u64), Timed>`（`control.rs:149`），`ColdConfig.updates: ClockMutex<BTreeMap<(i64, u64), Settings>>`（`control.rs:60`，`book` `control.rs:74-76`，`issued` `control.rs:78-81`，`settings` が `..=(origin, u64::MAX)` を昇順で畳む `control.rs:84-91`）．同じ instant の同じ key は到着の後が勝つ（UC-2，`design/05-module-api.md:227`）．
- **新しい segment への投影**: `segment_config` が held の timed を新しい `ColdConfig` に記帳し（`control.rs:335-341`，`timed.dir == dir` だけ），`settings(origin)` が `e' ≤ origin` のものを載せる（等号を含む）．0→n enable は uhd-control 自身が設定し，held を `(e, MAX)` まで畳む（`control.rs:558-564`）．release 済みの値は `issued` で `(effective, seq)` に付け替える（`control.rs:472-474`）．
- **装置の queue は1本**（`design/18-uhd-radio.md:239-243` UR-24）: `Control.released` は instance の Vec（`control.rs:150`）で，`release()` が `effective = latest.max(e)` とし（`control.rs:447-448`），release 済みより前の instant の command はその instant に遅れて適用される（`LATE_COMMAND`，`control.rs:457-459`）．hand over する instant は方向をまたいで単調．

### (a) 既存のテスト

**A. 同じ instant の timed どうしの順序と，装置 queue の順序（UC-2 の核）**

- `crates/ezsdr-radio-uhd/src/provider/control.rs:1083` `ur_24_a_timed_update_acts_on_the_configuration_in_force_at_its_instant` — 50 ms の同じ instant の gain 1.0→2.0 を到着順に発行し，100 ms の channels→0 の後は「no channel」で記録だけ，を `(e, seq)` の順序で固定する．
- `crates/ezsdr-radio-uhd/src/provider/control.rs:1176` `ur_24_the_device_queue_order_is_the_effective_order` — release 済み（10 ms）より前の instant（9.7 ms）の command を後から受けると，装置へは 10 ms で渡り `LATE_COMMAND { requested: 9.7 ms, applied: 10 ms }` が出る（mutation R03 の killing test）．
- `crates/ezsdr-radio-uhd/src/provider/control.rs:1152` `ur_24_a_halted_direction_fills_no_queue_slot` — halted の方向の held は queue 深さに数えない（`held.iter()` を instant ごとに読む `channels_at`，J68）．
- `crates/ezsdr-radio-uhd/tests/fake.rs:1327` `ur_24_hardware_timed_updates` — 受領の 1 ms 後の `at` は late で device lead 後に適用（`applied` の instant と `apply` の `at=` が一致），queue の深さ 16（U20，U21）．
- `crates/ezsdr-radio-uhd/tests/fake.rs:1359` `ur_24_a_far_future_update_does_not_delay_a_nearer_one` — +500 ms の後 +20 ms のものを release すると，近い方だけが release window で装置に渡る（`at == effective == now+20 ms`，`LATE_COMMAND` 無し；U37）．

**B. `cold` 変更の新しい segment が timed 値を運ぶ（投影）**

- `crates/ezsdr-radio-uhd/src/provider/control.rs:955` `ur_25_switch_projects_timed_updates_in_effective_order` — 純 `ColdConfig`: origin 100 で gain 7.0（e=70 が e=50 に勝つ），**origin と等しい instant の `(100, 4)` が freq を載せる**（`:962`），`issued` の付け替えで (50,2)→80 が gain を，`(100,4)→101` が origin 100 の freq を戻す（`:964-967`）．等号 instant を固定する唯一のテスト．
- `crates/ezsdr-radio-uhd/src/provider/control.rs:931` `ur_25_later_timed_update_survives_cold_switch` — `cold`（1 s）の**後**に booking した timed（e = now + device lead）が，両方向とも `plan[1].settings.settings(origin)` に載る．
- `crates/ezsdr-radio-uhd/tests/fake.rs:1389` `ur_25_the_switch_applies_the_configuration_in_effect_at_e2` — switch の untimed configure が `apply rx 0 rate=2000000 freq=2100000000 … at=-` を出す（timed は e2 の 20 ms 手前；R05 の killing test）．**timed の `at` が cold の `at` と等しい例は，この段階には無い**（後述）．
- `crates/ezsdr-radio-uhd/tests/fake.rs:1593` `ur_25_enabling_tx_applies_the_configuration` — 0→1 enable の configure が timed の freq/gain/antenna を載せる（U39）．timed は enable より手前の instant．
- `crates/ezsdr-radio-uhd/src/provider/control.rs:971` `ur_26_a_stop_cancels_no_update` — `Stop` は held を消さず，その後の `cold` の projection が held を載せ続ける（J11）．`Provider::stop` の `cancel_held` だけが消す．
- `crates/ezsdr-radio-uhd/src/provider/control.rs:1104` `ur_24_a_direction_halted_by_a_refusal_has_no_channel` / `control.rs:1201` `ur_24_a_late_update_halted_before_it_can_act_has_no_channel` — timed の「no channel」判定は `config_at(自分の instant)` と halt で決まり（J64，J71），enable の segment がその値を載せる．
- `crates/ezsdr-radio-uhd/src/provider/control.rs:795` `ur_14_booking_does_not_wait_for_a_future_cold_switch` — 1 s の `cold` の後でも timed の instant は自分のまま（`applied` の `at == ticks(2_000_000)`），booking は待たない（U41）．

**C. RM-25 の床（`cold` どうし）と `seq`**

- `crates/ezsdr-radio-uhd/src/provider/control.rs:884` `ur_25_a_cold_change_that_arrives_after_a_later_one_follows_it` — 後から来た `cold` が先の instant（300 ms）に床上げされ，`LATE_COMMAND` が出る（J17）．
- `crates/ezsdr-radio/src/timeline.rs:367-373` `effective` と `crates/ezsdr-radio/tests/timeline.rs:141` `rm_26_the_timeline_cases` — 同一 instant の `Stop`/`start_rx`/`0 channel` 往復（`:158-167`）と床の assert（`:272-283`，「a Stop is not floored」`timeline.rs:283`）．`crates/ezsdr-radio/tests/timeline.rs:814` `rm_34_a_move_does_not_floor_an_item_received_before_it_at_its_instant` も「同一 instant は到着順」を読む．
- `crates/ezsdr-radio-uhd/src/provider/rx.rs:1068` `ur_25_a_second_change_follows_the_first`，`rx.rs:1124` `ur_17_an_ended_segment_prunes_the_items_before_it`（line の `items` の `seq` を読む：`rx.rs:1142`）．
- `crates/ezsdr-radio-uhd/src/provider/rx.rs:966` `ur_29_a_loss_takes_effect_before_a_change_at_its_instant` — loss の `seq = 0`（`core.rs:130`）が同一 instant の変更より先に効く（J60）．

**D. 差分 harness（VH-8 の層）**

- `crates/ezsdr-radio-uhd/src/provider/control/differential.rs:84` `rm_26_the_timeline_against_uhd_control` — 1 000 seed の列で clock と発行済み timed（`issued`/`no channel` の行）を `timeline::plan` の期待と比べる．列は `crates/ezsdr-radio/tests/generator/mod.rs:239-250` が「4つの共有 instant から半分の確率で `at` を引く」ので，**同一 instant の `cold`×timed の組が普通に混ざる**．`by_instant`（`differential.rs:159-162`）は `(at, key)` で並べ替えて方向間の順序を不問にし，release window が効く列は `timed` を比較しない（`differential.rs:118-133`）．
- `crates/ezsdr-radio-uhd/src/provider/rx/differential.rs:111` `rm_26_the_timeline_against_uhd_rx` は timed を模型しない（`World` は `streams.seq` を `STOP_SEQ` に使うだけ：`rx/differential.rs:518`）．

**「同一 instant の `cold` と timed の組」を明示するテストは無い（検索）**: `grep -rn "same instant\|one instant\|at one instant" crates/ezsdr-radio-uhd/tests/fake.rs` → 0 件．`grep -n "HardwareTimed" crates/ezsdr-radio-uhd/tests/fake.rs` → `:1332`・`:1333`（40 ms 差），`:1365`・`:1366`（同一方向の timed どうし），`:1395`（cold は `:1396` の 20 ms 後），`:1771`（queue 深さ）だけで，timed と `cold` を同じ本体に持つのは `:1389` のみ・その `at` は 20 ms 違う．unit 側も `grep -rn "held.range\|updates.range" crates/ezsdr-radio-uhd/src` → `control.rs:86`・`:560` の実装2行だけで，テストが等号 instant を組で見るのは control.rs:955 の純 `ColdConfig` だけ．

### (b) assert と観測

不変として固定する形:

- `cold` の `e` は `command_instant`（`timeline.rs:380-389`）が決める: `at`，absent/past なら booking の instant，receive は `line.stream.origin` 以上，そして同じ line の先着 `cold` の `e` 以上．timed は床に入らない．
- timed の `e` は `book_timed`（`control.rs:391-428`）が決める: `at`，無ければ receipt + device lead，`at` がそれより手前なら `LATE_COMMAND` を出して `earliest`． release（`control.rs:434-481`）で `released` の最大 instant まで遅れる．`cold` はここに入らない．
- switch / enable の configure は `ColdConfig.settings(origin)`（`control.rs:84-91`）で `e' ≤ origin` の timed 値を `(e, seq)` の昇順に畳む（同じ field は後着が勝つ）．release 済みは `issued`（`control.rs:78-81`）が `(effective, seq)` に付け替えるので，遅れた command は元の instant の投影から抜ける．
- device へ渡る instant は `released`（instance の Vec）で単調（`control.rs:447-448`，`:457-459`）．

各テストの assert と観測:

| テスト | assert | 観測 |
|---|---|---|
| control.rs:1083 | `rec.applied` の gain 行が [(1.0, issued), (2.0, issued), (4.0, no channel)] の順；`apply tx` は2回 | `lock(&core.rec).applied` の行順と `device.calls()` |
| control.rs:1176 | device の `at` が [us(10 000), us(10 000)]；`LATE_COMMAND` 1件 (9 700→10 000) | `device.calls()` の `at=` を parse，`events` |
| control.rs:1152 | `held.len() == depth + 2`，`rejected` 空 | `control.held`，`rec.rejected` |
| fake.rs:1327 | +1 ms の `at` は `LATE_COMMAND.applied` と同 instant；17件目は `COMMAND_QUEUE_FULL{depth:16}` | events の payload，`device.calls()` の `at=` |
| fake.rs:1359 | `applied == [(now+20 ms, now+20 ms)]`（`at` と `effective`）；`LATE_COMMAND` 空 | `device.calls()` の `at=`/`effective=`，events |
| control.rs:955 | origin 100 で gain 7.0・freq 1.1e9（e=100 を含む）；`issued` 後は gain 5.0 / freq 1e9 | `ColdConfig::settings()` の戻り値（純） |
| control.rs:931 | `config[gain]==3.0`，`rejected` 空，`plan[1].settings…gain == Some(3.0)`（両方向） | `Control.config`，`rec.rejected`，`Planned::settings`＋`ColdConfig::settings` |
| fake.rs:1389 | `calls` に `apply rx 0 rate=2000000 freq=2100000000 … at=-` が1回 | `device.calls()` の文字列 |
| fake.rs:1593 | `apply tx 0 rate=1000000 freq=2400000000 gain=5 antenna=TX/RX at=-` | `device.calls()` |
| control.rs:971 | `Stop` 後も `held.len()==1` で projection が gain 3.0；`cancel_held` で1行 `cancelled` | `control.held`，projection，`rec.applied` |
| control.rs:1104 / :1201 | `no channel` の行と `apply` の有無が halt と一致；late は自分の instant で判定 | `rec.applied` の `note`/`issued`，`device.calls()` |
| control.rs:795 | booking は待たず（10 s 以内・`core.now()==0`），plan の cut/origin と timed の `at==2 ms` | plan（`Planned`），`rec.applied`，時間 |
| control.rs:884 | `LATE_COMMAND` (100 ms→300 ms)；`plan.by == [None, Some(2)]` | events の payload，plan |
| timeline.rs:141（`rm_26_the_timeline_cases`） | 同一 instant の stop/start/0-channel 往復の segments；`effective(late,&late[1])==2 s`（`:276-277`），`Stop` は floor されない（`:283`） | `segments()`/`effective()` の戻り値 |
| rx.rs:1068 | blocks `[(0,10 000),(0,1 000)]`，clocks `[(0, Some(10 000 sample)), (65/115 ms, None)]` | publish した block，`clocks(&core)` |
| rx.rs:966 | planned `[(0, Some(12 000))]`（loss が先） | plan |
| control/differential.rs:84 | 1 000 seed で rx/tx clock，timed 行（release window の列を除く），拒否数が一致 | clock registry，`rec.applied`，`rec.rejected` |

### (c) 壊れうる形

1. **`seq` を stream ごと・line ごとに取る**（`RxStream`/`TxStream` か line ごとの counter，方向ごとの `next_seq`）．line 内の順序は保たれるので plan は不変だが，(i) `held`/`colds` の `(e, seq)` が方向をまたいで別系列になり，同一 instant の release 順（device call の順）が変わる，(ii) `Streams::end` が両 line に**同じ seq** で記帳する形（`core.rs:130-133`）が崩れ，loss の `seq=0` と stop の End の位置が line ごとにずれる，(iii) `rx/differential.rs:518` の `streams.seq = STOP_SEQ - 1` が成り立たなくなる．
2. **`ColdConfig.settings` の bound / 順序を変える**（`..=(origin, u64::MAX)` → `..(origin, u64::MAX)` の排他，`range` の代わりに挿入順の Vec，`(e, seq)` を `e` だけに）．origin と等しい instant の timed 値が新しい segment から落ち，同じ field の同一 instant 2件の勝者が到着順でなくなる（UC-2 違反）．
3. **投影の配線を stream ごとに切る**: (i) `segment_config` の `timed.dir == dir` を落とす/別方向を混ぜる，(ii) `book_timed` の `self.projections(dir)` を「今の stream の config だけ」にする（今は各方向1本なので差が出ない．2本目で初めて出る → final の 1b-4c 一覧へ），(iii) enable の merge（`control.rs:558-564`）の bound/filter を変えて，同一 instant の timed を 0→n の configure が落とす．
4. **`released`/`latest` を方向ごとにする**（`Control` の field は instance だが，方向別の Vec に分ける/`latest` を同方向だけから取る）．同一 instant をまたぐ「先に release した後の instant へ遅れる」（UC-2 の装置 queue 1本）が方向をまたぐ場合だけ崩れる．R03 は同一方向しか見ない．
5. **`issued` の付け替えを落とす**（遅れた command が元の `e` のまま投影に残る）．次の segment の configure が，装置が後に適用する値を先に載せる．
6. **床が timed に広がる/狭まる**: `command_instant` や新設の per-stream booking が held の instant を `cold` の床に混ぜる，あるいは `book_timed` の `e` を何かの `cold` の instant で押し上げる（timed が cold に floor される）．`cold` と timed は互いの instant を動かさない，が壊れる．
7. **`ColdConfig.updates` の共有を stream ごとに割る**: `book_timed` はその方向の**全投影**に記帳する（`control.rs:424-426`，`release` も全投影の `updates` lock を一括で取る `control.rs:462-475`）ので，今の stream だけに記帳する形にすると，別 segment の projection が値を見失う（今は1本なので不可視; final へ）．

### (d) 捕まえるか・足す案

| (c) | 既存のテストが捕まえるか | 足す案 |
|---|---|---|
| 1 | **捕まえない**．`grep -rn "\.seq" crates/ezsdr-radio-uhd/tests crates/ezsdr-radio-uhd/src/provider/*.rs` の一致は `fake.rs:409`（Authority の handle）と `rx.rs:1116`・`:1142`，`tx.rs:1070`・`:1080`（line の items を読むが，テスト自身が seq を渡す）だけで，両 line の End の seq や held のキーの seq を比べるテストは無い（`control.rs:1077` は `order.0` の instant だけ） | **T2**（instance の counter を固定）．stop 側は項目2の「両 line に同じ `seq`」の assert と対 |
| 2 | **等号以外は捕まえる**（control.rs:955 が `:962` で e=origin を含む; 順序は `:961-967`）．**等号 instant が switch の device call に載ることを見る統合テストは無い** | **T1** |
| 3 | (i) 捕まえる（fake.rs:1389，R05；control.rs:931/971）．(iii) 捕まえない（enable の既存テストの timed は手前の instant: fake.rs:1600-1604） | **T1(c)**（timed を先に booking した enable の configure） |
| 4 | **同一方向は捕まえる**（control.rs:1176，R03）．方向をまたぐ形は無い | **T3** |
| 5 | 捕まえる（control.rs:966-967） | 不要 |
| 6 | **捕まえない**（床のテストは `cold` だけ: control.rs:884，timeline.rs:272-283） | **T2** の (iii)，mutation 行 |
| 7 | 今は不可視（各方向1本） | final の 1b-4c 一覧へ（テストは 2本目の段階で） |

**T1: `ur_25_a_timed_update_at_the_cold_change_s_instant_reaches_the_switch`（統合，`tests/fake.rs`）**

- 狙い: timed の `at` を `cold` の `at` と**等しく**して，(c)2・3(iii) を統合の経路で固定する．
- 形: `fake.rs:1389-1401` と同じ `Direct::with_links(FakeConfig::default(), &[], vec![attached(DropOldest)])`．`let at = direct.now() + ms(300);` に `update("radio.rx.frequency_hz", 2.1e9, HardwareTimed, Some(at))` と `update("radio.rx.sample_rate_hz", 2e6, Cold, Some(at))` を**両順序で**（2つの `#[test]` か1つのループ）流し，`settle` の後:
  - `calls` に `apply rx 0 rate=2000000 freq=2100000000 … at=-` が1回（switch の configure が等号 instant の timed 値を運ぶ）;
  - `calls` に freq の timed apply が `at={at}`・`effective={at}` で1回，`radio.LATE_COMMAND` 空（互いを floor しない）;
  - `applied` に freq の `issued: true` 行が `at == at` で1つ（C02 の観測形）．
- 観測: `device.calls()`（`at=`/`effective=`，`fake.rs:1374-1375` の parse 形），`direct.of("radio.LATE_COMMAND")`，Manifest `applied`．
- 既存で捕まらない理由: 等号 instant の組が無い（(a) の検索）．unit（control.rs:955）は純 `ColdConfig` で device call を見ない．
- mutation 行の案: `control.rs:86` の `updates.range(..=(origin, u64::MAX))` を `..(origin, u64::MAX)` に（**今の text で書ける唯一の行**）— T1 の configure が既定の freq になり死ぬ．もう1行は T1(c) 用（下）．

**T1(c): `ur_25_an_enable_at_a_timed_update_s_instant_applies_it`（統合，`tests/fake.rs`）**

- 狙い: 0→n enable と同一 instant の timed（timed を先に booking）で，enable の configure が `control.rs:558-564` の merge を通すこと．
- 形: `Direct::with_links(FakeConfig::default(), &[("radio.rx.channels", Scalar::from(0))], vec![attached(DropOldest)])`；`at = now + ms(300)`；`update("radio.rx.frequency_hz", 2.1e9, HardwareTimed, Some(at))` → `update("radio.rx.channels", 1, Cold, Some(at))`；`settle` 後 `calls` に `apply rx 0 … freq=2100000000 … at=-`（rate は既定のまま）．逆順（cold が先）では enable の configure には載らず timed の release が値を届ける（差は現状どおりで，この段階では変えない）— 2つ目は `applied` 行の形だけを記録として assert してよい．
- mutation 行の案: `control.rs:560` の `self.held.range(..=(e, u64::MAX))` を `..e` に — T1(c) が死ぬ．

**T2: `ur_25_a_cold_change_and_a_timed_update_at_one_instant_keep_their_own_instants`（unit，`src/provider/control.rs` の tests）**

- 狙い: (i) `seq` が instance の1つの counter から到着順に取られること（(c)1），(iii) 等号 instant の timed 値が新しい segment の projection に載ること（(c)2），互いに floor しないこと（(c)6）．
- 形: `rig()`（`control.rs:769`）＋ `control(&core, &[Dir::Rx, Dir::Tx])`．`ms(300)` に cold（rx rate）／timed（rx gain）／timed（tx gain）を**両順序**で booking し:
  - `lock(&core.streams).seq == 3`，rx line の `items` の seq と `control.held` のキーの seq が booking 順（例: 先頭の booking が 1，以降 2・3．tx の held も同じ counter）;
  - `held.keys().map(|(e, _)| *e) == [ms(300), ms(300)]`（instant が動かない），`events` の `LATE_COMMAND` 空;
  - `plan_of(&core, Dir::Rx)[1].settings…settings(origin).gain == Some(3.0)`（等号 instant が投影に載る）．
- 観測: `Streams.seq`・line の `items`・`Control.held`・`ColdConfig::settings`・events（`control.rs:887` の `rig_with_links` 形）．`control.rs:984`・`:1077`・`rx.rs:1142` と同じ白箱．
- mutation 行の案: (a) `control.rs:422` の `let seq = lock(&self.core.streams).next_seq();` を direction ごとの counter（新設 `next_timed_seq(dir)` 等）に置換 — seq の assert が死ぬ．(b) `book_timed` の `e` の決定の後に「`cold` の床まで `e` を押し上げる」分岐を足す（実装後に exact text）— `held` の instant assert が死ぬ．`old` は実装後の text で取る．

**T3（任意）: `ur_24_the_device_queue_is_one_across_the_directions`（unit，`src/provider/control.rs` の tests）**

- 狙い: (c)4（`released` は instance で，hand over する instant は方向をまたいで単調）．UR-24 の装置 queue 1本の形．
- 形: `control.rs:1176` と同じ rig で `control(&core, &[Dir::Rx, Dir::Tx])`，両方向の streamer を開く．`book_timed(tx gain 1.0, at(us(10_000)))` → `time.advance_to(us(7_500))` → `release()` → `book_timed(rx gain 2.0, at(us(9_700)))` → `release()`．rx の `apply` の `at == us(10_000)`，`LATE_COMMAND` 1件（9 700→10 000）．
- mutation 行の案: `latest` を同方向の `released` だけの最大にする（実装後）—— T3 が死ぬ．採るかは orchestrator 判断（UR-24 の範囲としてもよい）．

**当て直しが要りそうな既存の mutation 行（この項目の近く）**

- `plan/phase7/tools/mutations.json:937` R03（`release` の `latest.max(e)`）— `released`/`latest` の形が変われば `old` を取り直す．`:947` R04（`timeline.rs` の `command_instant`）— `timeline.rs` はこの段階では触らないので不変の見込み．`:957` R05（`segment_config` の held の loop）— stream key 化で取り直し．`:655` U20・`:665` U21・`:827` U37・`:1067` R21・`:1631` C02 — `book_timed`/`release`/記録の text が動けば取り直し．
- `plan/maintenance/tools/mutations.json:598` J17（`book_cold` の `late_command`）・`:538` J11（`Stop` の Tx 送信）・`:929` J60（loss の `seq=0`）・`:1009`/`:1019`/`:1039` J68/J69/J71（`held.iter()`・`channels_at`・`pending`）— `Streams`/`Control` の field の形が変われば取り直し．
- 新規行は `plan/maintenance/tools/mutations.json` に T1・T1(c)・T2（と T3 を採るなら）ぶんを足す．`old` は実装後に exact text を取る（この段階では実装が無い）．T1 の排他 bound の行だけは今の text（`control.rs:86`）で書ける．

### 関わる他の順序

- **1（start）**: `command_instant` の receive の床は `line.stream.origin`（`timeline.rs:382`）なので，T0 と同一 instant の `cold` はそれ以上に床上げされ，timed の `e` は T0 と無関係のまま．start が失敗して line を入れ直す経路（1b-3a の mock の `Lines` 作り直しに相当）で `seq`/`held` を失わないことが要点．
- **2（stop）**: End は両 line に同じ `(e, seq)` で1回記帳される（`core.rs:126-134`）．同一 instant の `cold`/timed との順序は `seq` の順で決まり，`Stop` は held を消さない（`ur_26_a_stop_cancels_no_update`，`ur_24_stop_cancels_held_commands`）．`seq` を stream ごとにすると同じ End の seq がずれる（(c)1，(d) の項目2 と対）．
- **3（thread の panic）**: panic → `device_lost` → `Streams::end`（seq 0 の loss）で，同一 instant の `cold`/timed の booking はそれ以降 `book` されない（`control.rs:245-247`）．順序としての干渉は無いが，timed は `release()` の先頭の lost 判定（`control.rs:435-437`）で渡らない．
- **4（loss）**: loss の `seq=0` は同一 instant の変更より先（`rx.rs:966`，J60）．`release()` は lost の間なにも release しない．`held`/`colds` は残るが，以後の booking が無ければ観測されない．
- **5（設定の拒否）**: 拒否と同一 instant の timed は `ur_24_a_late_update_halted_before_it_can_act_has_no_channel`（J71）の形で「no channel」になり，enable の segment がその値を載せる（`control.rs:1145-1148`）．拒否の halt（`line.halted(t)`）が `channels_at` の判定に入る（`control.rs:382-387`）ので，`Lines` への移動でこの読みが stream の line に届くことが要点．
- **6（enable / disable）**: 0→n の configure は `control.rs:553-567` の merge（held を `(e, MAX)` まで）を通り，0 への disable は `config_at(t)` の channels 0 として timed を「no channel」にする（`control.rs:452-455`）．同一 instant の等号は T1(c)（timed が先の順序のみ merge に載る）．
- **7（受信の cut と `delivered` の床）**: switch の origin は cut + call + allowance + lead（`timeline.rs` の VH-4 項）で，投影はその origin で timed を畳む．`delivered` を stream ごとに移しても，この projection の読み（`Planned::settings`）が別 stream の値を混ぜなければ不変．
- **8（送信の clock と `version`）**: 送信の `cold` と同一 instant の timed は clock 登録（`Streams::reconcile`，`core.rs:174-194`）と `version` の増加をまたぐ．手持ちの burst の宛先 clock は `tx_clocks.last()`（`control.rs:597-600`，U22 の killing test）なので，`tx_clocks` を stream ごとに移すときに instant の対応（plan の segment との index 対応）を保つこと．
- **10（lock の順序と device 呼び出しの間に line lock を持たない）**: `book_timed` は `streams` の lock を離してから全投影の `updates` lock を取る（`control.rs:462-475`）．`ColdConfig.configure` は `updates` の lock を device call の間持ち（clock lock，`control.rs:94-102`），`release` は全投影ぶんを一括で取る．投影を stream ごとに分けるときも「その方向の全投影を一括で」という取り方を崩さないこと（(c)7）．
- **11（thread の名前と Manifest の `rejected` の `thread`）**: この順序とは干渉しない（`rejected.thread` は panic/wedge の話）．

## 10. lock の順序と，device の呼び出しの間に line の lock を持たないこと

規則（この段階で変えない）：`updates`（各 segment の `ColdConfig.updates` の clock lock）→ line を持つ lock（今は `Core.streams` の1つの `Mutex`）→ `rec` の順で取り，**line の lock は device call の間は持たない**（`brief.md:34-36`；規範は `design/18-uhd-radio.md:244`（UR-25）の "under one lock that no thread holds across a device call"；記録は `plan/maintenance/23-one-line-per-stream.md:106`；複数 stream でも1 lock は D15，`plan/phase8/33-channels-and-streams.md:190-192`）．今は `Mutex<Streams>` が `lines`（`crates/ezsdr-radio-uhd/src/provider/core.rs:60`）と `configs`・`tx_clocks`・`delivered`・`idle`・`seq`・`ended`・`stop`・`version`（`core.rs:57-81`）を同じ guard で覆い，clock registry と event sink の lock はその内側で取る（`23-one-line-per-stream.md:106`）．`held`（`core.rs:268`）は今は必ず単独で取る．

### (a) 既存のテスト

- `crates/ezsdr-radio-uhd/tests/fake.rs:3602` `ur_25_a_configure_overlapped_with_a_timed_update_to_the_same_projection_finishes`（R01，doc は `:3596-3600`）．3 つの lock のうち `updates` を device call をまたいで持つ側を，唯一 end-to-end で固定する．土台は `crates/ezsdr-radio-uhd/src/host.rs:1098` `hand_off_the_clock_does_not_advance_after_the_lock_hand_off` と `:1144` `lock_a_waiter_waits_while_the_holder_sleeps_on_the_clock`（clock lock の M2/M4 の意味）．
- `crates/ezsdr-radio-uhd/src/provider/rx/differential.rs:111` `rm_26_the_timeline_against_uhd_rx`．line の lock を uhd-rx が自分の device call の間持たないことを**構造で**固定する唯一のテスト（後述）．
- `crates/ezsdr-radio-uhd/src/provider/control.rs:996` `ur_25_cold_deadline_overflow_is_rejected_without_panic`．line の lock を持ったまま `rec` に書く経路（`Streams::book` の失敗 → `core.command_rejected`，`core.rs:99-102`・`:373-378`）を `rejected` の行で固定する．同じ形は `crates/ezsdr-radio-uhd/src/provider/rx.rs:1100` `ur_25_a_refused_segment_is_booked_by_uhd_rx` も通る．
- `crates/ezsdr-radio-uhd/src/provider/control.rs:795` `ur_14_booking_does_not_wait_for_a_future_cold_switch`．booking が device instant を待たないこと（別 thread ＋ 10 s の `recv_timeout`，`:802-808`；clock が進んでいない，`:810`）を固定する（lock を device をまたいで持ち，その device が instant を待つ形への歯止め）．
- 線の lock についての**無いもの**（実行した検索はこのとおり）：
  - `grep -rn "try_lock" crates/` → provider には無い（`crates/ezsdr-kernel/src/coordinator/state.rs:250,252` のみ）．lock の空きを直接見るテストは無い．
  - `grep -rn "lock order\|Lock order\|lock_order" crates/` → `crates/ezsdr-radio-uhd/src/provider/core.rs:55` の doc comment のみ．順序を固定するテストは無い．
  - `grep -rn "across a device call\|across device calls" crates/` → `core.rs:56`（doc）と `crates/ezsdr-radio-uhd/src/host.rs:4`（clock lock の doc）のみ．

### (b) assert と観測

- R01（`fake.rs:3602`）：manual clock（`:3604`）＋ `apply_delay` 300 ms（`FakeDevice::apply` の `self.host.sleep(apply_delay)`，`crates/ezsdr-radio-uhd/src/device.rs:1015`）で，(i) `bounded`（`fake.rs:1009-1022`）の中で 10 s 未満に Run が終わる（`:3617-3623`）— deadlock なら `bounded` の "did not finish within"（`:1020`）で fail，(ii) Manifest の `rejected` に "did not join"/"timed out" が無い（`:3624-3625`），(iii) `Descheduled::applies()`（device call のログと device host clock の開始/終了，`fake.rs:3346-3349`・`:3416-3421`）で timed の `apply` が cold の `apply` の終了後に始まり hand-off が 200 ms 以内（`:3626-3631`）．観測は device の calls と Manifest，wall time．kill は行 J294（`plan/maintenance/tools/mutations.json` の `host.rs` 側）．
- differential（`rx/differential.rs:111`）：1 000 sequence を，test thread が `rx.turn()` で回す（`:783-815`）．`Scripted` は device call の**中で同じ line を読む**（`rx_start` の `lock(&core.streams)`，`:596`）／**記帳する**（`rx_recv` → `world.advance(ready)`，`:698` → `Ev::Book` の `lock(&core.streams)`，`:504-511`）．lock を device call の間持てば同じ thread の再 lock で回らなくなる．観測は block のヘッダ（`Headers`，`:402-414`）・`applied` の `rx_stop` 行・clock records・`straddled`/`pruned`（`Observed`，`:149-171`；"booking during a wait" は 1 000 中 506，`23-one-line-per-stream.md:126`）で，lock の規則そのものは「進むこと」でしか見ていない（mutant では hang．lib test に watchdog は無い）．
- `control.rs:996`：line への book が失敗したとき（RM-26）`core.rec.rejected` に "verflow" の行がある（`:1013`）・`version` 不変（`:1014`）・`device.calls()` 空（`:1015`）．`Streams::book` が guard の中で `command_rejected`（→ `rec`）を呼ぶことの固定．
- `control.rs:795`：booking を別 thread で行い `recv_timeout(10 s)` で返ること，clock が 0 のままであること．

### (c) 壊れうる形

**c1. line の lock を device call の間持ってしまう（UR-25 の直接違反）**．「1 つの stream の面倒を 1 か所にまとめる」組み直しで出やすい箇所（今の文で）：
- `Rx::begin`（`rx.rs:269-315`）：今は plan/idle の読みを block で閉じてから（`:270-275`）`rx_open`（`:281`）・`configure`（`:282`，内部で device `apply`）・`rx_start`（`:296`）・clock 宣言（`:300-301`）を呼ぶ．`k` を導入して「stream k の状態」を1つの guard で扱うと，この block が関数全体を覆いやすい．
- `Control::book_cold` の enable from none（`control.rs:549-570`）：今は `drop(streams)`（`:552`）→ `rx_open`/`tx_open`（`:555-556`）→ `configure`（`:565`）→ 再 lock（`:569`）．per-stream 型に移すとこの drop が消えやすい（この経路は `ur_26_a_change_booked_after_the_stop_instant_ends_its_clock`，`control.rs:916` が触れる唯一の drop 窓でもある）．
- `Rx::samples` と `stop_at_cut`（`rx.rs:452-469`，`:595-614`）：`delivered` と cut の読み書きは guard の中（`:455-463`），device の `rx_stop` は guard の外（`:465-469`→`:601`）．「区切りで stream を終わらせる」を1つのヘルパーにすると stop が内側に入る．
- `Rx::finish`（`rx.rs:321-333`）：今は device call なし（clock registry と `prune` が内側なのは正しい）．ここに device call を足すと違反．
- `Tx::do_switch`（`tx.rs:643-683`）：今は `end_open(true)`（`:645`，中で `tx_send`）と reopen/configure（`:655-662`）が guard の前，`refuse` だけ temporary の guard（`:672`）．「switch の初めに plan を読む」形にすると `tx_send` をまたぐ．
- `Core::device_lost`（`core.rs:383-400`）：`end` と `lost` の store は guard の中，`drop`（`:390`）の後に `mark_lost`（`:392`）と emit（`:394`）．順を変えると device call が内側に入る．
- `Provider::stop`（`mod.rs:494-506`）：end は guard の中（`:494-500`），`TxCmd::Stop` の送信と join は外（`:501-506`）．
- `Control::release`（`control.rs:434-481`）：`projections()`（`:462`，内部で streams を取り離す）→ `updates` 一括 lock（`:464`）→ device `apply`（`:466`）．device call をまたぐのは `updates` で正しい（`23-one-line-per-stream.md:106`）．ここに streams を足すと二重違反．
- 観測される差：manual clock では guard を待つ participant は busy（M2，`design/18-uhd-radio.md:283`）なので clock が止まり，device call 側（clock wait か `apply_delay` の sleep）が返らず deadlock になる．fake.rs 系は watchdog 120 s（`crates/ezsdr-radio-uhd/tests/common/watchdog.rs:24-27`）か `bounded` で fail/kill，lib test（differential）は hang（`mutate.py:140-146` が 180 s timeout を kill と数える）．ただし fake.rs の provider テストの多くは default の real clock（`FakeConfig::default()` の `clock: ClockMode::Real`，`device.rs:381`）で，lock を持っても deadlock しないので素通りする．

**c2. lock が2つになる／state が別 lock に分かれる**．D15 は「instance の全 line は1つの lock」．`Lines` は複数 line をまとめて扱う操作を持つ（`prune` は全 line，`crates/ezsdr-radio/src/timeline.rs:495-519`；`book_move` は2本，`:533-604`）ので，stream ごとの lock に割ると「instant を book する lock の下で取る」という `book_cold` の前提（`control.rs:509-511`）と `prune` の `configs.retain`（`core.rs:139-145`）が2つの lock にまたがる．instance 全体の `seq`・`ended`・`stop`・`version`・`lost` を line と別 lock にすると，`Tx::read` の version と plan（`tx.rs:160-172`），`book_cold` の seq と book（`control.rs:512-513`），`device_lost` の end と lost（`core.rs:384-389`）が対でなくなる．観測は単一 thread のテストでは出ない．

**c3. 順序の逆転**．`streams` を持ったまま `updates` を取る（例：`book_cold` の `segment_config`（`control.rs:508`，内部で `ColdConfig::book`→`updates`，`:74-76`）を guard の内側へ，`Rx::begin` で guard を持ったまま `Planned::configure`（`:282`→`control.rs:94-102`））．逆向きは今どこもしないので潜在違反で，`updates` が device call をまたぐ（clock lock）ことと組になって初めて deadlock になる．`held`（`core.rs:268`）を guard の内側に入れるのも同じ種類（今は単独，`control.rs:670`・`:689`，`tx.rs:528-538`・`:580-589`）．決め打ちの `0` が `Rx`/`Tx` に残ると，stream k の thread が stream 0 の guard の下で別 line を触る（＝lock と値の取り違えが同時に起きる）．

### (d) 捕まえるか・足す案

- c1 のうち **uhd-rx の `rx_recv`/`rx_start`** は differential が hang として捕まえる（弱い：lib test に watchdog が無く，kill は 180 s timeout か人間の気づき）．**`rx_stop`・`book_cold` の from-none・`do_switch`・`device_lost`・`stop` は捕まえない**（既存の enable テスト `fake.rs:1516`・`:1535` は real clock，`ur_21_a_descheduled_...`（`:3456`）は hold が real sleep なので clock wait の deadlock にならない）．
- c2・c3 は**捕まえない**（cycle が無ければ順序は観測できない）．緩和は D15 の1 lock の維持とレビューで，以下はそのうち「device call の間 line の lock が空いている」ことを直接見る案．
- **足すテストの案（新規，lib unit．deterministic で，同 thread の `try_lock` は deadlock しないので assert で clean に fail する）**：
  1. `ur_25_the_line_lock_is_free_inside_every_receive_device_call`（`crates/ezsdr-radio-uhd/src/provider/rx.rs` の tests．`test_support::rig_with_links`＋test 内の spy `Device` で `Arc<Core>` を `Weak` 越しに持ち，`rx_open`・`rx_start`・`rx_stop`・`rx_recv` の入口で `assert!(lock が取れる)`（`core.streams.try_lock().is_ok()`））．既存単体テストと同じく `rx(&core, …)`＋`poll`/`receive`/`begin`/`finish` で駆動する．
  2. `ur_25_the_line_lock_is_free_inside_every_transmit_device_call`（`provider/tx.rs` の tests．`tx_open`・`tx_send`・`tx_async` の入口で同じ assert）．
  3. `ur_25_the_line_lock_is_free_inside_every_control_device_call`（`provider/control.rs` の tests．`book_cold` の from-none 経路の `rx_open`/`apply` と `release` の `apply` の入口で同じ assert）．
  4. （推奨・任意）`ur_25_a_booking_overlapped_with_an_enable_from_no_stream_finishes`（`tests/fake.rs`，R01 と同型：manual clock＋`apply_delay` 300 ms，`radio.rx.channels = 0` から `= 1` の enable と timed gain 更新を重ね，`bounded` で終わる・`rejected` が空を assert．c1 の from-none を end-to-end で押さえる）．
- **mutation の行の案**（`old` は組み直し後の文に合わせて書く．現行の文での sketch．`crate: ezsdr-radio-uhd`，`test_bin: lib`，filter は新しいテスト）：
  - A：`rx.rs::begin` で guard を enable/switch の device call をまたいで持つ．`old: "        let Some(next) = next else { return false };\n        let (origin, channels) = (next.segment.origin, next.channels());"` → `new:` その間に `let _held = lock(&self.core.streams);` を足す．kill は 1．
  - B：`rx.rs::samples` で cut の読みの後 `rx_stop` まで guard を持つ．`old: "        if let Some(cut) = stream.cut.filter(|_| ended && !stream.stopped) {"` → `new:` 先頭に `let _held = lock(&self.core.streams);`．kill は 1（`rx_stop`）．
  - C：`control.rs::book_cold` の from-none で `drop(streams)` を外す（`old: "            // Its owner is idle: nothing it could prune comes meanwhile.\n            drop(streams);\n"`）．kill は 3（`rx_open`/`apply`）．現行文ではその後の再 lock（`:569`）が自己 deadlock になるので，spy の assert が先に panic する必要がある．
  - D：`tx.rs::do_switch` の先頭で guard を取る（`old: "        let Some(switch) = self.switch.take() else { return };\n        self.end_open(true);\n"`）．kill は 2（`tx_send`）．
  - E（任意）：`core.rs::device_lost` で `mark_lost` を guard の中へ（spy が `mark_lost` も見る場合）．
  - 順序（c3）そのものの行はこの段階では kill できない（1 mutant では cycle を作れない，上記）．無理に作らず「equivalent にしない」で残すか，入れない．
- lock の箇所を通る既存行（J15・J16・J52・J53・J64・R12・R13・U38・U40 など）は `old` が動くので当て直しが要る（一覧と id は「計画の最後に書くこと」の担当．1b-3a も 28 行を当て直した，`plan/phase8/00-overview.md:161`）．

### 関わる他の順序

- **1（start）**：`Control::new` が line と `tx_clocks`/`idle` を guard の中で入れてから thread を spawn する（`control.rs:216-221`，`mod.rs:469-477`）．stream ごとの型でも「全 line を入れてから起こす」順を崩すと，thread の最初の turn が空の `Lines` を引いて `Lines::line`/`line_mut` の panic（`timeline.rs:638`）か clock 欠落になる．
- **2（stop／Drain・Abort）**：`Provider::stop` と `device_lost` が別 thread から同じ line に `End` を記帳する（`mod.rs:494-500`，`core.rs:383-390`）．片方だけ別 lock にすると二重記帳か loss 前の end 読みになる（item 2 のテストは件数を見るので，順を崩すと deadlock か件数違いで出る）．
- **3（panic）**：panic は thread 名で `rejected` に 1 行足す（`mod.rs:238-244`）．panic 時に line の lock を持っていると poison するので，新しい lock も `lock()` の poison 回復（`core.rs:22-24`）を同じように通す必要がある（名前を stream ごとにする 1b-4c と同時に確認）．
- **4（loss）**：「end と lost は guard の中，`mark_lost` は外」の順（`core.rs:384-392`）を崩すと，全 stream が 1 回だけ終わる保証（item 4）が崩れる．同時に，loss を見つけた thread と booking する uhd-control が同じ lock を待つので順序の同じさが要る．
- **5（設定拒否 RM-25）**：拒否の記帳は device call の戻り値の後，短い temporary の lock で行う（`rx.rs:286`，`tx.rs:672`）．`begin`/`do_switch` の1つの guard にまとめると拒否経路も device call をまたぐ（item 5 のテストは単一 thread なので気づかない）．
- **6（0→n の enable／0 への disable）**：enable は唯一 lock を途中で離す場所（`control.rs:552-569`）．ここを持ったまま open/configure すると `ready`/origin の値は同じでも deadlock 経路になる（manual clock のテストでだけ出る）．`tx_open(0, &[])` の経路も同じ関数．
- **7（delivered の床）**：`delivered` は block の plan 読みと1つの guard の中で書き（`rx.rs:452-464`），`book` が同じ guard の中で読む（`core.rs:93-97`）．stream ごとの型に移すのはよいが別 lock にすると床の対（segment と k）が崩れ，item 7 のテストは単一 thread なので気づかない．
- **8（tx_clocks・version）**：`Tx::read` は `version` と plan を1回の guard で読む（`tx.rs:160-172`）．`version` だけ別 lock にすると，version と plan が対でない組を読む turn ができる（item 8 のテストは片方ずつしか見ない）．
- **9（同 instant の cold と timed，instance 全体の seq）**：`book_cold` は instant を取り，config を入れ，book するまでを1つの guard で行う（`control.rs:509-513`）．`seq`（instance 全体）を別 lock にすると RM-25 の床の順序（`core.rs:130`，`control.rs:304`・`:422`・`:512`）が崩れる（item 9 のテストは単一 thread）．
- **11（thread 名と rejected の thread）**：名は spawn 時の `&'static str`（`mod.rs:240`）で今は `uhd-rx`/`uhd-tx` のまま（`brief.md:31`）．stream ごとの lock を入れると「どの thread が記帳したか」が stream ごとに分かれるが，名前と `rejected` の形はこの段階では変えない（item 11 のテストと mutation は不変）．

## 11. thread の名前と，Manifest の `rejected` の `thread`

この順序の出どころ（組み直しが触る場所）:
- thread は `start` が `UhdRadio::spawn` で起こす（`crates/ezsdr-radio-uhd/src/provider/mod.rs:474-477`）．名前は "uhd-rx"・"uhd-tx"・"uhd-control" の3つ（mod.rs:232-251）．
- 名前は (i) `HostClock::spawn` を通って OS の thread 名になり（`crates/ezsdr-radio-uhd/src/host.rs:383-392`，`Builder::new().name(name)` は :386），(ii) `rejected` の行になる：panic は `{"thread": name, "reason": "panicked"}`（mod.rs:240），join の 1 s タイムアウトは `{"thread": name, "reason": "did not join within 1 s; left detached"}`（mod.rs:265）．`uhd-control` だけが device loss を出さない（mod.rs:241-243）．detach は `detached: BTreeSet<&'static str>`（mod.rs:90，:266）に名前で入り，cleanup の漏れの行 `{"leaked": "streamers", "because": […]}`（mod.rs:520-522）に現れる．section は cleanup で `rec.rejected` から書かれる（mod.rs:270-282，:279；core.rs:349-351）．
- 1b-4b は thread を stream ごとにするが名前は今のまま（stream ごとの `uhd-rx-s<k>` は 1b-4c：`plan/phase8/33-channels-and-streams.md:190-192`）．つまりこの順序は「名前と役割の対応が1つもずれないこと」が対象．

### (a) 既存のテスト

- `ur_16_a_panicking_thread_is_a_lost_device`：`crates/ezsdr-radio-uhd/tests/fake.rs:894`（thread の assert は :902）．
- `ur_16_a_wedged_thread_does_not_wedge_stop`：`tests/fake.rs:920`（helper `wedged` は :906-917，thread の assert は :924）．
- `ur_16_cleanup_keeps_the_streamer_of_a_detached_thread`：`tests/fake.rs:928`（:931-932）．
- 規範は UR-16（`design/18-uhd-radio.md:200`）と test 表（:366-368），UR-30 の `rejected` の内容（:264）．
- 名前を観測するのはこの3つだけ．`src/` の unit test は thread を起こさない（`src/provider/control.rs:802` は無名の `std::thread::spawn`，`src/provider/rx.rs:205` は `run()` を直接呼ぶ）．確認：`grep -rn '"thread"' crates/ezsdr-radio-uhd` → fake.rs:902，fake.rs:924，mod.rs:240，mod.rs:265 の4件のみ．

### (b) assert と観測

- ① panic（fake.rs:894）：`FakeFault::RxPanic` 2.2 ms（:896）で fake の `rx_recv` が panic（device.rs:1245）→ uhd-rx の body の panic を `spawn` の `catch_unwind` が受け（mod.rs:238），`{"thread": "uhd-rx", "reason": "panicked"}` を書く（mod.rs:240）．名前が "uhd-control" でないので `device_lost`（mod.rs:241-243）．観測は (i) Manifest の section `ezsdr.radio.uhd.rejected`（`tests/common/mod.rs:376-378`）を `r["thread"] == "uhd-rx"` で走査（:902），(ii) `termination.reason == Stopped { cause: Policy { DEVICE_LOST } }`（:901），(iii) device の calls の順（`assert_marked_lost_before_closed` :886-891，:903）．
- ② wedge（fake.rs:920）：`FakeFault::TxBlocks`（:908）で `tx_send` が永久に返らない．`Provider::stop` の `join("uhd-tx")`（mod.rs:504）が `JOIN_BOUND` 1 s（mod.rs:57，:257-260）で打ち切り，`{"thread": "uhd-tx", …}` を書き（mod.rs:265）`detached` に入れる（:266）．観測は `run.finish()` の wall 時間（<3 s，:923）と rejected の行（:924）．real clock の rig（`fake_real`，common/mod.rs:307-309）なので，block した thread が manual clock を凍らせない．
- ③ cleanup（fake.rs:928）：同じ rig．`detached` が空でないので `close_streams` を呼ばず（mod.rs:519-525），漏れの行を書く（:522）．観測は device の calls に `close_streams` が無いこと（:931）と `leaked` を持つ行（:932）．

### (c) 壊れうる形

1. **方向の取り違え（spawn 側）**：per-stream のループが受信 stream の thread に "uhd-tx" を渡す（逆も）．
2. **名前の一本化**：名前を返す helper が両方向に同じ定数を返す（"uhd-rx"／"uhd-tx"／"uhd-control"）．
3. **join 側の取り違え**：spawn と違う名前で `join` する（見つからなければ note も残らず黙って return：mod.rs:255）．
4. **役割判定の置き換え**：`if name != "uhd-control"`（mod.rs:241）を別の判定（`k`，`dir`，フラグ）にする；control の spawn 名と join 名の対応（"uhd-control" の行・join）がずれる．
5. **記帳の取り違え**：行の `"thread"` を別の出どころ（stream key，dir 名，per-stream の field）から取る；note の行き先を instance 全体の `rec` から stream ごとの records に移し，`write_sections`（mod.rs:279）に行が届かない；`"thread"` key の改名；逆に `thread` を持たない行（拒否の行，core.rs:373-378）に `thread` を足して Manifest の形を変える．
6. **thread の存在**：per-stream のループを「今ある line」だけで回し，stream の無い方向に thread が起きない（UR-15 は「stream がなくても起きる」：design/18:198；link の無い受信側では `Streams.lines` が `None`：core.rs:60）．
7. **名前の先取り**：1b-4c の `uhd-rx-s<k>` を今入れる．
8. **detach の取り違え**：`detached` に入る名前を誤る（漏れの行の `because` が誤る．close_streams を呼ばない判断は非空なら保たれる）．

### (d) 捕まえるか・足す案

- 1 は捕まる：rx 側は :902 が "uhd-rx" を要求．tx 側は spawn 名を誤ると `join("uhd-tx")`（:504）が引けず（mod.rs:255）:924 に行が出ない（このときは :931 も落ちうる）．どの取り違えでも少なくとも一方が落ちる．
- 2 は捕まる：定数 "uhd-rx" なら :924，定数 "uhd-tx" なら :902，"uhd-control" なら両方と :901（guard により loss も出ない）．
- 3 の tx 側は捕まる（行は「join が実際に待った thread の名前」なので，:924 の行は `join("uhd-tx")` が本物の wedge した thread を引いたときだけ出る）．**rx 側は観測不能**：1 s を超えて uhd-rx を wedge させる fault が無い（`grep -rn "with_rx_stall" crates/ezsdr-radio-uhd` → mod.rs:223，fake.rs:3647，common/mod.rs:591 の3件で，いずれも 300 ms．永久に block するのは `TxBlocks` だけ：device.rs:280，:1391）．足す案は出さない（通すには「`rx_recv` を永久に block する FakeFault」＋ fake.rs のテストが要る＝device.rs の変更になる．死角として記録し，mutation の行は作らない）．
- 4 の「stream の panic が loss を出す」半分は捕まる（:901）．**「control の panic が loss を出さない」「`thread == "uhd-control"` で記録される」半分は誰も観測しない**（control を panic させる fault が無い：`grep -n 'panic!' crates/ezsdr-radio-uhd/src/device.rs` → :1245 の `RxPanic` だけが fault，他はテストコード；`grep -rn 'uhd-control' crates/ezsdr-radio-uhd/tests` → コメントのみ）．足す案：control の device 呼び出し（`time_now` など）を panic させる `FakeFault` を足し，「`rejected` に `thread == "uhd-control"` と `reason == "panicked"` の行があり，`DEVICE_LOST` が出ない」ことを assert する．device.rs を触るので採否は orchestrator（この段階で足すか，1b-4c へ送るか）．
- 5 の「行が消える／値が変わる」半分は :902・:924 が捕まえる．`reason` の文面，`because` の内容，key の改名は assert されていない（例：mod.rs:265 の理由を "left detached" に縮める行は生存）．「テストは変えない」と衝突するので，既存 assert の強化か新テストは orchestrator の判断として案だけ残す．
- 6 はこの項目では観測しない（1・6 の担当．存在だけなら fake.rs:1500 が tx.channels=0 からの Session で uhd-tx の存在を，fake.rs:1516 が受信 enable を間接に押さえる）．
- 7 は捕まる（別の名前なら :902 か :924 が落ちる）．この段階の「名前を変えない」はテストが固定している．
- 8 の close_streams を呼ばない半分は :931（既存の行 U42 が `plan/phase7/tools/mutations.json:876-885` で `if !self.detached.is_empty()` を壊す）．`because` の誤りは観測されない（低優先）．

新規の mutation の行の案（id は `plan/maintenance/tools/mutations.json` の現在の最大 P241 の次から．`old` は今の文．組み直し後は同じ site に同じ規則・同じ形で当て直す）:
- a. mod.rs:474 `self.spawn(&core, "uhd-rx", move || rx.run())?;` の名前 → `"uhd-tx"`．`fake` / `ur_16_a_panicking_thread_is_a_lost_device`．
- b. mod.rs:476 の名前 → `"uhd-rx"`．`fake` / `ur_16_a_wedged_thread_does_not_wedge_stop`．
- c. mod.rs:504 `self.join(&core, "uhd-tx");` → `"uhd-rx"`．`fake` / `ur_16_a_wedged_thread_does_not_wedge_stop`．
- d. mod.rs:241 `if name != "uhd-control" {` → `if false {`．`fake` / `ur_16_a_panicking_thread_is_a_lost_device`．
- e. mod.rs:265 の `"thread": name` → `"thread": "uhd-control"`（記帳が thread を名指ししない）．`fake` / `ur_16_a_wedged_thread_does_not_wedge_stop`．
- f. mod.rs:265 の `reject_note` の行を削除（timeout を記録しない）．`fake` / `ur_16_a_wedged_thread_does_not_wedge_stop`．
- g. mod.rs:266 `self.detached.insert(name);` を削除．`fake` / `ur_16_cleanup_keeps_the_streamer_of_a_detached_thread`（U42 と裏表なので，どちらか一方でよい）．
- h. 生き残る形（テストが無い）：mod.rs:241 を `if true {`（control の panic），`"reason": "panicked"` の文面替え．4・5 のテストを足すなら一緒に流す．
- 既存の行の当て直し見込み（この項目の範囲）：`plan/phase7/tools/mutations.json:876-885` の U42 だけ．`detached`・`if !self.detached.is_empty()` の形を変えなければそのまま一致し，変えるなら同じ規則（detached がある間 close_streams を呼ばない）に当て直す．spawn・join・`"thread"` の記帳には既存の行が無い（`grep -rn 'panicked\|did not join\|uhd-control' plan/maintenance/tools/mutations.json plan/phase7/tools/mutations.json` で該当なし）．

### 関わる他の順序

- 1（`start`）：名前は spawn の site で決まる．per-stream のループが「今ある line」だけを回ると，stream の無い方向の thread が消える（UR-15，design/18:198）——私のテストはこれを捕まえない．
- 2（`stop`）：tx の timeout の行は stop の join から出る．join の順序（control → 送信 → 受信，RM-16）と 1 s の bound は 2 が，名前の一致は 11 が押さえる．名前が一致しないと join は黙って skip する（mod.rs:255）ので，両者の食い違いが静かに残りうる．
- 3（thread の panic）：同じ `catch_unwind`（mod.rs:238）と `device_lost`（:242）．3 が「全 stream が1回で終わる」を，11 が「どの名前で記録されるか」と control の例外を押さえる．行の案（catch_unwind を外す行など）は重複させず片方に寄せる．
- 4（loss を見つけた thread）：その記録は `timing` の `device_lost` 行（core.rs:393）で，`rejected` の名前は持たない．per-stream 化で `device_lost` の記帳（`ended` は instance 全体）に `thread` を足したり，名前の行に変えたりしないこと．
- 5（device の拒否）：拒否の記帳（core.rs:373-378）は `kind`／`reason` の行で `thread` を持たない．per-stream 化でここに `thread` を足すと Manifest の形が変わる（テストは見ていない）．
- 6（stream なしの enable）：thread は enable ではなく `start` で，方向ごとに必ず起きる（design/18:198）．存在が欠けると fake.rs:1500／:1516 が落ちるが，名前側の誤りは 6 では見えない．
- 10（lock）：note は `rec` の lock だけを取る（core.rs:349-351）．instance 全体の `rec` を stream ごとに割ると `write_sections`（mod.rs:279）に行が届かず ①②が落ちる．lock の下での panic は `lock()` の poison 回復（core.rs:22-24）に依存するので，その形も保つこと．

## final. 計画の最後に書くこと

### (a) provider の中で stream の番号 0 を決め打ちしている箇所と，どれを `k` に置き換えるか

検索（`crates/ezsdr-radio-uhd/src/provider` で実行。結果からテストの行を除いた）:

```
grep -n -E 'device\.(rx_recv|rx_open|rx_start|rx_stop|rx_packet_samples|tx_open|tx_send|tx_async)\(0[,)]' core.rs control.rs rx.rs tx.rs mod.rs
```

テストの行として除いたもの: control.rs:937, 977, 1088, 1113, 1158, 1183, 1206 / rx.rs:657 / tx.rs:834, 867, 902, 921, 930, 954, 985, 1017, 1042, 1088, 1108, 1125, 1149, 1204, 1228, 1251, 1331, 1355。
残る **16 箇所**の判断:

**Rx の thread — `self.k` に置き換える**（brief の「device の呼び出しは，決め打ちの `0` ではなく `k` を使う」）:
- rx.rs:241 `rx_recv(0, …)`（受信の待ち）
- rx.rs:281 `rx_open(0, …)`（channel 数の変わった segment の reopen）
- rx.rs:296 `rx_start(0, origin)`（segment の timed start）
- rx.rs:424 `rx_start(0, restart)`（`LateCommand` の restart）
- rx.rs:601 `rx_stop(0, None)`（cut での untimed stop。`stop_at_cut` に k を渡す）

**Tx の thread — `self.k`**:
- tx.rs:359 `tx_send(0, &tail, …)`（継続へ置き去りにした 1 sample）
- tx.rs:373 `tx_send(0, &sent, …)`（本送信）
- tx.rs:487 `tx_send(0, &buffers, …)`（device burst の終わり。`close_device_burst` に k を渡す）
- tx.rs:657 `tx_open(0, …)`（switch の reopen）
- tx.rs:688 `tx_async(0, timeout)`（非同期 report の読み）

**uhd-control — 宛先の stream の番号 `k`（今は 0）に置き換える**。line を `(dir, k)` で引くのと同じ k を使う:
- control.rs:555 `rx_open(0, &list)`（UR-25 の from-none enable）
- control.rs:556 `tx_open(0, &list)`（同じ）

**`UhdRadio` — `k` に置き換える**（`Prepared` の受信・送信を `(k, …)` の列にし，thread を stream ごとに起こす（D15）のに合わせる。今の列は 1 本）:
- mod.rs:405 `rx_open(0, …)`（`arm` の受信 streamer）
- mod.rs:409 `tx_open(0, …)`（`arm` の送信 streamer）
- mod.rs:459 `rx_start(0, t0)`（最初の受信 segment の timed start）
- （`Prepared` を今の 1 本のまま残す実装ならこの 3 箇所は 0 のままでよい。そのときは (b) の U11 も無傷。どちらでも挙動は同じ）

**0 のまま残す — 1 箇所**:
- control.rs:194 `rx_packet_samples(0)` — `Control::new` が作る受信 line 1 本（stream 0）の `call` の項。ここには k を取る文脈が無く，`k` にしても値は 0 で新しい情報がない。4c が stream ごとに line を作るときにその stream の `rx_packet_samples(k)` にする（design/18:244 の UR-25 の文面も今は `rx_packet_samples(0)`。`rx_recv(0, …)`・`rx_start(0, T0)` も design/18:205・198 にあるが，コードが `k`（=0）を渡しても今の意味は同じ）。
  - 実装が `Control::new` の line 組み立てを stream ごとに書き換えるならこの限りではない（そのときは (b) の B15・B22 を当て直す）。

補足（`0` の literal は無いが，4b で「宛先 stream 0」を表すことになる箇所。line の参照と組で 1 つの k にする）:
- control.rs:305（Stop の記帳），322-324（start_rx: `command_instant`・`configs`・`book`），513・539・548（cold 変更の line と idle），572-577（`configs` の insert・`book`・失敗時の remove），599-600（burst が読む最後の送信 clock）。
- core.rs:92-105（`book`），110-122（`refuse`），139-145（`prune`），148-159（`planned`），175-194（`reconcile`），197-207（`record`）は内部で方向の line を引く箇所——引数を `(dir, k)` にする。

注意: `0..channels`・`0..*channels`・`Channels::first`（rx.rs:281，tx.rs:657，mod.rs:405・409，control.rs:199・571，rx.rs:79）は **channel の番号**であって stream の番号ではない。同じ grep に出るが置き換えの対象ではない（layout の channel 列になるのは 4c）。

### (b) 当て直しが要りそうな既存の mutation の行の見込み

前提: provider の行は `plan/maintenance/tools/mutations.json` に 53 行（以下 **M**），`plan/phase7/tools/mutations.json` に 81 行（以下 **P7**），計 134 行。4b は provider だけを触るので，`crates/ezsdr-radio/src/timeline.rs` の 62 行，`crates/ezsdr-radio-uhd/src/device.rs` の 54 行，`tests` の 1 行は無傷の見込み（timeline の注意は下）。
前例: 1b-4a は device 境界に stream 番号が入ったとき，`old` に呼び出しの全文を持つ B01・B15・B20・B22・B28 を `0,` を足して当て直した（05e8d2f）。今回はその `0` が `k` になるので同じ行がまた当たる。

**ほぼ確実**（`old` が，4b が必ず書き換える構文（`lines[`・`configs[`・`.tx_clocks`・`.delivered`・device 呼び出しの `0`）を含む）:

| id | list | path:line | 何が変わるか |
|---|---|---|---|
| J21 | M | core.rs:187 | `self.tx_clocks` のループ（`reconcile`）。tx_clocks が stream ごとになる |
| J46 | M | core.rs:97 | `book` の `delivered` の床。stream ごとの delivered を引く形になる |
| J54 | M | core.rs:97 | 同じ行（床が cut を見る形） |
| J51 | M | core.rs:144 | `self.configs[dir as usize].retain`（`prune`）。configs が stream ごとになる |
| J64 | M | control.rs:383 | `lines[dir as usize]`（`channels_at` の halt 判定）。line を `(dir, k)` で引く |
| U22 | P7 | control.rs:600 | `streams.tx_clocks.last()`（burst の current clock） |
| B01 | P7 | rx.rs:241 | `rx_recv(0, …)` → k |
| B20 | P7 | tx.rs:353-373 | `tx_send(0, …)` が 2 つ（継続の送り）→ k |
| B28 | P7 | tx.rs:359 | `tx_send(0, &tail, …)` → k |

**濃厚**（stream の key を引数に取るようになる呼び出しを含む。呼び出しの形は実装の選び方で決まる）:

| id | list | path:line | 何が変わるか |
|---|---|---|---|
| J15 | M | rx.rs:286 | `refuse(&self.core, Dir::Rx, by)` に k が入る |
| J49 | M | rx.rs:332 | `streams.prune(Dir::Rx, by)` に k が入る |
| J52 | M | rx.rs:455-456 | `stream.follow(&streams)` が k（か line）を取る |
| J53 | M | rx.rs:324 | 同上（`finish`） |
| R12 | P7 | rx.rs:193-194 | 同上（`Rx::follow`） |
| J16 | M | tx.rs:672 | `refuse(&self.core, Dir::Tx, by)` に k |
| J50 | M | tx.rs:164 | `streams.prune(Dir::Tx, by)` に k |

**構造の選び方次第**（決めれば当て直しの有無が決まる）:

| id | list | path:line | 何が変わるか |
|---|---|---|---|
| J11 | M | control.rs:299-300 | `self.to_tx.send(TxCmd::Stop)`。送信の sender を stream ごとの列にすれば当て直す（1 本のままなら無傷） |
| U11 | P7 | mod.rs:412 | `arm` の送信 clock の登録。`Prepared` の tx を `(k, …)` の列にすれば行が動く |
| B15 | P7 | control.rs:194 | (a) の 1 箇所を k にすれば当て直す（0 のままなら無傷） |
| B22 | P7 | control.rs:194 | 同上 |
| J62 | M | core.rs:113 | `refuse` の match の腕（`self.changed(core, dir);`）。書き換えの形が変われば動く |

**無傷の見込み**（念のため）: `device_lost` の行（B33 = core.rs:392，J47・J58 = core.rs:388，U29 = core.rs:394）と `Streams::end` の seq の行（J60 = core.rs:130）は `end` の署名を変えない限り一致し続ける。mod.rs の UR-5・UR-6 の検査（U01〜U03，J36，J44，J66，P13，R22，R23，R26，R27），`from_binding`・`prepare` の行（J174，J277，U08，U09，U12，U42），control の `check_device`（B35・B39・B40・U27・U43），`book_burst` の本体（J197・J315・C02・U14・U15）も触らない見込み。

注意（timeline.rs）: UHD の prune は今，その方向の line にだけ効く（core.rs:139-145）。一方 `Lines::prune` は全部の line を畳む（timeline.rs:495-519）ので，これをそのまま使うと他方向の将来の拒否まで消える（`Line::prune` の前提，timeline.rs:322-326）。stream ごとの prune が要るなら **既存の `Lines` の行は触らず足すだけ**にする。触ると `Lines` の中の 26 行（timeline.rs:419-724 の P94・P111・P102・P108・P103・P98・P109・P99・P114・P92・P81・P115・P100・P101・P88・P87・P84・P83・P113・P96・P97・P105・P112・P110・P82・P91）も当て直しになる。

確認: 当て直したら，その行が同じ規則を同じ形で壊し，`filter` のテストで殺せることを `remote_mutate.py` の該当 id だけで確かめる（AGENTS.md §7 の修正の回の規則。各行が 1 回だけ一致すること）。

### (c) 2 本以上の stream でしか出ない誤り（1b-4c へ送る一覧）

1. thread の名前と join: `Running.threads` は `(&'static str, JoinHandle<()>)`（mod.rs:71），`spawn` の panic 記録は `{"thread": name}`（mod.rs:240），`join` は名前で 1 本だけ引く（mod.rs:253-266）。同名の 2 本目は join されず（detached にも記録されず）残り，Manifest の `rejected` はどちらの stream の panic か分からない（テストは fake.rs:902・924 が名前を見る）。per-stream の名前（D15 の `uhd-rx-s<k>`）と UR-16 の記録の形は 4c で決める。
2. stream の node と event の持ち主: `rx_id`・`tx_id` は方向ごとに 1 つ（core.rs:287-288），timed start の SampleClock の宣言も方向の node（rx.rs:300-301，core.rs:474）。2 本目の stream に node が無く（D4 の `rx/s<k>`，plan/phase8/33:40-41），Stop の宛先（control.rs:296），`start_rx`（control.rs:313），`LATE_COMMAND`・`ALIGNMENT_ERROR` の source（rx.rs:422，rx.rs:533）が 2 本目を名指しできない。
3. RX_OVERFLOW の handle: `Core.overflow` は受信 node 1 つの handle（core.rs:261，292-294。使うのは rx.rs:527）。D15 の「hot path の handle は stream ごと」が 2 本目に無い。
4. `Streams::end` が各方向 1 本の line にしか End を記帳しない（core.rs:126-134）。2 本目の line には End が入らず，その stream は RM-16 の終わり（cut，clock の終わり）を持たない。`seq` は instance 全体で 1 回（core.rs:130）なので，全 line に同じ End を付ける形は 4c で決める。
5. End の `delivered` の床は，その End が切る stream のものでなければならない（core.rs:93-97。書く側は rx.rs:153・307・462，消すのは rx.rs:330）。1 本では「line と床の組が同じ stream」である正しさが見えない——4b の後も `book` の呼び出し側が同じ `(dir, k)` を両方に使うことを 2 本目で確かめる。
6. line と stream ごとの状態の作成が方向ごとに 1 本分しかない（control.rs:208-221）。2 本目以降は line が無く，`Lines::config_at`・`Lines::book` は「その stream の line が無い」で panic する（timeline.rs:628-639）。`idle`・`tx_clocks`・`configs` の入れ物も 1 本分しか作られない。
7. `planned(dir)` は方向の 1 本の line を返す（core.rs:148-159）。`Rx::begin`（rx.rs:272）・`Tx::read`（tx.rs:169）が 2 本の line の segment を混ぜる。
8. `idle` と `channels_at` は方向ごとの 1 本目を見る（control.rs:220，383-387，548。rx.rs:273。tx.rs:171）。2 本目の halt・idle が 1 本目に混ざり，from-none の open（control.rs:555-556）も宛先を取り違えうる。
9. channel の並びが `0..n` 前提（core.rs:443-445 の `apply`，rx.rs:281・tx.rs:657・mod.rs:405・409 の open）。layout の stream の channel は 0..n ではない（RM-27）。同じく `Channels::first(count)` で集合を作る箇所（control.rs:199・571，rx.rs:79）と `Planned::channels()` が数だけを返すこと（control.rs:121-123）。
10. uhd-control の宛先の選択が方向だけ（control.rs:395・503 の key→dir，597-601 の burst の current clock。timed の footprint と release は control.rs:415-427・451-479）。D14 の `rx/s<k>`・`tx/s<k>` 宛てが 2 本目を選べない。
11. UR-30 の記録が方向ごとの plan（core.rs:197-207）。stream が増えると segment がどの stream のものか記録から消える（Manifest の形の判断）。
12. `rx_packet_samples(0)` の `call`（control.rs:194）。2 本目の受信 stream の `call` が 1 本目の packet 長になる。

### 関わる他の順序

- (b) の行は，その行が属する順序のテストで殺せることを確かめる（別の順序の副作用で死ぬと，その順序の観測を固定していないことになる）:
  - B01・J46・J54・J49・J51・J52・J53・R12 → 順序 2（Abort/Stop の End）・4（loss）・7（cut と delivered の床）。代表の filter: `ur_29_a_loss_ends_the_stream_at_its_instant`，`ur_17_an_ended_segment_prunes_the_items_before_it`，`ur_26_an_end_takes_delivered_only_from_the_segment_it_cuts`，`rm_26_the_timeline_against_uhd_rx`。
  - U22・J21・J50・J16・B20・B28 → 順序 8（送信の clock と version の読み直し）と 2（Stop が burst を終える）・5（device の拒否）。代表: `ur_25_a_burst_on_a_later_clock_survives_an_earlier_switch`，`ur_26_a_change_booked_after_the_stop_instant_ends_its_clock`，`ur_23_a_burst_booked_at_a_sent_burst_s_end_continues_it`。
  - J64・J15・J62 → 順序 5（拒否で halt）・6（channel 数 0→n の enable）・9（同じ instant の cold と timed）。代表: `ur_24_a_direction_halted_by_a_refusal_has_no_channel`，`ur_25_a_refused_segment_is_booked_by_uhd_rx`，`ur_23_a_refused_transmit_change_ends_its_clock_at_its_origin`。
  - J11 → 順序 2（`ur_26_a_stop_cancels_no_update`）。
  - U11・B15・B22 → 順序 1（`arm`・`start`）。代表: `ur_15_every_clock_starts_on_its_lattice`，`ur_25_a_cold_change_booked_anywhere_in_a_long_receive…`。
- (c) の各項目は，1〜11 のテストがすべて stream 1 本で通ってしまうもの。4c が複数の stream を配線したとき（stream 1 本ずつのテストの次）に，D15 の thread と node の形，UR-16 の記録，`Lines` の全 line の End と床の対応，宛先の選択を確かめる。

## 実装で増えた経路（2026-10-19，実装役がセルフレビュー 2 回目への対応で足した）

- **`start` が列の長さ 1 を要求する（`mod.rs`）**：裁定の構造（`Prepared` の受信・送信を stream ごとの列にする）で，今の key は各方向 1 本．列が 1 本でなければ始めず `Rejected` にする（`"UR-15: this stage starts one receive stream"` / `… transmit stream`）— 列の先頭だけを使って残りを黙って捨てない．
  - テスト：`ur_15_a_second_stream_is_refused_rather_than_half_started`（lib．`Prepared` を手で組んで rx/tx の 2 本と 0 本を試し，`Rejected` と「thread が 1 本も起きない」ことを見る）．
  - mutation の行：P280（`let [rx_stream] = …try_from(…)` を `rx_streams.into_iter().next()` にし，2 本目を黙って捨てる形）．killed（`mutate-1b4b-5.log`）．
