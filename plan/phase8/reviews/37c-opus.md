# 設計メモ 37 の 37c：orchestrator のレビュー，裁定，数

## 1. 何をしたか

設計メモ [37](../37-test-thread-timing.md)（#69）§3 の「掃除」の残り．37a の対応表は，試験のスレッドが時刻を読む所の大半を拾っていなかった（`fake.rs` の `direct.now()` が 89 か所，`time_now()` が 16 か所）．37c では grep で全部を挙げ（テストの 4 ファイルで 164 か所，`settle`・`until` で 216 か所），関数ごとの表（[37c-table](37c-table.md)，137 行）にしてから直した．本番のコードは変えていない．

- **制御された時計の unit test に移した**（`control.rs`・`tx.rs`・`rx.rs`・`authority.rs` の tests）：同時刻の 2 つの move（RX・TX），同サイズの membership の swap（新しい segment の実 sample の channel まで），有限の中間 segment の move chain，pending の cold change の floor（3 本），move をまたぐ timed update と read-back，TX の move の cut と held tail，repeat の cut をまたぐ予約済みの burst，拒否された origin の前の次の enable，構成の終わり `c` からの origin，uhd-clock の再アンカーの追従（U35），`rehearsal_b5_overflow`（overflow → 再開の block → `OverflowRestart` の gap）ほか．owner は実際に走らせ，参加者を登録してから時計を放し，試験のスレッドが次の `Frozen` を取るまで最後の owner を参加者のまま待たせる．
- **Spec で開始前に予約**：`captures_across_cold_changes`（予定の数の artifact，partial でない，両方の clock），`ur_25_a_layout_rate_change_…`，`ur_26_a_device_stop_…`．
- **時刻が動いても成り立つ assert**：`ur_07_now_tracks_…`（読みの bracket だけ），`ur_21_a_burst_targets_its_own_transmit_stream`（意図した late burst の後の event だけ），`ur_26_two_receive_streams_end_at_their_own_stops`（状態で観測）ほか．
- `ur_25_a_receive_enable_counts_its_start_lead_…` の 1 tick の不足は Authority の再アンカーで丸めの位相が変わるため（INFERRED）．許容幅にせず，unit test で `ready` が構成の終わりを記録することと origin の式を別々に見る．fake の側は origin の厳密な式だけを残す．
- mutation：F39，P252，P253，P265，P317（filter を 3 本に広げた），P320，P321，P339，P506，P507，P509，P518，R05，U35 を `lib` に当て直した．
- `fake` の binary は 237 本から 219 本に，`--lib` は 295 本から 322 本になった．

実装は Go の DeepSeek，セルフレビューは GPT-6 Luna．

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| 一覧 | Opus が grep で 164＋216 か所を挙げ，Go の DeepSeek が関数ごとに分けた | 129 行，S 106 |
| Opus | S の行を読む | `now + N` の予約が late で floor され `LATE_COMMAND` を出すこと（`control.rs:736-738`）から，61 行目（同時刻の 2 つの move）が S でないと分かった．S の 106 行を Sol に確かめさせた |
| Sol の表のレビュー | | **FAIL**：S から 7 行，(2) から (4) へ 6 行，漏れた系列 9（server の 5 本，`Direct::start`，Python の `test_v54_sleep_is_run_time` ほか） |
| 実装 | Go の DeepSeek | セルフレビュー 3 回（P1 3 → 2 → 2）．5 系列を「既存の部品で作れない」として止めた |
| Sol のコードレビュー | | **FAIL**，major 6，minor 1 |
| 修正 1 | | セルフレビュー 2 回（P1 1 → 1）．止めた 63・64・74 も移せた |
| Sol の再レビュー | | **FAIL**，major 3．`rehearsal_b5_overflow` の原因（項目 5） |
| 修正 2 | | セルフレビュー 2 回（P1 2 → 0） |
| Sol の 2 回目 | | **FAIL**，major 1 |
| 修正 3 | | セルフレビュー 1 回（P1 0）．遅れを入れたコピーで直す前は落ち，直した後は通ることを確かめた |
| Sol の 3 回目 | | PASS |
| gate | §4 | |

## 3. 指摘と扱い

すべて採用した．Sol の指摘の要点：
- **表**：同時刻の 2 つの move，pending の floor，held の burst，prebooked の burst のように，試験のスレッドが遅れると**場面そのもの**が別の系列になる行を S にしていた．記録した `e` を基準に付け替える (2) では floor の前提が消える．
- **コード（major 6）**：有限の segment の cut を instant で渡して cut に届いていなかった．swap の代わりのテストが owner の解放を通らず，試験の側で `complete()` していた．`c` の probe が `apply` の前で構成の終わりを区別できなかった．drift の許容幅を 1 ms に広げた．capture が 0 件でも通った．行 74 は既存の `check_rates` で作れた．
- **再レビュー（major 3）**：同サイズの swap の実 sample，TX の owner の時計を放した後の隙間，drift の bracket が遅れで U35 を見逃す．
- **2 回目（major 1）**：worker が終わってから次の `Frozen` までの隙間．

Opus：
- 実装役の報告で範囲外の `rehearsal_b5_overflow` が負荷の下で 1 回落ちたので，単独で 20 回流した：9/20（load 約 15，`/tmp` は空いていた）．overflow の assert の後の `capture_of` で「artifact が無い」．表の行 102 が S に誤って分類されていた．Sol の見立て（固定の horizon の直後の `finish`，CaptureSink の書き出し，実時間 5 秒の cleanup の期限）のとおり (4) に移した．
- 37b の gate の mutation で P04 は変異なしで落ちた（filter の `test_v54_sleep_is_run_time`）．実装役が測ると，debug の server で 10 s の仮想時間に 4.45 s，release で 0.49 s（負荷 0）．§58 の 2 は成り立っているが，判定は 1 MS/s の RX の 10 s 分（5 000 block）を作る場面の実時間の性能の上限になっている．37c では直していない（§5）．

止めた系列：94 `ur_15_a_failed_later_start_closes_after_the_owners_joined`，95 `ur_29_a_lost_later_start_is_published_before_the_owners_are_stopped`．stream 1 の `rx_start` が stream 0 の owner の後で失敗する device は `tests/fake.rs` の `OnceFails` にしか無く，`FakeConfig` に fault を足すことになる（設計メモ §3：fake の振る舞いを変えるなら risky に分け直す）．fake のテストはそのまま残り，窓も残る．

## 4. gate

テスト 1616（gate で `tm_06_no_crate_reads_ticks_in_their_own_domain` が 1 本落ちた：`rx.rs` の `rehearsal_b5_overflow` の unit test が gap の始まりの tick をそれ自身の domain で読んでいた．Opus が `clock.domain` を名指しする形に直し，`tm_06` と b5 を流し直した．セルフレビューも Sol も UHD の crate だけを流していたので，gate まで見えなかった），clippy，`rules.py` 0 件，docker の uhd（lib 330，`uhd_api` 5），Python 3.14・3.9 各 55/56（`test_v54_sleep_is_run_time` は既知で §5）．ローカルの workspace のテストは owner の解析の job で load 13〜15 の下で流れた．

mutation の全リスト 1291/1294，sim02・sim03 は解析の job で埋まっていたので 1 shard ずつ．J154（`tm_06` の行）は直す前の tree で変異なしに落ちたので，直した tree で流し直して killed．G09 は既知，P04 は filter の `test_v54_sleep_is_run_time` が変異なしで落ちる（§5）．

## 5. 残ったこと

- 94・95 の窓．
- `test_v54_sleep_is_run_time`：仮想時間の差（決定的）と性能の上限を分ける．
- この 2 つは owner の判断を経てから直す．#69 はその後に閉じる．

## 6. 数（AGENTS.md §9 の 8）

| 出どころ | major | minor | 機構 |
|---|---|---|---|
| Sol（表） | 13 行と漏れ 9 | — | 表の分類の誤り．機構なし |
| セルフレビュー（Luna，8 回） | P1 計 11（10 件直した，1 件は止めた系列の記録） | P2・P3 | テストの観測．機構なし |
| Opus | 0 | 0 | 表の S の誤り（61 行目）と `rehearsal_b5_overflow` の揺れを見つけた |
| Sol（コード，再レビュー 3 回） | 6 → 3 → 1 → 0 | 1 | すべて，移した unit test が場面に着いていないか，時計の保護に隙間があった．`wall-clock-test-oracle` の型だが #69 を直す段階の中の指摘なので新しい件に数えない（37a・37b と同じ扱い） |
| gate | 1 | — | 時刻の型の規則（TM-6）を破る test のコード．機構なし |

major の多さ：unit test は rig の上で plan の計算を繰り返すだけになりやすく，owner を実際に走らせて場面に着くこと，時計の保護に隙間がないことを，Sol が 4 回かけて詰めた．次に同じ形の移動をするときは，指示書に「owner を走らせる」「参加者を登録してから時計を放す」「最後の owner は次の hold まで参加者のまま」を最初から書く．
