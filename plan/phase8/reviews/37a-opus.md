# 設計メモ 37 の 37a：orchestrator のレビュー，裁定，数

## 1. 何をしたか

設計メモ [37](../37-test-thread-timing.md)（#69）の前半．FakeDevice の通しのテストの判定が，試験のスレッドがいつ動くかに依らないようにした．本番のコードは変えていない．

- **規則**：`crates/ezsdr-radio-uhd/tests/README.md` に「判定は試験のスレッドがいつ動くかに依らない」と 4 つの形（Spec で開始前に予約する，時刻が動いても成り立つ assert にする，遅れが判定に関わらない予約は `send_asap_and_flag`，時刻の窓が判定そのものなら制御された時計の unit test）を足した．
- **落ちていた 6 本**：`ur_23` の継続の窓の 5 本は `src/provider/tx.rs` の制御された時計の unit test（手動の時計の FakeDevice，`Frozen`，`device_at`）に移した．fake からは 4 本を消し（同名の unit test が代わり），`ur_23_a_burst_one_sample_after_a_burst_is_played` は Spec で開始前に予約する形に直した．`ur_21_a_descheduled_…` は `start_repeat` にした．
- **同じ隙間のテスト**：fake の 18 系列を，Spec で予約する形，`start_repeat`，時刻が動いても成り立つ assert に直した．4 系列（start-up の前の拒否，clock の origin の前の遅れ，target の順，最初の send が短く切れたときの report の対）は Spec では性質が作れないので `tx.rs`・`control.rs` の unit test に移した．Sol のコードレビューの後，in-flight window の上限（`Stop` の後）と最初の hand-over での再判定も unit test に移した．`uhd_61_02`（acceptance）と `ea_17_a_capture_ahead_starts_at_its_instant`（server）は開始を「頼んだ時刻以後」に緩めた（厳密な開始は `uhd_59` が Spec で固める）．
- spec 18 §6・§9 と Changes．mutation：phase7 の 17 行を `lib` に当て直し（B04，B08，B10，B16，B20，B26〜B29，B31，B32，R06，U18，U19，U36，U44，U46），B41（`close_device_burst` が tail の代わりに空の end-of-burst を送る）を足した．J197，J295，J317 は `fake` のまま．
- 37b（raw device の 13 本を `src/device.rs` へ，実時間の時計の `Direct` 系 6 系列）は次の段階．

テストで約 +770／−375．実装は Go の DeepSeek（`opencode-go/deepseek-v4.1-flash#max`．free の DeepSeek が 15:34 に上限に当たり，走っていたのは 1 本．owner の指示で Go に切り替えた），セルフレビューは GPT-6 Luna（`openai/gpt-6-luna#max`）．

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| 設計メモ | Opus が書いた．負荷の再現（16 コアに 16 本の busy loop，`fake` 3 回で毎回 4〜6 本が落ちる） | Sol FAIL（major 3）→ 直して PASS．owner が §5 の 3 つを推奨どおり決めた（risky としない） |
| 対応表 | free の DeepSeek，3 時間 | 表 A（移す先の系列と根拠），表 B（同じ隙間の一覧）．範囲が広いので Opus が 37a と 37b に分けた |
| 実装 | Go の DeepSeek | セルフレビュー 3 回（P1 2 → 1 → 0），選んだ 30 行 30/30 killed，テストを各 20 回 |
| Opus | 指示書への忠実さ，本番のコードの不変 | major なし．capture の 2 本を緩めた点は Sol に確かめさせた |
| Sol のコードレビュー | | **FAIL**，major 2 |
| 修正 | Go の DeepSeek | セルフレビュー 2 回（P1 1，1．どちらも直した），6 行 6/6 killed |
| Sol の再レビュー | | PASS，minor 1（spec 18 の説明の語．Opus が直した） |
| gate | §4 | |

## 3. 指摘と扱い

Sol のコードレビュー（すべて採用）：
- **major 1**：`ur_23_a_stop_ends_the_burst_within_the_in_flight_window` は Stop の上限の基準を `LogEntry.time` にしたが，Kernel はこの時刻を dispatch の前に読むので，試験のスレッドの遅れが残る．上限は `tx.rs` の unit test に移し，fake には end-of-burst と Underflow なしだけを残した．
- **major 2**：`ur_21_a_burst_inside_the_in_flight_window_is_late` は，5 ms 先の 2 本が遅れて予約されると予約の時点の判定だけで同じ結果になり，uhd-tx の再判定を通らずに通る．再判定（予約では on time，最初の hand-over で動くか落ちる）を `tx.rs` の unit test に移し，fake のテストは消した．
- 設計メモの「同じ形を掃除する」の判断で拾うはずの所を，最初の書き換え（`LogEntry.time`，5 ms の予約）がまた試験のスレッドの読みに頼っていた．規則の (2)「読んだ時刻を判定の基準にしない」の適用漏れ．

セルフレビュー（Luna，5 回）：P1 は計 4 件（target 順を Spec で試せない，start-up の前の拒否に遅れが残る，A-1 が Underflow を見ていない，3 本目の予約の結果を捨てていた，2 本目の再判定の target）．すべて直した．P3 の「manual clock の説明が誤り」「helper」は理由を付けて退けた（[37a-report](37a-report.md)）．

## 4. gate

テスト 1607（clippy，`rules.py` 0 件），docker の uhd（clippy，lib 285，`uhd_api` 5），Python 3.14・3.9 各 56，mutation の全リスト 1293/1294（G09 は既知）．ローカルの workspace のテストは 0 件の失敗（1b-7b の gate では fake の TX のテストが負荷で 4 件落ちた）．

負荷の確かめ（設計メモ §3 の (c)，usrp-lnx03 の 16 コアに 16 本の busy loop）：`fake` 3 回とも 255/255（直す前は毎回 4〜6 本が落ちた），`--lib` 277/277，acceptance の `uhd` 9/9，server の `protocol` 43/43．計画 30 §15 の sim03 の表は，sim03 が別の利用者の解析 job で埋まっていた（load 25）ので流していない．

## 5. 数（AGENTS.md §9 の 8）

| 出どころ | major | minor | 機構 |
|---|---|---|---|
| Sol（メモ） | 3 | 1 | メモの調べの漏れ（rig の時計，掃除の条件，mutation の行）．機構なし |
| セルフレビュー（Luna，5 回） | P1 4（すべて直した） | P2・P3 | テストの観測．機構なし |
| Opus | 0 | 0 | |
| Sol（コード） | 2 | 0 | 2 件とも試験のスレッドが読んだ時刻を判定に使う形が残った：`wall-clock-test-oracle` の型だが，#69 を直す段階の中の review の指摘で，同じ issue の内側なので新しい件に数えない |
| Sol（修正） | 0 | 1 | 文 |
| gate | 0 | — | なし |

risky としない判断（owner 2026-10-23）は，計画を先に見ない分，コードレビューの major 2 が出た．どちらも Sol が見つけ，unit test に移して直った．
