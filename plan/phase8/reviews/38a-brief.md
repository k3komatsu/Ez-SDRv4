# 設計メモ 38 の段階 38a：実装の指示書（#62，risky）

あなたは実装役です．HEAD は `7a3b553`，tree はきれい．作業 dir は `~/.cache/ezsdr-fixes/2026-10-24-note38/`．

先に読むもの：設計メモ `plan/phase8/38-timed-release-order.md`（全部），あなたの `test-plan.md`，orchestrator の判断 `plan-decisions.md`，Sol の計画レビュー `sol-plan-verdict.md`．**この指示書 §1 が三つより優先する**（計画と食い違えばこちら）．

**作業 dir の orchestrator のファイル（`*-brief.md`，`checklist*.md`，`plan-decisions.md`，`sol-*`，`.*.orig`）は読むだけ．書き換えない．見張りのプロセスは止めない．**

## 1. orchestrator の裁定（計画をこう直して実装する）

Sol の指摘 1〜9 はすべて採った．`plan-decisions.md` の O1〜O6，O8〜O14，F2 も採る．O7 と F1 は下の S4・S2 で置き換える．

- **S1 語彙の関数の表**（計画 §2 D2 (1)）：`now=0, lead=20, window=30` で earlier の 50・51 は窓（≤ 30）の外．表の 95〜98 行を直す：`Some(15),[25]` → `(25,true)`；`Some(25),[25]` → `(25,false)`（床ちょうど）；`Some(25),[26]` → `(26,true)`；`None,[25]` → `(25,false)`；窓の外の行として `Some(45),[50]` → `(45,false)` を残す．ほかの行も式で検算する．
- **S2 harness の Ceiling**（F1 を置き換える）：除外する command は，実装の記録（行が cancelled か）ではなく，**系列から独立に**決める．layer 2 の `run` は release を回す時刻（各 round，1 ms ごとの `now`；loss の `now` では `device_lost` が先なので回らない）を自分で決めているので，それを使う．B が Ceiling ⇔ `e_B < S`（loss）かつ，(e, 予約順) で B より前の command A があって，期待する layout で A の channel が `e_A` に無く後の segment にも無く，かつ `e_A ≤ t < S` の release の時刻 t が無い．channel 無しの先頭 A 自身は除外しない（両 Module で行がある）．
  - `timed`（`(key, value, e)`）は Ceiling の command も含めて**全部**比べる．observed は issued・no channel・cancelled のすべての行（cancelled の行も `at` は e）から作り，expected は loss の前に予約した timed 全部．Ceiling で外すのは正規化した `applied` の行（`(key, claimed, at, cancelled)`）の比較**だけ**．
  - harness の判定が先頭の誤分類を隠さないこと：「channel 無しの先頭を cancelled と記録する」変異の mutation 行を足し，`rm_26_the_timeline_against_uhd_control` と系列 9d の unit test の両方で killed になることを確かめる．
- **S3 喪失で `give` が失敗した command**（O5 を直す）：`held` に残す（戻す）．終わりの処理は D2 (4) の比較をそのまま使う（渡していないので issued にはならない：`e` が確定する側なら O3 の `targets` で `no channel` か Ceiling の cancelled，そうでなければ cancelled）．系列 8b を `e > S`，`e = S`，`e < S` の 3 つに分ける（`e < S`・`e = S` は `ur_24_a_release_after_its_instant_applies_on_arrival` の形：instant を過ぎてから release し，probe `before_timed_apply` で喪失を公開する）．どれも issued の行が無いことを見る．
- **S4 枠の境界**（O7 を置き換える）：D2 (2) の字面どおり，受け取り `r` で **`e > r`** の timed update を数える（instant を迎えたものは数えない）．Mock は `self.updates` をそのまま数えず `e > r` で絞る（step が `e ≤ u` の update を飛び越えても数えない）．UHD は台帳を `e > now` で数える．試験：**11b**（両 Module）16 件を `e = 1…16 ms` で 0 ms に予約し，受け取り 20 ms で 1 件 → 予約できる（Mock は 1 つの `step(20 ms)` で）．**11c**（両 Module）受け取りが最も早い `e` ちょうど → 予約できる；その 1 tick 前 → `Q`．Mock は同じ受け取りで直前に `step(e)` を済ませた場合と済ませない場合の両方で同じ結果．`at` は窓と lead の外に置き late を出さない．
- **S5 window を description から読む試験**：`ur_24_the_release_window_comes_from_the_description`（window 10 ms，A `e = 12 ms`）に，release の側も足す：2 ms の 1 tick 前の `release()` では apply 無し，2 ms の `release()` で timed apply 1 件（`at = 12`），行は 12 ms の後まで無し．床と release の定数化を別々の mutation 行で殺す．
- **S6 終わりの時刻と順**：Mock の系列 8 の loss は loss 8 ms・処理する step 10.6 ms にし，`rejected` の `at` が 8 ms であることを見る．Mock の 9c の停止側は A を適用せずに（`step(10)` を呼ばずに）時計を S に進め，直接 `mock.stop()` を呼ぶ．両 Module に loss→stop，stop→loss，喪失の再通知の系列を足し，S（最初のもの）・理由・行の数が変わらないこと（二重に記録しない）を見る．
- **S7 spec**：計画の一覧に足す：`design/07-radio-model.md` RM-6（TimingEnvelope の数），RM-16（`Provider::stop` で渡した command を issued のまま残す例外 → D2 (4) に合わせる），`design/09-mock-radio.md` MR-3 の profile の値の表（x3xx 3 ms，ideal 0），`design/18-uhd-radio.md:502` の VH-8 の表（`timed` を全系列で比べる）．
- **S8 `finish` の S**：S は 1 か所に 1 度だけ持つ（`streams.stop` か `lost_at`，最初のもの）．S が無いのに `finish` に来る道を黙って `now` で補わない．`finish` の呼び出し元を全部 grep し，S の無い道が本番にあれば止まって報告する．`ur_30_the_recorded_plan_has_no_refused_segment`（`control.rs:2100-2112`）は試験の側で stop の印を明示的に置く．
- **S9 mutation の対応**：系列 3 で「window を大きくする」は殺せない（言わない）．M16 の `late = e > asked` は同値の変異なので行にしない．系列 10 UHD は 7 ms にも `release()` を呼び，その時点で行が無いことを見る．M18 の killing test は 9d．
- **F2**：系列 1 UHD は B の予約の後の 7 ms の `release()` で A も B も渡る．その後の assert は `held` が空，device calls が A→B の順で両方 `at=10`，行は 10 ms の後．
- **メモの D2 (4) の補い**（orchestrator がメモに書く．あなたは spec に書く）：喪失で渡せなかった command は持っていたものと同じ（S3）；装置が拒否した command は終わりでも行を書かない（O6）；枠は `e > r`（S4）．

## 2. 範囲

設計メモ D1・D2 と計画（§1 で直したもの）の全部：語彙の `timed_instant`（`layout_instant` の隣）と `radio.timing.release_window_ns`（`TimingEnvelope`，key 表，capability，x3xx 3 ms，ideal 0，schema 再生成，fixture の期待値．**version は上げない，SCHEMA_CHANGELOG に書かない**），MockRadio（床，枠，判定の順 late→満杯，終わり），UHD Module（床，台帳，行を instant に，終わり．`RELEASE_WINDOW_NS` を消す．**release の条件そのものは変えない**．`released`／`released_at` は残す），harness 2 つ（UHD layer 2，Mock layer 1），generator の `Sequence::schedule`，計画 §3 の既存テストの書き換え，spec 07・09・18（DA-1 の形，各 spec の Changes に 1 行），mutation の行．

実装で計画に無い状態や失敗の道が増えたら，`test-plan.md` に行を足す（計画を伸ばす）．メモ・計画に無い仕組み（新しい rig の種類，時計の hold，fake の新しい fault，retry，`#[cfg(test)]` だけの本番の道）は足さない．既存の部品（`manual_rig_events`，`hold`，`Frozen`，`ApplyFails`，probe）で作れない系列があれば，その行だけ止めて報告する．

## 3. 確かめること

- 新しいテストと直したテストを各 20 回続けて流して全部通る（`timeout 300`，1 binary ずつ，`--test-threads` は既定）．`CARGO_TARGET_DIR` は main tree の既定（`.cargo/config.toml`）のまま．
- `cargo test --workspace`，`cargo clippy --workspace --all-targets -- -D warnings`，`PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`，`git diff --check`，`rm_20_schema_freeze`．
- mutation：新しい行と，変えたコードや killing test に触れる既存の行を `remote_mutate.py` で流して killed を確かめる．`REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py <list> sim02:2 <ID> <ID> ...`（ID は空白で区切る．**sim02・sim03 は別の job で混んでいるので `sim02:2` まで**）．流す前に tree の変えたファイルと新しいファイルを mode 644 にする．各行の `old` が tree で 1 回だけ一致すること．消した・名前を変えたテストを `filter` や `old`・`new` に持つ行を全リスト（`plan/maintenance/tools/mutations.json`，`plan/phase*/tools/mutations.json`）で grep して直す．全リストは流さない（orchestrator の gate）．
- **負荷をかけない**（busy loop，`--test-threads` を上げる，並列の cargo を禁止）．テストの一時ファイルは /tmp（ramdisk）のまま．`TMPDIR` を変えない．

## 4. セルフレビュー

報告の前に，読むだけの reviewer を新しく起こす．1 回ごとに新しい reviewer：

```
timeout 2400 opencode2 run --standalone -m "openai/gpt-6-luna#max" --agent plan --auto "Read ~/.cache/ezsdr-fixes/2026-10-24-note38/checklist-38a.md and do what it says. Your final message is the review." > ~/.cache/ezsdr-fixes/2026-10-24-note38/review-38a-<n>.log 2>&1
```

前景で流す．見つかったものを直して次の回へ．P1 以上が無くなるか 3 回で止める．指摘はすべて「直した」か「退けた（理由）」を `report.md` に書く．

## 5. 報告

`report.md` を書く．最初の行は `STATUS: done`（止まったなら `STATUS: blocked` と理由）．中身：やったこと（ファイルごと），計画から変えた所とその理由，系列ごとのテスト名，既存テストの書き換えの対応，spec の変更，mutation の行（足した行，当て直した行，killed の結果），確かめたことの結果（20 回の数，workspace，clippy，rules，schema_freeze），セルフレビューの指摘と扱い，残ったこと．

## 6. 守ること

- `cargo fmt`・`rustfmt` を流さない（tree は rustfmt の形でない．新しいコードは周りの形に手で合わせる）．
- ローカルで `mutate.py` を流さない．作業ツリーに mutant を当てない（debug の写しは repository の外）．
- commit・push・`git add`・`git stash` をしない．
- 到達できる系列のテストを代わりなしで消さない．differential の操作を縮めない．実時間に頼るテストを足さない（tests/README.md の規則）．
- **起きたことを書く．していないことは書かない．**

---

# 付録：orchestrator の計画への判断（plan-decisions.md）


対象：`test-plan.md`（DeepSeek）。§7 の未決と，orchestrator の読みで見つけた計画の誤り。

## A. §7 の未決への判断

- **O1（§7-4）正規化**：`(key, claimed, at, cancelled)` を採る。`issued`・`note`・`read_back` は落とす。
- **O2（§7-5）記録の形**：各 Module の今の形を保つ。UHD は `applied` の cancelled 行（`cancelled`，`cancelled_at = S`，UR-30），Mock は `rejected`（MR-27）。メモ系列 8 の「行は無い」は「issued の行は無い」の意味。各 harness が自分の形を正規化する。
- **O3（§7-6）S の前に e を迎え渡していない command**：終わりの処理で `targets(dir, chan, e)` を装置を呼ばずに読み，channel が無ければ `no channel` の行，あれば Ceiling（cancelled，`cancelled_at = S`）。
- **O4（§7-7）渡していない command の e == S**：渡したものと同じ比較を先にする（停止は `e ≤ S` で確定，喪失は `e < S` で確定），その後で O3 の結果。停止の e == S で live なら Ceiling，channel 無しなら `no channel`。喪失の e == S は cancelled。
- **O5（§7-8）喪失で `give` が失敗した command**：計画の「行なし」は採らない。装置に渡っていないので持っていたものと同じ：`held` に残す（`give` が成功してから外す，または戻す）。`e > S` は必ず成り立つ（喪失の公開が `give` の前で，`give` は `e` の前）ので cancelled の行。新しい系列 **8b**：`targets` の後，apply の前に喪失を公開（既存の probe `before_timed_apply`）→ `finish` の後に cancelled 行 1 つ（`cancelled_at = S`）。既存の 3623/3640/3656 は `finish` を呼ばないので今のまま。
- **O6（§7-9）装置が拒否した command**：終わりでも行を書かない（`e > S` でも）。拒否は `COMMAND_REJECTED` で既に記録済み。枠は `e` まで。計画どおり。
- **O7（§7-10）枠の境界**：両 Module とも「受け取り `r` で `e ≥ r` の timed update」を数える（`e == r` は占める）。Mock は `self.updates` をそのまま数えるのをやめ，`e ≥ r` で数える：Mock の `step(u)` は Action を `advance(u)` の前に処理するので，step が `e < u` の update を飛び越えると，効いたはずの update が枠を占めたまま満杯になる（今の Mock の緩さでなく厳しさの誤り，VH-8 で食い違う）。新しい系列 **11b**（両 Module）：16 件を `e = 1…16 ms` で 0 ms に予約，次の受け取り 20 ms で 1 件 → 予約できる（Mock は 1 つの `step(20 ms)` で）。**11c**（両 Module）：16 件の最も早い `e` ちょうどの受け取りで 1 件 → `Q`；1 tick 後なら予約できる（`at` は窓・lead の外に置き late を出さない）。
- **O8（§7-12）**：key を RM-4 の表と capability に足す。採る。
- **O9（§7-13，14）**：3.2 の直しを採る。`mr_18_hardware_timed_updates` の 17 件目は `at = 9 ms` にする案（置き換えではなく）。
- **O10（§7-15）reason**：喪失が停止より先なら喪失の理由。`cancelled_at` は最初の S。試験は `cancelled_at` と分類を見て，文字列は今のものを使う。
- **O11（§7-16）**：`Sequence::schedule` を `timed_instant` にそろえる。採る。
- **O12（§7-11）**：Mock の VH-8 layer 1 も同じ床。採る。
- **O13（§7-3）**：床の lead は UHD `DEVICE_LEAD_NS`（2 ms），Mock profile の `min_timed_command_lead_ns`。採る（今の挙動）。
- **O14（§6-2）**：stream command が同じ queue を使うかは 38a の外（D9 と bench）。

## B. 計画の誤り（orchestrator）

- **F1 harness の Ceiling の見分け方**：「`e < S` かつ渡した集合に無い」では，channel 無しの先頭（両 Module とも行がある：UHD `no channel`，Mock applied）まで外してしまう。Ceiling は「UHD の行が cancelled で `e < S`」，かつ `device.calls()` にその command の apply が無いもの，に限る（渡したのに cancelled にする誤りを隠さない）。
- **F2 系列 1 UHD**：B を予約した後の 7 ms の `release()` で A も B も渡る（10 ≤ 7+3）。その後に `held` に A・B が残るという assert は誤り。release の後は `held` が空，`released_at` に 2 件，device calls は A→B の順で両方 `at=10`。
- （計画の最終版で直っていたもの：系列 5b の C は `at=13 ms`，系列 11 の次の 1 件は `at=30 ms` で late にならない。）
- **O7 の補足**：計画の最終版は「両方 `e >= now`」とし Mock は `self.updates` をそのまま数えるとするが，それでは step が飛び越えた `e < u` の update も数えてしまう。Mock は `e ≥ r` で絞って数える（O7 のとおり）。

---

# 付録：直し 1 回目の指示書（fix1-brief.md）


あなたは実装役です．38a の未 commit の変更（HEAD `7a3b553`）の上で，Sol のコードレビュー `sol-code-verdict.md` の 3 件を直す．守ることと確かめ方は `impl-brief.md` の §3・§4・§6 と同じ（ただしセルフレビューの checklist は `checklist-fix1.md`，log は `review-fix1-<n>.log`）．作業 dir の orchestrator のファイル（`*-brief.md`，`checklist*.md`，`plan-decisions.md`，`sol-*`，`.*.orig`）は読むだけ．

## 1. orchestrator の裁定（3 件とも採る）

1. **major：停止の公開の後の release が行を確定する**（`control.rs` の fold）．fold の前の確認を「喪失したか」から「終わりが予約されたか」（streams の lock の下で `ended_at().is_some()`，喪失も停止も）に広げる．終わりが予約されていれば fold をせずに戻り，行は `settle` が S で確定する（`settle` は `e ≤ S` の given を issued に，`e > S` を cancelled にするので，S が now より後の停止でも結果は正しい）．release の条件（いつ・どの channel に渡すか）は変えない．試験：手動の時計の unit test で，A（`e = 10 ms`）を 7 ms に渡し，`streams.stop` に `S = 9.9 ms` を置いた後（run の loop の停止の確認を通った後の形）に 10.6 ms で `release()`，続けて `finish()` → A の行は cancelled で `cancelled_at = 9.9 ms`，issued の行は無い．同じ形で S = 10.2 ms（A は `e ≤ S`）→ issued が 1 行だけ（二重に書かない）．mutation 行：確認を `is_lost()` だけに戻す変異が，この試験で killed になること．
2. **major：harness の Ceiling が channel 無しの後続も外す**（`control/differential.rs:321-328`）．除外の条件に「その command 自身の channel が期待する layout で `e` に在る（live）」を足す．channel 無しの command は先頭でも後続でも外さない．明示系列を 1 つ足す：RX の channel を外し，A・B を RX，C を live な TX へ，どれも `e = 10.4 ms`，最後の release 10 ms，喪失 10.9 ms → 除外は C だけ（`rm_26_the_ceiling_is_decided_from_the_sequence` で集合を assert），A・B は `no channel` の行として比べる．mutation 行：除外の条件から live を落とす変異が killed．`timed`・`late` を全 command で比べるのは今のまま．
3. **minor：spec の文**．`design/09-mock-radio.md` MR-18 は床の式を書き直さず，`e` と late の判定を RM-21 に委ねる（`at` が床より後なら `e = at`）．Ceiling の定義は RM-16 に一度だけ置き，release の遅れ全般（poll，予約の途中の同期的な装置の呼び出し，channel の無い先頭，apply の直前の喪失）で `S` の前に渡されなかった live な command，とする．UR-26 はそれを引いて Module の仕組みだけを足す．設計メモ 38 は orchestrator が直すので触らない．

## 2. 確かめること

- 新しい試験と直した試験を各 20 回．`cargo test -p ezsdr-radio-uhd`，`-p ezsdr-mock-radio`，`-p ezsdr-radio`，clippy，`rules.py --base HEAD`，`git diff --check`．
- mutation：足した行と，触ったコードを `old` に持つ行（fold の周り，`ceilings()` の周り）を `remote_mutate.py ... sim02:2 <ID> ...` で流して killed．**sim02・sim03 は orchestrator の全行の run が使っている．その run が終わるまで（`ssh -o BatchMode=yes sim02 'docker ps --format {{.Names}}' | grep -c ezsdr-mut-multi` が 0 になるまで）mutation を流さず待つ．**
- セルフレビュー：`impl-brief.md` §4 のコマンドで checklist は `checklist-fix1.md`．P1 以上が無くなるか 3 回で止める．

## 3. 報告

`report.md` の最後に「## 9. 直し 1 回目」を足す（前の節は消さない）：直したこと，試験名，mutation の行と結果，セルフレビューの指摘と扱い．最初の行は `STATUS: done` か `STATUS: blocked`．**起きたことを書く．していないことは書かない．**

---

# 付録：直し 2 回目の指示書（fix2-brief.md）


あなたは実装役です．38a の未 commit の変更（HEAD `7a3b553`）の上で，Sol の再レビュー `sol-fix1-verdict.md` の major 2 件を直す．守ることと確かめ方は `impl-brief.md` の §3・§4・§6 と同じ（セルフレビューの checklist は `checklist-fix2.md`，log は `review-fix2-<n>.log`）．作業 dir の orchestrator のファイル（`*-brief.md`，`checklist*.md`，`plan-decisions.md`，`sol-*`，`.*.orig`）は読むだけ．

## 1. orchestrator の裁定

2 件とも直し 1 回目で足した「`now > S` の pass が渡したものを `held` に戻す」（`given_here`）の穴なので，その仕組みをやめ，「S までに渡した」を command ごとの時刻で決める．

- **定義**（orchestrator が決めた．RM-16 の Ceiling と UR-26 に書く）：command が `S` までに渡された ⇔ その command の装置の呼び出し（channel が複数なら**最後の** channel の `apply`）を始める直前に uhd-control が読んだ装置の時刻 `h` が，停止なら `h ≤ S`，喪失なら `h < S`．呼び出しが `S` をまたいでも，始めた時刻で決める（装置は受理の時刻を返さない．`device.rs:62-72`）．
- **実装**：`give` が各 `apply` の直前に `self.core.now()` を読み，最後のものを `Given` に `handed_at` として持つ．`release` の fold は，streams の lock の下で `ended_at()` が `Some` なら何もせずに戻る（直し 1 回目のとおり）．**`given_here` と，`held` へ戻す分岐は消す．** `settle` は `released_at` の given の command ごとに：`due`（停止 `e ≤ S`，喪失 `e < S`）かつ `handed_at` が `S` までなら issued（`record_given`）；`due` で `S` までに渡されていなければ Ceiling として cancelled（`cancelled_at = S`）；`due` でなければ cancelled．channel 無しの `Given { given: false }` は今のまま（`due` なら `no channel`，でなければ cancelled）．release の条件（いつ・どの channel に渡すか）と装置の呼び出しは変えない．
- **試験**（手動の時計，既存の部品．probe `before_timed_apply` で 2 つ目の command の apply の直前に時計を進めて停止を公開する形）：
  1. A・B（live，どちらも `e = 10 ms`），9.9 ms の release で A を渡し，B の apply の直前に時計を 10.3 ms に進めて停止 `S = 10.2 ms` を公開 → B は渡されるが `finish` の後は B が cancelled（`cancelled_at = 10.2`），A は issued．
  2. A・B（live，`e = 10 ms`），停止 `S = 10.2 ms` の後の 10.6 ms の release で A を渡し，B の apply が拒否（`ApplyFails`）→ `finish` の後に A は cancelled（10.6 > S），B は行なし．
  3. 2 の B が喪失を返す形 → A は cancelled，B は持っていたものとして cancelled（喪失の `S` で）．
  4. 直し 1 回目の 3 本（`ur_26_a_release_after_the_stop_was_published_leaves_the_row_to_the_end` の 2 形，`ur_26_a_command_given_after_the_stop_s_instant_is_cancelled_at_it`）は残して通す．
- **mutation**：`handed_at` を比べない（due だけで issued），比べる向きを `<`／`≤` で入れ替える，`handed_at` を最初の channel の時刻にする（複数 channel の試験があれば），の各行を足して killed．`given_here` に結びついた P599 などは消すか当て直す．
- **spec**：RM-16 の Ceiling に上の定義を一度だけ書き，UR-26 は引いて `handed_at` の仕組みだけを足す．DA-1 の形．

## 2. 確かめること・報告

- `impl-brief.md` §3 のとおり（新しい試験と直した試験を各 20 回，`cargo test -p ezsdr-radio-uhd`・`-p ezsdr-mock-radio`・`-p ezsdr-radio`，clippy，`rules.py --base HEAD`，`git diff --check`，足した行と触ったコードの行を `remote_mutate.py ... sim02:2 <ID> ...`）．
- セルフレビュー：`impl-brief.md` §4 のコマンドで checklist は `checklist-fix2.md`．P1 以上が無くなるか 3 回で止める．
- `report.md` の最後に「## 10. 直し 2 回目」を足す（前の節は消さない）．最初の行は `STATUS: done` か `STATUS: blocked`．**起きたことを書く．していないことは書かない．**
