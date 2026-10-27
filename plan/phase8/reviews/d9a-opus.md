# 段階 D9a：orchestrator のレビュー，裁定，数

## 1. 何をしたか

設計メモ [42](../42-d9-production-fix.md) の D9a．本番の UHD Module を変える risky な段階．

| 項目 | 中身 |
|---|---|
| R1（受信の周波数） | timed の受信の周波数は，拒否されていない予定の segment がその channel を持ち，その instant を覆う（origin ≤ e，cut が無いか instant(cut) > e）ときだけ装置に timed で渡す．それ以外は渡さず，宛先（e より後に始まる，拒否されていない，長さが 0 でない最も早い segment．release のたびに予定から読み直す）の configure が当てる．宛先の configure が済んでいれば（`ColdConfig` の印．configure が `updates` の lock の中で立てる．from-none は自分の configure の後に立てる．最初の segment は済んでいるとみなす）release の時に untimed で当てる．行・`live`・`handed_at`・queue の数・投影・利得・送信は変えない |
| R5（FakeDevice） | front end ごとの DDC/DUC の FIFO．実際に sample が流れた区間を装置の時刻で 1 tick に 1 つ取り出す．sample の値は変更の履歴から．DDC は 32 で 33 個目の timed と満杯の間の untimed の周波数の書き込みを失敗にする．DUC は時刻つきの send の間だけ取り出す．subdev の入れ替えで流れを閉じる |
| Ceiling の試験 | S2（送信の channel が離れて戻ると残った timed の値が当たる），F3′（burst の最初の時刻つきの send より後の instant は次の burst まで DSP が変わらない） |
| probe | P4〜P7（`tests/hardware.rs`，実機は未），probe 専用の `UhdDevice::tune_auto`，音の見分け `tests/common/tone.rs`（位相のまとまりと雑音の floor，欠けで INCONCLUSIVE） |
| 文書 | spec 18（§2，UR-24，UR-25，UR-33，Changes），設計 33 の D9 の規則 1〜4，設計メモ 39 と research の訂正 |

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| Opus の調査・Sol の裏取り | 本番の道と UHD 4.10 の source（[research/d9-production-investigation](../research/d9-production-investigation.md)，[d9-sol-verify](d9-sol-verify.md)） | F1・F3′ は CONFIRMED，R1 の例外は REFUTED（外した） |
| 試験計画 | Sonnet 5.5（計画を書いた同じ subagent が実装まで） | Sol の計画レビュー 4 回：major 6 → 2 → 1 → 0（[d9a-sol-plan](d9a-sol-plan.md)） |
| 実装 | 約 2 時間 6 分．セルフレビュー 1 回（free は上限，Go の DeepSeek：P1 0，P2 8，P3 13） | |
| Sol コード | | FAIL：major 4（subdev の入れ替えで流れが残る，P5 の誤判定，公開の順の試験が弱い，F3′ の spec が言い過ぎ） |
| Opus 独立 | | major 1（MA-1，owner に），minor 5 |
| 修正 1 | Sol 4 件，Opus minor 5 件 | |
| Sol 見直し 1 | | FAIL：major 1（P5 の閾値 0.2 が loopback の 0.008 の音を見逃す） |
| 修正 2 | 位相のまとまりと雑音の floor による見分け（`tone.rs`） | |
| Sol 見直し 2 | | FAIL：major 1（受信の sample の欠けで時刻どおりの切り替わりを遅れと判定） |
| 修正 3 | 欠けをまたぐ区間を捨て，移った区間の直前がすき間なく続かなければ INCONCLUSIVE | orchestrator が読んで確かめた：e が区間の境目に揃わないと 2 つの周波数が混ざる区間ができるが，その位相のまとまりは約 0.99 で閾値を超え，基準の区間として数えられるので，時刻どおりの切り替わりを欠けと誤らない |

## 3. 数（AGENTS.md §9 step 8）

| 出所 | major | minor | 機構 |
|---|---|---|---|
| Sol（計画，4 回） | 9 | 4 | 計画の段で：configure 中の宛先への untimed（流れている sample を壊す），fake の FIFO の進み方（host の読みでなく装置の時刻・radio rate），最初の segment の値の消失，ほか観測と境界 |
| Sol（コード，3 回） | 6 | — | fake の流れの閉じ忘れ 1，probe の判定 3，試験の観測 1，spec 1 |
| Opus（独立） | 1 | 5 | MA-1：時刻なしで当てる道が LO の位相の保証（`random_unless_timed_tune`）を失う．MockRadio（MR-34）は失わないので Mock が実機より強い（`radio-rule-implemented-twice` の形．owner の判断待ち） |
| セルフレビュー | 0 | 21 | |

計画の段のレビューで，コードの前に値の消失 2 件と sample の破壊 1 件が見つかった．コードのレビューの major は fake と probe と試験の側で，R1 の本体（覆う規則，宛先，印）は Sol と Opus の両方が設計どおりと判定した．

## 4. gate

- テスト 1919 全部通った．clippy，`rules.py` 0 件，docker の uhd（lib 429，`uhd_api` 5，`--features uhd` の clippy），Python 3.14・3.9 各 57．
- mutation の全リスト 1722/1723（G09 は既知．P216〈受信の stop が stream 0 を止める〉が生き残った：D9a で FakeDevice の `rx_stop` が stream を 2 度引き，止める stream を決めるのは 2 度目なのに行は 1 度目を向いていた．orchestrator が 1 度目を channel を読むだけにして止める処理を 2 度目に集め，P216 と J326 の行を当て直して kill）．

## 5. 残したこと

- **MA-1（owner の判断待ち）**：T0 より前や流れていない間の受信の周波数変更は untimed で当たるので，OBX・UBX の timed tune の位相の保証が無い．推奨は位相の保証を「流れている間の timed の tune」に限ると spec と `KNOWN_LIMITATIONS.md` に書き，MockRadio（MR-34）を同じ規則にそろえ，実機の probe で確かめること．
- 実機で：P4〜P7（R2・F7・R4 の採否），FIFO の容量と streamer の close をまたぐ残り，FPGA image の一致．
- 書かなかった試験：2c，3d（修正 1 で足した），12b，15，14，`handed_at > S` の値を渡さない場合．Sol と Opus はどれも共有の道で覆われると判定した．
- Opus の note：plan が覆っても `rx_start` が拒否されたなど sample が流れない場合は DDC に entry が溜まりうる．単一 channel の往復の move で行き先が飢える既存の穴の疑い（INFERRED，範囲外）．`tune_auto` と `tune_dsp` の重複．
- 本番の `close_device_burst` が最後の 1 sample の送信の 0 を無視する（揺れるテストの調査の副産物，[flaky-tests-1027](flaky-tests-1027.md)）：D9b で送信の道と一緒に扱う．
