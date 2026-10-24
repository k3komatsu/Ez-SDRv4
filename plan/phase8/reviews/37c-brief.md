# 設計メモ 37 の 37c：掃除の一覧の指示書（#69）

あなたは実装役です．今回は**一覧を作るだけ**で，テストもコードも変えない（tree は読むだけ）．HEAD は `010fd26`（37b の commit）．作業 dir は `~/.cache/ezsdr-fixes/2026-10-23-note37/`．

先に読むもの：設計メモ `plan/phase8/37-test-thread-timing.md`（§1 原因，§3 規則と「掃除」の行，§7），`crates/ezsdr-radio-uhd/tests/README.md` の規則（判定は試験のスレッドがいつ動くかに依らない，(1)〜(4)），計画 30 の手動の時計の規則 M1〜M7（`plan/maintenance/30-fakedevice-manual-time.md`），`tests/fake.rs` の `Direct`（`settle`，`until`，`now`）と `tests/common/mod.rs` の `fake`・`fake_real`．37a・37b で直した形の例は `plan/phase8/reviews/37a-opus.md`，`37b-opus.md`．

**作業 dir の orchestrator のファイル（`*-brief.md`，`checklist*.md`，`.*.orig`，`*.tsv`，`sol-*` など）は読むだけ．書き換えない．見張りのプロセスは止めない．`git stash` を使わない．`cargo fmt`，mutation の道具，commit を使わない．負荷をかけない．**

## 1. 何を一覧にするか

試験のスレッド（参加者でない）が時刻か状態を読んでから，次の操作か assert までに装置の時刻が進み得る系列のすべて（設計メモ §3 の「掃除」）．出発点は orchestrator が grep で作った 2 つの表：

- `37c-sites.tsv`：`direct.now()`，`run.now()`，`time_now()`，`Instant::now()`，`thread::sleep`，`host().sleep`，`after(&run`，`after_ticks(` の 164 か所（テストの 4 ファイル）．
- `37c-settle.tsv`：`fake.rs` の `settle(Wall…)` と `.until(` の 216 か所．

合わせて約 129 の関数．**表の行は関数ごと**（helper は呼ぶ側の行で追う）．grep が拾わない形（helper の中で時刻を読む，`run.advance_to` の後の読み，など）を見つけたら行を足し，どの grep で見つけたかを書く．

## 2. 各行に書くこと

`37c-table.md` に，関数ごとに 1 行：

| 列 | 中身 |
|---|---|
| 関数 | 名前と `path:line` |
| 時計 | `fake`（手動）／`fake_real`（実時間）／Spec の Run／unit |
| 読み → 次 | 試験のスレッドが何を読み，次に何をするか（予約の lead と late policy，`settle` の長さ，assert） |
| 判定 | **S**（遅れに依らない）か **(1)〜(4)** のどれに直すか |
| 理由 | S なら，遅れても判定が変わらない理由を一文で（例：Spec で開始前に予約；`send_asap_and_flag`；assert は「X 以後」で遅れは強めるだけ；`settle` の後の assert は起きたことだけを見て「まだ起きていない」を見ない；実時間の期限そのものが性質で `fake_real` を使う）．S でないなら，どれだけの遅れで何が落ちるか（例：40 ms 遅れると予約が late で落ち，`LATE_COMMAND` の数が合わない） |
| 根拠 | `path:line`（読んだ行）．推測は INFERRED と書く |

特に見る形：
- `now` を読んで `now + lead` に drop policy で予約する（lead より長く遅れると捨てられる）．
- `settle` の後に「まだ起きていない」「ちょうど n 件」を assert する（試験のスレッドが `settle` の後に遅れると，次のことが起きて数が変わる）．
- 読んだ時刻を期限や比較の基準にする．

## 3. 落ちたテストの調べ

`ur_25_a_receive_enable_counts_its_start_lead_from_the_end_of_its_configuration`（`fake.rs:1714`）が負荷の下で `assert!(configured - e >= ms(80))` で落ちた（`e=404266100`，`ready=420266099`，差は 80 ms より 1 tick 短い）．`e` と `ready` がどこで読まれるか（`src/provider/control.rs:863,917`，`core.rs` の `now`）を追い，なぜ 1 tick 短くなり得るか（Authority の推定，丸め，manual の `sleep` の終わり）を根拠付きで書く．本番の誤りなら止まって報告する（直さない）．

## 4. 報告

`37c-table.md` の最初の行は `STATUS: done`（止まったなら `STATUS: blocked` と理由）．表の後に：S と (1)〜(4) の数，§3 の結論，grep の外で見つけた行．**起きたことを書く．していないことは書かない．** 全行を埋める（「同様」で省かない）．

---

# 設計メモ 37 の 37c：実装の指示書（#69）

あなたは実装役です．あなたが作った `37c-table.md` と，Sol の表のレビュー `sol-37c-table-verdict.md` を先に読むこと．守ることと確かめ方は `impl2-brief.md` の §4・§5・§7 と同じ．HEAD は `010fd26`，tree はきれい．37a・37b の形の例は `plan/phase8/reviews/37a-opus.md`，`37b-opus.md`，tests/README.md の規則 (1)〜(4)．

**作業 dir の orchestrator のファイル（`*-brief.md`，`checklist*.md`，`.*.orig`，`*.tsv`，`sol-*` など）は読むだけ．書き換えない．見張りのプロセスは止めない．`git stash` を使わない．**

## 1. orchestrator の裁定（表をこう読み替える）

Sol の指摘はすべて採った．直す行は次のとおり（行番号は `37c-table.md`）．

- **(4)：制御された時計の unit test に移す**（`src/provider/control.rs`・`tx.rs`・`rx.rs` の tests．部品は既存の `manual_rig`・`manual_rig_events`・`hold`・`Frozen`・`device_at`）：表の 16，37，41，43，59，71，76，94，95 と，Sol が直した 47，48，60，61，62，63，64，65，75．66 は held burst と Stop の部分，74 は「未来の refused origin より前に次の enable を入れる」部分，87 は「stream 1 の終わりを見てから Abort まで」の部分．59 は Sol の言う read-back の窓（`wait_until` の戻りから `apply` の読み）も含めて固める．
- **(1)：Spec で開始前に予約**：表の 27，44，67．
- **(2)：時刻が動いても成り立つ assert**：表の 6，57，79，86（Sol：意図した late burst の直前に event の境界を取り，その後の event だけを見る）と，87 の残り．70・84 は `LATE_COMMAND` が空の assert（`fake.rs:5919`，`:6630`）も直す：未来の時刻が性質でないなら `at=None` にして窓をなくし，未来の時刻が性質なら (1)/(4) に分ける．
- **S のまま**：残りの行と，Sol が足した S の系列（`ur_07_callbacks_run_outside_the_schedule_lock`，`ur_25_a_receive_session_follows_its_recorded_plan`，`ur_23_a_stop_ends_the_burst_within_the_in_flight_window`，server の `ea_12_…`，`ea_07_…`，`ea_17_…` の 2 本）．表に行を足す．
- **実時間の診断**：行 2・5（`ur_07_schedule_wakes_…`，`ur_07_cancel_wakes_…`）は README が残す実時間の診断．変えない．表では S と別の分類にする．
- **行 7・8** の説明を helper の説明に限る（Sol の補足）．
- `Direct::start`（`fake.rs:731-735`）：(4) に移すテストは，開始もその時計の下で行う（`Direct` を使わない）．

各 (4) の移動は 37a・37b と同じく：unit test が目的の状態（同時刻の swap，pending の floor，held の burst，prebooked の burst，有限の中間 segment が受信すること，など）に着いたことを assert で見る．fake の側は，時刻に依らない性質が残るならその形に直し，残らないなら消して代わりを名指しする．**既存の部品で場面が作れない系列があれば，その行だけ止めて報告する**（新しい rig の種類，時計の hold，fake の新しい fault を足さない）．

## 2. `ur_25_a_receive_enable_counts_its_start_lead_from_the_end_of_its_configuration`（`fake.rs:1714`）

Sol の §3 のとおり．許容幅（`>= ms(80) - 1`）にしない．本番のコードを変えない．
- `control.rs` の制御された時計の unit test で，構成の終わり `c` が分かっている場面を作り，(a) `cold_change` の行の `ready` が booking の時刻ではなく `c` を記録すること，(b) `origin = ceil_lattice(max(e, c + lead))` が厳密に成り立つことを別々に assert する．
- fake の側は，`configured - e >= ms(80)` の assert を消し，`origin` の厳密な関係（今の 1733 行）は残す．
- 表の §3 の単調 guard の説明は誤り（Sol）．report に直した説明を書く．

## 3. `test_v54_sleep_is_run_time`（`python/tests/test_easy_api.py:296-304`）：調べるだけ

Vision §58 の 2「仮想時間は実時間より速く進む」の試験．この host では負荷が低くても 10 s の仮想時間に 10.1〜12.9 s かかる（`plan/maintenance/22-timing-simplification.md:675`，`21-amendments.md:277`）．**直さない．** 調べて report に書く：この Session の Time Authority と ExecutionClass は何か（free-running か wall-paced か），10 s の仮想時間で何が動き（どの Provider，どの rate，何 block），時間はどこで使われるか（server を `--release` で build した場合と debug の場合の実時間を各 1 回，`time` で測ってよい．負荷はかけない）．§58 の 2 が成り立っていないのか，試験の場面が重すぎるのかを根拠付きで書く．

## 4. mutation と spec

- 移したテスト，消したテスト，名前を変えたテストを `filter` に持つ行を全リストで grep し，移した先に当て直す（`test_bin: "lib"`）．`old` が tree で 1 回だけ一致すること．消した名前を `old`・`new` に持つ行も grep する．
- `design/18-uhd-radio.md` のテストの参照で，場所や形の変化で偽になる所を直す（DA-1 の形式，Changes の 1 行）．

## 5. 確かめること・セルフレビュー・報告

- impl2-brief §4 のとおり（新しい unit test と直したテストを各 20 回，負荷なし．当て直した行と新しい行を `remote_mutate.py` で流して killed．**sim02・sim03 は別の job で混んでいるので `sim02:2` まで**．clippy，`rules.py --base HEAD`，`git diff --check`）．
- セルフレビュー：impl2-brief §5 のコマンドで，checklist は `checklist-37c.md`，log は `review-37c-<n>.log`．P1 以上が無くなるか 3 回で止める．
- `report.md` を書き直す（最初の行 `STATUS: done` か `STATUS: blocked`）．中身は impl2-brief §6 の項目と，§2 の説明，§3 の調べの結果，止めた行があればその理由．**起きたことを書く．していないことは書かない．**

---

# 37c の修正の指示書（Sol のコードレビュー）

あなたは実装役です．`37c-impl-brief.md`，`impl2-brief.md`（§4・§5・§7），あなたの `report.md` と，Sol のコードレビュー `sol-37c-verdict.md` を読むこと．tree は 37c の未 commit の差分のまま．守ることは impl2-brief §7 と同じ（本番のコードを変えない，新しい rig の種類・時計の hold・fake の新しい fault を足さない，負荷をかけない，`cargo fmt` 禁止，ローカルで `mutate.py` 禁止，commit・`git add`・`git stash` 禁止，orchestrator のファイルは読むだけ）．

orchestrator は Sol の 7 件をすべて採った．

1. **major 1（`rx.rs` の `ur_17_a_finite_segment_delivers_through_its_cut`）**：`Segment.cut` は sample index．`cut: Some(5_000)` にし，旧 clock の終わり，次の segment の `begin()`，新しい clock の先頭が index 0 であることを assert する．move chain（B が実際に受信して終わり，それで C が進む）は 2 と同じ扱い．
2. **major 2（swap の代わり：`control.rs:4538-4566`，`tx.rs:3111-3134`，同サイズの swap `control.rs:4359-4428`）**：旧 streamer と両 owner を既存の部品で進め，owner 自身が旧 segment を解放して両方の新しい streamer に着くことを必須にする．`next == None` を成功にしない．同サイズの swap では新しい segment の実 sample の channel も見る．**既存の部品でこの場面が作れないなら，unit test で作るのをやめ，消した fake のテスト（両 swap，有限の中間 segment の chain）を (1) の形で戻す**：2 つの move（と chain の move）を Spec の update として開始前に予約し（37c の `captures_across_cold_changes` と同じやり方），同じ instant が予約で保証されるようにして，旧テストの assert（両 stream の再開・open，B の受信と C の進み）を残す．どちらにしたかを report に書く．
3. **major 3（`control.rs:4026-4054`，§2）**：probe は `apply` の直前なので，構成の終わりを区別できない．既存の `HoldApply` か `LockSpy` の戻り側の gate を使い，apply の最中と終わった後の Core の時刻を分け，既知の終わりの時刻から `ready` と `origin` を別々に assert する．
4. **major 4（`fake.rs:449-461`，`ur_07_now_tracks_the_device_while_no_call_runs`）**：許容幅を 1 ms に広げない（100 ppm・2 s で再アンカーなしでも約 200 µs ずれる）．bracket にしたうえで元の 100 µs の検出力を保つか，追従の判定を制御された時計の unit test に分ける．
5. **major 5（`fake.rs:2546-2586`，`captures_across_cold_changes`）**：`rec` の artifact の数が予定の数に等しく，各 capture が空でなく，必要な両方の clock の有効な sample を持つことを assert する．
6. **major 6（行 74，`ur_25_a_transmit_refusal_from_no_stream_starts_no_clock`）**：既存の `core.check_rates(Dir::Tx, 99)` で UR-13 の拒否を作る手法（`tx.rs:1242-1243`，`control.rs:1228-1240`）で (4) に移す．制御された時計で未来の拒否を予約し，Tx owner を進め，origin の前に次の enable が処理されることを見る．63・64 も同じ目で既存の部品を探し直し，作れるなら移す（作れないなら理由を具体的に）．
7. **minor（`design/18-uhd-radio.md:667`，`report.md`）**：Changes の no-op テストの記述を実装に合わせる．no-op の `LATE_COMMAND` 不在の assert を本当に戻すか，戻していないと書く．Python の調べの「5 000 block，20 M samples」を 10 M complex samples に直す．

各修正の後：そのテストを `filter` に持つ mutation の行（全リストを grep：F39，P252，P253，P265，P317，P320，P321，P339，P506，P507，R05 と，戻した fake のテストの行）を `remote_mutate.py`（`sim02:2` まで，chmod 644 を先に）で流して killed を確かめる．戻したテストは `test_bin` を `fake` に戻す．`old` が 1 回だけ一致すること．spec 18 の該当の参照を合わせる．

確かめること：直したテストと戻したテストを各 20 回（負荷なし），`cargo clippy --workspace --all-targets -- -D warnings`，`rules.py --base HEAD`，`git diff --check`．セルフレビューは 1 回（impl2-brief §5 のコマンド，checklist は `checklist-37c.md`，log は `review-fix37c-1.log`）．P1 が出たら直してもう 1 回まで（`review-fix37c-2.log`）．

`report.md` の最後に「## 修正の回（Sol）」を足す（何を変えたか，unit で作ったか fake に戻したか，mutation の結果，20 回の結果，セルフレビューの指摘と扱い）．最初の行は `STATUS: done` のまま（止まったら `STATUS: blocked`）．**起きたことを書く．していないことは書かない．**

---

# 37c の 2 回目の修正の指示書（Sol の再レビュー）

あなたは実装役です．`fix37c-brief.md`，`impl2-brief.md`（§4・§5・§7），あなたの `report.md`（§10 まで）と，Sol の再レビュー `sol-37c-fix-verdict.md` を読むこと．tree は 37c の未 commit の差分のまま．守ることは impl2-brief §7 と同じ（本番のコードを変えない，新しい rig の種類・時計の hold・fake の新しい fault を足さない，負荷をかけない，`cargo fmt` 禁止，ローカルで `mutate.py` 禁止，commit・`git add`・`git stash` 禁止，orchestrator のファイルは読むだけ）．

orchestrator は Sol の major 3 件を採り，項目 5 の `rehearsal_b5_overflow` も 37c に入れる（表の行 102 は S ではなかった）．

1. **major 1（`control.rs:4431-4444,4459-4470`，同サイズの swap）**：制御された時計の上で，旧 streamer から実 owner を切り替え，新しい segment の実 sample が新しい channel のものであることを assert する（channel を見分けられる sample：fake の loopback の波形か channel ごとの値）．RX と TX の両方．
2. **major 2（`tx.rs:3236-3252,3325-3338`，行 63・64）**：`Frozen` を落としてから owner の参加者が登録されるまで，と，その後の `device.set_time()`・`tx.step()` が参加者でない試験のスレッドで動く隙間をなくす．待ちのある切替は既存の参加者（`host.spawn` など，37b の `participant` の形）で動かし，時刻の設定と送信の操作は再び `Frozen` の下で行う．時計を放してから参加者を登録するまでにも隙間を残さない（参加者を先に登録してから `Frozen` を落とす順など）．
3. **major 3（`fake.rs:459-462`，`ur_07_now_tracks_the_device_while_no_call_runs`，U35）**：再アンカーの追従と U35 の検出を，既存の制御された時計と参加者を使う unit test に分ける（`authority.rs` の tests か `src/device.rs` の tests．`uhd-clock` が止まれば落ちることを，遅れに依らずに見る）．fake の側は bracket の読みの誤差だけを見る形に残すか，代わりを名指しして消す．U35 の `test_bin`・`filter` を移した先に当て直す．
4. **`rehearsal_b5_overflow`（`tests/fake.rs`，`tests/common/mod.rs` の `rehearse_overflow`）**：Sol の項目 5 のとおり (4)．overflow → 再開の block → `OverflowRestart` の gap → payload と gap の `lost` の一致を，制御された時計の unit test（`rx.rs` の tests，既存の rx の rig と `with_rx_stall` か同じ効果の既存の部品）で固める．大きな capture の書き出しを判定から外す．`rehearse_overflow` は `tests/hardware.rs` の `hw_b5_overflow` も呼ぶので，hardware 側の呼び出しは壊さない（関数は残し，fake の `rehearsal_b5_overflow` だけを unit test に置き換えるか，capture の完了を状態で待つ形にする）．その filter の mutation 行を grep して当て直す．

各修正の後：触ったテストを `filter` に持つ mutation の行（全リストを grep．U35，P509，P518，P317 と同サイズ swap の行，b5 の行）を `remote_mutate.py`（`sim02:2` まで，chmod 644 を先に）で流して killed を確かめる．`old` が 1 回だけ一致すること．spec 18 の該当の参照を合わせる．

確かめること：直したテストを各 20 回（負荷なし），`cargo clippy --workspace --all-targets -- -D warnings`，`rules.py --base HEAD`，`git diff --check`．セルフレビューは 1 回（impl2-brief §5 のコマンド，checklist は `checklist-37c.md`，log は `review-fix37c2-1.log`）．P1 が出たら直してもう 1 回まで（`review-fix37c2-2.log`）．

`report.md` の最後に「## 11. 2 回目の修正の回（Sol）」を足す（何を変えたか，mutation の結果，20 回の結果，セルフレビューの指摘と扱い）．最初の行は `STATUS: done` のまま（止まったら `STATUS: blocked`）．**起きたことを書く．していないことは書かない．**

---

# 37c の 3 回目の修正の指示書（Sol の 2 回目の再レビュー）

あなたは実装役です．Sol の 2 回目の再レビュー `sol-37c-fix2-verdict.md` と，あなたの `report.md` §11 を読むこと．tree は 37c の未 commit の差分のまま．守ることは impl2-brief §7 と同じ（本番のコードを変えない，新しい仕組みを足さない，負荷をかけない，`cargo fmt` 禁止，ローカルで `mutate.py` 禁止，commit・`git add`・`git stash` 禁止，orchestrator のファイルは読むだけ）．

**直すのは 1 件だけ**（major，`crates/ezsdr-radio-uhd/src/provider/tx.rs:3310-3325,3405-3419`，行 63・64 の 2 本）：worker が結果を送って終わってから，試験のスレッドが次の `Frozen` を取るまでの間，参加者がいなくなり時計が進む．destination（と source）の worker を，結果を送った後も参加者のまま待たせ，試験のスレッドが次の `Frozen` を取ってから終わりを許す（既存の channel で合図する．新しい仕組みは足さない）．時計の保護を切れ目なく渡す．同じ隙間が 37c で足したほかの owner のテスト（`rx.rs` の move chain・同時刻 swap・同サイズ swap，`tx.rs` の同時刻 swap・同サイズ swap）にもあれば，同じ形で直す（無ければ「無い」と根拠を書く）．

確かめること：直したテストを各 20 回（負荷なし），P509・P518 と，ほかに直したテストを `filter` に持つ行を `remote_mutate.py`（`sim02:2` まで，chmod 644 を先に）で流して killed，`cargo clippy --workspace --all-targets -- -D warnings`，`rules.py --base HEAD`，`git diff --check`．セルフレビューは 1 回（impl2-brief §5 のコマンド，checklist は `checklist-37c.md`，log は `review-fix37c3-1.log`）．

`report.md` の最後に「## 12. 3 回目の修正の回（Sol）」を足す．最初の行は `STATUS: done` のまま．**起きたことを書く．していないことは書かない．**

---

