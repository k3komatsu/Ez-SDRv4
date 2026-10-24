STATUS: done

# #59 `cold_change_refused` — 実装の報告

HEAD は `3109743`。変更は 12 ファイル，commit・push・`git add` はしていない（AGENTS.md §7，impl-brief §6）。version は上げず，`SCHEMA_CHANGELOG` にも書いていない。

## 1. やったこと（ファイルごと）

| ファイル | 変更 |
|---|---|
| `crates/ezsdr-sim/src/lib.rs` | `FaultKind::ColdChangeRefused`（serde 名 `cold_change_refused`）を既存 3 種の後ろに追加。doc comment は既存の形 |
| `schemas/sim/fault_entry.v1.json` | 再生成（`$defs.FaultKind.oneOf` の 4 項目目）。version v1 のまま |
| `crates/ezsdr-sim/tests/sim_vocabulary.rs` | `se_03_faults_reader` に 4 番目の entry（`at_ns: 20`）と `entries.len() == 4` |
| `crates/ezsdr-mock-radio/src/lib.rs` | 選び方（`refusal`），拒否（`apply_update`，`Lines::refuse`，`COMMAND_REJECTED`，`rejected`，`applied`，fault の行），送信側（`cancel_booked`，`tx_reconcile`，`tx_switch` を呼ばない，halt 中の burst の拒否），受信 block を切らない filter（`ScheduledFault::fires` を `advance`・`emit_rx_until`・`schedule_wakeup` の 3 か所が使う），拒否された change が同じ瞬間に始めた受信 segment の `segments` 行の削除（`forget_refused_segment`） |
| `crates/ezsdr-mock-radio/tests/mock_radio.rs` | 新規 8 本と helper 4 つ（`command_rejections`，`applied_rows`，`cold_change_refused_at`，`REFUSED`） |
| `crates/ezsdr-mock-radio/tests/mock_layout.rs` | 新規 10 本と helper 5 つ（`cold_change_refused_at`，`REFUSED_CHANGE`，`REFUSED_BURST`，`HALTED_STREAM`，`loop_burst`） |
| `design/04-run-and-session.md` | Not-in-scope の fault の数を 4 に，Changes に 1 行 |
| `design/07-radio-model.md` | RM-25 の halt の段落に 1 文（MockRadio は `cold_change_refused` で拒否する），拒否された送信 clock の終端に「先に予約された change が既に切っていればその cut が残る」を追記，RM-25 の Test 行，Changes に 1 行 |
| `design/08-simulation.md` | Not in scope を「these four」に，SE-3 に 4 種目，SE-4 に意味を 2 文，§5 の `se_03_faults_reader` 行，Changes に 1 行 |
| `design/09-mock-radio.md` | MR-18 の Ceiling（拒否は MR-20 の fault によってのみ起こり，拒否された値も `applied` に残る），MR-20 に選び方・拒否・記録・halt の段落（選び方はここ 1 か所だけ）と Test 行，MR-27 に fault の行の規則と Test 行，MR-40 の拒否を「予約できなかった move」に限定，§6 に 14 行，Changes に 1 行 |
| `plan/maintenance/22-timing-simplification.md` | VH-2 の送信 clock の終端に同じ但し書き，issue 表に `#59` **Closed**，follow-up 4 を **Done 2026-10-24** |
| `plan/maintenance/tools/mutations.json` | F22・P589 の `old`/`new` を新しいコードに当て直し，新規 18 行（P604–P621）。整形は変えていない（+155/−5 行） |

`test-plan.md` には §10「実装で足した行」を追記した（実装で増えた状態 A1–A5 と，計画の期待値から変えた数）。

## 2. 計画（impl-brief §1 で直したもの）から変えた所

- **選び方は plan を見る 1 関数 `refusal(e, seq, lines)`**：`f` 以前の change も，`lines` が plan に `by == Some(seq)` の segment を持たない change も候補にしない。`cut == 0` の TX segment は plan にあるので候補になる（S5(3)）。
- **`PendingUpdate` に `cold: Vec<(Direction, usize)>` を足した**：change が名指す line を知るため（選び方は plan，その plan は line ごと）。
- **`ScheduledFault::fires()`** を 1 つ作り，3 か所の filter がそれを使う（plan §0.4 の 3 か所を 1 つの規則に）。
- **`forget_refused_segment`**（自己レビュー 3 回目の P2）：`advance` は同じ instant で `RxBegin`（rank 0）を update（rank 1）より先に処理するので，`ideal` のように origin == `e` のとき拒否された受信 segment の行が `ended_at == origin` で残っていた。RM-25 が plan から除く segment に行だけ残るのは矛盾なので，拒否のときにその行を消す（正常な「同じ instant の 2 つ目の change が cut 0 で切る」場合も行は無い）。
- **halt 中の送信 burst の拒否**（S3(b)）：`handle_tx_burst` の入口で `Line::halted(now)` を見て `MR-25: the stream is halted by a refused change` で拒否する。これが無いと，halt 中に旧 clock の domain の burst が受理され，`tx_cut` が過去なので open burst が送信されないまま残る。
- **reason の文**は計画の案のまま：`MR-18: the cold change was refused`，`MR-18: cancelled by a refused cold change`。halt 中の burst は `MR-25: the stream is halted by a refused change`（計画に案が無かった）。
- 新しい rig・fault の種類・retry・`#[cfg(test)]` だけの本番の道は足していない。UHD Module と VH-8 の harness は変えていない。

## 3. 系列ごとのテスト名

| 系列 | 試験 |
|---|---|
| 1 | `mr_20_a_cold_change_refused_fault_halts_the_stream`（mock_radio） |
| 2 | `mr_20_the_next_cold_change_resumes_a_refused_stream`（mock_radio） |
| 3 | `mr_20_a_start_rx_resumes_a_refused_stream`（mock_layout） |
| 4 | `mr_20_a_hardware_timed_update_in_a_refused_halt_is_recorded`（mock_layout） |
| 5 | `mr_20_only_the_first_cold_change_is_refused`（mock_radio） |
| 6 / S3(a)(b)(c) | `mr_20_a_refused_transmit_change_ends_its_clock_and_cancels_its_bursts`，`mr_20_only_the_refused_clock_s_bursts_are_cancelled`（mock_layout） |
| S3(d) | `mr_20_a_refused_transmit_burst_leaves_the_channel_silent`（mock_layout，channel mode） |
| 7 | `mr_20_a_refused_move_halts_both_lines`（mock_layout） |
| 8 | `mr_20_a_fault_after_every_change_refuses_nothing`（mock_radio） |
| 9 | `mr_20_two_changes_at_one_instant_refuse_the_first_booked`（mock_layout） |
| 10 | `mr_20_a_loss_before_the_refused_change_refuses_nothing`（mock_radio） |
| 11 | `mr_20_a_cold_change_refused_fault_acts_on_both_profiles`（mock_radio） |
| S4 | `mr_20_a_cold_change_to_a_linkless_stream_is_no_candidate`（mock_layout） |
| S5(1) | `mr_20_a_change_before_the_fault_is_no_candidate`（mock_radio） |
| S5(2) | `mr_20_the_fault_follows_the_effective_instant_not_the_origin`（mock_layout） |
| S5(3) | `mr_20_a_change_the_plan_drops_is_no_candidate`（mock_radio），`mr_20_a_transmit_change_with_no_sample_is_a_candidate`（mock_layout） |

各試験は効果を読む前に場面（fault がまだ起きていないこと，stream・clock・予約が着いたこと）を assert する（37c）。期待値は「push の後の最初の step = 受け取り」で計算した（S1）。

## 4. 既存テストの書き換え

- `se_03_faults_reader`：4 種目を足した（3 entries → 4 entries）。
- ほかの既存テストは変えていない。`gz_08_hardware_tests_are_gated` は HEAD で既に落ちている（§8）。

## 5. spec の変更

§1 の表のとおり。選び方は MR-20 に 1 度だけ書き，SE-4・RM-25 は MR-20 を引く。MR-40 の「拒否されると applied も clock も変わらない」は予約できなかった move に限定し，予約後の拒否は MR-25・RM-34 を引く。DA-1 の形（規則に履歴・issue 番号を書かない／`*Test:*`／Changes 表）を守り，`rules.py` の marker・1500 字の制限を増やしていない（新しく長い段落は 2 行に折った）。

## 6. mutation

`remote_mutate.py maintenance sim02:2 …` で，新規 18 行と，コード・殺す試験に触れる既存 4 行を流した。**すべて killed**。

| id | 変える所 | 殺す試験 |
|---|---|---|
| P604 | `f <= e` を落とす | `mr_20_a_change_before_the_fault_is_no_candidate` |
| P605 | plan の segment の代わりに line の存在だけ見る | `mr_20_a_cold_change_to_a_linkless_stream_is_no_candidate` |
| P606 | `by == Some(seq)` の代わりに plan が空でないこと | `mr_20_a_change_the_plan_drops_is_no_candidate` |
| P607 | fault を spent にしない | `mr_20_only_the_first_cold_change_is_refused` |
| P608 | `Lines::refuse` を呼ばない | `mr_20_the_next_cold_change_resumes_a_refused_stream` |
| P609 | 拒否で `COMMAND_REJECTED` を出さない | `mr_20_a_cold_change_refused_fault_halts_the_stream` |
| P610 | 拒否された値の `applied` 行を書かない | 同上 |
| P611 | 拒否された送信 change で `tx_reconcile` の代わりに `tx_switch` | `mr_20_a_refused_transmit_change_ends_its_clock_and_cancels_its_bursts` |
| P612 | 拒否で拒否された clock 以降の burst も消す | `mr_20_only_the_refused_clock_s_bursts_are_cancelled` |
| P613 | halt 中の burst を通す | `mr_20_a_refused_transmit_change_ends_its_clock_and_cancels_its_bursts` |
| P614 | 待っている fault を stop で記録しない | `mr_20_a_fault_after_every_change_refuses_nothing` |
| P615 | 待っている fault が `f` で受信 block を切る | 同上 |
| P616 | この kind を step の loop で fire させる | `mr_20_a_cold_change_refused_fault_halts_the_stream` |
| P617 | serde 名を `cold_change_refuse` に | `se_03_faults_reader` |
| P618 | `f <= e` を `f < e` に（`e == f` を落とす） | `mr_20_a_cold_change_refused_fault_acts_on_both_profiles` |
| P619 | 拒否された送信 change が正常路（clock を採用し burst を残す）を通る | `mr_20_only_the_refused_clock_s_bursts_are_cancelled` |
| P620 | cut 0 の segment を候補から除く | `mr_20_a_transmit_change_with_no_sample_is_a_candidate` |
| P621 | 拒否された change の受信 segment の行を残す | `mr_20_a_cold_change_refused_fault_acts_on_both_profiles` |
| F22（当て直し） | `device_lost` を受信 cut にしない | `mr_20_a_lost_device_delivers_every_sample_before_the_loss` |
| P589（当て直し） | 予約時に `applied` を書く | `mr_18_the_row_is_written_at_its_instant` |
| J08・P567（変えたコードの隣） | 変更なしの再確認 | killed |

`old` はすべて tree で 1 回だけ一致（`rules.py` が検査）。全リストは流していない（orchestrator の gate）。

## 7. 確かめたこと

- 新しい試験の 20 回連続：`mock_radio` の `mr_20_` 18 本 × 20（すべて ok），`mock_layout` の `mr_20_` 11 本 × 20（すべて ok），`sim_vocabulary` 6 本 × 20（すべて ok）。`CARGO_TARGET_DIR` は main tree の既定のまま。
- `cargo test --workspace --no-fail-fast`：**`gz_08_hardware_tests_are_gated` 以外すべて ok**（§8）。
- `cargo clippy --workspace --all-targets -- -D warnings`：警告なし。
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`：0 problems。
- `git diff --check`：問題なし。
- `se_12_schema_freeze`：green。
- 変更したファイルはすべて mode 644。`TMPDIR` は変えていない。負荷をかける操作はしていない。

## 8. 残ったこと・報告

- **HEAD で既に落ちている試験**：`crates/ezsdr-acceptance/tests/governance.rs` の `gz_08_hardware_tests_are_gated` は，HEAD の最後の commit（`3109743`）が `crates/ezsdr-radio-uhd/tests/hardware.rs` に足した `hw_d9_p1` の `#[ignore = "needs a USRP: see plan/phase8/39-dsp-timed-commands.md"]` を，`gz_08` が `plan/phase7/bench.md` の文字列で期待しているため落ちる。このファイルは本作業で触っていない（`git diff --name-only HEAD` に無い）。#59 と無関係なので直していない。orchestrator の判断が要る。
- **MR-40 の「予約後の拒否」の文**（brief S6）は「予約できなかった move」に限定した。予約後の拒否（MR-20）は applied の値と両 line の halt を残す。
- **系列 6 の (b)**（halt 中の burst の拒否）は S3(b) にあるが，「新しい burst は拒否される」を実装する場所が計画に無かったので，halt の判定を `handle_tx_burst` の入口に置いた。spec は MR-20 の段落に 1 文で書いた。
- **VH-8 の harness には入れていない**（brief の範囲外，計画 §8 の 4 点が要るため）。
- **`faults` の行の `at`** は `f`（fault の instant），行を書くのは拒否した時点。MR-27 に 1 文で書いた。
- **系列 2 の復帰 origin は `e + 60 ms`**（計画の `e + 55 ms` ではない）：復帰の change の受け取りが `e + 10 ms` で，RM-25 の ready 項が受け取りを含むため。系列 9 の受け取りは 2 ms，系列 6 の B の burst は clock #2 の sample 16 000（sample 0 だと 2 ms lead に満たず `SendAsap` になる）。`test-plan.md` §10 に記録した。

## 9. セルフレビュー

`opencode2 run --standalone -m "openai/gpt-6-luna#max" --agent plan --auto` を 3 回，毎回新しい reviewer で流した（log は `review-59-1.log`〜`review-59-3.log`）。

| 回 | P1 | 指摘と扱い |
|---|---|---|
| 1 | 1 | **P1 TX clock の終端規則が指示書・試験と矛盾**（spec 3 か所が「常に origin で終わる」と読める）→ **直した**：RM-25・MR-20・spec 22 VH-2 に「先に予約された change が既に切っていればその cut が残る」を追記。<br>**P2 追加規則の Test 参照が不足** → **直した**：RM-25 の Test 行に送信 clock の 2 試験，MR-27 の Test 行に fault の行の 2 試験。<br>**P3 mutations.json が全体整形** → **直した**：HEAD の書式のまま必要な行だけ（+155/−5）。 |
| 2 | 0 | **P2 M9（拒否でも `tx_switch`）を検出できない** → **一部直した／一部退けた**：`cancel_booked` の後に `tx_reconcile` に加えて `tx_switch` を呼ぶ形は，後の change の switch が `cur` を上書きし，`cancel_booked` が唯一観測できる burst を先に消すため，外から区別できない（内部状態だけが違う）。代わりに「拒否された change が正常路を通る（clock を採用し burst を残す）」変異 P619 を足し，`mr_20_only_the_refused_clock_s_bursts_are_cancelled` で killed を確認した。P611（`tx_reconcile` の代わりに `tx_switch`）も別行で killed。指示の「拒否前に burst が受理されたこと」の assert も足した。<br>**P2 cut 0 の segment を候補に含める規則に mutation 行が無い** → **直した**：P620 を足し，`mr_20_a_transmit_change_with_no_sample_is_a_candidate` で killed。 |
| 3 | 0 | **P2 同じ瞬間に拒否された change の零長 RX segment が `segments` に残る** → **直した**：`forget_refused_segment` でその行を消し，系列 11 で `segments` が 1 行だけ・origin が T0 であることを assert。変異 P621 を足し killed。<br>**P3 MR-27 の Test 行の `.;`** → **直した**。 |

3 回で止めた（2・3 回目は P1 = 0）。

## 10. 直し 1 回目

STATUS: done

`sol-code-verdict.md` の 2 件（major 1・minor 1）を `fix1-brief.md` §1 のとおりに直した．HEAD `230aba9` の未 commit の変更の上に置いた（commit・push・add・stash はしていない）．

### 10.1 直したこと

1. **halt の間の burst を burst の clock で決める**（`crates/ezsdr-mock-radio/src/lib.rs:1540-1543`, `:1629-1636`）: `handle_tx_burst` の入口の一律の拒否（`line.halted(now)` で `HALTED_STREAM`）をやめた．`halted` を覚えておき，clock の lookup（`at` の domain の clock，`cur` 以上，plan の segment が cut 無しか cut > 0）を通った後，late policy が決めた `start` がその clock の plan segment の `cut` 以上なら `HALTED_STREAM` で拒否する．したがって:
   - 旧の clock の cut より後の burst（`start >= cut`）は拒否し，cut より前の target は late policy の判定に委ねる（`Drop`／`PlanViolation` はその理由と `TIME_ERROR` のまま）．`SendAsap` が start を cut の後へ移す場合は拒否する（移したまま受けると cut で止まる open burst が残り，以後の burst が「open burst の next sample 以前」で断られる）．
   - 拒否された change の clock は plan segment が cut 0 なので，lookup が MR-16 の理由（"MR-16: target, clock, waveform size, channel count or metadata is invalid"）で断る（以前は入口の一律の拒否が `HALTED_STREAM` を返していた）．
   - plan の後の有効な segment の clock（cut が無い）は受け，`later` に置いてその change が効いたときに送信する（UHD の `book_burst` と同じ）．
2. **spec の linkless RX の文を限る**（`design/09-mock-radio.md:166`）: 「リンクの無い受信 stream の change は候補にならない」を「リンクの無い stream は T0 で止まった line を持ち，`start_rx` が line を再開するまでは候補にならない」に直した（明示の `start_rx` の後の `cold` change は plan に segment を持ち候補になる: `lib.rs:1498-1508`, `:2029-2034`）．その系列は範囲外のまま（足していない）．
3. **spec の halt の文**（`design/09-mock-radio.md:166`）: 「every transmit burst」を「the refusal が切った clock の cut 以降に始まる burst」に限り，拒否された change が始めた clock は cut 0 で MR-16 が断ることと，後の `cold` change の clock は受けること（UR-21 の `book_burst`）を書いた．MR-20 の Test 行と §6 に新しい試験を足し，§6 の既存の行を直し，Changes に 1 行足した（2026-10-25）．ほかの spec は変えていない（spec 04・07・08 と spec 22 は 1 回目のまま）．

### 10.2 試験

- 新規 `mr_20_a_burst_on_a_clock_booked_after_a_refusal_plays`（`mock_layout`）: 系列は `test-plan.md` §11．A の拒否 → B を 300 ms に予約 → **290 ms（B の instant より前）** に B の clock へ sample 16 000 の burst → 受理（`rejected` は `[REFUSED_CHANGE]` のまま）→ 300 ms で B が効き，400 ms までに B の segment で送信: `BurstRecord` 1 本（target `(B の clock, 16 000)`，`blocks` 1，`samples` 100，`Eob`），`stats.tx_blocks == 1`，受信 port に 0 でない sample（channel mode で media に出たこと）．
- `mr_20_a_refused_transmit_change_ends_its_clock_and_cancels_its_bursts`: halt 中の 3 本の burst の理由を，旧の clock の cut の後（`HALTED_STREAM`），拒否された clock（MR-16 の理由），cut より前の target を `SendAsap` が cut の後へ移す（`HALTED_STREAM`）に直した．前の 2 本（旧の clock・拒否された clock への burst が拒否されること）は残している．

### 10.3 確かめたこと

- 新しい試験と直した試験: `mock_layout` の `mr_20` 12 本 × 20 回，`mock_radio` の `mr_20` 19 本 × 20 回，すべて ok（1 binary ずつ，`--test-threads` 既定）．`CARGO_TARGET_DIR` は main tree の既定．
- `cargo test -p ezsdr-mock-radio`: ok（4+32+60+91+1）．`-p ezsdr-sim`: ok（6，`se_12_schema_freeze` を含む）．
- `cargo test --workspace --no-fail-fast`: すべて ok（HEAD `230aba9` は `gz_08_hardware_tests_are_gated` も緑．前の報告 §8 の `gz_08` は `3109743` の話で，HEAD では直っている）．
- `cargo clippy --workspace --all-targets -- -D warnings`: 警告なし．`tm_06_no_crate_reads_ticks_in_their_own_domain` を含む `ezsdr-kernel` の time_model: ok（最初 `ticks_in(x.domain())` を書いて落ちたので，`start` と `cut` の i64 比較に直した）．
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`: 0 problem(s)．`git diff --check`: なし．`se_12_schema_freeze`: green（schema は変えていない）．
- mutation: `REMOTE_SSH="ssh -o BatchMode=yes" ... remote_mutate.py maintenance sim02:2 P613 P622` → **2/2 killed**．P613 は一律の拒否に戻す変異（新しい試験が kill），P622 は cut の判定を外す変異（直した試験が kill）．`old` は 2 行とも tree で 1 回だけ一致．
- `git diff --stat plan/maintenance/tools/mutations.json` は 1 回目の行（P604–P621）に P622 の追加と P613 の直しを足しただけで，ファイル全体の整形は変えていない（差分の削除は F22・P589 の 4 行だけ）．字下げは 1 個の空白のまま．
- 変更したファイルはすべて mode 644．`TMPDIR` は変えていない．負荷をかける操作はしていない．

### 10.4 セルフレビュー

reviewer は `openai/gpt-6-luna#max`（`--agent plan`，読むだけ），新しい reviewer を 1 回起こした（log は `review-fix1-1.log`）．

| 回 | P1 | 指摘と扱い |
|---|---|---|
| 1 | 0 | 指摘なし（P2・P3 もなし）．burst の clock と plan の cut による判定（cut 後の旧の clock は拒否，cut 0 の拒否された clock は lookup が拒否，後の有効な segment の clock は受理），late policy の結果が cut の判定より先に扱われること，新しい試験が B の instant より 10 ms 前に予約して送信まで見ていること，旧の clock・拒否された clock の拒否が残っていること，spec の 2 文と Test 行，mutations.json の差分（194/4 で F22・P589 の更新と P604–P622 の追加だけ）を確認した，という報告だった．テストと mutation の再実行はしていない（報告の結果とコード・assertion の対応から判断した） |

P1 = 0 だったので 1 回で止めた．

### 10.5 残ったこと

- **spec 22 の VH-2 の段落**（`plan/maintenance/22-timing-simplification.md:92`）は「bursts are refused until the next change」のままにした．規範は `design/09-mock-radio.md` の MR-20 で，VH-2 はその決定の要約（fix1-brief §1 が直すと名指したのは design/09 の 2 文だけ）．halt の間も後の `cold` change の clock への burst は受けるので，この要約は今は少し広い．orchestrator の判断が要る．
- mutation の全リスト，`mutate.py` の全行，node の負荷，docker のイメージ，Python suite は流していない（orchestrator の gate）．`sim02:2` で P613・P622 の 2 行だけを流した．
- 前の報告 §8 の `gz_08_hardware_tests_are_gated` は HEAD `3109743` の話で，HEAD `230aba9` では緑（§10.3）．
