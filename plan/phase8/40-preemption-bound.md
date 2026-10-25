# 40 — 走っている burst の preemption の bound（Phase 8 段階2）

Status: 完了（2026-10-26）．D1〜D4 と §6 の D2''，§7 の D5（owner 2026-10-26）で実装した．経過，裁定，数は [reviews/40-opus.md](reviews/40-opus.md)．§3 の期待値は計画の時のもの：Mock の境界は D5 で前線＋2 sample（例：系列 9 の H ちょうどは Mock では late，H＋2 から on time）．

## 1. 問題

MockRadio は走っている burst に割り込む burst を `now + device lead`（x3xx の Mock は 2 ms）で始めさせる．UHD Module は sample を in-flight window（`crates/ezsdr-radio-uhd/src/profile.rs` の `IN_FLIGHT_WINDOW_NS`，10 ms）先まで device に渡していて，渡した sample は取り戻せないので，割り込む burst は「まだ渡していない最初の sample」より前には始められない（RM-15 の第3段，UR-21，UR-23）．Phase 7 の bench では提出から約 10.4〜11.3 ms 要った（`hw_b8_preemption`：20 ms は通り，10・5・3 ms は drop．VERIFIED，`plan/phase8/bench-results.md:55`）．Mock が実機の拒否するものを通すので，Vision §59 の bug（`00-overview.md` の Mock が緩い表）．

## 2. 決定

**D1（orchestrator，設計メモ 38 D1 の前例どおり）：in-flight window を profile の timing の値にする．** `radio.timing.tx_in_flight_ns`（TimingEnvelope の8つ目）：x3xx の profile は 10 ms（INFERRED．UHD Module の今の定数．段階4で bench から決め直す），ideal は 0．UHD Module は定数をやめてこの値を読む．RM-15 の「Provider の spec が in-flight window を述べる」はこの値になる．

**D2（§5 の D2' に置き換えた．Sol の計画レビュー P1〜P4）：MockRadio は UHD Module の hand-over の前線（front）を真似る．** 受け取りの瞬間 `now` に，Mock が決めてある burst（走っている burst と held の burst）のうち，開始が `now + W`（W＝`tx_in_flight_ns`）より前のものについて，`min(その burst の終わり, now + W)` の最大を **front H** とする（UHD はそこまで device に渡している．繰り返しの burst に終わりはない）．そういう burst がなければ front はない．target が H より前の burst は，H を「まだ渡していない最初の sample」として RM-15 の第3段どおり決める：late で，`send_asap` は H へ動き（`TIME_ERROR { send_asap }`），`drop_and_flag` は捨てる（`TIME_ERROR { drop }`）．front と今の判定（RM-14 の decision instant と device lead）の遅い方が効く．W＝0 では H ≤ `now` なので，今の振る舞いと変わらない．

## 3. 系列（期待値．W＝10 ms，Mock の device lead L＝2 ms，UHD は L＝2 ms と配送 3 ms）

| # | 場面 | 期待 | 根拠 |
|---|---|---|---|
| 1 | 繰り返しの A が走っている．`now` に `send` の B，target `now + 5 ms` | B は drop，`TIME_ERROR { drop }`．A は続く | RM-15，`hw_b8_preemption`（VERIFIED） |
| 2 | 1 と同じで B は `start_repeat`（`send_asap`） | B は H＝`now + 10 ms` 以後の最初の sample へ動き，`TIME_ERROR { send_asap }`，`requested_target` は元の target．A は B の動いた開始で終わる | RM-14，RM-15 |
| 3 | B の target が `now + W` 以後 | on time．A は B の target で終わる | RM-15 |
| 4 | 繰り返しでない A が `now + 3 ms` で終わる．B の target は `now + 5 ms` | on time（front は A の終わり＝`now + 3 ms`） | UHD は A を渡し終えて次を device lead で決める（`tx.rs` の `dispatch_start`，INFERRED） |
| 5 | 繰り返しでない A が `now + 30 ms` で終わる．B の target は `now + 5 ms` | late（front は `now + 10 ms`） | RM-15 |
| 6 | burst が何もない．B の target は `now + 5 ms` | on time（front なし） | RM-14 |
| 7 | 走っている burst はなく，held の A が `now + 4 ms` に始まる．B の target は `now + 6 ms` | late（UHD は A を `start − now < W` で開き，`now + W` まで渡す） | `tx.rs:347-372`（INFERRED，コードの読み） |
| 8 | 7 で B の target が A の開始より前（`now + 3 ms`） | late（A を渡し始めているので，それより前には入れない） | 同上（INFERRED） |
| 9 | B の target がちょうど H | on time | RM-15（H は「まだ渡していない最初の sample」） |
| 10 | ideal の profile（W＝0） | 今の振る舞い（`now + L` と origin の bound）と同じ | D2 |
| 11 | 繰り返しでない A を渡し終えたあと（A の終わり ≤ `now + W`），B の target が A の終わりより前で `now + L` より後 | RM-15 では late（A の終わりより前は渡し終えた範囲）．UHD は `dispatch_start` が今の瞬間に対してだけ決めるので on time にして device で遅れる見込み（INFERRED）．どちらが正しいかテスト計画で確かめ，UHD が RM-15 と違えば直す | RM-15 |

## 4. 変わる spec とテスト（見込み）

- spec 07：RM-4（key の表），RM-6（timing の capability が8つ），RM-15（Mock の bound と in-flight window の出所），RM-20（envelope の timing の member が8つ），RM-29（instance の capability の並び，ideal の値）．`schemas/radio/envelope.v1.json` を再生成．
- spec 09：MR-3（Vocabulary の timing），MR-17（front での判定）．
- spec 18：§3 の定数（in-flight window は profile の値），UR-21，UR-23．
- 各 spec の Changes に1行．version は上げない．

## 5. テスト計画と Sol の計画レビュー（2026-10-25）

テスト計画：Luna（`~/.cache/ezsdr-fixes/2026-10-25-preempt/test-plan.md`）．系列 11 の UHD のずれ（終わりのある burst を渡し終えると front を失い，`dispatch_start` は今の瞬間と device lead だけで決める，`tx.rs:579-585`）をコードで確かめた．Sol の計画レビュー：**FAIL**（major 5，minor 1）．どれも D2 が UHD の hand-over を単純にしすぎていたという指摘で，採る．

| # | 指摘 | 裁定 |
|---|---|---|
| P1 | held の A を「渡し済み」とみなすのは誤り．uhd-tx は queue を先に読み切ってから進む（`tx.rs:136-150`）ので，同じ周回で予約した A（+4 ms）と B（+3 ms）は B が先に on time で出る | 採る．Mock は「B より前の瞬間に受け取った」burst だけを渡し済みの候補にする（同じ瞬間に受け取ったものは UHD が読み切ってから決めるので数えない）．両方の順（同じ瞬間／A が先の瞬間）をテストする |
| P2 | `now + W` は UHD の実際の front ではない．UHD は buffer の**始まり**を W と比べ，chunk ごと送る（`tx.rs:319-323,385-395`）ので W を最大 1 chunk 越える | 採る．**UHD 側を W の厳密な上限にする**（送る chunk の終わりを `now + W` 以後の最初の sample で切る）．UR-23 の「W より先には渡さない」は今は chunk の分だけ偽で，これで真になる（判断 D3） |
| P3 | 系列 11 の「渡し終えた」場面は，UHD が最後の sample を保持する（`tx.rs:405-413`）ことを無視している．device burst を終えた後にちょうど終わりで始めるのは失敗する（UR-23，`hw_b8_raw_empty_eob_gap`） | 採る．系列 11 を「最後の sample を保持中」と「EOB を送った後」に分け，device の sample と境界の metadata を assert する．Mock の front は終わりで（UHD の「終わり − 1 sample」より1 sample 厳しい側，緩くはない），EOB の後の終わりちょうどの開始は Mock も late にする |
| P4 | front の状態の寿命が試されていない：渡した後の Stop → 残りの区間の B，古い clock の front → cold change → 新しい clock の B，stream 0 の front → stream 1 の B，front が過ぎた後の B | 採る．4つともテストする．Mock の Stop は `open` と `held` を消す（`lib.rs:1142-1161`）が，UHD は渡した sample を取り戻せない（RM-16）．Mock は Stop の後も渡し済みの範囲を front として持つ（判断 D4） |
| P5 | `moved_duplicate` の数が誤り（A は2 **sample**，終わりは 2,003,002）．両方を同じ step で予約しているので P1 の順の場面でもある | 採る．P1 の規則では A と B は同じ瞬間の受け取りなので A は数えず，B は今どおり device lead で A の開始へ動いて重なりで拒否される．**既存のテストは変えない** |
| P6 | W＝0 の UHD のテストは成り立たない（W＝0 では held の burst が開始を過ぎてから開かれ late になる，`tx.rs:353-360`） | 採る．W＝0 は Mock の ideal だけで見る．UHD は W を変えた description で値を読むことだけ見る |
| 共有 | `tx_preemption_front(window_end, spans)` は P1〜P4 を解かない．UHD は実際の cursor を持つ | 採る．共有するのは「H に対して決める」境界と判定の計算だけ．UHD は自分の cursor，Mock は下の D2' の真似を持つ |

**D2'（D2 を置き換える）：MockRadio は UHD の hand-over の cursor を真似る．** stream ごとに，ある瞬間 `t` の渡し済みの終わり `handed(t)` は，`t` より前の瞬間に受け取った burst（走っているもの，held のもの）のうち開始が `t + W` より前のものの `min(終わり, t + W)` の最大（繰り返しの burst に終わりはない）．`t` に受け取った burst の target が `handed(t)` より前なら RM-15 の第3段どおり late で，`send_asap` は `handed(t)` へ動き，`drop_and_flag` は捨てる．渡し済みの範囲は Stop や cold change で消えない：Stop の瞬間 `s` の `handed(s)` を，その時刻が過ぎるまで front として持つ（D4）．比べるのは同じ SampleClock の上だけ．

**判断 D3（owner 2026-10-25，推奨どおり決定）：UHD Module の in-flight window を厳密な上限にする．** 送る chunk の終わりを `now + W` 以後の最初の sample で切る．利点：宣言した W が本当の上限になり，Mock の真似と Stop の終わりの bound（RM-16，UR-23）が一つの値で決まる．欠点：hand-over のコード（`send`）に手が入る．退けた案：越える分（最大 1 chunk＝W/4 か `block_len`）を Mock が真似る（chunk の大きさという UHD の内部を語彙に持ち込む）．

**判断 D4（owner 2026-10-25，推奨どおり決定）：Mock の Stop も in-flight window を真似る．** 渡した後の Stop の瞬間 `s` に，Mock の走っている burst は `handed(s)`（`s + W` 以下）まで送ってから終わる（RM-16 が手渡しする Provider に許す終わり方）．利点：Mock と UHD の記録が同じ形になり，「Stop の後も残る front」（P4）が Mock の中で筋が通る．欠点：Mock の Stop の記録が今より遅く終わる（x3xx で最大 10 ms）．既存の Stop のテストの期待値が変わりうる．退けた案：Mock は Stop で即座に終わり，front だけ持つ（Mock の記録では何も送っていない区間で B を late にすることになり，説明がつかない）．ideal（W＝0）では今と同じ．

## 6. 改訂した計画と Sol の再レビュー（2026-10-25）

改訂したテスト計画（Luna）に Sol は **FAIL**（major 3，minor 1）．P5，P6，共有の関数は解消，P1〜P4 は一部．採る．

| # | 指摘 | 裁定 |
|---|---|---|
| Q1 | 同じ瞬間でも hand-over は起こりうる．uhd-tx は queue を周回ごとに読む（`tx.rs:136-150`）ので，時計を止めたまま A を予約・手渡しし，その後 B を予約すれば B は late．D2' の「前の瞬間に受け取ったものだけ」は Mock を緩くする | 採る．**Mock は UHD と等しくはできない**（どの周回で手渡したかは受け取りの時刻からは分からない）ので，**保守的な側に寄せる**：受け取りの順で B より前に受け取った burst は，同じ瞬間でもすべて手渡し済みの候補にする（D2''）．同じ周回で予約した A と B（P1 の第一の順）では，UHD は B を on time で出し，Mock は late にする．Mock の方が厳しく，緩くはない（Vision §59）．この違いはテストに両方の期待値として書く |
| Q2 | Stop で止めた繰り返しは held の sample がなく，UHD の空の EOB が H に zero sample を1つ足す（`tx.rs:540-549`，UR-23）．device burst は H＋1 で終わり，H＋1 の開始も late | 採る．payload の上限は D3 のとおり W で厳密．空の EOB の zero sample 1つは UR-23 が述べる例外として残す．**device burst を閉じた後の安全な開始**：最後の sample を EOB と一緒に送ったなら終わり＋1 sample，空の EOB で閉じたなら H＋2 sample．Mock も同じ境界を使う．止めた繰り返し → B を H，H＋1，H＋2 で試し，padding，timed SOB，sample，report を見る |
| Q3 | D4 の試験が走っている繰り返しだけで，Stop の時にすでに手渡した未来の A（開始が `s` より後）がない．Mock の記録だけでなく放射（medium）も見ていない．MR-25・MR-32 は今「Stop の瞬間で切る」と書いている | 採る．UHD は手渡し済みの A を open として持ち，Stop で手渡した分を流してから EOB で閉じる（`end_open`，`close_device_burst`）．Mock もそれに合わせ，Stop の時に手渡し済みの burst は手渡した分まで流し，それより後と未手渡しの held を取り消す．MR-25 と MR-32 を直し，loopback の sample で遅らせた切れ目までは出て，その後は出ないことを見る |
| Q4 | 期限の過ぎた H を持ち続けても `max(lead, H)` は lead なので，期限のテストはその変異を殺せない | 採る．その変異の主張を消す（状態を消すことは求めない） |

**D2''（D2' を置き換える）**：stream ごとに，受け取りの瞬間 `t` の B に対し，受け取りの順で B より前に受け取った burst（走っているもの，held のもの）のうち開始が `t + W` より前のものの `min(終わり, t + W)` の最大が `handed(t)`（繰り返しに終わりはない）．device burst をすでに閉じた burst は Q2 の安全な開始を使う．target が `handed(t)` より前なら RM-15 の第3段どおり late．Stop の瞬間 `s` の `handed(s)` は時刻が過ぎるまで front として残り（D4），比べるのは同じ SampleClock の上だけ．W＝0 では今の振る舞いのまま．

## 7. 三度目の失敗：Mock が device burst の枠を推し量る（2026-10-26）

実装の後，Sol のコードレビューと二度の見直しが，同じ仕組みで major を四つ見つけた：F1（1 sample の burst は UHD が最初の buffer で EOB を送るのに，Mock は終わりちょうどの再開を通す），F2（2 回目の Stop が 1 回目で手渡した分を消す），G1（F1 の直しで，安全な再開の位置が Stop の payload の cut に混ざり，手渡していない sample が放射される），H1（held の続きで Kernel の burst が閉じると，Stop が空の EOB の H＋2 を失い，Mock が H＋1 を通す）．どれも `mech:radio-rule-implemented-twice` で，MockRadio が UHD の device burst の状態（開いているか，EOB が payload か空か，続きがあるか）を Kernel の状態から推し量り，場合ごとに足した規則が角で食い違う．AGENTS.md §6 の三度目にあたるので，局所の直しを止めてここで問う．

- **判断 D5（owner 2026-10-26，推奨 A に決定）：Mock は device burst の枠を推し量らず，一律の余裕を取る．** in-flight window が 0 でない profile で，Mock の予約の前線は「手渡した payload の終わり（D2'' の `handed`）＋ 2 sample」とする．2 sample は UHD の再開の隙間の最大（続き 0，payload の EOB ＋1，空の EOB ＋2）で，Mock の `handed` は UHD の実際の cursor 以上なので，Mock は UHD より緩くならない．Stop が payload を残す cut は `handed` のまま（余裕を足さない），Stop の後に残す前線は cut ＋ 2．1 sample の burst，閉じた burst の end＋1，held の続きの場合分けは Mock から消える．UHD は変えない（実際の cursor と EOB を知っている）．
  - 欠点：前線ちょうど（H，H＋1）を狙った burst は，UHD が続けて on time にする所を Mock は late にする（最大 2 sample 厳しい）．効くのは，前の burst の手渡しが始まった後（終わりまで W 以内）に受け取った burst だけで，W より前に予約した続きの burst は今までどおり on time．Mock の境界のテストの期待値が H から H＋2 に変わる．RM-15 の「equality is on time」は Provider の規則として残し，MR-17 が Mock の余裕を述べる．
- 退けた案 B：今の真似を保って H1 を直す．一つの段階で同じ仕組みの major が四つ出ており，Mock が見えない device の枠を推し量る限り次の角が残る．
- 退けた案 C：device burst の状態機械を `ezsdr_radio` の純粋な関数にして両方から使う．UHD の状態は実際の send と周回の時刻で進み，Mock はそれを再現できない（D2'' と同じ理由）．大きい割に食い違いの源が残る．
- 残る点（A でも B でも同じ）：uhd-tx は B を受け取りより後の周回で決めるので，その遅れの分だけ UHD の cursor は Mock の `handed(t)` より先にありうる（窓の端の近くを狙う B）．x3xx の W（INFERRED）を Phase 8 で測るとき，この遅れを W に含めて宣言する．
