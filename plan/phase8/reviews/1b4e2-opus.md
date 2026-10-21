# 1b-4e-2：orchestrator のレビュー，裁定，数

## 1. 何をしたか

UHD Module が論理 channel を同じ方向の別の stream へ動かす（spec 07 RM-34）．1b-4d-2 までは `UR-25:` で拒否していた．

- uhd-control は move を RM-34 の 1 つの item として両方の line に予約し，device は呼ばない．`applied` に論理の行を 1 つ，`timing` に move の行を書く．line が無い stream への move は Mock と同じ理由で拒否する．
- 移動元の owner は，今の segment を終えてから（受信は cut までの sample を届け，送信は cut 前の burst と held の尾を出してから）streamer を閉じる．閉じるのは，channel を持たない集合で開き直すとき，または集合が空になるとき．
- 移動先の owner は，他の stream の記録が channel を開いている間，または先行する未処理の segment（有限の cut を持つ中間の segment を含む）が channel を要る間，host clock で待つ．待ちの後は plan を読み直す．segment が消えた・拒否された・変わった場合は中断で，拒否ではない．待ちは loss，`Provider::stop`，受信の Stop，送信の Stop で終わる．
- swap では，各 owner が自分の streamer を先に閉じてから待つ．こうすると互いの待ちで止まらない．
- from-none（uhd-control の enable）は，他の stream が channel を開いているとき，または他の stream の未処理の segment がそれを要るときには開かず，owner に任せる．
- `Device::rx_close`・`tx_close` を足した（既定実装なし）．`UhdDevice` は open と同じ `control` の mutex で閉じる．
- rate の照合（UR-13）が失敗した open は，その場で streamer を閉じる．lost の device は閉じない．
- `device_gate` で，streamer の open・close・rate の読みと loss の公開を直列にした．
- 共有の `timeline::Lines` は，stream ごとの prune で，move の item と記録を両方の line の実効の境界が move を過ぎるまで残す．その境界は，どの境界も下がらなくなるまで繰り返して求める．
- VH-8 に move の 2 本を足した：stream 間の move と，空の stream への move．
- 実装は Luna（`openai/gpt-6-luna#max`，owner 2026-10-20）．計画は DeepSeek，レビューは Sol 6.1．

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| 計画 | DeepSeek，30 分．S0（channel を `-1` にしてから別の stream に入れると `reconnect` で落ちる．受信・送信・idle の移動先）と，片側の prune で `timeline.rs:278` の `expect` に届く panic を HEAD で確かめた．swap のデッドロックを異論として出した | Sol：FAIL，major 7・minor 2（[1b4e2-sol-plan.md](1b4e2-sol-plan.md)） |
| 裁定 | A1〜A10．Sol の指摘はすべて採り，swap の異論も採った（[1b4e2-adjudication.md](1b4e2-adjudication.md)） | — |
| 実装 | Luna，20:30〜08:21．OpenAI の 5 時間枠の上限で 2 回止まった（02:20，07:30）．枠が戻ってから，同じ session で続けさせた．計画 v2 とセルフレビュー 3 回を含む | セルフレビュー：P1 は 5 → 2 → 1．3 回目の P1（loss で受信と送信の owner が互いに待つ）は直した後にレビューを受けていない |
| Opus | 指示書への忠実さ，変えてはならない挙動，設計の意図（§3） | 採ったのは minor 1 |
| Sol のコードレビュー | 自分の失敗の筋書きから | **FAIL**，major 3・minor 2（[1b4e2-sol.md](1b4e2-sol.md) §1） |
| 修正の回 1 | Luna，48 分．セルフレビューはしない | mutation 14/14 killed |
| Sol の再レビュー | 直したところと隣の系列．連鎖・循環の prune を 2 880 通り試した | **PASS**（同 §2） |
| gate | §4 | — |

## 3. 指摘と扱い

**Sol のコードレビュー**

1. **major：連鎖の move で prune が片側だけを畳む**．相手の stream が「要求した」境界を見ていて，別の move で止まっている実効の境界を見ていなかった．そのため A→B，B→C で B と C だけを prune すると，C が B→C の記録まで畳み，後の拒否が C に届かなかった．直し：実効の境界を不動点まで求め，floor の固定と記録の削除は両方の line が実際に畳むときだけにする．
2. **major：from-none が先行の未処理の segment を追い越す**．owner の open は先行の segment を待つが，from-none は他の stream の開いている集合しか見ていなかった．直し：同じ述語（`needed_before`）を from-none の条件にも使う．
3. **major：P358 と P493 は equivalent ではない**．Sol は probe で確かめた．拒否より前に後の Cold が予約されていると，拒否された送信の segment は正の長さを持つ．送信の End の後も，後の Cold があれば同じ segment が plan に残る．直し：到達する系列のテストを書き，行をそのテストに当てた．
4. **minor：P494 の guard は到達しない**．planner が受信の cut-at-origin の segment を plan から除くので，その guard に届く segment は無い．guard と行を消した．
5. **minor：S8 が projection と実効値を見ていない**．assert を足した．

**Opus**

- **採った：`needed_elsewhere` は `needed_before(…, i64::MAX)` と同じ**．1 本にした．
- **私の誤り：P358 は死んだコードだと読んだ**．Luna が足した送信の cut-at-origin の skip が，拒否された segment を先に拾うと考えた．しかし Sol 3 の系列（正の長さを持つ拒否）がその skip を通らないので，私の読みは誤りだった．
- **採らなかった：送信の Stop を line に予約すること**（`control.rs:391-397`）．
  - timeline は送信の line の Stop では plan を変えない（`timeline.rs:901`）．
  - Manifest の `sequence` は arrival order で，受信の Stop も seq を使う．
  - 待っている owner への出口の信号を別に持つより，今の line を読み直す方が仕組みが少ない．
- **採らなかった：UR-25 の文の長さ**．Sol は，要求の文として書けていて rationale は混ざっていない，と判断した．spec 18 は DA-3 で書き直した spec なので，規則を分けるのは次にその節を触るときに回す．
- **gate で直した：plan 33 の行**．記録の場所が作業 dir（`~/.cache`）を指していたのを，この reviews に直した．

**セルフレビュー（Luna，3 回）**：P1 8・P2 6・P3 1．すべて直した．1 件だけ理由をつけて退けた（native の mutex の remote mutation．remote の image に libuhd が無いので，UHD の feature のテストで確かめた）．中身は [1b4e2-report.md](1b4e2-report.md) §4．

## 4. gate

- テスト：1568 本，clippy，`rules.py` 0 件．
- docker の UHD 4.10：feature の clippy と，`--lib --test uhd_api`．
- Python：3.14 と 3.9 を `ezsdr-server` に対して．
- mutation の全リスト：§6 の表．
- mutation の全行の `old` は，source にちょうど 1 回一致する（見張りの行検査を外していた代わりに，私が確かめた）．

## 5. 数（AGENTS.md §9 の 8）

| 出どころ | major | minor | 機構 |
|---|---|---|---|
| 計画のレビュー（Sol） | 7 | 2 | 計画の系列と assert の誤り．HEAD の潜在の bug が 1 つ：受信と送信の owner が，次の segment が無いと `idle` を立てるだけで streamer を閉じない（1b-4d-2 から）．`stream-lifecycle-scattered` |
| セルフレビュー（Luna，3 回） | P1 8 | P2 6，P3 1 | 次の 3 件が機構に数える bug，残りはテストの検出力と行の当て方 |
| ↳ loss と close・open の競合 | | | `stream-lifecycle-scattered` |
| ↳ 拒否された move の古い `Planned` を使う | | | `planned-state-read-as-actual` |
| ↳ loss の報告が stream の lock に戻る（デッドロック） | | | `stream-lifecycle-scattered` |
| Opus | 0 | 1 | 述語の二重．機構なし |
| Sol のコードレビュー | 3 | 2 | 下の 2 件が機構に数える bug |
| ↳ 要求した prune の境界を，実効の境界として読む | | | `planned-state-read-as-actual` |
| ↳ 同じ衝突を owner と from-none が別々に判定する | | | `radio-rule-implemented-twice`（1 つの Provider の 2 つの経路） |

**機構ごとの数**

- `stream-lifecycle-scattered`：3 件．
  - 1b-4d-2 では，この機構の 2 件に対して「device を呼ぶすべての経路に loss の検査を置く形は，1b-5 の後に見直す候補」と書いた．
  - この段階の `device_gate` は，その検査を 1 か所の直列化にまとめる方向の一歩である．
  - 設計メモは spec 22（VH-1〜VH-5）が兼ねる．
  - **1b-5 の後の見直しを owner に確かめる項目にする**（handoff）．
- `planned-state-read-as-actual`：2 件．
- `radio-rule-implemented-twice`：1 件．
  - 1b-4c の「from-none の拒否が owner と別」と同じ形である．
  - 今回は，同じ述語を両方の経路で呼ぶ形にした．

`failure-mechanisms.md` に 1 行足した．

## 6. mutation の全リスト

全リストの結果（`remote_mutate.py all sim02:8,sim03:8`）：

| list | 結果 |
|---|---|
| maintenance | 877/878（P359） |
| phase3〜phase6 | 83/83，34/34，29/29，63/63 |
| phase7 | 164/165（G09，既知） |

**P359 の生き残り**：`Streams::planned` の `refused` を常に false にする mutant．元のテスト（`ur_25_a_transmit_refusal_from_no_stream_starts_no_clock`）の拒否された 2 つの segment は，どちらも cut が origin 以前にある．そのため，この段階で足した送信の cut-at-origin の skip が先に拾い，`refused` の印を見なくなった．P359 を，Sol 3 で足した正の長さの拒否のテスト（`ur_25_a_refused_enable_with_a_later_cold_has_no_tx_clock`）に当て直し，P358 と一緒に流し直して 2/2 killed．

## 7. Luna（gpt-6-luna，max）の観察

- 計画 v2 を A1〜A10 の対応表つきで書き，S0 の 3 本を実装の前に落として見せた．指示書どおりである．
- 大きさは Rust だけで +2361/−152 になり，目安の約 1 500 行を大きく超えた．指示書の「どこで切るかを報告に書いて続けてよい」に対して，Luna は切る場所を書かず，理由だけを書いた．
- 実装は約 12 時間かかった（上限での停止を除く）．OpenAI の 5 時間枠を 2 回使い切った．
- セルフレビューは 3 回とも新しい P1 を出した．loss と待ちの並行性は，同じ model の reviewer が回を重ねるたびに新しい穴を見つけた．それでも Sol は 3 回目の後に major を 3 つ見つけた．
- equivalent の判定が 2 件誤っていた．「このテストでは差が出ない」を等価の根拠にしていた．
- mutation の見張り（`mutation_watchdog.py`）は，書き換えの多いこの段階で誤報を 4 回出した（P222，P491，P386・P506，P494）．
  - 行の `new` が `old` の一部である行（削除の mutant）は，実装役がその行を当て直す間，見張りが mutant が当たった状態と区別できない．
  - `new` がよくある語（`false`）の行も同じである．
  - 見張りの道具の直しは別の段階に回す．
