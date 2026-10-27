# 段階 3c-2b：指示書

## 実装（`impl-3c2b-brief.md`）

あなたは実装役です．HEAD は `4c69714`．作業 dir は `~/.cache/ezsdr-fixes/2026-10-26-stage3/`．**この tree を触っている他の agent はいません．他の process を待たない．** owner の決め（2026-10-26）で risky でない扱い．値の行は作らず，どれも observation だけ．

### 0. 守ること

`impl-3c2a-brief.md` §0 と同じ（`cargo fmt` 禁止，ローカルの mutation 禁止，mutant を作業ツリーに当てない，commit・push・`git add`・`git stash` 禁止，version・SCHEMA_CHANGELOG に触らない，`src/` を変えない，出力はファイルへ，負荷をかけない，/tmp のまま，helper は `/home/komatsu/.claude/jobs/8aebdd6c/tmp/`，mutation の行の字下げ）．**Sol（`openai/gpt-6.1-sol`）は起こさない．** **起きたことを書く．していないことは書かない．**

### 1. 読むもの

- 設計メモ `plan/phase8/41-measurement-records.md` §8（3c-2b の行）．`plan/maintenance/audits-2026-10.md` の「Phase 7 後の保守」の表（3：fake は TimeError で burst 全体を捨てるが FPGA の既定は packet 単位，4：保持した最後のサンプルの 2 ms，6：H1〜H5 の中身）．GitHub issue #56（`gh issue view 56`）．`plan/maintenance/23-one-line-per-stream.md` 89 行と 129 行（UR-17 の要求長の上限を残した理由）．
- `crates/ezsdr-radio-uhd/tests/hardware.rs`：raw の probe の形（`hw_b8_raw_burst_gap`，`hw_b8_raw_timed_tune_phase`，`hw_b8_stop_end`，`hw_d9_p2_duc_entry_after_its_samples`），helper（`raw`，`raw_rx`，`raw_tx`，`read`，`read_on`，`drain`，`flush_tx`，`tone_1msps`，`tone_freqs`，`median_freq`，`first_steps`，`tx_session`），`hw_measurements_record`（raw の probe を呼んで `expected` の集合と比べる），`measurement::push_raw_observation`．RF safety：`SAFE_TX_HZ`，1 GHz，送信は 2 MS/s まで，利得 0，振幅 0.5 以下（`raw_tx` が assert）．
- `crates/ezsdr-radio-uhd/src/provider/mod.rs` 745 行（`stop` の timing の行：`at`，`done`），`src/provider/rx.rs`（UR-17 の `recv_len`，`rx_stop_untimed` の行），`src/provider/tx.rs`（保持した最後のサンプル，design-notes N-5）．

### 2. 作るもの（§7 が優先する）

`hardware.rs` に raw の probe（と #56・H5 の Session の probe）を 6 つ足し，`hw_measurements_record` から呼んで `expected` に足す．今の raw の probe と同じく `push_raw_observation` で observation に書き，`println!` でも出す．判定（成功・失敗）はしない：観測したものを記録する．ただし，自明でない計算（tone の位相から sample のずれを出す，聞こえた区間を出す，報告を数える）は `tests/common/` の純粋な関数にし，合成の入力でテストする（mutation の行も）．

| 名前 | 何を見るか | 1 回の中身 | observation に残すもの |
|---|---|---|---|
| `hw_h1_late_multi_packet_burst` | 開始が遅れた複数 packet の burst を X310 がどう扱うか（保守表 3） | 1 MS/s で 20 000 sample の tone の burst を，開始の時刻を `time_now` の 1，5，10 ms 前（過去）にして送る．loopback で聞く．条件ごとに 3 回 | `tx_async` の報告（code と tick，全部），聞こえた区間（最初と最後の tick，sample 数），送った数との差 |
| `hw_h2_underflow_mid_burst` | burst の途中の Underflow の後の再開（保守表 2） | 1 MS/s で tone の burst の頭 10 000 sample を時刻つき（SOB，EOB なし）で送り，host で 20 ms 待ってから残り 10 000 sample（EOB）を送る．loopback で聞く．3 回 | 報告（Underflow の tick を含む），尾が聞こえ始めた tick，予定の時刻とのずれ，tone の位相から出した sample のずれ（頭と尾それぞれ，`hw_b8_raw_timed_tune_phase` と同じ参照 tone），尾が聞こえたか |
| `hw_h5_held_last_sample` | 保持した最後のサンプルの約 2 ms の期限を，負荷の無い bench で uhd-tx が守れるか（保守表 4） | Provider の Session（`tx_session` の形）で 1 buffer の短い burst を 200 個，間を空けずに並べる | `TIME_ERROR`・`TX_UNDERFLOW` の event の数と時刻，Manifest の `bursts` の section の要約，async の報告の code ごとの数 |
| `hw_56_stop_between_starts` | #56 の 1：受信の `cold` の変更の `e₁` と `e₂` の間に Stop が来たとき X310 が `e₂` で何を流すか，その後の start が動くか | Provider の Session で受信を有効にし，`cold` の rate change を予約して，`e₁` と `e₂` の間に `Stop`（`radio/rx/s0`）を出す．その後もう一度受信を有効にして capture する．3 回 | timing の行（`cold_change`，`rx_stop_untimed`，`rx_segment`，`stop`），async・rejected の section，その後の capture が始まったか（continuity），`COMMAND_REJECTED`・`TIME_ERROR` の event |
| `hw_56_stop_lag` | #56 の 2：`Stop` を出してから uhd-control が処理するまでの遅れ（負荷なし．負荷ありは 3c-3） | Provider の Session で受信を有効にし，`Stop` を出す．20 回 | Session の log の `Stop` の受け取りの時刻（`entry.time`）と，timing の `stop` の行の `at`・`done` の差（ns） |
| `hw_ur17_receive_call_duration` | UR-17：VH-4 の ready の項は「受信の呼び出し 1 回分 ＋ 3 ms」を見込む．実機の `rx_recv` の 1 回が何 ms かかるか | raw で受信を 1，10，50 MS/s，要求長 2 000 と 65 536 で流し，`rx_recv` を 200 回呼ぶ | 呼び出しごとの時間の最小・中央・最大，返った sample 数の分布，block の長さ（要求長 ÷ rate）との差 |

- `e₁`・`e₂` の意味と `cold` の変更の予約の仕方は `hw_b8_cold_change_capture` と issue #56 から読む．Stop を `e₁` と `e₂` の間に置けるかが試験のスレッドの時刻に依るなら，置けた試行と置けなかった試行を分けて残す（置けなかったことも観測）．
- H1・H2 の聞こえた区間は loopback の 44 sample の遅れ（`hw_b8_stop_end`）を引く．
- どの probe も最後に装置を既定の状態（受信 1 GHz，利得 0，送信の streamer を閉じる）に戻す．
- H3（送信の `max_num_samps`）は作らない（段階 4）．H4（負荷）は 3c-3．

**純粋なテスト**：位相から sample のずれを出す関数（ずれ 0，+k，−k，雑音あり），聞こえた区間を出す関数（無音，途中から，終わりの前で切れる），報告の数え方．

### 3. 範囲

上の 6 つの probe と，その計算の純粋な関数と，§7 の転送する `Device` だけ．値の行を作らない．`unmeasured_reason` を変えない．Provider と本番の道は変えない．新しい rig の種類（§7 の転送する `Device` を除く），fake の fault，retry は足さない．FakeDevice のリハーサルは足さない（どれも実機の振る舞いの観測）．大きさの目安は約 900 行以内．

### 4. 確かめること

- 純粋なテストは 1 回．`cargo test --workspace`，`cargo clippy --workspace --all-targets -- -D warnings`，`PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`，`git diff --check`．`hardware.rs` は docker の `--features uhd` の clippy（`impl-3c2a-brief.md` §4 のコマンド．実機は流さない）．
- mutation：純粋な関数に行を足し，`remote_mutate.py maintenance sim02:4,sim03:4 <ID>...` で流す（前に mode 644）．全 list の `old` が 1 回一致．全 list は流さない．
- spec：`design/18-uhd-radio.md` UR-34 の Test の hardware の列に新しい probe の名前，純粋なテストの名前（DA-1 の形）．Changes に 1 行（設計メモ 41，stage 3c-2b）．

### 5. セルフレビュー（1 回）

`impl-3c2a-brief.md` §5 と同じ形で，checklist は `checklist-3c2b.md`，log は `review-3c2b-1.log`．

### 6. 報告

`report-3c2b.md`．最初の行は `STATUS: done`．作ったもの，指示書から変えた所と理由，probe ごとのテスト名，bench の時間の見積もり，spec の変更，mutation の行と結果，確かめたことの結果，セルフレビューの指摘と扱い（model も），残ったこと．

### 7. Sol の事前の確認で直した手順（§2 の表より優先）

Sol が §2 の手順を UHD 4.10 の source と repo のコードに照らした（`sol-brief-3c2b.md`）．以下に従う．各行の根拠（`path:line`）はそのファイルにある．

- **H1**：UHD は一つの時刻つきの send を packet に分けるとき，後の packet にも時刻を進めて付ける（`tx_streamer_impl.hpp:295-350`）．だから表の手順は「時刻つきの packet が全部遅れる」場合だけを見る．これを条件 A として残し，条件 B を足す：小さな時刻つきの最初の send（例 500 sample，SOB，EOB なし．開始は過去）の後に，時刻なしの続きの send を何回か，最後だけ EOB．FPGA の既定は packet 単位の回復で，遅れた packet を捨て，後の時刻なしの packet は流しうる（`radio_tx_core.v:106-116,337-408`）．send ごとに長さ，受け付けた数，時刻・SOB・EOB を残す．async の報告は `BurstAck` を含め全部残す．聞こえた区間は外側の端だけでなく**全部の区間**を残す．
- **H2**：送った後に host で 20 ms 待つだけでは，開始が先なら尾が間に合い Underflow が起きないことがある．試行の前に device の時刻を 1 回だけ読み（host の時刻で前後を挟む．3c-2a と同じ形），その後は host の時計で，頭の予定の終わり ＋ 飢えさせる間（例 10 ms）を過ぎるまで尾を送らない．各 send の host の時刻の前後と受け付けた数を残す．Underflow が観測されなかった試行は分けて残す．ずれは tone の位相では決まらない（100 kHz・1 MS/s は 10 sample で一巡する）．tone の代わりに相関が一つに決まる波形（`pn`）を使い，受信と相関を取ってずれを出す純粋な関数にする．あいまいさの合成のテストも．
- **H5**：ちょうど隣り合う burst は一つの device の burst に続き，保持したサンプルは EOB 無しで送られて期限の道を通らない（`tx.rs:335-359,425-452`）．200 個の 1 buffer の burst の間に正の隙間を空け，十分先に予約する．隣り合う burst は「対照」として名前を付けて別に残す．要求した開始と受け付けた開始，拒否された submit，実際の burst の境目，async と event の要約を残す．observation の文は「負荷の無い bench での結果の観測」で，期限の測定ではない（INFERRED）と書く．
- **#56 Stop が e₁ と e₂ の間**：`rx_start(e₂)` は `Rx::begin` で sample が来る前に出る（`rx.rs:286-351`）．FPGA の STOP は受信の command の FIFO を飛ばして queue を消す（`radio_rx_core.v:171-183`）．Session の Stop が名目の e₁ と e₂ の間に入っても，装置の start の前か後かは分からない．試験だけの「転送する `Device`」を `tests/common/` に作る：内側の `Device` に全部を渡し，`rx_start`・`rx_stop`・`rx_recv`（返った最初の tick と数）を host の時刻つきで記録する（新しい rig の種類の例外としてこれ一つだけ許す．Provider と本番の道は変えない）．順序は記録から後で分類する（眠りで置けたとは言わない）．Manifest の continuity は publish された sample だけなので，転送の記録で装置が e₂ の後に流したものを見る．cut の後に reader が居なければ「無い」ことは決め手にならない：e₂ の後に，名前を付けた装置の水準の drain を（並行の reader 無しで）1 回入れてから受信を有効にし直す．回復の capture は別に残す．
- **#56 Stop の遅れ**：timing の `stop` の行は `Provider::stop` と thread の join を挟む終わりの行で，Session の Stop とは別物（`mod.rs:710-745`）．Session の対象つきの Stop は uhd-control の timeline の item になる（`control.rs:384-414`）．転送する `Device` の `rx_stop` の呼び出しの host の時刻と，試験のスレッドが `Stop` を submit する前後の host の時刻の差を遅れとする（両方 host の時計）．log の entry の時刻（root の tick）も残す．単位を書く．
- **UR-17**：`rx_recv` は UHD で要求が満ちるまで packet をまたいで埋める（`uhd.rs:983-995`，`rx_streamer_impl.hpp:154-201`）．buffer に溜まっていれば速く返るので，壁の時間だけでは VH-4 の見込みにならない．呼び出しごとに，rate，packet の大きさ（`rx_packet_samples`），timeout，返った最初と終わりの tick，結果，sample 数を残し，始まり・定常・エラーの呼び出しを分ける．rate に 390 625 S/s を足す．加えて，cut の近くでの overrun の観測を 1 つ：定常の途中で reader を止めて（host で眠る）overrun を起こし，その前後の呼び出しを残す（上限を残した理由，`23-one-line-per-stream.md:129`）．
- **安全と片付け**：送る波形の振幅が 0.5 以下であることを送る前に assert する（`raw_tx` は周波数・rate・利得しか見ない）．各 probe の終わりに受信を止めて drain，有限の送信を終えて報告を集め，streamer を閉じ，設定を戻す．timed な command を積んだ後に device の時刻を読まない（前に読んだ値と host の時計を使う）．
- 大きさの目安は約 1,200 行以内に上げる．

## セルフレビューの checklist（`checklist-3c2b.md`）

あなたは読むだけの reviewer です．リポジトリ /home/komatsu/local_work/Ez-SDRv4 の未 commit の差分（`git diff HEAD` と untracked の新しいファイル，HEAD `4c69714`）を見る．ファイルを書き換えない．`cargo fmt`，mutation の道具，`git stash`，commit を使わない．テストは `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0 timeout 300 cargo test ...` だけ．負荷をかけない．mutation は流さない．

読むもの：作業 dir `~/.cache/ezsdr-fixes/2026-10-26-stage3/` の `impl-3c2b-brief.md`（§2 の表と，それより優先する §7 が仕様）．

見ること（4つ）：
1. **probe とテスト**：表の probe ごとに，1 回の中身と observation に残すものが表のとおりか．RF safety（送信は `SAFE_TX_HZ` の中，2 MS/s まで，利得 0，振幅 0.5 以下）を破る道が無いか．probe が終わりに装置を既定の状態に戻すか．純粋な関数が合成の入力で試されているか．各 mutation の行はどのテストのどの assert で kill されるか．
2. **範囲**：指示書 §3 に対して，足りない所と余計な所．値の行，`src/` の変更，H3・H4，fake のリハーサル，新しい rig・fault・retry，version の上げは指摘．
3. **「すべて」の主張**：`hw_measurements_record` の `expected` が呼んだ probe とちょうど一致するか．各 mutation の行の `old` が tree で 1 回だけ一致するか．
4. **文書**：`design/18` に足した文が DA-1 の形か，挙げたテストが存在するか．

orchestrator の gate は指摘にしない．出力は日本語．指摘ごとに P1（誤り・RF safety・テストが性質を見ていない）／P2（弱い・漏れ）／P3（文・形），`path:line`，根拠，直し方．最初の行に `P1 の数：n`．

## Sol の事前の確認の依頼（`sol-brief-3c2b-brief.md`）

Repository: /home/komatsu/local_work/Ez-SDRv4 (HEAD as named in the brief; tree clean). READ-ONLY: no edits, no git writes, no cargo, no tests.

This is a short check before implementation, not a code review (no code exists yet). Read the brief `/home/komatsu/.cache/ezsdr-fixes/2026-10-26-stage3/impl-3c2b-brief.md` (§2's table). In stages 3b-2, 3c-1 and 3c-2a, code reviews found majors that came from the brief stating a procedure without checking how UHD 4.10 or the Provider actually behave (a TX SampleClock that does not end at Stop; RX restart only after the reader drains; `get_time_now()` queued behind timed control-port commands).

For each of the six probes, check the procedure against UHD 4.10 source (host and FPGA) and the repo code, and answer:
1. Can the procedure as written actually produce the thing it claims to observe on an X310 + OBX? (For example: does a timed send whose time is already past produce a TimeError per packet or per burst, and do later untimed packets play; is `tx_async` the place those reports arrive; does a host pause of 20 ms after a non-EOB first send produce an Underflow and what does the device do with the tail; does any step read device time while timed commands are queued; can a Session place a Stop between `e₁` and `e₂` of a receive cold change deterministically; do the `stop` timing row's `at`/`done` and the log entry's time share one clock domain so their difference is meaningful; does `rx_recv`'s duration at a given request length reflect what VH-4's ready term budgets.)
2. Does any step risk RF safety beyond the brief's limits, or leave the device in a state that breaks the next probe?
3. What would you change in the procedure or the recorded fields so the observation answers its question (maintenance table items 2–4 and 6, issue #56, UR-17)?

Mark each claim VERIFIED (with source path:line or URL) or INFERRED. Output (final message): per probe, OK or CHANGE with the change; under about 900 words.

## 修正（`fix-3c2b-brief.md`）

あなたは実装役です．tree の未 commit の変更は段階 3c-2b の実装です（`impl-3c2b-brief.md`，`report-3c2b.md`）．orchestrator のレビュー（`orch-3c2b.md`）と Sol のコードレビュー（`sol-3c2b.md`）が出ました．下の裁定ですべて直します．`impl-3c2b-brief.md` §0 の守ることはそのまま効く．**他の agent はいません．他の process を待たない．Sol は起こさない．セルフレビューも回さない．** mutation は tree から `remote_mutate.py` で流す（tree の外のコピーから流さない．流せないときは止めて報告に書く）．

### 1. 裁定

- **Sol 1（major）：#56 の 2 つの probe が受信を再開していない．** `radio.rx.c0.stream = 0` は所属が変わらないので値だけの更新で，新しい segment を予約しない（`src/provider/control.rs:901-949`）．再開は `start_rx`（`control.rs:421-439`）．両方の probe で受信を有効にし直すのは `radio/rx/s0` への `start_rx` にする．`hw_56_stop_lag` の各試行は，転送する `Device` の記録で新しい `rx_start` の呼び出しと sample の到着を確かめてから次の Stop を出す．再開できなかった試行は Stop の遅れに数えず，別に残す．recovery の capture も新しい start の後に．
- **Sol 2（major）：H5 の正の隙間の場合が期限の道を通らない．** 次の burst が予約済みなら，保持した最後のサンプルの後すぐ burst を閉じる（`tx.rs:341-359` の `Some`）．期限の道（`None`：次が無いので，保持した端が device lead だけ先になるまで待って閉じる）を通すには，次の burst が予約されていない状態にする．今の 2 つの場合（隣り合う，前もって全部予約した正の隙間）は対照として残す．期限の場合を足す：1 buffer の burst を 1 つずつ submit し，その burst の端が閉じた（転送する `Device` の `tx_send` の記録で EOB を見た）後に次を submit する．200 個．転送する `Device` に `tx_send` の記録（host の時刻の前後，sample 数，時刻，SOB，EOB，結果）を足し，すぐ閉じたか期限で閉じたかを記録から分けられるようにする．observation の文は「負荷の無い bench での結果の観測で，期限の保証の測定ではない」のまま．
- **O1（minor）**：`drain_without_reader` の `pause_reader` と `resume_reader` の間で panic しても reader が再開するよう，drop で再開する guard にする．

### 2. 確かめること

- 純粋なテストは 1 回（変えたもの）．`cargo test --workspace`，`cargo clippy --workspace --all-targets -- -D warnings`，`rules.py --base HEAD`，`git diff --check`，docker の `--features uhd` の clippy（`hardware.rs` を変えるので必ず）．
- mutation：新しい行と触れた行（P848–P851 のうち触れたもの）を `REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:4,sim03:4 <ID>...`（前に変えたファイルを mode 644）．全 list の `old` が 1 回一致．全 list は流さない．
- spec：UR-34 の Test の名前が変われば直す．

### 3. 報告

`fix-3c2b-report.md`．最初の行は `STATUS: done`．指摘ごとに直したこと（`path:line`），テスト，mutation の行と結果，確かめたことの結果，新しい bench の時間の見積もり，残ったこと．**起きたことを書く．していないことは書かない．**

## 修正 2（`fix2-3c2b-brief.md`）

あなたは実装役です．tree の未 commit の変更は段階 3c-2b の実装と 1 回目の修正です．Sol の再レビュー（`sol-fix-3c2b.md`）の指摘を直します．`impl-3c2b-brief.md` §0 の守ることはそのまま効く．**他の agent はいません．他の process を待たない．Sol は起こさない．セルフレビューも回さない．** mutation は tree から流す．

### 1. 裁定

- **Sol 1（major）：受け付けられなかった尾で次の burst に進む．** `probe_device.rs` の EOB の判定は `accepted_samples` が数なら 0 でも成功にしている．期限で閉じる道は 1 sample の尾を送る（`tx.rs:553-579`）ので，`Ok(0)` でも次を submit してしまう．判定を「記録した sample 数を全部受け付けた EOB」にする．1 sample の EOB が 0 を返す場合の合成のテストを足し，0 の場合を今の「成功」の assert から外す．不完全に閉じた試行は別に残し，その場で直列の列を止める．mutation の行も．
- **Sol 4（minor）**：P856・P857（`>=` を `>` に）は「等しい場合を拒む」mutant で，名前が言う「古い sample を通す」場合を捕まえていない．名前を実際の変化に合わせ，順序の条件を外す mutant を足す（新しい ID）．
- **Sol 5（note）**：転送する `Device` の `tx_send` の記録は送るたびに JSON を作り mutex を取る．H5 の直列の場合が EOB を待つのに `rows()`（記録全体の clone）を繰り返し呼ぶなら，EOB を数える軽い counter（atomic など）で待つ形にする．observation に「計測を挟んだ idle の bench での結果で，計測の負荷は測っていない」と書く．

### 2. 確かめること

1 回目の修正の指示書 §2 と同じ（変えた純粋なテスト，`cargo test --workspace`，clippy，`rules.py --base HEAD`，`git diff --check`，docker の `--features uhd` の clippy，新しい行と触れた行の remote mutation，全 list の `old` が 1 回一致）．

### 3. 報告

`fix2-3c2b-report.md`．最初の行は `STATUS: done`．直したこと（`path:line`），テスト，mutation の行と結果，確かめたことの結果，残ったこと．**起きたことを書く．していないことは書かない．**
