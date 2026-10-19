# 1b-4d-1b：指示書

## 1. 計画の指示書（DeepSeek へ）


あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `b190918`，1b-4d-1a まで commit 済み）．
この段階は **risky**（AGENTS.md §9：uhd-control と owner のどちらが device を操作するか＝ lifecycle と，fake と hardware の境界）．この回は **テスト計画だけ** を書く．コードは書かない．tree を変えない．

## 1. 背景

- 1b-4d-1 の計画（1a と 1b を合わせたもの）は `plan/phase8/reviews/1b4d1-test-plan.md`．冒頭が orchestrator の裁定，後が計画の本文（§1〜§12）．1a は commit `b190918` で実装済み（記録は `plan/phase8/reviews/1b4d1-report.md`・`1b4d1-fix-reports.md`・`1b4d1-opus.md`）．
- 1b の範囲は plan 33 §5 の 1b-4 行「分け方」の 1b-4d-1b：busy，D1 の A，lock の順序の検査．計画の系列では S9，S11d，S13・S13b，S15，S17〜S19，S22〜S24，S26〜S28．
- 計画は 1a の実装の前に書かれた．1a で名前と形が決まった（下の §2）．そのうえ orchestrator は 1a のコードと UHD のソースを読んで，busy の範囲と D1・D2 の裁定を §3 のとおり改めた．**§3 が計画と前の裁定より優先する．**

## 2. 1a で決まった形（コードで確かめてから使う）

- `crates/ezsdr-radio-uhd/src/provider/core.rs`：`DeviceSet`（`Unknown`／`Open(Channels)`），`Owner`（`Nothing`／`Configure`／`Open`），`StreamState` の `set`・`generation`・`configured: Option<(seq, 版)>`・`idle`；`Streams::owner_step`・`record_open`・`record_opened`・`record_refused`・`record_configured`；`Core::open_device`・`check_opened`・`open_streamer`・`owner_configure`．
- `control.rs`：`ColdConfig` の `version`（`AtomicU64`）・`changed()`・`configure`（`updates` の lock の中で版を読む）；`release` の記録だけの枝（`chans.is_empty()`，今は `changed_projections` の版を上げる）；`cancel_held`；`book_cold` の from-none（`from_none = !channels.is_empty() && idle && planned`，`drop(streams)` → `open_device` → `check_opened` → `core.configure` → `streams` を取り直して `record_opened`・`record_configured`・`book`）．
- `rx.rs` `begin`：plan を読む section で `idle = next.is_none()`，その後 `owner_configure` → `rx_start`．
- `tx.rs`：run loop は毎回 `read(None)`（`streams` を取る．`idle = clock.is_none() && switch.is_none()`）→ `reports(Wall::ZERO)` → `step()`．`do_switch` → `owner_configure` → `sync_streamer`．`shutdown` は `reports(10 ms)` を 100 ms．
- `mod.rs` `stop`：uhd-control を join してから uhd-tx・uhd-rx を Shutdown（`mod.rs:695-704`）．
- test 側の `Device` の wrapper：`test_support.rs` の `LockSpy`・`ApplyFails`・`ReportFeed`・`SectionWitness`．
- 1a ですでにあるテスト（計画の系列に当たるもの）：`ur_25_a_refused_transmit_switch_makes_no_device_call`（tx.rs），`ur_25_a_refused_receive_enable_is_booked_and_the_next_enable_starts`（control.rs），`ur_25_a_refused_enable_leaves_its_set_unopened_for_the_next_switch`・`ur_13_a_refused_comparison_is_repeated_at_the_next_switch`（tx.rs），`ur_25_the_line_lock_is_free_inside_every_*_device_call`（control.rs・rx.rs・tx.rs），`ur_24_a_record_only_release_after_a_from_none_configure_makes_the_owner_configure_again`（control.rs）．

## 3. orchestrator の裁定（計画と前の裁定を改める）

### B1. 操作の担当：uhd-control の from-none と owner の間だけで排他にする

前の裁定（Sol 2）は busy を streamer に触る全呼び出し（`rx_recv`・`tx_send`・`rx_stop`・restart の `rx_start`・`close_streams` …）に掛けた．これを次に改める．

- 理由（VERIFIED）：
  - 同じ stream の device を owner 以外で操作するのは uhd-control の from-none だけ．
  - from-none は owner が idle のときだけ起きる（`control.rs` の `from_none` の条件）．owner は plan を読む section で `idle` を false にしてから device を呼ぶ（RX `begin`，TX `read`）．だから owner が stream を持っている間（`rx_recv`・`tx_send`・`rx_stop`・restart）に from-none は来ない．呼び出しごとの busy は窓を閉じず，呼び出しごとに lock を 1 つ足すだけ．
  - FFI の寿命は UR-16 の形（`Arc` と streamer ごとの `call` の mutex，`uhd.rs:1010-1011,1050-1052`）で守られている．
- 規則：
  1. `StreamState` に `busy`（uhd-control の from-none がその stream の device を操作中）を置く．uhd-control が，from-none と決めた section（今の `idle && planned` の判断と同じ section）で立て，booking の section で下ろす．device の呼び出しが失敗・拒否・lost で終わる経路を含め，どの経路でも下ろす．
  2. owner は，plan を読む section で `busy` を見たら，その回は device を呼ばない．`idle` も cursor（RX の `from`，TX の `at`・`switch`）も変えない．次の回に読み直す．
  3. RX の owner は `busy` を取らない（`idle` を false にすることが owner の担当）．
- 計画から**消す系列**：S15（restart の `rx_start` の busy），S17（Stop の後の終わりの呼び出し．B1 では busy が終わりの呼び出しを止めない）．消す理由を計画に 1 行ずつ書く．
- **S24 は形を変える**：owner の `rx_open`（TX は `tx_open`）を止めている間に uhd-control が同じ stream の enable を受け取る → uhd-control は device を呼ばず，普通の cold 変更として記帳する（owner の `idle` が false だから．今の振る舞いを固定する）．

### B2. uhd-tx の report の読みは，idle のときも担当を取る

- 理由（VERIFIED，UHD 4.10）：古い streamer が生きている間に，同じ DUC・radio の入力 port へ新しい streamer をつなぐと，`rfnoc_graph` は `"Attempting to reconnect input port!"` を投げる（`host/lib/rfnoc/graph.cpp:187-197`）．古い streamer の辺が消えるのは，その destructor の disconnect の callback だけ（`host/lib/usrp/multi_usrp_rfnoc.cpp:436-475`）．uhd-tx の `tx_async` は呼び出しの間，古い streamer を自分の `Arc` で生かしている（`uhd.rs:1050-1052`）．だから uhd-tx が idle で `tx_async` の中にいる間に from-none が `tx_open` すると，open が UHD の例外で失敗しうる．
- 規則：
  1. uhd-tx は `reports` を呼ぶ回に，`read` の section（毎回取っている `streams` の lock）で `busy` が下りていれば `reading` を立て，`reports` の後に下ろす．`busy` が立っていれば，その回は `reports` も `step` の device 呼び出しもしない（S22）．shutdown の読みも同じ．
  2. uhd-control の from-none は，その section で `reading` が立っていても from-none をやめない：`busy` を立て（新しい読みが始まらない），lock を離し，`reading` が下りるのを待ってから最初の device 呼び出しをする（S23）．待ちの長さは `tx_async` の timeout（run loop は 0，shutdown は uhd-control の join の後）で決まる．
  3. 普通の cold 変更へ落とさない理由：落とすと，enable を uhd-control が configure するかどうか（ready の instant）が thread の時刻の偶然で変わり，VH-8 の比較が揺れる．
- 待ちの実装（lock なしで `reading` を見る回し方，manual clock の上で止まらないこと）は計画で決める．manual clock の participant の扱い（plan 30 §4.7）を読んで，止まらない理由を書く．

### B3. D2 は取り下げる

- B1・B2 で，from-none が owner の担当とぶつかるのは `reading` だけになり，それは待つ．from-none の条件は今の `idle && planned` のまま．owner の begin・switch の途中（`idle` が false）は今と同じく普通の cold 変更．
- 計画から D2 の記述を消し，S23 を B2 の規則 2 の期待に書き直す．

### B4. D1 の A の形

- 対象：`release` の記録だけの枝（channel が無い）で，その方向の **pending の segment**（origin が `e` より後，拒否されていない）がその channel を持つとき（layout は論理 channel，今の key は方向の channel）．
- 規則：最初のそういう segment について，
  - 記録の `configured` がその segment の `(seq, 今の版)` と一致する（owner が configure を省いた，または省く）なら，更新をその segment の origin で timed command として device に出す．`released` に入れる（device の queue は順番どおり：UC-2）．その segment の設定の版は上げない．
  - 一致しないなら，今（1a）と同じく版を上げる（owner が configure するときに投影から当てる）．timed command は出さない．
  - どちらでも，`applied` の行の `at` は `e` のまま（VH-8 が Mock と比べる．記録する instant は変えない）．行のほかの欄を変えるなら，`crates/ezsdr-acceptance/tests/parity.rs` が比べる欄を確かめてから．
- S11d の期待：from-none の configure の後，owner がその segment を始めてから，origin より前の timed 更新を記帳 → release（channel が無い）→ device に origin で timed の apply が 1 回．owner は configure し直さない．read-back（fake の設定）が origin の後にその値．逆の系列（owner がまだ configure していない）は 1a のテストが固定している．
- 未確かめ（計画で確かめる）：origin が release の時点で過ぎていることはあるか．あるなら期待を書く．

### B5. lock の順序の検査（S28）

- 規則：`updates` → `streams` → `rec`．`streams` を持ったまま `updates` を取らない．
- 検査は，本体の型を大きく変えずに作る（`lock(&core.streams)` の呼び出しは多い）．案を 1 つ選び，落とす誤り（`streams` の中で `ColdConfig::configure` や `changed()` を呼ぶ）を書く．`#[cfg(test)]` の hook は可．

### B6. helper

- 系列で使う「device の呼び出しを止める」wrapper は，4 つ目の `Device` の全実装を足さず，既存の wrapper（`LockSpy` など）に止める hook を足せるならそうする．
- テストの繰り返し（from-none の準備，owner を 1 回動かす，`calls()` から方向の呼び出しを抜く）は helper にする．1a のテストが持つ helper（`enable_from_none`・`channels_from_none`・`owner_rx`・`owner_rx_calls`・`tx_channels_from_none` など）を使う．

## 4. 計画に書くこと

`~/.cache/ezsdr-fixes/2026-10-19-phase8-1b4d1b/test-plan-1b.md` に日本語で．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．

1. **差分の表**：前の計画の 1b の系列（S9，S11d，S13，S13b，S15，S17〜S19，S22〜S24，S26〜S28）それぞれについて，残す・形を変える・消す・1a のテストで済み，と理由．Sol はこの表と §2 以降だけを読む．
2. **系列ごと**（残すもの・変えるもの）：テストの名前（`ur_<規則>_...`），置く場所，assert（1〜2 個），どう観測するか（`calls()` の列，記録，`busy`・`reading`），落とす誤った実装．
   - S9（RX・TX）：前の計画のとおり（B を残し，C の from-none の open を止める）．B1 の規則 2 の「`idle` も cursor も変えない」を assert に入れる．
   - S13：新しい待ちと section でも，device の呼び出しの間に line の lock を持たない．
   - S13b：`rx_open` の呼び出しの中で記録が `Unknown`（1a の `record_opened` の形で）．1a の行 P376 と重なるなら，そう書いて系列を消してよい．
   - S18・S19：1a のテストで済んでいるなら名前を書いて消す．足りないなら何が足りないか．
   - S22・S23・S24：B1・B2 のとおり．
   - S26：uhd-control の open・configure の最中に別の thread が lost を記録 → 戻った後に照合・configure をしない．`busy` は下りる．
   - S27：B1 の規則 1 の「どの経路でも下ろす」を，成功・open の失敗・照合の拒否・configure の失敗・lost の各経路で確かめる（1 本の表のテストでよい）．Shutdown は uhd-control の join の後なので，前の計画の「busy の最中の Shutdown」は uhd-control の join が 1 s で終わらない場合だけ．その場合の期待を書くか，消す理由を書く．
   - S11d・S28：B4・B5 のとおり．
3. **本体の変更の見積もり**：どの関数に何を足すか（`StreamState` の 2 つの欄，`book_cold` の from-none，RX `begin`，TX `read`・`reports`・`shutdown`，`release` の記録だけの枝，lock の順序の hook）と行数．
4. **既存のテストで変わるもの**：特に `ur_24_a_record_only_release_after_a_from_none_configure_makes_the_owner_configure_again` が B4 の後も成り立つか．
5. **mutation の行の方針**：新しい規則ごとに 1 行（busy を見ない owner，下ろし忘れる経路，`reading` を待たない from-none，D1 の一致の判断，lock の順序）．行は code と一緒に書くので，ここでは対応だけ．
6. **design/18 の改訂**：B1〜B5 で偽になる文と足す規則（UR-25 の「one lock …」の近く，UR-36 の記録，UR-24 の記録だけの更新）．DA-1 の形．
7. **大きさ**：本体とテストで約 1,500 行以内の見積もり．超えるなら分け方．
8. **疑問**：§3 の裁定がコードと合わないところを見つけたら，計画の中で止めずに，根拠（`path:line`）と代案を書く．

## 5. 守ること

- コードを書かない．tree を変えない（計画ファイルは `~/.cache/ezsdr-fixes/2026-10-19-phase8-1b4d1b/` に書く）．
- `cargo fmt`・`rustfmt` を流さない．mutation（`mutate.py`・`remote_mutate.py`）を流さない．commit・push・`git add` をしない．
- `cargo test` や `cargo build` も流さない（計画だけ）．
- セルフレビューはしない（計画の回）．
- 読むもの：§2 のファイル，`design/18-uhd-radio.md` の UR-24・UR-25・UR-36・UR-37，`plan/phase8/reviews/1b4d1-test-plan.md`，`plan/phase8/reviews/1b4d1-sol-plan.md`，`plan/maintenance/30-*.md` の §4.7（manual clock）．

## 2. 実装の指示書（DeepSeek へ）


あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `b190918`，tree はきれい）．この段階は risky（AGENTS.md §9）．

## 読むもの（`~/.cache/ezsdr-fixes/2026-10-19-phase8-1b4d1b/`）

- `adjudication.md`：Sol の計画レビューの裁定．**最優先**．§0 の分け方で，この段階は B1・B2・B3・B5 と loss・stop の出口だけ．B4（D1 の A，`release` の記録だけの枝，S11d，S28 の重ね合わせ）は **1c に送る．この段階では触らない**．
- `test-plan-1b.md`：あなた（前の回）が書いた計画．裁定で直すところ以外はこれに沿う．
- `plan-brief.md` §2・§3：1a の形と B1〜B6．
- `sol-plan.log` の末尾（`# Phase 8 1b-4d-1b テスト計画レビュー` 以降）：Sol の指摘の本文．

## やること

1. **計画を直す**：`test-plan-1b-v2.md` に，この段階の分だけを書き直す（裁定 §1 の 1〜8 を入れる．1c の分は「1c へ」と 1 行ずつ）．系列ごとにテスト名，assert（1〜2 個），観測，落とす誤った実装．大きさの数え直し．書けたら止まらずに 2 へ進む．
2. **実装する**：計画 v2 のとおり．計画に無い状態や失敗の経路を実装で足したら，テストを足して計画 v2 と報告に書く．
3. **`design/18-uhd-radio.md`**：裁定 §1 の 7 のとおり（UR-3，UR-25 の段落，UR-17・UR-23 は引くだけ，Changes の行）．DA-1 の形．UR-24・UR-36 は変えない．
4. **mutation の行**：新しい規則ごとに 1 行．`busy` を下ろす行は早い戻りごとに 1 行（裁定 §1 の 6）．動いた既存の行（P343，P351 など，from-none と `core.configure` に触れる行）は当て直す．

## 守ること

- `cargo fmt`・`rustfmt` を流さない（tree はわざと rustfmt の形にしていない．mutation の行はソースの文字列にちょうど一致する必要がある）．新しいコードは周りの書き方に手で合わせる．
- mutation は `plan/maintenance/tools/remote/remote_mutate.py` だけで流す．`mutate.py` をローカルで流さない．main tree に mutant を当てない．実験はリポジトリの外のコピーで．
- commit・push・`git add` をしない．
- `plan/*/tools/mutations.json` はファイル全体を書き直さない．変える行・足す行だけを，周りの書式のまま書く．行を当て直すときは，`old` の一致に加えて，名前を変えた識別子が `new` に残っていないかも grep する．
- 背景の job を残さない．あなたの shell は 1 command 120 秒で打ち切られることがある：長い command（`cargo test --workspace`，`rules.py`，`remote_mutate.py`）は timeout を延ばし（600000 ms），前景で流す．
- 手元（usrp-lnx03）に重い負荷をかけない（`cargo test` は普通に流してよいが，並行で何本も流さない）．
- Python は `PYTHONDONTWRITEBYTECODE=1`．
- テストの繰り返し（from-none の準備，owner を 1 回動かす，`calls()` から方向の呼び出しを抜く，gate を張る）は helper にする．テストは順序ごとに assert を 1〜2 個．sleep で競合を作らない（gate と hook で同期する）．
- 前の段階で出た誤り（同じ形を繰り返さない）：
  - 計画の要のテストを飛ばす．
  - 到達できる系列を「到達できない」と言って消す．
  - していない実装を報告に書く．
  - 記録を手で書き換えて，実際の経路を通さずに確かめる．

## 確かめること

1. `cargo test --workspace --no-fail-fast` と `cargo clippy --workspace --all-targets -- -D warnings`，`cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings`．
2. `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`．
3. 両方の `mutations.json` の全行が，ファイルにちょうど 1 回一致すること（rules.py が見る）．
4. mutation：`REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase7 sim02:8,sim03:8 <IDs>` で，足した行・変えた行と，コードかテストが変わった行を流す．全リストは orchestrator の gate で流す．COMPILE ERROR も生き残りとして扱う．

## セルフレビュー

reviewer は DeepSeek．回ごとに新しく起動する．チェックリストは小さくする（前の段階では，大きなチェックリストで 5 回とも判定が出なかった）：

```
cd /home/komatsu/local_work/Ez-SDRv4 && timeout 2400 opencode2 run --standalone -m "opencode-go/deepseek-v4.1-flash#max" --agent plan --auto "$(cat <チェックリストのファイル>)" > <ログ> 2>&1
```

チェックリスト（ファイルに書く）．読む範囲は `git diff HEAD` のうち `crates/ezsdr-radio-uhd/src/provider/` と `design/18-uhd-radio.md`．reviewer には「read-only．cargo・mutation・mutate.py・remote_mutate.py を流さない．指摘を P1〜P3 で，`path:line` つきで．無ければ『P1 なし』」と書く．項目：
1. 計画 v2 の各系列にテストがあり，そのテストが計画の言うものを観測するか．
2. uhd-control 以外で owner が idle の間に device を呼ぶ経路（uhd-tx の report の読み以外）が無いか．from-none の経路のすべてで `busy` が下りるか．
3. device を呼ぶ間に `core::lock` の guard を持たないか．`streams` を持ったまま `updates` を取らないか．
4. この変更で偽になる `design/18` の文（UR-3，UR-17，UR-23，UR-25）が残っていないか．

P1 が無くなるか 3 回まで．判定を出さずに終わった回は 1 回だけ起動し直し，2 回続けて出なければ止めて報告に書く．指摘はすべて直すか，理由をつけて退ける．黙って持ち越さない．

## 報告

`~/.cache/ezsdr-fixes/2026-10-19-phase8-1b4d1b/report.md` に日本語で書く．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．範囲の大きさを理由に blocked にしない．
中身：実装したもの，計画 v2 から変えたところと理由，テストと行（足した・変えた・消した），確かめることの結果（mutation は行ごと），セルフレビューの各回の指摘と扱い，変えた `design/` の文，大きさ（本体とテストの行数）．

## 3. 計画の差分レビューの指示書（Sol へ）


You are the independent reviewer (AGENTS.md §9 step 3). Repository: /home/komatsu/local_work/Ez-SDRv4 (HEAD `b190918`, clean). **Read-only**: no edits, no builds into the tree, no `cargo fmt`, no mutation tools, no commit or push.

You reviewed the plan of stage 1b-4d-1 before (`plan/phase8/reviews/1b4d1-sol-plan.md`; the plan and the adjudication of your review are `plan/phase8/reviews/1b4d1-test-plan.md`). Its first half, 1a (one per-stream record of what the device holds: set, `(seq, version)`, generation), is implemented in `b190918`. This review is of **the second half, 1b, only**: who operates a stream's device (uhd-control's enable from no stream vs the stream's owner thread), D1's option A, and the lock-order check.

The orchestrator changed parts of the adjudication you saw, after reading 1a's code and UHD 4.10: the brief `~/.cache/ezsdr-fixes/2026-10-19-phase8-1b4d1b/plan-brief.md` §3 (B1–B6). In short: `busy` is no longer a claim over every streamer call but uhd-control's alone, during an enable from no stream (the owner's `idle` mark is its own claim); uhd-tx's report reads take a `reading` mark that an enable from no stream waits for (UHD's `"Attempting to reconnect input port!"` while the old streamer lives); D2 (fall back to an owner-booked change) is withdrawn; D1 A sends a record-only release as a timed command at the pending segment's origin when the record shows the configure will not be made again.

**First**, before reading the brief's §3 or the plan, write your own failure scenarios for 1b from its purpose and the code (`crates/ezsdr-radio-uhd/src/provider/{core,control,rx,tx,mod,test_support}.rs`, `crates/ezsdr-radio-uhd/src/device.rs`, `crates/ezsdr-radio-uhd/src/uhd.rs`) and UHD 4.10 where it matters (`docker run --rm ghcr.io/k3komatsu/uhd:v4.10 …`, sources under `/root/tmp/uhd-4.10.0.0/`). Include: which thread may call the device for a stream at each moment (from-none, owner begin/switch, uhd-tx's idle report reads, shutdown); every exit path of a from-none; a loss during it; a record-only timed release against a segment already begun; lock order (`updates` → `streams` → `rec`); the manual clock (plan 30 §4.7) under any new wait.

**Then** read the brief §2–§3 and the plan `~/.cache/ezsdr-fixes/2026-10-19-phase8-1b4d1b/test-plan-1b.md` (its §1 table is the diff against the old plan's 1b sequences), and compare.

The orchestrator's notes on the plan, for you to check rather than accept:
- Plan §9.1 is right: the brief was wrong that a 1a test pins the "record does not match" side; that test is the match side and is rewritten.
- Plan §9.5 looks false to the orchestrator: `book_timed` runs on uhd-control, the thread doing the from-none, so no timed update can be booked during its device calls. Confirm or refute.

Report:
1. Scenarios of yours the brief and plan miss, each with its expected result.
2. The orchestrator's changes B1–B4: anything wrong (in particular: is narrowing `busy` safe — is there a device call by a thread other than the owner while the owner is not idle, or by the owner while idle, other than uhd-tx's report reads?). Your recommendation.
3. Plan tests that would not fail on the wrong implementation they name, or observe the wrong thing (S23's 50 ms real-time observation in particular).
4. B5's design (a transparent guard around `core::lock` with a `#[cfg(test)]` thread-local count).
5. The `design/18` text proposed in plan §7: correct, DA-1 shaped, anything it makes false that is not listed.
6. Whether the size estimate is credible.

Japanese. Severity major / minor for each item, `path:line` where it applies. Mark unverified points INFERRED. End with `判定：PASS` or `判定：要修正`.

## 4. コードレビューの指示書（Sol へ）


You are the independent reviewer (AGENTS.md §9 step 7). Repository: /home/komatsu/local_work/Ez-SDRv4. **Read-only**: no edits, no builds into the tree, no `cargo fmt`, no mutation tools, no commit or push. Read-only commands are fine (a build elsewhere needs `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review`; you should not need one).

## What to review

The uncommitted tree (`git diff` against HEAD `b190918`) implements stage 1b-4d-1b: who calls a stream's device. uhd-control's enable from no stream takes the stream (`StreamState.busy`) in the section that reads the owner's idle mark and the plan, and gives it back where it books or ends; owners that see `busy` make no device call and move neither cursor nor idle mark; uhd-tx's report read takes `reading` (now set in the same section as its plan read, `Tx::pass`/`plan_section`), which an enable from no stream waits for on the host clock (`Control::await_reading`); loss and `Provider::stop` exits; `Core::configure` checks `is_lost` before each channel's apply; the transmit shutdown's report read takes no busy check; a test-build lock-order check (`core::Locked`, `assert_line_free`). design/18 UR-3, UR-17, UR-23, UR-25 changed. B4 (D1 option A, the record-only release) is **out of scope** (stage 1c); say only if this stage makes it harder.

Material in `~/.cache/ezsdr-fixes/2026-10-19-phase8-1b4d1b/`: your plan review (end of `sol-plan.log`), the ruling `adjudication.md`, the plan `test-plan-1b-v2.md`, the report `report.md` (do not trust it; check the tree). UHD 4.10 sources: `docker run --rm ghcr.io/k3komatsu/uhd:v4.10 …` under `/root/tmp/uhd-4.10.0.0/` (a new streamer connecting while the old one lives throws "Attempting to reconnect input port!", `graph.cpp:187-197`).

## What to look for

1. Write your own failure scenarios first. Then: is there any device call for a stream by a thread other than its owner while the owner is not idle, or by the owner while uhd-control holds `busy`, other than the transmit shutdown's report read? Check in particular `Tx::pass` after `end_reading` (`step` while idle; a from-none deciding between `end_reading` and `step`), `Tx::read` callers that ignore its new `bool` (`do_switch` with `Some(passed)`: a skipped prune), the RX owner's other device calls (receive, stops) while idle, and `Rx::begin`'s early `return false` in the run loop.
2. Every exit of a from-none clears `busy` exactly once and books nothing, or books exactly as before; the record after each exit (`record_opened` written, `record_configured` not, on the early return after a loss or a stop during the calls) — is a stale `configured` reachable by any later owner decision, given what a loss and `Provider::stop` leave behind?
3. `await_reading` under the manual clock (plan 30 §4.7, M1–M7): is uhd-control a participant there, can the wait deadlock a test or the shutdown (Provider::stop joins uhd-control for 1 s before uhd-tx's shutdown)? A uhd-tx that dies with `reading` set.
4. Tests' power: does each S-sequence test observe what the plan says, and does its mutation row (P382–P402, the re-anchored P271, P343, P351, U39, J50, J59) break it? Synchronisation by gate/probe, not sleeps; any wall-clock bound that could pass the wrong implementation or fail under load.
5. The lock-order check: does `Locked` change any production behaviour (drop order, a guard held longer), and does `assert_line_free` sit before every `updates` lock?
6. design/18 text: correct against the code, DA-1 shaped, anything the stage makes false that is not changed (including plan 33's row 1b-4, `plan/phase8/33-channels-and-streams.md:266`, which the orchestrator will fix at close — just confirm what it must say).

Output in Japanese. First line `判定：PASS` or `判定：FAIL`. Numbered findings with severity (major / minor), `path:line`, evidence, scenario, fix. Mark unverified points INFERRED.
