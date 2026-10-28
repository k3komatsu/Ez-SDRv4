STATUS: plan v3

# 段階 43a 試験計画：拒否で後ろの origin を早めない（RM-25）と受信の start lead ≥ release window（RM-6）

作成：実装役（Sonnet）．HEAD `afbb125` を `git show` で読んだだけで，cargo は流していない．数値は手計算で，各所に「手計算」と書いた．実装後に実行して食い違えば，実行を正とせず，まず計算を疑って orchestrator に報告する．確かさの印：VERIFIED（code を読んだ），INFERRED（読みから推した）．

## 0. 読んで分かったこと（メモ 43 への補正）

メモ 43 と食い違う，または足りない点．計画はこれを前提にしている．

| # | 内容 | 印 |
|---|---|---|
| A1 | 受信の拒否は `State::replay` の `Kind::Cold` で `begin` した segment を捨てるだけ（`timeline.rs:983-993`，`if !item.refused \|\| !receive { self.running = ... }`）．`previous` は拒否の手前の cut のまま．これが F6 の原因．`Kind::Start` の拒否も同じ（`:967-972`，`self.halted = item.refused` のあと `!self.halted` のときだけ `running` を置く） | VERIFIED |
| A2 | **メモの「期待が変わる既存の試験」の列挙は不足**．`tests/timeline.rs` の `rm_26_the_timeline_cases` に，メモが挙げた `:269-271` のほかに `a refused receive change halts the stream until the next change`（`:262-264`）も変わる．手計算：C 拒否 e=1000 ms，origin 1050 ms；次の change e=2000 ms を仮想 segment が切ると cut = 2000 ms（1 MS/s の lattice に乗る）→ 次の origin = 2000+50 = 2050 ms．期待 `(2_000*MS, None)` → `(2_050*MS, None)`．§5 に完全な表 | 手計算 |
| A3 | `start_rx resumes a stream a refused change halted` と `a refused start_rx …` の 3 case は値が変わらない（call = 0 の stream なので cut + lead = ready + lead = 1550 / 3050）．ただし**生成試験（call 2000 sample・allowance 3 ms）では `ready >= cut + call + allowance` が効いて変わる**．同じ値のまま通るから期待を「変えない」と決め込まず，実行して確かめる | 手計算 |
| A4 | **送信にも拒否で後ろの origin を早める動きがある**（owner の確かめの依頼への答え）．`timeline.rs:938-941`（`refusal` が立つ間に `item.ready >= refused` の item が来ると `end_at_origin`）と `:977-979`（refused の tx は `refusal = Some(item.e)`）．refused な tx の clock は origin で終わり，`previous` が**その origin**になる．拒否が無ければ次の change N がその clock を N.e で切るので `previous` は N.e 側．N.ready ≥ C.e かつ N.e が C の origin より後なら，拒否で N の origin が最大 lead だけ早まる．手計算の例：tx，C = Cold(2 MS/s) e=1000 ready=500 refused，origin 1050 ms；N = Cold(1 MS/s) e=1200 ready=1005．拒否なしの N の origin = 1200 + 50 = 1250 ms．拒否ありの HEAD = max(1200, 1050+50, 1005+50) = **1200 ms**．早まる量は 50 ms = lead．N.ready < C.e の N は元から C を N.e で切るので動かない（`tests/timeline.rs:318-320` の case）．HEAD でこの値が出ることを T-X1 が先に固定する | 手計算（動きの存在は code の読みで VERIFIED，値は実行で確認） |
| A5 | 仮想 segment の cut は `item.delivered` を読んではいけない．`State::cut`（`:1003-1010`）は `(true, Some(delivered)) => delivered.max(0)` と `.max(delivered.unwrap_or(MIN))` で，item の `delivered` は**実際に走っていた segment**の配信量を表す．halt 中の仮想 segment は何も配信していない．そのまま読むと cut が過大になり，release と次の origin が過大になる．`delivered` を埋める所は 2 つ：Mock `lib.rs:1241-1249`（`End { abort }` の rx，`stream.running` の `next`）と UHD `core.rs:504-509`（`Streams::book`：rx の `End` で，`line.made(..)` が返す未 cut の実 segment がある場合だけ `first_undelivered` を入れる．仮想 segment は plan に無いので `made` が `None` を返し，`delivered` は `None` のまま）．つまり今の Provider は仮想 segment に古い index を渡さないが，`timeline` の側でも読まないことを自分の試験で固定する（Provider の暗黙の前提に頼らない） | VERIFIED（2 か所の code）．Provider の暗黙の前提は INFERRED |
| A6 | `Planned.refused`（UHD `core.rs:582-592`）は tx の clock だけが立てる（rx の拒否は plan に出ない）．規則 1 は受信の拒否を plan に出さないままにするので UHD の `planned()` は変わらない．UHD Module の本体は変えない | VERIFIED |
| A7 | 規則 2 の置き場：`UhdRadio::with_measurement_timing`（`provider/mod.rs:367-373`）は `Description::new` を通さず `description.timing` を差し替え，**capability の tree，`min_command_lead`，`envelope` section を公開し直す**（`description_surfaces`）．測定の道も Provider の公開 description なので，**同じ検査を強制する**（orchestrator の裁定：例外は作らない）．測定の候補（`tests/common/measurement.rs`）は `candidate_timing`（`:2253-2268`）が `default_timing()` に 1 つの field だけ入れ，rx の掃引は `plan.rx_start_leads_ns`（`:672,703,734`：既定 `[1,2,5,10,25,50,100] ms`，quick `[1,2] ms`）を回す（`:2547`）．lead < W の候補は構築できなくなるので，候補の表は W 以上に限る（T8 の「測定の道」） | VERIFIED（code） |
| A8 | 今の公開値：x3xx は W = 3 ms，rx lead = 50 ms（`x3xx.rs:393-404`）．ideal は W = 0，rx lead = 0（`channels.rs:380-389`）：等号で通る．候補の固定値 `release_window_ns: 13_000_003`，`rx_start_lead_ns: 19_000_005`（`provider/mod.rs:873-875`，`tests/fake.rs:534-536`）も通る | VERIFIED |
| A9 | `UhdRadio` の試験が `description.timing.release_window_ns = 10_000_000`（`control.rs:5810`）や `rx_start_lead_ns = lead_ns`（`:5851`，`rx.rs:1102`）を直接代入している．これらは `Description::new` の後なので規則 2 の検査を通らない．検査が `new` にあるかぎり壊れない．ただし規則 2 を破る組（W 10 ms と lead 50 ms は通る）が `rx.rs:1102` の 19 ms と W 3 ms は通る，など，**直接代入の組が規則 2 を破っていないか**を T8 の補助 grep で調べる | VERIFIED（箇所），INFERRED（破るか） |
| A10 | `Line::prune` は `State::replay` で items を `start` に畳む．仮想 segment の印は `State` の中にあるので，prune で自然に畳まれる．ただし印を `Line` の別の field に置くと畳まれない．印は `State` に置く（m5 が検出する） | VERIFIED |
| A11 | `Lines::released` は `plan` と `omitted` だけを読む（`timeline.rs:309-326`）．仮想 segment が `omitted` に入るのは**切られたときだけ**．切られない仮想 segment は `finish` が捨てる．move の item は同じ line の item なので，その時刻より前に始まった仮想 segment は必ず move が切る（`over` の後を除く）．この「必ず切られる」ことを T1 の不変条件に入れる（切られず捨てられた仮想 segment が release に効かない穴を塞ぐ） | VERIFIED（`replay` の読み），穴の有無は INFERRED |

## 1. 規則と実装の姿（試験の前提．実装役が実装で変えてよいのは「どう書くか」だけ）

規則 1（RM-25，受信）：拒否された `Cold` と `Start` は，始めるはずだった segment を「仮想」として `running` に置く（`State` に印 1 つ）．`cut` はそれを `out` でなく `omitted` に入れる（cut が 0 でも，cut > 0 でも）．`finish` は切られなかった仮想 segment を捨てる．halt した stream の `Start` は先に仮想 segment を切ってから始める．仮想 segment の cut は `item.delivered` を読まない（A5）．

規則 1 の送信への拡張（A4，owner の補足）：refused な tx の clock の**記録**は今のまま（origin で終わる，または拒否の前に booked な change が切った cut）．**次の segment の `previous`** だけを，拒否が無かったとして N が C を切る cut にそろえる．実装の形は実装役が決める．守る観測は T-X1〜X3 の 3 つ．

規則 2（RM-6）：`rx_start_lead_ns >= release_window_ns`．`Description::new` が検め，破れば `assert!`（programming error）．検査は 1 つの関数にして，試験が同じ関数に値を与える．

## 2. 共通の道具と観測

- timeline の試験は `tests/timeline.rs` の `item`・`refused`・`plan`・`segments`・`Lines`・`by`・`released`・`planned_at` を使う．拒否は 2 通り（booked refused：`refused(item(..))`，後の `Line::refuse`/`Lines::refuse`）で書き，**両方で同じ plan** を assert する（UHD は両方を使う：`control.rs:1121` は booked refused，`:1750` と `core.rs` の試験は後から refuse）．
- Mock の観測は `harness` の `segments` section，`clocks_of`，`origins(&m)`，`rejected()`，`applied()`，`coherence`，`bursts` section．UHD は `core.device.calls()`，`plan_of`，`lock(&core.streams)`，`Streams::planned`（`core.rs:582-596` の `clock`）と clock の登録（`core.rs:706-717`），manual clock．
- **観測の時刻は，求める予定から導いた値を test に書く．実行結果を見てから調整しない**（Sol M6）．手計算の値と実行が食い違ったら，値を直さず orchestrator に報告する．

### 2.1 oracle O-eq（実装に依らない）

受信 stream で item k を refused にした plan と，k を refused にしない plan を比べる．**入力は item の同一性がそろうものに限る**：両方で同じ `seq`・`e`・`ready`・`kind`・`delivered` を持ち，違いは k の `refused` だけ．k の segment を除いて，k より後の item が始めた segment の origin と cut が一致する．除外：(a) k の後，次の `Cold` までに halt 中の `Start` が来る sequence（拒否なしの Start は no-op，拒否ありは再開．`:965-973`），(b) k が `Start` でその前に stream が走っていた sequence，(c) **`delivered` が `Some` の item を含む sequence**（実 segment と仮想 segment で cut が正当に違う．例：delivered の床つき Stop が実 C を後ろで切る）．(a)(b)(c) の除外は生成器の filter とし，(a)(b) の値は T4 の固定 case，(c) は T1-d の別の試験で固定する．tx の version は T-X2．HEAD では A2・A4 の数値例で落ちる．

## 3. 系列と試験

### T1　生成した系列

置き場：`tests/timeline.rs`．生成器は `rm_26_generated_sequences_keep_the_timeline_invariants` と `rm_34_generated_moves_keep_both_lines_invariants` の `generator::sequence(seed, &FRACTIONAL_RATES)` と，後者の `Lines`（FROM rx[0,1] と TO rx[2]）の組み立てをそのまま使う（生成器を増やさない）．1 000 seed（`Lines` の版は 500）．stream は call 2000・allowance 3 ms（今の生成試験と同じ）に，lead が短い組を 1 つ足す（拒否が lead 内に重なる形）．

- **T1-a　`Line` の O-eq**　`rm_25_a_refusal_does_not_move_a_later_origin`：seq%5==4 でなく **1 つだけ** refused（位置を seed で回す）．`delivered` は全 item `None`．assert：(1) plan のどの segment の `by` も refused の seq でない；(2) O-eq；(3) 同じ sequence を booked refused と `Line::refuse` で作って `plan`・`released` の答え・`halted` が等しい（`rm_25_a_refused_receive_segment_is_not_in_the_plan` 内）．落とす間違い：m1，m2，m3（call>0 で値が違う），m10．
- **T1-b　`Lines` の拒否前後の move release（M4）**　`rm_34_a_refusal_does_not_change_a_move_release`：`rm_34_generated_moves_…` の組み立てで，move の前の半分に refused を 1 つ入れ，**`Lines::refuse` で後から拒否した `Lines`** と **拒否していない `Lines`** を作る．assert：(1) 全 move の `released(from, channel, seq)` が両方で等しい（O-eq の `Lines` 版）；(2) FROM・TO の**すべての stream** の，refused の item 以外が始めた segment の origin・cut が等しい；(3) move の後の item を FROM と TO に足した後も (1)(2) が成り立つ．**後の `Stop` が source の release を減らす case** を固定 case として別に持つ：FROM に Cold C（refused）e=1000 → move e=1000 → **後の Stop が move より前の instant（`tests/timeline.rs:707-724,1037-1060` の形）で source を切る**：release と destination の origin が拒否の前後で等しく，Stop で減った後の値も等しい．**A→B→C の連鎖**：A の change の拒否が B の release と C の origin を動かさない（`chained_move_prune_order` の形，prune 前後も）．
  落とす間違い：m1（release が 1000 に戻る），m4，m2．
- **T1-c　O2：覆いの境目（M4）**　`rm_25_a_booking_adds_no_coverage_before_its_start_lead`：`Lines` を組み，item を 1 つずつ足す（FROM・TO の両方に，refused を混ぜた系列でも，move の後の booking でも）．足す前後の**すべての stream の plan**から境目 `B` = 全 segment の origin と cut の instant の集合（足す前後の**両方**の plan の和集合）を取り，`t ∈ {x−1, x, x+1 : x ∈ B} ∪ {b−1, b, b+lead−1}` の各 t（**b + lead より前の区間全体．b より前の t も含む**）について，**足した後の `covering` が `Some` で足す前が `None` になる t が無い**ことを assert（b = その item の ready，lead = その stream の start lead）．**O2 が成り立たない seed が出たら，実装を止めて報告する**（設計メモ 43 の義務 O2 が偽の証拠）．
  落とす間違い：仮想 segment が plan に出る（m2），lead 内に覆いを足す実装．
- **T1-d　古い `delivered`（M5）**　`rm_25_a_virtual_segment_ignores_a_stale_delivered_index` と `rm_16_a_real_segment_keeps_its_delivered_floor`：固定 case．rx，lead 50 ms，call 0，C refused e=1000 ready=500（origin 1050，1 → 2 MS/s）．
  - **仮想・abort**：`End { abort: true }` e=1500 seq=3 を `delivered: None` と `delivered: Some(700_000)` の 2 通りで booking．RX の `End` は `over` を立て，その後の `Start` は無視される（`timeline.rs:934-936,949-951`）ので**次の origin は無く，比べない**．観測は 2 つ：(1) **`End` が再開を止める**：End の後の `Start` e=1600 を booking しても plan に segment が増えない（`planned_at` が End の前後で同じ，どちらの `delivered` でも）；(2) **仮想の end を `Lines::released` で見る**：FROM に C refused，End seq=3 e=1500，その後に **同じ e=1500 の move を seq=5 で `book_move`**（`sorted` は (e, seq) 順なので End が先に仮想 segment を切り，move の item は `over` で効かない）．`released(&lines, FROM, channel, 5)` = 仮想 segment の end = **1500 ms**（手計算：origin 1050，cut sample ceil(450 ms × 2 MS/s) = 900 000 → 1500 ms）を **`delivered` が `None` でも `Some(700_000)` でも**．`delivered` を読む実装（m8）は 700 000 sample 目の instant = 1400 ms を返して落ちる．`book_move` が `NotInSource` 等で move を拒むなら，その時点で止めて報告する（INFERRED：拒否された config の channel を source が「持つ」と `config_at` が読むはず）．
  - **仮想・Stop**：C refused，`Stop` e=1500 seq=3 を `delivered: None` と `delivered: Some(2_000_000)`，その後 `Start` e=1600 seq=4 ready=1600．assert：後の Start の origin = **1650 ms**（Stop が仮想 segment を 1500 で切り，previous = 1500，max(1600, 1500+50, 1600+50)）を両方で；`plan` は両方で `[(0,Some(1000))]` + Start の segment；同じ e=1500 の move（seq=5）の `released` = **1500 ms** を両方で．`delivered` を読む実装は previous が 2050 ms になり origin 2100 ms で落ちる．
  - 実：拒否しない C で同じ `End { abort: true }` に `delivered: Some(k)`：cut が k（RM-16），`Stop` に `delivered: Some(k)`：cut が `max(sample_at_or_after(e), k)`．既存の `rm_26_the_timeline_cases` の `a Stop's cut is floored at what was delivered`・`an abort cuts at the first sample not yet delivered`（`:290-296`）を残し，C が走っている状態の版を足す．
- **T1-e　prune**　`rm_25_a_pruned_line_with_a_refused_segment_plans_as_one_that_keeps_every_item`：拒否の後，仮想 segment が origin の前（pending）と後（running）にある 2 つの所で `prune(before)` し，その後に Stop，Start，Cold，move を記帳する．prune した `Lines` としない `Lines` で `plan`・`released`・`halted` が等しい．落とす間違い：m5．
- **T1-f**　既存の `check()`（`:1450-1478`）は変えない．rx の segment に cut = 0 が出ないことと「次の origin ≥ 前の kept cut + lead + call + allowance」は，仮想 segment を除いた plan で成り立つ．

### 仮想 segment の振る舞い（D5・D6 の決め．この計画の期待）

- `Stop`（on のとき）：仮想 segment を Stop の instant で切る（`delivered` は読まない）．`omitted` に end = cut の instant で入る．on = false．
- `End`（rx）：同じに切って `over`．以後の item は効かない．
- 切られない仮想 segment（以後に cut を起こす item が無い）：`finish` が捨てる．`omitted` に入らず，release に効かない．
- 後の move：move の item は source の `Cold` なので，move の時刻より前に始まった仮想 segment を必ず切る．`released` はそれを `omitted`（`began_before`）で数える．move が動かす channel が仮想 segment の config に無ければ数えない（`released` の `contains(channel)`）．`End` の後に来る move の item は `over` で効かない（今のまま）．
- 仮想 segment が move より後に始まる場合は数えない（`began_before` が false）．

### T2　S1 → C1（拒否）→ C2（重なり）（`tests/timeline.rs`）

`rx` は `stream(Direction::Rx, 1)`（lead 50 ms，call 0）．正確な値：
- C1 refused e=1000 ready=500（1 → 2 MS/s），C2 e=2000 ready=1500（1 MS/s）：S2 は plan に無い．S3 origin = **2050 ms**（HEAD 2000）．`halted(t)`：t < 2000 で真，t ≥ 2000 で偽．
- C1 refused e=1000，C2 e=1010 ready=1005：S3 origin = **1100 ms**（HEAD 1055）．
- C1 refused，C2 refused e=2000 ready=1500，C3 e=3000 ready=2500：1 つ目の仮想 origin 1050 → C2 が 2000 で切る（cut 2000）→ 2 つ目の仮想 origin = 2050 → C3 が 3000 で切る（cut 3000）→ C3 origin = **3050**．
- 同じ e の C1（拒否）→ C2（seq 順）：C2 は C1 の仮想 segment を同じ instant で切る（cut 0，instant = C1 の origin 1050）→ C2 origin = 1050 + 50 = **1100**．
`assert_eq!` の正確な値．落とす間違い：m1．

### T3　欠陥の系列と等号（UHD `control.rs` の `mod tests`，manual clock）

道具は HEAD の `ur_24_a_frequency_for_a_far_planned_segment_is_deferred_to_its_configure`，`ur_24_a_stopped_receive_channel_is_given_no_timed_frequency`，`ur_24_a_frequency_inside_a_running_segment_is_given_timed` の形を使う（D9a2 の worktree は見ない）．

- **T3-a（消失の回帰）**　`ur_24_a_refusal_booked_after_a_release_leaves_the_pending_segments_value_to_its_configure`：rx s0 が S1 を走らせ，C1（rate 変更）が S2，C2 が S3 を計画（S3 の origin o3．C1 が拒否されなければ S2 が o2，S3 は S2 の cut + lead）．V を **e = o3 − 1 tick** で `UpdateParameter`（e を覆う segment が無い）．`e − W` で release：Deferred（装置への `apply` call **0**）．owner が C1 の拒否を記帳（`streams.refuse`）．S3 の configure．assert：S3 の origin は **o3 のまま**（`plan_of`）；configure の untimed の書き込みに V がある（`device.calls()` の `apply rx <chan>` のうち周波数が V のもの **1 件，timed でない**）；timed の `apply` は 0；`dsp_pending == 0`；`freq_at(o3) == V`．**HEAD では落ちる**（o3' < e で configure が `effective ≤ origin` の V を運ばない）．HEAD で落ちることを実装前に実行して記録する．
- **T3-b（等号 e = o3．期待は別，M2）**　`ur_24_a_frequency_due_at_a_segment_origin_is_given_timed_and_the_configure_carries_it`：同じ系列で e = **o3**．S3 は `e` を覆う（`origin ≤ e`）ので `reach` は `Timed`（`control.rs:743-756` は configure 済みかを見る前に `covering` を返す）．期待は T3-a と違う：release で **timed の `apply … Some(e)` が 1 件**；S3 の configure の書き込みにも V（`effective ≤ origin`）；**消費は別に観測する**：fake device の DDC の消費モデルで，S3 の最初の sample が通った後 `dsp_pending == 0`（`ur_24_a_frequency_inside_a_running_segment_is_given_timed` と同じ観測）．`freq_at(o3) == V`．Deferred でない（T3-a と同じ assert を流用しない）．
- **T3-c（規則 2 の等号の証明）**　`ur_24_a_deferred_value_survives_a_start_booked_at_the_release_with_lead_equal_to_the_window`：停止中の channel，e = 100 ms，W = lead = 3 ms（x3xx の lead 50 ms は等号でないので，この試験は `with_measurement_timing` で `rx_start_lead_ns = 3_000_000`（= W）を与えて作る．規則 2 の検査関数を通り，等号は通る）．V を e = 100 ms で update；release 97 ms（b = e − W）で Deferred（`apply` 0）；b = 97 ms に `start_rx`：origin = 97 + 3 = **100 ms = e**；configure：V が `effective ≤ origin` で載る．assert：configure の untimed の書き込みに V が 1 件；`freq_at(100 ms) == V`；timed の `apply` 0．HEAD でも通る（等号の特性．規則 2 が `lead = W` で十分であることの固定）．**lead = W − 1 tick の同じ系列は規則 2 が構築を拒むので，ここには置かない**（検査関数の単体で T8 が拒否を固定）．落とす間違い：m6（規則 2 の向きを逆に）で `lead < W` の description ができ，この試験の前提 `Description` が作れる／別の試験が落ちる．

### T4　拒否の後の `start_rx`（M-minor：非 0 の call の算術を完成）

- timeline，call 0（`stream(Rx,1)`）：C refused e=1000 ready=500，`Start` e=1500 ready=1500：origin = **1550**（HEAD と同じ）．仮想 origin 1050 より前の Start e=1010：cut 0，instant 1050，origin = max(1010, 1100, ready+50) = **1100**．`Stop`(1000) → refused `Start`(2000) → `Start`(2500)：仮想 origin 2050，cut 2500，origin = **2550**．
- **call 2000・allowance 3 ms・初め 1 MS/s，拒否された 2 MS/s（Sol の例）**：S1（1 MS/s）→ C refused e=1000 ready=500 → `Start` e=1500 ready=1500．仮想 segment の origin：previous = S1 の cut 1000；ready = max(500, 1000 + 2000 sample × 1 µs + 3 ms = 1005) = 1005；earliest = max(1000, 1050, 1005 + 50 = 1055) = 1055（2 MS/s の lattice 500 ns に乗る）．Start が 1500 で切る：cut sample = ceil(445 ms × 2 MS/s) = 890 000 → instant 1500；previous = (1500, 2 MS/s)；ready = max(1500, 1500 + 2000 × 500 ns + 3 ms = 1504) = 1504；earliest = max(1500, 1550, 1554) = **1554 ms**．HEAD = 1550（previous が S1 の cut 1000 のため）．これを `assert_eq!`．
- from-none の enable を uhd-control が拒否した後の次の enable（`control.rs`，`ur_25_a_refused_receive_enable_is_booked_and_the_next_enable_starts` の拡張）：次の enable の origin が拒否が無かった場合の予定と等しい；間の Deferred の値は次の configure が運ぶ（T3-a の観測）．
落とす間違い：m3（halt した Start が仮想 segment を切らない：origin が 1550 になる）．

### T5　move（`Lines`，`tests/timeline.rs`）と，Mock・UHD の実際の move の case（m4 を殺す）

timeline：
- `rm_34_a_refused_source_change_moves_the_destination` の反転．名前 `rm_34_a_refused_source_change_does_not_move_the_destination`．拒否の前後で `released(&lines, FROM, 1, 1)` は **1050 のまま**，`by(&lines, TO, 1).origin` は **1100 のまま**．
- `pruned_move_order`（2 case）：`release_after < release_before` と `destination_after < destination_before` を `==` に．`== unpruned` の比較は残す．
- move 自体の拒否（`refuse_move`）：両 line が halt，後の move は仮想 segment の終わりを待つ．
- 仮想 segment が move より後（`began_before` false）：release に効かない．
- 仮想 segment の config に moved channel が無い：release に効かない．
実 Provider の move（**m4 を殺す．T6/T3/T4 は move を使わないので別に持つ**）：
- Mock：`mr_18_a_layout_move_after_a_refused_source_change_waits_the_refused_segments_end`：2 stream の layout instance で source の cold change を `cold_change_refused_at` で拒否し，同じ instant に move．destination の segment の origin（`segments` section）が，拒否が無かった場合の値と等しい．
- UHD：`ur_25_a_move_after_a_refused_source_change_configures_the_destination_at_the_unchanged_origin`（`core.rs:1821` の move の試験の形．`streams.refuse` を source の change に）：`plan_of` の destination origin が拒否の前後で等しく，その origin で configure が装置に届く．
落とす間違い：m4（release が仮想 `omitted` を読まない → destination が早まる），m1，m2．

### T6　Mock（`mock_layout.rs`）：旧い早まりの区間（M3），Session と Spec Run

`refusal_origins()` と `early` を使わない．**HEAD で実行して記録した定数を literal で書く**：実装前の HEAD で `refusal_run(false,true,None)`（拒否なし：o2，o3）と `refusal_run(true,true,None)`（拒否ありの旧い o3' = `early`）を求め，T0 からの差としてコードに literal で埋める．**Sol の HEAD の算術では T0 からの差が o2 = 65 ms，o3 = 124 ms，o3′ = 70 ms**（旧い早まりの区間は (70, 124) ms）．**production を変える前に HEAD で実行してこの 3 つを記録し，literal で固定する**（実行が違えば固定せず報告）．固定した e は，たとえば T0 + 75 ms（= o3′ + 5 ms）と T0 + 80 ms（= o3′ + 10 ms）で，どちらも o3 = T0 + 124 ms より前．
- 受け取り r・実効 e を，**区間の中に固定**して選ぶ：`e = o3' + 5 ms`（旧い `early + 5 ms`），`r = e − 10 ms`（受け取りは o3' より前，S1 は C1 で切れていて何も走らない）と，`r = o3' + 5 ms`・`e = o3' + 10 ms`（受け取りが旧い S3' の始まりの後）の 2 つ（既存 2 本の値と同じ）．**固定した値であって，修正後の `early` から導かない**．
- assert（修正後）：(1) 拒否ありの origins は `[T0, o3]`，**`o3` は拒否なしの S3 の origin と等しい**（`plain[2]`）；(2) 固定した e は o3 より前で，**どの segment にも覆われない**（S1 は C1 で切れ，S2 は仮想，S3 は o3 で始まる）：`covering` の結果が無い；(3) 更新は **untimed の基準**：`coherence` の `basis` は `mr_34_a_layout_update_before_a_planned_segment_is_untimed_and_the_segment_adds_its_draw`（`:3172`）が untimed の更新に期待する文字列と同じ；(4) **seed による位相の引き**を正確に：その更新が 1 回，S3 の origin の untimed tune が 1 回（Ceiling の「two draws」）で，`lo_draws()` の添字は `:3172` のテストと同じ順序（`retune_draw(0)` が更新，`retune_draw(1)` が origin）．timed の定数 `rx_timed` を使わず，`assert_near` の期待値は `rotated(0.5, tx_phase − retune_draw(i))` の形で書く；(5) `applied()` の `(key, at)` が e；(6) `segments` section に拒否された segment の行が無い；(7) `m.h.rejected() == [REFUSED_CHANGE]`．
- Session（Action を押す）と**本物の事前に admit された Spec Run**（timed の entry を arm の前に admit して Run が始まる：KC-17，`crates/ezsdr-acceptance/tests/v58.rs:802` の形）の両方．Spec Run で受け取り instant を指定できない形なら，事前 admit の e だけを固定し，受け取りの時刻の違いは Session に任せる，と報告に書く．
- `:3548`・`:3572` の 2 本は，期待が変わるかを実装後に確認（`:3572` は e_c = T0+6_000_500 が S1 の覆いの内：値は変わらない見込み，INFERRED）．
落とす間違い：m1，m2（Mock が覆われるとして timed と数える）．

### T7　`reach` の `holds` の channel の絞り（`control.rs`）

`ur_24_a_frequency_waits_for_the_next_segment_that_holds_its_channel`：rx stream 0 に channel 0 を configure 済みの segment A，あとに channel 1 だけを持つ未 configure の segment B（origin > e）．channel 1 の周波数 V（e が B の origin より前）：release では書かない（`device.calls()` に apply 無し），B の configure が運ぶ．落とす間違い：m7（`contains(chan)` を外す：A を Untimed と判定して release で `apply rx 1 … None`）．

### T8　規則 2（`crates/ezsdr-radio`，`ezsdr-radio-uhd`）

- **検査関数**（名前は実装役．1 つ）の単体：lead 1 ms・W 3 ms → 拒む；lead = W → 通す；lead > W → 通す；W = 0・lead = 0（ideal）→ 通す；lead = W − 1 ns → 拒む（境界）．拒否の形は `Err`／`assert!`（D8：programming error は `assert!`）．
- **呼び出しの証明（構築の境目に不正な timing を通す．比較と呼び出しを別の mutation にし，それぞれ単独で殺す）**：
  - **境目 B1（`Description` の構築）**：`Description::new` は固定値の `x3xx::timing(terms)`／ideal の定数しか使わないので，ふつうの入力では不正な timing が来ない．そこで検査の呼び出しを置く所を**timing を引数に取る 1 つの関数**（実装役の名前の例 `Description::build(layout, block_len, terms, timing)`，`channels.rs` の private）にし，`new` は profile の timing を渡してそれを呼ぶ．試験は `channels.rs` の `mod tests` から `build` を直接呼ぶ：lead 1 ms・W 3 ms → `#[should_panic(expected = "RM-6")]`，lead = W → 通る，lead = W − 1 ns → panic（x3xx と ideal の layout の両方）．**`build` の中の検査の呼び出しを消すと，この `should_panic` の試験が panic しなくなって落ちる**（m6b-1．他の row に依らない）．`new` が `build` を経由することは，全 description の試験（下）が `new` で作って規則を満たすことと，`new` から `build` を外す形の変更が構造上できないこと（`new` は `build` の薄い呼び出しだけ）で守る．
  - **境目 B2（`with_measurement_timing`）**：引数の `TimingEnvelope` がそのまま不正な値になりうるので，lead 1 ms を渡す `#[should_panic(expected = "RM-6")]` の試験が注入そのもの．呼び出しを消すと panic せず落ちる（m6b-2．単独）．lead = W は通り，公開される capability の `rx_start_lead_ns >= release_window_ns` を assert．
  - **境目 B3（`candidate_timing`，`measurement.rs:2253`）**：`candidate_timing("rx_start_lead_ns", 1_000_000)` が `Err` を返すこと（`with_measurement_timing` に着く前）．呼び出しを消すと `Ok` になって落ちる（m6b-3．単独）．lead = W（3 ms）は `Ok`．
  - **比較（m6a）**：検査関数の単体（上）が，比較の向き・`>=` を `>` に替える mutation を殺す．B1〜B3 の試験も境界（lead = W は通る，W − 1 ns は拒む）を含むので，比較を壊す mutation は B1〜B3 の各試験でも別々に殺される．
  - m6b の 3 row は上の 3 つに 1 対 1 で対応し，どの row も，その row を殺す試験が他の mutation row の有無に依らない．
- **公開する description のすべて**：`Description::new` に x3xx の layout（master clock の全種，link_gbps 1/10，rx/tx の組）と ideal を与え，`rx_start_lead_ns >= release_window_ns`（ideal は 0 = 0 の等号）．
- **公開する capability**：MockRadio の `capabilities[radio.timing.rx_start_lead_ns] >= capabilities[radio.timing.release_window_ns]`（`mock_radio.rs:499-528` の形）と，UHD の fake の同じ 2 key（`tests/fake.rs:164-166,504-521`）を各 profile で．測定の道の候補（`fake.rs:534-556` の `13_000_003` / `19_000_005`）も通る．
- **測定の掃引の候補（M1 の波及）**：受信の `plan.rx_start_leads_ns` は W 未満の 1・2 ms が構築できなくなるので，**受信だけ**を W 以上に限る：既定 `[3,5,10,25,50,100] ms`，rehearsal（quick）`[3,5] ms`．**送信の `tx_start_leads_ns` は規則 2 の外なので今のまま** `[1,2,5,10,25,50,100] ms`（quick は `[1,2] ms`）．したがって `measurement.rs:4912` の `tx_start_leads_ns == rx_start_leads_ns` は成り立たない（送信 ⊇ 受信でもない：送信に 3 ms が無い）．assert は**それぞれの値を別に**書く：受信の既定 == `[3_000_000, 5_000_000, 10_000_000, 25_000_000, 50_000_000, 100_000_000]`，rehearsal の受信 == `[3_000_000, 5_000_000]`，送信の既定と rehearsal は今の値のまま；さらに**受信の候補のすべてが `default_timing().release_window_ns` 以上**（等号の 3 ms を含む）．W は定数を 2 重に持たず `default_timing()` から読む．影響する試験：`fake.rs:672`・`:7970`（`len() * 2` は追随），`:787` の stage 総数の算術，`:4912`，`tests/hardware.rs` と `bench.md` の lead の記述．**今日の実機で測った lead 1 ms の記録は履歴として残る**（`bench-results.md:120`）．段階 4 は受信の lead ≥ W の範囲だけを測る．
- 時間の振る舞い：T3-c．
- 補助 grep（実装役が実行して報告）：`git grep -n "rx_start_lead_ns\|release_window_ns" -- crates` の代入箇所（`control.rs:5810,5851`，`rx.rs:1102`，`measurement.rs:2263`）のうち，`Description::new` も `with_measurement_timing` も通らない**直接代入**が規則 2 を破っていないか（A9）．破っていれば，その試験の組を直す．

### T9　送信と記録

変わらないもの：tx の clock の記録（origin で終わる，または拒否前に booked な change の cut），`mr_20_a_refused_transmit_change_ends_its_clock_and_cancels_its_bursts` の clock の `ended_at`，bursts の cancel，`ur_30_the_recorded_plan_has_no_refused_segment`，UR-37 の「拒否された segment に owner は装置を呼ばない」．期待を変えずに通ること．

### T-X　送信の起点（A4 の固定．M6）

**timeline**（`tests/timeline.rs`）：
- T-X1（HEAD の特性を先に固定）：tx，C Cold(2 MS/s) e=1000 ready=500 refused，N Cold(1 MS/s) e=1200 ready=1005．HEAD の N の origin = **1200**．実装後は **1250**（拒否なしの値）．
- T-X2（O-eq の tx 版）：T1-a の生成器で tx を回し，refused より後の item が始めた segment の **origin** が拒否なしと一致（`delivered: None` の入力に限る）．記録の cut は比べない．
- T-X3（記録の保存）：`:267-269` の `(1_050*MS, Some(1_050*MS))`，`:316-320` の `(1_050*MS, Some(1_100*MS))` が，拒否ありの plan の**記録の列で**変わらない．origin の列だけが変わる．
- T-X5（繰り返しの拒否）：tx，C1 Cold(2 MS/s) e=1000 ready=500 refused（origin 1050），C2 Cold(2 MS/s) e=1200 ready=1005 refused，N Cold(1 MS/s) e=1500 ready=1300．手計算：C2 の origin = max(1200, 1050+50, 1055) の前に C1 の仮想 cut 1200 + 50 = 1250；N は C2 の仮想 cut 1500 + 50 = **1550**（拒否が全く無い場合と同じ）．C1・C2 の各 clock は origin で終わる記録のまま．
- T-X6（拒否と後継の間の prune／Stop／End）：(prune) 拒否の後で後継の前に `prune(before)`：prune した line としない line で plan が等しい．(Stop) tx の `Stop` は no-op（`:975`）：後継の origin は変わらない．(End) tx の `End` e=1020 ready=1020（拒否の後）：`ended` が立ち，その後の item は効かず（`ended.is_some_and(item.ready >= end)`），refused の clock は origin で終わる記録のまま，N は plan に出ない．期待は **HEAD と同じ**（手で書く：`[(0,1000),(1050,1050)]`）．
**Mock と UHD の両方**（M6）：
- Mock（`mock_layout.rs`，`mr_20_a_refused_transmit_change_ends_its_clock_and_cancels_its_bursts` の `cold_change_refused_at`・`harness` を使う）：refused C（T0+100 ms，2 MS/s，origin 2150 ms），後継 N（e = T0+300 ms，4 MS/s）の 2 通り．**期待は 2 通りで違う**（Sol 1）：
  - (a) **拒否の前に予約**（`harness.step(0)` の前に push．N.ready = 0 < C.e）：N は C を **2300 ms** で切り，今の規則がその cut を保つ（`tests/timeline.rs:324-328`，`timeline.rs:938-941`）．**C の clock の `ended_at` = 2300 ms**．
  - (b) **拒否の後に予約**（`step(e_c)` の後に push．N.ready = e_c ≥ C.e）：**C の clock の `ended_at` = 2150 ms**（origin で終わる記録）．
  - **(a)(b) とも**：N の origin = **2350 ms**（拒否なしの値：C を 2300 で切る cut + lead 50），登録された clock の origin も 2350 ms，plan（`segments`）の origin も 2350 ms，N の clock 上の sample 16 000 の burst の `target` と開始 instant = 2350 + 4 = **2354 ms**（4 MS/s）．(a) では N の clock が拒否の前から登録され，拒否で origin が変わらない．
  - repeated refusal（C1，C2 とも `cold_change_refused_at`）の版（T-X5 の値）．
- UHD（`core.rs`/`control.rs` の tx の試験，`Streams::book`・`streams.refuse`）：同じ (a)(b)，repeated refusal．assert：`plan_of(&core, Dir::Tx)` の origin，`planned()[i].clock`（登録された clock の origin：`core.rs:706-717` が plan の位置で登録し既登録を保つ），burst の `target`，C の clock の end（(a) 拒否の前に予約 = N の cut の 2300 ms 相当，(b) 拒否の後に予約 = 自分の origin 2150 ms 相当．Mock と同じ値）．(a) では booking で登録された clock が refusal の replan の後も同じ位置・同じ origin を保つ．
- 既存の `mr_20_a_burst_on_a_clock_booked_after_a_refusal_plays`：B は **2290 ms に予約**され（e = T0+300 ms = 2300 ms，ready = 2290 ms：`mock_layout.rs:2691-2694`），HEAD の origin は ready + lead の **2340 ms**（`timeline.rs:1043-1053`：max(2300, 2150+50, 2290+50)）．新しい origin は C を e = 2300 で切る cut + lead の **2350 ms**．burst（16 000 sample @ 4 MS/s）は origin + 4 = **2354 ms** に再生され，`harness.step(eb + 100_000_000)`（= 2400 ms）はそれより後．この予定から導いた時刻で，実行で通らなければ時刻を調整せず報告する．

落とす間違い：tx の `end_at_origin` をそのまま残す（T-X1・X2・Mock/UHD の origin が落ちる），記録の cut まで仮想 cut にする（T-X3・Mock の `ended_at` が落ちる，m9）．

### T10　VH-8 の比較

`rm_26_the_timeline_against_uhd_control`（`control/differential.rs:177`），`rm_26_the_timeline_against_uhd_rx`（`rx/differential.rs:111`）は今のまま保つ．落ちたら timeline 以外に原因があるので，差分を直さずに止めて報告する．

## 4. 系列と mutation 行の対応

| 行 | 変える所（exact match の候補は実装後の text に合わせる） | 殺す試験 |
|---|---|---|
| m1 | 拒否された `Cold`/`Start` が仮想 segment を `running` に置かない | T1-a，T1-b，T2，T3-a，T5，T6 |
| m2 | 仮想 segment の cut を `omitted` でなく `out` に push する | T1-a(1)，T1-c，T6 |
| m3 | halt した `Start` が先に仮想 segment を `cut` しない | T4（1554），T1-a（call>0 の seed） |
| m4 | `released` が `omitted` を読まない（`omitted` の chain を外す） | T5 の timeline の 4 本，**Mock `mr_18_a_layout_move_after_a_refused_source_change_…` と UHD `ur_25_a_move_after_a_refused_source_change_…`**（両方で殺す） |
| m5 | 実装後に実行できる形に決める：仮想 segment の印を `Line::prune` の `start.replay` が畳まないようにする具体的な text（例：印を `State` の `Clone`/`replay` が持ち越さない形へ，`start` の State を `prune` 後に印だけ初期化する 1 行）．実装役が row を書き，「match が 1 回」と「T1-e で落ちる」を確かめる．形が決まらなければ実装を止めて報告する | T1-e |
| m6a | 検査関数の比較を外す／向きを逆にする／`>=` を `>` | T8 の単体，B1〜B3 の境界（lead = W は通る，W − 1 ns は拒む） |
| m6b-1 | `build` の中の検査関数の呼び出しを外す | T8 B1 の `should_panic`（単独） |
| m6b-2 | `with_measurement_timing` の中の呼び出しを外す | T8 B2 の `should_panic`（単独） |
| m6b-3 | `candidate_timing` の中の呼び出しを外す | T8 B3 の `Err` の試験（単独） |
| m7 | `reach` の `holds` から `contains(chan)` を外す | T7 |
| m8 | 仮想 segment の cut が `item.delivered` を読む | T1-d の仮想・abort（released 1500 ms）と仮想・Stop（Start 1650 ms） |
| m9 | tx：記録の cut を仮想 cut にそろえる／`previous` を origin のままにする | T-X1〜X3，Mock・UHD の tx の case |
| m10 | `finish` が切られなかった仮想 segment を捨てず `out` に残す | T1-a(1)，`a refused start_rx leaves the stream halted` |

Mock と UHD の両方で殺す：m1・m2・m4。

## 5. 壊れる既存の試験と新しい期待（手計算．実装後に実行して照合する）

下限に緩めない．全て `assert_eq!` の正確な値．

| 試験 | 今 | 新しい期待 | 理由 |
|---|---|---|---|
| `rm_26_the_timeline_cases` `a refused receive change halts the stream until the next change`（`:262-264`） | `[(0,1000),(2000,None)]` | `[(0,1000),(2050,None)]` | 仮想 S2 を C が e=2000 で切り，cut 2000，+ lead 50 |
| 同 `a change within one lead of a refused one counts from the cut made, not the refused origin`（`:269-271`，名前を `…counts from the refused segment's cut` に） | `(1055,None)` | `(1100,None)` | 仮想 origin 1050，cut 0 → instant 1050，+ lead 50 = 1100（ready 1005+50 = 1055 より後） |
| 同 `start_rx resumes a stream a refused change halted` | `(1550,None)` | **変わらない**（call=0）．生成試験では変わりうる | A3 |
| 同 `a refused start_rx starts nothing; …`（3 case） | `3050` / `(0,1000)` | 変わらない | A3 |
| 同 `a refused transmit change ends its clock at its origin`（`:267-269`） | `[(0,1000),(1050,1050),(2000,None)]` | `[(0,1000),(1050,1050),(2050,None)]`（**tx 整合を入れる場合のみ**） | 仮想 cut 2000 + 50．記録の列 `(1050,1050)` は変わらない |
| 同 `a transmit change booked before a refusal keeps its cut`（`:316-320`） | 同じ | 変わらない | N.ready 600 < C.e なので元から C を切る |
| `pruned_move_order`（`:815-865`，2 case：`:867-876`） | `release_after < release_before`，`destination_after < destination_before` | `==`．`release_after == released(&unpruned,..)` と `destination_after == by(&unpruned,..)` は残す | Start の segment は move に切られて origin で終わる．仮想でも同じ end |
| `rm_34_a_refused_source_change_moves_the_destination`（`:1084-1095`） | 拒否後 release 1000，destination 1050 | release 1050 のまま，destination 1100 のまま．名前を `…does_not_move_the_destination` に | 仮想 C を move が cut 0 で切り end = 1050 |
| `mock_layout.rs:3608-3649` の 2 本 | 拒否で早まった S3' が覆うので timed | 覆わないので untimed（T6） | 規則 1 |
| `mr_20_*`・`ur_25_a_refused_*`・`ur_30_*`・`ur_37_*` | — | 実装役が `git grep -n "refus" -- crates/*/tests crates/*/src` で洗い，変わった case だけ，上の形で理由を 1 行ずつ報告 | メモの指示 |
| 生成試験 `rm_26_generated_sequences_keep_the_timeline_invariants` | 5 つに 1 つ拒否 | 変えない．通ること（`check()` は仮想 segment を plan に含まないので成り立つ） | T1(iv) |

tx 整合（A4）は owner に確認が取れていて「同じ規則にそろえる」と指示されたので計画に入れるが，**tx の行の期待値の変更は段階の中で 1 つの塊**にし，Sol のレビューで外せるようにする（§9 D3）．

## 6. spec が変える規則（DA-1 の形：規範の本文 1 トピックを数行，`*Test:*` で終える．理由・履歴・issue 番号は書かない）

`plan/maintenance/tools/rules.py --base <変更の前の commit>` を通す．規則 ID は増やさない．

- **RM-25**（`design/07-radio-model.md:252` の段落「A planned event refused or pre-empted …」の後に 1 段落）：
  「A receive segment that a refused change or `start_rx` would have begun counts, for planning, as running until the next change, `start_rx`, `Stop` or end cuts it; that cut is the previous cut of RM-25's start rule and the instant RM-34 reads the release from. The segment is not in the plan: `covering` does not count it, its owner does not begin it, and it has no clock and no record. The stream halts as before. A transmit change booked after a refusal at or after the refused change's instant begins no earlier than it would had the change not been refused; the refused clock's record is unchanged.」
  `*Test:*` に `rm_25_a_refusal_does_not_move_a_later_origin`，`rm_25_a_refused_receive_segment_is_not_in_the_plan`，`rm_25_a_pruned_line_with_a_refused_segment_plans_as_one_that_keeps_every_item`，T2 の固定 case の名前，`rm_25_a_transmit_change_after_a_refusal_begins_as_if_not_refused`（T-X），Mock の MR-34 の 2 本，UHD の `ur_24_a_refusal_booked_after_a_release_leaves_the_pending_segments_value_to_its_configure`．
  既存の「`a refused change halts` … a transmit clock registered for the refused change ends at its origin — or at the cut a change booked earlier has already made of it」の文は残す（記録は変えない）．「Only transmit clocks can so end with no sample.」は，受信の仮想 segment が `omitted` に入ることと矛盾しない（clock が無い）が，読み違えられるので「ends with no sample **and has a clock**」と直すか，理由を足さず削る：実装役が `rules.py` に通る形で決める．
- **RM-6**（`:89`）：「A Description's `radio.timing.rx_start_lead_ns` is at least its `radio.timing.release_window_ns`; the Provider builds no Description that breaks this.」+ `*Test:*` `rm_6_every_published_description_keeps_the_start_lead_at_or_above_the_release_window`（T8）．
- **RM-34**（release の定義の文「a receive segment RM-25 leaves out of the plan counted as one the line holds」）：「…, a receive segment RM-25 leaves out of the plan counted as one the line holds, a refused one until its cut」と 1 句．*Test:* に T5 の 3 本．
- **design/09 MR-34**：Ceiling の「a timed tune the Mock counts is timed on the UHD Module only if the owner handles a refusal and issues the segment's timed start no later than 2 ms before its origin」から「handles a refusal and」を外す（timed start と host の遅れの前提は残す）．*Test:* に T6．
- **design/18 UR-24**「A receive frequency and the DDC」：「A later refusal does not move a segment that begins after `e` to before `e` (RM-25).」の 1 句．*Test:* に T3．
- 各 spec の Changes 行に 1 行（07・09・18）．`KNOWN_LIMITATIONS.md`：KL-7 に「拒否の後の次の segment は，拒否が無かった場合と同じ時刻に始まる（遷移ごとの遅れ）」，KL-15 の start lead の行に「W 以上（RM-6）」，KL-33 は直ったので status を直す（同じ commit で）．
- `design/` の他に誤りになる文：`design/07` RM-34 の `rm_34_a_refused_source_change_moves_the_destination` を名指す所があれば名前を直す（`git grep -n "moves_the_destination" -- design plan`）．実装役が grep して一覧にする．

## 7. 順序と gate への入力

1. 規則 2 を先に入れる（段階 4 が profile を変える前）：T8（測定の道と候補の表を含む）→ 検査．
2. 実装前に HEAD で実行して記録：T-X1（1200），T3-a（落ちる），T6 の旧い区間の定数（o2・o3・o3'），T1-d の仮想の 2 case（HEAD は読むので落ちる）．
3. 規則 1（rx）→ T1・T2・T4・T5・T6・T7 → 既存の期待の更新．
4. tx 整合（T-X）は別の塊．
5. mutation：m1〜m10 を `remote_mutate.py` に任せる．実装役は行を書くが，ローカルで走らせない．
6. 全試験，Mock と UHD の VH-8 の比較（T10），docker の uhd，Python は orchestrator の gate．

## 8. UHD Module の本体

変えない（メモ §5）．T3・T4・T7 の道具を作るのに本体の変更が要るなら止めて報告する．

## 9. 疑わしい所（INFERRED．Sol の再レビューで見てほしい所）

- **D1（解決）** 測定の道にも規則 2 を強制する（orchestrator の裁定）．残る疑い：測定の候補の表を W 以上にすると，段階 4 は lead < W の範囲を測れない．W を下げる根拠が要るなら，それは別の段階の決めである．`tx_start_leads_ns` の表との等号の assert（`measurement.rs:4912`）の直し方は実装役の最小案．
- **D2** O2 は INFERRED．T1-c が `Lines` の全 stream で固める．反例が出たら実装を止めて報告する．
- **D3** tx 整合：C の記録は origin で終わり，次の segment だけ仮想 cut から始まる．tx の burst の late policy（`decided_at`，`later_bound`）が origin を読むので，後継の origin が最大 lead 遅れると burst の開始が動く．Mock と UHD の登録された clock の origin が plan と一致するか（T-X の Mock・UHD の case）．外すときは T-X・m9・§5 の tx 行・RM-25 の tx の 1 文を一括で外せる．
- **D4（直した）** `delivered` を埋める所は 2 か所：Mock `lib.rs:1241-1249` と UHD `core.rs:504-509`．どちらも仮想 segment には `None`（UHD は `made` が仮想 segment を返さない）．それでも timeline が仮想 segment の cut で読まないことを T1-d で固定する．**INFERRED**：UHD の `first_undelivered` が仮想 segment の origin と偶然一致して `cuts` が真になる並びが無いか（`made` は plan の segment だけを返すので無い見込み）．
- **D5・D6（決めた）** §3「仮想 segment の振る舞い」に Stop・End・後の move・切られない場合の期待を書いた．call>0 の値は T4 の 1554 ms を手計算で完成した．生成試験の call>0 の値は実行で確かめる（oracle は O-eq の等号で，値を手計算しない）．
- **D7** 記録の値は変えない：`Kind::Start` が halted のとき新しい segment を `self.config`（拒否された config）で始める今の振る舞いのままにする（`running` の config が拒否された config になる）．T4 の 1554 ms は 2 MS/s（拒否された config）で再開する値．
- **D8** 規則 2 の「programming error」は `assert!`（panic）．`Description::new` は `Result` を返さない．
- **D9** T6 の固定定数は HEAD で実行して得る（修正前の o2・o3・o3'）．これは「実行結果で調整する」のでなく，旧い早まりの区間を literal で固定する手順で，修正後の実装から導かない（Sol M3）．

## 10. 範囲の外

送信の R4・R2・F7（D9b），profile の値そのもの（段階 4．ただし規則 2 で今の値が通ることは T8 で確かめる），`rx_start` の失敗が拒否として記帳されない件（メモ §8）．

## 11. v1 からの変更

| 項目 | v1 | v2 |
|---|---|---|
| M1 | 測定の道（`with_measurement_timing`）は検査しない案（D1） | 同じ検査を強制．`#[should_panic]` と `candidate_timing` の `Err` の試験．測定の掃引の候補（`rx_start_leads_ns`）を W 以上に限る（既定 `[3,5,10,25,50,100] ms`，quick `[3,5] ms`），影響する試験（`fake.rs:672,7970`，`measurement.rs:787,4912`）を列挙 |
| M2 | T3 は e = o3−1 と e = o3 を同じ期待で | T3-a は e = o3−1 だけを消失の回帰に．T3-b は e = o3 を別に（timed の発行 1 件，configure も V を運ぶ，消費を別に観測）．T3-c は規則 2 の等号の証明（Deferred → b = e−W に lead = W の start → origin = e → configure が V を運ぶ） |
| M3 | T6 は修正後の `early` から e を導く | HEAD で記録した旧い早まりの区間の literal の定数から，受け取り・実効の instant を固定．o3 が拒否なしと等しいこと，覆われないこと，untimed の基準，seed による位相の引き（`lo_draws`，`retune_draw`），記録を正確に．Session と事前 admit の Spec Run |
| M4 | T1(ii) は `Line` だけ | T1-b（`Lines` の拒否前後の move release，後の Stop，A→B→C），T1-c（全 stream で b + lead より前の区間全体と b より前の境目で，足す前後の plan の覆いを比べる．O2 が偽なら止める） |
| M5 | O-eq に `delivered` を混ぜる | O-eq は item の同一性がそろう入力（`delivered` なし）に限る．T1-d で仮想（古い index の有無で cut・release が変わらない）と実（RM-16 の床と abort）を Stop/End の固定 case で．D4 の在庫を UHD `core.rs:504-509` まで直した |
| M6 | T-X4 は burst が再生されるだけ | Mock と UHD の両方で，拒否の前に予約した後継と後に予約した後継，繰り返しの拒否，prune／Stop／End を間に挟む場合．plan の origin・登録された clock の origin・burst の target と開始 instant・拒否された clock の end（保つ）を，求める予定から導いた値で assert．観測の時刻は実行で調整しない |
| m1 | m4 を T6/T3/T4 が殺すと書いた | m4 は move の case（timeline の 4 本，Mock `mr_18_a_layout_move_after_a_refused_source_change_…`，UHD `ur_25_a_move_after_a_refused_source_change_…`）が殺す．m6 を m6a（比較）と m6b（呼び出し，3 か所）に分けた．m5 は実行できる形を実装役が決める条件を付けた |
| m2 | D5・D6 は「実行して決める」 | 仮想 segment の Stop・End・後の move・切られない場合の期待を書いた．T4 に非 0 の call の 1554 ms の算術を完成 |
| D7・D8 | — | D7 は記録の値を変えない，D8 は `assert!` のまま |

## 12. v2 からの変更

| 項目 | v2 | v3 |
|---|---|---|
| Major 1（T-X） | Mock の (a)(b) とも C の `ended_at` = 2150 ms．既存の `mr_20_a_burst_on_a_clock_booked_after_a_refusal_plays` の HEAD の origin を eb（2300 ms）と書いた | 期待を分けた：(a) 拒否の前に予約 → C の end = **2300 ms**（N が切った cut を今の規則が保つ），(b) 拒否の後に予約 → C の end = **2150 ms**，(a)(b) とも N は **2350 ms**，sample 16 000 は **2354 ms**．既存の mr_20 試験は B を 2290 ms に予約するので HEAD は ready + lead の **2340 ms**，新しい origin は 2350 ms |
| Major 2（T8） | 呼び出しの除去を「W = 60 ms の別 row」で殺すと書いた（別 row に依る） | 構築の境目ごとに不正な timing を注入する試験を置き，その呼び出しを消す row が単独で落ちる：B1 `build(.., timing)` を `channels.rs` の `mod tests` から呼ぶ `#[should_panic(expected="RM-6")]`，B2 `with_measurement_timing` に lead 1 ms，B3 `candidate_timing` が `Err`．m6a（比較）と m6b-1/2/3（呼び出し）を別 row にした |
| Minor 3（候補の表） | 送信 ⊇ 受信 | 受信 `[3,5,10,25,50,100] ms`，rehearsal `[3,5] ms`，送信は今のまま `[1,2,5,10,25,50,100] ms`（rehearsal `[1,2] ms`）．`:4912` の等号は成り立たないので，それぞれの値を別に assert し，受信の候補のすべてが W 以上（等号の 3 ms を含む）を assert |
| Minor 4（T1-d） | abort の End の後の Start の origin を比べる | End は `over` で再開を止めるので Start の origin は比べない．abort は「End の後の Start が plan を増やさない」と，同じ e の move（seq 5）の `Lines::released` = 1500 ms（delivered の有無に依らない）．Stop は release 1500 ms と後の Start の origin 1650 ms を delivered の有無で比べる．実 segment の delivered の床の対照は残した |
| T6 の定数 | HEAD で実行して記録 | Sol の HEAD の算術 o2 = 65 ms，o3 = 124 ms，o3′ = 70 ms（T0 からの差）を明記し，production を変える前に HEAD で記録・固定すると書いた |

## 13. 実装で足したこと（実装役，実装後）

計画 v3 から広げた・変えた所と理由．

| 項目 | 内容 | 理由 |
|---|---|---|
| 実装の形 | `State` に `refused_running: bool`（受信：`running` が拒否で作った仮想 segment）と `refused_clock: Option<Segment>`（送信：拒否で origin 終わりにした clock，次の `cold` の cut を前の cut にするまで）の 2 つ．`cut` は仮想なら `delivered` を読まず（`sample_at_or_after` のみ），`omitted` に入れ `out` に入れない．`finish` は仮想を捨てる．halt した `Start` は先に `cut` する．`Line::prune` は `State` ごと畳むので印も畳まれる | 計画 §1 の「実装の形は実装役が決める」の範囲 |
| 規則 2 | `TimingEnvelope::check_start_lead(&self) -> Result<(), String>`（`lib.rs`）が検査の 1 関数．`Description::new` は profile の timing を作って `Description::build(layout, block_len, terms, timing)`（private）を呼び，`build` が `unwrap_or_else(panic!)` で検査．`with_measurement_timing` は同じ呼び出し（panic），`candidate_timing` は `?`（Err） | 計画 T8．B3 は `Err` を返す形が要るので検査関数は `Result` にし，B1・B2 は panic に変換した |
| 既存の試験の期待を変えた | 計画 §5 の 7 本に加え，次を直した：`rm_34_a_move_does_not_wait_a_segment_of_another_channel`（拒否した change が走っていない stream でだけ「一度も走らなかった」になるので，Stop を足して stream を止めた形にした．走っている stream では仮想 segment が release に効くので 150 ms になる．これは設計の意図どおり），Mock の `mr_20_the_next_cold_change_resumes_a_refused_stream`（次の origin は e + 109 ms），`mr_20_only_the_first_cold_change_is_refused`（254 ms），`mr_20_a_refused_move_halts_both_lines`（started + 55 ms），`mr_20_a_refused_transmit_change_ends_its_clock_and_cancels_its_bursts`（後継の clock は later + 50 ms，step を later + 60 ms に），UHD の `ur_30_the_recorded_plan_has_no_refused_segment`（354 ms），`ur_25_a_refused_segment_is_booked_by_uhd_rx`（rx_start が 119 ms）．どれも手計算で先に求めた値が実行と一致した | 計画 §5 の最後の行「洗って報告」 |
| 計画の事実の訂正 | T8 の「rehearsal の送信 `[1,2] ms`」は誤り．実際の rehearsal の `tx_start_leads_ns` は `[1 ms]` で，それを変えていない．`mock_layout` の fault は「その時点で plan が segment を持つ最初の change」を拒否するので，T5 の Mock の move の試験は拒否する change の segment が move より前に origin を持つ並び（change 10 ms，move 100 ms）にした（move と同じ instant 近くだと move が拒否される） | 実行して分かった |
| 新しい試験（計画の名前以外） | `rm_25_a_refused_segment_runs_for_planning_until_the_next_change_cuts_it`（T2），`rm_25_a_start_after_a_refusal_begins_from_the_refused_segment_s_cut`（T4），`rm_34_a_refusal_before_a_stop_or_in_a_chain_does_not_change_a_release`（T1-b の固定 case），`rm_34_a_refused_segment_is_a_release_only_for_a_channel_it_holds_and_that_began_before`（T5 の 3 つの固定 case），`rm_25_a_transmit_refusal_does_not_move_a_later_origin`（T-X2），`rm_25_a_transmit_refusal_followed_by_other_items_keeps_the_next_origin`（T-X5・X6），Mock の `mr_34_a_layout_a_refusal_does_not_move_the_next_segment_s_origin`（T6 の定数 65・124・70 ms），acceptance の `mr_34_a_scheduled_receive_update_inside_a_refused_change_span_is_untimed`（T6 の Spec Run），UHD の `ur_24_a_frequency_due_at_a_segment_origin_is_given_timed_and_the_configure_carries_it`（T3-b），`ur_24_a_deferred_value_survives_a_start_booked_at_the_release_with_lead_equal_to_the_window`（T3-c）など | 名前は spec の Test 行 |
| mutation の行 | P1052〜P1075 の 24 行（m1・m2・m4・m9 は timeline・Mock・UHD の 3 つの試験で別々に，m3・m5・m8・m10・m6a・m6b・m7）．既存 J01・P68・P626・P460 の `old` を変えた text に合わせた | AGENTS.md §7 |
| 実行できなかった・しなかったこと | m9 の「記録の cut を仮想 cut にそろえる」変種の行（記録の cut は今のまま origin で終わる設計で，その形の mutation を機械的に作れる text が無い）．T6 の Spec Run は受け取り instant を指定できないので，事前 admit の e だけを固定した |  |
