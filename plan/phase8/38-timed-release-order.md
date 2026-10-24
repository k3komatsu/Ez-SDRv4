# 設計メモ 38：timed command の順番どおりの release を，MockRadio も同じ規則で模擬する（#62）

| 項目 | 内容 |
|---|---|
| 段階 | Phase 8 段階 2（[00-overview](00-overview.md) の P4 (1)，設計 [33](33-channels-and-streams.md) の「timed command の queue」） |
| 決まっていること | Mock は 1 列の queue で順番どおりの実行を模擬する（設計 33，P4，owner 2026-10-16）．ここで決めるのは，その仕方．release window の置き場は D1（P3 (6) を改める）で決めた |
| 機構 | `mech:radio-rule-implemented-twice`（三振を超えている．AGENTS.md §6）．規則は 2 か所に書かず，語彙の 1 か所に置く（設計メモ 36 と同じ形） |
| risky | はい：queue と期限の順序を変え，Mock と実機の境を変える（AGENTS.md §9） |
| 範囲の外 | 設計 33 の D9 の規則（packet の流れていない DDC/DUC には timed command を渡さない）．§5 |

## 1. 今の違い

**UHD Module**（UR-24，UR-14，`crates/ezsdr-radio-uhd/src/provider/control.rs`）：
- timed update（`hardware_timed` の frequency・gain，channel ごと）を `held` に `(e, seq)` の順で持つ（`:560`）．
- uhd-control は Action を 1 つ予約するたびと 1 ms の poll で `release` をまわし（`:285-303`），`e − now` が **release window**（`RELEASE_WINDOW_NS` = 3 ms，`profile.rs:16`，INFERRED）以内のものを装置に渡す（`:568-612`）．渡す時刻は poll と，予約の途中の同期的な装置の呼び出しで揺れる．
- `e` の時点で channel が無く後の segment にも渡せない command の前で release は止まり，`e` を過ぎると `no channel` で記録して進む（`:585-600`）．装置が渡すのを拒否した command は，記録も floor も残さず，投影にだけ残る（`:645-651`）．
- 予約の時点の floor は max（**実際に渡した** command の `e`，`now + device lead`）．頼まれた instant がそれより前なら `LATE_COMMAND`（`:538-543`）．late の判定の後に満杯を見る（`:544-553`）．
- `applied` の `issued` の行は，**渡した時点**で `at: e` として書く（`:655-662`）．Action の `Stop` は command を取り消さない（UR-24，`:349-352`）．`Provider::stop` は持っているものだけを取り消し，渡したものの行は残る（`:307-310`）．装置を失った後は何も渡さない（`:569-575`）．
- 装置の command queue は順番どおり（VERIFIED：`multi_usrp.hpp` の `set_command_time`．spec 18 §2）．

**MockRadio**（MR-18，`crates/ezsdr-mock-radio/src/lib.rs:1776-1792`）：floor は `now + lead` だけで，どの update も自分の instant で効き，その時点で `applied` に書く．満杯の判定が late の判定より先．Provider の停止と喪失では，効いていない update をすべて拒否として記録する（`:1149-1158`）．

**違い**：(a) A が渡された後に A より前の instant の B を予約すると，UHD では B は A の `e` で効き late，Mock では B の `e`．(b) A が渡された後，A の `e` の前に停止するか装置を失うと，UHD は A の行を残し，Mock は拒否として記録する．(c) 満杯と late が重なったときの順．VH-8 の harness は (a)(b) を `release_window_matters`（`control/differential.rs:208-221`）で見分けて `timed` を比べず，`LATE_COMMAND` の中身と `applied` の行はどの系列でも比べていない（`:43-65`）．Mock が緩い（§59 に反する）．

## 2. 推奨

**揺れる「実際に渡したか」に判定を頼らない．** 渡す時刻は UHD Module の都合（poll，同期的な装置の呼び出し，channel 待ち，装置の拒否）で必ず揺れ，Mock はそれを持たない．判定は予約の履歴と instant だけで決まる形にする．

**D1（owner 2026-10-24 決定）：release window は profile の timing の値にする．** `radio.timing.release_window_ns`：x3xx の profile は 3 ms（INFERRED，段階 4 で bench の測定から決め直す），ideal など release を模擬しない profile は 0．MockRadio は profile の値を使い，UHD Module は profile から読んで自分の定数を消す．floor の規則は語彙の関数で，window を引数に取る．UHD 以外の SDR の Module は，その装置の profile に自分の値を書く（command を持たずにすぐ渡すか，順番どおりの queue が無ければ 0 で，今の MockRadio と同じく各 update は自分の instant で効く）．release window は Module と装置の組の値で，MockRadio は UHD Module の crate に依存できない（AGENTS.md §3）ので，中立に受け取れる所は profile だけである．受理済みの P3 (6) を，この値について改める（in-flight window は Module の内部の定数のまま）．schema の再生成と x3xx・ideal の profile と fixture を直す（version は上げない）．

**D2（owner 2026-10-24 決定）：floor と行の書き方を語彙の規則にする．**
1. **floor**：timed update を受け取った時点 `r` での floor は，max（`r + device lead`，それより前に予約された timed update のうち `e ≤ r + window` のものの `e`）．渡したか，channel を待っているか，装置が拒否したかは問わない．頼まれた instant が floor より前なら `LATE_COMMAND { requested, applied: floor }`，`e` は floor．`ezsdr_radio::timeline` の 1 つの関数にし（`layout_instant` の隣），両 Module が呼ぶ．判定の順は late，満杯（UHD の今の順）．
2. **枠**：深さは，予約されて `e` がまだ来ていない timed update の数．装置が拒否したものも `e` まで枠を占める（UHD は今，拒否した時点で空ける．Mock は `e` まで持つ）．floor と同じく予約の履歴だけで決まる．
3. **行**：`applied` の行は，両 Module とも command が instant を迎えた時点で書く（UHD は渡した時点に書くのをやめ，`released_at` が記録に入る所で書く）．`no channel` の行も今どおり instant に書く．装置が渡すのを拒否した command は，今どおり行を書かない．
4. **終わり**：`Provider::stop` と装置の喪失は，起きた時刻 `S` を基準にする（制御のスレッドが終わりの処理をする時刻ではない．UHD Module は喪失の時刻を 1 度だけ読んで持つ）．`S` の前に `e` を迎えた command は，装置を呼ばずに今の結果で確定する：渡したものは `issued`，channel が無かったものは `no channel`，装置が拒否したものは行なし．`e` が `S` より後のものは，持っているものも渡したものも同じに記録する（MockRadio の今の記録：停止の理由で拒否として記録）．同じ時刻の順は MockRadio の今の順のまま：停止では `e ≤ S` のものが先に効き，喪失では喪失が先（`e < S` のものだけが効く）．`S` より後のものは渡してあれば装置で効くかもしれないが，Run の後のことで，記録は「Run の中で効いたことが観測されていない」ことを言い，装置から取り消したとは言わない（Ceiling）．Action の `Stop` は今どおり何も取り消さない．
   *Ceiling*：`S` の前に `e` を迎えた live な command を，uhd-control の release の処理が遅れて（poll，予約の途中の同期的な装置の呼び出し，channel の無い先頭）まだ渡していないうちに停止か喪失が起きると，UHD Module はそれを渡していないので `e > S` のものと同じに記録し，MockRadio は `e` で効いたと記録する．release の遅れは UHD Module の実装の都合で，Mock に持ち込まない．手動の時計の harness では，release の順は系列から再現でき（`control/differential.rs:303-364`），この場合は channel の無い先頭の後ろで 1 poll 以内にだけ起こる．harness は，後ろの command が終わりの前に効くはずのもの（停止なら `e ≤ S`，喪失なら `e < S`）で終わりまで渡されない場合に限り，**その command の行の比較だけ**を外し，`e`，`LATE_COMMAND` の中身，ほかの command の比較は残す．

UHD Module の release（いつ，どの channel に渡すか，channel 待ちで止まる，装置の拒否）は今のまま変えない．floor は今より保守的になる（渡していない A の後ろにも B を置く）だけで，B は A の後に渡るので装置の queue の順は保たれる．

これで harness の `release_window_matters` を消し，`timed` を全系列で比べ，`LATE_COMMAND` の中身と正規化した `applied` の行も比べる（行は D2 (4) の Ceiling の command だけを除く）．

## 3. 採らない案

- **floor の関数だけを共有し，「渡した」を今の状態から計算する**（1 版目）：渡した後の cold change・停止・喪失と poll のずれで揃わない（Sol 1 回目）．
- **timed command の台帳を共有し，境目（`e − window`）ちょうどに渡す**（2 版目）：uhd-control は予約の途中で同期的に装置を呼ぶので境目を守れず，channel 待ちと装置の拒否の状態も要り，型が大きくなる（Sol 2 回目）．
- **release window を `ezsdr_radio` の値にする**（前の推奨）：語彙はどの radio Provider も使うので，UHD Module の実装の都合がほかの SDR に及ぶ（owner）．
- **`ezsdr_radio::x3xx`（X3xx の description）に置く**：MockRadio が profile の名前から族の定数を引く暗黙の対応ができ，profile に書くより見えにくい．
- **MockRadio に UHD と同じ定数と規則を書く**：規則と値が 2 か所になる（機構の 10 件目）．
- **Ceiling として記録し，`timed` を比べないままにする**：P4 で採らないと決めた．

## 4. テストで固める系列（指示書で表にする）

device lead を 2 ms，window を 3 ms とする．各系列は MockRadio と UHD Module（FakeDevice，手動の時計）の両方で同じ期待値になる．時刻は受け取り（`r`）を固定して書く．

1. A（`e` = 10 ms）を 0 ms で予約．B（`at` = 9.5 ms）を 7 ms で予約 → B は 10 ms，`LATE_COMMAND { requested: 9.5, applied: 10 }`．
2. 同じ A と B，B を 7 ms − 1 tick で予約 → B は 9.5 ms，late なし．
3. B（`at` = 11 ms）を 7 ms で予約 → 11 ms，late なし．
4. A の channel が A の `e` に無く後ろの segment にも無い（release は A で止まっている）．B（`at` = 9.5 ms，別の channel で live）を 7 ms で予約 → B は 10 ms で late（A を渡していなくても数える）．A は `e` で `no channel`，B はその後に渡り，予約上の `e` は 10 ms（装置で実際に効く時刻は，渡した時刻が 10 ms を過ぎていればその到着時：`set_command_time` の「late なら到着時」．行の `at` は予約上の `e`）．B にも channel が無ければ B も `no channel`．
5. （UHD Module だけ，FakeDevice の fault）装置が A を渡すのを拒否する．その後 B（`at` = 9.5 ms）を 7.2 ms で予約 → B は 10 ms で late（拒否された A も数える），A の行は無く，A は 10 ms まで枠を占める．MockRadio には release の拒否が無いので，比べる系列には入れない．
5b. 深さ 16 で，A と遠い 15 件を予約，A が拒否された後に B → 満杯（A が `e` まで枠を占める）．UHD だけ．
6. A が渡された後，7.2 ms に A の channel を cold change で外し，7.3 ms に B（`at` = 9.5 ms）を予約 → B は 10 ms で late．
7. A と B（`at` = 12 ms）を予約し，8 ms に Action の `Stop` → A も B も取り消されず，それぞれの instant に行．
8. 同じ A と B，8 ms に `Provider::stop` → A（渡した）も B（持っている）も同じ記録（拒否），行は無い．8 ms に装置を失っても同じ．
9. A が 10 ms に効いた後に停止 → A の行は残る．
9b. A を 7 ms に渡し，最後の release が 9.6 ms，喪失が 10.2 ms，制御のスレッドが次に動くのが 10.6 ms → A は `S` = 10.2 ms より前に `e` を迎えたので行がある（両 Module）．停止が 9.9 ms で，終わりの処理が 10.6 ms なら，A は `S` より後なので終わりの記録．
9c. 渡した A の `e` = `S` ちょうど：停止なら A の行がある（停止の前に効く），喪失なら A は終わりの記録（喪失が先）．
9d. 系列 4 の A（channel 無し）と B（live），両方 `e` = 10 ms，最後の release が 9.6 ms，喪失が 10.2 ms，次の poll が 10.6 ms → UHD Module では B は渡されず終わりの記録．Ceiling の場合で，harness は外す（UHD Module だけのテストで固める）．
10. 同じ `e` の 2 つの command：行の順は予約の順．
11. 満杯で，頼まれた instant が floor より前 → `LATE_COMMAND` の後に `COMMAND_QUEUE_FULL`，予約されない．効いた command は instant で数から外れる．
12. differential harness：`release_window_matters` を消し，1 000 系列で `timed`，`LATE_COMMAND` の中身，正規化した `applied` の行（D2 (4) の Ceiling の command を除く）が一致する．

装置の command queue が順番どおりであることは VERIFIED．release window の値と，stream command（UR-17）が同じ queue を使うかは INFERRED．

## 4b. 変わる spec とテスト

- spec 18：UR-24（行は instant に書く，深さは予約から `e` まで，floor の規則は語彙の関数），UR-26（`Provider::stop` は `S` の前に効いたものを確定し，後のものは持っているものも渡したものも同じに記録），UR-30 の記録の説明．release window は profile の値（§3 の Module の定数から外す）．
- spec 09：MR-18（floor の規則は語彙の関数，判定の順は late，満杯），MR-25（終わりの `S`）．spec 07（語彙）に floor の関数と release window．
- UHD Module のテスト：release の直後に `issued` の行を見るもの（`control.rs` の `ur_24_a_release_without_a_live_channel_reaches_the_pending_segment` `:1686-1695`，`:1751`，`:1809`，`:3809` ほか）は，装置の呼び出しの順と read-back の確かめを残し，行の確かめだけを `e` の後へ移す．
- harness：`release_window_matters` を消し，`Record` に `LATE_COMMAND` の中身と正規化した `applied` の行を足す．
- UR-30 の plan は stream の timeline から作られ，timed の行を入力にしない（`core.rs:715-731`）ので変わらない．

## 5. D9 との関係

D9（packet の流れていない DDC/DUC には timed command を渡さず，次の開始の前に untimed に適用する．release 後の cut の扱い）は，UHD Module の release の条件と，渡さずに設定に持つ処理を変える．この推奨は release を変えず，floor と行と終わりの記録だけを決めるので，D9 の変更を妨げない（floor は渡したかを問わないので，D9 が渡さなくした command も同じに数える）．ただし UHD Module の中の「渡した」の記録（`released`，`released_at`）は消さない：D9 の release 後の cut の判定に要る．D9 は別の段階で決める．D9 が終わるまで，Phase 8 の parity（DDC/DUC の queue の安全）は閉じない．

## 6. 決定（owner 2026-10-24，すべて推奨どおり）

1. D1（D1 は最初の推奨の「`ezsdr_radio` の値」を owner が退け，改めた推奨の「profile の timing の値」で決めた）．
2. D2．
3. この段階を risky とする（指示書の系列の表 → 実装役のテスト計画 → Sol の計画レビュー → 実装）．

## 8. 実装

- 38a：次．

## 7. レビュー

| 回 | 結果 |
|---|---|
| Sol 1 回目（1 版目） | FAIL，major 4・minor 2：floor だけでは揃わない（poll のずれ，渡した履歴），D9 が未決，境目の系列が lead と混ざる，harness の比較が粗い，D1 が P3 (6) を変える．2 版目で台帳にした |
| Sol 2 回目（2 版目） | FAIL，major 4・minor 1：境目ちょうどの release を守れない，Action の `Stop` は取り消さない，channel 待ちの次の起床，装置の拒否の状態，系列 7 の期待値．3 版目で「実際に渡したか」に頼らない規則に変えた |
| Sol 3 回目（3 版目） | FAIL，major 2・minor 2：floor の方針は妥当．拒否の後の枠，停止・喪失の時刻と instant の間，系列 4 の channel と実際に効く時刻，変わる spec とテスト．枠を予約から `e` まで数え，終わりを `S` 基準にし，拒否の系列を UHD だけに分け，§4b を足した |
| Sol 4 回目 | FAIL，major 2：`S` の前に `e` を迎えても渡していない command（channel 待ちの後ろ，poll の間の喪失），`e = S` の喪失の順．終わりの確定を結果ごとに分け，同じ時刻の順を MockRadio の今の順に合わせ，poll の端を Ceiling にした |
| Sol 5 回目 | FAIL，major 1・minor 1：前回の 2 件は解消．Ceiling の「1 ms 以内」は実運用で保証されない，harness の除外の範囲が曖昧．Ceiling を実運用の形で定義し，手動の時計の harness で起こる条件と外す範囲（その command の行だけ）を分けた（文の修正なので 6 回目は頼まず owner の判断へ） |
