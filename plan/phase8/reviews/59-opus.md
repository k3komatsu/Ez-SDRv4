# #59：orchestrator のレビュー，裁定，数

## 1. 何をしたか

MockRadio が UHD Module の「cold change の read-back 拒否」を再現できるようにした（Phase 8 段階 2，P4）．risky な段階として，指示書の系列の表 → 実装役のテスト計画 → Sol の計画レビュー → 実装の順で進めた（AGENTS.md §9）．

- **sim の語彙**：`sim.faults` の `FaultKind` に `cold_change_refused` を足した（spec 08 SE-3・SE-4）．schema を再生成した（version は上げない）．
- **選び方**（MR-20 に 1 度だけ）：fault は pending のまま待ち，effective instant `e` が fault の instant 以後の `cold` change のうち，`e` の時点の plan にその `seq` の segment があるものを，`(e, seq)` の順で最初の 1 つ，その `e` で拒否する．origin は見ない．後の予約で plan から消えた change，停止中の line，T0 の Stop のまま再開していないリンクの無い受信 stream への change は候補にならない．
- **拒否のしかた**：UHD と同じく語彙の `Lines::refuse` で halt させ（Mock に halt の規則を書かない），`radio.COMMAND_REJECTED`（`action: null`）と `rejected` の行，拒否した値の `applied` の行，fault の行（`at` は fault の instant，`lost` 0）を書く．送信は `tx_switch` の代わりに `tx_reconcile`，拒否された clock の held burst だけを取り消す．halt の間の burst は clock で決める：旧の clock の cut の後と拒否された clock（cut 0）は拒否，後に予約した change の clock は受ける（UHD の `book_burst` と同じ）．この kind は受信 block を切らない（`ScheduledFault::fires`）．
- **spec**：04，07（RM-25），08（SE-3，SE-4），09（MR-18 の Ceiling，MR-20，MR-27，MR-40 を予約できなかった move に限る），maintenance spec 22（VH-2 の要約，follow-up 4 を Done）．
- 新しいテスト 19 本（`mr_20_*`，Mock の統合テスト）と `se_03_faults_reader` の拡張．mutation の行は新規 19（P604〜P622），当て直し 2（F22，P589）．
- VH-8 の harness と UHD Module は変えていない（計画 §8：選び方を harness に二重に書くことになる）．

実装は Go の DeepSeek，セルフレビューは GPT-6 Luna．

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| テスト計画 | Go の DeepSeek | 系列 11，未決 12 |
| Opus | 計画を読む | D1・D3（効く時と選び方）を「`e ≥ f` の，plan に segment のある最初の change」に決めた．D9 は「plan を見るのでリンクの無い受信 stream も候補」と判断したが，これは誤りだった（Sol 4） |
| Sol の計画レビュー | | **FAIL**，major 5・minor 2：`Harness::step` の受け取りの時刻で系列が作れない，初期の membership が無い，TX の後続の系列が無い，D9 の前提が誤り（リンクの無い受信 stream は T0 で Stop），選び方を見分ける試験が無い |
| 実装 | | セルフレビュー 3 回（P1 1 → 0 → 0） |
| Opus | 本番のコードを読む | halt の間の burst の一律の拒否が，後に予約した change の clock への burst まで拒否すると見た（UHD より厳しい）．`mutations.json` の全体の字下げが変わっていたので，HEAD の書式に戻した（差分は足した・直した行だけ） |
| Sol のコードレビュー | | **FAIL**，major 1・minor 1：Opus と同じ halt の burst，spec のリンクの無い stream の文が広すぎる |
| 直し 1 | | burst の clock で判定．セルフレビュー 1 回（P1 0） |
| Sol の再レビュー | | **PASS** |
| gate | §4 | |

## 3. 指摘と扱い

すべて採用した．Opus の誤り：D9（リンクの無い受信 stream も候補）は MockRadio が T0 に Stop を予約することを見落としていた（Sol の計画レビュー 4）．

この段階の途中で，Opus が先に push した設計メモ 39 の probe の commit（`3109743`）が `gz_08_hardware_tests_are_gated` を落としていたことを実装役が見つけた（hardware の試験の `#[ignore]` の文が GZ-8 の決まった文と違った）．`230aba9` で直した．Opus が commit の前に `--features uhd` の clippy だけを流し，workspace の試験を流さなかったため．

## 4. gate

テスト 1688（失敗 0），clippy，`rules.py` 0 件，docker の uhd（clippy，lib 361，`uhd_api` 5），Python 3.14・3.9 各 57/57，mutation の全リスト 1341/1342（G09 は既知．sim02・sim03 が空いていたので 16 shard）．spec 22 の VH-2 の要約（halt の間の burst）は gate で Opus が直した（決定の記録の要約で，規範は MR-20）．

## 5. 残ったこと

- VH-8 の harness には拒否を入れていない．入れるなら選び方を `ezsdr_radio::timeline` の 1 関数にし，generator が狙う change を持てるようにする（計画 §8）．
- 明示の `start_rx` の後のリンクの無い受信 stream の系列は範囲外（候補にはなる）．
- UHD と Mock の違い（計画 §6）：`applied` の行の形（Mock に `note` が無い），`rejected` の instant（UHD の受信は origin 付近の `now`，Mock は `e`），reason の文，UHD は読み返しのたびに拒否できるが Mock は fault 1 つに 1 回．

## 6. 数（AGENTS.md §9 の 8）

| 出どころ | major | minor | 機構 |
|---|---|---|---|
| Sol（テスト計画） | 5 | 2 | テストの力と計画の誤り．機構なし |
| Opus（計画） | — | — | D9 の誤り（Sol が見つけた） |
| セルフレビュー（Luna，4 回） | P1 1（直した） | P2・P3 | 送信 clock の終わりの文．機構なし |
| Opus（コード） | 1 | — | halt の間の burst（Sol と同じ） |
| Sol（コード，再レビュー） | 1 → 0 | 1 | halt の間の burst：送信の予約の可否を MockRadio と UHD Module が別々に決める（`radio-rule-implemented-twice` の型）．Mock が実機より厳しい向き．この段階の中で直したので新しい件に数えない（38a と同じ扱い） |
| gate | 0 | — | — |

計画のレビューがコードの前に major 5 を見つけ，コードの後の major は 1 だった．
