# 設計メモ 38 の 38a：orchestrator のレビュー，裁定，数

## 1. 何をしたか

設計メモ [38](../38-timed-release-order.md)（#62）の D1・D2 のすべて．risky な段階として，指示書の系列の表 → 実装役のテスト計画 → Sol の計画レビュー → 実装の順で進めた（AGENTS.md §9）．

- **語彙**：`ezsdr_radio::timeline::timed_instant(at, now, lead, window, earlier)`（`layout_instant` の隣）．floor は `now + lead` と，`e ≤ now + window` の先の予約の `e` の大きい方．`radio.timing.release_window_ns` を `TimingEnvelope`・key 表・capability に足した（x3xx 3 ms INFERRED，ideal 0）．schema を再生成した（version は上げない）．
- **MockRadio**：floor を `timed_instant` に，判定の順を late → 満杯に，枠を「`e > r` の予約」にした．停止・喪失の記録は元から `S` 基準だった．
- **UHD Module**：`RELEASE_WINDOW_NS` を消し，window を description から読む．予約の台帳 `booked` で floor と枠を決める（拒否されたものも `e` まで）．行は instant の fold で書く．終わりは `settle` が `S`（`Streams::ended_at`：停止か喪失の早い方，同じ時刻なら喪失）で確定する．`give` は `Handed::{Given(h), Refused, Lost}` を返し，喪失で渡せなかったものは held に残る．「`S` までに渡した」は command ごとの `handed_at`（最後の channel の `apply` の直前の読み）で決める．release の条件は変えず，`released`／`released_at` は D9 のために残した．
- **harness**：UHD の layer 2 は `release_window_matters` を消し，`timed`・`LATE_COMMAND` の中身・正規化した `applied`（`(key, claimed, at, cancelled)`）を全系列で比べる．Ceiling（`S` の前に `e` を迎えたのに release の遅れで渡されなかった live な command）は系列だけから決め，その command の行の比較だけを外す．1 000 系列には Ceiling が出ないので，明示の系列を 2 つ足した．Mock の layer 1 も同じ floor にした．
- **spec**：07（RM-4，RM-6，RM-16 の Ceiling と「`S` までに渡した」の定義，RM-20，RM-21，RM-29），09（MR-3，MR-18 は RM-21 を引く，MR-25），18（§3 の定数，UR-9，UR-24，UR-26，UR-30，U13，VH-8 の表）．overview の P3 (6) を release window について改めた．
- 新しいテスト関数 53 本（UHD・Mock・語彙・harness の分類），既存の書き換え約 20 本．mutation の行は新規 30（P573〜P603，P597 は足してから同値と分かって消した），当て直し 17（maintenance 14，phase7 3），削除 1（release の loop の先頭の喪失の確認が `give` と fold の確認と重なって同値になった P435）．

実装は Go の DeepSeek，セルフレビューは GPT-6 Luna．

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| テスト計画 | Go の DeepSeek | 系列 1〜12 を両 Module の行に，未決 16 件 |
| Opus | 計画を読む | 未決に裁定 O1〜O14，誤り F1（Ceiling の外し方），F2（系列 1 の `held`） |
| Sol の計画レビュー | | **FAIL**，major 7・minor 2．語彙の表の期待値 4 行，Ceiling の外し方が観測に頼る，O5 の「必ず `e > S`」は成り立たない，O7 の枠の境界が D2 を変える，window の試験が release 側を殺さない，終わりの時刻の試験の形，spec の漏れ（RM-6，RM-16，MR-3，VH-8 の表）．すべて採り，指示書 §1 に S1〜S9 として書いた |
| 実装 | | セルフレビュー 3 回（P1 1 → 1 → 2）．1 件は退けて orchestrator へ（停止と喪失が同じ tick） |
| Opus | 本番のコードを読む | release の条件と lock の順（streams の後に rec）を確かめた．同じ tick の件は「先に確定した終わりのまま」と裁定 |
| gate（1 回目） | 並行 | テスト 1664，mutation 全リスト 1316/1318（J11 は `cancel_held` の形が変わって compile error，G09 は既知） |
| Sol のコードレビュー | | **FAIL**，major 2・minor 1：停止の公開の後の release が取り消すべき行を issued にする，harness の Ceiling が channel 無しの後続も外す，spec の文 |
| 直し 1 | | セルフレビュー 3 回（P1 1 → 1 → 1）．実装役が「`now > S` の pass で渡したものを held に戻す」（`given_here`）を足した |
| Sol の再レビュー | | **FAIL**，major 2：`given_here` の穴（拒否・喪失の早い return，pass の先頭の `now` で判定） |
| 直し 2 | | orchestrator が「`S` までに渡した」を command ごとの `handed_at` で定義．セルフレビュー 2 回（P1 0 → 0） |
| Sol の 2 回目 | | **PASS** |
| gate（2 回目） | §4 | |

## 3. 指摘と扱い

すべて採用した（退けたのはセルフレビューの 3 件で，理由は [38a-report](38a-report.md) §7・§9.5・§10.4）．

Opus の裁定で設計メモに足したもの（メモ §8 の「38a で決めた D2 の細部」）：枠は `e > r`，喪失で渡せなかった command は持っていたものと同じ，拒否は終わりでも行なし，「`S` までに渡した」は `handed_at`，終わりの予約の後の release は行を書かない，同じ tick の停止と喪失は先に確定した方．

Opus の誤り：計画への判断 O7（枠を `e ≥ r`）は D2 の「`e` がまだ来ていない」を変えていた（Sol の計画レビュー 4）．取り下げて `e > r` にした．

## 4. gate

直しの後の tree で：テスト 1669（失敗 0），clippy，`rules.py` 0 件，docker の uhd（clippy，lib 361，`uhd_api` 5），Python 3.14・3.9 各 57/57．

mutation：直す前の tree で全リスト 1316/1318（sim02・sim03 が空いていたので 16 shard，約 40 分）．J11（Action の `Stop` が held を取り消す変異）は `new` を `settle` の呼び出しに当て直して killed．G09 は既知．直し 1・2 の後，`control.rs` と harness にかかる全行（1 回目 136，2 回目 139）を流して全部 killed．

## 5. 残ったこと

- D9（packet の流れていない DDC/DUC の release の規則）は別の段階（メモ §5）．
- `at=None` の fallback は `max` に吸収される同値の変異で，1b の試験は結果だけを固定する．
- `targets` は常に 1 channel を返すので，「最後の channel の `h`」の複数 channel の場合は試験も mutation も無い．
- 「`S` までに渡した」は呼び出しを始めた時刻で決める．装置が受理した本当の時刻は INFERRED（装置は返さない）．
- #62 を閉じる comment は owner の確認の後．

## 6. 数（AGENTS.md §9 の 8）

| 出どころ | major | minor | 機構 |
|---|---|---|---|
| Sol（テスト計画） | 7 | 2 | テストの力と計画の誤り．機構なし |
| Opus（計画） | 2（F1，F2） | — | 機構なし．Opus の O7 は誤りで Sol が見つけた |
| セルフレビュー（Luna，8 回） | P1 計 7（4 件直した，3 件退けた） | P2・P3 | 2 件は後で Sol の指摘と重なった（停止の公開の後の release） |
| Sol（コード） | 2 | 1 | 停止の公開の後の release：1 つの command の最後の行を release の fold と終わりの `settle` の 2 つの道が決める（`radio-rule-implemented-twice` の型，1 つの Provider の 2 つの道）．Ceiling の外し過ぎ：harness のテストの力，機構なし |
| Sol（再レビュー） | 2 → 0 | — | 実装役が指示書の外で足した `given_here` の穴．同じ型 |
| gate | 1（J11） | — | mutation の行の当て直し漏れ．機構なし |

`radio-rule-implemented-twice` の型の 4 件は，その機構に答える設計メモの段階の中の指摘なので，新しい件に数えない（37a〜37c と同じ扱い）．

**テスト計画を先に書く手順の評価**（AGENTS.md §9 の最後）：計画のレビューがコードの前に major 7 を見つけ，コードの後の major は 4（2＋2）で，すべて計画が持たなかった「別の thread が終わりを公開する間の release」の interleaving だった．次に終わりや lifecycle に触る risky な段階では，指示書の系列の表に「終わりの公開と release・予約の pass の重なり」を最初から入れ，「渡した」「効いた」のような時刻の境界は orchestrator が観測できる形で先に定義する（今回は Sol に 2 回言われてから定義した）．
