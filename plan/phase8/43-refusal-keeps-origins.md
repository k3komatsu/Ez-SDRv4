# 設計メモ 43：拒否で origin を早めない（受信の周波数の Deferred の欠陥への答え）

Status: owner が採用（2026-10-28）：§5 の (d)（拒否で後ろの origin を早めない）と規則 2（受信の start lead ≥ release window を profile の規則にして守らせる）．Sol のレビュー 2 回（ADOPT WITH CHANGES，指摘は全部反映）．owner の理由の補足：送信と受信を同じ周期で変える実験で，受信だけ拒否で origin が早まると送受がずれる．送信側に同じく origin を早める動きがあれば，43a で同じ規則にそろえる（§8 の送信の行）．ms 単位の時刻が肝心な処理は将来 FPGA（Phase 8c の RFNoC）で行うべきで，規則 2 はそれまでの安全側の決まり（owner）．

| 項目 | 内容 |
|---|---|
| きっかけ | AGENTS.md §6 の三振．D9a の欠陥（Deferred にした受信の周波数が装置に届かない）と，それを UHD Module の中で直そうとした D9a2 の major 16 件．owner 2026-10-28：継ぎ足しをやめて設計を問う．更新そのものを拒否する案も許す |
| 材料 | 欠陥メモ `d9a-defect-deferred-loss.md`（code の道は Sol が VERIFIED），D9a2 試験計画 v5，Sol の計画レビュー 5 回とコードレビュー（`sol-d9a2-plan{,2,3,4,5}.log`，`sol-d9a2-code.log` の最後の節），worktree `Ez-SDRv4-d9a2`（未 commit），このメモへの Sol のレビュー（`sol-note43.log`），設計メモ [42](42-d9-production-fix.md)・[39](39-dsp-timed-commands.md)，[reviews/d9a-sol-plan.md](reviews/d9a-sol-plan.md)，[reviews/ma1-opus.md](reviews/ma1-opus.md)，HEAD `afbb125` |
| 機構 | `planned-state-read-as-actual`：先に予定から読んだ判定（release の時の UHD，適用の時の Mock）を，後で記帳される拒否の replan が偽にする |
| risky | はい：予定の origin（締め切り）と profile の timing を変え，Mock と UHD が共有する `ezsdr_radio::timeline` を変える |

## 1. 1 つの原因，3 つの症状

RM-25 では，拒否された受信の segment を予定から消し，後の segment の origin をその手前の cut から計算し直す．後の origin は，拒否が記帳された時刻よりも前まで早まりうる（§2 F6）．拒否の記帳より前に予定を読んで決めた側は，記帳の後で誤る．

1. **MA1 の計画レビュー**（[reviews/ma1-opus.md](reviews/ma1-opus.md) §2・§3）：拒否で早まる segment について Mock と UHD の判定が食い違った．MR-34 は「owner が拒否を遅れずに処理する」前提を Ceiling に書いて閉じた（`design/09-mock-radio.md:218`）．
2. **D9a の欠陥**：release（e − W）の時に覆う segment が無く，宛先 S3（origin o3 > e）は未 configure なので Deferred になる（`control.rs:755-756,788`）．その後 owner が前の cold change の拒否を記帳すると，S3 の origin は o3' < e に早まる．S3 の configure が当てるのは `effective ≤ origin` の更新だけ（`control.rs:146-153`）なので e の値は入らず，その値は release の道からもすでに消えている．値は装置に届かない．
3. **D9a2**：Deferred の値を表に残し，release のたびに読み直す．読み直すたびに「どの configure が何を運んだか」「装置に何を書いたか」の証拠が要る．証拠の持ち方（`carried`，base の出所，`written_freq`）と時点（lock の待ち，prune，stop，時計）のそれぞれに穴が出た：major は計画で 4・3・1・2・0，コードで 6（処置は §8）．

## 2. 事実

| # | 事実 | 確かさ・出典 |
|---|---|---|
| F1 | DDC の timed `SR_FREQ` は，その時刻以後の sample が通るときだけ取り出される．満杯の DDC は制御 port を止め，untimed の周波数の書き込みも通さない | VERIFIED：`design/18-uhd-radio.md:23`（`rfnoc_block_ddc.v:298-319`，`axi_tag_time.v:207-230`，bench shake-out 2） |
| F2 | radio の timed command（LO）は timekeeper で実行され，sample を要しない | VERIFIED：設計メモ 39 §1.2，MA1 のレビュー |
| F3 | untimed の DDC の書き込みはすぐ当たる．待っている timed の値は，後の sample でそれを上書きする | VERIFIED：設計メモ 39 冒頭の訂正（`ddc.v`，`dds_timed.v`） |
| F4 | uhd-control は e − W より前に release しない．control の loop は Action を 1 つ記帳するたびに release する．したがって，release の後に記帳される予約の時刻 b は b ≥ e − W を満たす | VERIFIED：`control.rs:648`，`:358-365` |
| F5 | 時刻 b の予約は，b 以後に cut を置き，b + start lead 以後に segment を始める．move の release を通して，予約済みの move の宛先の origin を早めることも（後の `Stop` が source を早く切る場合）遅らせることもある（後に挟まれた start：`crates/ezsdr-radio/tests/timeline.rs:707-724,1037-1060`）．ただし早める先は b + lead より後になる | 動きの存在は VERIFIED（`timeline.rs:289-305,777-801,1001-1053`，Sol の `Stop` の例）．「b + lead より前に覆いを足さない」は INFERRED（§6 T1(ii) で固める） |
| F6 | 拒否は受信の segment を予定から消し（`timeline.rs:974-976`），後の origin と release を手前の cut（`State.previous`，`:1041-1049`）から計算し直す（`settle`，`:520-531,777-801`）．早まる先は，元の item の instant と ready + lead にしか縛られず，拒否の記帳の時刻 b には縛られない．今の試験もそれを assert している（`tests/timeline.rs:269-271,833-842,1084-1095`） | VERIFIED |
| F7 | x3xx の W は 3 ms，受信の start lead は 50 ms．2026-10-28 の bench は受信の start lead の候補として 1 ms を測った（段階 4 で profile がこの値になりうる）．lead が W より短いと，F5 の下限 b + lead が e より前に来る．そのため HEAD は拒否が無くても値を失う（Sol の反例：停止中の channel の e = 100 ms の更新を 97 ms に Deferred にする → 98 ms に start を予約 → origin 99 ms → configure は e > origin を入れない） | 値は VERIFIED：`x3xx.rs:398-400`，`plan/phase8/bench-results.md:120`，KL-15．反例の code の道は VERIFIED（Sol：`control.rs:146-153,669-673,784-788`） |
| F8 | 受信の拒否は，owner が configure に失敗したときに owner が記帳する．owner が遅れると記帳は release より後になる | VERIFIED：`rx.rs:306-323`．頻度は INFERRED（owner の遅れが要る：欠陥メモ） |
| F9 | Spec Run の timed template は arm で instant の順に admit され，`Stop` は agenda から自分の instant に出る．したがって受け取りの時の予定には，後の Stop がまだ入っていない | VERIFIED：KC-17（`design/06-kernel-coordinator.md:250`） |
| F10 | T0 より前に設定した周波数では 5 Session の LO の位相が 164.5° ばらつき，流れている間に `at` つきで設定したものは 2.3° だった | VERIFIED（観測）：`plan/phase8/bench-results.md:179` |
| F11 | configure 済みの宛先は，前の segment の sample が終わってから configure されている．そのため D9a が release で untimed に書くときに，届く sample は流れていない | VERIFIED（静的）：[reviews/d9a-sol-plan.md](reviews/d9a-sol-plan.md) 2 回目 |

## 3. 要件

| # | 要件 | 出典 |
|---|---|---|
| R1 | 拒否も cancel もされない値は装置に届く．e に流れている chain には timed で渡す．それ以外は e より後に始まる最初の segment の configure が当てるか，その configure が済んでいれば release の時に untimed で当てる | UR-24（`18-uhd-radio.md:257,262`），RM-21 |
| R2 | 届ける sample のうち，e より前のものには当てない | UR-24 `:262`，RM-21（`07-radio-model.md:233`） |
| R3 | 新しい値の後に古い値を当てない（effective の順） | UC-2，UR-24 `:261`「The device queue remains in e order」 |
| R4 | 取り出されない entry を DDC に残さない（F1 の wedge） | KL-9，設計メモ 42 §1 F1 |
| R5 | 行と記録は今のまま（`applied` の行は e に，configuration of record，floor，queue の数，late，settlement）．記録に出るのは実際の segment だけ | UR-24 `:258-261`，UR-26，MR-27 |
| R6 | 位相を保証するのは，流れている間の timed の tune だけ | UR-24 `:265`（MA-1），KL-32，F10 |
| R7 | Mock は実機より緩くない：Mock が timed と数える受信の周波数は，UHD Module でも timed で渡る | Vision §13・§59，AGENTS.md §3，MR-34（`09-mock-radio.md:214,217-218`） |
| R8 | release で「e を覆う segment が無い」とした値は，その後も e より前に覆われない．origin がちょうど e の segment に覆われるのはよい（その configure が値を運ぶ）．timed で渡した値の覆いが後の cut で減ることは，KL-9 の残り（Ceiling）として扱う | このメモ |

## 4. 案

### (a) Deferred を残し，D9a2 を仕上げる

- 中身：Deferred の値を表 `deferred` に残し，release のたびに (e, seq) の順に読み直す．後の値がもう書かれていれば捨てる（`written_freq`）．覆われていれば timed で渡し，e を過ぎていれば untimed で書いて `LATE_COMMAND` を出す．宛先の configure が運んでいれば捨てる（`carried`，base の出所 `base_freq`）．宛先が無く e を過ぎていれば捨てる（worktree `control.rs:828-881`）．diff は `control.rs` +1181 行（試験 28 本），mutation +306 行．
- 残っている major：コードレビューの 6 件（§8）．
- 新しい危険：読み直しは configure，prune，stop，loss，lock の待ち，時計と交わるたびに新しい証拠を要する．流れている channel に遅れて untimed で書くので，[e, now') の sample は古い周波数になる（新しい KL が要る）．F7 の短い lead の反例も直らない．
- 実機：P4 に加え，遅れた untimed の書き込みでの LO の位相．
- Mock：変えない．ただし MR-34 の拒否の前提（`:218`）は残る．owner が遅れると，Mock が timed と数える更新を UHD は遅れた untimed で当てる（R7 ✗）．
- Session と Spec Run：同じ．文：UR-24 と UR-36 が育つ（読み直しの 4 規則，結果の分類，印の代わりの `carried` の境目），KL-33．Sol は D9a2 の計画レビューで (a) を「UHD Module の中で示された最小の正しい案」としたが，その比較は origin が早まる予定を前提にしている．

### (b) Deferred を外す：覆われず，channel が idle なら release の時に untimed で書く

- 中身：e を覆う segment が無く，release の時に [now, e) に届く sample が無い（予定で判定する）とき，宛先の configure の有無に関わらず untimed で今書く．D9a の印（UR-36 の段落，`ColdConfig.configured`，最初の segment を configure 済みとみなす規則）が要らなくなる．
- 走っている segment が e より前に切れる（cut ∈ (now, e]）場合：untimed で書くと [now, cut) を壊す（R2）．道は 3 つ．(b1) 切れるまで held に残して pass を止める（`control.rs:652-658` の「channel を待つ」と同じ）．後ろの command は最大 W 待つ．(b2) 拒否する．すると同じ `at` の rate と周波数が拒否される（cold の instant が古い lattice に乗ると，cut の instant が e になり覆わない：`timeline.rs:194-200`）．(b3) Deferred に残す．この場合 Deferred は消えない．
- 宛先が configure 中の場合：`give` は全 projection の lock を取ってから書く（`control.rs:765-768`）ので，configure の後に書く．同じ値を 2 度書くだけで害は無い．
- 欠陥：これだけでは直らない．早まった S3 の configure（`settings_on(o3')`）が，release の untimed の後で古い値を書き直す．(d) と F7 の関係が要る．
- 新しい危険：装置を呼ぶ回数が増える．radio の queue で，untimed の LO の書き込みが先に渡した timed の後ろに並ぶ（設計メモ 39 §1.2 から INFERRED）．実機：P4 のほかに無い．Mock・Session・Spec Run：変わらない．文：UR-24 の印の文と UR-36 の印の段落を消し，idle の規則を足す．

### (c) 更新を拒否する

- 決める時：release の時（e − W 以後）．受け取りの時には決められない．Spec Run の更新は arm で admit され，Stop は後から来る（F9）．Session でも，後の command が予定を変える．
- 拒否するもの：(c1) e を覆う segment が無い受信の周波数のすべて．(c2) そのうち `at` のあるものだけ（`at` の無い Session の `set` は D9a のまま）．
- 利用者に見えるもの：release の時（e の約 3 ms 前）に `radio.COMMAND_REJECTED { kind: update_parameter, reason: "UR-24: no receive segment covers the instant" }` と `rejected` の行が出る．`applied` の行は無い．値は configuration of record にならず，projection から外れる．queue の席は今の「release の時の refusal」と同じく e まで残る（UR-24 `:261`）．`set` はもう admitted を返しているので，拒否は event でしか分からない（KL-23）．KL に新しい項目が要る．
- 同じ結果を得る道：最初の値は Spec か BindingProfile に書く．(c2) なら，Session で stop → set → start（EA-16）．Spec Run では「次の segment を新しい周波数で始める」道が無くなる：Stop → 周波数 → `start_rx` と並べても，予定に `start_rx` の segment があるので周波数は覆われず，拒否される．`at` を次の origin + W 以後にすれば通るが，その間の sample は古い周波数になる．同じ `at` の rate と周波数も拒否される（instant が古い lattice に乗るとき）．
- 失う use case：(c1) は EA-16（`design/16-easy-api.md:92-93`：「a stopped receive stream remains stopped through parameter changes until its start()」，start は「the configuration then in force」）．(c1) も (c2) も，上の Spec Run の 2 つを失う．
- 欠陥：(c1) では Deferred が無くなるので直る．(c2) では `at` の無い更新に欠陥が残る．
- Mock：MR-18・MR-34 に同じ拒否が要る．Mock は e で，UHD は e − W で判定する．その間に記帳された拒否が segment を早めると，UHD は拒否し，Mock は覆われているとして受ける（R7 ✗）．したがって (d) が要る．文：UR-24 に拒否の文，MR-18・MR-34 に同じ拒否，KL の項目．(c1) は EA-16 と矛盾する．

### (d) 拒否で origin を早めず，start lead を release window 以上に保つ（推奨）

- 規則 1（RM-25 に 1 文）：受信で，拒否された change か start が始めるはずだった segment は，予定の計算の上では走っているものとして扱う．次の change，`start_rx`，`Stop`，終わりがいつもどおりそれを切り，その cut が RM-25 の開始規則の「前の cut」になる．RM-34 はそこから release を読む．その segment は plan から外す（`covering` は覆わず，owner は始めず，clock も記録の行も作らない）．stream は今どおり halt する．
- 規則 2（RM-6 に 1 文）：description の timing は `radio.timing.rx_start_lead_ns ≥ radio.timing.release_window_ns` を満たす．値は code の定数なので（`x3xx.rs:393-404`，`channels.rs:380-389`），description を作る所で検め，満たさなければ programming error とする．公開するすべての description について試験する．F7 の反例があるので，この規則は (d) が無くても HEAD の D9a に要る．
- 義務（Sol の指摘で，大域の主張を 2 つの狭い義務に替えた）．
  - O1：受信の拒否の記帳は覆いを足さず，生き残る origin と release を早めない（規則 1 による）．
  - O2：時刻 b の予約は，b + start lead より前に覆いを足さない（F5）．F4 から b ≥ e − W なので，規則 2 の下では e より前に覆いを足さない．等号（origin = e）では configure が e ≤ origin の値を運ぶので（`control.rs:148`），値は失われない．
- 足りる理由：O1 と O2 で R8 が成り立つ．D9a の Deferred と untimed の判定は e まで偽にならない．R7 も成り立つ．Mock が e で「e − W までに始まった segment が e を覆う」と見たとき，その覆いは拒否で足されたもの（O1）でも，release の後の予約で足されたもの（O2：始まりは e 以後）でもない．よって release の時にもあり，UHD は timed で渡している．一方，timed で渡した後の cut は覆いを減らすので，KL-9 の残りのまま．似た lifecycle の race を (d) が全部直すとは言わない（§8）．
- 実装：`timeline.rs` の `State` に，走っている segment が拒否されたものだという印を 1 つ足す．拒否された受信の `Cold` と `Start` もその segment を `running` に置く．`cut` はそれを `out` でなく `omitted` に入れ，`finish` は切られなかったものを捨てる．halt した stream の `Start` は，先にそれを切ってから始める．prune は印ごと `start` の State に畳む．`Omitted` の doc（`timeline.rs:867-877`「cut with no sample，ends at its origin」）を「sample の無いもの（origin で終わる）か，拒否されたもの（cut で終わる）」に直す．規則 2 の検査を `Description::new` に置く．十数行の見込み（INFERRED）．UHD Module の本体は変えない（D9a のまま）．
- 消えるもの：欠陥（S3 の origin は o3 のままなので，configure が値を運ぶ），F7 の反例，D9a2 の道（§8），MR-34 の Ceiling のうち拒否を処理する前提（`09-mock-radio.md:218`）．timed start と host の遅れの前提は残る．
- 新しい危険：
  - 拒否 1 回の遷移ごとに，次の segment はその拒否が無かった場合と同じ時刻に始まる．今より遅れる量は，拒否された segment の rate での受信の call 1 回と allowance と start lead に，lattice の丸めと ready が加わる．拒否が重なれば各遷移の制約が残るので，総計の上限は約束しない．
  - 拒否の後の `start_rx` は，拒否された segment の origin + start lead より前に始まらない．move の宛先も，拒否された source の segment の終わりを待つ．
  - 段階 4 は受信の start lead を 1 ms にできない．W 以上（3 ms）にするか，W を下げる根拠を測る．
  - 期待が変わる既存の試験がある（§6）．
- 実機：(d) で新しく確かめることは無い（予定は host の中だけの話）．P4 は KL-9 の残りのために今どおり要る．段階 4 の start lead の測定は W が下限になる．
- Mock：同じ `timeline` を使うので，同じ規則になる．MR-34 の規則と MR-18 は変えない（更新は今どおり e に当たる）．MR-20 の拒否の後の origin は今より遅くなる．MockRadio の x3xx の timing も同じ定数なので規則 2 に従う．記録（`segments`，clock）には実際の segment だけが出る．Session と Spec Run：違いは無く，API と schema も変わらない．
- 文：RM-25 に規則 1 と Test の行．RM-6 に規則 2 と Test の行．RM-34 の release の数え方（`07-radio-model.md:321`「a receive segment RM-25 leaves out of the plan counted as one the line holds」）に，拒否された segment は RM-25 が切る所で終わることを足す．UR-24 の「A receive frequency and the DDC」に「a later refusal does not move that segment before `e` (RM-25)」の句を足す．MR-34 の Ceiling を短くする．各 spec の Changes の行．KL-7 に遷移ごとの遅れの 1 文，KL-15 の start lead の行に「W 以上（RM-6）」．規則 ID は増えない．

| | 欠陥 | D9a2 の major | R7（Mock） | 失う use case | 足すもの | 文 |
|---|---|---|---|---|---|---|
| (a) | 直る（遅れれば untimed）．F7 は残る | 6 件が残る | ✗（`:218` が残る） | 無し | 表 3 つ，境目，読み直し | UR-24・UR-36 が育つ，KL |
| (b) | (d) なしでは直らない | — | 今どおり | (b2) なら同じ `at` の rate と周波数 | idle の判定（印は消える） | UR-24・UR-36 |
| (c) | (c1) は直る，(c2) は残る | — | ✗（(d) が要る） | (c1) は EA-16，両方で Spec Run の 2 つ | UHD と Mock の拒否の道 | UR-24，MR-18・MR-34，KL |
| (d) | 直る．F7 も規則 2 で塞ぐ | 道ごと消える（§8） | ✓ | 無し（拒否の後の halt が遷移ごとに長くなる，段階 4 の lead ≥ W） | timeline の印 1 つ，timing の検査 1 つ | RM-25・RM-6 に 1 文ずつ，RM-34 の句，UR-24 の句，Ceiling の短縮 |

## 5. 推奨：(d)

(d) を採る．原因は Vocabulary の予定が拒否で後の origin を早めることなので，共有の `timeline` で 1 度直す．D9a の判定がもう 1 つ依っている時刻の前提（start lead ≥ W）は規則にして守らせる．UHD Module の D9a はそのまま正しくなる．判定は release の 1 か所だけで，読み直しも，装置に何が届いたかの証拠も要らない．Mock も同じ予定と同じ timing を読むので，MA1 が置いた拒否の前提が消える．費用は，故障である拒否の後に stream が遷移ごとに長く止まることと，段階 4 で start lead を W より短くできないことである．失う use case は無い．

退けた案：
- (a)：仕組みを足すたびに新しい交差が出た．直した後も Mock が緩い場所が残り（R7），F7 の反例も残る．
- (b)：単独では欠陥を直さない．(d) の上では D9a の印を消す単純化になるが，直す欠陥が無いのに，レビュー済みの道（D9a の Sol 計画レビュー 4 回）を変えることになる．今はしない．
- (c)：(c1) は EA-16 と Spec Run の 2 つの use case を失い，(c2) は欠陥を残す．どちらも R7 のために (d) を要する．
- 規則 2 の代わりに，(d) を「lead ≥ W の profile だけ」と書いて値の変更を見張る：守らせる仕組みは結局同じ試験になり，規則にした方が段階 4 の指示書に届く．
- timeline が受信の start lead として max(lead, W) を使う：RM-25 の lead の意味が隠れて変わる．
- UHD Module の中だけで origin を早めない：Mock と UHD で予定が分かれる（`radio-rule-implemented-twice`）．
- 拒否された segment を plan に残す（覆う）：sample の流れない chain に timed の値を渡すことになり，F1 の wedge に戻る．

## 6. 試験（系列）

| # | 系列 | 期待 | 置き場 |
|---|---|---|---|
| T1 | 生成した 1 000 の系列（`rm_26_generated_sequences_keep_the_timeline_invariants` と `rm_34_generated_moves_keep_both_lines_invariants` の生成器）．(i) 受信の change，start，move を 1 つずつ拒否して前後を比べる．(ii) 受信の item を記帳の順に足して前後を比べる．(iii) 拒否の後，仮想の segment が origin の前（pending）か後（running）にある所で prune し，その後に Stop，Start，change，move を記帳する | (i) 拒否された segment は plan に無い（`by` が一致する segment が無い）．覆い (channel, instant) は増えず，生き残る origin と `Lines::released` は早まらない．(ii) b + start lead より前の覆いは増えない．(iii) prune した `Lines` としない `Lines` で plan，release，halted が等しい | `crates/ezsdr-radio/tests/timeline.rs` |
| T2 | S1 → C1（拒否）→ C2（S2 の途中）．拒否が 2 回重なる形も | S3 の origin は拒否の前の値と正確に等しい（下限でなく値で）．S2 は plan に無い．[c1, c2) は halted | timeline |
| T3 | 欠陥の系列（D9a2 計画の試験 1 の形）：e ∈ (o3', o3) を release する（Deferred，装置を呼ばない）→ owner が C1 の拒否を記帳する → S3 の configure | configure の untimed の書き込みに V があり，timed の apply は 0，`dsp_pending == 0`，`freq_at(o3) == V`．境界は e = o3 − 1 tick．HEAD では落ちる | `control.rs` の `mod tests`（manual clock） |
| T4 | 拒否の後の `start_rx`（拒否された segment の origin の前と後）．from-none の enable を uhd-control が拒否した後の次の enable | 新しい origin は，拒否が無かったときの予定から決まる値（正確に）．その間の Deferred の値は次の configure が運ぶ | timeline，`control.rs` |
| T5 | 成功する move が依る source の change か start を拒否する．A → B → C の chained move で A の change を拒否する．move 自体を拒否する | 宛先の release と origin は動かない（今の `rm_34_a_refused_source_change_moves_the_destination` の逆）．B の release と C の origin も動かない．move 自体の拒否では両 line が halt し，後の move は仮想の segment の終わりを待つ | timeline（`Lines`） |
| T6 | Mock：MR-34 の 2 本を書き換える（今は早まった S3' が覆うので timed）．Session（Action を押す）と Spec Run（timed の entry を arm で admit する：KC-17）の両方で | origins は [T0, o3]．e は覆われないので untimed（timed tune の定数を使わない）．`segments` に拒否された segment の行は無い | `mock_layout.rs:3598-3649`，Spec Run の試験 |
| T7 | `reach` の `holds` の channel の絞り：別の channel の configure 済みの segment が先にあり，対象の channel の未 configure の segment が後にある | release では書かず，対象の segment の configure が運ぶ（欠陥メモの末尾） | `control.rs` |
| T8 | 規則 2：公開するすべての description（x3xx の各 terms，ideal）．検査の関数に lead 1 ms・W 3 ms（F7 の反例）と，lead = W の等号を与える | すべて満たす．反例の値は拒む．等号は通し，W = lead の line で e − W に予約した segment は e 以後に始まる | `crates/ezsdr-radio`（`channels.rs` の試験，timeline） |
| T9 | 送信と記録 | 送信の拒否の case（`tests/timeline.rs` の「a refused transmit change …」，`mr_20_a_refused_transmit_change_ends_its_clock_and_cancels_its_bursts` ほか）は期待を変えずに通る．`ur_30_the_recorded_plan_has_no_refused_segment` と，UR-37 の「拒否された segment に owner は装置を呼ばない」も通る | 既存 |

- 期待が変わる既存の試験：`tests/timeline.rs:269-271`（「a change within one lead of a refused one counts from the cut made」：名前も変える），`:833-842`（`pruned_move_order`．source-first と destination-first の 2 case，`:867-876`），`:1084-1095`（`rm_34_a_refused_source_change_moves_the_destination`），`mock_layout.rs:3598-3649` の 2 本．ほかの拒否の case（`mr_20_*`，`ur_25_a_refused_*`）は実装役が grep で洗う．期待は新しい正確な値に直し，下限に緩めない．
- VH-8 の比較（`control/differential.rs:177` の `rm_26_the_timeline_against_uhd_control`，`rx/differential.rs:111` の `rm_26_the_timeline_against_uhd_rx`）は今の形のまま保つ．
- mutation の行：
  - (m1) 拒否された受信の segment を `running` に置かない（今の振る舞い）→ T1(i)・T2・T3・T5・T6．
  - (m2) それを `out` に入れる（plan に出る）→ T1(i) の「plan に無い」と T6．
  - (m3) halt した `Start` が先にそれを切らない → T1(i)・T4．
  - (m4) `released` がそれを読まない → T5．
  - (m5) prune がその印を畳まない → T1(iii)．
  - (m6) 規則 2 の検査を外す，または向きを逆にする → T8．
  - (m7) `holds` の channel の絞りを外す → T7．

## 7. 次の段（43a，risky）

1. owner が (d) を決める．D9a2 の worktree は捨てる．T3 と T7 の形は，D9a2 の試験計画の試験 1 と試験 7 から持ってくる．
2. 指示書の系列は §6，UHD の振る舞いは §2 を共有する．実装役が試験計画を書き，Sol が計画をレビューし，その後に実装する．
3. 範囲：`timeline.rs`（規則 1，`Omitted` の doc），`channels.rs`（規則 2 の検査），spec 07 の RM-6・RM-25・RM-34，spec 09 の MR-34（MR-20 に origin の文があればそれも），spec 18 の UR-24 の句と Changes，KL-7・KL-15，試験 T1〜T9，§6 の既存の試験，mutation の行．UHD Module の本体と D9a の規則は変えない．
4. 順序：規則 2 は段階 4 が profile の timing を変える前に入れる．段階 4 の指示書には RM-6 の関係を書く．
5. gate：全試験，mutation の全リスト（timeline の行は Mock と UHD の両方の試験で殺す），docker の uhd，Python．

## 8. D9a2 の major 6 件の処置と，このメモで直さないもの

| # | D9a2 のコードの major | 処置 |
|---|---|---|
| ① | 覆う segment の configure が運んだ後に，古い値を untimed で再生する | D9a2 の道ごと消える．HEAD の普通の release では，覆われる値は origin ≤ e．configure は effective が origin 以下の更新を運ぶので，e > origin なら release される値は configure の境目より新しい．e = origin なら configure が同じ instant の更新を既に運んでいることがあり，同じ instant の装置の queue の振る舞いは今のまま（Sol，note 43 の見直し）．同じ instant に 2 つの値があれば，DDC が 1 入力 sample ずつ取るので，古い方が 1 入力 sample だけ当たる（F1）．これは既存の振る舞いとして残し，試験は足さない |
| ② | prune が運んだ証拠を消し，値を再生する | 道ごと消える．HEAD は宛先自身の印だけを読み，宛先は prune されない（[reviews/d9a-sol-plan.md](reviews/d9a-sol-plan.md) 4 回目） |
| ③ | projection の lock を待つ間に公開された stop を見ない | 既存の振る舞いとして残る．HEAD の `give` も lock を取った後に loss しか見ない（`control.rs:768-788`）．stop の後に手渡したものは，手渡しの時刻で S に cancel する（UR-26，`:834-845`，`ur_26_a_command_given_after_the_stop_s_instant_is_cancelled_at_it`）．D9a2 の「stop の後は書かない」約束は採らない |
| ④ | plan の lock より前に時計を読む | 道ごと消える（時計で timed と遅れの untimed を選ぶ判定が無い）．HEAD は pass の始めの `now` を使い，遅れた timed は着いた時に当たる（UR-24 `:261`，`ur_24_a_release_after_its_instant_applies_on_arrival`） |
| ⑤ | 出所の試験が 2 つの主張を観測しない | 消える（出所が無い）．試験だけの指摘 |
| ⑥ | UR-24 の queue の文が誤り | 消える（その文を入れない）．文だけの指摘 |

直さないもの：
- timed で渡した後に記帳された cut や拒否が，e の前で流れを止める場合：entry が DDC に残り，次の segment の最初の sample がそれを取る（KL-9 の残り，`ur_24_a_cut_booked_after_the_release_leaves_the_entry_for_the_next_segment`，bench P4）．cut で残る数は W の中で渡したものだけ（INFERRED）．owner が遅れて記帳した拒否で残る数は，拒否された segment の span に渡したものすべてで，遅れによっては 32 を越えうる（INFERRED．KL-9 の「予定は覆うが流れない」と同じ類で，(d) の前からある）．
- release が宛先の origin より遅れた（control が W より長く止まった）場合の untimed：configure が運んだ値を書き直し，同じ pass で新しい値に戻るまでの一瞬，古い値になりうる（INFERRED．D9a からある）．UR-24 の「released after e applies on arrival」の内に入る．
- `rx_start` の失敗は拒否として記帳されない（`rx.rs:332-335`）．予定が覆っていても sample が流れず，timed の entry が溜まりうる（KL-9 の INFERRED）．別の issue にする．
- 送信：拒否された change の後の clock の origin（`timeline.rs:938-941,977-979`）．登録済みの clock と予定が食い違いうる（INFERRED，未確認）．43a で確かめ，受信と同じく origin を早めるなら同じ規則にそろえる（owner 2026-10-28：送受がずれないため）．規則 1 は受信に限るので，送信の振る舞いは変えない（T9）．

## 9. 数（failure-mechanisms.md，案）

- 数えるもの（設計の機構）：D9a の欠陥 1 件，D9a2 の計画の major 7 件（1 回目 M1・M2・M4，2 回目 M1・M2，3 回目，4 回目 M1），コードの major 4 件（§8 の ①〜④），計 12 件．
  - `planned-state-read-as-actual` 9 件：欠陥，計画 5 件（1 回目 M1・M2，2 回目 M1・M2，3 回目），コード ①③④．
  - `pending-provenance-collapsed` 2 件：計画 4 回目 M1，コード ②．
  - `stream-lifecycle-scattered` 1 件：計画 1 回目 M4（新しい道の stop・loss・拒否との交差が未定義）．
- 数えないもの（AGENTS.md §6：試験だけの指摘と，規則の古い・誤った写しの指摘）：試験だけ 4 件（計画 1 回目 M3，2 回目 M3，4 回目 M2，コード ⑤）と，文だけ 1 件（コード ⑥）．
- MA1 の「拒否で早まる segment」は同じ原因だが，MA1 ですでに `radio-rule-implemented-twice` として数えたので，数え直さない．
- 答え：RM-25（拒否は覆いを足さない）と RM-6（start lead ≥ release window）．UHD Module の D9a の設計はそのまま残し，T1〜T9 で固める．分類は orchestrator が確定する．
