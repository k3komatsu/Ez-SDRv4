# 設計メモ 40：指示書

## 実装（`impl-brief.md`）

あなたは実装役です．HEAD は `d6ee79a`（`plan/phase8/40-preemption-bound.md` だけ未 commit．orchestrator のメモなので書き換えない）．作業 dir は `~/.cache/ezsdr-fixes/2026-10-25-preempt/`．

先に読むもの：設計メモ `plan/phase8/40-preemption-bound.md`（全部．**§6 の D2'' が D2・D2' を置き換える**．§5 の D3・D4 は owner の決定），テスト計画 `test-plan.md`（前の実装役が書いた改訂版），Sol の計画レビュー2回（`sol-plan.log`，`sol-plan2.log` の `VERDICT` 以降）．**この指示書 §1 が計画より優先する**（食い違えばこちら）．

**作業 dir の orchestrator のファイル（`*-brief.md`，`checklist*.md`，`sol-*`，`test-plan-v1.md`）は読むだけ．書き換えない．** helper の script は `/home/komatsu/.claude/jobs/8aebdd6c/tmp/` に置く．

## 1. orchestrator の裁定（計画をこう直して実装する）

- **S1 D2''（Sol Q1）**：Mock の候補は「受け取りの順で B より前に受け取った burst」（同じ瞬間でも数える）．計画 §4 の P1 の表を直す：
  - 同じ周回で A（+4 ms）→ B（+3 ms）：**UHD は B on time で先に出る．Mock は B late**（`handed(t)` へ動く／drop）．Mock の方が厳しい保守的な違いとして，二つのテストにそれぞれの期待値を書き，テストの doc comment に D2'' と Vision §59 を一行で書く．
  - 時計を止めたまま A を予約し `Tx::step` で手渡ししてから B を予約（Sol Q1 の第三の順）：UHD は B late．Mock も late．
  - A を前の瞬間に受け取った場合：両方 late（今の計画どおり）．
- **S2 `moved_duplicate`（計画 §8 を直す）**：D2'' では同じ step の A も数えるので，x3xx では B は A の終わり（sample 2,003,002）へ動き，重なりの拒否にならない．`mr_16_burst_refusals` の `moved_duplicate` は**目的（device lead で動いた開始が held の開始と重なれば拒否）を保ったまま ideal（W＝0）の profile へ移す**．x3xx の同じ並びは別の case にして，2,003,002 への move と `TIME_ERROR { send_asap }`，`requested_target` を assert する．x3xx の Mock で重なりの拒否の道にまだ届くかを読んで，report に書く．
- **S3 device burst を閉じた後の開始（Sol Q2）**：payload の上限は D3 のとおり W で厳密（`now + W` 以後の最初の sample が排他の境界）．空の EOB の zero sample 1つは UR-23 の例外として残す．閉じた後の安全な開始は，最後の sample を EOB と一緒に送ったなら終わり＋1 sample，空の EOB で閉じたなら H＋2 sample．Mock も同じ境界．止めた繰り返し → B を H，H＋1，H＋2 で試す（両 Module．UHD は FakeDevice の padding，timed SOB，sample，report を見る）．系列 11b の「終わりちょうどは late，＋1 sample へ」はそのまま．
- **S4 D4 の Stop（Sol Q3）**：UHD は手渡し済みの A を open として持ち，Stop で手渡した分を流して EOB で閉じる（`tx.rs` の `end_open`，`close_device_burst`．読んで確かめる）．Mock も同じに：Stop の瞬間 `s` に手渡し済みの burst（走っているものと，開始が `s` より後でも手渡し済みのもの）は手渡した分まで流し，それより後と未手渡しの held を取り消す．試験：走っている繰り返し（今の計画），Stop の時に手渡し済みの未来の A（`s + 4 ms` 開始），どちらも Mock の `bursts` の記録と **loopback の medium の sample**（遅らせた切れ目までは出て，その後は出ない）で見る．`design/09-mock-radio.md` MR-25 と MR-32 を直す．
- **S5 期限（Sol Q4）**：期限の過ぎた front を持ち続ける変異を「殺す」と書かない（`max(lead, H)` は lead なので殺せない）．状態を消すかどうかは実装の自由．
- **S6 spec**：`design/07-radio-model.md` RM-4，RM-6，RM-15（Mock の bound と in-flight window の出所），RM-16（手渡しする Provider の終わり方と空の EOB），RM-20，RM-29．`design/09-mock-radio.md` MR-3，MR-17，MR-25，MR-32．`design/18-uhd-radio.md` §3（in-flight window は profile の値，厳密な上限），UR-21，UR-23．各 spec の Changes に1行（DA-1：規則は数行，`*Test:*` 行で終わる，理由・issue 番号を本文に書かない）．`schemas/radio/envelope.v1.json` を再生成．**version は上げない．SCHEMA_CHANGELOG に書かない．**

## 2. 範囲

設計メモ D1，D2''，D3，D4 と計画（§1 で直したもの）の全部．共有するのは「現在の bound と H の遅い方」の計算だけ（計画 §4「共有するものだけ」）．UHD は自分の cursor，Mock は D2'' の真似を持つ．既存の部品（`provider::test_support` の rig，`FakeDevice`，`ManualTimeAuthority`，`Frozen`，Mock の `Harness`）で作る．新しい rig の種類，fake の新しい fault，retry，`#[cfg(test)]` だけの本番の道は足さない．作れない系列があればその行だけ止めて報告する．実装で計画に無い状態や失敗の道が増えたら `test-plan.md` に行を足す．大きさの目安は本体とテストで約 1 500 行（owner 2026-10-19）．大きく越えそうなら，どこが大きいかを report に書く（止めなくてよい）．

## 3. 確かめること

- 新しいテストと直したテストを各 20 回続けて流して全部通る（`timeout 300`，1 binary ずつ，`--test-threads` は既定）．
- `cargo test --workspace`，`cargo clippy --workspace --all-targets -- -D warnings`，`PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`，`git diff --check`，`rm_20_schema_freeze`．
- mutation：計画 §9 の行を `plan/maintenance/tools/mutations.json` に足す（形は 38a の行に倣う：`git show 30082bd -- plan/maintenance/tools/mutations.json`．行を足すだけで file を書き直さない．1 space の字下げを保つ）．新しい行と，変えたコードや killing test に触れる既存の行を流して killed を確かめる：`REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:4,sim03:4 <ID> <ID> ...`（ID は空白で区切る）．流す前に tree の変えたファイルと新しいファイルを mode 644 にする．各行の `old` が tree で 1 回だけ一致すること．消した・名前を変えたテストを `filter` や `old`・`new` に持つ行を全リスト（`plan/maintenance/tools/mutations.json`，`plan/phase*/tools/mutations.json`）で grep して直す．全リストは流さない（orchestrator の gate）．
- **負荷をかけない**（busy loop，`--test-threads` を上げる，並列の cargo を禁止）．テストの一時ファイルは /tmp のまま．`TMPDIR` を変えない．

## 4. セルフレビュー

報告の前に，読むだけの reviewer を新しく起こす．1 回ごとに新しい reviewer：

```
timeout 2400 opencode2 run --standalone -m "anthropic/claude-haiku-5-5#high" --agent plan --auto "Read /home/komatsu/.cache/ezsdr-fixes/2026-10-25-preempt/checklist.md and do what it says. Your final message is the review." > ~/.cache/ezsdr-fixes/2026-10-25-preempt/review-<n>.log 2>&1
```

前景で流す．見つかったものを直して次の回へ．P1 以上が無くなるか 3 回で止める．指摘はすべて「直した」か「退けた（理由）」を `report.md` に書く．

## 5. 報告

`report.md` を書く．最初の行は `STATUS: done`（止まったなら `STATUS: blocked` と理由）．中身：やったこと（ファイルごと），計画から変えた所とその理由，系列ごとのテスト名，既存テストの書き換えの対応，spec の変更，mutation の行（足した行，当て直した行，killed の結果），確かめたことの結果（20 回の数，workspace，clippy，rules，schema_freeze），セルフレビューの指摘と扱い，S2 の調べ（x3xx で重なりの拒否に届くか），残ったこと．

## 6. 守ること

- `cargo fmt`・`rustfmt` を流さない（tree は rustfmt の形でない．新しいコードは周りの形に手で合わせる）．
- ローカルで `mutate.py` を流さない．作業ツリーに mutant を当てない．
- commit・push・`git add`・`git stash` をしない．
- 到達できる系列のテストを代わりなしで消さない．実時間に頼るテストを足さない（tests/README.md の規則）．
- **起きたことを書く．していないことは書かない．**


## 実装の続き（セッションが落ちた後）（`impl2-brief.md`）

あなたは実装役です．前のセッション（同じ指示書 `impl-brief.md` で動いた）は，セルフレビュー 2 回目の指摘を直している途中で落ちました（opencode の "Compaction produced no summary"）．tree の未 commit の変更はそのセッションの仕事で，残っています．**`impl-brief.md` を全部読み（§6 の守ることはそのまま効く），その続きをする．**

## 0. 今の状態

- 実装，テスト，spec の変更，mutation の行はできている．`git diff --stat` で見る．
- セルフレビュー 1 回目：`review-1.log`．前のセッションが直した（P1-1 cold・loss の cut を瞬間に，`mr_32_x3xx_cold_change_cuts_at_its_instant`，`mr_20_x3xx_loss_cuts_transmit_at_its_instant`；P1-2 design/18 の INFERRED の注記を戻した，など）．P2・P3 のどれを直しどれを退けたかは，ログの最後の部分と diff から読み取って report に書く．
- セルフレビュー 2 回目：`review-2.log`．まだ直していない．下の §1 が orchestrator の裁定．
- 作業 dir の `impl.log` は 37 万字ある．**全部を読まない**（前のセッションは文脈があふれて落ちた）．必要なら `grep -n` と `sed -n` で一部だけ見る．cargo やテストの出力はファイルに出して `tail` で短く見る．

## 1. review-2 の裁定

- **P1-1（Mock の隣の burst）：退ける．** Mock の block の `END_OF_BURST` は Kernel の burst の枠で，UHD も Kernel の block には `eob` を付ける（`tx.rs` の header の flags）．UHD が EOB を device に送らず続けるのは device の枠の話で，Mock の Kernel の記録とは食い違わない．UHD は finite の burst の最後の sample を device lead の締め切りまで手元に残す（`tx.rs` の `hold`）ので，その前に来た隣の burst は続けて on time になる．Mock が開いている・手渡し済みの finite の burst に `end`（`+1` なし）を与えるのはこれに合う．HEAD にも同じ `held_cut == stop` がある．report にこの理由を書く．
- **review-1 P2-3（閉じた burst の 1 sample の隙間が ideal にも掛かる）：採る．** 設計メモ §6 D2'' は「W＝0 では今の振る舞いのまま」．Mock が閉じた burst に覚える安全な開始（`remember_tx_front(..., stop + 1)` と Stop の `+2`）は，`tx_in_flight_ns > 0` の profile だけにする（ideal は HEAD と同じ）．`mr_17_a_closed_finite_burst_needs_one_sample_gap` は ideal では「隙間なしで on time」を確かめる形に直す．x3xx での安全な開始は止めた repeat の H+2 などのテストが見ているはず．どれが見ているかを report に書く．MR-17 の文の「closed device burst」の所に，profile が in-flight window を宣言するときだけ，と足す．mutation の行を一つ（条件を外すと ideal のテストが落ちる）．
- **P2-1（期限切れを見ていないテスト）：採る．** 期限が切れたことを観測できる場面を作れるならそうする．作れないならテストの名前と spec の *Test:* の書き方から「expiry」を見ている主張を外す（Sol Q4 の裁定と同じ）．
- **P2-2（UHD の前線の経路の mutation 行）：採る．** `end_open` の `safe_gap`（1／2），`abandon` の `+2`，`dispatch_start` の前線の分岐に，それぞれ殺されることを確かめた行を足す．殺せない行は足さずに report に書く．
- **P2-3（`tx_in_flight_ns` が device lead より小さいと UHD が渡せない）：採る．** 一番小さい形で：UHD Module の profile が `tx_in_flight_ns ≥ DEVICE_LEAD_NS` を満たすことを確かめる単体テストと，その前提を書いた 1 行のコメント．検査の仕組みは足さない．
- **P2-4（`mr_17_handed_front_reduction_cases` に同じ瞬間の除外がない）：採る．**
- **P2-5（current の bound の式が Mock と UHD で違う）：退ける．** RM-14 の通常の bound は今回の変更の前からあり，範囲の外．report に書く．
- **P2-6（Mock の `stop_tx` が cold・loss でも前線を覚える）：** 1〜2 行で `Stop` だけに限れるなら直す．そうでなければ退けて理由を書く．
- **P3-1，P3-2，P3-4，P3-5：採る**（字下げ，mutant の名前，古いコメント，guard の assert）．review-1 の P3（mutation 行と MR-3 の表の字下げ）がまだ残っていれば一緒に直す．`mutations.json` は 1 space の字下げを保つ．
- **P3-3（UR-23 の中の状態の文言）：退ける．** review-1 P1-2 で戻した INFERRED の注記で，accepted spec は INFERRED を規則の中に持つ．

## 2. そのあと

1. 直したテストと新しいテストをそれぞれ 20 回．`cargo test --workspace`，clippy -D warnings，rules.py --base HEAD，`git diff --check`，`rm_20_schema_freeze`（`impl-brief.md` §3）．
2. 変えたコードや killing test に触れる mutation の行と新しい行を `remote_mutate.py` で流す（`impl-brief.md` §3 のコマンド．前に mode 644）．
3. セルフレビュー 3 回目（`review-3.log`，`impl-brief.md` §4 のコマンド）．これが最後の回．見つかったものは直すか退けて理由を書く．3 回目で直したものが mutation の行やテストに触れたら 1・2 をその分だけ流す．
4. `report.md` を `impl-brief.md` §5 のとおり書く．セルフレビューは 1〜3 回目すべての指摘と扱いを書く．前のセッションが落ちたこと，1 回目の扱いはログと diff から読み取ったことも書く．

**起きたことを書く．していないことは書かない．**


## 修正 1 回目（`fix1-brief.md`）

あなたは実装役です．tree の未 commit の変更は設計メモ 40 の実装です（`impl-brief.md`，`impl2-brief.md`，`report.md` が経緯．同じ作業 dir `~/.cache/ezsdr-fixes/2026-10-25-preempt/`）．orchestrator のレビュー（`orch-review.md`）と Sol のコードレビュー（`sol-code.log` の最後の `VERDICT` 以降）が出ました．下の §1 が orchestrator の裁定で，採った指摘をすべて直します．

**`impl-brief.md` §6 の守ることはそのまま効く**（`cargo fmt`／`rustfmt` 禁止，ローカルの `mutate.py` 禁止，commit・`git add`・`git stash` 禁止，負荷をかけない，テストの一時ファイルは /tmp のまま）．**文脈を守る**：`impl.log`・`impl2.log`・review の log は長いので全部を読まない．cargo やテストの出力はファイルに出して `tail` や `grep` で短く見る．helper の script は `/home/komatsu/.claude/jobs/8aebdd6c/tmp/` に．

## 1. 裁定（すべて採る）

- **F1（major）Mock は 1 sample の burst の終わりちょうどの再開を通すが，UHD は通さない．** UHD は 1 sample の burst を最初の buffer で device EOB つきで送り（`tx.rs` の `!(device_sob && count == 1)`），前線は end + 1 になる．Mock の前線は end のまま．直し方：window が 0 でないとき，Mock の前線の計算で，先に受け取った finite の burst のうち長さ 1 のものは end + 1 を出す（`handed_end` の所．ideal は変えない）．UHD が最後の sample を device lead の締め切りで EOB つきで送る場合は，その後に来る burst を通常の bound がすでに late にするので，足さない（理由を report に書く）．テストは Mock と FakeDevice の対：1 sample の A のあと，終わりちょうど（end）の B が両方で late，end + 1 の B が両方で on time．`late_by` と B の実際の開始を assert する．MR-17 と UR-23 の文に 1 sample の burst のことを足す．mutation の行を一つ．
- **F2（major）2 回目の Stop が 1 回目の Stop で手渡した分を消す．** 2 回目の Stop の cut が今の stop sample になり，`cut_segments` が手渡し済みの未来の segment を消す．直し方：Stop の cut に，閉じた burst の前線（前の Stop が覚えた H + 2 など，同じ SampleClock のもの）も入れる（`stop_tx` の `tx_handed_front(..., false)` を `true` にするのが一番小さいはず．それで足りるかを確かめる）．テスト：走っている repeat と未来の repeat のそれぞれで，同じ瞬間と少し後の 2 回目の Stop．record と medium の sample（手渡した分が出て，その後は無音）を assert する．走っている repeat の medium のテストは計画にあったのに無いので，これで一緒に足す．MR-25 の文を直す（閉じた前線も cut に入る）．mutation の行を一つ．
- **F3（major）送信が一部だけ受け取られたとき，`abandon` が受け取られる前の cursor を覚える．** `send` の `Ok(sent)` で `sent < sending` の枝は `open.next` を進めずに `abandon` を呼ぶので，前線が `open.next + 2` になり，受け取られた sample の中に次の timed start を許す．直し方：`abandon` に受け取られた数を渡して cursor を `open.next + sent` にする（continuation の tail を送った分があればそれも数える）．`Err` で受け取られた数が分からないときは，送ろうとした数を全部受け取られたとみなす（保守的）．テスト：FakeDevice の short send（`device.rs` の部分受け取り）のあと，受け取られた範囲の中の B が late，`cursor + 2` の B が on time．失敗した send（`Err`）でも一つ．これで P657 の候補（`+2` → `+1`）を殺す行として戻す．
- **F4・O1（minor）`design/18` の UR-21・UR-23 の *Test:* 行から，コードに残っているテストの名前と説明が消えた．** HEAD の行を元に戻し，新しいテストの名前は**足すだけ**にする．戻すもの：`ur_21_an_untimed_send_is_on_time_after_its_delivery`，`ur_23_a_burst_alone_is_ended_before_it_runs_dry`，`ur_23_a_one_sample_burst_booked_ahead_starts_at_its_time`，`ur_25_a_transmit_poll_during_a_control_open_starts_no_streamer_call`，`ur_25_a_from_none_enable_waits_for_a_transmit_report_read`，各テストの括弧の説明，FakeDevice が終わっていない burst の中の start-of-burst を数える文．ただし今回の変更で偽になった説明（例：`ur_21_a_burst_inside_the_in_flight_window_is_late` の説明）は今の挙動に合わせて書き直す．`design/07`・`design/09` の *Test:* 行も，HEAD にあってコードに残っている名前を落としていないか grep で確かめる．
- **O2（minor，確かめる）一つの held burst を `dispatch_start` が 2 回判定することがある．** uhd-tx の `step` は，`dispatch_start` が `window_cut` 以上へ動かした burst を held に戻す（`if held.k >= window_cut`）．`preempt` も cursor の後ろへ動かした burst を held に残す．どちらも次の手渡しで `dispatch_start` がもう一度走り，uhd-tx が遅れると 2 回目の `TIME_ERROR` を出して動かし直す．FakeDevice と手動の時計で起きるかを確かめる．起きるなら一度だけ判定する形に直し（late と決めた burst の印を持つなど，一番小さい形で），`TIME_ERROR` が 1 回だけのことを assert するテストを置く．起きないなら理由を report に書く．

## 2. 確かめること

- 新しいテストと直したテストをそれぞれ 20 回．`cargo test --workspace`，`cargo clippy --workspace --all-targets -- -D warnings`，`PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`，`git diff --check`，`rm_20_schema_freeze`．
- mutation：新しい行（F1，F2，F3，O2 の分）と，変えたコードや killing test に触れる既存の行だけを `remote_mutate.py` で流す（`impl-brief.md` §3 のコマンド．前に変えたファイルと新しいファイルを mode 644）．全リストは流さない．
- セルフレビューを 1 回（`review-4.log`，`impl-brief.md` §4 のコマンドと `checklist.md`）．P1 以上が出たら直してもう 1 回まで．

## 3. 報告

`fix1-report.md`．最初の行は `STATUS: done`（止まったなら `STATUS: blocked` と理由）．指摘ごとに直したこと（`path:line`），足したテストと assert，mutation の行と結果，確かめたことの結果，セルフレビューの指摘と扱い，残ったこと．**起きたことを書く．していないことは書かない．**


## 修正 1 回目の続き（`fix1b-brief.md`）

あなたは実装役です．`fix1-brief.md` で始めた修正の回（前のセッション）は，セルフレビュー 4 回目（`review-4.log`）の指摘を直している途中で止まりました．前のセッションは途中で文脈を失い，**自分自身の実行**を「別の run がまだ動いている」と取り違えて待ちに入りました．**今この tree を触っている他の agent はいません．他の process を待たない．`pgrep`／`ps` で他の run を探さない．**

`fix1-brief.md` と `impl-brief.md` §6 の守ることはそのまま効く．**文脈を守る**：`fix1.log`（86 万字）・`impl*.log`・review の log を全部読まない．必要なら `grep -n` で一部だけ．cargo とテストの出力はファイルに出して `tail` で短く見る．

## 0. 今の状態

- `fix1-brief.md` §1 の F1，F2，F3，F4／O1，O2 は直してある．mutation の行 P631，P657，P661〜P667 は `fix1-mutations.log` で killed．
- `review-4.log` の最後の部分が 4 回目の指摘（P1 一つ，P2 三つ，P3 五つ）．前のセッションはその一部を直した（`git diff` で今の形を見る）：Mock の held cut の続きで閉じた前線を覚えない（`mr_17_held_cut_continuation_does_not_remember_a_closed_front`），クロックをまたぐ比較の assert，使われない `closed` 引数，late policy の写像の共通化．どこまで終わったかは diff から確かめる．

## 1. 裁定

- **Kernel を変えない．** 前のセッションは `crates/ezsdr-kernel/src/stream/burst.rs` に公開の `LatePolicy::outcome` を足した．Kernel の公開の面は増やさない（AGENTS.md §3）．`burst.rs` を HEAD に戻し（`git checkout` ではなく手で戻す．`git diff crates/ezsdr-kernel` が空になること），写像は `ezsdr_radio::timeline` の関数一つにして Mock と UHD の両方から使う．
- **review-4 P1（UHD の tail を 2 回数える）：確かめてから直す．** `ur_23_a_continuation_tail_is_counted_at_recovery` を流し，保留した最後の sample が `open.next` にすでに数えられているかを，FakeDevice が受け取った sample の数と cursor で確かめる．二重なら直し（前線は受け取られた最後の sample の次 + 2），テストの期待値と P665・P666 を合わせる．二重でないなら理由を report に書く．
- **review-4 P2-2（Mock の held cut の続きで閉じた前線）：** 直したなら，mutation の行を一つ足して killed を確かめる．
- **review-4 P2-3（写像の重複）：** 上の Kernel の裁定どおり `timeline` に一つ．`H > current かつ target < H` の判定も同じ関数に入れられるなら入れる（無理に入れない）．
- **review-4 P2-4（クロックをまたぐ tick の比較）：** テストの assert を同じ clock の値どうし，または host の instant どうしの比較にする．
- **review-4 P3-5（W＝0 の UHD）：** UHD Module の profile は x3xx だけで，W＝0 にならない（`tx_in_flight_window_covers_device_lead`）．退けて理由を書く．
- **review-4 P3-6，P3-7：** 使われない引数は消す．`test-plan.md` は orchestrator の記録なので書き換えない．名前のずれは report に一覧で書く．
- **review-4 P3-8：** UR-23 の INFERRED の注記は残す（前の裁定と同じ）．Changes の行が UR-22 を挙げていて UR-22 の本文が変わっていないなら，行から UR-22 を外す．
- **review-4 P3-9：** `preempt` の隣の判定の変更と，`TIME_ERROR` を一回にする変更が design/18 の文に入っているか確かめ，無ければ UR-21／UR-23 に一文ずつ足す．

## 2. 確かめること

- 変えたテストと新しいテストをそれぞれ 20 回．`cargo test --workspace`，clippy -D warnings，`rules.py --base HEAD`，`git diff --check`，`rm_20_schema_freeze`．
- mutation：今回変えたコードや killing test に触れる行と新しい行だけを `remote_mutate.py` で流す（前に mode 644）．
- セルフレビュー 5 回目を一度（`review-5.log`，`impl-brief.md` §4 のコマンド）．P1 が出たら直す．もう一度は回さない．

## 3. 報告

`fix1-report.md` を書く（`fix1-brief.md` §3 の中身）．F1〜F4，O1，O2，review-4，review-5 の指摘ごとに，直したこと（`path:line`）か退けた理由．前のセッションが止まったことも書く．最初の行は `STATUS: done`．**起きたことを書く．していないことは書かない．**


## 修正 2 回目（`fix2-brief.md`）

あなたは実装役です．tree の未 commit の変更は設計メモ 40 の実装と修正 1 回目です（経緯は同じ作業 dir の `fix1-brief.md`，`fix1b-brief.md`，`fix1-report.md`）．Sol の見直し（`sol-fix1.log` の最後の `VERDICT` 以降）が二つ見つけました．両方を直します．**この tree を触っている他の agent はいません．他の process を待たない．**

`impl-brief.md` §6 の守ることはそのまま効く．文脈を守る：長い log（`fix1.log`，`fix1b.log`，`impl*.log`，review の log）を全部読まない．cargo とテストの出力はファイルに出して `tail` で短く見る．

## 1. 直すこと

- **G1（major）Mock の Stop が，手渡していない payload の sample を窓の外まで残す．** F1 で 1 sample の burst の前線を end + 1 にしたため，Stop の cut（payload を残す位置）に「次を安全に始められる位置」が混ざった．例（1 Msps，受け取りの時刻 0，窓の端 sample 10 000）：B を 10 000 に予約 → 1 sample の A を 9 999 に予約 → 同じ瞬間に Stop．A の前線 10 001 が cut になり，B が開かれ，B は窓の端なので block も record も出ないのに，`cut_segments` が B の segment を 10 001 まで残して medium が B の sample を出す．
  直し方：Stop が payload を残す cut は「手渡した payload の終わり」（窓の端と各 burst の exclusive end の小さい方．+1／+2 の安全な隙間を足さない）と stop sample の大きい方にする．安全な隙間（+1，+2）は後の予約の判定にだけ使う前線として別に覚える．前の Stop が覚えた前線も，payload の cut ではなく予約の判定にだけ効くようにする．ただし F2（2 回目の Stop が 1 回目で手渡した分を消さない）は保つこと：1 回目の Stop の payload の cut を別に覚えて，2 回目の Stop の cut の下限にするなど，一番小さい形で．
  テスト：上の並びを Mock で（record，B が取り消されること，medium の B の位置が無音），対の FakeDevice で（B の sample が送られない）．F2 の 2 回の Stop のテストが通り続けること．MR-25 の文を直す（payload の cut と予約の前線を分けて書く）．mutation の行を一つ．
- **G2（minor）O2 の直しが `preempt` に届いていない．** `dispatch_start` はすでに late と知らせた burst に 2 回目の `TIME_ERROR` を出さないが，`preempt`（`tx.rs` の SendAsap の枝）は出す．例（1 Msps，時刻 0）：repeat A を 3 000 から cursor 7 000 まで手渡す → SendAsap の B を 1 000 に予約 → control が B を 2 000 へ動かして 1 回目の `TIME_ERROR` → `preempt` が B を 7 000 へ動かしてもう一度出す．直し方：`dispatch_start` と同じ「すでに知らせた」の判定を `preempt` にも当てる（同じ判定を二度書かないなら一番よい）．テスト：この並びで `TIME_ERROR` が 1 回，`requested_target` が 1 000 のまま，B の続きの sample が 7 000 から．mutation の行を一つ．

## 2. 確かめること

- 新しいテストと直したテストをそれぞれ 20 回．`cargo test --workspace`，clippy -D warnings，`rules.py --base HEAD`，`git diff --check`，`rm_20_schema_freeze`．
- mutation：今回変えたコードや killing test に触れる行と新しい行だけを `remote_mutate.py` で流す（前に mode 644）．
- セルフレビューは回さない（Sol がこの直しを見直す）．

## 3. 報告

`fix2-report.md`．最初の行は `STATUS: done`．G1，G2 ごとに直したこと（`path:line`），テストと assert，mutation の行と結果，確かめたことの結果，残ったこと．**起きたことを書く．していないことは書かない．**


## 修正 3 回目（判断 D5）（`fix3-brief.md`）

あなたは実装役です．tree の未 commit の変更は設計メモ 40 の実装と修正 1・2 回目です（経緯は同じ作業 dir の `fix1-brief.md`，`fix2-brief.md`，`fix2-report.md`）．**この tree を触っている他の agent はいません．他の process を待たない．** `impl-brief.md` §6 の守ることはそのまま効く．文脈を守る：長い log を全部読まない．cargo とテストの出力はファイルに出して `tail` で短く見る．

## 0. なぜ

Sol の見直し（`sol-fix2.log` の最後）が H1 を見つけた：held の続きで Kernel の burst が閉じると，Stop が空の EOB の H＋2 を失い，Mock が H＋1 を通す（UHD は late）．これは F1・F2・G1 に続く同じ仕組み（Mock が UHD の device burst の枠を推し量る）の四つ目で，owner が設計を変えた．**設計メモ `plan/phase8/40-preemption-bound.md` §7 の判断 D5 を読む**（このファイルは orchestrator のもの．書き換えない）．

## 1. やること

- **Mock（`crates/ezsdr-mock-radio/src/lib.rs`）**：`tx_in_flight_ns > 0` の profile で，予約の前線を「D2'' の `handed`（先に受け取った，開始が窓の端より前の burst の `min(終わり, 窓の端)` の最大．repeat は窓の端）＋ 2 sample」にする．
  - 1 sample の burst の end＋1（F1 で足した `handed_end` の場合分け），閉じた burst を出し終えた時に覚える end＋1，held の続きで閉じた前線を覚えない場合分け，Stop の `open.next + 2` などの場合ごとの規則は消し，一律の ＋2 に置き換える．
  - Stop が payload を残す cut は `handed` と前の Stop の cut の大きい方（余裕を足さない．G1 と F2 の直しを保つ）．Stop の後に残す予約の前線は cut ＋ 2．
  - cold change と loss は今どおり瞬間で切り，前線を残さない．ideal（W＝0）は HEAD と同じ（前線なし）．
  - 消したものに使っていた field，関数，mutation の行は消すか当て直す（消した行の ID は再利用しない）．
- **UHD は変えない**（G2 の直しも含めて今のまま）．
- **テスト**：Mock の境界のテストの期待値を一律の ＋2 に合わせる（H が H＋2 へ，end＋1 が end＋2 へ，など）．Mock と FakeDevice の対のテストは「Mock は UHD と同じか厳しい」を assert する形にする（UHD が on time で Mock が late の対は，そう書く）．H1 の並び（B を 10 000，1 sample の A を 9 999，Stop，C を 10 001 と 10 002）を Mock で足す：Mock の cut は 10 000 なので前線は 10 002．C は 10 001 で late，10 002 で on time．FakeDevice の対は H1 の Sol の説明どおり 10 001 late，10 002 on time．
  - 消した場合分けを試していたテストは，一律の規則の下での期待値に直す．消さない．
- **spec**：`design/09-mock-radio.md` MR-17 と MR-25 を一律の ＋2 の形に書き直す（場合分けの文を消す）．`design/07-radio-model.md` RM-15 の「equality is on time」は Provider の規則として残し，Mock の余裕は MR-17 が述べる．**H2**：`design/18-uhd-radio.md` UR-23 の「1 sample の burst はその buffer に EOB」と「空の buffer で終わるのは途中で止めた repeat だけ」に，予約済みの続きがある場合の例外（続きが取り消されると空の EOB で閉じる）を足す．Changes の行を今の形に合わせる．
- 規模の目安：Mock の本体は今より短くなるはず．

## 2. 確かめること

- 直したテストと新しいテストをそれぞれ 20 回．`cargo test --workspace`，clippy -D warnings，`rules.py --base HEAD`，`git diff --check`，`rm_20_schema_freeze`．
- mutation：Mock の前線と Stop の cut の行（一律の ＋2 を ＋1 や ＋0 にする変異，Stop の cut に余裕を足す変異など）を足すか当て直し，触れた行と一緒に `remote_mutate.py` で流す（前に mode 644）．全 mutation list の `old` が tree でちょうど 1 回一致すること．
- セルフレビューを 1 回（`review-6.log`，`impl-brief.md` §4 のコマンドと `checklist.md`）．P1 が出たら直す．もう一度は回さない．

## 3. 報告

`fix3-report.md`．最初の行は `STATUS: done`．消した規則と field，変えたテストの期待値の一覧（前 → 後），新しいテスト，spec の変更，mutation の行と結果，確かめたことの結果，セルフレビューの指摘と扱い，残ったこと．**起きたことを書く．していないことは書かない．**


## 修正 4 回目（`fix4-brief.md`）

あなたは実装役です．tree の未 commit の変更は設計メモ 40 の実装と修正 1〜3 回目です（Sol は修正 3 回目で PASS）．orchestrator の gate で，mutation の全リストに設計メモ 40 で増えた生き残りが 3 つ出ました（G09 は前からの既知）．**この tree を触っている他の agent はいません．他の process を待たない．** `impl-brief.md` §6 の守ることはそのまま効く．長い log を全部読まない．cargo とテストの出力はファイルに出して `tail` で見る．`plan/phase8/` の orchestrator のファイル（`40-preemption-bound.md`，`reviews/40-*.md`）は触らない．

## 1. 生き残り（どれも sim02 で再現した．flake ではない）

- **P639**（`plan/maintenance/tools/mutations.json`）：`remember_tx_front` の `if self.profile.timing().tx_in_flight_ns == 0 { return; }` を消しても `mr_17_ideal_closed_finite_burst_has_no_extra_gap` が通る．予約の前線の計算（`lib.rs` の `tx_in_flight_ns == 0` で前線を返さない所，1402 行付近）が同じことをしているので，この guard は冗長に見える．`front` を読む所をすべて確かめ，W＝0 で覚えた `front` がどこにも効かないなら，guard を消して P639 の行も消す（ID は再利用しない）．効く所があるなら，それを観測するテストに filter を当て直す．
- **B18**（`plan/phase7/tools/mutations.json`）：一部だけ受け取られた send のあと，保留した最後の sample を EOB つきで送る変異が `ur_22_a_send_cut_short_ends_the_device_burst_after_what_it_took`（`crates/ezsdr-radio-uhd/tests/fake.rs`）で殺されなくなった．見込み：D3 で send が窓で切られるようになり，このテストの短く切られた send が burst の最後の buffer でなくなって，最後の sample を保留しない（tail が無い）．確かめ，テストの場面を「短く切られた send が最後の sample を保留する最後の buffer」になる形に直す（burst の終わりを窓の内側に置くなど）．テストの元の主張（受け取られた分の後で device burst を閉じ，残りを送らない）は保つ．
- **E21**（`plan/phase5/tools/mutations.json`）：responder が block ごとに rearm の状態を忘れる変異が `v58_12_a_reactor_answers_whatever_the_block_lengths`（`crates/ezsdr-acceptance/tests/reactive.rs`）で殺されなくなった．HEAD では殺されていた．このテストは x3xx の Mock を使うはずで，設計メモ 40 の Mock の送信の変更（割り込みの前線，Stop が手渡した分を流す）でテストの場面の何かが変わり，rearm を忘れても結果が同じになったと見る．HEAD と今でテストの中の burst と応答の数・時刻がどう変わったかを調べ（HEAD の版は `git show HEAD:<path>` で読む．tree を HEAD に戻さない），元の力を取り戻す形にテストを直す（例：rearm を忘れると 2 度目の応答が出る並びになるよう場面を合わせる）．テストの主張を弱めない．

## 2. 確かめること

- 直したテストをそれぞれ 20 回．`cargo test --workspace`，clippy -D warnings，`rules.py --base HEAD`，`git diff --check`．
- `remote_mutate.py` で P639（消したなら不要），B18，E21 と，直したテストを filter に持つ他の行を流して killed を確かめる（前に変えたファイルと新しいファイルを mode 644．コマンドは `impl-brief.md` §3．list は `maintenance,phase5,phase7` のように並べる）．
- セルフレビューは回さない．

## 3. 報告

`fix4-report.md`．最初の行は `STATUS: done`．三つそれぞれ：原因（何が変わって殺されなくなったか，`path:line`），直したこと，mutation の結果．**起きたことを書く．していないことは書かない．**

