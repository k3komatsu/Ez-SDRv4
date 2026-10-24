# #59：MockRadio の `cold_change_refused` fault — 実装の指示書（risky）

あなたは実装役です．HEAD は `3109743`，tree はきれい．作業 dir は `~/.cache/ezsdr-fixes/2026-10-24-issue59/`．

先に読むもの：`plan-brief.md`（決定 1〜4），あなたの `test-plan.md`，orchestrator の判断 `plan-decisions.md`，Sol の計画レビュー `sol-plan-verdict.md`．**この指示書 §1 が三つより優先する**．

**作業 dir の orchestrator のファイル（`*-brief.md`，`checklist*.md`，`plan-decisions.md`，`sol-*`，`.*.orig`）は読むだけ．書き換えない．見張りのプロセスは止めない．**

## 1. orchestrator の裁定

Sol の指摘 1〜7 はすべて採った．`plan-decisions.md` の D1〜D8・D10〜D12 も採る．D9 は下の S4 で直す．

- **選び方（D1・D3，確定）**：fault `f` は pending のまま待ち，effective instant `e ≥ f` の `cold` change のうち，**`e` の時点の plan にその `seq` の segment がある**最初のもの（`(e, seq)` の順，`seq` は `Lines` の item の `seq`）を，その `e` で拒否する．origin は見ない．`cut == 0` の segment（TX の 0 sample の segment など）も plan にあれば候補．後の予約で plan から消えた segment の change は候補でない．停止中（off）の line への change は segment を作らないので候補でない．
- **S1 受け取りの時刻（Sol 1）**：`Harness::step(t)` は Authority を `t` に進めてから Action を処理する（`mock_radio.rs:283-285`，`mock_layout.rs:300-302`，`lib.rs:2530-2536`）．各系列で「push の後の最初の step の時刻 = 受け取り」として期待値を計算し直す．系列 2 は `step(e)` で次の `cold` を予約する独立の系列にし，復帰の origin を受け取りから計算する．系列 4 は gain を `step(e)` で予約し，lead（x3xx 2 ms）を入れた instant で期待する（`LATE_COMMAND` を出さない `at` にする）．系列 7 は時刻が単調に増える順に並べ直す（Authority は巻き戻せない）．
- **S2 初期の membership（Sol 2）**：既定で stream 0 に入るのは RX c0 だけ（`channels.rs:514-533,957-958`）．系列 2・3 は `radio.rx.c1.stream = 0`，系列 6 は `radio.tx.c0.stream = 0` を初期の constraint に入れる．系列 2 の「次の `cold`」は halt を本当に解く変更にする（rate など，その stream に効くもの）．系列 6 の旧 clock の burst は，その clock の origin と ratio から cut の後の index を計算する．どの系列も，拒否の前に membership，clock，held の状態に着いたことを assert する．
- **S3 TX の系列を足す（Sol 3）**：(a) cold A・B を前もって予約し，それぞれの clock に burst，A を拒否 → A の clock の burst だけ取り消し，B の clock の burst は残り，B の segment で送信される．予約済みの clock の origin と end は語彙の replan のとおり（前に予約した B が A の clock を既に切っていれば，その end は残る：`timeline.rs:846-853`，`crates/ezsdr-radio/tests/timeline.rs:292-296`）．(b) halt の間に旧の clock や拒否された clock の domain への新しい burst は拒否される．(c) 拒否の時に open な burst があれば，cut で閉じて `BurstRecord` が書かれる（UHD の `tx.rs:718-769` と同じ扱いか比べ，違えば記録）．(d) channel mode（SimulationChannel）で，拒否された burst の信号が受信側に漏れない（1 例）．M9（拒否でも `tx_switch`）を殺す assert を (a) に入れる（current clock を誤って採用すれば B の送信の clock が変わる）．
- **S4 D9 の訂正（Sol 4）**：MockRadio はリンクの無い受信の line に T0 の Stop を予約する（`lib.rs:2421-2427`，spec 09 `:221`）ので，その line の `cold` change は segment を作らず候補にならない．試験：リンクの無い受信 stream への `cold` → fault は使われず，別の走っている stream の `cold` で使われる．明示の `start_rx` の後のリンクの無い stream は扱わない（範囲外と記録）．
- **S5 選び方を見分ける試験（Sol 5）**：(1) x3xx で `e < f < origin` の change は拒否されず，次の候補が拒否される（M13 を殺す）．(2) 2 つの stream で `e` の順と origin の順が違う場合，`e` の順で選ぶ（origin で選ぶ誤りを殺す）．(3) 後の予約で plan から消えた受信の segment の change は候補にならない；TX の 0 sample の segment は候補になる．
- **S6 spec（Sol 6）**：spec の差分は確定した選び方（effective instant，plan の segment）で書く（計画 §4 の 2・5 の「始まる segment」「`origin >= f`」「`cut > 0`」は使わない）．選び方は MR-20 に 1 度だけ書き，他の spec は引く．MR-40（`design/09-mock-radio.md:223`）の「move が拒否されたら applied も clock も変わらない」は予約できなかった move に限り，予約の後の拒否は RM-25・RM-34 を引く．
- **S7 期待値の誤記（Sol 7）**：系列 1 の単位（1 Msps で 3 ms は sample index 3 000，root tick 差 3 000 000）．系列 8 の block の header は stop の前に見るか，stop の後の末尾の block（`lib.rs:1175-1182`）を期待に入れる．

## 2. 範囲

計画（§1 で直したもの）の全部：`FaultKind::ColdChangeRefused`（schema 再生成，**version は上げない，SCHEMA_CHANGELOG に書かない**），MockRadio の選び方・拒否（`Lines::refuse`，`COMMAND_REJECTED`，`rejected`，`applied`，fault の行，`tx_reconcile`），受信 block を切らないこと（`advance`・`emit_rx_until`・`schedule_wakeup` の fault の filter），試験，spec 04・07・08・09 と maintenance spec 22 の follow-up 4，mutation の行．VH-8 の harness は変えない．UHD Module は変えない．

実装で計画に無い状態や失敗の道が増えたら `test-plan.md` に行を足す．計画に無い仕組み（新しい rig，fault の別の種類，retry，`#[cfg(test)]` だけの本番の道）は足さない．既存の部品で作れない系列があれば，その行だけ止めて報告する．

## 3. 確かめること・セルフレビュー・報告・守ること

38a と同じ：`~/.cache/ezsdr-fixes/2026-10-24-note38/impl-brief.md` の §3（20 回，workspace，clippy，rules，`git diff --check`，schema freeze，mutation は `remote_mutate.py ... sim02:2 <ID> ...`，mode 644，負荷をかけない，TMPDIR を変えない），§4（セルフレビュー：reviewer は `openai/gpt-6-luna#max`，checklist は **この dir の `checklist-59.md`**，log は **この dir の `review-59-<n>.log`**），§5（`report.md` をこの dir に），§6（cargo fmt・ローカル mutate.py・commit 禁止など）．

---

# 付録：テスト計画の指示書（plan-brief.md）


あなたは実装役です．今回は**テスト計画を書くだけ**で，コードは変えない（tree は読むだけ）．HEAD は `3109743`．作業 dir は `~/.cache/ezsdr-fixes/2026-10-24-issue59/`．

**作業 dir の orchestrator のファイル（`*-brief.md`，`.*.orig`）は読むだけ．書き換えない．見張りのプロセスは止めない．`git stash`，`cargo fmt`，mutation の道具，commit を使わない．負荷をかけない．**

## 1. 目的と決まっていること

- issue #59（`gh issue view 59 --repo k3komatsu/Ez-SDRv4`）：UHD Module は `cold` change を適用して読み返し，値が違えばその変更を拒否する（spec 18 UR-12 の「Configuring a stream」，UR-13）．拒否された stream は halt し，次の `cold` change か `start_rx` まで続く（maintenance spec 22 VH-2，spec 07 RM-25・RM-34）．MockRadio は拒否しないので，halt の道を Mock で再現できない（spec 07 の Changes 2026-10-09 の RM-25 の行：「MockRadio cannot refuse yet (#59)」）．
- **orchestrator の決定（この計画の前提）**：
  1. `sim.faults` の `FaultKind` に 1 つ足す（名前の案 `cold_change_refused`．`crates/ezsdr-sim/src/lib.rs:33`，spec 08 SE-3）．schema を再生成する．version は上げない．
  2. 意味：fault の `at_ns` 以後に **始まる segment のうち最初の，`cold` change が始めるもの**（その segment の `by` が `Some(seq)`：Run の開始の segment は含まない）を，MockRadio はその instant に拒否する．同じ instant に複数あれば予約の順（`seq`）で最初の 1 つ．fault は 1 度だけ効く．効く segment が無いまま Run が終われば何も拒否しない．
  3. 拒否のしかたは UHD Module と同じ：語彙の `Lines::refuse(stream, seq)`（`crates/ezsdr-radio/src/timeline.rs`）で halt させ，`radio.COMMAND_REJECTED`（`action: null`，kind `"update_parameter"`）と `rejected` の行を出す（UHD は `rx.rs:306-321`，`tx.rs:757-766`，`core.rs` の `command_rejected`）．halt の規則は語彙にあるものを使い，Mock に別の規則を書かない．
  4. 範囲の外：`prepare` での拒否（UR-12 の Rejected），VH-8 の harness に拒否を入れること（計画の最後に「入れるなら何が要るか」を 1 段落で書く．入れるかは orchestrator が決める）．

## 2. 系列（各系列に，期待する結果と，それが拠る規則を書いた）

| # | 系列 | 期待 | 拠り所 |
|---|---|---|---|
| 1 | 受信の stream 0 が走っている．fault `at_ns = t`．`t` より後の instant `e` の rate の `cold` change | `e` で前の segment は切れ（cut は残る），新しい segment は始まらない：`e` 以後の block なし，新しい SampleClock なし．`COMMAND_REJECTED` 1 件と `rejected` 1 行．設定の記録は拒否された値（RM-25，#64） | RM-25，VH-2 |
| 2 | 1 の後に次の `cold` change | halt が解け，新しい segment が lead の後に始まる | RM-25 |
| 3 | 1 の後に `start_rx` | halt が解ける | RM-25，RM-33 |
| 4 | 1 の後，halt の間の `hardware_timed` の更新 | UHD は channel が無い（`no channel`，RM-25 #64）．Mock の行と比べてどうなるか（Mock の行は `{key, value, at}` だけ）を書き，違いがあれば列挙する | RM-25，MR-18 |
| 5 | fault の後に `cold` change が 2 つ | 最初だけ拒否，2 つ目で再開 | 決定 2 |
| 6 | 送信の stream の `cold` change が fault の後 | 送信の stream が halt：その間の burst がどう扱われるか（UHD の `tx.rs` の halt の扱いと同じか）を書く | RM-25 |
| 7 | move（RM-34）が fault の後の最初の `cold` change | 両方の line が halt（`Lines::refuse` が move の両方を拒否） | RM-34 |
| 8 | fault の `at_ns` が全ての `cold` change の後 | 何も拒否しない．fault の記録（`ezsdr.radio.mock` の fault の行）がどうなるかを書く | 決定 2，MR-20 の fault の記録 |
| 9 | 同じ instant に 2 つの stream の `cold` change | 予約の順で最初の 1 つだけ拒否 | 決定 2 |
| 10 | fault と喪失（`device_lost`）が両方ある | 喪失の後は何もしない（MR-20） | MR-20 |
| 11 | ideal と x3xx の両方の profile | どちらでも効く（fault は profile に依らない） | SE-3 |

## 3. 読むもの

- spec：`design/08-simulation.md` SE-3・SE-5，`design/09-mock-radio.md` MR-18・MR-20（fault の扱い）・MR-25・MR-27（記録），`design/07-radio-model.md` RM-25・RM-33・RM-34 とその Changes，`design/18-uhd-radio.md` UR-12（Configuring a stream）・UR-13・UR-25，maintenance spec 22 の VH-2（`plan/maintenance/22-timing-simplification.md`）．
- コード：`crates/ezsdr-sim/src/lib.rs`（`FaultKind`，`FaultEntry`，`FaultsCheck`），MockRadio `crates/ezsdr-mock-radio/src/lib.rs`（fault の適用：`DeviceLost`・`RxOverflow` の道，`advance`，segment の開始，`record_rejected_action`，`emit_event`），`crates/ezsdr-radio/src/timeline.rs`（`Lines::refuse`），UHD の拒否：`crates/ezsdr-radio-uhd/src/provider/rx.rs:295-325`，`tx.rs:740-770`，`core.rs` の `Streams::refuse`・`command_rejected`．既存のテスト：`crates/ezsdr-mock-radio/tests/mock_radio.rs`・`mock_layout.rs` の fault の試験（`mr_20_*`），`crates/ezsdr-radio/tests/timeline.rs` の `rm_34_a_refused_move_halts_both_lines_until_a_change_or_a_start` など．
- 37c の学び（`plan/phase8/reviews/37c-opus.md` §6）：試験は場面に着いたことを assert で見る．

## 4. 書くもの：`test-plan.md`

系列 1〜11 と決定 1〜3 の各項目について表を書く：系列，どこで（MockRadio の統合テスト，語彙，sim の reader），テスト名（新規か既存か．既存なら `path:line`），場面の作り方（`Harness::step` で `now` を固定），assert（block の有無と時刻，SampleClock，event の中身，`rejected` の行，設定の記録，fault の行），観測（どの記録から読むか），落とす誤り（例：Run の開始の segment を拒否する；fault が 2 度効く；`Lines::refuse` を呼ばず Mock が自分で halt を作る；`COMMAND_REJECTED` を出さない；喪失の後に拒否する）．

加えて：schema の再生成と `se_03_faults_reader` の行，spec の変更の一覧（SE-3，MR-20 など，RM-25 の Changes の「cannot refuse yet」），mutation の行の案（変える式と殺す試験だけ），UHD と Mock で振る舞いが違う所（系列 4・6），設計の決定と合わない所・決めていない所（止めずに列挙）．

最初の行は `STATUS: done`（止まったなら `STATUS: blocked` と理由）．**起きたことを書く．していないことは書かない．**

---

# 付録：orchestrator の計画への判断（plan-decisions.md）


対象：`test-plan.md`（DeepSeek）．§7 の未決への判断．

- **D1・D3（fault が効く時）**：fault `f` は pending のまま待ち，**effective instant `e ≥ f` の `cold` change のうち，plan で segment を始める最初のもの（`(e, seq)` の順．`seq` は `Lines` の item の `seq`）**を，その `e` で拒否する．segment の origin は見ない（`origin ≥ f`，`cut > 0` の条件は使わない）．これで D3 の穴（`e < f ≤ origin`）は無くなる．計画 §0.2 の候補の条件 2・3 と §5 の M2（`origin >= f`）・M14 を直す：M2 は `e >= f` を `e > f` にする変異（試験 11 の `e == f`），M13（`f <= e` を落とす）は試験 8（`e = 0.5 ms < f = 1.25 ms` の change が拒否されない）が殺すので記録する．「segment を始める」＝ plan にその `seq` の segment がある（停止中の stream への change は候補にならない．リンクの無い受信 stream の change は候補になる：D9 は plan を見る）．
- **D2**：`rejected` の行と event の instant は `e`（UHD の受信は origin．§6 の違いとして記録する）．
- **D4**：reason は計画の案のまま．
- **D5**：拒否した change の値も `applied` に 1 行書く（UHD と同じ，RM-25 #64）．
- **D6**：選び方は MockRadio の中に置く（語彙に足さない）．
- **D7・D8**：計画のまま（`lost` 0，fault の行は効いた時点に書き `at` は `f`）．
- **D9**：plan を見る（上）．
- **D10**：範囲外のまま．
- **D11**：MR-20 の新しい段落に「この kind は受信 block を切らない」を 1 文入れる．
- **D12**：`Lines` の `seq`（計画のまま）．
- **VH-8**：今回は入れない（計画 §8 の理由に同意）．

---

# 付録：直し 1 回目の指示書（fix1-brief.md）


あなたは実装役です．#59 の未 commit の変更（HEAD `230aba9`）の上で，Sol のコードレビュー `sol-code-verdict.md` の 2 件を直す．守ることと確かめ方は `impl-brief.md` §3 と同じ（セルフレビューの checklist は `checklist-fix1.md`，log は `review-fix1-<n>.log`）．作業 dir の orchestrator のファイル（`*-brief.md`，`checklist*.md`，`plan-decisions.md`，`sol-*`，`.*.orig`）は読むだけ．`plan/maintenance/tools/mutations.json` は**行を足す・直すだけにし，ファイル全体を書き直さない**（字下げは 1 個の空白のまま．前回は全体が 2 個に変わっていた）．

## 1. orchestrator の裁定（2 件とも採る．orchestrator の確認でも同じ 1 を見つけていた）

1. **major：halt の間に，復帰する将来の clock への burst まで拒否する**（`crates/ezsdr-mock-radio/src/lib.rs:1542-1544`）．`line.halted(now)` は `e ≤ now` の予約だけで replay するので，halt を解く次の `cold` change B を予約した後でも B の `e` の前は真になる．UHD の `book_burst`（`crates/ezsdr-radio-uhd/src/provider/control.rs:1078-1093`）は，最後の終わっていない segment と登録済みの clock でこの予約を受ける．直し：入口の一律の拒否をやめ，burst の clock で決める：旧の clock の cut より後の burst と，拒否された change の clock（origin で終わった clock）への burst は拒否し，plan の後の有効な segment の clock への burst は受ける（UHD と同じ）．拒否の理由の文は今のものを使ってよい．`design/09-mock-radio.md:166` の「every transmit burst」をこの範囲に限る．試験：A を拒否 → B を将来の instant に予約 → **B の instant より前に** B の clock へ burst を予約 → B の segment で送信される（burst の記録と送信のサンプル）．halt の間の旧の clock・拒否された clock への burst が拒否されることは今の試験のまま残す．mutation 行：一律の halt の拒否に戻す変異が新しい試験で killed．
2. **minor：spec の「リンクの無い受信 stream の change は候補にならない」が広すぎる**（`design/09-mock-radio.md:166`）．明示の `start_rx` の後は line に Start があり，その後の `cold` change は候補になる（`lib.rs:1498-1508`，`:2023-2027`）．spec を「T0 の Stop のままで，`start_rx` で再開していないリンクの無い受信 stream」に限る．その系列の試験は範囲外のまま（足さない）．

## 2. 確かめること・報告

- `impl-brief.md` §3 のとおり（新しい試験と直した試験を各 20 回，`cargo test -p ezsdr-mock-radio`・`-p ezsdr-sim`，`cargo test --workspace`，clippy，`rules.py --base HEAD`，`git diff --check`，足した行と触ったコードの行を `remote_mutate.py maintenance sim02:2 <ID> ...`）．`git diff --stat plan/maintenance/tools/mutations.json` が足した・直した行の分だけであることを確かめる．
- セルフレビュー：checklist は `checklist-fix1.md`．P1 以上が無くなるか 3 回で止める．
- `report.md` の最後に「## 10. 直し 1 回目」を足す（前の節は消さない）．最初の行は `STATUS: done` か `STATUS: blocked`．**起きたことを書く．していないことは書かない．**
