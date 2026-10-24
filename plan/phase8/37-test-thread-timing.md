# 37 — 試験のスレッドの遅れで落ちる fake のテスト（#69）

| 項目 | 内容 |
|---|---|
| 状態 | 決定（owner 2026-10-23，§5 の 3 つとも推奨どおり）．Sol のレビュー 2 回（§6） |
| 対象 | [#69](https://github.com/k3komatsu/Ez-SDRv4/issues/69)，`mech:wall-clock-test-oracle` の 3 件目（#10，#68，#69．[failure-mechanisms](../maintenance/failure-mechanisms.md)） |
| 調べた時点 | `main`，HEAD `c953d27` |
| 問い | AGENTS.md §6 の三振：実時間に頼る FakeDevice のテストを残すか．概念を消せるか，規則を 2 か所で実装していないか，spec の文が複雑すぎないか |

## 1. 何が起きているか

**#69 の見立ては半分違った．** #69 は「実時間（`ClockMode::Real`）の FakeDevice のテスト」と書いたが，落ちる 6 本のうち 5 本は**手動の時計**（`common::fake`，[計画 30](../maintenance/30-fakedevice-manual-time.md)）の上で動く．

- `ur_23_a_burst_booked_at_a_sent_burst_s_end_continues_it`（`fake.rs:3040`）
- `ur_23_a_stop_while_the_device_burst_waits_for_a_continuation_ends_it`（`:3078`）
- `ur_23_a_burst_one_sample_after_a_burst_is_played`（`:3145`）
- `ur_23_a_continuation_whose_held_sample_is_not_taken_is_abandoned`（`:3402`）
- `ur_23_a_continuation_s_first_send_that_takes_nothing_ends_the_device_burst`（`:3438`）
- `ur_21_a_descheduled_timed_send_is_on_time_on_the_manual_clock_and_late_on_the_real_one`（`:3744`．手動と実時間の両方を流す）

**再現**（orchestrator の観測，2026-10-23，usrp-lnx03 16 コアに 16 本の busy loop，`fake` を 3 回．ログは残していない）：3 回とも 4〜6 本が落ちた．負荷なしでは 0．panic の文は次の 3 つだけだった．

- 予約が予約の時点で遅れとして捨てられる（`TIME_ERROR { cause: late, outcome: drop }`，`late_by_ns` 2〜25 ms）．
- 「case was not reached」の回は，送信の記録が B の 1 000 sample だけで，40 ms 先に予約した A が捨てられていた．
- `ur_21` の実時間の回は 30 ms 先の予約が捨てられ，`tx_send` に入らなかった．

**原因**（コードと計画 30 で VERIFIED．負荷の下での個々の失敗の筋は上の panic の文からの INFERRED）：試験のスレッドが時刻を読んでから操作が uhd に届くまでの隙間に，デバイスの時刻が進む．隙間は 2 種類ある．

- **最初の予約の遅れ**：どのテストも `run.now()` から 30〜40 ms 先に A を予約する（`tx_at`）．待たされればこの lead を食い，A は予約の時点で捨てられる．
- **窓を外す**：`ur_23` の継続の 3 本（`:3040`，`:3402`，`:3438`）は `run.now()` が A の終わりの 8 ms 前に着くまで回り，そこで B を予約する．狙いは「A の最後の buffer は出たが，5 ms の lead にはまだ間に合う」窓である．`Stop` のテスト（`:3078`）は 7 ms 前に `Stop` を出す．c+1 のテスト（`:3145`）は先に予約する分岐と，8 ms 前に予約する分岐を持つ．

- 手動の時計は，uhd の参加者のスレッド（`uhd-rx`，`uhd-tx`，`uhd-control`，`uhd-clock`）が busy の間は止まる．全員が idle の間は実時間の速さで進む（計画 30 の M3・M5）．
- 試験のスレッドは参加者でない（M6）．試験のスレッドが OS に待たされている間，参加者は予約を待って idle なので，デバイスの時刻は実時間で進む．
- 試験のスレッドが窓（2〜3 ms）や A の lead（30〜40 ms）より長く待たされると，予約は遅れる．`send` の late policy は `drop_and_flag`（`crates/ezsdr-radio/src/lib.rs:276-282`）なので，テストの場面が作れない．

これは計画 30 §5 の **(c)「試験のスレッドの遅れ」**そのものである．計画 30 は段階 B（試験のスレッドが時計を止める `hold()`）を用意したが，§15 の負荷の表で失敗が 0 だったので要らないとした．`ur_23` の 5 本は Phase 7 の Review N〜R（2026-09-30〜10-01），`ur_21_a_descheduled_…` は計画 30 の段階 A（`e19bba2`，2026-10-13）からあり，その表を通っている．表は sim03 の container の CPU を 2〜16 に絞り，自分のテストのスレッド（`--test-threads=128`）で埋める形だった．今回はほかのプロセスが CPU を奪う形で，試験のスレッドがより長く待たされる（INFERRED）．3 ms の窓を持つ `ur_23` が最も弱い．

**害**：この 6 本は mutation の 9 行の filter である：maintenance の J295，phase7 の B08・B10・B16・B20・B28・B29・B31・B32（`plan/phase7/tools/mutations.json`）．sim02・sim03 の shard は負荷の高い状態で流れるので，変異と関係なく落ちれば「killed」と数えられ，生き残りが隠れる（AGENTS.md §7）．

## 2. 三振の問いへの答え

- **概念を消せるか．** 消すべき概念は「試験のスレッドがいつ動くかで判定が変わるテスト」である．手動の時計そのものは正しく働いている．Provider のスレッドの遅れは時刻を止め，計画 30 §15 の負荷の表で失敗はない．残る穴は，試験のスレッドが時計の外にいることだけである．
- **規則を 2 か所で実装していないか．** していない．送信の規則（UR-21〜UR-23）は uhd-tx の 1 か所にあり，テストが窓を外して場面を作れないだけである．
- **spec の文が複雑すぎないか．** spec の問題ではない．テストの書き方の規則が足りない：tests/README.md は「短い実時間の区間で機能を判定しない」と書くが，試験のスレッド自身の遅れには触れていない．

**結論：設計（手動の時計，参加者は uhd のスレッドだけ）は残す．** 規則を 1 つ足す：**テストの判定は，試験のスレッドがいつ動くかに依らない．**

## 3. 推奨：窓の要るテストは制御された時計の unit test に移す

| 何 | どうする |
|---|---|
| 規則 | tests/README.md に足す：テストの判定は試験のスレッドがいつ動くかに依らない．試験のスレッドが時刻や状態を読んでから次の操作や assert までに時間が進み得るなら，その系列は次のどれかにする．(1) Spec で開始前に予約する．(2) 時刻が動いても成り立つ assert にする．(3) 遅れが判定に関わらない予約は late policy `send_asap_and_flag` にする．(4) 時刻の窓が判定そのものなら，その性質は制御された時計の unit test で固める．B を「十分前に」予約するのは確率を下げるだけで，(2) には数えない |
| `ur_23` の 5 本 | 窓（A の最後の buffer が出た後・lead の前の予約，その継続，保持した sample の送信の失敗，待っている間の `Stop`，c+1 の burst）は `src/provider/tx.rs` の unit test に移す．使うのは既存の部品：手動の時計の FakeDevice の rig（`manual_rig`，`tx.rs:2253`），時計を止める参加者 `Frozen` と，Core と device の両方の時計を同じ瞬間に置く `device_at`（`test_support.rs:140-175`）．A の送信を実際に進め，継続の位置と保持した sample を確かめた状態で，B・`Stop`・送信の失敗を入れる．5 本すべての系列がこの rig で作れるかは INFERRED で，指示書の段階で移す前に対応表を書かせて確かめる．fake の側は，窓の要らない形が既にほかのテストで見られているなら消し，見られていないなら窓の要らない形に直す．消すテストには代わりを名指しする |
| `ur_21_a_descheduled_…` | `send_asap_and_flag` で予約する．Session の verb では `start_repeat` がこの policy を持つ（`crates/ezsdr-radio/src/lib.rs:268-275`）ので，burst は repeat になる．予約が遅れても動かされるだけで，`tx_send` の 200 ms の deschedule には必ず入る（入ったことの確認は残す）．判定は今のまま：手動の時計で `LATE_AT_DEVICE` なし，実時間であり |
| 掃除 | `fake.rs`，`ezsdr-acceptance/tests/uhd.rs`，`ezsdr-server/tests/protocol.rs` から，**参加者でないスレッドが時刻を読むか状態を観測してから，次の操作か assert までに時間が進み得る系列**をすべて一覧にする（helper の中も追う）．TX の予約に限らない．Sol が挙げた候補：`fake.rs:1557-1569`（`Stop` の前に読んだ時刻を終わりの期限の基準にする），`:2877-2925`（`device.time_now()` から timed send），`:3364-3367`（送信の後，ACK が無いことの確認までに止まると ACK が届き得る），`ur_21_late_policies_on_the_device_lead`，`ur_23_bursts_go_out_in_target_order`，`ur_23_a_one_sample_burst_booked_ahead_starts_at_its_time`，`uhd.rs:234-240` と `protocol.rs:1092-1100`（未来の capture の時刻を決めてから投入する）．各系列を上の規則の (1)〜(4) に分け，一覧と扱いを記録に残す．今回落ちなかったものも同じ形なら同じ扱いにする |
| mutation | §1 の 9 行（J295，B08，B10，B16，B20，B28，B29，B31，B32）と，掃除で移すテストを `filter` に持つ全リストの行を grep で探す．移した先の unit test に `filter` と `test_bin`（`lib`）を当て直す．J295 の `ur_21_a_descheduled_…` は移さないので `test_bin` は `fake` のまま，当て直した後に流し直す．テストの名前を変えるか消すときは，`filter` に加えて `old`・`new` と `design/18-uhd-radio.md` のテストの参照（`:248`，`:458-467`）も追う．変異なしで対象のテストが走って通ること，各 mutant を狙った assert が kill することを確かめる |
| 確かめ方 | 負荷の試験は再発の検査で，決定的であることの証明ではない．(a) 移した各テストが目的の状態と分岐に着いたことを assert で観測する．(b) mutation でその assert の強さを確かめる．(c) §1 の再現（16 コアに 16 本の busy loop，`fake` 3 回）と，計画 30 §15 の負荷の表（sim03，2・4・16 CPU）で失敗 0 |

段階は 1 つで，risky としない（AGENTS.md §9：queue，期限，識別，lifecycle，FFI，fake と実機の境のどれも変えない．本番のコードは変えない）．新しい時計の仕組みや fake の挙動の変更が要ると分かったら，その時点で risky に分け直す．テストの移動なので，テスト計画の代わりに，指示書に移す性質ごとの「何を，どの unit test で，どう観測するか」の表を書かせる．実装は free の DeepSeek，セルフレビューは Luna（owner 2026-10-23），独立レビューは Sol．

## 4. 採らない案

- **試験のスレッドが時計を止める `hold()`（計画 30 の段階 B）**：通しのテストのまま決定的になる．crate の中には，時計を止める参加者 `Frozen`（`test_support.rs:140`）が既にあり，規則 M1〜M7 を変えずに unit test で使われている．しかし通しのテストから使うには，参加者を起こす `HostClock::spawn`（`host.rs:383`，今は `pub(crate)`）を外に出す必要がある．また，止めている間に `submit` や `advance_to` がデバイスの時刻を待てば止まる．計画 30 §5 は，hold の間も polling の待ちが実時間で終わること，模擬の遅延が早く終わらないこと，queue を満たす action をまたがないことを条件に挙げた（Sol R09）．`submit` が止めた時計の下で終わるかは確かめていない（INFERRED）．窓の要るテストは unit test で `Frozen` を使えば済むので，通しのテストに止める手段を足す価値はない．
- **lead を秒にのばす**：遅れの確率を下げるだけで決定的にならない．`ur_23` の窓（lead の 5 ms の手前）はのばせない．
- **trial を増やす，落ちたら流し直す**：計画 30 と tests/README.md が退けた形（「流し直して通っても負荷の失敗は消えない」）．mutation の偽の kill も残る．
- **mutation の filter から外すだけ**：偽の kill は消えるが，gate の揺れが残る．テストの場面が負荷で作れないことも変わらない．
- **実時間の FakeDevice のテストを全部やめる**：今の実時間のテスト（Authority の診断，`TxBlocks` の wedged のテスト，#68 の Gated のテスト）は，実時間の期限そのものを見るか，終わることだけを秒の上限で見る．今回の 6 本とは関係がない．

## 5. 決定（owner 2026-10-23，すべて推奨どおり）

1. §2 の結論（設計は残し，判定は試験のスレッドの遅れに依らないという規則を足す）．
2. §3 の推奨（窓の要るテストを `tx.rs` の unit test に移し，`ur_21` は `send_asap_and_flag`，同じ形のテストを掃除する）．
3. 段階を risky としないこと（テスト計画と Sol の計画レビューを省き，コードレビューだけ Sol に頼む）．
4. （owner 2026-10-24）`Direct::start` の 2 s 先の開始を規則の唯一の例外として tests/README.md に書く（§7 の残り (a)）．
5. （owner 2026-10-24）Python の `test_v54_sleep_is_run_time` を時刻の差と §58 の 2 の性能の 2 本に分ける（§7 の残り (b)）．

## 7. 実装

- 37a：完了（2026-10-23）．規則，`ur_23` の 5 本と `ur_21_a_descheduled_…`，fake の 18 系列，`uhd_61_02`，`ea_17_…`．負荷の下で `fake` 3 回とも失敗 0．記録は [reviews/37a-opus.md](reviews/37a-opus.md)．
- 37b：完了（2026-10-23）．raw device の 13 本を `src/device.rs` の `manual`・`participant` へ，`Direct` 系の 5 本を `control.rs`・`rx.rs` の制御された時計の unit test へ（[reviews/37a-table.md](reviews/37a-table.md) の B-2・B-3）．`Direct` は表の書いた実時間ではなく手動の時計の上で動くが，試験のスレッドが参加者でないので同じ扱いになる．記録は [reviews/37b-opus.md](reviews/37b-opus.md)．
- 37c：完了（2026-10-24）．試験のスレッドが時刻を読む所を grep で全部挙げ（164＋216 か所），関数ごとの表（[reviews/37c-table.md](reviews/37c-table.md)）にして Sol が確かめ，場面そのものが遅れで変わる系列を制御された時計の unit test へ（owner を実際に走らせ，参加者を登録してから時計を放す），残りを Spec の予約と時刻が動いても成り立つ assert にした．`ur_25_a_receive_enable_…` の 1 tick と `rehearsal_b5_overflow` の揺れもここで直した．記録は [reviews/37c-opus.md](reviews/37c-opus.md)．
- 残り（owner 2026-10-24，2 つとも推奨どおり）：(a) 94・95 の窓は `Direct::start` が開始を 2 s 先に予約する所だけで，この窓は `Direct` を使う約 71 本に共通する．tests/README.md の規則に，この開始だけを例外として書いた（統合テストは時計を止められない，§4）．`FakeConfig` に fault を足して 94・95 だけ移す案は，同じ窓が残りに残るので採らない．(b) Python の `test_v54_sleep_is_run_time` は Run の時刻の差（決定的，phase6 の P04 の filter）だけを見る形にし，§58 の 2 は受信 stream を止めた Session の `test_v58_02_virtual_time_runs_faster_than_the_wall_clock`（実時間 0.0 s，上限 10 s）に分けた．

## 6. レビュー

Sol 6.1（2026-10-23）：FAIL，major 3・minor 1．方針（手動の時計を残し，試験のスレッドがいつ動くかに依る判定をなくす，`hold()` は要らない）は支持された．指摘はすべて採用した．
- major 1：`rig_with` の Authority だけでは FakeDevice の時計は止まらない．§3 の移す先を `manual_rig`・`Frozen`・`device_at` にした．
- major 2：掃除の条件が TX の `drop` の予約に限られ，capture，`Stop`，device を直接呼ぶテストの同じ隙間を漏らす．§3 の規則と掃除を「時刻か状態を読んでから次の操作までに時間が進み得る系列」に広げ，候補を載せた．
- major 3：移すテストを filter に持つ mutation の行が 4 つ（B08，B10，B16，B20）漏れていた．§1 と §3 に足し，`test_bin` の当て直しも書いた．
- minor：§1 の系列（`Stop` は 7 ms 前，c+1 は 2 つの分岐）と時期（`ur_21_a_descheduled_…` は段階 A から）の誤り，再現のログが無いのに VERIFIED としていたこと．直した．

再レビュー（同日）：PASS．4 件とも解消．minor 1（J295 は移さないので `fake` のまま，名前を変えるときは spec 18 の参照も追う）を §3 に入れた．
