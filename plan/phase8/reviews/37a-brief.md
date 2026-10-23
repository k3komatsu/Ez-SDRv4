# 設計メモ 37 の実装：対応表の指示書（#69）

あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `814747a`，tree はきれい）．作業 dir は `~/.cache/ezsdr-fixes/2026-10-23-note37/`．

**作業 dir の orchestrator のファイル（`*-brief.md`，`.*.orig`，`sol-*.log` など）は読むだけ．書き換えない．orchestrator が動かしている見張りのプロセス（`watchdog_proc.py`，`mutation_watchdog.py`）は止めない．自分で起こした背景の job だけを片付ける．`git stash` を使わない．**

## 1. 背景

設計メモ `plan/phase8/37-test-thread-timing.md` を最初に全部読むこと（owner が 2026-10-23 に §5 の 3 つを決めた）．続いて `crates/ezsdr-radio-uhd/tests/README.md`，計画 30 の §1 と §5（`plan/maintenance/30-fakedevice-manual-time.md`），`crates/ezsdr-radio-uhd/src/host.rs` の M1〜M7 の説明を読む．

要点：FakeDevice の通しのテスト（`crates/ezsdr-radio-uhd/tests/fake.rs`）は手動の時計で動く．時計は uhd の参加者のスレッドが busy の間は止まり，全員が idle なら実時間で進む．試験のスレッドは参加者でない．だから試験のスレッドが時刻や状態を読んでから次の操作をするまでに OS に待たされると，その間にデバイスの時刻が進む．6 本のテストがこれで負荷の下で落ちる（メモ §1）．

この段階は **risky ではない**（メモ §3，owner 決定）．本番のコード（`crates/*/src/` の `#[cfg(test)]` の外）と時計の仕組みは変えない．変えるのはテストと tests/README.md，spec 18 のテストの参照，mutation の行だけ．

**今回は対応表だけを書いて止まる**（§4）．orchestrator が表を確かめてから，次の指示書で実装に進む．

## 2. 段階全体でやること（今回は表に書くだけ）

1. **規則**：tests/README.md に，メモ §3 の規則（判定は試験のスレッドがいつ動くかに依らない，(1)〜(4) の形）を短く足す．
2. **`ur_23` の 5 本**（`fake.rs:3040`，`:3078`，`:3145`，`:3402`，`:3438`）：窓の性質を `crates/ezsdr-radio-uhd/src/provider/tx.rs` の tests に移す．使う部品は既存のものだけ：`manual_rig`（`tx.rs:2253`），`test_support::Frozen` と `test_support::device_at`（`src/provider/test_support.rs:140-175`）．新しい rig，時計の機能，fake の挙動を足さない．fake の側のテストは，窓の要らない形が既にほかのテストで見られているなら消し（代わりを名指しする），見られていないなら窓の要らない形に直す．
3. **`ur_21_a_descheduled_timed_send_is_on_time_on_the_manual_clock_and_late_on_the_real_one`**（`fake.rs:3744`）：late policy が `send_asap_and_flag` の予約にする（Session の verb では `start_repeat`，`crates/ezsdr-radio/src/lib.rs:268-275`）．`tx_send` に入ったことの確認と，判定（手動で `LATE_AT_DEVICE` なし，実時間であり）は残す．
4. **掃除**：メモ §3 の「掃除」の行のとおり，`fake.rs`，`crates/ezsdr-acceptance/tests/uhd.rs`，`crates/ezsdr-server/tests/protocol.rs` から，参加者でないスレッドが時刻を読むか状態を観測してから，次の操作か assert までに時間が進み得る系列を全部探し，(1)〜(4) に分けて直す．helper（`tx_at`，`send`，`wait`，`past_t0` など）の中も追う．メモに載った候補は出発点で，それで全部とは限らない．
5. **mutation**：メモ §3 の「mutation」の行のとおり．移したテストを filter に持つ行（B08，B10，B16，B20，B28，B29，B31，B32 と，掃除で移すテストの行）の `filter` と `test_bin` を当て直す．J295 は `fake` のまま．
6. **spec 18**：`design/18-uhd-radio.md` で，名前を変えるか消すテストの参照（`:248`，`:458-467` ほか，自分で grep する）を直す．

## 3. 対応表（`table.md`）に書くこと

### 表 A：`ur_23` の 5 本と `ur_21_a_descheduled_…`

テストごとに：

- 今のテストが確かめている性質（コメントと assert から．1〜3 行）．
- 移す先の unit test の名前（案）と，その系列：rig の作り方，`Frozen` で時計を止める所，`device_at` で置く時刻，A の送信をどう進めるか，B・`Stop`・送信の失敗をどの時刻にどう入れるか．
- **その系列が今の rig と部品だけで作れるか**．作れる根拠（`tx.rs` の tests で同じ部品を使っている既存のテストの名前と行）．作れない，または確かでない所はそう書く（INFERRED）．ここが表のいちばん大事な所．
- 何を assert し，どう観測するか（`device.transmitted`，`tx_send` の記録，`unended_bursts`，Manifest の section など），目的の状態に着いたことをどう確かめるか．
- どんな誤った実装で落ちるか．その性質を守っている本番のコードの場所（`tx.rs` の関数と行）．
- fake の側のテストの扱い：消す（代わりのテストの名前）か，窓の要らない形に直す（どう直すか）．
- そのテストを filter に持つ mutation の行と，当て直し方（`filter`，`test_bin`，変異が新しいテストのどの assert で kill されるか）．

### 表 B：掃除の一覧

grep した式をそのまま書く（orchestrator が流し直せるように）．系列ごとに：ファイルと行，テストの名前，時刻か状態を読む所と次の操作か assert の所，その間に時間が進むと何が起きるか，(1)〜(4) のどれにするか（理由を 1 行），そのテストを filter に持つ mutation の行．「隙間はあるが判定に関わらない」ものもその理由を書いて載せる．

## 4. 守ること

- **今回はコードを書かない．`table.md` と `report.md` だけを書く．** `report.md` の最初の行は `STATUS: table`，続けて表の要約と，作れるか確かでない系列の一覧．
- テストを流してよい：`timeout 300` をつけ，1 本ずつか 1 binary ずつ．**負荷をかけない**（busy loop，`--test-threads` を上げる，並列の cargo を禁止）．
- `cargo fmt`・`rustfmt` を流さない．mutation は流さない．`mutate.py` をローカルで流さない．commit・push・`git add`・`git stash` をしない．
- **指示書と設計メモに無い仕組みを足す案を書かない**（新しい rig，時計の hold，fake の新しい fault，retry，lead を秒にのばす）．今の部品で作れないなら，作れないと書いて止まる．
- 前の段階で出た誤り：誤った実装を見抜けない観測（数だけ見て値を見ない），照合が空でも通るテスト，到達できる系列のテストを代わりなしで消す，differential の操作を縮める，していないことを報告に書く．
- **起きたことを書く．していないことは書かない．**

# 設計メモ 37 の実装 37a：実装の指示書（#69）

あなたは実装役です．`impl-brief.md`（§1 背景，§4 守ること）と，あなたが書いた `table.md`・`report.md` を先に読むこと．HEAD は `814747a`，tree はきれい．作業 dir は `~/.cache/ezsdr-fixes/2026-10-23-note37/`．

**作業 dir の orchestrator のファイル（`*-brief.md`，`checklist.md`，`.*.orig`，`sol-*.log` など）は読むだけ．書き換えない．orchestrator が動かしている見張りのプロセス（`watchdog_proc.py`，`mutation_watchdog.py`）は止めない．自分で起こした背景の job だけを片付ける．`git stash` を使わない．**

## 1. 範囲：37a と 37b に分ける

orchestrator は表を確かめ，段階を 2 つに分けた．**今回は 37a だけ．**

- **37a（今回）**：表 A の全部（A-1〜A-6），表 B の B-1（18 系列），B-5（`uhd.rs`），B-6（`protocol.rs`），表 B の当て直しのうちこれらに関わる行，tests/README.md の規則．
- **37b（次）**：B-2（raw device の 13 本を `src/device.rs` へ），B-3（`Direct` 系）．今回は触らない．

## 2. 表の「判断が要る所」への orchestrator の答え

1. **T4・T5 の fault 付きの rig**：`rig_with(FakeConfig { clock: ClockMode::Manual, command_queue: 64, faults, ..FakeConfig::default() }, Vec::new())` を直接呼ぶ．`manual_rig` は変えない．新しい helper を作らない．
2. **T5**：`tx.rs` に移す．B29 は `test_bin: "lib"`．
3. **T3 の mutation 行**：足す．`close_device_burst` が tail の代わりに空の end-of-burst を送る変異（表 A-3 の「誤った実装」）を，T3 の unit test を filter にして 1 行足す．ほかのテストでも kill されてよい．`old` が tree の中で 1 回だけ一致することを確かめる．
4. **B-1 の #1（`:1334`），#4（`:1414`），#10（`:1902`）**：表の案のとおり（#1 は同じ start の部分を Spec の 2 本で別テストか同じテストに，#4 は 5 ms の 2 本だけを判定，#10 は Spec）．#1 は新しいテストを作るより，同じテストの中で Spec の 2 本にする方を選ぶ．
5. **B-5（`uhd_61_02`，`uhd.rs:240`）と B-6（`ea_17_a_capture_ahead`，`protocol.rs:1100`）**：表の (2) の案（`first >= at` と長さ）でよい．厳密な開始は `uhd_59` が Spec で固めていることを，コメント 1 行で書く．
6. **B-3（`Direct` 系）**：37b．
7. **B-2（raw device）**：37b．
8. **B08・B20**：表のとおり．B は A と違う波形にする（`held()` の一様な波形を B に使わない）．
9. **Authority の実時間のテスト**：対象外（メモ §4）．

## 3. やること

1. **tests/README.md**：メモ 37 §3 の規則を短く足す（判定は試験のスレッドがいつ動くかに依らない．(1) Spec で開始前に予約，(2) 時刻が動いても成り立つ assert，(3) 遅れが判定に関わらない予約は `send_asap_and_flag`（`start_repeat`），(4) 時刻の窓が判定なら制御された時計の unit test）．今の README の文と重ならないように，前後を読んで置き場所を決める．
2. **表 A**：A-1〜A-5 の unit test を `crates/ezsdr-radio-uhd/src/provider/tx.rs` の tests に書き，fake の側を表のとおりに消すか書き換える．A-6 は `start_repeat` に直す．A-0 の駆動（15 回の `step`）が 5 本で重なるので，tests の中の小さな関数 1 つにまとめてよい（rig の新しい種類は作らない）．
3. **B-1**：18 系列を表の分類のとおり直す（#13 と #17 は表 A と同じ）．Spec で開始前に予約する形は，既存の `ur_23_two_bursts_back_to_back_loop_back_whole`（`fake.rs:3289-3298`）に倣う．
4. **B-5・B-6**：表のとおり．
5. **mutation**：表 A のまとめと B-7 のうち 37a に関わる行．移したテストは `test_bin: "lib"`，その場で直したテストは `fake` のまま．すべての行の `old` が tree で 1 回だけ一致すること．消すテストを `filter` に持つ行と，消す名前を `old`・`new` に持つ行を全リストで grep する（`plan/maintenance/tools/mutations.json`，`plan/phase*/tools/mutations.json`）．
6. **spec 18**：`design/18-uhd-radio.md` のテストの参照（`:248`，`:458-467` ほか）が，テストの場所や形の変化で偽になる所を直す（DA-1 の形式，AGENTS.md §2．Changes の 1 行）．名前は変えない方針なので，多くは変更不要のはず．

## 4. 確かめること

- 新しい unit test と直したテストを，それぞれ 20 回続けて流して全部通る（`timeout 300`，1 binary ずつ，`--test-threads` は既定）．
- 移した各テストが，目的の状態に着いたことを assert で見ている（表 A の「assert / 観測」）．照合が空でも通る形にしない．
- 当て直した行と新しい行を `plan/maintenance/tools/remote/remote_mutate.py` で流して killed を確かめる．コマンド：`REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py <list> sim02:4 <ID> <ID> ...`（ID は空白で区切る）．流す前に，tree の変えたファイルを mode 644 にする（`chmod 644`）．全リストは流さない（orchestrator の gate）．
- `cargo clippy --workspace --all-targets -- -D warnings`，`PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`，`git diff --check`．
- **負荷をかけない**（busy loop，`--test-threads` を上げる，並列の cargo を禁止）．負荷での確かめは orchestrator がする．

## 5. セルフレビュー

報告の前に，読むだけの reviewer を新しく起こして見てもらう．1 回ごとに新しい reviewer：

```
timeout 2400 opencode2 run --standalone -m "openai/gpt-6-luna#max" --agent plan --auto "Read /home/komatsu/.cache/ezsdr-fixes/2026-10-23-note37/checklist.md and do what it says. Your final message is the review." > ~/.cache/ezsdr-fixes/2026-10-23-note37/review-<n>.log 2>&1
```

前景で流す（背景に置いたまま終わらない）．見つかったものを直して次の回へ．P1 以上が無くなるか 3 回で止める．指摘はすべて「直した」か「退けた（理由）」を `report.md` に書く．reviewer に mutation を流させない（checklist に書いてある）．

## 6. 報告

`report.md` を書き直す．最初の行は `STATUS: done`（止まったなら `STATUS: blocked` と理由）．中身：やったこと（ファイルごと），表から変えた所とその理由，移したテストと消したテストの対応（消したものの代わり），mutation の行（当て直し，足した行，killed の結果），確かめたことの結果（20 回の数，clippy，rules），セルフレビューの指摘と扱い，残ったこと．

## 7. 守ること（impl-brief.md §4 に加えて）

- 本番のコード（`crates/*/src/` の `#[cfg(test)]` の外）を変えない．変えないと書けないと分かったら止まって報告する．
- 指示書，メモ，表に無い仕組みを足さない（新しい rig の種類，時計の hold，fake の新しい fault，retry，lead を秒にのばす，`#[cfg(test)]` だけの本番の道）．
- 到達できる系列のテストを代わりなしで消さない．differential の操作を縮めない．実時間に頼るテストを足さない．
- `cargo fmt`・`rustfmt` を流さない．ローカルで `mutate.py` を流さない．作業ツリーに mutant を当てない．commit・push・`git add`・`git stash` をしない．
- **起きたことを書く．していないことは書かない．**

# セルフレビューの checklist（設計メモ 37 の 37a，読むだけ）

あなたは読むだけの reviewer です．リポジトリ /home/komatsu/local_work/Ez-SDRv4 の未 commit の差分（`git diff HEAD`，HEAD `814747a`）を見る．ファイルを書き換えない．`cargo fmt`，mutation の道具，`git stash`，commit を使わない．テストは `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review timeout 300 cargo test ...` だけ．負荷をかけない（busy loop，`--test-threads` を上げる，並列の cargo を禁止）．

読むもの：設計メモ `plan/phase8/37-test-thread-timing.md`，作業 dir `~/.cache/ezsdr-fixes/2026-10-23-note37/` の `impl2-brief.md`，`table.md`．

見ること（4 つ）：
1. **系列とテスト**：表 A の A-1〜A-6 と B-1・B-5・B-6 の各系列に，表の「assert / 観測」を満たすテストがあるか．テストは目的の状態に着いたことを assert で見ているか（照合が空でも通る形，数だけ見て値を見ない形は指摘）．当て直した mutation の行と新しい行は，その filter のテストのどの assert で kill されるか（読んで判断する．mutation は流さない）．
2. **範囲**：37a の範囲（impl2-brief §1）に対して，足りない所と余計な所．B-2・B-3（37b）に手を付けていたら指摘．本番のコード（`crates/*/src/` の `#[cfg(test)]` の外）の変更は指摘．
3. **「すべて」の主張**：消したテストを `filter` に持つ行，消した名前を `old`・`new` に持つ行が全リスト（`plan/maintenance/tools/mutations.json`，`plan/phase*/tools/mutations.json`）に残っていないか grep する．各行の `old` が tree で 1 回だけ一致するか．`test_bin` が移した先と合っているか．
4. **文書**：tests/README.md の規則がメモ 37 §3 と合っているか．`design/18-uhd-radio.md` のテストの参照が，変えたテストの場所や形で偽になっていないか．

orchestrator の gate（全 mutation，負荷の確かめ，docker）は指摘にしない．出力は日本語．指摘ごとに P1（誤り・テストが性質を見ていない）／P2（弱い・漏れ）／P3（文・形），`path:line`，根拠，直し方．最初の行に `P1 の数：n`．

# 37a の修正の指示書（Sol のコードレビュー）

あなたは実装役です．`impl-brief.md`，`impl2-brief.md`，あなたの `report.md` と，Sol のコードレビュー `sol-code.log`（最後の「判定：FAIL」から）を読むこと．tree は 37a の未 commit の差分のまま．守ることは impl2-brief §7 と同じ（本番のコードを変えない，新しい仕組みを足さない，負荷をかけない，`cargo fmt` 禁止，ローカルで `mutate.py` 禁止，commit・`git add`・`git stash` 禁止，orchestrator のファイルは読むだけ）．

orchestrator は Sol の 2 件をどちらも採った．

1. **major 1（`fake.rs:1590-1598`，`ur_23_a_stop_ends_the_burst_within_the_in_flight_window`）**：`LogEntry.time` は Kernel が dispatch の前に読む時刻なので，試験のスレッドの遅れが残る．in-flight window の上限（Stop の後，記録の終わりが Stop の瞬間から window 以内）は `src/provider/tx.rs` の制御された時計の unit test で固める（37a の `at_a_held_tail` と同じ部品：`Frozen`，`device_at`，`stop_now`）．fake の側は，時刻の隙間に依らない性質だけを見る形に直す（Stop で burst が end-of-burst で閉じる，Underflow がない，など）．
2. **major 2（`fake.rs:1454-1466`，`ur_21_a_burst_inside_the_in_flight_window_is_late`）**：5 ms 先の 2 本が遅れて予約されると，uhd-tx の in-flight の判定ではなく予約の時点の判定（`control.rs:1052-1064`）だけで同じ結果になる．再判定（予約の時点では on time，uhd-tx の最初の hand-over で `SendAsap` に動き，`Drop` は落ちる）を `tx.rs` の制御された時計の unit test に移し，予約の時点では on time だったことも assert する．fake の側は消すか，隙間に依らない形に直す（消すなら代わりを名指しする）．

どちらも，そのテストを `filter` に持つ mutation の行（全リストを grep：U36，U44 ほか）を移した先に当て直し（`test_bin: "lib"`），`old` が 1 回だけ一致することを確かめ，`remote_mutate.py`（impl2-brief §4 のコマンド，chmod 644 を先に）で killed を確かめる．`design/18-uhd-radio.md` §6 の 2 本の説明と `(unit)` を合わせる．

確かめること：新しい unit test と直した fake のテストを各 20 回（負荷なし），`cargo clippy --workspace --all-targets -- -D warnings`，`rules.py --base HEAD`，`git diff --check`．セルフレビューは 1 回（impl2-brief §5 のコマンド，checklist は同じ）．P1 が出たら直してもう 1 回まで．

`report.md` の最後に「## 修正の回（Sol）」を足す（何を変えたか，移したテストと消したテストの対応，mutation の結果，20 回の結果，セルフレビューの指摘と扱い）．最初の行は `STATUS: done` のまま（止まったら `STATUS: blocked`）．
